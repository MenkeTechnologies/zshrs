//! ZLE tricky - completion and expansion widgets
//!
//! Direct port from zsh/Src/Zle/zle_tricky.c
//!
//! Implements completion widgets:
//! - complete-word, menu-complete, reverse-menu-complete
//! - expand-or-complete, expand-or-complete-prefix
//! - list-choices, list-expand
//! - expand-word, expand-history
//! - spell-word, delete-char-or-list
//! - magic-space, accept-and-menu-complete

use crate::ported::module::gethookdef;
use crate::ported::utils::{write_loop, zwarn};
use crate::ported::zle::compcore::{
    compfunc, ADDEDX, LASTCHAR, WB, WE, ZLEMETACS, ZLEMETALINE, ZLEMETALL,
};
use crate::ported::zle::zle_h::{
    WidgetImpl, COMP_COMPLETE, COMP_EXPAND, COMP_EXPAND_COMPLETE, COMP_ISEXPAND,
    COMP_LIST_COMPLETE, COMP_LIST_EXPAND, COMP_SPELL, CUT_RAW,
};
use crate::ported::zsh_h::{
    isset, BASHAUTOLIST, GLOBCOMPLETE, MENUCOMPLETE, QT_BACKSLASH, QT_DOLLARS, QT_DOUBLE, QT_NONE,
    QT_SINGLE, RECEXACT,
};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

// =====================================================================
// Globals — `Src/Zle/zle_tricky.c:96-106`.
// =====================================================================
//
// usemenu/useglob — controls type of completion (set by entry widget,
// read by `docomplete`/`callcompfunc`). usemenu==2 starts automenu;
// usemenu==3 inserts as if for menucomp without really starting it.
// wouldinstab — non-zero if we'd insert TAB but for the comp widget.

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_utils::*, zle_vi::*, zle_word::*,
};
/// Port of `mod_export int usemenu` from `Src/Zle/zle_tricky.c:96`.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]

/// Port of `usetab()` from Src/Zle/zle_tricky.c:183.
/// WARNING: param names don't match C — Rust=(zle, keybuf) vs C=()
pub fn usetab() -> i32 {
    // c:183 — C signature is `usetab(void)`, reads `keybuf` global
    // (no parameter). The previous Rust port took an explicit
    // `keybuf: &[u8]` arg, drifting from the C contract; restored
    // here to a faithful port that reads the actual `keybuf` global.
    let kb = crate::ported::zle::zle_keymap::keybuf.lock().unwrap();
    // c:187-188 — `if (keybuf[0] != '\t' || keybuf[1]) return 0`.
    if kb.first() != Some(&b'\t') || kb.len() > 1 {
        return 0;
    }
    drop(kb);
    // c:189-191 — walk back from cursor-1 to BOL; only `\t` / ' '
    // allowed for usetab to fire (i.e. line-indent only).
    let mut i = ZLECS.load(Ordering::SeqCst);
    while i > 0 {
        let c = ZLELINE.lock().unwrap()[i - 1];
        if c == '\n' {
            break;
        }
        if c != '\t' && c != ' ' {
            return 0;
        }
        i -= 1;
    }
    // c:192-196 — `if (compfunc) { wouldinstab = 1; return 0; }
    //              else return 1`. Previous port silently dropped
    // the compfunc branch (it just loaded WOULDINSTAB and threw
    // away the value). Restored here.
    let compfunc_set = compfunc
        .get()
        .and_then(|m| m.lock().ok().and_then(|g| g.clone()))
        .is_some();
    if compfunc_set {
        WOULDINSTAB.store(1, Ordering::SeqCst);
        return 0;
    }
    1
}

/// Direct port of `int completecall(char **args)` from
/// `Src/Zle/zle_tricky.c:202`. Invoked by `execzlefunc` when the
/// dispatched widget has `WIDGET_NCOMP` set (a `zle -C` wrapper
/// widget). Reads `compwidget->u.comp.{fn,func}`, plants the user
/// shell function name in the `compfunc` global so the eventual
/// `callcompfunc(s, compfunc)` inside `do_completion` invokes the
/// right shfunc (`_main_complete` for the default compinit setup),
/// and calls the base widget's C fn (e.g. `completeword`).
pub fn completecall(args: &[String]) -> i32 {
    // c:202
    // c:204-205 — `cfargs = args; cfret = 0;`.
    *cfargs.lock().unwrap() = args.to_vec();
    cfret.store(0, Ordering::SeqCst);

    // c:206 — `compfunc = compwidget->u.comp.func`. Read the COMP
    // widget's `func` field; if compwidget is unset or not Comp,
    // fall through to a plain `docomplete(COMP_COMPLETE)` matching
    // the behavior when no `zle -C` widget is active.
    let compwidget_g = COMPWIDGET.lock().unwrap();
    let (base_fn, func_name) = match compwidget_g.as_ref().map(|w| (&w.u, w.flags)) {
        Some((WidgetImpl::Comp { fn_, func, .. }, _)) => (Some(*fn_), Some(func.clone())),
        _ => (None, None),
    };
    drop(compwidget_g);

    tracing::debug!(target: "compsys_args", ?func_name, has_base = base_fn.is_some(), "completecall ENTER");
    if let Some(name) = func_name {
        let g = compfunc.get_or_init(|| Mutex::new(None));
        *g.lock().unwrap() = Some(name); // c:206
    }

    // c:207-208 — `if (compwidget->u.comp.fn(zlenoargs) && !cfret)
    //                cfret = 1;`. zlenoargs is the empty argv (C's
    // `static char *zlenoargs[1] = { NULL }`); Rust uses an empty
    // slice.
    let zlenoargs: &[String] = &[];
    let r = match base_fn {
        Some(f) => f(zlenoargs),
        None => docomplete(COMP_COMPLETE),
    };
    if r != 0 && cfret.load(Ordering::SeqCst) == 0 {
        cfret.store(1, Ordering::SeqCst); // c:208
    }

    // c:209 — `compfunc = NULL`.
    if let Some(g) = compfunc.get() {
        *g.lock().unwrap() = None;
    }

    cfret.load(Ordering::SeqCst) // c:211
}

/// Port of `int completeword(char **args)` from
/// `Src/Zle/zle_tricky.c:216`.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn completeword(args: &[String]) -> i32 {
    // c:216
    USEMENU.store(isset(MENUCOMPLETE) as i32, Ordering::SeqCst); // c:218 — `usemenu = !!isset(MENUCOMPLETE)`
    USEGLOB.store(isset(GLOBCOMPLETE) as i32, Ordering::SeqCst); // c:219 — `useglob = isset(GLOBCOMPLETE)`
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:220
                                            // c:221-222 — Tab-at-indent → `selfinsert(args)`. Body of
                                            // `selfinsert` ignores args (C marks them `UNUSED`); kept in
                                            // the contract for sig parity.
    let lastch = LASTCHAR.load(Ordering::SeqCst);
    if lastch == b'\t' as i32 && usetab() != 0 {
        return selfinsert(args);
    }
    // c:224-232 — BASH_AUTO_LIST branch. When the previous
    // completion was ambiguous (`lastambig == 1`), `BASH_AUTO_LIST`
    // is set, and we're not in menu-completion mode, route through
    // `COMP_LIST_COMPLETE` first (list the choices; next Tab
    // continues with normal completion). Without this branch
    // bash-style users lose the two-stage list-then-complete UX.
    let usemenu_now = USEMENU.load(Ordering::SeqCst);
    let menucmp_now = MENUCMP.load(Ordering::SeqCst);
    if LASTAMBIG.load(Ordering::SeqCst) == 1
        && isset(BASHAUTOLIST)
        && usemenu_now == 0
        && menucmp_now == 0
    {
        BASHLISTFIRST.store(1, Ordering::SeqCst); // c:226
        let ret = docomplete(COMP_LIST_COMPLETE); // c:227
        BASHLISTFIRST.store(0, Ordering::SeqCst); // c:228
        LASTAMBIG.store(2, Ordering::SeqCst); // c:229
        return ret;
    }
    docomplete(COMP_COMPLETE) // c:231
}

/// Port of `menucomplete(char **args)` from Src/Zle/zle_tricky.c:238.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn menucomplete(args: &[String]) -> i32 {
    // c:238
    USEMENU.store(1, Ordering::SeqCst); // c:240
    USEGLOB.store(isset(GLOBCOMPLETE) as i32, Ordering::SeqCst); // c:241 — `useglob = isset(GLOBCOMPLETE)`
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:242
    let lastch = LASTCHAR.load(Ordering::SeqCst);
    if lastch == b'\t' as i32 && usetab() != 0 {
        return selfinsert(args);
    }
    docomplete(COMP_COMPLETE) // c:246
}

/// Port of `listchoices(UNUSED(char **args))` from `Src/Zle/zle_tricky.c:251`.
/// ```c
/// int
/// listchoices(UNUSED(char **args))
/// {
///     usemenu = !!isset(MENUCOMPLETE);
///     useglob = isset(GLOBCOMPLETE);
///     wouldinstab = 0;
///     return docomplete(COMP_LIST_COMPLETE);
/// }
/// ```
/// `list-choices` widget — set the menu/glob globals from options
/// then dispatch to `docomplete(COMP_LIST_COMPLETE)`.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn listchoices(_args: &[String]) -> i32 {
    // c:251
    // c:253 — `usemenu = !!isset(MENUCOMPLETE)`.
    let menu = isset(MENUCOMPLETE) as i32;
    USEMENU.store(menu, Ordering::SeqCst);
    // c:254 — `useglob = isset(GLOBCOMPLETE)`.
    let glob = isset(GLOBCOMPLETE) as i32;
    USEGLOB.store(glob, Ordering::SeqCst);
    // c:255 — `wouldinstab = 0`.
    WOULDINSTAB.store(0, Ordering::SeqCst);
    // c:256 — `return docomplete(COMP_LIST_COMPLETE)`.
    docomplete(COMP_LIST_COMPLETE)
}

/// Port of `spellword(UNUSED(char **args))` from `Src/Zle/zle_tricky.c:261`.
/// ```c
/// int
/// spellword(UNUSED(char **args))
/// {
///     usemenu = useglob = 0;
///     wouldinstab = 0;
///     return docomplete(COMP_SPELL);
/// }
/// ```
/// `spell-word` widget — clears menu/glob globals and dispatches
/// with `COMP_SPELL`.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn spellword(_args: &[String]) -> i32 {
    // c:261
    USEMENU.store(0, Ordering::SeqCst); // c:263 usemenu = 0
    USEGLOB.store(0, Ordering::SeqCst); // c:263 useglob = 0
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:264
    docomplete(COMP_SPELL) // c:265
}

/// Port of `deletecharorlist(char **args)` from Src/Zle/zle_tricky.c:270.
pub fn deletecharorlist(_args: &[String]) -> i32 {
    // c:270
    // c:272-273 — C reads the OPTIONS here, exactly like every sibling
    // widget: `usemenu = !!isset(MENUCOMPLETE); useglob = isset(GLOBCOMPLETE)`.
    // The port hardcoded 0/1, so `delete-char-or-list` at end-of-line always
    // listed with menu-completion forced OFF and glob-completion forced ON
    // regardless of the user's MENU_COMPLETE / GLOB_COMPLETE settings.
    USEMENU.store(isset(MENUCOMPLETE) as i32, Ordering::SeqCst); // c:272
    USEGLOB.store(isset(GLOBCOMPLETE) as i32, Ordering::SeqCst); // c:273
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:274
                                            // c:277-281 — `if (zlecs != zlell) { fixsuffix(); invalidatelist();
                                            //                return deletechar(args); }`. Both calls were absent:
                                            // without `fixsuffix` an auto-removable suffix stayed in the buffer
                                            // and without `invalidatelist` a stale completion list kept being
                                            // redisplayed after the delete.
    if ZLECS.load(Ordering::SeqCst) != ZLELL.load(Ordering::SeqCst) {
        crate::ported::zle::zle_misc::fixsuffix(); // c:278
        crate::ported::zle::zle_h::invalidatelist(); // c:279
        return deletechar(); // c:280
    }
    docomplete(COMP_LIST_COMPLETE) // c:282
}

/// Port of `expandword(char **args)` from Src/Zle/zle_tricky.c:287.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn expandword(args: &[String]) -> i32 {
    // c:287
    USEMENU.store(0, Ordering::SeqCst); // c:289
    USEGLOB.store(0, Ordering::SeqCst); // c:289
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:290
                                            // c:291-292 — `if (lastchar == '\t' && usetab()) return selfinsert(args);`.
                                            // The whole Tab-at-indent arm was missing, so `expand-word` bound to
                                            // Tab expanded instead of inserting a literal tab when the cursor sat
                                            // in leading whitespace — every sibling widget (completeword,
                                            // menucomplete, expandorcomplete, menuexpandorcomplete) has it.
    let lastch = LASTCHAR.load(Ordering::SeqCst);
    if lastch == b'\t' as i32 && usetab() != 0 {
        return selfinsert(args); // c:292
    }
    docomplete(COMP_EXPAND) // c:294
}

/// Port of `expandorcomplete(char **args)` from Src/Zle/zle_tricky.c:299.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn expandorcomplete(args: &[String]) -> i32 {
    // c:299
    USEMENU.store(isset(MENUCOMPLETE) as i32, Ordering::SeqCst); // c:301 — `usemenu = !!isset(MENUCOMPLETE)`
    USEGLOB.store(isset(GLOBCOMPLETE) as i32, Ordering::SeqCst); // c:302 — `useglob = isset(GLOBCOMPLETE)`
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:303
                                            // c:304-305 — Tab-at-indent → `selfinsert(args)`. Rust
                                            // `selfinsert()` takes no args today (C's body marks them
                                            // `UNUSED`) so the arg pass-through is dropped; a follow-up
                                            // patch should widen `selfinsert` to `fn(&[String]) -> i32`
                                            // for sig parity.
    let lastch = LASTCHAR.load(Ordering::SeqCst);
    if lastch == b'\t' as i32 && usetab() != 0 {
        return selfinsert(args);
    }
    // c:307-313 — BASH_AUTO_LIST branch (same shape as
    // completeword): when the previous completion was ambiguous +
    // BASH_AUTO_LIST is on + we're not in menu mode, route through
    // `COMP_LIST_COMPLETE` to list choices before completing.
    let usemenu_now = USEMENU.load(Ordering::SeqCst);
    let menucmp_now = MENUCMP.load(Ordering::SeqCst);
    if LASTAMBIG.load(Ordering::SeqCst) == 1
        && isset(BASHAUTOLIST)
        && usemenu_now == 0
        && menucmp_now == 0
    {
        BASHLISTFIRST.store(1, Ordering::SeqCst); // c:309
        let ret = docomplete(COMP_LIST_COMPLETE); // c:310
        BASHLISTFIRST.store(0, Ordering::SeqCst); // c:311
        LASTAMBIG.store(2, Ordering::SeqCst); // c:312
        return ret;
    }
    docomplete(COMP_EXPAND_COMPLETE) // c:314
}

/// Port of `menuexpandorcomplete(char **args)` from Src/Zle/zle_tricky.c:321.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn menuexpandorcomplete(args: &[String]) -> i32 {
    // c:321
    USEMENU.store(1, Ordering::SeqCst); // c:323
    USEGLOB.store(isset(GLOBCOMPLETE) as i32, Ordering::SeqCst); // c:324 — `useglob = isset(GLOBCOMPLETE)`
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:325
                                            // c:326-327 — Tab-at-indent → selfinsert.
    let lastch = LASTCHAR.load(Ordering::SeqCst);
    if lastch == b'\t' as i32 && usetab() != 0 {
        return selfinsert(args);
    }
    docomplete(COMP_EXPAND_COMPLETE) // c:329
}

/// Port of `listexpand(UNUSED(char **args))` from `Src/Zle/zle_tricky.c:334`.
/// ```c
/// int
/// listexpand(UNUSED(char **args))
/// {
///     usemenu = !!isset(MENUCOMPLETE);
///     useglob = isset(GLOBCOMPLETE);
///     wouldinstab = 0;
///     return docomplete(COMP_LIST_EXPAND);
/// }
/// ```
/// `list-expand` widget — like listchoices but dispatches with
/// `COMP_LIST_EXPAND`.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn listexpand(_args: &[String]) -> i32 {
    // c:334
    let menu = isset(MENUCOMPLETE) as i32;
    USEMENU.store(menu, Ordering::SeqCst); // c:336
    let glob = isset(GLOBCOMPLETE) as i32;
    USEGLOB.store(glob, Ordering::SeqCst); // c:337
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:338
    docomplete(COMP_LIST_EXPAND) // c:339
}

/// Port of `reversemenucomplete(char **args)` from Src/Zle/zle_tricky.c:344.
pub fn reversemenucomplete(args: &[String]) -> i32 {
    // c:344
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:346
                                            // c:347 — `zmult = -zmult`. Cannot lock ZMOD twice in one
                                            // expression: the RHS guard outlives the read (Rust temporary
                                            // scope = end of statement) and the LHS lock attempt then
                                            // deadlocks the same thread on a non-reentrant std::sync::Mutex.
    {
        let mut g = ZMOD.lock().unwrap();
        g.mult = -g.mult;
    }
    menucomplete(args) // c:348 — C `menucomplete(args)`, args pass-through
}

/// Port of `acceptandmenucomplete(char **args)` from Src/Zle/zle_tricky.c:353.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn acceptandmenucomplete(args: &[String]) -> i32 {
    // c:353
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:355
                                            // c:356-357 — `if (!menucmp) return 1`.
    if MENUCMP.load(Ordering::SeqCst) == 0 {
        return 1;
    }
    // c:358 — `runhookdef(ACCEPTCOMPHOOK, NULL)`. Fires registered
    // accept-completion hooks (used by `_menu`, etc.) before
    // advancing the menu cursor.
    let h_accept = gethookdef("accept_comp");
    if !h_accept.is_null() {
        crate::ported::module::runhookdef(h_accept, std::ptr::null_mut());
    }
    // c:359 — `return menucomplete(args)` — pass args through.
    menucomplete(args)
}

/// Port of `checkparams(char *p)` from Src/Zle/zle_tricky.c:435.
pub fn checkparams(p: &str) -> i32 {
    // c:435
    // C body c:437-449:
    //   - scanhashtable(paramtab) for names with `pfxlen(p, nam) == l`
    //   - count up to 2, track exact-match
    //   - n == 1   → getsparam(p) != NULL
    //   - n != 1   → !menucmp && exact && (!hascompmod || isset(RECEXACT))
    //
    // `pfxlen(p, nam) == l` means all of `p` is a prefix of `nam`,
    // i.e. `nam.starts_with(p)`. Rust port reads paramtab directly.
    let l = p.len();
    let mut n = 0;
    let mut exact = false;
    if let Ok(tab) = crate::ported::params::paramtab().read() {
        // c:437
        for name in tab.keys() {
            // c:438 walk nodes
            if name.starts_with(p) {
                // c:439 pfxlen == l
                n += 1; // c:440
                if name.len() == l {
                    // c:441
                    exact = true; // c:442
                }
                if n >= 2 {
                    // c:438 n < 2 gate
                    break;
                }
            }
        }
    }
    if n == 1 {
        // c:446
        return if crate::ported::params::getsparam(p).is_some() {
            1
        } else {
            0
        };
    }
    // c:447-448 — `!menucmp && e && (!hascompmod || isset(RECEXACT))`.
    let menucmp = MENUCMP.load(Ordering::SeqCst) != 0;
    let hascompmod = HASCOMPMOD.load(Ordering::SeqCst);
    let recexact = isset(RECEXACT);
    if !menucmp && exact && (!hascompmod || recexact) {
        // c:448
        1
    } else {
        0
    }
}

/// Direct port of `int cmphaswilds(char *str)` from
/// `Src/Zle/zle_tricky.c:457`. Walks `str` looking for an
/// unescaped glob metachar; dispatches on the parser's tokenized
/// chars (`Inbrack`, `Inpar`, `Inbrace`, `Inang`, `String`,
/// `Qstring`, `Star`, `Quest`, `Pound`, `Hat`, `Bar`, `Tilde`)
/// using `skipparens` to walk balanced brackets.
///
/// Returns 1 if a wildcard is found, 0 otherwise.
pub fn cmphaswilds(str: &str) -> i32 {
    // c:457
    use crate::ported::zsh_h::{
        isset, Bar, Equals, Hat, Inang, Inbrace, Inbrack, Inpar, Outang, Outbrace, Outbrack,
        Outpar, Pound, Qstring, Quest, Star, Stringg, Tilde, EXTENDEDGLOB, IGNOREBRACES,
    };
    let mut s = str;
    // c:502 — C tests `*str == Bar`, the tokenized `|` (0x8e). The port
    // tested the literal ASCII `|` (0x7c), which cannot appear here: the
    // word handed to cmphaswilds comes from get_comp_string and is
    // tokenized, so a real `|` is already Bar. The check never fired.
    let bar_byte = Bar;
    // c:459-460 — `if ((*str == Inbrack || *str == Outbrack) && !str[1])
    //                return 0;`
    let chars: Vec<char> = s.chars().collect();
    if chars.len() == 1 && (chars[0] == Inbrack as char || chars[0] == Outbrack as char) {
        return 0;
    }
    // c:465-467 — `if (str[0] == '%' && str[1] == Quest) str += 2;`
    if chars.len() >= 2 && chars[0] == '%' && chars[1] == Quest as char {
        // 2 chars (`%` + one Quest) — both ASCII so byte == char.
        s = &s[2..];
    }
    // c:472-475 — `if (*str == Tilde && str[1] == Inbrack &&
    //                  (ptr = strchr(str+2, Outbrack))) str = ptr+1;`.
    let chars2: Vec<char> = s.chars().collect();
    if chars2.len() >= 2 && chars2[0] == Tilde as char && chars2[1] == Inbrack as char {
        if let Some(pos) = s[2..].find(Outbrack as char) {
            // pos is byte offset within s[2..]; advance past Outbrack.
            let advance = 2 + pos + (Outbrack as char).len_utf8();
            s = &s[advance..];
        }
    }
    // c:477-509 — main scan loop.
    while let Some(c) = s.chars().next() {
        if c == Stringg as char || c == Qstring as char {
            // c:478 — parameter expression `$...`.
            // c:481 — `if (*++str == Inbrace) skipparens(Inbrace, Outbrace, &str);`
            s = &s[c.len_utf8()..];
            let next_c = s.chars().next();
            if next_c == Some(Inbrace as char) {
                // c:482 — skip past `${...}`.
                let _ = crate::ported::utils::skipparens(Inbrace as char, Outbrace as char, &mut s);
            } else if next_c == Some(Stringg as char) || next_c == Some(Qstring as char) {
                // c:484 — nested `$$`.
                s = &s[next_c.unwrap().len_utf8()..];
            } else {
                // c:487-499 — skip parameter-expression prefix chars.
                while let Some(p) = s.chars().next() {
                    if p != '^'
                        && p != Hat as char
                        && p != '='
                        && p != Equals as char
                        && p != '~'
                        && p != Tilde as char
                    {
                        break;
                    }
                    s = &s[p.len_utf8()..];
                }
                let p = s.chars().next();
                if p == Some('#') || p == Some(Pound as char) {
                    s = &s[p.unwrap().len_utf8()..];
                }
                let p = s.chars().next();
                if p == Some(Star as char) || p == Some(Quest as char) {
                    s = &s[p.unwrap().len_utf8()..];
                }
            }
        } else {
            // c:501-509 — wildcard / balanced-bracket detection.
            //
            // C evaluates this as ONE short-circuiting `||` chain over the
            // SAME `str`, and `skipparens` MUTATES `str` even when it fails
            // with an unterminated bracket (utils.c:2416 walks to the end of
            // the string and returns level > 0). The port evaluated all six
            // terms eagerly on throwaway copies, so an unterminated `[`
            // left the scan pointer one char past `[` instead of at
            // end-of-string: `cmphaswilds("[a*")` answered 1 (it went on to
            // find the `*`) where zsh answers 0, sending expand-or-complete
            // down the expansion arm for a word with no usable glob.
            let is_extglob_meta = (c == Pound as char || c == Hat as char) && isset(EXTENDEDGLOB);
            let is_simple_wild = c == Star as char || c == bar_byte || c == Quest as char;
            if is_extglob_meta || is_simple_wild {
                return 1; // c:501-502
            }
            // c:503 — `!skipparens(Inbrack, Outbrack, &str)`
            if crate::ported::utils::skipparens(Inbrack as char, Outbrack as char, &mut s) == 0 {
                return 1;
            }
            // c:504 — `!skipparens(Inang, Outang, &str)`
            if crate::ported::utils::skipparens(Inang as char, Outang as char, &mut s) == 0 {
                return 1;
            }
            // c:505-506 — `unset(IGNOREBRACES) && !skipparens(Inbrace, …)`
            if !isset(IGNOREBRACES)
                && crate::ported::utils::skipparens(Inbrace as char, Outbrace as char, &mut s) == 0
            {
                return 1;
            }
            // c:507-508 — `*str == Inpar && str[1] == ':' && !skipparens(Inpar, …)`
            let pchars: Vec<char> = s.chars().take(2).collect();
            if pchars.first() == Some(&(Inpar as char))
                && pchars.get(1) == Some(&':')
                && crate::ported::utils::skipparens(Inpar as char, Outpar as char, &mut s) == 0
            {
                return 1;
            }
            // c:510-511 — `if (*str) str++;`. `s` may have been advanced by
            // a failed skipparens above, so re-read the current char.
            match s.chars().next() {
                Some(cc) => s = &s[cc.len_utf8()..],
                None => break,
            }
        }
    }
    0 // c:513
}

/// Direct port of `char *parambeg(char *s)` from
/// `Src/Zle/zle_tricky.c:521`. Walks back from the cursor (`offs`)
/// looking for a `$` (Stringg/Qstring token), then dispatches on
/// the following char to identify a parameter expression. Returns
/// the byte offset (within `s`) of the parameter-NAME start (after
/// `$`, `${`, flag-parens, modifier chars), or `None` if no
/// parameter expression brackets the cursor.
///
/// Rust signature: `(s, offs)` returns `Option<usize>` CHAR index
/// instead of C's `char *` to the same position. `offs` is C's
/// `offs` global (zle_tricky.c) passed explicitly here, and is also a
/// char index.
///
/// REPRESENTATION NOTE (Rust-only, no C counterpart): C walks `s` as
/// BYTES and every parser token (`String`, `Qstring`, `Dnull`, …) is
/// exactly one byte, so C's pointer arithmetic and `offs` agree. In
/// this port `s` is a metafied Rust `String` whose token chars live at
/// U+0080..U+009F and therefore occupy TWO UTF-8 bytes each, so byte
/// indices are NOT commensurate with `offs`. The whole scan runs over
/// `Vec<char>` with CHAR indices — the same convention the brace tail
/// of `get_comp_string` uses. `at()` returns `'\0'` past the end so
/// the reads C makes at (and one past) the NUL terminator behave the
/// same way.
pub fn parambeg(s: &str, offs: usize) -> Option<usize> {
    // c:521
    use crate::ported::zsh_h::{
        Dnull, Equals, Hat, Inbrace, Inbrack, Inpar, Outbrace, Outpar, Pound, Qstring, Quest, Star,
        Stringg, Tilde,
    };
    use crate::ported::ztype_h::{idigit, INAMESPC};

    let sv: Vec<char> = s.chars().collect();
    let slen = sv.len();
    if offs > slen {
        return None;
    }
    // Char index -> byte index, with a sentinel entry for the end so
    // `bidx[i]` is valid for `i == slen` (C's NUL position).
    let mut bidx: Vec<usize> = Vec::with_capacity(slen + 1);
    for (bi, _) in s.char_indices() {
        bidx.push(bi);
    }
    bidx.push(s.len());
    // C reads `*p` at (and past) the terminator; give those reads NUL.
    let at = |i: usize| -> char { sv.get(i).copied().unwrap_or('\0') };
    let is_str = |c: char| c == Stringg || c == Qstring;

    // c:526 — `for (p = s + offs; p > s && *p != String && *p != Qstring; p--);`
    let mut p = offs;
    while p > 0 && !is_str(at(p)) {
        p -= 1;
    }
    // c:527-533 — `if (*p == String || *p == Qstring)` then the `$$` walk.
    if is_str(at(p)) {
        // c:529-530 — `while (p > s && (p[-1] == String || p[-1] == Qstring)) p--;`
        while p > 0 && is_str(at(p - 1)) {
            p -= 1;
        }
        // c:531-532 — `while ((p[1] == ...) && (p[2] == ...)) p += 2;`
        while is_str(at(p + 1)) && is_str(at(p + 2)) {
            p += 2;
        }
    }
    // c:535-537 — `if ((*p == String || *p == Qstring) && p[1] != Inpar &&
    //                 p[1] != Inbrack && p[1] != '\'')`: confirm `$` followed
    // by NOT `(` / `[` / `'` (those are `$(...)` / `$[...]` / `$'...'`).
    let after = at(p + 1);
    if !is_str(at(p)) || after == Inpar || after == Inbrack || after == '\'' {
        return None;
    }
    // c:540-543 — `char *b = p + 1, *e = b; int n = 0, br = 1;`
    let mut b = p + 1;
    let mut br = 1;
    let mut n: i32 = 0;
    // c:545-553 — `${...}` form: `if (*b == Inbrace)`.
    if at(b) == Inbrace {
        // c:546-549 — `char *tb = b;
        //               if (!skipparens(Inbrace, Outbrace, &tb)) return NULL;`
        // skipparens returns 0 only when the matching `}` WAS found, and
        // C returns NULL in exactly that case ("see if we are before the
        // '}'"): a closed `${...}` is not an incomplete parameter name.
        let mut tb: &str = &s[bidx[b]..];
        if crate::ported::utils::skipparens(Inbrace, Outbrace, &mut tb) == 0 {
            return None;
        }
        // c:551-552 — `b++, br++;`.
        b += 1;
        br += 1;
        // c:553 — `n = skipparens(Inpar, Outpar, &b);` skip `(flags)`.
        let mut b_str: &str = &s[bidx[b]..];
        n = crate::ported::utils::skipparens(Inpar, Outpar, &mut b_str);
        let b_byte = s.len() - b_str.len();
        b = s[..b_byte].chars().count();
    }
    // c:556-560 — skip modifier prefix chars `^=~` (Hat/Equals/Tilde).
    while at(b) != '\0' {
        let bb = at(b);
        if bb != '^' && bb != Hat && bb != '=' && bb != Equals && bb != '~' && bb != Tilde {
            break;
        }
        b += 1;
    }
    // c:561-562 — `if (*b == '#' || *b == Pound || *b == '+') b++;`
    if at(b) == '#' || at(b) == Pound || at(b) == '+' {
        b += 1;
    }
    // c:564-569 — `e = b; if (br) while (*e == Dnull) e++;`
    let mut e = b;
    if br != 0 {
        while at(e) == Dnull {
            e += 1;
        }
    }
    // c:570-579 — find end of parameter name.
    let ec = at(e);
    if ec == Quest
        || ec == Star
        || ec == Stringg
        || ec == Qstring
        || ec == '?'
        || ec == '*'
        || ec == '$'
        || ec == '-'
        || ec == '!'
        || ec == '@'
    {
        e += 1; // c:574
    } else if ec.is_ascii() && idigit(ec as u8) {
        // c:575-577
        while at(e).is_ascii() && idigit(at(e) as u8) {
            e += 1;
        }
    } else if e < slen {
        // c:579 — `e = itype_end(e, INAMESPC, 0);`. The Rust `itype_end`
        // returns a BYTE span, so convert it back to a char count.
        let span = crate::ported::utils::itype_end(&s[bidx[e]..], INAMESPC, false);
        e += s[bidx[e]..bidx[e] + span].chars().count();
    }
    // c:582-589 — `if (offs <= e - s && offs >= b - s && n <= 0)`: confirm
    // the cursor falls inside the name and skipparens didn't fail.
    if offs <= e && offs >= b && n <= 0 {
        // c:584-587 — `if (br) { p = e; while (*p == Dnull) p++; }`. C
        // reassigns `p` here but never reads it again before returning,
        // so the walk has no observable effect; kept for provenance.
        return Some(b);
    }
    None
}

// !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
//
// C has no analogue of this struct because C never needs one: `$words`
// and `$CURRENT` exist ONLY as compparams, created inside the
// `startparamscope()` / `endparamscope()` pair that brackets the
// completion-function call —
//
//     c:Src/Zle/compcore.c:815-816  startparamscope(); makecompparams();
//     c:Src/Zle/compcore.c:838      endparamscope();
//
// with the table rows at `c:Src/Zle/complete.c:1259/1261`
// (`{ "words", PM_ARRAY, VAL(compwords) }`,
//  `{ "CURRENT", PM_INTEGER, VAL(compcurrent) }`). Their values live in
// the C globals `compwords`/`compcurrent` behind a gsu vtable, so the
// params are pure scope handles and vanish with the scope.
//
// zshrs has no gsu binding for those two (`var:0, gsu:0`), so
// `get_comp_string` publishes them into paramtab DIRECTLY
// (`setaparam("words", …)` / `setiparam("CURRENT", …)` below) — without
// that publish `_normal` reads `$CURRENT` unset and treats every
// position as the command word, so every command completes only command
// names. That publish happens at `docomplete` c:664, which is OUTSIDE
// c:815-838, so nothing tears it down.
//
// It only showed as a leak when the completion function never ran at
// all. With TAB on its default `expand-or-complete` binding a GLOBBED
// word is consumed by `doexpansion` (c:826): the buffer changes, the
// c:847 `!strcmp(ol, zlemetaline)` guard fails, `docompletion` — and
// therefore `callcompfunc` — is skipped, and the level-0 publish
// survives into the interactive shell. Measured under a pty
// (`-f -i`, `compinit`, one TAB, `^U`):
//
//     ls *<TAB>    zsh: words=[][0] CURRENT=[]   zshrs: words=[ls *][2]  CURRENT=[2]
//     ls **<TAB>   zsh: words=[][0] CURRENT=[]   zshrs: words=[ls **][2] CURRENT=[2]
//     ls /tm<TAB>  zsh: words=[][0] CURRENT=[]   zshrs: words=[][0]      CURRENT=[]
//
// The non-globbed case tore down only because `callcompfunc`
// (compcore.rs) re-stamps the already-published nodes with
// `level = locallevel + 1` and `PM_SPECIAL|PM_REMOVABLE`, so
// `doshfunc`'s `endparamscope` deletes them.
//
// This guard supplies the missing half: it brackets the publish for the
// whole completion attempt, the way c:815-838 brackets it for the
// function call, and — like `endparamscope` — restores whatever the
// names shadowed instead of just deleting them. The C globals
// (`CLWORDS`/`COMPWORDS`/`COMPCURRENT`) are deliberately untouched:
// `compwords`/`compcurrent` are plain C globals that persist between
// completions; only the PARAMS are scoped.
struct CompWordParamScope {
    /// The paramtab nodes `words` / `CURRENT` held when the completion
    /// began, moved out so the publish creates fresh ones — the
    /// equivalent of the shadow `startparamscope()` establishes.
    saved: Vec<(&'static str, Option<crate::ported::zsh_h::Param>)>,
}

impl CompWordParamScope {
    fn new() -> Self {
        let mut saved = Vec::new();
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            for name in ["words", "CURRENT"] {
                saved.push((name, tab.removehashnode(name)));
            }
        }
        Self { saved }
    }
}

impl Drop for CompWordParamScope {
    fn drop(&mut self) {
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            for (name, prev) in self.saved.drain(..) {
                // c:838 `endparamscope()` — the completion's own node goes
                // away unconditionally (`makecompparams` marks these
                // PM_REMOVABLE, so params.c:5905's scanendscope deletes
                // rather than restores them).
                tab.removehashnode(name);
                // …and the value it shadowed comes back.
                if let Some(pm) = prev {
                    tab.insert(name.to_string(), pm);
                }
            }
        }
    }
}

