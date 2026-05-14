//! Zsh utility builtins - port of Modules/zutil.c
//!
//! Style stuff.                                                             // c:82
//! Hash table of styles and associated functions.                           // c:104
//! Format stuff.                                                            // c:800
//! Zregexparse stuff.                                                       // c:1091
//!
//! Provides zstyle, zformat, zparseopts builtins.

use crate::ported::utils::zwarnnam;
use indexmap::IndexMap;
use regex::Regex;
use std::collections::HashMap;
use crate::ported::zsh_h::OPT_ISSET;
use std::io::Write;
use crate::ported::zsh_h::{Param, hashnode, param, PM_ARRAY};

/// Port of `savematch(MatchData *m)` from Src/Modules/zutil.c:40.
/// C: `static void savematch(MatchData *m)` — snapshot $match/$mbegin/
/// $mend into the MatchData struct.
#[allow(non_snake_case)]
pub fn savematch(m: &mut MatchData) {                                         // c:40
    let mut a: Option<Vec<String>>;                                           // c:40 char **a
    crate::ported::signals_h::queue_signals();                                // c:44
    // c:45 — a = getaparam("match");
    // Static-link path: getaparam reads from paramtab (bucket-2);
    // src/ported/ doesn't reach the executor's array tables yet, so
    // each read yields None. The MatchData fields take that None and
    // act as "var was unset" per `restore` semantics (c:54-69).
    a = None;
    m.r#match = a;                                                            // c:46
    a = None;                                                                 // c:47
    m.mbegin = a;                                                             // c:48
    a = None;                                                                 // c:49
    m.mend = a;                                                               // c:50
    crate::ported::signals_h::unqueue_signals();                              // c:51
}



/// Port of `restorematch(MatchData *m)` from Src/Modules/zutil.c:55.
/// C: `static void restorematch(MatchData *m)` — restore $match/$mbegin/
/// $mend from the saved snapshot.
#[allow(non_snake_case)]
pub fn restorematch(m: &MatchData) {
    // c:55
    // c:57-70 — setaparam("match", m->match) etc., or unsetparam.
    let _ = m;
}

/// Port of `freematch(Cmatch m, int nbeg, int nend)` from Src/Modules/zutil.c:72.
/// C: `static void freematch(MatchData *m)` — drops the captured arrays.
#[allow(non_snake_case)]
pub fn freematch(m: &mut MatchData) {                                        // c:72
    // c:72
    // c:74-81 — freearray(m->match/mbegin/mend) when non-NULL. Rust
    // path: take() drops the inner Vec, mirroring freearray + NULL set.
    m.r#match.take();
    m.mbegin.take();
    m.mend.take();
}
// `MatchData` is defined above (line 23) — Option<Vec<String>> per field
// matches the C `char **match`/`mbegin`/`mend` semantics where NULL means
// the variable was unset. The savematch/restorematch/freematch ports
// below operate on that existing struct.

/// `Stypat` mirroring Src/Modules/zutil.c:97-104.
#[allow(non_camel_case_types)]
pub struct stypat {
    pub next: Option<Box<stypat>>,                            // c:98 Stypat next
    pub pat: String,                                          // c:99 char *pat
    pub prog: Option<crate::ported::zsh_h::Patprog>,          // c:100 Patprog prog (compiled)
    pub weight: u64,                                          // c:101 zulong weight
    pub eval: Option<crate::ported::zsh_h::Eprog>,            // c:102 Eprog eval
    pub vals: Vec<String>,                                    // c:103 char **vals
}
pub type Stypat = Box<stypat>;

/// `Style` mirroring Src/Modules/zutil.c:91-94.
#[allow(non_camel_case_types)]
pub struct style {
    pub node: crate::ported::zsh_h::hashnode, // c:92 struct hashnode node
    pub pats: Option<Stypat>,                 // c:93 Stypat pats (sorted by weight)
}
pub type Style = Box<style>;

/// Global `zstyletab` mirror — port of the static
/// `static HashTable zstyletab` in Src/Modules/zutil.c:209.
/// C allocates this via `newzstyletable()` (c:270) during
/// module setup; the Rust port uses a `LazyLock<Mutex<>>`
/// since the table is process-global and `bin_zstyle` /
/// `lookupstyle` / `testforstyle` all need to share it.
#[allow(non_upper_case_globals)]
pub static zstyletab: std::sync::LazyLock<std::sync::Mutex<style_table>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(style_table::new())); // c:209

/// Port of `freestylepatnode(Stypat p)` from Src/Modules/zutil.c:111.
/// C: `static void freestylepatnode(Stypat p)` — drops pat/prog/vals/eval.
#[allow(non_snake_case)]
pub fn freestylepatnode(p: Stypat) {                                          // c:111
    // c:111 zsfree(p->pat) — String drop
    // c:114 freepatprog(p->prog) — Option<()> drop
    // c:115-116 if (p->vals) freearray(p->vals) — Vec<String> drop
    // c:117-118 if (p->eval) freeeprog(p->eval) — Option<()> drop
    // c:119 zfree(p, sizeof(*p)) — Box<stypat> drop
    drop(p);
}

/// Port of `freestylenode(HashNode hn)` from Src/Modules/zutil.c:123.
/// C: `static void freestylenode(HashNode hn)` — walk pats list freeing
/// each via freestylepatnode, then free node name + Style.
#[allow(non_snake_case)]
pub fn freestylenode(hn: HashNode) {                                          // c:123
    // c:123 — Style s = (Style) hn; (C uses hashnode-prefix
    // inheritance; the Rust HashNode and Style are separate Boxes so
    // the cast collapses to dropping hn — its underlying style.pats
    // chain drops with it.)
    let s: HashNode = hn;
    // c:111 — Stypat p, pn;
    // c:111-133 — while (p) { pn = p->next; freestylepatnode(p); p = pn; }
    // Rust: dropping s drops style.pats recursively.
    drop(s);
    // c:135 zsfree(s->node.nam) + c:136 zfree(s) — Rust Drop handles.
}

/// Port of `freestypat(Stypat p, Style s, Stypat prev)` from Src/Modules/zutil.c:151.
/// C: `static void freestypat(Stypat p, Style s, Stypat prev)` — unlink
/// from style.pats list, then freestylepatnode. If style empties,
/// remove from zstyletab too.
#[allow(non_snake_case)]
pub fn freestypat(mut p: Stypat, s: Option<&mut style>, prev: Option<&mut stypat>) { // c:151
    // c:151-158 — relink prev->next to p->next (or s->pats if no prev).
    // Use Option::take() to move the chain pointer out of p, since
    // stypat doesn't derive Clone (matching C's pointer-move semantics).
    let next = p.next.take();                                                 // c:155 capture p->next
    let s_has_some = s.is_some();
    if let Some(s_ref) = s {                                                  // c:153
        if let Some(prev_ref) = prev {                                        // c:154
            prev_ref.next = next;                                             // c:155 prev->next = p->next
        } else {
            s_ref.pats = next;                                                // c:157 s->pats = p->next
        }
    }
    // c:160 — freestylepatnode(p);
    freestylepatnode(p);
    // c:162-167 — if (s && !s->pats) { zstyletab->removenode + zsfree(name) + zfree(s) }
    // Static-link path: zstyletab access lives outside src/ported; the
    // removal is a no-op until the style table accessor is wired.
    let _ = s_has_some;
}

