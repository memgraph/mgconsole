//! The events the workbench reducer consumes, and the Frontend-neutral [`Key`]
//! they carry.
//!
//! [`Key`] is the analogue of [`repl::Line`](crate::repl::Line): the crossterm
//! adapter at the IO edge ([`super::run`]) translates a `crossterm::KeyEvent`
//! into a `Key` so the reducer never depends on crossterm and can be driven by
//! hand-built keys in tests with no terminal. Query-lifecycle events (a record
//! arrived, a query completed) carry the `id` of the query they belong to.

use std::path::PathBuf;
use std::time::Duration;

use mgconsole_core::{Error, Record, Summary, Value};

use super::schema::Schema;

/// A key the reducer can act on, modifier flags alongside a [`KeyCode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Key {
    /// A bare key press with no modifiers — the common case in tests.
    pub fn plain(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    /// A printable character with no modifiers.
    pub fn char(c: char) -> Self {
        Self::plain(KeyCode::Char(c))
    }

    /// Ctrl + the given key.
    pub fn ctrl(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: true,
            alt: false,
            shift: false,
        }
    }

    /// Alt + the given key.
    pub fn alt(code: KeyCode) -> Self {
        Self {
            code,
            ctrl: false,
            alt: true,
            shift: false,
        }
    }
}

/// The keys the workbench distinguishes. A neutral subset of crossterm's
/// `KeyCode`; unmapped keys translate to [`KeyCode::Other`] and are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// Any key the workbench does not act on.
    Other,
}

/// One event into the reducer: a terminal-input event, or a query-lifecycle
/// event delivered by the async execution edge ([`super::run`]) over the channel
/// (slice 02). Each lifecycle event carries the `id` of the query it belongs to,
/// so the reducer can drop stragglers from a superseded query.
///
/// Not `Clone`/`Eq`: lifecycle events carry owned, move-only payloads (a
/// [`Record`], a [`Summary`], an [`Error`]). The reducer consumes each event by
/// value, the way `run_loop` consumes a `Line`.
#[derive(Debug)]
pub enum Event {
    /// A terminal key press.
    Key(Key),
    /// The terminal was resized.
    Resize(u16, u16),
    /// A periodic timer tick, advancing the running-query spinner (slice 07).
    Tick,
    /// A query began; its column header is known.
    QueryStarted { id: u64, header: Vec<String> },
    /// One record streamed in.
    RecordArrived { id: u64, record: Record },
    /// The query drained cleanly; `elapsed` is the wall-clock round-trip and
    /// `summary` the trailing notifications/stats (surfaced fully in slice 18).
    QueryCompleted {
        id: u64,
        summary: Summary,
        elapsed: Duration,
    },
    /// The query failed; the Session survives (ADR 0005) and the next runs.
    QueryFailed { id: u64, error: Error },
    /// An export finished: `Ok(path)` written, or `Err(message)` (slice 09).
    ExportFinished(Result<PathBuf, String>),
    /// The Schema fetch completed: `Some` when the metadata feature is on,
    /// `None` when it is off/unavailable (degrade silently to static-only,
    /// slice 12).
    SchemaLoaded(Option<Schema>),
    /// A `:param` expression was evaluated server-side (slice 16): `Ok(value)` to
    /// store as `$name`, or `Err(message)` to report without losing the session.
    ParamEvaluated {
        name: String,
        value: Result<Value, String>,
    },
}
