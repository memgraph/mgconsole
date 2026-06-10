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

use mgconsole_core::{ConnectOptions, Session};
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage, ImageExt,
};
use tokio::sync::{Mutex, MutexGuard, OnceCell};

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

// --- Shared container harness (issue 12) -----------------------------------
//
// One Memgraph per test binary, leased one test at a time. Each lease wipes the
// database first, so tests are isolated by reset rather than by a fresh
// container each — the database is shared, Memgraph Community has no
// multi-database, so isolation is by wiping. Tests that mutate server-wide
// state (auth, storage mode, TLS) keep their own dedicated container via
// `start_memgraph` / `start_memgraph_tls` instead of leasing this one.

struct Shared {
    mg: Memgraph,
    /// Held for the duration of a lease so leased tests run serially.
    serial: Mutex<()>,
}

static SHARED: OnceCell<Shared> = OnceCell::const_new();

async fn shared() -> &'static Shared {
    SHARED
        .get_or_init(|| async {
            Shared {
                mg: start_memgraph().await,
                serial: Mutex::new(()),
            }
        })
        .await
}

/// Exclusive access to the shared Memgraph for one test: holds the serial lock
/// and a freshly reset Session until dropped.
pub struct Lease {
    _guard: MutexGuard<'static, ()>,
    pub session: Session,
}

/// Lease the shared Memgraph: take the serial lock, wipe the database, and hand
/// back a Session. Drop the returned `Lease` to release it for the next test.
pub async fn lease() -> Lease {
    let shared = shared().await;
    let guard = shared.serial.lock().await;
    let mut session = connect(&shared.mg).await;
    reset(&mut session).await;
    Lease {
        _guard: guard,
        session,
    }
}

/// Aggressively wipe the shared database between leases.
async fn reset(session: &mut Session) {
    let mut result = session
        .run("MATCH (n) DETACH DELETE n")
        .await
        .expect("reset: delete all");
    result.records().discard().await.expect("reset: drain");
}

/// Start a Memgraph configured for Bolt TLS, using a freshly generated
/// self-signed certificate copied into the container. The cert is self-signed
/// and never verified (ADR 0007), so the SAN does not need to match the host.
pub async fn start_memgraph_tls() -> Memgraph {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("generate self-signed cert");
    let cert_pem = cert.cert.pem().into_bytes();
    let key_pem = cert.signing_key.serialize_pem().into_bytes();

    let container = GenericImage::new("memgraph/memgraph", MEMGRAPH_TAG)
        .with_exposed_port(7687.tcp())
        .with_wait_for(WaitFor::message_on_stdout("You are running Memgraph"))
        .with_copy_to("/etc/memgraph/cert.pem", cert_pem)
        .with_copy_to("/etc/memgraph/key.pem", key_pem)
        .with_cmd([
            "--bolt-cert-file=/etc/memgraph/cert.pem",
            "--bolt-key-file=/etc/memgraph/key.pem",
        ])
        .start()
        .await
        .expect("start memgraph (tls) container");
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
    let tls = ConnectOptions {
        use_tls: true,
        ..ConnectOptions::default()
    };
    for attempt in 0..30 {
        match Session::connect_with(&mg.host, mg.port, &tls).await {
            Ok(_) => return mg,
            Err(e) if attempt == 29 => panic!("memgraph never accepted a TLS session: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(300)).await,
        }
    }
    mg
}
