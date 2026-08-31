//! CLI flag surface for the `mgconsole` binary (slice 09).
//!
//! The clap-derived [`Cli`] mirrors mgconsole's baseline flags, adds `jsonl` as
//! an output format, and validates the cross-field rules (single-character csv
//! delimiter/escape, escape required when doublequote is off) that clap's derive
//! cannot express alone. Parsing and validation are pure — no database, no IO —
//! so they're covered by the unit tests below.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use mgconsole_core::{ConnectOptions, Credentials, DisplayMode, Endpoint};

use crate::config::{Config, Profile};

pub mod config;
pub mod cypher_format;
pub mod frontend;
pub mod history;
pub mod keywords;
pub mod queries;
pub mod repl;
pub mod settings;
pub mod syntax;
pub mod theme;
#[cfg(feature = "tui")]
pub mod workbench;

/// The one output-format vocabulary (issue 12): `csv | jsonl | cypherl | table`,
/// used identically by the batch `--output-format` flag, the Workbench export
/// gesture, and `:o` redirection. `csv`/`jsonl`/`cypherl` stream; `table` is the
/// buffer-all tabular layout. (`tabular` is accepted as an alias of `table`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum OutputFormat {
    #[value(alias = "tabular")]
    Table,
    Csv,
    Jsonl,
    Cypherl,
}

impl OutputFormat {
    /// The canonical lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            OutputFormat::Table => "table",
            OutputFormat::Csv => "csv",
            OutputFormat::Jsonl => "jsonl",
            OutputFormat::Cypherl => "cypherl",
        }
    }

    /// Whether this format streams row-by-row (bounded memory) rather than
    /// buffering the whole result like `table` does.
    pub fn is_streaming(self) -> bool {
        !matches!(self, OutputFormat::Table)
    }

    /// Resolve the output format from an explicit `--output-format` and whether
    /// stdout is a terminal (issue 19, ADR 0014). An explicit choice always wins;
    /// otherwise the built-in default is a function of stdout — `table` at a
    /// terminal, `jsonl` when piped/redirected, since Memgraph Values nest and a
    /// non-interactive consumer needs a faithful, nesting format (ADR 0003). This
    /// default sits *beneath* the precedence chain (default < config < flag), in
    /// the `ColorChoice::resolve` style.
    pub fn resolve(explicit: Option<OutputFormat>, stdout_is_tty: bool) -> OutputFormat {
        match explicit {
            Some(format) => format,
            None if stdout_is_tty => OutputFormat::Table,
            None => OutputFormat::Jsonl,
        }
    }

    /// Infer the format from a file extension (issue 12): `.csv`, `.jsonl`,
    /// `.cypherl`/`.cypher`, `.txt`/`.tsv` → table. Unknown extensions are `None`.
    pub fn from_extension(path: &std::path::Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str())?.to_ascii_lowercase().as_str() {
            "csv" => Some(OutputFormat::Csv),
            "jsonl" | "json" => Some(OutputFormat::Jsonl),
            "cypherl" | "cypher" | "cql" => Some(OutputFormat::Cypherl),
            "txt" | "tsv" | "table" => Some(OutputFormat::Table),
            _ => None,
        }
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "table" | "tabular" => Ok(OutputFormat::Table),
            "csv" => Ok(OutputFormat::Csv),
            "jsonl" => Ok(OutputFormat::Jsonl),
            "cypherl" => Ok(OutputFormat::Cypherl),
            other => Err(format!(
                "unknown format '{other}' (expected csv, jsonl, cypherl, or table)"
            )),
        }
    }
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

    /// Start in read-only mode: every transaction runs with Bolt access mode
    /// READ, so the server rejects writes. Can be turned on later with
    /// `:set readonly on`, but turned off only at connect time or via a profile.
    #[arg(long)]
    pub read_only: bool,

    /// Output format for query results. With no flag the default depends on
    /// stdout (issue 19, ADR 0014): `table` at a terminal, `jsonl` when piped or
    /// redirected. An explicit value always wins.
    #[arg(long, value_enum)]
    pub output_format: Option<OutputFormat>,

    /// Truncate tabular output to fit the terminal width.
    #[arg(long)]
    pub fit_to_screen: bool,

    /// Result display mode: tabular, vertical, or auto (tabular until a row
    /// exceeds the terminal width, then vertical). Overrides the built-in default
    /// (auto); a runtime `:set display` overrides this in turn.
    #[arg(long, value_parser = parse_display_mode)]
    pub display: Option<DisplayMode>,

    /// Connect using a named profile from `config.toml` (`[profiles.<name>]`):
    /// its endpoint/auth/TLS drive the connection and its setting overrides apply
    /// at the config-file precedence layer. Explicit connection flags still win.
    #[arg(long)]
    pub profile: Option<String>,

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

    /// Run a single query string and exit (issue 21), like `psql -c` /
    /// `cypher-shell --command`. Non-interactive regardless of stdin: it bypasses
    /// both interactive Frontends and runs through the serial path, honouring the
    /// TTY-aware output default (issue 19). Several `;`-separated statements run in
    /// order. Composes with the connection flags and `--profile`.
    #[arg(short = 'c', long = "command")]
    pub one_shot: Option<String>,

    /// An optional subcommand (issue 20). With none, the bare invocation runs the
    /// interactive Frontend (TTY stdin) or the serial path over piped stdin.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// The explicit subcommands (issue 20, ADR 0014). Additive: the bare top-level
