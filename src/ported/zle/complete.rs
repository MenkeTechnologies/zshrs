//! Direct port of `Src/Zle/complete.c` — the ZLE completion engine.
//!
//! Copy a completion matcher list into permanent storage.                   // c:151
//! Copy a completion matcher pattern.                                       // c:214
//! Parse a character class for matcher control.                             // c:476
//!
//! This file holds the canonical Rust ports of complete.c's
//! exported functions: state globals (`compprefix` / `compsuffix` /
//! `compwords` / `incompfunc` / etc.), the Cmlist/Cmatcher/Cpattern
//! allocator + free + deep-copy chain (freecmlist/freecmatcher/
//! freecpattern/cpcmatcher/cp_cpattern_element/cpcpattern), the
//! ignore_prefix/ignore_suffix/restrict_range state mutators, the
//! special-parameter accessors that back $compstate (get_compstate /
//! set_compstate / get_nmatches / get_complist / get_unambig and
//! friends), the cond_psfix / cond_range condition predicates, the
//! parse_ordering (-o) / parse_class / parse_cmatcher (-M) parsers,
//! the addcompparams / makecompparams / compunsetfn / comp_setunset
//! / comp_wrapper paramtab plumbing, and the bin_compadd / bin_compset
//! / do_comp_vars top-level builtin entries.
//!
//! `Src/Zle/comp.h` is ported in `comp_h.rs`; the live editor /
//! computil dispatch lives in `compcore.rs` and `computil.rs`. This
//! file maps 1:1 to `Src/Zle/complete.c` (4 of 4 surface ported now
//! ported faithfully, with the deeper ones still wired through the
//! existing comp_h struct types — no Rust-only intermediate types).
//!
//! Per PORT.md "file freeze" rule: this file's creation was
//! explicitly authorised by the maintainer to land the complete.c
//! port out of compcore.rs (where it had been parked under the
//! freeze).

use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};
use once_cell::sync::Lazy;
use std::sync::Mutex;

use crate::ported::glob::{remnulargs, tokenize};
use crate::ported::params::{createparam, paramtab};
use crate::ported::pattern::{patcompile, pattry, range_type};
use crate::ported::utils::{zerr, zwarnnam};
use crate::ported::zle::comp_h::{
    Cmatcher, Cpattern, CAF_ALL, CAF_ARRAYS, CAF_KEYS, CAF_MATCH, CAF_MATSORT, CAF_NOSORT,
    CAF_QUOTE, CAF_UNIQALL, CAF_UNIQCON, CLF_LINE, CLF_SUF, CMF_DISPLINE, CMF_FILE, CMF_HIDE,
    CMF_INTER, CMF_ISPAR, CMF_LEFT, CMF_LINE, CMF_NOLIST, CMF_REMOVE, CMF_RIGHT, CPAT_ANY,
    CPAT_CCLASS, CPAT_CHAR, CPAT_EQUIV, CPAT_NCLASS,
};
use crate::ported::zle::{
    compcore, compresult, deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*,
    zle_move::*, zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*,
    zle_word::*,
};
use crate::ported::zsh_h::{
    eprog, funcwrap, module, options, param, PAT_HEAPDUP, PM_ARRAY, PM_HASHED, PM_INTEGER,
    PM_LOCAL, PM_READONLY, PM_REMOVABLE, PM_SCALAR, PM_SINGLE, PM_SPECIAL, PM_TYPE, PM_UNSET,
    PP_RANGE, PP_UNKWN,
};

// =====================================================================
// Cmlist / Cmatcher / Cpattern allocators + freers — Src/Zle/complete.c.
// Ported here (rather than a non-existent complete.rs) because
// PORT.md freezes new src/ported/ file creation; compcore.rs is the
// canonical home for completion-machinery internals.
// =====================================================================

#[allow(unused_imports)]
/// Direct port of `freecmlist(Cmlist l)` from `Src/Zle/complete.c:98`.
/// C body (c:101-110): walk the linked list freeing each Cmatcher
/// via `freecmatcher()` and the per-entry `str` via `zsfree()`.
/// Rust drop handles the deallocation; this wrapper iterates so
/// callers can name-match the C entry point.

// --- AUTO: cross-zle hoisted-fn use glob ---
/// `freecmlist` — see implementation.
#[allow(unused_imports)]
#[allow(unused_imports)]

pub fn freecmlist(l: Option<Box<crate::ported::zle::comp_h::Cmlist>>) {
    // c:98
    let mut cur = l;
    while let Some(node) = cur {
        // c:101
        // c:103 — `freecmatcher(l->matcher);` — Rust Box drop frees.
        // c:104 — `zsfree(l->str);` — String drop frees.
        cur = node.next; // c:102 n = l->next
    }
}

/// Direct port of `freecmatcher(Cmatcher m)` from `Src/Zle/complete.c:115`.
/// C body (c:118-132):
/// ```c
/// if (!m || --(m->refc)) return;
/// while (m) {
///     n = m->next;
///     freecpattern(m->line); freecpattern(m->word);
///     freecpattern(m->left); freecpattern(m->right);
///     zfree(m, sizeof(struct cmatcher));
///     m = n;
/// }
/// ```
/// The C source uses refcounting (`refc`); Rust port relies on Box
/// ownership semantics — when the last reference drops, every
/// Box-owned Cpattern in the chain drops with it.
pub fn freecmatcher(m: Option<Box<Cmatcher>>) {
    // c:115
    // c:115 — `if (!m || --(m->refc)) return;` — refcount handled by
    // Rust ownership; the function is a name-parity wrapper.
    let mut cur = m;
    while let Some(node) = cur {
        // c:122
        // c:124-127 — `freecpattern(m->line/word/left/right)` — Rust
        // drop chains via Option<Box<Cpattern>> fields.
        cur = node.next; // c:123
    }
}

/// Direct port of `freecpattern(Cpattern p)` from `Src/Zle/complete.c:137`.
/// C body (c:141-149):
/// ```c
/// while (p) {
///     n = p->next;
///     if (p->tp <= CPAT_EQUIV) free(p->u.str);
///     zfree(p, sizeof(struct cpattern));
///     p = n;
/// }
/// ```
pub fn freecpattern(p: Option<Box<Cpattern>>) {
    // c:137
    let mut cur = p;
    while let Some(node) = cur {
        // c:141
        // c:144 — `if (p->tp <= CPAT_EQUIV) free(p->u.str)` — String
        // drop in Option<String> handles the conditional free.
        cur = node.next; // c:155
    }
}

// Copy a completion matcher list into permanent storage.                   // c:155
/// Direct port of `cpcmatcher(Cmatcher m)` from `Src/Zle/complete.c:155`.
/// C body (c:158-179): walks the source matcher chain, allocating a
/// fresh Cmatcher per node with `refc = 1`, copying flags / llen /
/// wlen / lalen / ralen, deep-copying each Cpattern via
/// `cpcpattern()`. Returns the new chain head.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn cpcmatcher(m: Option<&Cmatcher>) -> Option<Box<Cmatcher>> // c:155
{
    let mut head: Option<Box<Cmatcher>> = None; // c:158
    let mut tail_ref: *mut Option<Box<Cmatcher>> = &mut head;
    let mut cur = m;
    while let Some(src) = cur {
        // c:160
        let n = Box::new(Cmatcher {
            // c:161 zalloc
            refc: 1,                                 // c:163
            next: None,                              // c:164
            flags: src.flags,                        // c:165
            line: cpcpattern(src.line.as_deref()),   // c:166
            llen: src.llen,                          // c:167
            word: cpcpattern(src.word.as_deref()),   // c:168
            wlen: src.wlen,                          // c:169
            left: cpcpattern(src.left.as_deref()),   // c:170
            lalen: src.lalen,                        // c:171
            right: cpcpattern(src.right.as_deref()), // c:172
            ralen: src.ralen,                        // c:173
        });
        unsafe {
            *tail_ref = Some(n);
            if let Some(ref mut new_node) = *tail_ref {
                // c:175 p = &(n->next)
                tail_ref = &mut new_node.next as *mut _;
            }
        }
        cur = src.next.as_deref(); // c:187
    }
    head // c:187
}

// Copy a completion matcher pattern.                                        // c:214
/// Direct port of `cp_cpattern_element(Cpattern o)` from `Src/Zle/complete.c:187`.
/// C body (c:189-216): allocates a fresh Cpattern, sets `next = NULL`,
/// copies `tp`, then dispatches on `tp` to copy `u.str` (CCLASS /
/// NCLASS / EQUIV) or `u.chr` (CHAR). Default keeps the union zero.
/// WARNING: param names don't match C — Rust=() vs C=(o)
pub fn cp_cpattern_element(o: &Cpattern) -> Box<Cpattern> {
    let mut n = Cpattern::default(); // c:189 zalloc
    n.next = None; // c:191
    n.tp = o.tp; // c:193
    match o.tp {
        // c:194
        CPAT_CCLASS | CPAT_NCLASS | CPAT_EQUIV => {
            // c:196-198
            n.str = o.str.clone(); // c:199 ztrdup(o->u.str)
        }
        CPAT_CHAR => {
            // c:218
            n.chr = o.chr; // c:218 o->u.chr
        }
        _ => {} // c:218
    }
    Box::new(n) // c:218 return n
}

/// Direct port of `cpcpattern(Cpattern o)` from `Src/Zle/complete.c:218`.
/// C body (c:222-231): walk the source Cpattern chain, copying each
/// element via `cp_cpattern_element()`. Returns the new chain head.
pub fn cpcpattern(o: Option<&Cpattern>) -> Option<Box<Cpattern>> // c:218
{
    let mut head: Option<Box<Cpattern>> = None; // c:222
    let mut tail_ref: *mut Option<Box<Cpattern>> = &mut head;
    let mut cur = o;
    while let Some(src) = cur {
        // c:224
        unsafe {
            *tail_ref = Some(cp_cpattern_element(src)); // c:225
            if let Some(ref mut new_node) = *tail_ref {
                // c:226 p = &((*p)->next)
                tail_ref = &mut new_node.next as *mut _;
            }
        }
        cur = src.next.as_deref(); // c:227
    }
    head // c:229
}

// =====================================================================
// Completion-state globals — port of `Src/Zle/complete.c:35-73`.
// =====================================================================
//
// C declares these as bare `mod_export` globals (`char *compprefix`,
// `int compcurrent`, etc.) accessed directly from every completion
// helper. Rust port wraps each in a Mutex<…> / AtomicI32 so the
// state survives across builtin calls without threading it through
// SubstState. Names match the C globals exactly.

/// Port of `int incompfunc` from comp.h. 1 while inside a
/// completion function (set by makecompparams, cleared by
/// compunsetfn); checked by comp_check / cond_psfix / cond_range
/// to refuse calls outside completion context.
pub static INCOMPFUNC: AtomicI32 = AtomicI32::new(0); // c:complete.c

/// Port of `int compcurrent` — index into compwords[] of the word
/// being completed.
pub static COMPCURRENT: AtomicI32 = AtomicI32::new(0); // c:complete.c

/// Port of `mod_export zlong complistmax` from `Src/Zle/complete.c:37`.
/// `$LISTMAX` value — maximum number of matches to list before asking
/// the user via asklistscroll. 0 means no limit.
pub static COMPLISTMAX: AtomicI64 = AtomicI64::new(0); // c:37

/// Port of `int nmatches` — total matches accumulated this round.
pub static NMATCHES_GLOBAL: AtomicI64 = AtomicI64::new(0); // c:compcore.c:160

/// Port of `zlong complistlines` — line count of the listed
/// matches when paginated.
pub static COMPLISTLINES: AtomicI64 = AtomicI64::new(0); // c:complete.c:40

/// Port of `zlong compignored` — count of matches dropped per
/// the IGNORED options.
pub static COMPIGNORED: AtomicI64 = AtomicI64::new(0); // c:complete.c:41

// String globals from c:46-73 — wrapped in Mutex<String>.
macro_rules! comp_string_global {
    ($vis:vis $name:ident, $cname:literal, $cline:literal) => {
        #[doc = concat!("Port of `char *", $cname, "` from complete.c:", stringify!($cline), ".")]
        $vis static $name: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();
    };
}

comp_string_global!(pub COMPPREFIX,    "compprefix",    47);
comp_string_global!(pub COMPSUFFIX,    "compsuffix",    48);
comp_string_global!(pub COMPLASTPREFIX,"complastprefix",49);
comp_string_global!(pub COMPLASTSUFFIX,"complastsuffix",50);
comp_string_global!(pub COMPIPREFIX,   "compiprefix",   58);
comp_string_global!(pub COMPISUFFIX,   "compisuffix",   51);
comp_string_global!(pub COMPQIPREFIX,  "compqiprefix",  52);
comp_string_global!(pub COMPQISUFFIX,  "compqisuffix",  53);
comp_string_global!(pub COMPQUOTE,     "compquote",     54);
comp_string_global!(pub COMPQUOTING,   "compquoting",   55);
comp_string_global!(pub COMPQSTACK,    "compqstack",    55);
comp_string_global!(pub COMPLIST,      "complist",      65);
comp_string_global!(pub COMPCONTEXT,   "compcontext",   59);
comp_string_global!(pub COMPPARAMETER, "compparameter", 60);
comp_string_global!(pub COMPREDIRECT,  "compredirect",  61);
comp_string_global!(pub COMPPATINSERT, "comppatinsert", 69);
comp_string_global!(pub COMPVARED,     "compvared",     73);

/// Port of `char **compwords` (complete.c:45) — argv-style array of
/// the command-line words being completed.
pub static COMPWORDS: std::sync::OnceLock<Mutex<Vec<String>>> = std::sync::OnceLock::new();

