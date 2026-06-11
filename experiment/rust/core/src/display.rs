//! Result display mode (issue 01): the Core's choice between the tabular layout
//! and a **vertical** layout that shows one Record as `column: value` per line.
//!
//! [`DisplayMode`] is the `display` Setting's value — `tabular`, `vertical`, or
//! the default `auto`, which renders tabular until a row will not fit the
//! terminal width, then falls back to vertical. The fit decision reuses the
//! tabular renderer's natural width ([`crate::tabular::natural_width`]) rather
//! than a parallel measurement path, and is isolated in the pure
//! [`resolve_layout`] so it is unit-tested with no rendering. [`render_records`]
//! is the one entry point a Frontend calls; it dispatches to the chosen layout.

use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

use crate::format::Header;
use crate::tabular::{self, render_table, TableOptions};
use crate::value::Value;

/// How a buffered Query result is laid out. The value of the `display` Setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DisplayMode {
    /// An aligned table with a header row (the classic layout).
    Tabular,
    /// One Record per block, `column: value` per line — readable when a row is
    /// too wide for a table.
    Vertical,
    /// Tabular until a row exceeds the terminal width, then vertical (default).
    #[default]
    Auto,
}

impl DisplayMode {
    /// Every mode, in listing order (for `:set` help and the no-arg listing).
    pub const ALL: [DisplayMode; 3] = [Self::Tabular, Self::Vertical, Self::Auto];

    /// The canonical lowercase spelling, as accepted by [`FromStr`] and shown by
    /// `:set`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tabular => "tabular",
            Self::Vertical => "vertical",
            Self::Auto => "auto",
        }
    }
}

impl fmt::Display for DisplayMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DisplayMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "tabular" => Ok(Self::Tabular),
            "vertical" => Ok(Self::Vertical),
            "auto" => Ok(Self::Auto),
            other => Err(format!(
                "invalid display mode '{other}' (expected tabular, vertical, or auto)"
            )),
        }
    }
}

/// The concrete layout chosen for one result — `auto` has been resolved away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Tabular,
    Vertical,
}

/// Decide the layout for a result. `tabular`/`vertical` are honoured verbatim;
/// `auto` falls back to vertical only when the table's `natural_width` exceeds a
/// known `term_width`, and stays tabular when the width is unknown (so a piped or
/// width-less context never silently flips).
pub fn resolve_layout(mode: DisplayMode, natural_width: u16, term_width: Option<u16>) -> Layout {
    match mode {
        DisplayMode::Tabular => Layout::Tabular,
        DisplayMode::Vertical => Layout::Vertical,
        DisplayMode::Auto => match term_width {
            Some(width) if natural_width > width => Layout::Vertical,
            _ => Layout::Tabular,
        },
    }
}

/// Everything the renderer needs to lay a buffered result out: the chosen
/// [`DisplayMode`], the tabular options (column fitting), and the terminal width
/// the `auto` decision consults.
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub mode: DisplayMode,
    pub table: TableOptions,
    /// The terminal width, for the `auto` fit decision. `None` keeps `auto`
    /// tabular (no width to compare against).
    pub term_width: Option<u16>,
}

/// Render a header and buffered rows vertically: a `-[ RECORD n ]-` marker, then
/// one `column: value` line per column, blocks separated by a blank line. Each
/// value uses the shared single-line [`crate::render::tabular`] rendering, so a
/// cell reads the same as it would in a table.
pub fn render_vertical(header: &Header, rows: &[Vec<Value>]) -> String {
    let names = header.names();
    let mut out = String::new();
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        writeln!(out, "-[ RECORD {} ]-", i + 1).unwrap();
        for (name, value) in names.iter().zip(row) {
            writeln!(out, "{name}: {}", crate::render::tabular(value)).unwrap();
        }
    }
    out.trim_end_matches('\n').to_string()
}