/// invocation keeps its behaviour; a subcommand makes the non-interactive path
/// explicit so committed scripts and CI need not rely on stdin being a pipe.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run one or more cypherl sources in order through the serial path. `-` means
    /// stdin, so `mgconsole run -` is the explicit spelling of `… | mgconsole`.
    /// Batch options (`--import-mode`/`--output-format`/csv) are the same top-level
    /// flags; place them before `run`.
    Run {
        /// Files to run in argument order; `-` (which may be mixed in) is stdin.
        files: Vec<String>,
    },
}

/// A source of queries for a non-interactive run: standard input, a named file
/// (issue 20), or an inline query string (`-c`, issue 21). `-` resolves to
/// [`Stdin`](QuerySource::Stdin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuerySource {
    Stdin,
    File(PathBuf),
    Inline(String),
}

/// Resolve the non-interactive query sources from `-c`, the parsed subcommand, and
/// whether stdin is a terminal, or `None` for the interactive Frontend. Pure, so
/// the routing is unit-tested. Precedence: `-c` wins and is always non-interactive
/// (issue 21); else `run` names its sources (`-` → stdin, possibly mixed, issue
/// 20); else a bare invocation reads stdin only when it is *not* a TTY (the
/// `… | mgconsole` sugar for `run -`), and otherwise goes interactive.
pub fn resolve_sources(
    one_shot: Option<&str>,
    command: Option<&Command>,
    stdin_is_tty: bool,
) -> Option<Vec<QuerySource>> {
    if let Some(query) = one_shot {
        return Some(vec![QuerySource::Inline(query.to_string())]);
    }
    match command {
        Some(Command::Run { files }) => {
            // A bare `run` with no files reads stdin, like `run -`.
            if files.is_empty() {
                return Some(vec![QuerySource::Stdin]);
            }
            Some(
                files
                    .iter()
                    .map(|f| {
                        if f == "-" {
                            QuerySource::Stdin
                        } else {
                            QuerySource::File(PathBuf::from(f))
                        }
                    })
                    .collect(),
            )
        }
        None if !stdin_is_tty => Some(vec![QuerySource::Stdin]),
        None => None,
    }
}

/// clap value parser for `--display`, deferring to [`DisplayMode`]'s `FromStr` so
/// the flag and the runtime `:set display` accept exactly the same spellings.
fn parse_display_mode(value: &str) -> Result<DisplayMode, String> {
    value.parse()
}

