//! The full-screen TUI workbench Frontend (ADR 0010), feature-gated behind `tui`.
//!
//! Mirrors the REPL's loop/IO split (`cli/src/repl.rs`): a pure reducer
//! ([`update`]) over a [`WorkbenchState`] driven by [`Event`]s and returning
//! [`Effect`]s, with the ratatui draw ([`draw`]) and the terminal lifecycle
//! ([`terminal`]) as thin edges. Slice 01 is the walking skeleton — shell,
//! editor, lifecycle; the async query-execution edge over the held Session
//! arrives in slice 02, at which point [`run`] becomes async and the loop
//! multiplexes terminal input with query-lifecycle events.

pub mod draw;
pub mod effect;
pub mod event;
pub mod state;
pub mod terminal;
pub mod update;

pub use effect::Effect;
pub use event::{Event, Key, KeyCode};
pub use state::{WorkbenchConfig, WorkbenchState};
pub use update::update;

use std::io::{self, stdout};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self as crossterm_event, Event as CrosstermEvent, KeyCode as CrosstermKeyCode, KeyEvent,
    KeyEventKind, KeyModifiers,
};
use ratatui::Terminal;

use mgconsole_core::Session;

use terminal::TerminalGuard;

/// Run the workbench shell to completion, restoring the terminal on quit and on
/// panic. Slice 01 wires editing, the newline key, focus, submit, and quit; the
/// `session`/`runtime` are held for slice 02, which drives query execution over
/// the connection.
// `session` is taken by value because slice 02 moves it into the execution task;
// the single-variant effect loop likewise grows non-diverging arms then. Both
// lints are transitional to this slice.
#[allow(clippy::needless_pass_by_value, clippy::never_loop)]
pub fn run(
    _session: Session,
    _runtime: &tokio::runtime::Runtime,
    config: WorkbenchConfig,
    color: bool,
) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let mut state = WorkbenchState::new(config, color);

    loop {
        terminal.draw(|frame| draw::draw(frame, &state))?;
        let Some(event) = read_event()? else {
            continue;
        };
        for effect in update(&mut state, event) {
            match effect {
                Effect::Quit => return Ok(()),
            }
        }
    }
}

/// Block for the next terminal event and translate it into a workbench [`Event`],
/// or `None` for an event the workbench ignores (a key *release*, a mouse event,
/// a focus change). Key repeats are treated like presses.
fn read_event() -> io::Result<Option<Event>> {
    match crossterm_event::read()? {
        CrosstermEvent::Key(key)
            if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
        {
            Ok(Some(Event::Key(translate_key(key))))
        }
        CrosstermEvent::Resize(width, height) => Ok(Some(Event::Resize(width, height))),
        _ => Ok(None),
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
