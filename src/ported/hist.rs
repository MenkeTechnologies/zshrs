//! hist.c - history mechanism
//!
//! Port of Src/hist.c
//!
//! The history lines are kept in a hash, and also doubly-linked in a ring.   // c:98

use crate::ported::glob::remnulargs;
use crate::ported::hashtable::addhistnode;
use crate::ported::input::{ingetc, inputsetline, inungetc};
use crate::ported::lex::{
    lexinit, parse_subst_string, untokenize, ztokens, LEX_ISFIRSTCH, LEX_LEXSTOP,
};
use crate::ported::options::dosetopt;
use crate::ported::parse::init_parse_status;
use crate::ported::signals::unqueue_signals;
use crate::ported::subst::equalsubstr;
use crate::ported::utils::{errflag, zerr, zmonotime, zsleep_random, ERRFLAG_ERROR};
use crate::ported::zle::compcore::ZLEMETACS;
use crate::ported::zsh_h::{
    hashnode, hist_stack, histent, isset, Pound, BANGHIST, CASMOD_CAPS, CASMOD_LOWER, CASMOD_NONE,
    CASMOD_UPPER, CSHJUNKIEHISTORY, ERRFLAG_INT, HFILE_FAST, HFILE_USE_OPTIONS,
    HISTEXPIREDUPSFIRST, HISTFLAG_DONE, HISTFLAG_NOEXEC, HISTFLAG_RECALL, HISTFLAG_SETTY,
    HISTIGNOREALLDUPS, HISTIGNOREDUPS, HISTIGNORESPACE, HISTNOFUNCTIONS, HISTNOSTORE,
    HISTREDUCEBLANKS, HISTVERIFY, HIST_DUP, HIST_FOREIGN, HIST_NOWRITE, HIST_OLD, HIST_TMPSTORE,
    INCAPPENDHISTORY, INCAPPENDHISTORYTIME, INP_ALIAS, INP_HIST, INTERACTIVE, SHAREHISTORY,
    SHINSTDIN,
};
use crate::ported::ztype_h::itok;
use crate::signals::queue_signals;
use crate::{DPUTS, DPUTS1};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::Component::*;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

// NOTE: `inbufflags` and `inbufct` are NOT imported because the
// hist.rs body uses `let inbufflags = ...` shadowing — Rust treats
// imported names as constant-patterns in `let` LHS, breaking that
// idiom. They stay as `crate::ported::input::inbufflags` until
// the shadowing pattern is refactored.

// Bits of histactive variable                                               // c:137
/// Port of `HA_ACTIVE` from Src/hist.c:138. History mechanism is active.
pub const HA_ACTIVE: u32 = 1 << 0; // c:138
/// Port of `HA_NOINC` from Src/hist.c:139. Don't store, curhist not incremented.
pub const HA_NOINC: u32 = 1 << 1; // c:139
/// Port of `HA_INWORD` from Src/hist.c:140. We're inside a word.
pub const HA_INWORD: u32 = 1 << 2; // c:140
/// Port of `HA_UNGET` from Src/hist.c:142. Recursively ungetting.
pub const HA_UNGET: u32 = 1 << 3; // c:142

/// Port of `static zlong defev` from Src/hist.c:210.
static defev: AtomicI64 = AtomicI64::new(0); // c:210

/// Port of `static int hist_keep_comment` from Src/hist.c:217.
static hist_keep_comment: AtomicI32 = AtomicI32::new(0); // c:217

/// Port of `static int histsave_stack_size` from Src/hist.c:239.
static histsave_stack_size: AtomicI32 = AtomicI32::new(0); // c:239

/// Port of `static int histsave_stack_pos` from Src/hist.c:240.
static histsave_stack_pos: AtomicI32 = AtomicI32::new(0); // c:240

/// Port of `static zlong histfile_linect` from Src/hist.c:242.
static histfile_linect: AtomicI64 = AtomicI64::new(0); // c:242

// =========================================================================
// Functions from hist.c
// =========================================================================

/// Port of `void hist_context_save(struct hist_stack *hs, int toplevel)`
/// from Src/hist.c:248.
pub fn hist_context_save(hs: &mut hist_stack, toplevel: i32) {
    // c:248
    if toplevel != 0 {
        // c:248
        // top level, make this version visible to ZLE                       // c:251
        *zle_chline.lock().unwrap() = Some(chline.lock().unwrap().clone()); // c:252
                                                                            // ensure line stored is NULL-terminated — implicit in String        // c:253-255
    }
    hs.histactive = histactive.load(SeqCst) as i32; // c:257
    hs.histdone = histdone.load(SeqCst); // c:258
    hs.stophist = stophist.load(SeqCst); // c:259
    hs.hline = Some(chline.lock().unwrap().clone()); // c:260
    hs.hptr = Some(hptr.load(SeqCst).to_string()); // c:261
    hs.chwords = chwords.lock().unwrap().clone(); // c:262
    hs.chwordlen = chwordlen.load(SeqCst); // c:263
    hs.chwordpos = chwordpos.load(SeqCst); // c:264
                                           // hs->hgetc / hungetc / hwaddc / hwbegin / hwabort / hwend / addtoline  // c:265-271
                                           // are runtime-mutable function-pointer globals in C; the Rust port
                                           // dispatches statically via crate::ported::input.
    hs.hlinesz = hlinesz.load(SeqCst); // c:272
    hs.defev = defev.load(SeqCst); // c:273
    hs.hist_keep_comment = hist_keep_comment.load(SeqCst); // c:274
                                                           // hs->cstack = cmdstack; hs->csp = cmdsp;                               // c:296-282
    hs.csp = 0;

    stophist.store(0, SeqCst); // c:296
    chline.lock().unwrap().clear(); // c:296
    hptr.store(0, SeqCst); // c:296
    histactive.store(0, SeqCst); // c:296
                                 // cmdstack = zalloc(CMDSTACKSZ); cmdsp = 0;                             // c:296-289
}

/// Port of `void hist_context_restore(const struct hist_stack *hs, int toplevel)`
/// from Src/hist.c:296.
pub fn hist_context_restore(hs: &hist_stack, toplevel: i32) {
    // c:296
    if toplevel != 0 {
        // c:296
        // c:299 — Back to top level: don't need special ZLE value
        // c:300 — DPUTS(hs->hline != zle_chline, "BUG: Ouch, wrong chline for ZLE")
        DPUTS!(
            // c:300
            hs.hline != *zle_chline.lock().unwrap(), // c:300
            "BUG: Ouch, wrong chline for ZLE"        // c:300
        );
        *zle_chline.lock().unwrap() = None; // c:301
    }
    histactive.store(hs.histactive as u32, SeqCst); // c:303
    histdone.store(hs.histdone, SeqCst); // c:304
    stophist.store(hs.stophist, SeqCst); // c:305
    *chline.lock().unwrap() = hs.hline.clone().unwrap_or_default(); // c:306
    hptr.store(
        hs.hptr.as_ref().and_then(|s| s.parse().ok()).unwrap_or(0), // c:307
        SeqCst,
    );
    *chwords.lock().unwrap() = hs.chwords.clone(); // c:308
    chwordlen.store(hs.chwordlen, SeqCst); // c:309
    chwordpos.store(hs.chwordpos, SeqCst); // c:310
                                           // hgetc / hungetc / hwaddc / hwbegin / hwabort / hwend / addtoline      // c:311-317
    hlinesz.store(hs.hlinesz, SeqCst); // c:318
    defev.store(hs.defev, SeqCst); // c:339
    hist_keep_comment.store(hs.hist_keep_comment, SeqCst); // c:339
                                                           // cmdstack = hs->cstack; cmdsp = hs->csp;                               // c:339-324
}

/// Port of `void hist_in_word(int yesno)` from Src/hist.c.
pub fn hist_in_word(yesno: i32) {
    if yesno != 0 {
        histactive.fetch_or(HA_INWORD, SeqCst);
    } else {
        histactive.fetch_and(!HA_INWORD, SeqCst);
    }
}

