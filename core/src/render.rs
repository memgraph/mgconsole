//! Tabular rendering of a Core [`Value`].
//!
//! Slice 02 renders scalars faithfully; composite and Memgraph-specific types
//! get a provisional rendering that the dedicated rendering slices (03–07)
//! replace with golden-tested output. The match is exhaustive with no wildcard
//! arm (ADR 0003), so each later slice is handed a compile error at the arm it
//! must refine.

use crate::value::Value;

/// Render a single Value as plain tabular text.
pub fn tabular(value: &Value) -> String {
    match value {
        Value::Null => "Null".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
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
}
