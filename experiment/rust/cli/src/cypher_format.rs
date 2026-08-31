//! Cypher pretty-printer (issue 15): a pure `text -> text` function over the Core
//! lexer's [`Token`] stream, with no parser of its own.
//!
//! [`format`] re-emits a query with one clause per line, single-space token
//! separation (preserving the author's token *adjacency*, so `abs(x)` and
//! `(n:Person)` stay tight), and clause keywords upper-cased. It is driven entirely
//! by the lexer — clause breaks key off keyword *shape*, never a grammar — so the
//! same function could later back a `:format` Meta-command; today it is the
//! Workbench's auto-format gesture only.
//!
//! Two invariants make it safe to bind to a destructive editor gesture:
//!
//! - **Never changes semantics.** The output is re-lexed and its non-whitespace
//!   tokens are compared to the input's (kind plus case-folded text). If anything
//!   but whitespace and keyword case differs, [`format`] declines and returns the
//!   input unchanged. Upper-casing is withheld in identifier positions (after `:`
//!   or `.`, or before `:`), so case-sensitive labels, types, properties, and map
//!   keys are never touched.
//! - **Leaves partial input alone.** A half-typed query — an unterminated string or
//!   block comment — is returned unchanged with a clear reason, never mangled.

use mgconsole_core::{lex, Token, TokenKind};

use crate::keywords::{CYPHER_KEYWORDS, MEMGRAPH_KEYWORDS};

/// The keywords that begin a new line when formatting — the reading-order clauses.
/// A keyword not in this set (an operator like `AND`, an inline `DISTINCT`) never
/// forces a break; it is upper-cased only when it continues a clause-keyword run
/// (`ORDER BY`, `ON CREATE SET`, `DETACH DELETE`).
const CLAUSE_KEYWORDS: &[&str] = &[
    "MATCH", "OPTIONAL", "WHERE", "RETURN", "WITH", "CREATE", "MERGE", "DELETE", "DETACH", "SET",
    "REMOVE", "ORDER", "SKIP", "LIMIT", "UNWIND", "FOREACH", "CALL", "YIELD", "UNION", "ON",
    "USING", "LOAD",
];

/// Pretty-print `input`, or return the input unchanged with a reason when it is
/// partial (an unterminated string/comment) or when formatting would alter the
/// token stream (the semantics guard). `Ok` is the formatted text.
pub fn format(input: &str) -> Result<String, String> {
    let tokens = lex(input);
    if let Some(reason) = incomplete_reason(input, &tokens) {
        return Err(reason);
    }
    let formatted = render(input, &tokens);
    if tokens_differ(input, &formatted) {
        return Err("formatting would change the query; left unchanged".to_string());
    }
    Ok(formatted)
}

/// Whether `word` is a Cypher/Memgraph keyword (case-insensitive).
fn is_keyword(word: &str) -> bool {
    let upper = word.to_ascii_uppercase();
    CYPHER_KEYWORDS.contains(&upper.as_str()) || MEMGRAPH_KEYWORDS.contains(&upper.as_str())
}

/// Whether `word` is a clause-leading keyword (case-insensitive).
fn is_clause_keyword(word: &str) -> bool {
    CLAUSE_KEYWORDS.contains(&word.to_ascii_uppercase().as_str())
}

/// A reason the input is partial and must be left untouched, or `None` if it is
/// complete enough to format. Only the last non-whitespace token can be unclosed,
/// since the lexer runs an unterminated string/comment to end of input.
fn incomplete_reason(input: &str, tokens: &[Token]) -> Option<String> {
    let last = tokens.iter().rev().find(|t| t.kind != TokenKind::Whitespace)?;
    let text = last.text(input);
    match last.kind {
        TokenKind::Comment if text.starts_with("/*") && !text.ends_with("*/") => {
            Some("unterminated block comment; left unchanged".to_string())
        }
        TokenKind::String if !is_closed_string(text) => {
            Some("unterminated string; left unchanged".to_string())
        }
        _ => None,
    }
}

