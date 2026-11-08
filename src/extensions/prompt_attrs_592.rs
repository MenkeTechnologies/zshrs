//! zsh 5.9.2's prompt attribute escapes, for the `zsh` drop-in.
//!
//! **!!! RUST-ONLY — the code it follows is zsh 5.9.2's, not the development
//! tree's that `src/ported/prompt.rs` ports.** The tree under
//! `~/forkedRepos/zsh` keeps a current and a pending attribute set and
//! `applytextattributes` prints only what changed, so `%B%B` prints bold once.
//! zsh 5.9.2 (`git show zsh-5.9.2:Src/prompt.c`, lines 521-574) prints a
//! capability for every escape as it is read, and `tsetcap` (lines 1014-1062)
//! follows any capability that may have cleared other attributes
//! (`TSC_DIRTY`) by switching the ones still on back on:
//!
//! ```text
//!   print -P '%B%Bx'       5.9.2: \e[1m\e[1mx       development tree: \e[1mx
//!   print -P '%B%S%Bx'     5.9.2: \e[1m\e[7m\e[1m\e[7mx
//! ```
//!
//! [`attr_escapes`] reproduces that for the escape just read and returns the
//! capability strings, one per `tsetcap` call. The caller brackets each with
//! Inpar/Outpar and sets the port's current and pending attribute sets to the
//! returned mask, so the port's own diff has nothing left to print.

use crate::ported::prompt::set_colour_attribute;
use crate::ported::zsh_h::{
    zattr, COL_SEQ_BG, COL_SEQ_FG, SINGLELINEZLE, TCALLATTRSOFF, TCBOLDFACEBEG, TCSTANDOUTBEG,
    TCSTANDOUTEND, TCUNDERLINEBEG, TCUNDERLINEEND, TERM_BAD, TERM_NOUP, TERM_UNKNOWN,
    TXTBGCOLOUR, TXTBOLDFACE, TXTFGCOLOUR, TXTSTANDOUT, TXTUNDERLINE, TXT_ATTR_BG_24BIT,
    TXT_ATTR_BG_COL_MASK, TXT_ATTR_FG_24BIT, TXT_ATTR_FG_COL_MASK,
};
use std::sync::atomic::Ordering;

/// 5.9.2's `TXT_ATTR_FG_ON_MASK` (`Src/zsh.h:2731`): everything `%F` sets.
const FG_ON_MASK: zattr = TXTFGCOLOUR | TXT_ATTR_FG_COL_MASK | TXT_ATTR_FG_24BIT;
/// 5.9.2's `TXT_ATTR_BG_ON_MASK` (`Src/zsh.h:2735`): everything `%K` sets.
const BG_ON_MASK: zattr = TXTBGCOLOUR | TXT_ATTR_BG_COL_MASK | TXT_ATTR_BG_24BIT;

/// One capability as 5.9.2's `tsetcap` writes it under `TSC_PROMPT`: the
/// escape and the number of `Nularg` glitch cells (`sg`/`ug`) after it.
pub struct Cap {
    pub seq: String,
    pub glitch: i32,
}