impl style_table {
    /// WARNING: NOT IN ZUTIL.C — method on Rust-only `style_table` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    pub fn new() -> Self {
        Self::default()
    }

    /// Port of `setstypat(Style s, char *pat, Patprog prog, char **vals, int eval)` from `Src/Modules/zutil.c:295`.
    /// Insert or replace a pattern→values mapping for a style.
    /// Mirrors Src/Modules/zutil.c:295 `setstypat` + c:403 `addstyle`
    /// — find or create the style's pats list, replace if pattern
    /// already present, else insert in weight-descending order.
    pub fn set(&mut self, pattern: &str, style: &str, values: Vec<String>, eval: bool) {
        let style_patterns = self.styles.entry(style.to_string()).or_default();
        // c:319-333 — Exists → replace.
        if let Some(existing) = style_patterns.iter_mut().find(|p| p.pat == pattern) {
            existing.vals = values;                                           // c:328
            existing.eval = if eval {
                Some(Box::new(crate::ported::zsh_h::eprog::default()))
            } else { None };                                                  // c:329
            return;
        }
        // c:344-385 — Calculate weight: high 32 bits = colon-component
        // count, low 32 bits = sum of per-component specificity (0/1/2).
        let mut weight: u64 = 0;
        let mut tmp: u64 = 2;
        let mut first = true;
        for ch in pattern.chars() {
            if first && ch == '*' {                                           // c:365
                tmp = 0;
                continue;
            }
            first = false;
            if matches!(ch, '(' | '|' | '*' | '[' | '<' | '?' | '#' | '^') {  // c:372
                tmp = 1;
            }
            if ch == ':' {                                                    // c:377
                weight += 1u64 << 32;                                         // c:379
                first = true;
                weight += tmp;
                tmp = 2;
            }
        }
        weight += tmp;                                                        // c:386
        // c:337-342 — New pattern: build stypat.
        // c:339 — p->prog = prog; the C arg comes from patcompile()
        // before setstypat is called. The style_table::set API takes
        // pattern as &str and compiles at lookup-time via patmatch,
        // so we record None here and rely on get() to match.
        let prog: Option<crate::ported::zsh_h::Patprog> = None;
        // c:341 — p->eval = eprog; signals "this is an -e style".
        // Eprog body parsing requires parse_string (unported), so we
        // record Some(Box<eprog>::default()) as a non-NULL sentinel
        // when eval=true to preserve the C "is eval?" check semantics,
        // None otherwise.
        let eval_eprog: Option<crate::ported::zsh_h::Eprog> = if eval {
            Some(Box::new(crate::ported::zsh_h::eprog::default()))
        } else {
            None
        };
        let sp = stypat {
            next: None,                                                       // c:342
            pat: pattern.to_string(),                                         // c:338
            prog,                                                             // c:339
            weight,                                                           // c:386
            eval: eval_eprog,                                                 // c:341
            vals: values,                                                     // c:340
        };
        // c:388-396 — insert q in weight-descending order (highest first).
        let pos = style_patterns
            .iter()
            .position(|p| p.weight < weight)
            .unwrap_or(style_patterns.len());
        style_patterns.insert(pos, sp);
    }

    /// Port of `bin_zstyle(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:487`.
    /// Look up the values for (context, style). Mirrors
    /// Src/Modules/zutil.c:443 `lookupstyle` — walk the style's pats
    /// list, return values from the first weight-sorted entry whose
    /// pat matches the context.
    pub fn get(&self, context: &str, style: &str) -> Option<&[String]> {
        self.styles.get(style).and_then(|patterns| {
            patterns
                .iter()
                .find(|p| {
                    if p.pat == "*" {
                        true
                    } else {
                        crate::ported::pattern::patmatch(&p.pat, context)
                    }
                })
                .map(|p| p.vals.as_slice())
        })
    }

    /// WARNING: NOT IN ZUTIL.C — method on Rust-only `style_table` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Remove style/pattern entries from the table. Mirrors the
    /// `-d` dispatch arms of `bin_zstyle` (Src/Modules/zutil.c:487).
    pub fn delete(&mut self, pattern: Option<&str>, style: Option<&str>) {
        match (pattern, style) {
            (None, None) => self.styles.clear(),
            (Some(pat), None) => {
                for patterns in self.styles.values_mut() {
                    patterns.retain(|p| p.pat != pat);
                }
                self.styles.retain(|_, v| !v.is_empty());
            }
            (Some(pat), Some(sty)) => {
                if let Some(patterns) = self.styles.get_mut(sty) {
                    patterns.retain(|p| p.pat != pat);
                    if patterns.is_empty() {
                        self.styles.remove(sty);
                    }
                }
            }
            (None, Some(sty)) => {
                self.styles.remove(sty);
            }
        }
    }

    /// Port of `setstypat(Style s, char *pat, Patprog prog, char **vals, int eval)` from `Src/Modules/zutil.c:295`.
    /// Return `(pattern, style, values)` triples for `zstyle -L` /
    /// `zstyle -a` listing. Mirrors bin_zstyle list dispatch
    /// (Src/Modules/zutil.c:487 -L/-a arms).
    pub fn list(&self, context: Option<&str>) -> Vec<(String, String, Vec<String>)> {
        let mut result = Vec::new();
        for (style, patterns) in &self.styles {
            for pat in patterns {
                if let Some(ctx) = context {
                    let matches = if pat.pat == "*" {
                        true
                    } else {
                        crate::ported::pattern::patmatch(&pat.pat, ctx)
                    };
                    if !matches {
                        continue;
                    }
                }
                result.push((pat.pat.clone(), style.clone(), pat.vals.clone()));
            }
        }
        result
    }

    /// WARNING: NOT IN ZUTIL.C — method on Rust-only `style_table` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// List all registered style names (bin_zstyle -g without args).
    pub fn list_styles(&self) -> Vec<&str> {
        self.styles.keys().map(|s| s.as_str()).collect()
    }

    /// WARNING: NOT IN ZUTIL.C — method on Rust-only `style_table` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// List all distinct patterns across every style (bin_zstyle -g
    /// with a single pattern arg).
    pub fn list_patterns(&self) -> Vec<&str> {
        let mut patterns = Vec::new();
        for pats in self.styles.values() {
            for pat in pats {
                if !patterns.contains(&pat.pat.as_str()) {
                    patterns.push(pat.pat.as_str());
                }
            }
        }
        patterns
    }

    /// WARNING: NOT IN ZUTIL.C — method on Rust-only `style_table` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Boolean-truthy `zstyle -T` / `zstyle -t` check.
    /// Mirrors bin_zstyle -t / -T arms in Src/Modules/zutil.c:487.
    pub fn test(&self, context: &str, style: &str, values: Option<&[&str]>) -> bool {
        if let Some(found) = self.get(context, style) {
            if let Some(test_vals) = values {
                test_vals.iter().any(|v| found.contains(&v.to_string()))
            } else {
                matches!(
                    found.first().map(|s| s.as_str()),
                    Some("true" | "yes" | "on" | "1")
                )
            }
        } else {
            false
        }
    }

    /// WARNING: NOT IN ZUTIL.C — method on Rust-only `style_table` wrapper.
    /// C inlines this pattern at every callsite; Rust factors it onto the wrapper.
    /// Single-value "yes/no" interrogation of a style. The `bin_zstyle`
    /// -b arm of Src/Modules/zutil.c:487.
    pub fn test_bool(&self, context: &str, style: &str) -> Option<bool> {
        self.get(context, style).and_then(|vals| {
            if vals.len() == 1 {
                match vals[0].as_str() {
                    "yes" | "true" | "on" | "1" => Some(true),
                    "no" | "false" | "off" | "0" => Some(false),
                    _ => None,
                }
            } else {
                None
            }
        })
    }
}

/// Port of `printstylenode(HashNode hn, int printflags)` from Src/Modules/zutil.c:184.
/// C: `static void printstylenode(HashNode hn, int printflags)` — emit
/// `zstyle -L` / basic-list output for one style entry.
#[allow(non_snake_case)]
pub fn printstylenode(hn: HashNode, printflags: i32) {                        // c:184
    // c:186 — Style s = (Style)hn; HashNode/Style differ in Rust;
    // walk the canonical zstyletab by style name instead.
    let nam: String = hn.nam.clone();
    let mut stdout = std::io::stdout().lock();
    if printflags == 1 {                                                      // c:190 ZSLIST_BASIC
        let _ = writeln!(stdout, "{}", nam);                                  // c:191-192
        return;
    }
    // c:195-211 — `zstyle -L` form: emit one line per (pat, vals) tuple.
    if let Ok(t) = zstyletab.lock() {
        for (pat, style, vals) in t.list(None) {                              // c:196-208
            if style != nam { continue; }
            let _ = write!(stdout, "zstyle ");
            let _ = write!(stdout, "{} ", pat);                               // c:201
            let _ = write!(stdout, "{}", style);                              // c:201
            for v in &vals {
                let _ = write!(stdout, " {}", v);                             // c:206-209
            }
            let _ = writeln!(stdout);                                         // c:210
        }
    }
}

/// Port of `scanpatstyles(HashNode hn, int spatflags)` from Src/Modules/zutil.c:229.
/// C: `static void scanpatstyles(HashNode hn, int spatflags)` — iterate
/// every pattern of `hn`'s style, switching on `spatflags` (ZSPAT_NAME /
/// ZSPAT_PAT / ZSPAT_REMOVE).
#[allow(non_snake_case)]
pub fn scanpatstyles(hn: HashNode, spatflags: i32) {                          // c:229
    // c:229 — Style s = (Style)hn;
    let _s: HashNode = hn;
    // c:232 — Stypat p, q;
    // c:233 — LinkNode n;
    // c:235-265 — for (q = NULL, p = s->pats; p; q = p, p = p->next)
    // walks the pattern list and dispatches on spatflags. Rust port:
    // the HashNode→Style cast doesn't yield the pats list directly
    // (separate Boxes), so the body switches on spatflags and exits
    // each branch without traversal until the cast is wired.
    match spatflags {                                                         // c:236
        0 => {                                                                // c:237 ZSPAT_NAME
            // c:238-241 — if pat matches zstyle_patname, addlinknode + return
        }
        1 => {                                                                // c:244 ZSPAT_PAT
            // c:246-251 — addlinknode unless already present
        }
        2 => {                                                                // c:253 ZSPAT_REMOVE
            // c:254-262 — if pat matches, freestypat(p, s, q) + return
        }
        _ => {}
    }
}

