//! The workbench reducer: `update(&mut state, event) -> Vec<Effect>`.
//!
//! Pure and terminal-free, the analogue of the REPL's `run_loop` body. It
//! mutates the [`WorkbenchState`] in place (D3: avoids cloning the result rows
//! every keystroke) and returns the IO to perform as [`Effect`]s. Slice 01
//! handles the shell gestures — editing, the newline key, focus, submit, and
//! quit; query execution and its lifecycle events arrive in slice 02. Driven in
//! tests by hand-built [`Event`]s, with no terminal and no database.

use crate::repl::{meta_command, MetaCommand};

use super::effect::Effect;
use super::event::{Event, Key, KeyCode};
use super::state::{Focus, WorkbenchState};

/// Apply one event to the state, returning the effects to perform.
// Events are consumed by value: slice 02's lifecycle events carry owned data
// (a Record, a Summary, an Error) the reducer takes ownership of. Today's
// Key/Resize payloads are Copy, so the lint is transitional to this slice.
#[allow(clippy::needless_pass_by_value)]
pub fn update(state: &mut WorkbenchState, event: Event) -> Vec<Effect> {
    match event {
        // The draw re-reads the terminal size each frame, so a resize needs no
        // state change today; the event exists for layouts that will.
        Event::Resize(_, _) => Vec::new(),
        Event::Key(key) => update_key(state, key),
    }
}

fn update_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    // Quit gestures work from any pane (ADR 0010 AC: Esc / Ctrl-D leave).
    if key.code == KeyCode::Esc || (key.ctrl && key.code == KeyCode::Char('d')) {
        return vec![Effect::Quit];
    }
    match state.focus {
        Focus::Editor => editor_key(state, key),
        Focus::Results => results_key(state, key),
    }
}

/// Keys while the editor pane has focus.
fn editor_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    match key {
        // Tab cycles focus to the results pane.
        Key {
            code: KeyCode::Tab,
            ctrl: false,
            alt: false,
            ..
        } => {
            state.focus = Focus::Results;
            Vec::new()
        }
        // The universal newline gesture (slice 01): Alt+Enter or Ctrl+J insert a
        // newline; plain Enter (below) submits. Ctrl/Shift+Enter on capable
        // terminals is negotiated in slice 06.
        Key {
            code: KeyCode::Enter,
            alt: true,
            ..
        }
        | Key {
            code: KeyCode::Char('j'),
            ctrl: true,
            ..
        } => {
            state.editor.insert_newline();
            Vec::new()
        }
        // Plain Enter submits the buffer.
        Key {
            code: KeyCode::Enter,
            ctrl: false,
            alt: false,
            ..
        } => submit(state),
        // Everything else is ordinary editing, delegated to the editor widget.
        other => {
            state.editor.edit(other);
            Vec::new()
        }
    }
}

/// Keys while the results pane has focus. Empty until slice 03 fills the pane;
/// Tab returns focus to the editor.
fn results_key(state: &mut WorkbenchState, key: Key) -> Vec<Effect> {
    if key.code == KeyCode::Tab {
        state.focus = Focus::Editor;
    }
    Vec::new()
}

/// Handle a submit (plain Enter). A `:quit`/`:exit` buffer leaves the workbench,
/// reusing the REPL's meta-command parser. Otherwise the submit is acknowledged
/// non-destructively — the buffer is kept — because query execution arrives in
/// slice 02; this slice only wires the gesture.
fn submit(state: &mut WorkbenchState) -> Vec<Effect> {
    let buffer = state.editor.buffer();
    if matches!(meta_command(buffer.trim()), Some(MetaCommand::Quit)) {
        return vec![Effect::Quit];
    }
    if !state.editor.is_empty() {
        state.status.message = "query ready — execution arrives in slice 02".to_string();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workbench::state::WorkbenchConfig;

    fn wb() -> WorkbenchState {
        WorkbenchState::new(WorkbenchConfig::default(), true)
    }

    /// Type a string into the editor, one character event at a time.
    fn type_str(state: &mut WorkbenchState, text: &str) {
        for c in text.chars() {
            update(state, Event::Key(Key::char(c)));
        }
    }

    #[test]
    fn typing_characters_builds_the_editor_buffer() {
        let mut s = wb();
        type_str(&mut s, "MATCH (n)");
        assert_eq!(s.editor.buffer(), "MATCH (n)");
    }

    #[test]
    fn backspace_deletes_the_last_character() {
        let mut s = wb();
        type_str(&mut s, "RETURNx");
        update(&mut s, Event::Key(Key::plain(KeyCode::Backspace)));
        assert_eq!(s.editor.buffer(), "RETURN");
    }

    #[test]
    fn the_cursor_moves_freely_for_mid_buffer_editing() {
        // Type, jump to the line start, and insert — free cursor editing, the
        // win over a line REPL (AC 2).
        let mut s = wb();
        type_str(&mut s, "abc");
        update(&mut s, Event::Key(Key::plain(KeyCode::Home)));
        type_str(&mut s, "X");
        assert_eq!(s.editor.buffer(), "Xabc");
    }

    #[test]
    fn alt_enter_inserts_a_newline_without_submitting() {
        let mut s = wb();
        type_str(&mut s, "ab");
        let effects = update(&mut s, Event::Key(Key::alt(KeyCode::Enter)));
        type_str(&mut s, "cd");
        assert_eq!(s.editor.buffer(), "ab\ncd");
        assert!(effects.is_empty(), "a newline produces no effect");
    }

    #[test]
    fn ctrl_j_also_inserts_a_newline() {
        let mut s = wb();
        type_str(&mut s, "ab");
        update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('j'))));
        type_str(&mut s, "cd");
        assert_eq!(s.editor.buffer(), "ab\ncd");
    }

    #[test]
    fn plain_enter_submits_non_destructively_in_slice_01() {
        // Execution arrives in slice 02; the gesture is wired but keeps the
        // buffer rather than discarding the user's typing.
        let mut s = wb();
        type_str(&mut s, "RETURN 1;");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert!(effects.is_empty(), "no quit on an ordinary submit");
        assert_eq!(s.editor.buffer(), "RETURN 1;", "the buffer is preserved");
        assert!(!s.status.message.is_empty(), "the submit is acknowledged");
    }

    #[test]
    fn a_quit_meta_command_leaves_the_workbench() {
        let mut s = wb();
        type_str(&mut s, ":quit");
        let effects = update(&mut s, Event::Key(Key::plain(KeyCode::Enter)));
        assert_eq!(effects, vec![Effect::Quit]);
    }

    #[test]
    fn esc_quits() {
        let mut s = wb();
        assert_eq!(
            update(&mut s, Event::Key(Key::plain(KeyCode::Esc))),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn ctrl_d_quits() {
        let mut s = wb();
        assert_eq!(
            update(&mut s, Event::Key(Key::ctrl(KeyCode::Char('d')))),
            vec![Effect::Quit]
        );
    }

    #[test]
    fn tab_cycles_focus_between_editor_and_results() {
        let mut s = wb();
        assert_eq!(s.focus, Focus::Editor);
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert_eq!(s.focus, Focus::Results);
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab)));
        assert_eq!(s.focus, Focus::Editor);
    }

    #[test]
    fn quit_works_from_the_results_pane_too() {
        let mut s = wb();
        update(&mut s, Event::Key(Key::plain(KeyCode::Tab))); // focus results
        assert_eq!(
            update(&mut s, Event::Key(Key::plain(KeyCode::Esc))),
            vec![Effect::Quit]
        );
    }
}
