//! Small helpers for reading Bolt message metadata. Shared by the Session and
//! the `RecordStream` so the two agree on how a header, a failure message, and
//! the streaming `has_more` flag are extracted.
//!
//! The known metadata keys form a closed, typed schema: each [`Key<T>`] names a
//! key string *and* the value type it projects, and [`Meta`] is the single place
//! the `match get(key) { Some(variant) => …, _ => None }` shape lives. A typo in
//! a key name is now a missing `const`, not a silent default; the per-key domain
//! defaults (empty string, empty list) stay in the readers below.

use std::collections::HashMap;
use std::marker::PhantomData;

use bolt_proto::Value as BValue;

use crate::error::QueryError;

/// A typed key into Bolt metadata: the key string plus the type its value
/// projects to. Used only as a `const` (see the key definitions below), so it
/// needs no runtime constructor surface beyond [`Key::new`].
pub(crate) struct Key<T> {
    name: &'static str,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Key<T> {
    const fn new(name: &'static str) -> Self {
        Key {
            name,
            _marker: PhantomData,
        }
    }
}

// A `Key` is a name plus a zero-size type tag, so it is freely copyable. Hand
// impls (not `derive`) keep `Copy` independent of whether `T: Copy`.
impl<T> Clone for Key<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Key<T> {}

/// Extraction of a Rust value from the matching `bolt_proto::Value` variant.
/// Returning `None` covers both an absent key and a present-but-wrong variant,
/// so the caller's domain default applies uniformly.
pub(crate) trait FromBolt: Sized {
    fn from_bolt(value: &BValue) -> Option<Self>;
}

impl FromBolt for String {
    fn from_bolt(value: &BValue) -> Option<Self> {
        match value {
            BValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }
}

impl FromBolt for bool {
    fn from_bolt(value: &BValue) -> Option<Self> {
        match value {
            BValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }
}

impl FromBolt for Vec<String> {
    fn from_bolt(value: &BValue) -> Option<Self> {
        match value {
            // Non-string list items keep the legacy debug-stringify fallback, so
            // a `fields` list stays a `Vec<String>` whatever it carries.
            BValue::List(items) => Some(
                items
                    .iter()
                    .map(|v| match v {
                        BValue::String(s) => s.clone(),
                        other => format!("{other:?}"),
                    })
                    .collect(),
            ),
            _ => None,
        }
    }
}

/// A borrowing view over a Bolt metadata map that reads through typed [`Key`]s.
pub(crate) struct Meta<'a>(&'a HashMap<String, BValue>);

impl<'a> Meta<'a> {
    pub(crate) fn new(map: &'a HashMap<String, BValue>) -> Self {
        Meta(map)
    }

    /// View an inner `Value::Map` (e.g. one notification's fields) as a `Meta`,
    /// so nested maps read through the same typed keys. `None` if not a map.
    pub(crate) fn from_value(value: &'a BValue) -> Option<Self> {
        match value {
            BValue::Map(m) => Some(Meta(m)),
            _ => None,
        }
    }

    /// The value at `key`, projected to its type, or `None` if absent or of the
    /// wrong variant.
    pub(crate) fn get<T: FromBolt>(&self, key: Key<T>) -> Option<T> {
        self.0.get(key.name).and_then(T::from_bolt)
    }
}

/// Column names from a RUN `SUCCESS`.
pub(crate) const FIELDS: Key<Vec<String>> = Key::new("fields");
/// The full dotted error code from a `FAILURE`, or a notification's code.
pub(crate) const CODE: Key<String> = Key::new("code");
/// The human-readable message from a `FAILURE`.
pub(crate) const MESSAGE: Key<String> = Key::new("message");
/// Whether a PULL `SUCCESS` indicates more records remain.
pub(crate) const HAS_MORE: Key<bool> = Key::new("has_more");
/// A notification's short title.
pub(crate) const TITLE: Key<String> = Key::new("title");
/// A notification's long description.
pub(crate) const DESCRIPTION: Key<String> = Key::new("description");
/// A notification's severity.
pub(crate) const SEVERITY: Key<String> = Key::new("severity");

/// Column names from a RUN `SUCCESS` (`fields`).
pub(crate) fn fields(meta: &HashMap<String, BValue>) -> Vec<String> {
    Meta::new(meta).get(FIELDS).unwrap_or_default()
}

/// The human-readable message from a `FAILURE`.
pub(crate) fn failure_message(meta: &HashMap<String, BValue>) -> String {
    Meta::new(meta)
        .get(MESSAGE)
        .unwrap_or_else(|| "unknown query error".to_string())
}

/// The full dotted error code from a `FAILURE` (e.g.
/// `Memgraph.ClientError.MemgraphError.SyntaxError`).
pub(crate) fn failure_code(meta: &HashMap<String, BValue>) -> String {
    Meta::new(meta).get(CODE).unwrap_or_default()
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
    Meta::new(meta).get(HAS_MORE).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: Vec<(&str, BValue)>) -> HashMap<String, BValue> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn present_key_projects_its_value() {
        let m = map(vec![("code", BValue::String("E_OOPS".to_string()))]);
        assert_eq!(Meta::new(&m).get(CODE), Some("E_OOPS".to_string()));
    }

    #[test]
    fn missing_key_is_none() {
        let m = map(vec![]);
        assert_eq!(Meta::new(&m).get(CODE), None);
        assert_eq!(Meta::new(&m).get(HAS_MORE), None);
    }

    #[test]
    fn wrong_variant_is_none() {
        // `code` present but as a bool, not a string.
        let m = map(vec![("code", BValue::Boolean(true))]);
        assert_eq!(Meta::new(&m).get(CODE), None);
    }

    #[test]
    fn bool_key_projects() {
        let m = map(vec![("has_more", BValue::Boolean(true))]);
        assert_eq!(Meta::new(&m).get(HAS_MORE), Some(true));
    }

    #[test]
    fn fields_list_projects_to_strings() {
        let m = map(vec![(
            "fields",
            BValue::List(vec![
                BValue::String("n".to_string()),
                BValue::String("m".to_string()),
            ]),
        )]);
        assert_eq!(
            Meta::new(&m).get(FIELDS),
            Some(vec!["n".to_string(), "m".to_string()])
        );
    }

    #[test]
    fn inner_map_reads_through_meta() {
        let inner = BValue::Map(map(vec![
            ("code", BValue::String("Hint".to_string())),
            ("title", BValue::String("use an index".to_string())),
        ]));
        let child = Meta::from_value(&inner).expect("a map is a child Meta");
        assert_eq!(child.get(CODE), Some("Hint".to_string()));
        assert_eq!(child.get(TITLE), Some("use an index".to_string()));
        assert_eq!(child.get(SEVERITY), None);
    }

    #[test]
    fn from_value_rejects_non_map() {
        assert!(Meta::from_value(&BValue::Boolean(true)).is_none());
    }
}