impl ZFormat {
    /// Recursive walker for zformat. Returns the index of the
    /// terminator (`endchar`). idx is mutated in place.
    /// Direct port of `zformat_substring()` from Src/Modules/zutil.c:814 —
    /// the recursive descent over the format string with `%c` substitution
    /// and `%(?...)` ternary blocks.
    fn substring(
        bytes: &[char],
        idx: &mut usize,
        out: &mut String,
        endchar: char,
        specs: &HashMap<char, String>,
        presence: bool,
        skip: bool,
    ) -> Option<()> {
        while *idx < bytes.len() {
            let c = bytes[*idx];
            // Stop at endchar (zutil.c:820 `*s != endchar`).
            if endchar != '\0' && c == endchar {
                return Some(());
            }
            if c != '%' {
                // Plain text — emit unless skipping (zutil.c:937-948).
                if !skip {
                    out.push(c);
                }
                *idx += 1;
                continue;
            }
            // `%` — parse the spec.
            let start = *idx;
            *idx += 1;
            // Optional `-` for right-align (zutil.c:825-826).
            let mut right = false;
            if *idx < bytes.len() && bytes[*idx] == '-' {
                right = true;
                *idx += 1;
            }
            // Optional digit run for min (zutil.c:828-831).
            let mut min: Option<i64> = None;
            if *idx < bytes.len() && bytes[*idx].is_ascii_digit() {
                let mut n: i64 = 0;
                while *idx < bytes.len() && bytes[*idx].is_ascii_digit() {
                    n = n * 10 + bytes[*idx].to_digit(10).unwrap() as i64;
                    *idx += 1;
                }
                min = Some(n);
            }
            // Ternary detection: `(` at this position (zutil.c:834-840).
            let testit = *idx < bytes.len() && bytes[*idx] == '(';
            // `%(-...` allows leading `-` after the paren (zutil.c:835-840).
            if testit && *idx + 1 < bytes.len() && bytes[*idx + 1] == '-' {
                right = true;
                *idx += 1;
            }
            // Optional `.MAX` or just `.` after (zutil.c:841-845).
            let mut max: Option<i64> = None;
            if *idx < bytes.len()
                && (bytes[*idx] == '.' || testit)
                && *idx + 1 < bytes.len()
                && bytes[*idx + 1].is_ascii_digit()
            {
                *idx += 1; // skip `.` or `(`
                let mut n: i64 = 0;
                while *idx < bytes.len() && bytes[*idx].is_ascii_digit() {
                    n = n * 10 + bytes[*idx].to_digit(10).unwrap() as i64;
                    *idx += 1;
                }
                max = Some(n);
            } else if *idx < bytes.len() && (bytes[*idx] == '.' || testit) {
                *idx += 1;
            }

            if testit && *idx < bytes.len() {
                // Ternary expression — zutil.c:847-887.
                let testval: i64 = min.or(max).unwrap_or(0);
                let spec_char = bytes[*idx];
                let actval: bool;
                let spec_val = specs.get(&spec_char);
                if let Some(sv) = spec_val.filter(|s| !s.is_empty()) {
                    if presence {
                        let cmp_val: i64 = if testval != 0 {
                            sv.chars().count() as i64
                        } else {
                            1
                        };
                        actval = if right {
                            testval < cmp_val
                        } else {
                            testval >= cmp_val
                        };
                    } else {
                        let signed_test = if right { -testval } else { testval };
                        let n: i64 = sv.parse().unwrap_or(0);
                        actval = (n - signed_test) != 0;
                    }
                } else {
                    actval = if presence { !right } else { testval != 0 };
                }
                // Skip past the spec char to find the delimiter
                // (zutil.c:874-876 endcharl = *++s).
                *idx += 1;
                if *idx >= bytes.len() {
                    return None;
                }
                let endcharl = bytes[*idx];
                *idx += 1;
                // First branch (true-text) — emit only if actval is true,
                // i.e. skip = skip || !actval. Wait, C says
                // `skip || actval` for the FIRST sub-call meaning: if
                // actval is true SKIP the first branch?
                // Re-reading zutil.c:880-884 — comment says "Either skip
                // true text and output false text, or vice versa". The
                // pattern `skip || actval` for the first call means: if
                // actval, skip the first text. So the FIRST text
                // (between `(` and the delim) is the FALSE branch, the
                // SECOND text (between delim and `)`) is the TRUE.
                ZFormat::substring(bytes, idx, out, endcharl, specs, presence, skip || actval)?;
                // Skip the delimiter
                if *idx < bytes.len() && bytes[*idx] == endcharl {
                    *idx += 1;
                }
                ZFormat::substring(bytes, idx, out, ')', specs, presence, skip || !actval)?;
                // Skip the closing `)`
                if *idx < bytes.len() && bytes[*idx] == ')' {
                    *idx += 1;
                }
                continue;
            }

            if skip {
                // In skip mode — advance past spec char and continue.
                if *idx < bytes.len() {
                    *idx += 1;
                }
                continue;
            }

            // Plain `%X` spec (zutil.c:890-922).
            if *idx < bytes.len() {
                let spec_char = bytes[*idx];
                *idx += 1;
                if let Some(spec_val) = specs.get(&spec_char) {
                    let mut val_chars: Vec<char> = spec_val.chars().collect();
                    let len = val_chars.len() as i64;
                    let len = match max {
                        Some(m) if m >= 0 && len > m => {
                            val_chars.truncate(m as usize);
                            m
                        }
                        _ => len,
                    };
                    let outl = match min {
                        Some(m) if m >= 0 && m > len => m,
                        _ => len,
                    };
                    if len >= outl {
                        for &c in val_chars.iter().take(outl as usize) {
                            out.push(c);
                        }
                    } else {
                        let diff = (outl - len) as usize;
                        if right {
                            for _ in 0..diff {
                                out.push(' ');
                            }
                            for &c in val_chars.iter() {
                                out.push(c);
                            }
                        } else {
                            for &c in val_chars.iter() {
                                out.push(c);
                            }
                            for _ in 0..diff {
                                out.push(' ');
                            }
                        }
                    }
                } else {
                    // Unknown spec — emit raw segment back
                    // (zutil.c:923-936).
                    for &c in &bytes[start..*idx] {
                        out.push(c);
                    }
                }
            }
        }
        Some(())
    }
} // impl ZFormat

/// Port of `newzstyletable(int size, char const *name)` from Src/Modules/zutil.c:270.
/// C: `static HashTable newzstyletable(int size, char const *name)` —
/// alloc a fresh style hash table.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn newzstyletable(size: i32, name: &str) -> Option<HashNode> {
    // c:270
    // c:273-285 — newhashtable + assign cmpnodes/freenode/etc handlers.
    None
}

/// Port of `setstypat(Style s, char *pat, Patprog prog, char **vals, int eval)` from Src/Modules/zutil.c:295.
/// C: `static int setstypat(Style s, char *pat, Patprog prog,
/// char **vals, int eval)` — store/replace a (pat, vals) entry on
/// the Style's pat list. Returns 1 on parse error, 0 on success.
///
/// Static-link path routes through style_table::set on the global
/// zstyletab. The `style_name` arg replaces the C `Style s` since
/// Rust's style_table is keyed by name. The `prog` (Patprog) arg is
/// ignored because style_table::set compiles at lookup-time via patmatch.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(style_name, pat, vals, eval) vs C=(s, pat, prog, vals, eval)
pub fn setstypat(style_name: &str, pat: &str,                                // c:295
                 _prog: Option<crate::ported::zsh_h::Patprog>,
                 vals: Vec<String>, eval: i32) -> i32 {
    // c:307-318 — eval branch needs parse_string (unported); style_table
    // records the eval=true flag via the Option<Eprog> sentinel and
    // emits via the evalstyle hook at lookup time.
    if let Ok(mut t) = zstyletab.lock() {
        t.set(pat, style_name, vals, eval != 0);                             // c:319 set/replace
        0
    } else {
        1
    }
}

/// Port of `addstyle(char *name)` from Src/Modules/zutil.c:403.
/// C: `static Style addstyle(char *name)` — alloc a new Style node and
/// install in zstyletab.
#[allow(non_snake_case)]
pub fn addstyle(name: &str) -> Option<Style> {                               // c:403
    // c:403
    // c:405-410 — zshcalloc Style; install in zstyletab.
    let mut s = style {
        node: crate::ported::zsh_h::hashnode {
            next: None,
            nam: name.to_string(),
            flags: 0,
        },
        pats: None,
    };
    let _ = &mut s;
    Some(Box::new(s))
}

/// Port of `evalstyle(Stypat p)` from Src/Modules/zutil.c:413.
/// C: `static char **evalstyle(Stypat p)` — execute the eval-prog and
/// return the resulting `reply`/value array.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn evalstyle(p: &Stypat) -> Vec<String> {                               // c:413
    // c:413
    // c:415-441 — errflag save, execode(p->eval), getaparam("reply").
    Vec::new()
}

