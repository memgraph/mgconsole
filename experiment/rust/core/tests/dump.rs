//! Export/import round-trip (slice 26): `DUMP DATABASE` written as cypherl
//! re-imports cleanly via the serial import path (slice 25), reproducing the
//! data. This closes the loop the cypherl writer (24) and serial import (25)
//! were built for; it is mostly an integration assertion against live Memgraph.

mod common;

use mgconsole_core::{run_serial, ImportFormat, QueryAssembler, Session, Value};

/// Dump a database to cypherl by streaming `DUMP DATABASE` through the cypherl
/// writer — exactly the production export path.
async fn dump(session: &mut Session) -> String {
    let mut sink = Vec::new();
    let report = run_serial(
        session,
        ["DUMP DATABASE".to_string()],
        &mut sink,
        &ImportFormat::Cypherl,
    )
    .await;
    assert!(report.is_success(), "dump failed: {report:?}");
    String::from_utf8(sink).unwrap()
}

/// Re-import a cypherl stream via the serial import path: assemble the text into
/// queries (as the non-interactive Frontend does) and run them in order.
async fn import(session: &mut Session, cypherl: &str) {
    let mut assembler = QueryAssembler::new();
    let mut queries = Vec::new();
    for line in cypherl.lines() {
        queries.extend(assembler.push(&format!("{line}\n")));
    }
    if assembler.has_pending() {
        queries.push(assembler.pending().trim().to_string());
    }
    let mut sink = Vec::new();
    let report = run_serial(session, queries, &mut sink, &ImportFormat::Cypherl).await;
    assert!(report.is_success(), "re-import failed: {report:?}");
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

/// Normalise dump output into a sorted set of statements, so two dumps of the
/// same data compare equal regardless of statement ordering.
fn statements(dump: &str) -> Vec<String> {
    let mut lines: Vec<String> = dump.lines().map(str::to_string).collect();
    lines.sort();
    lines
}

#[tokio::test]
async fn dump_reimport_round_trips_a_seeded_database() {
    // Two dedicated containers: a seeded source and a fresh target.
    let source = common::start_memgraph().await;
    let target = common::start_memgraph().await;
    let mut src = common::connect(&source).await;
    let mut tgt = common::connect(&target).await;

    // Seed the source with vertices, an edge, and properties of a few kinds.
    for query in [
        "CREATE (:Person {name: 'Ada', age: 36})",
        "CREATE (:Person {name: 'Bo', age: 21})",
        "MATCH (a:Person {name: 'Ada'}), (b:Person {name: 'Bo'}) \
         CREATE (a)-[:KNOWS {since: 2020}]->(b)",
    ] {
        src.run(query)
            .await
            .expect("seed query")
            .records()
            .discard()
            .await
            .expect("commit seed");
    }

    // Export: the dump captures the seeded data.
    let dump1 = dump(&mut src).await;
    assert!(dump1.contains("Ada"), "dump captures node data:\n{dump1}");
    assert!(dump1.contains("KNOWS"), "dump captures the edge:\n{dump1}");

    // Re-import into the fresh target reproduces the data.
    import(&mut tgt, &dump1).await;
    assert_eq!(
        scalar(&mut tgt, "MATCH (p:Person) RETURN count(p)").await,
        2
    );
    assert_eq!(
        scalar(
            &mut tgt,
            "MATCH (:Person)-[r:KNOWS]->(:Person) RETURN count(r)"
        )
        .await,
        1
    );
    assert_eq!(
        scalar(&mut tgt, "MATCH (p:Person {name: 'Ada'}) RETURN p.age").await,
        36
    );

    // Stability: dumping the re-imported database yields the same statements.
    let dump2 = dump(&mut tgt).await;
    assert_eq!(
        statements(&dump1),
        statements(&dump2),
        "dump → import → dump is stable"
    );
}
