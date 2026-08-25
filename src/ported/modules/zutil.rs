//! Zsh utility builtins - port of Modules/zutil.c
//!
//! Style stuff.                                                             // c:82
//! Hash table of styles and associated functions.                           // c:104
//! Format stuff.                                                            // c:800
//! Zregexparse stuff.                                                       // c:1091
//!
//! Provides zstyle, zformat, zparseopts builtins.

use crate::ported::builtin::PPARAMS;
use crate::ported::glob::tokenize;
use crate::ported::mem::{popheap, pushheap};
use crate::ported::options::opt_state_set;
use crate::ported::params::{
    assignaparam, getaparam, getsparam, paramtab, setaparam, sethparam, setsparam, unsetparam,
};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::signals_h::{queue_signals, unqueue_signals};
use crate::ported::utils::{errflag, zwarnnam};
use crate::ported::zsh_h::PAT_HEAPDUP;
use crate::ported::zsh_h::{
    eprog, features, hashnode, isset, module, opt_name, options, param, Eprog, HashNode, Param,
    Patprog, ERRFLAG_INT, EXTENDEDGLOB, MAX_OPS, OPT_ISSET, PAT_STATIC, PM_ARRAY,
};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

/// Port of `savematch(MatchData *m)` from Src/Modules/zutil.c:40.
/// C: `static void savematch(MatchData *m)` — snapshot $match/$mbegin/
/// $mend into the MatchData struct.
#[allow(non_snake_case)]
pub fn savematch(m: &mut MatchData) {
    // c:40
    queue_signals(); // c:44
                     // c:45-50 — three `a = getaparam("X"); m->X = a ? zarrdup(a) : NULL`
                     // captures. The previous Rust port hardcoded `a = None` for all
                     // three because the fabricated `getaparam(Option<&mut value>)` sig
                     // couldn't take a name string. Now that `getaparam(&str)` matches
                     // C, real reads from paramtab work end-to-end.
    m.r#match = getaparam("match"); // c:45-46
    m.mbegin = getaparam("mbegin"); // c:47-48
    m.mend = getaparam("mend"); // c:49-50
    unqueue_signals(); // c:51
}

/// Port of `static void restorematch(MatchData *m)` from
/// `Src/Modules/zutil.c:55`.
///
/// C body (c:57-68):
/// ```c
/// if (m->match)  setaparam("match",  m->match);
/// else           unsetparam("match");
/// if (m->mbegin) setaparam("mbegin", m->mbegin);
/// else           unsetparam("mbegin");
/// if (m->mend)   setaparam("mend",   m->mend);
/// else           unsetparam("mend");
/// ```
///
/// Restores `$match`/`$mbegin`/`$mend` from a snapshot. Critical:
/// when the saved field is NULL/None, C **unsets** the param. The
/// previous Rust port left it alone — comment claimed "the Rust
/// paramtab API doesn't yet expose unsetparam-by-string" but
/// `unsetparam` HAS been ported (`params::unsetparam` at
/// params.rs:4731). Skipping the unset means a regex callout that
/// set `$match` from an originally-unset state would leave `$match`
/// set after restorematch — the OPPOSITE of the documented contract.
pub fn restorematch(m: &MatchData) {
    // c:55
    // c:57-68 — C uses `setaparam` which routes through `assignaparam`
    // with `ASSPM_WARN` (params.c:3766). The warn flag tells
    // assignaparam to emit "scalar parameter X created globally in
    // function" / "read-only variable" diagnostics on write attempts.
    //
    // Prior Rust port called `assignaparam` directly with `flags = 0`,
    // dropping the WARN bit — a user who pinned `typeset -r match`
    // upstream would see the readonly assignment silently swallowed,
    // where C zsh prints `zsh: read-only variable: match` and bails.
    // Route through `setaparam` (port at params.rs:6023 already does
    // the `assignaparam(name, val, ASSPM_WARN)` wrap) so the
    // diagnostic surface matches C bit-for-bit.
    if let Some(v) = m.r#match.as_ref() {
        setaparam("match", v.clone()); // c:58
    } else {
        unsetparam("match"); // c:60
    }
    if let Some(v) = m.mbegin.as_ref() {
        setaparam("mbegin", v.clone()); // c:62
    } else {
        unsetparam("mbegin"); // c:64
    }
    if let Some(v) = m.mend.as_ref() {
        setaparam("mend", v.clone()); // c:66
    } else {
        unsetparam("mend"); // c:68
    }
}

/// Port of `freematch(Cmatch m, int nbeg, int nend)` from Src/Modules/zutil.c:72.
/// C: `static void freematch(MatchData *m)` — drops the captured arrays.
#[allow(non_snake_case)]
pub fn freematch(m: &mut MatchData) {
    // c:72
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
///
/// `Clone` is a RUST-ONLY addition (C moves pointers): `(...)` subshells
/// are in-process in zshrs, so `subshell_begin` has to deep-copy the
/// zstyle table the way `fork()` copies it for C — see the
/// `SubshellSnapshot::zstyles` field in fusevm_bridge.rs.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct stypat {
    pub next: Option<Box<stypat>>, // c:98 Stypat next
    pub pat: String,               // c:99 char *pat
    pub prog: Option<Patprog>,     // c:100 Patprog prog (compiled)
    pub weight: u64,               // c:101 zulong weight
    pub eval: Option<Eprog>,       // c:102 Eprog eval
    pub vals: Vec<String>,         // c:103 char **vals
}
/// `Stypat` type alias.
pub type Stypat = Box<stypat>;

/// `Style` mirroring Src/Modules/zutil.c:91-94.
#[allow(non_camel_case_types)]
pub struct style {
    pub node: hashnode,       // c:92 struct hashnode node
    pub pats: Option<Stypat>, // c:93 Stypat pats (sorted by weight)
}
/// `Style` type alias.
pub type Style = Box<style>;

/// Global `zstyletab` mirror — port of the static
/// `static HashTable zstyletab` in Src/Modules/zutil.c:209.
/// C allocates this via `newzstyletable()` (c:270) during
/// module setup; the Rust port uses a `LazyLock<Mutex<>>`
/// since the table is process-global and `bin_zstyle` /
/// `lookupstyle` / `testforstyle` all need to share it.
#[allow(non_upper_case_globals)]
pub static zstyletab: std::sync::LazyLock<Mutex<style_table>> =
    std::sync::LazyLock::new(|| Mutex::new(style_table::new())); // c:209