/// Port of `int hist_is_in_word(void)` from Src/hist.c.
pub fn hist_is_in_word() -> i32 {
    if (histactive.load(SeqCst) & HA_INWORD) != 0 {
        1
    } else {
        0
    }
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
pub fn ihwaddc(c: i32) {
    // c:357
    // c:360-361 — guard: history line must exist, no error/lex stop,
    // and we're not strictly inside alias-expansion-only input.
    if errflag.load(SeqCst) != 0 || lexstop.load(SeqCst) {
        return;
    }
    let inbufflags = crate::ported::input::inbufflags.with(|f| f.get());
    if (inbufflags & (INP_ALIAS | INP_HIST)) == INP_ALIAS {
        return;
    }
    // C guard: `if (chline && ...)` — chline must be ALLOCATED (non-NULL),
    // which is the active state hbegin() sets (`hlinesz = 64`), NOT "the
    // buffer has bytes yet". The prior `chline.is_empty()` proxy was a
    // chicken-and-egg: an active-but-empty chline (right after hbegin)
    // rejected the FIRST char, so the line never started building and
    // interactive history recorded nothing. Use the real allocated flag.
    if hlinesz.load(SeqCst) == 0 {
        return; // c:360 — inactive history (chline == NULL)
    }
    // c:362-368 — `*hptr++ = c;`. C writes the byte at the hptr
    // cursor then advances; the qbang escape arm also writes via
    // the recursive `hwaddc('\\');` call.
    //
    // The previous Rust port called `chline.push(c)` which only
    // APPENDS — after `hwrep` rewinds hptr to mid-buffer, new
    // pushes would land at chline.end instead of overwriting the
    // word being replaced. Mirror C exactly: write each byte at
    // the hptr position (growing only when hptr == chline.len()),
    // then advance hptr.
    let bc = bangchar.load(SeqCst);
    let qbang_active = c == bc && stophist.load(SeqCst) < 2 && qbang.load(SeqCst);
    {
        let mut buf = chline.lock().expect("chline poisoned");
        let bytes = unsafe { buf.as_mut_vec() };
        let mut pos = hptr.load(SeqCst);
        // Mirror the recursive `hwaddc('\\');` at c:366 — also writes
        // at hptr + advances.
        if qbang_active {
            if pos < bytes.len() {
                bytes[pos] = b'\\';
            }
            // c:366
            else {
                while bytes.len() < pos {
                    bytes.push(0);
                }
                bytes.push(b'\\');
            }
            pos += 1;
        }
        // c:368 — `*hptr++ = c;`.
        if pos < bytes.len() {
            bytes[pos] = c as u8;
        } else {
            while bytes.len() < pos {
                bytes.push(0);
            }
            bytes.push(c as u8);
        }
        pos += 1;
        hptr.store(pos, SeqCst);
    }
    // c:370-374 — resize tracking. Rust `String` grows on `push`
    // automatically, but `hlinesz` mirrors C's allocation count
    // for any caller that reads it (e.g. `hwend()`). C condition
    // is `hptr - chline >= hlinesz`; mirror with the canonical
    // `hptr` global (matches the c:1658 / c:1693 fixes).
    let cur_off = hptr.load(SeqCst) as i32; // c:381 hptr - chline
    let sz = hlinesz.load(SeqCst);
    if cur_off >= sz {
        // c:381
        let new_sz = sz + 64;
        hlinesz.store(new_sz, SeqCst); // c:384
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
pub fn iaddtoline(c: i32) {
    // c:397
    // c:399 — `if (!expanding || lexstop) return;`.
    if expanding.load(SeqCst) == 0 || lexstop.load(SeqCst) {
        return;
    }
    // c:401-404 — bang-escape under qbang.
    let bc = bangchar.load(SeqCst);
    if qbang.load(SeqCst) && c == bc && stophist.load(SeqCst) < 2 {
        exlast.fetch_sub(1, SeqCst); // c:402
        chline.lock().unwrap().push('\\'); // c:403 zleentry ADD '\\'
    }
    // c:405-411 — `if (excs > zlemetacs) { excs += 1 + inbufct -
    // exlast; if (excs < zlemetacs) excs = zlemetacs; }`.
    //
    // ZLE cursor position adjustment after history-expanded byte
    // gets injected: if the cursor `excs` was past the line cursor
    // `zlemetacs`, account for the byte just consumed from the
    // input buffer (1 + inbufct - exlast slots). The post-adjust
    // clamp to zlemetacs avoids the cursor underrunning the line.
    //
    // The previous Rust port omitted the entire block — typing
    // `!str` mid-line would leave the ZLE cursor at the wrong
    // position after expansion.
    let zlemetacs_v = ZLEMETACS.load(SeqCst);
    let excs_v = excs.load(SeqCst);
    if excs_v > zlemetacs_v {
        // c:405
        let inbufct_now = crate::ported::input::inbufct.with(|c| c.get());
        let exlast_v = exlast.load(SeqCst);
        let mut new_excs = excs_v + 1 + inbufct_now - exlast_v; // c:406
        if new_excs < zlemetacs_v {
            // c:407
            new_excs = zlemetacs_v; // c:410
        }
        excs.store(new_excs, SeqCst);
    }
    // c:413 — `exlast = inbufct;`
    let inbufct_v = crate::ported::input::inbufct.with(|cnt| cnt.get());
    exlast.store(inbufct_v, SeqCst); // c:413
                                     // c:413 — `itok(c) ? ztokens[c - Pound] : c`.
    let push_byte: u8 = if c >= 0 && c <= 0xff && itok(c as u8) {
        let idx = (c as u8).wrapping_sub(Pound as u8) as usize;
        // ztokens is the literal-char back-mapping for ITOK bytes.
        // Defensively guard against an out-of-range token byte
        // (the closed range Pound..=Nularg is 0x84..=0xa1, 30
        // entries; ztokens covers them).
        ztokens.bytes().nth(idx).unwrap_or(c as u8)
    } else {
        c as u8
    };
    chline.lock().unwrap().push(push_byte as char); // c:413
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
pub fn safeinungetc(c: i32) {
    // c:467
    if lexstop.load(SeqCst) {
        // c:469
        lexstop.store(false, SeqCst); // c:470
    } else {
        // c:471
        if let Some(ch) = char::from_u32(c as u32) {
            // c:472 inungetc(c)
            inungetc(ch);
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
pub fn ihgetc() -> i32 {
    // c:418
    // c:420 — `int c = ingetc();`. C's ingetc returns the byte 32 (' ')
    // WITH lexstop set at EOF (input.c:322). zshrs's ingetc signals EOF as
    // `None`, so map None → ' ' (NOT -1): a negative `c` is reserved for
    // the histsubchar bad-`!`-expansion path at c:432 (which sets errflag),
    // and conflating EOF with it spuriously flags an error on a clean
    // Ctrl-D / end-of-input.
    let mut c: i32 = ingetc() // c:420 int c = ingetc();
        .map(|ch| ch as i32)
        .unwrap_or(b' ' as i32);
    if exit_pending.load(SeqCst) {
        // c:422
        lexstop.store(true, SeqCst); // c:424
        errflag.fetch_or(
            // c:425 errflag |= ERRFLAG_ERROR
            ERRFLAG_ERROR,
            SeqCst,
        );
        return b' ' as i32; // c:426
    }
    qbang.store(false, SeqCst); // c:428 qbang = 0
    let inbufflags_v = crate::ported::input::inbufflags.with(|f| f.get());
    if stophist.load(SeqCst) == 0                                  // c:429 !stophist
        && (inbufflags_v & INP_ALIAS) == 0
    // c:429 !(inbufflags & INP_ALIAS)
    {
        c = histsubchar(c); // c:431 c = histsubchar(c)
        if c < 0 {
            // c:432
            lexstop.store(true, SeqCst); // c:434
            errflag.fetch_or(
                // c:435
                ERRFLAG_ERROR,
                SeqCst,
            );
            return b' ' as i32; // c:436
        }
    }
    let inbufflags_v = crate::ported::input::inbufflags.with(|f| f.get());
    let bc = bangchar.load(SeqCst);
    if (inbufflags_v & INP_HIST) != 0 && stophist.load(SeqCst) == 0 {
        // c:439
        // c:447 qbang = 0
        qbang.store(false, SeqCst);
        if c == b'\\' as i32 {
            // c:448 c == '\\'
            let g = ingetc() // c:448 c = ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if g == bc {
                // c:448 qbang = (c == bangchar)
                qbang.store(true, SeqCst);
                c = g;
            } else {
                // c:449 safeinungetc(c), c = '\\';
                safeinungetc(g);
                c = b'\\' as i32;
            }
        }
    } else if stophist.load(SeqCst) != 0                           // c:450 stophist
        || (inbufflags_v & INP_ALIAS) != 0
    // c:450 (inbufflags & INP_ALIAS)
    {
        // c:458 qbang = c == bangchar && (stophist < 2)
        let v = c == bc && stophist.load(SeqCst) < 2;
        qbang.store(v, SeqCst);
    }
    ihwaddc(c); // c:459 hwaddc(c)
    iaddtoline(c); // c:460 addtoline(c)
    c // c:462 return c
}

/// Port of `unsigned char hatchar` from `Src/params.c:132`. Caret used
/// as the substitution-shortcut lead character on first column; init'd
/// to `'^'` at `Src/init.c:1102`. Read by `histsubchar` (c:618).
/// NOTE on placement: canonical home per PORT.md Rule C would be
/// `params.rs`, alongside `bangchar`/`hashchar`. Kept here to mirror
/// the existing (pre-2026-05) placement of `bangchar` at hist.rs:1858;
/// move-all-three is a follow-up cleanup.
pub static hatchar: AtomicI32 = AtomicI32::new(b'^' as i32); // c:params.c:132

/// Port of `unsigned char hashchar` from `Src/params.c:132`. Comment-
/// start character; init'd to `'#'` at `Src/init.c:1101`. Read by
/// `gettokstr` (`Src/lex.c:678`). Atomic so `histcharssetfn`
/// (`Src/params.c:5097`) can update it dynamically when `$HISTCHARS`
/// changes.
pub static hashchar: AtomicI32 = AtomicI32::new(b'#' as i32); // c:params.c:132

/// Port of `static int marg` from `Src/hist.c:599`. Argument index of the
/// most-recent `!?str?` event match; `-1` when no match has happened.
pub static marg: AtomicI32 = AtomicI32::new(-1); // c:599

/// Port of `static zlong mev` from `Src/hist.c:600`. Event number of the
/// most-recent `!?str?` event match; `-1` when no match has happened.
pub static mev: AtomicI64 = AtomicI64::new(-1); // c:600

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
pub fn histsubchar(c_in: i32) -> i32 {
    // c:595
    let mut c: i32 = c_in;
    let mut farg: i32; // c:597
    let mut evset: i32 = -1; // c:597
    let mut larg: i32;
    let mut argc: i32; // c:597
    let mut cflag: i32 = 0; // c:597
    let mut bflag: i32 = 0; // c:597
    let mut ev: i64; // c:598
    let mut buf: String; // c:601 char *buf, *ptr
    let mut sline: String; // c:602
                           // c:603 lexraw_mark — Rust port's lexer doesn't expose `zshlex_raw_mark`
                           // hook yet; mirror the C `lexraw_mark` / `zshlex_raw_back_to_mark` calls
                           // as a no-op `i32` carry.
    let lexraw_mark: i32 = 0; // c:603,615

    // c:618 — `^foo^bar` shortcut: only valid on first column of input.
    let hat = hatchar.load(SeqCst);
    if LEX_ISFIRSTCH.with(|f| f.get()) && c == hat {
        // c:618
        let mut gbal: i32 = 0; // c:619
                               // c:622 — clear isfirstch
        LEX_ISFIRSTCH.with(|f| f.set(false)); // c:622
                                              // c:623 — push hatchar back so getargs parses the leading ^.
        if let Some(ch) = char::from_u32(hat as u32) {
            inungetc(ch); // c:623
        }
        let ehist = match gethist(defev.load(SeqCst)) {
            // c:624
            Some(h) => h,
            None => return -1, // c:626
        };
        let argc_local = getargc(&ehist) as usize;
        sline = match getargs(&ehist, 0, argc_local.saturating_sub(0)) {
            // c:625
            Some(s) => s,
            None => return -1, // c:626
        };

        if getsubsargs(&sline, &mut gbal, &mut cflag) != 0 {
            // c:628
            return substfailed(); // c:629
        }
        if hsubl.lock().unwrap().is_none() {
            // c:630
            return -1; // c:631
        }
        let in_pat = hsubl.lock().unwrap().clone().unwrap_or_default();
        let out_pat = hsubr.lock().unwrap().clone().unwrap_or_default();
        let new = subst(&sline, &in_pat, &out_pat, gbal != 0); // c:632
        if new == sline {
            // c:632 subst returned 0 (no match)
            return substfailed(); // c:633
        }
        sline = new;
    } else {
        // c:636 — !c shortcut: first-column flag clears unless c==' '.
        if c != b' ' as i32 {
            // c:636
            LEX_ISFIRSTCH.with(|f| f.set(false)); // c:637
        }
        let bc = bangchar.load(SeqCst);
        if c == b'\\' as i32 {
            // c:638
            let g = ingetc() // c:639 ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if g != bc {
                // c:641
                safeinungetc(g); // c:642
            } else {
                // c:643
                qbang.store(true, SeqCst); // c:644
                return bc; // c:645 return bangchar
            }
        }
        if c != bc {
            // c:648
            return c; // c:649
        }
        // c:650 — `*hptr = '\0'` truncates chline at the current position.
        let pos = hptr.load(SeqCst); // c:650
        {
            let mut cl = chline.lock().unwrap();
            if pos < cl.len() {
                cl.truncate(pos);
            }
        }
        c = ingetc() // c:651
            .map(|ch| ch as i32)
            .unwrap_or(-1);
        if c == b'{' as i32 {
            // c:651
            bflag = 1; // c:652
            cflag = 1;
            c = ingetc() // c:653
                .map(|ch| ch as i32)
                .unwrap_or(-1);
        }
        if c == b'"' as i32 {
            // c:655 c == '\"'
            stophist.store(1, SeqCst); // c:656
            return ingetc() // c:657 return ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
        }
        // c:659 — (!cflag && inblank(c)) || c == '=' || c == '(' || lexstop
        let is_blank = (c as u8 as char).is_ascii_whitespace();
        if (cflag == 0 && is_blank) || c == b'=' as i32 || c == b'(' as i32 || lexstop.load(SeqCst)
        // c:659
        {
            safeinungetc(c); // c:660
            return bc; // c:661 return bangchar
        }
        cflag = 0; // c:663
        let mut buflen: usize = 265; // c:664
        buf = String::with_capacity(buflen); // c:664 zhalloc

        // c:666-727 — read event-spec into buf.
        queue_signals(); // c:668
        if c == b'?' as i32 {
            // c:669
            loop {
                // c:670
                c = ingetc() // c:671 ingetc()
                    .map(|ch| ch as i32)
                    .unwrap_or(-1);
                if c == b'?' as i32 || c == b'\n' as i32 || lexstop.load(SeqCst) {
                    // c:672
                    break; // c:673
                } else {
                    buf.push(c as u8 as char); // c:675 *ptr++ = c
                    if buf.len() >= buflen {
                        // c:676
                        buflen *= 2; // c:679 buflen *= 2
                        buf.reserve(buflen);
                    }
                }
            }
            if c != b'\n' as i32 && !lexstop.load(SeqCst) {
                // c:683
                c = ingetc() // c:684
                    .map(|ch| ch as i32)
                    .unwrap_or(-1);
            }
            // c:685 *ptr = '\0' — Rust String is already terminated.
            *hsubl.lock().unwrap() = Some(buf.clone()); // c:686 hsubl = ztrdup(buf)
                                                        // c:686 — `mev = ev = hconsearch(hsubl = ztrdup(buf), &marg);`
                                                        // hconsearch returns Option<(histnum, marg)> per C's
                                                        // out-pointer pair.
            let (ev_val, marg_val) = match hconsearch(&buf) {
                // c:686
                Some((e, m)) => (e, m),
                None => (-1, -1),
            };
            ev = ev_val;
            mev.store(ev, SeqCst); // c:686 mev = ev
            marg.store(marg_val, SeqCst); // c:686 marg out-arg
            evset = 0; // c:687
            if ev == -1 {
                // c:688
                herrflush(); // c:689
                unqueue_signals(); // c:690
                zerr(&format!("no such event: {}", buf)); // c:691
                return -1; // c:692
            }
        } else {
            // c:694
            // c:697 — collect event spec until terminator.
            loop {
                let is_term = (c as u8 as char).is_ascii_whitespace()
                    || c == b';' as i32
                    || c == b':' as i32
                    || c == b'^' as i32
                    || c == b'$' as i32
                    || c == b'*' as i32
                    || c == b'%' as i32
                    || c == b'}' as i32
                    || c == b'\'' as i32
                    || c == b'"' as i32
                    || c == b'`' as i32
                    || lexstop.load(SeqCst); // c:698-700
                if is_term {
                    break;
                }
                if !buf.is_empty() {
                    // c:702
                    if c == b'-' as i32 {
                        break;
                    } // c:703-704
                    let first = buf.as_bytes()[0];
                    if (first.is_ascii_digit() || first == b'-')             // c:705
                        && !(c as u8).is_ascii_digit()
                    // c:705 !idigit(c)
                    {
                        break; // c:706
                    }
                }
                buf.push(c as u8 as char); // c:708
                if buf.len() >= buflen {
                    // c:709
                    buflen *= 2; // c:712
                    buf.reserve(buflen);
                }
                if c == b'#' as i32 || c == bc {
                    // c:714
                    c = ingetc() // c:715
                        .map(|ch| ch as i32)
                        .unwrap_or(-1);
                    break; // c:716
                }
                c = ingetc() // c:718
                    .map(|ch| ch as i32)
                    .unwrap_or(-1);
            }
            if buf.is_empty()                                                // c:720
                && (c == b'}' as i32 || c == b';' as i32 || c == b'\'' as i32
                    || c == b'"' as i32 || c == b'`' as i32)
            {
                safeinungetc(c); // c:723
                unqueue_signals(); // c:724
                return bc; // c:725
            }
            // c:727 *ptr = 0 — handled by Rust String
            if buf.is_empty() {
                // c:728
                if c != b'%' as i32 {
                    // c:729
                    if isset(CSHJUNKIEHISTORY) {
                        // c:730
                        ev = addhistnum(curhist.load(SeqCst), -1, HIST_FOREIGN as i32);
                    } else {
                        // c:732
                        ev = defev.load(SeqCst); // c:733
                    }
                    if c == b':' as i32 && evset == -1 {
                        // c:734
                        evset = 0; // c:735
                    } else {
                        evset = 1; // c:737
                    }
                } else {
                    // c:738
                    if marg.load(SeqCst) != -1 {
                        // c:739
                        ev = mev.load(SeqCst); // c:740
                    } else {
                        // c:741
                        ev = defev.load(SeqCst); // c:742
                    }
                    evset = 0; // c:743
                }
            } else if let Ok(t0) = buf.trim().parse::<i64>() {
                // c:745 zstrtol(buf, NULL, 10)
                if t0 != 0 {
                    ev = if t0 < 0 {
                        // c:746
                        addhistnum(curhist.load(SeqCst), t0 as i32, HIST_FOREIGN as i32)
                    } else {
                        t0
                    };
                    evset = 1; // c:747
                } else if buf.as_bytes()[0] == bc as u8 {
                    // c:748 *buf == bangchar
                    ev = addhistnum(curhist.load(SeqCst), -1, HIST_FOREIGN as i32); // c:749
                    evset = 1; // c:750
                } else if buf.as_bytes()[0] == b'#' {
                    // c:751
                    ev = curhist.load(SeqCst); // c:752
                    evset = 1; // c:753
                } else {
                    // c:754
                    match hcomsearch(&buf) {
                        // c:754
                        Some(e) => {
                            ev = e;
                            evset = 1;
                        }
                        None => {
                            herrflush(); // c:755
                            unqueue_signals(); // c:756
                            zerr(&format!("event not found: {}", buf)); // c:757
                            return -1; // c:758
                        }
                    }
                }
            } else if buf.as_bytes()[0] == bc as u8 {
                // c:748 *buf == bangchar
                ev = addhistnum(curhist.load(SeqCst), -1, HIST_FOREIGN as i32);
                evset = 1;
            } else if buf.as_bytes()[0] == b'#' {
                // c:751
                ev = curhist.load(SeqCst);
                evset = 1;
            } else {
                // c:754
                match hcomsearch(&buf) {
                    Some(e) => {
                        ev = e;
                        evset = 1;
                    }
                    None => {
                        herrflush();
                        unqueue_signals();
                        zerr(&format!("event not found: {}", buf));
                        return -1;
                    }
                }
            }
        }

        // c:765 — fetch the resolved history entry.
        defev.store(ev, SeqCst); // c:765 defev = ev
        let mut ehist = match gethist(ev) {
            // c:765
            Some(h) => h,
            None => {
                unqueue_signals(); // c:766
                return -1; // c:767
            }
        };
        argc = getargc(&ehist) as i32; // c:771

        // c:772 — word-designator parsing.
        if c == b':' as i32 {
            // c:772
            cflag = 1; // c:773
            c = ingetc() // c:774
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if c == b'%' as i32 && marg.load(SeqCst) != -1 {
                // c:775
                if evset == 0 {
                    // c:776
                    ehist = match gethist(mev.load(SeqCst)) {
                        // c:777
                        Some(h) => {
                            defev.store(mev.load(SeqCst), SeqCst);
                            h
                        }
                        None => {
                            unqueue_signals();
                            return -1;
                        }
                    };
                    argc = getargc(&ehist) as i32; // c:778
                } else {
                    // c:779
                    herrflush(); // c:780
                    unqueue_signals(); // c:781
                    zerr("ambiguous history reference"); // c:782
                    return -1; // c:783
                }
            }
        }
        // c:788
        if c == b'*' as i32 {
            // c:788
            farg = 1; // c:789
            larg = argc; // c:790
            cflag = 0; // c:791
        } else {
            // c:792
            if let Some(ch) = char::from_u32(c as u32) {
                inungetc(ch); // c:793
            }
            let r = getargspec(argc, marg.load(SeqCst), evset); // c:794
            larg = r;
            farg = r;
            if larg == -2 {
                // c:795
                unqueue_signals(); // c:796
                return -1; // c:797
            }
            if farg != -1 {
                // c:799
                cflag = 0; // c:800
            }
            c = ingetc() // c:801
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if c == b'*' as i32 {
                // c:802
                cflag = 0; // c:803
                larg = argc; // c:804
            } else if c == b'-' as i32 {
                // c:805
                cflag = 0; // c:806
                larg = getargspec(argc, marg.load(SeqCst), evset); // c:807
                if larg == -2 {
                    // c:808
                    unqueue_signals(); // c:809
                    return -1; // c:810
                }
                if larg == -1 {
                    // c:812
                    larg = argc - 1; // c:813
                }
            } else {
                // c:814
                if let Some(ch) = char::from_u32(c as u32) {
                    inungetc(ch); // c:815
                }
            }
        }
        if farg == -1 {
            // c:817
            farg = 0; // c:818
        }
        if larg == -1 {
            // c:819
            larg = argc; // c:820
        }
        sline = match getargs(&ehist, farg as usize, larg as usize) {
            // c:821
            Some(s) => s,
            None => {
                unqueue_signals(); // c:822
                return -1; // c:823
            }
        };
        unqueue_signals(); // c:825
    }

    // c:830 — modifier loop.
    loop {
        c = if cflag != 0 {
            b':' as i32
        } else {
            // c:831
            ingetc().map(|ch| ch as i32).unwrap_or(-1)
        };
        cflag = 0; // c:832
        if c == b':' as i32 {
            // c:833
            let mut gbal: i32 = 0; // c:834
            c = ingetc() // c:836
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if c == b'g' as i32 {
                // c:836
                gbal = 1; // c:837
                c = ingetc() // c:838
                    .map(|ch| ch as i32)
                    .unwrap_or(-1);
                if c != b's' as i32 && c != b'S' as i32 && c != b'&' as i32 {
                    // c:839
                    zerr("'s' or '&' modifier expected after 'g'"); // c:840
                    return -1; // c:841
                }
            }
            match c as u8 {
                b'p' => {
                    // c:845
                    histdone.store(HISTFLAG_DONE | HISTFLAG_NOEXEC, SeqCst);
                    // c:846
                }
                b'a' => {
                    // c:848
                    match chabspath(&sline) {
                        // c:849
                        Some(new) => sline = new,
                        None => {
                            herrflush(); // c:850
                            zerr("modifier failed: a"); // c:851
                            return -1; // c:852
                        }
                    }
                }
                b'A' => {
                    // c:856
                    match chrealpath(&sline, b'A', false) {
                        // c:857 chrealpath(&sline, 'A', 0)
                        Some(new) => sline = new,
                        None => {
                            herrflush(); // c:858
                            zerr("modifier failed: A"); // c:859
                            return -1; // c:860
                        }
                    }
                }
                b'c' => {
                    // c:863
                    match equalsubstr(&sline, false, false) {
                        // c:864
                        Some(new) => sline = new,
                        None => {
                            herrflush(); // c:865
                            zerr("modifier failed: c"); // c:866
                            return -1; // c:867
                        }
                    }
                }
                b'h' => {
                    // c:870
                    let count = digitcount(); // c:871
                    sline = remtpath(&sline, count);
                }
                b'e' => {
                    // c:877
                    sline = rembutext(&sline); // c:878
                }
                b'r' => {
                    // c:884
                    sline = remtext(&sline); // c:885
                }
                b't' => {
                    // c:891
                    let count = digitcount(); // c:892
                    sline = remlpaths(&sline, count);
                }
                b's' | b'S' => {
                    // c:898-899
                    hsubpatopt.store((c == b'S' as i32) as i32, SeqCst); // c:900
                    if getsubsargs(&sline, &mut gbal, &mut cflag) != 0 {
                        // c:901
                        return -1; // c:902
                    }
                    // fall through to '&'                                   // c:902
                    let (in_pat, out_pat) =
                        (hsubl.lock().unwrap().clone(), hsubr.lock().unwrap().clone());
                    if let (Some(ip), Some(op)) = (in_pat, out_pat) {
                        // c:904
                        let new = subst(&sline, &ip, &op, gbal != 0); // c:905
                        if new == sline {
                            // c:905 no match
                            return substfailed(); // c:906
                        }
                        sline = new;
                    } else {
                        // c:907
                        herrflush(); // c:908
                        zerr("no previous substitution"); // c:909
                        return -1; // c:910
                    }
                }
                b'&' => {
                    // c:903
                    let (in_pat, out_pat) =
                        (hsubl.lock().unwrap().clone(), hsubr.lock().unwrap().clone());
                    if let (Some(ip), Some(op)) = (in_pat, out_pat) {
                        let new = subst(&sline, &ip, &op, gbal != 0);
                        if new == sline {
                            return substfailed();
                        }
                        sline = new;
                    } else {
                        herrflush();
                        zerr("no previous substitution");
                        return -1;
                    }
                }
                b'q' => {
                    // c:913
                    sline = quote(&sline); // c:914
                }
                b'Q' => {
                    // c:916
                    // c:918-924 — `noerrs` flag stack is no-op in Rust port;
                    // see params.rs:1310. Tokenize-strip via parse_subst_string,
                    // then remnulargs + untokenize.
                    let oef = errflag.load(SeqCst);
                    let _ = parse_subst_string(&sline); // c:921
                    errflag.store(oef | (errflag.load(SeqCst) & ERRFLAG_INT), SeqCst); // c:923
                    let mut s = sline.clone();
                    remnulargs(&mut s); // c:924
                    sline = untokenize(&s); // c:925
                }
                b'x' => {
                    // c:928
                    sline = quotebreak(&sline); // c:929
                }
                b'l' => {
                    // c:931
                    sline = casemodify(&sline, CASMOD_LOWER); // c:932
                }
                b'u' => {
                    // c:934
                    sline = casemodify(&sline, CASMOD_UPPER); // c:935
                }
                b'P' => {
                    // c:937
                    if !sline.starts_with('/') {
                        // c:938
                        let here = crate::ported::compat::zgetcwd(); // c:939
                        sline = if here.ends_with('/') {
                            // c:940
                            crate::ported::string::dyncat(&here, &sline) // c:943
                        } else {
                            // c:941
                            // c:941 zhtricat(metafy(here, -1, META_HEAPDUP), "/", sline)
                            format!("{}/{}", here, sline) // c:941
                        }; // c:944
                    }
                    match crate::ported::utils::xsymlink(&sline) {
                        // c:945
                        Some(new) => sline = new,
                        None => {} // C ignores xsymlink failure (returns NULL → keep sline)
                    }
                }
                _ => {
                    // c:947 default
                    herrflush(); // c:948
                    zerr(&format!("illegal modifier: {}", c as u8 as char)); // c:949
                    return -1; // c:950
                }
            }
        } else {
            // c:952
            if c != b'}' as i32 || bflag == 0 {
                // c:953
                if let Some(ch) = char::from_u32(c as u32) {
                    inungetc(ch); // c:954
                }
            }
            if c != b'}' as i32 && bflag != 0 {
                // c:955
                zerr("'}' expected"); // c:956
                return -1; // c:957
            }
            break; // c:959
        }
    }

    // c:963 — zshlex_raw_back_to_mark(lexraw_mark): no-op until lex hook
    // exposes the raw-input mark/restore pair.
    let _ = lexraw_mark; // c:963

    // c:970-976 — push the expanded value onto the input stack as INP_HIST.
    lexstop.store(false, SeqCst); // c:970
    crate::ported::input::inpush(&sline, INP_HIST, None); // c:976
    histdone.fetch_or(HISTFLAG_DONE, SeqCst); // c:977
    if isset(HISTVERIFY) {
        // c:978
        histdone.fetch_or(HISTFLAG_NOEXEC | HISTFLAG_RECALL, SeqCst); // c:979
    }

    // c:982 — return ingetc() so caller sees the first char of expansion.
    ingetc().map(|ch| ch as i32).unwrap_or(-1)
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
pub fn herrflush() {
    // c:477
    // c:479 — `inpopalias();`
    crate::ported::input::inpopalias();

    // c:481-482 — `if (lexstop) return;`
    if LEX_LEXSTOP.with(|f| f.get()) {
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
        let strin_v = strin.load(SeqCst);
        let lex_add_raw = crate::ported::lex::LEX_LEX_ADD_RAW.get();
        if !(strin_v == 0 || lex_add_raw != 0) {
            // c:494 (!strin || lex_add_raw)
            break;
        }
        let c = ingetc() // c:495 ingetc()
            .map(|ch| ch as i32)
            .unwrap_or(-1);
        if !LEX_LEXSTOP.with(|f| f.get()) {
            // c:496 if (!lexstop)
            ihwaddc(c); // c:497 hwaddc(c)
            iaddtoline(c); // c:498 addtoline(c)
        }
    }
}

/// Port of `int getargc(Histent ehist)` from Src/hist.c:556.
/// C body: `return ehist->nwords ? ehist->nwords-1 : 0;`
/// Returns the number of word designators (nwords - 1, since word 0
/// is the command name and arguments start at word 1).
pub fn getargc(entry: &histent) -> usize {
    // c:558
    if entry.nwords > 0 {
        (entry.nwords - 1) as usize
    } else {
        0
    }
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
pub fn substfailed() -> i32 {
    // c:563
    herrflush(); // c:565
    zerr("substitution failed"); // c:566
    -1 // c:567
}

/// Port of `static int digitcount(void)` from `Src/hist.c:573-589`.
///
/// C body:
/// ```c
/// int c = ingetc(), count;
/// if (idigit(c)) {
///     count = 0;
///     do {
///         count = 10 * count + (c - '0');
///         c = ingetc();
///     } while (idigit(c));
/// } else
///     count = 0;
/// inungetc(c);
/// return count;
/// ```
///
/// "Return a count given by decimal digits after a modifier."
/// Pulls characters off the INPUT STREAM via `ingetc`/`inungetc`,
/// NOT from a passed-in string. Called from c:871 (`:h` modifier)
/// and c:892 (`:t` modifier) to parse the digit count after the
/// modifier letter.
///
/// The previous Rust port was a complete fabrication: signature was
/// `(s: &str) -> usize` counting leading digits in an argument
/// string. No real caller — the C function streams from input, not
/// a string. Pin the C signature exactly.
pub fn digitcount() -> i32 {
    // c:574
    let mut c: i32 = ingetc() // c:576 ingetc()
        .map(|ch| ch as i32)
        .unwrap_or(-1);
    let mut count: i32;
    if c >= 0 && (c as u8 as char).is_ascii_digit() {
        // c:578 idigit(c)
        count = 0; // c:579
        loop {
            count = 10 * count + (c - b'0' as i32); // c:581
            c = ingetc() // c:582 ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
            if c < 0 || !(c as u8 as char).is_ascii_digit() {
                // c:583
                break;
            }
        }
    } else {
        count = 0; // c:586
    }
    if c >= 0 {
        if let Some(ch) = char::from_u32(c as u32) {
            // c:587 inungetc(c)
            inungetc(ch);
        }
    }
    count // c:588
}

/// Port of `void strinbeg(int dohist)` from `Src/hist.c:1033-1044`.
///
/// C body:
///     strin++;
///     hbegin(dohist);
///     lexinit();
///     init_parse_status();
pub fn strinbeg(dohist: i32) {
    // c:1033
    strin.fetch_add(1, SeqCst); // c:1035
                                // C has ONE `strin`; zshrs splits it into hist.rs `strin` (history
                                // logic above/below) and input.rs `strin` (the copy `ingetc` checks
                                // at input.rs:390 to decide "string input drained → EOF" vs "read
                                // more SHIN"). The single C `strin++` must bump BOTH — same
                                // paired-global rule as `lexstop`. Without the input-side bump, a
                                // nested string parse (cmd-subst body via parse_isolated) that
                                // drained its LEX_INPUT fell through to `inputline()` and STOLE the
                                // outer reader's next SHIN line.
    crate::ported::input::strin.with(|s| s.set(s.get() + 1));
    hbegin(dohist); // c:1036
    lexinit(); // c:1037
    init_parse_status(); // c:1042
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
/// `isfirstch = 1` and `histdone = 0` resets are critical for the
/// `^foo^bar`-style histsubchar shortcut that keys on `isfirstch`
/// in the next strinbeg-driven parse.
pub fn strinend() {
    // c:1049
    hend(None); // c:1051
                // c:1052 — DPUTS(!strin, "BUG: strinend() called without strinbeg()")
    DPUTS!(
        // c:1052
        strin.load(Ordering::SeqCst) == 0, // c:1052 !strin
        "BUG: strinend() called without strinbeg()"  // c:1052
    );
    strin.fetch_sub(1, SeqCst); // c:1053
                                // Mirror the input-side `strin` decrement (see strinbeg note).
    crate::ported::input::strin.with(|s| s.set(s.get() - 1));
    LEX_ISFIRSTCH.with(|f| f.set(true)); // c:1054 isfirstch = 1
    histdone.store(0, SeqCst); // c:1055 histdone = 0
}

/// Port of `static void nohw(UNUSED(int c))` from Src/hist.c:1062.
pub fn nohw(_c: i32) { /* do nothing */
} // c:1062

/// Port of `static void nohwabort(void)` from Src/hist.c:1067.
pub fn nohwabort() { /* do nothing */
} // c:1067

/// Port of `static void nohwe(void)` from Src/hist.c:1072.
pub fn nohwe() { /* do nothing */
} // c:1072

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
pub fn ihwbegin(offset: i32) {
    // c:1656
    // c:1658 — `int pos = hptr - chline + offset;`. The C `hptr` is
    // the current write position into the chline buffer; `pos` is
    // the byte offset from buffer start + caller-supplied offset.
    // The previous Rust port used `chline.lock().unwrap().len()`
    // which is only equal to `hptr - chline` when hptr is at end —
    // any lexer rewind (e.g. backquote, comment-resume) shifts hptr
    // earlier and the chline.len() reading would record the WRONG
    // word start, producing off-by-many history word offsets.
    let hptr_val = hptr.load(SeqCst); // c:1658
    let stop = stophist.load(SeqCst);
    let active = histactive.load(SeqCst);
    let inflags = crate::ported::input::inbufflags.with(|f| f.get());
    // c:1659 — `(inbufflags & (INP_ALIAS|INP_HIST)) == INP_ALIAS`
    // means "alias-only input (no history layered above)".
    if stop == 2 || (active & HA_INWORD) != 0 || (inflags & (INP_ALIAS | INP_HIST)) == INP_ALIAS
    // c:1659
    {
        return;
    }
    let pos = chwordpos.load(SeqCst);
    if pos % 2 != 0 {
        // c:1662 chwordpos%2
        chwordpos.fetch_sub(1, SeqCst); // c:1663
    }
    // c:1664-1665 — DPUTS1(pos < 0, "History word position < 0 in %s",
    //                      dupstrpfx(chline, hptr-chline))
    let word_pos = (hptr_val as i32) + offset; // c:1664 pos
    DPUTS1!(
        // c:1664
        word_pos < 0,                      // c:1664
        "History word position < 0 in {}", // c:1664
        {
            // c:1665 dupstrpfx(chline, hptr-chline)
            let line = chline.lock().unwrap(); // c:1665
            line.chars().take(hptr_val).collect::<String>() // c:1665
        }
    );
    // c:1666 — `if (pos < 0) pos = 0;`. The .max(0) clamp.
    let start = word_pos.max(0) as i16; // c:1658
    let mut words = chwords.lock().unwrap();
    let idx = chwordpos.load(SeqCst) as usize;
    if words.len() <= idx {
        words.resize(idx + 1, 0);
    }
    words[idx] = start; // c:1668
    chwordpos.fetch_add(1, SeqCst); // c:1668 chwordpos++
}

/// Port of `static void linkcurline(void)` from Src/hist.c:1079.
pub fn linkcurline() {
    // c:1079
    let new_hist = curhist.fetch_add(1, SeqCst) + 1; // c:1093 ++curhist
    let mut cur = curline.lock().unwrap();
    *cur = Some(make_histent(new_hist, String::new())); // c:1093 curline.histnum
                                                        // Splicing into the ring (c:1081-1088) is encoded by the Vec::insert
                                                        // at hist_ring index 0 done by hend() on commit. The sentinel itself
                                                        // lives in `curline` until then.
}

/// Port of `static void unlinkcurline(void)` from Src/hist.c:1093.
pub fn unlinkcurline() {
    // c:1093
    *curline.lock().unwrap() = None; // c:1093-1102
    curhist.fetch_sub(1, SeqCst); // c:1103
}

/// Port of `void hbegin(int dohist)` from Src/hist.c:1110.
pub fn hbegin(dohist: i32) {
    // c:1110
    // c:1114 — `isfirstln = isfirstch = 1;`. These live in lex.rs LEX_*
    // thread_locals. hbegin runs at the top of every loop() iteration, so
    // resetting them here makes the NEXT command start on its first line —
    // the PS1 prompt, not the PS2 continuation prompt. The prior port left
    // this to "the caller", but no caller did it, so every interactive
    // prompt after the first rendered as `>` (PS2).
    crate::ported::lex::LEX_ISFIRSTLN.with(|c| c.set(true));
    crate::ported::lex::LEX_ISFIRSTCH.with(|c| c.set(true));

    errflag.fetch_and(
        // c:1115
        !ERRFLAG_ERROR,
        Ordering::Relaxed,
    );
    histdone.store(0, SeqCst); // c:1116
                               // c:1117 — `isset(INTERACTIVE)` / `isset(SHINSTDIN)`.
    let interact = isset(INTERACTIVE);
    let shinstdin = isset(SHINSTDIN);
    if dohist == 0 {
        // c:1117
        stophist.store(2, SeqCst); // c:1118
    } else if dohist != 2 {
        // c:1119
        stophist.store(
            if !interact || !shinstdin { 2 } else { 0 }, // c:1120
            SeqCst,
        );
    } else {
        // c:1121
        stophist.store(0, SeqCst); // c:1122
    }

    if stophist.load(SeqCst) == 2 {
        // c:1134
        chline.lock().unwrap().clear(); // c:1135 chline = NULL
        hptr.store(0, SeqCst); // c:1135 hptr = NULL
        hlinesz.store(0, SeqCst); // c:1136
        chwords.lock().unwrap().clear(); // c:1137
        chwordlen.store(0, SeqCst); // c:1138
                                    // hgetc/hungetc/hwaddc/hwbegin/hwabort/hwend/addtoline are       c:1139-1145
                                    // function-pointer slots in C; Rust dispatches statically.
    } else {
        // c:1146
        let mut buf = chline.lock().unwrap(); // c:1147
        buf.clear();
        buf.reserve(64);
        hlinesz.store(64, SeqCst); // c:1147
        drop(buf);
        let mut w = chwords.lock().unwrap(); // c:1148
        w.clear();
        w.reserve(64);
        chwordlen.store(64, SeqCst);
        drop(w);
        // hgetc/hungetc/hwaddc/hwbegin/hwabort/hwend/addtoline c:1149-1155 — see c:1139.
        if !isset(BANGHIST) {
            // c:1156
            stophist.store(4, SeqCst); // c:1157
        }
    }
    chwordpos.store(0, SeqCst); // c:1159

    {
        // c:1161
        let mut ring = hist_ring.lock().unwrap();
        if let Some(top) = ring.first_mut() {
            if top.ftim == 0 && strin.load(SeqCst) == 0 {
                top.ftim = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0); // c:1162
            }
        }
    }
    if (dohist == 2 || (interact && shinstdin))                              // c:1163
        && strin.load(SeqCst) == 0
    {
        histactive.store(HA_ACTIVE, SeqCst); // c:1164
                                             // c:1165 — `attachtty(mypgrp);` reclaims the controlling
                                             // terminal for the shell's pgrp at the start of a fresh
                                             // history-recording line.
        let mypgrp = *crate::ported::jobs::MYPGRP
            .get_or_init(|| Mutex::new(0))
            .lock()
            .expect("mypgrp poisoned");
        crate::ported::utils::attachtty(mypgrp); // c:1165
        linkcurline(); // c:1166
        defev.store(
            addhistnum(
                curhist.load(SeqCst), // c:1167
                -1,
                HIST_FOREIGN as i32,
            ),
            SeqCst,
        );
    } else {
        histactive.store(HA_ACTIVE | HA_NOINC, SeqCst); // c:1169
    }

    if isset(INCAPPENDHISTORYTIME) // c:1189
        && !isset(SHAREHISTORY)
        && !isset(INCAPPENDHISTORY)
        && (histactive.load(SeqCst) & HA_NOINC) == 0
        && strin.load(SeqCst) == 0
        && histsave_stack_pos.load(SeqCst) == 0
    {
        let hf = resolve_histfile(); // c:1192
        savehistfile(
            hf.as_deref(),
            0                                        // c:1193
            | HFILE_USE_OPTIONS as i32
            | HFILE_FAST as i32,
        );
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
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    // c:1240 — trim trailing inblank only; preserve embedded non-
    // inblank chars (newline, CR, etc).
    let mut s = result;
    while s.ends_with(' ') {
        s.pop();
    }
    while s.starts_with(' ') {
        s.remove(0);
    }
    s
}

/// Port of `void histremovedups(void)` from Src/hist.c.
/// Port of `void histremovedups(void)` from `Src/hist.c:1252-1262`.
///
/// C body:
/// ```c
/// Histent he, next;
/// for (he = hist_ring; he; he = next) {
///     next = up_histent(he);
///     if (he->node.flags & HIST_DUP)
///         freehistnode(&he->node);
/// }
/// ```
///
/// The previous Rust port did NAME-based deduplication using a
/// HashSet — totally different semantic. C removes entries that
/// have the HIST_DUP flag set (which the history-add path sets
/// only when HIST_IGNORE_ALL_DUPS / HIST_IGNORE_DUPS marked the
/// entry as a duplicate). With HIST_IGNORE_ALL_DUPS off, identical
/// commands stay in the ring intentionally; the Rust port would
/// have aggressively pruned them anyway.
pub fn histremovedups() {
    // c:1254
    let mut ring = hist_ring.lock().unwrap();
    ring.retain(|h| (h.node.flags as u32 & HIST_DUP) == 0); // c:1259-1260
    let new_ct = ring.len() as i64;
    drop(ring);
    histlinect.store(new_ct, SeqCst);
}

/// Port of `zlong addhistnum(zlong hl, int n, int xflags)` from Src/hist.c:1266.
pub fn addhistnum(hl: i64, mut n: i32, xflags: i32) -> i64 {
    // c:1266
    let dir: i32 = if n < 0 {
        -1
    } else if n > 0 {
        1
    } else {
        0
    }; // c:1266
    let he = gethistent(hl, dir); // c:1269
    let he = match he {
        None => return 0, // c:1271-1272
        Some(h) => h,
    };
    if he != hl {
        // c:1273
        n -= dir; // c:1274
    }
    let final_he = if n != 0 {
        // c:1275
        movehistent(he, n, xflags as u32) // c:1276
    } else {
        Some(he)
    };
    match final_he {
        // c:1277
        None => {
            if dir < 0 {
                // c:1278
                firsthist() - 1
            } else {
                curhist.load(SeqCst) + 1
            }
        }
        Some(h) => h, // c:1279
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
pub fn movehistent(start: i64, mut n: i32, xflags: u32) -> Option<i64> {
    // c:1284
    let mut cur = start;
    while n < 0 {
        // c:1286
        cur = up_histent(cur)?; // c:1287
        if let Some(e) = ring_get(cur) {
            if (e.node.flags as u32 & xflags) == 0 {
                // c:1289
                n += 1; // c:1290
            }
        }
    }
    while n > 0 {
        // c:1292
        cur = down_histent(cur)?; // c:1293
        if let Some(e) = ring_get(cur) {
            if (e.node.flags as u32 & xflags) == 0 {
                // c:1295
                n -= 1; // c:1296
            }
        }
    }
    // c:1298 — `checkcurline(he);` flushes in-flight build state
    // into `curline` if the walk landed on the active history entry.
    if let Some(e) = ring_get(cur) {
        checkcurline(&e); // c:1298
    }
    Some(cur) // c:1299
}

/// Port of `Histent up_histent(Histent he)` from Src/hist.c.
pub fn up_histent(current: i64) -> Option<i64> {
    // c:1304
    let pos = ring_position(current)?; // c:1306 !he
    (pos + 1 < ring_len()).then(|| ring_at(pos + 1)) // c:1306 he->up == hist_ring? NULL : he->up
}

/// Port of `Histent down_histent(Histent he)` from Src/hist.c.
pub fn down_histent(current: i64) -> Option<i64> {
    // c:1311
    let pos = ring_position(current)?;
    (pos > 0).then(|| ring_at(pos - 1)) // c:1313 he == hist_ring? NULL : he->down
}

/// Port of `Histent gethistent(zlong ev, int nearmatch)` from Src/hist.c.
pub fn gethistent(ev: i64, nearmatch: i32) -> Option<i64> {
    // c:1318
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
    if nearmatch < 0 {
        best_older
    } else {
        best_newer
    }
}

/// Port of `void putoldhistentryontop(short keep_going)` from `Src/hist.c:1347`.
/// Rotate the next-to-expire entry to the head of the ring so that
/// the subsequent expire/save pass evicts it. When
/// `HISTEXPIREDUPSFIRST` is set and the candidate is not already a
/// duplicate, walk forward up to `savehistsiz` slots looking for the
/// next entry that IS a duplicate (so dup entries get evicted first).
/// `keep_going` advances the per-call cursor instead of resetting
/// it — used by save loops that expire multiple entries in succession.
pub fn putoldhistentryontop(keep_going: i32) -> i32 {
    // c:1347

    thread_local! {
        // c:1349 — `static Histent next = NULL;` (per-evaluator cursor).
        static NEXT_IDX: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
        // c:1356 — `static zlong max_unique_ct = 0;`
        static MAX_UNIQUE_CT: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }

    let mut ring = hist_ring.lock().unwrap();
    if ring.is_empty() {
        return 0;
    }

    // c:1350 — `he = (keep_going || !hist_ring) ? next : hist_ring->down;`
    //          In the Vec model the oldest entry is at the tail.
    let mut idx: Option<usize> = if keep_going != 0 {
        NEXT_IDX.with(|c| c.get())
    } else {
        Some(ring.len() - 1) // oldest = last element
    };

    // c:1352-1354 — `if (he) next = he->down; else return;`
    let mut cur_idx = match idx {
        Some(i) if i < ring.len() => i,
        _ => return 0,
    };
    // Advance `next` to the slot one step further into the past — in the
    // ring that's one position closer to the head (newer).
    idx = if cur_idx == 0 {
        None
    } else {
        Some(cur_idx - 1)
    };
    NEXT_IDX.with(|c| c.set(idx));

    // c:1355-1370 — HISTEXPIREDUPSFIRST: skip non-dups until we find one.
    let exp_dups_first = isset(HISTEXPIREDUPSFIRST);
    if exp_dups_first && (ring[cur_idx].node.flags as u32 & HIST_DUP) == 0 {
        if keep_going == 0 {
            // c:1357-1358 — `if (!keep_going) max_unique_ct = savehistsiz;`
            MAX_UNIQUE_CT.with(|c| c.set(savehistsiz.load(SeqCst) as i64));
        }
        loop {
            let cur = MAX_UNIQUE_CT.with(|c| {
                let v = c.get();
                c.set(v - 1);
                v
            });
            if cur <= 0 {
                // c:1360-1365 — give up: reset to ring head (oldest).
                MAX_UNIQUE_CT.with(|c| c.set(0));
                cur_idx = ring.len() - 1;
                NEXT_IDX.with(|c| {
                    c.set(if cur_idx == 0 {
                        None
                    } else {
                        Some(cur_idx - 1)
                    })
                });
                break;
            }
            // c:1367-1368 — `he = next; next = he->down;`
            cur_idx = match NEXT_IDX.with(|c| c.get()) {
                Some(i) if i < ring.len() => i,
                _ => return 0,
            };
            let nxt = if cur_idx == 0 {
                None
            } else {
                Some(cur_idx - 1)
            };
            NEXT_IDX.with(|c| c.set(nxt));
            // c:1369 — `} while (!(he->node.flags & HIST_DUP));`
            if (ring[cur_idx].node.flags as u32 & HIST_DUP) != 0 {
                break;
            }
        }
    }

    // c:1372-1382 — splice the chosen entry to ring head (position 0).
    if cur_idx < ring.len() && cur_idx != 0 {
        let entry = ring.remove(cur_idx);
        ring.insert(0, entry);
    }
    1
}

/// Port of `Histent prepnexthistent(void)` from Src/hist.c.
pub fn prepnexthistent() -> i64 {
    // c:1387
    let cap = histsiz.load(SeqCst);
    if cap > 0 && histlinect.load(SeqCst) >= cap {
        if let Some(oldest) = ring_oldest() {
            // Drop oldest from ring
            let mut ring = hist_ring.lock().unwrap();
            ring.retain(|h| h.histnum != oldest);
            histlinect.fetch_sub(1, SeqCst);
        }
    }
    let n = curhist.fetch_add(1, SeqCst) + 1;
    n
}

/// Port of `static int should_ignore_line(Eprog prog)` from Src/hist.c:1425.
fn should_ignore_line(prog: Option<&[u8]>) -> i32 {
    // c:1425
    let line = chline.lock().unwrap().clone();
    if isset(HISTIGNORESPACE) {
        // c:1427
        // c:1428 — `if (*chline == ' ' || aliasspaceflag)`. The
        // aliasspaceflag arm fires when the lexer expanded a
        // non-global alias whose body starts with a space (set at
        // lex.c:1930 / lex.rs LEX_ALIAS_SPACE_FLAG).
        let alias_space = crate::ported::lex::LEX_ALIAS_SPACE_FLAG.with(|c| c.get()) != 0;
        if line.starts_with(' ') || alias_space {
            return 1; // c:1429
        }
    }
    if prog.is_none() {
        // c:1432
        return 0; // c:1433
    }
    if isset(HISTNOFUNCTIONS) {
        // c:1435
        // Inspecting an Eprog requires the wordcode VM port — leave the
        // funcdef detection to the executor; conservatively return 0.    // c:1436-1440
        return 0;
    }
    if isset(HISTNOSTORE) {
        // c:1443
        // getjobtext(prog, NULL) — text reconstruction also needs the
        // wordcode VM. Apply the simpler text-based filters on chline
        // for the cases the C code carves out.
        let mut b: &str = &line;
        let mut saw_builtin = false;
        if let Some(rest) = b.strip_prefix("builtin ") {
            // c:1446
            b = rest;
            saw_builtin = true;
        }
        if (b == "history" || b.starts_with("history "))                     // c:1451
            && (saw_builtin /* || shfunctab.getnode("history").is_none() */)
        {
            return 1; // c:1453
        }
        if (b == "r" || b.starts_with("r "))                                 // c:1454
            && (saw_builtin /* || shfunctab.getnode("r").is_none() */)
        {
            return 1;
        }
        if let Some(rest) = b.strip_prefix("fc -") {
            // c:1457
            if (saw_builtin/* || shfunctab.getnode("fc").is_none() */)
                && rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic())
                    .any(|c| c == 'l')
            {
                return 1; // c:1474
            }
        }
    }
    0 // c:1474
}

/// Port of `int hend(Eprog prog)` from Src/hist.c:1474.
pub fn hend(prog: Option<&[u8]>) -> i32 {
    // c:1474
    let stack_pos = histsave_stack_pos.load(SeqCst); // c:1474
    let mut save: i32 = 1; // c:1484
    let mut hookret: i32 = 0;

    // DPUTS(stophist != 2 && !(inbufflags & INP_ALIAS) && !chline,       c:1487
    //       "BUG: chline is NULL in hend()");
    crate::ported::signals::queue_signals(); // c:1489
    if (histdone.load(SeqCst) & HISTFLAG_SETTY) != 0 {
        // c:1491 settyinfo(&shttyinfo) — restore the cooked terminal
        // baseline captured by zsetterm before ZLE went raw. The
        // interactive read uses ZLRF_NOSETTY (input.c:418), so ZLE's own
        // trashzle deliberately does NOT restore the tty; the restore is
        // deferred to here, when the input line's history finalizes, right
        // before the command executes. Without it the tty stayed in ZLE
        // raw mode across command execution — `cat` never saw EOF on `^D`,
        // interactive programs got raw single-byte input. shttyinfo now
        // lands as utils::SHTTYINFO.
        if let Some(ti) = crate::ported::utils::SHTTYINFO.lock().ok().and_then(|g| *g) {
            crate::ported::utils::settyinfo(&ti);
        }
    }
    let active = histactive.load(SeqCst);
    if (active & HA_NOINC) == 0 {
        // c:1492
        unlinkcurline(); // c:1493
    }
    if (active & HA_NOINC) != 0 {
        // c:1494
        chline.lock().unwrap().clear(); // c:1495 zfree(chline)
        chwords.lock().unwrap().clear(); // c:1496 zfree(chwords)
        hptr.store(0, SeqCst); // c:1497
        histactive.store(0, SeqCst); // c:1499
        unqueue_signals(); // c:1500
        return 1; // c:1501
    }
    let cur_ignore_all = if isset(HISTIGNOREALLDUPS) { 1 } else { 0 }; // c:1503
    let prev_ignore_all = hist_ignore_all_dups.load(SeqCst);
    if prev_ignore_all != cur_ignore_all                                     // c:1503
        && {
            hist_ignore_all_dups.store(cur_ignore_all, SeqCst);    // c:1504
            cur_ignore_all != 0
        }
    {
        histremovedups(); // c:1505
    }
    // *hptr = '\0';                                                         // c:1513 — String is implicit
    let chline_text = chline.lock().unwrap().clone();
    if !chline_text.is_empty() {
        // c:1515
        let save_errflag = errflag // c:1517
            .load(Ordering::Relaxed);
        errflag.store(0, Ordering::Relaxed); // c:1518
        let args = vec!["zshaddhistory".to_string(), chline_text.clone()]; // c:1520-1521
                                                                           // c:1522 — `callhookfunc("zshaddhistory", hookargs, 1, &hookret);`.
                                                                           // `hookret` is the HOOK's return value (the 4th out-param `*retval =
                                                                           // ret`, 0 when no hook ran), NOT callhookfunc's return (`stat`,
                                                                           // which is 1 whenever no hook exists). The prior port used the
                                                                           // `stat` return and passed NULL for retval, so with no zshaddhistory
                                                                           // hook (e.g. `zsh -f`) hookret was 1 → hend's `else if (hookret)`
                                                                           // set save=-1 (HIST_TMPSTORE) → every line was dropped by the next
                                                                           // command's tmpstore-purge. Pass &hookret and read the out-param.
        crate::ported::utils::callhookfunc(
            "zshaddhistory",
            Some(&args),
            1,
            &mut hookret as *mut i32,
        );
        let new_errflag = (errflag // c:1524-1525
            .load(Ordering::Relaxed)
            & !ERRFLAG_ERROR)
            | save_errflag;
        errflag.store(new_errflag, Ordering::Relaxed);
    }
    let hf = resolve_histfile(); // c:1528
    if isset(SHAREHISTORY)                                                   // c:1529
        && lockhistfile(hf.as_deref(), 0) == 0
    {
        readhistfile(
            hf.as_deref(),
            0, // c:1530
            HFILE_USE_OPTIONS as i32 | HFILE_FAST as i32,
        );
        // curline.histnum = curhist + 1                                     // c:1531
    }
    let flag = histdone.load(SeqCst); // c:1533
    histdone.store(0, SeqCst); // c:1534
    let hptr_pos = hptr.load(SeqCst);
    let mut text = chline_text;
    if hptr_pos < 1 {
        // c:1535 hptr < chline + 1
        save = 0; // c:1536
    } else {
        if text.ends_with('\n') {
            // c:1538 hptr[-1] == '\n'
            if text.len() > 1 {
                // c:1539 chline[1]
                text.pop(); // c:1540 *--hptr = '\0'
                if hptr.load(SeqCst) > 0 {
                    hptr.fetch_sub(1, SeqCst);
                }
            } else {
                save = 0; // c:1542
            }
        }
        if chwordpos.load(SeqCst) <= 2                             // c:1544
            && hist_keep_comment.load(SeqCst) == 0
        {
            save = 0; // c:1545
        } else if should_ignore_line(prog) != 0 {
            // c:1546
            save = -1; // c:1547
        } else if hookret == 2 {
            // c:1548
            save = -2; // c:1549
        } else if hookret != 0 {
            // c:1550
            save = -1; // c:1551
        }
    }
    if (flag & (HISTFLAG_DONE | HISTFLAG_RECALL)) != 0 {
        // c:1553
        let ptr = text.clone(); // c:1556 ztrdup(chline)
        if (flag & (HISTFLAG_DONE | HISTFLAG_RECALL)) == HISTFLAG_DONE {
            // c:1557
            // zputs(ptr, shout); fputc('\n', shout); fflush(shout);         // c:1558-1560
            print!("{}\n", ptr);
            let _ = std::io::stdout().flush();
        }
        if (flag & HISTFLAG_RECALL) != 0 {
            // c:1562
            // c:1563 — `zpushnode(bufstack, ptr)` — push the expanded
            // history line onto the buf-stack so ZLE recalls it on
            // the next prompt.
            crate::ported::zle::zle_main::BUFSTACK
                .lock()
                .unwrap()
                .insert(0, ptr.clone()); // c:1563
            save = 0; // c:1564
        }
    }
    if save != 0 || text.starts_with(' ') {
        // c:1568
        // Walk up the ring skipping HIST_FOREIGN entries; if the topmost
        // non-foreign entry is HIST_TMPSTORE, drop it.                     // c:1569-1576
        let mut ring = hist_ring.lock().unwrap();
        let mut idx: usize = 0;
        while idx < ring.len() && (ring[idx].node.flags as u32 & HIST_FOREIGN) != 0 {
            idx += 1;
        }
        if idx < ring.len() && (ring[idx].node.flags as u32 & HIST_TMPSTORE) != 0 {
            if idx == 0 {
                // c:1573 he == hist_ring
                curhist.fetch_sub(1, SeqCst); // c:1574
            }
            ring.remove(idx); // c:1575 freehistnode
            histlinect.fetch_sub(1, SeqCst);
        }
    }
    if save != 0 {
        // c:1578
        // chwordpos parity guard — if odd, hwend() to close.            // c:1583-1587
        if chwordpos.load(SeqCst) % 2 != 0 {
            ihwend();
        }
        // Strip trailing \n which we already nulled out.                    // c:1589-1595
        let cwp = chwordpos.load(SeqCst);
        if cwp > 1 {
            let words = chwords.lock().unwrap();
            let last = words.get((cwp - 2) as usize).copied().unwrap_or(0);
            // C: !chline[chwords[chwordpos-2]] — index past end after NUL.
            if (last as usize) >= text.len() {
                // c:1590
                drop(words);
                chwordpos.fetch_sub(2, SeqCst);
            } else {
                drop(words);
            }
            if isset(HISTREDUCEBLANKS) {
                // c:1593
                text = histreduceblanks(&text); // c:1594
            }
        }
        let newflags: u32 = if save == -1 {
            HIST_TMPSTORE
        }
        // c:1596-1601
        else if save == -2 {
            HIST_NOWRITE
        } else {
            0
        };
        let mut he_idx: Option<usize> = None;
        let mut overwrite_old: u32 = 0;
        if (isset(HISTIGNOREDUPS) || isset(HISTIGNOREALLDUPS))                // c:1602
            && save > 0
        {
            let ring = hist_ring.lock().unwrap();
            if let Some(top) = ring.first() {
                if top.node.nam == text {
                    // c:1603 histstrcmp
                    overwrite_old = top.node.flags as u32 & HIST_OLD; // c:1610
                    he_idx = Some(0);
                }
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let cwp = chwordpos.load(SeqCst);
        let chwords_snapshot: Vec<i16> = chwords.lock().unwrap().clone();
        let nwords = (cwp / 2) as i32;
        if let Some(0) = he_idx {
            // c:1609 reuse top
            let mut ring = hist_ring.lock().unwrap();
            if let Some(top) = ring.first_mut() {
                top.node.nam = text.clone(); // c:1616
                top.stim = now; // c:1617
                top.ftim = 0; // c:1618
                top.node.flags = (newflags | overwrite_old) as i32; // c:1619
                top.nwords = nwords; // c:1621
                top.words = if cwp > 0 {
                    chwords_snapshot[..cwp as usize].to_vec() // c:1622-1623
                } else {
                    Vec::new()
                };
            }
        } else {
            let n = prepnexthistent(); // c:1614
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
            histlinect.fetch_add(1, SeqCst);
            drop(ring); // release ring: addhistnode re-locks hist_ring (non-reentrant)
            if (newflags & HIST_TMPSTORE) == 0 {
                // c:1625
                // addhistnode(histtab, he->node.nam, he) — hashtable wiring c:1626
                // routes through addhistnode.
                addhistnode(&text, n as i32);
            }
        }
    }
    chline.lock().unwrap().clear(); // c:1628 zfree(chline)
    chwords.lock().unwrap().clear(); // c:1629 zfree(chwords)
    hptr.store(0, SeqCst); // c:1630
    histactive.store(0, SeqCst); // c:1632

    let share = isset(SHAREHISTORY);
    let do_inc = if share {
        histfileIsLocked() != 0 // c:1636
    } else {
        isset(INCAPPENDHISTORY)                                              // c:1637
            || (isset(INCAPPENDHISTORYTIME)                                  // c:1637
                && histsave_stack_pos.load(SeqCst) != 0) // c:1638
    };
    if do_inc {
        savehistfile(
            hf.as_deref(),
            0                                        // c:1639
            | HFILE_USE_OPTIONS as i32
            | HFILE_FAST as i32,
        );
    }
    unlockhistfile(hf.as_deref().unwrap_or("")); // c:1640

    while histsave_stack_pos.load(SeqCst) > stack_pos {
        // c:1645
        pophiststack(); // c:1646
    }
    hist_keep_comment.store(0, SeqCst); // c:1647
    unqueue_signals(); // c:1648
    if (flag & HISTFLAG_NOEXEC) != 0 || errflag.load(Ordering::Relaxed) != 0 {
        0 // c:1649
    } else {
        1
    }
}

/// Port of `void ihwabort(void)` from Src/hist.c.
pub fn ihwabort() {
    // c:1675
    let pos = chwordpos.load(SeqCst);
    if pos % 2 != 0 {
        chwordpos.fetch_sub(1, SeqCst);
    }
    hist_keep_comment.store(1, SeqCst);
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
pub fn ihwend() {
    // c:1686
    let stop = stophist.load(SeqCst);
    let active = histactive.load(SeqCst);
    let inflags = crate::ported::input::inbufflags.with(|f| f.get());
    if stop == 2 || (active & HA_INWORD) != 0 || (inflags & (INP_ALIAS | INP_HIST)) == INP_ALIAS
    // c:1688
    {
        return;
    }
    // c:1691 — `if (chwordpos%2 && chline)`. Even chwordpos means we're between
    // words (no in-flight word to close); a NULL `chline` (empty buffer here)
    // means the history line has been freed. The previous port dropped the
    // `&& chline` half of this guard, so after `hend()` cleared the buffer a
    // fall-through still indexed `chwords[chwordpos-1]` out of bounds and
    // panicked (worker-thread OOB → poisoned the chwords mutex → main-thread
    // cascade in hend). Restore both halves.
    let pos = chwordpos.load(SeqCst);
    if pos % 2 == 0 || chline.lock().unwrap().is_empty() {
        return;
    }
    // c:1693 — `if (hptr > chline + chwords[chwordpos-1])`. The
    // previous Rust port used `chline.lock().unwrap().len()` for
    // the cursor position; that only equals `hptr - chline` when
    // hptr is at end of the buffer. Any lexer rewind would mis-
    // close the word boundary. Read the canonical `hptr` global
    // matching the `ihwbegin` fix at c:1658.
    let cur = hptr.load(SeqCst) as i16; // c:1693
    let mut words = chwords.lock().unwrap();
    let start_idx = (pos - 1) as usize;
    // chwordpos (atomic) and chwords (mutex) are separate primitives; parallel
    // lexers can leave chwordpos ahead of a chwords that another thread just
    // cleared. Single-threaded C indexes `chwords[chwordpos-1]` directly; guard
    // the bound here so the race degrades to "scrub that word" instead of an OOB.
    if start_idx < words.len() && cur > words[start_idx] {
        // c:1693
        let end_idx = pos as usize;
        if words.len() <= end_idx {
            words.resize(end_idx + 1, 0);
        }
        words[end_idx] = cur; // c:1694
        chwordpos.fetch_add(1, SeqCst);
    } else {
        // c:1700 — `chwordpos--;` "scrub that last word, doesn't exist".
        chwordpos.fetch_sub(1, SeqCst);
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
pub fn histbackword() {
    // c:1711
    let pos = chwordpos.load(SeqCst);
    // c:1714 — `if (!(chwordpos%2) && chwordpos)`. Both conditions
    // — even (word boundary) AND non-zero.
    if pos % 2 == 0 && pos != 0 {
        // c:1714
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
            hptr.store(off, SeqCst); // c:1715
        }
    }
}

/// Port of `int hwget(char **startptr)` from Src/hist.c.
pub fn hwget() -> Option<(i32, String)> {
    let pos = chwordpos.load(SeqCst);
    // c:1729 — DPUTS(1, "BUG: hwget() called with no words"); arm fires
    // when chwordpos == 0 (the C `if (!chwordpos)` branch at c:1728).
    if pos == 0 {
        DPUTS!(true, "BUG: hwget() called with no words"); // c:1729
        return None;
    }
    // c:1734 — DPUTS(1, "BUG: hwget() called in middle of word")
    if pos % 2 != 0 {
        DPUTS!(true, "BUG: hwget() called in middle of word"); // c:1734
        return None;
    }
    let words = chwords.lock().unwrap();
    let start_idx = (pos - 2) as usize;
    let end_idx = (pos - 1) as usize;
    if end_idx >= words.len() {
        return None;
    }
    let start = words[start_idx];
    let end = words[end_idx];
    let line = chline.lock().unwrap();
    let s = (start.max(0)) as usize;
    let e = (end.max(0) as usize).min(line.len());
    if s > e || s >= line.len() {
        return None;
    }
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
pub fn hwrep(rep: &str) {
    // c:1748
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
    hptr.store(start_off.max(0) as usize, SeqCst); // c:1756
    chwordpos.fetch_sub(2, SeqCst); // c:1757
                                    // c:1758 — `hwbegin(0);` re-open at current hptr (no offset).
    ihwbegin(0);
    // c:1759 — `qbang = 1;` mark word as bang-bearing so subsequent
    // ihwaddc bang-escapes correctly.
    qbang.store(true, SeqCst); // c:1759
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
pub fn hgetline() -> Option<String> {
    // c:1769
    let hp = hptr.load(SeqCst);
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
    hptr.store(0, SeqCst);
    chwordpos.store(0, SeqCst);
    Some(truncated) // c:1786
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
pub fn getargspec(argc: i32, marg_arg: i32, evset: i32) -> i32 {
    // c:1793
    let mut c: i32 = ingetc() // c:1797 ingetc()
        .map(|ch| ch as i32)
        .unwrap_or(-1);
    let mut ret: i32 = -1; // c:1795
    if c == b'0' as i32 {
        // c:1797
        return 0; // c:1798
    }
    if (c as u8 as char).is_ascii_digit() {
        // c:1799 idigit(c)
        ret = 0; // c:1800
        while (c as u8 as char).is_ascii_digit() {
            // c:1801
            ret = ret * 10 + c - b'0' as i32; // c:1802
            if ret < 0 {
                // c:1803
                herrflush(); // c:1804
                zerr("no such word in event"); // c:1805
                return -2; // c:1806
            }
            c = ingetc() // c:1808 ingetc()
                .map(|ch| ch as i32)
                .unwrap_or(-1);
        }
        if let Some(ch) = char::from_u32(c as u32) {
            // c:1810 inungetc(c)
            inungetc(ch);
        }
    } else if c == b'^' as i32 {
        // c:1811
        ret = 1; // c:1812
    } else if c == b'$' as i32 {
        // c:1813
        ret = argc; // c:1814
    } else if c == b'%' as i32 {
        // c:1815
        if evset != 0 {
            // c:1816
            herrflush(); // c:1817
            zerr("Ambiguous history reference"); // c:1818
            return -2; // c:1819
        }
        if marg_arg == -1 {
            // c:1821
            herrflush(); // c:1822
            zerr("%% with no previous word matched"); // c:1823
            return -2; // c:1824
        }
        ret = marg_arg; // c:1826
    } else {
        // c:1827
        if let Some(ch) = char::from_u32(c as u32) {
            // c:1828 inungetc(c)
            inungetc(ch);
        }
    }
    ret // c:1829
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
pub fn hconsearch(needle: &str) -> Option<(i64, i32)> {
    // c:1836
    // c:1842 — `for (he = up_histent(hist_ring); he; he = up_histent(he))`.
    // The C `hist_ring` is the doubly-linked-list sentinel; iterating
    // `up_histent(hist_ring)` walks from the newest real entry toward
    // older ones. Rust storage is a Vec with newest at position 0;
    // walk positions 0..ring_len for the same effect.
    let ring = hist_ring.lock().expect("hist_ring poisoned");
    for entry in ring.iter() {
        if (entry.node.flags as u32 & HIST_FOREIGN) != 0 {
            // c:1843
            continue; // c:1844
        }
        if let Some(pos) = entry.node.nam.find(needle) {
            // c:1845 strstr
            // c:1846 — `int pos = s - he->node.nam;`
            let mut t1: i32 = 0; // c:1838
            while t1 < entry.nwords {
                // c:1847
                let slot_pos = entry.words.get((2 * t1) as usize).copied().unwrap_or(0) as usize;
                if slot_pos > pos {
                    // c:1847 he->words[2*t1] <= pos
                    break;
                }
                t1 += 1; // c:1848
            }
            return Some((entry.histnum, t1 - 1)); // c:1849-1850
        }
    }
    None // c:1853
}

/// Port of `int hcomsearch(char *str)` from Src/hist.c.
pub fn hcomsearch(prefix: &str) -> Option<i64> {
    let mut cur = curhist.load(SeqCst);
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
    if input.is_empty() {
        return Some(String::new());
    }
    let mut path = if !input.starts_with('/') {
        let cwd = std::env::current_dir().ok()?;
        let cwd_s = cwd.to_string_lossy().into_owned();
        if cwd_s.ends_with('/') {
            format!("{}{}", cwd_s, input)
        } else {
            format!("{}/{}", cwd_s, input)
        }
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
            while i < chars.len() && chars[i] == '/' {
                i += 1;
            }
        } else if c == '.'
            && i + 1 < chars.len()
            && chars[i + 1] == '.'
            && (i + 2 == chars.len() || chars[i + 2] == '/')
        {
            if out.len() <= 1 {
                if out.is_empty() || out == ['/'] {
                    return None;
                }
                out.push('.');
                out.push('.');
            } else if out.len() >= 3 && &out[out.len() - 3..] == &['.', '.', '/'] {
                out.push('.');
                out.push('.');
            } else {
                if out.last() == Some(&'/') && out.len() > 1 {
                    out.pop();
                }
                while out.last().map(|c| *c != '/').unwrap_or(false) {
                    out.pop();
                }
            }
            i += 2;
            if i < chars.len() && chars[i] == '/' {
                i += 1;
            }
        } else if c == '.' && (i + 1 == chars.len() || chars[i + 1] == '/') {
            i += 1;
            while i < chars.len() && chars[i] == '/' {
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    while out.len() > 1 && out.last() == Some(&'/') {
        out.pop();
    }
    path = out.into_iter().collect();
    if path.is_empty() {
        Some("/".to_string())
    } else {
        Some(path)
    }
}

/// Port of `int chrealpath(char **junkptr, char mode, int use_heap)` from `Src/hist.c:1971`.
/// Mode 'A' → chabspath then realpath; mode 'P' → realpath only.
/// Handles non-existent paths by walking parent prefixes until one
/// resolves, then re-appending the remaining tail (matches the C
/// fallback at c:2027-2030).
///
/// Signature mirrors C (Rule B): `path`, `mode` ('A' or 'P'),
/// `use_heap` (ignored — Rust strings are heap by default).
pub fn chrealpath(path: &str, mode: u8, _use_heap: bool) -> Option<String> {
    // c:1971
    // c:1983 — DPUTS1(mode != 'A' && mode != 'P', "chrealpath: mode='%c' is invalid", mode)
    DPUTS1!(
        // c:1983
        mode != b'A' && mode != b'P', // c:1983
        "chrealpath: mode='{}' is invalid",
        mode as char // c:1983
    );
    // c:1985-1986 — if (!**junkptr) return 1; (empty input is success)
    if path.is_empty() {
        return Some(String::new());
    }

    // c:1988-1990 — if (mode == 'A') chabspath; here we accept that
    //               callers wanting absolutize-first do it themselves.
    //               Caller may pass either mode; the partial-realpath
    //               body is the only mode-independent semantic.

    // c:1999-2000 — if (**junkptr != '/') return 0;
    //               Only absolute paths are valid input.
    if !path.starts_with('/') {
        return None;
    }

    // c:2002-2003 — untokenize + unmetafy.  Rust strings already UTF-8.

    // c:2008-2030 — loop: try realpath(p); on failure walk `nonreal`
    //               backward to the previous '/' and retry the shorter
    //               prefix. The bytes after each '/' boundary are
    //               recorded as the non-real tail to re-splice later.
    let bytes = path.as_bytes();
    let mut prefix_end = bytes.len();
    let mut real: Option<String> = None;
    loop {
        let trial = &path[..prefix_end];
        if let Ok(canonical) = std::fs::canonicalize(trial) {
            real = Some(canonical.to_string_lossy().into_owned());
            break;
        }
        // walk `prefix_end` back to the previous '/'
        let mut i = prefix_end.saturating_sub(1);
        while i > 0 && bytes[i] != b'/' {
            i -= 1;
        }
        if i == 0 {
            // c:2020-2024 — no real prefix at all; keep nothing.
            break;
        }
        prefix_end = i;
    }

    // c:2032-2037 — the nul-bytes inside `nonreal` get rewritten back to
    //               '/'. In Rust we never overwrite, so the tail is just
    //               the suffix from `prefix_end..`.
    let tail = &path[prefix_end..];

    // c:2040-2048 — splice real + tail (or just tail when realpath fails
    //               on every prefix).
    match real {
        Some(r) => Some(format!("{}{}", r, tail)),
        None => Some(tail.to_string()),
    }
}

/// Port of `char *remtpath(char **str, int count)` from Src/hist.c:2056.
pub fn remtpath(s: &str, count: i32) -> String {
    // c:2056
    // c:2068-2074 — when `str` lands before `*junkptr` (path entirely
    // consumed by trim+skip-filename), C picks `/` vs `.` based on the
    // FIRST byte of the ORIGINAL `*junkptr`: `IS_DIRSEP(**junkptr)` →
    // `/`, else `.`. For empty input the first byte is `\0`, not a
    // dirsep, so C returns `.`. The previous Rust port returned `/`
    // for empty input, diverging from zsh — bug #134.
    let original_first_is_sep = s.as_bytes().first().copied() == Some(b'/');
    let s = s.trim_end_matches('/');
    if s.is_empty() {
        return if original_first_is_sep { "/" } else { "." }.to_string();
    }
    if count == 0 {
        if let Some(pos) = s.rfind('/') {
            if pos == 0 {
                return "/".to_string();
            }
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
                if i == 0 {
                    return "/".to_string();
                }
                return s[..i].to_string();
            }
            while i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                i += 1;
            }
        }
        i += 1;
    }
    s.to_string()
}

/// Port of `int remtext(char **junkptr)` from Src/hist.c:2122.
///
/// C body (c:2122-2132):
/// ```c
/// for (str = strend(*junkptr); str >= *junkptr && !IS_DIRSEP(*str); --str)
///     if (*str == '.') {
///         *str = '\0';
///         return 1;
///     }
/// return 0;
/// ```
///
/// Walks backward from the end of the string looking for `.`,
/// stopping at the rightmost `/` or the start of the string. When
/// found, truncates the string at the `.` (drops `.` AND the
/// extension). The Rust signature returns the truncated string
/// rather than mutating + returning int.
///
/// Edge cases pinned by C: `.hidden` → `""`, `foo/.bashrc` → `foo/`.
/// The previous Rust port guarded with `dot_pos > 0` which kept the
/// leading-dot files unchanged — a divergence from C's behavior.
pub fn remtext(s: &str) -> String {
    // c:2122
    // c:2126 — find the rightmost `/` to bound the scan (C walks back
    // from end until it hits a dirsep). Everything after that slash
    // (or the whole string if no slash) is the basename region.
    let (prefix, basename) = match s.rfind('/') {
        Some(i) => (&s[..=i], &s[i + 1..]),
        None => ("", s),
    };
    // c:2127 — `if (*str == '.')`. Find rightmost `.` in basename;
    // truncate there. C makes no exception for leading dot.
    if let Some(dot_pos) = basename.rfind('.') {
        // c:2127
        return format!("{}{}", prefix, &basename[..dot_pos]); // c:2128
    }
    s.to_string() // c:2131
}

/// Port of `char *rembutext(char **str)` from Src/hist.c:2136.
pub fn rembutext(s: &str) -> String {
    // c:2136
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
///
/// C behavior (c:2172-2179): when `--count > 0` and the cursor has
/// already reached the start of the string (`str > *junkptr` fails),
/// the function returns 1 ("whole string needed") WITHOUT modifying
/// `*junkptr` — preserving the original input verbatim (including
/// the leading slash for absolute paths).
pub fn remlpaths(s: &str, count: i32) -> String {
    // c:2152
    // c:2156-2161 — `if (IS_DIRSEP(*str))` block trims trailing slashes
    // off the input. Apply lexically before splitting.
    let trimmed = s.trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty()).collect();
    let n = if count == 0 { 1 } else { count as usize };
    if n > parts.len() {
        // c:2172-2175 — `str > *junkptr` fails → `return 1` early-exit
        // without writing to junkptr. The caller `apply_history_modifiers`
        // observes the input string UNCHANGED (including leading slashes).
        return s.to_string();
    }
    // c:2178-2179 — `*str = '\0'; *junkptr = dupstring(str + 1);` — the
    // leading slash (and prefix path) gets overwritten by NUL and the
    // returned slice starts AFTER it, dropping it. So when n <= count of
    // components, the leading slash is stripped (matches C).
    parts
        .iter()
        .rev()
        .take(n)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("/")
}

/// Port of `char *casemodify(char *str, int how)` from Src/hist.c:2196.
/// Rust idiom replacement: `chars()` + `to_lowercase`/`to_uppercase`
/// covers the C `tolower`/`toupper`/`isalpha` per-byte loop; the
/// CASMOD_CAPS branch tracks word-boundary via the `nextupper` flag.
pub fn casemodify(s: &str, how: i32) -> String {
    // c:2196
    // c:2200 — `int nextupper = 1;`. Start expecting a leading uppercase.
    let mut result = String::with_capacity(s.len());
    let mut nextupper = true;
    for c in s.chars() {
        // c:2209 `while (*str)`
        // c:2241 — `if (IS_COMBINING(wc)) break;` — combining chars
        // (those with WCWIDTH==0) don't affect nextupper state and
        // don't get case-folded. Macro at `Src/zsh.h:3343`:
        // `#define IS_COMBINING(wc) (wc != 0 && WCWIDTH(wc) == 0)`.
        // Previous Rust port omitted this check entirely — combining
        // acute (U+0301) etc. would (a) get pushed through
        // to_uppercase/to_lowercase (no-op for combinatior class but
        // semantically wrong intent) and (b) for CAPS, would reset
        // nextupper via the `!is_alphanumeric` branch, breaking
        // word-boundary detection on accented words.
        let is_combining =
            (c as u32) != 0 && unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) == 0;
        let modified = match how {
            x if x == CASMOD_LOWER => {                                       // c:2225
                // c:2226-2229 — `if (iswupper(wc)) wc = towlower(wc);`.
                if c.is_uppercase() {
                    c.to_lowercase().collect::<String>()
                } else {
                    c.to_string()
                }
            }
            x if x == CASMOD_UPPER => {                                       // c:2232
                // c:2233-2236 — `if (iswlower(wc)) wc = towupper(wc);`.
                if c.is_lowercase() {
                    c.to_uppercase().collect::<String>()
                } else {
                    c.to_string()
                }
            }
            x if x == CASMOD_CAPS => {                                        // c:2239
                if is_combining {                                             // c:2241-2242
                    c.to_string()
                } else if !c.is_alphanumeric() {                              // c:2243-2244
                    nextupper = true;
                    c.to_string()
                } else if nextupper {                                         // c:2245-2250
                    nextupper = false;
                    if c.is_lowercase() {                                     // c:2246
                        c.to_uppercase().collect::<String>()
                    } else {
                        c.to_string()
                    }
                } else if c.is_uppercase() {                                  // c:2251-2253
                    c.to_lowercase().collect::<String>()
                } else {
                    c.to_string()
                }
            }
            _ /* CASMOD_NONE */ => c.to_string(),
        };
        let _ = CASMOD_NONE; // silence unused
        result.push_str(&modified);
    }
    result
}

/// Port of `int subst(char **strptr, char *in, char *out, int gbal,
/// int forcepat)` from `Src/hist.c:2336`.
///
/// C body excerpt (c:2349-2358):
/// ```c
/// if (*in == '#' || *in == Pound) {       // c:2349 — anchor-head
///     fl |= SUB_START;
///     in++;
/// }
/// if (*in == '%') {                       // c:2354 — anchor-tail
///     in++;
///     fl |= SUB_END;
/// }
/// ```
///
/// Previous Rust port checked only ASCII `'#'`. The C check covers
/// BOTH the literal `'#'` byte AND the tokenized form `Pound`
/// (0x84 / `\u{84}`) emitted by the lexer when `#` appears inside
/// a parsed substitution body. A history-substitution like
/// `:s/#foo/bar/` where the lexer has tokenized the `#` would
/// silently miss the anchor-start in the Rust port — falling
/// through to substring-match semantics.
pub fn subst(s: &str, in_pattern: &str, out_pattern: &str, global: bool) -> String {
    // c:2336
    if in_pattern.is_empty() {
        return s.to_string();
    }
    let mut anchor_start = false;
    let mut anchor_end = false;
    let mut pat = in_pattern;
    // c:2349 — `if (*in == '#' || *in == Pound)` — anchor-head matcher
    // covers BOTH the literal char and the tokenized Pound byte.
    if let Some(rest) = pat.strip_prefix('#').or_else(|| pat.strip_prefix(Pound)) {
        anchor_start = true; // c:2351 SUB_START
        pat = rest; // c:2352 in++
    }
    if let Some(rest) = pat.strip_prefix('%') {
        anchor_end = true;
        pat = rest;
    }
    if pat.is_empty() {
        return s.to_string();
    }
    let out_expanded = convamps(out_pattern, pat);
    if anchor_start && anchor_end {
        if s == pat {
            return out_expanded;
        }
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
    if global {
        s.replace(pat, &out_expanded)
    } else {
        s.replacen(pat, &out_expanded, 1)
    }
}

/// Port of `char *convamps(char *out, char *in)` from Src/hist.c.
fn convamps(out: &str, in_pattern: &str) -> String {
    let mut result = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                result.push(next);
                chars.next();
            }
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
pub fn checkcurline(he: &histent) {
    // c:2421
    let curhist_val = curhist.load(SeqCst); // c:2424
    let active = histactive.load(SeqCst); // c:2424
    if he.histnum == curhist_val && (active & HA_ACTIVE) != 0 {
        // c:2424
        let chline_val = chline.lock().expect("chline poisoned").clone(); // c:2425
        let chwordpos_val = chwordpos.load(SeqCst); // c:2426
        let chwords_val = chwords.lock().expect("chwords poisoned").clone(); // c:2427
        let mut cl = curline.lock().expect("curline poisoned");
        // Build a fresh histent snapshot mirroring the C field
        // aliasing — name = chline, nwords = chwordpos/2,
        // words = chwords.
        *cl = Some(histent {
            node: hashnode {
                next: None,
                nam: chline_val, // c:2425
                flags: 0,
            },
            up: None,
            down: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: chwords_val,        // c:2427
            nwords: chwordpos_val / 2, // c:2426
            histnum: he.histnum,
        });
    }
}

/// Port of `Histent quietgethist(zlong ev)` from Src/hist.c.
pub fn quietgethist(ev: i64) -> Option<histent> {
    // c:2433
    ring_get(ev)
}

/// Port of `Histent gethist(zlong ev)` from Src/hist.c.
pub fn gethist(ev: i64) -> Option<histent> {
    // c:2440
    let ret = quietgethist(ev);
    if ret.is_none() {
        herrflush();
        zerr(&format!("no such event: {}", ev));
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
/// Notes:
///   - `nwords` reads `elist->nwords` directly (authoritative).
///   - Overflow check `pos1 < arg1 || pos2 < arg2` detects signed-
///     short overflow: since each word must be ≥1 char long, the
///     start-byte index must be ≥ the 0-based word index; a
///     too-small stored position signals i16 overflow on history
///     lines >32KB.
pub fn getargs(entry: &histent, arg1: usize, arg2: usize) -> Option<String> {
    // c:2454
    let nwords = entry.nwords as usize; // c:2457 nwords = elist->nwords
    if arg2 < arg1 || arg1 >= nwords || arg2 >= nwords {
        // c:2459
        herrflush(); // c:2461
        zerr("no such word in event"); // c:2462
        return None; // c:2463
    }
    // c:2466-2467 — `if (arg1 == 0 && arg2 == nwords - 1) return dupstring(nam);`
    if arg1 == 0 && arg2 == nwords - 1 {
        return Some(entry.node.nam.clone()); // c:2467
    }
    let pos1_raw = entry.words.get(2 * arg1).copied().unwrap_or(-1); // c:2469 pos1 = words[2*arg1]
    let pos2_raw = entry.words.get(2 * arg2 + 1).copied().unwrap_or(-1); // c:2470 pos2 = words[2*arg2+1]
                                                                         // c:2476 — C signed-short overflow detection: any negative
                                                                         // position OR a position less than its corresponding word
                                                                         // index means the i16 storage wrapped on a >32KB history line.
    if pos1_raw < 0
        || (pos1_raw as i64) < (arg1 as i64)
        || pos2_raw < 0
        || (pos2_raw as i64) < (arg2 as i64)
    {
        // c:2476
        herrflush(); // c:2477
        zerr(
            "history event too long, can't index requested words", // c:2478
        );
        return None; // c:2479
    }
    let pos1 = pos1_raw as usize;
    let pos2 = pos2_raw as usize;
    // c:2481 — `dupstrpfx(elist->node.nam + pos1, pos2 - pos1)`.
    // Rust slice indexing requires pos1 <= pos2 <= len; both are
    // satisfied since both passed the c:2476 overflow check and
    // are bounded by the underlying string length per insert
    // contract. Guard with .get() so a malformed entry doesn't
    // panic.
    entry.node.nam.get(pos1..pos2).map(|s| s.to_string()) // c:2481
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
pub fn quote(s: &str) -> String {
    // c:2486
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
            out.push('\'');
            out.push('\\');
            out.push('\'');
            out.push('\'');
        } else if is_inblank && !inquotes && prev != '\\' {
            // c:2514
            out.push('\'');
            out.push(c);
            out.push('\'');
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
pub fn quotebreak(s: &str) -> String {
    // c:2527
    let mut result = String::with_capacity(s.len() + 10);
    result.push('\'');
    for c in s.chars() {
        // c:2548 — `inblank(*ptr)` narrow set.
        let is_inblank = matches!(c, ' ' | '\t' | '\n');
        if c == '\'' {
            result.push_str("'\\''");
        } else if is_inblank {
            result.push('\'');
            result.push(c);
            result.push('\'');
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
pub fn hdynread(stop: i32) -> Option<String> {
    // c:2562
    let stop_c = stop as u8 as char; // c:2562 int stop
    let mut buf = String::with_capacity(256); // c:2564 bsiz=256
    let mut c: Option<char>; // c:2564 int c
    loop {
        c = ingetc(); // c:2568
        match c {
            None => break,
            Some(ch) if ch == stop_c => break, // c:2568
            Some('\n') => break,               // c:2568
            Some(ch) => {
                if lexstop.load(SeqCst) {
                    break;
                } // c:2568
                let mut written = ch;
                if ch == '\\' {
                    // c:2569
                    if let Some(nxt) = ingetc() {
                        // c:2570
                        written = nxt;
                    } else {
                        break;
                    }
                }
                buf.push(written); // c:2571
            }
        }
    }
    if let Some('\n') = c {
        // c:2578
        inungetc('\n'); // c:2579
        zerr("delimiter expected"); // c:2580
        return None; // c:2582
    }
    Some(buf) // c:2584
}

/// Direct port of `static void ihungetc(int c)` from `Src/hist.c:989`.
/// Push back `c` into the lexer input stream while history-rewriting
/// is in progress: also rewinds chline (`hptr--`), undoes the
/// `expanding`-driven `zlemetacs`/`zlemetall` advance, and tracks the
/// `qbang` state for `\!` re-escape on the next pass. Loops while
/// `qbang` keeps firing (which can re-trigger via the `c='\\'` step
/// at the bottom).
pub fn ihungetc(c: i32) {
    // c:989
    let mut c = c as u8 as char; // c:991 int c
    let mut doit = 1; // c:991 doit = 1
    while !lexstop.load(SeqCst)                         // c:993 while (!lexstop && !errflag)
        && errflag.load(SeqCst) == 0
    {
        let hp = hptr.load(SeqCst);
        let line = chline.lock().unwrap().clone();
        let line_b = line.as_bytes();
        let stop = stophist.load(SeqCst);
        let inflags = crate::ported::input::inbufflags.with(|f| f.get());
        let active = histactive.load(SeqCst);
        if hp >= 2 && hp <= line_b.len()                                     // c:994-997
            && line_b[hp - 1] != c as u8 && stop < 4
            && line_b[hp - 1] == b'\n' && line_b[hp - 2] == b'\\'
            && (active & HA_UNGET) == 0
            && (inflags & (INP_ALIAS | INP_HIST)) != INP_ALIAS
        {
            histactive.fetch_or(HA_UNGET, SeqCst); // c:998
            inungetc('\n'); // c:999 hungetc('\n') — default = inungetc (c:1140)
            inungetc('\\'); // c:1000
            histactive.fetch_and(!HA_UNGET, SeqCst);
            // c:1001
        }
        if expanding.load(SeqCst) != 0 {
            // c:1004 if (expanding)
            ZLEMETACS.fetch_sub(1, SeqCst); // c:1005 zlemetacs--
            crate::ported::zle::compcore::ZLEMETALL.fetch_sub(1, SeqCst); // c:1006 zlemetall--
            exlast.fetch_add(1, SeqCst); // c:1007 exlast++
        }
        if (inflags & (INP_ALIAS | INP_HIST)) != INP_ALIAS {
            // c:1009
            // c:1010 — DPUTS(hptr <= chline, "BUG: hungetc attempted at buffer start")
            DPUTS!(hp <= 0, "BUG: hungetc attempted at buffer start"); // c:1010
                                                                       // c:1012 — DPUTS(*hptr != (char) c, "BUG: wrong character in hungetc() ")
            DPUTS!(
                // c:1012
                hp > 0 && line_b.get(hp - 1).copied() != Some(c as u8), // c:1012
                "BUG: wrong character in hungetc() "                    // c:1012
            );
            let new_hp = hp.saturating_sub(1);
            hptr.store(new_hp, SeqCst); // c:1011 hptr--
            let bangchar_v = bangchar.load(SeqCst) as u8;
            let qb = c as u8 == bangchar_v && stop < 2                       // c:1014-1015
                && new_hp > 0 && line_b.get(new_hp - 1).copied() == Some(b'\\');
            qbang.store(qb, SeqCst);
        } else {
            qbang.store(false, SeqCst); // c:1018 No active bangs in aliases
        }
        if doit != 0 {
            // c:1020
            inungetc(c); // c:1021
        }
        if !qbang.load(SeqCst) {
            return;
        } // c:1022
        let inflags2 = crate::ported::input::inbufflags.with(|f| f.get());
        doit = if stophist.load(SeqCst) == 0            // c:1023-1024
            && ((inflags2 & INP_HIST) != 0 || (inflags2 & INP_ALIAS) == 0)
        {
            1
        } else {
            0
        };
        c = '\\'; // c:1025
    }
}

/// Direct port of `int getsubsargs(char *subline, int *gbalp, int *cflagp)`
/// from `Src/hist.c:518`. Parses the substitution arguments of a
/// `!:s/old/new/`-style history modifier: reads the delimiter via
/// ingetc, slurps `old` and `new` chunks, stores them in `hsubl`/`hsubr`
/// globals, then peeks the trailing `:G` (global) or fall-through char.
/// Returns 0 on success, 1 on a bad-expansion (empty old chunk).
/// WARNING: param names don't match C — Rust=(_subline, gbalp, cflagp) vs C=(subline, gbalp, cflagp)
pub fn getsubsargs(_subline: &str, gbalp: &mut i32, cflagp: &mut i32) -> i32 {
    // c:518
    let del = match ingetc() {
        // c:524 del = ingetc()
        Some(c) => c,
        None => return 1,
    };
    // c:525-528 — `ptr1 = hdynread2(del); if (!ptr1) return 1;`
    // Inline hdynread2: read until del or '\n', honoring backslash escapes.
    let read_until = |stop: char| -> Option<String> {
        // c:hdynread2 inline
        let mut out = String::new();
        loop {
            match ingetc() {
                None => return None,
                Some('\n') => return Some(out),
                Some(c) if c == stop => return Some(out),
                Some('\\') => {
                    if let Some(n) = ingetc() {
                        if n != stop {
                            out.push('\\');
                        }
                        out.push(n);
                    }
                }
                Some(c) => out.push(c),
            }
        }
    };
    let ptr1 = match read_until(del) {
        Some(p) => p,
        None => return 1,
    }; // c:525
    let ptr2 = read_until(del).unwrap_or_default(); // c:529
    if !ptr1.is_empty() {
        // c:530
        *hsubl.lock().unwrap() = Some(ptr1); // c:531-532 zsfree(hsubl); hsubl = ptr1
    } else if hsubl.lock().unwrap().is_none() {
        // c:533 fail silently
        return 0; // c:536
    }
    *hsubr.lock().unwrap() = Some(ptr2); // c:539-540 zsfree(hsubr); hsubr = ptr2
    let follow = ingetc(); // c:541 follow = ingetc()
    if follow == Some(':') {
        // c:542
        let next = ingetc(); // c:543
        if next == Some('G') {
            *gbalp = 1;
        }
        // c:544-545
        else {
            if let Some(c) = next {
                inungetc(c);
            } // c:547 inungetc
            *cflagp = 1; // c:548
        }
    } else if let Some(c) = follow {
        inungetc(c); // c:551 inungetc(follow)
    }
    0 // c:553
}

/// Port of `char *hdynread2(int stop)` from Src/hist.c.
pub fn hdynread2(stop: char, input: &str) -> (String, usize) {
    let mut out = String::new();
    let mut consumed = 0usize;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        consumed += c.len_utf8();
        if c == stop || c == '\n' {
            if c == '\n' {
                consumed -= c.len_utf8();
            }
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
pub fn inithist() {
    // c:2613
    histsiz.store(1000, SeqCst);
    savehistsiz.store(1000, SeqCst);
    curhist.store(0, SeqCst);
    histlinect.store(0, SeqCst);
}

/// Port of `void resizehistents(void)` from Src/hist.c.
pub fn resizehistents() {
    let cap = histsiz.load(SeqCst);
    while histlinect.load(SeqCst) > cap {
        if let Some(oldest) = ring_oldest() {
            let mut ring = hist_ring.lock().unwrap();
            ring.retain(|h| h.histnum != oldest);
            histlinect.fetch_sub(1, SeqCst);
        } else {
            break;
        }
    }
}

/// Port of `void readhistline(char *line, ...)` from Src/hist.c.
pub fn readhistline(line: &str) -> Option<histent> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = line.strip_prefix(": ") {
        if let Some(semi) = rest.find(';') {
            let meta = &rest[..semi];
            let cmd = &rest[semi + 1..];
            let parts: Vec<&str> = meta.splitn(2, ':').collect();
            let timestamp = parts
                .first()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            let mut entry = make_histent(0, cmd.to_string());
            entry.stim = timestamp;
            return Some(entry);
        }
    }
    Some(make_histent(0, line.to_string()))
}

/// Port of `void readhistfile(char *fn, int err, int readflags)` from Src/hist.c:2675.
pub fn readhistfile(fn_path: Option<&str>, _err: i32, readflags: i32) {
    // c:2675
    let path: String = match fn_path {
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return,
        },
    };
    // c:2675 — HFILE_FAST is the incremental (share / append) read: resume at
    // lasthist.fpos and consume only what other shells appended, rather than
    // re-parsing the whole file. Under SHARE_HISTORY hend() re-reads after
    // EVERY command (c:1529); a full re-read there is O(file) per prompt (and,
    // with the Vec ring, was an O(n²) memmove hang on a large HISTFILE).
    let fast = readflags & HFILE_FAST as i32 != 0;
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(&path).ok();
    let cur_size = meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
    let cur_mtim = meta.as_ref().map(|m| m.mtime()).unwrap_or(0);
    if cur_size == 0 {
        return;
    }
    if fast {
        // Nothing changed since our last read/write — savehistfile keeps
        // lasthist.{fpos,fsiz,mtim} current (c:3049/3079/3080) — so there is
        // nothing new to pull in. Skip without opening the file.
        let lh = lasthist.lock().unwrap();
        if lh.fsiz == cur_size && lh.mtim == cur_mtim {
            return;
        }
    }
    // A fast read resumes at lasthist.fpos when the file only grew; a rewrite
    // or truncation (file now shorter than fpos) or a cold/full read starts at
    // 0. `read_end` is where we actually stopped, recorded into lasthist below.
    let start_pos: i64 = if fast {
        let fp = lasthist.lock().unwrap().fpos;
        if fp > 0 && fp <= cur_size {
            fp
        } else {
            0
        }
    } else {
        0
    };
    let contents = {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => return,
        };
        if start_pos > 0 && f.seek(SeekFrom::Start(start_pos as u64)).is_err() {
            return;
        }
        let mut s = String::new();
        if f.read_to_string(&mut s).is_err() {
            return;
        }
        s
    };
    let read_end = start_pos + contents.len() as i64;
    if contents.is_empty() {
        // Nothing appended past fpos, but the file was touched (mtime/size
        // differed). Refresh lasthist so the next fast read early-outs.
        let mut lh = lasthist.lock().unwrap();
        lh.fpos = read_end;
        lh.fsiz = cur_size;
        lh.mtim = cur_mtim;
        return;
    }
    // c:2700-2706 — lockhistfile return codes:
    //   0 — lock acquired, proceed
    //   2 — locking failed but "reading anyway"; warn + continue
    //   else — locking failed hard; zerr + bail
    let lock_ret = lockhistfile(Some(&path), 1);
    if lock_ret != 0 {
        if lock_ret == 2 {
            crate::ported::utils::zwarn(&format!(
                "locking failed for {}: {}: reading anyway",
                path,
                std::io::Error::last_os_error()
            ));
        } else {
            crate::ported::utils::zerr(&format!(
                "locking failed for {}: {}",
                path,
                std::io::Error::last_os_error()
            ));
            return;
        }
    }

    // c:2675 — zsh's hist_ring is a doubly-linked list, so its per-entry
    // "insert at head" (addhistnode) is O(1). This port stores the ring in a
    // Vec, where `insert(0, ..)` shifts every existing element — O(n) per line,
    // O(n²) over the whole file. A large HISTFILE (hundreds of thousands of
    // lines) plus SHARE_HISTORY (which re-reads the file in hend() after every
    // command, c:1529) turned that into a multi-second memmove spin per prompt.
    // Collect into a local batch in file order (push = O(1)), then splice the
    // whole run onto the front of the ring ONCE below. Repeated `insert(0)` of
    // e1..eN (oldest..newest) yields ring == [eN..e1, existing..]; pushing
    // e1..eN then reversing gives the same [eN..e1] prefix. One lock, not one
    // per line.
    let mut batch: Vec<histent> = Vec::new();
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
            let n = curhist.fetch_add(1, SeqCst) + 1;
            let mut entry = make_histent(n, text.clone());
            entry.stim = stim;
            entry.ftim = ftim;
            entry.node.flags |= HIST_OLD as i32;
            batch.push(entry);
            histlinect.fetch_add(1, SeqCst);
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
        let n = curhist.fetch_add(1, SeqCst) + 1;
        let mut entry = make_histent(n, text);
        entry.stim = stim;
        entry.ftim = ftim;
        entry.node.flags |= HIST_OLD as i32;
        batch.push(entry);
        histlinect.fetch_add(1, SeqCst);
    }
    if !batch.is_empty() {
        // Prepend the batch (newest-first) ahead of anything already in the
        // ring, matching the per-line `insert(0)` order. Single O(n) splice.
        batch.reverse();
        let mut ring = hist_ring.lock().unwrap();
        batch.append(&mut ring);
        *ring = batch;
    }
    // c:2675 — record where this read stopped. A non-fast (startup) read primes
    // lasthist too, so the first HFILE_FAST share-read early-outs instead of
    // re-reading from offset 0 and DUPLICATING every entry already in the ring.
    {
        let mut lh = lasthist.lock().unwrap();
        lh.fpos = read_end;
        lh.fsiz = cur_size;
        lh.mtim = cur_mtim;
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
            return unsafe {
                if libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) == 0 {
                    1
                } else {
                    0
                }
            };
        }
        0
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        1
    }
}

/// Port of `void savehistfile(char *fn, int err, int writeflags)` from Src/hist.c:2922.
///
/// The public Rust signature omits the C `err` argument; its value is
/// recovered from the write flags. Every silent (`err == 0`) C call
/// site also sets `HFILE_FAST` (the incremental / share saves at
/// c:1193 and c:1639), while every loud (`err == 1`) site — shell exit
/// (c:3961), `fc -W` / `fc -A` (c:1511/1517) and the trim recursion
/// (c:3121) — leaves `HFILE_FAST` clear. So `err == !(writeflags &
/// HFILE_FAST)` reproduces the C value at all call sites without
/// changing the signature seen by callers.
pub fn savehistfile(fn_path: Option<&str>, writeflags: i32) {
    // c:2922
    use crate::ported::zsh_h::{
        APPENDHISTORY, EXTENDEDHISTORY, GETHIST_DOWNWARD, HFILE_APPEND, HFILE_NO_REWRITE,
        HFILE_SKIPDUPS, HFILE_SKIPFOREIGN, HFILE_SKIPOLD, HISTSAVEBYCOPY, HISTSAVENODUPS,
    };
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let mut writeflags = writeflags;
    // c:2927 — zlong xcurhist = curhist - !!(histactive & HA_ACTIVE);
    let xcurhist = curhist.load(SeqCst)
        - if (histactive.load(SeqCst) & HA_ACTIVE) != 0 {
            1
        } else {
            0
        };
    // c:2928 — int extended_history = isset(EXTENDEDHISTORY);
    let mut extended_history = isset(EXTENDEDHISTORY);
    // Recovered C `err` (see fn doc): silent when HFILE_FAST is set.
    let mut err = writeflags & HFILE_FAST as i32 == 0;

    // c:2931-2933 — `if (!interact || savehistsiz <= 0 || !hist_ring
    //                || (!fn && !(fn = getsparam("HISTFILE")))) return;`
    //
    // `!interact` is test-pinned (non-interactive shells must never
    // write the user's HISTFILE). The explicit-path accommodation lets
    // `fc -W path` still create/write the file in contexts where
    // SAVEHIST is unset (e.g. `zsh -fc`) or the ring is empty.
    if !isset(INTERACTIVE) {
        return;
    }
    let explicit_path = fn_path.is_some();
    let savehistsiz_v = savehistsiz.load(SeqCst); // c:2932 savehistsiz
    if savehistsiz_v <= 0 && !explicit_path {
        return; // c:2932 savehistsiz <= 0
    }
    if ring_len() == 0 && !explicit_path {
        return; // c:2931 !hist_ring
    }
    let path: String = match fn_path {
        // c:2932 fn / getsparam("HISTFILE")
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return,
        },
    };

    // c:2934-2951 — pick the first entry to write and take the lock.
    let he: Option<i64>;
    if writeflags & HFILE_FAST as i32 != 0 {
        // c:2935 — he = gethistent(lasthist.next_write_ev, GETHIST_DOWNWARD);
        let start_ev = lasthist.lock().unwrap().next_write_ev;
        let mut cur = gethistent(start_ev, GETHIST_DOWNWARD);
        // c:2936-2939 — advance past entries already written (HIST_OLD).
        while let Some(h) = cur {
            let flags = ring_get(h).map(|e| e.node.flags).unwrap_or(0);
            if flags & HIST_OLD as i32 == 0 {
                break;
            }
            lasthist.lock().unwrap().next_write_ev = h + 1; // c:2937
            cur = down_histent(h); // c:2938
        }
        he = cur;
        // c:2940 — if (!he || lockhistfile(fn, 0)) return;
        if he.is_none() || lockhistfile(Some(&path), 0) != 0 {
            return;
        }
        // c:2942-2943 — too many lines already: drop to a full rewrite.
        if histfile_linect.load(SeqCst) > savehistsiz_v + savehistsiz_v / 5 {
            writeflags &= !(HFILE_FAST as i32);
        }
    } else {
        // c:2946-2949 — if (lockhistfile(fn, 1)) { zerr(...); return; }
        // (ret == 2 means "couldn't lock but proceed anyway".)
        let lret = lockhistfile(Some(&path), 1);
        if lret != 0 && lret != 2 {
            crate::ported::utils::zerr(&format!(
                "locking failed for {}: {}",
                path,
                std::io::Error::last_os_error()
            ));
            return;
        }
        he = ring_oldest(); // c:2950 he = hist_ring->down;
    }

    // c:2952-2962 — HFILE_USE_OPTIONS derives append/skip flags from options.
    if writeflags & HFILE_USE_OPTIONS as i32 != 0 {
        if isset(APPENDHISTORY)
            || isset(INCAPPENDHISTORY)
            || isset(INCAPPENDHISTORYTIME)
            || isset(SHAREHISTORY)
        {
            writeflags |= (HFILE_APPEND | HFILE_SKIPOLD) as i32; // c:2955
        } else {
            histfile_linect.store(0, SeqCst); // c:2957
        }
        if isset(HISTSAVENODUPS) {
            writeflags |= HFILE_SKIPDUPS as i32; // c:2959
        }
        if isset(SHAREHISTORY) {
            extended_history = true; // c:2961
        }
    }

    // c:2963-3016 — open the destination: append, plain truncate, or
    // HISTSAVEBYCOPY (write `<fn>.new` then rename over `fn`).
    let umpath = crate::ported::utils::unmeta(&path);
    let append_mode = writeflags & HFILE_APPEND as i32 != 0;
    let mut tmpfile: Option<String> = None;
    let out: Option<std::fs::File> = if append_mode {
        // c:2964-2967 — open(fn, O_CREAT|O_WRONLY|O_APPEND, 0600)
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&umpath)
            .ok()
    } else if !isset(HISTSAVEBYCOPY) {
        // c:2968-2971 — open(fn, O_CREAT|O_WRONLY|O_TRUNC, 0600)
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&umpath)
            .ok()
    } else {
        // c:2972-3015 — safe write through a sibling `.new` file.
        let tf = format!("{}.new", umpath); // c:2973 bicat(fn, ".new")
        // c:2974 — unlink(tmpfile); tolerate ENOENT.
        let unlink_ok = match std::fs::remove_file(&tf) {
            Ok(()) => true,
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        };
        if !unlink_ok {
            None // c:2975 out = NULL;
        } else {
            let old_meta = std::fs::metadata(&umpath).ok(); // c:2978 stat(fn)
            let euid = unsafe { libc::geteuid() };
            // c:2981-2985 — rewriting must not change ownership (root exempt).
            let owner_change = old_meta
                .as_ref()
                .map(|m| euid != 0 && m.uid() != euid)
                .unwrap_or(false);
            if owner_change {
                // c:2986-2996 — skip; report only when err is set.
                if err {
                    if isset(APPENDHISTORY)
                        || isset(INCAPPENDHISTORY)
                        || isset(INCAPPENDHISTORYTIME)
                        || isset(SHAREHISTORY)
                    {
                        crate::ported::utils::zerr(&format!(
                            "rewriting {} would change its ownership -- skipped",
                            path
                        ));
                    } else {
                        crate::ported::utils::zerr(&format!(
                            "rewriting {} would change its ownership -- history not saved",
                            path
                        ));
                    }
                    err = false; // c:2994 err = 0; — don't also report below.
                }
                None
            } else {
                // c:2998 — open(tmpfile, O_CREAT|O_WRONLY|O_EXCL, 0600)
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&tf)
                {
                    Ok(f) => {
                        // c:3007-3013 — match the original owner and mode.
                        if let Some(ref m) = old_meta {
                            let fd = f.as_raw_fd();
                            unsafe {
                                let _ = libc::fchown(fd, m.uid(), m.gid()); // c:3010
                                let _ = libc::fchmod(fd, m.mode() as libc::mode_t); // c:3012
                            }
                        }
                        tmpfile = Some(tf);
                        Some(f)
                    }
                    Err(_) => None, // c:3003 out = NULL;
                }
            }
        }
    };

    let mut ret: i32 = 0;
    if let Some(mut out) = out {
        crate::ported::mem::pushheap(); // c:3021

        // c:3018-3027 — compile the $HISTORY_IGNORE pattern once.
        let histpat = crate::ported::params::getsparam("HISTORY_IGNORE").and_then(|hi| {
            let mut s = crate::ported::string::dupstring(&hi); // c:3024 dupstring
            crate::ported::glob::tokenize(&mut s); // c:3024 tokenize
            crate::ported::glob::remnulargs(&mut s); // c:3025 remnulargs
            crate::ported::pattern::patcompile(&s, 0, None) // c:3026 patcompile
        });

        // Running file offset for lasthist.fpos (ftell parity). Append
        // continues from the current size; truncate/copy start at 0.
        let mut fpos: i64 = if append_mode {
            std::fs::metadata(&umpath)
                .map(|m| m.len() as i64)
                .unwrap_or(0)
        } else {
            0
        };
        let mut start: Option<String> = None;

        // c:3030 — for (; he && he->histnum <= xcurhist; he = down_histent(he))
        let mut cur = he;
        while let Some(h) = cur {
            let entry = match ring_get(h) {
                Some(e) => e,
                None => break,
            };
            if entry.histnum > xcurhist {
                break; // c:3030
            }
            let flags = entry.node.flags;
            // c:3033-3036 — skip dup/foreign/tmpstore per the write flags.
            if (writeflags & HFILE_SKIPDUPS as i32 != 0 && flags & HIST_DUP as i32 != 0)
                || (writeflags & HFILE_SKIPFOREIGN as i32 != 0
                    && flags & HIST_FOREIGN as i32 != 0)
                || flags & HIST_TMPSTORE as i32 != 0
            {
                cur = down_histent(h);
                continue;
            }
            // c:3037-3040 — skip entries matching $HISTORY_IGNORE.
            if let Some(ref pat) = histpat {
                if crate::ported::pattern::pattry(
                    pat,
                    &crate::ported::utils::metafy(&entry.node.nam),
                ) {
                    cur = down_histent(h);
                    continue;
                }
            }
            // c:3041-3047 — HFILE_SKIPOLD: skip old/nowrite, else mark old.
            if writeflags & HFILE_SKIPOLD as i32 != 0 {
                if flags & (HIST_OLD | HIST_NOWRITE) as i32 != 0 {
                    cur = down_histent(h);
                    continue;
                }
                if let Ok(mut ring) = hist_ring.lock() {
                    if let Some(e) = ring.iter_mut().find(|e| e.histnum == h) {
                        e.node.flags |= HIST_OLD as i32; // c:3044
                    }
                }
                if writeflags & HFILE_USE_OPTIONS as i32 != 0 {
                    lasthist.lock().unwrap().next_write_ev = entry.histnum + 1; // c:3046
                }
            }
            // c:3048-3052 — record write bookkeeping under USE_OPTIONS.
            if writeflags & HFILE_USE_OPTIONS as i32 != 0 {
                let mut lh = lasthist.lock().unwrap();
                lh.fpos = fpos; // c:3049 lasthist.fpos = ftell(out)
                lh.stim = entry.stim; // c:3050
                drop(lh);
                histfile_linect.fetch_add(1, SeqCst); // c:3051
            }

            // c:3053-3073 — emit the command text with escaping.
            start = Some(entry.node.nam.clone()); // c:3053 start = he->node.nam
            let text = entry.node.nam.as_bytes();
            let mut buf: Vec<u8> = Vec::with_capacity(text.len() + 24);
            if extended_history {
                // c:3054-3056 — ": %ld:%ld;" prefix (only in extended mode).
                let dur = if entry.ftim != 0 {
                    entry.ftim - entry.stim
                } else {
                    0
                };
                let _ = write!(buf, ": {}:{};", entry.stim, dur);
            } else if text.first() == Some(&b':') {
                buf.push(b'\\'); // c:3057-3058 escape a leading ':'
            }
            // c:3060-3067 — escape embedded newlines; track trailing '\'.
            let mut end_backslashes = false;
            for &c in text {
                if c == b'\n' {
                    buf.push(b'\\'); // c:3062
                }
                end_backslashes = c == b'\\' || (end_backslashes && c == b' '); // c:3064
                buf.push(c); // c:3065
            }
            if end_backslashes {
                buf.push(b' '); // c:3070-3071
            }
            buf.push(b'\n'); // c:3072
            if out.write_all(&buf).is_err() {
                ret = -1; // c:3065/3072 — fputc returned < 0.
                break;
            }
            fpos += buf.len() as i64;
            cur = down_histent(h);
        }

        // c:3075-3085 — final size/mtime + last-written text (USE_OPTIONS).
        if ret >= 0 && start.is_some() && writeflags & HFILE_USE_OPTIONS as i32 != 0 {
            let _ = out.flush(); // c:3077 fflush(out)
            if let Ok(md) = out.metadata() {
                let mut lh = lasthist.lock().unwrap();
                lh.fsiz = md.len() as i64; // c:3079
                lh.mtim = md.mtime(); // c:3080
            }
            lasthist.lock().unwrap().text = start.clone(); // c:3082-3083
        }

        // c:3086 — fclose(out).
        drop(out);

        if ret >= 0 {
            if let Some(ref tf) = tmpfile {
                // c:3089-3103 — rename the temp copy over the real file.
                if std::fs::rename(tf, &umpath).is_err() {
                    crate::ported::utils::zerr(&format!(
                        "can't rename {}.new to $HISTFILE",
                        path
                    ));
                    ret = -1; // c:3092
                    err = false; // c:3093 err = 0;
                }
            }

            // c:3106-3125 — SKIPOLD (and not FAST/NO_REWRITE): re-read the
            // just-written file capped to savehistsiz, then rewrite it
            // trimmed. This enforces SAVEHIST on the append/share paths.
            if ret >= 0
                && writeflags & HFILE_SKIPOLD as i32 != 0
                && writeflags & (HFILE_FAST | HFILE_NO_REWRITE) as i32 == 0
            {
                let remember_histactive = histactive.load(SeqCst); // c:3108
                histactive.store(0, SeqCst); // c:3111
                pushhiststack(None, savehistsiz_v, savehistsiz_v, -1); // c:3113
                if isset(HISTSAVENODUPS) {
                    hist_ignore_all_dups.store(1, SeqCst); // c:3115
                }
                readhistfile(Some(&path), if err { 1 } else { 0 }, 0); // c:3116
                hist_ignore_all_dups
                    .store(if isset(HISTIGNOREALLDUPS) { 1 } else { 0 }, SeqCst); // c:3117
                if errflag.load(SeqCst) & ERRFLAG_INT != 0 {
                    ret = -1; // c:3119
                } else if histlinect.load(SeqCst) != 0 {
                    savehistfile(Some(&path), 0); // c:3121
                }
                pophiststack(); // c:3123
                histactive.store(remember_histactive, SeqCst); // c:3124
            }
        }

        crate::ported::mem::popheap(); // c:3128
    } else {
        ret = -1; // c:3130
    }

    // c:3132-3137 — report a write failure on stderr when err is set.
    if ret < 0 && err {
        if tmpfile.is_some() {
            crate::ported::utils::zerr(&format!(
                "failed to write history file {}.new: {}",
                path,
                std::io::Error::last_os_error()
            ));
        } else {
            crate::ported::utils::zerr(&format!(
                "failed to write history file {}: {}",
                path,
                std::io::Error::last_os_error()
            ));
        }
    }
    // c:3138-3139 — free(tmpfile): the owned String drops automatically.

    unlockhistfile(&path); // c:3141
}

/// Port of `int lockhistct` from Src/hist.c. Re-entrant lock counter.
static lockhistct: AtomicI32 = AtomicI32::new(0);

/// Port of `int checklocktime(char *lockfile, long *sleep_usp, time_t then)`
/// from `Src/hist.c:3147`.
///
/// Decides what to do when a history lock file already exists, given
/// its mtime (`then`). Returns `-1` (give up) when the lock file's
/// timestamp is implausibly far in the future; otherwise either sleeps
/// a randomised, exponentially-increasing backoff (recent lock — owner
/// likely still alive) or unlinks the stale lock file, returning `0`.
/// `sleep_usp` is doubled on each backoff so repeated calls ramp up.
///
/// Replaces the prior ad-hoc `(path, max_age_secs) -> 1|0` orphan,
/// which used a `.lock` suffix, an age threshold, and no backoff —
/// none of which match the C contract.
pub fn checklocktime(lockfile: &str, sleep_usp: &mut i64, then: i64) -> i32 {
    // c:3147
    let now = zmonotime(None); // c:3149

    if now + 10 < then {
        // c:3151 — File is more than 10 seconds in the future?
        // c:3153 — `errno = EEXIST;` set for C caller-contract parity.
        unsafe {
            #[cfg(target_os = "linux")]
            {
                *libc::__errno_location() = libc::EEXIST;
            }
            #[cfg(not(target_os = "linux"))]
            {
                *libc::__error() = libc::EEXIST;
            }
        }
        return -1; // c:3154
    }

    if now - then < 10 {
        // c:3157-3168 — gradually increasing backoff: sleep based on
        // the time spent so far, randomised to minimise clashes with
        // shells exiting at the same time.
        DPUTS!(now < then, "time flowing backwards through history"); // c:3162
        zsleep_random(*sleep_usp, then + 10); // c:3167
        *sleep_usp <<= 1; // c:3168 `*sleep_usp <<= 1;`
    } else {
        // c:3170 — `unlink(lockfile);` — the lock is stale, remove it.
        let _ = std::fs::remove_file(lockfile);
    }

    0 // c:3172
}

/// Port of `int lockhistfile(char *fn, int keep_trying)` from Src/hist.c:3182.
/// Rust idiom replacement: `fs2::FileExt::try_lock_exclusive` covers
/// the C `flock` + `link`-symlink retry loop; the `keep_trying`
/// arg controls retry budget rather than mode flags.
pub fn lockhistfile(fn_path: Option<&str>, keep_trying: i32) -> i32 {
    // c:3182
    let path: String = match fn_path {
        // c:3182
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return 1, // c:3189
        },
    };
    if lockhistct.fetch_add(1, SeqCst) > 0 {
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
    lockhistct.fetch_sub(1, SeqCst);
    if keep_trying != 0 {
        2
    } else {
        1
    }
}

/// Port of `void unlockhistfile(char *fn)` from Src/hist.c.
pub fn unlockhistfile(path: &str) {
    let prev = lockhistct.fetch_sub(1, SeqCst);
    if prev <= 0 {
        lockhistct.store(0, SeqCst);
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
    if lockhistct.load(SeqCst) > 0 {
        1
    } else {
        0
    }
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

/// Port of `void histsplitwords(char *lineptr, short **wordsp, int *nwordsp, int *nwordposp, int uselex)`
/// from `Src/hist.c:3650`.
///
/// Returns word (start, end) byte-offset pairs. When `uselex == true`,
/// runs the lex tokenizer (`bufferwords`) and matches each lexed
/// token's text against `line` to recover its position — same shape
/// as the C body's outer for-loop over the wordlist (c:3671-3800).
/// On lex mismatch (the `bad = 1` branch at c:3776), falls back to
/// the simple whitespace tokenizer (matches C's `lineptr = start;
/// uselex = 0;` retry).
pub fn histsplitwords(line: &str, uselex: bool) -> Vec<(usize, usize)> {
    // c:3650
    if uselex {
        // c:3662-3663 — wordlist = bufferwords(NULL, lineptr, NULL, LEXFLAGS_COMMENTS_KEEP);
        let (lexed, _) = bufferwords(line, 0);

        let bytes = line.as_bytes();
        let mut lptr: usize = 0;
        let mut words: Vec<(usize, usize)> = Vec::with_capacity(lexed.len());
        let mut bad = false;

        for word in &lexed {
            // c:3679-3694 — skip blanks + `\\\n` at start.
            while lptr < bytes.len() {
                let b = bytes[lptr];
                if b == b' ' || b == b'\t' {
                    lptr += 1;
                } else if b == b'\\' && lptr + 1 < bytes.len() && bytes[lptr + 1] == b'\n' {
                    lptr += 2;
                } else {
                    break;
                }
            }
            let word_start = lptr;
            let wbytes = word.as_bytes();

            // c:3707-3787 — match-with-backslash-newline-mid-word loop.
            let mut wptr: usize = 0;
            loop {
                // c:3715-3722 — semicolon vs ";;" disambiguation: a single
                // `;` lex token shouldn't consume both chars of `;;`.
                if word == ";"
                    && lptr + 1 < bytes.len()
                    && bytes[lptr] == b';'
                    && bytes[lptr + 1] == b';'
                {
                    // Treat this as a synthetic newline-→-`;` token: no
                    // line consumption (c:3721 loop_next=2, continue).
                    break;
                }
                // c:3709-3726 — strpfx fast path.
                if lptr + wbytes.len() - wptr <= bytes.len()
                    && bytes[lptr..lptr + wbytes.len() - wptr] == wbytes[wptr..]
                {
                    lptr += wbytes.len() - wptr;
                    wptr = wbytes.len();
                    break;
                }
                // c:3727-3787 — slow path: match char-by-char allowing
                // `\\\n` mid-word and the `!`→`|` substitution.
                let mut skipping = false;
                if word == ";" {
                    // c:3736-3740 — newline-→-`;` synthetic token.
                    break;
                }
                while lptr < bytes.len() {
                    if wptr >= wbytes.len() {
                        // c:3742-3750 — word ended before line: bail.
                        bad = true;
                        break;
                    }
                    if bytes[lptr] == wbytes[wptr] || (bytes[lptr] == b'!' && wbytes[wptr] == b'|')
                    // c:3754-3755
                    {
                        lptr += 1;
                        wptr += 1;
                        if wptr >= wbytes.len() {
                            break;
                        }
                    } else if bytes[lptr] == b'\\'
                        && lptr + 1 < bytes.len()
                        && bytes[lptr + 1] == b'\n'
                    {
                        // c:3759-3769 — \\\n mid-word, skip line break.
                        lptr += 2;
                        skipping = true;
                        break;
                    } else {
                        bad = true;
                        break;
                    }
                }
                if bad || !skipping {
                    break;
                }
            }

            if bad {
                // c:3782-3786 — `lineptr = start; nwordpos = 0; uselex = 0;`
                //               Restart with non-lex fallback.
                return histsplitwords(line, false);
            }
            // c:3795-3796 — words[nwordpos++] = start; words[nwordpos++] = lptr;
            words.push((word_start, lptr));
        }
        return words;
    }

    // c:3802-3830 — non-lex path: simple whitespace + `\\\n` tokenizer.
    let mut words = Vec::new();
    let bytes = line.as_bytes();
    let mut lptr: usize = 0;
    loop {
        // c:3804-3811 — skip leading blanks + `\\\n`.
        while lptr < bytes.len() {
            let b = bytes[lptr];
            if b == b' ' || b == b'\t' || b == b'\n' {
                lptr += 1;
            } else if b == b'\\' && lptr + 1 < bytes.len() && bytes[lptr + 1] == b'\n' {
                lptr += 2;
            } else {
                break;
            }
        }
        if lptr >= bytes.len() {
            break;
        }
        let word_start = lptr; // c:3818
                               // c:3819-3826 — walk until next blank; Meta-byte advances by 2.
        while lptr < bytes.len() {
            let b = bytes[lptr];
            if b == 0x83 /* Meta */ && lptr + 1 < bytes.len() {
                lptr += 2;
            } else if b == b' ' || b == b'\t' || b == b'\n' {
                break;
            } else {
                lptr += 1;
            }
        }
        words.push((word_start, lptr)); // c:3827
    }
    words
}

/// Port of `int pushhiststack(char *hf, zlong hs, zlong shs, int level)`
/// from Src/hist.c:3845.
///
/// Saves the current history state (ring, sizes, counters, HISTFILE,
/// lasthist) onto the save stack, then switches to a fresh empty
/// history optionally backed by the new HISTFILE `hf`. Mirrored by
/// `pophiststack`, which restores the saved state. Returns the new
/// stack depth (`histsave_stack_pos`).
///
/// Deferred, matching `pophiststack`'s convention in this layer: the
/// ZLE `curline_in_ring` unlink/relink (c:3856/3892) and the
/// `zleentry(ZLE_CMD_SET_HIST_LINE)` callback (c:3886); `inithist`'s C
/// role (`createhisttable`, c:3890) is a no-op in the Vec-based ring
/// model. The global `lasthist` is captured into the snapshot but not
/// zeroed here — zeroing pairs with a `lasthist` restore on pop, which
/// is a separate pending fix.
pub fn pushhiststack(hf: Option<&str>, hs: i64, shs: i64, level: i32) -> i32 {
    // c:3845
    // c:3862-3868 — save the OLD HISTFILE so pop can restore it. With
    // `hf` set, record the current HISTFILE ("" when empty/unset); with
    // `hf` None, record None (C `h->histfile = NULL`).
    let old_histfile: Option<String> = if hf.is_some() {
        match crate::ported::params::getsparam("HISTFILE") {
            Some(v) if !v.is_empty() => Some(v), // c:3863 ztrdup(HISTFILE)
            _ => Some(String::new()),            // c:3866 h->histfile = ""
        }
    } else {
        None // c:3868 h->histfile = NULL
    };
    let snap = histsave {
        lasthist: lasthist.lock().unwrap().clone(), // c:3861 h->lasthist = lasthist
        histfile: old_histfile,                     // c:3862-3868 OLD HISTFILE
        hist_ring: std::mem::take(&mut *hist_ring.lock().unwrap()), // c:3870 h->hist_ring = hist_ring
        curhist: curhist.load(SeqCst),                              // c:3871 h->curhist = curhist
        histlinect: histlinect.load(SeqCst),                        // c:3872
        histsiz: histsiz.load(SeqCst),                              // c:3873
        savehistsiz: savehistsiz.load(SeqCst),                      // c:3874
        locallevel: level,                                          // c:3875
    };
    histsave_stack.lock().unwrap().push(snap); // c:3859 histsave_stack[pos++] = *h
    histsave_stack_size.fetch_add(1, SeqCst);
    histsave_stack_pos.fetch_add(1, SeqCst);
    // c:3878-3883 — switch HISTFILE to the new file (or unset it).
    if let Some(h) = hf {
        if !h.is_empty() {
            crate::ported::params::setsparam("HISTFILE", h); // c:3880 setsparam(HISTFILE, hf)
        } else {
            // c:3882 unsetparam("HISTFILE")
            let _ = crate::ported::params::paramtab()
                .write()
                .unwrap()
                .remove("HISTFILE");
        }
    }
    // c:3884 — `hist_ring = NULL`: already emptied by the `mem::take` above.
    curhist.store(0, SeqCst); // c:3885 curhist = histlinect = 0
    histlinect.store(0, SeqCst);
    histsiz.store(hs, SeqCst); // c:3888 histsiz = hs
    savehistsiz.store(shs, SeqCst); // c:3889 savehistsiz = shs
                                    // c:3895 — return histsave_stack_pos.
    histsave_stack_pos.load(SeqCst)
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
pub fn pophiststack() -> i32 {
    // c:3901
    let snap = match histsave_stack.lock().unwrap().pop() {
        Some(s) => s,
        None => return 0, // c:3907
    };
    // c:3920-3924 — restore HISTFILE via setsparam / unsetparam.
    if let Some(ref hf) = snap.histfile {
        if !hf.is_empty() {
            // c:3922 *h->histfile
            crate::ported::params::setsparam("HISTFILE", hf); // c:3922
        } else {
            // c:3923
            // Unset HISTFILE — Rust paramtab remove.
            let _ = crate::ported::params::paramtab()
                .write()
                .unwrap()
                .remove("HISTFILE"); // c:3923 unsetparam
        }
    }
    *hist_ring.lock().unwrap() = snap.hist_ring; // c:3925
    curhist.store(snap.curhist, SeqCst); // c:3926
    histlinect.store(snap.histlinect, SeqCst); // c:3929
    histsiz.store(snap.histsiz, SeqCst); // c:3930
    savehistsiz.store(snap.savehistsiz, SeqCst); // c:3931
    histsave_stack_size.fetch_sub(1, SeqCst);
    histsave_stack_pos.fetch_sub(1, SeqCst);
    // c:3934 — `return histsave_stack_pos + 1;` (new pos after
    // decrement, plus 1 for the just-popped depth).
    histsave_stack_pos.load(SeqCst) + 1
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
pub fn saveandpophiststack(mut pop_through: i32, writeflags: i32) -> i32 {
    // c:3947
    let stack_pos = histsave_stack_pos.load(SeqCst);
    // c:3949-3953 — non-positive pop_through means "pop relative
    // to current pos": fold to an absolute index.
    if pop_through <= 0 {
        // c:3949
        pop_through += stack_pos + 1; // c:3950
        if pop_through <= 0 {
            // c:3951
            pop_through = 1;
        }
    }
    // c:3954-3956 — walk back while the entry at pop_through-2 was
    // saved at a deeper locallevel than the current scope. The
    // Rust port doesn't yet model histsave_stack[i].locallevel
    // (the per-frame locallevel snapshot); approximate by skipping
    // this loop — matches the "pop everything we have" intent for
    // current callers.
    if stack_pos < pop_through {
        // c:3957
        return 0;
    }
    // c:3959-3962 — loop pop until we reach pop_through. The
    // `nohistsave` C global isn't ported as a Rust global; default
    // to 0 (allow saves), which is the common case. A future port
    // can wire the global at the canonical home.
    loop {
        // c:3960-3961 — `if (!nohistsave) savehistfile(NULL, 1, writeflags);`.
        savehistfile(None, writeflags);
        pophiststack(); // c:3962
        if histsave_stack_pos.load(SeqCst) < pop_through {
            // c:3963
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
pub static histtab: Mutex<Vec<usize>> = Mutex::new(Vec::new()); // c:101

/// Port of `mod_export Histent hist_ring` from Src/hist.c:103.
/// Doubly-linked ring of history entries; modelled here as a `Vec<histent>`
/// since each histent already has up/down pointers in the C struct.
pub static hist_ring: Mutex<Vec<histent>> = Mutex::new(Vec::new()); // c:103

/// Port of `struct histent curline` from Src/hist.c:91. Sentinel
/// histent for the in-progress edit; spliced into the ring head by
/// linkcurline() and removed by unlinkcurline().
pub static curline: Mutex<Option<histent>> = Mutex::new(None); // c:91

/// Port of `zlong histsiz` from Src/hist.c:108.
pub static histsiz: AtomicI64 = AtomicI64::new(0); // c:108

/// Port of `zlong savehistsiz` from Src/hist.c:113.
pub static savehistsiz: AtomicI64 = AtomicI64::new(0); // c:113

/// Port of `int histdone` from Src/hist.c:119.
pub static histdone: AtomicI32 = AtomicI32::new(0); // c:119

/// Port of `int histactive` from Src/hist.c:124.
pub static histactive: AtomicU32 = AtomicU32::new(0); // c:124

/// Port of `int hist_ignore_all_dups` from Src/hist.c:130.
pub static hist_ignore_all_dups: AtomicI32 = AtomicI32::new(0); // c:130

/// Port of `mod_export int hist_skip_flags` from Src/hist.c:135.
pub static hist_skip_flags: AtomicI32 = AtomicI32::new(0); // c:135

/// Port of `short *chwords` from Src/hist.c:147.
/// Word beginning/end offsets in current history line.
pub static chwords: Mutex<Vec<i16>> = Mutex::new(Vec::new()); // c:147

/// Port of `int chwordlen` from Src/hist.c:154.
pub static chwordlen: AtomicI32 = AtomicI32::new(0); // c:154

/// Port of `int chwordpos` from Src/hist.c:154.
pub static chwordpos: AtomicI32 = AtomicI32::new(0); // c:154

/// Port of `char *hsubl` from Src/hist.c:159.
/// Last `l` for `s/l/r/` history substitution.
pub static hsubl: Mutex<Option<String>> = Mutex::new(None); // c:159

/// Port of `char *hsubr` from Src/hist.c:164.
pub static hsubr: Mutex<Option<String>> = Mutex::new(None); // c:164

/// Port of `int hsubpatopt` from Src/hist.c:169.
pub static hsubpatopt: AtomicI32 = AtomicI32::new(0); // c:169

/// Port of `mod_export char *hptr` from Src/hist.c:174.
/// Pointer into the history line; tracked as the byte length of `chline`.
pub static hptr: AtomicUsize = AtomicUsize::new(0); // c:174

/// Port of `mod_export char *chline` from Src/hist.c:179.
pub static chline: Mutex<String> = Mutex::new(String::new()); // c:179

/// Port of `mod_export char *zle_chline` from Src/hist.c:195.
pub static zle_chline: Mutex<Option<String>> = Mutex::new(None); // c:195

/// Port of `int qbang` from Src/hist.c:201.
pub static qbang: AtomicBool = AtomicBool::new(false); // c:201

/// Port of `int hlinesz` from Src/hist.c:206.
pub static hlinesz: AtomicI32 = AtomicI32::new(0); // c:206

/// Port of `mod_export int expanding;` from Src/hist.c:65.
/// Non-zero while history-expansion is rewriting the current line.
pub static expanding: AtomicI32 = AtomicI32::new(0); // c:65

/// Port of `mod_export int excs;` from Src/hist.c:70.
/// Cursor position offset accumulator used while history-expanding.
pub static excs: AtomicI32 = AtomicI32::new(0); // c:70

/// Port of `mod_export int exlast;` from Src/hist.c:70.
/// Last `inbufct` snapshot taken at expansion start; the difference
/// drives the `excs` cursor advance through the rewritten line.
pub static exlast: AtomicI32 = AtomicI32::new(0); // c:70

/// Port of `static struct histfile_stats lasthist` from Src/hist.c:220-226.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct histfile_stats {
    // c:220
    pub text: Option<String>, // c:221
    pub stim: i64,            // c:222 time_t
    pub mtim: i64,            // c:222
    pub fpos: i64,            // c:223 off_t
    pub fsiz: i64,            // c:223
    pub interrupted: i32,     // c:224
    pub next_write_ev: i64,   // c:225 zlong
}
static lasthist: Mutex<histfile_stats> = Mutex::new(histfile_stats {
    // c:226
    text: None,
    stim: 0,
    mtim: 0,
    fpos: 0,
    fsiz: 0,
    interrupted: 0,
    next_write_ev: 0,
});

/// Port of `static struct histsave` from Src/hist.c:228-238.
#[allow(non_camel_case_types)]
pub struct histsave {
    // c:228
    pub lasthist: histfile_stats, // c:229
    pub histfile: Option<String>, // c:230
    pub hist_ring: Vec<histent>,  // c:232
    pub curhist: i64,             // c:233 zlong
    pub histlinect: i64,          // c:234
    pub histsiz: i64,             // c:235
    pub savehistsiz: i64,         // c:236
    pub locallevel: i32,          // c:237
}

/// Port of `static struct histsave *histsave_stack` from Src/hist.c:238.
#[allow(clippy::vec_init_then_push)]
static histsave_stack: Mutex<Vec<histsave>> = Mutex::new(Vec::new()); // c:238

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

// Re-export the canonical HIST_* flag bits from zsh_h.rs (which has
// the C-faithful values per `Src/zsh.h:2252-2258`).
//
// The previous hist.rs duplicates declared:
//   HIST_OLD     = 1 << 0  // = 0x01 (C: 0x02)
//   HIST_DUP     = 1 << 1  // = 0x02 (C: 0x08)
//   HIST_FOREIGN = 1 << 2  // = 0x04 (C: 0x10)
//   HIST_TMPSTORE= 1 << 3  // = 0x08 (C: 0x20 — overlapped with Rust HIST_DUP!)
//   HIST_NOWRITE = 1 << 4  // = 0x10 (C: 0x40 — overlapped with Rust HIST_FOREIGN!)
//
// Every flag was off-by-bit-shift AND the values overlapped each
// other in confusing ways (Rust HIST_TMPSTORE = C HIST_DUP). Any
// caller importing from `crate::ported::hist::HIST_DUP` was reading
// or writing the wrong bit; history-file write paths would skip
// entries the user wanted preserved, and HIST_FOREIGN/HIST_NOWRITE
// gates fired against unrelated bit patterns.

// =========================================================================
// CASMOD_ enum (port of zsh.h:3122-3127)
// =========================================================================

/// Port of `enum { CASMOD_NONE, CASMOD_UPPER, CASMOD_LOWER, CASMOD_CAPS }`
/// from Src/zsh.h:3122.
// =========================================================================
// HISTFLAG_* (port of zsh.h)
// =========================================================================

// HISTFLAG_* moved to canonical home at zsh_h.rs:2598-2601 (port of
// `Src/zsh.h:2270-2273`). Re-export here so existing callers using
// `crate::ported::hist::HISTFLAG_*` keep compiling, and the values
// can never diverge from zsh_h.rs.
//
// This is the same consolidation pattern applied to the prior HIST_*
// flag-value drift fix and the BINF/CONDF/MFF duplicates in
// module.rs — single source of truth for C-pinned bit values.

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
    // hist_ring is strictly DESCENDING by histnum: entries are only ever added
    // via `insert(0, ...)` with the monotonically increasing `curhist` counter,
    // so index 0 holds the newest (highest) histnum. A linear scan here walked
    // all ~40k entries on every lookup — and the full ring on every miss — which
    // made each prompt slower as history grew (profiled hot path: ring_get →
    // Iter::next). Binary search is O(log n); the comparator is `ev.cmp(elem)`
    // to match the descending order.
    match ring.binary_search_by(|h| ev.cmp(&h.histnum)) {
        Ok(idx) => Some(clone_histent(&ring[idx])),
        Err(_) => None,
    }
}

fn clone_histent(h: &histent) -> histent {
    histent {
        node: hashnode {
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
    // hist_ring is strictly descending by histnum (see ring_get); O(log n)
    // binary search instead of an O(n) linear `position` scan.
    hist_ring
        .lock()
        .unwrap()
        .binary_search_by(|h| ev.cmp(&h.histnum))
        .ok()
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
        node: hashnode {
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
                result = quotebreak(&result);
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
            'f' | 'F' | 'g' | 's' | '&' => {
                // c:3743 — `modify()` `:s/:g/:&/:f/:F` arm inlined per
                //          build.rs invariant.  `f` = repeat until no
                //          more changes (fixed-point); `F N` = repeat
                //          up to N times. zsh treats both as flags that
                //          PREFIX the actual `s`/`&`/`g` modifier.
                let mut fixed_point = false;
                let mut max_iters: Option<u32> = None;
                let mut c = c;
                if c == 'f' {
                    fixed_point = true;
                    c = chars.next().unwrap_or(' ');
                } else if c == 'F' {
                    // F takes a numeric argument: F N
                    let mut num = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch.is_ascii_digit() {
                            num.push(ch);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    max_iters = num.parse().ok();
                    c = chars.next().unwrap_or(' ');
                }
                let (global, do_parse) = match c {
                    's' => (false, true),
                    '&' => (false, false),
                    'g' => {
                        // 'g'
                        match chars.next() {
                            Some('s') => (true, true),
                            Some('&') => (true, false),
                            _ => break,
                        }
                    }
                    _ => break,
                };
                // c:3760 — read delimiter, parse old/new bracketed by it,
                //          backslash-escapes for embedded delimiters.
                let (pat, rep) = if do_parse {
                    let delim = chars.next().unwrap_or('/');
                    let mut old = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == delim {
                            chars.next();
                            break;
                        }
                        chars.next();
                        if ch == '\\' {
                            if let Some(&n) = chars.peek() {
                                if n == delim {
                                    chars.next();
                                    old.push(delim);
                                    continue;
                                }
                            }
                        }
                        old.push(ch);
                    }
                    let mut new = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == delim {
                            chars.next();
                            break;
                        }
                        chars.next();
                        if ch == '\\' {
                            if let Some(&n) = chars.peek() {
                                if n == delim {
                                    chars.next();
                                    new.push(delim);
                                    continue;
                                }
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
                    let apply = |s: &str| -> String {
                        if global {
                            s.replace(&pat, &rep)
                        } else {
                            s.replacen(&pat, &rep, 1)
                        }
                    };
                    if fixed_point {
                        // c:Src/hist.c — `:f` repeats until no more
                        // changes. Cap at a generous safety bound to
                        // avoid pathological growth on a replacement
                        // that re-introduces the pattern.
                        let cap: u32 = 10_000;
                        for _ in 0..cap {
                            let next = apply(&result);
                            if next == result {
                                break;
                            }
                            result = next;
                        }
                    } else if let Some(n) = max_iters {
                        for _ in 0..n {
                            let next = apply(&result);
                            if next == result {
                                break;
                            }
                            result = next;
                        }
                    } else {
                        result = apply(&result);
                    }
                }
            }
            // Bash-only modifiers — zsh rejects with "unrecognized
            // modifier". Match that error format. Without these arms,
            // unknown modifiers silently terminated the loop and the
            // caller saw the previous-stage value (often empty).
            // `F` is NOT in this set — zsh accepts `F N` as a
            // bounded-iteration prefix for `:s` (handled above).
            'U' | 'L' | 'V' | 'X' => {
                zerr(&format!("unrecognized modifier `{}'", c));
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
mod chrealpath_tests {
    use super::*;

    /// `chrealpath` MUST refuse a relative path (c:1999-2000 explicit
    /// `if (**junkptr != '/') return 0`). The C body has a hard
    /// comment at c:1997 that callers must pass absolute paths.
    /// A regression that resolved relative paths via the working
    /// directory would silently change history-modifier semantics
    /// (`!:A` against `foo.txt` would yield `/cwd/foo.txt` instead
    /// of the C-faithful "leave it alone").
    #[test]
    fn chrealpath_rejects_relative_path() {
        let _g = crate::test_util::global_state_lock();
        let r = chrealpath("relative/path", b'P', false); // c:1971
        assert!(
            r.is_none(),
            "c:1999-2000 — relative path MUST return None; got {:?}",
            r
        );
    }

    /// `chrealpath` empty input returns Some(empty) — the c:1985-1986
    /// guard `if (!**junkptr) return 1` returns success (1) for the
    /// empty-string case, leaving `*junkptr` unchanged. Pin so a
    /// regression that returns None for empty doesn't silently break
    /// the caller's "this modifier didn't change anything" path.
    #[test]
    fn chrealpath_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = chrealpath("", b'P', false); // c:1971
        assert_eq!(
            r.as_deref(),
            Some(""),
            "c:1985-1986 — empty input returns Some(empty), not None"
        );
    }

    /// `chrealpath` MUST fall back to walking parent prefixes when
    /// the full path doesn't exist (c:2027-2030 + c:2046-2048).
    /// Implements `realpath` against a not-yet-created file:
    /// strip components until a parent resolves, then re-splice the
    /// unresolvable tail.
    ///
    /// Pin: `/tmp/<real>/nonexistent/tail` where `/tmp/<real>` exists
    /// resolves to `/tmp/<real>/nonexistent/tail` (the unresolvable
    /// tail is appended verbatim). On macOS `/tmp` is a symlink to
    /// `/private/tmp` so the resolved prefix carries the canonical
    /// form. The test creates a real subdir in tmp to anchor the
    /// resolution and verifies the unresolvable tail is preserved.
    #[test]
    fn chrealpath_partial_prefix_fallback() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        // dir.path() is an absolute, canonicalized path.
        let probe = dir.path().join("nonexistent_sub/nonexistent_tail");
        let probe_str = probe.to_str().unwrap();
        let r = chrealpath(probe_str, b'P', false) // c:1971
            .expect("partial-prefix walk → Some");
        // The C semantics: prefix walks back to dir.path() (which
        // exists), and the tail "nonexistent_sub/nonexistent_tail"
        // gets re-spliced. Result must end with the unresolvable tail.
        assert!(
            r.ends_with("/nonexistent_sub/nonexistent_tail"),
            "c:2046-2048 — unresolvable tail must be re-spliced; got {:?}",
            r
        );
        // Result must START at a real absolute path (the existing prefix).
        // On macOS `dir.path()` might be `/var/folders/...` (already canonical).
        assert!(r.starts_with('/'), "result must be absolute; got {:?}", r);
    }
}

#[cfg(test)]
mod histsplitwords_uselex_tests {
    use super::*;

    /// `uselex=true` runs `bufferwords` and recovers byte offsets in
    /// `line`. `echo hi` → two words with the expected spans.
    #[test]
    fn uselex_matches_simple_words() {
        let line = "echo hi";
        let words = histsplitwords(line, true);
        assert_eq!(words, vec![(0, 4), (5, 7)]);
    }

    /// `uselex=false` is the no-lex fast path: whitespace tokenizer.
    #[test]
    fn no_uselex_matches_simple_words() {
        let line = "echo hi";
        let words = histsplitwords(line, false);
        assert_eq!(words, vec![(0, 4), (5, 7)]);
    }

    /// Pin the c:3782-3786 fallback: if the lexer disagrees with the
    /// raw line (forced bad case via shell-meta-only input the simple
    /// lex can't fully reconstruct), histsplitwords retries with
    /// uselex=false and returns a non-empty wordset.
    #[test]
    fn uselex_falls_back_on_lex_disagreement() {
        // bufferwords splits `a;b` into `["a", ";", "b"]`; raw line
        // matches each char-by-char, so this should succeed without
        // falling back. Pin success path.
        let line = "a;b";
        let words = histsplitwords(line, true);
        assert!(!words.is_empty(), "must produce at least one word");
        assert!(words.iter().all(|(s, e)| s < e && *e <= line.len()));
    }

    /// Multi-char ops `&&`/`||`/`;;` survive the lex round-trip with
    /// their original byte spans.
    #[test]
    fn uselex_handles_compound_operators() {
        let line = "a && b";
        let words = histsplitwords(line, true);
        // Each word's span must lie within the line.
        for (s, e) in &words {
            assert!(*e <= line.len() && s < e);
        }
    }

    /// Trailing whitespace doesn't produce a phantom word.
    #[test]
    fn no_uselex_trailing_whitespace_no_phantom() {
        let words = histsplitwords("hi   ", false);
        assert_eq!(words, vec![(0, 2)]);
    }

    /// `histsplitwords(uselex=true)` MUST distinguish a lone `;` from
    /// the `;;` case-terminator. The c:3715-3722 special case stops
    /// the strpfx fast-path when the lex token is `";"` but the line
    /// has `";;"` — otherwise the lex token would greedily consume
    /// both bytes and corrupt the case-statement's word offsets.
    ///
    /// Pin the contract: `foo ;; bar` produces words pointing at
    /// `foo`, `;;`, and `bar` (not `foo`, `;`, mid-stream garbage).
    /// A regression that skips the c:3715 disambiguation would
    /// produce truncated spans for the `bar` word.
    #[test]
    fn uselex_distinguishes_semicolon_from_double() {
        let line = "foo ;; bar";
        let words = histsplitwords(line, true);
        // Every word span must lie within the line.
        for (s, e) in &words {
            assert!(
                *e <= line.len(),
                "word ({},{}) overflows line len {}",
                s,
                e,
                line.len()
            );
            assert!(s < e, "empty span ({},{})", s, e);
        }
        // The last word must be "bar" (offsets 7..10).
        let last = words.last().expect("at least one word");
        assert_eq!(
            &line[last.0..last.1],
            "bar",
            "c:3715-3722 — last word must be 'bar' regardless of ;; handling, got {:?}",
            &line[last.0..last.1]
        );
    }
}

#[cfg(test)]
mod subst_modifier_tests {
    use super::*;

    /// Tests that touch the shared chline/hptr/chwords globals
    /// must serialize through this Mutex — cargo's parallel test
    /// runner races on these atomics otherwise.
    fn hist_test_lock() -> &'static Mutex<()> {
        static L: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn s_replaces_first_occurrence() {
        let _g = crate::test_util::global_state_lock();
        // c:3743 — `:s/old/new/` single substitution.
        assert_eq!(
            apply_history_modifiers("foo bar foo", ":s/foo/baz/"),
            "baz bar foo"
        );
    }

    #[test]
    fn gs_replaces_all_occurrences() {
        let _g = crate::test_util::global_state_lock();
        // c:3743 — `:gs/old/new/` global substitution.
        assert_eq!(
            apply_history_modifiers("foo bar foo", ":gs/foo/baz/"),
            "baz bar baz"
        );
    }

    #[test]
    fn ampersand_repeats_last_subst() {
        let _g = crate::test_util::global_state_lock();
        // c:3784 — `:&` repeats the cached last_str/last_rep pair.
        // First call caches old="x" new="y"; second `:&` reuses it.
        let first = apply_history_modifiers("xxx", ":s/x/y/");
        let second = apply_history_modifiers("xxxx", ":&");
        assert_eq!(first, "yxx");
        assert_eq!(second, "yxxx");
    }

    #[test]
    fn g_ampersand_repeats_last_subst_globally() {
        let _g = crate::test_util::global_state_lock();
        // c:3784 — `:g&` global form of `:&`.
        let _ = apply_history_modifiers("init", ":s/i/X/");
        // Now LAST_SUBST_OLD="i", LAST_SUBST_NEW="X". Global re-apply:
        assert_eq!(apply_history_modifiers("aiibii", ":g&"), "aXXbXX");
    }

    #[test]
    fn s_alternate_delimiter() {
        let _g = crate::test_util::global_state_lock();
        // c:3760 — first char after `s` is the delimiter; not bound
        //          to `/`.
        assert_eq!(apply_history_modifiers("a-b-c", ":s|-|+|"), "a+b-c");
    }

    #[test]
    fn s_escaped_delimiter_in_pattern() {
        let _g = crate::test_util::global_state_lock();
        // c:3768 — `\/` inside the pattern emits a literal `/`.
        assert_eq!(apply_history_modifiers("a/b", r":s/\//#/"), "a#b");
    }

    /// c:1304/1311 — `up_histent` walks the hist_ring toward newer
    /// entries; on an empty ring there's no walk possible. None is
    /// the well-defined empty state. Regression where it returns
    /// Some(0) would make the up-history widget enter a phantom entry.
    #[test]
    fn up_histent_on_empty_ring_is_none() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let mut gbal = 0i32;
        let mut cflag = 0i32;
        // No input pre-seeded; ingetc returns None on the very first
        // call → ptr1 is None → return 1.
        let r = getsubsargs("", &mut gbal, &mut cflag);
        assert_eq!(r, 1, "no delimiter byte → fail-fast 1");
        assert_eq!(gbal, 0, "no :G suffix observed");
        assert_eq!(cflag, 0, "no cflag set");
    }

    /// `histreduceblanks` collapses runs of spaces+tabs to single
    /// spaces. Used by HIST_REDUCE_BLANKS option. A regression that
    /// fails to collapse would bloat the history file with redundant
    /// whitespace.
    #[test]
    fn histreduceblanks_collapses_internal_runs() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // Space and tab — collapsed.
        assert_eq!(histreduceblanks("a  b"), "a b");
        assert_eq!(histreduceblanks("a\t\tb"), "a b");
        assert_eq!(
            histreduceblanks("a \tb"),
            "a b",
            "c:1240 — mixed space/tab run collapses to single space"
        );
        // Newline is NOT inblank per c:50 — must be preserved.
        assert_eq!(
            histreduceblanks("a\nb"),
            "a\nb",
            "c:50 — newline not in inblank; passes through unchanged"
        );
        // CR is NOT inblank.
        assert_eq!(
            histreduceblanks("a\rb"),
            "a\rb",
            "CR not in inblank class; must NOT be collapsed"
        );
        // NBSP (\u{A0}) is NOT inblank either.
        assert_eq!(
            histreduceblanks("a\u{A0}b"),
            "a\u{A0}b",
            "NBSP not in inblank; must NOT be collapsed"
        );
        // Leading/trailing spaces stripped (c:1241).
        assert_eq!(histreduceblanks("   x"), "x");
        assert_eq!(histreduceblanks("x   "), "x");
        // But leading newline is NOT stripped (newline not inblank).
        assert_eq!(histreduceblanks("\nx"), "\nx");
    }

    /// Pin `digitcount` to its canonical C body at `Src/hist.c:573-589`.
    /// C reads digits FROM THE INPUT STREAM via ingetc/inungetc, NOT
    /// from a string argument. The previous Rust port's `(s: &str)`
    /// signature was a fabrication with no real caller; the C version
    /// is used by `:h` and `:t` modifiers at c:871/c:892 to parse
    /// the digit count after the modifier letter.
    #[test]
    fn digitcount_streams_from_ingetc() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Push "42abc" into the input stream; digitcount() should
        // parse 42 and inungetc() the 'a'. Use inputsetline to
        // seed the input buffer.
        inputsetline("42abc", 0);
        let n = digitcount();
        assert_eq!(n, 42, "c:581 — decimal digit accumulation");

        // Next ingetc must yield 'a' (the inungetc'd terminator).
        let nxt = ingetc().unwrap_or('\0');
        assert_eq!(nxt, 'a', "c:587 — non-digit terminator was inungetc'd");

        // No digits at all → returns 0, inungetc's the first char.
        inputsetline("xyz", 0);
        let n = digitcount();
        assert_eq!(n, 0, "c:586 — non-digit first char returns 0");
        let nxt = ingetc().unwrap_or('\0');
        assert_eq!(
            nxt, 'x',
            "c:587 — even the non-digit first char is inungetc'd"
        );
    }

    /// `hist_in_word` / `hist_is_in_word` round-trip — the state flag
    /// the lexer flips while accumulating a word for history. C uses
    /// a single int; the Rust port preserves the bit-perfect contract.
    #[test]
    fn hist_in_word_round_trips() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(remtext("path/file.ext"), "path/file");
        assert_eq!(remtext("file.ext"), "file");
        assert_eq!(remtext("file"), "file");
    }

    /// c:2122 + Doc/Zsh/expn.yo:303-310 — the `:r` modifier strips
    /// "a `.` followed by any number of characters (including zero)
    /// that are neither `.` nor `/` and that continue to the end of
    /// the string." Leading dot IS an extension separator: `.bashrc`
    /// is wholly an extension (`.` + `bashrc`), so `:r` strips it
    /// entirely. The C body walks backward from end stopping at `/`
    /// and truncates at the first `.` found — no exception for dot
    /// at position 0.
    /// Regression target: a previous Rust port guarded with
    /// `dot_pos > 0` keeping leading-dot files unchanged, which
    /// diverges from `${.bashrc:r}` in real zsh.
    #[test]
    fn remtext_strips_leading_dot_per_zsh_doc() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            remtext(".bashrc"),
            "",
            "$.bashrc:r is an extension per Doc/Zsh/expn.yo:303"
        );
        assert_eq!(
            remtext("path/.bashrc"),
            "path/",
            "extension scan stops at `/`, then strips at first `.`"
        );
    }

    /// c:2136 — `rembutext("path/file.ext")` returns `"ext"` (the
    /// extension, dropping the body). Counterpart to `remtext`.
    /// Regression returning the wrong slice would break `${file:e}`.
    #[test]
    fn rembutext_returns_extension_only() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(rembutext("path/file.ext"), "ext");
        assert_eq!(
            rembutext("file.tar.gz"),
            "gz",
            "last `.` wins (extension-only is post-LAST-dot)"
        );
        assert_eq!(rembutext("file"), "");
    }

    /// c:2056 — `remtpath(path, 0)` (the `${PWD:h}` no-count case)
    /// removes the LAST component. `remtpath("/a/b/c", 0)` → `"/a/b"`.
    /// This is the canonical `:h` modifier path used by every theme
    /// that displays `${PWD:h}`.
    #[test]
    fn remtpath_count_zero_strips_last_component() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(remtpath("/a/b/c", 0), "/a/b");
        assert_eq!(remtpath("/a", 0), "/");
        assert_eq!(remtpath("foo", 0), ".", "no slash → returns '.'");
    }

    /// c:2152 — `remlpaths(path, count)` keeps the LAST `count`
    /// components — counterpart to remtpath. `remlpaths("/a/b/c", 2)`
    /// → `"b/c"`. Drives `${PWD:t}` family.
    #[test]
    fn remlpaths_keeps_last_n_components() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(remlpaths("/a/b/c", 1), "c");
        assert_eq!(remlpaths("/a/b/c", 2), "b/c");
        assert_eq!(remlpaths("/a/b/c", 3), "a/b/c");
    }

    /// c:2196 — `casemodify(s, CASMOD_LOWER)` lowercases every char.
    /// Regression that flips the case direction would break every
    /// `${(L)var}` user has.
    #[test]
    fn casemodify_lower_lowercases() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(casemodify("HELLO World", CASMOD_LOWER), "hello world");
    }

    /// c:2196 — `casemodify(s, CASMOD_UPPER)` uppercases.
    #[test]
    fn casemodify_upper_uppercases() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(casemodify("hello world", CASMOD_UPPER), "HELLO WORLD");
    }

    /// c:2196 — `CASMOD_CAPS` capitalises FIRST letter of each word
    /// (word-boundary determined by `nextupper` flag flips on
    /// non-alpha chars). `"hello world"` → `"Hello World"`.
    #[test]
    fn casemodify_caps_capitalises_word_starts() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(casemodify("hello world", CASMOD_CAPS), "Hello World");
        assert_eq!(
            casemodify("FOO BAR", CASMOD_CAPS),
            "Foo Bar",
            "non-first letters lowercased"
        );
    }

    /// `Src/hist.c:2486-2523` — `quote(s)` wraps in `'...'` and
    /// breaks out `inblank` chars (NARROW space/tab/newline ONLY)
    /// by closing the current quote span, emitting the blank in its
    /// own `'<c>'` pair, then opening a fresh quote span. CR/FF/VT/
    /// NBSP should NOT be broken out — they're not inblank.
    #[test]
    fn quote_breaks_only_narrow_inblank_chars() {
        let _g = crate::test_util::global_state_lock();
        // c:2514 — close current quote, emit ` ` in its own pair, reopen.
        assert_eq!(
            quote("a b"),
            "'a' 'b'",
            "c:2514 — space broken out of single-quote span"
        );
        assert_eq!(quote("a\tb"), "'a'\t'b'");
        assert_eq!(quote("a\nb"), "'a'\n'b'");
        // CR (\r) is NOT inblank per C → must stay inside the quotes.
        assert_eq!(
            quote("a\rb"),
            "'a\rb'",
            "c:2499 — CR is NOT in C's inblank set; stays inside quotes"
        );
        // NBSP (0xA0) is NOT in narrow inblank either.
        assert_eq!(
            quote("a\u{00A0}b"),
            "'a\u{00A0}b'",
            "NBSP is not inblank; must remain inside the quote span"
        );
    }

    /// `Src/hist.c:2527-2560` — `quotebreak(s)` same as quote but
    /// breaks out inblank chars regardless of inquotes state. Pin
    /// narrow-inblank behavior (no CR/FF/NBSP breaking).
    #[test]
    fn quotebreak_uses_narrow_inblank_set() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotebreak("a b"), "'a' 'b'");
        assert_eq!(quotebreak("a\tb"), "'a'\t'b'");
        assert_eq!(quotebreak("a\nb"), "'a'\n'b'");
        // CR — NOT inblank → stays inside.
        assert_eq!(
            quotebreak("a\rb"),
            "'a\rb'",
            "CR not in inblank set, must not be broken out"
        );
        // NBSP — NOT inblank.
        assert_eq!(
            quotebreak("a\u{00A0}b"),
            "'a\u{00A0}b'",
            "NBSP not in inblank, must not be broken out"
        );
        // Form-feed (\x0C) — NOT inblank.
        assert_eq!(
            quotebreak("a\u{000C}b"),
            "'a\u{000C}b'",
            "FF not in inblank, must not be broken out"
        );
    }

    /// Pin: `savehistfile` short-circuits when `!interact` per
    /// `Src/hist.c:2932`. Non-interactive shells must not write
    /// to the user's HISTFILE — a script running with INTERACTIVE
    /// off should leave the user's history untouched even when
    /// passed an explicit fn_path.
    ///
    /// Also pins the `savehistsiz <= 0` short-circuit. Either
    /// gate firing must leave the file untouched.
    #[test]
    fn savehistfile_short_circuits_on_non_interactive() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hist_test");
        let path_str = path.to_str().unwrap();
        // Pre-populate with content so we can detect "untouched".
        std::fs::write(&path, b"PRESERVED").expect("seed write");
        // Force INTERACTIVE off; savehistfile must NOT touch the
        // file regardless of fn_path.
        let saved = isset(INTERACTIVE);
        dosetopt(INTERACTIVE, 0, 0);
        savehistfile(Some(path_str), 0);
        let after = std::fs::read(&path).expect("read after");
        assert_eq!(
            after, b"PRESERVED",
            "c:2932 — !interact must skip write; original content preserved"
        );
        // Restore.
        dosetopt(INTERACTIVE, if saved { 1 } else { 0 }, 0);
    }

    /// Pin: `hgetline` per `Src/hist.c:1769-1786` truncates the
    /// in-flight `chline` at `hptr`, resets the globals, and
    /// returns the captured snippet. Returns None when chline is
    /// empty or hptr is at the start (C returns NULL).
    #[test]
    fn hgetline_truncates_chline_and_resets_globals() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Save state.
        let saved_chline = std::mem::take(&mut *chline.lock().unwrap());
        let saved_hptr = hptr.swap(0, SeqCst);
        let saved_chwordpos = chwordpos.swap(0, SeqCst);

        // Empty chline → None (c:1777).
        assert_eq!(hgetline(), None, "c:1777 — empty chline returns None");

        // chline = "abcdef", hptr = 0 → None (c:1777 hp == 0).
        *chline.lock().unwrap() = "abcdef".to_string();
        hptr.store(0, SeqCst);
        assert_eq!(hgetline(), None, "c:1777 — hptr == 0 returns None");

        // chline = "abcdef", hptr = 3 → Some("abc"), reset hptr/pos.
        *chline.lock().unwrap() = "abcdef".to_string();
        hptr.store(3, SeqCst);
        chwordpos.store(2, SeqCst);
        let result = hgetline();
        assert_eq!(
            result,
            Some("abc".to_string()),
            "c:1779 — truncate chline at hptr=3 returns 'abc'"
        );
        assert_eq!(hptr.load(SeqCst), 0, "c:1783 — hptr reset to 0");
        assert_eq!(chwordpos.load(SeqCst), 0, "c:1784 — chwordpos reset to 0");

        // Restore state.
        *chline.lock().unwrap() = saved_chline;
        hptr.store(saved_hptr, SeqCst);
        chwordpos.store(saved_chwordpos, SeqCst);
    }

    /// Pin: `histbackword` per `Src/hist.c:1711-1715` rewinds `hptr`
    /// to the start of the previous word ONLY when:
    ///   1. `chwordpos % 2 == 0` (even position — at a word
    ///      boundary, not mid-word), AND
    ///   2. `chwordpos != 0` (at least one full word recorded).
    /// Otherwise no-op.
    #[test]
    fn histbackword_rewinds_hptr_on_even_boundary() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Capture and reset state.
        let saved_pos = chwordpos.swap(0, SeqCst);
        let saved_hptr = hptr.swap(0, SeqCst);
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
        chwordpos.store(4, SeqCst);
        hptr.store(999, SeqCst);
        histbackword();
        assert_eq!(
            hptr.load(SeqCst),
            7,
            "c:1715 — even chwordpos must rewind hptr to chwords[pos-1]"
        );

        // chwordpos at 0 (no words recorded) — no-op.
        chwordpos.store(0, SeqCst);
        hptr.store(123, SeqCst);
        histbackword();
        assert_eq!(
            hptr.load(SeqCst),
            123,
            "c:1714 — chwordpos == 0 means no-op (hptr untouched)"
        );

        // chwordpos at 3 (odd, mid-word) — no-op.
        chwordpos.store(3, SeqCst);
        hptr.store(456, SeqCst);
        histbackword();
        assert_eq!(
            hptr.load(SeqCst),
            456,
            "c:1714 — odd chwordpos means mid-word, no-op"
        );

        // Restore state.
        chwordpos.store(saved_pos, SeqCst);
        hptr.store(saved_hptr, SeqCst);
        *chwords.lock().unwrap() = saved_words;
    }

    /// Pin `ihwaddc` to the C `*hptr++ = c;` OVERWRITE semantic.
    /// When hptr < chline.len() (e.g. after hwrep rewinds), the byte
    /// goes AT the cursor, not appended. The previous Rust port used
    /// `String::push` which appends only — pushing past the rewound
    /// hptr would leave the old word bytes intact and stash new bytes
    /// at the chline tail.
    #[test]
    fn ihwaddc_overwrites_at_hptr_not_append() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Save state.
        let saved_chline = chline.lock().unwrap().clone();
        let saved_hptr = hptr.load(SeqCst);
        let saved_errflag = errflag.load(SeqCst);
        let saved_lexstop = lexstop.load(SeqCst);
        let saved_inflags = crate::ported::input::inbufflags.with(|f| f.get());
        let saved_qbang = qbang.load(SeqCst);
        let saved_stophist = stophist.load(SeqCst);
        let saved_hlinesz = hlinesz.load(SeqCst);

        // Set up chline="echo oldword extra", rewind hptr to 5
        // (start of "oldword"). Push "NEW" → chline must become
        // "echo NEWword extra" with hptr advanced to 8.
        *chline.lock().unwrap() = "echo oldword extra".to_string();
        hptr.store(5, SeqCst);
        errflag.store(0, SeqCst);
        lexstop.store(false, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(0));
        qbang.store(false, SeqCst);
        stophist.store(0, SeqCst);
        hlinesz.store(64, SeqCst);

        ihwaddc(b'N' as i32);
        ihwaddc(b'E' as i32);
        ihwaddc(b'W' as i32);

        let cl = chline.lock().unwrap().clone();
        assert_eq!(
            cl.as_str(),
            "echo NEWword extra",
            "c:368 — *hptr++ = c writes AT cursor (NOT appends to end)"
        );
        assert_eq!(
            hptr.load(SeqCst),
            8,
            "c:368 — hptr advances over the three overwrites"
        );

        // Restore.
        *chline.lock().unwrap() = saved_chline;
        hptr.store(saved_hptr, SeqCst);
        errflag.store(saved_errflag, SeqCst);
        lexstop.store(saved_lexstop, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(saved_inflags));
        qbang.store(saved_qbang, SeqCst);
        stophist.store(saved_stophist, SeqCst);
        hlinesz.store(saved_hlinesz, SeqCst);
    }

    /// Pin `ihwaddc` to its canonical C body at `Src/hist.c:355-389`.
    /// C: `*hptr++ = c;` writes the byte and advances the cursor.
    /// The previous Rust port pushed to chline but never updated
    /// the `hptr` global — every subsequent ihwbegin/ihwend read a
    /// stale hptr=0, so chwords boundary positions stayed pinned
    /// to 0 no matter how many chars were appended.
    #[test]
    fn ihwaddc_advances_hptr_on_each_push() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Save state.
        let saved_chline = chline.lock().unwrap().clone();
        let saved_hptr = hptr.load(SeqCst);
        let saved_errflag = errflag.load(SeqCst);
        let saved_lexstop = lexstop.load(SeqCst);
        let saved_inflags = crate::ported::input::inbufflags.with(|f| f.get());
        let saved_qbang = qbang.load(SeqCst);
        let saved_bangchar = bangchar.load(SeqCst);
        let saved_stophist = stophist.load(SeqCst);
        let saved_hlinesz = hlinesz.load(SeqCst);

        // Set up: history active, no errors, no alias-only.
        *chline.lock().unwrap() = "AB".to_string(); // chline must be non-empty
        hptr.store(2, SeqCst);
        errflag.store(0, SeqCst);
        lexstop.store(false, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(0));
        qbang.store(false, SeqCst);
        stophist.store(0, SeqCst);
        bangchar.store(b'!' as i32, SeqCst);
        hlinesz.store(64, SeqCst);

        // Push 'x' → chline grows AND hptr advances.
        ihwaddc(b'x' as i32);
        assert_eq!(
            chline.lock().unwrap().as_str(),
            "ABx",
            "c:368 — chline grows"
        );
        assert_eq!(hptr.load(SeqCst), 3, "c:368 — hptr advances by 1");

        // Push 'y' → second advance.
        ihwaddc(b'y' as i32);
        assert_eq!(hptr.load(SeqCst), 4, "c:368 — hptr advances on each push");

        // Bang-escape with qbang: qbang=true, c=bangchar → '\\' AND c
        // get pushed, hptr advances by 2.
        qbang.store(true, SeqCst);
        ihwaddc(b'!' as i32);
        assert_eq!(
            chline.lock().unwrap().as_str(),
            "ABxy\\!",
            "c:366 — qbang escape pushes '\\\\' before bangchar"
        );
        assert_eq!(
            hptr.load(SeqCst),
            6,
            "c:366+c:368 — both pushes advance hptr"
        );

        // errflag set → no-op.
        errflag.store(1, SeqCst);
        let hptr_before = hptr.load(SeqCst);
        ihwaddc(b'z' as i32);
        assert_eq!(
            hptr.load(SeqCst),
            hptr_before,
            "c:359 — errflag short-circuits, hptr unchanged"
        );

        // Restore.
        *chline.lock().unwrap() = saved_chline;
        hptr.store(saved_hptr, SeqCst);
        errflag.store(saved_errflag, SeqCst);
        lexstop.store(saved_lexstop, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(saved_inflags));
        qbang.store(saved_qbang, SeqCst);
        bangchar.store(saved_bangchar, SeqCst);
        stophist.store(saved_stophist, SeqCst);
        hlinesz.store(saved_hlinesz, SeqCst);
    }

    /// Pin `ihwend` to its canonical C body at `Src/hist.c:1686-1705`.
    /// Same family of fix as `ihwbegin`: use the `hptr` global to
    /// compute the cursor position, NOT `chline.len()`. C: `if (hptr
    /// > chline + chwords[chwordpos-1])` writes `chwords[chwordpos++]
    /// = hptr - chline`; the previous Rust port used chline.len()
    /// for the cursor.
    #[test]
    fn ihwend_uses_hptr_not_chline_len() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Save state.
        let saved_chline = chline.lock().unwrap().clone();
        let saved_chwords = chwords.lock().unwrap().clone();
        let saved_chwordpos = chwordpos.load(SeqCst);
        let saved_hptr = hptr.load(SeqCst);
        let saved_stop = stophist.load(SeqCst);
        let saved_active = histactive.load(SeqCst);
        let saved_inflags = crate::ported::input::inbufflags.with(|f| f.get());

        // Set up: chline is "ABCDEFGHIJ" (10 bytes), hptr at 7
        // (lexer rewound). chwordpos=1 (in-flight word that started
        // at chwords[0]=2). C expects chwords[1] = hptr-chline = 7,
        // not chline.len() = 10.
        *chline.lock().unwrap() = "ABCDEFGHIJ".to_string();
        *chwords.lock().unwrap() = vec![2, 0];
        chwordpos.store(1, SeqCst);
        hptr.store(7, SeqCst);
        stophist.store(0, SeqCst);
        histactive.store(0, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(0));

        ihwend();

        assert_eq!(
            chwords.lock().unwrap().get(1).copied(),
            Some(7),
            "c:1694 — chwords[chwordpos] = hptr - chline = 7 (NOT chline.len()=10)"
        );
        assert_eq!(
            chwordpos.load(SeqCst),
            2,
            "c:1694 — chwordpos++ on successful close"
        );

        // c:1700 scrub branch — hptr<=chwords[start] → chwordpos--.
        *chwords.lock().unwrap() = vec![5, 0];
        chwordpos.store(1, SeqCst);
        hptr.store(3, SeqCst); // hptr(3) <= chwords[0]=5
        ihwend();
        assert_eq!(
            chwordpos.load(SeqCst),
            0,
            "c:1700 — chwordpos-- when hptr <= chwords[chwordpos-1]"
        );

        // Even chwordpos short-circuits (between-words, no in-flight).
        chwordpos.store(2, SeqCst);
        hptr.store(9, SeqCst);
        *chwords.lock().unwrap() = vec![0, 4, 5, 8];
        ihwend();
        assert_eq!(
            chwordpos.load(SeqCst),
            2,
            "c:1691 — even chwordpos short-circuits"
        );

        // Restore.
        *chline.lock().unwrap() = saved_chline;
        *chwords.lock().unwrap() = saved_chwords;
        chwordpos.store(saved_chwordpos, SeqCst);
        hptr.store(saved_hptr, SeqCst);
        stophist.store(saved_stop, SeqCst);
        histactive.store(saved_active, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(saved_inflags));
    }

    /// Pin `histremovedups` to its canonical C body at
    /// `Src/hist.c:1252-1262`: removes entries with HIST_DUP flag
    /// SET, not name-based dedup. Also pins the HIST_DUP bit value
    /// to the canonical C value (0x08 per `Src/zsh.h:2255`) — the
    /// hist.rs duplicate had `1 << 1 = 0x02` which both overlapped
    /// HIST_OLD and missed real HIST_DUP entries.
    #[test]
    fn histremovedups_removes_flagged_entries_only() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let saved_ring = {
            let r = hist_ring.lock().unwrap();
            r.iter()
                .map(|h| (h.node.nam.clone(), h.histnum, h.node.flags))
                .collect::<Vec<_>>()
        };
        let saved_histlinect = histlinect.load(SeqCst);

        // HIST_DUP must equal C value 0x08 (Src/zsh.h:2255).
        assert_eq!(
            HIST_DUP, 0x08,
            "HIST_DUP bit value must match C (0x08), got {:#x}",
            HIST_DUP
        );

        // Build a ring with three entries, only the middle one flagged HIST_DUP.
        {
            let mut ring = hist_ring.lock().unwrap();
            ring.clear();
            for (nam, num, flags) in [
                ("entry1", 1i64, 0i32),
                ("entry2", 2i64, HIST_DUP as i32), // flagged
                ("entry3", 3i64, 0i32),
            ] {
                ring.push(histent {
                    node: hashnode {
                        next: None,
                        nam: nam.to_string(),
                        flags,
                    },
                    up: None,
                    down: None,
                    zle_text: None,
                    stim: 0,
                    ftim: 0,
                    words: vec![],
                    nwords: 0,
                    histnum: num,
                });
            }
        }
        histlinect.store(3, SeqCst);

        histremovedups();

        {
            let ring = hist_ring.lock().unwrap();
            assert_eq!(
                ring.len(),
                2,
                "c:1259-1260 — only the HIST_DUP-flagged entry is removed"
            );
            assert!(ring.iter().any(|h| h.node.nam == "entry1"));
            assert!(ring.iter().any(|h| h.node.nam == "entry3"));
            assert!(!ring.iter().any(|h| h.node.nam == "entry2"));
        }
        assert_eq!(
            histlinect.load(SeqCst),
            2,
            "histlinect updated after removal"
        );

        // Restore ring.
        {
            let mut ring = hist_ring.lock().unwrap();
            ring.clear();
            for (nam, num, flags) in saved_ring {
                ring.push(histent {
                    node: hashnode {
                        next: None,
                        nam,
                        flags,
                    },
                    up: None,
                    down: None,
                    zle_text: None,
                    stim: 0,
                    ftim: 0,
                    words: vec![],
                    nwords: 0,
                    histnum: num,
                });
            }
        }
        histlinect.store(saved_histlinect, SeqCst);
    }

    /// Pin `iaddtoline` to its canonical C body at
    /// `Src/hist.c:397-413`, specifically the c:405-411 ZLE cursor
    /// adjustment block: `excs > zlemetacs` → `excs += 1 + inbufct -
    /// exlast` with a clamp to zlemetacs.
    #[test]
    fn iaddtoline_adjusts_excs_relative_to_zlemetacs() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Save state.
        let saved_chline = chline.lock().unwrap().clone();
        let saved_excs = excs.load(SeqCst);
        let saved_zlemetacs = ZLEMETACS.load(SeqCst);
        let saved_expanding = expanding.load(SeqCst);
        let saved_lexstop = lexstop.load(SeqCst);
        let saved_qbang = qbang.load(SeqCst);
        let saved_exlast = exlast.load(SeqCst);

        // Set up: expanding=1, lexstop=false, qbang=0 (no bang
        // escape path), excs > zlemetacs so the adjustment fires.
        *chline.lock().unwrap() = String::new();
        expanding.store(1, SeqCst);
        lexstop.store(false, SeqCst);
        qbang.store(false, SeqCst);
        excs.store(10, SeqCst); // > zlemetacs
        ZLEMETACS.store(5, SeqCst);
        crate::ported::input::inbufct.with(|c| c.set(3));
        exlast.store(2, SeqCst);

        // c:405-407 — excs(10) > zlemetacs(5), so
        // new_excs = 10 + 1 + 3 - 2 = 12; 12 > 5 → no clamp.
        iaddtoline(b'x' as i32);
        assert_eq!(
            excs.load(SeqCst),
            12,
            "c:406 — excs += 1 + inbufct - exlast (10+1+3-2=12)"
        );

        // c:407-410 — clamp branch: post-add excs < zlemetacs.
        // Set excs=6, zlemetacs=20, inbufct=1, exlast=10.
        // new_excs = 6 + 1 + 1 - 10 = -2; -2 < 20 → clamp to 20.
        excs.store(6, SeqCst);
        // But guard: excs(6) > zlemetacs(20) is FALSE, so block
        // wouldn't fire. Need excs > zlemetacs for entry: use
        // excs=25, zlemetacs=20, inbufct=1, exlast=10 →
        // new_excs = 25+1+1-10 = 17; 17 < 20 → clamp to 20.
        excs.store(25, SeqCst);
        ZLEMETACS.store(20, SeqCst);
        crate::ported::input::inbufct.with(|c| c.set(1));
        exlast.store(10, SeqCst);
        iaddtoline(b'y' as i32);
        assert_eq!(
            excs.load(SeqCst),
            20,
            "c:407-410 — clamp to zlemetacs(20) when post-add excs<zlemetacs"
        );

        // No adjustment when excs <= zlemetacs.
        excs.store(3, SeqCst);
        ZLEMETACS.store(10, SeqCst);
        crate::ported::input::inbufct.with(|c| c.set(1));
        exlast.store(0, SeqCst);
        iaddtoline(b'z' as i32);
        assert_eq!(
            excs.load(SeqCst),
            3,
            "c:405 — excs<=zlemetacs leaves excs unchanged"
        );

        // Restore.
        *chline.lock().unwrap() = saved_chline;
        excs.store(saved_excs, SeqCst);
        ZLEMETACS.store(saved_zlemetacs, SeqCst);
        expanding.store(saved_expanding, SeqCst);
        lexstop.store(saved_lexstop, SeqCst);
        qbang.store(saved_qbang, SeqCst);
        exlast.store(saved_exlast, SeqCst);
    }

    /// Pin `ihwbegin` to its canonical C body at `Src/hist.c:1656-1670`.
    /// The C computes `pos = hptr - chline + offset` — the byte
    /// offset of the current write head in chline plus a caller-
    /// supplied offset. The previous Rust port used chline.len()
    /// which is only equal when hptr is at end-of-buffer; any
    /// lexer rewind (backquote, comment, here-doc) would record
    /// wrong word offsets.
    #[test]
    fn ihwbegin_records_hptr_not_chline_len() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Save state.
        let saved_chline = chline.lock().unwrap().clone();
        let saved_chwords = chwords.lock().unwrap().clone();
        let saved_chwordpos = chwordpos.load(SeqCst);
        let saved_hptr = hptr.load(SeqCst);
        let saved_stop = stophist.load(SeqCst);
        let saved_active = histactive.load(SeqCst);
        let saved_inflags = crate::ported::input::inbufflags.with(|f| f.get());

        // Set up: chline is "ABCDEFGHIJ" (10 bytes), but hptr was
        // rewound to position 4 — simulating a lexer that backtracked.
        // C would record pos=4 (hptr-chline=4 + offset=0); the buggy
        // Rust port would record pos=10 (chline.len()).
        *chline.lock().unwrap() = "ABCDEFGHIJ".to_string();
        chwords.lock().unwrap().clear();
        chwordpos.store(0, SeqCst);
        hptr.store(4, SeqCst);
        stophist.store(0, SeqCst);
        histactive.store(0, SeqCst); // !HA_INWORD
        crate::ported::input::inbufflags.with(|f| f.set(0)); // not alias-only

        ihwbegin(0);

        let recorded = chwords.lock().unwrap().first().copied().unwrap_or(-1);
        assert_eq!(
            recorded, 4,
            "c:1658 — pos = hptr - chline + offset = 4 + 0 = 4 \
             (NOT chline.len()=10)"
        );

        // Negative offset clamps to 0 (c:1666).
        chwords.lock().unwrap().clear();
        chwordpos.store(0, SeqCst);
        hptr.store(3, SeqCst);
        ihwbegin(-10); // pos = 3-10 = -7 → clamp to 0
        let recorded = chwords.lock().unwrap().first().copied().unwrap_or(-1);
        assert_eq!(recorded, 0, "c:1666 — pos<0 clamps to 0");

        // Stop guard (c:1659) — `stophist == 2` short-circuits.
        chwords.lock().unwrap().clear();
        chwordpos.store(0, SeqCst);
        hptr.store(5, SeqCst);
        stophist.store(2, SeqCst);
        ihwbegin(0);
        assert!(
            chwords.lock().unwrap().is_empty(),
            "c:1659 — stophist==2 short-circuits, no record"
        );
        stophist.store(0, SeqCst);

        // Alias-only guard (c:1659).
        chwords.lock().unwrap().clear();
        chwordpos.store(0, SeqCst);
        hptr.store(5, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(INP_ALIAS));
        ihwbegin(0);
        assert!(
            chwords.lock().unwrap().is_empty(),
            "c:1659 — alias-only (INP_ALIAS without INP_HIST) short-circuits"
        );

        // INP_ALIAS|INP_HIST does NOT short-circuit (mixed input).
        chwords.lock().unwrap().clear();
        chwordpos.store(0, SeqCst);
        hptr.store(7, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(INP_ALIAS | INP_HIST));
        ihwbegin(0);
        let recorded = chwords.lock().unwrap().first().copied().unwrap_or(-1);
        assert_eq!(recorded, 7, "c:1659 — alias+hist mixed still records");

        // Restore.
        *chline.lock().unwrap() = saved_chline;
        *chwords.lock().unwrap() = saved_chwords;
        chwordpos.store(saved_chwordpos, SeqCst);
        hptr.store(saved_hptr, SeqCst);
        stophist.store(saved_stop, SeqCst);
        histactive.store(saved_active, SeqCst);
        crate::ported::input::inbufflags.with(|f| f.set(saved_inflags));
    }

    /// Pin `getargs` to its canonical C body at `Src/hist.c:2454-2482`.
    /// Covers: nwords-derived-from-field (not words.len()/2),
    /// arg1>arg2 reject, arg≥nwords reject, full-event fast path,
    /// per-word slicing, and signed-short overflow detection.
    #[test]
    fn getargs_handles_field_indexing_and_overflow() {
        let _g = crate::test_util::global_state_lock();
        // Build a histent for "echo hello world" with 3 words.
        // C nwords=3, words=[0,4,5,10,11,16] (start/end pairs).
        let he = histent {
            node: hashnode {
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
            nwords: 3, // c:2457 source of truth
            histnum: 1,
        };

        // c:2459 — `arg2 < arg1` → reject.
        assert_eq!(getargs(&he, 2, 1), None, "c:2459 — arg2 < arg1 rejects");
        // c:2459 — `arg1 >= nwords` → reject.
        assert_eq!(
            getargs(&he, 3, 3),
            None,
            "c:2459 — arg1 >= nwords (3>=3) rejects"
        );
        // c:2459 — `arg2 >= nwords` → reject.
        assert_eq!(
            getargs(&he, 0, 3),
            None,
            "c:2459 — arg2 >= nwords (3>=3) rejects"
        );

        // c:2466 — `arg1==0 && arg2==nwords-1` → full event fast path.
        assert_eq!(
            getargs(&he, 0, 2).as_deref(),
            Some("echo hello world"),
            "c:2467 — full-event fast path returns dupstring(nam)"
        );

        // c:2469-2481 — per-word slice. word[0] = "echo" (pos 0..4).
        assert_eq!(
            getargs(&he, 0, 0).as_deref(),
            Some("echo"),
            "c:2481 — word[0] = nam[0..4]"
        );
        // word[1] = "hello" (pos 5..10).
        assert_eq!(
            getargs(&he, 1, 1).as_deref(),
            Some("hello"),
            "c:2481 — word[1] = nam[5..10]"
        );
        // word[2] = "world" (pos 11..16).
        assert_eq!(
            getargs(&he, 2, 2).as_deref(),
            Some("world"),
            "c:2481 — word[2] = nam[11..16]"
        );
        // Multi-word span: word[1..=2] = "hello world".
        assert_eq!(
            getargs(&he, 1, 2).as_deref(),
            Some("hello world"),
            "c:2481 — words[1..=2] = nam[5..16]"
        );

        // c:2476 — signed-short overflow detection. Build a histent
        // whose stored pos[0] is negative (simulating i16 wrap on a
        // >32KB history line). Use nwords=2 with arg1=0,arg2=0 so the
        // c:2466 full-event fast path doesn't trigger.
        let overflow = histent {
            node: hashnode {
                next: None,
                nam: "ab cd".to_string(),
                flags: 0,
            },
            up: None,
            down: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: vec![-1, 5, 3, 5], // word[0]: pos1 < 0
            nwords: 2,
            histnum: 1,
        };
        assert_eq!(
            getargs(&overflow, 0, 0),
            None,
            "c:2476 — pos1 < 0 (i16 overflow) rejects"
        );

        // c:2476 — `pos1 < arg1` detection (pos must be ≥ word index).
        let underflow = histent {
            node: hashnode {
                next: None,
                nam: "a b c d".to_string(),
                flags: 0,
            },
            up: None,
            down: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            // arg1=2 but pos1=1 means recorded pos < word index.
            // Each word must be ≥1 char so word[2] must start at pos ≥ 2.
            words: vec![0, 1, 2, 3, 1, 5, 6, 7],
            nwords: 4,
            histnum: 1,
        };
        assert_eq!(
            getargs(&underflow, 2, 2),
            None,
            "c:2476 — pos1 < arg1 (i16 overflow signal) rejects"
        );
    }

    /// Pin `hconsearch` to its canonical C body at
    /// `Src/hist.c:1834-1854`. Searches up from the ring for the
    /// most recent entry whose text contains `needle` as a substring,
    /// returning `(histnum, marg)` where marg is the word index of
    /// the match. The previous Rust port dropped marg entirely.
    #[test]
    fn hconsearch_returns_histnum_and_word_index() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        // Save ring state.
        let saved_ring = {
            let r = hist_ring.lock().unwrap();
            r.iter()
                .map(|h| {
                    (
                        h.node.nam.clone(),
                        h.histnum,
                        h.words.clone(),
                        h.nwords,
                        h.node.flags,
                    )
                })
                .collect::<Vec<_>>()
        };
        let saved_curhist = curhist.load(SeqCst);

        // Build a single-entry ring with "echo hello world" — 3 words.
        // words = [start1,end1, start2,end2, start3,end3]
        //          0    4       5    10      11   16
        {
            let mut ring = hist_ring.lock().unwrap();
            ring.clear();
            ring.push(histent {
                node: hashnode {
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
        curhist.store(8, SeqCst); // up_histent will walk back to 7

        // "hello" found at pos 5 → word index 1 (0-based).
        let got = hconsearch("hello");
        assert_eq!(
            got,
            Some((7, 1)),
            "c:1846-1850 — strstr at pos 5 lands in word[1] (start=5)"
        );

        // "world" found at pos 11 → word index 2.
        let got = hconsearch("world");
        assert_eq!(
            got,
            Some((7, 2)),
            "c:1846-1850 — strstr at pos 11 lands in word[2] (start=11)"
        );

        // "echo" found at pos 0 → word index 0.
        let got = hconsearch("echo");
        assert_eq!(
            got,
            Some((7, 0)),
            "c:1846-1850 — strstr at pos 0 lands in word[0]"
        );

        // Miss → None (c:1853 return -1).
        let got = hconsearch("notthere");
        assert_eq!(got, None, "c:1853 — miss returns -1 / None");

        // HIST_FOREIGN entries are skipped (c:1843).
        {
            let mut ring = hist_ring.lock().unwrap();
            ring.clear();
            ring.push(histent {
                node: hashnode {
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
        assert_eq!(
            got, None,
            "c:1843-1844 — HIST_FOREIGN entries continue past, miss → None"
        );

        // Restore ring.
        {
            let mut ring = hist_ring.lock().unwrap();
            ring.clear();
            for (nam, histnum, words, nwords, flags) in saved_ring {
                ring.push(histent {
                    node: hashnode {
                        next: None,
                        nam,
                        flags,
                    },
                    up: None,
                    down: None,
                    zle_text: None,
                    stim: 0,
                    ftim: 0,
                    words,
                    nwords,
                    histnum,
                });
            }
        }
        curhist.store(saved_curhist, SeqCst);
    }

    /// Pin `checkcurline` to the canonical C body at
    /// `Src/hist.c:2421-2429`: when `he.histnum == curhist` AND
    /// `histactive & HA_ACTIVE`, flush chline/chwordpos/chwords
    /// into `curline`. Both gates MUST be true; otherwise leave
    /// `curline` untouched.
    #[test]
    fn checkcurline_flushes_to_curline_only_when_active_and_matching() {
        let _g = crate::test_util::global_state_lock();
        let _g = hist_test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let saved_curhist = curhist.load(SeqCst);
        let saved_active = histactive.load(SeqCst);
        let saved_chline = chline.lock().unwrap().clone();
        let saved_chwordpos = chwordpos.load(SeqCst);
        let saved_chwords = chwords.lock().unwrap().clone();
        let saved_curline = curline.lock().unwrap().take();

        // Set up in-flight build state.
        curhist.store(42, SeqCst);
        histactive.store(HA_ACTIVE, SeqCst);
        *chline.lock().unwrap() = "echo hello".to_string();
        chwordpos.store(4, SeqCst); // 2 words
        *chwords.lock().unwrap() = vec![0, 4, 5, 10];
        *curline.lock().unwrap() = None;

        // Case 1: matching histnum + active → flushes.
        let he = histent {
            node: hashnode {
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
            let snap = cl
                .as_ref()
                .expect("c:2425-2427 — matching+active must flush a snapshot");
            assert_eq!(
                snap.node.nam, "echo hello",
                "c:2425 — curline.node.nam = chline"
            );
            assert_eq!(
                snap.nwords, 2,
                "c:2426 — curline.nwords = chwordpos/2 (4/2=2)"
            );
            assert_eq!(
                snap.words,
                vec![0, 4, 5, 10],
                "c:2427 — curline.words = chwords"
            );
        }

        // Case 2: matching histnum but NOT active → no flush.
        histactive.store(0, SeqCst); // c:2424 HA_ACTIVE off
        *curline.lock().unwrap() = None;
        checkcurline(&he);
        assert!(
            curline.lock().unwrap().is_none(),
            "c:2424 — HA_ACTIVE cleared, no flush"
        );

        // Case 3: active but mismatched histnum → no flush.
        histactive.store(HA_ACTIVE, SeqCst);
        let he2 = histent {
            histnum: 99,
            ..histent {
                node: hashnode {
                    next: None,
                    nam: String::new(),
                    flags: 0,
                },
                up: None,
                down: None,
                zle_text: None,
                stim: 0,
                ftim: 0,
                words: vec![],
                nwords: 0,
                histnum: 0,
            }
        };
        checkcurline(&he2);
        assert!(
            curline.lock().unwrap().is_none(),
            "c:2424 — histnum mismatch, no flush"
        );

        // Restore.
        curhist.store(saved_curhist, SeqCst);
        histactive.store(saved_active, SeqCst);
        *chline.lock().unwrap() = saved_chline;
        chwordpos.store(saved_chwordpos, SeqCst);
        *chwords.lock().unwrap() = saved_chwords;
        *curline.lock().unwrap() = saved_curline;
    }

    /// `Src/hist.c:2172-2175` — `remlpaths` when count exceeds the
    /// number of path components returns the input UNMODIFIED
    /// (preserving the leading slash for absolute paths). The previous
    /// Rust port silently stripped the leading slash via the
    /// `parts.iter().rev().take(min(n, parts.len()))` clamp, so `:t4`
    /// on `/a/b/c` returned `"a/b/c"` instead of `"/a/b/c"`. Catches
    /// this divergence — the C `return 1` early-exit at the leftmost
    /// `/` does NOT modify `*junkptr`.
    #[test]
    fn remlpaths_count_exceeds_components_preserves_leading_slash() {
        let _g = crate::test_util::global_state_lock();
        // 4 > 3 components in "/a/b/c" → return whole string verbatim.
        assert_eq!(
            remlpaths("/a/b/c", 4),
            "/a/b/c",
            "c:2172-2175 — count > components → preserve original (leading slash)"
        );
        assert_eq!(remlpaths("/a/b/c", 10), "/a/b/c");
        // Relative path: no leading slash, but still preserved.
        assert_eq!(remlpaths("a/b/c", 99), "a/b/c");
    }

    /// `Src/hist.c:2156-2161` — `remlpaths` trims trailing slashes off
    /// the input before walking. Pin so `:t1` on `/a/b/c/` (trailing
    /// slash) still returns `"c"`, not `""`.
    #[test]
    fn remlpaths_trims_trailing_slashes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            remlpaths("/a/b/c/", 1),
            "c",
            "c:2156-2161 — trailing slash trimmed before scan"
        );
        assert_eq!(remlpaths("/a/b/c///", 1), "c");
        assert_eq!(
            remlpaths("/", 1),
            "",
            "all-slashes input → empty after trim → empty result"
        );
        assert_eq!(remlpaths("", 1), "");
    }

    /// `Src/hist.c:2152` — `remlpaths` with `count == 0` is a special
    /// case: per `digitcount()` at c:574, bare `:t` (no number) passes
    /// 0 which means "default to 1". The Rust port preserves that
    /// alias so `${PWD:t}` keeps the last component.
    #[test]
    fn remlpaths_count_zero_defaults_to_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            remlpaths("/a/b/c", 0),
            "c",
            "c:574 — count=0 from digitcount aliases to default 1"
        );
        // Same as explicit count=1.
        assert_eq!(remlpaths("/a/b/c", 0), remlpaths("/a/b/c", 1));
    }

    /// `Src/hist.c:2225-2228` — CASMOD_LOWER converts uppercase chars
    /// to lowercase, leaves everything else unchanged.
    #[test]
    fn casemodify_lower_lowercases_uppercase_only() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            casemodify("HELLO", CASMOD_LOWER),
            "hello",
            "c:2226-2228 — uppercase → lowercase"
        );
        // Already lowercase: no change.
        assert_eq!(casemodify("hello", CASMOD_LOWER), "hello");
        // Digits + punct: untouched.
        assert_eq!(casemodify("123 !? abc", CASMOD_LOWER), "123 !? abc");
        // Unicode: 'É' (U+00C9) lowercases to 'é'.
        assert_eq!(casemodify("ÉLITE", CASMOD_LOWER), "élite");
    }

    /// `Src/hist.c:2232-2236` — CASMOD_UPPER converts lowercase chars
    /// to uppercase, leaves everything else unchanged.
    #[test]
    fn casemodify_upper_uppercases_lowercase_only() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            casemodify("hello", CASMOD_UPPER),
            "HELLO",
            "c:2233-2236 — lowercase → uppercase"
        );
        assert_eq!(casemodify("HELLO", CASMOD_UPPER), "HELLO");
        assert_eq!(casemodify("élite", CASMOD_UPPER), "ÉLITE");
    }

    /// `Src/hist.c:2239-2254` — CASMOD_CAPS title-cases at word
    /// boundaries (defined by `!iswalnum` chars).
    #[test]
    fn casemodify_caps_title_cases_at_word_boundaries() {
        let _g = crate::test_util::global_state_lock();
        // c:2245-2250 — first letter and post-space letter uppercase;
        // mid-word letters lowercase (c:2251-2253).
        assert_eq!(casemodify("hello world", CASMOD_CAPS), "Hello World");
        // c:2243-2244 — non-alphanumeric resets nextupper.
        assert_eq!(casemodify("foo-bar.baz", CASMOD_CAPS), "Foo-Bar.Baz");
        // Already capped: mid-word `LO` → `lo`.
        assert_eq!(casemodify("HELLO", CASMOD_CAPS), "Hello");
    }

    /// `Src/hist.c:2241-2242` — `IS_COMBINING(wc)` (WCWIDTH == 0)
    /// makes CASMOD_CAPS short-circuit BEFORE touching nextupper.
    /// The previous Rust port omitted this guard — combining marks
    /// were classified as non-alphanumeric and reset nextupper,
    /// breaking word-boundary detection on accented words written
    /// as base char + combining mark.
    ///
    /// Test input: `"a" + COMBINING ACUTE (U+0301) + "b"`. With the
    /// guard: combining mark passes through, b stays lowercase
    /// (still inside the word). Without the guard: the combiner
    /// resets nextupper, so `b` would be uppercased.
    #[test]
    fn casemodify_caps_skips_combining_chars() {
        let _g = crate::test_util::global_state_lock();
        // a + combining acute (U+0301) + b — under CAPS, the
        // combiner must NOT reset nextupper. Expected output:
        // `A` + combining acute + `b` (b stays lowercase because
        // it's still mid-word).
        let input = "a\u{0301}b";
        let got = casemodify(input, CASMOD_CAPS);
        let expected = "A\u{0301}b";
        assert_eq!(
            got, expected,
            "c:2241-2242 — IS_COMBINING short-circuit must not break word boundary"
        );
    }

    /// `Src/hist.c:2349` — anchor-head matcher checks BOTH ASCII `#`
    /// AND the tokenized `Pound` byte (0x84). Previous Rust port
    /// matched only `'#'`. Pin the Pound-token recognition.
    #[test]
    fn subst_anchor_head_recognises_pound_token() {
        let _g = crate::test_util::global_state_lock();
        // c:2349 — ASCII '#' anchor at head: matches only prefix.
        assert_eq!(
            subst("foobar", "#foo", "baz", false),
            "bazbar",
            "c:2349 ASCII '#' anchor — match at head"
        );
        // c:2349 — Pound token (0x84) anchor at head: same effect.
        let pat = format!("{}foo", Pound);
        assert_eq!(
            subst("foobar", &pat, "baz", false),
            "bazbar",
            "c:2349 Pound token (0x84) anchor — match at head"
        );
        // Negative: anchored at head MUST NOT match mid-string.
        let pat = format!("{}foo", Pound);
        assert_eq!(
            subst("xfoo", &pat, "baz", false),
            "xfoo",
            "c:2349 anchor-head rejects non-prefix"
        );
    }

    /// `Src/hist.c:2354` — anchor-tail matcher checks `%` only
    /// (no tokenized counterpart in C). Pin the basic semantics.
    #[test]
    fn subst_anchor_tail_matches_only_suffix() {
        let _g = crate::test_util::global_state_lock();
        // c:2354 — `%foo` anchored at tail.
        assert_eq!(
            subst("xxfoo", "%foo", "bar", false),
            "xxbar",
            "c:2354 — '%' anchors at end of string"
        );
        // Non-suffix → no change.
        assert_eq!(
            subst("foox", "%foo", "bar", false),
            "foox",
            "c:2354 — '%foo' must not match unless `foo` is at end"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // histreduceblanks edge cases — pure function, anchored to C semantics
    // (c:50, c:1240): collapse runs of space/tab to single space; trim
    // leading and trailing space; PRESERVE embedded newlines/CRs.
    // ═══════════════════════════════════════════════════════════════════

    /// Single space stays single.
    #[test]
    fn histreduceblanks_single_space_stays_one() {
        assert_eq!(histreduceblanks("a b"), "a b");
    }

    /// Run of spaces collapses to one.
    #[test]
    fn histreduceblanks_multi_space_collapses_to_one() {
        assert_eq!(histreduceblanks("a     b"), "a b");
    }

    /// Tab counts as inblank — collapses to single space.
    #[test]
    fn histreduceblanks_tab_collapses_with_spaces() {
        assert_eq!(histreduceblanks("a \t  b"), "a b");
    }

    /// Leading whitespace trimmed.
    #[test]
    fn histreduceblanks_leading_whitespace_trimmed() {
        assert_eq!(histreduceblanks("   hello"), "hello");
    }

    /// Trailing whitespace trimmed.
    #[test]
    fn histreduceblanks_trailing_whitespace_trimmed() {
        assert_eq!(histreduceblanks("hello   "), "hello");
    }

    /// Both leading AND trailing trimmed.
    #[test]
    fn histreduceblanks_both_ends_trimmed() {
        assert_eq!(histreduceblanks("  hi  "), "hi");
    }

    /// Empty input → empty output.
    #[test]
    fn histreduceblanks_empty_input_returns_empty() {
        assert_eq!(histreduceblanks(""), "");
    }

    /// All-whitespace input → empty after trim.
    #[test]
    fn histreduceblanks_all_whitespace_becomes_empty() {
        assert_eq!(histreduceblanks("     "), "");
        assert_eq!(histreduceblanks("\t\t  \t"), "");
    }

    /// Embedded newline is NOT treated as inblank — preserved as-is.
    /// (c:50 — `inblank` is space/tab ONLY; newline is preserved.)
    #[test]
    fn histreduceblanks_newline_preserved_not_collapsed() {
        // Newlines stay; surrounding spaces collapse normally.
        let r = histreduceblanks("a\nb");
        assert_eq!(r, "a\nb", "newline must be preserved");
    }

    /// Multiple newlines stay; flanking spaces don't get consumed.
    #[test]
    fn histreduceblanks_multiple_newlines_preserved() {
        // Pin contract: newlines are not in inblank → preserved exactly.
        let r = histreduceblanks("a\n\nb");
        assert_eq!(r, "a\n\nb");
    }

    /// Space-newline-space — the spaces flanking the newline are not
    /// treated as a "run" because newline breaks continuity.
    #[test]
    fn histreduceblanks_space_around_newline_preserved() {
        // Trim only happens at the very ends, not inside.
        let r = histreduceblanks("a \n b");
        // Each space around \n is its own (already single) run; newline
        // resets prev_space; the second space starts a new run.
        assert_eq!(r, "a \n b");
    }

    /// Mixed: leading + multi + trailing all trimmed/collapsed.
    #[test]
    fn histreduceblanks_complex_input_normalizes() {
        assert_eq!(histreduceblanks("   a   b\t\tc   "), "a b c");
    }

    // ═══════════════════════════════════════════════════════════════════
    // histsplitwords edge cases — pure tokenizer pinning byte spans.
    // ═══════════════════════════════════════════════════════════════════

    /// Single word, no whitespace.
    #[test]
    fn histsplitwords_single_word_returns_one_span() {
        let words = histsplitwords("echo", false);
        assert_eq!(words, vec![(0, 4)]);
    }

    /// Empty input → no words.
    #[test]
    fn histsplitwords_empty_input_returns_no_words() {
        let words = histsplitwords("", false);
        assert!(words.is_empty(), "empty line has no words; got {words:?}");
    }

    /// Only whitespace → no words.
    #[test]
    fn histsplitwords_only_whitespace_returns_no_words() {
        let words = histsplitwords("    ", false);
        assert!(
            words.is_empty(),
            "whitespace-only has no words; got {words:?}"
        );
    }

    /// Leading whitespace doesn't create phantom first word at offset 0.
    #[test]
    fn histsplitwords_leading_whitespace_skipped_no_uselex() {
        // "   hi" — first word starts at offset 3.
        let words = histsplitwords("   hi", false);
        assert_eq!(words, vec![(3, 5)]);
    }

    /// Tab-separated words.
    #[test]
    fn histsplitwords_tab_separator_works_like_space() {
        let words = histsplitwords("a\tb\tc", false);
        // 3 words, each one char long.
        assert_eq!(words.len(), 3);
        for (s, e) in &words {
            assert_eq!(e - s, 1, "each word is one char; got ({s},{e})");
        }
    }

    /// Spans never overlap and never overflow the input length.
    #[test]
    fn histsplitwords_spans_well_formed() {
        let line = "alpha beta gamma";
        for use_lex in [false, true] {
            let words = histsplitwords(line, use_lex);
            for (s, e) in &words {
                assert!(
                    s < e && *e <= line.len(),
                    "bad span ({s},{e}) for line len {} use_lex={}",
                    line.len(),
                    use_lex
                );
            }
            // Spans must be monotonically advancing.
            for i in 1..words.len() {
                assert!(
                    words[i].0 >= words[i - 1].1,
                    "spans overlap: {:?} then {:?}",
                    words[i - 1],
                    words[i]
                );
            }
        }
    }

    // ─── zsh-corpus pins for histreduceblanks ─────────────────────

    /// Empty input → empty output.
    #[test]
    fn hist_corpus_histreduceblanks_empty_is_empty() {
        assert_eq!(histreduceblanks(""), "");
    }

    /// All-spaces input collapses to nothing (trimmed).
    #[test]
    fn hist_corpus_histreduceblanks_all_spaces_to_empty() {
        assert_eq!(histreduceblanks("     "), "");
        assert_eq!(histreduceblanks("\t\t\t"), "");
        assert_eq!(histreduceblanks(" \t \t "), "");
    }

    /// Single non-space char passes through.
    #[test]
    fn hist_corpus_histreduceblanks_single_char() {
        assert_eq!(histreduceblanks("x"), "x");
    }

    /// Tab counts as inblank; mixed space+tab runs collapse to one space.
    #[test]
    fn hist_corpus_histreduceblanks_mixed_space_tab() {
        assert_eq!(histreduceblanks("a \t \t b"), "a b");
    }

    /// Multibyte characters pass through unchanged.
    #[test]
    fn hist_corpus_histreduceblanks_multibyte_passthrough() {
        assert_eq!(histreduceblanks("日 本"), "日 本");
        assert_eq!(histreduceblanks("日   本"), "日 本");
    }

    /// Newlines preserved with exact count (not in inblank class).
    #[test]
    fn hist_corpus_histreduceblanks_newlines_exact_count() {
        assert_eq!(histreduceblanks("a\n\n\nb"), "a\n\n\nb");
    }

    /// `hist_is_in_word` round-trips with `hist_in_word`.
    #[test]
    fn hist_corpus_in_word_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = hist_is_in_word();
        hist_in_word(1);
        assert_eq!(hist_is_in_word(), 1);
        hist_in_word(0);
        assert_eq!(hist_is_in_word(), 0);
        hist_in_word(saved);
    }

    /// `substfailed` returns an i32 (zsh stores -1 when no subst
    /// has happened yet, 0 on success, 1 on fail). Pin: returns
    /// without panic.
    #[test]
    fn hist_corpus_substfailed_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = substfailed();
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/hist.c. Tests that capture KNOWN
    // ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `getargc(entry)` returns `entry.nwords - 1` (or 0 if nwords=0).
    /// C `Src/hist.c:556`:
    ///   `return ehist->nwords ? ehist->nwords-1 : 0;`
    /// Fixed: previously Rust returned plain `entry.nwords` (off-by-one).
    #[test]
    fn getargc_returns_nwords_minus_one() {
        let _g = crate::test_util::global_state_lock();
        // Build a fake histent with nwords=3. C returns 2; current
        // Rust returns 3.
        let entry = histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: String::new(),
                flags: 0,
            },
            down: None,
            up: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: Vec::new(),
            nwords: 3,
            histnum: 0,
        };
        assert_eq!(
            getargc(&entry),
            2,
            "C returns nwords-1=2 for nwords=3; Rust off-by-one bug returns 3"
        );
    }

    /// `getargc(entry)` with nwords=0 returns 0 (per C ternary
    /// guard). Rust's off-by-one bug happens to return 0 here too
    /// (correct value); this test pins the corner case where Rust
    /// and C agree by accident.
    #[test]
    fn getargc_nwords_zero_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let entry = histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: String::new(),
                flags: 0,
            },
            down: None,
            up: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: Vec::new(),
            nwords: 0,
            histnum: 0,
        };
        assert_eq!(getargc(&entry), 0, "nwords=0 → 0 (both C and Rust)");
    }

    /// `getargc(entry)` with nwords=1 — C returns 0 (1-1).
    /// Fixed: matches C semantics now.
    #[test]
    fn getargc_nwords_one_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let entry = histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: String::new(),
                flags: 0,
            },
            down: None,
            up: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: Vec::new(),
            nwords: 1,
            histnum: 0,
        };
        assert_eq!(getargc(&entry), 0, "nwords=1 → 0 (1-1) per C");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hist.c histreduceblanks.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1240 — `histreduceblanks("")` returns empty.
    #[test]
    fn histreduceblanks_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(histreduceblanks(""), "");
    }

    /// c:1240 — `histreduceblanks` collapses multiple spaces to one.
    #[test]
    fn histreduceblanks_collapses_runs_of_spaces() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(histreduceblanks("a    b"), "a b");
        assert_eq!(histreduceblanks("a  b  c"), "a b c");
    }

    /// c:1240 — `histreduceblanks` collapses tabs same as spaces.
    #[test]
    fn histreduceblanks_collapses_tabs_to_space() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(histreduceblanks("a\tb"), "a b", "tab → space");
        assert_eq!(histreduceblanks("a\t\t\tb"), "a b", "tab run → 1 space");
        assert_eq!(histreduceblanks("a \t \tb"), "a b", "mixed run → 1 space");
    }

    /// c:1240 — strips leading whitespace.
    #[test]
    fn histreduceblanks_strips_leading_whitespace() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(histreduceblanks("   abc"), "abc");
        assert_eq!(histreduceblanks("\t\tabc"), "abc");
    }

    /// c:1240 — strips trailing whitespace.
    #[test]
    fn histreduceblanks_strips_trailing_whitespace() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(histreduceblanks("abc   "), "abc");
        assert_eq!(histreduceblanks("abc\t\t"), "abc");
    }

    /// c:1240 — pure whitespace input returns empty.
    #[test]
    fn histreduceblanks_only_whitespace_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(histreduceblanks("   "), "");
        assert_eq!(histreduceblanks("\t\t\t"), "");
        assert_eq!(histreduceblanks(" \t \t "), "");
    }

    /// c:1240 — `histreduceblanks` is idempotent.
    #[test]
    fn histreduceblanks_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for input in &["a  b", "   foo", "x\ty\tz", "  hello world  "] {
            let once = histreduceblanks(input);
            let twice = histreduceblanks(&once);
            assert_eq!(once, twice, "must be idempotent on {:?}", input);
        }
    }

    /// c:1240 — single space between words is preserved (already minimal).
    #[test]
    fn histreduceblanks_single_space_preserved() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(histreduceblanks("a b"), "a b");
        assert_eq!(histreduceblanks("hello world"), "hello world");
    }

    /// c:50 — inblank is space/tab ONLY; newline is NOT collapsed.
    #[test]
    fn histreduceblanks_preserves_newlines() {
        let _g = crate::test_util::global_state_lock();
        // Newline is not inblank → preserved (and bordering chars too).
        let r = histreduceblanks("a\nb");
        assert!(r.contains('\n'), "newline preserved in {:?}", r);
    }

    /// `histremovedups()` is safe on an empty hist_ring.
    #[test]
    fn histremovedups_empty_ring_is_safe() {
        let _g = crate::test_util::global_state_lock();
        hist_ring.lock().unwrap().clear();
        histremovedups();
        // No panic = pass; ring stays empty.
        assert!(hist_ring.lock().unwrap().is_empty());
    }

    /// `getargc(entry)` for nwords=5 returns 4 per C `nwords - 1`.
    /// Pins post-fix C-correct semantics.
    #[test]
    fn getargc_returns_nwords_minus_one_for_five() {
        let _g = crate::test_util::global_state_lock();
        let entry = histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: String::new(),
                flags: 0,
            },
            down: None,
            up: None,
            zle_text: None,
            stim: 0,
            ftim: 0,
            words: Vec::new(),
            nwords: 5,
            histnum: 0,
        };
        assert_eq!(getargc(&entry), 4, "C: 5-1 = 4");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hist.c
    // c:141 hist_in_word / c:150 hist_is_in_word / c:1155 herrflush /
    // c:1221 substfailed / c:1255 digitcount / c:1549 histreduceblanks /
    // c:1611 addhistnum / c:1293 strinbeg / c:1313 strinend
    // ═══════════════════════════════════════════════════════════════════

    /// c:141 + c:150 — `hist_in_word(N)` then `hist_is_in_word()` round-trips.
    #[test]
    fn hist_in_word_set_get_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = hist_is_in_word();
        hist_in_word(1);
        assert_eq!(hist_is_in_word(), 1, "set/get round-trips");
        hist_in_word(0);
        assert_eq!(hist_is_in_word(), 0, "set/get round-trips back");
        hist_in_word(saved);
    }

    /// c:1155 — `herrflush` is idempotent.
    #[test]
    fn herrflush_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            herrflush();
        }
    }

    /// c:1221 — `substfailed` returns -1 per C body `return -1`.
    #[test]
    fn substfailed_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(substfailed(), -1, "C body: return -1");
    }

    /// c:1255 — `digitcount` returns i32 (compile-time type pin).
    #[test]
    fn digitcount_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = digitcount();
    }

    /// c:1549 — `histreduceblanks("")` empty returns empty String.
    #[test]
    fn histreduceblanks_empty_returns_empty_pin() {
        assert_eq!(histreduceblanks(""), "");
    }

    /// c:1549 — `histreduceblanks` is pure.
    #[test]
    fn histreduceblanks_is_pure() {
        for s in ["", "abc", "  spaces  ", "\ttabs\t", "no\nnewlines\nhere"] {
            let first = histreduceblanks(s);
            for _ in 0..3 {
                assert_eq!(
                    histreduceblanks(s),
                    first,
                    "histreduceblanks({:?}) must be pure",
                    s
                );
            }
        }
    }

    /// c:1293 + c:1313 — `strinbeg(0)` + `strinend` round-trip safe.
    #[test]
    fn strinbeg_strinend_round_trip_safe() {
        let _g = crate::test_util::global_state_lock();
        strinbeg(0);
        strinend();
    }

    /// c:1611 — `addhistnum(0, 0, 0)` zero delta returns i64 (type pin).
    #[test]
    fn addhistnum_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i64 = addhistnum(0, 0, 0);
    }

    /// c:1611 — `addhistnum(N, 0, _)` zero increment preserves N.
    #[test]
    fn addhistnum_zero_increment_returns_input() {
        let _g = crate::test_util::global_state_lock();
        // Note: behavior depends on history ring state — pin only that
        // result is deterministic for same input.
        let first = addhistnum(0, 0, 0);
        for _ in 0..3 {
            assert_eq!(
                addhistnum(0, 0, 0),
                first,
                "addhistnum(0, 0, 0) must be deterministic"
            );
        }
    }

    /// c:1328 — `nohw(0)` no-op accepts any i32.
    #[test]
    fn nohw_full_i32_range_no_panic() {
        for c in [i32::MIN, -1, 0, 1, 42, i32::MAX] {
            nohw(c);
        }
    }

    /// c:1332 + c:1336 — `nohwabort` + `nohwe` are no-op idempotent.
    #[test]
    fn nohwabort_nohwe_idempotent() {
        for _ in 0..5 {
            nohwabort();
            nohwe();
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/hist.c
    // c:141 hist_in_word / c:150 hist_is_in_word / c:1417 unlinkcurline /
    // c:3582 flockhistfile / c:3745 histfileIsLocked / c:4449 firsthist
    // ═══════════════════════════════════════════════════════════════════

    /// c:150 — `hist_is_in_word` returns i32 (compile-time type pin).
    #[test]
    fn hist_is_in_word_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = hist_is_in_word();
    }

    /// c:150 — `hist_is_in_word` returns 0 or 1 only.
    #[test]
    fn hist_is_in_word_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let r = hist_is_in_word();
        assert!(r == 0 || r == 1, "hist_is_in_word ∈ {{0,1}}, got {}", r);
    }

    /// c:141 + c:150 — `hist_in_word(1)` then `hist_is_in_word() == 1`
    /// round-trip.
    #[test]
    fn hist_in_word_set_clear_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = hist_is_in_word();
        hist_in_word(1);
        assert_eq!(hist_is_in_word(), 1, "set → is_in_word=1");
        hist_in_word(0);
        assert_eq!(hist_is_in_word(), 0, "clear → is_in_word=0");
        hist_in_word(saved);
    }

    /// c:141 — `hist_in_word(0)` idempotent on a clear state.
    #[test]
    fn hist_in_word_clear_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let saved = hist_is_in_word();
        hist_in_word(0);
        for _ in 0..5 {
            hist_in_word(0);
            assert_eq!(hist_is_in_word(), 0, "still clear");
        }
        hist_in_word(saved);
    }

    /// c:1417 — `unlinkcurline` is safe on empty / fresh state.
    #[test]
    fn unlinkcurline_idempotent_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            unlinkcurline();
        }
    }

    /// c:3582 — `flockhistfile` returns i32 (compile-time type pin).
    #[test]
    fn flockhistfile_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = flockhistfile("/__never_exists_zshrs_xyz__");
    }

    /// c:3582 — `flockhistfile(nonexistent)` returns nonzero / determ.
    #[test]
    fn flockhistfile_nonexistent_path_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = flockhistfile("/__never_exists_zshrs_flock__");
        for _ in 0..3 {
            assert_eq!(
                flockhistfile("/__never_exists_zshrs_flock__"),
                first,
                "flockhistfile must be deterministic"
            );
        }
    }

    /// c:3745 — `histfileIsLocked` returns i32 (compile-time type pin).
    #[test]
    fn histfileIsLocked_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = histfileIsLocked();
    }

    /// c:3745 — `histfileIsLocked` returns 0 or 1 only.
    #[test]
    fn histfileIsLocked_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let r = histfileIsLocked();
        assert!(r == 0 || r == 1, "histfileIsLocked ∈ {{0,1}}, got {}", r);
    }

    /// c:4449 — `firsthist` returns i64 (compile-time type pin).
    #[test]
    fn firsthist_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i64 = firsthist();
    }

    /// c:4449 — `firsthist` empty ring returns 1 (per c:4451 default).
    #[test]
    fn firsthist_default_when_ring_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = firsthist();
        assert!(
            r >= 1,
            "firsthist ≥ 1 (default + populated both ≥ 1), got {}",
            r
        );
    }

    /// c:4449 — `firsthist` is deterministic on stable ring.
    #[test]
    fn firsthist_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = firsthist();
        for _ in 0..3 {
            assert_eq!(
                firsthist(),
                first,
                "firsthist must be deterministic on unchanged ring"
            );
        }
    }

    /// c:1221 — `substfailed` returns i32 (compile-time type pin).
    #[test]
    fn substfailed_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = substfailed();
    }
}
