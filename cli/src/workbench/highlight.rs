//! The workbench's rendering of the shared [`HighlightCategory`] (slice 04) as
//! ratatui styles — the second of "one classification, two renderings" (ADR
//! 0008/0010): the REPL maps each category to ANSI, the workbench maps it to a
//! ratatui `Style`. The palette mirrors the REPL's meaning so a misspelt keyword
//! (a plain `Word`) renders unstyled and stands out by contrast, and a keyword
//! or function inside a string/comment is not coloured (inherited from the
//! lexer, via `categorize`). Pure: text in, styled spans out, no terminal.

use mgconsole_core::lex;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::syntax::{categorize, HighlightCategory};
use crate::theme::{Palette, ThemeColor};

/// Map a theme colour onto a ratatui [`Color`] (issue 14). `Default` is the
/// terminal's own foreground (`Color::Reset`), so an unstyled category inherits it.
pub fn to_ratatui(color: ThemeColor) -> Color {
    match color {
        ThemeColor::Default => Color::Reset,
        ThemeColor::Black => Color::Black,
        ThemeColor::Red => Color::Red,
        ThemeColor::Green => Color::Green,
        ThemeColor::Yellow => Color::Yellow,
        ThemeColor::Blue => Color::Blue,
        ThemeColor::Magenta => Color::Magenta,
        ThemeColor::Cyan => Color::Cyan,
        ThemeColor::Gray => Color::Gray,
        ThemeColor::DarkGray => Color::DarkGray,
        ThemeColor::LightRed => Color::LightRed,
        ThemeColor::LightGreen => Color::LightGreen,
        ThemeColor::LightYellow => Color::LightYellow,
        ThemeColor::LightBlue => Color::LightBlue,
        ThemeColor::LightMagenta => Color::LightMagenta,
        ThemeColor::LightCyan => Color::LightCyan,
        ThemeColor::White => Color::White,
    }
}

/// The ratatui style for a highlight category under `palette` (issue 14). `Plain`
/// resolves to the terminal default, so the coloured categories pop and a misspelt
/// keyword stands out unstyled (matching the REPL).
pub fn style_for(palette: &Palette, category: HighlightCategory) -> Style {
    match palette.color(category) {
        ThemeColor::Default => Style::default(),
        color => Style::default().fg(to_ratatui(color)),
    }
}

/// Build a styled ratatui [`Line`] for one line of editor text, colouring each
/// lexer token by its category under `palette`. With `color` off the line is a
/// single unstyled span (a monochrome workbench shows the text, just without
/// colour — it is not disabled).
pub fn highlight_line(line: &str, color: bool, palette: &Palette) -> Line<'static> {
    if !color {
        return Line::from(line.to_string());
    }
    let spans: Vec<Span<'static>> = lex(line)
        .into_iter()
        .map(|token| {
            let text = token.text(line).to_string();
            let category = categorize(&token, &text);
            Span::styled(text, style_for(palette, category))
        })
        .collect();
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin_palette;

    fn default_palette() -> Palette {
        builtin_palette("default").expect("built in")
    }

    #[test]
    fn each_category_maps_to_a_distinct_style() {
        // The colour-bearing categories mirror the REPL palette (the default theme).
        let p = default_palette();
        assert_eq!(style_for(&p, HighlightCategory::Keyword).fg, Some(Color::Yellow));
        assert_eq!(style_for(&p, HighlightCategory::Function).fg, Some(Color::Cyan));
        assert_eq!(style_for(&p, HighlightCategory::String).fg, Some(Color::Green));
        assert_eq!(style_for(&p, HighlightCategory::Number).fg, Some(Color::Magenta));
        assert_eq!(style_for(&p, HighlightCategory::Comment).fg, Some(Color::DarkGray));
        assert_eq!(style_for(&p, HighlightCategory::Parameter).fg, Some(Color::Blue));
        // Plain carries no foreground colour.
        assert_eq!(style_for(&p, HighlightCategory::Plain).fg, None);
    }

    #[test]
    fn a_theme_override_recolours_a_category() {
        // The same seam config `[theme]` drives: a different palette repaints.
        let p = Palette {
            keyword: ThemeColor::Red,
            ..default_palette()
        };
        assert_eq!(style_for(&p, HighlightCategory::Keyword).fg, Some(Color::Red));
    }

    #[test]
    fn a_keyword_and_a_function_are_coloured_by_category() {
        let line = highlight_line("MATCH abs", true, &default_palette());
        let keyword = line.spans.iter().find(|s| s.content == "MATCH").expect("MATCH span");
        assert_eq!(keyword.style.fg, Some(Color::Yellow));
        let function = line.spans.iter().find(|s| s.content == "abs").expect("abs span");
        assert_eq!(function.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn a_keyword_inside_a_string_is_not_coloured_as_a_keyword() {
        // Inherited from the lexer via categorize: 'MATCH' is a String token here.
        let line = highlight_line("RETURN 'MATCH'", true, &default_palette());
        let string = line.spans.iter().find(|s| s.content == "'MATCH'").expect("string span");
        assert_eq!(string.style.fg, Some(Color::Green), "the whole literal is green");
        // No keyword-yellow appears for the in-string MATCH.
        assert!(
            line.spans.iter().all(|s| s.content != "MATCH"),
            "the in-string keyword is not its own coloured span"
        );
    }

    #[test]
    fn colour_off_yields_a_single_plain_span() {
        let line = highlight_line("MATCH (n) RETURN n", false, &default_palette());
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style.fg, None);
    }
}
