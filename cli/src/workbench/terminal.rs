//! The panic-safe terminal lifecycle.
//!
//! [`TerminalGuard::enter`] puts the terminal into raw mode on the alternate
//! screen and installs a panic hook; the terminal is restored both on normal
//! exit (the guard's `Drop`) and on a panic (the hook runs before the default
//! hook prints, so the backtrace does not land on the alternate screen and the
//! shell is never left in raw mode). This is the standard ratatui pattern and
//! satisfies issue 01's "a panic also restores the terminal".

use std::io::{self, stdout};

use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

/// Owns the raw-mode / alternate-screen state; restores it on drop.
pub struct TerminalGuard {
    _private: (),
}

impl TerminalGuard {
    /// Enter raw mode on the alternate screen and arm panic-time restoration.
    pub fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;
        install_panic_hook();
        Ok(Self { _private: () })
    }

    /// Leave the alternate screen and disable raw mode. Best-effort: errors are
    /// swallowed because this runs during teardown and on the panic path.
    fn restore() {
        let _ = execute!(stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        Self::restore();
    }
}

/// Chain a panic hook that restores the terminal before the previous hook prints
/// the panic message. Idempotent enough in practice — [`TerminalGuard::enter`] is
/// called once per process.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        TerminalGuard::restore();
        previous(info);
    }));
}
