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

use std::io::{self, Write};
use std::time::Duration;

use mgconsole_core::{tabular, Error, QueryAssembler};

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
    /// Print interactive-mode usage (`:help`).
    Help,
    /// Print documentation pointers (`:docs`).
    Docs,
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
        "help" => MetaCommand::Help,
        "docs" => MetaCommand::Docs,
        _ => MetaCommand::Unknown(trimmed.to_string()),
    })
}

/// Interactive-mode usage, printed by `:help`. Carried over from `mgconsole`'s
/// usage text, listing the commands the REPL implements (the `:param`/`:params`
/// family lands in slice 20).
pub fn help_text() -> &'static str {
    "In interactive mode you can enter Cypher queries and the commands below.\n\
     \n\
     Cypher queries can span multiple lines and conclude with a semicolon (;).\n\
     Each query is executed against the database and its results are printed.\n\
     \n\
     Supported commands:\n\
     \n\
     \t:help                  Print this usage for interactive mode\n\
     \t:quit                  Exit the shell (or press Ctrl-D)\n\
     \t:docs                  Print pointers to Memgraph documentation\n\
     \t:param <name> <expr>   Set a query parameter to a Cypher expression\n\
     \t                       (e.g. ':param age 21 * 2'); use it as $<name>\n\
     \t:params                List all currently set query parameters\n\
     \t:params clear          Remove all query parameters"
}

/// Documentation pointers, printed by `:docs`. Carried over from `mgconsole`.
pub fn docs_text() -> &'static str {
    "If you are new to Memgraph or the Cypher query language, check out these resources:\n\
     \n\
     \tQuerying with Cypher:    https://memgr.ph/querying\n\
     \tImporting data:          https://memgr.ph/importing-data\n\
     \tDatabase configuration:  https://memgr.ph/configuration\n\
     \n\
     Official mgconsole documentation: https://memgr.ph/mgconsole"
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

/// One unit of input handed back by a [`LineSource`]: a line of text, or a
/// terminal control signal the loop must act on.
pub enum Line {
    /// A line the user submitted (possibly a whole multiline buffer).
    Text(String),
    /// Ctrl-C: abandon the statement being assembled and start a fresh prompt.
    Interrupted,
    /// Ctrl-D / end of input: leave the REPL.
    Eof,
}

/// Where the REPL reads input. `continued` is true when a statement is part-way
/// assembled, so an interactive source can show a continuation prompt instead of
/// the primary one. Abstracted so the loop is testable without a terminal.
pub trait LineSource {
    fn read(&mut self, continued: bool) -> io::Result<Line>;
}

/// One query's result, rendered and measured, ready for the loop to print. The
/// runner owns rendering (it holds the table options and the row cap), so the
/// loop only writes the table text and the summary.
pub struct Rendered {
    /// The rendered table (empty for a result with no columns, e.g. a write).
    pub table: String,
    /// How many rows were rendered (capped at the row cap).
    pub row_count: usize,
    /// Whether the result exceeded the row cap and was truncated.
    pub overflowed: bool,
    /// Round-trip time for the query (run + drain).
    pub elapsed: Duration,
}

/// Runs one complete query and returns its rendered result. Abstracted so the
/// loop is testable without a live database.
pub trait QueryRunner {
    fn run(&mut self, query: &str) -> Result<Rendered, Error>;
}

/// Frontend-local REPL configuration.
pub struct ReplConfig {
    /// The tabular row cap, so an overflow warning can name it.
    pub row_cap: usize,
}

