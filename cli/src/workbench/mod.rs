//! The full-screen TUI workbench Frontend (ADR 0010), feature-gated behind `tui`.
//!
//! Mirrors the REPL's loop/IO split (`cli/src/repl.rs`): a pure reducer
//! ([`update`]) over a [`WorkbenchState`] driven by [`Event`]s and returning
//! [`Effect`]s, with the ratatui draw ([`draw`]) and the async execution edge
//! here as thin adapters. The render loop multiplexes terminal-input events and
//! query-lifecycle events over one channel and never blocks: a submitted query
//! runs as a spawned task that streams Records back as events while the UI stays
//! responsive (ADR 0010). One shared Session, one query in flight (ADR 0005).

pub mod clipboard;
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
pub use event::{Connected, Event, Key, KeyCode};
pub use state::{WorkbenchConfig, WorkbenchState};
pub use update::update;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, stdout, BufWriter, Write as _};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, EventStream,
    KeyCode as CrosstermKeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, Mutex};

use mgconsole_core::format::{CsvOptions, CsvWriter, CypherlWriter, Header, JsonlWriter, RowWriter};
use mgconsole_core::{Error, Record, Session, Value};
use rustyline::history::{FileHistory, History, SearchDirection};

use crate::frontend::Outcome;
use crate::history::HistoryFile;
use crate::settings::Settings;
use crate::OutputFormat;
use schema::{parse_schema, NODE_PROPERTIES_QUERY, REL_PROPERTIES_QUERY};
use terminal::TerminalGuard;

/// What the Workbench hands back to the dispatch loop when it returns (ADR 0019):
/// the [`Outcome`] (quit or switch to the named Frontend) plus the slice of the
/// Session bundle the Workbench owned for its lifetime — the `:param` store and
/// Settings — so the loop can lend them to the next Frontend unchanged. The
/// Session itself is shared via the `Arc<Mutex<…>>` the loop also holds, so it is
/// not returned here.
pub struct WorkbenchExit {
    pub outcome: Outcome,
    pub params: BTreeMap<String, Value>,
    pub settings: Settings,
    /// The active Buffer's editor text at exit (issue 03), carried down to seed the
    /// REPL's input line on a `:repl` switch so a half-drafted query is not lost.
    pub carry: String,
}

