//! Direct port of `Src/Zle/compcore.c` — completion core driver.
//!
//! Original C copyright: Sven Wischnowsky 1995-1997.
//!
//! C source is 3,638 lines. This Rust port covers the line-by-line
//! pure-string functions (rembslash, remsquote, comp_quoting_string,
//! multiquote, tildequote, matcheq) that have no global-state
//! dependencies. The substrate-heavy entry points (do_completion,
//! before_complete, after_complete, callcompfunc, makecomplist,
//! check_param, set_comp_sep, addmatch, addmatches, addexpl,
//! begcmgroup, endcmgroup, makearray, dupmatch, permmatches,
//! freematch, freematches, comp_str, ctokenize, get_user_var,
//! get_data_arr, set_list_array, matchcmp) are not yet ported —
//! they require the full `Cmgroup`/`Cmatch` group-state machine
//! (mgroup / matches / fmatches / expls / allccs / amatches /
//! curexpl / nmessages / newmatches globals) and the `tokenize()`
//! mutating-string substrate from glob.c.
//!
//! Most of the completion-engine surface ports got moved to
//! `complete.rs` (the user-visible `compadd` / `compset` entry
//! points and the comp* parameter table); this module is the
//! "behind the curtain" pure C-faithful helpers.

#![allow(non_snake_case, dead_code)]

use crate::ported::zsh_h::{QT_BACKSLASH, QT_DOLLARS, QT_DOUBLE, QT_SINGLE};
use crate::ported::zle::comp_h::Cmatch;

// =====================================================================
// rembslash — `Src/Zle/compcore.c:1322-1336`.
// =====================================================================

/// Port of `rembslash()` from `Src/Zle/compcore.c:1322`.
/// ```c
/// mod_export char *
/// rembslash(char *s)
/// {
///     char *t = s = dupstring(s);
///     while (*s)
///         if (*s == '\\') { chuck(s); if (*s) s++; }
///         else s++;
///     return t;
/// }
/// ```
/// Strip backslash escapes from a token, treating `\X` as `X`.
/// Used by the completion engine when a candidate has already been
/// quoted but a later stage needs the raw form for matching.
pub fn rembslash(s: &str) -> String {                                        // c:1322
    let mut result = String::with_capacity(s.len());                         // c:1325 t = s = dupstring(s)
    let mut chars = s.chars().peekable();                                    // c:1327 while (*s)
    while let Some(c) = chars.next() {
        if c == '\\' {                                                       // c:1328
            // c:1329 — `chuck(s)`. Advance past the backslash, then
            // include the *next* char verbatim (if any).
            if let Some(nxt) = chars.next() {                                // c:1330-1331
                result.push(nxt);
            }
        } else {
            result.push(c);                                                  // c:1332-1333
        }
    }
    result                                                                   // c:1335 return t
}

// =====================================================================
// remsquote — `Src/Zle/compcore.c:1342-1360`.
// =====================================================================

