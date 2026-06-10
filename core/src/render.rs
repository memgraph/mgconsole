//! Tabular rendering of a Core [`Value`].
//!
//! Slice 02 renders scalars faithfully; composite and Memgraph-specific types
//! get a provisional rendering that the dedicated rendering slices (03–07)
//! replace with golden-tested output. The match is exhaustive with no wildcard
//! arm (ADR 0003), so each later slice is handed a compile error at the arm it
//! must refine.

use crate::value::Value;

/// Format a float so it is always distinguishable from an integer (`3.0`, not
/// `3`), matching Cypher's float display.
fn float(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_string();
    }
    if f.is_infinite() {
        return if f < 0.0 { "-Inf" } else { "Inf" }.to_string();
    }
    let s = format!("{f}");
    if s.contains(['.', 'e', 'E']) {
        s
    } else {
        format!("{s}.0")
    }
}

/// Render a duration as Memgraph does: `P{days}DT{h}H{m}M{s}.{micros:06}S`.
/// The time-of-day part is decomposed from the `seconds` field; `months` (which
/// Memgraph does not emit) is included only if non-zero, to stay lossless.
fn duration(d: &crate::value::Duration) -> String {
    let hours = d.seconds / 3600;
    let minutes = (d.seconds % 3600) / 60;
    let seconds = d.seconds % 60;
    let micros = (d.nanos / 1000).abs();
    let mut out = String::from("P");
    if d.months != 0 {
        out.push_str(&format!("{}M", d.months));
    }
    out.push_str(&format!(
        "{}DT{}H{}M{}.{:06}S",
        d.days, hours, minutes, seconds, micros
    ));
    out
}

/// Escape control whitespace so a string stays on one tabular line. Quotes are
/// left literal here; quote-escaping is format-specific (CSV, slice 22).
fn escape_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Render a single Value as plain tabular text.
///
/// A top-level string cell is unquoted; strings nested inside a container are
/// quoted (mgconsole/Cypher convention). `quote` carries that distinction —
/// recursion into containers always sets it.
pub fn tabular(value: &Value) -> String {
    render(value, false)
}

