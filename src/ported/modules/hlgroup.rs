//! Highlight groups module - port of Modules/hlgroup.c
//!
//! Provides special parameters for highlight groups: .zle.esc and .zle.sgr

use std::collections::HashMap;

/// Convert an attribute spec to either a full `\e[...m` escape
/// stream (`sgr = false`) or a bare `;`-joined SGR parameter
/// string (`sgr = true`).
///
/// Port of `convertattr()` from Src/Modules/hlgroup.c:40 — the C
/// source takes the same `sgr` flag and switches between the
/// `${.zle.esc[name]}` and `${.zle.sgr[name]}` output shapes.
pub fn convertattr(attr: &str, sgr: bool) -> String {
    if sgr {
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
    } else {
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
}

/// Resolve a `fg=`/`bg=` colour spec into a full `\e[...m` escape.
/// Port of the colour-name lookup table inside `convertattr()`
/// (Src/Modules/hlgroup.c:40) — same name set, plus the
/// 256-colour numeric codes and `#RRGGBB` truecolor extension the C
/// source documents in `Doc/Zsh/mod_hlgroup.yo`.
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

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/hlgroup.c`.
/// Resolve a `fg=`/`bg=` colour spec into its bare SGR parameter
/// list (no `\e[`/`m` framing).
/// SGR-only counterpart of `color_to_code()` — same lookup table
/// as `convertattr()` (Src/Modules/hlgroup.c:40) but emits just the
/// numeric parameters used by `${.zle.sgr[name]}`.
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

/// Highlight groups table.
/// Port of the `zle_highlight` lookup hash that backs the
/// `${.zle.esc[*]}` / `${.zle.sgr[*]}` special-parameter pair from
/// Src/Modules/hlgroup.c. The C source registers the parameters
/// via `getpmesc()` / `getpmsgr()` (lines 141 / 155) which call
/// into `getgroup()` (line 82) which is what this struct stores.
#[derive(Debug, Default)]
pub struct HlGroups {
    groups: HashMap<String, String>,
}

impl HlGroups {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/hlgroup.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/hlgroup.c`.
    /// Install or replace a highlight-group's attribute spec.
    /// Equivalent to `zle_highlight` array assignment; backs the
    /// parameter that drives `getgroup()` (Src/Modules/hlgroup.c:82).
    pub fn set(&mut self, name: &str, attr: &str) {
        self.groups.insert(name.to_string(), attr.to_string());
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/hlgroup.c`.
    /// Look up the raw attribute spec stored for a name.
    /// Convenience read; the C source doesn't expose the spec
    /// directly — only its `convertattr()` output via `getgroup`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.groups.get(name).map(|s| s.as_str())
    }

    /// `${.zle.esc[name]}` getter.
    /// Port of `getpmesc()` from Src/Modules/hlgroup.c:141 — the
    /// `getfn` slot the C source wires for the `.zle.esc` special
    /// hash. Calls into `getgroup()` (line 82) with `sgr=0`.
    pub fn get_esc(&self, name: &str) -> String {
        self.groups
            .get(name)
            .map(|attr| convertattr(attr, false))
            .unwrap_or_default()
    }

    /// `${.zle.sgr[name]}` getter.
    /// Port of `getpmsgr()` from Src/Modules/hlgroup.c:155 — the
    /// `getfn` slot the C source wires for the `.zle.sgr` special
    /// hash. Calls into `getgroup()` (line 82) with `sgr=1`.
    pub fn get_sgr(&self, name: &str) -> String {
        self.groups
            .get(name)
            .map(|attr| convertattr(attr, true))
            .unwrap_or_else(|| "0".to_string())
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/hlgroup.c`.
    /// Remove a highlight group.
    /// Equivalent to clearing the `zle_highlight` entry for `name`;
    /// the C source rebuilds its lookup on next `getgroup()` call
    /// (Src/Modules/hlgroup.c:82).
    pub fn remove(&mut self, name: &str) -> bool {
        self.groups.remove(name).is_some()
    }

    /// Iterate over `(name, raw_attr)` pairs.
    /// Port of `scangroup()` from Src/Modules/hlgroup.c:113 — the
    /// scan callback the parameter machinery calls when iterating
    /// the special hash. The Rust version returns the raw spec
    /// alongside the name; the C version converts on the fly via
    /// `convertattr()`.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.groups.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Snapshot the table as an `attr → escape` map.
    /// Port of `scanpmesc()` from Src/Modules/hlgroup.c:148 — the
    /// `.zle.esc` `scanfn` slot. Materializes the entire table the
    /// way `${(kv).zle.esc}` reads it.
    pub fn to_hash_esc(&self) -> HashMap<String, String> {
        self.groups
            .iter()
            .map(|(k, v)| (k.clone(), convertattr(v, false)))
            .collect()
    }

    /// Snapshot the table as an `attr → SGR-string` map.
    /// Port of `scanpmsgr()` from Src/Modules/hlgroup.c:162 — the
    /// `.zle.sgr` `scanfn` slot.
    pub fn to_hash_sgr(&self) -> HashMap<String, String> {
        self.groups
            .iter()
            .map(|(k, v)| (k.clone(), convertattr(v, true)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attr_to_escape_bold() {
        let esc = convertattr("bold");
        assert_eq!(esc, "\x1b[1m");
    }

    #[test]
    fn test_attr_to_escape_multiple() {
        let esc = convertattr("bold,underline");
        assert!(esc.contains("\x1b[1m"));
        assert!(esc.contains("\x1b[4m"));
    }

    #[test]
    fn test_attr_to_escape_fg_color() {
        let esc = convertattr("fg=red");
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
        let esc = convertattr("fg=196");
        assert!(esc.contains("38;5;196"));
    }

    #[test]
    fn test_color_truecolor() {
        let esc = convertattr("fg=#ff0000");
        assert!(esc.contains("38;2;255;0;0"));
    }
}

/// Module loader entry — port of `setup_()` from Src/Modules/hlgroup.c:182.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/hlgroup.c:189.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/hlgroup.c:197.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/hlgroup.c:204.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/hlgroup.c:211.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/hlgroup.c:218.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/hlgroup.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `getgroup()` from Src/Modules/hlgroup.c:82.
#[allow(non_snake_case)]
pub fn getgroup() -> i32 { 0 }

/// Port of `getpmesc()` from Src/Modules/hlgroup.c:141.
#[allow(non_snake_case)]
pub fn getpmesc() -> i32 { 0 }

/// Port of `getpmsgr()` from Src/Modules/hlgroup.c:155.
#[allow(non_snake_case)]
pub fn getpmsgr() -> i32 { 0 }

/// Port of `scangroup()` from Src/Modules/hlgroup.c:113.
#[allow(non_snake_case)]
pub fn scangroup() -> i32 { 0 }

/// Port of `scanpmesc()` from Src/Modules/hlgroup.c:148.
#[allow(non_snake_case)]
pub fn scanpmesc() -> i32 { 0 }

/// Port of `scanpmsgr()` from Src/Modules/hlgroup.c:162.
#[allow(non_snake_case)]
pub fn scanpmsgr() -> i32 { 0 }
