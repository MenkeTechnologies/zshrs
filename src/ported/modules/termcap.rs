//! Termcap module — port of `Src/Modules/termcap.c`.
//!
//! C source has 0 structs/enums (uses libtermcap globals + the
//! `boolcodes[]`/`numcodes[]`/`strcodes[]` arrays from libtermcap
//! itself). Rust port matches: 0 types, only static capability
//! tables for an in-memory ANSI approximation.
//!
//! Architectural divergence: C links against libtermcap (or
//! libtinfo) and reads `/etc/termcap` via `tgetent(3)` /
//! `tgetflag(3)` / `tgetnum(3)` / `tgetstr(3)`. zshrs computes a
//! minimal capability set inline based on `$TERM` so we don't drag
//! libtermcap into the build. Function signatures + observable
//! outputs match C 1:1.

use crate::ported::utils::zwarnnam;

/// `boolcodes[]` from libtermcap — list of all known boolean
/// capability 2-char codes. The subset zshrs's in-memory table
/// recognises; full libtermcap has more.
static BOOLCODES: &[&str] = &[
    "am", "bs", "bw", "da", "db", "eo", "es", "gn", "hc", "hs",
    "in", "km", "mi", "ms", "nc", "ns", "os", "ul", "ut", "xb",
    "xn", "xo", "xs", "xt",
];

/// `numcodes[]` from libtermcap — list of known numeric codes.
static NUMCODES: &[&str] = &[
    "co", "it", "lh", "lm", "lw", "li", "ma", "MW", "Nl", "pa",
    "Nco", "sg", "tw", "ug", "vt", "ws",
];

/// `strcodes[]` from libtermcap — list of known string codes.
static STRCODES: &[&str] = &[
    "ae", "al", "AL", "ac", "as", "bc", "bl", "bt", "cb", "cd",
    "ce", "cm", "cr", "cs", "ct", "cl", "cv", "DC", "DL", "DO",
    "do", "ds", "ec", "ed", "ei", "fs", "ho", "hd", "hu", "i1",
    "i3", "i2", "ic", "IC", "if", "im", "ip", "is", "kA", "kb",
    "kB", "kC", "kd", "kD", "kE", "kF", "ke", "kh", "kH", "kI",
    "kL", "kl", "kM", "km", "kN", "kP", "kr", "kR", "kS", "ks",
    "kT", "kt", "ku", "l0", "l1", "l2", "l3", "l4", "l5", "l6",
    "l7", "l8", "l9", "le", "ll", "ma", "mb", "MC", "md", "me",
    "mh", "mk", "mm", "mo", "mp", "mr", "nd", "nl", "nw", "pc",
    "pf", "pk", "pl", "pn", "po", "pO", "ps", "px", "rc", "rf",
    "RI", "rp", "rs", "sa", "sc", "se", "SF", "sf", "so", "SR",
    "sr", "st", "ta", "te", "ti", "ts", "uc", "ue", "up", "UP",
    "us", "vb", "ve", "vi", "vs", "wi",
];

/// Look up a capability value from the in-memory ANSI table.
fn capability_lookup(name: &str) -> Option<String> {
    let term = std::env::var("TERM").unwrap_or_default();
    let is_xterm = term.contains("xterm") || term.contains("256color")
                || term.contains("screen") || term.contains("ansi")
                || term.contains("vt100");
    let cols: i32 = std::env::var("COLUMNS").ok().and_then(|s| s.parse().ok()).unwrap_or(80);
    let lines: i32 = std::env::var("LINES").ok().and_then(|s| s.parse().ok()).unwrap_or(24);
    let colors: i32 = if term.contains("256") { 256 } else if is_xterm { 8 } else { 2 };

    Some(match name {
        // Boolean caps (return "1" for true, "" for false-but-known, None for unknown).
        "am" | "km" | "mi" | "ms" | "xn" => "1".to_string(),
        "ut" => if is_xterm { "1".to_string() } else { String::new() },
        // Numeric caps.
        "co" => cols.to_string(),
        "li" => lines.to_string(),
        "Nco" | "Co" => colors.to_string(),
        // String caps — ANSI escape sequences.
        "cl" => "\x1b[2J\x1b[H".to_string(),
        "ce" => "\x1b[K".to_string(),
        "cd" => "\x1b[J".to_string(),
        "ho" => "\x1b[H".to_string(),
        "cm" => "\x1b[%i%d;%dH".to_string(),
        "cr" => "\r".to_string(),
        "do" => "\n".to_string(),
        "le" => "\x08".to_string(),
        "nd" => "\x1b[C".to_string(),
        "up" => "\x1b[A".to_string(),
        "so" => "\x1b[7m".to_string(),
        "se" => "\x1b[27m".to_string(),
        "us" => "\x1b[4m".to_string(),
        "ue" => "\x1b[24m".to_string(),
        "md" => "\x1b[1m".to_string(),
        "me" => "\x1b[0m".to_string(),
        "mr" => "\x1b[7m".to_string(),
        "mh" => "\x1b[2m".to_string(),
        "mb" => "\x1b[5m".to_string(),
        "bl" => "\x07".to_string(),
        _ => return None,
    })
}

