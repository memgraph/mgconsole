//! The Session: a single live conversation with one Memgraph server.
//!
//! The Bolt stack is async; the Session exposes `async` methods and a Frontend
//! calls `block_on` at the boundary (ADR 0002). A query returns a
//! [`QueryResult`] whose records stream lazily, pulled from the connection in
//! batches so memory stays bounded on large results (ADR 0004, slice 21). The
//! connection is shared with the live [`RecordStream`] via `Arc<Mutex<…>>`;
//! a result must be drained or discarded before the next query.

use std::sync::Arc;

use bolt_client::{Client, Metadata};
use bolt_proto::{version::*, Message};
use tokio::io::BufStream;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

use crate::error::Error;
use crate::proto;
use crate::result::{QueryResult, RecordStream, Summary, DEFAULT_BATCH_SIZE};

pub(crate) type Conn = Client<Compat<BufStream<TcpStream>>>;
pub(crate) type SharedConn = Arc<Mutex<Conn>>;

const USER_AGENT: &str = concat!("mgconsole/", env!("CARGO_PKG_VERSION"));

pub struct Session {
    conn: SharedConn,
}

impl Session {
    /// Connect to a Memgraph server by host and port and perform the Bolt
    /// handshake + HELLO (unauthenticated for now).
    pub async fn connect(host: &str, port: u16) -> Result<Self, Error> {
        let tcp = TcpStream::connect((host, port))
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        let mut client = Client::new(BufStream::new(tcp).compat(), &[V4_4, V4_3, V4_2, V4_1])
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        let hello = client
            .hello(Metadata::from_iter(vec![
                ("user_agent", USER_AGENT),
                ("scheme", "none"),
            ]))
            .await
            .map_err(|e| Error::Protocol(e.to_string()))?;
        match hello {
            Message::Success(_) => Ok(Self {
                conn: Arc::new(Mutex::new(client)),
            }),
            other => Err(Error::Protocol(format!("HELLO refused: {other:?}"))),
        }
    }

    /// Run a query and return its result. The records stream lazily; the
    /// previous result must be fully consumed or discarded first.
    pub async fn run(&mut self, query: &str) -> Result<QueryResult, Error> {
        let header = {
            let mut client = self.conn.lock().await;
            match client
                .run(query, None, None)
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
