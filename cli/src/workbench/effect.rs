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

use mgconsole_core::Value;

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
    /// Leave the workbench and restore the terminal.
    Quit,
}
