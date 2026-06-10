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

/// The raw shape of `config.toml` for serde. Unknown keys are rejected so a typo
/// is surfaced rather than silently ignored; the `[settings]` table is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    settings: RawSettings,
}

/// The `[settings]` table. Each value is a string so it shares the exact
/// vocabulary the runtime `:set` accepts (validated by the setting's `FromStr`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    display: Option<String>,
}

/// Load the config file at `path` into a [`FileSettings`] overlay.
///
/// A missing file yields an empty overlay (defaults apply) — the normal first-run
/// case, not an error. A read error, a TOML syntax error, or an invalid setting
/// value is returned as a clear, path-naming `Err` the caller reports before
/// continuing on defaults.
pub fn load(path: &Path) -> Result<FileSettings, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileSettings::default()),
        Err(e) => return Err(format!("could not read config {}: {e}", path.display())),
    };
    parse(&text).map_err(|e| format!("invalid config {}: {e}", path.display()))
}

/// Parse config text into a [`FileSettings`] overlay, validating each setting
/// value against its own `FromStr`. Split from [`load`] so the parsing and
/// validation are tested without the filesystem.
fn parse(text: &str) -> Result<FileSettings, String> {
    let raw: RawConfig = toml::from_str(text).map_err(|e| e.message().to_string())?;
    let display = match raw.settings.display {
        Some(value) => Some(value.parse().map_err(|e: String| e)?),
        None => None,
    };
    Ok(FileSettings { display })
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
        let overlay = parse("[settings]\ndisplay = \"vertical\"\n").expect("valid");
        assert_eq!(overlay.display, Some(DisplayMode::Vertical));
    }

    #[test]
    fn an_empty_or_settings_less_config_is_an_empty_overlay() {
        assert_eq!(parse("").expect("valid"), FileSettings::default());
        assert_eq!(parse("[settings]\n").expect("valid"), FileSettings::default());
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let overlay = load(Path::new("/no/such/mgconsole/config.toml")).expect("missing is ok");
        assert_eq!(overlay, FileSettings::default());
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
