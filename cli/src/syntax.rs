//! Cypher syntax support for the REPL: static completion and highlighting
//! (slice 18), carried over from `mgconsole`'s replxx hooks.
//!
//! The logic lives here as pure functions over `&str` so it is tested without a
//! terminal; `main`'s rustyline `Helper` is a thin adapter that calls in. The
//! completer is built from a list of [`CompletionSource`]s — today just the
//! [`StaticVocabulary`] of keyword/function tables — so a live schema-aware
//! source (labels, properties) becomes one more entry in that list, with no
//! change to the cursor handling or the rustyline adapter.

use mgconsole_core::{lex, Token, TokenKind};

use crate::keywords::{AWESOME_FUNCTIONS, CYPHER_KEYWORDS, MEMGRAPH_KEYWORDS};

/// Characters that separate one word from the next, matching `mgconsole`'s
/// replxx word-boundary set. The completion prefix and each highlighted word run
/// between these.
const WORD_BOUNDARIES: &[char] = &[
    ' ', '\t', '\n', '\r', '\x0b', '\x0c', '-', '=', '+', '*', '&', '^', '%', '$', '#', '@', '!',
    ',', '/', '?', '<', '>', ';', ':', '`', '~', '\'', '"', '[', ']', '{', '}', '(', ')', '\\',
    '|',
];

/// A provider of completion candidates for a typed prefix. The static tables are
/// one source; a future schema-aware source (labels, relationship types,
/// property keys) is another, consulted alongside it.
pub trait CompletionSource: Send + Sync {
    /// Append candidates whose start matches `prefix_upper` (already
    /// upper-cased) to `out`, preserving the source's own order.
    fn extend_matches(&self, prefix_upper: &str, out: &mut Vec<String>);
}

/// The built-in keyword/function tables: Cypher keywords, Memgraph keywords, and
/// the "awesome function" names, consulted in that order (as `mgconsole` does).
pub struct StaticVocabulary;

impl CompletionSource for StaticVocabulary {
    fn extend_matches(&self, prefix_upper: &str, out: &mut Vec<String>) {
        for table in [CYPHER_KEYWORDS, MEMGRAPH_KEYWORDS, AWESOME_FUNCTIONS] {
            for &word in table {
                if word.starts_with(prefix_upper) {
                    out.push(word.to_string());
                }
            }
        }
    }
}

/// Aggregates one or more [`CompletionSource`]s into the REPL's completer. New
/// sources (e.g. live schema) are added with [`Completer::add_source`] without
/// touching callers.
pub struct Completer {
    sources: Vec<Box<dyn CompletionSource>>,
}

impl Completer {
    /// The default completer: just the static keyword/function vocabulary.
    pub fn with_static_vocabulary() -> Self {
        Self {
            sources: vec![Box::new(StaticVocabulary)],
        }
    }

    /// Register an additional candidate source (e.g. a live schema source).
    pub fn add_source(&mut self, source: Box<dyn CompletionSource>) {
        self.sources.push(source);
    }

    /// Candidates that complete `prefix`, matched case-insensitively across all
    /// sources, de-duplicated while preserving first-seen order. An empty prefix
    /// offers nothing (avoids dumping the entire vocabulary).
    pub fn candidates(&self, prefix: &str) -> Vec<String> {
        if prefix.is_empty() {
            return Vec::new();
        }
        let prefix_upper = prefix.to_uppercase();
        let mut matches = Vec::new();
        for source in &self.sources {
            source.extend_matches(&prefix_upper, &mut matches);
        }
        let mut seen = std::collections::HashSet::new();
        matches.retain(|word| seen.insert(word.clone()));
        matches
    }
}

/// The byte index where the word under the cursor begins: scan back from `pos`
/// over non-boundary characters. The text from here to `pos` is the completion
/// prefix, and what a chosen candidate replaces.
pub fn word_start(line: &str, pos: usize) -> usize {
    line[..pos].rfind(WORD_BOUNDARIES).map_or(0, |boundary| {
        boundary + line[boundary..].chars().next().map_or(1, char::len_utf8)
    })
}

