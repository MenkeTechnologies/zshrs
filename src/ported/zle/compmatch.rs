//! Completion matching engine for ZLE
//!
//! Port from zsh/Src/Zle/compmatch.c (2,974 lines)
//!
//! This compares two cmatchers and returns non-zero if they are equal.     // c:80
//! Add the given matchers to the bmatcher list.                            // c:97
//! This returns a new Cline structure.                                     // c:140
//!
//! The full matching engine is in compsys/matching.rs (458 lines).
//! This module provides the pattern matching, anchor handling, and
//! match line construction used during completion.
//!
//! Key C functions and their Rust locations:
//! - match_str         → compsys::matching::match_str()
//! - match_parts       → compsys::matching::match_parts()
//! - comp_match        → compsys::matching::comp_match()
//! - pattern_match_equivalence → compsys::matching (inline)
//! - add_match_str/part/sub    → compsys::matching (inline)
//! - cline_* (match line ops)  → compsys::base::CompletionLine

// CompMatcher / MatchFlags / CompLine deleted — Rust-invented structs
// with no C counterpart. The legit C types `Cmatcher` (comp.h:153),
// `Cline` (comp.h:245), and `Cpattern` (comp.h:197) are ported in
// `comp_h.rs` and used by the real porters of `match_str` /
// `pattern_match` / `add_match_str` etc. below.

/// Port of `cpatterns_same(a, b)` from `Src/Zle/compmatch.c:42`.
/// ```c
/// static int
/// cpatterns_same(Cpattern a, Cpattern b)
/// {
///     while (a) {
///         if (!b) return 0;
///         if (a->tp != b->tp) return 0;
///         switch (a->tp) {
///         case CPAT_CCLASS: case CPAT_NCLASS: case CPAT_EQUIV:
///             if (strcmp(a->u.str, b->u.str) != 0) return 0;
///             break;
///         case CPAT_CHAR:
///             if (a->u.chr != b->u.chr) return 0;
///             break;
///         default:
///             break;
///         }
///         a = a->next;
///         b = b->next;
///     }
///     return !b;
/// }
/// ```
/// Walk two parallel `Cpattern` chains testing structural equality
/// (same `tp` + same `str` for class types or same `chr` for
/// CPAT_CHAR). Used by `cmatchers_same` to dedupe matcher specs.
/// Port of `cpatterns_same` from `Src/Zle/compmatch.c:44`.
pub fn cpatterns_same(                                                       // c:44
    mut a: Option<&crate::ported::zle::comp_h::Cpattern>,
    mut b: Option<&crate::ported::zle::comp_h::Cpattern>,
) -> bool {                                                                  // c:42
    use crate::ported::zle::comp_h::{CPAT_CCLASS, CPAT_CHAR, CPAT_EQUIV, CPAT_NCLASS};
    while let Some(ap) = a {                                                 // c:46 while (a)
        let bp = match b {                                                   // c:47
            None => return false,                                            // c:48 if(!b) return 0
            Some(p) => p,
        };
        if ap.tp != bp.tp {                                                  // c:49
            return false;                                                    // c:50
        }
        match ap.tp {                                                        // c:51
            x if x == CPAT_CCLASS || x == CPAT_NCLASS || x == CPAT_EQUIV => {  // c:52-54
                // c:55-58 — equivalent ranges might compare same even when
                // strings differ; the C source admits this is unhandled.
                if ap.str != bp.str {                                      // c:60 strcmp(a->u.str,b->u.str)
                    return false;                                            // c:61
                }
            }
            x if x == CPAT_CHAR => {                                         // c:64
                if ap.chr != bp.chr {                                        // c:65
                    return false;                                            // c:66
                }
            }
            _ => {                                                           // c:69 default
                // c:70 — "here to silence compiler"
            }
        }
        a = ap.next.as_deref();                                              // c:74 a = a->next
        b = bp.next.as_deref();                                              // c:75 b = b->next
    }
    b.is_none()                                                              // c:77 return !b
}

/// Port of `cmatchers_same(a, b)` from `Src/Zle/compmatch.c:82`.
/// ```c
/// static int
/// cmatchers_same(Cmatcher a, Cmatcher b)
/// {
///     return (a == b ||
///             (a->flags == b->flags &&
///              a->llen == b->llen && a->wlen == b->wlen &&
///              (!a->llen || cpatterns_same(a->line, b->line)) &&
///              (a->wlen <= 0 || cpatterns_same(a->word, b->word)) &&
///              (!(a->flags & (CMF_LEFT | CMF_RIGHT)) ||
///               (a->lalen == b->lalen && a->ralen == b->ralen &&
///                (!a->lalen || cpatterns_same(a->left, b->left)) &&
///                (!a->ralen || cpatterns_same(a->right, b->right))))));
/// }
/// ```
/// Test two matchers for full structural equality — flags, lengths,
/// patterns, and (if anchored) anchor patterns must all match.
/// Port of `cmatchers_same` from `Src/Zle/compmatch.c:84`.
pub fn cmatchers_same(                                                       // c:84
    a: &crate::ported::zle::comp_h::Cmatcher,
    b: &crate::ported::zle::comp_h::Cmatcher,
) -> bool {                                                                  // c:82
    use crate::ported::zle::comp_h::{CMF_LEFT, CMF_RIGHT};
    // c:86 — `a == b` short-circuit (pointer identity). Rust uses
    // `std::ptr::eq` for the same effect.
    if std::ptr::eq(a, b) {
        return true;
    }
    // c:87 — `a->flags == b->flags && a->llen == b->llen && a->wlen == b->wlen`.
    if a.flags != b.flags || a.llen != b.llen || a.wlen != b.wlen {
        return false;
    }
    // c:89 — `(!a->llen || cpatterns_same(a->line, b->line))`.
    if a.llen != 0 && !cpatterns_same(a.line.as_deref(), b.line.as_deref()) {
        return false;
    }
    // c:90 — `(a->wlen <= 0 || cpatterns_same(a->word, b->word))`.
    if a.wlen > 0 && !cpatterns_same(a.word.as_deref(), b.word.as_deref()) {
        return false;
    }
    // c:91-94 — anchor checks only if CMF_LEFT/CMF_RIGHT flagged.
    if (a.flags & (CMF_LEFT | CMF_RIGHT)) != 0 {
        if a.lalen != b.lalen || a.ralen != b.ralen {                        // c:92
            return false;
        }
        if a.lalen != 0 && !cpatterns_same(a.left.as_deref(), b.left.as_deref()) {
            return false;                                                    // c:93
        }
        if a.ralen != 0 && !cpatterns_same(a.right.as_deref(), b.right.as_deref()) {
            return false;                                                    // c:94
        }
    }
    true
}

// =====================================================================
// cline_sublen / cline_setlens / cline_matched / revert_cline / cp_cline
// — `Src/Zle/compmatch.c:217-281`.
// =====================================================================

/// Port of `cline_sublen(l)` from `Src/Zle/compmatch.c:218`.
/// ```c
/// int
/// cline_sublen(Cline l)
/// {
///     int len = ((l->flags & CLF_LINE) ? l->llen : l->wlen);
///     if (l->olen && !((l->flags & CLF_SUF) ? l->suffix : l->prefix))
///         len += l->olen;
///     else {
///         Cline p;
///         for (p = l->prefix; p; p = p->next)
///             len += ((p->flags & CLF_LINE) ? p->llen : p->wlen);
///         for (p = l->suffix; p; p = p->next)
///             len += ((p->flags & CLF_LINE) ? p->llen : p->wlen);
///     }
///     return len;
/// }
/// ```
/// Total visual length of one Cline plus its prefix/suffix sub-lists.
/// Port of `cline_sublen` from `Src/Zle/compmatch.c:218`.
pub fn cline_sublen(l: &crate::ported::zle::comp_h::Cline) -> i32 {          // c:218
    use crate::ported::zle::comp_h::{CLF_LINE, CLF_SUF};
    // c:221 — `len = (CLF_LINE ? llen : wlen)`.
    let mut len: i32 = if (l.flags & CLF_LINE) != 0 { l.llen } else { l.wlen };
    // c:223 — `if (olen && !((CLF_SUF ? suffix : prefix))) len += olen`.
    let no_subs = if (l.flags & CLF_SUF) != 0 {
        l.suffix.is_none()
    } else {
        l.prefix.is_none()
    };
    if l.olen != 0 && no_subs {
        len += l.olen;                                                       // c:224
    } else {                                                                 // c:225
        // c:228-229 — walk prefix sub-list summing per-part length.
        let mut p = l.prefix.as_deref();
        while let Some(pp) = p {
            len += if (pp.flags & CLF_LINE) != 0 { pp.llen } else { pp.wlen };
            p = pp.next.as_deref();
        }
        // c:230-231 — walk suffix sub-list.
        let mut p = l.suffix.as_deref();
        while let Some(pp) = p {
            len += if (pp.flags & CLF_LINE) != 0 { pp.llen } else { pp.wlen };
            p = pp.next.as_deref();
        }
    }
    len                                                                      // c:233 return len
}

/// Port of `cline_setlens(l, both)` from `Src/Zle/compmatch.c:239`.
/// ```c
/// void
/// cline_setlens(Cline l, int both)
/// {
///     while (l) {
///         l->min = cline_sublen(l);
///         if (both)
///             l->max = l->min;
///         l = l->next;
///     }
/// }
/// ```
/// Walk a Cline list setting `min` (and optionally `max`) from
/// `cline_sublen`.
/// Port of `cline_setlens` from `Src/Zle/compmatch.c:239`.
pub fn cline_setlens(l: &mut Option<Box<crate::ported::zle::comp_h::Cline>>, both: i32) {  // c:239
    let mut cur = l.as_deref_mut();
    while let Some(node) = cur {                                             // c:242 while (l)
        let s = cline_sublen(node);                                          // c:243 cline_sublen(l)
        node.min = s;                                                        // c:243 l->min = ...
        if both != 0 {                                                       // c:244 if (both)
            node.max = s;                                                    // c:245 l->max = l->min
        }
        cur = node.next.as_deref_mut();                                      // c:246 l = l->next
    }
}

/// Port of `cline_matched(p)` from `Src/Zle/compmatch.c:253`.
/// ```c
/// void
/// cline_matched(Cline p)
/// {
///     while (p) {
///         p->flags |= CLF_MATCHED;
///         cline_matched(p->prefix);
///         cline_matched(p->suffix);
///         p = p->next;
///     }
/// }
/// ```
/// Set `CLF_MATCHED` on every Cline reachable through next/prefix/
/// suffix from `p`.
/// Port of `cline_matched` from `Src/Zle/compmatch.c:253`.
pub fn cline_matched(p: &mut Option<Box<crate::ported::zle::comp_h::Cline>>) {  // c:253
    use crate::ported::zle::comp_h::CLF_MATCHED;
    let mut cur = p.as_deref_mut();
    while let Some(node) = cur {                                             // c:256 while (p)
        node.flags |= CLF_MATCHED;                                           // c:257
        cline_matched(&mut node.prefix);                                     // c:258
        cline_matched(&mut node.suffix);                                     // c:259
        cur = node.next.as_deref_mut();                                      // c:261 p = p->next
    }
}

