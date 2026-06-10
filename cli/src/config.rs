//! The hand-edited `~/.mgconsole/config.toml` reader (issue 02, ADR 0012).
//!
//! Where the config lives is decided by [`resolve_config_path`] — a pure function
//! of the `MGCONSOLE_CONFIG_PATH` override and the home directory, mirroring the
//! history resolver — so precedence is tested without the filesystem. [`load`]
//! reads and parses the file's `[settings]` table into a [`FileSettings`] overlay
//! that slots between the built-in defaults and the CLI flags
//! ([`crate::settings::Settings::resolve`]). A missing file is the normal case
//! (no override, defaults apply); a malformed file is a clear error the caller
//! reports before falling back to defaults, so the console always starts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::settings::FileSettings;

/// The environment variable that overrides the config-file location, alongside
/// `MGCONSOLE_HISTORY_PATH`.
pub const CONFIG_ENV: &str = "MGCONSOLE_CONFIG_PATH";

/// The home-relative state directory (ADR 0012); the config file lives inside it.
const DEFAULT_SUBDIR: &str = ".mgconsole";

/// The config file kept inside the state directory.
pub const CONFIG_FILENAME: &str = "config.toml";

/// Resolve the config-file path: the `MGCONSOLE_CONFIG_PATH` override wins and is
/// taken literally (the shell expands a typed `~`); otherwise the default is
/// `~/.mgconsole/config.toml` expanded against `home`. With neither an override
/// nor a known home, there is no config path (`None`) and defaults apply.
pub fn resolve_config_path(env: Option<&str>, home: Option<&Path>) -> Option<PathBuf> {
    match (env, home) {
        (Some(path), _) => Some(PathBuf::from(path)),
        (None, Some(home)) => Some(home.join(DEFAULT_SUBDIR).join(CONFIG_FILENAME)),
        (None, None) => None,
    }
}

/// A connection profile (issue 03): a named bundle of the *where* and *how* of a
/// connection, stored as `[profiles.<name>]`. Each field is optional — an absent
/// value falls through to the CLI flag / built-in default in the connection
/// precedence chain. `settings` carries per-profile Setting overrides that sit at
/// the config-file layer (overlaying the top-level `[settings]`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub use_ssl: Option<bool>,
    /// Whether the profile pins read-only mode (enforced by issue 04; parsed and
    /// carried here so the profile is the single source of the connection bundle).
    pub readonly: Option<bool>,
    pub settings: FileSettings,
}

/// The whole parsed `config.toml`: the top-level `[settings]` overlay and the
/// named connection profiles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub settings: FileSettings,
    pub profiles: BTreeMap<String, Profile>,
}

impl Config {
    /// Select a profile by name, or fail fast with a message listing the known
    /// profiles (issue 03: an unknown `--profile` is a clear, recoverable error).
    pub fn select(&self, name: &str) -> Result<&Profile, String> {
        self.profiles.get(name).ok_or_else(|| {
            let known = if self.profiles.is_empty() {
                "none defined".to_string()
            } else {
                self.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
            };
            format!("unknown profile '{name}' (known profiles: {known})")
        })
    }
}

/// The raw shape of `config.toml` for serde. Unknown keys are rejected so a typo
/// is surfaced rather than silently ignored; both tables are optional.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    settings: RawSettings,
    #[serde(default)]
    profiles: BTreeMap<String, RawProfile>,
}

/// The `[settings]` table. Each value is a string so it shares the exact
/// vocabulary the runtime `:set` accepts (validated by the setting's `FromStr`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    display: Option<String>,
}

/// A `[profiles.<name>]` table: connection fields plus a nested `[settings]`
/// override table mirroring the top-level one.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    use_ssl: Option<bool>,
    readonly: Option<bool>,
    #[serde(default)]
    settings: RawSettings,
}

/// Load and parse the config file at `path`.
///
/// A missing file yields an empty [`Config`] (defaults apply) — the normal
/// first-run case, not an error. A read error, a TOML syntax error, or an invalid
/// setting value is returned as a clear, path-naming `Err` the caller reports
/// before continuing on defaults.
pub fn load(path: &Path) -> Result<Config, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(format!("could not read config {}: {e}", path.display())),
    };
    parse(&text).map_err(|e| format!("invalid config {}: {e}", path.display()))
}

/// Parse a `[settings]` table into a [`FileSettings`] overlay, validating each
/// value against its own `FromStr`.
fn settings_from_raw(raw: RawSettings) -> Result<FileSettings, String> {
    let display = match raw.display {
        Some(value) => Some(value.parse().map_err(|e: String| e)?),
        None => None,
    };
    Ok(FileSettings { display })
}