/// Run the workbench until it quits or hands off to the other Frontend (ADR 0019),
/// restoring the terminal on return and on panic (the `TerminalGuard` is scoped to
/// this call). The Session is shared with the in-flight query task — and with the
/// dispatch loop — behind an async mutex, so a `:repl` switch hands the *same* live
/// Session to the REPL with nothing torn down underneath.
// The render loop is one cohesive select-multiplex over input/lifecycle/tick
// events plus the effect interpreter; splitting it would scatter the shared
// loop state (session, running task, history) across helpers for no clarity win.
#[allow(clippy::too_many_lines)]
pub async fn run(
    session: Arc<Mutex<Session>>,
    config: WorkbenchConfig,
    color: bool,
    history: Option<HistoryFile>,
) -> io::Result<WorkbenchExit> {
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut state = WorkbenchState::new(config, color);

    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();

    // Persisted command history (slice 17): load prior entries for recall, and
    // keep the store to append each submission to.
    let mut history_store = FileHistory::new();
    if let Some(file) = &history {
        file.load(&mut history_store);
        let _ = tx.send(Event::HistoryLoaded(history_entries(&history_store)));
    }
    let mut input = EventStream::new();
    // Drives the running-query spinner and the `:watch` timer; idle ticks are
    // cheap (the buffer diff is unchanged, so nothing is flushed to the terminal).
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(update::TICK_MS));
    // The single in-flight query task (one-live-result, ADR 0005).
    let mut running: Option<tokio::task::JoinHandle<()>> = None;
    // Fire-and-forget tasks that hold the shared Session (schema fetch, param
    // eval). Tracked so they can be aborted and awaited before `run` returns — a
    // `:repl` switch hands the Session to the REPL, and a straggler still holding
    // the lock would race the REPL's queries (ADR 0019 / ADR 0005).
    let mut background: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // Fetch the Schema on connect (Session idle), to back completion + sidebar.
    background.push(tokio::spawn(fetch_schema(Arc::clone(&session), tx.clone())));

    let outcome = 'session: loop {
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
                None => break 'session Outcome::Quit, // input stream closed
            },
            Some(lifecycle) = rx.recv() => lifecycle,
            _ = ticker.tick() => Event::Tick,
        };

        for effect in update(&mut state, event) {
            match effect {
                Effect::Quit => break 'session Outcome::Quit,
                Effect::SwitchTo(target) => {
                    // Hand the live Session to the named Frontend (ADR 0019). The
                    // post-loop cleanup aborts every Session-holding task before
                    // returning, and the TerminalGuard drops as `run` returns,
                    // restoring the terminal before the REPL takes the normal screen.
                    break 'session Outcome::SwitchTo(target);
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
                    background.retain(|task| !task.is_finished());
                    background.push(tokio::spawn(fetch_schema(Arc::clone(&session), tx.clone())));
                }
                Effect::EvaluateParam { name, expr, params } => {
                    let session = Arc::clone(&session);
                    let tx = tx.clone();
                    background.retain(|task| !task.is_finished());
                    background.push(tokio::spawn(evaluate_param(session, tx, name, expr, params)));
                }
                Effect::AppendHistory(line) => {
                    if let Some(file) = &history {
                        file.record(&mut history_store, &line);
                    }
                }
                Effect::SetReadOnly(on) => {
                    // Apply to the shared Session so the next query carries Bolt
                    // access mode READ (issue 04). The reducer only ever emits
                    // `true`; the off direction is refused at runtime.
                    session.lock().await.set_read_only(on);
                }
                Effect::Connect(target) => {
                    // Resolve the target and establish a fresh Session — a swap,
                    // not a mutation (issue 07). The prior Session is replaced only
                    // once the new one connects, so a failed connect leaves it intact.
                    let resolved = {
                        let guard = session.lock().await;
                        crate::resolve_connect_target(
                            &target,
                            &state.config.connect.config,
                            guard.endpoint(),
                            &state.config.connect.options,
                        )
                    };
                    let outcome = match resolved {
                        Ok(target) => {
                            match Session::connect_with(&target.endpoint, &target.options).await {
                                Ok(new_session) => {
                                    let label = match &target.profile {
                                        Some(name) => format!("{name} ({})", target.endpoint),
                                        None => target.endpoint.to_string(),
                                    };
                                    let read_only = new_session.is_read_only();
                                    *session.lock().await = new_session;
                                    // Re-fetch the Schema for the new database.
                                    tokio::spawn(fetch_schema(Arc::clone(&session), tx.clone()));
                                    Ok(Connected {
                                        endpoint: target.endpoint.to_string(),
                                        profile: target.profile,
                                        read_only,
                                        label,
                                    })
                                }
                                Err(e) => Err(e.to_string()),
                            }
                        }
                        Err(message) => Err(message),
                    };
                    let _ = tx.send(Event::Connected(outcome));
                }
                Effect::RunQueryToFile { id, query, params, format, path } => {
                    // Stream the query's result to a file (issue 12) off the render
                    // loop; report rows written via the tagged Redirected event.
                    if let Some(task) = running.take() {
                        task.abort();
                    }
                    let session = Arc::clone(&session);
                    let tx = tx.clone();
                    running = Some(tokio::spawn(run_query_to_file(
                        session, tx, id, query, params, format, path,
                    )));
                }
                Effect::PersistQuery(name) => {
                    // Reconcile the one changed file after a `:save`/`:forget` (issue
                    // 23, ADR 0020). A failure is a status warning — the in-memory
                    // store keeps the change for the session.
                    if let Err(message) = state.queries.sync_file(&name) {
                        state.status.message = format!("warning: {message}");
                    }
                }
                Effect::Source(path) => {
                    // Read the file off the render loop; the reducer runs its
                    // statements as a stop-on-error batch when the contents arrive
                    // (issue 10).
                    let tx = tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let result = std::fs::read_to_string(&path)
                            .map_err(|e| format!("cannot read source file {}: {e}", path.display()));
                        let _ = tx.send(Event::SourceLoaded(result));
                    });
                }
                Effect::UseDatabase(database) => {
                    // Switch the active Database on the shared Session (issue 08).
                    // On success re-fetch the Schema so completion + the sidebar
                    // reflect the newly-active Database.
                    let outcome = session.lock().await.use_database(&database).await;
                    let event = match outcome {
                        Ok(()) => {
                            tokio::spawn(fetch_schema(Arc::clone(&session), tx.clone()));
                            Event::DatabaseChanged(Ok(database))
                        }
                        Err(e) => Event::DatabaseChanged(Err(e.to_string())),
                    };
                    let _ = tx.send(event);
                }
                Effect::Transaction(op) => {
                    // Apply the explicit-transaction operation on the shared Session
                    // (issue 05) and report the resulting state + message.
                    let mut guard = session.lock().await;
                    let outcome = match op {
                        effect::TxOp::Begin => guard.begin().await,
                        effect::TxOp::Commit => guard.commit().await,
                        effect::TxOp::Rollback => guard.rollback().await,
                    };
                    let message = match outcome {
                        Ok(()) => op.success_message().to_string(),
                        Err(e) => format!("error: {e}"),
                    };
                    let state = guard.transaction_state();
                    drop(guard);
                    let _ = tx.send(Event::TransactionApplied {
                        state,
                        message: Some(message),
                    });
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
                Effect::CopyToClipboard(text) => {
                    // Prefer a local clipboard helper, fall back to OSC 52 (ADR
                    // 0018): a local box that does not honour OSC 52 still gets a
                    // real clipboard once a helper is installed, while SSH-without-
                    // helper keeps working. Report the path taken so a silent no-op
                    // becomes a visible, explained outcome.
                    let path = clipboard::copy(&text);
                    state.status.message =
                        format!("{} {}", state.status.message, path.status_suffix());
                }
                Effect::SetMouseCapture(on) => {
                    // Release or re-acquire mouse capture (issue 04): off restores
                    // native click-drag selection; on restores the mouse gestures.
                    let _ = if on {
                        execute!(stdout(), EnableMouseCapture)
                    } else {
                        execute!(stdout(), DisableMouseCapture)
                    };
                }
            }
        }
    };
    // Stop every task that holds the shared Session before returning, so nothing
    // touches it once the next Frontend (the REPL, on a `:repl` switch) takes over.
    // Abort *and await* each: the await returns only after the task has unwound and
    // dropped its Session lock/Arc clone, making the hand-off race-free (ADR 0019).
    if let Some(task) = running.take() {
        task.abort();
        let _ = task.await;
    }
    for task in background {
        task.abort();
        let _ = task.await;
    }
    // Hand the bundle slice the Workbench owned back to the dispatch loop so the
    // next Frontend inherits the params and Settings unchanged (ADR 0019), plus the
    // active Buffer's editor text to carry down onto the REPL's input line (issue 03).
    Ok(WorkbenchExit {
        outcome,
        params: std::mem::take(&mut state.params),
        settings: state.settings.clone(),
        carry: state.editor.buffer(),
    })
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
            // A query error inside an open transaction poisons it (issue 05); sync
            // the marker without a message (the error is already surfaced).
            let _ = tx.send(Event::TransactionApplied {
                state: session.transaction_state(),
                message: None,
            });
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

