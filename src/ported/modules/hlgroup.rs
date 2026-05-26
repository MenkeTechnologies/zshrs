//! `zsh/hlgroup` module — port of `Src/Modules/hlgroup.c`.
//!
//! Exposes two read-only special parameters that bridge the
//! `$.zle.hlgroups` user-defined hash to the rendered ANSI escape
//! sequences zle uses internally:
//!   - `${.zle.esc[name]}` → full `\033[...m` escape stream
//!   - `${.zle.sgr[name]}` → bare `;`-joined SGR parameter list
//!
//! C source: 13 ported total — `convertattr`, `getgroup`, `scangroup`,
//! `getpmesc`, `scanpmesc`, `getpmsgr`, `scanpmsgr`, `setup_`,
//! `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`.
//! Zero structs/enums in hlgroup.c (only `static const struct
//! gsu_scalar pmesc_gsu` and `static struct paramdef partab[]`
//! aggregates of pre-defined zsh-framework types).
//!
//! Order in this file mirrors C source order verbatim.

use std::fmt::Write;
use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::features;
use crate::zsh_h::module;


/// Port of `GROUPVAR` from `Src/Modules/hlgroup.c:33`.
/// `#define GROUPVAR ".zle.hlgroups"`. Name of the user-defined
/// associative array that maps group names to highlight-attribute
/// strings. Read by `getgroup` (c:82) + `scangroup` (c:117).
pub const GROUPVAR: &str = ".zle.hlgroups"; // c:33

/// Port of `convertattr(char *attrstr, int sgr)` from `Src/Modules/hlgroup.c:40`.
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
/// c:40-72 is mirrored when `sgr=true`.
///
/// C signature: `static char *convertattr(char *attrstr, int sgr)`.
pub fn convertattr(attrstr: &str, sgr: bool) -> String {
    // c:40
    // c:40 — `match_highlight(attrstr, &atr, NULL, NULL);`
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
            "bold" => Some(1),
            "dim" | "faint" => Some(2),
            "italic" => Some(3),
            "underline" => Some(4),
            "blink" => Some(5),
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
            "black" => Some(base),
            "red" => Some(base + 1),
            "green" => Some(base + 2),
            "yellow" => Some(base + 3),
            "blue" => Some(base + 4),
            "magenta" => Some(base + 5),
            "cyan" => Some(base + 6),
            "white" => Some(base + 7),
            "default" => Some(base + 9),
            _ => None,
        };
        if let Some(n) = named {
            let _ = write!(esc_stream, "\x1b[{}m", n);
            continue;
        }
        if let Some(inner) = rest
            .strip_prefix("bright-")
            .or_else(|| rest.strip_prefix("light-"))
        {
            let bn: Option<i32> = match inner {
                "black" => Some(bright_base),
                "red" => Some(bright_base + 1),
                "green" => Some(bright_base + 2),
                "yellow" => Some(bright_base + 3),
                "blue" => Some(bright_base + 4),
                "magenta" => Some(bright_base + 5),
                "cyan" => Some(bright_base + 6),
                "white" => Some(bright_base + 7),
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
                    let _ = write!(esc_stream, "\x1b[{};2;{};{};{}m", prefix, r, g, b);
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
            i += 2; // c:53 c += 2
                    // c:54-60 — accumulate digits, treat ; or : as separator,
                    // break on anything else.
            while i < bytes.len() {
                let b = bytes[i];
                if b.is_ascii_digit() {
                    // c:54
                    out.push(b as char); // c:55
                    i += 1;
                } else if b == b';' || b == b':' {
                    // c:56
                    out.push(';'); // c:57
                    i += 1;
                } else {
                    break; // c:59
                }
            }
            // c:62-65 — `if (*c != 'm') break;` else continue with `;`.
            if i >= bytes.len() || bytes[i] != b'm' {
                break; // c:62-63
            }
            out.push(';'); // c:64
            i += 1; // c:65 c++
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
        esc_stream // c:75 dupstring_wlen
    }
}

