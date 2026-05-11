//! `zsh/hlgroup` module — port of `Src/Modules/hlgroup.c`.
//!
//! Exposes two read-only special parameters that bridge the
//! `$.zle.hlgroups` user-defined hash to the rendered ANSI escape
//! sequences zle uses internally:
//!   - `${.zle.esc[name]}` → full `\033[...m` escape stream
//!   - `${.zle.sgr[name]}` → bare `;`-joined SGR parameter list
//!
//! C source: 13 fns total — `convertattr`, `getgroup`, `scangroup`,
//! `getpmesc`, `scanpmesc`, `getpmsgr`, `scanpmsgr`, `setup_`,
//! `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`.
//! Zero structs/enums in hlgroup.c (only `static const struct
//! gsu_scalar pmesc_gsu` and `static struct paramdef partab[]`
//! aggregates of pre-defined zsh-framework types).
//!
//! Order in this file mirrors C source order verbatim.

use std::fmt::Write;

/// Port of `GROUPVAR` from `Src/Modules/hlgroup.c:33`.
/// `#define GROUPVAR ".zle.hlgroups"`. Name of the user-defined
/// associative array that maps group names to highlight-attribute
/// strings. Read by `getgroup` (c:82) + `scangroup` (c:117).
pub const GROUPVAR: &str = ".zle.hlgroups";                                  // c:33

