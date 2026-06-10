//! Core library for the Rust Memgraph console.
//!
//! Owns the Session, the Value model, value rendering, and (later) the import
//! engine, with no dependency on any Frontend (ADR 0002). `bolt_proto::Value` is
//! translated into the Core [`Value`] at the Bolt boundary (ADR 0003); the
//! Session yields a [`QueryResult`] of a header, a [`RecordStream`], and a
//! trailing [`Summary`] (ADR 0004).

pub mod clause;
pub mod error;
pub mod format;
pub mod import;
pub mod parse;
mod proto;
pub mod render;
pub mod result;
pub mod session;
pub mod tabular;
mod transport;
pub mod value;

pub use clause::{scan_clauses, Clause};
pub use error::Error;
pub use import::{
    run_parser, run_serial, ImportFailure, ImportReport, OutputFormat as ImportFormat, ParsedQuery,
    ParserReport,
};
pub use parse::QueryAssembler;
pub use result::{ExecutionInfo, Notification, QueryResult, Record, RecordStream, Summary};
pub use tabular::{render_table, TableOptions, DEFAULT_ROW_CAP};
pub use session::{ConnectOptions, Credentials, ReconnectNotice, Session};
pub use value::Value;
