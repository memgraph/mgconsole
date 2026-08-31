//! ADR-0001 Bolt fidelity spike.
//!
//! Throwaway. Spins up Memgraph via testcontainers, then asks the pure-Rust
//! Bolt stack (`bolt-client` + `bolt-proto`) to decode one query per Memgraph
//! Value type. Records, per type, whether it:
//!   - DECODES        — bolt-proto returned a typed Value
//!   - NEEDS-CODEC    — server sent a struct signature bolt-proto rejects
//!   - SERVER-ERR     — Memgraph refused the query (usually our Cypher/syntax)
//!   - CONN-ERR       — couldn't talk to the server
//!
//! The output is the raw material for the go/no-go recommendation in
//! `01-bolt-fidelity-spike.md`.

use bolt_client::{Client, Metadata};
use bolt_proto::{version::*, Message, Value};
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    GenericImage,
};
use tokio::io::BufStream;
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

const MEMGRAPH_TAG: &str = "3.10.1";

type Conn = Client<Compat<BufStream<TcpStream>>>;
type R<T> = Result<T, Box<dyn std::error::Error>>;

/// One Value type under test: a label, optional setup statements (DDL / data
/// creation, run and discarded), and the query whose first field we inspect.
struct Case {
    label: &'static str,
    setup: &'static [&'static str],
    query: &'static str,
}

