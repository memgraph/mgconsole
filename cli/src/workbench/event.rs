//! The events the workbench reducer consumes, and the Frontend-neutral [`Key`]
//! they carry.
//!
//! [`Key`] is the analogue of [`repl::Line`](crate::repl::Line): the crossterm
//! adapter at the IO edge ([`super::run`]) translates a `crossterm::KeyEvent`
//! into a `Key` so the reducer never depends on crossterm and can be driven by
//! hand-built keys in tests with no terminal. Query-lifecycle events (a record
//! arrived, a query completed) join this enum in slice 02.

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

/// One input event into the reducer. Resize is observed so a future layout can
/// react; the draw already re-reads the terminal size each frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Key(Key),
    Resize(u16, u16),
}
