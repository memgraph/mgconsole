//! The Session: a single live conversation with one Memgraph server.
//!
//! The Bolt stack is async; the Session exposes `async` methods and a Frontend
//! calls `block_on` at the boundary (ADR 0002). A query returns a
//! [`QueryResult`] whose records stream lazily, pulled from the connection in
//! batches so memory stays bounded on large results (ADR 0004, slice 21). The
//! connection is shared with the live [`RecordStream`] via `Arc<Mutex<…>>`;
//! a result must be drained or discarded before the next query.

use std::sync::Arc;

use std::collections::BTreeMap;

use bolt_client::{Client, Metadata, Params};
use bolt_proto::{version::*, Message};
use tokio::io::BufStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

use crate::error::Error;
use crate::proto;
use crate::result::{QueryResult, RecordStream, Summary, DEFAULT_BATCH_SIZE};
use crate::transport::{self, MaybeTlsStream};
use crate::value::{self, Value};

pub(crate) type Conn = Client<Compat<BufStream<MaybeTlsStream>>>;
pub(crate) type SharedConn = Arc<Mutex<Conn>>;

const USER_AGENT: &str = concat!("mgconsole/", env!("CARGO_PKG_VERSION"));

/// Username and password for basic authentication against Memgraph.
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// How a [`Session`] is opened: authentication and transport security.
#[derive(Default)]
pub struct ConnectOptions {
    /// Basic-auth credentials, or `None` for an anonymous connection.
    pub credentials: Option<Credentials>,
    /// Encrypt the Bolt stream with TLS (rustls; ADR 0007).
    pub use_tls: bool,
}

pub struct Session {
    conn: SharedConn,
}

impl Session {
    /// Connect unauthenticated over plaintext (HELLO with `scheme: none`).
    pub async fn connect(host: &str, port: u16) -> Result<Self, Error> {
        Self::connect_with(host, port, &ConnectOptions::default()).await
    }

    /// Connect to a Memgraph server and perform the Bolt handshake + HELLO,
    /// honouring `options`: TLS when requested, and basic auth when credentials
    /// are supplied (`scheme: basic`).
    ///
    /// A HELLO refusal is reported as [`Error::Auth`] so the Frontend can tell a
    /// rejected password apart from a transport failure; a TLS handshake failure
    /// surfaces as [`Error::Connection`].
    pub async fn connect_with(
        host: &str,
        port: u16,
        options: &ConnectOptions,
    ) -> Result<Self, Error> {
        let stream = transport::connect_stream(host, port, options.use_tls).await?;
        let mut client = Client::new(BufStream::new(stream).compat(), &[V4_4, V4_3, V4_2, V4_1])
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;

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
            .map_err(|e| Error::Protocol(e.to_string()))?;
        match hello {
            Message::Success(_) => Ok(Self {
                conn: Arc::new(Mutex::new(client)),
            }),
            Message::Failure(f) => Err(Error::Auth(proto::failure_message(f.metadata()))),
            other => Err(Error::Protocol(format!("HELLO refused: {other:?}"))),
        }
    }

    /// Run a query with no parameters. See [`Session::run_with_params`].
    pub async fn run(&mut self, query: &str) -> Result<QueryResult, Error> {
        self.run_with_params(query, &BTreeMap::new()).await
    }

    /// Run a query bound to a set of named parameters and return its result. A
    /// query referencing `$name` resolves against `params["name"]`. The records
    /// stream lazily; the previous result must be fully consumed or discarded
    /// first.
    pub async fn run_with_params(
        &mut self,
        query: &str,
        params: &BTreeMap<String, Value>,
    ) -> Result<QueryResult, Error> {
        let bolt_params = encode_params(params)?;
        let header = {
            let mut client = self.conn.lock().await;
            match client
                .run(query, bolt_params, None)
                .await
                .map_err(|e| Error::Connection(e.to_string()))?
            {
                Message::Success(s) => proto::fields(s.metadata()),
                Message::Failure(f) => return Err(Error::Query(proto::failure_message(f.metadata()))),
                other => return Err(Error::Protocol(format!("unexpected RUN reply: {other:?}"))),
            }
        };

        let records = RecordStream::lazy(self.conn.clone(), DEFAULT_BATCH_SIZE);
        Ok(QueryResult::new(header, records, Summary::default()))
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
