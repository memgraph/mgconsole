//! Integration tests for the Core Session against a live Memgraph container.

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use mgconsole_core::{ConnectOptions, Credentials, Error, Session, Value};

#[tokio::test]
async fn runs_a_scalar_query_against_live_memgraph() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    let mut result = session.run("RETURN 1 AS n").await.expect("query runs");

    assert_eq!(result.header(), &["n".to_string()]);

    let record = result
        .records()
        .next()
        .await
        .expect("stream ok")
        .expect("one record");
    assert_eq!(record.fields(), &[Value::Integer(1)]);

    assert!(
        result.records().next().await.expect("stream ok").is_none(),
        "exactly one record"
    );
}

#[tokio::test]
async fn streams_a_large_result_incrementally() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // 50k rows — far more than one batch, so this exercises lazy batched pulls.
    let mut result = session
        .run("UNWIND range(1, 50000) AS i RETURN i")
        .await
        .expect("query runs");

    let mut count = 0i64;
    let mut last = 0i64;
    while let Some(record) = result.records().next().await.expect("stream ok") {
        if let [Value::Integer(i)] = record.fields() {
            last = *i;
        }
        count += 1;
    }
    assert_eq!(count, 50_000);
    assert_eq!(last, 50_000);
}

#[tokio::test]
async fn write_query_exposes_update_stats() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    let mut result = session
        .run("CREATE (:Person {name: 'x', age: 1})")
        .await
        .expect("create runs");
    result.records().discard().await.expect("drain to summary");

    let stats = &result.summary().stats;
    assert_eq!(stats.get("nodes-created"), Some(&Value::Integer(1)));
    assert_eq!(stats.get("labels-added"), Some(&Value::Integer(1)));
    // The update-stats map is present and keyed as Memgraph reports it.
    assert!(stats.contains_key("properties-set"));
}

#[tokio::test]
async fn query_exposes_notifications() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // A label scan with no index draws a plan-hint notification from Memgraph.
    let mut result = session
        .run("MATCH (n:Nonexistent) RETURN n")
        .await
        .expect("query runs");
    result.records().discard().await.expect("drain to summary");

    let notifications = &result.summary().notifications;
    assert!(!notifications.is_empty(), "expected a notification");
    let first = &notifications[0];
    assert!(!first.code.is_empty(), "notification has a code");
    assert!(!first.title.is_empty(), "notification has a title");
    assert_eq!(first.severity, "INFO");
}

#[tokio::test]
async fn summary_exposes_execution_info_and_clean_absences() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    let mut result = session.run("RETURN 1 AS n").await.expect("query runs");
    // Summary is readable after the records drain; no error when there are no
    // stats or notifications to report.
    result.records().discard().await.expect("drain to summary");
    let summary = result.summary();

    assert!(summary.stats.is_empty(), "a read has no update stats");
    assert!(
        summary.notifications.is_empty(),
        "a trivial read has no notifications"
    );

    // Verbose execution info is reported per query.
    let info = summary.execution_info();
    assert!(info.parsing_time.is_some(), "parse time reported");
    assert!(info.planning_time.is_some(), "plan time reported");
    assert!(info.plan_execution_time.is_some(), "execute time reported");
    assert!(info.cost_estimate.is_some(), "cost estimate reported");
}

#[tokio::test]
async fn binds_named_parameters_of_every_kind() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // A query referencing several parameters of different Value kinds; each must
    // round-trip into the query and come back unchanged.
    let mut params = BTreeMap::new();
    params.insert("n".to_string(), Value::Integer(7));
    params.insert("s".to_string(), Value::String("hello".to_string()));
    params.insert(
        "xs".to_string(),
        Value::List(vec![Value::Integer(1), Value::Integer(2)]),
    );
    params.insert(
        "m".to_string(),
        Value::Map(BTreeMap::from([("k".to_string(), Value::Boolean(true))])),
    );
    params.insert(
        "d".to_string(),
        Value::Date(chrono::NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()),
    );

    let mut result = session
        .run_with_params(
            "RETURN $n AS n, $s AS s, $xs AS xs, $m AS m, $d AS d",
            &params,
        )
        .await
        .expect("parameterised query runs");
    let record = result
        .records()
        .next()
        .await
        .expect("stream ok")
        .expect("one record");
    assert_eq!(
        record.fields(),
        &[
            Value::Integer(7),
            Value::String("hello".to_string()),
            Value::List(vec![Value::Integer(1), Value::Integer(2)]),
            Value::Map(BTreeMap::from([("k".to_string(), Value::Boolean(true))])),
            Value::Date(chrono::NaiveDate::from_ymd_opt(2026, 6, 10).unwrap()),
        ]
    );
}

