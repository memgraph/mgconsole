//! Golden-file tests for the pure tabular rendering seam (slices 03–07).
//! No database, no Session.

mod golden;

use mgconsole_core::Value;

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
