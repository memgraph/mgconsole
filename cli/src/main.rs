//! The interactive REPL Frontend (slice 16).
//!
//! Connects a Core Session, then drives [`mgconsole::repl::run_loop`] — the
//! testable loop seam — with the real IO it abstracts: rustyline for input on a
//! terminal, a plain line reader when stdin is piped, and a [`SessionRunner`]
//! that runs each query and renders it as tabular. The async Core is driven via
//! `block_on` at this boundary (ADR 0002). Auth/TLS/flag parsing (slices 09–11)
//! are reused unchanged; the loop itself is covered by unit tests in `repl`.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, IsTerminal};
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use rustyline::completion::Completer as RustylineCompleter;
use rustyline::highlight::{CmdKind, Highlighter as RustylineHighlighter};
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Editor, Helper, Hinter};

use mgconsole::frontend::{select_frontend, Frontend};
use mgconsole::history::{self, HistoryFile};
use mgconsole::config;
use mgconsole::repl::{self, Line, LineSource, QueryRunner, Rendered, ReplConfig};
use mgconsole::settings::{FileSettings, Settings};
use mgconsole::syntax::{self, Completer};
#[cfg(feature = "tui")]
use mgconsole::workbench;
use mgconsole::{no_color_active, resolve_password, Cli, ImportMode, OutputFormat};
use mgconsole_core::format::CsvOptions;
use mgconsole_core::{
    render_records, run_parallel_ordered, run_parser, run_serial, ConnectOptions, Credentials,
    DisplayMode, Endpoint, Error, Header, ImportFormat, ParserReport, QueryAssembler,
    ReconnectNotice, RenderOptions, Session, TableOptions, Value, Workers, DEFAULT_ROW_CAP,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if let Err(message) = cli.validate() {
        eprintln!("error: {message}");
        std::process::exit(2);
    }

    // Parser mode validates a piped import file with the clause scanner and never
    // touches the database (PRD: validate before touching Memgraph), so handle it
    // before resolving auth or connecting. It is meaningful only non-interactively.
    if !io::stdin().is_terminal() && cli.import_mode == ImportMode::Parser {
        let queries = read_queries(io::stdin().lock())?;
        let report = run_parser(queries);
        print!("{}", format_parser_report(&report, cli.parser_stats));
        return Ok(());
    }

    // Resolve auth before touching the network: a username with no password gets
    // a hidden prompt; an empty username stays anonymous.
    let password = match resolve_password(&cli.username, &cli.password, || {
        rpassword::prompt_password("Password: ")
    }) {
        Ok(password) => password,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };
    let options = ConnectOptions {
        credentials: (!cli.username.is_empty()).then(|| Credentials {
            username: cli.username.clone(),
            password,
        }),
        use_tls: cli.use_ssl,
    };
    let endpoint = Endpoint::new(cli.host.clone(), cli.port);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Batched-parallel import runs over N worker Sessions, not the single Session
    // the REPL/serial path uses, so establish them and run the executor here
    // (slice 30). Vertices-first ordering keeps a mixed node/edge dataset correct
    // (slice 31). Non-interactive only; exits non-zero if any query fails.
    if !io::stdin().is_terminal() && cli.import_mode == ImportMode::BatchedParallel {
        let queries = read_queries(io::stdin().lock())?;
        let report = runtime.block_on(async {
            let workers = Workers::connect(&endpoint, &options, cli.workers_number).await?;
            Ok::<_, Error>(run_parallel_ordered(workers, queries, cli.batch_size as usize).await)
        })?;
        for failure in &report.failures {
            eprintln!("error: {}: {}", failure.query, failure.error);
        }
        if !report.is_success() {
            std::process::exit(1);
        }
        return Ok(());
    }

    let mut session = runtime.block_on(Session::connect_with(&endpoint, &options))?;
    // Surface fatal-error reconnect attempts to the user (slice 14 hook).
    session.on_reconnect(|notice: ReconnectNotice| {
        eprintln!(
            "connection lost; reconnecting (attempt {}/{})...",
            notice.attempt, notice.max
        );
    });

    // `--fit-to-screen` resolves to the live terminal width here in the Frontend
    // (the Core renderer stays width-agnostic).
    let fit_width = cli
        .fit_to_screen
        .then(terminal_size::terminal_size)
        .flatten()
        .map(|(width, _)| width.0);
    let table_options = TableOptions { fit_width };

    // Resolve the console Settings: built-in default < config file < CLI flag
    // (ADR 0012; the runtime `:set` layer lives in each Frontend). A malformed
    // config is reported but never fatal — the console starts on defaults.
    let settings = Settings::resolve(&load_config(), cli.display);

    let mut out = io::stdout();
    let mut err = io::stderr();

    // An interactive terminal gets one of the two interactive Frontends, chosen by
    // the pure resolver (ADR 0010): the full-screen workbench by default, the line
    // REPL when `--plain` is set or the terminal cannot host the workbench. A pipe
    // gets the non-interactive serial import path (slice 25), which streams output
    // in the selected format and exits non-zero if any query fails.
    if io::stdin().is_terminal() {
        run_interactive(
            &cli,
            session,
            &runtime,
            table_options,
            settings,
            &mut out,
            &mut err,
        )?;
    } else {
        let queries = read_queries(io::stdin().lock())?;
        let format = import_format(&cli, table_options);
        let report = runtime.block_on(run_serial(&mut session, queries, &mut out, &format));
        // The Core stays diagnostic-free (ADR 0002); the Frontend reports each
        // failed query and maps any failure to a non-zero exit code.
        for failure in &report.failures {
            eprintln!("error: {}: {}", failure.query, failure.error);
        }
        if !report.is_success() {
            std::process::exit(1);
        }
    }

    Ok(())
}

