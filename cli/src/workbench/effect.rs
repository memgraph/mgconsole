//! The side-effects the reducer requests of its IO edge.
//!
//! [`super::update`] is pure: it mutates the [`WorkbenchState`](super::WorkbenchState)
//! and returns the IO to perform as a `Vec<Effect>`, the analogue of how the
//! REPL's loop hands query execution to its `QueryRunner` seam. The edge
//! ([`super::run`]) interprets each effect. Slice 07 adds `Cancel`.
//!
//! Not `Eq`: `RunQuery` carries the bound parameters (`Value` has a float, so it
//! is `PartialEq` only) — enough to assert effects in the reducer tests.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mgconsole_core::{Record, Value};

use crate::OutputFormat;

/// Cycle the export prompt through the streaming formats only (issue 12): the
/// on-screen export targets are `csv | jsonl | cypherl`; `table` is the buffered
/// REPL/screen layout and not an export target, so it is skipped.
#[must_use]
pub fn next_export_format(format: OutputFormat) -> OutputFormat {
    match format {
        OutputFormat::Csv => OutputFormat::Jsonl,
        OutputFormat::Jsonl => OutputFormat::Cypherl,
        // Cypherl wraps round; `table` (never offered) normalises to csv.
        OutputFormat::Cypherl | OutputFormat::Table => OutputFormat::Csv,
    }
}

/// An explicit-transaction operation requested by `:begin`/`:commit`/`:rollback`
/// (issue 05). The edge applies it to the shared Session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxOp {
    Begin,
    Commit,
    Rollback,
}

impl TxOp {
    /// The confirmation shown when the operation succeeds.
    pub fn success_message(self) -> &'static str {
        match self {
            TxOp::Begin => "transaction open",
            TxOp::Commit => "transaction committed",
            TxOp::Rollback => "transaction rolled back",
        }
    }
}

/// An IO action for the edge to perform.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Run a query on the Session as a cancellable task, tagged with `id` so its
    /// lifecycle events can be matched back. `params` are the bound `:param`
    /// values (empty until slice 16).
    RunQuery {
        id: u64,
        query: String,
        params: BTreeMap<String, Value>,
    },
    /// Cancel the in-flight query `id`: abort its task and recover the Session
    /// via Bolt `RESET` (ADR 0005). Rows already streamed stay on screen.
    Cancel { id: u64 },
    /// Fetch the database Schema (on connect and on manual refresh, slice 12).
    FetchSchema,
    /// Evaluate a `:param` expression server-side with the current params in
    /// scope, then store the result as `$name` (slice 16).
    EvaluateParam {
        name: String,
        expr: String,
        params: BTreeMap<String, Value>,
    },
    /// Write the on-screen result to `path` in `format`, reusing the Core's
    /// streaming writers (slice 09). Carries the loaded rows (including a partial
    /// result after a cancel), so the export reflects exactly what is shown.
    Export {
        format: OutputFormat,
        path: PathBuf,
        header: Vec<String>,
        rows: Vec<Record>,
    },
    /// Append a submitted query to the persisted command history (slice 17).
    AppendHistory(String),
    /// Turn read-only mode on the Session (issue 04): the next query carries Bolt
    /// access mode READ. Only ever `true` — the off direction is refused at runtime.
    SetReadOnly(bool),
    /// Apply an explicit-transaction operation to the Session (issue 05).
    Transaction(TxOp),
    /// Swap to a new Session at the given `:connect` target (issue 07): a profile
    /// name or `host[:port]`.
    Connect(String),
    /// Switch the active Database within the Session (`:use`, issue 08).
    UseDatabase(String),
    /// Read a file for `:source` (issue 10); its statements then run as a
    /// stop-on-error batch.
    Source(PathBuf),
    /// Run `query` and stream its result to a file (`:o`, issue 12) instead of the
    /// result pane; `id` matches the lifecycle like a normal run.
    RunQueryToFile {
        id: u64,
        query: String,
        params: BTreeMap<String, Value>,
        format: OutputFormat,
        path: PathBuf,
    },
    /// Reconcile one Named-query file with the in-memory mirror after a
    /// `:save`/`:forget` mutated it in the reducer (issue 23, ADR 0020): the named
    /// query is written as `<name>.cypher` (save) or its file deleted (forget). The
    /// reducer stays pure; the edge does the single-file write/delete and reports
    /// any failure. One file per effect keeps a crash to at most one bad file.
    PersistQuery(String),
    /// Copy rendered text to the system clipboard via an OSC 52 escape (issue 04 /
    /// ADR 0015): the edge base64-encodes the payload and writes the sequence to
    /// the terminal — pure bytes, no native clipboard dependency, works over SSH.
    CopyToClipboard(String),
    /// Turn the terminal's mouse capture on or off (issue 04): `:set mouse off`
    /// releases capture so native click-drag selection works; `:set mouse on`
    /// re-enables the Workbench's mouse gestures.
    SetMouseCapture(bool),
    /// Leave the workbench and restore the terminal.
    Quit,
}

/// Build the OSC 52 clipboard escape sequence carrying `text` (issue 04 / ADR
/// 0015): `ESC ] 52 ; c ; <base64> BEL`. The terminal — not the application —
/// sets the system clipboard from it, so it rides the same channel as the rest of
/// the TUI and works through SSH and `tmux`. Pure bytes; no FFI (ADR 0001).
#[must_use]
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64_encode(text.as_bytes()))
}

/// Minimal standard-alphabet base64 encoder (issue 04): a few lines of pure Rust
/// rather than a dependency, since OSC 52 is the only base64 the tool needs.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let n = b0 << 16 | b1 << 8 | b2;
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6 & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        // The classic RFC 4648 examples, including the two padding cases.
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn osc52_frames_the_base64_payload() {
        // ESC ] 52 ; c ; <base64> BEL — the terminal sets the clipboard from this.
        assert_eq!(osc52("foo"), "\x1b]52;c;Zm9v\x07");
    }
}