/// Parse config text into a [`Config`], validating each setting value against its
/// own `FromStr`. Split from [`load`] so parsing and validation are tested
/// without the filesystem.
fn parse(text: &str) -> Result<Config, String> {
    let raw: RawConfig = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let settings = settings_from_raw(raw.settings)?;
    let mut profiles = BTreeMap::new();
    for (name, raw_profile) in raw.profiles {
        let profile = Profile {
            host: raw_profile.host,
            port: raw_profile.port,
            username: raw_profile.username,
            password: raw_profile.password,
            use_ssl: raw_profile.use_ssl,
            readonly: raw_profile.readonly,
            settings: settings_from_raw(raw_profile.settings)
                .map_err(|e| format!("profile '{name}': {e}"))?,
        };
        profiles.insert(name, profile);
    }
    Ok(Config { settings, profiles })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mgconsole_core::DisplayMode;

    fn home() -> PathBuf {
        PathBuf::from("/home/user")
    }

    #[test]
    fn the_default_config_path_is_under_the_state_directory() {
        assert_eq!(
            resolve_config_path(None, Some(&home())),
            Some(PathBuf::from("/home/user/.mgconsole/config.toml"))
        );
    }

    #[test]
    fn the_environment_override_wins_and_is_literal() {
        assert_eq!(
            resolve_config_path(Some("/etc/mg/conf.toml"), Some(&home())),
            Some(PathBuf::from("/etc/mg/conf.toml"))
        );
    }

    #[test]
    fn no_home_and_no_override_means_no_config() {
        assert_eq!(resolve_config_path(None, None), None);
    }

    #[test]
    fn a_settings_table_loads_into_the_overlay() {
        let config = parse("[settings]\ndisplay = \"vertical\"\n").expect("valid");
        assert_eq!(config.settings.display, Some(DisplayMode::Vertical));
    }

    #[test]
    fn an_empty_or_settings_less_config_is_empty() {
        assert_eq!(parse("").expect("valid"), Config::default());
        assert_eq!(parse("[settings]\n").expect("valid"), Config::default());
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let config = load(Path::new("/no/such/mgconsole/config.toml")).expect("missing is ok");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn a_profile_table_parses_endpoint_auth_tls_readonly_and_settings() {
        let config = parse(
            "[profiles.prod]\n\
             host = \"db.example.com\"\n\
             port = 7688\n\
             username = \"neo\"\n\
             password = \"trinity\"\n\
             use_ssl = true\n\
             readonly = true\n\
             [profiles.prod.settings]\n\
             display = \"vertical\"\n",
        )
        .expect("valid");
        let prod = config.select("prod").expect("prod present");
        assert_eq!(prod.host.as_deref(), Some("db.example.com"));
        assert_eq!(prod.port, Some(7688));
        assert_eq!(prod.username.as_deref(), Some("neo"));
        assert_eq!(prod.password.as_deref(), Some("trinity"));
        assert_eq!(prod.use_ssl, Some(true));
        assert_eq!(prod.readonly, Some(true));
        assert_eq!(prod.settings.display, Some(DisplayMode::Vertical));
    }

    #[test]
    fn a_partial_profile_leaves_absent_fields_none() {
        let config = parse("[profiles.local]\nport = 7687\n").expect("valid");
        let local = config.select("local").expect("present");
        assert_eq!(local.port, Some(7687));
        assert_eq!(local.host, None);
        assert_eq!(local.use_ssl, None);
        assert_eq!(local.settings, FileSettings::default());
    }

    #[test]
    fn an_unknown_profile_lists_the_known_ones() {
        let config = parse("[profiles.a]\n[profiles.b]\n").expect("valid");
        let err = config.select("c").expect_err("unknown");
        assert!(err.contains("'c'"), "names the bad profile: {err}");
        assert!(err.contains("a") && err.contains("b"), "lists known: {err}");
    }

    #[test]
    fn an_invalid_setting_inside_a_profile_is_rejected_with_the_profile_named() {
        let err = parse("[profiles.p.settings]\ndisplay = \"grid\"\n").expect_err("invalid");
        assert!(err.contains("p"), "names the profile: {err}");
        assert!(err.contains("grid"), "names the bad value: {err}");
    }

    #[test]
    fn malformed_toml_is_a_clear_error() {
        let err = parse("this is not = = toml").expect_err("syntax error");
        assert!(!err.is_empty(), "a message is given: {err}");
    }

    #[test]
    fn an_unknown_setting_key_is_rejected() {
        let err = parse("[settings]\nbogus = \"x\"\n").expect_err("unknown key");
        assert!(err.contains("bogus") || err.contains("unknown"), "message: {err}");
    }

    #[test]
    fn an_invalid_setting_value_is_rejected_with_the_valid_set() {
        let err = parse("[settings]\ndisplay = \"grid\"\n").expect_err("invalid value");
        assert!(err.contains("grid"), "message names the bad value: {err}");
    }
}
