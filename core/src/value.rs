//! The Core-owned Value model (ADR 0003).
//!
//! Every Memgraph type the ADR-0001 spike found is a variant here, from the
//! first cut — the type is total so later rendering slices add behaviour, not
//! variants. `bolt_proto::Value` is translated into this type at the Bolt
//! boundary (`From`), and Memgraph quirks are normalised here once: most
//! notably an enum, which Memgraph transmits as a tagged map
//! (`{"__type": "mg_enum", "__value": "Status::Active"}`) and which becomes a
//! first-class [`Value::Enum`] rather than a map passed through.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use chrono_tz::Tz;

/// A single piece of data Memgraph returns, in its Core form.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Bytes(Vec<u8>),
    String(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
    Node(Node),
    Relationship(Relationship),
    UnboundRelationship(UnboundRelationship),
    Path(Path),
    Date(NaiveDate),
    /// A time with a fixed UTC offset (Bolt `Time`).
    Time(NaiveTime, FixedOffset),
    LocalTime(NaiveTime),
    LocalDateTime(NaiveDateTime),
    Duration(Duration),
    /// A zoned datetime carrying a fixed UTC offset.
    DateTimeOffset(DateTime<FixedOffset>),
    /// A zoned datetime carrying a named IANA zone.
    DateTimeZoned(DateTime<Tz>),
    Point2d(Point2d),
    Point3d(Point3d),
    /// A Memgraph enum member, written `Type::Member`.
    Enum(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: i64,
    pub labels: Vec<String>,
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relationship {
    pub id: i64,
    pub start_node_id: i64,
    pub end_node_id: i64,
    pub rel_type: String,
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnboundRelationship {
    pub id: i64,
    pub rel_type: String,
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub nodes: Vec<Node>,
    pub relationships: Vec<UnboundRelationship>,
    pub sequence: Vec<i64>,
}

/// A Memgraph duration, decomposed as Bolt transmits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duration {
    pub months: i64,
    pub days: i64,
    pub seconds: i64,
    pub nanos: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Point2d {
    pub srid: i32,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Point3d {
    pub srid: i32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Sentinel keys Memgraph uses to transmit an enum as a map.
const ENUM_TYPE_KEY: &str = "__type";
const ENUM_VALUE_KEY: &str = "__value";
const ENUM_TYPE_MARKER: &str = "mg_enum";

impl From<bolt_proto::Value> for Value {
    fn from(v: bolt_proto::Value) -> Self {
        use bolt_proto::Value as B;
        match v {
            B::Null => Value::Null,
            B::Boolean(b) => Value::Boolean(b),
            B::Integer(i) => Value::Integer(i),
            B::Float(f) => Value::Float(f),
            B::Bytes(b) => Value::Bytes(b),
            B::String(s) => Value::String(s),
            B::List(items) => Value::List(items.into_iter().map(Value::from).collect()),
            B::Map(m) => from_map(m),
            B::Node(n) => Value::Node(node(&n)),
            B::Relationship(r) => Value::Relationship(Relationship {
                id: r.rel_identity(),
                start_node_id: r.start_node_identity(),
                end_node_id: r.end_node_identity(),
                rel_type: r.rel_type().to_string(),
                properties: props(r.properties()),
            }),
            B::UnboundRelationship(r) => Value::UnboundRelationship(unbound(&r)),
            B::Path(p) => Value::Path(Path {
                nodes: p.nodes().iter().map(node).collect(),
                relationships: p.relationships().iter().map(unbound).collect(),
                sequence: p.sequence().to_vec(),
            }),
            B::Date(d) => Value::Date(d),
            B::Time(t, off) => Value::Time(t, off),
            B::LocalTime(t) => Value::LocalTime(t),
            B::LocalDateTime(dt) => Value::LocalDateTime(dt),
            B::Duration(d) => Value::Duration(Duration {
                months: d.months(),
                days: d.days(),
                seconds: d.seconds(),
                nanos: d.nanos(),
            }),
            B::DateTimeOffset(dt) => Value::DateTimeOffset(dt),
            B::DateTimeZoned(dt) => Value::DateTimeZoned(dt),
            B::Point2D(p) => Value::Point2d(Point2d {
                srid: p.srid(),
                x: p.x(),
                y: p.y(),
            }),
            B::Point3D(p) => Value::Point3d(Point3d {
                srid: p.srid(),
                x: p.x(),
                y: p.y(),
                z: p.z(),
            }),
        }
    }
}

fn node(n: &bolt_proto::value::Node) -> Node {
    Node {
        id: n.node_identity(),
        labels: n.labels().to_vec(),
        properties: props(n.properties()),
    }
}

fn unbound(r: &bolt_proto::value::UnboundRelationship) -> UnboundRelationship {
    UnboundRelationship {
        id: r.rel_identity(),
        rel_type: r.rel_type().to_string(),
        properties: props(r.properties()),
    }
}

fn props(m: &std::collections::HashMap<String, bolt_proto::Value>) -> BTreeMap<String, Value> {
    m.iter()
        .map(|(k, v)| (k.clone(), Value::from(v.clone())))
        .collect()
}

/// Translate a Bolt map, normalising Memgraph's enum sentinel into [`Value::Enum`].
fn from_map(m: std::collections::HashMap<String, bolt_proto::Value>) -> Value {
    if let Some(bolt_proto::Value::String(marker)) = m.get(ENUM_TYPE_KEY) {
        if marker == ENUM_TYPE_MARKER {
            if let Some(bolt_proto::Value::String(qualified)) = m.get(ENUM_VALUE_KEY) {
                return Value::Enum(qualified.clone());
            }
        }
    }
    Value::Map(
        m.into_iter()
            .map(|(k, v)| (k, Value::from(v)))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_translates() {
        assert_eq!(Value::from(bolt_proto::Value::Integer(42)), Value::Integer(42));
        assert_eq!(
            Value::from(bolt_proto::Value::String("hi".into())),
            Value::String("hi".into())
        );
        assert_eq!(Value::from(bolt_proto::Value::Null), Value::Null);
    }

    #[test]
    fn enum_sentinel_map_normalises_to_enum() {
        let mut m = std::collections::HashMap::new();
        m.insert("__type".to_string(), bolt_proto::Value::String("mg_enum".into()));
        m.insert(
            "__value".to_string(),
            bolt_proto::Value::String("Status::Active".into()),
        );
        assert_eq!(
            Value::from(bolt_proto::Value::Map(m)),
            Value::Enum("Status::Active".into())
        );
    }

    #[test]
    fn ordinary_map_stays_a_map() {
        let mut m = std::collections::HashMap::new();
        m.insert("a".to_string(), bolt_proto::Value::Integer(1));
        match Value::from(bolt_proto::Value::Map(m)) {
            Value::Map(bt) => assert_eq!(bt.get("a"), Some(&Value::Integer(1))),
            other => panic!("expected map, got {other:?}"),
        }
    }
}