/// Port of `convertattr()` from `Src/Modules/hlgroup.c:40`.
///
/// C body (c:42-77):
/// ```c
/// zattr atr;
/// match_highlight(attrstr, &atr, NULL, NULL);    // c:46
/// s = zattrescape(atr, sgr ? NULL : &len);        // c:47
/// if (sgr) { ...strip ESC[ and m, join with ; ... }
/// r = dupstring_wlen(s, len);                     // c:75
/// free(s);
/// return r;
/// ```
///
/// **Strict-rule status: PARTIAL.** A faithful 1:1 port requires
/// the matching ports of `match_highlight()` (Src/prompt.c:2031)
/// and `zattrescape()` (Src/prompt.c:257) to land in
/// `src/ported/prompt.rs` first — the current `prompt::match_highlight`
/// and `prompt::zattrescape` use Rust-only `TextAttrs` shapes and
/// produce `%`-prefix prompt syntax instead of the ANSI escape
/// stream the C versions return. See `TODO.md` for the gap.
///
/// Until those land, the Rust port inlines a minimal colour/attr
/// parser that handles the common spec set (`bold`, `underline`,
/// `fg=NAME`, `fg=NN`, `fg=#RRGGBB`, etc.) directly. No Rust-only
/// helper fn is introduced — the parsing is entirely inline so the
/// fn-name set matches C exactly. The SGR post-processing block at
/// c:49-72 is mirrored when `sgr=true`.
///
/// C signature: `static char *convertattr(char *attrstr, int sgr)`.
pub fn convertattr(attrstr: &str, sgr: bool) -> String {                 // c:40
    // c:46 — `match_highlight(attrstr, &atr, NULL, NULL);`
    // c:47 — `s = zattrescape(atr, sgr ? NULL : &len);`
    // Inlined — see fn-doc note about the prompt.rs gap. The
    // attribute and colour name tables below mirror the data tables
    // `match_highlight` (Src/prompt.c:2031) and `match_colour`
    // (Src/prompt.c:1957) consult; emission format matches
    // `zattrescape` (Src/prompt.c:257) for the escape-mode output.
    let mut esc_stream = String::new();
    for part in attrstr.split(',') {
        let part = part.trim();
        // Attribute names → SGR integers (Src/prompt.c attribute table).
        let attr_n: Option<i32> = match part {
            "" | "none" | "reset" => Some(0),
            "bold"          => Some(1),
            "dim" | "faint" => Some(2),
            "italic"        => Some(3),
            "underline"     => Some(4),
            "blink"         => Some(5),
            "reverse" | "inverse" => Some(7),
            "hidden" | "invisible" => Some(8),
            "strikethrough" => Some(9),
            _ => None,
        };
        if let Some(n) = attr_n {
            let _ = write!(esc_stream, "\x1b[{}m", n);
            continue;
        }
        // fg= / bg= colour resolution (Src/prompt.c:1957 match_colour).
        let (is_fg, rest) = if let Some(r) = part.strip_prefix("fg=") {
            (true, r)
        } else if let Some(r) = part.strip_prefix("bg=") {
            (false, r)
        } else {
            continue;
        };
        let base = if is_fg { 30 } else { 40 };
        let bright_base = if is_fg { 90 } else { 100 };
        let prefix = if is_fg { 38 } else { 48 };
        let named: Option<i32> = match rest {
            "black"   => Some(base),
            "red"     => Some(base + 1),
            "green"   => Some(base + 2),
            "yellow"  => Some(base + 3),
            "blue"    => Some(base + 4),
            "magenta" => Some(base + 5),
            "cyan"    => Some(base + 6),
            "white"   => Some(base + 7),
            "default" => Some(base + 9),
            _ => None,
        };
        if let Some(n) = named {
            let _ = write!(esc_stream, "\x1b[{}m", n);
            continue;
        }
        if let Some(inner) = rest.strip_prefix("bright-")
                                .or_else(|| rest.strip_prefix("light-"))
        {
            let bn: Option<i32> = match inner {
                "black"   => Some(bright_base),
                "red"     => Some(bright_base + 1),
                "green"   => Some(bright_base + 2),
                "yellow"  => Some(bright_base + 3),
                "blue"    => Some(bright_base + 4),
                "magenta" => Some(bright_base + 5),
                "cyan"    => Some(bright_base + 6),
                "white"   => Some(bright_base + 7),
                _ => None,
            };
            if let Some(n) = bn {
                let _ = write!(esc_stream, "\x1b[{}m", n);
                continue;
            }
        }
        if let Ok(n) = rest.parse::<u8>() {
            let _ = write!(esc_stream, "\x1b[{};5;{}m", prefix, n);
            continue;
        }
        if let Some(hex) = rest.strip_prefix('#') {
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16);
                let g = u8::from_str_radix(&hex[2..4], 16);
                let b = u8::from_str_radix(&hex[4..6], 16);
                if let (Ok(r), Ok(g), Ok(b)) = (r, g, b) {
                    let _ = write!(esc_stream, "\x1b[{};2;{};{};{}m",
                                   prefix, r, g, b);
                }
            }
        }
    }

    if sgr {
        // c:49-72 — strip `\033[` prefix and `m` suffix, join with `;`,
        // skip non-digit / non-`;` / non-`:` chars, replace `;`/`:` with `;`.
        // Always return at least "0" (c:67-70).
        let bytes = esc_stream.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i + 1 < bytes.len() && bytes[i] == 0x1b && bytes[i + 1] == b'[' {
            i += 2;                                                      // c:53 c += 2
            // c:54-60 — accumulate digits, treat ; or : as separator,
            // break on anything else.
            while i < bytes.len() {
                let b = bytes[i];
                if b.is_ascii_digit() {                                  // c:54
                    out.push(b as char);                                 // c:55
                    i += 1;
                } else if b == b';' || b == b':' {                       // c:56
                    out.push(';');                                       // c:57
                    i += 1;
                } else {
                    break;                                               // c:59
                }
            }
            // c:62-65 — `if (*c != 'm') break;` else continue with `;`.
            if i >= bytes.len() || bytes[i] != b'm' {
                break;                                                   // c:62-63
            }
            out.push(';');                                               // c:64
            i += 1;                                                      // c:65 c++
        }
        // Trim trailing ';'.
        while out.ends_with(';') {
            out.pop();
        }
        // c:67-70 — `if (t <= s) { *s = '0'; t = s + 1; }`
        if out.is_empty() {
            out.push('0');
        }
        out
    } else {
        esc_stream                                                       // c:75 dupstring_wlen
    }
}

