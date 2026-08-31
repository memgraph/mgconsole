//! Line/query parsing (slice 15): turn raw input into complete queries.
//!
//! A query spans multiple lines until a terminating `;`; several queries may
//! share a line; an unfinished trailing fragment carries over to the next
//! input. A `;` inside a string literal (`'…'`, `"…"`), a quoted identifier
//! (`` `…` ``), or a comment (`// …`, `/* … */`) does not terminate. Pure — no
//! database.
//!
//! The string/comment awareness comes from the Core [`lexer`](crate::lexer)
//! (ADR 0008 deepening, slice 06): a terminator is a `;` `Punct` token, since a
//! `;` inside a string or comment is part of that String/Comment token and never
//! surfaces as `Punct`. One lexical state machine, not a second copy here.

use crate::lexer::{lex, TokenKind};

/// Accumulates input across feeds and emits complete queries as they terminate.
#[derive(Debug, Default)]
pub struct QueryAssembler {
    pending: String,
}

impl QueryAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of input (typically one line). Returns the queries that
    /// completed, in order; an unterminated tail is retained.
    pub fn push(&mut self, input: &str) -> Vec<String> {
        self.pending.push_str(input);
        let (complete, remainder) = scan(&self.pending);
        self.pending = remainder;
        complete
    }

    /// The unterminated tail not yet emitted.
    pub fn pending(&self) -> &str {
        &self.pending
    }

    /// Whether there is buffered input awaiting a terminator.
    pub fn has_pending(&self) -> bool {
        !self.pending.trim().is_empty()
    }
}

/// Scan a buffer into (complete queries, unterminated remainder), splitting on
/// each `;` `Punct` token from the Core lexer. A `;` inside a string or comment
/// is part of a String/Comment token, not `Punct`, so it never terminates. Empty
/// queries (e.g. between `;;`) are skipped.
fn scan(buf: &str) -> (Vec<String>, String) {
    let mut complete = Vec::new();
    let mut start = 0; // byte index where the current query begins
    for token in lex(buf) {
        if token.kind == TokenKind::Punct && token.text(buf) == ";" {
            let query = buf[start..token.span.start].trim();
            if !query.is_empty() {
                complete.push(query.to_string());
            }
            start = token.span.end;
        }
    }
    (complete, buf[start..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(input: &str) -> (Vec<String>, String) {
        let mut a = QueryAssembler::new();
        let complete = a.push(input);
        (complete, a.pending().to_string())
    }

    #[test]
    fn single_query_terminates_on_semicolon() {
        assert_eq!(
            split("RETURN 1;"),
            (vec!["RETURN 1".to_string()], String::new())
        );
    }

    #[test]
    fn query_spans_multiple_lines() {
        let mut a = QueryAssembler::new();
        assert!(a.push("MATCH (n)\n").is_empty());
        assert_eq!(a.push("RETURN n;"), vec!["MATCH (n)\nRETURN n".to_string()]);
    }

    #[test]
    fn multiple_queries_on_one_line_split() {
        assert_eq!(
            split("RETURN 1; RETURN 2;").0,
            vec!["RETURN 1".to_string(), "RETURN 2".to_string()]
        );
    }

    #[test]
    fn trailing_fragment_carries_over() {
        let (complete, pending) = split("RETURN 1; RETURN 2");
        assert_eq!(complete, vec!["RETURN 1".to_string()]);
        assert_eq!(pending.trim(), "RETURN 2");
    }

    #[test]
    fn semicolon_inside_strings_does_not_terminate() {
        assert_eq!(split("RETURN 'a;b';").0, vec!["RETURN 'a;b'".to_string()]);
        assert_eq!(
            split("RETURN \"a;b\";").0,
            vec!["RETURN \"a;b\"".to_string()]
        );
    }

    #[test]
    fn escaped_quote_keeps_string_open() {
        assert_eq!(
            split(r"RETURN 'a\';b';").0,
            vec![r"RETURN 'a\';b'".to_string()]
        );
    }

    #[test]
    fn semicolon_inside_comments_does_not_terminate() {
        assert_eq!(
            split("RETURN 1 // c ; x\nRETURN 2;").0,
            vec!["RETURN 1 // c ; x\nRETURN 2".to_string()]
        );
        assert_eq!(
            split("RETURN /* ; */ 1;").0,
            vec!["RETURN /* ; */ 1".to_string()]
        );
    }

    #[test]
    fn semicolon_inside_backtick_identifier_does_not_terminate() {
        assert_eq!(
            split("MATCH (n:`Label;X`) RETURN n;").0,
            vec!["MATCH (n:`Label;X`) RETURN n".to_string()]
        );
    }

    #[test]
    fn empty_queries_between_semicolons_are_skipped() {
        assert_eq!(split(";;RETURN 1;;").0, vec!["RETURN 1".to_string()]);
    }

    #[test]
    fn string_spanning_feeds_is_handled() {
        let mut a = QueryAssembler::new();
        assert!(a.push("RETURN 'a;").is_empty()); // string still open
        assert_eq!(a.push("b';"), vec!["RETURN 'a;b'".to_string()]);
    }
}
