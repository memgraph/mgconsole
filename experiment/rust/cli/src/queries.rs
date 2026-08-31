//! Named queries as a directory of plain `.cypher` files (issue 23, ADR 0020).
//!
//! A Named query is a query saved under a name for later recall (CONTEXT.md).
//! Each one is one plain `.cypher` file (`<name>.cypher`) in a tool-owned
//! `~/.mgconsole/queries/` directory; the **directory is the source of truth**,
//! there is no separate index. Saving writes a file, `:forget` deletes one, and
//! `:saved` lists by reading the directory. Because the files are plain Cypher,
//! the same directory doubles as a `:source` library — a saved query *is* a
//! loadable script, one format not two.
//!
//! The store keeps **text only** — a reusable template that keeps its `$param`
//! placeholders and resolves them against the *current* `:param` store at run
//! time, never a frozen snapshot. Per ADR 0020 the console treats the directory as
//! authoritative and parses leniently: a hand-edited or oddly-named file is
//! tolerated and reported (see [`NamedQueries::warnings`]), never fatal. Save and
//! delete operate on a single file, so a crash mid-write can leave at most one bad
//! file, never a corrupt index.
//!
//! Where the directory lives is decided by [`resolve_queries_dir`] — a pure
//! function of the `MGCONSOLE_QUERIES_PATH` override and the home directory,
//! mirroring the config and history resolvers — so precedence is tested without
//! the filesystem. (ADR 0020 superseded the previous single `queries.toml` file;
//! `MGCONSOLE_QUERIES_PATH` now names the *directory*. The old file path is gone
//! on purpose — do not restore it.)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The environment variable that overrides the saved-queries *directory*
/// location, alongside `MGCONSOLE_CONFIG_PATH` and `MGCONSOLE_HISTORY_PATH`.
pub const QUERIES_ENV: &str = "MGCONSOLE_QUERIES_PATH";

/// The home-relative state directory (ADR 0012); the queries directory lives
/// inside it.
const DEFAULT_SUBDIR: &str = ".mgconsole";

/// The tool-owned saved-queries directory kept inside the state directory (ADR
/// 0020 superseded the single `queries.toml` file with this directory).
pub const QUERIES_SUBDIR: &str = "queries";

/// The extension every saved-query file carries (ADR 0020: plain Cypher, so the
/// directory doubles as a `:source` library).
pub const QUERY_EXT: &str = "cypher";

/// Resolve the saved-queries *directory* path: the `MGCONSOLE_QUERIES_PATH`
/// override wins and is taken literally (the shell expands a typed `~`); otherwise
/// the default is `~/.mgconsole/queries/` expanded against `home`. With neither an
/// override nor a known home there is no path (`None`), and saving is refused with
/// a clear message rather than writing somewhere surprising.
pub fn resolve_queries_dir(env: Option<&str>, home: Option<&Path>) -> Option<PathBuf> {
    match (env, home) {
        (Some(path), _) => Some(PathBuf::from(path)),
        (None, Some(home)) => Some(home.join(DEFAULT_SUBDIR).join(QUERIES_SUBDIR)),
        (None, None) => None,
    }
}

/// Whether `name` is a usable Named-query name: non-empty and a single path
/// component (no separators, not `.`/`..`), so it maps to exactly one file inside
/// the queries directory and can never escape it. Names come from `:save <name>`
/// (a single whitespace-free word), so this is a defensive guard against a
/// `../escape` rather than an everyday rejection.
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

/// The Named-query store: an in-memory mirror of the directory (name→template) and
/// the directory it persists to.
///
/// The mirror is loaded once from the directory at startup so the pure Workbench
/// reducer and the REPL can read names/templates without touching the filesystem;
/// [`sync_file`](Self::sync_file) writes or deletes the *single* file a `:save` or
/// `:forget` changed (never a whole-directory rewrite — ADR 0020's per-file
/// durability). Both Frontends share this one store, so `:save` means the same
/// thing in each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NamedQueries {
    /// `None` when no writable location is known (no override, no home): the store
    /// still works in-memory for the session, but `:save`/`:forget` report that
    /// nothing can be persisted rather than guessing a path.
    dir: Option<PathBuf>,
    entries: BTreeMap<String, String>,
    /// Lenient-parse notes from [`load`](Self::load): directory entries that were
    /// skipped (not a readable `.cypher` file). Surfaced by `:saved`, never fatal.
    warnings: Vec<String>,
}