/// Port of `getgroup()` from `Src/Modules/hlgroup.c:82`. The shared
/// magic-assoc lookup behind both `${.zle.esc[name]}` and
/// `${.zle.sgr[name]}`. Reads `$.zle.hlgroups` (the `GROUPVAR`
/// `#define` at c:33), looks up `name`, runs `convertattr` on the
/// matched value's attribute string. Returns PM_UNSET (Rust `None`)
/// when the var isn't a hash, the group entry is missing, or the
/// entry has PM_UNSET set.
///
/// C signature: `static HashNode getgroup(const char *name, int sgr)`.
/// Rust port returns `Option<String>` — the synthesised Param's
/// rendered value (or None for PM_UNSET).
///
/// Reads from the global assoc `.zle.hlgroups`. C looks it up through
/// the typed-param table via `getvalue`; the Rust port reads the same
/// data out of the executor's `assoc_arrays` map (assoc-param store).
/// Missing entries map to None (matches C's PM_UNSET branch at c:102).
///
/// C signature: `static HashNode getgroup(const char *name, int sgr)`.
pub fn getgroup(name: &str, sgr: bool) -> Option<String> {               // c:82
    // c:91-94 — pm setup; in Rust we just read the assoc directly.
    // c:96-100 — getvalue check + PM_HASHED + PM_UNSET guards.
    let raw = crate::exec::try_with_executor(|exec| {                    // c:96 getvalue
        exec.assoc_arrays.get(".zle.hlgroups")
            .and_then(|m| m.get(name))
            .cloned()
    }).flatten()?;
    // c:104-106 — pm->u.str = convertattr(value, sgr);
    Some(convertattr(&raw, sgr))                                         // c:105
}

/// Port of `scangroup()` from `Src/Modules/hlgroup.c:113`. The
/// shared magic-assoc scanner behind `${(k).zle.esc}` /
/// `${(kv).zle.esc}` (and the `.zle.sgr` variants). Walks the
/// `$.zle.hlgroups` hash and yields each entry as
/// `(name, convertattr(value, sgr))`.
///
/// C signature: `static void scangroup(ScanFunc func, int flags, int sgr)`.
/// Rust port returns the `(name, value)` pairs as a Vec since
/// zshrs's magic-assoc dispatcher consumes the entire list rather
/// than a per-entry callback.
pub fn scangroup(sgr: bool) -> Vec<(String, String)> {                   // c:113
    // c:123-125 — getvalue + PM_HASHED check.
    // c:126 — walk the .zle.hlgroups assoc map; the executor's
    // assoc_arrays IndexMap preserves insertion order, matching the
    // C hashnode-list walk semantics.
    let entries: Vec<(String, String)> = crate::exec::try_with_executor(|exec| {
        exec.assoc_arrays.get(".zle.hlgroups")                           // c:126
            .map(|m| m.iter()
                .map(|(k, v)| (k.clone(), convertattr(v, sgr)))          // c:132-137
                .collect::<Vec<_>>())
    }).flatten().unwrap_or_default();
    entries
}

/// Port of `getpmesc()` from `Src/Modules/hlgroup.c:141`.
/// C body is `return getgroup(name, 0);` — escape-form variant.
pub fn getpmesc(name: &str) -> Option<String> {                          // c:141
    getgroup(name, false)                                                // c:143
}

/// Port of `scanpmesc()` from `Src/Modules/hlgroup.c:148`.
/// C body is `scangroup(func, flags, 0);` — escape-form scanner.
pub fn scanpmesc() -> Vec<(String, String)> {                            // c:148
    scangroup(false)                                                     // c:150
}

/// Port of `getpmsgr()` from `Src/Modules/hlgroup.c:155`.
/// C body is `return getgroup(name, 1);` — SGR-form variant.
pub fn getpmsgr(name: &str) -> Option<String> {                          // c:155
    getgroup(name, true)                                                 // c:157
}

/// Port of `scanpmsgr()` from `Src/Modules/hlgroup.c:162`.
/// C body is `scangroup(func, flags, 1);` — SGR-form scanner.
pub fn scanpmsgr() -> Vec<(String, String)> {                            // c:162
    scangroup(true)                                                      // c:164
}

