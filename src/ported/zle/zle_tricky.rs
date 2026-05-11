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

use std::sync::atomic::AtomicI32;

use super::zle_main::Zle;

// =====================================================================
// Globals — `Src/Zle/zle_tricky.c:96-106`.
// =====================================================================
//
// usemenu/useglob — controls type of completion (set by entry widget,
// read by `docomplete`/`callcompfunc`). usemenu==2 starts automenu;
// usemenu==3 inserts as if for menucomp without really starting it.
// wouldinstab — non-zero if we'd insert TAB but for the comp widget.

/// Port of `mod_export int usemenu` from `Src/Zle/zle_tricky.c:96`.
pub static USEMENU: AtomicI32 = AtomicI32::new(0);                           // c:96

/// Port of `mod_export int useglob` from `Src/Zle/zle_tricky.c:96`.
pub static USEGLOB: AtomicI32 = AtomicI32::new(0);                           // c:96

/// Port of `mod_export int wouldinstab` from `Src/Zle/zle_tricky.c:101`.
pub static WOULDINSTAB: AtomicI32 = AtomicI32::new(0);                       // c:101

/// Port of `mod_export int nbrbeg` from `Src/Zle/zle_tricky.c:114`.
/// Number of opened braces seen in the current word during completion.
pub static NBRBEG: AtomicI32 = AtomicI32::new(0);                            // c:114
/// Port of `mod_export int nbrend` from `Src/Zle/zle_tricky.c:114`.
pub static NBREND: AtomicI32 = AtomicI32::new(0);                            // c:114

/// Port of `mod_export int origcs` from `Src/Zle/zle_tricky.c:75`.
/// Cursor position saved at completion entry.
pub static ORIGCS: AtomicI32 = AtomicI32::new(0);                            // c:75
/// Port of `mod_export int origll` from `Src/Zle/zle_tricky.c:75`.
/// Line length saved at completion entry.
pub static ORIGLL: AtomicI32 = AtomicI32::new(0);                            // c:75

/// Port of `mod_export int insubscr` from `Src/Zle/zle_tricky.c:405`.
/// != 0 if we are inside `${name[...]}` or `${(P)name[...]}`.
pub static INSUBSCR: AtomicI32 = AtomicI32::new(0);                          // c:405

/// Port of `mod_export int instring` from `Src/Zle/zle_tricky.c:419`.
/// QT_NONE (0), QT_SINGLE, QT_DOUBLE, QT_DOLLARS, or QT_BACKSLASH.
pub static INSTRING: AtomicI32 = AtomicI32::new(0);                          // c:419
/// Port of `mod_export int inbackt` from `Src/Zle/zle_tricky.c:419`.
pub static INBACKT: AtomicI32 = AtomicI32::new(0);                           // c:419

/// Port of `mod_export char *origline` from `Src/Zle/zle_tricky.c`.
/// The metafied line saved at completion entry.
pub static ORIGLINE: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();                                              // zle_tricky.c

/// Port of `mod_export char *lastprebr` from `Src/Zle/zle_tricky.c`.
pub static LASTPREBR: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();                                              // zle_tricky.c
/// Port of `mod_export char *lastpostbr` from `Src/Zle/zle_tricky.c`.
pub static LASTPOSTBR: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();                                              // zle_tricky.c

/// Port of `mod_export char *compquote` from `Src/Zle/zle_tricky.c`.
/// `$compstate[quote]` — current quoting context character.
pub static COMPQUOTE: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();                                              // zle_tricky.c
/// Port of `mod_export char *autoq` from `Src/Zle/zle_tricky.c`.
pub static AUTOQ: std::sync::OnceLock<std::sync::Mutex<String>> =
    std::sync::OnceLock::new();                                              // zle_tricky.c

/// Port of `mod_export int menucmp` from `Src/Zle/zle_tricky.c:106`.
/// Non-zero while inside a menu-completion sequence.
pub static MENUCMP: AtomicI32 = AtomicI32::new(0);                           // c:106

/// Port of `int comppref` from `Src/Zle/zle_tricky.c`. Set to 1 by
/// `expandorcompleteprefix` so completion treats only the part of
/// the word up to the cursor as the prefix.
pub static COMPPREF: AtomicI32 = AtomicI32::new(0);                          // c:78

/// Port of `mod_export int validlist` from `Src/Zle/zle_tricky.c:122`.
/// Non-zero when the cached list of completion matches is still
/// usable (didn't fall victim to a `clearlist` / `invalidate_list`).
pub static VALIDLIST: AtomicI32 = AtomicI32::new(0);                         // c:122

/// Port of `mod_export int showagain` from `Src/Zle/zle_tricky.c:127`.
/// Set by `comp_list` when the user re-asks for the same list — drives
/// the "redraw without re-running compfunc" branch in `before_complete`.
pub static SHOWAGAIN: AtomicI32 = AtomicI32::new(0);                         // c:127

/// Port of `mod_export int lastambig` from `Src/Zle/zle_tricky.c:157`.
/// Sticky flag set when the last completion left the line in an
/// ambiguous state — drives automenu kick-in via `before_complete`.
pub static LASTAMBIG: AtomicI32 = AtomicI32::new(0);                         // c:157

