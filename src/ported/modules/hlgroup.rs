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
                    if let Some(code) = match_colour(&s[3..], true, true) {
                        codes.push(code);
                    }
                }
                s if s.starts_with("bg=") => {
                    if let Some(code) = match_colour(&s[3..], false, true) {
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
                    if let Some(color) = match_colour(&s[3..], true, false) {
                        result.push_str(&color);
                    }
                }
                s if s.starts_with("bg=") => {
                    if let Some(color) = match_colour(&s[3..], false, false) {
                        result.push_str(&color);
                    }
                }
                _ => {}
            }
        }
        result
    }
}

/// Resolve a `fg=`/`bg=` colour spec into either a full `\e[...m`
/// escape (`sgr = false`) or the bare SGR parameter list used by
/// `${.zle.sgr[name]}` (`sgr = true`).
///
/// Port of `match_colour()` from Src/prompt.c:1957 — the C function
/// `convertattr()` (Src/Modules/hlgroup.c:40) calls indirectly via
/// `match_highlight()` to resolve a color spec. C signature is
/// `match_colour(const char **, int is_fg, int colour) -> zattr`
/// returning a bitmask the renderer later translates to escapes;
/// this Rust port does the resolve + escape in one step (since the
/// `${.zle.esc[name]}` / `${.zle.sgr[name]}` parameters expose the
/// rendered string directly). Handles the same name set
/// (`black`/`red`/.../`bright-red`/`light-red`), 256-colour numeric
/// codes (line 2008 of C source), and `#RRGGBB` truecolor (line 1972).
fn match_colour(color: &str, fg: bool, sgr: bool) -> Option<String> {
    let base = if fg { 30 } else { 40 };
    let bright_base = if fg { 90 } else { 100 };
    let wrap = |n: i32| -> String {
        if sgr {
            n.to_string()
        } else {
            format!("\x1b[{}m", n)
        }
    };
    let wrap_256 = |n: u8| -> String {
        let prefix = if fg { 38 } else { 48 };
        if sgr {
            format!("{};5;{}", prefix, n)
        } else {
            format!("\x1b[{};5;{}m", prefix, n)
        }
    };
    let wrap_truecolor = |r: u8, g: u8, b: u8| -> String {
        let prefix = if fg { 38 } else { 48 };
        if sgr {
            format!("{};2;{};{};{}", prefix, r, g, b)
        } else {
            format!("\x1b[{};2;{};{};{}m", prefix, r, g, b)
        }
    };
    match color {
        "black" => Some(wrap(base)),
        "red" => Some(wrap(base + 1)),
        "green" => Some(wrap(base + 2)),
        "yellow" => Some(wrap(base + 3)),
        "blue" => Some(wrap(base + 4)),
        "magenta" => Some(wrap(base + 5)),
        "cyan" => Some(wrap(base + 6)),
        "white" => Some(wrap(base + 7)),
        "default" => Some(wrap(base + 9)),
        s if s.starts_with("bright-") || s.starts_with("light-") => {
            let inner = s.split_once('-').map(|(_, c)| c)?;
            match inner {
                "black" => Some(wrap(bright_base)),
                "red" => Some(wrap(bright_base + 1)),
                "green" => Some(wrap(bright_base + 2)),
                "yellow" => Some(wrap(bright_base + 3)),
                "blue" => Some(wrap(bright_base + 4)),
                "magenta" => Some(wrap(bright_base + 5)),
                "cyan" => Some(wrap(bright_base + 6)),
                "white" => Some(wrap(bright_base + 7)),
                _ => None,
            }
        }
        s if s.parse::<u8>().is_ok() => Some(wrap_256(s.parse().unwrap())),
        s if s.starts_with('#') && s.len() == 7 => {
            let r = u8::from_str_radix(&s[1..3], 16).ok()?;
            let g = u8::from_str_radix(&s[3..5], 16).ok()?;
            let b = u8::from_str_radix(&s[5..7], 16).ok()?;
            Some(wrap_truecolor(r, g, b))
        }
        _ => None,
    }
}

/// Highlight groups table.
/// Port of the `zle_highlight` lookup hash that backs the
// C source has 0 structs/enums; Rust port matches. The `.zle.esc` /
// `.zle.sgr` magic-assoc pair is wired through the C `partab[]`
// paramdef (hlgroup.c:170-172) tying getpmesc/scanpmesc and
// getpmsgr/scanpmsgr to the underlying `$zle_highlight_groups`
// hash via `getgroup()`. Each is a free fn below.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attr_to_escape_bold() {
        let esc = convertattr("bold", false);
        assert_eq!(esc, "\x1b[1m");
    }

    #[test]
    fn test_attr_to_escape_multiple() {
        let esc = convertattr("bold,underline", false);
        assert!(esc.contains("\x1b[1m"));
        assert!(esc.contains("\x1b[4m"));
    }

    #[test]
    fn test_attr_to_escape_fg_color() {
        let esc = convertattr("fg=red", false);
        assert!(esc.contains("31"));
    }

    #[test]
    fn test_attr_to_sgr_bold() {
        let sgr = convertattr("bold", true);
        assert_eq!(sgr, "1");
    }

    #[test]
    fn test_attr_to_sgr_multiple() {
        let sgr = convertattr("bold,underline", true);
        assert!(sgr.contains("1"));
        assert!(sgr.contains("4"));
    }

    #[test]
    fn test_attr_to_sgr_empty() {
        let sgr = convertattr("", true);
        assert_eq!(sgr, "0");
    }

    #[test]
    fn test_color_256() {
        let esc = convertattr("fg=196", false);
        assert!(esc.contains("38;5;196"));
    }

    #[test]
    fn test_color_truecolor() {
        let esc = convertattr("fg=#ff0000", false);
        assert!(esc.contains("38;2;255;0;0"));
    }
}