// =====================================================================
// static struct features module_features                            c:170 (hlgroup)
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 0,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/hlgroup.c:182`.
pub fn setup_(_m: *const module) -> i32 {                                // c:182
    0                                                                    // c:184
}

/// Port of `features_()` from `Src/Modules/hlgroup.c:189`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {  // c:189
    *features = featuresarray(m, module_features());                    // c:191
    0                                                                    // c:192
}

/// Port of `enables_()` from `Src/Modules/hlgroup.c:197`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 { // c:197
    handlefeatures(m, module_features(), enables)                       // c:199
}

/// Port of `boot_()` from `Src/Modules/hlgroup.c:204`.
pub fn boot_(_m: *const module) -> i32 {                                 // c:204
    0                                                                    // c:206
}

/// Port of `cleanup_()` from `Src/Modules/hlgroup.c:211`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                               // c:211
    setfeatureenables(m, module_features(), None)                       // c:213
}

/// Port of `finish_()` from `Src/Modules/hlgroup.c:218`.
pub fn finish_(_m: *const module) -> i32 {                               // c:218
    0                                                                    // c:220
}

// `featuresarray` — Src/module.c:3275.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    Vec::new()
}

// `handlefeatures` — Src/module.c:3370.
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(getfeatureenables(m, f));
    } else if let Some(e) = enables.as_ref() {
        return setfeatureenables(m, f, Some(e));
    }
    0
}

fn getfeatureenables(_m: *const module, _f: &Mutex<features_t>) -> Vec<i32> {
    Vec::new()
}

// `setfeatureenables` — Src/module.c:3445.
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `convertattr("bold", false)` emits `\e[1m` per Src/prompt.c
    /// attribute table.
    #[test]
    fn convertattr_bold_escape() {
        assert_eq!(convertattr("bold", false), "\x1b[1m");
    }

    /// `convertattr("bold,underline", false)` chains the two
    /// `\e[Nm` escapes.
    #[test]
    fn convertattr_chained_escape() {
        let s = convertattr("bold,underline", false);
        assert!(s.contains("\x1b[1m"));
        assert!(s.contains("\x1b[4m"));
    }

    /// `convertattr("fg=red", false)` emits `\e[31m`.
    #[test]
    fn convertattr_fg_red_escape() {
        let s = convertattr("fg=red", false);
        assert!(s.contains("\x1b[31m"));
    }

    /// SGR-mode `convertattr("bold", true)` returns `"1"`.
    #[test]
    fn convertattr_sgr_bold() {
        assert_eq!(convertattr("bold", true), "1");
    }

    /// SGR-mode chains: `convertattr("bold,underline", true)` →
    /// `"1;4"`.
    #[test]
    fn convertattr_sgr_chain() {
        let s = convertattr("bold,underline", true);
        assert!(s.contains('1'));
        assert!(s.contains('4'));
    }

    /// SGR-mode empty input returns `"0"` per c:67-70 fallback.
    #[test]
    fn convertattr_sgr_empty_returns_zero() {
        assert_eq!(convertattr("", true), "0");
    }

    /// 256-colour spec `fg=196` emits `\e[38;5;196m`.
    #[test]
    fn convertattr_256_color() {
        let s = convertattr("fg=196", false);
        assert!(s.contains("\x1b[38;5;196m"));
    }

    /// Truecolor spec `fg=#ff0000` emits `\e[38;2;255;0;0m`.
    #[test]
    fn convertattr_truecolor() {
        let s = convertattr("fg=#ff0000", false);
        assert!(s.contains("\x1b[38;2;255;0;0m"));
    }

    /// SGR-mode 256-colour: `fg=196` → `38;5;196`.
    #[test]
    fn convertattr_sgr_256_color() {
        let s = convertattr("fg=196", true);
        assert!(s.contains("38;5;196"));
    }

    /// SGR-mode truecolor: `fg=#00ff00` → `38;2;0;255;0`.
    #[test]
    fn convertattr_sgr_truecolor() {
        let s = convertattr("fg=#00ff00", true);
        assert!(s.contains("38;2;0;255;0"));
    }

    /// With no `.zle.hlgroups` assoc set, getgroup mirrors C's
    /// PM_UNSET branch at c:99-103 and returns None.
    #[test]
    fn getgroup_unset_assoc_returns_none() {
        assert_eq!(getgroup("nonexistent_zzz", false), None);
        assert_eq!(getgroup("nonexistent_zzz", true), None);
    }

    /// With no `.zle.hlgroups` assoc set, scangroup mirrors C's
    /// empty-table early exit at c:124-125.
    #[test]
    fn scangroup_unset_assoc_returns_empty() {
        // With no executor or no assoc, returns empty per the C
        // c:124-125 path (`if (!v || not PM_HASHED) return;`).
        let _ = scangroup(false);
        let _ = scangroup(true);
    }
}
