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
pub fn tabular(value: &Value) -> String {
    match value {
        Value::Null => "Null".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => float(*f),
        Value::String(s) => escape_ws(s),
        Value::Bytes(b) => format!("{b:?}"),
        Value::Enum(q) => q.clone(),
        // Provisional renderings — refined by slices 04–07.
        Value::List(items) => {
            let inner: Vec<String> = items.iter().map(tabular).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Map(m) => {
            let inner: Vec<String> = m.iter().map(|(k, v)| format!("{k}: {}", tabular(v))).collect();
            format!("{{{}}}", inner.join(", "))
        }
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
    fn strings_escape_control_whitespace_not_quotes() {
        assert_eq!(
            tabular(&Value::String("a\tb\nc\rd\\e".into())),
            "a\\tb\\nc\\rd\\\\e"
        );
        // Quotes are literal in tabular (quote-escaping is a CSV concern, slice 22).
        assert_eq!(tabular(&Value::String("say \"hi\"".into())), "say \"hi\"");
    }
}
