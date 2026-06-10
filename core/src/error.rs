//! Core error type and the Session's error taxonomy (slice 14).
//!
//! The central distinction is **recoverable query error** (the server rejected a
//! query but the Session survives) versus **fatal connection error** (the
//! transport died; the Session must reconnect). Retryability is keyed off the
//! query error's full/leaf code — never the `TransientError` tier, which the
//! ADR-0001 spike found wrapping a permanent validation error.

/// A query error the server reported in a Bolt `FAILURE`. The Session survives
/// it: after a `RESET` it can run the next query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    /// The server's full dotted error code, e.g.
    /// `Memgraph.ClientError.MemgraphError.MemgraphError`.
    pub code: String,
    /// The human-readable message.
    pub message: String,
}

impl QueryError {
    /// The leaf (last) segment of the dotted error code.
    pub fn leaf(&self) -> &str {
        self.code.rsplit('.').next().unwrap_or(&self.code)
    }

    /// Whether retrying the query could plausibly succeed. Keyed off the **leaf**
    /// code against a small known set — NOT the `TransientError` tier, because
    /// the spike observed a permanent error carrying a `TransientError` code
    /// (`Memgraph.TransientError.MemgraphError.MemgraphError`). Matching the tier
    /// would make that permanent error look retryable.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self.leaf(),
            "ConflictingTransactionsError" | "SerializationError"
        )
    }
}

/// An error from the Core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The connection could not be established or was lost (fatal — reconnect).
    #[error("connection error: {0}")]
    Connection(String),

    /// The server rejected the supplied credentials during the Bolt handshake.
    #[error("authentication failed: {0}")]
    Auth(String),

    /// The server rejected a query (recoverable — the Session survives).
    #[error("query error ({}): {}", .0.code, .0.message)]
    Query(QueryError),

    /// A supplied query parameter value cannot be encoded for Bolt (e.g. a node
    /// or path, which are results, not inputs).
    #[error("invalid query parameter: {0}")]
    Parameter(String),

    /// `run()` was called while a previous result was still live (ADR 0005).
    #[error("a previous result is still open; drain or discard it before running another query")]
    ResultStillOpen,

    /// The Bolt exchange was not understood (handshake/HELLO refused, etc.).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// Writing rendered output failed (I/O on the output sink).
    #[error("output error: {0}")]
    Output(String),
}

impl Error {
    /// Wrap a transport/driver failure as a fatal [`Error::Connection`]. Carries
    /// the stringify-and-wrap so call sites read `.map_err(Error::connection)`.
    pub(crate) fn connection(e: impl std::fmt::Display) -> Self {
        Error::Connection(e.to_string())
    }

    /// Wrap a misunderstood Bolt exchange as [`Error::Protocol`].
    pub(crate) fn protocol(e: impl std::fmt::Display) -> Self {
        Error::Protocol(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Output(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_is_the_last_code_segment() {
        let e = QueryError {
            code: "Memgraph.ClientError.MemgraphError.SyntaxError".to_string(),
            message: "bad cypher".to_string(),
        };
        assert_eq!(e.leaf(), "SyntaxError");
    }

    #[test]
    fn transient_tier_does_not_imply_retryable() {
        // The spike's exact observation: a permanent validation error wrapped in
        // a TransientError code. Keying off the tier would wrongly retry it.
        let permanent = QueryError {
            code: "Memgraph.TransientError.MemgraphError.MemgraphError".to_string(),
            message: "invalid localDateTime literal".to_string(),
        };
        assert!(
            !permanent.is_retryable(),
            "a TransientError-tier code must not be treated as retryable"
        );
    }

    #[test]
    fn known_conflict_leaf_is_retryable() {
        let conflict = QueryError {
            code: "Memgraph.TransientError.MemgraphError.ConflictingTransactionsError".to_string(),
            message: "conflicting transactions".to_string(),
        };
        assert!(conflict.is_retryable());
    }
}