#[tokio::test]
async fn parameter_resolves_against_stored_data() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    let mut created = session
        .run("CREATE (:Person {name: 'Ada', age: 36})")
        .await
        .expect("create node");
    created.records().discard().await.expect("commit create");

    let params = BTreeMap::from([("name".to_string(), Value::String("Ada".to_string()))]);
    let mut result = session
        .run_with_params(
            "MATCH (p:Person {name: $name}) RETURN p.age AS age",
            &params,
        )
        .await
        .expect("lookup by parameter");
    let record = result
        .records()
        .next()
        .await
        .expect("stream ok")
        .expect("one record");
    assert_eq!(record.fields(), &[Value::Integer(36)]);
}

#[tokio::test]
async fn authenticates_with_valid_credentials_and_rejects_bad_ones() {
    // Dedicated container: creating a user flips on auth enforcement for all new
    // connections, so this must not run against a shared/wiped Memgraph.
    let mg = common::start_memgraph().await;

    // The first connection is still anonymous; use it to create a user. Once a
    // user exists, Memgraph requires authentication for subsequent connections.
    {
        let mut admin = common::connect(&mg).await;
        let mut created = admin
            .run("CREATE USER tester IDENTIFIED BY 'secret'")
            .await
            .expect("create user");
        // Drain (PULL) so the user is committed before we reconnect as them.
        created
            .records()
            .discard()
            .await
            .expect("commit create user");
    }

    let good = ConnectOptions {
        credentials: Some(Credentials {
            username: "tester".to_string(),
            password: "secret".to_string(),
        }),
        ..ConnectOptions::default()
    };
    let mut session = Session::connect_with(&mg.host, mg.port, &good)
        .await
        .expect("valid credentials authenticate");
    let mut result = session
        .run("RETURN 1 AS n")
        .await
        .expect("authed query runs");
    let record = result
        .records()
        .next()
        .await
        .expect("stream ok")
        .expect("one record");
    assert_eq!(record.fields(), &[Value::Integer(1)]);

    let bad = ConnectOptions {
        credentials: Some(Credentials {
            username: "tester".to_string(),
            password: "wrong".to_string(),
        }),
        ..ConnectOptions::default()
    };
    match Session::connect_with(&mg.host, mg.port, &bad).await {
        Err(Error::Auth(_)) => {}
        Err(other) => panic!("expected an auth error, got {other:?}"),
        Ok(_) => panic!("bad credentials must not authenticate"),
    }
}

#[tokio::test]
async fn connects_over_tls_to_an_ssl_configured_container() {
    let mg = common::start_memgraph_tls().await;

    let tls = ConnectOptions {
        use_tls: true,
        ..ConnectOptions::default()
    };
    let mut session = Session::connect_with(&mg.host, mg.port, &tls)
        .await
        .expect("TLS connection established");
    let mut result = session.run("RETURN 42 AS n").await.expect("query over TLS");
    let record = result
        .records()
        .next()
        .await
        .expect("stream ok")
        .expect("one record");
    assert_eq!(record.fields(), &[Value::Integer(42)]);

    // A plaintext connection to the same (TLS-only) port must fail, not panic.
    match Session::connect(&mg.host, mg.port).await {
        Err(_) => {}
        Ok(_) => panic!("plaintext must not connect to a TLS-only Bolt port"),
    }
}

#[tokio::test]
async fn query_error_leaves_session_usable() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // A syntactically invalid query is a recoverable query error.
    match session.run("RETURN ($").await {
        Err(Error::Query(e)) => assert!(!e.code.is_empty(), "query error carries a code"),
        Err(other) => panic!("expected a query error, got {other:?}"),
        Ok(_) => panic!("invalid query must not succeed"),
    }

    // The Session survives it (RESET cleared the FAILED state): the next query runs.
    let mut ok = session
        .run("RETURN 1 AS n")
        .await
        .expect("session still usable");
    let record = ok.records().next().await.expect("ok").expect("one row");
    assert_eq!(record.fields(), &[Value::Integer(1)]);
}

