//! Shared integration-test harness: a Memgraph container started on demand via
//! `testcontainers`, so `cargo test` needs only a Docker daemon (no CI). Reused
//! by the Session, import, and parallel-execution slices.
//!
//! Container strategy (agreed): as the integration suite grows past a handful of
//! tests, prefer **one shared container per test binary** plus a `reset()`
//! (`MATCH (n) DETACH DELETE n` + drop indexes/enums) that each test calls,
//! running those tests serially within the file — rather than a fresh container
//! per test, which would run dozens of heavyweight Memgraph containers at once.
//! Memgraph Community has no multi-database, so isolation is by wiping, and
//! `reset()` should be aggressive. Storage-mode-specific tests (analytical, for
//! parallel import) keep their own dedicated container. `start_memgraph()` below
//! stays available for those dedicated cases.

use std::time::Duration;

use mgconsole_core::Session;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage,
};

/// Pinned for reproducibility — the version the ADR-0001 fidelity spike used.
pub const MEMGRAPH_TAG: &str = "3.10.1";

/// A running Memgraph the test talks to. Holds the container so it lives for the
/// duration of the test and is removed on drop.
pub struct Memgraph {
    _container: ContainerAsync<GenericImage>,
    pub host: String,
    pub port: u16,
}

/// Start a fresh Memgraph and wait until Bolt accepts a Session.
pub async fn start_memgraph() -> Memgraph {
    let container = GenericImage::new("memgraph/memgraph", MEMGRAPH_TAG)
        .with_exposed_port(7687.tcp())
        .with_wait_for(WaitFor::message_on_stdout("You are running Memgraph"))
        .start()
        .await
        .expect("start memgraph container");
    let host = container
        .get_host()
        .await
        .expect("container host")
        .to_string();
    let port = container
        .get_host_port_ipv4(7687.tcp())
        .await
        .expect("mapped bolt port");

    let mg = Memgraph {
        _container: container,
        host,
        port,
    };
    // The "running" log line can precede Bolt being ready; retry the handshake.
    for attempt in 0..30 {
        match Session::connect(&mg.host, mg.port).await {
            Ok(_) => return mg,
            Err(e) if attempt == 29 => panic!("memgraph never accepted a session: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
    mg
}

/// Open a Session to a started Memgraph.
pub async fn connect(mg: &Memgraph) -> Session {
    Session::connect(&mg.host, mg.port)
        .await
        .expect("connect session")
}
