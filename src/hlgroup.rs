//! Highlight groups module - port of Modules/hlgroup.c
//!
//! Provides special parameters for highlight groups: .zle.esc and .zle.sgr

use std::collections::HashMap;

/// Convert attribute string to escape sequence
pub fn attr_to_escape(attr: &str) -> String {
    let mut result = String::new();

    for part in attr.split(',') {
        let part = part.trim();
        match part {
            "none" | "reset" => result.push_str("\x1b[0m"),
            "bold" => result.push_str("\x1b[1m"),
            "dim" | "faint" => result.push_str("\x1b[2m"),
            "italic" => result.push_str("\x1b[3m"),
            "underline" => result.push_str("\x1b[4m"),
            "blink" => result.push_str("\x1b[5m"),
            "reverse" | "inverse" => result.push_str("\x1b[7m"),
            "hidden" | "invisible" => result.push_str("\x1b[8m"),
            "strikethrough" => result.push_str("\x1b[9m"),
            s if s.starts_with("fg=") => {
                if let Some(color) = color_to_code(&s[3..], true) {
                    result.push_str(&color);
                }
            }
            s if s.starts_with("bg=") => {
                if let Some(color) = color_to_code(&s[3..], false) {
                    result.push_str(&color);
                }
            }
            _ => {}
        }
    }

    result
}

/// Convert attribute string to SGR parameter string (no escape sequences)
pub fn attr_to_sgr(attr: &str) -> String {
    let mut codes = Vec::new();

    for part in attr.split(',') {
        let part = part.trim();
        match part {
            "none" | "reset" => codes.push("0".to_string()),
            "bold" => codes.push("1".to_string()),
            "dim" | "faint" => codes.push("2".to_string()),
            "italic" => codes.push("3".to_string()),
            "underline" => codes.push("4".to_string()),
            "blink" => codes.push("5".to_string()),
            "reverse" | "inverse" => codes.push("7".to_string()),
            "hidden" | "invisible" => codes.push("8".to_string()),
            "strikethrough" => codes.push("9".to_string()),
            s if s.starts_with("fg=") => {
                if let Some(code) = color_to_sgr_code(&s[3..], true) {
                    codes.push(code);
                }
            }
            s if s.starts_with("bg=") => {
                if let Some(code) = color_to_sgr_code(&s[3..], false) {
                    codes.push(code);
                }
            }
            _ => {}
        }
    }

    if codes.is_empty() {
        "0".to_string()
    } else {
        codes.join(";")
    }
}

fn color_to_code(color: &str, fg: bool) -> Option<String> {
    let base = if fg { 30 } else { 40 };
    let bright_base = if fg { 90 } else { 100 };

    match color {
        "black" => Some(format!("\x1b[{}m", base)),
        "red" => Some(format!("\x1b[{}m", base + 1)),
        "green" => Some(format!("\x1b[{}m", base + 2)),
        "yellow" => Some(format!("\x1b[{}m", base + 3)),
        "blue" => Some(format!("\x1b[{}m", base + 4)),
        "magenta" => Some(format!("\x1b[{}m", base + 5)),
        "cyan" => Some(format!("\x1b[{}m", base + 6)),
        "white" => Some(format!("\x1b[{}m", base + 7)),
        "default" => Some(format!("\x1b[{}m", base + 9)),
        s if s.starts_with("bright-") || s.starts_with("light-") => {
            let inner = s.split_once('-').map(|(_, c)| c)?;
            match inner {
                "black" => Some(format!("\x1b[{}m", bright_base)),
                "red" => Some(format!("\x1b[{}m", bright_base + 1)),
                "green" => Some(format!("\x1b[{}m", bright_base + 2)),
                "yellow" => Some(format!("\x1b[{}m", bright_base + 3)),
                "blue" => Some(format!("\x1b[{}m", bright_base + 4)),
                "magenta" => Some(format!("\x1b[{}m", bright_base + 5)),
                "cyan" => Some(format!("\x1b[{}m", bright_base + 6)),
                "white" => Some(format!("\x1b[{}m", bright_base + 7)),
                _ => None,
            }
        }
        s if s.parse::<u8>().is_ok() => {
            let n: u8 = s.parse().unwrap();
            Some(format!("\x1b[{};5;{}m", if fg { 38 } else { 48 }, n))
        }
        s if s.starts_with('#') && s.len() == 7 => {
            let r = u8::from_str_radix(&s[1..3], 16).ok()?;
            let g = u8::from_str_radix(&s[3..5], 16).ok()?;
            let b = u8::from_str_radix(&s[5..7], 16).ok()?;
            Some(format!(
                "\x1b[{};2;{};{};{}m",
                if fg { 38 } else { 48 },
                r,
                g,
                b
            ))
        }
        _ => None,
    }
}