/// Port of `remsquote()` from `Src/Zle/compcore.c:1342`.
/// ```c
/// mod_export int
/// remsquote(char *s)
/// {
///     int ret = 0, qa = (isset(RCQUOTES) ? 1 : 3);
///     char *t = s;
///     while (*s)
///         if (qa == 1 ?
///             (s[0] == '\'' && s[1] == '\'') :
///             (s[0] == '\'' && s[1] == '\\' && s[2] == '\'' && s[3] == '\''))
///         { ret += qa; *t++ = '\''; s += qa + 1; }
///         else *t++ = *s++;
///     *t = '\0';
///     return ret;
/// }
/// ```
/// Remove one of every pair of single quotes, without copying.
/// Returns the number of removed quotes.
///
/// In Rust: takes `&mut String`, edits in place, returns the count.
pub fn remsquote(s: &mut String) -> i32 {                                    // c:1342
    let mut ret: i32 = 0;                                                    // c:1345 ret = 0
    // c:1345 — `qa = (isset(RCQUOTES) ? 1 : 3)`. Under RC_QUOTES,
    // doubled '' encodes a literal '. Under default quoting, a
    // literal ' inside '...' is written '\''.
    let rcquotes = crate::ported::options::opt_state_get("rcquotes")         // c:1345 isset(RCQUOTES)
        .unwrap_or(false);
    let qa: usize = if rcquotes { 1 } else { 3 };

    let bytes = s.as_bytes();                                                // c:1346 t = s
    let mut t = Vec::<u8>::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {                                                  // c:1348 while (*s)
        let matched = if qa == 1 {                                           // c:1349
            // c:1350 — `(s[0] == '\'' && s[1] == '\'')`
            i + 1 < bytes.len() && bytes[i] == b'\'' && bytes[i + 1] == b'\''
        } else {
            // c:1351 — `(s[0]=='\'' && s[1]=='\\' && s[2]=='\'' && s[3]=='\'')`
            i + 3 < bytes.len()
                && bytes[i]     == b'\''
                && bytes[i + 1] == b'\\'
                && bytes[i + 2] == b'\''
                && bytes[i + 3] == b'\''
        };
        if matched {
            ret += qa as i32;                                                // c:1352
            t.push(b'\'');                                                   // c:1353
            i += qa + 1;                                                     // c:1354
        } else {
            t.push(bytes[i]);                                                // c:1356
            i += 1;
        }
    }
    // c:1357 — `*t = '\0'`. Rust strings are length-tracked; we
    // overwrite the original String with the rebuilt buffer.
    *s = String::from_utf8(t).unwrap_or_default();
    ret                                                                      // c:1359 return ret
}

// =====================================================================
// comp_quoting_string — `Src/Zle/compcore.c:1434-1448`.
// =====================================================================

/// Port of `comp_quoting_string()` from `Src/Zle/compcore.c:1434`.
/// ```c
/// mod_export char *
/// comp_quoting_string(int stype)
/// {
///     switch (stype) {
///     case QT_SINGLE:  return "'";
///     case QT_DOUBLE:  return "\"";
///     case QT_DOLLARS: return "$'";
///     default:         return "\\";   /* shuts up compiler */
///     }
/// }
/// ```
/// Returns the opening-quote string for the given quoting style.
/// Used to assemble the prefix of a quoted match candidate.
pub fn comp_quoting_string(stype: i32) -> &'static str {                     // c:1434
    match stype {                                                            // c:1437
        x if x == QT_SINGLE  => "'",                                         // c:1439-1440
        x if x == QT_DOUBLE  => "\"",                                        // c:1441-1442
        x if x == QT_DOLLARS => "$'",                                        // c:1443-1444
        _ => {                                                               // c:1445 default
            let _ = QT_BACKSLASH; // c:1446 — comment "shuts up compiler"
            "\\"
        }
    }
}

// =====================================================================
// multiquote — `Src/Zle/compcore.c:1064-1082`.
// =====================================================================

