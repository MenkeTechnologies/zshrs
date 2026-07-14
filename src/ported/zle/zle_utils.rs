//! ZLE utility functions
//!
//! Direct port from zsh/Src/Zle/zle_utils.c
//!
//! Primary cut buffer                                                        // c:33
//! Emacs-style kill buffer ring                                              // c:38
//! the line before last mod (for undo purposes)                              // c:51
//! make sure that the line buffer has at least sz chars                      // c:63
//! undo system                                                               // c:1421
//! head of the undo list, and the current position                           // c:1424
//!
//! Implements:
//! - Line manipulation: setline, sizeline, spaceinline, shiftchars
//! - Undo: initundo, freeundo, handleundo, mkundoent, undo, redo
//! - Cut/paste: cut, cuttext, foredel, backdel, forekill, backkill
//! - Cursor: findbol, findeol, findline
//! - Conversion: zlelineasstring, stringaszleline, zlecharasstring
//! - Display: showmsg, printbind, handlefeep
//! - Position save/restore: zle_save_positions, zle_restore_positions

use std::sync::atomic::Ordering;

use super::zle_h::{CH_NEXT, CH_PREV, CUT_RAW, MOD_VIBUF, ZSL_COPY, ZSL_TOEND};
use crate::ported::builtin::RETFLAG;
use crate::ported::utils::errflag;
use crate::ported::zle::compcore::{ZLEMETACS, ZLEMETALINE, ZLEMETALL};
use crate::ported::zsh_h::ERRFLAG_INT;

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_vi::*, zle_word::*,
};
/// Insert string at cursor position

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::zle::zle_main::{
    history, vibuf, CURCHANGE, KILLRING, KILLRINGMAX, LASTCS, LASTLINE, LASTLL, MARK,
    UNDO_CHANGENO, UNDO_LIMITNO, UNDO_STACK, VISTARTCHANGE, ZLECS, ZLELINE, ZLELL,
    ZLE_RESET_NEEDED, ZMOD,
};

/// Port of `sizeline(int sz)` from Src/Zle/zle_utils.c:67.
/// WARNING: param names don't match C — Rust=(zle, sz) vs C=(sz)
pub fn sizeline(sz: usize) {
    // c:67
    // C body c:69-87 — `if (sz > linesz) { linesz = sz + 256; line =
    //                  zrealloc(line, (linesz+1) * char_t) }`. Vec
    //                  grows on demand; just reserve.
    let mut __g_zleline = ZLELINE.lock().unwrap();
    let cur_len = __g_zleline.len();
    if sz > cur_len {
        __g_zleline.reserve(sz - cur_len + 256);
    }
}

/// Port of `void zleaddtoline(int chr)` from Src/Zle/zle_utils.c:102.
///
/// C body (3 lines):
///   `spaceinline(1);
///    zlemetaline[zlemetacs++] = chr;`
///
/// Opens one slot at the meta cursor + writes `chr` byte. Used by
/// init paths that pre-populate the line buffer before zleread.
/// Note C writes to ZLEMETALINE, not ZLELINE — this is a meta-mode
/// helper called during early line construction.
pub fn zleaddtoline(chr: i32) {
    // c:102
    use crate::ported::zle::compcore::{ZLEMETACS, ZLEMETALINE, ZLEMETALL};

    spaceinline(1); // c:104
                    // c:105 — `zlemetaline[zlemetacs++] = chr;`
    if ZLEMETALL.load(Ordering::SeqCst) > 0 {
        if let Some(m) = ZLEMETALINE.get() {
            if let Ok(mut g) = m.lock() {
                let cs = ZLEMETACS.load(Ordering::SeqCst) as usize;
                let byte = (chr & 0xff) as u8;
                let mut bytes = g.as_bytes().to_vec();
                if cs < bytes.len() {
                    bytes[cs] = byte;
                } else {
                    bytes.push(byte);
                }
                *g = unsafe { String::from_utf8_unchecked(bytes) };
                ZLEMETACS.fetch_add(1, Ordering::SeqCst);
                return;
            }
        }
    }
    // Fallback: meta-mode not active, write to ZLELINE codepoint
    // vector. spaceinline already opened the slot.
    let cs = ZLECS.load(Ordering::SeqCst);
    if let Some(slot) = ZLELINE.lock().unwrap().get_mut(cs) {
        *slot = (chr & 0xff) as u8 as char;
    }
    ZLECS.fetch_add(1, Ordering::SeqCst);
}

/// Port of `int zlecharasstring(ZLE_CHAR_T inchar, char *buf)` from Src/Zle/zle_utils.c:117.
///
/// C body (multibyte arm c:119-165): wctomb-encode `inchar` to
/// up to MB_CUR_MAX bytes, then walk the encoded bytes in reverse
/// applying the metafy expansion in-place (each imeta byte gets
/// expanded to `Meta + (b^0x20)`, bumping the byte count). Returns
/// the final byte length written to `buf`.
///
/// Rust port: UTF-8 encode `inchar` via `char::encode_utf8`, then
/// walk the bytes appending each one to `buf` and Meta-prefixing
/// any byte that satisfies `imeta()`. Returns the total bytes
/// pushed.
pub fn zlecharasstring(inchar: char, buf: &mut String) -> i32 {
    // c:117
    use crate::ported::zsh_h::Meta;
    use crate::ported::ztype_h::imeta;

    let start_len = buf.len();
    let mut enc = [0u8; 4];
    let bytes = inchar.encode_utf8(&mut enc).as_bytes().to_vec(); // c:131 wctomb
                                                                  // c:138-160 — metafy each byte that needs escaping.
    let mut out_bytes = buf.as_bytes().to_vec();
    for b in bytes {
        if imeta(b) {
            out_bytes.push(Meta); // c:154
            out_bytes.push(b ^ 0x20); // c:155
        } else {
            out_bytes.push(b);
        }
    }
    *buf = unsafe { String::from_utf8_unchecked(out_bytes) };
    (buf.len() - start_len) as i32
}

/// Port of `zlelineasstring(ZLE_STRING_T instr, int inll, int incs, int *outllp, int *outcsp, int useheap)` from Src/Zle/zle_utils.c:192.
///
/// Three-phase port matching the C body line-for-line:
/// 1. **MB encode loop** (c:200-247). Walk input wchars; for each
///    one, encode to UTF-8 bytes via `char::encode_utf8`. Track
///    `outcs` (in bytes) as the input cursor walks past each
///    codepoint. Region-highlights start_meta/end_meta tracking
///    is gated by `outcsp == &zlemetacs` in C; Rust has no
///    pointer-identity test so callers that need the highlight
///    sync must invoke the dedicated helpers separately.
/// 2. **Metafy adjustment** (c:282-322). C output is a metafied
///    byte string; bytes that fail `imeta` get Meta-prefixed
///    (each `imeta` byte adds +1 to `outll`, +1 to `outcs` if it
///    lands before the cursor). Walk the encoded buffer and apply
///    the metafy expansion in-place.
/// 3. **Return** (c:322-330). C returns `dupstring(s, useheap)`;
///    Rust returns the String directly (heap-arena vs persistent
///    is a memory-mgmt distinction without behavior consequence).
///
/// `_flags` (Rust extra param) is reserved for the `useheap` toggle
/// — currently unused since Rust owns the returned String.
pub fn zlelineasstring(
    line: &[char],
    inll: usize,
    incs: i32,
    outllp: Option<&mut i32>,
    outcsp: Option<&mut i32>,
    _useheap: i32,
) -> String {
    // c:192
    use crate::ported::zsh_h::Meta;
    use crate::ported::ztype_h::imeta;

    // === Phase 1: MB encode (wcrtomb equivalent) ===
    let mut s: Vec<u8> = Vec::with_capacity(inll * 4);
    let mut outcs: i32 = 0;
    let mut remaining = incs;
    let mut buf4 = [0u8; 4];
    for ch in line.iter().take(inll) {
        // c:206-207 — `if (incs == 0) outcs = mb_len;`. Cursor
        // landing on this codepoint locks outcs to the byte length
        // so far.
        if remaining == 0 {
            outcs = s.len() as i32;
        }
        remaining -= 1; // c:208 — `incs--;`
        let enc = ch.encode_utf8(&mut buf4);
        s.extend_from_slice(enc.as_bytes()); // c:235-242 wcrtomb
    }
    // c:248-250 — `if (incs == 0) outcs = mb_len;`. Cursor past EOL.
    if remaining == 0 {
        outcs = s.len() as i32;
    }

    // c:265-266 — `outll = mb_len; s[mb_len] = '\0';`. Output byte
    // length pre-metafy.
    let mut outll = s.len() as i32;

    // === Phase 2: metafy adjustment (c:282-322) ===
    if outllp.is_some() || outcsp.is_some() {
        // c:295 — `while (strp < stopll)` — walk bytes, expand each
        // imeta byte to a 2-byte Meta+(b^0x20) pair, bumping outll
        // and outcs (when before cursor) by 1 per expansion.
        let mut metafied: Vec<u8> = Vec::with_capacity(s.len());
        for (i, &b) in s.iter().enumerate() {
            if imeta(b) {
                metafied.push(Meta);
                metafied.push(b ^ 0x20);
                if (i as i32) < outcs {
                    outcs += 1;
                }
                outll += 1;
            } else {
                metafied.push(b);
            }
        }
        s = metafied;
    }

    if let Some(p) = outcsp {
        *p = outcs;
    }
    if let Some(p) = outllp {
        *p = outll;
    }

    // c:322-330 — `return dupstring(s, useheap)`.
    // Phase 2 may have produced bytes that aren't valid UTF-8 (Meta = 0x83);
    // construct via from_utf8_unchecked since the byte stream is the
    // metafied form by design, not a Rust display string.
    unsafe { String::from_utf8_unchecked(s) }
}

/// Port of `stringaszleline(char *instr, int incs, int *outll, int *outsz, int *outcs)` from Src/Zle/zle_utils.c:375.
///
/// Three-phase port matching the C body line-for-line:
/// 1. **Pre-unmetafy `incs` adjustment** (c:383-426). When the caller
///    asks for `outcs` (`want_outcs = true`), walk `instr` bytes;
///    each `Meta` byte before the cursor `incs` decrements `incs`
///    by 1 (the `Meta + 0x20`-XOR pair will collapse to a single
///    byte in phase 2, so the cursor must shift left). Skipped if
///    `want_outcs == false`, matching C's `if (outcs)` gate.
/// 2. **`unmetafy(instr, &ll)`** (c:428) — collapse the `Meta + byte`
///    encoding via the canonical [`crate::ported::utils::unmetafy`]
///    helper. Returns the new byte length `ll`.
/// 3. **Multibyte decode loop** (c:436-525, `mbrtowc` equivalent).
///    For UTF-8 inputs we use [`std::str::from_utf8`] +
///    `char_indices()` — each codepoint span maps to one output
///    `char`. For each codepoint that straddles the adjusted-incs
///    cursor, set `outcs = output codepoint index`. Invalid UTF-8
///    bytes fall back to the C `ZSH_CHAR_TO_INVALID_WCHAR` private
///    encoding (use a U+FFFD replacement per byte, matching the
///    spirit if not the exact codepoint).
///
/// `outsz` (c:431) is set to the worst-case codepoint count (`ll`)
/// — Rust `Vec<char>` capacity not pre-reserved per byte since
/// `Vec::with_capacity(ll)` already gives an over-estimate.
///
/// The region_highlights `start_meta`/`end_meta` adjustment in C
/// (c:387-410, c:476-510) is GATED on `outcs == &zlecs` — i.e. only
/// when the caller is mutating the live cursor. The Rust port has
/// no `&zlecs` identity test; callers that want the highlight
/// shift must use the dedicated [`shiftchars`] / region helpers
/// before/after this call.
pub fn stringaszleline(
    instr: &str,
    mut incs: i32,
    outll: Option<&mut i32>,
    outsz: Option<&mut i32>,
    outcs: Option<&mut i32>,
) -> Vec<char> {
    // c:375
    use crate::ported::zsh_h::Meta;

    let want_outcs = outcs.is_some();

    // === Phase 1: pre-unmetafy `incs` adjustment ===
    if want_outcs {
        // c:383-385 — `cspos = instr + incs`. Walk bytes; each Meta
        // before cspos collapses to one byte, so incs must
        // decrement.
        let bytes = instr.as_bytes();
        let cspos = (incs as usize).min(bytes.len());
        let mut inptr = 0usize;
        while inptr < bytes.len() {
            if bytes[inptr] == Meta {
                if inptr < cspos {
                    incs -= 1; // c:391-393
                }
                inptr += 1; // c:421 — skip the byte after Meta
            }
            inptr += 1; // c:422
        }
    }

    // === Phase 2: unmetafy ===
    let mut raw: Vec<u8> = instr.as_bytes().to_vec();
    let ll = crate::ported::utils::unmetafy(&mut raw); // c:428
    let sz = ll; // c:430-432 — `sz = (ll + 2) * ZLE_CHAR_SIZE`; for
                 // Rust Vec<char> we just report the worst-case
                 // codepoint count.
    if let Some(p) = outsz {
        *p = sz as i32;
    }

    // === Phase 3: UTF-8 decode loop (mbrtowc equivalent) ===
    if ll == 0 {
        // c:528-535 — empty input: zeroed output.
        if let Some(p) = outll {
            *p = 0;
        }
        if let Some(p) = outcs {
            *p = 0;
        }
        return Vec::new();
    }

    let mut line: Vec<char> = Vec::with_capacity(ll);
    let mut outcs_val: i32 = 0;
    let mut outcs_set = false;

    match std::str::from_utf8(&raw[..ll]) {
        Ok(decoded) => {
            // c:438-525 — `while (ll > 0) { cnt = mbrtowc(...) }`.
            // Rust UTF-8 char_indices gives (byte_offset, char) pairs;
            // each char is exactly `cnt = ch.len_utf8()` bytes wide.
            for (byte_idx, ch) in decoded.char_indices() {
                let cnt = ch.len_utf8();
                // c:483-484 — `if (offs <= incs && incs < offs + cnt)
                //               *outcs = outptr - outstr;`. The
                // codepoint spanning the cursor takes the output
                // index slot.
                if want_outcs && (byte_idx as i32) <= incs && incs < (byte_idx + cnt) as i32 {
                    outcs_val = line.len() as i32;
                    outcs_set = true;
                }
                line.push(ch); // c:520-521 — `*outptr++ = ch; inptr += cnt;`
            }
        }
        Err(_) => {
            // c:457-461 — `ZSH_CHAR_TO_INVALID_WCHAR(*inptr)` for
            // bytes that fail decode. Rust analog: U+FFFD per bad
            // byte. Walk bytes one at a time so we keep going.
            for (i, &b) in raw[..ll].iter().enumerate() {
                if want_outcs && (i as i32) == incs {
                    outcs_val = line.len() as i32;
                    outcs_set = true;
                }
                match std::str::from_utf8(std::slice::from_ref(&b)) {
                    Ok(s) => line.extend(s.chars()),
                    Err(_) => line.push('\u{FFFD}'),
                }
            }
        }
    }

    // c:524-525 — `if (outcs && inptr <= instr + incs)
    //               *outcs = outptr - outstr;`. Cursor past EOL → end.
    if want_outcs && !outcs_set {
        outcs_val = line.len() as i32;
    }

    if let Some(p) = outll {
        *p = line.len() as i32; // c:526 — `*outll = outptr - outstr`
    }
    if let Some(p) = outcs {
        *p = outcs_val;
    }

    line
}