/// The Frontend-neutral lexical category of a token — the single classification
/// both colour-owning Frontends share (ADR 0008, extended by ADR 0010). The
/// Core lexer yields lexical *structure* (tokens); this maps that structure to
/// the seven colour categories, and each Frontend renders a category its own
/// way: the REPL to ANSI ([`highlight`]), the workbench to a ratatui `Style`
/// (slice 05). One classification, two renderings — no terminal or colour leaks
/// into the classification itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightCategory {
    /// A Cypher or Memgraph keyword.
    Keyword,
    /// An awesome-function name.
    Function,
    /// A string literal.
    String,
    /// A numeric literal.
    Number,
    /// A line or block comment.
    Comment,
    /// A `$name` parameter reference (the `:param` family).
    Parameter,
    /// Everything else: identifiers, punctuation, whitespace — left
    /// terminal-default so the coloured categories pop.
    Plain,
}

/// Classify one lexer token into its Frontend-neutral [`HighlightCategory`].
/// A Word token is looked up in the keyword/function tables (keywords winning
/// over functions when a name is in both, matching `mgconsole`'s precedence); a
/// keyword or function name inside a String or Comment is *that* token, not a
/// Word, so it classifies as String/Comment — the false-positive fix. Pure:
/// token in, category out, no ANSI and no terminal.
pub fn categorize(token: &Token, text: &str) -> HighlightCategory {
    match token.kind {
        TokenKind::Word => classify_word(text),
        TokenKind::String => HighlightCategory::String,
        TokenKind::Number => HighlightCategory::Number,
        TokenKind::Comment => HighlightCategory::Comment,
        TokenKind::Parameter => HighlightCategory::Parameter,
        TokenKind::Punct | TokenKind::Whitespace => HighlightCategory::Plain,
    }
}

/// Classify a single word (case-insensitively) against the keyword/function
/// tables. Keywords win over functions when a name appears in both.
fn classify_word(word: &str) -> HighlightCategory {
    let upper = word.to_uppercase();
    if CYPHER_KEYWORDS.contains(&upper.as_str()) || MEMGRAPH_KEYWORDS.contains(&upper.as_str()) {
        HighlightCategory::Keyword
    } else if AWESOME_FUNCTIONS.contains(&upper.as_str()) {
        HighlightCategory::Function
    } else {
        HighlightCategory::Plain
    }
}

const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const MAGENTA: &str = "\x1b[35m";
const GREY: &str = "\x1b[90m"; // bright-black
const BLUE: &str = "\x1b[34m";
const RESET: &str = "\x1b[0m";

/// Re-emit `line` with Cypher syntax coloured for the interactive REPL, driven
/// by the Core lexer's token stream (ADR 0008: the Core yields lexical
/// structure; the Frontend maps it to colour). Keywords colour yellow and
/// functions cyan, classified case-insensitively against the keyword/function
/// tables; a keyword or function name appearing inside a string or comment is a
/// String/Comment token — not a Word — so it is left uncoloured (the
/// false-positive fix). Every other token passes through untouched; later
/// slices widen the palette. Pure: `&str` in, text out, no terminal.
pub fn highlight(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for token in lex(line) {
        let text = token.text(line);
        match ansi_for(categorize(&token, text)) {
            Some(colour) => {
                out.push_str(colour);
                out.push_str(text);
                out.push_str(RESET);
            }
            None => out.push_str(text),
        }
    }
    out
}

