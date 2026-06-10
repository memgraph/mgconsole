//! Integration tests for the serial import engine against a live Memgraph
//! container (slice 25). Drive a cypherl stream through `run_serial`, asserting
//! input-order execution, format output, and the failure report that the
//! Frontend turns into an exit code.

mod common;

use mgconsole_core::format::CsvOptions;
use mgconsole_core::{run_parallel, run_serial, ImportFormat, Session, Value, Workers};

#[tokio::test]
async fn runs_a_cypherl_stream_serially_and_takes_effect() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // A small cypherl import: create two people in order.
    let queries = vec![
        "CREATE (:Person {name: 'Ada', age: 36})".to_string(),
        "CREATE (:Person {name: 'Bo', age: 21})".to_string(),
    ];
    let mut sink = Vec::new();
    let report = run_serial(
        session,
        queries,
        &mut sink,
        &ImportFormat::Cypherl,
    )
    .await;

    assert!(report.is_success(), "clean import has no failures: {report:?}");
    assert_eq!(report.executed, 2);
    // Write queries return no columns, so cypherl output is empty.
    assert!(sink.is_empty(), "writes produce no output: {sink:?}");

    // The effect landed: both nodes exist.
    let mut count = session
        .run("MATCH (p:Person) RETURN count(p) AS n")
        .await
        .expect("count query");
    let record = count.records().next().await.expect("ok").expect("one row");
    assert_eq!(record.fields(), &[Value::Integer(2)]);
}

#[tokio::test]
async fn renders_results_in_the_selected_format() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // Two statements: a write (no output) then a read that yields rows. The read
    // is rendered as CSV, header then one row per record.
    let queries = vec![
        "CREATE (:Person {name: 'Ada', age: 36})".to_string(),
        "MATCH (p:Person) RETURN p.name AS name, p.age AS age".to_string(),
    ];
    let mut sink = Vec::new();
    let report = run_serial(session, queries, &mut sink, &ImportFormat::Csv(CsvOptions::default())).await;

    assert!(report.is_success());
    assert_eq!(String::from_utf8(sink).unwrap(), "name,age\nAda,36\n");
}

#[tokio::test]
async fn a_failing_query_is_recorded_and_the_run_continues() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // A bad query between two good ones: the bad one is recorded, the run keeps
    // going, and the final effect reflects both good queries.
    let queries = vec![
        "CREATE (:Person {name: 'Ada'})".to_string(),
        "THIS IS NOT CYPHER".to_string(),
        "CREATE (:Person {name: 'Bo'})".to_string(),
    ];
    let mut sink = Vec::new();
    let report = run_serial(session, queries, &mut sink, &ImportFormat::Cypherl).await;

    assert!(!report.is_success(), "a failed query fails the run");
    assert_eq!(report.executed, 2, "the two valid queries still ran");
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].query, "THIS IS NOT CYPHER");

    // Both valid creates landed despite the error in the middle.
    let mut count = session
        .run("MATCH (p:Person) RETURN count(p) AS n")
        .await
        .expect("count query");
    let record = count.records().next().await.expect("ok").expect("one row");
    assert_eq!(record.fields(), &[Value::Integer(2)]);
}

#[tokio::test]
async fn batched_parallel_import_loads_a_dataset_across_workers() {
    // A dedicated container: this opens several worker connections of its own.
    let mg = common::start_memgraph().await;

    // 200 independent node creates — no inter-query dependencies, so any
    // interleaving across workers produces the same correct graph (vertices-first
    // ordering is slice 31; this slice is raw parallelism).
    let queries: Vec<String> = (0..200)
        .map(|i| format!("CREATE (:Item {{n: {i}}})"))
        .collect();

    let workers = Workers::connect(&mg.host, mg.port, &Default::default(), 4)
        .await
        .expect("4 workers");

    // batch-size 16 over 4 workers: many Batches pulled concurrently.
    let report = run_parallel(workers, queries, 16).await;
    assert!(report.is_success(), "clean parallel import: {report:?}");
    assert_eq!(report.executed, 200);

    // Every Batch's effect landed exactly once.
    let mut verify = common::connect(&mg).await;
    let count = scalar(&mut verify, "MATCH (i:Item) RETURN count(i)").await;
    assert_eq!(count, 200);
    let distinct = scalar(&mut verify, "MATCH (i:Item) RETURN count(DISTINCT i.n)").await;
    assert_eq!(distinct, 200, "no query ran twice or was dropped");
}

/// Run a query expected to return a single integer scalar.
async fn scalar(session: &mut Session, query: &str) -> i64 {
    let mut result = session.run(query).await.expect("scalar query");
    let record = result.records().next().await.expect("ok").expect("one row");
    match record.fields() {
        [Value::Integer(n)] => *n,
        other => panic!("expected one integer, got {other:?}"),
    }
}
