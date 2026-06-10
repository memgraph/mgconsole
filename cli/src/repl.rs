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

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::Duration;

use mgconsole_core::{render, tabular, DisplayMode, Error, QueryAssembler, Value};

use crate::settings::Settings;

/// Whether a buffer the user has entered is a complete submission, or still
/// needs a continuation line. Drives rustyline's `Validator`: incomplete input
/// keeps the editor open (slice 16 acceptance: "keeps editing until the
/// `QueryAssembler` reports a complete query").
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
    /// `:param <name> <expr>` — evaluate the Cypher expression server-side and
    /// store the result as `$name` for later queries (slice 20).
    SetParam { name: String, expr: String },
    /// `:params` — list the currently set parameters.
    ListParams,
    /// `:params clear` — remove all parameters.
    ClearParams,
    /// `:set` with no argument — list every Setting and its current value.
    ListSettings,
    /// `:set <name> <value>` — change one Setting (console behaviour, never query
    /// data — kept distinct from `:param`).
    SetSetting { name: String, value: String },
    /// A recognised command used wrongly (e.g. `:param` with no expression). The
    /// message explains the misuse so the Frontend can report it without ending
    /// the session.
    Invalid(String),
    /// A `:`-prefixed word that is not recognised.
    Unknown(String),
}

/// Recognise a meta-command in a submitted line, or `None` if it is ordinary
/// query text. Only the no-pending case calls this, so the whole line is the
/// command.
pub fn meta_command(line: &str) -> Option<MetaCommand> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix(':')?;
    let (keyword, args) = split_first_word(rest);
    Some(match keyword {
        "quit" | "exit" => MetaCommand::Quit,
        "help" => MetaCommand::Help,
        "docs" => MetaCommand::Docs,
        "param" => parse_set_param(args),
        "params" => parse_params(args),
        "set" => parse_set_setting(args),
        _ => MetaCommand::Unknown(trimmed.to_string()),
    })
}

/// Parse the argument of `:param` into a `SetParam`, or an [`MetaCommand::Invalid`]
/// describing the misuse. The name is the first word; everything after it is the
/// Cypher expression, kept verbatim (it may contain spaces, e.g. `21 * 2`).
fn parse_set_param(args: &str) -> MetaCommand {
    let (name, expr) = split_first_word(args);
    if name.is_empty() || expr.is_empty() {
        return MetaCommand::Invalid(
            ":param needs a name and an expression, e.g. ':param age 21 * 2'".to_string(),
        );
    }
    MetaCommand::SetParam {
        name: name.to_string(),
        expr: expr.to_string(),
    }
}

/// Parse the argument of `:params`: empty lists, `clear` empties, anything else
/// is a misuse.
fn parse_params(args: &str) -> MetaCommand {
    match args {
        "" => MetaCommand::ListParams,
        "clear" => MetaCommand::ClearParams,
        other => MetaCommand::Invalid(format!(
            ":params takes no argument or 'clear', got '{other}'"
        )),
    }
}

/// Parse the argument of `:set`: empty lists every Setting; `<name> <value>`
/// changes one. A name with no value is a misuse (distinct from listing, which
/// takes no name at all).
fn parse_set_setting(args: &str) -> MetaCommand {
    let args = args.trim();
    if args.is_empty() {
        return MetaCommand::ListSettings;
    }
    let (name, value) = split_first_word(args);
    if value.is_empty() {
        return MetaCommand::Invalid(format!(
            ":set needs a value, e.g. ':set display vertical' (or ':set' to list); got '{name}'"
        ));
    }
    MetaCommand::SetSetting {
        name: name.to_string(),
        value: value.to_string(),
    }
}

/// Split a string into its first whitespace-delimited word and the trimmed
/// remainder. Both are `""` when absent.
fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], s[i..].trim()),
        None => (s, ""),
    }
}

