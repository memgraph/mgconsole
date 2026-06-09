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