/// Extract the loaded history entries as plain strings, oldest→newest, for the
/// reducer's recall list (slice 17).
fn history_entries(history: &FileHistory) -> Vec<String> {
    (0..history.len())
        .filter_map(|i| {
            history
                .get(i, SearchDirection::Forward)
                .ok()
                .flatten()
                .map(|entry| entry.entry.into_owned())
        })
        .collect()
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
/// row writers (slice 09) and the shared format vocabulary (issue 12). Returns a
/// human-readable message on failure.
fn write_export(
    format: OutputFormat,
    path: &Path,
    header: &[String],
    rows: &[Record],
) -> Result<(), String> {
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut sink = BufWriter::new(file);
    let header = Header::new(header.to_vec());
    match format {
        OutputFormat::Csv => write_rows(
            &mut CsvWriter::new(&mut sink, &CsvOptions::default()),
            &header,
            rows,
        )?,
        OutputFormat::Jsonl => write_rows(&mut JsonlWriter::new(&mut sink), &header, rows)?,
        OutputFormat::Cypherl => write_rows(&mut CypherlWriter::new(&mut sink), &header, rows)?,
        // The export prompt never offers `table`, but render the layout if asked.
        OutputFormat::Table => {
            let fields: Vec<Vec<Value>> = rows.iter().map(|r| r.fields().to_vec()).collect();
            let opts = mgconsole_core::RenderOptions {
                mode: mgconsole_core::DisplayMode::Tabular,
                ..mgconsole_core::RenderOptions::default()
            };
            write!(sink, "{}", mgconsole_core::render_records(&header, &fields, &opts))
                .map_err(|e| e.to_string())?;
        }
    }
    sink.flush().map_err(|e| e.to_string())
}

/// Run a query and stream its result to a file (`:o`, issue 12), reporting the
/// rows written via a tagged [`Event::Redirected`]. Streams row-by-row for the
/// streaming formats (bounded memory); `table` buffers as it must.
async fn run_query_to_file(
    session: Arc<Mutex<Session>>,
    tx: mpsc::UnboundedSender<Event>,
    id: u64,
    query: String,
    params: BTreeMap<String, Value>,
    format: OutputFormat,
    path: std::path::PathBuf,
) {
    let result = stream_query_to_file(&session, &query, &params, format, &path).await;
    let _ = tx.send(Event::Redirected {
        id,
        result: result.map(|rows| (path, rows)),
    });
}

/// The async body of `:o`: run the query and write its result to `path`.
async fn stream_query_to_file(
    session: &Arc<Mutex<Session>>,
    query: &str,
    params: &BTreeMap<String, Value>,
    format: OutputFormat,
    path: &Path,
) -> Result<usize, String> {
    let mut session = session.lock().await;
    let mut result = session
        .run_with_params(query, params)
        .await
        .map_err(|e| e.to_string())?;
    let header = Header::new(result.header().to_vec());
    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut sink = BufWriter::new(file);
    // Stream row-by-row through a concrete writer (no trait object, so the spawned
    // future stays `Send`); `table` buffers as the documented exception.
    let rows = match format {
        OutputFormat::Csv => {
            stream_rows(CsvWriter::new(&mut sink, &CsvOptions::default()), &header, &mut result)
                .await?
        }
        OutputFormat::Jsonl => {
            stream_rows(JsonlWriter::new(&mut sink), &header, &mut result).await?
        }
        OutputFormat::Cypherl => {
            stream_rows(CypherlWriter::new(&mut sink), &header, &mut result).await?
        }
        OutputFormat::Table => {
            let records = result.records().collect().await.map_err(|e| e.to_string())?;
            let fields: Vec<Vec<Value>> = records.into_iter().map(Record::into_fields).collect();
            let opts = mgconsole_core::RenderOptions {
                mode: mgconsole_core::DisplayMode::Tabular,
                ..mgconsole_core::RenderOptions::default()
            };
            write!(sink, "{}", mgconsole_core::render_records(&header, &fields, &opts))
                .map_err(|e| e.to_string())?;
            fields.len()
        }
    };
    sink.flush().map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Drive a query's record stream through one concrete [`RowWriter`], returning the
/// rows written. Generic (not a trait object) so the spawned `:o` task is `Send`.
async fn stream_rows<W: RowWriter>(
    mut writer: W,
    header: &Header,
    result: &mut mgconsole_core::QueryResult,
) -> Result<usize, String> {
    writer.write_header(header).map_err(|e| e.to_string())?;
    let mut rows = 0usize;
    while let Some(record) = result.records().next().await.map_err(|e| e.to_string())? {
        writer.write_row(record.fields()).map_err(|e| e.to_string())?;
        rows += 1;
    }
    writer.finish().map_err(|e| e.to_string())?;
    Ok(rows)
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
        CrosstermEvent::Mouse(mouse) => translate_mouse(*mouse),
        _ => None,
    }
}

/// Translate a crossterm mouse event into a workbench [`Event::Mouse`], or `None`
/// for one the workbench ignores (drag, move, release, non-left buttons). Only a
/// left-button press and the scroll wheel drive the workbench (issue 17).
fn translate_mouse(mouse: crossterm::event::MouseEvent) -> Option<Event> {
    let kind = match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => event::MouseKind::Down,
        MouseEventKind::ScrollUp => event::MouseKind::ScrollUp,
        MouseEventKind::ScrollDown => event::MouseKind::ScrollDown,
        _ => return None,
    };
    Some(Event::Mouse(event::MouseEvent {
        kind,
        column: mouse.column,
        row: mouse.row,
    }))
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
        // Shift+Tab arrives as a distinct BackTab key code (CSI Z); normalise both
        // to Tab — the shift flag (set unconditionally for BackTab below) tells them
        // apart, so the reducer sees one Tab key with the right modifier.
        CrosstermKeyCode::Tab | CrosstermKeyCode::BackTab => KeyCode::Tab,
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
        // BackTab *is* Shift+Tab; some terminals omit the SHIFT modifier on it, so
        // set shift unconditionally for it (it normalised to Tab above).
        shift: key.modifiers.contains(KeyModifiers::SHIFT)
            || matches!(key.code, CrosstermKeyCode::BackTab),
    }
}