/// Drive the chosen interactive Frontend over a connected Session (ADR 0010):
/// the workbench by default, the line REPL when `--plain` is set or the terminal
/// cannot host the workbench. Colour is resolved here at the IO boundary and
/// governs colour *within* whichever Frontend runs, independently of the choice.
fn run_interactive(
    cli: &Cli,
    session: Session,
    runtime: &tokio::runtime::Runtime,
    table_options: TableOptions,
    settings: Settings,
    out: &mut io::Stdout,
    err: &mut io::Stderr,
) -> Result<(), Box<dyn std::error::Error>> {
    let colorize = cli.color.resolve(
        no_color_active(std::env::var("NO_COLOR").ok().as_deref()),
        true,
    );
    // `supports_tui` folds in the compile-time feature: a `--no-default-features`
    // build has no workbench and always falls back to the REPL.
    let supports_tui = {
        #[cfg(feature = "tui")]
        {
            terminal_supports_tui()
        }
        #[cfg(not(feature = "tui"))]
        {
            false
        }
    };
    // History resolution is shared by both interactive Frontends (slice 17).
    let history = open_history(cli);
    match select_frontend(cli.plain, true, supports_tui) {
        Frontend::Workbench => {
            #[cfg(feature = "tui")]
            {
                let config = workbench::WorkbenchConfig {
                    verbose: cli.verbose_execution_info,
                    settings,
                    ..workbench::WorkbenchConfig::default()
                };
                runtime.block_on(workbench::run(session, config, colorize, history))?;
            }
            #[cfg(not(feature = "tui"))]
            unreachable!("the resolver cannot pick the workbench without the tui feature");
        }
        Frontend::Repl => {
            let mut runner = SessionRunner {
                runtime,
                session,
                table_options,
                row_cap: DEFAULT_ROW_CAP,
            };
            let config = ReplConfig {
                row_cap: DEFAULT_ROW_CAP,
                settings,
            };
            let mut source = RustylineSource::new(history, colorize)?;
            repl::run_loop(&mut source, &mut runner, out, err, &config)?;
        }
        Frontend::Piped => {
            unreachable!("the piped path is handled by the non-terminal branch")
        }
    }
    Ok(())
}

/// Whether the current terminal can host the full-screen workbench: stdout is a
/// terminal and `TERM` is not the capability-less `dumb` terminal. The resolver
/// falls back to the REPL otherwise (ADR 0010's auto-fallback).
#[cfg(feature = "tui")]
fn terminal_supports_tui() -> bool {
    io::stdout().is_terminal() && std::env::var("TERM").map_or(true, |term| term != "dumb")
}

