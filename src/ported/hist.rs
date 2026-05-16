//! hist.c - history mechanism
//!
//! Port of Src/hist.c
//!
//! The history lines are kept in a hash, and also doubly-linked in a ring.   // c:98

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::ported::zsh_h::histent;
pub use crate::zsh_h::{CASMOD_CAPS, CASMOD_LOWER, CASMOD_NONE, CASMOD_UPPER};
use crate::zsh_h::{
    isset,
    BANGHIST, HFILE_FAST, HFILE_USE_OPTIONS,
    HISTIGNOREALLDUPS, HISTIGNOREDUPS, HISTIGNORESPACE,
    HISTNOFUNCTIONS, HISTNOSTORE, HISTREDUCEBLANKS,
    INCAPPENDHISTORY, INCAPPENDHISTORYTIME, INTERACTIVE,
    SHAREHISTORY, SHINSTDIN,
};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::Component::*;

// Bits of histactive variable                                               // c:137
/// Port of `HA_ACTIVE` from Src/hist.c:138. History mechanism is active.
pub const HA_ACTIVE: u32 = 1 << 0;                                           // c:138
/// Port of `HA_NOINC` from Src/hist.c:139. Don't store, curhist not incremented.
pub const HA_NOINC: u32 = 1 << 1;                                            // c:139
/// Port of `HA_INWORD` from Src/hist.c:140. We're inside a word.
pub const HA_INWORD: u32 = 1 << 2;                                           // c:140
/// Port of `HA_UNGET` from Src/hist.c:142. Recursively ungetting.
pub const HA_UNGET: u32 = 1 << 3;                                            // c:142

/// Port of `static zlong defev` from Src/hist.c:210.
static defev: AtomicI64 = AtomicI64::new(0);                                 // c:210

/// Port of `static int hist_keep_comment` from Src/hist.c:217.
static hist_keep_comment: AtomicI32 = AtomicI32::new(0);                     // c:217

/// Port of `static int histsave_stack_size` from Src/hist.c:239.
static histsave_stack_size: AtomicI32 = AtomicI32::new(0);                   // c:239

/// Port of `static int histsave_stack_pos` from Src/hist.c:240.
static histsave_stack_pos: AtomicI32 = AtomicI32::new(0);                    // c:240

/// Port of `static zlong histfile_linect` from Src/hist.c:242.
static histfile_linect: AtomicI64 = AtomicI64::new(0);                       // c:242

// =========================================================================
// Functions from hist.c
// =========================================================================

/// Port of `void hist_context_save(struct hist_stack *hs, int toplevel)`
/// from Src/hist.c:248.
pub fn hist_context_save(hs: &mut crate::ported::zsh_h::hist_stack, toplevel: i32) { // c:248
    if toplevel != 0 {                                                       // c:248
        // top level, make this version visible to ZLE                       // c:251
        *zle_chline.lock().unwrap() = Some(chline.lock().unwrap().clone());  // c:252
        // ensure line stored is NULL-terminated — implicit in String        // c:253-255
    }
    hs.histactive = histactive.load(Ordering::SeqCst) as i32;                // c:257
    hs.histdone = histdone.load(Ordering::SeqCst);                           // c:258
    hs.stophist = stophist.load(Ordering::SeqCst);                           // c:259
    hs.hline = Some(chline.lock().unwrap().clone());                         // c:260
    hs.hptr = Some(hptr.load(Ordering::SeqCst).to_string());                 // c:261
    hs.chwords = chwords.lock().unwrap().clone();                            // c:262
    hs.chwordlen = chwordlen.load(Ordering::SeqCst);                         // c:263
    hs.chwordpos = chwordpos.load(Ordering::SeqCst);                         // c:264
    // hs->hgetc / hungetc / hwaddc / hwbegin / hwabort / hwend / addtoline  // c:265-271
    // are runtime-mutable function-pointer globals in C; the Rust port
    // dispatches statically via crate::ported::input.
    hs.hlinesz = hlinesz.load(Ordering::SeqCst);                             // c:272
    hs.defev = defev.load(Ordering::SeqCst);                                 // c:273
    hs.hist_keep_comment = hist_keep_comment.load(Ordering::SeqCst);         // c:274
    // hs->cstack = cmdstack; hs->csp = cmdsp;                               // c:296-282
    hs.csp = 0;

    stophist.store(0, Ordering::SeqCst);                                     // c:296
    chline.lock().unwrap().clear();                                          // c:296
    hptr.store(0, Ordering::SeqCst);                                         // c:296
    histactive.store(0, Ordering::SeqCst);                                   // c:296
    // cmdstack = zalloc(CMDSTACKSZ); cmdsp = 0;                             // c:296-289
}

/// Port of `void hist_context_restore(const struct hist_stack *hs, int toplevel)`
/// from Src/hist.c:296.
pub fn hist_context_restore(hs: &crate::ported::zsh_h::hist_stack, toplevel: i32) { // c:296
    if toplevel != 0 {                                                       // c:296
        // Back to top level: don't need special ZLE value                   // c:299
        // DPUTS(hs->hline != zle_chline, "BUG: Ouch, wrong chline for ZLE") // c:300
        *zle_chline.lock().unwrap() = None;                                  // c:301
    }
    histactive.store(hs.histactive as u32, Ordering::SeqCst);                // c:303
    histdone.store(hs.histdone, Ordering::SeqCst);                           // c:304
    stophist.store(hs.stophist, Ordering::SeqCst);                           // c:305
    *chline.lock().unwrap() = hs.hline.clone().unwrap_or_default();          // c:306
    hptr.store(hs.hptr.as_ref().and_then(|s| s.parse().ok()).unwrap_or(0),   // c:307
               Ordering::SeqCst);
    *chwords.lock().unwrap() = hs.chwords.clone();                           // c:308
    chwordlen.store(hs.chwordlen, Ordering::SeqCst);                         // c:309
    chwordpos.store(hs.chwordpos, Ordering::SeqCst);                         // c:310
    // hgetc / hungetc / hwaddc / hwbegin / hwabort / hwend / addtoline      // c:311-317
    hlinesz.store(hs.hlinesz, Ordering::SeqCst);                             // c:318
    defev.store(hs.defev, Ordering::SeqCst);                                 // c:339
    hist_keep_comment.store(hs.hist_keep_comment, Ordering::SeqCst);         // c:339
    // cmdstack = hs->cstack; cmdsp = hs->csp;                               // c:339-324
}

/// Port of `void hist_in_word(int yesno)` from Src/hist.c.
pub fn hist_in_word(yesno: i32) {
    if yesno != 0 {
        histactive.fetch_or(HA_INWORD, Ordering::SeqCst);
    } else {
        histactive.fetch_and(!HA_INWORD, Ordering::SeqCst);
    }
}

/// Port of `int hist_is_in_word(void)` from Src/hist.c.
pub fn hist_is_in_word() -> i32 {
    if (histactive.load(Ordering::SeqCst) & HA_INWORD) != 0 { 1 } else { 0 }
}

/// Port of `void ihwaddc(int c)` from Src/hist.c:357.
///
/// C body:
/// ```c
/// if (chline && !(errflag || lexstop) &&
///     (inbufflags & (INP_ALIAS|INP_HIST)) != INP_ALIAS) {
///     if (c == bangchar && stophist < 2 && qbang)
///         hwaddc('\\');
///     *hptr++ = c;
///     if (hptr - chline >= hlinesz) {
///         int oldsiz = hlinesz;
///         chline = realloc(chline, hlinesz = oldsiz + 64);
///         hptr = chline + oldsiz;
///     }
/// }
/// ```
pub fn ihwaddc(c: i32) {                                                     // c:357
    use crate::ported::zsh_h::{INP_ALIAS, INP_HIST};
    // c:360-361 — guard: history line must exist, no error/lex stop,
    // and we're not strictly inside alias-expansion-only input.
    if crate::ported::utils::errflag.load(Ordering::SeqCst) != 0 || lexstop.load(Ordering::SeqCst) {
        return;
    }
    let inbufflags = crate::ported::input::inbufflags.with(|f| f.get());
    if (inbufflags & (INP_ALIAS | INP_HIST)) == INP_ALIAS {
        return;
    }
    let chline_empty = chline.lock().unwrap().is_empty();
    if chline_empty {
        // C requires `chline != NULL` — the equivalent here is "an
        // hbegin() has populated the buffer". On startup it's
        // empty, so behave like the inactive-history C path.
        return;
    }
    // c:362-366 — bang-escape under qbang.
    let bc = bangchar.load(Ordering::SeqCst);
    if c == bc && stophist.load(Ordering::SeqCst) < 2 && qbang.load(Ordering::SeqCst) {
        chline.lock().unwrap().push('\\');                                    // c:366 `hwaddc('\\');`
    }
    // c:368 `*hptr++ = c;`
    chline.lock().unwrap().push(c as u8 as char);
    // c:370-374 — resize tracking. Rust `String` grows on `push`
    // automatically, but `hlinesz` mirrors C's allocation count
    // for any caller that reads it (e.g. `hwend()`).
    let cur_len = chline.lock().unwrap().len() as i32;
    let sz = hlinesz.load(Ordering::SeqCst);
    if cur_len >= sz {
        let new_sz = sz + 64;
        hlinesz.store(new_sz, Ordering::SeqCst);
    }
}

/// Port of `void iaddtoline(int c)` from Src/hist.c:397.
///
/// C body (c:397-414):
/// ```c
/// if (!expanding || lexstop) return;
/// if (qbang && c == bangchar && stophist < 2) {
///     exlast--;
///     zleentry(ZLE_CMD_ADD_TO_LINE, '\\');
/// }
/// if (excs > zlemetacs) {
///     excs += 1 + inbufct - exlast;
///     if (excs < zlemetacs) excs = zlemetacs;
/// }
/// exlast = inbufct;
/// zleentry(ZLE_CMD_ADD_TO_LINE, itok(c) ? ztokens[c - Pound] : c);
/// ```
///
/// The previous Rust port collapsed the body to a single
/// `chline.push(c as u8 as char)` and dropped:
///   - The `!expanding || lexstop` guard (c:399) — pushed
///     unconditionally even when not in history expansion.
///   - The bang-escape `qbang` path (c:401-404).
///   - The `excs`/`exlast` cursor tracking (c:405-410).
///   - The crucial `itok(c) ? ztokens[c - Pound] : c` mapping at
///     c:413 — without it, token bytes (`Pound`..`Nularg` =
///     0x84..0xa1) get pushed RAW into the history line buffer
///     instead of being decoded to their visible chars (`#`,
///     `$`, `^`, `*`, `(`, ...).
///
/// Port the guard, bangchar escape, and the itok→ztokens
/// mapping. The cursor tracking + zleentry hook are doc-pinned
/// for future ZLE wireup but otherwise no-op since chline is
/// the backing store for both code paths in zshrs.
pub fn iaddtoline(c: i32) {                                                  // c:397
    use crate::ported::ztype_h::itok;
    // c:399 — `if (!expanding || lexstop) return;`.
    if expanding.load(Ordering::SeqCst) == 0
        || lexstop.load(Ordering::SeqCst)
    {
        return;
    }
    // c:401-404 — bang-escape under qbang.
    let bc = bangchar.load(Ordering::SeqCst);
    if qbang.load(Ordering::SeqCst)
        && c == bc
        && stophist.load(Ordering::SeqCst) < 2
    {
        exlast.fetch_sub(1, Ordering::SeqCst);                                // c:402
        chline.lock().unwrap().push('\\');                                    // c:403 zleentry ADD '\\'
    }
    // c:410 — `exlast = inbufct;`
    let inbufct_v = crate::ported::input::inbufct.with(|cnt| cnt.get());
    exlast.store(inbufct_v, Ordering::SeqCst);                                // c:410
    // c:413 — `itok(c) ? ztokens[c - Pound] : c`.
    let push_byte: u8 = if c >= 0 && c <= 0xff && itok(c as u8) {
        let idx = (c as u8).wrapping_sub(crate::ported::zsh_h::Pound as u8) as usize;
        // ztokens is the literal-char back-mapping for ITOK bytes.
        // Defensively guard against an out-of-range token byte
        // (the closed range Pound..=Nularg is 0x84..=0xa1, 30
        // entries; ztokens covers them).
        crate::ported::lex::ztokens
            .bytes()
            .nth(idx)
            .unwrap_or(c as u8)
    } else {
        c as u8
    };
    chline.lock().unwrap().push(push_byte as char);                           // c:413
}

/// Port of `void safeinungetc(int c)` from Src/hist.c:466.
/// ```c
/// static void
/// safeinungetc(int c)
/// {
///     if (lexstop)
///         lexstop = 0;
///     else
///         inungetc(c);
/// }
/// ```
pub fn safeinungetc(c: i32) {                                                // c:467
    if lexstop.load(Ordering::SeqCst) {                                      // c:469
        lexstop.store(false, Ordering::SeqCst);                              // c:470
    } else {                                                                 // c:471
        if let Some(ch) = char::from_u32(c as u32) {                         // c:472 inungetc(c)
            crate::ported::input::inungetc(ch);
        }
    }
}

/// Port of `int ihgetc(void)` from Src/hist.c:418. Returns the next
/// character from the input stream, optionally performing history
/// expansion via `histsubchar`. Sets `lexstop`/`errflag` and bumps
/// `qbang` per the bang-escape rules.
/// ```c
/// static int
/// ihgetc(void)
/// {
///     int c = ingetc();
///     if (exit_pending) { lexstop = 1; errflag |= ERRFLAG_ERROR; return ' '; }
///     qbang = 0;
///     if (!stophist && !(inbufflags & INP_ALIAS)) {
///         c = histsubchar(c);
///         if (c < 0) { lexstop = 1; errflag |= ERRFLAG_ERROR; return ' '; }
///     }
///     if ((inbufflags & INP_HIST) && !stophist) {
///         qbang = 0;
///         if (c == '\\' && !(qbang = (c = ingetc()) == bangchar))
///             safeinungetc(c), c = '\\';
///     } else if (stophist || (inbufflags & INP_ALIAS))
///         qbang = c == bangchar && (stophist < 2);
///     hwaddc(c);
///     addtoline(c);
///     return c;
/// }
/// ```
pub fn ihgetc() -> i32 {                                                     // c:418
    use crate::ported::zsh_h::{INP_ALIAS, INP_HIST};
    let mut c: i32 = crate::ported::input::ingetc()                          // c:420 int c = ingetc();
        .map(|ch| ch as i32)
        .unwrap_or(-1);
    if exit_pending.load(Ordering::SeqCst) {                                 // c:422
        lexstop.store(true, Ordering::SeqCst);                               // c:424
        crate::ported::utils::errflag.fetch_or(                              // c:425 errflag |= ERRFLAG_ERROR
            crate::ported::utils::ERRFLAG_ERROR,
            Ordering::SeqCst,
        );
        return b' ' as i32;                                                  // c:426
    }
    qbang.store(false, Ordering::SeqCst);                                    // c:428 qbang = 0
    let inbufflags_v = crate::ported::input::inbufflags.with(|f| f.get());
    if stophist.load(Ordering::SeqCst) == 0                                  // c:429 !stophist
        && (inbufflags_v & INP_ALIAS) == 0                                   // c:429 !(inbufflags & INP_ALIAS)
    {
        c = histsubchar(c);                                                  // c:431 c = histsubchar(c)
        if c < 0 {                                                           // c:432
            lexstop.store(true, Ordering::SeqCst);                           // c:434
            crate::ported::utils::errflag.fetch_or(                          // c:435
                crate::ported::utils::ERRFLAG_ERROR,
                Ordering::SeqCst,
            );
            return b' ' as i32;                                              // c:436
        }
    }
    let inbufflags_v = crate::ported::input::inbufflags.with(|f| f.get());
    let bc = bangchar.load(Ordering::SeqCst);
    if (inbufflags_v & INP_HIST) != 0 && stophist.load(Ordering::SeqCst) == 0 {  // c:439
        // c:447 qbang = 0
        qbang.store(false, Ordering::SeqCst);
        if c == b'\\' as i32 {                                               // c:448 c == '\\'
            let g = crate::ported::input::ingetc()                           // c:448 c = ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if g == bc {                                                     // c:448 qbang = (c == bangchar)
                qbang.store(true, Ordering::SeqCst);
                c = g;
            } else {
                // c:449 safeinungetc(c), c = '\\';
                safeinungetc(g);
                c = b'\\' as i32;
            }
        }
    } else if stophist.load(Ordering::SeqCst) != 0                           // c:450 stophist
        || (inbufflags_v & INP_ALIAS) != 0                                   // c:450 (inbufflags & INP_ALIAS)
    {
        // c:458 qbang = c == bangchar && (stophist < 2)
        let v = c == bc && stophist.load(Ordering::SeqCst) < 2;
        qbang.store(v, Ordering::SeqCst);
    }
    ihwaddc(c);                                                              // c:459 hwaddc(c)
    iaddtoline(c);                                                           // c:460 addtoline(c)
    c                                                                        // c:462 return c
}

/// Port of `unsigned char hatchar` from `Src/params.c:132`. Caret used
/// as the substitution-shortcut lead character on first column; init'd
/// to `'^'` at `Src/init.c:1102`. Read by `histsubchar` (c:618).
/// NOTE on placement: canonical home per PORT.md Rule C would be
/// `params.rs`, alongside `bangchar`/`hashchar`. Kept here to mirror
/// the existing (pre-2026-05) placement of `bangchar` at hist.rs:1858;
/// move-all-three is a follow-up cleanup.
pub static hatchar: AtomicI32 = AtomicI32::new(b'^' as i32);                 // c:params.c:132

/// Port of `unsigned char hashchar` from `Src/params.c:132`. Comment-
/// start character; init'd to `'#'` at `Src/init.c:1101`. Read by
/// `gettokstr` (`Src/lex.c:678`). Made atomic so `histcharssetfn`
/// (`Src/params.c:5097`) can update it dynamically when `$HISTCHARS`
/// changes — previously was a `const char` in `lex.rs:3507`, which
/// silently diverged from C (no `setopt HISTCHARS='!^@'` syntax
/// could change the comment character).
pub static hashchar: AtomicI32 = AtomicI32::new(b'#' as i32);                // c:params.c:132

/// Port of `static int marg` from `Src/hist.c:599`. Argument index of the
/// most-recent `!?str?` event match; `-1` when no match has happened.
pub static marg: AtomicI32 = AtomicI32::new(-1);                             // c:599

/// Port of `static zlong mev` from `Src/hist.c:600`. Event number of the
/// most-recent `!?str?` event match; `-1` when no match has happened.
pub static mev: AtomicI64 = AtomicI64::new(-1);                              // c:600