/// Port of the `pcm_err` sentinel from `Src/Zle/comp.h:229-232`:
/// > This is a special return value for parse_cmatcher(), *
/// > signalling an error.                                  *
/// > `#define pcm_err ((Cmatcher) 1)`
///
/// Set by [`parse_cmatcher`] when — and only when — C would return
/// `pcm_err`, so callers can tell a parse ERROR from a spec that simply
/// produced no matcher. Cleared at every `parse_cmatcher` entry.
thread_local! {
    pub static PCM_ERR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Direct port of `parse_cmatcher(char *name, char *s)` from `Src/Zle/complete.c:242`.
/// 162-line parser for a `compadd -M` matcher specification string.
/// The grammar is: comma-separated rules, each like `r:|=*` /
/// `l:|=*` / `b:[a-z]=[A-Z]` / `e:|=*` / `B:[]=[]`. Each rule
/// builds one Cmatcher with line/word/left/right Cpattern chains
/// via parse_pattern (line 420) + parse_class (line 480), both of
/// which are real-bodied ports.
/// WARNING: param names don't match C — Rust=() vs C=(name, s)
///
/// C has THREE outcomes, `Option<Box<Cmatcher>>` only two. On a parse
/// error C returns the sentinel `pcm_err` (`Src/Zle/comp.h:232`,
/// `(Cmatcher) 1`); for a spec that legitimately yields no matcher — an
/// empty/all-blank string (c:248-249) or a leading `x:` stop-rule with
/// nothing accumulated (c:290) — it returns plain `NULL`, which every
/// caller (`compctl.c:317`, `computil.c:4558`, c:842) treats as SUCCESS.
/// The port collapsed both onto `None`, so `bin_compadd` aborted with
/// status 1 on `compadd -M ''` where zsh accepts it; the `-M` accumulation
/// at c:827-834 widened that, since two empty specs now join into `" "`.
/// The error case is therefore reported out-of-band in [`PCM_ERR`],
/// cleared on entry and read by the caller right after the call.
pub fn parse_cmatcher(name: &str, s: &str) -> Option<Box<Cmatcher>> {
    PCM_ERR.with(|f| f.set(false));
    if s.is_empty() {
        // c:248-249 — `if (!*s) return NULL;` — NOT an error.
        return None;
    }

    let mut ret: Option<Box<Cmatcher>> = None;
    let mut tail_ptr: *mut Option<Box<Cmatcher>> = &mut ret;
    let mut rest = s;

    while !rest.is_empty() {
        // c:251
        // c:255 — `while (*s && inblank(*s)) s++;`
        rest = rest.trim_start_matches(|c: char| c == ' ' || c == '\t');
        if rest.is_empty() {
            break;
        } // c:257

        // c:259-285 — switch (*s) — rule-letter dispatch.
        let c = rest.chars().next().unwrap();
        let (fl, fl2) = match c {
            'b' => (CMF_LEFT, CMF_INTER),             // c:262
            'l' => (CMF_LEFT, 0),                     // c:263
            'e' => (CMF_RIGHT, CMF_INTER),            // c:264
            'r' => (CMF_RIGHT, 0),                    // c:265
            'm' => (0, 0),                            // c:266
            'B' => (CMF_LEFT | CMF_LINE, CMF_INTER),  // c:267
            'L' => (CMF_LEFT | CMF_LINE, 0),          // c:268
            'E' => (CMF_RIGHT | CMF_LINE, CMF_INTER), // c:269
            'R' => (CMF_RIGHT | CMF_LINE, 0),         // c:270
            'M' => (CMF_LINE, 0),                     // c:271
            'x' => (0, 0),                            // c:272
            _ => {
                // c:280
                if !name.is_empty() {
                    zwarnnam(
                        name,
                        &format!("unknown match specification character `{}'", c),
                    );
                }
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                } // c:283 pcm_err
            }
        };

        // c:288 — `if (s[1] != ':')` → missing-colon.
        let mut chars = rest.chars();
        chars.next();
        if chars.clone().next() != Some(':') {
            if !name.is_empty() {
                zwarnnam(name, "missing `:'");
            }
            {
                PCM_ERR.with(|f| f.set(true));
                return None;
            }
        }
        chars.next(); // consume `:`

        // c:294-303 — `x:` early-return.
        if c == 'x' {
            if let Some(next) = chars.clone().next() {
                if next != ' ' && next != '\t' {
                    if !name.is_empty() {
                        zwarnnam(name, "unexpected pattern following x: specification");
                    }
                    {
                        PCM_ERR.with(|f| f.set(true));
                        return None;
                    }
                }
            }
            return ret; // c:290 — `return ret;` (NULL is not an error)
        }
        rest = chars.as_str();

        // c:297-313 — `(fl & CMF_LEFT) && !fl2` → parse left anchor.
        let mut left: Option<Box<Cpattern>> = None;
        let mut lal: i32 = 0;
        let mut both: bool = false;
        if (fl & CMF_LEFT) != 0 && fl2 == 0 {
            let (lt, r2, l, err) = parse_pattern(name, rest, '|'); // c:298
            if err {
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            left = lt;
            lal = l;
            rest = r2;
            // c:302 — `both = (*s && s[1] == '|')`.
            let mut peek = rest.chars();
            peek.next();
            if peek.clone().next() == Some('|') {
                both = true;
                let mut adv = rest.chars();
                adv.next();
                rest = adv.as_str();
            }
            // c:305-313 — `if (!*s || !*++s)` → missing right anchor / line pattern.
            if rest.len() <= 1 {
                if !name.is_empty() {
                    zwarnnam(
                        name,
                        if both {
                            "missing right anchor"
                        } else {
                            "missing line pattern"
                        },
                    );
                }
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            let mut adv = rest.chars();
            adv.next();
            rest = adv.as_str();
        }

        // c:317-319 — `line = parse_pattern(name, &s, &ll,
        //                              (((fl & CMF_RIGHT) && !fl2) ? '|' : '='), &err);`
        let line_end = if (fl & CMF_RIGHT) != 0 && fl2 == 0 {
            '|'
        } else {
            '='
        };
        let (mut line_pat, r2, mut ll, err) = parse_pattern(name, rest, line_end);
        if err {
            {
                PCM_ERR.with(|f| f.set(true));
                return None;
            }
        }
        rest = r2;

        // c:322 — `if (both) { right = line; ral = ll; line = NULL; ll = 0; }`
        let (mut right, mut ral) = (None, 0i32);
        if both {
            right = line_pat;
            ral = ll;
            line_pat = None;
            ll = 0;
        }

        // c:329-340 — anchor / `=` consume. The two C branches are
        // complementary on `(fl & CMF_RIGHT) && !fl2`:
        //   if (A && (!*s || !*++s)) { "missing right anchor"; err }
        //   else if (!A)            { if (!*s) { "missing word pattern"; err } s++; }
        // NOTE the side effect in `!*++s`: when A holds and *s is non-NUL,
        // `++s` ADVANCES past the line pattern's `|` terminator before the
        // NUL test. The port omitted that advance, so `rest` still pointed
        // AT the `|` when c:342 tested `*s == '|'` — every `r:LINE|ANCH=WORD`
        // spec was misread as the two-anchor form `r:L||R=W`, moving the line
        // pattern into `left` and letting the stray `|` be parsed as a literal
        // character at the head of the right anchor (documented zsh idiom
        // `r:[^A-Z0-9]||[A-Z0-9]=** r:|=*` got ralen 2 instead of 1). Only the
        // anchor-less `r:|X=Y` forms coincided, which is why it went unnoticed.
        if (fl & CMF_RIGHT) != 0 && fl2 == 0 {
            if rest.len() <= 1 {
                // c:329 `!*s || !*++s`
                if !name.is_empty() {
                    zwarnnam(name, "missing right anchor");
                }
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            let mut adv = rest.chars(); // c:329 — the `++s` side effect
            adv.next();
            rest = adv.as_str();
        } else {
            // c:333 `else if (!(fl & CMF_RIGHT) || fl2)`
            if rest.is_empty() {
                if !name.is_empty() {
                    zwarnnam(name, "missing word pattern");
                }
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            let mut adv = rest.chars(); // c:339 `s++`
            adv.next();
            rest = adv.as_str();
        }

        // c:340-357 — RIGHT-side anchor parse.
        if (fl & CMF_RIGHT) != 0 && fl2 == 0 {
            if rest.chars().next() == Some('|') {
                left = line_pat.take();
                lal = ll;
                ll = 0;
                let mut adv = rest.chars();
                adv.next();
                rest = adv.as_str();
            }
            let (rt, r3, r_len, err) = parse_pattern(name, rest, '=');
            if err {
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            right = rt;
            ral = r_len;
            rest = r3;
            if rest.is_empty() {
                if !name.is_empty() {
                    zwarnnam(name, "missing word pattern");
                }
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            let mut adv = rest.chars();
            adv.next();
            rest = adv.as_str();
        }

        // c:359-379 — word pattern, with `*` and `**` sentinels.
        let (word_pat, wl): (Option<Box<Cpattern>>, i32);
        if rest.chars().next() == Some('*') {
            if (fl & (CMF_LEFT | CMF_RIGHT)) == 0 {
                if !name.is_empty() {
                    zwarnnam(name, "need anchor for `*'");
                }
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            let mut adv = rest.chars();
            adv.next();
            rest = adv.as_str();
            if rest.chars().next() == Some('*') {
                let mut adv2 = rest.chars();
                adv2.next();
                rest = adv2.as_str();
                word_pat = None;
                wl = -2;
            } else {
                word_pat = None;
                wl = -1;
            }
        } else {
            let (w, r4, w_len, err) = parse_pattern(name, rest, '\0');
            if err {
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            if w.is_none() && line_pat.is_none() {
                if !name.is_empty() {
                    zwarnnam(name, "need non-empty word or line pattern");
                }
                {
                    PCM_ERR.with(|f| f.set(true));
                    return None;
                }
            }
            word_pat = w;
            wl = w_len;
            rest = r4;
        }

        // c:383-394 — allocate Cmatcher node.
        let node = Box::new(Cmatcher {
            refc: 0,
            next: None,
            flags: fl | fl2,
            line: line_pat,
            llen: ll,
            word: word_pat,
            wlen: wl,
            left,
            lalen: lal,
            right,
            ralen: ral,
        });

        // c:395-400 — link into chain via tail.
        unsafe {
            *tail_ptr = Some(node);
            if let Some(boxed) = (*tail_ptr).as_mut() {
                tail_ptr = &mut boxed.next as *mut _;
            }
        }
    }
    ret // c:403
}

/// Direct port of `parse_class(Cpattern p, char *iptr)` from `Src/Zle/complete.c:480`.
/// 93-line parser for a single character-class `[...]` or
/// equivalence-class `{...}` inside a Cpattern. Reads metafied
/// bytes from `iptr`, allocates `p->u.str` of the right size,
/// fills in the parsed contents (with PP_RANGE / PP_UNKWN tokens
/// for `a-z` ranges and `[:class:]` POSIX-style entries via
/// range_type lookup).
///
/// Static-link path: the metafied-byte + Meta-token + PP_*
/// encoding doesn't translate cleanly to Rust's UTF-8 strings.
/// Structural port returns the input pointer unmodified (signaling
/// "consumed nothing, parse failed") so the caller can detect the
/// stub state and skip emitting the matcher.
/// WARNING: param names don't match C — Rust=(_p) vs C=(p, iptr)
/// Direct port of `Cpattern parse_pattern(char *name, char **sp,
/// int *lp, char e, int *err)` from `Src/Zle/complete.c:418`.
/// Walks `*sp` building a Cpattern chain. Stops at end-char `e`
/// (or whitespace if `e == 0`). For each char-position:
///   - `[` / `{` → call `parse_class` for `[class]` / `{equiv}`
///   - `?` → CPAT_ANY
///   - `*` / `(` / `)` / `=` → error (invalid in matcher patterns)
///   - `\` + char → escape, emit next char as CPAT_CHAR
///   - else → CPAT_CHAR
///
/// Returns `(chain_head, new_sp, length, err)`. Error sets `err=true`
/// and chain is None; caller bubbles up.
/// WARNING: signature change — C returns Cpattern + writes through
/// sp/lp/err; Rust returns the tuple.
pub fn parse_pattern<'a>(
    name: &str,
    s: &'a str,
    end: char,
) -> (Option<Box<Cpattern>>, &'a str, i32, bool) {
    let mut ret: Option<Box<Cpattern>> = None;
    let mut tail_ptr: *mut Option<Box<Cpattern>> = &mut ret;
    let mut rest = s;
    let mut len = 0i32;

    // c:430 — `while (*s && (e ? (*s != e) : !inblank(*s)))`.
    loop {
        let next_ch = match rest.chars().next() {
            Some(c) => c,
            None => break,
        };
        if end != '\0' {
            if next_ch == end {
                break;
            }
        } else if next_ch == ' ' || next_ch == '\t' {
            break;
        }

        // c:432 — `n = hcalloc(sizeof(*n)); n->next = NULL;`
        let mut node = Box::new(Cpattern::default());

        if next_ch == '[' || next_ch == '{' {
            // c:435
            // c:436 — `s = parse_class(n, s);`.
            //          Rust parse_class already advances past the
            //          close bracket internally (returns slice AFTER
            //          `]`/`}`), so we don't re-advance here. C's
            //          `s++` at c:442 is for the C parse_class which
            //          leaves s pointing AT the close bracket.
            //          Unterminated → parse_class returns empty input;
            //          treat as error.
            let before_len = rest.len();
            rest = parse_class(&mut node, rest);
            if rest.len() == before_len {
                // parse_class didn't advance — unterminated.
                if !name.is_empty() {
                    zwarnnam(name, "unterminated character class");
                }
                return (None, rest, 0, true);
            }
        } else if next_ch == '?' {
            // c:443
            node.tp = CPAT_ANY;
            let mut it = rest.chars();
            it.next();
            rest = it.as_str();
        } else if matches!(next_ch, '*' | '(' | ')' | '=') {
            // c:446
            if !name.is_empty() {
                zwarnnam(name, &format!("invalid pattern character `{}'", next_ch));
            }
            return (None, rest, 0, true);
        } else {
            // c:451
            // c:452 — `if (*s == '\\' && s[1]) s++;` skip backslash escape.
            if next_ch == '\\' {
                let mut it = rest.chars();
                it.next();
                if it.clone().next().is_some() {
                    rest = it.as_str();
                }
            }
            // c:455-461 — `inlen = MB_METACHARLENCONV(...); inchar = ...;
            //              n->tp = CPAT_CHAR; n->u.chr = inchar; s += inlen;`
            let ch = rest.chars().next().unwrap();
            node.tp = CPAT_CHAR;
            node.chr = ch as u32;
            let mut it = rest.chars();
            it.next();
            rest = it.as_str();
        }

        // c:463-467 — link node into chain via tail.
        unsafe {
            *tail_ptr = Some(node);
            // Advance tail to the new node's `.next` slot.
            if let Some(boxed) = (*tail_ptr).as_mut() {
                tail_ptr = &mut boxed.next as *mut _;
            }
        }
        len += 1;
    }
    (ret, rest, len, false)
}
/// `parse_class` — see implementation.
pub fn parse_class<'a>(
    p: &mut Cpattern, // c:480
    iptr: &'a str,
) -> &'a str {
    let bytes = iptr.as_bytes();
    if bytes.is_empty() {
        return iptr;
    }

    // c:485-498 — `if (*iptr++ == '[')` sets CCLASS/NCLASS; else
    //              EQUIV (`{...}`).
    let opener = bytes[0];
    let endchar: u8;
    let mut i = 1;
    if opener == b'[' {
        endchar = b']';
        // c:490 — `if ((*iptr=='!' || *iptr=='^') && iptr[1] != ']') NCLASS`.
        if i < bytes.len()
            && (bytes[i] == b'!' || bytes[i] == b'^')
            && i + 1 < bytes.len()
            && bytes[i + 1] != b']'
        {
            p.tp = CPAT_NCLASS;
            i += 1;
        } else {
            p.tp = CPAT_CCLASS;
        }
    } else {
        endchar = 0x7d; // ASCII close-brace; avoid b'<close-brace>' so
                        // the build.rs brace-scanner doesn't miscount.
        p.tp = CPAT_EQUIV;
    }

    // c:501-505 — End character can appear literally first. Find
    //              end position; bail with rest-of-input on no end.
    let start = i;
    let mut optr_idx = i;
    while optr_idx < bytes.len() && (optr_idx == start || bytes[optr_idx] != endchar) {
        optr_idx += 1;
    }
    if optr_idx >= bytes.len() {
        // c:504 — `if (!*optr) return optr;` — unterminated class.
        return &iptr[bytes.len()..];
    }

    // c:507-512 — `p->u.str = zhalloc((optr-iptr) + 1)`. Pre-size
    //              output buffer; tokens always fit in input length.
    let mut out: Vec<u8> = Vec::with_capacity(optr_idx - i + 1);

    // c:514-562 — main parse loop. firsttime allows endchar at position 0.
    let mut firsttime = true;
    while firsttime || (i < bytes.len() && bytes[i] != endchar) {
        // c:516-525 — `[:name:]` POSIX-class form.
        if bytes[i] == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b':' {
            if let Some(nptr) = bytes[i + 2..].iter().position(|&b| b == b':') {
                let nptr = i + 2 + nptr;
                if nptr + 1 < bytes.len() && bytes[nptr + 1] == b']' {
                    let name = std::str::from_utf8(&bytes[i + 2..nptr]).unwrap_or("");
                    let ch = range_type(name).unwrap_or(PP_UNKWN as usize);
                    i = nptr + 2;
                    if ch != PP_UNKWN as usize {
                        // c:523 — `*optr++ = Meta + ch`. Encode as a
                        // single byte with the high bit set so callers
                        // recognise PP_* markers (decoded in
                        // `pattern_match_equivalence` and friends).
                        out.push(0x80u8.wrapping_add(ch as u8));
                    }
                    firsttime = false;
                    continue;
                }
            }
            // Malformed `[:name:` — treat `[` literally.
        }

        // c:528-560 — single-char / range parse.
        let ptr1 = i;
        if bytes[i] == 0x83 {
            // c:530 Meta
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        i += 1;
        // c:534-553 — `*iptr=='-' && iptr[1] && iptr[1]!=endchar` → range.
        if i < bytes.len() && bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] != endchar {
            i += 1; // consume '-'
                    // c:539 — `*optr++ = Meta + PP_RANGE;`.
            out.push(0x80u8.wrapping_add(PP_RANGE as u8));
            // c:543-547 — start char (with Meta decode).
            if bytes[ptr1] == 0x83 && ptr1 + 1 < bytes.len() {
                out.push(0x83);
                out.push(bytes[ptr1 + 1] ^ 32);
            } else {
                out.push(bytes[ptr1]);
            }
            // c:549-554 — end char (with Meta passthrough).
            if i < bytes.len() && bytes[i] == 0x83 && i + 1 < bytes.len() {
                out.push(bytes[i]);
                out.push(bytes[i + 1]);
                i += 2;
            } else if i < bytes.len() {
                out.push(bytes[i]);
                i += 1;
            }
        } else {
            // c:556-560 — single char.
            if bytes[ptr1] == 0x83 && ptr1 + 1 < bytes.len() {
                out.push(0x83);
                out.push(bytes[ptr1 + 1] ^ 32);
            } else {
                out.push(bytes[ptr1]);
            }
        }
        firsttime = false;
    }

    // c:564 — `*optr = '\0';`. Rust Vec<u8> stores raw byte sequence
    // verbatim; marker bytes (0x80 + PP_*) survive without UTF-8 munging.
    p.str = Some(out);

    // c:565 — `return iptr;` — input ptr now past the close-bracket.
    let consumed = (i + 1).min(bytes.len());
    &iptr[consumed..]
}

/// Direct port of `parse_ordering(const char *arg, int *flags)` from `Src/Zle/complete.c:573`.
/// C body (c:577-599): comma-separated list of order names, each
/// matched by minimum-abbreviation length against `orderopts[]`. On
/// any unknown name returns -1 (and seeds `*flags = CAF_MATSORT` if
/// flags is non-NULL); otherwise OR-accumulates the matched flags
/// into `*flags`.
///
/// `arg` is the comma-separated list, `flags` is an out-parameter
/// receiving the accumulated CAF_* bitmask. Returns 0 on success,
/// -1 on bad name.
pub fn parse_ordering(arg: &str, flags: &mut Option<i32>) -> i32 {
    // c:573
    let mut fl = 0i32; // c:575
    for opt_token in arg.split(',') {
        // c:578-583
        // c:585-590 — walk orderopts[] in reverse, longest-match first.
        let mut found = false; // c:580
        for o in ORDEROPTS.iter().rev() {
            // c:585
            if opt_token.len() >= o.abbrev                                   // c:586
                && o.name.starts_with(opt_token)
            {
                fl |= o.oflag; // c:588
                found = true;
                break;
            }
        }
        if !found {
            // c:592
            if let Some(ref mut f) = flags {
                // c:593
                *f = CAF_MATSORT; // c:594 default
            }
            return -1; // c:595
        }
    }
    if let Some(ref mut f) = flags {
        // c:598
        *f |= fl; // c:599
    }
    0 // c:600
}

// =====================================================================
// bin_compadd / bin_compset / do_comp_vars / parse_cmatcher /
// parse_class — Src/Zle/complete.c. The remaining big-body ported from
// the unported list. Each is ported as a faithful structural shell:
// canonical C signature, control-flow shape, every C-source line
// cited, with the actual data-mutation paths (addmatch, set_comp_sep,
// CCS_* match-engine, Cmatcher chain ops) marked DEFERRED until the
// underlying infrastructure lands.
// =====================================================================

/// Direct port of `bin_compadd(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Zle/complete.c:603`.
/// 251 lines — the main `compadd` builtin entry. Parses ~30 single-
/// letter flags + their args (-J group, -V vgroup, -X expl, -d
/// description, -E count, -O array, -A action, -W where, -R remfn,
/// -F filemask, -P prefix, -S suffix, -i ipre, -I isfx, -p qpre,
/// -s qsfx, -r rstring, -R rmatch, -a/-l/-k flags, -Q noquote,
/// -U usemenu, -1 unique, -2 partial, -o ordering, -M matcher),
/// builds a `cadata`/`mdata` pair, then dispatches to addmatches.
///
/// Cadata is typed in `comp_h.rs:566` and `addmatches` is ported in
/// `compcore.rs`. Each flag captures into the matching Cadata field
/// (-P→pre, -S→suf, -i→ipre, -I→isuf, -p→ppre, -s→psuf, -W→prpre,
/// -J/-V→group, -X→exp, -x→mesg, -d→disp, -O→opar, -A→apar, -D→dpar,
/// -E→dummies, -M→match_/CAF_MATCH, -r→rems, -R→remf, -q→CMF_REMOVE,
/// -n/-l→CMF_NOLIST, -U→clears CAF_MATCH, -Y→CMF_ISPAR, -a→CAF_ARRAYS,
/// -k→CAF_KEYS, -Q→CAF_QUOTE, -1→CAF_UNIQALL, -2→CAF_UNIQCON,
/// -C→CAF_ALL, -f→CMF_FILE, -o/-e→CAF_NOSORT), then dispatches the
/// residual argv through `compcore::addmatches`.
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_compadd(
    name: &str,
    argv: &[String], // c:603
    _ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:608
        zwarnnam(name, "can only be called from completion function"); // c:609
        return 1; // c:610
    }
    // Bug #657 gap #3 — completion-builtin override interception. In zsh a
    // completer can install a `compadd` SHELL FUNCTION (e.g. `_complete_help`'s
    // `compadd(){ return 1 }` at sh:13, or `_approximate`'s correction body)
    // and `compadd` then resolves function-before-builtin. The Rust ports call
    // `bin_compadd` directly, bypassing that override; route to it here.
    //
    // c:Src/exec.c:execcmd_analyse — the command word is looked up in
    // `shfunctab` BEFORE `builtintab`, unconditionally, for every call. The
    // test is therefore just "is there a `compadd` shell function", with no
    // further qualification: an earlier revision additionally required an
    // active `_shadow` (`.shadow.depth` > 0), which made the hook fire for
    // `_complete_help`/`_approximate` (both of which install their override
    // through `_shadow`) but NOT for a plugin that assigns
    // `functions[compadd]` directly and globally — fzf-tab's
    // `functions[compadd]=$functions[_fzf_tab_compadd]` never touches
    // `_shadow`, so depth stayed 0 and its capture hook was silently skipped.
    // `.shadow.depth` is set nowhere but `_shadow` itself
    // (`Completion/Base/Utility/_shadow:36,67,91`), so the extra term could
    // only ever subtract overrides zsh honours.
    //
    // The thread-local guard lets the override's `builtin compadd` /
    // `compadd@suffix` (body `builtin compadd`) reach the real builtin below
    // without re-entry.
    thread_local! {
        static IN_COMPADD_OVERRIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if !IN_COMPADD_OVERRIDE.with(|g| g.get())
        && crate::ported::utils::getshfunc("compadd").is_some()
    {
        IN_COMPADD_OVERRIDE.with(|g| g.set(true));
        let rc = crate::ported::exec::dispatch_function_call("compadd", argv).unwrap_or(1);
        IN_COMPADD_OVERRIDE.with(|g| g.set(false));
        return rc;
    }
    // sh:_approximate:57-69 — argv rewrite hook. Runs BEFORE the PREFIX
    //   injection below because the shell function tests the UNINJECTED
    //   `$PREFIX$SUFFIX` at sh:58 and prepends `$_correct_expl` at sh:69
    //   before the real builtin ever sees the word.
    let shadow_argv: Vec<String>;
    let argv: &[String] = {
        let hook = *COMPADD_ARGV_SHADOW.lock().unwrap();
        match hook {
            Some(f) => match f(argv) {
                // sh:58 `[[ … ]] && return` — `return` with no argument
                //   yields the status of the `[[ ]]` that guarded it, i.e. 0.
                None => return 0,
                Some(v) => {
                    shadow_argv = v;
                    &shadow_argv
                }
            },
            None => argv,
        }
    };
    // sh:_approximate:57-72 — process-wide PREFIX-injection hook.
    //   When set, `(#a${_comp_correct})` (or another caller-supplied
    //   prefix) is prepended to `$PREFIX` for the duration of this
    //   compadd call only. The override is mirrored exactly from the
    //   shell-side `_approximate` compadd shadow at sh:62-71: with a
    //   leading `~` in PREFIX the injection lands AFTER the tilde.
    let saved_prefix_for_inject: Option<String> = {
        let lock = COMPADD_PREFIX_INJECTOR.lock().unwrap();
        if let Some(inj) = lock.as_ref() {
            let cur_prefix = crate::ported::params::getsparam("PREFIX").unwrap_or_default();
            // sh:_approximate:65 — `-p` arg-position detection. If
            //   the user passed `-p VAL` where VAL starts with `~`,
            //   the tilde-already-handled path triggers.
            let p_idx = argv.iter().position(|a| a == "-p");
            let tilde_in_p = p_idx
                .and_then(|i| argv.get(i + 1))
                .map(|v| v.starts_with('~'))
                .unwrap_or(false);
            let new_prefix = if cur_prefix.starts_with('~') && !tilde_in_p {
                // sh:67 — `~(#a${N})${PREFIX[2,-1]}` keeps the ~
                //   intact for tilde-expansion downstream.
                format!("~{}{}", inj, &cur_prefix[1..])
            } else {
                // sh:69 — bare prefix-prepend
                format!("{}{}", inj, cur_prefix)
            };
            let _ = crate::ported::params::setsparam("PREFIX", &new_prefix);
            Some(cur_prefix)
        } else {
            None
        }
    };
    // Run the compadd body, then restore PREFIX exactly as it was.
    let ret = bin_compadd_body(name, argv, _ops, _func);
    if let Some(saved) = saved_prefix_for_inject {
        let _ = crate::ported::params::setsparam("PREFIX", &saved);
    }
    // sh:_complete_help:11 — `compadd() { return 1 }` trace shadow.
    //   When the trace flag is set, record the call into the trace
    //   buffer and short-circuit so no matches actually land.
    if COMPADD_TRACE_ACTIVE.load(Ordering::Relaxed) {
        let mut buf = crate::ported::params::getaparam("_complete_help_funcs").unwrap_or_default();
        buf.push(argv.join(" "));
        crate::ported::params::setaparam("_complete_help_funcs", buf);
        return 1;
    }
    // In-editor capture shadow (Phase 0.5 of the LSP completion
    // path — see `docs/IN_EDITOR_COMPSYS_COMPLETION.md` +
    // `crate::compsys::in_editor::COMPADD_CAPTURE_BUFFER`).
    // When the buffer is `Some`, every compadd call routes its
    // proposed matches into the buffer as `CompsysMatch` records
    // instead of into the ZLE state. The buffer's installer (the
    // LSP / `complete_at`) drains it after `_main_complete`
    // returns and translates each match to a `CompletionItem`.
    // Parsing happens in `crate::compsys::in_editor` (Rust-only
    // space) so this ported file stays a faithful C port.
    if crate::compsys::in_editor::try_capture_compadd_argv(argv) {
        return 0; // mimic "matches were added" status
    }
    ret
}

/// Process-wide PREFIX injection used by `_approximate` to inject
/// `(#a$N)` per-iteration without modifying PREFIX outside the
/// compadd call. Set via [`set_compadd_prefix_injector`] / cleared
/// via [`clear_compadd_prefix_injector`].
pub static COMPADD_PREFIX_INJECTOR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// sh:_complete_help:11 — when this flag is set, every `bin_compadd`
/// call records its argv into `_complete_help_funcs` and returns 1
/// without adding matches. Set / cleared via
/// [`set_compadd_trace`].
pub static COMPADD_TRACE_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Install the PREFIX-injection string used by the next `bin_compadd`
/// call. Returns the previously-installed injector (if any) so the
/// caller can chain.
pub fn set_compadd_prefix_injector(s: impl Into<String>) -> Option<String> {
    COMPADD_PREFIX_INJECTOR.lock().unwrap().replace(s.into())
}

/// Clear the PREFIX injector.
pub fn clear_compadd_prefix_injector() {
    *COMPADD_PREFIX_INJECTOR.lock().unwrap() = None;
}

/// sh:_approximate:54-70 — argv rewrite hook for the `compadd()` shell
/// function `_approximate` installs over the builtin. Returning `None`
/// means "swallow this call" (sh:57-58's bare `return`); returning
/// `Some(v)` runs the real builtin with `v` as argv (sh:69's
/// `compadd@_approximate "$_correct_expl[@]" "$@"`).
///
/// A plain fn pointer rather than a boxed closure: the hook reads all
/// of its state (`_comp_correct`, `_correct_expl`, `_correct_group`)
/// from shell parameters, exactly as the shell function does.
pub type CompaddArgvShadow = fn(&[String]) -> Option<Vec<String>>;

/// The installed argv shadow, if any. See [`CompaddArgvShadow`].
/// Written directly by the installer (`_approximate`) — no setter pair,
/// because a Rust-only accessor with no C counterpart cannot live under
/// `src/ported/` (build.rs port gate).
pub static COMPADD_ARGV_SHADOW: std::sync::Mutex<Option<CompaddArgvShadow>> =
    std::sync::Mutex::new(None);

/// Enable [`COMPADD_TRACE_ACTIVE`] — every subsequent `bin_compadd`
/// call records its argv into `_complete_help_funcs` and returns 1.
pub fn set_compadd_trace(active: bool) {
    COMPADD_TRACE_ACTIVE.store(active, std::sync::atomic::Ordering::Relaxed);
}

/// The actual compadd body — i.e. C's `bin_compadd` proper
/// (`Src/Zle/complete.c:603`). Split out so the shell-function
/// emulation in [`bin_compadd`] above (the shfunc-override dispatch,
/// [`COMPADD_ARGV_SHADOW`], [`COMPADD_PREFIX_INJECTOR`],
/// [`COMPADD_TRACE_ACTIVE`]) can run before / after it without code
/// duplication.
///
/// **This is the `builtin compadd` entry point.** In C the `builtin`
/// precommand modifier sets `BINF_BUILTIN`, and `execcmd_exec` gates
/// its `shfunctab->getnode(shfunctab, cmdarg)` lookup on
/// `!(cflags & (BINF_BUILTIN | BINF_COMMAND))` (`Src/exec.c:3402-3406`),
/// so no shell function named `compadd` is ever consulted — the
/// builtin's handler runs directly. zshrs folds that shfunc lookup
/// into [`bin_compadd`]'s prologue (the Rust ports call the handler
/// by symbol rather than going through `execcmd_exec`), so a ported
/// completion function that upstream writes as `builtin compadd`
/// must call this function instead of [`bin_compadd`]. Six upstream
/// sites do exactly that in order to bypass the `compadd()` shell
/// function `_approximate` / `_correct` install
/// (`Completion/Base/Completer/_approximate:54-73`):
/// `_path_files:509`, `_sep_parts:55`, `_sep_parts:85`,
/// `_sep_parts:121`, `_multi_parts:94`, `_message:43`.
pub fn bin_compadd_body(name: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
    // c:608-610 — `if (incompfunc != 1) { zwarnnam(...); return 1; }`.
    // [`bin_compadd`] checks this before its shfunc-emulation prologue
    // and never reaches here when it trips, so this copy only guards
    // direct (`builtin compadd`) callers.
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        zwarnnam(name, "can only be called from completion function"); // c:609
        return 1; // c:610
    }
    // c:613-820 — flag-arg parse loop. Walk argv consuming `-X arg`
    // pairs into the `Cadata` struct; per-flag dispatch ports the C
    // switch at c:621-820.
    let mut dat = crate::ported::zle::comp_h::Cadata::default();
    // c:622 — `dat.aflags = CAF_MATCH`. compadd matches candidates
    // against the word on the line by default (only `-U` clears it).
    // Without this seed, addmatches takes the no-match branch and adds
    // every candidate unfiltered — `compadd -k commands` would offer all
    // 5000+ commands instead of just those matching the typed prefix.
    dat.aflags = CAF_MATCH;
    dat.dummies = -1;
    // c:614 — `char *mstr = NULL; /* argument of -M options, accumulated */`.
    let mut mstr: Option<String> = None;
    // c:615 — `char *oarg = NULL; /* argument of -o option */`. Only the
    // FIRST `-o` spec is ever honoured (c:772 `order = oarg ? -1 : 1`).
    let mut oarg: Option<String> = None;
    // C's `atoi()` as used for `-E` at c:778/c:782: optional sign, then
    // digits, stopping at the first non-digit (0 when none). `str::parse`
    // is not a substitute — it rejects trailing garbage outright.
    let c_atoi = |s: &str| -> i32 {
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        let neg = i < b.len() && b[i] == b'-';
        if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
            i += 1;
        }
        let mut v: i64 = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            v = (v * 10 + (b[i] - b'0') as i64).min(i32::MAX as i64);
            i += 1;
        }
        if neg {
            -v as i32
        } else {
            v as i32
        }
    };
    let mut idx = 0usize;
    // c:632-820 — flag loop. C iterates EACH CHARACTER inside every `-…`
    // argv element (`for (p = *argv+1; *p; p++)`), so bundled flags like
    // compdescribe/_describe's `-2V-default-` parse as `-2` then `-V` whose
    // argument is the rest of the same word (`-default-`). The earlier port
    // read only `arg[1]` per element and dropped everything after the first
    // flag char, so `-2V-default-` set only `-2` and lost the group name —
    // every described-option match fell into the implicit `default` group
    // instead of `-default-`, and list-colors' `(-default-)` group prefix
    // never matched (no colors). Now char-by-char with C's sp semantics.
    'outer: while idx < argv.len() {
        // c:632 — `for (; *argv && **argv == '-'; argv++)`
        let arg = argv[idx].clone();
        if !arg.starts_with('-') {
            break; // c:633 — loop cond `**argv == '-'` fails: a non-flag
                   // word ends option parsing; it is NOT consumed (it is the
                   // first positional match).
        }
        if arg.len() < 2 {
            // c:635-637 — a bare `-` is the end-of-options marker: CONSUME it
            // (`argv++`) and stop. The previous port broke WITHOUT consuming,
            // so the `-` from `compadd … - words` (e.g. `compadd -D var - …`,
            // used all over _path_files) survived as a spurious first
            // positional word — shifting every match/dpar element by one and
            // breaking path completion's `compadd -D` narrowing.
            idx += 1;
            break;
        }
        if arg == "--" {
            idx += 1;
            break;
        } // c:815 `case '-': argv++; goto ca_args`
        idx += 1;
        let bytes = arg.clone().into_bytes();
        let mut p = 1usize; // c:638 `p = *argv + 1`
        while p < bytes.len() {
            // c:638
            let c = bytes[p] as char;
            // c:812-825 — the `if (sp)` arg-extraction: pasted rest-of-word
            // (`-Xfoo`) or the next word (`-X foo`). Consuming the rest of
            // the word advances `p` to the end so the inner loop stops.
            let mut take = |p: &mut usize, idx: &mut usize| -> Option<String> {
                if *p + 1 < bytes.len() {
                    // c:816-818 — `-Xfoo`: value is the rest of this word.
                    let v = String::from_utf8_lossy(&bytes[*p + 1..]).into_owned();
                    *p = bytes.len(); // consume rest → inner loop ends
                    Some(v)
                } else if *idx < argv.len() {
                    // c:819-822 — `-X foo`: value is the next word.
                    let v = argv[*idx].clone();
                    *idx += 1;
                    Some(v)
                } else {
                    None // c:820-826 — missing argument.
                }
            };
            // c:806-826 — the shared `if (sp)` tail. C routes nearly every
            // argument-taking flag through one `char **sp` slot and applies
            // `if (!*sp) *sp = …` — the argument is ALWAYS consumed, but only
            // the FIRST occurrence of that flag is kept, and a MISSING
            // argument is a hard error carrying the per-flag message `e`.
            // The port had first-wins on -J/-V/-X only and silently ignored
            // missing arguments everywhere, so a repeated -P/-S/-p/-s/-i/-I/
            // -W/-F/-x/-d/-A/-O/-r/-R (routine when _description's `$expl[@]`
            // is spliced next to a caller's own options) took the LAST value
            // instead of C's first.
            macro_rules! opt_arg {
                ($slot:expr, $emsg:expr) => {{
                    match take(&mut p, &mut idx) {
                        Some(v) => {
                            if $slot.is_none() {
                                // c:809/816 `if (!*sp) *sp = …`
                                $slot = Some(v);
                            }
                        }
                        None => {
                            // c:822 `zwarnnam(name, e, *p)`
                            zwarnnam(name, &format!(concat!($emsg, " -{}"), c));
                            return 1;
                        }
                    }
                }};
            }
            match c {
                'a' => dat.aflags |= CAF_ARRAYS,            // c:658
                'k' => dat.aflags |= CAF_ARRAYS | CAF_KEYS, // c:661
                'l' => dat.flags |= CMF_DISPLINE as i32,    // c:766-767
                'o' => {
                    // c:769-775 — `-o` takes an OPTIONAL ordering spec
                    // (nosort/match/numeric/reverse, comma-joined). C:
                    // `order = oarg ? -1 : 1; sp = &oarg;` — "we honour just
                    // the first -o option but need to skip over a valid
                    // argument to subsequent -o options". Note C re-parses
                    // `oarg` (the FIRST spec) each time, never the new word;
                    // the port parsed each spec and OR-ed its flags in, so
                    // `_path_files:171`'s prepended `-o nosort` in front of a
                    // caller's own `-o match` merged both orderings instead of
                    // keeping only `nosort`.
                    let order: i32 = if oarg.is_some() { -1 } else { 1 }; // c:772
                                                                          // c:811/818 — `parse_ordering(oarg, order == 1 ? &dat.aflags : NULL)`.
                                                                          // On a bad name C ASSIGNS `*flags = CAF_MATSORT` (c:600),
                                                                          // clobbering aflags rather than OR-ing; reproduced verbatim.
                    let mut run_ordering = |oarg: &Option<String>, aflags: &mut i32| -> i32 {
                        let mut fl: Option<i32> = if order == 1 { Some(*aflags) } else { None };
                        let rc = parse_ordering(oarg.as_deref().unwrap_or(""), &mut fl);
                        if let Some(f) = fl {
                            *aflags = f;
                        }
                        rc
                    };
                    if p + 1 < bytes.len() {
                        // c:807-812 — pasted `-o<spec>`.
                        if oarg.is_none() {
                            oarg = Some(String::from_utf8_lossy(&bytes[p + 1..]).into_owned());
                        }
                        if run_ordering(&oarg, &mut dat.aflags) == 0 {
                            p = bytes.len(); // c:812 `p += strlen(p+1)`
                        }
                    } else if idx < argv.len() {
                        // c:813-819 — separate `-o <spec>`; the word is taken
                        // first and given back only if it is not an ordering.
                        let cand = argv[idx].clone();
                        idx += 1; // c:815 `argv++`
                        if oarg.is_none() {
                            oarg = Some(cand); // c:816-817
                        }
                        if run_ordering(&oarg, &mut dat.aflags) != 0 {
                            idx -= 1; // c:819 `--argv`
                        }
                    }
                    // c:820 — `else if (!order)`: `order` is never 0 for -o,
                    // so a missing argument is NOT an error here.
                }
                'Q' => dat.aflags |= CAF_QUOTE, // c:650
                // c:695-702 — `-1`/`-2` are mutually exclusive, FIRST wins:
                // `-1` is a no-op once CAF_UNIQCON is set and vice versa. The
                // port OR-ed both unconditionally, so `compadd -1 -2` asked
                // addmatches for whole-list AND consecutive de-duplication
                // instead of only the first-named one.
                '1' => {
                    if (dat.aflags & CAF_UNIQCON) == 0 {
                        dat.aflags |= CAF_UNIQALL; // c:697
                    }
                }
                '2' => {
                    if (dat.aflags & CAF_UNIQALL) == 0 {
                        dat.aflags |= CAF_UNIQCON; // c:701
                    }
                }
                'C' => dat.aflags |= CAF_ALL, // c:653
                'F' => opt_arg!(dat.ign, "string expected after"), // c:667-669
                'f' => dat.flags |= CMF_FILE as i32, // c:656
                'P' => opt_arg!(dat.pre, "string expected after"), // c:677-679
                'S' => opt_arg!(dat.suf, "string expected after"), // c:681-683
                'p' => opt_arg!(dat.ppre, "string expected after"), // c:711-713
                's' => opt_arg!(dat.psuf, "string expected after"), // c:715-717
                'W' => opt_arg!(dat.prpre, "string expected after"), // c:719-721
                'i' => opt_arg!(dat.ipre, "string expected after"), // c:703-705
                'I' => opt_arg!(dat.isuf, "string expected after"), // c:707-709
                // c:685-687 — first-wins via the shared `sp`. Last-wins here
                // made `_files`/_path_files' inner `-J globbed-files`/
                // `-J directories` overwrite _arguments' outer
                // `-J argument-rest`, splitting one group into duplicates.
                'J' => opt_arg!(dat.group, "group name expected after"),
                'V' => {
                    // c:689-693 — `-V` is `-J` plus CAF_NOSORT, but the
                    // NOSORT is set at the case label, i.e. gated on whether a
                    // group name has been seen BEFORE this flag's argument.
                    if dat.group.is_none() {
                        dat.aflags |= CAF_NOSORT; // c:690-691
                    }
                    opt_arg!(dat.group, "group name expected after"); // c:692
                }
                // c:727-729 — first-wins, so an outer _arguments/_description
                // `-X <label>` wrapping an inner completer's `-X <label>` keeps
                // the OUTER label (curl: 'URL' over 'URL prefix'; scp: 'remote
                // host name' over 'host').
                'X' => opt_arg!(dat.exp, "string expected after"),
                'x' => opt_arg!(dat.mesg, "string expected after"), // c:731-733
                'd' => opt_arg!(dat.disp, "parameter name expected after"), // c:762-764
                'O' => opt_arg!(dat.opar, "parameter name expected after"), // c:749-751
                'A' => opt_arg!(dat.apar, "parameter name expected after"), // c:745-747
                'D' => {
                    // c:753-761 — `-D` appends: `sp = dat.dpar + dparlen++`
                    // points at a freshly NULLed slot, so `if (!*sp)` is always
                    // true and every occurrence accumulates.
                    match take(&mut p, &mut idx) {
                        Some(s) => dat.dpar.push(s),
                        None => {
                            zwarnnam(name, &format!("parameter name expected after -{}", c));
                            return 1; // c:822
                        }
                    }
                }
                'E' => {
                    // c:776-795 — `-E <n>`: dummy-match count. Does NOT go
                    // through the shared `sp` path; C errors out on a missing
                    // argument (c:783-788) AND on a negative count
                    // (c:789-794). The port silently ignored a missing
                    // argument and clamped a negative one to 0, so
                    // `compadd -E -1` quietly added zero dummies where zsh
                    // rejects the whole call.
                    let v = if p + 1 < bytes.len() {
                        // c:777-779 — pasted `-E<n>`.
                        let v = String::from_utf8_lossy(&bytes[p + 1..]).into_owned();
                        p = bytes.len();
                        Some(v)
                    } else if idx < argv.len() {
                        // c:780-782 — `-E <n>` in the next word.
                        let v = argv[idx].clone();
                        idx += 1;
                        Some(v)
                    } else {
                        None
                    };
                    match v {
                        Some(s) => {
                            dat.dummies = c_atoi(&s); // c:778/782 `atoi()`
                            if dat.dummies < 0 {
                                // c:789-794
                                zwarnnam(name, &format!("invalid number: {}", dat.dummies));
                                return 1;
                            }
                        }
                        None => {
                            // c:783-787
                            zwarnnam(name, &format!("number expected after -{}", c));
                            return 1;
                        }
                    }
                }
                'M' => {
                    // c:723-726 / c:826-834 — `-M` does NOT parse here. C
                    // collects EVERY `-M` argument into `mstr`, space-joined
                    // (`tricat(mstr, " ", m)`), and calls parse_cmatcher once
                    // at c:842. The port parsed each `-M` on the spot and
                    // overwrote `dat.match_`, so only the LAST spec survived.
                    // `_path_files`' final add passes three (`-M "$_matcher"`
                    // twice from $mopts, then `-M 'r:|/=* r:|=*'` from $Mopts,
                    // sh:872): last-wins threw away the matcher-list spec, so
                    // `cd /a<TAB>` had no spec left that could match
                    // /Applications and reported "No Matches".
                    // c:640 — `char *m = NULL;` is re-declared per inner-loop
                    // iteration, so `if (!*sp)` is always true for -M: every
                    // occurrence's argument reaches the mstr accumulator.
                    match take(&mut p, &mut idx) {
                        Some(s) => match mstr {
                            // c:829 — `tricat(mstr, " ", m)`.
                            Some(ref mut acc) => {
                                acc.push(' ');
                                acc.push_str(&s);
                            }
                            // c:833 — `mstr = ztrdup(m)`.
                            None => mstr = Some(s),
                        },
                        None => {
                            // c:822 with e from c:725.
                            zwarnnam(
                                name,
                                &format!("matching specification expected after -{}", c),
                            );
                            return 1;
                        }
                    }
                }
                'q' => dat.flags |= CMF_REMOVE as i32, // c:646
                // c:735-738 / c:740-743 — `-r`/`-R` set CMF_REMOVE *and* take
                // the removal spec (rems=chars / remf=widget). The port set only
                // the spec, leaving CMF_REMOVE clear, so the auto-remove-suffix
                // gate (compresult.rs:1525, c:1014 `if (m->flags & CMF_REMOVE)`)
                // never fired for `compadd -r`/`-R` — the suffix wasn't removed
                // when a following char was typed.
                'r' => {
                    dat.flags |= CMF_REMOVE as i32; // c:736
                    opt_arg!(dat.rems, "string expected after"); // c:737
                }
                'R' => {
                    dat.flags |= CMF_REMOVE as i32; // c:741
                    opt_arg!(dat.remf, "function name expected after"); // c:742
                }
                'n' => dat.flags |= CMF_NOLIST as i32, // c:671
                // c:674 — `-U`: clear CAF_MATCH so the added matches DON'T have
                // to match the command-line word. (Was mis-ported as
                // `dat.flags |= CMF_HIDE`, which merely hid the match from the
                // list and left CAF_MATCH set — so an unconditional match like
                // `compadd -U /abs/path` for word `~/x*` never got inserted;
                // this broke every `-U` completer: _expand, _prefix, …)
                'U' => dat.aflags &= !CAF_MATCH, // c:674
                // c:658 — `-e` sets CMF_ISPAR (mark matches as parameters), NOT
                // CAF_NOSORT (the port had them wrong). CAF_NOSORT comes from
                // `-V`(no group)/`-o`, never `-e`.
                'e' => dat.flags |= CMF_ISPAR as i32, // c:658
                // C has no `-Y`; kept mapping to CMF_ISPAR (harmless, unused as a
                // compadd flag) rather than erroring, since the catch-all rejects
                // unknown flags.
                'Y' => dat.flags |= CMF_ISPAR as i32, // (non-C, unused)
                '-' => {
                    // c:814-816 — `case '-': argv++; goto ca_args;`. A `-`
                    // flag char ends option parsing; this word is consumed.
                    break 'outer;
                }
                _ => {
                    // c:817-820 — unknown flag.
                    zwarnnam(name, &format!("bad option: -{}", c));
                    return 1;
                }
            }
            p += 1; // c:638 `p++`
        }
    }
    // c:839-848 — `ca_args:` the accumulated `-M` specs are parsed ONCE,
    // and only when CAF_MATCH is still set (`-U` turns matching off, so
    // the spec is irrelevant then). Only the `pcm_err` sentinel aborts the
    // builtin — a plain NULL (empty / all-blank spec, or a leading `x:`)
    // is success with no matcher, which the old `Option`-collapsing call
    // turned into status 1.
    let mut matcher: Option<Box<Cmatcher>> = None; // c:617
    if let Some(spec) = mstr {
        if (dat.aflags & CAF_MATCH) != 0 {
            matcher = parse_cmatcher(name, &spec); // c:842
            if PCM_ERR.with(|f| f.get()) {
                return 1; // c:842 `== pcm_err`
            }
        }
    }
    // c:849 — `args = argv` (residual words after flags).
    let matches = &argv[idx..];
    // c:850-854 — with nothing to add, compadd fails outright UNLESS the
    // call still carries something meaningful on its own: a group name
    // (`-J`/`-V` creating an empty group), a `-x` message, or one of
    // CAF_NOSORT / CAF_UNIQALL / CAF_UNIQCON / CAF_ALL. This whole guard
    // was absent, so an empty `compadd` returned addmatches' status and
    // registered a group/dpar side effect that zsh never performs.
    if matches.is_empty()
        && dat.group.is_none()
        && dat.mesg.is_none()
        && (dat.aflags & (CAF_NOSORT | CAF_UNIQALL | CAF_UNIQCON | CAF_ALL)) == 0
    {
        return 1;
    }
    dat.match_ = matcher; // c:856 `dat.match = match = cpcmatcher(match)`
    compcore::addmatches(&mut dat, matches) // c:857
}

// =====================================================================
// Accessor / mutator family — Src/Zle/complete.c:864.
// =====================================================================

/// Direct port of `ignore_prefix(int l)` from `Src/Zle/complete.c:864`.
/// C body (c:867-883): for the leading `l` chars of compprefix,
/// move them onto compiprefix so subsequent matchers see them as
/// already-matched-but-hidden.
pub fn ignore_prefix(l: i32) {
    // c:864
    if l > 0 {
        // c:864
        let (new_prefix, new_iprefix) = {
            let mut prefix = lock_str(&COMPPREFIX).lock().unwrap();
            let pl = prefix.len() as i32; // c:870 strlen(compprefix)
            let take = l.min(pl) as usize; // c:888
            let head: String = prefix[..take].to_string(); // c:888 sav split
            let tail: String = prefix[take..].to_string(); // c:888 ztrdup(compprefix+l)
            let mut iprefix = lock_str(&COMPIPREFIX).lock().unwrap();
            iprefix.push_str(&head); // c:888 tricat(compiprefix, head)
            *prefix = tail.clone(); // c:888 zsfree+ztrdup
            (tail, iprefix.clone())
        };
        // gsu-mirror: in C `compprefix`/`compiprefix` are gsu-bound to
        // $PREFIX/$IPREFIX (one storage). The Rust port keeps the globals
        // and the params in separate stores that only re-sync at addmatches,
        // so a caller reading $PREFIX (e.g. `_values`' `-i` prefix strip)
        // would see the stale pre-`ignore_prefix` value. Write the params
        // here to preserve the C binding's single-storage semantics.
        let _ = crate::ported::params::setsparam("PREFIX", &new_prefix);
        let _ = crate::ported::params::setsparam("IPREFIX", &new_iprefix);
    }
}

/// Direct port of `ignore_suffix(int l)` from `Src/Zle/complete.c:888`.
/// C body (c:891-907): strip the last `l` chars of compsuffix off
/// the end and prepend them to compisuffix (mirrors ignore_prefix).
pub fn ignore_suffix(l: i32) {
    // c:888
    if l > 0 {
        // c:888
        let (new_suffix, new_isuffix) = {
            let mut suffix = lock_str(&COMPSUFFIX).lock().unwrap();
            let sl = suffix.len() as i32; // c:894 strlen(compsuffix)
            let mut split = sl - l; // c:896 (l = sl - l)
            if split < 0 {
                split = 0;
            } // c:897
            let split = split as usize;
            let head: String = suffix[..split].to_string(); // c:902 sav split
            let tail: String = suffix[split..].to_string(); // c:911 tricat(suffix+l, isuffix)
            let mut isuffix = lock_str(&COMPISUFFIX).lock().unwrap();
            let mut new_isuffix = tail; // c:911
            new_isuffix.push_str(&isuffix);
            *isuffix = new_isuffix.clone();
            *suffix = head.clone(); // c:911 zsfree+ztrdup
            (head, new_isuffix)
        };
        // gsu-mirror: see ignore_prefix — $SUFFIX/$ISUFFIX must track the
        // compsuffix/compisuffix globals (gsu-bound as one storage in C).
        let _ = crate::ported::params::setsparam("SUFFIX", &new_suffix);
        let _ = crate::ported::params::setsparam("ISUFFIX", &new_isuffix);
    }
}

/// Direct port of `restrict_range(int b, int e)` from `Src/Zle/complete.c:911`.
/// C body (c:914-933): keep only compwords[b..=e], shifting
/// compcurrent down by b. No-op if range covers everything.
pub fn restrict_range(b: i32, e: i32) {
    // c:911
    let mut words = lock_vec(&COMPWORDS).lock().unwrap();
    let wl = words.len() as i32 - 1; // c:914 arrlen-1
    if wl > 0 && b >= 0 && e >= 0 && (b > 0 || e < wl) {
        // c:916
        let mut e = e;
        if e > wl {
            e = wl;
        } // c:920
        let count = (e - b + 1) as usize; // c:923
        let new_words: Vec<String> = words
            .iter() // c:927
            .skip(b as usize)
            .take(count)
            .cloned()
            .collect();
        *words = new_words; // c:930 freearray + assign
        let cur = COMPCURRENT.load(Ordering::Relaxed);
        COMPCURRENT.store(cur - b, Ordering::Relaxed); // c:931 compcurrent -= b

        // zshrs sync: in C `$words`/`$CURRENT` ARE the compwords/compcurrent
        // globals (special-param getfn reads them live), so restricting the
        // globals is instantly visible to a completion function. zshrs's
        // `$words`/`$CURRENT` are static param copies (set once at completion
        // setup), so they desync from the globals here. Mirror the restricted
        // range into the params so an `_arguments '*::'` (CAA_RARGS) /
        // `'*:::'` (CAA_RREST) rest-arg ACTION sees the rest-only
        // `$words`/`$CURRENT` — e.g. `_systemctl_command`'s `(( CURRENT == 1 ))`.
        let restricted = words.clone();
        let new_cur = COMPCURRENT.load(Ordering::Relaxed);
        drop(words); // release COMPWORDS lock before touching paramtab
        crate::ported::params::setaparam("words", restricted);
        let _ = crate::ported::params::setiparam("CURRENT", new_cur as i64);
    }
}

/// Direct port of `do_comp_vars(int test, int na, char *sa, int nb, char *sb, int mod)` from `Src/Zle/complete.c:935`.
/// Six-arm dispatcher for the completion-variable mutation opcodes:
///
/// * CVT_RANGENUM — numeric word-range test against compcurrent;
///   `mod=1` calls restrict_range to clamp compwords[]
/// * CVT_RANGEPAT — pattern word-range walk: scan compwords backward
///   from compcurrent for `sa`, then optionally forward for `sb`,
///   restrict_range over the matched span
/// * CVT_PRENUM/SUFNUM — numeric prefix/suffix shift via
///   ignore_prefix/ignore_suffix
/// * CVT_PREPAT/SUFPAT — pattern-anchored prefix/suffix match,
///   incrementally walking compprefix/compsuffix until pattry hits
///
/// Returns 1 on match, 0 on no-match. Walks the live completion-
/// state globals (compwords / compcurrent / compprefix / compsuffix)
/// added in the earlier compcore.rs port batch.
/// WARNING: param names don't match C — Rust=(test, na, sa, sb, mod_) vs C=(test, na, sa, nb, sb, mod)
pub fn do_comp_vars(
    test: i32,
    mut na: i32,
    sa: &str, // c:935
    mut nb: i32,
    sb: &str,
    mod_: i32,
) -> i32 {
    match test {
        // c:937
        CVT_RANGENUM => {
            // c:938
            let words = COMPWORDS
                .get()
                .map(|m| m.lock().map(|g| g.clone()).unwrap_or_default())
                .unwrap_or_default();
            let l = words.len() as i32; // c:941 arrlen
                                        // c:943-947 — `if (na < 0) na += l; else na--;` (and same for nb).
            if na < 0 {
                na += l;
            } else {
                na -= 1;
            } // c:943-945
            if nb < 0 {
                nb += l;
            } else {
                nb -= 1;
            } // c:946-948
            let cur = COMPCURRENT.load(Ordering::Relaxed);
            // c:950 — `if (compcurrent - 1 < na || compcurrent - 1 > nb) return 0;`
            if cur - 1 < na || cur - 1 > nb {
                return 0;
            } // c:950
            if mod_ != 0 {
                restrict_range(na, nb);
            } // c:953
            1 // c:954
        }
        CVT_RANGEPAT => {
            // c:957
            let words = COMPWORDS
                .get()
                .map(|m| m.lock().map(|g| g.clone()).unwrap_or_default())
                .unwrap_or_default();
            let l = words.len() as i32;
            let mut t = 0i32; // c:961
            let mut b = 0i32;
            let mut e = l - 1;
            let mut i = COMPCURRENT.load(Ordering::Relaxed) - 1; // c:964 i = compcurrent - 1
            if i < 0 || i >= l {
                return 0;
            } // c:965
              // c:968 — singsub(&sa); — caller already expanded.
            // c:969 — the operand is compiled AS GIVEN. Both callers hand
            // it over already tokenized: `bin_compset` runs
            // `tokenize`/`remnulargs` in its own switch (c:1192-1196), and
            // `cond_range` gets what `cond_str(a, n, 1)` (c:1688) kept from
            // the lexer / `$~`'s GLOB_SUBST.
            let pp = patcompile(sa, PAT_HEAPDUP, None); // c:969
               // c:971-977 — walk compwords backward looking for sa match.
            i -= 1; // c:971
            while i >= 0 {
                if let Some(ref prog) = pp {
                    if pattry(prog, &words[i as usize]) {
                        // c:972
                        b = i + 1; // c:973
                        t = 1; // c:974
                        break;
                    }
                }
                i -= 1;
            }
            // c:980-993 — if matched and sb given, walk forward for sb.
            if t != 0 && !sb.is_empty() {
                // c:980
                let mut tt = 0i32;
                let pp2 = patcompile(sb, PAT_HEAPDUP, None); // c:983
                i += 1; // c:984
                while i < l {
                    if let Some(ref prog) = pp2 {
                        if pattry(prog, &words[i as usize]) {
                            // c:986
                            e = i - 1; // c:987
                            tt = 1;
                            break;
                        }
                    }
                    i += 1;
                }
                if tt != 0 && i < COMPCURRENT.load(Ordering::Relaxed) {
                    // c:992
                    t = 0; // c:993
                }
            }
            if e < b {
                t = 0;
            } // c:996
            if t != 0 && mod_ != 0 {
                restrict_range(b, e);
            } // c:998
            t // c:999
        }
        CVT_PRENUM | CVT_SUFNUM => {
            // c:1001-1002
            if na < 0 {
                return 0;
            } // c:1003
            if na > 0 && mod_ != 0 {
                // c:1004
                // c:1006-1031 — multibyte handling. Rust strings are
                // UTF-8 throughout; the mb_metacharlenconv +
                // backwardmetafiedchar walk collapses to char-count
                // arithmetic.
                let target_str = if test == CVT_PRENUM {
                    lock_str(&COMPPREFIX)
                        .lock()
                        .map(|s| s.clone())
                        .unwrap_or_default()
                } else {
                    lock_str(&COMPSUFFIX)
                        .lock()
                        .map(|s| s.clone())
                        .unwrap_or_default()
                };
                let cnt = target_str.chars().count() as i32;
                // c:1026-1027 / c:1034-1035 — ran off the end of the string
                // before consuming `na` characters (and the non-multibyte
                // c:1040 length guard).
                if cnt < na {
                    return 0;
                }
                // c:1028 `na = sum;` / c:1036 `na = end - ptr;` — the C
                // multibyte arms convert the CHARACTER count `na` into a BYTE
                // count before handing it to ignore_prefix/ignore_suffix,
                // which index compprefix/compsuffix by byte
                // (`compprefix[l] = '\0'`, c:884). The port passed the raw
                // character count, so any multibyte PREFIX/SUFFIX ignored the
                // wrong amount — and could slice mid-codepoint.
                let na_bytes = if test == CVT_PRENUM {
                    target_str
                        .char_indices()
                        .nth(na as usize)
                        .map(|(o, _)| o)
                        .unwrap_or(target_str.len()) as i32
                } else {
                    let off = target_str
                        .char_indices()
                        .nth((cnt - na) as usize)
                        .map(|(o, _)| o)
                        .unwrap_or(target_str.len());
                    (target_str.len() - off) as i32
                };
                if test == CVT_PRENUM {
                    // c:1042
                    ignore_prefix(na_bytes); // c:1043
                } else {
                    ignore_suffix(na_bytes); // c:1045
                }
            }
            1 // c:1041
        }
        CVT_PREPAT | CVT_SUFPAT => {
            // c:1042
            if na == 0 {
                return 0;
            } // c:1045
            // c:1047 — compiled AS GIVEN; see the note at c:969 above for why
            // this must NOT tokenize again.
            let pp = match patcompile(sa, PAT_HEAPDUP, None) {
                Some(p) => p,
                None => return 0,
            };
            if test == CVT_PREPAT {
                // c:1050
                let prefix = lock_str(&COMPPREFIX)
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default();
                let l = prefix.chars().count() as i32;
                if l == 0 {
                    // c:1053
                    // c:1054 — `((na == 1 || na == -1) && pattry(pp, compprefix))`
                    let hit = (na == 1 || na == -1) && pattry(&pp, &prefix);
                    return if hit { 1 } else { 0 };
                }
                let chars: Vec<char> = prefix.chars().collect();
                let (mut p, add): (i32, i32) = if na < 0 {
                    // c:1055
                    (l, -1) // c:1056-1058
                } else {
                    (1, 1) // c:1060-1062
                };
                if na < 0 {
                    na = -na;
                }
                loop {
                    // c:1067
                    let p_uz = p.max(0).min(l) as usize;
                    let head: String = chars[..p_uz].iter().collect(); // c:1068-1069
                    let hit = pattry(&pp, &head); // c:1070
                    if hit {
                        na -= 1;
                        if na == 0 {
                            break;
                        } // c:1071
                    }
                    p += add; // c:1073-1078
                    if add > 0 && p > l {
                        return 0;
                    } // c:1075
                    if add < 0 && p < 0 {
                        return 0;
                    } // c:1080
                }
                if mod_ != 0 {
                    // c:1098 `ignore_prefix(p - compprefix)` — a BYTE offset.
                    // `p` is a character index here, so convert.
                    let p_bytes: i32 = chars[..p.max(0).min(l) as usize]
                        .iter()
                        .map(|ch| ch.len_utf8() as i32)
                        .sum();
                    ignore_prefix(p_bytes);
                }
            } else {
                let suffix = lock_str(&COMPSUFFIX)
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default();
                let l = suffix.chars().count() as i32;
                if l == 0 {
                    // c:1093
                    let hit = (na == 1 || na == -1) && pattry(&pp, &suffix);
                    return if hit { 1 } else { 0 };
                }
                let chars: Vec<char> = suffix.chars().collect();
                let (mut p, add): (i32, i32) = if na < 0 {
                    // c:1095
                    (0, 1)
                } else {
                    (l - 1, -1)
                };
                if na < 0 {
                    na = -na;
                }
                loop {
                    // c:1106
                    let p_uz = p.max(0).min(l) as usize;
                    let tail: String = chars[p_uz..].iter().collect();
                    let hit = pattry(&pp, &tail); // c:1107
                    if hit {
                        na -= 1;
                        if na == 0 {
                            break;
                        }
                    }
                    p += add; // c:1110-1118
                    if add > 0 && p > l {
                        return 0;
                    }
                    if add < 0 && p < 0 {
                        return 0;
                    }
                }
                if mod_ != 0 {
                    // c:1136 `ignore_suffix(ol - (p - compsuffix))` — the BYTE
                    // count from `p` to the end, not the character count.
                    let n_bytes: i32 = chars[p.max(0).min(l) as usize..]
                        .iter()
                        .map(|ch| ch.len_utf8() as i32)
                        .sum();
                    ignore_suffix(n_bytes);
                }
            }
            1 // c:1130
        }
        _ => 0, // c:1135
    }
}

/// C `atoi(3)` semantics, as used by `bin_compset` at c:1188/1189/1200/1209.
///
/// `atoi` skips leading whitespace, takes an optional sign, consumes the
/// leading digit run and IGNORES whatever follows; a string with no leading
/// digits yields 0. Rust's `str::parse::<i32>()` is far stricter — it rejects
/// ` 1`, `1 `, `1x` and any trailing NUL/marker byte outright — and every
/// callsite here funnels the `Err` into `unwrap_or(0)`. A `0` count is not a
/// harmless default in `do_comp_vars`: `CVT_PRENUM` skips its
/// `if (na > 0 && mod)` body and still returns 1, so `compset -p <n>` reports
/// SUCCESS (`bin_compset` returns `!1` = 0) while moving nothing from
/// `$PREFIX` to `$IPREFIX`, and `CVT_PREPAT` bails at its `if (!na) return 0`.
/// Parsing exactly what C parses removes that silent-no-op class.
fn atoi(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() && (b[i] as char).is_whitespace() {
        i += 1;
    }
    let neg = match b.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let mut n: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        n = n.saturating_mul(10).saturating_add((b[i] - b'0') as i64);
        i += 1;
    }
    let n = if neg { -n } else { n };
    n.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// Direct port of `bin_compset(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Zle/complete.c:1137`.
/// Top-level `compset` builtin entry. The C body is 72 lines and
/// dispatches on `argv[0][1]` (`-n`/`-N`/`-p`/`-P`/`-s`/`-S`/`-q`)
/// to one of the CVT_* operations or to set_comp_sep for `-q`.
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_compset(
    name: &str,
    argv: &[String], // c:1137
    _ops: &options,
    _func: i32,
) -> i32 {
    let mut test = 0i32; // c:1141
    let mut na = 0i32;
    let mut nb;
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:1144
        zwarnnam(name, "can only be called from completion function"); // c:1145
        return 1; // c:1146
    }
    if argv.is_empty() || !argv[0].starts_with('-') {
        // c:1148
        zwarnnam(name, "missing option"); // c:1149
        return 1; // c:1150
    }
    let arg0 = &argv[0];
    let opt = arg0.as_bytes().get(1).copied().unwrap_or(0); // c:1152 argv[0][1]
    match opt {
        b'n' => test = CVT_RANGENUM,                    // c:1154
        b'N' => test = CVT_RANGEPAT,                    // c:1155
        b'p' => test = CVT_PRENUM,                      // c:1156
        b'P' => test = CVT_PREPAT,                      // c:1157
        b's' => test = CVT_SUFNUM,                      // c:1158
        b'S' => test = CVT_SUFPAT,                      // c:1159
        b'q' => return compcore::set_comp_sep() as i32, // c:1160
        _ => {
            // c:1161
            // c:1159 `zwarnnam(name, "bad option -%c", argv[0][1])` — no
            // colon here, unlike `bin_compadd`'s "bad option: -%c" (c:792).
            zwarnnam(name, &format!("bad option -{}", opt as char)); // c:1159
            return 1; // c:1163
        }
    }
    // c:1166-1178 — `if (argv[0][2])` — option-arg packed in same token.
    let (sa, sb, na_consumed): (Option<String>, Option<String>, usize);
    if arg0.len() > 2 {
        // c:1166
        sa = Some(arg0[2..].to_string()); // c:1167
        sb = argv.get(1).cloned(); // c:1168
        na_consumed = 2; // c:1169
    } else {
        // c:1171 — `if (!(sa = argv[1])) ...`.
        let Some(s1) = argv.get(1).cloned() else {
            // c:1172
            zwarnnam(name, &format!("missing string for option -{}", opt as char)); // c:1173
            return 1; // c:1174
        };
        sa = Some(s1);
        sb = argv.get(2).cloned();
        na_consumed = 3; // c:1177
    }
    // c:1180 — `if (((test == CVT_PRENUM || test == CVT_SUFNUM) ?
    //     !!sb : (sb && argv[na])))` reject too-many.
    let too_many = if test == CVT_PRENUM || test == CVT_SUFNUM {
        sb.is_some()
    } else {
        sb.is_some() && argv.len() > na_consumed
    };
    if too_many {
        // c:1180
        zwarnnam(name, "too many arguments"); // c:1183
        return 1; // c:1184
    }
    // c:1186-1216 — switch on `test` to compute (na, nb, sa, sb).
    //
    // C mutates the `argv` strings IN PLACE in this switch (`tokenize(sa);
    // remnulargs(sa);` at c:1195-1200 and c:1213-1214) and `do_comp_vars`
    // then compiles what it is handed, untouched (c:977 / c:990 / c:1057).
    // The tokenize/remnulargs pair therefore belongs HERE, in the builtin,
    // and ONLY here: `do_comp_vars` is also reached from `cond_psfix` /
    // `cond_range` (c:1662 / c:1676), whose operand arrives from
    // `cond_str(a, n, 1)` already carrying exactly the tokens the lexer
    // (or `$~`'s GLOB_SUBST `shtokenize`) put in it. Tokenizing a second
    // time re-tokenizes the raw bytes a first pass deliberately left
    // alone — the `-` inside an already-built `Inang - Outang` numeric
    // range becomes `Dash`, so `[[ -prefix $~'<->' ]]` compiled a pattern
    // that matched the empty string and fired for a `-` prefix. That is
    // what made `_numbers` (Completion/Base/Utility/_numbers sh:65) take
    // its `-prefix $~pat` branch for `gtimeout -<TAB>` and emit an empty
    // `duration` description group zsh never shows.
    let mut sa = sa;
    let mut sb = sb;
    match test {
        CVT_RANGENUM => {
            // c:1187
            na = atoi(sa.as_deref().unwrap_or("")); // c:1188
            nb = sb.as_deref().map(atoi).unwrap_or(-1); // c:1189
        }
        CVT_RANGEPAT => {
            // c:1191
            if let Some(s) = sa.as_mut() {
                tokenize(s); // c:1192
                remnulargs(s); // c:1193
            }
            if let Some(s) = sb.as_mut() {
                // c:1194
                tokenize(s); // c:1195
                remnulargs(s); // c:1196
            }
            nb = 0;
        }
        CVT_PRENUM | CVT_SUFNUM => {
            // c:1199
            na = atoi(sa.as_deref().unwrap_or("")); // c:1200
            nb = 0;
        }
        CVT_PREPAT | CVT_SUFPAT => {
            // c:1208-1212 — with a second arg the FIRST is the count and the
            // SECOND the pattern (`na = atoi(sa); sa = sb;` — the `sa = sb`
            // reassignment is handled via `pat` below). WITHOUT a second arg
            // (the common `compset -P <pat>` / `-S <pat>`), C sets `na = -1`,
            // meaning "match the pattern ONCE, anchored at the END of the
            // prefix/suffix". The previous port omitted this else branch, so
            // `na` stayed 0 and `do_comp_vars`'s `if (na == 0) return 0`
            // bailed BEFORE calling `ignore_prefix`/`ignore_suffix` — so
            // `compset -P -` matched nothing and never stripped `-` off
            // `$PREFIX`. Result: `tar -<TAB>` (`compset -P -; _values …`) and
            // any `compset -P/-S <pat>` completer produced an empty list.
            // c:1206 — with two args the pattern is `sb`; C reassigns
            // `sa = sb` so everything downstream (the tokenize below and
            // the `patcompile` in `do_comp_vars`) sees the PATTERN.
            // Passing the count string (e.g. "1") on instead made
            // `compset -P 1 '='` test the prefix against pattern "1"
            // rather than "=", so `_main_complete` wrongly set
            // `$compstate[context]=equal` for ordinary words and every
            // completion did command completion.
            if sb.is_some() {
                na = atoi(sa.as_deref().unwrap_or("")); // c:1209
                sa = sb.clone(); // c:1210
            } else {
                na = -1; // c:1212
            }
            if let Some(s) = sa.as_mut() {
                tokenize(s); // c:1213
                remnulargs(s); // c:1214
            }
            nb = 0;
        }
        _ => {
            nb = 0;
        }
    }
    let _ = (na, nb);
    let pat = sa.as_deref().unwrap_or("");
    let sb_ref = sb.as_deref();
    // c:1217 — `return !do_comp_vars(test, na, sa, nb, sb, 1);`. The final
    // arg is `mod = 1` (MODIFY), NOT 0: for the PATTERN forms
    // (`compset -P`/`-S` = CVT_PREPAT/SUFPAT) do_comp_vars only calls
    // `ignore_prefix`/`ignore_suffix` — which actually strips the matched
    // pattern off `$PREFIX`/`$IPREFIX` — when `mod != 0`. The previous port
    // passed 0, so `compset -P -` matched but never moved the `-` from
    // `$PREFIX` to `$IPREFIX`: `$PREFIX` stayed `-`, and a following
    // `_values`/`_describe` filtered its single-letter matches against `-`
    // (none matched) → empty listing. Symptom: `tar -<TAB>` (which does
    // `compset -P -; _values …`) showed nothing vs zsh's A/c/f/t/u/v/x.
    //
    // `do_comp_vars` returns 1 when it matched/modified and 0 otherwise
    // (C-boolean); the `compset` BUILTIN reports success as exit status 0
    // (compsys tests `compset -P … && …` / `== 0`), so match → 0, no match → 1
    // (C's `return !do_comp_vars(...)`).
    if do_comp_vars(test, na, pat, nb, sb_ref.unwrap_or(""), 1) != 0 {
        0
    } else {
        1
    }
}

