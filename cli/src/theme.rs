//! Workbench theme + keybinding resolution (issue 14).
//!
//! Two configurable surfaces, both resolved here as pure functions so they are
//! tested with no terminal:
//!
//! - **Theme** — a [`Palette`] mapping each frontend-neutral
//!   [`HighlightCategory`](crate::syntax::HighlightCategory) (slice 04 of
//!   tui-workbench) to a [`ThemeColor`]. Built-in themes are selected by name (the
//!   `theme` Setting, resolved by the issue-02 precedence) and a `[theme]` table in
//!   `config.toml` overrides individual colours on top of the selected base. An
//!   empty/absent table reproduces the built-in `default` (today's appearance).
//! - **Keybindings** — a [`KeyBindings`] map from each rebindable [`Gesture`] to a
//!   [`Chord`]. A `[keys]` table rebinds them; defaults reproduce today's chords.
//!   Unknown gestures, unparseable chords, and conflicting bindings are reported
//!   as warnings and fall back to the default for that gesture.
//!
//! Resolution returns warnings rather than failing, so one bad line never blocks
//! the console (mirroring the "report and continue on defaults" config policy).

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use crate::syntax::HighlightCategory;

/// A terminal colour in the theme vocabulary: the standard ANSI palette plus
/// `Default` (the terminal's own foreground — what an unstyled category uses). A
/// closed vocabulary so a `[theme]` value is validated, not an arbitrary string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeColor {
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
}

impl ThemeColor {
    /// The canonical lowercase name, for `:set`/error messages and round-tripping.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeColor::Default => "default",
            ThemeColor::Black => "black",
            ThemeColor::Red => "red",
            ThemeColor::Green => "green",
            ThemeColor::Yellow => "yellow",
            ThemeColor::Blue => "blue",
            ThemeColor::Magenta => "magenta",
            ThemeColor::Cyan => "cyan",
            ThemeColor::Gray => "gray",
            ThemeColor::DarkGray => "darkgray",
            ThemeColor::LightRed => "lightred",
            ThemeColor::LightGreen => "lightgreen",
            ThemeColor::LightYellow => "lightyellow",
            ThemeColor::LightBlue => "lightblue",
            ThemeColor::LightMagenta => "lightmagenta",
            ThemeColor::LightCyan => "lightcyan",
            ThemeColor::White => "white",
        }
    }
}

impl fmt::Display for ThemeColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ThemeColor {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normal = s.trim().to_ascii_lowercase().replace(['-', '_'], "");
        Ok(match normal.as_str() {
            "default" | "reset" => ThemeColor::Default,
            "black" => ThemeColor::Black,
            "red" => ThemeColor::Red,
            "green" => ThemeColor::Green,
            "yellow" => ThemeColor::Yellow,
            "blue" => ThemeColor::Blue,
            "magenta" => ThemeColor::Magenta,
            "cyan" => ThemeColor::Cyan,
            "gray" | "grey" => ThemeColor::Gray,
            "darkgray" | "darkgrey" => ThemeColor::DarkGray,
            "lightred" => ThemeColor::LightRed,
            "lightgreen" => ThemeColor::LightGreen,
            "lightyellow" => ThemeColor::LightYellow,
            "lightblue" => ThemeColor::LightBlue,
            "lightmagenta" => ThemeColor::LightMagenta,
            "lightcyan" => ThemeColor::LightCyan,
            "white" => ThemeColor::White,
            other => return Err(format!("unknown colour '{other}'")),
        })
    }
}

/// The colour of each highlight category. `Plain` is always the terminal default
/// (so a misspelt keyword renders unstyled and stands out by contrast, matching
/// the REPL), so it is not stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub keyword: ThemeColor,
    pub function: ThemeColor,
    pub string: ThemeColor,
    pub number: ThemeColor,
    pub comment: ThemeColor,
    pub parameter: ThemeColor,
    // UI-element colours (issue 08), beyond the syntax categories: the Workbench
    // chrome the draw used to hardcode. `Default` means "today's appearance" —
    // `border`/`status` fall back to no explicit colour and `selection` to
    // reverse-video — so an absent override changes nothing.
    pub border: ThemeColor,
    pub selection: ThemeColor,
    pub status: ThemeColor,
}

