//! Integration tests for worker Sessions (slice 29): stand up N long-lived
//! connections against a live Memgraph and confirm each runs queries
//! independently, including independent reconnect after a broken connection.

mod common;

use std::time::Duration;

use mgconsole_core::{ConnectOptions, Session, Value, Workers};

/// Run `RETURN <n>` on a single Session and assert the scalar comes back,
/// draining the result so it is cleanly finished before the next query (ADR 0005).
async fn assert_returns(session: &mut Session, n: i64) {
    let mut result = session
        .run(&format!("RETURN {n} AS n"))
        .await
        .expect("worker query runs");
    let record = result.records().next().await.expect("ok").expect("one row");
    assert_eq!(record.fields(), &[Value::Integer(n)]);
    assert!(
        result.records().next().await.expect("drain").is_none(),
        "exactly one row"
    );
}

#[tokio::test]
async fn establishes_n_workers_and_runs_a_query_on_each() {
    // A dedicated container: this opens several extra connections of its own.
    let mg = common::start_memgraph().await;

    let mut workers = Workers::connect(&mg.host, mg.port, &ConnectOptions::default(), 4)
        .await
        .expect("establish 4 workers");
    // The worker count bounds the number of connections.
    assert_eq!(workers.len(), 4);

    // Each worker has its own live connection and runs a query independently.
    for (i, session) in workers.sessions_mut().iter_mut().enumerate() {
        assert_returns(session, i as i64).await;
    }
}

#[tokio::test]
async fn zero_workers_auto_detects_at_least_one() {
    let mg = common::start_memgraph().await;
    let workers = Workers::connect(&mg.host, mg.port, &ConnectOptions::default(), 0)
        .await
        .expect("auto-detect workers");
    assert!(!workers.is_empty(), "auto-detect yields at least one worker");
}

#[tokio::test]
async fn a_broken_worker_reconnects_without_disturbing_the_others() {
    // Each worker connects through its own proxy to the same Memgraph, so one
    // worker's connection can be severed in isolation.
    let mg = common::start_memgraph().await;
    let proxy0 = common::start_proxy(mg.host.clone(), mg.port).await;
    let proxy1 = common::start_proxy(mg.host.clone(), mg.port).await;

    let worker0 = Session::connect(&proxy0.host, proxy0.port)
        .await
        .expect("worker 0 via proxy 0");
    let worker1 = Session::connect(&proxy1.host, proxy1.port)
        .await
        .expect("worker 1 via proxy 1");
    let mut workers = Workers::from_sessions(vec![worker0, worker1]);

    // Both workers start healthy.
    assert_returns(&mut workers.sessions_mut()[0], 1).await;
    assert_returns(&mut workers.sessions_mut()[1], 2).await;

    // Sever only worker 0's connection; worker 1's proxy stays up.
    proxy0.cut();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Worker 0 reconnects through its own proxy on its next query (slice 14);
    // worker 1 was never disturbed and keeps working.
    assert_returns(&mut workers.sessions_mut()[0], 10).await;
    assert_returns(&mut workers.sessions_mut()[1], 20).await;
}