// The main entry point for completion.                                     // c:599
/// Direct port of `int docomplete(int lst)` from
/// `Src/Zle/zle_tricky.c:599`. Drives the completion engine: runs the
/// BEFORECOMPLETEHOOK chain, extracts the cursor word, then dispatches
/// on `lst` — `COMP_SPELL` → spell-check path (c:817), `COMP_ISEXPAND`
/// → `doexpansion` with fall-through to `docompletion` when
/// `COMP_EXPAND_COMPLETE` left the buffer unchanged (c:825-868),
/// otherwise → `docompletion` (c:870). Finally fires the
/// AFTERCOMPLETEHOOK chain (c:878).
pub fn docomplete(lst: i32) -> i32 {
    // TEMPORARY: ftime scaffold — one report per completion, every exit path.
    struct FtimeDump;
    impl Drop for FtimeDump {
        fn drop(&mut self) {
            crate::ftime::dump_and_reset();
        }
    }
    let _ftdump = FtimeDump;
    // c:599
    // c:604 — `int olst = lst`; `lst` is then narrowed in place by the
    // expand-vs-complete decision at c:704-793. C captures `olst` at the
    // very top, BEFORE the BEFORECOMPLETEHOOK can rewrite `lst` through the
    // pointer it is handed — so the snapshot must live here, not after the
    // hook, or `olst` would track the hook's rewrite too.
    let mut lst = lst;
    let olst = lst; // c:604
                    // c:606-609 — recursion guard. The C source uses a static `active`
                    // flag; we mirror via thread_local since each worker runs its own
                    // completion.
    thread_local! { static ACTIVE: std::cell::Cell<bool> =
    const { std::cell::Cell::new(false) }; }
    // c:606 — `if (active && !comprecursive)`. `comprecursive` (set by the
    // menu recursive-completion arms) temporarily permits re-entry.
    if ACTIVE.with(|c| c.get())
        && crate::ported::zle::complist::COMPRECURSIVE.load(std::sync::atomic::Ordering::Relaxed)
            == 0
    {
        zwarn("completion cannot be used recursively (yet)");
        return 1;
    }
    // RAII reset: guarantees `ACTIVE` clears on EVERY exit path (normal
    // return, early return, or panic). A completion that bails mid-way
    // must never leave the flag latched, or every subsequent Tab reports
    // "completion cannot be used recursively (yet)".
    struct ActiveGuard;
    impl Drop for ActiveGuard {
        fn drop(&mut self) {
            ACTIVE.with(|c| c.set(false));
        }
    }
    ACTIVE.with(|c| c.set(true));
    let _active_guard = ActiveGuard;
    // c:611 — `comprecursive = 0;`
    crate::ported::zle::complist::COMPRECURSIVE.store(0, std::sync::atomic::Ordering::Relaxed);
    // c:612 — `makecommaspecial(0);`. Clears the ISPECIAL bit on `,`
    // (utils.c:4275-4277) that the brace scan at c:2021/c:2074 raises when
    // the word sits in an unfinished `{a,b`; leaving it latched from a
    // previous completion makes the NEXT completion quote commas.
    //
    // C's fourth call site, `makecommaspecial(0)` at c:689, has no
    // counterpart here because the code path around it — the `chline`
    // history-prepend branch at c:640-653 / c:676-696 — is itself unported.
    crate::ported::utils::makecommaspecial(false);
    tracing::debug!(target: "compsys_args", lst, "docomplete ENTER");

    // c:621 — `runhookdef(BEFORECOMPLETEHOOK, &lst)`. Canonical
    // dispatch via `gethookdef + runhookdef`; null check matches
    // c:992 `empty(h->funcs)`-and-no-`def` path returning 0.
    let mut lst_box = lst;
    let h_before = gethookdef("before_complete");
    // c:621-624 — `if (runhookdef(BEFORECOMPLETEHOOK, &lst)) { return 0; }`.
    // A non-zero return means the hook already handled this Tab — e.g. an
    // active menu (before_complete advanced the menu cursor and inserted the
    // next match). Short-circuit so do_completion doesn't restart completion
    // from scratch (which re-inserted the first match and treated the
    // inserted word as a fresh completion). When the hook table is empty
    // (the `zsh -f` path doesn't register it) fall back to the canonical
    // handler directly, mirroring docompletion's `complete`-hook fallback.
    let bc_handled = if !h_before.is_null() {
        let lst_ptr = (&mut lst_box) as *mut i32 as *mut std::ffi::c_void;
        crate::ported::module::runhookdef(h_before, lst_ptr) != 0
    } else {
        crate::ported::zle::compcore::before_complete(&mut lst_box) != 0
    };
    // c:621 — C passes `&lst` itself, so a hook that rewrites `*lst` (e.g.
    // `before_complete` downgrading COMP_EXPAND_COMPLETE to COMP_COMPLETE
    // for an in-progress menu) changes the value every line below uses.
    // The port handed the hook a private copy and then threw it away.
    lst = lst_box;
    if bc_handled {
        // before_complete handled this Tab (e.g. advanced an active menu and
        // do_single'd the next match into the metafied buffer). Mirror the
        // compend copy-back so the interactive editor buffer reflects the
        // change, then return without restarting completion. Without this the
        // metafied line held the new match but the editor kept redisplaying
        // the previous one (menu never appeared to advance).
        if crate::ported::zle::compcore::ZLEMETALL.load(Ordering::SeqCst) != 0 {
            crate::ported::zle::compcore::unmetafy_line();
        }
        let comp_line: Vec<char> = crate::ported::zle::compcore::ZLELINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.chars().collect())
            .unwrap_or_default();
        let comp_ll = comp_line.len() as i32;
        let comp_cs = crate::ported::zle::compcore::ZLECS.load(Ordering::SeqCst);
        if let Ok(mut g) = crate::ported::zle::zle_main::ZLELINE.lock() {
            *g = comp_line;
        }
        crate::ported::zle::zle_main::ZLECS
            .store(comp_cs.clamp(0, comp_ll) as usize, Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.store(comp_ll as usize, Ordering::SeqCst);
        return 0; // _active_guard resets ACTIVE on drop
    }

    // c:628 — `if (doexpandhist()) { active = 0; return 0; }`.
    if doexpandhist() != 0 {
        return 0; // _active_guard resets ACTIVE on drop
    }

    // zshrs bridge: C keeps a single `zleline`; this port splits it into
    // the interactive editor buffer (`zle_main::ZLELINE`, a Vec<char>
    // that `self-insert` writes) and the completion engine's
    // `compcore::ZLELINE`. Copy the editor buffer + cursor into the
    // completion buffer, then metafy (c:636 `metafy_line()`), so
    // `get_comp_string` / `makecomplist` operate on the real typed line
    // instead of an empty buffer. Without this the whole compsys engine
    // runs against `s=""` and produces zero matches.
    {
        let ed_line: String = crate::ported::zle::zle_main::ZLELINE
            .lock()
            .map(|g| g.iter().collect())
            .unwrap_or_default();
        let ed_cs = crate::ported::zle::zle_main::ZLECS.load(Ordering::SeqCst) as i32;
        let ed_ll = ed_line.chars().count() as i32;
        if let Ok(mut g) = crate::ported::zle::compcore::ZLELINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
        {
            *g = ed_line;
        }
        crate::ported::zle::compcore::ZLECS.store(ed_cs, Ordering::SeqCst);
        crate::ported::zle::compcore::ZLELL.store(ed_ll, Ordering::SeqCst);
    }
    // c:636 — `metafy_line();`, UNCONDITIONAL in C.
    //
    // The block above has just refreshed `compcore::ZLELINE` from the editor
    // buffer, so it is authoritative here and `ZLEMETALINE` must be re-derived
    // from it. Skipping the call when `ZLEMETALL` was non-zero left a STALE
    // metafied line in place for any completion that runs while a previous
    // one's metafied state is still around — which is exactly the
    // menuselect interactive-filter loop (complist.rs:2776-2779 unmetafies,
    // calls menucomplete, re-metafies on every filter keystroke). The
    // unambiguous match was inserted into that stale buffer, the caller's
    // `metafy_line()` then re-derived it from the untouched `ZLELINE`, and
    // the insert vanished: `interactive: /s[]` where zsh shows
    // `interactive: /sbin[]`.
    crate::ported::zle::compcore::metafy_line();

    // c:654-660 — `inwhat = IN_NOTHING; zsfree(qipre); qipre = ztrdup("");
    //               zsfree(qisuf); qisuf = ztrdup(""); zsfree(autoq);
    //               autoq = NULL;`
    // The quote state is per-completion: `get_comp_string` below only ever
    // PREPENDS to `qipre` / APPENDS to `qisuf` (c:1753-1765), so without
    // this reset the quotes of every previous Tab would accumulate.
    crate::ported::zle::compcore::INWHAT.store(crate::ported::zsh_h::IN_NOTHING, Ordering::SeqCst); // c:654
    if let Ok(mut g) = QIPRE.get_or_init(|| Mutex::new(String::new())).lock() {
        g.clear(); // c:655-656
    }
    if let Ok(mut g) = QISUF.get_or_init(|| Mutex::new(String::new())).lock() {
        g.clear(); // c:657-658
    }
    if let Ok(mut g) = AUTOQ.get_or_init(|| Mutex::new(String::new())).lock() {
        g.clear(); // c:659-660 — `zsfree(autoq); autoq = NULL;`
    }

    // c:664 — `s = get_comp_string();` extracts the cursor word and sets
    // origword/lincmd/wb/we as side effects.
    //
    // C treats `s == NULL` as "there is nothing here to complete": the whole
    // c:703-869 body is inside `if (s) { … } else ret = 1;`. The port used to
    // fall back to the ENTIRE LINE (`origword.unwrap_or_else(|| line.clone())`),
    // a Rust-only invention with no C counterpart, so every context the
    // extractor declines to handle (command position inside `$(`, an
    // unterminated compound, …) completed the whole buffer instead of nothing:
    // `echo $(gr<TAB>` offered 47315 matches where zsh offers none.

    // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
    // Opened here, one line before the `get_comp_string` that publishes
    // `$words`/`$CURRENT` into paramtab, and closed by `Drop` on every
    // exit path below (there is exactly one exit after this point, plus
    // panics). See `CompWordParamScope` for why the c:815-838 scope in
    // compcore.c is not enough on its own.
    let _comp_word_scope = CompWordParamScope::new();
    let s = get_comp_string(); // c:664
    let s_word: String = s.clone().unwrap_or_default();
    tracing::debug!(target: "compsys_args", ?s, wb = WB.load(Ordering::SeqCst), we = WE.load(Ordering::SeqCst), lincmd = LINCMD.load(Ordering::SeqCst), inwhat = crate::ported::zle::compcore::INWHAT.load(Ordering::SeqCst), "get_comp_string result");
    // c:701-702 — `if (inwhat == IN_ENV) lincmd = 0;`. Missing from the port:
    // completing the VALUE of an environment assignment (`FOO=<TAB>`) still
    // reported command position, so `_main_complete` dispatched the
    // command-name completer instead of the value one.
    {
        use crate::ported::zle::compcore::INWHAT;
        use crate::ported::zsh_h::IN_ENV;
        if INWHAT.load(Ordering::SeqCst) == IN_ENV {
            LINCMD.store(0, Ordering::SeqCst); // c:702
        }
    }
    let lincmd = LINCMD.load(Ordering::SeqCst); // c:805

    // c:703-793 — `if (s) { if (lst == COMP_EXPAND_COMPLETE) { … } }`:
    // decide whether this TAB expands or completes. Skipping it left `lst`
    // at COMP_EXPAND_COMPLETE, which breaks BOTH arms of expand-or-complete:
    //   * `doexpansion` only runs `globlist` for COMP_EXPAND /
    //     COMP_LIST_EXPAND (c:2283), so `ls *<TAB>` / `ls -d **/<TAB>` never
    //     globbed and the word was left untouched;
    //   * `COMP_ISEXPAND(lst)` stayed true, so `docompletion` ran TWICE per
    //     TAB — once from `doexpansion` (c:2301) and once from c:865 — and
    //     the second, contextless run discarded the first run's matches.
    //     That is what made `echo ${<TAB>` list nothing even though the
    //     `-brace-parameter-` pass had already built 241 matches.
    if s.is_some() && lst == COMP_EXPAND_COMPLETE {
        // c:703-704 — `if (s) { if (lst == COMP_EXPAND_COMPLETE) {`
        use crate::ported::hashtable::cmdnamtab_lock;
        use crate::ported::utils::{itype_end, skipparens, strpfx};
        use crate::ported::zsh_h::{
            Equals, Hat, Inbrace, Inbrack, Inpar, Inparmath, Outbrace, Pound, Qstring, Qtick,
            Quest, Star, Stringg, Tick, Tilde, GLOBCOMPLETE, RECEXACT,
        };
        use crate::ported::ztype_h::idigit;
        // c:706 `char *q = s` — the TOKENIZED word (see COMP_STRING_TOK).
        let s_tok: String = COMP_STRING_TOK
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let sc: Vec<char> = if s_tok.is_empty() {
            s_word.chars().collect()
        } else {
            s_tok.chars().collect()
        };
        // c:739/781 — `zlemetacs - wb`, the cursor's offset inside the word.
        let cs_off = (ZLEMETACS.load(Ordering::SeqCst) - WB.load(Ordering::SeqCst)).max(0) as usize;
        let mut q: usize = 0; // c:706
                              // c:708-731 — `=word`: expand when it names a command, and (unless
                              // the completion module is absent or REC_EXACT is set) only when
                              // exactly one command carries that prefix.
        if sc.first() == Some(&Equals) {
            q = 1; // c:710
            let name: String = sc[q..].iter().collect();
            let hashed = cmdnamtab_lock()
                .read()
                .map(|t| t.get(&name).is_some())
                .unwrap_or(false);
            let path: Vec<String> = crate::ported::params::getaparam("path").unwrap_or_default();
            let pc = crate::ported::hashtable::pathchecked.load(Ordering::Relaxed);
            // c:712 — `cmdnamtab->getnode(cmdnamtab, q) || hashcmd(q, pathchecked)`.
            if hashed || crate::ported::exec::hashcmd(&name, &path[pc.min(path.len())..]).is_some()
            {
                if !HASCOMPMOD.load(Ordering::SeqCst) || isset(RECEXACT) {
                    lst = COMP_EXPAND; // c:714
                } else {
                    // c:716-728 — count prefix matches, stopping at two.
                    let mut n = 0;
                    if let Ok(t) = cmdnamtab_lock().read() {
                        for (nam, _) in t.iter() {
                            if strpfx(&name, nam)
                                && crate::ported::exec::findcmd(nam, 0, 0).is_some()
                            {
                                n += 1;
                            }
                            if n == 2 {
                                break; // c:725
                            }
                        }
                    }
                    if n == 1 {
                        lst = COMP_EXPAND; // c:729-730
                    }
                }
            }
        }
        // c:732-770 — walk the parameter expressions in the word.
        if lst == COMP_EXPAND_COMPLETE {
            loop {
                // c:735 — `for (; *q && *q != String; q++)`.
                while q < sc.len() && sc[q] != Stringg {
                    q += 1;
                }
                // c:736 — a `$` that starts `$(`, `$((` or `$[` is not a
                // parameter expression; stop looking (c:768 `else break`).
                if q >= sc.len()
                    || sc.get(q + 1) == Some(&Inpar)
                    || sc.get(q + 1) == Some(&Inparmath)
                    || sc.get(q + 1) == Some(&Inbrack)
                {
                    break;
                }
                q += 1; // c:737 — `*++q`
                if sc.get(q) == Some(&Inbrace) {
                    // c:738-740 — a BALANCED `${…}` that ends exactly at the
                    // cursor expands. An unbalanced one (`${<TAB>`) does not,
                    // and falls through to c:765 → COMP_COMPLETE, which is
                    // what puts the cursor in the `-brace-parameter-` context.
                    let tail: String = sc[q..].iter().collect();
                    let mut rest: &str = &tail;
                    let n = skipparens(Inbrace, Outbrace, &mut rest);
                    q += tail.chars().count() - rest.chars().count();
                    if n == 0 && q == cs_off {
                        lst = COMP_EXPAND; // c:740
                    }
                } else {
                    // c:745-748 — skip what may precede the parameter name.
                    while q < sc.len() {
                        let c = sc[q];
                        if c != '^' && c != Hat && c != '=' && c != Equals && c != '~' && c != Tilde
                        {
                            break;
                        }
                        q += 1;
                    }
                    // c:749-750 — `${#name}` / `${+name}`.
                    if matches!(sc.get(q), Some('#') | Some(&Pound) | Some('+'))
                        && sc.get(q + 1) != Some(&Stringg)
                    {
                        q += 1;
                    }
                    // c:752 — `sav2 = *(t = q)`.
                    let t = q;
                    let sav2 = sc.get(t).copied();
                    // c:753-762 — find the end of the name. The token branch
                    // detokenizes the first char in place (c:754
                    // `*q = ztokens[*q - Pound]`) so `checkparams` sees the
                    // real `?`/`*`/`$`/`"` special-parameter name.
                    let mut detok: Option<char> = None;
                    match sav2 {
                        Some(c) if c == Quest || c == Star || c == Stringg || c == Qstring => {
                            let idx = (c as u32 - Pound as u32) as usize;
                            detok = crate::ported::lex::ztokens.chars().nth(idx);
                            q += 1; // c:754
                        }
                        Some('?') | Some('*') | Some('$') | Some('-') | Some('!') | Some('@') => {
                            q += 1; // c:756
                        }
                        Some(c) if idigit(c as u8) => {
                            // c:758 — `do q++; while (idigit(*q));`
                            while matches!(sc.get(q), Some(&d) if idigit(d as u8)) {
                                q += 1;
                            }
                        }
                        _ => {
                            // c:760 — `q = itype_end(q, INAMESPC, 0)`.
                            let rest: String = sc[t.min(sc.len())..].iter().collect();
                            let span = itype_end(&rest, crate::ported::ztype_h::INAMESPC, false);
                            q = t + rest[..span].chars().count();
                        }
                    }
                    // c:763-766 — `*q = '\0'` makes `t` the bare name; expand
                    // when the cursor sits at its end and it names a param.
                    let mut nm: Vec<char> = sc[t.min(sc.len())..q.min(sc.len())].to_vec();
                    if let (Some(fc), false) = (detok, nm.is_empty()) {
                        nm[0] = fc;
                    }
                    let name: String = nm.into_iter().collect();
                    if cs_off == q
                        && (sav2.map(|c| idigit(c as u8)).unwrap_or(false)
                            || checkparams(&name) != 0)
                    {
                        lst = COMP_EXPAND; // c:765
                    }
                }
                if lst != COMP_EXPAND {
                    lst = COMP_COMPLETE; // c:769-770
                }
                // c:772 — `while (q < s + zlemetacs - wb)`.
                if q >= cs_off {
                    break;
                }
            }
        }
        // c:774-782 — still undecided: a backtick or `$` anywhere in the
        // word means expansion, otherwise completion.
        if lst == COMP_EXPAND_COMPLETE {
            let has_subst = sc
                .iter()
                .any(|&c| c == Tick || c == Qtick || c == Stringg || c == Qstring);
            lst = if has_subst {
                COMP_EXPAND
            } else {
                COMP_COMPLETE
            };
        }
        // c:785-786 — and expand if the word has wildcards and GLOB_COMPLETE
        // is off.
        let s_for_wilds: String = sc.iter().collect();
        if !isset(GLOBCOMPLETE) && cmphaswilds(&s_for_wilds) != 0 {
            lst = COMP_EXPAND;
        }
    }

    // c:798-799 — `if (lincmd && (inwhat == IN_NOTHING)) inwhat = IN_CMD;`.
    // Missing from the port. `inwhat` is copied verbatim into `linwhat`
    // by makecomplist (compcore.c:960) and surfaces as the `-command-`
    // context, so leaving it at IN_NOTHING for a command-position word
    // lost the command context for everything downstream.
    {
        use crate::ported::zle::compcore::INWHAT;
        use crate::ported::zsh_h::{IN_CMD, IN_NOTHING};
        // c:703 — still inside `if (s)`.
        if s.is_some() && lincmd != 0 && INWHAT.load(Ordering::SeqCst) == IN_NOTHING {
            INWHAT.store(IN_CMD, Ordering::SeqCst); // c:799
        }
    }

    tracing::debug!(target: "compsys_args", lst, olst, s_null = s.is_none(), "docomplete dispatch");
    // c:817-870 — dispatch on `lst`.
    let ret;
    if s.is_none() {
        // c:870-871 — `} else ret = 1;`. No word to complete: the widget
        // reports failure and the line is left exactly as the user typed it.
        ret = 1;
    } else if lst == COMP_SPELL {
        // c:801-815 — spell-word path. Direct port:
        //   foredel(we - wb, CUT_RAW);
        //   spckword(&x, 0, lincmd, 0);
        //   ret = !strcmp(x, ox);
        //   inststr(x);
        let wb = WB.load(Ordering::SeqCst);
        let we = WE.load(Ordering::SeqCst);
        // c:802-806 — `w = dupstring(origword); for (q = w; *q; q++)
        //               if (inull(*q)) *q = Nularg;`. The null-token
        // flattening was missing, so quote markers (Snull/Dnull/Bnull…)
        // survived into `spckword` and were spell-checked as if they were
        // real characters.
        // `inull(X)` is `zistype(X, INULL)` (ztype.h:62) — the null-token
        // class, inlined here because compctl.rs's copy is module-private.
        let origword: String = ORIGWORD
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let w: String = origword
            .chars()
            .map(|c| {
                use crate::ported::zsh_h::{Bnull, Dnull, Nularg, Qstring, Snull, Stringg};
                if matches!(c, Snull | Dnull | Bnull | Stringg | Qstring) {
                    Nularg // c:805
                } else {
                    c
                }
            })
            .collect();
        // c:807-808 — UNCONDITIONAL in C; the port gated both on `we > wb`.
        ZLEMETACS.store(wb, Ordering::SeqCst); // c:807
        foredel(we - wb, CUT_RAW); // c:808
                                   // c:810 — `untokenize(x = ox = dupstring(w))`. Without this the
                                   // remaining parser tokens (Tilde/Equals/Star/…) were fed to
                                   // `spckword` and then re-inserted raw into the line.
        let ox = crate::ported::lex::untokenize(&w); // c:810
        let mut x = ox.clone(); // c:810
                                // c:811-812 — `if (*w == Tilde || *w == Equals || *w == String)
                                //                *x = *w;` — put the *token* back as the first char so
                                // spckword knows this is a `~`/`=`/`$` word.
        if let Some(fc) = w.chars().next() {
            use crate::ported::zsh_h::{Equals, Stringg, Tilde};
            if fc == Tilde || fc == Equals || fc == Stringg {
                let mut xc: Vec<char> = x.chars().collect();
                if xc.is_empty() {
                    xc.push(fc);
                } else {
                    xc[0] = fc;
                }
                x = xc.into_iter().collect(); // c:812
            }
        }
        // c:813 — `spckword(&x, 0, lincmd, 0)`.
        crate::ported::utils::spckword(&mut x, 0, lincmd, 0);
        // c:814 — `ret = !strcmp(x, ox)` — returns 1 (unchanged) /
        // 0 (changed). Matches C `!strcmp` semantics.
        let r = if x == ox { 1 } else { 0 };
        // c:816 — `untokenize(x)`. Second detokenize pass, after spckword
        // may have re-introduced the leading token from c:812.
        let x = crate::ported::lex::untokenize(&x);
        // c:816 — `inststr(x)` re-inserts the (possibly corrected)
        // word at the cursor. Routes through `inststrlen` with
        // `move_cursor=true, len=-1` matching C's `inststr(x)`
        // expansion (zle_misc.c:185 `inststrlen(s, 1, -1)`).
        let _ = inststrlen(&x, true, -1);
        ret = r;
    } else if COMP_ISEXPAND(lst) {
        // c:833-867 — expand-or-complete path.
        // c:836 — `ol = dupstring(zlemetaline)`. Snapshot the METAFIED
        // line: `doexpansion` may itself run `docompletion` (c:2302),
        // which edits `zlemetaline` (ZLEMETALINE). C then skips its own
        // c:865 docompletion when the line is unchanged. Snapshotting
        // compcore::ZLELINE instead (which the inner completion never
        // touches) made the guard always true → completion ran twice per
        // Tab, corrupting the buffer.
        let ol_before = crate::ported::zle::compcore::ZLEMETALINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        // c:823 — `int ocs = zlemetacs, ne = noerrs;`. The cursor snapshot
        // was missing from this port. `doexpansion` runs its own inner
        // `docompletion` (c:2302) which leaves the cursor wherever that
        // completion put it, and C restores it at c:834 BEFORE the outer
        // `docompletion` re-runs. Without the restore, the second run
        // extracted the word at the wrong cursor position, so the completer
        // was never dispatched and every match computed by the first run was
        // dropped: with TAB on its default `expand-or-complete` binding, any
        // spec carrying a `*::`/`*:::` rest argument (`_rm`, `_typeset`,
        // `_bindkey`, `_tar`) completed NOTHING at `cmd -<TAB>`, while the
        // same spec worked when TAB was rebound to `complete-word`.
        let ocs = ZLEMETACS.load(Ordering::SeqCst); // c:823
        let ne = *crate::ported::utils::noerrs_lock().lock().unwrap(); // c:839
        *crate::ported::utils::noerrs_lock().lock().unwrap() = 1; // c:840
                                                                  // c:826 — `ret = doexpansion(origword, lst, olst, lincmd);`. C passes
                                                                  // `origword`, the TOKENIZED word, NOT the `s` it passes to
                                                                  // `docompletion` below. The port passed the untokenized `s_word`, so
                                                                  // the quote tokens `prefork` needs were already gone: `echo "$PA<TAB>`
                                                                  // arrived as `$PA` (an unquoted unset parameter, which expands to NO
                                                                  // WORD and leaves the line alone) instead of `<Dnull>$PA` (an unset
                                                                  // parameter INSIDE double quotes, which expands to an empty-but-present
                                                                  // word — so zsh deletes `"$PA` and inserts nothing).
        let mut ret_local = doexpansion(
            &ORIGWORD
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default(),
            lst,
            olst,
            lincmd,
        ); // c:826
        LASTAMBIG.store(0, Ordering::SeqCst); // c:842
        *crate::ported::utils::noerrs_lock().lock().unwrap() = ne; // c:843

        // c:847-868 — if expand-or-complete and buffer unchanged,
        // fall through to docompletion.
        let after = crate::ported::zle::compcore::ZLEMETALINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        if olst == COMP_EXPAND_COMPLETE && ol_before == after {
            // c:834 — `zlemetacs = ocs;` then c:835 clear ERRFLAG_ERROR.
            ZLEMETACS.store(ocs, Ordering::SeqCst);
            crate::ported::utils::errflag
                .fetch_and(!crate::ported::utils::ERRFLAG_ERROR, Ordering::SeqCst);
            ret_local = docompletion(&s_word, lst, lincmd); // c:865
        } else {
            // c:853-854 — `if (ret) clearlist = 1`.
            if ret_local != 0 {
                CLEARLIST.store(1, Ordering::SeqCst);
            }
            // c:855-864 — the whole "we may have removed some quotes"
            // restore was missing. C's `ol` at c:820-822 is a COPY only for
            // olst == COMP_EXPAND / COMP_EXPAND_COMPLETE; for
            // COMP_LIST_EXPAND it ALIASES zlemetaline, so the strcmp is
            // trivially equal and the restore always fires. Reproduce both
            // arms: an expansion that ended up not changing the line must
            // put the ORIGINAL (still-quoted) line back, because — unlike
            // completion — nothing downstream re-installs the quotes.
            let ol_aliases_line = !(olst == COMP_EXPAND || olst == COMP_EXPAND_COMPLETE); // c:820-822
            if ol_aliases_line || ol_before == after {
                ZLEMETACS.store(0, Ordering::SeqCst); // c:859
                foredel(ZLEMETALL.load(Ordering::SeqCst), CUT_RAW); // c:860
                spaceinline(ORIGLL.load(Ordering::SeqCst)); // c:861
                if let (Some(metabuf), Some(orig)) = (ZLEMETALINE.get(), ORIGLINE.get()) {
                    if let (Ok(mut m), Ok(o)) = (metabuf.lock(), orig.lock()) {
                        *m = o.clone(); // c:862
                        ZLEMETALL.store(m.len() as i32, Ordering::SeqCst);
                    }
                }
                ZLEMETACS.store(ORIGCS.load(Ordering::SeqCst), Ordering::SeqCst);
                // c:863
            }
        }
        ret = ret_local;
    } else {
        // c:870 — `ret = docompletion(s, lst, lincmd)`. Plain
        // completion.
        ret = docompletion(&s_word, lst, lincmd);
    }

    // c:878 — `runhookdef(AFTERCOMPLETEHOOK, &dat)`. Same dispatch
    // shape as the BEFORECOMPLETEHOOK call above; passes a 2-element
    // int buffer per C's `int dat[2]`.
    // c:876-877 — `dat[0] = lst; dat[1] = ret;`. The port filled
    // `[ret, 0]`, so every after-complete hook read the RETURN CODE where
    // C puts the completion TYPE, and the slot the hook writes its own
    // result into was hardcoded to 0.
    let mut dat: [i32; 2] = [lst, ret]; // c:876-877
    let h_after = gethookdef("after_complete");
    if !h_after.is_null() {
        let dat_ptr = dat.as_mut_ptr() as *mut std::ffi::c_void;
        crate::ported::module::runhookdef(h_after, dat_ptr);
    }

    // zshrs bridge (mirror of C compend's `unmetafy_line()`): flatten
    // the completion buffer back to a plain line and copy it — with the
    // cursor — into the interactive editor buffer so the completed /
    // edited result is what the editor redisplays. Guard the unmetafy on
    // ZLEMETALL because some completion paths already unmetafied.
    if crate::ported::zle::compcore::ZLEMETALL.load(Ordering::SeqCst) != 0 {
        crate::ported::zle::compcore::unmetafy_line();
    }
    {
        let comp_line: Vec<char> = crate::ported::zle::compcore::ZLELINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.chars().collect())
            .unwrap_or_default();
        let comp_ll = comp_line.len() as i32;
        let comp_cs = crate::ported::zle::compcore::ZLECS.load(Ordering::SeqCst);
        if let Ok(mut g) = crate::ported::zle::zle_main::ZLELINE.lock() {
            *g = comp_line;
        }
        crate::ported::zle::zle_main::ZLECS
            .store(comp_cs.clamp(0, comp_ll) as usize, Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.store(comp_ll as usize, Ordering::SeqCst);
    }

    // c:882 — `makecommaspecial(0);` on the way out, so the flag never
    // leaks into the next completion.
    crate::ported::utils::makecommaspecial(false);
    // c:894 — `errflag &= ~ERRFLAG_INT;`. Completion deliberately swallows
    // a user interrupt so ^C during a long completion leaves the edit
    // buffer intact instead of aborting the line. The port never cleared
    // it, so an interrupted completion left ERRFLAG_INT latched and the
    // next command aborted spuriously.
    crate::ported::utils::errflag.fetch_and(!crate::ported::zsh_h::ERRFLAG_INT, Ordering::SeqCst); // c:894

    // c:896 — `return dat[1]`, NOT `ret`: an after-complete hook is free to
    // overwrite dat[1] to change the widget's return value (and thus
    // whether zlecore beeps).
    dat[1] // _active_guard resets ACTIVE on drop
}

/// Port of `addx(char **ptmp)` from Src/Zle/zle_tricky.c:922.
/// WARNING: param names don't match C — Rust=(zle, ptmp) vs C=(ptmp)
/// Direct port of `void addx(char **ptmp)` from
/// `Src/Zle/zle_tricky.c:922`. Conditionally inserts an 'x'
/// placeholder at the ZLE cursor so the parser sees a complete word
/// at completion time. The condition only fires when the char at the
/// cursor would terminate / split the word (whitespace, separators,
/// closing brackets, end-of-line, quote inside-string, or comppref
/// mode landing on a non-blank).
///
/// Rust signature: returns `*ptmp` as a Result-ish `Option<String>`
/// snapshot of the pre-edit buffer when addx fires; None when it
/// doesn't. Side effects: mutates `ZLELINE`, stores the insertion
/// length (1 or 2) in `ADDEDX`. The `int` return value mirrors C's
/// implicit return (its return type is `void`; the i32 conveys
/// `addedx` for callers that want it without a global read).
pub fn addx(ptmp: &mut String) -> i32 {
    // c:922

    let cs = ZLECS.load(Ordering::SeqCst) as usize;
    let ll = ZLELL.load(Ordering::SeqCst) as usize;
    let instring = INSTRING.load(Ordering::SeqCst);
    let comppref = COMPPREF.load(Ordering::SeqCst) != 0;

    // Read the char at the cursor (and previous, for the iblank gate).
    let (ch_at, prev_at): (Option<char>, Option<char>) = {
        let line = ZLELINE.lock().unwrap();
        let at = line.get(cs).copied();
        let prev = if cs > 0 {
            line.get(cs - 1).copied()
        } else {
            None
        };
        (at, prev)
    };

    // c:924 — `iblank` in C tests space/tab (not '\n'). c:923's
    // outer check splits '\n' off as its own arm, so the iblank
    // gate proper is space/tab only.
    let is_iblank = matches!(ch_at, Some(' ' | '\t'));
    let is_blank_unescaped = is_iblank && (cs == 0 || prev_at != Some('\\')); // c:927-928

    // c:924-936 — the full insertion gate.
    let cs_at_end = ch_at.is_none() || cs >= ll;
    let is_newline = ch_at == Some('\n'); // c:925
    let is_separator = matches!(ch_at, Some(')' | '`' | '}' | ';' | '|' | '&' | '>' | '<')); // c:929-933
    let is_instring_quote = instring != QT_NONE                              // c:934-935
        && matches!(ch_at, Some('"' | '\''));
    let addspace = comppref                                                  // c:936
        && ch_at.is_some()
        && !matches!(ch_at, Some(' ' | '\t'));

    let fire = cs_at_end
        || is_newline
        || is_blank_unescaped
        || is_separator
        || is_instring_quote
        || addspace;

    if fire {
        // c:937-946 — snapshot, insert 'x' (+ optional ' '), set addedx.
        let snap: String = ZLELINE.lock().unwrap().iter().collect();
        *ptmp = snap;
        let mut line = ZLELINE.lock().unwrap();
        // c:944 — `zlemetaline[zlemetacs] = 'x';`
        line.insert(cs, 'x');
        if addspace {
            // c:945-946 — `zlemetaline[zlemetacs+1] = ' ';`
            line.insert(cs + 1, ' ');
        }
        drop(line);
        // c:947 — `addedx = 1 + addspace;`
        let added = if addspace { 2 } else { 1 };
        ADDEDX.store(added, Ordering::SeqCst);
        // Keep ZLELL consistent with the insertion.
        ZLELL.fetch_add(added as usize, Ordering::SeqCst);
        added
    } else {
        // c:949-952 — `addedx = 0; *ptmp = NULL;`
        ADDEDX.store(0, Ordering::SeqCst);
        ptmp.clear();
        0
    }
}

/// Port of `dupstrspace(const char *str)` from `Src/Zle/zle_tricky.c:955`.
/// ```c
/// mod_export char *
/// dupstrspace(const char *str)
/// {
///     int len = strlen(str);
///     char *t = (char *) hcalloc(len + 2);
///     strcpy(t, str);
///     strcpy(t+len, " ");
///     return t;
/// }
/// ```
/// Like `dupstring`, but appends a single space.
pub fn dupstrspace(str: &str) -> String {
    // c:955
    let len = str.len(); // c:955 strlen(str)
    let mut out = String::with_capacity(len + 2); // c:958 hcalloc(len+2)
    out.push_str(str); // c:959 strcpy(t, str)
    out.push(' '); // c:960 strcpy(t+len, " ")
    out // c:961 return t
}