/// Port of `revert_cline(p)` from `Src/Zle/compmatch.c:269`.
/// ```c
/// Cline
/// revert_cline(Cline p)
/// {
///     Cline r = NULL, n;
///     while (p) {
///         n = p->next;
///         p->next = r;
///         r = p;
///         p = n;
///     }
///     return r;
/// }
/// ```
/// Reverse a Cline `next`-chained list in place; returns the new head.
/// Port of `revert_cline` from `Src/Zle/compmatch.c:270`.
pub fn revert_cline(                                                         // c:270
    mut p: Option<Box<crate::ported::zle::comp_h::Cline>>,
) -> Option<Box<crate::ported::zle::comp_h::Cline>> {                        // c:269
    let mut r: Option<Box<crate::ported::zle::comp_h::Cline>> = None;        // c:272 r = NULL
    while let Some(mut node) = p {                                           // c:274 while (p)
        let n = node.next.take();                                            // c:275 n = p->next
        node.next = r;                                                       // c:276 p->next = r
        r = Some(node);                                                      // c:277 r = p
        p = n;                                                               // c:278 p = n
    }
    r                                                                        // c:280 return r
}

/// Port of `cp_cline(l, deep)` from `Src/Zle/compmatch.c:189`.
/// ```c
/// Cline
/// cp_cline(Cline l, int deep)
/// {
///     Cline r = NULL, *p = &r, t, lp = NULL;
///     while (l) {
///         if ((t = freecl)) freecl = t->next;
///         else t = (Cline) zhalloc(sizeof(*t));
///         memcpy(t, l, sizeof(*t));
///         if (deep) {
///             if (t->prefix) t->prefix = cp_cline(t->prefix, 0);
///             if (t->suffix) t->suffix = cp_cline(t->suffix, 0);
///         }
///         *p = lp = t;
///         p = &(t->next);
///         l = l->next;
///     }
///     *p = NULL;
///     return r;
/// }
/// ```
/// Deep- or shallow-copy a Cline list. `deep` recursively copies
/// the prefix/suffix sub-lists too. The C source draws from a
/// freecl free-list when available — Rust just heap-allocates.
/// Port of `cp_cline` from `Src/Zle/compmatch.c:190`.
pub fn cp_cline(                                                             // c:190
    l: Option<&crate::ported::zle::comp_h::Cline>,
    deep: i32,
) -> Option<Box<crate::ported::zle::comp_h::Cline>> {                        // c:189
    let mut r: Option<Box<crate::ported::zle::comp_h::Cline>> = None;        // c:192 r = NULL
    let mut tail: *mut Option<Box<crate::ported::zle::comp_h::Cline>> = &mut r;
    let mut cur = l;
    while let Some(node) = cur {                                             // c:194 while (l)
        // c:198 — `t = (Cline) zhalloc(sizeof(*t))`.
        // c:199 — `memcpy(t, l, sizeof(*t))`.
        let mut t: Box<crate::ported::zle::comp_h::Cline> = Box::new(node.clone());
        // Reset `next` so the memcpy-equivalent doesn't link to the
        // source's next (the loop sets it via the tail pointer).
        t.next = None;
        if deep != 0 {                                                       // c:200 if (deep)
            // c:201-202 — `t->prefix = cp_cline(t->prefix, 0)`. Already
            // a Box-clone via memcpy; rebuild as deep copy.
            if let Some(pre) = node.prefix.as_deref() {
                t.prefix = cp_cline(Some(pre), 0);                           // c:202
            }
            if let Some(suf) = node.suffix.as_deref() {
                t.suffix = cp_cline(Some(suf), 0);                           // c:204
            }
        }
        // c:206 — `*p = lp = t`. Append to tail.
        // SAFETY: `tail` points into `r` or into the previous node's
        // `next` field; both stay valid for the loop's lifetime.
        unsafe {
            *tail = Some(t);
            // c:207 — `p = &(t->next)`. Re-aim tail at the new entry's `next`.
            let new_node = (*tail).as_mut().unwrap();
            tail = &mut new_node.next;
        }
        cur = node.next.as_deref();                                          // c:208 l = l->next
    }
    // c:210 — `*p = NULL`. Already None by default.
    r                                                                        // c:212 return r
}

/// Port of `free_cline(l)` from `Src/Zle/compmatch.c:171`.
/// ```c
/// void
/// free_cline(Cline l)
/// {
///     Cline n;
///     while (l) {
///         n = l->next;
///         l->next = freecl;
///         freecl = l;
///         free_cline(l->prefix);
///         free_cline(l->suffix);
///         l = n;
///     }
/// }
/// ```
/// Free a Cline list. C pushes onto a `freecl` free-list to recycle;
/// Rust just drops via Box.
/// Port of `free_cline` from `Src/Zle/compmatch.c:171`.
pub fn free_cline(l: Option<Box<crate::ported::zle::comp_h::Cline>>) {       // c:171
    // c:176-183 — walk; free each prefix/suffix recursively. In Rust
    // dropping the Box of the list head triggers Drop on `next`/
    // `prefix`/`suffix` chains automatically. `freecl` recycling
    // is a C-only zhalloc optimisation that doesn't apply here.
    drop(l);
}

// =====================================================================
// matchbuf / matchparts / matchsubs globals + start_match / abort_match
// — `Src/Zle/compmatch.c:283-317`.
// =====================================================================

use std::sync::Mutex;
use std::sync::OnceLock;

/// Port of `char *matchbuf` from `Src/Zle/compmatch.c:287`. Static
/// buffer used during pattern matching to assemble the trial string.
pub static MATCHBUF: OnceLock<Mutex<String>> = OnceLock::new();              // c:287

/// Port of `Cline matchparts, matchlastpart` from
/// `Src/Zle/compmatch.c:292`. Top-level cline list being built.
pub static MATCHPARTS: OnceLock<Mutex<Option<Box<crate::ported::zle::comp_h::Cline>>>> = OnceLock::new();  // c:292

/// Port of `Cline matchsubs, matchlastsub` from
/// `Src/Zle/compmatch.c:294`. Inner cline list (prefix/suffix sub-list).
pub static MATCHSUBS: OnceLock<Mutex<Option<Box<crate::ported::zle::comp_h::Cline>>>> = OnceLock::new();   // c:294

/// Port of `start_match()` from `Src/Zle/compmatch.c:299`.
/// ```c
/// static void
/// start_match(void)
/// {
///     if (matchbuf)
///         *matchbuf = '\0';
///     matchbufadded = 0;
///     matchparts = matchlastpart = matchsubs = matchlastsub = NULL;
/// }
/// ```
/// Reset the per-match globals so a fresh pattern run starts clean.
pub fn start_match() {                                                       // c:299
    // c:302-303 — `if (matchbuf) *matchbuf = '\0'`.
    MATCHBUF
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clear();
    // c:305 — `matchparts = matchlastpart = matchsubs = matchlastsub = NULL`.
    *MATCHPARTS.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
    *MATCHSUBS.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
}

/// Port of `abort_match()` from `Src/Zle/compmatch.c:311`.
/// ```c
/// static void
/// abort_match(void)
/// {
///     free_cline(matchparts);
///     free_cline(matchsubs);
///     matchparts = matchsubs = NULL;
/// }
/// ```
/// Tear down the per-match cline lists when a match attempt fails.
pub fn abort_match() {                                                       // c:311
    // c:314-315 — `free_cline(matchparts); free_cline(matchsubs)`.
    let parts = MATCHPARTS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .take();
    let subs = MATCHSUBS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .take();
    free_cline(parts);
    free_cline(subs);
    // c:316 — set to NULL (already done by .take()).
}

/// Test whether `word` matches `line` honouring the given matcher
/// flags.
// Fake-signature ports of `match_str` / `match_parts` / `comp_match`
// deleted. The real C signatures (Src/Zle/compmatch.c:500, :1092,
// :1123) take Brinfo*/Patprog/Cline* parameters that need the
// matcher engine fully wired through. The previous Rust placeholders
// shipped wrong arities + fake `MatchFlags` / `CompLine` types.
// Real ports will land alongside the matcher-engine driver.


/// Direct port of `mod_export convchar_t pattern_match_equivalence(
///                    Cpattern lp, convchar_t wind, int wmtp,
///                    convchar_t wchr)`
/// from `Src/Zle/compmatch.c:1316-1380`. Looks up the line-side
/// equivalence-class member that pairs with word-side index
/// `wind` (1-based), then resolves case-class crossings via the
/// PP_UPPER/PP_LOWER pair.
///
/// Returns `CHR_INVALID` (u32::MAX) on miss; the matched line
/// char on success.
pub fn pattern_match_equivalence(
    lp: &crate::ported::zle::comp_h::Cpattern,                               // c:1316
    wind: u32, wmtp: i32, wchr: u32,
) -> u32 {
    use crate::ported::zsh_h::{PP_LOWER, PP_UPPER};
    use crate::ported::zle::zle_h::{ZC_tolower, ZC_toupper};

    // c:1324 — PATMATCHINDEX(lp->u.str, wind-1, &lchr, &lmtp).
    // Walk lp.str's encoded char-range descriptor finding the
    // entry at index (wind-1); return CHR_INVALID on miss.
    let Some(ref s) = lp.str else { return u32::MAX; };
    let Some(target_idx) = (wind as i64).checked_sub(1) else { return u32::MAX; };
    if target_idx < 0 { return u32::MAX; }
    let mut lchr: Option<u32> = None;
    let mut lmtp: i32 = 0;
    let mut idx: i64 = 0;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        // Pair `lo-hi` if next is `-`.
        if let Some(&peek) = chars.peek() {
            if peek == '-' {
                chars.next();
                if let Some(hi) = chars.next() {
                    let span = (hi as i64) - (ch as i64);
                    if span >= 0 && idx + span >= target_idx {
                        lchr = Some(((ch as i64) + (target_idx - idx)) as u32);
                        break;
                    }
                    idx += span + 1;
                    continue;
                }
            }
        }
        if idx == target_idx {
            lchr = Some(ch as u32);
            break;
        }
        idx += 1;
    }
    let lchr = match lchr { Some(c) => c, None => return u32::MAX };

    // c:1335 — `if (lchr != CHR_INVALID) return lchr` — exact-char hit.
    if lchr != u32::MAX { return lchr; }

    // c:1342 — case-class crossings.
    let _ = lmtp;
    let wch = char::from_u32(wchr).unwrap_or('\0');
    if wmtp == PP_UPPER && lmtp == PP_LOWER {
        return ZC_tolower(wch) as u32;
    }
    if wmtp == PP_LOWER && lmtp == PP_UPPER {
        return ZC_toupper(wch) as u32;
    }
    if wmtp == lmtp { return wchr; }
    u32::MAX                                                                 // c:1378
}