fn render(value: &Value, quote: bool) -> String {
    match value {
        Value::Null => "Null".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => float(*f),
        Value::String(s) => {
            if quote {
                quoted(s)
            } else {
                escape_ws(s)
            }
        }
        Value::Bytes(b) => format!("{b:?}"),
        Value::Enum(q) => q.clone(),
        Value::List(items) => {
            let inner: Vec<String> = items.iter().map(|v| render(v, true)).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Map(m) => format!("{{{}}}", render_pairs(m)),
        Value::Node(n) => render_node(n),
        Value::Relationship(r) => {
            rel_body(&r.rel_type, &r.properties)
        }
        Value::UnboundRelationship(r) => rel_body(&r.rel_type, &r.properties),
        Value::Path(p) => render_path(p),
        // Temporals match Memgraph's textual conventions: 6-digit microseconds
        // throughout, and a named-zone datetime shows both offset and [Zone].
        Value::Date(d) => d.format("%Y-%m-%d").to_string(),
        Value::Time(t, off) => format!("{}{}", t.format("%H:%M:%S%.6f"), off),
        Value::LocalTime(t) => t.format("%H:%M:%S%.6f").to_string(),
        Value::LocalDateTime(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
        Value::Duration(d) => duration(d),
        Value::DateTimeOffset(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string(),
        Value::DateTimeZoned(dt) => format!(
            "{}[{}]",
            dt.format("%Y-%m-%dT%H:%M:%S%.6f%:z"),
            dt.timezone().name()
        ),
        // Provisional renderings — refined by slice 07.
        Value::Point2d(p) => format!("{:?}", p),
        Value::Point3d(p) => format!("{:?}", p),
    }
}

/// `(:Label1:Label2 {props})`. No labels and/or no props collapse cleanly.
fn render_node(n: &crate::value::Node) -> String {
    let labels: String = n.labels.iter().map(|l| format!(":{l}")).collect();
    let mut inner = labels;
    if !n.properties.is_empty() {
        if !inner.is_empty() {
            inner.push(' ');
        }
        inner.push_str(&format!("{{{}}}", render_pairs(&n.properties)));
    }
    format!("({inner})")
}

/// `[:TYPE {props}]` — shared by bound and unbound relationships (the standalone
/// form shows no endpoints; direction is supplied by the path).
fn rel_body(rel_type: &str, properties: &std::collections::BTreeMap<String, Value>) -> String {
    let mut inner = format!(":{rel_type}");
    if !properties.is_empty() {
        inner.push_str(&format!(" {{{}}}", render_pairs(properties)));
    }
    format!("[{inner}]")
}

/// Reconstruct a path as `(n)-[r]->(n)...` from Bolt's node/rel/sequence form.
/// The sequence alternates a signed 1-based relationship index (sign = forward
/// vs backward) and a 0-based node index for the hop's far end.
fn render_path(p: &crate::value::Path) -> String {
    let mut out = String::new();
    if let Some(first) = p.nodes.first() {
        out.push_str(&render_node(first));
    }
    for hop in p.sequence.chunks_exact(2) {
        let rel_signed = hop[0];
        let node_idx = hop[1] as usize;
        let rel = &p.relationships[rel_signed.unsigned_abs() as usize - 1];
        let body = rel_body(&rel.rel_type, &rel.properties);
        if rel_signed >= 0 {
            out.push_str(&format!("-{body}->"));
        } else {
            out.push_str(&format!("<-{body}-"));
        }
        out.push_str(&render_node(&p.nodes[node_idx]));
    }
    out
}

/// Render `key: value` pairs (unquoted keys, nested values), comma-separated.
fn render_pairs(m: &std::collections::BTreeMap<String, Value>) -> String {
    m.iter()
        .map(|(k, v)| format!("{k}: {}", render(v, true)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A quoted, fully-escaped string for nested contexts.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Node, Path, Relationship, UnboundRelationship};

    #[test]
    fn renders_scalars() {
        assert_eq!(tabular(&Value::Integer(42)), "42");
        assert_eq!(tabular(&Value::String("hi".into())), "hi");
        assert_eq!(tabular(&Value::Boolean(true)), "true");
        assert_eq!(tabular(&Value::Null), "Null");
    }

    #[test]
    fn renders_enum_qualified() {
        assert_eq!(tabular(&Value::Enum("Status::Active".into())), "Status::Active");
    }

    #[test]
    fn floats_keep_a_decimal_point() {
        assert_eq!(tabular(&Value::Float(3.0)), "3.0");
        assert_eq!(tabular(&Value::Float(123.456)), "123.456");
        assert_eq!(tabular(&Value::Float(-0.5)), "-0.5");
    }

    #[test]
    fn lists_and_maps_quote_nested_strings() {
        assert_eq!(tabular(&Value::List(vec![])), "[]");
        assert_eq!(
            tabular(&Value::List(vec![Value::Integer(1), Value::Integer(2)])),
            "[1, 2]"
        );
        // Strings nested in a container are quoted (unlike a top-level cell).
        assert_eq!(
            tabular(&Value::List(vec![
                Value::String("a".into()),
                Value::String("b".into())
            ])),
            "[\"a\", \"b\"]"
        );
        assert_eq!(tabular(&map(&[])), "{}");
        assert_eq!(
            tabular(&map(&[("a", Value::Integer(1)), ("b", Value::String("two".into()))])),
            "{a: 1, b: \"two\"}"
        );
    }

    #[test]
    fn containers_nest_to_arbitrary_depth() {
        assert_eq!(
            tabular(&Value::List(vec![map(&[("x", Value::Integer(1))])])),
            "[{x: 1}]"
        );
        assert_eq!(
            tabular(&map(&[("xs", Value::List(vec![Value::Integer(1), Value::Integer(2)]))])),
            "{xs: [1, 2]}"
        );
        // Quotes inside a nested string are escaped.
        assert_eq!(
            tabular(&Value::List(vec![Value::String("say \"hi\"".into())])),
            "[\"say \\\"hi\\\"\"]"
        );
    }

    fn map(pairs: &[(&str, Value)]) -> Value {
        Value::Map(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    fn props(pairs: &[(&str, Value)]) -> std::collections::BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn renders_nodes() {
        assert_eq!(
            tabular(&Value::Node(Node {
                id: 0,
                labels: vec!["Person".into()],
                properties: props(&[("name", Value::String("Ada".into())), ("age", Value::Integer(36))]),
            })),
            "(:Person {age: 36, name: \"Ada\"})"
        );
        assert_eq!(
            tabular(&Value::Node(Node {
                id: 1,
                labels: vec!["A".into(), "B".into()],
                properties: props(&[]),
            })),
            "(:A:B)"
        );
        assert_eq!(
            tabular(&Value::Node(Node {
                id: 2,
                labels: vec![],
                properties: props(&[]),
            })),
            "()"
        );
    }

    #[test]
    fn renders_relationships() {
        assert_eq!(
            tabular(&Value::Relationship(Relationship {
                id: 0,
                start_node_id: 1,
                end_node_id: 2,
                rel_type: "KNOWS".into(),
                properties: props(&[("since", Value::Integer(2020))]),
            })),
            "[:KNOWS {since: 2020}]"
        );
        assert_eq!(
            tabular(&Value::UnboundRelationship(UnboundRelationship {
                id: 0,
                rel_type: "KNOWS".into(),
                properties: props(&[]),
            })),
            "[:KNOWS]"
        );
    }

    #[test]
    fn renders_temporals_matching_memgraph() {
        use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
        use crate::value::Duration as Dur;

        let date = NaiveDate::from_ymd_opt(2021, 6, 15).unwrap();
        assert_eq!(tabular(&Value::Date(date)), "2021-06-15");

        let lt = NaiveTime::from_hms_micro_opt(12, 34, 56, 789_000).unwrap();
        assert_eq!(tabular(&Value::LocalTime(lt)), "12:34:56.789000");

        let ldt = NaiveDateTime::new(date, lt);
        assert_eq!(tabular(&Value::LocalDateTime(ldt)), "2021-06-15T12:34:56.789000");

        assert_eq!(
            tabular(&Value::Duration(Dur { months: 0, days: 1, seconds: 7384, nanos: 0 })),
            "P1DT2H3M4.000000S"
        );
        assert_eq!(
            tabular(&Value::Duration(Dur { months: 0, days: 1, seconds: 7384, nanos: 500_000_000 })),
            "P1DT2H3M4.500000S"
        );

        let off = FixedOffset::east_opt(2 * 3600).unwrap();
        let dto = off.with_ymd_and_hms(2021, 6, 15, 12, 34, 56).unwrap();
        assert_eq!(
            tabular(&Value::DateTimeOffset(dto)),
            "2021-06-15T12:34:56.000000+02:00"
        );

        let dtz = chrono_tz::Europe::Zagreb
            .with_ymd_and_hms(2021, 6, 15, 12, 34, 56)
            .unwrap();
        assert_eq!(
            tabular(&Value::DateTimeZoned(dtz)),
            "2021-06-15T12:34:56.000000+02:00[Europe/Zagreb]"
        );
    }

    #[test]
    fn renders_paths_with_direction() {
        let nodes = vec![
            Node { id: 0, labels: vec!["A".into()], properties: props(&[]) },
            Node { id: 1, labels: vec!["B".into()], properties: props(&[]) },
            Node { id: 2, labels: vec!["C".into()], properties: props(&[]) },
        ];
        let rels = vec![
            UnboundRelationship { id: 10, rel_type: "R1".into(), properties: props(&[]) },
            UnboundRelationship { id: 11, rel_type: "R2".into(), properties: props(&[]) },
        ];
        // Forward single hop.
        assert_eq!(
            tabular(&Value::Path(Path {
                nodes: nodes[..2].to_vec(),
                relationships: rels[..1].to_vec(),
                sequence: vec![1, 1],
            })),
            "(:A)-[:R1]->(:B)"
        );
        // Reverse single hop.
        assert_eq!(
            tabular(&Value::Path(Path {
                nodes: nodes[..2].to_vec(),
                relationships: rels[..1].to_vec(),
                sequence: vec![-1, 1],
            })),
            "(:A)<-[:R1]-(:B)"
        );
        // Two hops.
        assert_eq!(
            tabular(&Value::Path(Path {
                nodes: nodes.clone(),
                relationships: rels.clone(),
                sequence: vec![1, 1, 2, 2],
            })),
            "(:A)-[:R1]->(:B)-[:R2]->(:C)"
        );
        // Single-node path.
        assert_eq!(
            tabular(&Value::Path(Path {
                nodes: nodes[..1].to_vec(),
                relationships: vec![],
                sequence: vec![],
            })),
            "(:A)"
        );
    }

    #[test]
    fn strings_escape_control_whitespace_not_quotes() {
        assert_eq!(
            tabular(&Value::String("a\tb\nc\rd\\e".into())),
            "a\\tb\\nc\\rd\\\\e"
        );
        // Quotes are literal in tabular (quote-escaping is a CSV concern, slice 22).
        assert_eq!(tabular(&Value::String("say \"hi\"".into())), "say \"hi\"");
    }
}