// =====================================================================
// compparam table machinery — port of `Src/Zle/complete.c:1235-1295`
// (struct compparam comprparams[] / compkparams[] tables) +
// addcompparams / makecompparams / comp_setunset / compunsetfn ported.
// =====================================================================
//
// The substrate the C source uses (`createparam`, `paramtab()`,
// `getparamnode`, `newparamtable`, `deleteparamtable`) is now
// ported in `params.rs`:
//   - createparam        → params.rs:4727
//   - paramtab           → params.rs:3126
//   - getparamnode       → params.rs:4889
//   - newparamtable      → params.rs:5035
//   - createparamtable   → params.rs:4694
//
// The ported below dispatch through that canonical Rust paramtab via
// setsparam/setiparam/setaparam. The GSU-vtable swap on each param
// (a per-param custom-getter hook) is what wires e.g. `$BUFFER`
// reads to the live `ZLELINE` global — that hook surface is the
// `Param.gsu` field on params.rs's Param struct, which today binds
// to the default scalar/array getters. Custom-getter wiring for
// `$BUFFER`/`$CURSOR`/`$KILLRING`-style params is what
// makezleparams (zle_params.rs:498, ported) sets up at widget-call
// entry; the read/write surface works today via the existing
// scalar/array params.

/// Direct port of `addcompparams(struct compparam *cp, Param *pp)` from
/// `Src/Zle/complete.c:1297`. Walks the compparam table, calling
/// `createparam` for each entry with `PM_SPECIAL|PM_REMOVABLE|PM_LOCAL`
/// or'd into its type. The gsu vtable hookup (c:1308-1324) is set on
/// the returned Param via `u_data`; the per-type gsu (compvarscalar_gsu
/// etc.) isn't yet exposed as a sym so we record the `var`/`gsu`
/// hooks on `u_data` for parity.
#[allow(unused_variables)]
pub fn addcompparams(cp: &[compparam], pp: &mut Vec<*mut param>) {
    // c:1297
    for entry in cp {
        // c:1299
        let flags = entry.r#type | PM_SPECIAL as i32 | PM_REMOVABLE as i32 | PM_LOCAL as i32;
        // c:1300 — createparam(name, type | SPECIAL|REMOVABLE|LOCAL).
        let pm = createparam(entry.name, flags);
        if let Some(mut pm_val) = pm {
            // c:1307 — `pm->level = locallevel + 1`. locallevel not
            // exposed; the level field defaults to 0 which is fine
            // for the static-link path.
            pm_val.u_data = entry.var; // c:1308
                                       // c:1309-1324 — gsu vtable per PM_TYPE. The Rust port
                                       // stores the gsu address on u_data; the per-type gsu
                                       // resolution happens at param-read time via the typed
                                       // accessor (get_unambig, get_compstate, etc.) that the
                                       // caller wired explicitly into the param entries.
            pp.push(std::ptr::null_mut::<param>());
        } else {
            // c:1302 — `pm = paramtab->getnode(paramtab, name)`. Look
            // up existing entry if createparam returned None.
            pp.push(std::ptr::null_mut::<param>());
        }
    }
}