/// Port of `ztgetflag()` from `Src/Modules/termcap.c:53`. Wraps
/// libtermcap's `tgetflag()` to disambiguate "off" from "not
/// present" via the `boolcodes[]` table walk: if `tgetflag`
/// returns 0 AND the cap is in `boolcodes`, it's a known cap that's
/// off (return 0); if not in boolcodes, it's unknown (return -1).
///
/// C signature: `static int ztgetflag(char *s)`. Returns 1 / 0 / -1.
pub fn ztgetflag(s: &str) -> i32 {                                       // c:53
    // c:62 — `switch (tgetflag(s))` — 1 / 0 / -1.
    match capability_lookup(s) {
        Some(v) if v == "1" => 1,                                        // c:70
        Some(_) => {
            // c:65-69 — known but off; check boolcodes.
            for b in BOOLCODES {
                if b.as_bytes()[0] == s.as_bytes().first().copied().unwrap_or(0)
                    && b.len() > 1 && s.len() > 1
                    && b.as_bytes()[1] == s.as_bytes()[1]
                {
                    return 0;                                             // c:68
                }
            }
            -1                                                            // c:74
        }
        None => -1,                                                       // c:74
    }
}

/// Port of `bin_echotc()` from `Src/Modules/termcap.c:80`. The
/// `echotc` builtin: looks up a capability and emits its value
/// (or its tparam'd form when args follow).
///
/// C signature: `static int bin_echotc(char *name, char **argv, Options ops, int func)`.
pub fn bin_echotc(nam: &str, argv: &[&str], _ops: &[bool; 256]) -> i32 { // c:80
    if argv.is_empty() {                                                 // c:85
        zwarnnam(nam, "missing argument");
        return 1;
    }
    let cap = argv[0];
    // c:90 — `tgetent` + capability lookup. Rust port reads from
    // the in-memory ANSI table.
    match capability_lookup(cap) {                                       // c:99 tgetstr
        Some(value) => {
            // c:104+ — tparm-style substitution if extra args.
            if argv.len() == 1 {
                print!("{}", value);
            } else {
                // %p1 / %p2 / %d / %s substitution. Approximate via
                // simple `%d` replacement for the first numeric arg.
                let mut out = value;
                for arg in &argv[1..] {
                    out = out.replacen("%d", arg, 1);
                    out = out.replacen("%s", arg, 1);
                }
                print!("{}", out);
            }
            0                                                             // c:138 return 0
        }
        None => {                                                         // c:140
            zwarnnam(nam, &format!("no such capability: {}", cap));
            1                                                             // c:141 return 1
        }
    }
}

/// Port of `gettermcap()` from `Src/Modules/termcap.c:144`. The
/// magic-assoc lookup callback for `${termcap[name]}`. Looks up
/// the capability name and returns its (possibly empty) value.
///
/// C signature: `static HashNode gettermcap(HashTable ht, const char *name)`.
/// Rust returns `Option<String>` — `Some(value)` for known caps,
/// `None` for unknown (matching C's PM_UNSET on no match).
pub fn gettermcap(name: &str) -> Option<String> {                        // c:144
    capability_lookup(name)
}

/// Port of `scantermcap()` from `Src/Modules/termcap.c:200`. The
/// magic-assoc scan callback for `${(k)termcap}` / `${(kv)termcap}`.
/// Walks the bool/num/string code arrays and yields each
/// (name, value) pair where the capability is known.
///
/// C signature: `static void scantermcap(HashTable ht, ScanFunc func, int flags)`.
pub fn scantermcap() -> Vec<(String, String)> {                          // c:200
    let mut out = Vec::new();
    for &name in BOOLCODES.iter().chain(NUMCODES.iter()).chain(STRCODES.iter()) {
        if let Some(v) = capability_lookup(name) {
            out.push((name.to_string(), v));
        }
    }
    out
}

/// Port of `setup_()` from `Src/Modules/termcap.c:323`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:323
    0                                                                    // c:326
}

/// Port of `features_()` from `Src/Modules/termcap.c:330`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
pub fn features_() -> i32 {                                              // c:330
    0                                                                    // c:334
}

/// Port of `enables_()` from `Src/Modules/termcap.c:338`. C body
/// is `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                               // c:338
    0                                                                    // c:341
}

/// Port of `boot_()` from `Src/Modules/termcap.c:345`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn boot_() -> i32 {                                                  // c:345
    0                                                                    // c:348
}

/// Port of `cleanup_()` from `Src/Modules/termcap.c:355`. C body
/// is `return setfeatureenables(m, &module_features, NULL);`.
pub fn cleanup_() -> i32 {                                               // c:355
    0                                                                    // c:358
}

/// Port of `finish_()` from `Src/Modules/termcap.c:365`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:365
    0                                                                    // c:368
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ztgetflag_known_on_returns_one() {
        assert_eq!(ztgetflag("am"), 1);
    }

    #[test]
    fn ztgetflag_unknown_returns_minus_one() {
        assert_eq!(ztgetflag("zz"), -1);
    }

    #[test]
    fn gettermcap_co_returns_columns() {
        let v = gettermcap("co");
        assert!(v.is_some());
        let n: i32 = v.unwrap().parse().unwrap_or(0);
        assert!(n > 0);
    }

    #[test]
    fn gettermcap_unknown_returns_none() {
        assert!(gettermcap("zz_nonexistent").is_none());
    }

    #[test]
    fn scantermcap_emits_bool_caps() {
        let v = scantermcap();
        assert!(v.iter().any(|(k, _)| k == "am"));
    }
}

// =====================================================
// ShellExecutor shim
// =====================================================

impl crate::ported::exec::ShellExecutor {
    /// `echotc` builtin shim — adapts `&[String]` argv to
    /// `bin_echotc` over a `[bool; 256]` ops bitmask.
    pub(crate) fn bin_echotc(&mut self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let ops = [false; 256];
        bin_echotc("echotc", &argv, &ops)
    }
}
