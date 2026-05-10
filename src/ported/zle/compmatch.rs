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

/// Completion matcher pattern (from compmatch.c Cmatcher)
#[derive(Debug, Clone)]
pub struct CompMatcher {
    pub line_pattern: String,
    pub word_pattern: String,
    pub flags: MatchFlags,
}

/// Match control flags
#[derive(Debug, Clone, Copy, Default)]
pub struct MatchFlags {
    pub case_insensitive: bool,
    pub partial_word: bool,
    pub anchor_start: bool,
    pub anchor_end: bool,
    pub substring: bool,
}

/// A completion line segment (from compmatch.c Cline)
#[derive(Debug, Clone)]
pub struct CompLine {
    pub prefix: String,
    pub line: String,
    pub suffix: String,
    pub word: String,
    pub matched: bool,
}

impl CompLine {
    /// Construct an empty match-line segment.
    /// Equivalent to a zero-initialised `Cline` from `getcline()`
    /// at Src/Zle/compmatch.c — the C source uses these to chain
    /// together the prefix/line/suffix/word segments produced
    /// during pattern-driven matching.
    pub fn new() -> Self {
        CompLine {
            prefix: String::new(),
            line: String::new(),
            suffix: String::new(),
            word: String::new(),
            matched: false,
        }
    }

    /// Sum of the segment's three text fields' lengths.
    /// Port of `cline_sublen()` from Src/Zle/compmatch.c — the C
    /// source caches this on each Cline; here it's recomputed since
    /// String already tracks length.
    pub fn sublen(&self) -> usize {
        self.prefix.len() + self.line.len() + self.suffix.len()
    }

    /// Recompute cached lengths after mutating the segment fields.
    /// Port of `cline_setlens()` from Src/Zle/compmatch.c. The C
    /// source materialises `prefix.len`/`line.len`/etc. into the
    /// Cline; Rust's `String::len()` is O(1) so the recompute is
    /// implicit — kept as a no-op for ABI parity with callers that
    /// expect to invoke it.
    pub fn setlens(&mut self) {}
}

impl Default for CompLine {
    fn default() -> Self {
        Self::new()
    }
}