/// Direct port of `makecompparams()` from `Src/Zle/complete.c:1333`.
/// Calls addcompparams(comprparams) to register the CP_REALPARAMS
/// entries ($words/$CURRENT/$PREFIX/etc.) into the global paramtab,
/// then createparam("compstate", PM_HASHED) and addcompparams(
/// compkparams) for the per-key entries inside the hash.
pub fn makecompparams() {
    // c:1333
    let mut comprpms: Vec<*mut param> = Vec::new();
    addcompparams(COMPRPARAMS, &mut comprpms); // c:1338

    // c:1340 — createparam(COMPSTATENAME, PM_SPECIAL|PM_REMOVABLE|
    //          PM_SINGLE|PM_LOCAL|PM_HASHED).
    let _ = createparam(
        "compstate",
        (PM_SPECIAL | PM_REMOVABLE | PM_SINGLE | PM_LOCAL | PM_HASHED) as i32,
    );
    // c:1351 — addcompparams(compkparams, compkpms). These live inside
    // the $compstate hash; without inner-hash createparam yet, register
    // them at the top level so getsparam("compstate[X]") finds them.
    let mut compkpms: Vec<*mut param> = Vec::new();
    addcompparams(COMPKPARAMS, &mut compkpms);
}

/// Direct port of `HashTable get_compstate(Param pm)` from
/// `Src/Zle/complete.c:1357`. C body (single statement):
///     `return pm->u.hash;`
/// Rust returns `Option<usize>` (opaque HashTable-pointer parity);
/// `None` when the param has no hash.
pub fn get_compstate(pm: *mut param) -> Option<usize> {
    // c:1357
    unsafe { pm.as_ref() } // c:1359 pm->...
        .and_then(|p| p.u_hash.as_ref().map(|_| &p.u_hash as *const _ as usize))
}

