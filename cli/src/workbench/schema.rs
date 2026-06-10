//! The database **Schema** — the labels, relationship types, and property keys
//! the database holds (CONTEXT.md's open-vocabulary dual of the closed keyword/
//! function set). Fetched via Memgraph's schema-metadata feature and used as a
//! second completion source (slice 12) and to back the schema sidebar (slice 13).
//!
//! Build-time-verified (issue 12) against Memgraph 3.10.1 with
//! `--storage-enable-schema-metadata=true`: the metadata procedures return
//! tabular rows, parsed here by a pure function. When the feature is off the
//! procedures error, and the workbench degrades silently to static-only
//! completion — it never scans the graph to derive names.

use std::collections::BTreeSet;

use mgconsole_core::{Record, Value};

use crate::syntax::CompletionSource;

/// Node-property metadata: rows of `(nodeType, nodeLabels, propertyName, …)`.
/// `RETURN`ed explicitly so the column order the parser relies on is fixed.
pub const NODE_PROPERTIES_QUERY: &str =
    "CALL schema.node_type_properties() YIELD nodeType, nodeLabels, propertyName \
     RETURN nodeType, nodeLabels, propertyName";

/// Relationship-property metadata: rows of `(relType, propertyName, …)`.
pub const REL_PROPERTIES_QUERY: &str =
    "CALL schema.rel_type_properties() YIELD relType, propertyName \
     RETURN relType, propertyName";

/// The names the database holds, the open completion/sidebar vocabulary.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Schema {
    pub labels: Vec<String>,
    pub rel_types: Vec<String>,
    pub property_keys: Vec<String>,
}

impl Schema {
    /// Every name as one flat list (labels, then relationship types, then
    /// property keys), for the completion source.
    pub fn all_names(&self) -> Vec<String> {
        self.labels
            .iter()
            .chain(&self.rel_types)
            .chain(&self.property_keys)
            .cloned()
            .collect()
    }
}

/// Parse the schema-metadata procedure results into a [`Schema`]. Pure: Records
/// in, names out. Node rows carry the labels (a list) in column 1 and a property
/// key in column 2; relationship rows carry the type (colon-and-backtick-quoted,
/// e.g. `:WORKS_AT` backtick-wrapped) in column 0 and a property key in column 1.
/// Names are de-duplicated and sorted.
pub fn parse_schema(node_rows: &[Record], rel_rows: &[Record]) -> Schema {
    let mut labels = BTreeSet::new();
    let mut rel_types = BTreeSet::new();
    let mut property_keys = BTreeSet::new();

    for row in node_rows {
        if let Some(Value::List(items)) = row.fields().get(1) {
            for item in items {
                if let Value::String(label) = item {
                    labels.insert(label.clone());
                }
            }
        }
        insert_property(&mut property_keys, row.fields().get(2));
    }
    for row in rel_rows {
        if let Some(Value::String(raw)) = row.fields().first() {
            rel_types.insert(strip_type(raw));
        }
        insert_property(&mut property_keys, row.fields().get(1));
    }

    Schema {
        labels: labels.into_iter().collect(),
        rel_types: rel_types.into_iter().collect(),
        property_keys: property_keys.into_iter().collect(),
    }
}

/// Insert a non-empty property-key cell into the set.
fn insert_property(set: &mut BTreeSet<String>, cell: Option<&Value>) {
    if let Some(Value::String(key)) = cell {
        if !key.is_empty() {
            set.insert(key.clone());
        }
    }
}

/// Strip Memgraph's metadata type spelling (a leading colon then a
/// backtick-wrapped name) down to the bare name.
fn strip_type(raw: &str) -> String {
    raw.trim_start_matches(':').trim_matches('`').to_string()
}

/// A completion source backed by the fetched [`Schema`] names — the open-
/// vocabulary dual of the static keyword/function source, matched
/// case-insensitively (labels/types are mixed-case, unlike the keyword tables).
pub struct SchemaSource {
    names: Vec<String>,
}

impl SchemaSource {
    pub fn new(names: Vec<String>) -> Self {
        Self { names }
    }
}

impl CompletionSource for SchemaSource {
    fn extend_matches(&self, prefix_upper: &str, out: &mut Vec<String>) {
        for name in &self.names {
            if name.to_uppercase().starts_with(prefix_upper) {
                out.push(name.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build records mirroring the metadata procedures' shape (verified against
    /// Memgraph 3.10.1 by the schema probe).
    fn node_row(labels: &[&str], property: &str) -> Record {
        Record::new(vec![
            Value::String(format!(":`{}`", labels.first().copied().unwrap_or(""))),
            Value::List(labels.iter().map(|l| Value::String((*l).to_string())).collect()),
            Value::String(property.to_string()),
        ])
    }

    fn rel_row(rel_type: &str, property: &str) -> Record {
        Record::new(vec![
            Value::String(format!(":`{rel_type}`")),
            Value::String(property.to_string()),
        ])
    }

    #[test]
    fn parses_labels_rel_types_and_property_keys() {
        let nodes = vec![
            node_row(&["Person"], "name"),
            node_row(&["Person"], "age"),
            node_row(&["Company"], "title"),
        ];
        let rels = vec![rel_row("WORKS_AT", "since")];
        let schema = parse_schema(&nodes, &rels);
        assert_eq!(schema.labels, vec!["Company", "Person"]); // sorted, deduped
        assert_eq!(schema.rel_types, vec!["WORKS_AT"]);
        assert_eq!(schema.property_keys, vec!["age", "name", "since", "title"]);
    }

    #[test]
    fn an_empty_property_name_is_ignored() {
        let schema = parse_schema(&[node_row(&["Loner"], "")], &[]);
        assert_eq!(schema.labels, vec!["Loner"]);
        assert!(schema.property_keys.is_empty());
    }

    #[test]
    fn the_source_matches_names_case_insensitively() {
        let source = SchemaSource::new(vec!["Person".to_string(), "WORKS_AT".to_string()]);
        let mut out = Vec::new();
        source.extend_matches("PER", &mut out);
        assert_eq!(out, vec!["Person"]);
    }
}
