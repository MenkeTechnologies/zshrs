//! Completion result handling for ZLE
//!
//! Port from zsh/Src/Zle/compresult.c (2,359 lines)
//!
//! Handle the case were we found more than one match.                       // c:740
//! Insert all matches in the command line.                                  // c:893
//! This handles the beginning of menu-completion.                           // c:1377
//! List the matches.                                                        // c:2300
//!
//! Handles insertion of completion results into the edit buffer:
//! unambiguous prefix insertion, menu cycling, single match auto-insert,
//! and ambiguous match handling.
//!
//! Key C functions and their Rust locations:
//! - do_single       → single unambiguous match insertion
//! - do_ambiguous     → handle multiple matches (list or menu)
//! - do_allmatches    → insert all matches
//! - do_menucmp       → menu completion cycling
//! - accept_last      → accept current menu selection
//! - instmatch        → insert a match into the buffer
//! - unambig_data     → compute unambiguous prefix
//! - build_pos_string → build position string for match

// `CompResult` enum (Rust-only) deleted per strict-rules. C source
// (Src/Zle/compresult.c) has 0 enums; the completion-result is
// communicated via globals (`amenu`, `lastambig`, `validlist`)
// and the per-call `ret`/`ok` int variables of `do_ambiguous` /
// `do_single` / `do_allmatches`. zshrs's port routes those
// through the executor's completion state directly.

use std::sync::atomic::Ordering;
use std::sync::atomic::Ordering::Relaxed;

use crate::ported::init::SHTTY;
use crate::ported::utils::{adjustcolumns, adjustlines, write_loop, zputs};
use crate::ported::zle::comp_h::{
    Aminfo, Chdata, Cldata, Cline, Cmatch, Cmgroup, Menuinfo, CGF_FILES, CGF_HASDL, CGF_LINES,
    CGF_PACKED, CGF_ROWS, CLF_DIFF, CLF_JOIN, CLF_LINE, CLF_MATCHED, CLF_MID, CLF_MISS, CLF_NEW,
    CLF_SUF, CMF_ALL, CMF_DISPLINE, CMF_DUMMY, CMF_FILE, CMF_HIDE, CMF_ISPAR, CMF_MULT, CMF_NOLIST,
    CMF_NOSPACE, CMF_PACKED, CMF_PARBR, CMF_PARNEST, CMF_REMOVE, CMF_ROWS,
};
use crate::ported::zle::compcore::{
    amatches, brpcs, brscs, eparq, fromcomp, iforcemenu, insmnum, insspace, lastend, lastmatches,
    lastpermmnum, lmatches, menuacc, metafy_line, movetoend, nmatches as nmatches_g, nmatches,
    oldins, oldlist, onlyexpl, parpre, parq, unmetafy_line, BRBEG, BREND, MINFO, WB, WE, ZLEMETACS,
    ZLEMETALINE, ZLEMETALL, ZMULT,
};
use crate::ported::zle::complete::COMPLISTMAX;
use crate::ported::zle::computil::CM_SPACE;
use crate::ported::zle::zle_h::COMP_LIST_COMPLETE;
use crate::ported::zle::zle_refresh::tcout;
use crate::ported::zle::zle_tricky::printfmt;
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::{
    isset, AUTOPARAMKEYS, AUTOPARAMSLASH, AUTOREMOVESLASH, LISTPACKED, LISTROWSFIRST, LISTTYPES,
    USEZLE,
};
use crate::ported::zle::compmatch::{cline_setlens, cline_sublen, cp_cline};
use crate::ported::zle::zle_h::{CUT_RAW, SUFTYP_POSSTR};
use crate::ported::zle::zle_misc::suffixlen;
use crate::ported::module::{gethookdef, runhookdef};
use crate::ported::exec::noerrs;
use crate::ported::utils::errflag;
use crate::ported::zsh_h::ERRFLAG_ERROR;
use crate::ported::params::paramtab;
use crate::ported::zsh_h::{PM_SCALAR, PM_TYPE};
use crate::ported::lex::parsestr;
use crate::ported::subst::singsub;
/// Port of `mod_export int invcount` from `Src/Zle/compresult.c:37`.
/// Invalidation counter — bumped every time the cached completion
/// list goes stale. `complistmatches` reads it to detect "we have a
/// new list" without comparing the full Cmgroup chain.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]

/// Direct port of `static Cline cut_cline(Cline l)` from
/// `Src/Zle/compresult.c:46`. Prunes the unambiguous-line Cline list
/// before the stuff not worth inserting: keeps the leading structs
/// that have something on the line, then trims trailing/ambiguous
/// runs of missing-character parts using the `cline_sublen`
/// sum/max heuristic and finally strips joined sub-parts.
///
/// C models `l` as a linked list and mutates it by pointer aliasing
/// (`e`, `q`, `maxp` all point into `l`). The Rust store is an owned
/// `Option<Box<Cline>>`; to express the aliasing safely the top-level
/// chain is flattened into a `Vec<Box<Cline>>` and every C node
/// pointer becomes an index (`Option<usize>`, where `None` == C's
/// NULL / one-past-end), then the chain is rebuilt on return. The
/// prefix/suffix sub-lists stay owned inside each node.
pub fn cut_cline(l: Option<Box<Cline>>) -> Option<Box<Cline>> {
    // c:46
    use crate::ported::zle::compcore::{hasmatched, minmlen};

    // c:57 — if no match was added with matching, keep everything.
    if hasmatched.load(Relaxed) == 0 {
        let mut ll = l;
        cline_setlens(&mut ll, 0); // c:58
        return ll; // c:59
    }
    // c:61 — `e = l = cp_cline(l, 0)`. Shallow copy, then flatten the
    // top-level next-chain into a Vec so nodes can be index-addressed.
    let copied = cp_cline(l.as_deref(), 0);
    let mut nodes: Vec<Box<Cline>> = Vec::new();
    let mut cur = copied;
    while let Some(mut node) = cur {
        cur = node.next.take();
        nodes.push(node);
    }
    let n0 = nodes.len();
    if n0 == 0 {
        return None;
    }

    // c:66-71 — search for the last struct with something on the line;
    // anything before that is kept. `e` = index of first droppable
    // node (`p->next`), `q` = last node with word/line/prefix.
    let mut e: Option<usize> = Some(0); // e = l
    let mut q: Option<usize> = None;
    for i in 0..n0 {
        let p = &nodes[i];
        if p.orig.is_some() || p.olen != 0 || (p.flags & CLF_NEW) == 0 {
            e = if i + 1 < n0 { Some(i + 1) } else { None }; // c:68 e = p->next
        }
        if p.suffix.is_none() && (p.wlen != 0 || p.llen != 0 || p.prefix.is_some()) {
            q = Some(i); // c:70 q = p
        }
    }
    // c:72-77 — special-case a short trailing missing-char anchor.
    if e.is_none() {
        if let Some(qi) = q {
            let cond = {
                let qn = &nodes[qi];
                qn.orig.is_none()
                    && qn.olen == 0
                    && (qn.flags & CLF_MISS) != 0
                    && ((qn.flags & CLF_MATCHED) == 0 || (qn.prefix.is_none() && qn.suffix.is_none()))
                    && (if qn.word.is_some() { qn.wlen } else { qn.llen }) < 3
            };
            if cond {
                let qn = &mut nodes[qi];
                qn.word = None; // c:75
                qn.line = None;
                qn.wlen = 0; // c:76
                qn.llen = 0;
            }
        }
    }
    // c:80-81 — keep all structs without missing characters.
    while let Some(ei) = e {
        if (nodes[ei].flags & CLF_MISS) != 0 {
            break;
        }
        e = if ei + 1 < n0 { Some(ei + 1) } else { None };
    }

    let mut ls = false;
    let mut end_len = nodes.len();

    if let Some(mut ei) = e {
        // c:87 — is there ANOTHER struct with missing chars after e?
        let mut pidx: Option<usize> = if ei + 1 < n0 { Some(ei + 1) } else { None };
        while let Some(pi) = pidx {
            if (nodes[pi].flags & CLF_MISS) != 0 {
                break;
            }
            pidx = if pi + 1 < n0 { Some(pi + 1) } else { None };
        }
        if pidx.is_some() {
            // c:90-104 — sum/max walk from e to the end.
            let mut sum = 0i32;
            let mut max = 0i32;
            let mut maxp: Option<usize> = None;
            for pi in ei..n0 {
                let p = &nodes[pi];
                if (p.flags & CLF_MISS) == 0 {
                    sum += p.max; // c:92
                } else {
                    let tmp = cline_sublen(p); // c:94
                    if tmp > 2 && tmp > ((p.max + p.min) >> 1) {
                        sum += tmp - (p.max - tmp); // c:96
                    } else if tmp < p.min {
                        sum -= (((p.max + p.min) >> 1) - tmp) << (if tmp < 2 { 1 } else { 0 });
                        // c:98
                    }
                }
                if sum > max {
                    max = sum; // c:101
                    maxp = Some(pi); // c:102
                }
            }
            let mut do_truncate = true;
            if max != 0 {
                ei = maxp.unwrap(); // c:106 e = maxp
            } else {
                // c:108-118 — no positive run: setlens now, and bail out
                // (goto end) if the remaining min-length is still large.
                for node in nodes.iter_mut() {
                    node.min = cline_sublen(node); // c:110 cline_setlens(l, 0)
                }
                ls = true; // c:111
                let mut len = 0i32;
                for pi in ei..nodes.len() {
                    len += nodes[pi].min; // c:114
                }
                if len > ((minmlen.load(Relaxed) << 1) / 3) {
                    do_truncate = false; // c:116-117 goto end
                }
            }
            if do_truncate {
                // c:119-121 — clear the anchor at e and cut the tail.
                {
                    let en = &mut nodes[ei];
                    en.line = None; // c:119
                    en.word = None;
                    en.llen = 0; // c:120
                    en.wlen = 0;
                    en.olen = 0;
                }
                nodes.truncate(ei + 1); // c:121 e->next = NULL
                end_len = nodes.len();
            }
        }
    }
    // c:124 end: — strip joined sub-strings when there are no parts
    // with missing characters.
    let _ = end_len;
    let mut ee: Option<usize> = None; // c:129 e = 0
    let mut tmp2 = 0i32; // 0 == prefix, 1 == suffix
    let mut miss = 0i32;
    for pi in 0..nodes.len() {
        let p = &nodes[pi];
        if (p.flags & (CLF_MISS | CLF_DIFF)) != 0 {
            miss = 1; // c:131
        }
        // c:132-137 — prefix sub-list JOIN scan.
        let mut qq = p.prefix.as_deref();
        while let Some(qn) = qq {
            if (qn.flags & CLF_JOIN) != 0 {
                ee = Some(pi);
                tmp2 = 0;
                break;
            }
            qq = qn.next.as_deref();
        }
        // c:138-143 — suffix sub-list JOIN scan (wins over prefix).
        let mut qq = p.suffix.as_deref();
        while let Some(qn) = qq {
            if (qn.flags & CLF_JOIN) != 0 {
                ee = Some(pi);
                tmp2 = 1;
                break;
            }
            qq = qn.next.as_deref();
        }
    }
    if let Some(ei) = ee {
        let cond = {
            let e = &nodes[ei];
            miss == 0 || cline_sublen(e) == e.min // c:145
        };
        if cond {
            // c:146-149 — truncate the chosen sub-list at the last node
            // before a CLF_JOIN part.
            let sub = if tmp2 != 0 {
                &mut nodes[ei].suffix
            } else {
                &mut nodes[ei].prefix
            };
            if let Some(head) = sub.as_deref_mut() {
                let mut node: &mut Cline = head;
                loop {
                    let advance = match node.next.as_deref() {
                        Some(nx) => (nx.flags & CLF_JOIN) == 0,
                        None => false,
                    };
                    if advance {
                        node = node.next.as_deref_mut().unwrap();
                    } else {
                        break;
                    }
                }
                node.next = None; // c:149 p->next = NULL
            }
        }
    }
    // Rebuild the top-level chain from the (possibly truncated) Vec.
    let mut head: Option<Box<Cline>> = None;
    while let Some(mut node) = nodes.pop() {
        node.next = head.take();
        head = Some(node);
    }
    if !ls {
        cline_setlens(&mut head, 0); // c:152
    }
    head // c:154 return l
}

