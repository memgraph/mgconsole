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
        // Provisional renderings — refined by slices 05–07.
        Value::Node(n) => format!("{:?}", n),
        Value::Relationship(r) => format!("{:?}", r),
        Value::UnboundRelationship(r) => format!("{:?}", r),
        Value::Path(p) => format!("{:?}", p),
        Value::Date(d) => d.to_string(),
        Value::Time(t, off) => format!("{t}{off}"),
        Value::LocalTime(t) => t.to_string(),
        Value::LocalDateTime(dt) => dt.to_string(),
        Value::Duration(d) => format!("{:?}", d),
        Value::DateTimeOffset(dt) => dt.to_string(),
        Value::DateTimeZoned(dt) => dt.to_string(),
        Value::Point2d(p) => format!("{:?}", p),
        Value::Point3d(p) => format!("{:?}", p),
    }
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