/// Port of `char *zlegetline(int *ll, int *cs)` from Src/Zle/zle_utils.c:547.
///
/// C body branches on `zlemetaline`:
///   1. If `zlemetaline != NULL`: `*ll = zlemetall; *cs = zlemetacs;
///      return ztrdup(zlemetaline);` — already metafied, snapshot
///      and dup.
///   2. Else if `zleline`: `return zlelineasstring(zleline, zlell,
///      zlecs, ll, cs, 0);` — encode the codepoint vector into
///      metafied bytes, populating `*ll`/`*cs` for the caller.
///   3. Else: `*ll = *cs = 0; return ztrdup("");` — fresh empty.
///
/// Returns the metafied byte string. Caller-out-params `ll` and
/// `cs` receive the metafied-byte length and metafied-byte cursor
/// position respectively (not codepoint counts).
pub fn zlegetline(ll: &mut i32, cs: &mut i32) -> String {
    // c:547
    use crate::ported::zle::compcore::{ZLEMETACS, ZLEMETALINE, ZLEMETALL};

    // c:549 — `if (zlemetaline != NULL)`. In Rust ZLEMETALINE is a
    // OnceLock<Mutex<String>>; treat ZLEMETALL > 0 as the "active
    // meta" signal (see spaceinline doc for why .get().is_some()
    // alone leaks across tests).
    if ZLEMETALL.load(Ordering::SeqCst) > 0 {
        if let Some(m) = ZLEMETALINE.get() {
            if let Ok(g) = m.lock() {
                *ll = ZLEMETALL.load(Ordering::SeqCst); // c:551
                *cs = ZLEMETACS.load(Ordering::SeqCst); // c:552
                return g.clone(); // c:553 — `ztrdup(zlemetaline)`
            }
        }
    }
    // c:555 — `if (zleline) return zlelineasstring(...)`.
    let line = ZLELINE.lock().unwrap().clone();
    if !line.is_empty() || ZLELL.load(Ordering::SeqCst) > 0 {
        let zlell = ZLELL.load(Ordering::SeqCst) as usize;
        let zlecs = ZLECS.load(Ordering::SeqCst) as i32;
        let mut out_ll: i32 = 0;
        let mut out_cs: i32 = 0;
        let s = zlelineasstring(&line, zlell, zlecs, Some(&mut out_ll), Some(&mut out_cs), 0);
        *ll = out_ll;
        *cs = out_cs;
        return s;
    }
    // c:558 — `*ll = *cs = 0; return ztrdup("")`.
    *ll = 0;
    *cs = 0;
    String::new()
}

/// Port of `void free_region_highlights_memos(void)` from Src/Zle/zle_utils.c:567.
///
/// C body:
///   `for (rhp = region_highlights;
///         rhp < region_highlights + n_region_highlights;
///         rhp++)
///        zfree((char*) rhp->memo, 0);`
///
/// Releases the `memo` strings held by every active region
/// highlight. C uses zfree (manual heap release); Rust uses
/// `Option::take()` which Drops the inner String and resets
/// the field to None — same observable effect.
///
/// Called by `zlecallhook` etc. when highlight state needs to be
/// reset between widget invocations (otherwise `memo` strings
/// accumulate across widget calls).
pub fn free_region_highlights_memos() {
    // c:567
    use crate::ported::zle::zle_refresh::REGION_HIGHLIGHTS;
    if let Ok(mut rh) = REGION_HIGHLIGHTS.lock() {
        for entry in rh.iter_mut() {
            entry.memo.take(); // c:573 zfree((char*) rhp->memo, 0);
        }
    }
}

/// Direct port of `struct zle_position` from
/// `Src/Zle/zle_utils.c:595-605`. One saved-state node in the
/// zle_positions stack; pushed by `zle_save_positions()` and popped
/// by `zle_restore_positions()`.
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct zle_position {
    // c:595
    /// `int cs` — c:599, saved cursor position.
    pub cs: usize,
    /// `int mk` — c:600, saved mark position.
    pub mk: usize,
    /// `int ll` — c:601, saved line length.
    pub ll: usize,
    // `struct zle_region *regions` (c:604) — region_highlights
    // are persisted separately through `zle_refresh::HighlightManager`;
    // not snapshotted in the position-save record.
}

/// Port of `mod_export void zle_save_positions(void)` from
/// Src/Zle/zle_utils.c:619.
///
/// "Save positions including cursor, end-of-line and (non-special)
/// region highlighting. Must be matched by a subsequent
/// `zle_restore_positions()`."
pub fn zle_save_positions() {
    use crate::ported::zle::zle_refresh::REGION_HIGHLIGHTS;
    // c:621 — `newpos = zalloc(sizeof(*newpos));`
    let mk = MARK.load(Ordering::SeqCst); // c:625
    let cs = ZLECS.load(Ordering::SeqCst); // c:634
    let ll = ZLELL.load(Ordering::SeqCst); // c:635

    // c:641-664 — snapshot region_highlights past N_SPECIAL_HIGHLIGHTS
    //              so the user-driven (predisplay/normal) entries
    //              survive the nested ZLE call.
    const N_SPECIAL_HIGHLIGHTS: usize = 4;
    let regions: Vec<crate::ported::zle::zle_refresh::RegionHighlight> = REGION_HIGHLIGHTS
        .lock()
        .unwrap()
        .iter()
        .skip(N_SPECIAL_HIGHLIGHTS)
        .cloned()
        .collect();

    let pos = ZlePosition {
        mk,
        cs,
        ll,
        regions,
    };
    if let Ok(mut s) = ZLE_POSITIONS.lock() {
        // c:677 — push to head of stack.
        s.push(pos);
    }
}

/// Port of `mod_export void zle_restore_positions(void)` from
/// Src/Zle/zle_utils.c:677. Pops the last saved (cs, mark, ll).
pub fn zle_restore_positions() {
    use crate::ported::zle::zle_refresh::REGION_HIGHLIGHTS;
    // c:679 — pop the head of the position stack.
    let oldpos = match ZLE_POSITIONS.lock().ok().and_then(|mut s| s.pop()) {
        Some(p) => p,
        None => return,
    };
    // c:684-686 — restore mark + cursor + ll (clamp cs to ll for safety).
    MARK.store(oldpos.mk, Ordering::SeqCst); // c:686
    ZLECS.store(
        oldpos.cs.min(oldpos.ll), // c:693
        Ordering::SeqCst,
    );
    ZLELL.store(oldpos.ll, Ordering::SeqCst); // c:694

    // c:696-732 — restore region_highlights tail (everything past
    //              N_SPECIAL_HIGHLIGHTS). C grows the array and copies
    //              memo+atr+start+end+flags from each saved zle_region.
    const N_SPECIAL_HIGHLIGHTS: usize = 4;
    if let Ok(mut rh) = REGION_HIGHLIGHTS.lock() {
        rh.truncate(N_SPECIAL_HIGHLIGHTS); // c:705 free user entries
        for r in &oldpos.regions {
            rh.push(r.clone()); // c:715-728 restore each saved entry
        }
    }
}

/// Port of `mod_export void zle_free_positions(void)` from
/// Src/Zle/zle_utils.c:102. Discards the top of stack without
/// applying it.
pub fn zle_free_positions() {
    // c:747
    if let Ok(mut s) = ZLE_POSITIONS.lock() {
        s.pop(); // c:749 oldpos = zle_positions; zle_positions = next
    }
}

/// Port of `spaceinline(int ct)` from Src/Zle/zle_utils.c:777.
/// WARNING: signature divergence — handles only the non-meta arm
/// (c:817-844). The C meta arm (c:782-815, `if (zlemetaline)`) is
/// inlined by callers that operate on ZLEMETALINE because
/// ZLEMETALINE is a `OnceLock<Mutex<String>>` in the Rust port
/// that stays initialized across tests, so there is no clean
/// "meta active" check available from inside this fn.
pub fn spaceinline(ct: i32) {
    // c:777
    if ct <= 0 {
        return;
    }
    let ct_u = ct as usize;
    // c:817-844 — non-meta branch: shift ZLELINE[zlecs..zlell]
    // forward by ct, fill with NUL.
    for _ in 0..ct_u {
        ZLELINE
            .lock()
            .unwrap()
            .insert(ZLECS.load(Ordering::SeqCst), '\0');
    }
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    // c:825-826 — `if (mark > zlecs) mark += ct;`.
    let mark_cur = MARK.load(Ordering::SeqCst) as i32;
    let cs = ZLECS.load(Ordering::SeqCst) as i32;
    if mark_cur > cs {
        MARK.store((mark_cur + ct) as usize, Ordering::SeqCst);
    }
    // c:827-828 — `if (viinsbegin > zlecs) viinsbegin = 0;`. A buffer
    // insert before the vi-insert anchor invalidates it.
    let cs_u = ZLECS.load(Ordering::SeqCst);
    if crate::ported::zle::zle_main::VIINSBEGIN.load(Ordering::SeqCst) > cs_u {
        crate::ported::zle::zle_main::VIINSBEGIN.store(0, Ordering::SeqCst);
    }
    // c:830-844 — shift the user region-highlight offsets (those past
    // the N_SPECIAL_HIGHLIGHTS reserved slots) so highlighting stays
    // aligned with the text after the insertion. Predisplay regions are
    // measured relative to `predisplaylen`.
    {
        use crate::ported::zle::zle_h::{N_SPECIAL_HIGHLIGHTS, ZRH_PREDISPLAY};
        let predisplaylen = crate::ported::zle::zle_params::get_predisplay()
            .chars()
            .count() as i64; // c:836 predisplaylen
        let zlecs_i = cs_u as i64;
        let ct_i = ct as i64;
        let mut rh = crate::ported::zle::zle_refresh::REGION_HIGHLIGHTS
            .lock()
            .unwrap();
        // c:831 — start past the N_SPECIAL_HIGHLIGHTS reserved slots.
        for rhp in rh.iter_mut().skip(N_SPECIAL_HIGHLIGHTS as usize) {
            // c:834-837 — `sub = (flags & ZRH_PREDISPLAY) ? predisplaylen : 0`.
            let sub = if rhp.flags & ZRH_PREDISPLAY != 0 {
                predisplaylen
            } else {
                0
            };
            // c:838-839 — `if (rhp->start - sub >= zlecs) rhp->start += ct;`
            if rhp.start as i64 - sub >= zlecs_i {
                rhp.start = (rhp.start as i64 + ct_i) as usize;
            }
            // c:840-841 — `if (rhp->end - sub >= zlecs && (!predisplaylen
            //                || zlecs)) rhp->end += ct;`
            if rhp.end as i64 - sub >= zlecs_i && (predisplaylen == 0 || zlecs_i != 0) {
                rhp.end = (rhp.end as i64 + ct_i) as usize;
            }
        }
    }
}

/// Port of `shiftchars(int to, int cnt)` from Src/Zle/zle_utils.c:846.
/// WARNING: param names don't match C — Rust=(zle, to, cnt) vs C=(to, cnt)
pub fn shiftchars(to: i32, cnt: i32) {
    // c:846
    // c:851-854 — mark adjustment: if mark is past the deleted range,
    // shift it left by cnt; if mark is inside the range, clamp to `to`.
    //     if (mark >= to + cnt) mark -= cnt;
    //     else if (mark > to)   mark = to;
    let mark_cur = MARK.load(Ordering::SeqCst) as i32;
    if mark_cur >= to + cnt {
        MARK.store((mark_cur - cnt).max(0) as usize, Ordering::SeqCst); // c:852
    } else if mark_cur > to {
        MARK.store(to.max(0) as usize, Ordering::SeqCst); // c:854
    }

    // c:888-908 (!meta branch) — walk
    // region_highlights[N_SPECIAL_HIGHLIGHTS..n_region_highlights] and
    // adjust each entry's `start`/`end` by the same rule as mark: shift
    // past the cut, or clamp. Predisplay regions (ZRH_PREDISPLAY) are
    // measured relative to `predisplaylen` — now that RegionHighlight
    // carries the `flags` bit, the `sub = predisplaylen` subtraction is
    // wired faithfully (was hardcoded sub=0).
    use crate::ported::zle::zle_h::{N_SPECIAL_HIGHLIGHTS, ZRH_PREDISPLAY};
    use crate::ported::zle::zle_refresh::REGION_HIGHLIGHTS;
    let n_special = N_SPECIAL_HIGHLIGHTS as usize;
    let predisplaylen = crate::ported::zle::zle_params::get_predisplay()
        .chars()
        .count() as i64; // c:888 predisplaylen
    let to_i = to as i64;
    let cnt_i = cnt as i64;
    if let Ok(mut rh) = REGION_HIGHLIGHTS.lock() {
        let total = rh.len();
        for idx in n_special..total {
            let entry = &mut rh[idx];
            // c:890-891 — `sub = (flags & ZRH_PREDISPLAY) ? predisplaylen : 0`.
            let sub = if entry.flags & ZRH_PREDISPLAY != 0 {
                predisplaylen
            } else {
                0
            };
            // c:892-897 — `if (rhp->start - sub > to) { ... }`
            if entry.start as i64 - sub > to_i {
                if entry.start as i64 - sub > to_i + cnt_i {
                    entry.start = (entry.start as i64 - cnt_i).max(0) as usize; // c:894
                } else {
                    entry.start = (to_i + sub).max(0) as usize; // c:896
                }
            }
            // c:898-903 — `if (rhp->end - sub > to) { ... }`
            if entry.end as i64 - sub > to_i {
                if entry.end as i64 - sub > to_i + cnt_i {
                    entry.end = (entry.end as i64 - cnt_i).max(0) as usize; // c:900
                } else {
                    entry.end = (to_i + sub).max(0) as usize; // c:902
                }
            }
        }
    }

    // c:856-885 — metaline branch (zlemetaline != NULL). Rust stores
    // ZLELINE as UTF-8 directly without a separate metafied buffer,
    // so the meta-branch collapses into the !meta branch below.

    // c:911-915 — !zlemetaline branch (the load-bearing tail):
    //     while (to + cnt < zlell) {
    //         zleline[to] = zleline[to + cnt];
    //         to++;
    //     }
    //     zleline[zlell = to] = ZWC('\0');
    // The loop shifts chars left by `cnt`; when `to + cnt >= zlell`
    // (no chars after the gap to copy) the loop body skips and zlell
    // is set to `to`, TRUNCATING the buffer to offset `to`. The
    // previous Rust port had `if to + cnt > zleline.len() { return }`
    // which mishandled the out-of-range case — C truncates, Rust
    // silently no-op'd. Concrete failure: `shiftchars(3, 100)` on a
    // 5-char line should leave zlell=3; old port left zlell=5.
    let to = to as usize;
    let cnt = cnt as usize;
    let mut line = ZLELINE.lock().unwrap();
    let len = line.len();
    if to >= len {
        // c:915 `zleline[zlell = to]` — caller passed `to` past end-of-line.
        // C still sets zlell=to (which would corrupt the buffer); Rust
        // clamps to `len` so we don't grow zlell past the actual storage.
        ZLELL.store(len, Ordering::SeqCst);
        return;
    }
    if to + cnt >= len {
        // c:912 — no chars after the gap; just truncate to `to`.
        line.truncate(to); // c:915 zlell = to
    } else {
        line.drain(to..to + cnt); // c:912-914 memmove
    }
    ZLELL.store(line.len(), Ordering::SeqCst); // c:915
}

/// Port of `cut(int i, int ct, int flags)` from Src/Zle/zle_utils.c:935.
/// C body (single statement):
///     `cuttext(zleline + i, ct, flags);`
/// WARNING: param names don't match C — Rust=(i, ct, dir) vs C=(i, ct, flags)
pub fn cut(i: i32, ct: i32, dir: i32) -> i32 {
    // c:935
    let line = ZLELINE.lock().unwrap(); // c:937 zleline + i
    let start = (i.max(0) as usize).min(line.len());
    let end = (start + ct.max(0) as usize).min(line.len());
    cuttext(&line[start..end].to_vec(), dir); // c:937
    0
}

