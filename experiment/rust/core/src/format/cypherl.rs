//! Streaming cypherl writer (slice 24): emit replayable Cypher statements, one
//! per line, as Records arrive.
//!
//! Designed for `DUMP DATABASE`-style results (slice 26), where each row is a
//! single string column already holding a valid Cypher statement. Each
//! string-valued cell is written verbatim with exactly one trailing `;`, so the
//! output re-imports cleanly via the serial import path.

use std::io::Write;

use crate::render;
use crate::value::Value;

use super::{Header, RowWriter};

pub struct CypherlWriter<W: Write> {
    sink: W,
}

impl<W: Write> CypherlWriter<W> {
    pub fn new(sink: W) -> Self {
        Self { sink }
    }
}

/// Ensure the statement ends with exactly one semicolon.
fn statement(s: &str) -> String {
    let trimmed = s.trim_end();
    let trimmed = trimmed.trim_end_matches(';').trim_end();
    format!("{trimmed};")
}

impl<W: Write> RowWriter for CypherlWriter<W> {
    fn write_header(&mut self, _header: &Header) -> std::io::Result<()> {
        // cypherl has no header.
        Ok(())
    }

    fn write_row(&mut self, row: &[Value]) -> std::io::Result<()> {
        for value in row {
            let text = match value {
                Value::String(s) => statement(s),
                other => statement(&render::tabular(other)),
            };
            writeln!(self.sink, "{text}")?;
        }
        Ok(())
    }

    fn finish(&mut self) -> std::io::Result<()> {
        self.sink.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(rows: &[Vec<Value>]) -> String {
        let mut buf = Vec::new();
        {
            let mut w = CypherlWriter::new(&mut buf);
            w.write_header(&Header::new(Vec::new())).unwrap();
            for r in rows {
                w.write_row(r).unwrap();
            }
            w.finish().unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn dump_statements_written_one_per_line() {
        let out = write(&[
            vec![Value::String("CREATE (:Person {name: \"Ada\"});".into())],
            vec![Value::String("CREATE INDEX ON :Person(name)".into())],
        ]);
        assert_eq!(
            out,
            "CREATE (:Person {name: \"Ada\"});\nCREATE INDEX ON :Person(name);\n"
        );
    }

    #[test]
    fn trailing_semicolon_is_idempotent() {
        assert_eq!(
            write(&[vec![Value::String("RETURN 1;".into())]]),
            "RETURN 1;\n"
        );
        assert_eq!(
            write(&[vec![Value::String("RETURN 1".into())]]),
            "RETURN 1;\n"
        );
        assert_eq!(
            write(&[vec![Value::String("RETURN 1 ;  ".into())]]),
            "RETURN 1;\n"
        );
    }
}
