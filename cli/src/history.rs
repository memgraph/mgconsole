//! Persistent REPL command history (slice 17).
//!
//! Where history lives is decided by [`resolve_history_file`] — a pure function
//! of the `--history` flag, the `MGCONSOLE_HISTORY_PATH` environment override,
//! the `--no-history` switch, and the user's home directory — so the precedence
//! rules are tested without touching the filesystem. The directory is then
//! created by [`prepare_history_dir`], and [`HistoryFile`] loads prior entries
//! and persists each new one. History is a directory holding a `client_history`
//! file, the env override wins over the flag, and the bare default `~/.mgconsole`
//! expands against the home directory (ADR 0012: a clean break from mgconsole's
//! `~/.memgraph`, all console state under one `~/.mgconsole`).

use std::path::{Path, PathBuf};

use rustyline::history::History;

/// The environment variable that overrides the history location.
pub const HISTORY_ENV: &str = "MGCONSOLE_HISTORY_PATH";

/// The default `--history` value: the `~/.mgconsole` state directory (ADR 0012).
pub const DEFAULT_HISTORY_DIR: &str = "~/.mgconsole";

/// The home-relative directory the bare default expands to.
const DEFAULT_SUBDIR: &str = ".mgconsole";

/// The history file kept inside the history directory.
pub const HISTORY_FILENAME: &str = "client_history";

/// Resolve the history file path, or `None` when history is disabled.
///
/// Precedence: the `MGCONSOLE_HISTORY_PATH` environment override wins; otherwise
/// the `--history` directory is used (its default is `~/.mgconsole`, ADR 0012).
/// The resolved directory holds a `client_history` file. Only
/// the bare default expands its leading `~` against `home`; any other value —
/// flag or env — is taken literally (the shell expands a typed `~`).
pub fn resolve_history_file(
    flag: &str,
    env: Option<&str>,
    no_history: bool,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if no_history {
        return None;
    }
    let configured = env.unwrap_or(flag);
    let dir = match (configured, home) {
        (DEFAULT_HISTORY_DIR, Some(home)) => home.join(DEFAULT_SUBDIR),
        _ => PathBuf::from(configured),
    };
    Some(dir.join(HISTORY_FILENAME))
}

/// Ensure the directory that will hold the history file exists, returning a
/// clear, path-naming message if it cannot be created (acceptance: a missing or
/// un-creatable history directory is handled cleanly).
pub fn prepare_history_dir(file: &Path) -> Result<(), String> {
    let Some(dir) = file.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("could not create history directory {}: {e}", dir.display()))
}

/// A history file the REPL reads on start and appends to as queries are entered.
/// Thin glue over a rustyline [`History`]; persistence is exercised directly in
/// tests (no terminal needed).
pub struct HistoryFile {
    path: PathBuf,
}

impl HistoryFile {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Load prior entries into `history`. A missing file is the normal first-run
    /// case, not an error.
    pub fn load(&self, history: &mut dyn History) {
        if self.path.exists() {
            let _ = history.load(&self.path);
        }
    }

    /// Record one entered line and persist it immediately, so history survives a
    /// crash mid-session. Blank lines are not stored.
    pub fn record(&self, history: &mut dyn History, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        let _ = history.add(line);
        let _ = history.save(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyline::history::{FileHistory, SearchDirection};

    fn home() -> PathBuf {
        PathBuf::from("/home/user")
    }

    #[test]
    fn the_default_expands_against_the_home_directory() {
        let path = resolve_history_file(DEFAULT_HISTORY_DIR, None, false, Some(&home()))
            .expect("history enabled");
        assert_eq!(path, PathBuf::from("/home/user/.mgconsole/client_history"));
    }

    #[test]
    fn an_explicit_flag_directory_is_used_literally() {
        let path =
            resolve_history_file("/tmp/hist", None, false, Some(&home())).expect("history enabled");
        assert_eq!(path, PathBuf::from("/tmp/hist/client_history"));
    }

    #[test]
    fn the_environment_override_beats_the_flag() {
        let path = resolve_history_file("/tmp/hist", Some("/env/place"), false, Some(&home()))
            .expect("history enabled");
        assert_eq!(path, PathBuf::from("/env/place/client_history"));
    }

    #[test]
    fn the_environment_override_beats_the_default() {
        let path = resolve_history_file(
            DEFAULT_HISTORY_DIR,
            Some("/env/place"),
            false,
            Some(&home()),
        )
        .expect("history enabled");
        assert_eq!(path, PathBuf::from("/env/place/client_history"));
    }

    #[test]
    fn no_history_disables_history_entirely() {
        assert_eq!(
            resolve_history_file(DEFAULT_HISTORY_DIR, Some("/env/place"), true, Some(&home())),
            None
        );
    }

    #[test]
    fn an_uncreatable_history_directory_yields_a_clear_message() {
        // Put a regular file where a directory component must go, so creating the
        // directory underneath it fails.
        let blocker = std::env::temp_dir().join("mg_hist_blocker_file");
        std::fs::write(&blocker, b"x").expect("write blocker file");
        let file = blocker.join("sub").join(HISTORY_FILENAME);

        let err = prepare_history_dir(&file).expect_err("creating under a file must fail");
        assert!(
            err.contains("could not create history directory"),
            "message: {err}"
        );
        assert!(err.contains("sub"), "message names the directory: {err}");

        std::fs::remove_file(&blocker).ok();
    }

    #[test]
    fn a_creatable_history_directory_is_prepared() {
        let dir = std::env::temp_dir().join("mg_hist_prepare_ok");
        std::fs::remove_dir_all(&dir).ok();
        let file = dir.join(HISTORY_FILENAME);
        prepare_history_dir(&file).expect("directory is created");
        assert!(dir.is_dir());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn entries_are_saved_and_recalled_in_a_later_session() {
        let dir = std::env::temp_dir().join("mg_hist_roundtrip");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("temp history dir");
        let store = HistoryFile::new(dir.join(HISTORY_FILENAME));

        // First session: enter two queries (a blank line is not stored).
        let mut session_one = FileHistory::new();
        store.record(&mut session_one, "RETURN 1;");
        store.record(&mut session_one, "   ");
        store.record(&mut session_one, "MATCH (n) RETURN n;");

        // Later session: a fresh history loads what was saved.
        let mut session_two = FileHistory::new();
        store.load(&mut session_two);
        assert_eq!(session_two.len(), 2, "two non-blank queries persisted");
        let first = session_two
            .get(0, SearchDirection::Forward)
            .expect("read ok")
            .expect("an entry")
            .entry
            .into_owned();
        assert_eq!(first, "RETURN 1;");

        std::fs::remove_dir_all(&dir).ok();
    }
}