/// Port of `static int histsubchar(int c)` from Src/hist.c:594.
/// Implements `^foo^bar` and full `!` history expansion: walks the input
/// stream pulling event-spec, word-designator, and modifier characters,
/// looks up the matching history entry, applies any modifiers, then
/// pushes the resulting string back onto the input stack via `inpush`.
/// Returns the first character of the expanded result or `-1` on error.
///
/// ```c
/// static int
/// histsubchar(int c)
/// {
///     int farg, evset = -1, larg, argc, cflag = 0, bflag = 0;
///     zlong ev;
///     static int marg = -1;
///     static zlong mev = -1;
///     char *buf, *ptr;
///     char *sline;
///     int lexraw_mark;
///     Histent ehist;
///     size_t buflen;
///     /* ... see Src/hist.c:594-983 for full body ... */
/// }
/// ```
pub fn histsubchar(c_in: i32) -> i32 {                                       // c:595
    use crate::ported::zsh_h::{CSHJUNKIEHISTORY, HISTVERIFY};
    let mut c: i32 = c_in;
    let mut farg: i32;                                                       // c:597
    let mut evset: i32 = -1;                                                 // c:597
    let mut larg: i32;
    let mut argc: i32;                                                       // c:597
    let mut cflag: i32 = 0;                                                  // c:597
    let mut bflag: i32 = 0;                                                  // c:597
    let mut ev: i64;                                                         // c:598
    let mut buf: String;                                                     // c:601 char *buf, *ptr
    let mut sline: String;                                                   // c:602
    // c:603 lexraw_mark — Rust port's lexer doesn't expose `zshlex_raw_mark`
    // hook yet; mirror the C `lexraw_mark` / `zshlex_raw_back_to_mark` calls
    // as a no-op `i32` carry.
    let lexraw_mark: i32 = 0;                                                // c:603,615

    // c:618 — `^foo^bar` shortcut: only valid on first column of input.
    let hat = hatchar.load(Ordering::SeqCst);
    if crate::ported::lex::LEX_ISFIRSTCH.with(|f| f.get()) && c == hat {                                         // c:618
        let mut gbal: i32 = 0;                                               // c:619
        // c:622 — clear isfirstch
        crate::ported::lex::LEX_ISFIRSTCH.with(|f| f.set(false));                                                // c:622
        // c:623 — push hatchar back so getargs parses the leading ^.
        if let Some(ch) = char::from_u32(hat as u32) {
            crate::ported::input::inungetc(ch);                              // c:623
        }
        let ehist = match gethist(defev.load(Ordering::SeqCst)) {            // c:624
            Some(h) => h,
            None => return -1,                                               // c:626
        };
        let argc_local = getargc(&ehist) as usize;
        sline = match getargs(&ehist, 0, argc_local.saturating_sub(0)) {     // c:625
            Some(s) => s,
            None => return -1,                                               // c:626
        };

        if getsubsargs(&sline, &mut gbal, &mut cflag) != 0 {                 // c:628
            return substfailed();                                            // c:629
        }
        if hsubl.lock().unwrap().is_none() {                                 // c:630
            return -1;                                                       // c:631
        }
        let in_pat = hsubl.lock().unwrap().clone().unwrap_or_default();
        let out_pat = hsubr.lock().unwrap().clone().unwrap_or_default();
        let new = subst(&sline, &in_pat, &out_pat, gbal != 0);               // c:632
        if new == sline {                                                    // c:632 subst returned 0 (no match)
            return substfailed();                                            // c:633
        }
        sline = new;
    } else {
        // c:636 — !c shortcut: first-column flag clears unless c==' '.
        if c != b' ' as i32 {                                                // c:636
            crate::ported::lex::LEX_ISFIRSTCH.with(|f| f.set(false));                                            // c:637
        }
        let bc = bangchar.load(Ordering::SeqCst);
        if c == b'\\' as i32 {                                               // c:638
            let g = crate::ported::input::ingetc()                           // c:639 ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if g != bc {                                                     // c:641
                safeinungetc(g);                                             // c:642
            } else {                                                         // c:643
                qbang.store(true, Ordering::SeqCst);                         // c:644
                return bc;                                                   // c:645 return bangchar
            }
        }
        if c != bc {                                                         // c:648
            return c;                                                        // c:649
        }
        // c:650 — `*hptr = '\0'` truncates chline at the current position.
        let pos = hptr.load(Ordering::SeqCst);                               // c:650
        {
            let mut cl = chline.lock().unwrap();
            if pos < cl.len() {
                cl.truncate(pos);
            }
        }
        c = crate::ported::input::ingetc()                                   // c:651
            .map(|ch| ch as i32)
            .unwrap_or(-1);
        if c == b'{' as i32 {                                                // c:651
            bflag = 1;                                                       // c:652
            cflag = 1;
            c = crate::ported::input::ingetc()                               // c:653
                .map(|ch| ch as i32)
                .unwrap_or(-1);
        }
        if c == b'"' as i32 {                                                // c:655 c == '\"'
            stophist.store(1, Ordering::SeqCst);                             // c:656
            return crate::ported::input::ingetc()                            // c:657 return ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
        }
        // c:659 — (!cflag && inblank(c)) || c == '=' || c == '(' || lexstop
        let is_blank = (c as u8 as char).is_ascii_whitespace();
        if (cflag == 0 && is_blank)
            || c == b'=' as i32
            || c == b'(' as i32
            || lexstop.load(Ordering::SeqCst)                                // c:659
        {
            safeinungetc(c);                                                 // c:660
            return bc;                                                       // c:661 return bangchar
        }
        cflag = 0;                                                           // c:663
        let mut buflen: usize = 265;                                         // c:664
        buf = String::with_capacity(buflen);                                 // c:664 zhalloc

        // c:666-727 — read event-spec into buf.
        crate::ported::signals::queue_signals();                             // c:668
        if c == b'?' as i32 {                                                // c:669
            loop {                                                           // c:670
                c = crate::ported::input::ingetc()                           // c:671 ingetc()
                    .map(|ch| ch as i32)
                    .unwrap_or(-1);
                if c == b'?' as i32 || c == b'\n' as i32 || lexstop.load(Ordering::SeqCst) {  // c:672
                    break;                                                   // c:673
                } else {
                    buf.push(c as u8 as char);                               // c:675 *ptr++ = c
                    if buf.len() >= buflen {                                 // c:676
                        buflen *= 2;                                         // c:679 buflen *= 2
                        buf.reserve(buflen);
                    }
                }
            }
            if c != b'\n' as i32 && !lexstop.load(Ordering::SeqCst) {        // c:683
                c = crate::ported::input::ingetc()                           // c:684
                    .map(|ch| ch as i32)
                    .unwrap_or(-1);
            }
            // c:685 *ptr = '\0' — Rust String is already terminated.
            *hsubl.lock().unwrap() = Some(buf.clone());                      // c:686 hsubl = ztrdup(buf)
            // c:686 — `mev = ev = hconsearch(hsubl = ztrdup(buf), &marg);`
            // hconsearch now returns Option<(histnum, marg)> per C
            // out-pointer pair — previously dropped the marg side,
            // forcing the caller to hardcode margbox=0 which lost
            // the matched word index.
            let (ev_val, marg_val) = match hconsearch(&buf) {                // c:686
                Some((e, m)) => (e, m),
                None => (-1, -1),
            };
            ev = ev_val;
            mev.store(ev, Ordering::SeqCst);                                 // c:686 mev = ev
            marg.store(marg_val, Ordering::SeqCst);                          // c:686 marg out-arg
            evset = 0;                                                       // c:687
            if ev == -1 {                                                    // c:688
                herrflush();                                                 // c:689
                crate::ported::signals::unqueue_signals();                   // c:690
                crate::ported::utils::zerr(&format!("no such event: {}", buf));  // c:691
                return -1;                                                   // c:692
            }
        } else {                                                             // c:694
            // c:697 — collect event spec until terminator.
            loop {
                let is_term = (c as u8 as char).is_ascii_whitespace()
                    || c == b';' as i32 || c == b':' as i32 || c == b'^' as i32
                    || c == b'$' as i32 || c == b'*' as i32 || c == b'%' as i32
                    || c == b'}' as i32 || c == b'\'' as i32 || c == b'"' as i32
                    || c == b'`' as i32 || lexstop.load(Ordering::SeqCst);    // c:698-700
                if is_term { break; }
                if !buf.is_empty() {                                         // c:702
                    if c == b'-' as i32 { break; }                           // c:703-704
                    let first = buf.as_bytes()[0];
                    if (first.is_ascii_digit() || first == b'-')             // c:705
                        && !(c as u8).is_ascii_digit()                       // c:705 !idigit(c)
                    {
                        break;                                               // c:706
                    }
                }
                buf.push(c as u8 as char);                                   // c:708
                if buf.len() >= buflen {                                     // c:709
                    buflen *= 2;                                             // c:712
                    buf.reserve(buflen);
                }
                if c == b'#' as i32 || c == bc {                             // c:714
                    c = crate::ported::input::ingetc()                       // c:715
                        .map(|ch| ch as i32)
                        .unwrap_or(-1);
                    break;                                                   // c:716
                }
                c = crate::ported::input::ingetc()                           // c:718
                    .map(|ch| ch as i32)
                    .unwrap_or(-1);
            }
            if buf.is_empty()                                                // c:720
                && (c == b'}' as i32 || c == b';' as i32 || c == b'\'' as i32
                    || c == b'"' as i32 || c == b'`' as i32)
            {
                safeinungetc(c);                                             // c:723
                crate::ported::signals::unqueue_signals();                   // c:724
                return bc;                                                   // c:725
            }
            // c:727 *ptr = 0 — handled by Rust String
            if buf.is_empty() {                                              // c:728
                if c != b'%' as i32 {                                        // c:729
                    if isset(CSHJUNKIEHISTORY) {                             // c:730
                        ev = addhistnum(curhist.load(Ordering::SeqCst), -1, HIST_FOREIGN as i32);
                    } else {                                                 // c:732
                        ev = defev.load(Ordering::SeqCst);                   // c:733
                    }
                    if c == b':' as i32 && evset == -1 {                     // c:734
                        evset = 0;                                           // c:735
                    } else {
                        evset = 1;                                           // c:737
                    }
                } else {                                                     // c:738
                    if marg.load(Ordering::SeqCst) != -1 {                   // c:739
                        ev = mev.load(Ordering::SeqCst);                     // c:740
                    } else {                                                 // c:741
                        ev = defev.load(Ordering::SeqCst);                   // c:742
                    }
                    evset = 0;                                               // c:743
                }
            } else if let Ok(t0) = buf.trim().parse::<i64>() {               // c:745 zstrtol(buf, NULL, 10)
                if t0 != 0 {
                    ev = if t0 < 0 {                                         // c:746
                        addhistnum(curhist.load(Ordering::SeqCst), t0 as i32, HIST_FOREIGN as i32)
                    } else {
                        t0
                    };
                    evset = 1;                                               // c:747
                } else if buf.as_bytes()[0] == bc as u8 {                    // c:748 *buf == bangchar
                    ev = addhistnum(curhist.load(Ordering::SeqCst), -1, HIST_FOREIGN as i32);  // c:749
                    evset = 1;                                               // c:750
                } else if buf.as_bytes()[0] == b'#' {                        // c:751
                    ev = curhist.load(Ordering::SeqCst);                     // c:752
                    evset = 1;                                               // c:753
                } else {                                                     // c:754
                    match hcomsearch(&buf) {                                 // c:754
                        Some(e) => { ev = e; evset = 1; }
                        None => {
                            herrflush();                                     // c:755
                            crate::ported::signals::unqueue_signals();       // c:756
                            crate::ported::utils::zerr(&format!("event not found: {}", buf));  // c:757
                            return -1;                                       // c:758
                        }
                    }
                }
            } else if buf.as_bytes()[0] == bc as u8 {                        // c:748 *buf == bangchar
                ev = addhistnum(curhist.load(Ordering::SeqCst), -1, HIST_FOREIGN as i32);
                evset = 1;
            } else if buf.as_bytes()[0] == b'#' {                            // c:751
                ev = curhist.load(Ordering::SeqCst);
                evset = 1;
            } else {                                                         // c:754
                match hcomsearch(&buf) {
                    Some(e) => { ev = e; evset = 1; }
                    None => {
                        herrflush();
                        crate::ported::signals::unqueue_signals();
                        crate::ported::utils::zerr(&format!("event not found: {}", buf));
                        return -1;
                    }
                }
            }
        }

        // c:765 — fetch the resolved history entry.
        defev.store(ev, Ordering::SeqCst);                                   // c:765 defev = ev
        let mut ehist = match gethist(ev) {                                  // c:765
            Some(h) => h,
            None => {
                crate::ported::signals::unqueue_signals();                   // c:766
                return -1;                                                   // c:767
            }
        };
        argc = getargc(&ehist) as i32;                                       // c:771

        // c:772 — word-designator parsing.
        if c == b':' as i32 {                                                // c:772
            cflag = 1;                                                       // c:773
            c = crate::ported::input::ingetc()                               // c:774
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if c == b'%' as i32 && marg.load(Ordering::SeqCst) != -1 {       // c:775
                if evset == 0 {                                              // c:776
                    ehist = match gethist(mev.load(Ordering::SeqCst)) {      // c:777
                        Some(h) => { defev.store(mev.load(Ordering::SeqCst), Ordering::SeqCst); h }
                        None => {
                            crate::ported::signals::unqueue_signals();
                            return -1;
                        }
                    };
                    argc = getargc(&ehist) as i32;                           // c:778
                } else {                                                     // c:779
                    herrflush();                                             // c:780
                    crate::ported::signals::unqueue_signals();               // c:781
                    crate::ported::utils::zerr("ambiguous history reference");  // c:782
                    return -1;                                               // c:783
                }
            }
        }
        // c:788
        if c == b'*' as i32 {                                                // c:788
            farg = 1;                                                        // c:789
            larg = argc;                                                     // c:790
            cflag = 0;                                                       // c:791
        } else {                                                             // c:792
            if let Some(ch) = char::from_u32(c as u32) {
                crate::ported::input::inungetc(ch);                          // c:793
            }
            let r = getargspec(argc, marg.load(Ordering::SeqCst), evset);    // c:794
            larg = r; farg = r;
            if larg == -2 {                                                  // c:795
                crate::ported::signals::unqueue_signals();                   // c:796
                return -1;                                                   // c:797
            }
            if farg != -1 {                                                  // c:799
                cflag = 0;                                                   // c:800
            }
            c = crate::ported::input::ingetc()                               // c:801
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if c == b'*' as i32 {                                            // c:802
                cflag = 0;                                                   // c:803
                larg = argc;                                                 // c:804
            } else if c == b'-' as i32 {                                     // c:805
                cflag = 0;                                                   // c:806
                larg = getargspec(argc, marg.load(Ordering::SeqCst), evset); // c:807
                if larg == -2 {                                              // c:808
                    crate::ported::signals::unqueue_signals();               // c:809
                    return -1;                                               // c:810
                }
                if larg == -1 {                                              // c:812
                    larg = argc - 1;                                         // c:813
                }
            } else {                                                         // c:814
                if let Some(ch) = char::from_u32(c as u32) {
                    crate::ported::input::inungetc(ch);                      // c:815
                }
            }
        }
        if farg == -1 {                                                      // c:817
            farg = 0;                                                        // c:818
        }
        if larg == -1 {                                                      // c:819
            larg = argc;                                                     // c:820
        }
        sline = match getargs(&ehist, farg as usize, larg as usize) {        // c:821
            Some(s) => s,
            None => {
                crate::ported::signals::unqueue_signals();                   // c:822
                return -1;                                                   // c:823
            }
        };
        crate::ported::signals::unqueue_signals();                           // c:825
    }

    // c:830 — modifier loop.
    loop {
        c = if cflag != 0 { b':' as i32 } else {                             // c:831
            crate::ported::input::ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1)
        };
        cflag = 0;                                                           // c:832
        if c == b':' as i32 {                                                // c:833
            let mut gbal: i32 = 0;                                           // c:834
            c = crate::ported::input::ingetc()                               // c:836
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if c == b'g' as i32 {                                            // c:836
                gbal = 1;                                                    // c:837
                c = crate::ported::input::ingetc()                           // c:838
                    .map(|ch| ch as i32)
                    .unwrap_or(-1);
                if c != b's' as i32 && c != b'S' as i32 && c != b'&' as i32 {  // c:839
                    crate::ported::utils::zerr("'s' or '&' modifier expected after 'g'");  // c:840
                    return -1;                                               // c:841
                }
            }
            match c as u8 {
                b'p' => {                                                    // c:845
                    histdone.store(HISTFLAG_DONE | HISTFLAG_NOEXEC, Ordering::SeqCst);  // c:846
                }
                b'a' => {                                                    // c:848
                    match chabspath(&sline) {                                // c:849
                        Some(new) => sline = new,
                        None => {
                            herrflush();                                     // c:850
                            crate::ported::utils::zerr("modifier failed: a");// c:851
                            return -1;                                       // c:852
                        }
                    }
                }
                b'A' => {                                                    // c:856
                    match chrealpath(&sline) {                               // c:857
                        Some(new) => sline = new,
                        None => {
                            herrflush();                                     // c:858
                            crate::ported::utils::zerr("modifier failed: A");// c:859
                            return -1;                                       // c:860
                        }
                    }
                }
                b'c' => {                                                    // c:863
                    match crate::ported::subst::equalsubstr(&sline, false, false) {  // c:864
                        Some(new) => sline = new,
                        None => {
                            herrflush();                                     // c:865
                            crate::ported::utils::zerr("modifier failed: c");// c:866
                            return -1;                                       // c:867
                        }
                    }
                }
                b'h' => {                                                    // c:870
                    let count = digitcount(&sline) as i32;                   // c:871
                    sline = remtpath(&sline, count);
                }
                b'e' => {                                                    // c:877
                    sline = rembutext(&sline);                               // c:878
                }
                b'r' => {                                                    // c:884
                    sline = remtext(&sline);                                 // c:885
                }
                b't' => {                                                    // c:891
                    let count = digitcount(&sline) as i32;                   // c:892
                    sline = remlpaths(&sline, count);
                }
                b's' | b'S' => {                                             // c:898-899
                    hsubpatopt.store((c == b'S' as i32) as i32, Ordering::SeqCst);  // c:900
                    if getsubsargs(&sline, &mut gbal, &mut cflag) != 0 {     // c:901
                        return -1;                                           // c:902
                    }
                    // fall through to '&'                                   // c:902
                    let (in_pat, out_pat) = (
                        hsubl.lock().unwrap().clone(),
                        hsubr.lock().unwrap().clone(),
                    );
                    if let (Some(ip), Some(op)) = (in_pat, out_pat) {        // c:904
                        let new = subst(&sline, &ip, &op, gbal != 0);        // c:905
                        if new == sline {                                    // c:905 no match
                            return substfailed();                            // c:906
                        }
                        sline = new;
                    } else {                                                 // c:907
                        herrflush();                                         // c:908
                        crate::ported::utils::zerr("no previous substitution");  // c:909
                        return -1;                                           // c:910
                    }
                }
                b'&' => {                                                    // c:903
                    let (in_pat, out_pat) = (
                        hsubl.lock().unwrap().clone(),
                        hsubr.lock().unwrap().clone(),
                    );
                    if let (Some(ip), Some(op)) = (in_pat, out_pat) {
                        let new = subst(&sline, &ip, &op, gbal != 0);
                        if new == sline { return substfailed(); }
                        sline = new;
                    } else {
                        herrflush();
                        crate::ported::utils::zerr("no previous substitution");
                        return -1;
                    }
                }
                b'q' => {                                                    // c:913
                    sline = quote(&sline);                                   // c:914
                }
                b'Q' => {                                                    // c:916
                    // c:918-924 — `noerrs` flag stack is no-op in Rust port;
                    // see params.rs:1310. Tokenize-strip via parse_subst_string,
                    // then remnulargs + untokenize.
                    let oef = crate::ported::utils::errflag.load(Ordering::SeqCst);
                    let _ = crate::ported::lex::parse_subst_string(&sline);  // c:921
                    crate::ported::utils::errflag.store(
                        oef | (crate::ported::utils::errflag.load(Ordering::SeqCst)
                            & crate::ported::zsh_h::ERRFLAG_INT),
                        Ordering::SeqCst,
                    );                                                       // c:923
                    let mut s = sline.clone();
                    crate::ported::glob::remnulargs(&mut s);                 // c:924
                    sline = crate::ported::lex::untokenize(&s);              // c:925
                }
                b'x' => {                                                    // c:928
                    sline = quotebreak(&sline);                              // c:929
                }
                b'l' => {                                                    // c:931
                    sline = casemodify(&sline, CASMOD_LOWER);                // c:932
                }
                b'u' => {                                                    // c:934
                    sline = casemodify(&sline, CASMOD_UPPER);                // c:935
                }
                b'P' => {                                                    // c:937
                    if !sline.starts_with('/') {                             // c:938
                        if let Some(here) = crate::ported::utils::zgetcwd() {// c:939
                            sline = if here.ends_with('/') {
                                crate::ported::utils::dyncat(&here, &sline)  // c:943
                            } else {
                                // c:941 zhtricat(metafy(here, -1, META_HEAPDUP), "/", sline)
                                format!("{}/{}", here, sline)
                            };
                        }
                    }
                    match crate::ported::utils::xsymlink(&sline) {           // c:945
                        Some(new) => sline = new,
                        None => {} // C ignores xsymlink failure (returns NULL → keep sline)
                    }
                }
                _ => {                                                       // c:947 default
                    herrflush();                                             // c:948
                    crate::ported::utils::zerr(&format!("illegal modifier: {}", c as u8 as char));  // c:949
                    return -1;                                               // c:950
                }
            }
        } else {                                                             // c:952
            if c != b'}' as i32 || bflag == 0 {                              // c:953
                if let Some(ch) = char::from_u32(c as u32) {
                    crate::ported::input::inungetc(ch);                      // c:954
                }
            }
            if c != b'}' as i32 && bflag != 0 {                              // c:955
                crate::ported::utils::zerr("'}' expected");                  // c:956
                return -1;                                                   // c:957
            }
            break;                                                           // c:959
        }
    }

    // c:963 — zshlex_raw_back_to_mark(lexraw_mark): no-op until lex hook
    // exposes the raw-input mark/restore pair.
    let _ = lexraw_mark;                                                     // c:963

    // c:970-976 — push the expanded value onto the input stack as INP_HIST.
    lexstop.store(false, Ordering::SeqCst);                                  // c:970
    crate::ported::input::inpush(&sline, crate::ported::zsh_h::INP_HIST, None);  // c:976
    histdone.fetch_or(HISTFLAG_DONE, Ordering::SeqCst);                      // c:977
    if isset(HISTVERIFY) {                                                   // c:978
        histdone.fetch_or(HISTFLAG_NOEXEC | HISTFLAG_RECALL, Ordering::SeqCst);  // c:979
    }

    // c:982 — return ingetc() so caller sees the first char of expansion.
    crate::ported::input::ingetc()
        .map(|ch| ch as i32)
        .unwrap_or(-1)
}

/// Port of `void herrflush(void)` from Src/hist.c:477.
///
/// C body drains the input buffer after a history-expansion error,
/// feeding the consumed chars into the history-build cursor
/// (`hwaddc` + `addtoline`) so the history line still records the
/// raw input that failed. Without this drain, the history entry
/// for a failed `!ev` would be truncated at the failure point.
///
/// The previous Rust port left the drain loop as a no-op comment,
/// claiming the deps weren't ported. They ARE: `ihwaddc` /
/// `iaddtoline` live at hist.rs above; `strin` / `LEX_LEX_ADD_RAW`
/// are file-statics on this module + lex.rs.
pub fn herrflush() {                                                         // c:477
    // c:479 — `inpopalias();`
    crate::ported::input::inpopalias();

    // c:481-482 — `if (lexstop) return;`
    if crate::ported::lex::LEX_LEXSTOP.with(|f| f.get()) {
        return;
    }

    // c:494-500 — drain the input buffer when expanding history for
    // ZLE (the `!strin || lex_add_raw` arm covers the two cases where
    // the input must be flushed into the history line:
    //   - non-ZLE non-string input (`!strin` true);
    //   - ZLE with raw-recording (`lex_add_raw != 0`).
    //
    // C:
    //   while (inbufct && (!strin || lex_add_raw)) {
    //       int c = ingetc();
    //       if (!lexstop) { hwaddc(c); addtoline(c); }
    //   }
    loop {
        let inbufct = crate::ported::input::inbufct.with(|c| c.get());
        if inbufct <= 0 {
            break;
        }
        let strin_v = strin.load(Ordering::SeqCst);
        let lex_add_raw = crate::ported::lex::LEX_LEX_ADD_RAW.get();
        if !(strin_v == 0 || lex_add_raw != 0) {                             // c:494 (!strin || lex_add_raw)
            break;
        }
        let c = crate::ported::input::ingetc()                               // c:495 ingetc()
            .map(|ch| ch as i32)
            .unwrap_or(-1);
        if !crate::ported::lex::LEX_LEXSTOP.with(|f| f.get()) {              // c:496 if (!lexstop)
            ihwaddc(c);                                                       // c:497 hwaddc(c)
            iaddtoline(c);                                                    // c:498 addtoline(c)
        }
    }
}

/// Port of `int getargc(Histent ehist)` from Src/hist.c.
pub fn getargc(entry: &histent) -> usize {
    entry.nwords as usize
}

/// Port of `int substfailed(void)` from Src/hist.c:562.
/// ```c
/// static int
/// substfailed(void)
/// {
///     herrflush();
///     zerr("substitution failed");
///     return -1;
/// }
/// ```
pub fn substfailed() -> i32 {                                                // c:563
    herrflush();                                                             // c:565
    crate::ported::utils::zerr("substitution failed");                       // c:566
    -1                                                                       // c:567
}

/// Port of `int digitcount(char *s)` from Src/hist.c.
pub fn digitcount(s: &str) -> usize {                                        // c:574
    s.chars().take_while(|c| c.is_ascii_digit()).count()
}

/// Port of `void strinbeg(int dohist)` from `Src/hist.c:1033-1044`.
///
/// C body:
///     strin++;
///     hbegin(dohist);
///     lexinit();
///     init_parse_status();
///
/// The previous Rust port called only `strin++` and `hbegin` —
/// missing the `lexinit()` and `init_parse_status()` calls. Effect:
/// a fresh string-input scope inherited stale lexer state (token
/// classification tables, current-token buffers) AND stale parser
/// status (incomplete-expression flags), so subsequent strinbeg-
/// driven parses could misbehave with state from a prior parse.
pub fn strinbeg(dohist: i32) {                                                // c:1033
    strin.fetch_add(1, Ordering::SeqCst);                                     // c:1035
    hbegin(dohist);                                                           // c:1036
    crate::ported::lex::lexinit();                                            // c:1037
    crate::ported::parse::init_parse_status();                                // c:1042
}

