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

use std::path::PathBuf;

use mgconsole_core::{render, tabular, DisplayMode, Error, QueryAssembler, TransactionState, Value};

use crate::queries::NamedQueries;
use crate::settings::Settings;
use crate::OutputFormat;

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
    /// `:begin` — open an explicit transaction (ADR 0011).
    Begin,
    /// `:commit` — commit the open transaction.
    Commit,
    /// `:rollback` — roll back the open transaction.
    Rollback,
    /// `:connect <target>` — swap to a new Session at a profile or `host[:port]`
    /// (issue 07).
    Connect(String),
    /// `:use <db>` — switch the active Database within the Session (issue 08).
    Use(String),
    /// `:sysinfo` — run the server-status queries and render them (issue 09).
    Sysinfo,
    /// `:source <file>` — run a file through the normal query path (issue 10).
    Source(String),
    /// `:watch [interval] [query]` — re-run a query on a timer (issue 11). The
    /// raw argument is parsed at dispatch time, where the last query is known.
    Watch(String),
    /// `:o [format] <file>` — redirect the next query's result to a file (issue
    /// 12). The raw argument is parsed at dispatch time.
    Redirect(String),
    /// `:save <name> [query]` — save a query as a Named template (issue 13). With
    /// no query the last query is saved; `query` is kept verbatim (it may hold
    /// `$param` placeholders, resolved at run time).
    Save { name: String, query: Option<String> },
    /// `:saved` — list the saved Named queries.
    Saved,
    /// `:load <name>` — recall a Named query into the input for review/edit; never
    /// auto-run (issue 13).
    Load(String),
    /// `:forget <name>` — delete a Named query (a deliberately non-generic verb so
    /// it never reads as deleting data, issue 13).
    Forget(String),
    /// `:close` — close the active Workbench Buffer (a Workbench command, so it has
    /// no meaning in the line REPL, which reports it unavailable). The destructive
    /// counterpart to the Buffer-navigation gestures (CONTEXT.md "Workbench command").
    Close,
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
        "begin" => MetaCommand::Begin,
        "commit" => MetaCommand::Commit,
        "rollback" => MetaCommand::Rollback,
        "connect" => {
            if args.is_empty() {
                MetaCommand::Invalid(
                    ":connect needs a profile name or host[:port], e.g. ':connect prod'"
                        .to_string(),
                )
            } else {
                MetaCommand::Connect(args.to_string())
            }
        }
        "use" => {
            if args.is_empty() {
                MetaCommand::Invalid(":use needs a database name, e.g. ':use analytics'".to_string())
            } else {
                MetaCommand::Use(args.to_string())
            }
        }
        "sysinfo" => MetaCommand::Sysinfo,
        "source" => {
            if args.is_empty() {
                MetaCommand::Invalid(":source needs a file path, e.g. ':source setup.cypher'".to_string())
            } else {
                MetaCommand::Source(args.to_string())
            }
        }
        "watch" => MetaCommand::Watch(args.to_string()),
        "o" => {
            if args.is_empty() {
                MetaCommand::Invalid(
                    ":o needs a file path, e.g. ':o out.csv' or ':o csv out.txt'".to_string(),
                )
            } else {
                MetaCommand::Redirect(args.to_string())
            }
        }
        "save" => parse_save(args),
        "saved" => MetaCommand::Saved,
        "close" => MetaCommand::Close,
        "load" => {
            if args.is_empty() {
                MetaCommand::Invalid(":load needs a saved name, e.g. ':load recent'".to_string())
            } else {
                MetaCommand::Load(args.to_string())
            }
        }
        "forget" => {
            if args.is_empty() {
                MetaCommand::Invalid(":forget needs a saved name, e.g. ':forget recent'".to_string())
            } else {
                MetaCommand::Forget(args.to_string())
            }
        }
        _ => MetaCommand::Unknown(trimmed.to_string()),
    })
}

/// Parse `:save <name> [query]` (issue 13). The name is the first word; anything
/// after it is the template, kept verbatim (it may hold spaces and `$param`
/// placeholders). With no query word, the last query is saved (a `None` the
/// dispatcher fills). A bare `:save` with no name is a misuse.
fn parse_save(args: &str) -> MetaCommand {
    let (name, query) = split_first_word(args);
    if name.is_empty() {
        return MetaCommand::Invalid(
            ":save needs a name, e.g. ':save recent' or ':save recent MATCH (n) RETURN n'"
                .to_string(),
        );
    }
    MetaCommand::Save {
        name: name.to_string(),
        query: (!query.is_empty()).then(|| query.to_string()),
    }
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

/// Parse an on/off toggle value (`:set readonly on`). Accepts the common
/// spellings; anything else is a clear error rather than a silent default.
pub fn parse_on_off(value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        other => Err(format!("expected on or off, got '{other}'")),
    }
}