/// Read a piped query stream into complete queries, in input order. Physical
/// lines are assembled across `;` boundaries by the Core's [`QueryAssembler`]
/// (multiline queries, several per line, `;` inside strings/comments). A trailing
/// query missing its terminating `;` is still run rather than silently dropped.
fn read_queries(mut reader: impl BufRead) -> io::Result<Vec<String>> {
    let mut assembler = QueryAssembler::new();
    let mut queries = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        queries.extend(assembler.push(&line));
    }
    if assembler.has_pending() {
        queries.push(assembler.pending().trim().to_string());
    }
    Ok(queries)
}

/// Render a parser-mode report. Always lists each query's detected clauses (the
/// scanner's per-query report); with `stats` on, appends an aggregate clause
/// breakdown (the `--parser-stats` flag). Clause names use their enum spelling.
fn format_parser_report(report: &ParserReport, stats: bool) -> String {
    let mut out = String::new();
    for (i, parsed) in report.queries.iter().enumerate() {
        let clauses = if parsed.clauses.is_empty() {
            "(no ordering clauses)".to_string()
        } else {
            parsed
                .clauses
                .iter()
                .map(|c| format!("{c:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        writeln!(out, "{}: {clauses}", i + 1).unwrap();
    }
    writeln!(
        out,
        "Parsed {} queries; nothing executed.",
        report.query_count()
    )
    .unwrap();
    if stats {
        out.push_str("Clause statistics:\n");
        for (clause, count) in &report.clause_counts {
            writeln!(out, "  {clause:?}: {count}").unwrap();
        }
    }
    out
}

/// Map the CLI output-format flag (plus its csv options and resolved table width)
/// onto the Core's render format for the serial import path.
fn import_format(cli: &Cli, table_options: TableOptions) -> ImportFormat {
    match cli.output_format {
        OutputFormat::Tabular => ImportFormat::Tabular(table_options),
        OutputFormat::Csv => ImportFormat::Csv(CsvOptions {
            delimiter: cli.csv_delimiter as u8,
            quote: b'"',
            escape: cli.csv_escapechar.unwrap_or('\\') as u8,
            double_quote: cli.csv_doublequote,
        }),
        OutputFormat::Jsonl => ImportFormat::Jsonl,
        OutputFormat::Cypherl => ImportFormat::Cypherl,
    }
}

/// Resolve and prepare the history file for an interactive session, honouring
/// the `--history`/`--no-history` flags and the `MGCONSOLE_HISTORY_PATH` env
/// override (slice 17). A history directory that cannot be created is reported
/// and history is disabled, so the REPL still runs.
fn open_history(cli: &Cli) -> Option<HistoryFile> {
    let env = std::env::var(history::HISTORY_ENV).ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = history::resolve_history_file(
        &cli.history,
        env.as_deref(),
        cli.no_history,
        home.as_deref(),
    )?;
    match history::prepare_history_dir(&path) {
        Ok(()) => Some(HistoryFile::new(path)),
        Err(message) => {
            eprintln!("warning: {message}; continuing without history");
            None
        }
    }
}

/// Load the config-file Setting overlay (issue 02): resolve the path from
/// `MGCONSOLE_CONFIG_PATH`/home, then read it. A missing file is the normal case
/// (empty overlay); a malformed file is reported and treated as empty so the
/// console still starts on defaults (ADR 0012).
fn load_config() -> FileSettings {
    let env = std::env::var(config::CONFIG_ENV).ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(path) = config::resolve_config_path(env.as_deref(), home.as_deref()) else {
        return FileSettings::default();
    };
    match config::load(&path) {
        Ok(overlay) => overlay,
        Err(message) => {
            eprintln!("warning: {message}; continuing on defaults");
            FileSettings::default()
        }
    }
}

/// rustyline helper for the REPL: multiline validation (slice 16), static
/// keyword/function completion, and optional Cypher highlighting (slice 18).
/// Hinting stays the no-op default.
#[derive(Helper, Hinter)]
struct MgHelper {
    completer: Completer,
    /// Whether to colour input; resolved from `--color`/`NO_COLOR` (ADR 0009).
    colorize: bool,
}

impl MgHelper {
    fn new(colorize: bool) -> Self {
        Self {
            completer: Completer::with_static_vocabulary(),
            colorize,
        }
    }
}

impl Validator for MgHelper {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        if repl::is_complete(ctx.input()) {
            Ok(ValidationResult::Valid(None))
        } else {
            Ok(ValidationResult::Incomplete)
        }
    }
}

impl RustylineCompleter for MgHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        let start = syntax::word_start(line, pos);
        Ok((start, self.completer.candidates(&line[start..pos])))
    }
}