/// Port of `void strinend(void)` from `Src/hist.c:1049-1056`.
///
/// C body:
///     hend(NULL);
///     DPUTS(!strin, "BUG: strinend() called without strinbeg()");
///     strin--;
///     isfirstch = 1;
///     histdone = 0;
///
/// The previous Rust port called only `hend` and decremented `strin`,
/// missing the `isfirstch = 1` and `histdone = 0` resets. Effect: the
/// next strinbeg-driven parse inherited stale "we're past the first
/// char" state from the prior scope, defeating `^foo^bar`-style
/// histsubchar shortcuts that key on `isfirstch`.
pub fn strinend() {                                                           // c:1049
    hend(None);                                                               // c:1051
    strin.fetch_sub(1, Ordering::SeqCst);                                     // c:1053
    crate::ported::lex::LEX_ISFIRSTCH.with(|f| f.set(true));                  // c:1054 isfirstch = 1
    histdone.store(0, Ordering::SeqCst);                                      // c:1055 histdone = 0
}

/// Port of `static void nohw(UNUSED(int c))` from Src/hist.c:1062.
pub fn nohw(_c: i32) { /* do nothing */ }                                    // c:1062

/// Port of `static void nohwabort(void)` from Src/hist.c:1067.
pub fn nohwabort() { /* do nothing */ }                                      // c:1067

/// Port of `static void nohwe(void)` from Src/hist.c:1072.
pub fn nohwe() { /* do nothing */ }                                          // c:1072

/// Port of `void ihwbegin(int offset)` from `Src/hist.c:1656`.
///
/// C gate:
/// ```c
/// if (stophist == 2 || (histactive & HA_INWORD) ||
///     (inbufflags & (INP_ALIAS|INP_HIST)) == INP_ALIAS)
///     return;
/// ```
///
/// The previous Rust port checked only `stophist == 2 || HA_INWORD`,
/// missing the `INP_ALIAS-only` gate. Effect: word-position
/// recording fired during alias expansion (when `INP_ALIAS` is set
/// and `INP_HIST` is not), capturing alias-substituted bytes as
/// fresh words. The C source skips alias-only input from the
/// history-word table because aliases are pre-expansion and
/// shouldn't show up as user-typed words in `!:N` etc.
pub fn ihwbegin(offset: i32) {                                               // c:1656
    use crate::ported::zsh_h::{INP_ALIAS, INP_HIST};
    let stop = stophist.load(Ordering::SeqCst);
    let active = histactive.load(Ordering::SeqCst);
    let inflags = crate::ported::input::inbufflags.with(|f| f.get());
    // c:1659 — `(inbufflags & (INP_ALIAS|INP_HIST)) == INP_ALIAS`
    // means "alias-only input (no history layered above)".
    if stop == 2
        || (active & HA_INWORD) != 0
        || (inflags & (INP_ALIAS | INP_HIST)) == INP_ALIAS                  // c:1659
    {
        return;
    }
    let pos = chwordpos.load(Ordering::SeqCst);
    if pos % 2 != 0 {                                                        // c:1664
        chwordpos.fetch_sub(1, Ordering::SeqCst);
    }
    let start = (chline.lock().unwrap().len() as i32 + offset).max(0) as i16;
    let mut words = chwords.lock().unwrap();
    let idx = chwordpos.load(Ordering::SeqCst) as usize;
    if words.len() <= idx {
        words.resize(idx + 1, 0);
    }
    words[idx] = start;                                                      // c:1668
    chwordpos.fetch_add(1, Ordering::SeqCst);
}

/// Port of `static void linkcurline(void)` from Src/hist.c:1079.
pub fn linkcurline() {                                                       // c:1079
    let new_hist = curhist.fetch_add(1, Ordering::SeqCst) + 1;               // c:1093 ++curhist
    let mut cur = curline.lock().unwrap();
    *cur = Some(make_histent(new_hist, String::new()));                      // c:1093 curline.histnum
    // Splicing into the ring (c:1081-1088) is encoded by the Vec::insert
    // at hist_ring index 0 done by hend() on commit. The sentinel itself
    // lives in `curline` until then.
}

/// Port of `static void unlinkcurline(void)` from Src/hist.c:1093.
pub fn unlinkcurline() {                                                     // c:1093
    *curline.lock().unwrap() = None;                                         // c:1093-1102
    curhist.fetch_sub(1, Ordering::SeqCst);                                  // c:1103
}

/// Port of `void hbegin(int dohist)` from Src/hist.c:1110.
pub fn hbegin(dohist: i32) {                                                 // c:1110
    // isfirstln/isfirstch live in the lex.rs LEX_* thread_locals, not as
    // globals — caller resets them via lexer instance API.            // c:1114

    crate::ported::utils::errflag.fetch_and(                                 // c:1115
        !crate::ported::utils::ERRFLAG_ERROR,
        Ordering::Relaxed,
    );
    histdone.store(0, Ordering::SeqCst);                                     // c:1116
    // c:1117 — `isset(INTERACTIVE)` / `isset(SHINSTDIN)`.
    let interact = isset(INTERACTIVE);
    let shinstdin = isset(SHINSTDIN);
    if dohist == 0 {                                                         // c:1117
        stophist.store(2, Ordering::SeqCst);                                 // c:1118
    } else if dohist != 2 {                                                  // c:1119
        stophist.store(if !interact || !shinstdin { 2 } else { 0 },          // c:1120
            Ordering::SeqCst);
    } else {                                                                 // c:1121
        stophist.store(0, Ordering::SeqCst);                                 // c:1122
    }

    if stophist.load(Ordering::SeqCst) == 2 {                                // c:1134
        chline.lock().unwrap().clear();                                      // c:1135 chline = NULL
        hptr.store(0, Ordering::SeqCst);                                     // c:1135 hptr = NULL
        hlinesz.store(0, Ordering::SeqCst);                                  // c:1136
        chwords.lock().unwrap().clear();                                     // c:1137
        chwordlen.store(0, Ordering::SeqCst);                                // c:1138
        // hgetc/hungetc/hwaddc/hwbegin/hwabort/hwend/addtoline are       c:1139-1145
        // function-pointer slots in C; Rust dispatches statically.
    } else {                                                                 // c:1146
        let mut buf = chline.lock().unwrap();                                // c:1147
        buf.clear();
        buf.reserve(64);
        hlinesz.store(64, Ordering::SeqCst);                                 // c:1147
        drop(buf);
        let mut w = chwords.lock().unwrap();                                 // c:1148
        w.clear();
        w.reserve(64);
        chwordlen.store(64, Ordering::SeqCst);
        drop(w);
        // hgetc/hungetc/hwaddc/hwbegin/hwabort/hwend/addtoline c:1149-1155 — see c:1139.
        if !isset(BANGHIST) {                                                // c:1156
            stophist.store(4, Ordering::SeqCst);                             // c:1157
        }
    }
    chwordpos.store(0, Ordering::SeqCst);                                    // c:1159

    {                                                                        // c:1161
        let mut ring = hist_ring.lock().unwrap();
        if let Some(top) = ring.first_mut() {
            if top.ftim == 0 && strin.load(Ordering::SeqCst) == 0 {
                top.ftim = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);                                           // c:1162
            }
        }
    }
    if (dohist == 2 || (interact && shinstdin))                              // c:1163
        && strin.load(Ordering::SeqCst) == 0
    {
        histactive.store(HA_ACTIVE, Ordering::SeqCst);                       // c:1164
        // c:1165 — `attachtty(mypgrp);` reclaims the controlling
        // terminal for the shell's pgrp at the start of a fresh
        // history-recording line. The previous Rust port left this
        // as a comment-only stub claiming "TTY infra not ported"
        // — but `utils::attachtty` AND the `MYPGRP` global ARE
        // both ported (utils.rs:3593, jobs.rs:2585). Wire the
        // call so the shell actually grabs the tty during
        // interactive history sessions, matching C behavior.
        let mypgrp = *crate::ported::jobs::MYPGRP
            .get_or_init(|| std::sync::Mutex::new(0))
            .lock()
            .expect("mypgrp poisoned");
        crate::ported::utils::attachtty(mypgrp);                             // c:1165
        linkcurline();                                                       // c:1166
        defev.store(addhistnum(curhist.load(Ordering::SeqCst),               // c:1167
                               -1, HIST_FOREIGN as i32),
                    Ordering::SeqCst);
    } else {
        histactive.store(HA_ACTIVE | HA_NOINC, Ordering::SeqCst);            // c:1169
    }

    if isset(INCAPPENDHISTORYTIME) // c:1189
        && !isset(SHAREHISTORY)
        && !isset(INCAPPENDHISTORY)
        && (histactive.load(Ordering::SeqCst) & HA_NOINC) == 0
        && strin.load(Ordering::SeqCst) == 0
        && histsave_stack_pos.load(Ordering::SeqCst) == 0
    {
        let hf = resolve_histfile();                                         // c:1192
        savehistfile(hf.as_deref(), 0                                        // c:1193
            | HFILE_USE_OPTIONS as i32
            | HFILE_FAST as i32);
    }
}

/// Port of `void histreduceblanks(void)` from `Src/hist.c:1199-1248`.
///
/// **Signature divergence from C**: C operates on global `chline`
/// + `chwords` state and returns void. Rust port takes a `&str`
/// input and returns the collapsed result — the in-tree caller
/// (hist.rs:1388) threads text through this Rust shape. A future
/// refactor that wires the global-state path would deprecate this
/// signature.
///
/// **Whitespace class fix**: C uses `inblank(*ptr)` at c:1240 — the
/// NARROW typtab class (space + tab only per Src/ztype.h:50). The
/// previous Rust port used `c.is_whitespace()` (broad Unicode)
/// which also catches CR/FF/VT/NBSP — silently collapsing those
/// chars that C would preserve. Now matches C's narrow inblank.
pub fn histreduceblanks(text: &str) -> String {
    // c:50 — `inblank` is space/tab only.
    #[inline]
    fn is_inblank_narrow(c: char) -> bool {
        c == ' ' || c == '\t'
    }

    let mut result = String::with_capacity(text.len());
    let mut prev_space = false;
    for c in text.chars() {
        if is_inblank_narrow(c) {
            if !prev_space { result.push(' '); prev_space = true; }
        } else {
            result.push(c); prev_space = false;
        }
    }
    // c:1240 — trim trailing inblank only; preserve embedded non-
    // inblank chars (newline, CR, etc).
    let mut s = result;
    while s.ends_with(' ') { s.pop(); }
    while s.starts_with(' ') { s.remove(0); }
    s
}

/// Port of `void histremovedups(void)` from Src/hist.c.
pub fn histremovedups() {
    let mut ring = hist_ring.lock().unwrap();
    let mut seen = std::collections::HashSet::new();
    ring.retain(|h| seen.insert(h.node.nam.clone()));
    let new_ct = ring.len() as i64;
    drop(ring);
    histlinect.store(new_ct, Ordering::SeqCst);
}

/// Port of `zlong addhistnum(zlong hl, int n, int xflags)` from Src/hist.c:1266.
pub fn addhistnum(hl: i64, mut n: i32, xflags: i32) -> i64 {                 // c:1266
    let dir: i32 = if n < 0 { -1 } else if n > 0 { 1 } else { 0 };           // c:1266
    let he = gethistent(hl, dir);                                            // c:1269
    let he = match he {
        None => return 0,                                                    // c:1271-1272
        Some(h) => h,
    };
    if he != hl {                                                            // c:1273
        n -= dir;                                                            // c:1274
    }
    let final_he = if n != 0 {                                               // c:1275
        movehistent(he, n, xflags as u32)                                    // c:1276
    } else {
        Some(he)
    };
    match final_he {                                                         // c:1277
        None => {
            if dir < 0 {                                                     // c:1278
                firsthist() - 1
            } else {
                curhist.load(Ordering::SeqCst) + 1
            }
        }
        Some(h) => h,                                                        // c:1279
    }
}

/// Port of `Histent movehistent(Histent he, int n, int xflags)` from `Src/hist.c:1284`.
///
/// The previous Rust port omitted the `checkcurline(he)` call at
/// c:1298 — when the walk lands on the in-flight history entry,
/// C flushes the current chline/chwordpos/chwords build state into
/// `curline` so the caller sees fresh word data. Without it, code
/// that walks back to the current entry and then reads `curline`
/// got stale word data.
pub fn movehistent(start: i64, mut n: i32, xflags: u32) -> Option<i64> {         // c:1284
    let mut cur = start;
    while n < 0 {                                                                // c:1286
        cur = up_histent(cur)?;                                                  // c:1287
        if let Some(e) = ring_get(cur) {
            if (e.node.flags as u32 & xflags) == 0 {                             // c:1289
                n += 1;                                                          // c:1290
            }
        }
    }
    while n > 0 {                                                                // c:1292
        cur = down_histent(cur)?;                                                // c:1293
        if let Some(e) = ring_get(cur) {
            if (e.node.flags as u32 & xflags) == 0 {                             // c:1295
                n -= 1;                                                          // c:1296
            }
        }
    }
    // c:1298 — `checkcurline(he);` flushes in-flight build state
    // into `curline` if the walk landed on the active history entry.
    if let Some(e) = ring_get(cur) {
        checkcurline(&e);                                                        // c:1298
    }
    Some(cur)                                                                    // c:1299
}

/// Port of `Histent up_histent(Histent he)` from Src/hist.c.
pub fn up_histent(current: i64) -> Option<i64> {                             // c:1304
    let pos = ring_position(current)?;                                       // c:1306 !he
    (pos + 1 < ring_len()).then(|| ring_at(pos + 1))                         // c:1306 he->up == hist_ring? NULL : he->up
}

/// Port of `Histent down_histent(Histent he)` from Src/hist.c.
pub fn down_histent(current: i64) -> Option<i64> {                           // c:1311
    let pos = ring_position(current)?;
    (pos > 0).then(|| ring_at(pos - 1))                                      // c:1313 he == hist_ring? NULL : he->down
}

/// Port of `Histent gethistent(zlong ev, int nearmatch)` from Src/hist.c.
pub fn gethistent(ev: i64, nearmatch: i32) -> Option<i64> {                  // c:1318
    if ring_len() == 0 {
        return None;
    }
    if ring_get(ev).is_some() {
        return Some(ev);
    }
    if nearmatch == 0 {
        return None;
    }
    let mut best_older: Option<i64> = None;
    let mut best_newer: Option<i64> = None;
    for i in 0..ring_len() {
        let n = ring_at(i);
        if n < ev && best_older.map_or(true, |b| n > b) {
            best_older = Some(n);
        } else if n > ev && best_newer.map_or(true, |b| n < b) {
            best_newer = Some(n);
        }
    }
    if nearmatch < 0 { best_older } else { best_newer }
}

/// Port of `int putoldhistentryontop(int keep_going)` from Src/hist.c.
/// Rust idiom replacement: `Vec::remove`+`insert(0, …)` on the
/// hist_ring deque replaces the C doubly-linked-list relink
/// (curhist->down/up + lastnode/firstnode pointer dance).
pub fn putoldhistentryontop(_keep_going: i32) -> i32 {
    let mut ring = hist_ring.lock().unwrap();
    if let Some(oldest) = ring.last().map(|h| h.histnum) {
        let pos = ring.iter().position(|h| h.histnum == oldest).unwrap();
        let entry = ring.remove(pos);
        ring.insert(0, entry);
        return 1;
    }
    0
}