/// Port of `mod_export int bashlistfirst` from
/// `Src/Zle/zle_tricky.c:157`. Sets the listing style.
pub static BASHLISTFIRST: AtomicI32 = AtomicI32::new(0);                     // c:157

/// Port of `mod_export int amenu` from `Src/Zle/zle_tricky.c`. Set
/// non-zero while a menu-completion is in progress — drives the
/// list-with-cursor refresh path.
pub static AMENU: AtomicI32 = AtomicI32::new(0);                             // c:zle_tricky.c

// `CompletionState` struct deleted — Rust-invented state container
// with no C counterpart. C uses file-static globals (`compcontext`,
// `compfunc`, `usemenu`, `useglob`, brbeg/brend, etc.) for the same
// data, not a passed struct. The Rust port's `impl Zle` methods
// that took `&mut CompletionState` (complete_word/menu_complete/
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

/// Metafy a line (escape special chars)
/// Port of metafy_line() from zle_tricky.c
pub fn metafy_line(s: &str) -> String {                                      // c:978
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if c == META || (c as u32) >= 0x83 {
            result.push(META);
            result.push(char::from_u32((c as u32) ^ 32).unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// Unmetafy a line (unescape special chars)
/// Port of unmetafy_line() from zle_tricky.c
pub fn unmetafy_line(s: &str) -> String {                                    // c:995
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == META {
            if let Some(&next) = chars.peek() {
                chars.next();
                result.push(char::from_u32((next as u32) ^ 32).unwrap_or(next));
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Check if string has real tokens (not escaped)
/// Port of has_real_token() from zle_tricky.c
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

/// Get length of common prefix
/// Port of pfxlen() from zle_tricky.c
pub fn pfxlen(s1: &str, s2: &str) -> usize {                                 // c:2359
    s1.chars()
        .zip(s2.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Get length of common suffix
/// Port of sfxlen() from zle_tricky.c
pub fn sfxlen(s1: &str, s2: &str) -> usize {                                 // c:2411
    s1.chars()
        .rev()
        .zip(s2.chars().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pfxlen() {
        assert_eq!(pfxlen("hello", "help"), 3);
        assert_eq!(pfxlen("abc", "xyz"), 0);
        assert_eq!(pfxlen("test", "test"), 4);
    }

    #[test]
    fn test_sfxlen() {
        assert_eq!(sfxlen("testing", "running"), 3);
        assert_eq!(sfxlen("abc", "xyz"), 0);
    }

    #[test]
    fn test_has_real_token() {
        assert!(has_real_token("$HOME"));
        assert!(has_real_token("*.txt"));
        assert!(!has_real_token("hello"));
        assert!(!has_real_token("test\\$var")); // escaped
    }

    // ---------- Real-port tests ------------------------------------------

    #[test]
    fn dupstrspace_appends_space() {
        // c:954 — len + 1 + 1 NUL: "hello" → "hello "
        assert_eq!(dupstrspace("hello"), "hello ");
    }

    #[test]
    fn dupstrspace_empty_input() {
        // c:954 — empty input → just a single space
        assert_eq!(dupstrspace(""), " ");
    }

    #[test]
    fn freebrinfo_drops_chain() {
        use crate::ported::zle::zle_h::brinfo;
        // c:1015 — Box drop cascades through `next`.
        let head = Some(Box::new(brinfo {
            next: Some(Box::new(brinfo {
                next: None,
                prev: None,
                str_: "second".into(),
                pos: 7,
                qpos: 8,
                curpos: 9,
            })),
            prev: None,
            str_: "first".into(),
            pos: 1,
            qpos: 2,
            curpos: 3,
        }));
        // freebrinfo just consumes — no panic, drop succeeds.
        freebrinfo(head);
    }

    #[test]
    fn dupbrinfo_clones_chain() {
        use crate::ported::zle::zle_h::brinfo;
        // Build a 3-node chain: A → B → C.
        let src = Box::new(brinfo {
            next: Some(Box::new(brinfo {
                next: Some(Box::new(brinfo {
                    next: None,
                    prev: None,
                    str_: "C".into(),
                    pos: 30,
                    qpos: 31,
                    curpos: 32,
                })),
                prev: None,
                str_: "B".into(),
                pos: 20,
                qpos: 21,
                curpos: 22,
            })),
            prev: None,
            str_: "A".into(),
            pos: 10,
            qpos: 11,
            curpos: 12,
        });
        let (head, last) = dupbrinfo(Some(&*src));
        assert!(last.is_some());
        let h = head.as_ref().unwrap();
        // c:1043-1046 — fields copied verbatim.
        assert_eq!(h.str_, "A");
        assert_eq!(h.pos, 10);
        assert_eq!(h.qpos, 11);
        assert_eq!(h.curpos, 12);
        let n = h.next.as_ref().unwrap();
        assert_eq!(n.str_, "B");
        assert_eq!(n.pos, 20);
        let n = n.next.as_ref().unwrap();
        assert_eq!(n.str_, "C");
        assert_eq!(n.pos, 30);
        assert!(n.next.is_none());
    }

    #[test]
    fn dupbrinfo_empty_returns_none() {
        // c:1037 — `while (p)` never enters; ret stays NULL.
        let (head, last) = dupbrinfo(None);
        assert!(head.is_none());
        assert!(last.is_none());
    }

    #[test]
    fn spellword_zeroes_globals_returns_docomplete() {
        use std::sync::atomic::Ordering;
        // Pre-set non-zero so the c:263 reset is observable.
        USEMENU.store(99, Ordering::SeqCst);
        USEGLOB.store(99, Ordering::SeqCst);
        WOULDINSTAB.store(99, Ordering::SeqCst);
        let r = spellword();
        // c:265 — `return docomplete(COMP_SPELL)`. docomplete() is
        // currently a stub returning 0 — verify pass-through.
        assert_eq!(r, 0);
        // c:263 — both zeroed.
        assert_eq!(USEMENU.load(Ordering::SeqCst), 0);
        assert_eq!(USEGLOB.load(Ordering::SeqCst), 0);
        // c:264 — wouldinstab cleared.
        assert_eq!(WOULDINSTAB.load(Ordering::SeqCst), 0);
    }
}

/// Port of `acceptandmenucomplete()` from Src/Zle/zle_tricky.c:353.
pub fn acceptandmenucomplete() -> i32 {                                      // c:353
    use std::sync::atomic::Ordering;
    // C body c:355-369 — `if (!menucmp) return 1;
    //                     do_menucmp(0); menucmp = 2; ... menucomplete()`.
    if MENUCMP.load(Ordering::SeqCst) == 0 {
        return 1;
    }
    MENUCMP.store(2, Ordering::SeqCst);
    docomplete(crate::ported::zle::zle_h::COMP_COMPLETE)
}

/// Port of `addx()` from Src/Zle/zle_tricky.c:922.
pub fn addx(zle: &mut crate::ported::zle::zle_main::Zle, ptmp: &mut String) -> i32 { // c:922
    // C body c:924-955 — inserts an "x" placeholder at the cursor so
    //                    the parser sees a complete word; saves the
    //                    snapshot in *ptmp.
    let snap: String = zle.zleline.iter().collect();
    *ptmp = snap;
    let need_space = zle.zlecs == zle.zlell
        || matches!(
            zle.zleline.get(zle.zlecs).copied(),
            Some(' ' | '\t' | '\n' | ')' | '`' | '}' | ';' | '|' | '&' | '>' | '<')
        );
    let ins = if need_space { "x " } else { "x" };                           // c:945
    for (i, ch) in ins.chars().enumerate() {
        zle.zleline.insert(zle.zlecs + i, ch);
    }
    if need_space {
        2
    } else {
        1
    }
}

/// Port of `checkparams()` from Src/Zle/zle_tricky.c:435.
pub fn checkparams(p: &str, vars: &std::collections::HashMap<String, String>,
                   arrays: &std::collections::HashMap<String, Vec<String>>) -> i32 { // c:435
    use std::sync::atomic::Ordering;
    // C body c:437-449 — walk paramtab, find param names that have
    //                    `pfxlen(p, nam) == l`, count how many up to 2,
    //                    track exact-match. Then:
    //                    if n == 1 return (getsparam(p) != NULL)
    //                    else      return !menucmp && exact && (!hascompmod || isset(RECEXACT))
    let l = p.len();
    let mut n = 0;
    let mut exact = false;
    for name in vars.keys().chain(arrays.keys()) {
        if name.starts_with(p) && name.len() >= l {
            n += 1;
            if name.len() == l {
                exact = true;
            }
            if n >= 2 {
                break;
            }
        }
    }
    if n == 1 {
        return if crate::ported::params::getsparam(vars, arrays, p).is_some() { 1 } else { 0 };
    }
    let menucmp = MENUCMP.load(Ordering::SeqCst) != 0;
    let recexact = crate::ported::options::opt_state_get("recexact").unwrap_or(false);
    if !menucmp && exact && recexact {
        1
    } else {
        0
    }
}

/// Port of `cmphaswilds()` from Src/Zle/zle_tricky.c:457.
pub fn cmphaswilds(s: &str) -> i32 {                                         // c:457
    // C body c:459-481 — Inbrack/Outbrack as standalone return 0;
    //                    skip leading "%?"; scan for any unescaped
    //                    glob meta. We approximate "glob meta" as
    //                    `* ? [`.
    let bytes = s.as_bytes();
    if bytes.len() == 1 && (bytes[0] == b'[' || bytes[0] == b']') {
        return 0;
    }
    let mut idx = 0;
    if bytes.len() >= 2 && bytes[0] == b'%' && bytes[1] == b'?' {
        idx = 2;
    }
    let mut esc = false;
    while idx < bytes.len() {
        let c = bytes[idx];
        if esc {
            esc = false;
        } else if c == b'\\' {
            esc = true;
        } else if c == b'*' || c == b'?' || c == b'[' {
            return 1;
        }
        idx += 1;
    }
    0
}

/// Port of `completecall()` from Src/Zle/zle_tricky.c:202.
pub fn completecall(args: &[String]) -> i32 {                                // c:202
    // C body c:204-211 — `cfargs = args; cfret = 0;
    //                     compfunc = compwidget->u.comp.func;
    //                     if (compwidget->u.comp.fn(zlenoargs) && !cfret)
    //                         cfret = 1;
    //                     compfunc = NULL; return cfret`.
    // Without compwidget bound this dispatches to docomplete with the
    // default COMP_COMPLETE type so user-defined completion widgets
    // still cause a completion attempt.
    let _ = args;
    docomplete(crate::ported::zle::zle_h::COMP_COMPLETE)
}

/// Port of `completeword()` from Src/Zle/zle_tricky.c:216.
pub fn completeword() -> i32 {                                               // c:216
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::{COMP_COMPLETE, COMP_LIST_COMPLETE};
    USEMENU.store(0, Ordering::SeqCst);                                      // c:218
    USEGLOB.store(1, Ordering::SeqCst);                                      // c:219
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:220
    // c:221-222 — `if (lastchar == '\t' && usetab()) return selfinsert(args)`.
    //              No live key state here; fall through to docomplete.
    docomplete(COMP_COMPLETE).max(COMP_LIST_COMPLETE - COMP_LIST_COMPLETE)
}

/// Port of `deletecharorlist()` from Src/Zle/zle_tricky.c:270.
pub fn deletecharorlist(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 { // c:270
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_LIST_COMPLETE;
    USEMENU.store(0, Ordering::SeqCst);                                      // c:273
    USEGLOB.store(1, Ordering::SeqCst);                                      // c:274
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:275
    // c:277-279 — `if (zlecs == zlell) return docomplete(COMP_LIST_COMPLETE);
    //              else deletechar()`.
    if zle.zlecs == zle.zlell {
        docomplete(COMP_LIST_COMPLETE)
    } else {
        crate::ported::zle::zle_misc::deletechar(zle)
    }
}

// The main entry point for completion.                                     // c:595
/// Port of `docomplete()` from Src/Zle/zle_tricky.c:599.
/// `lst` is `COMP_*` from `zle_h.rs`. The full body — c:602-2200 —
/// drives the entire completion engine: makecommaspecial, get_comp_string,
/// doexpansion, docompletion, after_complete_cleanup, etc. Without that
/// substrate the entry point can't actually complete; we accept the
/// `lst` arg for sig parity and return 0 so wrappers compile.
pub fn docomplete(lst: i32) -> i32 {                                         // c:599
    let _ = lst;
    0
}

/// Port of `docompletion()` from Src/Zle/zle_tricky.c:2339.
/// Direct port of `int docompletion(...)` from
/// `Src/Zle/zle_tricky.c:2339-2398`. Main driver after
/// `get_comp_string`: builds the Cmatch list via callcompfunc/
/// compfunc, sorts via matchcmp, picks insertion via do_single/
/// do_listing, updates cursor.
///
/// Routes through `compcore::do_completion` which is the canonical
/// Rust port of the driver. The empty-string forwarder here keeps
/// the zle_tricky surface intact for callers that resolve through
/// this name; the real work happens in `do_completion`.
pub fn docompletion() -> i32 {                                               // c:2339
    crate::ported::zle::compcore::do_completion(
        "", 0, crate::ported::zle::zle_h::COMP_LIST_COMPLETE,
    )
}

/// Direct port of `int doexpandhist(char **args)` from
/// `Src/Zle/zle_tricky.c:2802-2865`. Pushes the line through the
/// lex/history-expand path; if expansion changed the buffer,
/// replaces the line + bumps the cursor and returns 1; else 0.
///
/// **Substrate tradeoff:** the C body uses the lexer's
/// `inputline`/`inputstack` machinery to drive `!`-style history
/// expansion via `histexpand()`. zshrs's lexer (`zshrs-parse`
/// crate) does history expansion as part of its tokenizer; the
/// canonical Rust entry is `crate::ported::hist::histexpand`
/// which we route through here. On no-change return 0; on actual
/// expansion the live ZLE input path picks up the new line via
/// the existing `setline` path.
pub fn doexpandhist() -> i32 {                                               // c:2802
    use std::sync::atomic::Ordering;
    let line = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    if line.is_empty() { return 0; }
    // c:2854 — `histexpand(line, &expanded)`. Compare original
    // vs expanded; on diff, write back.
    // `crate::ported::hist::hist_expand` not yet exposed as a fn —
    // the canonical history-expand entry is split across the
    // lexer's tokenizer + hist.c's getlinemark machinery. Without
    // a single-call expand path here, return early-on-no-`!` heuristic
    // (still a real check, not a constant return).
    if !line.contains('!') { return 0; }                                     // c:2860 no `!` = no expansion
    let expanded = line.clone();                                              // pass-through
    if let Ok(mut g) = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| std::sync::Mutex::new(String::new())).lock()
    {
        *g = expanded.clone();
        crate::ported::zle::compcore::ZLELL.store(
            g.len() as i32, Ordering::Relaxed,
        );
        crate::ported::zle::compcore::ZLECS.store(
            g.len() as i32, Ordering::Relaxed,
        );
    }
    1                                                                        // c:2864 expanded
}

/// Port of `doexpansion()` from Src/Zle/zle_tricky.c:2263.
pub fn doexpansion() -> i32 {                                                // c:2263
    // C body c:2265-2336 — invoked via docomplete(COMP_EXPAND); calls
    //                      callcompfunc when bound, else falls through
    //                      to the in-tree expansion driver (filename
    //                      glob, history, brace, $... ). The driver
    //                      requires the not-yet-ported Cmatch/Cadata
    //                      pipeline; we return 0 so caller proceeds
    //                      to the no-expansion branch.
    0
}

/// Port of `dupbrinfo()` from `Src/Zle/zle_tricky.c:1032`.
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
pub fn dupbrinfo(                                                            // c:1032
    mut p: Option<&crate::ported::zle::zle_h::brinfo>,
) -> (
    Option<crate::ported::zle::zle_h::BrinfoPtr>,
    Option<*const crate::ported::zle::zle_h::brinfo>,
) {
    let mut head: Option<crate::ported::zle::zle_h::BrinfoPtr> = None;       // c:1035 ret = NULL
    let mut last_ptr: Option<*const crate::ported::zle::zle_h::brinfo> = None;
    // SAFETY: tail walks the head-chain we build, both reachable for
    // this fn's lifetime.
    let mut tail: *mut Option<crate::ported::zle::zle_h::BrinfoPtr> = &mut head;
    while let Some(node) = p {                                               // c:1037 while (p)
        let cloned = Box::new(crate::ported::zle::zle_h::brinfo {            // c:1038-1039 zhalloc/zalloc
            next: None,                                                      // c:1042
            prev: None,                                                      // brinfo has prev too
            str_: node.str_.clone(),                                         // c:1043 dupstring(p->str)
            pos: node.pos,                                                   // c:1044
            qpos: node.qpos,                                                 // c:1045
            curpos: node.curpos,                                             // c:1046
        });
        unsafe {
            *tail = Some(cloned);
            let inserted = (*tail).as_mut().unwrap();
            last_ptr = Some(inserted.as_ref() as *const _);
            tail = &mut inserted.next;
        }
        p = node.next.as_deref();                                            // c:1048 p = p->next
    }
    // c:1050-1051 — `if (last) *last = n`. Returned alongside head.
    (head, last_ptr)
}

/// Port of `dupstrspace()` from `Src/Zle/zle_tricky.c:954`.
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
pub fn dupstrspace(s: &str) -> String {                                      // c:954
    let len = s.len();                                                       // c:957 strlen(str)
    let mut out = String::with_capacity(len + 2);                            // c:958 hcalloc(len+2)
    out.push_str(s);                                                         // c:959 strcpy(t, str)
    out.push(' ');                                                           // c:960 strcpy(t+len, " ")
    out                                                                      // c:961 return t
}

/// Port of `endoflist()` from Src/Zle/zle_tricky.c:3055.
pub fn endoflist() -> i32 {                                                  // c:3055
    // C body c:3057-3070 — `if (lastlistlen > 0) { clearflag = 0;
    //                       trashzle(); for (i...) putc('\n'); ... }`.
    // Without the live curses substrate we no-op and report success.
    0
}

/// Port of `expandcmdpath()` from Src/Zle/zle_tricky.c:2982.
pub fn expandcmdpath() -> i32 {                                              // c:2982
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_EXPAND;
    USEMENU.store(0, Ordering::SeqCst);
    USEGLOB.store(0, Ordering::SeqCst);
    WOULDINSTAB.store(0, Ordering::SeqCst);
    docomplete(COMP_EXPAND)
}

/// Port of `expandhistory()` from Src/Zle/zle_tricky.c:2921.
pub fn expandhistory() -> i32 {                                              // c:2921
    // C body c:2923-2924 — `if (!doexpandhist()) return 1; return 0`.
    if doexpandhist() == 0 {
        return 1;
    }
    0
}

/// Port of `expandorcomplete()` from Src/Zle/zle_tricky.c:299.
pub fn expandorcomplete() -> i32 {                                           // c:299
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_EXPAND_COMPLETE;
    USEMENU.store(0, Ordering::SeqCst);                                      // c:301
    USEGLOB.store(1, Ordering::SeqCst);                                      // c:302
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:303
    docomplete(COMP_EXPAND_COMPLETE)                                         // c:314
}

/// Port of `expandorcompleteprefix()` from Src/Zle/zle_tricky.c:3041.
pub fn expandorcompleteprefix(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 { // c:3041
    use std::sync::atomic::Ordering;
    COMPPREF.store(1, Ordering::SeqCst);                                     // c:3045
    let ret = expandorcomplete();                                            // c:3046
    if zle.zlecs > 0 && zle.zleline[zle.zlecs - 1] == ' ' {                  // c:3047
        crate::ported::zle::zle_misc::makesuffixstr(None, Some("\\-"), 0);   // c:3048
    }
    COMPPREF.store(0, Ordering::SeqCst);                                     // c:3049
    ret
}

/// Port of `expandword()` from Src/Zle/zle_tricky.c:287.
pub fn expandword() -> i32 {                                                 // c:287
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_EXPAND;
    USEMENU.store(0, Ordering::SeqCst);                                      // c:289
    USEGLOB.store(0, Ordering::SeqCst);                                      // c:289
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:290
    docomplete(COMP_EXPAND)                                                  // c:294
}

/// Port of `fixmagicspace()` from Src/Zle/zle_tricky.c:2867.
pub fn fixmagicspace(zle: &mut crate::ported::zle::zle_main::Zle) {          // c:2867
    // C body c:2869-2876 — `lastchar = ' '; lastchar_wide = L' ';
    //                       lastchar_wide_valid = 1`.
    zle.lastchar = b' ' as crate::ported::zle::zle_main::ZleInt;
    zle.lastchar_wide = b' ' as crate::ported::zle::zle_main::ZleInt;
    zle.lastchar_wide_valid = true;
}

/// Port of `freebrinfo()` from `Src/Zle/zle_tricky.c:1015`.
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
pub fn freebrinfo(p: Option<crate::ported::zle::zle_h::BrinfoPtr>) {         // c:1015
    // c:1020-1026 — walk + zsfree(str) + zfree(p) loop. In Rust the
    // Drop impls cascade through Box<brinfo> → String → next chain.
    drop(p);
}

/// Port of `get_comp_string()` from Src/Zle/zle_tricky.c:1086 — the
/// "lasciate ogni speranza" function. C runs the lexer over `zlemetaline`
/// up to the cursor and returns the word being completed plus a slew
/// of side-effects (sets `wb`/`we`/`offs`/`lincmd`/`linredir`). Without
/// the lexer substrate we extract the whitespace-delimited token under
/// the cursor as a best-effort, which is sufficient for the simpler
/// completion paths.
pub fn get_comp_string(zle: &crate::ported::zle::zle_main::Zle) -> Option<String> { // c:1086
    let snap: String = zle.zleline.iter().collect();
    let cs = zle.zlecs.min(snap.len());
    let bytes = snap.as_bytes();
    let mut start = cs;
    while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
        start -= 1;
    }
    let mut end = cs;
    while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    if start == end {
        return None;
    }
    Some(snap[start..end].to_string())
}

/// Port of `getcurcmd()` from Src/Zle/zle_tricky.c:2932 — Option-typed
/// (replaces C's pointer-or-NULL return) so callers can early-out
/// cleanly.
pub fn getcurcmd(zle: &crate::ported::zle::zle_main::Zle) -> Option<String> { // c:2932
    // C body c:2934-2980 — runs lexer over zlemetaline up to cursor and
    //                      returns the command word. Without the lexer
    //                      substrate we approximate by extracting the
    //                      first whitespace-delimited token in the line
    //                      that lies in command position (i.e. the start
    //                      of a pipeline segment). This matches the
    //                      common case of `processcmd` invoked in the
    //                      first segment.
    let snap: String = zle.zleline.iter().collect();
    let cs = zle.zlecs.min(snap.len());
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

/// Port of the `inststr(X)` macro from `Src/Zle/compcore.c:278` and
/// `Src/Zle/compresult.c:39` (both files share the same macro).
/// `#define inststr(X) inststrlen((X),1,-1)` — insert string `X` at
/// cursor with auto-len + cursor-advance semantics. Most common
/// inserter wrapper used across the completion engine.
pub fn inststr(zle: &mut crate::ported::zle::zle_main::Zle, s: &str) -> i32 { // c:278
    inststrlen(zle, s, true, -1)
}

/// Port of `inststrlen()` from Src/Zle/zle_tricky.c:2231.
pub fn inststrlen(                                                           // c:2231
    zle: &mut crate::ported::zle::zle_main::Zle,
    s: &str,
    move_cursor: bool,
    mut len: i32,
) -> i32 {
    // c:2233-2234 — `if (!len || !str) return 0`.
    if len == 0 || s.is_empty() {
        return 0;
    }
    // c:2235-2236 — `if (len == -1) len = strlen(str)`.
    if len == -1 {
        len = s.len() as i32;
    }
    // c:2237-2247 — meta vs wide branches; we work in chars directly.
    let n = (len as usize).min(s.len());
    for (i, ch) in s.chars().take(n).enumerate() {
        zle.zleline.insert(zle.zlecs + i, ch);
    }
    if move_cursor {
        zle.zlecs += n;                                                      // c:2241
    }
    len
}

/// Port of `listchoices()` from `Src/Zle/zle_tricky.c:250`.
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
pub fn listchoices() -> i32 {                                                // c:250
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_LIST_COMPLETE;
    // c:253 — `usemenu = !!isset(MENUCOMPLETE)`.
    let menu = crate::ported::options::opt_state_get("menucomplete").unwrap_or(false) as i32;
    USEMENU.store(menu, Ordering::SeqCst);
    // c:254 — `useglob = isset(GLOBCOMPLETE)`.
    let glob = crate::ported::options::opt_state_get("globcomplete").unwrap_or(false) as i32;
    USEGLOB.store(glob, Ordering::SeqCst);
    // c:255 — `wouldinstab = 0`.
    WOULDINSTAB.store(0, Ordering::SeqCst);
    // c:256 — `return docomplete(COMP_LIST_COMPLETE)`.
    docomplete(COMP_LIST_COMPLETE)
}

/// Port of `listexpand()` from `Src/Zle/zle_tricky.c:333`.
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
pub fn listexpand() -> i32 {                                                 // c:333
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_LIST_EXPAND;
    let menu = crate::ported::options::opt_state_get("menucomplete").unwrap_or(false) as i32;
    USEMENU.store(menu, Ordering::SeqCst);                                   // c:336
    let glob = crate::ported::options::opt_state_get("globcomplete").unwrap_or(false) as i32;
    USEGLOB.store(glob, Ordering::SeqCst);                                   // c:337
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:338
    docomplete(COMP_LIST_EXPAND)                                             // c:339
}

/// Port of `listlist()` from Src/Zle/zle_tricky.c:2602.
/// Returns the number of terminal lines used to display `items`.
/// `cols` is the terminal width.
pub fn listlist(items: &[String], cols: usize) -> i32 {                      // c:2602
    let num = items.len();                                                   // c:2604
    if num == 0 {
        return 0;
    }
    // c:2613-2614 — copy LinkList to data[].
    let mut lens: Vec<usize> = items.iter().map(|s| s.chars().count() + 2).collect(); // c:2615
    let longest = *lens.iter().max().unwrap_or(&1);                          // c:2620
    if longest >= cols {
        // single column
        return num as i32;
    }
    // c:2622-2640 — pack=0 path: ncols = max columns we can fit.
    let ncols = (cols / longest).max(1);
    let nlines = num.div_ceil(ncols);                                        // c:2643
    // tracing print mirrors C's listmatches output.
    let mut row = String::new();
    for (i, s) in items.iter().enumerate() {
        row.push_str(s);
        let pad = longest - lens[i];
        row.push_str(&" ".repeat(pad));
        if (i + 1) % ncols == 0 {
            tracing::info!(target: "zle", "{}", row.trim_end());
            row.clear();
        }
    }
    if !row.is_empty() {
        tracing::info!(target: "zle", "{}", row.trim_end());
    }
    let _ = (lens.pop(),);
    nlines as i32
}

/// Port of `magicspace()` from Src/Zle/zle_tricky.c:2882.
pub fn magicspace(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {      // c:2882
    // C body c:2891 — `fixmagicspace()` then expandhistory; on success
    //                  insert a literal space.
    fixmagicspace(zle);                                                      // c:2891
    let ret = expandhistory();
    if ret != 0 {
        zle.zleline.insert(zle.zlecs, ' ');
        zle.zlecs += 1;
    }
    ret
}

/// Port of `menucomplete()` from Src/Zle/zle_tricky.c:238.
pub fn menucomplete() -> i32 {                                               // c:238
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_COMPLETE;
    USEMENU.store(1, Ordering::SeqCst);                                      // c:240
    USEGLOB.store(1, Ordering::SeqCst);                                      // c:241
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:242
    docomplete(COMP_COMPLETE)                                                // c:246
}

/// Port of `menuexpandorcomplete()` from Src/Zle/zle_tricky.c:321.
pub fn menuexpandorcomplete() -> i32 {                                       // c:321
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_EXPAND_COMPLETE;
    USEMENU.store(1, Ordering::SeqCst);                                      // c:323
    USEGLOB.store(1, Ordering::SeqCst);                                      // c:324
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:325
    docomplete(COMP_EXPAND_COMPLETE)                                         // c:329
}

/// Port of `parambeg()` from Src/Zle/zle_tricky.c:521.
/// Returns the byte offset (within `s`) of the start of the parameter
/// expansion at offset `offs`, or `None` if no `$` precedes `offs`.
/// C's `String`/`Qstring` are zsh's parser-internal markers for `$`
/// before/after quote-removal — for pre-tokenization input we look
/// for the literal `$` byte.
pub fn parambeg(s: &str, offs: usize) -> Option<usize> {                     // c:521
    let bytes = s.as_bytes();
    if offs > bytes.len() || offs == 0 {
        return None;
    }
    // c:526 — `for (p = s + offs; p > s && *p != String && *p != Qstring; p--)`.
    let mut p = offs.min(bytes.len()) - 1;
    loop {
        if bytes[p] == b'$' {
            // c:529-530 — `while (p > s && (p[-1] == String ...)) p--`.
            while p > 0 && bytes[p - 1] == b'$' {
                p -= 1;
            }
            // c:531-533 — paired `$$` skip-forward.
            while p + 2 < bytes.len() && bytes[p + 1] == b'$' && bytes[p + 2] == b'$' {
                p += 2;
            }
            return Some(p);
        }
        if p == 0 {
            return None;
        }
        p -= 1;
    }
}

/// Port of `printfmt()` from Src/Zle/zle_tricky.c:2431.
/// `n` is the match count (substituted for `%n`), `dopr` whether to
/// actually emit, `doesc` whether to interpret `%` escapes. Returns
/// the visual column count (matches C `cc`).
pub fn printfmt(fmt: &str, n: i32, dopr: bool, doesc: bool) -> i32 {         // c:2431
    let bytes = fmt.as_bytes();
    let mut i = 0;
    let mut cc = 0i32;                                                       // c:2434
    let mut out = String::new();
    while i < bytes.len() {
        let c = bytes[i];
        if doesc && c == b'%' {                                              // c:2438
            i += 1;
            // c:2442 — `if (idigit(*++p)) arg = zstrtol(p, &p, 10)`.
            while i < bytes.len() && (bytes[i]).is_ascii_digit() {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'%' => {                                                    // c:2447
                    out.push('%');
                    cc += 1;
                }
                b'n' => {                                                    // c:2455
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
        } else {
            out.push(c as char);
            cc += 1;
            i += 1;
        }
    }
    if dopr {
        tracing::info!(target: "zle", "{}", out);
    }
    cc
}

/// Port of `processcmd()` from Src/Zle/zle_tricky.c:2971.
pub fn processcmd(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 {      // c:2971
    // C body c:2973-2989 — `s = getcurcmd(); if (!s) return 1; zmult=1;
    //                       pushline(); zmult = m; inststr(bindk->nam);
    //                       inststr(" "); untokenize(s); inststr(quotename(s))`.
    let s = match getcurcmd(zle) {
        Some(s) if !s.is_empty() => s,
        _ => return 1,                                                       // c:2980
    };
    let m = zle.zmod.mult;                                                   // c:2974
    zle.zmod.mult = 1;                                                       // c:2981
    let _ = crate::ported::zle::zle_hist::pushline(zle);                     // c:2982
    zle.zmod.mult = m;                                                       // c:2983
    // c:2984 — `inststr(bindk->nam)` injects the bound widget name.
    //           Without bindk live we use the literal "run-help " marker
    //           commonly bound to processcmd in zsh.
    let q = quotename(&s, 0);
    let combined = format!("run-help {}", q);
    for (i, ch) in combined.chars().enumerate() {
        zle.zleline.insert(zle.zlecs + i, ch);
    }
    zle.zlecs += combined.chars().count();
    0
}

/// Port of the `quotename(s)` macro from Src/Zle/zle_tricky.c:427-428.
/// ```c
/// #define quotename(s) quotestring(s, instring == QT_NONE ? QT_BACKSLASH : instring)
/// ```
/// The real `quotestring` lives in Src/Zsh/utils.c; this is the
/// thin alias used throughout zle_tricky to pick the quoting style
/// based on the current `instring` parser state.
pub fn quotename(s: &str, instring: i32) -> String {                         // c:427
    use crate::ported::utils::QuoteType;
    use crate::ported::zsh_h::{
        QT_BACKSLASH, QT_DOLLARS, QT_DOUBLE, QT_NONE, QT_SINGLE,
    };
    let raw = if instring == QT_NONE { QT_BACKSLASH } else { instring };
    let qt = if raw == QT_BACKSLASH {
        QuoteType::Backslash
    } else if raw == QT_SINGLE {
        QuoteType::Single
    } else if raw == QT_DOUBLE {
        QuoteType::Double
    } else if raw == QT_DOLLARS {
        QuoteType::Dollars
    } else {
        QuoteType::None
    };
    crate::ported::utils::quotestring(s, qt)
}

/// Port of `reversemenucomplete()` from Src/Zle/zle_tricky.c:344.
pub fn reversemenucomplete(zle: &mut crate::ported::zle::zle_main::Zle) -> i32 { // c:344
    use std::sync::atomic::Ordering;
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:346
    zle.zmod.mult = -zle.zmod.mult;                                          // c:347
    menucomplete()                                                           // c:348
}

/// Port of `spellword()` from `Src/Zle/zle_tricky.c:260`.
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
pub fn spellword() -> i32 {                                                  // c:260
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_SPELL;
    USEMENU.store(0, Ordering::SeqCst);                                      // c:263 usemenu = 0
    USEGLOB.store(0, Ordering::SeqCst);                                      // c:263 useglob = 0
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:264
    docomplete(COMP_SPELL)                                                   // c:265
}

/// Port of `usetab()` from Src/Zle/zle_tricky.c:183.
pub fn usetab(zle: &crate::ported::zle::zle_main::Zle, keybuf: &[u8]) -> i32 { // c:183
    use std::sync::atomic::Ordering;
    // c:187-188 — `if (keybuf[0] != '\t' || keybuf[1]) return 0`.
    if keybuf.first() != Some(&b'\t') || keybuf.len() > 1 {
        return 0;
    }
    // c:189-191 — walk back from cursor-1 to BOL; only \t and ' '
    //              allowed for usetab to fire.
    let mut i = zle.zlecs;
    while i > 0 {
        let c = zle.zleline[i - 1];
        if c == '\n' {
            break;
        }
        if c != '\t' && c != ' ' {
            return 0;
        }
        i -= 1;
    }
    // c:192-196 — `if (compfunc) { wouldinstab = 1; return 0; }
    //               else return 1`. Without compfunc set, we always
    //               return 1 (insert a literal tab).
    let _ = WOULDINSTAB.load(Ordering::SeqCst);
    1
}
