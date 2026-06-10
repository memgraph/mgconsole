//! Row-oriented streaming output formats (slices 22–24): csv, jsonl, cypherl.
//!
//! Each format implements [`RowWriter`] — a header write plus a per-row write —
//! so [`write_stream`] can drive a [`RecordStream`] through it one Record at a
//! time, keeping memory bounded regardless of result size (ADR 0002). The
//! buffer-all tabular path is the deliberate exception (slice 08).

pub mod csv;
pub mod cypherl;
pub mod jsonl;

use crate::error::Error;
use crate::result::RecordStream;
use crate::value::Value;

/// A streaming output format: write a header, then each row as it arrives.
pub trait RowWriter {
    fn write_header(&mut self, header: &[String]) -> std::io::Result<()>;
    fn write_row(&mut self, row: &[Value]) -> std::io::Result<()>;
    /// Flush any buffered output. Default: nothing to do.
    fn finish(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Drive a record stream through a writer, one Record at a time.
pub async fn write_stream<W: RowWriter>(
    writer: &mut W,
    header: &[String],
    stream: &mut RecordStream,
) -> Result<(), Error> {
    writer.write_header(header)?;
    while let Some(record) = stream.next().await? {
        writer.write_row(record.fields())?;
    }
    writer.finish()?;
    Ok(())
}

pub use csv::{CsvOptions, CsvWriter};
pub use cypherl::CypherlWriter;
pub use jsonl::JsonlWriter;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::result::{Record, RecordStream};

    #[tokio::test]
    async fn write_stream_drives_header_then_rows() {
        let mut stream = RecordStream::from_buffered(vec![
            Record::new(vec![Value::Integer(1), Value::String("Ada".into())]),
            Record::new(vec![Value::Integer(2), Value::String("Bo".into())]),
        ]);
        let header = vec!["n".to_string(), "name".to_string()];

        let mut buf = Vec::new();
        {
            let mut writer = CsvWriter::new(&mut buf, &CsvOptions::default());
            write_stream(&mut writer, &header, &mut stream).await.unwrap();
        }
        assert_eq!(String::from_utf8(buf).unwrap(), "n,name\n1,Ada\n2,Bo\n");
    }
}