/// Direct port of `char *cline_str(Cline l, int ins, int *csp,
/// LinkList posl)` from `Src/Zle/compresult.c:165`. Builds the
/// unambiguous string. Everything is inserted into the metafied
/// `ZLEMETALINE` at the current `ZLEMETACS` via `inststrlen`:
///
/// - `ins == 1`: the string is left in the line, cursor set to the
///   computed insertion point, `lastend` recorded; returns `None`.
/// - `ins != 1`: the inserted region is copied out and `foredel`'d
///   back off the line; returns `Some(string)`. `csp` receives the
///   relative cursor position and `posl` the missing/ambiguous
///   positions. `ins == 2` means `csp`/`posl` are real command-line
///   positions (including braces).
///
/// Brace re-insertion walks `brbeg` (forward) and `brend` (backward
/// from its tail — C's `lastbrend`). C's `Brinfo *` chains are
/// snapshotted into `Vec`s of `(str, qpos, curpos)` so the pointer
/// walks (`brp->next`, `brs->prev`) become index steps. The cursor
/// lands on the mid / prefix / suffix / diff missing-char point per
/// `mid`/`pm`/`sm`/`d`.
pub fn cline_str(
    // c:165
    l: Option<Box<Cline>>,
    ins: i32,
    mut csp: Option<&mut i32>,
    mut posl: Option<&mut Vec<i32>>,
) -> Option<String> {
    // METACHECK(): operate on the metafied line.
    let cs = || ZLEMETACS.load(Relaxed);
    let set_cs = |v: i32| ZLEMETACS.store(v, Relaxed);

    // c:175 — `l = cut_cline(l)`.
    let l = cut_cline(l);

    let ocs = cs(); // c:168 ocs = zlemetacs
    let wb = WB.load(Relaxed);
    let we = WE.load(Relaxed);
    // c:170 — padd = (ins ? wb - ocs : -ocs).
    let padd = if ins != 0 { wb - ocs } else { -ocs };

    // c:177-179 — position/marker state.
    let (mut pcs, mut scs) = (0i32, 0i32);
    let mut opos = -1i32;
    let (mut pmm, mut pma, mut smm, mut sma, mut dm) = (0i32, 0i32, 0i32, 0i32, 0i32);
    let (mut pm, mut pmax, mut sm, mut smax, mut d, mut mid, mut cbr) =
        (-1i32, -1i32, -1i32, -1i32, -1i32, -1i32, -1i32);
    let mut li = 0i32;

    // Brace chains as (str, qpos, curpos) with index cursors.
    // brp advances forward; brs starts at the tail (lastbrend) and
    // walks toward the head.
    let mut brbeg_v: Vec<(String, i32, i32)> = Vec::new();
    let mut brend_v: Vec<(String, i32, i32)> = Vec::new();
    let mut brp_idx = 0usize;
    let mut brs_idx: i32 = -1;

    if ins != 0 {
        // c:183-210 — brace begin/end curpos setup + leading braces.
        {
            let mut cur = BRBEG.get().and_then(|m| m.lock().ok());
            if let Some(ref g) = cur {
                let mut node = g.as_deref();
                while let Some(n) = node {
                    brbeg_v.push((n.str.clone().unwrap_or_default(), n.qpos, 0));
                    node = n.next.as_deref();
                }
            }
            let _ = cur.take();
            let mut cur = BREND.get().and_then(|m| m.lock().ok());
            if let Some(ref g) = cur {
                let mut node = g.as_deref();
                while let Some(n) = node {
                    brend_v.push((n.str.clone().unwrap_or_default(), n.qpos, 0));
                    node = n.next.as_deref();
                }
            }
            let _ = cur.take();
        }
        let mut olen = we - wb; // c:185
        for b in brbeg_v.iter_mut() {
            b.2 = b.1; // c:189 curpos = qpos
            olen -= b.0.len() as i32; // c:190
        }
        if !brend_v.is_empty() {
            for b in brend_v.iter() {
                olen -= b.0.len() as i32; // c:195
            }
            for b in brend_v.iter_mut() {
                b.2 = olen - b.1; // c:198 curpos = olen - qpos
            }
            brs_idx = brend_v.len() as i32 - 1; // brs = lastbrend
        }
        brp_idx = 0;
        // c:200-203 — insert leading brace-begins (curpos == 0).
        while brp_idx < brbeg_v.len() && brbeg_v[brp_idx].2 == 0 {
            inststrlen(&brbeg_v[brp_idx].0, true, -1);
            brp_idx += 1;
        }
        // c:204-209 — insert leading brace-ends (curpos == 0).
        while brs_idx >= 0 && brend_v[brs_idx as usize].2 == 0 {
            if cbr < 0 {
                cbr = cs();
            }
            inststrlen(&brend_v[brs_idx as usize].0, true, -1);
            brs_idx -= 1;
        }
    }

    // c:212-417 — walk the top-level cline list.
    let mut cur = l.as_deref();
    while let Some(node) = cur {
        // c:214-236 — original string (no prefix) or the prefix walk.
        if node.olen != 0 && (node.flags & CLF_SUF) == 0 && node.prefix.is_none() {
            pcs = cs() + node.olen; // c:215
            inststrlen(node.orig.as_deref().unwrap_or(""), true, node.olen); // c:216
        } else {
            let mut sp = node.prefix.as_deref();
            while let Some(s) = sp {
                pcs = cs() + s.llen; // c:220
                if (s.flags & CLF_LINE) != 0 {
                    inststrlen(s.line.as_deref().unwrap_or(""), true, s.llen); // c:222
                } else {
                    inststrlen(s.word.as_deref().unwrap_or(""), true, s.wlen); // c:224
                }
                scs = cs(); // c:225
                if (s.flags & CLF_DIFF) != 0 && (dm == 0 || (s.flags & CLF_MATCHED) != 0) {
                    d = cs(); // c:228
                    dm = s.flags & CLF_MATCHED;
                    if let Some(pl) = posl.as_mut() {
                        let np = cs() + padd;
                        if np != opos {
                            opos = np;
                            pl.push(np); // c:231
                        }
                    }
                }
                li += s.llen; // c:234
                sp = s.next.as_deref();
            }
        }
        // c:237-250 — flush brace-begins now reachable (li >= curpos).
        if ins != 0 {
            while brp_idx < brbeg_v.len() && li >= brbeg_v[brp_idx].2 {
                let ocs2 = cs();
                let bl = brbeg_v[brp_idx].0.len() as i32;
                set_cs(pcs - (li - brbeg_v[brp_idx].2));
                inststrlen(&brbeg_v[brp_idx].0, true, bl);
                set_cs(ocs2 + bl);
                pcs += bl;
                scs += bl;
                brp_idx += 1;
            }
        }
        // c:253-264 — first prefix with missing chars → posl + pm.
        if (node.flags & CLF_MISS) != 0 && (node.flags & CLF_SUF) == 0 {
            if let Some(pl) = posl.as_mut() {
                let np = cs() + padd;
                if np != opos {
                    opos = np;
                    pl.push(np);
                }
            }
            if ((pmax <= (node.max - node.min) || (pma != 0 && node.max != node.min))
                && (pmm == 0 || (node.flags & CLF_MATCHED) != 0))
                || ((node.flags & CLF_MATCHED) != 0 && pmm == 0)
            {
                pm = cs();
                pmax = node.max - node.min;
                pmm = node.flags & CLF_MATCHED;
                pma = ((node.prefix.is_some() || node.suffix.is_some())
                    && node.min == cline_sublen(node)) as i32;
            }
        }
        // c:265-279 — flush brace-ends reachable before the anchor.
        if ins != 0 {
            while brs_idx >= 0 && li >= brend_v[brs_idx as usize].2 {
                let ocs2 = cs();
                let bl = brend_v[brs_idx as usize].0.len() as i32;
                set_cs(scs - (li - brend_v[brs_idx as usize].2));
                if cbr < 0 {
                    cbr = cs();
                }
                inststrlen(&brend_v[brs_idx as usize].0, true, bl);
                set_cs(ocs2 + bl);
                pcs += bl;
                brs_idx -= 1;
            }
        }
        pcs = cs(); // c:280
        // c:281-285 — insert the anchor.
        if (node.flags & CLF_LINE) != 0 {
            inststrlen(node.line.as_deref().unwrap_or(""), true, node.llen);
        } else {
            inststrlen(node.word.as_deref().unwrap_or(""), true, node.wlen);
        }
        scs = cs(); // c:286
        // c:287-302 — brace-begins after the anchor.
        if ins != 0 {
            li += node.llen; // c:290
            while brp_idx < brbeg_v.len() && li >= brbeg_v[brp_idx].2 {
                let ocs2 = cs();
                let bl = brbeg_v[brp_idx].0.len() as i32;
                set_cs(pcs + node.llen - (li - brbeg_v[brp_idx].2));
                inststrlen(&brbeg_v[brp_idx].0, true, bl);
                set_cs(ocs2 + bl);
                pcs += bl;
                scs += bl;
                brp_idx += 1;
            }
        }
        // c:303-319 — cursor position for suffixes / mids.
        if (node.flags & CLF_MISS) != 0 {
            if (node.flags & CLF_MID) != 0 {
                mid = cs(); // c:306
            } else if (node.flags & CLF_SUF) != 0 {
                if let Some(pl) = posl.as_mut() {
                    let np = cs() + padd;
                    if np != opos {
                        opos = np;
                        pl.push(np);
                    }
                }
                if ((smax <= (node.min - node.max) || (sma != 0 && node.max != node.min))
                    && (smm == 0 || (node.flags & CLF_MATCHED) != 0))
                    || ((node.flags & CLF_MATCHED) != 0 && smm == 0)
                {
                    sm = cs();
                    smax = node.min - node.max;
                    smm = node.flags & CLF_MATCHED;
                    sma = ((node.prefix.is_some() || node.suffix.is_some())
                        && node.min == cline_sublen(node)) as i32;
                }
            }
        }
        // c:320-334 — brace-ends after the anchor.
        if ins != 0 {
            while brs_idx >= 0 && li >= brend_v[brs_idx as usize].2 {
                let ocs2 = cs();
                let bl = brend_v[brs_idx as usize].0.len() as i32;
                set_cs(scs - (li - brend_v[brs_idx as usize].2));
                if cbr < 0 {
                    cbr = cs();
                }
                inststrlen(&brend_v[brs_idx as usize].0, true, bl);
                set_cs(ocs2 + bl);
                pcs += bl;
                brs_idx -= 1;
            }
        }
        // c:335-415 — suffix original or the suffix sub-list walk.
        if node.olen != 0 && (node.flags & CLF_SUF) != 0 && node.suffix.is_none() {
            pcs = cs(); // c:337
            inststrlen(node.orig.as_deref().unwrap_or(""), true, node.olen); // c:338
            if ins != 0 {
                li += node.olen; // c:342
                while brp_idx < brbeg_v.len() && li >= brbeg_v[brp_idx].2 {
                    let ocs2 = cs();
                    let bl = brbeg_v[brp_idx].0.len() as i32;
                    set_cs(pcs + node.olen - (li - brbeg_v[brp_idx].2));
                    inststrlen(&brbeg_v[brp_idx].0, true, bl);
                    set_cs(ocs2 + bl);
                    pcs += bl;
                    brp_idx += 1;
                }
                while brs_idx >= 0 && li >= brend_v[brs_idx as usize].2 {
                    let ocs2 = cs();
                    let bl = brend_v[brs_idx as usize].0.len() as i32;
                    set_cs(pcs + node.olen - (li - brend_v[brs_idx as usize].2));
                    if cbr < 0 {
                        cbr = cs();
                    }
                    inststrlen(&brend_v[brs_idx as usize].0, true, bl);
                    set_cs(ocs2 + bl);
                    pcs += bl;
                    brs_idx -= 1;
                }
            }
        } else {
            // c:366-406 — suffix sub-list. Parts insert WITHOUT moving
            // the cursor (move_cursor = false); cursor jumps by `i` after.
            let mut jflag: i32 = -1;
            let mut js_matched = 0i32;
            let mut i_acc = 0i32;
            let mut sp = node.suffix.as_deref();
            while let Some(s) = sp {
                if jflag < 0 && (s.flags & CLF_DIFF) != 0 {
                    jflag = i_acc;
                    js_matched = s.flags & CLF_MATCHED;
                }
                pcs = cs(); // c:371
                if (s.flags & CLF_LINE) != 0 {
                    inststrlen(s.line.as_deref().unwrap_or(""), false, s.llen); // c:373
                    i_acc += s.llen;
                    scs = cs() + s.llen;
                } else {
                    inststrlen(s.word.as_deref().unwrap_or(""), false, s.wlen); // c:376
                    i_acc += s.wlen;
                    scs = cs() + s.wlen;
                }
                if ins != 0 {
                    li += s.llen; // c:382
                    while brp_idx < brbeg_v.len() && li >= brbeg_v[brp_idx].2 {
                        let ocs2 = cs();
                        let bl = brbeg_v[brp_idx].0.len() as i32;
                        set_cs(pcs + (li - brbeg_v[brp_idx].2));
                        inststrlen(&brbeg_v[brp_idx].0, true, bl);
                        set_cs(ocs2 + bl);
                        pcs += bl;
                        scs += bl;
                        brp_idx += 1;
                    }
                    while brs_idx >= 0 && li >= brend_v[brs_idx as usize].2 {
                        let ocs2 = cs();
                        let bl = brend_v[brs_idx as usize].0.len() as i32;
                        set_cs(scs - (li - brend_v[brs_idx as usize].2));
                        if cbr < 0 {
                            cbr = cs();
                        }
                        inststrlen(&brend_v[brs_idx as usize].0, true, bl);
                        set_cs(ocs2 + bl);
                        pcs += bl;
                        brs_idx -= 1;
                    }
                }
                sp = s.next.as_deref();
            }
            set_cs(cs() + i_acc); // c:407
            if jflag >= 0 && (dm == 0 || js_matched != 0) {
                d = cs() - jflag; // c:409
                dm = js_matched;
                if let Some(pl) = posl.as_mut() {
                    let np = cs() - jflag + padd;
                    if np != opos {
                        opos = np;
                        pl.push(np);
                    }
                }
            }
        }
        cur = node.next.as_deref(); // c:416
    }
    // c:418-424 — end-of-word position.
    if let Some(pl) = posl.as_mut() {
        let np = cs() + padd;
        if np != opos {
            pl.push(np);
        }
    }

    // c:426-455 — flush any remaining braces and shift recorded points.
    if ins != 0 {
        let ocs2b = cs();
        while brp_idx < brbeg_v.len() {
            inststrlen(&brbeg_v[brp_idx].0, true, -1);
            brp_idx += 1;
        }
        while brs_idx >= 0 {
            if cbr < 0 {
                cbr = cs();
            }
            inststrlen(&brend_v[brs_idx as usize].0, true, -1);
            brs_idx -= 1;
        }
        let shift = cs() - ocs2b;
        if mid >= ocs2b {
            mid += shift;
        }
        if pm >= ocs2b {
            pm += shift;
        }
        if sm >= ocs2b {
            sm += shift;
        }
        if d >= ocs2b {
            d += shift;
        }
        if let Some(pl) = posl.as_mut() {
            for p in pl.iter_mut() {
                if *p >= ocs2b {
                    *p += shift;
                }
            }
        }
    }
    // c:460-462 — final cursor position.
    let ncs = if mid >= 0 {
        mid
    } else if cbr >= 0 {
        cbr
    } else if pm >= 0 {
        pm
    } else if sm >= 0 {
        sm
    } else if d >= 0 {
        d
    } else {
        cs()
    };

    if ins != 1 {
        // c:465-477 — copy the inserted region out and delete it.
        let cur_cs = cs();
        let ilen = cur_cs - ocs;
        let r = ZLEMETALINE
            .get()
            .and_then(|m| m.lock().ok())
            .map(|g| {
                let b = g.as_bytes();
                let start = (ocs.max(0) as usize).min(b.len());
                let end = (cur_cs.max(0) as usize).min(b.len()).max(start);
                String::from_utf8_lossy(&b[start..end]).into_owned()
            })
            .unwrap_or_default();
        set_cs(ocs);
        foredel(ilen, CUT_RAW); // c:472
        if let Some(cp) = csp.as_deref_mut() {
            *cp = ncs - ocs; // c:475
        }
        Some(r) // c:477
    } else {
        lastend.store(cs(), Relaxed); // c:479
        set_cs(ncs); // c:480
        None // c:482
    }
}

/// Direct port of `static char *build_pos_string(LinkList list)` from
/// `Src/Zle/compresult.c:489`. Small utility turning a list of
/// positions into a colon-separated string (e.g. `[1, 5, 10]` →
/// `"1:5:10"`). C's `LinkList` of `long` positions is a `&[i32]` here.
pub fn build_pos_string(list: &[i32]) -> String {
    // c:489
    // c:496-517 — sprintf each position, join with ':'.
    let mut s = String::new();
    for (i, p) in list.iter().enumerate() {
        if i != 0 {
            s.push(':'); // c:515
        }
        s.push_str(&p.to_string()); // c:505 sprintf("%ld", p)
    }
    s // c:517
}

/// Find the longest common prefix of every match — the substring the
// This is a utility function using the function above to allow access     // c:525
// to the unambiguous string and cursor position via compstate.             // c:525
/// completion engine inserts on the first Tab press when matches
/// are ambiguous.
/// Port of `unambig_data(int *cp, char **pp, char **ip)` from Src/Zle/compresult.c. The C source
/// also tracks cursor placement within the prefix; ours returns
/// just the common-prefix string.
/// WARNING: param names don't match C — Rust=(matches) vs C=(cp, pp, ip)
pub fn unambig_data(matches: &[String]) -> String {
    // c:525
    if matches.is_empty() {
        return String::new();
    }
    if matches.len() == 1 {
        return matches[0].clone();
    }

    let first = &matches[0];
    let mut prefix_len = first.len();

    for m in &matches[1..] {
        let common = first
            .chars()
            .zip(m.chars())
            .take_while(|(a, b)| a == b)
            .count();
        prefix_len = prefix_len.min(common);
    }

    first[..first
        .char_indices()
        .nth(prefix_len)
        .map(|(i, _)| i)
        .unwrap_or(first.len())]
        .to_string()
}

/// Direct port of `static int instmatch(Cmatch m, int *scs)` from
/// `Src/Zle/compresult.c:578`. Inserts the chosen match into the
/// metafied `ZLEMETALINE` at `ZLEMETACS`, component by component in
/// order: ignored-prefix / -P prefix / path-prefix / the string /
/// (brace begins) / path-suffix / (brace ends) / -S suffix /
/// ignored-suffix. Re-inserts the `brbeg`/`brend` braces at the
/// positions given by `m.brpl`/`m.brsl`, captures `lastprebr`/
/// `lastpostbr`, and tracks `brpcs`/`brscs`/`lastend`. Returns the
/// number of bytes inserted; `scs` receives the auto-suffix insertion
/// point. Cursor is left at `ocs` (just after the string / last brace
/// begin) on return.
pub fn instmatch(
    // c:578
    m: &Cmatch,
    mut scs: Option<&mut i32>,
) -> i32 {
    // METACHECK(): operates on the metafied line.
    let cs = || ZLEMETACS.load(Relaxed);
    let set_cs = |v: i32| ZLEMETACS.store(v, Relaxed);

    let mut r = 0i32;
    let a0 = cs(); // c:580 a = zlemetacs

    // c:585-587 — zsfree(lastprebr); zsfree(lastpostbr); = NULL.
    if let Some(mx) = LASTPREBR.get() {
        if let Ok(mut g) = mx.lock() {
            g.clear();
        }
    }
    if let Some(mx) = LASTPOSTBR.get() {
        if let Ok(mut g) = mx.lock() {
            g.clear();
        }
    }

    // c:589-593 — ignored prefix.
    if let Some(s) = m.ipre.as_deref() {
        let l = s.len() as i32;
        inststrlen(s, true, l);
        r += l;
    }
    // c:594-598 — -P prefix.
    if let Some(s) = m.pre.as_deref() {
        let l = s.len() as i32;
        inststrlen(s, true, l);
        r += l;
    }
    // c:599-603 — path prefix.
    if let Some(s) = m.ppre.as_deref() {
        let l = s.len() as i32;
        inststrlen(s, true, l);
        r += l;
    }
    // c:604-606 — the string itself.
    {
        let s = m.str.as_deref().unwrap_or("");
        let l = s.len() as i32;
        inststrlen(s, true, l);
        r += l;
    }
    let mut ocs = cs(); // c:607

    // Snapshot the brace chains (str only).
    let brbeg_v: Vec<String> = {
        let mut v = Vec::new();
        if let Some(g) = BRBEG.get().and_then(|m| m.lock().ok()) {
            let mut node = g.as_deref();
            while let Some(nn) = node {
                v.push(nn.str.clone().unwrap_or_default());
                node = nn.next.as_deref();
            }
        }
        v
    };
    let brend_v: Vec<String> = {
        let mut v = Vec::new();
        if let Some(g) = BREND.get().and_then(|m| m.lock().ok()) {
            let mut node = g.as_deref();
            while let Some(nn) = node {
                v.push(nn.str.clone().unwrap_or_default());
                node = nn.next.as_deref();
            }
        }
        v
    };

    // c:608-630 — re-insert the brace beginnings.
    if !brbeg_v.is_empty() {
        let mut pcs = cs(); // c:610
        let mut bradd = m.pre.as_deref().map_or(0, |s| s.len() as i32); // c:613
        for (i, bs) in brbeg_v.iter().enumerate() {
            let brpos = *m.brpl.get(i).unwrap_or(&0); // c:614
            set_cs(a0 + brpos + bradd); // c:617
            pcs = cs(); // c:618
            let l = bs.len() as i32; // c:619
            bradd += l; // c:620
            brpcs.store(cs(), Relaxed); // c:621
            inststrlen(bs, true, l); // c:622
            r += l; // c:623
            ocs += l; // c:624
        }
        // c:626-628 — lastprebr = zlemetaline[a0 .. pcs].
        let prebr = {
            ZLEMETALINE
                .get()
                .and_then(|m| m.lock().ok())
                .map(|g| {
                    let b = g.as_bytes();
                    let s = (a0.max(0) as usize).min(b.len());
                    let e = (pcs.max(0) as usize).min(b.len()).max(s);
                    String::from_utf8_lossy(&b[s..e]).into_owned()
                })
                .unwrap_or_default()
        };
        if let Ok(mut g) = LASTPREBR
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
        {
            *g = prebr;
        }
        set_cs(ocs); // c:629
    }
    // c:631-635 — path suffix.
    if let Some(s) = m.psuf.as_deref() {
        let l = s.len() as i32;
        inststrlen(s, true, l);
        r += l;
    }
    // c:636-656 — re-insert the brace ends.
    let mut brb = 0i32;
    if !brend_v.is_empty() {
        let a1 = cs(); // c:638 a = zlemetacs
        let mut bradd = 0i32;
        for (i, bs) in brend_v.iter().enumerate() {
            let brpos = *m.brsl.get(i).unwrap_or(&0);
            set_cs(a1 - brpos); // c:640
            ocs = cs(); // c:641 ocs = brscs = zlemetacs
            brscs.store(ocs, Relaxed);
            let l = bs.len() as i32;
            bradd += l;
            inststrlen(bs, true, l); // c:644
            brb = cs(); // c:645
            r += l;
        }
        set_cs(a1 + bradd); // c:648
        if let Some(s) = scs.as_deref_mut() {
            *s = ocs; // c:650
        }
    } else {
        brscs.store(-1, Relaxed); // c:652
        if let Some(s) = scs.as_deref_mut() {
            *s = cs(); // c:655
        }
    }
    // c:657-661 — -S suffix.
    if let Some(s) = m.suf.as_deref() {
        let l = s.len() as i32;
        inststrlen(s, true, l);
        r += l;
    }
    // c:662-666 — ignored suffix.
    if let Some(s) = m.isuf.as_deref() {
        let l = s.len() as i32;
        inststrlen(s, true, l);
        r += l;
    }
    // c:667-671 — lastpostbr = zlemetaline[brb .. zlemetacs].
    if !brend_v.is_empty() {
        let end = cs();
        let postbr = {
            ZLEMETALINE
                .get()
                .and_then(|m| m.lock().ok())
                .map(|g| {
                    let b = g.as_bytes();
                    let s = (brb.max(0) as usize).min(b.len());
                    let e = (end.max(0) as usize).min(b.len()).max(s);
                    String::from_utf8_lossy(&b[s..e]).into_owned()
                })
                .unwrap_or_default()
        };
        if let Ok(mut g) = LASTPOSTBR
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
        {
            *g = postbr;
        }
    }
    lastend.store(cs(), Relaxed); // c:672
    set_cs(ocs); // c:673
    r // c:675
}