/// Which connection flags were given explicitly on the command line, as opposed
/// to left at their clap default. A profile fills only the flags the user did not
/// set, so an explicit flag always wins over the profile (issue 03 precedence).
// One bool per connection flag, a flat 1:1 map like [`Cli`] itself.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ExplicitFlags {
    pub host: bool,
    pub port: bool,
    pub username: bool,
    pub password: bool,
    pub use_ssl: bool,
}

/// The resolved *where* and *how* of a connection after folding flag, profile,
/// and default together. `readonly` is carried for issue 04 to enforce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connection {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub use_ssl: bool,
    pub readonly: bool,
}

/// Resolve the connection from flag > profile > default. An explicit flag wins;
/// otherwise a selected profile's value fills in; otherwise the clap default
/// (already sitting in `cli`'s field) stands. `readonly` has no flag yet, so it
/// comes from the profile (defaulting to off).
pub fn resolve_connection(
    cli: &Cli,
    explicit: &ExplicitFlags,
    profile: Option<&Profile>,
) -> Connection {
    fn pick<T>(explicit: bool, flag: T, from_profile: Option<T>) -> T {
        if explicit {
            flag
        } else {
            from_profile.unwrap_or(flag)
        }
    }
    Connection {
        host: pick(
            explicit.host,
            cli.host.clone(),
            profile.and_then(|p| p.host.clone()),
        ),
        port: pick(explicit.port, cli.port, profile.and_then(|p| p.port)),
        username: pick(
            explicit.username,
            cli.username.clone(),
            profile.and_then(|p| p.username.clone()),
        ),
        password: pick(
            explicit.password,
            cli.password.clone(),
            profile.and_then(|p| p.password.clone()),
        ),
        use_ssl: pick(explicit.use_ssl, cli.use_ssl, profile.and_then(|p| p.use_ssl)),
        // `--read-only` only ever turns the guard *on*; with the flag unset the
        // profile decides (it can also turn it off, the connect-time exception).
        readonly: cli.read_only || profile.and_then(|p| p.readonly).unwrap_or(false),
    }
}

/// Where a `:connect` is being pointed and how the swap is labelled afterwards
/// (issue 07). Carries the new Endpoint and connect options for the swap, plus the
/// profile name to show (`None` for a bare-endpoint connect). Not `Debug`/`Eq`:
/// it carries `ConnectOptions` (with credentials), which are neither.
pub struct ConnectTarget {
    pub endpoint: Endpoint,
    pub options: ConnectOptions,
    pub profile: Option<String>,
}

/// Resolve a `:connect <target>` argument (issue 07). A `target` that matches a
/// known profile name connects by that profile (its endpoint/auth/TLS/readonly);
/// otherwise it is parsed as a bare `host[:port]`, reusing the *current* Session's
/// auth/TLS/read-only and falling back to its host/port for an omitted part.
pub fn resolve_connect_target(
    target: &str,
    config: &Config,
    current_endpoint: &Endpoint,
    current_options: &ConnectOptions,
) -> Result<ConnectTarget, String> {
    let target = target.trim();
    if target.is_empty() {
        return Err(":connect needs a profile name or host[:port]".to_string());
    }
    if let Some(profile) = config.profiles.get(target) {
        let endpoint = Endpoint::new(
            profile
                .host
                .clone()
                .unwrap_or_else(|| current_endpoint.host().to_string()),
            profile.port.unwrap_or_else(|| current_endpoint.port()),
        );
        let options = ConnectOptions {
            credentials: profile.username.clone().map(|username| Credentials {
                username,
                password: profile.password.clone().unwrap_or_default(),
            }),
            use_tls: profile.use_ssl.unwrap_or(current_options.use_tls),
            read_only: profile.readonly.unwrap_or(false),
        };
        return Ok(ConnectTarget {
            endpoint,
            options,
            profile: Some(target.to_string()),
        });
    }
    let endpoint = parse_endpoint(target, current_endpoint)?;
    Ok(ConnectTarget {
        endpoint,
        // A bare endpoint reuses the current auth/TLS/read-only — the most useful
        // default when hopping between servers in one deployment.
        options: current_options.clone(),
        profile: None,
    })
}

