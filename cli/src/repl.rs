//! The interactive REPL Frontend's loop and its pure seams (slice 16).
//!
//! The loop is split from its IO so it can be unit-tested with no terminal and
//! no database (ADR 0002): [`run_loop`] is generic over a [`LineSource`] (where
//! lines come from) and a [`QueryRunner`] (what runs a query), so a test drives
//! it with scripted input and a fake runner and asserts the printed output.
//! `main` wires the real implementations: rustyline for the source, a Session
//! for the runner.
//!
//! The pure helpers below — completeness (multiline continuation), meta-command
//! recognition, and the result summary — carry the detail and are tested
//! directly.

use std::time::Duration;

use mgconsole_core::QueryAssembler;

/// Whether a buffer the user has entered is a complete submission, or still
/// needs a continuation line. Drives rustyline's `Validator`: incomplete input
/// keeps the editor open (slice 16 acceptance: "keeps editing until the
/// QueryAssembler reports a complete query").
///
/// A line beginning with `:` is a single-line meta-command, always complete.
/// Otherwise the buffer is complete once scanning it leaves no unterminated
/// query fragment — i.e. every statement ended in `;` (a bare blank buffer is
/// trivially complete, submitting nothing).
pub fn is_complete(buffer: &str) -> bool {
    if buffer.trim_start().starts_with(':') {
        return true;
    }
    let mut assembler = QueryAssembler::new();
    assembler.push(buffer);
    !assembler.has_pending()
}

/// A REPL meta-command — a `:`-prefixed directive that is not a Cypher query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaCommand {
    /// Leave the REPL (`:quit`).
    Quit,
    /// A `:`-prefixed word that is not recognised.
    Unknown(String),
}

/// Recognise a meta-command in a submitted line, or `None` if it is ordinary
/// query text. Only the no-pending case calls this, so the whole line is the
/// command.
pub fn meta_command(line: &str) -> Option<MetaCommand> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix(':')?;
    let name = rest.split_whitespace().next().unwrap_or("");
    Some(match name {
        "quit" | "exit" => MetaCommand::Quit,
        _ => MetaCommand::Unknown(trimmed.to_string()),
    })
}

/// The one-line summary printed after a query's result: how many rows came back
/// and how long the round-trip took (PRD stories 18 & 19). Mirrors the familiar
/// `mysql`-style line; an empty result reads "Empty set".
pub fn format_summary(rows: usize, elapsed: Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    match rows {
        0 => format!("Empty set ({seconds:.3} sec)"),
        1 => format!("1 row in set ({seconds:.3} sec)"),
        n => format!("{n} rows in set ({seconds:.3} sec)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminated_query_is_complete() {
        assert!(is_complete("RETURN 1;"));
    }

    #[test]
    fn unterminated_query_needs_a_continuation() {
        assert!(!is_complete("RETURN 1"));
        assert!(!is_complete("MATCH (n)"));
    }

    #[test]
    fn a_multiline_query_completes_on_its_terminator() {
        assert!(!is_complete("MATCH (n)\n"));
        assert!(is_complete("MATCH (n)\nRETURN n;"));
    }

    #[test]
    fn an_open_string_is_not_complete_even_with_a_semicolon() {
        // The `;` is inside the string literal, so the statement is unfinished.
        assert!(!is_complete("RETURN 'a;"));
    }

    #[test]
    fn a_blank_buffer_is_trivially_complete() {
        assert!(is_complete(""));
        assert!(is_complete("   \n  "));
    }

    #[test]
    fn a_meta_command_is_always_a_complete_single_line() {
        assert!(is_complete(":quit"));
        assert!(is_complete("  :quit"));
    }

    #[test]
    fn quit_aliases_are_recognised() {
        assert_eq!(meta_command(":quit"), Some(MetaCommand::Quit));
        assert_eq!(meta_command("  :exit  "), Some(MetaCommand::Quit));
    }

    #[test]
    fn an_unknown_colon_word_is_reported_not_run() {
        assert_eq!(
            meta_command(":frobnicate"),
            Some(MetaCommand::Unknown(":frobnicate".to_string()))
        );
    }

    #[test]
    fn ordinary_query_text_is_not_a_meta_command() {
        assert_eq!(meta_command("RETURN 1"), None);
    }

    #[test]
    fn summary_counts_and_times_the_result() {
        assert_eq!(
            format_summary(1, Duration::from_millis(1)),
            "1 row in set (0.001 sec)"
        );
        assert_eq!(
            format_summary(42, Duration::from_millis(123)),
            "42 rows in set (0.123 sec)"
        );
    }

    #[test]
    fn an_empty_result_reads_empty_set() {
        assert_eq!(
            format_summary(0, Duration::from_secs(0)),
            "Empty set (0.000 sec)"
        );
    }
}
