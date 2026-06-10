//! The tool-managed `~/.mgconsole/queries.toml` store of Named queries (issue 13).
//!
//! A Named query is a query saved under a name for later recall (CONTEXT.md). The
//! store keeps **text only** — a reusable template that keeps its `$param`
//! placeholders and resolves them against the *current* `:param` store at run
//! time, never a frozen snapshot. It lives in its own file, separate from the
//! hand-edited `config.toml`, precisely *because* the tool rewrites it on every
//! `:save`/`:forget`: a tool that rewrote `config.toml` would clobber the user's
//! comments (ADR 0012, CONTEXT.md).
//!
//! Where the file lives is decided by [`resolve_queries_path`] — a pure function
//! of the `MGCONSOLE_QUERIES_PATH` override and the home directory, mirroring the
//! config and history resolvers — so precedence is tested without the filesystem.
//! The pure [`parse`]/[`serialize`] pair carries the TOML shape and is tested
//! directly; [`NamedQueries`] folds them over a path, persisting on mutation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The environment variable that overrides the saved-queries file location,
/// alongside `MGCONSOLE_CONFIG_PATH` and `MGCONSOLE_HISTORY_PATH`.
pub const QUERIES_ENV: &str = "MGCONSOLE_QUERIES_PATH";

/// The home-relative state directory (ADR 0012); the queries file lives inside it.
const DEFAULT_SUBDIR: &str = ".mgconsole";

/// The tool-managed saved-queries file kept inside the state directory.
pub const QUERIES_FILENAME: &str = "queries.toml";

/// Resolve the saved-queries file path: the `MGCONSOLE_QUERIES_PATH` override wins
/// and is taken literally (the shell expands a typed `~`); otherwise the default
/// is `~/.mgconsole/queries.toml` expanded against `home`. With neither an
/// override nor a known home there is no path (`None`), and saving is refused with
/// a clear message rather than writing somewhere surprising.
pub fn resolve_queries_path(env: Option<&str>, home: Option<&Path>) -> Option<PathBuf> {
    match (env, home) {
        (Some(path), _) => Some(PathBuf::from(path)),
        (None, Some(home)) => Some(home.join(DEFAULT_SUBDIR).join(QUERIES_FILENAME)),
        (None, None) => None,
    }
}

/// The on-disk shape of `queries.toml`: a single `[queries]` table mapping each
/// name to its template text. A `BTreeMap` keeps the file in stable, name-sorted
/// order so a rewrite produces a minimal, reviewable diff.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQueries {
    #[serde(default)]
    queries: BTreeMap<String, String>,
}

/// Parse `queries.toml` text into the name→template map. An empty file is an empty
/// store; malformed TOML is a clear error the caller reports.
fn parse(text: &str) -> Result<BTreeMap<String, String>, String> {
    let raw: RawQueries = toml::from_str(text).map_err(|e| e.message().to_string())?;
    Ok(raw.queries)
}

/// Serialize the name→template map back to `queries.toml` text. The output is the
/// canonical `[queries]` table, name-sorted by the `BTreeMap`.
fn serialize(entries: &BTreeMap<String, String>) -> String {
    // A tool-managed file: a fixed header marks it as ours so a user who opens it
    // knows the tool rewrites it (and edits belong in config.toml instead).
    let body = toml::to_string(&RawQueries {
        queries: entries.clone(),
    })
    .unwrap_or_default();
    format!("# Managed by mgconsole — edited by :save/:forget, not by hand.\n{body}")
}

/// The Named-query store: the name→template map and the file it persists to.
///
/// Mutations ([`set`](Self::set)/[`remove`](Self::remove)) change the in-memory
/// map; [`persist`](Self::persist) writes the whole file. The REPL persists
/// directly; the Workbench mutates in its pure reducer and persists via an effect,
/// so both share this one store and `:save` means the same thing in each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedQueries {
    /// `None` when no writable location is known (no override, no home): the store
    /// still works in-memory for the session, but `:save`/`:forget` report that
    /// nothing can be persisted rather than guessing a path.
    path: Option<PathBuf>,
    entries: BTreeMap<String, String>,
}