const fn case(label: &'static str, setup: &'static [&'static str], query: &'static str) -> Case {
    Case { label, setup, query }
}

enum Outcome {
    Decoded(String),
    NeedsCodec(String),
    ServerErr(String),
    ConnErr(String),
}

async fn connect(addr: &str) -> R<Conn> {
    let stream = BufStream::new(TcpStream::connect(addr).await?);
    let mut client = Client::new(stream.compat(), &[V4_4, V4_3, V4_2, V4_1]).await?;
    let resp = client
        .hello(Metadata::from_iter(vec![
            ("user_agent", "bolt-fidelity-spike/0.0"),
            ("scheme", "none"),
        ]))
        .await?;
    match resp {
        Message::Success(_) => Ok(client),
        other => Err(format!("HELLO refused: {other:?}").into()),
    }
}

/// Run a statement, pull to completion, return Err(server message) on Failure.
async fn exec(client: &mut Conn, stmt: &str) -> R<Vec<bolt_proto::message::Record>> {
    match client.run(stmt, None, None).await? {
        Message::Success(_) => {}
        other => return Err(format!("{other:?}").into()),
    }
    let (records, end) = client
        .pull(Some(Metadata::from_iter(vec![("n", -1_i64)])))
        .await?;
    match end {
        Message::Success(_) => Ok(records),
        other => Err(format!("{other:?}").into()),
    }
}

async fn run_case(addr: &str, c: &Case) -> Outcome {
    let mut client = match connect(addr).await {
        Ok(c) => c,
        Err(e) => return Outcome::ConnErr(e.to_string()),
    };

    // Setup statements are best-effort: an "already exists" on a rerun is fine.
    for s in c.setup {
        if let Err(e) = exec(&mut client, s).await {
            let msg = e.to_string().to_lowercase();
            if !msg.contains("exists") && !msg.contains("already") {
                // A genuine setup failure — surface it so we can fix the Cypher.
                return Outcome::ServerErr(format!("setup `{s}` -> {e}"));
            }
            // Memgraph leaves the connection in FAILED state after a Failure;
            // a fresh connection is simplest for a throwaway spike.
            client = match connect(addr).await {
                Ok(c) => c,
                Err(e) => return Outcome::ConnErr(e.to_string()),
            };
        }
    }

    // The query itself. We split run() from pull() so we can tell a server-side
    // Failure (syntax) apart from a client-side decode gap (unknown signature).
    match client.run(c.query, None, None).await {
        Ok(Message::Success(_)) => {}
        Ok(other) => return Outcome::ServerErr(format!("{other:?}")),
        Err(e) => return Outcome::ConnErr(e.to_string()),
    }
    match client
        .pull(Some(Metadata::from_iter(vec![("n", -1_i64)])))
        .await
    {
        Ok((records, Message::Success(_))) => {
            match records.first().and_then(|r| r.fields().first()) {
                Some(v) => Outcome::Decoded(render(v)),
                None => Outcome::Decoded("<no rows>".into()),
            }
        }
        Ok((_, other)) => Outcome::ServerErr(format!("{other:?}")),
        // A decode gap surfaces here: bolt-proto rejected a struct signature.
        Err(e) => Outcome::NeedsCodec(format!("{e}")),
    }
}

/// Compact, fidelity-revealing rendering of a decoded Value.
fn render(v: &Value) -> String {
    format!("{v:?}")
}

#[tokio::main]
async fn main() -> R<()> {
    eprintln!("starting memgraph/memgraph:{MEMGRAPH_TAG} via testcontainers...");
    let container = GenericImage::new("memgraph/memgraph", MEMGRAPH_TAG)
        .with_exposed_port(7687.tcp())
        .with_wait_for(WaitFor::message_on_stdout("You are running Memgraph"))
        .start()
        .await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(7687.tcp()).await?;
    let addr = format!("{host}:{port}");
    eprintln!("memgraph up at {addr}");

    // Give Bolt a moment past the "running" log line, retrying the handshake.
    for attempt in 0..30 {
        match connect(&addr).await {
            Ok(_) => break,
            Err(e) if attempt == 29 => return Err(format!("never connected: {e}").into()),
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(300)).await,
        }
    }

    let cases: &[Case] = &[
        case("null", &[], "RETURN null"),
        case("boolean", &[], "RETURN true"),
        case("integer", &[], "RETURN 42"),
        case("float", &[], "RETURN 3.14159"),
        case("string", &[], "RETURN 'héllo ☃'"),
        case("list", &[], "RETURN [1, 'two', 3.0, null]"),
        case("map", &[], "RETURN {a: 1, b: 'two', c: [1, 2]}"),
        case("node", &[], "CREATE (n:Person {name: 'Ada', age: 36, active: true}) RETURN n"),
        case(
            "relationship",
            &[],
            "CREATE (a:A)-[r:KNOWS {since: 2020}]->(b:B) RETURN r",
        ),
        case(
            "path (+ unbound rel)",
            &[],
            "CREATE p = (:A)-[:KNOWS]->(:B) RETURN p",
        ),
        case("date", &[], "RETURN date('2021-06-15')"),
        case("localtime", &[], "RETURN localTime('12:34:56.789')"),
        case(
            "localdatetime",
            &[],
            "RETURN localDateTime('2021-06-15T12:34:56.789')",
        ),
        case("duration", &[], "RETURN duration('P1DT2H3M4S')"),
        case(
            "zoned datetime (offset)",
            &[],
            "RETURN datetime('2021-06-15T12:34:56+02:00')",
        ),
        case(
            "zoned datetime (named tz)",
            &[],
            "RETURN datetime('2021-06-15T12:34:56[Europe/Zagreb]')",
        ),
        case("point 2d cartesian", &[], "RETURN point({x: 1, y: 2})"),
        case(
            "point 2d wgs84",
            &[],
            "RETURN point({longitude: 1, latitude: 2})",
        ),
        case("point 3d cartesian", &[], "RETURN point({x: 1, y: 2, z: 3})"),
        case(
            "point 3d wgs84",
            &[],
            "RETURN point({longitude: 1, latitude: 2, height: 3})",
        ),
        case(
            "enum",
            &["CREATE ENUM Status VALUES { Active, Inactive }"],
            "RETURN Status::Active",
        ),
    ];

    let mut decoded = 0;
    let mut gaps = 0;
    let mut errs = 0;
    println!("\n{:<26} {:<12} {}", "TYPE", "RESULT", "DETAIL");
    println!("{}", "-".repeat(100));
    for c in cases {
        let (tag, detail) = match run_case(&addr, c).await {
            Outcome::Decoded(s) => {
                decoded += 1;
                ("DECODES", s)
            }
            Outcome::NeedsCodec(s) => {
                gaps += 1;
                ("NEEDS-CODEC", s)
            }
            Outcome::ServerErr(s) => {
                errs += 1;
                ("SERVER-ERR", s)
            }
            Outcome::ConnErr(s) => {
                errs += 1;
                ("CONN-ERR", s)
            }
        };
        let detail = detail.replace('\n', " ");
        println!("{:<26} {:<12} {}", c.label, tag, detail);
    }
    println!("{}", "-".repeat(100));
    println!("\nSUMMARY: {decoded} decode, {gaps} need codec extension, {errs} server/conn errors");

    Ok(())
}
