//! Cypher syntax support for the REPL: static completion and highlighting
//! (slice 18), carried over from `mgconsole`'s replxx hooks.
//!
//! The logic lives here as pure functions over `&str` so it is tested without a
//! terminal; `main`'s rustyline `Helper` is a thin adapter that calls in. The
//! completer is built from a list of [`CompletionSource`]s — today just the
//! [`StaticVocabulary`] of keyword/function tables — so a live schema-aware
//! source (labels, properties) becomes one more entry in that list, with no
//! change to the cursor handling or the rustyline adapter.

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

/// What a word is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordKind {
    /// A Cypher or Memgraph keyword.
    Keyword,
    /// An awesome-function name.
    Function,
    /// Anything else (identifiers, literals, punctuation).
    Plain,
}

/// Classify a single word (case-insensitively). Keywords win over functions when
/// a name appears in both tables, matching `mgconsole`'s precedence.
pub fn classify(word: &str) -> WordKind {
    let upper = word.to_uppercase();
    if CYPHER_KEYWORDS.contains(&upper.as_str()) || MEMGRAPH_KEYWORDS.contains(&upper.as_str()) {
        WordKind::Keyword
    } else if AWESOME_FUNCTIONS.contains(&upper.as_str()) {
        WordKind::Function
    } else {
        WordKind::Plain
    }
}

const YELLOW: &str = "\x1b[33m";
const BRIGHT_RED: &str = "\x1b[91m";
const RESET: &str = "\x1b[0m";

/// Re-emit `line` with keywords coloured yellow and functions bright red (the
/// `mgconsole` scheme), leaving word-boundary characters untouched. Plain words
/// pass through uncoloured.
pub fn highlight(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut word = String::new();

    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        match classify(word) {
            WordKind::Keyword => {
                out.push_str(YELLOW);
                out.push_str(word);
                out.push_str(RESET);
            }
            WordKind::Function => {
                out.push_str(BRIGHT_RED);
                out.push_str(word);
                out.push_str(RESET);
            }
            WordKind::Plain => out.push_str(word),
        }
        word.clear();
    };

    for ch in line.chars() {
        if WORD_BOUNDARIES.contains(&ch) {
            flush(&mut word, &mut out);
            out.push(ch);
        } else {
            word.push(ch);
        }
    }
    flush(&mut word, &mut out);
    out
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

    #[test]
    fn classify_recognises_each_kind() {
        assert_eq!(classify("match"), WordKind::Keyword); // cypher, case-insensitive
        assert_eq!(classify("DATABASE"), WordKind::Keyword); // memgraph
        assert_eq!(classify("abs"), WordKind::Function);
        assert_eq!(classify("myVariable"), WordKind::Plain);
    }

    #[test]
    fn highlight_colours_keywords_and_functions_only() {
        let out = highlight("MATCH (n) RETURN abs(n.x)");
        assert!(
            out.contains("\x1b[33mMATCH\x1b[0m"),
            "keyword yellow: {out:?}"
        );
        assert!(
            out.contains("\x1b[33mRETURN\x1b[0m"),
            "keyword yellow: {out:?}"
        );
        assert!(
            out.contains("\x1b[91mabs\x1b[0m"),
            "function bright-red: {out:?}"
        );
        // Boundaries and plain words survive untouched.
        assert!(out.contains("(n)"));
    }

    #[test]
    fn highlight_leaves_a_plain_line_unchanged() {
        assert_eq!(highlight("just some words 123"), "just some words 123");
    }
}