fn color_to_sgr_code(color: &str, fg: bool) -> Option<String> {
    let base = if fg { 30 } else { 40 };
    let bright_base = if fg { 90 } else { 100 };

    match color {
        "black" => Some(base.to_string()),
        "red" => Some((base + 1).to_string()),
        "green" => Some((base + 2).to_string()),
        "yellow" => Some((base + 3).to_string()),
        "blue" => Some((base + 4).to_string()),
        "magenta" => Some((base + 5).to_string()),
        "cyan" => Some((base + 6).to_string()),
        "white" => Some((base + 7).to_string()),
        "default" => Some((base + 9).to_string()),
        s if s.starts_with("bright-") || s.starts_with("light-") => {
            let inner = s.split_once('-').map(|(_, c)| c)?;
            match inner {
                "black" => Some(bright_base.to_string()),
                "red" => Some((bright_base + 1).to_string()),
                "green" => Some((bright_base + 2).to_string()),
                "yellow" => Some((bright_base + 3).to_string()),
                "blue" => Some((bright_base + 4).to_string()),
                "magenta" => Some((bright_base + 5).to_string()),
                "cyan" => Some((bright_base + 6).to_string()),
                "white" => Some((bright_base + 7).to_string()),
                _ => None,
            }
        }
        s if s.parse::<u8>().is_ok() => {
            let n: u8 = s.parse().unwrap();
            Some(format!("{};5;{}", if fg { 38 } else { 48 }, n))
        }
        s if s.starts_with('#') && s.len() == 7 => {
            let r = u8::from_str_radix(&s[1..3], 16).ok()?;
            let g = u8::from_str_radix(&s[3..5], 16).ok()?;
            let b = u8::from_str_radix(&s[5..7], 16).ok()?;
            Some(format!("{};2;{};{};{}", if fg { 38 } else { 48 }, r, g, b))
        }
        _ => None,
    }
}

/// Highlight groups table
#[derive(Debug, Default)]
pub struct HlGroups {
    groups: HashMap<String, String>,
}

impl HlGroups {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: &str, attr: &str) {
        self.groups.insert(name.to_string(), attr.to_string());
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.groups.get(name).map(|s| s.as_str())
    }

    pub fn get_esc(&self, name: &str) -> String {
        self.groups
            .get(name)
            .map(|attr| attr_to_escape(attr))
            .unwrap_or_default()
    }

    pub fn get_sgr(&self, name: &str) -> String {
        self.groups
            .get(name)
            .map(|attr| attr_to_sgr(attr))
            .unwrap_or_else(|| "0".to_string())
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.groups.remove(name).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.groups.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn to_hash_esc(&self) -> HashMap<String, String> {
        self.groups
            .iter()
            .map(|(k, v)| (k.clone(), attr_to_escape(v)))
            .collect()
    }

    pub fn to_hash_sgr(&self) -> HashMap<String, String> {
        self.groups
            .iter()
            .map(|(k, v)| (k.clone(), attr_to_sgr(v)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attr_to_escape_bold() {
        let esc = attr_to_escape("bold");
        assert_eq!(esc, "\x1b[1m");
    }

    #[test]
    fn test_attr_to_escape_multiple() {
        let esc = attr_to_escape("bold,underline");
        assert!(esc.contains("\x1b[1m"));
        assert!(esc.contains("\x1b[4m"));
    }

    #[test]
    fn test_attr_to_escape_fg_color() {
        let esc = attr_to_escape("fg=red");
        assert!(esc.contains("31"));
    }

    #[test]
    fn test_attr_to_sgr_bold() {
        let sgr = attr_to_sgr("bold");
        assert_eq!(sgr, "1");
    }

    #[test]
    fn test_attr_to_sgr_multiple() {
        let sgr = attr_to_sgr("bold,underline");
        assert!(sgr.contains("1"));
        assert!(sgr.contains("4"));
    }

    #[test]
    fn test_attr_to_sgr_empty() {
        let sgr = attr_to_sgr("");
        assert_eq!(sgr, "0");
    }

    #[test]
    fn test_hlgroups_set_get() {
        let mut groups = HlGroups::new();
        groups.set("error", "bold,fg=red");
        assert_eq!(groups.get("error"), Some("bold,fg=red"));
    }

    #[test]
    fn test_hlgroups_get_esc() {
        let mut groups = HlGroups::new();
        groups.set("error", "bold");
        assert_eq!(groups.get_esc("error"), "\x1b[1m");
    }

    #[test]
    fn test_hlgroups_get_sgr() {
        let mut groups = HlGroups::new();
        groups.set("error", "bold");
        assert_eq!(groups.get_sgr("error"), "1");
    }

    #[test]
    fn test_color_256() {
        let esc = attr_to_escape("fg=196");
        assert!(esc.contains("38;5;196"));
    }

    #[test]
    fn test_color_truecolor() {
        let esc = attr_to_escape("fg=#ff0000");
        assert!(esc.contains("38;2;255;0;0"));
    }
}
