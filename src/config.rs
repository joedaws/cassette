use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::theme::ThemeSpec;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub notes_dir: Option<PathBuf>,
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

pub fn default_notes_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("cassette").join("notes"))
}

/// Resolve the final output path.
/// - `note_name`: positional arg from CLI, or `None` for auto-generated timestamp name.
/// - `started_at`: session start time, used only when `note_name` is `None`.
/// - `notes_dir`: base directory (from config or XDG default).
///
/// If `note_name` contains a `/` or is absolute it is treated as a direct path
/// (relative paths resolve against cwd). Otherwise the name is joined under
/// `notes_dir`. `.md` is appended when no extension is present.
pub fn resolve_output_path(
    note_name: Option<&str>,
    started_at: &std::time::SystemTime,
    notes_dir: Option<&Path>,
) -> PathBuf {
    let name = match note_name {
        Some(n) => n.to_string(),
        None => {
            let dt: chrono::DateTime<chrono::Local> = (*started_at).into();
            dt.format("%Y-%m-%dT%H-%M-%S").to_string()
        }
    };

    let p = Path::new(&name);
    let is_qualified = p.is_absolute() || name.contains('/');

    let mut base: PathBuf = if is_qualified {
        p.to_path_buf()
    } else {
        match notes_dir {
            Some(dir) => dir.join(&name),
            None => PathBuf::from(&name),
        }
    };

    if base.extension().is_none() {
        base.set_extension("md");
    }
    base
}

/// Returns `(final_path, conflicted)`.
/// When `path` already exists, increments the stem until a free name is found:
/// `myjournal.md` → `myjournal_1.md` → `myjournal_2.md` …
pub fn find_available_path(path: &Path) -> (PathBuf, bool) {
    if !path.exists() {
        return (path.to_path_buf(), false);
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut n = 1u32;
    loop {
        let candidate = parent.join(format!("{}_{}{}", stem, n, ext));
        if !candidate.exists() {
            return (candidate, true);
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn dummy_time() -> SystemTime {
        SystemTime::UNIX_EPOCH
    }

    #[test]
    fn bare_name_no_notes_dir() {
        let path = resolve_output_path(Some("myjournal"), &dummy_time(), None);
        assert_eq!(path, PathBuf::from("myjournal.md"));
    }

    #[test]
    fn bare_name_with_notes_dir() {
        let dir = PathBuf::from("/home/user/notes");
        let path = resolve_output_path(Some("myjournal"), &dummy_time(), Some(&dir));
        assert_eq!(path, PathBuf::from("/home/user/notes/myjournal.md"));
    }

    #[test]
    fn name_with_slash_ignores_notes_dir() {
        let dir = PathBuf::from("/home/user/notes");
        let path = resolve_output_path(Some("../other/foo"), &dummy_time(), Some(&dir));
        assert_eq!(path, PathBuf::from("../other/foo.md"));
    }

    #[test]
    fn absolute_path_ignores_notes_dir() {
        let dir = PathBuf::from("/home/user/notes");
        let path = resolve_output_path(Some("/tmp/out"), &dummy_time(), Some(&dir));
        assert_eq!(path, PathBuf::from("/tmp/out.md"));
    }

    #[test]
    fn extension_not_doubled() {
        let path = resolve_output_path(Some("myjournal.md"), &dummy_time(), None);
        assert_eq!(path, PathBuf::from("myjournal.md"));
    }

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

    #[test]
    fn none_name_generates_timestamp() {
        let path = resolve_output_path(None, &dummy_time(), None);
        let name = path.file_name().unwrap().to_string_lossy();
        // YYYY-MM-DDTHH-MM-SS.md  (colons replaced with dashes)
        assert!(name.ends_with(".md"));
        assert!(name.len() > 4, "should be a timestamp filename");
    }
}
