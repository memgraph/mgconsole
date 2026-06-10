//! The Session: a single live conversation with one Memgraph server.
//!
//! The Bolt stack is async; the Session exposes `async` methods and a Frontend
//! calls `block_on` at the boundary (ADR 0002). A query returns a
//! [`QueryResult`] whose records stream lazily, pulled from the connection in
//! batches so memory stays bounded on large results (ADR 0004, slice 21). The
//! connection is shared with the live [`RecordStream`] via `Arc<Mutex<…>>`.
//!
//! Slice 14 adds the error taxonomy and recovery. A query error (Bolt `FAILURE`)
//! is recoverable: the Session sends `RESET` and stays usable. A fatal transport
//! error triggers a bounded **reconnect**. The one-live-result invariant (ADR
//! 0005) is enforced at runtime: `run()` errors with [`Error::ResultStillOpen`]
//! if a prior result is still being read, and recovers from an abandoned one
//! (its `RecordStream` dropped before drain) by sending `RESET` first.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use bolt_client::{Client, Metadata, Params};
use bolt_proto::{
    version::{V4_1, V4_2, V4_3, V4_4},
    Message,
};
use tokio::io::BufStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

use crate::error::Error;
use crate::proto;
use crate::result::{QueryResult, RecordStream, ResultGuard, Summary, DEFAULT_BATCH_SIZE};
use crate::transport::{self, MaybeTlsStream};
use crate::value::{self, Value};

pub(crate) type Conn = Client<Compat<BufStream<MaybeTlsStream>>>;
pub(crate) type SharedConn = Arc<Mutex<Conn>>;

const USER_AGENT: &str = concat!("mgconsole/", env!("CARGO_PKG_VERSION"));

/// How many times a fatal connection error retries re-establishing before the
/// Session gives up with a terminal [`Error::Connection`].
const RECONNECT_ATTEMPTS: usize = 3;
/// Delay between reconnect attempts.
const RECONNECT_BACKOFF: Duration = Duration::from_millis(200);

/// Username and password for basic authentication against Memgraph.
#[derive(Clone)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// The host and port identifying the one Memgraph server a Session connects to:
/// the *where* of a connection, distinct from the *how* ([`ConnectOptions`]).
/// `Display` is the single home for the `host:port` rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    host: String,
    port: u16,
}

impl Endpoint {
    /// Build an endpoint. Infallible: `u16` already bounds the port, and there is
    /// no invalid host clap can hand us that warrants a `Result` at every site.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Endpoint {
            host: host.into(),
            port,
        }
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// How a [`Session`] is opened: authentication and transport security. Retained
/// by the Session so a reconnect re-establishes with the same options.
#[derive(Clone, Default)]
pub struct ConnectOptions {
    /// Basic-auth credentials, or `None` for an anonymous connection.
    pub credentials: Option<Credentials>,
    /// Encrypt the Bolt stream with TLS (rustls; ADR 0007).
    pub use_tls: bool,
    /// Start the Session in read-only mode: every transaction runs with Bolt
    /// access mode READ, so the server rejects writes (the safety guard of
    /// CONTEXT.md "Read-only mode"). Set at connect time / via a profile; can be
    /// turned on later with [`Session::set_read_only`] but the Frontend refuses to
    /// turn it off at runtime.
    pub read_only: bool,
}

/// One reconnect attempt the Session is about to make after a fatal connection
/// error. Surfaced to a Frontend registered via [`Session::on_reconnect`] so it
/// can tell the user the link dropped and is being re-established (slice 16).
#[derive(Debug, Clone, Copy)]
pub struct ReconnectNotice {
    /// 1-based attempt number.
    pub attempt: usize,
    /// Total attempts the Session makes before surfacing a terminal error.
    pub max: usize,
}

/// A Frontend hook invoked once before each reconnect attempt.
type ReconnectObserver = Arc<dyn Fn(ReconnectNotice) + Send + Sync>;

pub struct Session {
    conn: SharedConn,
    /// The server this Session is bound to; reused verbatim on reconnect.
    endpoint: Endpoint,
    /// Retained so a reconnect re-authenticates identically.
    options: ConnectOptions,
    /// Liveness token of the last result issued, for the one-live-result guard.
    last: Option<Arc<ResultGuard>>,
    /// Optional Frontend hook notified before each reconnect attempt (ADR 0002:
    /// the Core stays Frontend-agnostic; surfacing is the Frontend's choice).
    on_reconnect: Option<ReconnectObserver>,
    /// Whether every transaction runs with Bolt access mode READ (the server
    /// rejects writes). Carried across reconnect so a dropped read-only guard
    /// re-establishes read-only.
    read_only: bool,
}

