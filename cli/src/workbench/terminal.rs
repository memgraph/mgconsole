//! The panic-safe terminal lifecycle.
//!
//! [`TerminalGuard::enter`] puts the terminal into raw mode on the alternate
//! screen and installs a panic hook; the terminal is restored both on normal
//! exit (the guard's `Drop`) and on a panic (the hook runs before the default
//! hook prints, so the backtrace does not land on the alternate screen and the
//! shell is never left in raw mode). This is the standard ratatui pattern and
//! satisfies issue 01's "a panic also restores the terminal".
//!
//! At startup it also negotiates the terminal's keyboard-enhancement protocol
//! (slice 06): when the terminal supports it, the disambiguation flags are
//! pushed so Shift+Enter / Ctrl+Enter arrive as distinct key events (the reducer
//! maps them to a newline). When unsupported, nothing is pushed and those chords
//! simply never arrive — Enter still submits and the universal newline key works.

use std::io::{self, stdout};

use ratatui::crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};

/// Owns the raw-mode / alternate-screen (and keyboard-enhancement) state;
/// restores it on drop.
pub struct TerminalGuard {
    /// Whether keyboard enhancement was negotiated, so the matching pop runs on
    /// teardown only when flags were pushed.
    enhanced: bool,
}

impl TerminalGuard {
    /// Enter raw mode on the alternate screen, negotiate keyboard enhancement,
    /// and arm panic-time restoration.
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        // Negotiated, not assumed: only push flags when the terminal reports
        // support, so Shift/Ctrl+Enter become distinguishable where possible.
        let enhanced = supports_keyboard_enhancement().unwrap_or(false);
        if enhanced {
            let _ = execute!(
                stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        install_panic_hook(enhanced);
        Ok(Self { enhanced })
    }

    /// Leave the alternate screen and disable raw mode, popping the enhancement
    /// flags first if they were pushed. Best-effort: errors are swallowed because
    /// this runs during teardown and on the panic path.
    fn restore(enhanced: bool) {
        if enhanced {
            let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore(self.enhanced);
    }
}

/// Chain a panic hook that restores the terminal before the previous hook prints
/// the panic message. Called once per process from [`TerminalGuard::enter`].
fn install_panic_hook(enhanced: bool) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard::restore(enhanced);
        previous(info);
    }));
}