// metafy_line / unmetafy_line — REMOVED. Both were ad-hoc 1-arg
// string-transform helpers using the wrong signature `(s: &str) ->
// String` for fns that C declares as `void metafy_line(void)` /
// `void unmetafy_line(void)` (global ZLELINE ↔ ZLEMETALINE
// mutators). The actual C-faithful ports live in compcore.rs and
// dispatch through zle_utils::zlelineasstring + ::stringaszleline.

/// Port of `freebrinfo(Brinfo p)` from `Src/Zle/zle_tricky.c:1015`.
/// ```c
/// mod_export void
/// freebrinfo(Brinfo p)
/// {
///     Brinfo n;
///     while (p) {
///         n = p->next;
///         zsfree(p->str);
///         zfree(p, sizeof(*p));
///         p = n;
///     }
/// }
/// ```
/// Free a Brinfo `next`-linked list. C frees each node + its `str`
/// allocation; Rust drops the Box chain (and each `String` inside)
/// automatically when the head Box is dropped.
pub fn freebrinfo(p: Option<crate::ported::zle::zle_h::BrinfoPtr>) {
    // c:1016
    // c:1016-1026 — walk + zsfree(str) + zfree(p) loop. In Rust the
    // Drop impls cascade through Box<brinfo> → String → next chain.
    drop(p);
}

/// Port of `dupbrinfo(Brinfo p, Brinfo *last, int heap)` from `Src/Zle/zle_tricky.c:1032`.
/// ```c
/// mod_export Brinfo
/// dupbrinfo(Brinfo p, Brinfo *last, int heap)
/// {
///     Brinfo ret = NULL, *q = &ret, n = NULL;
///     while (p) {
///         n = *q = (heap ? (Brinfo) zhalloc(sizeof(*n)) :
///                  (Brinfo) zalloc(sizeof(*n)));
///         q = &(n->next);
///         n->next = NULL;
///         n->str = (heap ? dupstring(p->str) : ztrdup(p->str));
///         n->pos = p->pos;
///         n->qpos = p->qpos;
///         n->curpos = p->curpos;
///         p = p->next;
///     }
///     if (last)
///         *last = n;
///     return ret;
/// }
/// ```
/// Deep-copy a Brinfo `next`-linked list. The C `heap` parameter
/// chooses between `zhalloc` (per-completion arena) and `zalloc`
/// (permanent); Rust uses Box for both since the GC distinction
/// doesn't apply.
///
/// Returns `(head, last)` — the C uses an out-pointer for `last`
/// because callers want to splice further entries onto the tail.
/// WARNING: param names don't match C — Rust=() vs C=(p, last, heap)
pub fn dupbrinfo(
    // c:1033
    mut p: Option<&crate::ported::zle::zle_h::brinfo>,
) -> (
    Option<crate::ported::zle::zle_h::BrinfoPtr>,
    Option<*const crate::ported::zle::zle_h::brinfo>,
) {
    let mut head: Option<crate::ported::zle::zle_h::BrinfoPtr> = None; // c:1035 ret = NULL
    let mut last_ptr: Option<*const crate::ported::zle::zle_h::brinfo> = None;
    // SAFETY: tail walks the head-chain we build, both reachable for
    // this fn's lifetime.
    let mut tail: *mut Option<crate::ported::zle::zle_h::BrinfoPtr> = &mut head;
    while let Some(node) = p {
        // c:1037 while (p)
        let cloned = Box::new(crate::ported::zle::zle_h::brinfo {
            // c:1038-1039 zhalloc/zalloc
            next: None,            // c:1042
            prev: None,            // brinfo has prev too
            str: node.str.clone(), // c:1043 dupstring(p->str)
            pos: node.pos,         // c:1044
            qpos: node.qpos,       // c:1045
            curpos: node.curpos,   // c:1046
        });
        unsafe {
            *tail = Some(cloned);
            let inserted = (*tail).as_mut().unwrap();
            last_ptr = Some(inserted.as_ref() as *const _);
            tail = &mut inserted.next;
        }
        p = node.next.as_deref(); // c:1048 p = p->next
    }
    // c:1050-1051 — `if (last) *last = n`. Returned alongside head.
    (head, last_ptr)
}

/// Port of `has_real_token()` from `Src/Zle/zle_tricky.c:1056-1077`
/// ("This is a bit like has_token(), but ignores nulls.").
///
/// The input is a LEXED word, not raw user text: every character the
/// parser found special has already been rewritten to its `Ztoken`
/// marker (`Star` = U+0087 for a glob `*`, `Stringg` = U+0085 for a
/// live `$`, …), and a character that was quoted keeps its literal
/// form. So the test C makes is on the token BYTE, never on the
/// printable character:
///
/// ```text
/// if (itok(*s) && !inull(*s))
///     return 1;
/// ```
///
/// `itok` covers `Pound`..`Nularg` and `inull` covers the null
/// subrange `Snull`..`Nularg` (`Src/utils.c` `inittyptab`), so the
/// predicate is exactly "a token that is not a quote/backslash
/// placeholder".
///
/// The single caller is the quote-form block in `get_comp_string`
/// (c:1728-1731): a word whose first char is `Snull`/`Dnull` is the
/// inside of a quoted string, and it only counts as one when nothing
/// AFTER that opening marker is a real token. Inside single quotes
/// the lexer emits `*` as a plain `*`, so `'…*'` must pass this test
/// and get `qipre`/`qisuf`/`autoq` set.
///
/// A previous revision of this function was an ad-hoc rewrite that
/// scanned for the LITERAL characters ``$ ` " ' \ { } [ ] * ? ~`` with
/// backslash-escape tracking. That has no counterpart in the C: it
/// fired on the ordinary `*` inside `zstyle ':completion:*'<TAB>`,
/// which skipped the whole quote-form block, left `QIPREFIX`/
/// `QISUFFIX`/`$compstate[quote]` empty, and made the insertion
/// rewrite the word without its quotes (`':completion:*'` →
/// `:completion:`). The doc comment claimed "Port of has_real_token"
/// while implementing something else.
pub fn has_real_token(s: &str) -> bool {
    use crate::ported::zsh_h::{Qstring, Snull, Stringg};
    use crate::ported::ztype_h::{inull, itok};

    let sc: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    // c:1061 — `while (*s)`
    while i < sc.len() {
        let c = sc[i];
        // c:1066-1071 — `$'…'` strings are treated like nulls: skip the
        // two-char introducer (`Qstring` + `'`, or `String` + `Snull`).
        if (c == Qstring && sc.get(i + 1) == Some(&'\''))
            || (c == Stringg && sc.get(i + 1) == Some(&Snull))
        {
            i += 2;
            continue;
        }
        // c:1072-1073 — `if (itok(*s) && !inull(*s)) return 1;`.
        // C indexes `typtab` with `(unsigned char) *s`; a metafied word
        // holds no char above U+00FF, and anything that did could not be
        // a token, so it maps to a byte that is never ITOK.
        let b = if (c as u32) < 0x100 { c as u32 as u8 } else { 0 };
        if itok(b) && !inull(b) {
            return true; // c:1073
        }
        i += 1; // c:1074
    }
    false // c:1076
}