/// Render the current parameters for `:params`. Empty reads as a clear sentence;
/// otherwise one `$name = value` per line, ordered by name (the store is a
/// `BTreeMap`). A string value is quoted so it is unambiguous in the listing.
pub fn format_params(params: &BTreeMap<String, Value>) -> String {
    if params.is_empty() {
        return "No parameters set.".to_string();
    }
    params
        .iter()
        .map(|(name, value)| format!("${name} = {}", display_param(value)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render a stored parameter value for the listing: a string is quoted (matching
/// the nested-string convention of tabular rendering) so it reads unambiguously;
/// every other Value uses the shared tabular renderer.
fn display_param(value: &Value) -> String {
    match value {
        Value::String(s) => format!("\"{s}\""),
        other => render::tabular(other),
    }
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
     \t:params clear          Remove all query parameters\n\
     \t:set                   List all console settings and their values\n\
     \t:set <name> <value>    Change a console setting (e.g. ':set display vertical')"
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

/// Runs queries against the database. Abstracted so the loop is testable without
/// a live database. `params` carries the REPL's current `:param` store, bound to
/// every query so `$name` references resolve (slice 20).
pub trait QueryRunner {
    /// Run one complete query bound to the current parameters and render it in the
    /// given display mode (the `display` Setting, resolved by the loop so a
    /// runtime `:set display` takes effect on the next query).
    fn run(
        &mut self,
        query: &str,
        params: &BTreeMap<String, Value>,
        display: DisplayMode,
    ) -> Result<Rendered, Error>;

    /// Evaluate a `:param` expression server-side with the existing parameters in
    /// scope, returning the resulting Value to store. Implemented by running
    /// `RETURN <expr>` and taking the single value back.
    fn evaluate(&mut self, expr: &str, params: &BTreeMap<String, Value>) -> Result<Value, Error>;
}

/// Frontend-local REPL configuration.
pub struct ReplConfig {
    /// The tabular row cap, so an overflow warning can name it.
    pub row_cap: usize,
    /// The console Settings resolved at startup (default < CLI flag). The loop
    /// takes its own mutable copy so runtime `:set` can change it.
    pub settings: Settings,
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
    // The `:param` store, bound to every query so `$name` references resolve.
    let mut params: BTreeMap<String, Value> = BTreeMap::new();
    // The console Settings, seeded from the resolved config; runtime `:set`
    // mutates this copy. Kept rigorously distinct from `params` (CONTEXT.md).
    let mut settings = config.settings.clone();
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
                // Evaluate the expression server-side with the existing params in
                // scope, then store the result. A bad expression is reported (the
                // Session survives it; slice 14) and the loop continues.
                Some(MetaCommand::SetParam { name, expr }) => {
                    match runner.evaluate(&expr, &params) {
                        Ok(value) => {
                            params.insert(name, value);
                        }
                        Err(e) => writeln!(err, "error: {e}")?,
                    }
                    continue;
                }
                Some(MetaCommand::ListParams) => {
                    writeln!(out, "{}", format_params(&params))?;
                    continue;
                }
                Some(MetaCommand::ClearParams) => {
                    params.clear();
                    continue;
                }
                Some(MetaCommand::ListSettings) => {
                    writeln!(out, "{}", settings.list())?;
                    continue;
                }
                // A bad name or value is reported (the session survives it) and
                // the store is left untouched; never touches the `:param` store.
                Some(MetaCommand::SetSetting { name, value }) => {
                    match settings.set(&name, &value) {
                        Ok(()) => writeln!(out, "{name} = {}", settings.get(&name).unwrap_or(value))?,
                        Err(message) => writeln!(err, "error: {message}")?,
                    }
                    continue;
                }
                Some(MetaCommand::Invalid(message)) => {
                    writeln!(err, "error: {message}")?;
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
            match runner.run(&query, &params, settings.display) {
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
        for command in [":help", ":quit", ":docs", ":param", ":params", ":set"] {
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

    // --- :param / :params parsing (pure) -------------------------------------

    #[test]
    fn set_param_keeps_the_whole_expression_verbatim() {
        // The expression may contain spaces; everything after the name is it.
        assert_eq!(
            meta_command(":param age 21 * 2"),
            Some(MetaCommand::SetParam {
                name: "age".to_string(),
                expr: "21 * 2".to_string(),
            })
        );
    }

    #[test]
    fn params_lists_and_clears() {
        assert_eq!(meta_command(":params"), Some(MetaCommand::ListParams));
        assert_eq!(
            meta_command(":params clear"),
            Some(MetaCommand::ClearParams)
        );
    }

    #[test]
    fn param_without_a_name_or_expression_is_invalid() {
        assert!(matches!(
            meta_command(":param"),
            Some(MetaCommand::Invalid(_))
        ));
        assert!(matches!(
            meta_command(":param age"),
            Some(MetaCommand::Invalid(_))
        ));
    }

    #[test]
    fn params_with_an_unknown_argument_is_invalid() {
        assert!(matches!(
            meta_command(":params bogus"),
            Some(MetaCommand::Invalid(_))
        ));
    }

    // --- :set parsing (pure) -------------------------------------------------

    #[test]
    fn set_with_no_argument_lists_settings() {
        assert_eq!(meta_command(":set"), Some(MetaCommand::ListSettings));
        assert_eq!(meta_command("  :set  "), Some(MetaCommand::ListSettings));
    }

    #[test]
    fn set_with_a_name_and_value_changes_one_setting() {
        assert_eq!(
            meta_command(":set display vertical"),
            Some(MetaCommand::SetSetting {
                name: "display".to_string(),
                value: "vertical".to_string(),
            })
        );
    }

    #[test]
    fn set_with_a_name_but_no_value_is_invalid() {
        assert!(matches!(
            meta_command(":set display"),
            Some(MetaCommand::Invalid(_))
        ));
    }

    #[test]
    fn format_params_reads_clearly_when_empty() {
        assert_eq!(format_params(&BTreeMap::new()), "No parameters set.");
    }

    #[test]
    fn format_params_lists_each_param_ordered_with_strings_quoted() {
        let params = BTreeMap::from([
            ("age".to_string(), Value::Integer(42)),
            ("name".to_string(), Value::String("Ada".to_string())),
        ]);
        // BTreeMap order: age before name.
        assert_eq!(format_params(&params), "$age = 42\n$name = \"Ada\"");
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

    /// Records every query (and the params bound to it) and returns scripted
    /// results. `:param` evaluations are recorded separately and answered from a
    /// scripted value queue (defaulting to `Null`), so a test can assert both the
    /// expression seen and the params in scope at evaluation time.
    struct ScriptedRunner {
        seen: Vec<String>,
        seen_params: Vec<BTreeMap<String, Value>>,
        seen_display: Vec<DisplayMode>,
        results: VecDeque<Result<Rendered, Error>>,
        eval_seen: Vec<(String, BTreeMap<String, Value>)>,
        eval_results: VecDeque<Result<Value, Error>>,
    }

    impl ScriptedRunner {
        fn returning(results: Vec<Result<Rendered, Error>>) -> Self {
            Self {
                seen: Vec::new(),
                seen_params: Vec::new(),
                seen_display: Vec::new(),
                results: results.into(),
                eval_seen: Vec::new(),
                eval_results: VecDeque::new(),
            }
        }

        /// Pre-seed the values `evaluate` hands back, in call order.
        fn evaluating(mut self, values: Vec<Result<Value, Error>>) -> Self {
            self.eval_results = values.into();
            self
        }
    }

    impl QueryRunner for ScriptedRunner {
        fn run(
            &mut self,
            query: &str,
            params: &BTreeMap<String, Value>,
            display: DisplayMode,
        ) -> Result<Rendered, Error> {
            self.seen.push(query.to_string());
            self.seen_params.push(params.clone());
            self.seen_display.push(display);
            self.results
                .pop_front()
                .unwrap_or_else(|| Ok(ok_result("", 0)))
        }

        fn evaluate(
            &mut self,
            expr: &str,
            params: &BTreeMap<String, Value>,
        ) -> Result<Value, Error> {
            self.eval_seen.push((expr.to_string(), params.clone()));
            self.eval_results.pop_front().unwrap_or(Ok(Value::Null))
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
        drive_with(lines, ScriptedRunner::returning(results))
    }

    fn drive_with(
        lines: Vec<Line>,
        mut runner: ScriptedRunner,
    ) -> (ScriptedSource, ScriptedRunner, String, String) {
        let mut source = ScriptedSource::of(lines);
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_loop(
            &mut source,
            &mut runner,
            &mut out,
            &mut err,
            &ReplConfig {
                row_cap: 1000,
                settings: Settings::default(),
            },
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
        assert_eq!(
            runner.seen,
            vec!["RETURN 1".to_string(), "RETURN 2".to_string()]
        );
        assert_eq!(out.matches("row in set").count(), 2);
    }

    #[test]
    fn a_query_error_is_reported_and_the_loop_survives() {
        let boom = Error::Query(mgconsole_core::error::QueryError {
            code: "Memgraph.ClientError.MemgraphError.SyntaxError".to_string(),
            message: "bad cypher".to_string(),
        });
        let (_src, runner, out, err) = drive(
            vec![Line::Text("BAD;".into()), Line::Text("RETURN 1;".into())],
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
        let (_src, _runner, _out, err) = drive(
            vec![Line::Text("MATCH (n) RETURN n;".into())],
            vec![Ok(capped)],
        );
        assert!(err.contains("1000-row"), "warning names the cap: {err}");
    }

    #[test]
    fn an_unknown_meta_command_is_reported_not_run() {
        let (_src, runner, _out, err) = drive(vec![Line::Text(":bogus".into())], vec![]);
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

    // --- :param family, driven through the loop -------------------------------

    #[test]
    fn a_set_param_is_bound_to_a_later_query() {
        let runner = ScriptedRunner::returning(vec![Ok(ok_result("t", 1))])
            .evaluating(vec![Ok(Value::Integer(42))]);
        let (_src, runner, _out, _err) = drive_with(
            vec![
                Line::Text(":param age 21 * 2".into()),
                Line::Text("RETURN $age;".into()),
            ],
            runner,
        );
        // The expression was evaluated server-side, and the stored value bound to
        // the next query.
        assert_eq!(runner.eval_seen[0].0, "21 * 2");
        assert_eq!(
            runner.seen_params,
            vec![BTreeMap::from([("age".to_string(), Value::Integer(42))])]
        );
    }

    #[test]
    fn existing_params_are_in_scope_when_evaluating_a_new_one() {
        let runner = ScriptedRunner::returning(vec![])
            .evaluating(vec![Ok(Value::Integer(1)), Ok(Value::Integer(2))]);
        let (_src, runner, _out, _err) = drive_with(
            vec![
                Line::Text(":param x 1".into()),
                Line::Text(":param y $x + 1".into()),
            ],
            runner,
        );
        // The second evaluation saw `x` already in scope.
        assert_eq!(runner.eval_seen[1].0, "$x + 1");
        assert_eq!(
            runner.eval_seen[1].1,
            BTreeMap::from([("x".to_string(), Value::Integer(1))])
        );
    }

    #[test]
    fn params_lists_then_clears_the_store() {
        let runner = ScriptedRunner::returning(vec![]).evaluating(vec![Ok(Value::Integer(7))]);
        let (_src, _runner, out, _err) = drive_with(
            vec![
                Line::Text(":param n 7".into()),
                Line::Text(":params".into()),
                Line::Text(":params clear".into()),
                Line::Text(":params".into()),
            ],
            runner,
        );
        // First listing shows the param; after clear, the listing is empty again.
        assert!(out.contains("$n = 7"), "listing shows the param: {out}");
        assert!(out.contains("No parameters set."), "cleared: {out}");
    }

    #[test]
    fn a_cleared_param_is_no_longer_bound() {
        let runner = ScriptedRunner::returning(vec![Ok(ok_result("t", 1))])
            .evaluating(vec![Ok(Value::Integer(7))]);
        let (_src, runner, _out, _err) = drive_with(
            vec![
                Line::Text(":param n 7".into()),
                Line::Text(":params clear".into()),
                Line::Text("RETURN 1;".into()),
            ],
            runner,
        );
        assert_eq!(runner.seen_params, vec![BTreeMap::new()]);
    }

    #[test]
    fn a_malformed_param_command_is_reported_and_the_loop_survives() {
        let (_src, runner, _out, err) = drive(
            vec![Line::Text(":param".into()), Line::Text("RETURN 1;".into())],
            vec![Ok(ok_result("t", 1))],
        );
        assert!(err.contains("error:"), "misuse reported: {err}");
        // The session survived: the following query still ran.
        assert_eq!(runner.seen, vec!["RETURN 1".to_string()]);
    }

    // --- :set family, driven through the loop ---------------------------------

    #[test]
    fn set_display_takes_effect_on_the_next_query() {
        let (_src, runner, out, _err) = drive(
            vec![
                Line::Text(":set display vertical".into()),
                Line::Text("RETURN 1;".into()),
            ],
            vec![Ok(ok_result("t", 1))],
        );
        // The query ran with the freshly-set display mode.
        assert_eq!(runner.seen_display, vec![DisplayMode::Vertical]);
        // The change was echoed.
        assert!(out.contains("display = vertical"), "echoed: {out}");
    }

    #[test]
    fn bare_set_lists_settings() {
        let (_src, _runner, out, _err) = drive(vec![Line::Text(":set".into())], vec![]);
        assert!(out.contains("display = auto"), "listing: {out}");
    }

    #[test]
    fn an_unknown_setting_is_reported_and_the_loop_survives() {
        let (_src, runner, _out, err) = drive(
            vec![
                Line::Text(":set bogus 1".into()),
                Line::Text("RETURN 1;".into()),
            ],
            vec![Ok(ok_result("t", 1))],
        );
        assert!(err.contains("error:"), "misuse reported: {err}");
        assert!(err.contains("bogus"), "names the setting: {err}");
        // The session survived: the following query still ran (with the default).
        assert_eq!(runner.seen, vec!["RETURN 1".to_string()]);
        assert_eq!(runner.seen_display, vec![DisplayMode::Auto]);
    }

    #[test]
    fn set_and_param_are_distinct_surfaces() {
        // Setting `display` never populates the `:param` store, and vice versa.
        let runner = ScriptedRunner::returning(vec![Ok(ok_result("t", 1))])
            .evaluating(vec![Ok(Value::Integer(7))]);
        let (_src, runner, _out, _err) = drive_with(
            vec![
                Line::Text(":set display vertical".into()),
                Line::Text(":param n 7".into()),
                Line::Text("RETURN 1;".into()),
            ],
            runner,
        );
        // The query saw the param but the setting stayed out of the param store.
        assert_eq!(
            runner.seen_params,
            vec![BTreeMap::from([("n".to_string(), Value::Integer(7))])]
        );
        assert_eq!(runner.seen_display, vec![DisplayMode::Vertical]);
    }

    #[test]
    fn a_failed_param_evaluation_is_reported_and_the_loop_survives() {
        let boom = Error::Query(mgconsole_core::error::QueryError {
            code: "Memgraph.ClientError.MemgraphError.SyntaxError".to_string(),
            message: "bad expression".to_string(),
        });
        let runner =
            ScriptedRunner::returning(vec![Ok(ok_result("t", 1))]).evaluating(vec![Err(boom)]);
        let (_src, runner, _out, err) = drive_with(
            vec![
                Line::Text(":param x @@@".into()),
                Line::Text("RETURN 1;".into()),
            ],
            runner,
        );
        assert!(err.contains("bad expression"), "eval error surfaced: {err}");
        // Nothing was stored, and the next query still ran with no params.
        assert_eq!(runner.seen_params, vec![BTreeMap::new()]);
    }
}