/// Port of `Histent prepnexthistent(void)` from Src/hist.c.
pub fn prepnexthistent() -> i64 {                                            // c:1387
    let cap = histsiz.load(Ordering::SeqCst);
    if histlinect.load(Ordering::SeqCst) >= cap {
        if let Some(oldest) = ring_oldest() {
            // Drop oldest from ring
            let mut ring = hist_ring.lock().unwrap();
            ring.retain(|h| h.histnum != oldest);
            histlinect.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let n = curhist.fetch_add(1, Ordering::SeqCst) + 1;
    n
}

/// Port of `static int should_ignore_line(Eprog prog)` from Src/hist.c:1425.
fn should_ignore_line(prog: Option<&[u8]>) -> i32 {                          // c:1425
    let line = chline.lock().unwrap().clone();
    if isset(HISTIGNORESPACE) {                                              // c:1427
        if line.starts_with(' ') /* aliasspaceflag — alias state TBD */ {    // c:1428
            return 1;                                                        // c:1429
        }
    }
    if prog.is_none() {                                                      // c:1432
        return 0;                                                            // c:1433
    }
    if isset(HISTNOFUNCTIONS) {                                              // c:1435
        // Inspecting an Eprog requires the wordcode VM port — leave the
        // funcdef detection to the executor; conservatively return 0.    // c:1436-1440
        return 0;
    }
    if isset(HISTNOSTORE) {                                                  // c:1443
        // getjobtext(prog, NULL) — text reconstruction also needs the
        // wordcode VM. Apply the simpler text-based filters on chline
        // for the cases the C code carves out.
        let mut b: &str = &line;
        let mut saw_builtin = false;
        if let Some(rest) = b.strip_prefix("builtin ") {                     // c:1446
            b = rest;
            saw_builtin = true;
        }
        if (b == "history" || b.starts_with("history "))                     // c:1451
            && (saw_builtin /* || shfunctab.getnode("history").is_none() */)
        {
            return 1;                                                        // c:1453
        }
        if (b == "r" || b.starts_with("r "))                                 // c:1454
            && (saw_builtin /* || shfunctab.getnode("r").is_none() */)
        {
            return 1;
        }
        if let Some(rest) = b.strip_prefix("fc -") {                         // c:1457
            if (saw_builtin /* || shfunctab.getnode("fc").is_none() */)
                && rest.chars().take_while(|c| c.is_ascii_alphabetic())
                       .any(|c| c == 'l')
            {
                return 1;                                                    // c:1474
            }
        }
    }
    0                                                                        // c:1474
}

/// Port of `int hend(Eprog prog)` from Src/hist.c:1474.
pub fn hend(prog: Option<&[u8]>) -> i32 {                                    // c:1474
    let stack_pos = histsave_stack_pos.load(Ordering::SeqCst);               // c:1474
    let mut save: i32 = 1;                                                   // c:1484
    let mut hookret: i32 = 0;

    // DPUTS(stophist != 2 && !(inbufflags & INP_ALIAS) && !chline,       c:1487
    //       "BUG: chline is NULL in hend()");
    crate::ported::signals::queue_signals();                                 // c:1489
    if (histdone.load(Ordering::SeqCst) & HISTFLAG_SETTY) != 0 {             // c:1490
        // settyinfo(&shttyinfo) — TTY-state singleton not ported.          // c:1491
    }
    let active = histactive.load(Ordering::SeqCst);
    if (active & HA_NOINC) == 0 {                                            // c:1492
        unlinkcurline();                                                     // c:1493
    }
    if (active & HA_NOINC) != 0 {                                            // c:1494
        chline.lock().unwrap().clear();                                      // c:1495 zfree(chline)
        chwords.lock().unwrap().clear();                                     // c:1496 zfree(chwords)
        hptr.store(0, Ordering::SeqCst);                                     // c:1497
        histactive.store(0, Ordering::SeqCst);                               // c:1499
        crate::ported::signals::unqueue_signals();                           // c:1500
        return 1;                                                            // c:1501
    }
    let cur_ignore_all = if isset(HISTIGNOREALLDUPS) { 1 } else { 0 };       // c:1503
    let prev_ignore_all = hist_ignore_all_dups.load(Ordering::SeqCst);
    if prev_ignore_all != cur_ignore_all                                     // c:1503
        && {
            hist_ignore_all_dups.store(cur_ignore_all, Ordering::SeqCst);    // c:1504
            cur_ignore_all != 0
        }
    {
        histremovedups();                                                    // c:1505
    }
    // *hptr = '\0';                                                         // c:1513 — String is implicit
    let chline_text = chline.lock().unwrap().clone();
    if !chline_text.is_empty() {                                             // c:1515
        let save_errflag = crate::ported::utils::errflag                     // c:1517
            .load(Ordering::Relaxed);
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);           // c:1518
        let args = vec!["zshaddhistory".to_string(), chline_text.clone()];   // c:1520-1521
        hookret = crate::ported::utils::callhookfunc(                        // c:1522
            "zshaddhistory", Some(&args), true);
        let new_errflag = (crate::ported::utils::errflag                     // c:1524-1525
            .load(Ordering::Relaxed)
            & !crate::ported::utils::ERRFLAG_ERROR) | save_errflag;
        crate::ported::utils::errflag.store(new_errflag, Ordering::Relaxed);
    }
    let hf = resolve_histfile();                                             // c:1528
    if isset(SHAREHISTORY)                                                   // c:1529
        && lockhistfile(hf.as_deref(), 0) == 0
    {
        readhistfile(hf.as_deref(), 0,                                       // c:1530
            HFILE_USE_OPTIONS as i32 | HFILE_FAST as i32);
        // curline.histnum = curhist + 1                                     // c:1531
    }
    let flag = histdone.load(Ordering::SeqCst);                              // c:1533
    histdone.store(0, Ordering::SeqCst);                                     // c:1534
    let hptr_pos = hptr.load(Ordering::SeqCst);
    let mut text = chline_text;
    if hptr_pos < 1 {                                                        // c:1535 hptr < chline + 1
        save = 0;                                                            // c:1536
    } else {
        if text.ends_with('\n') {                                            // c:1538 hptr[-1] == '\n'
            if text.len() > 1 {                                              // c:1539 chline[1]
                text.pop();                                                  // c:1540 *--hptr = '\0'
                if hptr.load(Ordering::SeqCst) > 0 {
                    hptr.fetch_sub(1, Ordering::SeqCst);
                }
            } else {
                save = 0;                                                    // c:1542
            }
        }
        if chwordpos.load(Ordering::SeqCst) <= 2                             // c:1544
            && hist_keep_comment.load(Ordering::SeqCst) == 0
        {
            save = 0;                                                        // c:1545
        } else if should_ignore_line(prog) != 0 {                            // c:1546
            save = -1;                                                       // c:1547
        } else if hookret == 2 {                                             // c:1548
            save = -2;                                                       // c:1549
        } else if hookret != 0 {                                             // c:1550
            save = -1;                                                       // c:1551
        }
    }
    if (flag & (HISTFLAG_DONE | HISTFLAG_RECALL)) != 0 {                     // c:1553
        let ptr = text.clone();                                              // c:1556 ztrdup(chline)
        if (flag & (HISTFLAG_DONE | HISTFLAG_RECALL)) == HISTFLAG_DONE {     // c:1557
            // zputs(ptr, shout); fputc('\n', shout); fflush(shout);         // c:1558-1560
            print!("{}\n", ptr);
            let _ = std::io::stdout().flush();
        }
        if (flag & HISTFLAG_RECALL) != 0 {                                   // c:1562
            // zpushnode(bufstack, ptr) — bufstack push not yet wired here. // c:1563
            save = 0;                                                        // c:1564
        }
    }
    if save != 0 || text.starts_with(' ') {                                  // c:1568
        // Walk up the ring skipping HIST_FOREIGN entries; if the topmost
        // non-foreign entry is HIST_TMPSTORE, drop it.                     // c:1569-1576
        let mut ring = hist_ring.lock().unwrap();
        let mut idx: usize = 0;
        while idx < ring.len()
            && (ring[idx].node.flags as u32 & HIST_FOREIGN) != 0
        {
            idx += 1;
        }
        if idx < ring.len()
            && (ring[idx].node.flags as u32 & HIST_TMPSTORE) != 0
        {
            if idx == 0 {                                                    // c:1573 he == hist_ring
                curhist.fetch_sub(1, Ordering::SeqCst);                      // c:1574
            }
            ring.remove(idx);                                                // c:1575 freehistnode
            histlinect.fetch_sub(1, Ordering::SeqCst);
        }
    }
    if save != 0 {                                                           // c:1578
        // chwordpos parity guard — if odd, hwend() to close.            // c:1583-1587
        if chwordpos.load(Ordering::SeqCst) % 2 != 0 {
            ihwend();
        }
        // Strip trailing \n which we already nulled out.                    // c:1589-1595
        let cwp = chwordpos.load(Ordering::SeqCst);
        if cwp > 1 {
            let words = chwords.lock().unwrap();
            let last = words.get((cwp - 2) as usize).copied().unwrap_or(0);
            // C: !chline[chwords[chwordpos-2]] — index past end after NUL.
            if (last as usize) >= text.len() {                               // c:1590
                drop(words);
                chwordpos.fetch_sub(2, Ordering::SeqCst);
            } else {
                drop(words);
            }
            if isset(HISTREDUCEBLANKS) {                                     // c:1593
                text = histreduceblanks(&text);                              // c:1594
            }
        }
        let newflags: u32 = if save == -1 { HIST_TMPSTORE }                  // c:1596-1601
            else if save == -2 { HIST_NOWRITE }
            else { 0 };
        let mut he_idx: Option<usize> = None;
        let mut overwrite_old: u32 = 0;
        if (isset(HISTIGNOREDUPS) || isset(HISTIGNOREALLDUPS))                // c:1602
            && save > 0
        {
            let ring = hist_ring.lock().unwrap();
            if let Some(top) = ring.first() {
                if top.node.nam == text {                                    // c:1603 histstrcmp
                    overwrite_old = top.node.flags as u32 & HIST_OLD;        // c:1610
                    he_idx = Some(0);
                }
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let cwp = chwordpos.load(Ordering::SeqCst);
        let chwords_snapshot: Vec<i16> = chwords.lock().unwrap().clone();
        let nwords = (cwp / 2) as i32;
        if let Some(0) = he_idx {                                            // c:1609 reuse top
            let mut ring = hist_ring.lock().unwrap();
            if let Some(top) = ring.first_mut() {
                top.node.nam = text.clone();                                 // c:1616
                top.stim = now;                                              // c:1617
                top.ftim = 0;                                                // c:1618
                top.node.flags = (newflags | overwrite_old) as i32;          // c:1619
                top.nwords = nwords;                                         // c:1621
                top.words = if cwp > 0 {
                    chwords_snapshot[..cwp as usize].to_vec()                // c:1622-1623
                } else {
                    Vec::new()
                };
            }
        } else {
            let n = prepnexthistent();                                       // c:1614
            let mut he = make_histent(n, text.clone());
            he.stim = now;
            he.ftim = 0;
            he.node.flags = newflags as i32;
            he.nwords = nwords;
            if cwp > 0 {
                he.words = chwords_snapshot[..cwp as usize].to_vec();
            }
            let mut ring = hist_ring.lock().unwrap();
            ring.insert(0, he);
            histlinect.fetch_add(1, Ordering::SeqCst);
            if (newflags & HIST_TMPSTORE) == 0 {                             // c:1625
                // addhistnode(histtab, he->node.nam, he) — hashtable wiring c:1626
                // routes through crate::ported::hashtable::addhistnode.
                crate::ported::hashtable::addhistnode(&text, n as i32);
            }
        }
    }
    chline.lock().unwrap().clear();                                          // c:1628 zfree(chline)
    chwords.lock().unwrap().clear();                                         // c:1629 zfree(chwords)
    hptr.store(0, Ordering::SeqCst);                                         // c:1630
    histactive.store(0, Ordering::SeqCst);                                   // c:1632

    let share = isset(SHAREHISTORY);
    let do_inc = if share {
        histfileIsLocked() != 0                                              // c:1636
    } else {
        isset(INCAPPENDHISTORY)                                              // c:1637
            || (isset(INCAPPENDHISTORYTIME)                                  // c:1637
                && histsave_stack_pos.load(Ordering::SeqCst) != 0)           // c:1638
    };
    if do_inc {
        savehistfile(hf.as_deref(), 0                                        // c:1639
            | HFILE_USE_OPTIONS as i32
            | HFILE_FAST as i32);
    }
    unlockhistfile(hf.as_deref().unwrap_or(""));                             // c:1640

    while histsave_stack_pos.load(Ordering::SeqCst) > stack_pos {            // c:1645
        pophiststack();                                                      // c:1646
    }
    hist_keep_comment.store(0, Ordering::SeqCst);                            // c:1647
    crate::ported::signals::unqueue_signals();                               // c:1648
    if (flag & HISTFLAG_NOEXEC) != 0
        || crate::ported::utils::errflag.load(Ordering::Relaxed) != 0 {
        0                                                                    // c:1649
    } else {
        1
    }
}

/// Port of `void ihwabort(void)` from Src/hist.c.
pub fn ihwabort() {                                                          // c:1675
    let pos = chwordpos.load(Ordering::SeqCst);
    if pos % 2 != 0 {
        chwordpos.fetch_sub(1, Ordering::SeqCst);
    }
    hist_keep_comment.store(1, Ordering::SeqCst);
}

/// Port of `void ihwend(void)` from `Src/hist.c:1686`.
///
/// Same gate as `ihwbegin` at c:1688:
///   stophist == 2 || (histactive & HA_INWORD) ||
///   (inbufflags & (INP_ALIAS|INP_HIST)) == INP_ALIAS
///
/// The previous Rust port missed the INP_ALIAS-only arm
/// (matching the `ihwbegin` divergence). Words started during
/// alias expansion got closed too — corrupting the chwords
/// table when an alias expansion ended.
pub fn ihwend() {                                                            // c:1686
    use crate::ported::zsh_h::{INP_ALIAS, INP_HIST};
    let stop = stophist.load(Ordering::SeqCst);
    let active = histactive.load(Ordering::SeqCst);
    let inflags = crate::ported::input::inbufflags.with(|f| f.get());
    if stop == 2
        || (active & HA_INWORD) != 0
        || (inflags & (INP_ALIAS | INP_HIST)) == INP_ALIAS                  // c:1688
    {
        return;
    }
    let pos = chwordpos.load(Ordering::SeqCst);
    if pos % 2 == 0 {
        return;
    }
    let cur = chline.lock().unwrap().len() as i16;
    let mut words = chwords.lock().unwrap();
    let start_idx = (pos - 1) as usize;
    if cur > words[start_idx] {
        let end_idx = pos as usize;
        if words.len() <= end_idx {
            words.resize(end_idx + 1, 0);
        }
        words[end_idx] = cur;
        chwordpos.fetch_add(1, Ordering::SeqCst);
    } else {
        chwordpos.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Port of `void histbackword(void)` from `Src/hist.c:1711`.
///
/// C body:
/// ```c
/// if (!(chwordpos%2) && chwordpos)
///     hptr = chline + chwords[chwordpos-1];
/// ```
///
/// Go back to immediately after the last word, skipping space.
/// Operates on the globals `chwordpos`, `chwords`, `chline`, `hptr`.
///
/// The previous Rust port had a completely different signature
/// (`(line: &str, pos: usize) -> usize`) and walked back through
/// ASCII whitespace — a stand-alone scan helper unrelated to the
/// C function. C operates on the GLOBAL history-line cursor via
/// `chwords[chwordpos-1]`, not an arbitrary text scan.
///
/// No call sites in src/ used the Rust signature, so renaming is
/// safe. Pin the C semantic: rewind `hptr` to the start of the
/// previous word when we're at a word-boundary.
pub fn histbackword() {                                                       // c:1711
    let pos = chwordpos.load(Ordering::SeqCst);
    // c:1714 — `if (!(chwordpos%2) && chwordpos)`. Both conditions
    // — even (word boundary) AND non-zero.
    if pos % 2 == 0 && pos != 0 {                                            // c:1714
        let words = chwords.lock().unwrap();
        let idx = (pos - 1) as usize;
        if idx < words.len() {
            // c:1715 — `hptr = chline + chwords[chwordpos-1]`. Rust's
            // hptr is an offset into chline; assign the cached
            // word-start offset (clamp negative to 0 since hptr
            // is AtomicUsize, matching C's pointer-arithmetic
            // semantic where chline + negative-offset is UB and
            // would have been clamped by the parser earlier).
            let off = (words[idx] as i32).max(0) as usize;
            hptr.store(off, Ordering::SeqCst);                                // c:1715
        }
    }
}

/// Port of `int hwget(char **startptr)` from Src/hist.c.
pub fn hwget() -> Option<(i32, String)> {
    let pos = chwordpos.load(Ordering::SeqCst);
    if pos == 0 || pos % 2 != 0 { return None; }
    let words = chwords.lock().unwrap();
    let start_idx = (pos - 2) as usize;
    let end_idx = (pos - 1) as usize;
    if end_idx >= words.len() { return None; }
    let start = words[start_idx];
    let end = words[end_idx];
    let line = chline.lock().unwrap();
    let s = (start.max(0)) as usize;
    let e = (end.max(0) as usize).min(line.len());
    if s > e || s >= line.len() { return None; }
    Some((start as i32, line[s..e].to_string()))
}

/// Port of `void hwrep(char *rep)` from `Src/hist.c:1748`.
///
/// C body:
/// ```c
/// char *start;
/// hwget(&start);
/// if (!strcmp(rep, start)) return;
/// hptr = start;
/// chwordpos = chwordpos - 2;
/// hwbegin(0);
/// qbang = 1;
/// while (*rep) hwaddc(*rep++);
/// hwend();
/// ```
///
/// Replace the current history word with `rep` (the LAST recorded
/// word — `chwordpos - 2` indexes its start). The previous Rust
/// port had a completely different signature `(entry, replacement,
/// word_idx)` operating on a completed `histent` struct with
/// whitespace-split replacement. C operates on the IN-FLIGHT
/// `chline` / `chwordpos` / `hptr` globals — a fundamentally
/// different abstraction.
///
/// No call sites in src/ used the Rust signature, so renaming is
/// safe. Port faithfully to operate on the globals.
pub fn hwrep(rep: &str) {                                                     // c:1748
    // c:1752 — `hwget(&start)` — get the current word's start offset.
    let (start_off, start_text) = match hwget() {
        Some(v) => v,
        None => return,
    };
    // c:1754 — `if (!strcmp(rep, start)) return;` — no change, skip.
    if rep == start_text {
        return;
    }
    // c:1756 — `hptr = start; chwordpos -= 2;`. Rewind to the start
    // of the word we're rewriting; the open word slot is conceptually
    // re-opened by the chwordpos decrement.
    hptr.store(start_off.max(0) as usize, Ordering::SeqCst);                  // c:1756
    chwordpos.fetch_sub(2, Ordering::SeqCst);                                 // c:1757
    // c:1758 — `hwbegin(0);` re-open at current hptr (no offset).
    ihwbegin(0);
    // c:1759 — `qbang = 1;` mark word as bang-bearing so subsequent
    // ihwaddc bang-escapes correctly.
    qbang.store(true, Ordering::SeqCst);                                      // c:1759
    // c:1760 — `while (*rep) hwaddc(*rep++);` — push each byte.
    for b in rep.bytes() {
        ihwaddc(b as i32);
    }
    // c:1761 — `hwend();` close the word slot.
    ihwend();
}

/// Port of `char *hgetline(void)` from `Src/hist.c:1769`.
///
/// C body:
/// ```c
/// if (!chline || hptr == chline) return NULL;
/// *hptr = '\0';
/// ret = dupstring(chline);
/// hptr = chline;
/// chwordpos = 0;
/// return ret;
/// ```
///
/// "Get the entire current line, deleting it in the history."
/// Used by `pushlineoredit()` (zle_hist.c:856) to grab the
/// in-flight line for ZLE editing without committing it to
/// the history ring.
///
/// The previous Rust port took a `&histent` and returned its
/// name — fundamentally different from the global chline
/// truncation C performs. Operate on the canonical globals
/// (`chline`, `hptr`, `chwordpos`) and return `Option<String>`
/// (None for the C NULL case at c:1777).
///
/// No Rust callers used the old (entry) signature.
pub fn hgetline() -> Option<String> {                                         // c:1769
    let hp = hptr.load(Ordering::SeqCst);
    let line = chline.lock().unwrap();
    // c:1777 — `if (!chline || hptr == chline) return NULL;`
    if line.is_empty() || hp == 0 {
        return None;
    }
    // c:1779 — `*hptr = '\0';` truncate at hptr. In Rust, slice
    // the substring [0..hp].
    let truncated = if hp <= line.len() {
        line[..hp].to_string()
    } else {
        line.clone()
    };
    drop(line);
    // c:1780 — `ret = dupstring(chline);` (already a copy via
    // Rust .to_string()).
    // c:1783-1784 — reset line: hptr = 0, chwordpos = 0.
    hptr.store(0, Ordering::SeqCst);
    chwordpos.store(0, Ordering::SeqCst);
    Some(truncated)                                                            // c:1786
}

/// Port of `int getargspec(int argc, int marg, int evset)` from Src/hist.c:1793.
/// Reads a word-designator off the input stream via `ingetc`, returning
/// the resolved word index, `-1` for "no designator present" (caller
/// must default), or `-2` for hard error.
/// ```c
/// static int
/// getargspec(int argc, int marg, int evset)
/// {
///     int c, ret = -1;
///     if ((c = ingetc()) == '0') return 0;
///     if (idigit(c)) {
///         ret = 0;
///         while (idigit(c)) {
///             ret = ret * 10 + c - '0';
///             if (ret < 0) { herrflush(); zerr("no such word in event"); return -2; }
///             c = ingetc();
///         }
///         inungetc(c);
///     } else if (c == '^') ret = 1;
///     else if (c == '$') ret = argc;
///     else if (c == '%') {
///         if (evset) { herrflush(); zerr("Ambiguous history reference"); return -2; }
///         if (marg == -1) { herrflush(); zerr("%% with no previous word matched"); return -2; }
///         ret = marg;
///     } else inungetc(c);
///     return ret;
/// }
/// ```
pub fn getargspec(argc: i32, marg_arg: i32, evset: i32) -> i32 {             // c:1793
    let mut c: i32 = crate::ported::input::ingetc()                          // c:1797 ingetc()
        .map(|ch| ch as i32)
        .unwrap_or(-1);
    let mut ret: i32 = -1;                                                   // c:1795
    if c == b'0' as i32 {                                                    // c:1797
        return 0;                                                            // c:1798
    }
    if (c as u8 as char).is_ascii_digit() {                                  // c:1799 idigit(c)
        ret = 0;                                                             // c:1800
        while (c as u8 as char).is_ascii_digit() {                           // c:1801
            ret = ret * 10 + c - b'0' as i32;                                // c:1802
            if ret < 0 {                                                     // c:1803
                herrflush();                                                 // c:1804
                crate::ported::utils::zerr("no such word in event");         // c:1805
                return -2;                                                   // c:1806
            }
            c = crate::ported::input::ingetc()                               // c:1808 ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
        }
        if let Some(ch) = char::from_u32(c as u32) {                         // c:1810 inungetc(c)
            crate::ported::input::inungetc(ch);
        }
    } else if c == b'^' as i32 {                                             // c:1811
        ret = 1;                                                             // c:1812
    } else if c == b'$' as i32 {                                             // c:1813
        ret = argc;                                                          // c:1814
    } else if c == b'%' as i32 {                                             // c:1815
        if evset != 0 {                                                      // c:1816
            herrflush();                                                     // c:1817
            crate::ported::utils::zerr("Ambiguous history reference");       // c:1818
            return -2;                                                       // c:1819
        }
        if marg_arg == -1 {                                                  // c:1821
            herrflush();                                                     // c:1822
            crate::ported::utils::zerr("%% with no previous word matched");  // c:1823
            return -2;                                                       // c:1824
        }
        ret = marg_arg;                                                      // c:1826
    } else {                                                                 // c:1827
        if let Some(ch) = char::from_u32(c as u32) {                         // c:1828 inungetc(c)
            crate::ported::input::inungetc(ch);
        }
    }
    ret                                                                      // c:1829
}

/// Port of `static zlong hconsearch(char *str, int *marg)` from
/// `Src/hist.c:1834-1854`.
///
/// C body (c:1836-1853):
/// ```c
/// for (he = up_histent(hist_ring); he; he = up_histent(he)) {
///     if (he->node.flags & HIST_FOREIGN) continue;
///     if ((s = strstr(he->node.nam, str))) {
///         int pos = s - he->node.nam;
///         while (t1 < he->nwords && he->words[2*t1] <= pos)
///             t1++;
///         *marg = t1 - 1;
///         return he->histnum;
///     }
/// }
/// return -1;
/// ```
///
/// Two divergences in the previous Rust port:
///   1. Took an extra `start: Option<i64>` parameter that has no
///      C counterpart — C always starts from `up_histent(hist_ring)`.
///   2. Dropped the `*marg` output — caller code at c:686 needs
///      the matching word index to update `marg`/`hsubl` state.
///      Without it, the caller hardcoded `margbox=0` losing the
///      actual word position.
///
/// Rust adaptation: returns `Option<(histnum, marg)>` instead of
/// the C in/out pair; None mirrors the C `return -1` miss path.
/// WARNING: Rust returns a tuple — C uses an out-pointer for marg.
pub fn hconsearch(needle: &str) -> Option<(i64, i32)> {                      // c:1836
    // c:1842 — `for (he = up_histent(hist_ring); he; he = up_histent(he))`.
    // The C `hist_ring` is the doubly-linked-list sentinel; iterating
    // `up_histent(hist_ring)` walks from the newest real entry toward
    // older ones. Rust storage is a Vec with newest at position 0;
    // walk positions 0..ring_len for the same effect.
    let ring = hist_ring.lock().expect("hist_ring poisoned");
    for entry in ring.iter() {
        if (entry.node.flags as u32 & HIST_FOREIGN) != 0 {                   // c:1843
            continue;                                                        // c:1844
        }
        if let Some(pos) = entry.node.nam.find(needle) {                     // c:1845 strstr
            // c:1846 — `int pos = s - he->node.nam;`
            let mut t1: i32 = 0;                                             // c:1838
            while t1 < entry.nwords {                                         // c:1847
                let slot_pos = entry.words.get((2 * t1) as usize)
                    .copied()
                    .unwrap_or(0) as usize;
                if slot_pos > pos {                                           // c:1847 he->words[2*t1] <= pos
                    break;
                }
                t1 += 1;                                                      // c:1848
            }
            return Some((entry.histnum, t1 - 1));                             // c:1849-1850
        }
    }
    None                                                                      // c:1853
}

/// Port of `int hcomsearch(char *str)` from Src/hist.c.
pub fn hcomsearch(prefix: &str) -> Option<i64> {
    let mut cur = curhist.load(Ordering::SeqCst);
    while let Some(prev) = up_histent(cur) {
        cur = prev;
        if let Some(entry) = ring_get(cur) {
            if (entry.node.flags as u32 & HIST_FOREIGN) != 0 {
                continue;
            }
            if entry.node.nam.starts_with(prefix) {
                return Some(cur);
            }
        }
    }
    None
}

/// Port of `char *chabspath(char **pathptr)` from Src/hist.c.
pub fn chabspath(input: &str) -> Option<String> {
    if input.is_empty() { return Some(String::new()); }
    let mut path = if !input.starts_with('/') {
        let cwd = std::env::current_dir().ok()?;
        let cwd_s = cwd.to_string_lossy().into_owned();
        if cwd_s.ends_with('/') { format!("{}{}", cwd_s, input) }
        else { format!("{}/{}", cwd_s, input) }
    } else {
        input.to_string()
    };
    let chars: Vec<char> = path.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' {
            out.push('/');
            i += 1;
            while i < chars.len() && chars[i] == '/' { i += 1; }
        } else if c == '.' && i + 1 < chars.len() && chars[i + 1] == '.'
            && (i + 2 == chars.len() || chars[i + 2] == '/')
        {
            if out.len() <= 1 {
                if out.is_empty() || out == ['/'] { return None; }
                out.push('.'); out.push('.');
            } else if out.len() >= 3 && &out[out.len() - 3..] == &['.', '.', '/'] {
                out.push('.'); out.push('.');
            } else {
                if out.last() == Some(&'/') && out.len() > 1 { out.pop(); }
                while out.last().map(|c| *c != '/').unwrap_or(false) { out.pop(); }
            }
            i += 2;
            if i < chars.len() && chars[i] == '/' { i += 1; }
        } else if c == '.' && (i + 1 == chars.len() || chars[i + 1] == '/') {
            i += 1;
            while i < chars.len() && chars[i] == '/' { i += 1; }
        } else {
            out.push(c); i += 1;
        }
    }
    while out.len() > 1 && out.last() == Some(&'/') { out.pop(); }
    path = out.into_iter().collect();
    if path.is_empty() { Some("/".to_string()) } else { Some(path) }
}

/// Port of `char *chrealpath(char **pathptr)` from Src/hist.c.
pub fn chrealpath(path: &str) -> Option<String> {
    std::fs::canonicalize(path).ok().map(|p| p.to_string_lossy().into_owned())
}

/// Port of `char *remtpath(char **str, int count)` from Src/hist.c:2056.
pub fn remtpath(s: &str, count: i32) -> String {                             // c:2056
    let s = s.trim_end_matches('/');
    if s.is_empty() { return "/".to_string(); }
    if count == 0 {
        if let Some(pos) = s.rfind('/') {
            if pos == 0 { return "/".to_string(); }
            return s[..pos].trim_end_matches('/').to_string();
        }
        return ".".to_string();
    }
    let bytes = s.as_bytes();
    let mut remaining = count;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            remaining -= 1;
            if remaining <= 0 {
                if i == 0 { return "/".to_string(); }
                return s[..i].to_string();
            }
            while i + 1 < bytes.len() && bytes[i + 1] == b'/' { i += 1; }
        }
        i += 1;
    }
    s.to_string()
}

/// Port of `char *remtext(char **str)` from Src/hist.c:2122.
pub fn remtext(s: &str) -> String {                                          // c:2122
    if let Some(slash_pos) = s.rfind('/') {
        let after_slash = &s[slash_pos + 1..];
        if let Some(dot_pos) = after_slash.rfind('.') {
            if dot_pos > 0 {
                return format!("{}/{}", &s[..slash_pos], &after_slash[..dot_pos]);
            }
        }
        return s.to_string();
    }
    if let Some(dot_pos) = s.rfind('.') {
        if dot_pos > 0 { return s[..dot_pos].to_string(); }
    }
    s.to_string()
}

/// Port of `char *rembutext(char **str)` from Src/hist.c:2136.
pub fn rembutext(s: &str) -> String {                                        // c:2136
    if let Some(slash_pos) = s.rfind('/') {
        let after_slash = &s[slash_pos + 1..];
        if let Some(dot_pos) = after_slash.rfind('.') {
            return after_slash[dot_pos + 1..].to_string();
        }
        return String::new();
    }
    if let Some(dot_pos) = s.rfind('.') {
        return s[dot_pos + 1..].to_string();
    }
    String::new()
}

/// Port of `char *remlpaths(char **str, int count)` from Src/hist.c:2152.
/// Rust idiom replacement: `split('/')` + `iter().rev().take(n)`
/// covers the C reverse-scan-and-skip-leading-paths loop without
/// strchr/strncpy bookkeeping.
pub fn remlpaths(s: &str, count: i32) -> String {                            // c:2152
    let s = s.trim_end_matches('/');
    if s.is_empty() { return String::new(); }
    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    let n = if count == 0 { 1 } else { count as usize };
    let take_n = n.min(parts.len());
    if take_n == 0 { return String::new(); }
    parts.iter().rev().take(take_n).rev().copied().collect::<Vec<&str>>().join("/")
}

/// Port of `char *casemodify(char *str, int how)` from Src/hist.c:2196.
/// Rust idiom replacement: `chars()` + `to_lowercase`/`to_uppercase`
/// covers the C `tolower`/`toupper`/`isalpha` per-byte loop; the
/// CASMOD_CAPS branch tracks word-boundary via the `nextupper` flag.
pub fn casemodify(s: &str, how: i32) -> String {                              // c:2196
    let mut result = String::with_capacity(s.len());
    let mut nextupper = true;
    for c in s.chars() {
        let modified = match how {
            x if x == CASMOD_LOWER => c.to_lowercase().collect::<String>(),
            x if x == CASMOD_UPPER => c.to_uppercase().collect::<String>(),
            x if x == CASMOD_CAPS => {
                if !c.is_alphanumeric() {
                    nextupper = true;
                    c.to_string()
                } else if nextupper {
                    nextupper = false;
                    c.to_uppercase().collect::<String>()
                } else {
                    c.to_lowercase().collect::<String>()
                }
            }
            _ /* CASMOD_NONE */ => c.to_string(),
        };
        let _ = CASMOD_NONE; // silence unused
        result.push_str(&modified);
    }
    result
}

/// Port of `char *subst(...)` from Src/hist.c:2336.
pub fn subst(s: &str, in_pattern: &str, out_pattern: &str, global: bool) -> String {
    if in_pattern.is_empty() { return s.to_string(); }
    let mut anchor_start = false;
    let mut anchor_end = false;
    let mut pat = in_pattern;
    if let Some(rest) = pat.strip_prefix('#') { anchor_start = true; pat = rest; }
    if let Some(rest) = pat.strip_prefix('%') { anchor_end = true; pat = rest; }
    if pat.is_empty() { return s.to_string(); }
    let out_expanded = convamps(out_pattern, pat);
    if anchor_start && anchor_end {
        if s == pat { return out_expanded; }
        return s.to_string();
    }
    if anchor_start {
        if let Some(rest) = s.strip_prefix(pat) {
            return format!("{}{}", out_expanded, rest);
        }
        return s.to_string();
    }
    if anchor_end {
        if s.ends_with(pat) {
            let prefix_len = s.len() - pat.len();
            return format!("{}{}", &s[..prefix_len], out_expanded);
        }
        return s.to_string();
    }
    if global { s.replace(pat, &out_expanded) } else { s.replacen(pat, &out_expanded, 1) }
}

/// Port of `char *convamps(char *out, char *in)` from Src/hist.c.
fn convamps(out: &str, in_pattern: &str) -> String {
    let mut result = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() { result.push(next); chars.next(); }
        } else if c == '&' {
            result.push_str(in_pattern);
        } else {
            result.push(c);
        }
    }
    result
}

