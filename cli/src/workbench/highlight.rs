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

/// The ratatui style for a highlight category. `Plain` is left unstyled so the
/// coloured categories pop, matching the REPL (where `Plain` is terminal-default).
pub fn style_for(category: HighlightCategory) -> Style {
    let style = Style::default();
    match category {
        HighlightCategory::Keyword => style.fg(Color::Yellow),
        HighlightCategory::Function => style.fg(Color::Cyan),
        HighlightCategory::String => style.fg(Color::Green),
        HighlightCategory::Number => style.fg(Color::Magenta),
        HighlightCategory::Comment => style.fg(Color::DarkGray),
        HighlightCategory::Parameter => style.fg(Color::Blue),
        HighlightCategory::Plain => style,
    }
}

/// Build a styled ratatui [`Line`] for one line of editor text, colouring each
/// lexer token by its category. With `color` off the line is a single unstyled
/// span (a monochrome workbench shows the text, just without colour — it is not
/// disabled).
pub fn highlight_line(line: &str, color: bool) -> Line<'static> {
    if !color {
        return Line::from(line.to_string());
    }
    let spans: Vec<Span<'static>> = lex(line)
        .into_iter()
        .map(|token| {
            let text = token.text(line).to_string();
            let category = categorize(&token, &text);
            Span::styled(text, style_for(category))
        })
        .collect();
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_category_maps_to_a_distinct_style() {
        // The colour-bearing categories mirror the REPL palette.
        assert_eq!(style_for(HighlightCategory::Keyword).fg, Some(Color::Yellow));
        assert_eq!(style_for(HighlightCategory::Function).fg, Some(Color::Cyan));
        assert_eq!(style_for(HighlightCategory::String).fg, Some(Color::Green));
        assert_eq!(style_for(HighlightCategory::Number).fg, Some(Color::Magenta));
        assert_eq!(style_for(HighlightCategory::Comment).fg, Some(Color::DarkGray));
        assert_eq!(style_for(HighlightCategory::Parameter).fg, Some(Color::Blue));
        // Plain carries no foreground colour.
        assert_eq!(style_for(HighlightCategory::Plain).fg, None);
    }

    #[test]
    fn a_keyword_and_a_function_are_coloured_by_category() {
        let line = highlight_line("MATCH abs", true);
        let keyword = line.spans.iter().find(|s| s.content == "MATCH").expect("MATCH span");
        assert_eq!(keyword.style.fg, Some(Color::Yellow));
        let function = line.spans.iter().find(|s| s.content == "abs").expect("abs span");
        assert_eq!(function.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn a_keyword_inside_a_string_is_not_coloured_as_a_keyword() {
        // Inherited from the lexer via categorize: 'MATCH' is a String token here.
        let line = highlight_line("RETURN 'MATCH'", true);
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
        let line = highlight_line("MATCH (n) RETURN n", false);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].style.fg, None);
    }
}
