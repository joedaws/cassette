use std::collections::HashMap;

use ratatui::style::Color;
use serde::Deserialize;

/// Colors for one theme. `text` and `background` of `None` mean the
/// terminal's own defaults — the stock look renders no color at all.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Focused cassette text.
    pub text: Option<Color>,
    /// Focused cassette background.
    pub background: Option<Color>,
    /// Minimized (unfocused) cassette colors.
    pub unfocused_bg: Color,
    pub unfocused_fg: Color,
    /// Side A cues: separator tag and line-number gutter.
    pub accent_a: Color,
    /// Side B cues.
    pub accent_b: Color,
    /// Key combos in the help line (`None` = terminal default fg).
    pub help_key: Option<Color>,
    /// Descriptions in the help line, dimmer than the keys.
    pub help_text: Option<Color>,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            text: None,
            background: None,
            unfocused_bg: Color::Rgb(0, 0, 170),
            unfocused_fg: Color::Rgb(255, 255, 255),
            accent_a: Color::Yellow,
            accent_b: Color::DarkGray,
            help_key: None,
            help_text: Some(Color::DarkGray),
        }
    }
}

/// A theme as written in config.toml under `[themes.<name>]`: any subset of
/// fields, colors as `"#rrggbb"` or ANSI names. Unset fields fall back to the
/// built-in theme of the same name, or to the default look.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ThemeSpec {
    pub text: Option<String>,
    pub background: Option<String>,
    pub unfocused_bg: Option<String>,
    pub unfocused_fg: Option<String>,
    pub accent_a: Option<String>,
    pub accent_b: Option<String>,
    pub help_key: Option<String>,
    pub help_text: Option<String>,
}

/// `"#rrggbb"` hex or an ANSI color name (`yellow`, `darkgray`, …).
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    match s.to_ascii_lowercase().as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Built-in themes, name-sorted. "default" is the stock terminal look.
pub fn builtins() -> Vec<(&'static str, Theme)> {
    vec![
        ("default", Theme::default()),
        (
            "dracula",
            Theme {
                text: Some(rgb(0xf8f8f2)),
                background: Some(rgb(0x282a36)),
                unfocused_bg: rgb(0x44475a),
                unfocused_fg: rgb(0xf8f8f2),
                accent_a: rgb(0xf1fa8c),
                accent_b: rgb(0x6272a4),
                help_key: Some(rgb(0xf8f8f2)),
                help_text: Some(rgb(0x6272a4)),
            },
        ),
        (
            "gruvbox",
            Theme {
                text: Some(rgb(0xebdbb2)),
                background: Some(rgb(0x282828)),
                unfocused_bg: rgb(0x3c3836),
                unfocused_fg: rgb(0xa89984),
                accent_a: rgb(0xfabd2f),
                accent_b: rgb(0x83a598),
                help_key: Some(rgb(0xebdbb2)),
                help_text: Some(rgb(0x928374)),
            },
        ),
        (
            "nord",
            Theme {
                text: Some(rgb(0xd8dee9)),
                background: Some(rgb(0x2e3440)),
                unfocused_bg: rgb(0x3b4252),
                unfocused_fg: rgb(0x81a1c1),
                accent_a: rgb(0xebcb8b),
                accent_b: rgb(0x88c0d0),
                help_key: Some(rgb(0xd8dee9)),
                help_text: Some(rgb(0x616e88)),
            },
        ),
        (
            "solarized-dark",
            Theme {
                text: Some(rgb(0x839496)),
                background: Some(rgb(0x002b36)),
                unfocused_bg: rgb(0x073642),
                unfocused_fg: rgb(0x93a1a1),
                accent_a: rgb(0xb58900),
                accent_b: rgb(0x268bd2),
                help_key: Some(rgb(0x839496)),
                help_text: Some(rgb(0x586e75)),
            },
        ),
        (
            "solarized-light",
            Theme {
                text: Some(rgb(0x657b83)),
                background: Some(rgb(0xfdf6e3)),
                unfocused_bg: rgb(0xeee8d5),
                unfocused_fg: rgb(0x586e75),
                accent_a: rgb(0xb58900),
                accent_b: rgb(0x268bd2),
                help_key: Some(rgb(0x657b83)),
                help_text: Some(rgb(0x93a1a1)),
            },
        ),
    ]
}

fn apply_spec(mut base: Theme, spec: &ThemeSpec, name: &str) -> Result<Theme, String> {
    let parse = |field: &str, val: &Option<String>| -> Result<Option<Color>, String> {
        match val {
            None => Ok(None),
            Some(s) => parse_color(s).map(Some).ok_or_else(|| {
                format!("theme '{name}': invalid color '{s}' for '{field}' (use '#rrggbb' or an ANSI name)")
            }),
        }
    };
    if let Some(c) = parse("text", &spec.text)? {
        base.text = Some(c);
    }
    if let Some(c) = parse("background", &spec.background)? {
        base.background = Some(c);
    }
    if let Some(c) = parse("unfocused_bg", &spec.unfocused_bg)? {
        base.unfocused_bg = c;
    }
    if let Some(c) = parse("unfocused_fg", &spec.unfocused_fg)? {
        base.unfocused_fg = c;
    }
    if let Some(c) = parse("accent_a", &spec.accent_a)? {
        base.accent_a = c;
    }
    if let Some(c) = parse("accent_b", &spec.accent_b)? {
        base.accent_b = c;
    }
    if let Some(c) = parse("help_key", &spec.help_key)? {
        base.help_key = Some(c);
    }
    if let Some(c) = parse("help_text", &spec.help_text)? {
        base.help_text = Some(c);
    }
    Ok(base)
}

