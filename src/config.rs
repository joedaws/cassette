use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::theme::ThemeSpec;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    /// Text rows shown per cassette; overridden by the `-l` CLI flag.
    pub visible_lines: Option<usize>,
    /// Name of the theme to use; overridden by the `--theme` CLI flag.
    pub theme: Option<String>,
    /// chrono format string for the `today` note's filename (default `%Y-%m-%d`).
    pub daily_format: Option<String>,
    /// User-defined themes: `[themes.<name>]` tables with color fields
    /// (`text`, `background`, `unfocused_bg`, `unfocused_fg`, `accent_a`,
    /// `accent_b`). A name matching a built-in overrides it field-by-field.
    #[serde(default)]
    pub themes: HashMap<String, ThemeSpec>,
    /// Named topic templates selectable with `-T`: each entry is a list of
    /// topics, and the session starts with one cassette per topic.
    ///
    /// ```toml
    /// [templates]
    /// morning = ["gratitude", "priorities", "loose thoughts"]
    /// ```
    #[serde(default)]
    pub templates: HashMap<String, Vec<String>>,
    /// Most open cassettes one session may hold; defaults to
    /// `store::MAX_OPEN` (36).
    pub max_open: Option<usize>,
}

/// XDG-style config path on every platform: `$XDG_CONFIG_HOME/cassette/config.toml`,
/// falling back to `~/.config/cassette/config.toml`. Deliberately not
/// `dirs::config_dir()`: on macOS that is ~/Library/Application Support, and
/// terminal tools conventionally live in ~/.config there too.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))?;
    Some(base.join("cassette").join("config.toml"))
}

/// A missing config file is the default config; a file that exists but does
/// not parse is an error the user must see, not a silent fallback.
pub fn load_config() -> Result<Config, String> {
    let Some(config_path) = config_path() else {
        return Ok(Config::default());
    };
    let Ok(contents) = std::fs::read_to_string(&config_path) else {
        return Ok(Config::default());
    };
    toml::from_str(&contents)
        .map_err(|e| format!("invalid config '{}':\n{}", config_path.display(), e))
}

#[cfg(test)]
mod tests {
    /// An unknown key is IGNORED, not an error — serde's derive skips
    /// fields it does not know and `Config` sets no `deny_unknown_fields`.
    /// That is what lets a config written for an older version keep working,
    /// and it is why `notes_dir` could be deleted outright rather than kept
    /// as a parsing stub: the earlier belief that removing the field would
    /// break such configs was simply wrong.
    #[test]
    fn an_unknown_config_key_is_ignored_rather_than_an_error() {
        let toml = "notes_dir = \"/home/someone/notes\"\nvisible_lines = 8\n";
        let cfg: Config = toml::from_str(toml).expect("an old config must still load");
        assert_eq!(
            cfg.visible_lines,
            Some(8),
            "and the keys it does know still apply"
        );
    }

    use super::*;
    use std::path::PathBuf;

    #[test]
    fn config_path_honors_xdg_config_home_and_falls_back_to_dot_config() {
        // Single test for both env states: parallel tests must not race on env.
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg-test");
        assert_eq!(
            config_path(),
            Some(PathBuf::from("/tmp/xdg-test/cassette/config.toml"))
        );
        std::env::remove_var("XDG_CONFIG_HOME");
        let home = dirs::home_dir().expect("test env has a home dir");
        assert_eq!(
            config_path(),
            Some(home.join(".config").join("cassette").join("config.toml"))
        );
    }
}
