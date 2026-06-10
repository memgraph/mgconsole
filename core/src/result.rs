//! The shape of one query's answer (ADR 0004).
//!
//! A [`QueryResult`] is a header, a [`RecordStream`], and a [`Summary`] that is
//! only readable once the records are drained — mirroring Bolt, where the
//! trailing `SUCCESS` (timing/notifications/stats) follows the last `RECORD`.
//! Slice 02 buffers the records inside `RecordStream`; slice 21 makes the
//! stream pull lazily for bounded memory, without changing this signature.
//! Slice 13 fills the `Summary`.

use std::collections::{BTreeMap, HashMap, VecDeque};

use bolt_client::Metadata;
use bolt_proto::Message;

use crate::error::Error;
use crate::proto;
use crate::session::SharedConn;
use crate::value::Value;

/// Default number of records pulled per batch on the lazy path.
pub(crate) const DEFAULT_BATCH_SIZE: i64 = 1000;

/// One row of a result: an ordered set of Values, one per column.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    fields: Vec<Value>,
}

impl Record {
    pub fn new(fields: Vec<Value>) -> Self {
        Self { fields }
    }

    pub fn fields(&self) -> &[Value] {
        &self.fields
    }

    pub fn into_fields(self) -> Vec<Value> {
        self.fields
    }
}

/// The Records of one query, consumed one at a time.
///
/// Owned by the Core — it does not expose `futures::Stream` in the public API
/// (ADR 0004). The lazy variant pulls from the connection in batches, so it
/// never holds more than one batch in memory regardless of result size.
pub struct RecordStream {
    inner: Inner,
    /// Filled from the RUN reply, then completed from the trailing `SUCCESS`
    /// once the stream drains (or is discarded). Readable via [`QueryResult`].
    summary: Summary,
}

enum Inner {
    /// A fully in-memory result. Lets the cap/collect logic be unit-tested
    /// without a live connection; the production path is always `Lazy`.
    #[allow(dead_code)]
    Buffered(VecDeque<Record>),
    /// Records pulled lazily from a shared connection.
    Lazy(Lazy),
}

struct Lazy {
    conn: SharedConn,
    batch: VecDeque<Record>,
    more: bool,
    batch_size: i64,
}

impl RecordStream {
    #[allow(dead_code)] // used by unit tests; production constructs `lazy`
    pub(crate) fn from_buffered(records: Vec<Record>) -> Self {
        Self {
            inner: Inner::Buffered(records.into()),
            summary: Summary::default(),
        }
    }

    pub(crate) fn lazy(conn: SharedConn, batch_size: i64, summary: Summary) -> Self {
        Self {
            inner: Inner::Lazy(Lazy {
                conn,
                batch: VecDeque::new(),
                more: true,
                batch_size,
            }),
            summary,
        }
    }

    /// The trailing summary. Complete only once the stream has drained or been
    /// discarded; before then it holds just the RUN-time metadata.
    pub(crate) fn summary(&self) -> &Summary {
        &self.summary
    }

    /// The next Record, or `None` when the stream is exhausted.
    pub async fn next(&mut self) -> Result<Option<Record>, Error> {
        match &mut self.inner {
            Inner::Buffered(b) => Ok(b.pop_front()),
            Inner::Lazy(l) => l.next(&mut self.summary).await,
        }
    }

    /// Discard any records not yet consumed, leaving the connection ready for
    /// the next query. A no-op once the stream is exhausted.
    pub async fn discard(&mut self) -> Result<(), Error> {
        match &mut self.inner {
            Inner::Buffered(b) => {
                b.clear();
                Ok(())
            }
            Inner::Lazy(l) => l.discard(&mut self.summary).await,
        }
    }

    /// Drain the remaining Records into a Vec (used by the buffered/tabular path).
    pub async fn collect(&mut self) -> Result<Vec<Record>, Error> {
        let mut out = Vec::new();
        while let Some(r) = self.next().await? {
            out.push(r);
        }
        Ok(out)
    }