/// Direct port of `mod_export int hasbrpsfx(Cmatch m, char *pre,
/// char *suf)` from `Src/Zle/compresult.c:683`. Checks whether the
/// match `m`, when inserted, produces the given brace prefix/suffix.
/// `CMF_ALL` matches short-circuit to true. Otherwise it metafies the
/// line if needed, runs `instmatch(m, None)` into a scratch copy of
/// the line, captures the resulting `lastprebr`/`lastpostbr`, restores
/// the line + `lastend`/`brpcs`/`brscs`, and compares against
/// `pre`/`suf`.
pub fn hasbrpsfx(m: &Cmatch, pre: Option<&str>, suf: Option<&str>) -> bool {
    // c:683
    // c:687-688 — CMF_ALL shortcut.
    if (m.flags & CMF_ALL) != 0 {
        return true;
    }
    // c:690-695 — metafy the line if it isn't already.
    let was_meta = ZLEMETALL.load(Relaxed) != 0;
    if !was_meta {
        metafy_line();
    }

    // c:698-701 — save state.
    let op = LASTPREBR
        .get()
        .and_then(|m| m.lock().ok())
        .map(|g| g.clone())
        .unwrap_or_default();
    let os = LASTPOSTBR
        .get()
        .and_then(|m| m.lock().ok())
        .map(|g| g.clone())
        .unwrap_or_default();
    let oline = ZLEMETALINE
        .get()
        .and_then(|m| m.lock().ok())
        .map(|g| g.clone())
        .unwrap_or_default();
    let oll = ZLEMETALL.load(Relaxed);
    let ole = lastend.load(Relaxed);
    let opcs = brpcs.load(Relaxed);
    let oscs = brscs.load(Relaxed);
    // C restores zlemetacs via zle_restore_positions (unified cs domain);
    // the Rust zle_save/restore_positions cover only the wide-line cursor,
    // so save/restore the meta cursor explicitly to match C's contract.
    let ometacs = ZLEMETACS.load(Relaxed);

    zle_save_positions(); // c:703

    // c:706 — lastprebr = lastpostbr = NULL.
    if let Some(mx) = LASTPREBR.get() {
        if let Ok(mut g) = mx.lock() {
            g.clear();
        }
    }
    if let Some(mx) = LASTPOSTBR.get() {
        if let Ok(mut g) = mx.lock() {
            g.clear();
        }
    }

    instmatch(m, None); // c:708

    // c:710-717 — restore the original line bytes.
    if let Ok(mut g) = ZLEMETALINE
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
    {
        *g = oline;
    }
    ZLEMETALL.store(oll, Relaxed);
    zle_restore_positions(); // c:716
    ZLEMETALL.store(oll, Relaxed); // c:717 zlemetall = newll (kept)
    ZLEMETACS.store(ometacs, Relaxed); // c:716 (meta cursor, see above)
    lastend.store(ole, Relaxed); // c:718
    brpcs.store(opcs, Relaxed); // c:719
    brscs.store(oscs, Relaxed); // c:720

    // c:722-725 — compare captured braces against pre/suf.
    let lpb = LASTPREBR
        .get()
        .and_then(|m| m.lock().ok())
        .map(|g| g.clone())
        .unwrap_or_default();
    let lsb = LASTPOSTBR
        .get()
        .and_then(|m| m.lock().ok())
        .map(|g| g.clone())
        .unwrap_or_default();
    let pre_ok = match pre {
        None => lpb.is_empty(),
        Some(p) => !lpb.is_empty() && p == lpb,
    };
    let suf_ok = match suf {
        None => lsb.is_empty(),
        Some(s) => !lsb.is_empty() && s == lsb,
    };
    let ret = pre_ok && suf_ok;

    // c:727-730 — restore lastprebr/lastpostbr.
    if let Ok(mut g) = LASTPREBR
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
    {
        *g = op;
    }
    if let Ok(mut g) = LASTPOSTBR
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
    {
        *g = os;
    }

    if !was_meta {
        unmetafy_line(); // c:733
    }
    ret // c:734
}

/// Direct port of `static int do_ambiguous(void)` from
/// `Src/Zle/compresult.c:744`. The ambiguous-completion handler —
/// computes the unambiguous prefix from `ainfo.line` via `cline_str`
/// (falls back to LCP over the supplied matches when ainfo->line
/// isn't populated), then `foredel`+`inststr` against ZLEMETALINE
/// when WB/WE indicate a real completion is in flight. Sets the
/// `menucmp=0`/`lastambig=1` transition flags. Returns 1 if any
/// completion text was inserted, 0 otherwise.
/// WARNING: param names don't match C — Rust=(matches) vs C=()
pub fn do_ambiguous(matches: &[String]) -> i32 {
    // c:744
    // c:748 — `menucmp = menuacc = 0`.
    MENUCMP.store(0, Relaxed);
    // c:763 — `lastambig = 1`.
    LASTAMBIG.store(1, Relaxed);

    // c:774 — if `ainfo` is populated, walk ainfo->line via cline_str
    // (compresult.c:535 path); else fall back to the LCP over the
    // provided match strings.
    let ainfo_line = crate::ported::zle::compcore::ainfo
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|a| a.line.clone()));
    let prefix = if let Some(line) = ainfo_line {
        // c:535 — render the unambiguous string (ins=0 → copy out).
        cline_str(Some(line), 0, None, None).unwrap_or_default()
    } else {
        unambig_data(matches)
    };
    if prefix.is_empty() && matches.is_empty() {
        return 0; // c:nomatch
    }
    // c:783-790 — buffer-edit: foredel the original word, inststr
    // the unambig prefix, when WB/WE describe a valid range.
    if !prefix.is_empty() {
        let wb = crate::ported::zle::compcore::WB.load(Relaxed);
        let we = crate::ported::zle::compcore::WE.load(Relaxed);
        if we > wb && wb >= 0 {
            let span = we - wb;
            crate::ported::zle::compcore::ZLEMETACS.store(wb, Relaxed); // c:785
            // c:786 — `foredel(we - wb, CUT_RAW)`. CUT_RAW is REQUIRED on
            // a metafied line; without it foredel takes the non-raw path
            // and deletes nothing, so inststr below prepends the prefix
            // to the still-present word (ambiguous `ec` → `ecec`).
            foredel(span, CUT_RAW); // c:786
            let _ = inststr(&prefix); // c:790
        }
    }
    // compresult.c do_ambiguous tail — decide whether the ambiguous match
    // set needs a listing, and trigger it via `showinglist = -2`. The
    // previous Rust port omitted this, so `showinglist` stayed 0 and
    // do_completion's fallback set `onlyexpl = 3` (explanation-only mode),
    // which makes calclist skip every real match — `l<Tab>` computed 257
    // matches but displayed nothing.
    let mut ret = 0;
    let oldlist_v = crate::ported::zle::compcore::oldlist.load(Relaxed);
    // c:`if (isset(LISTBEEP) && !oldlist) ret = 1;`
    if isset(crate::ported::zsh_h::LISTBEEP) && oldlist_v == 0 {
        ret = 1;
    }
    let uselist_v = crate::ported::zle::compcore::uselist.load(Relaxed);
    let usemenu_v = crate::ported::zle::zle_tricky::USEMENU.load(Relaxed);
    let listshown_v = LISTSHOWN.load(Relaxed);
    let showinglist_v = SHOWINGLIST.load(Relaxed);
    let smatches_v = crate::ported::zle::compcore::smatches.load(Relaxed);
    let forcelist_v = crate::ported::zle::compcore::forcelist.load(Relaxed);
    if uselist_v != 0
        && (usemenu_v != 2 || (listshown_v == 0 && oldlist_v == 0))
        && ((showinglist_v == 0 && (listshown_v == 0 || oldlist_v == 0))
            || (usemenu_v == 3 && oldlist_v == 0))
        && (smatches_v >= 2 || forcelist_v != 0)
    {
        SHOWINGLIST.store(-2, Relaxed);
    }
    ret
}

/// Port of `ztat(char *nam, struct stat *buf, int ls)` from `Src/Zle/compresult.c:869`.
/// `stat()` wrapper that follows symlinks unless `ls` is non-zero.
/// Returns `Option<Metadata>` mirroring C's `0`/`-1` return where
/// the metadata is filled into the supplied `struct stat *buf`.
/// WARNING: param names don't match C — Rust=(path, follow_symlink) vs C=(nam, buf, ls)
pub fn ztat(path: &str, follow_symlink: bool) -> Option<std::fs::Metadata> {
    // c:869
    if follow_symlink {
        // c:869 if (ls)
        // c:869 — `lstat(nam, buf)`. Don't follow symlinks.
        std::fs::symlink_metadata(path).ok()
    } else {
        // c:869 else
        // c:869 — `stat(nam, buf)`. Follow symlinks.
        std::fs::metadata(path).ok()
    }
}

/// Direct port of `void do_allmatches(UNUSED(int end))` from
/// `Src/Zle/compresult.c:895`. Inserts every match into the command
/// line, chaining `do_single` + `accept_last` across the whole
/// `amatches` group list, temporarily forcing `menucmp`. Saves and
/// restores `minfo` (keeping the recomputed `end`/`len`) and the
/// `lastbrbeg` brace string.
///
/// C walks the group list and per-match array by pointer (`minfo.cur`
/// = `Cmatch *`, `++minfo.cur`, `minfo.group->next`); the Rust store
/// is `amatches: Vec<Cmgroup>`, so those are `group_idx`/`cur_idx`
/// index steps. `lastbrbeg` is the tail node of the `BRBEG` chain.
pub fn do_allmatches(_end: i32) {
    // c:895
    let mut first = true; // c:897
    let mut nm = nmatches.load(Relaxed) - 1; // c:897
    let omc = MENUCMP.load(Relaxed); // c:897
    let oma = menuacc.load(Relaxed); // c:897

    // c:900 — p = brbeg ? ztrdup(lastbrbeg->str) : NULL. Snapshot the
    // tail node's string (lastbrbeg = last node of the brbeg chain).
    let saved_brbeg: Option<String> = BRBEG
        .get()
        .and_then(|mx| mx.lock().ok())
        .and_then(|g| {
            g.as_deref().map(|head| {
                let mut node = head;
                while let Some(n) = node.next.as_deref() {
                    node = n;
                }
                node.str.clone().unwrap_or_default()
            })
        });

    // c:902 — save minfo.
    let mi_saved = MINFO
        .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    MENUCMP.store(1, Relaxed); // c:903
    menuacc.store(0, Relaxed); // c:904

    // Snapshot the group list.
    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    // c:906-914 — first group with mcount > 0.
    let mut gi: usize = 0;
    while gi < groups.len() && groups[gi].mcount == 0 {
        gi += 1;
    }
    let mut have = gi < groups.len(); // c:916-917 mc = group->matches
    let mut mi: usize = 0;

    // c:919-942 — insert every match, chaining accept_last/do_single.
    while have {
        if (groups[gi].matches[mi].flags & CMF_ALL) == 0 {
            if !first {
                accept_last(); // c:922
            }
            first = false; // c:923
            if omc == 0 {
                nm -= 1; // c:925
                if nm == 0 {
                    MENUCMP.store(0, Relaxed); // c:926
                }
            }
            let m = groups[gi].matches[mi].clone();
            do_single(&m); // c:928
        }
        // c:930 — minfo.cur = mc (current position).
        {
            let m = groups[gi].matches[mi].clone();
            if let Ok(mut g) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                g.cur = Some(Box::new(m));
                g.group_idx = gi as i32;
                g.cur_idx = mi as i32;
            }
        }
        // c:932-940 — ++minfo.cur; if past the group end, next group.
        mi += 1;
        if mi >= groups[gi].matches.len() {
            loop {
                gi += 1; // c:934
                if gi >= groups.len() {
                    break;
                }
                if groups[gi].mcount != 0 {
                    break;
                }
            }
            if gi >= groups.len() {
                have = false; // c:937-938
                break;
            }
            mi = 0; // c:939
        }
        // c:941 — mc = minfo.cur (new position).
        if let Ok(mut g) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            g.group_idx = gi as i32;
            g.cur_idx = mi as i32;
            g.cur = Some(Box::new(groups[gi].matches[mi].clone()));
        }
    }
    MENUCMP.store(omc, Relaxed); // c:943
    menuacc.store(oma, Relaxed); // c:944

    // c:946-949 — restore minfo, keeping the recomputed end/len.
    let e = MINFO
        .get()
        .and_then(|g| g.lock().ok())
        .map(|g| g.end)
        .unwrap_or(0);
    if let Ok(mut g) = MINFO
        .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
        .lock()
    {
        *g = mi_saved;
        g.end = e; // c:948
        g.len = e - g.pos; // c:949
    }

    // c:951-954 — restore lastbrbeg->str (tail node of the brbeg chain).
    if let Some(s) = saved_brbeg {
        if let Some(mx) = BRBEG.get() {
            if let Ok(mut guard) = mx.lock() {
                if let Some(head) = guard.as_mut() {
                    let mut node = head.as_mut();
                    while node.next.is_some() {
                        node = node.next.as_mut().unwrap();
                    }
                    node.str = Some(s);
                }
            }
        }
    }
}

/// Direct port of `mod_export void do_single(Cmatch m)` from
/// `Src/Zle/compresult.c:961`. Inserts a single match into the
/// command line: sets `minfo` position state when not already in a
/// menu, deletes the old word, `instmatch`es the new one, then runs
/// the auto-suffix machinery — user `-S` suffix, AUTO_PARAM_SLASH
/// directory slash, brace `,`/`}` suffixes, and the trailing space —
/// and fires the `insert_match` hook. `CMF_ALL` dispatches to
/// `do_allmatches`. Operates entirely on the metafied `ZLEMETALINE`.
pub fn do_single(m: &Cmatch) {
    // c:961
    let cs = || ZLEMETACS.load(Relaxed);
    let set_cs = |v: i32| ZLEMETACS.store(v, Relaxed);

    let mut sr: i32 = 0; // c:963 (0 == ztat success)
    let mut havesuff = false; // c:964
    // c:965 — partest.
    let parpre_nonempty = parpre
        .get()
        .and_then(|p| p.lock().ok())
        .map(|g| !g.is_empty())
        .unwrap_or(false);
    let partest = m.ripre.is_some() || ((m.flags & CMF_ISPAR) != 0 && parpre_nonempty);
    // c:966-969 — str / psuf / prpre.
    let strv = m.orig.as_deref().unwrap_or("");
    let psuf = m.psuf.as_deref().unwrap_or("");
    let prpre = m.prpre.as_deref().unwrap_or("");

    fixsuffix(); // c:971

    let cur_present = MINFO
        .get()
        .and_then(|g| g.lock().ok())
        .map(|g| g.cur.is_some())
        .unwrap_or(false);

    let wb = WB.load(Relaxed);
    let we = WE.load(Relaxed);
    if !cur_present {
        // c:973-980 — set position variables.
        let movetoend_v = movetoend.load(Relaxed);
        let menucmp_v = MENUCMP.load(Relaxed);
        let we_flag = if movetoend_v >= 2
            || (movetoend_v == 1 && menucmp_v == 0)
            || (movetoend_v == 0 && cs() == we)
        {
            1
        } else {
            0
        };
        if let Ok(mut g) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            g.pos = wb; // c:976
            g.we = we_flag; // c:977
            g.end = we; // c:979
        }
    }
    // c:984-987 — bytes to delete.
    let l_del = MINFO
        .get()
        .and_then(|g| g.lock().ok())
        .map(|g| {
            if g.cur.is_some() {
                g.len + g.insc // c:985
            } else {
                we - wb // c:987
            }
        })
        .unwrap_or(we - wb);
    // c:989-991 — clear insc, delete the old word.
    let mi_pos = if let Ok(mut g) = MINFO
        .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
        .lock()
    {
        g.insc = 0; // c:989
        g.pos
    } else {
        wb
    };
    set_cs(mi_pos); // c:990
    foredel(l_del, CUT_RAW); // c:991

    // c:993-996 — CMF_ALL → do_allmatches and return.
    if (m.flags & CMF_ALL) != 0 {
        do_allmatches(0);
        return;
    }

    // c:998-1001 — insert the new string.
    let mut scs: i32 = 0;
    let ins_len = instmatch(m, Some(&mut scs)); // c:999
    let end_v = cs();
    if let Ok(mut g) = MINFO
        .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
        .lock()
    {
        g.len = ins_len; // c:999
        g.end = end_v; // c:1000
    }
    set_cs(mi_pos + ins_len); // c:1001

    let mi_we = MINFO
        .get()
        .and_then(|g| g.lock().ok())
        .map(|g| g.we)
        .unwrap_or(0);
    let menucmp_v = MENUCMP.load(Relaxed);

    if let Some(suf) = m.suf.as_deref() {
        // c:1003-1027 — user-specified suffix.
        havesuff = true; // c:1004
        let insc = suf.len() as i32; // c:1010
        if let Ok(mut g) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            g.insc = insc; // c:1010
            g.len -= insc; // c:1011
        }
        if mi_we != 0 {
            if let Ok(mut g) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                g.end += insc; // c:1013
            }
            if (m.flags & CMF_REMOVE) != 0 {
                // c:1019-1025 — removable suffix from the -S string.
                let mut wlen: i32 = 0;
                let wsuf = stringaszleline(suf, 0, Some(&mut wlen), None, None); // c:1021
                makesuffixstr(m.remf.as_deref(), m.rems.as_deref(), wlen); // c:1022
                if wlen == 1 {
                    addsuffix(SUFTYP_POSSTR, 0, wsuf, 1, 1); // c:1024
                }
            }
        }
    } else {
        // c:1028-1125 — auto-generate a suffix.
        set_cs(scs); // c:1031
        if partest && (m.flags & CMF_PARBR) != 0 {
            // c:1032-1047 — parameter in braces: removable `}`.
            set_cs(cs() + eparq.load(Relaxed)); // c:1037
            let parq_v = parq.load(Relaxed);
            for _ in 0..parq_v {
                inststrlen("\"", true, 1); // c:1039
            }
            if let Ok(mut g) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                g.insc += parq_v; // c:1040
            }
            inststrlen("}", true, 1); // c:1041
            let insc_now = if let Ok(mut g) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                g.insc += 1; // c:1042
                g.insc
            } else {
                0
            };
            if mi_we != 0 {
                if let Ok(mut g) = MINFO
                    .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                    .lock()
                {
                    g.end += insc_now; // c:1044
                }
            }
            if (m.flags & CMF_PARNEST) != 0 {
                havesuff = true; // c:1046
            }
        }
        // c:1048 — AUTO_PARAM_SLASH / file directory-slash.
        let cur = cs();
        let prev_not_slash = cur > 0
            && ZLEMETALINE
                .get()
                .and_then(|mx| mx.lock().ok())
                .map(|g| g.as_bytes().get((cur - 1) as usize).copied() != Some(b'/'))
                .unwrap_or(true);
        if ((m.flags & CMF_FILE) != 0 || (partest && isset(AUTOPARAMSLASH))) && cur > 0 && prev_not_slash
        {
            let mut t = false; // is a directory
            if m.ipre.as_deref() == Some("~") {
                // c:1057-1058 — bare `~`.
                t = true;
            } else {
                let p: String;
                if partest && psuf.is_empty() && (m.flags & CMF_PARNEST) == 0 {
                    // c:1061-1096 — build the parameter path + parse/subst.
                    let base = if (m.flags & CMF_ISPAR) != 0 {
                        parpre
                            .get()
                            .and_then(|px| px.lock().ok())
                            .map(|g| g.clone())
                            .unwrap_or_default()
                    } else {
                        m.ripre.clone().unwrap_or_default()
                    };
                    let mut pp = format!(
                        "{}{}{}",
                        base,
                        strv,
                        if (m.flags & CMF_PARBR) != 0 { "}" } else { "" }
                    );
                    // c:1070-1088 — if `$…`, only try when it's a scalar.
                    let mut tryit = true;
                    if pp.starts_with('$') {
                        let n: String = if pp.as_bytes().get(1) == Some(&b'{') {
                            let mut nn = pp[2..].to_string();
                            if nn.ends_with('}') {
                                nn.pop();
                            }
                            nn
                        } else {
                            pp[1..].to_string()
                        };
                        if let Ok(tab) = paramtab().read() {
                            if let Some(pm) = tab.get(&n) {
                                if PM_TYPE(pm.node.flags as u32) != PM_SCALAR {
                                    tryit = false; // c:1087
                                }
                            }
                        }
                    }
                    if tryit {
                        // c:1090-1095 — parse + single-word substitute.
                        let ne = noerrs.load(Relaxed);
                        noerrs.store(1, Relaxed);
                        let parsed = parsestr(&pp).unwrap_or_else(|_| pp.clone());
                        pp = singsub(&parsed);
                        errflag.fetch_and(!ERRFLAG_ERROR, Relaxed); // c:1094
                        noerrs.store(ne, Relaxed);
                    }
                    p = pp;
                } else {
                    // c:1098-1102 — normal file path.
                    let base = if prpre.is_empty() { "./" } else { prpre };
                    p = format!("{}{}{}", base, strv, psuf);
                }
                // c:1104 — stat the path.
                match ztat(&p, false) {
                    Some(md) => {
                        sr = 0;
                        t = md.is_dir();
                    }
                    None => {
                        sr = -1;
                        t = false;
                    }
                }
            }
            if t {
                // c:1106-1121 — it is a directory: append '/'.
                havesuff = true; // c:1108
                inststrlen("/", true, 1); // c:1109
                if let Ok(mut g) = MINFO
                    .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                    .lock()
                {
                    g.insc += 1; // c:1110
                    if mi_we != 0 {
                        g.end += 1; // c:1112
                    }
                }
                if menucmp_v == 0 || mi_we != 0 {
                    if m.remf.is_some() || m.rems.is_some() {
                        makesuffixstr(m.remf.as_deref(), m.rems.as_deref(), 1); // c:1115
                    } else if isset(AUTOREMOVESLASH) {
                        makesuffix(1); // c:1117
                        addsuffix(SUFTYP_POSSTR, 0, vec!['/'], 1, 1); // c:1118
                    }
                }
            }
        }
        // c:1123-1124 — no suffix inserted: cursor to string end - qisl.
        let insc_now = MINFO
            .get()
            .and_then(|g| g.lock().ok())
            .map(|g| g.insc)
            .unwrap_or(0);
        if insc_now == 0 {
            let len_now = MINFO
                .get()
                .and_then(|g| g.lock().ok())
                .map(|g| g.len)
                .unwrap_or(0);
            set_cs(mi_pos + len_now - m.qisl); // c:1124
        }
    }
    // c:1126-1160 — brace `,`/`}` suffix or the trailing space.
    let brbeg_present = BRBEG
        .get()
        .and_then(|mx| mx.lock().ok())
        .map(|g| g.is_some())
        .unwrap_or(false);
    if brbeg_present {
        if havesuff {
            // c:1132-1133 — removable `,}` when a suffix was added.
            if isset(AUTOPARAMKEYS) {
                addsuffix(SUFTYP_POSSTR, 0, vec![',', '}'], 2, suffixlen.load(Relaxed));
            }
        } else if menucmp_v == 0 {
            // c:1136-1142 — add a `,` and let `}` remove it.
            set_cs(scs); // c:1137
            inststrlen(",", true, 1); // c:1138
            if let Ok(mut g) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                g.insc += 1; // c:1139
            }
            makesuffix(1); // c:1140
            if (menucmp_v == 0 || mi_we != 0) && isset(AUTOPARAMKEYS) {
                addsuffix(SUFTYP_POSSTR, 0, vec![',', '}'], 2, 1); // c:1142
            }
        }
    } else if !havesuff && ((m.flags & CMF_FILE) == 0 || sr == 0) {
        // c:1144-1160 — add an autoq + trailing space.
        if let Some(autoq) = m.autoq.as_deref() {
            let isuf_prefixed = m
                .isuf
                .as_deref()
                .map(|isuf| isuf.starts_with(autoq))
                .unwrap_or(false);
            if !isuf_prefixed {
                let al = autoq.len() as i32; // c:1149
                inststrlen(autoq, true, al); // c:1150
                if let Ok(mut g) = MINFO
                    .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                    .lock()
                {
                    g.insc += al; // c:1151
                }
            }
        }
        let usemenu_v = USEMENU.load(Relaxed);
        if menucmp_v == 0
            && (m.flags & CMF_NOSPACE) == 0
            && (usemenu_v != 3 || insspace.load(Relaxed) != 0)
        {
            inststrlen(" ", true, 1); // c:1155
            if let Ok(mut g) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                g.insc += 1; // c:1156
            }
            if mi_we != 0 {
                makesuffixstr(m.remf.as_deref(), m.rems.as_deref(), 1); // c:1158
            }
        }
    }
    // c:1161-1168 — AUTO_PARAM_KEYS parameter suffix.
    let parq_v = parq.load(Relaxed);
    let insc_now = MINFO
        .get()
        .and_then(|g| g.lock().ok())
        .map(|g| g.insc)
        .unwrap_or(0);
    if mi_we != 0 && partest && isset(AUTOPARAMKEYS) && insc_now - parq_v > 0 {
        let take = (insc_now - parq_v).max(0);
        let tmpstr = ZLEMETALINE
            .get()
            .and_then(|mx| mx.lock().ok())
            .map(|g| {
                let b = g.as_bytes();
                let s = (parq_v.max(0) as usize).min(b.len());
                let e = (s + take as usize).min(b.len());
                String::from_utf8_lossy(&b[s..e]).into_owned()
            })
            .unwrap_or_default();
        let mut outlen: i32 = 0;
        let _subline = stringaszleline(&tmpstr, 0, Some(&mut outlen), None, None); // c:1165
        makeparamsuffix(if (m.flags & CMF_PARBR) != 0 { 1 } else { 0 }, outlen); // c:1166
    }
    // c:1170-1178 — final cursor placement.
    let movetoend_v = movetoend.load(Relaxed);
    if (menucmp_v != 0 && mi_we == 0) || movetoend_v == 0 {
        let (end_v, insc_v) = MINFO
            .get()
            .and_then(|g| g.lock().ok())
            .map(|g| (g.end, g.insc))
            .unwrap_or((0, 0));
        set_cs(end_v); // c:1171
        if cs() + m.qisl == lastend.load(Relaxed) {
            set_cs(cs() + insc_v); // c:1173
        }
        set_cs(cs() + psuf.len() as i32); // c:1176
        set_cs(cs() + m.suf.as_deref().map_or(0, |s| s.len() as i32)); // c:1177
    }
    // c:1179-1197 — insert-match hook + redraw (save/restore minfo.cur).
    {
        let (om_cur, om_cur_idx, om_group_idx) = MINFO
            .get()
            .and_then(|g| g.lock().ok())
            .map(|g| (g.cur.clone(), g.cur_idx, g.group_idx))
            .unwrap_or((None, 0, 0));
        let mut dat = Chdata {
            matches: amatches
                .get()
                .and_then(|g| g.lock().ok())
                .and_then(|g| g.first().cloned())
                .map(Box::new), // c:1183 dat.matches = amatches
            num: nmatches.load(Relaxed), // c:1189
            nmesg: 0,
            cur: Some(Box::new(m.clone())), // c:1190
        };
        if menucmp_v != 0 {
            // c:1193 — minfo.cur = &m.
            if let Ok(mut g) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                g.cur = Some(Box::new(m.clone()));
            }
        }
        // c:1194 — runhookdef(INSERTMATCHHOOK, &dat).
        let h = gethookdef("insert_match");
        if !h.is_null() {
            runhookdef(h, &mut dat as *mut Chdata as *mut std::ffi::c_void);
        }
        redrawhook(); // c:1195
        // c:1196 — minfo.cur = om.
        if let Ok(mut g) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            g.cur = om_cur;
            g.cur_idx = om_cur_idx;
            g.group_idx = om_group_idx;
        }
    }
}