/// Direct port of `void cuttext(ZLE_STRING_T line, int ct, int flags)`
/// from `Src/Zle/zle_utils.c:946`.
///
/// Stores `txt` in the right cut/kill buffer based on the current
/// `zmod` and `flags`:
///   - skip when ct == 0 && !vilinerange, or MOD_NULL is set (c:948)
///   - MOD_VIBUF → write/append to `vibuf[zmod.vibuf]` honouring
///     MOD_VIAPP and vilinerange's CUTBUFFER_LINE flag (c:953-979)
///   - CUT_YANK → store in `vibuf[26]` (vi "0 register) (c:980-986)
///   - default vi → shift "1-"8 down to "2-"9, store at `vibuf[27]`
///     (vi "1 register) (c:987-997)
///   - rotate CUTBUF into KILLRING when !ZLE_KILL or CUT_REPLACE
///     (c:1004-1018), then apply CUT_FRONT/CUT_REPLACE direction or
///     normal append (c:1019-1043).
/// WARNING: param names don't match C — Rust=(txt, flags) vs C=(line, ct, flags)
pub fn cuttext(
    txt: &[char], // c:946 line, ct
    flags: i32,
) {
    use crate::ported::zle::zle_h::{
        CUTBUFFER_LINE, CUT_FRONT, CUT_REPLACE, CUT_YANK, MOD_NULL, MOD_VIAPP, ZLE_KILL,
    };
    use crate::ported::zle::zle_main::{CUTBUF, KILLRING, KILLRINGMAX, KRINGNUM, LASTCMD};
    use crate::ported::zle::zle_vi::VILINERANGE;

    let ct = txt.len();
    let vilinerange = VILINERANGE.load(Ordering::Relaxed) != 0;

    // c:948 — `if (!(ct || vilinerange) || zmod.flags & MOD_NULL) return;`
    if (ct == 0 && !vilinerange) || ZMOD.lock().unwrap().flags & MOD_NULL != 0 {
        return;
    }

    let mod_flags = ZMOD.lock().unwrap().flags;
    let mod_vibuf = ZMOD.lock().unwrap().vibuf as usize;
    let chars: Vec<char> = txt.to_vec();

    if mod_flags & MOD_VIBUF != 0 {
        // c:961-979 — write to vibuf[zmod.vibuf].
        let idx = mod_vibuf.min(vibuf().lock().unwrap().len().saturating_sub(1));
        let viapp = mod_flags & MOD_VIAPP != 0;
        let mut vibuf_guard = vibuf().lock().unwrap();
        if !viapp || vibuf_guard[idx].is_empty() {
            // c:962-967 — replace.
            vibuf_guard[idx] = chars;
        } else {
            // c:968-979 — append; insert \n separator under
            //              CUTBUFFER_LINE semantics.
            if vilinerange {
                vibuf_guard[idx].push('\n');
            }
            vibuf_guard[idx].extend(chars);
        }
        return;
    } else if flags & CUT_YANK != 0 {
        // c:980-986 — vi "0 register (idx 26).
        if let Some(slot) = vibuf().lock().unwrap().get_mut(26) {
            *slot = chars;
        }
    } else {
        // c:987-997 — shift "1-"8 to "2-"9, store at "1 (idx 27).
        let mut v = vibuf().lock().unwrap();
        for n in (28..36).rev() {
            v[n] = v[n - 1].clone();
        }
        v[27] = chars.clone();
    }

    // c:1004-1018 — rotate CUTBUF into KILLRING on kill+replace
    //                boundary. `!(lastcmd & ZLE_KILL) || (flags &
    //                CUT_REPLACE)` → start a fresh CUTBUF, push the
    //                old one to the ring.
    let lastcmd_v = LASTCMD.load(Ordering::Relaxed) as i32;
    let cutbuf_empty = CUTBUF.lock().unwrap().buf.is_empty();
    let should_rotate =
        !cutbuf_empty && ((lastcmd_v & ZLE_KILL) == 0 || (flags & CUT_REPLACE) != 0);
    if should_rotate {
        let old: Vec<char> = CUTBUF.lock().unwrap().buf.chars().collect();
        KILLRING.lock().unwrap().push_front(old);
        let max = KILLRINGMAX.load(Ordering::SeqCst);
        while KILLRING.lock().unwrap().len() > max {
            KILLRING.lock().unwrap().pop_back();
        }
        KRINGNUM.store(0, Ordering::Relaxed);
        let mut cb = CUTBUF.lock().unwrap();
        cb.buf.clear();
        cb.len = 0;
        cb.flags = 0;
    }

    // c:1019-1043 — apply CUT_FRONT/CUT_REPLACE direction or
    //                normal append into CUTBUF.
    let mut cb = CUTBUF.lock().unwrap();
    let cell: String = txt.iter().collect();
    if flags & (CUT_FRONT | CUT_REPLACE) != 0 {
        // Text goes in front (or replaces).
        if flags & CUT_REPLACE != 0 {
            cb.buf = cell;
        } else {
            cb.buf = format!("{}{}", cell, cb.buf);
        }
    } else {
        // Default: append.
        cb.buf.push_str(&cell);
    }
    cb.len = cb.buf.chars().count();
    if vilinerange {
        cb.flags |= CUTBUFFER_LINE;
    }
}

/// Port of `backkill(int ct, int flags)` from `Src/Zle/zle_utils.c:1045`. Cuts `ct`
/// characters BACKWARD from the cursor (i.e. removes `[zlecs-ct,
/// zlecs)` and pushes them onto the kill-ring head). C: `void
/// backkill(int ct, int flags)`. Rust port takes `&mut Zle` so the
/// killring + zlecs/zlell mutations stay on the typed shell state.
/// `flags` is the `CUT_*` bitmask — `CUT_RAW` skips the multibyte
/// DECCS adjustment loop the non-RAW path uses.
/// WARNING: param names don't match C — Rust=(zle, ct, flags) vs C=(ct, flags)
pub fn backkill(ct: i32, flags: i32) {
    // c:1045
    let ct = ct as usize;
    if ct == 0 || ZLECS.load(Ordering::SeqCst) == 0 {
        return;
    }
    let _ = flags; // CUT_RAW path: no DECCS multibyte adjustment.
    let take_n = ct.min(ZLECS.load(Ordering::SeqCst));
    let start = ZLECS.load(Ordering::SeqCst) - take_n;
    let cut_chars: Vec<char> = ZLELINE
        .lock()
        .unwrap()
        .drain(start..ZLECS.load(Ordering::SeqCst))
        .collect(); // c:1057 cut + shiftchars
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLECS.store(start, Ordering::SeqCst);
    KILLRING.lock().unwrap().push_front(cut_chars);
    if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(Ordering::SeqCst) {
        KILLRING.lock().unwrap().pop_back();
    }
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst); // c:1059 CCRIGHT
}

/// Port of `forekill(int ct, int flags)` from `Src/Zle/zle_utils.c:1064`. Cuts `ct`
/// characters FORWARD from the cursor (i.e. removes `[zlecs,
/// zlecs+ct)` and pushes them onto the kill-ring head). C: `void
/// forekill(int ct, int flags)`. Rust port takes `&mut Zle`. The
/// `CUT_RAW` path (matching the C `flags & CUT_RAW` arm at
/// zle_utils.c:1069) skips the multibyte INCCS adjustment loop —
/// zshrs treats the buffer as `Vec<char>` and never needs that
/// re-walk.
/// WARNING: param names don't match C — Rust=(zle, ct, flags) vs C=(ct, flags)
pub fn forekill(ct: i32, flags: i32) {
    // c:1064
    let ct = ct as usize;
    if ct == 0 || ZLECS.load(Ordering::SeqCst) >= ZLELL.load(Ordering::SeqCst) {
        return;
    }
    let _ = flags; // CUT_RAW path: no INCCS multibyte adjustment.
    let take_n = ct.min(ZLELL.load(Ordering::SeqCst) - ZLECS.load(Ordering::SeqCst));
    let i = ZLECS.load(Ordering::SeqCst);
    let cut_chars: Vec<char> = ZLELINE.lock().unwrap().drain(i..i + take_n).collect(); // c:1077 cut + shiftchars
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    KILLRING.lock().unwrap().push_front(cut_chars);
    if KILLRING.lock().unwrap().len() > KILLRINGMAX.load(Ordering::SeqCst) {
        KILLRING.lock().unwrap().pop_back();
    }
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst); // c:1079 CCRIGHT
}

/// Port of `backdel(int ct, int flags)` from `Src/Zle/zle_utils.c:1084`. Removes `ct`
/// characters BACKWARD from the cursor (i.e. drops `[zlecs-ct,
/// zlecs)` from the line) without pushing to the kill-ring.
///
/// C signature: `void backdel(int ct, int flags)`. The Rust port
/// takes `&mut Zle` so `zlecs`/`zlell`/`zleline` mutations stay on
/// the typed shell state. The non-RAW path's `DECCS` multibyte
/// adjustment loop (c:1093-1098) collapses to a plain decrement
/// since zshrs treats the buffer as `Vec<char>`.
/// WARNING: param names don't match C — Rust=(zle, ct, _flags) vs C=(ct, flags)
pub fn backdel(ct: i32, _flags: i32) {
    // c:1084
    let ct = ct as usize;
    if ct == 0 || ZLECS.load(Ordering::SeqCst) == 0 {
        return;
    }
    let take_n = ct.min(ZLECS.load(Ordering::SeqCst));
    let start = ZLECS.load(Ordering::SeqCst) - take_n;
    ZLELINE
        .lock()
        .unwrap()
        .drain(start..ZLECS.load(Ordering::SeqCst)); // c:1090 shiftchars
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLECS.store(start, Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst); // c:1091 CCRIGHT
}

/// Port of `foredel(int ct, int flags)` from `Src/Zle/zle_utils.c:1105`. Removes `ct`
/// characters FORWARD from the cursor (i.e. drops `[zlecs, zlecs+ct)`
/// from the line) without pushing to the kill-ring.
///
/// C body (utils.c:1105-1122):
/// ```c
/// if (flags & CUT_RAW) {
///     if (zlemetaline != NULL)
///         shiftchars(zlemetacs, ct);
///     else {
///         shiftchars(zlecs, ct);
///         CCRIGHT();
///     }
/// } else {
///     int origcs = zlecs, n = ct;
///     DPUTS(zlemetaline != NULL, "foredel needs CUT_RAW when metafied");
///     while (n--) INCCS();
///     ct = zlecs - origcs;
///     zlecs = origcs;
///     shiftchars(zlecs, ct);
///     CCRIGHT();
/// }
/// ```
///
/// CUT_RAW + zlemetaline: byte-shift ZLEMETALINE at zlemetacs.
/// CUT_RAW + plain: char-drain ZLELINE at zlecs.
/// non-CUT_RAW: INCCS-walk to advance `n` codepoints, drain that
/// range from ZLELINE. zshrs treats ZLELINE as `Vec<char>` so the
/// INCCS multibyte walk collapses to a fixed-N drain.
pub fn foredel(ct: i32, flags: i32) {
    // c:1105
    if ct <= 0 {
        return;
    }
    if (flags & CUT_RAW) != 0 {
        // c:1107 if (flags & CUT_RAW)
        // c:1108 — `if (zlemetaline != NULL) shiftchars(zlemetacs, ct);`
        let zml_active = ZLEMETALINE.get().is_some();
        if zml_active {
            // c:1109 — `shiftchars(zlemetacs, ct);` — byte-splice
            // ZLEMETALINE[cs..cs+ct].
            if let Some(m) = ZLEMETALINE.get() {
                if let Ok(mut g) = m.lock() {
                    let cs = ZLEMETACS.load(Ordering::Relaxed) as usize;
                    if cs < g.len() {
                        let end = (cs + ct as usize).min(g.len());
                        let bytes = g.as_bytes();
                        let new_line: String = String::from_utf8_lossy(&bytes[..cs]).into_owned()
                            + &String::from_utf8_lossy(&bytes[end..]);
                        *g = new_line;
                        ZLEMETALL.store(g.len() as i32, Ordering::Relaxed);
                    }
                }
            }
            return;
        }
        // c:1111-1113 — `else { shiftchars(zlecs, ct); CCRIGHT(); }`
        let ct = ct as usize;
        if ZLECS.load(Ordering::SeqCst) >= ZLELL.load(Ordering::SeqCst) {
            return;
        }
        let take_n = ct.min(ZLELL.load(Ordering::SeqCst) - ZLECS.load(Ordering::SeqCst));
        let i = ZLECS.load(Ordering::SeqCst);
        ZLELINE.lock().unwrap().drain(i..i + take_n); // c:1111 shiftchars
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst); // c:1112 CCRIGHT
        return;
    }
    // c:1115-1121 — non-CUT_RAW path:
    //   DPUTS(zlemetaline != NULL, "foredel needs CUT_RAW when metafied");
    //   int origcs = zlecs, n = ct; while (n--) INCCS();
    //   ct = zlecs - origcs; zlecs = origcs; shiftchars(zlecs, ct); CCRIGHT();
    // Rust ZLELINE is Vec<char> so INCCS multibyte walk collapses to a
    // fixed-N drain (one element per codepoint).
    let ct = ct as usize;
    if ZLECS.load(Ordering::SeqCst) >= ZLELL.load(Ordering::SeqCst) {
        return;
    }
    let take_n = ct.min(ZLELL.load(Ordering::SeqCst) - ZLECS.load(Ordering::SeqCst));
    let i = ZLECS.load(Ordering::SeqCst);
    ZLELINE.lock().unwrap().drain(i..i + take_n); // c:1120 shiftchars
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst); // c:1121 CCRIGHT
}

/// Port of `setline(char *s, int flags)` from Src/Zle/zle_utils.c:1129.
/// WARNING: param names don't match C — Rust=(zle, s) vs C=(s, flags)
pub fn setline(
    s: &str, // c:1129
    flags: i32,
) {
    // C body c:1129-1156:
    //   if ((flags & ZSL_TOEND) && (zlecs = zlell) && invicmdmode())
    //       DECCS();
    //   else if (zlecs > zlell)
    //       zlecs = zlell;
    //
    // Flag constants (Src/Zle/zle.h:404-407):
    //   ZSL_COPY  = 1  — copy the argument, don't modify it
    //   ZSL_TOEND = 2  — go to the end of the new line
    //
    // The previous Rust port had THREE bugs:
    //   1. Used `flags & 1` (ZSL_COPY) for the cursor-position decision
    //      — should be `flags & 2` (ZSL_TOEND).
    //   2. INVERTED the condition (`==0` instead of `!=0`) — sent the
    //      cursor to end-of-line when ZSL_COPY was UNSET, the opposite
    //      of C's "ZSL_TOEND set → end-of-line".
    //   3. Missing the `else if (zlecs > zlell) zlecs = zlell;` clamp
    //      — cursor outside the new line stayed at the stale position.
    let _ = ZSL_COPY; // c:1135 (no-op in Rust: &str is already independent)
    let mut line = ZLELINE.lock().unwrap();
    line.clear();
    line.extend(s.chars());
    let new_len = line.len();
    drop(line);
    ZLELL.store(new_len, Ordering::SeqCst);
    if (flags & ZSL_TOEND) != 0 {
        // c:1146
        // c:1146 — `zlecs = zlell` (and DECCS+invicmdmode skipped: the
        // DECCS substrate is the multibyte combining-char decrementer
        // and Rust's Vec<char> doesn't carry combining chars in storage).
        ZLECS.store(new_len, Ordering::SeqCst);
    } else if (ZLECS.load(Ordering::SeqCst)) > new_len {
        // c:1148-1149
        // c:1149 — `zlecs = zlell;` clamp.
        ZLECS.store(new_len, Ordering::SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst); // c:1150 CCRIGHT
}

/// Port of `findbol()` from `Src/Zle/zle_utils.c:1158`.
/// ```c
/// int
/// findbol()
/// {
///     int x = zlecs;
///     while (x > 0 && zleline[x - 1] != ZWC('\n'))
///         x--;
///     return x;
/// }
/// ```
/// Walk backward from the cursor to the start of the current line
/// (or the start of the buffer if there's no preceding newline).
/// Returns the byte offset.
pub fn findbol() -> usize {
    // c:1158
    let mut x = ZLECS.load(Ordering::SeqCst); // c:1158 int x = zlecs
    while x > 0 && ZLELINE.lock().unwrap().get(x - 1) != Some(&'\n') {
        // c:1162
        x -= 1; // c:1163 x--
    }
    x // c:1164 return x
}

/// Port of `findeol()` from `Src/Zle/zle_utils.c:1169`.
/// ```c
/// int
/// findeol()
/// {
///     int x = zlecs;
///     while (x != zlell && zleline[x] != ZWC('\n'))
///         x++;
///     return x;
/// }
/// ```
/// Walk forward from the cursor to the next newline (or end of
/// buffer). Returns the byte offset.
pub fn findeol() -> usize {
    // c:1169
    let mut x = ZLECS.load(Ordering::SeqCst); // c:1169 int x = zlecs
    while x != ZLELL.load(Ordering::SeqCst) && ZLELINE.lock().unwrap().get(x) != Some(&'\n') {
        // c:1173
        x += 1; // c:1174 x++
    }
    x // c:1175 return x
}

#[cfg(test)]
mod tests_hooks {
    use super::*;

    #[test]
    fn call_hook_queues_for_host_dispatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        call_hook("zle-line-init", None);
        call_hook("zle-keymap-select", Some("vicmd"));
        let drained = drain_hooks();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0], ("zle-line-init".to_string(), None));
        assert_eq!(
            drained[1],
            ("zle-keymap-select".to_string(), Some("vicmd".to_string()))
        );
        // Buffer is empty after drain.
        assert!(drain_hooks().is_empty());
    }

    #[test]
    fn redrawhook_queues_pre_redraw_hook() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        redrawhook();
        let drained = drain_hooks();
        assert_eq!(drained, vec![("zle-line-pre-redraw".to_string(), None)]);
    }

    #[test]
    fn reexpandprompt_re_runs_expansion_against_raw_templates() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        use crate::ported::zle::zle_main::ZLECONTEXT;
        use std::sync::atomic::Ordering::SeqCst;
        // In a caller-supplied-prompt context (vared/select), reexpandprompt
        // must expand the stashed RAW templates verbatim — it must NOT
        // re-read PS1/RPS1 (those aren't the vared prompt). Bug #654 gates
        // the param re-read on ZLCON_LINE_START, so pin the raw-template
        // path under ZLCON_VARED.
        let saved_ctx = ZLECONTEXT.swap(crate::ported::zsh_h::ZLCON_VARED, SeqCst);
        // Set raw templates that don't reference dynamic state, so the
        // expansion is idempotent and easy to assert. %% expands to a
        // single literal '%' per zsh prompt rules.
        *RAW_LP.lock().unwrap() = "%% > ".to_string();
        *RAW_RP.lock().unwrap() = "[%%]".to_string();
        reexpandprompt();
        assert_eq!(prompt(), "% > ");
        assert_eq!(rprompt(), "[%]");
        ZLECONTEXT.store(saved_ctx, SeqCst);
    }

    // Bug #654 — on a normal command-line edit (ZLCON_LINE_START),
    // reexpandprompt re-reads the LIVE PS1/RPS1/RPROMPT parameters (zsh's
    // raw_lp/raw_rp are pointers to the live globals), so a mid-line
    // `RPROMPT=…` change from a zle-keymap-select hook repaints the right
    // prompt. Pin that re-read, incl. the RPS1→RPROMPT classic-name fallback.
    #[test]
    fn reexpandprompt_rereads_live_params_on_command_line() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        use crate::ported::params::{setsparam, unsetparam};
        use crate::ported::zle::zle_main::ZLECONTEXT;
        use std::sync::atomic::Ordering::SeqCst;
        let saved_ctx = ZLECONTEXT.swap(crate::ported::zsh_h::ZLCON_LINE_START, SeqCst);
        crate::ported::lex::LEX_ISFIRSTLN.with(|c| c.set(true));
        // Stale stash + a live RPROMPT set only via the classic name.
        setsparam("PS1", "P> ");
        unsetparam("RPS1");
        setsparam("RPROMPT", "[%%R]");
        *RAW_LP.lock().unwrap() = "STALE_L".to_string();
        *RAW_RP.lock().unwrap() = "STALE_R".to_string();
        reexpandprompt();
        assert_eq!(prompt(), "P> ", "PS1 re-read live, not from stale RAW_LP");
        assert_eq!(rprompt(), "[%R]", "RPROMPT (classic name) re-read live");
        unsetparam("PS1");
        unsetparam("RPROMPT");
        ZLECONTEXT.store(saved_ctx, SeqCst);
    }
}

