//! The interactive REPL Frontend (slice 02: a minimal non-interactive pipe).
//!
//! Reads a query from stdin, runs it through the Core Session, and prints each
//! record as tabular text. The async Core is driven via `block_on` at this
//! boundary (ADR 0002). The flag surface (slice 09) parses connection and output
//! options here; the real REPL loop (slice 16) and auth/TLS (10/11) build on it.

use std::io::Read;

use clap::Parser;
use mgconsole::{resolve_password, Cli};
use mgconsole_core::{
    render_table, tabular, ConnectOptions, Credentials, Error, Session, TableOptions, Value,
    DEFAULT_ROW_CAP,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    if let Err(message) = cli.validate() {
        eprintln!("error: {message}");
        std::process::exit(2);
    }

    // Resolve auth before touching the network: a username with no password gets
    // a hidden prompt; an empty username stays anonymous.
    let password = match resolve_password(&cli.username, &cli.password, || {
        rpassword::prompt_password("Password: ")
    }) {
        Ok(password) => password,
        Err(message) => {
            eprintln!("error: {message}");
            std::process::exit(2);
        }
    };
    let options = ConnectOptions {
        credentials: (!cli.username.is_empty()).then(|| Credentials {
            username: cli.username.clone(),
            password,
        }),
        use_tls: cli.use_ssl,
    };

    let mut query = String::new();
    std::io::stdin().read_to_string(&mut query)?;
    let query = query.trim();
    if query.is_empty() {
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        let mut session = Session::connect_with(&cli.host, cli.port, &options).await?;
        let mut result = session.run(query).await?;
        let header = result.header().to_vec();
        let (records, overflowed) = result.records().collect_capped(DEFAULT_ROW_CAP).await?;
        // Drop any rows beyond the cap so the connection is ready for reuse.
        result.records().discard().await?;
        let rows: Vec<Vec<Value>> = records.into_iter().map(|r| r.into_fields()).collect();

        // `--fit-to-screen` resolves to a concrete terminal width in the REPL
        // slice (16); the flag is parsed and validated here in slice 09.
        println!("{}", render_table(&header, &rows, &TableOptions::default()));
        if overflowed {
            eprintln!("{}", tabular::row_cap_warning(DEFAULT_ROW_CAP));
        }
        Ok::<(), Error>(())
    })?;

    Ok(())
}