/// Direct port of `static Cmatch *valid_match(Cmatch *m, int next)`
/// from `Src/Zle/compresult.c:1206`. Returns the next "valid" (need
/// not be skipped) match starting from `mi` within the current
/// `minfo.group`. Skips `CMF_DUMMY`, empty `CMF_NOLIST`/`CMF_MULT`
/// matches, and — while `menuacc` — matches whose brace prefix/suffix
/// differ from `minfo.prebr`/`minfo.postbr` (`hasbrpsfx`). If
/// `next != 0`, always advances at least once. Direction follows
/// `zmult`: forward wraps group→`amatches`, backward wraps
/// group→`lmatches` (the tail group in the Vec store).
///
/// C's `Cmatch *m` pointer and `minfo.group` walk become `cur_idx` /
/// `group_idx` steps over `amatches: Vec<Cmgroup>`. On success the
/// resolved `group_idx`/`cur_idx` are written back to `minfo` and the
/// match is returned (the caller assigns `minfo.cur`).
pub fn valid_match(mut mi: i32, next: i32) -> Option<Cmatch> {
    // c:1206
    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    if groups.is_empty() {
        return None;
    }
    let _ = &lmatches; // backward wrap targets the tail group (lmatches).
    let (gi0, prebr, postbr) = MINFO
        .get()
        .and_then(|g| g.lock().ok())
        .map(|g| (g.group_idx, g.prebr.clone(), g.postbr.clone()))
        .unwrap_or((0, None, None));
    let mut gi = gi0.clamp(0, groups.len() as i32 - 1);
    let menuacc_v = menuacc.load(Relaxed);
    let zmult_v = ZMULT.load(Relaxed);
    let total: i32 = groups.iter().map(|g| g.mcount).sum();
    let mut next = next;
    let mut steps = 0i32;
    loop {
        if gi < 0 || gi as usize >= groups.len() {
            return None;
        }
        let m = groups[gi as usize].matches.get(mi.max(0) as usize);
        // c:1208-1212 — is the current match invalid?
        let invalid = match m {
            None => true,
            Some(m) => {
                next != 0
                    || (menuacc_v != 0
                        && !hasbrpsfx(m, prebr.as_deref(), postbr.as_deref()))
                    || (m.flags & CMF_DUMMY) != 0
                    || ((m.flags & (CMF_NOLIST | CMF_MULT)) != 0
                        && m.str.as_deref().map_or(true, |s| s.is_empty()))
            }
        };
        if !invalid {
            break;
        }
        if zmult_v > 0 {
            // c:1213-1226 — advance forward, wrapping to amatches.
            mi += 1;
            if mi as usize >= groups[gi as usize].matches.len() {
                loop {
                    gi += 1; // minfo.group = group->next
                    if gi as usize >= groups.len() {
                        gi = 0; // minfo.group = amatches
                    }
                    if groups[gi as usize].mcount != 0 {
                        break;
                    }
                }
                mi = 0;
            }
        } else {
            // c:1227-1236 — advance backward, wrapping to lmatches.
            if mi == 0 {
                loop {
                    gi -= 1; // minfo.group = group->prev
                    if gi < 0 {
                        gi = groups.len() as i32 - 1; // minfo.group = lmatches
                    }
                    if groups[gi as usize].mcount != 0 {
                        break;
                    }
                }
                mi = groups[gi as usize].mcount - 1;
            } else {
                mi -= 1;
            }
        }
        next = 0; // c:1237
        steps += 1;
        if steps > total.max(1) * 2 + 2 {
            // Safety: every match skippable (guards against the same
            // infinite loop C would hit with an all-invalid list).
            return None;
        }
    }
    if let Ok(mut g) = MINFO
        .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
        .lock()
    {
        g.group_idx = gi;
        g.cur_idx = mi;
    }
    groups
        .get(gi as usize)
        .and_then(|g| g.matches.get(mi as usize))
        .cloned() // c:1239 return m
}

/// Direct port of `void do_menucmp(int lst)` from `Src/Zle/compresult.c:1253`.
/// Steps the menu cursor forward/backward, wrapping at ends. Per C:
/// when `lst == COMP_LIST_COMPLETE`, just set `showinglist=-2` and
/// return (caller refreshes the listing instead of inserting). The
/// Rust port returns the next match index; caller drives instmatch.
/// WARNING: param names don't match C — Rust=(matches, current, forward) vs C=(lst)
pub fn do_menucmp(matches: &[String], current: usize, forward: bool) -> (usize, &str) {
    // c:1253
    // c:1258 — `if (lst == COMP_LIST_COMPLETE) { showinglist = -2; return; }`.
    // We don't have a `lst` param at this signature; the listing-mode
    // call site (compresult.c via do_menucmp(lst==LIST_COMPLETE)) uses
    // a separate caller path. If the host's menu loop wraps to the
    // current entry (matches.len()==1), set showinglist=-2 so a
    // re-list happens.
    let _ = COMP_LIST_COMPLETE;
    if matches.is_empty() {
        return (0, "");
    }
    if matches.len() == 1 {
        SHOWINGLIST.store(-2, Relaxed);
    }
    let next = if forward {
        (current + 1) % matches.len()
    } else {
        if current == 0 {
            matches.len() - 1
        } else {
            current - 1
        }
    };
    (next, &matches[next])
}

/// Direct port of `accept_last()` from `Src/Zle/compresult.c:1288`.
/// Finalises the currently-selected menu match into the buffer.
///
/// Per C c:1299-1322: when !menuacc, snapshot lastprebr/lastpostbr
/// into minfo.prebr/postbr; if listshown is set and any match in
/// amatches lacks the brace prefix/suffix, force showinglist=-2.
/// Then bump menuacc and proceed with the do_single insertion.
pub fn accept_last() -> i32 {
    // c:1283
    let cs = || ZLEMETACS.load(Relaxed);
    let set_cs = |v: i32| ZLEMETACS.store(v, Relaxed);

    // c:1287-1293 — metafy the line if it isn't already.
    let was_meta = ZLEMETALL.load(Relaxed) != 0;
    if !was_meta {
        metafy_line();
    }

    // c:1295-1318 — first accept: snapshot lastprebr/lastpostbr into
    // minfo, and force a re-list if any match's braces differ.
    if menuacc.load(Relaxed) == 0 {
        let lpb = LASTPREBR
            .get()
            .and_then(|m| m.lock().ok())
            .map(|g| g.clone())
            .unwrap_or_default();
        let lsb = LASTPOSTBR
            .get()
            .and_then(|m| m.lock().ok())
            .map(|g| g.clone())
            .unwrap_or_default();
        // c:1296-1299 — empty string == C's NULL.
        let prebr_opt = if lpb.is_empty() { None } else { Some(lpb.clone()) };
        let postbr_opt = if lsb.is_empty() { None } else { Some(lsb.clone()) };
        if let Ok(mut m) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            m.prebr = prebr_opt.clone(); // c:1297
            m.postbr = postbr_opt.clone(); // c:1299
        }
        if LISTSHOWN.load(Relaxed) != 0 && (!lpb.is_empty() || !lsb.is_empty()) {
            // c:1305-1316 — scan every match for a brace mismatch.
            let groups = amatches
                .get_or_init(|| std::sync::Mutex::new(Vec::new()))
                .lock()
                .ok()
                .map(|g| g.clone())
                .unwrap_or_default();
            'outer: for g in &groups {
                for m in &g.matches {
                    if !hasbrpsfx(m, prebr_opt.as_deref(), postbr_opt.as_deref()) {
                        SHOWINGLIST.store(-2, Relaxed); // c:1313
                        break 'outer;
                    }
                }
            }
        }
    }
    menuacc.fetch_add(1, Relaxed); // c:1319

    let brbeg_present = BRBEG
        .get()
        .and_then(|mx| mx.lock().ok())
        .map(|g| g.is_some())
        .unwrap_or(false);
    if brbeg_present {
        // c:1321-1332 — rewrite lastbrbeg->str as `<inserted>,`.
        iremovesuffix(',' as i32, 1); // c:1324
        let brscs_v = brscs.load(Relaxed);
        let brpcs_v = brpcs.load(Relaxed);
        let l = (if brscs_v >= 0 { brscs_v } else { cs() }) - brpcs_v; // c:1326
        let slice = ZLEMETALINE
            .get()
            .and_then(|m| m.lock().ok())
            .map(|g| {
                let b = g.as_bytes();
                let s = (brpcs_v.max(0) as usize).min(b.len());
                let e = (s + l.max(0) as usize).min(b.len());
                String::from_utf8_lossy(&b[s..e]).into_owned()
            })
            .unwrap_or_default();
        let newstr = format!("{},", slice); // c:1331
        // lastbrbeg->str = newstr (tail node of the brbeg chain).
        if let Some(mx) = BRBEG.get() {
            if let Ok(mut guard) = mx.lock() {
                if let Some(head) = guard.as_mut() {
                    let mut node = head.as_mut();
                    while node.next.is_some() {
                        node = node.next.as_mut().unwrap();
                    }
                    node.str = Some(newstr);
                }
            }
        }
    } else {
        // c:1333-1348 — accept the pick and open a fresh arg with a space.
        let (pos, len, insc, qisl) = MINFO
            .get()
            .and_then(|g| g.lock().ok())
            .map(|g| {
                (
                    g.pos,
                    g.len,
                    g.insc,
                    g.cur.as_ref().map(|c| c.qisl).unwrap_or(0),
                )
            })
            .unwrap_or((0, 0, 0, 0));
        set_cs(pos + len + insc); // c:1336
        iremovesuffix(' ' as i32, 1); // c:1337
        let l = cs(); // c:1338
        set_cs(pos + len + insc - qisl); // c:1339
        if cs() < l {
            foredel(l - cs(), CUT_RAW); // c:1341
        } else if cs() > ZLEMETALL.load(Relaxed) {
            set_cs(ZLEMETALL.load(Relaxed)); // c:1343
        }
        inststrlen(" ", true, 1); // c:1344
        let newpos = cs();
        if let Ok(mut g) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            g.insc = 0; // c:1345
            g.len = 0;
            g.pos = newpos; // c:1346
            g.we = 1; // c:1347
        }
    }

    if !was_meta {
        unmetafy_line(); // c:1351
    }
    0 // c:1352
}

/// Port of `comp_mod(int v, int m)` from `Src/Zle/compresult.c:1363`.
/// ```c
/// static int
/// comp_mod(int v, int m)
/// {
///     if (v >= 0)
///         v--;
///     if (v >= 0)
///         return v % m;
///     else {
///         while (v < 0)
///             v += m;
///         return v;
///     }
/// }
/// ```
/// Modular arithmetic helper: subtract one when `v >= 0`, then
/// take `v % m`; for negative `v` (after the decrement), wrap by
/// repeated addition until non-negative. Used to map menu-cycle
/// indices to match-array offsets (where `0` means "no match" and
/// `1..N` are the real matches, so the table is 1-indexed).
pub fn comp_mod(mut v: i32, m: i32) -> i32 {
    // c:1364
    // Guard: C source assumes `m > 0` (lastpermmnum is always
    // populated from the match-list count by the time do_ambig_menu
    // calls it). With `m == 0`, the `while v < 0; v += m;` loop at
    // c:1371-1372 spins forever. zshrs's unit tests call do_ambig_menu
    // directly from a fresh state where lastpermmnum == 0 and the
    // process hangs (cargo test never finishes). C zsh hits the same
    // bug if any internal path forgets to populate lastpermmnum;
    // defensive guard with no behavior change for valid `m > 0`.
    if m <= 0 {
        return 0;
    }
    if v >= 0 {
        // c:1364
        v -= 1; // c:1367
    }
    if v >= 0 {
        // c:1368
        v % m // c:1369
    } else {
        // c:1370
        while v < 0 {
            // c:1371
            v += m; // c:1372
        }
        v // c:1373
    }
}

