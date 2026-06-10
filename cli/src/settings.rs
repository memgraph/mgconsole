//! The console Setting store (issue 01): the spine every `:set`-able value hangs
//! off, kept rigorously separate from the `:param` query-data store (CONTEXT.md).
//!
//! A Setting controls how the console *behaves* (result display mode today; more
//! land in later issues), as opposed to query data. Values resolve by precedence
//! — built-in default < CLI flag < runtime `:set` — with the config-file layer
//! slotting in at issue 02. [`Settings::from_cli`] folds the default and CLI
//! layers; [`Settings::set`] applies a runtime override, validating the name and
//! value and returning a message rather than panicking, so a bad `:set` never
//! ends the session.

use mgconsole_core::DisplayMode;

/// The resolved console settings, the one `:set` store every Frontend reads.
/// Each Frontend honours the settings its render model has a use for: `display`
/// governs the buffered render (REPL + non-interactive), while the Workbench
/// shows a live navigable table and ignores it (CONTEXT.md "Display mode");
/// `theme` is the Workbench's and the REPL stores it without repainting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The buffered-render layout (`tabular`/`vertical`/`auto`); consulted by the
    /// REPL and the non-interactive path, not the Workbench's live table.
    pub display: DisplayMode,
    /// The active Workbench theme name (issue 14): the built-in base palette
    /// `:set theme` switches between (`default`/`mono`). The REPL stores it for
    /// consistency but does not repaint on it.
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            display: DisplayMode::default(),
            theme: "default".to_string(),
        }
    }
}

/// The config-file layer of the precedence chain (issue 02): each setting is
/// optional, an absent value meaning "no override at this layer". Populated by
/// the `config.toml` `[settings]` reader and folded in by [`Settings::resolve`]
/// between the built-in defaults and the CLI flags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileSettings {
    pub display: Option<DisplayMode>,
    pub theme: Option<String>,
}

impl FileSettings {
    /// Overlay `other` onto `self`, with `other` winning for every value it
    /// supplies — used to layer a selected profile's `[settings]` over the
    /// top-level `[settings]` (issue 03), both at the config-file precedence layer.
    #[must_use]
    pub fn overlay(&self, other: &FileSettings) -> FileSettings {
        FileSettings {
            display: other.display.or(self.display),
            theme: other.theme.clone().or_else(|| self.theme.clone()),
        }
    }
}

impl Settings {
    /// Resolve the full precedence chain below runtime `:set`: built-in default <
    /// config file < CLI flag. Each later layer overrides only the values it
    /// actually supplies (`None`/absent leaves the layer below intact). The
    /// runtime `:set` layer is applied later, in each Frontend's loop.
    pub fn resolve(file: &FileSettings, cli_display: Option<DisplayMode>) -> Self {
        let mut settings = Self::default();
        if let Some(display) = file.display {
            settings.display = display;
        }
        // The theme has no CLI flag (issue 14: default < config < runtime `:set`).
        if let Some(theme) = &file.theme {
            settings.theme.clone_from(theme);
        }
        if let Some(display) = cli_display {
            settings.display = display;
        }
        settings
    }

    /// Apply a runtime `:set <name> <value>`. An unknown name or an invalid value
    /// is reported as an `Err` message the Frontend surfaces without losing the
    /// session; the store is left untouched on error.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
        match name {
            "display" => {
                self.display = value.parse()?;
                Ok(())
            }
            // The theme name is validated against the built-in set (issue 14) so a
            // typo is rejected rather than silently leaving the palette unchanged.
            "theme" => {
                let value = value.trim().to_ascii_lowercase();
                if crate::theme::builtin_palette(&value).is_none() {
                    return Err(format!(
                        "unknown theme '{value}' (known: {})",
                        crate::theme::BUILTIN_THEMES.join(", ")
                    ));
                }
                self.theme = value;
                Ok(())
            }
            other => Err(format!("unknown setting '{other}'")),
        }
    }

    /// The current value of one setting by name, or `None` if the name is unknown.
    pub fn get(&self, name: &str) -> Option<String> {
        match name {
            "display" => Some(self.display.to_string()),
            "theme" => Some(self.theme.clone()),
            _ => None,
        }
    }

    /// Every setting as a `(name, value)` pair, in listing order.
    pub fn entries(&self) -> Vec<(&'static str, String)> {
        vec![
            ("display", self.display.to_string()),
            ("theme", self.theme.clone()),
        ]
    }

    /// A multi-line `name = value` listing for `:set` with no argument.
    pub fn list(&self) -> String {
        self.entries()
            .iter()
            .map(|(name, value)| format!("{name} = {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_core_defaults() {
        assert_eq!(Settings::default().display, DisplayMode::Auto);
    }

    #[test]
    fn the_precedence_chain_is_default_then_config_then_cli() {
        let none = FileSettings::default();
        let config = FileSettings {
            display: Some(DisplayMode::Vertical),
            ..FileSettings::default()
        };
        // Default only.
        assert_eq!(Settings::resolve(&none, None).display, DisplayMode::Auto);
        // Config overrides the default.
        assert_eq!(Settings::resolve(&config, None).display, DisplayMode::Vertical);
        // CLI flag overrides the config layer.
        assert_eq!(
            Settings::resolve(&config, Some(DisplayMode::Tabular)).display,
            DisplayMode::Tabular
        );
        // A CLI flag with no config still wins over the default.
        assert_eq!(
            Settings::resolve(&none, Some(DisplayMode::Tabular)).display,
            DisplayMode::Tabular
        );
    }

    #[test]
    fn a_runtime_set_overrides_the_cli_layer() {
        let mut settings = Settings::resolve(&FileSettings::default(), Some(DisplayMode::Tabular));
        settings.set("display", "vertical").expect("valid");
        assert_eq!(settings.display, DisplayMode::Vertical);
    }

    #[test]
    fn an_unknown_setting_is_rejected_and_the_store_is_untouched() {
        let mut settings = Settings::default();
        let err = settings.set("colour", "blue").expect_err("unknown name");
        assert!(err.contains("colour"));
        assert_eq!(settings, Settings::default(), "store untouched on error");
    }

    #[test]
    fn an_invalid_value_is_rejected_and_the_store_is_untouched() {
        let mut settings = Settings::default();
        let err = settings.set("display", "grid").expect_err("invalid value");
        assert!(err.contains("grid"));
        assert_eq!(settings.display, DisplayMode::Auto, "unchanged on error");
    }

    #[test]
    fn listing_reads_as_name_equals_value() {
        assert_eq!(Settings::default().list(), "display = auto\ntheme = default");
    }

    #[test]
    fn the_theme_setting_validates_against_the_builtins_and_resolves_by_precedence() {
        // Default is `default`; config can pin another built-in; runtime `:set`
        // wins (issue 14: default < config < runtime, no CLI flag).
        assert_eq!(Settings::default().theme, "default");
        let config = FileSettings {
            theme: Some("mono".to_string()),
            ..FileSettings::default()
        };
        let mut settings = Settings::resolve(&config, None);
        assert_eq!(settings.theme, "mono", "config pins the theme");
        settings.set("theme", "default").expect("valid built-in");
        assert_eq!(settings.theme, "default", "runtime :set wins");
        // An unknown theme is rejected and the store is untouched.
        let err = settings.set("theme", "neon").expect_err("unknown theme");
        assert!(err.contains("neon"), "{err}");
        assert_eq!(settings.theme, "default", "unchanged on error");
    }
}