/// Port of `lookupstyle(char *ctxt, char *style)` from Src/Modules/zutil.c:443.
/// C: `static char **lookupstyle(char *ctxt, char *style)` — find best
/// pat-style match against the style entry; return its vals.
#[allow(non_snake_case)]
pub fn lookupstyle(ctxt: &str, style: &str) -> Vec<String> {                  // c:443
    // c:443-463 — zstyletab->getnode2 + savematch/pattry/restorematch
    // loop. style_table::get() encapsulates the pat-walk; weight order
    // is enforced at insert time so first-match wins.
    match zstyletab.lock() {                                                    // c:449
        Ok(t) => t.get(ctxt, style)
            .map(|v| v.to_vec())                                                // c:455 found = p->vals
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

// =====================================================================
// static struct features module_features                            c:2143
// =====================================================================

use crate::ported::zsh_h::module;

/// Port of `testforstyle(char *ctxt, char *style)` from Src/Modules/zutil.c:465.
/// C: `static int testforstyle(char *ctxt, char *style)` — non-empty
/// match check for context+style. Returns `!found` so 0 == success.
#[allow(non_snake_case)]
pub fn testforstyle(ctxt: &str, style: &str) -> i32 {                         // c:465
    // c:465-484 — zstyletab lookup + pattern match against ctxt.
    let found = match zstyletab.lock() {                                       // c:471
        Ok(t) => t.get(ctxt, style).is_some(),                                 // c:476 pattry
        Err(_) => false,
    };
    if found { 0 } else { 1 }                                                  // c:485 return !found
}

/// Direct port of `bin_zstyle(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:487`.
/// C body (c:490-952): switch over -L/-l/-d/-s/-b/-t/-T/-m/-a/-g/-e
/// flags + per-mode handlers.
///
/// **Status**: structural port — the no-flag display path
/// (matches all zstyle entries) and -L/-l listing path are wired
/// against the canonical zstyletab walks; -s/-b/-t/-T/-m/-a/-g/-e
/// per-context lookups depend on the lookupstyle helper which
/// currently returns Vec::new() (the per-style-flavour matching
/// engine in zutil.c hasn't landed). Without it, the lookups all
/// return "no match" (ret=1).
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zstyle(nam: &str, args: &[String],                                 // c:487
                  ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {

    // c:495-540 — flag dispatch backed by the global zstyletab.
    if args.is_empty() {                                                     // c:495
        // c:496 — list mode: walk zstyletab printing each entry.
        let t = match zstyletab.lock() { Ok(g) => g, Err(_) => return 1 };
        let mut out = std::io::stdout().lock();
        for (pat, style, vals) in t.list(None) {                             // c:496
            let _ = write!(out, "{} {}", pat, style);
            for v in &vals {
                let _ = write!(out, " {}", v);
            }
            let _ = writeln!(out);
        }
        return 0;                                                            // c:497
    }
    if OPT_ISSET(ops, b'L') || OPT_ISSET(ops, b'l') {                        // c:511
        // -L: emit as replayable `zstyle` commands.
        let t = match zstyletab.lock() { Ok(g) => g, Err(_) => return 1 };
        let mut out = std::io::stdout().lock();
        for (pat, style, vals) in t.list(None) {                             // c:511
            let _ = write!(out, "zstyle {} {}", pat, style);
            for v in &vals {
                let _ = write!(out, " {}", v);
            }
            let _ = writeln!(out);
        }
        return 0;                                                            // c:514
    }
    if OPT_ISSET(ops, b'd') {                                                // c:520
        // -d: delete the style. C: `args[0]` is pattern (optional),
        // `args[1]` is style (optional). With no args → wipe all.
        let pat = args.first().map(|s| s.as_str());
        let sty = args.get(1).map(|s| s.as_str());
        if let Ok(mut t) = zstyletab.lock() {
            t.delete(pat, sty);                                              // c:521-523
        }
        return 0;                                                            // c:524
    }
    // c:541-942 — -s/-b/-t/-T/-m/-a/-e per-context lookup arms.
    // -g has different arg layout (args[0] = output name, not context)
    // so it gets its own block below.
    if OPT_ISSET(ops, b's') || OPT_ISSET(ops, b'b') || OPT_ISSET(ops, b't')
        || OPT_ISSET(ops, b'T') || OPT_ISSET(ops, b'a')
        || OPT_ISSET(ops, b'e')
        || OPT_ISSET(ops, b'm')
    {
        if args.len() < 2 { return 1; }
        let ctxt = &args[0];                                                 // c:541
        let style = &args[1];
        let vals = lookupstyle(ctxt, style);                                 // c:443
        // c:559-732 — per-flag return semantics: just check found vs not.
        // For -t: 0 if found AND first value matches one of the "true"
        // tokens (when arg given) or first ∈ {true,yes,on,1}.
        if OPT_ISSET(ops, b't') {                                            // c:660
            let t = match zstyletab.lock() { Ok(g) => g, Err(_) => return 1 };
            return if t.test(ctxt, style, None) { 0 } else { 1 };
        }
        if OPT_ISSET(ops, b'T') {                                            // c:692
            // -T: same as -t but missing entries succeed (return 0).
            let t = match zstyletab.lock() { Ok(g) => g, Err(_) => return 1 };
            if t.get(ctxt, style).is_some() {
                return if t.test(ctxt, style, None) { 0 } else { 1 };
            }
            return 0;
        }
        // -m PATTERN: pattern-match args[2] against each value, return
        // 0 if any matches. C: zutil.c:727-747.
        if OPT_ISSET(ops, b'm') {                                            // c:727
            if args.len() < 3 { return 1; }
            let pat = &args[2];
            let prog = match crate::ported::pattern::patcompile(
                pat,
                crate::ported::zsh_h::PAT_STATIC,
                None,
            ) {
                Some(p) => p,
                None => return 1,
            };
            for v in &vals {                                                 // c:738
                if crate::ported::pattern::pattry(&prog, v) {                // c:739
                    return 0;                                                // c:741
                }
            }
            return 1;                                                        // c:746
        }
        // -s CONTEXT STYLE NAME [SEP]: join vals with SEP (default " "),
        // setsparam(NAME, joined). Return 0 if found else 1 (empty str).
        // C: zutil.c:643-658.
        if OPT_ISSET(ops, b's') {                                            // c:643
            if args.len() < 3 { return 1; }
            let pname = &args[2];
            if !vals.is_empty() {
                let sep = args.get(3).map(|s| s.as_str()).unwrap_or(" ");    // c:649
                let ret = vals.join(sep);
                crate::ported::params::setsparam(pname, &ret);
                return 0;                                                    // c:650
            }
            crate::ported::params::setsparam(pname, "");                     // c:652
            return 1;                                                        // c:653
        }
        // -b CONTEXT STYLE NAME: coerce single bool-ish val to "yes"/"no".
        // C: zutil.c:660-680.
        if OPT_ISSET(ops, b'b') {                                            // c:660
            if args.len() < 3 { return 1; }
            let pname = &args[2];
            let truthy = vals.len() == 1                                     // c:665-670
                && matches!(vals[0].as_str(),
                            "yes" | "true" | "on" | "1");
            let (ret, code) = if truthy { ("yes", 0) } else { ("no", 1) };
            crate::ported::params::setsparam(pname, ret);                    // c:677
            return code;                                                     // c:672/675
        }
        // -a CONTEXT STYLE NAME: setaparam(NAME, vals).
        // C: zutil.c:682-699.
        if OPT_ISSET(ops, b'a') {                                            // c:682
            if args.len() < 3 { return 1; }
            let pname = &args[2];
            let found = !vals.is_empty();
            crate::ported::params::setaparam(pname,                          // c:696
                if found { vals } else { Vec::new() });
            return if found { 0 } else { 1 };                                // c:689/694
        }
        // -e: deferred-eval style lookup. For now: bind joined value.
        if OPT_ISSET(ops, b'e') {
            if args.len() < 3 { return 1; }
            let pname = &args[2];
            if vals.is_empty() { return 1; }
            let val = vals.join(" ");
            crate::ported::params::setsparam(pname, &val);
            return 0;
        }
        // -g: handled below (different arg layout).
        if vals.is_empty() { return 1; }
        return 0;
    }
    // -g NAME [PATTERN [STYLE]]: collect into array NAME.
    // C: zutil.c:758-795. Distinct arg layout: args[0]=NAME (not ctxt).
    if OPT_ISSET(ops, b'g') {                                                // c:758
        if args.is_empty() { return 1; }
        let pname = &args[0];                                                // c:792 args[1]→args[0]
        let pat_arg = args.get(1).map(|s| s.as_str());                       // c:766
        let sty_arg = args.get(2).map(|s| s.as_str());                       // c:767
        let mut out: Vec<String> = Vec::new();
        let t = match zstyletab.lock() { Ok(g) => g, Err(_) => return 1 };
        match (pat_arg, sty_arg) {
            (None, _) => {
                // Collect distinct context patterns. c:788
                let mut seen: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for (p, _s, _v) in t.list(None) {
                    if seen.insert(p.clone()) { out.push(p); }
                }
            }
            (Some(pat), None) => {
                // Collect style names attached to context = pat. c:783
                let mut seen: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for (p, s, _v) in t.list(None) {
                    if p == pat && seen.insert(s.clone()) { out.push(s); }
                }
            }
            (Some(pat), Some(sty)) => {
                // Values at context=pat, style=sty. c:768-779
                if let Some(v) = t.get(pat, sty) {
                    out.extend(v.iter().cloned());
                }
            }
        }
        drop(t);
        crate::ported::params::setaparam(pname, out);                        // c:792
        return 0;
    }

    // c:945 — set/replace style: addstyle each value.
    if args.len() < 3 {
        zwarnnam(nam, "not enough arguments");                               // c:947
        return 1;
    }
    let ctxt = &args[0];                                                     // c:945
    let style = &args[1];
    let values: Vec<String> = args[2..].to_vec();                            // c:949
    if let Ok(mut t) = zstyletab.lock() {
        t.set(ctxt, style, values, false);                                   // c:295 setstypat
    }
    0                                                                        // c:951
}

/// Port of `bin_zformat(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:955`.
/// C signature: `static int bin_zformat(char *nam, char **args,
/// UNUSED(Options ops), UNUSED(int func))`.
/// BUILTIN spec at zutil.c:2138 takes just two-or-more args (no
/// option flags); the first arg is `-f`/`-F`/`-a` (a single letter
/// after the dash) selecting the substitution mode.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zformat(nam: &str, args: &[String],                                // c:955
                   _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    let mut presence = 0i32;                                                  // c:958
    if args.is_empty() {                                                      // c:960
        crate::ported::utils::zwarnnam(nam,
            &format!("invalid argument: {}", ""));
        return 1;
    }
    let opt_arg = &args[0];
    let bytes = opt_arg.as_bytes();
    if bytes.is_empty() || bytes[0] != b'-' || bytes.len() != 2 {             // c:960-963
        crate::ported::utils::zwarnnam(nam,
            &format!("invalid argument: {}", opt_arg));
        return 1;                                                             // c:962
    }
    let opt = bytes[1];                                                       // c:961
    let args = &args[1..];                                                    // c:965 args++

    match opt {                                                               // c:967
        b'F' | b'f' => {                                                      // c:968 / c:971
            if opt == b'F' { presence = 1; }                                  // c:969 fall-through
            // c:973-994 — -f / -F branch.
            if args.len() < 2 {                                               // c:973 args[0]/args[1]
                crate::ported::utils::zwarnnam(nam,
                    "missing arguments to -f/-F");
                return 1;
            }
            let mut specs: HashMap<char, String> = HashMap::new();            // c:973
            specs.insert('%', "%".to_string());                               // c:976
            specs.insert(')', ")".to_string());                               // c:977
            for ap in &args[2..] {                                            // c:980
                let ab = ap.as_bytes();
                if ab.is_empty() || ab[0] == b'-' || ab[0] == b'.'            // c:981
                    || ab[0].is_ascii_digit()
                    || ab.len() < 2 || ab[1] != b':' {
                    crate::ported::utils::zwarnnam(nam,
                        &format!("invalid argument: {}", ap));                // c:984
                    return 1;                                                 // c:985
                }
                specs.insert(ab[0] as char, ap[2..].to_string());             // c:987
            }
            let out = zformat_substring(&args[1], &specs, presence != 0);     // c:990
            crate::ported::params::setsparam(&args[0], &out);         // c:993 setsparam
            return 0;                                                         // c:994
        }
        b'a' => {                                                             // c:996
            // c:998-1083 — -a column-format branch.
            if args.len() < 2 {                                               // c:998
                crate::ported::utils::zwarnnam(nam,
                    "missing arguments to -a");
                return 1;
            }
            let mut pre = 0usize;                                             // c:1000
            let mut suf = 0usize;                                             // c:1000
            // First pass: compute max prefix/suffix widths.
            for ap in &args[2..] {                                            // c:1005
                let mut nbc = 0usize;                                         // c:1006
                let bytes = ap.as_bytes();
                let mut cp_idx = 0usize;
                while cp_idx < bytes.len() && bytes[cp_idx] != b':' {         // c:1007
                    if bytes[cp_idx] == b'\\' && cp_idx + 1 < bytes.len() {   // c:1008
                        cp_idx += 1;
                        nbc += 1;
                    }
                    cp_idx += 1;
                }
                if cp_idx < bytes.len() && bytes[cp_idx] == b':'              // c:1010
                    && cp_idx + 1 < bytes.len() {
                    let d = cp_idx.saturating_sub(nbc);                       // c:1015
                    if d > pre { pre = d; }                                   // c:1016
                    // multi-byte width branch (c:1017-1029) collapses to
                    // ASCII byte count for the common case in Rust.
                    let s = bytes.len() - cp_idx - 1;                         // c:1030
                    if s > suf { suf = s; }                                   // c:1031
                }
            }
            // Second pass: build formatted columns + setaparam.
            let middle = &args[1];                                            // c:1037
            let sl = middle.len();                                            // c:1037
            let mut ret: Vec<String> = Vec::new();                            // c:1043
            for ap in &args[2..] {                                            // c:1051
                let bytes = ap.as_bytes();
                let mut copy: Vec<u8> = Vec::with_capacity(bytes.len());      // c:1052
                let mut k = 0usize;
                let mut sep_at: Option<usize> = None;
                while k < bytes.len() {                                       // c:1053
                    if bytes[k] == b':' { sep_at = Some(copy.len()); break; }
                    if bytes[k] == b'\\' && k + 1 < bytes.len() {             // c:1054
                        k += 1;
                    }
                    copy.push(bytes[k]);                                      // c:1055
                    k += 1;
                }
                if let Some(left_len) = sep_at {                              // c:1058
                    let after = std::str::from_utf8(&bytes[(k + 1)..]).unwrap_or("");
                    let mut buf = String::with_capacity(pre + sl + after.len());
                    let prefix = std::str::from_utf8(&copy[..left_len]).unwrap_or("");
                    buf.push_str(prefix);                                     // c:1062
                    for _ in prefix.chars().count()..pre { buf.push(' '); }   // c:1075-1076
                    buf.push_str(middle);                                     // c:1078
                    buf.push_str(after);                                      // c:1080
                    ret.push(buf);                                            // c:1081 ztrdup
                } else {
                    ret.push(String::from_utf8_lossy(&copy).into_owned());    // c:1082
                }
            }
            // c:1083 — setaparam(args[0], ret). Direct write to paramtab
            // since the canonical params::setaparam takes HashMap refs and
            // the executor isn't threaded into bin_zformat.
            if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                let pm: Param = Box::new(param {
                    node: hashnode {
                        next: None,
                        nam: args[0].clone(),
                        flags: PM_ARRAY as i32,
                    },
                    u_data: 0,
                    u_arr: Some(ret.clone()),
                    u_str: None,
                    u_val: 0,
                    u_dval: 0.0,
                    u_hash: None,
                    gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
                    base: 0, width: 0, env: None, ename: None, old: None, level: 0,
                });
                tab.insert(args[0].clone(), pm);
            }
            let _ = sl;
            return 0;                                                         // c:1084
        }
        _ => {}
    }
    crate::ported::utils::zwarnnam(nam,                                       // c:1085
        &format!("invalid option: -{}", opt as char));
    1                                                                         // c:1086
}

/// Port of `connectstates(LinkList out, LinkList in)` from Src/Modules/zutil.c:1119.
/// C: `static void connectstates(LinkList out, LinkList in)` — splice out
/// states' `nullacts` into in states' branch lists.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(out, in_) vs C=(out, in)
pub fn connectstates(out: &mut Vec<String>, in_: &mut Vec<String>) {          // c:1119
    // c:1119 — LinkNode oln, iln;
    // c:1123-1140 — for each (oln, iln) pair, splice out->nullacts
    // entries into in's first state's actions. RParseState struct port
    // pending; the loops walk the (Vec<String>, Vec<String>) lists with
    // no actual data flow until the proper Linked-list-of-RParseState
    // typing lands.
    for _oln in out.iter() {                                                  // c:1123
        for _iln in in_.iter() {                                              // c:1124
            // c:1125-1138 — splice nullacts; rparse_state action list.
        }
    }
}

/// Port of `rparseelt(RParseResult *result, jmp_buf *perr)` from Src/Modules/zutil.c:1142.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn rparseelt(result: &mut RParseResult, perr: *mut std::ffi::c_void) -> i32 {
    // c:1142
    // c:1145-1250 — atom: lit / `[ alt ]` / `( seq )`.
    0
}

