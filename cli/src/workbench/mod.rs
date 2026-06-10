//! The full-screen TUI workbench Frontend (ADR 0010), feature-gated behind `tui`.
//!
//! Mirrors the REPL's loop/IO split (`cli/src/repl.rs`): a pure reducer
//! ([`update`]) over a [`WorkbenchState`] driven by [`Event`]s and returning
//! [`Effect`]s, with the ratatui draw ([`draw`]) and the async execution edge
//! here as thin adapters. The render loop multiplexes terminal-input events and
//! query-lifecycle events over one channel and never blocks: a submitted query
//! runs as a spawned task that streams Records back as events while the UI stays
//! responsive (ADR 0010). One shared Session, one query in flight (ADR 0005).

pub mod draw;
pub mod effect;
pub mod event;
pub mod highlight;
pub mod plan;
pub mod schema;
pub mod state;
pub mod terminal;
pub mod update;

pub use effect::Effect;
pub use event::{Event, Key, KeyCode};
pub use state::{WorkbenchConfig, WorkbenchState};
pub use update::update;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, stdout, BufWriter, Write as _};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{
    Event as CrosstermEvent, EventStream, KeyCode as CrosstermKeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, Mutex};

use mgconsole_core::format::{CsvOptions, CsvWriter, CypherlWriter, Header, JsonlWriter, RowWriter};
use mgconsole_core::{Error, Record, Session, Value};

use effect::ExportFormat;
use schema::{parse_schema, NODE_PROPERTIES_QUERY, REL_PROPERTIES_QUERY};
use terminal::TerminalGuard;

/// Run the workbench to completion over a connected Session, restoring the
/// terminal on quit and on panic. The Session is shared with the in-flight query
/// task behind an async mutex so the task can hold it for a query's duration and
/// release it on completion (or, in slice 07, on cancel).
pub async fn run(session: Session, config: WorkbenchConfig, color: bool) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut state = WorkbenchState::new(config, color);

    let session = Arc::new(Mutex::new(session));
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let mut input = EventStream::new();
    // Drives the running-query spinner; idle ticks are cheap (the buffer diff is
    // unchanged, so nothing is flushed to the terminal).
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(120));
    // The single in-flight query task (one-live-result, ADR 0005).
    let mut running: Option<tokio::task::JoinHandle<()>> = None;

    // Fetch the Schema on connect (Session idle), to back completion + sidebar.
    tokio::spawn(fetch_schema(Arc::clone(&session), tx.clone()));

    loop {
        terminal.draw(|frame| draw::draw(frame, &mut state))?;

        // Multiplex terminal input, query-lifecycle events, and the timer tick;
        // never block.
        let event = tokio::select! {
            maybe_input = input.next() => match maybe_input {
                Some(Ok(raw)) => match to_event(&raw) {
                    Some(event) => event,
                    None => continue,
                },
                Some(Err(_)) => continue,
                None => break, // input stream closed
            },
            Some(lifecycle) = rx.recv() => lifecycle,
            _ = ticker.tick() => Event::Tick,
        };

        for effect in update(&mut state, event) {
            match effect {
                Effect::Quit => {
                    if let Some(task) = running.take() {
                        task.abort();
                    }
                    return Ok(());
                }
                Effect::RunQuery { id, query, params } => {
                    // The reducer's busy guard means none should be live, but
                    // abort defensively so a task can never be orphaned.
                    if let Some(task) = running.take() {
                        task.abort();
                    }
                    let session = Arc::clone(&session);
                    let tx = tx.clone();
                    running = Some(tokio::spawn(run_query(session, tx, id, query, params)));
                }
                Effect::Cancel { .. } => {
                    // Abort the in-flight task: dropping its QueryResult releases
                    // the Session lock and abandons the stream, so the next query's
                    // run RESETs and recovers the Session (ADR 0005).
                    if let Some(task) = running.take() {
                        task.abort();
                    }
                }
                Effect::FetchSchema => {
                    // The fetch shares the one Session, so it serialises behind any
                    // in-flight query rather than competing with it (ADR 0005).
                    tokio::spawn(fetch_schema(Arc::clone(&session), tx.clone()));
                }
                Effect::EvaluateParam { name, expr, params } => {
                    let session = Arc::clone(&session);
                    let tx = tx.clone();
                    tokio::spawn(evaluate_param(session, tx, name, expr, params));
                }
                Effect::Export {
                    format,
                    path,
                    header,
                    rows,
                } => {
                    // File IO on the blocking pool so the render loop never stalls.
                    let tx = tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let outcome = write_export(format, &path, &header, &rows).map(|()| path);
                        let _ = tx.send(Event::ExportFinished(outcome));
                    });
                }
            }
        }
    }
    Ok(())
}

/// Run one query on the shared Session, streaming its lifecycle back as events.
/// Holds the Session lock for the query's duration so a second query cannot start
/// (ADR 0005); the lock releases when the task ends — or is dropped if the task
/// is aborted (the cancellation path wired in slice 07). Send failures mean the
/// workbench is shutting down, so the task simply stops.
// The spawned task owns `query`/`params` for its `'static` lifetime; they are
// borrowed into `run_with_params` rather than consumed, which the lint flags.
#[allow(clippy::needless_pass_by_value)]
async fn run_query(
    session: Arc<Mutex<Session>>,
    tx: mpsc::UnboundedSender<Event>,
    id: u64,
    query: String,
    params: BTreeMap<String, Value>,
) {
    let started = Instant::now();
    let mut session = session.lock().await;
    let mut result = match session.run_with_params(&query, &params).await {
        Ok(result) => result,
        Err(error) => {
            let _ = tx.send(Event::QueryFailed { id, error });
            return;
        }
    };
    if tx
        .send(Event::QueryStarted {
            id,
            header: result.header().to_vec(),
        })
        .is_err()
    {
        return;
    }
    loop {
        match result.records().next().await {
            Ok(Some(record)) => {
                if tx.send(Event::RecordArrived { id, record }).is_err() {
                    return;
                }
            }
            Ok(None) => {
                let _ = tx.send(Event::QueryCompleted {
                    id,
                    summary: result.summary().clone(),
                    elapsed: started.elapsed(),
                });
                return;
            }
            Err(error) => {
                let _ = tx.send(Event::QueryFailed { id, error });
                return;
            }
        }
    }
}

