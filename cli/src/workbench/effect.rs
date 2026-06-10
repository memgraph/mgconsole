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
    /// Persist the Named-query store to its backing file after a `:save`/`:forget`
    /// mutated it in the reducer (issue 13). The reducer stays pure; the edge does
    /// the write and reports any failure.
    PersistQueries,
    /// Leave the workbench and restore the terminal.
    Quit,
}