/// Whether a String token's text is a closed literal (mirrors the lexer's
/// escape-aware scan): walk the body and report whether the opening quote is
/// matched before the end.
fn is_closed_string(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(quote) = chars.next() else {
        return true;
    };
    let mut iter = chars.peekable();
    while let Some(c) = iter.next() {
        if quote != '`' && c == '\\' {
            iter.next(); // the escaped char cannot close the string
            continue;
        }
        if c == quote {
            if quote == '`' && iter.peek() == Some(&'`') {
                iter.next(); // doubled backtick escapes a backtick
                continue;
            }
            return true; // matched the opening quote
        }
    }
    false
}

/// Re-emit the tokens with normalised spacing, clause newlines, and keyword case.
fn render(input: &str, tokens: &[Token]) -> String {
    let mut out = String::new();
    // The previous *emitted* (non-whitespace) token's text, and whether any
    // whitespace separated it from the token now being emitted.
    let mut prev: Option<&str> = None;
    let mut prev_upper_clause = false;
    let mut had_space = false;

    for token in tokens {
        if token.kind == TokenKind::Whitespace {
            had_space = true;
            continue;
        }
        let raw = token.text(input);
        let next = next_nonws(input, tokens, token);

        // Keyword case: upper-case a keyword unless it sits in an identifier
        // position (a label/type after `:`, a property after `.`, or a map key
        // before `:`) or just inside a pattern/list opener, where it could be a
        // variable. A keyword continuing a clause run is also upper-cased.
        let in_identifier_pos =
            matches!(prev, Some(":" | "." | "(" | "[")) || next == Some(":");
        let recase = token.kind == TokenKind::Word
            && is_keyword(raw)
            && (!in_identifier_pos || prev_upper_clause);
        let text = if recase {
            raw.to_ascii_uppercase()
        } else {
            raw.to_string()
        };

        // Separator before this token.
        let is_clause = token.kind == TokenKind::Word && is_clause_keyword(raw);
        let prev_is_keyword = prev.is_some_and(is_keyword);
        let prev_blocks_break = matches!(prev, Some("(" | "[" | "{" | ":" | "." | ","));
        if let Some(prev_text) = prev {
            if is_clause && !prev_is_keyword && !prev_blocks_break {
                out.push('\n');
            } else if prev_text.starts_with("//") {
                // A line comment runs to end of line; the next token must not be
                // folded onto it (it would become commented out).
                out.push('\n');
            } else if had_space {
                out.push(' ');
            }
        }
        out.push_str(&text);

        prev = Some(token.text(input));
        prev_upper_clause = recase && (is_clause || prev_upper_clause);
        had_space = false;
    }
    out
}

/// The next non-whitespace token's text after `token`, for one-token lookahead.
fn next_nonws<'a>(input: &'a str, tokens: &'a [Token], token: &Token) -> Option<&'a str> {
    let start = token.span.end;
    tokens
        .iter()
        .find(|t| t.span.start >= start && t.kind != TokenKind::Whitespace)
        .map(|t| t.text(input))
}