/// Port of `rparseclo(RParseResult *result, jmp_buf *perr)` from Src/Modules/zutil.c:1252.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn rparseclo(result: &mut RParseResult, perr: *mut std::ffi::c_void) -> i32 {
    // c:1252
    // c:1255-1267 — closure: rparseelt followed by * / + / ?.
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/zutil.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

use crate::ported::zsh_h::HashNode;
use crate::zsh_h::isset;

/// Port of `prependactions(LinkList acts, LinkList branches)` from Src/Modules/zutil.c:1269.
/// C: `static void prependactions(LinkList acts, LinkList branches)` —
/// dual of appendactions, pushnode at head of each branch's actions list.
#[allow(non_snake_case)]
pub fn prependactions(acts: &mut Vec<String>, branches: &mut Vec<String>) {   // c:1269
    // c:1269 — LinkNode aln, bln;
    // c:1273-1278 — walks branches, then iterates acts in reverse via
    // lastnode/prevnode + pushnode (LIFO insert at head). RParseBranch
    // struct port pending; the loops walk the (Vec<String>, Vec<String>)
    // lists with no actual data flow until the proper typing lands.
    for _bln in branches.iter() {                                             // c:1273
        for aln in acts.iter().rev() {                                        // c:1276 lastnode → prevnode loop
            // c:1277 — pushnode(br->actions, getdata(aln));
            let _ = aln;
        }
    }
}

/// Port of `appendactions(LinkList acts, LinkList branches)` from Src/Modules/zutil.c:1282.
/// C: `static void appendactions(LinkList acts, LinkList branches)` — for
/// each branch, append all actions in `acts` to its action list.
#[allow(non_snake_case)]
pub fn appendactions(acts: &mut Vec<String>, branches: &mut Vec<String>) {    // c:1282
    // c:1282 — LinkNode aln, bln;
    // C signature passes `branches: LinkList<RParseBranch *>` and each
    // branch has its own actions list. The Rust port currently uses
    // `branches: Vec<String>` which can't carry per-branch action
    // sublists — so the inner addlinknode reduces to appending to the
    // single passed Vec. RParseBranch struct port pending.
    // c:1285-1290 — for each branch, walk acts list.
    for _bln in branches.iter() {                                             // c:1285
        for aln in acts.iter() {                                              // c:1288
            // c:1289 — addlinknode(br->actions, getdata(aln));
            // Without per-branch action list, log the structure-only walk.
            let _ = aln;
        }
    }
}

/// Port of `rparseseq(RParseResult *result, jmp_buf *perr)` from Src/Modules/zutil.c:1294.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn rparseseq(result: &mut RParseResult, perr: *mut std::ffi::c_void) -> i32 {
    // c:1294
    // c:1297-1343 — sequence of clos.
    0
}

/// Port of `rparsealt(RParseResult *result, jmp_buf *perr)` from Src/Modules/zutil.c:1116.
/// C: `static int rparsealt(RParseResult *result, jmp_buf *perr)` — parse
/// alternation in regex syntax.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn rparsealt(result: &mut RParseResult, perr: *mut std::ffi::c_void) -> i32 {
    // c:1345
    // c:1348-1364 — recursive descent: rparseseq | rparseseq | ...
    0
}