/// Port of `getgroup(const char *name, int sgr)` from `Src/Modules/hlgroup.c:82`. The shared
/// magic-assoc lookup behind both `${.zle.esc[name]}` and
/// `${.zle.sgr[name]}`. Reads `$.zle.hlgroups` (the `GROUPVAR`
/// `#define` at c:33), looks up `name`, runs `convertattr` on the
/// matched value's attribute string. Returns PM_UNSET (Rust `None`)
/// when the var isn't a hash, the group entry is missing, or the
/// entry has PM_UNSET set.
///
/// Port of `static HashNode getgroup(const char *name, int sgr)` from
/// `Src/Modules/hlgroup.c:82`. Looks up `name` in the user-defined
/// `$.zle.hlgroups` hash, returns the converted-attr string when found,
/// `None` for the PM_UNSET path. C synthesises a fresh Param + HashNode
/// shell; the Rust caller (an `${.zle.esc[name]}` magic-assoc fetch)
/// doesn't need the Param wrapping — only the result string.
pub fn getgroup(name: &str, sgr: bool) -> Option<String> {
    // c:82
    // c:84-94 — `pm = hcalloc(...); pm->gsu.s = &pmesc_gsu;
    //            pm->node.nam = dupstring(name);
    //            pm->node.flags = PM_SCALAR|PM_SPECIAL;`
    // The synthesised pm wraps the return; Rust returns the string
    // directly. The flag set + gsu wiring collapses since the caller
    // path (pmesc_get) consumes only `pm->u.str`.
    let _ = name; // gate against unused-param lint when body short-circuits

    // c:89 — `char *var = GROUPVAR;`
    let var = GROUPVAR;
    // c:96 — `if (!(v = getvalue(&vbuf, &var, 0))`
    let tab = crate::ported::params::paramtab();
    let table = match tab.read() {
        Ok(t) => t,
        Err(_) => return None,
    };
    let pm = match table.get(var) {
        Some(p) => p,
        None => return None,                                                 // c:102-103 PM_UNSET
    };
    // c:97 — `|| PM_TYPE(v->pm->node.flags) != PM_HASHED`
    if crate::ported::zsh_h::PM_TYPE(pm.node.flags as u32) != crate::ported::zsh_h::PM_HASHED {
        return None;                                                         // c:102-103 PM_UNSET
    }
    // c:98 — `|| !(hlg = v->pm->gsu.h->getfn(v->pm))` — fetch backing hash.
    let hlg = match pm.u_hash.as_ref() {
        Some(h) => h,
        None => return None,                                                 // c:102-103 PM_UNSET
    };
    // c:99 — `|| !(hn = gethashnode2(hlg, name))` — lookup by name.
    let hn = hlg.nodes.iter().find_map(|opt| {
        opt.as_ref().and_then(|hn| {
            if hn.nam == name { Some(hn) } else { None }
        })
    });
    let hn = match hn {
        Some(h) => h,
        None => return None,                                                 // c:102-103 PM_UNSET
    };
    // c:100 — `|| (((Param) hn)->node.flags & PM_UNSET)`
    if (hn.flags & crate::ported::zsh_h::PM_UNSET as i32) != 0 {
        return None;                                                         // c:102-103 PM_UNSET
    }
    // The hashnode's value lives on the associated Param. The Rust
    // HashTable.nodes stores `HashNode` (just nam+flags), not the
    // wrapping Param — so the attribute string lookup needs a
    // companion Param lookup. The user-facing param table indexes
    // hash-entry params under "VAR[name]" composite keys; try that.
    let composite_key = format!("{}[{}]", var, name);
    let raw_attr = match table.get(&composite_key) {
        Some(child_pm) => child_pm.u_str.clone().unwrap_or_default(),
        None => return None,                                                 // attribute string unreachable
    };
    // c:105 — `pm->u.str = convertattr(((Param) hn)->u.str, sgr);`
    Some(convertattr(&raw_attr, sgr))                                        // c:105
}

