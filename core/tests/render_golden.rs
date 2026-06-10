//! Golden-file tests for the pure tabular rendering seam (slices 03–07).
//! No database, no Session.

mod golden;

use mgconsole_core::value::{Node, Path, Relationship, UnboundRelationship};
use mgconsole_core::Value;

fn props(pairs: &[(&str, Value)]) -> std::collections::BTreeMap<String, Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
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
    Value::Map(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

#[test]
fn container_tabular_goldens() {
    let cases: Vec<(&str, Value)> = vec![
        ("list_empty", Value::List(vec![])),
        (
            "list_ints",
            Value::List(vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)]),
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
            map(&[("age", Value::Integer(36)), ("name", Value::String("Ada".into()))]),
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
fn graph_tabular_goldens() {
    let node_a = Node { id: 0, labels: vec!["A".into()], properties: props(&[]) };
    let node_b = Node { id: 1, labels: vec!["B".into()], properties: props(&[]) };
    let node_c = Node { id: 2, labels: vec!["C".into()], properties: props(&[]) };
    let r1 = UnboundRelationship { id: 10, rel_type: "R1".into(), properties: props(&[]) };
    let r2 = UnboundRelationship { id: 11, rel_type: "R2".into(), properties: props(&[]) };

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
            Value::Node(Node { id: 1, labels: vec!["A".into(), "B".into()], properties: props(&[]) }),
        ),
        ("node_bare", Value::Node(Node { id: 2, labels: vec![], properties: props(&[]) })),
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
            Value::Path(Path { nodes: vec![node_a], relationships: vec![], sequence: vec![] }),
        ),
    ];
    golden::check_tabular("graph", &cases);
}
