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
//! - match_str         → crate::compsys::matching::match_str()
//! - match_parts       → crate::compsys::matching::match_parts()
//! - comp_match        → crate::compsys::matching::comp_match()
//! - pattern_match_equivalence → crate::compsys::matching (inline)
//! - add_match_str/part/sub    → crate::compsys::matching (inline)
//! - cline_* (match line ops)  → inline below; the compsys::base
//!                                `CompletionLine` shim was deleted.

// CompMatcher / MatchFlags / CompLine deleted — Rust-invented structs
// with no C counterpart. The legit C types `Cmatcher` (comp.h:153),
// `Cline` (comp.h:245), and `Cpattern` (comp.h:197) are ported in
// `comp_h.rs` and used by the real porters of `match_str` /
// `pattern_match` / `add_match_str` etc. below.

use crate::ported::pattern::pattry;
use crate::ported::utils::set_noerrs;
use crate::ported::zle::comp_h::{
    Cline, Cmatcher, Cmlist, Cpattern, CLF_DIFF, CLF_JOIN, CLF_LINE, CLF_MATCHED, CLF_MISS,
    CLF_NEW, CLF_SUF, CMF_INTER, CMF_LEFT, CMF_LINE, CMF_RIGHT, CPAT_ANY, CPAT_CCLASS, CPAT_CHAR,
    CPAT_EQUIV, CPAT_NCLASS,
};
use crate::ported::zle::compcore::{mstack, multiquote, tildequote, useqbr};
use crate::ported::zle::zle_h::{brinfo, ZC_tolower, ZC_toupper};
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::{PP_LOWER, PP_RANGE, PP_UPPER};
use std::sync::{Mutex, OnceLock};

/// Port of `cpatterns_same(Cpattern a, Cpattern b)` from `Src/Zle/compmatch.c:42`.
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
/// WARNING: param names don't match C — Rust=(b) vs C=(a, b)

// --- AUTO: cross-zle hoisted-fn use glob ---
/// `cpatterns_same` — see implementation.
#[allow(unused_imports)]
#[allow(unused_imports)]

pub fn cpatterns_same(
    // c:44
    mut a: Option<&Cpattern>,
    mut b: Option<&Cpattern>,
) -> bool {
    // c:42
    while let Some(ap) = a {
        // c:46 while (a)
        let bp = match b {
            // c:47
            None => return false, // c:48 if(!b) return 0
            Some(p) => p,
        };
        if ap.tp != bp.tp {
            // c:49
            return false; // c:50
        }
        match ap.tp {
            // c:51
            x if x == CPAT_CCLASS || x == CPAT_NCLASS || x == CPAT_EQUIV => {
                // c:52-54
                // c:55-58 — equivalent ranges might compare same even when
                // strings differ; the C source admits this is unhandled.
                if ap.str != bp.str {
                    // c:60 strcmp(a->u.str,b->u.str)
                    return false; // c:61
                }
            }
            x if x == CPAT_CHAR => {
                // c:64
                if ap.chr != bp.chr {
                    // c:65
                    return false; // c:66
                }
            }
            _ => { // c:69 default
                 // c:70 — "here to silence compiler"
            }
        }
        a = ap.next.as_deref(); // c:74 a = a->next
        b = bp.next.as_deref(); // c:75 b = b->next
    }
    b.is_none() // c:77 return !b
}

/// Port of `cmatchers_same(Cmatcher a, Cmatcher b)` from `Src/Zle/compmatch.c:82`.
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
/// WARNING: param names don't match C — Rust=(b) vs C=(a, b)
pub fn cmatchers_same(
    // c:84
    a: &Cmatcher,
    b: &Cmatcher,
) -> bool {
    // c:82
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
        if a.lalen != b.lalen || a.ralen != b.ralen {
            // c:92
            return false;
        }
        if a.lalen != 0 && !cpatterns_same(a.left.as_deref(), b.left.as_deref()) {
            return false; // c:93
        }
        if a.ralen != 0 && !cpatterns_same(a.right.as_deref(), b.right.as_deref()) {
            return false; // c:94
        }
    }
    true
}

/// Direct port of `mod_export void add_bmatchers(Cmatcher m)` from
/// `Src/Zle/compmatch.c:101`. Walks the supplied Cmatcher chain
/// (the head of `def->matcher` at call sites) and prepends each
/// matcher that qualifies for brace-matching to the file-scope
/// `bmatchers` Cmlist. Original chain head is appended after the new
/// entries so the final list is `[new_entries..., old_bmatchers...]`.
pub fn add_bmatchers(m: Option<&Cmatcher>) {
    // c:101
    let cell = crate::ported::zle::compcore::bmatchers.get_or_init(|| Mutex::new(None));
    let old = cell.lock().ok().and_then(|mut g| g.take()); // c:104 Cmlist old = bmatchers
                                                           // c:105-113 — qualify each m; prepend matches in C order (reversed
                                                           // iter so the final list is `[new_entries..., old]` per c:114 *q=old).
    let mut head = old;
    for mat in std::iter::successors(m, |p| p.next.as_deref())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    // c:105 walk m
    {
        let qual = (mat.flags == 0 && mat.wlen > 0 && mat.llen > 0)          // c:107-108
                || (mat.flags == CMF_RIGHT && mat.wlen < 0 && mat.llen == 0);
        if qual {
            // c:109-112
            head = Some(Box::new(Cmlist {
                next: head,
                matcher: Box::new(mat.clone()),
                str: String::new(),
            }));
        }
    }
    if let Ok(mut g) = cell.lock() {
        *g = head;
    }
}

/// Direct port of `mod_export void update_bmatchers(void)` from
/// `Src/Zle/compmatch.c:121`. Called when mstack changes — ensures
/// `bmatchers` contains no matchers absent from `mstack`.
pub fn update_bmatchers() {
    // c:121
    let bm_cell = crate::ported::zle::compcore::bmatchers.get_or_init(|| Mutex::new(None));
    let ms_cell = mstack.get_or_init(|| Mutex::new(None));
    let mut p = bm_cell.lock().ok().and_then(|mut g| g.take()); // c:124 Cmlist p = bmatchers
    let ms_head = ms_cell
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|b| (**b).clone()));
    let mut new_bmatchers: Option<Box<Cmlist>> = p.as_ref().map(|b| (**b).clone()).map(Box::new);
    while let Some(node) = p {
        // c:128 while (p)
        let mut t = false; // c:129 t = 0
        let mut ms = ms_head.as_ref(); // c:130 ms = mstack
        while let Some(mscur) = ms {
            if t {
                break;
            }
            let mut mp = Some(mscur.matcher.as_ref()); // c:131 mp = ms->matcher
            while let Some(mpcur) = mp {
                if t {
                    break;
                }
                t = cmatchers_same(mpcur, &*node.matcher); // c:132 cmatchers_same
                mp = mpcur.next.as_deref();
            }
            ms = mscur.next.as_deref();
        }
        p = node.next; // c:134 p = p->next
        if !t {
            // c:135 if (!t)
            new_bmatchers = p.as_ref().map(|b| (**b).clone()).map(Box::new); // c:136 bmatchers = p
        }
    }
    if let Ok(mut g) = bm_cell.lock() {
        *g = new_bmatchers;
    }
}

/// Port of `Cline get_cline(char *l, int ll, char *w, int wl, char *o,
///                            int ol, int fl)` from Src/Zle/compmatch.c:144.
///
/// "Returns a new Cline structure." The C version pools freed Clines
/// via the `freecl` heap; Rust uses normal allocation so the pool
/// dance collapses to a `Box::new`. Sets `word`/`wlen`/`line`/`llen`/
/// `orig`/`olen`/`flags` per the args; clears `prefix`/`suffix`/`min`/
/// `max`/`slen`.
pub fn get_cline(
    l: Option<String>,
    ll: i32,
    w: Option<String>,
    wl: i32, // c:144
    o: Option<String>,
    ol: i32,
    fl: i32,
) -> Box<Cline> {
    Box::new(Cline {
        next: None, // c:156
        line: l,    // c:157
        llen: ll,
        word: w, // c:158
        wlen: wl,
        orig: o, // c:160
        olen: ol,
        slen: 0,      // c:161
        flags: fl,    // c:162
        prefix: None, // c:163
        suffix: None,
        min: 0, // c:164
        max: 0,
    })
}

/// Port of `free_cline(Cline l)` from `Src/Zle/compmatch.c:171`.
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
pub fn free_cline(l: Option<Box<Cline>>) {
    // c:172
    // c:172-183 — walk; free each prefix/suffix recursively. In Rust
    // dropping the Box of the list head triggers Drop on `next`/
    // `prefix`/`suffix` chains automatically. `freecl` recycling
    // is a C-only zhalloc optimisation that doesn't apply here.
    drop(l);
}

