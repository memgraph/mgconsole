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

use clap::parser::ValueSource;
use clap::{CommandFactory, FromArgMatches};
use rustyline::completion::Completer as RustylineCompleter;
use rustyline::highlight::{CmdKind, Highlighter as RustylineHighlighter};
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Editor, Helper, Hinter};

use mgconsole::frontend::{select_frontend, Frontend};
use mgconsole::history::{self, HistoryFile};
use mgconsole::config;
use mgconsole::queries;
use mgconsole::repl::{self, Line, LineSource, QueryRunner, Rendered, ReplConfig};
use mgconsole::settings::Settings;
use mgconsole::syntax::{self, Completer};
#[cfg(feature = "tui")]
use mgconsole::workbench;
use mgconsole::{
    no_color_active, resolve_connect_target, resolve_connection, resolve_password,
    resolve_sources, Cli, Connection, ExplicitFlags, ImportMode, OutputFormat, QuerySource,
};
use mgconsole_core::format::CsvOptions;
use mgconsole_core::{
    render_records, run_parallel_ordered, run_parser, run_serial, ConnectOptions, Credentials,
    DisplayMode, Endpoint, Error, Header, ImportFormat, ParserReport, QueryAssembler,
    ReconnectNotice, RenderOptions, Session, TableOptions, TransactionState, Value, Workers,
    DEFAULT_ROW_CAP,
};