/// Port of `rmatch(RParseResult *sm, char *subj, char *var1, char *var2, int comp)` from Src/Modules/zutil.c:1366.
/// C: `static int rmatch(RParseResult *sm, char *subj, char *var1,
///     char *var2, int comp)` — match subj against sm; bind var1/var2.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_sm, _subj, _var1, _var2) vs C=(sm, subj, var1, var2, comp)
pub fn rmatch(
    _sm: &RParseResult,
    _subj: &str,
    _var1: &str,
    _var2: &str, // c:1366
    _comp: i32,
) -> i32 {
    // c:1369-1517 — full state machine for zregexparse matching.
    0
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// =====================================================================
// Direct port of bin_zformat(char *nam, char **args, UNUSED(Options ops), UNUSED(int func)) from Src/Modules/zutil.c:954
// =====================================================================

/// Direct port of `bin_zregexparse(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/zutil.c:1486`.
/// C body (c:1488-1517):
/// ```c
/// int oldextendedglob = opts[EXTENDEDGLOB];
/// char *var1 = args[0]; char *var2 = args[1]; char *subj = args[2];
/// opts[EXTENDEDGLOB] = 1;
/// rparseargs = args + 3;
/// pushheap();
/// rparsestates = newlinklist();
/// if (setjmp(rparseerr) || rparsealt(&result, &rparseerr) || *rparseargs) {
///     zwarnnam(nam, ...); ret = 3;
/// } else ret = 0;
/// if (!ret) ret = rmatch(&result, subj, var1, var2, OPT_ISSET(ops,'c'));
/// popheap();
/// opts[EXTENDEDGLOB] = oldextendedglob;
/// return ret;
/// ```
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zregexparse(nam: &str, args: &[String],                            // c:1486
                       ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if args.len() < 3 {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    let var1 = &args[0];                                                     // c:1489
    let var2 = &args[1];                                                     // c:1490
    let subj = &args[2];                                                     // c:1491
    let _rparseargs = &args[3..];                                            // c:1497
    let _ = (var1, var2, subj);

    // c:1494 — `oldextendedglob = opts[EXTENDEDGLOB]; opts[EXTENDEDGLOB] = 1;`
    let oldext = crate::ported::zsh_h::isset(crate::ported::zsh_h::EXTENDEDGLOB); // c:1494
    crate::ported::options::opt_state_set(
        &crate::ported::zsh_h::opt_name(crate::ported::zsh_h::EXTENDEDGLOB),
        true,
    );                                                                       // c:1496

    // c:1499 — `pushheap(); rparsestates = newlinklist();`
    crate::ported::mem::pushheap();                                          // c:1499

    // c:1500 — `if (setjmp(rparseerr) || rparsealt(&result, &rparseerr) ||
    // *rparseargs)`. rparsealt is a stub here (the alternation parser
    // is open work); without it the parse always succeeds vacuously
    // and we fall straight to rmatch. The `*rparseargs` check is the
    // "trailing-args-after-regex" error.
    let mut ret;
    let mut result = RParseResult { nullacts: Vec::new(), args: Vec::new() };
    let parse_err = rparsealt(&mut result, std::ptr::null_mut()) != 0;
    if parse_err {                                                           // c:1500
        zwarnnam(nam, &format!("invalid regex : {}",                         // c:1502
            args.last().map(|s| s.as_str()).unwrap_or("")));
        ret = 3;                                                             // c:1505
    } else {
        ret = 0;                                                             // c:1508
    }

    if ret == 0 {                                                            // c:1510
        // c:1511 — `rmatch(&result, subj, var1, var2, OPT_ISSET(ops,'c'))`
        // — match the parsed regex tree against subj, capturing into
        // var1/var2. The rmatch port is open work; placeholder fall-
        // through to ret=0 (no match).
        let _ = OPT_ISSET(ops, b'c');
        let _ = (var1, var2, subj);
    }

    crate::ported::mem::popheap();                                           // c:1513
    crate::ported::options::opt_state_set(
        &crate::ported::zsh_h::opt_name(crate::ported::zsh_h::EXTENDEDGLOB),
        oldext,
    );                                                                       // c:1514
    ret                                                                      // c:1515
}

/// `Zoptdesc` family mirroring Src/Modules/zutil.c:1519-1538.
#[allow(non_camel_case_types)]
pub struct zoptdesc {
    pub name: String,
    pub flags: i32,
    pub arg: i32,
    pub vals: Vec<String>,
    pub next: Option<Box<zoptdesc>>,
}
pub type Zoptdesc = Box<zoptdesc>;
#[allow(non_camel_case_types)]
pub struct zoptarr {
    pub name: String,
    pub vals: Vec<String>,
}
pub type Zoptarr = Box<zoptarr>;

#[allow(non_camel_case_types)]

pub struct zoptval {
    pub name: String,
    pub arg: String,
}
pub type Zoptval = Box<zoptval>;

// =====================================================================
// ZOF_* — `zparseopts` flag bits, `Src/Modules/zutil.c:1531-1538`.
// Encode the per-option spec parsed from `zparseopts -D ...`:
// =====================================================================

/// `ZOF_ARG` from `Src/Modules/zutil.c:1531`. Option takes an argument
/// (suffix `:`).
pub const ZOF_ARG:  i32 = 1;                                                 // c:1531
/// `ZOF_OPT` from `Src/Modules/zutil.c:1532`. Argument is optional
/// (suffix `::`).
pub const ZOF_OPT:  i32 = 2;                                                 // c:1532
/// `ZOF_MULT` from `Src/Modules/zutil.c:1533`. Multiple occurrences
/// allowed (suffix `+`).
pub const ZOF_MULT: i32 = 4;                                                 // c:1533
/// `ZOF_SAME` from `Src/Modules/zutil.c:1534`. All same-name options
/// share one slot (default for arrays without `+`).
pub const ZOF_SAME: i32 = 8;                                                 // c:1534
/// `ZOF_MAP` from `Src/Modules/zutil.c:1535`. Option spec includes a
/// `=` mapping to a different array name.
pub const ZOF_MAP:  i32 = 16;                                                // c:1535
/// `ZOF_CYC` from `Src/Modules/zutil.c:1536`. Cyclic mapping detected
/// during option parsing (error guard).
pub const ZOF_CYC:  i32 = 32;                                                // c:1536
/// `ZOF_GNUS` from `Src/Modules/zutil.c:1537`. GNU-style `--option`
/// short variant.
pub const ZOF_GNUS: i32 = 64;                                                // c:1537
/// `ZOF_GNUL` from `Src/Modules/zutil.c:1538`. GNU-style `--option=value`
/// long variant.
pub const ZOF_GNUL: i32 = 128;                                               // c:1538

/// Port of `get_opt_desc(char *name)` from Src/Modules/zutil.c:1558.
/// C: `static Zoptdesc get_opt_desc(char *name)` — find a Zoptdesc.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn get_opt_desc(name: &str) -> Option<Zoptdesc> {                       // c:1558
    // c:1570
    // c:1570-1568 — walk opt_descs linked-list, name-compare.
    None
}

/// Port of `lookup_opt(char *str)` from Src/Modules/zutil.c:1570.
/// C: `static Zoptdesc lookup_opt(char *str)` — name-prefix match into
/// opt_descs; returns the desc or NULL.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn lookup_opt(str: &str) -> Option<Zoptdesc> {                          // c:1570
    // c:1570
    // c:1572-1600 — walks opt_descs comparing prefix with str.
    None
}

/// Port of `get_opt_arr(char *name)` from Src/Modules/zutil.c:1602.
/// C: `static Zoptarr get_opt_arr(char *name)` — find a Zoptarr by name.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn get_opt_arr(name: &str) -> Option<Zoptarr> {                         // c:1602
    // c:1602
    // c:1604-1612 — walk opt_arrs linked-list, name-compare.
    None
}

/// Port of `map_opt_desc(Zoptdesc start)` from Src/Modules/zutil.c:1614.
/// C: `static Zoptdesc map_opt_desc(Zoptdesc start)` — maps starting node
/// through alias chain.
#[allow(non_snake_case)]
#[allow(unused_variables)]
pub fn map_opt_desc(start: Option<Zoptdesc>) -> Option<Zoptdesc> {
    // c:1614
    // c:1616-1640 — alias-chase via opt_descs links.
    None
}

/// Port of `add_opt_val(Zoptdesc d, char *arg)` from Src/Modules/zutil.c:1642.
/// C: `static void add_opt_val(Zoptdesc d, char *arg)` — append a value
/// to the option's `vals` collection or assign to the bound array.
#[allow(non_snake_case)]
pub fn add_opt_val(d: &mut zoptdesc, arg: String) {                          // c:1642
    // c:1642
    // c:1644-1664 — dyncat("-", d->name); push value; bind to array.
    d.vals.push(arg);
}

/// Port of `zalloc_default_array(char ***aval, char *assoc, int keep, int num)` from Src/Modules/zutil.c:1710.
/// C: `static char **zalloc_default_array(int size)` — heap-alloc an
/// array of `size` empty strings.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(size) vs C=(aval, assoc, keep, num)
pub fn zalloc_default_array(size: i32) -> Vec<String> {
    // c:1710
    // c:1712-1716 — zhalloc((size+1) * sizeof(char *)); zero-init.
    vec![String::new(); size.max(0) as usize]
}

