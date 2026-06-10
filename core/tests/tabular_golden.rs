//! Golden tests for the buffer-all tabular layout (slice 08). Pure, no Session.

mod golden;

use mgconsole_core::{render_table, TableOptions, Value};

fn header(cols: &[&str]) -> Vec<String> {
    cols.iter().map(std::string::ToString::to_string).collect()
}

#[test]
fn basic_table_sizing_and_alignment() {
    let h = header(&["n", "name", "active"]);
    let rows = vec![
        vec![Value::Integer(1), Value::String("Ada".into()), Value::Boolean(true)],
        vec![Value::Integer(42), Value::String("Grace".into()), Value::Boolean(false)],
        vec![Value::Integer(1000), Value::String("Bo".into()), Value::Null],
    ];
    let out = render_table(&h, &rows, &TableOptions::default());
    golden::check_block("table", "basic", &out);
}

#[test]
fn wide_values_widen_columns() {
    let h = header(&["id", "data"]);
    let rows = vec![
        vec![
            Value::Integer(1),
            Value::List(vec![Value::Integer(1), Value::String("two".into()), Value::Float(3.0)]),
        ],
        vec![
            Value::Integer(2),
            Value::String("a reasonably long string value".into()),
        ],
    ];
    let out = render_table(&h, &rows, &TableOptions::default());
    golden::check_block("table", "wide_values", &out);
}

#[test]
fn fit_to_screen_wraps_to_width() {
    let h = header(&["id", "data"]);
    let rows = vec![vec![
        Value::Integer(1),
        Value::String("this is a long value that should wrap when fitted".into()),
    ]];
    let out = render_table(&h, &rows, &TableOptions { fit_width: Some(28) });
    golden::check_block("table", "fit_narrow", &out);
}
