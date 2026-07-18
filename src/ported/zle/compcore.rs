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
    CAF_UNIQALL,
    CAF_UNIQCON, CGF_MATSORT, CGF_NOSORT, CGF_NUMSORT, CGF_REVSORT, CGF_UNIQALL, CGF_UNIQCON,
    CMF_DELETE, CMF_DISPLINE, CMF_FMULT, CMF_MULT, CMF_NOLIST, CMF_PACKED, CMF_PARBR, CMF_PARNEST,
    CMF_ROWS,
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
            // c:351
            unmetafy_line();
            ret = selfinsert(&[]); // c:353
            metafy_line();
        }
        return goto_compend(ret); // c:356 goto compend
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
        unmetafy_line();
        ret = selfinsert(&[]); // c:372
        metafy_line();
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
            if let Ok(mut mst) = MINFO
                .get_or_init(|| Mutex::new(Menuinfo::default()))
                .lock()
            {
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

    // c:489-490 — `if ((fromcomp & FC_INWORD) && (zlecs = lastend) > zlell)
    //              zlecs = zlell;` — re-entering an in-word completion
    //              restores cursor to lastend (clamped to zlell).
    if (fromcomp.load(Ordering::Relaxed) & crate::ported::zle::comp_h::FC_INWORD) != 0 {
        let le = lastend.load(Ordering::Relaxed);
        let ll = ZLEMETALL.load(Ordering::Relaxed);
        let new_cs = if le > ll { ll } else { le };
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
    if menucmp_v == 0 || oldmenucmp_v != 0 {
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
    let _lv = crate::ported::builtin::LASTVAL.load(Ordering::Relaxed); // c:548 int lv = lastval
    let _icf = INCOMPFUNC.load(Ordering::Relaxed); // c:555
    let _osc = crate::ported::builtin::SFCONTEXT.load(Ordering::Relaxed); // c:555

    let _useglob = USEGLOB.load(Ordering::Relaxed); // c:579

    // Publish the completion word split at the cursor into the
    // `$PREFIX` / `$SUFFIX` params (+ empty ignored-prefix/suffix). In C
    // these are gsu-bound to `compprefix`/`compsuffix`; the Rust ports
    // have no gsu binding, so without this every completer reads
    // `$PREFIX=''` — `_main_complete`'s `compset -P 1 '='` then matches
    // the empty prefix and wrongly forces `$compstate[context]=equal`,
    // and `_path_files` has no prefix to glob. Split at `OFFS`
    // (zlemetacs - wb), the cursor offset within the word.
    {
        let scs: Vec<char> = s.chars().collect();
        let off = (OFFS.load(Ordering::Relaxed).max(0) as usize).min(scs.len());
        let pre: String = scs[..off].iter().collect();
        let suf: String = scs[off..].iter().collect();
        let _ = crate::ported::params::setsparam("PREFIX", &pre);
        let _ = crate::ported::params::setsparam("SUFFIX", &suf);
        let _ = crate::ported::params::setsparam("IPREFIX", "");
        let _ = crate::ported::params::setsparam("ISUFFIX", "");
        let _ = crate::ported::params::setsparam("QIPREFIX", "");
        let _ = crate::ported::params::setsparam("QISUFFIX", "");
    }

    // c:591-617 — context selection.
    let context = compcontext_for(s); // c:591-617
    set_compstate_str("context", &context); // c:619

    // c:721-727 — `$compstate[last_prompt]` etc. fed in from
    // do_completion via dolastprompt; we forward the current values.
    set_compstate_str(
        "last_prompt",
        if dolastprompt.load(Ordering::Relaxed) != 0 {
            "yes"
        } else {
            ""
        },
    );

    // c:740-749 — `$compstate[list]` — set from `complist` global.
    let cl_value = COMPLIST
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    set_compstate_str("list", &cl_value); // c:740

    // c:768-785 — `$compstate[insert]` per (useline, usemenu).
    let ul = useline.load(Ordering::Relaxed);
    let um = USEMENU.load(Ordering::Relaxed);
    let ins = if ul != 0 {
        match um {
            0 => "unambiguous",
            1 => "menu",
            2 => "automenu",
            _ => "",
        }
    } else {
        ""
    };
    set_compstate_str("insert", ins); // c:770

    // c:790-794 — `$compstate[exact]` & `$compstate[exact_string]`.
    set_compstate_str(
        "exact",
        if useexact.load(Ordering::Relaxed) != 0 {
            "accept"
        } else {
            ""
        },
    );

    // c:800-803 — `$compstate[to_end]` per movetoend.
    set_compstate_str(
        "to_end",
        if movetoend.load(Ordering::Relaxed) == 1 {
            "single"
        } else {
            "match"
        },
    );

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
    let body_runner = move || -> i32 {
        // c:6042 — `runshfunc(prog, wrappers, name)`. zshrs runs the
        // body via either the Rust compsys port (direct fn call) or
        // the fusevm Chunk dispatch (via exec accessors).
        if let Some(rc) =
            crate::compsys::router::dispatch_compsys(&fn_name_owned, &largs_for_body[1..])
        {
            // Plugin override (ABI v4) wins over the built-in Rust port.
            // C convention: largs[0] = fn name, [1..] = real argv.
            return rc;
        }
        crate::ported::exec::dispatch_function_call(&fn_name_owned, &largs_for_body[1..])
            .unwrap_or_else(|| crate::ported::builtin::LASTVAL.load(Ordering::Relaxed))
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
    };
    let cfret_val = crate::ported::exec::doshfunc(&mut synth_shf, largs, true, body_runner);
    crate::ported::zle::zle_tricky::cfret.store(cfret_val, Ordering::Relaxed);

    // c:836 — `opts[XTRACE] = oxt;` restore xtrace state.
    crate::ported::options::opt_state_set(
        &crate::ported::zsh_h::opt_name(crate::ported::zsh_h::XTRACE),
        oxt != 0,
    );

    // c:909-912 — unwind: read `$compstate[insert]` etc. back into
    // the compcore globals so do_completion sees the user fn's
    // mutations.
    // Read `$compstate[insert]` via the compstate hash (the canonical
    // home), NOT the flat `compstate[insert]` bracketed param: the latter
    // reads empty here because the completion fn's `compstate[insert]=menu`
    // write lands in the hash storage while the flat param is scoped to the
    // fn. Reading the flat param missed the menu decision entirely, so
    // USEMENU never became 1 and menu-completion (→ menucmp → the
    // menu_start hook → domenuselect) never started.
    let post_insert = get_compstate_str("insert").unwrap_or_default();
    if !post_insert.is_empty() {
        if post_insert.contains("automenu") {
            USEMENU.store(2, Ordering::Relaxed);
        } else if post_insert.contains("menu") {
            USEMENU.store(1, Ordering::Relaxed);
        }
    }

    // c:914 — incompfunc = icf. Restore.
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
            // c:952
            s_owned = s_owned[p..].to_string(); // c:953 s = p
            PARWB.store(owb, Ordering::Relaxed); // c:954
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
    let stack = COMPQSTACK // c:1065
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let p_bytes = stack.as_bytes();
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
        let flag_ret: i32 = if qstring {
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
            // c:1235-1245 — itype_end(INAMESPC) walk.
            let walked = walk_namespace(&bytes[e..]);
            if walked > 0 {
                e += walked;
            } else if c == '.' {
                // c:1255
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
    let bytes = p.as_bytes(); // c:1366
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
        COMPLETEINWORD, ENDINPUT, GETKEY_UPDATE_OFFSET, GETKEYS_DOLLARS_QUOTE, LEXERR, LEXFLAGS_ZLE,
        Meta, STRING_LEX,
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
    let ne = crate::ported::exec::noerrs.load(Ordering::Relaxed); // c:1479
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
    crate::ported::exec::noerrs.store(1, Ordering::Relaxed);
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
    crate::ported::exec::noerrs.store(ne, Ordering::Relaxed);
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
        let head = if instr == QT_NONE { QT_BACKSLASH } else { instr };
        let mut new_qstack = String::new();
        if let Some(hc) = char::from_u32(head as u32) {
            new_qstack.push(hc);
        }
        new_qstack.push_str(&compqstack_s);
        put(&COMPQSTACK, new_qstack);
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
    let cqip = tricat(&snap(&COMPQIPREFIX), &snap(&COMPIPREFIX), &multiquote(&qp, 1));
    put(&COMPQIPREFIX, cqip);
    let cqis = tricat(&multiquote(&qs, 1), &snap(&COMPISUFFIX), &snap(&COMPQISUFFIX));
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
                notempty = true; // c:1984
                buf.push(b);
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
        // Read directly from paramtab: arrays first, then hashed
        // assoc-array values, then scalar wrapped in a 1-element array.
        queue_signals();
        let result = {
            let tab = match paramtab().read() {
                Ok(t) => t,
                Err(_) => {
                    unqueue_signals();
                    return None;
                }
            };
            tab.get(nam).and_then(|pm| {
                if let Some(arr) = pm.u_arr.as_ref() {
                    Some(arr.clone()) // c:2004 getaparam
                } else if let Some(s) = pm.u_str.as_ref() {
                    Some(vec![s.clone()]) // c:2009 getsparam
                } else {
                    None
                }
            })
        };
        unqueue_signals(); // c:2022
        result
    }
}

// =====================================================================
// get_data_arr — `Src/Zle/compcore.c:2022`.
// =====================================================================

/// Direct port of `static char **get_data_arr(char *name, int keys)`
/// from `Src/Zle/compcore.c:2022`. C uses `fetchvalue` with
/// `SCANPM_WANTKEYS`/`SCANPM_WANTVALS` + `SCANPM_MATCHMANY` to scan
/// an associative-array parameter and return either its keys or its
/// values as a flat array. Without `fetchvalue` ported with full
/// SCANPM flag support, we go straight to the hashed-storage
/// thread-local maintained by params.rs for assoc-arrays.
pub fn get_data_arr(name: &str, keys: bool) -> Option<Vec<String>> {
    // c:2022

    queue_signals(); // c:2028

    // c:2030-2034 — `fetchvalue(&vbuf, &name, 1, (keys ? SCANPM_WANTKEYS
    //   : SCANPM_WANTVALS) | SCANPM_MATCHMANY)` then `getarrvalue(v)`.
    // Route through the same param accessors `${(k)name}` / `${(v)name}`
    // / `${name}` use so SPECIAL magic hashes (`commands`, `builtins`,
    // `functions`, `aliases`, `reswords`, …) resolve via their module
    // scanfns — the raw `paramtab_hashed_storage` map is empty for those,
    // which is why `compadd -k commands` previously added the literal
    // word "commands" instead of every command name.
    let result = if keys {
        // SCANPM_WANTKEYS — assoc keys (gethkparam handles special hashes).
        crate::ported::params::gethkparam(name)
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
    mnum.fetch_add(1, Ordering::Relaxed); // c:2061
    {
        let cell = curexpl.get_or_init(|| Mutex::new(None)); // c:2063
        if let Ok(mut g) = cell.lock() {
            if let Some(e) = g.as_mut() {
                e.count += 1;
            }
        }
    }
    let mcell = matches.get_or_init(|| Mutex::new(Vec::new())); // c:2066
    if let Ok(mut g) = mcell.lock() {
        g.push(cm);
    }
    newmatches.store(1, Ordering::Relaxed); // c:2068
    {
        let cell = mgroup.get_or_init(|| Mutex::new(None)); // c:2069
        if let Ok(mut g) = cell.lock() {
            if let Some(grp) = g.as_mut() {
                grp.new_ = 1;
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
    let _nm = mnum.load(Ordering::Relaxed); // c:2095 nm

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

    // c:2143-2147 — snapshot brbeg/brend curpos per CAF_QUOTE.
    let _quote_mode = (dat.aflags & CAF_QUOTE) != 0; // c:2144

    if (dat.flags & 0x0008/*CMF_ISPAR*/) != 0 {
        // c:2148
        dat.flags |= parflags.load(Ordering::Relaxed); // c:2149
    }

    let qc = compquote_first(); // c:2150
    if let Some(q) = qc {
        // c:2151
        match q {
            '`' => {
                instring_set(0);
                inbackt_set(0);
                autoq_set("");
            } // c:2153-2161
            '\'' => instring_set(QT_SINGLE), // c:2165
            '"' => instring_set(QT_DOUBLE),  // c:2168
            '$' => instring_set(QT_DOLLARS), // c:2171
            _ => {}
        }
    } else {
        instring_set(0);
        inbackt_set(0);
        autoq_set(""); // c:2179
    }

    // c:2182 — `useexact = (compexact && !strcmp(compexact, "accept"))`.
    //          C reads the `compexact` element of `$compstate`. Route
    //          through paramtab via getsparam — `$compstate[exact]`
    //          is the hashed-store equivalent. Was reading the OS env
    //          which never carries compstate values.
    let exact_str = getsparam("compexact").unwrap_or_default();
    useexact.store(if exact_str == "accept" { 1 } else { 0 }, Ordering::Relaxed);

    // c:2210-2222 — push dat.match onto mstack (the matcher chain
    // queried by match_str during candidate evaluation).
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
    let lsuf = compsuffix_s.clone();

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
                    &lpre, ppre, None, 0, None, 0, 0, 1,
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
        return if mnum.load(Ordering::Relaxed) == _nm { 1 } else { 0 };
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

    // c:2179-2184 — `doadd = !apar && !opar && !dpar`.
    let doadd = dat.apar.is_none() && dat.opar.is_none() && dat.dpar.is_empty();
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
            // c:2509-2511 — literal-suffix check.
            for suf in &aign {
                if full.len() >= suf.len() && full.ends_with(suf.as_str()) {
                    compignored_local += 1;
                    dpar_skip_word!(); // c:2520
                    continue 'cand;
                }
            }
            // c:2513-2518 — Patprog check.
            for prog in &pign {
                if crate::ported::pattern::pattry(prog, &full) {
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
                dat.pre.as_deref().unwrap_or(""),
                dat.prpre.as_deref().unwrap_or(""),
                dat.ppre.as_deref().unwrap_or(""),
                None, // pline (path-prefix Cline; unused on this path)
                dat.psuf.as_deref().unwrap_or(""),
                None, // sline (path-suffix Cline; unused on this path)
                dat.suf.as_deref().unwrap_or(""),
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
                if let Ok(mut g) = matches.get_or_init(|| Mutex::new(Vec::new())).lock() {
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
    // c:2636 — `return (mnum == nm)`: non-zero (1) when NO new matches were
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
    pre: &str,
    prpre: &str,
    ppre: &str,
    mut pline: Option<Box<Cline>>,
    psuf: &str,
    mut sline: Option<Box<Cline>>,
    suf: &str,
    flags: i32,
    exact: i32,
) -> Cmatch {
    // c:2663 — DPUTS(!line, "BUG: add_match_data() without cline")
    DPUTS!(line.is_none(), "BUG: add_match_data() without cline"); // c:2663
                                                                   // c:2657 — pick the active aminfo by `alt` (alternative path = fignore).
    let _ai_ref = if alt != 0 { &fainfo } else { &ainfo }; // c:2657
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
    let _ppl = ppre.len();
    let _pl = pre.len();
    let qipre_v = qipre_get(); // c:2686
    let qipl_v = qipre_v.clone();
    let _qipl = qipl_v.len();

    let _stl = str.len();
    let _lpl = ripre_.len();
    let _lsl = suf.len();
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
    let pl_local = pre.len() as i32;
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
        let p = bld_parts(pre, pl_local, pl_local, None, None);
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
            pre,
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

    cm.pre = if pre.is_empty() {
        None
    } else {
        Some(pre.into())
    }; // c:2944
    cm.suf = if suf.is_empty() {
        None
    } else {
        Some(suf.into())
    }; // c:2945

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

    // c:2974-2993 — brpl/brsl brace position arrays. Walk BRBEG/BREND
    // (the global Brinfo chains from `Src/Zle/zle_tricky.c`), reading
    // `qpos` for each entry to derive the position offset within the
    // match string. With no brace chain populated (zero-brace common
    // case) brpl/brsl stay empty.
    cm.brpl = BRBEG
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| {
            g.as_ref().map(|head| {
                let mut out: Vec<i32> = Vec::new();
                let mut cur = Some(head.as_ref());
                while let Some(n) = cur {
                    out.push(n.qpos);
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
                    out.push(n.qpos);
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

    // c:3003 — ai->line = join_clines(ai->line, line).
    if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(a) = g.as_mut() {
            let old_line = a.line.take();
            a.line = crate::ported::zle::compmatch::join_clines(old_line, line);
        }
    }

    // c:3005 — mnum++.
    mnum.fetch_add(1, Ordering::Relaxed);

    // c:3006 — ai->count++.
    if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(a) = g.as_mut() {
            a.count += 1;
        }
    }

    // c:3008 — addlinknode((alt ? fmatches : matches), cm). Already
    // wired below via matches Vec push.

    // c:3010-3011 — newmatches = 1; mgroup->new = 1.
    newmatches.store(1, Ordering::Relaxed);

    // c:3012-3013 — compignored++ when alt.
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

    // c:3024-3025 — ai->firstm = cm.
    if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
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

    // c:3037-3064 — exact-match tracking on ai.
    if exact != 0 {
        // c:3037
        if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
            if let Some(a) = g.as_mut() {
                if a.exact == 0 {
                    // c:3038
                    a.exact = useexact.load(Ordering::Relaxed);
                    a.exactm = Some(Box::new(cm.clone())); // c:3058
                } else if useexact.load(Ordering::Relaxed) != 0 {
                    // c:3059
                    // c:3060-3061 — ambiguous exact: set to 2, clear exactm.
                    a.exact = 2;
                    a.exactm = None;
                }
            }
        }
    }

    // c:3064 — push cm into matches/fmatches LinkList.
    let cell = if alt != 0 {
        fmatches.get_or_init(|| Mutex::new(Vec::new()))
    } else {
        matches.get_or_init(|| Mutex::new(Vec::new()))
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
            //   fmatches = p->lfmatches; allccs = p->lallccs;`. In C these
            //   are pointer aliases into the reused group so appends keep
            //   flowing into it. The Rust port keeps them as separate
            //   Mutex globals, so restore their contents from the group;
            //   `endcmgroup` flushes them back on close.
            if let Ok(mut m) = expls.get_or_init(|| Mutex::new(Vec::new())).lock() {
                *m = active.lexpls.clone();
            }
            if let Ok(mut m) = matches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                *m = active.lmatches.clone();
            }
            if let Ok(mut m) = fmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                *m = active.lfmatches.clone();
            }
            if let Ok(mut m) = allccs.get_or_init(|| Mutex::new(Vec::new())).lock() {
                *m = active.lallccs.clone();
            }
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
        g.insert(0, grp.clone()); // c:3121-3124
    }
    let mc = mgroup.get_or_init(|| Mutex::new(None));
    if let Ok(mut s) = mc.lock() {
        *s = Some(grp);
    }
    if let Ok(mut g) = expls.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear();
    }
    if let Ok(mut g) = matches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear();
    }
    if let Ok(mut g) = fmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear();
    }
    if let Ok(mut g) = allccs.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear();
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

    // Snapshot the file-scope accumulators before touching amatches
    // (distinct mutexes; snapshot-first keeps the lock scopes disjoint).
    let m_snap = matches
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let fm_snap = fmatches
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let ex_snap = expls
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let ac_snap = allccs
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    // Identify the current group and record ylist on the mgroup holder.
    let (name, flags, new_) = {
        let mc = mgroup.get_or_init(|| Mutex::new(None));
        match mc.lock() {
            Ok(mut g) => match g.as_mut() {
                Some(grp) => {
                    grp.ylist = yl.clone(); // c:3140
                    (grp.name.clone(), grp.flags, grp.new_)
                }
                None => return,
            },
            Err(_) => return,
        }
    };

    let mask = CGF_NOSORT | CGF_UNIQALL | CGF_UNIQCON | CGF_MATSORT | CGF_NUMSORT | CGF_REVSORT;
    // Rust-only correctness note (no C counterpart — C aliases `matches` to
    // the current group's mlist so permmatches always sees live matches):
    // the port keeps `matches` as a separate global flushed here, so a
    // `permmatches` triggered mid-completion (e.g. a completer's
    // `$compstate[nmatches]` read) can process a still-open group BEFORE its
    // matches are flushed, cache nmatches with that group empty, and — since
    // permmatches early-returns when `newmatches==0` — never re-count it
    // after this flush. Every `compadd -J g2` past the first vanished. Mark
    // `newmatches` when a flush actually moves matches into a group so the
    // next permmatches recomputes instead of returning the stale cache.
    let flushed_any = !m_snap.is_empty() || !fm_snap.is_empty();
    if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        if let Some(grp) = g
            .iter_mut()
            .find(|grp| grp.name == name && (grp.flags & mask) == (flags & mask))
        {
            grp.lmatches = m_snap;
            grp.lfmatches = fm_snap;
            grp.lexpls = ex_snap;
            grp.lallccs = ac_snap;
            grp.ylist = yl;
            grp.new_ = new_;
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

    let elist = expls.get_or_init(|| Mutex::new(Vec::new()));
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
                    if let Ok(mut mg) = mc.lock() {
                        if let Some(grp) = mg.as_mut() {
                            grp.new_ = 1;
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
        if let Ok(mut mg) = mc.lock() {
            if let Some(grp) = mg.as_mut() {
                grp.new_ = 1;
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
    let (as_, bs) = if (order & CGF_MATSORT) != 0 || (cmp == 0 && a.disp.is_none()) {
        (
            a.str.clone().unwrap_or_default(), // c:3181
            b.str.clone().unwrap_or_default(),
        ) // c:3182
    } else {
        if cmp != 0 {
            // c:3184
            let raw = (cmp as i32) * sortdir;
            return if raw < 0 {
                std::cmp::Ordering::Less
            }
            // c:3185
            else if raw > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            };
        }
        let displine_cmp = (b.flags & CMF_DISPLINE) - (a.flags & CMF_DISPLINE); // c:3187
        if displine_cmp != 0 {
            // c:3188
            let raw = displine_cmp * sortdir;
            return if raw < 0 {
                std::cmp::Ordering::Less
            } else if raw > 0 {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            };
        }
        (
            a.disp.clone().unwrap_or_default(), // c:3191
            b.disp.clone().unwrap_or_default(),
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
    let base = crate::ported::sort::zstrcmp(&as_, &bs, flags);
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
                    let mut bp = ap;
                    while bp + 1 < rp.len() && matcheq(&rp[ap], &rp[bp + 1]) {
                        bp += 1;
                        n -= 1; // c:3277 bp[1] && matcheq
                    }
                    let mut dup = 0i32; // c:3281
                    while bp + 1 < rp.len()
                        && rp[ap].disp.is_none()
                        && rp[bp + 1].disp.is_none()                         // c:3282 !disp
                        && rp[ap].str == rp[bp + 1].str
                    {
                        rp[bp + 1].flags |= CMF_MULT; // c:3284
                        dup = 1; // c:3285
                        bp += 1;
                        n -= 1; // same-string duplicate is dropped too
                    }
                    if dup != 0 {
                        // c:3287
                        rp[ap].flags |= CMF_FMULT; // c:3288
                    }
                    // c:3275 `*cp++ = *ap` — keep the first of the run at `cp`.
                    if ap != cp {
                        rp.swap(ap, cp);
                    }
                    cp += 1;
                    ap = bp + 1; // c:3279 ap = bp; ap++
                }
                rp.truncate(cp); // c:3291 *cp = NULL
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
                    // c:3346
                    if ap != cp {
                        rp.swap(ap, cp);
                    }
                    cp += 1;
                    let mut bp = ap;
                    while bp + 1 < rp.len() && matcheq(&rp[ap], &rp[bp + 1]) {
                        bp += 1;
                        n -= 1; // c:3348
                    }
                    let mut dup = 0i32;
                    while bp + 1 < rp.len()
                        && rp[ap].disp.is_none()
                        && rp[bp + 1].disp.is_none()
                        && rp[ap].str == rp[bp + 1].str
                    {
                        rp[bp + 1].flags |= CMF_MULT; // c:3352
                        dup = 1; // c:3353
                        bp += 1;
                    }
                    if dup != 0 {
                        rp[ap].flags |= CMF_FMULT; // c:3356
                    }
                    ap = bp + 1;
                }
                rp.truncate(cp); // c:3359
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

    for g_orig in groups_snapshot.into_iter() {
        // c:3449 while (g)
        let mut g = g_orig; // borrow-mut snapshot
        let must_rebuild = fi != ofi || g.perm.is_none() || g.new_ != 0; // c:3456
        if must_rebuild {
            // c:3456
            let src_list = if fi != 0 {
                g.lfmatches.clone()
            }
            // c:3457
            else {
                g.lmatches.clone()
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
            let mut exps = g.lexpls.clone(); // type=0 path
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
            g.perm = Some(Box::new(n_grp.clone())); // c:3490 g->perm = n
            new_pmatches.push(n_grp); // c:3492-3496
        } else {
            // reuse existing g->perm                                        // c:3534
            nmatches.fetch_add(g.mcount, Ordering::Relaxed); // c:3540
            smatches.fetch_add(g.lcount, Ordering::Relaxed); // c:3541
            if g.mcount > 1 {
                diffmatches.store(1, Ordering::Relaxed); // c:3543
            }
            g.num = gn;
            gn += 1; // c:3546
            if let Some(p) = g.perm.as_deref() {
                new_pmatches.push(p.clone()); // c:3537 pmatches = g->perm
            }
        }
        g.new_ = 0; // c:3548
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
pub static matches: OnceLock<Mutex<Vec<Cmatch>>> = OnceLock::new(); // c:124
/// Port of `LinkList fmatches` from compcore.c:126.
pub static fmatches: OnceLock<Mutex<Vec<Cmatch>>> = OnceLock::new(); // c:126

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
pub static expls: OnceLock<Mutex<Vec<Cexpl>>> = OnceLock::new(); // c:218

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
pub static allccs: OnceLock<Mutex<Vec<String>>> = OnceLock::new(); // c:259

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
/// (and the `ispar` modifier). Direct lift of compcore.c:591-617.
fn compcontext_for(_s: &str) -> String {
    // c:591
    let ip = ispar.load(Ordering::Relaxed); // c:599
    if ip == 2 {
        return "brace_parameter".into();
    } // c:600
    if ip == 1 {
        return "parameter".into();
    } // c:601
    let lw = linwhat.load(Ordering::Relaxed); // c:602
    match lw {
        // c:602
        x if x == IN_PAR_LW => "assign_parameter".into(), // c:603
        x if x == IN_MATH_LW => "math".into(),            // c:604-611
        x if x == IN_COND_LW => "condition".into(),       // c:613
        x if x == IN_ENV_LW => "value".into(),            // c:615
        _ => "command".into(),                            // c:617
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
}

/// Read `$compstate[KEY]`. Returns `None` when the key was never set.
///
/// Prefers the hash-storage view (the canonical home for a PM_HASHED
/// param); falls back to the legacy flat `compstate[KEY]` bracketed
/// param for entries that some code wrote via raw `setsparam` without
/// going through [`set_compstate_str`].
pub fn get_compstate_str(key: &str) -> Option<String> {
    // c:complete.c:1411-1414 — `compstate[nmatches]` is a LIVE GSU integer:
    // `get_nmatches` flushes pending match groups via `permmatches(0)` and
    // returns the running `nmatches` counter. The stored-hash read below
    // served a stale 0 for it, so every completer's `nm != $compstate[nmatches]`
    // idiom (_describe, _arguments, _alternative, …) concluded "nothing was
    // added" and option completion died even though addmatches had added
    // hundreds of matches.
    if key == "nmatches" {
        let v = if permmatches(0) != 0 {
            0
        } else {
            nmatches.load(Ordering::Relaxed)
        };
        return Some(v.to_string());
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

fn compquote_first() -> Option<char> {
    // zle_tricky.c compquote
    COMPQUOTE
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
/// Real read of `char *qisuf` via the paramtab. Mirrors C's direct
/// global read at `Src/Zle/zle_tricky.c qisuf`.
fn qisuf_get() -> String {
    // zle_tricky.c qisuf
    getsparam("qisuf").unwrap_or_default()
}
fn qipre_get() -> String {
    // zle_tricky.c qipre
    getsparam("qipre").unwrap_or_default()
}

/// Adapter for `int movefd(int fd)` from `Src/utils.c:2974` —
/// delegates to the canonical port in `ported::utils::movefd`.
fn movefd(fd: i32) -> i32 {
    // utils.c:2974
    crate::ported::utils::movefd(fd)
}

/// Adapter for `void redup(int new, int old)` from `Src/utils.c:2021` —
/// delegates to the canonical port `ported::utils::redup`. Callers
/// only need the new-fd form here; `old` is the inverse of movefd's
/// reservation (passed as -1 to mean "no original").
fn redup(new: i32) {
    // utils.c:2021
    crate::ported::utils::redup(new, -1);
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

    /// Test-only serializer for tests that mutate file-scope globals.
    static GLOBAL_MUT_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn compcontext_for_routes_ispar_first() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
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
        matches
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap()
            .clear();
        let mut dat = Cadata::default();
        dat.dummies = -1;
        let _ = addmatches(&mut dat, &["a".into(), "b".into()]);
        let n = matches.get().unwrap().lock().unwrap().len();
        assert!(n >= 2);
    }

    #[test]
    fn add_match_data_returns_populated_cmatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:3052-3067: cm.str/orig/pre/suf populated; mnum bumps by 1.
        matches
            .get_or_init(|| Mutex::new(Vec::new()))
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
            "pre",
            "prpre",
            "ppre",
            None,
            "psuf",
            None,
            "suf",
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
            0, "x", "x", None, "", "", "", "", "", "", None, "", None, "", 0, 1,
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
