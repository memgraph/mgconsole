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

/// The Session's explicit-transaction state (ADR 0011, CONTEXT.md "Transaction").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    /// Autocommit: each query is its own transaction (the default).
    Auto,
    /// A user transaction is open (`:begin`); queries run within it until
    /// `:commit` or `:rollback`.
    Open,
    /// A query failed inside the open transaction, poisoning it; the transaction
    /// can make no further progress and only `:rollback` recovers (ADR 0011).
    Failed,
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
    /// The explicit-transaction state (ADR 0011): autocommit, an open user
    /// transaction, or a poisoned one awaiting `:rollback`.
    tx: TransactionState,
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
            tx: TransactionState::Auto,
        })
    }

    /// The current explicit-transaction state (for the prompt/status marker).
    pub fn transaction_state(&self) -> TransactionState {
        self.tx
    }

    /// The Endpoint this Session is connected to (for the prompt/status, and as
    /// the fallback host/port when `:connect` is given a bare host, issue 07).
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Open an explicit transaction (`:begin`, ADR 0011). Subsequent queries run
    /// within it until [`commit`](Self::commit)/[`rollback`](Self::rollback). In
    /// read-only mode the transaction carries Bolt access mode READ, so writes are
    /// rejected (issue 04). Errors if a transaction is already open.
    pub async fn begin(&mut self) -> Result<(), Error> {
        match self.tx {
            TransactionState::Auto => {}
            TransactionState::Open => {
                return Err(Error::Transaction(
                    "a transaction is already open; :commit or :rollback first".to_string(),
                ))
            }
            TransactionState::Failed => return Err(Error::TransactionFailed),
        }
        self.enforce_one_live_result().await?;
        // `:begin` is autocommit-state work, so a connection loss here reconnects
        // and retries once (like an autocommit query) — nothing bracketed is yet
        // at stake (ADR 0011).
        match self.begin_once().await {
            Ok(()) => {
                self.tx = TransactionState::Open;
                Ok(())
            }
            Err(Error::Connection(_)) => {
                self.reconnect().await?;
                self.begin_once().await?;
                self.tx = TransactionState::Open;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Issue one BEGIN with the current access mode. A `FAILURE` is cleared with
    /// `RESET`; a transport failure becomes [`Error::Connection`] for the caller
    /// to reconnect.
    async fn begin_once(&self) -> Result<(), Error> {
        let metadata = self
            .read_only
            .then(|| Metadata::from_iter([("mode", "r")]));
        let mut client = self.conn.lock().await;
        match client.begin(metadata).await.map_err(Error::connection)? {
            Message::Success(_) => Ok(()),
            Message::Failure(f) => {
                let error = proto::query_error(f.metadata());
                client.reset().await.map_err(Error::connection)?;
                Err(Error::Query(error))
            }
            other => Err(Error::Protocol(format!("unexpected BEGIN reply: {other:?}"))),
        }
    }

    /// Commit the open transaction (`:commit`), returning to autocommit. Errors if
    /// no transaction is open, or if it is poisoned (rollback it instead).
    pub async fn commit(&mut self) -> Result<(), Error> {
        match self.tx {
            TransactionState::Auto => {
                return Err(Error::Transaction("no transaction to commit".to_string()))
            }
            TransactionState::Failed => return Err(Error::TransactionFailed),
            TransactionState::Open => {}
        }
        self.enforce_one_live_result().await?;
        let mut client = self.conn.lock().await;
        match client.commit().await {
            // A transport failure during COMMIT means the uncommitted work is gone;
            // never silently resurrect it — abort to autocommit (ADR 0011).
            Err(_) => {
                drop(client);
                self.tx = TransactionState::Auto;
                Err(Error::TransactionAborted)
            }
            Ok(Message::Success(_)) => {
                drop(client);
                self.tx = TransactionState::Auto;
                Ok(())
            }
            Ok(Message::Failure(f)) => {
                let error = proto::query_error(f.metadata());
                client.reset().await.map_err(Error::connection)?;
                self.tx = TransactionState::Auto;
                Err(Error::Query(error))
            }
            Ok(other) => Err(Error::Protocol(format!("unexpected COMMIT reply: {other:?}"))),
        }
    }

    /// Roll back the open transaction (`:rollback`), returning to autocommit. A
    /// clean open transaction is `ROLLBACK`ed; a poisoned one (the server is in
    /// FAILED state) is cleared with `RESET`. Errors if no transaction is open.
    pub async fn rollback(&mut self) -> Result<(), Error> {
        match self.tx {
            TransactionState::Auto => {
                return Err(Error::Transaction("no transaction to roll back".to_string()))
            }
            TransactionState::Open => {
                self.enforce_one_live_result().await?;
                let mut client = self.conn.lock().await;
                match client.rollback().await {
                    // Either a clean ROLLBACK, or a connection loss that already
                    // discarded the transaction — both meet the goal (back to
                    // autocommit), so report success.
                    Err(_) | Ok(Message::Success(_)) => {}
                    Ok(Message::Failure(f)) => {
                        let error = proto::query_error(f.metadata());
                        client.reset().await.map_err(Error::connection)?;
                        self.tx = TransactionState::Auto;
                        return Err(Error::Query(error));
                    }
                    Ok(other) => {
                        return Err(Error::Protocol(format!(
                            "unexpected ROLLBACK reply: {other:?}"
                        )))
                    }
                }
            }
            TransactionState::Failed => {
                // The server is parked in FAILED state; RESET clears it to Ready.
                let mut client = self.conn.lock().await;
                client.reset().await.map_err(Error::connection)?;
            }
        }
        self.tx = TransactionState::Auto;
        Ok(())
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

    /// Switch the active Database within this Session (`:use`, issue 08) via
    /// Memgraph multi-tenancy `USE DATABASE`. The connection is unchanged — same
    /// Endpoint, same Session. A failure (unknown database, or no multi-tenancy
    /// license) surfaces as [`Error::Query`] and leaves the current Database
    /// active. The empty result is drained so the Session is ready for the next
    /// query.
    pub async fn use_database(&mut self, database: &str) -> Result<(), Error> {
        let mut result = self.run(&format!("USE DATABASE {database}")).await?;
        result.records().discard().await?;
        Ok(())
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
        // A poisoned transaction can make no progress until rolled back (ADR 0011).
        if self.tx == TransactionState::Failed {
            return Err(Error::TransactionFailed);
        }
        self.enforce_one_live_result().await?;
        let bolt_params = encode_params(params)?;

        // Autocommit read-only wraps each query in an internal `BEGIN {mode: r}`
        // (issue 04); inside an open user transaction the access mode was already
        // set by `:begin`, so there is no per-query wrap and the user commits.
        let wrap_read_only = self.tx == TransactionState::Auto && self.read_only;

        let (header, summary) = match self.run_once(query, bolt_params.clone(), wrap_read_only).await
        {
            Ok(reply) => reply,
            // Fatal transport error. State-dependent (ADR 0011): in autocommit,
            // reconnect (bounded) and retry once on the fresh connection. Inside an
            // open transaction, the uncommitted work is gone — never silently
            // resurrect bracketed work — so abort to autocommit without retrying;
            // the next query reconnects lazily.
            Err(Error::Connection(_)) if self.tx != TransactionState::Auto => {
                self.tx = TransactionState::Auto;
                return Err(Error::TransactionAborted);
            }
            Err(Error::Connection(_)) => {
                self.reconnect().await?;
                self.run_once(query, bolt_params, wrap_read_only).await?
            }
            // A query error inside an open transaction poisons it (ADR 0011): the
            // server is left in FAILED state and only `:rollback` recovers.
            Err(e @ Error::Query(_)) if self.tx == TransactionState::Open => {
                self.tx = TransactionState::Failed;
                return Err(e);
            }
            Err(e) => return Err(e),
        };

        let guard = Arc::new(ResultGuard::default());
        self.last = Some(guard.clone());
        // Only an internal autocommit read-only wrap is committed by the stream;
        // a user transaction is committed explicitly by `:commit`.
        let records = RecordStream::lazy(
            self.conn.clone(),
            DEFAULT_BATCH_SIZE,
            summary,
            guard,
            wrap_read_only,
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
        wrap_read_only: bool,
    ) -> Result<(Vec<String>, Summary), Error> {
        let mut client = self.conn.lock().await;
        // Autocommit read-only opens an internal transaction with Bolt access mode
        // READ before the query: Memgraph ignores `mode` on an auto-commit RUN but
        // enforces it on a transaction, so the server rejects writes — the Clause
        // scanner is never consulted. The stream COMMITs it once drained.
        if wrap_read_only {
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
                // Bolt parks the connection in FAILED state after a FAILURE. In
                // autocommit, RESET clears it so the next query runs. Inside an
                // open user transaction we deliberately leave it FAILED — the
                // transaction is poisoned and only `:rollback` recovers (ADR 0011).
                if self.tx != TransactionState::Open {
                    client.reset().await.map_err(Error::connection)?;
                }
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
                    // A reconnect re-establishes a fresh connection; any open
                    // transaction is gone (issue 06 refines how this surfaces).
                    self.tx = TransactionState::Auto;
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