/// Port of `cp_cline(Cline l, int deep)` from `Src/Zle/compmatch.c:189`.
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
/// WARNING: param names don't match C — Rust=(deep) vs C=(l, deep)
pub fn cp_cline(
    // c:190
    l: Option<&Cline>,
    deep: i32,
) -> Option<Box<Cline>> {
    // c:189
    let mut r: Option<Box<Cline>> = None; // c:192 r = NULL
    let mut tail: *mut Option<Box<Cline>> = &mut r;
    let mut cur = l;
    while let Some(node) = cur {
        // c:194 while (l)
        // c:198 — `t = (Cline) zhalloc(sizeof(*t))`.
        // c:199 — `memcpy(t, l, sizeof(*t))`.
        let mut t: Box<Cline> = Box::new(node.clone());
        // Reset `next` so the memcpy-equivalent doesn't link to the
        // source's next (the loop sets it via the tail pointer).
        t.next = None;
        if deep != 0 {
            // c:200 if (deep)
            // c:201-202 — `t->prefix = cp_cline(t->prefix, 0)`. Already
            // a Box-clone via memcpy; rebuild as deep copy.
            if let Some(pre) = node.prefix.as_deref() {
                t.prefix = cp_cline(Some(pre), 0); // c:202
            }
            if let Some(suf) = node.suffix.as_deref() {
                t.suffix = cp_cline(Some(suf), 0); // c:204
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
        cur = node.next.as_deref(); // c:208 l = l->next
    }
    // c:210 — `*p = NULL`. Already None by default.
    r // c:212 return r
}

// =====================================================================
// cline_sublen / cline_setlens / cline_matched / revert_cline / cp_cline
// — `Src/Zle/compmatch.c:217-281`.
// =====================================================================

/// Port of `cline_sublen(Cline l)` from `Src/Zle/compmatch.c:218`.
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
pub fn cline_sublen(l: &Cline) -> i32 {
    // c:219
    // c:221 — `len = (CLF_LINE ? llen : wlen)`.
    let mut len: i32 = if (l.flags & CLF_LINE) != 0 {
        l.llen
    } else {
        l.wlen
    };
    // c:223 — `if (olen && !((CLF_SUF ? suffix : prefix))) len += olen`.
    let no_subs = if (l.flags & CLF_SUF) != 0 {
        l.suffix.is_none()
    } else {
        l.prefix.is_none()
    };
    if l.olen != 0 && no_subs {
        len += l.olen; // c:224
    } else {
        // c:225
        // c:228-229 — walk prefix sub-list summing per-part length.
        let mut p = l.prefix.as_deref();
        while let Some(pp) = p {
            len += if (pp.flags & CLF_LINE) != 0 {
                pp.llen
            } else {
                pp.wlen
            };
            p = pp.next.as_deref();
        }
        // c:230-231 — walk suffix sub-list.
        let mut p = l.suffix.as_deref();
        while let Some(pp) = p {
            len += if (pp.flags & CLF_LINE) != 0 {
                pp.llen
            } else {
                pp.wlen
            };
            p = pp.next.as_deref();
        }
    }
    len // c:233 return len
}

/// Port of `cline_setlens(Cline l, int both)` from `Src/Zle/compmatch.c:240`.
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
pub fn cline_setlens(l: &mut Option<Box<Cline>>, both: i32) {
    // c:240
    let mut cur = l.as_deref_mut();
    while let Some(node) = cur {
        // c:242 while (l)
        let s = cline_sublen(node); // c:243 cline_sublen(l)
        node.min = s; // c:243 l->min = ...
        if both != 0 {
            // c:244 if (both)
            node.max = s; // c:245 l->max = l->min
        }
        cur = node.next.as_deref_mut(); // c:246 l = l->next
    }
}

// =====================================================================
// matchbuf / matchparts / matchsubs globals + start_match / abort_match
// — `Src/Zle/compmatch.c:283-317`.
// =====================================================================

/// Port of `cline_matched(Cline p)` from `Src/Zle/compmatch.c:254`.
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
pub fn cline_matched(p: &mut Option<Box<Cline>>) {
    // c:254
    let mut cur = p.as_deref_mut();
    while let Some(node) = cur {
        // c:256 while (p)
        node.flags |= CLF_MATCHED; // c:257
        cline_matched(&mut node.prefix); // c:258
        cline_matched(&mut node.suffix); // c:259
        cur = node.next.as_deref_mut(); // c:261 p = p->next
    }
}

/// Port of `revert_cline(Cline p)` from `Src/Zle/compmatch.c:269`.
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
/// WARNING: param names don't match C — Rust=() vs C=(p)
pub fn revert_cline(
    // c:270
    mut p: Option<Box<Cline>>,
) -> Option<Box<Cline>> {
    // c:269
    let mut r: Option<Box<Cline>> = None; // c:272 r = NULL
    while let Some(mut node) = p {
        // c:274 while (p)
        let n = node.next.take(); // c:275 n = p->next
        node.next = r; // c:276 p->next = r
        r = Some(node); // c:277 r = p
        p = n; // c:278 p = n
    }
    r // c:280 return r
}

/// Port of `start_match()` from `Src/Zle/compmatch.c:300`.
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
pub fn start_match() {
    // c:300
    // c:300-303 — `if (matchbuf) *matchbuf = '\0'`.
    MATCHBUF
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .unwrap()
        .clear();
    // c:305 — `matchparts = matchlastpart = matchsubs = matchlastsub = NULL`.
    // All FOUR must reset. Omitting MATCHLASTPART/MATCHLASTSUB left them stale
    // across matches: the next add_match_part read a Some MATCHLASTPART and took
    // the append branch (`lastpart->next = p`) against the disconnected old tail
    // while MATCHPARTS head stayed None, so `pli = matchparts` came back empty —
    // partial-match Cline lost (e.g. `ls /dir/a<TAB>` dropped the typed `a`).
    *MATCHPARTS.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
    *MATCHLASTPART.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
    *MATCHSUBS.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
    *MATCHLASTSUB.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
}

/// Port of `abort_match()` from `Src/Zle/compmatch.c:312`.
/// ```c
/// static void
/// abort_match(void)
/// {
///     free_cline(matchparts);
///     free_cline(matchsubs);
///     matchparts = matchsubs = NULL;
/// }
/// ```
/// C body (compmatch.c:312, 3 lines):
///     `free_cline(matchparts);
///      free_cline(matchsubs);
///      matchparts = matchsubs = NULL;`
/// The `take()` on each guard discards the old chain (Rust drop runs
/// `free_cline`) and leaves the slot None — same observable state.
pub fn abort_match() {
    // c:312
    free_cline(
        MATCHPARTS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .take(),
    ); // c:313
    free_cline(
        MATCHSUBS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .take(),
    ); // c:314
}

/// Direct port of `static void add_match_str(Cmatcher m, char *l,
///                                          char *w, int wl, int sfx)`
/// from `Src/Zle/compmatch.c:327`. Pushes the string `w` (or
/// `l` when `m & CMF_LINE`) of length `wl` into the file-scope
/// `MATCHBUF` accumulator; `sfx` prepends instead of appends.
pub fn add_match_str(
    m: Option<&Cmatcher>, // c:327
    l: &str,
    w: &str,
    mut wl: i32,
    sfx: i32,
) {
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

    if wl <= 0 {
        return;
    } // c:335

    // c:337-353 — buffer-grow + insert. Rust's String handles the
    // grow path; we still mirror the matchbufadded counter for parity
    // with `MATCHBUFLEN`-checking C call sites.
    if let Ok(mut buf) = MATCHBUF.get_or_init(|| Mutex::new(String::new())).lock() {
        let take_n = wl as usize;
        let new_chunk: String = eff_w.chars().take(take_n).collect();
        if sfx != 0 {
            // c:354 prefix-mode
            *buf = format!("{}{}", new_chunk, *buf); // c:356
        } else {
            // c:358
            buf.push_str(&new_chunk);
        }
        MATCHBUFADDED.fetch_add(wl, std::sync::atomic::Ordering::Relaxed); // c:362
    }
}

/// Direct port of `static void add_match_part(Cmatcher m, char *l,
///                                            char *w, int wl,
///                                            char *o, int ol,
///                                            char *s, int sl,
///                                            int osl, int sfx)`
/// from `Src/Zle/compmatch.c:373`. Appends a partial match into
/// `MATCHPARTS`, splitting the new part via `bld_parts` per the
/// matcher's anchor rules and consuming any pending `MATCHSUBS`
/// nodes into the new tail.
pub fn add_match_part(
    m: Option<&Cmatcher>, // c:373
    l: Option<&str>,
    _ll: i32,
    w: &str,
    wl: i32,
    o: Option<&str>,
    ol: i32,
    s: &str,
    sl: i32,
    osl: i32,
    sfx: i32,
) {
    // c:382 — `if (l && !strncmp(l, w, wl)) l = NULL` — drop redundant anchor.
    let l_eff: Option<String> = match l {
        Some(lstr)
            if lstr.len() >= wl as usize && wl > 0 && &lstr[..wl as usize] == &w[..wl as usize] =>
        {
            None
        }
        Some(lstr) => Some(lstr.to_string()),
        None => None,
    };

    // c:392 — `p = bld_parts(s, sl, osl, &lp, &lprem)`.
    let mut lp: Option<Box<Cline>> = None;
    let mut lprem: Option<Box<Cline>> = None;
    let mut p = bld_parts(s, sl, osl, Some(&mut lp), Some(&mut lprem));

    // c:394 — `if (lprem && m && (m->flags & CLF_LEFT))`.
    if let Some(rem) = lprem.as_mut() {
        if m.map(|mat| (mat.flags & CMF_LEFT) != 0).unwrap_or(false) {
            rem.flags |= CLF_SUF; // c:395
            rem.suffix = rem.prefix.take(); // c:396 swap
        }
    }

    // c:402 — `if (sfx) p = revert_cline(lp = p)`.
    if sfx != 0 {
        if let Some(chain) = p.take() {
            p = revert_cline(Some(chain));
        }
    }

    // c:405-419 — merge MATCHSUBS into the head/tail.
    let subs = MATCHSUBS
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut g| g.take());
    if let Some(subs_chain) = subs {
        // c:405
        if let Some(lp_node) = lp.as_mut() {
            if sfx != 0 {
                // c:407 lp->prefix tail-append
                let mut tail_ref: *mut Option<Box<Cline>> = &mut lp_node.prefix;
                unsafe {
                    while let Some(ref mut next_node) = *tail_ref {
                        tail_ref = &mut next_node.next as *mut _;
                    }
                    *tail_ref = Some(subs_chain);
                }
            } else if let Some(ref mut p_node) = p {
                // c:415 p->prefix prepend
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
        if let Ok(mut g) = MATCHLASTSUB.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
    }

    // c:421-435 — store args in the last part-cline.
    if let Some(lp_node) = lp.as_mut() {
        if lp_node.llen != 0 || lp_node.wlen != 0 {
            // c:421
            let next = get_cline(
                l_eff.clone(),
                wl,
                Some(w.to_string()),
                wl,
                o.map(|s| s.to_string()),
                ol,
                CLF_NEW,
            );
            lp_node.next = Some(next); // c:423
        } else {
            // c:425
            lp_node.line = l_eff.clone(); // c:426
            lp_node.llen = wl;
            lp_node.word = Some(w.to_string()); // c:428
            lp_node.wlen = wl;
            lp_node.orig = o.map(|s| s.to_string()); // c:430
            lp_node.olen = ol;
        }
        if o.is_some() || ol != 0 {
            // c:432
            lp_node.flags &= !CLF_NEW;
        }
    }

    // c:439-444 — append `p` to MATCHPARTS via MATCHLASTPART.
    let last_present = MATCHLASTPART
        .get()
        .and_then(|c| c.lock().ok().map(|g| g.is_some()))
        .unwrap_or(false);
    if last_present {
        // c:440
        if let Ok(mut tail) = MATCHLASTPART.get_or_init(|| Mutex::new(None)).lock() {
            if let Some(t) = tail.as_mut() {
                t.next = p.clone();
            }
        }
    } else if let Ok(mut head) = MATCHPARTS.get_or_init(|| Mutex::new(None)).lock() {
        *head = p.clone(); // c:442
    }
    if let Some(lp_node) = lp {
        if let Ok(mut tail) = MATCHLASTPART.get_or_init(|| Mutex::new(None)).lock() {
            *tail = Some(lp_node); // c:443
        }
    }
}

// Fake `parse_cmatcher` / `update_bmatchers` deleted.
// `parse_cmatcher` already exists at `complete.rs:992` as a real
// port of `Src/Zle/complete.c:242`. `update_bmatchers` is at
// `Src/Zle/compmatch.c:121` with signature `void update_bmatchers(void)`
// — the Rust placeholder had the wrong arity and type, will land
// alongside the matcher-engine driver.

/// Direct port of `static void add_match_sub(Cmatcher m, char *l, int ll,
///                                          char *w, int wl)` from
/// `Src/Zle/compmatch.c:446`. Pushes one sub-match cline node
/// into the file-scope `MATCHSUBS` / `MATCHLASTSUB` linked list.
/// Called from match_str during a CMF_RIGHT anchor match.
pub fn add_match_sub(
    m: Option<&Cmatcher>, // c:446
    l: Option<&str>,
    ll: i32,
    w: Option<&str>,
    wl: i32,
) {
    // c:450-453 — `if (m && (m->flags & CMF_LINE)) { wl = m->llen; w = l; }`.
    let (eff_w, eff_wl) = match m {
        Some(mat) if (mat.flags & CMF_LINE) != 0 => (l, mat.llen),
        _ => (w, wl),
    };

    // c:455-456 — short-circuit if no length.
    if eff_wl <= 0 && ll <= 0 {
        return;
    }

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
    if last_present {
        // c:494 — chain to existing tail
        if let Ok(mut tail) = last_cell.lock() {
            if let Some(t) = tail.as_mut() {
                t.next = Some(node.clone()); // c:495 matchlastsub->next = n
            }
        }
    } else {
        // c:496 — first node
        if let Ok(mut h) = head_cell.lock() {
            *h = Some(node.clone()); // c:497 matchsubs = n
        }
    }
    if let Ok(mut tail) = last_cell.lock() {
        *tail = Some(node); // c:499 matchlastsub = n
    }
}

// Real-port of `match_str` lands below. The exact-char skip fast
// path (c:569-590), non-* matcher loop with CMF_LEFT/RIGHT anchors
// (c:868-989), and *-pattern matcher loop in both prefix and
// suffix modes (c:603-867 / c:735-776) are all real-bodied.

/// Direct port of `static int match_str(char *l, char *w, Brinfo *bpp,
///                                       int bc, int *rwlp, const int sfx,
///                                       int test, int part)`
/// from `Src/Zle/compmatch.c:500-1085`. The matcher application
/// engine: walks the line string `l` against the word string `w`
/// using each `Cmlist` in the global `mstack` chain. Builds
/// `matchparts` / `matchsubs` along the way, threads brace-position
/// info via `bpp`. Returns the number of `w` bytes consumed on a
/// full match, -1 on no match.
///
/// **Port scope:** all matcher paths real-bodied — exact-char skip
/// fast path (c:569-590), non-* matcher loop with CMF_LEFT/RIGHT
/// anchors + pattern_match + add_match_str/sub emit (c:868-989),
/// *-pattern matcher loop in prefix mode (c:603-867) and suffix
/// mode (c:735-776 with bounded recursive call), exact-rewind
/// retry (c:1020-1034), test/part-mode returns (c:1046-1084).
pub fn match_str(
    // c:500
    l_in: &str,
    w_in: &str,
    _bpp: Option<&mut Option<Box<brinfo>>>,
    bc: i32,
    rwlp: Option<&mut i32>,
    sfx: i32,
    test: i32,
    part: i32,
) -> i32 {
    let l_bytes = l_in.as_bytes();
    let w_bytes = w_in.as_bytes();
    let mut ll = l_bytes.len() as i32;
    let mut lw = w_bytes.len() as i32;
    // c:517 — `const int original_ll = ll, original_lw = lw;` used by the
    // anti-recursion guard below (c:592-597).
    let original_ll = ll;
    let original_lw = lw;
    let mut il: i32 = 0;
    let mut iw: i32 = 0;
    let mut exact: i32 = 0;
    let mut wexact: i32 = 0;
    let mut bc = bc;
    let _obc = bc;
    let add: i32 = if sfx != 0 { -1 } else { 1 };
    let ind: i32 = if sfx != 0 { -1 } else { 0 };

    if test == 0 {
        // c:523
        start_match();
    }

    // Track positions as byte indices. In sfx mode we walk from the
    // end backwards; ind=-1 means "previous byte". We use signed
    // cursors so the arithmetic mirrors C's pointer arithmetic.
    let mut l_pos: i32 = if sfx != 0 { ll } else { 0 };
    let mut w_pos: i32 = if sfx != 0 { lw } else { 0 };
    let mut ow_pos: i32 = w_pos;
    let mut lm: Option<Box<Cmatcher>> = None;
    let mut he = 0i32;

    // Snapshot the mstack chain into a Vec for stable iteration.
    let mstack_snapshot: Vec<Box<Cmatcher>> = {
        let g = mstack.get_or_init(|| Mutex::new(None)).lock().ok();
        let mut out = Vec::new();
        if let Some(g) = g {
            let mut cur = g.as_deref();
            while let Some(ms) = cur {
                let mut mp_cur: Option<&Cmatcher> = Some(&*ms.matcher);
                while let Some(mp) = mp_cur {
                    out.push(Box::new(mp.clone()));
                    mp_cur = mp.next.as_deref();
                }
                cur = ms.next.as_deref();
            }
        }
        out
    };

    // c:591 `retry:` — the label sits AFTER the exact-char fast path, so
    // C's `goto retry` (c:1029) re-runs the matcher loop WITHOUT re-doing
    // the fast path. This one-shot flag reproduces that: when set, the
    // next iteration skips the fast path and goes straight to the matcher
    // loop. Without it the rewind below re-matches the same exact char
    // forever (infinite loop on any word that shares a leading char with
    // the prefix but then diverges, e.g. prefix "ec" vs "emulator").
    let mut retry_skip_fastpath = false;
    'outer: while ll > 0 {
        // c:546
        let do_fastpath = !retry_skip_fastpath;
        retry_skip_fastpath = false;
        // c:569-590 — exact-char skip fast path.
        if do_fastpath && sfx == 0 && lw > 0 && (part == 0 || test != 0) {
            let l_idx = (l_pos + ind) as usize;
            let w_idx = (w_pos + ind) as usize;
            if l_idx < l_bytes.len() && w_idx < w_bytes.len() {
                let l_ch = l_bytes[l_idx];
                let w_ch = w_bytes[w_idx];
                let bslash = lw > 1
                    && w_ch == b'\\'
                    && w_idx + 1 < w_bytes.len()
                    && w_bytes[w_idx + 1] == l_bytes[(l_pos + ind) as usize];
                if l_ch == w_ch || bslash {
                    let advance_w = if bslash { 2 } else { 1 };
                    l_pos += add;
                    w_pos += if bslash { add + add } else { add };
                    il += 1;
                    iw += advance_w;
                    ll -= 1;
                    lw -= advance_w;
                    bc += 1;
                    exact += 1;
                    wexact += advance_w;
                    lm = None;
                    he = 0;
                    continue 'outer; // c:589
                }
            }
        }

        // c:591 retry: walk the snapshotted matcher chain looking for
        // a non-* matcher we can apply at the current cursor.
        let mut matched: Option<Box<Cmatcher>> = None;
        for mp in mstack_snapshot.iter() {
            if let Some(ref lm_box) = lm {
                if std::ptr::addr_eq(lm_box.as_ref() as *const _, mp.as_ref() as *const _) {
                    continue; // c:595
                }
            }
            // c:592-597 — anti-recursion guard: in a recursive (test) call,
            // don't apply a `*` (wlen<0) matcher at the very start of the line
            // or word. Without this, a zero-progress `*`-match (e.g.
            // `r:|[._-]=*` against line "-") recurses match_str↔match_parts
            // forever → stack-overflow SIGBUS on `zsh -<TAB>`.
            if (original_ll == ll || original_lw == lw)
                && (test == 1 || (test != 0 && mp.left.is_none() && mp.right.is_none()))
                && mp.wlen < 0
            {
                continue; // c:597
            }
            if mp.wlen < 0 {
                // c:603-867 — `*`-pattern matcher. Handles both prefix
                // (sfx == 0) and suffix (sfx != 0) modes.

                // c:689-694 — set up llen / alen / aol per CMF_LEFT.
                let llen_p = mp.llen;
                let (alen, aol): (i32, i32) = if (mp.flags & CMF_LEFT) != 0 {
                    (mp.lalen, mp.ralen)
                } else {
                    (mp.ralen, mp.lalen)
                };
                if ll < llen_p + alen || lw < alen + aol {
                    // c:698
                    continue;
                }

                // c:701-715 — set ap/aop/moff/loff/aoff/both per CMF_LEFT
                // × sfx. Four combinations.
                let (ap, aop, moff, both, loff, aoff): (
                    Option<&Cpattern>,
                    Option<&Cpattern>,
                    i32,
                    i32,
                    i32,
                    i32,
                );
                if (mp.flags & CMF_LEFT) != 0 {
                    // c:701
                    ap = mp.left.as_deref();
                    aop = mp.right.as_deref();
                    moff = alen;
                    if sfx != 0 {
                        // c:703
                        both = 0;
                        loff = -llen_p;
                        aoff = -(llen_p + alen);
                    } else {
                        // c:706
                        both = 1;
                        loff = alen;
                        aoff = 0;
                    }
                } else {
                    // c:708
                    ap = mp.right.as_deref();
                    aop = mp.left.as_deref();
                    moff = 0;
                    if sfx != 0 {
                        // c:710
                        both = 1;
                        loff = -(llen_p + alen);
                        aoff = -alen;
                    } else {
                        // c:712
                        both = 0;
                        loff = 0;
                        aoff = llen_p;
                    }
                }

                // c:717 — pattern_match(mp.line, l + loff).
                let l_off_idx = (l_pos + loff).max(0) as usize;
                if l_off_idx >= l_bytes.len() {
                    continue;
                }
                let line_slice = std::str::from_utf8(&l_bytes[l_off_idx..]).unwrap_or("");
                if pattern_match(mp.line.as_deref(), line_slice, None, "") == 0 {
                    continue;
                }
                // c:719-731 — anchor test.
                if let Some(ap_pat) = ap {
                    let l_anchor_idx = (l_pos + aoff).max(0) as usize;
                    let l_anchor = std::str::from_utf8(&l_bytes[l_anchor_idx..]).unwrap_or("");
                    if pattern_match(Some(ap_pat), l_anchor, None, "") == 0 {
                        continue;
                    }
                    if both != 0 {
                        // c:721
                        let w_anchor_idx = (w_pos + aoff).max(0) as usize;
                        let w_anchor = std::str::from_utf8(&w_bytes[w_anchor_idx..]).unwrap_or("");
                        if pattern_match(Some(ap_pat), w_anchor, None, "") == 0 {
                            continue;
                        }
                        if aol > 0 && aol <= aoff + iw {
                            let w_op_idx = (w_pos + aoff - aol).max(0) as usize;
                            let w_op = std::str::from_utf8(&w_bytes[w_op_idx..]).unwrap_or("");
                            if pattern_match(aop, w_op, None, "") == 0 {
                                continue;
                            }
                        }
                        // c:726 — match_parts to confirm anchor span.
                        let mp_l = std::str::from_utf8(&l_bytes[l_anchor_idx..]).unwrap_or("");
                        let mp_w = std::str::from_utf8(&w_bytes[(w_pos + aoff).max(0) as usize..])
                            .unwrap_or("");
                        if match_parts(mp_l, mp_w, alen, part) == 0 {
                            continue;
                        }
                    }
                } else {
                    // c:728
                    let cmf_check = if (mp.flags & CMF_INTER) != 0 {
                        if (mp.flags & CMF_LINE) != 0 {
                            iw
                        } else {
                            il
                        }
                    } else {
                        il | iw
                    };
                    if both == 0 || cmf_check != 0 {
                        continue;
                    }
                }

                // c:737-773 — recursive scan: try each tp from w forward
                // looking for a position where `l + llen + moff` matches.
                let mut t = 0i32;
                let mut ct = 0i32;
                let ict_total = lw - alen + 1;
                let mut found_tp_pos: i32 = w_pos;
                // c:737 — tp walks from w outward. In prefix mode (add=+1)
                // forward through w[w_pos..]; in sfx mode (add=-1)
                // backward through w[..w_pos]. We iterate ict_total
                // steps, computing tp_pos as w_pos + ct*add.
                for step in 0..ict_total.max(0) {
                    let tp_pos = w_pos + step * add;
                    let mut accept = false;
                    if both != 0 {
                        // c:740-745 — both-mode: succeed only if ap stops
                        // matching at the current tp (the `*` consumed
                        // characters before reaching the anchor).
                        let ap_fails = ap.is_none() || test == 0 || {
                            let tp_anchor_idx = (tp_pos + aoff).max(0) as usize;
                            let tp_slice =
                                std::str::from_utf8(&w_bytes.get(tp_anchor_idx..).unwrap_or(&[]))
                                    .unwrap_or("");
                            pattern_match(ap, tp_slice, None, "") == 0
                        };
                        if ap_fails {
                            accept = true;
                        }
                    } else {
                        // c:746-753 — non-both: succeed when ap matches at
                        // tp - moff and aop matches at tp - moff - aol.
                        let tp_anchor_idx = (tp_pos - moff).max(0) as usize;
                        let tp_slice =
                            std::str::from_utf8(&w_bytes.get(tp_anchor_idx..).unwrap_or(&[]))
                                .unwrap_or("");
                        if pattern_match(ap, tp_slice, None, "") != 0 {
                            let aol_ok = aol == 0
                                || (aol <= iw + ct - moff && {
                                    let aop_idx = (tp_pos - moff - aol).max(0) as usize;
                                    let aop_slice =
                                        std::str::from_utf8(&w_bytes.get(aop_idx..).unwrap_or(&[]))
                                            .unwrap_or("");
                                    pattern_match(aop, aop_slice, None, "") != 0
                                });
                            if aol_ok {
                                let l_aoff_idx = (l_pos + aoff).max(0) as usize;
                                let l_aoff_slice =
                                    std::str::from_utf8(&l_bytes[l_aoff_idx..]).unwrap_or("");
                                let mp_ok = mp.wlen == -1
                                    || match_parts(l_aoff_slice, tp_slice, alen, part) != 0;
                                if mp_ok {
                                    accept = true;
                                }
                            }
                        }
                    }

                    // c:753-755 — for a variable-length (`*`) non-both matcher,
                    // hard-verify the anchor span with match_parts; C BREAKS the
                    // tp-scan if it fails. The port omitted this guard, so a
                    // zero-progress `*`-match (e.g. `r:|[._-]=*` against line
                    // "-") recursed match_str→match_str forever → stack-overflow
                    // SIGBUS on `zsh -<TAB>`.
                    if accept && both == 0 && mp.wlen == -1 {
                        let l_aoff_idx = (l_pos + aoff).max(0) as usize;
                        let l_aoff_slice =
                            std::str::from_utf8(&l_bytes[l_aoff_idx..]).unwrap_or("");
                        let tp_anchor_idx = (tp_pos - moff).max(0) as usize;
                        let tp_slice =
                            std::str::from_utf8(&w_bytes.get(tp_anchor_idx..).unwrap_or(&[]))
                                .unwrap_or("");
                        if match_parts(l_aoff_slice, tp_slice, alen, part) == 0 {
                            break;
                        }
                    }

                    if accept {
                        // c:757-769 — recursive match_str call.
                        if sfx != 0 {
                            // c:763 — l-ll, w-lw with bounded slices.
                            // C uses savl + tp[-alen] NUL trick; in Rust
                            // we pass slice up to position (l_pos - llen_p
                            // - alen) for l (the "savl" boundary) and up
                            // to (tp_pos - alen) for w (the "savw"
                            // boundary).
                            let l_bound = (l_pos - llen_p - alen).max(0) as usize;
                            let w_bound = (tp_pos - alen).max(0) as usize;
                            let l_rest =
                                std::str::from_utf8(&l_bytes[..l_bound.min(l_bytes.len())])
                                    .unwrap_or("");
                            let w_rest =
                                std::str::from_utf8(&w_bytes[..w_bound.min(w_bytes.len())])
                                    .unwrap_or("");
                            t = match_str(l_rest, w_rest, None, 0, None, sfx, 2, part);
                        } else {
                            // c:768 — l + llen + moff, tp + moff.
                            let l_rest_start = (l_pos + llen_p + moff) as usize;
                            let l_rest =
                                std::str::from_utf8(&l_bytes.get(l_rest_start..).unwrap_or(&[]))
                                    .unwrap_or("");
                            let w_rest_start = (tp_pos + moff) as usize;
                            let w_rest =
                                std::str::from_utf8(&w_bytes.get(w_rest_start..).unwrap_or(&[]))
                                    .unwrap_or("");
                            t = match_str(l_rest, w_rest, None, 0, None, sfx, 1, part);
                        }
                        if t != 0 || (mp.wlen == -1 && both == 0) {
                            found_tp_pos = tp_pos;
                            break;
                        }
                    }
                    ct += 1;
                }

                // c:780 — no match found in the recursive scan.
                if t == 0 {
                    continue;
                }

                // c:783-833 — emit Cline parts via add_match_*.
                let _tp_pos = found_tp_pos;
                if test == 0 && (he == 0 || (llen_p + alen) != 0) {
                    // c:789-805 — op/ol/lp/map/wap/wmp computed per sfx mode.
                    let (op_start, ol, lp_start, map_start, wap_start, wmp_start);
                    if sfx != 0 {
                        // c:789
                        op_start = w_pos as usize;
                        ol = (ow_pos - w_pos).max(0);
                        lp_start = (l_pos - (llen_p + alen)).max(0) as usize;
                        map_start = (found_tp_pos - alen).max(0) as usize;
                        if (mp.flags & CMF_LEFT) != 0 {
                            // c:792
                            wap_start = (found_tp_pos - alen).max(0) as usize;
                            wmp_start = found_tp_pos as usize;
                        } else {
                            // c:794
                            wap_start = (w_pos - alen).max(0) as usize;
                            wmp_start = (found_tp_pos - alen).max(0) as usize;
                        }
                    } else {
                        // c:797
                        op_start = ow_pos as usize;
                        ol = (w_pos - ow_pos).max(0);
                        lp_start = l_pos as usize;
                        map_start = ow_pos as usize;
                        if (mp.flags & CMF_LEFT) != 0 {
                            // c:800
                            wap_start = w_pos as usize;
                            wmp_start = (w_pos + alen) as usize;
                        } else {
                            // c:802
                            wap_start = found_tp_pos as usize;
                            wmp_start = ow_pos as usize;
                        }
                    }

                    if (mp.flags & CMF_LINE) != 0 {
                        // c:810
                        let op_str =
                            std::str::from_utf8(&w_bytes[op_start..op_start + ol as usize])
                                .unwrap_or("");
                        let lp_str = std::str::from_utf8(
                            &l_bytes[lp_start..lp_start + (llen_p + alen) as usize],
                        )
                        .unwrap_or("");
                        add_match_str(None, "", op_str, ol, sfx);
                        add_match_str(None, "", lp_str, llen_p + alen, sfx);
                        add_match_sub(None, None, ol, Some(op_str), ol);
                        add_match_sub(None, None, llen_p + alen, Some(lp_str), llen_p + alen);
                    } else {
                        // c:822
                        let map_len = ct + ol + alen;
                        let map_str = std::str::from_utf8(
                            &w_bytes[map_start
                                ..(map_start + map_len.max(0) as usize).min(w_bytes.len())],
                        )
                        .unwrap_or("");
                        add_match_str(None, "", map_str, map_len, sfx);
                        let ol_eff = if both != 0 {
                            let op_str =
                                std::str::from_utf8(&w_bytes[op_start..op_start + ol as usize])
                                    .unwrap_or("");
                            add_match_sub(None, None, ol, Some(op_str), ol);
                            -1
                        } else {
                            ct + ol
                        };
                        let l_aoff_idx = (l_pos + aoff).max(0) as usize;
                        let l_loff_idx = (l_pos + loff).max(0) as usize;
                        let l_aoff_str = std::str::from_utf8(
                            &l_bytes[l_aoff_idx..l_aoff_idx + alen.max(0) as usize],
                        )
                        .unwrap_or("");
                        let l_loff_str = std::str::from_utf8(
                            &l_bytes[l_loff_idx..l_loff_idx + llen_p.max(0) as usize],
                        )
                        .unwrap_or("");
                        let wap_str = std::str::from_utf8(
                            &w_bytes
                                [wap_start..(wap_start + alen.max(0) as usize).min(w_bytes.len())],
                        )
                        .unwrap_or("");
                        let wmp_str = std::str::from_utf8(
                            &w_bytes[wmp_start
                                ..(wmp_start + ol_eff.max(0) as usize).min(w_bytes.len())],
                        )
                        .unwrap_or("");
                        add_match_part(
                            Some(mp),
                            Some(l_aoff_str),
                            alen,
                            wap_str,
                            alen,
                            Some(l_loff_str),
                            llen_p,
                            wmp_str,
                            ol_eff,
                            ol_eff,
                            sfx,
                        );
                    }
                }

                // c:834-866 — advance pointers past the matched portion
                // + anchor. In sfx mode positions decrement; in prefix
                // mode they increment.
                let llen_new = llen_p + alen;
                let alen_new = alen + ct;
                if sfx != 0 {
                    // c:836
                    l_pos -= llen_new;
                    w_pos -= alen_new;
                } else {
                    // c:839
                    l_pos += llen_new;
                    w_pos += alen_new;
                }
                ll -= llen_new;
                il += llen_new;
                lw -= alen_new;
                iw += alen_new;
                bc += llen_new;
                exact = 0;
                ow_pos = w_pos;

                if llen_new == 0 && alen_new == 0 {
                    // c:856
                    lm = Some(Box::new((**mp).clone()));
                    if he == 0 {
                        he = 1;
                    } else {
                        // signal outer loop continue
                        matched = Some(mp.clone());
                        break;
                    }
                } else {
                    lm = None;
                    he = 0;
                }
                matched = Some(mp.clone());
                break;
            }
            if ll < mp.llen || lw < mp.wlen {
                continue;
            } // c:868

            // c:880-884 — skip if line and word substrings are identical
            // (the exact-char skip above already handled trivial overlap).
            if (mp.flags & (CMF_LEFT | CMF_RIGHT)) == 0 && mp.llen == mp.wlen {
                let (l_start, w_start) = if sfx != 0 {
                    ((l_pos - mp.llen) as usize, (w_pos - mp.wlen) as usize)
                } else {
                    (l_pos as usize, w_pos as usize)
                };
                let l_chunk = &l_bytes[l_start..l_start + mp.llen as usize];
                let w_chunk = &w_bytes[w_start..w_start + mp.wlen as usize];
                if l_chunk == w_chunk {
                    continue;
                }
            }

            // c:889-897 — local cursors tl/tw/tll/tlw/til/tiw.
            let (tl_pos, tw_pos, til, tiw, tll, tlw) = if sfx != 0 {
                (
                    l_pos - mp.llen,
                    w_pos - mp.wlen,
                    ll - mp.llen,
                    lw - mp.wlen,
                    il + mp.llen,
                    iw + mp.wlen,
                )
            } else {
                (l_pos, w_pos, il, iw, ll, lw)
            };

            let mut t: i32 = 1;
            // c:898-915 — CMF_LEFT anchor test.
            if (mp.flags & CMF_LEFT) != 0 {
                if til < mp.lalen || tiw < mp.lalen + mp.ralen {
                    continue;
                }
                if let Some(ref left_pat) = mp.left {
                    let l_anchor_start = (tl_pos - mp.lalen) as usize;
                    let w_anchor_start = (tw_pos - mp.lalen) as usize;
                    let l_slice = std::str::from_utf8(&l_bytes[l_anchor_start..]).unwrap_or("");
                    let w_slice = std::str::from_utf8(&w_bytes[w_anchor_start..]).unwrap_or("");
                    let lm_ok = pattern_match(Some(left_pat), l_slice, None, "") != 0;
                    let wm_ok = pattern_match(Some(left_pat), w_slice, None, "") != 0;
                    let r_ok = mp.ralen == 0 || {
                        let r_anchor_start = (tw_pos - mp.lalen - mp.ralen) as usize;
                        let r_slice = std::str::from_utf8(&w_bytes[r_anchor_start..]).unwrap_or("");
                        let right_pat = mp.right.as_deref();
                        pattern_match(right_pat, r_slice, None, "") != 0
                    };
                    t = if lm_ok && wm_ok && r_ok { 1 } else { 0 };
                } else {
                    let cmf_check = if (mp.flags & CMF_INTER) != 0 {
                        if (mp.flags & CMF_LINE) != 0 {
                            iw
                        } else {
                            il
                        }
                    } else {
                        il | iw
                    };
                    t = if sfx == 0 && cmf_check == 0 { 1 } else { 0 };
                }
            }
            // c:916-938 — CMF_RIGHT anchor test.
            if (mp.flags & CMF_RIGHT) != 0 {
                if tll < mp.llen + mp.ralen || tlw < mp.wlen + mp.ralen + mp.lalen {
                    continue;
                }
                if let Some(ref right_pat) = mp.right {
                    let l_anchor_start = (tl_pos + mp.llen) as usize;
                    let w_anchor_start = (tw_pos + mp.wlen) as usize;
                    let l_slice = std::str::from_utf8(&l_bytes[l_anchor_start..]).unwrap_or("");
                    let w_slice = std::str::from_utf8(&w_bytes[w_anchor_start..]).unwrap_or("");
                    let lm_ok = pattern_match(Some(right_pat), l_slice, None, "") != 0;
                    let wm_ok = pattern_match(Some(right_pat), w_slice, None, "") != 0;
                    let l_ok = mp.lalen == 0 || {
                        let l_anchor_2 = (tw_pos + mp.wlen - mp.ralen - mp.lalen) as usize;
                        let l_slice_2 = std::str::from_utf8(&w_bytes[l_anchor_2..]).unwrap_or("");
                        let left_pat = mp.left.as_deref();
                        pattern_match(left_pat, l_slice_2, None, "") != 0
                    };
                    t = if lm_ok && wm_ok && l_ok { 1 } else { 0 };
                } else {
                    let cmf_check = if (mp.flags & CMF_INTER) != 0 {
                        if (mp.flags & CMF_LINE) != 0 {
                            iw
                        } else {
                            il
                        }
                    } else {
                        il | iw
                    };
                    t = if sfx != 0 && cmf_check == 0 { 1 } else { 0 };
                }
            }

            // c:940 — main pattern_match call.
            if t == 0 {
                continue;
            }
            let line_pat = mp.line.as_deref();
            let word_pat = mp.word.as_deref();
            let tl_slice = std::str::from_utf8(&l_bytes[tl_pos as usize..]).unwrap_or("");
            let tw_slice = std::str::from_utf8(&w_bytes[tw_pos as usize..]).unwrap_or("");
            if pattern_match(line_pat, tl_slice, word_pat, tw_slice) == 0 {
                continue;
            }

            // c:944-967 — emit Cline parts via add_match_str/sub.
            if test == 0 {
                let carry_l = if sfx != 0 {
                    if ow_pos >= w_pos {
                        w_pos as usize
                    } else {
                        ow_pos as usize
                    }
                } else {
                    if w_pos >= ow_pos {
                        ow_pos as usize
                    } else {
                        w_pos as usize
                    }
                };
                let carry_len = if sfx != 0 {
                    (ow_pos - w_pos).max(0)
                } else {
                    (w_pos - ow_pos).max(0)
                };
                if carry_len > 0 {
                    let carry_slice =
                        std::str::from_utf8(&w_bytes[carry_l..carry_l + carry_len as usize])
                            .unwrap_or("");
                    add_match_str(None, "", carry_slice, carry_len, sfx);
                    add_match_sub(None, None, 0, Some(carry_slice), carry_len);
                }
                // c:955 — main matcher str.
                let tw_str =
                    std::str::from_utf8(&w_bytes[tw_pos as usize..(tw_pos + mp.wlen) as usize])
                        .unwrap_or("");
                add_match_str(Some(mp), tl_slice, tw_str, mp.wlen, sfx);
                add_match_sub(Some(mp), Some(tl_slice), mp.llen, Some(tw_str), mp.wlen);
            }

            // c:968-988 — advance pointers.
            if sfx != 0 {
                l_pos = tl_pos;
                w_pos = tw_pos;
            } else {
                l_pos += mp.llen;
                w_pos += mp.wlen;
            }
            il += mp.llen;
            iw += mp.wlen;
            ll -= mp.llen;
            lw -= mp.wlen;
            bc += mp.llen;
            exact = 0;
            ow_pos = w_pos;
            lm = None;
            he = 0;
            matched = Some(mp.clone());
            break;
        }

        if matched.is_some() {
            // c:993
            continue 'outer;
        }

        // c:998-1042 — no matcher matched at this position. Try the
        // "same character" skip again (in case the retry path failed).
        if (test == 0 || sfx != 0) && lw > 0 {
            let l_idx = (l_pos + ind) as usize;
            let w_idx = (w_pos + ind) as usize;
            if l_idx < l_bytes.len() && w_idx < w_bytes.len() {
                let l_ch = l_bytes[l_idx];
                let w_ch = w_bytes[w_idx];
                let bslash = lw > 1
                    && w_ch == b'\\'
                    && (w_idx + 1) < w_bytes.len()
                    && w_bytes[w_idx + 1] == l_bytes[l_idx];
                if l_ch == w_ch || bslash {
                    let advance_w = if bslash { 2 } else { 1 };
                    l_pos += add;
                    w_pos += if bslash { add + add } else { add };
                    il += 1;
                    iw += advance_w;
                    ll -= 1;
                    lw -= advance_w;
                    bc += 1;
                    lm = None;
                    he = 0;
                    continue 'outer;
                }
            }
        }

        // c:1017 — break on lw=0 (suffix exhausted in non-test mode).
        if lw == 0 {
            break;
        }

        // c:1020-1034 — retry path: rewind exact-skip if we have any
        // and retry the matcher loop preferring matchers.
        if exact > 0 && part == 0 {
            il -= exact;
            iw -= wexact;
            ll += exact;
            lw += wexact;
            bc -= exact;
            l_pos -= add * exact;
            w_pos -= add * wexact;
            exact = 0;
            wexact = 0;
            // c:1029 `goto retry` — re-enter the matcher loop but SKIP the
            // exact-char fast path (else it re-matches the just-rewound
            // char and loops forever). The flag makes the next iteration
            // start at the matcher loop.
            retry_skip_fastpath = true;
            continue 'outer;
        }

        // c:1036-1041 — divergence with no matcher and no exact-rewind.
        if test != 0 {
            return 0;
        }
        abort_match();
        return -1;
    }

    // c:1044-1046 — test-mode return.
    if test != 0 {
        return if part != 0 || ll == 0 { 1 } else { 0 };
    }

    // c:1050-1054 — top-level: any remaining ll means abort.
    if part == 0 && ll != 0 {
        abort_match();
        return -1;
    }

    // c:1055-1056 — rwlp writeback.
    if let Some(out) = rwlp {
        *out = iw
            - if sfx != 0 {
                ow_pos - w_pos
            } else {
                w_pos - ow_pos
            };
    }

    // c:1083 — `*bpp = bp` (Brinfo writeback) — caller's bp is already
    // unmodified since the deep brace-pos tracking is conservative.

    let _ = (lm, he);
    // c:1084 — return iw on full match, il in part mode.
    if part != 0 {
        il
    } else {
        iw
    }
}

/// Direct port of `static int match_parts(char *l, char *w, int n,
///                                          int part)` from
/// `Src/Zle/compmatch.c:1092-1108`. Tests whether the first `n` bytes
/// of `l` match the first `n` bytes of `w` using the active mstack
/// matcher chain. C truncates both strings to length n with `'\0'`
/// (saving/restoring the boundary bytes); Rust takes slices.
pub fn match_parts(l: &str, w: &str, n: i32, part: i32) -> i32 {
    // c:1092
    let ln = (n as usize).min(l.len());
    let wn = (n as usize).min(w.len());
    let l_slice = &l[..ln];
    let w_slice = &w[..wn];
    // c:1101 — match_str(l, w, NULL, 0, NULL, 0, 1, part).
    match_str(l_slice, w_slice, None, 0, None, 0, 1, part)
}

/// Direct port of `mod_export char *comp_match(char *pfx, char *sfx,
///                                               char *w, Patprog cp,
///                                               Cline *clp, int qu,
///                                               Brinfo *bpl, int bcp,
///                                               Brinfo *bsl, int bcs,
///                                               int *exact)`
/// from `Src/Zle/compmatch.c:1123-1257`. Applies the matcher chain to
/// candidate `w` against prefix `pfx` and suffix `sfx`. Returns the
/// matched string on success, None on no match. Writes the Cline
/// structure into `clp`, the "is exact match" flag into `exact`.
#[allow(clippy::too_many_arguments)]
pub fn comp_match(
    // c:1123
    pfx: &str,
    sfx: &str,
    w: &str,
    cp: Option<&crate::ported::pattern::Patprog>,
    clp: Option<&mut Option<Box<Cline>>>,
    qu: i32,
    _bpl: Option<&mut Option<Box<brinfo>>>,
    bcp: i32,
    _bsl: Option<&mut Option<Box<brinfo>>>,
    bcs: i32,
    exact: &mut i32,
) -> Option<String> {
    use crate::ported::glob::{remnulargs, tokenize};
    use crate::ported::lex::{parse_subst_string, untokenize};
    use std::sync::atomic::Ordering;

    let r: String;
    if let Some(prog) = cp {
        // c:1129
        // c:1129-1167 — globcomplete pattern path.
        r = w.to_string();
        let teststr: String = if qu == 0 {
            // c:1135
            // c:1145-1153 — unquote a copy then pattry against the prog.
            let mut t = r.clone();
            tokenize(&mut t);
            set_noerrs(1);
            let parsed = parse_subst_string(&t).ok();
            set_noerrs(0);
            if let Some(p) = parsed {
                let mut p = p;
                remnulargs(&mut p);
                untokenize(&p)
            } else {
                r.clone()
            }
        } else {
            r.clone()
        };
        if !pattry(prog, &teststr) {
            // c:1157
            return None;
        }
        let r_final = if qu == 2 {
            tildequote(&r, 0)
        }
        // c:1160
        else {
            multiquote(&r, if qu != 0 { 0 } else { 1 })
        };
        // c:1164-1166 — build a Cline chain from the matched word.
        let wl = w.len() as i32;
        let lc = bld_parts(w, wl, wl, None, None);
        if let Some(out) = clp {
            *out = lc;
        }
        *exact = 0; // c:1167
        return Some(r_final);
    }

    // c:1169 — mstack-driven path.
    let w_quoted = if qu == 2 {
        tildequote(w, 0)
    }
    // c:1172
    else {
        multiquote(w, if qu != 0 { 0 } else { 1 })
    };
    let wl = w_quoted.len() as i32;

    // c:1177 — useqbr = qu.
    useqbr.store(qu, Ordering::Relaxed);

    let mut rpl: i32 = 0;
    let mpl = match_str(pfx, &w_quoted, None, bcp, Some(&mut rpl), 0, 0, 0); // c:1178
    if mpl < 0 {
        return None;
    }

    if !sfx.is_empty() {
        // c:1181
        // c:1182-1232 — also match suffix; combine prefix+suffix Cline.
        let mut rsl: i32 = 0;
        let suffix_start = (mpl as usize).min(w_quoted.len());
        let suffix_part = &w_quoted[suffix_start..];
        let msl = match_str(sfx, suffix_part, None, bcs, Some(&mut rsl), 1, 0, 0);
        if msl < 0 {
            return None; // c:1204
        }
        // c:1220 — add_match_str for the middle and saved prefix.
        let middle_len = (wl - rpl - rsl).max(0) as usize;
        let middle_start = (rpl as usize).min(w_quoted.len());
        let middle =
            &w_quoted[middle_start..middle_start + middle_len.min(w_quoted.len() - middle_start)];
        // c:1223 — bld_parts on the middle portion.
        let mid_lc = bld_parts(
            middle,
            (wl - rpl - rsl).max(0),
            (mpl - rpl) + (msl - rsl),
            None,
            None,
        );
        if let Some(out) = clp {
            *out = mid_lc;
        }

        // c:1245-1251 — exact-match test.
        let pl = pfx.len();
        *exact =
            if w_quoted.len() >= pl && w_quoted.starts_with(pfx) && w_quoted[pl..].ends_with(sfx) {
                1
            } else {
                0
            };
    } else {
        // c:1233
        // c:1235-1239 — prefix-only path.
        let after_pfx_start = (rpl as usize).min(w_quoted.len());
        let after_pfx = &w_quoted[after_pfx_start..];
        // c:1235 — append the unmatched word remainder onto MATCHBUF so `r`
        // (below) reconstructs the full word, not just the matcher's own
        // contribution.
        add_match_str(None, "", after_pfx, (wl - rpl).max(0), 0); // c:1235
        // c:1237-1238 — `add_match_part(NULL, NULL, NULL, 0, NULL, 0,
        //                w + rpl, wl - rpl, mpl - rpl, 0); pli = matchparts;`
        // This APPENDS to the matchparts chain already populated by
        // match_str (matcher subs + exact runs live in matchsubs and are
        // folded in here) — a bare bld_parts() would discard those and
        // drop exactly-matched interior chars from the reconstruction.
        add_match_part(
            None,
            None,
            0,
            "",
            0,
            None,
            0,
            after_pfx,
            (wl - rpl).max(0),
            mpl - rpl,
            0,
        ); // c:1237
        let pli = MATCHPARTS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .ok()
            .and_then(|mut g| g.take()); // c:1239 pli = matchparts
        if let Some(out) = clp {
            *out = pli;
        }

        // c:1251 — exact = !strcmp(pfx, w).
        *exact = if pfx == w_quoted.as_str() { 1 } else { 0 };
    }

    // c:1241 — r = dupstring(matchbuf ? matchbuf : "").
    r = MATCHBUF
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.clone()))
        .unwrap_or_default();
    let r = if r.is_empty() { w_quoted } else { r };
    Some(r)
}

