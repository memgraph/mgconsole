//! Golden-file tests for the pure tabular rendering seam (slices 03–07).
//! No database, no Session.

mod golden;

use mgconsole_core::value::{Node, Path, Relationship, UnboundRelationship};
use mgconsole_core::Value;

fn props(pairs: &[(&str, Value)]) -> std::collections::BTreeMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn scalar_tabular_goldens() {
    let cases: Vec<(&str, Value)> = vec![
        ("null", Value::Null),
        ("bool_true", Value::Boolean(true)),
        ("bool_false", Value::Boolean(false)),
        ("integer", Value::Integer(42)),
        ("integer_negative", Value::Integer(-7)),
        ("float_fraction", Value::Float(123.456)),
        ("float_whole", Value::Float(3.0)),
        ("float_negative", Value::Float(-0.5)),
        ("string_plain", Value::String("hello".into())),
        ("string_unicode", Value::String("héllo ☃".into())),
        ("string_quotes", Value::String("say \"hi\"".into())),
        ("string_whitespace", Value::String("a\tb\nc\rd".into())),
        ("string_backslash", Value::String("c:\\tmp".into())),
        ("string_empty", Value::String(String::new())),
    ];
    golden::check_tabular("scalars", &cases);
}

fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

#[test]
fn container_tabular_goldens() {
    let cases: Vec<(&str, Value)> = vec![
        ("list_empty", Value::List(vec![])),
        (
            "list_ints",
            Value::List(vec![
                Value::Integer(1),
                Value::Integer(2),
                Value::Integer(3),
            ]),
        ),
        (
            "list_strings",
            Value::List(vec![Value::String("a".into()), Value::String("b".into())]),
        ),
        (
            "list_mixed",
            Value::List(vec![
                Value::Integer(1),
                Value::String("two".into()),
                Value::Float(3.0),
                Value::Null,
            ]),
        ),
        ("map_empty", map(&[])),
        (
            "map_basic",
            map(&[
                ("age", Value::Integer(36)),
                ("name", Value::String("Ada".into())),
            ]),
        ),
        (
            "list_of_maps",
            Value::List(vec![
                map(&[("x", Value::Integer(1))]),
                map(&[("x", Value::Integer(2))]),
            ]),
        ),
        (
            "map_of_lists",
            map(&[(
                "xs",
                Value::List(vec![Value::Integer(1), Value::Integer(2)]),
            )]),
        ),
        (
            "deep_nesting",
            Value::List(vec![map(&[(
                "inner",
                Value::List(vec![map(&[("k", Value::String("v".into()))])]),
            )])]),
        ),
        (
            "string_with_quote_nested",
            Value::List(vec![Value::String("say \"hi\"".into())]),
        ),
    ];
    golden::check_tabular("containers", &cases);
}

#[test]
// A flat table of graph-shape golden cases; its length is the case count, not
// hidden complexity, so splitting it to satisfy `too_many_lines` adds no clarity.
#[allow(clippy::too_many_lines)]
fn graph_tabular_goldens() {
    let node_a = Node {
        id: 0,
        labels: vec!["A".into()],
        properties: props(&[]),
    };
    let node_b = Node {
        id: 1,
        labels: vec!["B".into()],
        properties: props(&[]),
    };
    let node_c = Node {
        id: 2,
        labels: vec!["C".into()],
        properties: props(&[]),
    };
    let r1 = UnboundRelationship {
        id: 10,
        rel_type: "R1".into(),
        properties: props(&[]),
    };
    let r2 = UnboundRelationship {
        id: 11,
        rel_type: "R2".into(),
        properties: props(&[]),
    };

    let cases: Vec<(&str, Value)> = vec![
        (
            "node_labelled_with_props",
            Value::Node(Node {
                id: 0,
                labels: vec!["Person".into()],
                properties: props(&[
                    ("name", Value::String("Ada".into())),
                    ("age", Value::Integer(36)),
                    ("active", Value::Boolean(true)),
                ]),
            }),
        ),
        (
            "node_multi_label",
            Value::Node(Node {
                id: 1,
                labels: vec!["A".into(), "B".into()],
                properties: props(&[]),
            }),
        ),
        (
            "node_bare",
            Value::Node(Node {
                id: 2,
                labels: vec![],
                properties: props(&[]),
            }),
        ),
        (
            "relationship",
            Value::Relationship(Relationship {
                id: 0,
                start_node_id: 1,
                end_node_id: 2,
                rel_type: "KNOWS".into(),
                properties: props(&[("since", Value::Integer(2020))]),
            }),
        ),
        (
            "unbound_relationship",
            Value::UnboundRelationship(UnboundRelationship {
                id: 0,
                rel_type: "KNOWS".into(),
                properties: props(&[]),
            }),
        ),
        (
            "path_forward",
            Value::Path(Path {
                nodes: vec![node_a.clone(), node_b.clone()],
                relationships: vec![r1.clone()],
                sequence: vec![1, 1],
            }),
        ),
        (
            "path_reverse",
            Value::Path(Path {
                nodes: vec![node_a.clone(), node_b.clone()],
                relationships: vec![r1.clone()],
                sequence: vec![-1, 1],
            }),
        ),
        (
            "path_two_hops",
            Value::Path(Path {
                nodes: vec![node_a.clone(), node_b, node_c],
                relationships: vec![r1, r2],
                sequence: vec![1, 1, 2, 2],
            }),
        ),
        (
            "path_single_node",
            Value::Path(Path {
                nodes: vec![node_a],
                relationships: vec![],
                sequence: vec![],
            }),
        ),
    ];
    golden::check_tabular("graph", &cases);
}