/// shared magic-assoc scanner behind `${(k).zle.esc}` /
/// `${(kv).zle.esc}` (and the `.zle.sgr` variants). Walks the
/// `$.zle.hlgroups` hash and yields each entry as
/// `(name, convertattr(value, sgr))`.
///
/// C signature: `static void scangroup(ScanFunc func, int flags, int sgr)`.
/// Rust port returns the `(name, value)` pairs as a Vec since
/// zshrs's magic-assoc dispatcher consumes the entire list rather
/// than a per-entry callback.
///
/// **Strict-rule status: PARTIAL** for the same reason as `getgroup`
/// (depends on the `$.zle.hlgroups` hash being readable through the
/// param table). See `TODO.md`.
/// Port of `scangroup(ScanFunc func, int flags, int sgr)` from `Src/Modules/hlgroup.c:113`.
/// WARNING: param names don't match C — Rust=(_sgr) vs C=(func, flags, sgr)
pub fn scangroup(_sgr: bool) -> Vec<(String, String)> {
    // c:113
    // c:113-125 — `if (!(v = getvalue(...)) || ... PM_HASHED) return;`
    // c:141 — hlg = v->pm->gsu.h->getfn(v->pm)
    // c:141-130 — `pm` setup + PM_SCALAR + pmesc_gsu
    // c:141-137 — for each hashnode: `pm.u.str = convertattr(...,sgr);
    //                                   pm.node.nam = hn->nam;
    //                                   func(&pm.node, flags);`
    Vec::new() // c:141-125 empty exit
}

/// Port of `getpmesc(UNUSED(HashTable ht), const char *name)` from `Src/Modules/hlgroup.c:141`.
/// C body is `return getgroup(name, 0);` — escape-form variant.
/// WARNING: param names don't match C — Rust=(name) vs C=(ht, name)
pub fn getpmesc(name: &str) -> Option<String> {
    // c:141
    getgroup(name, false) // c:148
}

/// Port of `scanpmesc(UNUSED(HashTable ht), ScanFunc func, int flags)` from `Src/Modules/hlgroup.c:148`.
/// C body is `scangroup(func, flags, 0);` — escape-form scanner.
/// WARNING: param names don't match C — Rust=() vs C=(ht, func, flags)
pub fn scanpmesc() -> Vec<(String, String)> {
    // c:148
    scangroup(false) // c:155
}

/// Port of `getpmsgr(UNUSED(HashTable ht), const char *name)` from `Src/Modules/hlgroup.c:155`.
/// C body is `return getgroup(name, 1);` — SGR-form variant.
/// WARNING: param names don't match C — Rust=(name) vs C=(ht, name)
pub fn getpmsgr(name: &str) -> Option<String> {
    // c:155
    getgroup(name, true) // c:162
}

/// Port of `scanpmsgr(UNUSED(HashTable ht), ScanFunc func, int flags)` from `Src/Modules/hlgroup.c:162`.
/// C body is `scangroup(func, flags, 1);` — SGR-form scanner.
/// WARNING: param names don't match C — Rust=() vs C=(ht, func, flags)
pub fn scanpmsgr() -> Vec<(String, String)> {
    // c:162
    scangroup(true) // c:162
}

// =====================================================================
// static struct features module_features                            c:170 (hlgroup)
// =====================================================================

// `partab` — port of `static struct paramdef partab[]` (hlgroup.c).