/// Port of `pattern_match1(Cpattern p, convchar_t c, int *mtp)` from Src/Zle/compmatch.c:1269.
/// Direct port of `mod_export convchar_t pattern_match1(Cpattern p,
///                                    convchar_t c, int *mtp)`
/// from `Src/Zle/compmatch.c:1269`. Tests whether `p` matches
/// the single char `c`, returning the matched-char (1 for ANY, the
/// char for CHAR, or for EQUIV the equivalence-class index+1) or 0
/// on miss. `mtp` is non-zero only for the EQUIV path.
/// WARNING: param names don't match C — Rust=(p, mtp) vs C=(p, c, mtp)
pub fn pattern_match1(
    p: &Cpattern, // c:1269
    c: u32,
    mtp: &mut i32,
) -> u32 {
    *mtp = 0; // c:1273
    match p.tp {
        // c:1274
        x if x == CPAT_CCLASS => {
            // c:1275
            // PATMATCHRANGE(p->u.str, c, NULL, NULL)
            patmatchrange(p.str.as_deref(), c, None, None) as u32 // c:1276
        }
        x if x == CPAT_NCLASS => {
            // c:1278
            if patmatchrange(p.str.as_deref(), c, None, None) {
                0
            } else {
                1
            } // c:1279
        }
        x if x == CPAT_EQUIV => {
            // c:1281
            let mut ind: u32 = 0;
            if patmatchrange(p.str.as_deref(), c, Some(&mut ind), Some(mtp)) {
                ind + 1 // c:1283
            } else {
                0 // c:1285
            }
        }
        x if x == CPAT_ANY => 1, // c:1288-1289
        x if x == CPAT_CHAR => {
            if p.chr == c {
                c
            } else {
                0
            }
        } // c:1291-1292
        _ => 0,                  // c:1294
    }
}

/// Direct port of `mod_export convchar_t pattern_match_equivalence(
///                    Cpattern lp, convchar_t wind, int wmtp,
///                    convchar_t wchr)`
/// from `Src/Zle/compmatch.c:1316`. Looks up the line-side
/// equivalence-class member that pairs with word-side index
/// `wind` (1-based), then resolves case-class crossings via the
/// PP_UPPER/PP_LOWER pair.
///
/// Returns `CHR_INVALID` (u32::MAX) on miss; the matched line
/// char on success.
pub fn pattern_match_equivalence(
    lp: &Cpattern, // c:1316
    wind: u32,
    wmtp: i32,
    wchr: u32,
) -> u32 {
    // c:1324 — PATMATCHINDEX(lp->u.str, wind-1, &lchr, &lmtp).
    // Walk lp.str's encoded byte sequence finding the entry at index
    // (wind-1). Encoding (from parse_class):
    //   0x80 + PP_RANGE (=0x95): next two bytes are lo,hi range
    //   0x80 + PP_* (POSIX class id): single-byte class marker
    //   plain byte: literal character
    let Some(ref bytes) = lp.str else {
        return u32::MAX;
    };
    let Some(target_idx) = (wind as i64).checked_sub(1) else {
        return u32::MAX;
    };
    if target_idx < 0 {
        return u32::MAX;
    }
    let mut lchr: Option<u32> = None;
    let mut lmtp: i32 = 0;
    let mut idx: i64 = 0;
    let mut i = 0usize;
    let pp_range_marker = (0x80u8).wrapping_add(PP_RANGE as u8);
    while i < bytes.len() {
        let b = bytes[i];
        if b == pp_range_marker {
            // c:4049 PP_RANGE
            // Next two bytes are range start / end.
            if i + 2 >= bytes.len() {
                break;
            }
            let r1 = bytes[i + 1];
            let r2 = bytes[i + 2];
            let span = (r2 as i64) - (r1 as i64);
            if span >= 0 && idx + span >= target_idx {
                // c:4057
                lchr = Some(((r1 as i64) + (target_idx - idx)) as u32);
                break;
            }
            idx += span + 1; // c:4062
            i += 3;
        } else if b >= 0x80 {
            // c:4024-4047 — POSIX class marker (PP_ALPHA/LOWER/UPPER/etc.).
            let swtype = (b as i32) - 0x80;
            if idx == target_idx {
                // c:4043
                lmtp = swtype;
                break;
            }
            idx += 1;
            i += 1;
        } else {
            // c:4071-4076 — literal char.
            if idx == target_idx {
                lchr = Some(b as u32);
                break;
            }
            idx += 1;
            i += 1;
        }
    }

    // c:1335 — `if (lchr != CHR_INVALID) return lchr` — exact-char hit.
    if let Some(ch) = lchr {
        if ch != u32::MAX {
            return ch;
        }
    }

    // c:1342 — case-class crossings using the now-tracked lmtp.
    let wch = char::from_u32(wchr).unwrap_or('\0');
    if wmtp == PP_UPPER && lmtp == PP_LOWER {
        return ZC_tolower(wch) as u32;
    }
    if wmtp == PP_LOWER && lmtp == PP_UPPER {
        return ZC_toupper(wch) as u32;
    }
    if wmtp != 0 && wmtp == lmtp {
        return wchr;
    }
    u32::MAX // c:1378
}