impl NamedQueries {
    /// Load the store by reading every `<name>.cypher` file in `dir`. A missing
    /// directory is the normal first-run case (an empty store, not an error). Each
    /// readable `.cypher` file becomes one entry (name = file stem, template = file
    /// text with a single trailing newline trimmed). A directory entry that is not
    /// a readable `.cypher` file is recorded as a [`warning`](Self::warnings) and
    /// skipped — lenient parse (ADR 0020), never fatal. A hard failure to *list* an
    /// existing directory is the one error returned.
    pub fn load(dir: PathBuf) -> Result<Self, String> {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(read_dir) => read_dir,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    dir: Some(dir),
                    entries: BTreeMap::new(),
                    warnings: Vec::new(),
                });
            }
            Err(e) => return Err(format!("could not read queries directory {}: {e}", dir.display())),
        };
        let mut entries = BTreeMap::new();
        let mut warnings = Vec::new();
        for entry in read_dir {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            // Only plain `.cypher` files are saved queries; anything else (a
            // sub-directory, a stray `notes.txt`) is reported and skipped.
            if !path.is_file() {
                continue;
            }
            let is_cypher = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(QUERY_EXT));
            let name = path.file_stem().and_then(|s| s.to_str());
            match (is_cypher, name) {
                (true, Some(name)) if is_valid_name(name) => match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        entries.insert(name.to_string(), strip_one_trailing_newline(&text));
                    }
                    Err(e) => warnings.push(format!("skipped {}: {e}", path.display())),
                },
                _ => warnings.push(format!(
                    "ignoring '{}' (not a .{QUERY_EXT} query file)",
                    path.file_name().and_then(|n| n.to_str()).unwrap_or_default()
                )),
            }
        }
        Ok(Self {
            dir: Some(dir),
            entries,
            warnings,
        })
    }

    /// An in-memory store with no backing directory — for the no-home case and for
    /// tests that exercise the verbs without touching the filesystem.
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

    /// Lenient-parse warnings gathered when the directory was read (skipped
    /// entries). Empty in the common case.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Insert or replace the template for `name`, in the in-memory mirror only.
    /// Persisting the changed file is a separate step ([`sync_file`](Self::sync_file))
    /// so the pure Workbench reducer can mutate and defer the write to an effect.
    pub fn set(&mut self, name: String, query: String) {
        self.entries.insert(name, query);
    }

    /// Remove the template for `name` from the in-memory mirror; returns whether it
    /// existed (so a `:forget` of an unknown name can be reported as such). The file
    /// is removed separately by [`sync_file`](Self::sync_file).
    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    /// The backing directory, if a writable location is known.
    #[must_use]
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Reconcile the single file for `name` with the in-memory mirror, after a
    /// `:save` (mirror now holds `name` → write `<name>.cypher`) or a `:forget`
    /// (mirror no longer holds `name` → delete the file). One file per call keeps a
    /// crash mid-write to at most one bad file (ADR 0020). With no known directory
    /// (no override, no home) this is a clear error, so `:save` says it cannot
    /// persist rather than silently dropping the query.
    pub fn sync_file(&self, name: &str) -> Result<(), String> {
        let Some(dir) = &self.dir else {
            return Err("no writable location for saved queries (set HOME or \
                        MGCONSOLE_QUERIES_PATH)"
                .to_string());
        };
        if !is_valid_name(name) {
            return Err(format!("invalid query name '{name}'"));
        }
        let path = dir.join(format!("{name}.{QUERY_EXT}"));
        match self.entries.get(name) {
            // A save: ensure the directory, then write the template as plain Cypher
            // (a trailing newline so the file is well-formed for editors/`:source`).
            Some(template) => {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
                let body = if template.ends_with('\n') {
                    template.clone()
                } else {
                    format!("{template}\n")
                };
                std::fs::write(&path, body)
                    .map_err(|e| format!("could not write saved query {}: {e}", path.display()))
            }
            // A forget: remove the one file; an already-absent file is success.
            None => match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("could not delete saved query {}: {e}", path.display())),
            },
        }
    }

    /// A multi-line listing for `:saved`: one name per line, or a clear sentence
    /// when nothing is saved, followed by any lenient-parse warnings so an
    /// oddly-named file in the directory is reported (ADR 0020), never hidden.
    #[must_use]
    pub fn list(&self) -> String {
        let mut out = if self.entries.is_empty() {
            "No saved queries.".to_string()
        } else {
            self.names().join("\n")
        };
        for warning in &self.warnings {
            out.push_str("\nwarning: ");
            out.push_str(warning);
        }
        out
    }

    /// A one-line summary for the Workbench status bar (which has no multi-line
    /// listing): the saved names joined by commas, or a clear sentence when empty,
    /// with a trailing count of any skipped files so they are not hidden.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = if self.entries.is_empty() {
            "No saved queries.".to_string()
        } else {
            format!("saved: {}", self.names().join(", "))
        };
        if !self.warnings.is_empty() {
            use std::fmt::Write as _;
            let _ = write!(out, " ({} file(s) skipped)", self.warnings.len());
        }
        out
    }
}