/// The escapes 5.9.2 prints for prompt escape `xc` (one of `SsBbUuFfKk`),
/// starting from attribute set `mask`. `atr` is `parsecolorchar`'s result for
/// `%F`/`%K`; 0 or `TXT_ERROR` falls through to `%f`/`%k` as in C.
/// Returns the capabilities and the attribute set after the escape.
pub fn attr_escapes(xc: u8, atr: zattr, valid_colour: bool, mut mask: zattr) -> (Vec<Cap>, zattr) {
    let mut out = Vec::new();
    match xc {
        b'S' => {
            mask |= TXTSTANDOUT;
            tsetcap(TCSTANDOUTBEG, false, mask, &mut out);
        }
        b's' => {
            mask &= !TXTSTANDOUT;
            tsetcap(TCSTANDOUTEND, true, mask, &mut out);
        }
        b'B' => {
            mask |= TXTBOLDFACE;
            tsetcap(TCBOLDFACEBEG, true, mask, &mut out);
        }
        b'b' => {
            mask &= !TXTBOLDFACE;
            tsetcap(TCALLATTRSOFF, true, mask, &mut out);
        }
        b'U' => {
            mask |= TXTUNDERLINE;
            tsetcap(TCUNDERLINEBEG, false, mask, &mut out);
        }
        b'u' => {
            mask &= !TXTUNDERLINE;
            tsetcap(TCUNDERLINEEND, true, mask, &mut out);
        }
        b'F' | b'f' if xc == b'F' && valid_colour => {
            mask = (mask & !TXT_ATTR_FG_COL_MASK) | (atr & FG_ON_MASK);
            colour(atr, COL_SEQ_FG, &mut out);
        }
        b'F' | b'f' => {
            mask &= !FG_ON_MASK;
            colour(0, COL_SEQ_FG, &mut out);
        }
        b'K' | b'k' if xc == b'K' && valid_colour => {
            mask = (mask & !TXT_ATTR_BG_COL_MASK) | (atr & BG_ON_MASK);
            colour(atr, COL_SEQ_BG, &mut out);
        }
        _ => {
            mask &= !BG_ON_MASK;
            colour(0, COL_SEQ_BG, &mut out);
        }
    }
    (out, mask)
}

/// `set_colour_attribute(atr, fg_bg, TSC_PROMPT)`; an attribute word without
/// the colour's on-flag is the "default colour" reset, 5.9.2's `TXTNOFGCOLOUR`.
fn colour(atr: zattr, fg_bg: i32, out: &mut Vec<Cap>) {
    let seq = set_colour_attribute(atr, fg_bg, 0);
    if !seq.is_empty() {
        out.push(Cap { seq, glitch: 0 });
    }
}

/// 5.9.2 `tsetcap(cap, TSC_PROMPT | (dirty ? TSC_DIRTY : 0))`.
fn tsetcap(cap: i32, dirty: bool, mask: zattr, out: &mut Vec<Cap>) {
    let can = {
        let tclen = crate::ported::init::tclen.lock().unwrap();
        cap >= 0 && (cap as usize) < tclen.len() && tclen[cap as usize] != 0
    };
    let termflags = crate::ported::params::TERMFLAGS.load(Ordering::SeqCst);
    if !can
        || crate::ported::zsh_h::isset(SINGLELINEZLE)
        || termflags & (TERM_NOUP | TERM_BAD | TERM_UNKNOWN) != 0
    {
        return;
    }
    let s = crate::ported::init::tcstr.lock().unwrap()[cap as usize].clone();
    let seq = String::from_utf8_lossy(&crate::shout::tputs(&s)).into_owned();
    let glitch = if cap == TCSTANDOUTBEG || cap == TCSTANDOUTEND {
        crate::terminfo_db::tgetnum("sg")
    } else if cap == TCUNDERLINEBEG || cap == TCUNDERLINEEND {
        crate::terminfo_db::tgetnum("ug")
    } else {
        0
    };
    out.push(Cap { seq, glitch: glitch.max(0) });

    if dirty {
        if mask & TXTBOLDFACE != 0 && cap != TCBOLDFACEBEG {
            tsetcap(TCBOLDFACEBEG, false, mask, out);
        }
        if mask & TXTSTANDOUT != 0 {
            tsetcap(TCSTANDOUTBEG, false, mask, out);
        }
        if mask & TXTUNDERLINE != 0 {
            tsetcap(TCUNDERLINEBEG, false, mask, out);
        }
        if mask & TXTFGCOLOUR != 0 {
            colour(mask, COL_SEQ_FG, out);
        }
        if mask & TXTBGCOLOUR != 0 {
            colour(mask, COL_SEQ_BG, out);
        }
    }
}
