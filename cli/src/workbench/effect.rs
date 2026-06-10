//! The side-effects the reducer requests of its IO edge.
//!
//! [`super::update`] is pure: it mutates the [`WorkbenchState`](super::WorkbenchState)
//! and returns the IO to perform as a `Vec<Effect>`, the analogue of how the
//! REPL's loop hands query execution to its `QueryRunner` seam. The edge
//! ([`super::run`]) interprets each effect. Slice 02 adds the execution effects
//! (`RunQuery`, `Cancel`); slice 01 needs only the quit signal.

/// An IO action for the edge to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Leave the workbench and restore the terminal.
    Quit,
}