/// Fetch the database Schema over the shared Session and deliver it as
/// [`Event::SchemaLoaded`] (slice 12). When the schema-metadata feature is off
/// the procedures error, so the result is `None` and the workbench degrades
/// silently to static-only completion — it never scans the graph.
async fn fetch_schema(session: Arc<Mutex<Session>>, tx: mpsc::UnboundedSender<Event>) {
    let schema = {
        let mut session = session.lock().await;
        match (
            collect(&mut session, NODE_PROPERTIES_QUERY).await,
            collect(&mut session, REL_PROPERTIES_QUERY).await,
        ) {
            (Ok(nodes), Ok(rels)) => Some(parse_schema(&nodes, &rels)),
            _ => None,
        }
    };
    let _ = tx.send(Event::SchemaLoaded(schema));
}

/// Run a query and collect all its records (used for the small schema results).
async fn collect(session: &mut Session, query: &str) -> Result<Vec<Record>, Error> {
    let mut result = session.run(query).await?;
    result.records().collect().await
}

/// Evaluate a `:param` expression server-side by running `RETURN <expr>` with the
/// current params in scope, and deliver the single resulting Value (slice 16,
/// mirroring the REPL's evaluation). A bad expression becomes `Err` — the Session
/// survives it (ADR 0005).
async fn evaluate_param(
    session: Arc<Mutex<Session>>,
    tx: mpsc::UnboundedSender<Event>,
    name: String,
    expr: String,
    params: BTreeMap<String, Value>,
) {
    let value = {
        let mut session = session.lock().await;
        match session
            .run_with_params(&format!("RETURN {expr}"), &params)
            .await
        {
            Ok(mut result) => match result.records().collect().await {
                Ok(rows) => Ok(rows
                    .into_iter()
                    .next()
                    .and_then(|row| row.into_fields().into_iter().next())
                    .unwrap_or(Value::Null)),
                Err(error) => Err(error.to_string()),
            },
            Err(error) => Err(error.to_string()),
        }
    };
    let _ = tx.send(Event::ParamEvaluated { name, value });
}

/// Write the on-screen result to `path` in `format`, reusing the Core's streaming
/// row writers (slice 09). Returns a human-readable message on failure.
fn write_export(
    format: ExportFormat,
    path: &Path,
    header: &[String],
    rows: &[Record],
) -> Result<(), String> {
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut sink = BufWriter::new(file);
    let header = Header::new(header.to_vec());
    match format {
        ExportFormat::Csv => write_rows(
            &mut CsvWriter::new(&mut sink, &CsvOptions::default()),
            &header,
            rows,
        )?,
        ExportFormat::Jsonl => write_rows(&mut JsonlWriter::new(&mut sink), &header, rows)?,
        ExportFormat::Cypherl => write_rows(&mut CypherlWriter::new(&mut sink), &header, rows)?,
    }
    sink.flush().map_err(|e| e.to_string())
}

/// Drive the in-memory rows through a [`RowWriter`]: header, then each row.
fn write_rows<W: RowWriter>(
    writer: &mut W,
    header: &Header,
    rows: &[Record],
) -> Result<(), String> {
    writer.write_header(header).map_err(|e| e.to_string())?;
    for row in rows {
        writer.write_row(row.fields()).map_err(|e| e.to_string())?;
    }
    writer.finish().map_err(|e| e.to_string())
}

/// Translate a crossterm event into a workbench [`Event`], or `None` for one the
/// workbench ignores (a key *release*, a mouse event, a focus change). Key
/// repeats are treated like presses.
fn to_event(raw: &CrosstermEvent) -> Option<Event> {
    match raw {
        CrosstermEvent::Key(key)
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
        {
            Some(Event::Key(translate_key(*key)))
        }
        CrosstermEvent::Resize(width, height) => Some(Event::Resize(*width, *height)),
        _ => None,
    }
}

/// Translate a crossterm key event into the Frontend-neutral [`Key`]. Keys
/// outside the workbench's vocabulary map to [`KeyCode::Other`] and are ignored
/// by the reducer.
fn translate_key(key: KeyEvent) -> Key {
    let code = match key.code {
        CrosstermKeyCode::Char(c) => KeyCode::Char(c),
        CrosstermKeyCode::Enter => KeyCode::Enter,
        CrosstermKeyCode::Esc => KeyCode::Esc,
        CrosstermKeyCode::Backspace => KeyCode::Backspace,
        CrosstermKeyCode::Delete => KeyCode::Delete,
        CrosstermKeyCode::Tab => KeyCode::Tab,
        CrosstermKeyCode::Left => KeyCode::Left,
        CrosstermKeyCode::Right => KeyCode::Right,
        CrosstermKeyCode::Up => KeyCode::Up,
        CrosstermKeyCode::Down => KeyCode::Down,
        CrosstermKeyCode::Home => KeyCode::Home,
        CrosstermKeyCode::End => KeyCode::End,
        CrosstermKeyCode::PageUp => KeyCode::PageUp,
        CrosstermKeyCode::PageDown => KeyCode::PageDown,
        _ => KeyCode::Other,
    };
    Key {
        code,
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: key.modifiers.contains(KeyModifiers::SHIFT),
    }
}
