//! The serial import engine (slice 25): run a stream of queries one after
//! another in input order, rendering each result to a sink in the selected
//! format, and report which queries failed so a Frontend can set an exit code.
//!
//! Serial mode is the foundation parser mode (slice 28) and batched-parallel
//! import (slices 29–32) build on. It stays Frontend-agnostic (ADR 0002): it
//! writes query *output* to the sink (rendering is the Core's job) but prints no
//! diagnostics — a failed query is returned in the [`ImportReport`] for the
//! Frontend to surface and map to an exit code.
//!
//! Output streams for the row-oriented formats (csv/jsonl/cypherl), staying
//! within bounded memory regardless of result size; the tabular format buffers
//! its rows, the deliberate exception (slice 08).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::clause::{scan_clauses, Clause};
use crate::error::Error;
use crate::format::{self, CsvOptions, CsvWriter, CypherlWriter, JsonlWriter};
use crate::session::Session;
use crate::tabular::{render_table, TableOptions};
use crate::value::Value;
use crate::workers::Workers;

/// How a serial run renders each query's result. Mirrors the CLI output-format
/// flag, carrying the per-format config the Core needs to render (csv options,
/// table layout); the Frontend maps its flag onto this.
pub enum OutputFormat {
    /// Buffered tabular table (the bounded-memory exception).
    Tabular(TableOptions),
    /// Streaming CSV.
    Csv(CsvOptions),
    /// Streaming JSON Lines.
    Jsonl,
    /// Streaming cypherl (one Cypher statement per line).
    Cypherl,
}

/// One query that failed during a serial run, kept so the Frontend can report it.
#[derive(Debug)]
pub struct ImportFailure {
    pub query: String,
    pub error: Error,
}

/// The outcome of a serial run: how many queries ran cleanly and which failed.
#[derive(Debug, Default)]
pub struct ImportReport {
    /// Count of queries that executed without error.
    pub executed: usize,
    /// The queries that failed, in the order they were attempted.
    pub failures: Vec<ImportFailure>,
}

