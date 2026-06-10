//! CLI flag surface for the `mgconsole` binary (slice 09).
//!
//! The clap-derived [`Cli`] mirrors mgconsole's baseline flags, adds `jsonl` as
//! an output format, and validates the cross-field rules (single-character csv
//! delimiter/escape, escape required when doublequote is off) that clap's derive
//! cannot express alone. Parsing and validation are pure — no database, no IO —
//! so they're covered by the unit tests below.

use clap::{Parser, ValueEnum};
use mgconsole_core::DisplayMode;

pub mod config;
pub mod frontend;
pub mod history;
pub mod keywords;
pub mod repl;
pub mod settings;
pub mod syntax;
#[cfg(feature = "tui")]
pub mod workbench;

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

/// When to colour interactive input (ADR 0009). `auto` — the default — means on
/// for an interactive terminal; `always`/`never` force the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
#[value(rename_all = "lower")]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    /// Resolve whether to colour, given whether `NO_COLOR` is in effect and
    /// whether the session is an interactive terminal. Precedence (ADR 0009): an
    /// explicit `--color` beats `NO_COLOR`, which beats the `auto` default — so
    /// `always` ignores `NO_COLOR`, `never` always wins, and `auto` colours only
    /// an interactive terminal with `NO_COLOR` unset.
    pub fn resolve(self, no_color: bool, interactive: bool) -> bool {
        match self {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => !no_color && interactive,
        }
    }
}

