//! Live-Memgraph integration for the workbench schema completion source (slice
//! 12). Verifies, against a real `memgraph/memgraph` via testcontainers, that the
//! schema-metadata feature populates completion when enabled, and that the
//! workbench degrades to static-only (no names) when it is off. The exact
//! introspection queries were build-time-verified here.
//!
//! Needs a Docker daemon (no CI), like the Core integration suite.

use std::time::Duration;

use mgconsole::syntax::Completer;
use mgconsole::workbench::schema::{
    parse_schema, SchemaSource, NODE_PROPERTIES_QUERY, REL_PROPERTIES_QUERY,
};
use mgconsole_core::{ConnectOptions, Endpoint, Session};
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};

const MEMGRAPH_TAG: &str = "3.10.1";

/// Start a Memgraph with the schema-metadata feature enabled and return a
/// connected Session (plus the container, kept alive for the test).
async fn start() -> (ContainerAsync<GenericImage>, Session) {
    let image = GenericImage::new("memgraph/memgraph", MEMGRAPH_TAG)
        .with_exposed_port(7687.tcp())
        .with_wait_for(WaitFor::message_on_stdout("You are running Memgraph"))
        .with_cmd(vec![
            "--telemetry-enabled=false",
            "--storage-enable-schema-metadata=true",
        ]);
    let container = image.start().await.expect("start memgraph");
    let host = container.get_host().await.unwrap().to_string();
    let port = container.get_host_port_ipv4(7687.tcp()).await.unwrap();
    let endpoint = Endpoint::new(host, port);

    let options = ConnectOptions {
        credentials: None,
        use_tls: false,
    };
    for attempt in 0..30 {
        match Session::connect_with(&endpoint, &options).await {
            Ok(session) => return (container, session),
            Err(e) if attempt == 29 => panic!("memgraph never accepted a session: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
    unreachable!()
}

async fn run_discard(session: &mut Session, query: &str) {
    session
        .run(query)
        .await
        .unwrap()
        .records()
        .discard()
        .await
        .unwrap();
}

#[tokio::test]
async fn schema_metadata_enabled_populates_completion() {
    let (_c, mut session) = start().await;
    for query in [
        "CREATE (:Person {name: 'Ada', age: 36})",
        "CREATE (:Company {title: 'Acme'})",
        "MATCH (p:Person), (c:Company) CREATE (p)-[:WORKS_AT {since: 2020}]->(c)",
    ] {
        run_discard(&mut session, query).await;
    }

    let nodes = session
        .run(NODE_PROPERTIES_QUERY)
        .await
        .expect("node metadata")
        .records()
        .collect()
        .await
        .expect("collect nodes");
    let rels = session
        .run(REL_PROPERTIES_QUERY)
        .await
        .expect("rel metadata")
        .records()
        .collect()
        .await
        .expect("collect rels");

    let schema = parse_schema(&nodes, &rels);
    assert!(schema.labels.contains(&"Person".to_string()), "{:?}", schema.labels);
    assert!(schema.labels.contains(&"Company".to_string()));
    assert!(schema.rel_types.contains(&"WORKS_AT".to_string()), "{:?}", schema.rel_types);
    assert!(schema.property_keys.contains(&"name".to_string()));
    assert!(schema.property_keys.contains(&"since".to_string()));

    // Registered as a second completion source, the live names are offered.
    let mut completer = Completer::with_static_vocabulary();
    completer.add_source(Box::new(SchemaSource::new(schema.all_names())));
    assert!(completer.candidates("Per").contains(&"Person".to_string()));
    // The static vocabulary still completes too.
    assert!(completer.candidates("RET").contains(&"RETURN".to_string()));
}

#[tokio::test]
async fn an_empty_or_unavailable_schema_degrades_to_static_only() {
    // The workbench degrades to static-only in two ways, both exercised here:
    //
    // (1) Nothing to offer: on an empty database the metadata procedure returns
    //     no rows, so the parsed Schema is empty and contributes no names.
    let (_c, mut session) = start().await;
    let nodes = session
        .run(NODE_PROPERTIES_QUERY)
        .await
        .expect("node metadata")
        .records()
        .collect()
        .await
        .expect("collect nodes");
    let rels = session
        .run(REL_PROPERTIES_QUERY)
        .await
        .expect("rel metadata")
        .records()
        .collect()
        .await
        .expect("collect rels");
    let schema = parse_schema(&nodes, &rels);
    assert!(schema.all_names().is_empty(), "empty database, no names: {schema:?}");

    // (2) Feature unavailable: a missing procedure errors, which the edge maps to
    //     `None` (no schema, no graph scan) — static-only completion either way.
    assert!(
        session.run("CALL does.not.exist() YIELD x RETURN x").await.is_err(),
        "an unavailable procedure errors, so the edge degrades to None"
    );

    let completer = Completer::with_static_vocabulary();
    assert!(completer.candidates("RET").contains(&"RETURN".to_string()));
}
