//! Core error type.
//!
//! Slice 02 carries a minimal taxonomy; slice 14 elaborates the
//! recoverable-query-error vs fatal-connection-error distinction and the
//! reconnect policy. Kept exhaustive (no catch-all) so 14 extends it explicitly.

/// An error from the Core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The connection could not be established or was lost.
    #[error("connection error: {0}")]
    Connection(String),

    /// The server rejected the supplied credentials during the Bolt handshake.
    #[error("authentication failed: {0}")]
    Auth(String),

    /// The server rejected a query (e.g. bad Cypher). The Session survives.
    #[error("query error: {0}")]
    Query(String),

    /// A supplied query parameter value cannot be encoded for Bolt (e.g. a node
    /// or path, which are results, not inputs).
    #[error("invalid query parameter: {0}")]
    Parameter(String),

    /// The Bolt exchange was not understood (handshake/HELLO refused, etc.).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Writing rendered output failed (I/O on the output sink).
    #[error("output error: {0}")]
    Output(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Output(e.to_string())
    }
}