/// Port of `multiquote()` from `Src/Zle/compcore.c:1064`.
/// ```c
/// mod_export char *
/// multiquote(char *s, int ign)
/// {
///     if (s) {
///         char *os = s, *p = compqstack;
///         if (p && *p && (ign == 0 || p[1])) {
///             if (ign) p++;
///             while (*p) { s = quotestring(s, *p); p++; }
///         }
///         return (s == os ? dupstring(s) : s);
///     }
///     DPUTS(1, "BUG: null pointer in multiquote()");
///     return NULL;
/// }
/// ```
///
/// Apply the quoting stack `compqstack` to `s`. `ign` skips the
/// first level (used when one wrapper has already been peeled off).
/// `compqstack` is a string of `QT_*` byte tags (each char is one
/// byte's worth of `QT_BACKSLASH` / `QT_SINGLE` / `QT_DOUBLE` /
/// `QT_DOLLARS` value).
pub fn multiquote(s: &str, ign: i32) -> String {                             // c:1064
    // c:1066 — `if (s)`. Rust takes &str so non-null is type-guaranteed.
    let stack = crate::ported::zle::complete::COMPQSTACK                     // c:1068 p = compqstack
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let p_bytes = stack.as_bytes();
    // c:1070 — `if (p && *p && (ign == 0 || p[1]))`. `*p` non-null
    // → first byte != 0; `p[1]` non-null → length >= 2.
    if !p_bytes.is_empty() && (ign == 0 || p_bytes.len() > 1) {
        let start = if ign != 0 { 1 } else { 0 };                            // c:1071-1072 if (ign) p++
        let mut cur = s.to_string();
        for &q in &p_bytes[start..] {                                        // c:1073 while (*p)
            // c:1074 — `s = quotestring(s, *p)`.
            let qt = match q as i32 {
                x if x == QT_BACKSLASH => crate::ported::utils::QuoteType::Backslash,
                x if x == QT_SINGLE    => crate::ported::utils::QuoteType::Single,
                x if x == QT_DOUBLE    => crate::ported::utils::QuoteType::Double,
                x if x == QT_DOLLARS   => crate::ported::utils::QuoteType::Dollars,
                _ => crate::ported::utils::QuoteType::Backslash,
            };
            cur = crate::ported::utils::quotestring(&cur, qt);
        }
        cur                                                                  // c:1078 return s (newly built)
    } else {
        s.to_string()                                                        // c:1078 return dupstring(s)
    }
}

// =====================================================================
// tildequote — `Src/Zle/compcore.c:1091-1110`.
// =====================================================================

/// Port of `tildequote()` from `Src/Zle/compcore.c:1091`.
/// ```c
/// mod_export char *
/// tildequote(char *s, int ign)
/// {
///     if (s) {
///         int tilde;
///         if ((tilde = (*s == '~'))) *s = 'x';
///         s = multiquote(s, ign);
///         if (tilde) *s = '~';
///         return s;
///     }
///     DPUTS(1, "BUG: null pointer in tildequote()");
///     return NULL;
/// }
/// ```
///
/// Equivalent to `multiquote(s, ign)`, except a leading `~` is
/// preserved unquoted (when compqstack[0] == QT_BACKSLASH the C
/// version skirts the quoting by temporarily rewriting `~` as `x`,
/// quoting, and then putting the `~` back).
pub fn tildequote(s: &str, ign: i32) -> String {                             // c:1091
    let bytes = s.as_bytes();                                                // c:1095
    let tilde = !bytes.is_empty() && bytes[0] == b'~';                       // c:1097
    let staged = if tilde {
        // c:1098 — `*s = 'x'`. Swap '~' for 'x' before quoting.
        let mut tmp = String::with_capacity(s.len());
        tmp.push('x');
        tmp.push_str(&s[1..]);
        tmp
    } else {
        s.to_string()
    };
    let mut quoted = multiquote(&staged, ign);                               // c:1099
    if tilde && !quoted.is_empty() {                                         // c:1100
        // c:1100 — `if (tilde) *s = '~'`. Restore the leading '~'.
        // Quoted result starts with the original first byte (or
        // its quoting prefix); replace the first '~'/'x' candidate.
        let mut new_q = String::with_capacity(quoted.len());
        let mut chars = quoted.chars();
        // Walk past any quoting prefix that wraps the leading char.
        // The C version simply does `*s = '~'` which only works
        // because backslash quoting prepends a single `\` byte —
        // we look for the first 'x' and swap it.
        let mut swapped = false;
        for c in chars.by_ref() {
            if !swapped && c == 'x' {
                new_q.push('~');
                swapped = true;
            } else {
                new_q.push(c);
            }
        }
        quoted = new_q;
    }
    quoted                                                                   // c:1101 return s
}

// =====================================================================
// matcheq — `Src/Zle/compcore.c:3206-3215`.
// =====================================================================

