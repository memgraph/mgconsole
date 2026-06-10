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

/// The on-screen result can be exported in any of the Core's streaming formats
/// (slice 09); tabular is the buffered REPL format and not an export target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Jsonl,
    Cypherl,
}

impl ExportFormat {
    /// Cycle to the next format (the export prompt's Tab).
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            ExportFormat::Csv => ExportFormat::Jsonl,
            ExportFormat::Jsonl => ExportFormat::Cypherl,
            ExportFormat::Cypherl => ExportFormat::Csv,
        }
    }

    /// The format's display name.
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Jsonl => "jsonl",
            ExportFormat::Cypherl => "cypherl",
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
        format: ExportFormat,
        path: PathBuf,
        header: Vec<String>,
        rows: Vec<Record>,
    },
    /// Leave the workbench and restore the terminal.
    Quit,
}