impl Palette {
    /// The colour for one category (`Plain` → terminal default).
    pub fn color(&self, category: HighlightCategory) -> ThemeColor {
        match category {
            HighlightCategory::Keyword => self.keyword,
            HighlightCategory::Function => self.function,
            HighlightCategory::String => self.string,
            HighlightCategory::Number => self.number,
            HighlightCategory::Comment => self.comment,
            HighlightCategory::Parameter => self.parameter,
            HighlightCategory::Plain => ThemeColor::Default,
        }
    }

    /// Set one category's colour by its `[theme]` key name, or fail with a clear
    /// message for an unknown category name.
    fn set_by_name(&mut self, name: &str, color: ThemeColor) -> Result<(), String> {
        match name {
            "keyword" => self.keyword = color,
            "function" => self.function = color,
            "string" => self.string = color,
            "number" => self.number = color,
            "comment" => self.comment = color,
            "parameter" => self.parameter = color,
            // UI-element slots (issue 08).
            "border" => self.border = color,
            "selection" => self.selection = color,
            "status" => self.status = color,
            // `plain` is always the terminal default; accept and ignore an explicit
            // override to it so a config that names it is not an error.
            "plain" if color == ThemeColor::Default => {}
            other => return Err(format!("unknown theme key '{other}'")),
        }
        Ok(())
    }
}

/// The built-in theme names, listed by `:set theme` and validated against.
pub const BUILTIN_THEMES: &[&str] = &["default", "mono", "light"];

/// The built-in `Palette` for a theme name, or `None` for an unknown name.
/// `default` reproduces today's appearance (Cyan chrome, reverse-video selection);
/// `mono` is monochrome (every category and the chrome the terminal default);
/// `light` is tuned for a light-background terminal so e.g. comment grey stays
/// legible. The UI slots (`border`/`selection`/`status`) are carried by each.
pub fn builtin_palette(name: &str) -> Option<Palette> {
    match name {
        "default" => Some(Palette {
            keyword: ThemeColor::Yellow,
            function: ThemeColor::Cyan,
            string: ThemeColor::Green,
            number: ThemeColor::Magenta,
            comment: ThemeColor::DarkGray,
            parameter: ThemeColor::Blue,
            // Today's chrome: Cyan focused borders, reverse-video selection, an
            // unstyled status bar.
            border: ThemeColor::Cyan,
            selection: ThemeColor::Default,
            status: ThemeColor::Default,
        }),
        "mono" => Some(Palette {
            keyword: ThemeColor::Default,
            function: ThemeColor::Default,
            string: ThemeColor::Default,
            number: ThemeColor::Default,
            comment: ThemeColor::Default,
            parameter: ThemeColor::Default,
            // Fully monochrome: even the chrome is the terminal default (selection
            // stays reverse-video, the one cue that needs no colour).
            border: ThemeColor::Default,
            selection: ThemeColor::Default,
            status: ThemeColor::Default,
        }),
        // Tuned for a light background: the darker base ANSI colours read on white
        // where the bright defaults wash out; comment grey stays legible.
        "light" => Some(Palette {
            keyword: ThemeColor::Blue,
            function: ThemeColor::Magenta,
            string: ThemeColor::Green,
            number: ThemeColor::Red,
            comment: ThemeColor::DarkGray,
            parameter: ThemeColor::Cyan,
            border: ThemeColor::Blue,
            selection: ThemeColor::Default,
            status: ThemeColor::Blue,
        }),
        _ => None,
    }
}