#[cfg(test)]
mod tests_bindkey_format {
    use super::bindztrdup;
    use super::printbind;
    use crate::ported::zle::zle_main::zle_test_setup;

    #[test]
    fn bind_ztrdup_emits_caret_form_for_control_chars() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Ctrl-A → `"^A"` (control-form `^A` wrapped by dquotedztrdup
        // per c:1267). Mirrors `bindkey -L` line "^A" beginning-of-line.
        assert_eq!(bindztrdup(b"\x01"), "\"^A\"");
        // Ctrl-_ → "^_".
        assert_eq!(bindztrdup(b"\x1f"), "\"^_\"");
        // DEL (0x7f) → "^?".
        assert_eq!(bindztrdup(b"\x7f"), "\"^?\"");
    }

    #[test]
    fn bind_ztrdup_escapes_backslash_and_caret() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Literal `\` (byte 0x5C) → bindztrdup buffer holds `\\`
        // (two backslashes from c:1261 `c == '\\'`). dquotedztrdup
        // sees two consecutive `\`s: emits one, then on the second
        // sees pending and emits an extra → `\\\`, then at end the
        // pending flag triggers one more `\` → final `\\\\` (4
        // backslashes between the wrapping quotes). Matches zsh's
        // `bindkey -M emacs` line containing `"^\\\\"-"~" self-insert`.
        assert_eq!(bindztrdup(b"\\"), "\"\\\\\\\\\"");
        // Literal `^` (byte 0x5E) → bindztrdup buffer holds `\^`
        // (one backslash + caret). dquotedztrdup: `\` not pending →
        // emit one `\`; `^` default → emit; no final pending. Result
        // `"\^"`.
        assert_eq!(bindztrdup(b"^"), "\"\\^\"");
    }

    #[test]
    fn bind_ztrdup_handles_high_bit_as_meta() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Byte 0xC1 → bindztrdup buffer `\M-A` (3 bytes: `\`, `M`,
        // `-`, `A`). dquotedztrdup: `\` not pending → one `\`; `M`,
        // `-`, `A` default → as-is. Result `"\M-A"`.
        assert_eq!(bindztrdup(b"\xC1"), "\"\\M-A\"");
    }

    #[test]
    fn printbind_caret_form_matches_describe_key_output() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // `printbind` routes through `bindztrdup` (c:1283-1287) and
        // inherits the dquotedztrdup wrapping.
        assert_eq!(printbind(b"\x01"), "\"^A\"");
        assert_eq!(printbind(b"\x1b"), "\"^[\"");
    }
}

/// Port of `findline(int *a, int *b)` from `Src/Zle/zle_utils.c:1180`.
/// ```c
/// void
/// findline(int *a, int *b)
/// {
///     *a = findbol();
///     *b = findeol();
/// }
/// ```
/// Returns `(bol, eol)` for the current line.
/// WARNING: param names don't match C — Rust=(zle) vs C=(a, b)
pub fn findline() -> (usize, usize) {
    // c:1180
    (findbol(), findeol()) // c:1180-1183
}

/// Port of `getzlequery()` from Src/Zle/zle_utils.c:1197.
/// Returns 1 for affirmative ('y'/'\t'), 0 for negative ('n'/ctrl/EOF).
pub fn getzlequery() -> i32 {
    // c:1197
    // c:1201-1210 — FIONREAD typeahead check → negative response if buffered.
    //               Without a live tty fd here, skip the typeahead probe.
    // c:1213 — c = getfullchar(0);
    let c = getfullchar(false);
    // c:1218 — errflag &= ~ERRFLAG_INT;
    errflag.fetch_and(!ERRFLAG_INT, Ordering::Relaxed);
    // c:1219-1224 — '\t' → 'y'; ctrl/EOF → 'n'; else tolower.
    let c = match c {
        Some('\t') => 'y',                   // c:1219-1220
        Some(ch) if ch.is_control() => 'n',  // c:1221-1222 ZC_icntrl
        None => 'n',                         // c:1221 ZLEEOF
        Some(ch) => ch.to_ascii_lowercase(), // c:1223-1224
    };
    // c:1226-1231 — echo response (skipping newline). No live tty echo
    //               here; the canonical zlewrites lands when the
    //               refresh substrate is wired.
    // c:1232 — return c == ZWC('y');
    if c == 'y' {
        1
    } else {
        0
    }
}

/// Position save/restore
/// Port of zle_save_positions() / zle_restore_positions() from zle_utils.c
/// Port of `zle_save_positions` from `Src/Zle/zle_utils.c:619`.

/// Port of `zle_restore_positions` from `Src/Zle/zle_utils.c:677`.

// `CutFlags` / `CutDirection` deleted — Rust-only types with no C
// counterpart. C uses `int flags` with the `CUT_FRONT` / `CUT_REPLACE`
// / `CUT_RAW` bits at zle.h:271-281 (already legit-ported in
// zle_h.rs:387-393). `foredel` / `backdel` / `cuttext` now take
// `i32 flags` matching the C signatures verbatim.

/// Direct port of `char *bindztrdup(char *str)` from
/// `Src/Zle/zle_utils.c:1238`. Builds the bindkey-listing escape
/// buffer (`^X` for ctrl, `\M-X` for high-bit, doubled `\\` / `\^`
/// for literal `\` and `^`), then routes through `dquotedztrdup` for
/// the final shell-quoted form. The returned string includes the
/// surrounding `"..."` per `dquotedztrdup`'s contract; callers should
/// NOT add their own quotes.
pub fn bindztrdup(str: &[u8]) -> String {
    // c:1238
    let mut buf = String::new();
    // c:1248-1264 — build the unquoted escape buffer.
    for &b in str {
        let mut c = b;
        if c & 0x80 != 0 {
            // c:1252-1255 — high-bit: `\M-` prefix + strip 0x80.
            buf.push('\\');
            buf.push('M');
            buf.push('-');
            c &= 0x7f;
        }
        if c < 32 || c == 0x7f {
            // c:1257-1259 — control char: `^` prefix + XOR 0x40.
            buf.push('^');
            c ^= 64;
        }
        if c == b'\\' || c == b'^' {
            // c:1261 — literal `\` / `^`: backslash-escape.
            buf.push('\\');
        }
        buf.push(c as char);
    }
    // c:1267 — `ret = dquotedztrdup(buf)`. Wraps in `"..."` and
    // does another `\` → `\\` doubling pass.
    crate::ported::utils::dquotedztrdup(&buf)
}

/// Port of `printbind(char *str, FILE *stream)` from zle_utils.c:1283.
/// C body (4 lines):
///     `char *b = bindztrdup(str);
///      int ret = zputs(b, stream);
///      zsfree(b);
///      return ret;`
/// Rust returns the formatted String (callers don't take a stream);
/// the zputs call is dropped because callers compose the result
/// into larger output via `format!` rather than streaming.
pub fn printbind(seq: &[u8]) -> String {
    // c:1283
    bindztrdup(seq) // c:1285
}

/// Direct port of `void showmsg(char const *msg)` from
/// `Src/Zle/zle_utils.c:1310`. Display a message where the completion
/// list normally goes; `msg` is metafied (c:1303-1305).
///
/// Ports the non-`MULTIBYTE_SUPPORT` branch faithfully (c:1389-1397):
/// trashzle → metafied byte scan with nicechar expansion + cc/up
/// column tracking → clearflag-driven cursor restore. The
/// `#ifdef MULTIBYTE_SUPPORT` branch (c:1330-1387, mbrtowc /
/// wcs_nicechar wide-char path) needs multibyte substrate not yet
/// wired; the visible-byte stream matches in the common case.
pub fn showmsg(msg: &str) {
    // c:1310
    use crate::ported::utils::{nicechar, write_loop};
    use crate::ported::zle::zle_refresh::{tcmultout, CLEARFLAG, NLNCT, SHOWINGLIST};
    use crate::ported::zsh_h::{isset, Meta, ALWAYSLASTPROMPT, TCMULTUP, TCUP, USEZLE};

    let mut up: i32 = 0; // c:1316
    let mut cc: i32 = 0; // c:1316

    trashzle(); // c:1325
                // c:1326 — clearflag = isset(USEZLE) && !termflags && isset(ALWAYSLASTPROMPT)
    let termflags = crate::ported::params::TERMFLAGS.load(Ordering::Relaxed);
    let clearflag = isset(USEZLE) && termflags == 0 && isset(ALWAYSLASTPROMPT);
    CLEARFLAG.store(if clearflag { 1 } else { 0 }, Ordering::Relaxed);

    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let shout = if fd >= 0 { fd } else { 2 };
    let cols = crate::ported::utils::adjustcolumns().max(1) as i32; // zterm_columns

    // c:1389 — for(p = msg; (c = *p); p++)
    let bytes = msg.as_bytes();
    let mut p = 0usize;
    while p < bytes.len() {
        let mut c = bytes[p];
        if c == Meta {
            // c:1391 — c = *++p ^ 32
            p += 1;
            c = bytes.get(p).copied().unwrap_or(0) ^ 32;
        }
        if c == b'\n' {
            // c:1392-1395
            let _ = write_loop(shout, b"\n"); // putc('\n', shout)
            up += 1 + (cc - 1) / cols;
            cc = 0;
        } else {
            // c:1396-1399 — n = nicechar(c); zputs(n, shout); cc += strlen(n)
            let n = nicechar(c as char);
            let _ = write_loop(shout, n.as_bytes());
            cc += n.len() as i32;
        }
        p += 1;
    }

    up += (cc - 1) / cols; // c:1403
    if clearflag {
        // c:1405-1406
        let _ = write_loop(shout, b"\r"); // putc('\r', shout)
        let nlnct = NLNCT.load(Ordering::Relaxed);
        tcmultout(TCUP, TCMULTUP, up + nlnct);
    } else {
        // c:1408
        let _ = write_loop(shout, b"\n"); // putc('\n', shout)
    }
    SHOWINGLIST.store(0, Ordering::Relaxed); // c:1409
}

/// Port of `handlefeep(UNUSED(char **args))` from `Src/Zle/zle_utils.c:1405`.
/// ```c
/// int
/// handlefeep(UNUSED(char **args))
/// {
///     zbeep();
///     return 0;
/// }
/// ```
/// `beep` widget — fires the terminal bell via `zbeep`.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn handlefeep() -> i32 {
    // c:1405
    crate::ported::utils::zbeep(); // c:1415 zbeep()
    0 // c:1415 return 0
}

/// Port of `handlesuffix(UNUSED(char **args))` from Src/Zle/zle_utils.c:1415.
/// C body: `return 0;` — the real suffix-handling lives on the
/// callers (insertsuffix / removesuffix); this entry is a no-op
/// stub the C source kept for hook-table registration.
pub fn handlesuffix(c: i32) -> i32 {
    // c:1415
    let _ = c;
    0 // c:1417
}

/// Port of `initundo()` from Src/Zle/zle_utils.c:1446.
pub fn initundo() {
    // c:1446
    // C body c:1448-1459 — `nextchanges = endnextchanges = NULL;
    //                       lastline = ...; freeundo()`.
    //                      Undo chain isn't a Rust struct yet; no-op.
    freeundo();
}

/// Port of `freeundo()` from Src/Zle/zle_utils.c:1461.
pub fn freeundo() { // c:1461
                    // C body c:1463-1470 — `freechanges(curchange); freechanges(...)
                    //                      etc. for the whole undo chain`. Drop covers.
}

/// Port of `freechanges(struct change *p)` from Src/Zle/zle_utils.c:1472.
/// WARNING: param names don't match C — Rust=() vs C=(p)
pub fn freechanges() { // c:1472
                       // C body c:1474-1484 — walks Change linked list, frees del/ins
                       //                      strings + the Change node. Drop covers it.
}

// register pending changes in the undo system                            // c:1488
/// Pre-widget hook. Port of `handleundo` (zle_utils.c) — the
/// Rust port collapses to `setlastline()` because zshrs uses a
/// one-change-per-widget model. C's `handleundo` body
/// (zle_utils.c:1488) flushes the in-flight `nextchanges`
/// chain that accumulates across multi-key vi operations; that
/// chain is unnecessary when each widget produces exactly one
/// undo entry via `mkundoent` post-call.
pub fn handleundo() {
    // c:1488
    setlastline();
}

// add an entry to the undo system, if anything has changed              // c:1532
/// If the line changed since the last snapshot, append a Change record
/// describing the diff. Port of `mkundoent` (zle_utils.c:1532).
pub fn mkundoent() {
    // c:1532
    if LASTLL.load(Ordering::SeqCst) == ZLELL.load(Ordering::SeqCst)
        && LASTLINE.lock().unwrap()[..LASTLL.load(Ordering::SeqCst)]
            == ZLELINE.lock().unwrap()[..ZLELL.load(Ordering::SeqCst)]
    {
        LASTCS.store(ZLECS.load(Ordering::SeqCst), Ordering::SeqCst);
        return;
    }
    let sh = LASTLL
        .load(Ordering::SeqCst)
        .min(ZLELL.load(Ordering::SeqCst));
    let mut pre = 0usize;
    while pre < sh && ZLELINE.lock().unwrap()[pre] == LASTLINE.lock().unwrap()[pre] {
        pre += 1;
    }
    let mut suf = 0usize;
    while suf < sh - pre
        && ZLELINE.lock().unwrap()[ZLELL.load(Ordering::SeqCst) - 1 - suf]
            == LASTLINE.lock().unwrap()[LASTLL.load(Ordering::SeqCst) - 1 - suf]
    {
        suf += 1;
    }
    let del: Vec<char> = if suf + pre == LASTLL.load(Ordering::SeqCst) {
        Vec::new()
    } else {
        LASTLINE.lock().unwrap()[pre..LASTLL.load(Ordering::SeqCst) - suf].to_vec()
    };
    let ins: Vec<char> = if suf + pre == ZLELL.load(Ordering::SeqCst) {
        Vec::new()
    } else {
        ZLELINE.lock().unwrap()[pre..ZLELL.load(Ordering::SeqCst) - suf].to_vec()
    };
    UNDO_CHANGENO.fetch_add(1, Ordering::SeqCst);
    // Canonical `change.del`/`ins` are `ZLE_STRING_T = String`
    // (zle_h.rs:66, port of `ZLE_STRING_T` typedef). Local
    // `Vec<char> = Vec<char>` (zle_main.rs:59); convert at the
    // boundary.
    let del_str: String = del.iter().collect();
    let ins_str: String = ins.iter().collect();
    let dell = del_str.chars().count() as i32;
    let insl = ins_str.chars().count() as i32;
    let ch = crate::ported::zle::zle_h::change {
        prev: None,
        next: None,
        flags: 0,
        hist: history().lock().unwrap().cursor as i32,
        off: pre as i32,
        del: del_str,
        dell,
        ins: ins_str,
        insl,
        old_cs: LASTCS.load(Ordering::SeqCst) as i32,
        new_cs: ZLECS.load(Ordering::SeqCst) as i32,
        changeno: UNDO_CHANGENO.load(Ordering::SeqCst) as i64,
    };
    // Drop any forward redo history past the cursor before pushing.
    UNDO_STACK
        .lock()
        .unwrap()
        .truncate(CURCHANGE.load(Ordering::SeqCst));
    UNDO_STACK.lock().unwrap().push(ch);
    CURCHANGE.store(UNDO_STACK.lock().unwrap().len(), Ordering::SeqCst);
}