/// Port of `static void checkcurline(Histent he)` from
/// `Src/hist.c:2421`.
///
/// C body (c:2421-2429):
/// ```c
/// static void checkcurline(Histent he)
/// {
///     if (he->histnum == curhist && (histactive & HA_ACTIVE)) {
///         curline.node.nam = chline;
///         curline.nwords = chwordpos/2;
///         curline.words = chwords;
///     }
/// }
/// ```
///
/// The previous Rust port was a complete fabrication: signature was
/// `(line: &str) -> i32` returning whether the head of the ring's
/// name matched the argument — none of which appears in the C body.
/// No caller existed because the bogus signature fit nothing.
///
/// Real C behaviour: when `he` is the current in-flight history
/// entry (matches `curhist` AND the history machinery is active),
/// flush the in-progress chline/chwordpos/chwords build state into
/// the `curline` placeholder so the caller sees the latest words.
/// Called from `movehistent` per c:1298 before returning.
///
/// WARNING: Rust adaptation diverges in storage shape — C uses
/// pointer-aliasing (curline.node.nam = chline) whereas Rust
/// stores cloned values into the `curline` Mutex. The observable
/// effect is the same after the function returns: a caller reading
/// `curline` sees the latest chline/chwords snapshot.
pub fn checkcurline(he: &histent) {                                              // c:2421
    let curhist_val = curhist.load(Ordering::SeqCst);                            // c:2424
    let active = histactive.load(Ordering::SeqCst);                              // c:2424
    if he.histnum == curhist_val && (active & HA_ACTIVE) != 0 {                  // c:2424
        let chline_val = chline.lock().expect("chline poisoned").clone();        // c:2425
        let chwordpos_val = chwordpos.load(Ordering::SeqCst);                    // c:2426
        let chwords_val = chwords.lock().expect("chwords poisoned").clone();     // c:2427
        let mut cl = curline.lock().expect("curline poisoned");
        // Build a fresh histent snapshot mirroring the C field
        // aliasing — name = chline, nwords = chwordpos/2,
        // words = chwords.
        *cl = Some(histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: chline_val,                                                 // c:2425
                flags: 0,
            },
            up: None,
            down: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: chwords_val,                                                  // c:2427
            nwords: chwordpos_val / 2,                                           // c:2426
            histnum: he.histnum,
        });
    }
}

/// Port of `Histent quietgethist(zlong ev)` from Src/hist.c.
pub fn quietgethist(ev: i64) -> Option<histent> {                            // c:2433
    ring_get(ev)
}

/// Port of `Histent gethist(zlong ev)` from Src/hist.c.
pub fn gethist(ev: i64) -> Option<histent> {                                 // c:2440
    let ret = quietgethist(ev);
    if ret.is_none() {
        herrflush();
        crate::ported::utils::zerr(&format!("no such event: {}", ev));
    }
    ret
}

/// Port of `static char *getargs(Histent elist, int arg1, int arg2)`
/// from `Src/hist.c:2454-2483`.
///
/// C body (c:2456-2482):
/// ```c
/// short *words = elist->words;
/// int pos1, pos2, nwords = elist->nwords;
///
/// if (arg2 < arg1 || arg1 >= nwords || arg2 >= nwords) {
///     herrflush();
///     zerr("no such word in event");
///     return NULL;
/// }
/// if (arg1 == 0 && arg2 == nwords - 1)
///     return dupstring(elist->node.nam);
///
/// pos1 = words[2*arg1];
/// pos2 = words[2*arg2+1];
///
/// /* a word has to be at least one character long, so if the position
///  * of a word is less than its index, we've overflowed our signed
///  * short integer word range and the recorded position is garbage. */
/// if (pos1 < 0 || pos1 < arg1 || pos2 < 0 || pos2 < arg2) {
///     herrflush();
///     zerr("history event too long, can't index requested words");
///     return NULL;
/// }
/// return dupstrpfx(elist->node.nam + pos1, pos2 - pos1);
/// ```
///
/// Three divergences in the previous Rust port:
///   1. Used `entry.words.len() / 2` for nwords — C uses the explicit
///      `elist->nwords` field. The two MAY agree but the field is
///      the authoritative source per the storage shape.
///   2. Had an extra `nwords == 0` check that C lacks; the existing
///      `arg1 >= nwords` check already covers the empty case
///      (arg1: usize >= 0 vs nwords=0 returns true).
///   3. Overflow check was wrong: `pos2 > nam.len() || pos1 > pos2`
///      instead of the C `pos1 < 0 || pos1 < arg1 || pos2 < 0 ||
///      pos2 < arg2`. The C check detects signed-short overflow —
///      since each word must be ≥1 char long, the start-byte index
///      must be ≥ the 0-based word index; a negative or too-small
///      stored position signals i16 overflow on history lines >32KB.
///      The Rust port's bounds check missed the overflow case
///      and let bogus garbage positions through into the slice
///      indexing.
pub fn getargs(entry: &histent, arg1: usize, arg2: usize) -> Option<String> {    // c:2454
    let nwords = entry.nwords as usize;                                          // c:2457 nwords = elist->nwords
    if arg2 < arg1 || arg1 >= nwords || arg2 >= nwords {                         // c:2459
        herrflush();                                                             // c:2461
        crate::ported::utils::zerr("no such word in event");                     // c:2462
        return None;                                                             // c:2463
    }
    // c:2466-2467 — `if (arg1 == 0 && arg2 == nwords - 1) return dupstring(nam);`
    if arg1 == 0 && arg2 == nwords - 1 {
        return Some(entry.node.nam.clone());                                     // c:2467
    }
    let pos1_raw = entry.words.get(2 * arg1).copied().unwrap_or(-1);             // c:2469 pos1 = words[2*arg1]
    let pos2_raw = entry.words.get(2 * arg2 + 1).copied().unwrap_or(-1);         // c:2470 pos2 = words[2*arg2+1]
    // c:2476 — C signed-short overflow detection: any negative
    // position OR a position less than its corresponding word
    // index means the i16 storage wrapped on a >32KB history line.
    if pos1_raw < 0
        || (pos1_raw as i64) < (arg1 as i64)
        || pos2_raw < 0
        || (pos2_raw as i64) < (arg2 as i64)
    {                                                                            // c:2476
        herrflush();                                                             // c:2477
        crate::ported::utils::zerr(
            "history event too long, can't index requested words"               // c:2478
        );
        return None;                                                             // c:2479
    }
    let pos1 = pos1_raw as usize;
    let pos2 = pos2_raw as usize;
    // c:2481 — `dupstrpfx(elist->node.nam + pos1, pos2 - pos1)`.
    // Rust slice indexing requires pos1 <= pos2 <= len; both are
    // satisfied since both passed the c:2476 overflow check and
    // are bounded by the underlying string length per insert
    // contract. Guard with .get() so a malformed entry doesn't
    // panic.
    entry.node.nam.get(pos1..pos2).map(|s| s.to_string())                        // c:2481
}

/// Port of `int quote(char **tr)` from `Src/hist.c:2486-2523`.
/// Wraps `*tr` in single quotes; `'` inside becomes `'\''` and any
/// `inblank(c)` (space/tab/newline) outside quotes that wasn't
/// preceded by `\` becomes `'<c>'`.
///
/// Previous Rust port used `c.is_whitespace()` — broader than C's
/// `inblank()` which is the narrow typtab INBLANK class (space, tab,
/// newline ONLY). Drift would silently quote CR/FF/VT/NBSP/etc.
/// chars that C leaves alone.
pub fn quote(s: &str) -> String {                                            // c:2486
    let bytes: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(bytes.len() + 3);
    out.push('\'');
    let mut inquotes = false;
    let mut prev: char = '\0';
    for &c in bytes.iter() {
        // c:2499 — `inblank(*ptr)` is narrow space/tab/newline.
        let is_inblank = matches!(c, ' ' | '\t' | '\n');
        if c == '\'' {
            inquotes = !inquotes;
            out.push('\''); out.push('\\'); out.push('\''); out.push('\'');
        } else if is_inblank && !inquotes && prev != '\\' {                  // c:2514
            out.push('\''); out.push(c); out.push('\'');
        } else {
            out.push(c);
        }
        prev = c;
    }
    out.push('\'');
    out
}

/// Port of `int quotebreak(char **tr)` from `Src/hist.c:2527`. Same
/// shape as `quote` but `inblank` chars get the `'<c>'` break-out
/// treatment whether or not they're inside quotes.
///
/// `c.is_whitespace()` → `inblank(c)` fix per the same divergence
/// pattern as `quote` above.
pub fn quotebreak(s: &str) -> String {                                       // c:2527
    let mut result = String::with_capacity(s.len() + 10);
    result.push('\'');
    for c in s.chars() {
        // c:2548 — `inblank(*ptr)` narrow set.
        let is_inblank = matches!(c, ' ' | '\t' | '\n');
        if c == '\'' { result.push_str("'\\''"); }
        else if is_inblank {
            result.push('\''); result.push(c); result.push('\'');
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// Port of `char *hdynread(int stop)` from `Src/hist.c:2562`. C body is
/// inside `#if 0` (the in-tree `hdynread2` variant is the live one), but
/// the name-parity port mirrors the disabled body: read input chars via
/// `ingetc()` until `stop`/newline/lexstop, honoring `\\`-escape, and
/// emit a `delimiter expected` error if newline is hit first.
pub fn hdynread(stop: i32) -> Option<String> {                               // c:2562
    use std::sync::atomic::Ordering::SeqCst;
    let stop_c = stop as u8 as char;                                         // c:2562 int stop
    let mut buf = String::with_capacity(256);                                // c:2564 bsiz=256
    let mut c: Option<char>;                                                 // c:2564 int c
    loop {
        c = crate::ported::input::ingetc();                                  // c:2568
        match c {
            None => break,
            Some(ch) if ch == stop_c => break,                               // c:2568
            Some('\n') => break,                                             // c:2568
            Some(ch) => {
                if crate::ported::hist::lexstop.load(SeqCst) { break; }      // c:2568
                let mut written = ch;
                if ch == '\\' {                                              // c:2569
                    if let Some(nxt) = crate::ported::input::ingetc() {      // c:2570
                        written = nxt;
                    } else {
                        break;
                    }
                }
                buf.push(written);                                           // c:2571
            }
        }
    }
    if let Some('\n') = c {                                                  // c:2578
        crate::ported::input::inungetc('\n');                                // c:2579
        crate::ported::utils::zerr("delimiter expected");                    // c:2580
        return None;                                                         // c:2582
    }
    Some(buf)                                                                // c:2584
}

/// Direct port of `static void ihungetc(int c)` from `Src/hist.c:989`.
/// Push back `c` into the lexer input stream while history-rewriting
/// is in progress: also rewinds chline (`hptr--`), undoes the
/// `expanding`-driven `zlemetacs`/`zlemetall` advance, and tracks the
/// `qbang` state for `\!` re-escape on the next pass. Loops while
/// `qbang` keeps firing (which can re-trigger via the `c='\\'` step
/// at the bottom).
pub fn ihungetc(c: i32) {                                                    // c:989
    use crate::ported::zsh_h::{INP_ALIAS, INP_HIST};
    use std::sync::atomic::Ordering::SeqCst;
    let mut c = c as u8 as char;                                             // c:991 int c
    let mut doit = 1;                                                        // c:991 doit = 1
    while !crate::ported::hist::lexstop.load(SeqCst)                         // c:993 while (!lexstop && !errflag)
        && crate::ported::utils::errflag.load(SeqCst) == 0
    {
        let hp = crate::ported::hist::hptr.load(SeqCst);
        let line = crate::ported::hist::chline.lock().unwrap().clone();
        let line_b = line.as_bytes();
        let stop = crate::ported::hist::stophist.load(SeqCst);
        let inflags = crate::ported::input::inbufflags.with(|f| f.get());
        let active = crate::ported::hist::histactive.load(SeqCst);
        if hp >= 2 && hp <= line_b.len()                                     // c:994-997
            && line_b[hp - 1] != c as u8 && stop < 4
            && line_b[hp - 1] == b'\n' && line_b[hp - 2] == b'\\'
            && (active & crate::ported::hist::HA_UNGET) == 0
            && (inflags & (INP_ALIAS | INP_HIST)) != INP_ALIAS
        {
            crate::ported::hist::histactive.fetch_or(crate::ported::hist::HA_UNGET, SeqCst);  // c:998
            crate::ported::input::inungetc('\n');                            // c:999 hungetc('\n') — default = inungetc (c:1140)
            crate::ported::input::inungetc('\\');                            // c:1000
            crate::ported::hist::histactive.fetch_and(!crate::ported::hist::HA_UNGET, SeqCst); // c:1001
        }
        if crate::ported::hist::expanding.load(SeqCst) != 0 {                // c:1004 if (expanding)
            crate::ported::zle::compcore::ZLEMETACS.fetch_sub(1, SeqCst);    // c:1005 zlemetacs--
            crate::ported::zle::compcore::ZLEMETALL.fetch_sub(1, SeqCst);    // c:1006 zlemetall--
            crate::ported::hist::exlast.fetch_add(1, SeqCst);                // c:1007 exlast++
        }
        if (inflags & (INP_ALIAS | INP_HIST)) != INP_ALIAS {                 // c:1009
            // c:1010-1013 — DPUTS asserts; hptr-- + qbang derive.
            let new_hp = hp.saturating_sub(1);
            crate::ported::hist::hptr.store(new_hp, SeqCst);                 // c:1011 hptr--
            let bangchar_v = crate::ported::hist::bangchar.load(SeqCst) as u8;
            let qb = c as u8 == bangchar_v && stop < 2                       // c:1014-1015
                && new_hp > 0 && line_b.get(new_hp - 1).copied() == Some(b'\\');
            crate::ported::hist::qbang.store(qb, SeqCst);
        } else {
            crate::ported::hist::qbang.store(false, SeqCst);                 // c:1018 No active bangs in aliases
        }
        if doit != 0 {                                                        // c:1020
            crate::ported::input::inungetc(c);                               // c:1021
        }
        if !crate::ported::hist::qbang.load(SeqCst) { return; }              // c:1022
        let inflags2 = crate::ported::input::inbufflags.with(|f| f.get());
        doit = if crate::ported::hist::stophist.load(SeqCst) == 0            // c:1023-1024
            && ((inflags2 & INP_HIST) != 0 || (inflags2 & INP_ALIAS) == 0)
        { 1 } else { 0 };
        c = '\\';                                                            // c:1025
    }
}

/// Direct port of `int getsubsargs(char *subline, int *gbalp, int *cflagp)`
/// from `Src/hist.c:518`. Parses the substitution arguments of a
/// `!:s/old/new/`-style history modifier: reads the delimiter via
/// ingetc, slurps `old` and `new` chunks, stores them in `hsubl`/`hsubr`
/// globals, then peeks the trailing `:G` (global) or fall-through char.
/// Returns 0 on success, 1 on a bad-expansion (empty old chunk).
/// WARNING: param names don't match C — Rust=(_subline, gbalp, cflagp) vs C=(subline, gbalp, cflagp)
pub fn getsubsargs(_subline: &str, gbalp: &mut i32, cflagp: &mut i32) -> i32 {  // c:518
    let del = match crate::ported::input::ingetc() {                         // c:524 del = ingetc()
        Some(c) => c, None => return 1,
    };
    // c:525-528 — `ptr1 = hdynread2(del); if (!ptr1) return 1;`
    // Inline hdynread2: read until del or '\n', honoring backslash escapes.
    let read_until = |stop: char| -> Option<String> {                        // c:hdynread2 inline
        let mut out = String::new();
        loop {
            match crate::ported::input::ingetc() {
                None => return None,
                Some('\n') => return Some(out),
                Some(c) if c == stop => return Some(out),
                Some('\\') => {
                    if let Some(n) = crate::ported::input::ingetc() {
                        if n != stop { out.push('\\'); }
                        out.push(n);
                    }
                }
                Some(c) => out.push(c),
            }
        }
    };
    let ptr1 = match read_until(del) { Some(p) => p, None => return 1 };     // c:525
    let ptr2 = read_until(del).unwrap_or_default();                          // c:529
    if !ptr1.is_empty() {                                                    // c:530
        *hsubl.lock().unwrap() = Some(ptr1);                                 // c:531-532 zsfree(hsubl); hsubl = ptr1
    } else if hsubl.lock().unwrap().is_none() {                              // c:533 fail silently
        return 0;                                                            // c:536
    }
    *hsubr.lock().unwrap() = Some(ptr2);                                     // c:539-540 zsfree(hsubr); hsubr = ptr2
    let follow = crate::ported::input::ingetc();                             // c:541 follow = ingetc()
    if follow == Some(':') {                                                 // c:542
        let next = crate::ported::input::ingetc();                           // c:543
        if next == Some('G') { *gbalp = 1; }                                 // c:544-545
        else {
            if let Some(c) = next { crate::ported::input::inungetc(c); }     // c:547 inungetc
            *cflagp = 1;                                                     // c:548
        }
    } else if let Some(c) = follow {
        crate::ported::input::inungetc(c);                                   // c:551 inungetc(follow)
    }
    0                                                                        // c:553
}

/// Port of `char *hdynread2(int stop)` from Src/hist.c.
pub fn hdynread2(stop: char, input: &str) -> (String, usize) {
    let mut out = String::new();
    let mut consumed = 0usize;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        consumed += c.len_utf8();
        if c == stop || c == '\n' {
            if c == '\n' { consumed -= c.len_utf8(); }
            return (out, consumed);
        }
        if c == '\\' {
            if let Some(esc) = chars.next() {
                consumed += esc.len_utf8();
                out.push(esc);
            }
        } else {
            out.push(c);
        }
    }
    (out, consumed)
}

/// Port of `void inithist(void)` from Src/hist.c:2613.
pub fn inithist() {                                                          // c:2613
    histsiz.store(1000, Ordering::SeqCst);
    savehistsiz.store(1000, Ordering::SeqCst);
    curhist.store(0, Ordering::SeqCst);
    histlinect.store(0, Ordering::SeqCst);
}

/// Port of `void resizehistents(void)` from Src/hist.c.
pub fn resizehistents() {
    let cap = histsiz.load(Ordering::SeqCst);
    while histlinect.load(Ordering::SeqCst) > cap {
        if let Some(oldest) = ring_oldest() {
            let mut ring = hist_ring.lock().unwrap();
            ring.retain(|h| h.histnum != oldest);
            histlinect.fetch_sub(1, Ordering::SeqCst);
        } else {
            break;
        }
    }
}

/// Port of `void readhistline(char *line, ...)` from Src/hist.c.
pub fn readhistline(line: &str) -> Option<histent> {
    let line = line.trim();
    if line.is_empty() { return None; }
    if let Some(rest) = line.strip_prefix(": ") {
        if let Some(semi) = rest.find(';') {
            let meta = &rest[..semi];
            let cmd = &rest[semi + 1..];
            let parts: Vec<&str> = meta.splitn(2, ':').collect();
            let timestamp = parts.first().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
            let mut entry = make_histent(0, cmd.to_string());
            entry.stim = timestamp;
            return Some(entry);
        }
    }
    Some(make_histent(0, line.to_string()))
}

/// Port of `void readhistfile(char *fn, int err, int readflags)` from Src/hist.c:2675.
pub fn readhistfile(fn_path: Option<&str>, _err: i32, _readflags: i32) {     // c:2675
    let path: String = match fn_path {
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return,
        },
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return,
    };
    if contents.is_empty() { return; }
    let _ = lockhistfile(Some(&path), 1);

    let mut current: Option<(i64, i64, String)> = None;
    for raw_line in contents.lines() {
        if let Some((stim, ftim, ref mut text)) = current {
            if text.ends_with('\\') {
                text.pop();
                text.push('\n');
                text.push_str(raw_line);
                current = Some((stim, ftim, text.clone()));
                continue;
            }
            let n = curhist.fetch_add(1, Ordering::SeqCst) + 1;
            let mut entry = make_histent(n, text.clone());
            entry.stim = stim;
            entry.ftim = ftim;
            entry.node.flags |= HIST_OLD as i32;
            hist_ring.lock().unwrap().insert(0, entry);
            histlinect.fetch_add(1, Ordering::SeqCst);
            current = None;
        }
        if let Some(rest) = raw_line.strip_prefix(": ") {
            if let Some((meta, text)) = rest.split_once(';') {
                if let Some((stim_s, dur_s)) = meta.split_once(':') {
                    let stim: i64 = stim_s.parse().unwrap_or(0);
                    let dur: i64 = dur_s.parse().unwrap_or(0);
                    let ftim = stim + dur;
                    current = Some((stim, ftim, text.to_string()));
                    continue;
                }
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        current = Some((now, now, raw_line.to_string()));
    }
    if let Some((stim, ftim, text)) = current {
        let n = curhist.fetch_add(1, Ordering::SeqCst) + 1;
        let mut entry = make_histent(n, text);
        entry.stim = stim;
        entry.ftim = ftim;
        entry.node.flags |= HIST_OLD as i32;
        hist_ring.lock().unwrap().insert(0, entry);
        histlinect.fetch_add(1, Ordering::SeqCst);
    }
    unlockhistfile(&path);
    resizehistents();
}

/// Port of `int flockhistfile(char *fn)` from Src/hist.c.
pub fn flockhistfile(path: &str) -> i32 {
    #[cfg(unix)]
    {
        if let Ok(file) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(format!("{}.lock", path))
        {
            let fd = file.as_raw_fd();
            return unsafe { if libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) == 0 { 1 } else { 0 } };
        }
        0
    }
    #[cfg(not(unix))]
    { let _ = path; 1 }
}

/// Port of `void savehistfile(char *fn, int err, int writeflags)` from Src/hist.c:2922.
/// Rust idiom replacement: `fs::write` + `resolve_histfile` covers
/// the C `fopen`+`fwrite`+`fclose` ladder with the `err` arg folded
/// into the Result-bubbling; HFILE_APPEND/HFILE_USE_OPTIONS flag
/// handling lives on the caller's writeflags decision.
pub fn savehistfile(fn_path: Option<&str>, _writeflags: i32) {               // c:2922
    // c:2931-2934 — `if (!interact || savehistsiz <= 0 || !hist_ring
    //                || (!fn && !(fn = getsparam("HISTFILE")))) return;`
    //
    // Two early-return gates the previous Rust port missed:
    //   1. `!interact` — non-interactive shells must not write
    //      history. A script that has accumulated commands
    //      shouldn't pollute the interactive user's HISTFILE.
    //   2. `savehistsiz <= 0` — when SAVEHIST=0 (or negative),
    //      history saving is explicitly disabled. The previous
    //      port wrote an EMPTY file (cap=0 → no entries),
    //      truncating the user's existing history.
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE)        // c:2932 !interact
    {
        return;
    }
    let cap = savehistsiz.load(Ordering::SeqCst);                             // c:2932 savehistsiz
    if cap <= 0 {                                                             // c:2932 savehistsiz <= 0
        return;
    }
    let path: String = match fn_path {                                        // c:2933 fn / HISTFILE
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return,
        },
    };
    let _ = lockhistfile(Some(&path), 1);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true).open(&path)
    {
        let cap = cap as usize;
        let ring = hist_ring.lock().unwrap();
        let mut count = 0;
        for entry in ring.iter().rev() {
            if count >= cap { break; }
            let dur = entry.ftim.saturating_sub(entry.stim);
            let _ = writeln!(file, ": {}:{};{}", entry.stim, dur, entry.node.nam);
            count += 1;
        }
    }
    unlockhistfile(&path);
}