impl Session {
    /// Connect unauthenticated over plaintext (HELLO with `scheme: none`).
    pub async fn connect(endpoint: &Endpoint) -> Result<Self, Error> {
        Self::connect_with(endpoint, &ConnectOptions::default()).await
    }

    /// Connect to a Memgraph server and perform the Bolt handshake + HELLO,
    /// honouring `options`: TLS when requested, and basic auth when credentials
    /// are supplied (`scheme: basic`).
    ///
    /// A HELLO refusal is reported as [`Error::Auth`] so the Frontend can tell a
    /// rejected password apart from a transport failure; a TLS handshake failure
    /// surfaces as [`Error::Connection`].
    pub async fn connect_with(
        endpoint: &Endpoint,
        options: &ConnectOptions,
    ) -> Result<Self, Error> {
        let conn = establish(endpoint, options).await?;
        Ok(Self {
            conn,
            endpoint: endpoint.clone(),
            options: options.clone(),
            last: None,
            on_reconnect: None,
            read_only: options.read_only,
        })
    }

    /// Turn read-only mode on or off (CONTEXT.md "Read-only mode"). The Core
    /// allows either direction; the asymmetry "on at runtime, off only at connect"
    /// is a Frontend policy, not a Core invariant. Takes effect on the next query.
    pub fn set_read_only(&mut self, on: bool) {
        self.read_only = on;
    }

    /// Whether the Session is currently in read-only mode.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Register a hook called once before each reconnect attempt, so a Frontend
    /// can surface that the connection dropped and is being re-established. The
    /// Core itself prints nothing (ADR 0002).
    pub fn on_reconnect(&mut self, observer: impl Fn(ReconnectNotice) + Send + Sync + 'static) {
        self.on_reconnect = Some(Arc::new(observer));
    }

    /// Run a query with no parameters. See [`Session::run_with_params`].
    pub async fn run(&mut self, query: &str) -> Result<QueryResult, Error> {
        self.run_with_params(query, &BTreeMap::new()).await
    }

    /// Run a query bound to a set of named parameters and return its result.
    ///
    /// Errors with [`Error::ResultStillOpen`] if a prior result is still being
    /// read (ADR 0005). A query error keeps the Session usable for the next
    /// query; a fatal connection error reconnects (bounded) and retries once.
    pub async fn run_with_params(
        &mut self,
        query: &str,
        params: &BTreeMap<String, Value>,
    ) -> Result<QueryResult, Error> {
        self.enforce_one_live_result().await?;
        let bolt_params = encode_params(params)?;

        let (header, summary) = match self.run_once(query, bolt_params.clone()).await {
            Ok(reply) => reply,
            // Fatal transport error: reconnect (bounded) and retry once on the
            // fresh connection. Exhausted retries surface as a terminal error.
            Err(Error::Connection(_)) => {
                self.reconnect().await?;
                self.run_once(query, bolt_params).await?
            }
            Err(e) => return Err(e),
        };

        let guard = Arc::new(ResultGuard::default());
        self.last = Some(guard.clone());
        // In read-only mode the query ran inside an explicit `BEGIN {mode: r}`
        // transaction (Memgraph ignores access mode on auto-commit RUN, but
        // enforces it on a transaction), so the stream must COMMIT it once drained.
        let records = RecordStream::lazy(
            self.conn.clone(),
            DEFAULT_BATCH_SIZE,
            summary,
            guard,
            self.read_only,
        );
        Ok(QueryResult::new(header, records))
    }

    /// Issue one RUN. A `FAILURE` is cleared with `RESET` (so the Session stays
    /// usable) and returned as [`Error::Query`]; a transport failure becomes
    /// [`Error::Connection`].
    async fn run_once(
        &self,
        query: &str,
        params: Option<Params>,
    ) -> Result<(Vec<String>, Summary), Error> {
        let mut client = self.conn.lock().await;
        // In read-only mode, open an explicit transaction with Bolt access mode
        // READ before the query: Memgraph ignores `mode` on an auto-commit RUN but
        // enforces it on a transaction, so the server rejects writes — the Clause
        // scanner is never consulted. The stream COMMITs it once drained.
        if self.read_only {
            let begin = client
                .begin(Some(Metadata::from_iter([("mode", "r")])))
                .await
                .map_err(Error::connection)?;
            match begin {
                Message::Success(_) => {}
                Message::Failure(f) => {
                    let error = proto::query_error(f.metadata());
                    client.reset().await.map_err(Error::connection)?;
                    return Err(Error::Query(error));
                }
                other => {
                    return Err(Error::Protocol(format!("unexpected BEGIN reply: {other:?}")))
                }
            }
        }
        let reply = client
            .run(query, params, None)
            .await
            .map_err(Error::connection)?;
        match reply {
            Message::Success(s) => {
                Ok((proto::fields(s.metadata()), Summary::from_run(s.metadata())))
            }
            Message::Failure(f) => {
                let error = proto::query_error(f.metadata());
                // Bolt parks the connection in FAILED state after a FAILURE; RESET
                // clears it so the next query runs.
                client.reset().await.map_err(Error::connection)?;
                Err(Error::Query(error))
            }
            other => Err(Error::Protocol(format!("unexpected RUN reply: {other:?}"))),
        }
    }

