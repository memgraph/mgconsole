//! The interactive REPL Frontend (slice 02: a minimal non-interactive pipe).
//!
//! Reads a query from stdin, runs it through the Core Session, and prints each
//! record as tab-separated tabular text. The async Core is driven via
//! `block_on` at this boundary (ADR 0002). The flag surface (slice 09) and the
//! real REPL loop (slice 16) build on this.

use std::io::Read;

use mgconsole_core::{render, Error, Session};

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
        while let Some(record) = result.records().next().await? {
            let cells: Vec<String> = record.fields().iter().map(render::tabular).collect();
            println!("{}", cells.join("\t"));
        }
        Ok::<(), Error>(())
    })?;

    Ok(())
}