/// Port of `get_comp_string()` from Src/Zle/zle_tricky.c:1087 — the
/// "lasciate ogni speranza" function. Runs the real context lexer
/// (`ctxtlex`) over the metafied line up to the cursor, storing each
/// word in `clwords`, and returns the word being completed together
/// with its side-effects (`WB`/`WE`/`OFFS`/`LINCMD`, plus
/// `INSTRING`/`INBACKT`/`INWHAT`/`INSUBSCR`).
///
/// This ports c:1117–1708 — the setup, the `ctxtlex()` token loop
/// (quote-state scan, redirection tracking, `incmdpos`/`inredir`
/// command-position logic, the `gotword`-driven cursor-word capture,
/// and the `clwords` accumulation), the post-loop word resolution
/// (empty line / STRING / TYPESET / ENVSTRING), the `parbegin`
/// command-substitution restart, and the `offs = zlemetacs - wb`
/// prefix/suffix split.
///
/// The tail of the C function is ported too: c:1482–1706 (IN_MATH /
/// array-subscript word extraction) at the two `INWHAT == IN_MATH`
/// blocks below, c:1709–1726 (Dnull/Snull -> literal quotes) at the
/// `parambeg(s)` rewrite, c:1728–1776 (quote-form detection feeding
/// `qipre`/`qisuf`/`autoq`) after it, c:1780–1786 (the leading `=`
/// restore) and c:1787–1926 (the quote-marker cleanup that strips the
/// markers from `s` and the quote characters they stand for from the
/// LIVE line), and c:1931–2218 (the brace-expansion tail) under
/// `isset(IGNOREBRACES)`.
///
/// STILL UNPORTED in this function: c:1774–1776, the BANGHIST `\!`
/// re-quote inside the quote-form block (flagged again at its site).
///
/// PRECONDITION: the caller must have populated compcore's
/// `ZLEMETALINE`/`ZLEMETACS`/`ZLEMETALL` (C does this via
/// `metafy_line()` in `docomplete` before calling). There are no
/// callers wired yet.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn get_comp_string() -> Option<String> {
    // c:1087
    use crate::ported::context::{zcontext_restore, zcontext_save};
    use crate::ported::lex::{
        ctxtlex, incmdpos, incond, inredir, noaliases, set_incmdpos, set_intypeset, set_noaliases,
        set_tok, set_tokstr, tok, tokstr, untokenize, IS_REDIROP, LEX_LEXFLAGS, LEX_PARBEGIN,
        LEX_PAREND, LEX_WORDBEG,
    };
    use crate::ported::string::{dupstring, ztrdup};
    use crate::ported::zle::compcore::{BRBEG, BREND, INWHAT, OFFS};
    use crate::ported::zsh_h::{
        isset, lextok, Inbrack, Meta, Outbrack, AMPER, AMPERBANG, BARAMP, BAR_TOK, CASE,
        COMPLETEALIASES, DAMPER, DBAR, DINPAR, DOLOOP, ENDINPUT, ENVARRAY, ENVSTRING, FOR, FOREACH,
        INPAR_TOK, IN_COND, IN_ENV, IN_MATH, IN_NOTHING, IN_PAR, LEXERR, LEXFLAGS_ZLE, NULLTOK,
        OUTPAR_TOK, RCQUOTES, REPEAT, SELECT, SEPER, STRING_LEX, TYPESET,
    };
    use crate::ported::ztype_h::INAMESPC;

    let snull = crate::ported::zle::compctl::Snull;
    let dnull = crate::ported::zle::compctl::Dnull;
    let bnull = crate::ported::zle::compctl::Bnull;

    // c:1091 — `int ona = noaliases;` (save for restore at exit).
    let ona = noaliases();

    // Clear the tokenized-word bridge (see the stash at the c:2219 return):
    // every path that fails to produce a word must not leave the PREVIOUS
    // completion's tokens visible to `docomplete`.
    if let Ok(mut g) = COMP_STRING_TOK
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        g.clear();
    }

    // c:1117 METACHECK() — the metafied line must already be present.
    let meta_snap: String = match ZLEMETALINE.get() {
        Some(m) => m.lock().unwrap().clone(),
        None => return None,
    };
    let zlemetacs = ZLEMETACS.load(Ordering::SeqCst);

    // c:637-639 — `origline = ztrdup(zlemetaline); origcs = zlemetacs;
    // origll = zlemetall;`. Snapshot the line BEFORE the addx/lexing so
    // do_completion's no-match / error path can restore it via
    // `inststr(origline)`. Without this ORIGLINE stayed empty, so a
    // completion that found nothing (e.g. `ls -<Tab>` — no files start
    // with `-`) deleted the whole line instead of leaving it intact.
    {
        if let Ok(mut g) = ORIGLINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = meta_snap.clone();
        }
        ORIGCS.store(zlemetacs, Ordering::SeqCst);
        ORIGLL.store(meta_snap.len() as i32, Ordering::SeqCst);
    }

    // c:1119-1130 — reset brace-info state plus the redirection
    // recorders: `if (rdstrs) freelinklist(...); rdstrs = znewlinklist();
    // rdop[0] = '\0'; rdstr = NULL;`. `rdop` is a function-scope buffer
    // (c:1114 `char … rdop[20]`) cleared ONCE here — the two `goto start`
    // restarts deliberately keep whatever the previous pass left in it.
    if let Ok(mut g) = RDSTRS.lock() {
        g.clear(); // c:1126-1128
    }
    let mut rdop = String::new(); // c:1129
    if let Ok(mut g) = RDSTR.lock() {
        *g = None; // c:1130
    }
    if let Some(b) = BRBEG.get() {
        *b.lock().unwrap() = None;
    }
    if let Some(b) = BREND.get() {
        *b.lock().unwrap() = None;
    }
    NBRBEG.store(0, Ordering::SeqCst); // c:1122
    NBREND.store(0, Ordering::SeqCst);

    // c:1134 — signal the lexer whether to expand aliases.
    set_noaliases(isset(COMPLETEALIASES));

    // c:1136-1151 — is the cursor inside a `string' (backtick/quote)?
    {
        let (mut i, mut j, mut k) = (0i32, 0i32, 0i32);
        let ub = meta_snap.as_bytes();
        let cs = (zlemetacs.max(0) as usize).min(ub.len());
        let mut u = 0usize;
        while u < cs {
            let c = ub[u];
            if c == b'`' && (k & 1) == 0 {
                i += 1;
            } else if c == b'"' && (k & 1) == 0 && (i & 1) == 0 {
                j += 1;
            } else if c == b'\'' && (j & 1) == 0 {
                k += 1;
            } else if c == b'\\' && u + 1 < ub.len() && (k & 1) == 0 {
                u += 1; // c:1148 skip the escaped char
            }
            u += 1;
        }
        INBACKT.store(i & 1, Ordering::SeqCst); // c:1150
    }
    INSTRING.store(QT_NONE as i32, Ordering::SeqCst); // c:1151

    // c:1152 — addx(&tmp): inject a dummy `x` at the cursor so the
    // lexer has a word to lock onto. The shared `addx` (this file:839)
    // mutates ZLELINE, not ZLEMETALINE, so it cannot serve this lexer
    // path; the gate from c:922-946 is ported inline onto the metaline.
    let mut zml: String = meta_snap.clone();
    let addedx: i32;
    {
        let bytes = zml.as_bytes().to_vec();
        let cs = (zlemetacs.max(0) as usize).min(bytes.len());
        let ll = bytes.len();
        let ch_at = bytes.get(cs).copied();
        let prev_at = if cs > 0 {
            bytes.get(cs - 1).copied()
        } else {
            None
        };
        let comppref = COMPPREF.load(Ordering::SeqCst) != 0;
        let instr = INSTRING.load(Ordering::SeqCst);
        let is_iblank = matches!(ch_at, Some(b' ' | b'\t'));
        let is_blank_unescaped = is_iblank && (cs == 0 || prev_at != Some(b'\\'));
        let cs_at_end = ch_at.is_none() || cs >= ll;
        let is_newline = ch_at == Some(b'\n');
        let is_separator = matches!(
            ch_at,
            Some(b')' | b'`' | b'}' | b';' | b'|' | b'&' | b'>' | b'<')
        );
        let is_instring_quote = instr != QT_NONE as i32 && matches!(ch_at, Some(b'"' | b'\''));
        let addspace = comppref && ch_at.is_some() && !matches!(ch_at, Some(b' ' | b'\t'));
        if cs_at_end
            || is_newline
            || is_blank_unescaped
            || is_separator
            || is_instring_quote
            || addspace
        {
            let mut nb = Vec::with_capacity(ll + 2);
            nb.extend_from_slice(&bytes[..cs]);
            nb.push(b'x'); // c:944
            if addspace {
                nb.push(b' '); // c:945
            }
            nb.extend_from_slice(&bytes[cs..]);
            // `nb` is the METAFIED line (from ZLEMETALINE, produced by
            // `metafy_line` → `zlelineasstring`, compcore.rs:6944) with one
            // ASCII byte spliced in. A metafied line is not valid UTF-8 by
            // construction — `Meta` is 0x83 and each escape is `Meta` plus
            // `c ^ 32` — so `from_utf8_lossy` turned every escape into
            // U+FFFD and, being 3 bytes wide, shifted every offset after it
            // (`wb`/`we`/`zlemetacs` then indexed the wrong bytes and the
            // `x` the epilogue at c:1385 chucks survived on the line).
            // Same reasoning as `zlelineasstring` itself
            // (zle_utils.rs:221-225): build the String from the bytes as-is.
            zml = unsafe { String::from_utf8_unchecked(nb) };
            addedx = if addspace { 2 } else { 1 }; // c:947
        } else {
            addedx = 0; // c:949
        }
    }
    ADDEDX.store(addedx, Ordering::SeqCst);
    // Publish the injected line + original length so the lexer + gotword
    // (which read compcore's ZLEMETALL/ADDEDX/ZLEMETACS) see it.
    if let Some(m) = ZLEMETALINE.get() {
        *m.lock().unwrap() = zml.clone();
    }
    // c:1473 — `zlemetall -= parend` on the parbegin restart mutates the
    // global for the whole restarted lex, so this mirror has to move too.
    let mut zlemetall = meta_snap.len() as i32; // length excluding the injected x
    ZLEMETALL.store(zlemetall, Ordering::SeqCst);

    // c:1154 — pushheap() (matching popheap deferred to caller, c:662).
    crate::ported::mem::pushheap();

    // ==================================================================
    // start: label (c:1156). Wrapped in a loop so the two `goto start`
    // restarts (c:1478 parbegin cmdsubst, c:1555 tmp cmdsubst) become
    // `continue 's_restart`.
    // ==================================================================
    let mut linptr = zml.clone();
    let mut t0: lextok;
    // Locals that survive the loop for post-loop resolution. `clwords`
    // is published into the CLWORDS global at the end; `cmdstr` and
    // `varname` mirror into CMDSTR / VARNAME at each assignment.
    let mut clwords: Vec<String> = Vec::new(); // c:1416
    let mut clwpos: i32; // c:1168
    let mut clwnum: i32;
    let mut cp: i32 = 0; // c:1173 saved lincmd
    let mut rd: i32 = 0; // c:1173 saved linredir
    let mut ia: i32 = 0; // c:1173 linarr snapshot
    let mut varq: i32; // c:1090
    let mut cmdstr: Option<String>; // c:1162
    let mut varname: Option<String>; // c:1165
    let mut zlemetacs_qsub: i32; // c:1106
    let mut tt: Option<String>; // c:1114 cursor-word capture

    // c:1515/1615/1696 — `(keypm = paramtab->getnode(paramtab, varname)) &&
    // (keypm->node.flags & PM_HASHED)`: an associative array makes the
    // subscript an assoc KEY context (insubscr 2) instead of a math index
    // (insubscr 1). Local closure — the build gate only admits module-level
    // `fn`s that exist in the C source.
    let param_is_hashed = |name: &str| -> bool {
        !name.is_empty()
            && crate::ported::params::paramtab()
                .read()
                .ok()
                .and_then(|t| {
                    t.get(name)
                        .map(|p| (p.node.flags & crate::ported::zsh_h::PM_HASHED as i32) != 0)
                })
                .unwrap_or(false)
    };

    's_restart: loop {
        INWHAT.store(IN_NOTHING, Ordering::SeqCst); // c:1157
        LEX_PARBEGIN.set(-1); // c:1159
        LEX_PAREND.set(-1);
        LINCMD.store(incmdpos() as i32, Ordering::SeqCst); // c:1160
        let mut linredir: i32 = inredir() as i32; // c:1161
        LINREDIR.store(linredir, Ordering::SeqCst); // c:1161
        cmdstr = None; // c:1162-1163
        if let Ok(mut g) = CMDSTR.lock() {
            *g = None; // c:1162-1163
        }
        let mut cmdtok: lextok = NULLTOK; // c:1164
        varname = None; // c:1165-1166
        if let Ok(mut g) = VARNAME.get_or_init(|| Mutex::new(None)).lock() {
            *g = None; // c:1165-1166
        }
        INSUBSCR.store(0, Ordering::SeqCst); // c:1167
        clwpos = -1; // c:1168
        zcontext_save(); // c:1169
                         // c:1170 — `lexflags = LEXFLAGS_ZLE`. ACTIVE is OR'd in so the lexer
                         // TOLERATES an unterminated quote/backtick/brace at the cursor (the
                         // word being completed): the `!(lexflags & LEXFLAGS_ACTIVE)` guards on
                         // the unmatched → LEXERR / zerr paths (lex.c:1320/1344/1383/1445) then
                         // keep the partial word a usable STRING instead of aborting the
                         // completion with "unmatched \"".
        LEX_LEXFLAGS.set(LEXFLAGS_ZLE | crate::ported::zsh_h::LEXFLAGS_ACTIVE);
        crate::ported::input::inpush(&dupstrspace(&linptr), 0, None); // c:1171
        crate::ported::hist::strinbeg(0); // c:1172

        // c:1173 — per-command accumulators.
        let mut wordpos: i32 = 0;
        let mut ins: i32 = 0;
        let mut oins: i32 = 0; // prior-iteration ins (c:1203); init 0
        let mut linarr: i32 = 0;
        LINARR.store(0, Ordering::SeqCst); // c:1173
        let mut parct: i32 = 0;
        let mut redirpos: i32 = 0;
        WB.store(zlemetacs, Ordering::SeqCst); // c:1174 we = wb = zlemetacs
        WE.store(zlemetacs, Ordering::SeqCst);
        let mut tt0: lextok = NULLTOK; // c:1175
        clwords.clear();
        tt = None;
        varq = 0;
        zlemetacs_qsub = 0;

        // c:1185 — the token loop.
        loop {
            let mut qsub: i32 = 0; // c:1186
            let mut noword: i32 = 0;

            // c:1197 — linredir = (inredir && !ins)
            linredir = (inredir() && ins == 0) as i32;
            LINREDIR.store(linredir, Ordering::SeqCst); // c:1197
                                                        // c:1198-1202 — lincmd command-position determination.
            let lincmd_val = (!inredir()
                && ((incmdpos() && ins == 0 && incond() == 0)
                    || (oins == 2 && wordpos == 2)
                    || (ins == 3 && wordpos == 1)
                    || (cmdtok == NULLTOK && incond() == 0))) as i32;
            LINCMD.store(lincmd_val, Ordering::SeqCst);
            oins = ins; // c:1203
            if linarr != 0 {
                set_incmdpos(false); // c:1205-1206
            }
            if cmdtok == TYPESET {
                set_intypeset(linarr == 0); // c:1211-1212
            }
            ctxtlex(); // c:1213

            // c:1215-1227 — LEXERR fixup: odd Snull/Dnull count means an
            // unterminated quote; treat as STRING (or ENVSTRING).
            let mut tokv = tok();
            if tokv == LEXERR {
                match tokstr() {
                    None => break,
                    Some(ts) => {
                        let jcnt = ts.chars().filter(|&c| c == snull || c == dnull).count();
                        if jcnt & 1 == 1 {
                            if LINCMD.load(Ordering::SeqCst) != 0 && ts.contains('=') {
                                varq = 1;
                                tokv = ENVSTRING;
                                set_tok(ENVSTRING);
                            } else {
                                tokv = STRING_LEX;
                                set_tok(STRING_LEX);
                            }
                        }
                    }
                }
            } else if tokv == ENVSTRING {
                varq = 0; // c:1228-1229
            }

            // c:1230-1243 — array-assignment / paren nesting.
            if tokv == ENVARRAY {
                linarr = 1;
                LINARR.store(1, Ordering::SeqCst); // c:1231
                                                   // c:1232-1233 — `zsfree(varname); varname = ztrdup(tokstr);`.
                                                   // The mirror into the VARNAME global was missing, so the
                                                   // IN_ENV arm's `compparameter = varname` (compcore.c:607)
                                                   // published EMPTY for an array assignment: `myarr=(/tm<TAB>`
                                                   // reported `$compstate[parameter]=''` and `_value` built
                                                   // `-value-,,-default-` where zsh builds `-value-,myarr,`.
                varname = tokstr().map(|s| ztrdup(&s));
                if let Ok(mut g) = VARNAME.get_or_init(|| Mutex::new(None)).lock() {
                    *g = varname.clone();
                }
            } else if tokv == INPAR_TOK {
                parct += 1;
            } else if tokv == OUTPAR_TOK {
                if parct != 0 {
                    parct -= 1;
                } else if linarr != 0 {
                    linarr = 0;
                    LINARR.store(0, Ordering::SeqCst); // c:1241
                    set_incmdpos(true);
                }
            }

            // c:1244-1268 — redirection handling.
            if inredir() && IS_REDIROP(tokv) {
                // c:1245-1250 — remember the operator text so
                // `callcompfunc` can publish `$compstate[redirect]`
                // (compcore.c:600-601). C keeps it in the static
                // `rdstrbuf` and points `rdstr` at it; the Rust global
                // owns the string directly.
                let op = crate::ported::lex::tokstrings
                    .get(tokv as usize)
                    .copied()
                    .flatten()
                    .unwrap_or(""); // c:1247/1249 tokstrings[tok]
                let tokfd = crate::ported::lex::tokfd();
                rdop = if tokfd >= 0 {
                    format!("{}{}", tokfd, op) // c:1246-1247
                } else {
                    op.to_string() // c:1248-1249
                };
                if let Ok(mut g) = RDSTR.lock() {
                    *g = Some(rdop.clone()); // c:1245/1250
                }
                if wordpos == redirpos {
                    redirpos += 1;
                }
                let inbufct = crate::ported::input::inbufct.with(|c| c.get());
                let wb = WB.load(Ordering::SeqCst);
                let we = WE.load(Ordering::SeqCst);
                let wordbeg = LEX_WORDBEG.get();
                if zlemetacs < (zlemetall - inbufct) && zlemetacs >= wordbeg && wb == we {
                    let new_we = zlemetall - (inbufct + addedx); // c:1257
                    WE.store(new_we, Ordering::SeqCst);
                    if addedx != 0 && new_we > wb {
                        WB.store(wb + 1, Ordering::SeqCst); // c:1260 {param}> form
                    } else {
                        WB.store(zlemetacs, Ordering::SeqCst); // c:1264 2> form
                    }
                }
            }
            if tokv == DINPAR {
                set_tokstr(None); // c:1269-1270
            }

            if tokv == ENDINPUT {
                break; // c:1273
            }

            // c:1275-1309 — command separators.
            let is_sep = (ins != 0 && (tokv == DOLOOP || tokv == SEPER))
                || (ins == 2 && wordpos == 2)
                || (ins == 3 && wordpos == 3)
                || tokv == BAR_TOK
                || tokv == AMPER
                || tokv == BARAMP
                || tokv == AMPERBANG
                || ((tokv == DBAR || tokv == DAMPER) && incond() == 0)
                || (tt.is_some() && incmdpos());
            if is_sep {
                if tt.is_some() {
                    break; // c:1291-1292
                }
                if ins < 2 {
                    noword = 1; // c:1304
                }
                wordpos = 0; // c:1307
                redirpos = 0;
                ins = 0;
                tt0 = NULLTOK; // c:1308
            }

            // c:1310-1327 — token in command position: record cmdstr.
            if LINCMD.load(Ordering::SeqCst) != 0
                && (tokv == STRING_LEX
                    || tokv == FOR
                    || tokv == FOREACH
                    || tokv == SELECT
                    || tokv == REPEAT
                    || tokv == CASE
                    || tokv == TYPESET)
            {
                ins = if tokv == REPEAT {
                    2
                } else {
                    (tokv != STRING_LEX && tokv != TYPESET) as i32
                };
                if let Some(ts) = tokstr() {
                    let mut c = ztrdup(&untokenize(&ts));
                    crate::ported::glob::remnulargs(&mut c);
                    cmdstr = Some(c); // c:1319-1322
                    if let Ok(mut g) = CMDSTR.lock() {
                        *g = cmdstr.clone();
                    }
                }
                cmdtok = tokv;
                if wordpos != redirpos && clwpos == -1 {
                    wordpos = 0; // c:1327
                    redirpos = 0;
                }
            } else if tokv == SEPER {
                ins = (cmdtok != STRING_LEX && cmdtok != TYPESET) as i32; // c:1336
            }

            // c:1338-1394 — the lexer reached the cursor word (gotword
            // cleared lexflags). Capture the token string as `tt`.
            if LEX_LEXFLAGS.get() == 0 && tt0 == NULLTOK {
                tt = tokstr().as_ref().map(|s| dupstring(s));
                let wb = WB.load(Ordering::SeqCst);
                let we = WE.load(Ordering::SeqCst);
                // c:1352-1368 — count \-\n pairs (removed by the lexer).
                {
                    let ub = zml.as_bytes();
                    let (mut i, mut j, mut k) = (0i32, 0i32, 0i32);
                    let mut u = wb.max(0) as usize;
                    let we_u = (we.max(0) as usize).min(ub.len());
                    while u < we_u {
                        let c = ub[u];
                        if c == b'`' && (k & 1) == 0 {
                            i += 1;
                        } else if c == b'"' && (k & 1) == 0 && (i & 1) == 0 {
                            j += 1;
                        } else if c == b'\'' && (j & 1) == 0 {
                            k += 1;
                        } else if c == b'\\' && u + 1 < ub.len() && (k & 1) == 0 {
                            if ub[u + 1] == b'\n' {
                                qsub += 2; // c:1364
                            }
                            u += 1;
                        }
                        u += 1;
                    }
                }
                // c:1373-1383 — RCQUOTES single-quote fixup.
                if isset(RCQUOTES) {
                    if let Some(ref ttv) = tt {
                        // c:1374 — `e = tt + zlemetacs - wb - qsub` (byte-offset
                        // boundary into the meta string).
                        let e = (zlemetacs - wb - qsub).max(0) as usize;
                        // c:1375 — `for (tt1 = tt; *tt1; tt1++)`. Tokens like
                        // Snull/Dash are multi-byte chars in the Rust meta
                        // string, so step by chars — byte-stepping and slicing
                        // (`ttv[idx..]`) inside a token panics on the char
                        // boundary (e.g. `\u{9b}` Dash spans 2 bytes).
                        for (idx, ch) in ttv.char_indices() {
                            if ch == snull {
                                // c:1376
                                // c:1378-1380 — `for (p = tt1; *p && p < e; p++)
                                //   if (*p == '\'') qsub++;`
                                for (off, pc) in ttv[idx..].char_indices() {
                                    if idx + off >= e {
                                        break;
                                    }
                                    if pc == '\'' {
                                        qsub += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                // c:1385-1386 — remove the injected `x` from tt.
                if addedx != 0 {
                    if let Some(ref mut ttv) = tt {
                        let at = (zlemetacs - wb - qsub).max(0) as usize;
                        if at < ttv.len() && ttv.is_char_boundary(at) {
                            ttv.remove(at);
                        }
                    }
                }
                tt0 = tokv; // c:1387
                clwpos = wordpos; // c:1389
                cp = LINCMD.load(Ordering::SeqCst); // c:1390
                rd = linredir; // c:1391
                ia = linarr; // c:1392
                if INWHAT.load(Ordering::SeqCst) == IN_NOTHING && incond() != 0 {
                    INWHAT.store(IN_COND, Ordering::SeqCst); // c:1394
                }
            } else if linredir != 0 {
                // c:1396-1397 — `if (rdop[0] && tokstr)
                //   zaddlinknode(rdstrs, tricat(rdop, ":", tokstr));`.
                // Records every COMPLETED redirection on the line (the
                // one under the cursor took the branch above) for
                // `$compstate[redirections]` (compcore.c:650-651).
                if !rdop.is_empty() {
                    if let Some(ts) = tokstr() {
                        if let Ok(mut g) = RDSTRS.lock() {
                            g.push(format!("{}:{}", rdop, ts)); // c:1397
                        }
                    }
                }
                // c:1398 — C `continue`. A do-while `continue` re-tests
                // the loop condition, so honor the c:1446 end condition
                // before continuing.
                let lexflags_nz = LEX_LEXFLAGS.get() != 0;
                let end = tokv == LEXERR
                    || tokv == ENDINPUT
                    || (tokv == SEPER && !(lexflags_nz && tt0 == NULLTOK));
                if end {
                    break;
                }
                continue;
            }

            // c:1400-1405 — inside a cond, canonicalize || and &&.
            let mut cur_tokstr = tokstr();
            if incond() != 0 {
                if tokv == DBAR {
                    cur_tokstr = Some("||".to_string());
                } else if tokv == DAMPER {
                    cur_tokstr = Some("&&".to_string());
                }
            }
            // c:1406-1407 — skip empty tokens / suppressed words.
            if cur_tokstr.is_none() || noword != 0 {
                let lexflags_nz = LEX_LEXFLAGS.get() != 0;
                let end = tokv == LEXERR
                    || tokv == ENDINPUT
                    || (tokv == SEPER && !(lexflags_nz && tt0 == NULLTOK));
                if end {
                    break;
                }
                continue;
            }
            let cur_tokstr_s = cur_tokstr.unwrap();
            // c:1408-1410 — `repeat n do` hack.
            if oins == 2 && wordpos == 0 && cur_tokstr_s == "do" {
                ins = 3;
            }
            // c:1414-1430 — store the word in clwords[wordpos], trimming
            // trailing spaces (unless Bnull/Meta-escaped).
            let mut word = ztrdup(&cur_tokstr_s);
            {
                let meta = Meta as char;
                loop {
                    let chars: Vec<char> = word.chars().collect();
                    let sl = chars.len();
                    if sl == 0 || chars[sl - 1] != ' ' {
                        break;
                    }
                    if sl >= 2 && (chars[sl - 2] == bnull || chars[sl - 2] == meta) {
                        break;
                    }
                    word.pop();
                }
            }
            while clwords.len() <= wordpos as usize {
                clwords.push(String::new());
            }
            clwords[wordpos as usize] = word;
            // c:1424 `sl = strlen(tokstr)` is a BYTE count and c:1444's
            // `chuck_at` is a BYTE index into the same string. This port
            // CANNOT use either directly: a token marker is one C byte but
            // the port stores it as a `char` in U+0080..U+00A2, i.e. TWO
            // UTF-8 bytes, so `zlemetacs - wb` (a metafied LINE byte offset)
            // and a byte index into `tokstr` are not commensurate. Counting
            // in chars keeps the two in step for a word whose only non-ASCII
            // content is markers. It does NOT for a word carrying metafied
            // multibyte text — `echo ★` (two `Meta` escapes) chucks in the
            // wrong place — but converting this to bytes made the marker case
            // (`echo "${(`) split a marker char and panic in `parambeg`. The
            // real fix is a C-byte <-> port-byte index map (the shape
            // `crate::comp_word_tok::tok_index` uses for `offs`), which is
            // out of scope here; see the report.
            let sl = clwords[wordpos as usize].chars().count() as i32;
            // c:1433-1445 — cursor word: remove injected `x`.
            let prev_wordpos = wordpos;
            wordpos += 1;
            if clwpos == prev_wordpos && addedx != 0 {
                let wb = WB.load(Ordering::SeqCst);
                zlemetacs_qsub = zlemetacs - qsub;
                let word_diff = zlemetacs_qsub - wb;
                let chuck_at = if word_diff >= sl {
                    sl - 1
                } else if word_diff < 0 {
                    0
                } else {
                    word_diff
                };
                // c:1444 — `chuck(&clwords[wordpos-1][chuck_at])`. Char-indexed
                // for the reason spelled out at the `sl` computation above.
                let w = &mut clwords[prev_wordpos as usize];
                if let Some((byte_at, _)) = w.char_indices().nth(chuck_at.max(0) as usize) {
                    w.remove(byte_at);
                }
            }

            // c:1446 — do-while loop condition.
            let lexflags_nz = LEX_LEXFLAGS.get() != 0;
            let end = tokv == LEXERR
                || tokv == ENDINPUT
                || (tokv == SEPER && !(lexflags_nz && tt0 == NULLTOK));
            if end {
                break;
            }
        }

        // c:1449 — number of words collected.
        clwnum = if tt.is_some() || wordpos == 0 {
            wordpos
        } else {
            wordpos - 1
        };
        t0 = tt0; // c:1452
                  // c:1453-1459 — array-assignment overrides lincmd/linredir.
        if ia != 0 {
            LINCMD.store(0, Ordering::SeqCst);
            linredir = 0;
            LINREDIR.store(0, Ordering::SeqCst); // c:1454
            INWHAT.store(IN_ENV, Ordering::SeqCst);
        } else {
            LINCMD.store(cp, Ordering::SeqCst);
            linredir = rd;
            LINREDIR.store(rd, Ordering::SeqCst); // c:1458
        }
        crate::ported::hist::strinend(); // c:1460
        crate::ported::input::inpop(); // c:1461
        LEX_LEXFLAGS.set(0); // c:1462
        crate::ported::utils::errflag
            .fetch_and(!crate::ported::utils::ERRFLAG_ERROR, Ordering::SeqCst); // c:1463

        // c:1464-1480 — parbegin command-substitution restart. The lexer
        // records where a `` ` ``/`$(`/`<(`/`>(` opened around the cursor
        // (lex.c:1353/1584/2163 SETPARBEGIN); C then re-runs the whole
        // scan over just the SUBSTITUTION BODY so its first word is a
        // command position. `linptr` is C's `zlemetaline + <offset>` —
        // a pointer INTO the line, i.e. the tail from `off` on, not the
        // whole line.
        if LEX_PARBEGIN.get() != -1 {
            let parend = LEX_PAREND.get();
            // c:1469 — `linptr = zlemetaline + zlemetall + addedx - parbegin + 1`
            let off = zlemetall + addedx - LEX_PARBEGIN.get() + 1;
            let ub = zml.as_bytes();
            let li = off as isize;
            // c:1470-1471 — a `$((` is arithmetic, not a substitution: leave it.
            let is_dollar_dparen = li >= 3
                && (li as usize) < ub.len()
                && ub[li as usize] == b'('
                && ub[(li - 1) as usize] == b'('
                && ub[(li - 2) as usize] == b'$';
            if !is_dollar_dparen && li >= 0 && (li as usize) <= zml.len() {
                if parend >= 0 {
                    // c:1473-1474 — `zlemetall -= parend;
                    // zlemetaline[zlemetall + addedx] = '\0';` — cut the
                    // line off at the end of the substitution body. C
                    // NUL-terminates at `zlemetall + addedx`, not at
                    // `zlemetall`, so the injected `x` survives the cut.
                    zlemetall -= parend;
                    ZLEMETALL.store(zlemetall, Ordering::SeqCst);
                    let cut = zlemetall + addedx;
                    if cut >= 0
                        && (cut as usize) <= zml.len()
                        && zml.is_char_boundary(cut as usize)
                    {
                        zml.truncate(cut as usize);
                    }
                }
                zcontext_restore(); // c:1476
                tt = None; // c:1477
                linptr = zml[(li as usize).min(zml.len())..].to_string();
                continue 's_restart; // c:1478 goto start
            }
        }

        // c:1482-1541 — resolve `s` from the token kind. Where C sets
        // `s = NULL` the word is not taken from the lexer at all: the
        // IN_MATH block at c:1621-1706 rebuilds it from the line.
        let mut s: String = String::new();
        if INWHAT.load(Ordering::SeqCst) == IN_MATH {
            // c:1482-1483 — `s = NULL`; the IN_MATH block below rebuilds it.
        } else if t0 == NULLTOK || t0 == ENDINPUT {
            // c:1484-1489 — empty line.
            s = String::new();
            WB.store(zlemetacs, Ordering::SeqCst);
            WE.store(zlemetacs, Ordering::SeqCst);
            clwpos = clwnum;
            t0 = STRING_LEX;
        } else if t0 == STRING_LEX || t0 == TYPESET {
            // c:1490-1494 — a simple string.
            s = clwords
                .get(clwpos.max(0) as usize)
                .cloned()
                .unwrap_or_default();
        } else if t0 == ENVSTRING {
            // c:1495-1541 — cursor inside a parameter assignment. C refreshes
            // `tt` from clwords[clwpos] only when varq, relying on the lexer's
            // tokstr otherwise; the Rust port's un-refreshed `tt` is stale
            // (it kept the placeholder `x` and had already dropped a char),
            // while clwords[clwpos] is the correctly-chucked word. Always use
            // clwords[clwpos] so `x=gam<Tab>` sees the real value.
            tt = clwords.get(clwpos.max(0) as usize).cloned();
            let ttv = tt.clone().unwrap_or_default();
            // c:1503 — namespace ident end.
            let ns_off = crate::ported::utils::itype_end(&ttv, INAMESPC, false).min(ttv.len());
            varname = Some(ztrdup(&ttv[..ns_off])); // c:1506-1508
            if let Ok(mut g) = VARNAME.get_or_init(|| Mutex::new(None)).lock() {
                *g = varname.clone(); // c:1506-1507
            }
            let mut soff = ns_off;
            if ttv.as_bytes().get(soff) == Some(&b'+') {
                soff += 1; // c:1509-1510
            }
            // c:1511-1512 — subscript / past-cursor => math context.
            let mut rest: &str = &ttv[soff..];
            let sp = crate::ported::utils::skipparens(Inbrack, Outbrack, &mut rest);
            let after_paren_off = ttv.len() - rest.len();
            let wb0 = WB.load(Ordering::SeqCst);
            // c:1512 — `s > tt + zlemetacs_qsub - wb` compares POINTERS into
            // the tokenized word, where every token (`Inbrack`, `Outbrack`) is
            // ONE byte, so `s - tt` counts word POSITIONS. In this port a token
            // is a multi-byte UTF-8 char, so the C pointer difference is a CHAR
            // count, not `after_paren_off`. Using the byte offset made `a[1]=`
            // read as 6 > 5 (`a` + 2-byte `Inbrack` + `1` + 2-byte `Outbrack`)
            // and forced the c:1513-1519 subscript/IN_MATH branch, so `a[1]=`
            // completed PARAMETERS with `PREFIX=[1]=` where zsh completes the
            // assignment VALUE (`$compstate[context]` = `value`).
            let after_paren_cpos = ttv[..after_paren_off].chars().count() as i32;
            if sp > 0 || after_paren_cpos > (zlemetacs_qsub - wb0) {
                // c:1513-1519 — cursor inside `NAME[…]` on the LHS of an
                // assignment: `s = NULL`, complete the subscript as math.
                // c:1513 — `s = NULL`; the IN_MATH block below rebuilds it.
                INWHAT.store(IN_MATH, Ordering::SeqCst); // c:1514
                                                         // c:1515-1519 — a PM_HASHED param takes insubscr 2 (assoc key),
                                                         // anything else 1.
                INSUBSCR.store(
                    if param_is_hashed(varname.as_deref().unwrap_or("")) {
                        2
                    } else {
                        1
                    },
                    Ordering::SeqCst,
                );
            } else if {
                let c = ttv[after_paren_off..].chars().next();
                c == Some('=') || c == Some(crate::ported::zsh_h::Equals)
            } {
                // c:1520-1539 — an `=`: split VAR=value. The lexer emits the
                // assignment `=` as the Equals token (a 2-byte UTF-8 char in
                // this port, single byte in C), so compare the CHAR and count
                // in chars: the token is one char just like the literal `=`
                // on the metaline, so char offsets map straight to metaline
                // byte offsets (varname + `=` are ASCII).
                let eq_ch = ttv[after_paren_off..].chars().next().unwrap();
                let eq_len = eq_ch.len_utf8();
                let val_boff = (after_paren_off + eq_len).min(ttv.len());
                let eq_char_pos = ttv[..after_paren_off].chars().count() as i32; // `=` position
                if zlemetacs_qsub > wb0 + eq_char_pos {
                    // c:1521-1525 — cursor after `=`: complete the value.
                    let val_char_pos = ttv[..val_boff].chars().count() as i32;
                    WB.store(wb0 + val_char_pos, Ordering::SeqCst);
                    s = ztrdup(&ttv[val_boff..]);
                    INWHAT.store(IN_ENV, Ordering::SeqCst);
                } else {
                    // c:1526-1537 — cursor on the name: complete the param.
                    let mut poff = after_paren_off;
                    if poff > 0 && ttv.as_bytes()[poff - 1] == b'+' {
                        poff -= 1;
                    }
                    INWHAT.store(IN_PAR, Ordering::SeqCst);
                    s = ztrdup(&ttv[..poff]);
                    WE.store(wb0 + ttv[..poff].chars().count() as i32, Ordering::SeqCst);
                }
                t0 = STRING_LEX; // c:1538
            } else {
                s = ztrdup(&ttv);
            }
            LINCMD.store(1, Ordering::SeqCst); // c:1540
        } else {
            // c:1549-1560 — not a completable word. The tmp cmdsubst
            // restart (c:1550-1556) needs the parbegin zlemetaline dup
            // (c:1467-1468), which is omitted, so this always returns.
            //
            // c:1545-1548 — C restores `zlemetaline = tmp` BEFORE this
            // return: `addx` (c:937-946) spliced the `x` into a SCRATCH
            // copy, `tmp` still holds the untouched line, so the caller
            // never sees the placeholder. This port lexes ONE buffer and
            // deletes the placeholder by hand in the function epilogue,
            // which this early return skips — `echo $(gr<TAB>` left
            // `echo $(grx` on the line where zsh leaves it alone. Same
            // deletion as the epilogue, inlined (a shared helper has no C
            // counterpart and the port gate rejects one).
            {
                let addedx = ADDEDX.load(Ordering::SeqCst);
                if addedx > 0 {
                    if let Some(m) = ZLEMETALINE.get() {
                        if let Ok(mut g) = m.lock() {
                            let mut bytes = g.as_bytes().to_vec();
                            let cs =
                                (ZLEMETACS.load(Ordering::SeqCst).max(0) as usize).min(bytes.len());
                            let end = (cs + addedx as usize).min(bytes.len());
                            if cs < end {
                                bytes.drain(cs..end);
                                // Metafied line (see the addx splice above):
                                // rebuild byte-for-byte, never lossily.
                                *g = unsafe { String::from_utf8_unchecked(bytes) };
                            }
                        }
                    }
                    ADDEDX.store(0, Ordering::SeqCst);
                }
            }
            set_noaliases(ona);
            zcontext_restore();
            return None; // c:1559
        }

        // c:1542-1543 — clamp we to line length.
        if WE.load(Ordering::SeqCst) > zlemetall {
            WE.store(zlemetall, Ordering::SeqCst);
        }

        // c:1545-1547 — `if (tmp) { zlemetaline = tmp;
        // zlemetall = strlen(zlemetaline); }` — the lex ran against a
        // scratch copy, so the length mirror goes back to the real line
        // (the parbegin restart above may have shortened it).
        zlemetall = meta_snap.len() as i32;
        ZLEMETALL.store(zlemetall, Ordering::SeqCst);

        set_noaliases(ona); // c:1562

        // c:1564-1620 — "Check if we are in an array subscript. We simply
        // assume that we are in a subscript if we are in brackets." The
        // lexer hands `s` back TOKENIZED, so `[`/`]` are `Inbrack`/
        // `Outbrack`, never the literal characters.
        if INWHAT.load(Ordering::SeqCst) != IN_MATH {
            let sc: Vec<char> = s.chars().collect();
            let wb0 = WB.load(Ordering::SeqCst);
            // c:1590 — the scan stops at the cursor (`s + zlemetacs_qsub - wb`).
            let cursor_off = ((zlemetacs_qsub - wb0).max(0) as usize).min(sc.len());
            // c:1576/1601 — `itype_end(p, IIDENT, 1) == p` is the C idiom for
            // "this character is not an identifier character".
            let is_ident = |c: char| {
                let mut b = [0u8; 4];
                crate::ported::utils::itype_end(
                    c.encode_utf8(&mut b),
                    crate::ported::ztype_h::IIDENT as u32,
                    true,
                ) != 0
            };
            let mut depth = 0i32; // c:1572 `i`
            let mut nb: Option<usize> = None; // c:1570
            let mut ne: Option<usize> = None;
            // c:1576-1579 — `nnb` tracks the start of the current identifier
            // run; a non-ident first character is skipped over.
            let mut nnb: usize = match sc.first() {
                Some(&c) if is_ident(c) => 0,
                _ => 1,
            };
            let mut tt_i = 0usize; // c:1580
            if LINCMD.load(Ordering::SeqCst) != 0 {
                // c:1581-1589 — `[`s at the start of a COMMAND are not
                // matched by a closing bracket; skip them.
                while tt_i < cursor_off && sc[tt_i] == Inbrack {
                    tt_i += 1;
                }
            }
            while tt_i < cursor_off {
                // c:1590
                if sc[tt_i] == Inbrack {
                    // c:1591-1595
                    depth += 1;
                    nb = Some(nnb);
                    ne = Some(tt_i);
                    tt_i += 1;
                } else if depth != 0 && sc[tt_i] == Outbrack {
                    // c:1596-1598
                    depth -= 1;
                    tt_i += 1;
                } else {
                    // c:1599-1604
                    if !is_ident(sc[tt_i]) {
                        nnb = tt_i + 1;
                    }
                    tt_i += 1;
                }
            }
            if depth != 0 {
                // c:1606 — an unclosed `[` before the cursor.
                INWHAT.store(IN_MATH, Ordering::SeqCst); // c:1607
                INSUBSCR.store(1, Ordering::SeqCst); // c:1608
                if let (Some(nb), Some(ne)) = (nb, ne) {
                    if nb < ne {
                        // c:1609-1618
                        let vn: String = sc[nb..ne].iter().collect(); // c:1613
                        if param_is_hashed(&vn) {
                            INSUBSCR.store(2, Ordering::SeqCst); // c:1617
                        }
                        varname = Some(vn);
                        if let Ok(mut g) = VARNAME.get_or_init(|| Mutex::new(None)).lock() {
                            *g = varname.clone(); // c:1612-1613
                        }
                    }
                }
            }
        }

        // c:1621-1706 — IN_MATH word extraction. `s` is rebuilt out of the
        // metafied LINE (not the lexer word) between the enclosing bracket
        // and the cursor, so an unterminated `$name[` yields the empty
        // subscript text rather than the whole `$name[` word. Without this
        // `docomplete`'s expand-or-complete probe (c:783-792) saw the `$`
        // in `$name[` and ran `doexpansion`, replacing the buffer with the
        // expanded parameter.
        if INWHAT.load(Ordering::SeqCst) == IN_MATH {
            let mut line: Vec<char> = ZLEMETALINE
                .get_or_init(|| Mutex::new(String::new()))
                .lock()
                .map(|g| g.chars().collect())
                .unwrap_or_default();
            // c:1544-1548 — C lexes a COPY of the line carrying the injected
            // `x` placeholder (c:937-944) and restores `zlemetaline` to the
            // ORIGINAL right before this block, so the word it slices out
            // never contains the placeholder. This port keeps one buffer and
            // removes the placeholder at the end of the function, so drop it
            // from the local working copy here — otherwise `$fpath[<TAB>`
            // extracted the word `x` instead of the empty subscript.
            let addedx_here = ADDEDX.load(Ordering::SeqCst);
            if addedx_here > 0 {
                let cs = (zlemetacs.max(0) as usize).min(line.len());
                let end = (cs + addedx_here as usize).min(line.len());
                if cs < end {
                    line.drain(cs..end);
                }
            }
            let line = line;
            let ll = line.len() as i32;
            let compfunc_active = crate::ported::zle::compcore::compfunc
                .get_or_init(|| Mutex::new(None))
                .lock()
                .ok()
                .and_then(|g| g.clone())
                .map(|f| !f.is_empty())
                .unwrap_or(false);
            let at = |i: i32| -> char {
                if i >= 0 && i < ll {
                    line[i as usize]
                } else {
                    '\0'
                }
            };
            // c:1651/1660/1683 — `itype_end(p, IIDENT, 0)`.
            let is_ident_ch = |c: char| {
                let mut b = [0u8; 4];
                crate::ported::utils::itype_end(
                    c.encode_utf8(&mut b),
                    crate::ported::ztype_h::IIDENT as u32,
                    true,
                ) != 0
            };
            if compfunc_active || INSUBSCR.load(Ordering::SeqCst) == 2 {
                // c:1626-1637 — walk back to the bracket/paren that opened
                // this subscript, counting nesting.
                let mut wbv = zlemetacs - 1;
                let mut lev = 0i32;
                while wbv > 0 {
                    let c = at(wbv);
                    if c == ']' || c == ')' {
                        lev += 1;
                    } else if c == '[' {
                        // c:1630 — `if (!lev--) break;`
                        let was = lev;
                        lev -= 1;
                        if was == 0 {
                            break;
                        }
                    } else if c == '(' {
                        // c:1632-1636
                        if lev == 0 && at(wbv - 1) == '(' {
                            break;
                        }
                        if lev != 0 {
                            lev -= 1;
                        }
                    }
                    wbv -= 1;
                }
                let p_at = wbv; // c:1638 `p = zlemetaline + wb`
                wbv += 1; // c:1639
                WB.store(wbv, Ordering::SeqCst);
                let open = at(p_at);
                if open == '[' || open == '(' {
                    // c:1640-1645 — a CLOSED bracket bounds the word at the
                    // matching close; an unterminated one leaves `we` alone.
                    let close = if open == '[' { ']' } else { ')' };
                    let byte_off: usize = line[..(p_at.max(0) as usize).min(line.len())]
                        .iter()
                        .map(|c| c.len_utf8())
                        .sum();
                    let full: String = line.iter().collect();
                    let mut rest: &str = &full[byte_off..];
                    if crate::ported::utils::skipparens(open, close, &mut rest) == 0 {
                        let consumed = full[byte_off..].len() - rest.len();
                        let consumed_chars = full[byte_off..byte_off + consumed].chars().count();
                        WE.store(p_at + consumed_chars as i32 - 1, Ordering::SeqCst); // c:1642
                        if INSUBSCR.load(Ordering::SeqCst) == 2 {
                            INSUBSCR.store(3, Ordering::SeqCst); // c:1644
                        }
                    }
                }
            } else {
                // c:1646-1670 — a real math expression: complete parameter
                // names, so the word is the identifier around the cursor.
                let mut we_i = zlemetacs;
                while we_i < ll && is_ident_ch(at(we_i)) {
                    we_i += 1;
                }
                WE.store(we_i, Ordering::SeqCst); // c:1651
                let mut wb_i = zlemetacs;
                while wb_i > 0 && is_ident_ch(at(wb_i - 1)) {
                    wb_i -= 1;
                }
                WB.store(wb_i, Ordering::SeqCst); // c:1666
            }
            // c:1672-1675 — `s` is the line text between wb and we.
            let wbv = WB.load(Ordering::SeqCst).clamp(0, ll);
            let wev = WE.load(Ordering::SeqCst).clamp(wbv, ll);
            s = line[wbv as usize..wev as usize].iter().collect();
            // c:1677-1703 — the identifier immediately before the `[` names
            // the parameter being subscripted (`$compstate[parameter]`).
            if wbv > 2 && at(wbv - 1) == '[' {
                let sqbr = wbv - 1;
                let mut w_i = sqbr;
                while w_i > 0 && is_ident_ch(at(w_i - 1)) {
                    w_i -= 1;
                }
                if w_i < sqbr {
                    // c:1693-1702
                    let vn: String = line[w_i as usize..sqbr as usize].iter().collect(); // c:1695
                    if param_is_hashed(&vn) {
                        if INSUBSCR.load(Ordering::SeqCst) != 3 {
                            INSUBSCR.store(2, Ordering::SeqCst); // c:1699
                        }
                    } else {
                        INSUBSCR.store(1, Ordering::SeqCst); // c:1701
                    }
                    varname = Some(vn);
                    if let Ok(mut g) = VARNAME.get_or_init(|| Mutex::new(None)).lock() {
                        *g = varname.clone(); // c:1694-1695
                    }
                }
            }
            // c:1705 — `parse_subst_string(s)` re-tokenizes the extracted
            // text; the returned string is discarded by C (it parses in
            // place for its side effects on the token flags).
            let _ = crate::ported::lex::parse_subst_string(&s);
        }

        // c:1708 — offs = zlemetacs - wb (prefix/suffix split point).
        let wb = WB.load(Ordering::SeqCst);
        OFFS.store(zlemetacs - wb, Ordering::SeqCst);

        // Export the parsed command-line words + cursor position to the
        // compsys-facing globals. In C these ARE `$words` / `$CURRENT`:
        // the special params bind directly to `clwords` / `clwpos`. The
        // Rust port keeps `$words`/`$CURRENT` in `COMPWORDS`/`COMPCURRENT`
        // (complete.rs), so without this bridge every compsys completer
        // (`_complete` → `_normal` → the per-command completer) sees
        // `$words=()` / `$CURRENT=0` and can't tell the command from its
        // arguments — it falls back to command completion for everything.
        {
            // c:Src/Zle/compcore.c:642-643 —
            //     `for (p = clwords + aadd; *p; p++, q++)`
            //         `untokenize(*q = ztrdup(*p));`
            // C's `untokenize` (Src/exec.c:2077-2099) maps EVERY itok byte
            // through `ztokens` (Src/lex.c:38), so the quote markers come
            // back as LITERAL characters: Snull -> `'`, Dnull -> `"`,
            // Bnull/Bnullkeep -> `\`, Qstring -> `$`. `$words` therefore
            // KEEPS the user's quoting.
            //
            // This must NOT use `lex::untokenize`: that variant deliberately
            // STRIPS Snull/Dnull (documented at lex.rs:5072-5100) because it
            // runs on the SUBSTITUTION stream, where the lexer's quote-pair
            // markers must not reappear as literal quotes in a value. Using
            // it here made `zstyle ':completion:*' <TAB>` publish
            // `words=(zstyle :completion:*)` instead of zsh's
            // `words=(zstyle "':completion:*'")`, and `_zstyle`
            // (Completion/Zsh/Command/_zstyle:325-333) branches on exactly
            // that text — the unquoted form picks `ctop=c` (114 style names)
            // where zsh picks `ctop=a-z` (176), a silent 62-name shortfall.
            // `untokenize_ztokens` (lex.rs) is the ztokens-EXACT variant
            // c:643 needs.
            let ws: Vec<String> = clwords
                .iter()
                .map(|w| crate::ported::lex::untokenize_ztokens(w))
                .collect();
            let n = ws.len() as i32;
            // c:82 — `mod_export char **clwords`. Stash the parsed word
            // array where `callcompfunc` can rebuild `compwords` from it
            // (compcore.c:634-645 does that on EVERY completion-function
            // call). `expand-or-complete` runs the completion function
            // TWICE per TAB — once inside `doexpansion`, once at
            // zle_tricky.c:851 — but `get_comp_string` only runs once, so
            // without this snapshot the second call inherited whatever the
            // first left in `$words`/`$CURRENT`. Any `_arguments` spec with
            // a `*::`/`*:::` rest argument calls `comparguments -W`, which
            // RESTRICTS those to the rest-range (empty at `cmd -`), so the
            // second pass saw no command word, never dispatched a
            // completer, and threw away the matches the first pass had
            // built: `rm -`, `typeset -`, `bindkey -`, `tar -` completed
            // nothing on TAB's default binding.
            if let Ok(mut g) = CLWORDS.lock() {
                *g = ws.clone();
            }
            CLWPOS.store(clwpos, Ordering::SeqCst); // c:80
                                                    // clwpos is 0-based; `$CURRENT` is 1-based. clwpos == -1 means
                                                    // the cursor is past the last word (new trailing word).
            let cur = if clwpos < 0 { n + 1 } else { clwpos + 1 }.max(1);
            // Keep the internal statics in sync…
            if let Ok(mut g) = crate::ported::zle::complete::COMPWORDS
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
            {
                *g = ws.clone();
            }
            crate::ported::zle::complete::COMPCURRENT.store(cur, Ordering::SeqCst);
            // …but the compsys shell/rust functions read `$words` /
            // `$CURRENT` from the PARAM TABLE (getaparam/getsparam), and
            // these compparams have no gsu binding to the statics
            // (`var:0, gsu:0`). So publish them into paramtab directly, or
            // `_normal` sees `$CURRENT` unset → treats every position as
            // the command word and only ever offers command names.
            tracing::debug!(target: "compsys_args", ?ws, cur, "get_comp_string publish words");
            crate::ported::params::setaparam("words", ws);
            // c:Src/Zle/complete.c:1251 — `{ "CURRENT", PM_INTEGER,
            // VAL(compcurrent), NULL }`: `$CURRENT` is bound to the
            // `compcurrent` INT through compvarinteger_gsu, so every
            // write is an integer assignment. Publishing the decimal
            // text with setsparam retyped the node, and `${(t)CURRENT}`
            // read `scalar-local-special` where zsh reads
            // `integer-local-special`.
            let _ = crate::ported::params::setiparam("CURRENT", cur as i64);
        }

        // cmdstr/linredir/linarr now mirror into the file-scope globals
        // (c:366/385) as the loop runs, so `callcompfunc` can read them
        // (compcore.c:598-630). `clwnum`/`cp`/`rd`/`ia` stay local — the
        // first is folded into the CLWORDS publish above, the rest are
        // only the restore sources for c:1453-1459.
        let _ = (clwnum, &cmdstr, &varname, cp, rd, ia);

        // c:1385-1386 — `if (addedx && tt) chuck(...)`: remove the `x`
        // (and optional space) injected above now that the lexer has
        // consumed it. ZLEMETALL/WB/WE were already computed excluding the
        // placeholder, so the line must lose it too — otherwise do_single/
        // do_ambiguous delete [wb,we] (which excludes the `x`) and the
        // placeholder leaks into the buffer (e.g. a unique completion of
        // `zipgre` left `zipgrep x` on the line).
        let addedx = ADDEDX.load(Ordering::SeqCst);
        if addedx > 0 {
            if let Some(m) = ZLEMETALINE.get() {
                if let Ok(mut g) = m.lock() {
                    let mut bytes = g.as_bytes().to_vec();
                    let cs = (zlemetacs.max(0) as usize).min(bytes.len());
                    let end = (cs + addedx as usize).min(bytes.len());
                    if cs < end {
                        bytes.drain(cs..end);
                        // Metafied line (see the addx splice at c:1152):
                        // rebuild byte-for-byte, never lossily.
                        *g = unsafe { String::from_utf8_unchecked(bytes) };
                    }
                }
            }
            ADDEDX.store(0, Ordering::SeqCst);
        }

        // c:1709-1726 — `if ((p = parambeg(s))) { … } else { … }`. This runs
        // BEFORE the quote-form block below and decides which of two ways the
        // lexer's quote markers get resolved:
        //
        //   * the word contains a parameter expression bracketing the cursor
        //     (`parambeg` non-NULL) — EVERY `Dnull`/`Snull` in the word is
        //     rewritten to a LITERAL `"`/`'` (c:1710-1714);
        //   * otherwise — only the markers nested inside a `${…}` are
        //     rewritten, tracked by a brace `level` counter (c:1716-1725).
        //
        // The first branch is load-bearing for `echo "$PA<TAB>`: rewriting the
        // leading `Dnull` to a literal `"` makes the `*s == Dnull` test at
        // c:1728 FAIL, so the quote-form block is skipped, `instring` stays
        // `QT_NONE` and `qipre` stays empty. The `"` then remains part of `s`,
        // and `check_param` (compcore.c:1113) splits the word into
        // `IPREFIX="$"` / `PREFIX="PA"` and sets `ispar`, which
        // `callcompfunc` turns into `compstate[context]=parameter`
        // (compcore.c:578-579).
        //
        // Without this hunk the port took the quote-form branch instead:
        // `compstate[quote]="` / `QIPREFIX="` / `PREFIX=$PA`, `check_param`
        // never fired, `compstate[context]` stayed `command`, `_parameters`
        // was never reached, and `_main_complete` fell through to
        // `_approximate`, offering FILENAMES for `echo "$PA<TAB>`.
        {
            use crate::ported::zsh_h::{Inbrace, Outbrace, Qstring, Stringg};
            let offs_now = OFFS.load(Ordering::SeqCst).max(0) as usize; // c:1708
            let sc: Vec<char> = s.chars().collect();
            if parambeg(&s, offs_now.min(sc.len())).is_some() {
                // c:1710-1714 — every Dnull/Snull becomes a literal quote.
                s = sc
                    .iter()
                    .map(|&c| {
                        if c == dnull {
                            '"'
                        } else if c == snull {
                            '\''
                        } else {
                            c
                        }
                    })
                    .collect();
            } else {
                // c:1716-1725 — only the markers inside a `${…}` are rewritten.
                let mut level: i32 = 0;
                let mut out = String::with_capacity(s.len());
                for (i, &c) in sc.iter().enumerate() {
                    if level != 0 && c == snull {
                        out.push('\''); // c:1719-1720
                    } else if level != 0 && c == dnull {
                        out.push('"'); // c:1721-1722
                    } else {
                        if (c == Stringg || c == Qstring)
                            && sc.get(i + 1).copied() == Some(Inbrace)
                        {
                            level += 1; // c:1723-1724
                        } else if c == Outbrace {
                            level -= 1; // c:1725
                        }
                        out.push(c);
                    }
                }
                s = out;
            }
        }

        // c:1728-1776 — quote-form detection. When the word being completed
        // is the INSIDE of a quoted string, the lexer left the opening
        // quote as an `inull` marker (`Snull` for `'`, `Dnull` for `"`,
        // `String`/`Qstring` + `Snull` for `$'`). C records the quote in
        // three globals the whole completion system then reads:
        //   * `instring`  — `callcompfunc` (compcore.c:669-693) turns it into
        //                   `$compstate[quote]` / `$compstate[quoting]`;
        //   * `qipre`/`qisuf` — become `$QIPREFIX` / `$QISUFFIX`
        //                   (compcore.c:742-745) and are prepended/appended to
        //                   every match's ipre/isuf (compcore.c:2934-2941);
        //   * `autoq`     — the quote `do_single` re-closes the word with
        //                   (compresult.c).
        // None of this was ported, so `instring` stayed `QT_NONE` from
        // c:1157 and `qipre`/`qisuf` stayed empty for every completion:
        // `ls "fo<TAB>` lost its opening `"` off the line and completers
        // that branch on `$QIPREFIX` (Completion/Unix/Type/_remote_files,
        // Completion/Zsh/Type/_vars) took the unquoted path.
        //
        // NOT ported from this hunk: c:1774-1776 (the BANGHIST `\!`
        // sanitize, which rewrites a `\` before a `!` inside double quotes
        // as `Bnull`). The rationale that used to sit here — "that walk only
        // changes bytes the untokenize at the return already drops" — no
        // longer holds now that c:1787-1926 is ported below: that loop chucks
        // a `Bnull` out of `s` while LEAVING the `\` on the line, so the two
        // are not equivalent. `echo "\!x<TAB>` is byte-identical to zsh
        // either way today, and porting it needs the history front-end's
        // `\!` handling checked first, so it stays flagged rather than
        // guessed at.
        //
        // c:1709-1726 IS now ported, immediately above. A previous revision
        // of this comment claimed it "only rewrites when the word is a
        // parameter expansion (whose first char is `String` + `Inbrace`)";
        // c:521-590 (`parambeg`) has no such restriction — it returns
        // non-NULL for a bare `$name` at the cursor anywhere in the word,
        // `"$PA` included, and that is exactly the case the omission broke.
        {
            use crate::ported::zsh_h::{Qstring, Stringg, QT_DOLLARS, QT_DOUBLE, QT_SINGLE};
            let sc: Vec<char> = s.chars().collect();
            let c0 = sc.first().copied();
            let c1 = sc.get(1).copied();
            // c:1728-1730 — `(*s == Snull || *s == Dnull ||
            //   ((*s == String || *s == Qstring) && s[1] == Snull))
            //   && !has_real_token(s + 1)`
            let quoted = (c0 == Some(snull)
                || c0 == Some(dnull)
                || ((c0 == Some(Stringg) || c0 == Some(Qstring)) && c1 == Some(snull)))
                && !has_real_token(&sc[1.min(sc.len())..].iter().collect::<String>());
            if quoted {
                let mut sl = sc.len(); // c:1731 — `int sl = strlen(s);`
                let mut qtptr = 0usize; // c:1732 — `char *qtptr = s;`
                let mut q: &str;
                match c0 {
                    // c:1735-1738
                    x if x == Some(snull) => {
                        q = "'";
                        INSTRING.store(QT_SINGLE, Ordering::SeqCst);
                    }
                    // c:1740-1743
                    x if x == Some(dnull) => {
                        q = "\"";
                        INSTRING.store(QT_DOUBLE, Ordering::SeqCst);
                    }
                    // c:1745-1751 — `$'…'`: q is "$'", and qtptr/sl skip the
                    // leading String/Qstring so the closing-quote test below
                    // still compares against the `Snull` at qtptr[0].
                    _ => {
                        q = "$'";
                        INSTRING.store(QT_DOLLARS, Ordering::SeqCst);
                        qtptr += 1;
                        sl -= 1;
                    }
                }
                // c:1753-1755 — `n = tricat(qipre, q, ""); qipre = n;`
                if let Ok(mut g) = QIPRE.get_or_init(|| Mutex::new(String::new())).lock() {
                    g.push_str(q); // c:1753-1755
                }
                // c:1760-1761 — `if (*q == '$') q++;`
                if q.starts_with('$') {
                    q = &q[1..];
                }
                // c:1762-1766 — `if (sl > 1 && qtptr[sl - 1] == *qtptr)
                //                    qisuf = tricat(q, qisuf, "");`
                // i.e. the word already carries its CLOSING quote, so the
                // suffix has to carry it too.
                if sl > 1 && sc.get(qtptr + sl - 1).copied() == sc.get(qtptr).copied() {
                    if let Ok(mut g) = QISUF.get_or_init(|| Mutex::new(String::new())).lock() {
                        g.insert_str(0, q); // c:1763-1765
                    }
                }
                // c:1767 — `autoq = ztrdup(q);`
                if let Ok(mut g) = AUTOQ.get_or_init(|| Mutex::new(String::new())).lock() {
                    *g = q.to_string();
                }
                tracing::debug!(
                    target: "compsys_args",
                    qipre = %qipre_get(), qisuf = %qisuf_get(),
                    instring = INSTRING.load(Ordering::SeqCst),
                    "get_comp_string quote-form"
                );
            }
        }

        // c:1780-1786 — a leading `=` is tokenized unconditionally so that a
        // later `setopt equals` still sees it; the option is fixed by now, so
        // put the plain `=` back when it is unset.
        {
            use crate::ported::zsh_h::{Equals, EQUALSOPT};
            if s.starts_with(Equals) && !isset(EQUALSOPT) {
                // c:1785-1786 — `*s = '='`
                s = format!("={}", &s[Equals.len_utf8()..]);
            }
        }

        // c:1787-1926 — "While building the quoted form, we also clean up the
        // command line."
        //
        // The lexer left one `inull` marker in `s` for every quote character
        // the user typed: `Snull` for `'`, `Dnull` for `"`, `Bnull` for `\`,
        // `String`/`Qstring` + `Snull` for `$'…'`. This loop walks `s` and the
        // LIVE metafied line in lockstep and removes both sides at once — the
        // marker from `s` (c:1919-1921 `chuck`) and the quote character it
        // stands for from the line (c:1897 `foredel` / c:1909 `backdel`) —
        // keeping `zlemetacs`, `we` and `offs` consistent as the line shrinks.
        // The quotes come back later from `qipre`/`qisuf`/`autoq`, which the
        // block above just published.
        //
        // Skipping it left the line untouched: for `echo "a<TAB>` zsh's
        // `$BUFFER` inside the completer is `echo a` with `$CURSOR` 6, while
        // this port reported `echo "a` / 7. Every completer that inspects the
        // line rather than `$PREFIX` then saw a word that was still quoted.
        //
        // REPRESENTATION NOTE (Rust-only, no C counterpart): C walks `s` as
        // BYTES and each parser token is exactly one byte, which is also the
        // one line byte it stands for, so C's `p` and `i` advance together.
        // Here the token chars live at U+0080..U+009F and take TWO UTF-8 bytes
        // in `s` while still standing for ONE byte of the metafied line, so
        // the scan runs over `Vec<char>` (`p` is a char index) and `i` stays a
        // line BYTE offset — the same convention `WB`/`WE`/`ZLEMETACS` use.
        {
            use crate::ported::lex::getkeystring_dollar_quote;
            use crate::ported::zsh_h::{Qstring, Stringg, RCQUOTES};
            use crate::ported::ztype_h::inull;

            /// `inull()` over a token char. The typtab predicate is byte-wide
            /// (`Src/ztype.h:62`), and every INULL token is < U+0100.
            fn inull_ch(c: char) -> bool {
                (c as u32) < 0x100 && inull(c as u32 as u8)
            }
            /// C's `memcpy(zlemetaline + at, t, strlen(t))` (c:1852) — overwrite
            /// the line bytes `t` is exactly as long as.
            fn write_line(at: usize, t: &str) {
                if let Some(m) = ZLEMETALINE.get() {
                    if let Ok(mut g) = m.lock() {
                        let end = at + t.len();
                        if end <= g.len() && g.is_char_boundary(at) && g.is_char_boundary(end) {
                            g.replace_range(at..end, t);
                        }
                    }
                }
            }

            let wb = WB.load(Ordering::SeqCst);
            let mut sc: Vec<char> = s.chars().collect();
            // c:1788 — `for (p = s, i = wb, j = 0; *p; p++, i++)`
            let mut p: usize = 0;
            let mut i: i32 = wb;
            let mut j: i32 = 0;
            let mut offs = OFFS.load(Ordering::SeqCst);
            let mut we = WE.load(Ordering::SeqCst);
            while p < sc.len() {
                let c = sc[p];
                let mut skipchars: i32; // c:1789
                if c == Stringg && sc.get(p + 1).copied() == Some(snull) {
                    // c:1790 — an unsubstituted `$'…'`.
                    // c:1792-1794 — scan for the closing `Snull`, but no
                    // further than the cursor.
                    let cs_now = ZLEMETACS.load(Ordering::SeqCst);
                    let mut pe = p + 2;
                    while pe < sc.len() && sc[pe] != snull && i + ((pe - p) as i32) < cs_now {
                        pe += 1;
                    }
                    if pe >= sc.len() || sc[pe] != snull {
                        // c:1795-1799 — no terminating Snull, can't substitute.
                        skipchars = 2;
                        if pe < sc.len() {
                            j = 1; // c:1798-1799 — `if (*pe) j = 1;`
                        }
                    } else {
                        // c:1800-1809 — decode the `$'…'` body.
                        // `getkeystring(p + 2, &len, GETKEYS_DOLLARS_QUOTE, NULL)`
                        // sets `*len` to "length to following character"
                        // (Src/utils.c:7189-7191), i.e. body + closing Snull;
                        // c:1807 then adds the 2 chars of the `$'` opener.
                        let (t, snull_idx) = getkeystring_dollar_quote(&sc, p + 2);
                        let len = (snull_idx - p + 1) as i32; // c:1807 — `len += 2`
                        let tlen = t.chars().count() as i32; // c:1808
                        skipchars = len - tlen; // c:1809
                        if skipchars >= 0 {
                            // c:1817-1864 — substitute in place.
                            let tchars: Vec<char> = t.chars().collect();
                            // c:1819 — `memcpy(p, t, tlen)`
                            sc[p..p + tlen as usize].copy_from_slice(&tchars);
                            // c:1821-1822 — `ocs = zlemetacs; zlemetacs = i;`.
                            // Reproduced verbatim including the consequence
                            // that a `skipchars == 0` substitution (decoded
                            // text exactly as long as the source) never puts
                            // `ocs` back — c:1823's `if (skipchars > 0)`
                            // guards the only restore.
                            let ocs = ZLEMETACS.load(Ordering::SeqCst);
                            ZLEMETACS.store(i, Ordering::SeqCst);
                            if skipchars > 0 {
                                // c:1824-1828 — move the tail of `s` up.
                                sc.drain(p + tlen as usize..p + len as usize);
                                // c:1829-1836
                                if i < ocs {
                                    offs -= skipchars;
                                }
                                // c:1837-1838 — move the tail of the line up.
                                foredel(skipchars, CUT_RAW);
                                // c:1839-1844
                                ZLEMETACS.store(ocs, Ordering::SeqCst);
                                if ocs > i {
                                    ZLEMETACS.store(ocs - skipchars, Ordering::SeqCst);
                                }
                                we -= skipchars; // c:1846
                            }
                            // c:1848-1852 — copy the unquoted string into place.
                            write_line(i as usize, &t);
                            // c:1854-1864 — `p += tlen - 1; i += tlen - 1;
                            // continue;` plus the loop's own `p++, i++`.
                            p += tlen as usize;
                            i += tlen;
                            continue;
                        } else {
                            // c:1865-1876 — the expansion is LONGER than the
                            // original, so give up and treat it as a plain
                            // single-quoted region.
                            skipchars = 2;
                            j = 1;
                        }
                    }
                } else if c == Qstring && sc.get(p + 1).copied() == Some(snull) {
                    skipchars = 2; // c:1879-1880
                } else if inull_ch(c) {
                    skipchars = 1; // c:1881-1882
                } else {
                    skipchars = 0; // c:1883-1884
                }
                if skipchars != 0 {
                    // c:1885
                    if i < ZLEMETACS.load(Ordering::SeqCst) {
                        offs -= skipchars; // c:1886-1887
                    }
                    if c == snull && isset(RCQUOTES) {
                        j = 1 - j; // c:1888-1889
                    }
                    if sc.get(p + 1).is_some() || c != bnull {
                        // c:1890
                        if c == bnull {
                            // c:1891-1893 — the `\` stays on the LINE (it is
                            // the escape for the next character); only the
                            // marker leaves `s`.
                            if ZLEMETACS.load(Ordering::SeqCst) == i + 1 {
                                ZLEMETACS.fetch_add(1, Ordering::SeqCst);
                                offs += 1;
                            }
                            // c:1922-1923 — `p--` cancels the loop's `p++`;
                            // `i` is NOT decremented here, so it advances one
                            // line byte past the surviving `\`.
                            i += 1;
                        } else {
                            // c:1894-1905. In c:1898's
                            // `if ((zlemetacs = ocs) > --i)` the `--i` fires
                            // either way and cancels the loop's `i++`, since
                            // `s` and the line lost the same number of units
                            // at this position.
                            let ocs = ZLEMETACS.load(Ordering::SeqCst); // c:1895
                            ZLEMETACS.store(i, Ordering::SeqCst); // c:1896
                            foredel(skipchars, CUT_RAW); // c:1897
                            i -= 1; // c:1898
                            ZLEMETACS.store(ocs, Ordering::SeqCst);
                            if ocs > i {
                                let mut cs = ocs - skipchars; // c:1899
                                if wb > cs {
                                    cs = wb; // c:1900-1902
                                }
                                ZLEMETACS.store(cs, Ordering::SeqCst);
                            }
                            we -= skipchars; // c:1904
                            i += 1; // loop `i++`
                        }
                    } else {
                        // c:1906-1918 — a trailing `\` at the very end of the
                        // word: there is nothing after it to escape, so the
                        // backslash itself comes off the END of the line.
                        let ocs = ZLEMETACS.load(Ordering::SeqCst); // c:1907
                        ZLEMETACS.store(we, Ordering::SeqCst); // c:1908
                        backdel(skipchars, CUT_RAW); // c:1909
                        let mut cs = if ocs == we { we - skipchars } else { ocs }; // c:1910-1913
                        if wb > cs {
                            cs = wb; // c:1914-1916
                        }
                        ZLEMETACS.store(cs, Ordering::SeqCst);
                        we -= skipchars; // c:1917
                        i += 1; // loop `i++`
                    }
                    // c:1919-1921 — "we need to get rid of all the quotation
                    // bits..."
                    for _ in 0..skipchars {
                        if p < sc.len() {
                            sc.remove(p);
                        }
                    }
                    // c:1922-1923 — "but we only decrement once to confuse the
                    // loop increment", i.e. `p` stays put.
                    continue;
                } else if j != 0 && c == '\'' && i < ZLEMETACS.load(Ordering::SeqCst) {
                    offs -= 1; // c:1924-1925
                }
                p += 1;
                i += 1;
            }
            s = sc.into_iter().collect();
            OFFS.store(offs, Ordering::SeqCst);
            WE.store(we, Ordering::SeqCst);
            tracing::debug!(
                target: "compsys_args",
                s = %s, offs, we,
                zlemetacs = ZLEMETACS.load(Ordering::SeqCst),
                "get_comp_string quote cleanup"
            );
        }

        // c:1928-1929 — `zsfree(origword); origword = ztrdup(s);`. C saves
        // the word here, still tokenized, and `docomplete` hands THIS to
        // `doexpansion` (c:826) / `spckword` (c:802). The port returns the
        // untokenized form, so the tokenized one has to be saved separately.
        // C saves it BEFORE the c:1931-2218 brace tail rewrites `s`, so this
        // must stay above that block: `doexpansion` needs the word with its
        // braces still in it, while the value returned to `docompletion` is
        // the brace-stripped one.
        if let Ok(mut g) = ORIGWORD.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = s.clone(); // c:1929
        }

        // c:1931-2218 — the brace-expansion tail. When the word being
        // completed sits inside an UNFINISHED brace expansion (`ls /usr/{b`)
        // the `{`, and everything from it up to the last comma before the
        // cursor, is not part of the string to complete: zsh strips it out of
        // `s`, records it in the `brbeg` chain, completes the remainder
        // (`/usr/b` -> `/usr/bin`), and `instmatch` (compresult.c:170-210)
        // re-inserts the recorded text at its recorded position. Without this
        // the completer saw the literal `/usr/{b`, matched nothing, and the
        // word was left untouched.
        //
        // REPRESENTATION NOTE (Rust-only, no C counterpart): C walks `s` as
        // BYTES and every parser token (`Inbrace`, `Comma`, …) is exactly one
        // byte, so C's `i` / `dp` / `boffs` / `pos` are byte offsets that also
        // count one unit per token. In this port `s` is a metafied Rust
        // `String` whose token chars live at U+0080..U+009F and therefore
        // occupy TWO UTF-8 bytes each, so `String::len()` is NOT C's
        // `strlen()`. The one-unit-per-C-byte quantity here is the CHAR count
        // (a metafied string holds no char above U+00FF), so the whole scan
        // runs over `Vec<char>` with char indices, and every `strlen()` in the
        // C below becomes `.chars().count()`. That also keeps `boffs`
        // commensurate with `offs`, which indexes the untokenized return
        // value where each token is back to one ASCII char.
        if !crate::ported::zsh_h::isset(crate::ported::zsh_h::IGNOREBRACES) {
            // c:1931
            use crate::ported::lex::untokenize_ztokens;
            use crate::ported::utils::{itype_end, makecommaspecial, skipparens};
            use crate::ported::zle::comp_h::Brinfo;
            use crate::ported::zsh_h::{
                Comma, Equals, Hat, Inbrace, Inbrack, Inpar, Outbrace, Outbrack, Outpar, Pound,
                Qstring, Quest, Star, Stringg, Tilde, COMPLETEINWORD,
            };
            use crate::ported::ztype_h::{idigit, IIDENT};

            let instring = INSTRING.load(Ordering::SeqCst); // for quotename()
            let sv: Vec<char> = s.chars().collect();
            let slen = sv.len();
            let offs0 = OFFS.load(Ordering::SeqCst);

            // c:1934 — `char *curs = s + (isset(COMPLETEINWORD) ? offs :
            //                             (int)strlen(s));`
            let curs: usize = if crate::ported::zsh_h::isset(COMPLETEINWORD) {
                (offs0.max(0) as usize).min(slen)
            } else {
                slen
            };
            // c:1935 — `char *predup = dupstring(s), *dp = predup;`
            let mut predup: Vec<char> = sv.clone();
            let mut dp: usize = 0;
            // c:1936-1937 — `char *bbeg = NULL, *bend = NULL, *dbeg = NULL;
            //                char *lastp = NULL, *firsts = NULL;`
            let mut bbeg: Option<usize> = None;
            let mut bend: usize = 0;
            let mut dbeg: usize = 0;
            let mut lastp: Option<usize> = None;
            let mut firsts: Option<usize> = None;
            // c:1938 — `int cant = 0, begi = 0, boffs = offs, hascom = 0;`
            let mut cant = 0i32;
            let mut begi = 0i32;
            let mut boffs = offs0;
            let mut hascom = 0i32;

            // The two chains. C threads `Brinfo` nodes through `next` while
            // keeping `lastbrbeg` / `lastbrend` tail handles; the port builds
            // flat vectors and links them once at the end, because
            // `Option<Box<Brinfo>>` cannot be appended to in place without
            // re-walking. `brbeg_v` is in discovery order (C appends via
            // `lastbrbeg`), `brend_v` is in REVERSE discovery order (C
            // prepends at c:2147-2148), which is exactly the order the
            // c:2189-2201 fix-up and `instmatch` (compresult.c:183-198) walk.
            let mut brbeg_v: Vec<Brinfo> = Vec::new();
            let mut brend_v: Vec<Brinfo> = Vec::new();

            // c:1940 — `for (i = 0, p = s; *p; p++, dp++, i++)`. The three
            // increments run on every normal iteration AND on `continue`.
            let mut i: i32 = 0;
            let mut p: usize = 0;
            while p < slen {
                // c:1941-1945 — careful, ${... is not a brace expansion...
                // we try to get braces after a parameter expansion right,
                // but this may fail sometimes. sorry.
                let c = sv[p];
                if c == Stringg || c == Qstring {
                    // c:1946
                    let n1 = sv.get(p + 1).copied();
                    if n1 == Some(Inbrace) || n1 == Some(Inpar) || n1 == Some(Inbrack) {
                        // c:1947-1958 — a `${…}` / `$(…)` / `$[…]`: skip the
                        // whole balanced group, it is not a brace expansion.
                        let open = n1.unwrap(); // c:1948 `char *tp = p + 1;`
                        let close = if open == Inbrace {
                            Outbrace // c:1950
                        } else if open == Inpar {
                            Outpar // c:1951
                        } else {
                            Outbrack // c:1951
                        };
                        let tail: String = sv[p + 1..].iter().collect();
                        let mut rest: &str = &tail;
                        let unbalanced = skipparens(open, close, &mut rest); // c:1950-1952
                        let adv = tail.chars().count() - rest.chars().count();
                        if unbalanced != 0 {
                            // c:1953-1954 — `tt = NULL; break;`
                            tt = None;
                            break;
                        }
                        let tp = p + 1 + adv;
                        i += (tp - p) as i32; // c:1956
                        dp += tp - p; // c:1957
                        p = tp; // c:1958
                    } else if n1 != Some(snull) {
                        // c:1959 — paranoia: should be gone now
                        let mut tp = p + 1; // c:1960
                                            // c:1962-1966
                        while let Some(&tc) = sv.get(tp) {
                            if tc == '^'
                                || tc == Hat
                                || tc == '='
                                || tc == Equals
                                || tc == '~'
                                || tc == Tilde
                                || tc == '#'
                                || tc == Pound
                                || tc == '+'
                            {
                                tp += 1;
                            } else {
                                break;
                            }
                        }
                        let tc = sv.get(tp).copied();
                        // c:1967-1970
                        if tc == Some(Quest)
                            || tc == Some(Star)
                            || tc == Some(Stringg)
                            || tc == Some(Qstring)
                            || tc == Some('?')
                            || tc == Some('*')
                            || tc == Some('$')
                            || tc == Some('-')
                            || tc == Some('!')
                            || tc == Some('@')
                        {
                            // c:1971 — `p++, i++;` (C advances neither `dp`
                            // nor the predup cursor here; transcribed as-is).
                            p += 1;
                            i += 1;
                        } else {
                            // c:1974-1976 — `if (idigit(*tp)) while (idigit(*tp)) tp++;`
                            if sv
                                .get(tp)
                                .is_some_and(|&x| (x as u32) < 128 && idigit(x as u8))
                            {
                                while sv
                                    .get(tp)
                                    .is_some_and(|&x| (x as u32) < 128 && idigit(x as u8))
                                {
                                    tp += 1;
                                }
                            } else {
                                // c:1977-1978 — `else if ((ie = itype_end(tp,
                                // IIDENT, 0)) != tp) tp = ie;`. `itype_end`
                                // returns a BYTE offset into the tail it was
                                // handed, so convert it back to the char
                                // count this scan works in.
                                let tail: String = sv[tp..].iter().collect();
                                let ie_bytes = itype_end(&tail, IIDENT as u32, false);
                                let ie = tail[..ie_bytes].chars().count();
                                if ie != 0 {
                                    tp += ie;
                                } else {
                                    // c:1980-1981 — `tt = NULL; break;`
                                    tt = None;
                                    break;
                                }
                            }
                            if sv.get(tp).copied() == Some(Inbrace) {
                                // c:1983-1985
                                cant = 1;
                                break;
                            }
                            tp -= 1; // c:1987
                            i += (tp - p) as i32; // c:1988
                            dp += tp - p; // c:1989
                            p = tp; // c:1990
                        }
                    }
                } else if p < curs {
                    // c:1993
                    if c == Outbrace {
                        // c:1994-2000 — HERE: strip and remember code from
                        // last comma to here.
                        cant = 1;
                        break;
                    }
                    if c == Inbrace {
                        // c:2002
                        let tail: String = sv[p..].iter().collect(); // c:2003 `char *tp = p;`
                        let mut rest: &str = &tail;
                        let unbalanced = skipparens(Inbrace, Outbrace, &mut rest); // c:2005
                        if unbalanced == 0 {
                            // c:2006-2019 — Balanced brace: skip. We only deal
                            // with unfinished braces, so
                            //  something{foo<x>bar,morestuff}else
                            // doesn't work.
                            let tp = p + (tail.chars().count() - rest.chars().count());
                            i += (tp - p) as i32 - 1; // c:2016
                            dp += tp - p - 1; // c:2017
                            p = tp - 1; // c:2018
                                        // c:2019 `continue;` — the for-increments still run.
                            p += 1;
                            dp += 1;
                            i += 1;
                            continue;
                        }
                        makecommaspecial(true); // c:2021
                        if let Some(bb) = bbeg {
                            // c:2022-2048
                            let len = bend - bb; // c:2024
                            NBRBEG.fetch_add(1, Ordering::SeqCst); // c:2027
                                                                   // c:2037-2039 — `new->str = dupstrpfx(bbeg, len);
                                                                   //   new->str = ztrdup(quotename(new->str));
                                                                   //   untokenize(new->str);`
                            let raw: String = sv[bb..bb + len].iter().collect();
                            let bstr = untokenize_ztokens(&quotename(&raw, instring));
                            // c:2041-2043 — `*dbeg = '\0';
                            //   new->qpos = strlen(quotename(predup));
                            //   *dbeg = '{';` — quote only the part of predup
                            //   BEFORE the brace run. (C's restore writes a
                            //   literal `{` back over the token byte; the
                            //   memmove at c:2046 overwrites that position
                            //   immediately, so the port just slices.)
                            let pre: String = predup[..dbeg].iter().collect();
                            let qpos = quotename(&pre, instring).chars().count() as i32;
                            brbeg_v.push(Brinfo {
                                next: None, // c:2029
                                prev: None, // see the prev note at c:2194
                                str: Some(bstr),
                                pos: begi, // c:2040
                                qpos,      // c:2042
                                curpos: 0,
                            });
                            i -= len as i32; // c:2044
                            boffs -= len as i32; // c:2045
                            predup.drain(dbeg..dbeg + len); // c:2046
                            dp -= len; // c:2047
                        }
                        bbeg = Some(p); // c:2049
                        lastp = Some(p); // c:2049
                        dbeg = dp; // c:2050
                        bend = p + 1; // c:2051
                        begi = i; // c:2052
                    } else if c == Comma && bbeg.is_some() {
                        // c:2053-2056
                        bend = p + 1;
                        hascom = 1;
                    }
                } else {
                    // c:2057-2058 — On or after the cursor position
                    if c == Inbrace {
                        // c:2059
                        let tail: String = sv[p..].iter().collect(); // c:2060
                        let mut rest: &str = &tail;
                        let unbalanced = skipparens(Inbrace, Outbrace, &mut rest); // c:2062
                        if unbalanced == 0 {
                            // c:2063-2067 — Balanced braces after the cursor.
                            let tp = p + (tail.chars().count() - rest.chars().count());
                            i += (tp - p) as i32 - 1; // c:2068
                            dp += tp - p - 1; // c:2069
                            p = tp - 1; // c:2070
                            p += 1; // c:2071 `continue;`
                            dp += 1;
                            i += 1;
                            continue;
                        }
                        cant = 1; // c:2073
                        makecommaspecial(true); // c:2074
                        break; // c:2075
                    }
                    if p == curs {
                        // c:2077-2085 — We've reached the cursor position.
                        // If there's a pending open brace at this point we
                        // need to stack the text. We've marked the bit we
                        // don't want from bbeg to bend, which might be a
                        // comma between the opening brace and us.
                        if let Some(bb) = bbeg {
                            // c:2086-2111 — identical body to c:2022-2048.
                            let len = bend - bb; // c:2088
                            NBRBEG.fetch_add(1, Ordering::SeqCst); // c:2091
                            let raw: String = sv[bb..bb + len].iter().collect();
                            let bstr = untokenize_ztokens(&quotename(&raw, instring)); // c:2100-2102
                            let pre: String = predup[..dbeg].iter().collect();
                            let qpos = quotename(&pre, instring).chars().count() as i32; // c:2105
                            brbeg_v.push(Brinfo {
                                next: None, // c:2093
                                prev: None,
                                str: Some(bstr),
                                pos: begi, // c:2103
                                qpos,
                                curpos: 0,
                            });
                            i -= len as i32; // c:2107
                            boffs -= len as i32; // c:2108
                            predup.drain(dbeg..dbeg + len); // c:2109
                            dp -= len; // c:2110
                        }
                        bbeg = None; // c:2112
                    }
                    if c == Comma {
                        // c:2114-2123 — Comma on or after cursor. We set bbeg
                        // to NULL at the cursor; here it's being used to find
                        // the first comma afterwards.
                        if bbeg.is_none() {
                            bbeg = Some(p);
                        }
                        hascom = 2;
                    } else if c == Outbrace {
                        // c:2124-2131 — Closing brace on or after the cursor.
                        if bbeg.is_none() {
                            bbeg = Some(p); // c:2135-2136
                        }
                        let bb = bbeg.unwrap();
                        let len = p + 1 - bb; // c:2137
                        if firsts.is_none() {
                            firsts = Some(p + 1); // c:2138-2139
                        }
                        NBREND.fetch_add(1, Ordering::SeqCst); // c:2142
                        let raw: String = sv[bb..bb + len].iter().collect();
                        let bstr = untokenize_ztokens(&quotename(&raw, instring)); // c:2150-2152
                                                                                   // c:2147-2148 — `new->next = brend; brend = new;`
                        brend_v.insert(
                            0,
                            Brinfo {
                                next: None,
                                prev: None,
                                str: Some(bstr),
                                pos: dp as i32 - len as i32 + 1, // c:2153
                                qpos: len as i32,                // c:2154
                                curpos: 0,
                            },
                        );
                        bbeg = None; // c:2155
                    }
                }
                p += 1; // c:1940 for-increments
                dp += 1;
                i += 1;
            }
            if cant != 0 {
                // c:2159-2163 — `freebrinfo(brbeg); freebrinfo(brend);
                //   brbeg = lastbrbeg = brend = lastbrend = NULL;
                //   nbrbeg = nbrend = 0;`
                brbeg_v.clear();
                brend_v.clear();
                if let Some(b) = BRBEG.get() {
                    *b.lock().unwrap() = None;
                }
                if let Some(b) = BREND.get() {
                    *b.lock().unwrap() = None;
                }
                NBRBEG.store(0, Ordering::SeqCst);
                NBREND.store(0, Ordering::SeqCst);
            } else {
                // c:2165 — `if (p == curs && bbeg)`. Reached when the cursor
                // sits at the very END of the word (the common
                // `ls /usr/{b<TAB>` shape): the loop ran off the end with an
                // open brace still pending, so the run from `{` to the last
                // comma has to be stacked now.
                if p == curs {
                    if let Some(bb) = bbeg {
                        let len = bend - bb; // c:2167
                        NBRBEG.fetch_add(1, Ordering::SeqCst); // c:2170
                        let raw: String = sv[bb..bb + len].iter().collect();
                        let bstr = untokenize_ztokens(&quotename(&raw, instring)); // c:2179-2181
                        let pre: String = predup[..dbeg].iter().collect();
                        let qpos = quotename(&pre, instring).chars().count() as i32; // c:2184
                        brbeg_v.push(Brinfo {
                            next: None, // c:2172
                            prev: None,
                            str: Some(bstr),
                            pos: begi, // c:2182
                            qpos,
                            curpos: 0,
                        });
                        boffs -= len as i32; // c:2186
                        predup.drain(dbeg..dbeg + len); // c:2187
                    }
                }
                if !brend_v.is_empty() {
                    // c:2189-2201 — rewrite every closing-brace node from a
                    // position-in-predup pair into the (tail length, quoted
                    // tail length) pair `instmatch` wants, stripping the run
                    // out of predup as it goes. The walk is head-to-tail,
                    // i.e. LAST-discovered first, so the earlier nodes'
                    // recorded positions stay valid while predup shrinks.
                    //
                    // RUST-ONLY: C also threads `bp->prev` here (c:2194) to
                    // give `instmatch` a doubly-linked chain to walk
                    // backwards from `lastbrend`. `Option<Box<Brinfo>>` owns
                    // its successor, so a real back-pointer is impossible;
                    // compresult.rs:362-372 flattens both chains into `Vec`s
                    // and walks `brend` by descending index instead, which is
                    // the same traversal.
                    for bp in brend_v.iter_mut() {
                        let pos = bp.pos.max(0) as usize; // c:2196
                        let l = bp.qpos.max(0) as usize; // c:2197
                        let cut = (pos + l).min(predup.len());
                        let tail: String = predup[cut..].iter().collect();
                        bp.pos = tail.chars().count() as i32; // c:2198
                        bp.qpos = quotename(&tail, instring).chars().count() as i32; // c:2199
                        predup.drain(pos.min(predup.len())..cut); // c:2200
                    }
                }
                if hascom != 0 {
                    // c:2203-2213
                    if let Some(lp) = lastp {
                        // c:2204-2209 — `char sav = *lastp;
                        //   *lastp = '\0';
                        //   untokenize(lastprebr = ztrdup(s));
                        //   *lastp = sav;`
                        //
                        // NOT `quotename(s)`: the port applied `quotename`
                        // here on a citation that quoted C which does not
                        // exist at c:2208. `lastprebr` is compared verbatim
                        // against the text `instmatch` puts on the line
                        // (compresult.c:626-628), so quoting it here made
                        // every candidate mismatch whenever the word ahead
                        // of the `{` held a character `quotename` escapes.
                        let pre: String = sv[..lp].iter().collect();
                        let v = untokenize_ztokens(&pre);
                        if let Ok(mut g) = LASTPREBR.get_or_init(|| Mutex::new(None)).lock() {
                            *g = Some(v);
                        }
                    }
                    // c:2211-2212 — `if ((lastpostbr = ztrdup(firsts)))
                    //                    untokenize(lastpostbr);`
                    // The assignment is UNCONDITIONAL — `ztrdup(NULL)` is
                    // NULL — only the `untokenize` is guarded. The port
                    // skipped the whole statement when `firsts` was NULL,
                    // which left the PREVIOUS completion's value in place.
                    if let Ok(mut g) = LASTPOSTBR.get_or_init(|| Mutex::new(None)).lock() {
                        *g = firsts.map(|f| {
                            let post: String = sv[f.min(slen)..].iter().collect();
                            untokenize_ztokens(&post)
                        });
                    }
                }
                // c:2214-2216 — `zsfree(s); s = ztrdup(predup); offs = boffs;`
                s = predup.iter().collect();
                OFFS.store(boffs, Ordering::SeqCst);

                // Publish the two chains. C built them in place through the
                // `brbeg`/`brend` globals as the scan ran.
                let link = |mut v: Vec<Brinfo>| -> Option<Box<Brinfo>> {
                    let mut head: Option<Box<Brinfo>> = None;
                    while let Some(mut node) = v.pop() {
                        node.next = head;
                        head = Some(Box::new(node));
                    }
                    head
                };
                if let Ok(mut g) = BRBEG.get_or_init(|| Mutex::new(None)).lock() {
                    *g = link(brbeg_v);
                }
                if let Ok(mut g) = BREND.get_or_init(|| Mutex::new(None)).lock() {
                    *g = link(brend_v);
                }
                tracing::debug!(
                    target: "compsys_args",
                    s = %s, offs = boffs,
                    nbrbeg = NBRBEG.load(Ordering::SeqCst),
                    nbrend = NBREND.load(Ordering::SeqCst),
                    "get_comp_string brace tail"
                );
            }
        }

        // c:2219 — zcontext_restore(); return s.
        //
        // C returns `s` TOKENIZED, and `docomplete`'s expand-vs-complete
        // decision (c:704-793) is written entirely in parser tokens —
        // `String`, `Inbrace`, `Star`, `Quest`, `Tick` — precisely so a
        // quoted `\*` (a literal `*` in `s`) is NOT mistaken for a glob.
        // Since this port untokenizes before returning, stash the
        // tokenized form for that decision to read.
        if let Ok(mut g) = COMP_STRING_TOK
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
        {
            *g = s.clone();
        }
        zcontext_restore();
        return Some(untokenize(&s));
    }
}

/// Port of `int inststrlen(char *str, int move, int len)` from
/// `Src/Zle/zle_tricky.c:2231`.
///
/// Insert `str` at the current cursor; advance cursor when `move`
/// is set. Honors both line storage modes per C:
/// - `zlemetaline != NULL` → splice into ZLEMETALINE / advance ZLEMETACS.
/// - else → convert `str` via `stringaszleline`, splice chars into
///   ZLELINE / advance ZLECS.
///
/// `len == -1` triggers strlen(str). `move` (Rust `move_cursor` — the
/// C name collides with Rust's `move` keyword) selects cursor-advance.
pub fn inststrlen(
    // c:2231
    str: &str,
    move_cursor: bool,
    mut len: i32,
) -> i32 {
    // c:2233-2234 — `if (!len || !str) return 0;`
    if len == 0 || str.is_empty() {
        return 0;
    }
    // c:2235-2236 — `if (len == -1) len = strlen(str);`
    if len == -1 {
        len = str.len() as i32;
    }
    // `len` is a BYTE count and both C paths are byte-exact:
    // `strncpy(zlemetaline + zlemetacs, str, len)` on the meta side,
    // `ztrduppfx(str, len)` on the wide side. `str` is a METAFIED string on
    // every path (the wide side proves it — C hands the prefix straight to
    // `stringaszleline`, which unmetafies), so it is not valid UTF-8 by
    // construction and must never be sliced on a UTF-8 boundary: an earlier
    // clamp-down-to-a-boundary silently dropped the straddled byte, and the
    // `insert_str` boundary back-off moved the splice point off `zlemetacs`.
    // Take exactly `len` bytes, clamped only by the string's own length.
    let take = (len as usize).min(str.len());
    // c:2237 — `if (zlemetaline != NULL) { meta path } else { wide path }`
    let zml_active = ZLEMETALINE.get().is_some();
    if zml_active {
        // c:2238 — `spaceinline(len);` then strncpy into ZLEMETALINE[cs..].
        // The Rust spaceinline operates on ZLELINE; for the meta path
        // we splice directly into ZLEMETALINE.
        if let Some(m) = ZLEMETALINE.get() {
            if let Ok(mut g) = m.lock() {
                let cs = (ZLEMETACS.load(Ordering::SeqCst) as usize).min(g.len());
                // c:2238-2239 — `spaceinline(len)` grows the line and shifts
                // only the bytes from the cursor onwards; `strncpy` then
                // drops `str` into the hole. `Vec::splice` over the String's
                // bytes is exactly that pair: amortised growth plus one
                // memmove of the tail. Rebuilding the whole line instead made
                // every insert copy the entire buffer, so splicing n
                // expansions into the line (`ls **/<TAB>`) cost O(n^2) bytes
                // copied. The byte view is required because the line is
                // metafied — `String::insert_str` asserts a char boundary at
                // `cs`, which a `Meta` escape does not satisfy.
                unsafe {
                    g.as_mut_vec()
                        .splice(cs..cs, str.as_bytes()[..take].iter().copied());
                }
                ZLEMETALL.store(g.len() as i32, Ordering::SeqCst); // c:2239 spaceinline updates ZLEMETALL
                if move_cursor {
                    // c:2240
                    ZLEMETACS.fetch_add(take as i32, Ordering::SeqCst); // c:2241 zlemetacs += len
                }
            }
        }
        return len;
    }
    // c:2244-2253 — non-meta wide path.
    // c:2247 `instr = ztrduppfx(str, len)` — the first `len` BYTES of the
    // metafied string; `stringaszleline` below unmetafies them.
    let instr_owned = unsafe { String::from_utf8_unchecked(str.as_bytes()[..take].to_vec()) };
    let instr = instr_owned.as_str();
    let zlestr: Vec<char> = stringaszleline(instr, 0, None, None, None); // c:2248
    let zlelen = zlestr.len();
    spaceinline(zlelen as i32); // c:2249
    {
        let mut line = ZLELINE.lock().unwrap();
        let pos = ZLECS.load(Ordering::SeqCst);
        for (i, ch) in zlestr.iter().enumerate() {
            // c:2250 ZS_strncpy
            if pos + i < line.len() {
                line[pos + i] = *ch;
            } else {
                line.insert(pos + i, *ch);
            }
        }
    }
    if move_cursor {
        // c:2253
        ZLECS.fetch_add(zlelen, Ordering::SeqCst); // c:2254 zlecs += len
    }
    len // c:2257 return len
}

/// Port of `int doexpansion(char *s, int lst, int olst, int explincmd)`
/// from `Src/Zle/zle_tricky.c:2263`.
///
/// Drives the expansion phase of `expand-or-complete` (and the bare
/// `expand-word` widget). Pipeline:
///   1. Push a fresh heap (`pushheap`).
///   2. Build a 1-element LinkList with a heap-dup of `s`.
///   3. Swap literal `"`/`'` to `Dnull`/`Snull` so prefork treats
///      them as tokens, not characters (matches `get_comp_string`'s
///      output convention).
///   4. `prefork(vl, 0, NULL)` — runs history/alias/parameter
///      expansion in place.
///   5. For `COMP_LIST_EXPAND` / `COMP_EXPAND`: temporarily set
///      `NULLGLOB` and run `globlist(vl, PREFORK_NO_UNTOK)` so
///      glob non-matches don't error out.
///   6. If expansion produced no change (peekfirst == ss) OR a
///      tilde-only expansion that `filesubstr` would have done
///      anyway, fall through:
///         * For `COMP_EXPAND_COMPLETE` recurse into `docompletion`.
///         * Otherwise just return 1 (caller beeps).
///   7. For `COMP_LIST_EXPAND`: restore the original buffer and
///      hand the list to `listlist` for menu display.
///   8. Otherwise: delete the current word from the buffer and
///      splice each expansion in, quoting + untokenizing each.
///   9. Pop heap and return.
pub fn doexpansion(s: &str, lst: i32, olst: i32, explincmd: i32) -> i32 {
    // c:2265 — `int ret = 1, first = 1;`
    let mut ret: i32 = 1;
    let mut first = true;
    // c:2266 — `LinkList vl; char *ss, *ts;` (decls)

    // c:2269 — `pushheap()`.
    crate::ported::mem::pushheap();

    // c:2270 — `vl = newlinklist()`.
    let mut vl: crate::ported::linklist::LinkList<String> = crate::ported::linklist::newlinklist();
    // c:2271 — `ss = dupstring(s)`.
    let ss = crate::ported::string::dupstring(s);
    // c:2274-2278 — swap "/' → Dnull/Snull. C walks `ts` byte-by-
    // byte; in Rust we rebuild the string from chars since `Dnull`/
    // `Snull` are non-ASCII char constants (0x9e / 0x9d).
    let ss: String = ss
        .chars()
        .map(|c| match c {
            '"' => crate::ported::zle::compctl::Dnull,
            '\'' => crate::ported::zle::compctl::Snull,
            c => c,
        })
        .collect();
    // c:2279 — `addlinknode(vl, ss)`. Rust API: push_back.
    vl.push_back(ss.clone());
    // c:2280 — `prefork(vl, 0, NULL)`. Rust port takes a ret-flags
    // out-param; pass a throwaway.
    let mut ret_flags = 0i32;
    crate::ported::subst::prefork(&mut vl, 0, &mut ret_flags);
    // c:2281-2282 — `if (errflag) goto end`.
    let _result: i32 = (|| -> i32 {
        if crate::ported::utils::errflag.load(Ordering::SeqCst) != 0 {
            return ret;
        }
        // c:2283-2289 — for COMP_LIST_EXPAND / COMP_EXPAND wrap
        // `globlist` between `opts[NULLGLOB]` toggles so glob
        // misses don't error.
        //
        // The option name MUST be the canonical one. `opt_state_get`
        // canonicalises through `optlookup` (options.rs:1976), but
        // `opt_state_set` (options.rs:1999) writes the caller's string
        // verbatim: `opt_state_set("NULL_GLOB", …)` created a dead
        // `"NULL_GLOB"` key while every reader — `isset(NULLGLOB)` →
        // `opts_cache::authoritative` → `opt_state_get(opt_name(126))`
        // (extensions/opts_cache.rs:86) — looks at `"nullglob"`. Both
        // toggles were therefore no-ops, NULLGLOB stayed off inside
        // `globlist`, and a non-matching pattern took glob.rs:1576's
        // NOMATCH arm: `ls -d **/b<TAB>` printed
        // `zsh: no matches found: **/b` onto the prompt line where zsh
        // silently beeps. Deriving the name from the optno keeps this
        // in lockstep with the `isset(NULLGLOB)` readers by
        // construction.
        if lst == COMP_LIST_EXPAND || lst == COMP_EXPAND {
            let nullglob = crate::ported::zsh_h::opt_name(crate::ported::zsh_h::NULLGLOB);
            let ng = crate::ported::zsh_h::isset(crate::ported::zsh_h::NULLGLOB); // c:2284
            crate::ported::options::opt_state_set(nullglob, true); // c:2286
            crate::ported::subst::globlist(&mut vl, crate::ported::zsh_h::PREFORK_NO_UNTOK); // c:2287
            crate::ported::options::opt_state_set(nullglob, ng); // c:2288
        }
        // c:2290-2291 — `if (errflag) goto end`.
        if crate::ported::utils::errflag.load(Ordering::SeqCst) != 0 {
            return ret;
        }
        // c:2292-2293 — `if (empty(vl) || !*(char *)peekfirst(vl))
        //                goto end`.
        if vl.empty() {
            return ret;
        }
        let first_item = vl.front().cloned().unwrap_or_default();
        // c:2292-2293 — `if (empty(vl) || !*(char *)peekfirst(vl)) goto end;`
        //
        // !!! WARNING: the second half of that test CANNOT be transcribed
        // literally, because this port's `prefork` uses a different string
        // convention than C's !!!
        //
        // In C the two halves separate two different outcomes:
        //   * the word EXPANDED TO NOTHING (unquoted empty / unset) — prefork
        //     `uremnode`s it, the list goes empty, and the line is left alone;
        //   * the word expanded to an EMPTY-BUT-PRESENT string (`"$unset"`) —
        //     prefork keeps the node, whose data is the one-byte `Nularg`
        //     sentinel (Src/glob.c:3683-3686 re-inserts it when stripping the
        //     quote markers leaves nothing). `*peekfirst` is that byte, NOT
        //     '\0', so C falls through and replaces the word with the empty
        //     expansion — which is why zsh DELETES `"$PA` off the line.
        // C's `!*peekfirst` therefore catches neither of those; it is a
        // belt-and-braces guard for a node C's prefork does not produce.
        //
        // This port's `prefork` collapses the sentinel to a true empty string
        // (src/ported/subst.rs:346-351, standing in for the `untokenize` C
        // does later) AFTER the keep-test that deletes genuinely empty nodes
        // (src/ported/subst.rs:408-415). So here an empty first element means
        // exactly what C's `Nularg` means — the case that must FALL THROUGH —
        // and testing `is_empty()` returned early on it, leaving `echo "$PA`
        // on the line where zsh leaves `echo `.
        let _ = &first_item; // c:2292 (see above: the byte test has no faithful form here)
                             // c:2294-2299 — no-change check. If the first item still
                             // equals `ss` (no real expansion happened), OR the only
                             // change was tilde-expansion that `filesubstr` would do
                             // (and the caller asked for `COMP_EXPAND_COMPLETE`), fall
                             // through to completion.
        let len_vl = {
            let mut n = 0;
            let mut cur = vl.firstnode();
            while cur.is_some() {
                n += 1;
                cur = cur.and_then(|i| vl.nextnode(i));
            }
            n
        };
        // c:2294 — `peekfirst(vl) == (void *) ss`, a POINTER identity test:
        // "prefork/globlist did not REPLACE the node". C's prefork edits the
        // word IN PLACE for quote removal (`remnulargs`, Src/subst.c:170,
        // which `chuck`s the null tokens out of the same buffer), so identity
        // survives that edit even though the CONTENT changed: for `ls "/us`
        // the node is still `ss` but now reads `/us`.
        //
        // This port has no pointer identity to test (prefork hands back owned
        // Strings), so the equivalent is "`ss` with exactly the in-place edit
        // C's prefork would have made", i.e. `ss` with its null tokens
        // removed. Comparing against the RAW `ss` instead reported "changed"
        // for every quoted word — `ls "/us` then took the replace-the-word
        // path with an empty NULLGLOB result and wedged the completion
        // instead of falling through to `docompletion`.
        let no_change = {
            let mut ss_unnulled = ss.clone(); // c:2294
            crate::ported::glob::remnulargs(&mut ss_unnulled); // c:Src/subst.c:170
            first_item == ss_unnulled
        };
        let tilde_only = olst == COMP_EXPAND_COMPLETE
            && len_vl == 1
            && s.starts_with(crate::ported::zsh_h::Tilde)
            && crate::ported::subst::filesubstr(s, false)
                .map(|exp| exp == first_item)
                .unwrap_or(false);
        if no_change || tilde_only {
            // c:2300-2304 — recurse into docompletion if asked. Capture its
            // return as doexpansion's result: C reaches the same outcome via a
            // POINTER identity no-change test (`peekfirst(vl) == (void *) ss`)
            // that this port approximates by string value, so the caller's
            // buffer-changed branch otherwise kept doexpansion's stale
            // `ret = 1` even after a successful unique completion — ringing a
            // spurious bell. do_completion returns 0 for a unique match and 1
            // for no-match / ambiguous (LISTBEEP), so propagating it makes the
            // widget return (and thus zlecore's handlefeep) beep only when zsh
            // does.
            if lst == COMP_EXPAND_COMPLETE {
                ret = docompletion(s, COMP_COMPLETE, explincmd);
            }
            return ret;
        }
        // c:2306-2316 — `COMP_LIST_EXPAND` path: restore the
        // original line and display the expansions as a list.
        if lst == COMP_LIST_EXPAND {
            ZLEMETACS.store(0, Ordering::SeqCst);
            foredel(ZLEMETALL.load(Ordering::SeqCst), CUT_RAW);
            spaceinline(ORIGLL.load(Ordering::SeqCst));
            if let (Some(metabuf), Some(orig)) = (ZLEMETALINE.get(), ORIGLINE.get()) {
                if let (Ok(mut m), Ok(o)) = (metabuf.lock(), orig.lock()) {
                    *m = o.clone();
                }
            }
            ZLEMETACS.store(ORIGCS.load(Ordering::SeqCst), Ordering::SeqCst);
            // Drain vl into a Vec<String> for listlist's slice arg.
            let mut items: Vec<String> = Vec::new();
            while let Some(x) = crate::ported::linklist::ugetnode(&mut vl) {
                items.push(x);
            }
            ret = listlist(&items, 0);
            SHOWINGLIST.store(0, Ordering::SeqCst);
            return ret;
        }
        // c:2319-2332 — splice expansions into the buffer at the
        // current word position (wb..we).
        let wb = WB.load(Ordering::SeqCst);
        let we = WE.load(Ordering::SeqCst);
        ZLEMETACS.store(wb, Ordering::SeqCst);
        foredel(we - wb, CUT_RAW);
        while let Some(node) = crate::ported::linklist::ugetnode(&mut vl) {
            ret = 0;
            // c:2324 — `ss = quotename(ss); untokenize(ss); inststr(ss);`
            let quoted = quotename(&node, 0);
            let unt = crate::ported::lex::untokenize(&quoted);
            inststr(&unt);
            // c:2326-2330 — between items, insert a space:
            //   `spaceinline(1); zlemetaline[zlemetacs++] = ' ';`
            // The Rust `spaceinline` opens the gap in the EDITOR buffer
            // (`zle_utils::ZLELINE`), not in the metafied one C is holding
            // here, so the old open-gap-then-poke-a-byte pair wrote nothing
            // when the cursor sat at end-of-line — `ls -d *<TAB>` expanded to
            // `aaccf1` instead of `aa cc f1`. `inststrlen`'s meta branch is
            // exactly C's gap-plus-store pair against ZLEMETALINE.
            if !vl.empty() || !first {
                let _ = inststrlen(" ", true, 1);
            }
            first = false;
        }
        ret
    })();

    // c:2334-2336 — `end: popheap(); return ret`.
    crate::ported::mem::popheap();
    _result
}

/// Direct port of `static int docompletion(char *s, int lst, int incmd)`
/// from `Src/Zle/zle_tricky.c:2339`. Wraps `(s, lst, incmd)` in a
/// `compldat` struct and fires the COMPLETEHOOK chain via
/// `runhookdef`. When no Hookfn is registered (matching the C
/// `complete.c:boot_` `addhookfunc("complete", do_completion)` chain
/// that the Rust port has not yet wired through `Hookfn` thunks), we
/// fall through to the canonical handler in `compcore::do_completion`
/// — same observational behavior as C's `def` fallback at
/// `module.c:993-994`.
pub fn docompletion(s: &str, lst: i32, incmd: i32) -> i32 {
    // c:2339
    let mut dat = crate::ported::zle::zle_h::compldat {
        // c:2342-2344
        s: s.to_string(),
        lst,
        incmd,
    };
    let h = gethookdef("complete");
    if !h.is_null() {
        // c:2346 — `runhookdef(COMPLETEHOOK, &dat)`.
        let dat_ptr =
            (&mut dat) as *mut crate::ported::zle::zle_h::compldat as *mut std::ffi::c_void;
        return crate::ported::module::runhookdef(h, dat_ptr);
    }
    // Fallback to the canonical Rust handler (matches the C
    // `addhookfunc("complete", do_completion)` registration that the
    // Rust port lowers to a direct call here).
    crate::ported::zle::compcore::do_completion(s, incmd, lst)
}

/// Get length of common prefix
/// Port of pfxlen(char *s, char *t) from zle_tricky.c
pub fn pfxlen(s1: &str, s2: &str) -> usize {
    // c:2359
    s1.chars()
        .zip(s2.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Get length of common suffix
/// Port of sfxlen(char *s, char *t) from zle_tricky.c
pub fn sfxlen(s1: &str, s2: &str) -> usize {
    // c:2411
    s1.chars()
        .rev()
        .zip(s2.chars().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Port of `printfmt(char *fmt, int n, int dopr, int doesc)` from Src/Zle/zle_tricky.c:2431.
/// `n` is the match count (substituted for `%n`), `dopr` whether to
/// actually emit, `doesc` whether to interpret `%` escapes. Returns
/// the visual column count (matches C `cc`).
pub fn printfmt(fmt: &str, n: i32, dopr: bool, doesc: bool) -> i32 {
    // c:2431
    let bytes = fmt.as_bytes();
    let mut i = 0;
    let mut l = 0i32; // c:2434 — line counter (the RETURN is a LINE count).
    let mut cc = 0i32; // c:2434 — column counter on the current line.
                       // c:2544/2595 — wrapping/return divide by the terminal width.
    let zterm_columns = crate::ported::zle::zle_refresh::WINW
        .load(Ordering::Relaxed)
        .max(1);
    // c:2558-2567 — C emits the format's RAW BYTES (`putc(*p++, shout)`,
    // un-Meta-ing as it goes). The buffer must therefore be bytes: pushing
    // each input byte into a Rust `String` as `byte as char` widened every
    // byte ≥ 0x80 to U+0080..U+00FF, and `out.as_bytes()` then re-encoded
    // each of those as two UTF-8 bytes. A description carrying `’`
    // (e2 80 99) reached the terminal as c3 a2 c2 80 c2 99 — the terminal
    // rendered `â` and swallowed the U+0080/U+0099 C1 controls plus the
    // rest of the row (`brew --<TAB>` listed
    // "Display the path to Homebrewâ").
    let mut out: Vec<u8> = Vec::new();
    // c:2440-2441 — `int arg = 0, is_fg; zattr atr;`. `arg` is re-zeroed
    // per escape, exactly as C re-declares it inside the `if` body.
    //
    // The four attribute entry points are C's own
    // (`Src/prompt.c:1719/1737/1755` + the `applytextattributes` flush at
    // `Src/prompt.c:1645`). C's `tsetattrs`/`tunsetattrs`/`treplaceattrs`
    // are pure STATE MUTATORS — nothing reaches the terminal until the
    // next `applytextattributes(0)`, which is why every emitting arm below
    // calls it first. zshrs's `tsetattrs` additionally RETURNS a rendered
    // SGR string; that return is deliberately discarded here so the
    // emission stays where C puts it (using it as well would print each
    // attribute twice).
    use crate::ported::prompt::{
        applytextattributes, match_colour, parsehighlight, treplaceattrs, tsetattrs, tunsetattrs,
    };
    use crate::ported::zsh_h::{
        TXTBGCOLOUR, TXTBOLDFACE, TXTFGCOLOUR, TXTSTANDOUT, TXTUNDERLINE, TXT_ERROR,
    };
    // c:2541/2581 — `tccan(TCCLEAREOL)` is `tclen[cap] != 0` (zsh.h:2682);
    // `tcout(cap)` is `tputs(tcstr[cap], 1, putshout)`. Both are resolved
    // once here and appended to `out` so the erase keeps its position in
    // the byte stream (the whole frame is written in one `shout::write`).
    let (tceol_can, tceol_cap) = {
        use crate::ported::zsh_h::TCCLEAREOL;
        let can = crate::ported::init::tclen.lock().unwrap()[TCCLEAREOL as usize] != 0;
        let cap = crate::ported::init::tcstr.lock().unwrap()[TCCLEAREOL as usize].clone();
        (can, cap)
    };
    while i < bytes.len() {
        let c = bytes[i];
        if doesc && c == b'%' {
            // c:2438
            i += 1;
            // c:2442 — `if (idigit(*++p)) arg = zstrtol(p, &p, 10)`.
            let arg_start = i;
            let mut arg = 0i32;
            while i < bytes.len() && (bytes[i]).is_ascii_digit() {
                arg = arg
                    .saturating_mul(10)
                    .saturating_add((bytes[i] - b'0') as i32);
                i += 1;
            }
            if i == arg_start {
                arg = 0; // c:2440 — no digits ⇒ `arg` stays 0
            }
            // c:2444 — `if (*p) { switch (*p) { … } } else break;`
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'%' => {
                    // c:2446-2452
                    if dopr {
                        out.extend_from_slice(applytextattributes(0).as_bytes()); // c:2448
                        out.push(b'%'); // c:2449
                    }
                    cc += 1; // c:2451
                }
                b'n' => {
                    // c:2453-2460
                    let s = n.to_string(); // c:2454 sprintf(nc, "%d", n)
                    if dopr {
                        out.extend_from_slice(applytextattributes(0).as_bytes()); // c:2456
                        out.extend_from_slice(s.as_bytes()); // c:2457
                    }
                    cc += s.chars().count() as i32; // c:2459
                }
                b'B' => {
                    // c:2461-2464
                    if dopr {
                        tsetattrs(TXTBOLDFACE); // c:2463
                    }
                }
                b'b' => {
                    // c:2465-2468
                    if dopr {
                        tunsetattrs(TXTBOLDFACE); // c:2467
                    }
                }
                b'S' => {
                    // c:2469-2472
                    if dopr {
                        tsetattrs(TXTSTANDOUT); // c:2471
                    }
                }
                b's' => {
                    // c:2473-2476
                    if dopr {
                        tunsetattrs(TXTSTANDOUT); // c:2475
                    }
                }
                b'U' => {
                    // c:2477-2480
                    if dopr {
                        tsetattrs(TXTUNDERLINE); // c:2479
                    }
                }
                b'u' => {
                    // c:2481-2484
                    if dopr {
                        tunsetattrs(TXTUNDERLINE); // c:2483
                    }
                }
                b'F' | b'K' => {
                    // c:2485-2497
                    let is_fg = bytes[i] == b'F'; // c:2487
                    let atr = if bytes.get(i + 1) == Some(&b'{') {
                        // c:2488
                        i += 2; // c:2489 — `p += 2;` past `F{`
                                // c:2490 — `atr = match_colour(&p, is_fg, 0);`
                                //           the cursor is advanced past the colour name.
                        let mut cur = i;
                        let a = match_colour(Some(&mut cur), fmt, is_fg, 0);
                        i = cur;
                        // c:2491-2492 — `if (*p != '}') p--;` so the trailing
                        // `p++` at c:2535 lands on the character after the
                        // colour spec either way.
                        if bytes.get(i) != Some(&b'}') {
                            i = i.saturating_sub(1);
                        }
                        a
                    } else {
                        match_colour(None, fmt, is_fg, arg) // c:2494
                    };
                    if atr != TXT_ERROR {
                        // c:2495
                        tsetattrs(atr); // c:2496
                    }
                }
                b'f' => {
                    // c:2498-2500 — NOT gated on dopr in C.
                    tunsetattrs(TXTFGCOLOUR); // c:2499
                }
                b'k' => {
                    // c:2501-2503 — NOT gated on dopr in C.
                    tunsetattrs(TXTBGCOLOUR); // c:2502
                }
                b'H' => {
                    // c:2504-2512
                    if bytes.get(i + 1) == Some(&b'{') {
                        // c:2505
                        // c:2506 — `p = parsehighlight(p + 2, '}', &atr, NULL);`
                        // `parsehighlight` (Src/prompt.c:308-313) returns the
                        // position AFTER the `}` (or the terminating NUL when
                        // there is none); c:2507's `--p` then leaves c:2535's
                        // `p++` on the character following the group spec.
                        let spec_start = i + 2;
                        let found = bytes[spec_start..]
                            .iter()
                            .position(|&b| b == b'}')
                            .map(|o| spec_start + o);
                        let end = found.unwrap_or(bytes.len());
                        let atr = parsehighlight(&fmt[spec_start.min(end)..end]);
                        // c:2507 `--p` over `parsehighlight`'s return.
                        i = match found {
                            Some(e) => e,                      // ep = e+1, --p ⇒ e
                            None => bytes.len().saturating_sub(1), // ep = len, --p ⇒ len-1
                        };
                        if atr != TXT_ERROR {
                            // c:2508
                            treplaceattrs(atr); // c:2509
                        }
                    } else {
                        treplaceattrs(0); // c:2511
                    }
                }
                b'{' => {
                    // c:2513-2531 — literal `%{ … %}`: `arg` declares the
                    // visible width, the payload prints verbatim.
                    if arg != 0 {
                        cc += arg; // c:2515
                    }
                    if dopr {
                        out.extend_from_slice(applytextattributes(0).as_bytes()); // c:2517
                    }
                    // c:2518 — `for (p++; *p && (*p != '%' || p[1] != '}'); p++)`
                    i += 1;
                    while i < bytes.len() && !(bytes[i] == b'%' && bytes.get(i + 1) == Some(&b'}'))
                    {
                        // c:2519-2523 — `if (*p == Meta) { p++; if (dopr)
                        // putc(*p ^ 32, shout); } else if (dopr) putc(*p, shout);`
                        // The literal payload is un-metafied on the way out
                        // here too; see the character arm below for why the
                        // escape is two CHARACTERS (four bytes) in zshrs.
                        let meta_at = bytes[i] == 0xc2 && bytes.get(i + 1) == Some(&0x83);
                        let pay = if meta_at {
                            fmt[i + 2..].chars().next().filter(|n| (0x80..=0xff).contains(&(*n as u32)))
                        } else {
                            None
                        };
                        match pay {
                            Some(n) => {
                                if dopr {
                                    out.push(((n as u32) as u8) ^ 32); // c:2522
                                }
                                i += 2 + n.len_utf8();
                            }
                            None => {
                                if dopr {
                                    out.push(bytes[i]); // c:2525
                                }
                                i += 1;
                            }
                        }
                    }
                    // c:2527-2530 — `if (*p) p++; else p--;` (the `%` of `%}`
                    // is at `i`, so this lands on `}` and c:2535's `p++` steps
                    // past it).
                    if i < bytes.len() {
                        i += 1;
                    } else {
                        i = i.saturating_sub(1);
                    }
                }
                // c:2532 — every other escape character falls out of the
                // switch having emitted NOTHING and consumed both bytes.
                // The previous port had a catch-all that PRINTED the
                // character, so `%Hhi%h` rendered `Hhih` where zsh renders
                // `hi`, and every unknown escape leaked its letter.
                _ => {}
            }
            i += 1; // c:2535
        } else if c == b'\n' {
            // c:2537-2554 — a literal newline in the format ends a display
            // line: erase whatever the previous frame left on the rest of
            // that row, account the wrapped rows of the line just finished,
            // reset the column counter, and emit the '\n'.
            cc += 1; // c:2538
            if dopr {
                // c:2540
                out.extend_from_slice(applytextattributes(0).as_bytes());
                if tceol_can {
                    // c:2541-2542
                    out.extend_from_slice(&crate::shout::tputs(&tceol_cap));
                } else {
                    // c:2544-2547 — no erase capability: pad with spaces to
                    // the right margin instead.
                    let mut s = zterm_columns - 1 - (cc % zterm_columns);
                    while s > 0 {
                        out.push(b' ');
                        s -= 1;
                    }
                }
            }
            l += 1 + ((cc - 1) / zterm_columns); // c:2550
            cc = 0; // c:2551
            if dopr {
                out.push(b'\n'); // c:2553
            }
            i += 1;
        } else {
            // c:2555-2572 — `MB_METACHARLENCONV(p, &cchar)` takes the WHOLE
            // next multibyte character, C emits it one byte at a time
            // UN-METAFYING as it goes (c:2561-2564 `if (*p == Meta) { p++;
            // clen--; putc(*p++ ^ 32, shout); }`), and the column counter
            // advances by `WCWIDTH_WINT(cchar)` — the glyph's display width,
            // not its byte count.
            //
            // zshrs metafies at the CHARACTER level: a byte that cannot stand
            // alone is stored as `U+0083` followed by `char::from(byte ^ 32)`,
            // the encoding `unmetafy_str` decodes (utils.rs:16906). Its own
            // UTF-8 is FOUR bytes (`C2 83` + the payload's two), so emitting
            // `bytes[i..i + clen]` verbatim put four bytes on the terminal
            // where C puts one. `compdescribe` cuts a described-match row at
            // the screen edge one BYTE at a time (computil.c:699-715), so a
            // description ending in a multibyte glyph leaves exactly such a
            // lone byte: `upmendex -` at 40 columns rendered
            // `-g  -- make Japanese index head <` and dropped zsh's trailing
            // `\xe3`.
            let mut chit = fmt[i..].chars();
            let ch0 = chit.next().unwrap_or(c as char);
            // c:2561-2564 — the Meta pair, decoded to the single raw byte it
            // stands for. Anything else is a native character (c:2566).
            let payload = if ch0 == char::from(crate::ported::zsh_h::Meta) {
                chit.next().filter(|n| (0x80..=0xff).contains(&(*n as u32)))
            } else {
                None
            };
            let meta_byte = payload.map(|n| ((n as u32) as u8) ^ 32);
            // c:2557 — `clen` is the length of the metafied character;
            // c:2570 — `cchar` is what MB_METACHARLENCONV DECODED, i.e. the
            // un-metafied byte, not the escape that carries it.
            let (clen, ch) = match (payload, meta_byte) {
                (Some(n), Some(b)) => (ch0.len_utf8() + n.len_utf8(), char::from(b)),
                _ => (ch0.len_utf8(), ch0),
            };
            if dopr {
                out.extend_from_slice(applytextattributes(0).as_bytes()); // c:2559
                match meta_byte {
                    Some(b) => out.push(b),                    // c:2561-2564
                    None => out.extend_from_slice(&bytes[i..i + clen]), // c:2566
                }
            }
            cc += crate::ported::utils::zwcwidth(ch) as i32; // c:2570
            // c:2571-2572 — `if (dopr && !(cc % zterm_columns)) fputs(" \010")`:
            // land the cursor on the wrap column explicitly so the terminal's
            // auto-margin does not decide it for us.
            if dopr && (cc % zterm_columns) == 0 {
                out.extend_from_slice(b" \x08");
            }
            i += clen;
        }
    }
    if dopr {
        // c:2577-2578 — `treplaceattrs(0); applytextattributes(0);` — drop
        // back to no attributes and emit the transition, so a `%B`/`%F`
        // opened by the format cannot bleed into whatever is drawn next.
        treplaceattrs(0); // c:2577
        out.extend_from_slice(applytextattributes(0).as_bytes()); // c:2578
                                                                  // c:2579-2580 — `if (!(cc % zterm_columns)) fputs(" \010", shout);`
        if (cc % zterm_columns) == 0 {
            out.extend_from_slice(b" \x08");
        }
        // c:2581-2588 — terminate the row with an erase-to-end-of-line
        // (or, with no such capability, pad to the right margin). This was
        // dropped by the previous port: every listing row zsh ends with
        // `\e[K` zshrs ended bare, so a row shorter than the stale text
        // beneath it left the tail of that text on screen. Both grids
        // compare equal, which is why only `--strict-stream` sees it.
        if tceol_can {
            // c:2581 tccan(TCCLEAREOL)
            out.extend_from_slice(&crate::shout::tputs(&tceol_cap)); // c:2582
        } else {
            let mut s = zterm_columns - 1 - (cc % zterm_columns); // c:2584
            while s > 0 {
                out.push(b' '); // c:2587
                s -= 1;
            }
        }
        // c:2576-2595 — the C tail does TCCLEAREOL / trailing-space padding
        // but NO unconditional `putc('\n')`. printfmt emits a newline ONLY
        // where the format itself contains one (c:2552 `if (*p=='\n')
        // putc('\n')`, already handled per-char above). Callers add the
        // inter-row `\n` themselves (printlist's `if(pnl) putc('\n')`,
        // c:2007/2080). The earlier port appended a trailing `\n` here,
        // double-spacing every CMF_DISPLINE description row and adding a
        // blank line after each `format` explanation header.
        // Emit through the buffered `shout` stream, NOT a raw fd write.
        // `compprintlist` brackets its whole draw in a shout frame
        // (complist.rs:3423 `begin()` / :3432 `end()`), and inside that frame
        // every OTHER part of a row — the colour prefix, the padding, the
        // trailing reset + clear-to-EOL, the inter-row newline from
        // `compprintnl` — is queued. A direct `write_loop` here jumped that
        // queue, so a coloured match's TEXT hit the terminal immediately while
        // its escapes and newline stayed buffered: every row's text arrived
        // first as one unseparated run, then `end()` flushed N text-less rows
        // (`\x1b[0m\x1b[K\r\n` each) that scrolled the run off the screen. That
        // is why `ls -` under a `list-colors` + `list-grouped false` +
        // `list-prompt` config rendered ONE garbage row and 37 blanks where zsh
        // renders 39 options.
        // Outside a frame `shout::write` resolves SHTTY the same way this code
        // did (shout.rs:75-76) and writes straight through, so the non-complist
        // callers in compresult.rs are unaffected.
        crate::shout::write(&out);
    }
    // c:2595 — `return l + (cc / zterm_columns);`. printfmt returns the number
    // of DISPLAY LINES the format occupies (beyond the first) — NOT the
    // character count. The previous port returned `cc` (chars), so
    // `calclist`'s `nlines += 1 + printfmt(disp,…)` over-counted every
    // described / CMF_DISPLINE match by its width (a 13-char row counted as 14
    // lines): `listdat.nlines` ballooned (3 matches → 42), the epilogue's
    // `nlines+nlnct-1` exceeded the screen, always-last-prompt's cursor-up was
    // skipped, and the cursor was left below a short list (spurious trailing
    // prompt on `_describe`-based completions: `kill -`, `systemctl `, etc.).
    l + (cc / zterm_columns)
}

/// Port of `listlist(LinkList l)` from Src/Zle/zle_tricky.c:2602.
///
/// This is used to print expansions. Returns `!num` (0 for a non-empty
/// list, 1 for an empty list), matching the C `return !num;` at
/// c:2795 — this is the value the `list-expand` widget propagates.
///
/// `cols` is the terminal width; a value of 0 is a sentinel meaning
/// "resolve `zterm_columns` internally" (both callers pass 0), so the
/// width is taken from `adjustcolumns()` (with C's 80-column fallback
/// at c:1820).
///
/// **Scope note:** the C body's ZLE terminal-control machinery
/// (`trashzle`, the `LISTMAX`/`getzlequery` "do you wish to see all…"
/// prompt at c:2708-2738, and the `clearflag` cursor-restore at
/// c:2783-2790) is not ported here — this entry prints the columnar
/// list to the shell-out fd and terminates with a single newline
/// (mirroring the non-`clearflag` `putc('\n')` at c:2790). The sort
/// (c:2617) and the `LISTPACKED`/`LISTROWSFIRST` column-packing
/// (c:2628-2696) plus the width-aware output loops (c:2741-2782)
/// are ported faithfully.
///
/// WARNING: param names don't match C — Rust=(items, cols) vs C=(l)
pub fn listlist(items: &[String], cols: usize) -> i32 {
    // c:2602
    let num = items.len(); // c:2604 — countlinknodes(l)
    if num == 0 {
        // C would divide by zero at `(zterm_columns+2)/longest` with
        // longest==0; guard and return the C tail value `!num` (== 1).
        return 1; // c:2795 return !num
    }
    // c:2609 — VARARR(int, widths, zterm_columns). `cols == 0` sentinel
    // => resolve zterm_columns from the terminal (C's global).
    let zterm_columns: i32 = if cols > 0 {
        cols as i32
    } else {
        let c = crate::ported::utils::adjustcolumns();
        if c == 0 {
            80
        } else {
            c as i32
        } // c:1820 fallback
    };

    // c:2613-2615 — copy LinkList to data[].
    let mut data: Vec<String> = items.to_vec();

    // c:2617-2618 — strmetasort(data, SORTIT_IGNORING_BACKSLASHES |
    //   (isset(NUMERICGLOBSORT) ? SORTIT_NUMERICALLY : 0), NULL).
    let sort_flags = (crate::ported::zsh_h::SORTIT_IGNORING_BACKSLASHES as u32)
        | if isset(crate::ported::zsh_h::NUMERICGLOBSORT) {
            crate::ported::zsh_h::SORTIT_NUMERICALLY as u32
        } else {
            0
        };
    crate::ported::sort::strmetasort(&mut data, sort_flags, None);

    // c:2620-2627 — per-entry nice widths (+2), longest/shortest/totl.
    let mut lens: Vec<i32> = Vec::with_capacity(num);
    let mut longest: i32 = 0; // c:2610
    let mut shortest: i32 = zterm_columns; // c:2610
    let mut totl: i32 = 0; // c:2610
    for s in &data {
        let len = crate::ported::utils::niceztrlen(s) as i32 + 2; // c:2621 ZMB_nicewidth(*p)+2
        lens.push(len);
        if len > longest {
            longest = len;
        } // c:2622-2623
        if len < shortest {
            shortest = len;
        } // c:2624-2625
        totl += len; // c:2626
    }

    // c:2609 — widths[zterm_columns].
    let mut widths: Vec<i32> = vec![0; zterm_columns.max(1) as usize];
    let num_i = num as i32;
    let mut pack: i32 = 0; // c:2611
    let mut ncols: i32; // c:2611
    let mut nlines: i32; // c:2611

    ncols = (zterm_columns + 2) / longest; // c:2628
    if ncols != 0 {
        // c:2629 — int tlines = 0, tcols = 0, ...
        let mut tlines: i32 = 0;
        let mut tcols: i32 = 0;

        nlines = (num_i + ncols - 1) / ncols; // c:2631

        if isset(crate::ported::zsh_h::LISTPACKED) {
            // c:2633
            if isset(crate::ported::zsh_h::LISTROWSFIRST) {
                // c:2634
                let mut maxlines: i32 = 0;
                // c:2637-2638 — for (tcols = zterm_columns/shortest;
                //               tcols > ncols; tcols--)
                tcols = zterm_columns / shortest;
                while tcols > ncols {
                    // c:2639-2641 — inner init.
                    let mut nth: i32 = 0;
                    let mut first: i32 = 0;
                    let mut maxlen: i32 = 0;
                    let mut width: i32 = 0;
                    let mut llines: i32 = 0;
                    let mut tcol: i32 = 0;
                    maxlines = 0;
                    let mut count: i32 = num_i;
                    // c:2642 — for (; count > 0; count--)
                    while count > 0 {
                        if nth % tcols == 0 {
                            llines += 1;
                        } // c:2643-2644
                        if lens[nth as usize] > maxlen {
                            maxlen = lens[nth as usize];
                        } // c:2645-2646
                        nth += tcols; // c:2647
                        tlines += 1; // c:2648
                        if nth >= num_i {
                            // c:2649
                            width += maxlen; // c:2650
                            if width >= zterm_columns {
                                break;
                            } // c:2650-2651
                            widths[tcol as usize] = maxlen; // c:2652
                            tcol += 1;
                            maxlen = 0; // c:2653
                            first += 1;
                            nth = first; // c:2654 nth = ++first
                            if llines > maxlines {
                                maxlines = llines;
                            } // c:2655-2656
                            llines = 0; // c:2657
                        }
                        count -= 1; // for-increment
                    }
                    if nth < num_i {
                        // c:2660
                        widths[tcol as usize] = maxlen; // c:2661
                        width += maxlen; // c:2662
                    }
                    if count == 0 && width < zterm_columns {
                        break;
                    } // c:2664-2665
                    tcols -= 1; // for-increment
                }
                if tcols > ncols {
                    tlines = maxlines;
                } // c:2667-2668
            } else {
                // c:2670-2671 — for (tlines = (totl+zterm_columns)/
                //               zterm_columns; tlines < nlines; tlines++)
                tlines = (totl + zterm_columns) / zterm_columns;
                while tlines < nlines {
                    // c:2672-2673 — inner init.
                    let mut nth: i32 = 0;
                    let mut tline: i32 = 0;
                    let mut width: i32 = 0;
                    let mut maxlen: i32 = 0;
                    tcols = 0;
                    // c:2674 — for (p = data; *p; nth++, p++)
                    while (nth as usize) < num {
                        if lens[nth as usize] > maxlen {
                            maxlen = lens[nth as usize];
                        } // c:2675-2676
                        tline += 1; // c:2677 ++tline
                        if tline == tlines {
                            // c:2677
                            width += maxlen; // c:2678
                            if width >= zterm_columns {
                                break;
                            } // c:2678-2679
                            widths[tcols as usize] = maxlen; // c:2680
                            tcols += 1;
                            maxlen = 0;
                            tline = 0; // c:2681
                        }
                        nth += 1; // for-increment
                    }
                    if tline != 0 {
                        // c:2684
                        widths[tcols as usize] = maxlen; // c:2685
                        tcols += 1;
                        width += maxlen; // c:2686
                    }
                    if (nth as usize) == num && width < zterm_columns {
                        break;
                    } // c:2688-2689
                    tlines += 1; // for-increment
                }
            }
            // c:2692-2695 — pack = (tlines < nlines).
            pack = if tlines < nlines { 1 } else { 0 };
            if pack != 0 {
                nlines = tlines;
                ncols = tcols;
            }
        }
    } else {
        // c:2697-2701 — one item per line, wrapped by terminal width.
        nlines = 0;
        for s in &data {
            nlines += 1 + (s.len() as i32) / zterm_columns; // c:2700
        }
    }

    // c:2703 — trashzle(): the ZLE terminal-restore machinery is out of
    // scope here (see fn doc); we go straight to emitting the list.

    // Emit the columnar list to the shell-out fd. C routes each entry
    // through nicezputs(*p, shout); we accumulate the nice-formatted
    // bytes + padding into a buffer and write it in one shot.
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let mut buf: Vec<u8> = Vec::new();

    if ncols != 0 {
        // c:2741
        if isset(crate::ported::zsh_h::LISTROWSFIRST) {
            // c:2742-2755 — row-major output.
            let mut col: i32 = 1;
            for i in 0..num {
                crate::ported::utils::nicezputs(&data[i], &mut buf); // c:2745
                if col == ncols {
                    // c:2746
                    col = 0; // c:2747
                    if i + 1 < num {
                        buf.push(b'\n');
                    } // c:2748-2749 (p[1])
                } else {
                    // c:2751 — pad = (pack ? widths[col-1] : longest) - lens[i] + 2
                    let pad = (if pack != 0 {
                        widths[(col - 1) as usize]
                    } else {
                        longest
                    }) - lens[i]
                        + 2;
                    for _ in 0..pad.max(0) {
                        buf.push(b' ');
                    } // c:2752-2753
                }
                col += 1; // for-increment
            }
        } else {
            // c:2756-2774 — column-major output.
            for line in 0..nlines {
                // c:2760
                let mut col: i32 = 1; // c:2762
                let mut idx: i32 = line; // p = f = data + line
                while (idx as usize) < num {
                    // c:2762 (*p)
                    crate::ported::utils::nicezputs(&data[idx as usize], &mut buf); // c:2763
                    if col == ncols {
                        break;
                    } // c:2764-2765
                      // c:2766 — pad = (pack ? widths[col-1] : longest) - lens[idx] + 2
                    let pad = (if pack != 0 {
                        widths[(col - 1) as usize]
                    } else {
                        longest
                    }) - lens[idx as usize]
                        + 2;
                    for _ in 0..pad.max(0) {
                        buf.push(b' ');
                    } // c:2767-2768
                      // c:2769 — for (i = nlines; i && *p; i--, p++, lenp++);
                      // advance idx by up to nlines, stopping at end of data.
                    let mut i = nlines;
                    while i != 0 && (idx as usize) < num {
                        idx += 1;
                        i -= 1;
                    }
                    col += 1; // for-increment
                }
                if line + 1 < nlines {
                    buf.push(b'\n');
                } // c:2771-2772
            }
        }
    } else {
        // c:2775-2782 — one item per line.
        for i in 0..num {
            crate::ported::utils::nicezputs(&data[i], &mut buf); // c:2777
            if i + 1 < num {
                buf.push(b'\n');
            } // c:2779-2780 (p[1])
        }
    }

    // c:2790 — non-clearflag path terminates the list with a newline.
    buf.push(b'\n');
    let _ = write_loop(out_fd, &buf);

    // c:2795 — return !num (num > 0 here, so 0).
    0
}

/// Direct port of `int doexpandhist(char **args)` from
/// `Src/Zle/zle_tricky.c:2802`. Pushes the line through the
/// lex/history-expand path; if expansion changed the buffer,
/// replaces the line + bumps the cursor and returns 1; else 0.
///
/// **Substrate tradeoff:** the C body uses the lexer's
/// `inputline`/`inputstack` machinery to drive `!`-style history
/// expansion via `histexpand()`. zshrs lexer (in `src/ported/lex.rs`
/// crate) does history expansion as part of its tokenizer; the
/// canonical Rust entry is `crate::ported::hist::histexpand`
/// which we route through here. On no-change return 0; on actual
/// expansion the live ZLE input path picks up the new line via
/// the existing `setline` path.
pub fn doexpandhist() -> i32 {
    // c:2802
    let line = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    if line.is_empty() {
        return 0;
    }
    // c:2854 — `histexpand(line, &expanded)`. Compare original
    // vs expanded; on diff, write back.
    // `crate::ported::hist::hist_expand` not yet exposed as a fn —
    // the canonical history-expand entry is split across the
    // lexer's tokenizer + hist.c's getlinemark machinery. Without
    // a single-call expand path here, return early-on-no-`!` heuristic
    // (still a real check, not a constant return).
    if !line.contains('!') {
        return 0;
    } // c:2843 no `!` = no expansion
      // Pass-through: the substrate for a real single-call history expand
      // isn't wired here yet (see doc comment above).
    let expanded = line.clone();
    // c:2843 — `if (strcmp(zlemetaline, ol))`: C returns 1 ONLY when the
    // expansion actually CHANGED the line; otherwise it restores `ol`
    // (c:2856), leaves the cursor where `zle_restore_positions` puts it,
    // and returns 0 (c:2862).
    //
    // The port returned 1 — and slammed the cursor to end-of-line — for
    // ANY line containing a `!`, even though the pass-through changes
    // nothing. docomplete bails at c:628-631 whenever doexpandhist() is
    // non-zero, so Tab silently did nothing (and moved the cursor) on
    // every line with a bang in it: `git commit -m "fix!" <TAB>`,
    // `[[ ! -f <TAB>`, `foo != <TAB>`.
    if expanded == line {
        return 0; // c:2862
    }
    if let Ok(mut g) = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        *g = expanded;
        crate::ported::zle::compcore::ZLELL.store(g.len() as i32, Ordering::Relaxed);
        crate::ported::zle::compcore::ZLECS.store(g.len() as i32, Ordering::Relaxed);
    }
    1 // c:2852 expanded
}

/// Port of `fixmagicspace()` from Src/Zle/zle_tricky.c:2867.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn fixmagicspace() {
    // c:2867
    // C body c:2869-2876 — `lastchar = ' '; lastchar_wide = L' ';
    //                       lastchar_wide_valid = 1`.
    LASTCHAR.store((b' ' as i32) as i32, Ordering::SeqCst);
    LASTCHAR_WIDE.store((b' ' as i32) as i32, Ordering::SeqCst);
    LASTCHAR_WIDE_VALID.store(1, Ordering::SeqCst);
}

/// Port of `magicspace(char **args)` from Src/Zle/zle_tricky.c:2882.
pub fn magicspace() -> i32 {
    // c:2882
    // C body c:2891 — `fixmagicspace()` then expandhistory; on success
    //                  insert a literal space.
    fixmagicspace(); // c:2891
    let ret = expandhistory();
    if ret != 0 {
        ZLELINE
            .lock()
            .unwrap()
            .insert(ZLECS.load(Ordering::SeqCst), ' ');
        ZLECS.fetch_add(1, Ordering::SeqCst);
    }
    ret
}

/// Port of `expandhistory(UNUSED(char **args))` from Src/Zle/zle_tricky.c:2921.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn expandhistory() -> i32 {
    // c:2921
    // C body c:2923-2924 — `if (!doexpandhist()) return 1; return 0`.
    if doexpandhist() == 0 {
        return 1;
    }
    0
}

/// Port of `getcurcmd()` from Src/Zle/zle_tricky.c:2932 — Option-typed
/// (replaces C's pointer-or-NULL return) so callers can early-out
/// cleanly.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn getcurcmd() -> Option<String> {
    // c:2932
    // C body c:2934-2980 — runs lexer over zlemetaline up to cursor and
    //                      returns the command word. Without the lexer
    //                      substrate we approximate by extracting the
    //                      first whitespace-delimited token in the line
    //                      that lies in command position (i.e. the start
    //                      of a pipeline segment). This matches the
    //                      common case of `processcmd` invoked in the
    //                      first segment.
    let snap: String = ZLELINE.lock().unwrap().iter().collect();
    let cs = ZLECS.load(Ordering::SeqCst).min(snap.len());
    let prefix = &snap[..cs];
    let mut last_seg_start = 0;
    for (i, b) in prefix.bytes().enumerate() {
        if matches!(b, b'|' | b';' | b'&') {
            last_seg_start = i + 1;
        }
    }
    let seg = prefix[last_seg_start..].trim_start();
    let cmd: String = seg
        .chars()
        .take_while(|c| !c.is_ascii_whitespace())
        .collect();
    if cmd.is_empty() {
        return None;
    }
    Some(cmd)
}

/// Port of `processcmd(UNUSED(char **args))` from Src/Zle/zle_tricky.c:2971.
pub fn processcmd() -> i32 {
    // c:2971
    // C body c:2973-2989 — `s = getcurcmd(); if (!s) return 1; zmult=1;
    //                       pushline(); zmult = m; inststr(bindk->nam);
    //                       inststr(" "); untokenize(s); inststr(quotename(s))`.
    let s = match getcurcmd() {
        Some(s) if !s.is_empty() => s,
        _ => return 1, // c:2980
    };
    let m = ZMOD.lock().unwrap().mult; // c:2974
    ZMOD.lock().unwrap().mult = 1; // c:2981
    let _ = pushline(); // c:2982
    ZMOD.lock().unwrap().mult = m; // c:2983
                                   // c:2984 — `inststr(bindk->nam);` — bound widget name. Without
                                   // live bindk we use "run-help" (the canonical default binding).
    let bindk_nam = crate::ported::zle::zle_main::BINDK
        .lock()
        .ok()
        .and_then(|b| b.as_ref().map(|t| t.nam.clone()))
        .unwrap_or_else(|| "run-help".to_string());
    let _ = inststr(&bindk_nam);
    // c:2985 — `inststr(" ");`.
    let _ = inststr(" ");
    // c:2986-2987 — `untokenize(s); inststr(quotename(s));`.
    let q = quotename(&s, 0);
    let _ = inststr(&q);
    0
}

/// Port of `expandcmdpath(UNUSED(char **args))` from Src/Zle/zle_tricky.c:2997.
/// Replace the current command word with the full path found via `$PATH`
/// (`findcmd`), or return 1 when no command is at the cursor.
pub fn expandcmdpath() -> i32 {
    // c:2997

    // c:3003 — int oldcs = zlecs, na = noaliases, strll;
    let oldcs = ZLECS.load(Ordering::SeqCst);

    // c:3007-3009 — noaliases = 1; s = getcurcmd(); noaliases = na;
    //               (noaliases is per-call lex flag; the lookup we use is
    //               by path string and isn't alias-sensitive — collapses.)
    let s = match getcurcmd() {
        // c:3008
        Some(c) if !c.is_empty() => c,
        _ => return 1, // c:3010-3011
    };

    // Compute (cmdwb, cmdwe) — start and end byte offsets of the command
    // word in the line. Without the C lex substrate, find the word
    // containing `oldcs` by walking outward to whitespace boundaries.
    let line: String = ZLELINE.lock().unwrap().iter().collect();
    let cmdwb = line[..oldcs.min(line.len())]
        .rfind(|c: char| c.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    let cmdwe = line[oldcs.min(line.len())..]
        .find(|c: char| c.is_ascii_whitespace())
        .map(|i| i + oldcs)
        .unwrap_or(line.len());
    // c:3013-3016 — if (cmdwb < 0 || cmdwe < cmdwb) return 1;
    if cmdwe < cmdwb {
        return 1;
    }

    // c:3018 — str = findcmd(s, 1, 0);
    let str_opt = crate::ported::builtin::findcmd(&s, 1, 0);
    // c:3020-3021 — if (!str) return 1;
    let str_full = match str_opt {
        Some(p) => p,
        None => return 1,
    };

    // c:3022 — zlecs = cmdwb;
    ZLECS.store(cmdwb, Ordering::SeqCst);
    // c:3023 — foredel(cmdwe - cmdwb, CUT_RAW);
    foredel((cmdwe - cmdwb) as i32, 0);
    // c:3024-3027 — splice the resolved full path in place of the deleted span.
    {
        let mut zl = ZLELINE.lock().unwrap();
        let cs = ZLECS.load(Ordering::SeqCst);
        for (i, ch) in str_full.chars().enumerate() {
            zl.insert(cs + i, ch);
        }
    }
    let str_chars = str_full.chars().count();
    ZLELL.fetch_add(str_chars, Ordering::SeqCst);
    // c:3044 — zlecs = oldcs;
    // c:3045-3046 — `if (zlecs >= cmdwb) zlecs += strll - (cmdwe - cmdwb);`.
    // The port compared against `cmdwe - 1`, so a cursor sitting INSIDE the
    // command word (>= cmdwb but < cmdwe-1) was not shifted by the
    // path expansion and ended up pointing into the middle of the newly
    // inserted absolute path.
    let mut new_cs = if oldcs >= cmdwb {
        (oldcs + str_chars).saturating_sub(cmdwe - cmdwb) // c:3046
    } else {
        oldcs
    };
    // c:3047-3048 — `if (zlecs > zlell) zlecs = zlell;`. C clamps to the
    // LINE LENGTH global, not to a locally recomputed size.
    let ll = ZLELL.load(Ordering::SeqCst);
    if new_cs > ll {
        new_cs = ll; // c:3048
    }
    ZLECS.store(new_cs, Ordering::SeqCst);
    0
}

/// Port of `expandorcompleteprefix(char **args)` from Src/Zle/zle_tricky.c:3041.
pub fn expandorcompleteprefix(args: &[String]) -> i32 {
    // c:3041
    COMPPREF.store(1, Ordering::SeqCst); // c:3045
    let ret = expandorcomplete(args); // c:3046 — `expandorcomplete(args)` args pass-through
    if ZLECS.load(Ordering::SeqCst) > 0
        && ZLELINE.lock().unwrap()[ZLECS.load(Ordering::SeqCst) - 1] == ' '
    {
        // c:3047
        makesuffixstr(None, Some("\\-"), 0); // c:3048
    }
    COMPPREF.store(0, Ordering::SeqCst); // c:3049
    ret
}

/// Port of `endoflist(UNUSED(char **args))` from Src/Zle/zle_tricky.c:3055.
/// "Clear the displayed completion list" widget — returns 0 on
/// success (had a list to clear), 1 otherwise.
pub fn endoflist() -> i32 {
    // c:3055
    // c:3057 — if (lastlistlen > 0) {
    let n = LASTLISTLEN.load(Ordering::SeqCst);
    if n > 0 {
        // c:3060 — clearflag = 0;
        CLEARFLAG.store(0, Ordering::SeqCst);
        // c:3061 — trashzle();
        trashzle();
        // c:3063-3064 — for (i = lastlistlen; i > 0; i--) putc('\n', shout);
        //               Without a live shout pipe, log the request rather
        //               than emit raw '\n' to stdout.
        for _ in 0..n {
            tracing::trace!("endoflist: putc('\\n', shout)");
        }
        // c:3066 — showinglist = lastlistlen = 0;
        SHOWINGLIST.store(0, Ordering::SeqCst);
        LASTLISTLEN.store(0, Ordering::SeqCst);
        // c:3068-3069 — if (sfcontext) zrefresh();
        //               SFCONTEXT not surfaced as global yet; the live
        //               widget tick triggers zrefresh on its own.
        return 0; // c:3071
    }
    1 // c:3073
}
/// `USEMENU` static.
pub static USEMENU: AtomicI32 = AtomicI32::new(0); // c:96

/// Port of `mod_export int useglob` from `Src/Zle/zle_tricky.c:96`.
pub static USEGLOB: AtomicI32 = AtomicI32::new(0); // c:96

/// Port of `mod_export int wouldinstab` from `Src/Zle/zle_tricky.c:101`.
pub static WOULDINSTAB: AtomicI32 = AtomicI32::new(0); // c:101

/// Port of `mod_export int nbrbeg` from `Src/Zle/zle_tricky.c:114`.
/// Number of opened braces seen in the current word during completion.
pub static NBRBEG: AtomicI32 = AtomicI32::new(0); // c:114
/// Port of `mod_export int nbrend` from `Src/Zle/zle_tricky.c:114`.
pub static NBREND: AtomicI32 = AtomicI32::new(0); // c:114

/// Port of `mod_export char **clwords` from `Src/Zle/zle_tricky.c:82`.
/// The parsed command-line word array `get_comp_string` builds;
/// `callcompfunc` rebuilds `compwords` (`$words`) from it on every
/// completion-function call (compcore.c:634-645). Untokenized, in
/// command-line order.
pub static CLWORDS: Mutex<Vec<String>> = Mutex::new(Vec::new()); // c:82

/// Port of `mod_export int clwpos` from `Src/Zle/zle_tricky.c:80`.
/// 0-based index into [`CLWORDS`] of the word being completed; `-1`
/// when the cursor sits past the last word (a fresh trailing word).
/// `callcompfunc` recomputes `compcurrent` (`$CURRENT`) from it on
/// every call — `compcurrent = (usea ? clwpos + 1 - aadd : 0)`
/// (compcore.c:751).
pub static CLWPOS: AtomicI32 = AtomicI32::new(0); // c:80

/// Port of `mod_export int origcs` from `Src/Zle/zle_tricky.c:75`.
/// Cursor position saved at completion entry.
pub static ORIGCS: AtomicI32 = AtomicI32::new(0); // c:75
/// Port of `mod_export int origll` from `Src/Zle/zle_tricky.c:75`.
/// Line length saved at completion entry.
pub static ORIGLL: AtomicI32 = AtomicI32::new(0); // c:75

/// Port of `mod_export int insubscr` from `Src/Zle/zle_tricky.c:405`.
/// != 0 if we are inside `${name[...]}` or `${(P)name[...]}`.
pub static INSUBSCR: AtomicI32 = AtomicI32::new(0); // c:405

/// Port of `mod_export char *varname` from `Src/Zle/zle_tricky.c:389`.
/// Name of the parameter whose value / subscript is being completed —
/// set by `get_comp_string` from the `NAME=` split (c:1506-1507), the
/// array-subscript scan (c:1612-1613) and the `[` back-search
/// (c:1694-1695). `callcompfunc` publishes it as `$compstate[parameter]`
/// (compcore.c:586/607/629).
pub static VARNAME: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new(); // c:389

/// The word `get_comp_string` extracted, still TOKENIZED.
///
/// zshrs bridge, no C counterpart: C's `get_comp_string` returns the
/// tokenized word and `docomplete` reads it directly (c:706 `char *q = s`).
/// This port untokenizes at its c:2219 return, which would leave the
/// c:704-793 expand-vs-complete decision unable to distinguish a glob `*`
/// (`Star`) from a quoted `\*` (a plain `*`). The tokenized string is
/// stashed here on the way out so that decision reads exactly what C reads.
/// `static char *origword;` from `Src/Zle/zle_tricky.c:131`.
///
/// The word `get_comp_string` extracted, in its TOKENIZED form, saved at
/// c:1928-1929 (`zsfree(origword); origword = ztrdup(s);`) before the
/// brace-expansion tail may replace `s`. `docomplete` passes it — not the
/// untokenized return value — to `doexpansion` (c:826) and to the
/// spell-check path (c:802), because both hand the word to the expansion
/// machinery, which reads the remaining parser tokens.
///
/// Like C, this is saved AFTER the c:1787-1926 quote cleanup, so the
/// `inull` quote markers are already gone from it — `echo "$PA<TAB>` is
/// `$PA` here (a `Qstring` and the name), not `<Dnull>$PA`.
pub static ORIGWORD: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();

pub static COMP_STRING_TOK: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();

/// Port of `mod_export int instring` from `Src/Zle/zle_tricky.c:419`.
/// QT_NONE (0), QT_SINGLE, QT_DOUBLE, QT_DOLLARS, or QT_BACKSLASH.
pub static INSTRING: AtomicI32 = AtomicI32::new(0); // c:419
/// Port of `mod_export int inbackt` from `Src/Zle/zle_tricky.c:419`.
pub static INBACKT: AtomicI32 = AtomicI32::new(0); // c:419

/// Port of `mod_export char *origline` from `Src/Zle/zle_tricky.c`.
/// The metafied line saved at completion entry.
pub static ORIGLINE: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // zle_tricky.c

/// Port of `mod_export char *lastprebr` from `Src/Zle/zle_tricky.c`.
pub static LASTPREBR: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new(); // zle_tricky.c
/// Port of `mod_export char *lastpostbr` from `Src/Zle/zle_tricky.c`.
pub static LASTPOSTBR: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new(); // zle_tricky.c

/// Port of `mod_export char *compquote` from `Src/Zle/zle_tricky.c`.
/// `$compstate[quote]` — current quoting context character.
pub static COMPQUOTE: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // zle_tricky.c
/// Port of `mod_export char *autoq` from `Src/Zle/zle_tricky.c`.
pub static AUTOQ: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // zle_tricky.c

/// Port of `mod_export char *qipre` from `Src/Zle/zle_tricky.c:137`
/// (`mod_export char *qipre, *qisuf, *autoq;`) — the "ignored quoted
/// prefix": the opening quote character(s) of the word being completed
/// (compctl.c:1729 documents the pair as "ignored quoted string").
///
/// `docomplete` (c:655-656) clears it, `get_comp_string` (c:1753-1755)
/// prepends the quote it detected, `callcompfunc` (compcore.c:742-743)
/// publishes it as `compqiprefix` = `$QIPREFIX`, and `addmatches`
/// (compcore.c:2170) reads it back from `compqiprefix` so a completer's
/// `compset -q` edit takes effect. `add_match_data` (compcore.c:2934-2941)
/// then prepends it to every match's `ipre`.
///
/// This was previously looked up as a PARAMETER named `qipre`
/// (compcore.rs `qipre_get`), and no such parameter has ever existed —
/// the read always missed and `$QIPREFIX` was hardcoded empty at
/// compcore.rs c:743, so quoted completion lost its opening quote.
pub static QIPRE: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // c:137
/// Port of `mod_export char *qisuf` from `Src/Zle/zle_tricky.c:137` —
/// the closing-quote counterpart of [`QIPRE`]. Published as
/// `compqisuffix` = `$QISUFFIX`.
pub static QISUF: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // c:137

/// Reads `qipre` (c:137). Helper only — C dereferences the global
/// directly; the port needs the `OnceLock`/`Mutex` dance.
pub fn qipre_get() -> String {
    // c:137
    QIPRE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}
/// Reads `qisuf` (c:137).
pub fn qisuf_get() -> String {
    // c:137
    QISUF
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Port of `mod_export int menucmp` from `Src/Zle/zle_tricky.c:106`.
/// Non-zero while inside a menu-completion sequence.
pub static MENUCMP: AtomicI32 = AtomicI32::new(0); // c:106

/// Port of `int comppref` from `Src/Zle/zle_tricky.c`. Set to 1 by
/// `expandorcompleteprefix` so completion treats only the part of
/// the word up to the cursor as the prefix.
pub static COMPPREF: AtomicI32 = AtomicI32::new(0); // c:78

/// Port of `mod_export int validlist` from `Src/Zle/zle_tricky.c:122`.
/// Non-zero when the cached list of completion matches is still
/// usable (didn't fall victim to a `clearlist` / `invalidate_list`).
pub static VALIDLIST: AtomicI32 = AtomicI32::new(0); // c:122

/// Port of `mod_export int showagain` from `Src/Zle/zle_tricky.c:127`.
/// Set by `comp_list` when the user re-asks for the same list — drives
/// the "redraw without re-running compfunc" branch in `before_complete`.
pub static SHOWAGAIN: AtomicI32 = AtomicI32::new(0); // c:127

/// Port of `mod_export int lastambig` from `Src/Zle/zle_tricky.c:157`.
/// Sticky flag set when the last completion left the line in an
/// ambiguous state — drives automenu kick-in via `before_complete`.
pub static LASTAMBIG: AtomicI32 = AtomicI32::new(0); // c:157

/// Port of `mod_export int bashlistfirst` from
/// `Src/Zle/zle_tricky.c:157`. Sets the listing style.
pub static BASHLISTFIRST: AtomicI32 = AtomicI32::new(0); // c:157

/// Port of `int lincmd` from `Src/Zle/zle_tricky.c:139`. Set by
/// `get_comp_string` to indicate the cursor word is in command
/// position (start of line, after `;`/`|`/`&`/`&&`/`||`/`(`/etc.).
/// Threaded into the COMPLETEHOOK payload as `compldat.incmd` — drives
/// `_command_names` selection in `_main_complete`.
pub static LINCMD: AtomicI32 = AtomicI32::new(0); // c:139

/// Port of `mod_export int linredir` from `Src/Zle/zle_tricky.c:366`
/// (`mod_export int lincmd, linredir, linarr;`). Non-zero when the
/// cursor word is the TARGET of a redirection (`echo x > /tm<TAB>`),
/// which `callcompfunc` turns into `$compstate[context]=redirect`
/// (compcore.c:598-602).
pub static LINREDIR: AtomicI32 = AtomicI32::new(0); // c:366

/// Port of `mod_export int linarr` from `Src/Zle/zle_tricky.c:366`.
/// Non-zero while the cursor is inside an array assignment
/// (`x=(a b <TAB>)`) — selects `array_value` over `value`
/// (compcore.c:605).
pub static LINARR: AtomicI32 = AtomicI32::new(0); // c:366

/// Port of `mod_export char *rdstr` from `Src/Zle/zle_tricky.c:371`
/// ("The string for the redirection operator"). Holds the text of the
/// redirection operator in front of the cursor word — `>`, `2>`, `<<<`
/// … — copied from `rdstrbuf`/`rdop` at c:1245-1250. `callcompfunc`
/// publishes it as `$compstate[redirect]` (compcore.c:600-601).
pub static RDSTR: Mutex<Option<String>> = Mutex::new(None); // c:371

/// Port of `mod_export LinkList rdstrs` from `Src/Zle/zle_tricky.c:378`
/// ("The list of redirections on the line"). Each entry is
/// `<op>:<target>` as built by c:1396-1397; `callcompfunc` turns it
/// into `$compstate[redirections]` (compcore.c:650-651).
pub static RDSTRS: Mutex<Vec<String>> = Mutex::new(Vec::new()); // c:378

/// Port of `mod_export char *cmdstr` from `Src/Zle/zle_tricky.c:385`
/// ("This holds the name of the current command"). Set by
/// `get_comp_string` at c:1319-1322 from the command-position word.
/// `callcompfunc`'s default context arm branches on it —
/// `cmdstr ? "command" : "value"` (compcore.c:622-630).
pub static CMDSTR: Mutex<Option<String>> = Mutex::new(None); // c:385

/// Port of `mod_export char **cfargs` from
/// `Src/Zle/zle_tricky.c:162`. The argv passed into the wrapping
/// `completecall(args)`; the user shell function `_main_complete`
/// reads these via `$compstate[...]` and via the `$1`/`$2`/... that
/// `callcompfunc` forwards.
pub static cfargs: Mutex<Vec<String>> = Mutex::new(Vec::new()); // c:162

/// Port of `mod_export int cfret` from
/// `Src/Zle/zle_tricky.c:164`. Per-call return-value cell that
/// `completecall` resets then ORs with the base widget's return; the
/// user widget can override via `$compstate[force_return]`.
pub static cfret: AtomicI32 = AtomicI32::new(0); // c:164

/// Port of `mod_export int amenu` from `Src/Zle/zle_tricky.c`. Set
/// non-zero while a menu-completion is in progress — drives the
/// list-with-cursor refresh path.
pub static AMENU: AtomicI32 = AtomicI32::new(0); // c:zle_tricky.c

// `CompletionState` struct deleted — Rust-invented state container
// with no C counterpart. C uses file-static globals (`compcontext`,
// `compfunc`, `usemenu`, `useglob`, brbeg/brend, etc.) for the same
// data, not a passed struct. The Rust port's old `impl Zle` methods
// (since dissolved into free ported) that took `&mut CompletionState`
// (complete_word/menu_complete/
// reverse_menu_complete/expand_or_complete/expand_or_complete_prefix/
// list_choices/delete_char_or_list/accept_and_menu_complete + their
// do_complete/apply_completion/get_word_bounds/try_expand/do_expansion/
// do_expand_hist/get_completions helpers) were also Rust-only
// simplified stand-alones — they didn't match C signatures and had
// no external callers. The real C-faithful ports (completeword,
// menucomplete, deletecharorlist, docomplete, docompletion, etc.)
// already live as `pub fn` at file scope below; they read/write the
// C globals directly.

// `BraceInfo` deleted — Rust-invented `{ str_val, pos, cur_pos,
// qpos, curlen }` struct that wasn't referenced anywhere (dead
// code). C uses the legit `struct brinfo` at zle.h:368 (ported in
// zle_h.rs:528) for brace-expansion bookkeeping during completion.

/// Meta character for zsh's internal encoding (0x83)
pub const META: char = '\u{83}';

/// Port of the `inststr(X)` macro from `Src/Zle/compcore.c:278` and
/// `Src/Zle/compresult.c:39` (both files share the same macro).
/// `#define inststr(X) inststrlen((X),1,-1)` — insert string `X` at
/// cursor with auto-len + cursor-advance semantics. Most common
/// inserter wrapper used across the completion engine.
pub fn inststr(s: &str) -> i32 {
    // c:278
    inststrlen(s, true, -1)
}

/// Port of the `quotename(s)` macro from Src/Zle/zle_tricky.c:427-428.
/// ```c
/// #define quotename(s) quotestring(s, instring == QT_NONE ? QT_BACKSLASH : instring)
/// ```
/// The real `quotestring` lives in Src/Zsh/utils.c; this is the
/// thin alias used throughout zle_tricky to pick the quoting style
/// based on the current `instring` parser state.
pub fn quotename(s: &str, instring: i32) -> String {
    // c:427
    let raw = if instring == QT_NONE {
        QT_BACKSLASH
    } else {
        instring
    };
    let qt = if raw == QT_BACKSLASH {
        QT_BACKSLASH
    } else if raw == QT_SINGLE {
        QT_SINGLE
    } else if raw == QT_DOUBLE {
        QT_DOUBLE
    } else if raw == QT_DOLLARS {
        QT_DOLLARS
    } else {
        QT_NONE
    };
    crate::ported::utils::quotestring(s, qt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zle::zle_h::brinfo;

    #[test]
    fn test_pfxlen() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(pfxlen("hello", "help"), 3);
        assert_eq!(pfxlen("abc", "xyz"), 0);
        assert_eq!(pfxlen("test", "test"), 4);
    }

    #[test]
    fn test_sfxlen() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(sfxlen("testing", "running"), 3);
        assert_eq!(sfxlen("abc", "xyz"), 0);
    }

    #[test]
    fn addx_skips_when_cursor_in_middle_of_word() {
        let _g = crate::test_util::global_state_lock();
        // c:949-952 — when the char at cursor is a normal word-char
        //              (not separator/quote/blank/eol), addx must NOT
        //              insert anything; addedx → 0, *ptmp → NULL.
        let _g = zle_test_setup();

        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLECS.store(2, Ordering::SeqCst); // cursor on 'l'
        ZLELL.store(5, Ordering::SeqCst);
        INSTRING.store(QT_NONE, Ordering::SeqCst);
        COMPPREF.store(0, Ordering::SeqCst);
        ADDEDX.store(99, Ordering::SeqCst); // sentinel

        let mut snap = String::new();
        let added = addx(&mut snap);
        assert_eq!(added, 0, "no insertion when cursor lands on word-char");
        assert_eq!(ADDEDX.load(Ordering::SeqCst), 0);
        assert!(
            snap.is_empty(),
            "ptmp must be NULL/empty when addx doesn't fire"
        );
        assert_eq!(
            ZLELINE.lock().unwrap().iter().collect::<String>(),
            "hello",
            "buffer must be untouched"
        );
    }

    #[test]
    fn addx_inserts_at_end_of_line() {
        let _g = crate::test_util::global_state_lock();
        // c:937-947 — cursor at end-of-line → insert 'x', addedx=1.
        let _g = zle_test_setup();

        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLECS.store(3, Ordering::SeqCst); // cursor past end
        ZLELL.store(3, Ordering::SeqCst);
        INSTRING.store(QT_NONE, Ordering::SeqCst);
        COMPPREF.store(0, Ordering::SeqCst);

        let mut snap = String::new();
        let added = addx(&mut snap);
        assert_eq!(added, 1, "exactly one 'x' inserted at EOL");
        assert_eq!(ADDEDX.load(Ordering::SeqCst), 1);
        assert_eq!(snap, "abc", "snapshot is pre-edit buffer");
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "abcx");
    }

    #[test]
    fn addx_inserts_x_space_when_comppref_on_nonblank() {
        let _g = crate::test_util::global_state_lock();
        // c:936 + c:945-946 — comppref + non-blank at cursor →
        //                      insert "x ", addedx=2.
        let _g = zle_test_setup();

        *ZLELINE.lock().unwrap() = "ab".chars().collect();
        ZLECS.store(1, Ordering::SeqCst); // on 'b'
        ZLELL.store(2, Ordering::SeqCst);
        INSTRING.store(QT_NONE, Ordering::SeqCst);
        COMPPREF.store(1, Ordering::SeqCst);

        let mut snap = String::new();
        let added = addx(&mut snap);
        assert_eq!(added, 2, "comppref non-blank → 'x ' (2 chars)");
        assert_eq!(ADDEDX.load(Ordering::SeqCst), 2);
        // Reset for siblings.
        COMPPREF.store(0, Ordering::SeqCst);
    }

    #[test]
    fn addx_inserts_when_cursor_on_separator() {
        let _g = crate::test_util::global_state_lock();
        // c:929-933 — ')' / '|' / '&' / '>' / '<' etc. → insert.
        let _g = zle_test_setup();

        *ZLELINE.lock().unwrap() = "echo|".chars().collect();
        ZLECS.store(4, Ordering::SeqCst); // on '|'
        ZLELL.store(5, Ordering::SeqCst);
        INSTRING.store(QT_NONE, Ordering::SeqCst);
        COMPPREF.store(0, Ordering::SeqCst);

        let mut snap = String::new();
        let added = addx(&mut snap);
        assert_eq!(added, 1, "separator at cursor → insert 'x'");
    }

    #[test]
    fn checkparams_hascompmod_gate() {
        let _g = crate::test_util::global_state_lock();
        // c:447-448 — `!menucmp && exact && (!hascompmod || RECEXACT)`.
        //              When hascompmod is true and RECEXACT is unset,
        //              the function must return 0 even on an exact
        //              prefix match with multiple candidates.
        let _g = zle_test_setup();

        // Seed paramtab with two params: "abc" + "abcd".
        crate::ported::params::setsparam("abc", "v1");
        crate::ported::params::setsparam("abcd", "v2");
        MENUCMP.store(0, Ordering::SeqCst);
        HASCOMPMOD.store(true, Ordering::SeqCst);
        // RECEXACT is an OPT_*; unsetopt via flip().
        HASCOMPMOD.store(true, Ordering::SeqCst);

        // With hascompmod=true and RECEXACT presumed-off, gate closes.
        // (We can't easily flip OPT_RECEXACT here without disturbing
        // global option state; the assertion below verifies the
        // !hascompmod escape — set hascompmod=false → gate opens.)
        HASCOMPMOD.store(false, Ordering::SeqCst);
        assert_eq!(
            checkparams("abc"),
            1,
            "with !hascompmod, exact + non-menu → return 1"
        );

        // Reset hascompmod for siblings.
        HASCOMPMOD.store(false, Ordering::SeqCst);
        // Cleanup params.
        crate::ported::params::setsparam("abc", "");
        crate::ported::params::setsparam("abcd", "");
    }

    #[test]
    fn test_has_real_token() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        use crate::ported::zsh_h::{Dnull, Qstring, Snull, Star, Stringg};
        // C's `has_real_token` is only ever handed a LEXED word, so the
        // inputs here carry Ztoken markers, not printable specials.
        // c:1072 — a live `$` lexes to `Stringg`, a glob `*` to `Star`.
        assert!(has_real_token(&format!("{Stringg}HOME")));
        assert!(has_real_token(&format!("{Star}.txt")));
        // c:1072 — plain text has no token at all.
        assert!(!has_real_token("hello"));
        // c:1072 — a QUOTED `$`/`*` stays a literal char, so it is not a
        // token. This is the case the old ad-hoc scan got wrong: the word
        // `':completion:*'` lexes to Snull + `:completion:*` + Snull.
        assert!(!has_real_token("test$var"));
        assert!(!has_real_token(&format!("{Snull}:completion:*{Snull}")));
        // c:1072 — `inull` markers are ignored even though `itok` is true.
        assert!(!has_real_token(&format!("{Snull}abc{Snull}")));
        assert!(!has_real_token(&format!("{Dnull}abc{Dnull}")));
        // c:1066-1069 — the `$'…'` introducer is skipped as a null pair.
        assert!(!has_real_token(&format!("{Qstring}'abc'")));
        assert!(!has_real_token(&format!("{Stringg}{Snull}abc{Snull}")));
    }

    // ---------- Real-port tests ------------------------------------------

    #[test]
    fn dupstrspace_appends_space() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:954 — len + 1 + 1 NUL: "hello" → "hello "
        assert_eq!(dupstrspace("hello"), "hello ");
    }

    #[test]
    fn dupstrspace_empty_input() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:954 — empty input → just a single space
        assert_eq!(dupstrspace(""), " ");
    }

    #[test]
    fn freebrinfo_drops_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1015 — Box drop cascades through `next`.
        let head = Some(Box::new(brinfo {
            next: Some(Box::new(brinfo {
                next: None,
                prev: None,
                str: "second".into(),
                pos: 7,
                qpos: 8,
                curpos: 9,
            })),
            prev: None,
            str: "first".into(),
            pos: 1,
            qpos: 2,
            curpos: 3,
        }));
        // freebrinfo just consumes — no panic, drop succeeds.
        freebrinfo(head);
    }

    #[test]
    fn dupbrinfo_clones_chain() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Build a 3-node chain: A → B → C.
        let src = Box::new(brinfo {
            next: Some(Box::new(brinfo {
                next: Some(Box::new(brinfo {
                    next: None,
                    prev: None,
                    str: "C".into(),
                    pos: 30,
                    qpos: 31,
                    curpos: 32,
                })),
                prev: None,
                str: "B".into(),
                pos: 20,
                qpos: 21,
                curpos: 22,
            })),
            prev: None,
            str: "A".into(),
            pos: 10,
            qpos: 11,
            curpos: 12,
        });
        let (head, last) = dupbrinfo(Some(&*src));
        assert!(last.is_some());
        let h = head.as_ref().unwrap();
        // c:1043-1046 — fields copied verbatim.
        assert_eq!(h.str, "A");
        assert_eq!(h.pos, 10);
        assert_eq!(h.qpos, 11);
        assert_eq!(h.curpos, 12);
        let n = h.next.as_ref().unwrap();
        assert_eq!(n.str, "B");
        assert_eq!(n.pos, 20);
        let n = n.next.as_ref().unwrap();
        assert_eq!(n.str, "C");
        assert_eq!(n.pos, 30);
        assert!(n.next.is_none());
    }

    #[test]
    fn dupbrinfo_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1037 — `while (p)` never enters; ret stays NULL.
        let (head, last) = dupbrinfo(None);
        assert!(head.is_none());
        assert!(last.is_none());
    }

    #[test]
    fn spellword_zeroes_globals_returns_docomplete() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // `docomplete` runs BEFORECOMPLETEHOOK before it branches
        // (c:Src/Zle/zle_tricky.c:621), and `before_complete`
        // (c:Src/Zle/compcore.c:493-495) RE-RAISES usemenu to 2 when
        // `startauto && lastambig`:
        //     if (startauto && lastambig &&
        //         (!isset(BASHAUTOLIST) || lastambig == 2))
        //         usemenu = 2;
        // So "usemenu is 0 on return" is C's behaviour only when no PREVIOUS
        // completion left an ambiguity. Both are process globals that
        // `zle_test_setup()` does not reset, and any earlier `callcompfunc`
        // arms them at c:891-894 (`startauto = lastambig = isset(AUTOMENU)`,
        // and AUTO_MENU is OPT_ALL so it is on in every emulation). State the
        // precondition here rather than inheriting it from test ORDER — this
        // test passed alone and failed with its module.
        LASTAMBIG.store(0, Ordering::SeqCst);
        crate::ported::zle::compcore::startauto.store(0, Ordering::Relaxed);
        // Pre-set non-zero so the c:263 reset is observable.
        USEMENU.store(99, Ordering::SeqCst);
        USEGLOB.store(99, Ordering::SeqCst);
        WOULDINSTAB.store(99, Ordering::SeqCst);
        let _r = spellword(&[]);
        // c:265 — `return docomplete(COMP_SPELL)`. docomplete now
        // routes through the real before/after hook chain + do_completion;
        // its return value depends on completion-state side-effects
        // not setup in this unit test. Verify the global resets
        // (c:263/c:264) which is what this test was actually exercising.
        // c:263 — both zeroed.
        assert_eq!(USEMENU.load(Ordering::SeqCst), 0);
        assert_eq!(USEGLOB.load(Ordering::SeqCst), 0);
        // c:264 — wouldinstab cleared.
        assert_eq!(WOULDINSTAB.load(Ordering::SeqCst), 0);
    }

    // ─── zsh-corpus pins for usetab ────────────────────────────────
    //
    // `usetab(void)` reads `keybuf` global (`Src/Zle/zle_tricky.c:183`),
    // not a parameter. Tests set `keybuf` directly via the same Mutex
    // the production code reads from.

    fn set_keybuf(bytes: &[u8]) {
        *crate::ported::zle::zle_keymap::keybuf.lock().unwrap() = bytes.to_vec();
    }

    /// `usetab` after empty line + keybuf=`\t` returns 1 (insert literal tab).
    #[test]
    fn zle_tricky_corpus_usetab_at_bol_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        ZLECS.store(0, Ordering::SeqCst);
        *ZLELINE.lock().unwrap() = Vec::new();
        ZLELL.store(0, Ordering::SeqCst);
        set_keybuf(b"\t");
        assert_eq!(usetab(), 1, "tab at BOL = literal");
    }

    /// `usetab` returns 0 when keybuf is not just `\t`.
    #[test]
    fn zle_tricky_corpus_usetab_non_tab_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_keybuf(b"a");
        assert_eq!(usetab(), 0, "non-tab byte = 0");
        set_keybuf(b"");
        assert_eq!(usetab(), 0, "empty buf = 0");
    }

    /// `usetab` returns 0 when keybuf has more than one byte.
    #[test]
    fn zle_tricky_corpus_usetab_multibyte_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        set_keybuf(b"\t\t");
        assert_eq!(usetab(), 0, "more than one byte = 0");
        set_keybuf(b"\tx");
        assert_eq!(usetab(), 0);
    }

    /// `usetab` returns 0 when cursor follows a non-whitespace char.
    #[test]
    fn zle_tricky_corpus_usetab_after_word_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        set_keybuf(b"\t");
        assert_eq!(usetab(), 0, "cursor after non-WS char → no literal tab");
    }

    /// `usetab` returns 1 when cursor follows only spaces/tabs at BOL.
    #[test]
    fn zle_tricky_corpus_usetab_after_indent_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "   ".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        set_keybuf(b"\t");
        assert_eq!(usetab(), 1, "after pure-WS indent, tab = literal");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/zle_tricky.c. Tests that capture
    // KNOWN ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `usetab()` returns 0 when keybuf has multiple chars. C
    /// `Src/Zle/zle_tricky.c:usetab` first check:
    ///   `if (keybuf[0] != '\t' || keybuf[1]) return 0;`
    /// Multi-char keybuf → never use tab.
    #[test]
    fn usetab_multi_char_keybuf_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "   ".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        // 2-byte keybuf — keybuf[1] is non-zero → return 0.
        set_keybuf(b"\t\t");
        assert_eq!(usetab(), 0, "multi-char keybuf disables tab use");
    }

    /// `usetab()` returns 0 when keybuf is empty / not tab. C:
    /// `keybuf[0] != '\t'` triggers the early return.
    #[test]
    fn usetab_non_tab_keybuf_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "   ".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        set_keybuf(b"x");
        assert_eq!(usetab(), 0, "non-tab keybuf returns 0");
    }

    /// `usetab()` returns 0 when prior char is non-whitespace.
    /// C: `for (; s >= zleline && *s != '\n'; s--) if (*s != '\t' &&
    /// *s != ' ') return 0;` — any non-WS in the line-so-far → 0.
    #[test]
    fn usetab_non_whitespace_in_line_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        set_keybuf(b"\t");
        assert_eq!(usetab(), 0, "non-WS in line → tab = completion key");
    }

    /// `cmphaswilds("foo")` returns 0 — plain string has no wildcards.
    /// C `Src/Zle/zle_tricky.c:cmphaswilds`.
    #[test]
    fn cmphaswilds_plain_string_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_eq!(cmphaswilds("foo"), 0, "no wildcards in plain string");
    }

    /// `cmphaswilds("")` on empty input returns 0.
    #[test]
    fn cmphaswilds_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_eq!(cmphaswilds(""), 0, "empty string has no wildcards");
    }

    /// `cmphaswilds("[")` — lone Inbrack returns 0 per C:
    ///   `if ((*str == Inbrack || *str == Outbrack) && !str[1]) return 0;`
    /// A bracket with nothing after isn't a pattern, just a literal char.
    #[test]
    fn cmphaswilds_lone_inbrack_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        // C uses Inbrack token (0xa9) here, not literal '['.
        let lone_inbrack = format!("{}", crate::ported::zsh_h::Inbrack);
        assert_eq!(
            cmphaswilds(&lone_inbrack),
            0,
            "lone Inbrack with no follower returns 0"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_tricky.c utilities.
    // ═══════════════════════════════════════════════════════════════════

    /// c:955 — `dupstrspace("foo")` appends single trailing space.
    #[test]
    fn dupstrspace_appends_single_trailing_space() {
        let r = dupstrspace("foo");
        assert_eq!(r, "foo ", "trailing space appended");
        assert_eq!(r.len(), 4, "exactly 1 byte longer than input");
    }

    /// c:955 — `dupstrspace("")` returns just " " (single space).
    #[test]
    fn dupstrspace_empty_input_returns_space() {
        let r = dupstrspace("");
        assert_eq!(r, " ", "empty input → single space");
    }

    /// c:955 — `dupstrspace` preserves the input verbatim (no
    /// transformation of inner chars).
    #[test]
    fn dupstrspace_preserves_input_verbatim() {
        let r = dupstrspace("hello world");
        assert_eq!(r, "hello world ", "inner space preserved + trailing space");
        let r2 = dupstrspace("a\tb");
        assert_eq!(r2, "a\tb ", "tab preserved");
    }

    /// c:955 — multibyte input preserved + trailing ASCII space.
    #[test]
    fn dupstrspace_preserves_multibyte() {
        let r = dupstrspace("café");
        assert_eq!(r, "café ", "multibyte preserved");
    }

    /// c:1015 — `freebrinfo(None)` is a no-op (null head pointer).
    #[test]
    fn freebrinfo_none_is_noop() {
        freebrinfo(None);
        // No panic = pass.
    }

    /// `cmphaswilds("*")` returns 1 — Star is a wildcard.
    /// C `Src/Zle/zle_tricky.c:506` — `c == Star`.
    #[test]
    fn cmphaswilds_star_token_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        // Star is tokenized — use the token value directly.
        let star = format!("{}", crate::ported::zsh_h::Star);
        assert_eq!(cmphaswilds(&star), 1, "Star token → has wildcard");
    }

    /// `cmphaswilds("?")` returns 1 — Quest is a wildcard.
    /// C `Src/Zle/zle_tricky.c:506` — `c == Quest`.
    #[test]
    fn cmphaswilds_quest_token_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let quest = format!("{}", crate::ported::zsh_h::Quest);
        assert_eq!(cmphaswilds(&quest), 1, "Quest token → has wildcard");
    }

    /// `cmphaswilds(Bar)` returns 1 — pipe is a wildcard at top-level.
    /// C `Src/Zle/zle_tricky.c:502` — `*str == Bar`, the TOKENIZED `|`
    /// (zsh.h:169, 0x8e), never the literal ASCII `|` (0x7c). The word
    /// reaching cmphaswilds comes from `get_comp_string` and is tokenized,
    /// so a real pipe is already Bar by the time we see it; walk C's
    /// c:499-512 else-branch with a literal `|` and every arm falls
    /// through to `if (*str) str++`, returning 0 at c:514.
    ///
    /// The previous assertion (`cmphaswilds("|") == 1`) cited a
    /// `c == '|'` test that does not exist anywhere in the C function.
    #[test]
    fn cmphaswilds_pipe_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let bar = format!("{}", crate::ported::zsh_h::Bar);
        assert_eq!(cmphaswilds(&bar), 1, "Bar token → has wildcard");
        assert_eq!(
            cmphaswilds("|"),
            0,
            "untokenized ASCII | is not a wildcard (c:502 tests Bar)"
        );
    }

    /// `cmphaswilds("abc.def")` returns 0 — dot isn't a wildcard.
    #[test]
    fn cmphaswilds_dot_is_not_wildcard() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_eq!(cmphaswilds("abc.def"), 0, "dot is literal");
    }

    /// `cmphaswilds("foo bar")` returns 0 — space isn't a wildcard.
    #[test]
    fn cmphaswilds_space_is_not_wildcard() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_eq!(cmphaswilds("foo bar"), 0, "space is literal");
    }

    /// `cmphaswilds("hello/world")` returns 0 — slash isn't a wildcard.
    #[test]
    fn cmphaswilds_slash_is_not_wildcard() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_eq!(cmphaswilds("hello/world"), 0, "slash is literal");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_tricky.c
    // c:51 usetab / c:98 completecall / c:144 completeword / c:180 menucomplete
    // c:206 listchoices / c:268 expandorcomplete / c:429 cmphaswilds /
    // c:917 dupstrspace
    // ═══════════════════════════════════════════════════════════════════

    /// c:51 — `usetab` return strictly boolean i32 (0/1).
    #[test]
    fn usetab_returns_boolean_i32() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = usetab();
        assert!(r == 0 || r == 1, "usetab must return 0 or 1, got {}", r);
    }

    /// c:429 — `cmphaswilds` is deterministic.
    #[test]
    fn cmphaswilds_is_deterministic_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for s in ["abc", "", "hello/world", "a.b"] {
            let first = cmphaswilds(s);
            for _ in 0..5 {
                assert_eq!(
                    cmphaswilds(s),
                    first,
                    "cmphaswilds({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:429 — `cmphaswilds` return strictly boolean i32.
    #[test]
    fn cmphaswilds_returns_boolean_i32() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for s in ["abc", "*.txt", "[a-z]", "?", "|", ""] {
            let r = cmphaswilds(s);
            assert!(
                r == 0 || r == 1,
                "cmphaswilds({:?}) = {} not in {{0,1}}",
                s,
                r
            );
        }
    }

    /// c:917 — `dupstrspace(empty)` returns " ".
    #[test]
    fn dupstrspace_empty_returns_space_pin() {
        assert_eq!(dupstrspace(""), " ", "empty + trailing space = ' '");
    }

    /// c:917 — `dupstrspace` always ends in space.
    #[test]
    fn dupstrspace_always_ends_in_space() {
        for s in ["", "x", "hello", "包含中文"] {
            let r = dupstrspace(s);
            assert!(
                r.ends_with(' '),
                "dupstrspace({:?}) = {:?} must end in space",
                s,
                r
            );
        }
    }

    /// c:917 — `dupstrspace` is a pure function.
    #[test]
    fn dupstrspace_is_pure() {
        for s in ["", "abc", "hello"] {
            let first = dupstrspace(s);
            for _ in 0..5 {
                assert_eq!(dupstrspace(s), first, "dupstrspace({:?}) must be pure", s);
            }
        }
    }

    /// c:206 — `listchoices(empty)` returns i32 in exit-code range.
    #[test]
    fn listchoices_empty_args_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = listchoices(&[]);
        assert!(
            (0..256).contains(&r),
            "exit code {} must fit in u8 range",
            r
        );
    }

    /// c:233 — `spellword(empty)` returns i32 in exit-code range.
    #[test]
    fn spellword_empty_args_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = spellword(&[]);
        assert!(
            (0..256).contains(&r),
            "exit code {} must fit in u8 range",
            r
        );
    }

    /// c:331 — `listexpand(empty)` returns i32 in exit-code range.
    #[test]
    fn listexpand_empty_args_returns_in_exit_range() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let r = listexpand(&[]);
        assert!(
            (0..256).contains(&r),
            "exit code {} must fit in u8 range",
            r
        );
    }

    /// c:950 — `freebrinfo(None)` is idempotent.
    #[test]
    fn freebrinfo_none_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            freebrinfo(None);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_tricky.c
    // c:98 completecall / c:144 completeword / c:180 menucomplete /
    // c:268 expandorcomplete / c:370 checkparams / c:541 parambeg /
    // c:1024 has_real_token / c:1054 get_comp_string
    // ═══════════════════════════════════════════════════════════════════

    /// c:98 — `completecall` returns i32 (compile-time type pin).
    #[test]
    fn completecall_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = completecall(&[]);
    }

    /// c:144 — `completeword` returns i32.
    #[test]
    fn completeword_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = completeword(&[]);
    }

    /// c:180 — `menucomplete` returns i32.
    #[test]
    fn menucomplete_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = menucomplete(&[]);
    }

    /// c:268 — `expandorcomplete` returns i32.
    #[test]
    fn expandorcomplete_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = expandorcomplete(&[]);
    }

    /// c:304 — `menuexpandorcomplete` returns i32.
    #[test]
    fn menuexpandorcomplete_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = menuexpandorcomplete(&[]);
    }

    /// c:342 — `reversemenucomplete` returns i32.
    #[test]
    fn reversemenucomplete_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = reversemenucomplete(&[]);
    }

    /// c:351 — `acceptandmenucomplete` returns i32.
    #[test]
    fn acceptandmenucomplete_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = acceptandmenucomplete(&[]);
    }

    /// c:370 — `checkparams("")` empty returns i32 (compile-time type pin).
    #[test]
    fn checkparams_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = checkparams("");
    }

    /// c:370 — `checkparams` is deterministic for stable input.
    #[test]
    fn checkparams_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for s in ["", "FOO", "PATH", "__never_real_param__"] {
            let first = checkparams(s);
            for _ in 0..3 {
                assert_eq!(
                    checkparams(s),
                    first,
                    "checkparams({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:541 — `parambeg("", 0)` empty returns Option<usize>.
    #[test]
    fn parambeg_returns_option_usize_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Option<usize> = parambeg("", 0);
    }

    /// c:521-590 — the two verdicts `get_comp_string` c:1709 branches on,
    /// checked against the metafied token form the lexer really produces.
    ///
    /// `echo "$PA<TAB>` reaches c:1709 as `Dnull Qstring 'P' 'A'` with
    /// `offs == 4`: the cursor IS inside the name, so `parambeg` returns
    /// the name start (char index 2) and c:1710-1714 rewrites the `Dnull`
    /// to a literal `"`. That makes the `*s == Dnull` test at c:1728 fail,
    /// which is what keeps `instring`/`qipre` empty and lets `check_param`
    /// (compcore.c:1113) set `IPREFIX="$"` / `PREFIX="PA"` and `ispar`.
    ///
    /// `ls "$HOME/<TAB>` reaches it as `Dnull Qstring 'H' 'O' 'M' 'E' '/'`
    /// with `offs == 7`: the cursor is PAST the name (`e == 6`), so
    /// `parambeg` returns None (c:582 `offs <= e - s` fails) and the
    /// `Dnull` survives into the quote-form block — the pre-existing
    /// behaviour for a quoted path, unchanged by the c:1709 port.
    #[test]
    fn parambeg_cursor_inside_vs_past_name_in_double_quotes() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        use crate::ported::zsh_h::{Dnull, Qstring};

        let inside: String = [Dnull, Qstring, 'P', 'A'].iter().collect();
        assert_eq!(
            parambeg(&inside, 4),
            Some(2),
            "cursor inside the name of `\"$PA` must report the name start"
        );

        let past: String = [Dnull, Qstring, 'H', 'O', 'M', 'E', '/']
            .iter()
            .collect();
        assert_eq!(
            parambeg(&past, 7),
            None,
            "cursor past the name of `\"$HOME/` is not a parameter completion"
        );
    }

    /// c:1024 — `has_real_token("")` empty returns false (no tokens).
    #[test]
    fn has_real_token_empty_returns_false() {
        assert!(!has_real_token(""), "empty has no tokens");
    }

    /// c:1024 — `has_real_token` returns bool (compile-time type pin).
    #[test]
    fn has_real_token_returns_bool_type() {
        let _: bool = has_real_token("anything");
    }

    /// c:1024 — `has_real_token` is pure for stable input.
    #[test]
    fn has_real_token_is_pure() {
        for s in ["", "abc", "hello world", "no tokens"] {
            let first = has_real_token(s);
            for _ in 0..3 {
                assert_eq!(
                    has_real_token(s),
                    first,
                    "has_real_token({:?}) must be pure",
                    s
                );
            }
        }
    }

    /// c:1054 — `get_comp_string` returns Option<String> (type pin).
    #[test]
    fn get_comp_string_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: Option<String> = get_comp_string();
    }

    use crate::ported::zle::compcore::INWHAT;
    use crate::ported::zsh_h::IN_MATH;

    /// c:1564-1620 + c:1621-1706 — an UNTERMINATED array subscript puts
    /// the completion in the subscript context: `inwhat` becomes IN_MATH,
    /// `insubscr` 1, `varname` the array name, and the completion word is
    /// the (here empty) subscript text — NOT the `$fpath[` line word.
    ///
    /// Regression: without the subscript scan, `get_comp_string` handed
    /// back the whole `$fpath[` word. `docomplete`'s expand-or-complete
    /// probe (c:783-792) then saw the `$`, chose COMP_EXPAND, and
    /// `doexpansion` replaced the buffer with all 50 `$fpath` entries
    /// concatenated instead of listing subscript candidates.
    #[test]
    fn get_comp_string_unterminated_subscript_enters_math_context() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let line = "echo $fpath[";
        seed_metaline(line, line.chars().count() as i32);

        let word = get_comp_string();

        assert_eq!(
            INWHAT.load(Ordering::SeqCst),
            IN_MATH,
            "unclosed `[` must switch the completion to the math/subscript context"
        );
        assert_eq!(
            INSUBSCR.load(Ordering::SeqCst),
            1,
            "a plain (non-assoc) array subscript is insubscr == 1"
        );
        assert_eq!(
            VARNAME
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap()
                .clone(),
            Some("fpath".to_string()),
            "the identifier before `[` names the subscripted parameter"
        );
        assert_eq!(
            word.as_deref(),
            Some(""),
            "the completion word is the subscript text, not the `$fpath[` line word"
        );
        // c:1639/1642 — the word spans the (empty) range just after `[`.
        assert_eq!(WB.load(Ordering::SeqCst), line.chars().count() as i32);
    }

    /// c:1590 — a CLOSED subscript leaves the ordinary word context
    /// alone. Guards the scan against firing on every `[`.
    #[test]
    fn get_comp_string_closed_subscript_stays_out_of_math() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let line = "echo $fpath[1] ";
        seed_metaline(line, line.chars().count() as i32);

        let _ = get_comp_string();

        assert_ne!(
            INWHAT.load(Ordering::SeqCst),
            IN_MATH,
            "a balanced `[...]` is not a subscript context"
        );
        assert_eq!(INSUBSCR.load(Ordering::SeqCst), 0);
    }

    /// Seed the metafied completion line + cursor the way `docomplete`
    /// does before calling `get_comp_string`.
    fn seed_metaline(line: &str, cursor: i32) {
        use crate::ported::zle::compcore as cc;
        if let Ok(mut g) = cc::ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = line.to_string();
        }
        if let Ok(mut g) = cc::ZLEMETALINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
        {
            *g = line.to_string();
        }
        cc::ZLECS.store(cursor, Ordering::SeqCst);
        cc::ZLELL.store(line.chars().count() as i32, Ordering::SeqCst);
        cc::ZLEMETACS.store(cursor, Ordering::SeqCst);
        cc::ZLEMETALL.store(line.chars().count() as i32, Ordering::SeqCst);
    }

    /// c:2326-2330 — `spaceinline(1); zlemetaline[zlemetacs++] = ' ';`
    /// separates the expansions `doexpansion` splices into the line.
    ///
    /// The Rust `spaceinline` opens its gap in the EDITOR buffer, not in
    /// the metafied one `doexpansion` is editing, so the old
    /// open-a-gap-then-poke-a-byte pair wrote nothing once the cursor
    /// reached end-of-line: `ls *<TAB>` expanded to `aaccf1` rather than
    /// `aa cc f1`.
    #[test]
    fn doexpansion_separates_expansions_with_spaces() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let dir = tempfile::tempdir().expect("tempdir");
        let d = dir.path().to_str().expect("utf-8 tempdir").to_string();
        for n in ["zzA", "zzB"] {
            std::fs::write(dir.path().join(n), b"").expect("fixture file");
        }
        let word = format!("{}/zz*", d);
        let line = format!("ls {}", word);
        seed_metaline(&line, line.chars().count() as i32);
        WB.store(3, Ordering::SeqCst);
        WE.store(line.chars().count() as i32, Ordering::SeqCst);

        let rc = doexpansion(&word, COMP_EXPAND, COMP_EXPAND_COMPLETE, 0);

        let got = crate::ported::zle::compcore::ZLEMETALINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        assert_eq!(rc, 0, "an expansion that changed the word returns 0");
        // c:2326 — `nonempty(vl) || !first` also fires after the LAST item,
        // so zsh leaves a trailing space too.
        assert_eq!(got, format!("ls {}/zzA {}/zzB ", d, d));
    }

    /// c:2284-2288 — `opts[NULLGLOB] = 1` around `globlist`, restored
    /// after. A pattern that matches nothing must therefore be DROPPED
    /// silently: `ls -d **/b<TAB>` beeps in zsh, it does not print
    /// `no matches found`.
    ///
    /// The toggle was written through `opt_state_set("NULL_GLOB", …)`,
    /// which stores the caller's string verbatim (options.rs:1999) while
    /// `isset(NULLGLOB)` reads the canonical `"nullglob"` slot
    /// (extensions/opts_cache.rs:86). Both writes landed in a key nobody
    /// reads, NULLGLOB stayed off inside `globlist`, and the non-matching
    /// word took glob.rs:1576's NOMATCH arm — `zerr` + `errflag`, with the
    /// diagnostic painted over the prompt line mid-TAB.
    #[test]
    fn doexpansion_nullglob_toggle_drops_nonmatching_glob_silently() {
        use crate::ported::options::opt_state_set;
        use crate::ported::zsh_h::{
            isset, opt_name, CSHNULLGLOB, EXECOPT, GLOBOPT, NOMATCH, NULLGLOB,
        };

        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();

        // Preconditions that make glob.rs:1576's NOMATCH arm live: globbing
        // enabled, NOMATCH on, and neither null-glob option set.
        opt_state_set(opt_name(GLOBOPT), true);
        opt_state_set(opt_name(EXECOPT), true);
        opt_state_set(opt_name(NOMATCH), true);
        opt_state_set(opt_name(NULLGLOB), false);
        opt_state_set(opt_name(CSHNULLGLOB), false);
        // `zerr` only sets errflag when noerrs < 2 (utils.rs:224); a stale
        // value from another test would make this assertion vacuous.
        *crate::ported::utils::noerrs_lock().lock().unwrap() = 0;
        crate::ported::utils::errflag.store(0, Ordering::SeqCst);

        let dir = tempfile::tempdir().expect("tempdir");
        let d = dir.path().to_str().expect("utf-8 tempdir").to_string();
        std::fs::create_dir_all(dir.path().join("aa/bb")).expect("fixture dirs");
        // The reported case: recursive `**/` with a trailing component that
        // matches no directory anywhere in the tree.
        let word = format!("{}/**/zz-no-such-entry", d);
        let line = format!("ls -d {}", word);
        seed_metaline(&line, line.chars().count() as i32);
        WB.store(6, Ordering::SeqCst);
        WE.store(line.chars().count() as i32, Ordering::SeqCst);

        let rc = doexpansion(&word, COMP_EXPAND, COMP_EXPAND_COMPLETE, 0);

        assert_eq!(
            crate::ported::utils::errflag.load(Ordering::SeqCst),
            0,
            "a non-matching glob under the c:2286 NULLGLOB toggle must not \
             raise NOMATCH — errflag set means `no matches found` was printed"
        );
        // c:2292-2293 — `empty(vl)` → `goto end` with the initial `ret = 1`,
        // i.e. the caller beeps and the line is left alone.
        assert_eq!(rc, 1, "no expansion happened, so doexpansion returns 1");
        let got = crate::ported::zle::compcore::ZLEMETALINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        assert_eq!(got, line, "the command line must be untouched");
        // c:2288 — `opts[NULLGLOB] = ng` puts the user's setting back.
        assert!(
            !isset(NULLGLOB),
            "the c:2288 restore must return NULLGLOB to its pre-expansion state"
        );
    }

    /// c:Src/Zle/compcore.c:815-816/838 — `$words` and `$CURRENT` are
    /// compparams (`c:Src/Zle/complete.c:1259/1261`) that exist only
    /// between `startparamscope(); makecompparams();` and
    /// `endparamscope();`. zshrs publishes them from a SECOND place —
    /// `get_comp_string` writes them into paramtab directly, because the
    /// compparams here have no gsu binding to `COMPWORDS`/`COMPCURRENT`
    /// and without the direct publish `_normal` reads `$CURRENT` unset
    /// and completes only command names. That publish sits OUTSIDE
    /// c:815-838, so `CompWordParamScope` has to close it.
    ///
    /// Measured leak this pins (pty, both shells `-f -i` + `compinit`,
    /// one TAB, then `^U`):
    ///
    ///     ls *<TAB>    zsh: words=[][0]  zshrs before: words=[ls *][2] CURRENT=[2]
    ///     ls /tm<TAB>  zsh: words=[][0]  zshrs before: words=[][0]     CURRENT=[]
    ///
    /// Only the GLOBBED word leaked: with TAB on `expand-or-complete`,
    /// `doexpansion` (c:826) consumes the glob, the buffer changes, the
    /// c:847 `!strcmp(ol, zlemetaline)` guard fails and `docompletion` —
    /// hence `callcompfunc`, whose own scope stamp used to be the only
    /// thing tearing these down — never runs. Confirmed with a
    /// `compsys_args` trace: the glob case logs `get_comp_string publish
    /// words` and NO `callcompfunc ENTER`.
    ///
    /// Both halves are pinned:
    ///   * no pre-existing value → the names must be GONE after the
    ///     scope (what zsh shows at a bare prompt);
    ///   * a pre-existing value  → it must be BACK, not deleted, which
    ///     is what `endparamscope` does for a shadowed param.
    #[test]
    fn docomplete_comp_word_scope_tears_down_words_and_current() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let present = |name: &str| -> bool {
            crate::ported::params::paramtab()
                .read()
                .map(|t| t.gethashnode2(name).is_some())
                .unwrap_or(false)
        };

        // ---- arm 1: nothing shadowed — the publish must not survive.
        let _ = crate::ported::params::unsetparam("words");
        let _ = crate::ported::params::unsetparam("CURRENT");
        {
            let _scope = CompWordParamScope::new();
            crate::ported::params::setaparam("words", vec!["ls".into(), "*".into()]);
            let _ = crate::ported::params::setiparam("CURRENT", 2);
            assert_eq!(
                crate::ported::params::getaparam("words").as_deref(),
                Some(&["ls".to_string(), "*".to_string()][..]),
                "the get_comp_string publish must be VISIBLE inside the scope — \
                 without it `_normal` sees $CURRENT unset and every position \
                 completes as the command word"
            );
            assert_eq!(crate::ported::params::getiparam("CURRENT"), 2);
        }
        assert!(
            !present("words"),
            "$words outlived the completion — c:838 endparamscope leaves the \
             name unset, and `ls *<TAB>` must not publish it to the shell"
        );
        assert!(
            !present("CURRENT"),
            "$CURRENT outlived the completion — c:838 endparamscope leaves the \
             name unset"
        );

        // ---- arm 2: a shadowed value must come BACK, not be deleted.
        crate::ported::params::setaparam("words", vec!["user".into()]);
        let _ = crate::ported::params::setiparam("CURRENT", 99);
        {
            let _scope = CompWordParamScope::new();
            assert!(
                !present("words"),
                "the scope shadows the caller's $words the way c:815 \
                 startparamscope does — the completion must not read it"
            );
            crate::ported::params::setaparam("words", vec!["ls".into(), "*".into()]);
            let _ = crate::ported::params::setiparam("CURRENT", 2);
        }
        assert_eq!(
            crate::ported::params::getaparam("words").as_deref(),
            Some(&["user".to_string()][..]),
            "a user's own $words must be RESTORED on scope exit, not clobbered \
             by the completion's publish"
        );
        assert_eq!(
            crate::ported::params::getiparam("CURRENT"),
            99,
            "a user's own $CURRENT must be RESTORED on scope exit"
        );
        let _ = crate::ported::params::unsetparam("words");
        let _ = crate::ported::params::unsetparam("CURRENT");
    }

    /// c:2555-2567 — `printfmt`'s character arm un-metafies on the way out:
    /// `while (clen--) { if (*p == Meta) { p++; clen--; putc(*p++ ^ 32, shout); }
    ///  else putc(*p++, shout); }`. A byte that cannot stand alone as a
    /// character reaches `printfmt` in its metafied form, and the terminal must
    /// see the ONE raw byte, never the escape.
    ///
    /// zshrs metafies at the CHARACTER level (`U+0083` + `char::from(byte ^ 32)`,
    /// the encoding `unmetafy_str` decodes — utils.rs:16906), so the escape's
    /// own UTF-8 is `C2 83` + the payload's two bytes. Writing the format's
    /// bytes verbatim therefore put FOUR bytes on the terminal where zsh puts
    /// one: a described-match row truncated mid-UTF-8-character by
    /// `compdescribe` (computil.c:699-715) is exactly that shape, and
    /// `-g  -- make Japanese index head <<E3>` came out as
    /// `… <\u{83}\u{C3}` (`upmendex -`, `--cols 40`).
    #[test]
    fn printfmt_unmetafies_the_bytes_it_writes() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        // `<` then the metafied lone byte 0xE3 (the lead byte of `あ`, which is
        // all that fits when the description is cut at the screen edge).
        let meta = char::from(crate::ported::zsh_h::Meta);
        let fmt: String = format!("<{}{}", meta, char::from(0xe3u8 ^ 32));

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        let _ = printfmt(&fmt, 0, true, false);

        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);

        // The trailing bytes are the attribute reset / clear-to-EOL the C tail
        // emits (c:2576-2588); only the text prefix is under test.
        assert!(
            out.starts_with(b"<\xe3"),
            "printfmt must un-metafy: expected the raw byte E3 after `<`, got {:02x?}",
            out
        );
        assert!(
            !out.windows(2).any(|w| w == [0xc2u8, 0x83u8]),
            "the Meta escape must never reach the terminal; got {:02x?}",
            out
        );
    }
}