/// Resolve the active [`Palette`]: the built-in base named by `base` (falling back
/// to `default` for an unknown name, with a warning), then the `[theme]` overrides
/// applied on top. Each override is validated; a bad key or colour is a warning and
/// is skipped (the base colour stands). Returns the palette and any warnings.
pub fn resolve_palette(base: &str, overrides: &BTreeMap<String, String>) -> (Palette, Vec<String>) {
    let mut warnings = Vec::new();
    let mut palette = builtin_palette(base).unwrap_or_else(|| {
        warnings.push(format!(
            "unknown theme '{base}' (known: {}); using 'default'",
            BUILTIN_THEMES.join(", ")
        ));
        builtin_palette("default").expect("default is built in")
    });
    for (key, value) in overrides {
        match value.parse::<ThemeColor>() {
            Ok(color) => {
                if let Err(message) = palette.set_by_name(key, color) {
                    warnings.push(format!("[theme]: {message}"));
                }
            }
            Err(message) => warnings.push(format!("[theme] {key}: {message}")),
        }
    }
    (palette, warnings)
}

/// A key in a [`Chord`], beyond the modifier flags: a printable character or one of
/// the named non-character keys the workbench distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordKey {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

impl ChordKey {
    fn as_string(self) -> String {
        match self {
            ChordKey::Char(' ') => "space".to_string(),
            ChordKey::Char(c) => c.to_ascii_lowercase().to_string(),
            ChordKey::Enter => "enter".to_string(),
            ChordKey::Esc => "esc".to_string(),
            ChordKey::Tab => "tab".to_string(),
            ChordKey::Backspace => "backspace".to_string(),
            ChordKey::Delete => "delete".to_string(),
            ChordKey::Up => "up".to_string(),
            ChordKey::Down => "down".to_string(),
            ChordKey::Left => "left".to_string(),
            ChordKey::Right => "right".to_string(),
            ChordKey::Home => "home".to_string(),
            ChordKey::End => "end".to_string(),
            ChordKey::PageUp => "pageup".to_string(),
            ChordKey::PageDown => "pagedown".to_string(),
        }
    }

    fn parse(token: &str) -> Result<Self, String> {
        let t = token.trim().to_ascii_lowercase();
        Ok(match t.as_str() {
            "enter" | "return" => ChordKey::Enter,
            "esc" | "escape" => ChordKey::Esc,
            "tab" => ChordKey::Tab,
            "space" => ChordKey::Char(' '),
            "backspace" => ChordKey::Backspace,
            "delete" | "del" => ChordKey::Delete,
            "up" => ChordKey::Up,
            "down" => ChordKey::Down,
            "left" => ChordKey::Left,
            "right" => ChordKey::Right,
            "home" => ChordKey::Home,
            "end" => ChordKey::End,
            "pageup" | "pgup" => ChordKey::PageUp,
            "pagedown" | "pgdn" => ChordKey::PageDown,
            other => {
                let mut chars = other.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => ChordKey::Char(c),
                    _ => return Err(format!("unknown key '{token}'")),
                }
            }
        })
    }
}

/// A parsed key chord: modifier flags plus the key. The neutral form a `[keys]`
/// string parses to and that a workbench `Key` maps onto for binding lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: ChordKey,
}

impl Chord {
    /// A `ctrl+<char>` chord, the common workbench gesture shape.
    #[must_use]
    pub fn ctrl(c: char) -> Self {
        Self {
            ctrl: true,
            alt: false,
            shift: false,
            key: ChordKey::Char(c),
        }
    }

    /// An `alt+<key>` chord.
    #[must_use]
    pub fn alt(key: ChordKey) -> Self {
        Self {
            ctrl: false,
            alt: true,
            shift: false,
            key,
        }
    }

    /// A `ctrl+<key>` chord for a non-character key (e.g. `Ctrl+PageDown`).
    #[must_use]
    pub fn ctrl_key(key: ChordKey) -> Self {
        Self {
            ctrl: true,
            alt: false,
            shift: false,
            key,
        }
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("ctrl".to_string());
        }
        if self.alt {
            parts.push("alt".to_string());
        }
        if self.shift {
            parts.push("shift".to_string());
        }
        parts.push(self.key.as_string());
        f.write_str(&parts.join("+"))
    }
}