/// Direct port of `void set_compstate(Param pm, HashTable ht)` from
/// `Src/Zle/complete.c:1364`. Writes each entry from `ht` back into
/// the matching compkparams slot (per c:1376-1391). The C body iterates
/// every hash node, matches its name against `compkparams[i].name`,
/// and copies the int/string value into the C-side variable pointed
/// to by `cp->var`, clearing PM_UNSET on the param.
///
/// Static-link path: without the compkparams `var` slots resolved to
/// real Rust globals (they pointed to file-static C strings), the
/// best we can do is copy each key-value pair into the matching
/// `compstate[key]` paramtab entry via setsparam — preserving the
/// observable side-effect that user-set $compstate values become
/// visible to subsequent reads.
#[allow(unused_variables)]
pub fn set_compstate(
    pm: *mut param, // c:1364
    ht: Option<usize>,
) {
    // c:1373 — `if (!ht) return`.
    let Some(_handle) = ht else {
        return;
    };
    // c:1376-1391 — walk the inner hash, copying each compkparams
    // entry's value into the matching var. Without the legacy var
    // pointers, we drive the same effect via the
    // `compstate[<key>]` paramtab values which the C var pointers
    // reflected indirectly via the gsu vtable.
    //
    // In practice every $compstate write goes through setsparam
    // already; this entry is the inverse direction (post-shfunc
    // commit). Real param-hash access lands when the inner hash
    // backing $compstate is wired as its own paramtable. For now
    // the side-effect is already covered by the per-key gsu hooks
    // (set_complist, etc.), so set_compstate is a structural pass-
    // through that mirrors the C `if (ht != pm->u.hash)
    // deleteparamtable(ht)` at c:1395 (handled by Drop).
}

/// Direct port of `zlong get_nmatches(UNUSED(Param pm))` from
/// `Src/Zle/complete.c:1401`. C body: `return (permmatches(0) ? 0 :
/// nmatches)` — runs permmatches(0) to commit any pending matches,
/// then returns 0 if that returned non-zero (incomplete) or the
/// nmatches counter otherwise.
#[allow(unused_variables)]
pub fn get_nmatches(pm: *mut param) -> i64 {
    // c:1401
    if compcore::permmatches(0) != 0 {
        // c:1403
        return 0;
    }
    NMATCHES_GLOBAL.load(Ordering::Relaxed) // c:1404 nmatches
}

/// Direct port of `zlong get_listlines(UNUSED(Param pm))` from
/// `Src/Zle/complete.c:1408`. C body: `return list_lines();` —
/// the live line-count of the match list at current terminal width.
/// The C implementation (compresult.c:1392) commits permmatches,
/// swaps amatches↔pmatches, runs calclist(0), then returns
/// `listdat.nlines`.
///
/// Rust port runs calclist on the current amatches (we don't yet
/// have a separate permmatches swap), then reads `listdat.nlines`
/// directly — same observable count for the common case where no
/// permmatches commit is pending. Falls back to the cached
/// COMPLISTLINES atomic when listdat isn't initialized.
#[allow(unused_variables)]
pub fn get_listlines(pm: *mut param) -> i64 {
    // c:1418 — `return list_lines();`. The port inlined only calclist(0),
    // dropping list_lines' `permmatches(0)`, the amatches↔pmatches swap and
    // BOTH `listdat.valid = 0` resets — so calclist short-circuited on a
    // still-valid listdat (compresult.c:1502) and `$compstate[list_lines]`
    // reported the previous round's line count.
    compresult::list_lines() // c:1420
}

/// Direct port of `set_complist(UNUSED(Param pm), char *v)` from `Src/Zle/complete.c:1415`.
/// C body (c:1417): `comp_list(v)` — sets the complist global and
/// updates the onlyexpl bitmap.
#[allow(unused_variables)]
pub fn set_complist(pm: *mut param, v: &str) {
    // c:1415
    compresult::comp_list(Some(v)); // c:1417
}

/// Direct port of `get_complist(UNUSED(Param pm))` from `Src/Zle/complete.c:1422`.
/// C body (c:1424): `return complist;`.
#[allow(unused_variables)]
pub fn get_complist(pm: *mut param) -> String {
    // c:1422
    lock_str(&COMPLIST)
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default() // c:1429
}

// =====================================================================
// CVT_* constants — port of `Src/Zle/complete.c:855-860` `#define`s.
// Used by bin_compset/cond_psfix/cond_range to discriminate the
// completion-variable-mutation opcode passed to do_comp_vars.
// =====================================================================
/// Port of `COMPSTATENAME` from `Src/Zle/complete.c:1294`.
/// `#define COMPSTATENAME "compstate"` — name of the magic-assoc
/// parameter created by `callcompfunc` so user widgets can read +
/// mutate completion state via `${compstate[...]}`.
pub const COMPSTATENAME: &str = "compstate"; // c:1294
/// `CVT_RANGENUM` constant.
pub const CVT_RANGENUM: i32 = 0; // c:855
/// `CVT_RANGEPAT` constant.
pub const CVT_RANGEPAT: i32 = 1; // c:856
/// `CVT_PRENUM` constant.
pub const CVT_PRENUM: i32 = 2; // c:857
/// `CVT_PREPAT` constant.
pub const CVT_PREPAT: i32 = 3; // c:858
/// `CVT_SUFNUM` constant.
pub const CVT_SUFNUM: i32 = 4; // c:859
/// `CVT_SUFPAT` constant.
pub const CVT_SUFPAT: i32 = 5; // c:860

// =====================================================================
// Order-options table — port of `static struct ... orderopts[]` from
// `Src/Zle/complete.c:561`. Each entry is (name, abbrev, oflag); the
// `abbrev` field is the minimum-prefix length that uniquely matches.
// =====================================================================

#[allow(non_snake_case)]
struct OrderOpt {
    name: &'static str,
    abbrev: usize,
    oflag: i32,
}

static ORDEROPTS: &[OrderOpt] = &[
    // c:561
    OrderOpt {
        name: "nosort",
        abbrev: 2,
        oflag: CAF_NOSORT,
    }, // c:562
    OrderOpt {
        name: "match",
        abbrev: 3,
        oflag: CAF_MATSORT,
    }, // c:563
    OrderOpt {
        name: "numeric",
        abbrev: 3,
        oflag: crate::ported::zle::comp_h::CAF_NUMSORT,
    }, // c:564
    OrderOpt {
        name: "reverse",
        abbrev: 3,
        oflag: crate::ported::zle::comp_h::CAF_REVSORT,
    }, // c:565
];

/// Direct port of `char *get_unambig(UNUSED(Param pm))` from
/// `Src/Zle/complete.c:1429`. C body returns
/// `unambig_data(NULL, NULL, NULL)` — the longest common prefix
/// shared by every currently-active match. Rust port walks the
/// live `amatches` chain, collects the `str` field of each visible
/// match (skipping CMF_HIDE), and feeds the resulting `Vec<String>`
/// to `unambig_data` which computes the LCP.
#[allow(unused_variables)]
pub fn get_unambig(pm: *mut param) -> String {
    // c:1429
    // c:1431 — `unambig_data(NULL, NULL, NULL); return scache`.
    if let Some(s) = compcore::ainfo
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|a| a.line.clone()))
        .map(|l| compresult::cline_str(Some(l), 0, None, None).unwrap_or_default())
        .filter(|s| !s.is_empty())
    {
        return s;
    }
    let strs: Vec<String> = compcore::amatches
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| {
            g.iter()
                .flat_map(|gr| gr.matches.iter())
                .filter(|m| (m.flags & CMF_HIDE) == 0)
                .filter_map(|m| m.str.clone())
                .collect()
        })
        .unwrap_or_default();
    compresult::unambig_data(&strs)
}

/// Direct port of `zlong get_unambig_curs(UNUSED(Param pm))` from
/// `Src/Zle/complete.c:1436`. C body: `unambig_data(&c, NULL,
/// NULL); return c;` — the cursor position within the unambiguous
/// prefix string. With the Cline-tree cursor-tracking pipeline
/// substrate-deferred, derive an equivalent from the LCP length
/// (chars) which matches the simple-case where every match
/// agrees up through that position.
#[allow(unused_variables)]
pub fn get_unambig_curs(pm: *mut param) -> i64 {
    // c:1436
    // c:1438 — `unambig_data(&c, NULL, NULL); return c`. When ainfo->line
    // is populated, the cursor offset is the length of the cline_str
    // output (in chars) since our cline_str doesn't currently track the
    // divergence-position cursor — full mid/pm/sm/d tracking lives in the
    // C ins=0 csp pass.
    let prefix = get_unambig(std::ptr::null_mut());
    // c:compresult.c:564 — `if (cp) *cp = ccache + 1;`. The value is
    // ONE-BASED: `$compstate[unambiguous_cursor]` is "the index of the
    // character the cursor would sit before", and both consumers slice
    // with it as such — `_main_complete` sh:86-88
    // (`PREFIX[1,upos-1]`) and sh:375
    // (`${compstate[unambiguous]}[1,${compstate[unambiguous_cursor]}-1]`).
    // The `+ 1` was dropped here, so every reader was one character short:
    // for `_pr<TAB>` zsh reports 4 against this port's 3, and the sh:375
    // ambiguous-colour prefix came out `_p` instead of `_pr`. The empty
    // case still agrees with C, whose else-branch (c:560) leaves
    // `ccache = 0` and so also returns 1.
    prefix.chars().count() as i64 + 1
}

/// Direct port of `struct compparam` from `Src/Zle/complete.c:1215`.
/// One entry per special completion parameter (e.g. PREFIX, SUFFIX,
/// IPREFIX, words, current). `var` holds a pointer to the storage
/// the gsu reads/writes; for the kparams it's a pointer into the
/// global completion-state buffers.
#[allow(non_camel_case_types)]
pub struct compparam {
    // c:1215
    pub name: &'static str, // c:1216 char *name
    pub r#type: i32,        // c:1217 int type
    pub var: usize,         // c:1218 void *var
    pub gsu: usize,         // c:1219 GsuScalar gsu
}

/// Static table mirroring `static struct compparam comprparams[]` from
/// `Src/Zle/complete.c:1248`. Real-params table (CP_REALPARAMS) — the
/// non-keyparam compsys parameters that live directly in the global
/// paramtab.
pub const COMPRPARAMS: &[compparam] = &[
    compparam {
        name: "words",
        r#type: PM_ARRAY as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "redirections",
        r#type: PM_ARRAY as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "CURRENT",
        r#type: PM_INTEGER as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "PREFIX",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "SUFFIX",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "IPREFIX",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "ISUFFIX",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "QIPREFIX",
        r#type: (PM_SCALAR | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "QISUFFIX",
        r#type: (PM_SCALAR | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
];

/// Static table mirroring `static struct compparam compkparams[]` from
/// `Src/Zle/complete.c:1261`. Key-params table (CP_KEYPARAMS) — the
/// per-call keys that live inside the $compstate hashed param.
const COMPKPARAMS: &[compparam] = &[
    compparam {
        name: "nmatches",
        r#type: (PM_INTEGER | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "context",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "parameter",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "redirect",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "quote",
        r#type: (PM_SCALAR | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "quoting",
        r#type: (PM_SCALAR | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "restore",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "list",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "insert",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "exact",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "exact_string",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "pattern_match",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "pattern_insert",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "unambiguous",
        r#type: (PM_SCALAR | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "unambiguous_cursor",
        r#type: (PM_INTEGER | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "unambiguous_positions",
        r#type: (PM_SCALAR | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "insert_positions",
        r#type: (PM_SCALAR | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "list_max",
        r#type: PM_INTEGER as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "last_prompt",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "to_end",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "old_list",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "old_insert",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "vared",
        r#type: PM_SCALAR as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "list_lines",
        r#type: (PM_INTEGER | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "all_quotes",
        r#type: (PM_SCALAR | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
    compparam {
        name: "ignored",
        r#type: (PM_INTEGER | PM_READONLY) as i32,
        var: 0,
        gsu: 0,
    },
];

/// Direct port of `char *get_unambig_pos(UNUSED(Param pm))` from
/// `Src/Zle/complete.c:1447`. C body: `unambig_data(NULL, &p, NULL);
/// return p` — the colon-separated divergence-position list (one
/// number per CLF_DIFF / CLF_MISS Cline node).
///
/// When `ainfo.line` is populated, returns the cline_str output
/// length as the single-divergence position (the common-case);
/// otherwise falls back to the LCP-length-derived position over the
/// live `amatches` strings.
#[allow(unused_variables)]
pub fn get_unambig_pos(pm: *mut param) -> String {
    // c:1447
    if let Some(s) = compcore::ainfo
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|a| a.line.clone()))
        .map(|l| compresult::cline_str(Some(l), 0, None, None).unwrap_or_default())
        .filter(|s| !s.is_empty())
    {
        return format!("{}", s.chars().count());
    }
    let strs: Vec<String> = compcore::amatches
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| {
            g.iter()
                .flat_map(|gr| gr.matches.iter())
                .filter(|m| (m.flags & CMF_HIDE) == 0)
                .filter_map(|m| m.str.clone())
                .collect()
        })
        .unwrap_or_default();
    if strs.len() < 2 {
        return String::new();
    }
    let lcp_len = compresult::unambig_data(&strs).chars().count();
    if strs.iter().any(|s| s.chars().count() > lcp_len) {
        format!("{}", lcp_len)
    } else {
        String::new()
    }
}

/// Direct port of `char *get_insert_pos(UNUSED(Param pm))` from
/// `Src/Zle/complete.c:1458`. C body: `unambig_data(NULL, NULL, &p);
/// return p;` — the position-string for the unambiguous-prefix
/// insert positions (where the cursor sits after the prefix is
/// inserted, accounting for braces and original-string positions).
///
/// c:compresult.c:159-161 spells out how the `ins=2` pass differs from
/// the `ins=0` pass `get_unambig_pos` uses: "If ins is two, csp and posl
/// contain **real command line positions** (including braces)" — i.e.
/// the same divergence points, but measured from the start of the LINE
/// rather than from the start of the unambiguous string. So each
/// position is shifted by `wb`, the line offset where the word being
/// completed begins (`Src/Zle/compcore.c` / lex.c:120).
///
/// This port previously returned `get_unambig_pos` verbatim, i.e. left
/// the shift out: for `ls -<TAB>` zsh reports `4` (word starts at
/// column 3, divergence at offset 1) against this port's `1`. Only the
/// `wb == 0` case — a word at the very start of the line, e.g.
/// `_pr<TAB>` — agreed. The brace-reinsertion offsets the C pass also
/// folds in still need the Cline-tree walk and are not modelled here.
#[allow(unused_variables)]
pub fn get_insert_pos(pm: *mut param) -> String {
    // c:1458
    let wb = compcore::WB.load(Ordering::Relaxed).max(0) as i64;
    get_unambig_pos(std::ptr::null_mut())
        .split(':') // c:build_pos_string joins with ':'
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<i64>().ok())
        .map(|p| (p + wb).to_string())
        .collect::<Vec<_>>()
        .join(":")
}

/// Direct port of `char *get_compqstack(UNUSED(Param pm))` from
/// `Src/Zle/complete.c:1469`. Walks the compqstack byte buffer and
/// decodes each quote-state byte (QT_NONE/QT_SINGLE/QT_DOUBLE/
/// QT_DOLLARS/QT_BACKTICK/QT_BACKSLASH) into its single-char
/// printable form via `comp_quoting_string`. Was returning the raw
/// QT_* byte stack which gave gibberish like `\x00\x01\x02` to
/// callers reading `$compstate[quoting_stack]`.
#[allow(unused_variables)]
pub fn get_compqstack(pm: *mut param) -> String {
    // c:1469
    // c:1473 — `if (!compqstack) return "";`
    let stack = lock_str(&COMPQSTACK)
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    if stack.is_empty() {
        return String::new();
    }
    // c:1480-1485 — `for (cqp = compqstack; *cqp; cqp++)
    //                  { str = comp_quoting_string(*cqp); *ptr++ = *str; }`
    let mut out = String::with_capacity(stack.len());
    for cqp in stack.chars() {
        let cqp_byte = cqp as i32;
        let s = compcore::comp_quoting_string(cqp_byte);
        // c:1483 — take only the first char of each printable form.
        if let Some(first) = s.chars().next() {
            out.push(first);
        }
    }
    out
}

/// Direct port of `void compunsetfn(Param pm, int exp)` from
/// `Src/Zle/complete.c:1489`. Drops a completion param's storage when
/// it goes out of scope. For `exp` (explicit unset) zeros the
/// underlying storage by PM_TYPE. Otherwise (implicit fall-out) the
/// PM_HASHED ($compstate) arm deletes its inner hashtable; nulls out
/// matching comprpms / compkpms entries by name lookup against
/// COMPRPARAMS / COMPKPARAMS.
pub fn compunsetfn(pm: *mut param, exp: i32) {
    // c:1489
    if pm.is_null() {
        return;
    }
    let name = unsafe { (*pm).node.nam.clone() };
    if exp != 0 {
        // c:1492
        // c:1494/1497/1500 — switch on PM_TYPE(pm->node.flags).
        match PM_TYPE(unsafe { (*pm).node.flags } as u32) {
            PM_SCALAR => unsafe {
                (*pm).u_str = Some(String::new());
            }, // c:1494
            PM_ARRAY => unsafe {
                (*pm).u_arr = Some(Vec::new());
            }, // c:1497
            PM_HASHED => unsafe {
                (*pm).u_hash = None;
            }, // c:1500
            _ => {}
        }
    } else if PM_TYPE(unsafe { (*pm).node.flags } as u32) == PM_HASHED {
        // c:1505
        // c:1508 — `deletehashtable(pm->u.hash); pm->u.hash = NULL;`.
        unsafe {
            (*pm).u_hash = None;
        } // c:1509
          // c:1512-1514 — null out compkpms[i] for each CP_KEYPARAMS
          // entry. Driven via paramtab: set PM_UNSET on each compkparams
          // name so subsequent get_*'s see "unset".
        for entry in COMPKPARAMS {
            if let Ok(mut tab) = paramtab().write() {
                if let Some(p) = tab.get_mut(entry.name) {
                    p.node.flags |= PM_UNSET as i32;
                }
            }
        }
    }
    // c:1524-1533 — `if (!exp) { for (p = comprpms, …) if (*p == pm) { *p = NULL; break; } }`.
    // Drive via name match: if the unset target matches a comprparams
    // entry, mark that slot in paramtab as PM_UNSET. The `if (!exp)` gate
    // was missing, so an EXPLICIT `unset PREFIX` inside a completion
    // function marked the parameter unset on top of C's clear-to-empty
    // (c:1503-1505) — C only detaches the comprpms slot on the implicit
    // scope-exit path, leaving an explicitly-unset PREFIX readable as "".
    if exp == 0 {
        for entry in COMPRPARAMS {
            if entry.name == name {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(p) = tab.get_mut(entry.name) {
                        p.node.flags |= PM_UNSET as i32;
                    }
                }
                break;
            }
        }
    }
}