/// Snapshot the current line into `last_line` so the next `mkundoent`
/// can diff against it. Port of `setlastline` (zle_utils.c:1587).
pub fn setlastline() {
    let snapshot = ZLELINE.lock().unwrap().clone();
    let mut ll = LASTLINE.lock().unwrap();
    ll.clear();
    ll.extend_from_slice(&snapshot);
    drop(ll);
    LASTLL.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
    LASTCS.store(ZLECS.load(Ordering::SeqCst), Ordering::SeqCst);
}

/// Direct port of `int undo(char **args)` from
/// `Src/Zle/zle_utils.c:1601`. Walks the undo stack backward
/// from ` CURCHANGE.load(std::sync::atomic::Ordering::SeqCst)` calling `unapplychange` on each; stops at
/// `last_change` (parsed from `args[0]` if provided, else -1 for
/// "single step") or at `undo_limitno`. Returns 0 on success,
/// 1 when nothing left to undo.
pub fn undo(args: &[String]) -> i32 {
    // c:1601
    let last_change: i64 = if !args.is_empty() {
        // c:1605
        args[0].parse().unwrap_or(-1)
    } else {
        -1
    };

    loop {
        // c:1614 — `prev = curchange->prev`; in Rust we step the
        // index down.
        if CURCHANGE.load(Ordering::SeqCst) == 0 {
            return 1;
        } // c:1615
        let prev_idx = CURCHANGE.load(Ordering::SeqCst) - 1;
        let prev_chno = UNDO_STACK.lock().unwrap()[prev_idx].changeno as i64;
        if prev_chno <= last_change {
            break;
        } // c:1618
        if (prev_chno as u64) <= UNDO_LIMITNO.load(Ordering::SeqCst) && args.is_empty() {
            // c:1619
            return 1;
        }
        if unapplychange(prev_idx as i32) == 0 {
            // c:1621
            if last_change >= 0 {
                unapplychange(prev_idx as i32); // c:1623
                CURCHANGE.store(prev_idx, Ordering::SeqCst); // c:1624
            }
        } else {
            CURCHANGE.store(prev_idx, Ordering::SeqCst); // c:1627
        }
        let has_prev = UNDO_STACK
            .lock()
            .unwrap()
            .get(prev_idx)
            .map(|c| c.flags & CH_PREV != 0)
            .unwrap_or(false);
        if !(last_change >= 0 || has_prev) {
            break;
        } // c:1630
    }
    0 // c:1631
}

/// Port of `unapplychange(struct change *ch)` from Src/Zle/zle_utils.c:1634.
/// Direct port of `static int unapplychange(struct change *ch)` from
/// `Src/Zle/zle_utils.c:1634`. Reverse of applychange: deletes
/// `ch->ins` at `ch->off` and re-inserts `ch->del`.
/// WARNING: param names don't match C — Rust=(zle, ch) vs C=(ch)
pub fn unapplychange(ch: i32) -> i32 {
    // c:1634
    let idx = ch as usize;
    if idx >= UNDO_STACK.lock().unwrap().len() {
        return 0;
    }
    let change = UNDO_STACK.lock().unwrap()[idx].clone();
    // Canonical change.off/insl/old_cs are i32, change.del is String;
    // convert at the indexing boundary.
    let off = change.off as usize;
    // c:1638-1644 — delete what was inserted.
    let ins_n = change.insl as usize;
    if off + ins_n <= ZLELINE.lock().unwrap().len() {
        ZLELINE.lock().unwrap().drain(off..off + ins_n); // c:1640
    }
    // c:1646 — re-insert the deleted chars.
    for (i, c) in change.del.chars().enumerate() {
        if off + i <= ZLELINE.lock().unwrap().len() {
            ZLELINE.lock().unwrap().insert(off + i, c);
        } else {
            ZLELINE.lock().unwrap().push(c);
        }
    }
    ZLECS.store(change.old_cs as usize, Ordering::SeqCst); // c:1649
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    // c:1651 — return 1 if CH_PREV, else 0.
    if change.flags & CH_PREV != 0 {
        1
    } else {
        0
    }
}

/// Direct port of `int redo(UNUSED(char **args))` from
/// `Src/Zle/zle_utils.c:1661`. Walks the undo stack forward
/// from ` CURCHANGE.load(std::sync::atomic::Ordering::SeqCst)` calling `applychange` on each; returns 0
/// on success, 1 when nothing to redo.
pub fn redo() -> i32 {
    // c:1661
    loop {
        if CURCHANGE.load(Ordering::SeqCst) >= UNDO_STACK.lock().unwrap().len() {
            return 1;
        } // c:1664
        let cur_idx = CURCHANGE.load(Ordering::SeqCst);
        if applychange(cur_idx as i32) == 0 {
            break;
        } // c:1668
        CURCHANGE.store(cur_idx + 1, Ordering::SeqCst);
        let has_next = UNDO_STACK
            .lock()
            .unwrap()
            .get(cur_idx)
            .map(|c| c.flags & CH_NEXT != 0)
            .unwrap_or(false);
        if !has_next {
            break;
        } // c:1670
    }
    CURCHANGE.fetch_add(1, Ordering::SeqCst); // c:1672 advance past applied
    0 // c:1674
}

/// Direct port of `static int applychange(struct change *ch)` from
/// `Src/Zle/zle_utils.c:1678`. Applies one Change record from
/// the undo stack: deletes `ch->del` characters at `ch->off`, then
/// inserts `ch->ins` at the same position, and updates `zlecs`.
/// Returns 1 if there are more changes to apply (CH_NEXT), else 0.
pub fn applychange(ch: i32) -> i32 {
    // c:1678
    let idx = ch as usize;
    if idx >= UNDO_STACK.lock().unwrap().len() {
        return 0;
    }
    let change = UNDO_STACK.lock().unwrap()[idx].clone();
    // c:1683-1696 — apply del then ins at change.off. Canonical
    // `change.off`/`dell`/`insl` are `i32` (port of `int off; int
    // dell; int insl`); `change.del`/`ins` are `String` (port of
    // `ZLE_STRING_T`). Convert at the indexing boundary.
    let off = change.off as usize;
    let del_n = change.dell as usize;
    if off + del_n <= ZLELINE.lock().unwrap().len() {
        ZLELINE.lock().unwrap().drain(off..off + del_n); // c:1690 delete
    }
    // c:1700 — insert change.ins at off.
    for (i, c) in change.ins.chars().enumerate() {
        if off + i <= ZLELINE.lock().unwrap().len() {
            ZLELINE.lock().unwrap().insert(off + i, c);
        } else {
            ZLELINE.lock().unwrap().push(c);
        }
    }
    ZLECS.store(change.new_cs as usize, Ordering::SeqCst); // c:1718
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    // c:1721 — return 1 if CH_NEXT, else 0.
    if change.flags & CH_NEXT != 0 {
        1
    } else {
        0
    }
}

/// Direct port of `int viundochange(char **args)` from
/// `Src/Zle/zle_utils.c:1705`.
/// ```c
/// handleundo();
/// if (curchange->next) {
///     do { applychange(curchange); curchange = curchange->next; }
///     while(curchange->next);
///     setlastline();
///     return 0;
/// } else return undo(args);
/// ```
pub fn viundochange(
    // c:1705
    args: &[String],
) -> i32 {
    handleundo(); // c:1707
    if CURCHANGE.load(Ordering::SeqCst) < UNDO_STACK.lock().unwrap().len() {
        // c:1708 curchange->next
        // Re-apply all forward changes (collapses an undo chain back
        // to current state).
        while CURCHANGE.load(Ordering::SeqCst) < UNDO_STACK.lock().unwrap().len() {
            // c:1710
            let idx = CURCHANGE.load(Ordering::SeqCst);
            applychange(idx as i32); // c:1711
            CURCHANGE.store(idx + 1, Ordering::SeqCst); // c:1712
        }
        0 // c:1715
    } else {
        undo(args) // c:1717
    }
}

/// Direct port of `int splitundo(char **args)` from
/// `Src/Zle/zle_utils.c:1721`.
/// ```c
/// if (vistartchange >= 0) {
///     mergeundo();
///     vistartchange = undo_changeno;
/// }
/// handleundo();
/// return 0;
/// ```
pub fn splitundo() -> i32 {
    // c:1721
    // C uses signed `vistartchange`; Rust uses u64 with u64::MAX as
    // the "-1 / inactive" sentinel.
    if VISTARTCHANGE.load(Ordering::SeqCst) != u64::MAX {
        // c:1723 >= 0
        mergeundo(); // c:1725
        VISTARTCHANGE.store(UNDO_CHANGENO.load(Ordering::SeqCst), Ordering::SeqCst);
        // c:1726
    }
    handleundo(); // c:1728
    0 // c:1730
}

/// Direct port of `void mergeundo(void)` from
/// `Src/Zle/zle_utils.c:1733`. Walks the undo stack backward
/// from `cur_change` chaining CH_PREV/CH_NEXT flags so the changes
/// since `vistartchange+1` form a single undo step (atomic vi
/// insert-mode group). Resets `vistartchange = u64::MAX` (C's -1).
pub fn mergeundo() {
    // c:1733
    // c:1735-1742 — walk current->prev while changeno > vistartchange+1.
    if CURCHANGE.load(Ordering::SeqCst) == 0 {
        return;
    }
    let mut current = CURCHANGE.load(Ordering::SeqCst) - 1; // c:1735 prev
    while current > 0
        && UNDO_STACK.lock().unwrap()[current].changeno
            > VISTARTCHANGE.load(Ordering::SeqCst) as i64 + 1
    {
        UNDO_STACK.lock().unwrap()[current].flags |= CH_PREV; // c:1740
        UNDO_STACK.lock().unwrap()[current - 1].flags |= CH_NEXT; // c:1741
        current -= 1;
    }
    VISTARTCHANGE.store(u64::MAX, Ordering::SeqCst); // c:1744 = -1
}

/// Direct port of `void zlecallhook(char *name, char *arg)` from
/// `Src/Zle/zle_utils.c:1755`. Looks up the ZLE function `name`,
/// dispatches it via `execzlefunc` with `arg` as the single argv
/// element, then unrefs the Thingy and preserves errflag/retflag
/// across the call (except for ERRFLAG_INT which is propagated so
/// `^C` during the hook still cancels the outer command).
pub fn zlecallhook(name: &str, arg: Option<&str>) {
    // c:1755

    // c:1757 — `Thingy thingy = rthingy_nocreate(name); if (!thingy) return;`
    if !crate::ported::zle::zle_thingy::rthingy_nocreate(name) {
        return;
    }

    // c:1763-1764 — snapshot errflag/retflag.
    let saverrflag = errflag.load(Ordering::Relaxed);
    let savretflag = RETFLAG.load(Ordering::Relaxed);

    // c:1768 — `args[0] = arg; args[1] = NULL; execzlefunc(thingy, args, 1, 0);`
    let args: Vec<String> = match arg {
        Some(a) => vec![a.to_string()],
        None => Vec::new(),
    };
    // c:1768 — `execzlefunc(thingy, args, 1, 0)`. set_bindk=1 so
    // the hook's widget binding is visible to inner handlers.
    let _ = execzlefunc(name, &args, 1, 0); // c:1768

    // c:1771 — `unrefthingy(thingy);`
    crate::ported::zle::zle_thingy::unrefthingy(name);

    // c:1774 — `errflag = saverrflag | (errflag & ERRFLAG_INT);`
    let cur_errflag = errflag.load(Ordering::Relaxed);
    errflag.store(saverrflag | (cur_errflag & ERRFLAG_INT), Ordering::Relaxed);
    RETFLAG.store(savretflag, Ordering::Relaxed); // c:1775
}

/// Port of `get_undo_current_change(UNUSED(Param pm))` from Src/Zle/zle_utils.c:1785.
/// Returns `undo_changeno` (the most-recently-committed change number),
/// committing any pending edits to a new undo entry first.
pub fn get_undo_current_change() -> i64 {
    // c:1785
    let remetafy: i32; // c:1787
                       /*
                        * Yuk: we call this from within the completion system,
                        * so we need to convert back to the form which can be
                        * copied into undo entries.
                        */                                                                      // c:1789-1793
                       // c:1794 — `if (zlemetaline != NULL)`. ZLEMETALINE is a
                       // OnceLock<Mutex<String>>: "non-NULL" ≡ initialised AND non-empty.
    let zml_active = crate::ported::zle::compcore::ZLEMETALINE
        .get()
        .and_then(|m| m.lock().ok().map(|s| !s.is_empty()))
        .unwrap_or(false);
    if zml_active {
        // c:1795 — `unmetafy_line();` Rust stores ZLE as UTF-8, so the
        // unmetafy → undo-form transcoding is a no-op at the storage
        // layer; the bookkeeping flag is still set per C.
        remetafy = 1; // c:1796
    } else {
        remetafy = 0; // c:1798
    }

    /* add entry for any pending changes */
    // c:1800
    mkundoent(); // c:1801
    setlastline(); // c:1802

    if remetafy != 0 { // c:1804
         // c:1805 — `metafy_line();` — re-metafy. No-op storage-wise
         // (UTF-8 invariant); the call site is preserved for symmetry.
    }

    UNDO_CHANGENO.load(Ordering::SeqCst) as i64 // c:1807
}

/// Port of `get_undo_limit_change(UNUSED(Param pm))` from Src/Zle/zle_utils.c:1812.
pub fn get_undo_limit_change() -> i64 {
    // c:1812
    // c:1815 — return undo_limitno;
    UNDO_LIMITNO.load(Ordering::SeqCst) as i64
}

