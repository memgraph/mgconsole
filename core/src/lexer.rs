//! Cypher lexer (slice 01): turn query text into a stream of total, typed
//! [`Token`]s — the input-side dual of the [`Value`](crate::Value) model
//! (CONTEXT.md, ADR 0008). Each token carries its byte span; concatenating the
//! spans reproduces the input exactly (the stream *tiles* the input). A token is
//! recognised by **shape alone**, never by consulting the database, reusing the
//! same string/comment/backtick/escape-aware discipline `parse.rs` and
//! `clause.rs` encode by hand today (consolidated onto this lexer in slices
//! 06/07).
//!
//! The lexer is **total from the outset** (ADR 0003 idiom): every kind is
//! emitted from the first cut, even kinds no consumer colours yet. It is pure
//! (no database, no IO) and unit-tested in isolation, exactly as the
//! [`QueryAssembler`](crate::QueryAssembler) (slice 15) and the clause scanner
//! (slice 27) are.

use std::ops::Range;

/// What a [`Token`] is, by shape alone. The lexer never decides keyword vs
/// function vs identifier — every alphanumeric run is a [`Word`](TokenKind::Word)
/// and that classification is the Frontend's job (ADR 0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// A run of alphanumerics, `_`, and `.` (so `point.distance` and `n.name`
    /// are each one Word), not starting with a digit. Emitted unclassified.
    Word,
    /// An integer / float / scientific literal (`42`, `3.14`, `1e10`).
    Number,
    /// A `'…'`, `"…"`, or backtick `` `…` `` literal, honouring `\` escapes and
    /// doubled backticks. A `;` or keyword inside is part of the one token.
    String,
    /// A `// …` line comment (up to, not including, the newline) or a `/* … */`
    /// block comment.
    Comment,
    /// A `$name` query-parameter reference.
    Parameter,
    /// A single punctuation/operator character not absorbed above (so a `;` is
    /// always its own token, never merged with neighbours).
    Punct,
    /// A run of whitespace, including newlines.
    Whitespace,
}

/// A lexical unit of Cypher query text: its [`kind`](Token::kind) and the byte
/// [`span`](Token::span) it occupies in the input. Spans tile the input with no
/// gaps or overlaps, so re-concatenating every token's text reproduces the
/// input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Range<usize>,
}

impl Token {
    /// The slice of `input` this token covers. `input` must be the same text the
    /// token was lexed from.
    pub fn text<'a>(&self, input: &'a str) -> &'a str {
        &input[self.span.clone()]
    }
}

/// Lex query text into its token stream. Pure: no database, no IO. The returned
/// tokens' spans tile `input` exactly.
pub fn lex(input: &str) -> Vec<Token> {
    // Index over (byte-offset, char) so spans are byte offsets while recognition
    // works in chars (multibyte-safe). `byte_at` maps a char index — including
    // the one-past-the-end sentinel — back to a byte offset.
    let cs: Vec<(usize, char)> = input.char_indices().collect();
    let n = cs.len();
    let byte_at = |k: usize| cs.get(k).map_or(input.len(), |&(b, _)| b);

    let mut tokens = Vec::new();
    let mut i = 0;
    while i < n {
        let start = i;
        let c = cs[i].1;
        let kind = if c.is_whitespace() {
            while i < n && cs[i].1.is_whitespace() {
                i += 1;
            }
            TokenKind::Whitespace
        } else if c == '/' && char_at(&cs, i + 1) == Some('/') {
            // Line comment: up to, not including, the newline (which is its own
            // Whitespace token).
            i += 2;
            while i < n && cs[i].1 != '\n' {
                i += 1;
            }
            TokenKind::Comment
        } else if c == '/' && char_at(&cs, i + 1) == Some('*') {
            i = scan_block_comment(&cs, i + 2);
            TokenKind::Comment
        } else if c == '\'' || c == '"' || c == '`' {
            i = scan_string(&cs, i, c);
            TokenKind::String
        } else if c == '$' && char_at(&cs, i + 1).is_some_and(is_param_char) {
            i += 1; // the `$`
            while i < n && is_param_char(cs[i].1) {
                i += 1;
            }
            TokenKind::Parameter
        } else if c.is_ascii_digit() {
            i = scan_number(&cs, i);
            TokenKind::Number
        } else if is_word_start(c) {
            i += 1;
            while i < n && is_word_char(cs[i].1) {
                i += 1;
            }
            TokenKind::Word
        } else {
            // A single punctuation/operator char, so a `;` is always its own
            // token (parse.rs relies on this in slice 06).
            i += 1;
            TokenKind::Punct
        };
        tokens.push(Token {
            kind,
            span: byte_at(start)..byte_at(i),
        });
    }
    tokens
}

