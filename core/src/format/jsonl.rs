//! Streaming JSONL writer (slice 23): one JSON object per Record, keyed by
//! column name, on its own line.
//!
//! JSON encoding per Value type:
//! - null/bool/integer/float/string/list/map → the natural JSON value
//!   (a non-finite float becomes `null`; bytes become an array of byte values)
//! - enum → its qualified string, e.g. `"Status::Active"`
//! - node → `{ "id", "labels": [...], "properties": {...} }`
//! - relationship → `{ "id", "start", "end", "type", "properties" }`
//! - unbound relationship → `{ "id", "type", "properties" }`
//! - path → `{ "nodes": [...], "relationships": [...] }`
//! - point → `{ "srid", "x", "y" [, "z"] }`
//! - date / time / datetime / duration → their textual string (as in tabular)

use std::io::Write;

use serde_json::{json, Map, Value as J};

use crate::render;
use crate::value::{Node, UnboundRelationship, Value};

use super::RowWriter;

pub struct JsonlWriter<W: Write> {
    sink: W,
    header: Vec<String>,
}

impl<W: Write> JsonlWriter<W> {
    pub fn new(sink: W) -> Self {
        Self {
            sink,
            header: Vec::new(),
        }
    }
}

impl<W: Write> RowWriter for JsonlWriter<W> {
    fn write_header(&mut self, header: &[String]) -> std::io::Result<()> {
        // JSONL has no header line; remember the keys for each row object.
        self.header = header.to_vec();
        Ok(())
    }

    fn write_row(&mut self, row: &[Value]) -> std::io::Result<()> {
        let mut obj = Map::new();
        for (i, value) in row.iter().enumerate() {
            let key = self.header.get(i).cloned().unwrap_or_else(|| i.to_string());
            obj.insert(key, json_value(value));
        }
        let line = serde_json::to_string(&J::Object(obj)).map_err(std::io::Error::other)?;
        writeln!(self.sink, "{line}")
    }

    fn finish(&mut self) -> std::io::Result<()> {
        self.sink.flush()
    }
}

fn json_props(m: &std::collections::BTreeMap<String, Value>) -> J {
    J::Object(m.iter().map(|(k, v)| (k.clone(), json_value(v))).collect())
}

fn node_json(n: &Node) -> J {
    json!({ "id": n.id, "labels": n.labels, "properties": json_props(&n.properties) })
}

fn unbound_json(r: &UnboundRelationship) -> J {
    json!({ "id": r.id, "type": r.rel_type, "properties": json_props(&r.properties) })
}

fn json_value(v: &Value) -> J {
    match v {
        Value::Null => J::Null,
        Value::Boolean(b) => J::Bool(*b),
        Value::Integer(i) => J::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f).map_or(J::Null, J::Number),
        Value::String(s) => J::String(s.clone()),
        Value::Bytes(b) => J::Array(b.iter().map(|x| J::Number(u64::from(*x).into())).collect()),
        Value::List(items) => J::Array(items.iter().map(json_value).collect()),
        Value::Map(m) => json_props(m),
        Value::Enum(q) => J::String(q.clone()),
        Value::Node(n) => node_json(n),
        Value::Relationship(r) => json!({
            "id": r.id,
            "start": r.start_node_id,
            "end": r.end_node_id,
            "type": r.rel_type,
            "properties": json_props(&r.properties),
        }),
        Value::UnboundRelationship(r) => unbound_json(r),
        Value::Path(p) => json!({
            "nodes": p.nodes.iter().map(node_json).collect::<Vec<_>>(),
            "relationships": p.relationships.iter().map(unbound_json).collect::<Vec<_>>(),
        }),
        Value::Point2d(p) => json!({ "srid": p.srid, "x": p.x, "y": p.y }),
        Value::Point3d(p) => json!({ "srid": p.srid, "x": p.x, "y": p.y, "z": p.z }),
        // Temporal types serialise as their textual form (same as tabular).
        Value::Date(_)
        | Value::Time(_, _)
        | Value::LocalTime(_)
        | Value::LocalDateTime(_)
        | Value::Duration(_)
        | Value::DateTimeOffset(_)
        | Value::DateTimeZoned(_) => J::String(render::tabular(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_json(header: &[&str], row: &[Value]) -> String {
        let mut buf = Vec::new();
        {
            let mut w = JsonlWriter::new(&mut buf);
            let hdr: Vec<String> = header
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            w.write_header(&hdr).unwrap();
            w.write_row(row).unwrap();
            w.finish().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn one_object_per_row_keyed_by_column() {
        let out = row_json(
            &["n", "name"],
            &[Value::Integer(1), Value::String("Ada".into())],
        );
        assert_eq!(out, "{\"n\":1,\"name\":\"Ada\"}\n");
    }

    #[test]
    fn temporal_and_enum_are_strings() {
        let out = row_json(&["e"], &[Value::Enum("Status::Active".into())]);
        assert_eq!(out, "{\"e\":\"Status::Active\"}\n");
    }

    #[test]
    fn output_is_valid_json() {
        let out = row_json(
            &["xs", "m"],
            &[
                Value::List(vec![Value::Integer(1), Value::Null]),
                Value::Map(
                    [("k".to_string(), Value::String("v".into()))]
                        .into_iter()
                        .collect(),
                ),
            ],
        );
        let parsed: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(parsed["xs"][0], 1);
        assert_eq!(parsed["m"]["k"], "v");
    }
}