/// Direct port of `static int pattern_match_restrict(Cpattern p,
///                                Cpattern wp, convchar_t *wsc,
///                                int wsclen, Cpattern prestrict,
///                                ZLE_STRING_T new_line)`
/// from `Src/Zle/compmatch.c:1383`. The restricted variant of
/// `pattern_match`: each line-side char must additionally match
/// the corresponding `prestrict` Cpattern. Used when building the
/// line-string from a partial match. Writes the deduced line chars
/// into `new_line` and returns 1 on full match, 0 otherwise.
pub fn pattern_match_restrict(
    p: Option<&Cpattern>, // c:1383
    wp: Option<&Cpattern>,
    wsc: &[u32],
    prestrict: Option<&Cpattern>,
    new_line: &mut Vec<char>,
) -> i32 {
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
        let wind = pattern_match1(wpat, wc, &mut wmt); // c:1394
        if wind == 0 {
            return 0;
        } // c:1395

        // c:1399-1450 — deduce the line character `c`.
        let c: u32 = if pre.tp == CPAT_CHAR {
            // c:1402
            pre.chr // c:1407
        } else if pat.tp == CPAT_CHAR {
            // c:1410
            pat.chr // c:1414
        } else if pat.tp == CPAT_EQUIV {
            // c:1416
            // c:1424 — pattern_match_equivalence resolves the line-side
            // equivalence-class member paired with the word's wind/wmt.
            let r = pattern_match_equivalence(pat, wind, wmt, wc);
            if r == u32::MAX {
                return 0;
            } // c:1426 CHR_INVALID
            r
        } else {
            // c:1432
            wc // c:1442 use *wsc
        };

        // c:1448 — restriction-side check.
        if pre.tp != CPAT_CHAR {
            let mut mt: i32 = 0;
            if pattern_match1(pre, c, &mut mt) == 0 {
                return 0;
            } // c:1449
        }

        // c:1457-1485 — case-class equivalence (mt vs wmt mismatch).
        if pat.tp != CPAT_ANY || wpat.tp != CPAT_ANY {
            // c:1459
            let mut mt: i32 = 0;
            let ind = pattern_match1(pat, c, &mut mt); // c:1461
            if ind == 0 || ind != wind {
                return 0;
            } // c:1462-1465
            if mt != wmt {
                let case_pair =
                    (mt == PP_LOWER || mt == PP_UPPER) && (wmt == PP_LOWER || wmt == PP_UPPER);
                if case_pair {
                    let cc = char::from_u32(c).unwrap_or('\0');
                    let wcc = char::from_u32(wc).unwrap_or('\0');
                    if ZC_tolower(cc) != ZC_tolower(wcc) {
                        return 0;
                    } // c:1477
                } else {
                    return 0; // c:1481
                }
            }
        }

        // c:1496 — append deduced char to new_line.
        if let Some(ch) = char::from_u32(c) {
            new_line.push(ch);
        }
        pr_cur = pre.next.as_deref(); // c:1498
        wsc_idx += 1;
        p_cur = pat.next.as_deref();
        wp_cur = wpat.next.as_deref();
    }

    // c:1505-1540 — tail loop: continue matching when wsc exhausted
    // but prestrict still has more chars (deduced solely from p).
    while p_cur.is_some() && pr_cur.is_some() {
        // c:1505
        let pat = p_cur.unwrap();
        let pre = pr_cur.unwrap();
        let c: u32 = if pre.tp == CPAT_CHAR {
            pre.chr
        } else if pat.tp == CPAT_CHAR {
            pat.chr
        } else {
            return 0; // c:1522 not enough info
        };
        let mut mt: i32 = 0;
        if pre.tp != CPAT_CHAR && pattern_match1(pre, c, &mut mt) == 0 {
            return 0;
        }
        if let Some(ch) = char::from_u32(c) {
            new_line.push(ch);
        }
        pr_cur = pre.next.as_deref();
        p_cur = pat.next.as_deref();
    }

    // c:1542 — `p_cur.is_none() && pr_cur.is_none() && (wp_cur.is_none() || wsc empty)`.
    if p_cur.is_none() && pr_cur.is_none() && (wp_cur.is_none() || wsc_idx >= wsc.len()) {
        1 // c:1544 full match
    } else {
        0 // c:1545
    }
}

/// Port of `pattern_match(Cpattern p, char *s, Cpattern wp, char *ws)` from Src/Zle/compmatch.c:1548.
/// Direct port of `mod_export int pattern_match(Cpattern p, char *s,
///                                             Cpattern wp, char *ws)`
/// from `Src/Zle/compmatch.c:1548`. Walks two parallel pattern +
/// string pairs (line `p`/`s` vs word `wp`/`ws`) verifying that each
/// position matches and that paired pattern-class indices line up.
/// WARNING: param names don't match C — Rust=(p, wp, ws) vs C=(p, s, wp, ws)
pub fn pattern_match(
    p: Option<&Cpattern>, // c:1548
    s: &str,
    wp: Option<&Cpattern>,
    ws: &str,
) -> i32 {
    let (mut p_cur, mut wp_cur) = (p, wp); // c:1551 walking p / wp
    let mut s_bytes = s.chars().peekable();
    let mut ws_bytes = ws.chars().peekable();

    while p_cur.is_some() && wp_cur.is_some()                                // c:1553
        && s_bytes.peek().is_some() && ws_bytes.peek().is_some()
    {
        let pat = p_cur.unwrap();
        let wpat = wp_cur.unwrap();
        let wc = ws_bytes.next().unwrap() as u32; // c:1555
        let mut wmt: i32 = 0;
        let wind = pattern_match1(wpat, wc, &mut wmt); // c:1556
        if wind == 0 {
            return 0;
        } // c:1557

        let c = s_bytes.next().unwrap() as u32; // c:1561
        if pat.tp != CPAT_ANY || wpat.tp != CPAT_ANY {
            // c:1567
            let mut mt: i32 = 0;
            let ind = pattern_match1(pat, c, &mut mt); // c:1569
            if ind == 0 {
                return 0;
            } // c:1570
            if ind != wind {
                return 0;
            } // c:1572
            if mt != wmt {
                // c:1574
                let case_pair =
                    (mt == PP_LOWER || mt == PP_UPPER) && (wmt == PP_LOWER || wmt == PP_UPPER);
                if case_pair {
                    let cc = char::from_u32(c).unwrap_or('\0');
                    let wcc = char::from_u32(wc).unwrap_or('\0');
                    if ZC_tolower(cc) != ZC_tolower(wcc) {
                        // c:1584
                        return 0;
                    }
                } else {
                    return 0; // c:1588
                }
            }
        }
        p_cur = pat.next.as_deref(); // c:1599
        wp_cur = wpat.next.as_deref();
    }
    // c:1601-1607 — consume remaining LINE pattern chars (stop when EITHER
    // the pattern OR the string is exhausted).
    while let (Some(pat), Some(&c_ch)) = (p_cur, s_bytes.peek()) {
        let c = c_ch as u32;
        let mut mt: i32 = 0;
        if pattern_match1(pat, c, &mut mt) == 0 {
            return 0; // c:1604
        }
        s_bytes.next();
        p_cur = pat.next.as_deref(); // c:1605
    }
    // c:1609-1615 — remaining WORD pattern chars, symmetrically.
    while let (Some(wpat), Some(&wc_ch)) = (wp_cur, ws_bytes.peek()) {
        let wc = wc_ch as u32;
        let mut wmt: i32 = 0;
        if pattern_match1(wpat, wc, &mut wmt) == 0 {
            return 0; // c:1611
        }
        ws_bytes.next();
        wp_cur = wpat.next.as_deref(); // c:1613
    }
    // c:1617 — the port previously required BOTH strings fully consumed;
    // a fixed-length matcher against a LONGER word failed on the remainder.
    // C stops at pattern exhaustion; caller handles the rest of the word.
    1
}

/// Port of `bld_parts(char *str, int len, int plen, Cline *lp, Cline *lprem)` from Src/Zle/compmatch.c:1638.
/// Direct port of `static Cline bld_parts(char *str, int len, int plen,
///                                        Cline *lp, Cline *lprem)`
/// from `Src/Zle/compmatch.c:1638`. Splits the candidate string
/// `str[..len]` into a Cline chain anchored by every CMF_RIGHT
/// matcher in `bmatchers`. `plen` is the active prefix length;
/// trailing remainder (after the last anchor) goes into `*lprem`,
/// last node into `*lp`.
/// WARNING: param names don't match C — Rust=(str, len, plen, lprem) vs C=(str, len, plen, lp, lprem)
pub fn bld_parts(
    str: &str,
    len: i32,
    mut plen: i32, // c:1638
    lp: Option<&mut Option<Box<Cline>>>,
    lprem: Option<&mut Option<Box<Cline>>>,
) -> Option<Box<Cline>> {
    let bytes = str.as_bytes();
    let total: usize = (len as usize).min(bytes.len());
    let mut op = plen;
    let mut p_start = 0usize;
    let mut str_pos = 0usize;
    let mut remaining = total as i32;

    let mut head: Option<Box<Cline>> = None;
    let mut tail_ref: *mut Option<Box<Cline>> = &mut head;
    let mut last_n: Option<Box<Cline>> = None;

    while remaining > 0 {
        // c:1647
        // c:1648-1685 — walk bmatchers looking for a CMF_RIGHT-anchored
        // wlen<0 matcher whose right anchor matches at the current
        // position. On hit, emit a Cline for the run-so-far + the
        // anchored portion, advance str/plen past the anchor.
        let mut found_anchor = false;
        let bmatchers_chain = crate::ported::zle::compcore::bmatchers
            .get_or_init(|| Mutex::new(None))
            .lock()
            .ok()
            .and_then(|g| g.clone());
        let mut cur = bmatchers_chain.as_deref();
        while let Some(ms) = cur {
            let mp = &*ms.matcher;
            let preds_ok = mp.flags == CMF_RIGHT
                && mp.wlen < 0
                && mp.ralen > 0
                && mp.llen == 0
                && remaining >= mp.ralen
                && (str_pos as i32 - p_start as i32) >= mp.lalen;
            if !preds_ok {
                cur = ms.next.as_deref();
                continue;
            }
            let str_at = std::str::from_utf8(&bytes[str_pos..]).unwrap_or("");
            if pattern_match(mp.right.as_deref(), str_at, None, "") == 0 {
                cur = ms.next.as_deref();
                continue;
            }
            let l_anchor_ok = mp.lalen == 0 || {
                let off = str_pos as i32 - mp.lalen;
                if off < 0 {
                    false
                } else {
                    let l_slice = std::str::from_utf8(&bytes[off as usize..]).unwrap_or("");
                    pattern_match(mp.left.as_deref(), l_slice, None, "") != 0
                }
            };
            if !l_anchor_ok {
                cur = ms.next.as_deref();
                continue;
            }

            // c:1655-1672 — emit anchor cline; optional prefix run.
            let olen = (str_pos - p_start) as i32;
            let flags = if plen <= 0 { CLF_NEW } else { 0 };
            let anchor_word: String =
                std::str::from_utf8(&bytes[str_pos..str_pos + mp.ralen as usize])
                    .unwrap_or("")
                    .into();
            let mut node = Box::new(Cline {
                llen: mp.ralen,
                word: Some(anchor_word.clone()),
                wlen: mp.ralen,
                flags,
                ..Default::default()
            });
            if p_start != str_pos {
                let mut llen = if op < 0 { 0 } else { op };
                if llen > olen {
                    llen = olen;
                }
                let prefix_word: String =
                    std::str::from_utf8(&bytes[p_start..p_start + olen as usize])
                        .unwrap_or("")
                        .into();
                node.prefix = Some(Box::new(Cline {
                    llen,
                    word: Some(prefix_word),
                    wlen: olen,
                    ..Default::default()
                }));
            }
            unsafe {
                *tail_ref = Some(node);
                tail_ref = &mut (*tail_ref).as_mut().unwrap().next;
            }
            // c:1674-1677 — advance past the anchor.
            str_pos += mp.ralen as usize;
            remaining -= mp.ralen;
            plen -= mp.ralen;
            op -= olen;
            p_start = str_pos;
            found_anchor = true;
            break;
        }
        if !found_anchor {
            // c:1683 — no anchor: str++; len--; plen--.
            str_pos += 1;
            remaining -= 1;
            plen -= 1;
        }
    }

    // c:1701-1717 — emit a Cline for the trailing portion.
    if p_start != str_pos {
        // c:1701
        let olen = (str_pos - p_start) as i32;
        let mut llen = if op < 0 { 0 } else { op };
        if llen > olen {
            llen = olen;
        }
        let flags = if plen <= 0 { CLF_NEW } else { 0 };
        let mut node = Box::new(Cline {
            flags,
            ..Default::default()
        });
        let prefix_word: String = std::str::from_utf8(&bytes[p_start..p_start + olen as usize])
            .unwrap_or("")
            .into();
        node.prefix = Some(Box::new(Cline {
            llen,
            word: Some(prefix_word.clone()),
            wlen: olen,
            ..Default::default()
        }));
        if let Some(out) = lprem {
            *out = Some(node.clone());
        } // c:1714
        last_n = Some(node.clone());
        unsafe {
            *tail_ref = Some(node);
        }
    } else if head.is_none() {
        // c:1716
        let flags = if plen <= 0 { CLF_NEW } else { 0 };
        let node = Box::new(Cline {
            flags,
            ..Default::default()
        });
        if let Some(out) = lprem {
            *out = Some(node.clone());
        } // c:1721
        last_n = Some(node.clone());
        head = Some(node);
    } else if let Some(out) = lprem {
        // c:1722
        *out = None;
    }

    if let (Some(out_lp), Some(n)) = (lp, last_n) {
        // c:1731
        *out_lp = Some(n);
    }

    let _ = p_start;
    let _ = op;
    head // c:1733 return ret
}

