//! Direct port of `Src/Zle/compcore.c` — completion core code.
//!
//! Original C copyright: Sven Wischnowsky 1995-1997.
//!
//! C source is 3,638 lines. This file ports:
//!   - the file-scope globals (c:36-279)
//!   - the pure-string helpers (`rembslash`, `remsquote`,
//!     `comp_quoting_string`, `multiquote`, `tildequote`, `matcheq`,
//!     `matchcmp`, `ctokenize`, `comp_str`)
//!   - the linked-list group manipulators (`begcmgroup`,
//!     `endcmgroup`, `addexpl`, `addmatch`)
//!   - the param-table helpers (`get_user_var`, `get_data_arr`,
//!     `set_list_array`)
//!   - the hook entry points (`before_complete`, `after_complete`)
//!     in their non-runhookdef branches
//!
//! Functions blocked on heavier substrate (`do_completion`,
//! `makecomplist`, `addmatches`, `callcompfunc`, `set_comp_sep`,
//! `check_param`, `permmatches`, `dupmatch`, `add_match_data`,
//! `makearray`) carry doc comments naming the missing dependencies.

#![allow(non_snake_case, non_upper_case_globals, dead_code)]

use crate::ported::context::zcontext_restore_partial;
use crate::ported::module::{gethookdef, runhookdef};
use crate::ported::params::{getsparam, paramtab, paramtab_hashed_storage, setaparam, setsparam};
use crate::ported::signals::{queue_signals, unqueue_signals};
use crate::ported::zle::comp_h::{
    Aminfo, Brinfo, Cadata, Ccmakedat, Cexpl, Cline, Cmatch, Cmgroup, Cmlist, Menuinfo, CAF_ALL,
    CAF_ARRAYS, CAF_KEYS, CAF_MATCH, CAF_MATSORT, CAF_NOSORT, CAF_NUMSORT, CAF_QUOTE, CAF_REVSORT,
    CAF_UNIQALL, CAF_UNIQCON, CGF_MATSORT, CGF_NOSORT, CGF_NUMSORT, CGF_REVSORT, CGF_UNIQALL,
    CGF_UNIQCON, CMF_DELETE, CMF_DISPLINE, CMF_FMULT, CMF_MULT, CMF_NOLIST, CMF_PACKED, CMF_PARBR,
    CMF_PARNEST, CMF_ROWS,
};
use crate::ported::zle::complete::{
    COMPIPREFIX, COMPLIST, COMPPREFIX, COMPQSTACK, COMPSUFFIX, INCOMPFUNC,
};
use crate::ported::zle::compmatch::{bld_parts, cline_matched};
use crate::ported::zle::compresult::{do_ambig_menu, ztat};
use crate::ported::zle::zle_h::{invalidatelist, COMP_LIST_COMPLETE, COMP_LIST_EXPAND, CUT_RAW};
use crate::ported::zle::zle_refresh::{CLEARLIST, SHOWINGLIST};
use crate::ported::zle::zle_tricky::{
    inststr, MENUCMP, ORIGCS, ORIGLINE, USEGLOB, USEMENU, VALIDLIST, WOULDINSTAB,
};
use crate::ported::zle::zle_utils::foredel;
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_h::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::{
    isset, Bnull, Dnull, Equals, Hat, Inbrace, Inbrack, Inpar, Outbrace, Outpar, Pound, Qstring,
    Quest, Snull, Star, Stringg, Tilde, BASHAUTOLIST, NUMERICGLOBSORT, PM_HASHED, PM_TYPE,
    QT_BACKSLASH, QT_DOLLARS, QT_DOUBLE, QT_NONE, QT_SINGLE, RCQUOTES, SORTIT_IGNORING_BACKSLASHES,
    SORTIT_NUMERICALLY, ZCONTEXT_HIST, ZCONTEXT_LEX, ZCONTEXT_PARSE,
};
use crate::DPUTS;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

// =====================================================================
// Substrate-blocked stubs — bodies need substrate listed in each
// doc comment. Returns shape-correct safe defaults.
// =====================================================================

// =====================================================================
// do_completion — `Src/Zle/compcore.c:287`.
// =====================================================================

/// Direct port of `int do_completion(Hookdef dummy, Compldat dat)`
/// from compcore.c:287. The top-level completion driver: per-round
/// state reset → `makecomplist` → dispatch to `do_ambiguous` /
/// `do_single` / `do_allmatches` per result count.
pub fn do_completion(s: &str, incmd: i32, lst: i32) -> i32 {
    // c:287

    // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
    // Native (Rust) plugins that registered completions via
    // `register_completion` (src/extensions/plugin_host.rs) have their
    // compsys `compdef` wiring deferred to here — the completion pipeline
    // is a safe point to eval the glue (compsys itself evals here),
    // whereas evaling during `zmodload -R` plugin-init hangs the VM.
    // Idempotent + cheap: no-ops once the pending queue is drained.
    crate::plugin_host::flush_pending_completions();

    let osl = SHOWINGLIST.load(Ordering::Relaxed); // c:289
    let mut ret: i32 = 0; // c:289

    // c:296-297 — `ainfo = fainfo = NULL`.
    if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
        *g = None;
    }
    if let Ok(mut g) = fainfo.get_or_init(|| Mutex::new(None)).lock() {
        *g = None;
    }
    if let Ok(mut g) = matchers.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear(); // c:298
    }

    // c:300-307 — compqstack reset.
    let instring = INSTRING.load(Ordering::Relaxed); // c:307
                                                     // c:305 — `compqstack = instring == QT_NONE ? "\\" : <quote-char>`.
                                                     // Inlined `char_from_qt(x)` as `(x as u8) as char`.
    let head_q: char = if instring == QT_NONE {
        // c:305
        QT_BACKSLASH as u8 as char
    } else {
        instring as u8 as char
    };
    if let Ok(mut g) = COMPQSTACK.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = head_q.to_string(); // c:305-306
    }
    // !!! RUST-ONLY LINE — NO C COUNTERPART !!!
    // In C, `$compstate[all_quotes]` has NO storage of its own: its
    // `compkparams` row is `{ "all_quotes", PM_SCALAR | PM_READONLY, NULL,
    // GSU(compqstack_gsu) }` (complete.c:1299) and `compqstack_gsu`
    // (complete.c:1242-1243) routes every read through `get_compqstack`
    // (complete.c:1479) against the live `compqstack` global — so the
    // c:305-306 assignment IS the parameter update. zshrs splits the two: a
    // single-key `${compstate[KEY]}` read comes straight out of
    // `paramtab_hashed_storage` (`src/ported/subst.rs:7034-7044`), which
    // special-cases only `nmatches`, so nothing ever published `all_quotes`
    // and it read EMPTY where zsh gives `\`, `"`, `'` (`_cmdambivalent`
    // sh:47 and the documented `compquote` idiom both read it). Run the
    // getter and store its result at each `compqstack` write.
    set_compstate_str(
        "all_quotes",
        &crate::ported::zle::complete::get_compqstack(std::ptr::null_mut()),
    );

    hasunqu.store(0, Ordering::Relaxed); // c:309
    let wouldinstab_v = WOULDINSTAB.load(Ordering::Relaxed); // c:310
    useline.store(
        // c:310
        if wouldinstab_v != 0 {
            -1
        } else if lst != COMP_LIST_COMPLETE {
            1
        } else {
            0
        },
        Ordering::Relaxed,
    );
    useexact.store(opt_isset("RECEXACT"), Ordering::Relaxed); // c:311
    set_compstate_str("exact_string", ""); // c:312
    let useline_v = useline.load(Ordering::Relaxed);
    uselist.store(
        // c:314
        if useline_v != 0 {
            if opt_isset("AUTOLIST") != 0 && opt_isset("BASHAUTOLIST") == 0 {
                if opt_isset("LISTAMBIGUOUS") != 0 {
                    3
                } else {
                    2
                }
            } else {
                0
            }
        } else {
            1
        },
        Ordering::Relaxed,
    );

    let useglob_v = USEGLOB.load(Ordering::Relaxed); // c:319
    let opm: String = if useglob_v != 0 {
        "*".into()
    } else {
        "".into()
    };
    if let Ok(mut g) = comppatmatch.get_or_init(|| Mutex::new(None)).lock() {
        *g = Some(opm.clone()); // c:319
    }
    // c:320-321 — `zsfree(comppatinsert); comppatinsert = ztrdup("menu");`
    // `comppatinsert` is a plain module global (`complete.c:69`) that the
    // `$compstate[pattern_insert]` entry is only a `VAL()` VIEW onto
    // (`complete.c:1281`), so it OUTLIVES the completion widget's parameter
    // scope. Writing only the parameter — as this port did — meant
    // `do_ambiguous`'s GLOB_COMPLETE test (`compresult.c:764`) read it back
    // absent after `endparamscope()` deleted `$compstate`, so that whole
    // branch was dead. Keep the global and the parameter in step here, and
    // re-sync the global from the parameter after the completion function
    // returns (the c:843-925 unwind below).
    if let Ok(mut g) = crate::ported::zle::complete::COMPPATINSERT
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        *g = "menu".into(); // c:321
    }
    set_compstate_str("pattern_insert", "menu"); // c:320
    forcelist.store(0, Ordering::Relaxed); // c:322
    haspattern.store(0, Ordering::Relaxed); // c:323
                                            // c:324 — complistmax mirrors the LISTMAX parameter for every
                                            // completion; asklist reads it to decide when to prompt "do you wish
                                            // to see all N possibilities?". Leaving the static at 0 made large
                                            // command lists (`l<Tab>`, 230 matches) dump without asking.
    crate::ported::zle::complete::COMPLISTMAX
        .store(env_iparam("LISTMAX") as i64, Ordering::Relaxed); // c:324

    set_compstate_str(
        // c:326
        "last_prompt",
        if opt_isset("ALWAYSLASTPROMPT") != 0 {
            "yes"
        } else {
            ""
        },
    );
    dolastprompt.store(1, Ordering::Relaxed); // c:327

    // c:329-330 — complist string.
    let cl_str = if opt_isset("LISTROWSFIRST") != 0 {
        if opt_isset("LISTPACKED") != 0 {
            "packed rows"
        } else {
            "rows"
        }
    } else if opt_isset("LISTPACKED") != 0 {
        "packed"
    } else {
        ""
    };
    if let Ok(mut g) = COMPLIST.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = cl_str.into(); // c:329
    }
    startauto.store(opt_isset("AUTOMENU"), Ordering::Relaxed); // c:331

    let zlc = ZLEMETACS.load(Ordering::Relaxed);
    let we_v = WE.load(Ordering::Relaxed);
    movetoend.store(
        // c:332
        if zlc == we_v || opt_isset("ALWAYSTOEND") != 0 {
            2
        } else {
            1
        },
        Ordering::Relaxed,
    );
    SHOWINGLIST.store(0, Ordering::Relaxed); // c:333
    hasmatched.store(0, Ordering::Relaxed); // c:334
    hasunmatched.store(0, Ordering::Relaxed); // c:334
    minmlen.store(1_000_000, Ordering::Relaxed); // c:335
    maxmlen.store(-1, Ordering::Relaxed); // c:336
                                          // c:337 — `compignored = 0`. This line was absent, so the counter
                                          // behind `$compstate[ignored]` (complete.c:41, exported through the
                                          // compstate table at complete.c:1300) accumulated across every
                                          // completion of the session instead of counting only THIS round's
                                          // ignored-pattern rejections. `_ignored` gates its whole body on
                                          // `[[ … || $compstate[ignored] -eq 0 ]] && return 1`, so a stale
                                          // non-zero made it run on rounds that ignored nothing.
    crate::ported::zle::complete::COMPIGNORED.store(0, Ordering::Relaxed); // c:337
    nmessages.store(0, Ordering::Relaxed); // c:338
    hasallmatch.store(0, Ordering::Relaxed); // c:339

    // c:342 — main dispatch.
    if makecomplist(s, incmd, lst) != 0 {
        // c:342
        // c:344 — error path.
        ZLEMETACS.store(0, Ordering::Relaxed); // c:344
        foredel(ZLEMETALL.load(Ordering::Relaxed), CUT_RAW); // c:345 — `foredel(zlemetall, CUT_RAW)`
        let _ = inststr(
            &ORIGLINE
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default(),
        ); // c:346 — `inststr(origline)`
        ZLEMETACS.store(ORIGCS.load(Ordering::Relaxed), Ordering::Relaxed); // c:347
        CLEARLIST.store(1, Ordering::Relaxed); // c:348
        ret = 1;
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(Menuinfo::default())).lock() {
            g.cur = None;
        } // c:350
        if useline.load(Ordering::Relaxed) < 0 {
            // c:352
            unmetafy_line(); // c:354
                             // zshrs bridge, no C counterpart: C has ONE line buffer —
                             // `zleline`/`zlecs`/`zlell` (zle_main.c:43,48) — which `doinsert`
                             // (zle_misc.c:37,51) writes and `metafy_line`/`unmetafy_line`
                             // (zle_tricky.c:978,995) convert, so C's sandwich round-trips
                             // through the same characters. This port splits it into the editor
                             // buffer (`zle_main::ZLELINE`, the Vec<char> `selfinsert` writes —
                             // zle_misc.rs:205) and the completion staging buffer
                             // (`compcore::ZLELINE`, the only one the two conversions read), so
                             // the two copies below hand the unmetafied line to the widget and
                             // take the edited line back. Without them the inserted character
                             // landed in a buffer `metafy_line` never reads and vanished: TAB on
                             // an all-blank line inserted nothing where zsh inserts a literal
                             // TAB (the `wouldinstab` path, zle_tricky.c:183-197 → c:311 →
                             // c:782 → `_main_complete`'s insert-tab early return → c:860).
                             // Same idiom as docomplete's entry bridge (zle_tricky.rs:857).
            {
                let comp_line: Vec<char> = ZLELINE
                    .get_or_init(|| Mutex::new(String::new()))
                    .lock()
                    .map(|g| g.chars().collect())
                    .unwrap_or_default();
                let comp_ll = comp_line.len();
                let comp_cs = ZLECS.load(Ordering::SeqCst).clamp(0, comp_ll as i32) as usize;
                if let Ok(mut g) = crate::ported::zle::zle_main::ZLELINE.lock() {
                    *g = comp_line;
                }
                crate::ported::zle::zle_main::ZLECS.store(comp_cs, Ordering::SeqCst);
                crate::ported::zle::zle_main::ZLELL.store(comp_ll, Ordering::SeqCst);
            }
            ret = selfinsert(&[]); // c:355
            {
                let ed_line: String = crate::ported::zle::zle_main::ZLELINE
                    .lock()
                    .map(|g| g.iter().collect())
                    .unwrap_or_default();
                let ed_ll = ed_line.chars().count() as i32;
                let ed_cs = crate::ported::zle::zle_main::ZLECS.load(Ordering::SeqCst) as i32;
                if let Ok(mut g) = ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
                    *g = ed_line;
                }
                ZLECS.store(ed_cs.clamp(0, ed_ll), Ordering::SeqCst);
                ZLELL.store(ed_ll, Ordering::SeqCst);
            }
            metafy_line(); // c:356
        }
        return goto_compend(ret); // c:358 goto compend
    }

    // c:359-361 — clear lastprebr/lastpostbr.
    lastprebr_set(""); // c:359
    lastpostbr_set(""); // c:360

    let curpm = comppatmatch
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default();
    if !curpm.is_empty() && curpm != opm {
        // c:363
        haspattern.store(1, Ordering::Relaxed); // c:364
    }
    let nm = nmatches.load(Ordering::Relaxed); // c:366
    let dm = diffmatches.load(Ordering::Relaxed);
    tracing::debug!(
        target: "compsys_args",
        nm,
        dm,
        useline = useline.load(Ordering::Relaxed),
        uselist = uselist.load(Ordering::Relaxed),
        iforcemenu = iforcemenu.load(Ordering::Relaxed),
        "do_completion branch point"
    );
    if iforcemenu.load(Ordering::Relaxed) != 0 {
        // c:366
        if nm != 0 {
            {
                let _ = do_ambig_menu();
            };
        } // c:367
        ret = if nm == 0 { 1 } else { 0 }; // c:369
    } else if useline.load(Ordering::Relaxed) < 0 {
        // c:370
        unmetafy_line(); // c:372
                         // zshrs bridge, no C counterpart — same split-line-buffer copies as the
                         // c:352-357 arm above; see the comment there.
        {
            let comp_line: Vec<char> = ZLELINE
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .map(|g| g.chars().collect())
                .unwrap_or_default();
            let comp_ll = comp_line.len();
            let comp_cs = ZLECS.load(Ordering::SeqCst).clamp(0, comp_ll as i32) as usize;
            if let Ok(mut g) = crate::ported::zle::zle_main::ZLELINE.lock() {
                *g = comp_line;
            }
            crate::ported::zle::zle_main::ZLECS.store(comp_cs, Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLELL.store(comp_ll, Ordering::SeqCst);
        }
        ret = selfinsert(&[]); // c:373
        {
            let ed_line: String = crate::ported::zle::zle_main::ZLELINE
                .lock()
                .map(|g| g.iter().collect())
                .unwrap_or_default();
            let ed_ll = ed_line.chars().count() as i32;
            let ed_cs = crate::ported::zle::zle_main::ZLECS.load(Ordering::SeqCst) as i32;
            if let Ok(mut g) = ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = ed_line;
            }
            ZLECS.store(ed_cs.clamp(0, ed_ll), Ordering::SeqCst);
            ZLELL.store(ed_ll, Ordering::SeqCst);
        }
        metafy_line(); // c:374
    } else if useline.load(Ordering::Relaxed) == 0 && uselist.load(Ordering::Relaxed) != 0 {
        // c:374
        ZLEMETACS.store(0, Ordering::Relaxed); // c:375
        foredel(ZLEMETALL.load(Ordering::Relaxed), CUT_RAW); // c:376 — `foredel(zlemetall, CUT_RAW)`
        let _ = inststr(
            &ORIGLINE
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default(),
        ); // c:377 — `inststr(origline)`
        ZLEMETACS.store(ORIGCS.load(Ordering::Relaxed), Ordering::Relaxed); // c:378
        SHOWINGLIST.store(-2, Ordering::Relaxed);
        // c:379
    } else if useline.load(Ordering::Relaxed) == 2 && nm > 1 {
        // c:380
        // c:381 — `do_allmatches(1)`. Faithful minfo-driven insertion:
        // iterates `amatches`, chaining do_single/accept_last against
        // the shared ZLEMETALINE buffer (see compresult::do_allmatches).
        crate::ported::zle::compresult::do_allmatches(1);
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(Menuinfo::default())).lock() {
            g.cur = None;
        } // c:383
        if forcelist.load(Ordering::Relaxed) != 0 {
            // c:385
            SHOWINGLIST.store(-2, Ordering::Relaxed);
        } else {
            invalidatelist(); // c:388
        }
    } else if useline.load(Ordering::Relaxed) != 0 {
        // c:389
        if nm > 1 && dm != 0 {
            // c:391
            // c:393 — `ret = do_ambiguous()`. Inlined: flatten `amatches`
            // into &[String] and dispatch.
            ret = {
                let groups = amatches
                    .get_or_init(|| Mutex::new(Vec::new()))
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                let all: Vec<String> = groups
                    .into_iter()
                    .flat_map(|g| g.matches.into_iter().filter_map(|m| m.str))
                    .collect();
                crate::ported::zle::compresult::do_ambiguous(&all)
            };
            if SHOWINGLIST.load(Ordering::Relaxed) == 0
                && uselist.load(Ordering::Relaxed) != 0
                && LISTSHOWN.load(Ordering::Relaxed) != 0
                && (USEMENU.load(Ordering::Relaxed) == 2 || oldlist.load(Ordering::Relaxed) != 0)
            {
                SHOWINGLIST.store(osl, Ordering::Relaxed);
                // c:395
            }
        } else if nm == 1 || (nm > 1 && dm == 0) {
            // c:396
            do_single_first_match(); // c:399-411
            if forcelist.load(Ordering::Relaxed) != 0 {
                // c:412
                if uselist.load(Ordering::Relaxed) != 0 {
                    SHOWINGLIST.store(-2, Ordering::Relaxed);
                } else {
                    CLEARLIST.store(1, Ordering::Relaxed);
                }
            } else {
                invalidatelist(); // c:418
            }
        } else if nmessages.load(Ordering::Relaxed) != 0 && forcelist.load(Ordering::Relaxed) != 0 {
            // c:419
            if uselist.load(Ordering::Relaxed) != 0 {
                SHOWINGLIST.store(-2, Ordering::Relaxed);
            } else {
                CLEARLIST.store(1, Ordering::Relaxed);
            }
        }
    } else {
        // c:425
        invalidatelist(); // c:426
        LASTAMBIG.store(
            // c:427
            opt_isset("BASHAUTOLIST"),
            Ordering::Relaxed,
        );
        if forcelist.load(Ordering::Relaxed) != 0 {
            CLEARLIST.store(1, Ordering::Relaxed);
        } // c:428
        ZLEMETACS.store(0, Ordering::Relaxed); // c:429
        foredel(ZLEMETALL.load(Ordering::Relaxed), CUT_RAW); // c:430 — `foredel(zlemetall, CUT_RAW)`
        let _ = inststr(
            &ORIGLINE
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default(),
        ); // c:431 — `inststr(origline)`
        ZLEMETACS.store(ORIGCS.load(Ordering::Relaxed), Ordering::Relaxed); // c:432
    }

    // c:436 — explanation strings.
    if SHOWINGLIST.load(Ordering::Relaxed) == 0
        && VALIDLIST.load(Ordering::Relaxed) != 0
        && USEMENU.load(Ordering::Relaxed) != 2
        && uselist.load(Ordering::Relaxed) != 0
        && (nm != 1 || dm != 0)
        && useline.load(Ordering::Relaxed) >= 0
        && useline.load(Ordering::Relaxed) != 2
        && (oldlist.load(Ordering::Relaxed) == 0 || LISTSHOWN.load(Ordering::Relaxed) == 0)
    {
        onlyexpl.store(3, Ordering::Relaxed); // c:441
        SHOWINGLIST.store(-2, Ordering::Relaxed);
        // c:442
    }

    goto_compend(ret)
}

// =====================================================================
// before_complete / after_complete — `Src/Zle/compcore.c:461 / 503`.
// =====================================================================

/// Direct port of `int before_complete(Hookdef dummy, int *lst)`
/// from `Src/Zle/compcore.c:461`. Pre-completion hook: snapshots
/// `menucmp` into `oldmenucmp`, decides whether the current state
/// shortcircuits via menu-completion, clamps the cursor when re-
/// entering an in-word completion, and toggles automenu mode.
/// Returns 1 to suppress the next-stage match build, 0 to continue.
pub fn before_complete(lst: &mut i32) -> i32 {
    // c:461

    // c:463 — `oldmenucmp = menucmp;`
    OLDMENUCMP.store(MENUCMP.load(Ordering::Relaxed), Ordering::Relaxed);

    // c:465-466 — `if (showagain && validlist) showinglist = -2;`
    if SHOWAGAIN.load(Ordering::Relaxed) != 0 && VALIDLIST.load(Ordering::Relaxed) != 0 {
        SHOWINGLIST.store(-2, Ordering::Relaxed);
    }
    // c:467 — `showagain = 0;`
    SHOWAGAIN.store(0, Ordering::Relaxed);

    let has_cur = MINFO
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.cur.is_some())
        .unwrap_or(false);
    let menucmp_v = MENUCMP.load(Ordering::Relaxed);

    // c:471-474 — menu-completion shortcircuit (non-listing path).
    // C: `do_menucmp(*lst); return 1;`. An active menu (minfo.cur set,
    // menucmp on) means this Tab should step the menu cursor and insert the
    // next match, NOT restart completion.
    if has_cur && menucmp_v != 0 && *lst != COMP_LIST_EXPAND {
        if *lst == COMP_LIST_COMPLETE {
            // do_menucmp c:1258-1260 — just (re)show the list.
            SHOWINGLIST.store(-2, Ordering::Relaxed);
        } else {
            // do_menucmp c:1263-1268 — `if (zlemetaline == NULL) metafy_line()`.
            // before_complete runs before docomplete metafies the buffer, so
            // do_single would otherwise edit a stale/unmetafied line and drop
            // the command prefix (`cat alpha.txt` → `alpine.md`). Metafy the
            // completion buffer (still holding the previous match's line) so
            // do_single replaces only the word region tracked by minfo.
            if ZLEMETALL.load(Ordering::Relaxed) == 0 {
                metafy_line();
            }
            // do_menucmp c:1270-1276 —
            //   while (zmult) { minfo.cur = valid_match(minfo.cur, 1);
            //                   zmult -= sign; }
            //   do_single(*minfo.cur);
            // valid_match advances the menu cursor one valid match in the
            // ZMULT direction, updating minfo.group_idx/cur_idx; do_single
            // then replaces the previously-inserted match (tracked via
            // minfo.pos/len/end) with the new one.
            let mult = crate::ported::zle::zle_main::ZMOD
                .lock()
                .map(|g| g.mult)
                .unwrap_or(1);
            ZMULT.store(mult, Ordering::Relaxed);
            let steps = mult.abs().max(1);
            let mut mc = None;
            for _ in 0..steps {
                let cur_idx = MINFO
                    .get()
                    .and_then(|m| m.lock().ok())
                    .map(|m| m.cur_idx)
                    .unwrap_or(0);
                mc = crate::ported::zle::compresult::valid_match(cur_idx, 1);
            }
            // c:1272 — minfo.cur = valid_match(...); set before do_single so
            // the insertion state stays consistent with the advanced cursor.
            if let Ok(mut mst) = MINFO.get_or_init(|| Mutex::new(Menuinfo::default())).lock() {
                mst.cur = mc.clone().map(Box::new);
            }
            if let Some(ref m) = mc {
                crate::ported::zle::compresult::do_single(m); // c:1276
            }
        }
        return 1; // c:473
    }
    // c:475-479 — menu-completion shortcircuit (listing path).
    if has_cur
        && menucmp_v != 0
        && VALIDLIST.load(Ordering::Relaxed) != 0
        && *lst == COMP_LIST_COMPLETE
    {
        SHOWINGLIST.store(-2, Ordering::Relaxed);
        onlyexpl.store(0, Ordering::Relaxed); // c:477
                                              // c:477 — `listdat.valid = 0;`
        if let Some(ld) = listdat.get() {
            if let Ok(mut g) = ld.lock() {
                g.valid = 0;
            }
        }
        return 1; // c:478
    }

    // c:488-489 — `if ((fromcomp & FC_INWORD) && (zlecs = lastend) > zlell)
    //              zlecs = zlell;` — re-entering an in-word completion
    //              restores the cursor to `lastend` (clamped to `zlell`).
    //
    // KNOWN DIVERGENCE, deliberately not made literal — see below.
    //
    // C names `zlecs`/`zlell` here, i.e. the INTERACTIVE editor buffer, and
    // its comment at c:483-487 says why ("Currently this hook runs before
    // metafication. This is the only hook of the three defined here of which
    // that is true."). It can do that because C has ONE line buffer: the
    // `lastend` that `do_single` records (`lastend = zlemetacs`,
    // compresult.c:479/672) indexes the same characters `zlecs` does.
    //
    // This port splits that buffer — `zle_main::ZLELINE`/`ZLECS` is the
    // editor's, `compcore::ZLELINE` + `ZLEMETALINE`/`ZLEMETACS` are
    // completion's — and `lastend` is written in the COMPLETION buffer's
    // coordinates. Storing it into `zle_main::ZLECS` was measured to break
    // the second TAB of a menu-select round outright: with the store
    // enabled, `docomplete` bailed before `do_completion` ever ran (no
    // `callcompfunc` reached on TAB 2), and `cd /<TAB><TAB>s` lost the whole
    // interactive menu. `lastend` is simply not a valid editor-cursor value
    // here.
    //
    // So the store stays on `ZLEMETACS`, which is where the port has always
    // put it. That is not what C writes, but it is inert (docomplete's
    // `metafy_line()` at zle_tricky.rs:815 recomputes it from the editor
    // buffer straight afterwards) rather than actively wrong.
    //
    // Honouring c:488 literally requires `lastend` to be maintained in
    // editor-buffer coordinates — i.e. `compresult`'s two `lastend =
    // zlemetacs` assignments need an editor-space counterpart, or the port
    // needs to stop splitting the line buffer. Neither is a compcore.rs
    // change; until one lands, this line cannot be ported faithfully.
    if (fromcomp.load(Ordering::Relaxed) & crate::ported::zle::comp_h::FC_INWORD) != 0 {
        let le = lastend.load(Ordering::Relaxed); // c:488 `zlecs = lastend`
        let ll = ZLEMETALL.load(Ordering::Relaxed);
        let new_cs = if le > ll { ll } else { le }; // c:489 `zlecs = zlell`
        ZLEMETACS.store(new_cs, Ordering::Relaxed);
    }

    // c:494-496 — automenu trigger.
    if startauto.load(Ordering::Relaxed) != 0 && LASTAMBIG.load(Ordering::Relaxed) != 0 {
        let bashauto = isset(BASHAUTOLIST);
        let last = LASTAMBIG.load(Ordering::Relaxed);
        if !bashauto || last == 2 {
            USEMENU.store(2, Ordering::Relaxed);
        }
    }

    0 // c:498
}

/// Direct port of `int after_complete(Hookdef dummy, int *dat)`
/// from `Src/Zle/compcore.c:503`. Post-completion hook: when a
/// completion has just transitioned into menu-completion (menucmp
/// went 0→1 across this round), runs MENUSTARTHOOK so registered
/// hook ported can veto or modify the about-to-display menu.
///
/// Hook handlers are registered via `addhookfunc("menu_start", fn)`
/// (see `crate::ported::module::addhookfunc`), which writes to the
/// global HOOKTAB. C's `comphooks[]` table declares `menu_start` as
/// HOOKF_ALL, so every handler fires and the first non-zero return
/// short-circuits the chain (see runhookdef at module.c:990).
///
/// Return value semantics (c:518-532):
///   - `ret == 0` → no action (no handler vetoed).
///   - `ret >= 1` → zero `dat[1]`, clear menucmp/menuacc, null minfo.cur.
///   - `ret >= 2` → also rewind buffer to origline.
///   - `ret == 2` → also schedule list clear (CLEARLIST=1, invalidatelist).
pub fn after_complete(dat: &mut [i32]) -> i32 {
    // c:503
    let menucmp_v = MENUCMP.load(Ordering::Relaxed);
    let oldmenucmp_v = OLDMENUCMP.load(Ordering::Relaxed);

    // c:505 — `if (menucmp && !oldmenucmp) { ... }`.
    //
    // `iforcemenu == -1` marks a completion driven from INSIDE
    // `domenuselect`'s interactive filter loop (complist.rs:2776-2779 —
    // C sets it at c:2773). C never reaches the restore below on that
    // path because its `oldmenucmp` is still 1 from the outer menu:
    // c:517's `runhookdef(MENUSTARTHOOK, …)` runs the whole interactive
    // loop BEFORE c:520 clears `menucmp`, so every nested
    // `before_complete` (c:462) snapshots a 1.
    //
    // In this port `menucmp` reads 0 at the nested `before_complete`
    // (measured: menucmp=1 oldmenucmp=0 ifm=-1 at the gate) and
    // `do_ambig_menu` only re-raises it later, so the gate opened and
    // the `ret >= 2` arm below ran `foredel` + `inststr(origline)` —
    // throwing away the match the same completion had just inserted.
    // That is why `menu select interactive` reported the typed
    // characters instead of the completion: the status line read the
    // line AFTER it had been reverted (`interactive: /s[]` where zsh
    // shows `interactive: /sbin[]`).
    //
    // The same `iforcemenu != -1` test is what C uses everywhere else to
    // mean "not driven by the interactive menu-select widget" (c:763,
    // c:832, c:1381, c:1437), so state it directly here.
    if menucmp_v == 0 || oldmenucmp_v != 0 || iforcemenu.load(Ordering::Relaxed) == -1 {
        return 0; // c:535
    }

    // c:506-517 — build chdata. cdat.matches=amatches, cdat.num=
    //              nmatches, cdat.nmesg=nmessages, cdat.cur=NULL. The
    //              Rust hook dispatch path doesn't yet thread chdata
    //              into shell-fn args (handlers in the standard zsh
    //              distribution all read directly from compsys globals
    //              via $compstate). The fields above are still tracked
    //              via amatches/nmatches/nmessages globals and visible
    //              to handlers through the normal completion-state
    //              parameter reads.

    // c:518 — `runhookdef(MENUSTARTHOOK, &cdat)`. Canonical dispatch
    // via `gethookdef("menu_start") + runhookdef(h, &cdat)`. Returns
    // 0 when no Hookfn is registered (matches c:993-995: empty funcs
    // and h->def NULL → return 0).
    let mut ret: i32 = 0;
    let h_menu_start = gethookdef("menu_start");
    if !h_menu_start.is_null() {
        ret = runhookdef(h_menu_start, std::ptr::null_mut());
    }

    if ret == 0 {
        return 0; // c:535
    }

    // c:519 — `dat[1] = 0`. The C caller passes a 2-int array; index 1
    // carries the menu-acceptance flag for the outer compfunc loop.
    if dat.len() > 1 {
        dat[1] = 0;
    }
    // c:520 — `menucmp = menuacc = 0`.
    MENUCMP.store(0, Ordering::Relaxed);
    menuacc.store(0, Ordering::Relaxed);
    // c:521 — `minfo.cur = NULL`.
    if let Some(m) = MINFO.get() {
        if let Ok(mut mi) = m.lock() {
            mi.cur = None;
        }
    }

    if ret >= 2 {
        // c:522
        // c:523 — `fixsuffix()`.
        fixsuffix();
        // c:524 — `zlemetacs = 0`.
        ZLEMETACS.store(0, Ordering::Relaxed);
        // c:525 — `foredel(zlemetall, CUT_RAW)` removes the entire line.
        let metall = ZLEMETALL.load(Ordering::Relaxed);
        foredel(metall, CUT_RAW);
        // c:526 — `inststr(origline)` reinserts the pre-completion buffer.
        let origline_v: String = ORIGLINE
            .get()
            .and_then(|m| m.lock().ok().map(|g| g.clone()))
            .unwrap_or_default();
        let _ = inststr(&origline_v);
        // c:527 — `zlemetacs = origcs`.
        let origcs_v = ORIGCS.load(Ordering::Relaxed);
        ZLEMETACS.store(origcs_v, Ordering::Relaxed);

        if ret == 2 {
            // c:528
            // c:529 — `clearlist = 1`.
            CLEARLIST.store(1, Ordering::Relaxed);
            // c:530 — `invalidatelist()`.
            invalidatelist();
        }
    }

    0 // c:535
}

// =====================================================================
// callcompfunc — `Src/Zle/compcore.c:544`.
// =====================================================================

/// Port of `static void callcompfunc(char *s, char *fn)` from
/// compcore.c:544. Selects the `$compstate[context]` value, then
/// dispatches into the user shell function `fn`. Paramtab setup
/// (`comprpms`/`compkpms`) + result-readback is stubbed locally
/// per PORT.md Rule 9 until `params.c` substrate lands.
pub fn callcompfunc(s: &str, fn_name: &str) {
    tracing::debug!(target: "compsys_args", %s, %fn_name, "callcompfunc ENTER");
    // c:544

    if fn_name.is_empty() {
        return;
    } // c:552 getshfunc(NULL)
      // Re-assert the `$module_path` compiled default at completion entry IF it
      // has gone empty. `MODULE_PATH` is PM_DONTIMPORT (no env var seeds it), and
      // its array half is not re-derived when the completion widget scope
      // re-establishes the tied colon-arrays the way PATH/FPATH are from the
      // environment — so `$module_path` reads empty inside completers, breaking
      // every `_files -W module_path` (e.g. `zmodload <tab>`). Only restore when
      // empty so a user's `module_path+=(…)` customization is preserved.
      // module_path_init is idempotent (OnceLock-cached MODULE_DIR).
    if crate::ported::params::getaparam("module_path")
        .map(|a| a.is_empty())
        .unwrap_or(true)
    {
        crate::ported::init::module_path_init();
    }
    let _lv = crate::ported::builtin::LASTVAL.load(Ordering::Relaxed); // c:548 int lv = lastval
    let _icf = INCOMPFUNC.load(Ordering::Relaxed); // c:555
    let _osc = crate::ported::builtin::SFCONTEXT.load(Ordering::Relaxed); // c:555

    let _useglob = USEGLOB.load(Ordering::Relaxed); // c:579

    // c:561-563 — `kset = CP_ALLKEYS & ~(CP_PARAMETER | CP_REDIRECT |
    // CP_QUOTE | CP_QUOTING | CP_EXACTSTR | CP_OLDLIST | CP_OLDINS | …)`,
    // handed to `comp_setunset(…, kset, ~kset & CP_ALLKEYS)` at c:818.
    // Every cleared bit raises PM_UNSET on that key's `compkpms` slot
    // (complete.c:1557-1558), and a PM_UNSET param is skipped by every
    // hash scan — so `${(@kv)compstate}`, and therefore `_lastcomp`
    // (`_main_complete` sh:407), carries no entry for it at all.
    //
    // zshrs's assoc backing is a flat name→map with no per-key flag bits,
    // so the equivalent of raising PM_UNSET is removing the entry. The
    // publishes below used to write "" for these keys instead, which is a
    // different observable state: present, with an empty value.
    let kunset = |key: &str| {
        // c:complete.c:1558 — `(*p)->node.flags |= PM_UNSET`.
        if let Ok(mut tab) = paramtab_hashed_storage().lock() {
            if let Some(hash) = tab.get_mut(crate::ported::zle::complete::COMPSTATENAME) {
                hash.remove(key);
            }
        }
        crate::ported::params::unsetparam(&format!("compstate[{}]", key));
    };
    // c:562 — `CP_EXACTSTR` is one of the bits cleared out of `kset`, so
    // `$compstate[exact_string]` starts the round UNSET; only a later
    // exact match publishes it (c:3046-3055, mirrored at the
    // `set_compstate_str("exact_string", …)` site below). do_completion's
    // c:312 `compexactstr = ""` resets the GLOBAL, not the param's set
    // bit — the port's matching publish left the key present-and-empty,
    // so `_lastcomp` carried an `exact_string` entry zsh does not have
    // whenever the round found no exact match.
    kunset("exact_string"); // c:562

    // c:667-693 — `compquote` / `compquoting` from the quote state
    // `get_comp_string` recorded. These ARE `$compstate[quote]` and
    // `$compstate[quoting]` (complete.c:1276-1277). Neither was ported, so
    // both read empty for every completion — `_main_complete`'s
    // `[[ -n $compstate[quote] ]]` branches, `_path_files`'s quoting
    // decisions and `addmatches`'s own c:2139 quote block all behaved as
    // if nothing were ever quoted.
    {
        use crate::ported::zle::complete::{COMPQUOTE, COMPQUOTING};
        let instring = INSTRING.load(Ordering::Relaxed);
        let (cq, cqg): (&str, &str) = if instring > QT_BACKSLASH {
            // c:669
            match instring {
                QT_SINGLE => ("'", "single"),    // c:671-674
                QT_DOUBLE => ("\"", "double"),   // c:676-679
                QT_DOLLARS => ("$'", "dollars"), // c:681-684
                _ => ("", ""),
            }
        } else if INBACKT.load(Ordering::Relaxed) != 0 {
            ("`", "backtick") // c:687-689
        } else {
            ("", "") // c:691-693
        };
        for (global, v) in [(&COMPQUOTE, cq), (&COMPQUOTING, cqg)] {
            if let Ok(mut g) = global.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = v.to_string();
            }
        }
        // The `$compstate` entries are gsu VIEWS onto those globals in C;
        // this port has to publish them explicitly.
        //
        // c:561-563 — `CP_QUOTE | CP_QUOTING` start cleared in `kset`;
        // only the two quoted arms (c:686, c:690) raise them. The
        // unquoted arm at c:691-693 leaves the globals empty AND the
        // params unset, so the keys must disappear rather than appear
        // with an empty value.
        if cq.is_empty() && cqg.is_empty() {
            kunset("quote"); // c:562
            kunset("quoting"); // c:562
        } else {
            set_compstate_str("quote", cq); // complete.c:1276, c:686/690
            set_compstate_str("quoting", cqg); // complete.c:1277, c:686/690
        }
    }

    // Publish the completion word split at the cursor into the
    // `$PREFIX` / `$SUFFIX` params (+ empty ignored-prefix/suffix). In C
    // these are gsu-bound to `compprefix`/`compsuffix`; the Rust ports
    // have no gsu binding, so without this every completer reads
    // `$PREFIX=''` — `_main_complete`'s `compset -P 1 '='` then matches
    // the empty prefix and wrongly forces `$compstate[context]=equal`,
    // and `_path_files` has no prefix to glob. The word is split the way
    // c:699-718 splits it — whole-word under `unset(COMPLETEINWORD)`, else
    // at `OFFS` (zlemetacs - wb), the cursor offset within the word.
    {
        // c:699-718 — the compprefix/compsuffix split. C branches on
        // `unset(COMPLETEINWORD)` FIRST: with the option OFF (the default)
        // the WHOLE word is the prefix and the suffix is EMPTY — completion
        // runs from the end of the word whatever column the cursor sits in;
        // only with completeinword SET is the word split at `offs`:
        //     if (unset(COMPLETEINWORD)) {
        //         tmp = (linwhat == IN_MATH ? dupstring(s) : multiquote(s, 0));
        //         untokenize(tmp);
        //         compprefix = ztrdup(tmp);
        //         compsuffix = ztrdup("");
        //     } else { … split at s + offs … }
        // The port applied the split unconditionally, i.e. completeinword
        // semantics for everyone, so a mid-word TAB completed against the
        // text left of the cursor instead of the whole word. The sibling
        // site (`set_comp_sep`, c:1892-1906) already carries this branch —
        // compcore.rs:2861-2868.
        //
        // Two steps of the C hunk are deliberately NOT replayed, because
        // this port's `s` is not in the token state C's `s` is:
        //
        //   * `multiquote(s, 0)` (c:700/711/715). It is NOT a no-op —
        //     `do_completion` seeds `$compqstack` with one QT_BACKSLASH
        //     element on every single completion (c:301-308,
        //     compcore.rs:100-112), so it backslash-escapes every
        //     `ispecial()` character it is handed. C can afford that
        //     because its word is still TOKENIZED here: an active glob `*`
        //     is the token byte `Star`, which `ispecial()`
        //     (utils.rs:9931-9946) does not match, while a `\*` the user
        //     typed has had its `Bnull` marker chucked out of the word
        //     already (zle_tricky.c:1885-1923) and is a plain `*` that
        //     correctly gets its backslash back. This port's
        //     `get_comp_string` returns the word ALREADY untokenized
        //     (zle_tricky.rs:2578-2587) and does not port that inull
        //     cleanup, so `multiquote` here sees plain ASCII and escapes
        //     the LIVE metacharacters: it published `$PREFIX` as `\*` for
        //     `ls *`, `\*\(` for `ls *(`, `\$\{\(` for `echo ${(` and
        //     `\\\*` for `ls \*`. Measured against zsh 5.9 through
        //     scripts/comptab_parity.py with a completer that prints
        //     `${(qq)PREFIX}`, zsh publishes the bare word for all of them
        //     (`'*('`, `'${('`, `'~/'`, `'../'`, `'\*'`) — which is exactly
        //     the un-requoted `s` this port already has.
        //   * `untokenize` (c:701/712/716) — already applied upstream at
        //     `get_comp_string`'s return. Repeating it is a no-op on ASCII
        //     but would silently drop a literal U+00A1 (`Nularg`) or
        //     U+0084-U+00A1 character out of a non-ASCII word.
        //
        // Reinstating either needs the tokenized word AND the unported
        // c:1788-1926 line cleanup, not a local change here. The IN_MATH
        // fork at c:700/711/715 (`dupstring` instead of `multiquote`)
        // therefore collapses into the same expression too.
        let (pre, suf) = if !isset(crate::ported::zsh_h::COMPLETEINWORD) {
            // c:699
            (s.to_string(), String::new()) // c:700-703
        } else {
            // c:704
            let scs: Vec<char> = s.chars().collect();
            let off = (OFFS.load(Ordering::Relaxed).max(0) as usize).min(scs.len()); // c:707
            (
                scs[..off].iter().collect::<String>(), // c:709-713
                scs[off..].iter().collect::<String>(), // c:714-717
            )
        };
        let _ = crate::ported::params::setsparam("PREFIX", &pre);
        let _ = crate::ported::params::setsparam("SUFFIX", &suf);
        // c:724-741 — `$IPREFIX` / `$ISUFFIX`.
        //
        //     zsfree(compiprefix); zsfree(compisuffix);
        //     if (parwb < 0) { compiprefix = ztrdup(""); compisuffix = ztrdup(""); }
        //     else {
        //         compiprefix = zalloc((l = wb - parwb) + 1);
        //         memcpy(compiprefix, zlemetaline + parwb, l);
        //         compisuffix = zalloc((l = parwe - we) + 1);
        //         memcpy(compisuffix, zlemetaline + we, l);
        //         wb = parwb; we = parwe; offs = paroffs;
        //     }
        //
        // `makecomplist` (c:952-957) parks the word's ORIGINAL boundaries in
        // `parwb`/`parwe`/`paroffs` before `check_param` narrows `wb`/`we`/
        // `offs` down to the parameter NAME, so the two spans differ by
        // exactly the sigil (`$`, `${`, `${(flags)`) and by the `}` plus any
        // modifiers on the other side. That difference IS `$IPREFIX` /
        // `$ISUFFIX`.
        //
        // Both were hardcoded to the empty string here, which dropped the
        // whole block: `echo $PATH<TAB>` published `IPREFIX=''` where zsh
        // publishes `IPREFIX='$'`. Anything that rebuilds the word from
        // `$IPREFIX$PREFIX$SUFFIX$ISUFFIX` then lost the sigil — `_expand`
        // (Completion/Base/Completer/_expand:22) saw the word as `PATH`, its
        // substitution step produced `PATH` again, and sh:128's
        // "expansion equals the word" test returned 1, so the completer
        // emitted no expansions at all.
        let (ipre_v, isuf_v) = if PARWB.load(Ordering::Relaxed) < 0 {
            (String::new(), String::new()) // c:727-728
        } else {
            let line = ZLEMETALINE
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            let parwb = PARWB.load(Ordering::Relaxed).max(0) as usize;
            let parwe = PARWE.load(Ordering::Relaxed).max(0) as usize;
            let wb = WB.load(Ordering::Relaxed).max(0) as usize;
            let we = WE.load(Ordering::Relaxed).max(0) as usize;
            // `get` returns None on an out-of-range or non-char-boundary
            // span; C's memcpy cannot fail because it copies bytes out of a
            // buffer it already sized, so the fallback here is the empty
            // string — the same value the `parwb < 0` arm uses.
            let ip = line.get(parwb..wb).unwrap_or("").to_string(); // c:732-734
            let is = line.get(we..parwe).unwrap_or("").to_string(); // c:735-737
            WB.store(PARWB.load(Ordering::Relaxed), Ordering::Relaxed); // c:739
            WE.store(PARWE.load(Ordering::Relaxed), Ordering::Relaxed); // c:740
            OFFS.store(PAROFFS.load(Ordering::Relaxed), Ordering::Relaxed); // c:741
            (ip, is)
        };
        let _ = crate::ported::params::setsparam("IPREFIX", &ipre_v);
        let _ = crate::ported::params::setsparam("ISUFFIX", &isuf_v);
        // c:742-745 — `compqiprefix = ztrdup(qipre ? qipre : "");
        //              compqisuffix = ztrdup(qisuf ? qisuf : "");`
        // `compqiprefix`/`compqisuffix` ARE `$QIPREFIX`/`$QISUFFIX`
        // (complete.c:1266-1267), and `qipre`/`qisuf` are what
        // `get_comp_string` (zle_tricky.c:1753-1766) filled in with the
        // opening/closing quote of the word being completed. The port
        // hardcoded both to "" here, so `$QIPREFIX` was permanently empty:
        // completing inside `"…"` / `'…'` / `$'…'` dropped the opening
        // quote off the command line and every `$QIPREFIX`-testing
        // completer took its unquoted branch.
        crate::vm_helper::set_readonly_special(
            "QIPREFIX",
            &crate::ported::zle::zle_tricky::qipre_get(),
        ); // c:743
        crate::vm_helper::set_readonly_special(
            "QISUFFIX",
            &crate::ported::zle::zle_tricky::qisuf_get(),
        ); // c:745
           // c:complete.c:1235-1295 — in C these params ARE `compprefix`/
           // `compsuffix`/`compiprefix`/`compisuffix` (gsu-bound, one
           // storage), so the publish above resets the globals too. The Rust
           // compparams have no gsu binding, so the globals kept the PREVIOUS
           // call's values — and `expand-or-complete` calls this twice per
           // TAB. Mirror the reset onto the globals. (Same block as
           // addmatches below and bin_compfiles -p/-P in computil.rs.)
        for (param, global) in [
            ("PREFIX", &COMPPREFIX),
            ("SUFFIX", &COMPSUFFIX),
            ("IPREFIX", &COMPIPREFIX),
            ("ISUFFIX", &crate::ported::zle::complete::COMPISUFFIX),
        ] {
            if let Some(v) = crate::ported::params::getsparam(param) {
                if let Ok(mut g) = global.get_or_init(|| Mutex::new(String::new())).lock() {
                    *g = v;
                }
            }
        }

        // c:720-723 — `zsfree(complastprefix); zsfree(complastsuffix);
        //              complastprefix = ztrdup(compprefix);
        //              complastsuffix = ztrdup(compsuffix);`.
        // Assigned from the compprefix/compsuffix GLOBALS the block above
        // just published — which is literally what C copies — rather than
        // from a second, locally recomputed split that can only agree with
        // them by accident. Taken here (before the completion fn runs and
        // before any `compset -P/-S` strips $PREFIX/$SUFFIX) so
        // `domenuselect`'s interactive status line renders the search
        // buffer as `interactive: <prefix>[]<suffix>` (setmstatus,
        // complist.c:2234-2235).
        for (src, dst) in [
            (&COMPPREFIX, &crate::ported::zle::complete::COMPLASTPREFIX),
            (&COMPSUFFIX, &crate::ported::zle::complete::COMPLASTSUFFIX),
        ] {
            let v = src
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            if let Ok(mut g) = dst.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = v; // c:722-723
            }
        }

        // c:743-751 — `compqiprefix = qipre; compqisuffix = qisuf;
        //   origlpre = strlen(compqiprefix)+strlen(compiprefix)+strlen(compprefix);
        //   origlsuf = strlen(compqisuffix)+strlen(compisuffix)+strlen(compsuffix);
        //   lenchanged = 0;`
        // `origlpre`/`origlsuf` record how long the prefix/suffix were when the
        // completion widget was entered; `addmatches` (c:2252-2254) compares the
        // CURRENT lengths against them and sets `lenchanged` when a completer has
        // moved the PREFIX/SUFFIX split (`compset -P/-S`, `_approximate`,
        // `_prefix`). `do_ambiguous` (compresult.c:794) reads that flag: with the
        // split moved it must NOT put the old word back when the unambiguous
        // string comes out short. All three were declared but never assigned, so
        // the flag was permanently 0 and `ls **/<TAB><TAB>s` restored `**/s` where
        // zsh leaves the word deleted (`interactive: []`).
        //
        // `compqiprefix`/`compqisuffix` ARE `$QIPREFIX`/`$QISUFFIX` in C
        // (gsu-bound, complete.c) — read back the values the publish above set
        // rather than a second, independently derived copy.
        {
            use crate::ported::zle::complete::{COMPISUFFIX, COMPQIPREFIX, COMPQISUFFIX};
            let qip = crate::ported::params::getsparam("QIPREFIX").unwrap_or_default(); // c:744
            let qis = crate::ported::params::getsparam("QISUFFIX").unwrap_or_default(); // c:746
            for (global, v) in [(&COMPQIPREFIX, &qip), (&COMPQISUFFIX, &qis)] {
                if let Ok(mut g) = global.get_or_init(|| Mutex::new(String::new())).lock() {
                    *g = v.clone();
                }
            }
            let glen = |g: &std::sync::OnceLock<Mutex<String>>| -> usize {
                g.get_or_init(|| Mutex::new(String::new()))
                    .lock()
                    .map(|s| s.len())
                    .unwrap_or(0)
            };
            origlpre.store(
                (qip.len() + glen(&COMPIPREFIX) + glen(&COMPPREFIX)) as i32,
                Ordering::Relaxed,
            ); // c:747-748
            origlsuf.store(
                (qis.len() + glen(&COMPISUFFIX) + glen(&COMPSUFFIX)) as i32,
                Ordering::Relaxed,
            ); // c:749-750
            lenchanged.store(0, Ordering::Relaxed); // c:751
        }
    }

    // c:591-617 — context selection.
    let context = compcontext_for(s); // c:591-617
    tracing::debug!(
        target: "compsys_args",
        %context,
        linwhat = linwhat.load(Ordering::Relaxed),
        ispar = ispar.load(Ordering::Relaxed),
        "callcompfunc context"
    );
    set_compstate_str("context", &context); // c:619

    // c:577 — `compparameter = compredirect = ""`, then c:586 (subscript),
    // c:607 (IN_ENV value / array_value) overwrite it with `varname`, the
    // parameter name `get_comp_string` split off the line. This publish was
    // missing entirely, so `$compstate[parameter]` kept whatever a previous
    // completion left: `_value` dispatched `-value-,,-default-` instead of
    // `-value-,PATH,-default-` and never reached the per-parameter completer.
    let varname = || {
        crate::ported::zle::zle_tricky::VARNAME
            .get_or_init(|| Mutex::new(None))
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default()
    };
    let compparameter = match context.as_str() {
        "subscript" if linwhat.load(Ordering::Relaxed) == IN_MATH_LW => varname(), // c:585-588
        "value" | "array_value" if linwhat.load(Ordering::Relaxed) == IN_ENV_LW => varname(), // c:607
        // c:628-629 — the default (no-command-word) `value` arm names
        // the parameter from the FIRST word on the line, not `varname`.
        "value" => crate::ported::zle::zle_tricky::CLWORDS
            .lock()
            .ok()
            .and_then(|g| g.first().cloned())
            .unwrap_or_default(),
        _ => String::new(), // c:577
    };
    // c:561-563 — `kset = CP_ALLKEYS & ~(CP_PARAMETER | …)`: the key
    // starts UNSET and only c:586 / c:594 / c:607 / c:626 raise
    // `kset |= CP_PARAMETER`, which is exactly the set of arms that give
    // `compparameter` a name. Publishing "" instead left the key present
    // in `${(@kv)compstate}` (and so in `_lastcomp`) where zsh has no
    // entry at all.
    if compparameter.is_empty() {
        kunset("parameter"); // c:562
    } else {
        set_compstate_str("parameter", &compparameter);
    }

    // c:598-602 — `compcontext = "redirect"; if (rdstr) compredirect =
    // rdstr;`. `compredirect` is `$compstate[redirect]` (complete.c:1265)
    // and was never written by this port, so `_redirect` had nothing to
    // dispatch on and `_expand`'s multios branch (sh:236) saw an empty
    // operator. Reset to "" (c:577) in every other context.
    let compredirect = if context == "redirect" {
        crate::ported::zle::zle_tricky::RDSTR
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default() // c:600-601
    } else {
        String::new() // c:577
    };
    // c:561-563 / c:601 — `CP_REDIRECT` likewise starts cleared and is
    // raised only by the `redirect` context arm.
    if compredirect.is_empty() {
        kunset("redirect"); // c:562
    } else {
        set_compstate_str("redirect", &compredirect); // c:601
    }
    // C binds `compredirect` to `$compstate[redirect]` through one gsu
    // storage; this port keeps the global and the param separate, so
    // mirror the write (same pattern as PREFIX/COMPPREFIX above).
    if let Ok(mut g) = crate::ported::zle::complete::COMPREDIRECT
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        *g = compredirect;
    }

    // c:648-653 — `compredirs = zlinklist2array(rdstrs, 1)`, published as
    // the `redirections` real-param (complete.c:1250). One entry per
    // COMPLETED redirection on the line, each `<op>:<target>`.
    setaparam(
        "redirections",
        crate::ported::zle::zle_tricky::RDSTRS
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default(),
    );

    // c:634-645 — `if (compwords) freearray(compwords); if (usea && …)
    // { compwords = copy of clwords } else compwords = empty`. C rebuilds
    // `$words` from the parsed line on EVERY call, which is what makes the
    // SECOND completion pass of `expand-or-complete` (zle_tricky.c:851)
    // see the full command line again — `get_comp_string` runs only once
    // per TAB. This port was missing the rebuild, so a first pass that
    // restricted `$words` (any `_arguments` spec with a `*::`/`*:::` rest
    // argument calls `comparguments -W` → restrict_range) left the second
    // pass with an empty word array: no command word, no completer
    // dispatch, and every match from the first pass discarded.
    //
    // `usea` (c:590) is 0 only in the math context; C's `aadd` sub-case
    // (parameter-subscript, c:626-630) needs `varname`, which this port
    // does not compute yet — it is treated as 0 here, exactly as before.
    {
        use crate::ported::zle::complete::{COMPCURRENT, COMPWORDS};
        let usea = linwhat.load(Ordering::Relaxed) != IN_MATH_LW;
        let ws: Vec<String> = if usea {
            crate::ported::zle::zle_tricky::CLWORDS
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default() // c:639-643
        } else {
            Vec::new() // c:645
        };
        let n = ws.len() as i32;
        // c:751 — `compcurrent = (usea ? (clwpos + 1 - aadd) : 0)`. Like
        // `compwords`, this is RECOMPUTED per call, never carried over: the
        // first pass' `comparguments -W` shifts it down to the restricted
        // range, and reusing that value left the second pass pointing at
        // the command word. `clwpos < 0` means the cursor sits past the
        // last word (fresh trailing word) — same guard as the publish site
        // in get_comp_string.
        let clwpos = crate::ported::zle::zle_tricky::CLWPOS.load(Ordering::Relaxed);
        let cur = if !usea {
            0 // c:751 — math context: no words, no current
        } else if clwpos < 0 {
            n + 1
        } else {
            (clwpos + 1).max(1)
        };
        if let Ok(mut g) = COMPWORDS.get_or_init(|| Mutex::new(Vec::new())).lock() {
            *g = ws.clone();
        }
        COMPCURRENT.store(cur, Ordering::Relaxed);
        // zshrs bridge: `$words`/`$CURRENT` are plain paramtab copies here
        // (C binds them to the globals via gsu), so the rebuild has to
        // reach the params too — see get_comp_string's publish site.
        setaparam("words", ws);
        let _ = crate::ported::params::setiparam("CURRENT", cur as i64);
    }

    // c:571-572 —
    // ```c
    //     if (!*complastprompt)
    //         kset &= ~CP_LASTPROMPT;
    // ```
    // C only READS `complastprompt` here (to drop CP_LASTPROMPT from the
    // "keys the completion function may set" mask); it never writes it.
    // The single writer is do_completion at c:325,
    // `complastprompt = ztrdup(isset(ALWAYSLASTPROMPT) ? "yes" : "")`,
    // ported at compcore.rs:200-208.
    //
    // This site used to WRITE `$compstate[last_prompt]` back from
    // `dolastprompt` (which do_completion has just set to 1 at c:326),
    // which stomped the "" that NO_ALWAYS_LAST_PROMPT had just stored.
    // addmatch's `if (!complastprompt || !*complastprompt) dolastprompt = 0`
    // (c:3014-3015) then never fired, so `dolastprompt` stayed 1,
    // `clearflag` came out 1 in asklist (c:1925) / compprintlist (c:2061),
    // and zrefresh's reset frame took the `if (clearflag)` branch at
    // c:1168-1172 (`\r` + `moveto(0, lpromptw)`) instead of the
    // `!clearflag` branch at c:1146-1167 (TCCLEAREOD + `zputs(lpromptbuf)`)
    // — so after a completion listing the prompt was never repainted.
    // `kset` is not materialised in this port (it is only used
    // descriptively, see c:561-563 above), so the C statement has no
    // representable effect beyond the read.
    let _complastprompt_isset = !get_compstate_str("last_prompt")
        .unwrap_or_default()
        .is_empty(); // c:571

    // c:753-765 — `$compstate[list]` is REBUILT here from `uselist`, it is
    // not the value do_completion left in `complist` at c:327-330:
    //
    //     switch (uselist) { case 0: ""; 1: "list"; 2: "autolist";
    //                        3: "ambiguous"; }
    //     if (isset(LISTPACKED))   complist = dyncat(complist, " packed");
    //     if (isset(LISTROWSFIRST)) complist = dyncat(complist, " rows");
    //
    // The port published only the do_completion half ("packed"/"rows"), so
    // the leading state word was never there: `_main_complete` and friends
    // test `$compstate[list]` for `list`/`autolist`/`ambiguous` and always
    // read them as absent. Write the rebuilt value to BOTH the param and the
    // `complist` global, which is one storage in C (gsu-bound) and is what
    // `addmatch` reads at c:2048-2050 for CMF_PACKED/CMF_ROWS.
    let mut cl_value = match uselist.load(Ordering::Relaxed) {
        0 => String::new(),           // c:755
        1 => "list".to_string(),      // c:756
        2 => "autolist".to_string(),  // c:757
        3 => "ambiguous".to_string(), // c:758
        _ => String::new(),
    };
    if opt_isset("LISTPACKED") != 0 {
        cl_value.push_str(" packed"); // c:761
    }
    if opt_isset("LISTROWSFIRST") != 0 {
        cl_value.push_str(" rows"); // c:763
    }
    if let Ok(mut g) = COMPLIST.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = cl_value.clone(); // c:765
    }
    set_compstate_str("list", &cl_value); // c:765

    // c:767-782 — `$compstate[insert]` per (useline, usemenu).
    let ul = useline.load(Ordering::Relaxed);
    let um = USEMENU.load(Ordering::Relaxed);
    let ins = if ul != 0 {
        // c:768-776
        match um {
            // c:769-772 — `compinsert = (isset(AUTOMENU) ?
            //                            "automenu-unambiguous" : "unambiguous");`
            //
            // AUTO_MENU is on in every emulation by default (options.c:90
            // lists it as `OPT_ALL`), so this arm — the one taken by an
            // ordinary first TAB — normally yields "automenu-unambiguous",
            // NOT the bare "unambiguous" this port hardcoded.
            //
            // The distinction is not cosmetic: it is the only way the shell
            // function layer learns that the next TAB is allowed to start
            // menu completion. Completion/Base/Core/_main_complete:302 gates
            // the whole MENUSELECT/MENUMODE block on
            // `[[ "$compstate[insert]" = *menu* ]]`, which
            // "automenu-unambiguous" satisfies and "unambiguous" does not
            // (ported at _main_complete.rs:929-931), and
            // Base/Completer/_match:53 tests for the value verbatim.
            0 => {
                if opt_isset("AUTOMENU") != 0 {
                    "automenu-unambiguous"
                } else {
                    "unambiguous"
                }
            }
            1 => "menu",
            2 => "automenu",
            _ => "",
        }
    } else {
        // c:777-780 — `compinsert = ""; kset &= ~CP_INSERT;`
        ""
    };
    // c:781 — `compinsert = (useline < 0 ? tricat("tab ", "", compinsert)
    //                                    : ztrdup(compinsert));`
    //
    // `useline < 0` is set at c:310 from `wouldinstab`, i.e. TAB was
    // pressed with nothing but blanks to its left AND a completion
    // widget is installed (zle_tricky.c:192-196). The "tab " prefix is
    // the ONLY signal `_main_complete` has for that case: sh:70-79 of
    // Completion/Base/Core/_main_complete tests `compstate[insert] =
    // tab*` and, with the default `insert-tab yes`, returns 0 before
    // any completer runs, so the widget falls through to inserting a
    // literal TAB. Dropping the prefix made zshrs run the full
    // completer chain on an empty command line, which surfaced every
    // diagnostic those completers emit (e.g. a user `_describe` over an
    // unset array printing "compdescribe: invalid argument") onto a
    // prompt where zsh prints nothing at all.
    let ins = if ul < 0 {
        format!("tab {}", ins) // c:781
    } else {
        ins.to_string()
    };
    set_compstate_str("insert", &ins); // c:781

    // c:790-794 — `$compstate[exact]` & `$compstate[exact_string]`.
    set_compstate_str(
        "exact",
        if useexact.load(Ordering::Relaxed) != 0 {
            "accept"
        } else {
            ""
        },
    );

    // c:791-794 — `$compstate[to_end]` per movetoend.
    set_compstate_str(
        "to_end",
        if movetoend.load(Ordering::Relaxed) == 1 {
            "single"
        } else {
            "match"
        },
    );

    // c:797-812 — `$compstate[old_list]` / `$compstate[old_insert]`:
    //
    //     if (hasoldlist && lastpermmnum) {
    //         compoldlist = listshown ? "shown" : "yes";
    //         if (minfo.cur) { sprintf(buf,"%d",(*minfo.cur)->gnum);
    //                          compoldins = buf; }
    //         else compoldins = "";
    //     } else compoldlist = compoldins = "";
    //
    // Both publishes were missing, so a completer could never tell that a
    // previous list is still around. `_menu` and `_history_complete_word`
    // read `$compstate[old_list]` and write back "keep" to reuse it; with
    // the entry value absent (and the c:923-925 readback below equally
    // absent) the keep round-trip could not work at all.
    let hasoldlist_v = hasoldlist.load(Ordering::Relaxed);
    let lastpermmnum_v = lastpermmnum.load(Ordering::Relaxed);
    if hasoldlist_v != 0 && lastpermmnum_v != 0 {
        // c:797
        set_compstate_str(
            "old_list",
            if LISTSHOWN.load(Ordering::Relaxed) != 0 {
                "shown" // c:799
            } else {
                "yes" // c:801
            },
        );
        // c:803-808 — `compoldins = minfo.cur ? (*minfo.cur)->gnum : ""`.
        let cur_gnum: Option<i32> = MINFO
            .get()
            .and_then(|m| m.lock().ok())
            .and_then(|m| m.cur.as_ref().map(|c| c.gnum));
        match cur_gnum {
            // c:806 — `kset |= CP_OLDINS` only in the minfo.cur arm.
            Some(g) => set_compstate_str("old_insert", &g.to_string()), // c:804-805
            None => kunset("old_insert"),                               // c:808
        }
    } else {
        // c:810 — `compoldlist = compoldins = ""` with CP_OLDLIST /
        // CP_OLDINS still cleared from c:562, i.e. both params stay
        // PM_UNSET and neither key appears in `${(@kv)compstate}`.
        kunset("old_list");
        kunset("old_insert");
    }

    // c:838 — `incompfunc = 1` before invoking the user fn.
    INCOMPFUNC.store(1, Ordering::Relaxed); // c:838

    // c:828-832 — `largs = newlinklist(); addlinknode(largs,
    //   dupstring(fn)); while (*cfargs) addlinknode(largs,
    //   dupstring(*p++));`. argv[0] = function name, then the
    // wrapper-widget args stored in `cfargs` by `completecall`.
    let largs: Vec<String> = {
        let mut v = vec![fn_name.to_string()];
        if let Ok(cf) = crate::ported::zle::zle_tricky::cfargs.lock() {
            v.extend(cf.iter().cloned());
        }
        v
    };

    // c:833-834 — `int oxt = isset(XTRACE); opts[XTRACE] = 0;`. Mute
    // xtrace during the body so PS4 noise doesn't appear from every
    // compsys helper line.
    let oxt = crate::ported::zsh_h::isset(crate::ported::zsh_h::XTRACE) as i32;
    crate::ported::options::opt_state_set(
        &crate::ported::zsh_h::opt_name(crate::ported::zsh_h::XTRACE),
        false,
    );
    let _ = oxt; // c:833 saved for restore at c:836

    // c:835 — `cfret = doshfunc(shfunc, largs, 1)`. The body runner
    // closure resolves the actual implementation:
    //   - If a Rust compsys port is registered for `fn_name` and
    //     `backend = "rust"`, run that.
    //   - Else autoload + run the upstream shfunc body via the
    //     standard dispatch path. dispatch_function_call already
    //     wraps the fusevm Chunk in its own doshfunc scope; we
    //     intentionally call only the body half here so the C-faithful
    //     prologue/epilogue runs exactly once around the body.
    let largs_for_body = largs.clone();
    let fn_name_owned = fn_name.to_string();

    // c:843-925 reads `complist`/`compinsert`/`compexact`/`comptoend`/
    // `compoldlist`/`compoldins` AFTER `endparamscope()` (c:838). It can do
    // that because in C those are plain globals (complete.c:36-44) and the
    // `$compstate` entries are only gsu VIEWS onto them (complete.c:1280-1300
    // `VAL(compinsert)` …) — tearing down the parameter cannot touch the
    // value.
    //
    // This port has no gsu binding: the values live in the `compstate`
    // parameter itself, which `callcompfunc` stamps PM_SPECIAL|PM_REMOVABLE
    // at `locallevel + 1` (the c:816-817 block above), so `doshfunc`'s
    // `endparamscope()` DELETES it. Measured with a tracing probe on the
    // `cd /<TAB>` round: every key read back `None` after the call — `list`,
    // `insert`, `exact` and `to_end` alike.
    //
    // So snapshot the hash at the END OF THE BODY — still inside the
    // function scope, and after the completion function's last write, which
    // is exactly the state C's globals hold when it reads them at c:843.
    let compstate_end: std::sync::Arc<Mutex<Option<indexmap::IndexMap<String, String>>>> =
        std::sync::Arc::new(Mutex::new(None));
    let compstate_end_body = std::sync::Arc::clone(&compstate_end);

    let body_runner = move || -> i32 {
        // c:6042 — `runshfunc(prog, wrappers, name)`. zshrs runs the
        // body via either the Rust compsys port (direct fn call) or
        // the fusevm Chunk dispatch (via exec accessors).
        let rc = if let Some(rc) =
            crate::compsys::router::dispatch_compsys(&fn_name_owned, &largs_for_body[1..])
        {
            // Plugin override (ABI v4) wins over the built-in Rust port.
            // C convention: largs[0] = fn name, [1..] = real argv.
            rc
        } else {
            crate::ported::exec::dispatch_function_call(&fn_name_owned, &largs_for_body[1..])
                .unwrap_or_else(|| crate::ported::builtin::LASTVAL.load(Ordering::Relaxed))
        };
        // Capture `$compstate` before the enclosing doshfunc scope ends.
        if let Ok(tab) = paramtab_hashed_storage().lock() {
            if let Some(h) = tab.get("compstate") {
                if let Ok(mut g) = compstate_end_body.lock() {
                    *g = Some(h.clone());
                }
            }
        }
        rc
    };

    // Look up the real shfunc; if missing we still want doshfunc's
    // scope around the Rust port (synth_shf carries just the name).
    let mut synth_shf = crate::ported::zsh_h::shfunc {
        node: crate::ported::zsh_h::hashnode {
            next: None,
            nam: fn_name.to_string(),
            flags: 0,
        },
        filename: None,
        lineno: 0,
        funcdef: None,
        redir: None,
        sticky: None,
        body: None,
        redir_text: None,
    };
    // c:816-817 — `startparamscope(); makecompparams();`.
    //
    // C creates `$words` / `$CURRENT` / `$PREFIX` / `$SUFFIX` /
    // `$IPREFIX` / `$ISUFFIX` / `$QIPREFIX` / `$QISUFFIX` /
    // `$compstate` here with `createparam(name, type|PM_SPECIAL|
    // PM_REMOVABLE|PM_LOCAL)` and then `pm->level = locallevel + 1`
    // (addcompparams c:1300-1307, makecompparams c:1348). Their VALUES
    // live in C globals reached through a gsu vtable, so creating them
    // empty here costs nothing.
    //
    // zshrs has no gsu binding: the values live in the params, and the
    // block above has already written every one of them with
    // `setsparam`/`setaparam`/`setiparam` — i.e. at level 0, flagged as
    // ordinary scalars. Re-running `createparam` would shadow those
    // values with empty ones, so only the scope half of c:817 is
    // applied: stamp the level and the special/removable bits onto the
    // params that are already in place.
    //
    // Without this, `${(t)PREFIX}` read `scalar` instead of
    // `scalar-local-special` and the names outlived the completion.
    // `_parameters` filters candidates with
    // `${(@k)parameters[(R)…~*local*]}`, so every one of these was
    // offered as a completion for `unset <TAB>`.
    //
    // `PM_READONLY` (on QIPREFIX/QISUFFIX in the c:1256-1259 table) is
    // stamped from each row's own type, the way c:1301 or's `cp->type`
    // in. The bit used to be dropped here because a sticky one failed
    // the next completion's publish; the publishes that run after this
    // stamp now go through `vm_helper::set_readonly_special`, the
    // gsu-setfn equivalent C uses (c:1308-1324), which is not subject to
    // the assignment gate.
    {
        use crate::ported::zsh_h::{PM_READONLY, PM_REMOVABLE, PM_SPECIAL};
        // c:1307 / c:1348 — `pm->level = locallevel + 1`.
        let level = crate::ported::params::locallevel.load(Ordering::Relaxed) + 1;
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            for (name, ty) in crate::ported::zle::complete::COMPRPARAMS
                .iter()
                .map(|cp| (cp.name, cp.r#type))
                .chain([("compstate", 0)])
            {
                if let Some(pm) = tab.get_mut(name) {
                    pm.level = level;
                    // c:1301 — the row's own type bits, which is where
                    // QIPREFIX/QISUFFIX's PM_READONLY comes from.
                    pm.node.flags |= ty & PM_READONLY as i32;
                    // c:1301 — `cp->type | PM_SPECIAL | PM_REMOVABLE |
                    // PM_LOCAL`. PM_REMOVABLE is load-bearing:
                    // `scanendscope` (params.c:5905) only takes the
                    // "restore the shadowed value" branch for
                    // `(flags & (PM_SPECIAL|PM_REMOVABLE)) == PM_SPECIAL`.
                    // These params have no shadow to restore — they must
                    // be deleted, which is the PM_REMOVABLE path.
                    pm.node.flags |= (PM_SPECIAL | PM_REMOVABLE) as i32;
                }
            }
        }
    }

    // c:820 — `makezleparams(1);`.
    //
    // This line was missing entirely, so EVERY completion function ran
    // with no ZLE parameters at all: `$BUFFER`, `$LBUFFER`, `$RBUFFER`,
    // `$CURSOR`, `$HISTNO`, `$WIDGET`, `$KEYS`, `$BUFFERLINES`,
    // `$PENDING` all read as the empty string. `_fc` computes
    // `(( num = num - HISTNO ))` to turn history event numbers into the
    // negative offsets it completes, so `fc -<TAB>` offered nothing;
    // more broadly any completer that inspects the line it is completing
    // (`$BUFFER`/`$CURSOR`) saw an empty line.
    crate::ported::zle::zle_params::makezleparams(1); // c:820
    {
        // c:839 — `endparamscope()` is what tears these down again in C:
        // `makezleparams` creates each one PM_SPECIAL|PM_REMOVABLE|
        // PM_LOCAL at `locallevel + 1` (zle_params.c:200-206), so leaving
        // the completion scope unsets them and they never reach the
        // interactive shell.
        //
        // The Rust `makezleparams` publishes through
        // `setsparam`/`setiparam`/`setaparam` (the values live in the
        // params, not behind a gsu vtable), which leaves them at the
        // enclosing level as ordinary params. Stamp the same level and
        // flags the comp params get above so `doshfunc`'s `endparamscope`
        // removes them on the way out. Without this the names leaked into
        // the interactive shell after the first TAB — `$BUFFER` and
        // friends stayed set at the prompt, and `_parameters` offered
        // them (its candidate filter is
        // `${(@k)parameters[(R)…~*local*]}`).
        //
        // As with QIPREFIX above, `PM_READONLY` (c:201, `ro` is 1 here)
        // is deliberately not stamped: C re-creates the params from
        // scratch on every call, so the bit never blocks the next one,
        // whereas a bit that outlived the teardown would fail the next
        // widget's `makezleparams(0)` publish.
        use crate::ported::zsh_h::{PM_REMOVABLE, PM_SPECIAL};
        let level = crate::ported::params::locallevel.load(Ordering::Relaxed) + 1;
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            for name in crate::ported::zle::zle_params::ZLEPARAM_NAMES {
                if let Some(pm) = tab.get_mut(*name) {
                    pm.level = level; // c:206 `pm->level = locallevel + 1`
                    pm.node.flags |= (PM_SPECIAL | PM_REMOVABLE) as i32; // c:200
                }
            }
        }
    }

    let cfret_val = crate::ported::exec::doshfunc(&mut synth_shf, largs, true, body_runner);
    crate::ported::zle::zle_tricky::cfret.store(cfret_val, Ordering::Relaxed);

    // c:836 — `opts[XTRACE] = oxt;` restore xtrace state.
    crate::ported::options::opt_state_set(
        &crate::ported::zsh_h::opt_name(crate::ported::zsh_h::XTRACE),
        oxt != 0,
    );

    // c:839-841 — `lastcmd = 0; incompfunc = icf; startauto = 0;`.
    // `startauto` is cleared BEFORE the c:908 recompute below; without the
    // clear the AUTO_MENU value do_completion stored at c:331 survived any
    // completer that emptied `$compstate[insert]`.
    startauto.store(0, Ordering::Relaxed); // c:841

    // c:843-925 — unwind: read the compstate values the completion function
    // may have rewritten back into the compcore globals. In C these ARE the
    // globals (the compstate entries are gsu-bound to `complist`,
    // `compinsert`, `compexact`, `comptoend`, `compoldlist`, `compoldins`),
    // so this is a plain read of a mutated variable; here it is a read of
    // `$compstate[…]`.
    //
    // Only the `usemenu` third of the c:857-907 arm existed before. Every
    // other assignment in the block — `uselist`, `forcelist`, `onlyexpl`,
    // `useline`, `insmnum`, `insspace`, `startauto`, `useexact`,
    // `movetoend`, `oldlist`, `oldins` — was simply absent, so a completion
    // function could not influence any of them: `compstate[list]=...force`
    // never forced a list, `_menu`'s `compstate[old_list]=keep` never kept
    // one, `compstate[insert]=2` never selected the 2nd match, and
    // `compstate[insert]=''` never suppressed insertion.
    //
    // Read `$compstate[…]` via the compstate hash (the canonical home), NOT
    // the flat `compstate[KEY]` bracketed param: the latter reads empty here
    // because the completion fn's write lands in the hash storage while the
    // flat param is scoped to the fn.

    // Read one `$compstate` entry as C reads its backing global at c:843+:
    // the end-of-body snapshot first, then whatever is still live.
    //
    // `None` means the port HAS NO VALUE for this entry — a state C cannot
    // be in, because `callcompfunc` itself assigned every one of these
    // globals before the call (c:753-812). Applying C's "NULL" arm to it
    // would be a mistranslation of "absent" as "empty", and that is exactly
    // what regressed `menu select`: the `insert`/`list` entries read back
    // absent, so `useline` and `uselist` were both driven to 0 and
    // `do_completion` took its c:425 else-branch (revert the line, show
    // nothing) instead of listing. On `None` the globals keep the values
    // this function published, which is the round-trip C would have seen.
    let post = |key: &str| -> Option<String> {
        if let Some(v) = compstate_end
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|h| h.get(key).cloned()))
        {
            return Some(v);
        }
        get_compstate_str(key)
    };

    // c:843-855 — uselist / forcelist / onlyexpl from `complist`.
    if let Some(post_list) = post("list") {
        uselist.store(
            if post_list.starts_with("list") {
                1 // c:846
            } else if post_list.starts_with("auto") {
                2 // c:848
            } else if post_list.starts_with("ambig") {
                3 // c:850
            } else {
                0 // c:844 / c:852
            },
            Ordering::Relaxed,
        );
        forcelist.store(
            if post_list.contains("force") { 1 } else { 0 }, // c:853
            Ordering::Relaxed,
        );
        onlyexpl.store(
            (if post_list.contains("expl") { 1 } else { 0 })      // c:854
                | (if post_list.contains("messages") { 2 } else { 0 }), // c:855
            Ordering::Relaxed,
        );
        // Keep the `complist` global in step with the param — one storage in C.
        if let Ok(mut g) = COMPLIST.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = post_list.clone();
        }
    }

    // c:857-907 — useline / usemenu / insmnum / insspace from `compinsert`.
    let post_insert = match post("insert") {
        Some(v) => v,
        // Absent: leave useline/usemenu/insmnum/insspace as published.
        None => String::new(),
    };
    let have_insert = post("insert").is_some();
    // c:857-858 — `if (!compinsert) useline = 0;`. C's test is on the
    // POINTER: it fires only when `$compstate[insert]` has no value at all.
    // An EMPTY STRING is a perfectly ordinary `char *` and falls all the way
    // through to the c:883-896 else, whose own comment spells out that the
    // empty case is the one it exists for:
    //
    //     } else {
    //         if (strpfx("menu", compinsert)) useline = 1, usemenu = 1;
    //         else if (strpfx("auto", compinsert)) useline = 1, usemenu = 2;
    //         else {
    //             useline = usemenu = 0;
    //             /* if compstate[insert] was emptied, no unambiguous prefix
    //              * ever gets inserted so allow the next tab to already start
    //              * menu completion */
    //             startauto = lastambig = isset(AUTOMENU);
    //         }
    //
    // The port used to short-circuit `post_insert.is_empty()` into the NULL
    // arm, so it set `useline = 0` and stopped — never clearing `usemenu`
    // and, far more visibly, never arming `startauto`/`lastambig`. That
    // arming is the ONLY thing that lets the SECOND Tab start menu
    // completion after a round that inserted nothing: `before_complete`
    // (c:493-495, `startauto && lastambig` → `usemenu = 2`) has no other
    // source. Measured on `bindkey -` under
    // scripts/parity_combos/full.zsh (`menu 'select=0' interactive`):
    // `$compstate[insert]` reads back EMPTY in both shells there (sampled
    // from `comppostfuncs`, the last hook `_main_complete` runs before it
    // snapshots `_lastcomp`), so c:895 is the only arming zsh gets — and
    // zsh's second Tab entered interactive menu-select and printed
    // `interactive: -[]` above the list where this port's did not. The
    // follow-on keystrokes then went to the line instead of the menu
    // filter: typing `s` narrowed zsh's list to `-s` and self-inserted here.
    if !have_insert {
        // no-op: keep the c:767-782 values
    } else if post_insert.contains("tab") {
        useline.store(-1, Ordering::Relaxed); // c:860
    } else if post_insert == "unambig"
        || post_insert == "unambiguous"
        || post_insert == "automenu-unambiguous"
    {
        // c:861-864 — C compares these three EXACTLY, and does so *before*
        // the `strpfx("menu", …)` / `strpfx("auto", …)` arms at c:885-888.
        // The ordering is load-bearing now that the entry value grew an
        // "automenu" prefix (c:770): a prefix test would see "auto" and pick
        // usemenu = 2, starting menu completion on the very FIRST TAB
        // instead of inserting the unambiguous prefix and only arming the
        // next TAB — the arming is carried by `startauto` (c:908 below).
        useline.store(1, Ordering::Relaxed); // c:864
        USEMENU.store(0, Ordering::Relaxed); // c:864
    } else if post_insert == "all" {
        useline.store(2, Ordering::Relaxed); // c:866
        USEMENU.store(0, Ordering::Relaxed); // c:866
    } else if post_insert
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_digit)
    {
        // c:867 — `idigit(*compinsert)`: insert the Nth match directly.
        // `first()` rather than `[0]`: an empty `$compstate[insert]` now
        // reaches this arm (see the c:857-858 note above), and C's
        // `idigit(*compinsert)` reads the NUL terminator there — false —
        // where an unguarded index would panic.
        useline.store(1, Ordering::Relaxed); // c:872
        USEMENU.store(3, Ordering::Relaxed); // c:872
                                             // c:873 — `insmnum = atoi(compinsert)`; inlined (leading digits).
        insmnum.store(
            post_insert
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<i32>()
                .unwrap_or(0),
            Ordering::Relaxed,
        ); // c:873
        insspace.store(
            if post_insert.ends_with(' ') { 1 } else { 0 }, // c:881
            Ordering::Relaxed,
        );
    } else {
        if post_insert.starts_with("menu") {
            // c:885
            useline.store(1, Ordering::Relaxed); // c:886
            USEMENU.store(1, Ordering::Relaxed); // c:886
        } else if post_insert.starts_with("auto") {
            // c:887
            useline.store(1, Ordering::Relaxed); // c:888
            USEMENU.store(2, Ordering::Relaxed); // c:888
        } else {
            useline.store(0, Ordering::Relaxed); // c:890
            USEMENU.store(0, Ordering::Relaxed); // c:890
                                                 // c:891-894 — "if compstate[insert] was emptied, no unambiguous
                                                 // prefix ever gets inserted so allow the next tab to already
                                                 // start menu completion".
            let am = opt_isset("AUTOMENU");
            startauto.store(am, Ordering::Relaxed); // c:894
            LASTAMBIG.store(am, Ordering::Relaxed); // c:894
        }
        // c:897-898 — `if (useline && (p = strchr(compinsert, ':')))
        //               insmnum = atoi(++p);`
        if useline.load(Ordering::Relaxed) != 0 {
            if let Some(colon) = post_insert.find(':') {
                // c:898 — `insmnum = atoi(++p)`; inlined (leading digits).
                insmnum.store(
                    post_insert[colon + 1..]
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse::<i32>()
                        .unwrap_or(0),
                    Ordering::Relaxed,
                ); // c:898
            }
        }
    }
    // c:908-911 — `startauto = startauto ||
    //     compinsert == "automenu-unambiguous" ||
    //     (bashlistfirst && isset(AUTOMENU) && !*compinsert);`
    if have_insert
        && startauto.load(Ordering::Relaxed) == 0
        && (post_insert == "automenu-unambiguous"
            || (crate::ported::zle::zle_tricky::BASHLISTFIRST.load(Ordering::Relaxed) != 0
                && opt_isset("AUTOMENU") != 0
                && post_insert.is_empty()))
    {
        startauto.store(1, Ordering::Relaxed); // c:908
    }

    // c:912 — `useexact = (compexact && !strcmp(compexact, "accept"));`
    if let Some(post_exact) = post("exact") {
        useexact.store(
            if post_exact == "accept" { 1 } else { 0 },
            Ordering::Relaxed,
        );
    }

    // `comppatinsert` (complete.c:69) is `VAL()`-bound to
    // `$compstate[pattern_insert]` (complete.c:1281), so a completer's
    // `compstate[pattern_insert]=unambiguous` (`_expand:_expand:…`,
    // `_approximate:91`) lands straight in the C global and C needs no
    // read-back here. This port keeps the two storages separate, so mirror
    // the parameter into the global now — same treatment `complist` gets at
    // c:846-853 above. Absent (`None`) keeps the c:321 value, which is the
    // state C would be in when no completer touched it.
    if let Some(post_patins) = post("pattern_insert") {
        if let Ok(mut g) = crate::ported::zle::complete::COMPPATINSERT
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
        {
            *g = post_patins;
        }
    }

    // c:914-921 — movetoend from `comptoend`.
    if let Some(post_toend) = post("to_end") {
        movetoend.store(
            if post_toend.is_empty() {
                0 // c:915
            } else if post_toend == "single" {
                1 // c:917
            } else if post_toend == "always" {
                3 // c:919
            } else {
                2 // c:921
            },
            Ordering::Relaxed,
        );
    }

    // c:923-925 — `oldlist = (hasoldlist && compoldlist &&
    //                         !strcmp(compoldlist, "keep"));
    //              oldins  = (hasoldlist && minfo.cur &&
    //                         compoldins && !strcmp(compoldins, "keep"));`
    let hasoldlist_v = hasoldlist.load(Ordering::Relaxed);
    oldlist.store(
        if hasoldlist_v != 0 && post("old_list").as_deref() == Some("keep") {
            1
        } else {
            0
        },
        Ordering::Relaxed,
    ); // c:923
    let has_cur = MINFO
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.cur.is_some())
        .unwrap_or(false);
    oldins.store(
        if hasoldlist_v != 0 && has_cur && post("old_insert").as_deref() == Some("keep") {
            1
        } else {
            0
        },
        Ordering::Relaxed,
    ); // c:924-925

    // c:932 — `lastval = lv`: the completion function's exit status must not
    // leak into the interactive shell's `$?`.
    crate::ported::builtin::LASTVAL.store(_lv, Ordering::Relaxed); // c:932

    // c:840 — incompfunc = icf. Restore.
    INCOMPFUNC.store(_icf, Ordering::Relaxed);
}

// =====================================================================
// makecomplist — `Src/Zle/compcore.c:946`.
// =====================================================================

/// Direct port of `int makecomplist(char *s, int incmd, int lst)` from
/// compcore.c:946. Top-level dispatch into the completion subsystem:
/// either the new compsys path (`callcompfunc`) or the legacy compctl
/// path (`COMPCTLMAKEHOOK`).
pub fn makecomplist(s: &str, incmd: i32, lst: i32) -> i32 {
    // c:946
    let owb = WB.load(Ordering::Relaxed); // c:946
    let owe = WE.load(Ordering::Relaxed);
    let ooffs = OFFS.load(Ordering::Relaxed);

    // c:952-958 — `if (compfunc && (p = check_param(s, 0, 0)))`.
    let mut s_owned = s.to_string();
    if compfunc_active() {
        if let Some(p) = check_param(&s_owned, false, false) {
            // c:951
            // c:952 — `s = p`, where C's `p` points at the parameter NAME and
            // check_param has already NUL-terminated it at the end of the
            // name (`b[we-wb] = '\0'`, c:1297). Taking `s[p..]` alone kept
            // everything that followed the name (the `}` of a `${…}`, any
            // `:modifiers`, the ignored suffix), so `callcompfunc` received
            // e.g. `PA}` instead of `PA` and published that as `$PREFIX`.
            // `we - wb` is exactly `e - b`, set by check_param at c:1294-1295.
            let namelen = (WE.load(Ordering::Relaxed) - WB.load(Ordering::Relaxed)).max(0) as usize;
            let tail = &s_owned[p..];
            s_owned = tail[..namelen.min(tail.len())].to_string(); // c:952 + c:1297
            PARWB.store(owb, Ordering::Relaxed); // c:953
            PARWE.store(owe, Ordering::Relaxed); // c:955
            PAROFFS.store(ooffs, Ordering::Relaxed); // c:956
        } else {
            PARWB.store(-1, Ordering::Relaxed); // c:958
        }
    } else {
        PARWB.store(-1, Ordering::Relaxed); // c:958
    }

    linwhat.store(INWHAT.load(Ordering::Relaxed), Ordering::Relaxed); // c:960

    if compfunc_active() {
        // c:962
        let os = s_owned.clone(); // c:964
        let onm = nmatches.load(Ordering::Relaxed); // c:965
        let odm = diffmatches.load(Ordering::Relaxed); // c:965
        let osi = movefd(0); // c:965 movefd(0)
                             // c:965 moves the shell's stdin off fd 0 and c:1013/1035/1039 `redup(osi, 0)`
                             // puts it back, so the completion function runs with fd 0 FREE — and
                             // the very next `open()`/`opendir()` in that window is handed
                             // descriptor 0. zsh parks /dev/null there for exactly the same window
                             // (the idiom, with its rationale, is spelled out at
                             // Src/Zle/zle_main.c:1521-1526: "Many commands don't like having a
                             // closed stdin, open on /dev/null instead"); measured against zsh 5.9,
                             // `[[ /dev/fd/0 -ef /dev/null ]]` is TRUE inside a completion
                             // function, while zshrs left fd 0 closed.
                             //
                             // A closed fd 0 is not merely untidy here: `_path_files -W /dev -g
                             // '*(-/)'` calls `opendir("/dev")`, which lands on descriptor 0, so
                             // `/dev/fd/0` — the target of the `/dev/stdin` symlink — resolves to
                             // the very directory being scanned. The `-/` qualifier then stats a
                             // directory and admits `/dev/stdin`, and `mount /dev/<TAB>` listed a
                             // bogus `stdin@` next to `fd/` and `monotonic/`.
        if osi > 0 {
            unsafe {
                let devnull = std::ffi::CString::new("/dev/null").unwrap();
                let _ = libc::open(devnull.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
            }
        }

        // c:967-968 — bmatchers = mstack = NULL.
        if let Ok(mut g) = bmatchers.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        // c:970-971 — ainfo = fainfo = hcalloc(sizeof(struct aminfo)).
        if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Aminfo::default());
        }
        if let Ok(mut g) = fainfo.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Aminfo::default());
        }
        if let Ok(mut g) = freecl.get_or_init(|| Mutex::new(None)).lock() {
            *g = None; // c:973
        }
        if VALIDLIST.load(Ordering::Relaxed) == 0 {
            LASTAMBIG.store(0, Ordering::Relaxed);
            // c:976
        }
        if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            g.clear(); // c:977
        }
        mnum.store(0, Ordering::Relaxed); // c:978
        unambig_mnum.store(-1, Ordering::Relaxed); // c:979
        if let Ok(mut g) = isuf.get_or_init(|| Mutex::new(String::new())).lock() {
            g.clear(); // c:980
        }
        insmnum.store(ZMULT.load(Ordering::Relaxed), Ordering::Relaxed); // c:981
        oldlist.store(0, Ordering::Relaxed); // c:986
        oldins.store(0, Ordering::Relaxed); // c:986
        begcmgroup(Some("default"), 0); // c:987
        MENUCMP.store(0, Ordering::Relaxed); // c:988
        menuacc.store(0, Ordering::Relaxed); // c:988
        newmatches.store(0, Ordering::Relaxed); // c:988
        onlyexpl.store(0, Ordering::Relaxed); // c:988

        let dup_s = crate::ported::mem::dupstring(&os); // c:990
        let cf_name = compfunc
            .get_or_init(|| Mutex::new(None))
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .unwrap_or_default();
        callcompfunc(&dup_s, &cf_name); // c:991
        endcmgroup(None); // c:992

        // c:995 — runhookdef(COMPCTLCLEANUPHOOK, NULL).
        runhookdef_compcore("COMPCTLCLEANUPHOOK"); // c:995

        if oldlist.load(Ordering::Relaxed) != 0 {
            // c:997
            nmatches.store(onm, Ordering::Relaxed); // c:998
            diffmatches.store(odm, Ordering::Relaxed); // c:999
            VALIDLIST.store(1, Ordering::Relaxed); // c:1000
            if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                if let Ok(last) = lastmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                    *g = last.clone(); // c:1001
                }
            }
            if let Ok(mut g) = lmatches.get_or_init(|| Mutex::new(None)).lock() {
                let last_l = lastlmatches
                    .get_or_init(|| Mutex::new(None))
                    .lock()
                    .ok()
                    .and_then(|g| g.clone());
                *g = last_l; // c:1007
            }
            // c:1008-1011 — `if (pmatches) freematches(pmatches, 1)`.
            let drained = pmatches
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .map(|mut g| std::mem::take(&mut *g))
                .unwrap_or_default();
            freematches(drained, 1); // c:1009-1010
            hasperm.store(0, Ordering::Relaxed); // c:1011
            redup(osi); // c:1012
            return 0; // c:1013
        }
        if !lastmatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map(|g| g.is_empty())
            .unwrap_or(true)
        {
            // c:1015
            if let Ok(mut g) = lastmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                g.clear(); // c:1016-1017
            }
        }
        permmatches(1); // c:1019
                        // c:1020-1029 — copy pmatches → amatches/lastmatches; swap holders.
        let p_snap = pmatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_default();
        if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            *g = p_snap.clone(); // c:1020
        }
        lastpermmnum.store(permmnum.load(Ordering::Relaxed), Ordering::Relaxed); // c:1021
        lastpermgnum.store(permgnum.load(Ordering::Relaxed), Ordering::Relaxed); // c:1022
        if let Ok(mut g) = lastmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            *g = p_snap; // c:1024
        }
        let lm_snap = lmatches
            .get_or_init(|| Mutex::new(None))
            .lock()
            .ok()
            .and_then(|g| g.clone());
        if let Ok(mut g) = lastlmatches.get_or_init(|| Mutex::new(None)).lock() {
            *g = lm_snap; // c:1025
        }
        if let Ok(mut g) = pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            g.clear(); // c:1026
        }
        hasperm.store(0, Ordering::Relaxed); // c:1027
        hasoldlist.store(1, Ordering::Relaxed); // c:1028

        let any_nm =
            nmatches.load(Ordering::Relaxed) != 0 || nmessages.load(Ordering::Relaxed) != 0;
        let errset = errflag_get();
        tracing::debug!(
            target: "compsys_args",
            nm = nmatches.load(Ordering::Relaxed),
            nmsg = nmessages.load(Ordering::Relaxed),
            errset,
            "makecomplist RETURN"
        );
        if any_nm && !errset {
            // c:1030
            VALIDLIST.store(1, Ordering::Relaxed); // c:1031
            redup(osi); // c:1032
            return 0; // c:1033
        }
        redup(osi); // c:1035
        return 1; // c:1036
    } else {
        // c:1038
        // c:1040-1047 — compctl dispatch via COMPCTLMAKEHOOK.
        let mut dat = Ccmakedat {
            str: Some(s_owned.clone()), // c:1042
            incmd,                      // c:1043
            lst,                        // c:1044
        };
        runhookdef_compctlmake(&mut dat); // c:1045
        runhookdef_compcore("COMPCTLCLEANUPHOOK"); // c:1048
        return dat.lst; // c:1050
    }
}

// =====================================================================
// multiquote — `Src/Zle/compcore.c:1065`.
// =====================================================================

/// Port of `mod_export char *multiquote(char *s, int ign)` from
/// compcore.c:1064.
pub fn multiquote(s: &str, ign: i32) -> String {
    // c:1065
    // c:1067 — `char *p = compqstack;`. C takes the global POINTER and
    // walks it; nothing is copied. `compqstack` holds one byte per open
    // quoting level (c:301-308 allocates 2 bytes; c:1853-1859 pushes one
    // more per nesting level), so it is a handful of bytes at most.
    // Copying it into a stack buffer keeps the read allocation-free —
    // `multiquote` runs twice per completion match (compmatch.c:1160 and
    // c:1172 via `comp_match`), so a 46765-match `compadd -k functions`
    // was doing ~94k heap clones of a one-byte String.
    let mut qbuf = [0u8; 32];
    let mut qspill = String::new();
    let qlen = match COMPQSTACK.get_or_init(|| Mutex::new(String::new())).lock() {
        Ok(g) => {
            let b = g.as_bytes();
            if b.len() <= qbuf.len() {
                qbuf[..b.len()].copy_from_slice(b);
                b.len()
            } else {
                // Deeper nesting than the buffer holds: fall back to an
                // owned copy so no level is ever dropped.
                qspill = g.clone();
                usize::MAX
            }
        }
        Err(_) => 0,
    };
    let p_bytes: &[u8] = if qlen == usize::MAX {
        qspill.as_bytes()
    } else {
        &qbuf[..qlen]
    };
    if !p_bytes.is_empty() && (ign == 0 || p_bytes.len() > 1) {
        // c:1070
        let start = if ign != 0 { 1 } else { 0 }; // c:1071
        let mut cur = s.to_string();
        for &q in &p_bytes[start..] {
            // c:1073
            let qt = match q as i32 {
                // c:1074
                x if x == QT_BACKSLASH => QT_BACKSLASH,
                x if x == QT_SINGLE => QT_SINGLE,
                x if x == QT_DOUBLE => QT_DOUBLE,
                x if x == QT_DOLLARS => QT_DOLLARS,
                _ => QT_BACKSLASH,
            };
            cur = crate::ported::utils::quotestring(&cur, qt);
        }
        cur // c:1092
    } else {
        s.to_string() // c:1092
    }
}

// =====================================================================
// tildequote — `Src/Zle/compcore.c:1092`.
// =====================================================================

/// Port of `mod_export char *tildequote(char *s, int ign)` from
/// compcore.c:1091.
pub fn tildequote(s: &str, ign: i32) -> String {
    // c:1092
    let bytes = s.as_bytes(); // c:1092
    let tilde = !bytes.is_empty() && bytes[0] == b'~'; // c:1097
    let staged = if tilde {
        // c:1098
        let mut tmp = String::with_capacity(s.len());
        tmp.push('x');
        tmp.push_str(&s[1..]);
        tmp
    } else {
        s.to_string()
    };
    let mut quoted = multiquote(&staged, ign); // c:1099
    if tilde && !quoted.is_empty() {
        // c:1100
        let mut new_q = String::with_capacity(quoted.len());
        let mut swapped = false;
        for c in quoted.chars() {
            if !swapped && c == 'x' {
                new_q.push('~');
                swapped = true;
            } else {
                new_q.push(c);
            }
        }
        quoted = new_q;
    }
    quoted // c:1101
}

// =====================================================================
// check_param — `Src/Zle/compcore.c:1113`.
// =====================================================================

/// Direct port of `static char *check_param(char *s, int set, int test)`
/// from compcore.c:1113. Walks backwards from cursor in `s` looking
/// for `$<name>`. When found and the cursor sits inside the name,
/// returns the byte index in `s` where the name starts; updates
/// `ispar`/`parq`/`eparq` (when `!test`) and `ipre`/`ripre`/`isuf`/
/// `parpre`/`parflags`/`mflags`/`wb`/`we`/`offs` (when `set`).
/// Returns `None` when there's no parameter expression at the cursor.
pub fn check_param(s: &str, set: bool, test: bool) -> Option<usize> {
    // c:1113

    // c:1117-1118 — zsfree(parpre); parpre = NULL.
    if let Ok(mut g) = parpre.get_or_init(|| Mutex::new(String::new())).lock() {
        g.clear();
    }

    if !test {
        // c:1120
        ispar.store(0, Ordering::Relaxed); // c:1121
        parq.store(0, Ordering::Relaxed); // c:1121
        eparq.store(0, Ordering::Relaxed); // c:1121
    }

    let bytes = s.as_bytes(); // local view
    let offs_v = OFFS.load(Ordering::Relaxed) as usize; // c:1140 cursor in word

    let mut found = false; // c:1115
    let mut qstring = false; // c:1115
    let mut p: usize = offs_v.min(bytes.len().saturating_sub(1)); // c:1140 p = s + offs

    // get_comp_string returns the word untokenized, so the `$` sigil
    // arrives as a literal 0x24 rather than the String token C scans for.
    // Treat the literal `$` as equivalent to the String token here so
    // `$VAR<Tab>` parameter completion fires; the len_utf8-based cursor
    // math below already handles the 1-byte literal vs 2-byte token.
    let is_str = |c: char| c == Stringg || c == '$';

    // c:1140-1162 — scan backward for `String` or `Qstring`.
    loop {
        if p < bytes.len() {
            let ch = char_at(bytes, p);
            if is_str(ch) || ch == Qstring {
                // c:1141
                let next = char_at(bytes, p + ch.len_utf8());
                let snull_next = is_str(ch) && next == Snull; // c:1151
                let qstr_quot = ch == Qstring && next == '\''; // c:1152
                if p < offs_v && !snull_next && !qstr_quot {
                    found = true; // c:1154
                    qstring = ch == Qstring; // c:1155
                    break;
                }
            }
        }
        if p == 0 {
            break;
        } // c:1160
        p = prev_char_index(bytes, p);
    }

    if found {
        // c:1166
        // c:1173-1174 — fold `$$$$` chains.
        while p > 0 {
            let prev = prev_char_index(bytes, p);
            let pc = char_at(bytes, prev);
            if is_str(pc) || pc == Qstring {
                p = prev;
            } else {
                break;
            }
        }
        loop {
            // c:1175-1176
            let n1 = p + char_at(bytes, p).len_utf8();
            if n1 >= bytes.len() {
                break;
            }
            let c1 = char_at(bytes, n1);
            let n2 = n1 + c1.len_utf8();
            if n2 >= bytes.len() {
                break;
            }
            let c2 = char_at(bytes, n2);
            if (is_str(c1) || c1 == Qstring) && (is_str(c2) || c2 == Qstring) {
                p = n2;
            } else {
                break;
            }
        }
    }

    // c:1179 — guard against `$(`, `$[`, `$'`.
    let next_char = if p + 1 <= bytes.len() {
        let dollar_len = char_at(bytes, p).len_utf8();
        char_at(bytes, p + dollar_len)
    } else {
        '\0'
    };
    if !(found && next_char != Inpar && next_char != Inbrack && next_char != Snull) {
        return None; // c:1316
    }

    // c:1181 — b = p + 1 (start of body), e = b initially.
    let dollar_len = char_at(bytes, p).len_utf8();
    let mut b: usize = p + dollar_len; // c:1181
    let mut br: i32 = 1; // c:1182
    let mut nest: i32 = 0; // c:1182

    // get_comp_string returns the word untokenized, so `${…}` arrives with a
    // literal `{`/`}` rather than the Inbrace/Outbrace tokens C matches here;
    // accept either so `${PA<Tab>` is recognized as a braced parameter.
    let brace_ch = char_at(bytes, b);
    if brace_ch == Inbrace || brace_ch == '{' {
        let (ib, ob) = if brace_ch == '{' {
            ('{', '}')
        } else {
            (Inbrace, Outbrace)
        };
        // c:1184
        // c:1188 — `if (!skipparens(Inbrace, Outbrace, &tb) && tb - s <= offs) return NULL;`
        let mut tb: &str = &s[b..];
        let bal = crate::ported::utils::skipparens(ib, ob, &mut tb);
        let tb_after = s.len() - tb.len();
        if bal == 0 && tb_after <= offs_v {
            return None; // c:1189
        }

        b += brace_ch.len_utf8(); // c:1192 b++
        br += 1;
        // c:1193-1203 — skip leading `(...)` flag group. C has a
        // ternary `qstring ? skipparens('(',')',&b) : skipparens(Inpar,Outpar,&b)`
        // — two source-level skipparens calls. Mirror that explicitly
        // so the call-coverage metric matches C.
        let mut b_str: &str = &s[b..];
        // `get_comp_string` hands this port the word UNTOKENIZED
        // (zle_tricky.c:2219), so the `(`/`)` of a `${(flags)name}` group
        // arrive as literals in the non-qstring case as well. Pick the pair
        // that is actually present — the same accommodation the
        // `Inbrace`/`{` test above already makes. Without it `skipparens`
        // was handed `Inpar` against a literal `(`, returned -1 (its
        // "wrong opening char" code) instead of 0, `b` never advanced past
        // the flags, the name scan then found `(` where a name should be
        // and `check_param` bailed with `ispar == 0`: `${(k)<TAB>` never
        // reached the `-brace-parameter-` context at all.
        let flag_ret: i32 = if qstring || char_at(bytes, b) == '(' {
            crate::ported::utils::skipparens('(', ')', &mut b_str)
        } else {
            crate::ported::utils::skipparens(Inpar, Outpar, &mut b_str)
        };
        let after_flags_pos = s.len() - b_str.len();
        if flag_ret > 0 || after_flags_pos > offs_v {
            ispar.store(2, Ordering::Relaxed); // c:1201
            return None; // c:1202
        }
        b = after_flags_pos;

        // c:1205 — detect `nest` from preceding `${ ${` chain.
        let mut tb = p;
        while tb > 0 {
            let prev = prev_char_index(bytes, tb);
            let pc = char_at(bytes, prev);
            if pc == Outbrace || pc == Inbrace {
                tb = prev;
                break;
            }
            tb = prev;
        }
        if tb > 0 {
            let cc = char_at(bytes, tb);
            let prev = prev_char_index(bytes, tb);
            let pp = char_at(bytes, prev);
            if cc == Inbrace && (pp == Stringg || cc == Qstring) {
                nest = 1; // c:1207
            }
        }
    }

    // c:1212-1213 — skip `^=~` prefix flags.
    while b < bytes.len() {
        let c = char_at(bytes, b);
        if c == '^' || c == Hat || c == '=' || c == Equals || c == '~' || c == Tilde {
            b += c.len_utf8();
        } else {
            break;
        }
    }
    // c:1215 — `#` / `+` length-prefix.
    if b < bytes.len() {
        let c = char_at(bytes, b);
        if c == '#' || c == Pound || c == '+' {
            b += c.len_utf8();
        }
    }

    let mut e: usize = b; // c:1219
    if br != 0 {
        // c:1220
        let qopen = if test { Dnull } else { '"' };
        while e < bytes.len() && char_at(bytes, e) == qopen {
            // c:1221
            e += qopen.len_utf8();
            parq.fetch_add(1, Ordering::Relaxed); // c:1221
        }
        if !test {
            b = e;
        } // c:1223
    }

    // c:1226-1252 — find end of name.
    if e < bytes.len() {
        let c = char_at(bytes, e);
        let one_char_name = matches!(c,
            ch if ch == Quest || ch == Star || ch == Stringg || ch == Qstring
                || ch == '?' || ch == '*' || ch == '$' || ch == '-' || ch == '!' || ch == '@');
        if one_char_name {
            // c:1230
            e += c.len_utf8();
        } else if c.is_ascii_digit() {
            // c:1232
            while e < bytes.len() && char_at(bytes, e).is_ascii_digit() {
                // c:1233
                e += 1;
            }
        } else {
            // c:1232-1241 — the `itype_end(e, INAMESPC, 0)` do/while:
            //
            //     do { e = ie;
            //          if (comppatmatch && *comppatmatch &&
            //              (*e == Star || *e == Quest))  ie = e + 1;
            //          else                              ie = itype_end(e, …);
            //     } while (ie != e);
            //
            // The loop, not just its first iteration, is what lets a glob
            // metacharacter sit INSIDE the parameter name when
            // `$compstate[pattern_match]` is on: `$fo*ba<TAB>` must treat
            // `fo*ba` as one name. The port ran a single `walk_namespace`
            // and stopped at the `*`, so pattern parameter-name completion
            // saw the name end early and completed the wrong span.
            let patmatch_on = comppatmatch
                .get_or_init(|| Mutex::new(None))
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .map(|v| !v.is_empty())
                .unwrap_or(false); // c:1235
            let mut ie = e + walk_namespace(&bytes[e..]); // c:1232
            loop {
                e = ie; // c:1234
                if e >= bytes.len() {
                    break;
                }
                let ec = char_at(bytes, e);
                if patmatch_on && (ec == Star || ec == Quest) {
                    ie = e + ec.len_utf8(); // c:1237
                } else {
                    ie = e + walk_namespace(&bytes[e..]); // c:1239
                }
                if ie == e {
                    break; // c:1240
                }
            }
            if e == b && c == '.' {
                // c:1242-1250 — a lone `.` counts as an incomplete name.
                e += 1;
            }
        }
    }

    // c:1259 — `if (offs <= e - s && offs >= b - s)`.
    if offs_v <= e && offs_v >= b {
        // c:1263 — strip trailing `"`s when br set.
        if br != 0 {
            let qopen = if test { Dnull } else { '"' };
            let mut pq = e;
            while pq < bytes.len() && char_at(bytes, pq) == qopen {
                pq += qopen.len_utf8();
                parq.fetch_sub(1, Ordering::Relaxed);
                eparq.fetch_add(1, Ordering::Relaxed);
            }
        }
        if test {
            // c:1269
            return Some(b); // c:1270
        }
        if set {
            // c:1273
            if br >= 2 {
                // c:1274
                mflags.fetch_or(CMF_PARBR, Ordering::Relaxed); // c:1275
                if nest != 0 {
                    // c:1276
                    mflags.fetch_or(CMF_PARNEST, Ordering::Relaxed); // c:1277
                }
            }
            // c:1280 — `isuf = dupstring(e); untokenize(isuf)`.
            let mut tail = String::from_utf8_lossy(&bytes[e..]).into_owned();
            tail = strip_tokens(&tail); // crate::lex::untokenize substitute
            if let Ok(mut g) = isuf.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = tail;
            }
            // c:1284 — `ripre = dyncat(ripre, s_through_b)`.
            let head = String::from_utf8_lossy(&bytes[..b]).into_owned();
            if let Ok(mut g) = ripre.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = format!("{}{}", *g, head);
            }
            if let Ok(mut g) = ipre.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = strip_tokens(&format!("{}{}", *g, head));
            }
        }
        // c:1295 — save prefix for compfunc.
        let cf_active = compfunc
            .get_or_init(|| Mutex::new(None))
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if cf_active {
            let pf = if br >= 2 {
                CMF_PARBR | (if nest != 0 { CMF_PARNEST } else { 0 })
            } else {
                0
            };
            parflags.store(pf, Ordering::Relaxed); // c:1298
            let head = String::from_utf8_lossy(&bytes[..b]).into_owned();
            if let Ok(mut g) = parpre.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = strip_tokens(&head); // c:1301
            }
        }
        // c:1306 — adjust wb/we/offs.
        let off_delta = b as i32;
        OFFS.fetch_sub(off_delta, Ordering::Relaxed); // c:1306
        let new_offs = OFFS.load(Ordering::Relaxed);
        let zlc = ZLEMETACS.load(Ordering::Relaxed);
        WB.store(zlc - new_offs, Ordering::Relaxed); // c:1307
        WE.store(
            WB.load(Ordering::Relaxed) + (e - b) as i32,
            Ordering::Relaxed,
        ); // c:1308
        ispar.store(if br >= 2 { 2 } else { 1 }, Ordering::Relaxed); // c:1309
        return Some(b); // c:1311
    } else if offs_v > e && e < bytes.len() && char_at(bytes, e) == ':' {
        // c:1312
        // c:1313-1316 — colon-modifier guess.
        let offsptr = offs_v;
        let mut e2 = e;
        while e2 < offsptr && e2 < bytes.len() {
            let c = char_at(bytes, e2);
            if c != ':' && !c.is_alphanumeric() {
                break;
            }
            e2 += c.len_utf8();
        }
        ispar.store(if br >= 2 { 2 } else { 1 }, Ordering::Relaxed); // c:1316
        return None; // c:1317
    }

    let _ = (Bnull,); // silence unused-import warning if Bnull not hit
    None // c:1320
}

// =====================================================================
// rembslash — `Src/Zle/compcore.c:1323`.
// =====================================================================

/// Port of `mod_export char *rembslash(char *s)` from compcore.c:1322.
///
/// "Strip backslash escapes from a token, treating `\X` as `X`."
pub fn rembslash(s: &str) -> String {
    // c:1323
    let mut result = String::with_capacity(s.len()); // c:1323
    let mut chars = s.chars().peekable(); // c:1327
    while let Some(c) = chars.next() {
        if c == '\\' {
            // c:1328
            if let Some(nxt) = chars.next() {
                // c:1329
                result.push(nxt);
            }
        } else {
            result.push(c); // c:1343-1333
        }
    }
    result // c:1343
}

// =====================================================================
// remsquote — `Src/Zle/compcore.c:1343`.
// =====================================================================

/// Port of `mod_export int remsquote(char *s)` from compcore.c:1342.
pub fn remsquote(s: &mut String) -> i32 {
    // c:1343
    let rcquotes = isset(RCQUOTES); // c:1343
    let qa: usize = if rcquotes { 1 } else { 3 };

    let bytes = s.as_bytes(); // c:1346
    let mut t = Vec::<u8>::with_capacity(bytes.len());
    let mut ret: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        // c:1348
        let matched = if qa == 1 {
            // c:1349
            i + 1 < bytes.len() && bytes[i] == b'\'' && bytes[i + 1] == b'\''
        } else {
            i + 3 < bytes.len()                                              // c:1351
                && bytes[i]     == b'\''
                && bytes[i + 1] == b'\\'
                && bytes[i + 2] == b'\''
                && bytes[i + 3] == b'\''
        };
        if matched {
            ret += qa as i32; // c:1352
            t.push(b'\''); // c:1353
            i += qa + 1; // c:1354
        } else {
            t.push(bytes[i]); // c:1356
            i += 1;
        }
    }
    *s = String::from_utf8(t).unwrap_or_default(); // c:1357
    ret // c:1366
}

// =====================================================================
// ctokenize — `Src/Zle/compcore.c:1366`.
// =====================================================================

/// Port of `mod_export char *ctokenize(char *p)` from compcore.c:1365.
///
/// C calls `tokenize(p)` first then walks the string replacing
/// unescaped `$`/`{`/`}` with the token bytes `String`/`Inbrace`/
/// `Outbrace`. Backslash-escaped variants become `Bnull`.
pub fn ctokenize(p: &str) -> String {
    // c:1366
    // c:1370 — `tokenize(p);` ran FIRST in C and was missing here entirely,
    // so the glob metacharacters (`*`, `?`, `[`, `]`, `~`, `^`, `#`, …) that
    // `tokenize` turns into their high-bit token chars stayed literal. Every
    // consumer of `comp_str(untok=0)` (c:1411-1414) therefore received a
    // string that `haswilds`/`patcompile` read as plain text, so a typed
    // pattern in $PREFIX/$SUFFIX was never seen as a pattern. The loop below
    // is only the extra `$`/`{`/`}` pass C runs after tokenize.
    let mut tokenized = p.to_string();
    crate::ported::glob::tokenize(&mut tokenized); // c:1370
    let p: &str = &tokenized;
    let bytes = p.as_bytes(); // c:1372
    let mut out = Vec::<u8>::with_capacity(bytes.len());
    let mut bslash = false; // c:1369
    let mut prev_idx: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i]; // c:1373
        if b == b'\\' {
            // c:1374
            bslash = true;
            out.push(b);
            prev_idx = Some(out.len() - 1);
        } else {
            if b == b'$' || b == b'{' || b == b'}' {
                // c:1377
                if bslash {
                    // c:1378
                    if let Some(pi) = prev_idx {
                        // c:1379
                        out.truncate(pi);
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(Bnull.encode_utf8(&mut buf).as_bytes());
                    }
                    out.push(b);
                } else {
                    let tok = if b == b'$' {
                        Stringg
                    }
                    // c:1381
                    else if b == b'{' {
                        Inbrace
                    }
                    // c:1382
                    else {
                        Outbrace
                    }; // c:1382
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(tok.encode_utf8(&mut buf).as_bytes());
                }
            } else {
                out.push(b);
            }
            bslash = false; // c:1384
            prev_idx = Some(out.len().saturating_sub(1));
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default() // c:1403
}

// =====================================================================
// comp_str — `Src/Zle/compcore.c:1403`.
// =====================================================================

/// Port of `mod_export char *comp_str(int *ipl, int *pl, int untok)`
/// from compcore.c:1402.
pub fn comp_str(untok: bool) -> (String, i32, i32) {
    // c:1403
    let mut p = COMPPREFIX
        .get_or_init(|| Mutex::new(String::new())) // c:1405
        .lock()
        .unwrap()
        .clone();
    let mut s = COMPSUFFIX
        .get_or_init(|| Mutex::new(String::new())) // c:1406
        .lock()
        .unwrap()
        .clone();
    let ip = COMPIPREFIX
        .get_or_init(|| Mutex::new(String::new())) // c:1407
        .lock()
        .unwrap()
        .clone();
    if !untok {
        // c:1411
        p = ctokenize(&p); // c:1412
        p = p.chars().filter(|&c| c != Bnull).collect(); // c:1413 remnulargs
        s = ctokenize(&s); // c:1414
        s = s.chars().filter(|&c| c != Bnull).collect(); // c:1415
    }
    let lp = p.len() as i32; // c:1417
    let lip = ip.len() as i32; // c:1419
    let mut str = String::with_capacity(ip.len() + p.len() + s.len() + 1); // c:1420
    str.push_str(&ip); // c:1435
    str.push_str(&p); // c:1435
    str.push_str(&s); // c:1435
    (str, lip, lp) // c:1435-1430
}

// =====================================================================
// comp_quoting_string — `Src/Zle/compcore.c:1435`.
// =====================================================================

/// Port of `mod_export char *comp_quoting_string(int stype)` from
/// compcore.c:1434.
pub fn comp_quoting_string(stype: i32) -> &'static str {
    // c:1435
    match stype {
        // c:1435
        x if x == QT_SINGLE => "'",   // c:1439-1440
        x if x == QT_DOUBLE => "\"",  // c:1441-1442
        x if x == QT_DOLLARS => "$'", // c:1443-1444
        _ => {
            // c:1445
            let _ = QT_BACKSLASH;
            "\\" // c:1446
        }
    }
}

// =====================================================================
// set_comp_sep — `Src/Zle/compcore.c:1460`.
// =====================================================================

/// Direct port of `int set_comp_sep(void)` from compcore.c:1458 —
/// the `compset -q` driver that re-parses the current completion
/// word splitting it on the IFS, then resubmits the right slice
/// as the new completion target.
///
/// Inputs are now published/correct: `wb`/`we`/`offs` are written by
/// `get_comp_string` (WB/WE/OFFS shared statics) and `compqstack` by
/// `callcompfunc`'s c:305 reset (deduped to `complete::COMPQSTACK`).
///
/// Byte model: all of C's single-metafied-byte index arithmetic (the
/// `inull` walk c:1774-1804, the `chuck` removals, the `s[swb-1-sqq+dq]`
/// indexing c:1830, the `p[soffs]` chuck c:1739) is performed on local
/// single-byte-metafied `Vec<u8>` buffers built by `to_sb` (each
/// token-null marker `Snull`/`Dnull`/`Bnull` maps to ONE byte
/// 0x9d/0x9e/0x9f; `Meta`-escapes stay two bytes) and converted back
/// with `from_sb` (byte -> `char`, the char-per-metafied-byte form the
/// downstream comp* globals expect). `wb`/`we`/`zlemetacs` are consumed
/// as byte offsets. For ASCII completion words — command names, paths,
/// options, the dominant `compset -q` case — byte == char, so the
/// offsets are exact and the algorithm is a verifiable translation of C.
///
/// Known limitation (INHERITED from zshrs's input/lexer model, not a
/// defect of this port): for non-ASCII quoted words the shared cursor
/// model conflates byte and char units — `ingetc` steps the input by
/// char while `inbufct`/`zlemetall` are byte lengths (input.rs:355/540,
/// lex.rs:3013-3024) — so the incoming `wb`/`we`/`zlemetacs` diverge
/// from single-byte offsets. Tracked separately with the
/// `get_comp_string` quote-form tail; see that function's note.
///
/// The `QT_DOLLARS` (`$'...'`) arm (c:1613-1622) is fully wired: the
/// `getkeystring_with` decode applies `GETKEY_UPDATE_OFFSET`, so
/// both the `dolq` byte-count delta and the `css += zlemetacs - j`
/// cursor micro-adjustment are computed (inheriting the same non-ASCII
/// byte/char caveat noted above).
pub fn set_comp_sep() -> i32 {
    use crate::ported::lex::{
        ctxtlex, noaliases, set_noaliases, set_tok, set_tokstr, tok, tokstr, untokenize,
        LEX_LEXFLAGS,
    };
    use crate::ported::string::{dupstring_wlen, tricat};
    use crate::ported::utils::getkeystring_with;
    use crate::ported::zle::comp_h::{CP_QUOTE, CP_QUOTING};
    // COMPPREFIX/COMPSUFFIX/COMPIPREFIX/COMPQSTACK are already imported at
    // the module top; only the remaining comp* globals are pulled in here.
    use crate::ported::zle::complete::{
        COMPCURRENT, COMPISUFFIX, COMPQIPREFIX, COMPQISUFFIX, COMPQUOTE, COMPQUOTING, COMPWORDS,
    };
    use crate::ported::zle::zle_utils::{zle_restore_positions, zle_save_positions};
    use crate::ported::zsh_h::{
        Meta, COMPLETEINWORD, ENDINPUT, GETKEYS_DOLLARS_QUOTE, GETKEY_UPDATE_OFFSET, LEXERR,
        LEXFLAGS_ZLE, STRING_LEX,
    };

    // ── single-byte-metafied <-> char-per-byte String conversions ──
    // Each metafied byte is one `char` in the port's string world, so
    // `c as u8` for c < 0x100 reconstructs the C single-byte buffer
    // (markers 0x9d/0x9e/0x9f = one byte, Meta-escapes = 0x83 + xor).
    let to_sb = |s: &str| -> Vec<u8> {
        let mut v = Vec::with_capacity(s.len());
        for c in s.chars() {
            let cp = c as u32;
            if cp < 0x100 {
                v.push(cp as u8);
            } else {
                let mut b = [0u8; 4];
                v.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            }
        }
        v
    };
    let from_sb = |b: &[u8]| -> String { b.iter().map(|&x| x as char).collect() };
    let snap = |g: &'static OnceLock<Mutex<String>>| -> String {
        g.get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap()
            .clone()
    };
    let put = |g: &'static OnceLock<Mutex<String>>, v: String| {
        *g.get_or_init(|| Mutex::new(String::new())).lock().unwrap() = v;
    };

    // marker byte values (Snull/Dnull/Bnull/Stringg/Qstring are `char`).
    let snull_b = Snull as u32 as u8; // 0x9d
    let dnull_b = Dnull as u32 as u8; // 0x9e
    let bnull_b = Bnull as u32 as u8; // 0x9f
    let stringg_b = Stringg as u32 as u8; // 0x85 ($)
    let qstring_b = Qstring as u32 as u8; // 0x8c ("$)
    let meta_b: u8 = Meta; // 0x83

    // c:1460 — s = comp_str(&lip, &lp, 1) with untok = 1.
    let (s_full, lip, lp) = comp_str(true);
    // c:1473 — int owe = we, owb = wb.
    let owe = WE.load(Ordering::Relaxed);
    let owb = WB.load(Ordering::Relaxed);

    let mut foo: Vec<String> = Vec::new(); // c:1462 newlinklist()

    // c:1478-1500 — locals.
    let mut swb: i32 = 0; // c:1490
    let mut swe: i32 = 0;
    let scs: i32; // cursor (fixed once set below)
    let mut soffs: i32 = 0;
    let ne = *crate::ported::utils::noerrs_lock().lock().unwrap(); // c:1479
    let mut got = false;
    let mut i: i32 = 0;
    let mut cur: i32 = -1;
    let mut css: i32 = 0;
    let mut remq = false;
    let mut dq: i32 = 0;
    let mut sq: i32 = 0;
    let qttype: i32;
    let mut sqq: i32 = 0;
    let mut lsq: i32 = 0;
    let mut qa: i32 = 0;
    let mut dolq: i32 = 0;
    let ois = INSTRING.load(Ordering::Relaxed); // c:1471
    let oib = INBACKT.load(Ordering::Relaxed);
    let noffs = lp; // active-prefix length
    let ona = noaliases();

    // c:1476 — s += lip; wb += lip; untokenize(s).
    let s_after: String = if (lip as usize) <= s_full.len() {
        s_full[lip as usize..].to_string()
    } else {
        String::new()
    };
    WB.store(owb + lip, Ordering::Relaxed);
    let s = untokenize(&s_after);
    let s_b_full = to_sb(&s); // reconstructed arg, single-byte

    // c:1483-1488 — zle_save_positions / addedx / noerrs / zcontext / lexflags.
    zle_save_positions();
    let ol = snap(&ZLEMETALINE);
    ADDEDX.store(1, Ordering::Relaxed);
    *crate::ported::utils::noerrs_lock().lock().unwrap() = 1;
    let lex_saved = lexsave(); // zcontext_save()
    LEX_LEXFLAGS.set(LEXFLAGS_ZLE);

    // c:1494-1499 — tl = strlen(s)+2; tmp = " " + s[..noffs] + 'x' + s[noffs..].
    let noffs_u = (noffs.max(0) as usize).min(s_b_full.len());
    let tl0 = s_b_full.len() as i32 + 2; // strlen(s) + 2
    let mut tmp_b: Vec<u8> = Vec::with_capacity(s_b_full.len() + 3);
    tmp_b.push(b' ');
    tmp_b.extend_from_slice(&s_b_full[..noffs_u]);
    scs = 1 + noffs;
    ZLEMETACS.store(scs, Ordering::Relaxed);
    tmp_b.push(b'x');
    tmp_b.extend_from_slice(&s_b_full[noffs_u..]);
    let mut tmp = from_sb(&tmp_b);

    // c:1501-1640 — quote-stack head processing.
    let compqstack_s = snap(&COMPQSTACK);
    qttype = compqstack_s
        .chars()
        .next()
        .map(|c| c as i32)
        .unwrap_or(QT_NONE);
    let qstack2 = compqstack_s
        .chars()
        .nth(1)
        .map(|c| c as u32 != 0)
        .unwrap_or(false); // compqstack[1]
    if qttype == QT_BACKSLASH {
        // c:1503-1506
        remq = true;
        tmp = rembslash(&tmp);
    } else if qttype == QT_SINGLE {
        // c:1508-1514
        qa = if isset(RCQUOTES) { 1 } else { 3 };
        let mut t = tmp.clone();
        sq = remsquote(&mut t);
        tmp = t;
    } else if qttype == QT_DOUBLE {
        // c:1516-1543 — strip \\ and \" pairs, tracking zlemetacs/css/dq.
        let mut v = to_sb(&tmp);
        let mut j: i32 = 0;
        let mut pi = 0usize;
        let mut zcs = ZLEMETACS.load(Ordering::Relaxed);
        while pi < v.len() {
            let c = v[pi];
            let nxt = v.get(pi + 1).copied();
            if c == b'\\' && (nxt == Some(b'\\') || nxt == Some(b'"')) {
                dq += 1;
                v.remove(pi); // chuck(p): drop the backslash
                match v.get(pi).copied() {
                    Some(b'"') => zcs -= 1,
                    _ => {
                        if j > zcs {
                            zcs += 1;
                            css += 1;
                        }
                    }
                }
                if pi >= v.len() {
                    break; // if (!*p) break
                }
            }
            pi += 1;
            j += 1;
        }
        ZLEMETACS.store(zcs, Ordering::Relaxed);
        tmp = from_sb(&v);
    } else if qttype == QT_DOLLARS {
        // c:1613-1622 — string decode + dolq, with the GETKEY_UPDATE_OFFSET
        // cursor micro-adjustment. `j = zlemetacs` (c:1614); the decode
        // updates zlemetacs in place as pre-cursor escapes collapse; then
        // `css += zlemetacs - j` (c:1621) folds the delta into the word
        // offset. GETKEYS_DOLLARS_QUOTE carries the port's decode flags;
        // GETKEY_UPDATE_OFFSET enables the offset bookkeeping in
        // getkeystring_with.
        let j = ZLEMETACS.load(Ordering::Relaxed); // c:1614 — j = zlemetacs
        let mut zcs = j;
        let (dec, _consumed) = getkeystring_with(
            &tmp,
            (GETKEYS_DOLLARS_QUOTE | GETKEY_UPDATE_OFFSET) as u32,
            Some(&mut zcs),
        );
        ZLEMETACS.store(zcs, Ordering::Relaxed);
        let sl_new = to_sb(&dec).len() as i32;
        // c:1619 — dolq = tl - sl (bytes removed by $' quoting).
        dolq = tl0 - sl_new;
        // c:1621 — css += zlemetacs - j.
        css += zcs - j;
        tmp = dec;
    }
    let odq = dq; // c:1642

    // c:1643-1647 — push into lexer, set the working metaline.
    crate::ported::input::inpush(&dupstrspace(&tmp), 0, None);
    put(&ZLEMETALINE, tmp.clone());
    ZLEMETALL.store(tl0 - 1, Ordering::Relaxed); // tl - addedx
    crate::ported::hist::strinbeg(0);
    set_noaliases(true);

    // c:1650-1755 — the ctxtlex token loop.
    let mut ns_b: Vec<u8> = Vec::new();
    loop {
        ctxtlex();
        let mut tokv = tok();
        let mut ts_opt = tokstr();
        if tokv == LEXERR {
            // c:1654-1668 — odd active-quote count means unterminated
            // string; treat as STRING and drop a trailing space.
            match &ts_opt {
                None => break,
                Some(ts) => {
                    let j = ts.chars().filter(|&c| c == Snull || c == Dnull).count();
                    if j & 1 == 1 {
                        tokv = STRING_LEX;
                        set_tok(STRING_LEX);
                        if ts.ends_with(' ') {
                            let mut t = ts.clone();
                            t.pop();
                            set_tokstr(Some(t.clone()));
                            ts_opt = Some(t);
                        }
                    }
                }
            }
        }
        if tokv == ENDINPUT {
            break; // c:1670
        }
        let mut last_p: Option<usize> = None;
        if let Some(ts) = ts_opt.as_ref() {
            if !ts.is_empty() {
                // c:1673-1680 — Bnull accounting against dq.
                if dq != 0 {
                    let cs: Vec<char> = ts.chars().collect();
                    let mut k = 0usize;
                    while dq != 0 && k < cs.len() {
                        if cs[k] == Bnull {
                            dq -= 1;
                            if cs.get(k + 1) == Some(&'\\') {
                                dq -= 1;
                            }
                        }
                        k += 1;
                    }
                }
                // c:1681-1690 — single-quote lsq accounting.
                if qttype == QT_SINGLE {
                    lsq = 0;
                    for c in ts.chars() {
                        if sq != 0 && c == Snull {
                            sq -= qa;
                        }
                        if c == '\'' {
                            sq -= qa;
                            lsq += qa;
                        }
                    }
                } else {
                    lsq = 0;
                }
                foo.push(ts.clone()); // addlinknode(foo, p = ztrdup(tokstr))
                last_p = Some(foo.len() - 1);
            }
        }
        // c:1694-1705 — capture the cursor word once lexflags cleared.
        if !got && LEX_LEXFLAGS.get() == 0 {
            if let Some(cur_idx) = last_p {
                got = true;
                cur = cur_idx as i32;
                swb = WB.load(Ordering::Relaxed) - dq - sq - dolq;
                swe = WE.load(Ordering::Relaxed) - dq - sq - dolq;
                sqq = lsq;
                soffs = ZLEMETACS.load(Ordering::Relaxed) - swb - css;
                // chuck(p + soffs): drop the injected 'x' from the node.
                let mut wb_bytes = to_sb(&foo[cur_idx]);
                if soffs >= 0 && (soffs as usize) < wb_bytes.len() {
                    wb_bytes.remove(soffs as usize);
                }
                foo[cur_idx] = from_sb(&wb_bytes);
                ns_b = wb_bytes; // ns = dupstring(p)
            }
        }
        i += 1;
        if tokv == ENDINPUT || tokv == LEXERR {
            break; // c:1707 do-while
        }
    }

    // c:1709-1719 — tear down lexer state, restore positions.
    set_noaliases(ona);
    crate::ported::hist::strinend();
    crate::ported::input::inpop();
    crate::ported::utils::errflag
        .fetch_and(!crate::ported::utils::ERRFLAG_ERROR, Ordering::Relaxed);
    *crate::ported::utils::noerrs_lock().lock().unwrap() = ne;
    lexrestore(lex_saved); // zcontext_restore()
    WB.store(owb, Ordering::Relaxed);
    WE.store(owe, Ordering::Relaxed);
    put(&ZLEMETALINE, ol);
    zle_restore_positions();
    if cur < 0 || i < 1 {
        return 1; // c:1721
    }

    // c:1723-1733 — check_param dispatch with offs temporarily = soffs.
    let o_offs = OFFS.load(Ordering::Relaxed);
    OFFS.store(soffs, Ordering::Relaxed);
    if check_param(&from_sb(&ns_b), false, true).is_some() {
        for b in ns_b.iter_mut() {
            if *b == dnull_b {
                *b = b'"';
            } else if *b == snull_b {
                *b = b'\'';
            }
        }
    }
    OFFS.store(o_offs, Ordering::Relaxed);

    // c:1735 — ts = untokenize(dupstring(ns)).
    let ts_str = untokenize(&from_sb(&ns_b));
    let ts_b = to_sb(&ts_str);

    // c:1737-1772 — quote-form detection: instring / inbackt / autoq.
    let ns0 = ns_b.first().copied();
    let ns1 = ns_b.get(1).copied();
    let quote_open = ns0 == Some(snull_b)
        || ns0 == Some(dnull_b)
        || ((ns0 == Some(stringg_b) || ns0 == Some(qstring_b)) && ns1 == Some(snull_b));
    let mut ts_off = 0usize;
    if quote_open {
        let mut nsptr = 0usize; // C's nsptr offset into ns
        match ns0 {
            x if x == Some(snull_b) => INSTRING.store(QT_SINGLE, Ordering::Relaxed),
            x if x == Some(dnull_b) => INSTRING.store(QT_DOUBLE, Ordering::Relaxed),
            _ => {
                INSTRING.store(QT_DOLLARS, Ordering::Relaxed);
                nsptr += 1;
                ts_off += 1;
                swb += 1;
            }
        }
        INBACKT.store(0, Ordering::Relaxed);
        swb += 1;
        // c:1747 — if (nsptr[strlen(nsptr)-1] == *nsptr && nsptr[1]) swe--
        let ns_slice = &ns_b[nsptr.min(ns_b.len())..];
        if ns_slice.len() >= 2 && *ns_slice.last().unwrap() == ns_slice[0] {
            swe -= 1;
        }
        // c:1749-1753 — autoq from ts prefix.
        ts_off += 1; // ++tsptr
        let ts_prefix = from_sb(&ts_b[..ts_off.min(ts_b.len())]);
        let autoq_v = if qstack2 {
            String::new()
        } else {
            multiquote(&ts_prefix, 1)
        };
        put(&AUTOQ, autoq_v);
    } else {
        INSTRING.store(QT_NONE, Ordering::Relaxed);
        put(&AUTOQ, String::new());
    }

    // c:1774-1804 — the inull walk: drop null markers from ns, adjusting
    // swb/scs/soffs. `scs` is copied to a mutable walker `scs_w`.
    let mut scs_w = scs;
    {
        let mut pi = 0usize;
        let mut wi = swb;
        while pi < ns_b.len() {
            let c = ns_b[pi];
            if crate::ported::ztype_h::inull(c) {
                let next = ns_b.get(pi + 1).copied();
                let next_truthy = next.is_some();
                if wi < scs_w && c == bnull_b {
                    if next_truthy && remq {
                        swb -= 2;
                    }
                    if odq != 0 {
                        swb -= 1;
                        if next == Some(b'\\') {
                            swb -= 1;
                        }
                    }
                }
                if next_truthy || c != bnull_b {
                    if c == bnull_b {
                        if scs_w == wi + 1 {
                            scs_w += 1;
                            soffs += 1;
                        }
                    } else {
                        let cond = scs_w > wi;
                        wi -= 1; // C's post-decrement in `scs > i--`
                        if cond {
                            scs_w -= 1;
                        }
                    }
                } else if scs_w == swe {
                    scs_w -= 1;
                }
                ns_b.remove(pi); // chuck(p--); loop p++ revisits index pi
                wi += 1;
            } else {
                pi += 1;
                wi += 1;
            }
        }
    }

    // c:1806 — ns = ts (the untokenized copy, advanced past open quote).
    let mut ns_final_b = ts_b[ts_off.min(ts_b.len())..].to_vec();

    // c:1808-1813 — backslash-quoting length fixup.
    let instr = INSTRING.load(Ordering::Relaxed);
    let qstack_has_bs = compqstack_s.chars().any(|c| c as i32 == QT_BACKSLASH);
    if instr != QT_NONE && qstack_has_bs {
        let ns_now = from_sb(&ns_final_b);
        let rl = ns_final_b.len() as i32;
        let ql = to_sb(&multiquote(&ns_now, if qstack2 { 1 } else { 0 })).len() as i32;
        if ql > rl {
            swb -= ql - rl;
        }
    }

    // c:1826-1855 — split the reconstructed s into qp (prefix) / qs (suffix)
    // around the word, with the empirical swb-1-sqq+dq / swe-- offsets.
    let idx = swb - 1 - sqq + dq;
    let iu = idx.clamp(0, s_b_full.len() as i32) as usize;
    let s_prefix = from_sb(&s_b_full[..iu]);
    let mut qp = if qttype == QT_SINGLE {
        dupstring_wlen(&s_prefix, iu)
    } else {
        rembslash(&s_prefix)
    };
    if swe < swb {
        swe = swb;
    }
    swe -= 1;
    let sl_s = s_b_full.len() as i32;
    if swe > sl_s {
        swe = sl_s;
        if ns_final_b.len() as i32 > swe - swb + 1 {
            let newlen = (swe - swb + 1).max(0) as usize;
            ns_final_b.truncate(newlen);
        }
    }
    let swe_u = swe.clamp(0, s_b_full.len() as i32) as usize;
    let s_suffix = from_sb(&s_b_full[swe_u..]);
    let mut qs = if qttype == QT_SINGLE {
        s_suffix.clone()
    } else {
        rembslash(&s_suffix)
    };
    let sl_ns = ns_final_b.len() as i32;
    if soffs > sl_ns {
        soffs = sl_ns;
    }
    if qttype == QT_SINGLE {
        let mut a = qp;
        remsquote(&mut a);
        qp = a;
        let mut b = qs;
        remsquote(&mut b);
        qs = b;
    }

    // c:1857-1935 — publish the results.
    // c:1861-1868 — prepend the active quote char to compqstack.
    {
        let head = if instr == QT_NONE {
            QT_BACKSLASH
        } else {
            instr
        };
        let mut new_qstack = String::new();
        if let Some(hc) = char::from_u32(head as u32) {
            new_qstack.push(hc);
        }
        new_qstack.push_str(&compqstack_s);
        put(&COMPQSTACK, new_qstack);
        // !!! RUST-ONLY LINE — NO C COUNTERPART !!!
        // Same reason as the c:305-306 site above: in C the c:1854-1860
        // `compqstack = p` IS the `$compstate[all_quotes]` update, because
        // `compqstack_gsu` (complete.c:1299) has no storage of its own.
        set_compstate_str(
            "all_quotes",
            &crate::ported::zle::complete::get_compqstack(std::ptr::null_mut()),
        );
    }

    // c:1870-1892 — compquote / compquoting + comp_setunset.
    let mut set = (CP_QUOTE | CP_QUOTING) as i32;
    let mut unset = 0i32;
    let (cq, cqg) = if instr == QT_DOUBLE {
        ("\"", "double")
    } else if instr == QT_SINGLE {
        ("'", "single")
    } else if instr == QT_DOLLARS {
        ("$'", "dollars")
    } else {
        unset = set;
        set = 0;
        ("", "")
    };
    put(&COMPQUOTE, cq.to_string());
    put(&COMPQUOTING, cqg.to_string());
    crate::ported::zle::complete::comp_setunset(0, 0, set, unset);

    // c:1894-1907 — compprefix / compsuffix from ns around soffs.
    if !isset(COMPLETEINWORD) {
        put(&COMPPREFIX, untokenize(&from_sb(&ns_final_b)));
        put(&COMPSUFFIX, String::new());
    } else {
        let so = (soffs.max(0) as usize).min(ns_final_b.len());
        put(&COMPPREFIX, untokenize(&from_sb(&ns_final_b[..so])));
        put(&COMPSUFFIX, untokenize(&from_sb(&ns_final_b[so..])));
    }
    // c:1908-1910 — drop a dangling final backslash from compprefix.
    {
        let cp = snap(&COMPPREFIX);
        let cpb = cp.as_bytes();
        let n = cpb.len();
        if n > 1 && cpb[n - 1] == b'\\' && cpb[n - 2] != b'\\' && cpb[n - 2] != meta_b {
            let mut t = cp;
            t.pop();
            put(&COMPPREFIX, t);
        }
    }

    // c:1912-1925 — fold qp/qs into the quoted ignored prefix/suffix.
    let cqip = tricat(
        &snap(&COMPQIPREFIX),
        &snap(&COMPIPREFIX),
        &multiquote(&qp, 1),
    );
    put(&COMPQIPREFIX, cqip);
    let cqis = tricat(
        &multiquote(&qs, 1),
        &snap(&COMPISUFFIX),
        &snap(&COMPQISUFFIX),
    );
    put(&COMPQISUFFIX, cqis);
    put(&COMPIPREFIX, String::new());
    put(&COMPISUFFIX, String::new());

    // c:1926-1934 — rebuild compwords / compcurrent from foo.
    {
        let words: Vec<String> = foo.iter().map(|w| untokenize(w)).collect();
        let cnt = words.len() as i32;
        *COMPWORDS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap() = words;
        let mut compcur = cur + 1;
        if compcur > cnt {
            compcur = cnt;
        }
        COMPCURRENT.store(compcur, Ordering::Relaxed);
    }

    // zshrs bridge: in C every comp* global written above IS the shell
    // parameter (gsu-bound at complete.c:1235-1295 — one storage), so a
    // completer sees the re-split word list the instant `compset -q`
    // returns. zshrs's `$words` / `$CURRENT` / `$PREFIX` / … are plain
    // paramtab copies published once per `callcompfunc`, so without this
    // mirror they still describe the PRE-split line. `_trap` is
    //     if [[ CURRENT -eq 2 ]]; then compset -q; _normal; else …
    // and `_normal` re-reads `$words[1]` — which stayed `trap`, so it
    // dispatched `_trap` again: unbounded recursion until FUNCNEST, with
    // the error text landing in the user's buffer. Same mirror
    // `restrict_range` (complete.rs:1301-1307) already does for its own
    // COMPWORDS/COMPCURRENT edit.
    {
        let words = COMPWORDS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        setaparam("words", words);
        let _ =
            crate::ported::params::setiparam("CURRENT", COMPCURRENT.load(Ordering::Relaxed) as i64);
        for (param, global) in [
            ("PREFIX", &COMPPREFIX),
            ("SUFFIX", &COMPSUFFIX),
            ("IPREFIX", &COMPIPREFIX),
            ("ISUFFIX", &COMPISUFFIX),
            ("QIPREFIX", &COMPQIPREFIX),
            ("QISUFFIX", &COMPQISUFFIX),
            ("QUOTE", &COMPQUOTE),
            ("QUOTING", &COMPQUOTING),
        ] {
            // Same gsu-setfn bypass as the restore in complete.rs:
            // QIPREFIX/QISUFFIX carry PM_READONLY (c:1256-1257).
            crate::vm_helper::set_readonly_special(param, &snap(global));
        }
    }

    // c:1935-1937 — restore instring / inbackt, ret = 0.
    INSTRING.store(ois, Ordering::Relaxed);
    INBACKT.store(oib, Ordering::Relaxed);
    0
}

// Brace counters live in zle_tricky.c:114 — re-exported there. Local
// re-exports here so call sites stay short:
#[doc(hidden)]
// =====================================================================
// set_list_array — `Src/Zle/compcore.c:1947`.
// =====================================================================

/// Port of `static void set_list_array(char *name, LinkList l)` from
/// compcore.c:1947. Writes an array-typed parameter via the canonical
/// `setaparam` (params.c:3595).
pub fn set_list_array(name: &str, l: &[String]) {
    // c:1947
    let _ = setaparam(name, l.to_vec()); // c:1956
}

// =====================================================================
// get_user_var — `Src/Zle/compcore.c:1956`.
// =====================================================================

/// Port of `mod_export char **get_user_var(char *nam)` from
/// compcore.c:1956.
pub fn get_user_var(nam: Option<&str>) -> Option<Vec<String>> {
    // c:1956
    let nam = nam?; // c:1956
    if nam.starts_with('(') {
        // c:1960
        let mut arrlist: Vec<String> = Vec::new();
        let bytes = nam.as_bytes();
        let mut buf = Vec::<u8>::new();
        let mut notempty = false; // c:1963
        let mut brk = false;
        let mut i = 1; // c:1967
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' && i + 1 < bytes.len() {
                // c:1969
                buf.push(bytes[i + 1]); // c:1970
                notempty = true;
                i += 2;
                continue;
            }
            if b == b',' || b == b' ' || b == b'\t' || b == b'\n' || b == b')' {
                if b == b')' {
                    brk = true;
                } // c:1972
                if notempty {
                    // c:1974
                    let mut start = 0;
                    if !buf.is_empty() && buf[0] == b'\n' {
                        start = 1;
                    } // c:1977
                    let s = String::from_utf8_lossy(&buf[start..]).into_owned();
                    arrlist.push(s); // c:1979
                }
                buf.clear(); // c:1981
                notempty = false;
            } else {
                notempty = true; // c:1983
                buf.push(b);
                // c:1984-1985 — `if (*ptr == Meta) ptr++;`. The byte AFTER a
                // Meta escape is the real character XOR 0x20 and must never
                // be examined as a delimiter: a metafied `)` is `Meta 0x09`,
                // whose second byte is TAB, so without this skip `inblank`
                // split the list in the middle of a quoted element.
                if b == crate::ported::zsh_h::Meta as u8 && i + 1 < bytes.len() {
                    i += 1;
                    buf.push(bytes[i]);
                }
            }
            i += 1;
            if brk {
                break;
            } // c:1988
        }
        if !brk || arrlist.is_empty() {
            return None;
        } // c:1991
        Some(arrlist) // c:1996
    } else {
        // c:1999
        // c:2003 — `if ((arr = getaparam(nam)) || (arr = gethparam(nam)))
        //          arr = (incompfunc ? arrdup(arr) : arr);
        //          else if ((val = getsparam(nam))) { arr = {val, NULL}; }`
        //
        // Route through the canonical accessors, exactly the three C calls in
        // that order. The previous port read `pm.u_arr` / `pm.u_str` straight
        // out of the paramtab node, which is NOT where every parameter keeps
        // its value:
        //
        //   * assoc arrays live in `paramtab_hashed_storage`, so `gethparam`
        //     was effectively unimplemented here — the whole middle arm of the
        //     C condition was missing;
        //   * a plain array whose node carries its value anywhere other than a
        //     populated `u_arr` (special/tied/magic params, and arrays
        //     materialised by a Rust compsys port rather than by a shell
        //     assignment) read back as None.
        //
        // A None here is reported by `cd_init` as `compdescribe: invalid
        // argument: <name>` (computil.rs:1052, c:516) and aborts the whole
        // description, so `_describe -t global-aliases 'global alias' ARR`
        // printed that error onto the command line and then a second
        // `compdescribe: no parsed state` from the following `-g` call.
        // Only reachable when `list-grouped` is FALSE: the grouped path in
        // `_describe` (sh:82) first copies each array into a local via
        // `eval local _a_N=( "${ARR[@]}" )`, and those locals DO land in
        // `u_arr`, which is why the same completion worked with the style on.
        queue_signals();
        let result = crate::ported::params::getaparam(nam)
            .or_else(|| crate::ported::params::gethparam(nam)) // c:2003
            .or_else(|| crate::ported::params::getsparam(nam).map(|v| vec![v])); // c:2005-2008
        unqueue_signals(); // c:2022
        result
    }
}

// =====================================================================
// get_data_arr — `Src/Zle/compcore.c:2022`.
// =====================================================================

/// Direct port of `static char **get_data_arr(char *name, int keys)`
/// from `Src/Zle/compcore.c:2022`:
/// ```c
/// queue_signals();
/// if (!(v = fetchvalue(&vbuf, &name, 1,
///                      (keys ? SCANPM_WANTKEYS : SCANPM_WANTVALS) |
///                      SCANPM_MATCHMANY)))
///     ret = NULL;
/// else
///     ret = getarrvalue(v);
/// unqueue_signals();
/// ```
/// A SUBSCRIPTED name goes straight through that call: `fetchvalue`
/// consumes the identifier (`Src/params.c:2196`) and hands the `[…]`
/// remainder to `getindex` (c:2289), which sets the SCANPM_MATCH* bits and
/// runs `getvaluearr` → `paramvalarr` → `scanparamvals`.
///
/// A BARE name keeps this port's accessor path (`getaparam` / `gethparam` /
/// `gethkparam`) rather than `fetchvalue`, because those are where the
/// zsh/parameter magic hashes and arrays (`commands`, `builtins`,
/// `reswords`, …) resolve their module scanfn/getfn — the paramtab node
/// holds no value for them, so a `fetchvalue` + `arrgetfn` read would come
/// back empty and `compadd -k commands` would add the literal word
/// "commands".
pub fn get_data_arr(name: &str, keys: bool) -> Option<Vec<String>> {
    // c:2022

    queue_signals(); // c:2028

    if name.contains('[') {
        // c:2030-2033 — `fetchvalue(&vbuf, &name, 1, (keys ?
        //   SCANPM_WANTKEYS : SCANPM_WANTVALS) | SCANPM_MATCHMANY)`.
        //
        // Both scan flags are FIXED at this call site: WANTKEYS vs WANTVALS
        // comes from `keys`, and SCANPM_MATCHMANY is ALWAYS set — so
        // `scanparamvals`' single-match early-out (`Src/params.c:648-650`)
        // never fires and EVERY match is returned, whatever the case of the
        // subscript's search letter. `Src/params.c:1719-1722` picks the
        // match TARGET (`i`/`I` → SCANPM_MATCHKEY, `k`/`K` →
        // SCANPM_KEYMATCH, else SCANPM_MATCHVAL) while
        // `scanparamvals` (c:665-681) picks the OUTPUT purely from
        // WANTKEYS/WANTVALS. That is why `compadd -k 'styles[(R)…]'`
        // (Zsh/Command/_zstyle:363) yields the KEYS of the entries whose
        // VALUE matched.
        let sf = (if keys {
            crate::ported::zsh_h::SCANPM_WANTKEYS
        } else {
            crate::ported::zsh_h::SCANPM_WANTVALS
        }) | crate::ported::zsh_h::SCANPM_MATCHMANY;
        let mut vbuf = crate::ported::zsh_h::value {
            pm: None,
            arr: Vec::new(),
            scanflags: 0,
            valflags: 0,
            start: 0,
            end: -1,
        };
        let mut cursor: &str = name;
        let result =
            match crate::ported::params::fetchvalue(Some(&mut vbuf), &mut cursor, 1, sf as i32) {
                None => None, // c:2034-2035
                Some(v) => {
                    // c:2037 — `ret = getarrvalue(v)`. zshrs's `getarrvalue`
                    // takes the resolved array plus the slice bounds rather
                    // than the Value, so spell out the C body's steps here.
                    //
                    // c:2554-2555 — `else if (IS_UNSET_VALUE(v)) return
                    // arrdup(&nular[1]);` (empty). IS_UNSET_VALUE (c:472-474)
                    // is `!pm || (pm->flags & PM_UNSET) || !*pm->nam`, which is
                    // how a missing assoc KEY reports nothing: getindex rebinds
                    // `v->pm` to a `PM_SCALAR|PM_UNSET` element on a miss
                    // (c:1588).
                    let unset = v.pm.as_ref().map_or(true, |p| {
                        (p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET) != 0
                            || p.node.nam.is_empty()
                    });
                    if unset {
                        Some(Vec::new()) // c:2555
                    } else {
                        let arr = crate::ported::params::getvaluearr(Some(&mut *v)); // c:2564
                        if v.start == 0 && v.end == -1 {
                            // c:2565-2566 — whole-value read: hand the array
                            // straight back. A PRESENT assoc key lands here
                            // with an empty `arr` (getvaluearr has no
                            // PM_SCALAR arm, c:719), which is C's "a scalar
                            // element is not a match list" answer.
                            Some(arr)
                        } else {
                            Some(crate::ported::params::getarrvalue(
                                &arr,
                                v.start as i64,
                                v.end as i64,
                            ))
                        }
                    }
                }
            };
        unqueue_signals(); // c:2039
        return result;
    }

    // Route through the same param accessors `${(k)name}` / `${(v)name}`
    // / `${name}` use so SPECIAL magic hashes (`commands`, `builtins`,
    // `functions`, `aliases`, `reswords`, …) resolve via their module
    // scanfns — the raw `paramtab_hashed_storage` map is empty for those,
    // which is why `compadd -k commands` previously added the literal
    // word "commands" instead of every command name.
    let result = if keys {
        // SCANPM_WANTKEYS — assoc keys (gethkparam handles special hashes,
        // returning Some incl. Some(empty)). But `compadd -k` on a REGULAR
        // array (e.g. `_setopt`'s `local -a onopts`) must add its ELEMENTS:
        // C's `fetchvalue(SCANPM_WANTKEYS)` ignores WANTKEYS for a non-hash and
        // returns the value array. gethkparam returns None for a non-hash, so
        // fall back to the elements — without this, `setopt`/`unsetopt <tab>`
        // (and any `compadd -k <plain-array>`) produced ZERO matches.
        match crate::ported::params::gethkparam(name) {
            Some(k) => Some(k),
            None => crate::ported::params::getaparam(name),
        }
    } else {
        // SCANPM_WANTVALS — plain-array elements, else assoc values.
        crate::ported::params::getaparam(name)
            .filter(|v| !v.is_empty())
            .or_else(|| crate::ported::params::gethparam(name))
    };

    unqueue_signals(); // c:2041
    result
}

// =====================================================================
// addmatch — `Src/Zle/compcore.c:2041`.
// =====================================================================

/// Port of `static void addmatch(char *str, int flags, char ***dispp,
///                                int line)` from compcore.c:2041.
pub fn addmatch(str: &str, flags: i32, disp: Option<&str>, line: bool) {
    // c:2041
    let mut cm = Cmatch::default(); // c:2041
    cm.str = Some(str.to_string()); // c:2047
                                    // c:2049-2051 — inline read of `complist` parameter, parse `packed`/
                                    // `rows` substrings into CMF_PACKED/CMF_ROWS flag bits.
    let complist_extra = {
        let s = COMPLIST
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let packed = if s.contains("packed") { CMF_PACKED } else { 0 }; // c:2050
        let rows = if s.contains("rows") { CMF_ROWS } else { 0 }; // c:2051
        if s.is_empty() {
            0
        } else {
            packed | rows
        }
    };
    cm.flags = flags | complist_extra; // c:2048
    if let Some(d) = disp {
        // c:2052
        cm.disp = Some(d.to_string()); // c:2056
    } else if line {
        // c:2057
        cm.disp = Some(String::new()); // c:2058
        cm.flags |= CMF_DISPLINE; // c:2059
    }
    mnum.fetch_add(1, Ordering::Relaxed); // c:2060
                                          // c:2061 — `ainfo->count++`. Missing here while the sibling
                                          // increment in `add_match_data` (c:3006) was present, so every
                                          // match added through THIS path (`compadd -x`/`-X` dummies, the
                                          // CAF_ALL `<all>` placeholder) was invisible to `ainfo.count` —
                                          // the counter `do_ambiguous`/`permmatches` use to decide whether
                                          // any match exists in the group.
    if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(a) = g.as_mut() {
            a.count += 1; // c:2061
        }
    }
    {
        let cell = curexpl.get_or_init(|| Mutex::new(None)); // c:2062
        if let Ok(mut g) = cell.lock() {
            if let Some(e) = g.as_mut() {
                e.count += 1;
            }
        }
    }
    let mcell = crate::comp_match_handles::matches_arc(); // c:2066
    if let Ok(mut g) = mcell.lock() {
        g.push(cm);
    }
    newmatches.store(1, Ordering::Relaxed); // c:2068
    {
        let cell = mgroup.get_or_init(|| Mutex::new(None)); // c:2069
        if let Ok(g) = cell.lock() {
            if let Some(grp) = g.as_ref() {
                // c:2068 `mgroup->new = 1` — `new_` is the shared flag the
                // `amatches` entry sees too (comp_h.rs Cmgroup::new_).
                grp.new_.store(1, Ordering::Relaxed);
            }
        }
    }
}

// =====================================================================
// addmatches — `Src/Zle/compcore.c:2080`.
// =====================================================================

/// Direct port of `int addmatches(Cadata dat, char **argv)` from
/// compcore.c:2080 — the workhorse called from every `compadd`
/// invocation. Walks `argv`, runs the matcher chain against each
/// candidate, builds the Cline chain via `add_match_data`, and
/// appends accepted matches to the current group.
///
/// Real-bodied across all major phases: prologue (group selection
/// c:2105-2118, brace-state snapshot c:2129-2132, instring/inbackt
/// save c:2148-2179, `*argv` empty short-circuit c:2127), mstack
/// push c:2210-2222, aign/pign suffix-ignore + Patprog filters
/// c:2223-2246, disp array c:2247-2250, lipre/lisuf/lpre/lsuf
/// assembly c:2253-2300, per-candidate match loop with comp_match
/// dispatch + add_match_data emit + apar/opar writeback
/// c:2482-2601, apar/opar setaparam c:2602-2605, exp addexpl
/// c:2610, hasallmatch CAF_ALL placeholder c:2612-2614, dummy
/// entries c:2616-2617.
pub fn addmatches(
    dat: &mut Cadata, // c:2080
    argv: &[String],
) -> i32 {
    let _nm = mnum.load(Ordering::Relaxed); // c:2089 nm
                                            // c:2089-2090 — `ohp = haspattern`, `ois = instring`, `oib = inbackt`,
                                            // and c:2084 `oaq = autoq`. All four are restored at c:2624-2633; the
                                            // port saved none of them.
    let ohp = haspattern.load(Ordering::Relaxed); // c:2089
    let ois = INSTRING.load(Ordering::Relaxed); // c:2090
    let oib = INBACKT.load(Ordering::Relaxed); // c:2090
    let oaq = AUTOQ
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default(); // c:2084
                              // c:2085 — `char *oqp = qipre, *oqs = qisuf`, restored at c:2629-2630.
    let oqp = crate::ported::zle::zle_tricky::qipre_get(); // c:2085
    let oqs = crate::ported::zle::zle_tricky::qisuf_get(); // c:2085

    // c:2093 — `Cmlist oms = mstack;`, restored at c:2622 `mstack = oms;`.
    // The `-M` matcher a compadd carries is pushed onto `mstack` (c:2212) for
    // the duration of THAT call only. This port pushed but never restored, so
    // the matcher leaked into every later compadd of the same completion: once
    // `_describe` added an option list with `-M 'r:|[_-]=* r:|=*'`, a following
    // bare `compadd -k commands` matched the line prefix `-` against every
    // command name. `hash -<TAB>` offered 1152 candidates where zsh offers the
    // 6 options; the same leak inflates any completion that mixes a matcher-
    // carrying compadd with a later plain one.
    let oms_saved: Option<Box<Cmlist>> = mstack
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|mut g| g.take());
    if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
        *g = oms_saved.clone();
    }
    // Restores `mstack` on EVERY exit from this function (C restores at the
    // single c:2622 return; the port has several early returns).
    struct MstackRestore(Option<Box<Cmlist>>);
    impl Drop for MstackRestore {
        fn drop(&mut self) {
            if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
                *g = self.0.take();
            }
        }
    }
    let _mstack_guard = MstackRestore(oms_saved);

    // c:2049 — C's `complist` global is GSU-backed by `$compstate[list]`, so a
    // completer's `compstate[list]="... packed"` write (e.g. `_describe`
    // setting the grouped/packed layout) is visible to `addmatch` via `complist`.
    // The Rust port keeps COMPLIST as a separate global set only from the global
    // at completion init (c:740), so param writes never reached it — matches
    // added by grouped `_describe`/`_arguments` never got CMF_PACKED, so their
    // group lost CGF_PACKED and calclist skipped the per-column-width pass,
    // collapsing the name/description columns into one uniform column. Sync the
    // param → global here so `packed`/`rows` reach CMF_PACKED/CMF_ROWS.
    if let Some(cl) = get_compstate_str("list") {
        if let Ok(mut g) = COMPLIST.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = cl;
        }
    }

    if dat.dummies >= 0 {
        // c:2106
        dat.aflags = (dat.aflags | CAF_NOSORT | CAF_UNIQCON) & !CAF_UNIQALL; // c:2107-2108
    }

    let gflags = (if (dat.aflags & CAF_NOSORT) != 0 {
        CGF_NOSORT
    } else {
        0
    }) | (if (dat.aflags & CAF_MATSORT) != 0 {
        CGF_MATSORT
    } else {
        0
    }) | (if (dat.aflags & CAF_NUMSORT) != 0 {
        CGF_NUMSORT
    } else {
        0
    }) | (if (dat.aflags & CAF_REVSORT) != 0 {
        CGF_REVSORT
    } else {
        0
    }) | (if (dat.aflags & CAF_UNIQALL) != 0 {
        CGF_UNIQALL
    } else {
        0
    }) | (if (dat.aflags & CAF_UNIQCON) != 0 {
        CGF_UNIQCON
    } else {
        0
    });

    tracing::debug!(target: "compsys_args", group = dat.group.as_deref().unwrap_or("<none>"), exp = dat.exp.as_deref().unwrap_or("<none>"), nargs = argv.len(), doadd = dat.dpar.is_empty(), "addmatches group");
    if let Some(g) = dat.group.as_deref() {
        // c:2115
        endcmgroup(None); // c:2116
        begcmgroup(Some(g), gflags); // c:2117
    } else {
        endcmgroup(None); // c:2119
        begcmgroup(Some("default"), 0); // c:2120
    }

    if dat.mesg.is_some() || dat.exp.is_some() {
        // c:2122
        let mut e = Cexpl::default(); // c:2123
        e.always = if dat.mesg.is_some() { 1 } else { 0 }; // c:2124
        e.count = 0;
        e.fcount = 0; // c:2125
        e.str = Some(
            dat.mesg
                .clone() // c:2126
                .or_else(|| dat.exp.clone())
                .unwrap_or_default(),
        );
        if let Ok(mut g) = curexpl.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(e);
        }
        if dat.mesg.is_some() && dat.dpar.is_empty() && dat.opar.is_none() && dat.apar.is_none() {
            // c:2129
            addexpl(true); // c:2130
        }
    } else if let Ok(mut g) = curexpl.get_or_init(|| Mutex::new(None)).lock() {
        *g = None; // c:2133
    }

    // c:2138 — empty-argv early return.
    if argv.is_empty() && dat.dummies == 0 && (dat.aflags & CAF_ALL) == 0 {
        return 1; // c:2139
    }

    // c:2132-2135 — `for (bp = brbeg; bp; bp = bp->next)
    //                    bp->curpos = ((dat->aflags & CAF_QUOTE) ? bp->pos
    //                                                            : bp->qpos);`
    // and the same for `brend`. Only the CAF_QUOTE test existed here (as an
    // unused local); the snapshot itself was never performed, so `curpos`
    // stayed 0 for every brace and `add_match_data`'s brpl/brsl (c:2979) had
    // nothing to read.
    {
        let quote_mode = (dat.aflags & CAF_QUOTE) != 0; // c:2133
        for chain in [&BRBEG, &BREND] {
            if let Ok(mut g) = chain.get_or_init(|| Mutex::new(None)).lock() {
                let mut cur = g.as_deref_mut();
                while let Some(n) = cur {
                    n.curpos = if quote_mode { n.pos } else { n.qpos }; // c:2133/2135
                    cur = n.next.as_deref_mut();
                }
            }
        }
    }

    if (dat.flags & 0x0008/*CMF_ISPAR*/) != 0 {
        // c:2148
        dat.flags |= parflags.load(Ordering::Relaxed); // c:2149
    }

    let qc = compquote_first(); // c:2139
    if let Some(q) = qc {
        // c:2139 — `if (compquote && (qc = *compquote))`
        match q {
            '`' => {
                instring_set(QT_NONE);
                inbackt_set(0);
                autoq_set(""); // c:2140-2146
            }
            _ => {
                match q {
                    '\'' => instring_set(QT_SINGLE), // c:2149-2151
                    '"' => instring_set(QT_DOUBLE),  // c:2153-2155
                    '$' => instring_set(QT_DOLLARS), // c:2157-2159
                    _ => {}
                }
                // c:2162-2163 — `inbackt = 0;
                //   autoq = multiquote(*compquote == '$' ? compquote+1 : compquote, 1);`
                // Both lines were missing: `autoq` kept the PREVIOUS
                // completion's quote, which is what `do_single`
                // (compresult.c) re-closes the inserted word with.
                inbackt_set(0); // c:2162
                let cq = crate::ported::zle::complete::COMPQUOTE
                    .get_or_init(|| Mutex::new(String::new()))
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                let cq = if cq.starts_with('$') {
                    &cq[1..]
                } else {
                    &cq[..]
                };
                autoq_set(&multiquote(cq, 1)); // c:2163
            }
        }
    } else {
        instring_set(QT_NONE);
        inbackt_set(0);
        autoq_set(""); // c:2166-2168
    }
    // c:2170-2171 — `qipre = ztrdup(compqiprefix ? compqiprefix : "");
    //                qisuf = ztrdup(compqisuffix ? compqisuffix : "");`
    // `compqiprefix`/`compqisuffix` ARE `$QIPREFIX`/`$QISUFFIX` (complete.c:
    // 1266-1267), so a completer that ran `compset -q` (or `compset -P`)
    // hands its edited value back to the match builder here. Without this,
    // `add_match_data` saw whatever `get_comp_string` had left.
    for (global, param) in [
        (&crate::ported::zle::zle_tricky::QIPRE, "QIPREFIX"), // c:2170
        (&crate::ported::zle::zle_tricky::QISUF, "QISUFFIX"), // c:2171
    ] {
        let v = crate::ported::params::getsparam(param).unwrap_or_default();
        if let Ok(mut g) = global.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = v;
        }
    }

    // c:2173 — `useexact = (compexact && !strcmp(compexact, "accept"))`.
    //
    // C's `compexact` global IS `$compstate[exact]` (gsu-bound at
    // complete.c:1281). The port read `getsparam("compexact")` — a shell
    // parameter of that literal name, which nothing ever sets — so this line
    // unconditionally CLEARED `useexact` on every compadd, defeating
    // REC_EXACT and any completer that set `compstate[exact]=accept`.
    let exact_str = get_compstate_str("exact").unwrap_or_default();
    useexact.store(if exact_str == "accept" { 1 } else { 0 }, Ordering::Relaxed);

    // c:2170-2175 —
    //     if ((doadd = (!dat->apar && !dat->opar && !dat->dpar))) {
    //         if (dat->aflags & CAF_MATCH)
    //             hasmatched = 1;
    //         else
    //             hasunmatched = 1;
    //     }
    // `doadd` says this compadd puts matches on the completion list rather
    // than only filling `-A`/`-O`/`-D` arrays; only those calls tell the
    // result stage anything about the line. CAF_MATCH is `compadd` WITHOUT
    // `-U`, i.e. the candidate was matched against `$PREFIX`/`$SUFFIX`, so
    // the cline parts carry meaningful line/word anchors — that is exactly
    // the condition `cut_cline` (compresult.c:57) tests before it is willing
    // to trim the unambiguous string, and the condition `do_ambiguous`
    // (compresult.c:794) tests before restoring the typed word.
    //
    // The port set `hasmatched` only from the compctl path (compctl.c:3292),
    // never from compadd, so every compsys completion left both flags 0:
    // `cut_cline` took the `!hasmatched` "keep everything" branch forever and
    // `do_ambiguous`'s `!hasunmatched` guard was always satisfied. Measured on
    // the parity corpus under the `full` zstyle fixture: `cut_cline` reads
    // `hasmatched=0` without these lines and `hasmatched=1` with them, on 22
    // of the 31 corpus buffers that reach it (`df -` also reaches it with
    // `hasunmatched=1`, so both arms of the `if/else` fire).
    //
    // Placement follows C: this is c:2169-2175, ahead of the mstack push
    // (c:2200) and well ahead of the `*argv = NULL` prefix-mismatch bail at
    // c:2335. The port hoists parts of c:2280-2454 above the `doadd` binding
    // at the candidate loop and returns early on that bail, so setting the
    // flags there would silently skip the calls C still counts.
    let doadd = dat.apar.is_none() && dat.opar.is_none() && dat.dpar.is_empty(); // c:2170
    if doadd {
        if (dat.aflags & CAF_MATCH) != 0 {
            hasmatched.store(1, Ordering::Relaxed); // c:2172
        } else {
            hasunmatched.store(1, Ordering::Relaxed); // c:2174
        }
    }

    // c:2210-2222 — push dat.match onto mstack (the matcher chain
    // queried by match_str during candidate evaluation).
    //
    // c:2094 `Cmlist oms = mstack` / c:2623 `mstack = oms` — the push is
    // scoped to THIS addmatches call: C stack-allocates the link (`struct
    // cmlist mst`) and restores the saved head on the way out. The port
    // pushed but never restored, so a `compadd -M` matcher stayed live for
    // every later compadd of the same completion. `ssh -<TAB>` showed it:
    // _arguments adds its options with `-M 'r:|[_-]=* r:|=*'`, that spec
    // survived onto _hosts' compadd, and match_str (c:593 walks the whole
    // mstack) then let the typed `-` match any host CONTAINING a `-` —
    // `ec2-23-20-9-23.compute-1.amazonaws.com` was offered where zsh
    // offers options only. `MstackPop` restores on every exit path.
    struct MstackPop(bool);
    impl Drop for MstackPop {
        fn drop(&mut self) {
            if !self.0 {
                return;
            }
            if let Ok(mut mst) = mstack.get_or_init(|| Mutex::new(None)).lock() {
                // C restores the saved head; since this frame pushed exactly
                // one link, dropping it is the same list.
                *mst = mst.take().and_then(|link| link.next);
            }
        }
    }
    let _mstack_pop = MstackPop(dat.match_.is_some());
    if let Some(ref m) = dat.match_ {
        // c:2210
        if let Ok(mut mst) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            // C: mst.next = mstack; mst.matcher = dat->match; mstack = &mst.
            let new_link = Box::new(Cmlist {
                next: mst.take(),
                matcher: m.clone(),
                str: String::new(),
            });
            *mst = Some(new_link);
        }
        // c:2215 — add_bmatchers(dat->match).
        crate::ported::zle::compmatch::add_bmatchers(Some(m));
        // c:2217 — addlinknode(matchers, dat->match).
        if let Ok(mut g) = matchers.get_or_init(|| Mutex::new(Vec::new())).lock() {
            g.push(m.clone());
        }
    }
    // c:2220-2221 — `if (mnum && (mstack || bmatchers)) update_bmatchers();`
    // prunes bmatchers of any matcher no longer on mstack; without it the
    // restored stack and the bmatchers list drift apart.
    if mnum.load(Ordering::Relaxed) != 0 {
        let live = mstack
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
            || bmatchers
                .get_or_init(|| Mutex::new(None))
                .lock()
                .map(|g| g.is_some())
                .unwrap_or(false);
        if live {
            crate::ported::zle::compmatch::update_bmatchers();
        }
    }

    // c:2223-2246 — get suffixes to ignore from dat.ign param.
    let (aign, pign) = if let Some(ign_name) = dat.ign.as_deref() {
        // c:2224
        let aign_raw = get_user_var(Some(ign_name)).unwrap_or_default();
        let mut literal_suffixes: Vec<String> = Vec::new();
        let mut pat_progs: Vec<crate::ported::pattern::Patprog> = Vec::new();
        for entry in aign_raw {
            // c:2231-2232 — `tokenize(tmp); remnulargs(tmp);` — fignore
            // entries are param values (untokenized text); C tokenizes
            // each BEFORE the token checks below and the patcompile.
            let mut tmp = entry.clone();
            crate::ported::glob::tokenize(&mut tmp); // c:2231
            crate::ported::glob::remnulargs(&mut tmp); // c:2232
                                                       // c:2233-2236 — `(tmp[0] == Quest && tmp[1] == Star) ||
                                                       // (tmp[1] == Quest && tmp[0] == Star)` token short-circuit:
                                                       // trailing literal suffix.
            let tch: Vec<char> = tmp.chars().collect();
            let suffix: String = tch.iter().skip(2).collect();
            let star_prefix = tch.len() >= 3
                && ((tch[0] == crate::ported::zsh_h::Quest
                    && tch[1] == crate::ported::zsh_h::Star)
                    || (tch[1] == crate::ported::zsh_h::Quest
                        && tch[0] == crate::ported::zsh_h::Star))
                && !crate::ported::pattern::haswilds(&suffix);
            if star_prefix {
                // c:2236 — `untokenize(*sp++ = tmp + 2);`
                literal_suffixes.push(crate::ported::lex::untokenize(&suffix));
            } else if let Some(prog) =
                crate::ported::pattern::patcompile(&tmp, 0, None::<&mut String>)
            {
                pat_progs.push(prog);
            }
        }
        (literal_suffixes, pat_progs)
    } else {
        (Vec::new(), Vec::new())
    };

    // c:2247-2250 — get display strings.
    let disp_arr: Vec<String> = if let Some(ref d) = dat.disp {
        get_user_var(Some(d.as_str())).unwrap_or_default()
    } else {
        Vec::new()
    };

    // zshrs bridge: in C the `compprefix`/`compsuffix`/`compiprefix`/
    // `compisuffix` globals ARE `$PREFIX`/`$SUFFIX`/`$IPREFIX`/`$ISUFFIX`
    // (the compparams are gsu-bound to them). The Rust compparams carry
    // no gsu binding (`complete.rs` `gsu: 0`), so the compsys completers'
    // writes to `$PREFIX` land in the param table while `compadd` reads
    // the globals — leaving `lpre` empty and every candidate matching.
    // During a live completion (`INCOMPFUNC`), refresh the globals from
    // the params so `comp_match` filters against the prefix the completer
    // actually set. Gated on INCOMPFUNC so direct-call unit tests that
    // seed the globals aren't clobbered.
    if INCOMPFUNC.load(Ordering::Relaxed) != 0 {
        for (param, global) in [
            ("PREFIX", &COMPPREFIX),
            ("SUFFIX", &COMPSUFFIX),
            ("IPREFIX", &COMPIPREFIX),
            ("ISUFFIX", &crate::ported::zle::complete::COMPISUFFIX),
        ] {
            if let Some(v) = getsparam(param) {
                if let Ok(mut g) = global.get_or_init(|| Mutex::new(String::new())).lock() {
                    *g = v;
                }
            }
        }
    }

    // c:2253-2300 — CAF_MATCH lipre/lisuf/lpre/lsuf assembly.
    let compiprefix_s = COMPIPREFIX
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let compisuffix_s = crate::ported::zle::complete::COMPISUFFIX
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let compprefix_s = COMPPREFIX
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let compsuffix_s = COMPSUFFIX
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let lipre = compiprefix_s.clone();
    let lisuf = compisuffix_s.clone();
    let mut lpre = compprefix_s.clone();
    let mut lsuf = compsuffix_s.clone();

    // c:2252-2254 — `if (llpl + strlen(compqiprefix) + strlen(lipre) != origlpre
    //                 || llsl + strlen(compqisuffix) + strlen(lisuf) != origlsuf)
    //                    lenchanged = 1;`
    // The completer moved the PREFIX/SUFFIX split since the widget was entered
    // (`compset -P/-S`, `_approximate`, `_prefix`). `do_ambiguous`
    // (compresult.c:794) uses this to decide NOT to put the old word back when
    // the unambiguous string comes out shorter than the word on the line.
    // Never assigned before, so the flag was stuck at 0.
    if (dat.aflags & CAF_MATCH) != 0 {
        let qip = getsparam("QIPREFIX").unwrap_or_default();
        let qis = getsparam("QISUFFIX").unwrap_or_default();
        if lpre.len() + qip.len() + lipre.len() != origlpre.load(Ordering::Relaxed) as usize
            || lsuf.len() + qis.len() + lisuf.len() != origlsuf.load(Ordering::Relaxed) as usize
        {
            lenchanged.store(1, Ordering::Relaxed); // c:2254
        }
    }

    // c:2266-2276 — "Test if there is an existing -P prefix":
    //     if (dat->pre && *dat->pre) {
    //         int prefix_length = pfxlen(dat->pre, lpre);
    //         if (dat->pre[prefix_length] == '\0' ||
    //             lpre[prefix_length] == '\0') {
    //             /* $compadd_args[-P] is a prefix of ${PREFIX}, or
    //              * vice-versa. */
    //             llpl -= prefix_length;
    //             lpre += prefix_length;
    //         }
    //     }
    // `compadd -P PRE` puts PRE on the line AHEAD of every candidate, so —
    // exactly like the `-p PPRE` strip below — it has to come off `lpre`
    // before comp_match compares the candidates against it. `dat.pre` stays
    // in the add_match_data call further down; that is what re-inserts it.
    // This step was missing entirely, so `_zstyle`'s
    //     compadd -P : -qS : chpwd completion vcs_info zftp zle
    // matched NOTHING the moment $PREFIX was ":" — comp_match(":", "",
    // "chpwd") rejects every candidate — which is the whole `zstyle <TAB>`
    // divergence. C compares C strings byte-wise, so the shared-prefix
    // length is a BYTE count here (zle_tricky::pfxlen counts chars, which
    // would mis-slice a multibyte prefix).
    if (dat.aflags & CAF_MATCH) != 0 {
        // c:2267
        if let Some(pre) = dat.pre.as_deref() {
            if !pre.is_empty() {
                // c:2268 — `pfxlen(dat->pre, lpre)`, in bytes.
                let prefix_length: usize = pre
                    .chars()
                    .zip(lpre.chars())
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a.len_utf8())
                    .sum();
                // c:2269-2270 — one of the two ran out at the split point,
                // i.e. one is a prefix of the other.
                if prefix_length == pre.len() || prefix_length == lpre.len() {
                    lpre = lpre[prefix_length..].to_string(); // c:2273-2274
                }
            }
        }
    }

    // c:2084 — `char *oppre = dat->ppre`: the UNQUOTED path prefix as it
    // arrived, captured before the c:2288 quoting rewrites `dat.ppre`. Used
    // as the `prpre` default at c:2436.
    let oppre: Option<String> = dat.ppre.clone(); // c:2084

    // c:2314-2340 — strip the path-prefix (`compadd -p PPRE`) from `lpre`
    // before matching each candidate. PPRE is already on the command line
    // ahead of the match, so comp_match must compare the candidate against
    // `lpre` with PPRE removed (candidate `sub` vs `su`, not vs the full
    // `/tmp/ptree/su`). Try the active matcher first (`match_str`, part=1),
    // else strip literally. Without this every `cmd /a/b/pre<TAB>` produced
    // no matches — the resolved candidate never matched the full-path lpre.
    let mut bcp: i32 = 0;
    let mut ppre_nomatch = false;
    if (dat.aflags & CAF_MATCH) != 0 {
        // ppre is quoted below (c:2288) in C before this block; do the same
        // ordering here so lengths line up with the on-line text.
        let ppre_q: Option<String> = dat.ppre.as_deref().map(|existing| {
            if (dat.flags & 0x0001/*CMF_FILE*/) != 0 {
                tildequote(existing, if (dat.aflags & CAF_QUOTE) != 0 { 0 } else { 1 })
            } else {
                multiquote(existing, if (dat.aflags & CAF_QUOTE) != 0 { 0 } else { 1 })
            }
        });
        if let Some(ppre) = ppre_q.as_deref() {
            if !ppre.is_empty() {
                // NOTE: `dat.ppre` is left UNquoted here — the canonical
                // quoting happens in the c:2288 block below. `ppre_q` is a
                // local, same-quoting copy used only for the length math.
                let lpl = ppre.len();
                // c:2314 — `ml = match_str(lpre, ppre, …, part=1)`. Reset the
                // match accumulators first so this probe doesn't leak into the
                // per-candidate comp_match (the Rust threads the match Cline
                // through comp_match's own output, and instmatch re-inserts
                // dat.ppre from the Cmatch, so the probe's `pline` is unused).
                crate::ported::zle::compmatch::start_match();
                let ml = crate::ported::zle::compmatch::match_str(
                    lpre.as_bytes(),
                    ppre.as_bytes(),
                    None,
                    0,
                    None,
                    0,
                    0,
                    1,
                );
                if ml >= 0 {
                    // c:2318-2325 — matcher matched the prefix.
                    let cut = (ml as usize).min(lpre.len());
                    lpre = lpre[cut..].to_string();
                    bcp = ml;
                } else if lpre.len() <= lpl && ppre.starts_with(lpre.as_str()) {
                    // c:2335 — lpre is a prefix of ppre → nothing left.
                    lpre.clear();
                } else if lpre.len() > lpl && lpre.starts_with(ppre) {
                    // c:2337 — ppre is a literal prefix of lpre → strip it.
                    lpre = lpre[lpl..].to_string();
                } else {
                    // c:2339 — `*argv = NULL`: no candidate can match.
                    ppre_nomatch = true;
                }
                crate::ported::zle::compmatch::start_match();
            }
        }
    }
    if ppre_nomatch {
        return if mnum.load(Ordering::Relaxed) == _nm {
            1
        } else {
            0
        };
    }

    // c:2360-2389 — when `$compstate[pattern_match]` is set, compile the
    // line prefix+suffix (with the completion point as a `*` placeholder)
    // into a Patprog so candidates are matched as GLOBS instead of literal
    // prefixes. This is what makes `_approximate` work: it turns on
    // pattern_match and injects a leading `(#a$n)` approximate-glob into
    // PREFIX (via the compadd prefix injector), so e.g. `(#a1)aple*` matches
    // `apple`. The Rust port previously always passed `cp = None` to
    // comp_match, so pattern_match / approximate completion did nothing.
    // C uses an `x` placeholder at the completion point precisely so the
    // point wildcard alone never enables pattern matching; mirror that by
    // probing haswilds() on lpre+lsuf WITHOUT the placeholder — only real
    // wildcards in the typed prefix/suffix (`(#a1)`, `*`, `[...]`) count.
    let cp: Option<crate::ported::pattern::Patprog> = {
        let cpm = get_compstate_str("pattern_match").unwrap_or_default();
        if !cpm.is_empty() {
            let is = cpm.starts_with('*'); // c:2361 is = (*comppatmatch == '*')
            let mut probe = format!("{}{}", lpre, lsuf);
            crate::ported::glob::tokenize(&mut probe); // c:2371
            if crate::ported::pattern::haswilds(&probe) {
                // c:2372 — has real wildcards.
                let mut pat = String::new();
                pat.push_str(&lpre); // c:2367 lpre
                if is {
                    // c:2374 — `tmp[llpl] = Star` (completion-point wildcard).
                    pat.push('*');
                }
                pat.push_str(&lsuf); // c:2369 lsuf
                crate::ported::glob::tokenize(&mut pat);
                crate::ported::glob::remnulargs(&mut pat); // c:2376
                let prog = crate::ported::pattern::patcompile(&pat, 0, None); // c:2377
                if prog.is_some() {
                    haspattern.store(1, Ordering::Relaxed); // c:2378
                }
                prog
            } else {
                None
            }
        } else {
            None
        }
    };

    // c:2278-2300 — dat.ipre/isuf/ppre/psuf duplication with lipre/lisuf.
    if let Some(ref existing) = dat.ipre.clone() {
        dat.ipre = Some(if !lipre.is_empty() {
            format!("{}{}", lipre, existing)
        } else {
            existing.clone()
        });
    } else if !lipre.is_empty() {
        dat.ipre = Some(lipre.clone());
    }
    if let Some(ref existing) = dat.isuf.clone() {
        dat.isuf = Some(if !lisuf.is_empty() {
            format!("{}{}", lisuf, existing)
        } else {
            existing.clone()
        });
    } else if !lisuf.is_empty() {
        dat.isuf = Some(lisuf.clone());
    }
    let quote_flag = if (dat.aflags & CAF_QUOTE) != 0 { 1 } else { 0 };
    if let Some(ref existing) = dat.ppre.clone() {
        let quoted = if (dat.flags & 0x0001/*CMF_FILE*/) != 0 {
            tildequote(existing, quote_flag)
        } else {
            multiquote(existing, quote_flag)
        };
        dat.ppre = Some(quoted);
    }
    if let Some(ref existing) = dat.psuf.clone() {
        dat.psuf = Some(multiquote(existing, quote_flag));
    }
    let ppl = dat.ppre.as_deref().map(|s| s.len()).unwrap_or(0);
    let psl = dat.psuf.as_deref().map(|s| s.len()).unwrap_or(0);

    // c:2431-2455 — the whole `if (*argv) { … }` block was missing.
    if !argv.is_empty() {
        // c:2436-2440 — `if (!dat->prpre && (dat->prpre = dupstring(oppre)))
        //                    { singsub(&(dat->prpre)); untokenize(dat->prpre); }`
        // `oppre` is the UNQUOTED `dat->ppre` saved at c:2084, before the
        // c:2288 quoting above. `prpre` is the path used for the real
        // filesystem probes (ztat/opendir in add_match_data c:2735+), so it
        // has to be the substituted, untokenized form — not the quoted one.
        // Without this default, `compadd -p <dir>/` with no explicit `-W`
        // left `prpre` empty and every file test ran against the bare match
        // name in the CWD.
        if dat.prpre.is_none() {
            if let Some(op) = oppre.clone() {
                let subbed = crate::ported::subst::singsub(&op); // c:2437
                dat.prpre = Some(crate::ported::lex::untokenize(&subbed).to_string());
                // c:2438
            }
        }
        // c:2443-2447 — `-r`/`-R` are mutually exclusive: a remove-func
        // (`remf`) wins and CLEARS `rems`. The port passed both through to
        // the match, so a compadd carrying both applied the char-based
        // suffix removal as well as the widget.
        if dat.remf.is_some() {
            dat.rems = None; // c:2445
        }
        // c:2449-2454 — quote the line prefix/suffix used for matching with
        // the OUTER quoting level (`ign = 1`), so comp_match compares the
        // candidate against text quoted the same way the line is.
        lpre = if (dat.aflags & CAF_QUOTE) == 0
            && dat.ppre.is_none()
            && (dat.flags & 0x0001/*CMF_FILE*/) != 0
        {
            tildequote(&lpre, 1) // c:2452
        } else {
            multiquote(&lpre, 1) // c:2452
        };
        lsuf = multiquote(&lsuf, 1); // c:2454
    }

    // c:2170 — `doadd` was computed (and set hasmatched/hasunmatched) above,
    // at C's position for the assignment.
    tracing::debug!(
        target: "compsys_args",
        %lpre,
        %lsuf,
        doadd,
        caf_match = (dat.aflags & CAF_MATCH) != 0,
        dpar = ?dat.dpar,
        "addmatches candidate-loop setup"
    );
    let mut apar_list: Vec<String> = Vec::new();
    let mut opar_list: Vec<String> = Vec::new();

    // c:2189-2207 — `-D par…` setup. Each `dpar` name holds an array
    // PARALLEL to the candidate words; as each word is added, the current
    // element of every dpar array is consumed into an output list, and at
    // the end the output lists are written back (so each dpar param ends
    // up holding just the entries for the words that matched). Names whose
    // array is empty/missing are dropped (C swaps them out). Bug #657 —
    // the previous port collected apar/opar but never handled dpar.
    let mut dpar_names: Vec<String> = Vec::new();
    let mut dparr: Vec<Vec<String>> = Vec::new();
    let mut dpar_idx: Vec<usize> = Vec::new();
    let mut dparl: Vec<Vec<String>> = Vec::new();
    for name in &dat.dpar {
        match crate::ported::params::getaparam(name) {
            Some(arr) if !arr.is_empty() => {
                dpar_names.push(name.clone()); // c:2197 getaparam non-empty
                dparr.push(arr);
                dpar_idx.push(0);
                dparl.push(Vec::new());
            }
            _ => {} // c:2200 — drop names with an empty/missing array
        }
    }

    // c:2460-2476 + c:2582-2600 — CAF_ARRAYS expansion. `compadd -a
    // NAME…` / `-k NAME…` pass PARAMETER names, not literal matches:
    // the candidates are the values of the named arrays (or, with
    // CAF_KEYS from `-k`, the keys of the named associative arrays).
    // C weaves array-switching through the candidate loop via the
    // `next_array` label; because every non-empty array's elements are
    // consumed in order, the resulting candidate set is just the
    // concatenation of `get_data_arr` over the names — so expand up
    // front. Without this, `_command_names`' `compadd -k commands`
    // adds the literal word "commands" instead of every command name.
    let expanded_argv: Vec<String>;
    let argv: &[String] = if (dat.aflags & CAF_ARRAYS) != 0 {
        let keys = (dat.aflags & CAF_KEYS) != 0; // c:2468 CAF_KEYS
        let mut acc: Vec<String> = Vec::new();
        for name in argv {
            if let Some(vals) = get_data_arr(name, keys) {
                acc.extend(vals);
            }
        }
        expanded_argv = acc;
        &expanded_argv
    } else {
        argv
    };

    // c:2482-2601 — main candidate loop.
    let mut added = 0i32;
    let mut disp_idx = 0usize;
    let mut compignored_local = 0i32;
    // c:2520-2522 / c:2540-2542 — `-D` parallel-array bookkeeping: the dpar
    // arrays advance in LOCKSTEP with the candidate words. When a word is
    // skipped (ignored, or comp_match fails) its dpar element must be dropped
    // — i.e. `dpar_idx` advances WITHOUT the corresponding value being kept —
    // so that surviving words stay aligned with their array elements. The
    // port only advanced `dpar_idx` on the KEEP path (else branch below), so
    // after any non-matching word every survivor grabbed the wrong element
    // (e.g. `compadd -D` for path completion kept `/Applications` instead of
    // `/tmp`), breaking `cmd /path/<TAB>` entirely.
    macro_rules! dpar_skip_word {
        () => {
            for i in 0..dpar_idx.len() {
                if dpar_idx[i] < dparr[i].len() {
                    dpar_idx[i] += 1;
                }
            }
        };
    }
    'cand: for word in argv {
        // c:2482
        // c:2486-2489 — advance disp index.
        let cur_disp = if !disp_arr.is_empty() && disp_idx < disp_arr.len() {
            let d = disp_arr[disp_idx].clone();
            disp_idx += 1;
            Some(d)
        } else {
            None
        };

        // c:2491-2527 — aign/pign suffix-test + Patprog test.
        if !aign.is_empty() || !pign.is_empty() {
            let full = format!(
                "{}{}{}",
                dat.ppre.as_deref().unwrap_or(""),
                word,
                dat.psuf.as_deref().unwrap_or("")
            );
            // c:2508-2510 — literal-suffix check. C requires the ignored
            // suffix to be STRICTLY shorter than the whole string
            // (`filell < il`), so an fignore entry equal to the candidate
            // does NOT ignore it; the port's `>=` dropped those candidates.
            for suf in &aign {
                if full.len() > suf.len() && full.ends_with(suf.as_str()) {
                    // c:2519 — `compignored++` on the GLOBAL, which is
                    // `$compstate[ignored]` (complete.c:1300). The port
                    // counted into a local that was discarded at the end of
                    // the function, so `_ignored` (which returns 1 unless
                    // `$compstate[ignored]` is non-zero) could never fire and
                    // ignored-patterns had no fallback completer.
                    crate::ported::zle::complete::COMPIGNORED.fetch_add(1, Ordering::Relaxed);
                    compignored_local += 1;
                    dpar_skip_word!(); // c:2520
                    continue 'cand;
                }
            }
            // c:2512-2517 — Patprog check.
            for prog in &pign {
                if crate::ported::pattern::pattry(prog, &full) {
                    crate::ported::zle::complete::COMPIGNORED.fetch_add(1, Ordering::Relaxed); // c:2519
                    compignored_local += 1;
                    dpar_skip_word!(); // c:2520
                    continue 'cand;
                }
            }
        }

        // c:2528 — CAF_MATCH dispatch: when CAF_MATCH is set, run
        // comp_match with the active matcher chain (else branch); else
        // emit the (multi)quoted word directly with a single-anchor
        // Cline (no-matcher path c:2530-2533).
        let ms: String;
        let _lc;
        let isexact;
        if (dat.aflags & CAF_MATCH) == 0 {
            // c:2528-2534 — non-match mode: just (multi)quote the word.
            ms = if (dat.aflags & CAF_QUOTE) != 0 {
                word.clone()
            } else {
                multiquote(word, 0)
            };
            let sl = ms.len() as i32;
            _lc = bld_parts(&ms, sl, -1, None, None);
            isexact = 0;
        } else {
            // c:2535
            // c:2535-2546 — matcher-driven mode via comp_match.
            let qu = if (dat.aflags & CAF_QUOTE) != 0 {
                0
            } else if dat.ppre.is_some() || (dat.flags & 0x0001/*CMF_FILE*/) == 0 {
                1
            } else {
                2
            };
            let mut lc_out: Option<Box<Cline>> = None;
            let mut isexact_out = 0i32;
            // c:2535 — comp_match(lpre, lsuf, s, cp, &lc, qu, &bpl, bcp,
            //          &bsl, bcs, &isexact).
            match crate::ported::zle::compmatch::comp_match(
                &lpre,
                &lsuf,
                word,
                cp.as_ref(),
                Some(&mut lc_out),
                qu,
                None,
                bcp, // c:2535 — brace-count base advanced by the ppre strip
                None,
                0,
                &mut isexact_out,
            ) {
                Some(matched) => {
                    ms = matched;
                    _lc = lc_out;
                    isexact = isexact_out;
                }
                None => {
                    dpar_skip_word!(); // c:2540 — drop this word's dpar element
                    continue 'cand; // c:2541-2545 reject
                }
            }
        }

        if doadd {
            // c:2547
            // c:2556 — add_match_data.
            let cm = add_match_data(
                0,
                &ms,
                word,
                _lc.clone(), // line — real Cline from comp_match
                dat.ipre.as_deref().unwrap_or(""),
                "", // ripre
                dat.isuf.as_deref().unwrap_or(""),
                dat.pre.as_deref(),
                dat.prpre.as_deref().unwrap_or(""),
                dat.ppre.as_deref().unwrap_or(""),
                None, // pline (path-prefix Cline; unused on this path)
                dat.psuf.as_deref().unwrap_or(""),
                None, // sline (path-suffix Cline; unused on this path)
                dat.suf.as_deref(),
                dat.flags,
                isexact,
            );
            // c:2557-2564 — `cm->disp = dparr ? *dparr++ : NULL` (and the
            // CMF_DISPLINE flag when `-l`/CAF_...): C patches the STORED
            // match through the returned pointer; the port's return-by-value
            // clone can't, so patch the just-pushed copy in the live list.
            // Without this every `compadd -d` display string (the
            // description lines _describe/_arguments build) was silently
            // dropped and lists rendered bare match names.
            // c:2562-2563 — `if (disp) cm->disp = dupstring(*disp)`. C
            // patches the STORED match through the returned pointer; the
            // port's add_match_data returns a VALUE clone while the stored
            // copy lives in the `matches` list, so patch that copy in place
            // (inline — no C-counterpart fn). Without this every
            // `compadd -d` display string (the description lines
            // _describe/_arguments build) was silently dropped and lists
            // rendered bare match names.
            // c:2560-2561 — `cm->rems = dat->rems; cm->remf = dat->remf`. These
            // were previously SKIPPED here (only `disp` was patched), so a
            // `compadd -r <chars>` / `-R <widget>` custom suffix-removal spec
            // never reached the match: makesuffixstr got `s=None` and the
            // char-based auto-remove-suffix was inert. Patch them on the just-
            // pushed match copy (the port's add_match_data returns a value).
            {
                if let Ok(mut g) = crate::comp_match_handles::matches_arc().lock() {
                    if let Some(last) = g.last_mut() {
                        last.rems = dat.rems.clone(); // c:2560
                        last.remf = dat.remf.clone(); // c:2561
                        if cur_disp.is_some() {
                            last.disp = cur_disp.clone(); // c:2562-2563
                        }
                    }
                }
            }
            let _ = cm;
            added += 1;
        } else {
            // c:2566
            if dat.apar.is_some() {
                // c:2567
                apar_list.push(ms.clone());
            }
            if dat.opar.is_some() {
                // c:2569
                opar_list.push(word.clone());
            }
            // c:2571-2578 — consume one element from each live dpar array
            // in lockstep with this added word.
            for i in 0..dparl.len() {
                if dpar_idx[i] < dparr[i].len() {
                    dparl[i].push(dparr[i][dpar_idx[i]].clone());
                    dpar_idx[i] += 1;
                }
            }
        }
    }

    // c:2602-2608 — apar/opar/dpar writeback.
    if let Some(ref name) = dat.apar {
        setaparam(name, apar_list);
    }
    if let Some(ref name) = dat.opar {
        setaparam(name, opar_list);
    }
    // c:2606-2607 — set_list_array(dpar[i], dparl[i]).
    for (i, name) in dpar_names.iter().enumerate() {
        setaparam(name, std::mem::take(&mut dparl[i]));
    }

    // c:2610 — explanation emit.
    if dat.exp.is_some() {
        // c:2610
        addexpl(false);
    }

    // c:2612-2614 — `<all>` placeholder when CAF_ALL set.
    let hasall = hasallmatch.load(Ordering::Relaxed);
    if hasall == 0 && (dat.aflags & CAF_ALL) != 0 {
        addmatch(
            "<all>",
            dat.flags | crate::ported::zle::comp_h::CMF_ALL,
            None,
            true,
        );
        hasallmatch.store(1, Ordering::Relaxed);
    }

    // c:2616-2617 — dummy entries. C's addmatch advances the SHARED disp
    // cursor (`&disp`, c:2054-2058), so each dummy carries the next
    // remaining display string — this is how the description-only lines
    // (compdescribe's CRT_EXPL `-E n` run: "opt  --  description") reach
    // the list. Passing None here dropped every description line.
    while dat.dummies > 0 {
        let d: Option<&str> = if disp_idx < disp_arr.len() {
            let d = Some(disp_arr[disp_idx].as_str());
            disp_idx += 1;
            d
        } else {
            None
        };
        addmatch(
            "",
            dat.flags | crate::ported::zle::comp_h::CMF_DUMMY,
            d,
            false,
        );
        dat.dummies -= 1;
    }

    tracing::debug!(
        target: "compsys_args",
        added,
        mnum = mnum.load(Ordering::Relaxed),
        doadd,
        "addmatches candidate-loop done"
    );
    let _ = (ppl, psl, compignored_local, added);

    // c:2624-2626 — `instring = ois; inbackt = oib; autoq = oaq;`. The
    // c:2139-2168 block above rewrites all three from `$compstate[quote]`
    // for the duration of THIS compadd only; the port set them and never
    // put them back, so a single `compadd` inside a quoted word left
    // `instring`/`inbackt` clobbered for the rest of the completion (and for
    // `get_comp_string`'s next round, which reads them).
    instring_set(ois); // c:2624
    inbackt_set(oib); // c:2625
    autoq_set(&oaq); // c:2626
                     // c:2627-2630 — `zsfree(qipre); zsfree(qisuf); qipre = oqp; qisuf = oqs;`
    for (global, v) in [
        (&crate::ported::zle::zle_tricky::QIPRE, oqp), // c:2629
        (&crate::ported::zle::zle_tricky::QISUF, oqs), // c:2630
    ] {
        if let Ok(mut g) = global.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = v;
        }
    }

    // c:2632-2633 — `if (mnum == nm) haspattern = ohp;`. A compadd that
    // added NOTHING must not leave `haspattern` raised by its own pattern
    // compile at c:2387/c:2428 — `do_ambiguous` keys menu-vs-insert
    // behaviour off it (compresult.c:764).
    if mnum.load(Ordering::Relaxed) == _nm {
        haspattern.store(ohp, Ordering::Relaxed); // c:2633
    }

    // c:2635 — `return (mnum == nm)`: non-zero (1) when NO new matches were
    // added this call, 0 when at least one landed. Was hardcoded `0`, so
    // every `compadd` reported success even against zero matches — the
    // `completer` chain (_main_complete) then stopped after `_complete`
    // and never advanced to `_approximate` / `_ignored` / etc., and any
    // `compadd … && ret=0` idiom in a completer wrongly took the match arm.
    if mnum.load(Ordering::Relaxed) == _nm {
        1
    } else {
        0
    }
}

// =====================================================================
// add_match_data — `Src/Zle/compcore.c:2643`.
// =====================================================================

/// Direct port of `Cmatch add_match_data(int alt, char *str, char *orig,
///    Cline line, char *ipre, char *ripre, char *isuf, char *pre,
///    char *prpre, char *ppre, Cline pline, char *psuf, Cline sline,
///    char *suf, int flags, int exact)` from compcore.c:2643.
///
/// Builds one `Cmatch` from the supplied prefix/suffix bits plus the
/// surrounding Cline chain. Threads `line`/`pline`/`sline` through
/// `cline_matched` so the CLF_MATCHED state-machine update fires the
/// same way as C, then performs path-prefix/suffix splicing via
/// `bld_parts` to extend the Cline chain at the appropriate anchor.
#[allow(clippy::too_many_arguments)]
pub fn add_match_data(
    // c:2643
    alt: i32,
    str: &str,
    orig: &str,
    mut line: Option<Box<Cline>>,
    ipre_: &str,
    ripre_: &str,
    isuf_: &str,
    pre: Option<&str>,
    prpre: &str,
    ppre: &str,
    mut pline: Option<Box<Cline>>,
    psuf: &str,
    mut sline: Option<Box<Cline>>,
    suf: Option<&str>,
    flags: i32,
    exact: i32,
) -> Cmatch {
    // c:2663 — DPUTS(!line, "BUG: add_match_data() without cline")
    DPUTS!(line.is_none(), "BUG: add_match_data() without cline"); // c:2663
                                                                   // c:2656 — `Aminfo ai = (alt ? fainfo : ainfo);` — EVERY `ai->…` below
                                                                   // (c:3002 join_clines, c:3005 count++, c:3023 firstm, c:3036-3063 exact)
                                                                   // goes through this selection. The port bound it and then used the plain
                                                                   // `ainfo` at all five sites, so every alternate-set match (`alt != 0` —
                                                                   // the fignore/ignored-suffix path) accumulated into the MAIN aminfo:
                                                                   // `ainfo.count` counted matches that were never offered, and the
                                                                   // unambiguous cline built from `ainfo.line` was polluted by strings the
                                                                   // user asked to ignore.
    let ai_ref: &OnceLock<Mutex<Option<Aminfo>>> = if alt != 0 { &fainfo } else { &ainfo }; // c:2656
                                                                                            // c:2666-2671 — cline_matched(line); pline; sline.
    cline_matched(&mut line);
    if pline.is_some() {
        cline_matched(&mut pline);
    }
    if sline.is_some() {
        cline_matched(&mut sline);
    }

    // c:2675-2697 — accumulator lengths.
    let psl = psuf.len();
    let isl = isuf_.len();
    let qisuf_v = qisuf_get(); // c:2680
    let qisl = qisuf_v.len();
    let _salen = (if sline.is_none() { psl } else { 0 }) + isl + qisl; // c:2675-2683

    let ipl = ipre_.len();
    // c:2769-2770 — `if (pre) palen += (pl = strlen(pre));`: C leaves `pl` at 0
    // for a NULL `pre`, so every later use (bld_parts at c:2809, the memcpy at
    // c:2837) copies nothing. A zero-length &str is exactly that behaviour, so
    // the arithmetic below keeps using a &str view; only the STORED cm.pre /
    // cm.suf keep the Option, which is the distinction c:2943-2944 preserves.
    let pre_s: &str = pre.unwrap_or("");
    let suf_s: &str = suf.unwrap_or("");
    let _ppl = ppre.len();
    let _pl = pre_s.len();
    let qipre_v = qipre_get(); // c:2686
    let qipl_v = qipre_v.clone();
    let _qipl = qipl_v.len();

    let _stl = str.len();
    let _lpl = ripre_.len();
    let _lsl = suf_s.len();
    let _ml = ipl;

    // c:2671-2762 — path-suffix Cline splicing. salen accumulates psl
    // (psuf when no sline), isl (isuf), qisl (qisuf). When salen > 0
    // and line is non-empty, we walk to the tail and append the
    // bld_parts-built Cline for each contributing string.
    let psl_local = if sline.is_none() && !psuf.is_empty() {
        psuf.len() as i32
    } else {
        0
    };
    let isl_local = isuf_.len() as i32;
    let qisl_local = qisuf_v.len() as i32;
    let salen = psl_local + isl_local + qisl_local;
    if salen > 0 && line.is_some() {
        // Walk to the tail of line via .next.
        unsafe {
            let mut tail: *mut Option<Box<Cline>> = &mut line;
            while let Some(ref n) = *tail {
                if n.next.is_none() {
                    break;
                }
                tail = &mut (*tail).as_mut().unwrap().next;
            }
            // For each contributing string, build a Cline chain via
            // bld_parts and attach to the tail node's .next.
            if psl_local > 0 {
                let s = bld_parts(psuf, psl_local, psl_local, None, None);
                if let Some(node) = (*tail).as_mut() {
                    node.next = s;
                    while let Some(ref nn) = node.next {
                        if nn.next.is_none() {
                            break;
                        }
                        // already linked correctly; loop to advance tail
                        break;
                    }
                }
                // Walk to the new tail.
                while let Some(ref n) = *tail {
                    if n.next.is_none() {
                        break;
                    }
                    tail = &mut (*tail).as_mut().unwrap().next;
                }
            }
            if isl_local > 0 {
                let s = bld_parts(isuf_, isl_local, isl_local, None, None);
                if let Some(node) = (*tail).as_mut() {
                    node.next = s;
                }
                while let Some(ref n) = *tail {
                    if n.next.is_none() {
                        break;
                    }
                    tail = &mut (*tail).as_mut().unwrap().next;
                }
            }
            if qisl_local > 0 {
                let mut s = bld_parts(&qisuf_v, qisl_local, qisl_local, None, None);
                // c:2741 — qsl->flags |= CLF_SUF; qsl->suffix = qsl->prefix.
                if let Some(qsl) = s.as_mut() {
                    qsl.flags |= crate::ported::zle::comp_h::CLF_SUF;
                    qsl.suffix = qsl.prefix.take();
                }
                if let Some(node) = (*tail).as_mut() {
                    node.next = s;
                }
            }
        }
    }

    // c:2766-2873 — path-prefix Cline splicing. palen accumulates qipl,
    // ipl, pl, ppl (when no pline). Each contributing string gets a
    // bld_parts Cline prepended to `line`.
    let qipl_local = qipre_v.len() as i32;
    let ipl_local = ipre_.len() as i32;
    let pl_local = pre_s.len() as i32;
    let ppl_local = if pline.is_none() && !ppre.is_empty() {
        ppre.len() as i32
    } else {
        0
    };
    if pl_local > 0 {
        if ppl_local > 0 {
            let p = bld_parts(ppre, ppl_local, ppl_local, None, None);
            // Walk p to its tail, link its tail's next to line.
            if p.is_some() {
                let mut p_chain = p;
                let mut tail: *mut Option<Box<Cline>> = &mut p_chain;
                unsafe {
                    while let Some(ref n) = *tail {
                        if n.next.is_none() {
                            break;
                        }
                        tail = &mut (*tail).as_mut().unwrap().next;
                    }
                    if let Some(t) = (*tail).as_mut() {
                        t.next = line.take();
                    }
                }
                line = p_chain;
            }
        }
        let p = bld_parts(pre_s, pl_local, pl_local, None, None);
        if let Some(mut head) = p {
            let mut t: *mut Option<Box<Cline>> = &mut head.next;
            unsafe {
                while (*t).is_some() {
                    if (*t).as_deref().unwrap().next.is_none() {
                        break;
                    }
                    t = &mut (*t).as_mut().unwrap().next;
                }
                *t = line.take();
            }
            line = Some(head);
        }
        if ipl_local > 0 {
            let p = bld_parts(ipre_, ipl_local, ipl_local, None, None);
            if let Some(mut head) = p {
                let mut t: *mut Option<Box<Cline>> = &mut head.next;
                unsafe {
                    // Walk to the empty slot past the tail node, then attach
                    // `line` there. The previous code stopped AT the last node
                    // and did `*t = line`, OVERWRITING it (dropped the final
                    // bld_parts segment — e.g. the last dir component `zdt3/`
                    // of `/private/tmp/zdt3/` — so ambiguous path completion
                    // corrupted the line). c:2812/2818/2824/2841 `lp->next =
                    // line` appends; it never replaces lp.
                    while (*t).is_some() {
                        t = &mut (*t).as_mut().unwrap().next;
                    }
                    *t = line.take();
                }
                line = Some(head);
            }
        }
        if qipl_local > 0 {
            let p = bld_parts(&qipre_v, qipl_local, qipl_local, None, None);
            if let Some(mut head) = p {
                let mut t: *mut Option<Box<Cline>> = &mut head.next;
                unsafe {
                    // Walk to the empty slot past the tail node, then attach
                    // `line` there. The previous code stopped AT the last node
                    // and did `*t = line`, OVERWRITING it (dropped the final
                    // bld_parts segment — e.g. the last dir component `zdt3/`
                    // of `/private/tmp/zdt3/` — so ambiguous path completion
                    // corrupted the line). c:2812/2818/2824/2841 `lp->next =
                    // line` appends; it never replaces lp.
                    while (*t).is_some() {
                        t = &mut (*t).as_mut().unwrap().next;
                    }
                    *t = line.take();
                }
                line = Some(head);
            }
        }
    } else if qipl_local + ipl_local + pl_local + ppl_local > 0 || pline.is_some() {
        // c:2827-2842 — consolidated apre buffer.
        let apre = format!(
            "{}{}{}{}",
            qipre_v.as_str(),
            ipre_,
            pre_s,
            if pline.is_none() { ppre } else { "" }
        );
        let apre_len = apre.len() as i32;
        if apre_len > 0 {
            let p = bld_parts(&apre, apre_len, apre_len, None, None);
            if let Some(mut head) = p {
                let mut t: *mut Option<Box<Cline>> = &mut head.next;
                unsafe {
                    // Walk to the empty slot past the tail node, then attach
                    // `line` there. The previous code stopped AT the last node
                    // and did `*t = line`, OVERWRITING it (dropped the final
                    // bld_parts segment — e.g. the last dir component `zdt3/`
                    // of `/private/tmp/zdt3/` — so ambiguous path completion
                    // corrupted the line). c:2812/2818/2824/2841 `lp->next =
                    // line` appends; it never replaces lp.
                    while (*t).is_some() {
                        t = &mut (*t).as_mut().unwrap().next;
                    }
                    *t = line.take();
                }
                line = Some(head);
            }
        }
    }

    let stl = str.len();

    // c:2929-2932 — Cmatch allocation + str/orig/ppre/psuf.
    let mut cm = Cmatch::default(); // c:2929
    cm.str = Some(str.to_string()); // c:2930
    cm.orig = Some(orig.to_string()); // c:2931
    cm.ppre = if ppre.is_empty() {
        None
    } else {
        Some(ppre.into())
    }; // c:2932
    cm.psuf = if psuf.is_empty() {
        None
    } else {
        Some(psuf.into())
    }; // c:2933

    // c:2934 — prpre only when CMF_FILE.
    cm.prpre = if (flags & CMF_FILE) != 0 && !prpre.is_empty() {
        Some(prpre.into())
    } else {
        None
    };

    // c:2935-2938 — ipre = qipre + ipre (concat when qipre non-empty).
    // qipre_v already computed above.
    cm.ipre = if !qipre_v.is_empty() {
        if !ipre_.is_empty() {
            Some(format!("{}{}", qipre_v, ipre_))
        } else {
            Some(qipre_v.clone())
        }
    } else if !ipre_.is_empty() {
        Some(ipre_.into())
    } else {
        None
    };

    cm.ripre = if ripre_.is_empty() {
        None
    } else {
        Some(ripre_.into())
    }; // c:2939

    // c:2940-2943 — isuf = isuf + qisuf (concat when qisuf non-empty).
    cm.isuf = if !qisuf_v.is_empty() {
        if !isuf_.is_empty() {
            Some(format!("{}{}", isuf_, qisuf_v))
        } else {
            Some(qisuf_v.clone())
        }
    } else if !isuf_.is_empty() {
        Some(isuf_.into())
    } else {
        None
    };

    // c:2943-2944 — `cm->pre = pre; cm->suf = suf;` are UNCONDITIONAL. The six
    // neighbours above them (ppre/psuf/prpre/ipre/ripre/isuf, c:2931-2942) each
    // collapse empty-to-NULL with `x && *x ? x : NULL`, and C pointedly does NOT
    // do that for these two: an explicit `compadd -S ''` is a REAL empty suffix
    // and must stay distinguishable from "no -S given" (`dat.suf` starts NULL at
    // complete.c:625). This port collapsed both, so `-S ''` was indistinguishable
    // from omitting -S.
    cm.pre = pre.map(|s| s.to_string()); // c:2943
    cm.suf = suf.map(|s| s.to_string()); // c:2944

    // c:2946 — flags + CMF_PACKED/CMF_ROWS from complist.
    let complist_s = COMPLIST
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.clone()))
        .unwrap_or_default();
    let extra_flags = (if complist_s.contains("packed") {
        CMF_PACKED
    } else {
        0
    }) | (if complist_s.contains("rows") {
        CMF_ROWS
    } else {
        0
    });
    cm.flags = flags | extra_flags;

    // c:2950-2951 — mode/fmode init to 0.
    cm.mode = 0;
    cm.fmode = 0;
    cm.modec = '\0';
    cm.fmodec = '\0';

    // c:2952-2970 — CMF_FILE: stat the path for mode + modec.
    use crate::ported::zle::comp_h::CMF_FILE;
    if (flags & CMF_FILE) != 0 && !orig.is_empty() && !orig.ends_with('/') {
        let pb = format!("{}{}", cm.prpre.as_deref().unwrap_or("./"), orig);
        // c:2960-2963 — `ztat(pb, &buf, 1); cm->mode = buf.st_mode;
        //   if ((cm->modec = file_type(buf.st_mode)) == ' ') cm->modec = 0;`
        // The `1` is ztat's ls flag → lstat, so modec is the type of the
        // link itself (`@` for a symlink). This is the marker printlist
        // shows, so it must come from lstat, not stat.
        if let Some(meta) = ztat(&pb, true) {
            use std::os::unix::fs::MetadataExt;
            cm.mode = meta.mode();
            let c = crate::ported::glob::file_type(cm.mode); // c:2962
            cm.modec = if c == ' ' { '\0' } else { c }; // c:2963
        }
        // c:2965-2968 — `ztat(pb, &buf, 0)` → stat, so fmode/fmodec is the
        // type of the symlink target (the marker used when following links).
        if let Some(meta) = ztat(&pb, false) {
            use std::os::unix::fs::MetadataExt;
            cm.fmode = meta.mode();
            let c = crate::ported::glob::file_type(cm.fmode); // c:2967
            cm.fmodec = if c == ' ' { '\0' } else { c }; // c:2968
        }
    }

    // c:2970-2972 — `if ((*compqstack == QT_BACKSLASH && compqstack[1]) ||
    //                    (autoq && *compqstack && compqstack[1] == QT_BACKSLASH))
    //                    cm->flags |= CMF_NOSPACE;`
    //
    // Missing entirely. A match produced inside a nested quoting context
    // whose outer level is backslash-quoting must not get the automatic
    // trailing space (it would be swallowed by the enclosing quotes);
    // without the flag every such completion inserted a stray space.
    {
        let qstack = COMPQSTACK
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let qb = qstack.as_bytes();
        let autoq_set_now = AUTOQ
            .get()
            .and_then(|m| m.lock().ok().map(|g| !g.is_empty()))
            .unwrap_or(false);
        let q0_bslash = !qb.is_empty() && qb[0] as i32 == QT_BACKSLASH;
        let q1_bslash = qb.len() > 1 && qb[1] as i32 == QT_BACKSLASH;
        if (q0_bslash && qb.len() > 1) || (autoq_set_now && !qb.is_empty() && q1_bslash) {
            cm.flags |= crate::ported::zle::comp_h::CMF_NOSPACE; // c:2972
        }
    }

    // c:2973-2992 — brpl/brsl brace position arrays. Walk BRBEG/BREND
    // (the global Brinfo chains from `Src/Zle/zle_tricky.c`), reading
    // `curpos` for each entry — the per-match position addmatches snapshots
    // at c:2132-2135 from `pos`/`qpos` according to CAF_QUOTE. The port read
    // the raw `qpos`, which is the UNQUOTED-case value only, so a
    // `compadd -Q` brace expansion got the wrong insertion offsets.
    cm.brpl = BRBEG
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| {
            g.as_ref().map(|head| {
                let mut out: Vec<i32> = Vec::new();
                let mut cur = Some(head.as_ref());
                while let Some(n) = cur {
                    out.push(n.curpos);
                    cur = n.next.as_deref();
                }
                out
            })
        })
        .unwrap_or_default();
    cm.brsl = BREND
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| {
            g.as_ref().map(|head| {
                let mut out: Vec<i32> = Vec::new();
                let mut cur = Some(head.as_ref());
                while let Some(n) = cur {
                    out.push(n.curpos);
                    cur = n.next.as_deref();
                }
                out
            })
        })
        .unwrap_or_default();

    cm.qipl = qipre_v.len() as i32; // c:2994
    cm.qisl = qisuf_v.len() as i32; // c:2995
                                    // c:2996 — autoq read.
    let autoq_v = AUTOQ
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.clone()))
        .unwrap_or_default();
    cm.autoq = if !autoq_v.is_empty() {
        Some(autoq_v)
    } else if INBACKT.load(Ordering::Relaxed) != 0 {
        Some("`".into())
    } else {
        None
    };

    cm.rems = None;
    cm.remf = None;
    cm.disp = None; // c:2997

    // c:3002 — ai->line = join_clines(ai->line, line).
    if let Ok(mut g) = ai_ref.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(a) = g.as_mut() {
            let old_line = a.line.take();
            a.line = crate::ported::zle::compmatch::join_clines(old_line, line);
        }
    }

    // c:3004 — mnum++.
    mnum.fetch_add(1, Ordering::Relaxed);

    // c:3005 — ai->count++.
    if let Ok(mut g) = ai_ref.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(a) = g.as_mut() {
            a.count += 1;
        }
    }

    // c:3008 — addlinknode((alt ? fmatches : matches), cm). Already
    // wired below via matches Vec push.

    // c:3009-3010 — newmatches = 1; mgroup->new = 1. Only the first half was
    // ported; without `mgroup->new` the group holding this match was not
    // marked dirty, so `permmatches` could serve its cached permanent copy
    // and the match never appeared (the sibling `addmatch` at c:2068 sets it).
    newmatches.store(1, Ordering::Relaxed); // c:3009
    if let Ok(mg) = mgroup.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(grp) = mg.as_ref() {
            grp.new_.store(1, Ordering::Relaxed); // c:3010
        }
    }

    // c:3011-3012 — compignored++ when alt.
    if alt != 0 {
        crate::ported::zle::complete::COMPIGNORED.fetch_add(1, Ordering::Relaxed);
    }

    // c:3015-3016 — `if (!*complastprompt) dolastprompt = 0;`. Read the
    // `complastprompt` value (the "last_prompt" compstate, set at c:326 to
    // "yes"/"" from ALWAYS_LAST_PROMPT) — NOT `complastprefix`, a different
    // variable the previous port read by mistake. With that bug dolastprompt
    // was cleared on every completion, so `clearflag` stayed 0 and `trashzle`
    // never emitted TCCLEAREOD: an on-screen completion list was left
    // stranded when the command was accepted.
    let complastprompt_v = get_compstate_str("last_prompt").unwrap_or_default();
    if complastprompt_v.is_empty() {
        dolastprompt.store(0, Ordering::Relaxed);
    }

    // c:3018-3023 — curexpl.count/fcount increment.
    if let Ok(mut g) = curexpl.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(e) = g.as_mut() {
            if alt != 0 {
                e.fcount += 1;
            } else {
                e.count += 1;
            }
        }
    }

    // c:3023-3024 — ai->firstm = cm.
    if let Ok(mut g) = ai_ref.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(a) = g.as_mut() {
            if a.firstm.is_none() {
                a.firstm = Some(Box::new(cm.clone()));
            }
        }
    }

    // c:3027-3034 — minmlen/maxmlen tracking.
    let lpl = cm.ppre.as_deref().map(|s| s.len()).unwrap_or(0);
    let lsl = cm.psuf.as_deref().map(|s| s.len()).unwrap_or(0);
    let ml = (stl + lpl + lsl) as i32;
    let cur_min = minmlen.load(Ordering::Relaxed);
    let cur_max = maxmlen.load(Ordering::Relaxed);
    if ml < cur_min {
        minmlen.store(ml, Ordering::Relaxed);
    }
    if ml > cur_max {
        maxmlen.store(ml, Ordering::Relaxed);
    }

    // c:3036-3064 — exact-match tracking on ai.
    if exact != 0 {
        // c:3036
        // c:3039-3056 — `if (incompfunc && !*compexactstr) compexactstr =
        // ppre + str + psuf;` — publishing `$compstate[exact_string]`, which
        // `_main_complete` reads to decide whether to accept the exact match
        // outright. The publish was missing, so the parameter stayed at the
        // "" that do_completion writes at c:312 and the exact match was never
        // recognisable to the shell-function layer. Computed BEFORE taking
        // the ai lock (set_compstate_str takes the paramtab lock).
        let publish_exact = INCOMPFUNC.load(Ordering::Relaxed) != 0
            && get_compstate_str("exact_string")
                .map(|s| s.is_empty())
                .unwrap_or(true);
        let exact_str = format!(
            "{}{}{}",
            cm.ppre.as_deref().unwrap_or(""), // c:3047-3049
            str,                              // c:3051
            cm.psuf.as_deref().unwrap_or("")  // c:3053
        );
        let mut do_publish = false;
        if let Ok(mut g) = ai_ref.get_or_init(|| Mutex::new(None)).lock() {
            if let Some(a) = g.as_mut() {
                if a.exact == 0 {
                    // c:3037
                    a.exact = useexact.load(Ordering::Relaxed); // c:3038
                    do_publish = publish_exact;
                    a.exactm = Some(Box::new(cm.clone())); // c:3057
                } else if useexact.load(Ordering::Relaxed) != 0
                    // c:3058 — C also requires that this exact match DIFFERS
                    // from the one already recorded (`!ai->exactm ||
                    // !matcheq(cm, ai->exactm)`). The port dropped that half,
                    // so a second compadd of the SAME string (routine when
                    // several completers offer it) escalated `exact` to 2 —
                    // "ambiguous exact" — and the accept-exact path was lost.
                    && a.exactm
                        .as_deref()
                        .map(|em| !matcheq(&cm, em))
                        .unwrap_or(true)
                {
                    // c:3059-3060 — ambiguous exact: set to 2, clear exactm.
                    a.exact = 2;
                    a.exactm = None;
                }
            }
        }
        if do_publish {
            set_compstate_str("exact_string", &exact_str); // c:3046-3055
        }
    }

    // c:3064 — push cm into matches/fmatches LinkList.
    let cell = if alt != 0 {
        crate::comp_match_handles::fmatches_arc()
    } else {
        crate::comp_match_handles::matches_arc()
    };
    if let Ok(mut g) = cell.lock() {
        g.push(cm.clone());
    }

    cm // c:3067 return cm
}

// `lookup_complist_flags` deleted — Rust-only 8-line helper. Inlined
// at the single call site in callcompfunc (c:2049-2051).

// =====================================================================
// begcmgroup — `Src/Zle/compcore.c:3073`.
// =====================================================================

/// Port of `mod_export void begcmgroup(char *n, int flags)` from
/// compcore.c:3073.
pub fn begcmgroup(n: Option<&str>, flags: i32) {
    // c:3073
    if let Some(name) = n {
        // c:3073
        let mask = CGF_NOSORT | CGF_UNIQALL | CGF_UNIQCON                    // c:3085
                 | CGF_MATSORT | CGF_NUMSORT | CGF_REVSORT;
        // c:3078-3094 — reuse an existing group with the same name+flags.
        let reused = {
            let cell = amatches.get_or_init(|| Mutex::new(Vec::new()));
            cell.lock().ok().and_then(|g| {
                g.iter()
                    .find(|grp| grp.name.as_deref() == Some(name) && (grp.flags & mask) == flags)
                    .cloned() // c:3088
            })
        };
        if let Some(active) = reused {
            // c:3090-3093 — `expls = p->lexpls; matches = p->lmatches;
            //   fmatches = p->lfmatches; allccs = p->lallccs;`. These are
            //   pointer aliases into the reused group so appends keep flowing
            //   into it. `active` was `.cloned()` from `amatches`, and the
            //   `l*` fields are `Arc<Mutex<…>>`, so this clone SHARES the same
            //   allocations as the `amatches` original — rebinding the handles
            //   to them makes every subsequent append land in both, no copy.
            crate::comp_match_handles::rebind_current(
                &active.lmatches,
                &active.lfmatches,
                &active.lexpls,
                &active.lallccs,
            );
            let mc = mgroup.get_or_init(|| Mutex::new(None));
            if let Ok(mut s) = mc.lock() {
                *s = Some(active);
            }
            return; // c:3095
        }
    }
    let mut grp = Cmgroup::default(); // c:3101
    grp.name = n.map(String::from); // c:3105
    grp.flags = flags; // c:3108
    let cell = amatches.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut g) = cell.lock() {
        g.insert(0, grp.clone()); // c:3121-3124 — shares grp's Arc l* fields
    }
    // c:3110-3118 — alias the file-scope handles to the fresh group's (empty)
    // `l*` Arcs. Same allocation as the `amatches` clone above, so appends
    // land in both. No clearing needed: the new group's accumulators start
    // empty. Rebind BEFORE moving `grp` into `mgroup`.
    crate::comp_match_handles::rebind_current(
        &grp.lmatches,
        &grp.lfmatches,
        &grp.lexpls,
        &grp.lallccs,
    );
    let mc = mgroup.get_or_init(|| Mutex::new(None));
    if let Ok(mut s) = mc.lock() {
        *s = Some(grp);
    }
}

// =====================================================================
// endcmgroup — `Src/Zle/compcore.c:3131`.
// =====================================================================

/// Port of `mod_export void endcmgroup(char **ylist)` from
/// compcore.c:3131.
pub fn endcmgroup(ylist: Option<Vec<String>>) {
    // c:3131 — C is a one-liner (`mgroup->ylist = ylist`) because the
    // file-scope `matches`/`fmatches`/`expls`/`allccs` LinkLists ARE this
    // group's `l*` lists (aliased by begcmgroup). The Rust port keeps
    // those as separate Mutex globals, so on close we flush them into the
    // matching group inside `amatches` (identified by name+flags exactly
    // as begcmgroup dedups). Without this the matches added between
    // begcmgroup/endcmgroup never reach permmatches and `nmatches`
    // stays 0.
    let yl = ylist.unwrap_or_default();

    // c:3131 — in C this is a one-liner (`mgroup->ylist = ylist`): `matches`
    // etc. already ARE `mgroup->lmatches` (aliased in begcmgroup), so nothing
    // to flush. The port now mirrors that — the group's `l*` are shared
    // `Arc`s the file-scope handles point at, so `compadd`'s appends are
    // already in the group. `new_` is shared the same way now
    // (comp_h.rs `Cmgroup::new_`), so the only per-clone SCALAR field left
    // to copy is `ylist`.

    // Identify the current group and record ylist on the mgroup holder.
    let (name, flags) = {
        let mc = mgroup.get_or_init(|| Mutex::new(None));
        match mc.lock() {
            Ok(mut g) => match g.as_mut() {
                Some(grp) => {
                    grp.ylist = yl.clone(); // c:3140
                    (grp.name.clone(), grp.flags)
                }
                None => return,
            },
            Err(_) => return,
        }
    };

    // Does the closing group carry any live content? Read the SHARED `l*`
    // Arcs (via handles that drop their guard before we lock the inner Vec,
    // so no lock is held into the `amatches` block below → no deadlock).
    // `newmatches` must be marked whenever a group holds matches OR an
    // explanation-only message (`_message -e`), else permmatches early-returns
    // on its stale cache and the group never displays.
    let flushed_any = !crate::comp_match_handles::matches_arc()
        .lock()
        .unwrap()
        .is_empty()
        || !crate::comp_match_handles::fmatches_arc()
            .lock()
            .unwrap()
            .is_empty()
        || !crate::comp_match_handles::expls_arc()
            .lock()
            .unwrap()
            .is_empty();

    let mask = CGF_NOSORT | CGF_UNIQALL | CGF_UNIQCON | CGF_MATSORT | CGF_NUMSORT | CGF_REVSORT;
    // Copy ONLY the scalar field to the amatches entry; its `l*` and `new_`
    // are the same allocations as this group's, already carrying the
    // appended matches and the dirty flag.
    if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        if let Some(grp) = g
            .iter_mut()
            .find(|grp| grp.name == name && (grp.flags & mask) == (flags & mask))
        {
            grp.ylist = yl;
        }
    }
    if flushed_any {
        newmatches.store(1, Ordering::Relaxed);
    }
}

// =====================================================================
// addexpl — `Src/Zle/compcore.c:3140`.
// =====================================================================

/// Port of `mod_export void addexpl(int always)` from compcore.c:3140.
pub fn addexpl(always: bool) {
    // c:3140
    let curexpl_snap = {
        let cell = curexpl.get_or_init(|| Mutex::new(None));
        cell.lock().ok().and_then(|g| g.clone())
    };
    let curexpl_str = match curexpl_snap.as_ref().and_then(|e| e.str.clone()) {
        Some(s) => s,
        None => return,
    };
    let curexpl_count = curexpl_snap.as_ref().map(|e| e.count).unwrap_or(0);
    let curexpl_fcount = curexpl_snap.as_ref().map(|e| e.fcount).unwrap_or(0);

    let elist = crate::comp_match_handles::expls_arc();
    if let Ok(mut g) = elist.lock() {
        for e in g.iter_mut() {
            // c:3145
            if e.str.as_deref() == Some(curexpl_str.as_str()) {
                // c:3147
                e.count += curexpl_count; // c:3148
                e.fcount += curexpl_fcount; // c:3149
                if always {
                    // c:3150
                    e.always = 1;
                    nmessages.fetch_add(1, Ordering::Relaxed); // c:3152
                    newmatches.store(1, Ordering::Relaxed); // c:3153
                    let mc = mgroup.get_or_init(|| Mutex::new(None));
                    if let Ok(mg) = mc.lock() {
                        if let Some(grp) = mg.as_ref() {
                            grp.new_.store(1, Ordering::Relaxed); // c:3154
                        }
                    }
                }
                return; // c:3156
            }
        }
        if let Some(e) = curexpl_snap {
            // c:3159
            g.push(e);
        }
    }
    newmatches.store(1, Ordering::Relaxed); // c:3160
    if always {
        // c:3161
        let mc = mgroup.get_or_init(|| Mutex::new(None));
        if let Ok(mg) = mc.lock() {
            if let Some(grp) = mg.as_ref() {
                grp.new_.store(1, Ordering::Relaxed); // c:3162
            }
        }
        nmessages.fetch_add(1, Ordering::Relaxed); // c:3173
    }
}

// =====================================================================
// matchcmp — `Src/Zle/compcore.c:3173`.
// =====================================================================

/// Port of `static int matchcmp(Cmatch *a, Cmatch *b)` from
/// compcore.c:3173.
pub fn matchcmp(a: &Cmatch, b: &Cmatch) -> std::cmp::Ordering {
    // c:3173
    let order = MATCHORDER.load(Ordering::Relaxed);
    let sortdir = if (order & CGF_REVSORT) != 0 { -1 } else { 1 }; // c:3177

    let cmp = (b.disp.is_some() as i32) - (a.disp.is_some() as i32); // c:3176
                                                                     // c:3175 — `const char *as, *bs;`. C assigns POINTERS here
                                                                     // (c:3181-3182 / c:3191-3192); the comparator allocates nothing.
                                                                     // The port used `.clone()`, so every one of the O(n log n)
                                                                     // comparisons heap-allocated two Strings — 46765 matches
                                                                     // (`compadd -k functions` under a real .zcompdump) means ~725k
                                                                     // comparisons and ~1.45M allocations per sort. `as_deref()` is
                                                                     // the direct analogue of C's `char *` assignment.
    let (as_, bs) = if (order & CGF_MATSORT) != 0 || (cmp == 0 && a.disp.is_none()) {
        (
            a.str.as_deref().unwrap_or(""), // c:3181
            b.str.as_deref().unwrap_or(""),
        ) // c:3182
    } else {
        // c:3183-3184 / c:3186-3188 — C returns these two orderings RAW
        // (`return cmp;`); `sortdir` is applied only to the final `zstrcmp`
        // at c:3194. The port multiplied both early returns by `sortdir`, so
        // under CGF_REVSORT (`compadd -R`/reverse sort) the structural
        // groupings inverted too: matches WITHOUT display strings sorted
        // ahead of matches with them, and one-per-line display strings sank
        // to the bottom of the list instead of heading it.
        if cmp != 0 {
            return if cmp < 0 {
                std::cmp::Ordering::Less // c:3184
            } else {
                std::cmp::Ordering::Greater
            };
        }
        let displine_cmp = (b.flags & CMF_DISPLINE) - (a.flags & CMF_DISPLINE); // c:3186
        if displine_cmp != 0 {
            return if displine_cmp < 0 {
                std::cmp::Ordering::Less // c:3188
            } else {
                std::cmp::Ordering::Greater
            };
        }
        (
            a.disp.as_deref().unwrap_or(""), // c:3191
            b.disp.as_deref().unwrap_or(""),
        ) // c:3192
    };
    // c:3195-3197 — `sortdir * zstrcmp(as, bs, SORTIT_IGNORING_BACKSLASHES |
    // ((isset(NUMERICGLOBSORT) || matchorder & CGF_NUMSORT) ?
    //   SORTIT_NUMERICALLY : 0))`. zstrcmp routes through strcoll, so the
    // ordering is the locale's collation (case-insensitive on the usual
    // macOS/Linux locales) — matching zsh. The previous byte comparison
    // sorted ASCII, putting every uppercase name (`README.md`) ahead of
    // lowercase ones (`alpha.txt`), which diverged from zsh's completion
    // order.
    let numeric = crate::ported::zsh_h::isset(crate::ported::zsh_h::NUMERICGLOBSORT)
        || (order & CGF_NUMSORT) != 0;
    let flags = crate::ported::zsh_h::SORTIT_IGNORING_BACKSLASHES as u32
        | if numeric {
            crate::ported::zsh_h::SORTIT_NUMERICALLY as u32
        } else {
            0
        };
    let base = crate::ported::sort::zstrcmp(as_, bs, flags);
    if sortdir < 0 {
        base.reverse()
    } else {
        base
    }
}

/// Port of `static int matcheq(Cmatch a, Cmatch b)` from
/// compcore.c:3206.
pub fn matcheq(a: &Cmatch, b: &Cmatch) -> bool {
    // c:3207
    matchstreq(a.ipre.as_ref(),  b.ipre.as_ref())  &&                        // c:3207
    matchstreq(a.pre.as_ref(),   b.pre.as_ref())   &&                        // c:3210
    matchstreq(a.ppre.as_ref(),  b.ppre.as_ref())  &&                        // c:3211
    matchstreq(a.psuf.as_ref(),  b.psuf.as_ref())  &&                        // c:3212
    matchstreq(a.suf.as_ref(),   b.suf.as_ref())   &&                        // c:3213
    matchstreq(a.str.as_ref(),  b.str.as_ref()) // c:3214
}

// =====================================================================
// makearray — `Src/Zle/compcore.c:3224`.
// =====================================================================

/// Port of `static Cmatch *makearray(LinkList l, int type, int flags,
///                                    int *np, int *nlp, int *llp)`
/// from compcore.c:3223. Returns `(arr, n, nl, ll)`.
///
/// `type` is fixed to `1` (match-sort path) for the in-file call sites
/// from `permmatches`. The `type=0` string-sort path on `lexpls` is
/// inlined at the `permmatches` call site (C uses a `(char **)` cast
/// trick that has no safe Rust equivalent).
pub fn makearray(mut rp: Vec<Cmatch>, flags: i32) -> (Vec<Cmatch>, i32, i32, i32) {
    // c:3224
    let mut n: i32 = rp.len() as i32; // c:3224
    let mut nl: i32 = 0; // c:3231
    let mut ll: i32 = 0; // c:3231

    if n > 0 {
        // c:3258 (type==1 branch)
        if (flags & CGF_NOSORT) == 0 {
            // c:3259
            // Now sort the array (it contains matches).                     // c:3260
            MATCHORDER.store(flags, Ordering::Relaxed); // c:3261
                                                        // c:3262 — C `qsort(rp, n, sizeof(Cmatch), matchcmp)`. Must use the
                                                        // qsort-tolerant sort: matchcmp→zstrcmp is not a strict weak order
                                                        // (numeric/natural sort), which makes Rust's sort_by PANIC.
            crate::tolerant_sort::qsort_tolerant(&mut rp, matchcmp);

            if (flags & CGF_UNIQCON) == 0 {
                // c:3269 not -2
                // remove dupes
                let mut cp = 0usize; // c:3272
                let mut ap = 0usize;
                while ap < rp.len() {
                    // c:3274 for ap;*ap;ap++
                    // Scan the run of duplicates FIRST, using the element at
                    // `ap` (C keeps `*ap` stable — `*cp++ = *ap` copies, it
                    // does not move). Doing `rp.swap(ap, cp)` before this scan
                    // (the old code) overwrote `rp[ap]` with a stale slot once
                    // any earlier removal made `cp < ap`, so `rp[ap].str ==
                    // rp[bp+1].str` compared the wrong element and same-string
                    // duplicates (e.g. `libpng-config` from several $path dirs)
                    // were never collapsed.
                    // c:3271 — collapse the run of matcheq duplicates.
                    let mut bp = ap;
                    while bp + 1 < rp.len() && matcheq(&rp[ap], &rp[bp + 1]) {
                        bp += 1;
                        n -= 1; // c:3271 bp[1] && matcheq
                    }
                    // c:3272 — `ap = bp`: from here C compares against the
                    // LAST element of the collapsed run.
                    let run_end = bp;
                    // c:3274-3278 — mark (do NOT remove) the following
                    // elements that are not matcheq but would DISPLAY the
                    // same string. C advances only `bp` here; `n` is not
                    // decremented and the outer loop resumes at `run_end + 1`,
                    // so every CMF_MULT element stays in the array and in
                    // `mcount` — it is excluded from the *listing* later via
                    // `nl` (c:3287-3288 → `lcount = nn - nl`). The port
                    // decremented `n` and skipped past them, deleting them
                    // outright: `mcount` under-counted, so a word with two
                    // same-string matches from different path prefixes looked
                    // like a single unambiguous match and was inserted
                    // without ever offering the choice.
                    let mut mark = run_end;
                    let mut dup = 0i32; // c:3274
                    while mark + 1 < rp.len()
                        && rp[run_end].disp.is_none()
                        && rp[mark + 1].disp.is_none()                       // c:3274 !disp
                        && rp[run_end].str == rp[mark + 1].str
                    {
                        rp[mark + 1].flags |= CMF_MULT; // c:3276
                        dup = 1; // c:3277
                        mark += 1;
                    }
                    if dup != 0 {
                        // c:3279
                        rp[run_end].flags |= CMF_FMULT; // c:3280
                    }
                    // c:3270 `*cp++ = *ap` — keep the first of the run at `cp`.
                    if ap != cp {
                        rp.swap(ap, cp);
                    }
                    cp += 1;
                    ap = run_end + 1; // c:3272 ap = bp; then outer ap++
                }
                rp.truncate(cp); // c:3282 *cp = NULL
            }
            for m in rp.iter() {
                // c:3293
                if m.disp.is_some() && (m.flags & CMF_DISPLINE) != 0 {
                    // c:3294
                    ll += 1;
                }
                if (m.flags & (CMF_NOLIST | CMF_MULT)) != 0 {
                    // c:3296
                    nl += 1;
                }
            }
        } else {
            // c:3300 used -O nosort or -V
            if (flags & CGF_UNIQALL) == 0 && (flags & CGF_UNIQCON) == 0 {
                // c:3302 didn't use -1 or -2
                MATCHORDER.store(flags, Ordering::Relaxed); // c:3306
                let mut sp: Vec<Cmatch> = rp.clone(); // c:3309-3312 zhalloc + memcpy
                                                      // c:3313 — qsort matchcmp; tolerant sort (non-total-order cmp).
                crate::tolerant_sort::qsort_tolerant(&mut sp, matchcmp);

                let mut del = false; // c:3303
                                     // Sweep sorted dup-detection back onto rp via flag marks.
                for w in sp.windows(2) {
                    // c:3315-3329
                    if matcheq(&w[0], &w[1]) {
                        // Mark in original rp by str+disp equality.
                        for m in rp.iter_mut() {
                            if matcheq(m, &w[1]) {
                                m.flags = CMF_DELETE; // c:3318
                                del = true; // c:3319
                                break;
                            }
                        }
                    } else if w[0].disp.is_none() {
                        if w[1].disp.is_none() && w[0].str == w[1].str {
                            // c:3322
                            for m in rp.iter_mut() {
                                if matcheq(m, &w[1]) {
                                    m.flags |= CMF_MULT; // c:3324
                                    break;
                                }
                            }
                            for m in rp.iter_mut() {
                                if matcheq(m, &w[0]) {
                                    m.flags |= CMF_FMULT; // c:3328
                                    break;
                                }
                            }
                        }
                    }
                }
                if del {
                    // c:3332
                    rp.retain(|m| (m.flags & CMF_DELETE) == 0); // c:3334-3340
                    n = rp.len() as i32;
                }
            } else if (flags & CGF_UNIQCON) == 0 {
                // c:3344 -1 not -2
                let mut cp = 0usize;
                let mut ap = 0usize;
                while ap < rp.len() {
                    // c:3334
                    // Scan the runs BEFORE the compaction swap: once an
                    // earlier removal makes `cp < ap`, `rp.swap(ap, cp)`
                    // overwrites `rp[ap]` with a stale slot and every
                    // comparison below reads the wrong element. C copies
                    // (`*cp++ = *ap`), it does not swap. Same fix as the
                    // sorted branch above.
                    let mut bp = ap;
                    while bp + 1 < rp.len() && matcheq(&rp[ap], &rp[bp + 1]) {
                        bp += 1;
                        n -= 1; // c:3336
                    }
                    let run_end = bp; // c:3337 `ap = bp`
                                      // c:3338-3342 — mark, do NOT remove: `n` is untouched and
                                      // the outer loop resumes at `run_end + 1`, so CMF_MULT
                                      // elements stay in the array (they are dropped from the
                                      // LISTING via `nl` at c:3351-3352, not from `mcount`).
                    let mut mark = run_end;
                    let mut dup = 0i32;
                    while mark + 1 < rp.len()
                        && rp[run_end].disp.is_none()
                        && rp[mark + 1].disp.is_none()
                        && rp[run_end].str == rp[mark + 1].str
                    {
                        rp[mark + 1].flags |= CMF_MULT; // c:3340
                        dup = 1; // c:3341
                        mark += 1;
                    }
                    if dup != 0 {
                        rp[run_end].flags |= CMF_FMULT; // c:3344
                    }
                    if ap != cp {
                        rp.swap(ap, cp); // c:3335 `*cp++ = *ap`
                    }
                    cp += 1;
                    ap = run_end + 1;
                }
                rp.truncate(cp); // c:3346
            }
            for m in rp.iter() {
                // c:3361
                if m.disp.is_some() && (m.flags & CMF_DISPLINE) != 0 {
                    // c:3362
                    ll += 1;
                }
                if (m.flags & (CMF_NOLIST | CMF_MULT)) != 0 {
                    // c:3364
                    nl += 1;
                }
            }
        }
    }
    (rp, n, nl, ll) // c:3366-3373
}

// =====================================================================
// dupmatch — `Src/Zle/compcore.c:3370`.
// =====================================================================

/// Port of `static Cmatch dupmatch(Cmatch m, int nbeg, int nend)` from
/// compcore.c:3370. Deep-copies one match; brpl/brsl are truncated to
/// nbeg/nend per the C body's nbeg/nend-sized `zalloc` + element copy.
pub fn dupmatch(m: &Cmatch, nbeg: i32, nend: i32) -> Cmatch {
    // c:3370
    let mut r = Cmatch::default(); // c:3370-3374
    r.str = m.str.clone(); // c:3376 ztrdup
    r.orig = m.orig.clone(); // c:3377
    r.ipre = m.ipre.clone(); // c:3378
    r.ripre = m.ripre.clone(); // c:3379
    r.isuf = m.isuf.clone(); // c:3380
    r.ppre = m.ppre.clone(); // c:3381
    r.psuf = m.psuf.clone(); // c:3382
    r.prpre = m.prpre.clone(); // c:3383
    r.pre = m.pre.clone(); // c:3384
    r.suf = m.suf.clone(); // c:3385
    r.flags = m.flags; // c:3386
    if !m.brpl.is_empty() {
        // c:3387
        let take = (nbeg as usize).min(m.brpl.len()); // c:3390 zalloc(nbeg)
        r.brpl = m.brpl[..take].to_vec(); // c:3392 element-wise copy
    } else {
        r.brpl = Vec::new(); // c:3395 NULL
    }
    if !m.brsl.is_empty() {
        // c:3396
        let take = (nend as usize).min(m.brsl.len()); // c:3399
        r.brsl = m.brsl[..take].to_vec(); // c:3401
    } else {
        r.brsl = Vec::new(); // c:3404
    }
    r.rems = m.rems.clone(); // c:3405
    r.remf = m.remf.clone(); // c:3406
    r.autoq = m.autoq.clone(); // c:3407
    r.qipl = m.qipl; // c:3408
    r.qisl = m.qisl; // c:3409
    r.disp = m.disp.clone(); // c:3410
    r.mode = m.mode; // c:3411
    r.modec = m.modec; // c:3412
    r.fmode = m.fmode; // c:3413
    r.fmodec = m.fmodec; // c:3414
    r // c:3416
}

/// Port of `mod_export int permmatches(int last)` from compcore.c:3422.
/// Promotes the per-round `amatches` accumulator into the permanent
/// `pmatches` snapshot via deep-copy through `dupmatch`/`makearray`.
pub fn permmatches(last: i32) -> i32 {
    // c:3423
    let ofi = PERMMATCHES_FI.load(Ordering::Relaxed); // c:3423 ofi = fi

    // c:3433 — `if (pmatches && !newmatches)`
    let pmatches_set = pmatches
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|g| !g.is_empty())
        .unwrap_or(false);
    if pmatches_set && newmatches.load(Ordering::Relaxed) == 0 {
        // c:3433
        if last != 0 && PERMMATCHES_FI.load(Ordering::Relaxed) != 0 {
            // c:3434
            // ainfo = fainfo                                                // c:3435
            let famref = fainfo
                .get_or_init(|| Mutex::new(None))
                .lock()
                .ok()
                .and_then(|g| g.clone());
            if let Ok(mut a) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
                *a = famref;
            }
        }
        return PERMMATCHES_FI.load(Ordering::Relaxed); // c:3437
    }
    newmatches.store(0, Ordering::Relaxed); // c:3439
    PERMMATCHES_FI.store(0, Ordering::Relaxed); // c:3439 fi = 0

    {
        // pmatches = lmatches = NULL                                        // c:3441
        if let Ok(mut g) = pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            g.clear();
        }
        if let Ok(mut g) = lmatches.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
    }
    nmatches.store(0, Ordering::Relaxed); // c:3442
    smatches.store(0, Ordering::Relaxed); // c:3442
    diffmatches.store(0, Ordering::Relaxed); // c:3442

    // c:3444 — `if (!ainfo->count)`.
    let ainfo_count = ainfo
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|a| a.count))
        .unwrap_or(0);
    if ainfo_count == 0 {
        // c:3444
        if last != 0 {
            // c:3445
            let famref = fainfo
                .get_or_init(|| Mutex::new(None))
                .lock()
                .ok()
                .and_then(|g| g.clone());
            if let Ok(mut a) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
                *a = famref;
            }
        }
        PERMMATCHES_FI.store(1, Ordering::Relaxed); // c:3447
    }

    let nbeg = NBRBEG.load(Ordering::Relaxed);
    let nend = NBREND.load(Ordering::Relaxed);

    let mut gn: i32 = 1; // c:3429 gn = 1
    let mut mn: i32 = 1; // c:3429 mn = 1
    let fi = PERMMATCHES_FI.load(Ordering::Relaxed);

    let groups_snapshot: Vec<Cmgroup> = {
        amatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_default()
    };
    let mut new_pmatches: Vec<Cmgroup> = Vec::with_capacity(groups_snapshot.len());
    // c:3488 `g->perm = n`, c:3460-3467 `g->mcount/lcount/llcount`, c:3471
    // `g->ccount = 0`, c:3542 `g->num = gn++` and c:3544 `g->new = 0` are all
    // writes to the LIVE group in `amatches`. The port mutated a detached
    // clone and threw it away, so `g->new` never cleared and `g->perm` was
    // never recorded: the c:3452 cache test (`!g->perm || g->new`) could
    // never take its reuse branch and every `permmatches` call rebuilt and
    // re-`dupmatch`ed every group from scratch. Collect the mutated groups
    // and store them back after the loop.
    let mut updated_groups: Vec<Cmgroup> = Vec::with_capacity(new_pmatches.capacity());
    // c:3490-3491 / c:3528-3529 — `if (!lmatches) lmatches = n;` (resp.
    // `g->perm`). `lmatches` is the head C keeps for `makecomplist`'s
    // old-list reuse (c:1006 `lmatches = lastlmatches`); the port never
    // assigned it, so `compstate[old_list]=keep` had no list to restore.
    let mut lmatches_head: Option<Cmgroup> = None;

    for g_orig in groups_snapshot.into_iter() {
        // c:3449 while (g)
        let mut g = g_orig; // borrow-mut snapshot
        let must_rebuild = fi != ofi || g.perm.is_none() || g.new_.load(Ordering::Relaxed) != 0; // c:3456
        if must_rebuild {
            // c:3456
            let src_list = if fi != 0 {
                g.lfmatches.lock().unwrap().clone()
            }
            // c:3457
            else {
                g.lmatches.lock().unwrap().clone()
            }; // c:3461

            let (arr, nn, nl, ll) = makearray(src_list, g.flags); // c:3463
            g.mcount = nn; // c:3464
            g.lcount = nn - nl; // c:3465
            if g.lcount < 0 {
                g.lcount = 0;
            } // c:3466
            g.llcount = ll; // c:3467
            if !g.ylist.is_empty() {
                // c:3468
                g.lcount = g.ylist.len() as i32; // c:3469
                smatches.store(2, Ordering::Relaxed); // c:3470
            }
            // c:3472 — makearray(lexpls, 0, 0, &ecount, NULL, NULL).
            let mut exps = g.lexpls.lock().unwrap().clone(); // type=0 path
            g.ecount = exps.len() as i32;
            // c:3475 ccount = 0
            g.ccount = 0; // c:3475
            tracing::debug!(
                target: "compsys_args",
                mcount = g.mcount,
                lcount = g.lcount,
                nn,
                nl,
                ll,
                name = ?g.name,
                "permmatches group"
            );
            nmatches.fetch_add(g.mcount, Ordering::Relaxed); // c:3477
            smatches.fetch_add(g.lcount, Ordering::Relaxed); // c:3478
            if g.mcount > 1 {
                // c:3480
                diffmatches.store(1, Ordering::Relaxed); // c:3481
            }

            // n = (Cmgroup) zshcalloc(...)                                  // c:3483
            let mut n_grp = Cmgroup::default();
            // c:3487 — `if (g->perm) freematches(g->perm, 0)`. Drop on
            // perm Box<Cmgroup> reclaims the C `free` path.
            g.perm = None; // c:3490 g->perm = n
                           // Then below we set g.perm = Some(Box::new(n_grp.clone())).

            n_grp.num = gn;
            gn += 1; // c:3499
            n_grp.flags = g.flags; // c:3500
            n_grp.mcount = g.mcount; // c:3501
            n_grp.matches = arr
                .iter() // c:3502-3505 dupmatch loop
                .map(|m| dupmatch(m, nbeg, nend))
                .collect();
            n_grp.name = g.name.clone(); // c:3504
            n_grp.lcount = g.lcount; // c:3508
            n_grp.llcount = g.llcount; // c:3509
            if !g.ylist.is_empty() {
                // c:3510
                n_grp.ylist = g.ylist.clone(); // c:3511 zarrdup
            } else {
                n_grp.ylist = Vec::new(); // c:3513
            }
            if g.ecount != 0 {
                // c:3515
                // Build n->expls from g->expls deep-copying str + (fi
                // ? fcount : count); always carries over; fcount = 0.
                n_grp.expls = exps
                    .drain(..)
                    .map(|o| Cexpl {
                        // c:3517-3525
                        count: if fi != 0 { o.fcount } else { o.count }, // c:3520
                        always: o.always,                                // c:3521
                        fcount: 0,                                       // c:3522
                        str: o.str.clone(),                              // c:3523 ztrdup
                    })
                    .collect();
                n_grp.ecount = g.ecount;
            } else {
                n_grp.expls = Vec::new(); // c:3528
            }
            n_grp.widths = Vec::new(); // c:3531
                                       // Stitch perm chain (prev/next handled implicitly by Vec).
            g.matches = arr; // mirror C: g->matches = makearray result
            g.perm = Some(Box::new(n_grp.clone())); // c:3488 g->perm = n
            if lmatches_head.is_none() {
                lmatches_head = Some(n_grp.clone()); // c:3490-3491
            }
            new_pmatches.push(n_grp); // c:3492-3495
        } else {
            // reuse existing g->perm                                        // c:3534
            nmatches.fetch_add(g.mcount, Ordering::Relaxed); // c:3540
            smatches.fetch_add(g.lcount, Ordering::Relaxed); // c:3541
            if g.mcount > 1 {
                diffmatches.store(1, Ordering::Relaxed); // c:3543
            }
            g.num = gn;
            gn += 1; // c:3542
            if let Some(p) = g.perm.as_deref() {
                if lmatches_head.is_none() {
                    lmatches_head = Some(p.clone()); // c:3528-3529
                }
                new_pmatches.push(p.clone()); // c:3533 pmatches = g->perm
            }
        }
        g.new_.store(0, Ordering::Relaxed); // c:3544
        updated_groups.push(g);
    }
    // c:3488/3542/3544 write-back — see the note above the loop.
    if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        *g = updated_groups;
    }
    // c:3490 — `lmatches` is the head of the permanent chain.
    if let Ok(mut g) = lmatches.get_or_init(|| Mutex::new(None)).lock() {
        *g = lmatches_head;
    }

    // c:3490-3538 — C threads each group onto `pmatches` via
    // `n->next = pmatches; pmatches = n` (a PREPEND), so pmatches ends up
    // in the REVERSE of amatches iteration order. The port accumulates into
    // a Vec with push() (append), so reverse once here to recover C's order.
    // Without this, `group-order`/`group-name` groups list in creation order
    // reversed (e.g. `group-order veggies fruits` showed fruits first), and
    // the gnum/rnum numbering below — which C runs over the reversed
    // pmatches — was assigned against the wrong sequence.
    new_pmatches.reverse();

    // c:3551-3563 — assign rnum/gnum, recompute diffmatches/nbrbeg.
    let mut first_first: Option<Cmatch> = None;
    for g_pm in new_pmatches.iter_mut() {
        g_pm.nbrbeg = nbeg; // c:3552
        g_pm.nbrend = nend; // c:3553
        let mut rn = 1i32; // c:3554
        for m in g_pm.matches.iter_mut() {
            m.rnum = rn;
            rn += 1; // c:3555
            m.gnum = mn;
            mn += 1; // c:3556
        }
        if diffmatches.load(Ordering::Relaxed) == 0 && !g_pm.matches.is_empty() {
            match first_first.as_ref() {
                // c:3558
                Some(p0) => {
                    if !matcheq(&g_pm.matches[0], p0) {
                        diffmatches.store(1, Ordering::Relaxed); // c:3560
                    }
                }
                None => first_first = Some(g_pm.matches[0].clone()), // c:3562
            }
        }
    }

    if let Ok(mut g) = pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        *g = new_pmatches;
    }

    hasperm.store(1, Ordering::Relaxed); // c:3565
    permmnum.store(mn - 1, Ordering::Relaxed); // c:3566
    permgnum.store(gn - 1, Ordering::Relaxed); // c:3567
    if let Ok(mut ld) = listdat
        .get_or_init(|| Mutex::new(Default::default()))
        .lock()
    {
        ld.valid = 0; // c:3568
    }

    fi // c:3570
}

// =====================================================================
// freematch / freematches — `Src/Zle/compcore.c:3575 / 3605`.
// =====================================================================

/// Port of `static void freematch(Cmatch m, int nbeg, int nend)` from
/// `Src/Zle/compcore.c:3575`. C frees each Cmatch field via `zsfree`
/// (str/orig/ipre/ripre/isuf/ppre/psuf/pre/suf/prpre/rems/remf/disp/
/// autoq) and `zfree(m->brpl, nbeg * sizeof(int))` /
/// `zfree(m->brsl, nend * sizeof(int))` — all collapse to Rust's
/// automatic Drop on the owned String / `Vec<i32>` fields. nbeg/nend
/// kept on the signature for C parity (consumed by C as `zfree` size
/// args; Rust Vec carries its own length).
pub fn freematch(m: Cmatch, _nbeg: i32, _nend: i32) {
    // c:3577 — `if (!m) return;` — Rust's owned `m` is always valid;
    // dropping it on return runs every field's destructor (c:3579-3596
    // zsfree / zfree calls collapsed).
    drop(m); // c:3598 zfree(m)
}

/// Direct port of `mod_export void freematches(Cmgroup g, int cm)` from
/// `Src/Zle/compcore.c:3605`. The C path walks the cmgroup chain freeing
/// each Cmatch + ylist + expls + widths + name; in Rust those are
/// owned by `Vec`/`Box`/`String` so Drop covers the per-node free.
/// The `cm` arm at c:3636-3637 (`minfo.cur = NULL`) is the only
/// side-effect that doesn't fall out of Rust's ownership model — wire
/// it explicitly.
pub fn freematches(g: Vec<Cmgroup>, cm: i32) {
    // c:3605
    drop(g);
    if cm != 0 {
        // c:3636
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(Menuinfo::default())).lock() {
            g.cur = None; // c:3637
        }
    }
}

// =====================================================================
// Extern globals — declared in other C files, mirrored here per
// PORT.md Rule 9 ("stub the EXTERN dependencies ... locally with
// file:line citations to their home file") so the local body ports
// below have a value source. When the canonical Rust homes land,
// these become `pub use crate::ported::<canonical>::*` re-exports.
// =====================================================================

/// Port of `mod_export int lastend` from `Src/Zle/compcore.c:276`.
/// Byte position in the metafied line where the most-recent
/// completion insertion ended.
///
/// Re-export alias of [`lastend`] — C has ONE `lastend` (compcore.c:276).
/// Two atomics existed: `compresult` wrote the lowercase `lastend`, while
/// `complist`'s menu-list positioning read this uppercase `LASTEND`, which was
/// never stored (always 0) — so menu-selection column math used a stale zero.
pub use self::lastend as LASTEND;

/// Port of `mod_export int wb` from `Src/lex.c:120`. Word-begin
/// position in the metafied line for the currently-completing word.
pub static WB: AtomicI32 = AtomicI32::new(0); // lex.c:120
/// Port of `mod_export int we` from `Src/lex.c:120`. Word-end position.
pub static WE: AtomicI32 = AtomicI32::new(0); // lex.c:120
/// Port of `mod_export int zlemetacs` from `Src/lex.c:104`. Cursor
/// position in the metafied line.
pub static ZLEMETACS: AtomicI32 = AtomicI32::new(0); // lex.c:104
/// Port of `mod_export int zlemetall` from `Src/lex.c:104`. Length
/// of the metafied line.
pub static ZLEMETALL: AtomicI32 = AtomicI32::new(0); // lex.c:104
/// Port of `mod_export int addedx` from `Src/lex.c:115`. Non-zero
/// while a dummy `x` cursor marker is in the line being lexed
/// (so completion can capture the partial word at the cursor).
pub static ADDEDX: AtomicI32 = AtomicI32::new(0); // lex.c:115

/// Port of `mod_export char *zlemetaline` from `Src/lex.c:103`. The
/// metafied edit buffer for the current ZLE session — `foredel`,
/// `inststr`, `selfinsert` operate on this directly when compcore's
/// error-recovery path fires (compcore.c:344-355).
pub static ZLEMETALINE: OnceLock<Mutex<String>> = OnceLock::new(); // lex.c:103
/// Port of `mod_export ZLE_STRING_T zleline` from `Src/zle_main.c`.
pub static ZLELINE: OnceLock<Mutex<String>> = OnceLock::new(); // zle_main.c
/// Port of `mod_export int zlecs` from `Src/zle_main.c`.
pub static ZLECS: AtomicI32 = AtomicI32::new(0); // zle_main.c
/// Port of `mod_export int zlell` from `Src/zle_main.c`.
pub static ZLELL: AtomicI32 = AtomicI32::new(0); // zle_main.c
/// Port of `mod_export int inwhat` from `Src/lex.c:110`. Lex context
/// classification — IN_NOTHING / IN_CMD / IN_COND / IN_MATH / IN_PAR /
/// IN_ENV.
pub static INWHAT: AtomicI32 = AtomicI32::new(0); // lex.c:110
/// Port of `mod_export int zmult` from `Src/zle_main.c`. Numeric
/// prefix multiplier for the current ZLE command.
pub static ZMULT: AtomicI32 = AtomicI32::new(1); // zle_main.c
/// Port of `mod_export char *compfunc` from `Src/Zle/zle_tricky.c:143`.
/// Name of the user completion shell function — non-empty when the
/// new completion system (`compsys`) is active; empty for compctl.
pub static compfunc: OnceLock<Mutex<Option<String>>> = OnceLock::new(); // zle_tricky.c:143
/// Port of `mod_export char *comppatmatch` from `Src/Zle/zle_tricky.c`.
/// `$compstate[pattern_match]` — when non-empty + non-"\0" enables
/// pattern-aware matching for parameter-name completion.
pub static comppatmatch: OnceLock<Mutex<Option<String>>> = OnceLock::new();
// `compqstack` (C: complete.c `mod_export char *compqstack`) is deduped
// to the single canonical `complete::COMPQSTACK`, imported at the top of
// this module. The former compcore-local `compqstack` static was an
// orphan: the c:305 reset wrote it while `multiquote` (c:1065) read the
// imported COMPQSTACK, so the reset never reached its reader — a real
// bug now fixed by pointing both at COMPQSTACK.
// =====================================================================
// File-scope globals — `Src/Zle/compcore.c:36-279`.
// =====================================================================

/// Port of `int useexact` from compcore.c:36.
pub static useexact: AtomicI32 = AtomicI32::new(0); // c:36
/// Port of `int useline` from compcore.c:36.
pub static useline: AtomicI32 = AtomicI32::new(0); // c:36
/// Port of `int uselist` from compcore.c:36.
pub static uselist: AtomicI32 = AtomicI32::new(0); // c:36
/// Port of `int forcelist` from compcore.c:36.
pub static forcelist: AtomicI32 = AtomicI32::new(0); // c:36
/// Port of `int startauto` from compcore.c:36.
pub static startauto: AtomicI32 = AtomicI32::new(0); // c:36

/// Port of `mod_export int iforcemenu` from compcore.c:39.
pub static iforcemenu: AtomicI32 = AtomicI32::new(0); // c:39

/// Port of `mod_export int dolastprompt` from compcore.c:44.
pub static dolastprompt: AtomicI32 = AtomicI32::new(0); // c:44

/// Port of `mod_export int oldlist` from compcore.c:49.
pub static oldlist: AtomicI32 = AtomicI32::new(0); // c:49
/// Port of `mod_export int oldins` from compcore.c:49.
pub static oldins: AtomicI32 = AtomicI32::new(0); // c:49

/// Port of `int origlpre` from compcore.c:54.
pub static origlpre: AtomicI32 = AtomicI32::new(0); // c:54
/// Port of `int origlsuf` from compcore.c:54.
pub static origlsuf: AtomicI32 = AtomicI32::new(0); // c:54
/// Port of `int lenchanged` from compcore.c:54.
pub static lenchanged: AtomicI32 = AtomicI32::new(0); // c:54

/// Port of `int movetoend` from compcore.c:61.
pub static movetoend: AtomicI32 = AtomicI32::new(0); // c:61

/// Port of `mod_export int insmnum` from compcore.c:66.
pub static insmnum: AtomicI32 = AtomicI32::new(0); // c:66
/// Port of `mod_export int insspace` from compcore.c:66.
pub static insspace: AtomicI32 = AtomicI32::new(0); // c:66

/// Port of `mod_export int menuacc` from compcore.c:81.
pub static menuacc: AtomicI32 = AtomicI32::new(0); // c:81

/// Port of `int hasunqu` from compcore.c:86.
pub static hasunqu: AtomicI32 = AtomicI32::new(0); // c:86
/// Port of `int useqbr` from compcore.c:86.
pub static useqbr: AtomicI32 = AtomicI32::new(0); // c:86
/// Port of `int brpcs` from compcore.c:86.
pub static brpcs: AtomicI32 = AtomicI32::new(0); // c:86
/// Port of `int brscs` from compcore.c:86.
pub static brscs: AtomicI32 = AtomicI32::new(0); // c:86

/// Port of `mod_export int ispar` from compcore.c:91.
pub static ispar: AtomicI32 = AtomicI32::new(0); // c:91
/// Port of `mod_export int linwhat` from compcore.c:91.
pub static linwhat: AtomicI32 = AtomicI32::new(0); // c:91

/// Port of `char *parpre` from compcore.c:96.
pub static parpre: OnceLock<Mutex<String>> = OnceLock::new(); // c:96

/// Port of `int parflags` from compcore.c:101.
pub static parflags: AtomicI32 = AtomicI32::new(0); // c:101

/// Port of `mod_export int mflags` from compcore.c:106.
pub static mflags: AtomicI32 = AtomicI32::new(0); // c:106

/// Port of `int parq` from compcore.c:111.
pub static parq: AtomicI32 = AtomicI32::new(0); // c:111
/// Port of `int eparq` from compcore.c:111.
pub static eparq: AtomicI32 = AtomicI32::new(0); // c:111

/// Port of `mod_export char *ipre` from compcore.c:118.
pub static ipre: OnceLock<Mutex<String>> = OnceLock::new(); // c:118
/// Port of `mod_export char *ripre` from compcore.c:118.
pub static ripre: OnceLock<Mutex<String>> = OnceLock::new(); // c:118
/// Port of `mod_export char *isuf` from compcore.c:118.
pub static isuf: OnceLock<Mutex<String>> = OnceLock::new(); // c:118

/// Port of `mod_export LinkList matches` from compcore.c:124.
pub static matches: OnceLock<Mutex<std::sync::Arc<Mutex<Vec<Cmatch>>>>> = OnceLock::new(); // c:124 (Arc handle — see comp_match_handles)
/// Port of `LinkList fmatches` from compcore.c:126.
pub static fmatches: OnceLock<Mutex<std::sync::Arc<Mutex<Vec<Cmatch>>>>> = OnceLock::new(); // c:126 (Arc handle)

/// Port of `mod_export Cmgroup amatches` from compcore.c:135.
pub static amatches: OnceLock<Mutex<Vec<Cmgroup>>> = OnceLock::new(); // c:135
/// Port of `mod_export Cmgroup pmatches` from compcore.c:135.
pub static pmatches: OnceLock<Mutex<Vec<Cmgroup>>> = OnceLock::new(); // c:135
/// Port of `mod_export Cmgroup lastmatches` from compcore.c:135.
pub static lastmatches: OnceLock<Mutex<Vec<Cmgroup>>> = OnceLock::new(); // c:135
/// Port of `mod_export Cmgroup lmatches` from compcore.c:135. Last
/// element pointer in the perm list; here a single-slot holder.
pub static lmatches: OnceLock<Mutex<Option<Cmgroup>>> = OnceLock::new(); // c:135
/// Port of `mod_export Cmgroup lastlmatches` from compcore.c:135.
pub static lastlmatches: OnceLock<Mutex<Option<Cmgroup>>> = OnceLock::new(); // c:135

/// Port of `mod_export int hasoldlist` from compcore.c:140.
pub static hasoldlist: AtomicI32 = AtomicI32::new(0); // c:140
/// Port of `mod_export int hasperm` from compcore.c:140.
pub static hasperm: AtomicI32 = AtomicI32::new(0); // c:140
/// Port of `int hasallmatch` from compcore.c:145.
pub static hasallmatch: AtomicI32 = AtomicI32::new(0); // c:145

/// Port of `mod_export int newmatches` from compcore.c:150.
pub static newmatches: AtomicI32 = AtomicI32::new(0); // c:150

/// Port of `mod_export int permmnum` from compcore.c:155.
pub static permmnum: AtomicI32 = AtomicI32::new(0); // c:155
/// Port of `mod_export int permgnum` from compcore.c:155.
pub static permgnum: AtomicI32 = AtomicI32::new(0); // c:155
/// Port of `mod_export int lastpermmnum` from compcore.c:155.
pub static lastpermmnum: AtomicI32 = AtomicI32::new(0); // c:155
/// Port of `mod_export int lastpermgnum` from compcore.c:155.
pub static lastpermgnum: AtomicI32 = AtomicI32::new(0); // c:155

/// Port of `mod_export int nmatches` from compcore.c:160.
pub static nmatches: AtomicI32 = AtomicI32::new(0); // c:160
/// Port of `mod_export int smatches` from compcore.c:162.
pub static smatches: AtomicI32 = AtomicI32::new(0); // c:162

/// Port of `mod_export int diffmatches` from compcore.c:167.
pub static diffmatches: AtomicI32 = AtomicI32::new(0); // c:167

/// Port of `mod_export int nmessages` from compcore.c:172.
pub static nmessages: AtomicI32 = AtomicI32::new(0); // c:172

/// Port of `mod_export int onlyexpl` from compcore.c:177.
pub static onlyexpl: AtomicI32 = AtomicI32::new(0); // c:177

/// Port of `mod_export struct cldata listdat` from compcore.c:182.
pub static listdat: OnceLock<Mutex<crate::ported::zle::comp_h::Cldata>> = OnceLock::new(); // c:182

/// Port of `mod_export int ispattern` from compcore.c:187.
pub static ispattern: AtomicI32 = AtomicI32::new(0); // c:187
/// Port of `mod_export int haspattern` from compcore.c:187.
pub static haspattern: AtomicI32 = AtomicI32::new(0); // c:187

/// Port of `mod_export int hasmatched` from compcore.c:192.
pub static hasmatched: AtomicI32 = AtomicI32::new(0); // c:192
/// Port of `mod_export int hasunmatched` from compcore.c:192.
pub static hasunmatched: AtomicI32 = AtomicI32::new(0); // c:192

/// Port of `Cmgroup mgroup` from compcore.c:197.
pub static mgroup: OnceLock<Mutex<Option<Cmgroup>>> = OnceLock::new(); // c:197

/// Port of `mod_export int mnum` from compcore.c:202.
pub static mnum: AtomicI32 = AtomicI32::new(0); // c:202

/// Port of `mod_export int unambig_mnum` from compcore.c:207.
pub static unambig_mnum: AtomicI32 = AtomicI32::new(0); // c:207

/// Port of `int maxmlen` from compcore.c:212.
pub static maxmlen: AtomicI32 = AtomicI32::new(0); // c:212
/// Port of `int minmlen` from compcore.c:212.
pub static minmlen: AtomicI32 = AtomicI32::new(0); // c:212

/// Port of `LinkList expls` from compcore.c:218.
pub static expls: OnceLock<Mutex<std::sync::Arc<Mutex<Vec<Cexpl>>>>> = OnceLock::new(); // c:218 (Arc handle)

/// Port of `mod_export Cexpl curexpl` from compcore.c:221.
pub static curexpl: OnceLock<Mutex<Option<Cexpl>>> = OnceLock::new(); // c:221

/// Port of `LinkList matchers` from compcore.c:236. The C list holds
/// `Cmatcher` pointers (the parsed match-spec chains pushed by
/// addmatches's `add_bmatchers` block). Rust port mirrors that with
/// owned `Box<Cmatcher>` entries.
pub static matchers: OnceLock<Mutex<Vec<Box<crate::ported::zle::comp_h::Cmatcher>>>> =
    OnceLock::new(); // c:236

/// Port of `mod_export Aminfo ainfo` from compcore.c:246.
pub static ainfo: OnceLock<Mutex<Option<Aminfo>>> = OnceLock::new(); // c:246
/// Port of `mod_export Aminfo fainfo` from compcore.c:246.
pub static fainfo: OnceLock<Mutex<Option<Aminfo>>> = OnceLock::new(); // c:246

/// Port of `mod_export LinkList allccs` from compcore.c:259.
pub static allccs: OnceLock<Mutex<std::sync::Arc<Mutex<Vec<String>>>>> = OnceLock::new(); // c:259 (Arc handle)

/// Port of `int fromcomp` from compcore.c:271.
pub static fromcomp: AtomicI32 = AtomicI32::new(0); // c:271

/// Port of `mod_export int lastend` from compcore.c:276.
pub static lastend: AtomicI32 = AtomicI32::new(0); // c:276

/// Port of `mod_export Brinfo brbeg` from `Src/Zle/zle_tricky.c`.
/// Linked list of opening-brace positions in the word being completed.
pub static BRBEG: OnceLock<Mutex<Option<Box<Brinfo>>>> = OnceLock::new(); // zle_tricky.c brbeg

/// Port of `mod_export Brinfo brend` from `Src/Zle/zle_tricky.c`.
/// Linked list of closing-brace positions in the word being completed.
pub static BREND: OnceLock<Mutex<Option<Box<Brinfo>>>> = OnceLock::new(); // zle_tricky.c brend

/// Port of `static int oldmenucmp` from compcore.c:457.
pub static OLDMENUCMP: AtomicI32 = AtomicI32::new(0); // c:457

/// Port of `static int parwb` from compcore.c:540.
pub static PARWB: AtomicI32 = AtomicI32::new(0); // c:540
/// Port of `static int parwe` from compcore.c:540.
pub static PARWE: AtomicI32 = AtomicI32::new(0); // c:540
/// Port of `static int paroffs` from compcore.c:540.
pub static PAROFFS: AtomicI32 = AtomicI32::new(0); // c:540

/// Port of `static int matchorder` from compcore.c:3169.
pub static MATCHORDER: AtomicI32 = AtomicI32::new(0); // c:3169

/// Port of `mod_export int lastchar` from `Src/Zle/zle_main.c`. Last
/// keyboard char consumed by the binding loop — read by `selfinsert`.
pub static LASTCHAR: AtomicI32 = AtomicI32::new(0); // zle_main.c
                                                    // minfo_clear_cur / minfo_asked_zero deleted — Rust-only 2-line
                                                    // wrappers around C's inline writes `minfo.cur = NULL` and
                                                    // `minfo.asked = 0`. All call sites inlined.

/// Direct port of `struct menuinfo minfo` — `Src/Zle/zle_tricky.c`
/// (the single file-scope instance). The struct type itself lives
/// in `comp_h.rs::Menuinfo` (port of comp.h:284-295).
pub static MINFO: OnceLock<Mutex<Menuinfo>> = OnceLock::new(); // zle_tricky.c minfo

// =====================================================================
// matcheq — `Src/Zle/compcore.c:3203-3215`.
// =====================================================================

#[inline]
fn matchstreq(a: Option<&String>, b: Option<&String>) -> bool {
    // c:3207
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// First-match shortcut path from compcore.c:398-411. `Cmgroup m = amatches;
/// while (!m->mcount) m = m->next; do_single(m->matches[0])`.
fn do_single_first_match() {
    // c:398
    let groups = amatches
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let first = groups
        .into_iter()
        .find(|g| g.mcount > 0)
        .and_then(|g| g.matches.first().cloned());
    if let Some(m) = first {
        // c:407 — `minfo.cur = NULL`.
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(Menuinfo::default())).lock() {
            g.cur = None;
            g.asked = 0; // c:408
        }
        // c:409 — `do_single(m->matches[0])`. This is the actual
        // single-match insert: it deletes the word between wb/we (incl.
        // the addx placeholder) and inserts the completed string with
        // its suffix. Previously this call was omitted (the match was
        // merely stashed on minfo.cur), so a unique completion left the
        // typed prefix + 'x' on the line instead of the completed word.
        crate::ported::zle::compresult::do_single(&m);
    }
}

/// compcore.c:444 `compend:` epilogue — free matchers, snap zlemetacs.
fn goto_compend(ret: i32) -> i32 {
    // c:444
    if let Ok(mut g) = matchers.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear(); // c:445-446 freecmatcher loop
    }
    let line_len = ZLEMETALL.load(Ordering::Relaxed); // c:448 strlen(zlemetaline)
    if ZLEMETACS.load(Ordering::Relaxed) > line_len {
        // c:449
        ZLEMETACS.store(line_len, Ordering::Relaxed); // c:450
    }
    ret // c:453
}

// `COMP_LIST_COMPLETE` / `QT_NONE_STUB` / `QT_BACKSLASH_STUB` local
// aliases deleted — call sites now reach the real C-side constants
// directly (`crate::ported::zle::zle_h::COMP_LIST_COMPLETE`,
// `QT_NONE`, `QT_BACKSLASH`).
// The local `COMP_LIST_COMPLETE = 2` was a value-mismatch bug (the
// real constant is 1 per `Src/Zle/zle.h:357`).

// `char_from_qt` deleted — Rust-only 1-line `(qt as u8) as char`
// helper. Inlined at the two call sites in get_compstate_str.

// `showinglist_stub` / `showinglist_set` / `clearlist_set` /
// `listshown_stub` / `instring_stub` deleted — Rust-only 1-line
// accessors for C globals (SHOWINGLIST / CLEARLIST / LISTSHOWN /
// INSTRING). C reads/writes the bare globals inline; callers in
// compcore.rs now do `<GLOBAL>.load(Ordering::Relaxed)` /
// `<GLOBAL>.store(v, Ordering::Relaxed)` directly.
// `fn foredel` / `fn inststr` locals deleted — both duplicated
// canonical ports living in their proper home files:
//   - foredel: `Src/Zle/zle_utils.c:1105`
//     → `foredel`
//   - inststr: macro `Src/Zle/zle_tricky.c:57` (inststr(X) →
//     inststrlen(X,1,-1)) → `inststr`
// The duplicates here narrowed C signatures (foredel dropped `flags`,
// inststr dropped the i32 return + duplicated inststrlen's body) and
// violated Rule C (every decl in its mirroring C file). Callers in
// this module now route through the canonical ported.
/// `IN_NOTHING_LW` constant.
// These MUST match the raw IN_* enum in zsh_h (Src/zsh.h:2322-2332), which is
// the single encoding C uses for both `inwhat` and `linwhat`. `linwhat` is
// copied verbatim from `INWHAT` (makecomplist c:960), so a divergent ordering
// here made makecomplistglobal's `linwhat == IN_ENV_LW` never match after an
// assignment (`x=<Tab>`): INWHAT held IN_ENV=4 while IN_ENV_LW was 5, so
// value/assignment completion silently did nothing.
pub const IN_NOTHING_LW: i32 = 0; // = IN_NOTHING (zsh.h:2322)
/// `IN_CMD_LW` constant.
pub const IN_CMD_LW: i32 = 1; // = IN_CMD (zsh.h:2324)
/// `IN_MATH_LW` constant.
pub const IN_MATH_LW: i32 = 2; // = IN_MATH (zsh.h:2326)
/// `IN_COND_LW` constant.
pub const IN_COND_LW: i32 = 3; // = IN_COND (zsh.h:2328)
/// `IN_ENV_LW` constant.
pub const IN_ENV_LW: i32 = 4; // = IN_ENV (zsh.h:2330)
/// `IN_PAR_LW` constant.
pub const IN_PAR_LW: i32 = 5; // = IN_PAR (zsh.h:2332)
                              // `origline_stub` / `origcs_stub` deleted — Rust-only 1-line
                              // accessors for the `ORIGLINE` / `ORIGCS` globals (ports of C
                              // `origline` / `origcs` at zle_tricky.c:75 etc.). C reads these
                              // globals inline; callers in compcore.rs now do the lock/load
                              // directly.
/// Port of `void unmetafy_line(void)` from `zle_tricky.c:995`.
///
/// C body:
///   `zlemetaline[zlemetall] = '\0';
///    zleline = stringaszleline(zlemetaline, zlemetacs, &zlell,
///                              &linesz, &zlecs);
///    free(zlemetaline); zlemetaline = NULL;
///    CCRIGHT();`
///
/// Reads ZLEMETALINE, decodes via the canonical stringaszleline
/// (handles incs adjustment + unmetafy + UTF-8 decode), populates
/// ZLELINE / ZLELL / ZLECS, clears ZLEMETALINE/ZLEMETALL.
pub fn unmetafy_line() {
    // zle_tricky.c:995
    let meta = ZLEMETALINE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let zlemetacs = ZLEMETACS.load(Ordering::Relaxed) as i32;
    let mut out_ll: i32 = 0;
    let mut out_cs: i32 = 0;
    // c:998 — `zleline = stringaszleline(zlemetaline, zlemetacs, &zlell, &linesz, &zlecs);`
    let line = crate::ported::zle::zle_utils::stringaszleline(
        &meta,
        zlemetacs,
        Some(&mut out_ll),
        None,
        Some(&mut out_cs),
    );
    if let Ok(mut g) = ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = line.iter().collect();
    }
    ZLELL.store(out_ll, Ordering::Relaxed);
    ZLECS.store(out_cs, Ordering::Relaxed);
    // c:1001-1002 — `free(zlemetaline); zlemetaline = NULL;`. Rust:
    // clear the buffer + zero the length to mark meta-mode inactive.
    if let Some(m) = ZLEMETALINE.get() {
        if let Ok(mut g) = m.lock() {
            g.clear();
        }
    }
    ZLEMETALL.store(0, Ordering::Relaxed);
    // c:1007 — CCRIGHT(): combining-char alignment fixup. No-op in
    // this Rust port (handled by stringaszleline's codepoint walk).
}

/// Port of `void metafy_line(void)` from `zle_tricky.c:978`.
///
/// C body:
///   `zlemetaline = zlelineasstring(zleline, zlell, zlecs,
///                                  &zlemetall, &zlemetacs, 0);
///    metalinesz = zlemetall;
///    free(zleline); zleline = NULL;`
///
/// Reads ZLELINE, encodes via the canonical zlelineasstring (handles
/// wcrtomb + metafy expansion), populates ZLEMETALINE / ZLEMETALL /
/// ZLEMETACS, clears ZLELINE/ZLELL.
pub fn metafy_line() {
    // zle_tricky.c:978
    let raw_vec: Vec<char> = ZLELINE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.chars().collect())
        .unwrap_or_default();
    let zlell = raw_vec.len();
    let zlecs = ZLECS.load(Ordering::Relaxed);
    let mut out_ll: i32 = 0;
    let mut out_cs: i32 = 0;
    // c:982 — `zlemetaline = zlelineasstring(zleline, zlell, zlecs, &zlemetall, &zlemetacs, 0);`
    let meta = crate::ported::zle::zle_utils::zlelineasstring(
        &raw_vec,
        zlell,
        zlecs,
        Some(&mut out_ll),
        Some(&mut out_cs),
        0,
    );
    if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = meta;
    }
    ZLEMETALL.store(out_ll, Ordering::Relaxed);
    ZLEMETACS.store(out_cs, Ordering::Relaxed);
    // c:985 — `metalinesz = zlemetall;`. Rust String grows on demand;
    // no separate sizeline tracker.
    // c:989-990 — `free(zleline); zleline = NULL;`. Rust: clear the
    // buffer + zero ZLELL.
    if let Ok(mut g) = ZLELINE.get().unwrap().lock() {
        g.clear();
    }
    ZLELL.store(0, Ordering::Relaxed);
}

fn opt_isset(name: &str) -> i32 {
    // options.c
    if crate::ported::options::opt_state_get(name).unwrap_or(false) {
        1
    } else {
        0
    }
}
/// Real call into `getiparam(name)` — the canonical paramtab read.
/// Mirrors C's `getiparam` at params.c:3044 which reads the global
/// `paramtab` directly via `gethashnode2`.
fn env_iparam(name: &str) -> i32 {
    // params.c:3044
    crate::ported::params::getiparam(name) as i32
}
fn lastprebr_set(s: &str) {
    // zle_tricky.c lastprebr
    if let Ok(mut g) = LASTPREBR.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = s.to_string();
    }
}
fn lastpostbr_set(s: &str) {
    // zle_tricky.c lastpostbr
    if let Ok(mut g) = LASTPOSTBR.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = s.to_string();
    }
}

/// Choose `$compstate[context]` per the lex classification in `inwhat`
/// (and the `ispar` modifier). Direct lift of compcore.c:578-633.
///
/// The arm ORDER is load-bearing and follows C exactly: `ispar`,
/// `IN_PAR`, `IN_MATH`, **`lincmd`**, **`linredir`**, then the
/// `switch (linwhat)` for IN_ENV / IN_COND / default. An earlier
/// revision tested IN_COND/IN_ENV before `lincmd`, which is the
/// opposite of C — harmless in practice only because `get_comp_string`
/// forces `lincmd = 0` inside a `[[ … ]]` (c:1198-1202, the `!incond`
/// terms) and `docomplete` zeroes it for IN_ENV (zle_tricky.c:701-702).
fn compcontext_for(_s: &str) -> String {
    use crate::ported::zle::zle_tricky::{CMDSTR, LINARR, LINCMD, LINREDIR};
    let ip = ispar.load(Ordering::Relaxed); // c:578
    if ip != 0 {
        // c:579
        return if ip == 2 {
            "brace_parameter".into()
        } else {
            "parameter".into()
        };
    }
    let lw = linwhat.load(Ordering::Relaxed);
    let insubscr = crate::ported::zle::zle_tricky::INSUBSCR.load(Ordering::Relaxed); // c:583
    if lw == IN_PAR_LW {
        return "assign_parameter".into(); // c:580-581
    }
    if lw == IN_MATH_LW {
        // c:582-591 — inside an unclosed `[` the math context is a
        // SUBSCRIPT, and `$compstate[parameter]` names the array
        // (published by callcompfunc from `varname`). Without this
        // `echo $fpath[<TAB>` never reached `_subscript`.
        return if insubscr != 0 {
            "subscript".into() // c:584
        } else {
            "math".into() // c:590
        };
    }
    if LINCMD.load(Ordering::Relaxed) != 0 {
        // c:592-597 — `[` in COMMAND position is a subscript too.
        return if insubscr != 0 {
            "subscript".into() // c:594
        } else {
            "command".into() // c:597
        };
    }
    if LINREDIR.load(Ordering::Relaxed) != 0 {
        // c:598-602 — the cursor word is a redirection TARGET
        // (`echo x > /tm<TAB>`). `callcompfunc` publishes `rdstr` as
        // `$compstate[redirect]` right after this returns, which is what
        // `_redirect` (Completion/Zsh/Context/_redirect) dispatches on.
        return "redirect".into(); // c:599
    }
    match lw {
        // c:604
        // c:605-606 — an array assignment (`x=(a b <TAB>)`) is
        // `array_value`, a scalar one (`x=<TAB>`) plain `value`.
        x if x == IN_ENV_LW => if LINARR.load(Ordering::Relaxed) != 0 {
            "array_value"
        } else {
            "value"
        }
        .into(),
        x if x == IN_COND_LW => "condition".into(), // c:619-620
        // c:622-630 — no command word was parsed at all, so this is the
        // value of something rather than an argument to a command.
        _ => {
            let have_cmdstr = CMDSTR.lock().map(|g| g.is_some()).unwrap_or(false); // c:623
            if have_cmdstr {
                "command".into() // c:624
            } else {
                "value".into() // c:626
            }
        }
    }
}

/// File-scope `int offs` from `Src/Zle/zle_tricky.c:88`. The C source
/// declares this as `mod_export`; mirrored here per Rule 9 since it's
/// not yet at a canonical Rust home.
pub static OFFS: AtomicI32 = AtomicI32::new(0); // zle_tricky.c:88

/// File-scope `Compctl freecl` from `Src/Zle/compcore.c:255`. The
/// freelist of available Compctl slots for the current completion call.
pub static freecl: OnceLock<Mutex<Option<i32>>> = OnceLock::new(); // c:255

/// Real call into `doshfunc` — `Src/exec.c`. Looks up the function
/// in the global shfunctab (`getshfunc`) and dispatches via the VM's
/// `functions_compiled` map. Returns the function's exit status
/// (LASTVAL after the call), matching C's `doshfunc` return value.
pub fn shfunc_call(name: &str) -> i32 {
    // exec.c
    if crate::ported::utils::getshfunc(name).is_none() {
        // c:exec.c:5800
        return 1; // missing fn → status 1
    }
    // Route through the canonical exec accessors dispatcher so the
    // function actually executes. Hook returns Option<i32>; None
    // means no executor context is set up yet (fall back to
    // LASTVAL read).
    crate::ported::exec::dispatch_function_call(name, &[])
        .unwrap_or_else(|| crate::ported::builtin::LASTVAL.load(Ordering::Relaxed))
}
/// Real call into `setsparam(&format!("compstate[{key}]"), val)` — the
/// canonical paramtab write. Mirrors C's `setsparam` at params.c:3350.
///
/// Now `pub` so compsys engine ports can write `$compstate[KEY]`
/// directly. Also dual-writes to `paramtab_hashed_storage()` under
/// the "compstate" key so subscript lookups via the hash-param
/// machinery see the same value — `$compstate` IS a PM_HASHED param
/// (created by `makecompparams` at `complete.rs:1499`), and shell
/// scripts read it as such.
pub fn set_compstate_str(key: &str, val: &str) {
    // params.c:3350 — flat bracketed-param write (preserves the
    // pre-existing access path used by `set_compstate_str` callers).
    let pname = format!("compstate[{}]", key);
    let _ = setsparam(&pname, val);

    // Hash-storage write: dual-store under the `compstate` hash so
    // `${compstate[KEY]}` shell reads (via the hashparam machinery)
    // and any direct `paramtab_hashed_storage()` consumer see the
    // same value.
    if let Ok(mut tab) = paramtab_hashed_storage().lock() {
        tab.entry("compstate".to_string())
            .or_default()
            .insert(key.to_string(), val.to_string());
    }

    // The `VAL(...)` rows in `compkparams` (`Src/Zle/complete.c:1292`,
    // `:1297`, `:1300`) name a real variable rather than a getter, so in
    // C an assignment to `$compstate[KEY]` updates that variable and a
    // later read sees it. [`get_compstate_str`] serves those keys from
    // the backing global, so the write has to land there too or the
    // round-trip is lost. The getter-only rows are deliberately absent:
    // C recomputes them on every read and a stored value would be stale.
    match key {
        // c:1292 `VAL(complistmax)`.
        "list_max" => {
            if let Ok(n) = val.parse::<i64>() {
                crate::ported::zle::complete::COMPLISTMAX.store(n, Ordering::Relaxed);
            }
        }
        // c:1297 `VAL(compvared)`.
        "vared" => {
            if let Ok(mut s) = crate::ported::zle::complete::COMPVARED
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
            {
                *s = val.to_string();
            }
        }
        // c:1300 `VAL(compignored)`.
        "ignored" => {
            if let Ok(n) = val.parse::<i64>() {
                crate::ported::zle::complete::COMPIGNORED.store(n, Ordering::Relaxed);
            }
        }
        _ => {}
    }
}

/// The `$compstate` keys whose values C does not store: their
/// `compkparams` rows (`Src/Zle/complete.c:1261-1300`) carry a `gsu`
/// vtable instead of a `var` pointer, so every read runs the getter
/// against live completion state. Listed in `compkparams` order.
pub const LIVE_COMPSTATE_KEYS: &[&str] = &[
    "nmatches",              // c:1262 nmatches_gsu
    "unambiguous",           // c:1285 unambig_gsu
    "unambiguous_cursor",    // c:1286 unambig_curs_gsu
    "unambiguous_positions", // c:1288 unambig_pos_gsu
    "insert_positions",      // c:1290 insert_pos_gsu
    "list_max",              // c:1292 VAL(complistmax)
    "vared",                 // c:1297 VAL(compvared)
    "list_lines",            // c:1298 listlines_gsu
    "all_quotes",            // c:1299 compqstack_gsu
    "ignored",               // c:1300 VAL(compignored)
];

/// Read `$compstate[KEY]`. Returns `None` when the key was never set.
///
/// The [`LIVE_COMPSTATE_KEYS`] arm below is the Rust stand-in for C's
/// per-key gsu getter firing on each read; everything else comes from
/// the hash-storage view (the canonical home for a PM_HASHED param),
/// falling back to the legacy flat `compstate[KEY]` bracketed param for
/// entries that some code wrote via raw `setsparam` without going
/// through [`set_compstate_str`].
pub fn get_compstate_str(key: &str) -> Option<String> {
    // c:complete.c:1236-1252 — the gsu-backed keys are recomputed on
    // every read; a stored value would be stale. Before this arm covered
    // more than `nmatches`, none of them existed anywhere in zshrs's
    // compstate storage, so `_lastcomp` (`_main_complete` sh:407) came
    // back missing nine entries — `_lastcomp[unambiguous]` and
    // `[unambiguous_cursor]` (read at sh:84-86 and by `_next_tags`
    // sh:105) among them.
    let nil = std::ptr::null_mut();
    match key {
        // c:complete.c:1401-1405 — `get_nmatches`: flush pending match
        // groups via `permmatches(0)`, then read the counter. A stored
        // read served a stale 0, so every completer's
        // `nm != $compstate[nmatches]` idiom (_describe, _arguments,
        // _alternative, …) concluded "nothing was added" and option
        // completion died even though addmatches had added hundreds.
        "nmatches" => {
            let v = if permmatches(0) != 0 {
                0
            } else {
                nmatches.load(Ordering::Relaxed)
            };
            return Some(v.to_string());
        }
        // c:1439-1442 — `unambig_data(NULL, NULL, NULL)`.
        "unambiguous" => return Some(crate::ported::zle::complete::get_unambig(nil)),
        // c:1446-1450 — `unambig_data(&c, NULL, NULL); return c`.
        "unambiguous_cursor" => {
            return Some(crate::ported::zle::complete::get_unambig_curs(nil).to_string())
        }
        // c:1447-1456 — `unambig_data(NULL, &p, NULL); return p`.
        "unambiguous_positions" => return Some(crate::ported::zle::complete::get_unambig_pos(nil)),
        // c:1458-1466 — `unambig_data(NULL, NULL, &p); return p`.
        "insert_positions" => return Some(crate::ported::zle::complete::get_insert_pos(nil)),
        // c:1292 `VAL(complistmax)`; seeded from $LISTMAX at c:323.
        "list_max" => {
            return Some(
                crate::ported::zle::complete::COMPLISTMAX
                    .load(Ordering::Relaxed)
                    .to_string(),
            )
        }
        // c:1297 `VAL(compvared)` — the parameter name `vared` is
        // editing, `""` outside `vared` (c:compcore.c:565-570). zshrs
        // does not track `varedarg` yet, so the global stays at the
        // `""` C publishes for every non-`vared` completion.
        "vared" => {
            return Some(
                crate::ported::zle::complete::COMPVARED
                    .get_or_init(|| Mutex::new(String::new()))
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default(),
            )
        }
        // c:1408-1420 — `get_listlines` → `list_lines()`.
        "list_lines" => return Some(crate::ported::zle::complete::get_listlines(nil).to_string()),
        // c:1469 — `get_compqstack`: one char per quoting level.
        "all_quotes" => return Some(crate::ported::zle::complete::get_compqstack(nil)),
        // c:1300 `VAL(compignored)` — matches dropped by `compadd -F`.
        "ignored" => {
            return Some(
                crate::ported::zle::complete::COMPIGNORED
                    .load(Ordering::Relaxed)
                    .to_string(),
            )
        }
        _ => {}
    }
    if let Ok(tab) = paramtab_hashed_storage().lock() {
        if let Some(hash) = tab.get("compstate") {
            if let Some(v) = hash.get(key) {
                return Some(v.clone());
            }
        }
    }
    // Fallback: pre-existing callers wrote via raw `setsparam` only.
    let pname = format!("compstate[{}]", key);
    getsparam(&pname)
}

/// Local helper: position before-the-current char (handles UTF-8).
#[inline]
fn prev_char_index(bytes: &[u8], pos: usize) -> usize {
    // local
    if pos == 0 {
        return 0;
    }
    let mut i = pos - 1;
    while i > 0 && (bytes[i] & 0xC0) == 0x80 {
        i -= 1;
    }
    i
}

#[inline]
fn char_at(bytes: &[u8], pos: usize) -> char {
    // local
    if pos >= bytes.len() {
        return '\0';
    }
    let s = match std::str::from_utf8(&bytes[pos..]) {
        Ok(s) => s,
        Err(_) => return '\0',
    };
    s.chars().next().unwrap_or('\0')
}

/// Depth counter so `set_comp_sep`'s sanity assert ("lexsave/restore
/// balanced") fires when a future port mismatches them.
static LEXSAVE_DEPTH: AtomicI32 = AtomicI32::new(0); // local

/// Walk a balanced pair of in/out token bytes starting at `start`,
/// returning the index just after the closing token, or None if
/// unbalanced. C `skipparens` returns the position; this version
/// returns the same semantic.
fn skip_token_parens(bytes: &[u8], start: usize, open: char, close: char) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut i = start;
    while i < bytes.len() {
        let c = char_at(bytes, i);
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i + c.len_utf8());
            }
        }
        i += c.len_utf8();
    }
    if depth == 0 {
        Some(i)
    } else {
        None
    }
}

/// Walk the INAMESPC name-character class — equivalent to C's
/// `itype_end(e, INAMESPC, 0)` loop. Stops at first non-name char.
fn walk_namespace(bytes: &[u8]) -> usize {
    // local
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let mut len = 0usize;
    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' {
            len += c.len_utf8();
        } else {
            break;
        }
    }
    len
}

/// Strip Inbrace/Outbrace/Stringg/etc. token bytes back to literal
/// characters — substitute for C `untokenize()` over the slice. The
/// canonical Rust untokenize lives in `crate::lex::untokenize`.
fn strip_tokens(s: &str) -> String {
    // local
    crate::lex::untokenize(s).to_string()
}

/// File-scope `int hcompcall` accessor — `compfunc` active iff non-empty.
fn compfunc_active() -> bool {
    compfunc
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

/// Direct port of `void lexsave(void)` from `Src/lex.c`. Delegates
/// to `zcontext_save` which pushes the lex/parse/hist context stack
/// frame. Returns a token (current stack depth) for symmetry with
/// the C `int` save token used by `set_comp_sep` for invariant check.
fn lexsave() -> usize {
    // lex.c via context.c:80
    crate::ported::context::zcontext_save();
    (LEXSAVE_DEPTH.fetch_add(1, Ordering::SeqCst) + 1) as usize
}

/// Direct port of `void lexrestore(void)` from `Src/lex.c`. Pops the
/// last `zcontext_save` frame. C body restores hist/lex/parse via
/// `zcontext_restore_partial(ZCONTEXT_HIST|ZCONTEXT_LEX|ZCONTEXT_PARSE)`.
fn lexrestore(_token: usize) {
    // lex.c via context.c:117
    let parts = ZCONTEXT_HIST | ZCONTEXT_LEX | ZCONTEXT_PARSE;
    zcontext_restore_partial(parts);
    LEXSAVE_DEPTH.fetch_sub(1, Ordering::SeqCst);
}

// ---- Extern stubs for addmatches's bucket-3 dependencies ----

/// Reads the first char of `char *compquote` — `Src/Zle/complete.c:54`,
/// gsu-bound to `$compstate[quote]` (complete.c:1276), i.e. C's
/// `(qc = *compquote)` at c:2139.
///
/// !!! WARNING: RUST-ONLY HELPER !!!
/// C dereferences the bare global; the port keeps it in an
/// `OnceLock<Mutex<String>>`, so the deref needs a function. It used to
/// read `zle_tricky::COMPQUOTE` — a Rust-only DUPLICATE of the global
/// that has no counterpart in `Src/Zle/zle_tricky.c` and that nothing
/// ever writes, so `addmatches`'s quote block (c:2139-2168) always took
/// the `else` arm and cleared `instring`/`autoq` on every compadd.
fn compquote_first() -> Option<char> {
    // complete.c:54
    crate::ported::zle::complete::COMPQUOTE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .ok()
        .and_then(|g| g.chars().next())
}
fn instring_set(v: i32) {
    // zle_tricky.c:419
    INSTRING.store(v, Ordering::Relaxed);
}
fn inbackt_set(v: i32) {
    // zle_tricky.c:419
    INBACKT.store(v, Ordering::Relaxed);
}
fn autoq_set(s: &str) {
    // zle_tricky.c autoq
    if let Ok(mut g) = AUTOQ.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = s.to_string();
    }
}

// ---- Extern stubs for makecomplist's bucket-3 dependencies ----

/// File-scope holder for `Cmlist bmatchers` — `Src/Zle/compcore.c:236`.
/// C linked-list of matchers active for brace-matching, populated by
/// `add_bmatchers` walking the user-installed `Cmatcher` chain.
pub static bmatchers: OnceLock<Mutex<Option<Box<Cmlist>>>> = OnceLock::new(); // c:236

/// File-scope holder for `Cmlist mstack` — `Src/Zle/compcore.c:236`.
/// Matcher-stack — current active matcher list for compadd recursion.
pub static mstack: OnceLock<Mutex<Option<Box<Cmlist>>>> = OnceLock::new(); // c:236

// ---- Extern stubs for add_match_data's Cline operations ----

/// Bridge to `cline_matched()` — `Src/Zle/compmatch.c:253`. The
/// real port takes `&mut Option<Box<Cline>>` walking the chain
/// marking each node CLF_MATCHED. With only a string slice here we
/// build a one-node Cline shim and route the call through it so the
/// CLF_MATCHED state-machine update fires the same way as in C.
fn cline_matched_compcore(line: Option<&str>) {
    // compmatch.c:253
    let Some(s) = line else {
        return;
    };
    if s.is_empty() {
        return;
    }
    let mut head = Some(Box::new(Cline {
        line: Some(s.to_string()),
        llen: s.len() as i32,
        ..Default::default()
    }));
    cline_matched(&mut head);
}
/// Reads `char *qisuf` — `Src/Zle/zle_tricky.c:137`.
///
/// This used to be `getsparam("qisuf")`, i.e. a lookup for a SHELL
/// PARAMETER spelled with the C variable's name. No such parameter
/// exists in zsh or in this port (the shell-visible names are
/// `$QIPREFIX` / `$QISUFFIX`, complete.c:1266-1267, and they are gsu
/// views onto `compqiprefix`/`compqisuffix`, not onto `qipre`/`qisuf`),
/// so the read missed on every match and `add_match_data` built every
/// `cm->ipre`/`cm->isuf` without the word's quotes.
fn qisuf_get() -> String {
    // zle_tricky.c:137
    crate::ported::zle::zle_tricky::qisuf_get()
}
/// Reads `char *qipre` — `Src/Zle/zle_tricky.c:137`. See [`qisuf_get`].
fn qipre_get() -> String {
    // zle_tricky.c:137
    crate::ported::zle::zle_tricky::qipre_get()
}

/// Adapter for `int movefd(int fd)` from `Src/utils.c:2974` —
/// delegates to the canonical port in `ported::utils::movefd`.
fn movefd(fd: i32) -> i32 {
    // utils.c:2974
    crate::ported::utils::movefd(fd)
}

/// Adapter for `int redup(int x, int y)` from `Src/utils.c:2021` —
/// delegates to the canonical port `ported::utils::redup`. Every caller
/// here is one of C's three `redup(osi, 0)` sites (compcore.c:1013/1035/
/// 1039), i.e. "put the descriptor `movefd(0)` parked at c:964 back on
/// fd 0", so the target is 0.
///
/// The adapter used to pass `-1` as the target. `redup(osi, -1)` takes
/// the `x != y` arm, `dup2(osi, -1)` fails with EBADF, and the saved
/// descriptor is closed anyway — so the shell's stdin was DESTROYED by
/// the first Tab and fd 0 stayed closed for the rest of the session
/// (`cat` at the prompt after a completion read a bad descriptor).
fn redup(new: i32) {
    // utils.c:2021
    crate::ported::utils::redup(new, 0);
}

/// File-scope registry mirroring `Src/init.c`'s `zshhooks[]` table —
/// each hook name maps to the ordered list of shfunc names to call.
pub static HOOK_FNS: OnceLock<Mutex<std::collections::HashMap<String, Vec<String>>>> =
    OnceLock::new(); // init.c zshhooks

/// Adapter for the `errflag` global from `Src/init.c` — reads the
/// canonical atomic in `ported::utils::errflag`.
fn errflag_get() -> bool {
    crate::ported::utils::errflag.load(Ordering::Relaxed) != 0 // init.c
}

/// Local dispatcher used by compcore call sites for hook names that
/// don't yet have a typed-data argument. Delegates to the canonical
/// `module::runhookdef(gethookdef(name), NULL)` — no-op when no
/// Hookfn is registered (c:993-995). Returns the Hookfn return value
/// (or 0 when no handler fires).
fn runhookdef_compcore(hook: &str) -> i32 {
    // c:990
    let h = gethookdef(hook);
    if h.is_null() {
        return 0;
    }
    runhookdef(h, std::ptr::null_mut())
}

/// Port of `static int ccmakehookfn(Hookdef, Ccmakedat dat)` from
/// `Src/Zle/compctl.c:1763` — the function the compctl module registers
/// on COMPCTLMAKEHOOK. This is the compctl-side analog of the compfunc
/// branch in `makecomplist`: it builds the match list via
/// `makecomplistglobal` (the default command/file completion the bare
/// `zsh -f` shell uses), finalizes it with `permmatches`, and reports
/// success through `dat.lst` (0 = matches, 1 = none). The previous stub
/// only called the unrelated `makecomplistctl` and never set `dat.lst`,
/// so `makecomplist` returned the raw list-type (nonzero) and
/// `do_completion` always took the feep/error path — `l<Tab>` produced
/// nothing.
///
/// The C source loops over the global matcher list (`cmatcher`); the
/// common case (and always under `-f`) has none, so this port runs the
/// single no-matcher pass, matching begcmgroup/makecomplistglobal/
/// endcmgroup/permmatches exactly as the compfunc branch does.
fn runhookdef_compctlmake(
    // c:1763
    dat: &mut Ccmakedat,
) {
    let os = dat.str.clone().unwrap_or_default();
    let incmd = dat.incmd;
    let lst = dat.lst;

    // c:1794-1810 — no global matchers → mstack = NULL, bmatchers = NULL.
    if let Ok(mut g) = bmatchers.get_or_init(|| Mutex::new(None)).lock() {
        *g = None;
    }
    if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
        *g = None;
    }
    // c:1812-1813 — ainfo = fainfo = hcalloc.
    if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
        *g = Some(Aminfo::default());
    }
    if let Ok(mut g) = fainfo.get_or_init(|| Mutex::new(None)).lock() {
        *g = Some(Aminfo::default());
    }
    if let Ok(mut g) = freecl.get_or_init(|| Mutex::new(None)).lock() {
        *g = None; // c:1815
    }
    if VALIDLIST.load(Ordering::Relaxed) == 0 {
        LASTAMBIG.store(0, Ordering::Relaxed); // c:1817
    }
    if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear(); // c:1818
    }
    mnum.store(0, Ordering::Relaxed); // c:1819
    unambig_mnum.store(-1, Ordering::Relaxed); // c:1820
    if let Ok(mut g) = isuf.get_or_init(|| Mutex::new(String::new())).lock() {
        g.clear(); // c:1821
    }
    insmnum.store(ZMULT.load(Ordering::Relaxed), Ordering::Relaxed); // c:1822
    oldlist.store(0, Ordering::Relaxed); // c:1829
    oldins.store(0, Ordering::Relaxed); // c:1829
    begcmgroup(Some("default"), 0); // c:1830
    MENUCMP.store(0, Ordering::Relaxed); // c:1831
    menuacc.store(0, Ordering::Relaxed); // c:1831
    newmatches.store(0, Ordering::Relaxed); // c:1831
    onlyexpl.store(0, Ordering::Relaxed); // c:1831

    // c:1836-1837 — makecomplistglobal(s, incmd, lst, 0) generates the
    // matches (per-command compctl or the default command/file logic).
    crate::ported::zle::compctl::makecomplistglobal(&os, incmd != 0, lst, 0);
    endcmgroup(None); // c:1838

    // c:1879-1892 — permmatches(1); amatches = pmatches; swap holders.
    permmatches(1); // c:1879
    let p_snap = pmatches
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        *g = p_snap.clone(); // c:1880
    }
    lastpermmnum.store(permmnum.load(Ordering::Relaxed), Ordering::Relaxed); // c:1881
    lastpermgnum.store(permgnum.load(Ordering::Relaxed), Ordering::Relaxed); // c:1882
    if let Ok(mut g) = lastmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        *g = p_snap; // c:1884
    }
    let lm_snap = lmatches
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.clone());
    if let Ok(mut g) = lastlmatches.get_or_init(|| Mutex::new(None)).lock() {
        *g = lm_snap; // c:1885
    }
    if let Ok(mut g) = pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear(); // c:1886
    }
    hasperm.store(0, Ordering::Relaxed); // c:1887
    hasoldlist.store(1, Ordering::Relaxed); // c:1888

    // c:1890-1901 — success iff we produced matches and no error.
    if nmatches.load(Ordering::Relaxed) != 0 && !errflag_get() {
        VALIDLIST.store(1, Ordering::Relaxed); // c:1891
        dat.lst = 0; // c:1894
    } else {
        dat.lst = 1; // c:1908
    }
}

// =====================================================================
// permmatches — `Src/Zle/compcore.c:3423`.
// =====================================================================

/// Static state for `permmatches`'s `static int fi`. C scopes the
/// flag to the function; Rust hoists it to file scope per Rule S1.
static PERMMATCHES_FI: AtomicI32 = AtomicI32::new(0); // c:3423 static int fi

/// Port of the `type==0` string-sort branch of `makearray()` from
/// compcore.c:3239-3257. Sorts strings via `strmetasort` + dedup.
pub fn makearray_strings(mut rp: Vec<String>, flags: i32) -> (Vec<String>, i32) {
    // c:3239
    let mut n: i32 = rp.len() as i32;
    if flags != 0 && n > 0 {
        // c:3240
        let numeric = isset(NUMERICGLOBSORT); // c:3243
        let mut sf = SORTIT_IGNORING_BACKSLASHES as u32;
        if numeric {
            sf |= SORTIT_NUMERICALLY as u32;
        }
        crate::ported::sort::strmetasort(&mut rp, sf, None); // c:3242-3244

        // Dedup consecutive equals.                                         // c:3247
        let mut cp = 0usize;
        let mut ap = 0usize;
        while ap < rp.len() {
            if ap != cp {
                rp.swap(ap, cp);
            }
            cp += 1;
            let mut bp = ap;
            while bp + 1 < rp.len() && rp[ap] == rp[bp + 1] {
                // c:3250
                bp += 1;
                n -= 1;
            }
            ap = bp + 1; // c:3252
        }
        rp.truncate(cp); // c:3253
    }
    (rp, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rembslash_basic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(rembslash("hello\\ world"), "hello world");
        assert_eq!(rembslash("no\\\\slash"), "no\\slash");
        assert_eq!(rembslash("plain"), "plain");
    }

    #[test]
    fn comp_quoting_string_table() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(comp_quoting_string(QT_SINGLE), "'");
        assert_eq!(comp_quoting_string(QT_DOUBLE), "\"");
        assert_eq!(comp_quoting_string(QT_DOLLARS), "$'");
        assert_eq!(comp_quoting_string(0), "\\");
        assert_eq!(comp_quoting_string(QT_BACKSLASH), "\\");
    }

    #[test]
    fn matcheq_equal_strings() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.str = Some("foo".into());
        let mut b = Cmatch::default();
        b.str = Some("foo".into());
        assert!(matcheq(&a, &b));
    }

    #[test]
    fn matcheq_different_strings() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.str = Some("foo".into());
        let mut b = Cmatch::default();
        b.str = Some("bar".into());
        assert!(!matcheq(&a, &b));
    }

    #[test]
    fn matcheq_one_side_none() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.pre = Some("p".into());
        let b = Cmatch::default();
        assert!(!matcheq(&a, &b));
    }

    #[test]
    fn get_user_var_reads_array_from_paramtab() {
        let _g = crate::test_util::global_state_lock();
        // c:2003 — `getaparam(nam)` first. Verify array params come
        //          out as a Vec, not via env.
        let _g = zle_test_setup();
        setaparam("__test_arr", vec!["a".into(), "bb".into(), "ccc".into()]);
        let got = get_user_var(Some("__test_arr"));
        assert_eq!(got, Some(vec!["a".into(), "bb".into(), "ccc".into()]));
        // Cleanup so we don't poison other tests.
        setaparam("__test_arr", vec![]);
    }

    #[test]
    fn get_user_var_reads_scalar_as_single_element_array() {
        let _g = crate::test_util::global_state_lock();
        // c:2007-2009 — getsparam fallback: wrap scalar in 1-element array.
        let _g = zle_test_setup();
        setsparam("__test_scalar", "hello");
        let got = get_user_var(Some("__test_scalar"));
        assert_eq!(got, Some(vec!["hello".to_string()]));
        setsparam("__test_scalar", "");
    }

    #[test]
    fn get_user_var_paren_list_splits_on_separators() {
        let _g = crate::test_util::global_state_lock();
        // c:1960-1996 — `(a b c)` paren list, NOT a param lookup.
        let _g = zle_test_setup();
        let got = get_user_var(Some("(one two three)"));
        assert_eq!(got, Some(vec!["one".into(), "two".into(), "three".into()]));
    }

    #[test]
    fn get_user_var_none_for_missing() {
        let _g = crate::test_util::global_state_lock();
        // c:1956 + c:2009 — missing param returns None.
        let _g = zle_test_setup();
        // (env vars must not leak through — we don't read $PATH etc.)
        let got = get_user_var(Some("__definitely_not_a_param_xyz"));
        assert_eq!(got, None);
    }

    #[test]
    fn get_data_arr_reads_hashed_keys_or_values() {
        let _g = crate::test_util::global_state_lock();
        // c:2022 — fetchvalue(name, SCANPM_WANTKEYS|WANTVALS|MATCHMANY).
        let _g = zle_test_setup();
        crate::ported::params::sethparam(
            "__test_hash",
            vec!["k1".into(), "v1".into(), "k2".into(), "v2".into()],
        );

        let keys = get_data_arr("__test_hash", true);
        assert!(keys.is_some(), "hashed param should have keys");
        let mut keys = keys.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["k1".to_string(), "k2".to_string()]);

        let vals = get_data_arr("__test_hash", false);
        assert!(vals.is_some(), "hashed param should have values");
        let mut vals = vals.unwrap();
        vals.sort();
        assert_eq!(vals, vec!["v1".to_string(), "v2".to_string()]);
    }

    #[test]
    fn get_data_arr_none_for_non_hashed() {
        let _g = crate::test_util::global_state_lock();
        // c:2032 — fetchvalue NULL → return NULL for params that
        //          aren't associative arrays.
        let _g = zle_test_setup();
        setsparam("__test_scalar2", "value");
        let got = get_data_arr("__test_scalar2", false);
        assert_eq!(got, None, "scalar params must NOT come out of get_data_arr");
    }

    #[test]
    fn before_complete_snapshots_oldmenucmp() {
        let _g = crate::test_util::global_state_lock();
        // c:463 — `oldmenucmp = menucmp;`
        let _g = zle_test_setup();
        MENUCMP.store(7, Ordering::Relaxed);
        OLDMENUCMP.store(0, Ordering::Relaxed);
        let mut lst = 0;
        let _ = before_complete(&mut lst);
        assert_eq!(OLDMENUCMP.load(Ordering::Relaxed), 7);
        // Reset for other tests.
        MENUCMP.store(0, Ordering::Relaxed);
        OLDMENUCMP.store(0, Ordering::Relaxed);
    }

    #[test]
    fn before_complete_clears_showagain() {
        let _g = crate::test_util::global_state_lock();
        // c:467 — `showagain = 0;` always (after the validlist gate).
        let _g = zle_test_setup();
        SHOWAGAIN.store(5, Ordering::Relaxed);
        let mut lst = 0;
        let _ = before_complete(&mut lst);
        assert_eq!(
            SHOWAGAIN.load(Ordering::Relaxed),
            0,
            "SHOWAGAIN must be cleared by before_complete"
        );
    }

    #[test]
    fn remsquote_default_quoting() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut s = String::from("a'\\''b");
        let n = remsquote(&mut s);
        assert_eq!(s, "a'b");
        assert_eq!(n, 3);
    }

    #[test]
    fn ctokenize_dollar_substitution() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let out = ctokenize("$x{y}");
        let chars: Vec<char> = out.chars().collect();
        assert_eq!(chars[0], Stringg);
        assert_eq!(chars[1], 'x');
        assert_eq!(chars[2], Inbrace);
        assert_eq!(chars[3], 'y');
        assert_eq!(chars[4], Outbrace);
    }

    #[test]
    fn get_user_var_inline_list() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let result = get_user_var(Some("(a b c)")).unwrap();
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn matchcmp_str_sort_default() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        MATCHORDER.store(CGF_MATSORT, Ordering::Relaxed);
        let mut a = Cmatch::default();
        a.str = Some("apple".into());
        let mut b = Cmatch::default();
        b.str = Some("banana".into());
        assert_eq!(matchcmp(&a, &b), std::cmp::Ordering::Less);
        assert_eq!(matchcmp(&b, &a), std::cmp::Ordering::Greater);
        assert_eq!(matchcmp(&a, &a), std::cmp::Ordering::Equal);
        MATCHORDER.store(0, Ordering::Relaxed);
    }

    #[test]
    fn dupmatch_clones_strings_and_truncates_braces() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // C body c:3370: deep-copy strings, truncate brpl/brsl to nbeg/nend.
        let mut src = Cmatch::default();
        src.str = Some("foo".into());
        src.ipre = Some("ipre".into());
        src.flags = 7;
        src.brpl = vec![10, 20, 30, 40];
        src.brsl = vec![5, 6, 7];
        src.qipl = 1;
        src.qisl = 2;
        src.mode = 0o755;
        src.modec = 'd';

        let r = dupmatch(&src, 2, 1);
        assert_eq!(r.str.as_deref(), Some("foo"));
        assert_eq!(r.ipre.as_deref(), Some("ipre"));
        assert_eq!(r.flags, 7);
        assert_eq!(r.brpl, vec![10, 20]); // truncated to nbeg=2
        assert_eq!(r.brsl, vec![5]); // truncated to nend=1
        assert_eq!(r.qipl, 1);
        assert_eq!(r.qisl, 2);
        assert_eq!(r.mode, 0o755);
        assert_eq!(r.modec, 'd');
    }

    #[test]
    fn dupmatch_empty_braces_stay_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // C body c:3395/3404: NULL brpl/brsl stay NULL regardless of nbeg/nend.
        let src = Cmatch::default();
        let r = dupmatch(&src, 5, 5);
        assert!(r.brpl.is_empty());
        assert!(r.brsl.is_empty());
    }

    #[test]
    fn makearray_sorted_and_deduped() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3262-3291: sort + dedup with matcheq. Same str + nil disp =>
        // collapses into one entry with CMF_FMULT set on the survivor.
        let mut a = Cmatch::default();
        a.str = Some("z".into());
        let mut b = Cmatch::default();
        b.str = Some("a".into());
        let mut c = Cmatch::default();
        c.str = Some("a".into());
        let (arr, n, _nl, _ll) = makearray(vec![a, b, c], CGF_MATSORT);
        // Two distinct visible strings after dedup ("a", "z").
        assert_eq!(arr.len(), 2);
        assert_eq!(n, 2);
        assert_eq!(arr[0].str.as_deref(), Some("a"));
        assert_eq!(arr[1].str.as_deref(), Some("z"));
    }

    #[test]
    fn makearray_nosort_unchanged_order() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3300: CGF_NOSORT branch; with no UNIQ flags, order preserved.
        let mut a = Cmatch::default();
        a.str = Some("z".into());
        let mut b = Cmatch::default();
        b.str = Some("a".into());
        let (arr, n, _, _) = makearray(vec![a, b], CGF_NOSORT | CGF_UNIQALL);
        // UNIQALL active so no dedup pass runs.
        assert_eq!(n, 2);
        assert_eq!(arr[0].str.as_deref(), Some("z"));
        assert_eq!(arr[1].str.as_deref(), Some("a"));
    }

    #[test]
    fn makearray_strings_dedup_consecutive() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:3239 path: sort + drop adjacent duplicates.
        let (arr, n) = makearray_strings(vec!["b".into(), "a".into(), "a".into(), "c".into()], 1);
        assert_eq!(n, 3);
        assert_eq!(arr, vec!["a", "b", "c"]);
    }

    #[test]
    fn check_param_no_dollar_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1316: no `$` in string → return None.
        OFFS.store(2, Ordering::Relaxed);
        assert_eq!(check_param("abc", false, false), None);
    }

    #[test]
    fn check_param_simple_dollar_var_at_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1259-1311: `$FOO` with cursor inside the name → return b.
        OFFS.store(2, Ordering::Relaxed);
        let s = format!("{}FOO", Stringg);
        let r = check_param(&s, false, true);
        assert!(r.is_some(), "expected Some(b) inside $FOO");
    }

    /// c:1194-1203 + c:1309 — `${(k)<cursor>` is a brace parameter: the
    /// `(k)` flag group is skipped and `ispar` ends at 2, which is what
    /// `compcontext_for` turns into `brace_parameter`.
    ///
    /// `get_comp_string` hands this port the word UNTOKENIZED, so the flag
    /// group arrives as literal `(`/`)`. Handing those to
    /// `skipparens(Inpar, Outpar, …)` returns -1 ("wrong opening char"),
    /// `b` never moved past the flags, the name scan hit `(` and bailed —
    /// `ispar` stayed 0, the context stayed `command`, and `echo ${(k)<TAB>`
    /// produced nothing at all.
    #[test]
    fn check_param_brace_with_flag_group_sets_ispar_two() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ispar.store(0, Ordering::Relaxed);
        // `${(k)` with the cursor just past the `)`.
        let s = "${(k)";
        OFFS.store(s.len() as i32, Ordering::Relaxed);
        let r = check_param(s, false, false);
        assert!(r.is_some(), "cursor sits at the (empty) parameter name");
        assert_eq!(
            ispar.load(Ordering::Relaxed),
            2,
            "a flag group must still leave a brace-parameter context"
        );
    }

    #[test]
    fn callcompfunc_empty_fn_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:552: getshfunc(NULL) early-return.
        callcompfunc("anything", "");
    }

    #[test]
    fn callcompfunc_sets_compstate_context() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:619: context selection — verified via the pure
        // compcontext_for helper (callcompfunc calls it and writes
        // to paramtab via setsparam, but paramtab read-back in a
        // unit-test context without a live VM is unreliable).
        ispar.store(0, Ordering::Relaxed);
        linwhat.store(IN_PAR_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("foo"), "assign_parameter");
        // Body executes without panicking against the real paramtab.
        callcompfunc("foo", "_test_fn");
    }

    /// c:complete.c:1235-1295 — `$PREFIX`/`$SUFFIX`/`$IPREFIX`/`$ISUFFIX`
    /// and the `compprefix`/`compsuffix`/`compiprefix`/`compisuffix`
    /// globals are ONE storage in C, so `callcompfunc`'s per-call publish
    /// of the cursor word split resets BOTH.
    ///
    /// Regression for `PATH=/usr/bin:<TAB>`: `expand-or-complete` calls
    /// the completion function twice per TAB and `_path_files` leaves its
    /// last path component in `$PREFIX`. Because only the param was
    /// republished, the second pass's `compset -P '*:'` (which reads the
    /// GLOBAL) matched the stale `bin:` instead of the live `/usr/bin:` —
    /// `$IPREFIX` came out `bin:` and the rebuilt line lost `/usr/`.
    #[test]
    fn callcompfunc_republishes_word_split_onto_comp_globals() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // Leftovers from the previous completion pass.
        *COMPPREFIX
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap() = "bin:".to_string();
        *COMPIPREFIX
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .unwrap() = "PATH=/usr/".to_string();

        // Cursor at the end of the word, as get_comp_string leaves it.
        OFFS.store("/usr/bin:".chars().count() as i32, Ordering::Relaxed);
        callcompfunc("/usr/bin:", "_test_fn");

        assert_eq!(
            COMPPREFIX
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap()
                .clone(),
            "/usr/bin:",
            "compprefix must track the freshly published $PREFIX"
        );
        assert_eq!(
            COMPIPREFIX
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap()
                .clone(),
            "",
            "compiprefix must be reset with $IPREFIX, not carried over"
        );
    }

    /// c:compcore.c:820 — `makezleparams(1)` publishes the ZLE special
    /// params for the completion function, and c:839's `endparamscope`
    /// takes them away again on the way out.
    ///
    /// The publish was missing entirely, so every completer saw
    /// `$BUFFER`/`$CURSOR`/`$HISTNO`/`$WIDGET`/`$KEYS`/`$LBUFFER`/
    /// `$RBUFFER`/`$BUFFERLINES`/`$PENDING` as the empty string.
    ///
    /// Both halves are pinned with ONE observable: seed `BUFFER` with a
    /// sentinel at the enclosing scope first.
    ///   * publish missing  → the sentinel survives (`Some(sentinel)`),
    ///   * publish present but teardown missing → the live line survives,
    ///   * both present      → the name is gone entirely.
    /// Only the last is C behaviour.
    #[test]
    fn callcompfunc_publishes_and_tears_down_zle_params() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();

        // `$FUNCNEST` is unset in a bare unit-test paramtab, and
        // `getiparam` reports 0 for unset — which trips doshfunc's
        // c:6000 guard (`funcstacksz >= zsh_funcnest`) on the very first
        // frame and returns BEFORE c:839's endparamscope. Give it the
        // shell's real default so the scope actually unwinds.
        let funcnest_save = crate::ported::params::getiparam("FUNCNEST");
        let _ = crate::ported::params::setiparam("FUNCNEST", 500);

        const SENTINEL: &str = "@@not-published@@";
        for name in ["BUFFER", "WIDGET", "LBUFFER"] {
            let _ = crate::ported::params::setsparam(name, SENTINEL);
        }
        // A non-empty editor line so the publish is distinguishable from
        // "published an empty string".
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "fc -".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(4, Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, Ordering::SeqCst);

        OFFS.store(1, Ordering::Relaxed);
        callcompfunc("-", "_test_fn");
        let _ = crate::ported::params::setiparam("FUNCNEST", funcnest_save);

        for name in ["BUFFER", "WIDGET", "LBUFFER"] {
            let after = crate::ported::params::getsparam(name);
            assert_ne!(
                after.as_deref(),
                Some(SENTINEL),
                "${name} still holds the pre-call sentinel — c:820 makezleparams(1) \
                 never ran, so the completion function saw no ZLE parameters"
            );
            assert_eq!(
                after, None,
                "${name} outlived the completion scope — c:839 endparamscope must \
                 unset every PM_LOCAL zleparam, or the name leaks into the \
                 interactive shell after the first TAB"
            );
        }
    }

    /// Test-only serializer for tests that mutate file-scope globals.
    static GLOBAL_MUT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn compcontext_for_routes_ispar_first() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:583/593 — the subscript arms key off `insubscr`; pin it off so
        // the non-subscript routing below is deterministic (`zle_reset`
        // does not clear it).
        crate::ported::zle::zle_tricky::INSUBSCR.store(0, Ordering::Relaxed);
        crate::ported::zle::zle_tricky::LINCMD.store(0, Ordering::Relaxed);
        // c:598/606/623 — the redirect / array_value / default-command
        // arms read three more file-scope globals; pin them so the
        // routing below is deterministic regardless of test order.
        crate::ported::zle::zle_tricky::LINREDIR.store(0, Ordering::Relaxed);
        crate::ported::zle::zle_tricky::LINARR.store(0, Ordering::Relaxed);
        *crate::ported::zle::zle_tricky::CMDSTR.lock().unwrap() = Some("ls".into());
        ispar.store(2, Ordering::Relaxed);
        linwhat.store(IN_NOTHING_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "brace_parameter");
        ispar.store(1, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "parameter");
        ispar.store(0, Ordering::Relaxed);
        linwhat.store(IN_MATH_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "math");
        linwhat.store(IN_COND_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "condition");
        linwhat.store(IN_ENV_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "value");
        linwhat.store(IN_NOTHING_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "command");
        *crate::ported::zle::zle_tricky::CMDSTR.lock().unwrap() = None;
    }

    /// c:598-602 / c:605-606 / c:622-630 — the three arms that were
    /// missing from this port entirely. `linredir` outranks the
    /// `switch (linwhat)` but is outranked by `lincmd`; `linarr`
    /// upgrades IN_ENV from `value` to `array_value`; and the default
    /// arm degrades to `value` when no command word was parsed.
    #[test]
    fn compcontext_for_redirect_array_value_and_cmdstr_arms() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        use crate::ported::zle::zle_tricky::{CMDSTR, INSUBSCR, LINARR, LINCMD, LINREDIR};
        ispar.store(0, Ordering::Relaxed);
        INSUBSCR.store(0, Ordering::Relaxed);
        LINCMD.store(0, Ordering::Relaxed);
        LINARR.store(0, Ordering::Relaxed);
        *CMDSTR.lock().unwrap() = Some("echo".into());

        // c:598-599 — `echo x > /tm<TAB>`.
        linwhat.store(IN_NOTHING_LW, Ordering::Relaxed);
        LINREDIR.store(1, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "redirect");
        // c:592 — but a command-position word still wins over it.
        LINCMD.store(1, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "command");
        LINCMD.store(0, Ordering::Relaxed);
        LINREDIR.store(0, Ordering::Relaxed);

        // c:605-606 — IN_ENV splits on `linarr`.
        linwhat.store(IN_ENV_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "value");
        LINARR.store(1, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "array_value");
        LINARR.store(0, Ordering::Relaxed);

        // c:623-626 — the default arm needs a command word to say
        // "command"; without one it is a `value`.
        linwhat.store(IN_NOTHING_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "command");
        *CMDSTR.lock().unwrap() = None;
        assert_eq!(compcontext_for("x"), "value");
    }

    /// c:582-597 — with `insubscr` set, BOTH the math arm and the command
    /// arm select the `subscript` context (so `_complete` dispatches
    /// `-subscript-`). Regression for `echo $fpath[<TAB>`, which routed to
    /// `math` and never reached `_subscript`.
    #[test]
    fn compcontext_for_routes_subscript_when_insubscr_set() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        use crate::ported::zle::zle_tricky::{CMDSTR, INSUBSCR, LINCMD, LINREDIR};
        ispar.store(0, Ordering::Relaxed);
        LINCMD.store(0, Ordering::Relaxed);
        LINREDIR.store(0, Ordering::Relaxed);
        *CMDSTR.lock().unwrap() = Some("ls".into());

        // c:584 — math context inside an unclosed subscript.
        linwhat.store(IN_MATH_LW, Ordering::Relaxed);
        INSUBSCR.store(1, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "subscript");
        // c:590 — a real math expression stays `math`.
        INSUBSCR.store(0, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "math");

        // c:593-597 — `[` in command position is a subscript, not a command.
        linwhat.store(IN_NOTHING_LW, Ordering::Relaxed);
        LINCMD.store(1, Ordering::Relaxed);
        INSUBSCR.store(1, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "subscript");
        INSUBSCR.store(0, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "command");
        LINCMD.store(0, Ordering::Relaxed);
        *CMDSTR.lock().unwrap() = None;
    }

    #[test]
    fn addmatches_empty_argv_early_return() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:2138-2139: empty argv + dummies==0 + no CAF_ALL → return 1.
        let mut dat = Cadata::default();
        dat.dummies = 0;
        dat.aflags = 0;
        assert_eq!(addmatches(&mut dat, &[]), 1);
    }

    #[test]
    fn addmatches_appends_argv_to_default_group() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:2200 simplified body: each argv entry → addmatch into "default" group.
        amatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .clear();
        crate::comp_match_handles::matches_arc()
            .lock()
            .unwrap()
            .clear();
        let mut dat = Cadata::default();
        dat.dummies = -1;
        let _ = addmatches(&mut dat, &["a".into(), "b".into()]);
        let n = crate::comp_match_handles::matches_arc()
            .lock()
            .unwrap()
            .len();
        assert!(n >= 2);
    }

    /// c:2094 `Cmlist oms = mstack` / c:2623 `mstack = oms` — a `compadd -M`
    /// matcher lives only for the duration of that one call. When the port
    /// leaked it, `_arguments`' option matcher `r:|[_-]=* r:|=*` stayed live
    /// for the `_hosts` compadd that followed and a typed `-` matched every
    /// host CONTAINING a `-`, so `ssh -<TAB>` listed
    /// `ec2-…compute-1.amazonaws.com` among the options.
    #[test]
    fn addmatches_restores_mstack_after_the_call() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        fn mstack_depth() -> usize {
            let g = mstack.get_or_init(|| Mutex::new(None)).lock().unwrap();
            let mut n = 0;
            let mut cur = g.as_deref();
            while let Some(link) = cur {
                n += 1;
                cur = link.next.as_deref();
            }
            n
        }
        let before = mstack_depth();
        let mut dat = Cadata::default();
        dat.dummies = -1;
        dat.match_ = crate::ported::zle::complete::parse_cmatcher("compadd", "r:|[_-]=* r:|=*");
        assert!(dat.match_.is_some(), "matcher spec must parse");
        let _ = addmatches(&mut dat, &["a-b".into()]);
        assert_eq!(mstack_depth(), before, "matcher leaked past its compadd");
    }

    #[test]
    fn add_match_data_returns_populated_cmatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:3052-3067: cm.str/orig/pre/suf populated; mnum bumps by 1.
        crate::comp_match_handles::matches_arc()
            .lock()
            .unwrap()
            .clear();
        let before = mnum.load(Ordering::Relaxed);
        let cm = add_match_data(
            0,
            "match",
            "match-orig",
            None,
            "ipre",
            "ripre",
            "isuf",
            Some("pre"),
            "prpre",
            "ppre",
            None,
            "psuf",
            None,
            Some("suf"),
            0,
            0,
        );
        assert_eq!(cm.str.as_deref(), Some("match"));
        assert_eq!(cm.orig.as_deref(), Some("match-orig"));
        assert_eq!(cm.pre.as_deref(), Some("pre"));
        assert_eq!(cm.suf.as_deref(), Some("suf"));
        assert_eq!(mnum.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn add_match_data_exact_records_into_ainfo() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:3037-3058: exact != 0 writes `ai->exact = useexact` and
        // `ai->exactm = cm`. The test sets useexact=1 to exercise the
        // accept-exact path.
        let saved_useexact = useexact.load(Ordering::Relaxed);
        useexact.store(1, Ordering::Relaxed);
        if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Aminfo::default());
        }
        let _ = add_match_data(
            0,
            "x",
            "x",
            None,
            "",
            "",
            "",
            Some(""),
            "",
            "",
            None,
            "",
            None,
            Some(""),
            0,
            1,
        );
        let a = ainfo.get().unwrap().lock().unwrap().clone().unwrap();
        useexact.store(saved_useexact, Ordering::Relaxed);
        assert_eq!(a.exact, 1);
        assert!(a.exactm.is_some());
    }

    /// Faithful end-to-end exercise of `set_comp_sep` (`compset -q`): an
    /// ASCII argument `a b c` with the cursor at the end of the middle
    /// word must be re-lexed into three words, narrowed to the cursor
    /// word `b`, and split into qp/qs ignored prefix/suffix around it.
    /// Drives the real lexer (via `global_state_lock`'s `inittyptab`) and
    /// asserts the word split, cursor index, compprefix, and the qp/qs
    /// reconstruction — covering the marker/offset arithmetic
    /// (`swb-1-sqq+dq`, `p[soffs]` chuck) for the byte-consistent case.
    #[test]
    fn set_comp_sep_splits_ascii_word() {
        use crate::ported::zle::complete::{
            COMPCURRENT, COMPISUFFIX, COMPQIPREFIX, COMPQISUFFIX, COMPQUOTE, COMPWORDS,
        };
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();

        let setg = |g: &'static OnceLock<Mutex<String>>, v: &str| {
            *g.get_or_init(|| Mutex::new(String::new())).lock().unwrap() = v.to_string();
        };
        let getg = |g: &'static OnceLock<Mutex<String>>| -> String {
            g.get_or_init(|| Mutex::new(String::new()))
                .lock()
                .unwrap()
                .clone()
        };

        // Reconstructed arg "a b c"; cursor between 'b' and the following
        // space: compprefix="a b" (active-prefix len 3), compsuffix=" c".
        setg(&COMPPREFIX, "a b");
        setg(&COMPSUFFIX, " c");
        setg(&COMPIPREFIX, "");
        setg(&COMPISUFFIX, "");
        setg(&COMPQIPREFIX, "");
        setg(&COMPQISUFFIX, "");
        setg(&COMPQSTACK, ""); // empty => qttype = QT_NONE (unquoted)
        setg(&COMPQUOTE, "");
        OFFS.store(0, Ordering::Relaxed);
        WB.store(0, Ordering::Relaxed);
        WE.store(0, Ordering::Relaxed);
        ZLEMETACS.store(0, Ordering::Relaxed);
        INSTRING.store(0, Ordering::Relaxed);
        INBACKT.store(0, Ordering::Relaxed);

        // c:1721 — a real split happened (cur >= 0), so ret == 0.
        assert_eq!(set_comp_sep(), 0, "compset -q must split the ASCII word");

        // c:1926-1934 — three words, cursor word 'b' with its injected
        // 'x' chucked back out.
        let words = COMPWORDS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .clone();
        assert_eq!(
            words,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "arg must re-lex into three words"
        );
        // c:1930 — 0-offset cur=1 -> 1-based compcurrent=2.
        assert_eq!(COMPCURRENT.load(Ordering::Relaxed), 2);

        // c:1894-1906 — prefix/suffix of the cursor word (COMPLETEINWORD off).
        assert_eq!(getg(&COMPPREFIX), "b");
        assert_eq!(getg(&COMPSUFFIX), "");

        // c:1912-1919 — qp/qs fold into compqiprefix/compqisuffix. The
        // just-prepended 1-char compqstack makes multiquote(...,1) a
        // no-op, so the ignored prefix/suffix are verbatim slices of the
        // arg; qip + word + qis must reconstruct "a b c".
        let recon = format!(
            "{}{}{}",
            getg(&COMPQIPREFIX),
            getg(&COMPPREFIX),
            getg(&COMPQISUFFIX)
        );
        assert_eq!(recon, "a b c", "qip + word + qis must reconstruct the arg");

        // c:1926-1934 + c:complete.c:1235-1295 — in C `$words` / `$CURRENT`
        // ARE `compwords` / `compcurrent` (gsu-bound, one storage), so the
        // re-split is visible to the calling completer the moment `compset -q`
        // returns. zshrs's params are separate paramtab copies, so the port
        // has to publish them explicitly. Without that publish `_trap`
        //     if [[ CURRENT -eq 2 ]]; then compset -q; _normal; else …
        // left `$words` as `(trap -)` with `$CURRENT` still 2, `_normal`
        // re-dispatched the command word `trap`, and `_trap` recursed until
        // FUNCNEST — printing `maximum nested function level reached` into
        // the user's edit buffer on `trap -<TAB>`.
        assert_eq!(
            crate::ported::params::getaparam("words"),
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()]),
            "compset -q must republish $words for the calling completer"
        );
        assert_eq!(
            crate::ported::params::getsparam("CURRENT").as_deref(),
            Some("2"),
            "compset -q must republish $CURRENT for the calling completer"
        );
        assert_eq!(
            crate::ported::params::getsparam("PREFIX").as_deref(),
            Some("b"),
            "compset -q must republish $PREFIX for the calling completer"
        );
    }

    #[test]
    fn foredel_deletes_forward_from_zlemetacs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // zle_utils.c:1105 — delete `ct` chars forward from ZLEMETACS.
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = "abcdef".to_string();
        }
        ZLEMETACS.store(2, Ordering::Relaxed);
        ZLEMETALL.store(6, Ordering::Relaxed);
        foredel(3, CUT_RAW);
        let line = ZLEMETALINE.get().unwrap().lock().unwrap().clone();
        assert_eq!(line, "abf");
        assert_eq!(ZLEMETALL.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn inststr_inserts_at_zlemetacs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // zle_tricky.c:278 — insert at cursor.
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = "hello".to_string();
        }
        ZLEMETACS.store(5, Ordering::Relaxed);
        ZLEMETALL.store(5, Ordering::Relaxed);
        let _ = inststr(" world");
        let line = ZLEMETALINE.get().unwrap().lock().unwrap().clone();
        assert_eq!(line, "hello world");
        assert_eq!(ZLEMETACS.load(Ordering::Relaxed), 11);
    }

    #[test]
    fn metafy_and_unmetafy_roundtrip_globals() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // zle_tricky.c:978,995 — meta/unmeta operate on the global pair.
        if let Ok(mut g) = ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = "plain ascii".to_string();
        }
        ZLECS.store(3, Ordering::Relaxed);
        ZLELL.store(11, Ordering::Relaxed);
        metafy_line();
        // For ASCII input the meta form equals the raw form.
        assert_eq!(
            ZLEMETALINE.get().unwrap().lock().unwrap().clone(),
            "plain ascii"
        );
        assert_eq!(ZLEMETACS.load(Ordering::Relaxed), 3);
        unmetafy_line();
        assert_eq!(
            ZLELINE.get().unwrap().lock().unwrap().clone(),
            "plain ascii"
        );
    }

    #[test]
    fn selfinsert_appends_lastchar_at_zlecs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // zle_misc.c:112-141 — insert one char at cursor, bump zlecs.
        //
        // `selfinsert()` (zle_misc.rs:180) writes through
        // `self_insert(c)` which mutates the `zle_main::ZLELINE`
        // (`Mutex<Vec<char>>`) plus `zle_main::ZLECS`/`ZLELL` —
        // NOT the `compcore::ZLELINE` (`OnceLock<Mutex<String>>`)
        // used by the meta/unmeta tests above. The original test
        // seeded the wrong buffer, so the assert kept seeing "ab"
        // (the compcore buffer never received the insert) while
        // self_insert silently appended 'c' to the zle_main buffer.
        //
        // Set up the zle_main buffer for the insert, then read back
        // from there. `zle_test_setup` already clears the zle_main
        // statics, so we start from a known zero state.
        {
            let mut g = crate::ported::zle::zle_main::ZLELINE.lock().unwrap();
            *g = "ab".chars().collect();
        }
        crate::ported::zle::zle_main::ZLECS.store(2, Ordering::Relaxed);
        crate::ported::zle::zle_main::ZLELL.store(2, Ordering::Relaxed);
        LASTCHAR.store(b'c' as i32, Ordering::Relaxed);
        // Force the wide-char re-derive path (lastchar_wide_valid=0 →
        // selfinsert refills it from LASTCHAR per zle_misc.c:119-122).
        LASTCHAR_WIDE_VALID.store(0, Ordering::Relaxed);
        let rv = selfinsert(&[]);
        assert_eq!(rv, 0);
        let buf: String = crate::ported::zle::zle_main::ZLELINE
            .lock()
            .unwrap()
            .iter()
            .collect();
        assert_eq!(buf, "abc");
        assert_eq!(
            crate::ported::zle::zle_main::ZLECS.load(Ordering::Relaxed),
            3,
        );
    }

    #[test]
    fn minfo_clear_and_asked_zero_mutate_state() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(Menuinfo::default())).lock() {
            let mut cm = Cmatch::default();
            cm.str = Some("x".into());
            g.cur = Some(Box::new(cm));
            g.asked = 1;
        }
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(Menuinfo::default())).lock() {
            g.cur = None;
        }
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(Menuinfo::default())).lock() {
            g.asked = 0;
        }
        let m = MINFO.get().unwrap().lock().unwrap().clone();
        assert!(m.cur.is_none());
        assert_eq!(m.asked, 0);
    }

    #[test]
    fn cline_matched_stub_marks_node() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // compmatch.c:253 — sets CLF_MATCHED on the node chain. We
        // verify by running through the stub on a non-empty string
        // without panicking and trusting compmatch's body for the
        // actual flag set.
        cline_matched_compcore(Some("foo"));
        cline_matched_compcore(None);
        cline_matched_compcore(Some(""));
    }

    #[test]
    fn permmatches_returns_fi_zero_when_count_present() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:3444-3447: if ainfo->count is non-zero, fi stays 0.
        amatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .clear();
        pmatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .clear();
        if let Ok(mut a) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
            *a = Some(Aminfo {
                count: 5,
                ..Default::default()
            });
        }
        newmatches.store(1, Ordering::Relaxed);
        let fi = permmatches(0);
        assert_eq!(fi, 0);
        assert_eq!(hasperm.load(Ordering::Relaxed), 1);
    }

    /// c:2068 / c:3010 / c:3154 / c:3162 — `mgroup->new = 1` MUST be visible
    /// through the `amatches` chain, because in C `mgroup` and the `amatches`
    /// entry are the same `struct cmgroup` (`begcmgroup` c:3087 on reuse,
    /// c:3100+c:3123 on create). `permmatches` reads the flag while walking
    /// `amatches` (c:3452 `if (fi != ofi || !g->perm || g->new)`) and clears it
    /// there (c:3544).
    ///
    /// With `new_` a per-clone `i32`, the write landed only on the `mgroup`
    /// copy, so an OPEN group's freshly added matches were invisible: the c:3452
    /// test took its reuse branch and c:3536 added the group's STALE `mcount` to
    /// `nmatches`. `$compstate[nmatches]` is live through `get_nmatches`
    /// (`Src/Zle/complete.c:1411-1413`), so every completer that returns
    /// `[[ nm -ne compstate[nmatches] ]]` — `Completion/Unix/Type/_path_files`
    /// sh:895 — reported "added nothing" and the whole completer chain ran past
    /// the completer that should have ended it.
    #[test]
    fn mgroup_new_flag_is_shared_with_the_amatches_entry() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        amatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .clear();

        begcmgroup(Some("corrections"), 0); // c:3072
                                            // c:2068 — exactly the write `addmatch` performs, through `mgroup`.
        mgroup
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .as_ref()
            .expect("begcmgroup must leave a current group")
            .new_
            .store(1, Ordering::Relaxed);

        let seen = amatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .iter()
            .find(|g| g.name.as_deref() == Some("corrections"))
            .map(|g| g.new_.load(Ordering::Relaxed));
        assert_eq!(
            seen,
            Some(1),
            "c:3452 permmatches reads `g->new` off the amatches walk, so the \
             mgroup write must alias it"
        );

        // c:3544 — and the clear on the amatches walk must reach `mgroup` too.
        amatches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .iter()
            .for_each(|g| g.new_.store(0, Ordering::Relaxed));
        assert_eq!(
            mgroup
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .new_
                .load(Ordering::Relaxed),
            0
        );
    }

    /// c:1323 — `rembslash` removes backslash escapes by walking the
    /// string and dropping every `\` while keeping its successor.
    /// `\\\\` (literal `\\`) → `\` (single backslash); `\a` → `a`.
    /// A regression that drops the successor too would silently strip
    /// real chars from `path/to/\$file`.
    #[test]
    fn rembslash_unescapes_canonical_pairs() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(r"\a"), "a");
        assert_eq!(rembslash(r"\\"), r"\");
        assert_eq!(rembslash(r"\$foo"), "$foo");
        assert_eq!(rembslash("plain"), "plain");
    }

    /// c:1323 — empty input → empty output (the loop never runs).
    /// Catches a regression that returns " " or "\0" for empty input.
    #[test]
    fn rembslash_empty_input_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(""), "");
    }

    /// c:1323 — trailing lone `\` MUST silently drop (no following
    /// char to keep). C's pattern `if (let Some) { push }` is
    /// equivalent. Regression that pushes the literal `\` would break
    /// shell paths with trailing backslashes (rare but legal).
    #[test]
    fn rembslash_trailing_lone_backslash_drops_silently() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(r"foo\"), "foo");
    }

    /// c:1366 — `ctokenize` is the inverse of `untokenize`: escapes
    /// shell metacharacters into their tokenised forms used by the
    /// completion machinery. Plain alphanumerics pass through.
    #[test]
    fn ctokenize_passes_alphanumerics_through() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ctokenize("foo123"), "foo123");
        assert_eq!(ctokenize(""), "");
        assert_eq!(ctokenize("path/to"), "path/to");
    }

    /// c:1435 — `comp_quoting_string` returns one of the canonical
    /// quote strings: `'`, `"`, `$'`, or "" depending on `stype`.
    /// Catches a regression where the dispatch returns the wrong
    /// quote — completion would generate `cmd 'arg"` (mismatched).
    #[test]
    fn comp_quoting_string_dispatches_known_styles() {
        let _g = crate::test_util::global_state_lock();
        // The exact stype values are private to the completion impl,
        // but the function MUST return a non-panicking string for
        // every reasonable input. Probe a few values.
        for stype in 0..=8 {
            let _ = comp_quoting_string(stype);
        }
    }

    /// c:1065 — `multiquote` with empty COMPQSTACK is a no-op (returns
    /// input unchanged). The stack is the per-completion quoting
    /// context; outside completion it's empty. Regression that quotes
    /// regardless would corrupt every non-completion caller.
    #[test]
    fn multiquote_empty_stack_returns_input_unchanged() {
        let _g = crate::test_util::global_state_lock();
        // Reset COMPQSTACK to empty.
        if let Some(c) = COMPQSTACK.get() {
            if let Ok(mut g) = c.lock() {
                g.clear();
            }
        }
        assert_eq!(multiquote("hello", 0), "hello");
        assert_eq!(multiquote("", 0), "");
    }

    /// c:1073 — `multiquote` ESCAPES for the active quoting level; it must
    /// never WRAP the candidate in quote characters. `comp_match`
    /// (compmatch.c:1172) runs every compadd candidate through it and then
    /// matches the result against `$PREFIX` (compmatch.c:1178), so a wrapped
    /// candidate can never share a prefix with the typed word.
    ///
    /// Regression pinned: `quotestring(QT_DOUBLE)` used to return `"abcdef"`
    /// (with the quote pair) instead of the body `abcdef`, against C's
    /// contract at utils.c:6131-6134. Inside a quoted word — `cmd "ab<TAB>`,
    /// where compqstack holds QT_DOUBLE — `comp_match` compared `ab` to
    /// `"abcdef"`, rejected it, and `compadd` produced ZERO matches, so
    /// completion silently did nothing for EVERY quoted word.
    #[test]
    fn multiquote_escapes_but_never_wraps_the_candidate() {
        let _g = crate::test_util::global_state_lock();
        for (qt, label) in [
            (QT_DOUBLE, "QT_DOUBLE"),
            (QT_SINGLE, "QT_SINGLE"),
            (QT_DOLLARS, "QT_DOLLARS"),
            (QT_BACKSLASH, "QT_BACKSLASH"),
        ] {
            if let Ok(mut g) = COMPQSTACK.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = (qt as u8 as char).to_string(); // c:305-306
            }
            assert_eq!(
                multiquote("abcdef", 0),
                "abcdef",
                "{label}: a candidate with nothing to escape must pass through \
                 unchanged so comp_match can prefix-match it"
            );
        }
        if let Some(c) = COMPQSTACK.get() {
            if let Ok(mut g) = c.lock() {
                g.clear();
            }
        }
    }

    /// c:1092 — `tildequote("foo")` (no leading ~) MUST behave like
    /// multiquote — the tilde-special path is a no-op when there's no
    /// `~` to protect. Regression that always strips/restores would
    /// silently mangle non-tilde inputs.
    #[test]
    fn tildequote_non_tilde_input_unchanged() {
        let _g = crate::test_util::global_state_lock();
        // Empty COMPQSTACK + no ~ → input unchanged.
        if let Some(c) = COMPQSTACK.get() {
            if let Ok(mut g) = c.lock() {
                g.clear();
            }
        }
        assert_eq!(tildequote("foo/bar", 0), "foo/bar");
    }

    /// c:1092 — empty input through tildequote is empty out.
    #[test]
    fn tildequote_empty_input_empty_output() {
        let _g = crate::test_util::global_state_lock();
        if let Some(c) = COMPQSTACK.get() {
            if let Ok(mut g) = c.lock() {
                g.clear();
            }
        }
        assert_eq!(tildequote("", 0), "");
    }

    // ─── zsh-corpus pins for rembslash ─────────────────────────────

    /// `rembslash("\\a\\b\\c")` strips backslashes → "abc".
    #[test]
    fn compcore_corpus_rembslash_strips_escapes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(r"\a\b\c"), "abc");
    }

    /// `rembslash` with no backslashes is identity.
    #[test]
    fn compcore_corpus_rembslash_no_escapes_identity() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash("hello"), "hello");
    }

    /// `rembslash("")` returns empty string.
    #[test]
    fn compcore_corpus_rembslash_empty_is_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(""), "");
    }

    /// `rembslash("\\\\")` (escaped backslash) returns "\\".
    #[test]
    fn compcore_corpus_rembslash_escaped_backslash() {
        let _g = crate::test_util::global_state_lock();
        // r"\\" in Rust is the 2-char string "\\\\"  → one literal backslash + one literal backslash
        assert_eq!(rembslash(r"\\"), r"\");
    }

    /// `rembslash` mid-string escape preserves surroundings.
    #[test]
    fn compcore_corpus_rembslash_preserves_context() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(r"hello\ world"), "hello world");
    }

    /// Trailing single backslash is consumed (no char after).
    #[test]
    fn compcore_corpus_rembslash_trailing_backslash_consumed() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(r"abc\"), "abc");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compcore.c rembslash + remsquote
    // + ctokenize + multiquote / tildequote string transforms.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1323 — `rembslash("abc")` (no backslash) is identity.
    #[test]
    fn rembslash_no_backslash_is_identity() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash("abc"), "abc");
        assert_eq!(rembslash("hello world"), "hello world");
    }

    /// c:1323 — `rembslash` of single backslash + char drops the backslash.
    #[test]
    fn rembslash_drops_backslash_keeps_next() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(r"\a"), "a");
        assert_eq!(rembslash(r"\."), ".");
        assert_eq!(rembslash(r"\$"), "$");
    }

    /// c:1323 — repeated escapes: every `\X` collapses to `X`.
    #[test]
    fn rembslash_multiple_escapes_chain() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembslash(r"\a\b\c"), "abc");
    }

    /// c:1343 — `remsquote("")` returns 0 (no chars consumed).
    #[test]
    fn remsquote_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut s = String::new();
        let r = remsquote(&mut s);
        assert_eq!(r, 0);
        assert_eq!(s, "");
    }

    /// c:1343 — `remsquote("abc")` (no quote sequences) is identity,
    /// returns 0.
    #[test]
    fn remsquote_no_quotes_is_identity() {
        let _g = crate::test_util::global_state_lock();
        let mut s = String::from("hello world");
        let r = remsquote(&mut s);
        assert_eq!(r, 0, "no quote sequences → 0");
        assert_eq!(s, "hello world", "string unchanged");
    }

    /// c:1366 — `ctokenize("")` returns empty string.
    #[test]
    fn ctokenize_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ctokenize(""), "");
    }

    /// c:1366 — `ctokenize("abc")` (no special chars) preserves content.
    #[test]
    fn ctokenize_plain_ascii_preserved() {
        let _g = crate::test_util::global_state_lock();
        // No $, {, }, or backslash → byte-for-byte preservation.
        let r = ctokenize("abc");
        assert_eq!(r.as_bytes()[0], b'a');
        assert_eq!(r.as_bytes()[1], b'b');
        assert_eq!(r.as_bytes()[2], b'c');
    }

    /// c:1505 — `comp_quoting_string(0)` returns a static str (no panic).
    #[test]
    fn comp_quoting_string_returns_static_str_for_all_stypes() {
        let _g = crate::test_util::global_state_lock();
        for stype in 0..10 {
            let _s = comp_quoting_string(stype);
            // No panic = pass; returned &'static str is well-defined.
        }
    }

    /// c:954 — `multiquote("")` returns empty.
    #[test]
    fn multiquote_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = multiquote("", 0);
        assert_eq!(r, "");
    }

    /// c:980 — `tildequote("")` returns empty.
    #[test]
    fn tildequote_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = tildequote("", 0);
        assert_eq!(r, "");
    }

    /// c:980 — `tildequote("plain")` (no tilde) is identity.
    #[test]
    fn tildequote_no_tilde_is_identity() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = tildequote("plain", 0);
        assert_eq!(r, "plain", "no tilde → input unchanged");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compcore.c
    // c:936 multiquote / c:972 tildequote / c:1014 check_param /
    // c:1315 rembslash / c:1338 remsquote / c:1381 ctokenize /
    // c:1478 comp_quoting_string / c:2809 matchcmp / c:2872 matcheq
    // ═══════════════════════════════════════════════════════════════════

    /// c:1315 — `rembslash("")` empty returns empty.
    #[test]
    fn rembslash_empty_returns_empty() {
        assert_eq!(rembslash(""), "");
    }

    /// c:1315 — `rembslash` is pure.
    #[test]
    fn rembslash_is_pure() {
        for s in ["", "abc", r"\a", r"\\\\"] {
            let first = rembslash(s);
            for _ in 0..3 {
                assert_eq!(rembslash(s), first, "rembslash({:?}) must be pure", s);
            }
        }
    }

    /// c:1381 — `ctokenize` is deterministic.
    #[test]
    fn ctokenize_is_deterministic() {
        for s in ["", "abc", "a*b", "a?b"] {
            let first = ctokenize(s);
            for _ in 0..3 {
                assert_eq!(
                    ctokenize(s),
                    first,
                    "ctokenize({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:1478 — `comp_quoting_string(0)` returns non-empty static.
    #[test]
    fn comp_quoting_string_zero_returns_static_str() {
        let _: &'static str = comp_quoting_string(0);
    }

    /// c:936 — `multiquote` is pure for arbitrary input.
    #[test]
    fn multiquote_is_pure() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for s in ["", "abc", "x y"] {
            let first = multiquote(s, 0);
            for _ in 0..3 {
                assert_eq!(
                    multiquote(s, 0),
                    first,
                    "multiquote({:?}, 0) must be pure",
                    s
                );
            }
        }
    }

    /// c:972 — `tildequote` is pure for non-tilde input.
    #[test]
    fn tildequote_is_pure() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for s in ["", "abc", "no_tilde", "/path/to/file"] {
            let first = tildequote(s, 0);
            for _ in 0..3 {
                assert_eq!(
                    tildequote(s, 0),
                    first,
                    "tildequote({:?}, 0) must be pure",
                    s
                );
            }
        }
    }

    /// c:1338 — `remsquote(&mut empty)` returns 0.
    #[test]
    fn remsquote_empty_returns_zero_pin() {
        let mut s = String::new();
        let r = remsquote(&mut s);
        assert_eq!(r, 0, "empty input → 0");
    }

    /// c:1014 — `check_param("")` empty returns Option<usize>.
    #[test]
    fn check_param_empty_returns_option_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Option<usize> = check_param("", false, false);
    }

    /// c:1014 — `check_param` is deterministic for empty input.
    #[test]
    fn check_param_empty_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = check_param("", false, false);
        for _ in 0..3 {
            assert_eq!(check_param("", false, false), first);
        }
    }

    /// c:1439 — `comp_str(false)` returns (String, i32, i32) tuple.
    #[test]
    fn comp_str_returns_string_i32_i32_tuple() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: (String, i32, i32) = comp_str(false);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compcore.c
    // c:1505 set_comp_sep / c:1544 set_list_array / c:1555 get_user_var /
    // c:1645 get_data_arr / c:2681 begcmgroup / c:2735 endcmgroup /
    // c:2749 addexpl / c:1478 comp_quoting_string
    // ═══════════════════════════════════════════════════════════════════

    /// c:1505 — `set_comp_sep` returns i32 (compile-time type pin).
    #[test]
    fn set_comp_sep_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = set_comp_sep();
    }

    /// c:1555 — `get_user_var(None)` returns Option<Vec<String>>.
    #[test]
    fn get_user_var_returns_option_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Option<Vec<String>> = get_user_var(None);
    }

    /// c:1555 — `get_user_var(None)` is deterministic.
    #[test]
    fn get_user_var_none_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let first = get_user_var(None);
        for _ in 0..3 {
            assert_eq!(
                get_user_var(None),
                first,
                "get_user_var(None) must be deterministic"
            );
        }
    }

    /// c:1645 — `get_data_arr("", false)` returns Option<Vec<String>>.
    #[test]
    fn get_data_arr_returns_option_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Option<Vec<String>> = get_data_arr("", false);
    }

    /// c:1645 — `get_data_arr("", _)` empty name returns None.
    #[test]
    fn get_data_arr_empty_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert!(get_data_arr("", false).is_none(), "empty name → None");
        assert!(
            get_data_arr("", true).is_none(),
            "empty name (keys=true) → None"
        );
    }

    /// c:2681 — `begcmgroup(None, 0)` safe.
    #[test]
    fn begcmgroup_none_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        begcmgroup(None, 0);
    }

    /// c:2735 — `endcmgroup(None)` safe.
    #[test]
    fn endcmgroup_none_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        endcmgroup(None);
    }

    /// c:2681 + c:2735 — beg/end cmgroup round-trip safe.
    #[test]
    fn begcmgroup_endcmgroup_round_trip_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..3 {
            begcmgroup(None, 0);
            endcmgroup(None);
        }
    }

    /// c:2749 — `addexpl(false)` safe.
    #[test]
    fn addexpl_false_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        addexpl(false);
    }

    /// c:1544 — `set_list_array("", &[])` empty inputs is safe.
    #[test]
    fn set_list_array_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        set_list_array("", &[]);
    }

    /// c:1478 — `comp_quoting_string(N)` returns &'static str (type pin).
    #[test]
    fn comp_quoting_string_returns_static_str_type() {
        let _: &'static str = comp_quoting_string(0);
    }

    /// c:1478 — `comp_quoting_string` is pure across stypes.
    #[test]
    fn comp_quoting_string_pure_across_stypes() {
        for stype in 0..10 {
            let first = comp_quoting_string(stype);
            for _ in 0..3 {
                assert_eq!(
                    comp_quoting_string(stype),
                    first,
                    "comp_quoting_string({}) must be pure",
                    stype
                );
            }
        }
    }
}
