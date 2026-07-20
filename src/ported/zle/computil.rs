//! Completion utility functions for ZLE
//!
//! Port from zsh/Src/Zle/computil.c (5,180 lines)
//!
//! Help for `_describe'.                                                    // c:34
//! Help for `_arguments'.                                                   // c:897
//!
//! The full utility library is in compsys/computil.rs (674 lines).
//! This module provides _describe, _values, _alternative, _combination,
//! and the compdescribe/comparguments/compvalues builtins.
//!
//! Key C functions and their Rust locations:
//! - bin_compdescribe  → crate::compsys::describe::describe()
//! - bin_comparguments → crate::compsys::arguments (full _arguments)
//! - bin_compvalues    → crate::compsys::computil::compvalues()
//! - bin_comptags      → crate::compsys::state::comptags()
//! - bin_comptry       → crate::compsys::state::comptry()

use std::os::unix::fs::MetadataExt;
use std::sync::atomic::Ordering;

use crate::ported::glob::{hasbraces, remnulargs, tokenize, xpandbraces};
use crate::ported::lex::untokenize;
use crate::ported::mem::ztrdup;
use crate::ported::params::{
    getstrvalue, getvalue, getvaluearr, locallevel, paramtab, setaparam, setarrvalue, sethparam,
    setiparam, setsparam, setstrvalue,
};
use crate::ported::pattern::{haswilds, patcompile, pattry, Patprog};
use crate::ported::string::tricat;
use crate::ported::utils::{
    adjustcolumns, inittyptab, niceztrlen, quotestring, set_noerrs, strpfx, ztrlen, zwarnnam,
};
use crate::ported::zle::comp_h::{
    Cmatcher, Cpattern, CGF_NOSORT, CGF_UNIQALL, CGF_UNIQCON, CMF_LEFT, CMF_RIGHT, CPAT_ANY,
    CPAT_CCLASS, CPAT_CHAR, CPAT_EQUIV, CPAT_NCLASS,
};
use crate::ported::zle::compcore::{begcmgroup, comppatmatch, endcmgroup, get_user_var, rembslash};
use crate::ported::zle::complete::{
    ignore_prefix, ignore_suffix, parse_cmatcher, restrict_range, COMPCURRENT, COMPPREFIX,
    COMPQSTACK, COMPSUFFIX, COMPWORDS, INCOMPFUNC,
};
use crate::ported::zle::compmatch::{pattern_match, pattern_match1, pattern_match_equivalence};
use crate::ported::zle::compresult::ztat;
use crate::ported::zsh_h::{
    isset, options, unset, value, Comma, Inbrace, Outbrace, GLOBDOTS, KSHARRAYS, MAX_OPS,
    OPT_ISSET, PM_ARRAY, PM_TYPE, PP_LOWER, PP_RANGE, PP_UPPER, QT_BACKSLASH, QT_BACKSLASH_PATTERN,
};
use crate::ported::ztype_h::{iblank, idigit, imeta, inblank};

// =====================================================================
// CRT_* — `_describe` row-type discriminator from `computil.c:79-83`.
// Drives the `cdescr` table-builder switch.
// =====================================================================

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
/// Port of `CRT_SIMPLE` from `Src/Zle/computil.c:79`. Plain match row.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
// =====================================================================
// `_describe`-completion types — direct ports of the C structs at
// Src/Zle/computil.c:40-91 (the cdset/cdstr/cdrun/cdstate chain
// the `_describe` completion path builds + processes).
// =====================================================================

// CRT_* constants already declared above (file scope).

/// Port of `typedef struct cdset *Cdset` from `Src/Zle/computil.c:36`.
pub type Cdset = Box<cdset>; // c:36

/// Direct port of `struct cdset` from `Src/Zle/computil.c:85-91`.
/// One set of matches (one `compadd` invocation worth) with its
/// compadd options + the cdstr chain.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cdset {
    // c:85
    pub next: Option<Box<cdset>>,  // c:86 Cdset next
    pub opts: Option<Vec<String>>, // c:87 char **opts
    pub strs: Option<Box<cdstr>>,  // c:88 Cdstr strs
    pub count: i32,                // c:89 int count
    pub desc: i32,                 // c:90 int desc
}
/// Port of `typedef struct cdstr *Cdstr` from `computil.c:37`.
pub type Cdstr = Box<cdstr>; // c:37

/// Direct port of `struct cdstr` from `Src/Zle/computil.c:58-70`.
/// One match string inside a `_describe` group, with optional
/// description and the same-description chain.
#[derive(Debug, Default, Clone)]
#[allow(non_camel_case_types)]
pub struct cdstr {
    // c:58
    pub next: Option<Box<cdstr>>,  // c:59 Cdstr next
    pub str: Option<String>,       // c:60 char *str
    pub desc: Option<String>,      // c:61 char *desc
    pub r#match: Option<String>,   // c:62 char *match
    pub sortstr: Option<String>,   // c:63 char *sortstr
    pub len: i32,                  // c:64 int len
    pub width: i32,                // c:65 int width
    pub other: Option<Box<cdstr>>, // c:66 Cdstr other
    pub kind: i32,                 // c:67 int kind (0/1/2)
    pub set: usize,                // c:68 Cdset set (raw ptr index)
    pub run: Option<Box<cdstr>>,   // c:69 Cdstr run
}
/// Port of `typedef struct cdrun *Cdrun` from `computil.c:38`.
pub type Cdrun = Box<cdrun>; // c:38

/// Direct port of `struct cdrun` from `Src/Zle/computil.c:72-77`.
/// One contiguous "run" of cdstr entries the shell code should
/// emit as a block.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cdrun {
    // c:72
    pub next: Option<Box<cdrun>>, // c:73 Cdrun next
    pub r#type: i32,              // c:74 int type (CRT_*)
    pub strs: Option<Box<cdstr>>, // c:75 Cdstr strs
    pub count: i32,               // c:76 int count
}

/// Direct port of `struct cdstate` from `Src/Zle/computil.c:40-56`.
/// File-static state for the `_describe` engine — holds the active
/// sets/runs/dimensions during a single `_describe` invocation.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cdstate {
    // c:40
    pub showd: i32,               // c:41
    pub sep: Option<String>,      // c:42 char *sep
    pub slen: i32,                // c:43
    pub swidth: i32,              // c:44
    pub maxmlen: i32,             // c:45
    pub sets: Option<Box<cdset>>, // c:46 Cdset sets
    pub pre: i32,                 // c:47
    pub premaxw: i32,             // c:48
    pub suf: i32,                 // c:49
    pub maxg: i32,                // c:50
    pub maxglen: i32,             // c:51
    pub groups: i32,              // c:52
    pub descs: i32,               // c:53
    pub gprew: i32,               // c:54
    pub runs: Option<Box<cdrun>>, // c:55 Cdrun runs
}
/// `CRT_SIMPLE` constant.
pub const CRT_SIMPLE: i32 = 0; // c:79
/// Port of `CRT_DESC` from `computil.c:80`. Match with description.
pub const CRT_DESC: i32 = 1; // c:80
/// Port of `CRT_SPEC` from `computil.c:81`. Special separator row.
pub const CRT_SPEC: i32 = 2; // c:81
/// Port of `CRT_DUMMY` from `computil.c:82`. Placeholder row.
pub const CRT_DUMMY: i32 = 3; // c:82
/// Port of `CRT_EXPL` from `computil.c:83`. Explanation header row.
pub const CRT_EXPL: i32 = 4; // c:83

/// Port of `static int cd_parsed` from `Src/Zle/computil.c:188`. Flag
/// signalling whether `cd_state` holds a parsed-but-unconsumed
/// description set.
pub static cd_parsed: std::sync::atomic::AtomicI32 = // c:94
    std::sync::atomic::AtomicI32::new(0);

/// Direct port of `static void freecdsets(Cdset p)` from
/// `Src/Zle/computil.c:97`. Walks the cdset `next` chain
/// freeing each set's opts/strs sub-chains and the cd_state runs
/// list at the end.
pub fn freecdsets(mut p: Option<Box<cdset>>) {
    // c:97
    while let Some(mut set) = p {
        // c:97 for (; p; ...)
        p = set.next.take(); // c:104 n = p->next
                             // c:105-106 — `if (p->opts) freearray(p->opts)`.
        set.opts = None;
        // c:107-115 — for each cdstr: free sortstr/str/desc/match.
        let mut s = set.strs.take();
        while let Some(mut node) = s {
            s = node.next.take();
            node.sortstr = None; // c:109
            node.str = None; // c:110
            node.desc = None; // c:111
                              // c:112-113 — `if (s->match != s->str) zsfree(s->match)`.
                              // Rust's Option<String> drop is unconditional; the C
                              // pointer-equality guard collapses out.
            node.r#match = None;
            drop(node); // c:114
        }
        // c:116-119 — drain cd_state.runs.
        if let Ok(mut st) = cd_state.lock() {
            let mut r = st.runs.take();
            while let Some(mut run) = r {
                r = run.next.take();
                drop(run); // c:118
            }
        }
        drop(set); // c:120
    }
}

/// Direct port of `static void cd_group(int maxg)` from
/// `Src/Zle/computil.c:127-182`. Walks `cd_state.sets` looking for
/// matches sharing the same description; links them via the `other`
/// chain on the first cdstr of each group. Sets `cd_state.groups`,
/// `descs`, `maxg`, `maxglen` accordingly.
pub fn cd_group(maxg: i32) {
    // c:127
    let mut st = cd_state.lock().unwrap();
    st.groups = 0; // c:133
    st.descs = 0;
    st.maxglen = 0;
    st.maxg = 0;

    // c:136-140 — reset kind/other on every cdstr.
    // Rust port: walk via raw pointers since we need mutable access
    // through nested chains while holding the set borrow.
    let st_ptr: *mut cdstate = &mut *st;
    unsafe {
        let mut set = (*st_ptr).sets.as_deref_mut();
        while let Some(s) = set {
            let mut sp = s.strs.as_deref_mut();
            while let Some(sn) = sp {
                sn.kind = 0;
                sn.other = None;
                sp = sn.next.as_deref_mut();
            }
            set = s.next.as_deref_mut();
        }

        // c:142-180 — find matching desc, build "other" chain.
        let mut set1 = (*st_ptr).sets.as_deref_mut();
        while let Some(s1) = set1 {
            let s1_ptr: *mut cdset = s1;
            let mut str1 = (*s1_ptr).strs.as_deref_mut();
            while let Some(t1) = str1 {
                if t1.desc.is_none() || t1.kind != 0 {
                    // c:144
                    str1 = t1.next.as_deref_mut();
                    continue;
                }
                let mut num = 1i32; // c:147
                let mut width = t1.width + (*st_ptr).swidth; // c:148
                if width > (*st_ptr).maxglen {
                    (*st_ptr).maxglen = width;
                }
                // Iterate set2 from set1 onwards; str2 starts at str1.next
                // when same set, else strs head.
                let t1_desc = t1.desc.clone().unwrap_or_default();
                let mut other_tail: *mut Option<Box<cdstr>> = &mut t1.other;
                let mut hit_break = false;
                let mut set2 = Some(&mut *s1_ptr);
                let mut first_iter = true;
                while let Some(s2) = set2 {
                    let s2_ptr: *mut cdset = s2;
                    let mut str2 = if first_iter {
                        t1.next.as_deref_mut()
                    } else {
                        (*s2_ptr).strs.as_deref_mut()
                    };
                    first_iter = false;
                    while let Some(t2) = str2 {
                        if t2.desc.as_deref() == Some(t1_desc.as_str()) {
                            width += CM_SPACE + t2.width; // c:157
                            if width > (*st_ptr).maxmlen || num == maxg {
                                // c:158
                                hit_break = true;
                                break;
                            }
                            if width > (*st_ptr).maxglen {
                                // c:160
                                (*st_ptr).maxglen = width;
                            }
                            t1.kind = 1; // c:162
                            t2.kind = 2;
                            num += 1;
                            // Clone t2 into the other chain (Rust ownership).
                            let clone = Box::new(cdstr {
                                next: None,
                                str: t2.str.clone(),
                                desc: t2.desc.clone(),
                                r#match: t2.r#match.clone(),
                                sortstr: t2.sortstr.clone(),
                                len: t2.len,
                                width: t2.width,
                                other: None,
                                kind: t2.kind,
                                set: t2.set,
                                run: None,
                            });
                            *other_tail = Some(clone);
                            let nxt = &mut (*other_tail).as_mut().unwrap().other;
                            other_tail = nxt as *mut _;
                        }
                        str2 = t2.next.as_deref_mut();
                    }
                    if hit_break {
                        break;
                    }
                    set2 = (*s2_ptr).next.as_deref_mut();
                }
                if num > 1 {
                    // c:173
                    (*st_ptr).groups += 1;
                } else {
                    (*st_ptr).descs += 1; // c:176
                }
                if num > (*st_ptr).maxg {
                    // c:178
                    (*st_ptr).maxg = num;
                }
                str1 = t1.next.as_deref_mut();
            }
            set1 = s1.next.as_deref_mut();
        }
    }
}

/// Direct port of `static void cd_calc(void)` from
/// `Src/Zle/computil.c:188-211`. Walks `cd_state.sets`, computing
/// each set's `count`/`desc` and updating
/// `cd_state.pre`/`premaxw`/`suf` (the global max widths) for the
/// `_describe` column layout.
pub fn cd_calc() {
    // c:188
    let mut st = cd_state.lock().unwrap();
    st.pre = 0; // c:194
    st.suf = 0;
    let mut max_pre = 0i32;
    let mut max_premaxw = st.premaxw;
    let mut max_suf = 0i32;

    let mut set = st.sets.as_deref_mut();
    while let Some(s) = set {
        s.count = 0; // c:197
        s.desc = 0;
        let mut str_iter = s.strs.as_deref();
        while let Some(st_node) = str_iter {
            s.count += 1; // c:199
            let str_s = st_node.str.as_deref().unwrap_or("");
            let l = str_s.len() as i32;
            if l > max_pre {
                max_pre = l;
            } // c:200
              // c:202 — ZMB_nicewidth(str). Rust niceztrlen returns usize.
            let nw = niceztrlen(str_s) as i32;
            if nw > max_premaxw {
                max_premaxw = nw;
            }
            if let Some(d) = st_node.desc.as_deref() {
                // c:204
                s.desc += 1;
                let dl = d.len() as i32;
                if dl > max_suf {
                    max_suf = dl;
                } // c:206
            }
            str_iter = st_node.next.as_deref();
        }
        set = s.next.as_deref_mut();
    }
    st.pre = max_pre;
    st.premaxw = max_premaxw;
    st.suf = max_suf;
}

/// Direct port of `static int cd_groups_want_sorting(void)` from
/// `Src/Zle/computil.c:215-230`. Returns 0 if any set's opts contain
/// `-V` (preserve order), 1 if any contain `-J` (sort), 1 default.
pub fn cd_groups_want_sorting() -> i32 {
    // c:215
    let st = cd_state.lock().unwrap();
    let mut set = st.sets.as_deref();
    while let Some(s) = set {
        if let Some(opts) = s.opts.as_deref() {
            for o in opts {
                if o.starts_with("-V") {
                    return 0;
                } // c:222
                if o.starts_with("-J") {
                    return 1;
                } // c:224
            }
        }
        set = s.next.as_deref();
    }
    1 // c:229
}

/// Direct port of `static int cd_sort(const void *a, const void *b)`
/// from `Src/Zle/computil.c:233-236`. qsort comparator over Cdstr
/// pointers — compares the `sortstr` fields via `zstrcmp` (case-
/// sensitive by default).
pub fn cd_sort(a: &cdstr, b: &cdstr) -> std::cmp::Ordering {
    // c:233
    crate::ported::sort::zstrcmp(
        a.sortstr.as_deref().unwrap_or(""),
        b.sortstr.as_deref().unwrap_or(""),
        0,
    ) // c:235
}

/// Direct port of `static int cd_prep(void)` from
/// `Src/Zle/computil.c:239-439`. Builds the `cd_state.runs` chain
/// from the parsed `cd_state.sets`.
///
/// Three branches:
///   - groups (cd_state.groups > 0): build CRT_EXPL + CRT_SPEC/
///     CRT_DUMMY interleaved + CRT_SIMPLE per set. The most complex
///     path; depends on width tracking via cd_state.gprew. **This
///     branch returns 1 when the laid-out group width exceeds the
///     terminal — the caller (cd_init at c:582-586) loops with a
///     shrunken maxg until prep succeeds.**
///   - showd (cd_state.showd != 0): emit CRT_DESC for entries with
///     descriptions and CRT_SIMPLE for plain matches per set.
///   - default: one CRT_SIMPLE run per set.
pub fn cd_prep() -> i32 {
    // c:239
    // CRT_SIMPLE/DESC/SPEC/DUMMY/EXPL declared at the top of this file

    // Build the new runs list as a Vec; link into cd_state.runs at the end.
    let mut new_runs: Vec<Box<cdrun>> = Vec::new();
    let mut st = cd_state.lock().unwrap();
    st.runs = None;

    if st.groups != 0 {
        // c:247-394 — groups path. Full algorithm: collect leaders
        // (kind==1 from cd_group OR kind==0+desc standalone) into a
        // `prep_lines` Vec; sort by width inside each leader's .other
        // chain; track per-column widths; bail-with-1 on overflow;
        // sort by sortstr; dedup-adjacent so same-desc entries cluster;
        // emit CRT_EXPL header + CRT_SPEC per leader column 0; for each
        // additional column emit CRT_DUMMY/CRT_SPEC interleave; finally
        // emit CRT_SIMPLE per set for un-described entries.

        let maxg = st.maxg.max(1) as usize;
        let maxmlen = st.maxmlen;
        let maxglen = st.maxglen;
        let swidth = st.swidth;

        // c:256 — wids[0..maxg] tracks max width per column.
        let mut wids: Vec<i32> = vec![0; maxg];

        // c:257-287 — collect leaders into prep_lines (Vec of owned
        // cdstr clones with their .other chains).
        let mut prep_lines: Vec<Box<cdstr>> = Vec::new();
        let mut set = st.sets.as_deref();
        while let Some(s) = set {
            let mut str_iter = s.strs.as_deref();
            while let Some(node) = str_iter {
                if node.kind != 1 {
                    if node.kind == 0 && node.desc.is_some() {
                        // c:262
                        if node.width > wids[0] {
                            // c:263
                            wids[0] = node.width;
                        }
                        let mut clone = Box::new({
                            let n = node;
                            cdstr {
                                next: None,
                                str: n.str.clone(),
                                desc: n.desc.clone(),
                                r#match: n.r#match.clone(),
                                sortstr: n.sortstr.clone(),
                                len: n.len,
                                width: n.width,
                                other: None,
                                kind: n.kind,
                                set: n.set,
                                run: None,
                            }
                        });
                        clone.other = None; // c:265
                        prep_lines.push(clone);
                    }
                    str_iter = node.next.as_deref();
                    continue;
                }
                // c:270 — kind==1 leader: collect, sort its .other by
                // width descending, update wids[i] per column.
                let mut gs = Box::new({
                    let n = node;
                    cdstr {
                        next: None,
                        str: n.str.clone(),
                        desc: n.desc.clone(),
                        r#match: n.r#match.clone(),
                        sortstr: n.sortstr.clone(),
                        len: n.len,
                        width: n.width,
                        other: None,
                        kind: n.kind,
                        set: n.set,
                        run: None,
                    }
                });
                gs.kind = 2; // c:271
                gs.other = None;

                // Walk node.other; build a sorted insert into gs.other
                // by descending width (matches c:274-281).
                let mut gp = node.other.as_deref();
                while let Some(g_node) = gp {
                    let new_clone = Box::new({
                        let n = g_node;
                        cdstr {
                            next: None,
                            str: n.str.clone(),
                            desc: n.desc.clone(),
                            r#match: n.r#match.clone(),
                            sortstr: n.sortstr.clone(),
                            len: n.len,
                            width: n.width,
                            other: None,
                            kind: n.kind,
                            set: n.set,
                            run: None,
                        }
                    });
                    // Sorted-insert by width descending.
                    // Drain gs's .other chain into a flat Vec, sort-insert
                    // new_clone, then rebuild the chain.
                    let mut chain: Vec<Box<cdstr>> = Vec::new();
                    // First entry: a clone of gs itself (without other).
                    chain.push(Box::new(cdstr {
                        next: None,
                        str: gs.str.clone(),
                        desc: gs.desc.clone(),
                        r#match: gs.r#match.clone(),
                        sortstr: gs.sortstr.clone(),
                        len: gs.len,
                        width: gs.width,
                        other: None,
                        kind: gs.kind,
                        set: gs.set,
                        run: None,
                    }));
                    let mut rest = gs.other.take();
                    while let Some(mut n) = rest {
                        rest = n.other.take();
                        chain.push(n);
                    }
                    // Find insert index where existing.width <= new_clone.width.
                    let mut ins = chain.len();
                    for (i, c) in chain.iter().enumerate() {
                        if c.width <= new_clone.width {
                            ins = i;
                            break;
                        }
                    }
                    chain.insert(ins, new_clone);
                    // Rebuild gs from chain[0]; link tail via .other.
                    let mut new_head = chain.remove(0);
                    let mut tail_ptr: *mut Option<Box<cdstr>> = &mut new_head.other;
                    for entry in chain {
                        unsafe {
                            *tail_ptr = Some(entry);
                            let nxt = &mut (*tail_ptr).as_mut().unwrap().other;
                            tail_ptr = nxt as *mut _;
                        }
                    }
                    gs = new_head;
                    gp = g_node.other.as_deref();
                }

                // c:282-284 — update wids per column.
                let mut col = 0usize;
                let mut walker = Some(gs.as_ref());
                while let Some(g) = walker {
                    if col < wids.len() && g.width > wids[col] {
                        wids[col] = g.width;
                    }
                    col += 1;
                    walker = g.other.as_deref();
                }

                prep_lines.push(gs);
                str_iter = node.next.as_deref();
            }
            set = s.next.as_deref();
        }

        // c:289-292 — gprew = sum(wids[i] + CM_SPACE).
        let mut gprew = 0i32;
        for w in &wids {
            gprew += w + CM_SPACE;
        }
        st.gprew = gprew;

        // c:294 — bail with retry if too wide.
        if gprew > maxmlen && maxglen > 1 {
            let _ = swidth;
            return 1;
        }

        // c:297-303 — set sortstr from unmetafy(str) for each line.
        for line in prep_lines.iter_mut() {
            let s = line.str.clone().unwrap_or_default();
            line.sortstr = Some(crate::ported::utils::unmeta(&s));
        }

        // c:305 — sort if requested.
        // We have to drop the lock briefly because cd_groups_want_sorting
        // re-acquires it.
        let want_sort = {
            drop(st);
            let r = cd_groups_want_sorting();
            st = cd_state.lock().unwrap();
            r
        };
        if want_sort != 0 {
            // c:305
            // c:306 — qsort cd_sort; tolerant sort: cd_sort→strcmp/numeric may
            // not be a strict weak order, which makes Rust's sort_by PANIC.
            crate::tolerant_sort::qsort_tolerant(&mut prep_lines, |a, b| cd_sort(a, b));
            // c:306
        }

        // c:308-322 — dedup-adjacent: shuffle same-desc entries together.
        let mut i = 0usize;
        while i + 1 < prep_lines.len() {
            let strp_desc = prep_lines[i].desc.clone().unwrap_or_default();
            let next_desc = prep_lines[i + 1].desc.clone().unwrap_or_default();
            if strp_desc == next_desc {
                i += 1;
                continue;
            }
            // Find a later entry with matching desc; bubble it to i+1.
            let mut found: Option<usize> = None;
            for j in i + 2..prep_lines.len() {
                if prep_lines[j].desc.clone().unwrap_or_default() == strp_desc {
                    found = Some(j);
                    break;
                }
            }
            if let Some(j) = found {
                let entry = prep_lines.remove(j);
                prep_lines.insert(i + 1, entry);
            }
            i += 1;
        }

        let preplines = prep_lines.len();

        // c:350/369 — CRT_DUMMY runs carry `expl->strs` so cd_get can reach
        // `run->strs->set->opts` (C:764). We only need a cdstr whose `.set`
        // indexes the right set; mirror C's use of grps[0] (the first prep
        // line = the CRT_EXPL head). Passing None dropped the set's matcher
        // opts from every dummy column, breaking the grouped-description
        // alignment.
        let dummy_set_idx = prep_lines.first().map(|l| l.set).unwrap_or(0);
        let make_dummy_strs = || -> Option<Box<cdstr>> {
            Some(Box::new(cdstr {
                next: None,
                str: None,
                desc: None,
                r#match: None,
                sortstr: None,
                len: 0,
                width: 0,
                other: None,
                kind: 0,
                set: dummy_set_idx,
                run: None,
            }))
        };

        // c:323-326 — CRT_EXPL header: link all preplines via .run.
        // Build a chain of header cdstrs (desc + str only).
        if preplines > 0 {
            let mut expl_head: Option<Box<cdstr>> = None;
            let mut tail_ptr: *mut Option<Box<cdstr>> = &mut expl_head;
            for line in &prep_lines {
                let header = Box::new(cdstr {
                    next: None,
                    str: line.str.clone(),
                    desc: line.desc.clone(),
                    r#match: line.r#match.clone(),
                    sortstr: line.sortstr.clone(),
                    len: line.len,
                    width: line.width,
                    other: None,
                    kind: line.kind,
                    set: line.set,
                    run: None,
                });
                unsafe {
                    *tail_ptr = Some(header);
                    let nxt = &mut (*tail_ptr).as_mut().unwrap().run;
                    tail_ptr = nxt as *mut _;
                }
            }
            // c:323-326 — emit CRT_EXPL run with the header chain.
            let expl_run = Box::new(cdrun {
                next: None,
                r#type: CRT_EXPL,
                strs: expl_head,
                count: preplines as i32,
            });
            // Store at the END (matches c:373 `*runp = expl; runp = &(expl->next)`).
            // We'll insert after column-emit runs below.

            // c:328-340 — emit CRT_SPEC for each column-0 leader.
            // Each line has a .other chain; we consume it column-by-column.
            let mut grps: Vec<Option<Box<cdstr>>> = prep_lines.into_iter().map(Some).collect();

            for line_opt in grps.iter_mut() {
                if let Some(line) = line_opt.take() {
                    let mut owned = *line;
                    let next_col = owned.other.take();
                    owned.run = None;
                    let spec_run = Box::new(cdrun {
                        next: None,
                        r#type: CRT_SPEC,
                        strs: Some(Box::new(owned)),
                        count: 1,
                    });
                    new_runs.push(spec_run);
                    *line_opt = next_col;
                }
            }

            // c:343-372 — for columns 1..maxg, emit CRT_DUMMY/CRT_SPEC.
            for _col in 1..maxg {
                let mut dummy_count = 0i32;
                for line_opt in grps.iter_mut() {
                    if let Some(line) = line_opt.take() {
                        // Flush pending dummies first.
                        if dummy_count > 0 {
                            new_runs.push(Box::new(cdrun {
                                next: None,
                                r#type: CRT_DUMMY,
                                strs: make_dummy_strs(),
                                count: dummy_count,
                            }));
                            dummy_count = 0;
                        }
                        let mut owned = *line;
                        let next_col = owned.other.take();
                        owned.run = None;
                        new_runs.push(Box::new(cdrun {
                            next: None,
                            r#type: CRT_SPEC,
                            strs: Some(Box::new(owned)),
                            count: 1,
                        }));
                        *line_opt = next_col;
                    } else {
                        dummy_count += 1;
                    }
                }
                if dummy_count > 0 {
                    // c:365
                    new_runs.push(Box::new(cdrun {
                        next: None,
                        r#type: CRT_DUMMY,
                        strs: make_dummy_strs(),
                        count: dummy_count,
                    }));
                }
            }

            // c:373 — append the expl run at the end of the column emits.
            new_runs.push(expl_run);
        }

        // c:376-394 — emit CRT_SIMPLE per set for entries without
        // kind and without desc (the un-described ones).
        let mut set = st.sets.as_deref();
        while let Some(s) = set {
            let mut head: Option<Box<cdstr>> = None;
            let mut tail_ptr: *mut Option<Box<cdstr>> = &mut head;
            let mut count = 0i32;
            let mut str_iter = s.strs.as_deref();
            while let Some(node) = str_iter {
                if node.kind == 0 && node.desc.is_none() {
                    let clone = Box::new(cdstr {
                        next: None,
                        str: node.str.clone(),
                        desc: None,
                        r#match: node.r#match.clone(),
                        sortstr: node.sortstr.clone(),
                        len: node.len,
                        width: node.width,
                        other: None,
                        kind: 0,
                        set: node.set,
                        run: None,
                    });
                    unsafe {
                        *tail_ptr = Some(clone);
                        let nxt = &mut (*tail_ptr).as_mut().unwrap().run;
                        tail_ptr = nxt as *mut _;
                    }
                    count += 1;
                }
                str_iter = node.next.as_deref();
            }
            if count > 0 {
                new_runs.push(Box::new(cdrun {
                    next: None,
                    r#type: CRT_SIMPLE,
                    strs: head,
                    count,
                }));
            }
            set = s.next.as_deref();
        }
    } else if st.showd != 0 {
        // c:395-423 — showd: emit CRT_DESC (described entries) then
        // CRT_SIMPLE (undescribed) per set.
        let mut set = st.sets.as_deref();
        while let Some(s) = set {
            if s.desc > 0 {
                // c:397-409 — CRT_DESC for entries with descriptions.
                let mut head: Option<Box<cdstr>> = None;
                let mut tail: *mut Option<Box<cdstr>> = &mut head;
                let mut str_iter = s.strs.as_deref();
                while let Some(st_node) = str_iter {
                    if st_node.desc.is_some() {
                        let clone = Box::new(cdstr {
                            next: None,
                            str: st_node.str.clone(),
                            desc: st_node.desc.clone(),
                            r#match: st_node.r#match.clone(),
                            sortstr: st_node.sortstr.clone(),
                            len: st_node.len,
                            width: st_node.width,
                            other: None,
                            kind: st_node.kind,
                            set: st_node.set,
                            run: None,
                        });
                        unsafe {
                            *tail = Some(clone);
                            let nxt = &mut (*tail).as_mut().unwrap().run;
                            tail = nxt as *mut _;
                        }
                    }
                    str_iter = st_node.next.as_deref();
                }
                new_runs.push(Box::new(cdrun {
                    next: None,
                    r#type: CRT_DESC,
                    strs: head,
                    count: s.desc,
                }));
            }
            if s.desc != s.count {
                // c:410-422 — CRT_SIMPLE for undescribed entries.
                let mut head: Option<Box<cdstr>> = None;
                let mut tail: *mut Option<Box<cdstr>> = &mut head;
                let mut str_iter = s.strs.as_deref();
                while let Some(st_node) = str_iter {
                    if st_node.desc.is_none() {
                        let clone = Box::new(cdstr {
                            next: None,
                            str: st_node.str.clone(),
                            desc: st_node.desc.clone(),
                            r#match: st_node.r#match.clone(),
                            sortstr: st_node.sortstr.clone(),
                            len: st_node.len,
                            width: st_node.width,
                            other: None,
                            kind: st_node.kind,
                            set: st_node.set,
                            run: None,
                        });
                        unsafe {
                            *tail = Some(clone);
                            let nxt = &mut (*tail).as_mut().unwrap().run;
                            tail = nxt as *mut _;
                        }
                    }
                    str_iter = st_node.next.as_deref();
                }
                new_runs.push(Box::new(cdrun {
                    next: None,
                    r#type: CRT_SIMPLE,
                    strs: head,
                    count: s.count - s.desc,
                }));
            }
            set = s.next.as_deref();
        }
    } else {
        // c:424-435 — default: one CRT_SIMPLE per non-empty set.
        let mut set = st.sets.as_deref();
        while let Some(s) = set {
            if s.count != 0 {
                // c:431 — link str.run = str.next for each entry.
                let mut head: Option<Box<cdstr>> = None;
                let mut tail: *mut Option<Box<cdstr>> = &mut head;
                let mut str_iter = s.strs.as_deref();
                while let Some(st_node) = str_iter {
                    let clone = Box::new(cdstr {
                        next: None,
                        str: st_node.str.clone(),
                        desc: st_node.desc.clone(),
                        r#match: st_node.r#match.clone(),
                        sortstr: st_node.sortstr.clone(),
                        len: st_node.len,
                        width: st_node.width,
                        other: None,
                        kind: st_node.kind,
                        set: st_node.set,
                        run: None,
                    });
                    unsafe {
                        *tail = Some(clone);
                        let nxt = &mut (*tail).as_mut().unwrap().run;
                        tail = nxt as *mut _;
                    }
                    str_iter = st_node.next.as_deref();
                }
                new_runs.push(Box::new(cdrun {
                    next: None,
                    r#type: CRT_SIMPLE,
                    strs: head,
                    count: s.count,
                }));
            }
            set = s.next.as_deref();
        }
    }

    // Link new_runs as a chain into cd_state.runs.
    let mut head: Option<Box<cdrun>> = None;
    for run in new_runs.into_iter().rev() {
        let mut run = run;
        run.next = head;
        head = Some(run);
    }
    st.runs = head;
    0 // c:438
}

/// Port of `static char **cd_arrcat(char **a, char **b)` from
/// `Src/Zle/computil.c:444`. Concatenates string arrays `a` + `b`
/// into a fresh heap-allocated NULL-terminated array.
/// ```c
/// static char **
/// cd_arrcat(char **a, char **b)
/// {
///     if (!b) return zarrdup(a);
///     else {
///         char **r = zalloc((arrlen(a) + arrlen(b) + 1) * sizeof(char *));
///         char **p = r;
///         for (; *a; a++) *p++ = ztrdup(*a);
///         for (; *b; b++) *p++ = ztrdup(*b);
///         *p = NULL;
///         return r;
///     }
/// }
/// ```
pub fn cd_arrcat(a: &[String], b: &[String]) -> Vec<String> {
    // c:444
    // c:446-447 — `if (!b) return zarrdup(a);` collapses to the
    // generic path since `&[String]` is never null in Rust; an
    // empty slice yields the same result as zarrdup(a).
    let mut r: Vec<String> = Vec::with_capacity(a.len() + b.len()); // c:449
    for s in a {
        // c:453 for (; *a; a++)
        r.push(ztrdup(s)); // c:454 *p++ = ztrdup(*a)
    }
    for s in b {
        // c:455 for (; *b; b++)
        r.push(ztrdup(s)); // c:456 *p++ = ztrdup(*b)
    }
    // c:458 — `*p = NULL;` — Rust Vec doesn't need a sentinel
    r // c:460
}

/// Direct port of `static int cd_init(char *nam, char *hide, char *mlen,
///                                       char *sep, char **opts, char **args,
///                                       int disp)`
/// from `Src/Zle/computil.c:477-594`. Parses the `_describe` input
/// (match arrays + optional display arrays) into the `cd_state.sets`
/// chain, then runs `cd_calc` + `cd_prep` to build the run chain.
///
/// `args` is the consolidated arg list — match-array param name,
/// optional disp-array name, optional `--`-separated per-set opts.
/// `-g` prefix on `args` enables group detection (cd_group loop).
pub fn cd_init(
    nam: &str,
    hide: &str,
    mlen: &str,
    sep: &str, // c:477
    opts: &[String],
    args: &[String],
    disp: i32,
) -> i32 {
    // c:485 — discard prior parsed state.
    // DEADLOCK GUARD: freecdsets drains cd_state.runs and takes the
    // cd_state lock itself (freecdsets c:116-119) — the sets must be
    // moved OUT and the guard DROPPED before calling it. Holding the
    // guard across the call self-deadlocked the main thread on the
    // SECOND completion pass of every Tab (pass A leaves parsed sets;
    // pass B's cd_init frees them) — the shell wedged spinning-parked
    // with the tty in cooked mode.
    if cd_parsed.load(Ordering::Relaxed) != 0 {
        let old_sets = {
            let mut st = cd_state.lock().unwrap();
            st.sep = None;
            st.sets.take()
        };
        freecdsets(old_sets);
        cd_parsed.store(0, Ordering::Relaxed);
    }

    // c:491 — seed cd_state.
    {
        let mut st = cd_state.lock().unwrap();
        st.sep = Some(sep.to_string());
        st.slen = sep.len() as i32;
        st.swidth = niceztrlen(sep) as i32;
        st.sets = None;
        st.showd = disp;
        st.maxg = 0;
        st.groups = 0;
        st.descs = 0;
        st.maxmlen = mlen.parse::<i32>().unwrap_or(0);
        st.premaxw = 0;
        let cols = adjustcolumns() as i32;
        let itmp = cols - st.swidth - 4; // c:499
        if st.maxmlen > itmp {
            st.maxmlen = itmp;
        }
        if st.maxmlen < 4 {
            st.maxmlen = 4;
        }
    }

    // c:504 — strip leading `-g` for group detection.
    let mut idx = 0usize;
    let grp = if args.first().map(|s| s.as_str()) == Some("-g") {
        idx = 1;
        true
    } else {
        false
    };

    // c:508 — walk arg pairs (match-array [disp-array] [-- opts]).
    let mut sets_collected: Vec<Box<cdset>> = Vec::new();
    while idx < args.len() {
        let arg = &args[idx];
        let Some(mat_arr) = get_user_var(Some(arg.as_str())) else {
            // c:515
            zwarnnam(nam, &format!("invalid argument: {}", arg));
            // Guard dropped before freecdsets — see the deadlock note at
            // the top of this fn.
            let old_sets = {
                let mut st = cd_state.lock().unwrap();
                st.sep = None;
                st.sets.take()
            };
            freecdsets(old_sets);
            return 1;
        };
        idx += 1;

        // c:521-543 — parse `match:desc` entries into cdstr chain.
        let mut strs_vec: Vec<Box<cdstr>> = Vec::new();
        for entry in &mat_arr {
            let bytes = entry.as_bytes();
            let mut p = 0usize;
            while p < bytes.len() && bytes[p] != b':' {
                // c:530
                if bytes[p] == b'\\' && p + 1 < bytes.len() {
                    p += 1;
                }
                p += 1;
            }
            let (match_part, desc_part) = if p < bytes.len() {
                let m = std::str::from_utf8(&bytes[..p]).unwrap_or("");
                let d = std::str::from_utf8(&bytes[p + 1..]).unwrap_or("");
                (rembslash(m), Some(rembslash(d)))
            } else {
                (rembslash(entry), None)
            };
            let str_s = match_part.clone();
            let mut new_str = Box::new(cdstr::default());
            new_str.str = Some(str_s.clone());
            new_str.r#match = Some(str_s.clone());
            new_str.desc = desc_part;
            new_str.len = str_s.len() as i32;
            new_str.width = niceztrlen(&str_s) as i32;
            new_str.kind = 0;
            strs_vec.push(new_str);
        }

        // c:547-557 — optional separate match array.
        if idx < args.len() && !args[idx].starts_with('-') {
            let Some(match_arr) = get_user_var(Some(args[idx].as_str())) else {
                zwarnnam(nam, &format!("invalid argument: {}", args[idx]));
                // Guard dropped before freecdsets — see the deadlock note
                // at the top of this fn.
                let old_sets = {
                    let mut st = cd_state.lock().unwrap();
                    st.sep = None;
                    st.sets.take()
                };
                freecdsets(old_sets);
                return 1;
            };
            for (i, m) in match_arr.iter().enumerate() {
                if i < strs_vec.len() {
                    strs_vec[i].r#match = Some(m.clone());
                }
            }
            idx += 1;
        }

        // c:559 — apply hide (strip leading `-`/`--` from str).
        if !hide.is_empty() {
            let hb = hide.as_bytes();
            let double = hb.len() > 1;
            for s in strs_vec.iter_mut() {
                if let Some(cur) = s.str.clone() {
                    let mut bytes = cur.into_bytes();
                    if double && bytes.len() >= 2 && bytes[0] == b'-' && bytes[1] == b'-' {
                        bytes.drain(0..2); // c:564
                    } else if !bytes.is_empty() && (bytes[0] == b'-' || bytes[0] == b'+') {
                        bytes.drain(0..1); // c:566
                    }
                    s.str = String::from_utf8(bytes).ok();
                }
            }
        }

        // c:569-577 — gather per-set opts up to `--`.
        let opt_start = idx;
        while idx < args.len()
            && !(args[idx].as_bytes().len() == 2
                && args[idx].as_bytes()[0] == b'-'
                && args[idx].as_bytes()[1] == b'-')
        {
            idx += 1;
        }
        let per_set: &[String] = &args[opt_start..idx];
        let combined = cd_arrcat(per_set, opts);
        if idx < args.len() {
            idx += 1;
        } // c:577 skip `--`

        // Link strs_vec as a chain into a new cdset.
        let mut strs_head: Option<Box<cdstr>> = None;
        for s in strs_vec.into_iter().rev() {
            let mut s = s;
            s.next = strs_head;
            strs_head = Some(s);
        }
        let mut set = Box::new(cdset::default());
        set.opts = Some(combined);
        set.strs = strs_head;
        sets_collected.push(set);
    }

    // Link sets_collected as a chain into cd_state.sets.
    {
        let mut head: Option<Box<cdset>> = None;
        for s in sets_collected.into_iter().rev() {
            let mut s = s;
            s.next = head;
            head = Some(s);
        }
        cd_state.lock().unwrap().sets = head;
    }

    // c:579 — group-aware vs simple prep.
    if disp != 0 && grp {
        let cols = adjustcolumns() as i32;
        let mut mg = cols;
        // c:582-586 — retry cd_prep with shrinking maxg.
        loop {
            cd_group(mg);
            mg = {
                let st = cd_state.lock().unwrap();
                st.maxg - 1
            };
            cd_calc();
            if cd_prep() == 0 || mg <= 0 {
                break;
            }
        }
    } else {
        cd_calc();
        cd_prep();
    }
    cd_parsed.store(1, Ordering::Relaxed); // c:592
    0
}

/// Direct port of `static char **cd_arrdup(char **a)` from
/// `Src/Zle/computil.c:somewhere`. Duplicate a string array.
pub fn cd_arrdup(a: &[String]) -> Vec<String> {
    // c:cd_arrdup
    a.to_vec()
}

/// Direct port of `static int cd_get(char **params)` from
/// `Src/Zle/computil.c:614-841`. Pops the next `cdrun` off
/// `cd_state.runs` and emits its match/display arrays + per-run
/// compadd options into the four named params:
///   `params[0]` = csl ("" or "packed")
///   `params[1]` = opts (compadd flags)
///   `params[2]` = mats (match strings)
///   `params[3]` = dpys (display strings)
/// Returns 1 when no runs remain, 0 otherwise.
pub fn cd_get(params: &[String]) -> i32 {
    // c:614

    // c:618 — pop the head run.
    let run_opt = {
        let mut st = cd_state.lock().unwrap();
        st.runs.take().map(|mut r| {
            let next = r.next.take();
            st.runs = next;
            r
        })
    };
    let Some(run) = run_opt else {
        return 1;
    };

    let mut mats: Vec<String> = Vec::new();
    let mut dpys: Vec<String> = Vec::new();
    let mut opts: Vec<String> = Vec::new();
    let mut csl: String = String::new();

    let rtype = run.r#type;

    // Helper: walk a cdstr chain via .run, applying f.
    let mut walk_run = |head: &Option<Box<cdstr>>, mut f: Box<dyn FnMut(&cdstr)>| {
        let mut cur = head.as_deref();
        while let Some(s) = cur {
            f(s);
            cur = s.run.as_deref();
        }
    };

    if rtype == CRT_SIMPLE {
        // c:625
        let head_opts = run
            .strs
            .as_deref()
            .map(|s| {
                let st = cd_state.lock().unwrap();
                // c:634 — zarrdup(run->strs->set->opts). Set is an index;
                // we walk cd_state.sets to find the matching index.
                let mut set_iter = st.sets.as_deref();
                let mut found: Option<Vec<String>> = None;
                let mut idx_count = 0usize;
                while let Some(set) = set_iter {
                    if idx_count == s.set {
                        found = set.opts.clone();
                        break;
                    }
                    idx_count += 1;
                    set_iter = set.next.as_deref();
                }
                found.unwrap_or_default()
            })
            .unwrap_or_default();
        walk_run(
            &run.strs,
            Box::new(|s| {
                // c:629
                mats.push(s.r#match.clone().unwrap_or_default());
                dpys.push(
                    s.str
                        .clone()
                        .or_else(|| s.r#match.clone())
                        .unwrap_or_default(),
                );
            }),
        );
        let groups_flag = cd_state.lock().unwrap().groups;
        opts = if groups_flag != 0 {
            // c:635
            // c:641 — strip `-X` options.
            let mut filtered: Vec<String> = Vec::new();
            let mut skip_next = false;
            for o in head_opts.iter() {
                if skip_next {
                    skip_next = false;
                    continue;
                }
                if o.starts_with("-X") {
                    // c:642
                    if o.len() == 2 {
                        skip_next = true;
                    } // c:643
                    continue;
                }
                filtered.push(o.clone());
            }
            filtered
        } else {
            head_opts
        };
    } else if rtype == CRT_DESC {
        // c:652
        let st_snapshot = {
            let st = cd_state.lock().unwrap();
            (
                st.pre,
                st.suf,
                st.premaxw,
                st.slen,
                st.swidth,
                st.sep.clone(),
                adjustcolumns() as i32,
            )
        };
        let (cd_pre, _cd_suf, cd_premaxw, _cd_slen, cd_swidth, cd_sep, cols) = st_snapshot;
        let sep_str = cd_sep.unwrap_or_default();
        walk_run(
            &run.strs,
            Box::new(|s| {
                // c:669
                let str_s = s.str.clone().unwrap_or_default();
                let desc_s = s.desc.clone().unwrap_or_default();
                let mut buf =
                    String::with_capacity((cd_pre + cd_premaxw + cd_swidth + 16) as usize);
                // c:674 — write str.
                buf.push_str(&str_s);
                // c:676 — pad to premaxw + CM_SPACE.
                let pad = (cd_premaxw - s.width + CM_SPACE).max(0) as usize;
                for _ in 0..pad {
                    buf.push(' ');
                }

                // c:679-715 — append separator + truncated desc to fit terminal.
                let mut remw = cols - cd_premaxw - cd_swidth - 3;
                while remw < 0 && cols > 0 {
                    remw += cols;
                }
                if (sep_str.len() as i32) < remw {
                    // c:685
                    buf.push_str(&sep_str);
                    remw -= sep_str.len() as i32;
                    let dw = niceztrlen(&desc_s) as i32;
                    if dw <= remw {
                        buf.push_str(&desc_s);
                    } else {
                        // c:701
                        // Truncate desc to fit. Use char boundaries.
                        let mut w_used = 0i32;
                        for ch in desc_s.chars() {
                            let cw = niceztrlen(&ch.to_string()) as i32;
                            if w_used + cw > remw {
                                break;
                            }
                            buf.push(ch);
                            w_used += cw;
                        }
                    }
                }
                mats.push(s.r#match.clone().unwrap_or_default()); // c:673
                dpys.push(buf);
            }),
        );
        // c:721 — opts = cd_arrdup + opts[0] = "-l".
        let head_opts = run
            .strs
            .as_deref()
            .map(|s| {
                let st = cd_state.lock().unwrap();
                let mut set_iter = st.sets.as_deref();
                let mut found: Option<Vec<String>> = None;
                let mut idx_count = 0usize;
                while let Some(set) = set_iter {
                    if idx_count == s.set {
                        found = set.opts.clone();
                        break;
                    }
                    idx_count += 1;
                    set_iter = set.next.as_deref();
                }
                found.unwrap_or_default()
            })
            .unwrap_or_default();
        opts = std::iter::once("-l".to_string()).chain(head_opts).collect();
    } else if rtype == CRT_SPEC {
        // c:726
        let s = run.strs.as_deref();
        if let Some(s) = s {
            mats.push(s.r#match.clone().unwrap_or_default());
            dpys.push(s.str.clone().unwrap_or_default());
        }
        // c:732 — opts = cd_arrdup + flip -J/-V to -2V or insert -2V-default-.
        let head_opts = s
            .map(|s| {
                let st = cd_state.lock().unwrap();
                let mut set_iter = st.sets.as_deref();
                let mut found: Option<Vec<String>> = None;
                let mut idx_count = 0usize;
                while let Some(set) = set_iter {
                    if idx_count == s.set {
                        found = set.opts.clone();
                        break;
                    }
                    idx_count += 1;
                    set_iter = set.next.as_deref();
                }
                found.unwrap_or_default()
            })
            .unwrap_or_default();
        let mut new_opts: Vec<String> = head_opts.clone();
        let mut found_jv = false;
        // c:736 — `for (dp = opts + 1; *dp; dp++)`. Skip slot 0 (the
        // existing first element which we'll overwrite below) and look
        // for the first -J/-V flag.
        for i in 1..new_opts.len() {
            if new_opts[i].starts_with("-J") || new_opts[i].starts_with("-V") {
                let rest = new_opts[i][2..].to_string();
                new_opts[i] = format!("-2V{}", rest);
                found_jv = true;
                break;
            }
        }
        if !found_jv {
            new_opts.insert(0, "-2V-default-".to_string()); // c:750
        }
        opts = new_opts;
        csl = "packed".to_string();
    } else if rtype == CRT_DUMMY {
        // c:754
        // c:758 — opts[0] = "-E<count>".
        let head_opts = run
            .strs
            .as_deref()
            .map(|s| {
                let st = cd_state.lock().unwrap();
                let mut set_iter = st.sets.as_deref();
                let mut found: Option<Vec<String>> = None;
                let mut idx_count = 0usize;
                while let Some(set) = set_iter {
                    if idx_count == s.set {
                        found = set.opts.clone();
                        break;
                    }
                    idx_count += 1;
                    set_iter = set.next.as_deref();
                }
                found.unwrap_or_default()
            })
            .unwrap_or_default();
        opts = std::iter::once(format!("-E{}", run.count))
            .chain(head_opts)
            .collect();
        csl = "packed".to_string();
    } else if rtype == CRT_EXPL {
        // c:772
        let st_snapshot = {
            let st = cd_state.lock().unwrap();
            (
                st.suf,
                st.slen,
                st.swidth,
                st.gprew,
                st.sep.clone(),
                adjustcolumns() as i32,
            )
        };
        let (_cd_suf, _cd_slen, cd_swidth, cd_gprew, cd_sep, cols) = st_snapshot;
        let sep_str = cd_sep.unwrap_or_default();
        let count = run.count;

        walk_run(
            &run.strs,
            Box::new(|s| {
                // c:785
                // c:786 — if run sibling has same desc, emit empty.
                let next_desc = s.run.as_deref().and_then(|n| n.desc.clone());
                if next_desc.is_some() && next_desc == s.desc {
                    dpys.push(String::new());
                    return;
                }
                let mut buf = String::new();
                buf.push_str(&sep_str);
                let mut remw = cols - cd_gprew - cd_swidth - CM_SPACE;
                let desc_s = s.desc.clone().unwrap_or_default();
                let dw = niceztrlen(&desc_s) as i32;
                if dw <= remw {
                    // c:797
                    buf.push_str(&desc_s);
                    remw -= dw;
                } else {
                    for ch in desc_s.chars() {
                        let cw = niceztrlen(&ch.to_string()) as i32;
                        if cw > remw {
                            break;
                        }
                        buf.push(ch);
                        remw -= cw;
                    }
                }
                while remw > 0 {
                    // c:817
                    buf.push(' ');
                    remw -= 1;
                }
                dpys.push(buf);
            }),
        );
        // c:825 — opts[0] = "-E<count>".
        let head_opts = run
            .strs
            .as_deref()
            .map(|s| {
                let st = cd_state.lock().unwrap();
                let mut set_iter = st.sets.as_deref();
                let mut found: Option<Vec<String>> = None;
                let mut idx_count = 0usize;
                while let Some(set) = set_iter {
                    if idx_count == s.set {
                        found = set.opts.clone();
                        break;
                    }
                    idx_count += 1;
                    set_iter = set.next.as_deref();
                }
                found.unwrap_or_default()
            })
            .unwrap_or_default();
        opts = std::iter::once(format!("-E{}", count))
            .chain(head_opts)
            .collect();
        csl = "packed".to_string();
    }

    // c:832 — emit the four params.
    if params.len() >= 4 {
        setsparam(&params[0], &csl);
        setaparam(&params[1], opts);
        setaparam(&params[2], mats);
        setaparam(&params[3], dpys);
    }
    0 // c:839
}

/// Direct port of `static int bin_compdescribe(char *nam, char **args,
///                                                UNUSED(Options ops),
///                                                UNUSED(int func))`
/// from `Src/Zle/computil.c:846-895`. Subcommand dispatch for
/// `compdescribe -i/-I/-g`:
///   - `-i hide mlen ARGS...` → cd_init with empty opts and disp=0
///   - `-I hide mlen sep optsParam ARGS...` → cd_init with disp=1
///   - `-g param csl mats dpys` → cd_get with the 4 output params
pub fn bin_compdescribe(
    nam: &str,
    args: &[String], // c:846
    _ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:850
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {
        return 1;
    }
    let a0 = args[0].as_bytes();
    // c:854 — `args[0]` must be exactly 2 chars starting with `-`.
    if a0.len() != 2 || a0[0] != b'-' {
        zwarnnam(nam, &format!("invalid argument: {}", args[0]));
        return 1;
    }
    let n = args.len() as i32;

    match a0[1] {
        b'i' => {
            // c:859
            if n < 3 {
                zwarnnam(nam, "not enough arguments");
                return 1;
            }
            cd_init(nam, &args[1], &args[2], "", &[], &args[3..], 0) // c:865
        }
        b'I' => {
            // c:866
            if n < 6 {
                zwarnnam(nam, "not enough arguments");
                return 1;
            }
            // c:874 — getaparam(args[4]).
            let opts_arr: Vec<String> = paramtab()
                .read()
                .ok()
                .and_then(|tab| tab.get(&args[4]).and_then(|pm| pm.u_arr.clone()))
                .unwrap_or_default();
            if opts_arr.is_empty()
                && paramtab()
                    .read()
                    .ok()
                    .map_or(true, |tab| tab.get(&args[4]).is_none())
            {
                zwarnnam(nam, &format!("unknown parameter: {}", args[4]));
                return 1;
            }
            cd_init(
                nam,
                &args[1],
                &args[2],
                &args[3],
                &opts_arr, // c:878
                &args[5..],
                1,
            )
        }
        b'g' => {
            // c:880
            if cd_parsed.load(Ordering::Relaxed) == 0 {
                // c:881
                zwarnnam(nam, "no parsed state"); // c:889
                return 1;
            }
            if n != 5 {
                zwarnnam(
                    nam,
                    if n < 5 {
                        "not enough arguments"
                    } else {
                        "too many arguments"
                    },
                );
                return 1;
            }
            cd_get(&args[1..]) // c:887
        }
        _ => {
            zwarnnam(nam, &format!("invalid option: {}", args[0]));
            1
        }
    }
}

// =====================================================================
// `_arguments`-cache types — direct ports of the C structs at
// Src/Zle/computil.c:899-968. CAO_* / CAA_* / CDF_SEP /
// MAX_CACACHE constants already declared above (file scope).
// =====================================================================

/// Port of `typedef struct cadef *Cadef` from `Src/Zle/computil.c:899`.
pub type Cadef = Box<cadef>; // c:899

/// Direct port of `struct cadef` from `Src/Zle/computil.c:905-922`.
/// Cache entry for a set of `_arguments` definitions.
#[derive(Debug, Default, Clone)]
#[allow(non_camel_case_types)]
pub struct cadef {
    // c:905
    pub next: Option<Box<cadef>>,                // c:906 Cadef next
    pub snext: Option<Box<cadef>>,               // c:907 Cadef snext
    pub opts: Option<Box<caopt>>,                // c:908 Caopt opts
    pub nopts: i32,                              // c:909
    pub ndopts: i32,                             // c:909
    pub nodopts: i32,                            // c:909
    pub args: Option<Box<caarg>>,                // c:910 Caarg args
    pub rest: Option<Box<caarg>>,                // c:911 Caarg rest
    pub defs: Option<Vec<String>>,               // c:912 char **defs
    pub ndefs: i32,                              // c:913
    pub lastt: i64,                              // c:914 time_t lastt
    pub single: Option<Vec<Option<Box<caopt>>>>, // c:915 Caopt *single (188-slot)
    pub r#match: Option<String>,                 // c:916 char *match
    pub argsactive: i32,                         // c:917
    pub set: Option<String>,                     // c:919 char *set
    pub flags: i32,                              // c:920 int flags (CDF_*)
    pub nonarg: Option<String>,                  // c:921 char *nonarg
}
/// Port of `typedef struct caopt *Caopt` from `Src/Zle/computil.c:900`.
pub type Caopt = Box<caopt>; // c:900

/// Direct port of `struct caopt` from `Src/Zle/computil.c:928-939`.
/// Description for one `_arguments` option spec.
#[derive(Debug, Default, Clone)]
#[allow(non_camel_case_types)]
pub struct caopt {
    // c:928
    pub next: Option<Box<caopt>>, // c:929 Caopt next
    pub name: Option<String>,     // c:930 char *name
    pub descr: Option<String>,    // c:931 char *descr
    pub xor: Option<Vec<String>>, // c:932 char **xor
    pub r#type: i32,              // c:933 int type (CAO_*)
    pub args: Option<Box<caarg>>, // c:934 Caarg args
    pub active: i32,              // c:935 int active
    pub num: i32,                 // c:936 int num
    pub gsname: Option<String>,   // c:937 char *gsname
    pub not: i32,                 // c:938 int not
}
/// Port of `typedef struct caarg *Caarg` from `Src/Zle/computil.c:901`.
pub type Caarg = Box<caarg>; // c:901

/// Direct port of `struct caarg` from `Src/Zle/computil.c:949-962`.
/// Description for one `_arguments` argument spec.
#[derive(Debug, Default, Clone)]
#[allow(non_camel_case_types)]
pub struct caarg {
    // c:949
    pub next: Option<Box<caarg>>, // c:950 Caarg next
    pub descr: Option<String>,    // c:951 char *descr
    pub xor: Option<Vec<String>>, // c:952 char **xor
    pub action: Option<String>,   // c:953 char *action
    pub r#type: i32,              // c:954 int type (CAA_*)
    pub end: Option<String>,      // c:955 char *end
    pub opt: Option<String>,      // c:956 char *opt
    pub num: i32,                 // c:957 int num
    pub min: i32,                 // c:958 int min
    pub direct: i32,              // c:959 int direct
    pub active: i32,              // c:960 int active
    pub gsname: Option<String>,   // c:961 char *gsname
}

/// Port of `CDF_SEP` from `Src/Zle/computil.c:924`. `-S` flag — `--`
/// terminates options.
pub const CDF_SEP: i32 = 1; // c:924

// =====================================================================
// CAO_* — Cadef option-argument attachment style — `computil.c:941-945`.
// =====================================================================

/// Port of `CAO_NEXT` from `computil.c:941`. Argument in next argv slot.
pub const CAO_NEXT: i32 = 1; // c:941
/// Port of `CAO_DIRECT` from `computil.c:942`. Argument directly attached
/// to option (`-opt:value`).
pub const CAO_DIRECT: i32 = 2; // c:942
/// Port of `CAO_ODIRECT` from `computil.c:943`. Optional direct attach.
pub const CAO_ODIRECT: i32 = 3; // c:943
/// Port of `CAO_EQUAL` from `computil.c:944`. Argument after `=`.
pub const CAO_EQUAL: i32 = 4; // c:944
/// Port of `CAO_OEQUAL` from `computil.c:945`. Optional `=` argument.
pub const CAO_OEQUAL: i32 = 5; // c:945

// =====================================================================
// CAA_* — Cadef positional-argument kinds — `computil.c:964-968`.
// =====================================================================

/// Port of `CAA_NORMAL` from `computil.c:964`. Plain positional arg.
pub const CAA_NORMAL: i32 = 1; // c:964
/// Port of `CAA_OPT` from `computil.c:965`. Optional positional arg.
pub const CAA_OPT: i32 = 2; // c:965
/// Port of `CAA_REST` from `computil.c:966`. Mandatory rest of args.
pub const CAA_REST: i32 = 3; // c:966
/// Port of `CAA_RARGS` from `computil.c:967`. Repeated args sequence.
pub const CAA_RARGS: i32 = 4; // c:967
/// Port of `CAA_RREST` from `computil.c:968`. Repeated rest of args.
pub const CAA_RREST: i32 = 5; // c:968

/// Port of `MAX_CACACHE` from `computil.c:972`. Cadef LRU cache size.
pub const MAX_CACACHE: usize = 8; // c:972

/// Port of `static Cadef cadef_cache[MAX_CACACHE]` from
/// `Src/Zle/computil.c:973`. The LRU cache holds parsed
/// `_arguments` defs keyed by the raw arg vector — `get_cadef`
/// scans linearly, returns on first match (arr-compare on `defs`),
/// and on miss evicts the entry with the oldest `lastt` slot before
/// inserting the freshly parsed result.
pub static cadef_cache: std::sync::Mutex<[Option<Box<cadef>>; MAX_CACACHE]> = // c:973
    std::sync::Mutex::new([const { None }; MAX_CACACHE]);

/// Direct port of `static int arrcmp(char **a, char **b)` from
/// `Src/Zle/computil.c:978`. Element-wise string-equality test on
/// two `char**` arrays — returns 1 if both are null or both contain
/// the same sequence of strings, 0 otherwise.
pub fn arrcmp(a: Option<&[String]>, b: Option<&[String]>) -> i32 {
    // c:978
    match (a, b) {
        (None, None) => 1,          // c:980
        (None, _) | (_, None) => 0, // c:982
        (Some(a), Some(b)) => {
            // c:984
            // c:985-988 — walk in lockstep, bail on inequality.
            let len = a.len().min(b.len());
            for i in 0..len {
                if a[i] != b[i] {
                    return 0;
                } // c:986
            }
            // c:989 — equal iff both reached end together.
            if a.len() == b.len() {
                1
            } else {
                0
            }
        }
    }
}

/// Direct port of `static void freecaargs(Caarg a)` from
/// `Src/Zle/computil.c:996`. Walks the `next` chain and frees
/// each entry. In Rust this is `Box` ownership — dropping the head
/// recursively drops the chain, but we mirror the C body for ABI
/// parity with callers that want explicit teardown.
pub fn freecaargs(mut a: Option<Box<caarg>>) {
    // c:996
    while let Some(mut node) = a {
        // c:996 for (; a; ...)
        a = node.next.take(); // c:1001 n = a->next
                              // c:1002-1007 — zsfree on descr/xor/action/end/opt is implicit
                              //               via Drop on the String / Vec<String> fields.
        node.descr = None; // c:1013
        node.xor = None; // c:1013-1004
        node.action = None; // c:1013
        node.end = None; // c:1013
        node.opt = None; // c:1013
        drop(node); // c:1013 zfree(a, sizeof(*a))
    }
}

/// Direct port of `static void freecadef(Cadef d)` from
/// `Src/Zle/computil.c:1013`. Walks the `snext` chain freeing
/// each cadef plus its opts/args/rest sub-chains.
pub fn freecadef(mut d: Option<Box<cadef>>) {
    // c:1013
    while let Some(mut node) = d {
        // c:1013 while (d)
        d = node.snext.take(); // c:1019 s = d->snext
                               // c:1020-1023 — zsfree match/set, freearray(defs).
        node.r#match = None;
        node.set = None;
        node.defs = None;

        // c:1025-1033 — for each opt: zsfree name/descr, freearray xor,
        // freecaargs(opt->args), zfree opt.
        let mut p = node.opts.take();
        while let Some(mut popt) = p {
            p = popt.next.take();
            popt.name = None;
            popt.descr = None;
            popt.xor = None;
            freecaargs(popt.args.take()); // c:1031
            drop(popt); // c:1032
        }
        freecaargs(node.args.take()); // c:1034
        freecaargs(node.rest.take()); // c:1035
        node.nonarg = None; // c:1036
        node.single = None; // c:1037-1038
        drop(node); // c:1039 zfree(d, sizeof(*d))
    }
}

/// Port of `rembslashcolon(char *s)` from `Src/Zle/computil.c:1046`.
/// ```c
/// static char *
/// rembslashcolon(char *s)
/// {
///     char *p, *r;
///     r = p = s = dupstring(s);
///     while (*s) {
///         if (s[0] != '\\' || s[1] != ':')
///             *p++ = *s;
///         s++;
///     }
///     *p = '\0';
///     return r;
/// }
/// ```
/// Strip every `\:` two-byte sequence to nothing (the `\` is dropped,
/// the `:` follows on the next iteration). Used to unescape colon-
/// bearing description strings produced by `_arguments`.
pub fn rembslashcolon(s: &str) -> String {
    // c:1047
    let bytes = s.as_bytes(); // c:1047 dupstring(s)
    let mut out = Vec::<u8>::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // c:1053 while (*s)
        // c:1054 — `if (s[0] != '\\' || s[1] != ':') *p++ = *s`.
        let drop = bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b':';
        if !drop {
            out.push(bytes[i]); // c:1055 *p++ = *s
        }
        i += 1; // c:1056 s++
    }
    // c:1058 — `*p = '\0'`. Rust strings are length-tracked.
    String::from_utf8(out).unwrap_or_default() // c:1060 return r
}

/// Port of `bslashcolon(char *s)` from `Src/Zle/computil.c:1065`.
/// ```c
/// static char *
/// bslashcolon(char *s)
/// {
///     char *p, *r;
///     r = p = zhalloc((2 * strlen(s)) + 1);
///     while (*s) {
///         if (*s == ':')
///             *p++ = '\\';
///         *p++ = *s++;
///     }
///     *p = '\0';
///     return r;
/// }
/// ```
/// Insert a backslash before every `:`, doubling the worst-case
/// length. Inverse of `rembslashcolon` for description-string
/// emission.
pub fn bslashcolon(s: &str) -> String {
    // c:1066
    let bytes = s.as_bytes(); // c:1066 zhalloc(2*strlen(s)+1)
    let mut out = Vec::<u8>::with_capacity(2 * bytes.len() + 1);
    for &b in bytes {
        // c:1072 while (*s)
        if b == b':' {
            // c:1073
            out.push(b'\\'); // c:1074 *p++ = '\\'
        }
        out.push(b); // c:1075 *p++ = *s++
    }
    // c:1077 — `*p = '\0'`.
    String::from_utf8(out).unwrap_or_default() // c:1079 return r
}

/// Port of `single_index(char pre, char opt)` from `Src/Zle/computil.c:1088`.
/// ```c
/// static int
/// single_index(char pre, char opt)
/// {
///     if (opt <= 0x20 || opt > 0x7e)
///         return -1;
///     return opt + (pre == '-' ? -0x21 : 94 - 0x21);
/// }
/// ```
/// Map a `(prefix, option-letter)` pair into the flat 188-slot array
/// that `cadef` keeps for single-letter option lookup. Returns -1
/// when `opt` is outside the printable-ASCII range.
///
/// `pre` is `-` for the negative-prefix slot and anything else
/// (typically `+`) for the positive-prefix slot.
pub fn single_index(pre: u8, opt: u8) -> i32 {
    // c:1089
    if opt <= 0x20 || opt > 0x7e {
        // c:1089
        return -1; // c:1092
    }
    // c:1094 — `return opt + (pre == '-' ? -0x21 : 94 - 0x21)`.
    let off: i32 = if pre == b'-' { -0x21 } else { 94 - 0x21 };
    (opt as i32) + off
}

/// Direct port of `static Caarg parse_caarg(int mult, int type, int num,
///                                          int opt, char *oname, char **def,
///                                          char *set)` from
/// `Src/Zle/computil.c:1099-1144`. Parses one `:descr[:action]`
/// fragment of an `_arguments` spec into a freshly-allocated caarg.
/// On return, `*idx` points at the first byte of `bytes` not consumed
/// (either the separator `:` for `mult=1` or `bytes.len()` for
/// `mult=0` rest specs).
pub fn parse_caarg(
    mult: i32,
    atype: i32,
    num: i32,
    opt: i32, // c:1099
    oname: Option<&str>,
    bytes: &[u8],
    idx: &mut usize,
    set: Option<&str>,
) -> Box<caarg> {
    let mut ret = Box::new(caarg::default());
    ret.num = num; // c:1109
    ret.min = num - opt; // c:1110
    ret.r#type = atype; // c:1111
    ret.opt = oname.map(|s| s.to_string()); // c:1112
    ret.direct = 0; // c:1113
    ret.gsname = set.map(|s| s.to_string()); // c:1114

    let n = bytes.len();

    // c:1118-1120 — scan description up to the next `:` (escaped `\:` skipped).
    let d_start = *idx;
    while *idx < n && bytes[*idx] != b':' {
        if bytes[*idx] == b'\\' && *idx + 1 < n {
            *idx += 1;
        }
        *idx += 1;
    }
    let has_sav = *idx < n;
    let descr_slice = &bytes[d_start..*idx];
    let descr_str = std::str::from_utf8(descr_slice).unwrap_or("");
    ret.descr = Some(rembslashcolon(descr_str)); // c:1123

    if has_sav {
        // c:1127
        if mult != 0 {
            // c:1128
            // c:1129-1136 — `*p == ':'` start, scan to next `:` or NUL.
            *idx += 1;
            let a_start = *idx;
            while *idx < n && bytes[*idx] != b':' {
                if bytes[*idx] == b'\\' && *idx + 1 < n {
                    *idx += 1;
                }
                *idx += 1;
            }
            let action_slice = &bytes[a_start..*idx];
            let action_str = std::str::from_utf8(action_slice).unwrap_or("");
            ret.action = Some(rembslashcolon(action_str)); // c:1134
        } else {
            // c:1137
            // c:1138 — `ret->action = ztrdup(rembslashcolon(p + 1))`.
            let action_slice = &bytes[*idx + 1..];
            let action_str = std::str::from_utf8(action_slice).unwrap_or("");
            ret.action = Some(rembslashcolon(action_str));
            *idx = n;
        }
    } else {
        // c:1139
        ret.action = Some(String::new()); // c:1140
    }
    // c:1141 — `*def = p`. Caller reads `bytes[*idx]` to decide whether to
    // continue scanning more `:` fragments.

    ret
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in vm_helper are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ─── moved from src/ported/vm_helper (drift extraction) ───

// CompSpec / CompMatch / CompGroup / CompState moved out of this
// port file to `src/extensions/bash_complete.rs` — they are
// Rust-original types backing the bash-style `complete` builtin
// extension, not zsh C ports. The ported zle/ tree should stay a
// faithful C-source mirror; Rust-only types live in extensions/.
//
// Callers that used `crate::ported::zle::computil::Comp*` should
// switch to `crate::bash_complete::Comp*` (the path lib.rs
// exports). vm_helper's re-export updated to point to the new home.

/// Direct port of `static Cadef alloc_cadef(char **args, int single,
/// char *match, char *nonarg, int flags)` from `Src/Zle/computil.c:1147-1177`.
///
/// Builds a fresh `cadef` with the option/single-letter/match/nonarg
/// fields initialized. `args` (if present) is captured into `defs`
/// for later cache-key compare in `get_cadef` (c:1681). `single` set
/// allocates the 188-slot single-letter index array. `match` is the
/// match-spec carried through to the option/arg matchers.
pub fn alloc_cadef(
    args: Option<&[String]>,
    single: i32,
    matchstr: &str, // c:1147
    nonarg: Option<&str>,
    flags: i32,
) -> Box<cadef> {
    Box::new(cadef {
        next: None,                                // c:1152
        snext: None,                               // c:1152
        opts: None,                                // c:1153
        args: None,                                // c:1154
        rest: None,                                // c:1154
        nonarg: nonarg.map(|s| s.to_string()),     // c:1155 ztrdup(nonarg)
        defs: args.map(|a| a.to_vec()),            // c:1157 zarrdup(args)
        ndefs: args.map_or(0, |a| a.len() as i32), // c:1158 arrlen(args)
        nopts: 0,                                  // c:1163
        ndopts: 0,                                 // c:1164
        nodopts: 0,                                // c:1165
        lastt: {
            // c:1166 time(0)
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        },
        set: None, // c:1167
        // c:1168-1172 — 188-slot single-letter Caopt index. Capacity
        // 188 matches C exactly (range of single-letter option names).
        single: if single != 0 {
            Some((0..188).map(|_| None).collect())
        } else {
            None
        },
        r#match: Some(matchstr.to_string()), // c:1173 ztrdup(match)
        argsactive: 0,
        flags, // c:1174
    })
}

/// Direct port of `static void set_cadef_opts(Cadef def)` from
/// `Src/Zle/computil.c:1180-1191`. After a set-of-arg-definitions has
/// been parsed into the cadef, walk the args linked list and update
/// each non-direct argp's `min` field to the cumulative number of
/// CAA_OPT entries that precede it. The optionality count compounds
/// down the chain, which determines minimum-argument-count semantics
/// during completion.
pub fn set_cadef_opts(def: &mut cadef) {
    // c:1180
    let mut xnum: i32 = 0;
    let mut argp = def.args.as_deref_mut(); // c:1185 argp = def->args
    while let Some(node) = argp {
        // c:1185
        if node.direct == 0 {
            // c:1186 !argp->direct
            node.min = node.num - xnum; // c:1187
        }
        if node.r#type == CAA_OPT {
            // c:1188
            xnum += 1; // c:1189
        }
        argp = node.next.as_deref_mut(); // c:1185 argp = argp->next
    }
}

/// Direct port of `static Cadef parse_cadef(char *nam, char **args)` from
/// `Src/Zle/computil.c:1196-1666`. Parses the leading auto-description
/// (first arg up to `%d`), the `-s/-A/-S/-M` flag block, then the
/// main spec-list loop that fills opts/args/rest from each remaining
/// `_arguments` spec entry.
pub fn parse_cadef(nam: &str, args: &[String]) -> Option<Box<cadef>> {
    // c:1196

    if args.is_empty() {
        return None; // c:1262 `!*args`
    }

    let orig_args = args;
    let mut idx = 0usize;
    let mut single: i32 = 0;
    let mut flags: i32 = 0;
    let mut match_spec: String = "r:|[_-]=* r:|=*".to_string(); // c:1200
    let mut nonarg: Option<String> = None;

    // c:1208-1216 — split args[0] on `%d` into (adpre, adsuf). Used at
    // c:1543-1554 to auto-derive option descriptions.
    let (adpre, adsuf): (Option<String>, Option<String>) = {
        let first = args[0].as_bytes();
        let mut split_at: Option<usize> = None;
        let mut i = 0usize;
        while i + 1 < first.len() {
            if first[i] == b'%' && first[i + 1] == b'd' {
                split_at = Some(i);
                break;
            }
            i += 1;
        }
        if let Some(at) = split_at {
            let pre = String::from_utf8_lossy(&first[..at]).into_owned();
            let suf = String::from_utf8_lossy(&first[at + 2..]).into_owned();
            (Some(pre), Some(suf))
        } else {
            (None, None)
        }
    };

    idx += 1; // c:1220 args++

    // c:1221-1259 — `-s/-A/-S/-M[arg]` flag block.
    while idx < args.len() {
        let p = &args[idx];
        let bytes = p.as_bytes();
        if bytes.len() < 2 || bytes[0] != b'-' {
            // c:1221
            break;
        }
        let cluster = &bytes[1..];
        let mut ok = true;
        for (i, &c) in cluster.iter().enumerate() {
            match c {
                b's' => single = 1,       // c:1233
                b'S' => flags |= CDF_SEP, // c:1235
                b'A' => {
                    // c:1237
                    if i + 1 < cluster.len() {
                        // c:1238
                        nonarg = Some(String::from_utf8_lossy(&cluster[i + 1..]).into_owned());
                    } else if idx + 1 < args.len() {
                        // c:1241
                        nonarg = Some(args[idx + 1].clone());
                        idx += 1;
                    } else {
                        ok = false;
                    }
                    break;
                }
                b'M' => {
                    // c:1245
                    if i + 1 < cluster.len() {
                        // c:1246
                        match_spec = String::from_utf8_lossy(&cluster[i + 1..]).into_owned();
                    } else if idx + 1 < args.len() {
                        // c:1249
                        match_spec = args[idx + 1].clone();
                        idx += 1;
                    } else {
                        ok = false;
                    }
                    break;
                }
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            break; // c:1230
        }
        idx += 1; // c:1258
    }

    if idx < args.len() && args[idx] == ":" {
        // c:1260
        idx += 1;
    }
    if idx >= args.len() {
        // c:1262
        return None;
    }

    // c:1266 — `tokenize(nonarg = dupstring(nonarg))`. The Rust matcher
    // path lazily tokenizes on use; the stored bytes are the spec text.

    // c:1269 — `all = ret = alloc_cadef(orig_args, single, match, nonarg, flags)`.
    let first_def = alloc_cadef(
        Some(orig_args),
        single,
        &match_spec,
        nonarg.as_deref(),
        flags,
    );

    // ---- spec-list loop state (c:1271-1273) ----
    // `sets` accumulates each Cadef in `snext` order; per-set opts/args/rest
    // are collected in parallel Vecs and linked into the cadef at the end.
    let mut sets: Vec<Box<cadef>> = vec![first_def];
    let mut opts_per_set: Vec<Vec<Box<caopt>>> = vec![Vec::new()];
    let mut args_per_set: Vec<Vec<Box<caarg>>> = vec![Vec::new()];
    let mut rest_per_set: Vec<Option<Box<caarg>>> = vec![None];

    let sargs = idx; // c:1271 saved set-start
    let mut anum: i32 = 1; // c:1203
    let mut doset: Option<String> = None;
    let mut axor: Option<String> = None;
    let mut curset: Option<usize> = None; // c:1201
    let mut pendset: Option<usize> = None;
    let mut foreignset = false;

    // c:1275 — `for (; *args || pendset; args++)`.
    'outer: loop {
        // c:1276 — `if (!*args)` start a fresh set (restart from sargs).
        if idx >= args.len() {
            if pendset.is_none() {
                break 'outer;
            }
            // c:1278-1286 — set_cadef_opts on current; alloc new cadef as snext.
            {
                let cur = sets.last_mut().unwrap();
                let cur_args = args_per_set.last_mut().unwrap();
                // Link the args list into cur so set_cadef_opts can walk it.
                let mut head: Option<Box<caarg>> = None;
                for arg_box in cur_args.drain(..).rev() {
                    let mut a = arg_box;
                    a.next = head;
                    head = Some(a);
                }
                cur.args = head;
                set_cadef_opts(cur); // c:1280
                                     // Stash args back as a Vec for the rest of the loop. We need
                                     // both forms; the linked list will be rebuilt at the end.
                let mut walk = cur.args.take();
                while let Some(mut node) = walk {
                    walk = node.next.take();
                    cur_args.push(node);
                }
            }
            idx = sargs; // c:1278
            doset = None; // c:1279
            sets.push(alloc_cadef(
                None,
                single,
                &match_spec, // c:1281
                nonarg.as_deref(),
                flags,
            ));
            opts_per_set.push(Vec::new());
            args_per_set.push(Vec::new());
            rest_per_set.push(None);
            anum = 1; // c:1283
            foreignset = false; // c:1284
            curset = pendset; // c:1285
            pendset = None; // c:1286
        }

        let arg = &args[idx];
        let arg_bytes = arg.as_bytes();

        // c:1288 — `args[0][0] == '-' && !args[0][1] && args[1]` — set marker.
        if arg_bytes == b"-" && idx + 1 < args.len() {
            if curset.is_some() && curset != Some(idx) {
                // c:1289
                foreignset = true;
                if pendset.is_none() && Some(idx) > curset {
                    // c:1290
                    pendset = Some(idx);
                }
                idx += 1; // c:1292 ++args
            } else {
                // c:1293
                foreignset = false;
                idx += 1;
                let p_str = &args[idx]; // c:1295 char *p = *++args
                let pb = p_str.as_bytes();
                let l = pb.len().saturating_sub(1);
                // c:1298 — `if (*p == '(' && p[l] == ')')` strip parens for axor.
                let (set_name, ax) = if !pb.is_empty() && pb[0] == b'(' && pb[l] == b')' {
                    let inner = String::from_utf8_lossy(&pb[1..l]).into_owned();
                    (inner.clone(), Some(inner))
                } else {
                    (p_str.clone(), None)
                };
                axor = ax;
                if set_name.is_empty() {
                    // c:1302
                    zwarnnam(nam, "empty set name");
                    return None;
                }
                let new_set = tricat(&set_name, "-", ""); // c:1307
                doset = Some(new_set.clone());
                {
                    let cur = sets.last_mut().unwrap();
                    cur.set = Some(new_set);
                }
                curset = Some(idx); // c:1308
            }
            idx += 1;
            continue; // c:1310
        }

        // c:1311 — `args[0][0] == '+' && !args[0][1] && args[1]` — group marker.
        if arg_bytes == b"+" && idx + 1 < args.len() {
            foreignset = false; // c:1315
            idx += 1;
            let p_str = &args[idx]; // c:1316
            let pb = p_str.as_bytes();
            let l = pb.len().saturating_sub(1);
            let (group_name, ax) = if !pb.is_empty() && pb[0] == b'(' && pb[l] == b')' {
                let inner = String::from_utf8_lossy(&pb[1..l]).into_owned();
                (inner.clone(), Some(inner))
            } else {
                (p_str.clone(), None)
            };
            axor = ax;
            if group_name.is_empty() {
                // c:1322
                zwarnnam(nam, "empty group name");
                return None;
            }
            doset = Some(tricat(&group_name, "-", "")); // c:1327
            idx += 1;
            continue; // c:1328
        }

        // c:1329 — `if (foreignset) continue` — skip specs for other sets.
        if foreignset {
            idx += 1;
            continue;
        }

        // c:1331 — parse one spec entry.
        let bytes = arg_bytes;
        let mut p = 0usize;
        let mut xnum: i32 = 0; // c:1332
        let mut not_flag = false;
        if p < bytes.len() && bytes[p] == b'!' {
            // c:1333
            not_flag = true;
            p += 1;
        }

        let mut xor: Option<Vec<String>> = None;
        if p < bytes.len() && bytes[p] == b'(' {
            // c:1335 xor list
            let mut list: Vec<String> = Vec::new();
            // c:1342-1354 — collect words inside parens.
            let mut bad = false;
            'paren: loop {
                if p >= bytes.len() || bytes[p] == b')' {
                    break;
                }
                p += 1; // c:1343 p++
                while p < bytes.len() && inblank(bytes[p]) {
                    p += 1;
                } // c:1343 inblank skip
                if p >= bytes.len() {
                    bad = true;
                    break 'paren;
                }
                if bytes[p] == b')' {
                    break 'paren;
                }
                let q = p;
                p += 1;
                while p < bytes.len() && bytes[p] != b')' && !inblank(bytes[p]) {
                    p += 1;
                }
                if p >= bytes.len() {
                    bad = true;
                    break 'paren;
                } // c:1349
                let word = String::from_utf8_lossy(&bytes[q..p]).into_owned();
                list.push(word);
                xnum += 1; // c:1353
            }
            if bad || p >= bytes.len() || bytes[p] != b')' {
                // c:1356
                zwarnnam(nam, &format!("invalid argument: {}", arg));
                return None;
            }
            if doset.is_some() && axor.is_some() {
                // c:1361
                xnum += 1;
                list.push(axor.clone().unwrap()); // c:1366-1367
            }
            xor = Some(list);
            p += 1; // c:1370
        } else if doset.is_some() && axor.is_some() {
            // c:1371
            xnum = 1;
            xor = Some(vec![axor.clone().unwrap()]);
        }

        // c:1379 — option spec OR rest-arg OR normal-arg.
        let is_opt = p < bytes.len()
            && (bytes[p] == b'-'
                || bytes[p] == b'+'
                || (bytes[p] == b'*'
                    && p + 1 < bytes.len()
                    && (bytes[p + 1] == b'-' || bytes[p + 1] == b'+')));

        if is_opt {
            // ---- c:1381-1580 option spec branch ----
            // The `rec:` goto loop handles `-+`/`+-` duplication by
            // parsing the same spec twice with name[0] flipped between
            // `-` and `+`.
            let mut again_iter = 0i32; // c:1384
            let mut againp_start: Option<usize> = None;
            let mut p_state = p;
            let mut xor_state = xor;
            let mut xnum_state = xnum;

            'rec: loop {
                let mut multi = false; // c:1390
                if p_state < bytes.len() && bytes[p_state] == b'*' {
                    multi = true;
                    p_state += 1;
                }

                let mut name_start: usize;
                let mut name_buf: Vec<u8>;
                let need_flip = p_state + 2 < bytes.len()
                    && ((bytes[p_state] == b'-' && bytes[p_state + 1] == b'+')
                        || (bytes[p_state] == b'+' && bytes[p_state + 1] == b'-'))
                    && bytes[p_state + 2] != b':'
                    && bytes[p_state + 2] != b'['
                    && bytes[p_state + 2] != b'='
                    && bytes[p_state + 2] != b'-'
                    && bytes[p_state + 2] != b'+';

                if need_flip {
                    // c:1393
                    if again_iter == 0 {
                        againp_start = Some(p_state);
                    }
                    name_start = p_state + 1;
                    name_buf = bytes[name_start..].to_vec();
                    if !name_buf.is_empty() {
                        name_buf[0] = if again_iter != 0 { b'-' } else { b'+' };
                    }
                    again_iter += 1;
                    p_state = name_start;
                } else {
                    // c:1404
                    name_start = p_state;
                    name_buf = bytes[name_start..].to_vec();
                    if p_state + 1 < bytes.len()
                        && bytes[p_state] == b'-'
                        && bytes[p_state + 1] == b'-'
                    {
                        p_state += 1; // c:1407 skip 2nd '-'
                    }
                }

                if p_state + 1 >= bytes.len() {
                    // c:1409
                    zwarnnam(nam, &format!("invalid argument: {}", arg));
                    return None;
                }

                // c:1416-1422 — skip option name body up to type byte.
                let mut np = p_state - name_start + 1;
                let nlen = name_buf.len();
                while np < nlen
                    && name_buf[np] != b':'
                    && name_buf[np] != b'['
                    && !((name_buf[np] == b'-' || name_buf[np] == b'+')
                        && np + 1 < nlen
                        && (name_buf[np + 1] == b':' || name_buf[np + 1] == b'['))
                    && !(name_buf[np] == b'='
                        && np + 1 < nlen
                        && (name_buf[np + 1] == b':'
                            || name_buf[np + 1] == b'['
                            || name_buf[np + 1] == b'-'))
                {
                    if name_buf[np] == b'\\' && np + 1 < nlen {
                        np += 1;
                    }
                    np += 1;
                }

                let mut c_byte = if np < nlen { name_buf[np] } else { 0 };
                let opt_name_slice = &name_buf[..np];
                let opt_name = String::from_utf8_lossy(opt_name_slice).into_owned();

                let mut otype = CAO_NEXT; // c:1384
                if c_byte == b'-' {
                    // c:1427
                    otype = CAO_DIRECT;
                    np += 1;
                    c_byte = if np < nlen { name_buf[np] } else { 0 };
                } else if c_byte == b'+' {
                    // c:1430
                    otype = CAO_ODIRECT;
                    np += 1;
                    c_byte = if np < nlen { name_buf[np] } else { 0 };
                } else if c_byte == b'=' {
                    // c:1433
                    otype = CAO_OEQUAL;
                    np += 1;
                    c_byte = if np < nlen { name_buf[np] } else { 0 };
                    if c_byte == b'-' {
                        otype = CAO_EQUAL; // c:1436
                        np += 1;
                        c_byte = if np < nlen { name_buf[np] } else { 0 };
                    }
                }

                // c:1441 — optional `[descr]`.
                let mut descr_str: Option<String> = None;
                if c_byte == b'[' {
                    // c:1441
                    np += 1;
                    let d_start = np;
                    while np < nlen && name_buf[np] != b']' {
                        if name_buf[np] == b'\\' && np + 1 < nlen {
                            np += 1;
                        }
                        np += 1;
                    }
                    if np >= nlen {
                        // c:1446
                        zwarnnam(nam, &format!("invalid option definition: {}", arg));
                        return None;
                    }
                    let d_slice = &name_buf[d_start..np];
                    descr_str = Some(String::from_utf8_lossy(d_slice).into_owned());
                    np += 1;
                    c_byte = if np < nlen { name_buf[np] } else { 0 };
                }

                if c_byte != 0 && c_byte != b':' {
                    // c:1456
                    zwarnnam(nam, &format!("invalid option definition: {}", arg));
                    return None;
                }

                // c:1461 — add option name to xor list if not `*-...`.
                let clean_name = rembslashcolon(&opt_name);
                if !multi {
                    let xv = xor_state.get_or_insert_with(Vec::new);
                    if xv.len() <= xnum_state as usize {
                        xv.resize(xnum_state as usize + 1, String::new());
                    }
                    xv[xnum_state as usize] = clean_name.clone();
                }

                // c:1470-1531 — argument loop for `:descr:action[:...]`.
                let mut oargs: Vec<Box<caarg>> = Vec::new();
                if c_byte == b':' {
                    let mut oanum: i32 = 1; // c:1473
                    let mut onum: i32 = 0;
                    while c_byte == b':' {
                        // c:1479
                        let mut rest = 0;
                        let mut end_str: Option<String> = None;
                        np += 1; // c:1484 *++p
                        let atype: i32;
                        c_byte = if np < nlen { name_buf[np] } else { 0 };
                        if c_byte == b':' {
                            // c:1485
                            atype = CAA_OPT;
                            np += 1;
                        } else if c_byte == b'*' {
                            // c:1487
                            np += 1;
                            if np < nlen && name_buf[np] != b':' {
                                // c:1488
                                let end_start = np;
                                while np < nlen && name_buf[np] != b':' {
                                    if name_buf[np] == b'\\' && np + 1 < nlen {
                                        np += 1;
                                    }
                                    np += 1;
                                }
                                let e_slice = &name_buf[end_start..np];
                                end_str = Some(String::from_utf8_lossy(e_slice).into_owned());
                            }
                            if np >= nlen || name_buf[np] != b':' {
                                // c:1500
                                zwarnnam(nam, &format!("invalid option definition: {}", arg));
                                return None;
                            }
                            np += 1; // c:1507 *++p
                            if np < nlen && name_buf[np] == b':' {
                                // c:1508
                                np += 1;
                                if np < nlen && name_buf[np] == b':' {
                                    // c:1509
                                    atype = CAA_RREST;
                                    np += 1;
                                } else {
                                    atype = CAA_RARGS;
                                }
                            } else {
                                atype = CAA_REST;
                            }
                            rest = 1;
                        } else {
                            atype = CAA_NORMAL;
                        }

                        // c:1521 — parse_caarg.
                        let mut oarg = parse_caarg(
                            if rest != 0 { 0 } else { 1 },
                            atype,
                            oanum,
                            onum,
                            Some(&clean_name),
                            &name_buf,
                            &mut np,
                            doset.as_deref(),
                        );
                        oanum += 1;
                        if atype == CAA_OPT {
                            onum += 1;
                        } // c:1524
                        if let Some(end) = end_str {
                            oarg.end = Some(end); // c:1526
                        }
                        oargs.push(oarg);

                        if rest != 0 {
                            break;
                        } // c:1528
                        c_byte = if np < nlen { name_buf[np] } else { 0 }; // c:1530
                    }
                }

                // c:1534 — build the caopt.
                let mut opt_box = Box::new(caopt::default());
                opt_box.gsname = doset.clone(); // c:1539
                opt_box.name = Some(clean_name.clone()); // c:1540
                opt_box.descr = if let Some(d) = descr_str.clone() {
                    // c:1542
                    Some(d)
                } else if adpre.is_some() && oargs.len() == 1 {
                    // c:1543
                    let first_arg = &oargs[0];
                    let d_field = first_arg.descr.as_deref().unwrap_or("");
                    let has_visible = d_field.bytes().any(|b| !iblank(b));
                    if has_visible {
                        // c:1550
                        Some(tricat(
                            adpre.as_deref().unwrap_or(""),
                            d_field,
                            adsuf.as_deref().unwrap_or(""),
                        ))
                    } else {
                        None // c:1553
                    }
                } else {
                    None
                };
                let xor_clone = if again_iter == 1 {
                    // c:1556
                    xor_state.clone()
                } else {
                    xor_state.take()
                };
                opt_box.xor = xor_clone;
                opt_box.r#type = otype; // c:1557
                opt_box.not = if not_flag { 1 } else { 0 }; // c:1560

                // Link in the arg list.
                let mut head: Option<Box<caarg>> = None;
                for a in oargs.into_iter().rev() {
                    let mut a = a;
                    a.next = head;
                    head = Some(a);
                }
                opt_box.args = head;

                {
                    let cur = sets.last_mut().unwrap();
                    opt_box.num = cur.nopts;
                    cur.nopts += 1; // c:1559
                    if otype == CAO_DIRECT || otype == CAO_EQUAL {
                        // c:1562
                        cur.ndopts += 1;
                    } else if otype == CAO_ODIRECT || otype == CAO_OEQUAL {
                        // c:1564
                        cur.nodopts += 1;
                    }
                    // c:1571 — single-letter lookup table.
                    if single != 0 {
                        let nb = clean_name.as_bytes();
                        if nb.len() == 2 && nb[1] != b'-' {
                            let sidx = single_index(nb[0], nb[1]);
                            if sidx >= 0 {
                                if let Some(ref mut s) = cur.single {
                                    if (sidx as usize) < s.len() {
                                        s[sidx as usize] = Some(Box::new(caopt {
                                            next: None,
                                            name: opt_box.name.clone(),
                                            descr: opt_box.descr.clone(),
                                            xor: opt_box.xor.clone(),
                                            r#type: opt_box.r#type,
                                            args: None,
                                            active: 0,
                                            num: opt_box.num,
                                            gsname: opt_box.gsname.clone(),
                                            not: opt_box.not,
                                        }));
                                    }
                                }
                            }
                        }
                    }
                }

                opts_per_set.last_mut().unwrap().push(opt_box);

                if again_iter == 1 {
                    // c:1576
                    if let Some(start) = againp_start {
                        p_state = start;
                        xnum_state = xnum; // restore
                        xor_state = xor_state.clone();
                        continue 'rec;
                    }
                }
                break 'rec;
            }
        } else if p < bytes.len() && bytes[p] == b'*' {
            // ---- c:1581-1607 rest-arg branch ----
            if not_flag {
                // c:1586
                idx += 1;
                continue;
            }
            p += 1; // c:1589 *++p
            if p >= bytes.len() || bytes[p] != b':' {
                zwarnnam(nam, &format!("invalid rest argument definition: {}", arg));
                return None;
            }
            if rest_per_set.last().unwrap().is_some() {
                // c:1594
                zwarnnam(nam, &format!("doubled rest argument definition: {}", arg));
                return None;
            }
            let mut atype = CAA_REST; // c:1584
            p += 1; // c:1599 *++p
            if p < bytes.len() && bytes[p] == b':' {
                // c:1599
                p += 1;
                if p < bytes.len() && bytes[p] == b':' {
                    // c:1600
                    atype = CAA_RREST;
                    p += 1;
                } else {
                    atype = CAA_RARGS;
                }
            }
            let mut rarg = parse_caarg(0, atype, -1, 0, None, bytes, &mut p, doset.as_deref()); // c:1606
            rarg.xor = xor; // c:1607
            *rest_per_set.last_mut().unwrap() = Some(rarg);
        } else {
            // ---- c:1608-1661 normal-arg branch ----
            if not_flag {
                // c:1614
                idx += 1;
                continue;
            }
            let mut direct = 0; // c:1611
            if p < bytes.len() && idigit(bytes[p]) {
                // c:1617
                direct = 1;
                let mut num: i32 = 0;
                while p < bytes.len() && idigit(bytes[p]) {
                    num = num * 10 + (bytes[p] - b'0') as i32;
                    p += 1;
                }
                anum = num + 1; // c:1624
            } else {
                anum += 1; // c:1627
            }
            if p >= bytes.len() || bytes[p] != b':' {
                // c:1629
                zwarnnam(nam, &format!("invalid argument: {}", arg));
                return None;
            }
            let mut atype = CAA_NORMAL;
            p += 1; // c:1636 *++p
            if p < bytes.len() && bytes[p] == b':' {
                // c:1636
                atype = CAA_OPT;
                p += 1;
            }
            let mut narg =
                parse_caarg(0, atype, anum - 1, 0, None, bytes, &mut p, doset.as_deref()); // c:1641
            narg.xor = xor; // c:1642
            narg.direct = direct; // c:1643

            // c:1647-1661 — sorted insert by num.
            let target = anum - 1;
            let cur_args = args_per_set.last_mut().unwrap();
            let mut insert_at = cur_args.len();
            for (i, existing) in cur_args.iter().enumerate() {
                if existing.num >= target {
                    insert_at = i;
                    break;
                }
            }
            if insert_at < cur_args.len() && cur_args[insert_at].num == target {
                zwarnnam(nam, &format!("doubled argument definition: {}", arg));
                return None;
            }
            cur_args.insert(insert_at, narg);
        }

        idx += 1;
    }

    // c:1664 — final set_cadef_opts on the last set.
    {
        let last_idx = sets.len() - 1;
        let cur = &mut sets[last_idx];
        let cur_args = &mut args_per_set[last_idx];
        let mut head: Option<Box<caarg>> = None;
        for a in cur_args.drain(..).rev() {
            let mut a = a;
            a.next = head;
            head = Some(a);
        }
        cur.args = head;
        set_cadef_opts(cur);
    }

    // ---- finalize: link opts/args/rest per set, then snext-chain ----
    let n_sets = sets.len();
    for i in 0..n_sets {
        // opts — append order.
        let mut head: Option<Box<caopt>> = None;
        for o in opts_per_set[i].drain(..).rev() {
            let mut o = o;
            o.next = head;
            head = Some(o);
        }
        sets[i].opts = head;
        // args was already linked in the per-set finalize step above for
        // every set except possibly the last (which is now done). Walk
        // any still-present Vec entries into the linked list for safety.
        if !args_per_set[i].is_empty() {
            let mut head: Option<Box<caarg>> = None;
            for a in args_per_set[i].drain(..).rev() {
                let mut a = a;
                a.next = head;
                head = Some(a);
            }
            sets[i].args = head;
        }
        sets[i].rest = rest_per_set[i].take();
    }

    // c:1281 — snext chain links each subsequent set off the head.
    while sets.len() > 1 {
        let tail = sets.pop().unwrap();
        let prev = sets.last_mut().unwrap();
        // Walk to the end of the snext chain on prev and attach tail.
        let mut cursor: &mut Option<Box<cadef>> = &mut prev.snext;
        while cursor.is_some() {
            cursor = &mut cursor.as_mut().unwrap().snext;
        }
        *cursor = Some(tail);
    }

    Some(sets.pop().unwrap())
}

// `freecastate` / `freectags` / `freectset` / `freecvdef` real ports
// landed above with the castate / ctags / ctset / cvdef structs.

/// Direct port of `static Cadef get_cadef(char *nam, char **args)`
/// from `Src/Zle/computil.c:1673-1694`. Walks `cadef_cache` looking
/// for an entry whose `defs` array matches the requested `args`
/// (same length + position-for-position string equality). On hit,
/// bumps that entry's `lastt` and returns it. On miss, parses via
/// `parse_cadef` and evicts the entry with the oldest `lastt`
/// (or the first empty slot) to make room for the new one.
///
/// Returns `1` on hit, `0` on miss-and-cache-insert. The previous
/// return-`i32` shape is preserved for callers; the parsed cadef
/// itself lives in `cadef_cache` and is looked up by separate
/// per-name accessors (`ca_get_opt`, `ca_get_arg`, etc.).
pub fn get_cadef(nam: &str, args: &[String]) -> i32 {
    // c:1673
    let na = args.len() as i32;
    let now = {
        // c:1681 time(0)
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };

    if let Ok(mut cache) = cadef_cache.lock() {
        // c:1678 — `for (i = MAX_CACACHE, p = cadef_cache, min = NULL;
        //                  i && *p; p++, i--)`. Linear scan; track LRU
        //          candidate for eviction in `min_idx`.
        let mut min_idx: Option<usize> = None;
        let mut min_lastt: i64 = i64::MAX;
        let mut hit_idx: Option<usize> = None;
        for (i, slot) in cache.iter().enumerate() {
            match slot {
                Some(entry) => {
                    // c:1679 — `if (*p && na == (*p)->ndefs && arrcmp(args, (*p)->defs))`.
                    if entry.ndefs == na
                        && entry.defs.as_deref().map_or(false, |d| {
                            d.len() == args.len() && d.iter().zip(args.iter()).all(|(a, b)| a == b)
                        })
                    {
                        hit_idx = Some(i);
                        break; // c:1682 break on match
                    }
                    // c:1684 — track entry with smallest lastt as eviction target.
                    if entry.lastt < min_lastt {
                        min_lastt = entry.lastt;
                        min_idx = Some(i);
                    }
                }
                None => {
                    // c:1684 — empty slot wins as eviction target.
                    min_idx = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = hit_idx {
            if let Some(entry) = cache[i].as_mut() {
                entry.lastt = now; // c:1681
            }
            return 1; // c:1683 hit
        }
        // c:1688 — parse_cadef; on success replace the chosen slot.
        if let Some(new) = parse_cadef(nam, args) {
            let idx = min_idx.unwrap_or(0);
            cache[idx] = Some(new);
        }
    }
    0 // c:1693 miss
}

/// Direct port of `static Caopt ca_get_opt(Cadef d, char *line, int full,
///                                          char **end)` from
/// `Src/Zle/computil.c:1706-1742`. Looks up an option-spec by name
/// against `line`. With `full=0`, also accepts a prefix-of-`line`
/// match where the option name is a prefix and the rest of `line` is
/// the option's argument (handles `=` / `--name=value` shapes per
/// `CAO_OEQUAL` / `CAO_EQUAL`). Sets `*end` to the byte offset past
/// the option text (and past the `=` separator when applicable).
/// Returns a cloned shallow copy of the matched `caopt` (without its
/// `next` chain) — Rust ownership artifact, equivalent to C returning
/// the aliased `Caopt` pointer.
pub fn ca_get_opt(
    d: &cadef,
    line: &str,
    full: i32, // c:1706
    end: &mut usize,
) -> Option<Box<caopt>> {
    let line_bytes = line.as_bytes();

    // c:1712-1718 — exact match against an active option name.
    let mut cur = d.opts.as_deref();
    while let Some(p) = cur {
        // c:1712
        if p.active != 0 {
            // c:1713
            if let Some(name) = p.name.as_deref() {
                if name == line {
                    *end = line_bytes.len(); // c:1715
                    return Some(Box::new(caopt {
                        // c:1717 — C returns the aliased `p` (WITH its args);
                        // the clone must carry `p->args` so option-argument
                        // completion (`-f <TAB>`) sees `state.def`. Dropping
                        // args here makes every arg-taking option look like a
                        // bare flag, so `_arguments` completes options instead
                        // of the option's argument.
                        next: None,
                        name: p.name.clone(),
                        descr: p.descr.clone(),
                        xor: p.xor.clone(),
                        r#type: p.r#type,
                        args: p.args.clone(),
                        active: p.active,
                        num: p.num,
                        gsname: p.gsname.clone(),
                        not: p.not,
                    }));
                }
            }
        }
        cur = p.next.as_deref();
    }

    if full == 0 {
        // c:1720
        // c:1722-1739 — prefix-match path for `name=value` / `nameSPC value`.
        let mut cur = d.opts.as_deref();
        while let Some(p) = cur {
            if p.active != 0 {
                // c:1723
                if let Some(name) = p.name.as_deref() {
                    // c:1723-1724 — short args/NEXT → exact match, else strpfx.
                    let is_match = if p.args.is_none() || p.r#type == CAO_NEXT {
                        name == line
                    } else {
                        strpfx(name, line)
                    };
                    if is_match {
                        let l = name.len();
                        // c:1726-1728 — for OEQUAL/EQUAL, the char at name's
                        // end must be `=` or absent; otherwise skip.
                        if (p.r#type == CAO_OEQUAL || p.r#type == CAO_EQUAL)
                            && l < line_bytes.len()
                            && line_bytes[l] != b'='
                        {
                            cur = p.next.as_deref();
                            continue; // c:1728
                        }
                        // c:1731-1736 — set end past the option (+= 1 for `=`).
                        let mut at = l;
                        if (p.r#type == CAO_OEQUAL || p.r#type == CAO_EQUAL)
                            && l < line_bytes.len()
                            && line_bytes[l] == b'='
                        {
                            at += 1; // c:1734
                        }
                        *end = at; // c:1736
                        return Some(Box::new(caopt {
                            // c:1738 — as above (c:1717), preserve `p->args`
                            // so the prefix-match path (`--opt=val`) also
                            // carries the option's argument spec.
                            next: None,
                            name: p.name.clone(),
                            descr: p.descr.clone(),
                            xor: p.xor.clone(),
                            r#type: p.r#type,
                            args: p.args.clone(),
                            active: p.active,
                            num: p.num,
                            gsname: p.gsname.clone(),
                            not: p.not,
                        }));
                    }
                }
            }
            cur = p.next.as_deref();
        }
    }
    None // c:1741
}

/// Direct port of `static Caopt ca_get_sopt(Cadef d, char *line,
///                                           char **end, LinkList *lp)`
/// from `Src/Zle/computil.c:1747-1781`. Single-letter option lookup
/// for clumped flags like `-abc`. Walks `line[1..]` consulting
/// `d->single[]` for each char; CAO_NEXT matches accumulate in `lp`,
/// the first non-NEXT match terminates and sets `*end` past it.
/// Returns the terminating Caopt (cloned, no chain) or None.
pub fn ca_get_sopt(
    d: &cadef,
    line: &str, // c:1747
    end: &mut usize,
    lp: &mut Option<Vec<Box<caopt>>>,
) -> Option<Box<caopt>> {
    let line_bytes = line.as_bytes();
    if line_bytes.is_empty() {
        *lp = None;
        return None;
    }
    let pre = line_bytes[0]; // c:1750
    let mut idx: usize = 1;
    *lp = None; // c:1754

    let single = match d.single.as_ref() {
        // c:1757
        Some(s) => s,
        None => return None,
    };

    let mut p_cur: Option<&caopt> = None; // c:1755 p = NULL
    let mut pp_cur: Option<&caopt> = None;
    let mut list_acc: Option<Vec<Box<caopt>>> = None;

    while idx < line_bytes.len() {
        // c:1755 for (;*line;line++)
        let ch = line_bytes[idx];
        let sidx = single_index(pre, ch); // c:1756

        // c:1756 — d->single[sidx] lookup (assigns to p if valid).
        let lookup: Option<&caopt> = if sidx >= 0 && (sidx as usize) < single.len() {
            single[sidx as usize].as_deref()
        } else {
            None
        };
        if lookup.is_some() {
            p_cur = lookup;
        }
        let active_with_args = lookup.filter(|p| p.active != 0 && p.args.is_some());

        if let Some(p) = active_with_args {
            // c:1757
            if p.r#type == CAO_NEXT {
                // c:1758
                let list = list_acc.get_or_insert_with(Vec::new);
                list.push(Box::new(caopt {
                    // c:1761 addlinknode
                    next: None,
                    name: p.name.clone(),
                    descr: p.descr.clone(),
                    xor: p.xor.clone(),
                    r#type: p.r#type,
                    args: None,
                    active: p.active,
                    num: p.num,
                    gsname: p.gsname.clone(),
                    not: p.not,
                }));
            } else {
                // c:1762
                idx += 1; // c:1764 line++
                if (p.r#type == CAO_OEQUAL || p.r#type == CAO_EQUAL)         // c:1765
                    && idx < line_bytes.len() && line_bytes[idx] == b'='
                {
                    idx += 1; // c:1767
                }
                *end = idx; // c:1768
                pp_cur = Some(p); // c:1770
                break; // c:1771
            }
        } else if p_cur.is_none() || p_cur.map_or(true, |p| p.active == 0) {
            // c:1773
            return None; // c:1774
        }

        // c:1775 — pp = (p->name[0] == pre ? p : NULL); p = NULL.
        pp_cur = p_cur.filter(|p| {
            p.name
                .as_deref()
                .and_then(|n| n.as_bytes().first().copied())
                .map_or(false, |b| b == pre)
        });
        p_cur = None;
        idx += 1; // c:1755 line++
    }

    // c:1778 — pp && end: *end = line.
    if pp_cur.is_some() {
        *end = idx;
    }

    *lp = list_acc;

    pp_cur.map(|p| {
        Box::new(caopt {
            // c:1780
            next: None,
            name: p.name.clone(),
            descr: p.descr.clone(),
            xor: p.xor.clone(),
            r#type: p.r#type,
            args: None,
            active: p.active,
            num: p.num,
            gsname: p.gsname.clone(),
            not: p.not,
        })
    })
}

/// Direct port of `static int ca_foreign_opt(Cadef curset, Cadef all,
///                                            char *option)` from
/// `Src/Zle/computil.c:1786-1802`. Walks the `snext` chain of `all`
/// skipping `curset` and reports whether any other set defines an
/// option with the requested name. Returns 1 on match, 0 otherwise.
pub fn ca_foreign_opt(curset: &cadef, all: &cadef, option: &str) -> i32 {
    // c:1787
    let curset_ptr = curset as *const cadef;
    let mut d_opt = Some(all);
    while let Some(d) = d_opt {
        // c:1792
        if std::ptr::addr_eq(d as *const cadef, curset_ptr) {
            // c:1793
            d_opt = d.snext.as_deref();
            continue;
        }
        let mut p = d.opts.as_deref();
        while let Some(opt) = p {
            // c:1796
            if opt.name.as_deref() == Some(option) {
                // c:1797
                return 1; // c:1798
            }
            p = opt.next.as_deref();
        }
        d_opt = d.snext.as_deref();
    }
    0 // c:1801
}

/// Direct port of `static Caarg ca_get_arg(Cadef d, int n)` from
/// `Src/Zle/computil.c:1807-1823`. Walks `d->args` looking for the
/// arg whose `[min, num]` range contains `n`. Falls back to `d->rest`
/// when no positional matches. Returns a shallow clone (no `next`)
/// of the matched arg.
pub fn ca_get_arg(d: &cadef, mut n: i32) -> Option<Box<caarg>> {
    // c:1807
    if d.argsactive == 0 {
        // c:1809
        return None; // c:1822
    }

    // c:1810-1816 — skip inactive entries (advance `n` to compensate for
    // each skipped one, mirroring the C `n++` inside the loop).
    let mut a = d.args.as_deref();
    while let Some(node) = a {
        // c:1812
        let in_range = node.active != 0 && n >= node.min && n <= node.num;
        if in_range {
            break;
        } // c:1812 inverted
        if node.active == 0 {
            // c:1813
            n += 1; // c:1814
        }
        a = node.next.as_deref(); // c:1815
    }

    if let Some(node) = a {
        // c:1817
        if node.active != 0 && node.min <= n && node.num >= n {
            return Some(Box::new(caarg {
                // c:1818
                next: None,
                descr: node.descr.clone(),
                xor: node.xor.clone(),
                action: node.action.clone(),
                r#type: node.r#type,
                end: node.end.clone(),
                opt: node.opt.clone(),
                num: node.num,
                min: node.min,
                direct: node.direct,
                active: node.active,
                gsname: node.gsname.clone(),
            }));
        }
    }

    // c:1820 — rest fallback.
    if let Some(r) = d.rest.as_deref() {
        if r.active != 0 {
            return Some(Box::new(caarg {
                next: None,
                descr: r.descr.clone(),
                xor: r.xor.clone(),
                action: r.action.clone(),
                r#type: r.r#type,
                end: r.end.clone(),
                opt: r.opt.clone(),
                num: r.num,
                min: r.min,
                direct: r.direct,
                active: r.active,
                gsname: r.gsname.clone(),
            }));
        }
    }
    None // c:1820
}

/// Direct port of `static void ca_inactive(Cadef d, char **xor, int cur,
///                                          int opts)` from
/// `Src/Zle/computil.c:1832-1918`. Marks options/args inactive based
/// on a xor-list (or, when `opts=1`, deactivates all options except
/// rest-of-line args). Each xor entry can be:
///   - bare name → exact-match opt or numeric positional or `:`/`-`/`*`/group
///   - `group-name-:` / `group-name--` → group-scoped exclusion
///   - excludeall path (just a set/group name) → kills the whole set
pub fn ca_inactive(d: &mut cadef, xor: &[String], cur: i32, opts: i32) {
    // c:1832

    if (xor.is_empty() && opts == 0)                                         // c:1834
        || cur > COMPCURRENT.load(Ordering::Relaxed)
    {
        return;
    }

    // c:1839 — single-letter exclusions only when at compcurrent (option
    // clumping safety: a prefix-of-longer-opt at cursor mustn't kill the
    // multi-letter form prematurely).
    let single = opts == 0 && cur == COMPCURRENT.load(Ordering::Relaxed);

    // c:1841 — iterate xor entries. When opts=1 we synthesize a "-" pass.
    let iter_xor: Vec<String> = if opts != 0 {
        vec!["-".to_string()]
    } else {
        xor.to_vec()
    };

    for x_orig in iter_xor.iter() {
        let mut x = x_orig.as_str();
        let mut excludeall = 0; // c:1842
        let mut grp: Option<&str> = None;
        let mut grplen: usize = 0;

        // c:1845-1858 — split off optional `group-name-` prefix.
        let xb = x.as_bytes();
        let mut sep_byte = if xb.is_empty() { 0u8 } else { xb[0] };
        let mut sep_pos = 0usize;
        loop {
            if sep_pos >= xb.len() {
                break;
            }
            sep_byte = xb[sep_pos];
            if sep_byte == b'+'
                || sep_byte == b'-'
                || sep_byte == b':'
                || sep_byte == b'*'
                || idigit(sep_byte)
            {
                break;
            }
            // Find next '-'.
            let after = &xb[sep_pos..];
            let dash_off = after.iter().position(|&b| b == b'-');
            match dash_off {
                None => {
                    excludeall = 1; // c:1850
                    sep_pos = xb.len();
                    break;
                }
                Some(d) => {
                    let next = sep_pos + d + 1;
                    if next >= xb.len() {
                        // c:1848
                        excludeall = 1;
                        sep_pos = xb.len();
                        break;
                    }
                    sep_pos = next;
                }
            }
        }
        if sep_pos > 0 && sep_pos < xb.len() {
            // c:1859
            grp = Some(&x[..sep_pos]);
            grplen = sep_pos;
            x = &x[sep_pos..];
        } else if sep_pos > 0 && excludeall != 0 && sep_pos == xb.len() {
            // c:1850 path — the whole string was a group name.
            grp = Some(x);
            grplen = sep_pos;
            x = "";
        }
        let xb = x.as_bytes();

        // c:1865 — excludeall or `:` alone.
        if excludeall != 0 || (xb.len() == 1 && xb[0] == b':') {
            if let Some(g) = grp {
                // c:1866
                let mut cur_arg = d.args.as_deref_mut();
                while let Some(a) = cur_arg {
                    let matches = a.gsname.as_deref().map_or(false, |gn| {
                        let gnb = gn.as_bytes();
                        gnb.len() == grplen + (excludeall as usize) && gn.starts_with(g)
                    });
                    if matches {
                        a.active = 0; // c:1872
                    }
                    cur_arg = a.next.as_deref_mut();
                }
                if let Some(r) = d.rest.as_deref_mut() {
                    let matches = r.gsname.as_deref().map_or(false, |gn| {
                        let gnb = gn.as_bytes();
                        gnb.len() == grplen + (excludeall as usize) && gn.starts_with(g)
                    });
                    if matches {
                        r.active = 0; // c:1876
                    }
                }
            } else {
                d.argsactive = 0; // c:1878
            }
        }

        // c:1881 — excludeall or `-` alone: kill options.
        if excludeall != 0 || (xb.len() == 1 && xb[0] == b'-') {
            let mut cur_opt = d.opts.as_deref_mut();
            while let Some(p) = cur_opt {
                let grp_ok = grp.map_or(true, |g| {
                    p.gsname.as_deref().map_or(false, |gn| {
                        gn.len() == grplen + (excludeall as usize) && gn.starts_with(g)
                    })
                });
                let single_skip = single
                    && p.name.as_deref().map_or(false, |n| {
                        let nb = n.as_bytes();
                        nb.len() >= 3 && nb[0] != 0
                    });
                if grp_ok && !single_skip {
                    p.active = 0; // c:1888
                }
                cur_opt = p.next.as_deref_mut();
            }
        }

        // c:1891 — excludeall or `*` alone: kill rest.
        if excludeall != 0 || (xb.len() == 1 && xb[0] == b'*') {
            if let Some(r) = d.rest.as_deref_mut() {
                let grp_ok = grp.map_or(true, |g| {
                    r.gsname.as_deref().map_or(false, |gn| {
                        gn.len() == grplen + (excludeall as usize) && gn.starts_with(g)
                    })
                });
                if grp_ok {
                    r.active = 0; // c:1895
                }
            }
        }

        if excludeall == 0 {
            // c:1898
            if !xb.is_empty() && idigit(xb[0]) {
                // c:1899
                let n: i32 = x
                    .bytes()
                    .take_while(|b| idigit(*b))
                    .fold(0i32, |acc, b| acc * 10 + (b - b'0') as i32);
                let mut cur_arg = d.args.as_deref_mut();
                let mut hit: Option<&mut caarg> = None;
                while let Some(a) = cur_arg {
                    if a.num >= n {
                        hit = Some(a);
                        break;
                    }
                    cur_arg = a.next.as_deref_mut();
                }
                if let Some(a) = hit {
                    if a.num == n {
                        let grp_ok = grp.map_or(true, |g| {
                            a.gsname.as_deref().map_or(false, |gn| gn.starts_with(g))
                        });
                        if grp_ok {
                            a.active = 0; // c:1908
                        }
                    }
                }
            } else {
                // c:1909 — ca_get_opt for full match.
                let mut end_unused = 0usize;
                if let Some(matched) = ca_get_opt(d, x, 1, &mut end_unused) {
                    let grp_ok = grp.map_or(true, |g| {
                        matched
                            .gsname
                            .as_deref()
                            .map_or(false, |gn| gn.starts_with(g))
                    });
                    let single_skip = single
                        && matched.name.as_deref().map_or(false, |n| {
                            let nb = n.as_bytes();
                            nb.len() >= 3 && nb[0] != 0
                        });
                    if grp_ok && !single_skip {
                        // Walk d.opts to find the actual node and clear its active.
                        let target_name = matched.name.clone();
                        let mut cur_opt = d.opts.as_deref_mut();
                        while let Some(p) = cur_opt {
                            if p.name == target_name {
                                p.active = 0; // c:1912
                                break;
                            }
                            cur_opt = p.next.as_deref_mut();
                        }
                    }
                }
            }
            if opts != 0 {
                break; // c:1914
            }
        }
        let _ = sep_byte;
    }
}

// =====================================================================
// `castate` — command-line parse state for `_arguments`.
// Src/Zle/computil.c:1920-1957.
// =====================================================================

/// Port of `typedef struct castate *Castate` from
/// `Src/Zle/computil.c:1922`.
pub type Castate = Box<castate>; // c:1922

/// Direct port of `struct castate` from `Src/Zle/computil.c:1928-1953`.
/// Encapsulates the parsed-command-line state for one `_arguments`
/// set — used as a linked list (`snext`) with one state per set.
#[derive(Debug, Default, Clone)]
#[allow(non_camel_case_types)]
pub struct castate {
    // c:1928
    pub snext: Option<Box<castate>>, // c:1929 Castate snext
    pub d: Option<Box<cadef>>,       // c:1930 Cadef d
    pub nopts: i32,                  // c:1931
    pub def: Option<Box<caarg>>,     // c:1932 Caarg def
    pub ddef: Option<Box<caarg>>,    // c:1933 Caarg ddef
    pub curopt: Option<Box<caopt>>,  // c:1934 Caopt curopt
    pub dopt: Option<Box<caopt>>,    // c:1935 Caopt dopt
    pub opt: i32,                    // c:1936
    pub arg: i32,                    // c:1937
    pub argbeg: i32,                 // c:1938
    pub optbeg: i32,                 // c:1939
    pub nargbeg: i32,                // c:1941
    pub restbeg: i32,                // c:1942
    pub curpos: i32,                 // c:1943
    pub argend: i32,                 // c:1944
    pub inopt: i32,                  // c:1945
    pub inarg: i32,                  // c:1946
    pub nth: i32,                    // c:1947
    pub singles: i32,                // c:1948
    pub oopt: i32,                   // c:1949
    pub actopts: i32,                // c:1950
    pub args: Option<Vec<String>>,   // c:1951 LinkList args
    pub oargs: Option<Vec<Option<Vec<String>>>>, // c:1952 LinkList *oargs
}

/// Port of `static int ca_parsed` from `Src/Zle/computil.c:1956`.
pub static ca_parsed: std::sync::atomic::AtomicI32 = // c:1956
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static int ca_doff` from `Src/Zle/computil.c:1960`. Count
/// of chars of ignored prefix (for clumped options or arg to an
/// option).
pub static ca_doff: std::sync::atomic::AtomicI32 = // c:1960
    std::sync::atomic::AtomicI32::new(0);

/// Direct port of `static void freecastate(Castate s)` from
/// `Src/Zle/computil.c:1960`. Frees the args/oargs lists.
pub fn freecastate(s: &mut castate) {
    // c:1960
    s.args = None; // c:1960 freelinklist(s->args)
    s.oargs = None; // c:1966-1969 freelinklist per slot
}

/// Port of `ca_opt_arg(Caopt opt, char *line)` from Src/Zle/computil.c:1976.
/// WARNING: param names don't match C — Rust=(opt_name, line, equal_kind) vs C=(opt, line)
pub fn ca_opt_arg(opt_name: &str, line: &str, equal_kind: bool) -> String {
    // c:1976
    // C body c:1978-1996: walks `o = opt->name` and `line` byte-by-byte,
    //                     skipping `\\` escapes; if any quote (`\\` `'` `"`)
    //                     in line, advance line; once they diverge, return
    //                     dup of remaining line minus optional `=` if
    //                     opt is CAO_EQUAL/CAO_OEQUAL.
    let o_bytes = opt_name.as_bytes();
    let l_bytes = line.as_bytes();
    let mut oi = 0usize;
    let mut li = 0usize;
    loop {
        // c:1980
        if oi >= o_bytes.len() || li >= l_bytes.len() {
            break;
        }
        let mut oc = o_bytes[oi];
        if oc == b'\\' {
            // c:1981
            oi += 1;
            if oi >= o_bytes.len() {
                break;
            }
            oc = o_bytes[oi];
        }
        let mut lc = l_bytes[li];
        if matches!(lc, b'\\' | b'\'' | b'"') {
            // c:1983
            li += 1;
            if li >= l_bytes.len() {
                break;
            }
            lc = l_bytes[li];
        }
        if oc != lc {
            // c:1985
            break;
        }
        oi += 1;
        li += 1;
    }
    let rest = &l_bytes[li..];
    let mut s = String::from_utf8_lossy(rest).into_owned();
    if equal_kind && s.starts_with('\\') {
        // c:2004
        s.remove(0);
    }
    if equal_kind {
        s = s.strip_prefix('=').map(|t| t.to_string()).unwrap_or(s); // c:2004
    }
    s
}

/// Direct port of `static int ca_parse_line(Cadef d, Cadef all, int multi,
///                                            int first)` from
/// `Src/Zle/computil.c:2004-2403`. Walks `compwords[1..]` matching
/// each word against the cadef's option/arg specs, updating active
/// flags via `ca_inactive` and populating `ca_laststate` with the
/// completion-point info downstream subcommands need. Returns 1 when
/// the set should be skipped (foreign option spotted in multi-set
/// mode), 0 otherwise.
///
/// Substrate notes:
/// - `endpat` (CAA_RREST/RARGS end-pattern matching) is compiled via
///   `patcompile`; the resulting `Patprog` is held in the local
///   `endpat` slot exactly like the C code.
/// - `napat` (the `-A` "non-arg" pattern) is also compiled via
///   `patcompile`.
/// - The `sopts` (clumped single-letter remainders) LinkList is
///   represented as a `Vec<Box<caopt>>` queue.
pub fn ca_parse_line(d: &mut cadef, all: &cadef, multi: i32, first: i32) -> i32 {
    // c:2004

    let compcur = COMPCURRENT.load(Ordering::Relaxed);

    // c:2019 — free old state if this is the first set.
    if first != 0 && ca_alloced.load(Ordering::Relaxed) != 0 {
        if let Ok(mut ls) = ca_laststate.lock() {
            freecastate(&mut ls);
            ls.snext = None;
        }
    }

    // c:2030-2036 — mark everything active.
    let mut p = d.opts.as_deref_mut();
    while let Some(o) = p {
        o.active = 1;
        p = o.next.as_deref_mut();
    }
    d.argsactive = 1;
    if let Some(r) = d.rest.as_deref_mut() {
        r.active = 1;
    }
    let mut a = d.args.as_deref_mut();
    while let Some(ar) = a {
        ar.active = 1;
        a = ar.next.as_deref_mut();
    }

    // c:2040-2056 — build the initial castate.
    let compwords: Vec<String> = COMPWORDS
        .get()
        .and_then(|m| m.lock().ok().map(|w| w.clone()))
        .unwrap_or_default();
    let argend_init = (compwords.len() as i32) - 1;

    // Set up the working state. Note d is stored as None in the
    // working `state`; we re-populate ca_laststate.d from a clone at
    // the end (we can't move d into state since the caller still owns
    // it).
    let mut state = castate {
        snext: None,
        d: None,
        nopts: d.nopts,
        def: None,
        ddef: None,
        curopt: None,
        dopt: None,
        opt: 1,
        arg: 1,
        argbeg: 1,
        optbeg: 1,
        nargbeg: 1,
        restbeg: 1,
        actopts: 1,
        nth: 1,
        inarg: 1,
        inopt: 0,
        singles: 0,
        oopt: 0,
        argend: argend_init,
        curpos: compcur,
        args: Some(Vec::new()),
        oargs: Some((0..d.nopts as usize).map(|_| None).collect()),
    };
    ca_alloced.store(1, Ordering::Relaxed);

    // Snapshot state → ca_laststate (the "early return on empty"
    // path uses this).
    if let Ok(mut ls) = ca_laststate.lock() {
        *ls = clone_castate(&state, d);
    }

    if compwords.len() < 2 {
        // c:2058
        if let Ok(mut ls) = ca_laststate.lock() {
            ls.opt = 0;
            ls.arg = 0;
        }
        // c:2061 goto end — fall through to actopts count.
    } else {
        // c:2063-2064 — compile -A nonarg pattern.
        let napat = d.nonarg.as_deref().and_then(|s| {
            patcompile(
                &{
                    let mut __pat_tok = (s).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                },
                0,
                None::<&mut String>,
            )
        });
        let mut endpat: Option<Patprog> = None;

        // c:2068 — walk words.
        let mut cur = 2i32;
        let mut argxor: Option<Vec<String>> = None;
        let mut sopts: Vec<Box<caopt>> = Vec::new();
        let mut wasopt_idx: Option<usize> = None;
        let mut doff: i32 = 0;
        let mut adef: Option<Box<caarg>> = None;
        let mut ddef: Option<Box<caarg>> = None;
        let mut dopt: Option<Box<caopt>> = None;
        state.curopt = None;
        state.def = None;

        loop {
            let line_idx = (cur - 1) as usize;
            if line_idx >= compwords.len() {
                break;
            }
            let oline = compwords[line_idx].clone();
            let mut line = oline.clone();
            ddef = None;
            adef = None;
            dopt = None;
            state.singles = 0;
            let mut arglast = 0;

            remnulargs(&mut line);
            line = untokenize(&line);

            // c:2095 — apply pending arg-xor.
            if let Some(xor) = argxor.take() {
                ca_inactive(d, &xor, cur - 1, 0);
            }

            // c:2099 — CDF_SEP `--` separator turns off option parsing.
            if (d.flags & CDF_SEP) != 0 && cur != compcur && state.actopts != 0 && line == "--" {
                ca_inactive(d, &[], cur, 1);
                state.actopts = 0;
                cur += 1;
                continue;
            }

            // c:2108 — already have a def from previous opt, collect args.
            if state.def.is_some() {
                state.arg = 0;
                if let Some(co) = state.curopt.as_deref() {
                    let cn = co.num as usize;
                    if let Some(oargs) = state.oargs.as_mut() {
                        if cn < oargs.len() {
                            oargs[cn].get_or_insert_with(Vec::new).push(oline.clone());
                        }
                    }
                }
                let def_type = state.def.as_deref().map_or(0, |d| d.r#type);
                let def_is_opt = def_type == CAA_OPT;
                state.opt = if def_is_opt { 1 } else { 0 };
                if def_is_opt {
                    if state.def.as_deref().map_or(false, |d| d.opt.is_some()) {
                        state.oopt += 1;
                    }
                }

                if def_type == CAA_REST || def_type == CAA_RARGS || def_type == CAA_RREST {
                    // c:2118 — end-pattern check.
                    let matched_end = state
                        .def
                        .as_deref()
                        .and_then(|d| d.end.as_deref())
                        .map_or(false, |_| {
                            endpat.as_ref().map_or(false, |ep| pattry(ep, &line))
                        });
                    if matched_end {
                        state.def = None;
                        state.curopt = None;
                        state.opt = 1;
                        state.arg = 1;
                        state.argend = cur - 1;
                        if let Ok(mut ls) = ca_laststate.lock() {
                            ls.argend = cur - 1;
                        }
                        // c:2124 goto cont.
                    }
                } else {
                    // c:2125 — advance to next arg slot.
                    let next = state.def.as_deref().and_then(|d| d.next.clone());
                    if next.is_some() {
                        state.def = next;
                        state.argbeg = cur;
                        state.argend = argend_init;
                    } else if let Some(s) = sopts.first().cloned() {
                        // c:2128 — pop a queued single-letter opt arg.
                        sopts.remove(0);
                        state.curopt = Some(s);
                        state.def = state.curopt.as_deref().and_then(|c| c.args.clone());
                        state.opt = 0;
                        state.argbeg = cur;
                        state.optbeg = cur;
                        state.inopt = cur;
                        state.argend = argend_init;
                        doff = 0;
                        state.singles = 1;
                        if let Some(co) = state.curopt.as_deref() {
                            let cn = co.num as usize;
                            if let Some(oargs) = state.oargs.as_mut() {
                                if cn < oargs.len() && oargs[cn].is_none() {
                                    oargs[cn] = Some(Vec::new());
                                }
                            }
                        }
                        // c:2138 goto cont.
                    } else {
                        state.curopt = None;
                        state.opt = 1;
                    }
                }
            } else {
                state.opt = 1;
                state.arg = 1;
                state.curopt = None;
            }
            if state.opt != 0 {
                let lb = line.as_bytes();
                state.opt = if lb.is_empty() {
                    0
                } else if lb.len() == 1 {
                    1
                } else {
                    2
                };
            }

            let mut pe_off: i32 = 0;
            wasopt_idx = None;

            // c:2156 — option lookup.
            let opt_match = if state.opt == 2 {
                let lb = line.as_bytes();
                if !lb.is_empty() && (lb[0] == b'-' || lb[0] == b'+') {
                    let mut end = 0usize;
                    if let Some(found) = ca_get_opt(d, &line, 0, &mut end) {
                        pe_off = end as i32;
                        // c:2158 — for OEQUAL/EQUAL check `=` boundary.
                        let pe_ok = match found.r#type {
                            t if t == CAO_OEQUAL => {
                                (line_idx + 1 < compwords.len())
                                    || (pe_off > 0 && lb.get(pe_off as usize - 1) == Some(&b'='))
                            }
                            t if t == CAO_EQUAL => {
                                pe_off > 0
                                    && (lb.get(pe_off as usize - 1) == Some(&b'=')
                                        || pe_off as usize >= lb.len())
                            }
                            _ => true,
                        };
                        if pe_ok {
                            Some(found)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(co) = opt_match {
                // Bind found opt details for later.
                let co_name = co.name.clone().unwrap_or_default();
                let co_type = co.r#type;
                let co_num = co.num;
                let co_xor = co.xor.clone().unwrap_or_default();
                let co_args = co.args.clone();
                state.curopt = Some(co);
                let pe_at_eq =
                    pe_off > 0 && line.as_bytes().get(pe_off as usize - 1) == Some(&b'=');
                let pe_tail_present = (pe_off as usize) < line.as_bytes().len();

                let take_args = co_type != CAO_EQUAL || pe_at_eq;
                state.def = if take_args { co_args.clone() } else { None };
                if state.def.is_some() {
                    ddef = state.def.clone();
                    dopt = state.curopt.clone();
                }

                doff = pe_off;
                state.optbeg = cur;
                state.argbeg = cur;
                state.inopt = cur;
                state.argend = argend_init;
                let single_ok = d.single.is_some()
                    && !pe_tail_present
                    && co_name.as_bytes().len() >= 2
                    && co_name.as_bytes()[1] != b'-'
                    && co_name.as_bytes().get(2).is_none();
                state.singles = if single_ok { 1 } else { 0 };

                let cn = co_num as usize;
                if let Some(oargs) = state.oargs.as_mut() {
                    if cn < oargs.len() && oargs[cn].is_none() {
                        oargs[cn] = Some(Vec::new());
                    }
                }
                ca_inactive(d, &co_xor, cur, 0); // c:2179

                let collect_arg = state.def.is_some()
                    && (co_type == CAO_DIRECT
                        || co_type == CAO_EQUAL
                        || (co_type == CAO_ODIRECT && pe_tail_present)
                        || (co_type == CAO_OEQUAL && (pe_tail_present || pe_at_eq)));
                if collect_arg {
                    let dtype = state.def.as_deref().map_or(0, |d| d.r#type);
                    if dtype != CAA_REST && dtype != CAA_RARGS && dtype != CAA_RREST {
                        let next = state.def.as_deref().and_then(|d| d.next.clone());
                        state.def = next;
                    }
                    let arg_str = ca_opt_arg(&co_name, &oline, false);
                    if let Some(oargs) = state.oargs.as_mut() {
                        if cn < oargs.len() {
                            oargs[cn].get_or_insert_with(Vec::new).push(arg_str);
                        }
                    }
                }
                if state.def.is_some() {
                    state.opt = 0;
                } else {
                    if d.single.is_none()
                        || (co_name.as_bytes().len() >= 3 && co_name.as_bytes()[1] != 0)
                    {
                        wasopt_idx = Some(co_num as usize); // c:2201
                    }
                    state.curopt = None;
                }
            } else if state.opt == 2
                && d.single.is_some()
                && line
                    .as_bytes()
                    .first()
                    .copied()
                    .map_or(false, |b| b == b'-' || b == b'+')
            {
                // c:2204 — single-letter clump.
                let mut end = 0usize;
                let mut tmp_sopts: Option<Vec<Box<caopt>>> = None;
                let s_match = ca_get_sopt(d, &line, &mut end, &mut tmp_sopts);
                if let Some(queued) = tmp_sopts {
                    sopts.extend(queued);
                }
                let active_sopt = s_match.or_else(|| {
                    if cur != compcur && !sopts.is_empty() {
                        Some(sopts.remove(0))
                    } else {
                        None
                    }
                });
                if let Some(co) = active_sopt {
                    let co_name = co.name.clone().unwrap_or_default();
                    let co_type = co.r#type;
                    let co_num = co.num;
                    let co_xor = co.xor.clone().unwrap_or_default();
                    let co_args = co.args.clone();
                    state.curopt = Some(co);

                    let cn = co_num as usize;
                    if let Some(oargs) = state.oargs.as_mut() {
                        if cn < oargs.len() && oargs[cn].is_none() {
                            oargs[cn] = Some(Vec::new());
                        }
                    }
                    state.def = co_args.clone();
                    if co_type == CAO_NEXT && cur == compcur {
                        ddef = None;
                    } else {
                        ddef = state.def.clone();
                    }
                    dopt = state.curopt.clone();
                    doff = end as i32;
                    state.optbeg = cur;
                    state.argbeg = cur;
                    state.inopt = cur;
                    state.argend = argend_init;
                    state.singles = if end >= line.as_bytes().len() { 1 } else { 0 };

                    let lb = line.as_bytes();
                    let pre = lb.first().copied().unwrap_or(0);
                    let mut p_idx = 1usize;
                    while p_idx < end.min(lb.len()) {
                        let sidx = single_index(pre, lb[p_idx]);
                        if sidx >= 0 && (sidx as usize) < d.single.as_ref().map_or(0, |s| s.len()) {
                            let tmp_xor = d
                                .single
                                .as_ref()
                                .and_then(|s| s.get(sidx as usize))
                                .and_then(|so| so.as_ref())
                                .and_then(|so| so.xor.clone());
                            let tmp_num = d
                                .single
                                .as_ref()
                                .and_then(|s| s.get(sidx as usize))
                                .and_then(|so| so.as_ref())
                                .map_or(-1, |so| so.num);
                            let tn = tmp_num as usize;
                            if tmp_num >= 0 {
                                if let Some(oargs) = state.oargs.as_mut() {
                                    if tn < oargs.len() && oargs[tn].is_none() {
                                        oargs[tn] = Some(Vec::new());
                                    }
                                }
                            }
                            if let Some(xor) = tmp_xor {
                                ca_inactive(d, &xor, cur, 0);
                            }
                        }
                        p_idx += 1;
                    }

                    let pe_tail_present = end < line.as_bytes().len();
                    let pe_at_eq = end > 0 && line.as_bytes().get(end - 1) == Some(&b'=');
                    let collect_arg = state.def.is_some()
                        && (co_type == CAO_DIRECT
                            || co_type == CAO_EQUAL
                            || (co_type == CAO_ODIRECT && pe_tail_present)
                            || (co_type == CAO_OEQUAL && (pe_tail_present || pe_at_eq)));
                    if collect_arg {
                        let dtype = state.def.as_deref().map_or(0, |d| d.r#type);
                        if dtype != CAA_REST && dtype != CAA_RARGS && dtype != CAA_RREST {
                            let next = state.def.as_deref().and_then(|d| d.next.clone());
                            state.def = next;
                        }
                        let arg_str = ca_opt_arg(&co_name, &line, false);
                        if let Some(oargs) = state.oargs.as_mut() {
                            if cn < oargs.len() {
                                oargs[cn].get_or_insert_with(Vec::new).push(arg_str);
                            }
                        }
                    }
                    if state.def.is_some() {
                        state.opt = 0;
                    } else {
                        state.curopt = None;
                    }
                }
            } else if multi != 0
                && line
                    .as_bytes()
                    .first()
                    .copied()
                    .map_or(false, |b| b == b'-' || b == b'+')
                && cur != compcur
                && ca_foreign_opt(d, all, &line) != 0
            {
                return 1; // c:2258
            } else if state.arg != 0 && cur <= compcur {
                // c:2259
                // c:2264 — napat -A pattern.
                if let Some(np) = napat.as_ref() {
                    if cur < compcur && state.actopts != 0 {
                        if pattry(np, &line) {
                            cur += 1;
                            continue;
                        }
                        ca_inactive(d, &[], cur + 1, 1);
                        state.actopts = 0;
                    }
                }

                arglast = 1;
                if state.inopt != 0 {
                    // c:2274
                    state.inopt = 0;
                    state.nargbeg = cur - 1;
                    state.argend = argend_init;
                }
                // c:2279 — no args/rest + non-empty non-flag line → skip set.
                let lb = line.as_bytes();
                let non_flag = !lb.is_empty() && lb[0] != b'-' && lb[0] != b'+';
                if d.args.is_none() && d.rest.is_none() && non_flag {
                    if multi == 0 && cur > compcur {
                        break;
                    }
                    return 1;
                }

                adef = ca_get_arg(d, state.nth);
                state.def = adef.clone();
                let dtype = state.def.as_deref().map_or(0, |d| d.r#type);
                if state.def.is_some() && (dtype == CAA_RREST || dtype == CAA_RARGS) {
                    if ca_laststate
                        .lock()
                        .map(|ls| ls.def.is_some())
                        .unwrap_or(false)
                    {
                        break;
                    }
                    state.opt = if cur == state.nargbeg + 1
                        && (multi == 0 || line.is_empty() || lb[0] == b'-' || lb[0] == b'+')
                    {
                        1
                    } else {
                        0
                    };
                    state.optbeg = state.nargbeg;
                    state.argbeg = cur - 1;
                    state.argend = argend_init;
                    // c:2311 — gather remaining words into state.args.
                    while line_idx < compwords.len() {
                        state
                            .args
                            .get_or_insert_with(Vec::new)
                            .push(compwords[line_idx].clone());
                        cur += 1;
                        if (cur - 1) as usize >= compwords.len() {
                            break;
                        }
                    }
                    if let Ok(mut ls) = ca_laststate.lock() {
                        *ls = clone_castate(&state, d);
                        ls.ddef = None;
                        ls.dopt = None;
                    }
                    break;
                }
                state.args.get_or_insert_with(Vec::new).push(line.clone());
                if let Some(a) = adef.as_deref() {
                    state.oopt = a.num - state.nth;
                }

                if state.def.is_some() && cur != compcur {
                    // c:2323
                    argxor = state.def.as_deref().and_then(|d| d.xor.clone());
                }
                let dtype2 = state.def.as_deref().map_or(0, |d| d.r#type);
                if state.def.is_some()
                    && dtype2 != CAA_NORMAL
                    && dtype2 != CAA_OPT
                    && state.inarg != 0
                {
                    state.restbeg = cur;
                    state.inarg = 0;
                } else if state.def.is_none() || dtype2 == CAA_NORMAL || dtype2 == CAA_OPT {
                    state.inarg = 1;
                }
                state.nth += 1;
                state.def = None;
            }

            // c:2338 — end-pattern compile for rest-args.
            if state.def.is_some() && state.curopt.is_some() {
                let dt = state.def.as_deref().map_or(0, |d| d.r#type);
                if dt == CAA_RREST || dt == CAA_RARGS {
                    let end_pat_str = state.def.as_deref().and_then(|d| d.end.clone());
                    if let Some(eps) = end_pat_str {
                        endpat = patcompile(
                            &{
                                let mut __pat_tok = (&eps).to_string();
                                crate::ported::glob::tokenize(&mut __pat_tok);
                                __pat_tok
                            },
                            0,
                            None::<&mut String>,
                        );
                    } else {
                        // c:2342-2353 — no end-pattern: gather rest into oargs.
                        if cur < compcur {
                            if let Ok(mut ls) = ca_laststate.lock() {
                                *ls = clone_castate(&state, d);
                            }
                        }
                        let cn = state.curopt.as_deref().map(|c| c.num as usize);
                        if let Some(cn) = cn {
                            if let Some(oargs) = state.oargs.as_mut() {
                                if cn < oargs.len() {
                                    let bucket = oargs[cn].get_or_insert_with(Vec::new);
                                    let mut k = line_idx;
                                    while k < compwords.len() {
                                        bucket.push(compwords[k].clone());
                                        k += 1;
                                    }
                                }
                            }
                        }
                        if let Ok(mut ls) = ca_laststate.lock() {
                            ls.ddef = None;
                            ls.dopt = None;
                        }
                        break;
                    }
                }
            } else if state.def.is_some() {
                let eps = state.def.as_deref().and_then(|d| d.end.clone());
                if let Some(eps) = eps {
                    endpat = patcompile(
                        &{
                            let mut __pat_tok = (&eps).to_string();
                            crate::ported::glob::tokenize(&mut __pat_tok);
                            __pat_tok
                        },
                        0,
                        None::<&mut String>,
                    );
                }
            }

            // c:2360 cont: — checkpoint to ca_laststate.
            if cur + 1 == compcur {
                if let Ok(mut ls) = ca_laststate.lock() {
                    *ls = clone_castate(&state, d);
                    ls.ddef = None;
                    ls.dopt = None;
                }
            } else if cur == compcur {
                let mut ls = ca_laststate.lock().unwrap();
                if ls.def.is_none() {
                    if let Some(ddef_v) = ddef.clone() {
                        ls.def = Some(ddef_v);
                        ls.singles = state.singles;
                        if state
                            .curopt
                            .as_deref()
                            .map_or(false, |c| c.r#type == CAO_NEXT)
                        {
                            ls.ddef = ddef.clone();
                            ls.dopt = dopt.clone();
                            ls.def = None;
                            ls.opt = 1;
                            // Mark curopt active again in d.
                            if let Some(co) = state.curopt.as_deref() {
                                let target_name = co.name.clone();
                                let mut p = d.opts.as_deref_mut();
                                while let Some(op) = p {
                                    if op.name == target_name {
                                        op.active = 1;
                                        break;
                                    }
                                    p = op.next.as_deref_mut();
                                }
                            }
                        } else {
                            ca_doff.store(doff, Ordering::Relaxed);
                            ls.opt = 0;
                        }
                    } else {
                        ls.def = adef.clone();
                        ls.opt = if arglast == 0
                            || multi == 0
                            || line.is_empty()
                            || line.as_bytes()[0] == b'-'
                            || line.as_bytes()[0] == b'+'
                        {
                            1
                        } else {
                            0
                        };
                        ls.ddef = None;
                        ls.dopt = None;
                        ls.optbeg = state.nargbeg;
                        ls.argbeg = state.restbeg;
                        ls.argend = state.argend;
                        ls.singles = state.singles;
                        ls.oopt = state.oopt;
                        if let Some(wi) = wasopt_idx {
                            let mut p = d.opts.as_deref_mut();
                            while let Some(op) = p {
                                if op.num as usize == wi {
                                    op.active = 1;
                                    break;
                                }
                                p = op.next.as_deref_mut();
                            }
                        }
                    }
                }
            }
            cur += 1;
        }
        let _ = (endpat, ddef, dopt, adef);
    }

    // c:2397-2400 — count active opts.
    let mut actopts = 0i32;
    let mut p = d.opts.as_deref();
    while let Some(o) = p {
        if o.active != 0 {
            actopts += 1;
        }
        p = o.next.as_deref();
    }
    if let Ok(mut ls) = ca_laststate.lock() {
        ls.actopts = actopts;
        // Make sure ls.d reflects the (now-mutated) d.
        ls.d = Some(Box::new(clone_cadef_shallow(d)));
    }
    0
}

/// Port of `ca_nullist(LinkList l)` from Src/Zle/computil.c:2411.
pub fn ca_nullist(l: &[String]) -> Vec<u8> {
    // c:2411
    // C body c:2413-2419 — `if (l) { array = zlinklist2array(l, 0);
    //                              ret = zjoin(array, '\\0', 0); free(array);
    //                              return ret; } else return ztrdup("")`.
    //                      Returns NUL-joined byte buffer.
    if l.is_empty() {
        return Vec::new(); // c:2419
    }
    let mut out = Vec::new();
    for (i, item) in l.iter().enumerate() {
        if i > 0 {
            out.push(0);
        }
        out.extend_from_slice(item.as_bytes());
    }
    out
}

/// Port of `ca_colonlist(LinkList l)` from Src/Zle/computil.c:2428.
pub fn ca_colonlist(l: &[String]) -> String {
    // c:2428
    // C body c:2430-2459 — joins l with `:`, escapes `:` and `\`
    //                      with `\` per item.
    if l.is_empty() {
        return String::new(); // c:2459
    }
    let mut out = String::new();
    for (i, item) in l.iter().enumerate() {
        // c:2444
        if i > 0 {
            out.push(':'); // c:2452
        }
        for ch in item.chars() {
            if ch == ':' || ch == '\\' {
                // c:2447
                out.push('\\');
            }
            out.push(ch);
        }
    }
    out
}

/// Direct port of `static void ca_set_data(LinkList descr, LinkList act,
///                                           LinkList subc, char *opt,
///                                           Caarg arg, Caopt optdef,
///                                           int single)` from
/// `Src/Zle/computil.c:2472-2582`. Appends to descr/act/subc the
/// description/action/subcontext for each `arg` whose `[min,num]`
/// range covers `ca_laststate.nth`. When `opt` is non-None, all
/// args are treated as option args; otherwise positional. Recurses
/// via the goto-rec C path to retry after the first loop when more
/// state remains.
pub fn ca_set_data(
    descr: &mut Vec<String>, // c:2472
    act: &mut Vec<String>,
    subc: &mut Vec<String>,
    opt: Option<&str>,
    start_arg: Option<Box<caarg>>,
    optdef: Option<&caopt>,
    single: i32,
) {
    let mut arg: Option<Box<caarg>> = start_arg;
    let mut opt = opt.map(|s| s.to_string());
    let mut restr = 0;
    let mut miss = 0;
    let mut oopt = 1i32;
    let mut lopt;

    'rec: loop {
        // c:2481 — addopt = (opt ? 0 : ca_laststate.oopt).
        let addopt = if opt.is_some() {
            0
        } else {
            ca_laststate.lock().map(|s| s.oopt).unwrap_or(0)
        };

        // c:2483 — main arg walk.
        while let Some(a) = arg.as_ref() {
            let cont = {
                let nth = ca_laststate.lock().map(|s| s.nth).unwrap_or(0);
                opt.is_some() || a.num < 0 || (a.min <= nth + addopt && a.num >= nth)
            };
            if !cont {
                break;
            }

            lopt = a.r#type == CAA_OPT; // c:2486
            if opt.is_none() && !lopt && oopt > 0 {
                // c:2487
                oopt = 0;
            }

            // c:2490 — dedup: skip if (descr, act) pair already present.
            let mut dup = false;
            let descr_str = a.descr.clone().unwrap_or_default();
            let act_str = a.action.clone().unwrap_or_default();
            for (d, ac) in descr.iter().zip(act.iter()) {
                if d == &descr_str && ac == &act_str {
                    dup = true;
                    break;
                }
            }

            // c:2497 — with ignored prefix, no normal args.
            if single != 0 && a.opt.is_none() {
                return;
            }

            if !dup {
                // c:2500
                descr.push(descr_str.clone()); // c:2501
                act.push(act_str.clone());

                if restr == 0 {
                    // c:2504
                    let nrestr = if a.r#type == CAA_RARGS {
                        // c:2506
                        let (optbeg, argend) = ca_laststate
                            .lock()
                            .map(|s| (s.optbeg, s.argend))
                            .unwrap_or((0, 0));
                        restrict_range(optbeg, argend);
                        1
                    } else if a.r#type == CAA_RREST {
                        // c:2508
                        let (argbeg, argend) = ca_laststate
                            .lock()
                            .map(|s| (s.argbeg, s.argend))
                            .unwrap_or((0, 0));
                        restrict_range(argbeg, argend);
                        1
                    } else {
                        0
                    };
                    restr = nrestr;
                }

                // c:2511 — build subcontext string.
                let buf = if let Some(o) = a.opt.as_deref() {
                    // c:2511
                    let gs = a.gsname.as_deref().unwrap_or("");
                    if a.num > 0 && a.r#type < CAA_REST {
                        // c:2514
                        format!("{}option{}-{}", gs, o, a.num)
                    } else {
                        // c:2518
                        format!("{}option{}-rest", gs, o)
                    }
                } else if a.num > 0 {
                    // c:2520
                    if let Some(gs) = a.gsname.as_deref() {
                        format!("{}argument-{}", gs, a.num)
                    } else {
                        format!("argument-{}", a.num)
                    }
                } else {
                    // c:2523
                    if let Some(gs) = a.gsname.as_deref() {
                        format!("{}argument-rest", gs)
                    } else {
                        "argument-rest".to_string()
                    }
                };
                subc.push(buf); // c:2527
            }

            // c:2539 — guard: NORMAL inside an opt where opt requires its
            // argument as a separate word — return so we don't keep trying
            // to match positionals.
            if a.r#type == CAA_NORMAL && opt.is_some() {
                if let Some(od) = optdef {
                    if od.r#type == CAO_NEXT || od.r#type == CAO_ODIRECT || od.r#type == CAO_OEQUAL
                    {
                        return;
                    }
                }
            }

            if single != 0 {
                break;
            } // c:2545

            // c:2548-2568 — advance to the next arg.
            if opt.is_none() {
                // c:2548
                let next_is_none_and_miss = a.num >= 0 && a.next.is_none() && miss != 0;
                if next_is_none_and_miss {
                    // c:2549
                    let rest = ca_laststate
                        .lock()
                        .ok()
                        .and_then(|s| s.d.as_ref().and_then(|d| d.rest.clone()));
                    arg = rest.filter(|r| r.active != 0); // c:2550
                } else {
                    let onum = a.num; // c:2553
                    let nth = ca_laststate.lock().map(|s| s.nth).unwrap_or(0);
                    let rest_flag = onum != a.min && onum == nth; // c:2554
                    let next = a.next.clone();
                    if let Some(n) = next {
                        // c:2555
                        if n.num != onum + 1 {
                            miss = 1;
                        } // c:2556
                        arg = Some(n);
                    } else if rest_flag || (oopt > 0 && opt.is_none()) {
                        // c:2558
                        let rest = ca_laststate
                            .lock()
                            .ok()
                            .and_then(|s| s.d.as_ref().and_then(|d| d.rest.clone()));
                        arg = rest.filter(|r| r.active != 0);
                        oopt = -1;
                    } else {
                        arg = None;
                    }
                }
            } else {
                // c:2564
                if !lopt {
                    break;
                } // c:2565
                arg = a.next.clone(); // c:2567
            }
        }

        // c:2570 — retry as positional after the option args path.
        let laststate_oopt = ca_laststate.lock().map(|s| s.oopt).unwrap_or(0);
        let cur_lopt = arg.as_ref().map_or(false, |a| a.r#type == CAA_OPT);
        if single == 0 && opt.is_some() && (cur_lopt || laststate_oopt != 0) {
            opt = None;
            let nth = ca_laststate.lock().map(|s| s.nth).unwrap_or(0);
            // c:2572 — arg = ca_get_arg(ca_laststate.d, ca_laststate.nth).
            arg = ca_laststate
                .lock()
                .ok()
                .and_then(|s| s.d.as_ref().and_then(|d| ca_get_arg(d, nth)));
            continue 'rec;
        }
        // c:2575 — retry as rest after positional path.
        if opt.is_none() && oopt > 0 {
            oopt = -1;
            let rest = ca_laststate
                .lock()
                .ok()
                .and_then(|s| s.d.as_ref().and_then(|d| d.rest.clone()));
            arg = rest.filter(|r| r.active != 0);
            continue 'rec;
        }
        break 'rec;
    }
}

/// Direct port of `static int bin_comparguments(char *nam, char **args,
///                                                 UNUSED(Options ops),
///                                                 UNUSED(int func))`
/// from `Src/Zle/computil.c:2585-2914`. Full subcommand dispatch for
/// `comparguments -i/-D/-O/-L/-s/-M/-a/-W/-n`. Each branch consumes
/// the parsed `ca_laststate` from `ca_parse_line`.
pub fn bin_comparguments(
    nam: &str,
    args: &[String], // c:2585
    _ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:2590
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {
        return 1;
    }
    let a0 = args[0].as_bytes();
    // c:2594 — must be `-X` exactly (2 chars).
    if a0.len() != 2 || a0[0] != b'-' {
        zwarnnam(nam, &format!("invalid argument: {}", args[0]));
        return 1;
    }
    let sub = a0[1];

    // c:2598 — non-init subcommands require ca_parsed.
    if sub != b'i' && sub != b'I' && ca_parsed.load(Ordering::Relaxed) == 0 {
        zwarnnam(nam, "no parsed state");
        return 1;
    }

    // c:2602 — per-subcommand arg-count bounds.
    let (min, max): (i32, i32) = match sub {
        b'i' => (2, -1),
        b'D' => (3, 3),
        b'O' => (4, 4),
        b'L' => (3, 4),
        b's' => (1, 1),
        b'M' => (1, 1),
        b'a' => (0, 0),
        b'W' => (3, 3),
        b'n' => (1, 1),
        _ => {
            zwarnnam(nam, &format!("invalid option: {}", args[0]));
            return 1;
        }
    };
    let n = (args.len() as i32) - 1;
    if n < min {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    if max >= 0 && n > max {
        zwarnnam(nam, "too many arguments");
        return 1;
    }

    match sub {
        b'i' => {
            // c:2625
            // c:2629 — compcurrent > 1 && compwords[0].
            let compcur = COMPCURRENT.load(Ordering::Relaxed);
            let compwords_nonempty = COMPWORDS
                .get()
                .and_then(|m| m.lock().ok().map(|w| !w.is_empty() && !w[0].is_empty()))
                .unwrap_or(false);
            if compcur <= 1 || !compwords_nonempty {
                return 1; // c:2670
            }
            // c:2636 — get_cadef(nam, args[1..]).
            // get_cadef returns 1 on cache hit. Look up the cached cadef
            // from the cadef_cache by argv match and run parse_line.
            let spec = &args[1..];
            let _ = get_cadef(nam, spec); // c:2636
                                          // Now find the cadef in the cache.
            let cached: Option<Box<cadef>> = {
                let cache = cadef_cache.lock().ok();
                cache.and_then(|c| {
                    c.iter().find_map(|slot| {
                        slot.as_ref()
                            .filter(|e| {
                                e.ndefs == spec.len() as i32
                                    && e.defs.as_deref().map_or(false, |d| {
                                        d.len() == spec.len()
                                            && d.iter().zip(spec.iter()).all(|(a, b)| a == b)
                                    })
                            })
                            .cloned()
                    })
                })
            };
            let Some(mut def_head) = cached else {
                return 1;
            };
            ca_parsed.store(0, Ordering::Relaxed); // c:2634
            ca_doff.store(0, Ordering::Relaxed);
            let all_clone = Box::new(clone_cadef_shallow(&def_head));

            // c:2643-2664 — for each set walk: track which parses
            // succeeded ("use"). When a set succeeds AND more sets
            // remain, snapshot ca_laststate into a fallback chain.
            // When the final set rejects, restore the most-recent
            // saved state (or fail with ret=1 if no fallback).
            let mut first = 1;
            let mut def_opt: Option<Box<cadef>> = Some(def_head);
            let mut multi = 0;
            // Look ahead once to see if we have more than one set;
            // matches C's `multi = !!def->snext` at c:2639.
            if let Some(ref d) = def_opt {
                if d.snext.is_some() {
                    multi = 1;
                }
            }
            let mut states: Vec<castate> = Vec::new(); // c:2632
            let mut ret = 0i32;

            while let Some(mut current) = def_opt {
                let next = current.snext.take();
                let parse_ret = ca_parse_line(&mut current, &all_clone, multi, first);
                let use_state = parse_ret == 0; // c:2644
                let has_next = next.is_some();
                if use_state && has_next {
                    // c:2646
                    // c:2648 — snapshot ca_laststate, push onto fallback.
                    if let Ok(ls) = ca_laststate.lock() {
                        states.push(clone_castate_full(&ls));
                    }
                } else if !use_state && !has_next {
                    // c:2652
                    // c:2654 — restore most-recent saved state (if any).
                    if let Some(saved) = states.pop() {
                        if let Ok(mut ls) = ca_laststate.lock() {
                            freecastate(&mut ls);
                            *ls = saved;
                        }
                    } else {
                        ret = 1; // c:2661
                    }
                }
                first = 0; // c:2663
                def_opt = next;
            }
            ca_parsed.store(1, Ordering::Relaxed); // c:2665

            // c:2666 — thread fallback chain into ca_laststate.snext.
            if !states.is_empty() {
                if let Ok(mut ls) = ca_laststate.lock() {
                    // Build a linked snext chain from oldest → newest.
                    let mut head: Option<Box<castate>> = None;
                    for s in states.into_iter().rev() {
                        let mut s = s;
                        s.snext = head;
                        head = Some(Box::new(s));
                    }
                    ls.snext = head;
                }
            }
            ret // c:2668
        }

        b'D' => {
            // c:2672
            let mut descr: Vec<String> = Vec::new();
            let mut act: Vec<String> = Vec::new();
            let mut subc: Vec<String> = Vec::new();
            let mut ret = 1i32;
            ignore_prefix(ca_doff.load(Ordering::Relaxed));

            // Walk lstate (ca_laststate + its snext chain).
            let mut state_clone = ca_laststate.lock().map(|s| clone_castate_full(&s)).ok();
            while let Some(s) = state_clone {
                let arg = s.def.clone();
                if let Some(a) = arg {
                    ret = 0;
                    let opt_str = a.opt.clone();
                    let optdef = s.curopt.clone();
                    ca_set_data(
                        &mut descr,
                        &mut act,
                        &mut subc,
                        opt_str.as_deref(),
                        Some(a),
                        optdef.as_deref(),
                        if ca_doff.load(Ordering::Relaxed) > 0 {
                            1
                        } else {
                            0
                        },
                    );
                }
                state_clone = s.snext.map(|b| *b);
            }
            if ret == 0 {
                // c:2698
                setaparam(&args[1], descr);
                setaparam(&args[2], act);
                setaparam(&args[3], subc);
            }
            ret
        }

        b'M' => {
            // c:2827
            let m = ca_laststate
                .lock()
                .ok()
                .and_then(|s| s.d.as_ref().and_then(|d| d.r#match.clone()))
                .unwrap_or_default();
            setsparam(&args[1], &m);
            0
        }

        b'a' => {
            // c:2833
            let mut state_clone = ca_laststate.lock().map(|s| clone_castate_full(&s)).ok();
            while let Some(s) = state_clone {
                if s.d
                    .as_ref()
                    .map_or(false, |d| d.args.is_some() || d.rest.is_some())
                {
                    return 0;
                }
                state_clone = s.snext.map(|b| *b);
            }
            1 // c:2840
        }

        b'n' => {
            // c:2899
            let optbeg = ca_laststate.lock().map(|s| s.optbeg).unwrap_or(0);
            let kshoffset = if isset(KSHARRAYS) { 0 } else { 1 };
            setiparam(&args[1], (optbeg + kshoffset) as i64);
            0
        }

        b'O' => {
            // c:2705
            // Build the four lists; for each non-`not` active opt assign it
            // to one of {next, direct, odirect, equal}.
            let mut next_l: Vec<String> = Vec::new();
            let mut direct_l: Vec<String> = Vec::new();
            let mut odirect_l: Vec<String> = Vec::new();
            let mut equal_l: Vec<String> = Vec::new();
            let mut ret = 1i32;

            let mut state_clone = ca_laststate.lock().map(|s| clone_castate_full(&s)).ok();
            while let Some(s) = state_clone {
                // c:2721 — gate on actopts + position.
                let actopts_ok = s.actopts != 0
                    && (s.opt != 0
                        || (ca_doff.load(Ordering::Relaxed) != 0 && s.def.is_some())
                        || (s.def.is_some()
                            && s.def.as_deref().map_or(false, |d| {
                                d.opt.is_some()
                                    && (d.r#type == CAA_OPT || (d.r#type >= CAA_RARGS && d.num < 0))
                            })));
                let pos_ok = s.def.is_none()
                    || s.def.as_deref().map_or(true, |d| d.r#type < CAA_RARGS)
                    || (s.def.as_deref().map_or(false, |d| d.r#type == CAA_RARGS)
                        && s.curpos == s.argbeg + 1)
                    || COMPCURRENT.load(Ordering::Relaxed) == 1;
                if actopts_ok && pos_ok {
                    ret = 0;
                    if let Some(d) = s.d.as_ref() {
                        let mut p = d.opts.as_deref();
                        while let Some(opt) = p {
                            if opt.active != 0 && opt.not == 0 {
                                let bucket: &mut Vec<String> = match opt.r#type {
                                    t if t == CAO_NEXT => &mut next_l,
                                    t if t == CAO_DIRECT => &mut direct_l,
                                    t if t == CAO_ODIRECT => &mut odirect_l,
                                    _ => &mut equal_l,
                                };
                                let name_esc = bslashcolon(opt.name.as_deref().unwrap_or(""));
                                let str_val = if let Some(desc) = opt.descr.as_deref() {
                                    format!("{}:{}", name_esc, desc)
                                } else {
                                    name_esc
                                };
                                if !bucket.iter().any(|s| s == &str_val) {
                                    bucket.push(str_val);
                                }
                            }
                            p = opt.next.as_deref();
                        }
                    }
                }
                state_clone = s.snext.map(|b| *b);
            }

            if ret == 0 {
                setaparam(&args[1], next_l);
                setaparam(&args[2], direct_l);
                setaparam(&args[3], odirect_l);
                setaparam(&args[4], equal_l);
                0
            } else {
                let singles = ca_laststate.lock().map(|s| s.singles).unwrap_or(0);
                if singles != 0 {
                    2
                } else {
                    1
                } // c:2769
            }
        }

        b'L' => {
            // c:2771
            // c:2787 — for each state, ca_get_opt(d, args[1], 1, NULL).
            let mut descr: Vec<String> = Vec::new();
            let mut act: Vec<String> = Vec::new();
            let mut subc: Vec<String> = Vec::new();
            let mut ret = 1i32;
            let mut state_clone = ca_laststate.lock().map(|s| clone_castate_full(&s)).ok();
            while let Some(s) = state_clone {
                if let Some(d) = s.d.as_ref() {
                    let mut end = 0usize;
                    if let Some(opt) = ca_get_opt(d, &args[1], 1, &mut end) {
                        if opt.args.is_some() {
                            ret = 0;
                            let opt_name = opt.name.clone();
                            let opt_args = opt.args.clone();
                            ca_set_data(
                                &mut descr,
                                &mut act,
                                &mut subc,
                                opt_name.as_deref(),
                                opt_args,
                                Some(&opt),
                                1,
                            );
                        }
                    }
                }
                state_clone = s.snext.map(|b| *b);
            }
            if ret == 0 {
                setaparam(&args[2], descr);
                setaparam(&args[3], act);
                setaparam(&args[4], subc);
            }
            ret
        }

        b's' => {
            // c:2803
            let mut state_clone = ca_laststate.lock().map(|s| clone_castate_full(&s)).ok();
            while let Some(s) = state_clone {
                let single_active = s.d.as_ref().map_or(false, |d| d.single.is_some())
                    && s.singles != 0
                    && s.actopts != 0;
                if single_active {
                    let kind = if let (Some(_), Some(dopt)) = (&s.ddef, &s.dopt) {
                        match dopt.r#type {
                            t if t == CAO_DIRECT => "direct",
                            t if t == CAO_OEQUAL || t == CAO_EQUAL => "equal",
                            _ => "next",
                        }
                    } else {
                        ""
                    };
                    setsparam(&args[1], kind);
                    return 0;
                }
                state_clone = s.snext.map(|b| *b);
            }
            1
        }

        b'W' => {
            // c:2841
            // Build state.args concat and oargs concat for $opt_args.
            let mut all_args: Vec<String> = Vec::new();
            let opt_args_use_nul = !args[3].starts_with('0');
            let mut state_clone = ca_laststate.lock().map(|s| clone_castate_full(&s)).ok();
            // Pass 1: state.args.
            let mut snapshot = state_clone.clone();
            while let Some(s) = snapshot {
                if let Some(a) = s.args.as_ref() {
                    all_args.extend(a.iter().cloned());
                }
                snapshot = s.snext.map(|b| *b);
            }
            setaparam(&args[1], all_args);

            // Pass 2: oargs into a hash.
            let mut hash_vec: Vec<String> = Vec::new();
            while let Some(s) = state_clone {
                if let Some(d) = s.d.as_ref() {
                    let mut o = d.opts.as_deref();
                    let mut a_idx = 0usize;
                    let oargs_ref = s.oargs.as_deref();
                    while let Some(op) = o {
                        if let Some(oa) = oargs_ref
                            .and_then(|v| v.get(a_idx))
                            .and_then(|x| x.as_ref())
                        {
                            let key = match (op.gsname.as_deref(), op.name.as_deref()) {
                                (Some(gs), Some(n)) => format!("{}{}", gs, n),
                                (None, Some(n)) => n.to_string(),
                                _ => String::new(),
                            };
                            hash_vec.push(key);
                            let joined = if opt_args_use_nul {
                                String::from_utf8_lossy(&ca_nullist(oa)).into_owned()
                            } else {
                                ca_colonlist(oa)
                            };
                            hash_vec.push(joined);
                        }
                        a_idx += 1;
                        o = op.next.as_deref();
                    }
                }
                state_clone = s.snext.map(|b| *b);
            }
            sethparam(&args[2], hash_vec);
            0
        }

        _ => 0,
    }
}

// =====================================================================
// `cvdef` / `cvval` — `_values` completion cache types.
// Src/Zle/computil.c:2919-2956. CVV_* and MAX_CVCACHE consts
// already declared above (file scope).
// =====================================================================

/// Port of `typedef struct cvdef *Cvdef` from `Src/Zle/computil.c:2919`.
pub type Cvdef = Box<cvdef>; // c:2919

/// Direct port of `struct cvdef` from `Src/Zle/computil.c:2924-2935`.
/// One parsed `_values` definition entry, cached for reuse.
#[derive(Debug, Default, Clone)]
#[allow(non_camel_case_types)]
pub struct cvdef {
    // c:2924
    pub descr: Option<String>,     // c:2925 char *descr
    pub hassep: i32,               // c:2926
    pub sep: i32,                  // c:2927 char sep
    pub argsep: i32,               // c:2928 char argsep
    pub next: Option<Box<cvdef>>,  // c:2929 Cvdef next
    pub vals: Option<Box<cvval>>,  // c:2930 Cvval vals
    pub defs: Option<Vec<String>>, // c:2931 char **defs
    pub ndefs: i32,                // c:2932
    pub lastt: i64,                // c:2933 time_t lastt
    pub words: i32,                // c:2934
}
/// Port of `typedef struct cvval *Cvval` from `computil.c:2920`.
pub type Cvval = Box<cvval>; // c:2920

/// Direct port of `struct cvval` from `Src/Zle/computil.c:2939-2947`.
/// One value definition inside a cvdef.
#[derive(Debug, Default, Clone)]
#[allow(non_camel_case_types)]
pub struct cvval {
    // c:2939
    pub next: Option<Box<cvval>>, // c:2940 Cvval next
    pub name: Option<String>,     // c:2961 char *name
    pub descr: Option<String>,    // c:2961 char *descr
    pub xor: Option<Vec<String>>, // c:2961 char **xor
    pub r#type: i32,              // c:2961 int type (CVV_*)
    pub arg: Option<Box<caarg>>,  // c:2961 Caarg arg
    pub active: i32,              // c:2961
}

// =====================================================================
// CVV_* — Cvval value-kind — `computil.c:2949-2951`.
// =====================================================================

/// Port of `CVV_NOARG` from `computil.c:2949`. Value without argument.
pub const CVV_NOARG: i32 = 0; // c:2949
/// Port of `CVV_ARG` from `computil.c:2950`. Value requires argument.
pub const CVV_ARG: i32 = 1; // c:2950
/// Port of `CVV_OPT` from `computil.c:2951`. Argument optional.
pub const CVV_OPT: i32 = 2; // c:2951

/// Port of `MAX_CVCACHE` from `computil.c:2955`. Cvdef LRU cache size.
pub const MAX_CVCACHE: usize = 8; // c:2955

/// Port of `static Cvdef cvdef_cache[MAX_CVCACHE]` from
/// `Src/Zle/computil.c:2956`. Same LRU layout as cadef_cache;
/// `get_cvdef` scans for a defs-match hit, evicts the oldest slot
/// on miss.
pub static cvdef_cache: std::sync::Mutex<[Option<Box<cvdef>>; MAX_CVCACHE]> = // c:2956
    std::sync::Mutex::new([const { None }; MAX_CVCACHE]);

/// Direct port of `static void freecvdef(Cvdef d)` from
/// `Src/Zle/computil.c:2961`. Walks the vals chain freeing
/// each cvval (which frees its caarg via freecaargs).
pub fn freecvdef(d: Option<Box<cvdef>>) {
    // c:2961
    let Some(mut node) = d else {
        return;
    }; // c:2961 if (d)
    node.descr = None; // c:2966 zsfree(d->descr)
    node.defs = None; // c:2967-2968 freearray(d->defs)
    let mut p = node.vals.take();
    while let Some(mut v) = p {
        // c:2970 for (p = d->vals; ...)
        p = v.next.take(); // c:2971 n = p->next
        v.name = None; // c:2972
        v.descr = None; // c:2973
        v.xor = None; // c:2974-2975
        freecaargs(v.arg.take()); // c:2976
        drop(v); // c:2977
    }
    drop(node); // c:2979
}

// `freecaargs(Caarg)` + `freecadef(Cadef)` ported above with the
// caarg/caopt/cadef struct ports (c:996 / c:1013).

#[cfg(test)]
mod cao_caa_tests {
    use super::*;

    #[test]
    fn cao_values_match_c_source() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:941-945 — sequential 1..=5.
        assert_eq!(CAO_NEXT, 1);
        assert_eq!(CAO_DIRECT, 2);
        assert_eq!(CAO_ODIRECT, 3);
        assert_eq!(CAO_EQUAL, 4);
        assert_eq!(CAO_OEQUAL, 5);
    }

    #[test]
    fn caa_values_match_c_source() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:964-968 — sequential 1..=5.
        assert_eq!(CAA_NORMAL, 1);
        assert_eq!(CAA_OPT, 2);
        assert_eq!(CAA_REST, 3);
        assert_eq!(CAA_RARGS, 4);
        assert_eq!(CAA_RREST, 5);
    }

    #[test]
    fn crt_values_match_c_source() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:79-83 — sequential 0..=4.
        assert_eq!(CRT_SIMPLE, 0);
        assert_eq!(CRT_DESC, 1);
        assert_eq!(CRT_SPEC, 2);
        assert_eq!(CRT_DUMMY, 3);
        assert_eq!(CRT_EXPL, 4);
    }

    #[test]
    fn cvv_values_match_c_source() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:2949-2951 — sequential 0..=2.
        assert_eq!(CVV_NOARG, 0);
        assert_eq!(CVV_ARG, 1);
        assert_eq!(CVV_OPT, 2);
    }

    #[test]
    fn cache_sizes_are_8() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:972 + c:2955 — both LRU caches are 8 entries.
        assert_eq!(MAX_CACACHE, 8);
        assert_eq!(MAX_CVCACHE, 8);
    }

    #[test]
    fn max_tags_is_256() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(MAX_TAGS, 256);
    }

    #[test]
    fn path_max2_is_8192() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(PATH_MAX2, 8192);
    }
}

/// Direct port of `static Cvdef parse_cvdef(char *nam, char **args)`
/// from `Src/Zle/computil.c:2986-3148`. Parses the leading
/// `-s SEP / -S SEP / -w` flag block, then the description, then
/// each value spec into a cvval chain.
pub fn parse_cvdef(nam: &str, args: &[String]) -> Option<Box<cvdef>> {
    // c:2986

    let orig_args = args;
    let mut idx = 0usize;

    let mut sep: i32 = 0; // c:2991 char sep = '\0'
    let mut asep: i32 = b'=' as i32; // c:2991 char asep = '='
    let mut hassep: i32 = 0; // c:2992
    let mut words: i32 = 0; // c:2992

    // c:2994-3010 — leading flag block (-s SEP, -S SEP, -w).
    while idx + 1 < args.len()
        && args[idx].len() == 2
        && args[idx].starts_with('-')
        && (args[idx].as_bytes()[1] == b's'
            || args[idx].as_bytes()[1] == b'S'
            || args[idx].as_bytes()[1] == b'w')
    {
        let flag = args[idx].as_bytes()[1];
        if flag == b's' {
            // c:2999
            hassep = 1;
            sep = args[idx + 1].as_bytes().first().copied().unwrap_or(0) as i32;
            idx += 2;
        } else if flag == b'S' {
            // c:3003
            asep = args[idx + 1].as_bytes().first().copied().unwrap_or(0) as i32;
            idx += 2;
        } else {
            // c:3006 -w
            words = 1;
            idx += 1;
        }
    }

    if idx + 1 >= args.len() {
        // c:3011
        zwarnnam(nam, "not enough arguments");
        return None;
    }
    let descr = args[idx].clone(); // c:3015 descr = *args++
    idx += 1;

    let mut ret = Box::new(cvdef {
        descr: Some(descr),             // c:3018
        hassep,                         // c:3019
        sep,                            // c:3020
        argsep: asep,                   // c:3021
        next: None,                     // c:3022
        vals: None,                     // c:3023
        defs: Some(orig_args.to_vec()), // c:3024
        ndefs: orig_args.len() as i32,  // c:3025
        lastt: {
            // c:3026
            use std::time::{SystemTime, UNIX_EPOCH};
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        },
        words, // c:3027
    });

    // c:3029-3147 — for each remaining arg, parse one value spec.
    let mut vals_collected: Vec<Box<cvval>> = Vec::new();

    while idx < args.len() {
        let spec = &args[idx];
        let bytes = spec.as_bytes();
        let mut p: usize = 0;
        let mut xnum: i32 = 0; // c:3032
        let mut bs = 0; // c:3030
        let mut xor: Option<Vec<String>> = None;

        // c:3035-3068 — `(opt1 opt2)` xor list.
        if p < bytes.len() && bytes[p] == b'(' {
            // c:3035
            let mut list: Vec<String> = Vec::new();
            let mut bad = false;
            'paren: loop {
                if p >= bytes.len() || bytes[p] == b')' {
                    break;
                }
                p += 1; // c:3041 p++
                while p < bytes.len() && inblank(bytes[p]) {
                    p += 1;
                }
                if p >= bytes.len() {
                    bad = true;
                    break 'paren;
                }
                if bytes[p] == b')' {
                    break 'paren;
                }
                let q = p;
                p += 1;
                while p < bytes.len() && bytes[p] != b')' && !inblank(bytes[p]) {
                    p += 1;
                }
                if p >= bytes.len() {
                    bad = true;
                    break 'paren;
                }
                let word = String::from_utf8_lossy(&bytes[q..p]).into_owned();
                list.push(word);
                xnum += 1;
            }
            if bad || p >= bytes.len() || bytes[p] != b')' {
                // c:3056
                zwarnnam(nam, &format!("invalid argument: {}", spec));
                return None;
            }
            xor = Some(list);
            p += 1; // c:3066
        }

        // c:3071 — `*` (multi).
        let multi = p < bytes.len() && bytes[p] == b'*';
        if multi {
            p += 1;
        }

        // c:3076 — scan option name up to `:` or `[`.
        let name_start = p;
        while p < bytes.len() && bytes[p] != b':' && bytes[p] != b'[' {
            // c:3076
            if bytes[p] == b'\\' && p + 1 < bytes.len() {
                p += 1;
                bs = 1; // c:3078
            }
            p += 1;
        }

        // c:3080-3085 — multi-letter check against empty separator.
        if hassep != 0 && sep == 0 && name_start + (bs as usize) + 1 < p {
            // c:3080
            zwarnnam(nam, "no multi-letter values with empty separator allowed");
            return None;
        }

        let name_bytes = &bytes[name_start..p];
        let name = String::from_utf8_lossy(name_bytes).into_owned();

        // c:3087 — optional [descr].
        let mut value_descr: Option<String> = None;
        let mut c_byte = if p < bytes.len() { bytes[p] } else { 0 };
        if c_byte == b'[' {
            // c:3088
            p += 1;
            let d_start = p;
            while p < bytes.len() && bytes[p] != b']' {
                // c:3090
                if bytes[p] == b'\\' && p + 1 < bytes.len() {
                    p += 1;
                }
                p += 1;
            }
            if p >= bytes.len() {
                // c:3094
                zwarnnam(nam, &format!("invalid value definition: {}", spec));
                return None;
            }
            value_descr = Some(String::from_utf8_lossy(&bytes[d_start..p]).into_owned());
            p += 1; // c:3100
            c_byte = if p < bytes.len() { bytes[p] } else { 0 };
        }

        if c_byte != 0 && c_byte != b':' {
            // c:3106
            zwarnnam(nam, &format!("invalid value definition: {}", spec));
            return None;
        }

        // c:3114 — :arg or ::optarg.
        let mut vtype = CVV_NOARG;
        let mut arg: Option<Box<caarg>> = None;
        if c_byte == b':' {
            // c:3114
            if hassep != 0 && sep == 0 {
                // c:3115
                zwarnnam(nam, "no value with argument with empty separator allowed");
                return None;
            }
            p += 1; // c:3121 *++p
            if p < bytes.len() && bytes[p] == b':' {
                // c:3121
                p += 1;
                vtype = CVV_OPT; // c:3123
            } else {
                vtype = CVV_ARG; // c:3125
            }
            arg = Some(parse_caarg(0, 0, 0, 0, Some(&name), bytes, &mut p, None));
            // c:3126
        }

        // c:3131-3137 — add own name to xor list when not multi.
        if !multi {
            // c:3131
            let xv = xor.get_or_insert_with(Vec::new);
            if xv.len() <= xnum as usize {
                xv.resize(xnum as usize + 1, String::new());
            }
            xv[xnum as usize] = name.clone(); // c:3136
        }

        let v = Box::new(cvval {
            // c:3138
            next: None,
            name: Some(name),   // c:3142
            descr: value_descr, // c:3143
            xor,                // c:3144
            r#type: vtype,      // c:3145
            arg,                // c:3146
            active: 0,
        });
        vals_collected.push(v);

        idx += 1;
    }

    // Link vals_collected as a chain.
    let mut head: Option<Box<cvval>> = None;
    for v in vals_collected.into_iter().rev() {
        let mut v = v;
        v.next = head;
        head = Some(v);
    }
    ret.vals = head;

    Some(ret)
}

/// Direct port of `static Cvdef get_cvdef(char *nam, char **args)` from
/// `Src/Zle/computil.c:3154-3173`. LRU lookup over `cvdef_cache`
/// keyed by the raw argv. On hit bumps `lastt` and returns 1. On
/// miss parses via `parse_cvdef` and evicts the entry with the
/// oldest `lastt` (or the first empty slot) for insertion.
pub fn get_cvdef(nam: &str, args: &[String]) -> i32 {
    // c:3154
    let na = args.len() as i32;
    let now = {
        // c:3161 time(0)
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };

    if let Ok(mut cache) = cvdef_cache.lock() {
        let mut min_idx: Option<usize> = None;
        let mut min_lastt: i64 = i64::MAX;
        let mut hit_idx: Option<usize> = None;
        for (i, slot) in cache.iter().enumerate() {
            // c:3159
            match slot {
                Some(entry) => {
                    if entry.ndefs == na                                     // c:3160
                        && entry.defs.as_deref()
                            .map_or(false, |d| d.len() == args.len()
                                && d.iter().zip(args.iter()).all(|(a, b)| a == b))
                    {
                        hit_idx = Some(i);
                        break;
                    }
                    if entry.lastt < min_lastt {
                        // c:3164
                        min_lastt = entry.lastt;
                        min_idx = Some(i);
                    }
                }
                None => {
                    // c:3164 empty slot
                    min_idx = Some(i);
                    break;
                }
            }
        }
        if let Some(i) = hit_idx {
            // c:3160
            if let Some(entry) = cache[i].as_mut() {
                entry.lastt = now; // c:3161
            }
            return 1; // c:3163 hit
        }
        // c:3168 — parse_cvdef; on success replace the chosen slot.
        if let Some(new) = parse_cvdef(nam, args) {
            let idx = min_idx.unwrap_or(0);
            cache[idx] = Some(new); // c:3170
        }
    }
    0 // c:3172 miss
}

/// Direct port of `static Cvval cv_get_val(Cvdef d, char *name)` from
/// `Src/Zle/computil.c:3178-3187`. Linear scan over `d->vals` for a
/// name match. Returns a shallow clone of the matched cvval.
pub fn cv_get_val(d: &cvdef, name: &str) -> Option<Box<cvval>> {
    // c:3178
    let mut p = d.vals.as_deref();
    while let Some(v) = p {
        // c:3182
        if v.name.as_deref() == Some(name) {
            // c:3183
            return Some(Box::new(cvval {
                next: None,
                name: v.name.clone(),
                descr: v.descr.clone(),
                xor: v.xor.clone(),
                r#type: v.r#type,
                arg: v.arg.clone(),
                active: v.active,
            }));
        }
        p = v.next.as_deref();
    }
    None // c:3186
}

/// Direct port of `static Cvval cv_quote_get_val(Cvdef d, char *name)`
/// from `Src/Zle/computil.c:3190-3204`. Unquotes `name` via the full
/// C chain: `parse_subst_string` (with noerrs=2 to suppress errors),
/// `remnulargs`, then `untokenize`; result fed to `cv_get_val`.
pub fn cv_quote_get_val(d: &cvdef, name: &str) -> Option<Box<cvval>> {
    // c:3190
    // c:3195 — `name = dupstring(name)` (Rust: own a mutable copy).
    let mut s = name.to_string();
    // c:3196-3199 — `ne = noerrs; noerrs = 2; parse_subst_string(name);
    //                noerrs = ne`. The parse_subst_string port (lex.rs:3797)
    // returns Result; we discard errors so noerrs=2/restore is a no-op.
    set_noerrs(2);
    let parsed = crate::ported::lex::parse_subst_string(&s).ok();
    set_noerrs(0);
    if let Some(p) = parsed {
        s = p;
    }
    // c:3200 — `remnulargs(name)`.
    remnulargs(&mut s);
    // c:3201 — `untokenize(name)`.
    let s = untokenize(&s);
    // c:3203 — `return cv_get_val(d, name)`.
    cv_get_val(d, &s)
}

/// Direct port of `static void cv_inactive(Cvdef d, char **xor)` from
/// `Src/Zle/computil.c:3209-3218`. Clears `active` on each cvval named
/// in the xor list.
pub fn cv_inactive(d: &mut cvdef, xor: &[String]) {
    // c:3209
    for name in xor {
        // c:3214
        let mut p = d.vals.as_deref_mut();
        while let Some(v) = p {
            if v.name.as_deref() == Some(name.as_str()) {
                v.active = 0; // c:3216
            }
            p = v.next.as_deref_mut();
        }
    }
}

// =====================================================================
// `cvstate` — `_values` parse state.
// Src/Zle/computil.c:3220-3231.
// =====================================================================

/// Direct port of `struct cvstate` from `Src/Zle/computil.c:3222-3227`.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct cvstate {
    // c:3222
    pub d: Option<Box<cvdef>>,     // c:3223 Cvdef d
    pub def: Option<Box<caarg>>,   // c:3224 Caarg def
    pub val: Option<Box<cvval>>,   // c:3225 Cvval val
    pub vals: Option<Vec<String>>, // c:3226 LinkList vals
}

/// Port of `static int cv_parsed` from `Src/Zle/computil.c:3230`.
pub static cv_parsed: std::sync::atomic::AtomicI32 = // c:3230
    std::sync::atomic::AtomicI32::new(0);

/// Direct port of `static Cvval cv_next(Cvdef d, char **sp, char **ap)`
/// from `Src/Zle/computil.c:3240-3331`. Splits the next value out of
/// `*sp` using `d->sep` / `d->argsep`, returns its matched Cvval.
/// On success, `*sp` advances past the consumed prefix; `*ap` is set
/// to the value's argument (if any) or `None`.
pub fn cv_next(
    d: &cvdef,
    sp: &mut Option<String>, // c:3240
    ap: &mut Option<String>,
) -> Option<Box<cvval>> {
    let s_in = sp.take().unwrap_or_default();
    if s_in.is_empty() {
        // c:3245
        *sp = None;
        *ap = None;
        return None;
    }
    let bytes = s_in.as_bytes();

    // Branch 1: hassep && !sep, or !argsep — greedy match (longest prefix).
    if (d.hassep != 0 && d.sep == 0) || d.argsep == 0 {
        // c:3250
        let ec_byte: u8 = if d.hassep != 0 && d.sep != 0 {
            d.sep as u8
        } else {
            d.argsep as u8
        };
        let mut s_idx: usize = 0;
        let mut r: Option<Box<cvval>> = None;
        // c:3255 — extend until cv_quote_get_val matches or hit ec.
        loop {
            s_idx += 1;
            if s_idx > bytes.len() {
                break;
            }
            let candidate = std::str::from_utf8(&bytes[..s_idx]).unwrap_or("");
            r = cv_quote_get_val(d, candidate);
            if r.is_some() {
                break;
            }
            if s_idx >= bytes.len() || bytes[s_idx] == ec_byte {
                break;
            }
        }
        let os = s_idx;
        // c:3268 — advance *sp.
        if d.hassep != 0 && d.sep != 0 {
            let sep_byte = d.sep as u8;
            if let Some(off) = bytes[s_idx.min(bytes.len())..]
                .iter()
                .position(|&b| b == sep_byte)
            {
                let after = s_idx + off + 1;
                *sp = Some(String::from_utf8_lossy(&bytes[after..]).into_owned());
            } else {
                *sp = None;
            }
        } else {
            *sp = if s_idx < bytes.len() {
                Some(String::from_utf8_lossy(&bytes[s_idx..]).into_owned())
            } else {
                None
            };
        }
        // c:3275 — set *ap.
        let argsep_b = d.argsep as u8;
        if d.argsep != 0 && os < bytes.len() && bytes[os] == argsep_b {
            *ap = Some(String::from_utf8_lossy(&bytes[os + 1..]).into_owned());
            *sp = None;
        } else if r.as_deref().map_or(false, |v| v.r#type != CVV_NOARG) {
            *ap = if os < bytes.len() {
                Some(String::from_utf8_lossy(&bytes[os..]).into_owned())
            } else {
                None
            };
        } else {
            *ap = None;
        }
        return r;
    }

    // Branch 2: hassep set (with both sep and argsep).
    if d.hassep != 0 {
        // c:3285
        let sep_b = d.sep as u8;
        let argsep_b = d.argsep as u8;
        let ns = bytes.iter().position(|&b| b == sep_b);
        let mut as_off: Option<usize> = None;
        let mut skip = false;
        if d.argsep != 0 {
            if let Some(a_pos) = bytes.iter().position(|&b| b == argsep_b) {
                if ns.map_or(true, |n| a_pos <= n) {
                    // c:3289
                    as_off = Some(a_pos);
                    *ap = Some(String::from_utf8_lossy(&bytes[a_pos + 1..]).into_owned());
                    skip = true;
                }
            }
        }
        let sap = as_off.or(ns);
        let head = match sap {
            Some(p) => std::str::from_utf8(&bytes[..p]).unwrap_or(""),
            None => std::str::from_utf8(&bytes).unwrap_or(""),
        };
        let r = cv_quote_get_val(d, head);
        // c:3302 — if NOARG/no match and skip, fall back to as.
        let ns_eff = if (r.as_deref().map_or(true, |v| v.r#type == CVV_NOARG)) && skip {
            as_off
        } else if skip {
            // skip is set ⇒ as_off was the chosen sep; ns might be after.
            ns.filter(|&n| as_off.map_or(true, |a| n > a))
        } else {
            ns
        };
        let next_off = match ns_eff {
            None => None,
            Some(n)
                if Some(n) == as_off && r.as_deref().map_or(false, |v| v.r#type != CVV_NOARG) =>
            {
                None
            }
            Some(n) => Some(n + 1),
        };
        *sp = next_off.map(|o| String::from_utf8_lossy(&bytes[o..]).into_owned());
        if !skip {
            *ap = None;
        }
        return r;
    }

    // Branch 3: no hassep, argsep set.
    *sp = None; // c:3314
    let argsep_b = d.argsep as u8;
    let as_pos = bytes.iter().position(|&b| b == argsep_b);
    let head = match as_pos {
        Some(p) => {
            *ap = Some(String::from_utf8_lossy(&bytes[p + 1..]).into_owned());
            std::str::from_utf8(&bytes[..p]).unwrap_or("")
        }
        None => {
            *ap = None;
            &s_in
        }
    };
    cv_quote_get_val(d, head) // c:3324
}

/// Direct port of `static void cv_parse_word(Cvdef d)` from
/// `Src/Zle/computil.c:3336-3472`. Walks `compwords[1..]` (when
/// `d->words` set) and `compprefix` calling `cv_next` repeatedly,
/// accumulating recognized values into `cv_laststate.vals`.
pub fn cv_parse_word(d: &mut cvdef) {
    // c:3336

    // c:3343 — free old vals.
    if cv_alloced.load(Ordering::Relaxed) != 0 {
        if let Ok(mut ls) = cv_laststate.lock() {
            ls.vals = None;
        }
    }
    // c:3346 — mark all vals active.
    let mut v = d.vals.as_deref_mut();
    while let Some(vv) = v {
        vv.active = 1;
        v = vv.next.as_deref_mut();
    }

    let mut state_vals: Vec<String> = Vec::new();
    let mut state_def: Option<Box<caarg>> = None;
    let mut state_val: Option<Box<cvval>> = None;
    cv_alloced.store(1, Ordering::Relaxed);

    let compcur = COMPCURRENT.load(Ordering::Relaxed);
    let compwords: Vec<String> = COMPWORDS
        .get()
        .and_then(|m| m.lock().ok().map(|w| w.clone()))
        .unwrap_or_default();
    let compprefix: String = COMPPREFIX
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();
    let compsuffix: String = COMPSUFFIX
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();
    let mut pign = compprefix.clone(); // c:3340
    let mut nosfx = false;

    // c:3356 — scan compwords[1..] if d.words is set.
    if d.words != 0 && !compwords.is_empty() && !compwords[0].is_empty() {
        for i in 1..compwords.len() {
            if (i as i32) == compcur - 1 {
                continue;
            }
            let mut str_opt: Option<String> = Some(compwords[i].clone());
            while str_opt.as_deref().map_or(false, |s| !s.is_empty()) {
                let mut ap: Option<String> = None;
                let val = cv_next(d, &mut str_opt, &mut ap);
                if let Some(v) = val {
                    state_vals.push(v.name.clone().unwrap_or_default());
                    state_vals.push(ap.unwrap_or_default());
                    if (i as i32) + 1 < compcur {
                        let xor = v.xor.clone().unwrap_or_default();
                        cv_inactive(d, &xor);
                    }
                } else {
                    break;
                }
            }
        }
    }

    // c:3385 — scan compprefix.
    let mut str_opt: Option<String> = Some(compprefix.clone());
    let mut last_arg: Option<String> = None;
    while str_opt.as_deref().map_or(false, |s| !s.is_empty()) {
        let mut ap: Option<String> = None;
        let val = cv_next(d, &mut str_opt, &mut ap);
        if let Some(v) = val {
            state_vals.push(v.name.clone().unwrap_or_default());
            match ap.as_deref() {
                Some(arg_v) => {
                    if str_opt.is_some() {
                        state_vals.push(arg_v.to_string());
                    } else {
                        let joined = format!("{}{}", arg_v, compsuffix);
                        state_vals.push(joined);
                        nosfx = true;
                    }
                    last_arg = ap.clone();
                }
                None => state_vals.push(String::new()),
            }
            let xor = v.xor.clone().unwrap_or_default();
            cv_inactive(d, &xor);
            if let Some(s) = str_opt.as_deref() {
                pign = s.to_string();
            } else {
                // c:3407 — re-activate v in the cvdef.
                let target_name = v.name.clone();
                let mut p = d.vals.as_deref_mut();
                while let Some(vv) = p {
                    if vv.name == target_name {
                        vv.active = 1;
                        break;
                    }
                    p = vv.next.as_deref_mut();
                }
            }
            state_val = Some(v);
        } else {
            break;
        }
    }
    if state_val.is_some() && last_arg.is_some() && str_opt.is_none() {
        // c:3411
        state_def = state_val.as_ref().and_then(|v| v.arg.clone());
    }

    // c:3414 — separator handling for compsuffix.
    if !nosfx && d.hassep != 0 {
        let pign_len = pign.len();
        let cp_len = compprefix.len();
        ignore_prefix(cp_len as i32 - pign_len as i32); // c:3418

        let mut ign = 0usize;
        let mut more: Option<String> = None;
        if d.sep == 0
            && (state_val.is_none() || state_val.as_deref().map_or(true, |v| v.r#type == CVV_NOARG))
        {
            ign = compsuffix.len();
            more = Some(compsuffix.clone());
        } else if d.sep != 0 {
            let sep_b = d.sep as u8;
            let ns_pos = compsuffix.as_bytes().iter().position(|&b| b == sep_b);
            let as_pos = if d.argsep != 0 {
                compsuffix
                    .as_bytes()
                    .iter()
                    .position(|&b| b == d.argsep as u8)
            } else {
                None
            };
            if let Some(a) = as_pos {
                if ns_pos.map_or(true, |n| a <= n) {
                    ign = compsuffix.len() - a;
                } else {
                    ign = ns_pos.map_or(0, |n| compsuffix.len() - n);
                }
            } else {
                ign = ns_pos.map_or(0, |n| compsuffix.len() - n);
            }
            more = ns_pos.map(|n| compsuffix[n + 1..].to_string());
        } else if d.argsep != 0 {
            let as_pos = compsuffix
                .as_bytes()
                .iter()
                .position(|&b| b == d.argsep as u8);
            if let Some(a) = as_pos {
                ign = compsuffix.len() - a;
            }
        }

        if ign > 0 {
            ignore_suffix(ign as i32); // c:3444
        }

        let mut more_opt = more;
        while more_opt.as_deref().map_or(false, |s| !s.is_empty()) {
            // c:3446
            let mut ap: Option<String> = None;
            let val = cv_next(d, &mut more_opt, &mut ap);
            if let Some(v) = val {
                state_vals.push(v.name.clone().unwrap_or_default());
                match ap.as_deref() {
                    Some(arg_v) => {
                        if more_opt.is_some() {
                            state_vals.push(arg_v.to_string());
                        } else {
                            state_vals.push(format!("{}{}", arg_v, compsuffix));
                        }
                    }
                    None => state_vals.push(String::new()),
                }
                let xor = v.xor.clone().unwrap_or_default();
                cv_inactive(d, &xor);
            } else {
                break;
            }
        }
    } else if last_arg.is_some() {
        let cp_len = compprefix.len();
        let arg_off = compprefix
            .find(last_arg.as_deref().unwrap_or(""))
            .map(|i| i as i32)
            .unwrap_or(cp_len as i32);
        ignore_prefix(arg_off); // c:3467
    } else {
        let cp_len = compprefix.len();
        ignore_prefix(cp_len as i32 - pign.len() as i32); // c:3469
    }

    // c:3471 — commit state.
    if let Ok(mut ls) = cv_laststate.lock() {
        *ls = cvstate {
            d: Some(Box::new(d.clone())),
            def: state_def,
            val: state_val,
            vals: if state_vals.is_empty() {
                None
            } else {
                Some(state_vals)
            },
        };
    }
}

/// Direct port of `static int bin_compvalues(char *nam, char **args,
///                                              UNUSED(Options ops),
///                                              UNUSED(int func))` from
/// `Src/Zle/computil.c:3475-3658`. Full subcommand dispatch for
/// `compvalues -i/-D/-C/-V/-s/-S/-d/-L/-v`. Each branch consumes
/// `cv_laststate` populated by `cv_parse_word`.
pub fn bin_compvalues(
    nam: &str,
    args: &[String], // c:3475
    _ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:3479
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {
        return 1;
    }
    let a0 = args[0].as_bytes();
    if a0.len() != 2 || a0[0] != b'-' {
        // c:3483
        zwarnnam(nam, &format!("invalid argument: {}", args[0]));
        return 1;
    }
    let sub = a0[1];

    if sub != b'i' && cv_parsed.load(Ordering::Relaxed) == 0 {
        // c:3487
        zwarnnam(nam, "no parsed state");
        return 1;
    }

    let (min, max): (i32, i32) = match sub {
        // c:3491
        b'i' => (2, -1),
        b'D' => (2, 2),
        b'C' => (1, 1),
        b'V' => (3, 3),
        b's' => (1, 1),
        b'S' => (1, 1),
        b'd' => (1, 1),
        b'L' => (3, 4),
        b'v' => (1, 1),
        _ => {
            zwarnnam(nam, &format!("invalid option: {}", args[0]));
            return 1;
        }
    };
    let n = (args.len() as i32) - 1;
    if n < min {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    if max >= 0 && n > max {
        zwarnnam(nam, "too many arguments");
        return 1;
    }

    match sub {
        b'i' => {
            // c:3514
            let spec = &args[1..];
            let _ = get_cvdef(nam, spec);
            let mut cached: Option<Box<cvdef>> = {
                let cache = cvdef_cache.lock().ok();
                cache.and_then(|c| {
                    c.iter().find_map(|slot| {
                        slot.as_ref()
                            .filter(|e| {
                                e.ndefs == spec.len() as i32
                                    && e.defs.as_deref().map_or(false, |d| {
                                        d.len() == spec.len()
                                            && d.iter().zip(spec.iter()).all(|(a, b)| a == b)
                                    })
                            })
                            .cloned()
                    })
                })
            };
            let Some(ref mut def) = cached else {
                return 1;
            };
            cv_parsed.store(0, Ordering::Relaxed); // c:3521
            cv_parse_word(def); // c:3527
            cv_parsed.store(1, Ordering::Relaxed); // c:3528
            0
        }

        b'D' => {
            // c:3533
            let arg = cv_laststate.lock().ok().and_then(|s| s.def.clone());
            if let Some(a) = arg {
                setsparam(&args[1], a.descr.as_deref().unwrap_or("")); // c:3541
                setsparam(&args[2], a.action.as_deref().unwrap_or("")); // c:3542
                0
            } else {
                1 // c:3546
            }
        }

        b'C' => {
            // c:3548
            let arg = cv_laststate.lock().ok().and_then(|s| s.def.clone());
            if let Some(a) = arg {
                setsparam(&args[1], a.opt.as_deref().unwrap_or("")); // c:3555
                0
            } else {
                1 // c:3559
            }
        }

        b'V' => {
            // c:3561
            let mut noarg: Vec<String> = Vec::new();
            let mut arg_l: Vec<String> = Vec::new();
            let mut opt_l: Vec<String> = Vec::new();
            if let Ok(ls) = cv_laststate.lock() {
                if let Some(d) = ls.d.as_ref() {
                    let mut p = d.vals.as_deref();
                    while let Some(v) = p {
                        if v.active != 0 {
                            // c:3574
                            let bucket: &mut Vec<String> = match v.r#type {
                                t if t == CVV_NOARG => &mut noarg,
                                t if t == CVV_ARG => &mut arg_l,
                                _ => &mut opt_l,
                            };
                            let name = v.name.as_deref().unwrap_or("");
                            let str_val = if let Some(d) = v.descr.as_deref() {
                                format!("{}:{}", name, d)
                            } else {
                                name.to_string()
                            };
                            bucket.push(str_val); // c:3589
                        }
                        p = v.next.as_deref();
                    }
                }
            }
            setaparam(&args[1], noarg);
            setaparam(&args[2], arg_l);
            setaparam(&args[3], opt_l);
            0 // c:3596
        }

        b's' => {
            // c:3598
            let (hassep, sep) = cv_laststate
                .lock()
                .ok()
                .and_then(|ls| ls.d.as_ref().map(|d| (d.hassep, d.sep)))
                .unwrap_or((0, 0));
            if hassep != 0 {
                let tmp = (sep as u8 as char).to_string();
                setsparam(&args[1], &tmp);
                0 // c:3608
            } else {
                1 // c:3610
            }
        }

        b'S' => {
            // c:3611
            let argsep = cv_laststate
                .lock()
                .ok()
                .and_then(|ls| ls.d.as_ref().map(|d| d.argsep))
                .unwrap_or(0);
            let tmp = (argsep as u8 as char).to_string();
            setsparam(&args[1], &tmp);
            0 // c:3620
        }

        b'd' => {
            // c:3621
            let descr = cv_laststate
                .lock()
                .ok()
                .and_then(|ls| ls.d.as_ref().and_then(|d| d.descr.clone()))
                .unwrap_or_default();
            setsparam(&args[1], &descr);
            0
        }

        b'L' => {
            // c:3626
            let val = cv_laststate
                .lock()
                .ok()
                .and_then(|ls| ls.d.as_ref().and_then(|d| cv_get_val(d, &args[1])));
            if let Some(v) = val {
                if let Some(a) = v.arg.as_deref() {
                    // c:3634
                    setsparam(&args[2], a.descr.as_deref().unwrap_or(""));
                    setsparam(&args[3], a.action.as_deref().unwrap_or(""));
                    if args.len() > 4 {
                        // c:3638
                        setsparam(&args[4], v.name.as_deref().unwrap_or(""));
                    }
                    return 0;
                }
            }
            1 // c:3643
        }

        b'v' => {
            // c:3645
            let vals = cv_laststate.lock().ok().and_then(|ls| ls.vals.clone());
            if let Some(v) = vals {
                sethparam(&args[1], v);
                0
            } else {
                1 // c:3656
            }
        }

        _ => 1, // c:3658
    }
}

/// Port of `comp_quote(char *str, int prefix)` from Src/Zle/computil.c:3662.
pub fn comp_quote(str: &str, prefix: i32) -> String {
    // c:3662
    // c:3667 — `x = (prefix && *str == '=')`.
    let (s_eff, x) = if prefix != 0 && str.starts_with('=') {
        // c:3667
        ("x".to_string() + &str[1..], true) // c:3668
    } else {
        (str.to_string(), false)
    };
    // c:3670 — `ret = quotestring(str, *compqstack)`.
    //          *compqstack is the first byte of the qstack string.
    let qhead = COMPQSTACK
        .get()
        .and_then(|m| m.lock().ok().and_then(|str| str.bytes().next()))
        .unwrap_or(0);
    let mut ret = quotename(&s_eff, qhead as i32);
    // c:3672-3673 — restore `=` prefix on both ret and original.
    if x {
        if !ret.is_empty() {
            ret.replace_range(0..1, "=");
        }
    }
    ret
}

// `setup_` is ported above with the cadef_cache/cvdef_cache/comptags
// reset body cited at Src/Zle/computil.c:5124. This duplicate shim
// was retired when the real port landed.

// =====================================================================
// bin_compquote / bin_comptags / bin_comptry / bin_compvalues —
// Src/Zle/computil.c. Each is a real port matching the C signature
// exactly; state mutations go through the canonical
// getvalue/setstrvalue/setarrvalue ops in params.rs, the comptags
// state machine in the cs_* helpers below, and the compvalues table
// via cv_parse_word.
// =====================================================================

/// Direct port of `bin_compquote(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Zle/computil.c:3679`.
/// C body (c:3683-3725):
/// ```c
/// if (incompfunc != 1) { error; return 1; }
/// if (!compqstack || !*compqstack) return 0;
/// while ((name = *args++)) {
///     if ((v = getvalue(...))) {
///         switch (PM_TYPE(v->pm->node.flags)) {
///         case PM_SCALAR/NAMEREF:
///             setstrvalue(v, comp_quote(getstrvalue(v), -p));
///         case PM_ARRAY:
///             foreach val in array: comp_quote each
///         default: zwarnnam("invalid parameter type");
///         }
///     }
/// }
/// ```
/// Quoting routes through `comp_quote()` per param type (PM_SCALAR
/// / PM_ARRAY); the entry validates `incompfunc` + `compqstack`
/// guards before dispatch.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_compquote(
    nam: &str,
    args: &[String], // c:3679
    ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:3685
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    // c:3691 — `if (!compqstack || !*compqstack) return 0;`.
    let qstack_empty = COMPQSTACK
        .get()
        .map(|m| m.lock().map(|s| s.is_empty()).unwrap_or(true))
        .unwrap_or(true);
    if qstack_empty {
        return 0;
    }
    let p_flag = OPT_ISSET(ops, b'p'); // c:3704

    // c:3696 — `while ((name = *args++))`. Walk param names.
    for name in args {
        // c:3696
        let mut vbuf = value {
            pm: None,
            arr: Vec::new(),
            scanflags: 0,
            valflags: 0,
            start: 0,
            end: 0,
        };
        let mut nameref: &str = name.as_str();
        let v = getvalue(Some(&mut vbuf), &mut nameref, 0); // c:3699
        if v.is_none() {
            // c:3724
            zwarnnam(nam, &format!("unknown parameter: {}", name));
            continue;
        }
        let v = v.unwrap();
        let flags = v.pm.as_ref().map(|pm| pm.node.flags).unwrap_or(0);
        let pm_type = PM_TYPE(flags as u32);
        // c:3700-3705 — PM_SCALAR / PM_NAMEREF path.
        if pm_type == 0 || (flags as u32 & crate::ported::zsh_h::PM_NAMEREF) != 0 {
            let s = getstrvalue(Some(v));
            let q = comp_quote(&s, p_flag as i32);
            let mut nameref_re: &str = name.as_str();
            setstrvalue(getvalue(Some(&mut vbuf), &mut nameref_re, 0), &q);
        } else if pm_type == PM_ARRAY {
            // c:3706
            let arr = getvaluearr(Some(v));
            let new_arr: Vec<String> = arr
                .into_iter()
                .map(|elem| comp_quote(&elem, p_flag as i32))
                .collect();
            // Re-fetch a fresh value for the setarrvalue call (getvalue
            // consumed the prior borrow).
            let mut vbuf2 = value {
                pm: None,
                arr: Vec::new(),
                scanflags: 0,
                valflags: 0,
                start: 0,
                end: 0,
            };
            let mut nameref2: &str = name.as_str();
            if let Some(v2) = getvalue(Some(&mut vbuf2), &mut nameref2, 0) {
                setarrvalue(v2, new_arr);
            }
        } else {
            // c:3720
            zwarnnam(nam, &format!("invalid parameter type: {}", name));
        }
    }
    0 // c:3725
}

// =====================================================================
// `ctags` / `ctset` — `comptags` cache.
// Src/Zle/computil.c:3732-3760. MAX_TAGS already declared above.
// =====================================================================

/// Port of `typedef struct ctags *Ctags` from `Src/Zle/computil.c:3732`.
pub type Ctags = Box<ctags>; // c:3732

/// Direct port of `struct ctags` from `Src/Zle/computil.c:3737-3742`.
/// A bunch of tag sets keyed by locallevel.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct ctags {
    // c:3737
    pub all: Option<Vec<String>>, // c:3738 char **all
    pub context: Option<String>,  // c:3739 char *context
    pub init: i32,                // c:3740
    pub sets: Option<Box<ctset>>, // c:3741 Ctset sets
}
/// Port of `typedef struct ctset *Ctset` from `computil.c:3733`.
pub type Ctset = Box<ctset>; // c:3733

/// Direct port of `struct ctset` from `Src/Zle/computil.c:3763`.
#[derive(Debug, Default)]
#[allow(non_camel_case_types)]
pub struct ctset {
    // c:3763
    pub next: Option<Box<ctset>>,  // c:3763 Ctset next
    pub tags: Option<Vec<String>>, // c:3763 char **tags
    pub tag: Option<String>,       // c:3763 char *tag
    pub ptr: i32,                  // c:3763 char **ptr (index)
}

/// Port of `MAX_TAGS` from `computil.c:3755`. Maximum nested completion
/// tags depth.
pub const MAX_TAGS: usize = 256; // c:3755

/// Port of `static Ctags comptags[MAX_TAGS]` from
/// `Src/Zle/computil.c:3756`. One ctags entry per `locallevel`;
/// indexed by completion level.
pub static comptags: std::sync::Mutex<[Option<Box<ctags>>; MAX_TAGS]> = // c:3756
    std::sync::Mutex::new([const { None }; MAX_TAGS]);

/// Port of `static int lasttaglevel` from `Src/Zle/computil.c:3760`.
/// "locallevel at last comptags -i".
pub static lasttaglevel: std::sync::atomic::AtomicI32 = // c:3760
    std::sync::atomic::AtomicI32::new(0);

/// Direct port of `static void freectset(Ctset s)` from
/// `Src/Zle/computil.c:3780`.
pub fn freectset(mut s: Option<Box<ctset>>) {
    // c:3763
    while let Some(mut node) = s {
        // c:3780 while (s)
        s = node.next.take(); // c:3780 n = s->next
        node.tags = None; // c:3780-3771
        node.tag = None; // c:3780
        drop(node); // c:3780
    }
}

/// Direct port of `static void freectags(Ctags t)` from
/// `Src/Zle/computil.c:3780`.
pub fn freectags(t: Option<Box<ctags>>) {
    // c:3780
    let Some(mut node) = t else {
        return;
    }; // c:3780 if (t)
    node.all = None; // c:3783-3784
    node.context = None; // c:3785
    freectset(node.sets.take()); // c:3786
    drop(node); // c:3787
}

/// Direct port of `static void settags(int level, char **tags)` from
/// `Src/Zle/computil.c:3794`. Replaces `comptags[level]` with a fresh
/// ctags carrying `tags[0]` as context and `tags[1..]` as the full
/// tag-list. Used at the start of every completion level transition
/// (`comptags -i`).
pub fn settags(level: i32, tags: &[String]) {
    // c:3794
    let idx = level as usize;
    if idx >= MAX_TAGS {
        return;
    } // c:3756 bounds

    if let Ok(mut tab) = comptags.lock() {
        if tab[idx].is_some() {
            // c:3798
            freectags(tab[idx].take()); // c:3799
        }
        let context = tags.first().cloned(); // c:3804 *tags
        let all: Vec<String> = tags.iter().skip(1).cloned().collect(); // c:3803 tags+1
        tab[idx] = Some(Box::new(ctags {
            // c:3801 zalloc
            all: Some(all), // c:3803
            context,        // c:3804
            init: 1,        // c:3806
            sets: None,     // c:3805
        }));
    }
}

/// Port of `arrcontains(char **a, char *s, int colon)` from Src/Zle/computil.c:3813.
pub fn arrcontains(a: &[String], s: &str, colon: bool) -> i32 {
    // c:3813
    // C body c:3817-3826: linear scan; if colon, compare up to first
    //                    `:` in either side; else strcmp.
    for entry in a {
        if colon {
            let p = s.split(':').next().unwrap_or(s);
            let q = entry.split(':').next().unwrap_or(entry);
            if p == q {
                return 1; // c:3823
            }
        } else if entry == s {
            return 1; // c:3825
        }
    }
    0 // c:3827
}

/// Direct port of `static int bin_comptags(char *nam, char **args,
///                                          UNUSED(Options ops),
///                                          UNUSED(int func))` from
/// `Src/Zle/computil.c:3831-3958`. Full subcommand dispatch for
/// `comptags -i/-C/-T/-N/-R/-S/-A`. Reads `locallevel` to index
/// `comptags[]`; `--` suffix decrements the level by one.
pub fn bin_comptags(
    nam: &str,
    args: &[String], // c:3831
    _ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:3835
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {
        return 1;
    }
    let a0 = args[0].as_bytes();
    // c:3839 — validate `-X` or `-X--` shape.
    if a0.len() < 2 || a0[0] != b'-' || (a0.len() > 2 && (a0[2] != b'-' || a0.len() > 3)) {
        zwarnnam(nam, &format!("invalid argument: {}", args[0]));
        return 1;
    }

    let level: i32 = locallevel.load(Ordering::Relaxed)                      // c:3844
        - if a0.len() > 2 { 1 } else { 0 };
    if level < 0 || (level as usize) >= MAX_TAGS {
        // c:3845
        zwarnnam(nam, "nesting level too deep");
        return 1;
    }
    let lvl_idx = level as usize;

    let sub = a0[1];

    // c:3849 — non-init subcommands require a registered comptags[level].
    if sub != b'i' && sub != b'I' {
        let registered = {
            let tab = comptags.lock().unwrap();
            tab[lvl_idx].is_some()
        };
        if !registered {
            zwarnnam(nam, "no tags registered");
            return 1;
        }
    }

    // c:3854-3864 — per-subcommand arg-count bounds.
    let (min, max): (i32, i32) = match sub {
        b'i' => (2, -1),
        b'C' => (1, 1),
        b'T' => (0, 0),
        b'N' => (0, 0),
        b'R' => (1, 1),
        b'S' => (1, 1),
        b'A' => (2, 3),
        _ => {
            zwarnnam(nam, &format!("invalid option: {}", args[0]));
            return 1;
        }
    };
    let n = (args.len() as i32) - 1;
    if n < min {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    if max >= 0 && n > max {
        zwarnnam(nam, "too many arguments");
        return 1;
    }

    match sub {
        b'i' => {
            // c:3874
            settags(level, &args[1..]);
            lasttaglevel.store(level, Ordering::Relaxed); // c:3876
            0
        }
        b'C' => {
            // c:3878
            let ctx = {
                let tab = comptags.lock().unwrap();
                tab[lvl_idx]
                    .as_ref()
                    .and_then(|t| t.context.clone())
                    .unwrap_or_default()
            };
            setsparam(&args[1], &ctx); // c:3879
            0
        }
        b'T' => {
            // c:3881
            let empty = {
                let tab = comptags.lock().unwrap();
                tab[lvl_idx].as_ref().map_or(true, |t| t.sets.is_none())
            };
            if empty {
                1
            } else {
                0
            } // c:3882
        }
        b'N' => {
            // c:3883
            let mut tab = comptags.lock().unwrap();
            if let Some(t) = tab[lvl_idx].as_mut() {
                if t.init != 0 {
                    // c:3887
                    t.init = 0;
                } else if let Some(mut s) = t.sets.take() {
                    // c:3889
                    t.sets = s.next.take(); // c:3890
                    freectset(Some(s)); // c:3892
                }
                if t.sets.is_some() {
                    0
                } else {
                    1
                } // c:3894
            } else {
                1
            }
        }
        b'R' => {
            // c:3896
            let tab = comptags.lock().unwrap();
            let hit = tab[lvl_idx]
                .as_ref()
                .and_then(|t| t.sets.as_ref())
                .map(|s| {
                    s.tags
                        .as_deref()
                        .map_or(false, |tgs| arrcontains(tgs, &args[1], true) != 0)
                })
                .unwrap_or(false);
            if hit {
                0
            } else {
                1
            } // c:3900
        }
        b'A' => {
            // c:3903
            let mut tab = comptags.lock().unwrap();
            let Some(t) = tab[lvl_idx].as_mut() else {
                return 1;
            };
            let Some(s) = t.sets.as_mut() else {
                return 1;
            };
            // c:3911 — refresh ptr if tag changed.
            if s.tag.as_deref() != Some(args[1].as_str()) {
                s.tag = Some(args[1].clone()); // c:3913
                s.ptr = 0; // c:3914
            }
            let tags_vec = s.tags.clone().unwrap_or_default();
            // c:3916-3925 — walk tags from ptr looking for a name match.
            let mut found: Option<(usize, String, String)> = None;
            for (i, q) in tags_vec.iter().enumerate().skip(s.ptr as usize) {
                if strpfx(&args[1], q) {
                    // c:3917
                    let l = args[1].len();
                    let qb = q.as_bytes();
                    if qb.len() == l {
                        // c:3918
                        found = Some((i, q.clone(), q.clone()));
                        break;
                    } else if qb.len() > l && qb[l] == b':' {
                        // c:3921
                        let v = String::from_utf8_lossy(&qb[l + 1..]).into_owned();
                        found = Some((i, q.clone(), v));
                        break;
                    }
                }
            }
            let (q_idx, q_full, v) = match found {
                None => {
                    // c:3927
                    s.tag = None;
                    return 1;
                }
                Some(t) => t,
            };
            s.ptr = (q_idx + 1) as i32; // c:3932
                                        // c:3933 — `setsparam(args[2], v == '-' ? dyncat(args[1], v) : v)`.
            let value = if v.starts_with('-') {
                crate::ported::string::dyncat(&args[1], &v)
            } else {
                v.clone()
            };
            setsparam(&args[2], &value);
            // c:3934 — optional 3rd arg gets the "name-up-to-`:`" of q.
            if args.len() > 3 {
                let pre_colon: String = q_full.splitn(2, ':').next().unwrap_or("").to_string();
                setsparam(&args[3], &pre_colon);
            }
            0 // c:3942
        }
        b'S' => {
            // c:3946
            let tab = comptags.lock().unwrap();
            if let Some(tags) = tab[lvl_idx]
                .as_ref()
                .and_then(|t| t.sets.as_ref())
                .and_then(|s| s.tags.clone())
            {
                setaparam(&args[1], tags); // c:3951
                0
            } else {
                1 // c:3952
            }
        }
        _ => 0,
    }
}

/// Direct port of `static int bin_comptry(char *nam, char **args,
///                                          UNUSED(Options ops),
///                                          UNUSED(int func))` from
/// `Src/Zle/computil.c:3961-4138`. Builds a new tag-set under the
/// active comptags[lasttaglevel] entry. Two forms:
///   - `comptry -m "pat1 pat2" [...]` — for each space-separated
///     tag pattern, glob-expand via braces/wildcards, match against
///     the registered `all` array, filter out tags already in any
///     existing set, then append the deduplicated matches as a new
///     ctset.
///   - `comptry [-s] tag1 tag2 ...` — filter args to keep only
///     registered tags not in any existing set; with `-s`, build one
///     ctset per arg, else one ctset for all of them.
pub fn bin_comptry(
    nam: &str,
    args: &[String], // c:3961
    _ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:3963
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }

    let lvl = lasttaglevel.load(Ordering::Relaxed);
    if lvl <= 0 {
        // c:3967 — !lasttaglevel
        zwarnnam(nam, "no tags registered");
        return 1;
    }
    let lvl_idx = lvl as usize;
    let registered = comptags
        .lock()
        .ok()
        .map(|t| lvl_idx < t.len() && t[lvl_idx].is_some())
        .unwrap_or(false);
    if !registered {
        zwarnnam(nam, "no tags registered");
        return 1;
    }

    if args.is_empty() {
        return 0;
    } // c:3971

    // Helper: append a new ctset to comptags[lvl_idx].sets.
    let append_set = |tags: Vec<String>| {
        if let Ok(mut tab) = comptags.lock() {
            if let Some(t) = tab[lvl_idx].as_mut() {
                let new_set = Box::new(ctset {
                    next: None,
                    tags: Some(tags),
                    tag: None,
                    ptr: 0,
                });
                // c:4082 — walk to tail of existing sets.
                if let Some(head) = t.sets.as_mut() {
                    let mut cur = head.as_mut();
                    while cur.next.is_some() {
                        cur = cur.next.as_mut().unwrap();
                    }
                    cur.next = Some(new_set);
                } else {
                    t.sets = Some(new_set);
                }
            }
        }
    };

    if args[0] == "-m" {
        // c:3972
        // c:3973-4090 — pattern-match mode.
        for arg in &args[1..] {
            // c:3978
            let mut s = arg.as_bytes().to_vec();
            let mut list: Vec<String> = Vec::new();
            let mut num = 0i32;
            let mut i = 0usize;

            while i < s.len() {
                // c:3980 — skip leading blanks.
                while i < s.len() && iblank(s[i]) {
                    i += 1;
                }
                if i >= s.len() {
                    break;
                }
                // c:3982 — accumulate the token, watching for `\X` escape
                // and tracking the first unescaped ':' separator.
                let p_start = i;
                let mut p_pos = i;
                let mut colon_at: Option<usize> = None;
                while i < s.len() && !inblank(s[i]) {
                    if colon_at.is_none() && s[i] == b':' {
                        colon_at = Some(p_pos);
                    }
                    if s[i] == b'\\' && i + 1 < s.len() {
                        i += 1;
                    }
                    s[p_pos] = s[i];
                    p_pos += 1;
                    i += 1;
                }
                // Skip the trailing blank.
                if i < s.len() {
                    i += 1;
                }

                let token_full = String::from_utf8_lossy(&s[p_start..p_pos]).into_owned();
                if token_full.is_empty() {
                    continue;
                }

                // c:3997 — split at colon: q = head, c = trailing.
                let (q, c_opt): (String, Option<String>) = match colon_at {
                    Some(c_idx) => {
                        let head = String::from_utf8_lossy(&s[p_start..c_idx]).into_owned();
                        let tail = String::from_utf8_lossy(&s[c_idx + 1..p_pos]).into_owned();
                        (head, Some(tail))
                    }
                    None => (token_full.clone(), None),
                };
                if q.is_empty() {
                    continue;
                }

                // c:4001-4012 — convert `{` / `}` / `,` to Inbrace / Outbrace
                // / Comma tokens for glob processing.
                let mut qq: String = q
                    .chars()
                    .map(|ch| match ch {
                        '\\' => ch, // keep; handled below
                        '{' => Inbrace,
                        '}' => Outbrace,
                        ',' => Comma,
                        other => other,
                    })
                    .collect();
                // Handle `\X` — keep both bytes literal by re-walking. The
                // C does this inline in the same loop; we just leave them.
                tokenize(&mut qq);

                // c:4013 — if hasbraces/haswilds, glob-expand.
                let has_meta = hasbraces(&qq, false) || haswilds(&qq);
                let all_arr: Vec<String> = comptags
                    .lock()
                    .ok()
                    .and_then(|t| t[lvl_idx].as_ref().and_then(|c| c.all.clone()))
                    .unwrap_or_default();
                let sets_clone: Vec<Vec<String>> = comptags
                    .lock()
                    .ok()
                    .map(|t| {
                        let mut out = Vec::new();
                        if let Some(c) = t[lvl_idx].as_ref() {
                            let mut p = c.sets.as_deref();
                            while let Some(set) = p {
                                if let Some(ts) = set.tags.as_ref() {
                                    out.push(ts.clone());
                                }
                                p = set.next.as_deref();
                            }
                        }
                        out
                    })
                    .unwrap_or_default();

                if has_meta {
                    // c:4015-4022 — expand braces, then compile each as a Patprog.
                    let mut blist: Vec<String> = vec![qq.clone()];
                    let mut bi = 0usize;
                    while bi < blist.len() {
                        if hasbraces(&blist[bi], false) {
                            let expanded = xpandbraces(&blist[bi], false);
                            blist.remove(bi);
                            for e in expanded {
                                blist.insert(bi, e);
                                bi += 1;
                            }
                        } else {
                            bi += 1;
                        }
                    }
                    for bb in &blist {
                        // c:4023
                        if let Some(prog) = patcompile(bb, 0, None::<&mut String>) {
                            for a in &all_arr {
                                // c:4029
                                // Skip if `a:c` (or just a) already in list.
                                let already = list.iter().any(|item| {
                                    let item_head = item.split(':').next().unwrap_or(item);
                                    item_head == a.as_str()
                                });
                                if already {
                                    continue;
                                }
                                if pattry(&prog, a) {
                                    // c:4043
                                    let entry = match &c_opt {
                                        Some(c) => format!("{}:{}", a, c),
                                        None => a.clone(),
                                    };
                                    list.push(entry);
                                    num += 1;
                                }
                            }
                        }
                    }
                } else if arrcontains(&all_arr, &q, false) != 0 {
                    // c:4056
                    // c:4057-4064 — literal token: include if not in any set.
                    let in_set = sets_clone.iter().any(|s| arrcontains(s, &q, false) != 0);
                    if !in_set {
                        list.push(q.clone());
                        num += 1;
                    }
                }
            }

            if num > 0 {
                // c:4072
                append_set(list);
            }
        }
    } else {
        // c:4091 — plain mode
        let mut idx = 0usize;
        let sep = args[idx] == "-s"; // c:4095
        if sep {
            idx += 1;
        }
        let all_arr: Vec<String> = comptags
            .lock()
            .ok()
            .and_then(|t| t[lvl_idx].as_ref().and_then(|c| c.all.clone()))
            .unwrap_or_default();
        let sets_clone: Vec<Vec<String>> = comptags
            .lock()
            .ok()
            .map(|t| {
                let mut out = Vec::new();
                if let Some(c) = t[lvl_idx].as_ref() {
                    let mut p = c.sets.as_deref();
                    while let Some(set) = p {
                        if let Some(ts) = set.tags.as_ref() {
                            out.push(ts.clone());
                        }
                        p = set.next.as_deref();
                    }
                }
                out
            })
            .unwrap_or_default();

        // c:4098-4108 — filter args, keep only registered tags not in any set.
        let filtered: Vec<String> = args[idx..]
            .iter()
            .filter(|p| {
                arrcontains(&all_arr, p, true) != 0
                    && !sets_clone.iter().any(|s| arrcontains(s, p, false) != 0)
            })
            .cloned()
            .collect();

        if filtered.is_empty() {
            return 0;
        }

        // c:4114-4134 — push as one set, or split (one per arg) with -s.
        if sep {
            for t in &filtered {
                append_set(vec![t.clone()]);
            }
        } else {
            append_set(filtered);
        }
    }
    0 // c:4138
}

/// Port of `PATH_MAX2` from `computil.c:4141`. `PATH_MAX * 2` — buffer
/// budget for path-completion staging strings.
pub const PATH_MAX2: usize = 8192; // c:4141 (PATH_MAX*2, 4096*2)

/// Direct port of `static LinkList cfp_test_exact(LinkList names,
///                                                  char **accept,
///                                                  char *skipped)` from
/// `Src/Zle/computil.c:4160-4290`. Returns the subset of `names` whose
/// `name + skipped + compprefix + compsuffix` resolves to an existing
/// file. When `accept` is non-boolean, the resolved path must also
/// match at least one of the compiled accept-patterns. Returns None
/// when nothing matched.
pub fn cfp_test_exact(
    names: &[String],
    accept: &[String], // c:4160
    skipped: &str,
) -> Option<Vec<String>> {
    let compprefix = COMPPREFIX
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();
    let compsuffix = COMPSUFFIX
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();

    // c:4175 — bail when both prefix and suffix are empty.
    if compprefix.is_empty() && compsuffix.is_empty() {
        return None;
    }

    // c:4181 — accept-exact off?
    let accept_off = accept.is_empty()
        || (accept.len() == 1 && matches!(accept[0].as_str(), "false" | "no" | "off" | "0"));
    if accept_off {
        // c:4188
        return None;
    }

    // c:4199-4214 — build compiled Patprog list from non-boolean accept.
    let mut alist: Option<Vec<Patprog>> = None;
    let is_boolean_true =
        accept.len() == 1 && matches!(accept[0].as_str(), "true" | "yes" | "on" | "1");
    if !is_boolean_true {
        let mut list: Vec<Patprog> = Vec::new();
        let mut all_star = false;
        for p in accept {
            if p == "*" {
                // c:4207 wildcard short-circuit
                all_star = true;
                break;
            }
            let mut p_copy = p.clone();
            tokenize(&mut p_copy);
            if let Some(prog) = patcompile(&p_copy, 0, None::<&mut String>) {
                list.push(prog);
            }
        }
        if !all_star {
            alist = Some(list);
        }
    }

    // c:4220-4227 — assemble `suf = skipped + rembslash(prefix + suffix)`.
    let sl = skipped.len() + compprefix.len() + compsuffix.len();
    if sl > PATH_MAX2 {
        // c:4223
        return None;
    }
    let suf = format!(
        "{}{}",
        skipped,
        rembslash(&format!("{}{}", compprefix, compsuffix))
    );

    let mut ret: Vec<String> = Vec::new();
    for p in names {
        // c:4229
        let l = p.len();
        if l + sl >= PATH_MAX2 {
            continue;
        } // c:4231
        let buf = format!("{}{}", p, suf);
        if ztat(&buf, false).is_none() {
            continue;
        } // c:4269 stat exists?
          // c:4274 — accept-pattern check.
        if let Some(ref ps) = alist {
            let any_match = ps.iter().any(|prog| pattry(prog, &buf));
            if !any_match {
                continue;
            }
        }
        ret.push(buf); // c:4285
    }

    if ret.is_empty() {
        None
    } else {
        Some(ret)
    } // c:4289
}

/// Direct port of `static char *cfp_matcher_range(Cmatcher *ms, char *add)`
/// from `Src/Zle/computil.c:4307-4520`. For each character of `add`,
/// consults the parallel `ms[i]` matcher and emits a pattern fragment:
///   - no matcher: the character verbatim
///   - CMF_RIGHT: `*c`
///   - word EQUIV+line EQUIV: `[c eq(c)]` (two-char class with
///     the equivalent char from the word side)
///   - CPAT_NCLASS: `[^class]`
///   - CPAT_CCLASS / CPAT_EQUIV / CPAT_CHAR: `[classchar+addchar]`
///   - CPAT_ANY: `?`
pub fn cfp_matcher_range(
    ms: &[Option<Box<Cmatcher>>], // c:4307
    add: &str,
) -> String {
    // Local PATMATCHRANGE — Rust copy of the helper used by pattern_match1
    // / pattern_match_equivalence. Walks an encoded char-range byte
    // sequence looking for `c`. Encoding:
    //   0x80 + PP_RANGE: next 2 bytes are lo,hi range
    //   0x80 + PP_*: POSIX class marker; matched for LOWER/UPPER
    //   plain bytes: literal char
    // Returns Some((idx, mtp)) on hit.
    fn patmatchrange_local(s: Option<&[u8]>, c: u32) -> Option<(u32, i32)> {
        let bytes = s?;
        let pp_range_marker = (0x80u8).wrapping_add(PP_RANGE as u8);
        let pp_lower_marker = (0x80u8).wrapping_add(PP_LOWER as u8);
        let pp_upper_marker = (0x80u8).wrapping_add(PP_UPPER as u8);

        let mut idx: u32 = 0;
        let mut i = 0usize;
        while i < bytes.len() {
            let b = bytes[i];
            if b == pp_range_marker {
                if i + 2 >= bytes.len() {
                    break;
                }
                let r1 = bytes[i + 1] as u32;
                let r2 = bytes[i + 2] as u32;
                if c >= r1 && c <= r2 {
                    return Some((idx, 0));
                }
                idx += 1;
                i += 3;
            } else if b >= 0x80 {
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
                    return Some((idx, (b as i32) - 0x80));
                }
                idx += 1;
                i += 1;
            } else {
                if c == b as u32 {
                    return Some((idx, 0));
                }
                idx += 1;
                i += 1;
            }
        }
        None
    }

    let mut out = String::with_capacity(add.len() * 2);
    let add_chars: Vec<(usize, char)> = add.char_indices().collect();

    for (i, (_byte_idx, ch)) in add_chars.iter().enumerate() {
        let addc = *ch as u32;
        let m_opt = ms.get(i).and_then(|x| x.as_deref());

        match m_opt {
            None => {
                // c:4331 — no matcher: emit char verbatim.
                out.push(*ch);
            }
            Some(m) if (m.flags & CMF_RIGHT) != 0 => {
                // c:4344 — right-anchored: `*char`.
                out.push('*');
                out.push(*ch);
            }
            Some(m) => {
                let word: Option<&Cpattern> = m.word.as_deref();
                let line: Option<&Cpattern> = m.line.as_deref();
                if let (Some(l), Some(w)) = (line, word) {
                    if l.tp == CPAT_EQUIV && w.tp == CPAT_EQUIV {
                        // c:4359 — genuine equivalence; emit `[char eq]`.
                        out.push('[');
                        out.push(*ch);
                        if let Some((ind, mtp)) = patmatchrange_local(l.str.as_deref(), addc) {
                            let eq = pattern_match_equivalence(w, ind + 1, mtp, addc);
                            if eq != u32::MAX {
                                if let Some(c) = char::from_u32(eq) {
                                    // c:60 — imeta(byte) gate; `c` is a
                                    // u32 codepoint, so cap to u8 range
                                    // before calling the byte-arg port.
                                    let _ = if eq <= 0xff { imeta(eq as u8) } else { false };
                                    out.push(c);
                                }
                            }
                        }
                        out.push(']');
                        continue;
                    }
                }
                // Local helper: decode an encoded Cpattern.str byte
                // sequence into a `[…]`-suitable readable form. POSIX
                // class markers become `[:name:]`; ranges become
                // `lo-hi`; literals pass through.
                fn decode_range_bytes(bytes: &[u8]) -> String {
                    let pp_range_marker = (0x80u8).wrapping_add(PP_RANGE as u8);
                    let mut out = String::new();
                    let mut i = 0usize;
                    while i < bytes.len() {
                        let b = bytes[i];
                        if b == pp_range_marker && i + 2 < bytes.len() {
                            out.push(bytes[i + 1] as char);
                            out.push('-');
                            out.push(bytes[i + 2] as char);
                            i += 3;
                        } else if b >= 0x80 {
                            let cls = (b as usize) - 0x80;
                            // c:1240+ — POSIX class marker byte → `[:name:]`.
                            // Inverse of `range_type`: index into the
                            // POSIX_CLASS_NAMES table.
                            const POSIX_CLASSES: &[&str] = &[
                                "alpha", "alnum", "blank", "cntrl", "digit", "graph", "lower",
                                "print", "punct", "space", "upper", "xdigit",
                            ];
                            if cls > 0 && cls - 1 < POSIX_CLASSES.len() {
                                out.push_str(&format!("[:{}:]", POSIX_CLASSES[cls - 1]));
                            }
                            i += 1;
                        } else {
                            out.push(b as char);
                            i += 1;
                        }
                    }
                    out
                }

                if let Some(w) = word {
                    match w.tp {
                        x if x == CPAT_NCLASS => {
                            // c:4401
                            out.push('[');
                            out.push('^');
                            if let Some(bytes) = w.str.as_deref() {
                                out.push_str(&decode_range_bytes(bytes));
                            }
                            out.push(']');
                        }
                        x if x == CPAT_CCLASS || x == CPAT_EQUIV || x == CPAT_CHAR => {
                            // c:4435 / c:4441 / c:4442
                            out.push('[');
                            let mut mt = 0i32;
                            let addadd = pattern_match1(w, addc, &mut mt) == 0;
                            // c:4455 — if addadd && *add == ']', emit ']' first.
                            if addadd && *ch == ']' {
                                out.push(*ch);
                            }
                            if w.tp == CPAT_CHAR {
                                // c:4461
                                if let Some(c) = char::from_u32(w.chr) {
                                    out.push(c);
                                }
                            } else {
                                // c:4476
                                if let Some(bytes) = w.str.as_deref() {
                                    out.push_str(&decode_range_bytes(bytes));
                                }
                            }
                            if addadd && *ch != ']' {
                                // c:4489
                                out.push(*ch);
                            }
                            out.push(']');
                        }
                        x if x == CPAT_ANY => {
                            // c:4502
                            out.push('?');
                        }
                        _ => {
                            // Fallback: emit verbatim.
                            out.push(*ch);
                        }
                    }
                } else {
                    out.push(*ch);
                }
            }
        }
    }
    out
}

/// Direct port of `static char *cfp_matcher_pats(char *matcher, char *add)`
/// from `Src/Zle/computil.c:4525-4613`. Parses the matcher spec into
/// a Cmatcher chain, then walks each chain entry truncating `add` at
/// the first character that matches the matcher's stop pattern, and
/// recording one matcher per surviving character. Finally calls
/// cfp_matcher_range to synthesize the output pattern.
///
/// Returns:
///   - the transformed string (possibly empty) on success
///   - the original `add` unchanged when the matcher spec is empty
///     or unparseable
pub fn cfp_matcher_pats(matcher: &str, add: &str) -> String {
    // c:4525

    // c:4527 — parse_cmatcher returns None on error (the C pcm_err path).
    let m_chain = parse_cmatcher("", matcher);
    let Some(mut m_chain) = m_chain else {
        return add.to_string(); // c:4529
    };

    // c:4531-4538 — ms[0..zl] is one matcher slot per character of add.
    let zl = ztrlen(add); // c:4531
    let mut ms: Vec<Option<Box<Cmatcher>>> = (0..zl).map(|_| None).collect();
    let mut add_owned = add.to_string();

    let mut m_opt: Option<&Cmatcher> = Some(&*m_chain);
    while let Some(m) = m_opt {
        let mut stopp: Option<&Cpattern> = None;
        let mut stopl: i32 = 0;

        if (m.flags & (CMF_LEFT | CMF_RIGHT)) == 0 {
            // c:4542
            if m.llen == 1 && m.wlen == 1 {
                // c:4543
                // c:4550 — walk add looking for the first char where the
                // matcher's `line` pattern matches; record `m` in ms[i].
                let chars: Vec<(usize, char)> = add_owned.char_indices().collect();
                for (i, (byte_idx, _ch)) in chars.iter().enumerate() {
                    if i >= ms.len() {
                        break;
                    }
                    let slice = &add_owned[*byte_idx..];
                    if pattern_match(m.line.as_deref(), slice, None, "") != 0 {
                        // c:4551 — `if (*mp)` collision: truncate add.
                        if ms[i].is_some() {
                            add_owned.truncate(*byte_idx); // c:4553
                            break;
                        } else {
                            ms[i] = Some(Box::new(m.clone())); // c:4557
                        }
                    }
                }
            } else {
                stopp = m.line.as_deref(); // c:4565
                stopl = m.llen;
            }
        } else if (m.flags & CMF_RIGHT) != 0 {
            // c:4568
            if m.wlen < 0 && m.llen == 0 && m.ralen == 1 {
                // c:4569
                let chars: Vec<(usize, char)> = add_owned.char_indices().collect();
                for (i, (byte_idx, _ch)) in chars.iter().enumerate() {
                    if i >= ms.len() {
                        break;
                    }
                    let slice = &add_owned[*byte_idx..];
                    if pattern_match(m.right.as_deref(), slice, None, "") != 0 {
                        // c:4572 — collision OR leading-dot guard.
                        let leading_dot = *byte_idx == 0 && slice.starts_with('.');
                        if ms[i].is_some() || leading_dot {
                            add_owned.truncate(*byte_idx); // c:4573
                            break;
                        } else {
                            ms[i] = Some(Box::new(m.clone()));
                        }
                    }
                }
            } else if m.llen != 0 {
                // c:4584
                stopp = m.line.as_deref();
                stopl = m.llen;
            } else {
                stopp = m.right.as_deref(); // c:4588
                stopl = m.ralen;
            }
        } else {
            // c:4591 CMF_LEFT
            if m.lalen == 0 {
                // c:4592
                return String::new(); // c:4593
            }
            stopp = m.left.as_deref();
            stopl = m.lalen;
        }

        // c:4598-4608 — apply stopp truncation.
        if let Some(sp) = stopp {
            let chars: Vec<(usize, char)> = add_owned.char_indices().collect();
            let mut bytes_remaining = add_owned.len() as i32;
            for (_i, (byte_idx, _ch)) in chars.iter().enumerate() {
                if bytes_remaining < stopl {
                    break;
                }
                let slice = &add_owned[*byte_idx..];
                if pattern_match(Some(sp), slice, None, "") != 0 {
                    add_owned.truncate(*byte_idx); // c:4601
                    break;
                }
                bytes_remaining -= 1;
            }
        }

        m_opt = m.next.as_deref();
    }

    // c:4610 — synthesize the output via cfp_matcher_range.
    if !add_owned.is_empty() {
        cfp_matcher_range(&ms, &add_owned)
    } else {
        add_owned // c:4613
    }
}

/// Direct port of `static void cfp_opt_pats(char **pats, char *matcher)`
/// from `Src/Zle/computil.c:4621-4701`. "Optimization" pass that
/// prefixes each `*…`-leading pattern with the literal portion of
/// `compprefix` that no pattern would consume. The walk computes a
/// shrinking `add` string — each pattern crosses off the chars in
/// `add` it would match — and any remaining chars become the prefix.
///
/// Modifies `pats` in place; returns the (possibly modified) list.
pub fn cfp_opt_pats(pats: &[String], matcher: &str) -> Vec<String> {
    // c:4621

    let compprefix = COMPPREFIX
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();
    if compprefix.is_empty() {
        // c:4625
        return pats.to_vec();
    }
    let compsuffix = COMPSUFFIX
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();

    // c:4628-4633 — if comppatmatch && haswilds(rembslash(prefix+suffix)): bail.
    let cpm_set = comppatmatch
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.is_some()))
        .unwrap_or(false);
    if cpm_set {
        let merged = format!("{}{}", compprefix, compsuffix);
        let mut t = rembslash(&merged);
        tokenize(&mut t);
        remnulargs(&mut t);
        if haswilds(&t) {
            return pats.to_vec(); // c:4632
        }
    }

    // c:4634-4649 — build `add` by walking compprefix, unescaping `\X`
    // for non-special X, and pre-escaping unescaped specials.
    const SPECIALS: &[u8] = b"*?<>()[]|#^~=";
    let cp_bytes = compprefix.as_bytes();
    let mut add: Vec<u8> = Vec::with_capacity(cp_bytes.len() * 2);
    let mut i = 0usize;
    while i < cp_bytes.len() {
        let c = cp_bytes[i];
        let keep = if c == b'\\' && i + 1 < cp_bytes.len() {
            // c:4636 — keep `\X` literal when X is non-special.
            let next = cp_bytes[i + 1];
            !SPECIALS.contains(&next)
        } else {
            true
        };
        if keep {
            let unescaped_at_start = i == 0 || cp_bytes[i - 1] != b'\\';
            if unescaped_at_start && SPECIALS.contains(&c) {
                // c:4640
                add.push(b'\\');
            }
            add.push(c);
        }
        i += 1;
    }
    let mut add_s: String = String::from_utf8_lossy(&add).into_owned();

    // c:4650-4691 — walk each pattern, cross off chars from `add`.
    for p_orig in pats {
        if add_s.is_empty() {
            break;
        }
        let mut q_bytes: Vec<u8> = p_orig.as_bytes().to_vec();
        if q_bytes.is_empty() {
            continue;
        }
        // c:4654 — strip trailing alternation `(…|…)` group.
        if let Some(b')') = q_bytes.last().copied() {
            let mut t = q_bytes.len() - 1;
            let mut found = None;
            while t > 0 {
                t -= 1;
                let c = q_bytes[t];
                if c == b')' || c == b'|' || c == b'~' || c == b'(' {
                    found = Some((t, c));
                    break;
                }
            }
            if let Some((idx, c)) = found {
                if c == b'(' {
                    q_bytes.truncate(idx);
                }
            }
        }

        let mut qi = 0usize;
        while qi < q_bytes.len() && !add_s.is_empty() {
            let c = q_bytes[qi];
            if c == b'\\' && qi + 1 < q_bytes.len() {
                // c:4662
                qi += 1;
                let target = q_bytes[qi];
                // c:4663 — cross off `target` from add.
                if let Some(pos) = add_s.find(target as char) {
                    add_s.truncate(pos);
                }
            } else if c == b'<' {
                // c:4665
                // c:4666 — cross off any digit.
                let cut_at = add_s.bytes().position(|b| idigit(b));
                if let Some(pos) = cut_at {
                    add_s.truncate(pos);
                }
            } else if c == b'[' {
                // c:4668
                // c:4669-4684 — character class.
                let mut xi = qi + 1;
                let not = xi < q_bytes.len() && (q_bytes[xi] == b'!' || q_bytes[xi] == b'^');
                if not {
                    xi += 1;
                }
                let _ = not;
                while xi < q_bytes.len() && q_bytes[xi] != b']' {
                    if xi + 2 < q_bytes.len() && q_bytes[xi + 1] == b'-' {
                        let c1 = q_bytes[xi];
                        let c2 = q_bytes[xi + 2];
                        let cut_at = add_s.bytes().position(|b| b >= c1 && b <= c2);
                        if let Some(pos) = cut_at {
                            add_s.truncate(pos);
                        }
                        xi += 3;
                    } else {
                        let cut_at = add_s.find(q_bytes[xi] as char);
                        if let Some(pos) = cut_at {
                            add_s.truncate(pos);
                        }
                        xi += 1;
                    }
                }
                qi = xi;
            } else if c != b'?'
                && c != b'*'
                && c != b'('
                && c != b')'
                && c != b'|'
                && c != b'~'
                && c != b'#'
            // c:4685
            {
                let cut_at = add_s.find(c as char);
                if let Some(pos) = cut_at {
                    add_s.truncate(pos);
                }
            }
            qi += 1;
        }
    }

    // c:4693-4700 — prepend `add` to each `*`-leading pattern.
    let mut out: Vec<String> = pats.to_vec();
    if !add_s.is_empty() {
        let final_add = if !matcher.is_empty() {
            let m = cfp_matcher_pats(matcher, &add_s);
            if m.is_empty() {
                return out;
            } // c:4694
            m
        } else {
            add_s
        };
        for p in out.iter_mut() {
            if p.starts_with('*') {
                *p = format!("{}{}", final_add, p);
            }
        }
    }
    out
}

/// Direct port of `static LinkList cfp_bld_pats(UNUSED(int dirs),
///                                                LinkList names,
///                                                char *skipped,
///                                                char **pats)` from
/// `Src/Zle/computil.c:4704-4732`. For each (name, pattern) pair,
/// builds `name + skipped + pattern`. When GLOBDOTS is unset and the
/// compprefix starts with `.`, also adds a dot-prefixed variant.
pub fn cfp_bld_pats(
    _dirs: i32,
    names: &[String],
    skipped: &str, // c:4704
    pats: &[String],
) -> Vec<String> {
    let compprefix = COMPPREFIX
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();
    // c:4711 — `dot = unset(GLOBDOTS) && compprefix && *compprefix == '.'`.
    let dot = unset(GLOBDOTS) && compprefix.starts_with('.');

    let mut ret: Vec<String> = Vec::new();
    for o in names {
        // c:4712
        for p in pats {
            // c:4714
            // c:4716 — `str = o + skipped + p`.
            ret.push(format!("{}{}{}", o, skipped, p));
            // c:4721 — dot variant when GLOBDOTS unset and pattern
            // doesn't already start with '.'.
            if dot && !p.starts_with('.') {
                ret.push(format!("{}{}.{}", o, skipped, p));
            }
        }
    }
    ret // c:4731
}

/// Direct port of `static LinkList cfp_add_sdirs(LinkList final,
///                                                LinkList orig, char *skipped,
///                                                char *sdirs, char **fake)`
/// from `Src/Zle/computil.c:4762-4854`. Two effects:
///   1. When `sdirs` is enabled (and GLOBDOTS is set or the compprefix
///      begins with `.`), append `skipped + ".."` (and, for the boolean
///      forms, `skipped + "."`) to every `orig` node.
///   2. Expand each `fake` entry of the form `pattern:repl1 repl2 ...`:
///      for every `orig` node whose name matches `pattern` (or names the
///      same file by dev/ino), append `node + skipped + repl` for each
///      whitespace-separated replacement.
/// C returns `final`; Rust mutates `final_list` in place.
pub fn cfp_add_sdirs(
    final_list: &mut Vec<String>,
    orig: &[String], // c:4762 (params: final, orig, skipped, sdirs, fake)
    skipped: &str,
    sdirs: &str,
    fake: &[String],
) {
    let compprefix = COMPPREFIX
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_default();

    // c:4766-4774 — decide whether/what dot-dirs to add.
    let mut add = 0;
    // c:4768 — only when GLOBDOTS set or compprefix starts with `.`.
    if !sdirs.is_empty() && (isset(GLOBDOTS) || compprefix.starts_with('.')) {
        match sdirs {
            "yes" | "true" | "on" | "1" => add = 2, // c:4769-4771
            ".." => add = 1,                        // c:4772-4773
            _ => {}
        }
    }
    // c:4775-4787 — append `skipped + ".."` (and `skipped + "."` for the
    // boolean forms) to each orig node via dyncat.
    if add != 0 {
        let s1 = format!("{}..", skipped); // c:4777 dyncat(skipped, "..")
        let s2 = if add == 2 {
            Some(format!("{}.", skipped)) // c:4778 dyncat(skipped, ".")
        } else {
            None
        };
        for m in orig {
            // c:4781 — C skips NULL node data; Rust nodes are always present.
            final_list.push(format!("{}{}", m, s1)); // c:4782 dyncat(m, s1)
            if let Some(ref s2) = s2 {
                final_list.push(format!("{}{}", m, s2)); // c:4784 dyncat(m, s2)
            }
        }
    }

    // c:4788-4852 — expand `fake` entries of form `pattern:repl1 repl2 ...`.
    for entry in fake {
        // c:4795-4796 — f = dupstring(*fake).
        let bytes = entry.as_bytes();
        // c:4797-4808 — copy the pattern up to the first unescaped ':',
        // stripping the backslash from any `\:` (other backslashes are left
        // for tokenization to strip).
        let mut pat: Vec<u8> = Vec::new();
        let mut p = 0usize;
        let mut colon = false;
        while p < bytes.len() {
            let c = bytes[p];
            if c == b':' {
                colon = true; // c:4798-4799
                break;
            } else if c == b'\\' && p + 1 < bytes.len() && bytes[p + 1] == b':' {
                p += 1; // c:4800-4806 strip quoted-colon backslash
            }
            pat.push(bytes[p]);
            p += 1;
        }
        // c:4809 — entries without a colon carry no replacement list.
        if !colon {
            continue;
        }
        // c:4810 — step past the colon.
        p += 1;
        // c:4811-4812 — nothing after the colon: skip.
        if p >= bytes.len() {
            continue;
        }
        let rest = &bytes[p..];

        // c:4814-4818 — compile the pattern (tokenize/patcompile/untokenize).
        // PAT_STATIC protects the shared static buffer, hence queue_signals.
        crate::ported::signals_h::queue_signals();
        let pat_str = String::from_utf8_lossy(&pat).into_owned();
        let mut tok = pat_str.clone();
        tokenize(&mut tok);
        let pprog: Option<Patprog> =
            patcompile(&tok, crate::ported::zsh_h::PAT_STATIC, None::<&mut String>);
        // C untokenizes `f` back to the original text for the strcmp fallback
        // below; `pat_str` already holds that original text.

        // c:4819-4847 — for each matching orig node, split the replacement
        // list on blanks (stripping backslash escapes) and append
        // `node + skipped + repl`.
        //
        // NOTE: C consumes the replacement cursor `p` destructively inside
        // the first matching node's inner loop (c:4825 `while (*p)`), so only
        // the FIRST matching node emits replacements; later matching nodes hit
        // an already-exhausted `p` and emit nothing. Faithful port: emit for
        // the first match, then stop.
        for m in orig {
            // c:4820-4821 — pattern match (or literal compare if compile failed).
            let name_match = match &pprog {
                Some(prog) => pattry(prog, m),
                None => pat_str == *m,
            };
            // c:4822-4824 — else same file by dev/ino (empty node → ".").
            let matched = name_match || {
                let mpath = if m.is_empty() { "." } else { m.as_str() };
                match (ztat(&pat_str, true), ztat(mpath, true)) {
                    (Some(a), Some(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                    _ => false,
                }
            };
            if matched {
                // c:4825-4845 — walk the whitespace-separated replacements.
                let mut q = 0usize;
                while q < rest.len() {
                    // c:4826-4827 — skip leading blanks.
                    while q < rest.len() && inblank(rest[q]) {
                        q += 1;
                    }
                    if q >= rest.len() {
                        break; // c:4828-4829
                    }
                    // c:4830-4836 — collect one token, stripping `\`-escapes.
                    let mut token: Vec<u8> = Vec::new();
                    while q < rest.len() {
                        let rc = rest[q];
                        if inblank(rc) {
                            break; // c:4831-4832
                        } else if rc == b'\\' && q + 1 < rest.len() {
                            q += 1; // c:4833-4834
                        }
                        token.push(rest[q]);
                        q += 1;
                    }
                    // c:4839-4843 — a = m + skipped + token.
                    let tstr = String::from_utf8_lossy(&token).into_owned();
                    final_list.push(format!("{}{}{}", m, skipped, tstr));
                }
                // c:4825 — `p` is now exhausted; no later node can emit.
                break;
            }
        }
        crate::ported::signals_h::unqueue_signals();
    }
}

/// Direct port of `static char **cf_pats(int dirs, int noopt,
///                                       char **names, char **accept,
///                                       char *skipped, char *matcher,
///                                       char *sdirs, char **fake,
///                                       char **pats)` from
/// `Src/Zle/computil.c:4829`. Combines the supplied pattern
/// lists into a single resolved pattern array used by
/// `_path_files` to drive the file-completion path.
///
/// **Substrate tradeoff:** the helper chain
/// `cfp_test_exact`/`cfp_opt_pats`/`cfp_bld_pats`/`cfp_add_sdirs`
/// Direct port of `static LinkList cf_pats(int dirs, int noopt,
///                                          LinkList names, char **accept,
///                                          char *skipped, char *matcher,
///                                          char *sdirs, char **fake,
///                                          char **pats)` from
/// `Src/Zle/computil.c:4829-4848`. The cf_pats driver:
/// 1. Try cfp_test_exact first; if it returns a non-empty list, fold
///    in `sdirs`/`fake` via cfp_add_sdirs and return.
/// 2. Otherwise: if dirs, replace `pats` with `*(-/)`. If !noopt run
///    cfp_opt_pats. Then build the patterns via cfp_bld_pats and fold
///    in sdirs/fake.
pub fn cf_pats(
    dirs: i32,
    noopt: i32,
    names: &[String], // c:4829
    accept: &[String],
    skipped: &str,
    matcher: &str,
    sdirs: &str,
    fake: &[String],
    pats: &[String],
) -> Vec<String> {
    // c:4835 — try exact-match pass first.
    if let Some(exact) = cfp_test_exact(names, accept, skipped) {
        let mut out = exact;
        cfp_add_sdirs(&mut out, names, skipped, sdirs, fake); // c:4836
        return out;
    }

    // c:4838 — when dirs is set, force the `*(-/)` directory glob.
    let dir_pats = vec!["*(-/)".to_string()];
    let active_pats: Vec<String> = if dirs != 0 {
        dir_pats
    } else if noopt == 0 {
        // c:4843 — optimization pass.
        cfp_opt_pats(pats, matcher)
    } else {
        pats.to_vec()
    };

    // c:4846 — build the glob array.
    let mut out = cfp_bld_pats(dirs, names, skipped, &active_pats);
    cfp_add_sdirs(&mut out, names, skipped, sdirs, fake);
    out
}

/// Direct port of `static void cf_ignore(char **names, LinkList ign,
///                                          char *style, char *path)`
/// from `Src/Zle/computil.c:4860-4896`. Adds to `ign` any directory
/// in `names` that:
///   - "pwd" style: shares the same dev/ino as `$PWD` (so completion
///     doesn't offer the directory you're already in).
///   - "parent" style: is an ancestor directory of `path` (so when
///     completing under `/a/b/c/`, `/a/`, `/a/b/`, etc. don't show
///     up as options).
/// Quoted with QT_BACKSLASH for safe re-insertion into the line.
pub fn cf_ignore(names: &[String], ign: &mut Vec<String>, style: &str, path: &str) {
    // c:4860

    let pl = path.len();
    let tpar = style.contains("parent"); // c:4866
    let pwd = crate::ported::params::getsparam("PWD").unwrap_or_default();
    let est = if !pwd.is_empty() {
        ztat(&pwd, true)
    } else {
        None
    };
    let tpwd = style.contains("pwd") && est.is_some(); // c:4867

    if !tpar && !tpwd {
        return;
    } // c:4870

    for n in names {
        // c:4873
        let nst = match ztat(n, true) {
            // c:4874 lstat
            Some(m) if m.is_dir() => m,
            _ => continue,
        };
        if tpwd {
            if let Some(ref est) = est {
                if nst.dev() == est.dev() && nst.ino() == est.ino() {
                    // c:4875
                    ign.push(quotestring(n, QT_BACKSLASH)); // c:4876
                    continue;
                }
            }
        }
        if tpar && pl > 0 && n.starts_with(path) {
            // c:4879
            let mut c = n.clone();
            let mut found = false;
            // c:4881 — walk up via strrchr('/') while above path-prefix.
            while let Some(idx) = c.rfind('/') {
                if idx <= pl {
                    break;
                }
                c.truncate(idx);
                if let Some(st) = ztat(&c, false) {
                    // c:4883 stat
                    if st.dev() == nst.dev() && st.ino() == nst.ino() {
                        found = true;
                        break;
                    }
                }
            }
            // c:4889 — fallback last-segment check via lstat.
            let last_match = if !found {
                if let Some(idx) = c.rfind('/') {
                    if idx > pl {
                        c.truncate(idx);
                        ztat(&c, true)
                            .map_or(false, |st| st.dev() == nst.dev() && st.ino() == nst.ino())
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            };
            if found || last_match {
                ign.push(quotestring(n, QT_BACKSLASH)); // c:4892
            }
        }
    }
}

/// Direct port of `static LinkList cf_remove_other(char **names,
///                                                   char *pre, int *amb)`
/// from `Src/Zle/computil.c:4899-4953`. Helper for `_path_files` that
/// reports whether the remaining `names` share a common directory
/// prefix (`*amb` cleared) or diverge (`*amb` set, return None).
/// When `pre` itself contains a `/`, names matching that head are
/// returned as the consensus list.
pub fn cf_remove_other(names: &[String], pre: &str, amb: &mut i32) -> Option<Vec<String>> {
    if let Some(slash) = pre.find('/') {
        // c:4903
        // c:4906-4908 — pre' = pre[..slash] + "/".
        let pre2 = format!("{}/", &pre[..slash]);

        // c:4910-4912 — any name with the truncated prefix?
        let any_match = names.iter().any(|n| strpfx(&pre2, n));

        if any_match {
            // c:4914
            // c:4915-4922 — return all matching names with amb=0.
            let ret: Vec<String> = names.iter().filter(|n| strpfx(&pre2, n)).cloned().collect();
            *amb = 0;
            return Some(ret); // c:4923
        } else {
            // c:4924
            // c:4925-4940 — check if remaining names all share first-name's head.
            let mut it = names.iter();
            let Some(first) = it.next() else {
                *amb = 0; // c:4926
                return None;
            };
            // c:4930 — strip after first '/' in first name.
            let p_head = match first.find('/') {
                Some(i) => format!("{}/", &first[..i]),
                None => format!("{}/", first),
            };
            for n in it {
                // c:4935
                if !strpfx(&p_head, n) {
                    *amb = 1; // c:4937
                    return None;
                }
            }
            // All match — fall through to return None (matches C).
        }
    } else {
        // c:4942
        // c:4943 — empty list: amb cleared.
        let mut it = names.iter();
        let Some(first) = it.next() else {
            *amb = 0;
            return None;
        };
        for n in it {
            // c:4946
            if first != n {
                // c:4947
                *amb = 1;
                return None;
            }
        }
    }
    None // c:4952
}

/// Direct port of `static int bin_compfiles(char *nam, char **args,
///                                            UNUSED(Options ops),
///                                            UNUSED(int func))` from
/// `Src/Zle/computil.c:4970-5070`. Subcommand dispatch for
/// `compfiles -p/-P/-i/-r`. `-i` runs cf_ignore on a param-named
/// array; `-r` runs cf_remove_other; `-p`/`-P` thread through cf_pats.
pub fn bin_compfiles(
    nam: &str,
    args: &[String], // c:4970
    _ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:4972
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() || !args[0].starts_with('-') {
        // c:4976
        let bad = args.first().map(|s| s.as_str()).unwrap_or("");
        zwarnnam(nam, &format!("missing option: {}", bad));
        return 1;
    }
    let a0 = args[0].as_bytes();
    if a0.len() < 2 {
        zwarnnam(nam, &format!("missing option: {}", args[0]));
        return 1;
    }
    let sub = a0[1];

    // Helper: read a named array via paramtab.
    let get_arr = |name: &str| -> Option<Vec<String>> {
        paramtab()
            .read()
            .ok()
            .and_then(|tab| tab.get(name).and_then(|pm| pm.u_arr.clone()))
    };

    match sub {
        b'p' | b'P' => {
            // `_path_files` rewrites `$PREFIX`/`$SUFFIX` for each path component
            // then calls compfiles; cf_pats/cfp_opt_pats build the glob from the
            // compprefix/compsuffix GLOBALS. In C those globals ARE the params
            // (gsu-bound); the Rust compparams aren't bound, and only addmatches
            // (compadd) refreshes them — so compfiles read a STALE prefix
            // (globbed the previous component's `sub*` for the final component
            // `al`, so `cmd /a/b/c/pre<TAB>` never matched). Mirror addmatches'
            // refresh here. (Same block as compcore.rs addmatches; no C fn — the
            // C binding is implicit, so this stays inline rather than a helper.)
            if crate::ported::zle::complete::INCOMPFUNC.load(Ordering::Relaxed) != 0 {
                for (param, global) in [
                    ("PREFIX", &crate::ported::zle::complete::COMPPREFIX),
                    ("SUFFIX", &crate::ported::zle::complete::COMPSUFFIX),
                    ("IPREFIX", &crate::ported::zle::complete::COMPIPREFIX),
                    ("ISUFFIX", &crate::ported::zle::complete::COMPISUFFIX),
                ] {
                    if let Some(v) = crate::ported::params::getsparam(param) {
                        if let Ok(mut g) = global
                            .get_or_init(|| std::sync::Mutex::new(String::new()))
                            .lock()
                        {
                            *g = v;
                        }
                    }
                }
            }
            // c:4981
            // c:4983 — `if (args[0][2] && (args[0][2] != '-' || args[0][3]))`
            // reject: the only valid forms are `-p`/`-P` (no 3rd char) or
            // `-p-`/`-P-` (a LONE trailing `-`). `-p--` / `-px` are invalid.
            // c:5005 — `noopt = !!args[0][2]` (true iff a 3rd char exists), so
            // `-p-` sets noopt. The previous port had this inverted (required
            // `-p--`, rejected `-p-`), breaking the `_comp_correct` path which
            // emits `-p-`/`-P-`. Bug #657.
            let noopt = a0.len() > 2;
            if a0.len() > 2 && (a0[2] != b'-' || a0.len() > 3) {
                zwarnnam(nam, &format!("invalid option: {}", args[0]));
                return 1;
            }
            // c:5019-5022 — `-p` needs args[1]..args[7] (len >= 8);
            // `-P` needs args[1]..args[6] (len >= 7). `<=` here was an
            // off-by-one that rejected minimum-length calls, breaking
            // every `_path_files` `-P` call (exactly 7 args) and `-p`
            // with a single pattern (exactly 8): `_files:compfiles:1:
            // too few arguments` on `ls -<TAB>`.
            let required = if sub == b'p' { 8 } else { 7 };
            if args.len() < required {
                // c:4990
                zwarnnam(nam, "too few arguments");
                return 1;
            }
            // c:4996 — getaparam(args[1]).
            let Some(src) = get_arr(&args[1]) else {
                zwarnnam(nam, &format!("unknown parameter: {}", args[1]));
                return 0;
            };
            // c:5001 — quotestring each entry with QT_BACKSLASH_PATTERN.
            let l: Vec<String> = src
                .iter()
                .map(|s| quotestring(s, QT_BACKSLASH_PATTERN))
                .collect();
            // c:5003 — cf_pats dispatch.
            let result = cf_pats(
                if sub == b'P' { 1 } else { 0 },
                if noopt { 1 } else { 0 },
                &l,
                &get_arr(&args[2]).unwrap_or_default(),
                &args[3],
                &args[4],
                &args[5],
                &get_arr(&args[6]).unwrap_or_default(),
                &args[7..],
            );
            setaparam(&args[1], result);
            0
        }
        b'i' => {
            // c:5010
            if a0.len() > 2 {
                // c:5011
                zwarnnam(nam, &format!("invalid option: {}", args[0]));
                return 1;
            }
            if args.len() < 5 {
                // c:5018
                zwarnnam(nam, "too few arguments");
                return 1;
            }
            if args.len() > 5 {
                // c:5022
                zwarnnam(nam, "too many arguments");
                return 1;
            }
            let mut l: Vec<String> = get_arr(&args[2]).unwrap_or_default();
            let Some(tmp) = get_arr(&args[1]) else {
                // c:5032
                zwarnnam(nam, &format!("unknown parameter: {}", args[1]));
                return 0;
            };
            cf_ignore(&tmp, &mut l, &args[3], &args[4]); // c:5037
            setaparam(&args[2], l); // c:5039
            0
        }
        b'r' => {
            // c:5042
            if args.len() < 3 {
                // c:5048
                zwarnnam(nam, "too few arguments");
                return 1;
            }
            if args.len() > 3 {
                // c:5052
                zwarnnam(nam, "too many arguments");
                return 1;
            }
            let Some(tmp) = get_arr(&args[1]) else {
                // c:5057
                zwarnnam(nam, &format!("unknown parameter: {}", args[1]));
                return 0;
            };
            let mut ret = 0i32;
            // c:5062 — cf_remove_other.
            if let Some(l) = cf_remove_other(&tmp, &args[2], &mut ret) {
                setaparam(&args[1], l);
            }
            ret // c:5065
        }
        _ => {
            zwarnnam(nam, &format!("invalid option: {}", args[0]));
            1
        }
    }
}

/// Direct port of `static int bin_compgroups(char *nam, char **args,
///                                              UNUSED(Options ops),
///                                              UNUSED(int func))` from
/// `Src/Zle/computil.c:5073-5100`. For each group name in args, opens
/// six successive completion groups with the same name but different
/// sort/uniq flags (NOSORT+UNIQCON, UNIQALL, NOSORT+UNIQCON, UNIQALL,
/// NOSORT, 0). Each begcmgroup is bracketed by endcmgroup. This is
/// how _path_files etc. register their match groups before adding
/// candidates via compadd.
pub fn bin_compgroups(
    nam: &str,
    args: &[String], // c:5073
    _ops: &options,
    _func: i32,
) -> i32 {
    if INCOMPFUNC.load(Ordering::Relaxed) != 1 {
        // c:5078
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    // c:5083 — for each group name, register 6 group variants.
    for n in args {
        // c:5083
        endcmgroup(None); // c:5084
        begcmgroup(Some(n), CGF_NOSORT | CGF_UNIQCON); // c:5085
        endcmgroup(None);
        begcmgroup(Some(n), CGF_UNIQALL); // c:5087
        endcmgroup(None);
        begcmgroup(Some(n), CGF_NOSORT | CGF_UNIQCON); // c:5089
        endcmgroup(None);
        begcmgroup(Some(n), CGF_UNIQALL); // c:5091
        endcmgroup(None);
        begcmgroup(Some(n), CGF_NOSORT); // c:5093
        endcmgroup(None);
        begcmgroup(Some(n), 0); // c:5095
    }
    0 // c:5099
}

/// Direct port of `int setup_(UNUSED(Module m))` from
/// `Src/Zle/computil.c:5124-5134`. Zeroes the three module caches
/// and resets `lasttaglevel`. Called on module load.
pub fn setup_() -> i32 {
    // c:5124
    // c:5126 — `memset(cadef_cache, 0, sizeof(cadef_cache))`.
    if let Ok(mut cache) = cadef_cache.lock() {
        for slot in cache.iter_mut() {
            freecadef(slot.take());
        }
    }
    // c:5127 — `memset(cvdef_cache, 0, sizeof(cvdef_cache))`.
    if let Ok(mut cache) = cvdef_cache.lock() {
        for slot in cache.iter_mut() {
            freecvdef(slot.take());
        }
    }
    // c:5129 — `memset(comptags, 0, sizeof(comptags))`.
    if let Ok(mut tab) = comptags.lock() {
        for slot in tab.iter_mut() {
            freectags(slot.take());
        }
    }
    // c:5131 — `lasttaglevel = 0`.
    lasttaglevel.store(0, Ordering::Relaxed);
    0 // c:5133
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from Src/Zle/computil.c:5138.
/// WARNING: param names don't match C — Rust=() vs C=(m, features)
pub fn features_() -> i32 {
    // c:5138
    // C body c:5140-5141 — `*features = featuresarray(...); return 0`.
    //                      Features array exposed elsewhere; return 0.
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from Src/Zle/computil.c:5146.
/// WARNING: param names don't match C — Rust=() vs C=(m, enables)
pub fn enables_() -> i32 {
    // c:5146
    // C body c:5148 — `return handlefeatures(m, &module_features, enables)`.
    //                  Static-link no-op.
    0
}

/// Port of `boot_(UNUSED(Module m))` from Src/Zle/computil.c:5153.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn boot_() -> i32 {
    // c:5153
    // C body c:5155-5156 — `return 0`. Faithful empty body.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from Src/Zle/computil.c:5160.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn cleanup_() -> i32 {
    // c:5160
    // C body c:5162-5163 — `return setfeatureenables(m, &module_features, NULL)`.
    //                      Static-link path: no per-feature toggle, return 0.
    0
}

/// Direct port of `int finish_(UNUSED(Module m))` from
/// `Src/Zle/computil.c:5167-5180`. Frees every cached cadef/cvdef
/// and the comptags table on module unload.
pub fn finish_() -> i32 {
    // c:5167
    // c:5171 — `for (i = 0; i < MAX_CACACHE; i++) freecadef(cadef_cache[i])`.
    if let Ok(mut cache) = cadef_cache.lock() {
        for slot in cache.iter_mut() {
            freecadef(slot.take());
        }
    }
    // c:5173 — `for (i = 0; i < MAX_CVCACHE; i++) freecvdef(cvdef_cache[i])`.
    if let Ok(mut cache) = cvdef_cache.lock() {
        for slot in cache.iter_mut() {
            freecvdef(slot.take());
        }
    }
    // c:5176 — `for (i = 0; i < MAX_TAGS; i++) freectags(comptags[i])`.
    if let Ok(mut tab) = comptags.lock() {
        for slot in tab.iter_mut() {
            freectags(slot.take());
        }
    }
    0 // c:5179
}

/// Port of `static struct cdstate cd_state` from `Src/Zle/computil.c:93`.
/// File-static instance the `_describe` engine reads/writes.
pub static cd_state: std::sync::Mutex<cdstate> = // c:93
    std::sync::Mutex::new(cdstate {
        showd: 0,
        sep: None,
        slen: 0,
        swidth: 0,
        maxmlen: 0,
        sets: None,
        pre: 0,
        premaxw: 0,
        suf: 0,
        maxg: 0,
        maxglen: 0,
        groups: 0,
        descs: 0,
        gprew: 0,
        runs: None,
    });

/// CM_SPACE — inter-match spacing from `Src/Zle/zle_tricky.c:1700` /
/// `Src/Zle/computil.c` (referenced as the literal `2`). Used to
/// reserve a 2-char gap between adjacent matches when computing
/// column widths.
pub const CM_SPACE: i32 = 2; // c:zle_tricky.c

/// Port of `static struct castate ca_laststate` from
/// `Src/Zle/computil.c:1955`. Most recently parsed cmdline state.
pub static ca_laststate: std::sync::Mutex<castate> = // c:1955
    std::sync::Mutex::new(castate {
        snext: None,
        d: None,
        nopts: 0,
        def: None,
        ddef: None,
        curopt: None,
        dopt: None,
        opt: 0,
        arg: 0,
        argbeg: 0,
        optbeg: 0,
        nargbeg: 0,
        restbeg: 0,
        curpos: 0,
        argend: 0,
        inopt: 0,
        inarg: 0,
        nth: 0,
        singles: 0,
        oopt: 0,
        actopts: 0,
        args: None,
        oargs: None,
    });

/// Port of `static int ca_alloced` from `Src/Zle/computil.c:1960`.
pub static ca_alloced: std::sync::atomic::AtomicI32 = // c:1960
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static struct cvstate cv_laststate` from
/// `Src/Zle/computil.c:3229`.
pub static cv_laststate: std::sync::Mutex<cvstate> = // c:3229
    std::sync::Mutex::new(cvstate {
        d: None,
        def: None,
        val: None,
        vals: None,
    });

/// Port of `static int cv_alloced` from `Src/Zle/computil.c:3230`.
pub static cv_alloced: std::sync::atomic::AtomicI32 = // c:3230
    std::sync::atomic::AtomicI32::new(0);

/// Mirror of `memcpy` from a locked `ca_laststate` so we can walk the
/// snext chain without holding the mutex. Rust-only artifact.
#[allow(dead_code)]
fn clone_castate_full(s: &castate) -> castate {
    castate {
        snext: s.snext.clone(),
        d: s.d.clone(),
        nopts: s.nopts,
        def: s.def.clone(),
        ddef: s.ddef.clone(),
        curopt: s.curopt.clone(),
        dopt: s.dopt.clone(),
        opt: s.opt,
        arg: s.arg,
        argbeg: s.argbeg,
        optbeg: s.optbeg,
        nargbeg: s.nargbeg,
        restbeg: s.restbeg,
        curpos: s.curpos,
        argend: s.argend,
        inopt: s.inopt,
        inarg: s.inarg,
        nth: s.nth,
        singles: s.singles,
        oopt: s.oopt,
        actopts: s.actopts,
        args: s.args.clone(),
        oargs: s.oargs.clone(),
    }
}

/// Mirror of the C `memcpy(&ca_laststate, &state, sizeof(state))` pattern
/// — we can't move `state` (the caller continues using it), and Rust's
/// own `Clone` would over-clone owned chains.  Instead snapshot the
/// salient fields plus a fresh shallow clone of `d`. Local to
/// `ca_parse_line`; matches no C function.
#[allow(dead_code)]
fn clone_castate(s: &castate, d: &cadef) -> castate {
    castate {
        snext: None,
        d: Some(Box::new(clone_cadef_shallow(d))),
        nopts: s.nopts,
        def: s.def.clone(),
        ddef: s.ddef.clone(),
        curopt: s.curopt.clone(),
        dopt: s.dopt.clone(),
        opt: s.opt,
        arg: s.arg,
        argbeg: s.argbeg,
        optbeg: s.optbeg,
        nargbeg: s.nargbeg,
        restbeg: s.restbeg,
        curpos: s.curpos,
        argend: s.argend,
        inopt: s.inopt,
        inarg: s.inarg,
        nth: s.nth,
        singles: s.singles,
        oopt: s.oopt,
        actopts: s.actopts,
        args: s.args.clone(),
        oargs: s.oargs.clone(),
    }
}

#[allow(dead_code)]
fn clone_cadef_shallow(d: &cadef) -> cadef {
    cadef {
        next: None,
        snext: None,
        opts: d.opts.clone(),
        nopts: d.nopts,
        ndopts: d.ndopts,
        nodopts: d.nodopts,
        args: d.args.clone(),
        rest: d.rest.clone(),
        defs: d.defs.clone(),
        ndefs: d.ndefs,
        lastt: d.lastt,
        single: d.single.clone(),
        r#match: d.r#match.clone(),
        argsactive: d.argsactive,
        set: d.set.clone(),
        flags: d.flags,
        nonarg: d.nonarg.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for cd_get / cd_init / cd_sort / cd_prep removed — those
    // tests exercised the deleted CompDescItem/CompDescSet Rust-only
    // wrappers. The C-faithful entries (cd_get takes char**params and
    // returns int) get exercised through the full `_describe` widget
    // path under integration tests; per-fn unit tests would just
    // lock in the deleted Rust-side shape.

    // test_parse_caarg / test_parse_cadef removed — they exercised
    // the deleted CompArgDef/CompOptDef Rust-only types via fake-
    // signature wrappers. Real ports land alongside the cadef chain.

    #[test]
    fn test_rembslashcolon() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1054 — `\:` two-byte sequence drops the backslash.
        assert_eq!(rembslashcolon("a\\:b\\:c"), "a:b:c");
    }

    #[test]
    fn test_rembslashcolon_lone_backslash_kept() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1054 — `\X` (X != ':') keeps the backslash.
        assert_eq!(rembslashcolon("a\\nb"), "a\\nb");
    }

    #[test]
    fn test_rembslashcolon_trailing_backslash() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1054 — trailing `\` with no follow-up keeps the `\`.
        assert_eq!(rembslashcolon("a\\"), "a\\");
    }

    #[test]
    fn test_rembslashcolon_unescaped_colon_passes_through() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1054 — bare `:` (no preceding `\`) is kept.
        assert_eq!(rembslashcolon("a:b"), "a:b");
    }

    #[test]
    fn test_bslashcolon() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1073 — every `:` gets `\` prepended.
        assert_eq!(bslashcolon("a:b:c"), "a\\:b\\:c");
    }

    #[test]
    fn test_bslashcolon_no_colons() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1072 — non-colon bytes pass through unchanged.
        assert_eq!(bslashcolon("hello"), "hello");
    }

    #[test]
    fn test_bslashcolon_already_escaped_doubled() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1073-1074 — C doesn't track previous backslash, so an
        // already-escaped `\:` becomes `\\:` (the `\` passes
        // through, then the `:` gets a fresh `\` prepended).
        assert_eq!(bslashcolon("a\\:b"), "a\\\\:b");
    }

    #[test]
    fn test_single_index_dash_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1094 — `pre == '-'` → offset = -0x21.
        // For opt='a' (0x61): 0x61 + -0x21 = 0x40 = 64.
        assert_eq!(single_index(b'-', b'a'), 64);
        // For opt='A' (0x41): 0x41 + -0x21 = 0x20 = 32.
        assert_eq!(single_index(b'-', b'A'), 32);
        // For opt='!' (0x21): 0x21 + -0x21 = 0.
        assert_eq!(single_index(b'-', b'!'), 0);
        // For opt='~' (0x7e): 0x7e + -0x21 = 0x5d = 93.
        assert_eq!(single_index(b'-', b'~'), 93);
    }

    #[test]
    fn test_single_index_plus_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1094 — `pre == '+'` → offset = 94 - 0x21 = 61.
        // For opt='a' (0x61): 0x61 + 61 = 158.
        assert_eq!(single_index(b'+', b'a'), 158);
        // For opt='!' (0x21): 0x21 + 61 = 94.
        assert_eq!(single_index(b'+', b'!'), 94);
        // For opt='~' (0x7e): 0x7e + 61 = 187.
        assert_eq!(single_index(b'+', b'~'), 187);
    }

    #[test]
    fn test_single_index_out_of_range() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1091-1092 — opt <= 0x20 OR opt > 0x7e returns -1.
        assert_eq!(single_index(b'-', 0x20), -1); // space (0x20) excluded
        assert_eq!(single_index(b'-', 0x00), -1); // NUL
        assert_eq!(single_index(b'-', 0x7f), -1); // DEL (0x7f) excluded
        assert_eq!(single_index(b'+', 0xff), -1); // outside ASCII
    }

    // test_cd_group removed — used the deleted CompDescItem; the
    // function `cd_group` itself wasn't a real C export and was
    // also removed alongside the fake structs.

    #[test]
    fn caarg_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:949-962 — fresh caarg: every field zero / None.
        let a = caarg::default();
        assert!(a.next.is_none());
        assert!(a.descr.is_none());
        assert!(a.action.is_none());
        assert_eq!(a.r#type, 0);
        assert_eq!(a.num, 0);
        assert_eq!(a.active, 0);
    }

    #[test]
    fn caopt_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:928-939 — fresh caopt: zero / None across all fields.
        let o = caopt::default();
        assert!(o.next.is_none());
        assert!(o.name.is_none());
        assert!(o.args.is_none());
        assert_eq!(o.r#type, 0);
        assert_eq!(o.num, 0);
        assert_eq!(o.not, 0);
    }

    #[test]
    fn cadef_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:905-922 — fresh cadef: zero / None across all fields.
        let d = cadef::default();
        assert!(d.next.is_none());
        assert!(d.opts.is_none());
        assert!(d.args.is_none());
        assert_eq!(d.nopts, 0);
        assert_eq!(d.flags, 0);
    }

    #[test]
    fn freecaargs_walks_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:996-1010 — freecaargs walks `next` chain freeing each
        // entry. After call, the chain owner observes no remaining
        // refs (Drop handles deallocation).
        let mut head = caarg {
            descr: Some("a".into()),
            ..Default::default()
        };
        let mid = caarg {
            descr: Some("b".into()),
            ..Default::default()
        };
        let tail = caarg {
            descr: Some("c".into()),
            ..Default::default()
        };
        let mut mid_box = Box::new(mid);
        mid_box.next = Some(Box::new(tail));
        head.next = Some(mid_box);
        freecaargs(Some(Box::new(head)));
        // No panic, no leak — Box drop chains the rest.
    }

    #[test]
    fn cao_caa_constants_match_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:941-945 and c:964-968 — sequential 1..=5.
        assert_eq!(CAO_NEXT, 1);
        assert_eq!(CAO_DIRECT, 2);
        assert_eq!(CAO_ODIRECT, 3);
        assert_eq!(CAO_EQUAL, 4);
        assert_eq!(CAO_OEQUAL, 5);
        assert_eq!(CAA_NORMAL, 1);
        assert_eq!(CAA_OPT, 2);
        assert_eq!(CAA_REST, 3);
        assert_eq!(CAA_RARGS, 4);
        assert_eq!(CAA_RREST, 5);
    }

    #[test]
    fn cdf_max_cacache_constants_match_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:924 — CDF_SEP = 1; c:972 — MAX_CACACHE = 8.
        assert_eq!(CDF_SEP, 1);
        assert_eq!(MAX_CACACHE, 8);
    }

    #[test]
    fn crt_constants_match_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:79-83 — sequential 0..=4.
        assert_eq!(CRT_SIMPLE, 0);
        assert_eq!(CRT_DESC, 1);
        assert_eq!(CRT_SPEC, 2);
        assert_eq!(CRT_DUMMY, 3);
        assert_eq!(CRT_EXPL, 4);
    }

    #[test]
    fn cdstr_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:58-70 — fresh cdstr: zero/None across all fields.
        let s = cdstr::default();
        assert!(s.next.is_none());
        assert!(s.str.is_none());
        assert!(s.desc.is_none());
        assert!(s.r#match.is_none());
        assert_eq!(s.len, 0);
        assert_eq!(s.width, 0);
        assert_eq!(s.kind, 0);
    }

    #[test]
    fn cdrun_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:72-77 — fresh cdrun: zero/None.
        let r = cdrun::default();
        assert!(r.next.is_none());
        assert!(r.strs.is_none());
        assert_eq!(r.r#type, 0);
        assert_eq!(r.count, 0);
    }

    #[test]
    fn cdset_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:85-91 — fresh cdset: zero/None.
        let s = cdset::default();
        assert!(s.next.is_none());
        assert!(s.opts.is_none());
        assert!(s.strs.is_none());
        assert_eq!(s.count, 0);
        assert_eq!(s.desc, 0);
    }

    #[test]
    fn cdstate_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:40-56 — fresh cdstate: zero/None.
        let st = cdstate::default();
        assert_eq!(st.showd, 0);
        assert!(st.sep.is_none());
        assert!(st.sets.is_none());
        assert!(st.runs.is_none());
    }

    #[test]
    fn freecdsets_walks_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:96-122 — freecdsets walks `next` chain freeing each set
        // and its strs sub-chain.
        let head_str = cdstr {
            str: Some("foo".into()),
            desc: Some("first".into()),
            ..Default::default()
        };
        let tail_str = cdstr {
            str: Some("bar".into()),
            ..Default::default()
        };
        let mut head_str_b = Box::new(head_str);
        head_str_b.next = Some(Box::new(tail_str));
        let set = cdset {
            strs: Some(head_str_b),
            count: 2,
            ..Default::default()
        };
        freecdsets(Some(Box::new(set)));
        // No panic / no leak — Box drop chains the rest.
    }

    #[test]
    fn castate_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1928-1953 — fresh castate: zero/None.
        let s = castate::default();
        assert!(s.snext.is_none());
        assert!(s.d.is_none());
        assert!(s.def.is_none());
        assert!(s.args.is_none());
        assert_eq!(s.nopts, 0);
        assert_eq!(s.curpos, 0);
    }

    #[test]
    fn cvdef_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:2924-2935 — fresh cvdef: zero/None.
        let d = cvdef::default();
        assert!(d.descr.is_none());
        assert!(d.vals.is_none());
        assert_eq!(d.hassep, 0);
        assert_eq!(d.sep, 0);
        assert_eq!(d.argsep, 0);
    }

    #[test]
    fn cvval_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:2939-2947 — fresh cvval: zero/None.
        let v = cvval::default();
        assert!(v.next.is_none());
        assert!(v.name.is_none());
        assert!(v.arg.is_none());
        assert_eq!(v.r#type, 0);
        assert_eq!(v.active, 0);
    }

    #[test]
    fn cvstate_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3222-3227 — fresh cvstate: None across all 4 fields.
        let s = cvstate::default();
        assert!(s.d.is_none());
        assert!(s.def.is_none());
        assert!(s.val.is_none());
        assert!(s.vals.is_none());
    }

    #[test]
    fn ctags_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3737-3742 — fresh ctags: zero/None.
        let t = ctags::default();
        assert!(t.all.is_none());
        assert!(t.context.is_none());
        assert!(t.sets.is_none());
        assert_eq!(t.init, 0);
    }

    #[test]
    fn ctset_default_zero_initialized() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3746-3751 — fresh ctset: zero/None.
        let s = ctset::default();
        assert!(s.next.is_none());
        assert!(s.tags.is_none());
        assert!(s.tag.is_none());
        assert_eq!(s.ptr, 0);
    }

    #[test]
    fn cvv_constants_match_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:2949-2951 — sequential 0..=2.
        assert_eq!(CVV_NOARG, 0);
        assert_eq!(CVV_ARG, 1);
        assert_eq!(CVV_OPT, 2);
    }

    #[test]
    fn max_tags_cvcache_match_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3755 — MAX_TAGS = 256; c:2955 — MAX_CVCACHE = 8.
        assert_eq!(MAX_TAGS, 256);
        assert_eq!(MAX_CVCACHE, 8);
    }

    #[test]
    fn freectset_walks_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3762-3777 — freectset walks `next` chain freeing each
        // ctset's tags/tag fields.
        let mut head = ctset {
            tag: Some("foo".into()),
            ..Default::default()
        };
        let tail = ctset {
            tag: Some("bar".into()),
            ..Default::default()
        };
        head.next = Some(Box::new(tail));
        freectset(Some(Box::new(head)));
    }

    #[test]
    fn freectags_drops_one_node() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3779-3789 — freectags releases all/context/sets on one ctags.
        let t = ctags {
            all: Some(vec!["a".into(), "b".into()]),
            context: Some("ctx".into()),
            ..Default::default()
        };
        freectags(Some(Box::new(t)));
    }

    #[test]
    fn freecvdef_walks_vals_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:2960-2981 — freecvdef walks vals freeing each cvval.
        let v_tail = cvval {
            name: Some("opt2".into()),
            ..Default::default()
        };
        let mut v_head = cvval {
            name: Some("opt1".into()),
            ..Default::default()
        };
        v_head.next = Some(Box::new(v_tail));
        let d = cvdef {
            descr: Some("test".into()),
            vals: Some(Box::new(v_head)),
            ..Default::default()
        };
        freecvdef(Some(Box::new(d)));
    }

    /// c:1196 — `_arguments '-foo[only foo]' '*:file:_files'`. Verify
    /// that the option-name xor list contains the spec name, that
    /// nopts/ndopts reflect the option type (CAO_NEXT here), and that
    /// the rest arg lands on `rest` with type CAA_REST.
    #[test]
    fn parse_cadef_simple_opt_and_rest() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let args = vec![
            String::from(""), // adpre/adsuf split (no %d)
            String::from("-foo[only foo]"),
            String::from("*:file:_files"),
        ];
        let def = parse_cadef("_arguments", &args).expect("cadef built");
        let opt = def.opts.as_deref().expect("opt linked");
        assert_eq!(opt.name.as_deref(), Some("-foo"));
        assert_eq!(opt.descr.as_deref(), Some("only foo"));
        assert_eq!(opt.r#type, CAO_NEXT);
        // c:1462-1468 — non-multi option appends its own name to xor.
        let xor = opt.xor.as_ref().expect("xor list");
        assert!(
            xor.iter().any(|s| s == "-foo"),
            "xor must include -foo: {:?}",
            xor
        );

        let rest = def.rest.as_deref().expect("rest linked");
        assert_eq!(rest.r#type, CAA_REST);
        assert_eq!(rest.descr.as_deref(), Some("file"));
        assert_eq!(rest.action.as_deref(), Some("_files"));
    }

    /// c:1617-1661 — numbered positional argument `1:cmd:_commands` lands
    /// on `def.args` with the right slot (num=0 because anum is `1`
    /// then `arg->num = anum - 1`).
    #[test]
    fn parse_cadef_numbered_positional_arg() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let args = vec![String::from(""), String::from("1:cmd:_commands")];
        let def = parse_cadef("_arguments", &args).expect("cadef built");
        let pos = def.args.as_deref().expect("positional arg linked");
        assert_eq!(pos.num, 1);
        assert_eq!(pos.r#type, CAA_NORMAL);
        assert_eq!(pos.descr.as_deref(), Some("cmd"));
        assert_eq!(pos.action.as_deref(), Some("_commands"));
        assert_eq!(pos.direct, 1, "explicit numbering sets direct=1");
    }

    /// c:1647-1656 — duplicate numbered argument must error out and
    /// return None (the cadef cache miss path picks this up).
    #[test]
    fn parse_cadef_doubled_arg_errors() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let args = vec![
            String::from(""),
            String::from("1:a:_a"),
            String::from("1:b:_b"),
        ];
        let def = parse_cadef("_arguments", &args);
        assert!(def.is_none(), "duplicate arg num=1 must reject");
    }

    /// c:1335-1370 — `(opt-x opt-y)-foo[descr]` builds a 3-element
    /// xor list `[opt-x, opt-y, -foo]` (the option's own name gets
    /// added at the end via c:1462-1468).
    #[test]
    fn parse_cadef_xor_list_populated() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let args = vec![String::from(""), String::from("(opt-x opt-y)-foo[descr]")];
        let def = parse_cadef("_arguments", &args).expect("cadef built");
        let opt = def.opts.as_deref().expect("opt linked");
        let xor = opt.xor.as_ref().expect("xor list");
        assert_eq!(xor.len(), 3, "xor: {:?}", xor);
        assert_eq!(xor[0], "opt-x");
        assert_eq!(xor[1], "opt-y");
        assert_eq!(xor[2], "-foo");
    }

    /// c:3796 — `settags(0, ["ctx", "tag1", "tag2"])` populates
    /// `comptags[0]` with context="ctx", all=["tag1","tag2"], init=1.
    #[test]
    fn settags_populates_slot() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Clear slot to make test order-independent.
        if let Ok(mut tab) = comptags.lock() {
            tab[0] = None;
        }
        settags(
            0,
            &["ctx".to_string(), "tag-a".to_string(), "tag-b".to_string()],
        );
        let tab = comptags.lock().unwrap();
        let slot = tab[0].as_deref().expect("comptags[0] populated");
        assert_eq!(slot.context.as_deref(), Some("ctx"));
        assert_eq!(slot.init, 1);
        let all = slot.all.as_ref().expect("all populated");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], "tag-a");
        assert_eq!(all[1], "tag-b");
        assert!(slot.sets.is_none());
    }

    /// c:1712-1718 — exact name match returns the opt with `*end`
    /// pointing past the option name.
    #[test]
    fn ca_get_opt_exact_match() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let args = vec![String::from(""), String::from("-foo[d]")];
        let mut def = *parse_cadef("_arguments", &args).expect("cadef built");
        // Mark the only opt active so ca_get_opt accepts it.
        let mut cur = def.opts.as_deref_mut();
        while let Some(o) = cur {
            o.active = 1;
            cur = o.next.as_deref_mut();
        }
        let mut end: usize = 0;
        let hit = ca_get_opt(&def, "-foo", 1, &mut end).expect("hit");
        assert_eq!(hit.name.as_deref(), Some("-foo"));
        assert_eq!(end, 4);
    }

    /// c:1809-1822 — `argsactive=0` short-circuits to None even when
    /// args are linked. Guards against the easy off-by-one error of
    /// returning the first matching arg unconditionally.
    #[test]
    fn ca_get_arg_argsactive_zero_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let args = vec![String::from(""), String::from("1:c:_c")];
        let def = *parse_cadef("_arguments", &args).expect("cadef built");
        // argsactive defaults to 0 — must short-circuit.
        assert!(ca_get_arg(&def, 1).is_none());
    }

    /// c:1817 — when `argsactive=1` and the positional arg is active,
    /// `n` inside `[min, num]` returns the matching node.
    #[test]
    fn ca_get_arg_in_range_active() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let args = vec![String::from(""), String::from("1:c:_c")];
        let mut def = *parse_cadef("_arguments", &args).expect("cadef built");
        def.argsactive = 1;
        if let Some(a) = def.args.as_deref_mut() {
            a.active = 1;
        }
        let hit = ca_get_arg(&def, 1).expect("hit");
        assert_eq!(hit.num, 1);
        assert_eq!(hit.descr.as_deref(), Some("c"));
    }

    /// c:2999-3027 — `-s , descr opt1[a]:val1: opt2[b]` builds a cvdef
    /// with sep=',', descr="descr", vals chain of two cvvals.
    #[test]
    fn parse_cvdef_sep_and_two_vals() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let args = vec![
            String::from("-s"),
            String::from(","),
            String::from("descr"),
            String::from("opt1[a]:val1:"),
            String::from("opt2[b]"),
        ];
        let def = parse_cvdef("_values", &args).expect("cvdef built");
        assert_eq!(def.hassep, 1);
        assert_eq!(def.sep, b',' as i32);
        assert_eq!(def.descr.as_deref(), Some("descr"));
        let v1 = def.vals.as_deref().expect("val1");
        assert_eq!(v1.name.as_deref(), Some("opt1"));
        assert_eq!(v1.descr.as_deref(), Some("a"));
        assert_eq!(v1.r#type, CVV_ARG);
        let v2 = v1.next.as_deref().expect("val2");
        assert_eq!(v2.name.as_deref(), Some("opt2"));
        assert_eq!(v2.descr.as_deref(), Some("b"));
        assert_eq!(v2.r#type, CVV_NOARG);
    }

    /// c:1786-1801 — ca_foreign_opt walks snext skipping curset.
    /// When `curset == all` (head pointer matches), only the OTHER
    /// sets in the snext chain are scanned.
    #[test]
    fn ca_foreign_opt_finds_in_other_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let other = cadef {
            opts: Some(Box::new(caopt {
                name: Some("-bar".into()),
                active: 1,
                ..Default::default()
            })),
            ..Default::default()
        };
        let all = cadef {
            opts: Some(Box::new(caopt {
                name: Some("-foo".into()),
                active: 1,
                ..Default::default()
            })),
            snext: Some(Box::new(other)),
            ..Default::default()
        };
        // curset = &all (head). `-bar` lives in snext set — found.
        assert_eq!(ca_foreign_opt(&all, &all, "-bar"), 1);
        // `-foo` lives ONLY in the head (which gets skipped) — not found.
        assert_eq!(ca_foreign_opt(&all, &all, "-foo"), 0);
        // `-missing` not anywhere — not found.
        assert_eq!(ca_foreign_opt(&all, &all, "-missing"), 0);
    }

    /// c:1834 — guard: with neither xor nor opts AND no compcurrent
    /// position, the function returns without mutating any active flag.
    #[test]
    fn ca_inactive_guard_noop_keeps_active() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let mut d = cadef {
            opts: Some(Box::new(caopt {
                name: Some("-foo".into()),
                active: 1,
                ..Default::default()
            })),
            argsactive: 1,
            ..Default::default()
        };
        ca_inactive(&mut d, &[], 0, 0);
        assert_eq!(d.opts.as_deref().unwrap().active, 1);
        assert_eq!(d.argsactive, 1);
    }

    /// c:1881 — with `opts=1`, every option's `active` clears.
    #[test]
    fn ca_inactive_opts_flag_deactivates_options() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let saved_compcur = COMPCURRENT.load(Ordering::Relaxed);
        let mut d = cadef {
            opts: Some(Box::new(caopt {
                name: Some("-foo".into()),
                active: 1,
                num: 0,
                next: Some(Box::new(caopt {
                    name: Some("-bar".into()),
                    active: 1,
                    num: 1,
                    ..Default::default()
                })),
                ..Default::default()
            })),
            argsactive: 1,
            ..Default::default()
        };
        // Force COMPCURRENT >= cur so the guard at c:1834 is satisfied.
        COMPCURRENT.store(2, Ordering::Relaxed);
        ca_inactive(&mut d, &[], 1, 1);
        // Restore COMPCURRENT immediately so parallel non-ZLE tests
        // see the original value.
        COMPCURRENT.store(saved_compcur, Ordering::Relaxed);
        let mut p = d.opts.as_deref();
        while let Some(o) = p {
            assert_eq!(o.active, 0, "{:?} should be deactivated", o.name);
            p = o.next.as_deref();
        }
    }

    /// c:3798-3801 — `comptags -i 0 a b` populates comptags[0].
    /// Then `-T` reports empty sets (`1`), `-C` reads context.
    #[test]
    fn bin_comptags_init_and_context() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let saved_incompfunc = INCOMPFUNC.load(Ordering::Relaxed);
        let saved_locallevel = locallevel.load(Ordering::Relaxed);
        // Reset slot 0 + locallevel.
        if let Ok(mut tab) = comptags.lock() {
            tab[0] = None;
        }
        locallevel.store(0, Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);

        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        // -i my-ctx tag-a tag-b
        let r = bin_comptags(
            "comptags",
            &["-i".into(), "my-ctx".into(), "tag-a".into(), "tag-b".into()],
            &ops,
            0,
        );
        assert_eq!(r, 0);
        // -T returns 1 (no sets yet).
        let r = bin_comptags("comptags", &["-T".into()], &ops, 0);
        assert_eq!(r, 1);
        // comptags[0].context should be "my-ctx".
        let ctx = comptags.lock().unwrap()[0]
            .as_ref()
            .and_then(|t| t.context.clone());
        // Restore the globals before assertion-fail can leave them mutated.
        INCOMPFUNC.store(saved_incompfunc, Ordering::Relaxed);
        locallevel.store(saved_locallevel, Ordering::Relaxed);
        assert_eq!(ctx.as_deref(), Some("my-ctx"));
    }

    /// c:3178-3186 — cv_get_val finds a value by name; missing returns None.
    #[test]
    fn cv_get_val_hits_and_misses() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let d = cvdef {
            vals: Some(Box::new(cvval {
                name: Some("foo".into()),
                r#type: CVV_NOARG,
                next: Some(Box::new(cvval {
                    name: Some("bar".into()),
                    r#type: CVV_ARG,
                    ..Default::default()
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        let hit = cv_get_val(&d, "bar").expect("hit");
        assert_eq!(hit.name.as_deref(), Some("bar"));
        assert_eq!(hit.r#type, CVV_ARG);
        assert!(cv_get_val(&d, "missing").is_none());
    }

    /// c:5126-5131 — setup_ frees every cache slot and zeros
    /// lasttaglevel. Pre-fill all three caches + lasttaglevel, then
    /// call setup_ and verify they're cleared.
    #[test]
    fn setup_clears_all_caches() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Pre-fill.
        if let Ok(mut cache) = cadef_cache.lock() {
            cache[0] = Some(Box::new(cadef::default()));
        }
        if let Ok(mut cache) = cvdef_cache.lock() {
            cache[0] = Some(Box::new(cvdef::default()));
        }
        if let Ok(mut tab) = comptags.lock() {
            tab[0] = Some(Box::new(ctags::default()));
        }
        lasttaglevel.store(42, Ordering::Relaxed);

        let r = setup_();
        assert_eq!(r, 0);

        assert!(
            cadef_cache.lock().unwrap()[0].is_none(),
            "cadef_cache[0] should be cleared"
        );
        assert!(
            cvdef_cache.lock().unwrap()[0].is_none(),
            "cvdef_cache[0] should be cleared"
        );
        assert!(
            comptags.lock().unwrap()[0].is_none(),
            "comptags[0] should be cleared"
        );
        assert_eq!(
            lasttaglevel.load(Ordering::Relaxed),
            0,
            "lasttaglevel should reset to 0"
        );
    }

    /// c:5171-5177 — finish_ frees every slot in all three caches.
    /// Same pre-fill pattern as setup_clears_all_caches; finish_
    /// differs in not zeroing lasttaglevel.
    #[test]
    fn finish_frees_all_caches() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        if let Ok(mut cache) = cadef_cache.lock() {
            cache[1] = Some(Box::new(cadef::default()));
        }
        if let Ok(mut cache) = cvdef_cache.lock() {
            cache[1] = Some(Box::new(cvdef::default()));
        }
        if let Ok(mut tab) = comptags.lock() {
            tab[1] = Some(Box::new(ctags::default()));
        }
        let r = finish_();
        assert_eq!(r, 0);
        assert!(cadef_cache.lock().unwrap()[1].is_none());
        assert!(cvdef_cache.lock().unwrap()[1].is_none());
        assert!(comptags.lock().unwrap()[1].is_none());
    }

    /// c:4592-4593 — cfp_matcher_pats hits CMF_LEFT && lalen==0:
    /// return empty string immediately. Constructed Cmatcher chain
    /// has one matcher with CMF_LEFT flag and lalen=0, llen=1, wlen=1.
    /// We seed cmatcher_global so parse_cmatcher returns it via a
    /// matcher spec — but parse_cmatcher is complex, so this test
    /// instead constructs the chain directly and verifies the bail
    /// happens via the public surface (we exercise the same edge via
    /// a matcher spec parsed by parse_cmatcher that triggers the
    /// left-anchor zero-len path indirectly through the dispatcher).
    /// The simpler observable: a malformed/empty matcher spec is
    /// rejected and returns add untouched.
    #[test]
    fn cfp_matcher_pats_left_anchor_zero_lalen_returns_empty_via_invalid_spec() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        // A malformed matcher should fall through parse_cmatcher's
        // None return; cfp_matcher_pats then returns `add` unchanged
        // (the non-bail path; the CMF_LEFT-lalen0 bail requires a
        // successfully-parsed but pathological matcher which the
        // public parser does not produce).
        let r = cfp_matcher_pats("not-a-real-matcher-spec", "xyz");
        assert_eq!(r, "xyz");
    }

    /// c:3967 — bin_comptry with lasttaglevel == 0 (no -i call yet)
    /// errors with "no tags registered".
    #[test]
    fn bin_comptry_no_taglevel_errors() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let saved_incompfunc = INCOMPFUNC.load(Ordering::Relaxed);
        let saved_lvl = lasttaglevel.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        lasttaglevel.store(0, Ordering::Relaxed);
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_comptry("comptry", &["tag1".into()], &ops, 0);
        INCOMPFUNC.store(saved_incompfunc, Ordering::Relaxed);
        lasttaglevel.store(saved_lvl, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    /// c:4091-4134 — bin_comptry plain mode: filters args to registered
    /// tags not already in any set, then appends one set with the
    /// surviving tags.
    #[test]
    fn bin_comptry_plain_adds_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let saved_incompfunc = INCOMPFUNC.load(Ordering::Relaxed);
        let saved_lvl = lasttaglevel.load(Ordering::Relaxed);
        // Seed comptags[1] with two registered tags.
        if let Ok(mut tab) = comptags.lock() {
            tab[1] = Some(Box::new(ctags {
                all: Some(vec!["files".into(), "directories".into()]),
                context: Some("ctx".into()),
                init: 0,
                sets: None,
            }));
        }
        INCOMPFUNC.store(1, Ordering::Relaxed);
        lasttaglevel.store(1, Ordering::Relaxed);
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_comptry("comptry", &["files".into(), "unknown".into()], &ops, 0);
        // Inspect comptags[1].sets BEFORE restoring globals so a panic
        // doesn't leave them mutated.
        let sets_first_tags = comptags.lock().unwrap()[1]
            .as_ref()
            .and_then(|t| t.sets.as_ref().and_then(|s| s.tags.clone()));
        // Clean up.
        if let Ok(mut tab) = comptags.lock() {
            tab[1] = None;
        }
        INCOMPFUNC.store(saved_incompfunc, Ordering::Relaxed);
        lasttaglevel.store(saved_lvl, Ordering::Relaxed);
        assert_eq!(r, 0);
        let tags = sets_first_tags.expect("a set was appended");
        assert_eq!(
            tags,
            vec!["files".to_string()],
            "only registered tags survive the filter"
        );
    }

    /// c:4525 — cfp_matcher_pats returns add unchanged when matcher
    /// spec is empty (parse_cmatcher returns None).
    #[test]
    fn cfp_matcher_pats_empty_matcher_passthrough() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let r = cfp_matcher_pats("", "abc");
        assert_eq!(r, "abc");
    }

    /// c:4307 — cfp_matcher_range with no matchers (all None) emits
    /// each char verbatim.
    #[test]
    fn cfp_matcher_range_no_matchers_verbatim() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let ms: Vec<Option<Box<Cmatcher>>> = vec![None, None, None];
        let r = cfp_matcher_range(&ms, "abc");
        assert_eq!(r, "abc");
    }

    /// c:247-394 — cd_prep groups path emits CRT_EXPL + CRT_SPEC runs
    /// when cd_state.groups > 0. With two singleton kind=0+desc entries
    /// we expect: 2 CRT_SPEC runs (one per leader) followed by the
    /// CRT_EXPL header.
    #[test]
    fn cd_prep_groups_emits_expl_and_spec_runs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Seed cd_state: one set with two kind=0+desc entries.
        {
            let mut st = cd_state.lock().unwrap();
            st.showd = 0;
            st.groups = 2;
            st.descs = 2;
            st.maxg = 1;
            st.maxglen = 5;
            st.maxmlen = 100;
            st.sets = Some(Box::new(cdset {
                count: 2,
                desc: 2,
                strs: Some(Box::new(cdstr {
                    str: Some("alpha".into()),
                    r#match: Some("alpha".into()),
                    desc: Some("first".into()),
                    width: 5,
                    len: 5,
                    kind: 0,
                    next: Some(Box::new(cdstr {
                        str: Some("beta".into()),
                        r#match: Some("beta".into()),
                        desc: Some("second".into()),
                        width: 4,
                        len: 4,
                        kind: 0,
                        ..Default::default()
                    })),
                    ..Default::default()
                })),
                ..Default::default()
            }));
            st.runs = None;
        }
        let r = cd_prep();
        assert_eq!(r, 0);
        let st = cd_state.lock().unwrap();
        let r1 = st.runs.as_deref().expect("first run");
        assert_eq!(r1.r#type, CRT_SPEC, "first run is CRT_SPEC");
        let r2 = r1.next.as_deref().expect("second run");
        assert_eq!(r2.r#type, CRT_SPEC, "second run is CRT_SPEC");
        let r3 = r2.next.as_deref().expect("third run");
        assert_eq!(r3.r#type, CRT_EXPL, "third run is CRT_EXPL");
        assert_eq!(r3.count, 2, "CRT_EXPL covers both prep_lines");
    }

    /// c:4704-4732 — cfp_bld_pats combines names with skipped + pat.
    /// With one name "dir" and pats ["*.c"], produces ["dir*.c"].
    #[test]
    fn cfp_bld_pats_concatenates_skipped_and_pat() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let out = cfp_bld_pats(0, &["dir".to_string()], "", &["*.c".to_string()]);
        assert_eq!(out, vec!["dir*.c".to_string()]);
    }

    /// c:4711 — when GLOBDOTS is unset AND compprefix starts with `.`,
    /// add a dot-prefixed variant of each non-`.`-leading pattern.
    #[test]
    fn cfp_bld_pats_globdots_variant() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Seed COMPPREFIX with a leading dot.
        let m = COMPPREFIX.get_or_init(|| std::sync::Mutex::new(String::new()));
        *m.lock().unwrap() = ".foo".to_string();
        // Force GLOBDOTS unset — that's the default in zle_test_setup.
        let out = cfp_bld_pats(0, &["d".to_string()], "", &["*.x".to_string()]);
        // Reset compprefix to avoid bleed.
        *m.lock().unwrap() = String::new();
        assert!(out.contains(&"d*.x".to_string()));
        assert!(
            out.contains(&"d.*.x".to_string()),
            "dot-variant must be emitted: {:?}",
            out
        );
    }

    /// c:4625 — cfp_opt_pats with empty compprefix passes pats through.
    #[test]
    fn cfp_opt_pats_passthrough_empty_compprefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let m = COMPPREFIX.get_or_init(|| std::sync::Mutex::new(String::new()));
        *m.lock().unwrap() = String::new();
        let pats = vec!["*".to_string(), "*.c".to_string()];
        let out = cfp_opt_pats(&pats, "");
        assert_eq!(out, pats);
    }

    /// c:4175 — cfp_test_exact returns None when both compprefix and
    /// compsuffix are empty (no anchoring context).
    #[test]
    fn cfp_test_exact_no_anchor_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let pm = COMPPREFIX.get_or_init(|| std::sync::Mutex::new(String::new()));
        *pm.lock().unwrap() = String::new();
        let sm = COMPSUFFIX.get_or_init(|| std::sync::Mutex::new(String::new()));
        *sm.lock().unwrap() = String::new();
        let r = cfp_test_exact(&["/tmp".to_string()], &["true".to_string()], "");
        assert!(r.is_none());
    }

    /// c:5083 — bin_compgroups registers 6 group variants per name.
    /// Sanity test: returns 0 on the empty-args path (no-op).
    #[test]
    fn bin_compgroups_empty_args_succeeds() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let saved = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_compgroups("compgroups", &[], &ops, 0);
        INCOMPFUNC.store(saved, Ordering::Relaxed);
        assert_eq!(r, 0);
    }

    /// c:215-230 — cd_groups_want_sorting: returns 0 when ANY set's
    /// opts contains `-V`, 1 when `-J` (or default).
    #[test]
    fn cd_groups_want_sorting_respects_opts() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Default (no sets) → 1 (sorted).
        {
            let mut st = cd_state.lock().unwrap();
            st.sets = None;
        }
        assert_eq!(cd_groups_want_sorting(), 1);
        // Inject a set with -V option → returns 0.
        {
            let mut st = cd_state.lock().unwrap();
            st.sets = Some(Box::new(cdset {
                opts: Some(vec!["-V".into(), "grpname".into()]),
                ..Default::default()
            }));
        }
        assert_eq!(cd_groups_want_sorting(), 0);
        // Cleanup so other tests don't see the injected state.
        cd_state.lock().unwrap().sets = None;
    }

    /// c:233 — cd_sort compares Cdstr.sortstr lexically.
    #[test]
    fn cd_sort_orders_by_sortstr() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let a = cdstr {
            sortstr: Some("apple".into()),
            ..Default::default()
        };
        let b = cdstr {
            sortstr: Some("banana".into()),
            ..Default::default()
        };
        assert_eq!(cd_sort(&a, &b), std::cmp::Ordering::Less);
        assert_eq!(cd_sort(&b, &a), std::cmp::Ordering::Greater);
        assert_eq!(cd_sort(&a, &a), std::cmp::Ordering::Equal);
    }

    /// c:425-435 — cd_prep default branch: one CRT_SIMPLE run per
    /// non-empty set with the str chain mirrored via `.run` links.
    #[test]
    fn cd_prep_default_builds_simple_run_per_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Seed cd_state with 2 sets, each 1 match.
        {
            let mut st = cd_state.lock().unwrap();
            st.showd = 0;
            st.groups = 0;
            st.sets = Some(Box::new(cdset {
                count: 1,
                strs: Some(Box::new(cdstr {
                    str: Some("a".into()),
                    r#match: Some("a".into()),
                    ..Default::default()
                })),
                next: Some(Box::new(cdset {
                    count: 1,
                    strs: Some(Box::new(cdstr {
                        str: Some("b".into()),
                        r#match: Some("b".into()),
                        ..Default::default()
                    })),
                    ..Default::default()
                })),
                ..Default::default()
            }));
            st.runs = None;
        }
        let r = cd_prep();
        assert_eq!(r, 0);
        // Two CRT_SIMPLE runs, one per set.
        let st = cd_state.lock().unwrap();
        let r1 = st.runs.as_deref().expect("first run");
        assert_eq!(r1.r#type, CRT_SIMPLE);
        assert_eq!(r1.count, 1);
        let r2 = r1.next.as_deref().expect("second run");
        assert_eq!(r2.r#type, CRT_SIMPLE);
        assert_eq!(r2.count, 1);
        assert!(r2.next.is_none(), "no third run");
    }

    /// c:846-895 — bin_compdescribe with invalid option returns 1.
    #[test]
    fn bin_compdescribe_rejects_bad_option() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let saved_incompfunc = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        // -xx is two chars but ends with `x` not a known subcommand
        // letter — should fall through to the `invalid option` arm.
        let r = bin_compdescribe("compdescribe", &["-x".into()], &ops, 0);
        INCOMPFUNC.store(saved_incompfunc, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    /// c:4903-4923 — cf_remove_other with pre="dir/foo" returns
    /// only names starting with "dir/" and clears `amb`.
    #[test]
    fn cf_remove_other_filters_by_dir_head() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let names = vec![
            "dir/a".to_string(),
            "dir/b".to_string(),
            "other/c".to_string(),
        ];
        let mut amb = 99;
        let ret = cf_remove_other(&names, "dir/foo", &mut amb);
        assert_eq!(amb, 0);
        let v = ret.expect("matching names returned");
        assert_eq!(v, vec!["dir/a".to_string(), "dir/b".to_string()]);
    }

    /// c:4942-4951 — pre without '/' and names diverge → `amb=1`,
    /// returns None.
    #[test]
    fn cf_remove_other_no_slash_diverge_sets_amb() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let names = vec!["a".to_string(), "b".to_string()];
        let mut amb = 0;
        let ret = cf_remove_other(&names, "x", &mut amb);
        assert!(ret.is_none());
        assert_eq!(amb, 1);
    }

    /// c:4870 — no "parent" and no "pwd" in style → cf_ignore returns
    /// without touching `ign`.
    #[test]
    fn cf_ignore_no_style_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut ign: Vec<String> = vec!["x".into()];
        cf_ignore(&["/tmp".into()], &mut ign, "", "/tmp/foo");
        assert_eq!(
            ign,
            vec!["x".to_string()],
            "no style match must leave ign untouched"
        );
    }

    /// c:3691 — empty compqstack short-circuits bin_compquote to 0.
    #[test]
    fn bin_compquote_returns_zero_when_qstack_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let saved_incompfunc = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        // Ensure compqstack is empty (zle_test_setup resets things).
        if let Some(m) = COMPQSTACK.get() {
            if let Ok(mut s) = m.lock() {
                s.clear();
            }
        }
        let ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_compquote("compquote", &["foo".into()], &ops, 0);
        INCOMPFUNC.store(saved_incompfunc, Ordering::Relaxed);
        assert_eq!(r, 0);
    }

    /// c:3192-3203 — cv_quote_get_val unquotes input then delegates
    /// to cv_get_val. Quoted name with backslash should still match
    /// after parse_subst_string strips the quoting.
    #[test]
    fn cv_quote_get_val_unquotes_then_lookup() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        inittyptab();
        let d = cvdef {
            vals: Some(Box::new(cvval {
                name: Some("foo".into()),
                r#type: CVV_NOARG,
                ..Default::default()
            })),
            ..Default::default()
        };
        // Plain name → hit.
        assert!(cv_quote_get_val(&d, "foo").is_some());
        // Unknown → miss.
        assert!(cv_quote_get_val(&d, "bar").is_none());
    }

    /// c:3211-3217 — cv_inactive clears active for each name in xor.
    #[test]
    fn cv_inactive_clears_named_vals() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut d = cvdef {
            vals: Some(Box::new(cvval {
                name: Some("a".into()),
                active: 1,
                next: Some(Box::new(cvval {
                    name: Some("b".into()),
                    active: 1,
                    next: Some(Box::new(cvval {
                        name: Some("c".into()),
                        active: 1,
                        ..Default::default()
                    })),
                    ..Default::default()
                })),
                ..Default::default()
            })),
            ..Default::default()
        };
        cv_inactive(&mut d, &["a".into(), "c".into()]);
        let mut p = d.vals.as_deref();
        let mut by_name = std::collections::HashMap::new();
        while let Some(v) = p {
            by_name.insert(v.name.clone().unwrap_or_default(), v.active);
            p = v.next.as_deref();
        }
        assert_eq!(by_name["a"], 0);
        assert_eq!(by_name["b"], 1, "untouched val stays active");
        assert_eq!(by_name["c"], 0);
    }

    // ─── zsh-corpus pins for cd_arrcat / cd_arrdup ──────────────────

    /// `cd_arrcat` concatenates two arrays.
    #[test]
    fn computil_corpus_cd_arrcat_concatenates() {
        let a = vec!["a".to_string(), "b".to_string()];
        let b = vec!["c".to_string(), "d".to_string()];
        let r = cd_arrcat(&a, &b);
        assert_eq!(r, vec!["a", "b", "c", "d"]);
    }

    /// `cd_arrcat` with empty left side returns copy of right side.
    #[test]
    fn computil_corpus_cd_arrcat_empty_left() {
        let a: Vec<String> = Vec::new();
        let b = vec!["x".to_string(), "y".to_string()];
        let r = cd_arrcat(&a, &b);
        assert_eq!(r, vec!["x", "y"]);
    }

    /// `cd_arrcat` with empty right side returns copy of left side.
    #[test]
    fn computil_corpus_cd_arrcat_empty_right() {
        let a = vec!["p".to_string(), "q".to_string()];
        let b: Vec<String> = Vec::new();
        let r = cd_arrcat(&a, &b);
        assert_eq!(r, vec!["p", "q"]);
    }

    /// `cd_arrcat` with both empty returns empty.
    #[test]
    fn computil_corpus_cd_arrcat_both_empty() {
        let a: Vec<String> = Vec::new();
        let b: Vec<String> = Vec::new();
        let r = cd_arrcat(&a, &b);
        assert!(r.is_empty());
    }

    /// `cd_arrdup` is identity for valid input.
    #[test]
    fn computil_corpus_cd_arrdup_round_trips() {
        let a = vec!["one".to_string(), "two".to_string(), "three".to_string()];
        let r = cd_arrdup(&a);
        assert_eq!(r, a);
    }

    /// `cd_arrdup` on empty returns empty.
    #[test]
    fn computil_corpus_cd_arrdup_empty() {
        let a: Vec<String> = Vec::new();
        let r = cd_arrdup(&a);
        assert!(r.is_empty());
    }

    /// `cd_arrdup` returns a deep copy (modifying result doesn't
    /// affect input).
    #[test]
    fn computil_corpus_cd_arrdup_independent_copy() {
        let a = vec!["alpha".to_string()];
        let mut r = cd_arrdup(&a);
        r[0].push('!');
        assert_eq!(a[0], "alpha", "original unchanged");
        assert_eq!(r[0], "alpha!", "copy mutated independently");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/computil.c
    // c:172 freecdsets / c:372 cd_groups_want_sorting / c:396 cd_sort /
    // c:1725 arrcmp / c:1754 freecaargs / c:1773 freecadef /
    // c:1821 rembslashcolon / c:1858 bslashcolon / c:1890 single_index /
    // c:3540 ca_opt_arg
    // ═══════════════════════════════════════════════════════════════════

    /// c:172 — `freecdsets(None)` is safe idempotent.
    #[test]
    fn freecdsets_none_idempotent() {
        for _ in 0..5 {
            freecdsets(None);
        }
    }

    /// c:1754 — `freecaargs(None)` is safe idempotent.
    #[test]
    fn freecaargs_none_idempotent() {
        for _ in 0..5 {
            freecaargs(None);
        }
    }

    /// c:1773 — `freecadef(None)` is safe idempotent.
    #[test]
    fn freecadef_none_idempotent() {
        for _ in 0..5 {
            freecadef(None);
        }
    }

    /// c:980 — `arrcmp(None, None)` returns 1 ("equal" sentinel, NOT 0).
    /// C uses inverted boolean: 1 = equal, 0 = unequal (opposite of strcmp).
    #[test]
    fn arrcmp_both_none_returns_one_inverted_boolean() {
        assert_eq!(
            arrcmp(None, None),
            1,
            "C inverted-boolean: 1=equal, 0=unequal (per c:980)"
        );
    }

    /// c:980 — `arrcmp(None, Some)` returns 0 (unequal sentinel).
    #[test]
    fn arrcmp_none_vs_some_returns_zero_unequal() {
        let a = vec!["x".to_string()];
        assert_eq!(arrcmp(None, Some(&a)), 0, "None vs Some → 0 (unequal)");
        assert_eq!(arrcmp(Some(&a), None), 0, "Some vs None → 0 (unequal)");
    }

    /// c:984 — `arrcmp(Some(a), Some(a))` returns 1 (equal).
    #[test]
    fn arrcmp_identical_arrays_returns_one_equal() {
        let a = vec!["x".to_string(), "y".to_string()];
        assert_eq!(
            arrcmp(Some(&a), Some(&a)),
            1,
            "identical arrays → 1 (equal) per c:989"
        );
    }

    /// c:1725 — `arrcmp` is pure for non-empty arrays.
    #[test]
    fn arrcmp_is_pure() {
        let a = vec!["x".to_string()];
        let b = vec!["y".to_string()];
        let first = arrcmp(Some(&a), Some(&b));
        for _ in 0..5 {
            assert_eq!(arrcmp(Some(&a), Some(&b)), first, "arrcmp must be pure");
        }
    }

    /// c:1821 — `rembslashcolon("")` empty returns empty.
    #[test]
    fn rembslashcolon_empty_returns_empty() {
        assert_eq!(rembslashcolon(""), "");
    }

    /// c:1821 — `rembslashcolon` is pure.
    #[test]
    fn rembslashcolon_is_pure() {
        for s in ["", "abc", r"a\:b", r"\:\:"] {
            let first = rembslashcolon(s);
            for _ in 0..3 {
                assert_eq!(
                    rembslashcolon(s),
                    first,
                    "rembslashcolon({:?}) must be pure",
                    s
                );
            }
        }
    }

    /// c:1858 — `bslashcolon("")` empty returns empty.
    #[test]
    fn bslashcolon_empty_returns_empty() {
        assert_eq!(bslashcolon(""), "");
    }

    /// c:1890 — `single_index(0, 0)` returns i32 (type pin).
    #[test]
    fn single_index_returns_i32_type() {
        let _: i32 = single_index(0, 0);
    }

    /// c:3540 — `ca_opt_arg(empty, empty, false)` is safe.
    #[test]
    fn ca_opt_arg_empty_inputs_no_panic() {
        let _: String = ca_opt_arg("", "", false);
    }

    /// c:1858 — `bslashcolon` is pure.
    #[test]
    fn bslashcolon_is_pure() {
        for s in ["", "a:b", "no colon", "a:b:c:d"] {
            let first = bslashcolon(s);
            for _ in 0..3 {
                assert_eq!(bslashcolon(s), first, "bslashcolon({:?}) must be pure", s);
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/computil.c
    // c:933 cd_arrcat / c:1153 cd_arrdup / c:1167 cd_get /
    // c:1509 bin_compdescribe / c:1754 freecaargs / c:1773 freecadef /
    // c:1821 rembslashcolon / c:1858 bslashcolon / c:2884 get_cadef
    // ═══════════════════════════════════════════════════════════════════

    /// c:933 — `cd_arrcat(&[], &[])` empty + empty returns empty.
    #[test]
    fn cd_arrcat_both_empty_returns_empty() {
        assert!(cd_arrcat(&[], &[]).is_empty(), "empty + empty → empty");
    }

    /// c:933 — `cd_arrcat(&[], &b)` empty-A returns clone of B.
    #[test]
    fn cd_arrcat_empty_a_returns_clone_of_b() {
        let b = vec!["x".to_string(), "y".to_string()];
        assert_eq!(cd_arrcat(&[], &b), b, "empty + B → B");
    }

    /// c:933 — `cd_arrcat(&a, &[])` empty-B returns clone of A.
    #[test]
    fn cd_arrcat_empty_b_returns_clone_of_a() {
        let a = vec!["m".to_string(), "n".to_string()];
        assert_eq!(cd_arrcat(&a, &[]), a, "A + empty → A");
    }

    /// c:933 — `cd_arrcat` preserves order: A first, then B.
    #[test]
    fn cd_arrcat_preserves_a_then_b_order() {
        let a = vec!["1".to_string(), "2".to_string()];
        let b = vec!["3".to_string(), "4".to_string()];
        let r = cd_arrcat(&a, &b);
        assert_eq!(r, vec!["1", "2", "3", "4"]);
    }

    /// c:933 — `cd_arrcat` returns Vec<String> (compile-time type pin).
    #[test]
    fn cd_arrcat_returns_vec_string_type() {
        let _: Vec<String> = cd_arrcat(&[], &[]);
    }

    /// c:1153 — `cd_arrdup` is identity on the input (deep clone).
    #[test]
    fn cd_arrdup_returns_independent_clone() {
        let a = vec!["x".to_string(), "y".to_string()];
        let dup = cd_arrdup(&a);
        assert_eq!(dup, a, "dup equal to input");
        let mut mut_dup = dup;
        mut_dup.push("added".to_string());
        assert_eq!(a.len(), 2, "original unchanged after mutating dup");
    }

    /// c:1153 — `cd_arrdup(&[])` empty returns empty.
    #[test]
    fn cd_arrdup_empty_returns_empty() {
        assert!(cd_arrdup(&[]).is_empty());
    }

    /// c:1167 — `cd_get(&[])` empty args returns i32 (type pin).
    #[test]
    fn cd_get_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cd_get(&[]);
    }

    /// c:1509 — `bin_compdescribe` returns i32 (compile-time type pin).
    #[test]
    fn bin_compdescribe_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_compdescribe("compdescribe", &[], &ops, 0);
    }

    /// c:1754 — `freecaargs(None)` is safe.
    #[test]
    fn freecaargs_none_no_panic() {
        freecaargs(None);
    }

    /// c:1773 — `freecadef(None)` is safe.
    #[test]
    fn freecadef_none_no_panic() {
        freecadef(None);
    }

    /// c:1821 — `rembslashcolon` returns String (compile-time type pin).
    #[test]
    fn rembslashcolon_returns_string_type() {
        let _: String = rembslashcolon("");
    }

    /// c:1858 — `bslashcolon` returns String (compile-time type pin).
    #[test]
    fn bslashcolon_returns_string_type() {
        let _: String = bslashcolon("");
    }

    /// c:1821 + c:1858 — `rembslashcolon(bslashcolon(s))` round-trips
    /// for inputs without pre-existing escaped colons.
    #[test]
    fn bslashcolon_rembslashcolon_roundtrip_safe() {
        for s in ["a:b", "x:y:z", "no_colon"] {
            let escaped = bslashcolon(s);
            let unescaped = rembslashcolon(&escaped);
            assert_eq!(
                unescaped, s,
                "bslashcolon→rembslashcolon must round-trip for {:?}",
                s
            );
        }
    }

    /// c:2884 — `get_cadef("", &[])` empty inputs returns i32.
    #[test]
    fn get_cadef_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = get_cadef("", &[]);
    }
}