/// Render a toggle as `on`/`off` for listings and confirmations.
pub fn on_off(value: bool) -> &'static str {
    if value {
        "on"
    } else {
        "off"
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
     \t:set <name> <value>    Change a console setting (e.g. ':set display vertical')\n\
     \t:set readonly on       Guard the session read-only (off only at connect time)\n\
     \t:begin                 Open an explicit transaction\n\
     \t:commit                Commit the open transaction\n\
     \t:rollback              Roll back the open transaction\n\
     \t:connect <target>      Swap to another server (a profile or host[:port])\n\
     \t:use <db>              Switch the active database (multi-tenancy)\n\
     \t:sysinfo               Show server version and storage/runtime info\n\
     \t:source <file>         Run a file's queries and commands in this session\n\
     \t:watch [interval] [q]  Re-run a query on a timer (default 2s; Enter stops)\n\
     \t:o [format] <file>     Redirect the next query's result to a file (csv/jsonl/cypherl/table)\n\
     \t:save <name> [query]   Save a query as a named template (the last query if none given)\n\
     \t:saved                 List the saved named queries\n\
     \t:load <name>           Recall a saved query into the input for review (does not run it)\n\
     \t:forget <name>         Delete a saved named query"
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
/// the primary one. `initial`, when `Some`, pre-fills the editor with recalled
/// text the user can review and edit before submitting (`:load`, issue 13).
/// Abstracted so the loop is testable without a terminal.
pub trait LineSource {
    fn read(&mut self, continued: bool, initial: Option<&str>) -> io::Result<Line>;
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

    /// Turn read-only mode on or off on the underlying Session (issue 04). The
    /// loop enforces the asymmetry (off refused at runtime); the runner only
    /// applies the change so the next query carries Bolt access mode READ.
    fn set_read_only(&mut self, on: bool);

    /// Whether the Session is currently read-only, for the `:set` listing and the
    /// prompt marker.
    fn is_read_only(&self) -> bool;

    /// Open an explicit transaction (`:begin`, ADR 0011).
    fn begin(&mut self) -> Result<(), Error>;

    /// Commit the open transaction (`:commit`).
    fn commit(&mut self) -> Result<(), Error>;

    /// Roll back the open transaction (`:rollback`).
    fn rollback(&mut self) -> Result<(), Error>;

    /// The current explicit-transaction state, for the prompt marker.
    fn transaction_state(&self) -> TransactionState;

    /// Swap to a new Session at `target` (a profile name or `host[:port]`, issue
    /// 07). On success returns a label for the confirmation and the prompt
    /// reflects the new connection; on failure the prior Session is left intact.
    fn connect(&mut self, target: &str) -> Result<String, Error>;

    /// Switch the active Database within the Session (`:use`, issue 08). On
    /// success the prompt reflects the new Database; on failure the current one is
    /// left active.
    fn use_database(&mut self, database: &str) -> Result<(), Error>;

    /// Re-run `query` on a timer (`:watch`, issue 11), clearing and redrawing each
    /// tick as a snapshot, until the user stops it (press Enter). Owns its own IO
    /// loop so it is only meaningful for the interactive runner.
    fn watch(
        &mut self,
        query: &str,
        params: &BTreeMap<String, Value>,
        interval: Duration,
        display: DisplayMode,
        out: &mut dyn Write,
    ) -> io::Result<()>;

    /// Run `query` and stream its result to `path` in `format` (`:o`, issue 12),
    /// returning the number of rows written. Streams row-by-row for the streaming
    /// formats (bounded memory); `table` buffers as it must.
    fn run_to_file(
        &mut self,
        query: &str,
        params: &BTreeMap<String, Value>,
        format: OutputFormat,
        path: &std::path::Path,
    ) -> Result<usize, Error>;
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
/// The default `:watch` interval, matching `watch(1)` (issue 11).
pub const DEFAULT_WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// A parsed `:watch` request: how often to re-run, and what to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchSpec {
    pub interval: Duration,
    pub query: String,
}

/// Parse `:watch [interval] [query]` (issue 11). A leading token that reads as an
/// interval (`2`, `2s`, `500ms`, `1.5s`) sets the interval; the rest is the query.
/// With no query the last query is reused; with neither a query nor a prior one it
/// is an error.
pub fn parse_watch(args: &str, last_query: Option<&str>) -> Result<WatchSpec, String> {
    let args = args.trim();
    let (interval, rest) = match args.split_once(char::is_whitespace) {
        // `<interval> <query>`
        Some((first, rest)) if parse_interval(first).is_some() => {
            (parse_interval(first).unwrap(), rest.trim())
        }
        // a lone token that is an interval (`:watch 5`)
        None if !args.is_empty() && parse_interval(args).is_some() => {
            (parse_interval(args).unwrap(), "")
        }
        // no leading interval: the whole argument is the query (or it is empty)
        _ => (DEFAULT_WATCH_INTERVAL, args),
    };
    let query = if rest.is_empty() {
        last_query
            .filter(|q| !q.trim().is_empty())
            .ok_or_else(|| "no previous query to watch; give one: ':watch <query>'".to_string())?
            .to_string()
    } else {
        rest.to_string()
    };
    Ok(WatchSpec { interval, query })
}

/// Parse `:o [format] <file>` (issue 12). A leading token that names a format
/// (`csv`/`jsonl`/`cypherl`/`table`) sets it and the rest is the path; otherwise
/// the whole argument is the path and the format is inferred from its extension.
/// An unknown format/extension is a clear error.
pub fn parse_redirect(args: &str) -> Result<(OutputFormat, PathBuf), String> {
    let args = args.trim();
    if args.is_empty() {
        return Err(":o needs a file path".to_string());
    }
    let (first, rest) = split_first_word(args);
    if let Ok(format) = first.parse::<OutputFormat>() {
        if rest.is_empty() {
            return Err(format!(":o {first} needs a file path, e.g. ':o {first} out.{first}'"));
        }
        return Ok((format, PathBuf::from(rest)));
    }
    let path = PathBuf::from(args);
    let format = OutputFormat::from_extension(&path).ok_or_else(|| {
        format!(
            "cannot infer a format from '{}'; name one: ':o csv {}'",
            path.display(),
            path.display()
        )
    })?;
    Ok((format, path))
}

/// Parse a `:watch` interval token: a plain number or `Ns` is seconds (fractions
/// allowed), `Nms` is milliseconds. Returns `None` for anything else (so it reads
/// as query text instead).
fn parse_interval(token: &str) -> Option<Duration> {
    let token = token.trim();
    if let Some(ms) = token.strip_suffix("ms") {
        return ms.parse::<u64>().ok().map(Duration::from_millis);
    }
    let secs = token.strip_suffix('s').unwrap_or(token);
    secs.parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64)
}

/// The server-status queries `:sysinfo` runs and renders (issue 09): the version
/// and the storage/runtime info Memgraph exposes. Each renders like any other
/// result, and one that a server does not support degrades to a reported error
/// without stopping the rest.
pub const SYSINFO_QUERIES: &[&str] = &["SHOW VERSION", "SHOW STORAGE INFO"];

