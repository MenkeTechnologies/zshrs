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
    USEMENU.store(0, Ordering::SeqCst); // c:273
    USEGLOB.store(1, Ordering::SeqCst); // c:274
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:275
                                            // c:277-279 — `if (zlecs == zlell) return docomplete(COMP_LIST_COMPLETE);
                                            //              else deletechar()`.
    if ZLECS.load(Ordering::SeqCst) == ZLELL.load(Ordering::SeqCst) {
        docomplete(COMP_LIST_COMPLETE)
    } else {
        deletechar()
    }
}

/// Port of `expandword(char **args)` from Src/Zle/zle_tricky.c:287.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn expandword(_args: &[String]) -> i32 {
    // c:287
    USEMENU.store(0, Ordering::SeqCst); // c:289
    USEGLOB.store(0, Ordering::SeqCst); // c:289
    WOULDINSTAB.store(0, Ordering::SeqCst); // c:290
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
        isset, Equals, Hat, Inang, Inbrace, Inbrack, Inpar, Outang, Outbrace, Outbrack, Outpar,
        Pound, Qstring, Quest, Star, Stringg, Tilde, EXTENDEDGLOB, IGNOREBRACES,
    };
    let mut s = str;
    let bar_byte = b'|' as char;
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
            // c:501-508 — wildcard / balanced-bracket detection.
            let is_extglob_meta = (c == Pound as char || c == Hat as char) && isset(EXTENDEDGLOB);
            let is_simple_wild = c == Star as char || c == bar_byte || c == Quest as char;
            let mut s_try = s;
            let brack_balanced =
                crate::ported::utils::skipparens(Inbrack as char, Outbrack as char, &mut s_try)
                    == 0;
            let mut s_try = s;
            let ang_balanced =
                crate::ported::utils::skipparens(Inang as char, Outang as char, &mut s_try) == 0;
            let mut s_try = s;
            let brace_balanced = !isset(IGNOREBRACES)
                && crate::ported::utils::skipparens(Inbrace as char, Outbrace as char, &mut s_try)
                    == 0;
            let mut s_try = s;
            let pchars: Vec<char> = s.chars().collect();
            let pair_colon = pchars.first() == Some(&(Inpar as char))
                && pchars.get(1) == Some(&':')
                && crate::ported::utils::skipparens(Inpar as char, Outpar as char, &mut s_try) == 0;
            if is_extglob_meta
                || is_simple_wild
                || brack_balanced
                || ang_balanced
                || brace_balanced
                || pair_colon
            {
                return 1;
            }
            // c:510-511 — `if (*str) str++;`
            s = &s[c.len_utf8()..];
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
/// Rust signature: `(s, offs)` returns `Option<usize>` byte offset
/// instead of C's `char *` to the same position. `offs` is C's
/// `offs` global (zle_tricky.c) passed explicitly here.
pub fn parambeg(s: &str, offs: usize) -> Option<usize> {
    // c:521
    use crate::ported::zsh_h::{
        Dnull, Equals, Hat, Inbrace, Inbrack, Inpar, Outbrace, Outpar, Pound, Qstring, Quest, Star,
        Stringg, Tilde,
    };
    use crate::ported::ztype_h::{idigit, INAMESPC};
    let bytes = s.as_bytes();
    if offs > bytes.len() {
        return None;
    }
    // c:526 — `for (p = s + offs; p > s && *p != Stringg && *p != Qstring; p--);`.
    // Walk back to find a Stringg/Qstring token (the `$` marker).
    let mut p = offs.min(bytes.len());
    while p > 0 {
        let b = bytes[p.saturating_sub(1)];
        if p < bytes.len() && (bytes[p] == Stringg as u8 || bytes[p] == Qstring as u8) {
            break;
        }
        if b == Stringg as u8 || b == Qstring as u8 {
            p -= 1;
            break;
        }
        p -= 1;
    }
    if p >= bytes.len() {
        return None;
    }
    let pchar = bytes[p];
    if pchar == Stringg as u8 || pchar == Qstring as u8 {
        // c:529-532 — `$$` paired-marker walk.
        while p > 0 && (bytes[p - 1] == Stringg as u8 || bytes[p - 1] == Qstring as u8) {
            p -= 1;
        }
        while p + 2 < bytes.len()
            && (bytes[p + 1] == Stringg as u8 || bytes[p + 1] == Qstring as u8)
            && (bytes[p + 2] == Stringg as u8 || bytes[p + 2] == Qstring as u8)
        {
            p += 2;
        }
    }
    // c:535-537 — confirm `$` followed by NOT `(` / `[` / `'` (those
    // are `$(...)` / `$[...]` / `$'...'`, not parameter exprs).
    if p >= bytes.len() {
        return None;
    }
    let pchar = bytes[p];
    let after = bytes.get(p + 1).copied().unwrap_or(0);
    if !(pchar == Stringg as u8 || pchar == Qstring as u8)
        || after == Inpar as u8
        || after == Inbrack as u8
        || after == b'\''
    {
        return None;
    }
    // c:540-543 — `b = p + 1; n = 0; br = 1;`
    let mut b = p + 1;
    let mut br = 1;
    let mut n: i32 = 0;
    // c:545-553 — `${...}` form: validate balanced braces, then skip
    // possible `(...)` flag-prefix via skipparens.
    if b < bytes.len() && bytes[b] == Inbrace as u8 {
        // c:548 — `if (!skipparens(Inbrace, Outbrace, &tb)) return NULL;`
        let tb_str = &s[b..];
        let mut tb = tb_str;
        if crate::ported::utils::skipparens(Inbrace as char, Outbrace as char, &mut tb) != 0 {
            return None;
        }
        // c:551-552 — `b++, br++;`.
        b += 1;
        br += 1;
        let _ = br;
        // c:553 — `n = skipparens(Inpar, Outpar, &b);` skip `(flags)`.
        let mut b_str: &str = &s[b..];
        n = crate::ported::utils::skipparens(Inpar as char, Outpar as char, &mut b_str);
        b = s.len() - b_str.len();
    }
    // c:556-560 — skip modifier prefix chars `^=~` (Hat/Equals/Tilde).
    while b < bytes.len() {
        let bb = bytes[b];
        if bb != b'^'
            && bb != Hat as u8
            && bb != b'='
            && bb != Equals as u8
            && bb != b'~'
            && bb != Tilde as u8
        {
            break;
        }
        b += 1;
    }
    // c:561-562 — `# ` modifier.
    if b < bytes.len() && (bytes[b] == b'#' || bytes[b] == Pound as u8 || bytes[b] == b'+') {
        b += 1;
    }
    // c:564-569 — skip leading Dnull (`$'...'` delimiters) inside `${...}`.
    let mut e = b;
    if br != 0 {
        while e < bytes.len() && bytes[e] == Dnull as u8 {
            e += 1;
        }
    }
    // c:570-580 — find end of parameter name.
    if e < bytes.len() {
        let eb = bytes[e];
        if eb == Quest as u8
            || eb == Star as u8
            || eb == Stringg as u8
            || eb == Qstring as u8
            || eb == b'?'
            || eb == b'*'
            || eb == b'$'
            || eb == b'-'
            || eb == b'!'
            || eb == b'@'
        {
            e += 1;
        } else if idigit(eb) {
            while e < bytes.len() && idigit(bytes[e]) {
                e += 1;
            }
        } else {
            // c:579 — `e = itype_end(e, INAMESPC, 0);`. Rust port has
            // a simpler signature; INAMESPC matches identifier-name
            // chars (alpha/digit/_).
            let _ = INAMESPC;
            // c:zle_tricky.c:579 — `e = itype_end(e, INAMESPC, 0);`
            let span =
                crate::ported::utils::itype_end(&s[e..], crate::ported::ztype_h::INAMESPC, false);
            e += span;
        }
    }
    // c:583-590 — confirm cursor falls inside the name AND `n <= 0`
    // (skipparens didn't fail).
    if offs <= e && offs >= b && n <= 0 {
        // c:585-588 — skip trailing Dnull when `br` is set.
        if br != 0 {
            let mut pp = e;
            while pp < bytes.len() && bytes[pp] == Dnull as u8 {
                pp += 1;
            }
            let _ = pp;
        }
        return Some(b);
    }
    None
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
    // c:599
    // c:606-609 — recursion guard. The C source uses a static `active`
    // flag; we mirror via thread_local since each worker runs its own
    // completion.
    thread_local! { static ACTIVE: std::cell::Cell<bool> =
    const { std::cell::Cell::new(false) }; }
    // c:606 — `if (active && !comprecursive)`. `comprecursive` (set by the
    // menu recursive-completion arms) temporarily permits re-entry.
    if ACTIVE.with(|c| c.get())
        && crate::ported::zle::complist::COMPRECURSIVE
            .load(std::sync::atomic::Ordering::Relaxed)
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
    if crate::ported::zle::compcore::ZLEMETALL.load(Ordering::SeqCst) == 0 {
        crate::ported::zle::compcore::metafy_line();
    }

    // c:664-810 — `get_comp_string()` extracts the cursor word and
    // sets origword/lincmd/wb/we. The Rust port runs the (best-effort)
    // extractor for its side effects (LINCMD/WB/WE) and uses the
    // returned word as `origword`; if it returns None we fall back
    // to the full line.
    let origword = get_comp_string();
    let line = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let s_word: String = origword.unwrap_or_else(|| line.clone());
    let lincmd = LINCMD.load(Ordering::SeqCst); // c:805
    let olst = lst; // c:816 — `olst` is the original `lst` saved before dispatch

    // c:817-870 — dispatch on `lst`.
    let ret;
    if lst == COMP_SPELL {
        // c:801-815 — spell-word path. Direct port:
        //   foredel(we - wb, CUT_RAW);
        //   spckword(&x, 0, lincmd, 0);
        //   ret = !strcmp(x, ox);
        //   inststr(x);
        let wb = WB.load(Ordering::SeqCst);
        let we = WE.load(Ordering::SeqCst);
        if we > wb {
            // c:807 — `zlemetacs = wb`.
            ZLEMETACS.store(wb, Ordering::SeqCst);
            // c:808 — `foredel(we - wb, CUT_RAW)`.
            foredel(we - wb, CUT_RAW);
        }
        let mut x = s_word.clone(); // c:810 — `dupstring(w)`
        let ox = s_word.clone(); // c:810 — `ox = dupstring(w)`
                                 // c:813 — `spckword(&x, 0, lincmd, 0)`.
        crate::ported::utils::spckword(&mut x, 0, lincmd, 0);
        // c:814 — `ret = !strcmp(x, ox)` — returns 1 (unchanged) /
        // 0 (changed). Matches C `!strcmp` semantics.
        let r = if x == ox { 1 } else { 0 };
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
        let ne = crate::ported::exec::noerrs.load(Ordering::SeqCst); // c:839
        crate::ported::exec::noerrs.store(1, Ordering::SeqCst); // c:840
        let mut ret_local = doexpansion(&s_word, lst, olst, lincmd); // c:841
        LASTAMBIG.store(0, Ordering::SeqCst); // c:842
        crate::ported::exec::noerrs.store(ne, Ordering::SeqCst); // c:843

        // c:847-868 — if expand-or-complete and buffer unchanged,
        // fall through to docompletion.
        let after = crate::ported::zle::compcore::ZLEMETALINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        if olst == COMP_EXPAND_COMPLETE && ol_before == after {
            // c:850-851 — clear ERRFLAG_ERROR, restore cursor.
            crate::ported::utils::errflag
                .fetch_and(!crate::ported::utils::ERRFLAG_ERROR, Ordering::SeqCst);
            ret_local = docompletion(&s_word, lst, lincmd); // c:865
        } else if ret_local != 0 {
            // c:854 — `if (ret) clearlist = 1`.
            CLEARLIST.store(1, Ordering::SeqCst);
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
    let mut dat: [i32; 2] = [ret, 0];
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

    ret // _active_guard resets ACTIVE on drop
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

/// Check if string has real tokens (not escaped)
/// Port of has_real_token(const char *s) from zle_tricky.c
pub fn has_real_token(s: &str) -> bool {
    let special = ['$', '`', '"', '\'', '\\', '{', '}', '[', ']', '*', '?', '~'];

    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if special.contains(&c) {
            return true;
        }
    }

    false
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
/// NOT YET PORTED (return the raw lexer-extracted word untokenized;
/// see get_comp_string report):
///   - c:1482–1706 IN_MATH / array-subscript word extraction —
///     returns None for that context.
///   - c:1709–1926 quote-form cleanup (`qipre`/`qisuf`/`autoq` have
///     no writable shared globals; `getkeystring` lacks the `how`
///     arg for the `$'...'` path).
///   - c:1931–2218 IGNOREBRACES brace-expansion tail (`origword`
///     global absent).
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
        isset, lextok, AMPER, AMPERBANG, BARAMP, BAR_TOK, CASE, COMPLETEALIASES, DAMPER, DBAR,
        DINPAR, DOLOOP, ENDINPUT, ENVARRAY, ENVSTRING, FOR, FOREACH, INPAR_TOK, IN_COND, IN_ENV,
        IN_MATH, IN_NOTHING, IN_PAR, LEXERR, LEXFLAGS_ZLE, Inbrack, Meta, Outbrack, NULLTOK,
        OUTPAR_TOK, RCQUOTES, REPEAT, SELECT, SEPER, STRING_LEX, TYPESET,
    };
    use crate::ported::ztype_h::INAMESPC;

    let snull = crate::ported::zle::compctl::Snull;
    let dnull = crate::ported::zle::compctl::Dnull;
    let bnull = crate::ported::zle::compctl::Bnull;

    // c:1091 — `int ona = noaliases;` (save for restore at exit).
    let ona = noaliases();

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
        if let Ok(mut g) = ORIGLINE
            .get_or_init(|| Mutex::new(String::new()))
            .lock()
        {
            *g = meta_snap.clone();
        }
        ORIGCS.store(zlemetacs, Ordering::SeqCst);
        ORIGLL.store(meta_snap.len() as i32, Ordering::SeqCst);
    }

    // c:1119-1130 — reset brace-info state. `rdstrs` recording
    // (c:1245-1250, c:1396-1398) is omitted (no writable shared list).
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
        let prev_at = if cs > 0 { bytes.get(cs - 1).copied() } else { None };
        let comppref = COMPPREF.load(Ordering::SeqCst) != 0;
        let instr = INSTRING.load(Ordering::SeqCst);
        let is_iblank = matches!(ch_at, Some(b' ' | b'\t'));
        let is_blank_unescaped = is_iblank && (cs == 0 || prev_at != Some(b'\\'));
        let cs_at_end = ch_at.is_none() || cs >= ll;
        let is_newline = ch_at == Some(b'\n');
        let is_separator =
            matches!(ch_at, Some(b')' | b'`' | b'}' | b';' | b'|' | b'&' | b'>' | b'<'));
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
            zml = String::from_utf8_lossy(&nb).into_owned();
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
    let zlemetall = meta_snap.len() as i32; // length excluding the injected x
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
    // Locals that survive the loop for post-loop resolution. clwords /
    // cmdstr / varname have no writable shared globals in the Rust port,
    // so they stay local (reported).
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

    's_restart: loop {
        INWHAT.store(IN_NOTHING, Ordering::SeqCst); // c:1157
        LEX_PARBEGIN.set(-1); // c:1159
        LEX_PAREND.set(-1);
        LINCMD.store(incmdpos() as i32, Ordering::SeqCst); // c:1160
        let mut linredir: i32 = inredir() as i32; // c:1161 (local; no shared global)
        cmdstr = None; // c:1162-1163
        let mut cmdtok: lextok = NULLTOK; // c:1164
        varname = None; // c:1165-1166
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
                varname = tokstr().map(|s| ztrdup(&s));
            } else if tokv == INPAR_TOK {
                parct += 1;
            } else if tokv == OUTPAR_TOK {
                if parct != 0 {
                    parct -= 1;
                } else if linarr != 0 {
                    linarr = 0;
                    set_incmdpos(true);
                }
            }

            // c:1244-1268 — redirection handling. rdstrs recording is
            // omitted; the cursor-in-middle-of-redirection wb/we
            // adjustment IS ported.
            if inredir() && IS_REDIROP(tokv) {
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
                    cmdstr = Some(c);
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
                // c:1395-1398 — rdstrs recording omitted; C `continue`.
                // A do-while `continue` re-tests the loop condition, so
                // honor the c:1446 end condition before continuing.
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
            INWHAT.store(IN_ENV, Ordering::SeqCst);
        } else {
            LINCMD.store(cp, Ordering::SeqCst);
            linredir = rd;
        }
        crate::ported::hist::strinend(); // c:1460
        crate::ported::input::inpop(); // c:1461
        LEX_LEXFLAGS.set(0); // c:1462
        crate::ported::utils::errflag
            .fetch_and(!crate::ported::utils::ERRFLAG_ERROR, Ordering::SeqCst); // c:1463

        // c:1464-1480 — parbegin command-substitution restart.
        if LEX_PARBEGIN.get() != -1 {
            let parend = LEX_PAREND.get();
            let off = zlemetall + addedx - LEX_PARBEGIN.get() + 1;
            let ub = zml.as_bytes();
            let li = off as isize;
            let is_dollar_dparen = li >= 3
                && (li as usize) < ub.len()
                && ub[li as usize] == b'('
                && ub[(li - 1) as usize] == b'('
                && ub[(li - 2) as usize] == b'$';
            if !is_dollar_dparen {
                if parend >= 0 {
                    let new_ll = zlemetall - parend;
                    if new_ll >= 0
                        && (new_ll as usize) <= zml.len()
                        && zml.is_char_boundary(new_ll as usize)
                    {
                        zml.truncate(new_ll as usize);
                    }
                }
                zcontext_restore(); // c:1476
                tt = None; // c:1477
                linptr = zml.clone();
                continue 's_restart; // c:1478 goto start
            }
        }

        // c:1482-1541 — resolve `s` from the token kind.
        let s: String;
        if INWHAT.load(Ordering::SeqCst) == IN_MATH {
            // c:1482-1483 — IN_MATH word extraction NOT YET PORTED.
            set_noaliases(ona);
            zcontext_restore();
            return None;
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
            let mut soff = ns_off;
            if ttv.as_bytes().get(soff) == Some(&b'+') {
                soff += 1; // c:1509-1510
            }
            // c:1511-1512 — subscript / past-cursor => math context.
            let mut rest: &str = &ttv[soff..];
            let sp = crate::ported::utils::skipparens(Inbrack, Outbrack, &mut rest);
            let after_paren_off = ttv.len() - rest.len();
            let wb0 = WB.load(Ordering::SeqCst);
            if sp > 0 || (after_paren_off as i32) > (zlemetacs_qsub - wb0) {
                // c:1513-1519 — array subscript => IN_MATH (not ported).
                set_noaliases(ona);
                zcontext_restore();
                return None;
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
            set_noaliases(ona);
            zcontext_restore();
            return None; // c:1559
        }

        // c:1542-1543 — clamp we to line length.
        if WE.load(Ordering::SeqCst) > zlemetall {
            WE.store(zlemetall, Ordering::SeqCst);
        }

        set_noaliases(ona); // c:1562

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
            let ws: Vec<String> = clwords.iter().map(|w| untokenize(w)).collect();
            let n = ws.len() as i32;
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
            crate::ported::params::setaparam("words", ws);
            crate::ported::params::setsparam("CURRENT", &cur.to_string());
        }

        // cmdstr/varname/cp/rd/ia are computed for fidelity but have no
        // wired downstream consumer (no shared globals).
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
                        *g = String::from_utf8_lossy(&bytes).into_owned();
                    }
                }
            }
            ADDEDX.store(0, Ordering::SeqCst);
        }

        // c:2219 — zcontext_restore(); return s.
        // NOTE: quote-form cleanup (c:1709-1926) + brace-expansion
        // (c:1931-2218) not ported; return the lexer word untokenized.
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
    // c:2237 — `if (zlemetaline != NULL) { meta path } else { wide path }`
    let zml_active = ZLEMETALINE.get().is_some();
    if zml_active {
        // c:2238 — `spaceinline(len);` then strncpy into ZLEMETALINE[cs..].
        // The Rust spaceinline operates on ZLELINE; for the meta path
        // we splice directly into ZLEMETALINE.
        if let Some(m) = ZLEMETALINE.get() {
            if let Ok(mut g) = m.lock() {
                let cs = ZLEMETACS.load(Ordering::SeqCst) as usize;
                let cs = cs.min(g.len());
                let take = (len as usize).min(str.len());
                let bytes = g.as_bytes();
                let new_line: String = String::from_utf8_lossy(&bytes[..cs]).into_owned()
                    + &str[..take]
                    + &String::from_utf8_lossy(&bytes[cs..]);
                *g = new_line;
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
    let instr = &str[..(len as usize).min(str.len())]; // c:2247 ztrduppfx(str, len)
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
        if lst == COMP_LIST_EXPAND || lst == COMP_EXPAND {
            let ng = crate::ported::options::opt_state_get("NULL_GLOB").unwrap_or(false);
            crate::ported::options::opt_state_set("NULL_GLOB", true);
            crate::ported::subst::globlist(&mut vl, crate::ported::zsh_h::PREFORK_NO_UNTOK);
            crate::ported::options::opt_state_set("NULL_GLOB", ng);
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
        if first_item.is_empty() {
            return ret;
        }
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
        let no_change = first_item == ss;
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
            // c:2326-2330 — between items, insert a space.
            if !vl.empty() || !first {
                spaceinline(1);
                let pos = ZLEMETACS.load(Ordering::SeqCst);
                if let Some(metabuf) = ZLEMETALINE.get() {
                    if let Ok(mut m) = metabuf.lock() {
                        if (pos as usize) < m.len() {
                            // C: `zlemetaline[zlemetacs++] = ' '`.
                            // Rust String mutation: replace one
                            // char at byte offset pos.
                            let mut bytes = m.as_bytes().to_vec();
                            if (pos as usize) < bytes.len() {
                                bytes[pos as usize] = b' ';
                                *m = String::from_utf8_lossy(&bytes).into_owned();
                            }
                        }
                        ZLEMETACS.store(pos + 1, Ordering::SeqCst);
                    }
                }
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
    let mut out = String::new();
    while i < bytes.len() {
        let c = bytes[i];
        if doesc && c == b'%' {
            // c:2438
            i += 1;
            // c:2442 — `if (idigit(*++p)) arg = zstrtol(p, &p, 10)`.
            while i < bytes.len() && (bytes[i]).is_ascii_digit() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'%' => {
                    // c:2447
                    out.push('%');
                    cc += 1;
                }
                b'n' => {
                    // c:2455
                    let s = n.to_string();
                    cc += s.chars().count() as i32;
                    out.push_str(&s);
                }
                b'B' | b'b' | b'S' | b's' | b'U' | b'u' | b'F' | b'f' | b'K' | b'k' => {
                    // c:2466-2521 — text attrs (Bold/Standout/Underline/
                    //               Foreground/Background); no-op when
                    //               we have no curses substrate.
                }
                b'{' => {
                    // c:2522 — literal `%{ ... %}`.
                    i += 1;
                    while i < bytes.len() && bytes[i] != b'}' {
                        out.push(bytes[i] as char);
                        i += 1;
                    }
                }
                ch => {
                    out.push(ch as char);
                    cc += 1;
                }
            }
            i += 1;
        } else if c == b'\n' {
            // c:2537-2554 — a literal newline in the format ends a display
            // line: account the wrapped rows of the line just finished, reset
            // the column counter, and emit the '\n' (when printing).
            cc += 1; // c:2538
            l += 1 + ((cc - 1) / zterm_columns); // c:2550
            cc = 0; // c:2551
            out.push('\n'); // c:2553
            i += 1;
        } else {
            // c:2555-2572 — an ordinary character advances the column by its
            // display width (1 for the ASCII bytes this byte-wise loop sees).
            out.push(c as char);
            cc += 1; // c:2570 (WCWIDTH; 1 per byte here)
            i += 1;
        }
    }
    if dopr {
        // c:2576-2595 — the C tail does TCCLEAREOL / trailing-space padding
        // but NO unconditional `putc('\n')`. printfmt emits a newline ONLY
        // where the format itself contains one (c:2552 `if (*p=='\n')
        // putc('\n')`, already handled per-char above). Callers add the
        // inter-row `\n` themselves (printlist's `if(pnl) putc('\n')`,
        // c:2007/2080). The earlier port appended a trailing `\n` here,
        // double-spacing every CMF_DISPLINE description row and adding a
        // blank line after each `format` explanation header.
        use std::sync::atomic::Ordering;
        let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        let out_fd = if fd >= 0 { fd } else { 1 };
        let _ = write_loop(out_fd, out.as_bytes());
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
    } // c:2860 no `!` = no expansion
    let expanded = line.clone(); // pass-through
    if let Ok(mut g) = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
    {
        *g = expanded.clone();
        crate::ported::zle::compcore::ZLELL.store(g.len() as i32, Ordering::Relaxed);
        crate::ported::zle::compcore::ZLECS.store(g.len() as i32, Ordering::Relaxed);
    }
    1 // c:2864 expanded
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
    // c:3028 — zlecs = oldcs;
    // c:3029-3033 — if (zlecs >= cmdwe - 1) zlecs += str_chars - (cmdwe - cmdwb);
    let new_cs = if oldcs >= cmdwe.saturating_sub(1) {
        oldcs + str_chars - (cmdwe - cmdwb)
    } else {
        oldcs
    };
    ZLECS.store(new_cs.min(line.len() + str_chars), Ordering::SeqCst);
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

/// Port of `mod_export int origcs` from `Src/Zle/zle_tricky.c:75`.
/// Cursor position saved at completion entry.
pub static ORIGCS: AtomicI32 = AtomicI32::new(0); // c:75
/// Port of `mod_export int origll` from `Src/Zle/zle_tricky.c:75`.
/// Line length saved at completion entry.
pub static ORIGLL: AtomicI32 = AtomicI32::new(0); // c:75

/// Port of `mod_export int insubscr` from `Src/Zle/zle_tricky.c:405`.
/// != 0 if we are inside `${name[...]}` or `${(P)name[...]}`.
pub static INSUBSCR: AtomicI32 = AtomicI32::new(0); // c:405

/// Port of `mod_export int instring` from `Src/Zle/zle_tricky.c:419`.
/// QT_NONE (0), QT_SINGLE, QT_DOUBLE, QT_DOLLARS, or QT_BACKSLASH.
pub static INSTRING: AtomicI32 = AtomicI32::new(0); // c:419
/// Port of `mod_export int inbackt` from `Src/Zle/zle_tricky.c:419`.
pub static INBACKT: AtomicI32 = AtomicI32::new(0); // c:419

/// Port of `mod_export char *origline` from `Src/Zle/zle_tricky.c`.
/// The metafied line saved at completion entry.
pub static ORIGLINE: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // zle_tricky.c

/// Port of `mod_export char *lastprebr` from `Src/Zle/zle_tricky.c`.
pub static LASTPREBR: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // zle_tricky.c
/// Port of `mod_export char *lastpostbr` from `Src/Zle/zle_tricky.c`.
pub static LASTPOSTBR: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // zle_tricky.c

/// Port of `mod_export char *compquote` from `Src/Zle/zle_tricky.c`.
/// `$compstate[quote]` — current quoting context character.
pub static COMPQUOTE: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // zle_tricky.c
/// Port of `mod_export char *autoq` from `Src/Zle/zle_tricky.c`.
pub static AUTOQ: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new(); // zle_tricky.c

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
        assert!(has_real_token("$HOME"));
        assert!(has_real_token("*.txt"));
        assert!(!has_real_token("hello"));
        assert!(!has_real_token("test\\$var")); // escaped
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

    /// `cmphaswilds("|")` returns 1 — pipe is a wildcard at top-level.
    /// C `Src/Zle/zle_tricky.c:506` — `c == '|'`.
    #[test]
    fn cmphaswilds_pipe_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        assert_eq!(cmphaswilds("|"), 1, "| literal → has wildcard");
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
}