#[tokio::test]
async fn run_errors_while_a_result_is_still_open() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // Open a result and partially read it, so it is still live.
    let mut open = session
        .run("UNWIND range(1, 100) AS i RETURN i")
        .await
        .expect("first query");
    let _ = open.records().next().await.expect("read one row");

    // A second run while the first result is open is rejected.
    match session.run("RETURN 1").await {
        Err(Error::ResultStillOpen) => {}
        Err(other) => panic!("expected ResultStillOpen, got {other:?}"),
        Ok(_) => panic!("run must be rejected while a result is open"),
    }

    // Draining the open result clears the guard; the Session is usable again.
    open.records().discard().await.expect("drain open result");
    let mut next = session
        .run("RETURN 2 AS n")
        .await
        .expect("usable after drain");
    let record = next.records().next().await.expect("ok").expect("one row");
    assert_eq!(record.fields(), &[Value::Integer(2)]);
}

#[tokio::test]
async fn recovers_from_an_abandoned_result_via_reset() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    {
        // Open a large result, pull one batch, then drop it without draining.
        let mut abandoned = session
            .run("UNWIND range(1, 100000) AS i RETURN i")
            .await
            .expect("open result");
        let _ = abandoned.records().next().await.expect("pull a batch");
    } // dropped here, still with records pending on the wire

    // The next run detects the abandoned result and RESETs before running.
    let mut next = session
        .run("RETURN 7 AS n")
        .await
        .expect("recovered via reset");
    let record = next.records().next().await.expect("ok").expect("one row");
    assert_eq!(record.fields(), &[Value::Integer(7)]);
}

#[tokio::test]
async fn reconnects_after_a_dropped_connection() {
    // Dedicated container behind a proxy we can sever (the shared lease's
    // container is not reachable through a proxy).
    let mg = common::start_memgraph().await;
    let proxy = common::start_proxy(mg.host.clone(), mg.port).await;
    let mut session = Session::connect(&proxy.host, proxy.port)
        .await
        .expect("connect via proxy");

    // A Frontend hook records each reconnect attempt the Session announces.
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    session.on_reconnect(move |notice| {
        assert!(notice.attempt >= 1 && notice.attempt <= notice.max);
        seen.fetch_add(1, Ordering::SeqCst);
    });

    let mut first = session.run("RETURN 1 AS n").await.expect("first query");
    first.records().discard().await.expect("drain");

    // Sever the live connection; the server stays up behind the proxy.
    proxy.cut();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // The next query hits the dead socket, reconnects through the proxy, and runs.
    let mut second = session
        .run("RETURN 2 AS n")
        .await
        .expect("reconnect and run");
    let record = second.records().next().await.expect("ok").expect("one row");
    assert_eq!(record.fields(), &[Value::Integer(2)]);

    // The reconnect was surfaced to the Frontend (at least the first attempt).
    assert!(
        attempts.load(Ordering::SeqCst) >= 1,
        "the reconnect observer should have been notified"
    );
}

#[tokio::test]
async fn exhausting_reconnect_retries_surfaces_a_terminal_error() {
    let mg = common::start_memgraph().await;
    let proxy = common::start_proxy(mg.host.clone(), mg.port).await;
    let mut session = Session::connect(&proxy.host, proxy.port)
        .await
        .expect("connect via proxy");

    let mut first = session.run("RETURN 1 AS n").await.expect("first query");
    first.records().discard().await.expect("drain");

    // Tear the proxy down: the live socket dies and reconnect attempts are refused.
    proxy.shutdown();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    match session.run("RETURN 2").await {
        Err(Error::Connection(msg)) => {
            assert!(
                msg.contains("reconnect"),
                "terminal error mentions reconnect: {msg}"
            );
        }
        Err(other) => panic!("expected a terminal connection error, got {other:?}"),
        Ok(_) => panic!("query must fail when the server is unreachable"),
    }
}

#[tokio::test]
async fn discard_after_partial_read_leaves_connection_usable() {
    let mut lease = common::lease().await;
    let session = &mut lease.session;

    // Read only part of a large result, then discard the rest.
    let mut result = session
        .run("UNWIND range(1, 50000) AS i RETURN i")
        .await
        .expect("query runs");
    let (rows, overflowed) = result.records().collect_capped(10).await.expect("capped");
    assert_eq!(rows.len(), 10);
    assert!(overflowed);
    result.records().discard().await.expect("discard");

    // The same Session can run another query.
    let mut next = session.run("RETURN 7 AS n").await.expect("reuse session");
    let rec = next.records().next().await.expect("ok").expect("one row");
    assert_eq!(rec.fields(), &[Value::Integer(7)]);
}