    /// Collect up to `cap` Records, returning them and whether more remained.
    ///
    /// Used by the tabular path (ADR 0002): it never holds more than `cap`
    /// Records (one extra is pulled transiently to detect overflow, then
    /// dropped), so the buffered renderer is bounded even on a huge result.
    pub async fn collect_capped(&mut self, cap: usize) -> Result<(Vec<Record>, bool), Error> {
        let mut rows = Vec::new();
        while rows.len() < cap {
            match self.next().await? {
                Some(r) => rows.push(r),
                None => return Ok((rows, false)),
            }
        }
        let overflowed = self.next().await?.is_some();
        Ok((rows, overflowed))
    }
}

impl Lazy {
    async fn next(&mut self, summary: &mut Summary) -> Result<Option<Record>, Error> {
        if let Some(r) = self.batch.pop_front() {
            return Ok(Some(r));
        }
        if !self.more {
            return Ok(None);
        }
        self.pull_batch(summary).await?;
        Ok(self.batch.pop_front())
    }

    /// Pull up to `batch_size` records, translating them into Core Records and
    /// updating `more` from the trailing `SUCCESS`. The final `SUCCESS` (the one
    /// with no more records) carries the summary, so absorb it then.
    async fn pull_batch(&mut self, summary: &mut Summary) -> Result<(), Error> {
        let (records, end) = {
            let mut client = self.conn.lock().await;
            client
                .pull(Some(Metadata::from_iter(vec![("n", self.batch_size)])))
                .await
                .map_err(|e| Error::Connection(e.to_string()))?
        };
        match end {
            Message::Success(s) => {
                self.more = proto::has_more(s.metadata());
                if !self.more {
                    summary.absorb_terminal(s.metadata());
                }
            }
            Message::Failure(f) => {
                self.more = false;
                return Err(Error::Query(proto::failure_message(f.metadata())));
            }
            other => {
                self.more = false;
                return Err(Error::Protocol(format!("unexpected PULL reply: {other:?}")));
            }
        }
        self.batch = records
            .into_iter()
            .map(|r| Record::new(r.fields().iter().cloned().map(Value::from).collect()))
            .collect();
        Ok(())
    }