#[test]
fn temporal_tabular_goldens() {
    use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
    use mgconsole_core::value::Duration;

    let date = NaiveDate::from_ymd_opt(2021, 6, 15).unwrap();
    let time = NaiveTime::from_hms_micro_opt(12, 34, 56, 789_000).unwrap();
    let off = FixedOffset::east_opt(2 * 3600).unwrap();

    let cases: Vec<(&str, Value)> = vec![
        ("date", Value::Date(date)),
        ("local_time", Value::LocalTime(time)),
        (
            "local_time_whole",
            Value::LocalTime(NaiveTime::from_hms_opt(9, 0, 0).unwrap()),
        ),
        (
            "local_datetime",
            Value::LocalDateTime(NaiveDateTime::new(date, time)),
        ),
        (
            "duration",
            Value::Duration(Duration {
                months: 0,
                days: 1,
                seconds: 7384,
                nanos: 0,
            }),
        ),
        (
            "duration_fractional",
            Value::Duration(Duration {
                months: 0,
                days: 1,
                seconds: 7384,
                nanos: 500_000_000,
            }),
        ),
        (
            "datetime_offset",
            Value::DateTimeOffset(off.with_ymd_and_hms(2021, 6, 15, 12, 34, 56).unwrap()),
        ),
        (
            "datetime_zoned",
            Value::DateTimeZoned(
                chrono_tz::Europe::Zagreb
                    .with_ymd_and_hms(2021, 6, 15, 12, 34, 56)
                    .unwrap(),
            ),
        ),
        ("time_with_offset", Value::Time(time, off)),
    ];
    golden::check_tabular("temporal", &cases);
}

#[test]
fn point_and_enum_tabular_goldens() {
    use mgconsole_core::value::{Point2d, Point3d};

    let cases: Vec<(&str, Value)> = vec![
        (
            "point_2d_cartesian",
            Value::Point2d(Point2d {
                srid: 7203,
                x: 1.0,
                y: 2.0,
            }),
        ),
        (
            "point_2d_wgs84",
            Value::Point2d(Point2d {
                srid: 4326,
                x: 1.0,
                y: 2.0,
            }),
        ),
        (
            "point_3d_cartesian",
            Value::Point3d(Point3d {
                srid: 9157,
                x: 1.0,
                y: 2.0,
                z: 3.0,
            }),
        ),
        (
            "point_3d_wgs84",
            Value::Point3d(Point3d {
                srid: 4979,
                x: 1.0,
                y: 2.0,
                z: 3.0,
            }),
        ),
        (
            "point_2d_fractional",
            Value::Point2d(Point2d {
                srid: 4326,
                x: 1.5,
                y: 2.25,
            }),
        ),
        ("enum", Value::Enum("Status::Active".into())),
    ];
    golden::check_tabular("point_enum", &cases);
}