impl ImportReport {
    /// Whether every query succeeded (drives a zero exit code).
    pub fn is_success(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Run `queries` serially in input order against `session`, rendering each
/// result to `sink` in `format`.
///
/// A failing query is recorded and the run continues — the Session survives a
/// recoverable query error (slice 14), so the whole stream is attempted and
/// every failure is surfaced rather than masking later ones. The returned
/// [`ImportReport`] tells the Frontend what failed and whether to exit non-zero.
pub async fn run_serial<I, W>(
    session: &mut Session,
    queries: I,
    sink: &mut W,
    format: &OutputFormat,
) -> ImportReport
where
    I: IntoIterator<Item = String>,
    W: Write,
{
    let mut report = ImportReport::default();
    let params = BTreeMap::new();
    for query in queries {
        match execute_and_render(session, &query, &params, sink, format).await {
            Ok(()) => report.executed += 1,
            Err(error) => report.failures.push(ImportFailure { query, error }),
        }
    }
    report
}

/// One query inspected by parser mode: the query text and the ordering-relevant
/// clauses the scanner found in it (slice 27).
#[derive(Debug, Clone)]
pub struct ParsedQuery {
    pub query: String,
    pub clauses: BTreeSet<Clause>,
}

/// The outcome of a parser-mode run: every query inspected (no execution) plus
/// aggregate statistics — how many queries contained each clause.
#[derive(Debug, Default)]
pub struct ParserReport {
    pub queries: Vec<ParsedQuery>,
    pub clause_counts: BTreeMap<Clause, usize>,
}

impl ParserReport {
    /// How many queries were inspected.
    pub fn query_count(&self) -> usize {
        self.queries.len()
    }
}

/// Inspect a query stream with the clause scanner and report on it, executing
/// nothing (parser Import mode). Pure — it never touches a Session, so an import
/// file can be validated before a database is even reachable.
pub fn run_parser<I: IntoIterator<Item = String>>(queries: I) -> ParserReport {
    let mut report = ParserReport::default();
    for query in queries {
        let clauses = scan_clauses(&query);
        for &clause in &clauses {
            *report.clause_counts.entry(clause).or_insert(0) += 1;
        }
        report.queries.push(ParsedQuery { query, clauses });
    }
    report
}

/// Run one query and render its result to the sink. A result with no columns is
/// a write and produces no output in any format; it is drained so the connection
/// is ready for the next query (ADR 0005).
async fn execute_and_render<W: Write>(
    session: &mut Session,
    query: &str,
    params: &BTreeMap<String, Value>,
    sink: &mut W,
    format: &OutputFormat,
) -> Result<(), Error> {
    let mut result = session.run_with_params(query, params).await?;
    let header = result.header().to_vec();
    if header.is_empty() {
        result.records().discard().await?;
        return Ok(());
    }
    match format {
        OutputFormat::Csv(opts) => {
            let mut writer = CsvWriter::new(&mut *sink, opts);
            format::write_stream(&mut writer, &header, result.records()).await?;
        }
        OutputFormat::Jsonl => {
            let mut writer = JsonlWriter::new(&mut *sink);
            format::write_stream(&mut writer, &header, result.records()).await?;
        }
        OutputFormat::Cypherl => {
            let mut writer = CypherlWriter::new(&mut *sink);
            format::write_stream(&mut writer, &header, result.records()).await?;
        }
        OutputFormat::Tabular(opts) => {
            let rows: Vec<Vec<Value>> = result
                .records()
                .collect()
                .await?
                .into_iter()
                .map(|r| r.into_fields())
                .collect();
            writeln!(sink, "{}", render_table(&header, &rows, opts))?;
        }
    }
    Ok(())
}

// --- Batched-parallel executor (slice 30) ---------------------------------

/// A group of queries submitted together as one unit of concurrency (CONTEXT.md
/// "Batch"). A worker pulls a Batch and runs its queries in order on its Session.
#[derive(Debug, Clone)]
pub struct Batch {
    pub queries: Vec<String>,
}

/// The outcome of a batched-parallel run: how many queries ran cleanly and which
/// failed (across all workers, so order is not significant).
#[derive(Debug, Default)]
pub struct ParallelReport {
    pub executed: usize,
    pub failures: Vec<ImportFailure>,
}

impl ParallelReport {
    /// Whether every query succeeded (drives a zero exit code).
    pub fn is_success(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Split a query stream into Batches of at most `batch_size` queries, preserving
/// input order within and across Batches. A `batch_size` of 0 is treated as 1.
pub fn into_batches<I: IntoIterator<Item = String>>(queries: I, batch_size: usize) -> Vec<Batch> {
    let batch_size = batch_size.max(1);
    let mut batches = Vec::new();
    let mut current = Vec::with_capacity(batch_size);
    for query in queries {
        current.push(query);
        if current.len() == batch_size {
            batches.push(Batch {
                queries: std::mem::take(&mut current),
            });
        }
    }
    if !current.is_empty() {
        batches.push(Batch { queries: current });
    }
    batches
}

/// Run a query stream as a batched-parallel import (ADR 0006 worker-pull): split
/// it into Batches and let the `workers` pull from a shared queue until it
/// drains. The worker count *is* the concurrency bound — there is no separate
/// semaphore. Query output is discarded (parallel import loads data; result
/// ordering across workers is meaningless); a failed query is recorded.
///
/// This slice delivers raw parallelism: Batches run in input order *within* a
/// worker but interleave *across* workers, so cross-Batch ordering is not
/// guaranteed — vertices-first ordering (slice 31) and retry (slice 32) layer on.
pub async fn run_parallel<I: IntoIterator<Item = String>>(
    workers: Workers,
    queries: I,
    batch_size: usize,
) -> ParallelReport {
    let queue: Arc<Mutex<VecDeque<Batch>>> =
        Arc::new(Mutex::new(into_batches(queries, batch_size).into()));

    let mut handles = Vec::new();
    for mut session in workers.into_sessions() {
        let queue = queue.clone();
        handles.push(tokio::spawn(async move {
            let mut outcome = ParallelReport::default();
            // Pull one Batch at a time until the shared queue drains.
            while let Some(batch) = queue.lock().await.pop_front() {
                for query in batch.queries {
                    match run_write(&mut session, &query).await {
                        Ok(()) => outcome.executed += 1,
                        Err(error) => outcome.failures.push(ImportFailure { query, error }),
                    }
                }
            }
            outcome
        }));
    }

    let mut report = ParallelReport::default();
    for handle in handles {
        let outcome = handle.await.expect("worker task joins");
        report.executed += outcome.executed;
        report.failures.extend(outcome.failures);
    }
    report
}

/// Run one import query and discard its result, leaving the Session ready for the
/// next (ADR 0005). Used by the parallel path, where output is not rendered.
async fn run_write(session: &mut Session, query: &str) -> Result<(), Error> {
    let mut result = session.run(query).await?;
    result.records().discard().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queries(qs: &[&str]) -> Vec<String> {
        qs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parser_reports_clauses_per_query() {
        let report = run_parser(queries(&[
            "CREATE (:Person {name: 'Ada'})",
            "MATCH (n) DETACH DELETE n",
        ]));
        assert_eq!(report.query_count(), 2);
        assert_eq!(report.queries[0].clauses, BTreeSet::from([Clause::Create]));
        assert_eq!(
            report.queries[1].clauses,
            BTreeSet::from([Clause::Match, Clause::DetachDelete])
        );
    }

    #[test]
    fn parser_aggregates_clause_counts_across_queries() {
        let report = run_parser(queries(&[
            "CREATE (a)",
            "CREATE (b)",
            "MATCH (n) RETURN n",
        ]));
        assert_eq!(report.clause_counts.get(&Clause::Create), Some(&2));
        assert_eq!(report.clause_counts.get(&Clause::Match), Some(&1));
        assert_eq!(report.clause_counts.get(&Clause::Merge), None);
    }

    #[test]
    fn parser_handles_a_query_with_no_ordering_clauses() {
        let report = run_parser(queries(&["RETURN 1"]));
        assert_eq!(report.query_count(), 1);
        assert!(report.queries[0].clauses.is_empty());
        assert!(report.clause_counts.is_empty());
    }

    fn batch_sizes(qs: &[&str], size: usize) -> Vec<usize> {
        into_batches(queries(qs), size)
            .iter()
            .map(|b| b.queries.len())
            .collect()
    }

    #[test]
    fn batches_are_filled_to_size_with_a_short_final_batch() {
        assert_eq!(batch_sizes(&["a", "b", "c", "d", "e"], 2), vec![2, 2, 1]);
        assert_eq!(batch_sizes(&["a", "b", "c", "d"], 2), vec![2, 2]);
    }

    #[test]
    fn batching_preserves_input_order() {
        let batches = into_batches(queries(&["a", "b", "c"]), 2);
        assert_eq!(batches[0].queries, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(batches[1].queries, vec!["c".to_string()]);
    }

    #[test]
    fn an_empty_input_yields_no_batches() {
        assert!(into_batches(Vec::<String>::new(), 10).is_empty());
    }

    #[test]
    fn a_zero_batch_size_is_treated_as_one() {
        assert_eq!(batch_sizes(&["a", "b"], 0), vec![1, 1]);
    }
}