// The startup flow is one linear sequence — parse, resolve sources/auth, then
// dispatch to the parser/parallel/serial/interactive paths — that reads better
// whole than split across helpers that each take the same dozen locals.
#[allow(clippy::too_many_lines)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse via ArgMatches (not the bare derive) so we can see which connection
    // flags were given explicitly — a profile fills only the unset ones (issue 03).
    let matches = Cli::command().get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => e.exit(),
    };
    if let Err(message) = cli.validate() {
        eprintln!("error: {message}");
        std::process::exit(2);
    }
    let explicit = explicit_flags(&matches);

    // Resolve the non-interactive query sources: `-c` runs one-shot (issue 21),
    // the `run` subcommand names files (`-` = stdin, issue 20), and a bare
    // invocation reads piped stdin (the sugar for `run -`); a TTY stdin with no
    // `-c`/subcommand is interactive (`None`).
    let sources = resolve_sources(
        cli.one_shot.as_deref(),
        cli.command.as_ref(),
        io::stdin().is_terminal(),
    );

    // Parser mode validates the sources with the clause scanner and never touches
    // the database (PRD: validate before touching Memgraph), so handle it before
    // resolving auth or connecting. It is meaningful only non-interactively.
    if let Some(sources) = &sources {
        if cli.import_mode == ImportMode::Parser {
            let queries = read_all_sources(sources);
            let report = run_parser(queries);
            print!("{}", format_parser_report(&report, cli.parser_stats));
            return Ok(());
        }
    }

    // Resolve config, the selected profile, the effective connection, and the
    // settings (issue 03). An unknown --profile fails fast inside the helper.
    let config = load_config();
    let (connection, settings) = resolve_profile_connection(&cli, explicit, &config);

    // Resolve auth before touching the network: a username with no password gets
    // a hidden prompt; an empty username stays anonymous.
    let password = match resolve_password(&connection.username, &connection.password, || {
        rpassword::prompt_password("Password: ")
    }) {
        Ok(password) => password,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };
    let options = ConnectOptions {
        credentials: (!connection.username.is_empty()).then(|| Credentials {
            username: connection.username.clone(),
            password,
        }),
        use_tls: connection.use_ssl,
        read_only: connection.readonly,
    };
    let endpoint = Endpoint::new(connection.host.clone(), connection.port);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Batched-parallel import runs over N worker Sessions, not the single Session
    // the REPL/serial path uses, so establish them and run the executor here
    // (slice 30). Vertices-first ordering keeps a mixed node/edge dataset correct
    // (slice 31). Non-interactive only; exits non-zero if any query fails.
    if let Some(sources) = &sources {
        if cli.import_mode == ImportMode::BatchedParallel {
            let queries = read_all_sources(sources);
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

    let mut out = io::stdout();
    let mut err = io::stderr();

    // With no non-interactive sources, an interactive terminal gets one of the two
    // Frontends, chosen by the pure resolver (ADR 0010): the full-screen workbench
    // by default, the line REPL when `--plain` is set or the terminal cannot host
    // it. Otherwise the non-interactive serial path runs the sources (`run` files
    // or piped stdin, issue 20), streaming output in the resolved format and
    // exiting non-zero if any query fails.
    match &sources {
        None => {
            run_interactive(
                &cli,
                session,
                &runtime,
                table_options,
                settings,
                cli.profile.clone(),
                config,
                options,
                &mut out,
                &mut err,
            )?;
        }
        Some(sources) => {
            let queries = read_all_sources(sources);
            // The default format is a function of stdout (issue 19): jsonl into a
            // pipe so `… | mgconsole | jq` composes, table at a terminal. An
            // explicit `--output-format` still wins. Result data goes to stdout;
            // the per-query failure reporting goes to stderr (ADR 0014).
            let resolved = OutputFormat::resolve(cli.output_format, out.is_terminal());
            let format = import_format(&cli, resolved, table_options);
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
    }

    Ok(())
}

/// Drive the chosen interactive Frontend over a connected Session (ADR 0010):
/// the workbench by default, the line REPL when `--plain` is set or the terminal
/// cannot host the workbench. Colour is resolved here at the IO boundary and
/// governs colour *within* whichever Frontend runs, independently of the choice.
#[allow(clippy::too_many_arguments)]
fn run_interactive(
    cli: &Cli,
    session: Session,
    runtime: &tokio::runtime::Runtime,
    table_options: TableOptions,
    settings: Settings,
    profile: Option<String>,
    config: mgconsole::config::Config,
    options: ConnectOptions,
    out: &mut io::Stdout,
    err: &mut io::Stderr,
) -> Result<(), Box<dyn std::error::Error>> {
    // First interactive run with no config and no override: scaffold a commented
    // example so the format is discoverable (issue 22). Inert until edited.
    scaffold_config_on_first_run();

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
    // The Session may already be read-only (connect-time flag/profile); seed the
    // marker state from it. The REPL shares a flag with its prompt so a runtime
    // `:set readonly on` updates the prompt too.
    let read_only = session.is_read_only();
    match select_frontend(cli.plain, true, supports_tui) {
        Frontend::Workbench => {
            #[cfg(feature = "tui")]
            {
                // Resolve the Workbench theme + keybindings (issue 14): the built-in
                // palette named by the `theme` Setting with the `[theme]` overrides,
                // and the `[keys]` rebindings over the defaults. Warnings are
                // reported to stderr; one bad line never blocks the workbench.
                let (palette, theme_warnings) =
                    mgconsole::theme::resolve_palette(&settings.theme, &config.theme);
                let (keys, key_warnings) = mgconsole::theme::resolve_keys(&config.keys);
                for warning in theme_warnings.iter().chain(&key_warnings) {
                    eprintln!("warning: {warning}");
                }
                let wb_config = workbench::WorkbenchConfig {
                    verbose: cli.verbose_execution_info,
                    settings,
                    profile,
                    read_only,
                    endpoint: session.endpoint().to_string(),
                    queries: load_queries(),
                    palette,
                    keys,
                    theme_overrides: config.theme.clone(),
                    connect: workbench::state::ConnectContext { config, options },
                    ..workbench::WorkbenchConfig::default()
                };
                runtime.block_on(workbench::run(session, wb_config, colorize, history))?;
            }
            #[cfg(not(feature = "tui"))]
            unreachable!("the resolver cannot pick the workbench without the tui feature");
        }
        Frontend::Repl => {
            let prompt = std::sync::Arc::new(std::sync::Mutex::new(PromptInfo {
                endpoint: session.endpoint().to_string(),
                profile: profile.clone(),
                read_only,
                tx: session.transaction_state(),
                database: None,
            }));
            // The REPL renders result data in the TTY-aware resolved format (issue
            // 19): tabular at a terminal, jsonl when stdout is redirected, so
            // `mgconsole > out` writes faithful jsonl to the file. An explicit
            // `--output-format` still wins.
            let stdout_is_tty = out.is_terminal();
            let resolved = OutputFormat::resolve(cli.output_format, stdout_is_tty);
            let mut runner = SessionRunner {
                runtime,
                session,
                table_options,
                row_cap: DEFAULT_ROW_CAP,
                output_format: resolved,
                config,
                options,
                prompt: prompt.clone(),
            };
            let repl_config = ReplConfig {
                row_cap: DEFAULT_ROW_CAP,
                settings,
            };
            let mut source = RustylineSource::new(history, colorize, prompt)?;
            let mut queries = load_queries();
            // Stream discipline (ADR 0014): result data → stdout (`out`); all chrome
            // — summaries, echoes, confirmations — → stderr, where it stays visible
            // on the terminal even when stdout is redirected to a file.
            let mut chrome = io::stderr();
            repl::run_loop(
                &mut source,
                &mut runner,
                &mut queries,
                out,
                &mut chrome,
                err,
                &repl_config,
            )?;
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

/// Read every query source in order into one list of queries (issue 20). A `-`
/// source reads stdin; a file source opens and reads the named file. An unreadable
/// or missing file is a clear, path-naming error that exits non-zero *before*
/// running anything downstream — so later files are never run (acceptance: a
/// missing file stops the run). The same `QueryAssembler` discipline as the piped
/// path splits each source into queries.
fn read_all_sources(sources: &[QuerySource]) -> Vec<String> {
    let mut queries = Vec::new();
    for source in sources {
        let result = match source {
            QuerySource::Stdin => read_queries(io::stdin().lock()),
            // The inline `-c` string is split into queries by the same assembler
            // the piped path uses, so `-c "Q1; Q2"` runs both in order (issue 21).
            QuerySource::Inline(query) => read_queries(io::Cursor::new(query.as_bytes())),
            QuerySource::File(path) => match std::fs::File::open(path) {
                Ok(file) => read_queries(io::BufReader::new(file)),
                Err(e) => {
                    eprintln!("error: cannot open {}: {e}", path.display());
                    std::process::exit(1);
                }
            },
        };
        match result {
            Ok(read) => queries.extend(read),
            Err(e) => {
                let what = match source {
                    QuerySource::Stdin => "stdin".to_string(),
                    QuerySource::Inline(_) => "the -c query".to_string(),
                    QuerySource::File(path) => path.display().to_string(),
                };
                eprintln!("error: reading {what}: {e}");
                std::process::exit(1);
            }
        }
    }
    queries
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

/// Map the resolved output format (plus its csv options and table width) onto the
/// Core's render format for the serial import path. The format is resolved by
/// [`OutputFormat::resolve`] from the `--output-format` flag and stdout's TTY-ness
/// (issue 19) before reaching here.
fn import_format(cli: &Cli, format: OutputFormat, table_options: TableOptions) -> ImportFormat {
    match format {
        OutputFormat::Table => ImportFormat::Tabular(table_options),
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

/// Render result rows in a streaming output format (issue 19), for a redirected
/// REPL or an explicit `--output-format`. Reuses the Core's row writers, returning
/// the text without a trailing newline (the loop adds line separation). `Table` is
/// handled by the tabular renderer at the call site and never reaches here.
fn render_rows_as(format: OutputFormat, header: &Header, rows: &[Vec<Value>]) -> String {
    use mgconsole_core::format::{CsvWriter, CypherlWriter, JsonlWriter, RowWriter};
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer: Box<dyn RowWriter> = match format {
            OutputFormat::Csv => Box::new(CsvWriter::new(&mut buf, &CsvOptions::default())),
            OutputFormat::Cypherl => Box::new(CypherlWriter::new(&mut buf)),
            // Table is rendered by the tabular path and never reaches here; fall
            // back to jsonl defensively alongside the real jsonl case.
            OutputFormat::Jsonl | OutputFormat::Table => Box::new(JsonlWriter::new(&mut buf)),
        };
        let _ = writer.write_header(header);
        for row in rows {
            let _ = writer.write_row(row);
        }
        let _ = writer.finish();
    }
    String::from_utf8_lossy(&buf).trim_end_matches('\n').to_string()
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

/// On the first interactive run, scaffold a commented example `config.toml` at
/// the default path so the format is discoverable (issue 22). Gated to the
/// default location: with `MGCONSOLE_CONFIG_PATH` set, the override path is the
/// user's to manage and we never write into it. A write failure is a warning, not
/// fatal — the console still starts on defaults. Called only from the interactive
/// path, so a piped/`run`/`-c`/NDJSON invocation never writes the file.
fn scaffold_config_on_first_run() {
    // An explicit override path is the user's to manage — don't scaffold into it.
    if std::env::var_os(config::CONFIG_ENV).is_some() {
        return;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(path) = config::resolve_config_path(None, home.as_deref()) else {
        return;
    };
    match config::scaffold_example_config(&path) {
        Ok(true) => eprintln!("wrote example config to {}", path.display()),
        Ok(false) => {}
        Err(message) => eprintln!("warning: {message}; continuing without an example config"),
    }
}

/// Load the config file (issue 02/03): resolve the path from
/// `MGCONSOLE_CONFIG_PATH`/home, then read its `[settings]` overlay and
/// `[profiles.*]` tables. A missing file is the normal case (empty config); a
/// malformed file is reported and treated as empty so the console still starts on
/// defaults (ADR 0012).
fn load_config() -> config::Config {
    let env = std::env::var(config::CONFIG_ENV).ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(path) = config::resolve_config_path(env.as_deref(), home.as_deref()) else {
        return config::Config::default();
    };
    match config::load(&path) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("warning: {message}; continuing on defaults");
            config::Config::default()
        }
    }
}

/// Load the tool-managed saved-queries file (issue 13): resolve its path from
/// `MGCONSOLE_QUERIES_PATH`/home, then read it. A missing file is the normal case
/// (an empty store); a malformed file is reported and treated as empty so the
/// console still starts. With no writable location the store works in-memory and
/// `:save` later reports it cannot persist.
fn load_queries() -> queries::NamedQueries {
    let env = std::env::var(queries::QUERIES_ENV).ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let Some(path) = queries::resolve_queries_path(env.as_deref(), home.as_deref()) else {
        return queries::NamedQueries::in_memory();
    };
    match queries::NamedQueries::load(path) {
        Ok(store) => store,
        Err(message) => {
            eprintln!("warning: {message}; continuing with no saved queries");
            queries::NamedQueries::in_memory()
        }
    }
}

/// Resolve the config file, the selected connection profile, the effective
/// connection (flag > profile > default), and the console Settings
/// (default < config < profile-settings < CLI flag). An unknown `--profile` is a
/// fatal, clearly-reported error (issue 03).
fn resolve_profile_connection(
    cli: &Cli,
    explicit: ExplicitFlags,
    config: &config::Config,
) -> (Connection, Settings) {
    let profile = match cli.profile.as_deref() {
        Some(name) => match config.select(name) {
            Ok(profile) => Some(profile.clone()),
            Err(message) => {
                eprintln!("error: {message}");
                std::process::exit(2);
            }
        },
        None => None,
    };
    let connection = resolve_connection(cli, &explicit, profile.as_ref());
    let file_layer = match &profile {
        Some(profile) => config.settings.overlay(&profile.settings),
        None => config.settings.clone(),
    };
    let settings = Settings::resolve(&file_layer, cli.display);
    (connection, settings)
}

/// Read which connection flags clap saw on the command line (vs a default), so a
/// profile fills only the unset ones (issue 03 precedence).
fn explicit_flags(matches: &clap::ArgMatches) -> ExplicitFlags {
    let given = |name: &str| matches.value_source(name) == Some(ValueSource::CommandLine);
    ExplicitFlags {
        host: given("host"),
        port: given("port"),
        username: given("username"),
        password: given("password"),
        use_ssl: given("use_ssl"),
    }
}

/// The REPL primary prompt: names the active profile when one is selected (issue
/// 03) and shows a `[read-only]` marker while the guard is active (issue 04), so
/// the user always knows which connection they are on and whether writes are
/// blocked. With neither, the prompt is the plain `memgraph> `.
fn repl_prompt(info: &PromptInfo) -> String {
    use std::fmt::Write as _;
    let mut prompt = format!("memgraph@{}", info.endpoint);
    if let Some(db) = &info.database {
        write!(prompt, "/{db}").unwrap();
    }
    if let Some(name) = &info.profile {
        write!(prompt, " ({name})").unwrap();
    }
    if info.read_only {
        prompt.push_str(" [read-only]");
    }
    match info.tx {
        TransactionState::Auto => {}
        TransactionState::Open => prompt.push_str(" [tx]"),
        TransactionState::Failed => prompt.push_str(" [tx failed]"),
    }
    prompt.push_str("> ");
    prompt
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
    /// Shared prompt state (profile/read-only/transaction/endpoint), updated by
    /// the `SessionRunner` and read each line so the prompt tracks them live
    /// (issues 03–07).
    prompt: std::sync::Arc<std::sync::Mutex<PromptInfo>>,
}

impl RustylineSource {
    fn new(
        history: Option<HistoryFile>,
        colorize: bool,
        prompt: std::sync::Arc<std::sync::Mutex<PromptInfo>>,
    ) -> rustyline::Result<Self> {
        let mut editor = Editor::new()?;
        editor.set_helper(Some(MgHelper::new(colorize)));
        if let Some(history) = &history {
            history.load(editor.history_mut());
        }
        Ok(Self {
            editor,
            history,
            prompt,
        })
    }
}

impl LineSource for RustylineSource {
    fn read(&mut self, continued: bool, initial: Option<&str>) -> io::Result<Line> {
        let primary = self
            .prompt
            .lock()
            .map_or_else(|_| "memgraph> ".to_string(), |info| repl_prompt(&info));
        let prompt = if continued { "      -> " } else { &primary };
        // `:load` recall (issue 13) pre-fills the line with the saved template for
        // review and edit; an ordinary read starts from an empty line.
        let read = match initial {
            Some(text) => self.editor.readline_with_initial(prompt, (text, "")),
            None => self.editor.readline(prompt),
        };
        match read {
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
    /// The resolved result format (issue 19): `Table` renders the display-mode
    /// tabular layout (the interactive default); the streaming formats render rows
    /// via the Core writers, so a redirected REPL (`mgconsole > out`) emits jsonl.
    output_format: OutputFormat,
    /// The config (profiles) and the current connect options, so `:connect` can
    /// resolve a profile or a bare endpoint and re-establish (issue 07).
    config: mgconsole::config::Config,
    options: ConnectOptions,
    /// Shared prompt state, read by the rustyline source each line so the prompt
    /// reflects read-only (issue 04), the transaction (issue 05), and the
    /// connection (issue 07) live.
    prompt: std::sync::Arc<std::sync::Mutex<PromptInfo>>,
}

/// The connection facts the REPL prompt shows, shared with the rustyline source.
#[derive(Clone)]
struct PromptInfo {
    endpoint: String,
    profile: Option<String>,
    read_only: bool,
    tx: TransactionState,
    /// The active Database name once switched with `:use` (issue 08); `None` keeps
    /// the server default and shows nothing.
    database: Option<String>,
}

impl SessionRunner<'_> {
    /// Mirror the Session's transaction state into the shared prompt so the prompt
    /// reflects it (after begin/commit/rollback, and after a query that may have
    /// poisoned an open transaction).
    fn sync_tx(&self) {
        if let Ok(mut prompt) = self.prompt.lock() {
            prompt.tx = self.session.transaction_state();
        }
    }
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
        let output_format = self.output_format;
        // The `auto` display mode needs the live terminal width to decide whether
        // a row fits; resolve it here at the IO boundary (the Core stays
        // width-agnostic), independently of the `--fit-to-screen` column fitting.
        let render_options = RenderOptions {
            mode: display,
            table: self.table_options.clone(),
            term_width: terminal_size::terminal_size().map(|(width, _)| width.0),
        };

        let result = runtime.block_on(async move {
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
            } else if output_format == OutputFormat::Table {
                render_records(&header, &rows, &render_options)
            } else {
                // A redirected REPL (or an explicit `--output-format`, issue 19)
                // renders rows through the Core writers instead of the table.
                render_rows_as(output_format, &header, &rows)
            };
            Ok(Rendered {
                table,
                row_count: rows.len(),
                overflowed,
                elapsed,
            })
        });
        // A query error inside an open transaction poisons it (issue 05); mirror
        // the new state so the prompt marker updates.
        self.sync_tx();
        result
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

    fn set_read_only(&mut self, on: bool) {
        self.session.set_read_only(on);
        if let Ok(mut prompt) = self.prompt.lock() {
            prompt.read_only = on;
        }
    }

    fn is_read_only(&self) -> bool {
        self.session.is_read_only()
    }

    fn begin(&mut self) -> Result<(), Error> {
        let result = self.runtime.block_on(self.session.begin());
        self.sync_tx();
        result
    }

    fn commit(&mut self) -> Result<(), Error> {
        let result = self.runtime.block_on(self.session.commit());
        self.sync_tx();
        result
    }

    fn rollback(&mut self) -> Result<(), Error> {
        let result = self.runtime.block_on(self.session.rollback());
        self.sync_tx();
        result
    }

    fn transaction_state(&self) -> TransactionState {
        self.session.transaction_state()
    }

    fn connect(&mut self, target: &str) -> Result<String, Error> {
        // Resolve the target (profile or host[:port]) against the current
        // connection, then establish a fresh Session — a swap, not a mutation
        // (issue 07). The prior Session is only dropped once the new one connects,
        // so a failed connect leaves the session intact.
        let resolved = resolve_connect_target(target, &self.config, self.session.endpoint(), &self.options)
            .map_err(Error::Protocol)?;
        let session = self
            .runtime
            .block_on(Session::connect_with(&resolved.endpoint, &resolved.options))?;
        let label = match &resolved.profile {
            Some(name) => format!("{name} ({})", resolved.endpoint),
            None => resolved.endpoint.to_string(),
        };
        self.session = session;
        self.options = resolved.options;
        if let Ok(mut prompt) = self.prompt.lock() {
            prompt.endpoint = resolved.endpoint.to_string();
            prompt.profile = resolved.profile;
            prompt.read_only = self.session.is_read_only();
            prompt.tx = TransactionState::Auto;
            // A new Session starts on the server's default Database.
            prompt.database = None;
        }
        Ok(label)
    }

    fn use_database(&mut self, database: &str) -> Result<(), Error> {
        self.runtime.block_on(self.session.use_database(database))?;
        if let Ok(mut prompt) = self.prompt.lock() {
            prompt.database = Some(database.to_string());
        }
        Ok(())
    }

    fn run_to_file(
        &mut self,
        query: &str,
        params: &BTreeMap<String, Value>,
        format: OutputFormat,
        path: &std::path::Path,
    ) -> Result<usize, Error> {
        use mgconsole_core::format::{CypherlWriter, JsonlWriter, RowWriter};
        let runtime = self.runtime;
        let session = &mut self.session;
        runtime.block_on(async move {
            let mut result = session.run_with_params(query, params).await?;
            let header = Header::new(result.header());
            let file = std::fs::File::create(path).map_err(|e| Error::Output(e.to_string()))?;
            let mut sink = std::io::BufWriter::new(file);
            let mut rows = 0usize;
            match format {
                // Streaming formats: write each row as it arrives (bounded memory).
                OutputFormat::Csv | OutputFormat::Jsonl | OutputFormat::Cypherl => {
                    let mut writer: Box<dyn RowWriter> = match format {
                        OutputFormat::Csv => Box::new(mgconsole_core::format::CsvWriter::new(
                            &mut sink,
                            &CsvOptions::default(),
                        )),
                        OutputFormat::Jsonl => Box::new(JsonlWriter::new(&mut sink)),
                        _ => Box::new(CypherlWriter::new(&mut sink)),
                    };
                    writer.write_header(&header)?;
                    while let Some(record) = result.records().next().await? {
                        writer.write_row(record.fields())?;
                        rows += 1;
                    }
                    writer.finish()?;
                }
                // `table` buffers (the documented exception) and writes the layout.
                OutputFormat::Table => {
                    let records = result.records().collect().await?;
                    let fields: Vec<Vec<Value>> = records
                        .into_iter()
                        .map(mgconsole_core::Record::into_fields)
                        .collect();
                    rows = fields.len();
                    let opts = RenderOptions {
                        mode: DisplayMode::Tabular,
                        ..RenderOptions::default()
                    };
                    std::io::Write::write_all(
                        &mut sink,
                        render_records(&header, &fields, &opts).as_bytes(),
                    )
                    .map_err(|e| Error::Output(e.to_string()))?;
                }
            }
            std::io::Write::flush(&mut sink).map_err(|e| Error::Output(e.to_string()))?;
            Ok(rows)
        })
    }

    fn watch(
        &mut self,
        query: &str,
        params: &BTreeMap<String, Value>,
        interval: std::time::Duration,
        display: DisplayMode,
        out: &mut dyn io::Write,
    ) -> io::Result<()> {
        // A one-shot stdin reader lets the user stop the watch by pressing Enter
        // (no crossterm/raw-mode, so this works in the lean REPL-only build too).
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = io::stdin().read_line(&mut line);
            let _ = tx.send(());
        });
        loop {
            // Clear the screen and home the cursor, then render the snapshot.
            write!(out, "\x1b[2J\x1b[H")?;
            match self.run(query, params, display) {
                Ok(result) => {
                    if !result.table.is_empty() {
                        writeln!(out, "{}", result.table)?;
                    }
                    writeln!(
                        out,
                        "{}",
                        repl::format_summary(result.row_count, result.elapsed)
                    )?;
                }
                Err(e) => writeln!(out, "error: {e}")?,
            }
            writeln!(
                out,
                "(watching every {:.1}s — press Enter to stop)",
                interval.as_secs_f64()
            )?;
            out.flush()?;
            match rx.recv_timeout(interval) {
                // Enter pressed (or the reader ended): stop watching.
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use std::io::Cursor;

    fn queries_of(input: &str) -> Vec<String> {
        read_queries(Cursor::new(input)).expect("read")
    }

    #[test]
    fn the_prompt_shows_endpoint_profile_read_only_and_transaction() {
        use TransactionState::{Auto, Failed, Open};
        let info = |profile: Option<&str>, read_only, tx| PromptInfo {
            endpoint: "127.0.0.1:7687".to_string(),
            profile: profile.map(str::to_string),
            read_only,
            tx,
            database: None,
        };
        assert_eq!(repl_prompt(&info(None, false, Auto)), "memgraph@127.0.0.1:7687> ");
        assert_eq!(
            repl_prompt(&info(Some("prod"), false, Auto)),
            "memgraph@127.0.0.1:7687 (prod)> "
        );
        assert_eq!(
            repl_prompt(&info(None, true, Auto)),
            "memgraph@127.0.0.1:7687 [read-only]> "
        );
        assert_eq!(
            repl_prompt(&info(None, false, Open)),
            "memgraph@127.0.0.1:7687 [tx]> "
        );
        assert_eq!(
            repl_prompt(&info(None, false, Failed)),
            "memgraph@127.0.0.1:7687 [tx failed]> "
        );
        // The active database shows after the endpoint (issue 08).
        let with_db = PromptInfo {
            database: Some("analytics".to_string()),
            ..info(Some("prod"), false, Auto)
        };
        assert_eq!(
            repl_prompt(&with_db),
            "memgraph@127.0.0.1:7687/analytics (prod)> "
        );
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
                OutputFormat::Jsonl,
                TableOptions::default()
            ),
            ImportFormat::Jsonl
        ));
        assert!(matches!(
            import_format(
                &cli_with(&["--output-format", "cypherl"]),
                OutputFormat::Cypherl,
                TableOptions::default()
            ),
            ImportFormat::Cypherl
        ));
        assert!(matches!(
            import_format(&cli_with(&[]), OutputFormat::Table, TableOptions::default()),
            ImportFormat::Tabular(_)
        ));
    }

    #[test]
    fn csv_options_carry_the_csv_flags() {
        let format = import_format(
            &cli_with(&["--output-format", "csv", "--csv-delimiter", ";"]),
            OutputFormat::Csv,
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