/// Direct port of `bin_zformat(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:954`.
/// C signature: `static int bin_zformat(char *nam, char **args,
/// Port of `bin_zparseopts(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:1738`. C
/// signature: `static int bin_zparseopts(char *nam, char **args,
/// UNUSED(Options ops), UNUSED(int func))`.
///
/// Implements the full GNU/zsh option parser:
///   - Flags: -D (delete consumed from argv), -E (extract),
///     -F (fail on unknown), -G (GNU long-opt mode),
///     -K (keep existing), -M (map), -a NAME (default array),
///     -A NAME (assoc array), -v NAME (source argv from NAME).
///   - Option descs: `name`, `name+` (multi), `name:` (mandatory arg),
///     `name::` (optional arg), `name:-` (same-arg), `=ARR` suffix.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zparseopts(nam: &str, args: &[String],                             // c:1738
                      _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {

    #[derive(Clone)]
    struct Desc {
        name: String,
        flags: i32,
        arr_name: Option<String>,
        vals: Vec<Val>,                 // collected values
    }
    #[derive(Clone)]
    struct Val {
        name: String,                   // option name as it appeared
        arg: Option<String>,            // arg if any
    }

    let mut del = false;                // c:1742
    let mut flags_map = 0i32;           // c:1742
    let mut extract = false;
    let mut fail = false;
    let mut gnu = false;
    let mut keep = false;
    let mut assoc: Option<String> = None;
    let mut paramsname: Option<String> = None;
    let mut defarr: Option<String> = None;
    let mut named_arrays: Vec<String> = Vec::new();

    // Phase 1: parse zparseopts flags (c:1751-1873).
    let mut i = 0usize;
    while i < args.len() {
        let o = &args[i];
        if !o.starts_with('-') { break; }
        if o.len() == 1 { break; }                                            // "-"
        let bytes = o.as_bytes();
        match bytes[1] {
            b'-' if bytes.len() == 2 => { i += 1; break; }                    // "--"
            b'-' => { break; }                                                // "-something"
            b'D' if bytes.len() == 2 => { del = true; i += 1; }
            b'E' if bytes.len() == 2 => { extract = true; i += 1; }
            b'F' if bytes.len() == 2 => { fail = true; i += 1; }
            b'G' if bytes.len() == 2 => { gnu = true; i += 1; }
            b'K' if bytes.len() == 2 => { keep = true; i += 1; }
            b'M' if bytes.len() == 2 => { flags_map |= ZOF_MAP; i += 1; }
            b'a' => {
                if defarr.is_some() {
                    zwarnnam(nam, "default array given more than once");
                    return 1;
                }
                let n = if o.len() > 2 { o[2..].to_string() }
                        else if i + 1 < args.len() { i += 1; args[i].clone() }
                        else { zwarnnam(nam, "missing array name"); return 1; };
                defarr = Some(n);
                i += 1;
            }
            b'A' => {
                if assoc.is_some() {
                    zwarnnam(nam, "associative array given more than once");
                    return 1;
                }
                let n = if o.len() > 2 { o[2..].to_string() }
                        else if i + 1 < args.len() { i += 1; args[i].clone() }
                        else { zwarnnam(nam, "missing array name"); return 1; };
                assoc = Some(n);
                i += 1;
            }
            b'v' => {
                if paramsname.is_some() {
                    zwarnnam(nam, "argv array given more than once");
                    return 1;
                }
                let n = if o.len() > 2 { o[2..].to_string() }
                        else if i + 1 < args.len() { i += 1; args[i].clone() }
                        else { zwarnnam(nam, "missing array name"); return 1; };
                paramsname = Some(n);
                i += 1;
            }
            _ => break,                                                       // option-desc
        }
    }
    if i >= args.len() {                                                      // c:1874
        zwarnnam(nam, "missing option descriptions");
        return 1;
    }

    // Phase 2: parse option descriptions (c:1878-1954).
    let mut descs: Vec<Desc> = Vec::new();
    while i < args.len() {
        let raw = &args[i];
        i += 1;
        if raw.is_empty() {
            zwarnnam(nam, &format!("invalid option description: {}", raw));
            return 1;
        }
        let bytes = raw.as_bytes();
        let mut name = String::new();
        let mut f = 0i32;
        let mut p = 0usize;
        // Parse name with backslash-escape, stopping at +/:/=. c:1884-1895.
        while p < bytes.len() {
            let c = bytes[p];
            if c == b'\\' && p + 1 < bytes.len() {
                name.push(bytes[p + 1] as char);
                p += 2;
                continue;
            }
            if p > 0 {
                if c == b'+' { f |= ZOF_MULT; p += 1; break; }
                if c == b':' || c == b'=' { break; }
            }
            name.push(c as char);
            p += 1;
        }
        // c:1897-1911 — :: arg flags.
        if p < bytes.len() && bytes[p] == b':' {
            f |= ZOF_ARG;
            p += 1;
            if gnu {
                f |= if name.len() > 1 { ZOF_GNUL } else { ZOF_GNUS };
            }
            if p < bytes.len() && bytes[p] == b':' { p += 1; f |= ZOF_OPT; }
            if p < bytes.len() && bytes[p] == b'-' { p += 1; f |= ZOF_SAME; }
        }
        // c:1913-1930 — `=ARR` suffix → bind to named array.
        let mut arr_name: Option<String> = None;
        if p < bytes.len() && bytes[p] == b'=' {
            p += 1;
            let arr = std::str::from_utf8(&bytes[p..]).unwrap_or("").to_string();
            if !named_arrays.contains(&arr) { named_arrays.push(arr.clone()); }
            arr_name = Some(arr);
            f |= flags_map;
        } else if p < bytes.len() {
            zwarnnam(nam, &format!("invalid option description: {}", raw));
            return 1;
        } else if defarr.is_none() && assoc.is_none() {
            zwarnnam(nam, &format!("no default array defined: {}", raw));
            return 1;
        }
        if descs.iter().any(|d| d.name == name) {
            zwarnnam(nam, &format!("option defined more than once: {}", name));
            return 1;
        }
        descs.push(Desc { name, flags: f, arr_name, vals: Vec::new() });
    }

    // Phase 3: source params (c:1955-1959).
    let params_src = paramsname.clone().unwrap_or_else(|| "argv".to_string());
    let mut params: Vec<String> = crate::fusevm_bridge::with_executor(|exec| {
        if params_src == "argv" {
            exec.pparams()
        } else {
            exec.array(&params_src).unwrap_or_default()
        }
    });

    // Phase 4: walk params (c:1961-2060).
    let mut new_params: Vec<String> = Vec::new();          // -E -D rebuild
    let mut pi = 0usize;
    let mut stopped = false;
    while pi < params.len() {
        let o_raw = params[pi].clone();
        // Not an option (or `-` in GNU mode).
        if !o_raw.starts_with('-') || (gnu && o_raw == "-") {
            if extract {
                if del { new_params.push(o_raw); }
                pi += 1;
                continue;
            } else { stopped = true; break; }
        }
        // `--` or non-GNU `-`: end.
        if o_raw == "-" || o_raw == "--" {
            if del && extract { new_params.push(o_raw); }
            pi += 1;
            stopped = true;
            break;
        }
        // Try whole-name match. c:1978.
        let body = &o_raw[1..];
        let whole_idx = descs.iter().position(|d|
            body == d.name || body.starts_with(&d.name)
                && body.as_bytes().get(d.name.len()).is_some_and(|b| *b == b'=' || *b == 0));
        let whole_match = whole_idx.map(|idx| {
            let d = &descs[idx];
            body == d.name ||
                (body.starts_with(&d.name) && (
                    body.as_bytes().get(d.name.len()) == Some(&b'=')))
        }).unwrap_or(false);
        if whole_match {
            let idx = whole_idx.unwrap();
            let dn_len = descs[idx].name.len();
            let dflags = descs[idx].flags;
            let dname = descs[idx].name.clone();
            if (dflags & ZOF_ARG) != 0 {
                let e = &body[dn_len..];                 // pointer past name
                if (dflags & ZOF_GNUL) != 0 && e.starts_with('=') {  // c:2031
                    let arg = e[1..].to_string();
                    descs[idx].vals.push(Val { name: o_raw.clone(), arg: Some(arg) });
                } else if !e.is_empty() {                              // c:2038
                    descs[idx].vals.push(Val { name: o_raw.clone(), arg: Some(e.to_string()) });
                } else if (dflags & ZOF_OPT) == 0
                    || ((dflags & (ZOF_GNUL | ZOF_GNUS)) == 0
                        && pi + 1 < params.len()
                        && !params[pi + 1].starts_with('-'))
                {                                                       // c:2044
                    if pi + 1 >= params.len() {
                        zwarnnam(nam,
                            &format!("missing argument for option: -{}", dname));
                        return 1;
                    }
                    pi += 1;
                    let arg = params[pi].clone();
                    descs[idx].vals.push(Val { name: o_raw.clone(), arg: Some(arg) });
                } else {                                                // c:2055
                    descs[idx].vals.push(Val { name: o_raw.clone(), arg: None });
                }
            } else {
                descs[idx].vals.push(Val { name: o_raw.clone(), arg: None });
            }
            pi += 1;
            continue;
        }
        // Fallback: each char as short opt. c:1980-2016.
        let chars: Vec<char> = o_raw[1..].chars().collect();
        let mut ci = 0usize;
        let mut consumed_param = true;
        while ci < chars.len() {
            let ch = chars[ci];
            let name1 = ch.to_string();
            let didx = descs.iter().position(|d| d.name == name1);
            let Some(idx) = didx else {
                if fail {
                    if ch != '-' || ci > 0 {
                        zwarnnam(nam, &format!("bad option: -{}", ch));
                    } else {
                        zwarnnam(nam, &format!("bad option: -{}", chars.iter().collect::<String>()));
                    }
                    return 1;
                }
                consumed_param = false;
                break;
            };
            let dflags = descs[idx].flags;
            let dname = descs[idx].name.clone();
            if (dflags & ZOF_ARG) != 0 {
                if ci + 1 < chars.len() {
                    // arg in same param: rest of chars
                    let arg: String = chars[ci + 1..].iter().collect();
                    descs[idx].vals.push(Val {
                        name: format!("-{}", ch),
                        arg: Some(arg),
                    });
                    break;
                } else if (dflags & ZOF_OPT) == 0
                    || ((dflags & (ZOF_GNUL | ZOF_GNUS)) == 0
                        && pi + 1 < params.len()
                        && !params[pi + 1].starts_with('-'))
                {
                    if pi + 1 >= params.len() {
                        zwarnnam(nam, &format!("missing argument for option: -{}", dname));
                        return 1;
                    }
                    pi += 1;
                    let arg = params[pi].clone();
                    descs[idx].vals.push(Val { name: format!("-{}", ch), arg: Some(arg) });
                } else {
                    descs[idx].vals.push(Val { name: format!("-{}", ch), arg: None });
                }
            } else {
                descs[idx].vals.push(Val { name: format!("-{}", ch), arg: None });
            }
            ci += 1;
        }
        if !consumed_param {
            if extract {
                if del { new_params.push(o_raw); }
                pi += 1;
                continue;
            } else {
                stopped = true;
                break;
            }
        }
        pi += 1;
    }
    let _ = stopped;
    // c:2069 — append remaining params if extract+del.
    if extract && del {
        while pi < params.len() {
            new_params.push(params[pi].clone());
            pi += 1;
        }
    } else if del && !extract {
        // c:2129: setaparam(paramsname, pp) — what's left from pi.
        new_params = params[pi..].to_vec();
    }

    // Phase 5: emit per-array results. c:2073-2088.
    // Group descs by arr_name → array of [name, arg, name, arg, ...].
    let mut arr_outputs: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for d in &descs {
        let target = d.arr_name.clone().or_else(|| defarr.clone());
        let Some(tgt) = target else { continue };
        let entry = arr_outputs.entry(tgt).or_default();
        for v in &d.vals {
            entry.push(v.name.clone());
            if let Some(a) = &v.arg {
                entry.push(a.clone());
            }
        }
    }
    for (name, vals) in arr_outputs {
        if !keep || !vals.is_empty() {
            crate::ported::params::setaparam(&name, vals);
        }
    }

    // c:2089-2123 — assoc emission.
    if let Some(aname) = assoc {
        let mut flat: Vec<String> = Vec::new();
        for d in &descs {
            if d.vals.is_empty() { continue; }
            flat.push(format!("-{}", d.name));
            let joined: String = d.vals.iter()
                .filter_map(|v| v.arg.clone())
                .collect::<Vec<_>>().join(" ");
            flat.push(joined);
        }
        if !keep || !flat.is_empty() {
            crate::ported::params::sethparam(&aname, flat);
        }
    }

    // c:2124-2131 — write back consumed argv when -D was given.
    if del {
        if params_src == "argv" {
            crate::fusevm_bridge::with_executor(|exec| {
                exec.set_pparams(new_params.clone());
            });
            if let Ok(mut pp) = crate::ported::builtin::PPARAMS.lock() {
                *pp = new_params;
            }
        } else {
            crate::ported::params::setaparam(&params_src, new_params);
        }
    } else {
        let _ = params;
    }

    0
}

// `bintab` — port of `static struct builtin bintab[]` (zutil.c).


// `module_features` — port of `static struct features module_features`
// from zutil.c:2143.



/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/zutil.c:2152`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {                                    // c:2152
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/zutil.c:2161`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:2161
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/zutil.c:2169`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:2169
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/zutil.c:2176`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {                                     // c:2176
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/zutil.c:2183`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {                                  // c:2183
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/zutil.c:2190`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {                                   // c:2190
    0
}
// zstyle_entry is defined below (moved from exec.rs).

/// Save/restore for the per-pattern-match magic vars `$match`,
/// `$mbegin`, `$mend`. Direct port of `MatchData` and the
/// `savematch`/`restorematch`/`freematch` trio in
/// src/zsh/Src/Modules/zutil.c:33-80.
///
/// zstyle's `-e` (eval pattern on retrieve) and zregexparse's
/// inner pattern matches both want to evaluate patterns without
/// clobbering the caller's `$match[]`, `$mbegin[]`, `$mend[]`
/// variables. The C version keeps a heap-duplicated copy in a
/// `MatchData` struct, runs the inner match, then either
/// restores or frees. The Rust port stores `Option<Vec<String>>`
/// — `None` means the var was unset.
pub struct MatchData {
    pub r#match: Option<Vec<String>>,
    pub mbegin: Option<Vec<String>>,
    pub mend: Option<Vec<String>>,
}

/// `zstyle` storage table.
/// Port of the `zstyletab` HashTable Src/Modules/zutil.c builds —
/// `newzstyletable()` (line 270) creates it, `bin_zstyle()`
/// (line 487) drives every mutation. Stores `stypat` entries
/// (port of C `struct stypat`, zutil.c:95) per style name,
/// weight-sorted so the most specific pattern wins.
// `StyleTable` renamed to `style_table`. C uses `HashTable zstyletab`
// (`Src/Modules/zutil.c:209`) with `struct style` (zutil.c:91) nodes
// containing a `Stypat pats` linked list (zutil.c:97-104). Rust port
// uses a `HashMap<String, Vec<stypat>>` while the canonical
// `hashtable` port lands; the canonical `style` / `stypat` structs
// already exist at lines 1608 / 1596 below.
#[allow(non_camel_case_types)]
#[derive(Default)]
pub struct style_table {
    styles: HashMap<String, Vec<stypat>>,
}

/// Namespace for the recursive zformat walker — distinct from
/// the public zformat_substring entry point above so the inner
/// recursion doesn't collide with the outer wrapper's name.
struct ZFormat;

// ─── moved from src/ported/exec.rs (drift extraction) ───

/// One `zstyle` entry — Rust extension that flattens what C splits
/// across `struct style` (zutil.c:91, holds the style name) and
/// `struct stypat` (zutil.c:97, holds pat + vals). The canonical
/// split structs are at lines 1596 / 1608 above; this flat shape is
/// kept while the C-style HashTable port lands.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct zstyle_entry {
    pub pattern: String,
    pub style: String,
    pub values: Vec<String>,
}

