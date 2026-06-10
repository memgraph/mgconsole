//! Integration tests for the Core Session against a live Memgraph container.

mod common;

use mgconsole_core::{ConnectOptions, Credentials, Error, Session, Value};

#[tokio::test]
async fn runs_a_scalar_query_against_live_memgraph() {
    let mg = common::start_memgraph().await;
    let mut session = common::connect(&mg).await;

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
    let mg = common::start_memgraph().await;
    let mut session = common::connect(&mg).await;

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
        created.records().discard().await.expect("commit create user");
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
    let mut result = session.run("RETURN 1 AS n").await.expect("authed query runs");
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
async fn discard_after_partial_read_leaves_connection_usable() {
    let mg = common::start_memgraph().await;
    let mut session = common::connect(&mg).await;

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
