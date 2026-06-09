//! Integration tests for the Core Session against a live Memgraph container.

mod common;

use mgconsole_core::Value;

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