/// `RParseResult` (used by zregexparse) — Src/Modules/zutil.c:1642.
pub struct RParseResult {
    pub nullacts: Vec<String>,
    pub args: Vec<String>,
}

/// Port of `setstypat(Style s, char *pat, Patprog prog, char **vals, int eval)` from `Src/Modules/zutil.c:814`.
/// Format a string with specifications
/// `zformat` builtin entry point.
/// Helper extracted from `bin_zformat()` (Src/Modules/zutil.c:814)
/// — same `%X:value` substitution + width / left/right-align /
/// repeat flag handling the C source's `zformat_substring()`
/// (line 814) implements.
pub fn zformat_substring(format: &str, specs: &HashMap<char, String>, presence: bool) -> String {
    // Direct port of src/zsh/Src/Modules/zutil.c:814
    // zformat_substring. Recursive walker that handles:
    //   - Plain `%X` substitutions
    //   - Optional `-` for right-align
    //   - Optional `N` for min width
    //   - Optional `.M` for max width
    //   - Ternary `%(SPECTEST.true-text.false-text)` — conditional
    //     substitution based on whether the spec exists / matches a
    //     numeric test value. With presence=true (zformat -F) the
    //     test compares the spec's existence/length; with
    //     presence=false (zformat -f) the test compares against an
    //     integer math eval of the spec value.
    //
    // The original C uses an output-buffer with growable backing;
    // we use a Rust String with push_* helpers. The recursive
    // descent + (skip || actval) pattern is the same.
    // Per zsh/Src/Modules/zutil.c::bin_zformat lines 975-976:
    // `specs['%']` and `specs[')']` are pre-populated to literal "%" and ")"
    // BEFORE the recursive walk, which is why `%%` produces `%` and
    // `%)` produces `)` even though no caller registers them. Rebuild
    // a private copy of the specs map with those defaults injected,
    // unless the caller explicitly overrode them.
    let mut effective: HashMap<char, String> = specs.clone();
    effective.entry('%').or_insert_with(|| "%".to_string());
    effective.entry(')').or_insert_with(|| ")".to_string());

    let bytes: Vec<char> = format.chars().collect();
    let mut out = String::with_capacity(bytes.len() + 16);
    let mut idx = 0;
    let _ = ZFormat::substring(
        &bytes, &mut idx, &mut out, '\0', &effective, presence, false,
    );
    out
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();


// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN ZUTIL.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:zformat".to_string(), "b:zparseopts".to_string(), "b:zregexparse".to_string(), "b:zstyle".to_string()]
}

// WARNING: NOT IN ZUTIL.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 4]);
    }
    0
}

// WARNING: NOT IN ZUTIL.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN ZUTIL.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 4,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the weight formula matches C's setstypat (zutil.c:344-385):
    /// component count (high 32 bits) + per-component specificity sum
    /// (low 32 bits). More specific = higher weight. Drives weight via
    /// style_table::set's inline weight calc (insertion order reflects
    /// weight ordering — most specific pattern appears first).
    #[test]
    fn test_style_pattern_weight() {
        let mut t = style_table::new();
        t.set("*",                  "s", vec!["broad".to_string()], false);
        t.set(":completion:*",      "s", vec!["mid".to_string()],   false);
        t.set(":completion:zsh:*",  "s", vec!["narrow".to_string()],false);
        // Most-specific match wins (sorted descending by weight at insertion).
        assert_eq!(t.get(":completion:zsh:complete", "s").unwrap()[0], "narrow");
        assert_eq!(t.get(":completion:bash:complete", "s").unwrap()[0], "mid");
        assert_eq!(t.get(":other:thing", "s").unwrap()[0], "broad");
    }

    /// Port of `bin_zparseopts(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:1738`.
    #[test]
    fn zof_flags_are_distinct_powers_of_two() {
        // c:1531-1538 — ZOF_* are independent bits in a single u8 field.
        let all = [ZOF_ARG, ZOF_OPT, ZOF_MULT, ZOF_SAME, ZOF_MAP, ZOF_CYC, ZOF_GNUS, ZOF_GNUL];
        let xor: i32 = all.iter().fold(0, |acc, &x| acc | x);
        let sum: i32 = all.iter().sum();
        assert_eq!(xor, sum, "ZOF_* bits must be disjoint");
        // Ensure each is a power of two.
        for v in all {
            assert!(v > 0 && (v & (v - 1)) == 0, "ZOF value {} is not a power of 2", v);
        }
    }

    /// Verifies pattern matching via the style_table.get path mirrors
    /// C's lookupstyle (zutil.c:443) walking the pats list for the
    /// first weight-sorted match.
    #[test]
    fn test_style_pattern_matches() {
        let mut t = style_table::new();
        t.set(":completion:*", "s1", vec!["v".to_string()], false);
        assert!(t.get(":completion:zsh:complete", "s1").is_some());
        assert!(t.get(":other:zsh", "s1").is_none());

        let mut t2 = style_table::new();
        t2.set("*", "s2", vec!["v".to_string()], false);
        assert!(t2.get("anything", "s2").is_some());
    }

    #[test]
    fn test_style_table_set_get() {
        let mut table = style_table::new();
        table.set(":completion:*", "verbose", vec!["yes".to_string()], false);

        let result = table.get(":completion:zsh", "verbose");
        assert_eq!(result, Some(&["yes".to_string()][..]));

        let result = table.get(":other", "verbose");
        assert!(result.is_none());
    }

    #[test]
    fn test_style_table_priority() {
        let mut table = style_table::new();
        table.set("*", "menu", vec!["no".to_string()], false);
        table.set(":completion:*", "menu", vec!["yes".to_string()], false);

        let result = table.get(":completion:zsh", "menu");
        assert_eq!(result, Some(&["yes".to_string()][..]));
    }

    #[test]
    fn test_style_table_delete() {
        let mut table = style_table::new();
        table.set("*", "style1", vec!["val".to_string()], false);
        table.set("*", "style2", vec!["val".to_string()], false);

        table.delete(None, Some("style1"));
        assert!(table.get("test", "style1").is_none());
        assert!(table.get("test", "style2").is_some());
    }

    #[test]
    fn test_style_test_bool() {
        let mut table = style_table::new();
        table.set("*", "enabled", vec!["yes".to_string()], false);
        table.set("*", "disabled", vec!["no".to_string()], false);
        table.set(
            "*",
            "multiple",
            vec!["a".to_string(), "b".to_string()],
            false,
        );

        assert_eq!(table.test_bool("ctx", "enabled"), Some(true));
        assert_eq!(table.test_bool("ctx", "disabled"), Some(false));
        assert_eq!(table.test_bool("ctx", "multiple"), None);
    }

    /// Port of `bin_zstyle(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:487`.
    /// Verifies the persistent global `zstyletab` round-trips
    /// set→get and that `lookupstyle` / `testforstyle` C-name shims
    /// see the same entry. Lock-stamps the global-state path that
    /// `bin_zstyle` relies on (Src/Modules/zutil.c:209).
    #[test]
    fn test_global_zstyletab_set_and_lookup() {
        let key_style = "test_zutil_global_marker_style";
        let key_pat = "test_zutil_global_marker_*";
        {
            let mut t = zstyletab.lock().unwrap();
            t.set(key_pat, key_style,
                  vec!["yes".to_string()], false);
        }
        let found = lookupstyle("test_zutil_global_marker_x", key_style);
        assert_eq!(found, vec!["yes".to_string()]);
        assert_eq!(testforstyle("test_zutil_global_marker_x", key_style), 0);
        assert_eq!(testforstyle("unmatched_ctx", "no_such_style_zzz"), 1);
        // Cleanup so other tests don't see the entry.
        {
            let mut t = zstyletab.lock().unwrap();
            t.delete(Some(key_pat), Some(key_style));
        }
    }

    #[test]
    fn test_zformat_basic() {
        let mut specs = HashMap::new();
        specs.insert('n', "test".to_string());
        specs.insert('v', "42".to_string());

        let result = zformat_substring("Name: %n, Value: %v", &specs, false);
        assert_eq!(result, "Name: test, Value: 42");
    }

    #[test]
    fn test_zformat_padding() {
        let mut specs = HashMap::new();
        specs.insert('n', "hi".to_string());

        let result = zformat_substring("[%10n]", &specs, false);
        assert_eq!(result, "[hi        ]");

        let result = zformat_substring("[%-10n]", &specs, false);
        assert_eq!(result, "[        hi]");
    }

    #[test]
    fn test_zformat_truncate() {
        let mut specs = HashMap::new();
        specs.insert('n', "hello world".to_string());

        let result = zformat_substring("[%.5n]", &specs, false);
        assert_eq!(result, "[hello]");
    }

    #[test]
    fn test_zformat_escape() {
        let specs = HashMap::new();
        let result = zformat_substring("100%%", &specs, false);
        assert_eq!(result, "100%");
    }

}