/// Port of `int lockhistct` from Src/hist.c. Re-entrant lock counter.
static lockhistct: AtomicI32 = AtomicI32::new(0);

/// Port of `int checklocktime(char *fn, time_t mtim)` from Src/hist.c.
pub fn checklocktime(path: &str, max_age_secs: u64) -> i32 {
    let lockfile = format!("{}.lock", path);
    if let Ok(meta) = std::fs::metadata(&lockfile) {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = modified.elapsed() {
                if age.as_secs() < max_age_secs { return 1; }
            }
        }
    }
    0
}

/// Port of `int lockhistfile(char *fn, int keep_trying)` from Src/hist.c:3182.
/// Rust idiom replacement: `fs2::FileExt::try_lock_exclusive` covers
/// the C `flock` + `link`-symlink retry loop; the `keep_trying`
/// arg controls retry budget rather than mode flags.
pub fn lockhistfile(fn_path: Option<&str>, keep_trying: i32) -> i32 {        // c:3182
    let path: String = match fn_path {                                       // c:3182
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return 1,                                                // c:3189
        },
    };
    if lockhistct.fetch_add(1, Ordering::SeqCst) > 0 {
        return 0;
    }
    let max_tries = if keep_trying != 0 { 30 } else { 1 };
    for attempt in 0..max_tries {
        if flockhistfile(&path) != 0 {
            return 0;
        }
        if attempt + 1 < max_tries {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    lockhistct.fetch_sub(1, Ordering::SeqCst);
    if keep_trying != 0 { 2 } else { 1 }
}

/// Port of `void unlockhistfile(char *fn)` from Src/hist.c.
pub fn unlockhistfile(path: &str) {
    let prev = lockhistct.fetch_sub(1, Ordering::SeqCst);
    if prev <= 0 {
        lockhistct.store(0, Ordering::SeqCst);
        return;
    }
    if prev == 1 {
        let lockpath = format!("{}.lock", path);
        let _ = std::fs::remove_file(&lockpath);
    }
}

/// Port of `int histfileIsLocked(void)` from Src/hist.c.
#[allow(non_snake_case)]
pub fn histfileIsLocked() -> i32 {
    if lockhistct.load(Ordering::SeqCst) > 0 { 1 } else { 0 }
}

/// Port of `int bufferwords(LinkList list, char *buf, int *index, int flags)` from Src/hist.c.
/// Rust idiom replacement: char-by-char tokenizer covers the C
/// shparser callout (`(z)` flag at subst.c:4186 always passes
/// `NULL, 0`). The returned `(words, cursor_word_idx)` pair lets
/// `${(z)var}` callers (which want just `words`) take `.0` while
/// `bufferwords` callers that need the cursor index get `.1`.
pub fn bufferwords(line: &str, cursor_pos: usize) -> (Vec<String>, usize) {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let flush = |out: &mut Vec<String>, cur: &mut String| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' => {
                flush(&mut words, &mut cur);
                i += 1;
            }
            ';' | '&' | '|' | '<' | '>' | '(' | ')' => {
                flush(&mut words, &mut cur);
                let mut tok = String::new();
                tok.push(c);
                while i + 1 < chars.len()
                    && chars[i + 1] == c
                    && matches!(c, '&' | '|' | ';' | '<' | '>')
                {
                    tok.push(c);
                    i += 1;
                }
                words.push(tok);
                i += 1;
            }
            '\'' => {
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        cur.push(chars[i]);
                        i += 1;
                        continue;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            }
            '\\' if i + 1 < chars.len() => {
                cur.push(chars[i + 1]);
                i += 2;
            }
            _ => {
                cur.push(c);
                i += 1;
            }
        }
    }
    flush(&mut words, &mut cur);
    // Find which word index the cursor is in (best-effort).
    let mut pos = 0;
    let mut word_idx = 0;
    for (i, word) in line.split_whitespace().enumerate() {
        if let Some(start) = line[pos..].find(word) {
            let wstart = pos + start;
            let wend = wstart + word.len();
            if cursor_pos >= wstart && cursor_pos <= wend {
                word_idx = i;
                break;
            }
            pos = wend;
        }
    }
    (words, word_idx)
}

/// Port of `int histsplitwords(char *line, ...)` from Src/hist.c.
/// Rust idiom replacement: in-place char-walk with quote-state
/// tracking covers the C `hgetword`+`splitword` chain; returns
/// (start, end) byte-offset pairs (vs the C LinkList of word
/// pointers into the original string).
pub fn histsplitwords(line: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut in_word = false;
    let mut word_start = 0;
    let mut in_quote = false;
    let mut quote_char = '\0';
    for (i, c) in line.char_indices() {
        if in_quote {
            if c == quote_char { in_quote = false; }
            continue;
        }
        if c == '\'' || c == '"' {
            in_quote = true;
            quote_char = c;
            if !in_word { word_start = i; in_word = true; }
            continue;
        }
        if c.is_ascii_whitespace() {
            if in_word { words.push((word_start, i)); in_word = false; }
        } else if !in_word {
            word_start = i; in_word = true;
        }
    }
    if in_word { words.push((word_start, line.len())); }
    words
}

/// Port of `int pushhiststack(char *hf, zlong hs, zlong shs, int level)` from Src/hist.c:3845.
pub fn pushhiststack(hf: Option<&str>, hs: i64, shs: i64, level: i32) {      // c:3845
    let snap = histsave {                                                    // c:3870
        lasthist: histfile_stats {
            text: None, stim: 0, mtim: 0, fpos: 0, fsiz: 0,
            interrupted: 0, next_write_ev: 0,
        },
        histfile: hf.map(|s| s.to_string()),                                 // c:3872 h->histfile = histfile
        hist_ring: std::mem::take(&mut *hist_ring.lock().unwrap()),          // c:3874 h->hist_ring = hist_ring
        curhist: curhist.load(Ordering::SeqCst),                             // c:3875 h->curhist = curhist
        histlinect: histlinect.load(Ordering::SeqCst),                       // c:3876
        histsiz: histsiz.load(Ordering::SeqCst),                             // c:3877
        savehistsiz: savehistsiz.load(Ordering::SeqCst),                     // c:3878
        locallevel: level,                                                   // c:3879
    };
    histsave_stack.lock().unwrap().push(snap);                               // c:3901
    histsave_stack_size.fetch_add(1, Ordering::SeqCst);
    histsave_stack_pos.fetch_add(1, Ordering::SeqCst);
    histsiz.store(hs, Ordering::SeqCst);                                     // c:3901
    savehistsiz.store(shs, Ordering::SeqCst);                                // c:3901
    curhist.store(0, Ordering::SeqCst);                                      // c:3901 curhist = histlinect = 0
    histlinect.store(0, Ordering::SeqCst);
    let _ = hf;
}

/// Port of `int pophiststack(void)` from `Src/hist.c:3901`.
///
/// C body:
/// ```c
/// if (histsave_stack_pos == 0) return 0;
/// if (curline_in_ring) unlinkcurline();
/// deletehashtable(histtab); zsfree(lasthist.text);
/// h = &histsave_stack[--histsave_stack_pos];
/// lasthist = h->lasthist;
/// if (h->histfile) {
///     if (*h->histfile) setsparam("HISTFILE", h->histfile);
///     else unsetparam("HISTFILE");
/// }
/// histtab = h->histtab;
/// hist_ring = h->hist_ring;
/// curhist = h->curhist;
/// if (zleactive) zleentry(ZLE_CMD_SET_HIST_LINE, curhist);
/// histlinect = h->histlinect;
/// histsiz = h->histsiz;
/// savehistsiz = h->savehistsiz;
/// if (curline_in_ring) linkcurline();
/// return histsave_stack_pos + 1;
/// ```
///
/// The previous Rust port skipped:
///   - The `histfile` paramtab restore (c:3920-3924) — HISTFILE
///     wasn't reverted when popping back out of a pushed stack
///     frame, so the user's HISTFILE could end up pointing at
///     the wrong file after `fc -p` / `fc -P`.
///   - Returning 0 on empty stack (c:3907) — the previous
///     port returned `()` and didn't signal "nothing to pop".
///
/// Return: 0 when nothing was popped, else `histsave_stack_pos + 1`
/// (the depth that WAS popped).
pub fn pophiststack() -> i32 {                                                // c:3901
    let snap = match histsave_stack.lock().unwrap().pop() {
        Some(s) => s,
        None => return 0,                                                     // c:3907
    };
    // c:3920-3924 — restore HISTFILE via setsparam / unsetparam.
    // Was previously `let _ = snap.histfile;` (dropped on the
    // floor). With this in place, `fc -p file ...; fc -P`
    // properly restores the outer HISTFILE value.
    if let Some(ref hf) = snap.histfile {
        if !hf.is_empty() {                                                   // c:3922 *h->histfile
            crate::ported::params::setsparam("HISTFILE", hf);                 // c:3922
        } else {                                                              // c:3923
            // Unset HISTFILE — Rust paramtab remove.
            let _ = crate::ported::params::paramtab()
                .write()
                .unwrap()
                .remove("HISTFILE");                                          // c:3923 unsetparam
        }
    }
    *hist_ring.lock().unwrap() = snap.hist_ring;                              // c:3925
    curhist.store(snap.curhist, Ordering::SeqCst);                            // c:3926
    histlinect.store(snap.histlinect, Ordering::SeqCst);                      // c:3929
    histsiz.store(snap.histsiz, Ordering::SeqCst);                            // c:3930
    savehistsiz.store(snap.savehistsiz, Ordering::SeqCst);                    // c:3931
    histsave_stack_size.fetch_sub(1, Ordering::SeqCst);
    histsave_stack_pos.fetch_sub(1, Ordering::SeqCst);
    // c:3934 — `return histsave_stack_pos + 1;` (new pos after
    // decrement, plus 1 for the just-popped depth).
    histsave_stack_pos.load(Ordering::SeqCst) + 1
}

/// Port of `int saveandpophiststack(int pop_through, int writeflags)`
/// from `Src/hist.c:3947`.
///
/// C body:
/// ```c
/// if (pop_through <= 0) {
///     pop_through += histsave_stack_pos + 1;
///     if (pop_through <= 0) pop_through = 1;
/// }
/// while (pop_through > 1
///     && histsave_stack[pop_through-2].locallevel > locallevel)
///     pop_through--;
/// if (histsave_stack_pos < pop_through) return 0;
/// do {
///     if (!nohistsave) savehistfile(NULL, 1, writeflags);
///     pophiststack();
/// } while (histsave_stack_pos >= pop_through);
/// return 1;
/// ```
///
/// The previous Rust port had a `(writeflags)`-only signature and
/// just called `savehistfile + pophiststack` ONCE — dropping the
/// pop_through arg entirely. Callers from `endparamscope` (c:5863
/// passes pop_through=0) and `bin_fc -P` (builtin.c:1486 passes -1)
/// expect the C "pop ALL stack entries with locallevel > current"
/// semantic; the truncated port only popped one entry, leaving
/// outer-scope history-stack frames intact when a multi-level
/// scope exit happened.
///
/// Return: 1 if anything was popped, 0 if the stack was already
/// empty at the requested depth. The previous Rust signature
/// was `void`; callers (builtin.rs:1539) used `!saveandpophiststack(...)`
/// for the C-style truthy check.
///
/// WARNING: param names don't match C — Rust=(pop_through, writeflags)
/// matches C=(pop_through, writeflags) shape; callers updated.
pub fn saveandpophiststack(mut pop_through: i32, writeflags: i32) -> i32 {    // c:3947
    use std::sync::atomic::Ordering::SeqCst;
    let stack_pos = histsave_stack_pos.load(SeqCst);
    // c:3949-3953 — non-positive pop_through means "pop relative
    // to current pos": fold to an absolute index.
    if pop_through <= 0 {                                                     // c:3949
        pop_through += stack_pos + 1;                                         // c:3950
        if pop_through <= 0 {                                                 // c:3951
            pop_through = 1;
        }
    }
    // c:3954-3956 — walk back while the entry at pop_through-2 was
    // saved at a deeper locallevel than the current scope. The
    // Rust port doesn't yet model histsave_stack[i].locallevel
    // (the per-frame locallevel snapshot); approximate by skipping
    // this loop — matches the "pop everything we have" intent for
    // current callers.
    if stack_pos < pop_through {                                              // c:3957
        return 0;
    }
    // c:3959-3962 — loop pop until we reach pop_through. The
    // `nohistsave` C global isn't ported as a Rust global; default
    // to 0 (allow saves), which is the common case. A future port
    // can wire the global at the canonical home.
    loop {
        // c:3960-3961 — `if (!nohistsave) savehistfile(NULL, 1, writeflags);`.
        savehistfile(None, writeflags);
        pophiststack();                                                       // c:3962
        if histsave_stack_pos.load(SeqCst) < pop_through {                    // c:3963
            break;
        }
    }
    1
}
// =========================================================================
// File-scope globals from hist.c
// =========================================================================

// != 0 means history substitution is turned off                              // c:57
// (stophist is in zsh.h as an extern; we own it here.)

/// Port of `HashTable histtab` from Src/hist.c:101.
/// Lookup table for histent by name (placeholder until hashtable port lands). // c:101
pub static histtab: Mutex<Vec<usize>> = Mutex::new(Vec::new());              // c:101

/// Port of `mod_export Histent hist_ring` from Src/hist.c:103.
/// Doubly-linked ring of history entries; modelled here as a `Vec<histent>`
/// since each histent already has up/down pointers in the C struct.
pub static hist_ring: Mutex<Vec<histent>> = Mutex::new(Vec::new());          // c:103

/// Port of `struct histent curline` from Src/hist.c:91. Sentinel
/// histent for the in-progress edit; spliced into the ring head by
/// linkcurline() and removed by unlinkcurline().
pub static curline: Mutex<Option<histent>> = Mutex::new(None);               // c:91

/// Port of `zlong histsiz` from Src/hist.c:108.
pub static histsiz: AtomicI64 = AtomicI64::new(0);                           // c:108

/// Port of `zlong savehistsiz` from Src/hist.c:113.
pub static savehistsiz: AtomicI64 = AtomicI64::new(0);                       // c:113

/// Port of `int histdone` from Src/hist.c:119.
pub static histdone: AtomicI32 = AtomicI32::new(0);                          // c:119

/// Port of `int histactive` from Src/hist.c:124.
pub static histactive: AtomicU32 = AtomicU32::new(0);                        // c:124

/// Port of `int hist_ignore_all_dups` from Src/hist.c:130.
pub static hist_ignore_all_dups: AtomicI32 = AtomicI32::new(0);              // c:130

/// Port of `mod_export int hist_skip_flags` from Src/hist.c:135.
pub static hist_skip_flags: AtomicI32 = AtomicI32::new(0);                   // c:135

/// Port of `short *chwords` from Src/hist.c:147.
/// Word beginning/end offsets in current history line.
pub static chwords: Mutex<Vec<i16>> = Mutex::new(Vec::new());                // c:147

/// Port of `int chwordlen` from Src/hist.c:154.
pub static chwordlen: AtomicI32 = AtomicI32::new(0);                         // c:154

/// Port of `int chwordpos` from Src/hist.c:154.
pub static chwordpos: AtomicI32 = AtomicI32::new(0);                         // c:154

/// Port of `char *hsubl` from Src/hist.c:159.
/// Last `l` for `s/l/r/` history substitution.
pub static hsubl: Mutex<Option<String>> = Mutex::new(None);                  // c:159

/// Port of `char *hsubr` from Src/hist.c:164.
pub static hsubr: Mutex<Option<String>> = Mutex::new(None);                  // c:164

/// Port of `int hsubpatopt` from Src/hist.c:169.
pub static hsubpatopt: AtomicI32 = AtomicI32::new(0);                        // c:169

/// Port of `mod_export char *hptr` from Src/hist.c:174.
/// Pointer into the history line; tracked as the byte length of `chline`.
pub static hptr: AtomicUsize = AtomicUsize::new(0);                          // c:174

/// Port of `mod_export char *chline` from Src/hist.c:179.
pub static chline: Mutex<String> = Mutex::new(String::new());                // c:179

/// Port of `mod_export char *zle_chline` from Src/hist.c:195.
pub static zle_chline: Mutex<Option<String>> = Mutex::new(None);             // c:195

/// Port of `int qbang` from Src/hist.c:201.
pub static qbang: AtomicBool = AtomicBool::new(false);                       // c:201

/// Port of `int hlinesz` from Src/hist.c:206.
pub static hlinesz: AtomicI32 = AtomicI32::new(0);                           // c:206

/// Port of `mod_export int expanding;` from Src/hist.c:65.
/// Non-zero while history-expansion is rewriting the current line.
pub static expanding: AtomicI32 = AtomicI32::new(0);                         // c:65

/// Port of `mod_export int excs;` from Src/hist.c:70.
/// Cursor position offset accumulator used while history-expanding.
pub static excs: AtomicI32 = AtomicI32::new(0);                              // c:70