/// Render a buffered result in the layout the options select. The single entry
/// point a Frontend calls; `auto` measures the table's natural width here and
/// resolves to a concrete [`Layout`] before rendering.
pub fn render_records(header: &Header, rows: &[Vec<Value>], opts: &RenderOptions) -> String {
    let natural = if opts.mode == DisplayMode::Auto {
        tabular::natural_width(header, rows)
    } else {
        0
    };
    match resolve_layout(opts.mode, natural, opts.term_width) {
        Layout::Tabular => render_table(header, rows, &opts.table),
        Layout::Vertical => render_vertical(header, rows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(cols: &[&str]) -> Header {
        Header::new(cols.iter().map(ToString::to_string).collect::<Vec<_>>())
    }

    #[test]
    fn display_mode_round_trips_through_its_spelling() {
        for mode in DisplayMode::ALL {
            assert_eq!(mode.as_str().parse::<DisplayMode>(), Ok(mode));
        }
        assert_eq!(DisplayMode::default(), DisplayMode::Auto);
    }

    #[test]
    fn an_unknown_display_mode_is_rejected_with_the_valid_set() {
        let err = "grid".parse::<DisplayMode>().expect_err("rejected");
        assert!(err.contains("tabular") && err.contains("vertical") && err.contains("auto"));
    }

    #[test]
    fn explicit_modes_are_honoured_regardless_of_width() {
        // tabular/vertical never consult the width.
        assert_eq!(resolve_layout(DisplayMode::Tabular, 9999, Some(1)), Layout::Tabular);
        assert_eq!(resolve_layout(DisplayMode::Vertical, 1, Some(9999)), Layout::Vertical);
    }

    #[test]
    fn auto_flips_to_vertical_only_when_a_row_exceeds_the_width() {
        // Fits: stays tabular. Exceeds: vertical. Exactly equal: still fits.
        assert_eq!(resolve_layout(DisplayMode::Auto, 40, Some(80)), Layout::Tabular);
        assert_eq!(resolve_layout(DisplayMode::Auto, 81, Some(80)), Layout::Vertical);
        assert_eq!(resolve_layout(DisplayMode::Auto, 80, Some(80)), Layout::Tabular);
    }

    #[test]
    fn auto_stays_tabular_when_the_width_is_unknown() {
        assert_eq!(resolve_layout(DisplayMode::Auto, 9999, None), Layout::Tabular);
    }

    #[test]
    fn vertical_shows_one_column_value_per_line_with_record_markers() {
        let h = header(&["n", "name"]);
        let rows = vec![
            vec![Value::Integer(1), Value::String("Ada".into())],
            vec![Value::Integer(2), Value::String("Bo".into())],
        ];
        assert_eq!(
            render_vertical(&h, &rows),
            "-[ RECORD 1 ]-\nn: 1\nname: Ada\n\n-[ RECORD 2 ]-\nn: 2\nname: Bo"
        );
    }

    #[test]
    fn render_records_dispatches_on_the_mode() {
        let h = header(&["n"]);
        let rows = vec![vec![Value::Integer(1)]];
        let vertical = render_records(
            &h,
            &rows,
            &RenderOptions {
                mode: DisplayMode::Vertical,
                ..RenderOptions::default()
            },
        );
        assert!(vertical.contains("-[ RECORD 1 ]-"));
        let tabular = render_records(
            &h,
            &rows,
            &RenderOptions {
                mode: DisplayMode::Tabular,
                ..RenderOptions::default()
            },
        );
        assert!(tabular.contains("| n"));
    }

    #[test]
    fn auto_picks_vertical_for_a_too_wide_row() {
        // One very wide cell with a tight terminal width: auto goes vertical.
        let h = header(&["data"]);
        let rows = vec![vec![Value::String("x".repeat(200))]];
        let out = render_records(
            &h,
            &rows,
            &RenderOptions {
                mode: DisplayMode::Auto,
                term_width: Some(40),
                ..RenderOptions::default()
            },
        );
        assert!(out.contains("-[ RECORD 1 ]-"), "auto fell back to vertical: {out}");
    }
}
