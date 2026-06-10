//! The interactive REPL Frontend (slice 16).
//!
//! Connects a Core Session, then drives [`mgconsole::repl::run_loop`] — the
//! testable loop seam — with the real IO it abstracts: rustyline for input on a
//! terminal, a plain line reader when stdin is piped, and a [`SessionRunner`]
//! that runs each query and renders it as tabular. The async Core is driven via
//! `block_on` at this boundary (ADR 0002). Auth/TLS/flag parsing (slices 09–11)
//! are reused unchanged; the loop itself is covered by unit tests in `repl`.

use std::borrow::Cow;
use std::io::{self, BufRead, IsTerminal};
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use rustyline::completion::Completer as RustylineCompleter;
use rustyline::highlight::{CmdKind, Highlighter as RustylineHighlighter};
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Editor, Helper, Hinter};

use mgconsole::history::{self, HistoryFile};
use mgconsole::repl::{self, Line, LineSource, QueryRunner, Rendered, ReplConfig};
use mgconsole::syntax::{self, Completer};
use mgconsole::{resolve_password, Cli};
use mgconsole_core::{
    render_table, ConnectOptions, Credentials, Error, ReconnectNotice, Session, TableOptions, Value,
    DEFAULT_ROW_CAP,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if let Err(message) = cli.validate() {
        eprintln!("error: {message}");
        std::process::exit(2);
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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let mut session = runtime.block_on(Session::connect_with(&cli.host, cli.port, &options))?;
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

    let mut runner = SessionRunner {
        runtime: &runtime,
        session,
        table_options,
        row_cap: DEFAULT_ROW_CAP,
    };
    let config = ReplConfig {
        row_cap: DEFAULT_ROW_CAP,
    };

    let mut out = io::stdout();
    let mut err = io::stderr();

    // A terminal gets the full line editor; a pipe gets a plain reader so
    // scripted input (and slice 25's import) runs without a TTY.
    if io::stdin().is_terminal() {
        let history = open_history(&cli);
        let mut source = RustylineSource::new(history, cli.term_colors)?;
        repl::run_loop(&mut source, &mut runner, &mut out, &mut err, &config)?;
    } else {
        let stdin = io::stdin();
        let mut source = PipeSource {
            reader: stdin.lock(),
        };
        repl::run_loop(&mut source, &mut runner, &mut out, &mut err, &config)?;
    }

    Ok(())
}

/// Resolve and prepare the history file for an interactive session, honouring
/// the `--history`/`--no-history` flags and the `MGCONSOLE_HISTORY_PATH` env
/// override (slice 17). A history directory that cannot be created is reported
/// and history is disabled, so the REPL still runs.
fn open_history(cli: &Cli) -> Option<HistoryFile> {
    let env = std::env::var(history::HISTORY_ENV).ok();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path =
        history::resolve_history_file(&cli.history, env.as_deref(), cli.no_history, home.as_deref())?;
    match history::prepare_history_dir(&path) {
        Ok(()) => Some(HistoryFile::new(path)),
        Err(message) => {
            eprintln!("warning: {message}; continuing without history");
            None
        }
    }
}

/// rustyline helper for the REPL: multiline validation (slice 16), static
/// keyword/function completion, and optional Cypher highlighting (slice 18).
/// Hinting stays the no-op default.
#[derive(Helper, Hinter)]
struct MgHelper {
    completer: Completer,
    /// Whether to colour input; gated by `--term-colors` (mgconsole default off).
    term_colors: bool,
}

impl MgHelper {
    fn new(term_colors: bool) -> Self {
        Self {
            completer: Completer::with_static_vocabulary(),
            term_colors,
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
        if self.term_colors {
            Cow::Owned(syntax::highlight(line))
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_char(&self, line: &str, _pos: usize, _kind: CmdKind) -> bool {
        // Re-highlight on every keystroke while colouring is on.
        self.term_colors && !line.is_empty()
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
    fn new(history: Option<HistoryFile>, term_colors: bool) -> rustyline::Result<Self> {
        let mut editor = Editor::new()?;
        editor.set_helper(Some(MgHelper::new(term_colors)));
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

/// Non-interactive input: one physical line per read, EOF at end of stream.
struct PipeSource<R: BufRead> {
    reader: R,
}

impl<R: BufRead> LineSource for PipeSource<R> {
    fn read(&mut self, _continued: bool) -> io::Result<Line> {
        let mut buf = String::new();
        if self.reader.read_line(&mut buf)? == 0 {
            return Ok(Line::Eof);
        }
        let trimmed = buf.trim_end_matches(['\n', '\r']);
        Ok(Line::Text(trimmed.to_string()))
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
    fn run(&mut self, query: &str) -> Result<Rendered, Error> {
        let runtime = self.runtime;
        let session = &mut self.session;
        let cap = self.row_cap;
        let table_options = &self.table_options;

        runtime.block_on(async move {
            let start = Instant::now();
            let mut result = session.run(query).await?;
            let header = result.header().to_vec();
            let (records, overflowed) = result.records().collect_capped(cap).await?;
            // Drop any rows beyond the cap so the connection is ready for reuse.
            result.records().discard().await?;
            let elapsed = start.elapsed();

            let rows: Vec<Vec<Value>> = records.into_iter().map(|r| r.into_fields()).collect();
            // A write returns no columns; rendering an empty table is just a
            // degenerate box, so leave it out and let the summary speak.
            let table = if header.is_empty() {
                String::new()
            } else {
                render_table(&header, &rows, table_options)
            };
            Ok(Rendered {
                table,
                row_count: rows.len(),
                overflowed,
                elapsed,
            })
        })
    }
}