/// Scan from just past a string's opening `quote` to one-past its close,
/// honouring `\` escapes (for `'`/`"`) and doubled backticks. An unterminated
/// string returns the end of input, so a mid-typed line still lexes.
fn scan_string(cs: &[(usize, char)], open: usize, quote: char) -> usize {
    let n = cs.len();
    let mut i = open + 1;
    while i < n {
        let c = cs[i].1;
        if quote != '`' && c == '\\' {
            i += 2; // skip the backslash and whatever it escapes
            continue;
        }
        if c == quote {
            if quote == '`' && char_at(cs, i + 1) == Some('`') {
                i += 2; // doubled backtick escapes a backtick
                continue;
            }
            return i + 1; // include the closing quote
        }
        i += 1;
    }
    n // unterminated: to end of input
}

/// Scan from just past a `/*` to one-past its `*/`. Unterminated → end of input.
fn scan_block_comment(cs: &[(usize, char)], from: usize) -> usize {
    let n = cs.len();
    let mut i = from;
    while i < n {
        if cs[i].1 == '*' && char_at(cs, i + 1) == Some('/') {
            return i + 2;
        }
        i += 1;
    }
    n
}

/// The char at index `k`, or `None` past the end — the lexer's one-char
/// lookahead.
fn char_at(cs: &[(usize, char)], k: usize) -> Option<char> {
    cs.get(k).map(|&(_, c)| c)
}

/// Scan a numeric literal (integer / float / scientific) from `start`, which is
/// known to be a digit. A trailing non-numeric char ends the run (and begins the
/// next token).
fn scan_number(cs: &[(usize, char)], start: usize) -> usize {
    let n = cs.len();
    let mut i = start;
    while i < n {
        match cs[i].1 {
            c if c.is_ascii_digit() => i += 1,
            '.' => i += 1,
            'e' | 'E' => {
                i += 1;
                if matches!(char_at(cs, i), Some('+' | '-')) {
                    i += 1; // signed exponent
                }
            }
            _ => break,
        }
    }
    i
}

/// A word's first char: a letter, `_`, or `.` (a digit would begin a Number).
fn is_word_start(c: char) -> bool {
    c == '_' || c == '.' || c.is_alphabetic()
}

/// A word's interior char: alphanumeric, `_`, or `.` (so `n.name` is one Word).
fn is_word_char(c: char) -> bool {
    c == '_' || c == '.' || c.is_alphanumeric()
}

