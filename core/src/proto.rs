//! Small helpers for reading Bolt message metadata. Shared by the Session and
//! the RecordStream so the two agree on how a header, a failure message, and
//! the streaming `has_more` flag are extracted.

use std::collections::HashMap;

use bolt_proto::Value as BValue;

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

/// Whether a PULL `SUCCESS` indicates more records remain on the stream.
pub(crate) fn has_more(meta: &HashMap<String, BValue>) -> bool {
    matches!(meta.get("has_more"), Some(BValue::Boolean(true)))
}