impl FromStr for Chord {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut key = None;
        for token in s.split('+').map(str::trim).filter(|t| !t.is_empty()) {
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "c" => ctrl = true,
                "alt" | "meta" | "option" => alt = true,
                "shift" => shift = true,
                _ => {
                    if key.is_some() {
                        return Err(format!("chord '{s}' has more than one key"));
                    }
                    key = Some(ChordKey::parse(token)?);
                }
            }
        }
        let key = key.ok_or_else(|| format!("chord '{s}' names no key"))?;
        Ok(Chord {
            ctrl,
            alt,
            shift,
            key,
        })
    }
}

/// A rebindable workbench gesture. The drawer/schema toggles land in issue 14; the
/// Buffer (tab) gestures are defined here so issue 14's keybinding config is their
/// home (issue 18 acts on them). Editing gestures (Enter/newline/Tab-complete) are
/// structural and not rebindable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Gesture {
    RefreshSchema,
    ToggleSchema,
    ToggleParams,
    ToggleSummary,
    FormatBuffer,
    Search,
    NewBuffer,
    NextBuffer,
    PrevBuffer,
}

/// Every gesture, in a stable order — for defaults and for listing. Closing a
/// Buffer is a Workbench command (`:close`), not a gesture, so it is absent here.
pub const GESTURES: &[Gesture] = &[
    Gesture::RefreshSchema,
    Gesture::ToggleSchema,
    Gesture::ToggleParams,
    Gesture::ToggleSummary,
    Gesture::FormatBuffer,
    Gesture::Search,
    Gesture::NewBuffer,
    Gesture::NextBuffer,
    Gesture::PrevBuffer,
];

impl Gesture {
    /// The kebab-case `[keys]` name for this gesture.
    pub fn name(self) -> &'static str {
        match self {
            Gesture::RefreshSchema => "refresh-schema",
            Gesture::ToggleSchema => "toggle-schema",
            Gesture::ToggleParams => "toggle-params",
            Gesture::ToggleSummary => "toggle-summary",
            Gesture::FormatBuffer => "format-buffer",
            Gesture::Search => "search",
            Gesture::NewBuffer => "new-buffer",
            Gesture::NextBuffer => "next-buffer",
            Gesture::PrevBuffer => "prev-buffer",
        }
    }

    /// The gesture for a `[keys]` name, or `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<Self> {
        GESTURES.iter().copied().find(|g| g.name() == name)
    }

    /// The default chord, reproducing today's binding.
    pub fn default_chord(self) -> Chord {
        match self {
            Gesture::RefreshSchema => Chord::ctrl('r'),
            Gesture::ToggleSchema => Chord::ctrl('b'),
            Gesture::ToggleParams => Chord::ctrl('p'),
            // Ctrl+Y is the editor's redo (ADR 0016 — the editor owns the chords it
            // uses for editing), so the summary drawer sits on Ctrl+N (notifications).
            Gesture::ToggleSummary => Chord::ctrl('n'),
            // Ctrl+L is free in a TUI (no scrollback to clear); Ctrl+W stays the
            // editor's delete-word and is no longer a Workbench gesture.
            Gesture::FormatBuffer => Chord::ctrl('l'),
            Gesture::Search => Chord::ctrl('f'),
            Gesture::NewBuffer => Chord::ctrl('t'),
            // Buffer navigation lives on Ctrl+PageDown/PageUp, off the editor's
            // single-Ctrl/Alt+letter keyspace.
            Gesture::NextBuffer => Chord::ctrl_key(ChordKey::PageDown),
            Gesture::PrevBuffer => Chord::ctrl_key(ChordKey::PageUp),
        }
    }
}

/// The resolved gesture→chord bindings. Defaults reproduce today's chords; a
/// `[keys]` table rebinds individual gestures by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    map: BTreeMap<Gesture, Chord>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            map: GESTURES.iter().map(|&g| (g, g.default_chord())).collect(),
        }
    }
}

impl KeyBindings {
    /// The chord bound to a gesture.
    pub fn chord(&self, gesture: Gesture) -> Chord {
        self.map
            .get(&gesture)
            .copied()
            .unwrap_or_else(|| gesture.default_chord())
    }

    /// The gesture a chord triggers, or `None` if the chord is unbound — the
    /// reverse lookup the reducer does for each key press.
    pub fn gesture_for(&self, chord: Chord) -> Option<Gesture> {
        self.map
            .iter()
            .find_map(|(&g, &c)| (c == chord).then_some(g))
    }