/// The REPL's execute loop: read input, assemble it into complete queries, run
/// each, and print its result and summary. A query error is reported and the
/// loop continues (the Session survives it; slice 14). Exits on `:quit`, Ctrl-D,
/// or end of input.
///
/// Generic over its IO seams so it runs identically under rustyline-with-a-
/// Session and under a test's scripted input with a fake runner.
pub fn run_loop(
    source: &mut dyn LineSource,
    runner: &mut dyn QueryRunner,
    out: &mut dyn Write,
    err: &mut dyn Write,
    config: &ReplConfig,
) -> io::Result<()> {
    let mut assembler = QueryAssembler::new();
    loop {
        let continued = assembler.has_pending();
        let text = match source.read(continued)? {
            Line::Eof => break,
            // Drop the half-typed statement and re-prompt from scratch.
            Line::Interrupted => {
                assembler = QueryAssembler::new();
                continue;
            }
            Line::Text(text) => text,
        };

        // A meta-command is only meaningful at the start of a statement; mid-
        // assembly the same text is ordinary query content.
        if !continued {
            match meta_command(&text) {
                Some(MetaCommand::Quit) => break,
                Some(MetaCommand::Help) => {
                    writeln!(out, "{}", help_text())?;
                    continue;
                }
                Some(MetaCommand::Docs) => {
                    writeln!(out, "{}", docs_text())?;
                    continue;
                }
                Some(MetaCommand::Unknown(cmd)) => {
                    writeln!(err, "error: unknown command '{cmd}'")?;
                    continue;
                }
                None => {}
            }
        }

        // The trailing newline lets a line comment close and separates physical
        // lines when assembling across reads.
        for query in assembler.push(&format!("{text}\n")) {
            match runner.run(&query) {
                Ok(result) => {
                    if !result.table.is_empty() {
                        writeln!(out, "{}", result.table)?;
                    }
                    writeln!(out, "{}", format_summary(result.row_count, result.elapsed))?;
                    if result.overflowed {
                        writeln!(err, "{}", tabular::row_cap_warning(config.row_cap))?;
                    }
                }
                // The Session classifies recoverable vs fatal and recovers
                // internally (slice 14); the REPL reports and keeps going.
                Err(e) => writeln!(err, "error: {e}")?,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

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
    fn help_and_docs_are_recognised() {
        assert_eq!(meta_command(":help"), Some(MetaCommand::Help));
        assert_eq!(meta_command(":docs"), Some(MetaCommand::Docs));
    }

    #[test]
    fn an_unknown_colon_word_is_reported_not_run() {
        assert_eq!(
            meta_command(":frobnicate"),
            Some(MetaCommand::Unknown(":frobnicate".to_string()))
        );
    }

    #[test]
    fn help_text_describes_query_entry_and_every_implemented_command() {
        let help = help_text();
        // Query entry is explained.
        assert!(help.contains("Cypher"));
        assert!(help.contains("semicolon"));
        // Every implemented command is listed (acceptance criterion 4).
        for command in [":help", ":quit", ":docs", ":param", ":params"] {
            assert!(help.contains(command), "help should list {command}");
        }
    }

    #[test]
    fn docs_text_points_at_memgraph_documentation() {
        assert!(docs_text().contains("memgr.ph"));
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

    // --- run_loop, driven by scripted fakes (no terminal, no database) -------

    /// Hands back pre-scripted lines and records the `continued` flag it was
    /// asked with on each read (so a test can assert continuation prompting).
    struct ScriptedSource {
        lines: VecDeque<Line>,
        continued_at: Vec<bool>,
    }

    impl ScriptedSource {
        fn of(lines: Vec<Line>) -> Self {
            Self {
                lines: lines.into(),
                continued_at: Vec::new(),
            }
        }
    }

    impl LineSource for ScriptedSource {
        fn read(&mut self, continued: bool) -> io::Result<Line> {
            self.continued_at.push(continued);
            Ok(self.lines.pop_front().unwrap_or(Line::Eof))
        }
    }

    /// Records every query it is asked to run and returns scripted results.
    struct ScriptedRunner {
        seen: Vec<String>,
        results: VecDeque<Result<Rendered, Error>>,
    }

    impl ScriptedRunner {
        fn returning(results: Vec<Result<Rendered, Error>>) -> Self {
            Self {
                seen: Vec::new(),
                results: results.into(),
            }
        }
    }

    impl QueryRunner for ScriptedRunner {
        fn run(&mut self, query: &str) -> Result<Rendered, Error> {
            self.seen.push(query.to_string());
            self.results
                .pop_front()
                .unwrap_or_else(|| Ok(ok_result("", 0)))
        }
    }

    fn ok_result(table: &str, rows: usize) -> Rendered {
        Rendered {
            table: table.to_string(),
            row_count: rows,
            overflowed: false,
            elapsed: Duration::from_millis(1),
        }
    }

    fn drive(
        lines: Vec<Line>,
        results: Vec<Result<Rendered, Error>>,
    ) -> (ScriptedSource, ScriptedRunner, String, String) {
        let mut source = ScriptedSource::of(lines);
        let mut runner = ScriptedRunner::returning(results);
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_loop(
            &mut source,
            &mut runner,
            &mut out,
            &mut err,
            &ReplConfig { row_cap: 1000 },
        )
        .expect("loop runs to EOF");
        (
            source,
            runner,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn runs_a_completed_query_and_prints_table_then_summary() {
        let (_src, runner, out, _err) = drive(
            vec![Line::Text("RETURN 1;".into())],
            vec![Ok(ok_result("<the table>", 1))],
        );
        assert_eq!(runner.seen, vec!["RETURN 1".to_string()]);
        assert!(out.contains("<the table>"));
        assert!(out.contains("1 row in set (0.001 sec)"));
    }

    #[test]
    fn assembles_a_query_across_lines_with_a_continuation_prompt() {
        let (src, runner, _out, _err) = drive(
            vec![
                Line::Text("MATCH (n)".into()),
                Line::Text("RETURN n;".into()),
            ],
            vec![Ok(ok_result("t", 1))],
        );
        // One query, assembled from both lines.
        assert_eq!(runner.seen, vec!["MATCH (n)\nRETURN n".to_string()]);
        // The second read was asked with `continued = true` (continuation prompt).
        assert_eq!(src.continued_at, vec![false, true, false]);
    }

    #[test]
    fn runs_multiple_queries_from_one_submission_in_order() {
        let (_src, runner, out, _err) = drive(
            vec![Line::Text("RETURN 1; RETURN 2;".into())],
            vec![Ok(ok_result("a", 1)), Ok(ok_result("b", 1))],
        );
        assert_eq!(runner.seen, vec!["RETURN 1".to_string(), "RETURN 2".to_string()]);
        assert_eq!(out.matches("row in set").count(), 2);
    }

    #[test]
    fn a_query_error_is_reported_and_the_loop_survives() {
        let boom = Error::Query(mgconsole_core::error::QueryError {
            code: "Memgraph.ClientError.MemgraphError.SyntaxError".to_string(),
            message: "bad cypher".to_string(),
        });
        let (_src, runner, out, err) = drive(
            vec![
                Line::Text("BAD;".into()),
                Line::Text("RETURN 1;".into()),
            ],
            vec![Err(boom), Ok(ok_result("t", 1))],
        );
        // The error was surfaced, then the next query still ran.
        assert!(err.contains("error:"));
        assert!(err.contains("bad cypher"));
        assert_eq!(runner.seen.len(), 2);
        assert!(out.contains("1 row in set"));
    }

    #[test]
    fn quit_exits_before_reading_further_input() {
        let (_src, runner, _out, _err) = drive(
            vec![Line::Text(":quit".into()), Line::Text("RETURN 1;".into())],
            vec![],
        );
        assert!(runner.seen.is_empty(), "nothing runs after :quit");
    }

    #[test]
    fn end_of_input_exits_cleanly() {
        let (_src, runner, out, err) = drive(vec![Line::Eof], vec![]);
        assert!(runner.seen.is_empty());
        assert!(out.is_empty());
        assert!(err.is_empty());
    }

    #[test]
    fn ctrl_c_abandons_the_half_typed_statement() {
        let (_src, runner, _out, _err) = drive(
            vec![
                Line::Text("MATCH (n)".into()),
                Line::Interrupted,
                Line::Text("RETURN 1;".into()),
            ],
            vec![Ok(ok_result("t", 1))],
        );
        // The interrupted fragment is gone: only the fresh statement runs.
        assert_eq!(runner.seen, vec!["RETURN 1".to_string()]);
    }

    #[test]
    fn an_overflowing_result_warns_about_the_row_cap() {
        let mut capped = ok_result("big", 1000);
        capped.overflowed = true;
        let (_src, _runner, _out, err) =
            drive(vec![Line::Text("MATCH (n) RETURN n;".into())], vec![Ok(capped)]);
        assert!(err.contains("1000-row"), "warning names the cap: {err}");
    }

    #[test]
    fn an_unknown_meta_command_is_reported_not_run() {
        let (_src, runner, _out, err) =
            drive(vec![Line::Text(":bogus".into())], vec![]);
        assert!(runner.seen.is_empty());
        assert!(err.contains("unknown command"));
    }

    #[test]
    fn help_and_docs_print_to_output_without_running_a_query() {
        let (_src, runner, out, _err) = drive(
            vec![Line::Text(":help".into()), Line::Text(":docs".into())],
            vec![],
        );
        assert!(runner.seen.is_empty(), "no query runs for :help/:docs");
        assert!(out.contains("Supported commands"), "help printed: {out}");
        assert!(out.contains("memgr.ph"), "docs printed: {out}");
    }
}