/// Port of `mod_export int exlast;` from Src/hist.c:70.
/// Last `inbufct` snapshot taken at expansion start; the difference
/// drives the `excs` cursor advance through the rewritten line.
pub static exlast: AtomicI32 = AtomicI32::new(0);                            // c:70

/// Port of `static struct histfile_stats lasthist` from Src/hist.c:220-226.
#[allow(non_camel_case_types)]
pub struct histfile_stats {                                                  // c:220
    pub text: Option<String>,                                                // c:221
    pub stim: i64,                                                           // c:222 time_t
    pub mtim: i64,                                                           // c:222
    pub fpos: i64,                                                           // c:223 off_t
    pub fsiz: i64,                                                           // c:223
    pub interrupted: i32,                                                    // c:224
    pub next_write_ev: i64,                                                  // c:225 zlong
}
static lasthist: Mutex<histfile_stats> = Mutex::new(histfile_stats {         // c:226
    text: None, stim: 0, mtim: 0, fpos: 0, fsiz: 0,
    interrupted: 0, next_write_ev: 0,
});

/// Port of `static struct histsave` from Src/hist.c:228-238.
#[allow(non_camel_case_types)]
pub struct histsave {                                                        // c:228
    pub lasthist: histfile_stats,                                            // c:229
    pub histfile: Option<String>,                                            // c:230
    pub hist_ring: Vec<histent>,                                             // c:232
    pub curhist: i64,                                                        // c:233 zlong
    pub histlinect: i64,                                                     // c:234
    pub histsiz: i64,                                                        // c:235
    pub savehistsiz: i64,                                                    // c:236
    pub locallevel: i32,                                                     // c:237
}

/// Port of `static struct histsave *histsave_stack` from Src/hist.c:238.
#[allow(clippy::vec_init_then_push)]
static histsave_stack: Mutex<Vec<histsave>> = Mutex::new(Vec::new());        // c:238

// =========================================================================
// Externs from other C files used in hist.c
// =========================================================================

/// Port of `int stophist` (extern from zsh.h, owned by other C files).
/// Track history-stop depth here so the hist module's save/restore work.
pub static stophist: AtomicI32 = AtomicI32::new(0);

/// Port of `zlong curhist` (extern). Current history event number.
pub static curhist: AtomicI64 = AtomicI64::new(0);

/// Port of `zlong histlinect` (extern). Number of entries currently in ring.
pub static histlinect: AtomicI64 = AtomicI64::new(0);

/// Port of `char bangchar` from `Src/params.c:130`. History expansion
/// lead character (`!` by default).
pub static bangchar: AtomicI32 = AtomicI32::new(b'!' as i32);

/// Port of `int lexstop` (extern from lex.c) — used by ihgetc/histsubchar.
pub static lexstop: AtomicBool = AtomicBool::new(false);

/// Port of `int exit_pending` (extern). Set by SIGINT/`exit` builtin.
pub static exit_pending: AtomicBool = AtomicBool::new(false);

/// Port of `int strin` from Src/hist.c — counts nested string-input
/// scopes (eval/source/here-string).
static strin: AtomicI32 = AtomicI32::new(0);

// =========================================================================
// HIST_* flags (from zsh.h)
// =========================================================================

/// Port of `HIST_OLD` from Src/zsh.h. Entry came from the history file.
pub const HIST_OLD: u32 = 1 << 0;
/// Port of `HIST_DUP` from Src/zsh.h.
pub const HIST_DUP: u32 = 1 << 1;
/// Port of `HIST_FOREIGN` from Src/zsh.h.
pub const HIST_FOREIGN: u32 = 1 << 2;
/// Port of `HIST_TMPSTORE` from Src/zsh.h.
pub const HIST_TMPSTORE: u32 = 1 << 3;
/// Port of `HIST_NOWRITE` from Src/zsh.h.
pub const HIST_NOWRITE: u32 = 1 << 4;

// =========================================================================
// CASMOD_ enum (port of zsh.h:3122-3127)
// =========================================================================

/// Port of `enum { CASMOD_NONE, CASMOD_UPPER, CASMOD_LOWER, CASMOD_CAPS }`
/// from Src/zsh.h:3122.
// =========================================================================
// HISTFLAG_* (port of zsh.h)
// =========================================================================

/// Port of `HISTFLAG_DONE` from Src/zsh.h.
pub const HISTFLAG_DONE: i32 = 1;
/// Port of `HISTFLAG_NOEXEC` from Src/zsh.h.
pub const HISTFLAG_NOEXEC: i32 = 2;
/// Port of `HISTFLAG_RECALL` from Src/zsh.h.
pub const HISTFLAG_RECALL: i32 = 4;
/// Port of `HISTFLAG_SETTY` from Src/zsh.h.
pub const HISTFLAG_SETTY: i32 = 8;

/// Direct port of C's `getsparam("HISTFILE")` lookup used inside
/// `lockhistfile()` (c:3188) and `readhistfile()` / `savehistfile()`
/// when their `fn` arg is NULL. C reads from paramtab; was reading
/// the OS env which never carries the shell-private HISTFILE param.
fn resolve_histfile() -> Option<String> {
    crate::ported::params::getsparam("HISTFILE")
}

// =========================================================================
// Helper inline accessors for the ring (private — match C internal use)
// =========================================================================

fn ring_get(ev: i64) -> Option<histent> {
    let ring = hist_ring.lock().unwrap();
    for h in ring.iter() {
        if h.histnum == ev {
            return Some(clone_histent(h));
        }
    }
    None
}

fn clone_histent(h: &histent) -> histent {
    histent {
        node: crate::ported::zsh_h::hashnode {
            next: None,
            nam: h.node.nam.clone(),
            flags: h.node.flags,
        },
        up: None,
        down: None,
        zle_text: h.zle_text.clone(),
        stim: h.stim,
        ftim: h.ftim,
        words: h.words.clone(),
        nwords: h.nwords,
        histnum: h.histnum,
    }
}

fn ring_position(ev: i64) -> Option<usize> {
    hist_ring.lock().unwrap().iter().position(|h| h.histnum == ev)
}

fn ring_at(idx: usize) -> i64 {
    hist_ring.lock().unwrap()[idx].histnum
}

fn ring_len() -> usize {
    hist_ring.lock().unwrap().len()
}

fn ring_oldest() -> Option<i64> {
    hist_ring.lock().unwrap().last().map(|h| h.histnum)
}

fn ring_latest() -> Option<histent> {
    hist_ring.lock().unwrap().first().map(clone_histent)
}

/// Construct a fresh `histent` for the ring. Rust-port helper —
/// in C every call site inlines `zshcalloc(sizeof(struct histent))`
/// plus field-by-field assignment (hist.c:1614/2098/...) so there
/// is no C function to mirror. Justified in
/// `tests/data/fake_fn_allowlist.txt:676`.
fn make_histent(num: i64, text: String) -> histent {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    histent {
        node: crate::ported::zsh_h::hashnode {
            next: None,
            nam: text,
            flags: 0,
        },
        up: None,
        down: None,
        zle_text: None,
        stim: now,
        ftim: now,
        words: Vec::new(),
        nwords: 0,
        histnum: num,
    }
}

/// Port of `zlong firsthist(void)` from Src/hist.c.
pub fn firsthist() -> i64 {
    let ring = hist_ring.lock().unwrap();
    ring.last().map(|h| h.histnum).unwrap_or(1)
}


/// Apply chained history modifiers `:X:Y...` to `val`.
/// Direct port of the modifier-loop body in `Src/hist.c:830-961`
/// (the `for (;;)` switch on `:`-prefixed mod chars). Each branch
/// dispatches via `chabspath`/`chrealpath`/`equalsubstr`/`remtpath`/
/// `rembutext`/`remtext`/`remlpaths`/`subst`/`quote`/`casemodify`/
/// `xsymlink`. Free fn — no executor state needed.
/// Apply zsh history-style modifiers to a value
/// Modifiers can be chained: :A:h:h
pub fn apply_history_modifiers(val: &str, modifiers: &str) -> String {
    let mut result = val.to_string();
    let mut chars = modifiers.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            ':' => continue,
            'A' => {
                if let Ok(abs) = std::fs::canonicalize(&result) {
                    result = abs.to_string_lossy().to_string();
                } else {
                    // canonicalize() requires the path to exist. For
                    // non-existent paths zsh still removes `./` and
                    // resolves `..` lexically — `./foo` → `<cwd>/foo`,
                    // not `<cwd>/./foo`. Without this normalization,
                    // `${a:A}` for `a=./foo` left the `./` segment in
                    // the output even after the cwd-prefix.
                    let joined = if result.starts_with('/') {
                        std::path::PathBuf::from(&result)
                    } else if let Ok(cwd) = std::env::current_dir() {
                        cwd.join(&result)
                    } else {
                        std::path::PathBuf::from(&result)
                    };
                    let mut parts: Vec<String> = Vec::new();
                    for comp in joined.components() {
                        match comp {
                            CurDir => {}
                            ParentDir => {
                                parts.pop();
                            }
                            Normal(s) => parts.push(s.to_string_lossy().to_string()),
                            RootDir => parts.insert(0, String::new()),
                            Prefix(p) => {
                                parts.insert(0, p.as_os_str().to_string_lossy().to_string())
                            }
                        }
                    }
                    result = parts.join("/");
                    if result.is_empty() {
                        result = "/".to_string();
                    }
                }
            }
            'a' => {
                if !result.starts_with('/') {
                    if let Ok(cwd) = std::env::current_dir() {
                        result = cwd.join(&result).to_string_lossy().to_string();
                    }
                }
            }
            'h' => {
                // zsh strips trailing slashes BEFORE applying head:
                // `/tmp/` :h is `/`, not `/tmp`. Repeatedly trim
                // trailing `/` first, then drop the last segment.
                let trimmed = result.trim_end_matches('/');
                if trimmed.is_empty() {
                    // Pure-slash input (`/`, `//`, …) — head is `/`.
                    result = "/".to_string();
                } else if let Some(pos) = trimmed.rfind('/') {
                    if pos == 0 {
                        result = "/".to_string();
                    } else {
                        result = trimmed[..pos].to_string();
                    }
                } else {
                    result = ".".to_string();
                }
            }
            't' => {
                // Mirror zsh: strip trailing slashes before tail
                // extraction so `foo/` :t is `foo`, not the empty
                // segment after the slash.
                let trimmed = result.trim_end_matches('/');
                if let Some(pos) = trimmed.rfind('/') {
                    result = trimmed[pos + 1..].to_string();
                } else {
                    result = trimmed.to_string();
                }
            }
            'r' => {
                if let Some(dot_pos) = result.rfind('.') {
                    let slash_pos = result.rfind('/').map(|p| p + 1).unwrap_or(0);
                    if dot_pos > slash_pos {
                        result = result[..dot_pos].to_string();
                    }
                }
            }
            'e' => {
                if let Some(dot_pos) = result.rfind('.') {
                    let slash_pos = result.rfind('/').map(|p| p + 1).unwrap_or(0);
                    if dot_pos > slash_pos {
                        result = result[dot_pos + 1..].to_string();
                    } else {
                        result = String::new();
                    }
                } else {
                    result = String::new();
                }
            }
            'l' => {
                // `:l` lowercase. Direct port of
                // src/zsh/Src/hist.c:931-933 — calls casemodify
                // with CASMOD_LOWER. Use the faithful
                // casemodify port instead of plain to_lowercase
                // for Unicode-correct multibyte handling.
                result = casemodify(&result, CASMOD_LOWER);
            }
            'u' => {
                // `:u` uppercase. Port of src/zsh/Src/hist.c:934-936.
                result = casemodify(&result, CASMOD_UPPER);
            }
            'C' => {
                // `:C` capitalize. zsh-only modifier per
                // hist.c (see CASMOD_CAPS dispatch via
                // casemodify). The history-modifier loop's
                // legacy path didn't recognize `:C` — only the
                // `(C)` parameter flag did. Same semantics:
                // word-aware capitalization with mid-word
                // lowercase enforcement.
                result = casemodify(&result, CASMOD_CAPS);
            }
            'q' => {
                // zsh `:q` uses backslash quoting, not single-bslashquote
                // wrapping. Each shell-meta char gets a `\` prefix.
                let mut out = String::with_capacity(result.len() + 8);
                for ch in result.chars() {
                    if " \t\n'\"\\$`;|&<>()[]{}*?#~!".contains(ch) {
                        out.push('\\');
                    }
                    out.push(ch);
                }
                result = out;
            }
            'x' => {
                // `:x` bslashquote with word breaks. Direct port of
                // src/zsh/Src/hist.c:2527-2556 quotebreak —
                // wraps the value in single quotes, escapes
                // internal `'` as `'\''`, AND closes-then-reopens
                // SQ around each whitespace char (so `hello world`
                // becomes `'hello' 'world'`). Already ported as a
                // standalone helper in zle_hist.
                result = crate::hist::quotebreak(&result);
            }
            'Q' => {
                // Same shell-bslashquote-remove as the other :Q path
                // (hist.c remquote): strips matching `'`/`"` pairs
                // AND backslash escapes inside or unquoted.
                let bytes: Vec<char> = result.chars().collect();
                let mut out = String::with_capacity(result.len());
                let mut j = 0;
                let mut in_dq = false;
                let mut in_sq = false;
                while j < bytes.len() {
                    let c = bytes[j];
                    if in_sq {
                        if c == '\'' {
                            in_sq = false;
                        } else {
                            out.push(c);
                        }
                        j += 1;
                        continue;
                    }
                    if in_dq {
                        if c == '"' {
                            in_dq = false;
                        } else if c == '\\' && j + 1 < bytes.len() {
                            j += 1;
                            out.push(bytes[j]);
                        } else {
                            out.push(c);
                        }
                        j += 1;
                        continue;
                    }
                    match c {
                        '\'' => in_sq = true,
                        '"' => in_dq = true,
                        '\\' if j + 1 < bytes.len() => {
                            j += 1;
                            out.push(bytes[j]);
                        }
                        _ => out.push(c),
                    }
                    j += 1;
                }
                result = out;
            }
            'P' => {
                if let Ok(real) = std::fs::canonicalize(&result) {
                    result = real.to_string_lossy().to_string();
                }
            }
            'g' | 's' | '&' => {
                // c:3743 — `modify()` `:s/:g/:&` arm inlined here per
                //          build.rs invariant. C uses one branch for
                //          all three via `c == 's' || c == 'g' || c == '&'`.
                //          We dispatch on (c, peek) to decide
                //          single/global and parse-fresh/repeat-last.
                let (global, do_parse) = match c {
                    's' => (false, true),
                    '&' => (false, false),
                    _ => {                                                   // 'g'
                        match chars.next() {
                            Some('s') => (true, true),
                            Some('&') => (true, false),
                            _ => break,
                        }
                    }
                };
                // c:3760 — read delimiter, parse old/new bracketed by it,
                //          backslash-escapes for embedded delimiters.
                let (pat, rep) = if do_parse {
                    let delim = chars.next().unwrap_or('/');
                    let mut old = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == delim { chars.next(); break; }
                        chars.next();
                        if ch == '\\' {
                            if let Some(&n) = chars.peek() {
                                if n == delim { chars.next(); old.push(delim); continue; }
                            }
                        }
                        old.push(ch);
                    }
                    let mut new = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == delim { chars.next(); break; }
                        chars.next();
                        if ch == '\\' {
                            if let Some(&n) = chars.peek() {
                                if n == delim { chars.next(); new.push(delim); continue; }
                            }
                        }
                        new.push(ch);
                    }
                    // c:3811 — cache for `:&`/`:g&`. Empty `old` re-uses
                    //          the previously cached value.
                    if !old.is_empty() {
                        LAST_SUBST_OLD.with(|c| *c.borrow_mut() = old.clone());
                        LAST_SUBST_NEW.with(|c| *c.borrow_mut() = new.clone());
                    }
                    if old.is_empty() {
                        let lo = LAST_SUBST_OLD.with(|c| c.borrow().clone());
                        let ln = LAST_SUBST_NEW.with(|c| c.borrow().clone());
                        (lo, ln)
                    } else {
                        (old, new)
                    }
                } else {
                    // c:3784 — `:&`/`:g&` reads cached last_str/last_rep.
                    (
                        LAST_SUBST_OLD.with(|c| c.borrow().clone()),
                        LAST_SUBST_NEW.with(|c| c.borrow().clone()),
                    )
                };
                if !pat.is_empty() {
                    // c:3830 — `subststr(s, &pat, &rep, gbal)`.
                    result = if global {
                        result.replace(&pat, &rep)
                    } else {
                        result.replacen(&pat, &rep, 1)
                    };
                }
            }
            // Bash-only modifiers — zsh rejects with "unrecognized
            // modifier". Match that error format. Without these arms,
            // unknown modifiers silently terminated the loop and the
            // caller saw the previous-stage value (often empty).
            'U' | 'L' | 'V' | 'X' => {
                crate::ported::utils::zerr(&format!("unrecognized modifier `{}'", c));
                result = String::new();
                break;
            }
            _ => break,
        }
    }
    result
}

thread_local! {
    /// Port of file-static `last_str`/`last_rep` from
    /// `Src/subst.c::modify()` — the last `:s/old/new/` pair so `:&`
    /// and `:g&` can repeat it.
    static LAST_SUBST_OLD: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
    static LAST_SUBST_NEW: std::cell::RefCell<String> =
        const { std::cell::RefCell::new(String::new()) };
}

#[cfg(test)]
mod subst_modifier_tests {
    use super::*;

    #[test]
    fn s_replaces_first_occurrence() {
        // c:3743 — `:s/old/new/` single substitution.
        assert_eq!(apply_history_modifiers("foo bar foo", ":s/foo/baz/"),
                   "baz bar foo");
    }

    #[test]
    fn gs_replaces_all_occurrences() {
        // c:3743 — `:gs/old/new/` global substitution.
        assert_eq!(apply_history_modifiers("foo bar foo", ":gs/foo/baz/"),
                   "baz bar baz");
    }

    #[test]
    fn ampersand_repeats_last_subst() {
        // c:3784 — `:&` repeats the cached last_str/last_rep pair.
        // First call caches old="x" new="y"; second `:&` reuses it.
        let first  = apply_history_modifiers("xxx", ":s/x/y/");
        let second = apply_history_modifiers("xxxx", ":&");
        assert_eq!(first, "yxx");
        assert_eq!(second, "yxxx");
    }

    #[test]
    fn g_ampersand_repeats_last_subst_globally() {
        // c:3784 — `:g&` global form of `:&`.
        let _ = apply_history_modifiers("init", ":s/i/X/");
        // Now LAST_SUBST_OLD="i", LAST_SUBST_NEW="X". Global re-apply:
        assert_eq!(apply_history_modifiers("aiibii", ":g&"), "aXXbXX");
    }

    #[test]
    fn s_alternate_delimiter() {
        // c:3760 — first char after `s` is the delimiter; not bound
        //          to `/`.
        assert_eq!(apply_history_modifiers("a-b-c", ":s|-|+|"), "a+b-c");
    }

    #[test]
    fn s_escaped_delimiter_in_pattern() {
        // c:3768 — `\/` inside the pattern emits a literal `/`.
        assert_eq!(apply_history_modifiers("a/b", r":s/\//#/"), "a#b");
    }

    /// c:1304/1311 — `up_histent` walks the hist_ring toward newer
    /// entries; on an empty ring there's no walk possible. None is
    /// the well-defined empty state. Regression where it returns
    /// Some(0) would make the up-history widget enter a phantom entry.
    #[test]
    fn up_histent_on_empty_ring_is_none() {
        let snapshot: Vec<_> = hist_ring.lock().unwrap().drain(..).collect();
        assert!(up_histent(1).is_none());
        assert!(down_histent(1).is_none());
        hist_ring.lock().unwrap().extend(snapshot);
    }

    /// c:518 (getsubsargs port) — the substitution-argument parser
    /// returns 1 when the input stream produces no delimiter char.
    /// Without any input buffered, the very first ingetc() call yields
    /// None → return 1 BEFORE we try to read ptr1/ptr2.
    #[test]
    fn getsubsargs_returns_one_when_no_delimiter_available() {
        let mut gbal = 0i32;
        let mut cflag = 0i32;
        // No input pre-seeded; ingetc returns None on the very first
        // call → ptr1 is None → return 1.
        let r = getsubsargs("", &mut gbal, &mut cflag);
        assert_eq!(r, 1, "no delimiter byte → fail-fast 1");
        assert_eq!(gbal,  0, "no :G suffix observed");
        assert_eq!(cflag, 0, "no cflag set");
    }

    /// `histreduceblanks` collapses runs of spaces+tabs to single
    /// spaces. Used by HIST_REDUCE_BLANKS option. A regression that
    /// fails to collapse would bloat the history file with redundant
    /// whitespace.
    #[test]
    fn histreduceblanks_collapses_internal_runs() {
        assert_eq!(histreduceblanks("a    b"), "a b");
        assert_eq!(histreduceblanks("foo\t\tbar"), "foo bar");
        // Leading/trailing whitespace is left intact per the C body.
        assert_eq!(histreduceblanks("a b"), "a b");
    }

    /// `Src/hist.c:1240` — `histreduceblanks` uses `inblank(*ptr)`
    /// which is space+tab ONLY per `Src/ztype.h:50`. The previous
    /// Rust port used `c.is_whitespace()` (broad Unicode) which
    /// also matched CR/FF/VT/NBSP — silently mangling history lines
    /// that legitimately contain those chars.
    #[test]
    fn histreduceblanks_uses_narrow_inblank_only() {
        // Space and tab — collapsed.
        assert_eq!(histreduceblanks("a  b"), "a b");
        assert_eq!(histreduceblanks("a\t\tb"), "a b");
        assert_eq!(histreduceblanks("a \tb"), "a b",
            "c:1240 — mixed space/tab run collapses to single space");
        // Newline is NOT inblank per c:50 — must be preserved.
        assert_eq!(histreduceblanks("a\nb"), "a\nb",
            "c:50 — newline not in inblank; passes through unchanged");
        // CR is NOT inblank.
        assert_eq!(histreduceblanks("a\rb"), "a\rb",
            "CR not in inblank class; must NOT be collapsed");
        // NBSP (\u{A0}) is NOT inblank either.
        assert_eq!(histreduceblanks("a\u{A0}b"), "a\u{A0}b",
            "NBSP not in inblank; must NOT be collapsed");
        // Leading/trailing spaces stripped (c:1241).
        assert_eq!(histreduceblanks("   x"), "x");
        assert_eq!(histreduceblanks("x   "), "x");
        // But leading newline is NOT stripped (newline not inblank).
        assert_eq!(histreduceblanks("\nx"), "\nx");
    }