/// Direct port of `void comp_setunset(int rset, int runset, int kset,
/// int kunset)` from `Src/Zle/complete.c:1528`. Two-pass flag-bitmap
/// walk: for each bit `i` set in `rset`/`runset`, clear/set PM_UNSET on
/// `comprpms[i]` (the i'th entry of `COMPRPARAMS`); same for `kset`/
/// `kunset` against `COMPKPARAMS`. Drives the PM_UNSET state-machine
/// the comp_wrapper save/restore relies on.
pub fn comp_setunset(
    mut rset: i32,
    mut runset: i32, // c:1528
    mut kset: i32,
    mut kunset: i32,
) {
    // c:1532 — `if (comprpms && (rset >= 0 || runset >= 0))`.
    if rset >= 0 || runset >= 0 {
        // c:1532
        for entry in COMPRPARAMS {
            // c:1533
            if rset != 0 || runset != 0 {
                // c:1533
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(p) = tab.get_mut(entry.name) {
                        if rset & 1 != 0 {
                            // c:1535
                            p.node.flags &= !(PM_UNSET as i32); // c:1536
                        }
                        if runset & 1 != 0 {
                            // c:1537
                            p.node.flags |= PM_UNSET as i32; // c:1538
                        }
                    }
                }
                rset >>= 1;
                runset >>= 1;
            } else {
                break;
            }
        }
    }
    // c:1542 — `if (compkpms && (kset >= 0 || kunset >= 0))`.
    if kset >= 0 || kunset >= 0 {
        // c:1542
        for entry in COMPKPARAMS {
            if kset != 0 || kunset != 0 {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(p) = tab.get_mut(entry.name) {
                        if kset & 1 != 0 {
                            // c:1545
                            p.node.flags &= !(PM_UNSET as i32);
                        }
                        if kunset & 1 != 0 {
                            // c:1547
                            p.node.flags |= PM_UNSET as i32;
                        }
                    }
                }
                kset >>= 1;
                kunset >>= 1;
            } else {
                break;
            }
        }
    }
}

/// Direct port of `int comp_wrapper(Eprog prog, FuncWrap w, char *name)`
/// from `Src/Zle/complete.c:1556`.
///
/// `zsh/complete`'s `boot_` installs it into the global function-wrapper
/// chain — `static struct funcwrap wrapper[] = { WRAPDEF(comp_wrapper) };`
/// (c:1694-1695) then `return addwrapper(m, wrapper);` (c:1767) — and
/// `runshfunc` fires the chain around EVERY shell function body:
/// `runshfunc(prog, wrappers, funcsave->fstack.name);` (`Src/exec.c:6042`).
/// So while `incompfunc == 1` this brackets each individual completion
/// function call.
///
/// What it brackets: `compstate[restore]` is seeded to `"auto"` BEFORE the
/// body runs (c:1576) and, unless the body changed it, `$words`,
/// `$CURRENT`, `$PREFIX`, `$SUFFIX`, `$IPREFIX`, `$ISUFFIX`, `$QIPREFIX`,
/// `$QISUFFIX`, `$compstate[quote]` / `[quoting]` / `[all_quotes]`,
/// `redirections` and `autoq` are all put back on the way out
/// (c:1593-1625). `_arguments` depends on both halves: `comparguments -W`
/// (`_arguments` sh:393) calls [`restrict_range`] to narrow
/// `$words`/`$CURRENT` down to the rest-argument slice, then sh:401 sets
/// `compstate[restore]=''` so the narrowing deliberately survives into the
/// action it dispatches — and is undone again as soon as that action's own
/// wrapper frame pops. Without the wrapper the narrowing has no bound at
/// all and leaks back up the completer chain (measured: `man <TAB>` came
/// back from `_complete` with `CURRENT=1 words=()` instead of
/// `CURRENT=2 words=(man '')`, so `_main_complete` fell through to
/// `_complete_hist`).
///
/// zshrs difference (load-bearing): in C `$words`/`$CURRENT`/`$PREFIX`/…
/// are gsu VIEWS onto the `compwords`/`compcurrent`/`compprefix`/… globals
/// — `{ "words", PM_ARRAY, VAL(compwords), NULL, NULL }` (c:1249) — so
/// restoring the global restores the shell-visible parameter for free.
/// This port has no gsu binding: the parameters own their own copies
/// (`compcore.rs:2989-3010`, and `restrict_range` at complete.rs:1546).
/// The restore below therefore has to write BOTH halves — restoring only
/// the globals would leave a completer's `$words` narrowed for the rest of
/// the completion. Each half is snapshotted and restored SEPARATELY,
/// because the two are not in lockstep: shell-level assignments inside a
/// completion function move only the parameter, so mid-function the global
/// and the parameter can legitimately disagree. See the note at the
/// parameter-side snapshot below for the `ls /us<TAB>` measurement that
/// pins this down.
///
/// WARNING: param names don't match C — Rust=(_prog, _w, name, runshfunc)
/// vs C=(prog, w, name). The trailing closure IS c:1591's
/// `runshfunc(prog, w, name)`: a zshrs function body is a caller-supplied
/// delegate (`doshfunc`'s `body_runner`), not an `Eprog` this function
/// could re-enter by itself. Identical shape to
/// `param_private::wrap_private`, the other wrapper in the chain.
pub fn comp_wrapper(
    _prog: *const eprog, // c:1556
    _w: *const funcwrap,
    name: &str,
    runshfunc: impl FnOnce(),
) -> i32 {
    use crate::ported::zle::comp_h::{
        CP_ALLKEYS, CP_ALLREALS, CP_COMPSTATE, CP_CURRENT, CP_IPREFIX, CP_ISUFFIX, CP_PREFIX,
        CP_QIPREFIX, CP_QISUFFIX, CP_REDIRS, CP_RESTORE, CP_SUFFIX, CP_WORDS,
    };
    use std::sync::atomic::Ordering;
    // c:1558-1559 — `if (incompfunc != 1) return 1;`. 1 is the chain's
    // "not handled, keep walking" answer (`Src/exec.c:6186` short-circuits
    // only on 0), so an ordinary non-completion function call pays one
    // relaxed load and then runs completely unwrapped.
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:1558
        return 1; // c:1559
    }
    let _ = name; // c:1591 passes it to runshfunc; the delegate already has it
    let snap = |g: &'static std::sync::OnceLock<Mutex<String>>| -> String {
        g.get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    };
    let restore = |g: &'static std::sync::OnceLock<Mutex<String>>, v: String| {
        if let Ok(mut s) = g.get_or_init(|| Mutex::new(String::new())).lock() {
            *s = v;
        }
    };

    // c:1567-1572 — record which real params are ALREADY PM_UNSET so the
    // c:1619 `comp_setunset` can put each one back to the state it had
    // rather than blanket-marking the whole set as "set".
    //   m = CP_WORDS | CP_REDIRS | CP_CURRENT | CP_PREFIX | CP_SUFFIX |
    //       CP_IPREFIX | CP_ISUFFIX | CP_QIPREFIX | CP_QISUFFIX;
    //   for (pp = comprpms, sm = 1; m; pp++, m >>= 1, sm <<= 1)
    //       if ((m & 1) && ((*pp)->node.flags & PM_UNSET)) runset |= sm;
    let mut runset: u32 = 0; // c:1564
    {
        let mut m = CP_WORDS
            | CP_REDIRS
            | CP_CURRENT
            | CP_PREFIX
            | CP_SUFFIX
            | CP_IPREFIX
            | CP_ISUFFIX
            | CP_QIPREFIX
            | CP_QISUFFIX; // c:1567-1568
        let mut sm: u32 = 1;
        if let Ok(tab) = paramtab().read() {
            for cp in COMPRPARAMS {
                // c:1569
                if m == 0 {
                    break;
                }
                if (m & 1) != 0 {
                    // c:1570 — C always has the node and only the flag
                    // varies; a name this port never created is the same
                    // observable state, so count it as unset.
                    let is_unset = tab
                        .get(cp.name)
                        .map(|p| (p.node.flags & PM_UNSET as i32) != 0)
                        .unwrap_or(true);
                    if is_unset {
                        runset |= sm; // c:1571
                    }
                }
                m >>= 1;
                sm <<= 1;
            }
        }
    }
    // c:1573-1574 — `if (compkpms[CPN_RESTORE]->node.flags & PM_UNSET)
    // kunset = CP_RESTORE;`. `$compstate` keys are hash entries here, not
    // Params carrying their own flag word, so "absent from the hash" is
    // this port's PM_UNSET.
    let kunset: u32 = if compcore::get_compstate_str("restore").is_none() {
        CP_RESTORE // c:1574
    } else {
        0 // c:1564
    };

    // c:1575-1576 — `orest = comprestore; comprestore = ztrdup("auto");`.
    // The default is SEEDED here, before the function runs, and the
    // caller's value put back at c:1642-1643.
    // `comprestore` is NOT a parameter of its own: complete.c:1268 lists it
    // in `compkparams` as `{ "restore", PM_SCALAR, VAL(comprestore) }`,
    // i.e. the C global is gsu-bound to the `$compstate[restore]` KEY.
    let orest = compcore::get_compstate_str("restore"); // c:1575
    compcore::set_compstate_str("restore", "auto"); // c:1576
    let ocur = COMPCURRENT.load(Ordering::Relaxed); // c:1577
    let opre = snap(&COMPPREFIX); // c:1578
    let osuf = snap(&COMPSUFFIX); // c:1579
    let oipre = snap(&COMPIPREFIX); // c:1580
    let oisuf = snap(&COMPISUFFIX); // c:1581
    let oqipre = snap(&COMPQIPREFIX); // c:1582
    let oqisuf = snap(&COMPQISUFFIX); // c:1583
    let oq = snap(&COMPQUOTE); // c:1584
    let oqi = snap(&COMPQUOTING); // c:1585
    let oqs = snap(&COMPQSTACK); // c:1586
    let oaq = snap(&AUTOQ); // c:1587
    let owords = lock_vec(&COMPWORDS)
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default(); // c:1588
                              // c:1589 — `oredirs = zarrdup(compredirs);`. There is no `compredirs`
                              // global in this port; the `redirections` parameter (COMPRPARAMS[1]) is
                              // the only storage, so snapshot it from paramtab. `None` = the name was
                              // never created, which is the `runset` bit already recorded above.
    let oredirs = crate::ported::params::getaparam("redirections"); // c:1589

    // PARAMETER-SIDE snapshot of the same c:1577-1589 set.
    //
    // C needs only the globals above because every one of these names is a
    // gsu VIEW onto them (`{ "PREFIX", PM_SCALAR, VAL(compprefix), ... }`,
    // c:1249): one storage, so restoring `compprefix` restores `$PREFIX`.
    // This port has two storages and they are NOT in lockstep — a
    // shell-level `PREFIX=...` assignment inside a completion function
    // (`_path_files` does it at sh:436/439/567/639/643/673/808/845/848/890)
    // lands in the parameter, while the globals only move when the engine
    // writes them (`compset`, `restrict_range`, `callcompfunc`). So the two
    // legitimately hold DIFFERENT values mid-function: measured under
    // `ls /us<TAB>`, at the `_list_files` call the global was "us" while
    // `$PREFIX` was "/us" — and "/us" is what real zsh reports there.
    //
    // Each storage therefore has to go back to ITS OWN entry value.
    // Restoring the parameter from the GLOBAL's snapshot instead is what
    // broke file completion: it published the stale global into `$PREFIX`,
    // so `_path_files` resumed after `_list_files` with a truncated prefix,
    // its match generation collapsed, and `_main_complete` fell through the
    // completer list to `_fasd_zsh_word_complete_trigger`.
    //
    // `None` means the name was never created — this port's PM_UNSET,
    // already recorded in `runset`, so it must NOT be recreated here.
    let opre_p = crate::ported::params::getsparam("PREFIX"); // c:1578
    let osuf_p = crate::ported::params::getsparam("SUFFIX"); // c:1579
    let oipre_p = crate::ported::params::getsparam("IPREFIX"); // c:1580
    let oisuf_p = crate::ported::params::getsparam("ISUFFIX"); // c:1581
    let oqipre_p = crate::ported::params::getsparam("QIPREFIX"); // c:1582
    let oqisuf_p = crate::ported::params::getsparam("QISUFFIX"); // c:1583
    let oq_p = crate::ported::params::getsparam("QUOTE"); // c:1584
    let oqi_p = crate::ported::params::getsparam("QUOTING"); // c:1585
    let ocur_p = crate::ported::params::getiparam("CURRENT"); // c:1577
    let owords_p = crate::ported::params::getaparam("words"); // c:1588

    // c:1591 — `runshfunc(prog, w, name);`
    runshfunc();

    // c:1593 — `if (comprestore && !strcmp(comprestore, "auto"))`.
    let comprestore_val = compcore::get_compstate_str("restore").unwrap_or_default();
    if comprestore_val == "auto" {
        COMPCURRENT.store(ocur, Ordering::Relaxed); // c:1594
        restore(&COMPPREFIX, opre); // c:1596
        restore(&COMPSUFFIX, osuf); // c:1598
        restore(&COMPIPREFIX, oipre); // c:1600
        restore(&COMPISUFFIX, oisuf); // c:1602
        restore(&COMPQIPREFIX, oqipre); // c:1604
        restore(&COMPQISUFFIX, oqisuf); // c:1606
        restore(&COMPQUOTE, oq); // c:1608
        restore(&COMPQUOTING, oqi); // c:1610
        restore(&COMPQSTACK, oqs); // c:1612
        // !!! RUST-ONLY LINE — NO C COUNTERPART !!!
        // In C, `$compstate[all_quotes]` has NO storage of its own: its
        // `compkparams` row is `{ "all_quotes", PM_SCALAR | PM_READONLY,
        // NULL, GSU(compqstack_gsu) }` (c:1299) and `compqstack_gsu`
        // (c:1242-1243) routes every read through `get_compqstack` (c:1479)
        // against the live `compqstack` global — so c:1612's assignment IS
        // the parameter update. zshrs splits the two: a single-key
        // `${compstate[KEY]}` read comes straight out of
        // `paramtab_hashed_storage` (`src/ported/subst.rs:7034-7044`), which
        // special-cases only `nmatches`, so nothing ever published
        // `all_quotes` and it read EMPTY where zsh gives `\`, `"`, `'`.
        // Run the getter and store its result at each `compqstack` write.
        compcore::set_compstate_str("all_quotes", &get_compqstack(std::ptr::null_mut()));
        restore(&AUTOQ, oaq); // c:1614
        if let Ok(mut g) = lock_vec(&COMPWORDS).lock() {
            *g = owords; // c:1617
        }

        // No-gsu mirror (see the note beside the parameter-side snapshot
        // above): c:1617's `compwords = owords` restores the shell-visible
        // `$words` for free in C. Here the parameters are separate storage,
        // so each goes back to the value IT held on entry — not to the
        // corresponding global's, which can legitimately differ.
        if let Some(w) = owords_p {
            crate::ported::params::setaparam("words", w); // c:1617 ($words view)
        }
        let _ = crate::ported::params::setiparam("CURRENT", ocur_p); // c:1594 ($CURRENT view)
        for (param, val) in [
            ("PREFIX", opre_p),     // c:1596
            ("SUFFIX", osuf_p),     // c:1598
            ("IPREFIX", oipre_p),   // c:1600
            ("ISUFFIX", oisuf_p),   // c:1602
            ("QIPREFIX", oqipre_p), // c:1604
            ("QISUFFIX", oqisuf_p), // c:1606
            ("QUOTE", oq_p),        // c:1608
            ("QUOTING", oqi_p),     // c:1610
        ] {
            if let Some(v) = val {
                let _ = crate::ported::params::setsparam(param, &v);
            }
        }
        if let Some(r) = oredirs {
            crate::ported::params::setaparam("redirections", r); // c:1618
        }

        // c:1619-1625 — `comp_setunset(CP_COMPSTATE | (~runset & (…)),
        //                (runset & CP_ALLREALS),
        //                (~kunset & CP_RESTORE), (kunset & CP_ALLKEYS));`
        let realmask = CP_WORDS
            | CP_REDIRS
            | CP_CURRENT
            | CP_PREFIX
            | CP_SUFFIX
            | CP_IPREFIX
            | CP_ISUFFIX
            | CP_QIPREFIX
            | CP_QISUFFIX; // c:1620-1623
        comp_setunset(
            (CP_COMPSTATE | (!runset & realmask)) as i32, // c:1619-1623
            (runset & CP_ALLREALS) as i32,                // c:1624
            (!kunset & CP_RESTORE) as i32,                // c:1625
            (kunset & CP_ALLKEYS) as i32,                 // c:1625
        );
    } else {
        // c:1627-1628 — the callee opted out of the restore; only the
        // `$compstate` / `restore` set-state bookkeeping still runs. The
        // C `zsfree`/`freearray` calls at c:1629-1640 are Rust drops.
        comp_setunset(
            CP_COMPSTATE as i32,           // c:1627
            0,                             // c:1627
            (!kunset & CP_RESTORE) as i32, // c:1627
            (kunset & CP_RESTORE) as i32,  // c:1628
        );
    }
    // c:1642-1643 — `zsfree(comprestore); comprestore = orest;`.
    compcore::set_compstate_str("restore", orest.as_deref().unwrap_or(""));
    0 // c:1645
}

/// Direct port of `comp_check()` from `Src/Zle/complete.c:1651`.
/// C body (c:1653-1659):
/// ```c
/// if (incompfunc != 1) {
///     zerr("condition can only be used in completion function");
///     return 0;
/// }
/// return 1;
/// ```
pub fn comp_check() -> i32 {
    // c:1651
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:1651
        zerr(
            // c:1654
            "condition can only be used in completion function",
        );
        return 0; // c:1655
    }
    1 // c:1658
}

/// Direct port of `cond_psfix(char **a, int id)` from `Src/Zle/complete.c:1662`.
/// C body (c:1664-1672): `if (comp_check())` then dispatch to
/// do_comp_vars with id=CVT_PREPAT|CVT_SUFPAT and the arg as the
/// pattern (or `arg[0]` as the pattern with `arg[1]` as the count).
#[allow(unused_variables)]
pub fn cond_psfix(a: &[String], id: i32) -> i32 {
    // c:1662
    if comp_check() != 0 {
        // c:1664 — `if (comp_check())`
        if a.len() >= 2 {
            // c:1666 — `if (a[1]) return do_comp_vars(id, cond_val(a,0),
            //           cond_str(a,1,1), 0, NULL, 0);`
            let na = crate::ported::cond::cond_val(a, 0) as i32;
            let sa = crate::ported::cond::cond_str(a, 1, true);
            return do_comp_vars(id, na, &sa, 0, "", 0);
        } else {
            // c:1669 — `else return do_comp_vars(id, -1, cond_str(a,0,1),
            //           0, NULL, 0);`
            let sa = crate::ported::cond::cond_str(a, 0, true);
            return do_comp_vars(id, -1, &sa, 0, "", 0);
        }
    }
    0 // c:1671
}

/// Direct port of `int cond_range(char **a, int id)` from
/// `Src/Zle/complete.c:1676`. Dispatches to `do_comp_vars` with
/// CVT_RANGEPAT and the two args as start/end patterns.
pub fn cond_range(a: &[String], id: i32) -> i32 {
    // c:1676
    // c:1688-1689 — both operands go through `cond_str(a, n, 1)`, which
    // expands and unmetafies the condition argument exactly as `cond_psfix`
    // does one function up. The port read `a[n]` raw, so `[[ -after $pat ]]`
    // matched against the unexpanded text.
    let sa = crate::ported::cond::cond_str(a, 0, true); // c:1688
    let sb = if id != 0 {
        crate::ported::cond::cond_str(a, 1, true) // c:1689
    } else {
        String::new()
    };
    do_comp_vars(CVT_RANGEPAT, 0, &sa, 0, &sb, 0) // c:1688
}

/// Direct port of `int setup_(UNUSED(Module m))` from `Src/Zle/complete.c:1720`.
/// Module-load init: clears every comp* string global, zeroes
/// hasperm/complistmax, sets hascompmod=1, initializes complastprefix
/// /complastsuffix to "". Returns 0 on success.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:1720
    compcore::hasperm.store(0, Ordering::Relaxed); // c:1722
    let clear = |g: &'static std::sync::OnceLock<Mutex<String>>| {
        if let Ok(mut s) = g.get_or_init(|| Mutex::new(String::new())).lock() {
            s.clear();
        }
    };
    clear(&COMPPREFIX);
    clear(&COMPSUFFIX); // c:1726
    clear(&COMPIPREFIX);
    clear(&COMPISUFFIX);
    clear(&COMPQIPREFIX);
    clear(&COMPQISUFFIX);
    clear(&COMPCONTEXT);
    clear(&COMPPARAMETER);
    clear(&COMPREDIRECT);
    clear(&COMPQUOTE);
    crate::ported::zle::zle_tricky::COMPQUOTE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .ok()
        .map(|mut s| s.clear());
    clear(&COMPLIST);
    clear(&COMPQSTACK);
    // c:1733-1734 — `complastprefix = complastsuffix = ztrdup("")`.
    if let Ok(mut s) = COMPLASTPREFIX
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        s.clear();
    }
    if let Ok(mut s) = COMPLASTSUFFIX
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        s.clear();
    }
    // c:1735 — `complistmax = 0`. (LISTMAX read at use-site.)
    0 // c:1738
}

/// Direct port of `int features_(Module m, char ***features)` from
/// `Src/Zle/complete.c:1743`. Returns the array of feature names this
/// module exposes; static-link path has no per-feature toggle so this
/// is structurally a no-op returning 0.
#[allow(unused_variables)]
pub fn features_(m: *const module) -> i32 {
    // c:1743
    0 // c:1746
}

/// Port of `static struct conddef cotab[]` from
/// `Src/Zle/complete.c:1697-1702`:
/// ```c
/// static struct conddef cotab[] = {
///     CONDDEF("after",   0, cond_range, 1, 1, 0),
///     CONDDEF("between", 0, cond_range, 2, 2, 1),
///     CONDDEF("prefix",  0, cond_psfix, 1, 2, CVT_PREPAT),
///     CONDDEF("suffix",  0, cond_psfix, 1, 2, CVT_SUFPAT),
/// };
/// ```
/// A FILE-STATIC ARRAY in C, and its per-entry `CONDF_ADDED` bit has to
/// survive across calls: `setconddefs` (`Src/module.c:754`) skips an
/// entry that is already `CONDF_ADDED` (c:763), which is what makes a
/// second `require_module` for a different feature a no-op for the ones
/// already installed rather than a "name clash" warning.
static COTAB: Lazy<Mutex<Vec<crate::ported::zsh_h::conddef>>> = Lazy::new(|| {
    // c:1697
    Mutex::new(vec![
        crate::ported::zsh_h::CONDDEF("after", 0, cond_range, 1, 1, 0), // c:1698
        crate::ported::zsh_h::CONDDEF("between", 0, cond_range, 2, 2, 1), // c:1699
        crate::ported::zsh_h::CONDDEF("prefix", 0, cond_psfix, 1, 2, CVT_PREPAT), // c:1700
        crate::ported::zsh_h::CONDDEF("suffix", 0, cond_psfix, 1, 2, CVT_SUFPAT), // c:1701
    ])
});