/// Look up `name` among user themes first (overriding the built-in of the
/// same name field-by-field), then built-ins. `None` is the default theme.
pub fn resolve(name: Option<&str>, user: &HashMap<String, ThemeSpec>) -> Result<Theme, String> {
    let Some(name) = name else {
        return Ok(Theme::default());
    };
    let base = builtins()
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| t);
    if let Some(spec) = user.get(name) {
        return apply_spec(base.unwrap_or_default(), spec, name);
    }
    base.ok_or_else(|| format!("unknown theme '{name}' — run 'cassette +themes' to list themes"))
}

/// All selectable themes for `+themes`: built-ins then extra user themes,
/// each resolved and flagged `true` when (partly) user-defined.
pub fn all(user: &HashMap<String, ThemeSpec>) -> Vec<(String, Theme, bool)> {
    let mut out: Vec<(String, Theme, bool)> = Vec::new();
    for (name, theme) in builtins() {
        match resolve(Some(name), user) {
            Ok(t) => out.push((name.to_string(), t, user.contains_key(name))),
            Err(_) => out.push((name.to_string(), theme, false)),
        }
    }
    let mut extra: Vec<&String> = user
        .keys()
        .filter(|k| !out.iter().any(|(n, _, _)| n == *k))
        .collect();
    extra.sort();
    for name in extra {
        if let Ok(t) = resolve(Some(name), user) {
            out.push((name.clone(), t, true));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(text: Option<&str>, accent_a: Option<&str>) -> ThemeSpec {
        ThemeSpec {
            text: text.map(String::from),
            accent_a: accent_a.map(String::from),
            ..ThemeSpec::default()
        }
    }

    #[test]
    fn parses_hex_and_names() {
        assert_eq!(parse_color("#ff8000"), Some(Color::Rgb(255, 128, 0)));
        assert_eq!(parse_color("Yellow"), Some(Color::Yellow));
        assert_eq!(parse_color("darkgrey"), Some(Color::DarkGray));
        assert_eq!(parse_color("#ff80"), None, "short hex rejected");
        assert_eq!(parse_color("#gggggg"), None);
        assert_eq!(parse_color("mauve-ish"), None);
    }

    #[test]
    fn default_theme_swaps_side_accents() {
        // Issue #34: side A carries the yellow accent, side B the gray.
        let t = Theme::default();
        assert_eq!(t.accent_a, Color::Yellow);
        assert_eq!(t.accent_b, Color::DarkGray);
        assert_eq!(t.text, None, "stock look keeps terminal colors");
    }

    #[test]
    fn resolves_builtin_and_rejects_unknown() {
        let none = HashMap::new();
        let g = resolve(Some("gruvbox"), &none).unwrap();
        assert_eq!(g.background, Some(Color::Rgb(0x28, 0x28, 0x28)));
        assert!(resolve(Some("no-such"), &none)
            .unwrap_err()
            .contains("+themes"));
        assert_eq!(resolve(None, &none).unwrap(), Theme::default());
    }

    #[test]
    fn user_theme_from_scratch_fills_from_default() {
        let mut user = HashMap::new();
        user.insert("mine".to_string(), spec(Some("#101010"), Some("red")));
        let t = resolve(Some("mine"), &user).unwrap();
        assert_eq!(t.text, Some(Color::Rgb(0x10, 0x10, 0x10)));
        assert_eq!(t.accent_a, Color::Red);
        assert_eq!(
            t.accent_b,
            Theme::default().accent_b,
            "unset fields keep defaults"
        );
    }

    #[test]
    fn user_spec_overrides_builtin_field_by_field() {
        let mut user = HashMap::new();
        user.insert("gruvbox".to_string(), spec(Some("#ffffff"), None));
        let t = resolve(Some("gruvbox"), &user).unwrap();
        assert_eq!(t.text, Some(Color::Rgb(255, 255, 255)), "overridden");
        assert_eq!(
            t.background,
            Some(Color::Rgb(0x28, 0x28, 0x28)),
            "rest stays gruvbox"
        );
    }

    #[test]
    fn invalid_color_reports_theme_and_field() {
        let mut user = HashMap::new();
        user.insert("bad".to_string(), spec(Some("notacolor"), None));
        let err = resolve(Some("bad"), &user).unwrap_err();
        assert!(err.contains("bad") && err.contains("text") && err.contains("notacolor"));
    }

    #[test]
    fn all_lists_builtins_plus_user_extras() {
        let mut user = HashMap::new();
        user.insert("zzz-mine".to_string(), spec(Some("#000000"), None));
        user.insert("gruvbox".to_string(), spec(Some("#ffffff"), None));
        let list = all(&user);
        let names: Vec<&str> = list.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"default") && names.contains(&"zzz-mine"));
        let gruv = list.iter().find(|(n, _, _)| n == "gruvbox").unwrap();
        assert!(gruv.2, "overridden builtin is flagged as user-modified");
        assert_eq!(gruv.1.text, Some(Color::Rgb(255, 255, 255)));
        assert_eq!(
            names.iter().filter(|n| **n == "gruvbox").count(),
            1,
            "no duplicate for overridden builtin"
        );
    }
}