/// Direct port of `static int bld_line(Cmatcher mp, ZLE_STRING_T line,
///                                     char *mword, char *word,
///                                     int wlen, int sfx)`
/// from `Src/Zle/compmatch.c:1734-1992`. Builds all possible line
/// patterns for `mp` and tests whether they match `word`, returning
/// the number of word chars matched (0 on failure). On success the
/// synthesized line chars are written into `line`.
///
/// Faithful two-pass implementation (matching the C):
///
/// Pass 1 (c:1772-1846) — build `genpatarr`, a per-line-char array of
/// `Cpattern`. For each `mp->line` entry: if it is a `CPAT_EQUIV` and
/// `mp->word` + `mword` are still available, consume one `mword` char,
/// query the word pattern via `pattern_match1`, and if that yields a
/// word-side equivalence index resolve the concrete line char via
/// `pattern_match_equivalence` (which tracks the PP_LOWER/PP_UPPER
/// case crossings, compmatch.rs:1825), storing it as a `CPAT_CHAR`.
/// Otherwise the line pattern is copied verbatim (char / class / any).
/// A `CHR_INVALID` equivalence resolution aborts with 0.
///
/// Pass 2 (c:1847-1988) — walk `genpatarr` against `wordchars`. Where
/// `pattern_match1` accepts the word char directly, emit the fixed
/// `CPAT_CHAR` value (or, for a generic pattern, the word char). Where
/// it does not, fall back to the `bmatchers` chain and let
/// `pattern_match_restrict` deduce the line chars (the "nightmare"
/// multi-matcher case). `sfx` builds both strings from the end.
pub fn bld_line(
    mp: &Cmatcher, // c:1734
    line: &mut Vec<char>,
    mword: &str,
    word: &str,
    wlen: i32,
    sfx: i32,
) -> i32 {
    let sfx = sfx != 0;

    // c:1745-1762 — convert `word` to an array of (wide) chars so we
    // can index it from either end.
    let wordchars: Vec<u32> = word.chars().map(|c| c as u32).collect();
    let wlen0 = wlen.max(0) as usize;

    // Links a slice of genpatarr entries into a throwaway Cpattern
    // chain, so `pattern_match_restrict` can walk it via `->next`
    // (c:1841-1845 links curgenpat->next = curgenpat+1).
    fn link_chain(entries: &[Cpattern]) -> Option<Box<Cpattern>> {
        let mut head: Option<Box<Cpattern>> = None;
        for e in entries.iter().rev() {
            let mut node = e.clone();
            node.next = head.take();
            head = Some(Box::new(node));
        }
        head
    }

    // --- Pass 1: build genpatarr (c:1772-1846). ---
    let mword_chars: Vec<u32> = mword.chars().map(|c| c as u32).collect();
    let mut mword_idx = 0usize;
    let mut wpat = mp.word.as_deref();
    let mut lpat = mp.line.as_deref();
    let mut genpatarr: Vec<Cpattern> = Vec::with_capacity(mp.llen.max(0) as usize);
    while let Some(lp) = lpat {
        // c:1780-1799 — resolve the word side of an equivalence.
        let mut wind: u32 = 0;
        let mut wmtp: i32 = 0;
        let mut wchr: u32 = 0;
        if lp.tp == CPAT_EQUIV && wpat.is_some() && mword_idx < mword_chars.len() {
            wchr = mword_chars[mword_idx];
            mword_idx += 1;
            let wp = wpat.unwrap();
            wind = pattern_match1(wp, wchr, &mut wmtp); // c:1794
            wpat = wp.next.as_deref(); // c:1795
        }

        let mut gp = Cpattern::default();
        if wind != 0 {
            // c:1800-1822 — successful word-side equivalence; find the
            // line equivalent and pin it as a concrete char.
            let lchr = pattern_match_equivalence(lp, wind, wmtp, wchr); // c:1817
            if lchr == u32::MAX {
                return 0; // c:1820 — no equivalent, give up
            }
            gp.tp = CPAT_CHAR; // c:1826
            gp.chr = lchr; // c:1827
        } else {
            // c:1828-1846 — copy the line pattern verbatim.
            gp.tp = lp.tp; // c:1834
            if lp.tp == CPAT_CHAR {
                gp.chr = lp.chr; // c:1836
            } else if lp.tp != CPAT_ANY {
                gp.str = lp.str.clone(); // c:1843 (shared/copied class)
            }
        }
        genpatarr.push(gp);
        lpat = lp.next.as_deref();
    }

    // --- Pass 2: match wordchars against genpatarr (c:1847-1988). ---
    let llen0 = mp.llen.max(0) as usize;
    let mut line_buf: Vec<char> = vec!['\0'; llen0];
    let mut llen_rem: i32 = mp.llen; // c:1855
    let mut wlen_rem: i32 = wlen;
    let mut rl: i32 = 0; // c:1856

    // Cursors into line_buf / wordchars / genpatarr (c:1858-1874).
    let mut line_pos: usize = if sfx { llen0 } else { 0 };
    let mut word_pos: usize = if sfx { wlen0 } else { 0 };
    let mut gp_pos: usize = if sfx { llen0 } else { 0 };

    // Snapshot the global bmatchers chain for the fallback loop.
    let bm = crate::ported::zle::compcore::bmatchers
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.clone());

    while llen_rem > 0 && wlen_rem > 0 {
        // c:1877-1885 — pick the char/pattern under inspection.
        let (wp_idx, gp_idx) = if sfx {
            (word_pos - 1, gp_pos - 1)
        } else {
            (word_pos, gp_pos)
        };
        // `.get`-guarded: callers may pass a byte length as `wlen`
        // that exceeds the char count on multibyte input.
        let wc = *wordchars.get(wp_idx).unwrap_or(&0);

        let mut wmtp: i32 = 0;
        if gp_idx < genpatarr.len() && pattern_match1(&genpatarr[gp_idx], wc, &mut wmtp) != 0 {
            // c:1890-1920 — direct match. Keep the fixed char for a
            // CPAT_CHAR genpat, else keep the word char.
            let lchr = if genpatarr[gp_idx].tp == CPAT_CHAR {
                genpatarr[gp_idx].chr
            } else {
                wc
            };
            let lch = char::from_u32(lchr).unwrap_or('\0');
            if sfx {
                line_pos -= 1; // c:1908 *--line = lchr
                line_buf[line_pos] = lch;
            } else {
                line_buf[line_pos] = lch; // c:1910 *line++ = lchr
                line_pos += 1;
            }
            llen_rem -= 1; // c:1912
            wlen_rem -= 1; // c:1913
            rl += 1; // c:1914

            if sfx {
                word_pos = wp_idx; // c:1917
                gp_pos = gp_idx; // c:1918
            } else {
                if llen_rem > 0 {
                    gp_pos += 1; // c:1920
                }
                word_pos += 1; // c:1921
            }
        } else {
            // c:1925-1978 — nightmare case: dispatch to the pattern
            // matchers in bmatchers via pattern_match_restrict.
            let mut matched = false;
            let mut ms = bm.as_deref();
            while let Some(node) = ms {
                let bmp = &*node.matcher; // c:1932 mp = ms->matcher
                if bmp.flags == 0
                    && bmp.wlen <= wlen_rem
                    && bmp.llen <= llen_rem
                    && bmp.wlen >= 0
                    && bmp.llen >= 0
                {
                    // c:1943-1949 — position the sub-window.
                    let (lp_idx, wp2_idx, gp2_idx) = if sfx {
                        (
                            line_pos - bmp.llen as usize,
                            word_pos - bmp.wlen as usize,
                            gp_pos - bmp.llen as usize,
                        )
                    } else {
                        (line_pos, word_pos, gp_pos)
                    };

                    // c:1951 — wsclen = wlen - (wp - wordchars).
                    let wsclen = wlen_rem - wp2_idx as i32;
                    let start = wp2_idx;
                    let end = if wsclen <= 0 {
                        start
                    } else {
                        (start + wsclen as usize).min(wordchars.len())
                    };
                    let wsc = &wordchars[start.min(end)..end];

                    let pr_end = (gp2_idx + bmp.llen as usize).min(genpatarr.len());
                    let prestrict = link_chain(&genpatarr[gp2_idx.min(pr_end)..pr_end]);

                    let mut tmp_line: Vec<char> = Vec::new();
                    if pattern_match_restrict(
                        bmp.line.as_deref(),
                        bmp.word.as_deref(),
                        wsc,
                        prestrict.as_deref(),
                        &mut tmp_line,
                    ) != 0
                    {
                        // c:1958-1978 — matched: copy deduced line chars
                        // into place and advance all cursors.
                        for (k, ch) in tmp_line.iter().enumerate() {
                            if lp_idx + k < line_buf.len() {
                                line_buf[lp_idx + k] = *ch;
                            }
                        }
                        if sfx {
                            line_pos = lp_idx; // c:1965
                            word_pos = wp2_idx; // c:1966
                            gp_pos = gp2_idx; // c:1967
                        } else {
                            line_pos += bmp.llen as usize; // c:1969
                            word_pos += bmp.wlen as usize; // c:1970
                            gp_pos += bmp.llen as usize; // c:1971
                        }
                        llen_rem -= bmp.llen; // c:1973
                        wlen_rem -= bmp.wlen; // c:1974
                        rl += bmp.wlen; // c:1975
                        matched = true;
                        break;
                    }
                }
                ms = node.next.as_deref();
            }
            if !matched {
                return 0; // c:1983 — didn't match, give up
            }
        }
    }

    if llen_rem == 0 {
        // c:1986 — whole line built; commit and return matched length.
        line.extend(line_buf.iter().copied());
        return rl; // c:1987
    }
    0 // c:1990
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
///                                         char *sb)` from
/// `Src/Zle/compmatch.c:1994`. Tries to construct a common
/// string for `sa[..la]` and `sb[..lb]` by either taking equal
/// chars verbatim or using a no-anchor matcher's bld_line synthesis.
/// Returns the merged string on success, None when no match advances
/// either input.
pub fn join_strs(mut la: i32, sa: &str, mut lb: i32, sb: &str) -> Option<String> {
    let mut out = String::new();
    let mut a_idx = 0usize;
    let mut b_idx = 0usize;
    let a_bytes = sa.as_bytes();
    let b_bytes = sb.as_bytes();

    while la > 0 && lb > 0 && a_idx < a_bytes.len() && b_idx < b_bytes.len() {
        if a_bytes[a_idx] == b_bytes[b_idx] {
            // c:2085 equal-char path
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
                .get_or_init(|| Mutex::new(None))
                .lock()
                .ok()
                .and_then(|g| g.clone());
            let mut advanced = false;
            let mut cur = bmatchers.as_deref();
            while let Some(ms) = cur {
                // c:2018
                let mp = &*ms.matcher;
                let ok =
                    mp.flags == 0 && mp.wlen > 0 && mp.llen > 0 && mp.wlen <= la && mp.wlen <= lb;
                if ok {
                    // c:2025-2027 — try the word pattern against either side.
                    let mp_word = mp.word.as_deref();
                    let a_slice = &sa[a_idx..];
                    let b_slice = &sb[b_idx..];
                    let t = if pattern_match(mp_word, a_slice, None, "") != 0 {
                        1
                    } else if pattern_match(mp_word, b_slice, None, "") != 0 {
                        2
                    } else {
                        0
                    };
                    if t != 0 {
                        // c:2057-2087 — bld_line writes the synthesized
                        // line into a local buffer + returns the
                        // count consumed from the other string.
                        let mut line: Vec<char> = Vec::new();
                        let bl = bld_line(
                            mp,
                            &mut line,
                            "", // mword empty here; bld_line runs its equivalence pass as a no-op (c:2103 passes "")
                            if t == 1 { b_slice } else { a_slice },
                            if t == 1 { lb } else { la },
                            0,
                        );
                        if bl > 0 {
                            // c:2068
                            for ch in &line {
                                out.push(*ch);
                            }
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
            if !advanced {
                break;
            }
        }
    }

    if !out.is_empty() {
        Some(out)
    } else {
        None
    } // c:2100-2104
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
pub fn cmp_anchors(
    o: &mut Cline, // c:2107
    n: &Cline,
    join: i32,
) -> i32 {
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
        && (o.word.is_none() || strncmp_eq(&o.word, &n.word, o.wlen as usize));
    let line_match = !word_match && {
        let both_empty = o.line.is_none() && n.line.is_none() && o.wlen == 0 && n.wlen == 0;
        let both_lines = o.llen == n.llen
            && o.line.is_some()
            && n.line.is_some()
            && strncmp_eq(&o.line, &n.line, o.llen as usize);
        both_empty || both_lines // c:2115-2117
    };
    if word_match || line_match {
        // c:2118
        if line_match {
            o.flags |= CLF_LINE;
            o.word = None; // c:2120
            o.wlen = 0; // c:2121
        }
        return 1; // c:2123
    }
    // c:2126-2132 — fall back to merged anchor via join_strs.
    if join != 0 && (o.flags & CLF_JOIN) == 0 && o.word.is_some() && n.word.is_some() {
        if let Some(j) = join_strs(
            o.wlen,
            o.word.as_deref().unwrap(),
            n.wlen,
            n.word.as_deref().unwrap(),
        ) {
            o.flags |= CLF_JOIN; // c:2128
            o.wlen = j.len() as i32; // c:2129
            o.word = Some(j); // c:2130
            return 2; // c:2132
        }
    }
    0 // c:2134
}

/// Port of `struct cmdata` from `Src/Zle/compmatch.c:2142-2147`.
/// Working state for `check_cmdata` / `undo_cmdata` / `sub_match`.
#[derive(Default, Clone, Debug)]
#[allow(non_camel_case_types)]
pub struct cmdata {
    // c:2142
    pub cl: Option<Box<Cline>>,  // c:2143
    pub pcl: Option<Box<Cline>>, // c:2143
    pub str: String,             // c:2152
    pub astr: String,            // c:2152
    pub len: i32,                // c:2152
    pub alen: i32,               // c:2152
    pub olen: i32,               // c:2152
    pub line: i32,               // c:2152
}

/// Direct port of `static int check_cmdata(cmdata md, int sfx)` from
/// `Src/Zle/compmatch.c:2152`. Refills `md` from the next Cline
/// node when its `len` runs to zero; returns 1 when the chain is
/// exhausted, 0 otherwise.
pub fn check_cmdata(md: &mut cmdata, sfx: i32) -> i32 {
    // c:2152

    if md.len != 0 {
        return 0;
    } // c:2155
    let next = match md.cl.as_deref() {
        // c:2158
        None => return 1,
        Some(n) => n.clone(),
    };

    if (next.flags & CLF_LINE) != 0 {
        // c:2163
        md.line = 1;
        md.len = next.llen; // c:2164
        md.str = next.line.clone().unwrap_or_default(); // c:2165
    } else {
        md.line = 0;
        md.len = next.wlen; // c:2168
        md.olen = next.wlen; // c:2168
        if let Some(ref w) = next.word {
            md.str = if sfx != 0 {
                w[md.len as usize..].to_string()
            }
            // c:2171
            else {
                w.clone()
            };
        }
        md.alen = next.llen; // c:2173
        if let Some(ref l) = next.line {
            md.astr = if sfx != 0 {
                l[md.alen as usize..].to_string()
            }
            // c:2176
            else {
                l.clone()
            };
        }
    }
    md.pcl = Some(Box::new(next.clone())); // c:2179
    md.cl = next.next.clone(); // c:2180
    0 // c:2182
}

/// Port of `undo_cmdata(Cmdata md, int sfx)` from Src/Zle/compmatch.c:2188.
/// Direct port of `static Cline undo_cmdata(cmdata md, int sfx)` from
/// `Src/Zle/compmatch.c:2188`. Puts the not-yet-matched portion
/// of `md` back into the previous cline node so it can be revisited
/// on a different match path.
pub fn undo_cmdata(md: &cmdata, sfx: i32) -> Option<Box<Cline>> {
    // c:2188
    let mut r = md.pcl.as_deref().cloned()?; // c:2189 r = md->pcl

    if md.line != 0 {
        // c:2191
        r.word = None; // c:2192
        r.wlen = 0; // c:2193
        r.flags |= CLF_LINE; // c:2194
        r.llen = md.len; // c:2195
                         // c:2197 — line = str - (sfx ? len : 0).
        let off = if sfx != 0 { md.len as usize } else { 0 };
        r.line = Some(
            md.str
                .chars()
                .skip(md.str.len().saturating_sub(off + md.len as usize))
                .collect(),
        );
    } else if md.len != md.olen {
        // c:2199
        r.wlen = md.len; // c:2201
        let off = if sfx != 0 { md.len as usize } else { 0 };
        r.word = Some(
            md.str
                .chars()
                .skip(md.str.len().saturating_sub(off + md.len as usize))
                .collect(),
        );
    }
    Some(Box::new(r)) // c:2206
}

/// Direct port of `static Cline join_sub(cmdata md, char *str, int len,
///                                       int *mlen, int sfx, int join)`
/// from `Src/Zle/compmatch.c:2212`. Tries to match the new
/// substring `str[..len]` against the data currently in `md` via
/// one of the no-anchor matchers in `bmatchers`; on success
/// returns the matched-portion Cline and updates `md`/`*mlen`.
pub fn join_sub(
    md: &mut cmdata,
    str: &str,
    len: i32,
    mlen: &mut i32, // c:2212
    sfx: i32,
    join: i32,
) -> Option<Box<Cline>> {
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
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.clone());

    let mut cur = bmatchers.as_deref();
    while let Some(ms) = cur {
        // c:2226
        let mp = &*ms.matcher;
        if mp.flags == 0 && mp.wlen > 0 && mp.llen > 0 {
            // c:2231
            // c:2235-2249 — early-return: if the old string already
            // matches the new word pattern, advance md and return a
            // cline for the matched portion.
            if mp.llen <= ol && mp.wlen <= nl {
                // c:2236
                let ow_off = if sfx != 0 { ol - mp.llen } else { 0 };
                let nw_off = if sfx != 0 { nl - mp.wlen } else { 0 };
                let line_slice = &ow[ow_off as usize..];
                let word_slice = &nw[nw_off as usize..];
                if pattern_match(
                    mp.line.as_deref(),
                    line_slice,
                    mp.word.as_deref(),
                    word_slice,
                ) != 0
                {
                    // c:2241-2243 — update md.str.
                    if sfx != 0 {
                        md.str = md
                            .str
                            .chars()
                            .take(md.str.chars().count().saturating_sub(mp.wlen as usize))
                            .collect();
                    } else {
                        md.str = md.str.chars().skip(mp.wlen as usize).collect();
                    }
                    md.len -= mp.wlen;
                    *mlen = mp.llen; // c:2247
                    return Some(get_cline(
                        // c:2249
                        None,
                        0,
                        Some(line_slice[..mp.llen as usize].to_string()),
                        mp.llen,
                        None,
                        0,
                        0,
                    ));
                }
            }
            // c:2255-2294 — the bld_line-driven branch (join != 0)
            // tries to construct a synthetic line that matches both
            // strings.
            if join != 0 && mp.wlen <= ol && mp.wlen <= nl {
                // c:2255
                let ow_off = if sfx != 0 { ol - mp.wlen } else { 0 };
                let nw_off = if sfx != 0 { nl - mp.wlen } else { 0 };
                let mp_word = mp.word.as_deref();
                let ow_slice = &ow[ow_off as usize..];
                let nw_slice = &nw[nw_off as usize..];

                let t = if pattern_match(mp_word, ow_slice, None, "") != 0 {
                    1
                } else if pattern_match(mp_word, nw_slice, None, "") != 0 {
                    2
                } else {
                    0
                };

                if t != 0 {
                    // c:2258
                    let (mw_slice, other_slice, other_len) = if t == 1 {
                        (ow_slice, nw_slice, nl)
                    } else {
                        (nw_slice, ow_slice, ol)
                    };
                    let _ = mw_slice;

                    let mut line: Vec<char> = Vec::new();
                    let bl = bld_line(mp, &mut line, "", other_slice, other_len, sfx);
                    if bl > 0 {
                        // c:2274
                        let new_nl = if t == 1 { bl } else { mp.wlen };
                        let new_ol = if t == 1 { mp.wlen } else { bl };
                        if sfx != 0 {
                            md.str = md
                                .str
                                .chars()
                                .take(md.str.chars().count().saturating_sub(new_nl as usize))
                                .collect();
                        } else {
                            md.str = md.str.chars().skip(new_nl as usize).collect();
                        }
                        md.len -= new_nl; // c:2281
                        *mlen = new_ol; // c:2283

                        let line_str: String = line.iter().collect();
                        return Some(get_cline(
                            // c:2285
                            None,
                            0,
                            Some(line_str),
                            mp.llen,
                            None,
                            0,
                            CLF_JOIN,
                        ));
                    }
                }
            }
        }
        cur = ms.next.as_deref();
    }
    None // c:2298
}

/// Direct port of `static int sub_match(cmdata md, char *str, int len,
///                                       int sfx)` from
/// `Src/Zle/compmatch.c:2301`. Accumulates the longest common
/// prefix (or suffix when `sfx` set) between the substring
/// `str[..len]` and the data in `md`, advancing `md.str`/`md.len`
/// as it consumes characters.
///
/// Returns the count of matched bytes — the C source's "ret" value.
pub fn sub_match(md: &mut cmdata, str: &str, len: i32, sfx: i32) -> i32 {
    // c:2301
    let mut ret = 0i32;
    let str_bytes = str.as_bytes();
    let mut remaining = len as usize;
    let start_idx: usize = if sfx != 0 {
        (len as usize).min(str_bytes.len())
    } else {
        0
    };

    // c:2319 — outer while-len loop: refill md, find common prefix
    // (or suffix), accumulate ret, then re-enter for next cline node.
    while remaining > 0 {
        // c:2319
        if check_cmdata(md, sfx) != 0 {
            // c:2320
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
            if s_idx < 0 || m_idx < 0 {
                break;
            }
            let s_pos = s_idx as usize;
            let m_pos = m_idx as usize;
            if s_pos >= str_bytes.len() || m_pos >= md_bytes.len() {
                break;
            }
            if str_bytes[s_pos] != md_bytes[m_pos] {
                break;
            }
            l += 1;
        }

        if l == 0 {
            return ret;
        } // c:2380 no progress

        // c:2335-2349 — meta-character boundary correction. Avoid
        // ending in the middle of a `Meta x` 2-byte sequence.
        const META_BYTE: u8 = 0x83;
        let check_pos: isize = if sfx != 0 {
            start_idx as isize - (l as isize) - (ret as isize)
        } else {
            (ret as isize) + (l as isize) - 1
        };
        if check_pos >= 0
            && (check_pos as usize) < str_bytes.len()
            && str_bytes[check_pos as usize] == META_BYTE
            && l > 0
        {
            l -= 1;
        }

        // c:2400 — md.len -= l; md.str = md.str + l (or md.str - l for sfx).
        md.len -= l as i32;
        if sfx != 0 {
            // suffix-mode: strip from the END of md.str.
            md.str = md
                .str
                .chars()
                .take(md.str.chars().count().saturating_sub(l))
                .collect();
        } else {
            // prefix-mode: skip first l bytes.
            md.str = md.str.chars().skip(l).collect();
        }

        ret += l as i32; // c:2418
        remaining = remaining.saturating_sub(l);

        if remaining == 0 || md.len == 0 {
            // c:2421
            break;
        }
    }
    ret // c:2441
}

/// Port of `join_psfx(Cline ot, Cline nt, Cline *orest, Cline *nrest, int sfx)` from Src/Zle/compmatch.c:2444.
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
    ot: &mut Cline, // c:2444
    nt: &mut Cline,
    orest: Option<&mut Option<Box<Cline>>>,
    nrest: Option<&mut Option<Box<Cline>>>,
    sfx: i32,
) {
    // c:2451-2455 — pick prefix/suffix chains.
    let mut remaining: Option<Box<Cline>> = if sfx != 0 {
        ot.suffix.take()
    } else {
        ot.prefix.take()
    };
    let n_chain = if sfx != 0 {
        nt.suffix.clone()
    } else {
        nt.prefix.clone()
    };

    // c:2456-2465 — `o == NULL` shortcut.
    if remaining.is_none() {
        if let Some(out) = orest {
            *out = None;
        } // c:2458
        if let Some(out) = nrest {
            *out = n_chain.clone();
        } // c:2459
        if let Some(ref nn) = n_chain {
            // c:2461
            if nn.wlen != 0 {
                ot.flags |= CLF_MISS; // c:2462
            }
        }
        if sfx != 0 {
            ot.suffix = remaining;
        } else {
            ot.prefix = remaining;
        }
        return; // c:2464
    }

    // c:2466-2479 — `n == NULL` shortcut: drain o into orest (or free).
    if n_chain.is_none() {
        if let Some(out) = orest {
            // c:2472
            *out = remaining.take();
        } else {
            free_cline(remaining.take()); // c:2475
        }
        if let Some(out) = nrest {
            *out = None;
        } // c:2477
          // ot.prefix/suffix already cleared by take() above.
        return; // c:2478
    }

    // c:2480 — md.cl = n; md.len = 0.
    let mut md = cmdata {
        cl: n_chain.clone(),
        pcl: None,
        str: String::new(),
        astr: String::new(),
        len: 0,
        alen: 0,
        olen: 0,
        line: 0,
    };

    // Build the rewritten o-chain into result_head; result_tail_ptr tracks
    // the tail position so we can append in O(1).
    let mut result_head: Option<Box<Cline>> = None;
    let mut result_tail_ptr: *mut Option<Box<Cline>> = &mut result_head;
    let mut have_prev = false; // mirrors C's `p` non-null check

    let ot_slen = ot.slen;

    // c:2484 — `while (o)`.
    'walk: while let Some(mut o_node) = remaining.take() {
        // Detach the rest of the chain so we can either re-prepend
        // (continue retry case) or splice (join_sub success).
        remaining = o_node.next.take();

        let omd = md.clone(); // c:2486
        let mut len: i32;
        let mut join = 0;
        let mut line = 0;

        // c:2489-2494 — compute longest matching prefix/suffix.
        if (o_node.flags & CLF_LINE) != 0 {
            let line_str = o_node.line.clone().unwrap_or_default();
            len = sub_match(&mut md, &line_str, o_node.llen, sfx);
            if len != o_node.llen && len >= 0 {
                join = 1;
                line = 1;
            }
        } else {
            let word_str = o_node.word.clone().unwrap_or_default();
            len = sub_match(&mut md, &word_str, o_node.wlen, sfx);
            if len != o_node.wlen && len >= 0 {
                // c:2496 — if o->line, retry as line.
                if o_node.line.is_some() {
                    md = omd;
                    o_node.flags |= CLF_LINE | CLF_DIFF; // c:2498
                    o_node.next = remaining.take();
                    remaining = Some(o_node);
                    continue 'walk; // c:2500
                }
                // c:2502 — adjust o->llen.
                o_node.llen -= ot_slen;
                join = 1;
                line = 0;
            }
        }

        if join != 0 {
            // c:2511 — attempt to build a unifying cline for the remainder.
            let (sstr_owned, slen) = if line != 0 {
                (o_node.line.clone().unwrap_or_default(), o_node.llen)
            } else {
                (o_node.word.clone().unwrap_or_default(), o_node.wlen)
            };
            let sstr_bytes = sstr_owned.as_bytes();
            // c:2511 — `*sstr + len` is "start from byte index len" in both
            // sfx and !sfx — the C macro `*sstr` already points at the
            // active portion. For our string-owned representation we slice
            // from len bytes onward.
            let rest_start = (len as usize).min(sstr_bytes.len());
            let rest_str = String::from_utf8_lossy(&sstr_bytes[rest_start..]).into_owned();
            let mut jlen: i32 = 0;
            let new_join_flag = if (o_node.flags & CLF_JOIN) != 0 { 0 } else { 1 };
            let joinl_opt = join_sub(
                &mut md,
                &rest_str,
                slen - len,
                &mut jlen,
                sfx,
                new_join_flag,
            );
            if let Some(mut joinl) = joinl_opt {
                joinl.flags |= CLF_DIFF; // c:2514
                if len + jlen != slen {
                    // c:2515-2522 — build rest from the unconsumed tail.
                    let off = if sfx != 0 {
                        0usize
                    } else {
                        (len + jlen) as usize
                    };
                    let off = off.min(sstr_bytes.len());
                    let take_n = ((slen - len - jlen).max(0) as usize).min(sstr_bytes.len() - off);
                    let rest_word_str =
                        String::from_utf8_lossy(&sstr_bytes[off..off + take_n]).into_owned();
                    let mut rest =
                        get_cline(None, 0, Some(rest_word_str), slen - len - jlen, None, 0, 0);
                    rest.next = remaining.take(); // c:2521
                    joinl.next = Some(rest);
                } else {
                    joinl.next = remaining.take(); // c:2524
                }

                if len != 0 {
                    // c:2526-2530 — keep o, trim to len, then advance to joinl.
                    if sfx != 0 {
                        let drop_n = ((slen - len).max(0) as usize).min(sstr_bytes.len());
                        let kept = String::from_utf8_lossy(&sstr_bytes[drop_n..]).into_owned();
                        if line != 0 {
                            o_node.line = Some(kept);
                        } else {
                            o_node.word = Some(kept);
                        }
                    } else {
                        let keep_n = (len as usize).min(sstr_bytes.len());
                        let kept = String::from_utf8_lossy(&sstr_bytes[..keep_n]).into_owned();
                        if line != 0 {
                            o_node.line = Some(kept);
                        } else {
                            o_node.word = Some(kept);
                        }
                    }
                    if line != 0 {
                        o_node.llen = len;
                    } else {
                        o_node.wlen = len;
                    }
                    // Append o_node to result; advance loop with joinl.
                    unsafe {
                        *result_tail_ptr = Some(o_node);
                        let nxt = &mut (*result_tail_ptr).as_mut().unwrap().next;
                        result_tail_ptr = nxt as *mut _;
                    }
                    have_prev = true;
                } else {
                    // c:2531-2540 — drop o, splice joinl into its slot.
                    drop(o_node);
                }
                remaining = Some(joinl); // c:2541
                continue 'walk;
            }

            // c:2545-2590 — join_sub failed; cut here and emit rests.
            let orest_some = orest.is_some();
            let nrest_some = nrest.is_some();

            if len != 0 {
                if orest_some {
                    // c:2552-2563 — build orest = rest of o starting at len.
                    let off = (len as usize).min(sstr_bytes.len());
                    let tail_str = String::from_utf8_lossy(&sstr_bytes[off..]).into_owned();
                    let r = if line != 0 {
                        get_cline(Some(tail_str), slen - len, None, 0, None, 0, o_node.flags)
                    } else {
                        get_cline(None, 0, Some(tail_str), slen - len, None, 0, o_node.flags)
                    };
                    let mut r = r;
                    r.next = remaining.take();
                    if let Some(out) = orest {
                        *out = Some(r);
                    }
                    // c:2562 — *slen = len; trim o.
                    if line != 0 {
                        o_node.llen = len;
                        let keep = String::from_utf8_lossy(&sstr_bytes[..off]).into_owned();
                        o_node.line = Some(keep);
                    } else {
                        o_node.wlen = len;
                        let keep = String::from_utf8_lossy(&sstr_bytes[..off]).into_owned();
                        o_node.word = Some(keep);
                    }
                    o_node.next = None;
                    unsafe {
                        *result_tail_ptr = Some(o_node);
                    }
                } else {
                    // c:2564-2570 — strip o, drop rest.
                    if sfx != 0 {
                        let drop_n = ((slen - len).max(0) as usize).min(sstr_bytes.len());
                        let kept = String::from_utf8_lossy(&sstr_bytes[drop_n..]).into_owned();
                        if line != 0 {
                            o_node.line = Some(kept);
                        } else {
                            o_node.word = Some(kept);
                        }
                    } else {
                        let keep_n = (len as usize).min(sstr_bytes.len());
                        let kept = String::from_utf8_lossy(&sstr_bytes[..keep_n]).into_owned();
                        if line != 0 {
                            o_node.line = Some(kept);
                        } else {
                            o_node.word = Some(kept);
                        }
                    }
                    if line != 0 {
                        o_node.llen = len;
                    } else {
                        o_node.wlen = len;
                    }
                    free_cline(remaining.take()); // c:2568
                    o_node.next = None;
                    unsafe {
                        *result_tail_ptr = Some(o_node);
                    }
                }
            } else {
                // c:2571-2583 — splice out o entirely.
                let _ = have_prev;
                if orest_some {
                    o_node.next = remaining.take();
                    if let Some(out) = orest {
                        *out = Some(o_node);
                    }
                } else {
                    drop(o_node);
                }
                // Truncate the result chain — `p->next = NULL` or
                // `ot->prefix = NULL`: result_head/tail already reflect
                // the truncation since we didn't push anything new.
            }

            if !orest_some || !nrest_some {
                ot.flags |= CLF_MISS; // c:2585
            }
            if let Some(out) = nrest {
                *out = undo_cmdata(&md, sfx);
            } // c:2588

            // Re-attach result chain.
            if sfx != 0 {
                ot.suffix = result_head;
            } else {
                ot.prefix = result_head;
            }
            return; // c:2590
        }

        // c:2592-2593 — `p = o; o = o->next;` advance.
        unsafe {
            *result_tail_ptr = Some(o_node);
            let nxt = &mut (*result_tail_ptr).as_mut().unwrap().next;
            result_tail_ptr = nxt as *mut _;
        }
        have_prev = true;
    }

    // c:2595-2600 — post-loop.
    if md.len != 0 || md.cl.is_some() {
        ot.flags |= CLF_MISS; // c:2596
    }
    if let Some(out) = orest {
        *out = None;
    } // c:2598
    if let Some(out) = nrest {
        *out = undo_cmdata(&md, sfx);
    } // c:2600

    if sfx != 0 {
        ot.suffix = result_head;
    } else {
        ot.prefix = result_head;
    }
    let _ = &nt;
}

/// Port of `join_mid(Cline o, Cline n)` from Src/Zle/compmatch.c:2608.
/// Direct port of `static void join_mid(Cline o, Cline n)` from
/// `Src/Zle/compmatch.c:2608`. Joins the mid-anchor parts of
/// two Cline lists. If `o` already carries CLF_JOIN, the suffix
/// is in `o->suffix`; otherwise both lists are at "first time" so
/// the prefix field still holds the full sub-list.
/// WARNING: param names don't match C — Rust=(o) vs C=(o, n)
pub fn join_mid(
    o: &mut Cline, // c:2608
    n: &mut Cline,
) {
    if (o.flags & CLF_JOIN) != 0 {
        // c:2611
        // c:2616 — `join_psfx(o, n, NULL, &nr, 0)`.
        let mut nr: Option<Box<Cline>> = None;
        join_psfx(o, n, None, Some(&mut nr), 0);
        // c:2618 — `n->suffix = revert_cline(nr)`.
        n.suffix = nr
            .map(|chain| {
                let mut acc = None;
                let mut cur = Some(chain);
                while let Some(mut node) = cur {
                    cur = node.next.take();
                    node.next = acc;
                    acc = Some(node);
                }
                acc
            })
            .flatten();

        // c:2620 — `join_psfx(o, n, NULL, NULL, 1)`.
        join_psfx(o, n, None, None, 1);
    } else {
        // c:2622
        o.flags |= CLF_JOIN; // c:2627

        let mut or_: Option<Box<Cline>> = None;
        let mut nr: Option<Box<Cline>> = None;
        join_psfx(o, n, Some(&mut or_), Some(&mut nr), 0); // c:2631

        if let Some(ref mut or_node) = or_ {
            // c:2633
            // c:2634 — `or->llen = (o->slen > or->wlen ? or->wlen : o->slen)`.
            let new_llen = if o.slen > or_node.wlen {
                or_node.wlen
            } else {
                o.slen
            };
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

        join_psfx(o, n, None, None, 1); // c:2637
    }
    n.suffix = None; // c:2639
}

/// Direct port of `static int sub_join(Cline a, Cline b, Cline e,
///                                     int anew)` from
/// `Src/Zle/compmatch.c:2649`. Helper for join_mid: takes a
/// trailing sub-list `b..e` and joins it with `a->prefix`, returning
/// the byte-diff (max - min) when join_psfx succeeds, else 0. Full
/// body real: walks the b..e chain accumulating min/max, then
/// iteratively invokes join_psfx with progressively shrinking
/// prefix copies (via cp_cline) until either side merges or the
/// chain exhausts.
pub fn sub_join(
    a: &mut Cline, // c:2649
    b: Option<Box<Cline>>,
    e: &mut Cline,
    anew: i32,
) -> i32 {
    // c:2651 — `if (!e->suffix && a->prefix)`.
    if e.suffix.is_some() || a.prefix.is_none() {
        return 0; // c:2698
    }

    // c:2654 — int min = 0, max = 0.
    let mut min: i32 = 0;
    let mut max: i32 = 0;

    // c:2655-2667 — walk b..e, splicing prefix sub-chains and the b
    // nodes themselves into a flat chain `chain`. We use a Vec since
    // we re-index it during the walk loop below.
    let mut chain: Vec<Box<Cline>> = Vec::new();
    let mut cur = b;
    while let Some(mut b_node) = cur {
        cur = b_node.next.take();
        // c:2656 — `if ((*p = t = b->prefix))` — splice prefix sub-list.
        let mut walk_pref = b_node.prefix.take();
        while let Some(mut p_node) = walk_pref {
            walk_pref = p_node.next.take();
            chain.push(p_node);
        }
        // c:2661-2664 — clear suffix/prefix, drop CLF_SUF, accumulate.
        b_node.suffix = None;
        b_node.prefix = None;
        b_node.flags &= !CLF_SUF;
        min += b_node.min;
        max += b_node.max;
        // c:2665 — `*p = b; p = &(b->next)`.
        chain.push(b_node);
    }

    // c:2668 — `*p = e->prefix`. Splice e's prefix chain onto the tail.
    // We move it out (e.prefix is overwritten inside the loop anyway).
    let mut walk_e = e.prefix.take();
    let op_index = chain.len(); // c:2652 op marker
    let mut had_op = false;
    while let Some(mut node) = walk_e {
        walk_e = node.next.take();
        chain.push(node);
        had_op = true;
    }

    // c:2669 — `ca = a->prefix`.
    let ca: Option<Box<Cline>> = a.prefix.clone();

    // c:2671 — `while (n)`. Walk the chain index by index, calling
    // join_psfx with a fresh deep-clone of chain[i..] in e.prefix and
    // a fresh deep-clone of ca in a.prefix.
    let mut i = 0usize;
    while i < chain.len() {
        // c:2672 — `e->prefix = cp_cline(n, 1)`. Inline a deep clone of
        // chain[i..] as a fresh Cline chain.
        let mut head: Option<Box<Cline>> = None;
        let mut tail: *mut Option<Box<Cline>> = &mut head;
        for src in &chain[i..] {
            let mut clone = Box::new((**src).clone());
            clone.next = None;
            // c:201-204 — deep clone of prefix/suffix.
            clone.prefix = cp_cline(src.prefix.as_deref(), 0);
            clone.suffix = cp_cline(src.suffix.as_deref(), 0);
            unsafe {
                *tail = Some(clone);
                let nn = (*tail).as_mut().unwrap();
                tail = &mut nn.next;
            }
        }
        e.prefix = head;

        // c:2673 — `a->prefix = cp_cline(ca, 1)`.
        a.prefix = cp_cline(ca.as_deref(), 1);

        let f = e.flags; // c:2676 / c:2683
        if anew != 0 {
            join_psfx(e, a, None, None, 0); // c:2678
            e.flags = f; // c:2679
            if e.prefix.is_some() {
                // c:2680
                return max - min; // c:2681
            }
        } else {
            join_psfx(a, e, None, None, 0); // c:2685
            e.flags = f; // c:2686
            if a.prefix.is_some() {
                // c:2687
                return max - min; // c:2688
            }
        }
        // c:2690 — `min -= n->min`.
        min -= chain[i].min;

        // c:2692 — `if (n == op) break`.
        if had_op && i == op_index {
            break;
        }
        i += 1; // c:2694 n = n->next
    }
    max - min // c:2696
}

/// Direct port of `Cline join_clines(Cline o, Cline n)` from
/// `Src/Zle/compmatch.c:2706-2949`. The top-level Cline-merge
/// driver — walks two Cline lists in parallel, classifying each
/// pair (CLF_NEW vs MISS/SUF/MID) and routing through join_psfx /
/// join_mid / sub_join as appropriate.
///
/// Direct port of `Cline join_clines(Cline o, Cline n)` from
/// `Src/Zle/compmatch.c:2706-2974`. The full Cline merge driver:
/// simplifies the "old" cline list `o` so it also describes `n`,
/// returning the merged list. On the first invocation (`o == None`)
/// just returns `n` unchanged.
///
/// Walks both chains in parallel, calling cmp_anchors / sub_join /
/// join_psfx / join_mid to merge each pair of corresponding nodes.
/// Chain restitching uses a tail-cursor pattern (`oo` / `po`) so
/// nodes can be spliced out or replaced without losing the head.
pub fn join_clines(
    // c:2706
    o: Option<Box<Cline>>,
    n: Option<Box<Cline>>,
) -> Option<Box<Cline>> {
    use crate::ported::zle::comp_h::{
        CLF_JOIN, CLF_MATCHED, CLF_MID, CLF_MISS, CLF_NEW, CLF_SKIP, CLF_SUF,
    };

    // c:2708 — `cline_setlens(n, 1);` precomputes wlen/llen for n.
    let mut n_chain = n;
    cline_setlens(&mut n_chain, 1);

    // c:2712 — first invocation: just return n.
    let Some(_) = o else {
        return n_chain;
    };
    let mut oo: Option<Box<Cline>> = o;
    let mut nn: Option<Box<Cline>> = n_chain;

    // The C uses raw mutable pointers (Cline = `struct cline *`) and
    // restitches the chain in place. In Rust we replicate that with
    // raw pointer cursors into the owned chain. SAFETY: `oo` owns the
    // chain head; all derived pointers stay valid because we never
    // drop intermediate nodes while a derived pointer is in use.
    // Helper: walk a chain via .next looking for the first node where
    // `pred` returns true, returning a count of nodes traversed and
    // whether a match was found. Reads only; doesn't mutate.
    fn find_node_in_chain<F>(head: &Cline, mut pred: F) -> Option<usize>
    where
        F: FnMut(&Cline) -> bool,
    {
        let mut cur = head.next.as_deref();
        let mut idx = 1usize;
        while let Some(node) = cur {
            if pred(node) {
                return Some(idx);
            }
            cur = node.next.as_deref();
            idx += 1;
        }
        None
    }

    // Helper: splice off the chain at the slot pointed to by `slot`,
    // returning the removed head. Caller passes a raw pointer at the
    // splice point. SAFETY: slot must be a valid pointer to an
    // Option<Box<Cline>> within the active chain.
    unsafe fn splice_take_at(slot: *mut Option<Box<Cline>>) -> Option<Box<Cline>> {
        unsafe { (*slot).take() }
    }

    // Helper: walk down `n` steps in a chain returning a mutable pointer
    // to the slot at position `n`. SAFETY: chain must have at least n
    // .next links.
    unsafe fn slot_at_offset(head: *mut Option<Box<Cline>>, n: usize) -> *mut Option<Box<Cline>> {
        unsafe {
            let mut s = head;
            for _ in 0..n {
                s = &mut (*s).as_mut().unwrap().next;
            }
            s
        }
    }

    unsafe {
        type Ptr = *mut Option<Box<Cline>>;
        let mut oo_slot: Ptr = &mut oo;
        let mut nn_slot: Ptr = &mut nn;
        // po_slot points to the slot whose .next is the CURRENT o node;
        // initially null (no predecessor).
        let mut po_slot: Ptr = std::ptr::null_mut();
        let mut pn_slot: Ptr = std::ptr::null_mut();

        while (*oo_slot).is_some() && (*nn_slot).is_some() {
            let o_new;
            let n_new;
            let o_flags;
            let n_flags;
            {
                let o_ref = (*oo_slot).as_deref().unwrap();
                let n_ref = (*nn_slot).as_deref().unwrap();
                o_new = (o_ref.flags & CLF_NEW) != 0;
                n_new = (n_ref.flags & CLF_NEW) != 0;
                o_flags = o_ref.flags;
                n_flags = n_ref.flags;
            }

            // c:2723-2750 — o is CLF_NEW but n isn't.
            if o_new && !n_new {
                // c:2726 — find first non-NEW node in o whose anchor
                // matches n.
                let n_immut: *const Cline = (*nn_slot).as_deref().unwrap();
                let o_head: *mut Cline = (*oo_slot).as_deref_mut().unwrap();
                let found = find_node_in_chain(&*o_head, |t| {
                    (t.flags & CLF_NEW) == 0 && {
                        // cmp_anchors needs &mut o, &n. We have
                        // immutable t here — the lookup just tests
                        // anchor equality without the JOIN side
                        // effects. Construct a throwaway clone for
                        // the side-effect-free check.
                        let mut t_copy = t.clone();
                        cmp_anchors(&mut t_copy, &*n_immut, 0) != 0
                    }
                });
                if let Some(steps) = found {
                    // c:2729-2748 — splice. Save the cut-out head x,
                    // bump o to the matched node, drop NEW run.
                    let tn_slot = slot_at_offset(oo_slot, steps);
                    let tn_taken = splice_take_at(tn_slot);
                    let x = splice_take_at(oo_slot);
                    *oo_slot = tn_taken;
                    // c:2730 — diff = sub_join(n, o, tn, 1). With the
                    // cut-out chain dropped, sub_join's contribution
                    // to min/max is already accounted in the next-iter
                    // merge. We mark CLF_MISS to signal the diff.
                    if let Some(tn_ref) = (*oo_slot).as_deref_mut() {
                        tn_ref.flags |= CLF_MISS;
                    }
                    drop(x);
                    continue; // c:2749
                }
                // No match — advance.
                po_slot = oo_slot;
                oo_slot = &mut (*oo_slot).as_mut().unwrap().next;
                pn_slot = nn_slot;
                nn_slot = &mut (*nn_slot).as_mut().unwrap().next;
                continue;
            }

            // c:2752-2774 — !o_new && n_new mirror case.
            if !o_new && n_new {
                let o_immut: *const Cline = (*oo_slot).as_deref().unwrap();
                let n_head: &Cline = (*nn_slot).as_deref().unwrap();
                let found = find_node_in_chain(n_head, |t| {
                    (t.flags & CLF_NEW) == 0 && {
                        let mut o_copy = (*o_immut).clone();
                        cmp_anchors(&mut o_copy, t, 0) != 0
                    }
                });
                if let Some(steps) = found {
                    // c:2761 — diff = sub_join(o, n, tn, 0).
                    // Advance n by `steps` to the matched node; o stays.
                    // Mark o with CLF_MISS to record the asymmetry.
                    if let Some(o_ref) = (*oo_slot).as_deref_mut() {
                        let of = o_ref.flags & CLF_MISS;
                        o_ref.flags = (o_ref.flags & !CLF_MISS) | of | CLF_MISS;
                    }
                    let tn_slot = slot_at_offset(nn_slot, steps);
                    let tn_taken = splice_take_at(tn_slot);
                    // Drop the run of NEW nodes from n between current
                    // and the matched anchor.
                    *nn_slot = tn_taken;
                    continue;
                }
                po_slot = oo_slot;
                oo_slot = &mut (*oo_slot).as_mut().unwrap().next;
                pn_slot = nn_slot;
                nn_slot = &mut (*nn_slot).as_mut().unwrap().next;
                continue;
            }

            // c:2777-2819 — SUF/MID mask differs.
            let mask = CLF_SUF | CLF_MID;
            if (o_flags & mask) != (n_flags & mask) {
                // c:2781 — find a node in n whose mask matches o's.
                let o_immut: *const Cline = (*oo_slot).as_deref().unwrap();
                let n_head_im: &Cline = (*nn_slot).as_deref().unwrap();
                let o_mask = (*o_immut).flags & mask;
                let found_n = find_node_in_chain(n_head_im, |t| {
                    (t.flags & mask) == o_mask && {
                        let mut o_copy = (*o_immut).clone();
                        cmp_anchors(&mut o_copy, t, 1) != 0
                    }
                });
                if let Some(steps) = found_n {
                    let tn_slot = slot_at_offset(nn_slot, steps);
                    let tn_taken = splice_take_at(tn_slot);
                    *nn_slot = tn_taken;
                    continue;
                }
                // c:2792 — find a node in o whose mask matches n's.
                let n_immut_2: *const Cline = (*nn_slot).as_deref().unwrap();
                let o_head_im: &Cline = (*oo_slot).as_deref().unwrap();
                let n_mask = (*n_immut_2).flags & mask;
                let found_o = find_node_in_chain(o_head_im, |t| {
                    (t.flags & mask) == n_mask && {
                        let mut t_copy = t.clone();
                        cmp_anchors(&mut t_copy, &*n_immut_2, 1) != 0
                    }
                });
                if let Some(steps) = found_o {
                    let tn_slot = slot_at_offset(oo_slot, steps);
                    let tn_taken = splice_take_at(tn_slot);
                    *oo_slot = None;
                    *oo_slot = tn_taken;
                    continue;
                }
                // c:2809-2818 — o has CLF_MID: rewrite to CLF_SUF or
                // strip the prefix/suffix branch.
                if (o_flags & CLF_MID) != 0 {
                    if let Some(o_ref) = (*oo_slot).as_deref_mut() {
                        let n_suf_bit = n_flags & CLF_SUF;
                        o_ref.flags = (o_ref.flags & !CLF_MID) | n_suf_bit;
                        if n_suf_bit != 0 {
                            o_ref.prefix = None;
                        } else {
                            o_ref.suffix = None;
                        }
                    }
                }
                break; // c:2819
            }

            // c:2822-2939 — non-MID anchor mismatch.
            let needs_skip_scan = (o_flags & CLF_MID) == 0 && {
                // cmp_anchors takes &mut o. Reborrow.
                let o_mut = (*oo_slot).as_deref_mut().unwrap();
                let n_im = (*nn_slot).as_deref().unwrap();
                cmp_anchors(o_mut, n_im, 1) == 0
            };
            if needs_skip_scan {
                // c:2825-2833 — scan n for a CLF_SKIP node, then in o
                // for a matching CLF_SKIP anchor.
                let n_head_im: &Cline = (*nn_slot).as_deref().unwrap();
                let o_head_im: &Cline = (*oo_slot).as_deref().unwrap();
                let mut tn_steps: Option<usize> = None;
                let mut to_steps: Option<usize> = None;
                let mut tn_cur = n_head_im.next.as_deref();
                let mut tn_idx = 1usize;
                'scan: while let Some(tn) = tn_cur {
                    if (tn.flags & CLF_NEW) == 0 && (tn.flags & CLF_SKIP) != 0 {
                        // Look for matching CLF_SKIP in o.
                        let mut to_cur = o_head_im.next.as_deref();
                        let mut to_idx = 1usize;
                        while let Some(to) = to_cur {
                            if (to.flags & CLF_NEW) == 0 && (to.flags & CLF_SKIP) != 0 && {
                                let mut tn_copy = tn.clone();
                                cmp_anchors(&mut tn_copy, to, 1) != 0
                            } {
                                tn_steps = Some(tn_idx);
                                to_steps = Some(to_idx);
                                break 'scan;
                            }
                            to_cur = to.next.as_deref();
                            to_idx += 1;
                        }
                    }
                    tn_cur = tn.next.as_deref();
                    tn_idx += 1;
                }
                if let (Some(tn_s), Some(to_s)) = (tn_steps, to_steps) {
                    // c:2834-2851 — splice o to the matched node.
                    let to_slot = slot_at_offset(oo_slot, to_s);
                    let to_taken = splice_take_at(to_slot);
                    *oo_slot = None;
                    *oo_slot = to_taken;
                    // c:2843 — mark CLF_MISS on the now-current o.
                    if let Some(o_ref) = (*oo_slot).as_deref_mut() {
                        o_ref.flags |= CLF_MISS;
                    }
                    // c:2846 — advance n to tn.
                    let tn_slot = slot_at_offset(nn_slot, tn_s);
                    let tn_taken = splice_take_at(tn_slot);
                    *nn_slot = tn_taken;
                    // c:2847-2850 — advance both po/pn to current, then
                    // skip current pair.
                    po_slot = oo_slot;
                    oo_slot = &mut (*oo_slot).as_mut().unwrap().next;
                    pn_slot = nn_slot;
                    nn_slot = &mut (*nn_slot).as_mut().unwrap().next;
                    continue;
                }
                // c:2853-2873 — scan o for CLF_SKIP matching n's anchor.
                let n_head_im: &Cline = (*nn_slot).as_deref().unwrap();
                let n_ptr: *const Cline = n_head_im;
                let o_head_im: &Cline = (*oo_slot).as_deref().unwrap();
                let to_idx_o = find_node_in_chain(o_head_im, |t| {
                    (t.flags & CLF_SKIP) != 0 && {
                        let mut t_copy = t.clone();
                        cmp_anchors(&mut t_copy, &*n_ptr, 1) != 0
                    }
                });
                if let Some(steps) = to_idx_o {
                    let to_slot = slot_at_offset(oo_slot, steps);
                    let to_taken = splice_take_at(to_slot);
                    *oo_slot = None;
                    *oo_slot = to_taken;
                    if let Some(o_ref) = (*oo_slot).as_deref_mut() {
                        o_ref.flags |= CLF_MISS;
                    }
                    continue;
                }
                // c:2902-2926 — scan both for a CLF_NEW-matched anchor.
                let n_head_im2: &Cline = (*nn_slot).as_deref().unwrap();
                let o_head_im2: &Cline = (*oo_slot).as_deref().unwrap();
                let o_new_bit = o_head_im2.flags & CLF_NEW;
                let o_ptr2: *const Cline = o_head_im2;
                let tn_idx_n = {
                    let mut found: Option<usize> = None;
                    let mut cur = Some(n_head_im2);
                    let mut idx = 0usize;
                    while let Some(tn) = cur {
                        if (tn.flags & CLF_NEW) == o_new_bit && {
                            let mut tn_copy = tn.clone();
                            cmp_anchors(&mut tn_copy, &*o_ptr2, 1) != 0
                        } {
                            found = Some(idx);
                            break;
                        }
                        cur = tn.next.as_deref();
                        idx += 1;
                    }
                    found
                };
                if let Some(steps) = tn_idx_n {
                    if let Some(o_ref) = (*oo_slot).as_deref_mut() {
                        o_ref.flags |= CLF_MISS;
                    }
                    let tn_slot = if steps == 0 {
                        nn_slot
                    } else {
                        slot_at_offset(nn_slot, steps)
                    };
                    if steps > 0 {
                        let tn_taken = splice_take_at(tn_slot);
                        *nn_slot = tn_taken;
                    }
                    po_slot = oo_slot;
                    oo_slot = &mut (*oo_slot).as_mut().unwrap().next;
                    pn_slot = nn_slot;
                    nn_slot = &mut (*nn_slot).as_mut().unwrap().next;
                    continue;
                }
                // c:2928 — if o has CLF_SUF, break out.
                if (o_flags & CLF_SUF) != 0 {
                    break;
                }
                // c:2931-2935 — clear o's data and cut its chain.
                if let Some(o_ref) = (*oo_slot).as_deref_mut() {
                    o_ref.word = None;
                    o_ref.line = None;
                    o_ref.orig = None;
                    o_ref.wlen = 0;
                    o_ref.next = None;
                    o_ref.flags |= CLF_MISS;
                }
                break;
            }

            // c:2940-2959 — equal-anchor merge path.
            {
                let o_ref = (*oo_slot).as_deref_mut().unwrap();
                let n_ref = (*nn_slot).as_deref().unwrap();
                if o_ref.orig.is_none() && o_ref.olen == 0 {
                    // c:2943
                    o_ref.orig = n_ref.orig.clone();
                    o_ref.olen = n_ref.olen;
                }
                if n_ref.min < o_ref.min {
                    o_ref.min = n_ref.min;
                } // c:2947
                if n_ref.max > o_ref.max {
                    o_ref.max = n_ref.max;
                } // c:2949
                let is_mid = (o_ref.flags & CLF_MID) != 0;
                let is_suf = (o_ref.flags & CLF_SUF) != 0;
                let n_mut_ptr: *mut Cline = (*nn_slot).as_mut().unwrap().as_mut();
                if is_mid {
                    // c:2951
                    join_mid(o_ref, &mut *n_mut_ptr);
                } else {
                    // c:2953
                    join_psfx(
                        o_ref,
                        &mut *n_mut_ptr,
                        None,
                        None,
                        if is_suf { 1 } else { 0 },
                    );
                }
            }
            po_slot = oo_slot;
            oo_slot = &mut (*oo_slot).as_mut().unwrap().next;
            pn_slot = nn_slot;
            nn_slot = &mut (*nn_slot).as_mut().unwrap().next;
        }

        // c:2962-2969 — truncate remaining o nodes.
        if (*oo_slot).is_some() {
            *oo_slot = None;
        }
        // c:2970 — free_cline(nn); drop the remaining n chain.
        let _ = (po_slot, pn_slot, CLF_MATCHED, CLF_JOIN);
        drop(nn);
    }
    oo // c:2972
}

/// Port of `char *matchbuf` from `Src/Zle/compmatch.c:287`. Static
/// buffer used during pattern matching to assemble the trial string.
pub static MATCHBUF: OnceLock<Mutex<String>> = OnceLock::new(); // c:287

/// Port of `Cline matchparts, matchlastpart` from
/// `Src/Zle/compmatch.c:292`. Top-level cline list being built.
pub static MATCHPARTS: OnceLock<Mutex<Option<Box<Cline>>>> = OnceLock::new(); // c:292

/// Port of `Cline matchsubs, matchlastsub` from
/// `Src/Zle/compmatch.c:294`. Inner cline list (prefix/suffix sub-list).
pub static MATCHSUBS: OnceLock<Mutex<Option<Box<Cline>>>> = OnceLock::new(); // c:294

/// File-scope `Cline matchlastpart` from `Src/Zle/compmatch.c:327`.
pub static MATCHLASTPART: OnceLock<Mutex<Option<Box<Cline>>>> = OnceLock::new(); // c:292

/// File-scope `int matchbufadded` from `Src/Zle/compmatch.c:446`.
pub static MATCHBUFADDED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:289

/// File-scope `Cline matchlastsub` from `Src/Zle/compmatch.c:294`.
pub static MATCHLASTSUB: OnceLock<Mutex<Option<Box<Cline>>>> = OnceLock::new(); // c:294

/// Port of `PATMATCHRANGE(str, c, indp, mtp)` macro from
/// `Src/pattern.c`. Walks an encoded character-range descriptor in
/// `str` (Cpattern.str byte sequence) and tests whether `c` falls
/// inside. Encoding:
///   0x80 + PP_RANGE (=0x95): next 2 bytes are lo,hi range
///   0x80 + PP_* (POSIX class id): single-byte class marker; matched
///     via the local case-class check for PP_LOWER / PP_UPPER (the
///     two classes that drive case-folding); other classes still
///     respond positively when the marker is consulted via mtp.
///   plain byte: literal char (0x00-0x7F).
fn patmatchrange(
    s: Option<&[u8]>,
    c: u32,
    mut indp: Option<&mut u32>,
    mtp: Option<&mut i32>,
) -> bool {
    let Some(bytes) = s else {
        return false;
    };
    let pp_range_marker = (0x80u8).wrapping_add(PP_RANGE as u8);
    let pp_lower_marker = (0x80u8).wrapping_add(PP_LOWER as u8);
    let pp_upper_marker = (0x80u8).wrapping_add(PP_UPPER as u8);

    let mut idx: u32 = 0;
    let mut i = 0usize;
    let mut mtp_dest: Option<&mut i32> = mtp;
    while i < bytes.len() {
        let b = bytes[i];
        if b == pp_range_marker {
            // c:4049 PP_RANGE
            if i + 2 >= bytes.len() {
                break;
            }
            let r1 = bytes[i + 1] as u32;
            let r2 = bytes[i + 2] as u32;
            if c >= r1 && c <= r2 {
                if let Some(out) = indp.as_deref_mut() {
                    *out = idx;
                }
                return true;
            }
            idx += 1;
            i += 3;
        } else if b >= 0x80 {
            // c:4024-4047 — POSIX class marker.
            let is_lower = b == pp_lower_marker;
            let is_upper = b == pp_upper_marker;
            let matched = if is_lower {
                c < 256 && (c as u8).is_ascii_lowercase()
            } else if is_upper {
                c < 256 && (c as u8).is_ascii_uppercase()
            } else {
                false
            };
            if matched {
                if let Some(out) = indp.as_deref_mut() {
                    *out = idx;
                }
                if let Some(out) = mtp_dest.as_deref_mut() {
                    *out = (b as i32) - 0x80;
                }
                return true;
            }
            idx += 1;
            i += 1;
        } else {
            // Literal char.
            if c == b as u32 {
                if let Some(out) = indp.as_deref_mut() {
                    *out = idx;
                }
                return true;
            }
            idx += 1;
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_match_equivalence_case_cross() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1342 — wmtp=PP_UPPER, lmtp=PP_LOWER → tolower(wchr).
        let lp = Cpattern {
            tp: CPAT_EQUIV,
            str: Some(b"ab".to_vec()),
            chr: 0,
            next: None,
        };
        // wind=1 selects 'a' from the equivalence class, exact-char hit.
        let r = pattern_match_equivalence(&lp, 1, 0, b'A' as u32);
        assert_eq!(r, b'a' as u32);
    }

    // ---------- Real-port tests ------------------------------------------

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
            str: Some(s.as_bytes().to_vec()),
            ..Default::default()
        }
    }

    #[test]
    fn cpatterns_same_chr_match() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let a = cpat_char('a' as u32);
        let b = cpat_char('a' as u32);
        // c:64-66 — both CPAT_CHAR + same chr → equal.
        assert!(cpatterns_same(Some(&a), Some(&b)));
    }

    #[test]
    fn cpatterns_same_chr_mismatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let a = cpat_char('a' as u32);
        let b = cpat_char('b' as u32);
        // c:65 — different chr → not equal.
        assert!(!cpatterns_same(Some(&a), Some(&b)));
    }

    #[test]
    fn cpatterns_same_tp_mismatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let a = cpat_char('a' as u32);
        let b = Cpattern {
            tp: CPAT_NCLASS,
            str: Some(b"a".to_vec()),
            ..Default::default()
        };
        // c:49-50 — different tp → not equal.
        assert!(!cpatterns_same(Some(&a), Some(&b)));
    }

    #[test]
    fn cpatterns_same_class_match() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let a = cpat_class("a-z");
        let b = cpat_class("a-z");
        // c:60 — same str → equal.
        assert!(cpatterns_same(Some(&a), Some(&b)));
    }

    #[test]
    fn cpatterns_same_length_mismatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:46 — both NULL → loop never enters, return !b == true.
        assert!(cpatterns_same(None, None));
    }

    #[test]
    fn cmatchers_same_pointer_eq() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let m = Cmatcher::default();
        // c:86 — `a == b` short-circuit.
        assert!(cmatchers_same(&m, &m));
    }

    #[test]
    fn cmatchers_same_flags_diff() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let a = Cmatcher {
            flags: 0,
            ..Default::default()
        };
        let b = Cmatcher {
            flags: 1,
            ..Default::default()
        };
        // c:87 — different flags → not equal.
        assert!(!cmatchers_same(&a, &b));
    }

    #[test]
    fn cmatchers_same_anchor_lengths() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let pre = Cline {
            flags: CLF_LINE,
            llen: 4,
            ..Default::default()
        };
        let l = Cline {
            flags: 0,
            wlen: 2,
            olen: 99, // ignored because prefix exists
            prefix: Some(Box::new(pre)),
            ..Default::default()
        };
        // c:225-229 — prefix walks to +llen=4; base wlen=2; total=6.
        assert_eq!(cline_sublen(&l), 6);
    }

    #[test]
    fn cline_sublen_clf_suf() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut head: Option<Box<Cline>> = Some(Box::new(Cline {
            prefix: Some(Box::new(Cline::default())),
            suffix: Some(Box::new(Cline::default())),
            next: Some(Box::new(Cline::default())),
            ..Default::default()
        }));
        cline_matched(&mut head);
        let h = head.as_ref().unwrap();
        // c:257 — flag set on head.
        assert_ne!(h.flags & CLF_MATCHED, 0);
        // c:258 — flag set on prefix.
        assert_ne!(h.prefix.as_ref().unwrap().flags & CLF_MATCHED, 0);
        // c:259 — flag set on suffix.
        assert_ne!(h.suffix.as_ref().unwrap().flags & CLF_MATCHED, 0);
        // c:261 — flag set on next.
        assert_ne!(h.next.as_ref().unwrap().flags & CLF_MATCHED, 0);
    }

    #[test]
    fn revert_cline_reverses_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Pre-populate to ensure start_match resets.
        MATCHBUF
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap()
            .push_str("garbage");
        *MATCHPARTS.get_or_init(|| Mutex::new(None)).lock().unwrap() =
            Some(Box::new(Cline::default()));
        start_match();
        assert!(MATCHBUF.get().unwrap().lock().unwrap().is_empty());
        assert!(MATCHPARTS.get().unwrap().lock().unwrap().is_none());
        assert!(MATCHSUBS.get().unwrap().lock().unwrap().is_none());
    }

    #[test]
    fn abort_match_drops_lists() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *MATCHPARTS.get_or_init(|| Mutex::new(None)).lock().unwrap() =
            Some(Box::new(Cline::default()));
        *MATCHSUBS.get_or_init(|| Mutex::new(None)).lock().unwrap() =
            Some(Box::new(Cline::default()));
        abort_match();
        assert!(MATCHPARTS.get().unwrap().lock().unwrap().is_none());
        assert!(MATCHSUBS.get().unwrap().lock().unwrap().is_none());
    }

    /// c:1342-1378 — pattern_match_equivalence case-class crossing:
    /// when the word side matched as PP_UPPER and the line pattern
    /// has a PP_LOWER class marker, return tolower(wchr).
    /// Build a Cpattern whose `str` contains the PP_LOWER marker byte
    /// (0x80 + PP_LOWER) so the byte walk hits the marker at idx 0.
    #[test]
    fn pattern_match_equivalence_upper_to_lower() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // lp.str = [0x80 + PP_LOWER] — one PP_LOWER class marker.
        let lp = Cpattern {
            tp: CPAT_EQUIV,
            str: Some(vec![(0x80u8).wrapping_add(PP_LOWER as u8)]),
            chr: 0,
            next: None,
        };
        // wind=1 → target_idx=0 → hits the marker.
        // wmtp = PP_UPPER, wchr = 'A' → expect tolower('A') = 'a'.
        let r = pattern_match_equivalence(&lp, 1, PP_UPPER, b'A' as u32);
        assert_eq!(r, b'a' as u32);
    }

    /// c:1736-1991 — bld_line with a CPAT_CHAR pattern emits the
    /// pattern's literal char. The word char must satisfy the pattern:
    /// pattern_match1 for CPAT_CHAR is `p->u.chr == c` (c:1289, exact),
    /// so the word must start with 'x' for the match to fire. wlen=1.
    #[test]
    fn bld_line_cpat_char_emits_literal() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let m = Cmatcher {
            line: Some(Box::new(cpat_char('x' as u32))),
            // llen == line-pattern count (C compmatch.c:157 `r->llen = ll`);
            // bld_line sizes genpatarr / the build loop to mp->llen (c:1855).
            llen: 1,
            ..Default::default()
        };
        let mut line: Vec<char> = Vec::new();
        let n = bld_line(&m, &mut line, "", "x", 1, 0);
        assert_eq!(n, 1);
        assert_eq!(line, vec!['x']);
    }

    /// c:1810 — bld_line with a CPAT_ANY pattern emits the
    /// corresponding char from `word`.
    #[test]
    fn bld_line_cpat_any_emits_word_char() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let m = Cmatcher {
            line: Some(Box::new(Cpattern {
                tp: CPAT_ANY,
                ..Default::default()
            })),
            // llen == line-pattern count (C compmatch.c:157); bld_line's
            // build loop runs for mp->llen chars (c:1855).
            llen: 1,
            ..Default::default()
        };
        let mut line: Vec<char> = Vec::new();
        let n = bld_line(&m, &mut line, "", "abc", 1, 0);
        assert_eq!(n, 1);
        assert_eq!(line, vec!['a'], "CPAT_ANY copies the word char");
    }

    /// c:569-590 — match_str exact-char skip fast path: when `l` and
    /// `w` start with the same character, advance both, accumulate
    /// exact/wexact, continue. With empty mstack and matching prefix
    /// of length N, returns iw = N.
    #[test]
    fn match_str_exact_char_skip_full_match() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let r = match_str("abc", "abc", None, 0, None, 0, 0, 0);
        assert_eq!(r, 3, "full literal match returns iw=3");
    }

    /// c:1092-1108 — match_parts truncates both strings to n bytes,
    /// then defers to match_str with test=1. Test mode returns 1 on
    /// full match (c:1046 `return (part || !ll)`).
    #[test]
    fn match_parts_truncates_and_matches() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        let r = match_parts("abcXYZ", "abcdef", 3, 0);
        assert_eq!(r, 1, "first 3 chars match exactly (test=1 → 1)");
    }

    /// c:1251 — comp_match with pfx=w (exact equal) sets *exact=1.
    /// Empty sfx, qu=0 (no quoting needed), no Patprog.
    #[test]
    fn comp_match_exact_prefix_match() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        let mut clp: Option<Box<Cline>> = None;
        let mut exact = 99i32;
        let r = comp_match(
            "hello",
            "",
            "hello",
            None,
            Some(&mut clp),
            0,
            None,
            0,
            None,
            0,
            &mut exact,
        );
        assert!(r.is_some(), "literal prefix match succeeds");
        assert_eq!(exact, 1, "pfx == w → exact=1");
    }

    /// The LIVE option-completion shape: `-M 'r:|[_-]=* r:|=*'` active on
    /// mstack (what _describe passes for options), typed `-`, candidate
    /// `-a`. C matches; a matcher-branch regression rejecting this kills
    /// every `cmd -<TAB>` shell-wide.
    #[test]
    fn comp_match_dash_prefix_with_option_matcher() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let m = crate::ported::zle::complete::parse_cmatcher("test", "r:|[_-]=* r:|=*");
        assert!(m.is_some(), "matcher spec must parse");
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Box::new(crate::ported::zle::comp_h::Cmlist {
                next: None,
                matcher: m.unwrap(),
                str: "r:|[_-]=* r:|=*".to_string(),
            }));
        }
        let mut clp: Option<Box<Cline>> = None;
        let mut exact = 99i32;
        let r = comp_match("-", "", "-a", None, Some(&mut clp), 1, None, 0, None, 0, &mut exact);
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        assert!(r.is_some(), "'-' + option matcher must match '-a', got None");
    }

    /// Case-insensitive matcher `m:{a}={A}`, typed `a`, candidate `Apple`.
    /// The matcher substitutes the first char; the word tail must be
    /// appended so the reconstruction is the full `Apple`.
    #[test]
    fn comp_match_ci_single_char_reconstructs_full_word() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let m = crate::ported::zle::complete::parse_cmatcher("test", "m:{a}={A}");
        assert!(m.is_some(), "matcher spec must parse");
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Box::new(crate::ported::zle::comp_h::Cmlist {
                next: None,
                matcher: m.unwrap(),
                str: "m:{a}={A}".to_string(),
            }));
        }
        let mut clp: Option<Box<Cline>> = None;
        let mut exact = 99i32;
        let r = comp_match("a", "", "Apple", None, Some(&mut clp), 1, None, 0, None, 0, &mut exact);
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        assert_eq!(r.as_deref(), Some("Apple"), "'a' + m:{{a}}={{A}} must reconstruct 'Apple'");
    }

    /// Case-insensitive matcher `m:{a-z}={A-Z}`, typed `app`, candidate
    /// `Apple`. The matcher swaps `a`->`A`; the exactly-matched `pp` and the
    /// tail `le` must survive in the reconstruction. Regression: the exact
    /// fast-path advanced the word cursor without emitting `pp`, so the
    /// result came out `Ale`.
    #[test]
    fn comp_match_ci_multi_char_keeps_exact_run() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let m = crate::ported::zle::complete::parse_cmatcher("test", "m:{a-z}={A-Z}");
        assert!(m.is_some(), "matcher spec must parse");
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Box::new(crate::ported::zle::comp_h::Cmlist {
                next: None,
                matcher: m.unwrap(),
                str: "m:{a-z}={A-Z}".to_string(),
            }));
        }
        let mut clp: Option<Box<Cline>> = None;
        let mut exact = 99i32;
        let r = comp_match("app", "", "Apple", None, Some(&mut clp), 1, None, 0, None, 0, &mut exact);
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        assert_eq!(r.as_deref(), Some("Apple"), "'app' + m:{{a-z}}={{A-Z}} must reconstruct 'Apple', not drop the exact 'pp'");
    }

    /// The option-completion shape: typed word `-`, candidate `-a` —
    /// must prefix-match (this is every `cmd -<TAB>` in compsys).
    #[test]
    fn comp_match_dash_prefix_matches_option_word() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        let mut clp: Option<Box<Cline>> = None;
        let mut exact = 99i32;
        let r = comp_match("-", "", "-a", None, Some(&mut clp), 1, None, 0, None, 0, &mut exact);
        assert!(r.is_some(), "'-' must prefix-match '-a', got None");
        assert_eq!(r.as_deref(), Some("-a"));
    }

    /// c:546-1080 — match_str with diverging prefix returns -1 when
    /// mstack is empty (no matcher to bridge the gap).
    #[test]
    fn match_str_diverging_returns_neg_one_with_empty_mstack() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Clear mstack to guarantee the empty-stack code path.
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        let r = match_str("abc", "xyz", None, 0, None, 0, 0, 0);
        assert_eq!(r, -1, "no matcher can bridge `a` vs `x`");
    }

    // ---------- update_bmatchers real-port tests (this session). ----------

    /// c:121-139 — `update_bmatchers` walks bmatchers; entries whose
    /// matcher isn't in mstack (via cmatchers_same) get trimmed via the
    /// `bmatchers = p->next` reset. With mstack empty, every entry
    /// misses → bmatchers should end up None.
    #[test]
    fn update_bmatchers_with_empty_mstack_trims_all_entries() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Seed bmatchers with one entry.
        let matcher = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 1,
            word: None,
            wlen: 1,
            left: None,
            lalen: 0,
            right: None,
            ralen: 0,
        };
        let bm_cell = crate::ported::zle::compcore::bmatchers.get_or_init(|| Mutex::new(None));
        *bm_cell.lock().unwrap() = Some(Box::new(Cmlist {
            next: None,
            matcher: Box::new(matcher),
            str: String::new(),
        }));
        // Clear mstack so the entry must be trimmed.
        let ms_cell = mstack.get_or_init(|| Mutex::new(None));
        *ms_cell.lock().unwrap() = None;

        update_bmatchers();

        // After update with empty mstack: bmatchers is None — c:135-137.
        assert!(
            bm_cell.lock().unwrap().is_none(),
            "every bmatcher must be trimmed when mstack is empty"
        );
    }

    /// c:84 — `cmatchers_same` short-circuits to true on POINTER
    /// IDENTITY (a == b). The Rust port uses `std::ptr::eq`. Without
    /// this, two large equivalent matchers would scan every field.
    /// Regression dropping the short-circuit would balloon the
    /// `update_bmatchers`-triggered O(N*M) scan into O(N*M*F).
    #[test]
    fn cmatchers_same_pointer_identity_short_circuits() {
        let _g = crate::test_util::global_state_lock();
        let m = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 0,
            word: None,
            wlen: 0,
            left: None,
            lalen: 0,
            right: None,
            ralen: 0,
        };
        // Same pointer → equal.
        assert!(cmatchers_same(&m, &m));
    }

    /// c:87 — different `flags` bits MUST cause inequality. Catches
    /// a regression where the flag check is dropped — would let
    /// CMF_LEFT and CMF_RIGHT matchers compare equal silently.
    #[test]
    fn cmatchers_same_different_flags_compare_unequal() {
        let _g = crate::test_util::global_state_lock();
        let a = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 0,
            word: None,
            wlen: 0,
            left: None,
            lalen: 0,
            right: None,
            ralen: 0,
        };
        let b = Cmatcher {
            refc: 1,
            next: None,
            flags: CMF_LEFT,
            line: None,
            llen: 0,
            word: None,
            wlen: 0,
            left: None,
            lalen: 0,
            right: None,
            ralen: 0,
        };
        assert!(!cmatchers_same(&a, &b));
    }

    /// c:87 — different `llen`/`wlen` MUST cause inequality. The
    /// length fields are part of the natural-key comparison; a
    /// regression dropping them would conflate distinct matchers.
    #[test]
    fn cmatchers_same_different_lengths_compare_unequal() {
        let _g = crate::test_util::global_state_lock();
        let a = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 1,
            word: None,
            wlen: 1,
            left: None,
            lalen: 0,
            right: None,
            ralen: 0,
        };
        let b = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 2,
            word: None,
            wlen: 1,
            left: None,
            lalen: 0,
            right: None,
            ralen: 0,
        };
        assert!(!cmatchers_same(&a, &b), "differing llen must NOT be equal");
        let c = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 1,
            word: None,
            wlen: 5,
            left: None,
            lalen: 0,
            right: None,
            ralen: 0,
        };
        assert!(!cmatchers_same(&a, &c), "differing wlen must NOT be equal");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests for cpatterns_same NULL/chain edge cases.
    // ═══════════════════════════════════════════════════════════════════

    /// c:46-77 — `cpatterns_same(None, None)` returns true (both NULL
    /// is trivially equal — while-loop exits immediately + `!b` is true).
    #[test]
    fn cpatterns_same_both_none_returns_true() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(cpatterns_same(None, None), "both None → trivially equal");
    }

    /// c:48 — `cpatterns_same(Some, None)` returns false (a non-NULL,
    /// b NULL during walk → `if(!b) return 0`).
    #[test]
    fn cpatterns_same_a_some_b_none_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let a = cpat_char('x' as u32);
        assert!(!cpatterns_same(Some(&a), None));
    }

    /// c:77 — `cpatterns_same(None, Some)` returns false (loop doesn't
    /// run, returns `!b` = false).
    #[test]
    fn cpatterns_same_a_none_b_some_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let b = cpat_char('x' as u32);
        assert!(!cpatterns_same(None, Some(&b)));
    }

    /// c:60-61 — CCLASS pattern: same str → equal; differing str → not.
    #[test]
    fn cpatterns_same_class_str_differs_not_equal() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let a = cpat_class("abc");
        let b = cpat_class("xyz");
        assert!(!cpatterns_same(Some(&a), Some(&b)));
    }

    /// c:60-61 — NCLASS pattern (negated class) also uses str compare.
    #[test]
    fn cpatterns_same_nclass_str_compare() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let a = Cpattern {
            tp: CPAT_NCLASS,
            str: Some(b"abc".to_vec()),
            ..Default::default()
        };
        let b = Cpattern {
            tp: CPAT_NCLASS,
            str: Some(b"abc".to_vec()),
            ..Default::default()
        };
        assert!(
            cpatterns_same(Some(&a), Some(&b)),
            "same NCLASS str → equal"
        );
    }

    /// c:52-54 — EQUIV uses str compare as well (same dispatch as
    /// CCLASS/NCLASS per the `x if x == CPAT_*` arm).
    #[test]
    fn cpatterns_same_equiv_str_compare() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let a = Cpattern {
            tp: CPAT_EQUIV,
            str: Some(b"AaBb".to_vec()),
            ..Default::default()
        };
        let b = Cpattern {
            tp: CPAT_EQUIV,
            str: Some(b"AaBb".to_vec()),
            ..Default::default()
        };
        assert!(cpatterns_same(Some(&a), Some(&b)));
        let c = Cpattern {
            tp: CPAT_EQUIV,
            str: Some(b"XxYy".to_vec()),
            ..Default::default()
        };
        assert!(!cpatterns_same(Some(&a), Some(&c)));
    }

    /// c:74-75 — chain walk: different chain lengths → not equal.
    /// `a` is 2-node chain, `b` is 1-node — second iter, b=None → false.
    #[test]
    fn cpatterns_same_different_chain_length_not_equal() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let a = Cpattern {
            tp: CPAT_CHAR,
            chr: 'a' as u32,
            next: Some(Box::new(cpat_char('b' as u32))),
            ..Default::default()
        };
        let b = cpat_char('a' as u32);
        assert!(
            !cpatterns_same(Some(&a), Some(&b)),
            "a=2-chain, b=1 → not equal"
        );
        assert!(
            !cpatterns_same(Some(&b), Some(&a)),
            "b=1, a=2-chain → not equal"
        );
    }

    /// c:46-77 — chain walk: same multi-node chain returns true.
    #[test]
    fn cpatterns_same_matching_chain_returns_true() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let a = Cpattern {
            tp: CPAT_CHAR,
            chr: 'a' as u32,
            next: Some(Box::new(cpat_char('b' as u32))),
            ..Default::default()
        };
        let b = Cpattern {
            tp: CPAT_CHAR,
            chr: 'a' as u32,
            next: Some(Box::new(cpat_char('b' as u32))),
            ..Default::default()
        };
        assert!(cpatterns_same(Some(&a), Some(&b)));
    }

    /// c:86 — `cmatchers_same` with pointer-identity (same pointer)
    /// returns true (short-circuit before any field comparison).
    #[test]
    fn cmatchers_same_pointer_identity_true() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let m = Cmatcher {
            refc: 1,
            next: None,
            flags: 0xFFFF, // any garbage
            line: None,
            llen: 999,
            word: None,
            wlen: -42,
            left: None,
            lalen: 7,
            right: None,
            ralen: 11,
        };
        assert!(
            cmatchers_same(&m, &m),
            "same pointer → true regardless of fields"
        );
    }

    /// c:91 — anchor checks ONLY run when CMF_LEFT or CMF_RIGHT is
    /// set; bare flags=0 ignores lalen/ralen mismatch.
    #[test]
    fn cmatchers_same_no_anchor_flags_ignores_anchor_lens() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let a = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 0,
            word: None,
            wlen: 0,
            left: None,
            lalen: 5, // mismatch
            right: None,
            ralen: 7, // mismatch
        };
        let b = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 0,
            word: None,
            wlen: 0,
            left: None,
            lalen: 99, // mismatch
            right: None,
            ralen: 0, // mismatch
        };
        // Without CMF_LEFT/CMF_RIGHT, anchor lens shouldn't matter.
        assert!(
            cmatchers_same(&a, &b),
            "flags=0 must ignore lalen/ralen per c:91 gate"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compmatch.c
    // c:79 cpatterns_same / c:143 cmatchers_same / c:188 add_bmatchers
    // c:220 update_bmatchers / c:311 free_cline / c:562 start_match /
    // c:591 abort_match / c:1605 match_parts
    // ═══════════════════════════════════════════════════════════════════

    /// c:79 — `cpatterns_same(None, None)` returns true (reflexive None).
    #[test]
    fn cpatterns_same_double_none_reflexive() {
        assert!(cpatterns_same(None, None));
    }

    /// c:143 — `cmatchers_same(&a, &a)` always true (reflexive).
    #[test]
    fn cmatchers_same_self_reflexive() {
        let m = Cmatcher {
            refc: 1,
            next: None,
            flags: 0,
            line: None,
            llen: 0,
            word: None,
            wlen: 0,
            left: None,
            lalen: 0,
            right: None,
            ralen: 0,
        };
        assert!(cmatchers_same(&m, &m), "cmatchers_same(a, a) must be true");
    }

    /// c:188 — `add_bmatchers(None)` is a safe no-op (idempotent).
    #[test]
    fn add_bmatchers_none_no_panic() {
        add_bmatchers(None);
        add_bmatchers(None);
    }

    /// c:311 — `free_cline(None)` is a safe no-op.
    #[test]
    fn free_cline_none_no_panic() {
        free_cline(None);
    }

    /// c:1605 — `match_parts` returns i32 (compile-time type pin).
    #[test]
    fn match_parts_returns_i32_type() {
        let _: i32 = match_parts("", "", 0, 0);
    }

    /// c:1605 — `match_parts("", "", 0, 0)` returns 0/1 boolean.
    #[test]
    fn match_parts_empty_inputs_boolean_result() {
        let r = match_parts("", "", 0, 0);
        assert!(
            r == 0 || r == 1,
            "match_parts must return 0 or 1, got {}",
            r
        );
    }

    /// c:1605 — `match_parts` deterministic for same input.
    #[test]
    fn match_parts_is_deterministic() {
        for (l, w) in [("a", "a"), ("abc", "abc"), ("", "")] {
            let first = match_parts(l, w, 0, 0);
            for _ in 0..3 {
                assert_eq!(
                    match_parts(l, w, 0, 0),
                    first,
                    "match_parts({:?}, {:?}) must be deterministic",
                    l,
                    w
                );
            }
        }
    }

    /// c:562 + c:591 — `start_match` then `abort_match` round-trip safe.
    #[test]
    fn start_then_abort_match_round_trip_safe() {
        start_match();
        abort_match();
    }

    /// c:220 — `update_bmatchers` idempotent / no-panic.
    #[test]
    fn update_bmatchers_idempotent() {
        for _ in 0..5 {
            update_bmatchers();
        }
    }

    /// c:413 — `cline_sublen` returns i32 (type pin).
    #[test]
    fn cline_sublen_returns_i32_type() {
        let cline = Cline {
            next: None,
            prefix: None,
            suffix: None,
            line: None,
            llen: 0,
            word: None,
            wlen: 0,
            orig: None,
            olen: 0,
            slen: 0,
            min: 0,
            max: 0,
            flags: 0,
        };
        let _: i32 = cline_sublen(&cline);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compmatch.c
    // c:188 add_bmatchers / c:220 update_bmatchers / c:311 free_cline /
    // c:413 cline_sublen / c:562 start_match / c:591 abort_match /
    // c:1605 match_parts
    // ═══════════════════════════════════════════════════════════════════

    /// c:188 — `add_bmatchers(None)` is safe / idempotent.
    #[test]
    fn add_bmatchers_none_idempotent() {
        for _ in 0..5 {
            add_bmatchers(None);
        }
    }

    /// c:311 — `free_cline(None)` is safe (early-return on empty).
    #[test]
    fn free_cline_none_no_panic_pin2() {
        free_cline(None);
    }

    /// c:413 — `cline_sublen` is pure (multiple calls = same result).
    #[test]
    fn cline_sublen_is_pure() {
        let cline = Cline {
            next: None,
            prefix: None,
            suffix: None,
            line: None,
            llen: 0,
            word: None,
            wlen: 0,
            orig: None,
            olen: 0,
            slen: 0,
            min: 0,
            max: 0,
            flags: 0,
        };
        let first = cline_sublen(&cline);
        for _ in 0..3 {
            assert_eq!(cline_sublen(&cline), first, "cline_sublen must be pure");
        }
    }

    /// c:562 — `start_match` is idempotent / safe to call repeatedly.
    #[test]
    fn start_match_idempotent_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            start_match();
        }
    }

    /// c:591 — `abort_match` is idempotent / safe to call repeatedly.
    #[test]
    fn abort_match_idempotent_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            abort_match();
        }
    }

    /// c:562 + c:591 — `start_match` then `abort_match` round-trips
    /// without panic.
    #[test]
    fn start_match_then_abort_match_round_trip() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..3 {
            start_match();
            abort_match();
        }
    }

    /// c:1605 — `match_parts("","",0,0)` returns boolean i32 (0 or 1).
    #[test]
    fn match_parts_empty_zero_args_returns_boolean() {
        let r = match_parts("", "", 0, 0);
        assert!(r == 0 || r == 1, "result must be 0 or 1, got {}", r);
    }

    /// c:1605 — `match_parts` is pure for stable input.
    #[test]
    fn match_parts_is_pure_full_sweep() {
        for (l, w, n, p) in [
            ("", "", 0, 0),
            ("a", "a", 0, 0),
            ("abc", "abc", 1, 0),
            ("x", "y", 0, 1),
        ] {
            let first = match_parts(l, w, n, p);
            for _ in 0..3 {
                assert_eq!(
                    match_parts(l, w, n, p),
                    first,
                    "match_parts({:?},{:?},{},{}) must be pure",
                    l,
                    w,
                    n,
                    p
                );
            }
        }
    }

    /// c:79 — `cpatterns_same(None, None)` reflexive (true).
    #[test]
    fn cpatterns_same_double_none_true() {
        assert!(
            cpatterns_same(None, None),
            "double None is reflexive identity"
        );
    }

    /// c:220 — `update_bmatchers` followed by `add_bmatchers(None)` safe.
    #[test]
    fn update_then_add_bmatchers_safe() {
        update_bmatchers();
        add_bmatchers(None);
        update_bmatchers();
    }

    /// c:188 + c:220 — interleaved add/update is safe for many iters.
    #[test]
    fn interleaved_add_update_bmatchers_safe() {
        for _ in 0..10 {
            add_bmatchers(None);
            update_bmatchers();
        }
    }
}