/// Parse a `host[:port]` endpoint, defaulting the port to the current Session's
/// when omitted. A missing host or an unparseable port is a clear error.
fn parse_endpoint(target: &str, current: &Endpoint) -> Result<Endpoint, String> {
    let (host, port) = match target.rsplit_once(':') {
        Some((host, port)) => {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("invalid port in '{target}'"))?;
            (host, port)
        }
        None => (target, current.port()),
    };
    if host.is_empty() {
        return Err(format!("missing host in '{target}'"));
    }
    Ok(Endpoint::new(host.to_string(), port))
}

/// The environment variable that supplies the password non-interactively (issue
/// 24), alongside the other `MGCONSOLE_*` overrides. It sits between the explicit
/// `--password` flag and a profile password in [`resolve_password`]'s precedence,
/// so a script can authenticate without leaking the password to shell history or
/// `ps` (cf. `PGPASSWORD`).
pub const PASSWORD_ENV: &str = "MGCONSOLE_PASSWORD";

/// Resolve the password to authenticate with, folding the non-interactive sources
/// into one precedence chain and prompting only as the last resort.
///
/// Precedence (issue 24): explicit `--password` flag > `MGCONSOLE_PASSWORD` env >
/// profile password > hidden prompt. The first non-empty source wins, so a script
/// can export `MGCONSOLE_PASSWORD` and connect with no prompt and no CLI leak
/// (cf. `PGPASSWORD`), while an explicit `--password` still takes precedence. Each
/// source is passed in separately (rather than pre-merged) precisely so env can sit
/// *between* the flag and the profile.
///
/// The hidden, no-echo prompt is terminal IO, so it is injected as `prompt`:
/// `main` passes a real no-echo reader; tests pass a closure. The env value is read
/// once at the call site and passed in, so this stays pure and testable. An empty
/// username means an anonymous (unauthenticated) connection, so no password is
/// sought — `MGCONSOLE_PASSWORD` is ignored entirely, no prompt fires. A prompt
/// that yields an empty password fails with a clear message rather than attempting
/// a doomed empty login.
pub fn resolve_password(
    username: &str,
    flag_password: Option<&str>,
    env_password: Option<&str>,
    profile_password: Option<&str>,
    prompt: impl FnOnce() -> std::io::Result<String>,
) -> Result<String, String> {
    // Anonymous connection: no password is sought and the env var is ignored.
    if username.is_empty() {
        return Ok(String::new());
    }
    // flag > env > profile: the first source that carries a non-empty password.
    let chosen = [flag_password, env_password, profile_password]
        .into_iter()
        .flatten()
        .find(|p| !p.is_empty());
    if let Some(password) = chosen {
        return Ok(password.to_string());
    }
    // Nothing supplied a password for a named user: fall back to the prompt.
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
        assert!(!cli.read_only, "read-only is opt-in");
        assert_eq!(cli.output_format, None, "no flag: the TTY-aware default applies");
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
        assert!(cli.command.is_none(), "bare invocation has no subcommand");
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
    fn profile_flag_parses() {
        assert_eq!(parse(&[]).expect("bare").profile, None);
        assert_eq!(
            parse(&["--profile", "prod"]).expect("prod").profile.as_deref(),
            Some("prod")
        );
    }

    #[test]
    fn connection_resolves_flag_over_profile_over_default() {
        let profile = Profile {
            host: Some("db.example.com".to_string()),
            port: Some(7688),
            username: Some("neo".to_string()),
            use_ssl: Some(false),
            readonly: Some(true),
            ..Profile::default()
        };
        // No explicit flags: the profile fills in, default stands where the
        // profile is silent (password).
        let from_profile = resolve_connection(
            &parse(&[]).expect("bare"),
            &ExplicitFlags::default(),
            Some(&profile),
        );
        assert_eq!(from_profile.host, "db.example.com");
        assert_eq!(from_profile.port, 7688);
        assert_eq!(from_profile.username, "neo");
        assert_eq!(from_profile.password, "", "profile silent → default empty");
        assert!(!from_profile.use_ssl);
        assert!(from_profile.readonly);

        // An explicit --host wins over the profile.
        let explicit = ExplicitFlags {
            host: true,
            ..ExplicitFlags::default()
        };
        let overridden = resolve_connection(
            &parse(&["--host", "localhost"]).expect("host"),
            &explicit,
            Some(&profile),
        );
        assert_eq!(overridden.host, "localhost", "explicit flag beats profile");
        assert_eq!(overridden.port, 7688, "unset flag still takes the profile");
    }

    #[test]
    fn read_only_comes_from_the_flag_or_the_profile() {
        // The flag forces it on.
        let conn = resolve_connection(
            &parse(&["--read-only"]).expect("flag"),
            &ExplicitFlags::default(),
            None,
        );
        assert!(conn.readonly);
        // A profile can pin it on with no flag...
        let ro_profile = Profile {
            readonly: Some(true),
            ..Profile::default()
        };
        assert!(resolve_connection(
            &parse(&[]).expect("bare"),
            &ExplicitFlags::default(),
            Some(&ro_profile)
        )
        .readonly);
        // ...or leave it off.
        let rw_profile = Profile {
            readonly: Some(false),
            ..Profile::default()
        };
        assert!(!resolve_connection(
            &parse(&[]).expect("bare"),
            &ExplicitFlags::default(),
            Some(&rw_profile)
        )
        .readonly);
    }

    #[test]
    fn connection_with_no_profile_uses_the_flag_defaults() {
        let conn = resolve_connection(
            &parse(&[]).expect("bare"),
            &ExplicitFlags::default(),
            None,
        );
        assert_eq!(conn.host, "127.0.0.1");
        assert_eq!(conn.port, 7687);
        assert!(conn.use_ssl);
        assert!(!conn.readonly);
    }

    #[test]
    fn connect_resolves_a_bare_endpoint_reusing_current_auth() {
        let current = Endpoint::new("127.0.0.1", 7687);
        let options = ConnectOptions {
            use_tls: true,
            ..ConnectOptions::default()
        };
        let config = Config::default();
        // host:port given.
        let t = resolve_connect_target("db.example.com:7690", &config, &current, &options)
            .expect("endpoint");
        assert_eq!(t.endpoint, Endpoint::new("db.example.com", 7690));
        assert!(t.options.use_tls, "auth/TLS reused from current");
        assert_eq!(t.profile, None);
        // bare host reuses the current port.
        let t = resolve_connect_target("other", &config, &current, &options).expect("host only");
        assert_eq!(t.endpoint, Endpoint::new("other", 7687));
    }

    #[test]
    fn connect_resolves_a_known_profile() {
        let current = Endpoint::new("127.0.0.1", 7687);
        let options = ConnectOptions::default();
        let mut config = Config::default();
        config.profiles.insert(
            "prod".to_string(),
            Profile {
                host: Some("prod.db".to_string()),
                port: Some(7688),
                username: Some("neo".to_string()),
                readonly: Some(true),
                ..Profile::default()
            },
        );
        let t = resolve_connect_target("prod", &config, &current, &options).expect("profile");
        assert_eq!(t.endpoint, Endpoint::new("prod.db", 7688));
        assert_eq!(t.profile.as_deref(), Some("prod"));
        assert!(t.options.read_only);
        assert_eq!(
            t.options.credentials.as_ref().map(|c| c.username.as_str()),
            Some("neo")
        );
    }

    #[test]
    fn connect_rejects_an_empty_or_bad_target() {
        let current = Endpoint::new("127.0.0.1", 7687);
        let options = ConnectOptions::default();
        let config = Config::default();
        assert!(resolve_connect_target("", &config, &current, &options).is_err());
        assert!(resolve_connect_target("host:notaport", &config, &current, &options).is_err());
        assert!(resolve_connect_target(":7687", &config, &current, &options).is_err());
    }

    #[test]
    fn jsonl_is_accepted_as_output_format() {
        let cli = parse(&["--output-format", "jsonl"]).expect("jsonl parses");
        assert_eq!(cli.output_format, Some(OutputFormat::Jsonl));
    }

    #[test]
    fn every_output_format_parses() {
        for (text, want) in [
            ("tabular", OutputFormat::Table),
            ("csv", OutputFormat::Csv),
            ("jsonl", OutputFormat::Jsonl),
            ("cypherl", OutputFormat::Cypherl),
        ] {
            let cli = parse(&["--output-format", text]).expect("format parses");
            assert_eq!(cli.output_format, Some(want));
        }
    }

    #[test]
    fn run_subcommand_parses_files_in_order() {
        let cli = parse(&["run", "a.cypherl", "b.cypherl"]).expect("run parses");
        match cli.command {
            Some(Command::Run { files }) => assert_eq!(files, vec!["a.cypherl", "b.cypherl"]),
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn batch_flags_precede_the_run_subcommand() {
        let cli = parse(&["--output-format", "csv", "run", "a.cypherl"]).expect("parses");
        assert_eq!(cli.output_format, Some(OutputFormat::Csv));
        assert!(matches!(cli.command, Some(Command::Run { .. })));
    }

    #[test]
    fn sources_resolve_from_the_command_and_stdin_tty() {
        use QuerySource::{File, Stdin};
        // `run` names its sources; `-` is stdin and may be mixed in at its position.
        let run = Command::Run {
            files: vec!["a.cypherl".to_string(), "-".to_string(), "b.cypherl".to_string()],
        };
        assert_eq!(
            resolve_sources(None, Some(&run), true),
            Some(vec![
                File(PathBuf::from("a.cypherl")),
                Stdin,
                File(PathBuf::from("b.cypherl")),
            ]),
            "run resolves regardless of stdin TTY (you can run from a terminal)"
        );
        // `run -` is the explicit spelling of `… | mgconsole`.
        let run_dash = Command::Run { files: vec!["-".to_string()] };
        assert_eq!(resolve_sources(None, Some(&run_dash), true), Some(vec![Stdin]));
        // A bare `run` reads stdin, like `run -`.
        let run_bare = Command::Run { files: vec![] };
        assert_eq!(resolve_sources(None, Some(&run_bare), true), Some(vec![Stdin]));
        // No subcommand: a piped stdin is sugar for `run -`; a TTY goes interactive.
        assert_eq!(resolve_sources(None, None, false), Some(vec![Stdin]));
        assert_eq!(resolve_sources(None, None, true), None, "interactive");
    }

    #[test]
    fn the_command_flag_runs_one_shot_and_bypasses_the_frontends() {
        use QuerySource::Inline;
        // `-c` is non-interactive regardless of stdin TTY, and wins over a
        // subcommand and the piped-stdin default (issue 21).
        assert_eq!(
            resolve_sources(Some("RETURN 1"), None, true),
            Some(vec![Inline("RETURN 1".to_string())]),
            "-c bypasses the interactive frontend even at a terminal"
        );
        let run = Command::Run { files: vec!["a.cypherl".to_string()] };
        assert_eq!(
            resolve_sources(Some("RETURN 1"), Some(&run), false),
            Some(vec![Inline("RETURN 1".to_string())]),
            "-c takes precedence over run"
        );
    }

    #[test]
    fn the_command_flag_parses_with_its_short_and_long_forms() {
        assert_eq!(parse(&["-c", "RETURN 1"]).expect("short").one_shot.as_deref(), Some("RETURN 1"));
        assert_eq!(
            parse(&["--command", "RETURN 2"]).expect("long").one_shot.as_deref(),
            Some("RETURN 2")
        );
        assert_eq!(parse(&[]).expect("none").one_shot, None);
    }

    #[test]
    fn the_default_output_format_is_tty_aware_and_an_explicit_flag_wins() {
        // No flag: tabular at a terminal, jsonl when piped/redirected (ADR 0014).
        assert_eq!(OutputFormat::resolve(None, true), OutputFormat::Table);
        assert_eq!(OutputFormat::resolve(None, false), OutputFormat::Jsonl);
        // An explicit choice wins in both directions.
        assert_eq!(
            OutputFormat::resolve(Some(OutputFormat::Table), false),
            OutputFormat::Table,
            "explicit table into a pipe still tabulates"
        );
        assert_eq!(
            OutputFormat::resolve(Some(OutputFormat::Jsonl), true),
            OutputFormat::Jsonl
        );
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
    fn anonymous_connection_never_prompts_and_ignores_the_env() {
        // An empty username is anonymous: MGCONSOLE_PASSWORD is ignored entirely,
        // no prompt fires, and the password is empty.
        let pw = resolve_password("", Some("flagpw"), Some("envpw"), Some("profpw"), never_prompts)
            .expect("anonymous ok");
        assert_eq!(pw, "");
    }

    #[test]
    fn explicit_flag_password_wins_over_env_and_profile() {
        let pw = resolve_password(
            "alice",
            Some("secret"),
            Some("envpw"),
            Some("profpw"),
            never_prompts,
        )
        .expect("explicit ok");
        assert_eq!(pw, "secret", "the --password flag wins");
    }

    #[test]
    fn env_password_beats_the_profile_but_loses_to_the_flag() {
        // No flag: the env var authenticates with no prompt, over the profile.
        let pw = resolve_password("alice", None, Some("envpw"), Some("profpw"), never_prompts)
            .expect("env ok");
        assert_eq!(pw, "envpw");
    }

    #[test]
    fn the_profile_password_is_used_when_neither_flag_nor_env_is_set() {
        let pw = resolve_password("alice", None, None, Some("profpw"), never_prompts)
            .expect("profile ok");
        assert_eq!(pw, "profpw");
    }

    #[test]
    fn username_with_no_flag_env_or_profile_prompts() {
        let pw = resolve_password("alice", None, None, None, || Ok("typed".to_string()))
            .expect("prompt ok");
        assert_eq!(pw, "typed");
    }

    #[test]
    fn empty_prompted_password_fails_clearly() {
        let err = resolve_password("alice", None, None, None, || Ok(String::new()))
            .expect_err("empty password rejected");
        assert!(err.contains("alice"), "message names the user: {err}");
    }

    #[test]
    fn output_format_vocabulary_parses_and_infers() {
        use std::path::Path;
        // FromStr accepts the shared vocabulary (and the `tabular` alias).
        assert_eq!("csv".parse(), Ok(OutputFormat::Csv));
        assert_eq!("table".parse(), Ok(OutputFormat::Table));
        assert_eq!("tabular".parse(), Ok(OutputFormat::Table));
        assert!("xml".parse::<OutputFormat>().is_err());
        // Extension inference.
        assert_eq!(OutputFormat::from_extension(Path::new("a.csv")), Some(OutputFormat::Csv));
        assert_eq!(OutputFormat::from_extension(Path::new("a.jsonl")), Some(OutputFormat::Jsonl));
        assert_eq!(
            OutputFormat::from_extension(Path::new("a.cypherl")),
            Some(OutputFormat::Cypherl)
        );
        assert_eq!(OutputFormat::from_extension(Path::new("a.weird")), None);
        // Display round-trips.
        assert_eq!(OutputFormat::Cypherl.to_string(), "cypherl");
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