/// Run one query and print its rendered table, summary, and any overflow warning
/// — the shared body of the loop's per-query handling, `:sysinfo`, and `:source`.
/// Returns whether the query succeeded (so `:source` can stop on the first
/// error); a query error is reported either way without ending the loop.
#[allow(clippy::too_many_arguments)]
fn execute_query(
    runner: &mut dyn QueryRunner,
    query: &str,
    params: &BTreeMap<String, Value>,
    display: DisplayMode,
    row_cap: usize,
    data: &mut dyn Write,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> io::Result<bool> {
    match runner.run(query, params, display) {
        Ok(result) => {
            // Result data goes to the data sink (stdout); the summary and any
            // overflow notice are chrome and go to `out`/`err` (ADR 0014, issue 19).
            if !result.table.is_empty() {
                writeln!(data, "{}", result.table)?;
            }
            writeln!(out, "{}", format_summary(result.row_count, result.elapsed))?;
            if result.overflowed {
                writeln!(err, "{}", tabular::row_cap_warning(row_cap))?;
            }
            Ok(true)
        }
        // The Session classifies recoverable vs fatal and recovers internally
        // (slice 14); the REPL reports and keeps going.
        Err(e) => {
            writeln!(err, "error: {e}")?;
            Ok(false)
        }
    }
}

/// Whether a handled meta-command should end the loop or carry on.
enum MetaFlow {
    /// `:quit` — leave the REPL.
    Quit,
    /// The command was handled (output already written); re-prompt.
    Handled,
    /// `:load` recalled a Named query: pre-fill the next prompt with this text for
    /// the user to review and edit (never auto-run, issue 13).
    Recall(String),
}

/// Handle one recognised meta-command, writing its output/errors. Split from
/// [`run_loop`] so the loop body stays small as the vocabulary grows (issues
/// 04–13). `Unknown`/`Invalid` are reported here too — every `:`-line is a
/// command, handled, never run as a query.
// One arm per command in the shared vocabulary; the match (and the seams it
// threads — settings, params, the Named-query store) grows with each issue.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn dispatch_meta(
    cmd: MetaCommand,
    runner: &mut dyn QueryRunner,
    settings: &mut Settings,
    params: &mut BTreeMap<String, Value>,
    queries: &mut NamedQueries,
    last_query: Option<&str>,
    pending_redirect: &mut Option<(OutputFormat, PathBuf)>,
    row_cap: usize,
    data: &mut dyn Write,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> io::Result<MetaFlow> {
    match cmd {
        MetaCommand::Quit => return Ok(MetaFlow::Quit),
        MetaCommand::Help => writeln!(out, "{}", help_text())?,
        MetaCommand::Docs => writeln!(out, "{}", docs_text())?,
        // Evaluate the expression server-side with the existing params in scope,
        // then store the result. A bad expression is reported (the Session
        // survives it; slice 14) without ending the loop.
        MetaCommand::SetParam { name, expr } => match runner.evaluate(&expr, params) {
            Ok(value) => {
                params.insert(name, value);
            }
            Err(e) => writeln!(err, "error: {e}")?,
        },
        MetaCommand::ListParams => writeln!(out, "{}", format_params(params))?,
        MetaCommand::ClearParams => params.clear(),
        MetaCommand::ListSettings => {
            writeln!(out, "{}", settings.list())?;
            // `readonly` is a Session guard, not a render setting, but it lists
            // alongside for completeness (issue 04).
            writeln!(out, "readonly = {}", on_off(runner.is_read_only()))?;
        }
        // `readonly` is special: it lives on the Session and can only be turned
        // off at connect time / via a profile (issue 04). Every other setting
        // goes through the Settings store, kept distinct from `:param`.
        MetaCommand::SetSetting { name, value } if name == "readonly" => match parse_on_off(&value) {
            Ok(true) => {
                runner.set_read_only(true);
                writeln!(out, "readonly = on")?;
            }
            Ok(false) => writeln!(
                err,
                "error: read-only can only be turned off at connect time \
                 (--read-only / a profile), not at runtime"
            )?,
            Err(message) => writeln!(err, "error: {message}")?,
        },
        MetaCommand::SetSetting { name, value } => match settings.set(&name, &value) {
            Ok(()) => writeln!(out, "{name} = {}", settings.get(&name).unwrap_or(value))?,
            Err(message) => writeln!(err, "error: {message}")?,
        },
        // Explicit transactions (ADR 0011): honoured identically here and in the
        // Workbench, surfacing errors without ending the session.
        MetaCommand::Begin => match runner.begin() {
            Ok(()) => writeln!(out, "transaction open")?,
            Err(e) => writeln!(err, "error: {e}")?,
        },
        MetaCommand::Commit => match runner.commit() {
            Ok(()) => writeln!(out, "transaction committed")?,
            Err(e) => writeln!(err, "error: {e}")?,
        },
        MetaCommand::Rollback => match runner.rollback() {
            Ok(()) => writeln!(out, "transaction rolled back")?,
            Err(e) => writeln!(err, "error: {e}")?,
        },
        // `:connect` swaps the whole Session (issue 07). An open transaction is
        // aborted by the swap — warn before it is silently lost (ADR 0011).
        MetaCommand::Connect(target) => {
            if runner.transaction_state() != TransactionState::Auto {
                writeln!(err, "note: the open transaction is aborted by :connect")?;
            }
            match runner.connect(&target) {
                Ok(label) => writeln!(out, "connected to {label}")?,
                Err(e) => writeln!(err, "error: {e}")?,
            }
        }
        // `:use` switches the active Database on the same Session (issue 08).
        MetaCommand::Use(database) => match runner.use_database(&database) {
            Ok(()) => writeln!(out, "using database {database}")?,
            Err(e) => writeln!(err, "error: {e}")?,
        },
        // `:sysinfo` runs the server-status queries through the normal render
        // path, honouring the current `display` mode (issue 09).
        MetaCommand::Sysinfo => {
            for query in SYSINFO_QUERIES {
                execute_query(runner, query, params, settings.display, row_cap, data, out, err)?;
            }
        }
        // `:source` feeds a file through the same line/query path (issue 10), so a
        // file may carry meta-commands too. A missing file is reported without
        // ending the session.
        MetaCommand::Source(path) => match std::fs::read_to_string(&path) {
            Ok(content) => {
                source_content(
                    &content, runner, settings, params, queries, row_cap, data, out, err,
                )?;
            }
            Err(e) => writeln!(err, "error: cannot read source file '{path}': {e}")?,
        },
        // `:watch` re-runs a query on a timer (issue 11); refused while a
        // transaction is open (a repeating timer holding a tx is a footgun).
        MetaCommand::Watch(args) => {
            if runner.transaction_state() == TransactionState::Auto {
                match parse_watch(&args, last_query) {
                    Ok(spec) => {
                        runner.watch(&spec.query, params, spec.interval, settings.display, data)?;
                    }
                    Err(message) => writeln!(err, "error: {message}")?,
                }
            } else {
                writeln!(err, "error: :watch is refused while a transaction is open")?;
            }
        }
        // `:o` arms the next query to redirect to a file (issue 12); one-shot.
        MetaCommand::Redirect(args) => match parse_redirect(&args) {
            Ok((format, path)) => {
                writeln!(out, "next query → {} ({format})", path.display())?;
                *pending_redirect = Some((format, path));
            }
            Err(message) => writeln!(err, "error: {message}")?,
        },
        // Named queries (issue 13): a tool-managed store of reusable templates.
        // `:save` keeps text only ($param placeholders survive); persistence
        // errors are reported without losing the in-memory save or the session.
        MetaCommand::Save { name, query } => {
            let text = query.or_else(|| last_query.map(str::to_string));
            match text {
                Some(text) => {
                    // The in-memory save always succeeds; a persist failure is a
                    // warning (the session keeps the save), not a lost command.
                    queries.set(name.clone(), text);
                    writeln!(out, "saved '{name}'")?;
                    if let Err(e) = queries.persist() {
                        writeln!(err, "warning: {e}")?;
                    }
                }
                None => writeln!(
                    err,
                    "error: no query to save; give one: ':save {name} <query>'"
                )?,
            }
        }
        MetaCommand::Saved => writeln!(out, "{}", queries.list())?,
        // `:load` recalls the template into the input for review/edit — it never
        // auto-runs (issue 13). An unknown name is reported and the loop carries on.
        MetaCommand::Load(name) => match queries.get(&name) {
            Some(text) => return Ok(MetaFlow::Recall(text.to_string())),
            None => writeln!(err, "error: no saved query named '{name}'")?,
        },
        MetaCommand::Forget(name) => {
            if queries.remove(&name) {
                writeln!(out, "forgot '{name}'")?;
                if let Err(e) = queries.persist() {
                    writeln!(err, "warning: {e}")?;
                }
            } else {
                writeln!(err, "error: no saved query named '{name}'")?;
            }
        }
        // A Workbench command has no meaning in the line REPL (it has no Buffers),
        // so it is reported unavailable rather than acted on (CONTEXT.md).
        MetaCommand::Close => {
            writeln!(err, "error: :close is a workbench command, not available here")?;
        }
        MetaCommand::Invalid(message) => writeln!(err, "error: {message}")?,
        MetaCommand::Unknown(cmd) => writeln!(err, "error: unknown command '{cmd}'")?,
    }
    Ok(MetaFlow::Handled)
}

/// Run a sourced file's contents through the same line/query path the loop uses
/// (issue 10): meta-commands are honoured, queries are assembled and run, each is
/// echoed before its result, and execution stops on the first query error. A
/// `:quit` inside a source ends sourcing, not the session.
#[allow(clippy::too_many_arguments)]
fn source_content(
    content: &str,
    runner: &mut dyn QueryRunner,
    settings: &mut Settings,
    params: &mut BTreeMap<String, Value>,
    queries: &mut NamedQueries,
    row_cap: usize,
    data: &mut dyn Write,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> io::Result<()> {
    let mut assembler = QueryAssembler::new();
    let mut pending_redirect: Option<(OutputFormat, PathBuf)> = None;
    for line in content.lines() {
        if !assembler.has_pending() {
            if let Some(cmd) = meta_command(line) {
                writeln!(out, "{line}")?;
                if matches!(cmd, MetaCommand::Quit) {
                    return Ok(());
                }
                dispatch_meta(
                    cmd,
                    runner,
                    settings,
                    params,
                    queries,
                    None,
                    &mut pending_redirect,
                    row_cap,
                    data,
                    out,
                    err,
                )?;
                continue;
            }
        }
        for query in assembler.push(&format!("{line}\n")) {
            writeln!(out, "{query}")?;
            if let Some((format, path)) = pending_redirect.take() {
                if let Err(e) = runner.run_to_file(&query, params, format, &path) {
                    writeln!(err, "error: {e}")?;
                    writeln!(err, "source stopped at the failed statement")?;
                    return Ok(());
                }
            } else if !execute_query(runner, &query, params, settings.display, row_cap, data, out, err)? {
                writeln!(err, "source stopped at the failed statement")?;
                return Ok(());
            }
        }
    }
    // A trailing statement with no terminating `;` still runs.
    if assembler.has_pending() {
        let query = assembler.pending().trim().to_string();
        if !query.is_empty() {
            writeln!(out, "{query}")?;
            if let Some((format, path)) = pending_redirect.take() {
                if let Err(e) = runner.run_to_file(&query, params, format, &path) {
                    writeln!(err, "error: {e}")?;
                }
            } else {
                execute_query(runner, &query, params, settings.display, row_cap, data, out, err)?;
            }
        }
    }
    Ok(())
}

pub fn run_loop(
    source: &mut dyn LineSource,
    runner: &mut dyn QueryRunner,
    queries: &mut NamedQueries,
    data: &mut dyn Write,
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
    // The most recently run query, so `:watch` with no query reuses it (issue 11).
    let mut last_query: Option<String> = None;
    // A one-shot `:o` redirect armed for the next query (issue 12).
    let mut pending_redirect: Option<(OutputFormat, PathBuf)> = None;
    // Text recalled by `:load` to pre-fill the next prompt (issue 13).
    let mut pending_initial: Option<String> = None;
    loop {
        let continued = assembler.has_pending();
        let initial = pending_initial.take();
        let text = match source.read(continued, initial.as_deref())? {
            Line::Eof => break,
            // Drop the half-typed statement and re-prompt from scratch.
            Line::Interrupted => {
                assembler = QueryAssembler::new();
                continue;
            }
            Line::Text(text) => text,
        };

        // A meta-command is only meaningful at the start of a statement; mid-
        // assembly the same text is ordinary query content. Handled commands and
        // `:quit` short-circuit; only ordinary query text falls through to run.
        if !continued {
            if let Some(cmd) = meta_command(&text) {
                match dispatch_meta(
                    cmd,
                    runner,
                    &mut settings,
                    &mut params,
                    queries,
                    last_query.as_deref(),
                    &mut pending_redirect,
                    config.row_cap,
                    data,
                    out,
                    err,
                )? {
                    MetaFlow::Quit => break,
                    MetaFlow::Handled => continue,
                    // Re-prompt with the recalled text pre-filled; never auto-run.
                    MetaFlow::Recall(text) => {
                        pending_initial = Some(text);
                        continue;
                    }
                }
            }
        }

        // The trailing newline lets a line comment close and separates physical
        // lines when assembling across reads.
        for query in assembler.push(&format!("{text}\n")) {
            // A `:o` redirect (issue 12) sends the next query's result to a file
            // (one-shot); otherwise it renders to the screen.
            if let Some((format, path)) = pending_redirect.take() {
                match runner.run_to_file(&query, &params, format, &path) {
                    Ok(rows) => {
                        writeln!(out, "wrote {rows} row(s) to {} ({format})", path.display())?;
                    }
                    Err(e) => writeln!(err, "error: {e}")?,
                }
            } else {
                execute_query(
                    runner, &query, &params, settings.display, config.row_cap, data, out, err,
                )?;
            }
            last_query = Some(query);
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
    fn close_is_parsed_as_a_workbench_command() {
        assert_eq!(meta_command(":close"), Some(MetaCommand::Close));
    }

    #[test]
    fn close_is_reported_unavailable_in_the_line_repl() {
        // The line REPL has no Buffers, so the Workbench command :close is
        // reported unavailable rather than acted on; the loop survives.
        let (_src, _runner, _out, err) = drive(
            vec![Line::Text(":close".into()), Line::Text("RETURN 1;".into())],
            vec![Ok(ok_result("t", 1))],
        );
        assert!(err.contains(":close"), "names the command: {err}");
        assert!(err.to_lowercase().contains("not available") || err.contains("workbench"), "{err}");
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
    fn transaction_commands_parse() {
        assert_eq!(meta_command(":begin"), Some(MetaCommand::Begin));
        assert_eq!(meta_command(":commit"), Some(MetaCommand::Commit));
        assert_eq!(meta_command("  :rollback "), Some(MetaCommand::Rollback));
    }

    #[test]
    fn begin_commit_rollback_drive_the_runner() {
        let (_src, runner, out, _err) = drive(
            vec![
                Line::Text(":begin".into()),
                Line::Text(":commit".into()),
                Line::Text(":begin".into()),
                Line::Text(":rollback".into()),
            ],
            vec![],
        );
        // Ends back in autocommit, with confirmations printed.
        assert_eq!(runner.transaction_state(), TransactionState::Auto);
        assert!(out.contains("transaction open"), "begin confirmed: {out}");
        assert!(out.contains("transaction committed"), "commit confirmed: {out}");
        assert!(out.contains("transaction rolled back"), "rollback confirmed: {out}");
    }

    #[test]
    fn connect_parses_its_target_and_rejects_an_empty_one() {
        assert_eq!(
            meta_command(":connect prod"),
            Some(MetaCommand::Connect("prod".to_string()))
        );
        assert_eq!(
            meta_command(":connect host:7688"),
            Some(MetaCommand::Connect("host:7688".to_string()))
        );
        assert!(matches!(meta_command(":connect"), Some(MetaCommand::Invalid(_))));
    }

    #[test]
    fn connect_drives_the_runner_and_warns_about_an_open_transaction() {
        let mut runner = ScriptedRunner::returning(vec![]);
        runner.begin().unwrap(); // open a tx so the swap warns
        let (_src, runner, out, err) = drive_with(
            vec![Line::Text(":connect prod".into())],
            runner,
        );
        assert_eq!(runner.connected, vec!["prod".to_string()]);
        assert!(out.contains("connected to prod"), "confirmation: {out}");
        assert!(err.contains("aborted by :connect"), "tx-abort warning: {err}");
        assert_eq!(runner.transaction_state(), TransactionState::Auto);
    }

    #[test]
    fn use_parses_and_drives_the_runner() {
        assert_eq!(
            meta_command(":use analytics"),
            Some(MetaCommand::Use("analytics".to_string()))
        );
        assert!(matches!(meta_command(":use"), Some(MetaCommand::Invalid(_))));
        let (_src, runner, out, _err) =
            drive(vec![Line::Text(":use analytics".into())], vec![]);
        assert_eq!(runner.used, vec!["analytics".to_string()]);
        assert!(out.contains("using database analytics"), "confirmation: {out}");
    }

    #[test]
    fn sysinfo_runs_the_status_queries_through_the_render_path() {
        let results: Vec<_> = SYSINFO_QUERIES.iter().map(|_| Ok(ok_result("t", 1))).collect();
        let (_src, runner, out, _err) = drive(vec![Line::Text(":sysinfo".into())], results);
        // Each status query ran, in order, rendering like a normal result.
        assert_eq!(
            runner.seen,
            SYSINFO_QUERIES.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
        assert_eq!(out.matches("row in set").count(), SYSINFO_QUERIES.len());
    }

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, content).expect("write temp source");
        path
    }

    #[test]
    fn source_runs_queries_in_order_honouring_meta_and_echoing() {
        let path = write_temp(
            "mg_source_ok.cypher",
            ":set display vertical\nRETURN 1;\nRETURN 2;\n",
        );
        let (_src, runner, out, _err) = drive(
            vec![Line::Text(format!(":source {}", path.display()))],
            vec![Ok(ok_result("a", 1)), Ok(ok_result("b", 1))],
        );
        // Both queries ran in order, under the display the sourced :set chose.
        assert_eq!(runner.seen, vec!["RETURN 1".to_string(), "RETURN 2".to_string()]);
        assert_eq!(runner.seen_display, vec![DisplayMode::Vertical, DisplayMode::Vertical]);
        // Each statement was echoed before its result.
        assert!(out.contains("RETURN 1") && out.contains("RETURN 2"), "echoed: {out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn source_stops_on_the_first_error() {
        let path = write_temp("mg_source_err.cypher", "RETURN 1;\nBAD;\nRETURN 3;\n");
        let boom = Error::Query(mgconsole_core::error::QueryError {
            code: "Memgraph.ClientError.MemgraphError.SyntaxError".to_string(),
            message: "bad".to_string(),
        });
        let (_src, runner, _out, err) = drive(
            vec![Line::Text(format!(":source {}", path.display()))],
            vec![Ok(ok_result("a", 1)), Err(boom)],
        );
        // Stopped after the failing statement; the third never ran.
        assert_eq!(runner.seen, vec!["RETURN 1".to_string(), "BAD".to_string()]);
        assert!(err.contains("source stopped"), "stop reported: {err}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn source_of_a_missing_file_reports_a_clear_error() {
        let (_src, runner, _out, err) = drive(
            vec![Line::Text(":source /no/such/mg/file.cypher".into())],
            vec![],
        );
        assert!(runner.seen.is_empty(), "nothing ran");
        assert!(err.contains("cannot read source file"), "clear error: {err}");
    }

    #[test]
    fn parse_watch_handles_interval_and_query_combinations() {
        // No args: reuse the last query at the default interval.
        let spec = parse_watch("", Some("RETURN 1")).expect("last query");
        assert_eq!(spec.interval, DEFAULT_WATCH_INTERVAL);
        assert_eq!(spec.query, "RETURN 1");
        // Interval only: reuse the last query.
        let spec = parse_watch("5s", Some("RETURN 1")).expect("interval only");
        assert_eq!(spec.interval, Duration::from_secs(5));
        assert_eq!(spec.query, "RETURN 1");
        // Interval + query.
        let spec = parse_watch("500ms MATCH (n) RETURN n", None).expect("interval + query");
        assert_eq!(spec.interval, Duration::from_millis(500));
        assert_eq!(spec.query, "MATCH (n) RETURN n");
        // Query only (no leading interval): default interval.
        let spec = parse_watch("MATCH (n) RETURN n", None).expect("query only");
        assert_eq!(spec.interval, DEFAULT_WATCH_INTERVAL);
        assert_eq!(spec.query, "MATCH (n) RETURN n");
        // No query and no prior one: an error.
        assert!(parse_watch("", None).is_err());
        assert!(parse_watch("2s", None).is_err());
    }

    #[test]
    fn watch_reuses_the_last_query_and_is_refused_in_a_transaction() {
        // After running a query, `:watch` with no args reuses it.
        let (_src, runner, _out, _err) = drive(
            vec![Line::Text("RETURN 1;".into()), Line::Text(":watch 3s".into())],
            vec![Ok(ok_result("t", 1))],
        );
        assert_eq!(runner.watched, vec![("RETURN 1".to_string(), Duration::from_secs(3))]);

        // Inside an open transaction, `:watch` is refused.
        let mut runner = ScriptedRunner::returning(vec![]);
        runner.begin().unwrap();
        let (_src, runner, _out, err) =
            drive_with(vec![Line::Text(":watch RETURN 1".into())], runner);
        assert!(runner.watched.is_empty(), "watch refused in a tx");
        assert!(err.contains("refused while a transaction is open"), "message: {err}");
    }

    #[test]
    fn parse_redirect_infers_or_takes_an_explicit_format() {
        // Explicit format + path.
        let (fmt, path) = parse_redirect("csv /tmp/out.txt").expect("explicit");
        assert_eq!(fmt, OutputFormat::Csv);
        assert_eq!(path, PathBuf::from("/tmp/out.txt"));
        // Inferred from the extension.
        let (fmt, path) = parse_redirect("/tmp/data.jsonl").expect("inferred");
        assert_eq!(fmt, OutputFormat::Jsonl);
        assert_eq!(path, PathBuf::from("/tmp/data.jsonl"));
        // Unknown extension with no explicit format is an error.
        assert!(parse_redirect("/tmp/data.weird").is_err());
        assert!(parse_redirect("").is_err());
    }

    #[test]
    fn redirect_sends_only_the_next_query_to_a_file() {
        let (_src, runner, out, _err) = drive(
            vec![
                Line::Text(":o csv /tmp/mg_o_test.csv".into()),
                Line::Text("RETURN 1;".into()),
                Line::Text("RETURN 2;".into()),
            ],
            vec![Ok(ok_result("a", 1))], // only the screen query consumes a result
        );
        // The first query went to the file; the second rendered to screen.
        assert_eq!(
            runner.redirected,
            vec![(
                "RETURN 1".to_string(),
                OutputFormat::Csv,
                PathBuf::from("/tmp/mg_o_test.csv")
            )]
        );
        assert_eq!(runner.seen, vec!["RETURN 2".to_string()], "only the 2nd hit the screen path");
        assert!(out.contains("next query →"), "redirect armed: {out}");
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
        /// The `initial` (recalled `:load` text) each read was offered, so a test
        /// can assert recall-to-input without a terminal (issue 13).
        initials: Vec<Option<String>>,
    }

    impl ScriptedSource {
        fn of(lines: Vec<Line>) -> Self {
            Self {
                lines: lines.into(),
                continued_at: Vec::new(),
                initials: Vec::new(),
            }
        }
    }

    impl LineSource for ScriptedSource {
        fn read(&mut self, continued: bool, initial: Option<&str>) -> io::Result<Line> {
            self.continued_at.push(continued);
            self.initials.push(initial.map(str::to_string));
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
        read_only: bool,
        tx: TransactionState,
        connected: Vec<String>,
        used: Vec<String>,
        watched: Vec<(String, Duration)>,
        redirected: Vec<(String, OutputFormat, PathBuf)>,
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
                read_only: false,
                tx: TransactionState::Auto,
                connected: Vec::new(),
                used: Vec::new(),
                watched: Vec::new(),
                redirected: Vec::new(),
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

        fn set_read_only(&mut self, on: bool) {
            self.read_only = on;
        }

        fn is_read_only(&self) -> bool {
            self.read_only
        }

        fn begin(&mut self) -> Result<(), Error> {
            self.tx = TransactionState::Open;
            Ok(())
        }

        fn commit(&mut self) -> Result<(), Error> {
            self.tx = TransactionState::Auto;
            Ok(())
        }

        fn rollback(&mut self) -> Result<(), Error> {
            self.tx = TransactionState::Auto;
            Ok(())
        }

        fn transaction_state(&self) -> TransactionState {
            self.tx
        }

        fn connect(&mut self, target: &str) -> Result<String, Error> {
            self.connected.push(target.to_string());
            self.tx = TransactionState::Auto;
            Ok(target.to_string())
        }

        fn use_database(&mut self, database: &str) -> Result<(), Error> {
            self.used.push(database.to_string());
            Ok(())
        }

        fn watch(
            &mut self,
            query: &str,
            _params: &BTreeMap<String, Value>,
            interval: Duration,
            _display: DisplayMode,
            _out: &mut dyn Write,
        ) -> io::Result<()> {
            // Record the request rather than running a real timer loop, so the
            // scripted loop never blocks on real time/stdin.
            self.watched.push((query.to_string(), interval));
            Ok(())
        }

        fn run_to_file(
            &mut self,
            query: &str,
            _params: &BTreeMap<String, Value>,
            format: OutputFormat,
            path: &std::path::Path,
        ) -> Result<usize, Error> {
            self.redirected
                .push((query.to_string(), format, path.to_path_buf()));
            Ok(0)
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
        runner: ScriptedRunner,
    ) -> (ScriptedSource, ScriptedRunner, String, String) {
        let (source, runner, _queries, out, err) =
            drive_with_queries(lines, runner, NamedQueries::in_memory());
        (source, runner, out, err)
    }

    /// Like [`drive_with`] but threads a [`NamedQueries`] store in and back out, so
    /// the issue-13 tests can pre-seed saved queries and assert the store after the
    /// run.
    fn drive_with_queries(
        lines: Vec<Line>,
        mut runner: ScriptedRunner,
        mut queries: NamedQueries,
    ) -> (ScriptedSource, ScriptedRunner, NamedQueries, String, String) {
        let mut source = ScriptedSource::of(lines);
        // The data sink (result rows) and the chrome sink (summaries, echoes,
        // confirmations) are separate in production (issue 19); the tests combine
        // them into one `out` string so the existing assertions read both.
        let mut data = Vec::new();
        let mut chrome = Vec::new();
        let mut err = Vec::new();
        run_loop(
            &mut source,
            &mut runner,
            &mut queries,
            &mut data,
            &mut chrome,
            &mut err,
            &ReplConfig {
                row_cap: 1000,
                settings: Settings::default(),
            },
        )
        .expect("loop runs to EOF");
        let out = String::from_utf8(data).unwrap() + &String::from_utf8(chrome).unwrap();
        (
            source,
            runner,
            queries,
            out,
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn result_data_goes_to_the_data_sink_and_chrome_to_the_chrome_sink() {
        // Stream discipline (ADR 0014, issue 19): the result rows land in the data
        // sink (stdout); the summary is chrome and lands in the chrome sink.
        let mut source = ScriptedSource::of(vec![Line::Text("RETURN 1;".into())]);
        let mut runner = ScriptedRunner::returning(vec![Ok(ok_result("<rows>", 1))]);
        let mut queries = NamedQueries::in_memory();
        let mut data = Vec::new();
        let mut chrome = Vec::new();
        let mut err = Vec::new();
        run_loop(
            &mut source,
            &mut runner,
            &mut queries,
            &mut data,
            &mut chrome,
            &mut err,
            &ReplConfig {
                row_cap: 1000,
                settings: Settings::default(),
            },
        )
        .expect("loop runs");
        let data = String::from_utf8(data).unwrap();
        let chrome = String::from_utf8(chrome).unwrap();
        assert!(data.contains("<rows>"), "result data on the data sink: {data:?}");
        assert!(!data.contains("row in set"), "no chrome on the data sink: {data:?}");
        assert!(chrome.contains("1 row in set"), "summary on the chrome sink: {chrome:?}");
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
    fn set_readonly_on_turns_the_session_read_only() {
        let (_src, runner, out, _err) = drive(
            vec![Line::Text(":set readonly on".into())],
            vec![],
        );
        assert!(runner.is_read_only(), "the runner was switched read-only");
        assert!(out.contains("readonly = on"), "confirmed: {out}");
    }

    #[test]
    fn set_readonly_off_at_runtime_is_refused() {
        // Start read-only, then try to clear it at runtime.
        let mut runner = ScriptedRunner::returning(vec![]);
        runner.set_read_only(true);
        let (_src, runner, _out, err) = drive_with(
            vec![Line::Text(":set readonly off".into())],
            runner,
        );
        assert!(runner.is_read_only(), "still read-only — runtime off is refused");
        assert!(err.contains("connect time"), "explains the asymmetry: {err}");
    }

    #[test]
    fn bare_set_lists_readonly_state() {
        let (_src, _runner, out, _err) = drive(vec![Line::Text(":set".into())], vec![]);
        assert!(out.contains("readonly = off"), "listing includes readonly: {out}");
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

    // --- :save / :saved / :load / :forget — Named queries (issue 13) ---------

    #[test]
    fn save_parses_a_name_and_optional_verbatim_query() {
        assert_eq!(
            meta_command(":save recent MATCH (n) RETURN n"),
            Some(MetaCommand::Save {
                name: "recent".to_string(),
                query: Some("MATCH (n) RETURN n".to_string()),
            })
        );
        // No query word: the last query is saved (a None the dispatcher fills).
        assert_eq!(
            meta_command(":save recent"),
            Some(MetaCommand::Save {
                name: "recent".to_string(),
                query: None,
            })
        );
        // A bare :save with no name is a misuse, not a save.
        assert!(matches!(meta_command(":save"), Some(MetaCommand::Invalid(_))));
    }

    #[test]
    fn saved_load_and_forget_parse() {
        assert_eq!(meta_command(":saved"), Some(MetaCommand::Saved));
        assert_eq!(
            meta_command(":load recent"),
            Some(MetaCommand::Load("recent".to_string()))
        );
        assert_eq!(
            meta_command(":forget recent"),
            Some(MetaCommand::Forget("recent".to_string()))
        );
        // load/forget need a name.
        assert!(matches!(meta_command(":load"), Some(MetaCommand::Invalid(_))));
        assert!(matches!(meta_command(":forget"), Some(MetaCommand::Invalid(_))));
    }

    #[test]
    fn save_with_a_query_stores_a_template_and_persists() {
        let (_src, _runner, queries, out, _err) = drive_with_queries(
            vec![Line::Text(":save recent MATCH (n) RETURN $limit".into())],
            ScriptedRunner::returning(vec![]),
            // A real path so persist() succeeds; the round-trip is asserted via the
            // returned store.
            NamedQueries::in_memory(),
        );
        // The $param placeholder is kept verbatim — a template, not a frozen value.
        assert_eq!(queries.get("recent"), Some("MATCH (n) RETURN $limit"));
        assert!(out.contains("saved 'recent'"));
    }

    #[test]
    fn save_with_no_query_saves_the_last_query() {
        let (_src, _runner, queries, _out, _err) = drive_with_queries(
            vec![
                Line::Text("RETURN 1;".into()),
                Line::Text(":save one".into()),
            ],
            ScriptedRunner::returning(vec![Ok(ok_result("t", 1))]),
            NamedQueries::in_memory(),
        );
        assert_eq!(queries.get("one"), Some("RETURN 1"));
    }

    #[test]
    fn save_with_no_query_and_no_last_query_is_reported() {
        let (_src, _runner, queries, _out, err) = drive_with_queries(
            vec![Line::Text(":save one".into())],
            ScriptedRunner::returning(vec![]),
            NamedQueries::in_memory(),
        );
        assert!(err.contains("no query to save"), "reported: {err}");
        assert!(queries.is_empty());
    }

    #[test]
    fn saved_lists_the_store() {
        let mut seed = NamedQueries::in_memory();
        seed.set("a".to_string(), "RETURN 1".to_string());
        seed.set("b".to_string(), "RETURN 2".to_string());
        let (_src, _runner, _queries, out, _err) = drive_with_queries(
            vec![Line::Text(":saved".into())],
            ScriptedRunner::returning(vec![]),
            seed,
        );
        assert!(out.contains("a\nb"), "lists saved names: {out}");
    }

    #[test]
    fn load_recalls_the_template_into_the_input_and_does_not_run_it() {
        let mut seed = NamedQueries::in_memory();
        seed.set("recent".to_string(), "MATCH (n) RETURN $limit".to_string());
        let (src, runner, _queries, _out, _err) = drive_with_queries(
            vec![Line::Text(":load recent".into())],
            ScriptedRunner::returning(vec![]),
            seed,
        );
        // The next read was offered the template as initial text — recall to input.
        assert!(
            src.initials.contains(&Some("MATCH (n) RETURN $limit".to_string())),
            "recalled to input: {:?}",
            src.initials
        );
        // It was not auto-run.
        assert!(runner.seen.is_empty(), "load never runs the query");
    }

    #[test]
    fn a_loaded_template_resolves_params_at_run_time_not_at_save_time() {
        // Save references $limit, set $limit, then a recalled-and-submitted template
        // runs with the *current* param value bound — templates, not frozen values.
        let mut seed = NamedQueries::in_memory();
        seed.set("recent".to_string(), "RETURN $limit;".to_string());
        let runner = ScriptedRunner::returning(vec![Ok(ok_result("t", 1))])
            .evaluating(vec![Ok(Value::Integer(10))]);
        let (_src, runner, _queries, _out, _err) = drive_with_queries(
            vec![
                Line::Text(":param limit 10".into()),
                // Recall loads to input; the next line is the user submitting it.
                Line::Text(":load recent".into()),
                Line::Text("RETURN $limit;".into()),
            ],
            runner,
            seed,
        );
        assert_eq!(runner.seen, vec!["RETURN $limit".to_string()]);
        assert_eq!(
            runner.seen_params,
            vec![BTreeMap::from([("limit".to_string(), Value::Integer(10))])]
        );
    }

    #[test]
    fn load_of_an_unknown_name_is_reported_and_the_loop_survives() {
        let (_src, _runner, _queries, _out, err) = drive_with_queries(
            vec![Line::Text(":load nope".into())],
            ScriptedRunner::returning(vec![]),
            NamedQueries::in_memory(),
        );
        assert!(err.contains("no saved query named 'nope'"), "reported: {err}");
    }

    #[test]
    fn forget_removes_a_saved_query_and_an_unknown_name_is_reported() {
        let mut seed = NamedQueries::in_memory();
        seed.set("a".to_string(), "RETURN 1".to_string());
        let (_src, _runner, queries, out, err) = drive_with_queries(
            vec![
                Line::Text(":forget a".into()),
                Line::Text(":forget gone".into()),
            ],
            ScriptedRunner::returning(vec![]),
            seed,
        );
        assert!(queries.get("a").is_none(), "removed");
        assert!(out.contains("forgot 'a'"));
        assert!(err.contains("no saved query named 'gone'"), "reported: {err}");
    }
}
