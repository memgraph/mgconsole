//! Core library for the Rust Memgraph console.
//!
//! Owns the Session, the Value model, value rendering, and (later) the import
//! engine, with no dependency on any Frontend (ADR 0002). `bolt_proto::Value` is
//! translated into the Core [`Value`] at the Bolt boundary (ADR 0003); the
//! Session yields a [`QueryResult`] of a header, a [`RecordStream`], and a
//! trailing [`Summary`] (ADR 0004).

pub mod clause;
pub mod display;
pub mod error;
pub mod format;
pub mod import;
pub mod lexer;
pub mod parse;
mod proto;
pub mod render;
pub mod result;
pub mod session;
pub mod tabular;
mod transport;
pub mod value;
pub mod workers;

pub use clause::{scan_clauses, Clause};
pub use display::{
    render_records, render_vertical, resolve_layout, DisplayMode, Layout, RenderOptions,
};
pub use error::Error;
pub use format::Header;
pub use lexer::{lex, Token, TokenKind};
pub use import::{
    classify_phase, into_batches, run_parallel, run_parallel_ordered, run_parser, run_serial,
    Batch, ImportFailure, ImportReport, OutputFormat as ImportFormat, ParallelReport, ParsedQuery,
    ParserReport, Phase,
};
pub use parse::QueryAssembler;
pub use result::{ExecutionInfo, Notification, QueryResult, Record, RecordStream, Summary};
pub use session::{ConnectOptions, Credentials, Endpoint, ReconnectNotice, Session};
pub use tabular::{natural_width, render_table, TableOptions, DEFAULT_ROW_CAP};
pub use value::Value;
pub use workers::Workers;