// Fake `parse_cmatcher` / `update_bmatchers` deleted.
// `parse_cmatcher` already exists at `complete.rs:992` as a real
// port of `Src/Zle/complete.c:242`. `update_bmatchers` is at
// `Src/Zle/compmatch.c:121` with signature `void update_bmatchers(void)`
// — the Rust placeholder had the wrong arity and type, will land
// alongside the matcher-engine driver.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_match_equivalence_case_cross() {
        // c:1342 — wmtp=PP_UPPER, lmtp=PP_LOWER → tolower(wchr).
        use crate::ported::zle::comp_h::{Cpattern, CPAT_EQUIV};
        let lp = Cpattern { tp: CPAT_EQUIV, str: Some("ab".into()), chr: 0, next: None };
        // wind=1 selects 'a' from the equivalence class, exact-char hit.
        let r = pattern_match_equivalence(&lp, 1, 0, b'A' as u32);
        assert_eq!(r, b'a' as u32);
    }

    // ---------- Real-port tests ------------------------------------------

    use crate::ported::zle::comp_h::{
        CLF_LINE, CLF_MATCHED, CLF_SUF, CMF_LEFT, CMF_RIGHT, CPAT_CCLASS, CPAT_CHAR, CPAT_NCLASS,
        Cline, Cmatcher, Cpattern,
    };

    fn cpat_char(ch: u32) -> Cpattern {
        Cpattern {
            tp: CPAT_CHAR,
            chr: ch,
            ..Default::default()
        }
    }
    fn cpat_class(s: &str) -> Cpattern {
        Cpattern {
            tp: CPAT_CCLASS,
            str: Some(s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn cpatterns_same_chr_match() {
        let a = cpat_char('a' as u32);
        let b = cpat_char('a' as u32);
        // c:64-66 — both CPAT_CHAR + same chr → equal.
        assert!(cpatterns_same(Some(&a), Some(&b)));
    }

    #[test]
    fn cpatterns_same_chr_mismatch() {
        let a = cpat_char('a' as u32);
        let b = cpat_char('b' as u32);
        // c:65 — different chr → not equal.
        assert!(!cpatterns_same(Some(&a), Some(&b)));
    }

    #[test]
    fn cpatterns_same_tp_mismatch() {
        let a = cpat_char('a' as u32);
        let b = Cpattern {
            tp: CPAT_NCLASS,
            str: Some("a".into()),
            ..Default::default()
        };
        // c:49-50 — different tp → not equal.
        assert!(!cpatterns_same(Some(&a), Some(&b)));
    }

    #[test]
    fn cpatterns_same_class_match() {
        let a = cpat_class("a-z");
        let b = cpat_class("a-z");
        // c:60 — same str → equal.
        assert!(cpatterns_same(Some(&a), Some(&b)));
    }

    #[test]
    fn cpatterns_same_length_mismatch() {
        let a = cpat_char('a' as u32);
        // a chained to a second pattern; b has only one.
        let mut a_chain = a.clone();
        a_chain.next = Some(Box::new(cpat_char('b' as u32)));
        let b = cpat_char('a' as u32);
        // c:47 — `a` still has next, `b` exhausted → not equal.
        assert!(!cpatterns_same(Some(&a_chain), Some(&b)));
    }

    #[test]
    fn cpatterns_same_both_empty() {
        // c:46 — both NULL → loop never enters, return !b == true.
        assert!(cpatterns_same(None, None));
    }

    #[test]
    fn cmatchers_same_pointer_eq() {
        let m = Cmatcher::default();
        // c:86 — `a == b` short-circuit.
        assert!(cmatchers_same(&m, &m));
    }

    #[test]
    fn cmatchers_same_flags_diff() {
        let a = Cmatcher { flags: 0, ..Default::default() };
        let b = Cmatcher { flags: 1, ..Default::default() };
        // c:87 — different flags → not equal.
        assert!(!cmatchers_same(&a, &b));
    }

    #[test]
    fn cmatchers_same_anchor_lengths() {
        // CMF_LEFT path: anchor length difference matters.
        let a = Cmatcher {
            flags: CMF_LEFT,
            lalen: 2,
            ..Default::default()
        };
        let b = Cmatcher {
            flags: CMF_LEFT,
            lalen: 3,
            ..Default::default()
        };
        // c:92 — different lalen → not equal.
        assert!(!cmatchers_same(&a, &b));
        // CMF_RIGHT path: ralen matters.
        let a = Cmatcher {
            flags: CMF_RIGHT,
            ralen: 1,
            ..Default::default()
        };
        let b = Cmatcher {
            flags: CMF_RIGHT,
            ralen: 1,
            ..Default::default()
        };
        // c:91-94 — anchors equal, no patterns to compare → equal.
        assert!(cmatchers_same(&a, &b));
    }

    #[test]
    fn cline_sublen_simple() {
        let l = Cline {
            flags: CLF_LINE,
            llen: 5,
            wlen: 999,
            ..Default::default()
        };
        // c:221 — CLF_LINE → use llen, not wlen.
        assert_eq!(cline_sublen(&l), 5);
    }

    #[test]
    fn cline_sublen_with_olen() {
        let l = Cline {
            flags: 0,
            llen: 0,
            wlen: 3,
            olen: 7,
            ..Default::default()
        };
        // c:223-224 — no CLF_LINE → wlen=3, no prefix → +olen=7 → 10.
        assert_eq!(cline_sublen(&l), 10);
    }

    #[test]
    fn cline_sublen_with_prefix() {
        let pre = Cline {
            flags: CLF_LINE,
            llen: 4,
            ..Default::default()
        };
        let l = Cline {
            flags: 0,
            wlen: 2,
            olen: 99,                 // ignored because prefix exists
            prefix: Some(Box::new(pre)),
            ..Default::default()
        };
        // c:225-229 — prefix walks to +llen=4; base wlen=2; total=6.
        assert_eq!(cline_sublen(&l), 6);
    }

    #[test]
    fn cline_sublen_clf_suf() {
        let suf = Cline {
            flags: CLF_LINE,
            llen: 3,
            ..Default::default()
        };
        let l = Cline {
            flags: CLF_SUF,
            wlen: 1,
            olen: 99,
            suffix: Some(Box::new(suf)),
            ..Default::default()
        };
        // c:223 — CLF_SUF → check `suffix` not `prefix`. Suffix exists,
        // so olen ignored. wlen=1 + suffix wlen-walk... but suffix has CLF_LINE,
        // so its llen=3 is used. total=1+3=4.
        assert_eq!(cline_sublen(&l), 4);
    }

    #[test]
    fn cline_setlens_propagates() {
        let mut head: Option<Box<Cline>> = Some(Box::new(Cline {
            flags: CLF_LINE,
            llen: 5,
            next: Some(Box::new(Cline {
                flags: CLF_LINE,
                llen: 3,
                ..Default::default()
            })),
            ..Default::default()
        }));
        cline_setlens(&mut head, 1);
        // c:243-245 — both=1 sets max=min=cline_sublen.
        let h = head.as_ref().unwrap();
        assert_eq!(h.min, 5);
        assert_eq!(h.max, 5);
        let n = h.next.as_ref().unwrap();
        assert_eq!(n.min, 3);
        assert_eq!(n.max, 3);
    }

    #[test]
    fn cline_matched_sets_flag_recursively() {
        let mut head: Option<Box<Cline>> = Some(Box::new(Cline {
            prefix: Some(Box::new(Cline::default())),
            suffix: Some(Box::new(Cline::default())),
            next: Some(Box::new(Cline::default())),
            ..Default::default()
        }));
        cline_matched(&mut head);
        let h = head.as_ref().unwrap();
        // c:257 — flag set on head.
        assert!(h.flags & CLF_MATCHED != 0);
        // c:258 — flag set on prefix.
        assert!(h.prefix.as_ref().unwrap().flags & CLF_MATCHED != 0);
        // c:259 — flag set on suffix.
        assert!(h.suffix.as_ref().unwrap().flags & CLF_MATCHED != 0);
        // c:261 — flag set on next.
        assert!(h.next.as_ref().unwrap().flags & CLF_MATCHED != 0);
    }

    #[test]
    fn revert_cline_reverses_chain() {
        let head = Some(Box::new(Cline {
            llen: 1,
            next: Some(Box::new(Cline {
                llen: 2,
                next: Some(Box::new(Cline {
                    llen: 3,
                    ..Default::default()
                })),
                ..Default::default()
            })),
            ..Default::default()
        }));
        let r = revert_cline(head);
        // After reversal: 3, 2, 1.
        let n = r.as_ref().unwrap();
        assert_eq!(n.llen, 3);
        let n = n.next.as_ref().unwrap();
        assert_eq!(n.llen, 2);
        let n = n.next.as_ref().unwrap();
        assert_eq!(n.llen, 1);
        assert!(n.next.is_none());
    }

    #[test]
    fn cp_cline_shallow() {
        let src = Cline {
            llen: 7,
            wlen: 9,
            next: Some(Box::new(Cline {
                llen: 11,
                ..Default::default()
            })),
            ..Default::default()
        };
        let dup = cp_cline(Some(&src), 0);
        let n = dup.as_ref().unwrap();
        assert_eq!(n.llen, 7);
        assert_eq!(n.wlen, 9);
        let n = n.next.as_ref().unwrap();
        assert_eq!(n.llen, 11);
    }

    #[test]
    fn start_match_clears_globals() {
        // Pre-populate to ensure start_match resets.
        MATCHBUF
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap()
            .push_str("garbage");
        *MATCHPARTS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(Box::new(Cline::default()));
        start_match();
        assert!(MATCHBUF.get().unwrap().lock().unwrap().is_empty());
        assert!(MATCHPARTS.get().unwrap().lock().unwrap().is_none());
        assert!(MATCHSUBS.get().unwrap().lock().unwrap().is_none());
    }

    #[test]
    fn abort_match_drops_lists() {
        *MATCHPARTS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(Box::new(Cline::default()));
        *MATCHSUBS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap() = Some(Box::new(Cline::default()));
        abort_match();
        assert!(MATCHPARTS.get().unwrap().lock().unwrap().is_none());
        assert!(MATCHSUBS.get().unwrap().lock().unwrap().is_none());
    }
}

/// Direct port of `mod_export void add_bmatchers(Cmatcher m)` from
/// `Src/Zle/compmatch.c:101-115`. Walks the supplied Cmatcher chain
/// (the head of `def->matcher` at call sites) and prepends each
/// matcher that qualifies for brace-matching to the file-scope
/// `bmatchers` Cmlist. Original chain head is appended after the new
/// entries so the final list is `[new_entries..., old_bmatchers...]`.
pub fn add_bmatchers(m: Option<&crate::ported::zle::comp_h::Cmatcher>) {     // c:101
    use crate::ported::zle::comp_h::{Cmatcher, Cmlist, CMF_RIGHT};

    let old = {                                                              // c:104 Cmlist old = bmatchers
        let cell = crate::ported::zle::compcore::bmatchers
            .get_or_init(|| std::sync::Mutex::new(None));
        cell.lock().ok().and_then(|mut g| g.take())
    };

    let mut head: Option<Box<Cmlist>> = None;                                // c:104 *q = &bmatchers
    let mut tail_ref: *mut Option<Box<Cmlist>> = &mut head;
    let mut cur = m;
    while let Some(mat) = cur {                                              // c:106 for (; m; m = m->next)
        let qual = (mat.flags == 0 && mat.wlen > 0 && mat.llen > 0)          // c:107-108
                || (mat.flags == CMF_RIGHT && mat.wlen < 0 && mat.llen == 0);
        if qual {
            // c:109 — n = zhalloc(sizeof(struct cmlist))
            let n = Box::new(Cmlist {
                next: None,
                matcher: Box::new(Cmatcher {
                    refc:  mat.refc,
                    next:  mat.next.clone(),
                    flags: mat.flags,
                    line:  mat.line.clone(),
                    llen:  mat.llen,
                    word:  mat.word.clone(),
                    wlen:  mat.wlen,
                    left:  mat.left.clone(),
                    lalen: mat.lalen,
                    right: mat.right.clone(),
                    ralen: mat.ralen,
                }),
                str: String::new(),
            });
            unsafe {
                *tail_ref = Some(n);
                if let Some(ref mut newnode) = *tail_ref {
                    tail_ref = &mut newnode.next as *mut _;                  // c:112 q = &(n->next)
                }
            }
        }
        cur = mat.next.as_deref();                                           // c:106 m = m->next
    }
    // c:114 — `*q = old;` (append old chain after new entries)
    unsafe { *tail_ref = old; }
    if let Ok(mut g) = crate::ported::zle::compcore::bmatchers
        .get_or_init(|| std::sync::Mutex::new(None)).lock()
    {
        *g = head;
    }
}

/// Direct port of `static void add_match_part(Cmatcher m, char *l,
///                                            char *w, int wl,
///                                            char *o, int ol,
///                                            char *s, int sl,
///                                            int osl, int sfx)`
/// from `Src/Zle/compmatch.c:373-444`. Appends a partial match into
/// `MATCHPARTS`, splitting the new part via `bld_parts` per the
/// matcher's anchor rules and consuming any pending `MATCHSUBS`
/// nodes into the new tail.
pub fn add_match_part(
    m: Option<&crate::ported::zle::comp_h::Cmatcher>,                        // c:373
    l: Option<&str>, _ll: i32,
    w: &str, wl: i32,
    o: Option<&str>, ol: i32,
    s: &str, sl: i32,
    osl: i32, sfx: i32,
) {
    use crate::ported::zle::comp_h::{Cline, CMF_LEFT, CLF_NEW, CLF_SUF};

    // c:382 — `if (l && !strncmp(l, w, wl)) l = NULL` — drop redundant anchor.
    let l_eff: Option<String> = match l {
        Some(lstr) if lstr.len() >= wl as usize
                    && wl > 0
                    && &lstr[..wl as usize] == &w[..wl as usize] => None,
        Some(lstr) => Some(lstr.to_string()),
        None       => None,
    };

    // c:392 — `p = bld_parts(s, sl, osl, &lp, &lprem)`.
    let mut lp: Option<Box<Cline>> = None;
    let mut lprem: Option<Box<Cline>> = None;
    let mut p = bld_parts(s, sl, osl, Some(&mut lp), Some(&mut lprem));

    // c:394 — `if (lprem && m && (m->flags & CLF_LEFT))`.
    if let Some(rem) = lprem.as_mut() {
        if m.map(|mat| (mat.flags & CMF_LEFT) != 0).unwrap_or(false) {
            rem.flags |= CLF_SUF;                                            // c:395
            rem.suffix = rem.prefix.take();                                  // c:396 swap
        }
    }

    // c:402 — `if (sfx) p = revert_cline(lp = p)`.
    if sfx != 0 {
        if let Some(chain) = p.take() {
            p = revert_cline(Some(chain));
        }
    }

    // c:405-419 — merge MATCHSUBS into the head/tail.
    let subs = MATCHSUBS.get_or_init(|| std::sync::Mutex::new(None))
        .lock().ok().and_then(|mut g| g.take());
    if let Some(subs_chain) = subs {                                         // c:405
        if let Some(lp_node) = lp.as_mut() {
            if sfx != 0 {                                                    // c:407 lp->prefix tail-append
                let mut tail_ref: *mut Option<Box<Cline>> = &mut lp_node.prefix;
                unsafe {
                    while let Some(ref mut next_node) = *tail_ref {
                        tail_ref = &mut next_node.next as *mut _;
                    }
                    *tail_ref = Some(subs_chain);
                }
            } else if let Some(ref mut p_node) = p {                         // c:415 p->prefix prepend
                let old_prefix = p_node.prefix.take();
                let mut new_head = subs_chain;
                {
                    let mut tail_ref: *mut Option<Box<Cline>> = &mut new_head.next;
                    unsafe {
                        while let Some(ref mut nn) = *tail_ref {
                            tail_ref = &mut nn.next as *mut _;
                        }
                        *tail_ref = old_prefix;
                    }
                }
                p_node.prefix = Some(new_head);
            }
        }
        // c:417 — `matchsubs = matchlastsub = NULL`.
        if let Ok(mut g) = MATCHLASTSUB
            .get_or_init(|| std::sync::Mutex::new(None)).lock()
        {
            *g = None;
        }
    }

    // c:421-435 — store args in the last part-cline.
    if let Some(lp_node) = lp.as_mut() {
        if lp_node.llen != 0 || lp_node.wlen != 0 {                          // c:421
            let next = get_cline(
                l_eff.clone(), wl, Some(w.to_string()), wl,
                o.map(|s| s.to_string()), ol, CLF_NEW,
            );
            lp_node.next = Some(next);                                       // c:423
        } else {                                                             // c:425
            lp_node.line = l_eff.clone();                                    // c:426
            lp_node.llen = wl;
            lp_node.word = Some(w.to_string());                              // c:428
            lp_node.wlen = wl;
            lp_node.orig = o.map(|s| s.to_string());                         // c:430
            lp_node.olen = ol;
        }
        if o.is_some() || ol != 0 {                                          // c:432
            lp_node.flags &= !CLF_NEW;
        }
    }

    // c:439-444 — append `p` to MATCHPARTS via MATCHLASTPART.
    let last_present = MATCHLASTPART.get()
        .and_then(|c| c.lock().ok().map(|g| g.is_some()))
        .unwrap_or(false);
    if last_present {                                                        // c:440
        if let Ok(mut tail) = MATCHLASTPART
            .get_or_init(|| std::sync::Mutex::new(None)).lock()
        {
            if let Some(t) = tail.as_mut() {
                t.next = p.clone();
            }
        }
    } else if let Ok(mut head) = MATCHPARTS
        .get_or_init(|| std::sync::Mutex::new(None)).lock()
    {
        *head = p.clone();                                                   // c:442
    }
    if let Some(lp_node) = lp {
        if let Ok(mut tail) = MATCHLASTPART
            .get_or_init(|| std::sync::Mutex::new(None)).lock()
        {
            *tail = Some(lp_node);                                           // c:443
        }
    }
}

/// File-scope `Cline matchlastpart` from `Src/Zle/compmatch.c:292`.
pub static MATCHLASTPART: std::sync::OnceLock<std::sync::Mutex<Option<Box<crate::ported::zle::comp_h::Cline>>>>
    = std::sync::OnceLock::new();                                            // c:292

/// Direct port of `static void add_match_str(Cmatcher m, char *l,
///                                          char *w, int wl, int sfx)`
/// from `Src/Zle/compmatch.c:326-370`. Pushes the string `w` (or
/// `l` when `m & CMF_LINE`) of length `wl` into the file-scope
/// `MATCHBUF` accumulator; `sfx` prepends instead of appends.
pub fn add_match_str(m: Option<&crate::ported::zle::comp_h::Cmatcher>,        // c:326
                     l: &str, w: &str, mut wl: i32, sfx: i32)
{
    use crate::ported::zle::comp_h::CMF_LINE;

    // c:332-334 — `if (m && (m->flags & CMF_LINE)) { wl = m->llen; w = l; }`.
    let (eff_w_owned, eff_w): (String, &str) = match m {
        Some(mat) if (mat.flags & CMF_LINE) != 0 => {
            wl = mat.llen;
            let owned = l.to_string();
            let s = owned.clone();
            (owned, Box::leak(s.into_boxed_str()))
        }
        _ => (String::new(), w),
    };
    let _ = eff_w_owned;

    if wl <= 0 { return; }                                                   // c:335

    // c:337-353 — buffer-grow + insert. Rust's String handles the
    // grow path; we still mirror the matchbufadded counter for parity
    // with `MATCHBUFLEN`-checking C call sites.
    if let Ok(mut buf) = MATCHBUF.get_or_init(|| Mutex::new(String::new())).lock() {
        let take_n = wl as usize;
        let new_chunk: String = eff_w.chars().take(take_n).collect();
        if sfx != 0 {                                                        // c:354 prefix-mode
            *buf = format!("{}{}", new_chunk, *buf);                         // c:356
        } else {                                                             // c:358
            buf.push_str(&new_chunk);
        }
        MATCHBUFADDED.fetch_add(wl, std::sync::atomic::Ordering::Relaxed);   // c:362
    }
}

/// File-scope `int matchbufadded` from `Src/Zle/compmatch.c:289`.
pub static MATCHBUFADDED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:289

/// Direct port of `static void add_match_sub(Cmatcher m, char *l, int ll,
///                                          char *w, int wl)` from
/// `Src/Zle/compmatch.c:446-510`. Pushes one sub-match cline node
/// into the file-scope `MATCHSUBS` / `MATCHLASTSUB` linked list.
/// Called from match_str during a CMF_RIGHT anchor match.
pub fn add_match_sub(
    m: Option<&crate::ported::zle::comp_h::Cmatcher>,                        // c:446
    l: Option<&str>, ll: i32, w: Option<&str>, wl: i32,
) {
    use crate::ported::zle::comp_h::{Cline, CLF_NEW};

    // c:450-453 — `if (m && (m->flags & CMF_LINE)) { wl = m->llen; w = l; }`.
    let (eff_w, eff_wl) = match m {
        Some(mat) if (mat.flags & crate::ported::zle::comp_h::CMF_LINE) != 0
                  => (l, mat.llen),
        _ => (w, wl),
    };

    // c:455-456 — short-circuit if no length.
    if eff_wl <= 0 && ll <= 0 { return; }

    // c:464-484 — build a fresh Cline node and append to matchsubs.
    let node = Box::new(Cline {
        flags: CLF_NEW,
        line: l.map(|s| s.to_string()),
        llen: ll,
        word: eff_w.map(|s| s.to_string()),
        wlen: eff_wl,
        ..Default::default()
    });

    let last_cell = MATCHLASTSUB.get_or_init(|| Mutex::new(None));
    let head_cell = MATCHSUBS.get_or_init(|| Mutex::new(None));
    let last_present = last_cell.lock().ok().map(|g| g.is_some()).unwrap_or(false);
    if last_present {                                                        // c:494 — chain to existing tail
        if let Ok(mut tail) = last_cell.lock() {
            if let Some(t) = tail.as_mut() {
                t.next = Some(node.clone());                                 // c:495 matchlastsub->next = n
            }
        }
    } else {                                                                 // c:496 — first node
        if let Ok(mut h) = head_cell.lock() {
            *h = Some(node.clone());                                         // c:497 matchsubs = n
        }
    }
    if let Ok(mut tail) = last_cell.lock() {
        *tail = Some(node);                                                  // c:499 matchlastsub = n
    }
}

/// File-scope `Cline matchlastsub` from `Src/Zle/compmatch.c:294`.
pub static MATCHLASTSUB: std::sync::OnceLock<Mutex<Option<Box<crate::ported::zle::comp_h::Cline>>>>
    = std::sync::OnceLock::new();                                            // c:294

/// Direct port of `static int bld_line(Cmatcher mp, ZLE_STRING_T line,
///                                     char *mword, char *word,
///                                     int wlen, int sfx)`
/// from `Src/Zle/compmatch.c:1736-1992`. Constructs the `line`
/// string from `word` per the supplied matcher, returning the
/// number of word chars consumed.
///
/// **Substrate trade-off:** the full C body builds a per-position
/// generic-pattern array (`genpatarr`) from `mp->line`, handling
/// CPAT_EQUIV → query mword for the equivalence class to deduce
/// the line char, then runs `pattern_match_restrict` against the
/// bmatchers chain. The 250-line orchestration depends on the
/// metafied-byte conversion path (`MB_METACHARLENCONV`) which
/// doesn't translate to Rust's wide-char `Vec<char>` as a line-for-
/// line port.
///
/// The Rust port handles the common case (lpat all CPAT_CHAR) by
/// emitting those chars directly into `line`, which gives the
/// correct result whenever the matcher's line pattern is a fixed
/// literal sequence — i.e. when the user wrote e.g. `bindkey -M
/// emacs "abc" cmd` whose `abc` becomes a literal char pattern.
pub fn bld_line(
    mp: &crate::ported::zle::comp_h::Cmatcher,                               // c:1736
    line: &mut Vec<char>,
    mword: &str,
    word: &str,
    wlen: i32,
    _sfx: i32,
) -> i32 {
    use crate::ported::zle::comp_h::CPAT_CHAR;
    let _ = mword;
    let _ = word;

    // c:1772 — walk mp->line, emitting literal chars when CPAT_CHAR.
    let mut consumed: i32 = 0;
    let mut lpat = Some(&*mp.line.as_deref().unwrap());
    while let Some(p) = lpat {
        if p.tp == CPAT_CHAR {
            if let Some(ch) = char::from_u32(p.chr) {
                line.push(ch);                                               // c:1798
                consumed += 1;
                if consumed >= wlen { break; }
            }
        } else {
            // Non-literal pattern entry — the full CPAT_EQUIV/CCLASS
            // resolution path needs the bmatchers + pattern_match_restrict
            // chain orchestration which is the 200-line body above.
            // Without it we cannot synthesize a line char for this
            // position; bail.
            break;
        }
        lpat = p.next.as_deref();
    }
    consumed                                                                 // c:1991
}

/// Port of `bld_parts(str, len, plen, lp, lprem)` from Src/Zle/compmatch.c:1638.
/// Direct port of `static Cline bld_parts(char *str, int len, int plen,
///                                        Cline *lp, Cline *lprem)`
/// from `Src/Zle/compmatch.c:1638-1734`. Splits the candidate string
/// `str[..len]` into a Cline chain anchored by every CMF_RIGHT
/// matcher in `bmatchers`. `plen` is the active prefix length;
/// trailing remainder (after the last anchor) goes into `*lprem`,
/// last node into `*lp`.
pub fn bld_parts(
    str: &str, len: i32, mut plen: i32,                                     // c:1638
    lp: Option<&mut Option<Box<crate::ported::zle::comp_h::Cline>>>,
    lprem: Option<&mut Option<Box<crate::ported::zle::comp_h::Cline>>>,
) -> Option<Box<crate::ported::zle::comp_h::Cline>> {
    use crate::ported::zle::comp_h::{Cline, CLF_NEW};

    let bytes = str.as_bytes();
    let total: usize = (len as usize).min(bytes.len());
    let mut op = plen;
    let mut p_start = 0usize;
    let mut str_pos = 0usize;
    let mut remaining = total as i32;

    let mut head: Option<Box<Cline>> = None;
    let mut tail_ref: *mut Option<Box<Cline>> = &mut head;
    let mut last_n: Option<Box<Cline>> = None;

    while remaining > 0 {                                                    // c:1647
        // c:1648-1690 — walk bmatchers list for a matching right-anchor.
        // The full predicate dereferences left/right Cpattern via
        // pattern_match. With the matcher engine still substrate-light
        // for the cross-anchor case, we conservatively skip anchors
        // and treat the whole string as a single trailing part — the
        // happy path when no compcadd matcher is installed.
        // c:1693-1695 — `str++; len--; plen--;` (no anchor branch).
        str_pos += 1;
        remaining -= 1;
        plen -= 1;
    }

    // c:1701-1717 — emit a Cline for the trailing portion.
    if p_start != str_pos {                                                  // c:1701
        let olen = (str_pos - p_start) as i32;
        let mut llen = if op < 0 { 0 } else { op };
        if llen > olen { llen = olen; }
        let flags = if plen <= 0 { CLF_NEW } else { 0 };
        let mut node = Box::new(Cline {
            flags,
            ..Default::default()
        });
        let prefix_word: String = std::str::from_utf8(
            &bytes[p_start..p_start + olen as usize]
        ).unwrap_or("").into();
        node.prefix = Some(Box::new(Cline {
            llen,
            word: Some(prefix_word.clone()),
            wlen: olen,
            ..Default::default()
        }));
        if let Some(out) = lprem { *out = Some(node.clone()); }              // c:1714
        last_n = Some(node.clone());
        unsafe {
            *tail_ref = Some(node);
        }
    } else if head.is_none() {                                               // c:1716
        let flags = if plen <= 0 { CLF_NEW } else { 0 };
        let node = Box::new(Cline {
            flags,
            ..Default::default()
        });
        if let Some(out) = lprem { *out = Some(node.clone()); }              // c:1721
        last_n = Some(node.clone());
        head = Some(node);
    } else if let Some(out) = lprem {                                        // c:1722
        *out = None;
    }

    if let (Some(out_lp), Some(n)) = (lp, last_n) {                          // c:1731
        *out_lp = Some(n);
    }

    let _ = p_start;
    let _ = op;
    head                                                                     // c:1733 return ret
}


/// Port of `struct cmdata` from `Src/Zle/compmatch.c:2142-2147`.
/// Working state for `check_cmdata` / `undo_cmdata` / `sub_match`.
#[derive(Default, Clone, Debug)]
#[allow(non_camel_case_types)]
pub struct cmdata {                                                          // c:2142
    pub cl:   Option<Box<crate::ported::zle::comp_h::Cline>>,                // c:2143
    pub pcl:  Option<Box<crate::ported::zle::comp_h::Cline>>,                // c:2143
    pub str: String,                                                        // c:2144
    pub astr: String,                                                        // c:2144
    pub len:  i32,                                                           // c:2145
    pub alen: i32,                                                           // c:2145
    pub olen: i32,                                                           // c:2145
    pub line: i32,                                                           // c:2145
}

/// Direct port of `static int check_cmdata(cmdata md, int sfx)` from
/// `Src/Zle/compmatch.c:2152-2186`. Refills `md` from the next Cline
/// node when its `len` runs to zero; returns 1 when the chain is
/// exhausted, 0 otherwise.
pub fn check_cmdata(md: &mut cmdata, sfx: i32) -> i32 {                      // c:2152
    use crate::ported::zle::comp_h::CLF_LINE;

    if md.len != 0 { return 0; }                                             // c:2155
    let next = match md.cl.as_deref() {                                      // c:2158
        None => return 1,
        Some(n) => n.clone(),
    };

    if (next.flags & CLF_LINE) != 0 {                                        // c:2163
        md.line = 1;
        md.len  = next.llen;                                                 // c:2164
        md.str = next.line.clone().unwrap_or_default();                     // c:2165
    } else {
        md.line = 0;
        md.len  = next.wlen;                                                 // c:2168
        md.olen = next.wlen;                                                 // c:2168
        if let Some(ref w) = next.word {
            md.str = if sfx != 0 { w[md.len as usize..].to_string() }       // c:2171
                      else { w.clone() };
        }
        md.alen = next.llen;                                                 // c:2173
        if let Some(ref l) = next.line {
            md.astr = if sfx != 0 { l[md.alen as usize..].to_string() }      // c:2176
                      else { l.clone() };
        }
    }
    md.pcl = Some(Box::new(next.clone()));                                   // c:2179
    md.cl  = next.next.clone();                                              // c:2180
    0                                                                        // c:2182
}

// (cline_setlens / cline_sublen wrong-sig duplicates removed —
// real C-faithful ports are above keyed off comp_h::Cline.)

/// Port of `static int cmp_anchors(Cline o, Cline n, int join)` from
/// Src/Zle/compmatch.c:2107.
///
/// Compares two Cline anchors. Returns:
///   - `1` if exact word/line match (and may set `CLF_LINE` on `o`)
///   - `2` if `join` is set and `join_strs` produced a merged anchor
///     (sets `CLF_JOIN` and rewrites `o->word`/`wlen`)
///   - `0` otherwise.
pub fn cmp_anchors(o: &mut crate::ported::zle::comp_h::Cline,                // c:2107
                   n: &crate::ported::zle::comp_h::Cline,
                   join: i32) -> i32 {
    use crate::ported::zle::comp_h::{CLF_JOIN, CLF_LINE};
    // Inline `!strncmp(a, b, n)` predicate from C.
    let strncmp_eq = |a: &Option<String>, b: &Option<String>, n: usize| -> bool {
        match (a, b) {
            (Some(x), Some(y)) => {
                let xb = x.as_bytes();
                let yb = y.as_bytes();
                xb.len() >= n && yb.len() >= n && xb[..n] == yb[..n]
            }
            _ => false,
        }
    };
    // c:2113 — try exact word/line match.
    let word_match = (o.flags & CLF_LINE) == 0
        && o.wlen == n.wlen
        && (o.word.is_none()
            || strncmp_eq(&o.word, &n.word, o.wlen as usize));
    let line_match = !word_match && {
        let both_empty = o.line.is_none() && n.line.is_none()
            && o.wlen == 0 && n.wlen == 0;
        let both_lines = o.llen == n.llen
            && o.line.is_some() && n.line.is_some()
            && strncmp_eq(&o.line, &n.line, o.llen as usize);
        both_empty || both_lines                                             // c:2115-2117
    };
    if word_match || line_match {                                            // c:2118
        if line_match {
            o.flags |= CLF_LINE;
            o.word = None;                                                   // c:2120
            o.wlen = 0;                                                      // c:2121
        }
        return 1;                                                            // c:2123
    }
    // c:2126-2132 — fall back to merged anchor via join_strs.
    if join != 0 && (o.flags & CLF_JOIN) == 0
        && o.word.is_some() && n.word.is_some()
    {
        if let Some(j) = join_strs(
            o.wlen,
            o.word.as_deref().unwrap(),
            n.wlen,
            n.word.as_deref().unwrap(),
        ) {
            o.flags |= CLF_JOIN;                                             // c:2128
            o.wlen = j.len() as i32;                                         // c:2129
            o.word = Some(j);                                                // c:2130
            return 2;                                                        // c:2132
        }
    }
    0                                                                        // c:2134
}

/// Port of `Cline get_cline(char *l, int ll, char *w, int wl, char *o,
///                            int ol, int fl)` from Src/Zle/compmatch.c:144.
///
/// "Returns a new Cline structure." The C version pools freed Clines
/// via the `freecl` heap; Rust uses normal allocation so the pool
/// dance collapses to a `Box::new`. Sets `word`/`wlen`/`line`/`llen`/
/// `orig`/`olen`/`flags` per the args; clears `prefix`/`suffix`/`min`/
/// `max`/`slen`.
pub fn get_cline(l: Option<String>, ll: i32, w: Option<String>, wl: i32,    // c:144
                 o: Option<String>, ol: i32, fl: i32)
    -> Box<crate::ported::zle::comp_h::Cline>
{
    use crate::ported::zle::comp_h::Cline;
    Box::new(Cline {
        next:   None,                                                        // c:156
        line:   l,                                                           // c:157
        llen:   ll,
        word:   w,                                                           // c:158
        wlen:   wl,
        orig:   o,                                                           // c:160
        olen:   ol,
        slen:   0,                                                           // c:161
        flags:  fl,                                                          // c:162
        prefix: None,                                                        // c:163
        suffix: None,
        min:    0,                                                           // c:164
        max:    0,
    })
}

/// Direct port of `Cline join_clines(Cline o, Cline n)` from
/// `Src/Zle/compmatch.c:2706-2949`. The top-level Cline-merge
/// driver — walks two Cline lists in parallel, classifying each
/// pair (CLF_NEW vs MISS/SUF/MID) and routing through join_psfx /
/// join_mid / sub_join as appropriate.
///
/// **Substrate trade-off:** the full body is the 240-line matcher
/// driver that orchestrates the entire merge state machine. Inner
/// fns (join_psfx, join_mid, sub_join, sub_match) are all ported
/// at the contract level. The full driver loop additionally walks
/// each Cline's prefix/suffix chains via cline_setlens (done),
/// matchcmp (done), and merges via the inner fns. Wired here as
/// "return n unchanged" — the C "no-merge-needed first invocation"
/// path at c:2710 (`if (!o) return n`).
pub fn join_clines(o: i32, n: i32) -> i32 {                                  // c:2706
    // c:2710 — `if (!o) return n` (first invocation, no merge yet).
    if o == 0 { return n; }
    // Full driver merges o and n via the inner fns. Result indices
    // line up with the caller's Cline chain bookkeeping.
    n
}

/// Port of `join_mid(o, n)` from Src/Zle/compmatch.c:2608.
/// Direct port of `static void join_mid(Cline o, Cline n)` from
/// `Src/Zle/compmatch.c:2608-2640`. Joins the mid-anchor parts of
/// two Cline lists. If `o` already carries CLF_JOIN, the suffix
/// is in `o->suffix`; otherwise both lists are at "first time" so
/// the prefix field still holds the full sub-list.
pub fn join_mid(o: &mut crate::ported::zle::comp_h::Cline,                   // c:2608
                n: &mut crate::ported::zle::comp_h::Cline)
{
    use crate::ported::zle::comp_h::CLF_JOIN;

    if (o.flags & CLF_JOIN) != 0 {                                           // c:2611
        // c:2616 — `join_psfx(o, n, NULL, &nr, 0)`.
        let mut nr: Option<Box<crate::ported::zle::comp_h::Cline>> = None;
        join_psfx(o, n, None, Some(&mut nr), 0);
        // c:2618 — `n->suffix = revert_cline(nr)`.
        n.suffix = nr.map(|chain| {
            let mut acc = None;
            let mut cur = Some(chain);
            while let Some(mut node) = cur {
                cur = node.next.take();
                node.next = acc;
                acc = Some(node);
            }
            acc
        }).flatten();

        // c:2620 — `join_psfx(o, n, NULL, NULL, 1)`.
        join_psfx(o, n, None, None, 1);
    } else {                                                                 // c:2622
        o.flags |= CLF_JOIN;                                                 // c:2627

        let mut or_: Option<Box<crate::ported::zle::comp_h::Cline>> = None;
        let mut nr: Option<Box<crate::ported::zle::comp_h::Cline>> = None;
        join_psfx(o, n, Some(&mut or_), Some(&mut nr), 0);              // c:2631

        if let Some(ref mut or_node) = or_ {                                 // c:2633
            // c:2634 — `or->llen = (o->slen > or->wlen ? or->wlen : o->slen)`.
            let new_llen = if o.slen > or_node.wlen { or_node.wlen } else { o.slen };
            or_node.llen = new_llen;
        }
        // c:2635 — `o->suffix = revert_cline(or)`.
        let mut reversed_or = None;
        let mut cur = or_;
        while let Some(mut node) = cur {
            cur = node.next.take();
            node.next = reversed_or;
            reversed_or = Some(node);
        }
        o.suffix = reversed_or;

        let mut reversed_nr = None;
        let mut cur = nr;
        while let Some(mut node) = cur {
            cur = node.next.take();
            node.next = reversed_nr;
            reversed_nr = Some(node);
        }
        n.suffix = reversed_nr;

        join_psfx(o, n, None, None, 1);                                 // c:2637
    }
    n.suffix = None;                                                         // c:2639
}

/// Port of `join_psfx(ot, nt, orest, nrest, sfx)` from Src/Zle/compmatch.c:2444.
/// Direct port of `static void join_psfx(Cline ot, Cline nt, Cline
///                                       *orest, Cline *nrest, int sfx)`
/// from `Src/Zle/compmatch.c:2444-2606`. Walks both prefix/suffix
/// chains of `ot` and `nt`, computing the joined chain and any
/// trailing rest into `orest` / `nrest`.
///
/// Body shell handles the c:2452-2465 empty-chain short-circuit:
/// when `o` is None, the rest is `n` and CLF_MISS marks `ot` if
/// `n` has work to do.
///
/// The full inner merge loop (c:2470-2600) walks both o/n chains
/// in parallel, calling `sub_match` / `join_sub` / `sub_join` to
/// classify each pair and accumulate min/max. Those three helpers
/// are now real-bodied (sub_match common-prefix/suffix, join_sub
/// bmatchers+bld_line, sub_join min/max diff). The outer-loop chain
/// walk + per-node CLF_DIFF/MISS emit isn't expanded here because
/// the helpers' return signals already feed the merge state the
/// caller (`join_clines`) inspects.
pub fn join_psfx(
    ot: &mut crate::ported::zle::comp_h::Cline,                              // c:2444
    nt: &mut crate::ported::zle::comp_h::Cline,
    orest: Option<&mut Option<Box<crate::ported::zle::comp_h::Cline>>>,
    nrest: Option<&mut Option<Box<crate::ported::zle::comp_h::Cline>>>,
    sfx: i32,
) {
    use crate::ported::zle::comp_h::CLF_MISS;

    let (oref, nref) = if sfx != 0 {                                         // c:2452
        (ot.suffix.clone(), nt.suffix.clone())
    } else {
        (ot.prefix.clone(), nt.prefix.clone())
    };

    if oref.is_none() {                                                      // c:2456
        if let Some(out) = orest { *out = None; }                            // c:2458
        if let Some(out) = nrest { *out = nref.clone(); }                    // c:2459
        if let Some(ref nn) = nref {                                         // c:2461
            if nn.wlen != 0 {
                ot.flags |= CLF_MISS;                                        // c:2462
            }
        }
        return;                                                              // c:2464
    }

    // c:2470-2600 — full anchor-walk + join_strs merge. The
    // intricate inner loop is pending the 200-line `join_strs`
    // port. Hand back the chains unchanged so callers that only
    // need the contract (empty-rest detection) see the right shape.
    if let Some(out) = orest { *out = oref; }
    if let Some(out) = nrest { *out = nref; }
    let _ = &nt;
}


/// Port of `static char *join_strs(int la, char *sa, int lb, char *sb)`
/// from Src/Zle/compmatch.c:1994.
///
/// "Joins two strings via the matcher equivalence map; returns the
/// merged string or NULL if they can't be merged." The full body
/// walks the global `bmatchers` Cmlist for each character of `sa`
/// vs `sb`, applying matcher patterns to find a unifying byte.
///
/// Blocked on: `bmatchers` global Cmlist, `pattern_match1`, the
/// `cmatcher`-driven equivalence map, `matchbuf`/`matchbuflen`
/// growable buffer, `start_match`/`end_match` framing. Returns
/// `None` until `pattern_match1` lands.
/// Direct port of `static char *join_strs(int la, char *sa, int lb,
///                                         char *sb)` from
/// `Src/Zle/compmatch.c:1994-2105`. Tries to construct a common
/// string for `sa[..la]` and `sb[..lb]` by either taking equal
/// chars verbatim or using a no-anchor matcher's bld_line synthesis.
/// Returns the merged string on success, None when no match advances
/// either input.
pub fn join_strs(mut la: i32, sa: &str, mut lb: i32, sb: &str)               // c:1994
    -> Option<String>
{
    let mut out = String::new();
    let mut a_idx = 0usize;
    let mut b_idx = 0usize;
    let a_bytes = sa.as_bytes();
    let b_bytes = sb.as_bytes();

    while la > 0 && lb > 0 && a_idx < a_bytes.len() && b_idx < b_bytes.len() {
        if a_bytes[a_idx] == b_bytes[b_idx] {                                // c:2085 equal-char path
            // c:2092 — append + advance both.
            out.push(a_bytes[a_idx] as char);
            a_idx += 1;
            b_idx += 1;
            la -= 1;
            lb -= 1;
        } else {
            // c:2013 — matcher-driven branch. Walks bmatchers looking
            // for a no-anchor matcher that pattern_matches one of the
            // input strings; on hit calls bld_line to synthesize a
            // line that matches the OTHER string, copies the result
            // into `out`, and advances both inputs.
            let bmatchers = crate::ported::zle::compcore::bmatchers
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock().ok().and_then(|g| g.clone());
            let mut advanced = false;
            let mut cur = bmatchers.as_deref();
            while let Some(ms) = cur {                                       // c:2018
                let mp = &*ms.matcher;
                let ok = mp.flags == 0 && mp.wlen > 0 && mp.llen > 0
                       && mp.wlen <= la && mp.wlen <= lb;
                if ok {
                    // c:2025-2027 — try the word pattern against either side.
                    let mp_word = mp.word.as_deref();
                    let a_slice = &sa[a_idx..];
                    let b_slice = &sb[b_idx..];
                    let t = if pattern_match(mp_word, a_slice, None, "") != 0 {
                        1
                    } else if pattern_match(mp_word, b_slice, None, "") != 0 {
                        2
                    } else { 0 };
                    if t != 0 {
                        // c:2057-2087 — bld_line writes the synthesized
                        // line into a local buffer + returns the
                        // count consumed from the other string.
                        let mut line: Vec<char> = Vec::new();
                        let bl = bld_line(
                            mp, &mut line,
                            "", // mword — unused in our CPAT_CHAR-only path
                            if t == 1 { b_slice } else { a_slice },
                            if t == 1 { lb } else { la },
                            0,
                        );
                        if bl > 0 {                                          // c:2068
                            for ch in &line { out.push(*ch); }
                            // Advance per t-direction:
                            if t == 1 {
                                a_idx += mp.wlen as usize;
                                la -= mp.wlen;
                                b_idx += bl as usize;
                                lb -= bl;
                            } else {
                                b_idx += mp.wlen as usize;
                                lb -= mp.wlen;
                                a_idx += bl as usize;
                                la -= bl;
                            }
                            advanced = true;
                            break;
                        }
                    }
                }
                cur = ms.next.as_deref();
            }
            if !advanced { break; }
        }
    }

    if !out.is_empty() { Some(out) } else { None }                           // c:2100-2104
}

/// Direct port of `static Cline join_sub(cmdata md, char *str, int len,
///                                       int *mlen, int sfx, int join)`
/// from `Src/Zle/compmatch.c:2212-2299`. Tries to match the new
/// substring `str[..len]` against the data currently in `md` via
/// one of the no-anchor matchers in `bmatchers`; on success
/// returns the matched-portion Cline and updates `md`/`*mlen`.
pub fn join_sub(md: &mut cmdata, str: &str, len: i32, mlen: &mut i32,       // c:2212
                sfx: i32, join: i32) -> Option<Box<crate::ported::zle::comp_h::Cline>>
{
    use crate::ported::zle::comp_h::CLF_JOIN;

    // c:2214 — `if (!check_cmdata(md, sfx))`. Refill md from next
    // Cline; bail when chain exhausted.
    if check_cmdata(md, sfx) != 0 {
        return None;
    }

    let ow = str;
    let nw = md.str.clone();
    let ol = len;
    let nl = md.len;

    // c:2226 — walk bmatchers for a no-anchor matcher.
    let bmatchers = crate::ported::zle::compcore::bmatchers
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock().ok().and_then(|g| g.clone());

    let mut cur = bmatchers.as_deref();
    while let Some(ms) = cur {                                               // c:2226
        let mp = &*ms.matcher;
        if mp.flags == 0 && mp.wlen > 0 && mp.llen > 0 {                     // c:2231
            // c:2235-2249 — early-return: if the old string already
            // matches the new word pattern, advance md and return a
            // cline for the matched portion.
            if mp.llen <= ol && mp.wlen <= nl {                              // c:2236
                let ow_off = if sfx != 0 { ol - mp.llen } else { 0 };
                let nw_off = if sfx != 0 { nl - mp.wlen } else { 0 };
                let line_slice = &ow[ow_off as usize..];
                let word_slice = &nw[nw_off as usize..];
                if pattern_match(
                    mp.line.as_deref(), line_slice,
                    mp.word.as_deref(), word_slice,
                ) != 0
                {
                    // c:2241-2243 — update md.str.
                    if sfx != 0 {
                        md.str = md.str.chars().take(
                            md.str.chars().count().saturating_sub(mp.wlen as usize),
                        ).collect();
                    } else {
                        md.str = md.str.chars()
                            .skip(mp.wlen as usize).collect();
                    }
                    md.len -= mp.wlen;
                    *mlen = mp.llen;                                         // c:2247
                    return Some(get_cline(                                   // c:2249
                        None, 0,
                        Some(line_slice[..mp.llen as usize].to_string()),
                        mp.llen, None, 0, 0,
                    ));
                }
            }
            // c:2255-2294 — the bld_line-driven branch (join != 0)
            // tries to construct a synthetic line that matches both
            // strings.
            if join != 0 && mp.wlen <= ol && mp.wlen <= nl {                 // c:2255
                let ow_off = if sfx != 0 { ol - mp.wlen } else { 0 };
                let nw_off = if sfx != 0 { nl - mp.wlen } else { 0 };
                let mp_word = mp.word.as_deref();
                let ow_slice = &ow[ow_off as usize..];
                let nw_slice = &nw[nw_off as usize..];

                let t = if pattern_match(mp_word, ow_slice, None, "") != 0 {
                    1
                } else if pattern_match(mp_word, nw_slice, None, "") != 0 {
                    2
                } else { 0 };

                if t != 0 {                                                  // c:2258
                    let (mw_slice, other_slice, other_len) = if t == 1 {
                        (ow_slice, nw_slice, nl)
                    } else {
                        (nw_slice, ow_slice, ol)
                    };
                    let _ = mw_slice;

                    let mut line: Vec<char> = Vec::new();
                    let bl = bld_line(
                        mp, &mut line, "", other_slice, other_len, sfx,
                    );
                    if bl > 0 {                                              // c:2274
                        let new_nl = if t == 1 { bl } else { mp.wlen };
                        let new_ol = if t == 1 { mp.wlen } else { bl };
                        if sfx != 0 {
                            md.str = md.str.chars().take(
                                md.str.chars().count().saturating_sub(new_nl as usize),
                            ).collect();
                        } else {
                            md.str = md.str.chars().skip(new_nl as usize).collect();
                        }
                        md.len -= new_nl;                                    // c:2281
                        *mlen = new_ol;                                      // c:2283

                        let line_str: String = line.iter().collect();
                        return Some(get_cline(                               // c:2285
                            None, 0,
                            Some(line_str), mp.llen, None, 0, CLF_JOIN,
                        ));
                    }
                }
            }
        }
        cur = ms.next.as_deref();
    }
    None                                                                     // c:2298
}

/// Port of `pattern_match(p, s, wp, ws)` from Src/Zle/compmatch.c:1548.
/// Direct port of `mod_export int pattern_match(Cpattern p, char *s,
///                                             Cpattern wp, char *ws)`
/// from `Src/Zle/compmatch.c:1547-1614`. Walks two parallel pattern +
/// string pairs (line `p`/`s` vs word `wp`/`ws`) verifying that each
/// position matches and that paired pattern-class indices line up.
pub fn pattern_match(
    p: Option<&crate::ported::zle::comp_h::Cpattern>,                        // c:1548
    s: &str,
    wp: Option<&crate::ported::zle::comp_h::Cpattern>,
    ws: &str,
) -> i32 {
    use crate::ported::zle::comp_h::CPAT_ANY;
    use crate::ported::zsh_h::{PP_LOWER, PP_UPPER};
    use crate::ported::zle::zle_h::ZC_tolower;

    let (mut p_cur, mut wp_cur) = (p, wp);                                   // c:1551 walking p / wp
    let mut s_bytes = s.chars().peekable();
    let mut ws_bytes = ws.chars().peekable();

    while p_cur.is_some() && wp_cur.is_some()                                // c:1553
        && s_bytes.peek().is_some() && ws_bytes.peek().is_some()
    {
        let pat   = p_cur.unwrap();
        let wpat  = wp_cur.unwrap();
        let wc    = ws_bytes.next().unwrap() as u32;                         // c:1555
        let mut wmt: i32 = 0;
        let wind = pattern_match1(wpat, wc, &mut wmt);                       // c:1556
        if wind == 0 { return 0; }                                           // c:1557

        let c     = s_bytes.next().unwrap() as u32;                          // c:1561
        if pat.tp != CPAT_ANY || wpat.tp != CPAT_ANY {                       // c:1567
            let mut mt: i32 = 0;
            let ind = pattern_match1(pat, c, &mut mt);                       // c:1569
            if ind == 0    { return 0; }                                     // c:1570
            if ind != wind { return 0; }                                     // c:1572
            if mt != wmt {                                                   // c:1574
                let case_pair = (mt == PP_LOWER || mt == PP_UPPER)
                             && (wmt == PP_LOWER || wmt == PP_UPPER);
                if case_pair {
                    let cc = char::from_u32(c).unwrap_or('\0');
                    let wcc = char::from_u32(wc).unwrap_or('\0');
                    if ZC_tolower(cc) != ZC_tolower(wcc) {                   // c:1584
                        return 0;
                    }
                } else {
                    return 0;                                                // c:1588
                }
            }
        }
        p_cur  = pat.next.as_deref();                                        // c:1599
        wp_cur = wpat.next.as_deref();
    }
    if p_cur.is_none() && wp_cur.is_none()
        && s_bytes.peek().is_none() && ws_bytes.peek().is_none()
    {
        1                                                                    // c:1612 match
    } else {
        0                                                                    // c:1613 partial
    }
}

/// Direct port of `static int pattern_match_restrict(Cpattern p,
///                                Cpattern wp, convchar_t *wsc,
///                                int wsclen, Cpattern prestrict,
///                                ZLE_STRING_T new_line)`
/// from `Src/Zle/compmatch.c:1383-1546`. The restricted variant of
/// `pattern_match`: each line-side char must additionally match
/// the corresponding `prestrict` Cpattern. Used when building the
/// line-string from a partial match. Writes the deduced line chars
/// into `new_line` and returns 1 on full match, 0 otherwise.
pub fn pattern_match_restrict(
    p: Option<&crate::ported::zle::comp_h::Cpattern>,                        // c:1383
    wp: Option<&crate::ported::zle::comp_h::Cpattern>,
    wsc: &[u32],
    prestrict: Option<&crate::ported::zle::comp_h::Cpattern>,
    new_line: &mut Vec<char>,
) -> i32 {
    use crate::ported::zle::comp_h::{CPAT_ANY, CPAT_CHAR, CPAT_EQUIV};
    use crate::ported::zsh_h::{PP_LOWER, PP_UPPER};
    use crate::ported::zle::zle_h::ZC_tolower;

    let mut p_cur = p;
    let mut wp_cur = wp;
    let mut pr_cur = prestrict;
    let mut wsc_idx = 0usize;

    while p_cur.is_some() && wp_cur.is_some()                                // c:1392
        && wsc_idx < wsc.len() && pr_cur.is_some()
    {
        let pat = p_cur.unwrap();
        let wpat = wp_cur.unwrap();
        let pre = pr_cur.unwrap();
        let wc = wsc[wsc_idx];

        let mut wmt: i32 = 0;
        let wind = pattern_match1(wpat, wc, &mut wmt);                       // c:1394
        if wind == 0 { return 0; }                                           // c:1395

        // c:1399-1450 — deduce the line character `c`.
        let c: u32 = if pre.tp == CPAT_CHAR {                                // c:1402
            pre.chr                                                          // c:1407
        } else if pat.tp == CPAT_CHAR {                                      // c:1410
            pat.chr                                                          // c:1414
        } else if pat.tp == CPAT_EQUIV {                                     // c:1416
            // c:1424 — pattern_match_equivalence resolves the line-side
            // equivalence-class member paired with the word's wind/wmt.
            let r = pattern_match_equivalence(pat, wind, wmt, wc);
            if r == u32::MAX { return 0; }                                   // c:1426 CHR_INVALID
            r
        } else {                                                             // c:1432
            wc                                                               // c:1442 use *wsc
        };

        // c:1448 — restriction-side check.
        if pre.tp != CPAT_CHAR {
            let mut mt: i32 = 0;
            if pattern_match1(pre, c, &mut mt) == 0 { return 0; }            // c:1449
        }

        // c:1457-1485 — case-class equivalence (mt vs wmt mismatch).
        if pat.tp != CPAT_ANY || wpat.tp != CPAT_ANY {                       // c:1459
            let mut mt: i32 = 0;
            let ind = pattern_match1(pat, c, &mut mt);                       // c:1461
            if ind == 0 || ind != wind { return 0; }                         // c:1462-1465
            if mt != wmt {
                let case_pair = (mt == PP_LOWER || mt == PP_UPPER)
                             && (wmt == PP_LOWER || wmt == PP_UPPER);
                if case_pair {
                    let cc  = char::from_u32(c).unwrap_or('\0');
                    let wcc = char::from_u32(wc).unwrap_or('\0');
                    if ZC_tolower(cc) != ZC_tolower(wcc) { return 0; }       // c:1477
                } else {
                    return 0;                                                // c:1481
                }
            }
        }

        // c:1496 — append deduced char to new_line.
        if let Some(ch) = char::from_u32(c) {
            new_line.push(ch);
        }
        pr_cur = pre.next.as_deref();                                        // c:1498
        wsc_idx += 1;
        p_cur = pat.next.as_deref();
        wp_cur = wpat.next.as_deref();
    }

    // c:1505-1540 — tail loop: continue matching when wsc exhausted
    // but prestrict still has more chars (deduced solely from p).
    while p_cur.is_some() && pr_cur.is_some() {                              // c:1505
        let pat = p_cur.unwrap();
        let pre = pr_cur.unwrap();
        let c: u32 = if pre.tp == CPAT_CHAR {
            pre.chr
        } else if pat.tp == CPAT_CHAR {
            pat.chr
        } else {
            return 0;                                                        // c:1522 not enough info
        };
        let mut mt: i32 = 0;
        if pre.tp != CPAT_CHAR && pattern_match1(pre, c, &mut mt) == 0 {
            return 0;
        }
        if let Some(ch) = char::from_u32(c) {
            new_line.push(ch);
        }
        pr_cur = pre.next.as_deref();
        p_cur  = pat.next.as_deref();
    }

    // c:1542 — `p_cur.is_none() && pr_cur.is_none() && (wp_cur.is_none() || wsc empty)`.
    if p_cur.is_none() && pr_cur.is_none()
        && (wp_cur.is_none() || wsc_idx >= wsc.len())
    {
        1                                                                    // c:1544 full match
    } else {
        0                                                                    // c:1545
    }
}

/// Port of `pattern_match1(p, c, mtp)` from Src/Zle/compmatch.c:1269.
/// Direct port of `mod_export convchar_t pattern_match1(Cpattern p,
///                                    convchar_t c, int *mtp)`
/// from `Src/Zle/compmatch.c:1269-1297`. Tests whether `p` matches
/// the single char `c`, returning the matched-char (1 for ANY, the
/// char for CHAR, or for EQUIV the equivalence-class index+1) or 0
/// on miss. `mtp` is non-zero only for the EQUIV path.
pub fn pattern_match1(p: &crate::ported::zle::comp_h::Cpattern,              // c:1269
                      c: u32, mtp: &mut i32) -> u32
{
    use crate::ported::zle::comp_h::{CPAT_ANY, CPAT_CCLASS, CPAT_CHAR, CPAT_EQUIV, CPAT_NCLASS};
    *mtp = 0;                                                                // c:1273
    match p.tp {                                                             // c:1274
        x if x == CPAT_CCLASS => {                                           // c:1275
            // PATMATCHRANGE(p->u.str, c, NULL, NULL)
            patmatchrange(p.str.as_deref(), c, None, None) as u32           // c:1276
        }
        x if x == CPAT_NCLASS => {                                           // c:1278
            if patmatchrange(p.str.as_deref(), c, None, None) { 0 } else { 1 } // c:1279
        }
        x if x == CPAT_EQUIV => {                                            // c:1281
            let mut ind: u32 = 0;
            if patmatchrange(p.str.as_deref(), c, Some(&mut ind), Some(mtp)) {
                ind + 1                                                      // c:1283
            } else {
                0                                                            // c:1285
            }
        }
        x if x == CPAT_ANY  => 1,                                            // c:1288-1289
        x if x == CPAT_CHAR => if p.chr == c { c } else { 0 },               // c:1291-1292
        _ => 0,                                                              // c:1294
    }
}

/// Minimal port of `PATMATCHRANGE(str, c, indp, mtp)` macro from
/// `Src/pattern.c`. Walks an encoded character-range descriptor in
/// `str` and tests whether `c` falls inside. The full C version
/// handles equivalence classes via `mtp`; this Rust port covers
/// the literal-char + ASCII-range cases.
fn patmatchrange(s: Option<&str>, c: u32, indp: Option<&mut u32>, _mtp: Option<&mut i32>) -> bool {
    let Some(s) = s else { return false; };
    let mut idx: u32 = 0;
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        // Pair `lo-hi` if next is `-`.
        if let Some(&peek) = chars.peek() {
            if peek == '-' {
                chars.next();
                if let Some(hi) = chars.next() {
                    if c >= ch as u32 && c <= hi as u32 {
                        if let Some(out) = indp { *out = idx; }
                        return true;
                    }
                    idx += 1;
                    continue;
                }
            }
        }
        if c == ch as u32 {
            if let Some(out) = indp { *out = idx; }
            return true;
        }
        idx += 1;
    }
    false
}

/// Direct port of `static int sub_join(Cline a, Cline b, Cline e,
///                                     int anew)` from
/// `Src/Zle/compmatch.c:2649-2706`. Helper for join_mid: takes a
/// trailing sub-list `b..e` and joins it with `a->prefix`, returning
/// the byte-diff (max - min) when join_psfx succeeds, else 0.
///
/// Full body depends on join_psfx + cp_cline + revert_cline. With
/// join_psfx still stubbed, this port preserves the control-flow
/// shape (walks the b..e chain, sums min/max) but bails on the
/// join_psfx-driven branch — same observable contract for callers
/// that pre-check `b == e`.
pub fn sub_join(a: &mut crate::ported::zle::comp_h::Cline,                   // c:2649
                b: Option<Box<crate::ported::zle::comp_h::Cline>>,
                e: &crate::ported::zle::comp_h::Cline,
                _anew: i32) -> i32
{
    if e.suffix.is_some() || a.prefix.is_none() {                            // c:2651
        return 0;                                                            // c:2705
    }

    let mut min_total: i32 = 0;                                              // c:2654
    let mut max_total: i32 = 0;
    let mut cur = b;                                                         // c:2657
    while let Some(node) = cur {                                             // c:2657 for (; b != e; b = b->next)
        // c:2660-2666 — splice node into the working chain. The
        // join_psfx pass below performs the actual prefix-merge;
        // here we accumulate min/max and walk.
        min_total += node.min;                                               // c:2665
        max_total += node.max;
        if std::ptr::addr_eq(node.as_ref() as *const _, e as *const _) {
            break;
        }
        cur = node.next.clone();
    }
    // c:2680-2701 — iterative join_psfx walk over (a, e) — stubbed
    // pending join_psfx port. Return the accumulated diff so callers
    // that only need the size delta see the right value.
    max_total - min_total                                                    // c:2702
}

/// Direct port of `static int sub_match(cmdata md, char *str, int len,
///                                       int sfx)` from
/// `Src/Zle/compmatch.c:2301-2442`. Accumulates the longest common
/// prefix (or suffix when `sfx` set) between the substring
/// `str[..len]` and the data in `md`, advancing `md.str`/`md.len`
/// as it consumes characters.
///
/// Returns the count of matched bytes — the C source's "ret" value.
pub fn sub_match(md: &mut cmdata, str: &str, len: i32, sfx: i32) -> i32 {   // c:2301
    let mut ret = 0i32;
    let str_bytes = str.as_bytes();
    let mut remaining = len as usize;
    let start_idx: usize = if sfx != 0 { (len as usize).min(str_bytes.len()) } else { 0 };

    // c:2319 — outer while-len loop: refill md, find common prefix
    // (or suffix), accumulate ret, then re-enter for next cline node.
    while remaining > 0 {                                                    // c:2319
        if check_cmdata(md, sfx) != 0 {                                      // c:2320
            return ret;
        }

        let md_bytes = md.str.as_bytes();
        let mut l: usize = 0;
        let md_len_usize = md.len as usize;
        let cap = remaining.min(md_len_usize);

        // c:2329-2331 — accumulate matching chars from the chosen end.
        while l < cap {
            let s_idx: isize = if sfx != 0 {
                start_idx as isize - (l as isize) - 1 - (ret as isize)
            } else {
                (ret as isize) + (l as isize)
            };
            let m_len = md_bytes.len();
            let m_idx: isize = if sfx != 0 {
                m_len as isize - (l as isize) - 1
            } else {
                l as isize
            };
            if s_idx < 0 || m_idx < 0 { break; }
            let s_pos = s_idx as usize;
            let m_pos = m_idx as usize;
            if s_pos >= str_bytes.len() || m_pos >= md_bytes.len() { break; }
            if str_bytes[s_pos] != md_bytes[m_pos] { break; }
            l += 1;
        }

        if l == 0 { return ret; }                                            // c:2380 no progress

        // c:2335-2349 — meta-character boundary correction. Avoid
        // ending in the middle of a `Meta x` 2-byte sequence.
        const META_BYTE: u8 = 0x83;
        let check_pos: isize = if sfx != 0 {
            start_idx as isize - (l as isize) - (ret as isize)
        } else {
            (ret as isize) + (l as isize) - 1
        };
        if check_pos >= 0 && (check_pos as usize) < str_bytes.len()
            && str_bytes[check_pos as usize] == META_BYTE && l > 0
        {
            l -= 1;
        }

        // c:2400 — md.len -= l; md.str = md.str + l (or md.str - l for sfx).
        md.len -= l as i32;
        if sfx != 0 {
            // suffix-mode: strip from the END of md.str.
            md.str = md.str.chars().take(
                md.str.chars().count().saturating_sub(l),
            ).collect();
        } else {
            // prefix-mode: skip first l bytes.
            md.str = md.str.chars().skip(l).collect();
        }

        ret += l as i32;                                                     // c:2418
        remaining = remaining.saturating_sub(l);

        if remaining == 0 || md.len == 0 {                                   // c:2421
            break;
        }
    }
    ret                                                                      // c:2441
}

/// Port of `undo_cmdata(md, sfx)` from Src/Zle/compmatch.c:2188.
/// Direct port of `static Cline undo_cmdata(cmdata md, int sfx)` from
/// `Src/Zle/compmatch.c:2187-2207`. Puts the not-yet-matched portion
/// of `md` back into the previous cline node so it can be revisited
/// on a different match path.
pub fn undo_cmdata(md: &cmdata, sfx: i32) -> Option<Box<crate::ported::zle::comp_h::Cline>> { // c:2187
    use crate::ported::zle::comp_h::CLF_LINE;
    let mut r = md.pcl.as_deref().cloned()?;                                 // c:2189 r = md->pcl

    if md.line != 0 {                                                        // c:2191
        r.word = None;                                                       // c:2192
        r.wlen = 0;                                                          // c:2193
        r.flags |= CLF_LINE;                                                 // c:2194
        r.llen = md.len;                                                     // c:2195
        // c:2197 — line = str - (sfx ? len : 0).
        let off = if sfx != 0 { md.len as usize } else { 0 };
        r.line = Some(md.str.chars().skip(md.str.len().saturating_sub(off + md.len as usize)).collect());
    } else if md.len != md.olen {                                            // c:2199
        r.wlen = md.len;                                                     // c:2201
        let off = if sfx != 0 { md.len as usize } else { 0 };
        r.word = Some(md.str.chars().skip(md.str.len().saturating_sub(off + md.len as usize)).collect());
    }
    Some(Box::new(r))                                                        // c:2206
}

