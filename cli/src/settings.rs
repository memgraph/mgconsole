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

/// The resolved console settings, shared by every Frontend so `:set display`
/// means the same thing in the REPL and the Workbench.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    /// How a buffered result is laid out (`tabular`/`vertical`/`auto`).
    pub display: DisplayMode,
}

impl Settings {
    /// Resolve the default and CLI-flag layers: start from the built-in defaults
    /// and apply each flag that was given (a `None` flag leaves the default).
    pub fn from_cli(display: Option<DisplayMode>) -> Self {
        let mut settings = Self::default();
        if let Some(display) = display {
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
            other => Err(format!("unknown setting '{other}'")),
        }
    }

    /// The current value of one setting by name, or `None` if the name is unknown.
    pub fn get(&self, name: &str) -> Option<String> {
        match name {
            "display" => Some(self.display.to_string()),
            _ => None,
        }
    }

    /// Every setting as a `(name, value)` pair, in listing order.
    pub fn entries(&self) -> Vec<(&'static str, String)> {
        vec![("display", self.display.to_string())]
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
    fn a_cli_flag_overrides_the_default() {
        // No flag keeps the default; a flag wins.
        assert_eq!(Settings::from_cli(None).display, DisplayMode::Auto);
        assert_eq!(
            Settings::from_cli(Some(DisplayMode::Vertical)).display,
            DisplayMode::Vertical
        );
    }

    #[test]
    fn a_runtime_set_overrides_the_cli_layer() {
        let mut settings = Settings::from_cli(Some(DisplayMode::Tabular));
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
        assert_eq!(Settings::default().list(), "display = auto");
    }
}