    async fn discard(&mut self, summary: &mut Summary) -> Result<(), Error> {
        self.batch.clear();
        if !self.more {
            return Ok(());
        }
        let end = {
            let mut client = self.conn.lock().await;
            client
                .discard(Some(Metadata::from_iter(vec![("n", -1_i64)])))
                .await
                .map_err(|e| Error::Connection(e.to_string()))?
        };
        self.more = false;
        // DISCARD's SUCCESS carries the same summary the final PULL would have.
        match end {
            Message::Success(s) => summary.absorb_terminal(s.metadata()),
            Message::Failure(f) => return Err(Error::Query(proto::failure_message(f.metadata()))),
            other => return Err(Error::Protocol(format!("unexpected DISCARD reply: {other:?}"))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(n: usize) -> RecordStream {
        RecordStream::from_buffered(
            (0..n)
                .map(|i| Record::new(vec![Value::Integer(i as i64)]))
                .collect(),
        )
    }

    #[tokio::test]
    async fn collect_capped_under_cap_reports_no_overflow() {
        let (rows, overflowed) = stream(3).collect_capped(10).await.unwrap();
        assert_eq!(rows.len(), 3);
        assert!(!overflowed);
    }

    #[tokio::test]
    async fn collect_capped_over_cap_caps_and_flags_overflow() {
        let (rows, overflowed) = stream(100).collect_capped(10).await.unwrap();
        assert_eq!(rows.len(), 10);
        assert!(overflowed);
    }

    #[tokio::test]
    async fn collect_capped_exactly_at_cap_is_not_overflow() {
        let (rows, overflowed) = stream(10).collect_capped(10).await.unwrap();
        assert_eq!(rows.len(), 10);
        assert!(!overflowed);
    }
}

/// One advisory Notification Memgraph attaches to a result (CONTEXT.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub code: String,
    pub title: String,
    pub description: String,
    pub severity: String,
}

impl Notification {
    fn from_value(value: &bolt_proto::Value) -> Option<Self> {
        let bolt_proto::Value::Map(m) = value else {
            return None;
        };
        let s = |k: &str| match m.get(k) {
            Some(bolt_proto::Value::String(v)) => v.clone(),
            _ => String::new(),
        };
        Some(Notification {
            code: s("code"),
            title: s("title"),
            description: s("description"),
            severity: s("severity"),
        })
    }
}

/// Metadata the server attaches after the records: Notifications, execution
/// stats, and the residual timing/execution-info keys (cost, parse, plan,
/// execute) the server returns. Each field is empty when the server sends none,
/// so absence is just an empty collection — never an error.
#[derive(Debug, Default, Clone)]
pub struct Summary {
    /// Advisory notifications attached to the result.
    pub notifications: Vec<Notification>,
    /// Update statistics (e.g. `nodes-created`) for a write query.
    pub stats: BTreeMap<String, Value>,
    /// Remaining trailing metadata, keyed as the server sends it — timing and
    /// verbose execution info live here, since their keys are server-specific.
    pub metadata: BTreeMap<String, Value>,
}

impl Summary {
    /// Build the initial summary from the RUN reply's metadata (timings such as
    /// `t_first` arrive here, before any records).
    pub(crate) fn from_run(meta: &HashMap<String, bolt_proto::Value>) -> Self {
        Summary {
            metadata: lift_metadata(meta),
            ..Summary::default()
        }
    }

    /// The verbose execution info (cost/parse/plan/execute) Memgraph reports in
    /// the trailing metadata, lifted into typed fields. Each is `None` when the
    /// server did not report it.
    pub fn execution_info(&self) -> ExecutionInfo {
        let number = |k: &str| self.metadata.get(k).and_then(as_f64);
        ExecutionInfo {
            cost_estimate: number("cost_estimate"),
            parsing_time: number("parsing_time"),
            planning_time: number("planning_time"),
            plan_execution_time: number("plan_execution_time"),
        }
    }

    /// Merge in the trailing `SUCCESS` metadata once the records are drained.
    fn absorb_terminal(&mut self, meta: &HashMap<String, bolt_proto::Value>) {
        if let Some(bolt_proto::Value::List(items)) = meta.get("notifications") {
            self.notifications = items.iter().filter_map(Notification::from_value).collect();
        }
        if let Some(bolt_proto::Value::Map(stats)) = meta.get("stats") {
            self.stats = stats
                .iter()
                .map(|(k, v)| (k.clone(), Value::from(v.clone())))
                .collect();
        }
        self.metadata.extend(lift_metadata(meta));
    }
}

/// Verbose execution info Memgraph reports per query: estimated cost and the
/// parse/plan/execute timings (CONTEXT.md: "verbose execution info").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecutionInfo {
    pub cost_estimate: Option<f64>,
    pub parsing_time: Option<f64>,
    pub planning_time: Option<f64>,
    pub plan_execution_time: Option<f64>,
}

/// Read a numeric metadata value as `f64`, accepting either a float or integer.
fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Float(f) => Some(*f),
        Value::Integer(i) => Some(*i as f64),
        _ => None,
    }
}

/// Project Bolt metadata into the residual `metadata` map, dropping keys that
/// are control flags or are lifted into typed fields elsewhere.
fn lift_metadata(meta: &HashMap<String, bolt_proto::Value>) -> BTreeMap<String, Value> {
    meta.iter()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "has_more" | "notifications" | "stats" | "fields"
            )
        })
        .map(|(k, v)| (k.clone(), Value::from(v.clone())))
        .collect()
}

/// The whole answer to one query.
pub struct QueryResult {
    header: Vec<String>,
    records: RecordStream,
}

impl QueryResult {
    pub(crate) fn new(header: Vec<String>, records: RecordStream) -> Self {
        Self { header, records }
    }

    /// The column names.
    pub fn header(&self) -> &[String] {
        &self.header
    }

    /// The record stream.
    pub fn records(&mut self) -> &mut RecordStream {
        &mut self.records
    }

    /// The trailing summary. Complete once the records are drained or discarded;
    /// before then it carries only the RUN-time metadata.
    pub fn summary(&self) -> &Summary {
        self.records.summary()
    }
}