/// Whether `formatted` lexes to a different non-whitespace token stream than
/// `input` (the semantics guard): a difference in count, kind, or case-folded text
/// means a token was dropped, merged, added, or substantively altered.
fn tokens_differ(input: &str, formatted: &str) -> bool {
    let strip = |text: &str| -> Vec<(TokenKind, String)> {
        lex(text)
            .into_iter()
            .filter(|t| t.kind != TokenKind::Whitespace)
            .map(|t| (t.kind, t.text(text).to_ascii_lowercase()))
            .collect()
    };
    strip(input) != strip(formatted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_clause_query_is_unchanged_apart_from_keyword_case() {
        assert_eq!(format("match (n) return n").unwrap(), "MATCH (n)\nRETURN n");
    }

    #[test]
    fn each_clause_starts_a_new_line() {
        let input = "MATCH (n:Person) WHERE n.age > 21 RETURN n.name ORDER BY n.name";
        assert_eq!(
            format(input).unwrap(),
            "MATCH (n:Person)\nWHERE n.age > 21\nRETURN n.name\nORDER BY n.name"
        );
    }

    #[test]
    fn token_adjacency_is_preserved_so_calls_and_patterns_stay_tight() {
        // No space is inserted where the author wrote none: `abs(x)` and `(n)` stay
        // tight; existing spaces collapse to one.
        assert_eq!(
            format("WITH   abs(x)  AS   y RETURN y").unwrap(),
            "WITH abs(x) AS y\nRETURN y"
        );
    }

    #[test]
    fn multi_word_clauses_are_not_split() {
        assert_eq!(
            format("optional match (n) detach delete n").unwrap(),
            "OPTIONAL MATCH (n)\nDETACH DELETE n"
        );
    }

    #[test]
    fn merge_on_create_set_stays_on_one_clause_line() {
        assert_eq!(
            format("merge (n) on create set n.x = 1").unwrap(),
            "MERGE (n)\nON CREATE SET n.x = 1"
        );
    }

    #[test]
    fn a_label_matching_a_keyword_keeps_its_case() {
        // `:User` (USER is a Memgraph keyword) and `:Index` are case-sensitive
        // labels — they must not be upper-cased.
        assert_eq!(format("match (n:User) return n").unwrap(), "MATCH (n:User)\nRETURN n");
        assert_eq!(format("MATCH (n:Index) RETURN n").unwrap(), "MATCH (n:Index)\nRETURN n");
    }

    #[test]
    fn a_relationship_type_matching_a_keyword_keeps_its_case() {
        let input = "match (a)-[r:Reset]->(b) return r";
        assert_eq!(format(input).unwrap(), "MATCH (a)-[r:Reset]->(b)\nRETURN r");
    }

    #[test]
    fn a_map_key_matching_a_keyword_keeps_its_case() {
        // `order` is a map key here, not the ORDER clause.
        assert_eq!(format("return {order: 1}").unwrap(), "RETURN {order: 1}");
    }

    #[test]
    fn comments_are_preserved() {
        let input = "// header\nmatch (n) return n // trailing";
        assert_eq!(
            format(input).unwrap(),
            "// header\nMATCH (n)\nRETURN n // trailing"
        );
    }

    #[test]
    fn a_block_comment_is_preserved_inline() {
        assert_eq!(
            format("match /* hint */ (n) return n").unwrap(),
            "MATCH /* hint */ (n)\nRETURN n"
        );
    }

    #[test]
    fn a_string_containing_a_keyword_is_untouched() {
        // The keyword and the clause break live inside the literal: one token.
        assert_eq!(
            format("return 'match where return'").unwrap(),
            "RETURN 'match where return'"
        );
    }

    #[test]
    fn parameters_survive_and_are_not_recased() {
        assert_eq!(format("return $userName").unwrap(), "RETURN $userName");
    }

    #[test]
    fn an_unterminated_string_is_left_unchanged() {
        let err = format("RETURN 'half").expect_err("partial");
        assert!(err.contains("unterminated string"), "{err}");
    }

    #[test]
    fn an_unterminated_block_comment_is_left_unchanged() {
        let err = format("MATCH (n) /* open").expect_err("partial");
        assert!(err.contains("unterminated block comment"), "{err}");
    }

    #[test]
    fn formatting_is_idempotent() {
        let input = "match (n:Person) where n.age > 21 return n.name order by n.name";
        let once = format(input).expect("formats");
        let twice = format(&once).expect("re-formats");
        assert_eq!(once, twice, "formatting an already-formatted query is a no-op");
    }

    #[test]
    fn the_token_stream_is_preserved_modulo_whitespace_and_keyword_case() {
        // The semantics guard's own invariant, asserted directly: the formatted
        // output lexes to the same non-whitespace tokens (case-folded).
        let input = "MATCH (n:Person)-[:KNOWS]->(m) WHERE n.age>21 RETURN n,m";
        let out = format(input).expect("formats");
        assert!(!tokens_differ(input, &out), "tokens preserved");
    }

    #[test]
    fn blank_input_formats_to_blank() {
        assert_eq!(format("").unwrap(), "");
        assert_eq!(format("   \n  ").unwrap(), "");
    }
}
