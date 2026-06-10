//! Streaming CSV writer (slice 22), backed by the `csv` crate, which handles
//! quoting/escaping of cells containing the delimiter, quotes, or newlines.

use std::io::Write;

use crate::render;
use crate::value::Value;

use super::RowWriter;

/// CSV formatting options, mirroring today's `mgconsole` flags.
#[derive(Debug, Clone)]
pub struct CsvOptions {
    pub delimiter: u8,
    pub quote: u8,
    pub escape: u8,
    /// When true, an embedded quote is doubled (`""`); when false, it is
    /// prefixed with `escape`.
    pub double_quote: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote: b'"',
            escape: b'\\',
            double_quote: true,
        }
    }
}

pub struct CsvWriter<W: Write> {
    inner: csv::Writer<W>,
}

impl<W: Write> CsvWriter<W> {
    pub fn new(sink: W, opts: &CsvOptions) -> Self {
        let inner = csv::WriterBuilder::new()
            .delimiter(opts.delimiter)
            .quote(opts.quote)
            .escape(opts.escape)
            .double_quote(opts.double_quote)
            .from_writer(sink);
        Self { inner }
    }
}

/// The logical cell text for a Value in CSV. Strings are raw (the csv crate
/// quotes them as needed); a null is an empty cell; composites reuse the
/// tabular rendering.
fn cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        _ => render::tabular(value),
    }
}

impl<W: Write> RowWriter for CsvWriter<W> {
    fn write_header(&mut self, header: &[String]) -> std::io::Result<()> {
        self.inner.write_record(header).map_err(csv_io)
    }

    fn write_row(&mut self, row: &[Value]) -> std::io::Result<()> {
        self.inner
            .write_record(row.iter().map(cell))
            .map_err(csv_io)
    }

    fn finish(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Unwrap a `csv::Error` to its underlying I/O error where possible.
fn csv_io(e: csv::Error) -> std::io::Error {
    match e.into_kind() {
        csv::ErrorKind::Io(io) => io,
        _ => std::io::Error::other("csv write error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(header: &[&str], rows: &[Vec<Value>], opts: &CsvOptions) -> String {
        let mut buf = Vec::new();
        {
            let mut w = CsvWriter::new(&mut buf, opts);
            let hdr: Vec<String> = header
                .iter()
                .map(std::string::ToString::to_string)
                .collect();
            w.write_header(&hdr).unwrap();
            for r in rows {
                w.write_row(r).unwrap();
            }
            w.finish().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn quotes_cells_with_delimiter_quote_or_newline() {
        let out = write(
            &["a", "b"],
            &[vec![
                Value::String("x,y".into()),
                Value::String("line1\nline2".into()),
            ]],
            &CsvOptions::default(),
        );
        assert_eq!(out, "a,b\n\"x,y\",\"line1\nline2\"\n");
    }

    #[test]
    fn custom_delimiter() {
        let out = write(
            &["a", "b"],
            &[vec![Value::Integer(1), Value::Integer(2)]],
            &CsvOptions {
                delimiter: b'\t',
                ..Default::default()
            },
        );
        assert_eq!(out, "a\tb\n1\t2\n");
    }

    #[test]
    fn null_is_empty_cell_composites_rendered() {
        let out = write(
            &["n", "xs"],
            &[vec![
                Value::Null,
                Value::List(vec![Value::Integer(1), Value::Integer(2)]),
            ]],
            &CsvOptions::default(),
        );
        // The list cell contains a comma, so it is quoted.
        assert_eq!(out, "n,xs\n,\"[1, 2]\"\n");
    }
}
