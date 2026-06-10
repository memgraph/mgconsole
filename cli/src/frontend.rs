//! Frontend selection (ADR 0010): which Frontend handles a run — the full-screen
//! TUI workbench, the line-based REPL, or the non-interactive piped path.
//!
//! The choice is a pure function of three signals, in the [`ColorChoice::resolve`]
//! style, so it is unit-tested with no terminal. `main` reads the live signals
//! (the `--plain` flag, whether stdin is a terminal, whether the terminal can
//! host the TUI) at the IO boundary and routes to the chosen Frontend.
//!
//! [`ColorChoice::resolve`]: crate::ColorChoice::resolve

/// The Frontend chosen for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frontend {
    /// Non-interactive: stdin is piped, so the import/serial path handles it as
    /// today, bypassing both interactive Frontends.
    Piped,
    /// The line-based REPL (rustyline).
    Repl,
    /// The full-screen TUI workbench (the default interactive Frontend).
    Workbench,
}

/// Choose the Frontend from the three signals (ADR 0010).
///
/// The piped path is selected first and bypasses both interactive Frontends,
/// exactly as today. Within an interactive terminal the workbench is the default;
/// it yields to the REPL when `--plain` is set or the terminal cannot host the
/// full-screen UI. `supports_tui` folds in the compile-time feature: the caller
/// passes `cfg!(feature = "tui") && terminal_is_capable`, so a binary built
/// without the `tui` feature never selects the workbench.
pub fn select_frontend(plain: bool, stdin_is_tty: bool, supports_tui: bool) -> Frontend {
    if !stdin_is_tty {
        Frontend::Piped
    } else if plain || !supports_tui {
        Frontend::Repl
    } else {
        Frontend::Workbench
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_piped_stdin_takes_the_non_interactive_path_regardless() {
        // The automation path is unaffected by --plain or TUI capability.
        assert_eq!(select_frontend(false, false, true), Frontend::Piped);
        assert_eq!(select_frontend(true, false, false), Frontend::Piped);
    }

    #[test]
    fn a_capable_interactive_terminal_gets_the_workbench_by_default() {
        assert_eq!(select_frontend(false, true, true), Frontend::Workbench);
    }

    #[test]
    fn plain_falls_back_to_the_repl() {
        assert_eq!(select_frontend(true, true, true), Frontend::Repl);
    }

    #[test]
    fn an_incapable_terminal_falls_back_to_the_repl() {
        // e.g. TERM=dumb, or a build without the `tui` feature (supports_tui false).
        assert_eq!(select_frontend(false, true, false), Frontend::Repl);
    }

    #[test]
    fn plain_wins_even_on_a_capable_terminal() {
        assert_eq!(select_frontend(true, true, true), Frontend::Repl);
    }
}