/// Strip a single trailing newline (the one [`sync_file`](NamedQueries::sync_file)
/// adds, or the one a hand-editor leaves) so a recalled template matches what was
/// saved. Internal newlines and intentional blank lines are preserved.
fn strip_one_trailing_newline(text: &str) -> String {
    text.strip_suffix('\n').unwrap_or(text).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/user")
    }

    /// A fresh temp directory unique to this test name and process.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mgconsole-queries-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_default_path_is_the_queries_directory_under_the_state_directory() {
        assert_eq!(
            resolve_queries_dir(None, Some(&home())),
            Some(PathBuf::from("/home/user/.mgconsole/queries"))
        );
    }

    #[test]
    fn the_environment_override_wins_and_is_literal() {
        assert_eq!(
            resolve_queries_dir(Some("/tmp/q"), Some(&home())),
            Some(PathBuf::from("/tmp/q"))
        );
    }

    #[test]
    fn no_home_and_no_override_means_no_path() {
        assert_eq!(resolve_queries_dir(None, None), None);
    }

    #[test]
    fn names_with_path_separators_are_rejected() {
        assert!(is_valid_name("recent"));
        assert!(is_valid_name("schema-probe"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("."));
        assert!(!is_valid_name(".."));
        assert!(!is_valid_name("../escape"));
        assert!(!is_valid_name("a/b"));
        assert!(!is_valid_name("a\\b"));
    }

    #[test]
    fn set_get_remove_and_names_track_the_in_memory_mirror() {
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
    fn sync_without_a_directory_is_a_clear_error() {
        let mut q = NamedQueries::in_memory();
        q.set("a".to_string(), "RETURN 1".to_string());
        assert!(q.sync_file("a").expect_err("no dir").contains("no writable location"));
    }

    #[test]
    fn a_missing_directory_loads_as_an_empty_store() {
        let q = NamedQueries::load(PathBuf::from("/no/such/dir/queries")).expect("missing ok");
        assert!(q.is_empty());
        assert!(q.warnings().is_empty());
    }

    #[test]
    fn save_writes_one_plain_cypher_file_loadable_as_a_script() {
        // Acceptance: `:save` writes <name>.cypher as plain Cypher (no TOML wrapper),
        // so the same file is readable by `:source` — one format, not two.
        let dir = temp_dir("plain");
        let mut q = NamedQueries::load(dir.clone()).expect("fresh load");
        q.set("recent".to_string(), "MATCH (n) RETURN $limit".to_string());
        q.sync_file("recent").expect("write");

        let file = dir.join("recent.cypher");
        let on_disk = std::fs::read_to_string(&file).expect("read back");
        // Plain Cypher with a trailing newline — exactly what `:source` would run.
        assert_eq!(on_disk, "MATCH (n) RETURN $limit\n");
        assert!(!on_disk.contains("[queries]"), "no TOML wrapper");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_load_and_forget_round_trip_per_file() {
        // The filesystem test: each save is its own file, a fresh load reads the
        // directory, and a forget deletes only that file (others untouched).
        let dir = temp_dir("roundtrip");
        let mut q = NamedQueries::load(dir.clone()).expect("fresh load");
        q.set("a".to_string(), "RETURN 1".to_string());
        q.sync_file("a").expect("write a");
        q.set("b".to_string(), "MATCH (n)\nRETURN n".to_string());
        q.sync_file("b").expect("write b");

        // A fresh load sees both, with multi-line templates intact.
        let reloaded = NamedQueries::load(dir.clone()).expect("reload");
        assert_eq!(reloaded.get("a"), Some("RETURN 1"));
        assert_eq!(reloaded.get("b"), Some("MATCH (n)\nRETURN n"));
        assert_eq!(reloaded.names(), vec!["a", "b"]);

        // Forgetting one deletes its file and leaves the other.
        let mut q = reloaded;
        assert!(q.remove("a"));
        q.sync_file("a").expect("delete a");
        assert!(!dir.join("a.cypher").exists(), "a.cypher removed");
        assert!(dir.join("b.cypher").exists(), "b.cypher preserved");

        let again = NamedQueries::load(dir.clone()).expect("reload again");
        assert_eq!(again.get("a"), None);
        assert_eq!(again.get("b"), Some("MATCH (n)\nRETURN n"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_oddly_named_file_is_reported_not_fatal() {
        // Lenient parse (ADR 0020): a stray non-.cypher file is warned about on
        // `:saved`, the real queries still load.
        let dir = temp_dir("lenient");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("good.cypher"), "RETURN 1\n").expect("write good");
        std::fs::write(dir.join("notes.txt"), "just notes").expect("write stray");

        let q = NamedQueries::load(dir.clone()).expect("loads despite the stray file");
        assert_eq!(q.get("good"), Some("RETURN 1"));
        assert_eq!(q.names(), vec!["good"], "the stray file is not a query");
        assert_eq!(q.warnings().len(), 1, "the stray file is reported");
        assert!(q.warnings()[0].contains("notes.txt"), "{:?}", q.warnings());
        // `:saved` surfaces the warning rather than hiding it.
        assert!(q.list().contains("notes.txt"));
        assert!(q.summary().contains("skipped"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