/// A char that may follow `$` in a parameter name: alphanumeric or `_`.
fn is_param_char(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::TokenKind::{Comment, Number, Parameter, Punct, String as Str, Whitespace, Word};
    use super::*;

    /// Lex into `(kind, text)` pairs — the shape a test asserts against.
    fn kinds(input: &str) -> Vec<(TokenKind, &str)> {
        lex(input)
            .iter()
            .map(|t| (t.kind, t.text(input)))
            .collect()
    }

    #[test]
    fn spans_tile_the_input_losslessly() {
        let input = "MATCH (n:Person {age: 42}) // note\nRETURN n.name, $p;";
        let tokens = lex(input);
        // No gaps or overlaps: each span begins where the previous ended.
        let mut at = 0;
        for t in &tokens {
            assert_eq!(t.span.start, at, "gap or overlap before {t:?}");
            at = t.span.end;
        }
        assert_eq!(at, input.len(), "final span must reach end of input");
        // Re-concatenation reproduces the input exactly.
        let rebuilt: std::string::String = tokens.iter().map(|t| t.text(input)).collect();
        assert_eq!(rebuilt, input);
    }

    #[test]
    fn words_are_emitted_unclassified() {
        // The lexer never consults a keyword table: MATCH, RETURN, abs are all
        // plain Words, distinguished only by shape from numbers/strings.
        assert_eq!(
            kinds("MATCH abs RETURN"),
            vec![
                (Word, "MATCH"),
                (Whitespace, " "),
                (Word, "abs"),
                (Whitespace, " "),
                (Word, "RETURN"),
            ]
        );
    }

    #[test]
    fn dot_is_a_word_constituent() {
        // `n.name` and `point.distance` are each one Word (the dotted-builtin
        // rider), so the function table still matches them downstream.
        assert_eq!(kinds("n.name"), vec![(Word, "n.name")]);
        assert_eq!(kinds("point.distance"), vec![(Word, "point.distance")]);
    }

    #[test]
    fn numbers_are_int_float_scientific() {
        assert_eq!(kinds("42"), vec![(Number, "42")]);
        assert_eq!(kinds("3.14"), vec![(Number, "3.14")]);
        assert_eq!(kinds("1e10"), vec![(Number, "1e10")]);
        assert_eq!(kinds("6.022e+23"), vec![(Number, "6.022e+23")]);
    }

    #[test]
    fn single_and_double_quoted_strings_are_one_token() {
        assert_eq!(kinds("'hello'"), vec![(Str, "'hello'")]);
        assert_eq!(kinds("\"hello\""), vec![(Str, "\"hello\"")]);
    }

    #[test]
    fn backtick_identifier_is_one_string_token() {
        assert_eq!(kinds("`weird name`"), vec![(Str, "`weird name`")]);
    }

    #[test]
    fn a_semicolon_or_keyword_inside_a_string_is_not_surfaced() {
        // The `;` and `RETURN` are inside the literal: one String token, no
        // Punct/Word split out of it.
        assert_eq!(kinds("'a;b RETURN'"), vec![(Str, "'a;b RETURN'")]);
    }

    #[test]
    fn backslash_escapes_keep_a_string_open() {
        // The escaped quote does not close the string.
        assert_eq!(kinds(r"'a\'b'"), vec![(Str, r"'a\'b'")]);
        assert_eq!(kinds(r#""a\"b""#), vec![(Str, r#""a\"b""#)]);
    }

    #[test]
    fn doubled_backtick_escapes_a_backtick() {
        assert_eq!(kinds("`a``b`"), vec![(Str, "`a``b`")]);
    }

    #[test]
    fn line_and_block_comments_are_one_token_each() {
        // A line comment runs up to (not including) the newline; the newline is
        // whitespace.
        assert_eq!(
            kinds("// hi\nx"),
            vec![(Comment, "// hi"), (Whitespace, "\n"), (Word, "x")]
        );
        assert_eq!(kinds("/* a;b */"), vec![(Comment, "/* a;b */")]);
    }

    #[test]
    fn a_parameter_reference_is_one_token() {
        assert_eq!(kinds("$age"), vec![(Parameter, "$age")]);
        assert_eq!(
            kinds("RETURN $x"),
            vec![(Word, "RETURN"), (Whitespace, " "), (Parameter, "$x")]
        );
    }

    #[test]
    fn a_bare_dollar_is_punctuation() {
        // `$` not followed by a name is not a parameter.
        assert_eq!(kinds("$ "), vec![(Punct, "$"), (Whitespace, " ")]);
    }

    #[test]
    fn punctuation_is_one_char_per_token_so_a_semicolon_stands_alone() {
        assert_eq!(
            kinds("(n);"),
            vec![
                (Punct, "("),
                (Word, "n"),
                (Punct, ")"),
                (Punct, ";"),
            ]
        );
    }

    #[test]
    fn an_unterminated_string_runs_to_end_of_input() {
        // Mid-typing a line can still be coloured: the open quote lexes to EOF.
        assert_eq!(kinds("RETURN 'foo"), vec![
            (Word, "RETURN"),
            (Whitespace, " "),
            (Str, "'foo"),
        ]);
    }

    #[test]
    fn an_unterminated_block_comment_runs_to_end_of_input() {
        assert_eq!(kinds("/* open"), vec![(Comment, "/* open")]);
    }

    #[test]
    fn multibyte_characters_keep_byte_spans_aligned() {
        // A non-ASCII char inside a string must not desync the byte spans.
        let input = "'café' x";
        assert_eq!(
            kinds(input),
            vec![(Str, "'café'"), (Whitespace, " "), (Word, "x")]
        );
        // Lossless re-concatenation still holds across the multibyte boundary.
        let rebuilt: std::string::String =
            lex(input).iter().map(|t| t.text(input)).collect();
        assert_eq!(rebuilt, input);
    }
}
