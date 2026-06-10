//! The buffer-all tabular renderer (ADR 0002).
//!
//! Arranges rendered Values into an aligned table with a header, sizing columns
//! to their content. With `fit_width` set, the table is fitted to a terminal
//! width (cells wrap). The row-cap behaviour lives on
//! [`crate::RecordStream::collect_capped`]; past the cap the caller shows
//! [`row_cap_warning`] and renders only the capped rows, so memory stays
//! bounded.

use comfy_table::{presets, ContentArrangement, Table};

use crate::render;
use crate::value::Value;

/// Default number of rows the tabular path will buffer before warning.
pub const DEFAULT_ROW_CAP: usize = 1000;

/// Options for the tabular renderer.
#[derive(Debug, Clone, Default)]
pub struct TableOptions {
    /// Fit the table to this terminal width (cells wrap); `None` = natural width.
    pub fit_width: Option<u16>,
}

/// Render a header and buffered rows as an aligned ASCII table.
pub fn render_table(header: &[String], rows: &[Vec<Value>], opts: &TableOptions) -> String {
    let mut table = Table::new();
    table.load_preset(presets::ASCII_FULL);
    table.force_no_tty();
    table.set_header(header.to_vec());

    match opts.fit_width {
        Some(width) => {
            table
                .set_content_arrangement(ContentArrangement::Dynamic)
                .set_width(width);
        }
        None => {
            table.set_content_arrangement(ContentArrangement::Disabled);
        }
    }

    for row in rows {
        let cells: Vec<String> = row.iter().map(render::tabular).collect();
        table.add_row(cells);
    }

    table.trim_fmt()
}

/// The warning shown when a tabular result exceeds the row cap.
pub fn row_cap_warning(cap: usize) -> String {
    format!(
        "warning: result exceeds the {cap}-row tabular cap; showing the first \
         {cap} rows. Use a streaming output format (csv, jsonl, cypherl) to see \
         every row."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_names_the_cap_and_points_to_streaming() {
        let w = row_cap_warning(1000);
        assert!(w.contains("1000-row"));
        assert!(w.contains("csv"));
    }

    #[test]
    fn table_has_header_and_aligned_columns() {
        let header = vec!["n".to_string(), "name".to_string()];
        let rows = vec![
            vec![Value::Integer(1), Value::String("Ada".into())],
            vec![Value::Integer(42), Value::String("Bob".into())],
        ];
        let out = render_table(&header, &rows, &TableOptions::default());
        // Header present, both names present, aligned (a separator line of dashes).
        assert!(out.contains("| n"));
        assert!(out.contains("name"));
        assert!(out.contains("Ada"));
        assert!(out.lines().count() >= 5); // top border, header, sep, 2 rows, bottom
    }
}
