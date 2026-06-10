//! Line/query parsing (slice 15): turn raw input into complete queries.
//!
//! A query spans multiple lines until a terminating `;`; several queries may
//! share a line; an unfinished trailing fragment carries over to the next
//! input. A `;` inside a string literal (`'…'`, `"…"`), a quoted identifier
//! (`` `…` ``), or a comment (`// …`, `/* … */`) does not terminate. Pure — no
//! database.

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

#[derive(Clone, Copy)]
enum State {
    Normal,
    Single,
    Double,
    Backtick,
    LineComment,
    BlockComment,
}

/// Scan a buffer into (complete queries, unterminated remainder). Empty queries
/// (e.g. between `;;`) are skipped.
fn scan(buf: &str) -> (Vec<String>, String) {
    use State::{Backtick, BlockComment, Double, LineComment, Normal, Single};

    let chars: Vec<char> = buf.chars().collect();
    let mut complete = Vec::new();
    let mut state = Normal;
    let mut start = 0;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match state {
            Normal => match c {
                '\'' => state = Single,
                '"' => state = Double,
                '`' => state = Backtick,
                '/' if chars.get(i + 1) == Some(&'/') => {
                    state = LineComment;
                    i += 1;
                }
                '/' if chars.get(i + 1) == Some(&'*') => {
                    state = BlockComment;
                    i += 1;
                }
                ';' => {
                    let q: String = chars[start..i].iter().collect();
                    let trimmed = q.trim();
                    if !trimmed.is_empty() {
                        complete.push(trimmed.to_string());
                    }
                    start = i + 1;
                }
                _ => {}
            },
            Single => match c {
                '\\' => i += 1, // skip escaped char
                '\'' => state = Normal,
                _ => {}
            },
            Double => match c {
                '\\' => i += 1,
                '"' => state = Normal,
                _ => {}
            },
            Backtick => {
                if c == '`' {
                    if chars.get(i + 1) == Some(&'`') {
                        i += 1; // doubled backtick escapes a backtick
                    } else {
                        state = Normal;
                    }
                }
            }
            LineComment => {
                if c == '\n' {
                    state = Normal;
                }
            }
            BlockComment => {
                if c == '*' && chars.get(i + 1) == Some(&'/') {
                    state = Normal;
                    i += 1;
                }
            }
        }
        i += 1;
    }

    let remainder: String = chars[start..].iter().collect();
    (complete, remainder)
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