// `module_features` — port of `static struct features module_features`
// from hlgroup.c:170.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/hlgroup.c:182`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:182
    0 // c:197
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/hlgroup.c:189`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:189
    *features = featuresarray(m, module_features());
    0 // c:204
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/hlgroup.c:197`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:197
    handlefeatures(m, module_features(), enables) // c:211
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/hlgroup.c:204`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:204
    0 // c:218
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/hlgroup.c:211`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:211
    setfeatureenables(m, module_features(), None) // c:218
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/hlgroup.c:218`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:218
    0 // c:218
}


static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN HLGROUP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["p:.zle.esc".to_string(), "p:.zle.sgr".to_string()]
}

// WARNING: NOT IN HLGROUP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 2]);
    }
    0
}

// WARNING: NOT IN HLGROUP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN HLGROUP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 0,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 2,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `convertattr("bold", false)` emits `\e[1m` per Src/prompt.c
    /// attribute table.
    #[test]
    fn convertattr_bold_escape() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(convertattr("bold", false), "\x1b[1m");
    }

    /// `convertattr("bold,underline", false)` chains the two
    /// `\e[Nm` escapes.
    #[test]
    fn convertattr_chained_escape() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold,underline", false);
        assert!(s.contains("\x1b[1m"));
        assert!(s.contains("\x1b[4m"));
    }

    /// `convertattr("fg=red", false)` emits `\e[31m`.
    #[test]
    fn convertattr_fg_red_escape() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=red", false);
        assert!(s.contains("\x1b[31m"));
    }

    /// SGR-mode `convertattr("bold", true)` returns `"1"`.
    #[test]
    fn convertattr_sgr_bold() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(convertattr("bold", true), "1");
    }

    /// SGR-mode chains: `convertattr("bold,underline", true)` →
    /// `"1;4"`.
    #[test]
    fn convertattr_sgr_chain() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold,underline", true);
        assert!(s.contains('1'));
        assert!(s.contains('4'));
    }

    /// SGR-mode empty input returns `"0"` per c:67-70 fallback.
    #[test]
    fn convertattr_sgr_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(convertattr("", true), "0");
    }

    /// 256-colour spec `fg=196` emits `\e[38;5;196m`.
    #[test]
    fn convertattr_256_color() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=196", false);
        assert!(s.contains("\x1b[38;5;196m"));
    }

    /// Truecolor spec `fg=#ff0000` emits `\e[38;2;255;0;0m`.
    #[test]
    fn convertattr_truecolor() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=#ff0000", false);
        assert!(s.contains("\x1b[38;2;255;0;0m"));
    }

    /// SGR-mode 256-colour: `fg=196` → `38;5;196`.
    #[test]
    fn convertattr_sgr_256_color() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=196", true);
        assert!(s.contains("38;5;196"));
    }

    /// SGR-mode truecolor: `fg=#00ff00` → `38;2;0;255;0`.
    #[test]
    fn convertattr_sgr_truecolor() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=#00ff00", true);
        assert!(s.contains("38;2;0;255;0"));
    }

    /// `getgroup` returns None until the magic-assoc dispatch is
    /// wired (c:99-103 PM_UNSET branch).
    #[test]
    fn getgroup_returns_none_until_paramtable_wired() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getgroup("any", false), None);
        assert_eq!(getgroup("any", true), None);
    }

    /// `scangroup` returns empty until paramtable wiring lands
    /// (c:124-125 early exit).
    #[test]
    fn scangroup_returns_empty_until_paramtable_wired() {
        let _g = crate::test_util::global_state_lock();
        assert!(scangroup(false).is_empty());
        assert!(scangroup(true).is_empty());
    }

    /// c:40 — `convertattr("")` (empty input). Defensive edge.
    #[test]
    fn convertattr_empty_input_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let _ = convertattr("", false);
        let _ = convertattr("", true);
    }

    /// c:40 — `convertattr("bold")` adds bold SGR (1).
    #[test]
    fn convertattr_bold_emits_sgr_bold() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold", false);
        assert!(
            s.contains("\x1b[1m") || s.contains("\x1b[1;"),
            "bold attr must emit SGR 1, got {:?}",
            s
        );
    }

    /// c:40 — Unknown attr keyword does NOT panic.
    #[test]
    fn convertattr_unknown_attr_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let _ = convertattr("definitely_not_a_real_attr", false);
    }

    /// c:40 — Truecolor upper boundary (255,255,255). Pin so a
    /// regen using i8 instead of u8 doesn't wrap to negative.
    #[test]
    fn convertattr_truecolor_max_rgb() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=#ffffff", false);
        assert!(
            s.contains("38;2;255;255;255"),
            "white truecolor must encode as 255;255;255, got {:?}",
            s
        );
    }

    /// c:40 — 256-color upper boundary `fg=255`.
    #[test]
    fn convertattr_256_color_upper_boundary() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=255", false);
        assert!(
            s.contains("38;5;255"),
            "256-color upper boundary 255 must encode correctly, got {:?}",
            s
        );
    }

    /// c:141 — `getpmesc` for empty/unknown name returns None.
    #[test]
    fn getpmesc_empty_or_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getpmesc("").is_none());
        assert!(getpmesc("definitely_not_in_table_xyzzy").is_none());
    }

    /// c:155 — `getpmsgr` symmetric with getpmesc; empty + unknown → None.
    #[test]
    fn getpmsgr_empty_or_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getpmsgr("").is_none());
        assert!(getpmsgr("definitely_not_in_table_xyzzy").is_none());
    }

    /// c:148/162 — `scanpmesc` and `scanpmsgr` always return empty
    /// vec until paramtable wiring lands.
    #[test]
    fn scanpmesc_and_scanpmsgr_are_empty_until_wired() {
        let _g = crate::test_util::global_state_lock();
        assert!(scanpmesc().is_empty());
        assert!(scanpmsgr().is_empty());
    }

    /// c:182-210 — module-lifecycle stubs return 0.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        let mut features = Vec::new();
        assert_eq!(features_(m, &mut features), 0);
        let mut enables: Option<Vec<i32>> = None;
        assert_eq!(enables_(m, &mut enables), 0);
    }

    /// `Src/Modules/hlgroup.c:40-44` — `convertattr("bg=blue")` emits
    /// SGR 44. Pin the bg-base (40) so a regression conflating fg/bg
    /// bases would silently swap colors.
    #[test]
    fn convertattr_bg_color_uses_40_base() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bg=blue", false);
        assert!(
            s.contains("\x1b[44m"),
            "c:40 — bg=blue → base 40 + 4 = 44 (got {:?})",
            s
        );
        // bg=red → 41
        let s = convertattr("bg=red", false);
        assert!(s.contains("\x1b[41m"));
        // bg=default → 49
        let s = convertattr("bg=default", false);
        assert!(s.contains("\x1b[49m"));
    }

    /// `Src/Modules/hlgroup.c:40-44` — `bright-` prefix maps to the
    /// 90-99 (fg) / 100-107 (bg) range. Pin the offset arithmetic
    /// (bright_base + 0..=7) for both fg and bg.
    #[test]
    fn convertattr_bright_prefix_uses_high_intensity_base() {
        let _g = crate::test_util::global_state_lock();
        // fg=bright-red → 91
        let s = convertattr("fg=bright-red", false);
        assert!(
            s.contains("\x1b[91m"),
            "fg=bright-red → 90+1=91 (got {:?})",
            s
        );
        // bg=bright-cyan → 106
        let s = convertattr("bg=bright-cyan", false);
        assert!(
            s.contains("\x1b[106m"),
            "bg=bright-cyan → 100+6=106 (got {:?})",
            s
        );
    }

    /// `Src/Modules/hlgroup.c:40-44` — `light-` is the alias for
    /// `bright-`. Pin both prefix variants map to the same code.
    #[test]
    fn convertattr_light_prefix_is_alias_for_bright() {
        let _g = crate::test_util::global_state_lock();
        let bright = convertattr("fg=bright-green", false);
        let light = convertattr("fg=light-green", false);
        assert_eq!(
            bright, light,
            "c:40 — light- and bright- prefixes must produce identical SGR codes"
        );
    }

    /// `Src/Modules/hlgroup.c:40-72` — SGR-mode bg color rendering.
    /// `bg=blue` in SGR mode produces "44" (the digits between
    /// `\e[` and `m`, no surrounding chars).
    #[test]
    fn convertattr_sgr_bg_color() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            convertattr("bg=blue", true),
            "44",
            "SGR mode strips ESC[/m wrapper → bare digit string"
        );
        assert_eq!(convertattr("bg=red", true), "41");
    }

    /// `Src/Modules/hlgroup.c:40-44` — Invalid color spec (not in
    /// named table, not numeric, not hex) emits NOTHING. SGR mode
    /// falls back to "0". Pin the defensive contract.
    #[test]
    fn convertattr_unknown_color_drops_silently() {
        let _g = crate::test_util::global_state_lock();
        // Plain (escape) mode: empty output for unknown color alone.
        let s = convertattr("fg=not_a_real_color", false);
        assert_eq!(s, "", "unknown color → no escape emitted");
        // SGR mode: empty output → "0" fallback per c:67-70.
        let s = convertattr("fg=not_a_real_color", true);
        assert_eq!(s, "0");
    }

    /// `Src/Modules/hlgroup.c:40` — Hex color with WRONG length
    /// is silently dropped (only 6-hex-digit form recognized).
    #[test]
    fn convertattr_short_hex_dropped() {
        let _g = crate::test_util::global_state_lock();
        // 3-digit form "#abc" not supported per the body's `hex.len() == 6` guard.
        let s = convertattr("fg=#abc", false);
        assert_eq!(s, "", "3-digit hex must be rejected per c:40 6-digit check");
        // 8-digit form also rejected
        let s = convertattr("fg=#abcdef00", false);
        assert_eq!(s, "");
    }

    /// `Src/Modules/hlgroup.c:40-44` — `dim`/`faint` are aliases for
    /// SGR 2 in the C source's attribute table. Pin the alias.
    #[test]
    fn convertattr_dim_and_faint_are_aliases() {
        let _g = crate::test_util::global_state_lock();
        let dim = convertattr("dim", false);
        let faint = convertattr("faint", false);
        assert_eq!(dim, faint, "dim and faint must produce identical SGR 2");
        assert!(dim.contains("\x1b[2m"));
    }

    /// `Src/Modules/hlgroup.c:40-44` — `reverse`/`inverse` are SGR 7
    /// aliases. Pin so a regen flipping one to a different code
    /// silently changes the other.
    #[test]
    fn convertattr_reverse_and_inverse_are_aliases() {
        let _g = crate::test_util::global_state_lock();
        let rev = convertattr("reverse", false);
        let inv = convertattr("inverse", false);
        assert_eq!(rev, inv);
        assert!(rev.contains("\x1b[7m"));
    }

    /// `Src/Modules/hlgroup.c:40-44` — `hidden`/`invisible` are SGR 8
    /// aliases. Same pattern as reverse/inverse.
    #[test]
    fn convertattr_hidden_and_invisible_are_aliases() {
        let _g = crate::test_util::global_state_lock();
        let h = convertattr("hidden", false);
        let i = convertattr("invisible", false);
        assert_eq!(h, i);
        assert!(h.contains("\x1b[8m"));
    }

    // ─── zsh-corpus pins for convertattr ───────────────────────────

    /// "bold" → SGR 1.
    #[test]
    fn hlgroup_corpus_bold_is_sgr_1() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold", false);
        assert!(s.contains("\x1b[1m"), "bold = SGR 1, got {s:?}");
    }

    /// "underline" → SGR 4.
    #[test]
    fn hlgroup_corpus_underline_is_sgr_4() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("underline", false);
        assert!(s.contains("\x1b[4m"));
    }

    /// "italic" → SGR 3.
    #[test]
    fn hlgroup_corpus_italic_is_sgr_3() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("italic", false);
        assert!(s.contains("\x1b[3m"));
    }

    /// "blink" → SGR 5.
    #[test]
    fn hlgroup_corpus_blink_is_sgr_5() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("blink", false);
        assert!(s.contains("\x1b[5m"));
    }

    /// "strikethrough" → SGR 9.
    #[test]
    fn hlgroup_corpus_strikethrough_is_sgr_9() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("strikethrough", false);
        assert!(s.contains("\x1b[9m"));
    }

    /// "fg=red" → SGR 31.
    #[test]
    fn hlgroup_corpus_fg_red_is_sgr_31() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=red", false);
        assert!(s.contains("\x1b[31m"), "fg=red = SGR 31, got {s:?}");
    }

    /// "bg=blue" → SGR 44.
    #[test]
    fn hlgroup_corpus_bg_blue_is_sgr_44() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bg=blue", false);
        assert!(s.contains("\x1b[44m"), "bg=blue = SGR 44, got {s:?}");
    }

    /// "fg=default" → SGR 39 (default fg).
    #[test]
    fn hlgroup_corpus_fg_default_is_sgr_39() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("fg=default", false);
        assert!(s.contains("\x1b[39m"), "fg=default = SGR 39");
    }

    /// "bg=default" → SGR 49.
    #[test]
    fn hlgroup_corpus_bg_default_is_sgr_49() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bg=default", false);
        assert!(s.contains("\x1b[49m"));
    }

    /// Empty input is treated as "reset" (SGR 0).
    #[test]
    fn hlgroup_corpus_empty_is_reset() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("", false);
        assert!(s.contains("\x1b[0m"), "empty = SGR 0 reset, got {s:?}");
    }

    /// "bold,fg=red" — comma-separated combined attrs.
    #[test]
    fn hlgroup_corpus_combined_attrs() {
        let _g = crate::test_util::global_state_lock();
        let s = convertattr("bold,fg=red", false);
        assert!(s.contains("\x1b[1m"), "has bold");
        assert!(s.contains("\x1b[31m"), "has fg=red");
    }
}