/// Port of `setup_()` from `Src/Modules/hlgroup.c:182`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:182
    0                                                                    // c:185
}

/// Port of `features_()` from `Src/Modules/hlgroup.c:189`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
pub fn features_() -> i32 {                                              // c:189
    0                                                                    // c:193
}

/// Port of `enables_()` from `Src/Modules/hlgroup.c:197`. C body is
/// `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:197
    0                                                                    // c:200
}

/// Port of `boot_()` from `Src/Modules/hlgroup.c:204`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn boot_() -> i32 {                                                  // c:204
    0                                                                    // c:207
}

/// Port of `cleanup_()` from `Src/Modules/hlgroup.c:211`. C body is
/// `return setfeatureenables(m, &module_features, NULL);`.
pub fn cleanup_() -> i32 {                                               // c:211
    0                                                                    // c:214
}

/// Port of `finish_()` from `Src/Modules/hlgroup.c:218`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:218
    0                                                                    // c:221
}

/// Port of `getgroup()` from `Src/Modules/hlgroup.c:82`. The shared
/// magic-assoc lookup behind both `${.zle.esc[name]}` and
/// `${.zle.sgr[name]}`. Reads the `$zle_highlight_groups` hashtable
/// for an entry named `name`, then converts its attribute string
/// via `convertattr(...)`.
///
/// `sgr=false` → escape-form (esc), `sgr=true` → SGR-form (sgr).
/// Returns `None` (PM_UNSET) when the group isn't defined.
///
/// C signature: `static HashNode getgroup(const char *name, int sgr)`.
/// Rust port returns `Option<String>` — the string form of the
/// resulting param value.
#[allow(non_snake_case)]
pub fn getgroup(_name: &str, _sgr: bool) -> Option<String> {             // c:82
    // C reads `GROUPVAR` ($zle_highlight_groups) via `getvalue` →
    // hashtable.getfn → gethashnode2; if the entry exists, runs
    // convertattr(entry->u.str, sgr). zshrs's param table
    // integration for this magic-assoc is wired through the
    // executor's hash-param path; until that's plumbed, this
    // entry is the name-parity hook returning UNSET (None) to
    // mirror the C "no entry" branch (hlgroup.c:99-103).
    None                                                                 // c:99-103
}

/// Port of `scangroup()` from `Src/Modules/hlgroup.c:113`. The
/// shared magic-assoc scanner behind `${(k).zle.esc}` etc. Walks
/// every entry in `$zle_highlight_groups` and yields each name +
/// converted-attribute string.
///
/// C signature: `static void scangroup(ScanFunc func, int flags, int sgr)`.
/// Rust port returns the `(name, value)` pairs as a Vec since the
/// callback-driven C API doesn't translate cleanly.
#[allow(non_snake_case)]
pub fn scangroup(_sgr: bool) -> Vec<(String, String)> {                  // c:113
    // C iterates the hashtable behind `$zle_highlight_groups` and
    // calls `func` per entry. Pending the magic-assoc plumbing on
    // the executor side, return empty; mirrors C's "no var" exit
    // branch at hlgroup.c:127-129.
    Vec::new()                                                           // c:127-129
}

/// Port of `getpmesc()` from `Src/Modules/hlgroup.c:141`. C body
/// is `return getgroup(name, 0);` — the escape-form variant.
#[allow(non_snake_case)]
pub fn getpmesc(name: &str) -> Option<String> {                          // c:141
    getgroup(name, false)                                                // c:144
}

/// Port of `scanpmesc()` from `Src/Modules/hlgroup.c:148`. C body
/// is `scangroup(func, flags, 0);` — the escape-form scanner.
#[allow(non_snake_case)]
pub fn scanpmesc() -> Vec<(String, String)> {                            // c:148
    scangroup(false)                                                     // c:151
}

/// Port of `getpmsgr()` from `Src/Modules/hlgroup.c:155`. C body
/// is `return getgroup(name, 1);` — the SGR-form variant.
#[allow(non_snake_case)]
pub fn getpmsgr(name: &str) -> Option<String> {                          // c:155
    getgroup(name, true)                                                 // c:158
}

/// Port of `scanpmsgr()` from `Src/Modules/hlgroup.c:162`. C body
/// is `scangroup(func, flags, 1);` — the SGR-form scanner.
#[allow(non_snake_case)]
pub fn scanpmsgr() -> Vec<(String, String)> {                            // c:162
    scangroup(true)                                                      // c:165
}