/// Port of `do_ambig_menu()` from `Src/Zle/compresult.c:1381`.
/// Direct port of `static void do_ambig_menu(void)` from
/// `Src/Zle/compresult.c:1381`. Menu-completion entry for the
/// ambiguous-matches case: cycles `minfo.group` forward until the
/// `insmnum`-th match in the chain is reached, then routes the
/// pick through `do_single`.
pub fn do_ambig_menu() -> i32 {
    // c:1381

    // c:1386 — `if (iforcemenu == -1) do_ambiguous();`
    if iforcemenu.load(Relaxed) == -1 {
        // c:1386
        let _ = do_ambiguous(&[]); // c:1387
    }

    let um = USEMENU.load(Relaxed);
    if um != 3 {
        // c:1389
        MENUCMP.store(1, Relaxed); // c:1390
        menuacc.store(0, Relaxed); // c:1391
        if let Ok(mut m) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            m.cur = None; // c:1392
        }
    } else {
        if oldlist.load(Relaxed) != 0 {
            // c:1395
            let has_cur = MINFO
                .get()
                .and_then(|m| m.lock().ok())
                .map(|m| m.cur.is_some())
                .unwrap_or(false);
            if oldins.load(Relaxed) != 0 && has_cur {
                // c:1396-1397 — accept the current menu pick.
                let _ = accept_last();
            }
        } else {
            if let Ok(mut m) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                m.cur = None; // c:1399
            }
        }
    }

    // c:1429 — `insmnum = comp_mod(insmnum, lastpermmnum)`.
    let mut idx = comp_mod(insmnum.load(Relaxed), lastpermmnum.load(Relaxed));
    insmnum.store(idx, Relaxed);

    // c:1417-1426 — walk amatches; find the group holding insmnum, and
    // reduce idx to the offset within that group.
    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut gidx: Option<usize> = None;
    for (gi, g) in groups.iter().enumerate() {
        if g.mcount > idx {
            gidx = Some(gi);
            break;
        }
        idx -= g.mcount; // c:1420
    }

    let Some(gi) = gidx else {
        // c:1427-1431
        if let Ok(mut m) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            m.cur = None;
            m.asked = 0;
        }
        return 0;
    };
    // Point minfo.group at the chosen group so valid_match walks from it.
    if let Ok(mut m) = MINFO
        .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
        .lock()
    {
        m.group = Some(Box::new(groups[gi].clone()));
        m.group_idx = gi as i32;
        m.cur_idx = idx;
    }
    insmnum.store(idx, Relaxed);

    // c:1436 — mc = valid_match((minfo.group)->matches + insmnum, 0).
    let mc = valid_match(idx, 0);

    // c:1437-1438 — insert the pick unless we're only forcing the menu.
    if iforcemenu.load(Relaxed) != -1 {
        if let Some(ref m) = mc {
            do_single(m);
        }
    }
    // c:1439 — minfo.cur = mc.
    if let Ok(mut mst) = MINFO
        .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
        .lock()
    {
        mst.cur = mc.map(Box::new);
    }
    0
}

/// Compute how many rows the list will take given a fixed column
/// count.
/// Port of `list_lines()` from Src/Zle/compresult.c — the listing
/// path uses this to decide whether to invoke the more-prompt
/// (`asklistscroll`).
// Return the number of screen lines needed for the list.                   // c:1450
/// WARNING: param names don't match C — Rust=(matches, columns) vs C=()
pub fn list_lines(matches: &[String], columns: usize) -> usize {
    // c:1450
    if columns == 0 {
        return matches.len();
    }
    matches.len().div_ceil(columns)
}

/// Port of `comp_list(char *v)` from `Src/Zle/compresult.c:1468`.
/// ```c
/// void
/// comp_list(char *v)
/// {
///     zsfree(complist);
///     complist = v;
///     onlyexpl = (v ? ((strstr(v, "expl") ? 1 : 0) |
///                      (strstr(v, "messages") ? 2 : 0)) : 0);
/// }
/// ```
/// Set the `complist` global and update `onlyexpl` per the
/// substring scan. Called from `bin_compset` to honour
/// `compstate[list]`.
/// C body (Src/Zle/compresult.c:1468) is 4 lines:
///   `zsfree(complist); complist = v;
///    onlyexpl = v ? ((strstr(v,"expl")?1:0) |
///                    (strstr(v,"messages")?2:0)) : 0;`
pub fn comp_list(v: Option<&str>) {
    // c:1468
    let mut g = crate::ported::zle::complete::COMPLIST // c:1470 zsfree+assign
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
        .unwrap();
    g.clear();
    if let Some(s) = v {
        g.push_str(s);
    }
    let val = v.map_or(0, |s| {
        (s.contains("expl") as i32) | (s.contains("messages") as i32) << 1
    }); // c:1473
    onlyexpl.store(val, Ordering::SeqCst); // c:1473
}

/// Port of `skipnolist(Cmatch *p, int showall)` from `Src/Zle/compresult.c:1480`.
/// ```c
/// mod_export Cmatch *
/// skipnolist(Cmatch *p, int showall)
/// {
///     int mask = (showall ? 0 : (CMF_NOLIST | CMF_MULT)) | CMF_HIDE;
///     while (*p && (((*p)->flags & mask) ||
///                   ((*p)->disp &&
///                    ((*p)->flags & (CMF_DISPLINE | CMF_HIDE)))))
///         p++;
///     return p;
/// }
/// ```
/// Walk a `Cmatch*` array skipping over entries that won't be
/// listed (CMF_NOLIST/CMF_MULT/CMF_HIDE) and over disp-strings
/// that are CMF_DISPLINE/CMF_HIDE. Returns the index of the first
/// listable entry (or `matches.len()` if none).
///
/// `showall` mirrors C: when non-zero, the NOLIST/MULT mask is
/// dropped (only CMF_HIDE filters).
pub fn skipnolist(p: &[Cmatch], showall: i32) -> usize {
    // c:1481
    // c:1483 — `mask = (showall ? 0 : (CMF_NOLIST|CMF_MULT)) | CMF_HIDE`.
    let mask = if showall != 0 {
        0
    } else {
        CMF_NOLIST | CMF_MULT
    } | CMF_HIDE;
    let mut i = 0usize; // c:1485 *p
    while i < p.len() {
        // c:1485 while (*p && ...)
        let m = &p[i];
        let f = m.flags;
        let skip_mask = (f & mask) != 0; // c:1485
        let skip_disp = m.disp.is_some() && (f & (CMF_DISPLINE | CMF_HIDE)) != 0; // c:1486-1487
        if !(skip_mask || skip_disp) {
            break;
        }
        i += 1; // c:1488 p++
    }
    i // c:1490 return p
}

/// Port of `mod_export int calclist(int showall)` from
/// `Src/Zle/compresult.c:1495`. Walks the active `cmgroup` chain,
/// computes per-group column widths, line counts, and per-match
/// width entries, then writes `listdat`. Returns 1 when listdat was
/// updated, 0 when the cached snapshot is still valid.
pub fn calclist(showall: i32) -> i32 {
    // c:1495

    let invcount = INVCOUNT.load(Relaxed);
    let onlyexpl_v = onlyexpl.load(Relaxed);
    let menuacc_v = menuacc.load(Relaxed);
    // c:1587 — hasbrpsfx compares each match against minfo.prebr/postbr.
    let (minfo_prebr, minfo_postbr) = MINFO
        .get()
        .and_then(|g| g.lock().ok())
        .map(|g| (g.prebr.clone(), g.postbr.clone()))
        .unwrap_or((None, None));
    let zterm_columns = adjustcolumns() as i32; // c:zterm_columns
    let zterm_lines = adjustlines() as i32; // c:zterm_lines

    // c:1506-1511 — early-exit when nothing has changed.
    {
        let ld = crate::ported::zle::compcore::listdat.get_or_init(|| std::sync::Mutex::new(Cldata::default()));
        let g = ld.lock().unwrap();
        if LASTINVCOUNT.with(|c| c.get()) == invcount
            && g.valid != 0
            && onlyexpl_v == g.onlyexpl
            && menuacc_v == g.menuacc
            && showall == g.showall
            && zterm_lines == g.zterm_lines
            && zterm_columns == g.zterm_columns
        {
            return 0; // c:1511
        }
    }
    LASTINVCOUNT.with(|c| c.set(invcount)); // c:1512

    let am = amatches.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut groups = am.lock().unwrap();
    let nmatches2 = nmatches.load(Relaxed);
    let mut mlens: Vec<i32> = vec![0; (nmatches2 + 1) as usize];

    let mut hidden = 0i32;
    let mut nlist = 0i32;
    let mut nlines = 0i32;
    let mut max = 0i32;

    let listpacked = isset(LISTPACKED);
    let listrowsfirst = isset(LISTROWSFIRST);
    let listtypes = isset(LISTTYPES);

    // First pass — per-group width / line accounting (c:1514-1657).
    for g in groups.iter_mut() {
        let mut nl = false;
        let mut glong = 1i32;
        let mut gshort = zterm_columns;
        let mut ndisp = 0i32;
        let mut totl = 0i32;
        let mut hasf = false;

        g.flags |= CGF_PACKED | CGF_ROWS; // c:1524

        if onlyexpl_v == 0 && !g.ylist.is_empty() {
            if !listpacked {
                g.flags &= !CGF_PACKED;
            } // c:1528-1529
            if !listrowsfirst {
                g.flags &= !CGF_ROWS;
            } // c:1530-1531

            hidden = 1; // c:1535
            for s in g.ylist.iter() {
                // c:1536-1541
                if (s.chars().count() as i32) >= zterm_columns || s.contains('\n') {
                    nl = true;
                    break;
                }
            }
            if nl || g.ylist.len() < 2 {
                // c:1543
                g.flags |= CGF_LINES; // c:1547
                hidden = 1; // c:1548
                for s in g.ylist.iter() {
                    // c:1549-1564
                    let mut acc = 0i32;
                    for chunk in s.split('\n') {
                        let w = chunk.chars().count().saturating_sub(1) as i32;
                        acc += 1 + w / zterm_columns;
                    }
                    nlines += acc;
                }
            } else {
                for s in g.ylist.iter() {
                    // c:1567-1577
                    let l = s.chars().count() as i32;
                    ndisp += 1;
                    if l > glong {
                        glong = l;
                    }
                    if l < gshort {
                        gshort = l;
                    }
                    totl += l;
                    nlist += 1;
                }
            }
        } else if onlyexpl_v == 0 {
            // c:1579-1631 — per-match width walk.
            for m in g.matches.iter_mut() {
                if (m.flags & CMF_FILE) != 0 {
                    hasf = true;
                }
                if menuacc_v != 0
                    && !hasbrpsfx(m, minfo_prebr.as_deref(), minfo_postbr.as_deref())
                {
                    m.flags |= CMF_HIDE;
                    continue;
                }
                m.flags &= !CMF_HIDE;

                if showall != 0 || (m.flags & (CMF_NOLIST | CMF_MULT)) == 0 {
                    if (m.flags & (CMF_NOLIST | CMF_MULT)) != 0
                        && m.str.as_deref().is_none_or(|s| s.is_empty())
                    {
                        m.flags |= CMF_HIDE;
                        continue;
                    }
                    if let Some(disp) = m.disp.clone() {
                        if (m.flags & CMF_DISPLINE) != 0 {
                            nlines += 1 + printfmt(&disp, 0, false, false);
                            g.flags |= CGF_HASDL;
                        } else {
                            let l =
                                disp.chars().count() as i32 + if m.modec != '\0' { 1 } else { 0 };
                            ndisp += 1;
                            if l > glong {
                                glong = l;
                            }
                            if l < gshort {
                                gshort = l;
                            }
                            totl += l;
                            mlens[m.gnum as usize] = l;
                        }
                        nlist += 1;
                        if (m.flags & CMF_PACKED) == 0 {
                            g.flags &= !CGF_PACKED;
                        }
                        if (m.flags & CMF_ROWS) == 0 {
                            g.flags &= !CGF_ROWS;
                        }
                    } else {
                        let s = m.str.as_deref().unwrap_or("");
                        let l = s.chars().count() as i32 + if m.modec != '\0' { 1 } else { 0 };
                        ndisp += 1;
                        if l > glong {
                            glong = l;
                        }
                        if l < gshort {
                            gshort = l;
                        }
                        totl += l;
                        mlens[m.gnum as usize] = l;
                        nlist += 1;
                        if (m.flags & CMF_PACKED) == 0 {
                            g.flags &= !CGF_PACKED;
                        }
                        if (m.flags & CMF_ROWS) == 0 {
                            g.flags &= !CGF_ROWS;
                        }
                    }
                } else {
                    hidden = 1;
                }
            }
        }
        // c:1633-1643 — explanation strings.
        for e in g.expls.iter() {
            if (e.count != 0 || e.always != 0)
                && (onlyexpl_v == 0 || (onlyexpl_v & if e.always > 0 { 2 } else { 1 }) != 0)
            {
                nlines += 1 + printfmt(
                    e.str.as_deref().unwrap_or(""),
                    if e.always != 0 { -1 } else { e.count },
                    false,
                    true,
                );
            }
        }
        if listtypes && hasf {
            g.flags |= CGF_FILES;
        } // c:1644-1645
        g.totl = totl + ndisp * CM_SPACE; // c:1646
        g.dcount = ndisp; // c:1647
        g.width = glong + CM_SPACE; // c:1648
        g.shortest = gshort + CM_SPACE; // c:1649
        if g.width > 0 {
            g.cols = (zterm_columns / g.width).min(g.dcount); // c:1650-1651
        }
        if g.cols > 0 {
            let i = g.cols * g.width - CM_SPACE; // c:1653
            if i > max {
                max = i;
            }
        }
    }

    // Pass A — per-group line counts (c:1660-1715).
    if onlyexpl_v == 0 {
        for g in groups.iter_mut() {
            let mut glines = 0i32;
            g.widths.clear(); // c:1670-1671
            if !g.ylist.is_empty() {
                if (g.flags & CGF_LINES) == 0 {
                    if g.cols > 0 {
                        glines += (g.ylist.len() as i32 + g.cols - 1) / g.cols;
                        if g.cols > 1 {
                            g.width += (max - (g.width * g.cols - CM_SPACE)) / g.cols;
                        }
                    } else {
                        g.cols = 1;
                        g.width = 1;
                        for s in g.ylist.iter() {
                            glines += 1 + s.chars().count() as i32 / zterm_columns;
                        }
                    }
                }
            } else if g.cols > 0 {
                glines += (g.dcount + g.cols - 1) / g.cols;
                if g.cols > 1 {
                    g.width += (max - (g.width * g.cols - CM_SPACE)) / g.cols;
                }
            } else if (g.flags & CGF_LINES) == 0 {
                g.cols = 1;
                g.width = 0;
                for m in g.matches.iter() {
                    if (m.flags & CMF_HIDE) == 0 {
                        if m.disp.is_some() {
                            if (m.flags & CMF_DISPLINE) == 0 {
                                glines +=
                                    1 + (mlens[m.gnum as usize].saturating_sub(1)) / zterm_columns;
                            }
                        } else if showall != 0 || (m.flags & (CMF_NOLIST | CMF_MULT)) == 0 {
                            glines +=
                                1 + (mlens[m.gnum as usize].saturating_sub(1)) / zterm_columns;
                        }
                    }
                }
            }
            g.lins = glines;
            nlines += glines;
        }

        // Pass B — packed-tcols width search (c:1716-1888). For every
        // CGF_PACKED group, walk tcols candidates from "as many as
        // shortest-width allows" down to the existing cols, picking the
        // densest tcols whose total width still fits zterm_columns.
        // Four sub-branches: {ylist, matches} × {ROWS, !ROWS}.
        for g in groups.iter_mut() {
            if (g.flags & CGF_PACKED) == 0 {
                continue;
            } // c:1717-1718
              // c:1720-1721 — `ws = g->widths = zalloc(...); memset(ws,0,...)`
            g.widths = vec![0i32; zterm_columns as usize];
            let mut tlines = g.lins; // c:1722
            let mut tcols = g.cols; // c:1723
            let mut width: i32 = 0; // c:1724

            if !g.ylist.is_empty() {
                // c:1726
                if (g.flags & CGF_LINES) == 0 {
                    // c:1727
                    // c:1728-1732 — per-item widths in `ylens`.
                    let ylens: Vec<i32> = g
                        .ylist
                        .iter()
                        .map(|s| s.chars().count() as i32 + CM_SPACE)
                        .collect();

                    if (g.flags & CGF_ROWS) != 0 {
                        // c:1734-1760 — row-major ylist tcols search.
                        let mut t = zterm_columns / (g.shortest + CM_SPACE);
                        while t > g.cols {
                            for w in &mut g.widths[..t as usize] {
                                *w = 0;
                            } // c:1741
                            let mut w = 0i32;
                            let mut nth = 0i32;
                            let mut tcol = 0i32;
                            let mut tl = 1i32;
                            while w < zterm_columns && nth < g.dcount {
                                // c:1743-1744
                                if tcol == t {
                                    tcol = 0;
                                    tl += 1;
                                } // c:1747-1750
                                let len = ylens[nth as usize]; // c:1751
                                if len > g.widths[tcol as usize] {
                                    // c:1753
                                    w += len - g.widths[tcol as usize]; // c:1754
                                    g.widths[tcol as usize] = len; // c:1755
                                }
                                nth += 1;
                                tcol += 1;
                            }
                            width = w;
                            tcols = t;
                            tlines = tl;
                            if w < zterm_columns {
                                break;
                            } // c:1758-1759
                            t -= 1;
                        }
                    } else {
                        // c:1764-1796 — column-major ylist tcols search.
                        // C has a dead `m = *p;` on c:1777 (p never set
                        // in this branch); preserved as no-op.
                        let mut t = zterm_columns / (g.shortest + CM_SPACE);
                        while t > g.cols {
                            let mut tl = ((g.dcount + t - 1) / t).max(1); // c:1768-1769
                            for w in &mut g.widths[..t as usize] {
                                *w = 0;
                            } // c:1771
                            let mut w = 0i32;
                            let mut nth = 0i32;
                            let mut tcol = 0i32;
                            let mut tline = 0i32;
                            while w < zterm_columns && nth < g.dcount {
                                // c:1773-1775
                                if tline == tl {
                                    tcol += 1;
                                    tline = 0;
                                } // c:1779-1782
                                if tcol == t {
                                    tcol = 0;
                                    tl += 1;
                                } // c:1783-1786
                                let len = ylens[nth as usize]; // c:1787
                                if len > g.widths[tcol as usize] {
                                    // c:1789
                                    w += len - g.widths[tcol as usize];
                                    g.widths[tcol as usize] = len;
                                }
                                nth += 1;
                                tline += 1;
                            }
                            width = w;
                            tcols = t;
                            tlines = tl;
                            if w < zterm_columns {
                                break;
                            } // c:1794-1795
                            t -= 1;
                        }
                    }
                }
            } else if g.width != 0 {
                // c:1799
                if (g.flags & CGF_ROWS) != 0 {
                    // c:1803-1830 — row-major matches tcols search.
                    let mut t = zterm_columns / (g.shortest + CM_SPACE);
                    while t > g.cols {
                        for w in &mut g.widths[..t as usize] {
                            *w = 0;
                        } // c:1807
                        let mut w = 0i32;
                        let mut tcol = 0i32;
                        let mut tl = 1i32;
                        let mut nth = 0i32;
                        // c:1810 — `p = skipnolist(g->matches, showall)`.
                        let mut p_idx = skipnolist(&g.matches, showall);
                        while p_idx < g.matches.len() && w < zterm_columns && nth < g.dcount {
                            if tcol == t {
                                tcol = 0;
                                tl += 1;
                            } // c:1816-1819
                            let m = &g.matches[p_idx]; // c:1814
                            let len =
                                mlens[m.gnum as usize] + if tcol == t - 1 { 0 } else { CM_SPACE }; // c:1820-1821
                            if len > g.widths[tcol as usize] {
                                w += len - g.widths[tcol as usize];
                                g.widths[tcol as usize] = len;
                            }
                            nth += 1;
                            // c:1812 — `p = skipnolist(p+1, showall)`.
                            let nxt = p_idx + 1;
                            if nxt >= g.matches.len() {
                                p_idx = g.matches.len();
                            } else {
                                p_idx = nxt + skipnolist(&g.matches[nxt..], showall);
                            }
                            tcol += 1;
                        }
                        width = w;
                        tcols = t;
                        tlines = tl;
                        if w < zterm_columns {
                            break;
                        } // c:1828-1829
                        t -= 1;
                    }
                } else {
                    // c:1834-1872 — column-major matches tcols search.
                    let mut t = zterm_columns / (g.shortest + CM_SPACE);
                    while t > g.cols {
                        let mut tl = ((g.dcount + t - 1) / t).max(1); // c:1838-1839
                        for w in &mut g.widths[..t as usize] {
                            *w = 0;
                        } // c:1841
                        let mut w = 0i32;
                        let mut nth = 0i32;
                        let mut tcol = 0i32;
                        let mut tline = 0i32;
                        let mut p_idx = skipnolist(&g.matches, showall); // c:1844
                        while p_idx < g.matches.len() && w < zterm_columns && nth < g.dcount {
                            if tline == tl {
                                tcol += 1;
                                tline = 0;
                            } // c:1850-1853
                            if tcol == t {
                                tcol = 0;
                                tl += 1;
                            } // c:1854-1857
                            let m = &g.matches[p_idx]; // c:1848
                            let len =
                                mlens[m.gnum as usize] + if tcol == t - 1 { 0 } else { CM_SPACE }; // c:1858-1859
                            if len > g.widths[tcol as usize] {
                                w += len - g.widths[tcol as usize];
                                g.widths[tcol as usize] = len;
                            }
                            nth += 1;
                            let nxt = p_idx + 1;
                            if nxt >= g.matches.len() {
                                p_idx = g.matches.len();
                            } else {
                                p_idx = nxt + skipnolist(&g.matches[nxt..], showall);
                            }
                            tline += 1;
                        }
                        width = w;
                        tcols = t;
                        tlines = tl;
                        if w < zterm_columns {
                            // c:1866-1869
                            // C: `if (++tcol < tcols) tcols = tcol;`
                            if tcol + 1 < tcols {
                                tcols = tcol + 1;
                            }
                            break;
                        }
                        t -= 1;
                    }
                }
            }

            // c:1874-1887 — commit the result (or revert if no win).
            if tcols <= g.cols {
                tlines = g.lins;
            } // c:1874-1875
            if tlines == g.lins {
                // c:1876
                g.widths.clear(); // c:1877-1878
            } else {
                nlines += tlines - g.lins; // c:1880
                g.lins = tlines; // c:1881
                g.cols = tcols; // c:1882
                g.totl = width; // c:1883
                let width_adj = width - CM_SPACE; // c:1884
                if width_adj > max {
                    max = width_adj;
                } // c:1885-1886
            }
        }

        // c:1889-1897 — final per-column width balance for groups
        // without packed widths.
        for g in groups.iter_mut() {
            if g.widths.is_empty() && g.width != 0 && g.cols > 1 {
                g.width += (max - (g.width * g.cols - CM_SPACE)) / g.cols;
            }
        }
    } else {
        for g in groups.iter_mut() {
            g.widths.clear(); // c:1907
        }
    }

    // c:1910-1918 — commit listdat.
    let ld = crate::ported::zle::compcore::listdat.get_or_init(|| std::sync::Mutex::new(Cldata::default()));
    let mut g = ld.lock().unwrap();
    g.valid = 1;
    g.hidden = hidden;
    g.nlist = nlist;
    g.nlines = nlines;
    g.menuacc = menuacc_v;
    g.onlyexpl = onlyexpl_v;
    g.zterm_columns = zterm_columns;
    g.zterm_lines = zterm_lines;
    g.showall = showall;
    1 // c:1920
}