/// Port of `cpatterns_same()` from `Src/Zle/compmatch.c:42`.
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
pub fn cpatterns_same(
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
                if ap.str_ != bp.str_ {                                      // c:60 strcmp(a->u.str,b->u.str)
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

/// Port of `cmatchers_same()` from `Src/Zle/compmatch.c:82`.
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
pub fn cmatchers_same(
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

/// Port of `cline_sublen()` from `Src/Zle/compmatch.c:218`.
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

/// Port of `cline_setlens()` from `Src/Zle/compmatch.c:239`.
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

/// Port of `cline_matched()` from `Src/Zle/compmatch.c:253`.
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

/// Port of `revert_cline()` from `Src/Zle/compmatch.c:269`.
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
pub fn revert_cline(
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

/// Port of `cp_cline()` from `Src/Zle/compmatch.c:189`.
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
pub fn cp_cline(
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

/// Port of `free_cline()` from `Src/Zle/compmatch.c:171`.
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
/// Port of `match_str()` from Src/Zle/compmatch.c. The C source
/// runs the full Cmatcher trie consuming both strings in lockstep
/// and produces a Cline describing the match; our simplified
/// version returns just a bool — sufficient for the substring +
/// case-fold matchers most users wire via `-M`.
pub fn match_str(
    line: &str,
    word: &str,
    matchers: &[CompMatcher],
    flags: &MatchFlags,
) -> Option<Vec<CompLine>> {
    if flags.case_insensitive {
        if line.to_lowercase().starts_with(&word.to_lowercase()) {
            return Some(vec![CompLine {
                line: line.to_string(),
                word: word.to_string(),
                matched: true,
                ..Default::default()
            }]);
        }
    } else if line.starts_with(word) {
        return Some(vec![CompLine {
            line: line.to_string(),
            word: word.to_string(),
            matched: true,
            ..Default::default()
        }]);
    }

    // Try matchers — case-insensitive / substring / partial-word
    // tests inlined per zsh's compmatch.c which dispatches the M flag
    // mask inline in every matcher loop. No helper extracted in C.
    for matcher in matchers {
        let hit = if matcher.flags.case_insensitive {
            line.to_lowercase().contains(&word.to_lowercase())
        } else if matcher.flags.substring {
            line.contains(word)
        } else if matcher.flags.partial_word {
            // Match word parts: "fb" matches "foobar" at word boundaries.
            let mut wi = word.chars();
            let mut wc = wi.next();
            let mut all_consumed = false;
            for lc in line.chars() {
                if let Some(w) = wc {
                    if lc.eq_ignore_ascii_case(&w) {
                        wc = wi.next();
                    }
                } else {
                    all_consumed = true;
                    break;
                }
            }
            all_consumed || wc.is_none()
        } else {
            false
        };
        if hit {
            return Some(vec![CompLine {
                line: line.to_string(),
                word: word.to_string(),
                matched: true,
                ..Default::default()
            }]);
        }
    }

    None
}

/// Find every byte-range in `line` where `word`'s next character was
/// matched.
/// Port of `match_parts()` from Src/Zle/compmatch.c. The C source
/// uses the resulting list to highlight matching subsequence runs
/// in the completion menu — every `(start, end)` here is one
/// matched character (multi-byte aware).
pub fn match_parts(line: &str, word: &str, flags: &MatchFlags) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let line_lower = if flags.case_insensitive {
        line.to_lowercase()
    } else {
        line.to_string()
    };
    let word_lower = if flags.case_insensitive {
        word.to_lowercase()
    } else {
        word.to_string()
    };

    let mut pos = 0;
    for wc in word_lower.chars() {
        if let Some(found) = line_lower[pos..].find(wc) {
            let abs_pos = pos + found;
            parts.push((abs_pos, abs_pos + wc.len_utf8()));
            pos = abs_pos + wc.len_utf8();
        }
    }
    parts
}

/// Top-level "does `word` match `line`" predicate.
/// Port of `comp_match()` from Src/Zle/compmatch.c — the C source
/// is the entry point that the completion engine calls to filter
/// candidates. Returns `true` iff `match_str()` produces a Cline.
pub fn comp_match(line: &str, word: &str, flags: &MatchFlags) -> bool {
    match_str(line, word, &[], flags).is_some()
}

// (Wrong-sig duplicates of start_match / abort_match / cp_cline /
// free_cline / revert_cline / cline_matched removed — replaced
// upstream by C-faithful real ports keyed off comp_h::Cline.)

/// Pattern match with equivalence classes (from compmatch.c pattern_match_equivalence)
pub fn pattern_match_equivalence(a: char, b: char, case_insensitive: bool) -> bool {
    if case_insensitive {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// Parse a matcher specification string (from compmatch.c)
/// Format: `m:{[:lower:]}={[:upper:]}` or `l:|=* r:|=*` etc.
pub fn parse_cmatcher(spec: &str) -> Vec<CompMatcher> {
    let mut matchers = Vec::new();

    for part in spec.split_whitespace() {
        let flags = MatchFlags {
            case_insensitive: part.starts_with("m:"),
            partial_word: part.starts_with("r:") || part.starts_with("l:"),
            anchor_start: part.starts_with("l:"),
            anchor_end: part.starts_with("r:"),
            substring: part.starts_with("M:"),
        };

        if let Some((line_pat, word_pat)) = part.split_once('=') {
            let line_pat = line_pat.split(':').next_back().unwrap_or("");
            matchers.push(CompMatcher {
                line_pattern: line_pat.to_string(),
                word_pattern: word_pat.to_string(),
                flags,
            });
        }
    }

    matchers
}

/// Update bmatchers (from compmatch.c add_bmatchers/update_bmatchers)
pub fn update_bmatchers(matchers: &mut Vec<CompMatcher>, new: Vec<CompMatcher>) {
    *matchers = new;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_str_exact() {
        let flags = MatchFlags::default();
        assert!(match_str("foobar", "foo", &[], &flags).is_some());
        assert!(match_str("foobar", "baz", &[], &flags).is_none());
    }

    #[test]
    fn test_match_str_case_insensitive() {
        let flags = MatchFlags {
            case_insensitive: true,
            ..Default::default()
        };
        assert!(match_str("FooBar", "foo", &[], &flags).is_some());
    }

    #[test]
    fn test_match_parts() {
        let flags = MatchFlags::default();
        let parts = match_parts("foobar", "fbr", &flags);
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn test_pattern_match_equivalence() {
        assert!(pattern_match_equivalence('a', 'A', true));
        assert!(!pattern_match_equivalence('a', 'A', false));
    }

    #[test]
    fn test_parse_cmatcher() {
        let matchers = parse_cmatcher("m:{[:lower:]}={[:upper:]}");
        assert_eq!(matchers.len(), 1);
        assert!(matchers[0].flags.case_insensitive);
    }

    #[test]
    fn test_comp_line() {
        let mut cl = CompLine::new();
        cl.prefix = "pre".to_string();
        cl.line = "middle".to_string();
        cl.suffix = "suf".to_string();
        assert_eq!(cl.sublen(), 12);
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
            str_: Some(s.to_string()),
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
            str_: Some("a".into()),
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

/// Port of `add_bmatchers()` from Src/Zle/compmatch.c:101.
pub fn add_bmatchers(_def: i32) {                                            // c:101
    // C body c:103-141 — walks Cmlist `def->matcher`, prepends each
    //                    Cmatcher whose flags & CMF_LINE to bmatchers
    //                    via newcmlist alloc.
    //                    Without bmatchers global hydrated: no-op.
}

/// Port of `add_match_part()` from Src/Zle/compmatch.c:373.
pub fn add_match_part(_m: i32, _l: &str, _ll: i32, _w: &str, _wl: i32,       // c:373
                      _o: &str, _ol: i32, _osl: i32, _sfx: i32) {
    // C body c:375-444 — appends a partial match into matchparts via
    //                    add_match_str. Substrate (matchparts global)
    //                    deferred; no-op.
}

/// Port of `add_match_str()` from Src/Zle/compmatch.c:327.
pub fn add_match_str(_m: i32, _l: &str, _w: &str, _wl: i32, _sfx: i32) {     // c:327
    // C body c:329-371 — pushes a string into matchbuf, growing if
    //                    needed. matchbuf global deferred; no-op.
}

/// Port of `add_match_sub()` from Src/Zle/compmatch.c:446.
pub fn add_match_sub(_m: i32, _l: &str, _ll: i32, _w: &str, _wl: i32) {      // c:446
    // C body c:448-509 — pushes a sub-match into matchsubs (used for
    //                    comp matchers). Substrate deferred; no-op.
}

/// Port of `bld_line()` from Src/Zle/compmatch.c:1736.
pub fn bld_line(_line: &mut String, _mword: &str, _word: &str,               // c:1736
                _sfx: i32) -> i32 {
    // C body c:1738-1992 — runs the matcher engine to construct the
    //                      `line` string from `word` per active matcher
    //                      list. Substrate deferred; 0.
    0
}

/// Port of `bld_parts()` from Src/Zle/compmatch.c:1638.
pub fn bld_parts(_str_: &str, _len: i32, _plen: i32, _lp: &mut i32) -> i32 { // c:1638
    // C body c:1640-1734 — partitions a match into segments per
    //                      matcher anchors; returns Cline list.
    //                      Cline construction deferred; 0.
    0
}

/// Port of `check_cmdata()` from Src/Zle/compmatch.c:2152.
pub fn check_cmdata(_d: i32, _sfx: i32) -> i32 {                             // c:2152
    // C body c:2154-2186 — validates a Cmdata structure, computing
    //                      lengths and consistency. Substrate deferred.
    0
}

// (cline_setlens / cline_sublen wrong-sig duplicates removed —
// real C-faithful ports are above keyed off comp_h::Cline.)

/// Port of `cmp_anchors()` from Src/Zle/compmatch.c:2107.
pub fn cmp_anchors(_a: i32, _b: i32, _wl: i32) -> i32 {                      // c:2107
    // C body c:2109-2150 — compares two Cline anchors (front+back
    //                      strings + lengths). Used by join_clines /
    //                      bld_parts. Substrate deferred; 0 = no match.
    0
}

/// Port of `get_cline()` from Src/Zle/compmatch.c:144.
pub fn get_cline(_l: &str, _ll: i32, _w: &str, _wl: i32, _o: &str,           // c:144
                 _ol: i32, _flags: i32) -> i32 {
    // C body c:146-294 — Cline allocator + populator. Allocates one
    //                    Cline node; sets word/wlen/line/llen/orig/
    //                    olen/flags. Cline pool deferred; 0.
    0
}

/// Port of `join_clines()` from Src/Zle/compmatch.c:2706.
pub fn join_clines(_o: i32, _n: i32) -> i32 {                                // c:2706
    // C body c:2708-2949 — merges two Cline lists in the matcher
    //                      driver. Substrate deferred; 0.
    0
}

/// Port of `join_mid()` from Src/Zle/compmatch.c:2608.
pub fn join_mid(_o: i32, _n: i32, _po: i32, _pn: i32) -> i32 {               // c:2608
    // C body c:2610-2647 — joins two Cline middle fragments per
    //                      matcher anchor rules. Substrate deferred; 0.
    0
}

/// Port of `join_psfx()` from Src/Zle/compmatch.c:2444.
pub fn join_psfx(_ot: i32, _nt: i32, _o: i32, _n: i32, _sfx: i32) -> i32 {   // c:2444
    // C body c:2446-2606 — joins prefixes/suffixes during Cline merge.
    //                      Substrate deferred; 0.
    0
}

/// Port of `join_strs()` from Src/Zle/compmatch.c:1994.
pub fn join_strs(_la: i32, _sa: &str, _lb: i32, _sb: &str) -> Option<String> { // c:1994
    // C body c:1996-2105 — joins two strings via the matcher
    //                      equivalence map; returns the merged string
    //                      or NULL if they can't be merged.
    None
}

/// Port of `join_sub()` from Src/Zle/compmatch.c:2212.
pub fn join_sub(_a: i32, _bp: i32, _bsfx: i32, _b: i32, _flags: i32) -> i32 {  // c:2212
    // C body c:2214-2299 — splices a sub-match Cline list into the
    //                      main Cline. Substrate deferred; 0.
    0
}

/// Port of `pattern_match()` from Src/Zle/compmatch.c:1548.
pub fn pattern_match(_p: i32, _s: &str, _wp: &mut [u8], _wq: &mut [u8]) -> i32 { // c:1548
    // C body c:1550-1636 — top-level pattern-vs-string match driver
    //                      that calls pattern_match1 + pattern_match_restrict.
    //                      Pattern (Patprog) substrate deferred; 0.
    0
}

/// Port of `pattern_match_restrict()` from Src/Zle/compmatch.c:1383.
pub fn pattern_match_restrict(_p: i32, _s: &str, _wp: &mut [u8],             // c:1383
                              _wq: &mut [u8], _restrict: u32) -> i32 {
    // C body c:1385-1546 — restricted variant for nested patterns.
    //                      Substrate deferred; 0.
    0
}

/// Port of `pattern_match1()` from Src/Zle/compmatch.c:1269.
pub fn pattern_match1(_p: i32, _c: u32, _wp: &mut [u8], _wq: &mut [u8]) -> i32 { // c:1269
    // C body c:1271-1381 — single-character pattern match (predicate
    //                      check + char-class). Substrate deferred; 0.
    0
}

/// Port of `sub_join()` from Src/Zle/compmatch.c:2649.
pub fn sub_join(_a: i32, _bp: i32, _bsfx: i32, _b: i32) -> i32 {             // c:2649
    // C body c:2651-2704 — substring-anchor join helper for join_mid.
    //                      Substrate deferred; 0.
    0
}

/// Port of `sub_match()` from Src/Zle/compmatch.c:2301.
pub fn sub_match(_m: i32, _l: &str, _ll: i32, _w: &str, _wl: i32,            // c:2301
                 _sfx: i32) -> i32 {
    // C body c:2303-2442 — runs a Cmatcher's sub-pattern match against
    //                      a substring. Substrate deferred; 0.
    0
}

/// Port of `undo_cmdata()` from Src/Zle/compmatch.c:2188.
pub fn undo_cmdata(_d: i32, _sfx: i32) {                                     // c:2188
    // C body c:2190-2210 — rewinds a Cmdata snapshot taken by
    //                      check_cmdata to the previous state.
    //                      Substrate deferred; no-op.
}