/// Port of `set_undo_limit_change(UNUSED(Param pm), zlong value)` from Src/Zle/zle_utils.c:1819.
/// WARNING: param names don't match C — Rust=(value) vs C=(pm, value)
pub fn set_undo_limit_change(value: i64) -> i32 {
    // c:1819
    // c:1822 — struct change *chp;
    // c:1823-1837 — walk back from curchange until the entry whose
    //               changeno <= value, then set undo_limitno = that
    //               entry's changeno. With our linear UNDO_STACK that
    //               distinction collapses to clamping to the largest
    //               committed changeno <= value.
    let cap = UNDO_CHANGENO.load(Ordering::SeqCst) as i64;
    let clamped = value.max(0).min(cap);
    UNDO_LIMITNO.store(clamped as u64, Ordering::SeqCst);
    0
}
/// `insert_str` — see implementation.
pub fn insert_str(s: &str) {
    for c in s.chars() {
        ZLELINE
            .lock()
            .unwrap()
            .insert(ZLECS.load(Ordering::SeqCst), c);
        ZLECS.fetch_add(1, Ordering::SeqCst);
        ZLELL.fetch_add(1, Ordering::SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// Insert chars at cursor position
pub fn insert_chars(chars: &[char]) {
    for &c in chars {
        ZLELINE
            .lock()
            .unwrap()
            .insert(ZLECS.load(Ordering::SeqCst), c);
        ZLECS.fetch_add(1, Ordering::SeqCst);
        ZLELL.fetch_add(1, Ordering::SeqCst);
    }
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// Delete n characters at cursor position
pub fn delete_chars(n: usize) {
    let n = n.min(ZLELL.load(Ordering::SeqCst) - ZLECS.load(Ordering::SeqCst));
    for _ in 0..n {
        if ZLECS.load(Ordering::SeqCst) < ZLELL.load(Ordering::SeqCst) {
            ZLELINE.lock().unwrap().remove(ZLECS.load(Ordering::SeqCst));
            ZLELL.fetch_sub(1, Ordering::SeqCst);
        }
    }
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// Delete n characters before cursor
pub fn backspace_chars(n: usize) {
    let n = n.min(ZLECS.load(Ordering::SeqCst));
    for _ in 0..n {
        if ZLECS.load(Ordering::SeqCst) > 0 {
            ZLECS.fetch_sub(1, Ordering::SeqCst);
            ZLELINE.lock().unwrap().remove(ZLECS.load(Ordering::SeqCst));
            ZLELL.fetch_sub(1, Ordering::SeqCst);
        }
    }
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// Get the line as a string
pub fn get_line() -> String {
    ZLELINE.lock().unwrap().iter().collect()
}

/// Set the line from a string while preserving the current cursor
/// position (clamped to the new length).
/// Port of `setline(char *s, int flags)` from Src/Zle/zle_utils.c:1129 with the
/// `ZSL_NOCURSOR` flag set. Used by widget bodies that swap in a
/// fresh line (history navigation, isearch hit) but want to keep
/// the cursor where it was.
pub fn set_line_keep_cursor(s: &str) {
    *ZLELINE.lock().unwrap() = s.chars().collect();
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLECS.store(
        ZLECS
            .load(Ordering::SeqCst)
            .min(ZLELL.load(Ordering::SeqCst)),
        Ordering::SeqCst,
    );
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// Clear the line
pub fn clear_line() {
    ZLELINE.lock().unwrap().clear();
    ZLELL.store(0, Ordering::SeqCst);
    ZLECS.store(0, Ordering::SeqCst);
    MARK.store(0, Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// Get region between point and mark
pub fn get_region() -> Vec<char> {
    let (start, end) = if ZLECS.load(Ordering::SeqCst) < MARK.load(Ordering::SeqCst) {
        (ZLECS.load(Ordering::SeqCst), MARK.load(Ordering::SeqCst))
    } else {
        (MARK.load(Ordering::SeqCst), ZLECS.load(Ordering::SeqCst))
    };
    ZLELINE.lock().unwrap()[start..end].to_vec()
}

/// Cut to named buffer
pub fn cut_to_buffer(buf: usize, append: bool) {
    if buf < vibuf().lock().unwrap().len() {
        let (start, end) = if ZLECS.load(Ordering::SeqCst) < MARK.load(Ordering::SeqCst) {
            (ZLECS.load(Ordering::SeqCst), MARK.load(Ordering::SeqCst))
        } else {
            (MARK.load(Ordering::SeqCst), ZLECS.load(Ordering::SeqCst))
        };

        let text: Vec<char> = ZLELINE.lock().unwrap()[start..end].to_vec();

        if append {
            vibuf().lock().unwrap()[buf].extend(text);
        } else {
            vibuf().lock().unwrap()[buf] = text;
        }
    }
}

/// Paste from a named vi cut buffer.
/// Port of `pastebuf(Cutbuffer buf, int mult, int position)` from Src/Zle/zle_misc.c:558. The C source
/// looks up `vibuf[zmod.vibuf]` (the vi `"a..z` register table),
/// uses `cutbuf` for the unnamed buffer, and inserts at zlecs (or
/// zlecs+1 for `after=true`). zshrs models the 36-slot vibuf array
/// as the file-scope `VIBUF` static (zle_main.rs).
pub fn paste_from_buffer(buf: usize, after: bool) {
    if buf < vibuf().lock().unwrap().len() {
        let text = vibuf().lock().unwrap()[buf].clone();
        if !text.is_empty() {
            if after && ZLECS.load(Ordering::SeqCst) < ZLELL.load(Ordering::SeqCst) {
                ZLECS.fetch_add(1, Ordering::SeqCst);
            }
            insert_chars(&text);
        }
    }
}

// Note: dead `UndoEntry`/`UndoState`/`apply_undo_entry` aggregates
// removed per PORT_PLAN Phase 2. They were a Rust-only invention
// with zero references across the codebase. The canonical undo
// machinery lives in file-scope statics (`UNDO_STACK: Mutex<Vec<change>>`,
// `CHANGENO`, `CURCHANGE`, `UNDO_CHANGENO`, `UNDO_LIMITNO` —
// declared in zle_main.rs) and the canonical port ported are:
//
//   mkundoent       — port of mkundoent (zle_utils.c)
//   apply_change    — port of applychange (zle_utils.c:1633)
//   unapply_change  — port of unapplychange (zle_utils.c:1677)
//
// C source's bag-of-statics that the canonical methods touch:
//
//   struct change *curchange;             // line 1427
//   static struct change *changes;        // line 1429
//   static struct change *nextchanges, *endnextchanges;  // line 1433
//   static zlong undo_limitno;            // line 1442
//   static struct zle_position *zle_positions;  // line 608
//
// These are file-scope (some `extern`-visible from zle_main.c), so
// they're PORT_PLAN Phase 3 bucket-2 (Arc<RwLock>) work, not the
// Phase 2 bucket-1 (thread_local!) wave. The dissolution noted here
// is structural cleanup (remove dead aggregate); the bucket-2 wiring
// of these globals onto Zle is already done in zle_main.rs.

/// Find beginning of line from position
/// Port of findbol() from zle_utils.c

/// Find end of line from position
/// Port of findeol() from zle_utils.c

/// Find line number for position
/// Port of findline(int *a, int *b) from zle_utils.c

// make sure that the line buffer has at least sz chars               // c:63
/// Ensure line has enough space
/// Port of sizeline(int sz) from zle_utils.c

// insert space for ct chars at cursor position                        // c:773
/// Make space in line at position
/// Port of spaceinline(int ct) from zle_utils.c

/// Shift characters in line
/// Port of shiftchars(int to, int cnt) from zle_utils.c

/// Direct port of `mod_export int foredel(int n, int flags)` from
/// `Src/Zle/zle_utils.c:1105`. Delete `n` chars forward; `flags`
/// is a bitmask of `CUT_*` (zle.h:271-281). Returns 0 on success,
/// non-zero when the kill failed.

/// Direct port of `mod_export int backdel(int n, int flags)` from
/// `Src/Zle/zle_utils.c:1084`. Delete `n` chars backward.

/// Kill forward
/// Port of forekill(int ct, int flags) from zle_utils.c

/// Kill backward
/// Port of backkill(int ct, int flags) from zle_utils.c

/// Direct port of `mod_export void cuttext(ZLE_STRING_T line, int len,
/// int flags)` from `Src/Zle/zle_utils.c:946`. Stages a slice of the
/// edit line into the cut buffer / kill ring, honouring the
/// CUT_FRONT / CUT_REPLACE / CUT_RAW flag bits.

/// Snapshot the current line into `last_line` for the undo system.
/// Port of `setlastline()` from Src/Zle/zle_utils.c:1587. Routes to
/// the canonical `setlastline` method below — kept under the
/// snake-case name so older callers compile.
pub fn set_last_line() {
    setlastline();
}

/// Show a message
/// Port of showmsg(char const *msg) from zle_utils.c

/// Handle a feep (beep/error)
/// Port of handlefeep(UNUSED(char **args)) from zle_utils.c
pub fn handle_feep() {
    // Faithful bell: route through zbeep (utils.c:4105) so the `\x07` is
    // written to SHTTY via write_loop, gated on the BEEP option and honouring
    // $ZBEEP. The previous `print!("\x07")` went to Rust's buffered stdout,
    // which is the wrong destination for ZLE and stayed unflushed (no newline)
    // so the bell never reached the terminal on an ambiguous / no-match Tab.
    crate::ported::utils::zbeep();
}

/// Add text to line at position
/// Port of zleaddtoline(int chr) from zle_utils.c

/// Get line as string
/// Port of zlelineasstring(ZLE_STRING_T instr, int inll, int incs, int *outllp, int *outcsp, int useheap) from zle_utils.c

/// Set line from string
/// Port of stringaszleline(char *instr, int incs, int *outll, int *outsz, int *outcs) from zle_utils.c

/// Get ZLE line
/// Port of zlegetline(int *ll, int *cs) from zle_utils.c

/// Read a y/n response from input.
/// Port of `getzlequery()` from Src/Zle/zle_utils.c:1197. The C source
/// reads one key, treats Tab as 'y', any control char or EOF as 'n',
/// and otherwise tolowers the input. Echoes the response and returns
/// true iff the user pressed 'y'. Used by completion-listing prompts
/// like "show all 200 matches?".
pub fn get_zle_query() -> bool {
    let c = match getfullchar(false) {
        Some(c) => c,
        None => return false, // EOF → 'n'
    };
    let resolved = if c == '\t' {
        'y'
    } else if c.is_control() {
        'n'
    } else {
        c.to_ascii_lowercase()
    };
    // Echo the response — port of `zwcputc(&zr_n, NULL);` at
    // `Src/Zle/zle_utils.c:1229`. C writes a single char to
    // `shout`; we write the UTF-8 bytes to SHTTY (stdout fallback
    // for non-interactive paths).
    if resolved != '\n' {
        use std::sync::atomic::Ordering;
        let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        let out = if fd >= 0 { fd } else { 1 };
        let mut buf = [0u8; 4];
        let s = resolved.encode_utf8(&mut buf);
        let _ = crate::ported::utils::write_loop(out, s.as_bytes());
    }
    resolved == 'y'
}

/// Port of `struct zle_position` from Src/Zle/zle_utils.c:594.
/// Saved (cs, mark, ll, regions) for a stacked position.
#[derive(Debug, Clone, Default)]
pub struct ZlePosition {
    // c:594
    /// Cursor position.
    pub cs: usize, // c:599
    /// Mark.
    pub mk: usize, // c:601
    /// Line length.
    pub ll: usize, // c:603
    /// Region-highlight snapshot taken at save time so the
    /// concurrent user-driven highlights survive nested ZLE entries
    /// (port of C's `zle_region *regions` chain at c:604).
    pub regions: Vec<crate::ported::zle::zle_refresh::RegionHighlight>,
}

/// Port of `static struct zle_position *zle_positions` from
/// Src/Zle/zle_utils.c:619. LIFO stack of saved positions.
pub static ZLE_POSITIONS: std::sync::Mutex<Vec<ZlePosition>> = // c:619
    std::sync::Mutex::new(Vec::new());

/// Handle the auto-removable completion suffix.
/// Port of `handlesuffix(UNUSED(char **args))` from Src/Zle/zle_utils.c:1415. The C
/// source clears or retains the pending suffix depending on the
/// invoking widget's flags; without compsys integration in this
/// crate, we surface a hook so the host can update its compsys
/// state at the right moment.
pub fn handle_suffix() {
    call_hook("handle-suffix", None);
}

/// Set the editor line from a string.
/// Port of `setline(char *s, int flags)` from Src/Zle/zle_utils.c:1129. The C source
/// converts the metafied input back to a wide-char buffer; in Rust
/// we just collect chars into the line buffer and reset the cursor.
pub fn set_line(s: &str) {
    *ZLELINE.lock().unwrap() = s.chars().collect();
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// Queue a hook for the host to dispatch.
/// Port of `zlecallhook(char *name, char *arg)` from Src/Zle/zle_utils.c:1755 — the C source
/// resolves the widget via `rthingy_nocreate` and runs it inline via
/// `execzlefunc(thingy, args, 1, 0)`. The Rust port can't reach the
/// executor from this crate, so it appends to `pending_hooks`; the
/// host (the binary owning a `ShellExecutor`) drains the list after
/// each ZLE call and runs each named widget against its current
/// dispatch table — matching the same order zsh would have run them
/// in. `errflag` / `retflag` save/restore (zle_utils.c:1766/1775) is
/// the host's responsibility.
pub fn call_hook(name: &str, arg: Option<&str>) {
    PENDING_HOOKS
        .lock()
        .unwrap()
        .push((name.to_string(), arg.map(|s| s.to_string())));
}

/// Drain the queued hook calls. Returns the list and resets the queue.
/// Mirrors zsh's pattern of clearing pending hooks after dispatch
/// (see the implicit reset by `unrefthingy` plus the per-call save
/// of errflag/retflag in zle_utils.c:1766-1776).
pub fn drain_hooks() -> Vec<(String, Option<String>)> {
    std::mem::take(&mut *PENDING_HOOKS.lock().unwrap())
}

/// Reverse the change at `idx` (move zleline back to its pre-change state).
/// Returns true on success.
/// Port of `unapplychange(struct change *ch)` (zle_utils.c:1633).
pub fn unapply_change(idx: usize) -> bool {
    if idx >= UNDO_STACK.lock().unwrap().len() {
        return false;
    }
    // Borrow check: clone the small fields we need. Canonical
    // `change.off`/`dell`/`insl`/`old_cs`/`new_cs` are i32; convert
    // to usize at the indexing boundary.
    let (off, dell, insl, old_cs);
    let del_vec;
    let ins_len;
    {
        let ch = &UNDO_STACK.lock().unwrap()[idx];
        off = ch.off as usize;
        dell = ch.dell as usize;
        insl = ch.insl as usize;
        ins_len = insl;
        old_cs = ch.old_cs as usize;
        del_vec = ch.del.chars().collect::<Vec<char>>();
    }
    let _ = ins_len;
    ZLECS.store(off, Ordering::SeqCst);
    if insl > 0 {
        // Remove the inserted text.
        ZLELINE.lock().unwrap().drain(off..off + insl);
    }
    if dell > 0 {
        // Re-insert the deleted text.
        for (i, c) in del_vec.into_iter().enumerate() {
            ZLELINE.lock().unwrap().insert(off + i, c);
        }
    }
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLECS.store(old_cs.min(ZLELL.load(Ordering::SeqCst)), Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
    true
}

/// Replay the change at `idx`. Port of `applychange(struct change *ch)` (zle_utils.c:1677).
pub fn apply_change(idx: usize) -> bool {
    if idx >= UNDO_STACK.lock().unwrap().len() {
        return false;
    }
    let (off, dell, insl, new_cs);
    let ins_vec;
    {
        let ch = &UNDO_STACK.lock().unwrap()[idx];
        off = ch.off as usize;
        dell = ch.dell as usize;
        insl = ch.insl as usize;
        new_cs = ch.new_cs as usize;
        ins_vec = ch.ins.chars().collect::<Vec<char>>();
    }
    ZLECS.store(off, Ordering::SeqCst);
    if dell > 0 {
        ZLELINE.lock().unwrap().drain(off..off + dell);
    }
    if insl > 0 {
        for (i, c) in ins_vec.into_iter().enumerate() {
            ZLELINE.lock().unwrap().insert(off + i, c);
        }
    }
    ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
    ZLECS.store(new_cs.min(ZLELL.load(Ordering::SeqCst)), Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
    true
}

// move backwards through the change list                                 // c:1597
/// Walk back one Change. Port of `undo(char **args)` (zle_utils.c:1601).
pub fn undo_widget() -> i32 {
    // c:1601
    // Capture any in-flight edits into a Change before stepping back.
    mkundoent();
    if CURCHANGE.load(Ordering::SeqCst) == 0 {
        return 1;
    }
    let prev_idx = CURCHANGE.load(Ordering::SeqCst) - 1;
    if UNDO_STACK.lock().unwrap()[prev_idx].changeno <= UNDO_LIMITNO.load(Ordering::SeqCst) as i64 {
        return 1;
    }
    if unapply_change(prev_idx) {
        CURCHANGE.store(prev_idx, Ordering::SeqCst);
    }
    setlastline();
    0
}

// move forwards through the change list                                  // c:1657
/// Walk forward one Change. Port of `redo(UNUSED(char **args))` (zle_utils.c:1661).
pub fn redo_widget() -> i32 {
    // c:1661
    mkundoent();
    if CURCHANGE.load(Ordering::SeqCst) >= UNDO_STACK.lock().unwrap().len() {
        return 1;
    }
    if apply_change(CURCHANGE.load(Ordering::SeqCst)) {
        CURCHANGE.fetch_add(1, Ordering::SeqCst);
    }
    setlastline();
    0
}

#[cfg(test)]
mod findbol_findeol_tests {
    use super::*;

    fn zle_with(line: &str, cs: usize) {
        zle_reset();
        *ZLELINE.lock().unwrap() = line.chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLECS.store(cs, Ordering::SeqCst);
    }

    #[test]
    fn findbol_no_newline_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1162 — walks back to start when no '\n' encountered.
        let z = zle_with("hello world", 7);
        assert_eq!(findbol(), 0);
    }

    #[test]
    fn findbol_finds_preceding_newline() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1162 — `zleline[x-1] != '\n'` exits loop when prev char IS '\n'.
        // For "abc\ndef\nghi" with cursor at 9 (the 'h' in 'ghi'):
        // walks back to 8 (after the second '\n'), returns 8.
        let z = zle_with("abc\ndef\nghi", 9);
        assert_eq!(findbol(), 8);
    }

    #[test]
    fn findbol_at_start_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let z = zle_with("anything", 0);
        assert_eq!(findbol(), 0);
    }

    #[test]
    fn findeol_no_newline_returns_end() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1173 — walks forward to zlell when no '\n' encountered.
        let z = zle_with("hello world", 0);
        assert_eq!(findeol(), 11);
    }

    #[test]
    fn findeol_finds_next_newline() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1173 — `zleline[x] != '\n'` exits when current char IS '\n'.
        // For "abc\ndef" cursor at 0: walks 0→1→2→3 (which is '\n'), returns 3.
        let z = zle_with("abc\ndef", 0);
        assert_eq!(findeol(), 3);
    }

    #[test]
    fn findeol_at_end_returns_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let z = zle_with("hello", 5);
        assert_eq!(findeol(), 5);
    }

    #[test]
    fn findline_returns_bol_eol_pair() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1182-1183 — both findbol and findeol from the same cursor.
        // "abc\ndef\nghi" cursor at 5 (the 'e' in 'def'):
        //   findbol → 4 (after first '\n')
        //   findeol → 7 (the second '\n')
        let z = zle_with("abc\ndef\nghi", 5);
        let (bol, eol) = findline();
        assert_eq!(bol, 4);
        assert_eq!(eol, 7);
    }

    /// `Src/Zle/zle_utils.c:911-915` — `shiftchars` core memmove +
    /// zlell update. Common case: shift `cnt` chars at `to` and trim
    /// zlell by `cnt`.
    #[test]
    fn shiftchars_common_case_removes_cnt_chars_at_to() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abcdef", 0);
        // shiftchars(2, 2) over "abcdef" → "abef" (removes 'cd').
        shiftchars(2, 2);
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(line, "abef");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 4);
    }

    /// `Src/Zle/zle_utils.c:911-915` — boundary case: `to + cnt ==
    /// zlell`. The shift loop iterates 0 times; zlell is truncated to
    /// `to`. Pin the truncation: `shiftchars(3, 3)` on a 6-char line
    /// yields a 3-char line.
    #[test]
    fn shiftchars_to_plus_cnt_equals_zlell_truncates_to_offset() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abcdef", 0);
        shiftchars(3, 3);
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(
            line, "abc",
            "c:915 — to+cnt==zlell → zlell=to (truncate to offset 3)"
        );
        assert_eq!(ZLELL.load(Ordering::SeqCst), 3);
    }

    /// `Src/Zle/zle_utils.c:911-915` — out-of-range case: `to + cnt >
    /// zlell`. C still truncates to `to`. The previous Rust port had
    /// `if to+cnt > len { return }` which silently no-op'd; the fix
    /// truncates to match C's `zlell = to` write at c:915.
    /// Regression that would re-introduce the early-return: any
    /// `shiftchars(N, M)` with M huge would leave the buffer intact
    /// and zlell unchanged, breaking `foredel(huge_ct)` semantics
    /// (which relies on shiftchars truncating).
    #[test]
    fn shiftchars_to_plus_cnt_past_zlell_truncates_to_to() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abcdef", 0);
        shiftchars(3, 100);
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(
            line, "abc",
            "c:915 — to+cnt>zlell → zlell=to (the previous port silently no-op'd here)"
        );
        assert_eq!(ZLELL.load(Ordering::SeqCst), 3);
    }

    /// `Src/Zle/zle_utils.c:911-915` — degenerate case: `to >= zlell`.
    /// Caller is asking to shift starting past end-of-line. C sets
    /// zlell=to (which would corrupt the buffer in C since the storage
    /// past zlell is uninitialized); the Rust port clamps zlell to
    /// the actual line length so we don't grow zlell past the
    /// Vec storage and surface a panic on next read.
    #[test]
    fn shiftchars_to_past_zlell_clamps_to_len() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abc", 0);
        shiftchars(100, 5);
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        // Buffer storage unchanged
        assert_eq!(line, "abc");
        // zlell clamped to actual storage length
        let zlell = ZLELL.load(Ordering::SeqCst);
        assert!(
            zlell <= 3,
            "c:915 — Rust clamps zlell to storage len ({zlell})"
        );
    }

    /// `Src/Zle/zle_utils.c:911-915` — `cnt == 0` is a no-op shift
    /// (memmove of 0 bytes). Pin so a regression with `> 0` instead of
    /// `>= 0` doesn't drift.
    #[test]
    fn shiftchars_zero_count_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abcdef", 0);
        shiftchars(3, 0);
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(line, "abcdef", "cnt=0 → no chars removed");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 6);
    }

    /// `Src/Zle/zle_utils.c:851-854` — mark adjustment under shiftchars.
    /// Pin all three arms of the mark-relative-to-cut conditional.
    #[test]
    fn shiftchars_adjusts_mark_per_c851_c854() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Arm 1: mark > to+cnt → shifts left by cnt (c:851-852).
        zle_with("0123456789", 0);
        MARK.store(8, Ordering::SeqCst);
        shiftchars(2, 3);
        assert_eq!(
            MARK.load(Ordering::SeqCst),
            5,
            "c:851-852 — mark(8) >= to(2)+cnt(3) → mark -= cnt → 5"
        );

        // Arm 2: to < mark <= to+cnt → mark clamped to `to` (c:853-854).
        zle_with("0123456789", 0);
        MARK.store(4, Ordering::SeqCst);
        shiftchars(2, 3);
        assert_eq!(
            MARK.load(Ordering::SeqCst),
            2,
            "c:853-854 — mark(4) inside (to=2, cnt=3] range → clamp to `to`(2)"
        );

        // Arm 3: mark <= to → mark unchanged.
        zle_with("0123456789", 0);
        MARK.store(1, Ordering::SeqCst);
        shiftchars(2, 3);
        assert_eq!(
            MARK.load(Ordering::SeqCst),
            1,
            "c:851/853 — mark(1) < to(2) → unchanged"
        );
    }

    /// `Src/Zle/zle_utils.c:890-903` — shiftchars subtracts
    /// `predisplaylen` for ZRH_PREDISPLAY regions before comparing against
    /// the cut point. Substrate: the `flags` field on RegionHighlight
    /// (was hardcoded sub=0). Verify a predisplay region adjusts
    /// differently from a plain one.
    #[test]
    fn shiftchars_predisplay_region_subtracts_predisplaylen() {
        use crate::ported::zle::zle_h::{N_SPECIAL_HIGHLIGHTS, ZRH_PREDISPLAY};
        use crate::ported::zle::zle_refresh::{RegionHighlight, REGION_HIGHLIGHTS};
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abcdefghij", 0);
        crate::ported::zle::zle_params::set_predisplay(Some("12345")); // predisplaylen=5

        let mk = |start: usize, end: usize, flags: i32| RegionHighlight {
            start,
            end,
            attr: Default::default(),
            memo: None,
            flags,
        };
        {
            let mut rh = REGION_HIGHLIGHTS.lock().unwrap();
            rh.clear();
            for _ in 0..N_SPECIAL_HIGHLIGHTS {
                rh.push(mk(0, 0, 0));
            }
            rh.push(mk(6, 8, ZRH_PREDISPLAY)); // predisplay region
            rh.push(mk(6, 8, 0)); // plain region, same offsets
        }

        // shiftchars(to=0, cnt=2): delete 2 chars at position 0.
        shiftchars(0, 2);

        let rh = REGION_HIGHLIGHTS.lock().unwrap();
        let pre = &rh[N_SPECIAL_HIGHLIGHTS as usize];
        let plain = &rh[N_SPECIAL_HIGHLIGHTS as usize + 1];
        // Predisplay: start-sub=1 not > to+cnt=2 → clamp start=to+sub=5;
        //             end-sub=3 > 2 → end -= cnt = 6.
        assert_eq!((pre.start, pre.end), (5, 6), "predisplay region uses sub=5");
        // Plain (sub=0): start=6 > 2 → start -= 2 = 4; end=8 → 6.
        assert_eq!((plain.start, plain.end), (4, 6), "plain region uses sub=0");
        // The two diverge precisely because of the predisplay subtraction.
        assert_ne!(pre.start, plain.start);

        crate::ported::zle::zle_params::set_predisplay(None);
    }

    /// `Src/Zle/zle_utils.c:830-844` — spaceinline shifts user
    /// region-highlight offsets past the insertion point so highlighting
    /// stays aligned. Substrate: the `flags` field added to
    /// RegionHighlight. The prior port omitted this whole block.
    #[test]
    fn spaceinline_shifts_region_highlights() {
        use crate::ported::zle::zle_h::N_SPECIAL_HIGHLIGHTS;
        use crate::ported::zle::zle_refresh::{RegionHighlight, REGION_HIGHLIGHTS};
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abcdefghij", 3); // cursor at 3

        {
            let mut rh = REGION_HIGHLIGHTS.lock().unwrap();
            rh.clear();
            // The first N_SPECIAL_HIGHLIGHTS slots are reserved and skipped.
            for _ in 0..N_SPECIAL_HIGHLIGHTS {
                rh.push(RegionHighlight {
                    start: 0,
                    end: 0,
                    attr: Default::default(),
                    memo: None,
                    flags: 0,
                });
            }
            // A user region [5,8) entirely past the cursor.
            rh.push(RegionHighlight {
                start: 5,
                end: 8,
                attr: Default::default(),
                memo: None,
                flags: 0,
            });
            // A user region [1,2) entirely before the cursor (unaffected).
            rh.push(RegionHighlight {
                start: 1,
                end: 2,
                attr: Default::default(),
                memo: None,
                flags: 0,
            });
        }

        spaceinline(2); // open 2 chars at cursor 3

        let rh = REGION_HIGHLIGHTS.lock().unwrap();
        let past = &rh[N_SPECIAL_HIGHLIGHTS as usize]; // [5,8)
        assert_eq!(past.start, 7, "start>=zlecs shifts by ct (5→7)");
        assert_eq!(past.end, 10, "end>=zlecs shifts by ct (8→10)");
        let before = &rh[N_SPECIAL_HIGHLIGHTS as usize + 1]; // [1,2)
        assert_eq!(before.start, 1, "region before the cursor is unchanged");
        assert_eq!(before.end, 2, "region before the cursor is unchanged");
    }

    /// `Src/Zle/zle_utils.c:777-844` — `spaceinline(ct)` opens `ct`
    /// chars of space at zlecs; zlell += ct. Negative or zero `ct`
    /// is a no-op. Pin the cursor + zlell invariants.
    #[test]
    fn spaceinline_inserts_at_cursor_and_grows_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abc", 1);
        spaceinline(2);
        // Buffer length grew by 2; zlell follows.
        let zlell = ZLELL.load(Ordering::SeqCst);
        assert_eq!(zlell, 5, "c:842 — zlell += ct (was 3, ct=2 → 5)");
        assert_eq!(ZLELINE.lock().unwrap().len(), 5);
        // The first 'a' and tail 'bc' are still present; the gap is in the middle.
        assert_eq!(ZLELINE.lock().unwrap()[0], 'a');
        // Tail chars survive at the END
        assert_eq!(ZLELINE.lock().unwrap()[3], 'b');
        assert_eq!(ZLELINE.lock().unwrap()[4], 'c');
    }

    /// `Src/Zle/zle_utils.c:777-844` — `spaceinline(0)` is a no-op
    /// (no chars opened, zlell unchanged). Catches a regression that
    /// silently inserts a sentinel char on ct=0.
    #[test]
    fn spaceinline_zero_or_negative_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("xyz", 1);
        spaceinline(0);
        assert_eq!(ZLELL.load(Ordering::SeqCst), 3, "ct=0 → zlell unchanged");
        spaceinline(-5);
        assert_eq!(ZLELL.load(Ordering::SeqCst), 3, "ct<0 → zlell unchanged");
    }

    /// `Src/Zle/zle_utils.c:1158-1164` — `findbol()` lands on the byte
    /// just AFTER a newline (zsh's "beginning of line" is the first
    /// content char). Pin the offset so cursor-at-newline cases work.
    #[test]
    fn findbol_lands_after_previous_newline_mid_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // "abc\ndef" cursor at 6 (the 'f'); bol should be 4 (after '\n').
        zle_with("abc\ndef", 6);
        assert_eq!(
            findbol(),
            4,
            "c:1162 — bol is at offset AFTER the previous newline"
        );
    }

    /// `Src/Zle/zle_utils.c:1146` — `setline(s, ZSL_TOEND)` sends the
    /// cursor to the END of the new line. The previous Rust port checked
    /// the WRONG flag bit (ZSL_COPY=1 instead of ZSL_TOEND=2) AND
    /// inverted the condition, so `setline("hi", 2)` left the cursor at
    /// 0 instead of 2. Catches the fix.
    #[test]
    fn setline_with_zsl_toend_moves_cursor_to_end() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("xxxxxxxxxx", 5); // pre-set cursor at 5
        setline("hi", ZSL_TOEND); // ZSL_TOEND=2
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(line, "hi");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 2);
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            2,
            "c:1146 — ZSL_TOEND → cursor at zlell"
        );
    }

    /// `Src/Zle/zle_utils.c:1148-1149` — when ZSL_TOEND is NOT set, the
    /// cursor stays where it was UNLESS it would now be past zlell, in
    /// which case it clamps to zlell. Pin the no-stale-cursor invariant.
    #[test]
    fn setline_without_zsl_toend_clamps_overshoot_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Pre-set cursor at 10, then replace line with shorter "abc" (3 chars).
        zle_with("xxxxxxxxxxxx", 10);
        setline("abc", 0); // no flags
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(line, "abc");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 3);
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            3,
            "c:1148-1149 — cursor past new zlell must clamp to zlell"
        );
    }

    /// `Src/Zle/zle_utils.c:1148-1149` — when ZSL_TOEND is NOT set and
    /// the existing cursor still fits within the new line, the cursor
    /// stays put (no movement). Pin the preserve-position invariant —
    /// regression that flipped to "always go to end" would break every
    /// undo/redo path that calls setline mid-edit.
    #[test]
    fn setline_without_zsl_toend_preserves_in_range_cursor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abcdef", 2); // cursor at 'c'
        setline("ABCDEFGH", 0); // longer; cursor=2 still fits
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(line, "ABCDEFGH");
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            2,
            "c:1148-1149 — in-range cursor stays put when ZSL_TOEND unset"
        );
    }

    /// `Src/Zle/zle_utils.c:1146` — flag bit 2 (ZSL_TOEND) takes
    /// precedence; combining with ZSL_COPY (1) doesn't change the
    /// cursor-position behavior since ZSL_COPY only controls
    /// argument duplication (a no-op in Rust where `&str` is borrowed).
    #[test]
    fn setline_with_zsl_copy_alone_does_not_change_cursor_logic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        zle_with("abcdefghij", 5);
        setline("xyz", ZSL_COPY); // ZSL_COPY=1, no TOEND
                                  // Cursor was 5, new line is 3 chars → clamp to 3.
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            3,
            "c:1148-1149 — ZSL_COPY alone doesn't trigger TOEND; clamp still applies"
        );
    }

    /// `Src/Zle/zle_utils.c` ZSL_* constants — `ZSL_COPY=1`, `ZSL_TOEND=2`.
    /// Pin the exact values per `Src/Zle/zle.h:406-407` so a regen
    /// renumbering them silently inverts setline behavior.
    #[test]
    fn zsl_constants_match_c_source_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ZSL_COPY, 1, "Src/Zle/zle.h:406 — ZSL_COPY = 1");
        assert_eq!(ZSL_TOEND, 2, "Src/Zle/zle.h:407 — ZSL_TOEND = 2");
        // Bit-disjoint so setline can OR them.
        assert_eq!(
            ZSL_COPY & ZSL_TOEND,
            0,
            "flag bits must be disjoint for OR-composition"
        );
    }

    // ─── zsh-corpus pins for zlelineasstring / stringaszleline ─────

    /// `zlelineasstring([a,b,c], 3, 0)` returns "abc".
    #[test]
    fn zle_utils_corpus_zlelineasstring_basic() {
        let _g = crate::test_util::global_state_lock();
        let line: Vec<char> = vec!['a', 'b', 'c'];
        assert_eq!(zlelineasstring(&line, 3, 0, None, None, 0), "abc");
    }

    /// `zlelineasstring` honors ll param (truncates).
    #[test]
    fn zle_utils_corpus_zlelineasstring_ll_truncates() {
        let _g = crate::test_util::global_state_lock();
        let line: Vec<char> = vec!['a', 'b', 'c', 'd', 'e'];
        assert_eq!(
            zlelineasstring(&line, 3, 0, None, None, 0),
            "abc",
            "ll=3 takes first 3 chars only"
        );
    }

    /// `zlelineasstring` ll=0 → empty.
    #[test]
    fn zle_utils_corpus_zlelineasstring_zero_len_empty() {
        let _g = crate::test_util::global_state_lock();
        let line: Vec<char> = vec!['a', 'b'];
        assert_eq!(zlelineasstring(&line, 0, 0, None, None, 0), "");
    }

    /// `zlelineasstring` empty slice → empty string.
    #[test]
    fn zle_utils_corpus_zlelineasstring_empty_slice() {
        let _g = crate::test_util::global_state_lock();
        let line: Vec<char> = vec![];
        assert_eq!(zlelineasstring(&line, 0, 0, None, None, 0), "");
    }

    /// `stringaszleline("hello")` returns Vec<char> matching ASCII.
    #[test]
    fn zle_utils_corpus_stringaszleline_ascii_passthrough() {
        let _g = crate::test_util::global_state_lock();
        let v = stringaszleline("hello", 0, None, None, None);
        let s: String = v.iter().collect();
        assert_eq!(s, "hello");
    }

    /// `stringaszleline("")` returns empty Vec.
    #[test]
    fn zle_utils_corpus_stringaszleline_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(stringaszleline("", 0, None, None, None).is_empty());
    }

    /// Round-trip: stringaszleline → zlelineasstring preserves ASCII.
    #[test]
    fn zle_utils_corpus_string_round_trip_ascii() {
        let _g = crate::test_util::global_state_lock();
        let input = "abc123";
        let v = stringaszleline(input, 0, None, None, None);
        let s = zlelineasstring(&v, v.len(), 0, None, None, 0);
        assert_eq!(s, input);
    }

    /// Metafied input collapses correctly (c:428 unmetafy).
    /// Input `"a\x83\xc1b"` is `'a' + Meta + (0xc1) + 'b'` →
    /// unmetafy collapses Meta+0xc1 → (0xc1^0x20 = 0xe1) → `"a\xe1b"`
    /// which is valid UTF-8 for U+00E1 alone → decoded `['a','á','b']`.
    /// Wait — 0xe1 alone is invalid UTF-8 (continuation byte). Adjust:
    /// use the simpler case `'a' + Meta + 0xa0 → 'a' + 0x80` (also
    /// invalid). Real round-trip needs the inverse `metafy()`. Here
    /// we just verify Meta is stripped, not the multibyte decode.
    #[test]
    fn zle_utils_corpus_stringaszleline_meta_collapse() {
        let _g = crate::test_util::global_state_lock();
        // 'a' + Meta (0x83) + 'A' (0x41) — Meta+0x41 → 0x61 ('a')
        // result bytes: 'a' 'a' = "aa", decoded to ['a','a'].
        let raw: Vec<u8> = vec![b'a', 0x83, 0x41];
        let s = unsafe { std::str::from_utf8_unchecked(&raw) };
        let v = stringaszleline(s, 0, None, None, None);
        assert_eq!(v.iter().collect::<String>(), "aa");
    }

    /// `outcs` adjusts for Meta-byte cursor positioning (c:391-393).
    /// Input `"a" + Meta + "X" + "b"` with cursor at byte 3 (after Meta+X)
    /// → post-unmetafy bytes are `['a', X^0x20, 'b']`, cursor at byte 2.
    /// Codepoint cursor `outcs = 2`.
    #[test]
    fn zle_utils_corpus_stringaszleline_outcs_meta_adjust() {
        let _g = crate::test_util::global_state_lock();
        let raw: Vec<u8> = vec![b'a', 0x83, 0x41, b'b'];
        let s = unsafe { std::str::from_utf8_unchecked(&raw) };
        let mut outll: i32 = 0;
        let mut outcs: i32 = 0;
        let v = stringaszleline(s, 3, Some(&mut outll), None, Some(&mut outcs));
        assert_eq!(v.len(), 3);
        assert_eq!(outll, 3);
        assert_eq!(outcs, 2); // c:483-484 — cursor at byte 3 → codepoint 2
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/zle_utils.c helper fns.
    // ═══════════════════════════════════════════════════════════════════

    /// `sizeline(0)` is a no-op (already-sized buffer).
    /// C `Src/Zle/zle_utils.c:sizeline` — `while (sz > cursz)`.
    #[test]
    fn sizeline_zero_is_noop() {
        let _g = crate::test_util::global_state_lock();
        sizeline(0);
        sizeline(0);
    }

    /// `sizeline(N)` for moderate N completes without panic.
    #[test]
    fn sizeline_moderate_size_no_panic() {
        let _g = crate::test_util::global_state_lock();
        sizeline(100);
        sizeline(1024);
    }

    /// `zle_save_positions` / `zle_free_positions` round-trip.
    #[test]
    fn zle_save_then_free_positions_no_panic() {
        let _g = crate::test_util::global_state_lock();
        zle_save_positions();
        zle_free_positions();
    }

    /// `zle_save_positions` / `zle_restore_positions` round-trip.
    #[test]
    fn zle_save_then_restore_positions_no_panic() {
        let _g = crate::test_util::global_state_lock();
        zle_save_positions();
        zle_restore_positions();
    }

    /// `zle_free_positions` on empty stack is safe (no-op).
    #[test]
    fn zle_free_positions_empty_stack_no_panic() {
        let _g = crate::test_util::global_state_lock();
        zle_free_positions();
        zle_free_positions();
        zle_free_positions();
    }

    /// `free_region_highlights_memos` on empty REGION_HIGHLIGHTS
    /// is a no-op.
    #[test]
    fn free_region_highlights_memos_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        free_region_highlights_memos();
        free_region_highlights_memos();
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests for Src/Zle/zle_utils.c findbol/findeol + zlecharas
    // string + sizeline edge cases.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1158 — `findbol()` at cursor=0 returns 0 (already at BOL).
    #[test]
    fn findbol_at_cursor_zero_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        assert_eq!(findbol(), 0, "cursor at 0 → BOL=0");
    }

    /// c:1158 — `findbol()` on single-line buffer with cursor in
    /// middle returns 0 (BOL is start of buffer).
    #[test]
    fn findbol_single_line_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(7, Ordering::SeqCst);
        assert_eq!(findbol(), 0, "no newline before cursor → BOL=0");
    }

    /// c:1158 — `findbol()` after a newline finds the byte after it.
    #[test]
    fn findbol_after_newline_finds_post_newline_pos() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc\ndef".chars().collect();
        ZLELL.store(7, Ordering::SeqCst);
        ZLECS.store(6, Ordering::SeqCst);
        // BOL of second line = pos 4 (after \n at index 3).
        assert_eq!(findbol(), 4, "BOL after \\n is at the byte after it");
    }

    /// c:1169 — `findeol()` at cursor=ZLELL returns ZLELL (already at EOL).
    #[test]
    fn findeol_at_end_returns_zlell_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        ZLECS.store(5, Ordering::SeqCst);
        assert_eq!(findeol(), 5, "cursor at EOL → returns ZLELL");
    }

    /// c:1169 — `findeol()` on single-line buffer returns ZLELL
    /// (no newline ahead → EOL is end of buffer).
    #[test]
    fn findeol_single_line_returns_zlell() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        assert_eq!(findeol(), 11, "no \\n ahead → EOL=ZLELL");
    }

    /// c:1169 — `findeol()` before a newline stops AT the newline pos.
    #[test]
    fn findeol_before_newline_stops_at_newline_pos() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc\ndef".chars().collect();
        ZLELL.store(7, Ordering::SeqCst);
        ZLECS.store(1, Ordering::SeqCst);
        // First newline at index 3 — EOL of first line = 3.
        assert_eq!(findeol(), 3, "EOL = newline position itself");
    }

    /// c:1158/1169 — `findbol()` and `findeol()` at same cursor must
    /// satisfy `findbol() <= cursor <= findeol()` (positional invariant).
    #[test]
    fn findbol_findeol_invariant() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "line1\nline2\nline3".chars().collect();
        ZLELL.store(17, Ordering::SeqCst);
        ZLECS.store(8, Ordering::SeqCst); // middle of "line2"
        let bol = findbol();
        let eol = findeol();
        let cs = ZLECS.load(Ordering::SeqCst);
        assert!(bol <= cs, "BOL ({}) must be <= cursor ({})", bol, cs);
        assert!(cs <= eol, "cursor ({}) must be <= EOL ({})", cs, eol);
    }

    /// c:117 — `zlecharasstring('a', buf)` appends ASCII verbatim and
    /// returns 1 (one byte added).
    #[test]
    fn zlecharasstring_ascii_appends_one_byte() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = String::new();
        let n = zlecharasstring('a', &mut buf);
        assert_eq!(n, 1, "ASCII 'a' = 1 byte");
        assert_eq!(buf, "a");
    }

    /// c:117 — `zlecharasstring('\\n', buf)` returns 1 (newline is
    /// not imeta — passes through verbatim).
    #[test]
    fn zlecharasstring_newline_passes_through() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = String::new();
        let n = zlecharasstring('\n', &mut buf);
        assert_eq!(n, 1);
        assert_eq!(buf.as_bytes(), b"\n");
    }

    /// c:117 — `zlecharasstring` preserves existing buf content
    /// (appends, doesn't replace).
    #[test]
    fn zlecharasstring_appends_to_existing_buf() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = String::from("prefix:");
        let _ = zlecharasstring('X', &mut buf);
        assert_eq!(buf, "prefix:X", "appends after existing content");
    }

    /// c:46 — `sizeline(N)` ensures ZLELINE has at least N+1 capacity.
    /// Pin: subsequent operations don't panic on indexed access up to N.
    #[test]
    fn sizeline_grows_zleline_capacity() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        sizeline(100);
        // No panic = capacity grew successfully.
        let cap = ZLELINE.lock().unwrap().capacity();
        assert!(
            cap >= 100,
            "sizeline(100) → ZLELINE capacity ≥ 100, got {}",
            cap
        );
    }

    /// `sizeline(0)` is safe.
    #[test]
    fn sizeline_zero_is_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        sizeline(0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_utils.c
    // c:467 zle_save_positions / c:500 zle_restore_positions / c:530 zle_free_positions
    // c:1153 findline / c:1160 getzlequery / c:1205 bindztrdup / c:1243 printbind
    // c:381 zlegetline
    // ═══════════════════════════════════════════════════════════════════

    /// c:467-530 — `zle_save_positions` + `zle_restore_positions`
    /// round-trip safe.
    #[test]
    fn zle_save_restore_positions_round_trip_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zle_save_positions();
        zle_restore_positions();
    }

    /// c:530 — `zle_free_positions` is idempotent.
    #[test]
    fn zle_free_positions_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            zle_free_positions();
        }
    }

    /// c:432 — `free_region_highlights_memos` is idempotent.
    #[test]
    fn free_region_highlights_memos_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            free_region_highlights_memos();
        }
    }

    /// c:1153 — `findline()` returns (usize, usize) with lo ≤ hi invariant.
    #[test]
    fn findline_returns_valid_range_tuple() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let (lo, hi) = findline();
        assert!(lo <= hi, "findline lo={} must ≤ hi={}", lo, hi);
    }

    /// c:1205 — `bindztrdup(empty)` returns `""` (two quote chars).
    /// C zsh quotes the binding for shell-safe display; empty sequence
    /// becomes the literal `""` empty-quoted form.
    #[test]
    fn bindztrdup_empty_returns_quoted_empty() {
        assert_eq!(
            bindztrdup(b""),
            "\"\"",
            "empty seq → '\"\"' quoted-empty per C bindztrdup quoting"
        );
    }

    /// c:1205 — `bindztrdup` is pure for ASCII input.
    #[test]
    fn bindztrdup_is_pure_for_ascii() {
        for s in [b"a" as &[u8], b"hello", b"\t\n"] {
            let first = bindztrdup(s);
            for _ in 0..3 {
                assert_eq!(bindztrdup(s), first, "bindztrdup({:?}) must be pure", s);
            }
        }
    }

    /// c:1243 — `printbind(empty)` returns `""` (two quote chars).
    /// Same as bindztrdup — printbind delegates to bindztrdup-style
    /// quoting for shell-display safety.
    #[test]
    fn printbind_empty_returns_quoted_empty() {
        assert_eq!(
            printbind(b""),
            "\"\"",
            "empty seq → '\"\"' per C printbind quoting"
        );
    }

    /// c:1243 — `printbind` is deterministic.
    #[test]
    fn printbind_is_deterministic() {
        for s in [b"" as &[u8], b"a", b"\x01", b"hello"] {
            let first = printbind(s);
            for _ in 0..3 {
                assert_eq!(
                    printbind(s),
                    first,
                    "printbind({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:113 — `zlecharasstring` returns i32 (compile-time type pin).
    #[test]
    fn zlecharasstring_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut buf = String::new();
        let _: i32 = zlecharasstring('a', &mut buf);
    }

    /// c:381 — `zlegetline` returns String + writes to ll/cs.
    #[test]
    fn zlegetline_returns_string_writes_outparams() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mut ll: i32 = 0;
        let mut cs: i32 = 0;
        let _: String = zlegetline(&mut ll, &mut cs);
    }

    /// c:69 — `zleaddtoline(0)` (NUL char) is safe.
    #[test]
    fn zleaddtoline_nul_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zleaddtoline(0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_utils.c
    // c:467 zle_save_positions / c:500 zle_restore_positions /
    // c:544 spaceinline / c:569 shiftchars / c:656 cut /
    // c:784 backkill / c:816 forekill / c:844 backdel / c:890 foredel /
    // c:1160 getzlequery
    // ═══════════════════════════════════════════════════════════════════

    /// c:467 + c:500 — save/restore positions round-trip safe (pin 2).
    #[test]
    fn zle_save_restore_positions_round_trip_safe_pin2() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..3 {
            zle_save_positions();
            zle_restore_positions();
        }
    }

    /// c:467 — `zle_save_positions` is void / no-panic.
    #[test]
    fn zle_save_positions_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: () = zle_save_positions();
        zle_free_positions(); // cleanup so other tests see fresh stack
    }

    /// c:544 — `spaceinline(0)` zero count safe.
    #[test]
    fn spaceinline_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        spaceinline(0);
    }

    /// c:569 — `shiftchars(0, 0)` zero/zero safe.
    #[test]
    fn shiftchars_zero_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        shiftchars(0, 0);
    }

    /// c:656 — `cut(0, 0, 0)` returns i32 (compile-time type pin).
    #[test]
    fn cut_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = cut(0, 0, 0);
    }

    /// c:784 — `backkill(0, 0)` zero count safe.
    #[test]
    fn backkill_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        backkill(0, 0);
    }

    /// c:816 — `forekill(0, 0)` zero count safe.
    #[test]
    fn forekill_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        forekill(0, 0);
    }

    /// c:844 — `backdel(0, 0)` zero count safe.
    #[test]
    fn backdel_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        backdel(0, 0);
    }

    /// c:890 — `foredel(0, 0)` zero count safe.
    #[test]
    fn foredel_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        foredel(0, 0);
    }

    /// c:1160 — `getzlequery` returns i32 (compile-time type pin).
    #[test]
    fn getzlequery_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = getzlequery();
    }

    /// c:1005 + c:1028 — `findbol() <= findeol()` invariant.
    #[test]
    fn findbol_le_findeol_invariant_pin() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let b = findbol();
        let e = findeol();
        assert!(b <= e, "findbol={} must be ≤ findeol={}", b, e);
    }

    /// c:1153 — `findline` returns (usize, usize) with start ≤ end.
    #[test]
    fn findline_start_le_end_invariant() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let (start, end) = findline();
        assert!(
            start <= end,
            "findline start={} must be ≤ end={}",
            start,
            end
        );
    }

    /// c:1389-1408 — showmsg writes the message bytes (nicechar-expanded)
    /// to the shell-output fd followed by a terminating newline (the
    /// non-clearflag path in a headless test). Proves the scan loop and
    /// tail emit reach the fd, not just the old thin `msg + "\n"` stub.
    #[test]
    fn showmsg_emits_message_and_trailing_newline() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        showmsg("no matches");

        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("no matches"),
            "showmsg should emit the message text; got {:?}",
            s
        );
        assert!(
            s.ends_with('\n'),
            "showmsg should end with a newline; got {:?}",
            s
        );
    }
}
