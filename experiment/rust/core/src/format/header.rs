//! The column names of a query result, given a type at the format seam.
//!
//! A query's header and each row are co-derived from one query, so they cannot
//! realistically differ in length. [`Header`] makes that a length-safe pairing
//! ([`Header::zip`]) rather than an index-and-fallback at each writer, and gives
//! the bare `&[String]` a cohesive type where it threads through the writers and
//! the tabular renderer.

use crate::value::Value;

/// The column names of a query result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header(Vec<String>);

impl Header {
    /// Build a header from the column names (accepts `&[String]` or `Vec<String>`).
    pub fn new(names: impl Into<Vec<String>>) -> Self {
        Header(names.into())
    }

    /// The column names, e.g. for CSV's header row.
    pub fn names(&self) -> &[String] {
        &self.0
    }

    /// Whether there are no columns (a write query returns an empty header).
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Pair each column name with the row value in the same position, in order.
    /// Yields only as many pairs as the shorter of names and values — there is
    /// no positional fallback for a value with no name.
    pub fn zip<'a>(&'a self, row: &'a [Value]) -> impl Iterator<Item = (&'a str, &'a Value)> {
        self.0.iter().map(String::as_str).zip(row.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_pairs_names_with_values_in_order() {
        let header = Header::new(vec!["n".to_string(), "name".to_string()]);
        let row = vec![Value::Integer(1), Value::String("Ada".into())];
        let pairs: Vec<_> = header.zip(&row).collect();
        assert_eq!(
            pairs,
            vec![
                ("n", &Value::Integer(1)),
                ("name", &Value::String("Ada".into())),
            ]
        );
    }

    #[test]
    fn zip_yields_no_pair_for_a_value_without_a_name() {
        // A value with no matching name is dropped, not given a positional key.
        let header = Header::new(vec!["only".to_string()]);
        let row = vec![Value::Integer(1), Value::Integer(2)];
        let pairs: Vec<_> = header.zip(&row).collect();
        assert_eq!(pairs, vec![("only", &Value::Integer(1))]);
    }

    #[test]
    fn names_round_trip_and_empty_is_detected() {
        assert!(Header::new(Vec::new()).is_empty());
        let header = Header::new(["a".to_string(), "b".to_string()].as_slice());
        assert_eq!(header.names(), &["a".to_string(), "b".to_string()]);
    }
}
