//! The shape of one query's answer (ADR 0004).
//!
//! A [`QueryResult`] is a header, a [`RecordStream`], and a [`Summary`] that is
//! only readable once the records are drained — mirroring Bolt, where the
//! trailing `SUCCESS` (timing/notifications/stats) follows the last `RECORD`.
//! Slice 02 buffers the records inside `RecordStream`; slice 21 makes the
//! stream pull lazily for bounded memory, without changing this signature.
//! Slice 13 fills the `Summary`.

use std::collections::VecDeque;

use crate::error::Error;
use crate::value::Value;

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
/// (ADR 0004). `next` and `collect` are `async` so the slice-21 lazy-pull
/// implementation is a drop-in.
pub struct RecordStream {
    buffered: VecDeque<Record>,
}

impl RecordStream {
    pub(crate) fn from_buffered(records: Vec<Record>) -> Self {
        Self {
            buffered: records.into(),
        }
    }

    /// The next Record, or `None` when the stream is exhausted.
    pub async fn next(&mut self) -> Result<Option<Record>, Error> {
        Ok(self.buffered.pop_front())
    }

    /// Drain the remaining Records into a Vec (used by the buffered/tabular path).
    pub async fn collect(&mut self) -> Result<Vec<Record>, Error> {
        let mut out = Vec::with_capacity(self.buffered.len());
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

/// Metadata the server attaches after the records (timing, notifications,
/// stats). Empty in slice 02; populated in slice 13.
#[derive(Debug, Default, Clone)]
pub struct Summary {}

/// The whole answer to one query.
pub struct QueryResult {
    header: Vec<String>,
    records: RecordStream,
    summary: Summary,
}

impl QueryResult {
    pub(crate) fn new(header: Vec<String>, records: RecordStream, summary: Summary) -> Self {
        Self {
            header,
            records,
            summary,
        }
    }

    /// The column names.
    pub fn header(&self) -> &[String] {
        &self.header
    }

    /// The record stream.
    pub fn records(&mut self) -> &mut RecordStream {
        &mut self.records
    }

    /// The trailing summary (readable after the records are drained).
    pub fn summary(&self) -> &Summary {
        &self.summary
    }
}
