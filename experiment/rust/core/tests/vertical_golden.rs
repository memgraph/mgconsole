//! Golden tests for the vertical display layout (issue 01). Pure, no Session —
//! the analogue of `tabular_golden.rs` for the `column: value` layout.

mod golden;

use mgconsole_core::{render_vertical, Header, Value};

fn header(cols: &[&str]) -> Header {
    Header::new(
        cols.iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>(),
    )
}

#[test]
fn basic_vertical_layout() {
    let h = header(&["n", "name", "active"]);
    let rows = vec![
        vec![
            Value::Integer(1),
            Value::String("Ada".into()),
            Value::Boolean(true),
        ],
        vec![
            Value::Integer(42),
            Value::String("Grace".into()),
            Value::Boolean(false),
        ],
    ];
    let out = render_vertical(&h, &rows);
    golden::check_block("vertical", "basic", &out);
}

#[test]
fn wide_values_stay_on_one_line_each() {
    // A row too wide for a table is exactly what vertical exists for: each value
    // gets its own line, unwrapped.
    let h = header(&["id", "data"]);
    let rows = vec![vec![
        Value::Integer(1),
        Value::String("a reasonably long string value that would widen a column".into()),
    ]];
    let out = render_vertical(&h, &rows);
    golden::check_block("vertical", "wide_values", &out);
}