/// Whether the `NO_COLOR` convention is in effect for the given environment
/// value: the variable is present and non-empty (an empty value reads as unset,
/// the common practical interpretation).
pub fn no_color_active(no_color_var: Option<&str>) -> bool {
    no_color_var.is_some_and(|v| !v.is_empty())
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
// A CLI flag struct is a flat bag of independent toggles by nature; grouping the
// bools to satisfy the lint would only obscure their 1:1 map to command flags.
#[allow(clippy::struct_excessive_bools)]
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

    /// Result display mode: tabular, vertical, or auto (tabular until a row
    /// exceeds the terminal width, then vertical). Overrides the built-in default
    /// (auto); a runtime `:set display` overrides this in turn.
    #[arg(long, value_parser = parse_display_mode)]
    pub display: Option<DisplayMode>,

    /// Use the line-based REPL instead of the full-screen TUI workbench (ADR
    /// 0010). The workbench is the default for a capable interactive terminal;
    /// `--plain` forces the minimal REPL, as does an incapable terminal.
    #[arg(long)]
    pub plain: bool,

    /// When to syntax-highlight Cypher input: auto (on for an interactive
    /// terminal), always, or never. Honours the `NO_COLOR` convention.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,

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

    /// Path to the persisted command-history directory (ADR 0012: under the
    /// `~/.mgconsole` state directory by default).
    #[arg(long, default_value = "~/.mgconsole")]
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

/// clap value parser for `--display`, deferring to [`DisplayMode`]'s `FromStr` so
/// the flag and the runtime `:set display` accept exactly the same spellings.
fn parse_display_mode(value: &str) -> Result<DisplayMode, String> {
    value.parse()
}

/// Resolve the password to authenticate with, prompting only when a username is
/// given without one.
///
/// The hidden, no-echo prompt is terminal IO, so it is injected as `prompt`:
/// `main` passes a real no-echo reader; tests pass a closure. An empty username
/// means an anonymous (unauthenticated) connection, so the password is
/// irrelevant and never prompted for. A prompt that yields an empty password
/// fails with a clear message rather than attempting a doomed empty login.
pub fn resolve_password(
    username: &str,
    password: &str,
    prompt: impl FnOnce() -> std::io::Result<String>,
) -> Result<String, String> {
    if username.is_empty() || !password.is_empty() {
        return Ok(password.to_string());
    }
    let entered = prompt().map_err(|e| format!("could not read password: {e}"))?;
    if entered.is_empty() {
        return Err(format!("a password is required for user '{username}'"));
    }
    Ok(entered)
}

impl Cli {
    /// Cross-field validation that clap's per-argument parsing cannot express.
    ///
    /// Single-character constraints on the csv delimiter/escape are enforced at
    /// parse time by their `char` type; this catches the one rule that spans two
    /// flags: an escape character is mandatory once doublequote escaping is off.
    pub fn validate(&self) -> Result<(), String> {
        if !self.csv_doublequote && self.csv_escapechar.is_none() {
            return Err("--csv-escapechar is required when --csv-doublequote is false".to_string());
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
        assert_eq!(cli.display, None, "no --display flag: the built-in default applies");
        assert!(!cli.plain, "the workbench is the default; --plain is opt-in");
        assert_eq!(cli.color, ColorChoice::Auto);
        assert_eq!(cli.csv_delimiter, ',');
        assert_eq!(cli.csv_escapechar, None);
        assert!(cli.csv_doublequote);
        assert_eq!(cli.history, "~/.mgconsole");
        assert!(!cli.no_history);
        assert!(!cli.verbose_execution_info);
        assert_eq!(cli.import_mode, ImportMode::Serial);
        assert_eq!(cli.batch_size, 10_000);
        assert_eq!(cli.workers_number, 0);
        assert!(!cli.parser_stats);
        cli.validate().expect("defaults validate");
    }

    #[test]
    fn color_defaults_to_auto_and_accepts_each_choice() {
        // ADR 0009: colour is on by default (auto), no flag needed — the inverse
        // of mgconsole's old opt-in `--term-colors`.
        assert_eq!(parse(&[]).expect("bare").color, ColorChoice::Auto);
        assert_eq!(
            parse(&["--color", "always"]).expect("always").color,
            ColorChoice::Always
        );
        assert_eq!(
            parse(&["--color", "never"]).expect("never").color,
            ColorChoice::Never
        );
        assert_eq!(
            parse(&["--color", "auto"]).expect("auto").color,
            ColorChoice::Auto
        );
        // The old boolean flag is gone.
        assert!(parse(&["--term-colors"]).is_err());
    }

    #[test]
    fn color_resolution_follows_the_precedence_rules() {
        // auto: on for an interactive terminal with NO_COLOR unset...
        assert!(ColorChoice::Auto.resolve(false, true));
        // ...off when NO_COLOR is set, or when not interactive.
        assert!(!ColorChoice::Auto.resolve(true, true));
        assert!(!ColorChoice::Auto.resolve(false, false));
        // always beats NO_COLOR; never always wins.
        assert!(ColorChoice::Always.resolve(true, false));
        assert!(!ColorChoice::Never.resolve(false, true));
    }

    #[test]
    fn no_color_is_active_only_when_present_and_non_empty() {
        assert!(!no_color_active(None));
        assert!(!no_color_active(Some("")));
        assert!(no_color_active(Some("1")));
    }

    #[test]
    fn plain_selects_the_repl_frontend() {
        assert!(!parse(&[]).expect("bare").plain);
        assert!(parse(&["--plain"]).expect("--plain parses").plain);
    }

    #[test]
    fn display_flag_parses_each_mode_and_rejects_others() {
        assert_eq!(parse(&[]).expect("bare").display, None);
        assert_eq!(
            parse(&["--display", "vertical"]).expect("vertical").display,
            Some(DisplayMode::Vertical)
        );
        assert_eq!(
            parse(&["--display", "tabular"]).expect("tabular").display,
            Some(DisplayMode::Tabular)
        );
        assert_eq!(
            parse(&["--display", "auto"]).expect("auto").display,
            Some(DisplayMode::Auto)
        );
        assert!(parse(&["--display", "grid"]).is_err(), "unknown mode rejected");
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
        let cli = parse(&["--csv-doublequote", "false", "--csv-escapechar", "\\"]).expect("parses");
        assert_eq!(cli.csv_escapechar, Some('\\'));
        cli.validate().expect("escape char supplied");
    }

    fn never_prompts() -> std::io::Result<String> {
        panic!("prompt must not be called")
    }

    #[test]
    fn anonymous_connection_never_prompts() {
        let pw = resolve_password("", "", never_prompts).expect("anonymous ok");
        assert_eq!(pw, "");
    }

    #[test]
    fn explicit_password_is_used_without_prompting() {
        let pw = resolve_password("alice", "secret", never_prompts).expect("explicit ok");
        assert_eq!(pw, "secret");
    }

    #[test]
    fn username_without_password_prompts() {
        let pw = resolve_password("alice", "", || Ok("typed".to_string())).expect("prompt ok");
        assert_eq!(pw, "typed");
    }

    #[test]
    fn empty_prompted_password_fails_clearly() {
        let err = resolve_password("alice", "", || Ok(String::new()))
            .expect_err("empty password rejected");
        assert!(err.contains("alice"), "message names the user: {err}");
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