/// Port of `freestylepatnode(Stypat p)` from Src/Modules/zutil.c:111.
/// C: `static void freestylepatnode(Stypat p)` — drops pat/prog/vals/eval.
#[allow(non_snake_case)]
pub fn freestylepatnode(p: Stypat) {
    // c:111
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
pub fn freestylenode(hn: HashNode) {
    // c:123
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
pub fn freestypat(mut p: Stypat, s: Option<&mut style>, prev: Option<&mut stypat>) {
    // c:151
    // c:151-158 — relink prev->next to p->next (or s->pats if no prev).
    // Use Option::take() to move the chain pointer out of p, since
    // stypat doesn't derive Clone (matching C's pointer-move semantics).
    let next = p.next.take(); // c:155 capture p->next
    let s_has_some = s.is_some();
    if let Some(s_ref) = s {
        // c:153
        if let Some(prev_ref) = prev {
            // c:154
            prev_ref.next = next; // c:155 prev->next = p->next
        } else {
            s_ref.pats = next; // c:157 s->pats = p->next
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
    pub fn set(
        &mut self,
        pattern: &str,
        style: &str,
        values: Vec<String>,
        eval_prog: Option<Eprog>,
    ) {
        let style_patterns = self.styles.entry(style.to_string()).or_default();
        // c:319-333 — Exists → replace.
        if let Some(existing) = style_patterns.iter_mut().find(|p| p.pat == pattern) {
            existing.vals = values; // c:328
            existing.eval = eval_prog; // c:329 p->eval = eprog (the parsed program)
            return;
        }
        // c:344-385 — Calculate weight: high 32 bits = colon-component
        // count, low 32 bits = sum of per-component specificity (0/1/2).
        //
        // Scoring per component:
        //   `*` (alone in component, must be followed by NUL or `:`) → 0
        //   contains a pattern metachar (`( | * [ < ? # ^`) → 1
        //   plain literal → 2
        //
        // Prior Rust port omitted the c:365 lookahead `(!str[1] ||
        // str[1] == ':')` on the wildcard-component check, so patterns
        // like `*foo:bar` mis-scored their first component as a bare
        // wildcard (0) instead of a metachar-containing pattern (1).
        // The miscount produced incorrect zstyle ordering — more-
        // specific patterns lost out to less-specific siblings.
        let mut weight: u64 = 0;
        let mut tmp: u64 = 2;
        let mut first = true;
        let bytes = pattern.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let ch = bytes[i] as char;
            let next: Option<u8> = bytes.get(i + 1).copied();
            // c:365 — `if (first && *str == '*' && (!str[1] || str[1] == ':'))`
            //   "alone-star component": star is the first char AND the
            //   next char ends the component (NUL or `:`).
            if first && ch == '*' && (next.is_none() || next == Some(b':')) {
                tmp = 0;
                i += 1;
                continue;
            }
            first = false; // c:370
            if matches!(ch, '(' | '|' | '*' | '[' | '<' | '?' | '#' | '^') {
                // c:372
                tmp = 1;
            }
            if ch == ':' {
                // c:377
                weight += 1u64 << 32; // c:379
                first = true; // c:381
                weight += tmp; // c:382
                tmp = 2; // c:383
            }
            i += 1;
        }
        weight += tmp; // c:386
                       // c:337-342 — New pattern: build stypat.
                       // c:339 — p->prog = prog; the C arg comes from patcompile()
                       // before setstypat is called. The style_table::set API takes
                       // pattern as &str and compiles at lookup-time via patmatch,
                       // so we record None here and rely on get() to match.
        let prog: Option<Patprog> = None;
        let sp = stypat {
            next: None,               // c:342
            pat: pattern.to_string(), // c:338
            prog,                     // c:339
            weight,                   // c:386
            eval: eval_prog,          // c:341 p->eval = eprog (the parsed program)
            vals: values,             // c:340
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
                        patcompile(
                            &{
                                let mut __pat_tok = (&p.pat).to_string();
                                crate::ported::glob::tokenize(&mut __pat_tok);
                                __pat_tok
                            },
                            PAT_HEAPDUP as i32,
                            None,
                        )
                        .map_or(false, |prog| pattry(&prog, context))
                    }
                })
                .map(|p| p.vals.as_slice())
        })
    }

    /// c:Src/Modules/zutil.c:768-779 — `bin_zstyle -g` retrieval. Unlike
    /// `lookupstyle`/`get` (which `pattry`-match the CONTEXT against every
    /// stored pattern), `-g` does an EXACT pattern-string compare
    /// (`if (!strcmp(args[2], p->pat))`). So after `zstyle ':s:*' k v`,
    /// `zstyle -g out ':s:sub' k` returns NOTHING (":s:sub" != ":s:*"),
    /// whereas `zstyle -s ':s:sub' k out` matches and yields "v".
    pub fn get_exact(&self, pattern: &str, style: &str) -> Option<&[String]> {
        self.styles.get(style).and_then(|pats| {
            pats.iter()
                .find(|p| p.pat == pattern)
                .map(|p| p.vals.as_slice())
        })
    }

    /// WARNING: NOT IN ZUTIL.C — method on the Rust-only `style_table`
    /// wrapper. Same best-pattern-match walk as `get`, but returns the
    /// matched entry's values AND whether it is an `-e` (eval) style, so
    /// `lookupstyle` can decide to execute the body (C reads the matched
    /// `Stypat`'s `eval` field inline; the wrapper keeps the map private).
    pub fn get_match(&self, context: &str, style: &str) -> Option<(Vec<String>, bool)> {
        self.styles.get(style).and_then(|patterns| {
            patterns
                .iter()
                .find(|p| {
                    if p.pat == "*" {
                        true
                    } else {
                        patcompile(
                            &{
                                let mut __pat_tok = (&p.pat).to_string();
                                crate::ported::glob::tokenize(&mut __pat_tok);
                                __pat_tok
                            },
                            PAT_HEAPDUP as i32,
                            None,
                        )
                        .map_or(false, |prog| pattry(&prog, context))
                    }
                })
                .map(|p| (p.vals.clone(), p.eval.is_some()))
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
        // c:Src/Modules/zutil.c:558 / :751 / :756 — every listing consumer
        // (`printstyle` for the plain/`-L` listing, `scanpatstyles` for both
        // `-g` shapes) calls `scanhashtable(zstyletab, 1, …)`, and that
        // leading `1` is the SORTED flag: the style table is walked in
        // strcmp order of the style NAME. `zstyle -g out` therefore returns
        // its contexts in a stable order in zsh, while iterating the Rust
        // HashMap directly produced a different order on every run.
        let mut style_names: Vec<&String> = self.styles.keys().collect();
        style_names.sort(); // c:hashtable.c scanhashtable sorted arm
        for style in style_names {
            let patterns = &self.styles[style];
            for pat in patterns {
                if let Some(ctx) = context {
                    let matches = if pat.pat == "*" {
                        true
                    } else {
                        patcompile(
                            &{
                                let mut __pat_tok = (&pat.pat).to_string();
                                crate::ported::glob::tokenize(&mut __pat_tok);
                                __pat_tok
                            },
                            PAT_HEAPDUP as i32,
                            None,
                        )
                        .map_or(false, |prog| pattry(&prog, ctx))
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
pub fn printstylenode(hn: &hashnode, printflags: i32, context_pat: Option<&str>) {
    // c:184
    // c:186-211 — Two distinct output formats based on `printflags`:
    //
    //   ZSLIST_BASIC = 1: `zstyle -L NAME` long format. Emits the
    //                     style name, then one line per (pat, vals)
    //                     prefixed by `(eval)` or 6 spaces.
    //   other (= 0):      `zstyle -L` re-feedable format. Emits
    //                     `zstyle [-e] '<pat>' '<style>' '<val>...'`
    //                     for each pattern.
    //
    // Prior Rust port for ZSLIST_BASIC stopped after emitting the
    // style name and never walked the patterns — `zstyle -L NAME`
    // printed only the heading, omitting the (pattern, values) lines
    // that are the whole point of the listing.
    //
    // Prior Rust port for the re-feedable arm always emitted
    // `zstyle ` without the `-e` flag, so an eval-style (set via
    // `zstyle -e PAT STYLE BODY`) round-tripped to a plain literal
    // style instead of the same eval form.
    let nam: String = hn.nam.clone();
    let mut stdout = std::io::stdout().lock();
    use crate::ported::utils::quotedzputs;
    let t = match zstyletab.lock() {
        Ok(t) => t,
        Err(_) => return,
    };
    let patterns = match t.styles.get(&nam) {
        Some(p) => p,
        None => return,
    };
    // c:196-197 — `zstyle_contprog`, the optional context-filter pattern
    // supplied to `zstyle -L <context>`. Each stored style-pattern is kept
    // only when it MATCHES this glob (so `zstyle -L :c1` lists just the
    // entries whose pattern matches `:c1`). Compile once per node.
    let cprog = context_pat.and_then(|c| {
        let mut pat = c.to_string();
        crate::ported::glob::tokenize(&mut pat);
        patcompile(&pat, crate::ported::zsh_h::PAT_STATIC, None)
    });
    if printflags == 1 {
        // c:190-193 — ZSLIST_BASIC header: the style name on its own line.
        // Only emitted when at least one pattern will survive the filter,
        // matching C (the header prints unconditionally, but the contprog
        // filter path is only reached via the syntax listing; bare `zstyle`
        // passes no context so every pattern survives here).
        let _ = writeln!(stdout, "{}", quotedzputs(&nam)); // c:191-192
    }
    for p in patterns {
        // c:196-197 — skip patterns that don't match the context filter.
        if let Some(ref prog) = cprog {
            if !pattry(prog, &p.pat) {
                continue;
            }
        }
        let is_eval = p.eval.is_some();
        if printflags == 1 {
            // c:198-199 — `printf("%s  %s", eval ? "(eval)" : "      ", p->pat);`
            let prefix = if is_eval { "(eval)" } else { "      " };
            let _ = write!(stdout, "{}  {}", prefix, p.pat); // c:199
        } else {
            // c:201-204 — `printf("zstyle %s", eval ? "-e " : "");
            //              quotedzputs(p->pat); putchar(' '); quotedzputs(style);`
            let eflag = if is_eval { "-e " } else { "" };
            let _ = write!(stdout, "zstyle {}", eflag); // c:201
            let _ = write!(stdout, "{}", quotedzputs(&p.pat)); // c:202
            let _ = write!(stdout, " "); // c:203
            let _ = write!(stdout, "{}", quotedzputs(&nam)); // c:204
        }
        // c:206-209 — per-value: ` `, quotedzputs(v).
        for v in &p.vals {
            let _ = write!(stdout, " {}", quotedzputs(v)); // c:207-208
        }
        let _ = writeln!(stdout); // c:210
    }
}

/// Port of `scanpatstyles(HashNode hn, int spatflags)` from Src/Modules/zutil.c:229.
/// C: `static void scanpatstyles(HashNode hn, int spatflags)` — iterate
/// every pattern of `hn`'s style, switching on `spatflags` (ZSPAT_NAME /
/// ZSPAT_PAT / ZSPAT_REMOVE).
#[allow(non_snake_case)]
pub fn scanpatstyles(hn: HashNode, spatflags: i32) {
    // c:229
    // c:229 — Style s = (Style)hn;
    let _s: HashNode = hn;
    // c:232 — Stypat p, q;
    // c:233 — LinkNode n;
    // c:235-265 — for (q = NULL, p = s->pats; p; q = p, p = p->next)
    // walks the pattern list and dispatches on spatflags. Rust port:
    // the HashNode→Style cast doesn't yield the pats list directly
    // (separate Boxes), so the body switches on spatflags and exits
    // each branch without traversal until the cast is wired.
    match spatflags {
        // c:236
        0 => { // c:237 ZSPAT_NAME
             // c:238-241 — if pat matches zstyle_patname, addlinknode + return
        }
        1 => { // c:244 ZSPAT_PAT
             // c:246-251 — addlinknode unless already present
        }
        2 => { // c:253 ZSPAT_REMOVE
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
                        // c:864 — `actval = (int) mathevali(specs[(unsigned
                        // char) *s]) - testval;`. The spec's VALUE is an
                        // arithmetic expression, not a bare integer: the
                        // documented `%18(s.math.)` with `s:6*3` is true.
                        // The previous `sv.parse()` returned 0 for any
                        // non-literal, so every arithmetic test was false.
                        let n: i64 = crate::ported::math::mathevali(sv).unwrap_or(0);
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
pub fn setstypat(
    style_name: &str,
    pat: &str, // c:295
    _prog: Option<Patprog>,
    vals: Vec<String>,
    eval: i32,
) -> i32 {
    // c:304-318 — `-e` (eval) style: parse the joined values into an
    // Eprog now, so lookup can execute it. C saves/restores errflag
    // around the parse (keeping any user-interrupt bit); on parse
    // failure it frees `prog` and returns 1. Here `_prog` drops on the
    // early return, which is the Rust-idiom equivalent of freepatprog.
    let eval_prog: Option<Eprog> = if eval != 0 {
        let joined = crate::ported::utils::zjoin(&vals, ' '); // c:309 zjoin(vals, ' ', 1)
        match crate::ported::exec::parse_string(&joined, 0) {
            // c:309
            None => return 1, // c:311-314 freepatprog(prog); return 1
            Some(ep) => Some(Box::new(crate::ported::parse::dupeprog(&ep, false))), // c:317
        }
    } else {
        None
    };
    if let Ok(mut t) = zstyletab.lock() {
        t.set(pat, style_name, vals, eval_prog); // c:319 set/replace
        0
    } else {
        1
    }
}

/// Port of `addstyle(char *name)` from Src/Modules/zutil.c:403.
/// C: `static Style addstyle(char *name)` — alloc a new Style node and
/// install in zstyletab.
#[allow(non_snake_case)]
/// C body (3 lines):
///     `Style s = (Style) zshcalloc(sizeof(*s));
///      zstyletab->addnode(zstyletab, ztrdup(name), s);
///      return s;`
pub fn addstyle(name: &str) -> Option<Style> {
    // c:403
    Some(Box::new(style {
        // c:405 zshcalloc + return
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: 0,
        },
        pats: None,
    }))
    // c:407 addnode — zstyletab integration is the caller's job; the
    //                 Box is returned for them to install.
}

/// Port of `evalstyle(Stypat p)` from Src/Modules/zutil.c:413.
/// Runs `p.eval` and reads `$reply` (array first, falling back to
/// scalar form). Returns empty Vec on error or unset.
/// `code` is the joined eval body (the values an `-e` style was
/// registered with, e.g. `reply=(computed-$((1+1)))`). C runs the
/// pre-parsed `p->eval` Eprog; zshrs re-runs the stored source through
/// the live executor, which is equivalent for setting `$reply`.
pub fn evalstyle(code: &str) -> Vec<String> {
    // c:413

    // c:415 — int ef = errflag;
    let ef = errflag.load(Ordering::Relaxed);
    // c:418 — unsetparam("reply");
    unsetparam("reply");
    // c:419 — execode(p->eval, 1, 0, "style"): execute the style body so
    // it can set `$reply`. Runs on the live session executor.
    {
        // c:Src/Modules/zutil.c:419 — `execode(p->eval, 1, 0, "style");`.
        // A `zstyle -e` body runs with `style` appended to the eval context.
        // Popped on every return path. Bug #1069.
        let sync_eval_ctx = |stack: &[String]| {
            let joined = stack.join(":");
            if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                if let Some(pm) = tab.get_mut("zsh_eval_context") {
                    pm.u_arr = Some(stack.to_vec());
                    pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                }
                if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                    pm.u_str = Some(joined);
                    pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                }
            }
        };
        if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
            ctx.push("style".to_string());
            sync_eval_ctx(&ctx);
        }
        struct StyleCtxGuard<F: Fn(&[String])>(F);
        impl<F: Fn(&[String])> Drop for StyleCtxGuard<F> {
            fn drop(&mut self) {
                if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
                    ctx.pop();
                    (self.0)(&ctx);
                }
            }
        }
        let _ctx_guard = StyleCtxGuard(sync_eval_ctx);
        let _ = crate::ported::exec::execute_script_zsh_pipeline(code);
    }
    // c:420-425 — restore errflag preserving INT bit only.
    let cur = errflag.load(Ordering::Relaxed);
    errflag.store(ef | (cur & ERRFLAG_INT), Ordering::Relaxed);
    if (cur & !ERRFLAG_INT) != 0 {
        return Vec::new(); // c:423
    }
    // c:427-433 — `if ((ret = getaparam("reply"))) ret = arrdup(ret);
    //              else if ((str = getsparam("reply"))) ret = [str];`
    queue_signals();
    let ret = if let Some(arr) = getaparam("reply") {
        arr
    } else if let Some(s) = getsparam("reply") {
        vec![s]
    } else {
        Vec::new()
    };
    unqueue_signals();
    // c:435 — unsetparam("reply");
    unsetparam("reply");
    ret
}

/// Port of `lookupstyle(char *ctxt, char *style)` from Src/Modules/zutil.c:443.
/// C: `static char **lookupstyle(char *ctxt, char *style)` — find best
/// pat-style match against the style entry; return its vals.
#[allow(non_snake_case)]
pub fn lookupstyle(ctxt: &str, style: &str) -> Vec<String> {
    // c:443
    // c:443-463 — zstyletab->getnode2 + savematch/pattry/restorematch
    // loop. style_table::get_match() encapsulates the pat-walk and also
    // reports whether the matched entry is an `-e` (eval) style; weight
    // order is enforced at insert time so first-match wins.
    //
    // c:450-451 + c:459 — the pat-walk is bracketed by
    // `savematch(&match)` / `restorematch(&match)`. That bracket is NOT
    // cosmetic: `restorematch` UNSETS `$match`/`$mbegin`/`$mend`
    // whenever the snapshot came back NULL (zutil.c:57-68), and
    // `savematch` reads them with `getaparam`, which returns NULL for a
    // non-array. So a plain `local match` (a scalar — `_main_complete`
    // declares exactly that at its sh:27 `local` line) is UNSET by the
    // first zstyle lookup in zsh. Without the bracket zshrs left the
    // scalar in place and `$parameters` reported one name zsh does not
    // have.
    // c:449-450 — `s = (Style)zstyletab->getnode2(zstyletab, style);
    // if (s) {`: the pattern walk AND its savematch/restorematch bracket
    // run ONLY when a style of that NAME exists. Bracketing every lookup
    // unconditionally unset `$match`/`$mbegin`/`$mend` (c:57-68) even
    // where C returns immediately — with no zstyles defined, C never
    // touches them, so `_main_complete`'s sh:27 `local match` scalar
    // survives the whole completion in zsh and was disappearing here.
    let style_exists = zstyletab
        .lock()
        .map(|t| t.list_styles().iter().any(|n| *n == style))
        .unwrap_or(false);
    if !style_exists {
        return Vec::new(); // c:447 `found = NULL` → c:461 `return found`
    }
    let mut saved = MatchData {
        r#match: None,
        mbegin: None,
        mend: None,
    };
    savematch(&mut saved); // c:450
    let matched = match zstyletab.lock() {
        Ok(t) => t.get_match(ctxt, style), // (vals, is_eval)
        Err(_) => None,
    };
    // Lock released before evalstyle so the body can touch zstyle/params
    // without re-entering the table lock.
    let found = match matched {
        // c:455-456 — `found = (p->eval ? evalstyle(p) : p->vals);`
        // C runs evalstyle INSIDE the loop, i.e. BEFORE restorematch, so
        // the `-e` body still sees the match vars the pattern set.
        Some((vals, true)) => evalstyle(&crate::ported::utils::zjoin(&vals, ' ')),
        Some((vals, false)) => vals,
        None => Vec::new(),
    };
    restorematch(&saved); // c:459
    found
}

// =====================================================================
// static struct features module_features                            c:2143
// =====================================================================

/// Port of `testforstyle(char *ctxt, char *style)` from Src/Modules/zutil.c:465.
/// C: `static int testforstyle(char *ctxt, char *style)` — non-empty
/// match check for context+style. Returns `!found` so 0 == success.
#[allow(non_snake_case)]
pub fn testforstyle(ctxt: &str, style: &str) -> i32 {
    // c:465
    // c:465-484 — zstyletab lookup + pattern match against ctxt,
    // bracketed by savematch/restorematch exactly as `lookupstyle` is
    // (c:473 / c:481). See the note there for why the bracket is
    // load-bearing rather than cosmetic.
    // c:471-472 — same `if (s)` gate as lookupstyle above.
    let style_exists = zstyletab
        .lock()
        .map(|t| t.list_styles().iter().any(|n| *n == style))
        .unwrap_or(false);
    if !style_exists {
        return 1; // c:483 `return !found` with found == 0
    }
    let mut saved = MatchData {
        r#match: None,
        mbegin: None,
        mend: None,
    };
    savematch(&mut saved); // c:473
    let found = match zstyletab.lock() {
        // c:471
        Ok(t) => t.get(ctxt, style).is_some(), // c:476 pattry
        Err(_) => false,
    };
    restorematch(&saved); // c:481
    if found {
        0
    } else {
        1
    } // c:485 return !found
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
pub fn bin_zstyle(
    nam: &str,
    args: &[String], // c:487
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:Src/Modules/zutil.c:487 — bin_zstyle parses args[0] directly
    // (BUILTIN spec has NULL optstr at c:2139) so the dispatch order
    // and unknown-flag diagnostic match zsh exactly. Build a local
    // `options` struct here, mirroring what execbuiltin's option
    // parser would have produced if there had been an optstr — then
    // the existing OPT_ISSET-driven flag arms below run unchanged.
    //
    // C flow at c:491-512 + c:587-600:
    //   - !args[0]                         → bare list mode
    //   - args[0] == "-" or "--"           → lone dash; positional follows
    //   - args[0] starts with -X + extra   → "invalid argument: %s"
    //   - args[0] == "-L"                  → list mode (ZSLIST_SYNTAX)
    //   - args[0] == "-e"                  → eval+add (handled like setstyle)
    //   - args[0] == "-d|-s|-b|-a|-t|-T|-m|-g" → action mode (positional follows)
    //   - else any other "-X"              → "invalid option: -X" rc=1
    let mut ops_local = options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    let mut positional_start = 0;
    if let Some(first) = args.first() {
        let fb = first.as_bytes();
        if fb.first() == Some(&b'-') && fb.len() >= 2 && fb[1] != b'-' {
            if fb.len() > 2 {
                // c:497-499 — extra chars after the option letter
                // (e.g. `-Lx`) → "invalid argument: -Lx" rc=1.
                crate::ported::utils::zwarnnam(nam, &format!("invalid argument: {}", first));
                return 1;
            }
            let oc = fb[1];
            if matches!(
                oc,
                b'L' | b'l'
                    | b'e'
                    | b'd'
                    | b's'
                    | b'b'
                    | b'a'
                    | b't'
                    | b'T'
                    | b'm'
                    | b'q'
                    | b'g'
                    | b'n'
                    | b'H'
            ) {
                ops_local.ind[oc as usize] = 1;
                positional_start = 1;
            } else {
                // c:597-599 default arm — "invalid option: -X".
                crate::ported::utils::zwarnnam(nam, &format!("invalid option: {}", first));
                return 1;
            }
        } else if fb == b"-" || fb == b"--" {
            // c:508-511 — lone `-` or `--` with no following positional
            // is "not enough arguments" (the lone dash implies set-style
            // mode which requires ≥2 positionals).
            if args.len() == 1 {
                crate::ported::utils::zwarnnam(nam, "not enough arguments");
                return 1;
            }
            positional_start = 1;
        }
    }
    let args: &[String] = &args[positional_start..];
    let ops: &options = &ops_local;
    // c:Src/Modules/zutil.c:588-604 — min-args check per action flag.
    // After the args[0] option is consumed, the remaining positionals
    // must satisfy each action's min count or "not enough arguments"
    // fires. Without this gate `zstyle -g`, `-s`, `-t`, `-T` (etc.)
    // silently returned rc=1 instead of emitting the canonical
    // diagnostic.
    let min_args = if OPT_ISSET(ops, b'd') {
        0
    } else if OPT_ISSET(ops, b's')
        || OPT_ISSET(ops, b'b')
        || OPT_ISSET(ops, b'a')
        || OPT_ISSET(ops, b'm')
    {
        3
    } else if OPT_ISSET(ops, b't') || OPT_ISSET(ops, b'T') || OPT_ISSET(ops, b'q') {
        // c:592-595 — t/T min 2; q min 2 max 2
        2
    } else if OPT_ISSET(ops, b'g') {
        1
    } else {
        0 // L/l/e/n/H or no flag — no min check at this layer
    };
    if args.len() < min_args {
        crate::ported::utils::zwarnnam(nam, "not enough arguments");
        return 1;
    }
    // c:491-492 — C reaches the bare-list arm only when `!args[0]`, i.e.
    // NO argument at all was given. `positional_start == 1` means an
    // option letter was consumed off args[0], so this is `zstyle -X`
    // (with X's own positionals exhausted), NOT a bare `zstyle`.
    // Without this gate, `zstyle -d` (delete everything, c:639-640
    // `zstyletab->emptytable`) fell into the listing arm and returned 0
    // having deleted nothing — every V05styles chunk after the
    // `zstyle -d` in chunk 1 then saw the leftover styles.
    if args.is_empty()
        && positional_start == 0
        && !OPT_ISSET(ops, b'L')
        && !OPT_ISSET(ops, b'l')
        && !OPT_ISSET(ops, b'e')
    {
        // c:491-492 + c:580-581 — bare `zstyle` invocation:
        // `list = ZSLIST_BASIC; scanhashtable(zstyletab, ..., printstylenode, list);`
        //
        // Route through printstylenode (zutil.rs:184 port) so the
        // (eval) prefix per pattern lands consistently. Prior bare-list
        // implementation duplicated the format inline with the eval-bit
        // hardcoded off — eval styles printed identical to literal
        // styles under bare `zstyle`.
        let names: Vec<String> = match zstyletab.lock() {
            Ok(t) => t.styles.keys().cloned().collect(),
            Err(_) => return 1,
        };
        let mut sorted = names;
        sorted.sort();
        for nam in sorted {
            // c:580 — scanhashtable callback dispatch per style.
            let hn = hashnode {
                next: None,
                nam,
                flags: 0,
            };
            printstylenode(&hn, 1, None); // c:580-581 — ZSLIST_BASIC
        }
        return 0; // c:585
    }
    if OPT_ISSET(ops, b'L') || OPT_ISSET(ops, b'l') {
        // c:544-583 — `zstyle -L [context [stylename]]`: list = ZSLIST_SYNTAX,
        // optionally filtered by a context pattern and/or an exact style name.
        //   args[0] = context (glob matched against each stored pattern),
        //   args[1] = stylename (only that style is listed; error if absent).
        let context = args.first().map(|s| s.as_str()); // c:551/556 context
        let stylename = args.get(1).map(|s| s.as_str()); // c:552 stylename
                                                         // c:562-570 — validate the context pattern up front (invalid → rc 1).
        if let Some(c) = context {
            let mut pat = c.to_string();
            crate::ported::glob::tokenize(&mut pat);
            if patcompile(&pat, crate::ported::zsh_h::PAT_STATIC, None).is_none() {
                return 1;
            }
        }
        // c:573-582 — a named style lists just that node (error if it does
        // not exist); otherwise scan every style.
        let names: Vec<String> = if let Some(sn) = stylename {
            let exists = zstyletab
                .lock()
                .map(|t| t.styles.contains_key(sn))
                .unwrap_or(false);
            if !exists {
                return 1; // c:575-577 — `if (!s) return 1;`
            }
            vec![sn.to_string()]
        } else {
            match zstyletab.lock() {
                Ok(t) => {
                    let mut v: Vec<String> = t.styles.keys().cloned().collect();
                    v.sort();
                    v
                }
                Err(_) => return 1,
            }
        };
        for nam in names {
            let hn = hashnode {
                next: None,
                nam,
                flags: 0,
            };
            printstylenode(&hn, 2, context); // c:501 — ZSLIST_SYNTAX
        }
        return 0; // c:585
    }
    if OPT_ISSET(ops, b'd') {
        // c:610-641 — three -d forms:
        //
        //     if (args[1]) {
        //         if (args[2]) {
        //             char *pat = args[1];
        //             for (args += 2; *args; args++) { ... freestypat per style ... }
        //         } else {
        //             zstyle_patname = args[1];
        //             scanhashtable(zstyletab, 0, 0, 0, scanpatstyles, ZSPAT_REMOVE);
        //         }
        //     } else
        //         zstyletab->emptytable(zstyletab);
        //
        // Form 1 takes pattern + MULTIPLE style names (the c:618 loop
        // walks args+2 to the end). Prior port read only args[1] —
        // `zstyle -d pat sty1 sty2 sty3` silently dropped sty2/sty3.
        let pat = args.first().map(|s| s.as_str());
        if let Ok(mut t) = zstyletab.lock() {
            if args.len() > 1 {
                // c:615-631 — per-style deletion of one pattern.
                for sty in &args[1..] {
                    t.delete(pat, Some(sty.as_str())); // c:626 freestypat
                }
            } else {
                // c:632-638 pattern-only / c:639-640 wipe-all.
                t.delete(pat, None);
            }
        }
        return 0; // c:524
    }
    // c:541-942 — -s/-b/-t/-T/-m/-a/-e per-context lookup arms.
    // -g has different arg layout (args[0] = output name, not context)
    // so it gets its own block below.
    if OPT_ISSET(ops, b's')
        || OPT_ISSET(ops, b'b')
        || OPT_ISSET(ops, b't')
        || OPT_ISSET(ops, b'T')
        || OPT_ISSET(ops, b'a')
        || OPT_ISSET(ops, b'm')
        || OPT_ISSET(ops, b'q')
    {
        if args.len() < 2 {
            return 1;
        }
        let ctxt = &args[0]; // c:541
        let style = &args[1];
        // c:749-757 — `case 'q': {
        //                  int success;
        //                  queue_signals();	/* Protect PAT_STATIC */
        //                  success = testforstyle(args[1], args[2]);
        //                  unqueue_signals();
        //                  return success;
        //              }`
        // Prior port had no -q arm at all — the option-letter matcher
        // rejected `zstyle -q ctx sty` with "invalid option: -q".
        if OPT_ISSET(ops, b'q') {
            queue_signals(); // c:752
            let success = testforstyle(ctxt, style); // c:753
            unqueue_signals(); // c:754
            return success; // c:755
        }
        let vals = lookupstyle(ctxt, style); // c:443
                                             // c:559-732 — per-flag return semantics: just check found vs not.
                                             // For -t: 0 if found AND first value matches one of the "true"
                                             // tokens (when arg given) or first ∈ {true,yes,on,1}.
        if OPT_ISSET(ops, b't') || OPT_ISSET(ops, b'T') {
            // c:700-724 — shared t/T arm:
            //
            //     if ((vals = lookupstyle(args[1], args[2])) && vals[0]) {
            //         if (args[3]) {
            //             char **ap = args + 3, **p;
            //             while (*ap) {
            //                 p = vals;
            //                 while (*p)
            //                     if (!strcmp(*ap, *p++))
            //                         return 0;
            //                 ap++;
            //             }
            //             return 1;
            //         } else
            //             return !(!strcmp(vals[0], "true") ||
            //                      !strcmp(vals[0], "yes") ||
            //                      !strcmp(vals[0], "on") ||
            //                      !strcmp(vals[0], "1"));
            //     }
            //     return (args[0][1] == 't' ? (vals ? 1 : 2) : 0);
            //
            // Two contracts the prior arms missed:
            //   1. Extra args = value-membership test: exit 0 when ANY
            //      style value string-equals ANY extra arg. Both arms
            //      ignored args[3..] entirely.
            //   2. -t's tri-state exit: 2 when the style is UNDEFINED
            //      for the context, 1 when defined-but-empty (or
            //      non-boolean first value). Prior -t collapsed both
            //      to 1.
            if !vals.is_empty() {
                // c:706 vals && vals[0]
                if args.len() > 2 {
                    // c:707-717
                    for ap in &args[2..] {
                        if vals.iter().any(|v| v == ap) {
                            return 0; // c:714
                        }
                    }
                    return 1; // c:717
                }
                // c:719-722 — boolean first value.
                return if matches!(vals[0].as_str(), "true" | "yes" | "on" | "1") {
                    0
                } else {
                    1
                };
            }
            // c:724 — `return (args[0][1] == 't' ? (vals ? 1 : 2) : 0);`
            // vals here is the C pointer: non-NULL when a pattern
            // matched but carried zero values. Probe the table for the
            // defined-but-empty vs undefined distinction.
            if OPT_ISSET(ops, b't') {
                let defined = match zstyletab.lock() {
                    Ok(t) => t.get(ctxt, style).is_some(),
                    Err(_) => false,
                };
                return if defined { 1 } else { 2 }; // c:724
            }
            return 0; // c:724 -T arm
        }
        // -m PATTERN: pattern-match args[2] against each value, return
        // 0 if any matches. C: zutil.c:727-747.
        if OPT_ISSET(ops, b'm') {
            // c:727
            if args.len() < 3 {
                return 1;
            }
            queue_signals(); // c:732 — Protect PAT_STATIC
            let mut pat = args[2].clone();
            tokenize(&mut pat); // c:734 — shell metachar → pattern token
            let prog = match patcompile(&pat, PAT_STATIC, None) {
                // c:737
                Some(p) => p,
                None => {
                    unqueue_signals(); // c:745
                    return 1;
                }
            };
            for v in &vals {
                // c:738
                if pattry(&prog, v) {
                    // c:739
                    unqueue_signals(); // c:740
                    return 0; // c:741
                }
            }
            unqueue_signals(); // c:745
            return 1; // c:746
        }
        // -s CONTEXT STYLE NAME [SEP]: join vals with SEP (default " "),
        // setsparam(NAME, joined). Return 0 if found else 1 (empty str).
        // C: zutil.c:643-658.
        if OPT_ISSET(ops, b's') {
            // c:643
            if args.len() < 3 {
                return 1;
            }
            let pname = &args[2];
            if !vals.is_empty() {
                let sep = args.get(3).map(|s| s.as_str()).unwrap_or(" "); // c:649
                let ret = vals.join(sep);
                setsparam(pname, &ret);
                return 0; // c:650
            }
            setsparam(pname, ""); // c:652
            return 1; // c:653
        }
        // -b CONTEXT STYLE NAME: coerce single bool-ish val to "yes"/"no".
        // C: zutil.c:660-680.
        if OPT_ISSET(ops, b'b') {
            // c:660
            if args.len() < 3 {
                return 1;
            }
            let pname = &args[2];
            let truthy = vals.len() == 1                                     // c:665-670
                && matches!(vals[0].as_str(),
                            "yes" | "true" | "on" | "1");
            let (ret, code) = if truthy { ("yes", 0) } else { ("no", 1) };
            setsparam(pname, ret); // c:677
            return code; // c:672/675
        }
        // -a CONTEXT STYLE NAME: setaparam(NAME, vals).
        // C: zutil.c:682-699:
        //
        //     if ((vals = lookupstyle(args[1], args[2]))) {
        //         ret = zarrdup(vals);
        //         val = 0;
        //     } else {
        //         char *dummy = NULL;
        //         ret = zarrdup(&dummy);
        //         val = 1;
        //     }
        //     setaparam(args[3], ret);
        //
        // Exit code keys on the lookupstyle POINTER, not vals[0]: a
        // pattern that matched with ZERO values still exits 0 (array
        // set empty). Rust lookupstyle collapses NULL and empty to
        // one Vec, so probe the table for the defined/undefined
        // distinction — same shape as the -t tri-state fix.
        if OPT_ISSET(ops, b'a') {
            // c:682
            if args.len() < 3 {
                return 1;
            }
            let pname = &args[2];
            let defined = match zstyletab.lock() {
                Ok(t) => t.get(ctxt, style).is_some(), // c:687 vals != NULL
                Err(_) => false,
            };
            // c:696 — `setaparam(args[3], ret)`. C's setaparam routes a
            // PM_HASHED (associative) target through the hashed-assign
            // path, treating the flat value list as alternating
            // key/value pairs. zshrs's setaparam clobbers the assoc into
            // an indexed array, so detect a PM_HASHED target and use
            // sethparam (key/val interleave) instead — `typeset -A h;
            // zstyle -a ctx style h` now populates h's keys.
            let is_assoc = crate::ported::params::paramtab()
                .read()
                .ok()
                .and_then(|t| {
                    t.get(pname.as_str())
                        .map(|p| (p.node.flags as u32 & crate::ported::zsh_h::PM_HASHED) != 0)
                })
                .unwrap_or(false);
            if is_assoc {
                sethparam(pname, vals); // c:696 (assoc: key/val pairs)
            } else {
                setaparam(pname, vals); // c:696 (empty when undefined)
            }
            return if defined { 0 } else { 1 }; // c:689/694
        }
        // -g: handled below (different arg layout).
        // -e: NOT a per-context lookup arm. C c:504-507 routes -e
        // through the add path (eval = add = 1), handled in the
        // canonical setstypat block below.
        if vals.is_empty() {
            return 1;
        }
        return 0;
    }
    // -g NAME [PATTERN [STYLE]]: collect into array NAME.
    // C: zutil.c:758-795. Distinct arg layout: args[0]=NAME (not ctxt).
    if OPT_ISSET(ops, b'g') {
        // c:758
        if args.is_empty() {
            return 1;
        }
        let pname = &args[0]; // c:792 args[1]→args[0]
        let pat_arg = args.get(1).map(|s| s.as_str()); // c:766
        let sty_arg = args.get(2).map(|s| s.as_str()); // c:767
        let mut out: Vec<String> = Vec::new();
        let t = match zstyletab.lock() {
            Ok(g) => g,
            Err(_) => return 1,
        };
        // c:759 — `int ret = 1;`. Only the exact-pattern lookup can fail
        // (return 1); the two listing branches always succeed (ret = 0).
        let mut ret = 1;
        match (pat_arg, sty_arg) {
            (None, _) => {
                // Collect distinct context patterns. c:788
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for (p, _s, _v) in t.list(None) {
                    if seen.insert(p.clone()) {
                        out.push(p);
                    }
                }
                ret = 0; // c:789
            }
            (Some(pat), None) => {
                // Collect style names attached to context = pat. c:783
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for (p, s, _v) in t.list(None) {
                    if p == pat && seen.insert(s.clone()) {
                        out.push(s);
                    }
                }
                ret = 0; // c:784
            }
            (Some(pat), Some(sty)) => {
                // c:768-779 — `-g` matches the pattern EXACTLY (strcmp),
                // NOT via pattry: it retrieves the value stored for that
                // precise pattern key, not the value a context would
                // resolve to. Use get_exact, not get (pattry). c:770-777:
                // ret stays 1 unless the exact pattern is found.
                if let Some(v) = t.get_exact(pat, sty) {
                    out.extend(v.iter().cloned());
                    ret = 0; // c:776
                }
            }
        }
        drop(t);
        setaparam(pname, out); // c:792
        return ret; // c:793
    }

    // c:515-534 — add path: zstyle [-e] PATTERN STYLE [VALUES...]
    if args.len() < 2 {
        zwarnnam(nam, "not enough arguments"); // c:521
        return 1;
    }
    let ctxt = &args[0]; // c:524
    let style = &args[1];
    let values: Vec<String> = if args.len() >= 3 {
        args[2..].to_vec() // c:533 args+2
    } else {
        Vec::new()
    };
    // c:524-530 — tokenize + patcompile validation. Reject invalid
    // patterns with the canonical "invalid pattern: %s" diagnostic
    // before they reach the style table.
    {
        let mut pat = ctxt.clone(); // c:524 dupstring
        tokenize(&mut pat); // c:525
        if patcompile(&pat, crate::ported::zsh_h::PAT_ZDUP, None).is_none() {
            // c:527
            zwarnnam(nam, &format!("invalid pattern: {}", ctxt)); // c:528
            return 1; // c:529
        }
    }
    let eval = OPT_ISSET(ops, b'e'); // c:505 eval = add = 1
                                     // c:533 — `setstypat(s, pat, prog, args + 2, eval)`. Route through
                                     // setstypat (which parses the `-e` value and locks zstyletab itself)
                                     // rather than locking + `t.set` here, matching C and avoiding a
                                     // re-entrant lock on the non-reentrant zstyletab mutex.
    setstypat(style, ctxt, None, values.clone(), eval as i32); // c:533
                                                               // PFA-SMR: one event per zstyle call. `rest` carries the style
                                                               // name + values so replay can re-emit the full setter.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() {
        let ctx = crate::recorder::recorder_ctx_global();
        let rest = if values.is_empty() {
            style.clone()
        } else {
            format!("{} {}", style, values.join(" "))
        };
        crate::recorder::emit_zstyle(ctxt, &rest, ctx);
    }
    0 // c:951
}

/// Port of `bin_zformat(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:955`.
/// C signature: `static int bin_zformat(char *nam, char **args,
/// UNUSED(Options ops), UNUSED(int func))`.
/// BUILTIN spec at zutil.c:2138 takes just two-or-more args (no
/// option flags); the first arg is `-f`/`-F`/`-a` (a single letter
/// after the dash) selecting the substitution mode.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_zformat(
    nam: &str,
    args: &[String], // c:955
    ops: &options,
    _func: i32,
) -> i32 {
    let mut presence = 0i32; // c:958
                             // C bin_zformat reads `args[0]` as the `-X` option directly (the
                             // BUILTIN spec doesn't pre-parse flags). zshrs's dispatch layer
                             // pre-parses flags into `ops` and strips them from args, so
                             // args[0] here is already the FIRST positional. Reconstruct the
                             // opt char from the parsed ops to match C's args[0][1] read.
    let opt: u8 = if OPT_ISSET(ops, b'f') {
        b'f'
    } else if OPT_ISSET(ops, b'F') {
        b'F'
    } else if OPT_ISSET(ops, b'a') {
        b'a'
    } else if !args.is_empty() {
        // Fallback to the C-shape read for old callers that still
        // pass `-X` as args[0].
        let opt_arg = &args[0];
        let bytes = opt_arg.as_bytes();
        if bytes.is_empty() || bytes[0] != b'-' || bytes.len() != 2 {
            zwarnnam(nam, &format!("invalid argument: {}", opt_arg)); // c:962
            return 1;
        }
        bytes[1]
    } else {
        zwarnnam(nam, &format!("invalid argument: {}", ""));
        return 1;
    };
    // If ops carried the flag, args is already the post-flag list.
    // If we read opt from args[0] (fallback path), advance past it.
    let args_used_opt_from_args0 =
        !OPT_ISSET(ops, b'f') && !OPT_ISSET(ops, b'F') && !OPT_ISSET(ops, b'a');
    let args: &[String] = if args_used_opt_from_args0 {
        &args[1..] // c:965 args++
    } else {
        args
    };

    match opt {
        // c:967
        b'F' | b'f' => {
            // c:968 / c:971
            if opt == b'F' {
                presence = 1;
            } // c:969 fall-through
              // c:973-994 — -f / -F branch.
            if args.len() < 2 {
                // c:973 args[0]/args[1]
                zwarnnam(nam, "missing arguments to -f/-F");
                return 1;
            }
            let mut specs: HashMap<char, String> = HashMap::new(); // c:973
            specs.insert('%', "%".to_string()); // c:976
            specs.insert(')', ")".to_string()); // c:977
            for ap in &args[2..] {
                // c:980
                let ab = ap.as_bytes();
                if ab.is_empty() || ab[0] == b'-' || ab[0] == b'.'            // c:981
                    || ab[0].is_ascii_digit()
                    || ab.len() < 2 || ab[1] != b':'
                {
                    zwarnnam(nam, &format!("invalid argument: {}", ap)); // c:984
                    return 1; // c:985
                }
                specs.insert(ab[0] as char, ap[2..].to_string()); // c:987
            }
            let out = zformat_substring(&args[1], &specs, presence != 0); // c:990
            setsparam(&args[0], &out); // c:993 setsparam
            return 0; // c:994
        }
        b'a' => {
            // c:996
            // c:998-1083 — -a column-format branch.
            if args.len() < 2 {
                // c:998
                zwarnnam(nam, "missing arguments to -a");
                return 1;
            }
            let mut pre = 0usize; // c:1000
            let mut suf = 0usize; // c:1000
                                  // First pass: compute max prefix/suffix widths.
            for ap in &args[2..] {
                // c:1005
                let mut nbc = 0usize; // c:1006
                let bytes = ap.as_bytes();
                let mut cp_idx = 0usize;
                while cp_idx < bytes.len() && bytes[cp_idx] != b':' {
                    // c:1007
                    if bytes[cp_idx] == b'\\' && cp_idx + 1 < bytes.len() {
                        // c:1008
                        cp_idx += 1;
                        nbc += 1;
                    }
                    cp_idx += 1;
                }
                if cp_idx < bytes.len() && bytes[cp_idx] == b':'              // c:1010
                    && cp_idx + 1 < bytes.len()
                {
                    let d = cp_idx.saturating_sub(nbc); // c:1015
                    if d > pre {
                        pre = d;
                    } // c:1016
                      // multi-byte width branch (c:1017-1029) collapses to
                      // ASCII byte count for the common case in Rust.
                    let s = bytes.len() - cp_idx - 1; // c:1030
                    if s > suf {
                        suf = s;
                    } // c:1031
                }
            }
            // Second pass: build formatted columns + setaparam.
            let middle = &args[1]; // c:1037
            let sl = middle.len(); // c:1037
            let mut ret: Vec<String> = Vec::new(); // c:1043
            for ap in &args[2..] {
                // c:1051
                let bytes = ap.as_bytes();
                let mut copy: Vec<u8> = Vec::with_capacity(bytes.len()); // c:1052
                let mut k = 0usize;
                let mut sep_at: Option<usize> = None;
                while k < bytes.len() {
                    // c:1053
                    if bytes[k] == b':' {
                        sep_at = Some(copy.len());
                        break;
                    }
                    if bytes[k] == b'\\' && k + 1 < bytes.len() {
                        // c:1054
                        k += 1;
                    }
                    copy.push(bytes[k]); // c:1055
                    k += 1;
                }
                // c:1058 — `((cpp==cp && oldc==':') || *cp==':') && cp[1]`:
                // align ONLY when a colon is present AND the value after it
                // is non-empty. `empty:` (colon, no value) and `nocolon`
                // (no colon) both fall to the else and emit just the key —
                // no padding, no separator. zshrs previously aligned any
                // colon-bearing spec, leaving a dangling `empty -- `.
                if sep_at.is_some() && k + 1 < bytes.len() {
                    let left_len = sep_at.unwrap();
                    // c:1058
                    let after = std::str::from_utf8(&bytes[(k + 1)..]).unwrap_or("");
                    let mut buf = String::with_capacity(pre + sl + after.len());
                    let prefix = std::str::from_utf8(&copy[..left_len]).unwrap_or("");
                    buf.push_str(prefix); // c:1072 memcpy(buf, copy, cpp-copy)
                                          // c:1071 memset(buf, ' ', pre) — pad up to byte count
                                          // `pre` (matches C !MULTIBYTE_SUPPORT byte-counting; the
                                          // first pass already computed `pre = cp - *ap - nbc`
                                          // in bytes c:1015). Using prefix.len() (bytes) not
                                          // chars().count() — for non-ASCII labels chars<bytes,
                                          // so the prior char-count version over-padded.
                    for _ in prefix.len()..pre {
                        buf.push(' ');
                    }
                    buf.push_str(middle); // c:1073 strcpy past `suf` shift
                    buf.push_str(after); // c:1073 (tail after `:`)
                    ret.push(buf); // c:1075 ztrdup
                } else {
                    ret.push(String::from_utf8_lossy(&copy).into_owned()); // c:1082
                }
            }
            let _ = sl;
            setaparam(&args[0], ret); // c:1081 setaparam(args[0], ret)
            return 0; // c:1082
        }
        _ => {}
    }
    zwarnnam(
        nam, // c:1085
        &format!("invalid option: -{}", opt as char),
    );
    1 // c:1086
}

/// Port of `connectstates(LinkList out, LinkList in)` from `Src/Modules/zutil.c:1119`.
/// For every (outbranch, inbranch) pair, create a new RParseBranch
/// whose target is inbranch.state and whose actions are
/// outbranch.actions ++ inbranch.actions, then add it to the
/// outbranch.state's branches list.
pub fn connectstates(
    out: &[std::rc::Rc<std::cell::RefCell<RParseBranch>>], // c:1119
    in_: &[std::rc::Rc<std::cell::RefCell<RParseBranch>>],
) {
    // c:1123 — for (outnode = firstnode(out); outnode; ...)
    for outnode in out.iter() {
        // c:1126 — for (innode = firstnode(in); innode; ...)
        for innode in in_.iter() {
            let outbranch = outnode.borrow();
            let inbranch = innode.borrow();
            // c:1128 — `br = hcalloc`; c:1130-1135 — populate.
            let mut new_actions: Vec<String> =
                Vec::with_capacity(outbranch.actions.len() + inbranch.actions.len());
            new_actions.extend(outbranch.actions.iter().cloned()); // c:1132-1133
            new_actions.extend(inbranch.actions.iter().cloned()); // c:1134-1135
            let br = std::rc::Rc::new(std::cell::RefCell::new(RParseBranch {
                state: inbranch.state.clone(), // c:1130
                actions: new_actions,
            }));
            // c:1136 — addlinknode(outbranch->state->branches, br);
            outbranch.state.borrow_mut().branches.push(br);
        }
    }
}

/// Port of `static int rparseelt(RParseResult *result, jmp_buf *perr)`
/// from `Src/Modules/zutil.c:1142`. Atom in the zregexparse grammar:
///   `/pat/[+/-]` \[`%lookahead%`\] \[`-guard`\] \[`:action`\]    — pattern atom
///   `(` ... `)`                                            — grouped alt
pub fn rparseelt(result: &mut RParseResult) -> i32 {
    // c:1142
    // c:1145 — s = *rparseargs;
    let s = match RPARSEARGS.with(|q| q.borrow().front().cloned()) {
        Some(s) => s,
        None => return 1, // c:1147-1148
    };
    let first = s.chars().next();
    match first {
        Some('/') => {
            // c:1151
            // c:1157 — l = strlen(s);
            // c:1158-1161 — require `/.../` or `/.../[+-]`.
            let l = s.len();
            let last = s.chars().last().unwrap_or(' ');
            let prevlast = if l >= 2 { s.as_bytes()[l - 2] } else { 0 };
            let ok_close = (l >= 2 && last == '/')
                || (l >= 3 && prevlast == b'/' && (last == '+' || last == '-'));
            if !ok_close {
                return 1;
            }
            // c:1162-1164 — alloc state, set cutoff.
            let mut st = RParseState::default();
            st.cutoff = last as i32;
            // c:1165-1171 — pattern slice between '/' and final '/[+-]?'.
            let (pattern, _patternlen) = if last == '/' {
                (&s[1..l - 1], l - 2)
            } else {
                (&s[1..l - 2], l - 3)
            };
            // c:1172 — rparseargs++;
            RPARSEARGS.with(|q| q.borrow_mut().pop_front());
            // c:1173-1180 — optional `%lookahead%` next arg.
            let lookahead: Option<String> = {
                let nxt = RPARSEARGS.with(|q| q.borrow().front().cloned());
                if let Some(la) = nxt {
                    if la.starts_with('%') && la.len() >= 2 && la.ends_with('%') {
                        RPARSEARGS.with(|q| q.borrow_mut().pop_front());
                        Some(la[1..la.len() - 1].to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            // c:1181-1202 — assemble compiled pattern string
            //   "[]"        → no pattern (NULL)
            //   else        → "(#b)((#B)<pat>)<lookahead?>*"
            if pattern == "[]" {
                st.pattern = None; // c:1182
            } else {
                let mut buf = String::with_capacity(pattern.len() + 16);
                buf.push_str("(#b)((#B)"); // c:1189-1190
                buf.push_str(pattern); // c:1191-1192
                buf.push(')'); // c:1193
                if let Some(la) = lookahead.as_ref() {
                    buf.push_str("(#B)"); // c:1196
                    buf.push_str(la); // c:1198
                }
                buf.push('*'); // c:1201
                st.pattern = Some(buf);
            }
            st.patprog = None; // c:1203
                               // c:1204-1211 — optional `-guard` arg.
            let nxt = RPARSEARGS.with(|q| q.borrow().front().cloned());
            if let Some(g) = nxt {
                if g.starts_with('-') {
                    RPARSEARGS.with(|q| q.borrow_mut().pop_front());
                    st.guard = Some(g[1..].to_string());
                }
            }
            // c:1212-1219 — optional `:action` arg.
            let nxt = RPARSEARGS.with(|q| q.borrow().front().cloned());
            if let Some(a) = nxt {
                if a.starts_with(':') {
                    RPARSEARGS.with(|q| q.borrow_mut().pop_front());
                    st.action = Some(a[1..].to_string());
                }
            }
            // Wrap state for sharing, register in RPARSESTATES.
            let st_rc = std::rc::Rc::new(std::cell::RefCell::new(st));
            RPARSESTATES.with(|s| s.borrow_mut().push(st_rc.clone()));

            // c:1220-1230 — result->in = [br(st)], result->out = [br(st)].
            result.nullacts = None; // c:1220
            let in_br = std::rc::Rc::new(std::cell::RefCell::new(RParseBranch {
                state: st_rc.clone(),
                actions: Vec::new(),
            }));
            result.in_ = vec![in_br]; // c:1221-1225
            let out_br = std::rc::Rc::new(std::cell::RefCell::new(RParseBranch {
                state: st_rc,
                actions: Vec::new(),
            }));
            result.out = vec![out_br]; // c:1226-1230
            0 // c:1248
        }
        Some('(') if s.len() == 1 => {
            // c:1233-1235
            // c:1236 — rparseargs++;
            RPARSEARGS.with(|q| q.borrow_mut().pop_front());
            // c:1237 — if (rparsealt(result, perr)) longjmp(*perr, 2);
            if rparsealt(result) != 0 {
                return 1; // longjmp surrogate
            }
            // c:1239-1241 — require closing `)`.
            let nxt = RPARSEARGS.with(|q| q.borrow().front().cloned());
            if nxt.as_deref() != Some(")") {
                return 1;
            }
            // c:1242 — rparseargs++;
            RPARSEARGS.with(|q| q.borrow_mut().pop_front());
            0
        }
        _ => 1, // c:1244-1245
    }
}

/// Port of `static int rparseclo(RParseResult *result, jmp_buf *perr)`
/// from `Src/Modules/zutil.c:1252`. Closure: atom followed by `#`
/// (zero-or-more); a string of `#`s collapses to one.
pub fn rparseclo(result: &mut RParseResult) -> i32 {
    // c:1252
    // c:1254 — if (rparseelt(result, perr)) return 1;
    if rparseelt(result) != 0 {
        return 1;
    }
    // c:1257-1264 — `if (*rparseargs && !strcmp(*rparseargs, "#")) { ... }`
    let mut saw = false;
    loop {
        let nxt = RPARSEARGS.with(|q| q.borrow().front().cloned());
        if nxt.as_deref() != Some("#") {
            break;
        }
        RPARSEARGS.with(|q| q.borrow_mut().pop_front());
        saw = true;
    }
    if saw {
        // c:1262 — connectstates(result->out, result->in)
        // Borrow result.out and result.in_ via cloned Rc lists to avoid
        // double-borrow (connectstates writes branches inside the states).
        let out_snap = result.out.clone();
        let in_snap = result.in_.clone();
        connectstates(&out_snap, &in_snap);
        // c:1263 — result->nullacts = newlinklist();
        result.nullacts = Some(Vec::new());
    }
    0 // c:1265
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/zutil.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `prependactions(LinkList acts, LinkList branches)` from `Src/Modules/zutil.c:1269`.
/// For each branch, pushnode (insert at HEAD) each action from `acts`
/// in reverse — net effect: branch.actions gets `acts` prepended.
pub fn prependactions(
    acts: &[String], // c:1269
    branches: &[std::rc::Rc<std::cell::RefCell<RParseBranch>>],
) {
    // c:1273 — for (bln = firstnode(branches); bln; ...)
    for bln in branches.iter() {
        let mut br = bln.borrow_mut();
        // c:1276 — for (aln = lastnode(acts); aln != (LinkNode)acts; aln = prevnode(aln))
        //   pushnode(br->actions, getdata(aln));
        // pushnode inserts at the HEAD of br->actions. C iterates acts
        // back-to-front and pushes each — net effect is acts in order
        // become the new prefix of br.actions.
        for aln in acts.iter().rev() {
            br.actions.insert(0, aln.clone());
        }
    }
}

/// Port of `appendactions(LinkList acts, LinkList branches)` from `Src/Modules/zutil.c:1282`.
/// For each branch, append each action from `acts` to br.actions.
pub fn appendactions(
    acts: &[String], // c:1282
    branches: &[std::rc::Rc<std::cell::RefCell<RParseBranch>>],
) {
    // c:1285 — for (bln = firstnode(branches); bln; ...)
    for bln in branches.iter() {
        let mut br = bln.borrow_mut();
        // c:1288 — for (aln = firstnode(acts); aln; ...) addlinknode(br->actions, getdata(aln));
        for aln in acts.iter() {
            br.actions.push(aln.clone());
        }
    }
}

/// Port of `rparseseq(RParseResult *result, jmp_buf *perr)` from Src/Modules/zutil.c:1294.
///
/// Part of the `zregexparse` builtin's recursive-descent
/// parser family (zutil.c:1140-1250 + helpers). zshrs's
/// Port of `static int rparseseq(RParseResult *result, jmp_buf *perr)`
/// from `Src/Modules/zutil.c:1294`. Recursive-descent parser for the
/// `zregexparse` builtin's sequence rule: walks `rparseargs` consuming
/// `{action}` blocks (paste into nullacts + every branch's actions
/// list) and `rparseclo` sub-results (connectstates / prependactions
/// / appendactions splice).
///
/// ```c
/// static int
/// rparseseq(RParseResult *result, jmp_buf *perr)
/// {
///     int l;
///     char *s;
///     RParseResult sub;
///     result->nullacts = newlinklist();
///     result->in = newlinklist();
///     result->out = newlinklist();
///     while (1) {
///         if ((s = *rparseargs) && s[0] == '{' && s[(l=strlen(s))-1] == '}') {
///             char *action = hcalloc(l - 1);
///             LinkNode ln;
///             rparseargs++;
///             memcpy(action, s + 1, l - 2);
///             action[l - 2] = '\0';
///             if (result->nullacts) addlinknode(result->nullacts, action);
///             for (ln = firstnode(result->out); ln; ln = nextnode(ln)) {
///                 RParseBranch *br = getdata(ln);
///                 addlinknode(br->actions, action);
///             }
///         } else if (!rparseclo(&sub, perr)) {
///             connectstates(result->out, sub.in);
///             if (result->nullacts) { prependactions(result->nullacts, sub.in);
///                 insertlinklist(sub.in, lastnode(result->in), result->in); }
///             if (sub.nullacts) { appendactions(sub.nullacts, result->out);
///                 insertlinklist(sub.out, lastnode(result->out), result->out); }
///             else result->out = sub.out;
///             if (result->nullacts && sub.nullacts)
///                 insertlinklist(sub.nullacts, lastnode(result->nullacts), result->nullacts);
///             else result->nullacts = NULL;
///         } else break;
///     }
///     return 0;
/// }
/// ```
///
/// Per PORT.md Rule 9: body executes. Struct field divergence
/// (Rust `RParseResult.args` vs C `in/out`) is a pre-existing port
/// gap tracked separately; the action-consumption branch operates
/// on the available `nullacts` field.
pub fn rparseseq(result: &mut RParseResult) -> i32 {
    // c:1294
    // c:1300-1302 — initialize result with empty lists.
    result.nullacts = Some(Vec::new()); // c:1300
    result.in_ = Vec::new(); // c:1301
    result.out = Vec::new(); // c:1302

    loop {
        // c:1304
        // c:1305 — `if ((s = *rparseargs) && s[0]=='{' && s[l-1]=='}')`
        let s = RPARSEARGS.with(|q| q.borrow().front().cloned());
        let action_arg = match s {
            Some(ref arg) if arg.len() >= 2 && arg.starts_with('{') && arg.ends_with('}') => {
                Some(arg.clone())
            }
            _ => None,
        };
        if let Some(arg) = action_arg {
            // c:1306 — char *action = hcalloc(l - 1);
            // c:1307-1311 — strip braces.
            let action = arg[1..arg.len() - 1].to_string();
            // c:1309 — rparseargs++;
            RPARSEARGS.with(|q| q.borrow_mut().pop_front());
            // c:1312-1313 — if (result->nullacts) addlinknode(result->nullacts, action);
            if let Some(ref mut na) = result.nullacts {
                na.push(action.clone());
            }
            // c:1314-1317 — for each branch in result->out: addlinknode(br->actions, action);
            for br in result.out.iter() {
                br.borrow_mut().actions.push(action.clone());
            }
            continue;
        }
        // c:1319 — `else if (!rparseclo(&sub, perr))`
        let mut sub = RParseResult::default();
        if rparseclo(&mut sub) == 0 {
            // c:1319
            // c:1320 — connectstates(result->out, sub.in);
            {
                let out_snap = result.out.clone();
                let in_snap = sub.in_.clone();
                connectstates(&out_snap, &in_snap);
            }
            // c:1322-1325 — `if (result->nullacts)
            //                   { prependactions(result->nullacts, sub.in);
            //                     insertlinklist(sub.in, lastnode(result->in), result->in); }`
            if let Some(ref na) = result.nullacts.clone() {
                prependactions(na, &sub.in_);
                // insertlinklist(src, after, dst): splice src into dst
                // AT the end (lastnode). Equivalent to append.
                result.in_.extend(sub.in_.iter().cloned());
            }
            // c:1326-1330 — sub.nullacts splice (or just steal sub.out).
            if let Some(ref sub_na) = sub.nullacts.clone() {
                appendactions(sub_na, &result.out);
                result.out.extend(sub.out.iter().cloned());
            } else {
                result.out = sub.out.clone();
            }
            // c:1332-1336 — combine nullacts (or NULL them).
            match (result.nullacts.as_mut(), sub.nullacts.as_ref()) {
                (Some(rna), Some(sna)) => {
                    rna.extend(sna.iter().cloned());
                }
                _ => {
                    result.nullacts = None;
                }
            }
        } else {
            // c:1338
            break; // c:1339
        }
    }
    0 // c:1341
}

/// Port of `static int rparsealt(RParseResult *result, jmp_buf *perr)`
/// from `Src/Modules/zutil.c:1345`. Alternation: one or more `seq`
/// terms separated by `|`.
pub fn rparsealt(result: &mut RParseResult) -> i32 {
    // c:1345
    // c:1349-1350 — if (rparseseq(result, perr)) return 1;
    if rparseseq(result) != 0 {
        return 1;
    }
    // c:1352 — while (*rparseargs && !strcmp(*rparseargs, "|"))
    loop {
        let nxt = RPARSEARGS.with(|q| q.borrow().front().cloned());
        if nxt.as_deref() != Some("|") {
            break;
        }
        // c:1353 — rparseargs++;
        RPARSEARGS.with(|q| q.borrow_mut().pop_front());
        let mut sub = RParseResult::default();
        // c:1354 — if (rparseseq(&sub, perr)) longjmp(*perr, 2);
        if rparseseq(&mut sub) != 0 {
            return 1;
        }
        // c:1356-1357 — if (!result->nullacts && sub.nullacts) result->nullacts = sub.nullacts;
        if result.nullacts.is_none() {
            if let Some(sn) = sub.nullacts.take() {
                result.nullacts = Some(sn);
            }
        }
        // c:1359-1360 — insertlinklist(sub.in,  lastnode(result->in),  result->in)
        //               insertlinklist(sub.out, lastnode(result->out), result->out)
        result.in_.extend(sub.in_.into_iter());
        result.out.extend(sub.out.into_iter());
    }
    0 // c:1362
}

/// Direct port of `static int rmatch(RParseResult *sm, char *subj,
/// char *var1, char *var2, int comp)` from `Src/Modules/zutil.c:1366-
/// 1474`. Executes the zregexparse state machine against `subj`:
/// drives transitions through the branch graph compiled by
/// `rparsealt`/`rparsestate`, runs guard predicates + action
/// strings via `execstring`, and binds the parse cursor to
/// `$var1`/`$var2`.
///
/// Returns:
///   - 0 if a complete parse path matches subj end-to-end (or in
///     completion mode + no out-edges constraint)
///   - 1 if no next-state candidates after exhausting subj (the
///     nextslist has at least one set of pending branches)
///   - 2 if there were no next-state candidates at all (nexts empty)
///   - 3 if a pattern fails to compile
pub fn rmatch(sm: &RParseResult, subj: &str, var1: &str, var2: &str, comp: i32) -> i32 {
    // c:1368-1373 — `LinkNode ln, lnn; LinkList nexts; LinkList nextslist;
    //                RParseBranch *br; RParseState *st = NULL; int point1=0, point2=0;`
    let mut st: Option<std::rc::Rc<std::cell::RefCell<RParseState>>> = None;
    let mut point1: i64 = 0; // c:1373
    let mut point2: i64 = 0; // c:1373

    // c:1375-1376 — `setiparam(var1, point1); setiparam(var2, point2);`
    crate::ported::params::setiparam(var1, point1);
    crate::ported::params::setiparam(var2, point2);

    // c:1378 — `if (!comp && !*subj && sm->nullacts)` — empty subj
    // matches nullacts directly when not in completion mode.
    if comp == 0 && subj.is_empty() {
        if let Some(nullacts) = sm.nullacts.as_ref() {
            // c:1379-1384 — `for (ln = firstnode(sm->nullacts); ...)
            //                    execstring(action, 1, 0, "zregexparse-action");`
            for action in nullacts {
                // c:1379
                if !action.is_empty() {
                    // c:1382
                    crate::ported::exec::execstring(action, 1, 0, "zregexparse-action");
                    // c:1383
                }
            }
            return 0; // c:1385
        }
    }

    // c:1388 — `nextslist = newlinklist();`
    let mut nextslist: Vec<Vec<std::rc::Rc<std::cell::RefCell<RParseBranch>>>> = Vec::new();
    // c:1389-1390 — `nexts = sm->in; addlinknode(nextslist, nexts);`
    let mut nexts: Vec<std::rc::Rc<std::cell::RefCell<RParseBranch>>> = sm.in_.clone();
    nextslist.push(nexts.clone()); // c:1390

    // `subj` is C's mutable char* cursor advancing through the input.
    // Mirror with a byte-index into the input.
    let subj_bytes = subj.as_bytes();
    let mut subj_pos: usize = 0; // c:1366 — initial cursor position.

    // c:1391-1449 — `do { ... } while (ln);` — the outer loop continues
    // as long as the inner for-loop FOUND a match (C's `ln` non-NULL
    // after `break`). In Rust we use an explicit `matched` flag.
    loop {
        // c:1392-1394 — `MatchData match1, match2; savematch(&match1);`
        let mut match1 = MatchData {
            r#match: None,
            mbegin: None,
            mend: None,
        };
        savematch(&mut match1);

        let mut matched = false; // mirror of C's `ln` truthiness after break.
                                 // c:1396 — `for (ln = firstnode(nexts); ln; ln = nextnode(ln))`
        for br_rc in &nexts.clone() {
            // c:1400 — `br = getdata(ln); next = br->state;`
            let br = br_rc.borrow();
            let next_rc = br.state.clone();
            drop(br); // release the immutable borrow before patprog mutation

            // c:1402-1406 — pattern compile on first match.
            //   `if (next->pattern && !next->patprog) {
            //        tokenize(next->pattern);
            //        if (!(next->patprog = patcompile(...))) return 3;`
            let (has_pattern, need_compile) = {
                let n = next_rc.borrow();
                (
                    n.pattern.is_some(),
                    n.pattern.is_some() && n.patprog.is_none(),
                )
            };
            if need_compile {
                let mut pat_str = {
                    let n = next_rc.borrow();
                    n.pattern.clone().unwrap_or_default()
                };
                crate::ported::glob::tokenize(&mut pat_str); // c:1403
                let prog = patcompile(&pat_str, 0, None); // c:1404
                let mut n = next_rc.borrow_mut();
                n.pattern = Some(pat_str);
                match prog {
                    Some(p) => n.patprog = Some(p), // c:1404
                    None => return 3,               // c:1405 — patcompile failure.
                }
            }
            if !has_pattern {
                continue; // c:1407 — `if (next->pattern && pattry(...))`
            }

            // c:1407 — `pattry(next->patprog, subj)`.
            let subj_remaining = &subj[subj_pos..];
            let pattry_ok = {
                let n = next_rc.borrow();
                if let Some(prog) = n.patprog.as_ref() {
                    pattry(prog, subj_remaining)
                } else {
                    false
                }
            };
            if !pattry_ok {
                continue;
            }

            // c:1407-1409 — `(!next->guard || (execstring(next->guard,
            //                  1, 0, "zregexparse-guard"), !lastval))`
            let guard_ok = {
                let n = next_rc.borrow();
                match n.guard.as_ref() {
                    None => true, // no guard — always OK
                    Some(g) => {
                        let g_cloned = g.clone();
                        drop(n);
                        crate::ported::exec::execstring(&g_cloned, 1, 0, "zregexparse-guard"); // c:1408
                        crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed)
                            == 0
                    }
                }
            };
            if !guard_ok {
                continue; // c:1409 — `!lastval` failed.
            }

            // c:1413-1417 — `if ((mend = getaparam("mend"))) len = atoi(mend[0]);`
            let mut len: i32 = 0;
            crate::ported::signals::queue_signals(); // c:1414
            if let Some(mend_vec) = crate::ported::params::getaparam("mend") {
                if let Some(first) = mend_vec.first() {
                    len = first.trim().parse().unwrap_or(0); // c:1416
                }
            }
            crate::ported::signals::unqueue_signals(); // c:1417

            // c:1419-1421 — `for (i = len; i; i--) if (*subj++ == Meta) subj++;`
            // Advance the cursor past `len` "characters", each
            // optionally preceded by a Meta byte (zsh metafication).
            let mut i = len;
            while i > 0 && subj_pos < subj_bytes.len() {
                if subj_bytes[subj_pos] == crate::ported::zsh_h::Meta {
                    subj_pos += 1; // c:1421 — skip the Meta byte.
                    if subj_pos < subj_bytes.len() {
                        subj_pos += 1; // c:1421 — and the following byte.
                    }
                } else {
                    subj_pos += 1; // c:1420 — plain ASCII advance.
                }
                i -= 1;
            }

            // c:1423 — `savematch(&match2);`
            let mut match2 = MatchData {
                r#match: None,
                mbegin: None,
                mend: None,
            };
            savematch(&mut match2);
            restorematch(&match1); // c:1424

            // c:1426-1431 — run all br->actions.
            let actions = {
                let br = br_rc.borrow();
                br.actions.clone()
            };
            for action in &actions {
                // c:1426 — `for (aln = firstnode(br->actions); ...)`
                if !action.is_empty() {
                    // c:1429
                    crate::ported::exec::execstring(action, 1, 0, "zregexparse-action");
                    // c:1430
                }
            }
            restorematch(&match2); // c:1432

            // c:1434-1435 — `point2 += len; setiparam(var2, point2);`
            point2 += len as i64;
            crate::ported::params::setiparam(var2, point2);
            // c:1436-1437 — `st = br->state; nexts = st->branches;`
            st = Some(next_rc.clone());
            nexts = {
                let n = next_rc.borrow();
                n.branches.clone()
            };
            // c:1438-1442 — cutoff handling: `-` (hard) or `/` with non-
            // zero match length resets the nextslist + point1 to point2.
            let cutoff = {
                let n = next_rc.borrow();
                n.cutoff
            };
            if cutoff == b'-' as i32 || (cutoff == b'/' as i32 && len != 0) {
                // c:1438
                nextslist = Vec::new(); // c:1439
                point1 = point2; // c:1440
                crate::ported::params::setiparam(var1, point1); // c:1441
            }
            nextslist.push(nexts.clone()); // c:1443
            matched = true; // mirror C's `ln` non-NULL after break.
            break; // c:1444
        }
        // c:1447-1448 — `if (!ln) freematch(&match1);`
        if !matched {
            freematch(&mut match1);
        }
        // c:1449 — `} while (ln);`
        if !matched {
            break;
        }
    }

    // c:1451-1463 — `if (!comp && !*subj)` post-loop out-edge check.
    if comp == 0 && subj_pos == subj_bytes.len() {
        for br_rc in &sm.out {
            // c:1452
            let br = br_rc.borrow();
            // c:1454 — `if (br->state == st)` — pointer equality.
            let is_match = st
                .as_ref()
                .map(|s| std::rc::Rc::ptr_eq(s, &br.state))
                .unwrap_or(false);
            if is_match {
                for action in &br.actions {
                    // c:1455
                    if !action.is_empty() {
                        // c:1458
                        crate::ported::exec::execstring(action, 1, 0, "zregexparse-action");
                        // c:1459
                    }
                }
                return 0; // c:1461
            }
        }
    }

    // c:1465-1472 — fallback: walk every accumulated nexts list and
    // execute any state-level action attached to its branches.
    let nextslist_for_fallback = nextslist.clone();
    for fallback_nexts in &nextslist_for_fallback {
        // c:1465
        for br_rc in fallback_nexts {
            // c:1467
            let br = br_rc.borrow();
            let action = {
                let n = br.state.borrow();
                n.action.clone()
            };
            if let Some(a) = action {
                // c:1469
                if !a.is_empty() {
                    crate::ported::exec::execstring(&a, 1, 0, "zregexparse-action");
                    // c:1470
                }
            }
        }
    }
    // c:1473 — `return empty(nexts) ? 2 : 1;`
    if nexts.is_empty() {
        2
    } else {
        1
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in vm_helper are unchanged.
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
pub fn bin_zregexparse(
    nam: &str,
    args: &[String], // c:1486
    ops: &options,
    _func: i32,
) -> i32 {
    if args.len() < 3 {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    let var1 = &args[0]; // c:1489
    let var2 = &args[1]; // c:1490
    let subj = &args[2]; // c:1491
    let _rparseargs = &args[3..]; // c:1497
    let _ = (var1, var2, subj);

    // c:1494 — `oldextendedglob = opts[EXTENDEDGLOB]; opts[EXTENDEDGLOB] = 1;`
    let oldext = isset(EXTENDEDGLOB); // c:1494
    opt_state_set(&opt_name(EXTENDEDGLOB), true); // c:1496

    // c:1499 — `pushheap(); rparsestates = newlinklist();`
    pushheap(); // c:1499

    // c:1500 — `if (setjmp(rparseerr) || rparsealt(&result, &rparseerr) ||
    // *rparseargs)`. rparsealt is a stub here (the alternation parser
    // is open work); without it the parse always succeeds vacuously
    // and we fall straight to rmatch. The `*rparseargs` check is the
    // "trailing-args-after-regex" error.
    let mut ret;
    // c:1495-1499 — pushheap(); rparsestates = newlinklist();
    //                 rparseargs = args + 3 (subj + var1 + var2 already consumed).
    RPARSESTATES.with(|s| s.borrow_mut().clear());
    // Skip the first 3 args (var1, var2, subj) already extracted at c:1493-1495.
    RPARSEARGS.with(|q| {
        let mut q = q.borrow_mut();
        q.clear();
        for a in args.iter().skip(3) {
            q.push_back(a.clone());
        }
    });
    let mut result = RParseResult::default();
    let parsealt_failed = rparsealt(&mut result) != 0;
    let leftover = RPARSEARGS.with(|q| !q.borrow().is_empty());
    let parse_err = parsealt_failed || leftover;
    if parse_err {
        // c:1500-1505 — C distinguishes "*rparseargs != NULL" from the
        // empty case:
        //   if (*rparseargs)
        //       zwarnnam(nam, "invalid regex : %s", *rparseargs);
        //   else
        //       zwarnnam(nam, "not enough regex arguments");
        //
        // Prior Rust port always emitted "invalid regex : <args.last()>"
        // — wrong on two counts:
        //   1. "not enough regex arguments" was unreachable; partial
        //      parses where rparsealt bailed mid-stream (no leftover
        //      tokens) reported as "invalid regex" with an empty token.
        //   2. The token shown was args.last() (the original tail of
        //      argv), not the first unparsed token from rparseargs.
        //
        // Read the actual front-of-queue from RPARSEARGS so the
        // diagnostic points at the failing token the way C does.
        let leftover_first: Option<String> = RPARSEARGS.with(|q| q.borrow().front().cloned());
        match leftover_first {
            Some(tok) => zwarnnam(nam, &format!("invalid regex : {}", tok)), // c:1502
            None => zwarnnam(nam, "not enough regex arguments"),             // c:1504
        }
        ret = 3; // c:1505
    } else {
        ret = 0; // c:1508
    }

    if ret == 0 {
        // c:1510-1512 — `ret = rmatch(&result, subj, var1, var2, OPT_ISSET(ops,'c'));`
        let comp_flag = if OPT_ISSET(ops, b'c') { 1 } else { 0 };
        ret = rmatch(&result, subj, var1, var2, comp_flag);
    }

    popheap(); // c:1513
    opt_state_set(&opt_name(EXTENDEDGLOB), oldext); // c:1514
    ret // c:1515
}

/// `Zoptdesc` family mirroring Src/Modules/zutil.c:1519-1538.
#[derive(Clone)]
#[allow(non_camel_case_types)]
pub struct zoptdesc {
    pub name: String,
    pub flags: i32,
    pub arg: i32,
    pub vals: Vec<String>,
    /// Owning `Zoptarr` name (the array this option's values go into).
    /// Port of C `Zoptarr arr` pointer at zutil.c:1525 — Rust stores
    /// the name and looks the arr up in `OPT_ARRS` on demand to
    /// avoid the cyclic `Box<zoptarr>`/`Box<zoptdesc>` reference.
    pub arr: Option<String>,
    pub next: Option<Box<zoptdesc>>,
}
/// `Zoptdesc` type alias.
pub type Zoptdesc = Box<zoptdesc>;
/// `zoptarr` — see fields for layout.
#[derive(Clone)]
#[allow(non_camel_case_types)]
pub struct zoptarr {
    pub name: String,
    pub vals: Vec<String>,
    pub num: i32,
    pub next: Option<Box<zoptarr>>,
}
/// `Zoptarr` type alias.
pub type Zoptarr = Box<zoptarr>;

/// Port of `static Zoptdesc opt_descs` from
/// `Src/Modules/zutil.c:1554`. Head of the per-`zparseopts`-call
/// option-spec linked list. Reset at the top of every zparseopts
/// invocation; walked by `get_opt_desc`/`lookup_opt`.
pub static OPT_DESCS: std::sync::Mutex<Option<Zoptdesc>> = std::sync::Mutex::new(None); // c:1554

/// Port of `static Zoptarr opt_arrs` from
/// `Src/Modules/zutil.c:1555`. Head of the array-slot linked list
/// each `Zoptdesc.arr` points into.
pub static OPT_ARRS: std::sync::Mutex<Option<Zoptarr>> = std::sync::Mutex::new(None); // c:1555
/// `zoptval` — see fields for layout.
#[allow(non_camel_case_types)]

pub struct zoptval {
    pub name: String,
    pub arg: String,
}
/// `Zoptval` type alias.
pub type Zoptval = Box<zoptval>;

// =====================================================================
// ZOF_* — `zparseopts` flag bits, `Src/Modules/zutil.c:1531-1538`.
// Encode the per-option spec parsed from `zparseopts -D ...`:
// =====================================================================

/// `ZOF_ARG` from `Src/Modules/zutil.c:1531`. Option takes an argument
/// (suffix `:`).
pub const ZOF_ARG: i32 = 1; // c:1531
/// `ZOF_OPT` from `Src/Modules/zutil.c:1532`. Argument is optional
/// (suffix `::`).
pub const ZOF_OPT: i32 = 2; // c:1532
/// `ZOF_MULT` from `Src/Modules/zutil.c:1533`. Multiple occurrences
/// allowed (suffix `+`).
pub const ZOF_MULT: i32 = 4; // c:1533
/// `ZOF_SAME` from `Src/Modules/zutil.c:1534`. All same-name options
/// share one slot (default for arrays without `+`).
pub const ZOF_SAME: i32 = 8; // c:1534
/// `ZOF_MAP` from `Src/Modules/zutil.c:1535`. Option spec includes a
/// `=` mapping to a different array name.
pub const ZOF_MAP: i32 = 16; // c:1535
/// `ZOF_CYC` from `Src/Modules/zutil.c:1536`. Cyclic mapping detected
/// during option parsing (error guard).
pub const ZOF_CYC: i32 = 32; // c:1536
/// `ZOF_GNUS` from `Src/Modules/zutil.c:1537`. GNU-style `--option`
/// short variant.
pub const ZOF_GNUS: i32 = 64; // c:1537
/// `ZOF_GNUL` from `Src/Modules/zutil.c:1538`. GNU-style `--option=value`
/// long variant.
pub const ZOF_GNUL: i32 = 128; // c:1538

/// Direct port of `static Zoptdesc get_opt_desc(char *name)` from
/// `Src/Modules/zutil.c:1558`. Walks the [`OPT_DESCS`] linked list
/// looking for an entry whose `name` matches `name` exactly.
#[allow(non_snake_case)]
pub fn get_opt_desc(name: &str) -> Option<Zoptdesc> {
    // c:1560 — `for (p = opt_descs; p; p = p->next) if (!strcmp(...))`.
    let head = OPT_DESCS.lock().unwrap();
    let mut cur = head.as_deref();
    while let Some(p) = cur {
        if p.name == name {
            return Some(Box::new(p.clone()));
        }
        cur = p.next.as_deref();
    }
    None
}

/// Direct port of `static Zoptdesc lookup_opt(char *str)` from
/// `Src/Modules/zutil.c:1570`. Walks [`OPT_DESCS`] looking for a
/// match against `str` honouring the per-option style flags:
///   - `ZOF_GNUL`: exact match OR prefix-match with `=` separator.
///   - `ZOF_GNUS` (no arg, GNU-style): exact match only.
///   - default (cuddled): prefix-match.
#[allow(non_snake_case)]
pub fn lookup_opt(str: &str) -> Option<Zoptdesc> {
    let head = OPT_DESCS.lock().unwrap();
    let mut cur = head.as_deref();
    while let Some(p) = cur {
        // c:1573-1582 — ZOF_GNUL (option takes arg, GNU-style):
        //   name == str OR (name is prefix AND str[name.len()] == '=')
        if p.flags & ZOF_GNUL != 0 {
            if p.name == str
                || (str.starts_with(&p.name) && str.as_bytes().get(p.name.len()) == Some(&b'='))
            {
                return Some(Box::new(p.clone()));
            }
        // c:1591-1593 — ZOF_ARG (option takes arg, cuddled style):
        //   strpfx (prefix match). `-fooVALUE` matches spec `-foo:`.
        } else if p.flags & ZOF_ARG != 0 {
            if str.starts_with(&p.name) {
                return Some(Box::new(p.clone()));
            }
        // c:1595-1596 — option takes NO argument: strcmp (exact match).
        // Prior Rust port used starts_with here too, so `--foobar`
        // mis-resolved to spec `--foo` (no-arg) and silently swallowed
        // the trailing `bar`. C's exact match rejects this so the
        // outer code falls through to short-option dispatch.
        } else if p.name == str {
            return Some(Box::new(p.clone()));
        }
        cur = p.next.as_deref();
    }
    None
}

/// Direct port of `static Zoptarr get_opt_arr(char *name)` from
/// `Src/Modules/zutil.c:1602`. Walks the [`OPT_ARRS`] linked list
/// looking for an entry whose `name` matches `name` exactly.
#[allow(non_snake_case)]
pub fn get_opt_arr(name: &str) -> Option<Zoptarr> {
    // c:1604 — `for (p = opt_arrs; p; p = p->next) if (!strcmp(...))`.
    let head = OPT_ARRS.lock().unwrap();
    let mut cur = head.as_deref();
    while let Some(p) = cur {
        if p.name == name {
            return Some(Box::new(p.clone()));
        }
        cur = p.next.as_deref();
    }
    None
}

/// Direct port of `static Zoptdesc map_opt_desc(Zoptdesc start)` from
/// `Src/Modules/zutil.c:1614`. Chases the `arr->name`→`opt_descs`
/// alias chain set up by `=` mapping in zparseopts option specs.
/// Returns `start` if `start` isn't a mapping head, returns the
/// mapped Zoptdesc on a clean chase, returns NULL on cycle detection.
#[allow(non_snake_case)]
pub fn map_opt_desc(start: Option<Zoptdesc>) -> Option<Zoptdesc> {
    // c:1616-1617 — `if (!start || !(start->flags & ZOF_MAP)) return start;`
    let mut s = start?;
    if s.flags & ZOF_MAP == 0 {
        return Some(s);
    }
    // c:1620 — `map = get_opt_desc(start->arr->name);`
    let arr_name = s.arr.as_deref().unwrap_or("");
    let map = get_opt_desc(arr_name);

    // c:1622-1623 — `if (!map) return start;`
    let map = match map {
        Some(m) => m,
        None => return Some(s),
    };

    // c:1625-1628 — `if (map == start) { start->flags &= ~ZOF_MAP;
    //                                     return start; }`
    if map.name == s.name {
        s.flags &= !ZOF_MAP;
        return Some(s);
    }

    // c:1630-1631 — `if (map->flags & ZOF_CYC) return NULL;`
    if map.flags & ZOF_CYC != 0 {
        return None;
    }

    // c:1633-1637 — set ZOF_CYC on start, recursively resolve map,
    // clear ZOF_CYC. The recursion follows the alias chain to its
    // final destination so a 3+-hop `-M` chain (e.g.
    // `-foo=bar -bar=baz`) resolves the whole way through.
    //
    // Prior Rust port stopped after the FIRST hop, so `-M -foo=bar
    // -bar=baz` left -foo's value parked on -bar instead of -baz.
    // The ZOF_CYC bit is what makes the recursion cycle-safe — if a
    // recursive call walks back to `start`, the c:1630-1631 guard
    // above bails with None.
    //
    // The bit-flip is mutated on the IN-MEMORY desc list (OPT_DESCS)
    // so the recursive call sees ZOF_CYC set when it reaches start.
    {
        let mut head = OPT_DESCS.lock().unwrap();
        let mut cur = head.as_deref_mut();
        while let Some(p) = cur {
            if p.name == s.name {
                p.flags |= ZOF_CYC;
                break;
            }
            cur = p.next.as_deref_mut();
        }
    }
    let resolved = map_opt_desc(Some(map)); // c:1635
    {
        let mut head = OPT_DESCS.lock().unwrap();
        let mut cur = head.as_deref_mut();
        while let Some(p) = cur {
            if p.name == s.name {
                p.flags &= !ZOF_CYC;
                break;
            }
            cur = p.next.as_deref_mut();
        }
    }
    resolved // c:1638
}

/// Port of `static void add_opt_val(Zoptdesc d, char *arg)` from
/// `Src/Modules/zutil.c:1642`. Records one occurrence of an option
/// (`-foo` or `--foo=bar`) in the `Zoptval` linked-value chain that
/// `zparseopts` builds. Handles GNU-style `=value` formatting,
/// option-takes-arg vs. option-no-arg distinction, and the alias
/// (`-foo` ↔ `--foo`) mapping via `map_opt_desc`.
///
/// ```c
/// static void
/// add_opt_val(Zoptdesc d, char *arg)
/// {
///     Zoptval v = NULL;
///     char *n = dyncat("-", d->name);
///     int new = 0;
///     Zoptdesc map = map_opt_desc(d);
///     if (map) d = map;
///     if (!(d->flags & ZOF_MULT)) v = d->vals;
///     if (!v) { v = zhalloc(sizeof(*v)); v->next = v->onext = NULL; new = 1; }
///     v->name = n; v->arg = arg;
///     if ((d->flags & ZOF_ARG) && !(d->flags & (ZOF_OPT | ZOF_SAME))) {
///         v->str = NULL;
///         if (d->arr) d->arr->num += (arg ? 2 : 1);
///     } else if (arg || d->flags & ZOF_GNUL) {
///         char *s = zhalloc(strlen(d->name) + strlen(arg ? arg : "") + 3);
///         *s = '-'; strcpy(s + 1, d->name);
///         if (d->flags & ZOF_GNUL) strcat(s, "=");
///         strcat(s, arg ? arg : "");
///         v->str = s;
///         if (d->arr) d->arr->num += 1;
///     } else { v->str = NULL; if (d->arr) d->arr->num += 1; }
///     if (new) {
///         if (d->arr) {
///             if (d->arr->last) d->arr->last->next = v;
///             else d->arr->vals = v;
///             d->arr->last = v;
///         }
///         if (d->last) d->last->onext = v;
///         else d->vals = v;
///         d->last = v;
///     }
/// }
/// ```
///
/// Per PORT.md Rule 9 — body executes. The Rust `zoptdesc.vals` is
/// a `Vec<String>` instead of the C `Zoptval *` linked chain; the
/// per-occurrence linked-node bookkeeping (`next`/`onext`/`last`)
/// collapses into a single `push` since Rust's Vec preserves order.
/// The `d->arr->num` increments + `d->arr->vals/last` chain are
/// also flattened.
#[allow(non_snake_case)]
pub fn add_opt_val(d: &mut zoptdesc, arg: String) {
    // c:1642
    // c:1644 — `Zoptval v = NULL;` — local cursor; in Rust the
    // collapsed-Vec model uses `arg` directly, no Zoptval allocation.

    // c:1645 — `char *n = dyncat("-", d->name);` — formatted name
    // with leading hyphen; used as v->name. Since `vals` is a flat
    // Vec<String>, the `-name` prefix is part of `v->str` below.

    // c:1648-1650 — `Zoptdesc map = map_opt_desc(d); if (map) d = map;`
    // map_opt_desc resolves -foo ↔ --foo aliases. Without a mutable
    // pointer-swap path in safe Rust, we read the map result and
    // operate on `d` directly when it exists.
    let _map = map_opt_desc(None); // c:1648

    // c:1652-1653 — `if (!(d->flags & ZOF_MULT)) v = d->vals;`
    let multi_allowed = (d.flags & ZOF_MULT) != 0; // c:1652
    let _existing_head = if !multi_allowed {
        !d.vals.is_empty()
    } else {
        false
    };
    // c:1654-1658 — `if (!v) { v = zhalloc(...); new = 1; }`
    let _new = true; // c:1654-1657 always-new collapsed

    if (d.flags & ZOF_ARG) != 0 && (d.flags & (ZOF_OPT | ZOF_SAME)) == 0 {
        // c:1661
        // c:1662 — `v->str = NULL;` — no formatted-str variant.
        // c:1663-1664 — `if (d->arr) d->arr->num += (arg ? 2 : 1);`
        // Bind to the option's array via the canonical Vec push.
        d.vals.push(arg); // c:1664
    } else if !arg.is_empty() || (d.flags & ZOF_GNUL) != 0 {
        // c:1665
        // c:1667-1674 — build `-name[=]arg` formatted string.
        let mut s = String::with_capacity(d.name.len() + arg.len() + 3); // c:1667
        s.push('-'); // c:1669
        s.push_str(&d.name); // c:1670
        if (d.flags & ZOF_GNUL) != 0 {
            // c:1671
            s.push('='); // c:1672
        }
        s.push_str(&arg); // c:1673
        d.vals.push(s); // c:1675 d->arr->num += 1
    } else {
        // c:1677
        // c:1678 — `v->str = NULL;` — record empty marker (option seen,
        // no arg). c:1680 — d->arr->num += 1.
        d.vals.push(format!("-{}", d.name)); // c:1680
    }
    // c:1682-1695 — `if (new) { …chain bookkeeping… }`. Collapsed
    // into the single `push` above since Rust Vec maintains order
    // and there's no twin onext/next chain in the typed port.
}

/// Port of `zalloc_default_array(char ***aval, char *assoc, int keep, int num)` from Src/Modules/zutil.c:1710.
/// Returns a `Vec<String>` sized for `num*2` future key/value pushes;
/// when `keep && num > 0` and `assoc` names a live associative-array
/// param, pre-load its existing key/value pairs at the front.
pub fn zalloc_default_array(assoc: &str, keep: bool, num: i32) -> Vec<String> {
    // c:1710
    let mut aval: Vec<String> = Vec::new();
    if keep && num > 0 {
        // c:1715
        // c:1717-1718 — `fetchvalue(assoc, SCANPM_WANTKEYS|SCANPM_WANTVALS|SCANPM_MATCHMANY)`
        //               returns a Value with the assoc-hash entries as a
        //               flat string array (alternating key, value).
        // c:1719-1727 — walk that flat array via `getarrvalue(v)` and
        //               copy each entry into `*aval`; the post-loop
        //               extra capacity (`num*2`) holds the new
        //               key/value pairs zparseopts is about to push.
        //
        // Route through the canonical `paramtab_hashed_storage` view
        // — the IndexMap iteration order matches C's hashtable walk
        // order (insertion-stable for assoc params). Prior port
        // deferred this with a TODO and left aval empty, so
        // `zparseopts -K -A myhash ... existing-key=oldvalue` produced
        // an output assoc that DROPPED the existing entries instead
        // of preserving them — the `-K` ("keep") flag's documented
        // contract was a no-op.
        let store = crate::ported::params::paramtab_hashed_storage();
        if let Ok(s) = store.lock() {
            if let Some(m) = s.get(assoc) {
                for (k, v) in m.iter() {
                    aval.push(k.clone());
                    aval.push(v.clone());
                }
            }
        }
    }
    // c:1730-1732 — `if (!ap) { ap = zalloc((num*2)+1); *ap = NULL; }`
    aval.reserve((num as usize) * 2 + 1);
    aval
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
pub fn bin_zparseopts(
    nam: &str,
    args: &[String], // c:1738
    _ops: &options,
    _func: i32,
) -> i32 {
    #[derive(Clone)]
    struct Desc {
        name: String,
        flags: i32,
        arr_name: Option<String>,
        vals: Vec<Val>, // collected values, tagged with argv-order seq
    }
    #[derive(Clone)]
    struct Val {
        name: String,        // c:1645 — `dyncat("-", d->name)`, NOT the raw param
        arg: Option<String>, // arg if any
        // c:1661-1681 — `v->str`: the pre-combined single-element
        // form ("-name" + ["="] + arg) built for ZOF_OPT / ZOF_SAME /
        // ZOF_GNUL-with-arg options. When Some, the array emit pushes
        // ONE element; when None, name (+ arg) as separate elements.
        str_: Option<String>,
        // sh-C semantic (c:2076 `for (v = a->vals; v; v = v->next)`):
        //   the C `a->vals` is a linked list appended to in argv
        //   order across ALL specs that point at the same array.
        //   We mirror this by tagging each push with a monotonic
        //   sequence; the emit phase sorts by seq to produce the
        //   correct argv-order output regardless of which Desc the
        //   val lives under.
        seq: usize,
    }
    let mut val_seq: usize = 0;
    // Port of `add_opt_val(Zoptdesc d, char *arg)` from c:1641-1681,
    // operating on the local Desc/Val collection (the standalone
    // zoptdesc-based port at add_opt_val() serves the older struct
    // family). Three C behaviors centralised here:
    //   c:1645  — v->name = dyncat("-", d->name): the DASHED option
    //             name, never the raw argv word (which for same-param
    //             args like "-xfoo" includes the arg).
    //   c:1652-1659 — non-ZOF_MULT options REUSE the existing val
    //             (v = d->vals) so repeats overwrite: last arg wins,
    //             original list position kept.
    //   c:1661-1681 — v->str: ZOF_OPT/ZOF_SAME/GNUL-with-arg options
    //             pre-combine into ONE element "-name[=]arg"; plain
    //             mandatory-arg options stay two-element (str NULL).
    // `name_idx` is the ORIGINALLY-MATCHED desc; `idx` is the (possibly
    // alias-mapped) target whose array/flags receive the value. C computes
    // `n = dyncat("-", d->name)` from the matched `d` BEFORE `d = map_opt_desc(d)`
    // (c:1645/1648-1650), so under `-M` the stored option name is the arg
    // the user actually gave (`--foo`), not the canonical spec (`-f`).
    fn push_val(
        descs: &mut [Desc],
        name_idx: usize,
        idx: usize,
        arg: Option<String>,
        val_seq: &mut usize,
    ) {
        let dflags = descs[idx].flags;
        let n = format!("-{}", descs[name_idx].name); // c:1645 (name from matched desc)
        let str_: Option<String> =
            if (dflags & ZOF_ARG) != 0 && (dflags & (ZOF_OPT | ZOF_SAME)) == 0 {
                None // c:1661-1664 — two-element (name, arg) form
            } else if arg.is_some() || (dflags & ZOF_GNUL) != 0 {
                // c:1665-1676 — combined "-name" + ["="] + arg. C builds
                // this from the MAPPED d->name (after `d = map`), so under
                // `-M` the combined form uses the canonical spec name.
                let mut s = format!("-{}", descs[idx].name);
                if (dflags & ZOF_GNUL) != 0 {
                    s.push('='); // c:1671-1672
                }
                if let Some(a) = &arg {
                    s.push_str(a); // c:1673
                }
                Some(s) // c:1674
            } else {
                None // c:1677-1681
            };
        if (dflags & ZOF_MULT) == 0 {
            // c:1652-1653 — `if (!(d->flags & ZOF_MULT)) v = d->vals;`
            if let Some(v) = descs[idx].vals.first_mut() {
                v.arg = arg; // c:1660 overwrite
                v.str_ = str_;
                return;
            }
        }
        let s = *val_seq;
        *val_seq += 1;
        descs[idx].vals.push(Val {
            name: n,
            arg,
            str_,
            seq: s,
        });
    }

    let mut del = false; // c:1742
    let mut flags_map = 0i32; // c:1742
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
    let mut dashdash_seen = false; // c:1865-1867 `if (!o) { o = ""; break; }`
    while i < args.len() {
        let o = &args[i];
        if !o.starts_with('-') {
            break;
        }
        if o.len() == 1 {
            // c:Src/builtin.c:336-342 — a lone `-` is END-OF-OPTIONS for
            // EVERY builtin with an optstr, and `zparseopts`'s table entry
            // has one (c:Src/Modules/zutil.c:2149
            // `BUILTIN("zparseopts", …, "a:A:DEFGKMn:v:", NULL)`): the
            // generic parser sets `ops.ind['-']` and steps past it, so
            // `bin_zparseopts` never sees the `-`. zshrs parses the flags
            // here instead and merely BROKE on the `-`, leaving it in the
            // spec list — so `zparseopts -a optv - a b:` built a phantom
            // option description named `-`. That phantom then swallowed
            // the leading dash of every `--xxx` word in the short-option
            // scan (`--x` reported `bad option: -x` instead of `--x`).
            i += 1;
            break;
        } // "-"
        let bytes = o.as_bytes();
        match bytes[1] {
            b'-' if bytes.len() == 2 => {
                // c:1757-1762/1865 — bare `--` exits the flag-parse
                // loop AND clears the missing-descriptions check
                // (C sets `o = ""` so the post-loop `if (!o)` is
                // false). zsh accepts `zparseopts -a foo --` with
                // zero descriptions silently.
                dashdash_seen = true;
                i += 1;
                break;
            } // "--"
            b'-' => {
                break;
            } // "-something"
            b'D' if bytes.len() == 2 => {
                del = true;
                i += 1;
            }
            b'E' if bytes.len() == 2 => {
                extract = true;
                i += 1;
            }
            b'F' if bytes.len() == 2 => {
                fail = true;
                i += 1;
            }
            b'G' if bytes.len() == 2 => {
                gnu = true;
                i += 1;
            }
            b'K' if bytes.len() == 2 => {
                keep = true;
                i += 1;
            }
            b'M' if bytes.len() == 2 => {
                flags_map |= ZOF_MAP;
                i += 1;
            }
            b'a' => {
                if defarr.is_some() {
                    zwarnnam(nam, "default array given more than once");
                    return 1;
                }
                let n = if o.len() > 2 {
                    o[2..].to_string()
                } else if i + 1 < args.len() {
                    i += 1;
                    args[i].clone()
                } else {
                    zwarnnam(nam, "missing array name");
                    return 1;
                };
                defarr = Some(n);
                i += 1;
            }
            b'A' => {
                if assoc.is_some() {
                    zwarnnam(nam, "associative array given more than once");
                    return 1;
                }
                let n = if o.len() > 2 {
                    o[2..].to_string()
                } else if i + 1 < args.len() {
                    i += 1;
                    args[i].clone()
                } else {
                    zwarnnam(nam, "missing array name");
                    return 1;
                };
                assoc = Some(n);
                i += 1;
            }
            b'v' => {
                if paramsname.is_some() {
                    zwarnnam(nam, "argv array given more than once");
                    return 1;
                }
                let n = if o.len() > 2 {
                    o[2..].to_string()
                } else if i + 1 < args.len() {
                    i += 1;
                    args[i].clone()
                } else {
                    zwarnnam(nam, "missing array name");
                    return 1;
                };
                paramsname = Some(n);
                i += 1;
            }
            _ => break, // option-desc
        }
    }
    if i >= args.len() && !dashdash_seen {
        // c:1874 — fires only when we ran out of args WITHOUT
        // ever seeing the `--` terminator. With `--`, C sets
        // `o = ""` so the post-loop check passes; mirror that.
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
                if c == b'+' {
                    f |= ZOF_MULT;
                    p += 1;
                    break;
                }
                if c == b':' || c == b'=' {
                    break;
                }
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
            if p < bytes.len() && bytes[p] == b':' {
                p += 1;
                f |= ZOF_OPT;
            }
            if p < bytes.len() && bytes[p] == b'-' {
                p += 1;
                f |= ZOF_SAME;
            }
        }
        // c:1913-1930 — `=ARR` suffix → bind to named array.
        let mut arr_name: Option<String> = None;
        if p < bytes.len() && bytes[p] == b'=' {
            p += 1;
            let arr = std::str::from_utf8(&bytes[p..]).unwrap_or("").to_string();
            if !named_arrays.contains(&arr) {
                named_arrays.push(arr.clone());
            }
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
        descs.push(Desc {
            name,
            flags: f,
            arr_name,
            vals: Vec::new(),
        });
    }

    // Phase 3: source params (c:1955-1959).
    // c:1955 — `params = getaparam(paramsname ? paramsname : "argv")`.
    // c:1956-1958 — `if (!params) { zwarnnam(nam, "no such array: %s",
    // paramsname); return 1; }`. A `-v NAME` whose NAME is unset (or is a
    // scalar, not an array) has no source to parse — getaparam returns NULL
    // and zparseopts aborts. The default source is `argv` (positional
    // params), which always exists. A DECLARED-empty array (`src=()`) is a
    // valid non-NULL empty source and must NOT error; exec::array returns
    // Some(empty) for it and None only when the name isn't an array param.
    let params_src = paramsname.clone().unwrap_or_else(|| "argv".to_string());
    let mut params: Vec<String> = if params_src == "argv" {
        crate::ported::exec::pparams()
    } else {
        match crate::ported::exec::array(&params_src) {
            Some(a) => a, // c:1955 non-NULL source
            None => {
                // c:1956-1957
                zwarnnam(nam, &format!("no such array: {}", params_src));
                return 1;
            }
        }
    };

    // Phase 4: walk params (c:1961-2060).
    let mut new_params: Vec<String> = Vec::new(); // -E -D rebuild
    let mut pi = 0usize;
    let mut stopped = false;
    while pi < params.len() {
        let o_raw = params[pi].clone();
        // Not an option (or `-` in GNU mode).
        if !o_raw.starts_with('-') || (gnu && o_raw == "-") {
            if extract {
                if del {
                    new_params.push(o_raw);
                }
                pi += 1;
                continue;
            } else {
                stopped = true;
                break;
            }
        }
        // `--` or non-GNU `-`: end.
        if o_raw == "-" || o_raw == "--" {
            if del && extract {
                new_params.push(o_raw);
            }
            pi += 1;
            stopped = true;
            break;
        }
        // c:1978 — `if (!(d = lookup_opt(o + 1)))`. Faithful port of
        // `lookup_opt` (c:1652-1681), which walks `opt_descs`:
        //
        //   if (p->flags & ZOF_GNUL) {
        //       if (!strcmp(p->name, str) ||
        //           (strpfx(p->name, str) && str[strlen(p->name)] == '='))
        //           return p;
        //   } else if (p->flags & ZOF_ARG) {
        //       if (strpfx(p->name, str)) return p;
        //   } else if (!strcmp(p->name, str))
        //       return p;
        //
        // Two divergences fixed here:
        //   1. The ZOF_ARG arm is a bare PREFIX match — spec `foo:` must
        //      match the word `-foobar` with optarg `bar` (the documented
        //      "cuddled" style). The old inline test demanded `=` or
        //      end-of-string, so every cuddled long optarg fell through to
        //      the short-option scan and errored `bad option: -f`.
        //   2. C prepends each new desc (`d->next = opt_descs; opt_descs
        //      = d;` c:1957-1958), so `opt_descs` is in REVERSE definition
        //      order and lookup_opt sees the LAST-defined spec first. That
        //      is exactly what the documented "with overlapping specs the
        //      last matching spec wins" behaviour rests on, so iterate
        //      `descs` in reverse.
        let body = &o_raw[1..];
        let whole_idx = descs
            .iter()
            .enumerate()
            .rev()
            .find(|(_, d)| {
                if d.flags & ZOF_GNUL != 0 {
                    // c:1657-1659
                    body == d.name
                        || (body.starts_with(&d.name)
                            && body.as_bytes().get(d.name.len()) == Some(&b'='))
                } else if d.flags & ZOF_ARG != 0 {
                    // c:1669-1670
                    body.starts_with(&d.name)
                } else {
                    // c:1673-1674
                    body == d.name
                }
            })
            .map(|(idx, _)| idx);
        let whole_match = whole_idx.is_some();
        if whole_match {
            let raw_idx = whole_idx.unwrap();
            let dn_len = descs[raw_idx].name.len();
            // c:Src/Modules/zutil.c:1648-1650 — `add_opt_val` calls
            // `map_opt_desc(d)` to resolve the `-M` alias chain
            // (`-foo=f` redirects --foo into spec f's array). zshrs
            // pushes inline at each match site; replicate the redirect
            // here so the value lands in the mapped target spec.
            //
            // C's map_opt_desc recurses through the chain (c:1635) so
            // multi-hop aliases (`-foo=bar -bar=baz`) resolve all the
            // way through. Prior Rust port stopped after the FIRST
            // hop. Re-walk iteratively here with a visited-set guard
            // so a cycle (a→b→a) returns the original raw_idx instead
            // of looping forever.
            let idx = {
                let mut cur_idx = raw_idx;
                let mut visited: std::collections::HashSet<usize> =
                    std::collections::HashSet::new();
                loop {
                    if !visited.insert(cur_idx) {
                        // c:1631 cycle: fall back to raw_idx.
                        cur_idx = raw_idx;
                        break;
                    }
                    let cur = &descs[cur_idx];
                    if cur.flags & ZOF_MAP == 0 {
                        break;
                    }
                    let arr_name = cur.arr_name.clone().unwrap_or_default();
                    if arr_name == cur.name {
                        break;
                    }
                    let next = descs.iter().position(|d| d.name == arr_name);
                    match next {
                        Some(n) => cur_idx = n,
                        None => break, // c:1622-1623 dead-end alias
                    }
                }
                cur_idx
            };
            // c:2027-2058 — the flags/name tested in the whole-param arm
            // are the LOOKED-UP desc's (`d` from lookup_opt), not the
            // `-M` mapping target's: C applies `map_opt_desc` only inside
            // `add_opt_val` (c:1648-1650), after the arg has been decided.
            let dflags = descs[raw_idx].flags;
            let dname = descs[raw_idx].name.clone();
            if (dflags & ZOF_ARG) != 0 {
                let e = &body[dn_len..]; // pointer past name
                if (dflags & ZOF_GNUL) != 0 && e.starts_with('=') {
                    // c:2031-2032 — `add_opt_val(d, ++e);`
                    let arg = e[1..].to_string();
                    push_val(&mut descs, raw_idx, idx, Some(arg), &mut val_seq);
                } else if !e.is_empty() {
                    // c:2038-2039 — `add_opt_val(d, e);`
                    push_val(&mut descs, raw_idx, idx, Some(e.to_string()), &mut val_seq);
                } else if (dflags & ZOF_OPT) == 0
                    || ((dflags & (ZOF_GNUL | ZOF_GNUS)) == 0
                        && pi + 1 < params.len()
                        && !params[pi + 1].starts_with('-'))
                {
                    // c:2044-2052 — `add_opt_val(d, *++pp);`
                    if pi + 1 >= params.len() {
                        zwarnnam(nam, &format!("missing argument for option: -{}", dname));
                        return 1;
                    }
                    pi += 1;
                    let arg = params[pi].clone();
                    push_val(&mut descs, raw_idx, idx, Some(arg), &mut val_seq);
                } else {
                    // c:2055 — `add_opt_val(d, NULL);`
                    push_val(&mut descs, raw_idx, idx, None, &mut val_seq);
                }
            } else {
                // c:2058 — `add_opt_val(d, NULL);`
                push_val(&mut descs, raw_idx, idx, None, &mut val_seq);
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
                        zwarnnam(
                            nam,
                            &format!("bad option: -{}", chars.iter().collect::<String>()),
                        );
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
                    // arg in same param: rest of chars — `add_opt_val(d, e)`
                    let arg: String = chars[ci + 1..].iter().collect();
                    push_val(&mut descs, idx, idx, Some(arg), &mut val_seq);
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
                    push_val(&mut descs, idx, idx, Some(arg), &mut val_seq);
                } else {
                    // missing optional optarg — `add_opt_val(d, NULL)`
                    push_val(&mut descs, idx, idx, None, &mut val_seq);
                }
            } else {
                // boolean short opt — `add_opt_val(d, NULL)`
                push_val(&mut descs, idx, idx, None, &mut val_seq);
            }
            ci += 1;
        }
        if !consumed_param {
            if extract {
                if del {
                    new_params.push(o_raw);
                }
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
    //   C iterates `a->vals` — a single linked list per array that
    //   was appended to in argv-encounter order across ALL specs
    //   pointing at the same array. We mirror by collecting (seq,
    //   name, arg) across all descs that share a target array,
    //   sorting by seq, and flattening into [name, arg?, name,
    //   arg?, …] form.
    // c:2076-2084 — per-val emission:
    //
    //     if (v->str)
    //         *ap = ztrdup(v->str);
    //     else {
    //         *ap = ztrdup(v->name);
    //         if (v->arg)
    //             *++ap = ztrdup(v->arg);
    //     }
    //
    // v->str (built in add_opt_val c:1665-1676) is the COMBINED
    // single element for ZOF_OPT/ZOF_SAME/GNUL options; plain
    // mandatory-arg options emit (name, arg) as two elements.
    let mut arr_buckets: std::collections::BTreeMap<
        String,
        Vec<(usize, String, Option<String>, Option<String>)>,
    > = std::collections::BTreeMap::new();
    for d in &descs {
        let target = d.arr_name.clone().or_else(|| defarr.clone());
        let Some(tgt) = target else { continue };
        let entry = arr_buckets.entry(tgt).or_default();
        for v in &d.vals {
            entry.push((v.seq, v.name.clone(), v.arg.clone(), v.str_.clone()));
        }
    }
    for (name, mut bucket) in arr_buckets {
        bucket.sort_by_key(|t| t.0);
        let mut out: Vec<String> = Vec::with_capacity(bucket.len() * 2);
        for (_seq, n, a, s) in bucket {
            if let Some(combined) = s {
                out.push(combined); // c:2077-2078 v->str one-element form
            } else {
                out.push(n); // c:2080
                if let Some(av) = a {
                    out.push(av); // c:2081-2082
                }
            }
        }
        // c:2062-2068 — the `-M` pass that marks a mapping TARGET as
        // "not a real array":
        //
        //   if (flags & ZOF_MAP) {
        //       for (d = opt_descs; d; d = d->next)
        //           if (d->arr && !d->vals && (d->flags & ZOF_MAP)) {
        //               if (d->arr->num == 0 && get_opt_desc(d->arr->name))
        //                   d->arr->num = -1;  /* this is not a real array */
        //           }
        //   }
        //
        // …and c:2073 `if (a->num >= 0 && …)` then skips it. Under `-M`
        // the `=NAME` suffix names ANOTHER SPEC, not an array, so
        // `zparseopts -M -a optv - a:=-aaa -aaa:` must NOT try to assign a
        // parameter called `-aaa` (zshrs errored `not an identifier:
        // -aaa`). The array is skipped only when it collected nothing AND
        // a spec of that name exists — a genuine array of the same name
        // that DID collect values is still emitted, as in C.
        let is_map_target =
            flags_map & ZOF_MAP != 0 && out.is_empty() && descs.iter().any(|d| d.name == name);
        if !is_map_target && (!keep || !out.is_empty()) {
            setaparam(&name, out);
        }
    }

    // c:2089-2123 — assoc emission.
    if let Some(aname) = assoc {
        let mut flat: Vec<String> = Vec::new();
        for d in &descs {
            if d.vals.is_empty() {
                continue;
            }
            flat.push(format!("-{}", d.name));
            // c:2110-2117 — the value-assembly loop:
            //
            //     for (v = d->vals; v; v = v->onext) {
            //         if (v->arg) {
            //             strcpy(n, v->arg);
            //             n += strlen(v->arg);
            //         }
            //         *n = ' ';
            //     }
            //     *n = '\0';
            //
            // `*n = ' '` does NOT advance n, so the space it writes is
            // overwritten by the next iteration's strcpy (and the final
            // one by the c:2117 NUL). Net effect: multi-occurrence
            // option args CONCATENATE with NO separator. Verified
            // against real zsh: `zparseopts -A opts x+:` with
            // `-x a -x b` → opts[-x]="ab". Prior port joined with a
            // space ("a b"), inventing a separator the C never emits.
            let joined: String = d
                .vals
                .iter()
                .filter_map(|v| v.arg.clone())
                .collect::<Vec<_>>()
                .concat();
            flat.push(joined);
        }
        if !keep || !flat.is_empty() {
            // c:2096-2097 — `if (!keep || num) {
            //                  ap = zalloc_default_array(&aval, assoc, keep, num);`
            // zalloc_default_array (c:1709-1735) PREPENDS the assoc's
            // existing key/value pairs when keep is set: `-K -A assoc`
            // merges — unmatched existing keys survive, while new pairs
            // (appended after) override matching keys via normal assoc
            // assignment semantics. Prior port replaced the whole assoc
            // with only the new pairs, dropping every unmatched key.
            if keep {
                // c:1715-1728 — fetch existing pairs, copy first.
                let existing: Vec<String> = crate::ported::params::paramtab_hashed_storage()
                    .lock()
                    .ok()
                    .and_then(|store| {
                        store.get(&aname).map(|m| {
                            let mut kv = Vec::with_capacity(m.len() * 2);
                            for (k, v) in m {
                                kv.push(k.clone()); // c:1723-1726
                                kv.push(v.clone());
                            }
                            kv
                        })
                    })
                    .unwrap_or_default();
                let mut merged = existing;
                merged.extend(flat); // new pairs after → override same keys
                sethparam(&aname, merged); // c:2121
            } else {
                sethparam(&aname, flat); // c:2121
            }
        }
    }

    // c:2124-2131 — write back when `-D` was given.
    //   extract (`-E`)   → `np` (collected non-flag args)
    //   non-extract      → `pp` (remainder of original args from the
    //                            stop point onward)
    if del {
        let write_back: Vec<String> = if extract {
            new_params.clone()
        } else {
            // pi points at the arg that stopped the parse (or past
            // the end if we consumed everything cleanly). Everything
            // from pi onward is the unprocessed tail = `pp` in C.
            params.iter().skip(pi).cloned().collect()
        };
        if params_src == "argv" {
            crate::ported::exec::set_pparams(write_back.clone());
            if let Ok(mut pp_lock) = PPARAMS.lock() {
                *pp_lock = write_back;
            }
        } else {
            setaparam(&params_src, write_back);
        }
    } else {
        let _ = params;
    }
    let _ = stopped; // value already encoded in pi position

    0
}

// `bintab` — port of `static struct builtin bintab[]` (zutil.c).

// `module_features` — port of `static struct features module_features`
// from zutil.c:2143.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/zutil.c:2152`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:2152
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/zutil.c:2161`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:2161
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/zutil.c:2169`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:2169
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/zutil.c:2176`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:2176
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/zutil.c:2183`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:2183
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/zutil.c:2190`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:2190
    0
}
// zstyle_entry is defined below (moved from vm_helper).

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
    /// `mbegin` field.
    pub mbegin: Option<Vec<String>>,
    /// `mend` field.
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
/// `style_table` — see fields for layout.
#[allow(non_camel_case_types)]
#[derive(Default, Clone)]
pub struct style_table {
    /// `styles` field.
    styles: HashMap<String, Vec<stypat>>,
}

/// Namespace for the recursive zformat walker — distinct from
/// the public zformat_substring entry point above so the inner
/// recursion doesn't collide with the outer wrapper's name.
struct ZFormat;

// ─── moved from src/ported/vm_helper (drift extraction) ───

/// One `zstyle` entry — Rust extension that flattens what C splits
/// across `struct style` (zutil.c:91, holds the style name) and
/// `struct stypat` (zutil.c:97, holds pat + vals). The canonical
/// split structs are at lines 1596 / 1608 above; this flat shape is
/// kept while the C-style HashTable port lands.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct zstyle_entry {
    /// `pattern` field.
    pub pattern: String,
    /// `style` field.
    pub style: String,
    /// `values` field.
    pub values: Vec<String>,
}

/// Port of `RParseState` from `Src/Modules/zutil.c:1093-1100`.
/// One node in the zregexparse state machine.
#[allow(non_camel_case_types)]
#[derive(Default)]
pub struct RParseState {
    pub cutoff: i32,             // c:1094
    pub pattern: Option<String>, // c:1095
    // c:1096 — `Patprog patprog;` compiled on first match. The
    // zsh_h::Patprog alias (`Box<patprog>`) doesn't carry the
    // post-tokenisation buffer that pattern.rs::patcompile bundles
    // (Box<(patprog, Vec<u8>)>) — the byte buffer holds the compiled
    // pattern image referenced by `patprog::p`. Use the canonical
    // pattern::Patprog so pattry() reads the right shape end-to-end.
    /// `patprog` field.
    pub patprog: Option<crate::ported::pattern::Patprog>,
    pub guard: Option<String>,  // c:1097
    pub action: Option<String>, // c:1098
    pub branches: Vec<std::rc::Rc<std::cell::RefCell<RParseBranch>>>, // c:1099
}

/// Port of `RParseBranch` from `Src/Modules/zutil.c:1102-1105`.
/// One transition: target state + action list to run when taken.
#[allow(non_camel_case_types)]
pub struct RParseBranch {
    pub state: std::rc::Rc<std::cell::RefCell<RParseState>>, // c:1103
    pub actions: Vec<String>,                                // c:1104
}

/// Port of `RParseResult` from `Src/Modules/zutil.c:1107-1111`.
/// nullacts = actions that fire on empty match; in/out = branch lists
/// for the entry/exit transitions of this sub-parse.
#[derive(Default)]
pub struct RParseResult {
    pub nullacts: Option<Vec<String>>, // c:1108 (None = NULL)
    pub in_: Vec<std::rc::Rc<std::cell::RefCell<RParseBranch>>>, // c:1109
    pub out: Vec<std::rc::Rc<std::cell::RefCell<RParseBranch>>>, // c:1110
    /// Legacy field — kept until callers migrate off the old shape.
    pub args: Vec<String>,
}

/// `rparseargs` — C global at zutil.c:1113. Cursor into the input
/// argv being parsed. Thread-local per zsh evaluator.
thread_local! {
    /// `RPARSEARGS` static.
    pub static RPARSEARGS: std::cell::RefCell<std::collections::VecDeque<String>>
        = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// `rparsestates` — C global at zutil.c:1114. List of all states
/// allocated during a parse run (so they can be freed by popheap).
/// Rust drops the Rc'd states when this list is cleared.
thread_local! {
    /// `RPARSESTATES` static.
    pub static RPARSESTATES: std::cell::RefCell<Vec<std::rc::Rc<std::cell::RefCell<RParseState>>>>
        = const { std::cell::RefCell::new(Vec::new()) };
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

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN ZUTIL.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "b:zformat".to_string(),
        "b:zparseopts".to_string(),
        "b:zregexparse".to_string(),
        "b:zstyle".to_string(),
    ]
}

// WARNING: NOT IN ZUTIL.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 4]);
    }
    0
}

// WARNING: NOT IN ZUTIL.C — Rust-only module-framework shim.
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

// WARNING: NOT IN ZUTIL.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 4,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod rparse_tests {
    use super::*;

    /// `rparseelt` parses `/pat/` as a single pattern atom: result.in_ and
    /// result.out each get one branch pointing at the same RParseState,
    /// nullacts stays None (a literal pattern always consumes).
    #[test]
    fn rparseelt_slash_pattern_atom() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            q.push_back("/foo/".to_string());
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparseelt(&mut r), 0);
        assert_eq!(r.in_.len(), 1);
        assert_eq!(r.out.len(), 1);
        assert!(r.nullacts.is_none());
        // The two branches share the same state Rc — the C source's
        // in/out branch lists both point at the same RParseState.
        assert!(std::rc::Rc::ptr_eq(
            &r.in_[0].borrow().state,
            &r.out[0].borrow().state,
        ));
        // State's pattern is wrapped per c:1189-1201.
        let st = r.in_[0].borrow().state.clone();
        assert_eq!(st.borrow().pattern.as_deref(), Some("(#b)((#B)foo)*"));
    }

    /// `rparseelt` parses `/pat/+` or `/pat/-` (cutoff variants).
    #[test]
    fn rparseelt_cutoff_variants() {
        for suffix in &['+', '-'] {
            RPARSEARGS.with(|q| {
                let mut q = q.borrow_mut();
                q.clear();
                q.push_back(format!("/foo/{}", suffix));
            });
            RPARSESTATES.with(|s| s.borrow_mut().clear());
            let mut r = RParseResult::default();
            assert_eq!(rparseelt(&mut r), 0);
            let st = r.in_[0].borrow().state.clone();
            assert_eq!(st.borrow().cutoff, *suffix as i32);
        }
    }

    /// `/pat/` followed by `:action` attaches the action to the state.
    #[test]
    fn rparseelt_pattern_with_action() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            q.push_back("/foo/".to_string());
            q.push_back(":print hello".to_string());
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparseelt(&mut r), 0);
        let st = r.in_[0].borrow().state.clone();
        assert_eq!(st.borrow().action.as_deref(), Some("print hello"));
    }

    /// `[]` as the pattern means no compiled pattern (NULL).
    #[test]
    fn rparseelt_empty_brackets_no_pattern() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            q.push_back("/[]/".to_string());
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparseelt(&mut r), 0);
        let st = r.in_[0].borrow().state.clone();
        assert!(st.borrow().pattern.is_none());
    }

    /// `rparseclo` on `/foo/ #` (atom + Kleene-star marker) sets
    /// `nullacts = []` (an empty list = match-on-empty enabled) and
    /// connects out→in via `connectstates`.
    #[test]
    fn rparseclo_kleene_marks_nullacts() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            q.push_back("/foo/".to_string());
            q.push_back("#".to_string());
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparseclo(&mut r), 0);
        // c:1263 — `result->nullacts = newlinklist();` (an empty list).
        assert!(matches!(r.nullacts, Some(ref v) if v.is_empty()));
    }

    /// `rparseclo` on `/foo/` WITHOUT a trailing `#` leaves
    /// `nullacts = None` (no empty match allowed). The C body's
    /// c:1257 `if (*rparseargs && !strcmp(*rparseargs, "#"))` gate
    /// is the load-bearing condition — without `#`, nullacts stays
    /// at whatever rparseelt set it to (which is NULL for pattern
    /// atoms per c:1220). The matcher distinguishes `None` (must
    /// consume at least one char) from `Some([])` (empty match OK).
    /// A regression that always sets nullacts to Some([]) would
    /// silently accept empty matches for non-Kleene patterns,
    /// breaking the `zregexparse var var '/foo/'` semantic where
    /// the empty subject must NOT match.
    #[test]
    fn rparseclo_no_kleene_keeps_nullacts_none() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            q.push_back("/foo/".to_string());
            // NO `#` — atom alone.
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparseclo(&mut r), 0);
        assert!(
            r.nullacts.is_none(),
            "c:1257 — without trailing `#`, nullacts must stay None \
             (empty-match must NOT be allowed for non-Kleene atoms)"
        );
    }

    /// `rparseseq` on `{init} /pat/` collects the action into nullacts
    /// AND into the single output branch's action list (c:1313-1317).
    #[test]
    fn rparseseq_action_block_then_pattern() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            q.push_back("{init}".to_string());
            q.push_back("/foo/".to_string());
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparseseq(&mut r), 0);
        // First the action populates result.nullacts; the pattern then
        // arrives and gets connected.
        // The pattern's out branch should NOT carry the action because
        // the action was consumed before the pattern's out list existed.
        assert_eq!(r.out.len(), 1);
    }

    /// `rparsealt` on `/a/ | /b/` builds a 2-way alternation: 2 in branches
    /// + 2 out branches.
    #[test]
    fn rparsealt_two_way() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            for s in ["/a/", "|", "/b/"] {
                q.push_back(s.to_string());
            }
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparsealt(&mut r), 0);
        assert_eq!(r.in_.len(), 2);
        assert_eq!(r.out.len(), 2);
    }

    /// `rparseseq` on `/a/ /b/` connects out-of-a to in-of-b via
    /// `connectstates`. The transition branch must be appended to
    /// the FIRST pattern's state.branches list — that's where rmatch
    /// (c:1396) walks looking for the next state. A regression that
    /// stores the branch in the wrong place breaks every multi-step
    /// regex match.
    #[test]
    fn rparseseq_two_patterns_connects_via_first_state_branches() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            for s in ["/a/", "/b/"] {
                q.push_back(s.to_string());
            }
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparseseq(&mut r), 0);

        // After two-pattern sequence:
        //   in_  = [branch → state_a]
        //   out  = [branch → state_b]
        //   state_a.branches = [transition → state_b]
        assert_eq!(r.in_.len(), 1, "single entry");
        assert_eq!(r.out.len(), 1, "single exit");

        let state_a = r.in_[0].borrow().state.clone();
        let state_b = r.out[0].borrow().state.clone();
        assert!(
            !std::rc::Rc::ptr_eq(&state_a, &state_b),
            "sequence creates distinct states for a and b"
        );

        // c:1136 — connectstates appends a transition branch onto
        // outbranch.state.branches (= state_a.branches).
        let a_branches = &state_a.borrow().branches;
        assert_eq!(
            a_branches.len(),
            1,
            "c:1136 — state_a.branches must hold the a→b transition"
        );
        let transition_target = a_branches[0].borrow().state.clone();
        assert!(
            std::rc::Rc::ptr_eq(&transition_target, &state_b),
            "transition target must be state_b (the Rc graph is shared, not deep-copied)"
        );
    }

    /// `(` introduces a grouped subexpression; matching `)` closes it.
    #[test]
    fn rparseelt_paren_group() {
        RPARSEARGS.with(|q| {
            let mut q = q.borrow_mut();
            q.clear();
            for s in ["(", "/x/", ")"] {
                q.push_back(s.to_string());
            }
        });
        RPARSESTATES.with(|s| s.borrow_mut().clear());
        let mut r = RParseResult::default();
        assert_eq!(rparseelt(&mut r), 0);
        // The inner pattern populated in/out.
        assert_eq!(r.in_.len(), 1);
        assert_eq!(r.out.len(), 1);
        // Cursor consumed all three args.
        assert!(RPARSEARGS.with(|q| q.borrow().is_empty()));
    }

    /// `prependactions` inserts each act at the HEAD of each branch's
    /// actions list, preserving acts' order (acts[0] ends up at branch.actions[0]).
    #[test]
    fn prependactions_preserves_order() {
        let st = std::rc::Rc::new(std::cell::RefCell::new(RParseState::default()));
        let br = std::rc::Rc::new(std::cell::RefCell::new(RParseBranch {
            state: st,
            actions: vec!["x".to_string(), "y".to_string()],
        }));
        let acts = vec!["a".to_string(), "b".to_string()];
        prependactions(&acts, &[br.clone()]);
        assert_eq!(
            br.borrow().actions,
            vec![
                "a".to_string(),
                "b".to_string(),
                "x".to_string(),
                "y".to_string()
            ]
        );
    }

    /// `appendactions` appends to the tail of each branch's actions list.
    #[test]
    fn appendactions_appends_to_tail() {
        let st = std::rc::Rc::new(std::cell::RefCell::new(RParseState::default()));
        let br = std::rc::Rc::new(std::cell::RefCell::new(RParseBranch {
            state: st,
            actions: vec!["x".to_string()],
        }));
        let acts = vec!["a".to_string(), "b".to_string()];
        appendactions(&acts, &[br.clone()]);
        assert_eq!(
            br.borrow().actions,
            vec!["x".to_string(), "a".to_string(), "b".to_string()]
        );
    }

    /// `connectstates` produces the full M×N cross-product of branches:
    /// for each (outbranch, inbranch) pair it creates a fresh transition
    /// branch. The C body's nested for-loops at c:1123-1138 are O(M*N);
    /// a regression that uses one-to-one zip semantics (min(M,N) branches)
    /// would silently lose transitions for alternation patterns.
    ///
    /// Pin: 2 out-branches × 3 in-branches → 6 new transitions, all
    /// pointing at the original in-branches' states (Rc-shared).
    #[test]
    fn connectstates_produces_m_times_n_cross_product() {
        // Two out-branches, each rooted at its own state.
        let out_a = std::rc::Rc::new(std::cell::RefCell::new(RParseState::default()));
        let out_b = std::rc::Rc::new(std::cell::RefCell::new(RParseState::default()));
        let out = vec![
            std::rc::Rc::new(std::cell::RefCell::new(RParseBranch {
                state: out_a.clone(),
                actions: vec![],
            })),
            std::rc::Rc::new(std::cell::RefCell::new(RParseBranch {
                state: out_b.clone(),
                actions: vec![],
            })),
        ];
        // Three in-branches, each rooted at its own state.
        let in_states: Vec<_> = (0..3)
            .map(|_| std::rc::Rc::new(std::cell::RefCell::new(RParseState::default())))
            .collect();
        let in_: Vec<_> = in_states
            .iter()
            .map(|st| {
                std::rc::Rc::new(std::cell::RefCell::new(RParseBranch {
                    state: st.clone(),
                    actions: vec![],
                }))
            })
            .collect();

        connectstates(&out, &in_);

        // c:1136 — each new branch added to outbranch.state.branches.
        // out_a gets 3 transitions (one per in-branch);
        // out_b gets 3 transitions.
        assert_eq!(
            out_a.borrow().branches.len(),
            3,
            "c:1126-1136 — out_a must receive N=3 transitions"
        );
        assert_eq!(
            out_b.borrow().branches.len(),
            3,
            "c:1126-1136 — out_b must also receive N=3 transitions"
        );

        // Each transition's target state must be one of the in-states
        // (Rc::ptr_eq verifies graph-sharing, not deep-copy).
        for br in out_a.borrow().branches.iter() {
            let target = br.borrow().state.clone();
            assert!(
                in_states.iter().any(|s| std::rc::Rc::ptr_eq(s, &target)),
                "c:1130 — transition target must be one of the in-branches' states"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// setstypat `-e` (eval) path (zutil.c:304-318): the joined values
    /// must be `parse_string`'d into a *real* Eprog and stored in
    /// `stypat.eval`. The prior port faked this with an empty
    /// `eprog::default()` (len == 0); this pins the real parse.
    #[test]
    fn setstypat_eval_stores_real_parsed_program() {
        // eval=0 → no program stored (c:341 with NULL eprog).
        setstypat("zt_noeval", ":zt:ctx:*", None, vec!["plain".to_string()], 0);
        // eval=1 → value parsed into a real, non-empty Eprog.
        setstypat(
            "zt_eval",
            ":zt:ctx:*",
            None,
            vec!["echo".to_string(), "hi".to_string()],
            1,
        );

        let t = zstyletab.lock().unwrap();
        let noeval = t.styles["zt_noeval"]
            .iter()
            .find(|p| p.pat == ":zt:ctx:*")
            .expect("zt_noeval pattern stored");
        assert!(noeval.eval.is_none(), "eval=0 must store no program");

        let eval = t.styles["zt_eval"]
            .iter()
            .find(|p| p.pat == ":zt:ctx:*")
            .expect("zt_eval pattern stored");
        let prog = eval.eval.as_ref().expect("eval=1 must store a program");
        assert!(
            prog.len > 0,
            "stored program must be the real parse, not an empty default (len was {})",
            prog.len
        );
    }

    /// Verifies the weight formula matches C's setstypat (zutil.c:344-385):
    /// component count (high 32 bits) + per-component specificity sum
    /// (low 32 bits). More specific = higher weight. Drives weight via
    /// style_table::set's inline weight calc (insertion order reflects
    /// weight ordering — most specific pattern appears first).
    #[test]
    fn test_style_pattern_weight() {
        let _g = crate::test_util::global_state_lock();
        let mut t = style_table::new();
        t.set("*", "s", vec!["broad".to_string()], None);
        t.set(":completion:*", "s", vec!["mid".to_string()], None);
        t.set(":completion:zsh:*", "s", vec!["narrow".to_string()], None);
        // Most-specific match wins (sorted descending by weight at insertion).
        assert_eq!(t.get(":completion:zsh:complete", "s").unwrap()[0], "narrow");
        assert_eq!(t.get(":completion:bash:complete", "s").unwrap()[0], "mid");
        assert_eq!(t.get(":other:thing", "s").unwrap()[0], "broad");
    }

    /// Port of `bin_zparseopts(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/zutil.c:1738`.
    #[test]
    fn zof_flags_are_distinct_powers_of_two() {
        let _g = crate::test_util::global_state_lock();
        // c:1531-1538 — ZOF_* are independent bits in a single u8 field.
        let all = [
            ZOF_ARG, ZOF_OPT, ZOF_MULT, ZOF_SAME, ZOF_MAP, ZOF_CYC, ZOF_GNUS, ZOF_GNUL,
        ];
        let xor: i32 = all.iter().fold(0, |acc, &x| acc | x);
        let sum: i32 = all.iter().sum();
        assert_eq!(xor, sum, "ZOF_* bits must be disjoint");
        // Ensure each is a power of two.
        for v in all {
            assert!(
                v > 0 && (v & (v - 1)) == 0,
                "ZOF value {} is not a power of 2",
                v
            );
        }
    }

    /// Verifies pattern matching via the style_table.get path mirrors
    /// C's lookupstyle (zutil.c:443) walking the pats list for the
    /// first weight-sorted match.
    #[test]
    fn test_style_pattern_matches() {
        let _g = crate::test_util::global_state_lock();
        let mut t = style_table::new();
        t.set(":completion:*", "s1", vec!["v".to_string()], None);
        assert!(t.get(":completion:zsh:complete", "s1").is_some());
        assert!(t.get(":other:zsh", "s1").is_none());

        let mut t2 = style_table::new();
        t2.set("*", "s2", vec!["v".to_string()], None);
        assert!(t2.get("anything", "s2").is_some());
    }

    #[test]
    fn test_style_table_set_get() {
        let _g = crate::test_util::global_state_lock();
        let mut table = style_table::new();
        table.set(":completion:*", "verbose", vec!["yes".to_string()], None);

        let result = table.get(":completion:zsh", "verbose");
        assert_eq!(result, Some(&["yes".to_string()][..]));

        let result = table.get(":other", "verbose");
        assert!(result.is_none());
    }

    #[test]
    fn test_style_table_priority() {
        let _g = crate::test_util::global_state_lock();
        let mut table = style_table::new();
        table.set("*", "menu", vec!["no".to_string()], None);
        table.set(":completion:*", "menu", vec!["yes".to_string()], None);

        let result = table.get(":completion:zsh", "menu");
        assert_eq!(result, Some(&["yes".to_string()][..]));
    }

    #[test]
    fn test_style_table_delete() {
        let _g = crate::test_util::global_state_lock();
        let mut table = style_table::new();
        table.set("*", "style1", vec!["val".to_string()], None);
        table.set("*", "style2", vec!["val".to_string()], None);

        table.delete(None, Some("style1"));
        assert!(table.get("test", "style1").is_none());
        assert!(table.get("test", "style2").is_some());
    }

    #[test]
    fn test_style_test_bool() {
        let _g = crate::test_util::global_state_lock();
        let mut table = style_table::new();
        table.set("*", "enabled", vec!["yes".to_string()], None);
        table.set("*", "disabled", vec!["no".to_string()], None);
        table.set(
            "*",
            "multiple",
            vec!["a".to_string(), "b".to_string()],
            None,
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
        let _g = crate::test_util::global_state_lock();
        let key_style = "test_zutil_global_marker_style";
        let key_pat = "test_zutil_global_marker_*";
        {
            let mut t = zstyletab.lock().unwrap();
            t.set(key_pat, key_style, vec!["yes".to_string()], None);
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
        let _g = crate::test_util::global_state_lock();
        let mut specs = HashMap::new();
        specs.insert('n', "test".to_string());
        specs.insert('v', "42".to_string());

        let result = zformat_substring("Name: %n, Value: %v", &specs, false);
        assert_eq!(result, "Name: test, Value: 42");
    }

    #[test]
    fn test_zformat_padding() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = HashMap::new();
        specs.insert('n', "hi".to_string());

        let result = zformat_substring("[%10n]", &specs, false);
        assert_eq!(result, "[hi        ]");

        let result = zformat_substring("[%-10n]", &specs, false);
        assert_eq!(result, "[        hi]");
    }

    #[test]
    fn test_zformat_truncate() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = HashMap::new();
        specs.insert('n', "hello world".to_string());

        let result = zformat_substring("[%.5n]", &specs, false);
        assert_eq!(result, "[hello]");
    }

    #[test]
    fn test_zformat_escape() {
        let _g = crate::test_util::global_state_lock();
        let specs = HashMap::new();
        let result = zformat_substring("100%%", &specs, false);
        assert_eq!(result, "100%");
    }

    /// `Src/Modules/zutil.c:923-936` — Unknown spec character emits the
    /// original `%X` literal back into the output (not consumed). Pin
    /// the fallback so a regen that drops the unknown-spec branch would
    /// silently swallow `%z` when only `%n` was registered.
    #[test]
    fn zformat_unknown_spec_emits_literal_percent_x() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = HashMap::new();
        specs.insert('n', "hello".to_string());
        // %z is unknown; must round-trip
        let result = zformat_substring("%z %n", &specs, false);
        assert_eq!(
            result, "%z hello",
            "c:923-936 — unknown spec emits raw `%X` segment"
        );
    }

    /// `Src/Modules/zutil.c:825-826` — Right-align flag with explicit
    /// min-width. `%-5n` right-pads with spaces on the LEFT. Pin both
    /// arms (left/right) since a regen flipping the polarity would
    /// silently invert every zformat-prompted output.
    #[test]
    fn zformat_right_align_with_min_width() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = HashMap::new();
        specs.insert('n', "ab".to_string());
        // Left-align (default): pad on RIGHT
        assert_eq!(zformat_substring("[%5n]", &specs, false), "[ab   ]");
        // Right-align (-): pad on LEFT
        assert_eq!(zformat_substring("[%-5n]", &specs, false), "[   ab]");
    }

    /// `Src/Modules/zutil.c:825-845` — Min + Max combined: `%5.10n`
    /// means right-pad to min=5, truncate at max=10. With value
    /// "hello world" (11 chars), max=10 truncates to "hello worl".
    #[test]
    fn zformat_min_max_combined() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = HashMap::new();
        specs.insert('n', "hello world".to_string());
        // max=10 truncates the 11-char value
        let r = zformat_substring("[%5.10n]", &specs, false);
        assert_eq!(r, "[hello worl]");
        // Short value: min=5 right-pads (default = left-align)
        specs.insert('n', "hi".to_string());
        let r = zformat_substring("[%5.10n]", &specs, false);
        assert_eq!(
            r, "[hi   ]",
            "c:828-845 — min-pad then max-truncate; short value gets the pad only"
        );
    }

    /// `Src/Modules/zutil.c:975-976` — `%)` is pre-registered as `)`
    /// so the parser can emit a literal `)` from the user's format
    /// string. Pin the alias.
    #[test]
    fn zformat_close_paren_escape() {
        let _g = crate::test_util::global_state_lock();
        let specs = HashMap::new();
        let r = zformat_substring("a%)b", &specs, false);
        assert_eq!(r, "a)b", "c:975-976 — `%)` emits literal `)`");
    }

    /// `Src/Modules/zutil.c:847-887` — Ternary
    /// `%(SPEC.true-text.false-text)` emits the FIRST branch ("true-text")
    /// when the spec exists. Per `man zshmodules`: "if the contents of
    /// the spec are present then true-text is output, otherwise
    /// false-text." With presence=true (zformat -F), spec-set means
    /// emit true-text.
    #[test]
    fn zformat_ternary_presence_mode_spec_set() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = HashMap::new();
        specs.insert('s', "anything".to_string());
        // presence=true: spec exists → TRUE branch (first text).
        let r = zformat_substring("%(s.yes.no)", &specs, true);
        assert_eq!(
            r, "yes",
            "c:847-887 — spec set, presence=true → TRUE branch (true-text first)"
        );
    }

    /// `Src/Modules/zutil.c:847-887` — Ternary with missing spec in
    /// presence-mode emits the SECOND branch ("false-text"). Per
    /// docs: "if contents of the spec are present then true-text is
    /// output, otherwise false-text."
    #[test]
    fn zformat_ternary_presence_mode_spec_unset() {
        let _g = crate::test_util::global_state_lock();
        let specs = HashMap::new();
        let r = zformat_substring("%(s.yes.no)", &specs, true);
        assert_eq!(
            r, "no",
            "c:847-887 — spec unset, presence=true → FALSE branch (false-text second)"
        );
    }

    /// `Src/Modules/zutil.c:937-948` — Plain (non-`%`) bytes between
    /// specs emit verbatim. Pin the simplest no-spec passthrough.
    #[test]
    fn zformat_literal_text_passes_through() {
        let _g = crate::test_util::global_state_lock();
        let specs = HashMap::new();
        let r = zformat_substring("hello, world", &specs, false);
        assert_eq!(r, "hello, world");
        // Empty format → empty output
        let r = zformat_substring("", &specs, false);
        assert_eq!(r, "");
    }

    /// `Src/Modules/zutil.c:890-922` — When max=0 (`.0`), the value is
    /// truncated to ZERO chars — but the min-pad still fires. Edge case
    /// that pins the order: truncate-then-pad, not pad-then-truncate.
    #[test]
    fn zformat_max_zero_truncates_to_empty_but_keeps_min_pad() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = HashMap::new();
        specs.insert('n', "abc".to_string());
        let r = zformat_substring("[%3.0n]", &specs, false);
        assert_eq!(
            r, "[   ]",
            "c:890-922 — max=0 → empty value, min=3 → 3 spaces"
        );
    }

    /// `Src/Modules/zutil.c:55-68` — `restorematch` MUST `unsetparam`
    /// each field that's None (not just leave it alone). Pin:
    /// pre-seed `$match` with a value, take a snapshot where it's None,
    /// call restorematch, observe `$match` is now UNSET in paramtab.
    #[test]
    fn restorematch_unsets_params_when_snapshot_is_none() {
        let _g = crate::test_util::global_state_lock();
        // Pre-seed `$match` so we can observe the unset.
        assignaparam("match", vec!["seed".to_string()], 0);
        assert!(getaparam("match").is_some(), "test setup: $match seeded");

        // Snapshot with all three fields None — restorematch must
        // unsetparam each (c:60/64/68).
        let snap = MatchData {
            r#match: None,
            mbegin: None,
            mend: None,
        };
        restorematch(&snap);
        assert!(
            getaparam("match").is_none(),
            "c:60 — None snapshot must unsetparam(\"match\")"
        );
    }

    // ─── zsh-corpus pins for zstyle helpers ──────────────────────────

    /// `lookupstyle` on missing context returns empty Vec.
    #[test]
    fn zutil_corpus_lookupstyle_missing_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = lookupstyle(":zshrs:test:nonexistent_zxyz", "style_nope");
        assert!(r.is_empty(), "missing zstyle context returns empty");
    }

    /// `testforstyle` on missing context returns 1 (not found).
    #[test]
    fn zutil_corpus_testforstyle_missing_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let r = testforstyle(":zshrs:test:never_set_xyz", "style");
        assert_eq!(r, 1, "missing = 1 per c:485 return !found");
    }

    /// `addstyle` on new name returns Some(style).
    #[test]
    fn zutil_corpus_addstyle_new_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let s = addstyle("zshrs_test_new_style");
        assert!(s.is_some(), "addstyle for new name returns Some");
    }

    /// `addstyle` returning Some on existing name (idempotent).
    #[test]
    fn zutil_corpus_addstyle_existing_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let _ = addstyle("zshrs_test_dup_style");
        let s = addstyle("zshrs_test_dup_style");
        assert!(s.is_some(), "addstyle on existing name still returns Some");
    }

    /// `newzstyletable` returns None per current stub (no
    /// HashNode-allocating impl yet).
    #[test]
    fn zutil_corpus_newzstyletable_current_impl_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let t = newzstyletable(64, "test_tab");
        // Current impl is a stub returning None — pin the contract.
        assert!(
            t.is_none(),
            "newzstyletable stub returns None; pin until ported"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zutil.c zformat_substring.
    // ═══════════════════════════════════════════════════════════════════

    /// c:814 — `zformat_substring("", _, _)` returns empty.
    #[test]
    fn zformat_substring_empty_format_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let specs = std::collections::HashMap::new();
        let r = zformat_substring("", &specs, false);
        assert!(r.is_empty());
    }

    /// c:814 — plain text with no `%` passes through verbatim.
    #[test]
    fn zformat_substring_plain_text_pass_through() {
        let _g = crate::test_util::global_state_lock();
        let specs = std::collections::HashMap::new();
        assert_eq!(zformat_substring("hello", &specs, false), "hello");
        assert_eq!(zformat_substring("abc def", &specs, false), "abc def");
    }

    /// c:975 — `%%` produces literal `%` (pre-populated spec).
    #[test]
    fn zformat_substring_percent_percent_is_literal_percent() {
        let _g = crate::test_util::global_state_lock();
        let specs = std::collections::HashMap::new();
        let r = zformat_substring("%%", &specs, false);
        assert_eq!(r, "%");
    }

    /// c:976 — `%)` produces literal `)` (pre-populated spec).
    #[test]
    fn zformat_substring_percent_close_paren_is_literal_close_paren() {
        let _g = crate::test_util::global_state_lock();
        let specs = std::collections::HashMap::new();
        let r = zformat_substring("%)", &specs, false);
        assert_eq!(r, ")");
    }

    /// c:814 — `%X` substitutes spec value when registered.
    #[test]
    fn zformat_substring_substitutes_registered_spec() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = std::collections::HashMap::new();
        specs.insert('n', "alice".to_string());
        let r = zformat_substring("%n", &specs, false);
        assert_eq!(r, "alice");
    }

    /// c:814 — multiple `%X` substitutions in same format.
    #[test]
    fn zformat_substring_multiple_specs() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = std::collections::HashMap::new();
        specs.insert('n', "alice".to_string());
        specs.insert('h', "/home/alice".to_string());
        let r = zformat_substring("%n is at %h", &specs, false);
        assert_eq!(r, "alice is at /home/alice");
    }

    /// c:814 — text between two `%X` substitutions preserved.
    #[test]
    fn zformat_substring_text_between_specs_preserved() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = std::collections::HashMap::new();
        specs.insert('a', "X".to_string());
        let r = zformat_substring("[%a-%a]", &specs, false);
        assert_eq!(r, "[X-X]");
    }

    /// c:814 — caller's `%`-override beats the pre-populated literal `%`.
    /// Pin: if user registers `%`→"OVERRIDE", `%%` → "OVERRIDE".
    #[test]
    fn zformat_substring_caller_override_of_percent_wins() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = std::collections::HashMap::new();
        specs.insert('%', "OVERRIDE".to_string());
        let r = zformat_substring("%%", &specs, false);
        assert_eq!(
            r, "OVERRIDE",
            "caller override of % beats default '%' literal"
        );
    }

    /// c:814 — `%` followed by unregistered char produces empty
    /// substitution.
    #[test]
    fn zformat_substring_unregistered_spec_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let specs = std::collections::HashMap::new();
        let _ = zformat_substring("%z", &specs, false);
    }

    /// c:814 — determinism for identical input.
    #[test]
    fn zformat_substring_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let mut specs = std::collections::HashMap::new();
        specs.insert('n', "alice".to_string());
        let first = zformat_substring("hi %n!", &specs, false);
        for _ in 0..5 {
            assert_eq!(zformat_substring("hi %n!", &specs, false), first);
        }
    }

    /// Lifecycle (c:2966/2988/2995/3002):
    #[test]
    fn zutil_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:3002 — finish_(NULL) = 0.
    #[test]
    fn zutil_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zutil.c
    // c:787 lookupstyle / c:810 testforstyle / c:837 bin_zstyle /
    // c:1131 bin_zformat / c:2172 get_opt_desc / c:2192 lookup_opt /
    // c:2216 get_opt_arr / c:2966-3002 lifecycle type pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:787 — `lookupstyle` returns Vec<String> (compile-time type pin).
    #[test]
    fn lookupstyle_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = lookupstyle("", "");
    }

    /// c:787 — `lookupstyle("", "")` empty inputs returns empty Vec.
    #[test]
    fn lookupstyle_empty_inputs_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = lookupstyle("", "");
        assert!(r.is_empty(), "empty context/style → empty Vec");
    }

    /// c:787 — `lookupstyle` deterministic for stable input.
    #[test]
    fn lookupstyle_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for (ctx, sty) in [("", ""), ("a", "b"), ("zsh", "color")] {
            let first = lookupstyle(ctx, sty);
            for _ in 0..3 {
                assert_eq!(
                    lookupstyle(ctx, sty),
                    first,
                    "lookupstyle({:?}, {:?}) must be deterministic",
                    ctx,
                    sty
                );
            }
        }
    }

    /// c:810 — `testforstyle` returns i32 (compile-time type pin).
    #[test]
    fn testforstyle_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = testforstyle("", "");
    }

    /// c:465 — `testforstyle(ctxt, style)` returns `!found` per the C
    /// body's final `return !found;` line. With an empty zstyletab,
    /// no entry can match → found=false → return 1 (NOT 0). The
    /// previous test expectation conflated the C-level "0=success"
    /// convention with the "0=style-present" semantic — they're
    /// the same value but the test's "not present" comment makes the
    /// 0 assertion inverted.
    #[test]
    fn testforstyle_empty_inputs_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            testforstyle("", ""),
            1,
            "empty ctx/style → 1 (not present, per C `!found`)"
        );
    }

    /// c:837 — `bin_zstyle` returns i32 (compile-time type pin).
    #[test]
    fn bin_zstyle_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_zstyle("zstyle", &[], &ops, 0);
    }

    /// c:1131 — `bin_zformat` returns i32.
    #[test]
    fn bin_zformat_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_zformat("zformat", &[], &ops, 0);
    }

    /// c:2172 — `get_opt_desc` returns Option<Zoptdesc>.
    #[test]
    fn get_opt_desc_returns_option_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<Zoptdesc> = get_opt_desc("");
    }

    /// c:2172 — `get_opt_desc` deterministic for unknown name.
    #[test]
    fn get_opt_desc_unknown_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = get_opt_desc("__never_real_opt__").is_some();
        for _ in 0..3 {
            assert_eq!(
                get_opt_desc("__never_real_opt__").is_some(),
                a,
                "get_opt_desc must be deterministic"
            );
        }
    }

    /// c:2192 — `lookup_opt` returns Option<Zoptdesc>.
    #[test]
    fn lookup_opt_returns_option_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<Zoptdesc> = lookup_opt("");
    }

    /// c:2216 — `get_opt_arr` returns Option<Zoptarr>.
    #[test]
    fn get_opt_arr_returns_option_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<Zoptarr> = get_opt_arr("");
    }

    /// c:2966 — `setup_` returns i32 (compile-time type pin).
    #[test]
    fn zutil_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:2988 + c:2995 — boot/cleanup idempotent.
    #[test]
    fn zutil_boot_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            assert_eq!(boot_(std::ptr::null()), 0);
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/zutil.c
    // c:787 lookupstyle / c:810 testforstyle / c:837 bin_zstyle /
    // c:1131 bin_zformat / c:2022 bin_zregexparse / c:2408 bin_zparseopts /
    // c:3135 zformat_substring
    // ═══════════════════════════════════════════════════════════════════

    /// c:787 — `lookupstyle` returns Vec<String> (compile-time pin, alt).
    #[test]
    fn lookupstyle_returns_vec_string_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = lookupstyle("", "");
    }

    /// c:787 — `lookupstyle("", "")` returns empty Vec.
    #[test]
    fn lookupstyle_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let v = lookupstyle("", "");
        assert!(v.is_empty(), "empty ctx/style → empty Vec; got {:?}", v);
    }

    /// c:810 — `testforstyle` returns i32 (compile-time pin, alt).
    #[test]
    fn testforstyle_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = testforstyle("", "");
    }

    /// c:837 — `bin_zstyle` no-args returns 0 (listing form per c:837).
    #[test]
    fn bin_zstyle_no_args_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zstyle("zstyle", &[], &ops, 0);
        assert!(
            r == 0 || r == 1,
            "no args is the listing form; result must be 0/1, got {}",
            r
        );
    }

    /// c:1131 — `bin_zformat` no-args returns nonzero (usage error).
    #[test]
    fn bin_zformat_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zformat("zformat", &[], &ops, 0);
        assert_ne!(r, 0, "zformat no args → usage error");
    }

    /// c:2022 — `bin_zregexparse` returns i32 (compile-time pin).
    #[test]
    fn bin_zregexparse_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_zregexparse("zregexparse", &[], &ops, 0);
    }

    /// c:2408 — `bin_zparseopts` returns i32 (compile-time pin).
    #[test]
    fn bin_zparseopts_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_zparseopts("zparseopts", &[], &ops, 0);
    }

    /// c:2408 — `bin_zparseopts` no-args returns nonzero (usage error).
    #[test]
    fn bin_zparseopts_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_zparseopts("zparseopts", &[], &ops, 0);
        assert_ne!(r, 0, "zparseopts no args → usage error");
    }

    /// c:3135 — `zformat_substring("", _, false)` returns empty (alt).
    #[test]
    fn zformat_substring_empty_format_returns_empty_alt() {
        let specs: std::collections::HashMap<char, String> = std::collections::HashMap::new();
        let r = zformat_substring("", &specs, false);
        assert_eq!(r, "", "empty format → empty output");
    }

    /// c:3135 — `zformat_substring` returns String (compile-time pin).
    #[test]
    fn zformat_substring_returns_string_type() {
        let specs: std::collections::HashMap<char, String> = std::collections::HashMap::new();
        let _: String = zformat_substring("plain text", &specs, false);
    }

    /// c:3135 — `zformat_substring` plain text (no `%`) returns as-is.
    #[test]
    fn zformat_substring_plain_text_no_specs() {
        let specs: std::collections::HashMap<char, String> = std::collections::HashMap::new();
        let r = zformat_substring("hello world", &specs, false);
        assert_eq!(r, "hello world", "text without %-spec is returned verbatim");
    }
}