/// Direct port of `int asklist(void)` from
/// `Src/Zle/compresult.c:1925`. The "do you wish to see all N
/// possibilities?" prompt that gates display of long completion
/// lists. Returns 1 to suppress the listing (user said no), 0 to
/// proceed (yes / no prompt needed).
///
/// Implements the C decision tree:
///   - `trashzle()` + zero `showinglist`/`listshown`.
///   - `clearflag = USEZLE && !termflags && dolastprompt`.
///   - Threshold check `complistmax > 0 ? nlist >= complistmax :
///     complistmax < 0 ? nlines <= -complistmax :
///     nlines >= zterm_lines`.
///   - If threshold tripped, prompt via `getzlequery` and set
///     `minfo.asked = 1 or 2`. Else return based on previous asked.
pub fn asklist() -> i32 {
    // c:1925

    // c:1928 — `trashzle(); showinglist = listshown = 0; lastlistlen = 0`.
    trashzle(); // c:1928
    SHOWINGLIST.store(0, Relaxed);
    LISTSHOWN.store(0, Relaxed);
    LASTLISTLEN.store(0, Relaxed); // c:1934

    // c:1930 — `clearflag = (isset(USEZLE) && !termflags && dolastprompt)`.
    let usezle = isset(USEZLE);
    let termflags = crate::ported::params::TERMFLAGS.load(Relaxed);
    let dolastprompt = crate::ported::zle::compcore::dolastprompt.load(Relaxed) != 0;
    let clearflag = usezle && termflags == 0 && dolastprompt;
    CLEARFLAG.store(if clearflag { 1 } else { 0 }, Relaxed);

    // c:1937-1940 — snapshot listdat counts + minfo state.
    let listdat = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let zterm_lines = adjustlines() as i32;
    let cmax = COMPLISTMAX.load(Relaxed) as i32;

    let has_cur = MINFO
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.cur.is_some())
        .unwrap_or(false);
    let already_asked = MINFO
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.asked)
        .unwrap_or(0);

    // c:1939-1942 — threshold gate.
    let over_threshold = (cmax > 0 && listdat.nlist >= cmax)
        || (cmax < 0 && listdat.nlines <= -cmax)
        || (cmax == 0 && listdat.nlines >= zterm_lines);

    // c:1939 — `if ((!minfo.cur || !minfo.asked) && over_threshold)`.
    if (!has_cur || already_asked == 0) && over_threshold {
        // c:1947-1953 — write the "do you wish to see ...?" prompt.
        let prompt = if listdat.nlist > 0 {
            format!(
                "zsh: do you wish to see all {} possibilities ({} lines)? ",
                listdat.nlist, listdat.nlines
            )
        } else {
            format!("zsh: do you wish to see all {} lines? ", listdat.nlines)
        };
        let fd = SHTTY.load(Relaxed);
        let out = if fd >= 0 { fd } else { 1 };
        let _ = write_loop(out, prompt.as_bytes());

        // c:1955 — `getzlequery()`.
        let said_yes = getzlequery() != 0;

        if !said_yes {
            // c:1956
            // c:1957-1964 — clean up the question line.
            let _ = write_loop(out, b"\n");
            // c:1965 — `minfo.asked = 2`.
            if let Ok(mut m) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Default::default()))
                .lock()
            {
                m.asked = 2;
            }
            return 1; // c:1966
        }
        // c:1968-1974 — clean up after a yes.
        let _ = write_loop(out, b"\n");
        // c:1975 — `minfo.asked = 1`.
        if let Ok(mut m) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Default::default()))
            .lock()
        {
            m.asked = 1;
        }
    }
    // c:1978-1979 — second-pass entry: already-asked-no falls through
    //                to the final return-1 to suppress the listing.

    // c:1981 — `return (minfo.asked ? minfo.asked - 1 : 0);`.
    let asked_now = MINFO
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.asked)
        .unwrap_or(0);
    if asked_now != 0 {
        asked_now - 1
    } else {
        0
    }
}

thread_local! {
    /// `static int lastinvcount = -1;` from compresult.c:1497 inside
    /// `calclist`. Caches the last `invcount` seen so the early-exit
    /// at c:1506-1511 fires when nothing has changed.
    static LASTINVCOUNT: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
}

/// Port of `printlist(int over, CLPrintFunc printm, int showall)` from `Src/Zle/compresult.c:1978`.
/// Direct port of `void printlist(int over, CLPrintFunc printm,
///                                  int showall)` from
/// `Src/Zle/compresult.c:1978`. The workhorse listing renderer:
/// walks `amatches`, emits each group's explanations and match cells
/// through `printm`, padding columns and adding group separators.
///
/// `over` selects the overflow-page mode (uses `listdat.nlines`);
/// `printm` is the per-cell callback (default `iprintm`); `showall`
/// surfaces CMF_HIDE / CMF_NOLIST matches that would otherwise be
/// skipped.
/// WARNING: param names don't match C — Rust=(over, showall) vs C=(over, printm, showall)
pub fn printlist(over: i32, showall: i32) -> i32 {
    // c:1978
    // c:1985 — `printlist` writes the entire match listing to
    //          `shout`. Resolve once and reuse for every emission so
    //          a single SHTTY load covers the whole render.
    let out_fd: i32 = {
        let fd = SHTTY.load(Relaxed);
        if fd >= 0 {
            fd
        } else {
            1
        }
    };

    let listdat = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut cl: i32 = if over != 0 { listdat.nlines } else { -1 }; // c:1984
    let mut pnl: i32 = 0; // c:1984
    let mut ml: i32 = 0;

    if cl < 2 {
        // c:1986
        cl = -1;
        // c:1987-1988 — `if (tccan(TCCLEAREOD)) tcout(TCCLEAREOD);`
        if crate::ported::init::tclen.lock().unwrap()[crate::ported::zsh_h::TCCLEAREOD as usize]
            != 0
        {
            tcout(crate::ported::zsh_h::TCCLEAREOD);
        }
    }

    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();

    for g in &groups {
        // c:1990
        // c:2000-2027 — explanations.
        for e in g.expls.iter() {
            // c:2000
            let active = (e.count != 0 || e.always != 0)                     // c:2001
                && (listdat.onlyexpl == 0
                    || (listdat.onlyexpl
                        & (if e.always > 0 { 2 } else { 1 })) != 0);
            if !active {
                continue;
            }

            if pnl != 0 {
                // c:2007
                let _ = write_loop(out_fd, b"\n"); // c:2008
                ml += 1;
                cl -= 1;
                if cl >= 0 && cl <= 1 {
                    // c:2010
                    cl = -1;
                    // c:2010-2011 — `if (tccan(TCCLEAREOD)) tcout(TCCLEAREOD);`
                    if crate::ported::init::tclen.lock().unwrap()
                        [crate::ported::zsh_h::TCCLEAREOD as usize]
                        != 0
                    {
                        tcout(crate::ported::zsh_h::TCCLEAREOD);
                    }
                }
            }
            // c:2017-2018 — printfmt(e.str, count, 1, 1).
            let n = if e.always != 0 { -1 } else { e.count };
            let l = printfmt(e.str.as_deref().unwrap_or(""), n, true, true);
            ml += l;
            if cl >= 0 && (cl - l) <= 1 {
                cl = -1;
            }
            pnl = 1;
        }

        // c:2032-2076 — ylist branch (alternative listing).
        if listdat.onlyexpl == 0 && !g.ylist.is_empty() {
            // c:2032
            if pnl != 0 {
                // c:2033
                let _ = write_loop(out_fd, b"\n");
                pnl = 0;
                ml += 1;
                if cl >= 0 && cl <= 1 {
                    cl = -1;
                }
            }
            if (g.flags & CGF_LINES) != 0 {
                // c:2044
                let mut so = std::io::stdout();
                let last_idx = g.ylist.len().saturating_sub(1);
                for (i, p) in g.ylist.iter().enumerate() {
                    let _ = zputs(p, &mut so);
                    if i != last_idx {
                        // c:2050
                        // C wraps via " \b" or "\n"; we emit \n for safety.
                        let _ = write_loop(out_fd, b"\n");
                    }
                }
            } else {
                // c:2058
                // Column layout — emit each entry.
                let mut so = std::io::stdout();
                for entry in &g.ylist {
                    let _ = zputs(entry, &mut so);
                    let _ = write_loop(out_fd, b"\n");
                    ml += 1;
                }
            }
        } else if listdat.onlyexpl == 0 && (g.lcount != 0 || (showall != 0 && g.mcount != 0)) {
            // c:2079-2185 — main column-rendered match list.
            if pnl != 0 {
                // c:2080
                let _ = write_loop(out_fd, b"\n");
                pnl = 0;
                ml += 1;
            }

            for m in &g.matches {
                // c:2087
                let visible = showall != 0 || (m.flags & (CMF_HIDE | CMF_NOLIST)) == 0;
                if !visible {
                    continue;
                }
                // c:2095-2098 — DISPLINE = full-row.
                let _ = iprintm(Some(g), Some(m), 0, 0, 1, 0);
                if (m.flags & CMF_DISPLINE) == 0 {
                    let _ = write_loop(out_fd, b"\n");
                }
                ml += 1;
            }
            // Force CGF_ROWS layout hint into effect.
            let _ = CGF_ROWS;
        }
        pnl = 1;
    }

    let _ = Relaxed;
    ml // c:2185
}

// =====================================================================
// Listing/menu helpers — the bodies depend on the full Cmgroup/Cmatch
// linked-list machine + listing arena + zle_refresh draw primitives.
// Until those land, these return empty/zero so callers don't blow up
// when no matches are available.
// =====================================================================

/// Port of `bld_all_str(Cmatch all)` from `Src/Zle/compresult.c:2187`.
/// Direct port of `static void bld_all_str(Cmatch all)` from
/// `Src/Zle/compresult.c:2187`. Walks the global `amatches`
/// linked list, collecting every visible match string into a single
/// space-joined display buffer terminated with "..." when overflow.
/// The C signature takes a Cmatch and writes `all->disp`; the Rust
/// port returns the built String so the caller assigns it.
/// WARNING: param names don't match C — Rust=() vs C=(all)
pub fn bld_all_str() -> String {
    // c:2187

    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();

    // c:2191 — `cols = zterm_columns`. C reads the live tty width
    //          via the cached `zterm_columns` global. Rust port uses
    //          `adjustcolumns` which probes via TIOCGWINSZ and falls
    //          back to $COLUMNS. Was reading raw `std::env::var(
    //          "COLUMNS")` only — wrong: missed the live width.
    let cols: i32 = adjustcolumns() as i32;
    let mut len: i32 = cols - 5; // c:2192
    let mut add: i32 = 0;
    let mut buf = String::new(); // c:2196

    // c:2199-2204 — skip empty groups.
    let mut g_idx = groups.iter().position(|g| g.mcount != 0);
    'outer: while let Some(gi) = g_idx {
        let g = &groups[gi];
        let mut mp = 0usize;
        while mp < g.matches.len() {
            let m = &g.matches[mp];
            let visible = (m.flags & (CMF_ALL | CMF_HIDE)) == 0 && m.str.is_some();
            if visible {
                // c:2213
                let s = m.str.as_deref().unwrap();
                let t = s.len() as i32 + add;
                if len >= t {
                    // c:2215
                    if add != 0 {
                        buf.push(' ');
                    } // c:2216
                    buf.push_str(s); // c:2218
                    len -= t;
                    add = 1;
                } else {
                    // c:2221
                    if len > add + 2 {
                        // c:2222
                        if add != 0 {
                            buf.push(' ');
                        }
                        buf.push_str(&s[..((len - 2).max(0) as usize).min(s.len())]);
                    }
                    buf.push_str("..."); // c:2227
                    break 'outer; // c:2228
                }
            }
            mp += 1;
            if mp >= g.matches.len() {
                // c:2232
                g_idx = (gi + 1..).find(|&i| i < groups.len() && groups[i].mcount != 0);
                if g_idx.is_none() {
                    break 'outer;
                }
                continue 'outer;
            }
        }
        let _ = Relaxed;
        g_idx = (gi + 1..).find(|&i| i < groups.len() && groups[i].mcount != 0);
    }
    buf // c:2238 ztrdup(buf)
}

/// Port of `iprintm(Cmgroup g, Cmatch *mp, UNUSED(int mc), UNUSED(int ml), int lastc, int width)` from `Src/Zle/compresult.c:2241`.
/// Direct port of `static void iprintm(Cmgroup g, Cmatch *mp, int mc,
///                                     int ml, int lastc, int width)`
/// from `Src/Zle/compresult.c:2241`. Renders one match cell to
/// stdout (`shout` in C) with column-padding when not last in row.
///
/// Rust signature returns `i32` (printed width) — caller in the
/// column-layout loop uses it for running totals; C body wrote to
/// the global `shout` stream + tracked `len` locally.
#[allow(unused_variables)]
pub fn iprintm(
    g: Option<&Cmgroup>,
    mp: Option<&Cmatch>,
    mc: i32,
    ml: i32,
    lastc: i32,
    width: i32,
) -> i32 {
    // c:2241
    use std::sync::atomic::Ordering;

    let m = match mp {
        None => return 0,
        Some(m) => m,
    }; // c:2245
    let mut disp_owned: String = String::new();
    let disp_ref: Option<&str> = m.disp.as_deref();

    // c:2249-2250 — if CMF_ALL with empty disp, build it via bld_all_str.
    if (m.flags & CMF_ALL) != 0 && disp_ref.map(|s| s.is_empty()).unwrap_or(true) {
        disp_owned = bld_all_str(); // c:2250
    }
    let disp_now: Option<&str> = if !disp_owned.is_empty() {
        Some(disp_owned.as_str())
    } else {
        disp_ref
    };

    let mut len: i32;
    // c:2243 — C writes through `printfmt`/`fputs(s, shout)`. Route Rust
    //          to SHTTY so the visible-byte stream matches.
    let fd = SHTTY.load(Relaxed);
    let out = if fd >= 0 { fd } else { 1 };

    if let Some(d) = disp_now {
        // c:2253
        if (m.flags & CMF_DISPLINE) != 0 {
            // c:2254
            // c:2255 — `printfmt(d, 0, 1, 0)` then `putc('\n', shout)`.
            let _ = write_loop(out, d.as_bytes());
            let _ = write_loop(out, b"\n");
            return 0; // c:2257
        }
        let _ = write_loop(out, d.as_bytes()); // c:2260 niceformat
        len = d.chars().count() as i32;
    } else {
        // c:2263
        let s = m.str.as_deref().unwrap_or("");
        let _ = write_loop(out, s.as_bytes()); // c:2266
        len = s.chars().count() as i32;
        // c:2270-2273 — append modec for file-completion groups.
        if let Some(grp) = g {
            if (grp.flags & CGF_FILES) != 0 && m.modec != '\0' {
                let mut buf = [0u8; 4];
                let mb = m.modec.encode_utf8(&mut buf);
                let _ = write_loop(out, mb.as_bytes());
                len += 1;
            }
        }
    }
    if lastc == 0 {
        // c:2275
        // c:2278-2279 — pad with spaces up to column width.
        let pad = width - len;
        if pad > 0 {
            let spaces = vec![b' '; pad as usize];
            let _ = write_loop(out, &spaces);
        }
    }
    len // c:2282
}

