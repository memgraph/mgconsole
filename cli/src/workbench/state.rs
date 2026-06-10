//! The pure workbench state.
//!
//! Built up across slices: slice 01 ships the shell subset (editor, focus,
//! status, colour, config); slices 02–18 add fields (run state, result history,
//! completion, schema, parameters, drawers) without reshaping the type. The
//! state holds no terminal and no Session — it is driven by [`super::update`]
//! and rendered by [`super::draw`], the analogue of the REPL's loop/IO split.

use tui_textarea::{Input, Key as TaKey, TextArea};

use super::event::{Key, KeyCode};

/// The complete workbench state for the current slice.
pub struct WorkbenchState {
    /// The multiline query editor.
    pub editor: EditorState,
    /// Which pane has keyboard focus.
    pub focus: Focus,
    /// The transient status message (errors, hints; running/elapsed in slice 07).
    pub status: StatusLine,
    /// Whether colour is on (resolved `--color`/`NO_COLOR`); slice 05 uses it for
    /// editor highlighting. A monochrome workbench is styled, not disabled.
    pub color: bool,
    /// Frontend-local configuration.
    pub config: WorkbenchConfig,
}

impl WorkbenchState {
    /// A fresh workbench: an empty editor with focus, ready for input.
    pub fn new(config: WorkbenchConfig, color: bool) -> Self {
        Self {
            editor: EditorState::new(),
            focus: Focus::Editor,
            status: StatusLine::default(),
            color,
            config,
        }
    }
}

/// Which pane the keyboard drives. The results pane fills in from slice 03;
/// drawers/overlays (schema, params, summary, cell-detail) are added by their
/// slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editor,
    Results,
}

/// The one-line status message. Distinct from the keybind hint, which the draw
/// composes from [`WorkbenchConfig`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub message: String,
}

/// Frontend-local configuration resolved at startup.
#[derive(Debug, Clone)]
pub struct WorkbenchConfig {
    /// The editor pane's share of the vertical split, in percent.
    pub editor_percent: u16,
    /// The universal newline key, shown in the status hint (slice 01). Ctrl/Shift
    /// +Enter on capable terminals is negotiated in slice 06.
    pub newline_hint: &'static str,
}

impl Default for WorkbenchConfig {
    fn default() -> Self {
        Self {
            editor_percent: 40,
            newline_hint: "Alt+Enter",
        }
    }
}

/// The multiline editor, wrapping a [`tui_textarea::TextArea`] for free cursor
/// movement, selection, and undo. The widget runs **headlessly** — its
/// [`input`](TextArea::input) mutates state with no terminal — so reducer tests
/// over the editor need no terminal. The reducer feeds ordinary editing keys
/// here via [`edit`](Self::edit) and reads the buffer on submit; gesture keys
/// (Enter, the newline key, quit) it handles itself.
pub struct EditorState {
    textarea: TextArea<'static>,
}

impl EditorState {
    /// An empty editor.
    pub fn new() -> Self {
        Self {
            textarea: TextArea::default(),
        }
    }

    /// Apply an ordinary editing key (character, backspace, arrows, …) to the
    /// buffer. Keys the editor does not act on are ignored. Enter and the newline
    /// key are *not* routed here — the reducer owns those gestures.
    pub fn edit(&mut self, key: Key) {
        if let Some(input) = to_input(key) {
            self.textarea.input(input);
        }
    }

    /// Insert a newline at the cursor (the universal newline gesture).
    pub fn insert_newline(&mut self) {
        self.textarea.insert_newline();
    }

    /// The whole buffer as one string, physical lines joined by `\n`.
    pub fn buffer(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Whether the buffer is empty (no text on any line).
    pub fn is_empty(&self) -> bool {
        self.textarea.is_empty()
    }

    /// Clear the buffer back to empty (after a submit).
    pub fn clear(&mut self) {
        self.textarea = TextArea::default();
    }

    /// The underlying widget, for the draw edge to render.
    pub fn textarea(&self) -> &TextArea<'static> {
        &self.textarea
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

/// Translate a neutral [`Key`] into a `tui_textarea` input, or `None` for a key
/// the editor ignores. Enter/Esc are deliberately absent — the reducer handles
/// them as gestures, never as editing.
fn to_input(key: Key) -> Option<Input> {
    let ta_key = match key.code {
        KeyCode::Char(c) => TaKey::Char(c),
        KeyCode::Backspace => TaKey::Backspace,
        KeyCode::Delete => TaKey::Delete,
        KeyCode::Tab => TaKey::Tab,
        KeyCode::Left => TaKey::Left,
        KeyCode::Right => TaKey::Right,
        KeyCode::Up => TaKey::Up,
        KeyCode::Down => TaKey::Down,
        KeyCode::Home => TaKey::Home,
        KeyCode::End => TaKey::End,
        KeyCode::PageUp => TaKey::PageUp,
        KeyCode::PageDown => TaKey::PageDown,
        KeyCode::Enter | KeyCode::Esc | KeyCode::Other => return None,
    };
    Some(Input {
        key: ta_key,
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
    })
}