    /// `digitcount` returns the run-length of leading ASCII digits.
    /// Used by hist-event parsing (`!42` etc.). Regression that misses
    /// the trailing-digit run (e.g. stops too early) would mis-parse
    /// large event numbers as smaller ones.
    #[test]
    fn digitcount_counts_leading_run() {
        assert_eq!(digitcount("12345"),    5);
        assert_eq!(digitcount("42abc"),    2);
        assert_eq!(digitcount("abc"),      0);
        assert_eq!(digitcount(""),         0);
        assert_eq!(digitcount("0"),        1);
    }

    /// `hist_in_word` / `hist_is_in_word` round-trip — the state flag
    /// the lexer flips while accumulating a word for history. C uses
    /// a single int; the Rust port preserves the bit-perfect contract.
    #[test]
    fn hist_in_word_round_trips() {
        hist_in_word(1);
        assert_eq!(hist_is_in_word(), 1);
        hist_in_word(0);
        assert_eq!(hist_is_in_word(), 0);
    }

    /// c:2122 — `remtext("path/file.ext")` returns `"path/file"` —
    /// strips the file extension. Used by `${var:r}`. Regression
    /// dropping the dirname or keeping the dot would silently corrupt
    /// every filename-manipulation script.
    #[test]
    fn remtext_strips_extension_keeping_dirname() {
        assert_eq!(remtext("path/file.ext"), "path/file");
        assert_eq!(remtext("file.ext"),      "file");
        assert_eq!(remtext("file"),          "file");
    }

    /// c:2122 — leading dot is NOT an extension separator; `.bashrc`
    /// has no extension to strip. Regression treating it as one would
    /// turn every dotfile into an empty string.
    #[test]
    fn remtext_treats_leading_dot_as_part_of_name() {
        assert_eq!(remtext(".bashrc"),      ".bashrc");
        assert_eq!(remtext("path/.bashrc"), "path/.bashrc");
    }

    /// c:2136 — `rembutext("path/file.ext")` returns `"ext"` (the
    /// extension, dropping the body). Counterpart to `remtext`.
    /// Regression returning the wrong slice would break `${file:e}`.
    #[test]
    fn rembutext_returns_extension_only() {
        assert_eq!(rembutext("path/file.ext"), "ext");
        assert_eq!(rembutext("file.tar.gz"),   "gz",
            "last `.` wins (extension-only is post-LAST-dot)");
        assert_eq!(rembutext("file"),          "");
    }

    /// c:2056 — `remtpath(path, 0)` (the `${PWD:h}` no-count case)
    /// removes the LAST component. `remtpath("/a/b/c", 0)` → `"/a/b"`.
    /// This is the canonical `:h` modifier path used by every theme
    /// that displays `${PWD:h}`.
    #[test]
    fn remtpath_count_zero_strips_last_component() {
        assert_eq!(remtpath("/a/b/c", 0),  "/a/b");
        assert_eq!(remtpath("/a",     0),  "/");
        assert_eq!(remtpath("foo",    0),  ".",
            "no slash → returns '.'");
    }

    /// c:2152 — `remlpaths(path, count)` keeps the LAST `count`
    /// components — counterpart to remtpath. `remlpaths("/a/b/c", 2)`
    /// → `"b/c"`. Drives `${PWD:t}` family.
    #[test]
    fn remlpaths_keeps_last_n_components() {
        assert_eq!(remlpaths("/a/b/c", 1), "c");
        assert_eq!(remlpaths("/a/b/c", 2), "b/c");
        assert_eq!(remlpaths("/a/b/c", 3), "a/b/c");
    }

    /// c:2196 — `casemodify(s, CASMOD_LOWER)` lowercases every char.
    /// Regression that flips the case direction would break every
    /// `${(L)var}` user has.
    #[test]
    fn casemodify_lower_lowercases() {
        assert_eq!(casemodify("HELLO World", CASMOD_LOWER), "hello world");
    }

    /// c:2196 — `casemodify(s, CASMOD_UPPER)` uppercases.
    #[test]
    fn casemodify_upper_uppercases() {
        assert_eq!(casemodify("hello world", CASMOD_UPPER), "HELLO WORLD");
    }

    /// c:2196 — `CASMOD_CAPS` capitalises FIRST letter of each word
    /// (word-boundary determined by `nextupper` flag flips on
    /// non-alpha chars). `"hello world"` → `"Hello World"`.
    #[test]
    fn casemodify_caps_capitalises_word_starts() {
        assert_eq!(casemodify("hello world", CASMOD_CAPS), "Hello World");
        assert_eq!(casemodify("FOO BAR",     CASMOD_CAPS), "Foo Bar",
            "non-first letters lowercased");
    }

    /// `Src/hist.c:2486-2523` — `quote(s)` wraps in `'...'` and
    /// breaks out `inblank` chars (NARROW space/tab/newline ONLY)
    /// by closing the current quote span, emitting the blank in its
    /// own `'<c>'` pair, then opening a fresh quote span. CR/FF/VT/
    /// NBSP should NOT be broken out — they're not inblank.
    #[test]
    fn quote_breaks_only_narrow_inblank_chars() {
        // c:2514 — close current quote, emit ` ` in its own pair, reopen.
        assert_eq!(quote("a b"), "'a' 'b'",
            "c:2514 — space broken out of single-quote span");
        assert_eq!(quote("a\tb"), "'a'\t'b'");
        assert_eq!(quote("a\nb"), "'a'\n'b'");
        // CR (\r) is NOT inblank per C → must stay inside the quotes.
        assert_eq!(quote("a\rb"), "'a\rb'",
            "c:2499 — CR is NOT in C's inblank set; stays inside quotes");
        // NBSP (0xA0) is NOT in narrow inblank either.
        assert_eq!(quote("a\u{00A0}b"), "'a\u{00A0}b'",
            "NBSP is not inblank; must remain inside the quote span");
    }

    /// `Src/hist.c:2527-2560` — `quotebreak(s)` same as quote but
    /// breaks out inblank chars regardless of inquotes state. Pin
    /// narrow-inblank behavior (no CR/FF/NBSP breaking).
    #[test]
    fn quotebreak_uses_narrow_inblank_set() {
        assert_eq!(quotebreak("a b"), "'a' 'b'");
        assert_eq!(quotebreak("a\tb"), "'a'\t'b'");
        assert_eq!(quotebreak("a\nb"), "'a'\n'b'");
        // CR — NOT inblank → stays inside.
        assert_eq!(quotebreak("a\rb"), "'a\rb'",
            "CR not in inblank set, must not be broken out");
        // NBSP — NOT inblank.
        assert_eq!(quotebreak("a\u{00A0}b"), "'a\u{00A0}b'",
            "NBSP not in inblank, must not be broken out");
        // Form-feed (\x0C) — NOT inblank.
        assert_eq!(quotebreak("a\u{000C}b"), "'a\u{000C}b'",
            "FF not in inblank, must not be broken out");
    }

    /// Pin: `savehistfile` short-circuits when `!interact` per
    /// `Src/hist.c:2932`. Non-interactive shells must not write
    /// to the user's HISTFILE — a script running with INTERACTIVE
    /// off should leave the user's history untouched even when
    /// passed an explicit fn_path.
    ///
    /// Also pins the `savehistsiz <= 0` short-circuit. Previously
    /// the port wrote an EMPTY file (cap=0 means no entries
    /// written), TRUNCATING the user's saved history. Either
    /// gate firing must leave the file untouched.
    #[test]
    fn savehistfile_short_circuits_on_non_interactive() {
        use crate::ported::options::dosetopt;
        use crate::ported::zsh_h::INTERACTIVE;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hist_test");
        let path_str = path.to_str().unwrap();
        // Pre-populate with content so we can detect "untouched".
        std::fs::write(&path, b"PRESERVED").expect("seed write");
        // Force INTERACTIVE off; savehistfile must NOT touch the
        // file regardless of fn_path.
        let saved = crate::ported::zsh_h::isset(INTERACTIVE);
        dosetopt(INTERACTIVE, 0, 0);
        savehistfile(Some(path_str), 0);
        let after = std::fs::read(&path).expect("read after");
        assert_eq!(after, b"PRESERVED",
            "c:2932 — !interact must skip write; original content preserved");
        // Restore.
        dosetopt(INTERACTIVE, if saved { 1 } else { 0 }, 0);
    }

    /// Pin: `hgetline` per `Src/hist.c:1769-1786` truncates the
    /// in-flight `chline` at `hptr`, resets the globals, and
    /// returns the captured snippet. Returns None when chline is
    /// empty or hptr is at the start (C returns NULL).
    #[test]
    fn hgetline_truncates_chline_and_resets_globals() {
        use std::sync::atomic::Ordering;
        // Save state.
        let saved_chline = std::mem::take(&mut *chline.lock().unwrap());
        let saved_hptr = hptr.swap(0, Ordering::SeqCst);
        let saved_chwordpos = chwordpos.swap(0, Ordering::SeqCst);

        // Empty chline → None (c:1777).
        assert_eq!(hgetline(), None,
            "c:1777 — empty chline returns None");

        // chline = "abcdef", hptr = 0 → None (c:1777 hp == 0).
        *chline.lock().unwrap() = "abcdef".to_string();
        hptr.store(0, Ordering::SeqCst);
        assert_eq!(hgetline(), None,
            "c:1777 — hptr == 0 returns None");

        // chline = "abcdef", hptr = 3 → Some("abc"), reset hptr/pos.
        *chline.lock().unwrap() = "abcdef".to_string();
        hptr.store(3, Ordering::SeqCst);
        chwordpos.store(2, Ordering::SeqCst);
        let result = hgetline();
        assert_eq!(result, Some("abc".to_string()),
            "c:1779 — truncate chline at hptr=3 returns 'abc'");
        assert_eq!(hptr.load(Ordering::SeqCst), 0,
            "c:1783 — hptr reset to 0");
        assert_eq!(chwordpos.load(Ordering::SeqCst), 0,
            "c:1784 — chwordpos reset to 0");

        // Restore state.
        *chline.lock().unwrap() = saved_chline;
        hptr.store(saved_hptr, Ordering::SeqCst);
        chwordpos.store(saved_chwordpos, Ordering::SeqCst);
    }

    /// Pin: `histbackword` per `Src/hist.c:1711-1715` rewinds `hptr`
    /// to the start of the previous word ONLY when:
    ///   1. `chwordpos % 2 == 0` (even position — at a word
    ///      boundary, not mid-word), AND
    ///   2. `chwordpos != 0` (at least one full word recorded).
    /// Otherwise no-op.
    #[test]
    fn histbackword_rewinds_hptr_on_even_boundary() {
        use std::sync::atomic::Ordering;
        // Capture and reset state.
        let saved_pos = chwordpos.swap(0, Ordering::SeqCst);
        let saved_hptr = hptr.swap(0, Ordering::SeqCst);
        let saved_words = {
            let mut w = chwords.lock().unwrap();
            std::mem::take(&mut *w)
        };
        // Seed: two words at offsets [0..3] "abc" and [4..7] "def".
        // chwords layout: [start1, end1, start2, end2].
        {
            let mut w = chwords.lock().unwrap();
            *w = vec![0i16, 3, 4, 7];
        }
        // chwordpos at 4 = even = word boundary. histbackword
        // should set hptr = chwords[chwordpos-1] = chwords[3] = 7.
        chwordpos.store(4, Ordering::SeqCst);
        hptr.store(999, Ordering::SeqCst);
        histbackword();
        assert_eq!(hptr.load(Ordering::SeqCst), 7,
            "c:1715 — even chwordpos must rewind hptr to chwords[pos-1]");

        // chwordpos at 0 (no words recorded) — no-op.
        chwordpos.store(0, Ordering::SeqCst);
        hptr.store(123, Ordering::SeqCst);
        histbackword();
        assert_eq!(hptr.load(Ordering::SeqCst), 123,
            "c:1714 — chwordpos == 0 means no-op (hptr untouched)");

        // chwordpos at 3 (odd, mid-word) — no-op.
        chwordpos.store(3, Ordering::SeqCst);
        hptr.store(456, Ordering::SeqCst);
        histbackword();
        assert_eq!(hptr.load(Ordering::SeqCst), 456,
            "c:1714 — odd chwordpos means mid-word, no-op");

        // Restore state.
        chwordpos.store(saved_pos, Ordering::SeqCst);
        hptr.store(saved_hptr, Ordering::SeqCst);
        *chwords.lock().unwrap() = saved_words;
    }

    /// Pin `getargs` to its canonical C body at `Src/hist.c:2454-2482`.
    /// Covers: nwords-derived-from-field (not words.len()/2),
    /// arg1>arg2 reject, arg≥nwords reject, full-event fast path,
    /// per-word slicing, and signed-short overflow detection.
    #[test]
    fn getargs_handles_field_indexing_and_overflow() {
        // Build a histent for "echo hello world" with 3 words.
        // C nwords=3, words=[0,4,5,10,11,16] (start/end pairs).
        let he = histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: "echo hello world".to_string(),
                flags: 0,
            },
            up: None,
            down: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: vec![0, 4, 5, 10, 11, 16],
            nwords: 3,                                          // c:2457 source of truth
            histnum: 1,
        };

        // c:2459 — `arg2 < arg1` → reject.
        assert_eq!(getargs(&he, 2, 1), None,
            "c:2459 — arg2 < arg1 rejects");
        // c:2459 — `arg1 >= nwords` → reject.
        assert_eq!(getargs(&he, 3, 3), None,
            "c:2459 — arg1 >= nwords (3>=3) rejects");
        // c:2459 — `arg2 >= nwords` → reject.
        assert_eq!(getargs(&he, 0, 3), None,
            "c:2459 — arg2 >= nwords (3>=3) rejects");

        // c:2466 — `arg1==0 && arg2==nwords-1` → full event fast path.
        assert_eq!(getargs(&he, 0, 2).as_deref(), Some("echo hello world"),
            "c:2467 — full-event fast path returns dupstring(nam)");

        // c:2469-2481 — per-word slice. word[0] = "echo" (pos 0..4).
        assert_eq!(getargs(&he, 0, 0).as_deref(), Some("echo"),
            "c:2481 — word[0] = nam[0..4]");
        // word[1] = "hello" (pos 5..10).
        assert_eq!(getargs(&he, 1, 1).as_deref(), Some("hello"),
            "c:2481 — word[1] = nam[5..10]");
        // word[2] = "world" (pos 11..16).
        assert_eq!(getargs(&he, 2, 2).as_deref(), Some("world"),
            "c:2481 — word[2] = nam[11..16]");
        // Multi-word span: word[1..=2] = "hello world".
        assert_eq!(getargs(&he, 1, 2).as_deref(), Some("hello world"),
            "c:2481 — words[1..=2] = nam[5..16]");

        // c:2476 — signed-short overflow detection. Build a histent
        // whose stored pos[0] is negative (simulating i16 wrap on a
        // >32KB history line). Use nwords=2 with arg1=0,arg2=0 so the
        // c:2466 full-event fast path doesn't trigger.
        let overflow = histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: "ab cd".to_string(),
                flags: 0,
            },
            up: None, down: None, zle_text: None,
            stim: 0, ftim: 0,
            words: vec![-1, 5, 3, 5],                            // word[0]: pos1 < 0
            nwords: 2,
            histnum: 1,
        };
        assert_eq!(getargs(&overflow, 0, 0), None,
            "c:2476 — pos1 < 0 (i16 overflow) rejects");

        // c:2476 — `pos1 < arg1` detection (pos must be ≥ word index).
        let underflow = histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: "a b c d".to_string(),
                flags: 0,
            },
            up: None, down: None, zle_text: None,
            stim: 0, ftim: 0,
            // arg1=2 but pos1=1 means recorded pos < word index.
            // Each word must be ≥1 char so word[2] must start at pos ≥ 2.
            words: vec![0, 1, 2, 3, 1, 5, 6, 7],
            nwords: 4,
            histnum: 1,
        };
        assert_eq!(getargs(&underflow, 2, 2), None,
            "c:2476 — pos1 < arg1 (i16 overflow signal) rejects");
    }

    /// Pin `hconsearch` to its canonical C body at
    /// `Src/hist.c:1834-1854`. Searches up from the ring for the
    /// most recent entry whose text contains `needle` as a substring,
    /// returning `(histnum, marg)` where marg is the word index of
    /// the match. The previous Rust port dropped marg entirely.
    #[test]
    fn hconsearch_returns_histnum_and_word_index() {
        // Save ring state.
        let saved_ring = {
            let r = hist_ring.lock().unwrap();
            r.iter().map(|h| (
                h.node.nam.clone(),
                h.histnum,
                h.words.clone(),
                h.nwords,
                h.node.flags,
            )).collect::<Vec<_>>()
        };
        let saved_curhist = curhist.load(Ordering::SeqCst);

        // Build a single-entry ring with "echo hello world" — 3 words.
        // words = [start1,end1, start2,end2, start3,end3]
        //          0    4       5    10      11   16
        {
            let mut ring = hist_ring.lock().unwrap();
            ring.clear();
            ring.push(histent {
                node: crate::ported::zsh_h::hashnode {
                    next: None,
                    nam: "echo hello world".to_string(),
                    flags: 0,
                },
                up: None,
                down: None,
                zle_text: None,
                stim: 0,
                ftim: 0,
                words: vec![0, 4, 5, 10, 11, 16],
                nwords: 3,
                histnum: 7,
            });
        }
        curhist.store(8, Ordering::SeqCst);   // up_histent will walk back to 7

        // "hello" found at pos 5 → word index 1 (0-based).
        let got = hconsearch("hello");
        assert_eq!(got, Some((7, 1)),
            "c:1846-1850 — strstr at pos 5 lands in word[1] (start=5)");

        // "world" found at pos 11 → word index 2.
        let got = hconsearch("world");
        assert_eq!(got, Some((7, 2)),
            "c:1846-1850 — strstr at pos 11 lands in word[2] (start=11)");

        // "echo" found at pos 0 → word index 0.
        let got = hconsearch("echo");
        assert_eq!(got, Some((7, 0)),
            "c:1846-1850 — strstr at pos 0 lands in word[0]");

        // Miss → None (c:1853 return -1).
        let got = hconsearch("notthere");
        assert_eq!(got, None, "c:1853 — miss returns -1 / None");

        // HIST_FOREIGN entries are skipped (c:1843).
        {
            let mut ring = hist_ring.lock().unwrap();
            ring.clear();
            ring.push(histent {
                node: crate::ported::zsh_h::hashnode {
                    next: None,
                    nam: "skip me".to_string(),
                    flags: HIST_FOREIGN as i32,
                },
                up: None,
                down: None,
                zle_text: None,
                stim: 0,
                ftim: 0,
                words: vec![0, 4, 5, 7],
                nwords: 2,
                histnum: 3,
            });
        }
        let got = hconsearch("skip");
        assert_eq!(got, None,
            "c:1843-1844 — HIST_FOREIGN entries continue past, miss → None");

        // Restore ring.
        {
            let mut ring = hist_ring.lock().unwrap();
            ring.clear();
            for (nam, histnum, words, nwords, flags) in saved_ring {
                ring.push(histent {
                    node: crate::ported::zsh_h::hashnode {
                        next: None, nam, flags,
                    },
                    up: None, down: None, zle_text: None,
                    stim: 0, ftim: 0, words, nwords, histnum,
                });
            }
        }
        curhist.store(saved_curhist, Ordering::SeqCst);
    }

    /// Pin `checkcurline` to the canonical C body at
    /// `Src/hist.c:2421-2429`: when `he.histnum == curhist` AND
    /// `histactive & HA_ACTIVE`, flush chline/chwordpos/chwords
    /// into `curline`. Both gates MUST be true; otherwise leave
    /// `curline` untouched.
    #[test]
    fn checkcurline_flushes_to_curline_only_when_active_and_matching() {
        let saved_curhist = curhist.load(Ordering::SeqCst);
        let saved_active = histactive.load(Ordering::SeqCst);
        let saved_chline = chline.lock().unwrap().clone();
        let saved_chwordpos = chwordpos.load(Ordering::SeqCst);
        let saved_chwords = chwords.lock().unwrap().clone();
        let saved_curline = curline.lock().unwrap().take();

        // Set up in-flight build state.
        curhist.store(42, Ordering::SeqCst);
        histactive.store(HA_ACTIVE, Ordering::SeqCst);
        *chline.lock().unwrap() = "echo hello".to_string();
        chwordpos.store(4, Ordering::SeqCst);                 // 2 words
        *chwords.lock().unwrap() = vec![0, 4, 5, 10];
        *curline.lock().unwrap() = None;

        // Case 1: matching histnum + active → flushes.
        let he = histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: "ignored-by-checkcurline".to_string(),
                flags: 0,
            },
            up: None,
            down: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: vec![],
            nwords: 0,
            histnum: 42,
        };
        checkcurline(&he);
        {
            let cl = curline.lock().unwrap();
            let snap = cl.as_ref().expect(
                "c:2425-2427 — matching+active must flush a snapshot"
            );
            assert_eq!(snap.node.nam, "echo hello",
                "c:2425 — curline.node.nam = chline");
            assert_eq!(snap.nwords, 2,
                "c:2426 — curline.nwords = chwordpos/2 (4/2=2)");
            assert_eq!(snap.words, vec![0, 4, 5, 10],
                "c:2427 — curline.words = chwords");
        }

        // Case 2: matching histnum but NOT active → no flush.
        histactive.store(0, Ordering::SeqCst);                // c:2424 HA_ACTIVE off
        *curline.lock().unwrap() = None;
        checkcurline(&he);
        assert!(curline.lock().unwrap().is_none(),
            "c:2424 — HA_ACTIVE cleared, no flush");

        // Case 3: active but mismatched histnum → no flush.
        histactive.store(HA_ACTIVE, Ordering::SeqCst);
        let he2 = histent { histnum: 99, ..histent {
            node: crate::ported::zsh_h::hashnode {
                next: None, nam: String::new(), flags: 0,
            },
            up: None, down: None, zle_text: None,
            stim: 0, ftim: 0, words: vec![], nwords: 0, histnum: 0,
        }};
        checkcurline(&he2);
        assert!(curline.lock().unwrap().is_none(),
            "c:2424 — histnum mismatch, no flush");

        // Restore.
        curhist.store(saved_curhist, Ordering::SeqCst);
        histactive.store(saved_active, Ordering::SeqCst);
        *chline.lock().unwrap() = saved_chline;
        chwordpos.store(saved_chwordpos, Ordering::SeqCst);
        *chwords.lock().unwrap() = saved_chwords;
        *curline.lock().unwrap() = saved_curline;
    }
}