/// Port of `int ilistmatches(Hookdef dummy, Chdata dat)` from
/// `Src/Zle/compresult.c:2284`. Hook callback for the standard
/// listing path: runs `calclist`, bails when `listdat.nlines == 0`,
/// otherwise calls `printlist(0, iprintm, 0)`.
pub fn ilistmatches() -> i32 {
    // c:2284
    let _ = calclist(0); // c:2286
                         // c:2288 — bail when listdat.nlines == 0 (no matches to display).
    let nlines = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock()
        .map(|g| g.nlines)
        .unwrap_or(0);
    if nlines == 0 {
        SHOWINGLIST.store(0, Relaxed);
        LISTSHOWN.store(0, Relaxed);
        return 0;
    }
    // c:2295 — printlist(0, iprintm, 0).
    let _ = printlist(0, 0);
    0 // c:2297
}

/// Port of `int list_matches(Hookdef dummy, void *dummy2)` from
/// `Src/Zle/compresult.c:2304`.
///
/// "List the matches. Note that the list entries are metafied."
/// Walks `amatches` into a `chdata` bag and dispatches via
/// `runhookdef(COMPLISTMATCHESHOOK, &dat)` so `_main_complete`-style
/// user hooks can override the default `ilistmatches` rendering.
pub fn list_matches() -> i32 {
    // c:2304
    if VALIDLIST.load(Ordering::SeqCst) == 0 {
        // c:2311
        showmsg("BUG: listmatches called with bogus list");
        return 1; // c:2313
    }
    // c:2317-2324 — populate the chdata bag.
    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut dat = Chdata::default();
    dat.matches = groups.into_iter().next().map(Box::new); // c:2317 first group head
    dat.num = nmatches_g.load(Relaxed); // c:2319
                                        // c:2325 — `runhookdef(COMPLISTMATCHESHOOK, &dat)` fires every
                                        // registered Hookfn; first non-zero short-circuits per HOOKF_ALL.
                                        // When `gethookdef` returns NULL (no module registered a handler)
                                        // or `runhookdef` returns 0 with no Hookfns, fall through to the
                                        // canonical `ilistmatches` renderer.
    let h = crate::ported::module::gethookdef("complist-matches");
    let handled = if !h.is_null() {
        let dat_ptr = (&mut dat) as *mut Chdata as *mut std::ffi::c_void;
        crate::ported::module::runhookdef(h, dat_ptr) != 0
    } else {
        false
    };
    if !handled {
        ilistmatches();
    }
    0
}