impl NamedQueries {
    /// Load the store from `path`. A missing file is the normal first-run case (an
    /// empty store, not an error); a read or parse error is a clear, path-naming
    /// `Err` the caller reports before continuing on an empty store.
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    path: Some(path),
                    entries: BTreeMap::new(),
                });
            }
            Err(e) => return Err(format!("could not read saved queries {}: {e}", path.display())),
        };
        let entries =
            parse(&text).map_err(|e| format!("invalid saved queries {}: {e}", path.display()))?;
        Ok(Self {
            path: Some(path),
            entries,
        })
    }

    /// An in-memory store with no backing file — for the no-home case and for tests
    /// that exercise the verbs without touching the filesystem.
    #[must_use]
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// The template saved under `name`, or `None` if no such name is saved.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.entries.get(name).map(String::as_str)
    }

    /// The saved names, in name-sorted order (the `BTreeMap`'s order).
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    /// Whether the store holds no saved queries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Insert or replace the template for `name`, in memory only. Persisting is a
    /// separate step so the pure Workbench reducer can mutate and defer the write.
    pub fn set(&mut self, name: String, query: String) {
        self.entries.insert(name, query);
    }

    /// Remove the template for `name`, in memory only; returns whether it existed
    /// (so a `:forget` of an unknown name can be reported as such).
    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    /// The whole store serialized to `queries.toml` text (the pure form the
    /// Workbench effect writes).
    #[must_use]
    pub fn serialized(&self) -> String {
        serialize(&self.entries)
    }

    /// The backing file path, if a writable location is known.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Write the whole store to its backing file, creating the state directory if
    /// needed. With no known path (no override, no home) this is a clear error, so
    /// `:save` says it cannot persist rather than silently dropping the query.
    pub fn persist(&self) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("no writable location for saved queries (set HOME or \
                        MGCONSOLE_QUERIES_PATH)"
                .to_string());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        std::fs::write(path, self.serialized())
            .map_err(|e| format!("could not write saved queries {}: {e}", path.display()))
    }

    /// A multi-line listing for `:saved`: one name per line, or a clear sentence
    /// when nothing is saved.
    #[must_use]
    pub fn list(&self) -> String {
        if self.entries.is_empty() {
            return "No saved queries.".to_string();
        }
        self.names().join("\n")
    }

    /// A one-line summary for the Workbench status bar (which has no multi-line
    /// listing): the saved names joined by commas, or a clear sentence when empty.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.entries.is_empty() {
            return "No saved queries.".to_string();
        }
        format!("saved: {}", self.names().join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/user")
    }

    #[test]
    fn the_default_path_is_under_the_state_directory() {
        assert_eq!(
            resolve_queries_path(None, Some(&home())),
            Some(PathBuf::from("/home/user/.mgconsole/queries.toml"))
        );
    }

    #[test]
    fn the_environment_override_wins_and_is_literal() {
        assert_eq!(
            resolve_queries_path(Some("/tmp/q.toml"), Some(&home())),
            Some(PathBuf::from("/tmp/q.toml"))
        );
    }

    #[test]
    fn no_home_and_no_override_means_no_path() {
        assert_eq!(resolve_queries_path(None, None), None);
    }

    #[test]
    fn parse_reads_the_queries_table() {
        let entries = parse("[queries]\nrecent = \"MATCH (n) RETURN n LIMIT 10\"\n").expect("valid");
        assert_eq!(
            entries.get("recent").map(String::as_str),
            Some("MATCH (n) RETURN n LIMIT 10")
        );
    }

    #[test]
    fn parse_of_empty_or_tableless_text_is_empty() {
        assert!(parse("").expect("valid").is_empty());
        assert!(parse("[queries]\n").expect("valid").is_empty());
    }

    #[test]
    fn malformed_toml_is_a_clear_error() {
        assert!(parse("this is = = not toml").is_err());
    }

    #[test]
    fn serialize_round_trips_through_parse() {
        let mut entries = BTreeMap::new();
        entries.insert("a".to_string(), "RETURN 1".to_string());
        entries.insert("b".to_string(), "MATCH (n)\nRETURN n".to_string());
        let text = serialize(&entries);
        assert_eq!(parse(&text).expect("round-trips"), entries);
    }

    #[test]
    fn serialize_marks_the_file_as_tool_managed() {
        let text = serialize(&BTreeMap::new());
        assert!(text.contains("Managed by mgconsole"), "header present: {text}");
    }

    #[test]
    fn set_get_remove_and_names_track_the_store() {
        let mut q = NamedQueries::in_memory();
        assert!(q.is_empty());
        q.set("recent".to_string(), "RETURN 1".to_string());
        q.set("count".to_string(), "MATCH (n) RETURN count(n)".to_string());
        assert_eq!(q.get("recent"), Some("RETURN 1"));
        // BTreeMap order: count before recent.
        assert_eq!(q.names(), vec!["count", "recent"]);
        assert!(q.remove("recent"));
        assert!(!q.remove("recent"), "removing again reports it was absent");
        assert_eq!(q.get("recent"), None);
    }

    #[test]
    fn set_replaces_an_existing_template() {
        let mut q = NamedQueries::in_memory();
        q.set("x".to_string(), "RETURN 1".to_string());
        q.set("x".to_string(), "RETURN 2".to_string());
        assert_eq!(q.get("x"), Some("RETURN 2"));
        assert_eq!(q.names().len(), 1);
    }

    #[test]
    fn listing_reads_clearly_when_empty_and_lists_when_not() {
        let mut q = NamedQueries::in_memory();
        assert_eq!(q.list(), "No saved queries.");
        assert_eq!(q.summary(), "No saved queries.");
        q.set("a".to_string(), "RETURN 1".to_string());
        q.set("b".to_string(), "RETURN 2".to_string());
        assert_eq!(q.list(), "a\nb");
        assert_eq!(q.summary(), "saved: a, b");
    }

    #[test]
    fn persist_without_a_path_is_a_clear_error() {
        let q = NamedQueries::in_memory();
        assert!(q.persist().expect_err("no path").contains("no writable location"));
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_store() {
        let q = NamedQueries::load(PathBuf::from("/no/such/dir/queries.toml")).expect("missing ok");
        assert!(q.is_empty());
    }

    #[test]
    fn save_load_and_forget_round_trip_through_a_real_file() {
        // The one filesystem test: the store persists and a fresh load sees it, and
        // a forget rewrite preserves the other entries (acceptance criterion 2).
        let dir = std::env::temp_dir().join(format!("mgconsole-queries-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("queries.toml");

        let mut q = NamedQueries::load(path.clone()).expect("fresh load");
        q.set("a".to_string(), "RETURN 1".to_string());
        q.set("b".to_string(), "RETURN 2".to_string());
        q.persist().expect("persist");

        let mut reloaded = NamedQueries::load(path.clone()).expect("reload");
        assert_eq!(reloaded.get("a"), Some("RETURN 1"));
        assert_eq!(reloaded.get("b"), Some("RETURN 2"));

        // Forgetting one and rewriting preserves the other.
        assert!(reloaded.remove("a"));
        reloaded.persist().expect("rewrite");
        let again = NamedQueries::load(path).expect("reload again");
        assert_eq!(again.get("a"), None);
        assert_eq!(again.get("b"), Some("RETURN 2"), "other entry preserved");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