impl RustylineHighlighter for MgHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if self.colorize {
            Cow::Owned(syntax::highlight(line))
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_char(&self, line: &str, _pos: usize, _kind: CmdKind) -> bool {
        // Re-highlight on every keystroke while colouring is on.
        self.colorize && !line.is_empty()
    }
}

/// Interactive input via rustyline, with its `Validator` assembling multiline
/// queries inside a single `readline` call. When a [`HistoryFile`] is present,
/// prior entries are loaded on start and each entered line is persisted.
struct RustylineSource {
    editor: Editor<MgHelper, rustyline::history::DefaultHistory>,
    history: Option<HistoryFile>,
}

impl RustylineSource {
    fn new(history: Option<HistoryFile>, colorize: bool) -> rustyline::Result<Self> {
        let mut editor = Editor::new()?;
        editor.set_helper(Some(MgHelper::new(colorize)));
        if let Some(history) = &history {
            history.load(editor.history_mut());
        }
        Ok(Self { editor, history })
    }
}

impl LineSource for RustylineSource {
    fn read(&mut self, continued: bool) -> io::Result<Line> {
        let prompt = if continued { "      -> " } else { "memgraph> " };
        match self.editor.readline(prompt) {
            Ok(line) => {
                if let Some(history) = &self.history {
                    history.record(self.editor.history_mut(), &line);
                }
                Ok(Line::Text(line))
            }
            Err(rustyline::error::ReadlineError::Interrupted) => Ok(Line::Interrupted),
            Err(rustyline::error::ReadlineError::Eof) => Ok(Line::Eof),
            Err(other) => Err(io::Error::other(other)),
        }
    }
}

/// The production [`QueryRunner`]: runs a query through the Session and renders
/// it as tabular, timing the round trip. Bounded memory — it buffers at most the
/// row cap (one extra row is pulled to detect overflow, then dropped via
/// `collect_capped`), and discards the remainder so the connection is reusable.
struct SessionRunner<'a> {
    runtime: &'a tokio::runtime::Runtime,
    session: Session,
    table_options: TableOptions,
    row_cap: usize,
}

