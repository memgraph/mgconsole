//! The interactive REPL Frontend (slice 16).
//!
//! Connects a Core Session, then drives [`mgconsole::repl::run_loop`] — the
//! testable loop seam — with the real IO it abstracts: rustyline for input on a
//! terminal, a plain line reader when stdin is piped, and a [`SessionRunner`]
//! that runs each query and renders it as tabular. The async Core is driven via
//! `block_on` at this boundary (ADR 0002). Auth/TLS/flag parsing (slices 09–11)
//! are reused unchanged; the loop itself is covered by unit tests in `repl`.

use std::io::{self, BufRead, IsTerminal};
use std::time::Instant;

use clap::Parser;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Completer, Editor, Helper, Highlighter, Hinter};

use mgconsole::repl::{self, Line, LineSource, QueryRunner, Rendered, ReplConfig};
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
        let mut source = RustylineSource::new()?;
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

/// rustyline helper whose only custom behaviour is multiline validation: keep
/// editing until [`repl::is_complete`] reports the buffer is a whole query
/// (slice 16). Completion/hinting/highlighting are the no-op defaults until
/// slice 18 fills them in.
#[derive(Completer, Helper, Highlighter, Hinter)]
struct MgHelper;

impl Validator for MgHelper {
    fn validate(&self, ctx: &mut ValidationContext) -> rustyline::Result<ValidationResult> {
        if repl::is_complete(ctx.input()) {
            Ok(ValidationResult::Valid(None))
        } else {
            Ok(ValidationResult::Incomplete)
        }
    }
}

/// Interactive input via rustyline, with its `Validator` assembling multiline
/// queries inside a single `readline` call.
struct RustylineSource {
    editor: Editor<MgHelper, rustyline::history::DefaultHistory>,
}

impl RustylineSource {
    fn new() -> rustyline::Result<Self> {
        let mut editor = Editor::new()?;
        editor.set_helper(Some(MgHelper));
        Ok(Self { editor })
    }
}

impl LineSource for RustylineSource {
    fn read(&mut self, continued: bool) -> io::Result<Line> {
        let prompt = if continued { "      -> " } else { "memgraph> " };
        match self.editor.readline(prompt) {
            Ok(line) => {
                // Best-effort in-memory history; persistence is slice 17.
                let _ = self.editor.add_history_entry(line.as_str());
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
