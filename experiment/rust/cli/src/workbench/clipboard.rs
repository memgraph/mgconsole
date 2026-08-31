//! Clipboard delivery for yank (ADR 0018): prefer a local clipboard helper
//! process, fall back to OSC 52.
//!
//! ADR 0015 set yank to copy via OSC 52 — pure bytes that ride the SSH/tmux
//! channel — and "never a native clipboard library". A grilling session found the
//! common *local* case (a Wayland VTE terminal that does not honour OSC 52) left
//! yank a silent no-op. ADR 0018 refines the *mechanism ordering*: try an external
//! helper binary first (`std::process::Command`, no FFI, no new crate — so ADR
//! 0001 stands), and only fall back to OSC 52 when none is present.
//!
//! The selection logic ([`copy_via`]) is pure over injected probe/run/fallback
//! closures, so the helper-probe and fallback are tested at this seam with no real
//! process or terminal; [`copy`] wires it to the real PATH probe and `Command`.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// One clipboard helper: the binary name and the args that make it *read stdin
/// into the clipboard*. Probed in declaration order (ADR 0018).
pub struct Helper {
    pub name: &'static str,
    pub args: &'static [&'static str],
}

/// The helper probe order (ADR 0018): Wayland, then X11 (two tools), then macOS,
/// then WSL/Windows. The first one present on PATH wins.
pub const HELPERS: &[Helper] = &[
    Helper { name: "wl-copy", args: &[] },
    Helper { name: "xclip", args: &["-selection", "clipboard"] },
    Helper { name: "xsel", args: &["--clipboard", "--input"] },
    Helper { name: "pbcopy", args: &[] },
    Helper { name: "clip.exe", args: &[] },
];

/// Which path a yank took (ADR 0018), for the status line to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyPath {
    /// Piped to a local helper (its name), the real-clipboard win.
    Helper(&'static str),
    /// Fell back to the OSC 52 escape (no helper, or the helper failed).
    Osc52,
}

impl CopyPath {
    /// The parenthesised suffix appended to the yank status (ADR 0018): the helper
    /// name on success, or an OSC 52 note that paste may need terminal support — so
    /// a silent no-op becomes a visible, explained outcome.
    pub fn status_suffix(&self) -> String {
        match self {
            CopyPath::Helper(name) => format!("({name})"),
            CopyPath::Osc52 => "(OSC 52 — paste may need terminal support)".to_string(),
        }
    }
}

/// Deliver `text` to the clipboard (ADR 0018): the real wiring — probe PATH for
/// each helper, pipe to the first present, and fall back to writing OSC 52 to
/// stdout. Returns the path taken so the caller can report it.
pub fn copy(text: &str) -> CopyPath {
    copy_via(text, HELPERS, |name| which(name).is_some(), run_helper, write_osc52)
}

/// The pure selection + dispatch (ADR 0018): pipe `text` to the first helper that
/// is `available`, via `run`; if that helper fails — or none is available — invoke
/// `osc52` and report the fallback. Only the *first present* helper is tried (a
/// helper that exists but fails falls back to OSC 52, it does not try the next).
pub fn copy_via(
    text: &str,
    helpers: &[Helper],
    available: impl Fn(&str) -> bool,
    run: impl Fn(&Helper, &str) -> bool,
    osc52: impl FnOnce(&str),
) -> CopyPath {
    for helper in helpers {
        if available(helper.name) {
            if run(helper, text) {
                return CopyPath::Helper(helper.name);
            }
            // Present but failed (e.g. no running compositor): fall back, do not
            // try the next helper.
            break;
        }
    }
    osc52(text);
    CopyPath::Osc52
}

/// Locate `name` on PATH (a tiny `which`): the first directory holding a file by
/// that name. A pure-Rust lookup — no `which` crate, no FFI.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// Spawn `helper` and pipe `text` to its stdin (ADR 0018), returning whether it
/// accepted the bytes and exited cleanly. The helpers fork to hold the selection
/// and then exit, so the wait returns. Any spawn/write/exit error is a failure the
/// caller turns into an OSC 52 fallback.
fn run_helper(helper: &Helper, text: &str) -> bool {
    let Ok(mut child) = Command::new(helper.name)
        .args(helper.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(text.as_bytes()).is_err() {
            return false;
        }
        // Drop stdin to send EOF so the helper finishes reading.
    }
    matches!(child.wait(), Ok(status) if status.success())
}

/// Write the OSC 52 clipboard escape to stdout (ADR 0015, the fallback): the
/// terminal sets the system clipboard from it. Best-effort.
fn write_osc52(text: &str) {
    let mut out = std::io::stdout();
    let _ = write!(out, "{}", super::effect::osc52(text));
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn helpers() -> &'static [Helper] {
        HELPERS
    }

    #[test]
    fn the_first_available_helper_is_used_and_osc52_is_not() {
        let osc_called = Cell::new(false);
        let path = copy_via(
            "payload",
            helpers(),
            |name| name == "wl-copy", // only wl-copy present
            |_, _| true,              // it succeeds
            |_| osc_called.set(true),
        );
        assert_eq!(path, CopyPath::Helper("wl-copy"));
        assert!(!osc_called.get(), "the helper succeeded, so no OSC 52");
    }

    #[test]
    fn probing_skips_absent_helpers_in_order() {
        // wl-copy absent, xclip present → xclip is chosen.
        let path = copy_via(
            "payload",
            helpers(),
            |name| name == "xclip",
            |_, _| true,
            |_| {},
        );
        assert_eq!(path, CopyPath::Helper("xclip"));
    }

    #[test]
    fn no_helper_present_falls_back_to_osc52() {
        let osc_payload: Cell<Option<String>> = Cell::new(None);
        let path = copy_via(
            "the bytes",
            helpers(),
            |_| false, // nothing present
            |_, _| true,
            |text| osc_payload.set(Some(text.to_string())),
        );
        assert_eq!(path, CopyPath::Osc52);
        assert_eq!(osc_payload.into_inner().as_deref(), Some("the bytes"), "exact payload to OSC 52");
    }

    #[test]
    fn a_present_helper_that_fails_falls_back_to_osc52() {
        let osc_called = Cell::new(false);
        let path = copy_via(
            "payload",
            helpers(),
            |name| name == "wl-copy", // present...
            |_, _| false,             // ...but it fails
            |_| osc_called.set(true),
        );
        assert_eq!(path, CopyPath::Osc52, "a failing helper falls back");
        assert!(osc_called.get());
    }

    #[test]
    fn the_status_suffix_reports_the_path() {
        assert_eq!(CopyPath::Helper("wl-copy").status_suffix(), "(wl-copy)");
        assert!(CopyPath::Osc52.status_suffix().contains("OSC 52"));
    }
}