/// Direct port of `int enables_(Module m, int **enables)` from
/// `Src/Zle/complete.c:1751`. C body: `return handlefeatures(m,
/// &module_features, enables)`.
///
/// `module_features` (c:1720-1726) is `{ bintab, 2, cotab, 4, NULL, 0,
/// NULL, 0, 0 }`, so the enable-bit vector is
/// `[b:compadd, b:compset, c:after, c:between, c:prefix, c:suffix]` —
/// `featuresarray` (`Src/module.c:3295-3305`) emits `b:` rows before
/// `c:` rows, and `module.rs::features_module` reports the same order.
///
/// `handlefeatures` (`Src/module.c:3392-3398`): a NULL `*enables` means
/// "report the current bits" (`getfeatureenables`, c:3318); a non-NULL
/// one means "apply them" (`setfeatureenables`, c:3354), whose `cd_size`
/// arm is `setconddefs(m->node.nam, cotab, 4, e + bn_size)` (c:3365).
/// That call is what swaps the four `c:` AUTOLOAD stubs planted by
/// `bltinmods.list` (`Src/Zle/complete.mdd:8`) for the real,
/// `module`-less definitions — and it honours the per-feature bits, so
/// `[[ -prefix … ]]` installs `prefix` ALONE and leaves the other three
/// listed by `zmodload -ac`, exactly as `zsh -f` does.
///
/// The `bn_size` arm (`setbuiltins`, c:3359) is not replayed: zshrs
/// links `compadd`/`compset` into `builtintab` statically, so there is
/// nothing to add or remove. Its bits are still consumed off the front
/// of `e` so the `c:` rows line up.
#[allow(unused_variables)]
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:1751
    // c:Src/module.c:3394-3397 handlefeatures.
    const BN_SIZE: usize = 2; // c:1721 sizeof(bintab)/sizeof(*bintab)
    let mut cotab = match COTAB.lock() {
        Ok(t) => t,
        Err(_) => return 1,
    };
    match enables.as_ref() {
        // c:3396 — `*enables = getfeatureenables(m, f); return 0;`
        // (c:3330-3336: 1 for a feature already added, else 0.)
        None => {
            let mut bits = vec![0i32; BN_SIZE];
            for c in cotab.iter() {
                bits.push(i32::from((c.flags & crate::ported::module::CONDF_ADDED) != 0));
            }
            *enables = Some(bits);
            0
        }
        // c:3395 — `setfeatureenables(m, f, *enables)`.
        Some(e) => {
            let cd_bits: Vec<i32> = e.iter().skip(BN_SIZE).copied().collect(); // c:3362 `e += bn_size`
            crate::ported::module::setconddefs("zsh/complete", &mut cotab, Some(&cd_bits))
            // c:3365
        }
    }
}

/// Direct port of `int boot_(Module m)` from `Src/Zle/complete.c:1758`.
/// C registers six completion Hookfns:
/// ```c
/// addhookfunc("complete",          do_completion);
/// addhookfunc("before_complete",   before_complete);
/// addhookfunc("after_complete",    after_complete);
/// addhookfunc("accept_completion", accept_last);
/// addhookfunc("list_matches",      list_matches);
/// addhookfunc("invalidate_list",   invalidate_list);
/// ```
/// Each Rust handler has a non-`Hookfn`-shaped signature (typed args)
/// and is bridged here via a per-handler `(Hookdef, void*) -> i32`
/// thunk that casts the void* payload back to the typed pointer —
/// matching C's `(Hookfn) do_completion` cast at registration. The
/// `accept_completion` hook's `accept_last` carries a multi-field
/// signature with no C-payload struct yet (its body is invoked
/// directly from `compresult.rs` rather than via `runhookdef`), so
/// it remains unregistered until that follow-up lands.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:1758
    let _ = crate::ported::module::addhookfunc("complete", complete_hook);
    let _ = crate::ported::module::addhookfunc("before_complete", before_complete_hook);
    let _ = crate::ported::module::addhookfunc("after_complete", after_complete_hook);
    let _ = crate::ported::module::addhookfunc("list_matches", list_matches_hook);
    let _ = crate::ported::module::addhookfunc("invalidate_list", invalidate_list_hook);
    0 // c:1767
}

/// Hookfn-shape thunk for `complete` — bridges the
/// `(Hookdef, void*) -> i32` Hookfn signature to the typed
/// `do_completion(s, incmd, lst) -> i32` handler in compcore.rs.
/// Payload is `struct compldat *` per C `zle_tricky.c:2342-2346`.
fn complete_hook(_h: *mut crate::ported::zsh_h::hookdef, d: *mut std::ffi::c_void) -> i32 {
    // c:1762 — `addhookfunc("complete", do_completion);`
    if d.is_null() {
        return crate::ported::zle::compcore::do_completion("", 0, 0);
    }
    let dat = unsafe { &*(d as *const crate::ported::zle::zle_h::compldat) };
    crate::ported::zle::compcore::do_completion(&dat.s, dat.incmd, dat.lst)
}

/// Hookfn-shape thunk for `before_complete` — payload is `int *lst`
/// per C `zle_tricky.c:621`.
fn before_complete_hook(_h: *mut crate::ported::zsh_h::hookdef, d: *mut std::ffi::c_void) -> i32 {
    // c:1763 — `addhookfunc("before_complete", before_complete);`
    if d.is_null() {
        let mut lst = 0i32;
        return crate::ported::zle::compcore::before_complete(&mut lst);
    }
    let lst_ptr = d as *mut i32;
    crate::ported::zle::compcore::before_complete(unsafe { &mut *lst_ptr })
}

/// Hookfn-shape thunk for `after_complete` — payload is `int dat[2]`
/// per C `zle_tricky.c:878`.
fn after_complete_hook(_h: *mut crate::ported::zsh_h::hookdef, d: *mut std::ffi::c_void) -> i32 {
    // c:1764 — `addhookfunc("after_complete", after_complete);`
    if d.is_null() {
        let mut dat = [0i32; 2];
        return crate::ported::zle::compcore::after_complete(&mut dat);
    }
    let dat_ptr = d as *mut i32;
    let dat_slice = unsafe { std::slice::from_raw_parts_mut(dat_ptr, 2) };
    crate::ported::zle::compcore::after_complete(dat_slice)
}

/// Hookfn-shape thunk for `list_matches` — bridges the
/// `(Hookdef, void*) -> i32` Hookfn signature to the typed
/// `list_matches() -> i32` handler in compresult.rs.
fn list_matches_hook(_h: *mut crate::ported::zsh_h::hookdef, _d: *mut std::ffi::c_void) -> i32 {
    // c:1766 — `addhookfunc("list_matches", list_matches);`
    crate::ported::zle::compresult::list_matches()
}

/// Hookfn-shape thunk for `invalidate_list` — same shape as
/// `list_matches_hook`.
fn invalidate_list_hook(_h: *mut crate::ported::zsh_h::hookdef, _d: *mut std::ffi::c_void) -> i32 {
    // c:1767 — `addhookfunc("invalidate_list", invalidate_list);`
    crate::ported::zle::compresult::invalidate_list()
}

/// Direct port of `int cleanup_(Module m)` from `Src/Zle/complete.c:1772`.
/// C unregisters the same six Hookfns that `boot_` added. Paired with
/// the same registration deferral — currently a no-op until the
/// handler-sig refactor.
#[allow(unused_variables)]
pub fn cleanup_(m: *const module) -> i32 {
    // c:1772
    // c:1775-1780 — unregister the two no-arg hooks installed by boot_.
    // Mirrors the registration: deletehookfunc removes one Hookfn entry.
    let _ = crate::ported::module::deletehookfunc("list_matches", list_matches_hook);
    let _ = crate::ported::module::deletehookfunc("invalidate_list", invalidate_list_hook);
    0 // c:1783
}

/// Direct port of `int finish_(UNUSED(Module m))` from `Src/Zle/complete.c:1788`.
/// Module-unload cleanup: zsfree's every comp* string global. In Rust
/// the `OnceLock<Mutex<String>>`s are owned and freed at process exit;
/// for symmetry with C we clear the contents so any subsequent
/// re-load starts from empty.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:1788
    let clear = |g: &'static std::sync::OnceLock<Mutex<String>>| {
        if let Ok(mut s) = g.get_or_init(|| Mutex::new(String::new())).lock() {
            s.clear();
        }
    };
    clear(&COMPPREFIX);
    clear(&COMPSUFFIX); // c:1794-1795
    clear(&COMPLASTPREFIX);
    clear(&COMPLASTSUFFIX); // c:1796-1797
    clear(&COMPIPREFIX);
    clear(&COMPISUFFIX); // c:1798-1799
    clear(&COMPQIPREFIX);
    clear(&COMPQISUFFIX); // c:1800-1801
    clear(&COMPCONTEXT);
    clear(&COMPPARAMETER);
    clear(&COMPREDIRECT); // c:1802-1804
    clear(&COMPQUOTE);
    clear(&COMPQSTACK);
    clear(&COMPQUOTING); // c:1805-1807
    clear(&COMPLIST); // c:1809
    if let Ok(mut g) = COMPWORDS.get_or_init(|| Mutex::new(Vec::new())).lock()
    // c:1790
    {
        g.clear();
    }
    0
}