    /// Enforce one live result at a time (ADR 0005). Returns
    /// [`Error::ResultStillOpen`] when the previous result is still being read,
    /// and `RESET`s the connection when it was abandoned (dropped before drain).
    async fn enforce_one_live_result(&mut self) -> Result<(), Error> {
        if let Some(prev) = self.last.take() {
            if !prev.is_done() {
                if Arc::strong_count(&prev) > 1 {
                    // The RecordStream clone is still alive: a result is open.
                    self.last = Some(prev);
                    return Err(Error::ResultStillOpen);
                }
                // Only the Session's clone remains: the stream was dropped before
                // draining. Clear its pending records before the next query.
                self.reset().await?;
            }
        }
        Ok(())
    }

    /// Send Bolt `RESET`, clearing any pending result or FAILED state.
    async fn reset(&self) -> Result<(), Error> {
        let mut client = self.conn.lock().await;
        client.reset().await.map_err(Error::connection)?;
        Ok(())
    }

    /// Re-establish the connection with the stored options, retrying up to
    /// [`RECONNECT_ATTEMPTS`] times before surfacing a terminal error.
    async fn reconnect(&mut self) -> Result<(), Error> {
        let mut last_err = String::from("no attempt made");
        for attempt in 0..RECONNECT_ATTEMPTS {
            if let Some(observer) = &self.on_reconnect {
                observer(ReconnectNotice {
                    attempt: attempt + 1,
                    max: RECONNECT_ATTEMPTS,
                });
            }
            if attempt > 0 {
                tokio::time::sleep(RECONNECT_BACKOFF).await;
            }
            match establish(&self.endpoint, &self.options).await {
                Ok(conn) => {
                    self.conn = conn;
                    self.last = None;
                    return Ok(());
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        Err(Error::Connection(format!(
            "reconnect to {} failed after {RECONNECT_ATTEMPTS} attempts: {last_err}",
            self.endpoint
        )))
    }
}

/// Open a connection and complete the Bolt handshake + HELLO, returning the
/// shared connection. Shared by the initial connect and by reconnect.
async fn establish(endpoint: &Endpoint, options: &ConnectOptions) -> Result<SharedConn, Error> {
    let stream = transport::connect_stream(endpoint, options.use_tls).await?;
    let mut client = Client::new(BufStream::new(stream).compat(), &[V4_4, V4_3, V4_2, V4_1])
        .await
        .map_err(Error::connection)?;

    let mut entries: Vec<(&str, &str)> = vec![("user_agent", USER_AGENT)];
    match &options.credentials {
        Some(creds) => {
            entries.push(("scheme", "basic"));
            entries.push(("principal", &creds.username));
            entries.push(("credentials", &creds.password));
        }
        None => entries.push(("scheme", "none")),
    }

    let hello = client
        .hello(Metadata::from_iter(entries))
        .await
        .map_err(Error::protocol)?;
    match hello {
        Message::Success(_) => Ok(Arc::new(Mutex::new(client))),
        Message::Failure(f) => Err(Error::Auth(proto::failure_message(f.metadata()))),
        other => Err(Error::Protocol(format!("HELLO refused: {other:?}"))),
    }
}

/// Encode named parameters into Bolt `Params`, or `None` when empty so the wire
/// message carries no parameter map at all.
fn encode_params(params: &BTreeMap<String, Value>) -> Result<Option<Params>, Error> {
    if params.is_empty() {
        return Ok(None);
    }
    let encoded = params
        .iter()
        .map(|(name, v)| Ok((name.clone(), value::to_bolt(v)?)))
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(Some(Params::from_iter(encoded)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_displays_as_host_colon_port() {
        let endpoint = Endpoint::new("localhost", 7687);
        assert_eq!(endpoint.to_string(), "localhost:7687");
        assert_eq!(endpoint.host(), "localhost");
        assert_eq!(endpoint.port(), 7687);
    }
}
