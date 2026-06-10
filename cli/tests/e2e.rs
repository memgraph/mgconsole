//! End-to-end CLI goldens (slice 25): pipe a cypherl stream into the built
//! binary over non-TTY stdin and assert its stdout and exit code, mirroring
//! `mgconsole`'s `run-tests.sh`. Kept thin — the rendering and import seams carry
//! the detail; this proves the non-interactive path is wired end to end.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use mgconsole_core::Session;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage,
};

/// Pinned to the version the rest of the suite uses.
const MEMGRAPH_TAG: &str = "3.10.1";

struct Memgraph {
    _container: ContainerAsync<GenericImage>,
    host: String,
    port: u16,
}

/// Start a fresh Memgraph and wait until Bolt accepts a Session.
async fn start_memgraph() -> Memgraph {
    let container = GenericImage::new("memgraph/memgraph", MEMGRAPH_TAG)
        .with_exposed_port(7687.tcp())
        .with_wait_for(WaitFor::message_on_stdout("You are running Memgraph"))
        .start()
        .await
        .expect("start memgraph container");
    let host = container.get_host().await.expect("host").to_string();
    let port = container
        .get_host_port_ipv4(7687.tcp())
        .await
        .expect("bolt port");

    // The "running" log line can precede Bolt being ready; retry the handshake so
    // the spawned binary connects on its first attempt.
    for attempt in 0..30 {
        match Session::connect(&host, port).await {
            Ok(_) => break,
            Err(e) if attempt == 29 => panic!("memgraph never accepted a session: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
    Memgraph {
        _container: container,
        host,
        port,
    }
}

/// Run the built `mgconsole` binary with `args`, piping `stdin` in over a non-TTY
/// pipe (so the non-interactive path runs).
fn run_binary(args: &[&str], stdin: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_mgconsole"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mgconsole");
    child
        .stdin
        .take()
        .expect("stdin piped")
        .write_all(stdin.as_bytes())
        .expect("write stdin"); // dropping the handle here closes stdin (EOF)
    child.wait_with_output().expect("await mgconsole")
}

/// Run the binary against `mg` over plaintext Bolt.
fn run_cli(mg: &Memgraph, extra_args: &[&str], stdin: &str) -> std::process::Output {
    let port = mg.port.to_string();
    let mut args = vec!["--host", &mg.host, "--port", &port, "--use-ssl", "false"];
    args.extend_from_slice(extra_args);
    run_binary(&args, stdin)
}

#[tokio::test]
async fn pipes_a_cypherl_stream_and_reflects_the_exit_code() {
    let mg = start_memgraph().await;

    // A clean import: a write (no output) then a read rendered as CSV. The effect
    // of the write is observable in the read's count, all in input order.
    let clean = run_cli(
        &mg,
        &["--output-format", "csv"],
        "CREATE (:Person {name: 'Ada'});\nMATCH (p:Person) RETURN count(p) AS n;\n",
    );
    assert!(
        clean.status.success(),
        "a clean run exits zero; stderr: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
    assert_eq!(String::from_utf8(clean.stdout).unwrap(), "n\n1\n");

    // A failing query produces a non-zero exit code, so the tool is usable in CI.
    let broken = run_cli(&mg, &[], "THIS IS NOT CYPHER;\n");
    assert!(
        !broken.status.success(),
        "a failing query must exit non-zero"
    );
}

#[tokio::test]
async fn batched_parallel_import_loads_data_via_the_cli_flags() {
    let mg = start_memgraph().await;

    // Pipe 60 independent node creates through the batched-parallel path, driven
    // entirely by the CLI flags (workers + batch size).
    let mut stream = String::new();
    for i in 0..60 {
        stream.push_str(&format!("CREATE (:Item {{n: {i}}});\n"));
    }
    let imported = run_cli(
        &mg,
        &[
            "--import-mode",
            "batched-parallel",
            "--workers-number",
            "4",
            "--batch-size",
            "8",
        ],
        &stream,
    );
    assert!(
        imported.status.success(),
        "parallel import exits zero; stderr: {}",
        String::from_utf8_lossy(&imported.stderr)
    );

    // A follow-up read confirms every node landed exactly once.
    let counted = run_cli(
        &mg,
        &["--output-format", "csv"],
        "MATCH (i:Item) RETURN count(i) AS n;\n",
    );
    assert!(counted.status.success());
    assert_eq!(String::from_utf8(counted.stdout).unwrap(), "n\n60\n");
}

#[test]
fn parser_mode_reports_without_touching_the_database() {
    // Point at an address nothing listens on: parser mode must succeed anyway,
    // proving it never connects (no DB execution, no side effects).
    let out = run_binary(
        &[
            "--host",
            "127.0.0.1",
            "--port",
            "1", // unused — a connection attempt here would fail
            "--import-mode",
            "parser",
            "--parser-stats",
        ],
        "CREATE (:Person {name: 'Ada'});\nMATCH (n) RETURN n;\n",
    );
    assert!(
        out.status.success(),
        "parser mode needs no database; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("1: Create"), "per-query report: {stdout}");
    assert!(stdout.contains("2: Match"), "per-query report: {stdout}");
    assert!(
        stdout.contains("Parsed 2 queries; nothing executed."),
        "summary: {stdout}"
    );
    assert!(stdout.contains("Create: 1"), "statistics: {stdout}");
}