impl QueryRunner for SessionRunner<'_> {
    fn run(
        &mut self,
        query: &str,
        params: &BTreeMap<String, Value>,
        display: DisplayMode,
    ) -> Result<Rendered, Error> {
        let runtime = self.runtime;
        let session = &mut self.session;
        let cap = self.row_cap;
        // The `auto` display mode needs the live terminal width to decide whether
        // a row fits; resolve it here at the IO boundary (the Core stays
        // width-agnostic), independently of the `--fit-to-screen` column fitting.
        let render_options = RenderOptions {
            mode: display,
            table: self.table_options.clone(),
            term_width: terminal_size::terminal_size().map(|(width, _)| width.0),
        };

        runtime.block_on(async move {
            let start = Instant::now();
            let mut result = session.run_with_params(query, params).await?;
            let header = Header::new(result.header());
            let (records, overflowed) = result.records().collect_capped(cap).await?;
            // Drop any rows beyond the cap so the connection is ready for reuse.
            result.records().discard().await?;
            let elapsed = start.elapsed();

            let rows: Vec<Vec<Value>> = records
                .into_iter()
                .map(mgconsole_core::Record::into_fields)
                .collect();
            // A write returns no columns; rendering an empty table is just a
            // degenerate box, so leave it out and let the summary speak.
            let table = if header.is_empty() {
                String::new()
            } else {
                render_records(&header, &rows, &render_options)
            };
            Ok(Rendered {
                table,
                row_count: rows.len(),
                overflowed,
                elapsed,
            })
        })
    }

    fn evaluate(&mut self, expr: &str, params: &BTreeMap<String, Value>) -> Result<Value, Error> {
        let runtime = self.runtime;
        let session = &mut self.session;

        runtime.block_on(async move {
            // Evaluate the expression server-side with the existing params in
            // scope; the single returned value becomes the stored parameter.
            let query = format!("RETURN {expr}");
            let mut result = session.run_with_params(&query, params).await?;
            let first = result.records().next().await?;
            // A multi-row/column expression is unusual for a param; keep only the
            // first field and drain the rest so the connection stays reusable.
            result.records().discard().await?;
            Ok(first
                .and_then(|record| record.into_fields().into_iter().next())
                .unwrap_or(Value::Null))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn queries_of(input: &str) -> Vec<String> {
        read_queries(Cursor::new(input)).expect("read")
    }

    #[test]
    fn reads_piped_queries_in_input_order() {
        assert_eq!(
            queries_of("RETURN 1;\nRETURN 2;\n"),
            vec!["RETURN 1".to_string(), "RETURN 2".to_string()]
        );
    }

    #[test]
    fn assembles_a_multiline_query_across_lines() {
        assert_eq!(
            queries_of("MATCH (n)\nRETURN n;\n"),
            vec!["MATCH (n)\nRETURN n".to_string()]
        );
    }

    #[test]
    fn splits_several_queries_sharing_a_line() {
        assert_eq!(
            queries_of("RETURN 1; RETURN 2;\n"),
            vec!["RETURN 1".to_string(), "RETURN 2".to_string()]
        );
    }

    #[test]
    fn a_trailing_unterminated_query_is_still_run() {
        // No final ';' — rather than drop it silently, run the trailing fragment.
        assert_eq!(queries_of("RETURN 1"), vec!["RETURN 1".to_string()]);
    }

    #[test]
    fn an_empty_stream_yields_no_queries() {
        assert!(queries_of("").is_empty());
        assert!(queries_of("\n  \n").is_empty());
    }

    fn cli_with(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("mgconsole").chain(args.iter().copied()))
            .expect("parse")
    }

    #[test]
    fn output_flag_maps_to_the_render_format() {
        assert!(matches!(
            import_format(
                &cli_with(&["--output-format", "jsonl"]),
                TableOptions::default()
            ),
            ImportFormat::Jsonl
        ));
        assert!(matches!(
            import_format(
                &cli_with(&["--output-format", "cypherl"]),
                TableOptions::default()
            ),
            ImportFormat::Cypherl
        ));
        assert!(matches!(
            import_format(&cli_with(&[]), TableOptions::default()),
            ImportFormat::Tabular(_)
        ));
    }

    #[test]
    fn csv_options_carry_the_csv_flags() {
        let format = import_format(
            &cli_with(&["--output-format", "csv", "--csv-delimiter", ";"]),
            TableOptions::default(),
        );
        match format {
            ImportFormat::Csv(opts) => {
                assert_eq!(opts.delimiter, b';');
                assert_eq!(opts.quote, b'"');
                assert!(opts.double_quote);
            }
            _ => panic!("expected csv format"),
        }
    }

    #[test]
    fn parser_report_lists_clauses_and_a_no_execution_summary() {
        let report = run_parser(vec!["CREATE (n)".to_string(), "RETURN 1".to_string()]);
        let text = format_parser_report(&report, false);
        assert!(text.contains("1: Create"), "per-query clauses: {text}");
        assert!(
            text.contains("2: (no ordering clauses)"),
            "empty case: {text}"
        );
        assert!(
            text.contains("Parsed 2 queries; nothing executed."),
            "{text}"
        );
        // Without the flag, no statistics block.
        assert!(!text.contains("Clause statistics"), "{text}");
    }

    #[test]
    fn parser_report_appends_statistics_when_requested() {
        let report = run_parser(vec!["CREATE (a)".to_string(), "CREATE (b)".to_string()]);
        let text = format_parser_report(&report, true);
        assert!(text.contains("Clause statistics:"), "{text}");
        assert!(text.contains("Create: 2"), "aggregate count: {text}");
    }
}
