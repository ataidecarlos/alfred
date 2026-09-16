use std::path::PathBuf;

use ratatui::style::Color;

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub background: Color,
    pub message_bg: Color,
    pub user_accent: Color,
    pub agent_accent: Color,
    pub text: Color,
    pub text_dim: Color,
    pub menu_bg: Color,
    pub menu_selected: Color,
    pub menu_text: Color,
}

#[derive(Debug, serde::Deserialize)]
struct ThemeFile {
    colors: ThemeColors,
}

#[derive(Debug, serde::Deserialize)]
struct ThemeColors {
    background: String,
    message_bg: String,
    user_accent: String,
    agent_accent: String,
    text: String,
    text_dim: String,
    menu_bg: String,
    menu_selected: String,
    menu_text: String,
}

const DEFAULT_DARK_TOML: &str = r##"[colors]
background = "#000000"
message_bg = "#1a1a1a"
user_accent = "#4682e6"
agent_accent = "#e63946"
text = "#ffffff"
text_dim = "#888888"
menu_bg = "#1a1a1a"
menu_selected = "#333333"
menu_text = "#ffffff"
"##;

const DEFAULT_LIGHT_TOML: &str = r##"[colors]
background = "#ffffff"
message_bg = "#f0f0f0"
user_accent = "#2b6cb0"
agent_accent = "#c53030"
text = "#000000"
text_dim = "#6b7280"
menu_bg = "#f0f0f0"
menu_selected = "#d0d0d0"
menu_text = "#000000"
"##;

/// Write bundled themes into the user themes dir if missing.
pub fn ensure_default_themes() {
    let dir = theme_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let dark = dir.join("dark.toml");
    if !dark.exists() {
        let _ = std::fs::write(&dark, DEFAULT_DARK_TOML);
    }
    let light = dir.join("light.toml");
    if !light.exists() {
        let _ = std::fs::write(&light, DEFAULT_LIGHT_TOML);
    }
}

pub fn theme_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("alfred")
            .join("themes")
    } else if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("alfred/themes")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/alfred/themes")
    } else {
        PathBuf::from("themes")
    }
}

fn parse_hex(hex: &str) -> Result<Color, String> {
    let hex = hex.trim().trim_start_matches('#');
    let expanded = if hex.len() == 3 {
        hex.chars().flat_map(|c| [c, c]).collect::<String>()
    } else {
        hex.to_string()
    };
    if expanded.len() != 6 {
        return Err(format!("invalid hex color: {}", hex));
    }
    let r = u8::from_str_radix(&expanded[0..2], 16).map_err(|e| e.to_string())?;
    let g = u8::from_str_radix(&expanded[2..4], 16).map_err(|e| e.to_string())?;
    let b = u8::from_str_radix(&expanded[4..6], 16).map_err(|e| e.to_string())?;
    Ok(Color::Rgb(r, g, b))
}

impl Theme {
    pub fn load(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // 1. Installed/user themes dir
        let candidates = [
            theme_dir().join(format!("{}.toml", name)),
            // 2. Repo-relative fallback (dev)
            PathBuf::from("themes").join(format!("{}.toml", name)),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("themes").join(format!("{}.toml", name)),
        ];
        let mut last_err: Option<Box<dyn std::error::Error>> = None;
        for path in &candidates {
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    let file: ThemeFile = toml::from_str(&content)?;
                    return Ok(Self {
                        name: name.into(),
                        background: parse_hex(&file.colors.background)?,
                        message_bg: parse_hex(&file.colors.message_bg)?,
                        user_accent: parse_hex(&file.colors.user_accent)?,
                        agent_accent: parse_hex(&file.colors.agent_accent)?,
                        text: parse_hex(&file.colors.text)?,
                        text_dim: parse_hex(&file.colors.text_dim)?,
                        menu_bg: parse_hex(&file.colors.menu_bg)?,
                        menu_selected: parse_hex(&file.colors.menu_selected)?,
                        menu_text: parse_hex(&file.colors.menu_text)?,
                    });
                }
                Err(e) => last_err = Some(e.into()),
            }
        }
        Err(last_err.unwrap_or_else(|| format!("theme '{}' not found", name).into()))
    }

    pub fn default_dark() -> Self {
        Self {
            name: "dark".into(),
            background: Color::Black,
            message_bg: Color::Rgb(0x1a, 0x1a, 0x1a),
            user_accent: Color::Rgb(0x46, 0x82, 0xe6),
            agent_accent: Color::Rgb(0xe6, 0x39, 0x46),
            text: Color::White,
            text_dim: Color::DarkGray,
            menu_bg: Color::Rgb(0x1a, 0x1a, 0x1a),
            menu_selected: Color::Rgb(0x33, 0x33, 0x33),
            menu_text: Color::White,
        }
    }

    pub fn default_light() -> Self {
        Self {
            name: "light".into(),
            background: Color::White,
            message_bg: Color::Rgb(0xf0, 0xf0, 0xf0),
            user_accent: Color::Rgb(0x2b, 0x6c, 0xb0),
            agent_accent: Color::Rgb(0xc5, 0x30, 0x30),
            text: Color::Black,
            text_dim: Color::DarkGray,
            menu_bg: Color::Rgb(0xf0, 0xf0, 0xf0),
            menu_selected: Color::Rgb(0xd0, 0xd0, 0xd0),
            menu_text: Color::Black,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_six_digit() {
        assert_eq!(parse_hex("#4682e6").unwrap(), Color::Rgb(0x46, 0x82, 0xe6));
        assert_eq!(parse_hex("ffffff").unwrap(), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn parse_hex_three_digit() {
        assert_eq!(parse_hex("#fff").unwrap(), Color::Rgb(255, 255, 255));
        assert_eq!(parse_hex("#000").unwrap(), Color::Rgb(0, 0, 0));
    }

    #[test]
    fn parse_hex_invalid() {
        assert!(parse_hex("#zzzzz").is_err());
        assert!(parse_hex("#12345").is_err());
        assert!(parse_hex("").is_err());
    }

    #[test]
    fn load_missing_theme_errors() {
        assert!(Theme::load("no-such-theme-xyz").is_err());
    }
}
