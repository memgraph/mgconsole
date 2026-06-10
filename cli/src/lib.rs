//! CLI flag surface for the `mgconsole` binary (slice 09).
//!
//! The clap-derived [`Cli`] mirrors mgconsole's baseline flags, adds `jsonl` as
//! an output format, and validates the cross-field rules (single-character csv
//! delimiter/escape, escape required when doublequote is off) that clap's derive
//! cannot express alone. Parsing and validation are pure — no database, no IO —
//! so they're covered by the unit tests below.

use clap::{Parser, ValueEnum};

/// Output format for a Query result (CONTEXT.md: tabular buffers; csv/jsonl/
/// cypherl stream). `jsonl` is the addition over today's mgconsole flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum OutputFormat {
    Tabular,
    Csv,
    Jsonl,
    Cypherl,
}

/// The discipline by which a batch of queries from a file is run (CONTEXT.md
/// "Import mode").
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ImportMode {
    /// Run queries one after another in the given order (the default).
    Serial,
    /// Run queries concurrently in batches for speed.
    BatchedParallel,
    /// Inspect queries and report on them without running any.
    Parser,
}

/// The complete flag surface for the binary. Defaults mirror today's mgconsole.
#[derive(Debug, Parser)]
#[command(name = "mgconsole", version, about = "Rust Memgraph console")]
pub struct Cli {
    /// Server host to connect to.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Server Bolt port.
    #[arg(long, default_value_t = 7687)]
    pub port: u16,

    /// Username for authentication.
    #[arg(long, default_value = "")]
    pub username: String,

    /// Password for authentication.
    #[arg(long, default_value = "")]
    pub password: String,

    /// Connect over SSL/TLS.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub use_ssl: bool,

    /// Output format for query results.
    #[arg(long, value_enum, default_value_t = OutputFormat::Tabular)]
    pub output_format: OutputFormat,

    /// Truncate tabular output to fit the terminal width.
    #[arg(long)]
    pub fit_to_screen: bool,

    /// Field delimiter for csv output (a single character).
    #[arg(long, default_value_t = ',')]
    pub csv_delimiter: char,

    /// Escape character for csv output (a single character). Required when
    /// `--csv-doublequote false`.
    #[arg(long)]
    pub csv_escapechar: Option<char>,

    /// Escape a quote inside a csv field by doubling it, rather than using the
    /// escape character.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub csv_doublequote: bool,

    /// Path to the persisted command-history file.
    #[arg(long, default_value = "~/.memgraph")]
    pub history: String,

    /// Disable persisting command history.
    #[arg(long)]
    pub no_history: bool,

    /// Print verbose execution info (cost, parse, plan, execute times).
    #[arg(long)]
    pub verbose_execution_info: bool,

    /// Import discipline for a non-interactive query stream.
    #[arg(long, value_enum, default_value_t = ImportMode::Serial)]
    pub import_mode: ImportMode,

    /// Number of queries submitted together as one Batch in parallel import.
    #[arg(long, default_value_t = 10_000)]
    pub batch_size: u64,

    /// Worker count for parallel import (0 = auto-detect).
    #[arg(long, default_value_t = 0)]
    pub workers_number: usize,

    /// Collect and print per-query statistics in parser mode.
    #[arg(long)]
    pub parser_stats: bool,
}

impl Cli {
    /// Cross-field validation that clap's per-argument parsing cannot express.
    ///
    /// Single-character constraints on the csv delimiter/escape are enforced at
    /// parse time by their `char` type; this catches the one rule that spans two
    /// flags: an escape character is mandatory once doublequote escaping is off.
    pub fn validate(&self) -> Result<(), String> {
        if !self.csv_doublequote && self.csv_escapechar.is_none() {
            return Err(
                "--csv-escapechar is required when --csv-doublequote is false".to_string(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("mgconsole").chain(args.iter().copied()))
    }

    #[test]
    fn defaults_match_baseline() {
        let cli = parse(&[]).expect("bare invocation parses");
        assert_eq!(cli.host, "127.0.0.1");
        assert_eq!(cli.port, 7687);
        assert_eq!(cli.username, "");
        assert_eq!(cli.password, "");
        assert!(cli.use_ssl);
        assert_eq!(cli.output_format, OutputFormat::Tabular);
        assert!(!cli.fit_to_screen);
        assert_eq!(cli.csv_delimiter, ',');
        assert_eq!(cli.csv_escapechar, None);
        assert!(cli.csv_doublequote);
        assert_eq!(cli.history, "~/.memgraph");
        assert!(!cli.no_history);
        assert!(!cli.verbose_execution_info);
        assert_eq!(cli.import_mode, ImportMode::Serial);
        assert_eq!(cli.batch_size, 10_000);
        assert_eq!(cli.workers_number, 0);
        assert!(!cli.parser_stats);
        cli.validate().expect("defaults validate");
    }

    #[test]
    fn jsonl_is_accepted_as_output_format() {
        let cli = parse(&["--output-format", "jsonl"]).expect("jsonl parses");
        assert_eq!(cli.output_format, OutputFormat::Jsonl);
    }

    #[test]
    fn every_output_format_parses() {
        for (text, want) in [
            ("tabular", OutputFormat::Tabular),
            ("csv", OutputFormat::Csv),
            ("jsonl", OutputFormat::Jsonl),
            ("cypherl", OutputFormat::Cypherl),
        ] {
            let cli = parse(&["--output-format", text]).expect("format parses");
            assert_eq!(cli.output_format, want);
        }
    }

    #[test]
    fn invalid_output_format_is_rejected() {
        let err = parse(&["--output-format", "xml"]).expect_err("xml is rejected");
        assert_eq!(err.kind(), ErrorKind::InvalidValue);
        // The message lists the allowed set so the user can recover.
        assert!(err.to_string().contains("jsonl"));
    }

    #[test]
    fn csv_delimiter_must_be_a_single_character() {
        let err = parse(&["--csv-delimiter", ";;"]).expect_err("multi-char delimiter rejected");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn escapechar_required_when_doublequote_off() {
        let cli = parse(&["--csv-doublequote", "false"]).expect("parses");
        assert!(cli.validate().is_err());
    }

    #[test]
    fn escapechar_satisfies_doublequote_off() {
        let cli = parse(&["--csv-doublequote", "false", "--csv-escapechar", "\\"])
            .expect("parses");
        assert_eq!(cli.csv_escapechar, Some('\\'));
        cli.validate().expect("escape char supplied");
    }

    #[test]
    fn help_and_version_are_handled() {
        assert_eq!(
            parse(&["--help"]).expect_err("help exits").kind(),
            ErrorKind::DisplayHelp
        );
        assert_eq!(
            parse(&["--version"]).expect_err("version exits").kind(),
            ErrorKind::DisplayVersion
        );
    }
}