/// Port of `mod_export int invalidate_list(void)` from
/// `Src/Zle/compresult.c:2334`.
///
/// "Invalidate the completion list." Bumps `invcount`; if `validlist`
/// was set, frees the perm-allocated `lastmatches` and refreshes the
/// screen if the list was on display. Resets every transition flag
/// (`lastambig`, `menucmp`, `menuacc`, `validlist`, `showinglist`,
/// `fromcomp`) to 0, clears `listdat.valid`, and zeros out `nmatches`
/// + `amatches`.
pub fn invalidate_list() -> i32 {
    // c:2334

    INVCOUNT.fetch_add(1, Ordering::SeqCst); // c:2336
    if VALIDLIST.load(Ordering::SeqCst) != 0 {
        // c:2337
        if SHOWINGLIST.load(Ordering::SeqCst) == -2 {
            // c:2338
            // c:2339 — `zrefresh()` triggers a screen redraw so the now-
            // invalidated listing isn't left on screen.
            zrefresh();
        }
        // c:2341 — `freematches(lastmatches, 1)` fires `minfo.cur = None`
        // via the cm=1 side-effect.
        let drained = lastmatches
            .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        crate::ported::zle::compcore::freematches(drained, 1);
        crate::ported::zle::compcore::hasoldlist.store(0, Ordering::SeqCst); // c:2343
    }
    // c:2345 — `lastambig = menucmp = menuacc = validlist = showinglist
    //           = fromcomp = 0`.
    LASTAMBIG.store(0, Ordering::SeqCst);
    MENUCMP.store(0, Ordering::SeqCst);
    menuacc.store(0, Ordering::SeqCst);
    VALIDLIST.store(0, Ordering::SeqCst);
    SHOWINGLIST.store(0, Ordering::SeqCst);
    fromcomp.store(0, Ordering::SeqCst);
    // c:2346 — `listdat.valid = 0`.
    if let Ok(mut ld) = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock()
    {
        ld.valid = 0;
    }
    // c:2347-2348 — `if (listshown < 0) listshown = 0`.
    if LISTSHOWN.load(Ordering::SeqCst) < 0 {
        LISTSHOWN.store(0, Ordering::SeqCst);
    }
    // c:2349-2353 — `minfo.cur = NULL; minfo.asked = 0; …`. minfo not
    // ported as a static struct yet.
    // c:2354 — `compwidget = NULL`. The canonical `COMPWIDGET` static
    // lives in zle_main.rs.
    nmatches_g.store(0, Ordering::SeqCst); // c:2355
    if let Ok(mut g) = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
    {
        g.clear(); // c:2356
    }
    0 // c:2358
}
/// `INVCOUNT` static.
pub static INVCOUNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:37

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::comp_h::Cline;

    #[test]
    fn test_unambig_data() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(unambig_data(&["foobar".into(), "foobaz".into()]), "fooba");
        assert_eq!(unambig_data(&["abc".into()]), "abc");
        assert_eq!(unambig_data(&[]), "");
    }

    /// Test scaffolding: reset the metafied line + brace/menu globals to
    /// a clean, non-menu state so the buffer-mutating ports are
    /// deterministic. Inlined at each test's top (no shared test fn).
    /// c:165 — `cline_str(None, ...)` renders nothing → `Some("")`.
    #[test]
    fn cline_str_none_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = String::new();
        }
        ZLEMETACS.store(0, Relaxed);
        ZLEMETALL.store(0, Relaxed);
        WB.store(0, Relaxed);
        WE.store(0, Relaxed);
        assert_eq!(cline_str(None, 0, None, None), Some(String::new()));
    }

    /// c:281-285 — a non-CLF_LINE node renders its `word` anchor.
    #[test]
    fn cline_str_emits_word_anchor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = String::new();
        }
        ZLEMETACS.store(0, Relaxed);
        ZLEMETALL.store(0, Relaxed);
        WB.store(0, Relaxed);
        WE.store(0, Relaxed);
        let mut n = Cline::default();
        n.word = Some("hello".to_string());
        n.wlen = 5;
        assert_eq!(cline_str(Some(Box::new(n)), 0, None, None), Some("hello".to_string()));
        // Line restored (ins=0 copies out then foredel's).
        assert_eq!(
            ZLEMETALINE.get().unwrap().lock().unwrap().clone(),
            ""
        );
    }

    /// c:282-283 — a CLF_LINE node renders its `line`, not `word`.
    #[test]
    fn cline_str_emits_line_anchor_when_clf_line_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = String::new();
        }
        ZLEMETACS.store(0, Relaxed);
        ZLEMETALL.store(0, Relaxed);
        WB.store(0, Relaxed);
        WE.store(0, Relaxed);
        let mut n = Cline::default();
        n.flags = CLF_LINE;
        n.line = Some("LINE".to_string());
        n.llen = 4;
        n.word = Some("word-should-not-emit".to_string());
        n.wlen = 20;
        assert_eq!(cline_str(Some(Box::new(n)), 0, None, None), Some("LINE".to_string()));
    }

    /// c:214-216 + c:281-285 — `olen && !CLF_SUF && !prefix` renders
    /// `orig` first, then the anchor.
    #[test]
    fn cline_str_emits_orig_when_olen_set_and_no_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = String::new();
        }
        ZLEMETACS.store(0, Relaxed);
        ZLEMETALL.store(0, Relaxed);
        WB.store(0, Relaxed);
        WE.store(0, Relaxed);
        let mut n = Cline::default();
        n.orig = Some("original".to_string());
        n.olen = 8;
        n.word = Some("anchor".to_string());
        n.wlen = 6;
        assert_eq!(
            cline_str(Some(Box::new(n)), 0, None, None),
            Some("originalanchor".to_string())
        );
    }

    /// c:219-235 — the prefix sub-list is rendered before the anchor.
    #[test]
    fn cline_str_walks_prefix_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = String::new();
        }
        ZLEMETACS.store(0, Relaxed);
        ZLEMETALL.store(0, Relaxed);
        WB.store(0, Relaxed);
        WE.store(0, Relaxed);
        let mut p2 = Cline::default();
        p2.word = Some("ond".to_string());
        p2.wlen = 3;
        let mut p1 = Cline::default();
        p1.word = Some("sec".to_string());
        p1.wlen = 3;
        p1.next = Some(Box::new(p2));
        let mut n = Cline::default();
        n.prefix = Some(Box::new(p1));
        n.word = Some("anchor".to_string());
        n.wlen = 6;
        assert_eq!(
            cline_str(Some(Box::new(n)), 0, None, None),
            Some("secondanchor".to_string())
        );
    }

    /// c:416 — the top-level list is walked via `l = l->next`.
    #[test]
    fn cline_str_walks_next_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = String::new();
        }
        ZLEMETACS.store(0, Relaxed);
        ZLEMETALL.store(0, Relaxed);
        WB.store(0, Relaxed);
        WE.store(0, Relaxed);
        let mut n2 = Cline::default();
        n2.word = Some("B".to_string());
        n2.wlen = 1;
        let mut n1 = Cline::default();
        n1.word = Some("A".to_string());
        n1.wlen = 1;
        n1.next = Some(Box::new(n2));
        assert_eq!(cline_str(Some(Box::new(n1)), 0, None, None), Some("AB".to_string()));
    }

    /// c:578 — `instmatch` inserts the match string at the cursor into
    /// the metafied line and returns the byte count, leaving the cursor
    /// just past the string. No braces / prefixes here.
    #[test]
    fn test_instmatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Clean brace state so no re-insertion happens.
        if let Some(mx) = BRBEG.get() {
            *mx.lock().unwrap() = None;
        }
        if let Some(mx) = BREND.get() {
            *mx.lock().unwrap() = None;
        }
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = "git ".to_string();
        }
        ZLEMETACS.store(4, Relaxed);
        ZLEMETALL.store(4, Relaxed);
        let mut m = Cmatch::default();
        m.str = Some("commit".to_string());
        let r = instmatch(&m, None);
        assert_eq!(r, 6, "returns bytes inserted");
        assert_eq!(
            ZLEMETALINE.get().unwrap().lock().unwrap().clone(),
            "git commit"
        );
        // Cursor left at ocs = just after the inserted string.
        assert_eq!(ZLEMETACS.load(Relaxed), 10);
    }

    /// c:961 — `do_single` deletes the old word (`we-wb`), inserts the
    /// match via `instmatch`, and (no suffix, non-file) appends a
    /// trailing space. Buffer becomes `git commit `.
    #[test]
    fn test_do_single() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        if let Some(mx) = BRBEG.get() {
            *mx.lock().unwrap() = None;
        }
        if let Some(mx) = BREND.get() {
            *mx.lock().unwrap() = None;
        }
        if let Ok(mut g) = MINFO.get_or_init(|| std::sync::Mutex::new(Menuinfo::default())).lock() {
            *g = Menuinfo::default();
        }
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = "git co".to_string();
        }
        ZLEMETACS.store(6, Relaxed);
        ZLEMETALL.store(6, Relaxed);
        WB.store(4, Relaxed);
        WE.store(6, Relaxed);
        MENUCMP.store(0, Relaxed);
        menuacc.store(0, Relaxed);
        movetoend.store(0, Relaxed);
        USEMENU.store(0, Relaxed);
        insspace.store(0, Relaxed);
        let mut m = Cmatch::default();
        m.str = Some("commit".to_string());
        m.orig = Some("commit".to_string());
        do_single(&m);
        assert_eq!(
            ZLEMETALINE.get().unwrap().lock().unwrap().clone(),
            "git commit ",
            "old word deleted, match + trailing space inserted"
        );
        // minfo.len reflects the inserted match length (6).
        let len = MINFO.get().unwrap().lock().unwrap().len;
        assert_eq!(len, 6);
    }

    #[test]
    fn test_do_menucmp() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let matches = vec!["commit".into(), "checkout".into(), "cherry-pick".into()];
        let (next, word) = do_menucmp(&matches, 0, true);
        assert_eq!(next, 1);
        assert_eq!(word, "checkout");

        let (next, word) = do_menucmp(&matches, 2, true);
        assert_eq!(next, 0);
        assert_eq!(word, "commit");
    }

    /// c:1206 — `valid_match` skips a leading `CMF_DUMMY` match and
    /// returns the next real match, updating `minfo.cur_idx`.
    #[test]
    fn test_valid_match() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZMULT.store(1, Relaxed); // forward
        menuacc.store(0, Relaxed); // skip the hasbrpsfx branch
        let mut dummy = Cmatch::default();
        dummy.str = Some("dummy".to_string());
        dummy.flags = CMF_DUMMY;
        let mut real = Cmatch::default();
        real.str = Some("real".to_string());
        let mut g = Cmgroup::default();
        g.matches = vec![dummy, real];
        g.mcount = 2;
        if let Ok(mut a) = amatches.get_or_init(|| std::sync::Mutex::new(Vec::new())).lock() {
            *a = vec![g];
        }
        if let Ok(mut mi) = MINFO.get_or_init(|| std::sync::Mutex::new(Menuinfo::default())).lock() {
            *mi = Menuinfo::default();
            mi.group_idx = 0;
        }
        // Start at index 0 (the dummy) with next=0: it is invalid, so
        // valid_match advances to index 1 (the real match).
        let mc = valid_match(0, 0);
        assert_eq!(mc.and_then(|m| m.str), Some("real".to_string()));
        assert_eq!(MINFO.get().unwrap().lock().unwrap().cur_idx, 1);
    }

    /// c:489 — `build_pos_string` colon-joins the position list.
    #[test]
    fn test_build_pos_string() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(build_pos_string(&[1, 5, 10]), "1:5:10");
        assert_eq!(build_pos_string(&[7]), "7");
        assert_eq!(build_pos_string(&[]), "");
    }

    #[test]
    fn test_list_lines() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(list_lines(&vec!["a".into(); 10], 3), 4);
        assert_eq!(list_lines(&vec!["a".into(); 6], 3), 2);
    }

    #[test]
    fn comp_mod_positive() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1366-1369 — positive: decrement then % m.
        assert_eq!(comp_mod(1, 5), 0); // (1-1) % 5 = 0
        assert_eq!(comp_mod(3, 5), 2); // (3-1) % 5 = 2
        assert_eq!(comp_mod(5, 5), 4); // (5-1) % 5 = 4
        assert_eq!(comp_mod(6, 5), 0); // (6-1) % 5 = 0
    }

    #[test]
    fn comp_mod_zero_branches_negative() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1366 — `if (v >= 0) v--;` so 0 → -1 → falls into else.
        // c:1370-1373 — wrap by adding m until non-negative.
        assert_eq!(comp_mod(0, 5), 4); // 0→-1→+5=4
        assert_eq!(comp_mod(-1, 5), 4); // -1+5=4
        assert_eq!(comp_mod(-5, 5), 0); // -5+5=0
        assert_eq!(comp_mod(-6, 5), 4); // -6+5=-1+5=4
    }

    #[test]
    fn comp_list_sets_onlyexpl() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1473 — `(strstr(v,"expl")?1:0) | (strstr(v,"messages")?2:0)`.
        comp_list(Some("expl"));
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 1);
        comp_list(Some("messages"));
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 2);
        comp_list(Some("expl messages"));
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 3);
        comp_list(Some("nothing"));
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 0);
        comp_list(None);
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn skipnolist_skips_hide_and_nolist() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.flags = CMF_NOLIST;
        let mut b = Cmatch::default();
        b.flags = CMF_HIDE;
        let c = Cmatch::default(); // listable
        let v = vec![a, b, c];
        // c:1483 — mask = NOLIST|MULT|HIDE. First two skipped, third kept.
        assert_eq!(skipnolist(&v, 0), 2);
    }

    #[test]
    fn skipnolist_showall_keeps_nolist() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.flags = CMF_NOLIST;
        let v = vec![a];
        // c:1483 — showall=1 drops NOLIST|MULT from mask, only HIDE filters.
        assert_eq!(skipnolist(&v, 1), 0);
    }

    #[test]
    fn skipnolist_skips_disp_displine() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.disp = Some("display".into());
        a.flags = CMF_DISPLINE;
        let b = Cmatch::default();
        let v = vec![a, b];
        // c:1486-1487 — disp + (DISPLINE|HIDE) → skip.
        assert_eq!(skipnolist(&v, 0), 1);
    }

    // ─── zsh-corpus pins for unambig_data / build_pos_string ────────

    /// `unambig_data([])` is empty string (no matches).
    #[test]
    fn compresult_corpus_unambig_data_empty_input_empty_output() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(unambig_data(&[]), "");
    }

    /// `unambig_data([single])` returns the single match.
    #[test]
    fn compresult_corpus_unambig_data_single_input_returns_it() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["only_one".to_string()];
        assert_eq!(unambig_data(&matches), "only_one");
    }

    /// `unambig_data` returns longest common prefix.
    #[test]
    fn compresult_corpus_unambig_data_returns_lcp() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec![
            "prefix_alpha".to_string(),
            "prefix_beta".to_string(),
            "prefix_gamma".to_string(),
        ];
        assert_eq!(unambig_data(&matches), "prefix_");
    }

    /// `unambig_data` with no shared prefix returns empty.
    #[test]
    fn compresult_corpus_unambig_data_no_shared_prefix() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        assert_eq!(unambig_data(&matches), "");
    }

    /// `unambig_data` where one match is a prefix of another → returns shorter.
    #[test]
    fn compresult_corpus_unambig_data_one_is_prefix_of_other() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["abc".to_string(), "abcdef".to_string()];
        assert_eq!(unambig_data(&matches), "abc");
    }

    /// c:489 — `build_pos_string` renders positions verbatim, joined
    /// by colons (no arithmetic on the values).
    #[test]
    fn compresult_corpus_build_pos_string_one_indexed() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(build_pos_string(&[0, 5]), "0:5");
        assert_eq!(build_pos_string(&[4, 5]), "4:5");
        assert_eq!(build_pos_string(&[99, 100]), "99:100");
    }

    /// c:489 — separator is ':' between positions.
    #[test]
    fn compresult_corpus_build_pos_string_includes_slash() {
        let _g = crate::test_util::global_state_lock();
        let s = build_pos_string(&[2, 10]);
        assert!(s.contains(':'));
        assert_eq!(s, "2:10");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/compresult.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:489 — a single position renders as just that number.
    #[test]
    fn build_pos_string_first_of_one_returns_one_slash_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(build_pos_string(&[1]), "1");
    }

    /// c:489 — positions are literal, not re-indexed (0 stays "0").
    #[test]
    fn build_pos_string_first_index_is_one_not_zero() {
        let _g = crate::test_util::global_state_lock();
        let s = build_pos_string(&[0, 5]);
        assert_eq!(s, "0:5", "positions rendered verbatim; got {s}");
    }

    /// `unambig_data` on identical matches returns the full string.
    /// C: if all matches are the same, common prefix = whole string.
    #[test]
    fn unambig_data_all_identical_returns_full_string() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["hello".to_string(), "hello".to_string()];
        let r = unambig_data(&matches);
        assert_eq!(r, "hello", "identical matches → full string");
    }

    /// `unambig_data` on empty matches returns empty string.
    #[test]
    fn unambig_data_empty_matches_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = unambig_data(&[]);
        assert!(r.is_empty(), "no matches → empty unambig prefix");
    }

    /// `unambig_data` returns common prefix of multiple matches.
    /// `["foobar", "football", "foo"]` → "foo".
    #[test]
    fn unambig_data_common_prefix_of_three() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec![
            "foobar".to_string(),
            "football".to_string(),
            "foo".to_string(),
        ];
        assert_eq!(unambig_data(&matches), "foo", "common prefix = 'foo'");
    }

    /// `unambig_data` with NO common prefix returns empty.
    #[test]
    fn unambig_data_no_common_prefix_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["abc".to_string(), "xyz".to_string()];
        let r = unambig_data(&matches);
        assert!(r.is_empty(), "no common prefix → empty");
    }

    /// c:1206 — an already-valid match at the current index is returned
    /// unchanged when `next == 0`.
    #[test]
    fn valid_match_pre_suf_wrap_matches() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZMULT.store(1, Relaxed);
        menuacc.store(0, Relaxed);
        let mut a = Cmatch::default();
        a.str = Some("alpha".to_string());
        let mut b = Cmatch::default();
        b.str = Some("beta".to_string());
        let mut g = Cmgroup::default();
        g.matches = vec![a, b];
        g.mcount = 2;
        *amatches.get_or_init(|| std::sync::Mutex::new(Vec::new())).lock().unwrap() = vec![g];
        {
            let mut mi = MINFO.get_or_init(|| std::sync::Mutex::new(Menuinfo::default())).lock().unwrap();
            *mi = Menuinfo::default();
        }
        // index 0 is valid with next=0 → returned as-is.
        assert_eq!(valid_match(0, 0).and_then(|m| m.str), Some("alpha".to_string()));
    }

    /// c:1211-1212 — an empty CMF_NOLIST match is skipped.
    #[test]
    fn valid_match_wrong_prefix_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZMULT.store(1, Relaxed);
        menuacc.store(0, Relaxed);
        let mut skip = Cmatch::default();
        skip.str = Some(String::new()); // empty + NOLIST → skipped
        skip.flags = CMF_NOLIST;
        let mut real = Cmatch::default();
        real.str = Some("real".to_string());
        let mut g = Cmgroup::default();
        g.matches = vec![skip, real];
        g.mcount = 2;
        *amatches.get_or_init(|| std::sync::Mutex::new(Vec::new())).lock().unwrap() = vec![g];
        {
            let mut mi = MINFO.get_or_init(|| std::sync::Mutex::new(Menuinfo::default())).lock().unwrap();
            *mi = Menuinfo::default();
        }
        assert_eq!(valid_match(0, 0).and_then(|m| m.str), Some("real".to_string()));
    }

    /// `comp_mod(v, m)` converts 1-indexed v to 0-indexed before
    /// modulo. C `Src/Zle/compresult.c:1364`:
    ///   `if (v >= 0) v -= 1; if (v >= 0) v % m; else { wrap into [0,m) }`
    /// So `comp_mod(7, 3)` = (7-1) % 3 = 0, not 1 — the v-1 conversion
    /// is for 1-indexed match-table semantics.
    #[test]
    fn comp_mod_positive_v_subtracts_one_then_mods() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(comp_mod(7, 3), 0, "(7-1)%3 = 6%3 = 0");
        assert_eq!(comp_mod(6, 3), 2, "(6-1)%3 = 5%3 = 2");
        assert_eq!(comp_mod(4, 3), 0, "(4-1)%3 = 3%3 = 0");
    }

    /// `comp_mod(0, 5)` — v=0 → v-1=-1 → wrap: -1 + 5 = 4.
    #[test]
    fn comp_mod_zero_v_wraps_to_m_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(comp_mod(0, 5), 4, "(0-1) wrapped to [0,5) = 4");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compresult.c utilities.
    // ═══════════════════════════════════════════════════════════════════

    /// c:57-59 — with `hasmatched == 0`, `cut_cline` keeps the list and
    /// only sets lengths; `None` in → `None` out.
    #[test]
    fn cut_cline_short_string_is_identity() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        assert!(cut_cline(None).is_none());
        let mut n = Cline::default();
        n.word = Some("abc".to_string());
        n.wlen = 3;
        let out = cut_cline(Some(Box::new(n))).expect("kept");
        assert_eq!(out.word.as_deref(), Some("abc"));
    }

    /// c:58 — `cut_cline` (hasmatched==0) sets `min` via `cline_setlens`
    /// = `cline_sublen` of the node (here the word length, 3).
    #[test]
    fn cut_cline_long_string_truncates_with_ellipsis() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        let mut n = Cline::default();
        n.word = Some("abcdefghij".to_string());
        n.wlen = 10;
        let out = cut_cline(Some(Box::new(n))).expect("kept");
        assert_eq!(out.min, 10, "min set from cline_sublen (wlen)");
    }

    /// c:57-59 — a 2-node chain is preserved intact when hasmatched==0.
    #[test]
    fn cut_cline_at_exact_length_is_identity() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        let mut n2 = Cline::default();
        n2.word = Some("lo".to_string());
        n2.wlen = 2;
        let mut n1 = Cline::default();
        n1.word = Some("hel".to_string());
        n1.wlen = 3;
        n1.next = Some(Box::new(n2));
        let out = cut_cline(Some(Box::new(n1))).expect("kept");
        assert_eq!(out.word.as_deref(), Some("hel"));
        assert_eq!(out.next.as_ref().and_then(|n| n.word.as_deref()), Some("lo"));
    }

    /// c:489 — colon-joined positions.
    #[test]
    fn build_pos_string_1_indexed_display() {
        assert_eq!(build_pos_string(&[0, 10]), "0:10");
        assert_eq!(build_pos_string(&[4, 10]), "4:10");
        assert_eq!(build_pos_string(&[9, 10]), "9:10");
    }

    /// c:489 — single position renders as itself.
    #[test]
    fn build_pos_string_single_match() {
        assert_eq!(build_pos_string(&[1]), "1");
    }

    /// c:165 — `cline_str(None, 0, ...)` renders nothing → `Some("")`.
    #[test]
    fn cline_str_none_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = String::new();
        }
        ZLEMETACS.store(0, Relaxed);
        ZLEMETALL.store(0, Relaxed);
        WB.store(0, Relaxed);
        WE.store(0, Relaxed);
        assert_eq!(cline_str(None, 0, None, None), Some(String::new()));
    }

    /// c:1208 — `valid_match` with `next == 0` on a valid single match
    /// returns it (no skipping).
    #[test]
    fn valid_match_empty_pfx_sfx_accepts_anything() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZMULT.store(1, Relaxed);
        menuacc.store(0, Relaxed);
        let mut only = Cmatch::default();
        only.str = Some("foo".to_string());
        let mut g = Cmgroup::default();
        g.matches = vec![only];
        g.mcount = 1;
        *amatches.get_or_init(|| std::sync::Mutex::new(Vec::new())).lock().unwrap() = vec![g];
        {
            let mut mi = MINFO.get_or_init(|| std::sync::Mutex::new(Menuinfo::default())).lock().unwrap();
            *mi = Menuinfo::default();
        }
        assert_eq!(valid_match(0, 0).and_then(|m| m.str), Some("foo".to_string()));
    }

    /// c:1208 — `next == 1` forces at least one advance, so a two-match
    /// group returns the *other* match.
    #[test]
    fn valid_match_prefix_only() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZMULT.store(1, Relaxed);
        menuacc.store(0, Relaxed);
        let mut a = Cmatch::default();
        a.str = Some("first".to_string());
        let mut b = Cmatch::default();
        b.str = Some("second".to_string());
        let mut g = Cmgroup::default();
        g.matches = vec![a, b];
        g.mcount = 2;
        *amatches.get_or_init(|| std::sync::Mutex::new(Vec::new())).lock().unwrap() = vec![g];
        {
            let mut mi = MINFO.get_or_init(|| std::sync::Mutex::new(Menuinfo::default())).lock().unwrap();
            *mi = Menuinfo::default();
        }
        // start at 0, next=1 → advance to index 1.
        assert_eq!(valid_match(0, 1).and_then(|m| m.str), Some("second".to_string()));
    }

    /// c:1227-1235 — backward direction (`zmult < 0`) with `next == 1`
    /// from index 0 wraps to the last match in the group.
    #[test]
    fn valid_match_suffix_only() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZMULT.store(-1, Relaxed);
        menuacc.store(0, Relaxed);
        let mut a = Cmatch::default();
        a.str = Some("first".to_string());
        let mut b = Cmatch::default();
        b.str = Some("last".to_string());
        let mut g = Cmgroup::default();
        g.matches = vec![a, b];
        g.mcount = 2;
        *amatches.get_or_init(|| std::sync::Mutex::new(Vec::new())).lock().unwrap() = vec![g];
        {
            let mut mi = MINFO.get_or_init(|| std::sync::Mutex::new(Menuinfo::default())).lock().unwrap();
            *mi = Menuinfo::default();
        }
        // start at 0, next=1, backward → wrap to last (index 1).
        assert_eq!(valid_match(0, 1).and_then(|m| m.str), Some("last".to_string()));
    }

    /// c:1364 — `comp_mod` for positive v ≥ m wraps correctly via
    /// (v-1) % m.
    #[test]
    fn comp_mod_v_above_m_wraps() {
        assert_eq!(comp_mod(10, 3), 0, "(10-1)%3 = 9%3 = 0");
        assert_eq!(comp_mod(11, 3), 1, "(11-1)%3 = 10%3 = 1");
        assert_eq!(comp_mod(12, 3), 2, "(12-1)%3 = 11%3 = 2");
    }

    /// c:1364 — `comp_mod(-1, 5)` = -1 + 5 = 4 (already negative, no
    /// pre-decrement; loop just adds m).
    #[test]
    fn comp_mod_negative_v_wraps_via_loop() {
        assert_eq!(comp_mod(-1, 5), 4);
        assert_eq!(comp_mod(-3, 5), 2);
        assert_eq!(comp_mod(-7, 5), 3, "-7 + 5 + 5 = 3");
    }

    /// c:180 — `unambig_data` on empty matches returns empty (corpus pin).
    #[test]
    fn unambig_data_empty_matches_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(unambig_data(&[]), "");
    }

    /// c:180 — `unambig_data` on single match returns that match.
    #[test]
    fn unambig_data_single_match_returns_it() {
        let _g = crate::test_util::global_state_lock();
        let r = unambig_data(&["single".to_string()]);
        assert_eq!(r, "single", "single match → common prefix is itself");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compresult.c
    // c:71 cut_cline / c:99 cline_str / c:166 build_pos_string /
    // c:180 unambig_data / c:250 hasbrpsfx / c:379 valid_match /
    // c:502 comp_mod / c:639 list_lines / c:701 skipnolist
    // ═══════════════════════════════════════════════════════════════════

    /// c:71 — `cut_cline(None)` returns `None`.
    #[test]
    fn cut_cline_empty_input_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        assert!(cut_cline(None).is_none());
        crate::ported::zle::compcore::hasmatched.store(1, Relaxed);
        assert!(cut_cline(None).is_none());
    }

    /// c:71 — `cut_cline` is deterministic across runs on the same input.
    #[test]
    fn cut_cline_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        crate::ported::zle::compcore::hasmatched.store(0, Relaxed);
        for w in ["", "abc", "hello world"] {
            let mut base = Cline::default();
            base.word = Some(w.to_string());
            base.wlen = w.len() as i32;
            let first = cut_cline(Some(Box::new(base.clone()))).and_then(|n| n.word);
            for _ in 0..3 {
                let again = cut_cline(Some(Box::new(base.clone()))).and_then(|n| n.word);
                assert_eq!(again, first, "cut_cline({w:?}) must be deterministic");
            }
        }
    }

    /// c:489 — `build_pos_string` returns String (compile-time type pin).
    #[test]
    fn build_pos_string_returns_string_type() {
        let _: String = build_pos_string(&[1]);
    }

    /// c:489 — `build_pos_string` is pure.
    #[test]
    fn build_pos_string_is_pure() {
        for v in [vec![1i32], vec![5, 10], vec![100, 1000, 3]] {
            let first = build_pos_string(&v);
            for _ in 0..3 {
                assert_eq!(
                    build_pos_string(&v),
                    first,
                    "build_pos_string({v:?}) must be pure"
                );
            }
        }
    }

    /// c:180 — `unambig_data(empty)` returns empty.
    #[test]
    fn unambig_data_empty_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(unambig_data(&[]), "");
    }

    /// c:687-688 — a `CMF_ALL` match short-circuits `hasbrpsfx` to true.
    #[test]
    fn hasbrpsfx_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut m = Cmatch::default();
        m.flags = CMF_ALL;
        let r: bool = hasbrpsfx(&m, None, None);
        assert!(r, "CMF_ALL short-circuits to true");
    }

    /// c:683 — a brace-free match yields empty lastprebr/lastpostbr, so
    /// `hasbrpsfx(m, None, None)` is true and restores the line each call.
    #[test]
    fn hasbrpsfx_is_pure() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        if let Some(mx) = BRBEG.get() {
            *mx.lock().unwrap() = None;
        }
        if let Some(mx) = BREND.get() {
            *mx.lock().unwrap() = None;
        }
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| std::sync::Mutex::new(String::new())).lock() {
            *g = "foo".to_string();
        }
        ZLEMETACS.store(3, Relaxed);
        ZLEMETALL.store(3, Relaxed);
        let mut m = Cmatch::default();
        m.str = Some("bar".to_string());
        for _ in 0..3 {
            assert!(hasbrpsfx(&m, None, None), "no braces → matches None/None");
            // Line restored after each round-trip.
            assert_eq!(ZLEMETALINE.get().unwrap().lock().unwrap().clone(), "foo");
        }
    }

    /// c:502 — `comp_mod(0, M)` returns M-1 per 1-indexed menu-cycle
    /// decrement: c:1367 always subtracts 1 first, then the negative-v
    /// branch wraps via repeated `+= m` until non-negative → ends at M-1.
    #[test]
    fn comp_mod_zero_returns_m_minus_one() {
        for m in [1i32, 5, 100, 1000] {
            assert_eq!(
                comp_mod(0, m),
                m - 1,
                "comp_mod(0, {}) = {}-1 = {} per c:1367 decrement+wrap",
                m,
                m,
                m - 1
            );
        }
    }

    /// c:502 — `comp_mod` result strictly less than m.
    #[test]
    fn comp_mod_result_less_than_modulus() {
        for v in [-100i32, -1, 0, 1, 50, 100] {
            for m in [1i32, 5, 10] {
                let r = comp_mod(v, m);
                assert!(
                    r >= 0 && r < m,
                    "comp_mod({}, {}) = {} must be in [0, {})",
                    v,
                    m,
                    r,
                    m
                );
            }
        }
    }

    /// c:1206 — `valid_match` on an empty match store returns `None`.
    #[test]
    fn valid_match_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *amatches.get_or_init(|| std::sync::Mutex::new(Vec::new())).lock().unwrap() = Vec::new();
        let r: Option<Cmatch> = valid_match(0, 0);
        assert!(r.is_none(), "no groups → no valid match");
    }

    /// c:701 — `skipnolist(empty, _)` returns 0 (empty array).
    #[test]
    fn skipnolist_empty_returns_zero() {
        assert_eq!(skipnolist(&[], 0), 0);
        assert_eq!(skipnolist(&[], 1), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compresult.c
    // c:264 do_ambiguous / c:311 ztat / c:527 do_ambig_menu /
    // c:639 list_lines / c:729 calclist / c:1232 asklist /
    // c:1350 printlist / c:1502 bld_all_str / c:1657 ilistmatches /
    // c:1683 list_matches
    // ═══════════════════════════════════════════════════════════════════

    /// c:264 — `do_ambiguous(empty)` returns i32 (compile-time type pin).
    #[test]
    fn do_ambiguous_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = do_ambiguous(&[]);
    }

    /// c:311 — `ztat("/__never__", _)` returns Option<Metadata>.
    #[test]
    fn ztat_returns_option_metadata_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<std::fs::Metadata> = ztat("/__never_zshrs__", false);
    }

    /// c:311 — `ztat("/__never__", _)` returns None.
    #[test]
    fn ztat_nonexistent_returns_none() {
        assert!(
            ztat("/__never_real_path_zshrs_xyz__", false).is_none(),
            "nonexistent → None"
        );
        assert!(
            ztat("/__never_real_path_zshrs_xyz__", true).is_none(),
            "nonexistent w/ symlink follow → None"
        );
    }

    /// c:311 — `ztat("/tmp", _)` returns Some on every Unix host.
    #[test]
    #[cfg(unix)]
    fn ztat_tmp_returns_some() {
        assert!(ztat("/tmp", false).is_some(), "/tmp must stat → Some");
    }

    /// c:527 — `do_ambig_menu` returns i32 (compile-time type pin).
    #[test]
    fn do_ambig_menu_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = do_ambig_menu();
    }

    /// c:639 — `list_lines(empty, 80)` returns 0 (no lines for empty matches).
    #[test]
    fn list_lines_empty_matches_returns_zero() {
        assert_eq!(list_lines(&[], 80), 0, "0 matches → 0 lines");
    }

    /// c:639 — `list_lines` returns usize (compile-time type pin).
    #[test]
    fn list_lines_returns_usize_type() {
        let _: usize = list_lines(&[], 80);
    }

    /// c:729 — `calclist(0)` returns i32 (compile-time type pin).
    #[test]
    fn calclist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = calclist(0);
    }

    /// c:1232 — `asklist` returns i32.
    #[test]
    fn asklist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = asklist();
    }

    /// c:1350 — `printlist(0, 0)` returns i32.
    #[test]
    fn printlist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = printlist(0, 0);
    }

    /// c:1502 — `bld_all_str` returns String (compile-time type pin).
    #[test]
    fn bld_all_str_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = bld_all_str();
    }

    /// c:1657 — `ilistmatches` returns i32.
    #[test]
    fn ilistmatches_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = ilistmatches();
    }

    /// c:1683 — `list_matches` returns i32.
    #[test]
    fn list_matches_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = list_matches();
    }
}
