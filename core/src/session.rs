//! The Session: a single live conversation with one Memgraph server.
//!
//! The Bolt stack is async; the Session exposes `async` methods and a Frontend
//! calls `block_on` at the boundary (ADR 0002). Slice 02 connects by host/port
//! and runs one query, returning a [`QueryResult`]. Auth (slice 10), TLS
//! (slice 11), parameters (slice 12), the summary (slice 13), and the error
//! taxonomy + reconnect (slice 14) extend this.

use std::collections::HashMap;

use bolt_client::{Client, Metadata};
use bolt_proto::{version::*, Message};
use tokio::io::BufStream;
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

use crate::error::Error;
use crate::result::{QueryResult, Record, RecordStream, Summary};
use crate::value::Value;

type Conn = Client<Compat<BufStream<TcpStream>>>;

const USER_AGENT: &str = concat!("mgconsole/", env!("CARGO_PKG_VERSION"));

pub struct Session {
    client: Conn,
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
            Message::Success(_) => Ok(Self { client }),
            other => Err(Error::Protocol(format!("HELLO refused: {other:?}"))),
        }
    }

    /// Run a query and return its result: header, record stream, and summary.
    pub async fn run(&mut self, query: &str) -> Result<QueryResult, Error> {
        let run_reply = self
            .client
            .run(query, None, None)
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        let header = match run_reply {
            Message::Success(s) => extract_fields(s.metadata()),
            Message::Failure(f) => return Err(Error::Query(failure_message(f.metadata()))),
            other => return Err(Error::Protocol(format!("unexpected RUN reply: {other:?}"))),
        };

        let (records, end) = self
            .client
            .pull(Some(Metadata::from_iter(vec![("n", -1_i64)])))
            .await
            .map_err(|e| Error::Connection(e.to_string()))?;
        match end {
            Message::Success(_) => {}
            Message::Failure(f) => return Err(Error::Query(failure_message(f.metadata()))),
            other => return Err(Error::Protocol(format!("unexpected PULL reply: {other:?}"))),
        }

        let core_records = records
            .into_iter()
            .map(|r| Record::new(r.fields().iter().cloned().map(Value::from).collect()))
            .collect();

        Ok(QueryResult::new(
            header,
            RecordStream::from_buffered(core_records),
            Summary::default(),
        ))
    }
}

fn extract_fields(meta: &HashMap<String, bolt_proto::Value>) -> Vec<String> {
    match meta.get("fields") {
        Some(bolt_proto::Value::List(items)) => items
            .iter()
            .map(|v| match v {
                bolt_proto::Value::String(s) => s.clone(),
                other => format!("{other:?}"),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn failure_message(meta: &HashMap<String, bolt_proto::Value>) -> String {
    match meta.get("message") {
        Some(bolt_proto::Value::String(s)) => s.clone(),
        _ => "unknown query error".to_string(),
    }
}