/// Port of the `matchstreq(a, b)` macro from `Src/Zle/compcore.c:3203`.
/// ```c
/// #define matchstreq(a, b) \
///     ((!(a) && !(b)) || ((a) && (b) && !strcmp((a), (b))))
/// ```
/// Both sides null OR both non-null and equal.
#[inline]
fn matchstreq(a: Option<&String>, b: Option<&String>) -> bool {              // c:3203
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Port of `matcheq()` from `Src/Zle/compcore.c:3206`.
/// ```c
/// static int
/// matcheq(Cmatch a, Cmatch b)
/// {
///     return matchstreq(a->ipre, b->ipre) &&
///         matchstreq(a->pre, b->pre) &&
///         matchstreq(a->ppre, b->ppre) &&
///         matchstreq(a->psuf, b->psuf) &&
///         matchstreq(a->suf, b->suf) &&
///         matchstreq(a->str, b->str);
/// }
/// ```
/// Tests whether two matches would produce the same strings on
/// the command line — used by `makearray` to dedupe sorted matches.
pub fn matcheq(a: &Cmatch, b: &Cmatch) -> bool {                             // c:3206
    matchstreq(a.ipre.as_ref(),  b.ipre.as_ref())  &&                        // c:3209
    matchstreq(a.pre.as_ref(),   b.pre.as_ref())   &&                        // c:3210
    matchstreq(a.ppre.as_ref(),  b.ppre.as_ref())  &&                        // c:3211
    matchstreq(a.psuf.as_ref(),  b.psuf.as_ref())  &&                        // c:3212
    matchstreq(a.suf.as_ref(),   b.suf.as_ref())   &&                        // c:3213
    matchstreq(a.str_.as_ref(),  b.str_.as_ref())                            // c:3214
}

// =====================================================================
// set_comp_sep — `Src/Zle/compcore.c:1458-1944`.
// =====================================================================

/// Port of `set_comp_sep()` from `Src/Zle/compcore.c:1458`.
/// ~485-line C function that splits the current word as if it were
/// a command line — the implementation behind `compset -q`.
///
/// **Not yet ported.** The C body depends on the line-buffer
/// state (`zlemetaline`, `zlemetacs`, `zlemetall`), the lexer
/// pushback path (`addedx`, `lexsave` / `lexrestore`), the
/// quoting-stack machine (`compqstack` mutation), and the heap
/// allocator's per-completion arena (`zhalloc`, `metafy_line`).
/// Real port requires landing those substrates first.
///
/// Real caller: `bin_compset` → `'q'` arg in
/// `src/ported/zle/complete.rs:704`.
pub fn set_comp_sep() -> i32 {                                               // c:1458
    // c:1458-1944 — full body deferred. Panic loudly so a real
    // call from `compset -q` surfaces the gap instead of silently
    // returning success.
    unimplemented!(
        "compcore.rs::set_comp_sep — port of Src/Zle/compcore.c:1458 \
         deferred; depends on lexsave/lexrestore + zlemetaline + \
         compqstack mutation. Real caller: complete.rs::bin_compset \
         arg 'q'."
    );
}

// =====================================================================
// Restored stubs — `unimplemented!()` placeholders so the C-source
// fn names remain searchable in the codebase. Bodies are deferred
// until the surrounding substrate (Cmgroup linked-list machine,
// mgroup/matches/fmatches/expls/allccs/amatches/curexpl/nmessages
// /newmatches globals, mutating tokenize() from glob.c, lexsave
// /lexrestore, zhalloc heap arena) lands. Each panics on call so
// silent fakes can't escape — per the no-shortcuts rule.
// =====================================================================

/// Port of `add_match_data()` from `Src/Zle/compcore.c:2643`.
/// Computes the per-match buckets used by addmatch — needs the
/// full Cadata + Cline + matcher pipeline.
pub fn add_match_data() -> i32 {                                             // c:2643
    unimplemented!("compcore.rs::add_match_data — c:2643 deferred (Cadata pipeline)");
}

/// Port of `addexpl()` from `Src/Zle/compcore.c:3140`.
pub fn addexpl() -> i32 {                                                    // c:3140
    unimplemented!("compcore.rs::addexpl — c:3140 deferred (expls/curexpl/mgroup/nmessages globals)");
}

/// Port of `addmatch()` from `Src/Zle/compcore.c:2041`.
pub fn addmatch() -> i32 {                                                   // c:2041
    unimplemented!("compcore.rs::addmatch — c:2041 deferred (Cmatch alloc + matches list push)");
}

/// Port of `addmatches()` from `Src/Zle/compcore.c:2080`.
pub fn addmatches() -> i32 {                                                 // c:2080
    unimplemented!("compcore.rs::addmatches — c:2080 deferred (Cadata-driven match-set construction)");
}

/// Port of `after_complete()` from `Src/Zle/compcore.c:503`.
pub fn after_complete() -> i32 {                                             // c:503
    unimplemented!("compcore.rs::after_complete — c:503 deferred (Hookdef plumbing)");
}

/// Port of `before_complete()` from `Src/Zle/compcore.c:461`.
pub fn before_complete() -> i32 {                                            // c:461
    unimplemented!("compcore.rs::before_complete — c:461 deferred (Hookdef plumbing)");
}

/// Port of `begcmgroup()` from `Src/Zle/compcore.c:3073`.
pub fn begcmgroup() -> i32 {                                                 // c:3073
    unimplemented!("compcore.rs::begcmgroup — c:3073 deferred (mgroup/amatches/expls/matches/fmatches/allccs LinkLists)");
}

/// Port of `callcompfunc()` from `Src/Zle/compcore.c:544`.
pub fn callcompfunc() -> i32 {                                               // c:544
    unimplemented!("compcore.rs::callcompfunc — c:544 deferred (shfunc invocation + comp* params hydration)");
}

/// Port of `check_param()` from `Src/Zle/compcore.c:1113`.
pub fn check_param() -> i32 {                                                // c:1113
    unimplemented!("compcore.rs::check_param — c:1113 deferred (paramtable + subscript probe)");
}

/// Port of `comp_str()` from `Src/Zle/compcore.c:1402`.
pub fn comp_str() -> i32 {                                                   // c:1402
    unimplemented!("compcore.rs::comp_str — c:1402 deferred (compiprefix/compprefix/compsuffix globals + ctokenize)");
}

/// Port of `ctokenize()` from `Src/Zle/compcore.c:1365`.
pub fn ctokenize() -> i32 {                                                  // c:1365
    unimplemented!("compcore.rs::ctokenize — c:1365 deferred (mutating tokenize() from glob.c)");
}

/// Port of `do_completion()` from `Src/Zle/compcore.c:287`.
pub fn do_completion() -> i32 {                                              // c:287
    unimplemented!("compcore.rs::do_completion — c:287 deferred (Hookdef + Compldat + full callcompfunc dispatch)");
}

/// Port of `dupmatch()` from `Src/Zle/compcore.c:3370`.
pub fn dupmatch() -> i32 {                                                   // c:3370
    unimplemented!("compcore.rs::dupmatch — c:3370 deferred (Cmatch deep-copy across heap/perm arenas)");
}

/// Port of `endcmgroup()` from `Src/Zle/compcore.c:3131`.
pub fn endcmgroup() -> i32 {                                                 // c:3131
    unimplemented!("compcore.rs::endcmgroup — c:3131 deferred (mgroup->ylist assignment)");
}

/// Port of `freematch()` from `Src/Zle/compcore.c:3575`.
pub fn freematch() -> i32 {                                                  // c:3575
    unimplemented!("compcore.rs::freematch — c:3575 deferred (Cmatch perm-alloc free walk)");
}

/// Port of `freematches()` from `Src/Zle/compcore.c:3605`.
pub fn freematches() -> i32 {                                                // c:3605
    unimplemented!("compcore.rs::freematches — c:3605 deferred (Cmgroup walk + freematch per element)");
}

/// Port of `get_data_arr()` from `Src/Zle/compcore.c:2022`.
pub fn get_data_arr() -> i32 {                                               // c:2022
    unimplemented!("compcore.rs::get_data_arr — c:2022 deferred (assoc-array param scan helper)");
}

/// Port of `get_user_var()` from `Src/Zle/compcore.c:1956`.
pub fn get_user_var() -> i32 {                                               // c:1956
    unimplemented!("compcore.rs::get_user_var — c:1956 deferred (paramtable getstrvalue/getaparam)");
}

/// Port of `makearray()` from `Src/Zle/compcore.c:3223`.
pub fn makearray() -> i32 {                                                  // c:3223
    unimplemented!("compcore.rs::makearray — c:3223 deferred (LinkList→Cmatch* + qsort dedupe)");
}

/// Port of `makecomplist()` from `Src/Zle/compcore.c:946`.
pub fn makecomplist() -> i32 {                                               // c:946
    unimplemented!("compcore.rs::makecomplist — c:946 deferred (full completion driver)");
}

/// Port of `matchcmp()` from `Src/Zle/compcore.c:3173`.
pub fn matchcmp() -> i32 {                                                   // c:3173
    unimplemented!("compcore.rs::matchcmp — c:3173 deferred (matchorder global + zstrcmp w/ SORTIT_*)");
}

/// Port of `permmatches()` from `Src/Zle/compcore.c:3422`.
pub fn permmatches() -> i32 {                                                // c:3422
    unimplemented!("compcore.rs::permmatches — c:3422 deferred (perm-alloc copy of mgroup/matches)");
}

/// Port of `set_list_array()` from `Src/Zle/compcore.c:1947`.
pub fn set_list_array() -> i32 {                                             // c:1947
    unimplemented!("compcore.rs::set_list_array — c:1947 deferred (param assignment helper)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rembslash_basic() {
        // c:1322-1336 — `\X` becomes `X`, `\\` becomes `\`.
        assert_eq!(rembslash("hello\\ world"), "hello world");
        assert_eq!(rembslash("no\\\\slash"),   "no\\slash");
        assert_eq!(rembslash("plain"),         "plain");
    }

    #[test]
    fn comp_quoting_string_table() {
        // c:1434-1447 — switch arms.
        assert_eq!(comp_quoting_string(QT_SINGLE),  "'");
        assert_eq!(comp_quoting_string(QT_DOUBLE),  "\"");
        assert_eq!(comp_quoting_string(QT_DOLLARS), "$'");
        assert_eq!(comp_quoting_string(0),          "\\");
        assert_eq!(comp_quoting_string(QT_BACKSLASH), "\\");
    }

    #[test]
    fn matcheq_equal_strings() {
        // c:3206-3215 — same str_ + all None elsewhere → equal.
        let mut a = Cmatch::default(); a.str_ = Some("foo".into());
        let mut b = Cmatch::default(); b.str_ = Some("foo".into());
        assert!(matcheq(&a, &b));
    }

    #[test]
    fn matcheq_different_strings() {
        let mut a = Cmatch::default(); a.str_ = Some("foo".into());
        let mut b = Cmatch::default(); b.str_ = Some("bar".into());
        assert!(!matcheq(&a, &b));
    }

    #[test]
    fn matcheq_one_side_none() {
        // c:3203 matchstreq — one None one Some → not equal.
        let mut a = Cmatch::default(); a.pre = Some("p".into());
        let b = Cmatch::default();
        assert!(!matcheq(&a, &b));
    }

    #[test]
    fn remsquote_default_quoting() {
        // c:1342 with RCQUOTES off (qa=3): '\''  →  '
        let mut s = String::from("a'\\''b");
        let n = remsquote(&mut s);
        assert_eq!(s, "a'b");
        assert_eq!(n, 3);
    }
}