/// The REPL's rendering of a [`HighlightCategory`]: the ANSI colour it is
/// painted, or `None` to leave it terminal-default. This is the REPL Frontend's
/// half of "one classification, two renderings" (ADR 0008/0010) — the workbench
/// maps the same categories to ratatui styles instead (slice 05). `Plain` stays
/// terminal-default so the coloured categories pop and a typo'd keyword stands
/// out by contrast.
fn ansi_for(category: HighlightCategory) -> Option<&'static str> {
    match category {
        HighlightCategory::Keyword => Some(YELLOW),
        HighlightCategory::Function => Some(CYAN),
        HighlightCategory::String => Some(GREEN),
        HighlightCategory::Number => Some(MAGENTA),
        HighlightCategory::Comment => Some(GREY),
        // A `$name` reference gets its own colour so a writer sees at a glance
        // which `$name`s are bound vs typos.
        HighlightCategory::Parameter => Some(BLUE),
        HighlightCategory::Plain => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_cypher_keywords_case_insensitively() {
        let c = Completer::with_static_vocabulary();
        let matches = c.candidates("ret");
        assert!(matches.contains(&"RETURN".to_string()));
    }

    #[test]
    fn completion_spans_all_three_tables() {
        let c = Completer::with_static_vocabulary();
        assert!(c.candidates("RETURN").contains(&"RETURN".to_string())); // cypher
        assert!(c.candidates("DATABASE").contains(&"DATABASE".to_string())); // memgraph
        assert!(c.candidates("toInt").contains(&"TOINTEGER".to_string())); // function
    }

    #[test]
    fn a_prefix_matching_nothing_yields_no_candidates() {
        let c = Completer::with_static_vocabulary();
        assert!(c.candidates("zzqx").is_empty());
    }

    #[test]
    fn an_empty_prefix_offers_nothing() {
        let c = Completer::with_static_vocabulary();
        assert!(c.candidates("").is_empty());
    }

    #[test]
    fn candidates_are_deduplicated() {
        // POINT is both a Cypher keyword and a function name; it must appear once.
        let c = Completer::with_static_vocabulary();
        let points: Vec<_> = c
            .candidates("POINT")
            .into_iter()
            .filter(|w| w == "POINT")
            .collect();
        assert_eq!(points.len(), 1, "POINT should not be duplicated");
    }

    #[test]
    fn an_added_source_contributes_candidates() {
        struct Labels;
        impl CompletionSource for Labels {
            fn extend_matches(&self, prefix_upper: &str, out: &mut Vec<String>) {
                if "PERSON".starts_with(prefix_upper) {
                    out.push("Person".to_string());
                }
            }
        }
        let mut c = Completer::with_static_vocabulary();
        c.add_source(Box::new(Labels));
        assert!(c.candidates("Per").contains(&"Person".to_string()));
    }

    #[test]
    fn word_start_is_the_prefix_under_the_cursor() {
        // "MATCH (n) RET" — cursor at end; the word starts at "RET".
        let line = "MATCH (n) RET";
        assert_eq!(word_start(line, line.len()), line.len() - 3);
        assert_eq!(&line[word_start(line, line.len())..], "RET");
    }

    #[test]
    fn word_start_after_a_boundary_is_an_empty_prefix() {
        let line = "MATCH ";
        assert_eq!(word_start(line, line.len()), line.len());
    }

    #[test]
    fn word_start_at_the_line_start_is_zero() {
        assert_eq!(word_start("MATCH", 5), 0);
    }

    /// Categorise the first token of a single-token input, so the classification
    /// can be asserted with no ANSI and no terminal (acceptance: pure, ANSI-free).
    fn category_of(src: &str) -> HighlightCategory {
        let token = lex(src).into_iter().next().expect("at least one token");
        categorize(&token, token.text(src))
    }

    #[test]
    fn categorize_classifies_each_token_kind_without_ansi() {
        assert_eq!(category_of("match"), HighlightCategory::Keyword); // cypher, case-insensitive
        assert_eq!(category_of("DATABASE"), HighlightCategory::Keyword); // memgraph
        assert_eq!(category_of("abs"), HighlightCategory::Function);
        assert_eq!(category_of("myVariable"), HighlightCategory::Plain);
        assert_eq!(category_of("'hi'"), HighlightCategory::String);
        assert_eq!(category_of("42"), HighlightCategory::Number);
        assert_eq!(category_of("// note"), HighlightCategory::Comment);
        assert_eq!(category_of("$age"), HighlightCategory::Parameter);
    }

    #[test]
    fn punctuation_categorizes_as_plain() {
        assert_eq!(category_of("("), HighlightCategory::Plain);
    }

    #[test]
    fn categorize_keyword_wins_over_function_when_a_name_is_in_both() {
        // POINT is both a Cypher keyword and a function name; keyword precedence
        // holds (matching mgconsole), independent of any colouring.
        assert_eq!(category_of("POINT"), HighlightCategory::Keyword);
    }

    #[test]
    fn highlight_colours_keywords_yellow_and_functions_cyan() {
        let out = highlight("MATCH (n) RETURN abs(n.x)");
        assert!(
            out.contains("\x1b[33mMATCH\x1b[0m"),
            "keyword yellow: {out:?}"
        );
        assert!(
            out.contains("\x1b[33mRETURN\x1b[0m"),
            "keyword yellow: {out:?}"
        );
        // Functions are cyan now, not bright-red (slice 02 palette change).
        assert!(out.contains("\x1b[36mabs\x1b[0m"), "function cyan: {out:?}");
        assert!(!out.contains("\x1b[91m"), "no bright-red anywhere: {out:?}");
        // Boundaries and plain words survive untouched.
        assert!(out.contains("(n)"));
    }

    #[test]
    fn highlight_leaves_a_plain_line_unchanged() {
        // Plain identifiers and punctuation only — none of the coloured
        // categories, so the line passes through untouched.
        assert_eq!(highlight("some plain words (n.x)"), "some plain words (n.x)");
    }

    #[test]
    fn a_keyword_inside_a_string_is_not_coloured() {
        // 'MATCH' here is a String token, not a Word — the false positive the
        // lexer cutover fixes.
        let out = highlight("RETURN 'MATCH'");
        assert!(out.contains("\x1b[33mRETURN\x1b[0m"), "the real keyword: {out:?}");
        assert!(out.contains("'MATCH'"), "string survives intact: {out:?}");
        assert!(
            !out.contains("\x1b[33mMATCH"),
            "the in-string keyword is not coloured: {out:?}"
        );
    }

    #[test]
    fn a_function_name_inside_a_comment_is_not_coloured() {
        let out = highlight("RETURN 1 // call abs here");
        assert!(
            !out.contains("\x1b[36m"),
            "nothing inside the comment is coloured cyan: {out:?}"
        );
    }

    #[test]
    fn strings_numbers_and_comments_get_the_literal_palette() {
        assert!(
            highlight("RETURN 'hi'").contains("\x1b[32m'hi'\x1b[0m"),
            "strings green"
        );
        assert!(
            highlight("RETURN 42").contains("\x1b[35m42\x1b[0m"),
            "numbers magenta"
        );
        assert!(
            highlight("RETURN 1 // note").contains("\x1b[90m// note\x1b[0m"),
            "comments grey"
        );
    }

    #[test]
    fn punctuation_and_identifiers_stay_terminal_default() {
        // A typo'd keyword renders as a plain identifier (no colour), so it
        // stands out by contrast; punctuation is never coloured either.
        assert_eq!(highlight("retrn (n.x) + 1.0 * y"), "retrn (n.x) + \x1b[35m1.0\x1b[0m * y");
    }

    #[test]
    fn an_unterminated_string_colours_green_to_end_of_buffer() {
        // Mid-typing signal: the open quote lexes to EOF and colours through.
        assert!(
            highlight("RETURN 'foo").contains("\x1b[32m'foo\x1b[0m"),
            "unterminated string colours green to end"
        );
    }

    #[test]
    fn a_parameter_reference_is_coloured_blue() {
        assert!(
            highlight("RETURN $age").contains("\x1b[34m$age\x1b[0m"),
            "parameter blue"
        );
    }

    #[test]
    fn a_dollar_inside_a_string_is_not_coloured_as_a_parameter() {
        // '$x' is part of a String token, not a Parameter — it colours green,
        // never blue.
        let out = highlight("RETURN '$x'");
        assert!(out.contains("\x1b[32m'$x'\x1b[0m"), "the string is green: {out:?}");
        assert!(!out.contains("\x1b[34m"), "no parameter blue: {out:?}");
    }
}
