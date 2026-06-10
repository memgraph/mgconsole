//! Small helpers for reading Bolt message metadata. Shared by the Session and
//! the `RecordStream` so the two agree on how a header, a failure message, and
//! the streaming `has_more` flag are extracted.

use std::collections::HashMap;

use bolt_proto::Value as BValue;

use crate::error::QueryError;

/// Column names from a RUN `SUCCESS` (`fields`).
pub(crate) fn fields(meta: &HashMap<String, BValue>) -> Vec<String> {
    match meta.get("fields") {
        Some(BValue::List(items)) => items
            .iter()
            .map(|v| match v {
                BValue::String(s) => s.clone(),
                other => format!("{other:?}"),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The human-readable message from a `FAILURE`.
pub(crate) fn failure_message(meta: &HashMap<String, BValue>) -> String {
    match meta.get("message") {
        Some(BValue::String(s)) => s.clone(),
        _ => "unknown query error".to_string(),
    }
}

/// The full dotted error code from a `FAILURE` (e.g.
/// `Memgraph.ClientError.MemgraphError.SyntaxError`).
pub(crate) fn failure_code(meta: &HashMap<String, BValue>) -> String {
    match meta.get("code") {
        Some(BValue::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Build a [`QueryError`] (code + message) from a `FAILURE`'s metadata.
pub(crate) fn query_error(meta: &HashMap<String, BValue>) -> QueryError {
    QueryError {
        code: failure_code(meta),
        message: failure_message(meta),
    }
}

/// Whether a PULL `SUCCESS` indicates more records remain on the stream.
pub(crate) fn has_more(meta: &HashMap<String, BValue>) -> bool {
    matches!(meta.get("has_more"), Some(BValue::Boolean(true)))
}
