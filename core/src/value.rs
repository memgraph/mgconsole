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

use crate::error::Error;

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

/// Encode a Core [`Value`] as a `bolt_proto::Value` for use as a query
/// parameter. The structural result types — nodes, relationships, paths — and
/// enums are results, not inputs, so they cannot be parameters and are rejected
/// with [`Error::Parameter`] rather than silently mangled.
pub(crate) fn to_bolt(value: &Value) -> Result<bolt_proto::Value, Error> {
    use bolt_proto::value::{Duration as BDuration, Point2D as BPoint2D, Point3D as BPoint3D};
    use bolt_proto::Value as B;

    Ok(match value {
        Value::Null => B::Null,
        Value::Boolean(b) => B::Boolean(*b),
        Value::Integer(i) => B::Integer(*i),
        Value::Float(f) => B::Float(*f),
        Value::Bytes(b) => B::Bytes(b.clone()),
        Value::String(s) => B::String(s.clone()),
        Value::List(items) => B::List(items.iter().map(to_bolt).collect::<Result<Vec<_>, _>>()?),
        Value::Map(m) => B::Map(
            m.iter()
                .map(|(k, v)| Ok((k.clone(), to_bolt(v)?)))
                .collect::<Result<std::collections::HashMap<_, _>, Error>>()?,
        ),
        Value::Date(d) => B::Date(*d),
        Value::Time(t, off) => B::Time(*t, *off),
        Value::LocalTime(t) => B::LocalTime(*t),
        Value::LocalDateTime(dt) => B::LocalDateTime(*dt),
        Value::Duration(d) => B::Duration(BDuration::new(d.months, d.days, d.seconds, d.nanos)),
        Value::DateTimeOffset(dt) => B::DateTimeOffset(*dt),
        Value::DateTimeZoned(dt) => B::DateTimeZoned(*dt),
        Value::Point2d(p) => B::Point2D(BPoint2D::new(p.srid, p.x, p.y)),
        Value::Point3d(p) => B::Point3D(BPoint3D::new(p.srid, p.x, p.y, p.z)),
        Value::Node(_)
        | Value::Relationship(_)
        | Value::UnboundRelationship(_)
        | Value::Path(_)
        | Value::Enum(_) => {
            return Err(Error::Parameter(format!(
                "{} cannot be used as a query parameter",
                unsupported_kind(value)
            )))
        }
    })
}

/// Name of a Value kind that cannot be a query parameter, for the error message.
fn unsupported_kind(value: &Value) -> &'static str {
    match value {
        Value::Node(_) => "a node",
        Value::Relationship(_) | Value::UnboundRelationship(_) => "a relationship",
        Value::Path(_) => "a path",
        Value::Enum(_) => "an enum",
        _ => "this value",
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
    Value::Map(m.into_iter().map(|(k, v)| (k, Value::from(v))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_translates() {
        assert_eq!(
            Value::from(bolt_proto::Value::Integer(42)),
            Value::Integer(42)
        );
        assert_eq!(
            Value::from(bolt_proto::Value::String("hi".into())),
            Value::String("hi".into())
        );
        assert_eq!(Value::from(bolt_proto::Value::Null), Value::Null);
    }

    #[test]
    fn enum_sentinel_map_normalises_to_enum() {
        let mut m = std::collections::HashMap::new();
        m.insert(
            "__type".to_string(),
            bolt_proto::Value::String("mg_enum".into()),
        );
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

    #[test]
    fn scalars_encode_to_bolt() {
        assert_eq!(
            to_bolt(&Value::Integer(7)).unwrap(),
            bolt_proto::Value::Integer(7)
        );
        assert_eq!(to_bolt(&Value::Null).unwrap(), bolt_proto::Value::Null);
        assert_eq!(
            to_bolt(&Value::String("hi".into())).unwrap(),
            bolt_proto::Value::String("hi".into())
        );
    }

    #[test]
    fn nested_list_and_map_encode_to_bolt() {
        let value = Value::List(vec![
            Value::Integer(1),
            Value::Map(BTreeMap::from([("k".to_string(), Value::Boolean(true))])),
        ]);
        match to_bolt(&value).unwrap() {
            bolt_proto::Value::List(items) => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], bolt_proto::Value::Integer(1));
                match &items[1] {
                    bolt_proto::Value::Map(m) => {
                        assert_eq!(m.get("k"), Some(&bolt_proto::Value::Boolean(true)));
                    }
                    other => panic!("expected map, got {other:?}"),
                }
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn temporal_and_point_encode_to_bolt() {
        let dur = Value::Duration(Duration {
            months: 1,
            days: 2,
            seconds: 3,
            nanos: 4,
        });
        assert!(matches!(
            to_bolt(&dur).unwrap(),
            bolt_proto::Value::Duration(_)
        ));

        let point = Value::Point2d(Point2d {
            srid: 4326,
            x: 1.0,
            y: 2.0,
        });
        assert!(matches!(
            to_bolt(&point).unwrap(),
            bolt_proto::Value::Point2D(_)
        ));
    }

    #[test]
    fn structural_values_are_rejected_as_parameters() {
        let node = Value::Node(Node {
            id: 1,
            labels: vec!["L".into()],
            properties: BTreeMap::new(),
        });
        assert!(matches!(to_bolt(&node), Err(Error::Parameter(_))));
        assert!(matches!(
            to_bolt(&Value::Enum("S::A".into())),
            Err(Error::Parameter(_))
        ));
    }
}
