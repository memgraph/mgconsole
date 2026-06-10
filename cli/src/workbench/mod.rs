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
pub mod state;
pub mod terminal;
pub mod update;

pub use effect::Effect;
pub use event::{Event, Key, KeyCode};
pub use state::{WorkbenchConfig, WorkbenchState};
pub use update::update;

use std::collections::BTreeMap;
use std::io::{self, stdout};
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

use mgconsole_core::{Session, Value};

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
    // The single in-flight query task (one-live-result, ADR 0005).
    let mut running: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        terminal.draw(|frame| draw::draw(frame, &mut state))?;

        // Multiplex terminal input and query-lifecycle events; never block.
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
