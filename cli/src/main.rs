//! The interactive REPL Frontend (slice 02: a minimal non-interactive pipe).
//!
//! Reads a query from stdin, runs it through the Core Session, and prints each
//! record as tab-separated tabular text. The async Core is driven via
//! `block_on` at this boundary (ADR 0002). The flag surface (slice 09) and the
//! real REPL loop (slice 16) build on this.

use std::io::Read;

use mgconsole_core::{render_table, tabular, Error, Session, TableOptions, Value, DEFAULT_ROW_CAP};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut query = String::new();
    std::io::stdin().read_to_string(&mut query)?;
    let query = query.trim();
    if query.is_empty() {
        return Ok(());
    }

    let host = std::env::var("MG_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port: u16 = std::env::var("MG_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7687);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        let mut session = Session::connect(&host, port).await?;
        let mut result = session.run(query).await?;
        let header = result.header().to_vec();
        let (records, overflowed) = result.records().collect_capped(DEFAULT_ROW_CAP).await?;
        let rows: Vec<Vec<Value>> = records.into_iter().map(|r| r.into_fields()).collect();

        println!("{}", render_table(&header, &rows, &TableOptions::default()));
        if overflowed {
            eprintln!("{}", tabular::row_cap_warning(DEFAULT_ROW_CAP));
        }
        Ok::<(), Error>(())
    })?;

    Ok(())
}