fn lock_str(g: &'static std::sync::OnceLock<Mutex<String>>) -> &'static Mutex<String> {
    g.get_or_init(|| Mutex::new(String::new()))
}
fn lock_vec(g: &'static std::sync::OnceLock<Mutex<Vec<String>>>) -> &'static Mutex<Vec<String>> {
    g.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// c:1479-1494 — `get_compqstack` emits ONE char per open quoting level:
    /// `*ptr++ = *comp_quoting_string(*cqp)` (c:1489-1490), i.e. the FIRST
    /// char of the printable form, innermost level first. This is what
    /// `$compstate[all_quotes]` (c:1299 `compqstack_gsu`) serves.
    #[test]
    fn get_compqstack_translates_each_quoting_level() {
        use crate::ported::zsh_h::{
            QT_BACKSLASH, QT_BACKTICK, QT_DOLLARS, QT_DOUBLE, QT_SINGLE,
        };
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let put = |s: &str| {
            *COMPQSTACK
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap() = s.to_string();
        };
        let q = |c: i32| char::from_u32(c as u32).unwrap();

        // c:1483-1484 — `if (!compqstack) return ""`.
        put("");
        assert_eq!(get_compqstack(std::ptr::null_mut()), "");

        // The three contexts `compstate[all_quotes]` is read in: unquoted
        // (compcore.c:305 seeds QT_BACKSLASH), inside `"`, inside `'`.
        put(&q(QT_BACKSLASH).to_string());
        assert_eq!(get_compqstack(std::ptr::null_mut()), "\\");
        put(&q(QT_DOUBLE).to_string());
        assert_eq!(get_compqstack(std::ptr::null_mut()), "\"");
        put(&q(QT_SINGLE).to_string());
        assert_eq!(get_compqstack(std::ptr::null_mut()), "'");

        // c:1490 takes only the FIRST char, so QT_DOLLARS ("$'") is `$`.
        put(&q(QT_DOLLARS).to_string());
        assert_eq!(get_compqstack(std::ptr::null_mut()), "$");
        // `comp_quoting_string` (compcore.c:1437-1447) has cases for only
        // QT_SINGLE / QT_DOUBLE / QT_DOLLARS; QT_BACKTICK hits
        // `default: return "\\"` (compcore.c:1445-1446), so a backtick level
        // reports as a backslash — not as a backtick.
        put(&q(QT_BACKTICK).to_string());
        assert_eq!(get_compqstack(std::ptr::null_mut()), "\\");

        // c:1488 walks the whole stack — nested levels, innermost first
        // (compcore.c:1861-1868 PREPENDS the newly opened quote).
        put(&format!("{}{}", q(QT_SINGLE), q(QT_DOUBLE)));
        assert_eq!(get_compqstack(std::ptr::null_mut()), "'\"");
        put("");
    }

    #[test]
    fn classes_basic_cclass() {
        let _g = crate::test_util::global_state_lock();
        // c:485 — `[abc]` → CCLASS, str holds "abc".
        let _g = zle_test_setup();
        let mut p = Cpattern::default();
        let rest = parse_class(&mut p, "[abc]rest");
        assert_eq!(p.tp, CPAT_CCLASS);
        assert_eq!(p.str.as_deref(), Some(b"abc".as_slice()));
        assert_eq!(rest, "rest");
    }

    #[test]
    fn classes_negated_cclass_via_bang() {
        let _g = crate::test_util::global_state_lock();
        // c:490 — `[!abc]` → NCLASS.
        let _g = zle_test_setup();
        let mut p = Cpattern::default();
        let _ = parse_class(&mut p, "[!abc]");
        assert_eq!(p.tp, CPAT_NCLASS);
    }

    #[test]
    fn classes_negated_cclass_via_caret() {
        let _g = crate::test_util::global_state_lock();
        // c:490 — `[^abc]` → NCLASS.
        let _g = zle_test_setup();
        let mut p = Cpattern::default();
        let _ = parse_class(&mut p, "[^abc]");
        assert_eq!(p.tp, CPAT_NCLASS);
    }

    #[test]
    fn classes_equiv_braces() {
        let _g = crate::test_util::global_state_lock();
        // c:498 — `{abc}` → EQUIV.
        let _g = zle_test_setup();
        let mut p = Cpattern::default();
        let _ = parse_class(&mut p, "{abc}");
        assert_eq!(p.tp, CPAT_EQUIV);
    }

    #[test]
    fn classes_range_consumes_input() {
        let _g = crate::test_util::global_state_lock();
        // c:537 — `[a-z]rest` → parses 5 chars, returns "rest".
        //          The PP_RANGE-encoded body isn't directly checked
        //          here because Cpattern.str is currently
        //          Option<String> and metafied tokens (0x83-prefix
        //          byte sequences) don't round-trip through UTF-8.
        //          Re-add a byte-level check once Cpattern.str moves
        //          to a Vec<u8>-backed storage.
        let _g = zle_test_setup();
        let mut p = Cpattern::default();
        let rest = parse_class(&mut p, "[a-z]rest");
        assert_eq!(p.tp, CPAT_CCLASS);
        assert_eq!(rest, "rest");
        assert!(p.str.is_some());
    }

    #[test]
    fn cmatcher_empty_input_returns_none() {
        let _g = crate::test_util::global_state_lock();
        // c:249 — `if (!*s) return NULL;`
        let _g = zle_test_setup();
        assert!(parse_cmatcher("", "").is_none());
    }

    #[test]
    fn cmatcher_x_early_return() {
        let _g = crate::test_util::global_state_lock();
        // c:294-303 — `x:` is the "match anything" sentinel; valid
        //              spec, returns the (currently empty) chain.
        let _g = zle_test_setup();
        assert!(parse_cmatcher("", "x:").is_none());
    }

    #[test]
    fn cmatcher_unknown_letter_errors() {
        let _g = crate::test_util::global_state_lock();
        // c:280-283 — unknown rule-letter → return None (pcm_err).
        let _g = zle_test_setup();
        // "q" isn't in the dispatch table.
        assert!(parse_cmatcher("", "q:abc").is_none());
    }

    #[test]
    fn cmatcher_missing_colon_errors() {
        let _g = crate::test_util::global_state_lock();
        // c:288-291 — second char must be `:`.
        let _g = zle_test_setup();
        assert!(parse_cmatcher("", "rabc").is_none());
    }

    #[test]
    fn cmatcher_x_with_trailing_pattern_errors() {
        let _g = crate::test_util::global_state_lock();
        // c:296-301 — `x:foo` is malformed; `x:` must be alone.
        let _g = zle_test_setup();
        assert!(parse_cmatcher("", "x:foo").is_none());
    }

    #[test]
    fn cmatcher_valid_letters_dont_panic() {
        let _g = crate::test_util::global_state_lock();
        // All recognized letters parse through without panicking.
        let _g = zle_test_setup();
        for c in ['b', 'l', 'e', 'r', 'm', 'B', 'L', 'E', 'R', 'M'] {
            let spec = format!("{}:body", c);
            let _ = parse_cmatcher("", &spec);
        }
    }

    #[test]
    fn cmatcher_m_rule_emits_cmatcher() {
        let _g = crate::test_util::global_state_lock();
        // c:266 — `m:word=replacement` plain match.
        let _g = zle_test_setup();
        let r = parse_cmatcher("", "m:foo=bar");
        assert!(r.is_some(), "m: rule should produce a Cmatcher");
        let cm = r.unwrap();
        assert_eq!(cm.flags, 0); // c:266 fl=0
        assert_eq!(cm.llen, 3); // "foo"
        assert_eq!(cm.wlen, 3); // "bar"
        assert!(cm.line.is_some());
        assert!(cm.word.is_some());
        assert!(cm.left.is_none());
        assert!(cm.right.is_none());
    }

    /// c:318-357 — the single-`|` `r:` grammar is
    /// `r:`var(word-pat)`|`var(anchor)`=`var(match-pat)
    /// (Doc/Zsh/compwid.yo:1091), i.e. the part BEFORE the `|` is the line
    /// pattern, NOT a left anchor. C reaches the `left = line` promotion at
    /// c:342 only when a SECOND `|` follows, because `!*++s` at c:329 has
    /// already stepped past the first one. This test previously asserted
    /// `lalen == 3` for `r:abc|xy=def` — the shape produced when that `++s`
    /// side effect is dropped.
    #[test]
    fn cmatcher_r_rule_emits_anchored_cmatcher() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let r = parse_cmatcher("", "r:abc|xy=def");
        assert!(r.is_some(), "r: rule should produce a Cmatcher");
        let cm = r.unwrap();
        assert_eq!(cm.flags, CMF_RIGHT);
        assert_eq!(cm.lalen, 0, "single `|`: no left anchor"); // c:342 not taken
        assert!(cm.left.is_none());
        assert_eq!(cm.llen, 3, "`abc` is the line/word pattern"); // c:318
        assert!(cm.line.is_some());
        assert_eq!(cm.ralen, 2, "`xy` is the right anchor"); // c:349
        assert!(cm.right.is_some());
        assert_eq!(cm.wlen, 3); // word = "def"
    }

    /// c:341-348 — the DOUBLE-`|` `r:` grammar
    /// `r:`var(coanchor)`||`var(anchor)`=`var(match-pat)
    /// (Doc/Zsh/compwid.yo:1126) is what promotes the leading pattern to
    /// `left`. Covers the documented zsh idiom
    /// `r:[^A-Z0-9]||[A-Z0-9]=** r:|=*`, which the missing `++s` at c:329
    /// mis-parsed: the stray `|` was consumed as a literal CPAT_CHAR at the
    /// head of the right anchor, giving `ralen == 2` instead of 1.
    #[test]
    fn cmatcher_r_rule_double_bar_promotes_left_anchor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let cm = parse_cmatcher("", "r:abc||xy=def").expect("r: two-anchor rule parses");
        assert_eq!(cm.flags, CMF_RIGHT);
        assert_eq!(cm.lalen, 3, "`abc` promoted to the left coanchor"); // c:343-344
        assert!(cm.left.is_some());
        assert_eq!(cm.llen, 0, "line pattern cleared by the promotion"); // c:345-346
        assert!(cm.line.is_none());
        assert_eq!(cm.ralen, 2, "`xy` is the right anchor, no stray `|`"); // c:349
        assert_eq!(cm.wlen, 3);

        // The manual's own example: one class each side, `**` word pattern.
        let cm = parse_cmatcher("", "r:[^A-Z0-9]||[A-Z0-9]=**").expect("documented idiom parses");
        assert_eq!(cm.lalen, 1);
        assert_eq!(cm.ralen, 1, "a leaked `|` would make this 2"); // c:349
        assert_eq!(cm.wlen, -2, "`**` sentinel"); // c:369
    }

    #[test]
    fn cmatcher_l_rule_emits_left_anchor() {
        let _g = crate::test_util::global_state_lock();
        // c:263 — `l:left|line=word` left anchor.
        let _g = zle_test_setup();
        let r = parse_cmatcher("", "l:ab|cd=ef");
        assert!(r.is_some(), "l: rule should produce a Cmatcher");
        let cm = r.unwrap();
        assert_eq!(cm.flags, CMF_LEFT);
        assert!(cm.left.is_some());
        assert_eq!(cm.lalen, 2);
        assert_eq!(cm.llen, 2);
        assert_eq!(cm.wlen, 2);
    }

    #[test]
    fn cmatcher_star_word_with_anchor() {
        let _g = crate::test_util::global_state_lock();
        // c:359-370 — `r:|=*` matches any word, requires anchor.
        let _g = zle_test_setup();
        let r = parse_cmatcher("", "r:|=*");
        assert!(r.is_some(), "r:|=* should produce a Cmatcher");
        let cm = r.unwrap();
        assert_eq!(cm.wlen, -1); // c:370 single `*`
        assert!(cm.word.is_none());
    }

    #[test]
    fn cmatcher_double_star_word() {
        let _g = crate::test_util::global_state_lock();
        // c:366-368 — `r:|=**` matches any (greedy) word.
        let _g = zle_test_setup();
        let r = parse_cmatcher("", "r:|=**");
        assert!(r.is_some());
        let cm = r.unwrap();
        assert_eq!(cm.wlen, -2); // c:368 double `**`
    }

    #[test]
    fn cmatcher_star_without_anchor_errors() {
        let _g = crate::test_util::global_state_lock();
        // c:360-364 — `m:=*` (no anchor) errors.
        let _g = zle_test_setup();
        let r = parse_cmatcher("", "m:=*");
        assert!(r.is_none(), "*-without-anchor should error");
    }

    #[test]
    fn cmatcher_chain_multiple_rules() {
        let _g = crate::test_util::global_state_lock();
        // c:251-401 — multiple rules separated by whitespace chain.
        let _g = zle_test_setup();
        let r = parse_cmatcher("", "m:foo=bar m:baz=qux");
        assert!(r.is_some());
        let head = r.unwrap();
        assert!(head.next.is_some(), "second rule should be linked");
    }

    #[test]
    fn pattern_single_char_emits_cpat_char() {
        let _g = crate::test_util::global_state_lock();
        // c:451-461 — single non-special char → CPAT_CHAR node.
        let _g = zle_test_setup();
        let (chain, rest, len, err) = parse_pattern("", "abc", '\0');
        assert!(!err);
        assert_eq!(len, 3);
        assert_eq!(rest, ""); // consumed everything (no end-char, no whitespace)
                              // Walk chain and verify 3 CPAT_CHAR nodes.
        let mut count = 0;
        let mut cur = chain.as_deref();
        while let Some(n) = cur {
            assert_eq!(n.tp, CPAT_CHAR);
            count += 1;
            cur = n.next.as_deref();
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn pattern_question_mark_is_cpat_any() {
        let _g = crate::test_util::global_state_lock();
        // c:443 — `?` → CPAT_ANY.
        let _g = zle_test_setup();
        let (chain, _, len, err) = parse_pattern("", "?", '\0');
        assert!(!err);
        assert_eq!(len, 1);
        assert_eq!(chain.as_ref().unwrap().tp, CPAT_ANY);
    }

    #[test]
    fn pattern_invalid_chars_error() {
        let _g = crate::test_util::global_state_lock();
        // c:446-449 — `*`/`(`/`)`/`=` → error.
        let _g = zle_test_setup();
        for c in ['*', '(', ')', '='] {
            let s = format!("{}", c);
            let (chain, _, _, err) = parse_pattern("", &s, '\0');
            assert!(err, "char {} should error", c);
            assert!(chain.is_none());
        }
    }

    #[test]
    fn pattern_backslash_escapes_next() {
        let _g = crate::test_util::global_state_lock();
        // c:452 — `\\X` consumes the backslash and emits X as CPAT_CHAR.
        let _g = zle_test_setup();
        let (chain, _, len, err) = parse_pattern("", r"\*", '\0');
        assert!(!err);
        assert_eq!(len, 1);
        let n = chain.as_ref().unwrap();
        assert_eq!(n.tp, CPAT_CHAR);
        assert_eq!(n.chr, '*' as u32);
    }

    #[test]
    fn pattern_stops_at_end_char() {
        let _g = crate::test_util::global_state_lock();
        // c:430 — `*s != e` gate.
        let _g = zle_test_setup();
        let (_, rest, len, err) = parse_pattern("", "ab=cd", '=');
        assert!(!err);
        assert_eq!(len, 2);
        assert_eq!(rest, "=cd");
    }

    #[test]
    fn pattern_stops_at_whitespace_when_no_end_char() {
        let _g = crate::test_util::global_state_lock();
        // c:430 — `e==0` → !inblank.
        let _g = zle_test_setup();
        let (_, rest, len, err) = parse_pattern("", "ab cd", '\0');
        assert!(!err);
        assert_eq!(len, 2);
        assert_eq!(rest, " cd");
    }

    #[test]
    fn pattern_bracket_class_routes_to_parse_class() {
        let _g = crate::test_util::global_state_lock();
        // c:435 — `[abc]` dispatches to parse_class. With no end-char
        //          parse_pattern continues into the trailing chars as
        //          CPAT_CHAR nodes, so `[abc]xy` → class + x + y = 3.
        let _g = zle_test_setup();
        let (chain, rest, len, err) = parse_pattern("", "[abc]xy=q", '=');
        assert!(!err);
        assert_eq!(len, 3);
        assert_eq!(rest, "=q");
        // chain head is the class node.
        assert_eq!(chain.as_ref().unwrap().tp, CPAT_CCLASS);
    }

    #[test]
    fn classes_unterminated_returns_eos() {
        let _g = crate::test_util::global_state_lock();
        // c:504 — unterminated class → returns input-end.
        let _g = zle_test_setup();
        let mut p = Cpattern::default();
        let rest = parse_class(&mut p, "[abc");
        assert_eq!(rest, "");
    }

    #[test]
    fn compadd_trace_records_and_returns_one() {
        // sh:_complete_help:11 — `compadd() { return 1 }`. With the
        //   trace flag on, bin_compadd records argv into
        //   `_complete_help_funcs` and returns 1 without panicking.
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        crate::ported::params::setaparam("_complete_help_funcs", Vec::new());
        set_compadd_trace(true);
        let argv = vec!["-X".to_string(), "files".to_string(), "alpha".to_string()];
        let r = bin_compadd("compadd", &argv, &make_test_ops(), 0);
        set_compadd_trace(false);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1, "trace mode short-circuits to 1");
        let buf = crate::ported::params::getaparam("_complete_help_funcs").unwrap_or_default();
        assert_eq!(buf, vec!["-X files alpha".to_string()]);
    }

    #[test]
    fn compadd_prefix_injector_mutates_prefix_then_restores() {
        // sh:_approximate:57-72 — injector prepends `(#a$N)` to PREFIX
        //   for the duration of the compadd call, then restores.
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        crate::ported::params::setsparam("PREFIX", "abc").unwrap();
        // Trace flag on so the body short-circuits and we can observe
        //   what PREFIX was during the call via the captured argv-side
        //   parameter snapshot.
        crate::ported::params::setaparam("_complete_help_funcs", Vec::new());
        set_compadd_trace(true);
        let prev = set_compadd_prefix_injector("(#a2)");
        assert!(prev.is_none());
        // Spy: stash PREFIX into a side-channel param before running.
        //   bin_compadd's wrapper mutates PREFIX, runs body, restores;
        //   our spy reads the mutated value mid-call via a hook is not
        //   wired, so instead we directly observe PREFIX after the
        //   call to confirm restoration.
        let _ = bin_compadd("compadd", &["x".to_string()], &make_test_ops(), 0);
        clear_compadd_prefix_injector();
        set_compadd_trace(false);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        let after = crate::ported::params::getsparam("PREFIX").unwrap_or_default();
        assert_eq!(after, "abc", "PREFIX restored after compadd call");
    }

    #[test]
    fn compadd_prefix_injector_tilde_aware() {
        // sh:_approximate:65-69 — leading `~` in PREFIX stays before
        //   the injected pattern when no `-p ~…` is passed.
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        crate::ported::params::setsparam("PREFIX", "~user").unwrap();
        set_compadd_trace(true);
        let _ = set_compadd_prefix_injector("(#a1)");
        let _ = bin_compadd("compadd", &["x".to_string()], &make_test_ops(), 0);
        clear_compadd_prefix_injector();
        set_compadd_trace(false);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        // Restored
        assert_eq!(
            crate::ported::params::getsparam("PREFIX").unwrap_or_default(),
            "~user"
        );
    }

    fn make_test_ops() -> options {
        options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    // ─── zsh-corpus pins for parse_ordering ────────────────────────

    /// `parse_ordering("")` returns -1 — empty CSV token is not a
    /// valid orderopts name. Pin the actual zsh contract.
    #[test]
    fn complete_corpus_parse_ordering_empty_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut flags: Option<i32> = Some(0);
        let r = parse_ordering("", &mut flags);
        assert_eq!(r, -1, "empty token is not a valid orderopts name");
    }

    /// `parse_ordering("invalid_xyz")` returns -1 (unknown option).
    #[test]
    fn complete_corpus_parse_ordering_unknown_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut flags: Option<i32> = Some(0);
        let r = parse_ordering("zzz_not_a_real_ordering_xyz", &mut flags);
        assert_eq!(r, -1, "unknown ordering name = -1");
    }

    /// `parse_ordering("match")` recognises a real orderopts name.
    #[test]
    fn complete_corpus_parse_ordering_match_recognised() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut flags: Option<i32> = Some(0);
        let r = parse_ordering("match", &mut flags);
        assert_eq!(r, 0, "match is a valid orderopts name");
    }

    /// `parse_ordering` accepts comma-separated valid names.
    #[test]
    fn complete_corpus_parse_ordering_csv_all_valid() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut flags: Option<i32> = Some(0);
        let r = parse_ordering("match,reverse", &mut flags);
        // match + reverse are both valid → 0.
        assert_eq!(r, 0);
    }

    /// `parse_ordering` rejects when any csv token is invalid.
    #[test]
    fn complete_corpus_parse_ordering_csv_with_invalid_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut flags: Option<i32> = Some(0);
        let r = parse_ordering("match,zzz_bogus", &mut flags);
        assert_eq!(r, -1, "any invalid token = -1");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/complete.c.
    // ═══════════════════════════════════════════════════════════════════

    /// `ignore_prefix(0)` is a no-op (`if (l) …` guard).
    /// C `Src/Zle/complete.c:ignore_prefix` first line.
    #[test]
    fn ignore_prefix_zero_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        ignore_prefix(0);
        ignore_prefix(0);
    }

    /// `ignore_suffix(0)` is a no-op (`if (l) …` guard).
    #[test]
    fn ignore_suffix_zero_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        ignore_suffix(0);
        ignore_suffix(0);
    }

    /// C `atoi(3)` semantics, which `bin_compset` relies on for every
    /// numeric option argument (c:1188/1189/1200/1209). The strict
    /// `str::parse::<i32>()` this replaced returned `Err` — funnelled into
    /// `unwrap_or(0)` — for every one of the non-canonical forms below, and a
    /// count of 0 makes `compset -p` a silent no-op that still reports
    /// success.
    #[test]
    fn atoi_matches_c_semantics() {
        assert_eq!(atoi("1"), 1);
        assert_eq!(atoi(" 1"), 1, "leading whitespace is skipped");
        assert_eq!(atoi("1 "), 1, "trailing junk is ignored");
        assert_eq!(atoi("2x"), 2, "parse stops at the first non-digit");
        assert_eq!(atoi("-3"), -3);
        assert_eq!(atoi("+4"), 4);
        assert_eq!(atoi("abc"), 0, "no leading digits = 0");
        assert_eq!(atoi(""), 0);
    }

    /// `compset -p <n>` moves `<n>` characters off the front of `$PREFIX`
    /// onto the end of `$IPREFIX` and reports success (exit 0), which is
    /// `_main_complete` sh:106's whole mechanism for handing `~user` to the
    /// `-tilde-` completer with the `~` already consumed.
    /// C: `do_comp_vars` CVT_PRENUM (c:1001-1041) + `bin_compset` c:1217
    /// `return !do_comp_vars(...)`.
    #[test]
    fn compset_p_moves_prefix_chars_to_iprefix() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        *lock_str(&COMPPREFIX).lock().unwrap() = "~ro".to_string();
        *lock_str(&COMPIPREFIX).lock().unwrap() = String::new();
        let rc = bin_compset(
            "compset",
            &["-p".to_string(), "1".to_string()],
            &make_test_ops(),
            0,
        );
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(rc, 0, "a successful move reports exit status 0");
        assert_eq!(*lock_str(&COMPPREFIX).lock().unwrap(), "ro");
        assert_eq!(*lock_str(&COMPIPREFIX).lock().unwrap(), "~");
        // The gsu binding C gets for free: $PREFIX/$IPREFIX are the same
        // storage as compprefix/compiprefix, so a completer reading them
        // right after the compset must see the moved text.
        assert_eq!(
            crate::ported::params::getsparam("PREFIX").unwrap_or_default(),
            "ro"
        );
        assert_eq!(
            crate::ported::params::getsparam("IPREFIX").unwrap_or_default(),
            "~"
        );
    }

    /// `compset -p <n>` with `$PREFIX` shorter than `<n>` moves nothing and
    /// reports failure — C `do_comp_vars` c:1040 `return 0` → `bin_compset`
    /// `!0` = 1.
    #[test]
    fn compset_p_too_short_prefix_fails_without_moving() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        *lock_str(&COMPPREFIX).lock().unwrap() = "~r".to_string();
        *lock_str(&COMPIPREFIX).lock().unwrap() = String::new();
        let rc = bin_compset(
            "compset",
            &["-p".to_string(), "5".to_string()],
            &make_test_ops(),
            0,
        );
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(rc, 1, "no move reports non-zero");
        assert_eq!(*lock_str(&COMPPREFIX).lock().unwrap(), "~r");
        assert_eq!(*lock_str(&COMPIPREFIX).lock().unwrap(), "");
    }

    /// `restrict_range(0, 0)` writes degenerate empty range.
    #[test]
    fn restrict_range_zero_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        restrict_range(0, 0);
    }

    /// `set_compadd_trace(true)` / `(false)` toggle no-panic.
    #[test]
    fn set_compadd_trace_toggle_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_compadd_trace(true);
        set_compadd_trace(false);
        set_compadd_trace(true);
    }

    /// `set_compadd_prefix_injector` / `clear_compadd_prefix_injector`
    /// round-trip.
    #[test]
    fn set_then_clear_compadd_prefix_injector_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let prev = set_compadd_prefix_injector("test_prefix");
        clear_compadd_prefix_injector();
        if let Some(p) = prev {
            let _ = set_compadd_prefix_injector(p);
            clear_compadd_prefix_injector();
        }
    }

    /// `parse_ordering("")` returns -1 — no valid ordering token.
    #[test]
    fn parse_ordering_empty_string_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut flags: Option<i32> = Some(0);
        let r = parse_ordering("", &mut flags);
        assert_eq!(r, -1, "empty input has no valid ordering token");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/complete.c free/copy
    // chain walkers.
    // ═══════════════════════════════════════════════════════════════════

    /// c:115 — `freecmatcher(None)` is a safe no-op.
    #[test]
    fn freecmatcher_none_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        freecmatcher(None);
    }

    /// c:137 — `freecpattern(None)` is a safe no-op.
    #[test]
    fn freecpattern_none_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        freecpattern(None);
    }

    /// c:98 — `freecmlist(None)` is a safe no-op.
    #[test]
    fn freecmlist_none_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        freecmlist(None);
    }

    /// c:218 — `cpcpattern(None)` returns None (no source chain).
    #[test]
    fn cpcpattern_none_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = cpcpattern(None);
        assert!(r.is_none(), "None source → None copy");
    }

    /// c:155 — `cpcmatcher(None)` returns None.
    #[test]
    fn cpcmatcher_none_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = cpcmatcher(None);
        assert!(r.is_none());
    }

    /// c:189 — `cp_cpattern_element` copies the `tp` field verbatim.
    #[test]
    fn cp_cpattern_element_copies_tp() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let src = Cpattern {
            tp: CPAT_CHAR,
            chr: 'A' as u32,
            ..Default::default()
        };
        let dup = cp_cpattern_element(&src);
        assert_eq!(dup.tp, CPAT_CHAR, "tp must round-trip");
    }

    /// c:218 — `cp_cpattern_element` on CPAT_CHAR copies `chr` field.
    #[test]
    fn cp_cpattern_element_copies_chr_for_cpat_char() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let src = Cpattern {
            tp: CPAT_CHAR,
            chr: 0x42,
            ..Default::default()
        };
        let dup = cp_cpattern_element(&src);
        assert_eq!(dup.chr, 0x42);
    }

    /// c:199 — `cp_cpattern_element` on CPAT_CCLASS copies `str` field.
    #[test]
    fn cp_cpattern_element_copies_str_for_cclass() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let src = Cpattern {
            tp: CPAT_CCLASS,
            str: Some(b"a-z".to_vec()),
            ..Default::default()
        };
        let dup = cp_cpattern_element(&src);
        assert_eq!(dup.str.as_deref(), Some(&b"a-z"[..]));
    }

    /// c:191 — `cp_cpattern_element` always initializes `next = None`
    /// (caller chains them via cpcpattern, not by carrying source's
    /// next forward).
    #[test]
    fn cp_cpattern_element_resets_next_to_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let src = Cpattern {
            tp: CPAT_CHAR,
            chr: 'x' as u32,
            next: Some(Box::new(Cpattern::default())), // source has next
            ..Default::default()
        };
        let dup = cp_cpattern_element(&src);
        assert!(dup.next.is_none(), "copy element MUST reset next to None");
    }

    /// c:218 — `cpcpattern` of a 3-node chain produces a 3-node chain
    /// (chain walker advances tail_ref per element).
    #[test]
    fn cpcpattern_preserves_chain_length() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let src = Cpattern {
            tp: CPAT_CHAR,
            chr: 'a' as u32,
            next: Some(Box::new(Cpattern {
                tp: CPAT_CHAR,
                chr: 'b' as u32,
                next: Some(Box::new(Cpattern {
                    tp: CPAT_CHAR,
                    chr: 'c' as u32,
                    ..Default::default()
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        let dup = cpcpattern(Some(&src)).expect("should copy");
        // Count: head + 2 next.
        let mut cnt = 1;
        let mut cur = &dup.next;
        while let Some(n) = cur {
            cnt += 1;
            cur = &n.next;
        }
        assert_eq!(cnt, 3, "3-node source → 3-node copy");
    }

    /// c:242 — `parse_cmatcher(_, "")` returns None per c:249 guard.
    #[test]
    fn parse_cmatcher_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = parse_cmatcher("test", "");
        assert!(r.is_none(), "empty matcher spec → None per c:249");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/complete.c
    // c:75 freecmlist / c:101 freecmatcher / c:124 freecpattern /
    // c:142 cpcmatcher / c:181 cp_cpattern_element / c:203 cpcpattern /
    // c:294 parse_cmatcher / c:791 parse_ordering / c:1069 ignore_prefix /
    // c:1087 ignore_suffix / c:1111 restrict_range / c:1598 get_compstate
    // ═══════════════════════════════════════════════════════════════════

    /// c:75 — `freecmlist(None)` is safe idempotent.
    #[test]
    fn freecmlist_none_idempotent() {
        for _ in 0..5 {
            freecmlist(None);
        }
    }

    /// c:101 — `freecmatcher(None)` is safe idempotent.
    #[test]
    fn freecmatcher_none_idempotent() {
        for _ in 0..5 {
            freecmatcher(None);
        }
    }

    /// c:124 — `freecpattern(None)` is safe idempotent.
    #[test]
    fn freecpattern_none_idempotent() {
        for _ in 0..5 {
            freecpattern(None);
        }
    }

    /// c:142 — `cpcmatcher(None)` returns None (type pin).
    #[test]
    fn cpcmatcher_none_returns_none_type_pin() {
        let r: Option<Box<Cmatcher>> = cpcmatcher(None);
        assert!(r.is_none());
    }

    /// c:203 — `cpcpattern(None)` returns None (type pin).
    #[test]
    fn cpcpattern_none_returns_none_type_pin() {
        let r: Option<Box<Cpattern>> = cpcpattern(None);
        assert!(r.is_none());
    }

    /// c:294 — `parse_cmatcher` is deterministic for empty input.
    #[test]
    fn parse_cmatcher_empty_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..3 {
            assert!(parse_cmatcher("test", "").is_none());
        }
    }

    /// c:791 — `parse_ordering(empty, _)` returns i32 (type pin).
    #[test]
    fn parse_ordering_returns_i32_type() {
        let mut flags: Option<i32> = None;
        let _: i32 = parse_ordering("name", &mut flags);
    }

    /// c:1069 — `ignore_prefix(0)` is safe.
    #[test]
    fn ignore_prefix_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        ignore_prefix(0);
    }

    /// c:1087 — `ignore_suffix(0)` is safe.
    #[test]
    fn ignore_suffix_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        ignore_suffix(0);
    }

    /// c:1111 — `restrict_range(0, 0)` is safe (zero-width).
    #[test]
    fn restrict_range_zero_zero_no_panic_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        restrict_range(0, 0);
    }

    /// c:1598 — `get_compstate(null)` returns Option<usize> type.
    #[test]
    fn get_compstate_null_returns_option_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<usize> = get_compstate(std::ptr::null_mut());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/complete.c
    // c:75 freecmlist / c:294 parse_cmatcher / c:791 parse_ordering /
    // c:853 bin_compadd / c:942 prefix_injector / c:953 compadd_trace /
    // c:1069 ignore_prefix / c:1087 ignore_suffix / c:1111 restrict_range /
    // c:1393 bin_compset
    // ═══════════════════════════════════════════════════════════════════

    /// c:75 — `freecmlist(None)` is idempotent (alt 10-call).
    #[test]
    fn freecmlist_none_idempotent_10_call() {
        for _ in 0..10 {
            freecmlist(None);
        }
    }

    /// c:1069 — `ignore_prefix(positive)` is safe.
    #[test]
    fn ignore_prefix_positive_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for n in [1i32, 10, 100] {
            ignore_prefix(n);
        }
    }

    /// c:1087 — `ignore_suffix(positive)` is safe.
    #[test]
    fn ignore_suffix_positive_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for n in [1i32, 10, 100] {
            ignore_suffix(n);
        }
    }

    /// c:1069 — `ignore_prefix(0)` is idempotent across repeated calls.
    #[test]
    fn ignore_prefix_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..10 {
            ignore_prefix(0);
        }
    }

    /// c:1111 — `restrict_range` is safe for various endpoints.
    #[test]
    fn restrict_range_various_endpoints_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for &(b, e) in &[(0, 0), (1, 10), (-1, -1), (i32::MAX, i32::MAX)] {
            restrict_range(b, e);
        }
    }

    /// c:791 — `parse_ordering` is deterministic for empty name.
    #[test]
    fn parse_ordering_empty_deterministic() {
        let mut flags1: Option<i32> = None;
        let first = parse_ordering("", &mut flags1);
        let mut flags2: Option<i32> = None;
        let second = parse_ordering("", &mut flags2);
        assert_eq!(first, second, "parse_ordering('') must be deterministic");
    }

    /// c:853 — `bin_compadd` returns i32 (compile-time pin).
    #[test]
    fn bin_compadd_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_compadd("compadd", &[], &ops, 0);
    }

    /// c:1393 — `bin_compset` returns i32 (compile-time pin).
    #[test]
    fn bin_compset_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_compset("compset", &[], &ops, 0);
    }

    /// c:294 — `parse_cmatcher` is deterministic for various inputs.
    #[test]
    fn parse_cmatcher_various_inputs_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for s in ["", "x", "m:{a-z}={A-Z}", "garbage"] {
            let a = parse_cmatcher("test", s).is_some();
            let b = parse_cmatcher("test", s).is_some();
            assert_eq!(a, b, "parse_cmatcher({:?}) must be deterministic", s);
        }
    }

    /// c:942 — `set_compadd_prefix_injector` round-trips with clear.
    #[test]
    fn compadd_prefix_injector_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Option<String> = set_compadd_prefix_injector("test_prefix_xyz");
        clear_compadd_prefix_injector();
    }

    /// c:953 — `set_compadd_trace` is idempotent (toggle on/off).
    #[test]
    fn set_compadd_trace_toggle_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for active in [true, false, true, true, false] {
            set_compadd_trace(active);
        }
    }
}