    /// Whether any *other* gesture already holds this chord (a conflict).
    fn conflicts(&self, gesture: Gesture, chord: Chord) -> bool {
        self.map
            .iter()
            .any(|(&g, &c)| g != gesture && c == chord)
    }
}

/// Resolve `[keys]` over the defaults (issue 14). Each override is applied in name
/// order; an unknown gesture, an unparseable chord, or a chord already held by
/// another gesture is a warning and is skipped (that gesture keeps its default).
/// Returns the resolved bindings and any warnings.
pub fn resolve_keys(overrides: &BTreeMap<String, String>) -> (KeyBindings, Vec<String>) {
    let mut bindings = KeyBindings::default();
    let mut warnings = Vec::new();
    for (name, value) in overrides {
        let Some(gesture) = Gesture::from_name(name) else {
            warnings.push(format!("[keys]: unknown gesture '{name}'"));
            continue;
        };
        let chord = match value.parse::<Chord>() {
            Ok(chord) => chord,
            Err(message) => {
                warnings.push(format!("[keys] {name}: {message}"));
                continue;
            }
        };
        if bindings.conflicts(gesture, chord) {
            warnings.push(format!(
                "[keys] {name}: chord '{chord}' already bound; keeping default"
            ));
            continue;
        }
        bindings.map.insert(gesture, chord);
    }
    (bindings, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_color_round_trips_through_its_name() {
        for color in [
            ThemeColor::Default,
            ThemeColor::Yellow,
            ThemeColor::DarkGray,
            ThemeColor::LightBlue,
        ] {
            assert_eq!(color.to_string().parse(), Ok(color));
        }
        // Common spelling variants are accepted.
        assert_eq!("grey".parse(), Ok(ThemeColor::Gray));
        assert_eq!("dark-gray".parse(), Ok(ThemeColor::DarkGray));
        assert_eq!("reset".parse(), Ok(ThemeColor::Default));
        assert!("chartreuse".parse::<ThemeColor>().is_err());
    }

    #[test]
    fn the_default_theme_reproduces_todays_palette() {
        let palette = builtin_palette("default").expect("built in");
        assert_eq!(palette.color(HighlightCategory::Keyword), ThemeColor::Yellow);
        assert_eq!(palette.color(HighlightCategory::Function), ThemeColor::Cyan);
        assert_eq!(palette.color(HighlightCategory::String), ThemeColor::Green);
        assert_eq!(palette.color(HighlightCategory::Number), ThemeColor::Magenta);
        assert_eq!(palette.color(HighlightCategory::Comment), ThemeColor::DarkGray);
        assert_eq!(palette.color(HighlightCategory::Parameter), ThemeColor::Blue);
        // Plain is always the terminal default.
        assert_eq!(palette.color(HighlightCategory::Plain), ThemeColor::Default);
    }

    #[test]
    fn an_empty_theme_table_reproduces_the_default() {
        let (palette, warnings) = resolve_palette("default", &BTreeMap::new());
        assert_eq!(palette, builtin_palette("default").unwrap());
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_theme_table_overrides_individual_colours() {
        let overrides = BTreeMap::from([
            ("keyword".to_string(), "red".to_string()),
            ("parameter".to_string(), "lightmagenta".to_string()),
        ]);
        let (palette, warnings) = resolve_palette("default", &overrides);
        assert!(warnings.is_empty());
        assert_eq!(palette.color(HighlightCategory::Keyword), ThemeColor::Red);
        assert_eq!(
            palette.color(HighlightCategory::Parameter),
            ThemeColor::LightMagenta
        );
        // Untouched categories keep the base.
        assert_eq!(palette.color(HighlightCategory::String), ThemeColor::Green);
    }

    #[test]
    fn overrides_apply_on_top_of_the_named_base_theme() {
        // `mono` base (all default), with one explicit colour put back.
        let overrides = BTreeMap::from([("keyword".to_string(), "yellow".to_string())]);
        let (palette, _) = resolve_palette("mono", &overrides);
        assert_eq!(palette.color(HighlightCategory::Keyword), ThemeColor::Yellow);
        assert_eq!(palette.color(HighlightCategory::Function), ThemeColor::Default);
    }

    #[test]
    fn an_unknown_theme_falls_back_to_default_with_a_warning() {
        let (palette, warnings) = resolve_palette("neon", &BTreeMap::new());
        assert_eq!(palette, builtin_palette("default").unwrap());
        assert!(warnings.iter().any(|w| w.contains("neon")), "{warnings:?}");
    }

    #[test]
    fn a_bad_theme_key_or_colour_is_a_warning_and_is_skipped() {
        let overrides = BTreeMap::from([
            ("bogus".to_string(), "red".to_string()),
            ("keyword".to_string(), "chartreuse".to_string()),
        ]);
        let (palette, warnings) = resolve_palette("default", &overrides);
        // Both are skipped: keyword keeps the base colour.
        assert_eq!(palette.color(HighlightCategory::Keyword), ThemeColor::Yellow);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("bogus")));
        assert!(warnings.iter().any(|w| w.contains("chartreuse")));
    }

    #[test]
    fn chord_round_trips_through_its_string() {
        for s in ["ctrl+b", "alt+right", "ctrl+pageup", "ctrl+shift+up", "ctrl+space"] {
            let chord: Chord = s.parse().expect("parses");
            assert_eq!(chord.to_string(), s);
        }
    }

    #[test]
    fn chord_parsing_is_lenient_about_aliases_and_case() {
        assert_eq!("Ctrl+B".parse(), Ok(Chord::ctrl('b')));
        assert_eq!("control + b".parse(), Ok(Chord::ctrl('b')));
        assert_eq!("alt+Right".parse(), Ok(Chord::alt(ChordKey::Right)));
        assert!("ctrl+".parse::<Chord>().is_err(), "no key");
        assert!("ctrl+a+b".parse::<Chord>().is_err(), "two keys");
        assert!("ctrl+nope".parse::<Chord>().is_err(), "unknown named key");
    }

    #[test]
    fn the_ui_slots_resolve_and_default_reproduces_todays_chrome() {
        // An absent [theme] table reproduces today's chrome: Cyan focused borders,
        // reverse-video selection (the Default sentinel), an unstyled status bar.
        let (palette, warnings) = resolve_palette("default", &BTreeMap::new());
        assert!(warnings.is_empty());
        assert_eq!(palette.border, ThemeColor::Cyan);
        assert_eq!(palette.selection, ThemeColor::Default);
        assert_eq!(palette.status, ThemeColor::Default);
        // The new slots are overridable through [theme].
        let overrides = BTreeMap::from([
            ("border".to_string(), "magenta".to_string()),
            ("selection".to_string(), "blue".to_string()),
            ("status".to_string(), "green".to_string()),
        ]);
        let (palette, warnings) = resolve_palette("default", &overrides);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(palette.border, ThemeColor::Magenta);
        assert_eq!(palette.selection, ThemeColor::Blue);
        assert_eq!(palette.status, ThemeColor::Green);
    }

    #[test]
    fn the_light_builtin_exists_and_differs_from_default() {
        assert!(BUILTIN_THEMES.contains(&"light"), "light is listed among the built-ins");
        let light = builtin_palette("light").expect("light is built in");
        let default = builtin_palette("default").expect("default is built in");
        assert_ne!(light, default, "light is a distinct palette");
    }

    #[test]
    fn the_default_bindings_reproduce_todays_chords() {
        let keys = KeyBindings::default();
        assert_eq!(keys.chord(Gesture::ToggleSchema), Chord::ctrl('b'));
        assert_eq!(keys.chord(Gesture::ToggleParams), Chord::ctrl('p'));
        assert_eq!(keys.chord(Gesture::RefreshSchema), Chord::ctrl('r'));
        // The reverse lookup matches.
        assert_eq!(keys.gesture_for(Chord::ctrl('b')), Some(Gesture::ToggleSchema));
        assert_eq!(keys.gesture_for(Chord::ctrl('x')), None);
    }

    #[test]
    fn the_buffer_and_format_chords_are_remapped_off_the_editor_keyspace() {
        let keys = KeyBindings::default();
        // Buffer navigation moves to Ctrl+PageDown / Ctrl+PageUp; new stays Ctrl+T.
        assert_eq!(keys.chord(Gesture::NextBuffer), Chord::ctrl_key(ChordKey::PageDown));
        assert_eq!(keys.chord(Gesture::PrevBuffer), Chord::ctrl_key(ChordKey::PageUp));
        assert_eq!(keys.chord(Gesture::NewBuffer), Chord::ctrl('t'));
        // Auto-format moves to Ctrl+L (free in a TUI; Ctrl+W is no longer bound).
        assert_eq!(keys.chord(Gesture::FormatBuffer), Chord::ctrl('l'));
        assert_eq!(keys.gesture_for(Chord::ctrl('w')), None, "Ctrl+W is freed");
    }

    #[test]
    fn close_buffer_is_not_a_rebindable_gesture() {
        // Close is a Workbench command (:close), not a chord, so it has no [keys]
        // name and rebinding it is an unknown-gesture warning.
        assert!(Gesture::from_name("close-buffer").is_none());
    }

    #[test]
    fn an_empty_keys_table_yields_the_defaults() {
        let (keys, warnings) = resolve_keys(&BTreeMap::new());
        assert_eq!(keys, KeyBindings::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn keys_rebinds_a_gesture() {
        let overrides = BTreeMap::from([("toggle-schema".to_string(), "ctrl+g".to_string())]);
        let (keys, warnings) = resolve_keys(&overrides);
        assert!(warnings.is_empty());
        assert_eq!(keys.chord(Gesture::ToggleSchema), Chord::ctrl('g'));
        assert_eq!(keys.gesture_for(Chord::ctrl('g')), Some(Gesture::ToggleSchema));
        // The old chord is now unbound.
        assert_eq!(keys.gesture_for(Chord::ctrl('b')), None);
    }

    #[test]
    fn an_unknown_gesture_is_reported_and_skipped() {
        let overrides = BTreeMap::from([("teleport".to_string(), "ctrl+t".to_string())]);
        let (keys, warnings) = resolve_keys(&overrides);
        assert_eq!(keys, KeyBindings::default(), "untouched");
        assert!(warnings.iter().any(|w| w.contains("teleport")), "{warnings:?}");
    }

    #[test]
    fn an_unparseable_chord_is_reported_and_the_gesture_keeps_its_default() {
        let overrides = BTreeMap::from([("toggle-params".to_string(), "ctrl+".to_string())]);
        let (keys, warnings) = resolve_keys(&overrides);
        assert_eq!(keys.chord(Gesture::ToggleParams), Chord::ctrl('p'));
        assert!(warnings.iter().any(|w| w.contains("toggle-params")), "{warnings:?}");
    }

    #[test]
    fn a_conflicting_binding_is_reported_and_falls_back_to_default() {
        // Bind toggle-schema to ctrl+p, which toggle-params already holds.
        let overrides = BTreeMap::from([("toggle-schema".to_string(), "ctrl+p".to_string())]);
        let (keys, warnings) = resolve_keys(&overrides);
        assert!(warnings.iter().any(|w| w.contains("already bound")), "{warnings:?}");
        // Both keep their defaults.
        assert_eq!(keys.chord(Gesture::ToggleSchema), Chord::ctrl('b'));
        assert_eq!(keys.chord(Gesture::ToggleParams), Chord::ctrl('p'));
    }

    #[test]
    fn swapping_two_gestures_chords_is_not_a_conflict() {
        // toggle-params → its old default is freed before toggle-schema claims it.
        let overrides = BTreeMap::from([
            ("toggle-params".to_string(), "ctrl+x".to_string()),
            ("toggle-schema".to_string(), "ctrl+p".to_string()),
        ]);
        let (keys, warnings) = resolve_keys(&overrides);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(keys.chord(Gesture::ToggleParams), Chord::ctrl('x'));
        assert_eq!(keys.chord(Gesture::ToggleSchema), Chord::ctrl('p'));
    }
}
