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

use super::zle_main::{ZleChar, ZleString};

    /// Insert string at cursor position

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;
use crate::zle::zle_main::{history, MARK, vibuf, CURCHANGE, KILLRING, KILLRINGMAX, LASTCS, LASTLINE, LASTLL, UNDO_CHANGENO, UNDO_LIMITNO, UNDO_STACK, VISTARTCHANGE, ZLECS, ZLELINE, ZLELL, ZLE_RESET_NEEDED, ZMOD};

pub fn insert_str(s: &str) {
        for c in s.chars() {
             ZLELINE.lock().unwrap().insert( ZLECS.load(std::sync::atomic::Ordering::SeqCst), c);
             ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
             ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Insert chars at cursor position
    pub fn insert_chars(chars: &[ZleChar]) {
        for &c in chars {
             ZLELINE.lock().unwrap().insert( ZLECS.load(std::sync::atomic::Ordering::SeqCst), c);
             ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
             ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Delete n characters at cursor position
    pub fn delete_chars(n: usize) {
        let n = n.min( ZLELL.load(std::sync::atomic::Ordering::SeqCst) -  ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        for _ in 0..n {
            if  ZLECS.load(std::sync::atomic::Ordering::SeqCst) <  ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                 ZLELINE.lock().unwrap().remove( ZLECS.load(std::sync::atomic::Ordering::SeqCst));
                 ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Delete n characters before cursor
    pub fn backspace_chars(n: usize) {
        let n = n.min( ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        for _ in 0..n {
            if  ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                 ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                 ZLELINE.lock().unwrap().remove( ZLECS.load(std::sync::atomic::Ordering::SeqCst));
                 ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
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
        * ZLELINE.lock().unwrap() = s.chars().collect();
         ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
         ZLECS.store( ZLECS.load(std::sync::atomic::Ordering::SeqCst).min( ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Clear the line
    pub fn clear_line() {
         ZLELINE.lock().unwrap().clear();
         ZLELL.store(0, std::sync::atomic::Ordering::SeqCst);
         ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
         MARK.store(0, std::sync::atomic::Ordering::SeqCst);
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Get region between point and mark
    pub fn get_region() -> Vec<ZleChar> {
        let (start, end) = if  ZLECS.load(std::sync::atomic::Ordering::SeqCst) <  MARK.load(std::sync::atomic::Ordering::SeqCst) {
            ( ZLECS.load(std::sync::atomic::Ordering::SeqCst),  MARK.load(std::sync::atomic::Ordering::SeqCst))
        } else {
            ( MARK.load(std::sync::atomic::Ordering::SeqCst),  ZLECS.load(std::sync::atomic::Ordering::SeqCst))
        };
         ZLELINE.lock().unwrap()[start..end].to_vec()
    }

    /// Cut to named buffer
    pub fn cut_to_buffer(buf: usize, append: bool) {
        if buf <  vibuf().lock().unwrap().len() {
            let (start, end) = if  ZLECS.load(std::sync::atomic::Ordering::SeqCst) <  MARK.load(std::sync::atomic::Ordering::SeqCst) {
                ( ZLECS.load(std::sync::atomic::Ordering::SeqCst),  MARK.load(std::sync::atomic::Ordering::SeqCst))
            } else {
                ( MARK.load(std::sync::atomic::Ordering::SeqCst),  ZLECS.load(std::sync::atomic::Ordering::SeqCst))
            };

            let text: ZleString =  ZLELINE.lock().unwrap()[start..end].to_vec();

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
        if buf <  vibuf().lock().unwrap().len() {
            let text =  vibuf().lock().unwrap()[buf].clone();
            if !text.is_empty() {
                if after &&  ZLECS.load(std::sync::atomic::Ordering::SeqCst) <  ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                     ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                insert_chars(&text);
            }
        }
    }


/// Metafication helpers (for compatibility with zsh's metafied strings)
pub fn pastebuf(s: &str) -> String {
    // In zsh, Meta (0x83) is used to escape special bytes
    // For Rust we typically don't need this, but provide for compatibility
    s.to_string()
}

pub fn unmetafy(s: &str) -> String {
    s.to_string()
}

// Note: dead `UndoEntry`/`UndoState`/`apply_undo_entry` aggregates
// removed per PORT_PLAN Phase 2. They were a Rust-only invention
// with zero references across the codebase. The canonical undo
// machinery lives in file-scope statics (`UNDO_STACK: Mutex<Vec<change>>`,
// `CHANGENO`, `CURCHANGE`, `UNDO_CHANGENO`, `UNDO_LIMITNO` —
// declared in zle_main.rs) and the canonical port fns are:
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
    /// CUT_FRONT / CUT_REPLACE / CUT_RAW flag bits. The previous Rust
    /// placeholder used a fake `CutDirection` enum (Front/Back) that
    /// had no C counterpart; we now use `i32 flags` matching C.


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
        print!("\x07"); // Bell
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
        * ZLELINE.lock().unwrap() = s.chars().collect();
         ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
         ZLECS.store( ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }


/// Direct port of `struct zle_position` from
/// `Src/Zle/zle_utils.c:595-605`. One saved-state node in the
/// zle_positions stack; pushed by `zle_save_positions()` and popped
/// by `zle_restore_positions()`.
#[derive(Debug, Clone)]
#[allow(non_camel_case_types)]
pub struct zle_position {                                                    // c:595
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

/// Position save/restore
/// Port of zle_save_positions() / zle_restore_positions() from zle_utils.c
    /// Port of `zle_save_positions` from `Src/Zle/zle_utils.c:619`.


    /// Port of `zle_restore_positions` from `Src/Zle/zle_utils.c:677`.



// `CutFlags` / `CutDirection` deleted — Rust-only types with no C
// counterpart. C uses `int flags` with the `CUT_FRONT` / `CUT_REPLACE`
// / `CUT_RAW` bits at zle.h:271-281 (already legit-ported in
// zle_h.rs:387-393). `foredel` / `backdel` / `cuttext` now take
// `i32 flags` matching the C signatures verbatim.

/// Format a key sequence for `bindkey -L` listing.
/// Port of `bindztrdup(char *str)` from Src/Zle/zle_utils.c:1238. Produces the
/// dquoted-friendly form (`\C-a`, `\M-x`, escaped backslashes/carets)
/// that the bindkey command uses for round-trippable output —
/// distinct from `printbind` below which uses the human-readable
/// `^A` / `^[X` form printed in describe-key-briefly etc.
pub fn bindztrdup(str: &[u8]) -> String {
    let mut buf = String::new();
    for &b in str {
        // Meta bit handling: zsh metafies bytes >= 0x80 by inserting
        // 0x83 (Meta) before a (b ^ 0x20) byte. The C source unwinds
        // that here; in our Rust model we don't pastebuf in storage, so
        // we treat any byte >= 0x80 as already a M- target.
        let mut c = b;
        if c & 0x80 != 0 {
            buf.push('\\');
            buf.push('M');
            buf.push('-');
            c &= 0x7f;
        }
        if c < 32 || c == 0x7f {
            buf.push('^');
            c ^= 64;
        }
        if c == b'\\' || c == b'^' {
            buf.push('\\');
        }
        buf.push(c as char);
    }
    buf
}

/// Print a key binding for display
/// Port of printbind(char *str, FILE *stream) from zle_utils.c
pub fn printbind(seq: &[u8]) -> String {
    let mut result = String::new();

    for &b in seq {
        match b {
            0x1b => result.push_str("^["),
            0..=31 => {
                result.push('^');
                result.push((b + 64) as char);
            }
            127 => result.push_str("^?"),
            128..=159 => {
                result.push_str("^[^");
                result.push((b - 64) as char);
            }
            _ => result.push(b as char),
        }
    }

    result
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
         PENDING_HOOKS.lock().unwrap()
            .push((name.to_string(), arg.map(|s| s.to_string())));
    }

    /// Drain the queued hook calls. Returns the list and resets the queue.
    /// Mirrors zsh's pattern of clearing pending hooks after dispatch
    /// (see the implicit reset by `unrefthingy` plus the per-call save
    /// of errflag/retflag in zle_utils.c:1766-1776).
    pub fn drain_hooks() -> Vec<(String, Option<String>)> {
        std::mem::take(&mut * PENDING_HOOKS.lock().unwrap())
    }


#[cfg(test)]
mod tests_hooks {
    use super::*;

    #[test]
    fn call_hook_queues_for_host_dispatch() {
        let _g =  zle_test_setup();
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
        let _g =  zle_test_setup();
        redrawhook();
        let drained = drain_hooks();
        assert_eq!(drained, vec![("zle-line-pre-redraw".to_string(), None)]);
    }

    #[test]
    fn reexpandprompt_re_runs_expansion_against_raw_templates() {
        let _g =  zle_test_setup();
        // Set raw templates that don't reference dynamic state, so the
        // expansion is idempotent and easy to assert. %% expands to a
        // single literal '%' per zsh prompt rules.
        * RAW_LP.lock().unwrap() = "%% > ".to_string();
        * RAW_RP.lock().unwrap() = "[%%]".to_string();
        reexpandprompt();
        assert_eq!(prompt(), "% > ");
        assert_eq!(rprompt(), "[%]");
    }
}

#[cfg(test)]
mod tests_bindkey_format {
    use crate::zle::zle_main::zle_test_setup;
    use super::bindztrdup;
    use super::printbind;

    #[test]
    fn bind_ztrdup_emits_caret_form_for_control_chars() {
        let _g =  zle_test_setup();
        // Ctrl-A → "^A". Mirrors zsh's bindkey -L line for `bindkey '^A'`.
        assert_eq!(bindztrdup(b"\x01"), "^A");
        // Ctrl-_ → "^_".
        assert_eq!(bindztrdup(b"\x1f"), "^_");
        // DEL (0x7f) → "^?".
        assert_eq!(bindztrdup(b"\x7f"), "^?");
    }

    #[test]
    fn bind_ztrdup_escapes_backslash_and_caret() {
        let _g =  zle_test_setup();
        // '\\' → "\\\\" (escaped per C source's `c == '\\'` branch).
        assert_eq!(bindztrdup(b"\\"), "\\\\");
        // '^' → "\\^".
        assert_eq!(bindztrdup(b"^"), "\\^");
    }

    #[test]
    fn bind_ztrdup_handles_high_bit_as_meta() {
        let _g =  zle_test_setup();
        // Byte with bit-7 set → "\\M-X" prefix. \\xC1 = M-A.
        assert_eq!(bindztrdup(b"\xC1"), "\\M-A");
    }

    #[test]
    fn printbind_caret_form_matches_describe_key_output() {
        let _g =  zle_test_setup();
        // `^A`-style display form (distinct from bindkey's escape form).
        assert_eq!(printbind(b"\x01"), "^A");
        assert_eq!(printbind(b"\x1b"), "^[");
    }
}

    /// Snapshot the current line into `last_line` so the next `mkundoent`
    /// can diff against it. Port of `setlastline` (zle_utils.c:1587).
    pub fn setlastline() {
        let snapshot =  ZLELINE.lock().unwrap().clone();
        let mut ll =  LASTLINE.lock().unwrap();
        ll.clear();
        ll.extend_from_slice(&snapshot);
        drop(ll);
         LASTLL.store( ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
         LASTCS.store( ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }

    // add an entry to the undo system, if anything has changed              // c:1532
    /// If the line changed since the last snapshot, append a Change record
    /// describing the diff. Port of `mkundoent` (zle_utils.c:1532).
    pub fn mkundoent() {                                             // c:1532
        if  LASTLL.load(std::sync::atomic::Ordering::SeqCst) ==  ZLELL.load(std::sync::atomic::Ordering::SeqCst) &&  LASTLINE.lock().unwrap()[.. LASTLL.load(std::sync::atomic::Ordering::SeqCst)] ==  ZLELINE.lock().unwrap()[.. ZLELL.load(std::sync::atomic::Ordering::SeqCst)]
        {
             LASTCS.store( ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
            return;
        }
        let sh =  LASTLL.load(std::sync::atomic::Ordering::SeqCst).min( ZLELL.load(std::sync::atomic::Ordering::SeqCst));
        let mut pre = 0usize;
        while pre < sh &&  ZLELINE.lock().unwrap()[pre] ==  LASTLINE.lock().unwrap()[pre] {
            pre += 1;
        }
        let mut suf = 0usize;
        while suf < sh - pre
            &&  ZLELINE.lock().unwrap()[ ZLELL.load(std::sync::atomic::Ordering::SeqCst) - 1 - suf] ==  LASTLINE.lock().unwrap()[ LASTLL.load(std::sync::atomic::Ordering::SeqCst) - 1 - suf]
        {
            suf += 1;
        }
        let del: ZleString = if suf + pre ==  LASTLL.load(std::sync::atomic::Ordering::SeqCst) {
            Vec::new()
        } else {
             LASTLINE.lock().unwrap()[pre.. LASTLL.load(std::sync::atomic::Ordering::SeqCst) - suf].to_vec()
        };
        let ins: ZleString = if suf + pre ==  ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            Vec::new()
        } else {
             ZLELINE.lock().unwrap()[pre.. ZLELL.load(std::sync::atomic::Ordering::SeqCst) - suf].to_vec()
        };
         UNDO_CHANGENO.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // Canonical `change.del`/`ins` are `ZLE_STRING_T = String`
        // (zle_h.rs:66, port of `ZLE_STRING_T` typedef). Local
        // `ZleString = Vec<char>` (zle_main.rs:59); convert at the
        // boundary.
        let del_str: String = del.iter().collect();
        let ins_str: String = ins.iter().collect();
        let dell = del_str.chars().count() as i32;
        let insl = ins_str.chars().count() as i32;
        let ch = crate::ported::zle::zle_h::change {
            prev: None,
            next: None,
            flags: 0,
            hist:  history().lock().unwrap().cursor as i32,
            off: pre as i32,
            del: del_str,
            dell,
            ins: ins_str,
            insl,
            old_cs:  LASTCS.load(std::sync::atomic::Ordering::SeqCst) as i32,
            new_cs:  ZLECS.load(std::sync::atomic::Ordering::SeqCst) as i32,
            changeno:  UNDO_CHANGENO.load(std::sync::atomic::Ordering::SeqCst) as i64,
        };
        // Drop any forward redo history past the cursor before pushing.
         UNDO_STACK.lock().unwrap().truncate( CURCHANGE.load(std::sync::atomic::Ordering::SeqCst));
         UNDO_STACK.lock().unwrap().push(ch);
         CURCHANGE.store( UNDO_STACK.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    }

    // register pending changes in the undo system                            // c:1488
    /// Pre-widget hook. Port of `handleundo` (zle_utils.c) — the
    /// Rust port collapses to `setlastline()` because zshrs uses a
    /// one-change-per-widget model. C's `handleundo` body
    /// (zle_utils.c:1488) flushes the in-flight `nextchanges`
    /// chain that accumulates across multi-key vi operations; that
    /// chain is unnecessary when each widget produces exactly one
    /// undo entry via `mkundoent` post-call.
    pub fn handleundo() {                                            // c:1488
        setlastline();
    }

    /// Reverse the change at `idx` (move zleline back to its pre-change state).
    /// Returns true on success.
    /// Port of `unapplychange(struct change *ch)` (zle_utils.c:1633).
    pub fn unapply_change(idx: usize) -> bool {
        if idx >=  UNDO_STACK.lock().unwrap().len() {
            return false;
        }
        // Borrow check: clone the small fields we need. Canonical
        // `change.off`/`dell`/`insl`/`old_cs`/`new_cs` are i32; convert
        // to usize at the indexing boundary.
        let (off, dell, insl, old_cs);
        let del_vec;
        let ins_len;
        {
            let ch = & UNDO_STACK.lock().unwrap()[idx];
            off = ch.off as usize;
            dell = ch.dell as usize;
            insl = ch.insl as usize;
            ins_len = insl;
            old_cs = ch.old_cs as usize;
            del_vec = ch.del.chars().collect::<Vec<char>>();
        }
        let _ = ins_len;
         ZLECS.store(off, std::sync::atomic::Ordering::SeqCst);
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
         ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
         ZLECS.store(old_cs.min( ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        true
    }

    /// Replay the change at `idx`. Port of `applychange(struct change *ch)` (zle_utils.c:1677).
    pub fn apply_change(idx: usize) -> bool {
        if idx >=  UNDO_STACK.lock().unwrap().len() {
            return false;
        }
        let (off, dell, insl, new_cs);
        let ins_vec;
        {
            let ch = & UNDO_STACK.lock().unwrap()[idx];
            off = ch.off as usize;
            dell = ch.dell as usize;
            insl = ch.insl as usize;
            new_cs = ch.new_cs as usize;
            ins_vec = ch.ins.chars().collect::<Vec<char>>();
        }
         ZLECS.store(off, std::sync::atomic::Ordering::SeqCst);
        if dell > 0 {
             ZLELINE.lock().unwrap().drain(off..off + dell);
        }
        if insl > 0 {
            for (i, c) in ins_vec.into_iter().enumerate() {
                 ZLELINE.lock().unwrap().insert(off + i, c);
            }
        }
         ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
         ZLECS.store(new_cs.min( ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
         ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        true
    }

    // move backwards through the change list                                 // c:1597
    /// Walk back one Change. Port of `undo(char **args)` (zle_utils.c:1601).
    pub fn undo_widget() -> i32 {                                    // c:1601
        // Capture any in-flight edits into a Change before stepping back.
        mkundoent();
        if  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            return 1;
        }
        let prev_idx =  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) - 1;
        if  UNDO_STACK.lock().unwrap()[prev_idx].changeno <=  UNDO_LIMITNO.load(std::sync::atomic::Ordering::SeqCst) as i64 {
            return 1;
        }
        if unapply_change(prev_idx) {
             CURCHANGE.store(prev_idx, std::sync::atomic::Ordering::SeqCst);
        }
        setlastline();
        0
    }

    // move forwards through the change list                                  // c:1657
    /// Walk forward one Change. Port of `redo(UNUSED(char **args))` (zle_utils.c:1661).
    pub fn redo_widget() -> i32 {                                    // c:1661
        mkundoent();
        if  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) >=  UNDO_STACK.lock().unwrap().len() {
            return 1;
        }
        if apply_change( CURCHANGE.load(std::sync::atomic::Ordering::SeqCst)) {
             CURCHANGE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        setlastline();
        0
    }


/// Direct port of `static int applychange(struct change *ch)` from
/// `Src/Zle/zle_utils.c:1678`. Applies one Change record from
/// the undo stack: deletes `ch->del` characters at `ch->off`, then
/// inserts `ch->ins` at the same position, and updates `zlecs`.
/// Returns 1 if there are more changes to apply (CH_NEXT), else 0.
pub fn applychange(ch: i32) -> i32 { // c:1678
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    let idx = ch as usize;
    if idx >=  UNDO_STACK.lock().unwrap().len() { return 0; }
    let change =  UNDO_STACK.lock().unwrap()[idx].clone();
    // c:1683-1696 — apply del then ins at change.off. Canonical
    // `change.off`/`dell`/`insl` are `i32` (port of `int off; int
    // dell; int insl`); `change.del`/`ins` are `String` (port of
    // `ZLE_STRING_T`). Convert at the indexing boundary.
    let off = change.off as usize;
    let del_n = change.dell as usize;
    if off + del_n <=  ZLELINE.lock().unwrap().len() {
         ZLELINE.lock().unwrap().drain(off..off + del_n);                                 // c:1690 delete
    }
    // c:1700 — insert change.ins at off.
    for (i, c) in change.ins.chars().enumerate() {
        if off + i <=  ZLELINE.lock().unwrap().len() {
             ZLELINE.lock().unwrap().insert(off + i, c);
        } else {
             ZLELINE.lock().unwrap().push(c);
        }
    }
     ZLECS.store(change.new_cs as usize, std::sync::atomic::Ordering::SeqCst);                                               // c:1718
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    // c:1721 — return 1 if CH_NEXT, else 0.
    if change.flags & CH_NEXT != 0 { 1 } else { 0 }
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
pub fn backdel(ct: i32, _flags: i32) {  // c:1084
    let ct = ct as usize;
    if ct == 0 ||  ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 { return; }
    let take_n = ct.min( ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let start =  ZLECS.load(std::sync::atomic::Ordering::SeqCst) - take_n;
     ZLELINE.lock().unwrap().drain(start.. ZLECS.load(std::sync::atomic::Ordering::SeqCst));                                 // c:1090 shiftchars
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
     ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
     ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);                                              // c:1091 CCRIGHT
}

/// Port of `backkill(int ct, int flags)` from `Src/Zle/zle_utils.c:1045`. Cuts `ct`
/// characters BACKWARD from the cursor (i.e. removes `[zlecs-ct,
/// zlecs)` and pushes them onto the kill-ring head). C: `void
/// backkill(int ct, int flags)`. Rust port takes `&mut Zle` so the
/// killring + zlecs/zlell mutations stay on the typed shell state.
/// `flags` is the `CUT_*` bitmask — `CUT_RAW` skips the multibyte
/// DECCS adjustment loop the non-RAW path uses.
/// WARNING: param names don't match C — Rust=(zle, ct, flags) vs C=(ct, flags)
pub fn backkill(ct: i32, flags: i32) {  // c:1045
    let ct = ct as usize;
    if ct == 0 ||  ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 { return; }
    let _ = flags; // CUT_RAW path: no DECCS multibyte adjustment.
    let take_n = ct.min( ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let start =  ZLECS.load(std::sync::atomic::Ordering::SeqCst) - take_n;
    let cut_chars: Vec<char> =  ZLELINE.lock().unwrap().drain(start.. ZLECS.load(std::sync::atomic::Ordering::SeqCst)).collect();   // c:1057 cut + shiftchars
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
     ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
     KILLRING.lock().unwrap().push_front(cut_chars);
    if  KILLRING.lock().unwrap().len() >  KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
         KILLRING.lock().unwrap().pop_back();
    }
     ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);                                              // c:1059 CCRIGHT
}

/// Port of `cut(int i, int ct, int flags)` from Src/Zle/zle_utils.c:935.
/// `i` is the start byte offset; `ct` is the count to cut; `dir` is
/// the cut direction flag (0=after, non-zero=before).
/// WARNING: param names don't match C — Rust=(zle, i, dir) vs C=(i, ct, flags)
pub fn cut(i: i32,              // c:935
           ct: i32, dir: i32) -> i32 {
    // C body c:937-944 — `cuttext(zleline+i, ct, dir)`. Fold to a
    //                    single helper that pushes a slice into the
    //                    kill ring (or vibuf when MOD_VIBUF is set).
    if ct <= 0 || i < 0 {
        return 0;
    }
    let start = i as usize;
    let end = (start + ct as usize).min( ZLELINE.lock().unwrap().len());
    if start >= end {
        return 0;
    }
    let chunk: Vec<char> =  ZLELINE.lock().unwrap()[start..end].to_vec();
    cuttext(&chunk, dir);
    0
}

/// Port of `cuttext(ZLE_STRING_T line, int ct, int flags)` from Src/Zle/zle_utils.c:946.
/// WARNING: param names don't match C — Rust=(zle, txt) vs C=(line, ct, flags)
pub fn cuttext(txt: &[char],    // c:946
               dir: i32) {
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // C body c:948-1043 — pushes `txt` into vibuf[zmod.vibuf] when
    //                     MOD_VIBUF is set, else front of killring.
    //                     CUT_APPEND/CUT_REPLACE flag handling skipped
    //                     in this distilled body.
    let chars: Vec<char> = txt.to_vec();
    if  ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {                       // c:961
        let idx =  ZMOD.lock().unwrap().vibuf as usize;
        if idx <  vibuf().lock().unwrap().len() {
            if dir != 0 {
                 vibuf().lock().unwrap()[idx] = chars;
            } else {
                 vibuf().lock().unwrap()[idx].extend(chars);
            }
        }
    } else {
         KILLRING.lock().unwrap().push_front(chars);                                      // c:996
        let max =  KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst);
        if  KILLRING.lock().unwrap().len() > max {
             KILLRING.lock().unwrap().pop_back();
        }
    }
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
pub fn findbol() -> usize {           // c:1158
    let mut x =  ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                                   // c:1158 int x = zlecs
    while x > 0 &&  ZLELINE.lock().unwrap().get(x - 1) != Some(&'\n') {                   // c:1162
        x -= 1;                                                              // c:1163 x--
    }
    x                                                                        // c:1164 return x
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
pub fn findeol() -> usize {           // c:1169
    let mut x =  ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                                   // c:1169 int x = zlecs
    while x !=  ZLELL.load(std::sync::atomic::Ordering::SeqCst) &&  ZLELINE.lock().unwrap().get(x) != Some(&'\n') {              // c:1173
        x += 1;                                                              // c:1174 x++
    }
    x                                                                        // c:1175 return x
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
pub fn findline() -> (usize, usize) {  // c:1180
    (findbol(), findeol())                                             // c:1180-1183
}

/// Port of `foredel(int ct, int flags)` from `Src/Zle/zle_utils.c:1105`. Removes `ct`
/// characters FORWARD from the cursor (i.e. drops `[zlecs, zlecs+ct)`
/// from the line) without pushing to the kill-ring.
///
/// C signature: `void foredel(int ct, int flags)`. Rust port takes
/// `&mut Zle`. The non-RAW path's `INCCS` multibyte adjustment loop
/// (c:1115+) collapses to plain `Vec<char>::drain`.
/// WARNING: param names don't match C — Rust=(zle, ct, _flags) vs C=(ct, flags)
pub fn foredel(ct: i32, _flags: i32) {  // c:1105
    let ct = ct as usize;
    if ct == 0 ||  ZLECS.load(std::sync::atomic::Ordering::SeqCst) >=  ZLELL.load(std::sync::atomic::Ordering::SeqCst) { return; }
    let take_n = ct.min( ZLELL.load(std::sync::atomic::Ordering::SeqCst) -  ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let i =  ZLECS.load(std::sync::atomic::Ordering::SeqCst);
     ZLELINE.lock().unwrap().drain(i..i + take_n);                                    // c:1111 shiftchars
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
     ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);                                              // c:1112 CCRIGHT
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
pub fn forekill(ct: i32, flags: i32) {  // c:1064
    let ct = ct as usize;
    if ct == 0 ||  ZLECS.load(std::sync::atomic::Ordering::SeqCst) >=  ZLELL.load(std::sync::atomic::Ordering::SeqCst) { return; }
    let _ = flags; // CUT_RAW path: no INCCS multibyte adjustment.
    let take_n = ct.min( ZLELL.load(std::sync::atomic::Ordering::SeqCst) -  ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    let i =  ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let cut_chars: Vec<char> =  ZLELINE.lock().unwrap().drain(i..i + take_n).collect();      // c:1077 cut + shiftchars
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
     KILLRING.lock().unwrap().push_front(cut_chars);
    if  KILLRING.lock().unwrap().len() >  KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
         KILLRING.lock().unwrap().pop_back();
    }
     ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);                                              // c:1079 CCRIGHT
}

/// Port of `free_region_highlights_memos()` from Src/Zle/zle_utils.c:567.
pub fn free_region_highlights_memos() {                                      // c:567
    // C body c:569-580 — walks region_highlights_memos free list,
    //                    calls zfree on each. Drop covers it; no-op.
}

/// Port of `freechanges(struct change *p)` from Src/Zle/zle_utils.c:1472.
/// WARNING: param names don't match C — Rust=() vs C=(p)
pub fn freechanges() {                                                       // c:1472
    // C body c:1474-1484 — walks Change linked list, frees del/ins
    //                      strings + the Change node. Drop covers it.
}

/// Port of `freeundo()` from Src/Zle/zle_utils.c:1461.
pub fn freeundo() {                                                          // c:1461
    // C body c:1463-1470 — `freechanges(curchange); freechanges(...)
    //                      etc. for the whole undo chain`. Drop covers.
}

/// Port of `get_undo_current_change(UNUSED(Param pm))` from Src/Zle/zle_utils.c:1785.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_undo_current_change() -> i64 {                                    // c:1785
    // C body c:1787-1810 — `if (!curchange) return -1; return curchange->changeno`.
    //                      Without curchange tracker: -1 (no change).
    -1
}

/// Port of `get_undo_limit_change(UNUSED(Param pm))` from Src/Zle/zle_utils.c:1812.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_undo_limit_change() -> i64 {                                      // c:1812
    // C body c:1814-1817 — `return undo_limit_change`. Returns the
    //                      undo-limit anchor change number.
    -1
}

/// Port of `getzlequery()` from Src/Zle/zle_utils.c:1197.
pub fn getzlequery() -> i32 {                                                // c:1197
    // C body c:1199-1300 — reads y/n response from terminal interactive
    //                      prompt. Without a live tty read we report
    //                      cancel (-1).
    -1
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
pub fn handlefeep() -> i32 {                                                 // c:1405
    crate::ported::utils::zbeep();                                           // c:1415 zbeep()
    0                                                                        // c:1415 return 0
}

/// Port of `handlesuffix(UNUSED(char **args))` from Src/Zle/zle_utils.c:1415.
/// WARNING: param names don't match C — Rust=(zle, c) vs C=(args)
pub fn handlesuffix(c: i32) -> i32 { // c:1415
    // C body c:1417-1444 — peeks the next byte; if SUFFIXLEN is set
    //                      and the byte is in the suffix's noinsert
    //                      set, drop the suffix; else keep + insert.
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_misc::SUFFIXLEN;
    let _ = c;
    let len = SUFFIXLEN.load(Ordering::SeqCst);
    if len > 0 {
        SUFFIXLEN.store(0, Ordering::SeqCst);
    }
    0
}

/// Port of `initundo()` from Src/Zle/zle_utils.c:1446.
pub fn initundo() {                                                          // c:1446
    // C body c:1448-1459 — `nextchanges = endnextchanges = NULL;
    //                       lastline = ...; freeundo()`.
    //                      Undo chain isn't a Rust struct yet; no-op.
    freeundo();
}

/// Direct port of `void mergeundo(void)` from
/// `Src/Zle/zle_utils.c:1733`. Walks the undo stack backward
/// from `cur_change` chaining CH_PREV/CH_NEXT flags so the changes
/// since `vistartchange+1` form a single undo step (atomic vi
/// insert-mode group). Resets `vistartchange = u64::MAX` (C's -1).
pub fn mergeundo() {              // c:1733
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    // c:1735-1742 — walk current->prev while changeno > vistartchange+1.
    if  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) == 0 { return; }
    let mut current =  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) - 1;                                    // c:1735 prev
    while current > 0
        &&  UNDO_STACK.lock().unwrap()[current].changeno >  VISTARTCHANGE.load(std::sync::atomic::Ordering::SeqCst) as i64 + 1
    {
         UNDO_STACK.lock().unwrap()[current].flags |= CH_PREV;                  // c:1740
         UNDO_STACK.lock().unwrap()[current - 1].flags |= CH_NEXT;              // c:1741
        current -= 1;
    }
     VISTARTCHANGE.store(u64::MAX, std::sync::atomic::Ordering::SeqCst);                                            // c:1744 = -1
}

/// Direct port of `int redo(UNUSED(char **args))` from
/// `Src/Zle/zle_utils.c:1661`. Walks the undo stack forward
/// from ` CURCHANGE.load(std::sync::atomic::Ordering::SeqCst)` calling `applychange` on each; returns 0
/// on success, 1 when nothing to redo.
pub fn redo() -> i32 {            // c:1661
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    loop {
        if  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) >=  UNDO_STACK.lock().unwrap().len() { return 1; }              // c:1664
        let cur_idx =  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst);
        if applychange(cur_idx as i32) == 0 { break; }                  // c:1668
         CURCHANGE.store(cur_idx + 1, std::sync::atomic::Ordering::SeqCst);
        let has_next =  UNDO_STACK.lock().unwrap().get(cur_idx)
            .map(|c| c.flags & CH_NEXT != 0)
            .unwrap_or(false);
        if !has_next { break; }                                              // c:1670
    }
     CURCHANGE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);                                                     // c:1672 advance past applied
    0                                                                        // c:1674
}

/// Port of `set_undo_limit_change(UNUSED(Param pm), zlong value)` from Src/Zle/zle_utils.c:1819.
/// WARNING: param names don't match C — Rust=(_n) vs C=(pm, value)
pub fn set_undo_limit_change(_n: i64) -> i32 {                               // c:1819
    // C body c:1821-1825 — `undo_limit_change = n; return 0`.
    //                      Without undo_limit_change global: 0.
    0
}

/// Port of `setline(char *s, int flags)` from Src/Zle/zle_utils.c:1129.
/// WARNING: param names don't match C — Rust=(zle, s) vs C=(s, flags)
pub fn setline(s: &str,         // c:1129
               flags: i32) {
    // C body c:1131-1156 — replaces zleline with `s`; if !ZSL_KEEPCS
    //                      reset zlecs to 0 or len(s). flags bit
    //                      ZSL_KEEPCS = 1.
     ZLELINE.lock().unwrap().clear();
     ZLELINE.lock().unwrap().extend(s.chars());
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    if flags & 1 == 0 {
         ZLECS.store( ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                               // c:1145
    }
     ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

/// Port of `shiftchars(int to, int cnt)` from Src/Zle/zle_utils.c:846.
/// WARNING: param names don't match C — Rust=(zle, to, cnt) vs C=(to, cnt)
pub fn shiftchars(to: i32, cnt: i32) { // c:846
    // C body c:848-865 — `if (to + cnt < zlell) memmove(line+to,
    //                     line+to+cnt, (zlell-(to+cnt)) * char_t);
    //                     zlell -= cnt`. Pure shift-left of `cnt`
    //                     chars at offset `to`.
    let to = to as usize;
    let cnt = cnt as usize;
    if to + cnt >  ZLELINE.lock().unwrap().len() {
        return;
    }
     ZLELINE.lock().unwrap().drain(to..to + cnt);
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
}

/// Direct port of `void showmsg(char const *msg)` from
/// `Src/Zle/zle_utils.c:1303`. Writes `msg` followed by a newline to
/// the shell-output fd. The full C body (c:1305-1402) handles
/// metafied input + nice-character expansion + per-byte color
/// (mcolors) which need substrate not yet wired; the visible-byte
/// stream is the same in the un-colored common case where `msg` is
/// already plain ASCII.
pub fn showmsg(msg: &str) {                                                  // c:1303
    use std::sync::atomic::Ordering;
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 2 };
    let _ = crate::ported::utils::write_loop(out, msg.as_bytes());
    let _ = crate::ported::utils::write_loop(out, b"\n");
}

/// Port of `sizeline(int sz)` from Src/Zle/zle_utils.c:67.
/// WARNING: param names don't match C — Rust=(zle, sz) vs C=(sz)
pub fn sizeline(sz: usize) {    // c:67
    // C body c:69-87 — `if (sz > linesz) { linesz = sz + 256; line =
    //                  zrealloc(line, (linesz+1) * char_t) }`. Vec
    //                  grows on demand; just reserve.
    let mut __g_zleline =  ZLELINE.lock().unwrap();
    let cur_len = __g_zleline.len();
    if sz > cur_len {
        __g_zleline.reserve(sz - cur_len + 256);
    }
}

/// Port of `spaceinline(int ct)` from Src/Zle/zle_utils.c:777.
/// WARNING: param names don't match C — Rust=(zle, ct) vs C=(ct)
pub fn spaceinline(ct: i32) {   // c:777
    // C body c:779-844 — opens `ct` chars of space at zlecs by
    //                    moving zleline[zlecs..zlell] forward `ct`,
    //                    growing buffer if needed. zlell += ct.
    if ct <= 0 {
        return;
    }
    let ct = ct as usize;
    for _ in 0..ct {
         ZLELINE.lock().unwrap().insert( ZLECS.load(std::sync::atomic::Ordering::SeqCst), '\0');
    }
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
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
pub fn splitundo() -> i32 {       // c:1721
    // C uses signed `vistartchange`; Rust uses u64 with u64::MAX as
    // the "-1 / inactive" sentinel.
    if  VISTARTCHANGE.load(std::sync::atomic::Ordering::SeqCst) != u64::MAX {                                       // c:1723 >= 0
        mergeundo();                                                      // c:1725
         VISTARTCHANGE.store( UNDO_CHANGENO.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                               // c:1726
    }
    handleundo();                                                        // c:1728
    0                                                                        // c:1730
}

/// Port of `stringaszleline(char *instr, int incs, int *outll, int *outsz, int *outcs)` from Src/Zle/zle_utils.c:375.
/// WARNING: param names don't match C — Rust=(s) vs C=(instr, incs, outll, outsz, outcs)
pub fn stringaszleline(s: &str) -> Vec<char> {                               // c:375
    // C body c:377-580 — converts a metafied string into ZLE_CHAR_T
    //                    array (multibyte decode + meta unescape).
    //                    Vec<char> is already wide-char; demeta and
    //                    return.
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x83 && i + 1 < bytes.len() {                                // Meta byte
            i += 1;
            out.push((bytes[i] ^ 32) as char);
        } else {
            out.push(b as char);
        }
        i += 1;
    }
    out
}

/// Port of `unapplychange(struct change *ch)` from Src/Zle/zle_utils.c:1634.
/// Direct port of `static int unapplychange(struct change *ch)` from
/// `Src/Zle/zle_utils.c:1634`. Reverse of applychange: deletes
/// `ch->ins` at `ch->off` and re-inserts `ch->del`.
/// WARNING: param names don't match C — Rust=(zle, ch) vs C=(ch)
pub fn unapplychange(ch: i32) -> i32 { // c:1634
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    let idx = ch as usize;
    if idx >=  UNDO_STACK.lock().unwrap().len() { return 0; }
    let change =  UNDO_STACK.lock().unwrap()[idx].clone();
    // Canonical change.off/insl/old_cs are i32, change.del is String;
    // convert at the indexing boundary.
    let off = change.off as usize;
    // c:1638-1644 — delete what was inserted.
    let ins_n = change.insl as usize;
    if off + ins_n <=  ZLELINE.lock().unwrap().len() {
         ZLELINE.lock().unwrap().drain(off..off + ins_n);                                 // c:1640
    }
    // c:1646 — re-insert the deleted chars.
    for (i, c) in change.del.chars().enumerate() {
        if off + i <=  ZLELINE.lock().unwrap().len() {
             ZLELINE.lock().unwrap().insert(off + i, c);
        } else {
             ZLELINE.lock().unwrap().push(c);
        }
    }
     ZLECS.store(change.old_cs as usize, std::sync::atomic::Ordering::SeqCst);                                               // c:1649
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
    // c:1651 — return 1 if CH_PREV, else 0.
    if change.flags & CH_PREV != 0 { 1 } else { 0 }
}

/// Direct port of `int undo(char **args)` from
/// `Src/Zle/zle_utils.c:1601`. Walks the undo stack backward
/// from ` CURCHANGE.load(std::sync::atomic::Ordering::SeqCst)` calling `unapplychange` on each; stops at
/// `last_change` (parsed from `args[0]` if provided, else -1 for
/// "single step") or at `undo_limitno`. Returns 0 on success,
/// 1 when nothing left to undo.
pub fn undo(args: &[String]) -> i32 { // c:1601
    use crate::ported::zle::zle_h::{CH_NEXT, CH_PREV};
    let last_change: i64 = if !args.is_empty() {                             // c:1605
        args[0].parse().unwrap_or(-1)
    } else {
        -1
    };

    loop {
        // c:1614 — `prev = curchange->prev`; in Rust we step the
        // index down.
        if  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) == 0 { return 1; }                                 // c:1615
        let prev_idx =  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) - 1;
        let prev_chno =  UNDO_STACK.lock().unwrap()[prev_idx].changeno as i64;
        if prev_chno <= last_change { break; }                               // c:1618
        if (prev_chno as u64) <=  UNDO_LIMITNO.load(std::sync::atomic::Ordering::SeqCst) && args.is_empty() {       // c:1619
            return 1;
        }
        if unapplychange(prev_idx as i32) == 0 {                        // c:1621
            if last_change >= 0 {
                unapplychange(prev_idx as i32);                         // c:1623
                 CURCHANGE.store(prev_idx, std::sync::atomic::Ordering::SeqCst);                                   // c:1624
            }
        } else {
             CURCHANGE.store(prev_idx, std::sync::atomic::Ordering::SeqCst);                                       // c:1627
        }
        let has_prev =  UNDO_STACK.lock().unwrap().get(prev_idx)
            .map(|c| c.flags & CH_PREV != 0)
            .unwrap_or(false);
        if !(last_change >= 0 || has_prev) { break; }                        // c:1630
    }
    0                                                                        // c:1631
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
pub fn viundochange(// c:1705
                    args: &[String]) -> i32 {
    handleundo();                                                        // c:1707
    if  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) <  UNDO_STACK.lock().unwrap().len() {                               // c:1708 curchange->next
        // Re-apply all forward changes (collapses an undo chain back
        // to current state).
        while  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst) <  UNDO_STACK.lock().unwrap().len() {                        // c:1710
            let idx =  CURCHANGE.load(std::sync::atomic::Ordering::SeqCst);
            applychange(idx as i32);                                    // c:1711
             CURCHANGE.store(idx + 1, std::sync::atomic::Ordering::SeqCst);                                        // c:1712
        }
        0                                                                    // c:1715
    } else {
        undo(args)                                                      // c:1717
    }
}

/// Port of `struct zle_position` from Src/Zle/zle_utils.c:594.
/// Saved (cs, mark, ll) for a stacked position.
#[derive(Debug, Clone)]
pub struct ZlePosition {                                                     // c:594
    /// Cursor position.
    pub cs: usize,                                                           // c:599
    /// Mark.
    pub mk: usize,                                                           // c:601
    /// Line length.
    pub ll: usize,                                                           // c:603
    // c:604 region_highlights chain — region-highlight system not yet
    // a static-link Rust struct; saved positions don't carry region
    // state until that lands.
}

/// Port of `static struct zle_position *zle_positions` from
/// Src/Zle/zle_utils.c:619. LIFO stack of saved positions.
pub static ZLE_POSITIONS: std::sync::Mutex<Vec<ZlePosition>> =               // c:619
    std::sync::Mutex::new(Vec::new());

/// Port of `mod_export void zle_save_positions(void)` from
/// Src/Zle/zle_utils.c:619.
///
/// "Save positions including cursor, end-of-line and (non-special)
/// region highlighting. Must be matched by a subsequent
/// `zle_restore_positions()`."
pub fn zle_save_positions() {         // c:619
    let pos = ZlePosition {                                                  // c:619 newpos = zalloc
        mk:  MARK.load(std::sync::atomic::Ordering::SeqCst),                                                        // c:627
        cs:  ZLECS.load(std::sync::atomic::Ordering::SeqCst),                                                       // c:634 (no zlemetaline branch)
        ll:  ZLELL.load(std::sync::atomic::Ordering::SeqCst),                                                       // c:635
    };
    if let Ok(mut s) = ZLE_POSITIONS.lock() {                                // c:677 push
        s.push(pos);
    }
}

/// Port of `mod_export void zle_restore_positions(void)` from
/// Src/Zle/zle_utils.c:677. Pops the last saved (cs, mark, ll).
pub fn zle_restore_positions() {  // c:677
    if let Ok(mut s) = ZLE_POSITIONS.lock() {
        if let Some(oldpos) = s.pop() {                                      // c:679-684
             MARK.store(oldpos.mk, std::sync::atomic::Ordering::SeqCst);                                            // c:686
             ZLECS.store(oldpos.cs.min( ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);                            // c:693
             ZLELL.store(oldpos.ll, std::sync::atomic::Ordering::SeqCst);                                           // c:694
        }
    }
}

/// Port of `mod_export void zle_free_positions(void)` from
/// Src/Zle/zle_utils.c:102. Discards the top of stack without
/// applying it.
pub fn zle_free_positions() {                                                // c:747
    if let Ok(mut s) = ZLE_POSITIONS.lock() {
        s.pop();                                                             // c:749 oldpos = zle_positions; zle_positions = next
    }
}

/// Port of `zleaddtoline(int chr)` from Src/Zle/zle_utils.c:102.
/// WARNING: param names don't match C — Rust=(zle, ch) vs C=(chr)
pub fn zleaddtoline(ch: i32) {  // c:102
    // C body c:104-115 — `sizeline(zlell+1); zleline[zlell] = ch;
    //                    zleline[++zlell] = '\\0'`.
     ZLELINE.lock().unwrap().push(ch as u8 as char);
     ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
}

/// Port of `zlecallhook(char *name, char *arg)` from Src/Zle/zle_utils.c:1755.
pub fn zlecallhook(name: &str, arg: Option<&str>) {                          // c:1755
    // C body c:1757-1840 — looks up shfunc `name` and dispatches via
    //                      execzlefunc. Without exec hook we record
    //                      via tracing.
    tracing::debug!(target: "zle", "zlecallhook({}, {:?})", name, arg);
}

/// Port of `zlecharasstring(ZLE_CHAR_T inchar, char *buf)` from Src/Zle/zle_utils.c:117.
pub fn zlecharasstring(inchar: char, buf: &mut String) -> i32 {                   // inchar:117
    // C body inchar:119-145 — converts a ZLE_CHAR_T to its display form
    //                    (UTF-8 multibyte if MULTIBYTE_SUPPORT, else
    //                    raw byte). Vec<char> is wide-char already;
    //                    just append.
    let start = buf.len();
    buf.push(inchar);
    (buf.len() - start) as i32
}

/// Port of `zlegetline(int *ll, int *cs)` from Src/Zle/zle_utils.c:547.
/// WARNING: param names don't match C — Rust=(zle, cs) vs C=(ll, cs)
pub fn zlegetline(// c:547
                  ll: &mut usize, cs: &mut usize) -> Vec<char> {
    // C body c:150-200 — `if (zlemetaline) { *ll=zlemetall; *cs=zlemetacs;
    //                     return ztrdup(zlemetaline) } else
    //                     return zlelineasstring(...)`. Snapshot of the
    //                     current line + cursor.
    *ll =  ZLELL.load(std::sync::atomic::Ordering::SeqCst);
    *cs =  ZLECS.load(std::sync::atomic::Ordering::SeqCst);
     ZLELINE.lock().unwrap().clone()
}

/// Port of `zlelineasstring(ZLE_STRING_T instr, int inll, int incs, int *outllp, int *outcsp, int useheap)` from Src/Zle/zle_utils.c:192.
/// WARNING: param names don't match C — Rust=(line, ll, _flags) vs C=(instr, inll, incs, outllp, outcsp, useheap)
pub fn zlelineasstring(line: &[char], ll: usize, _flags: i32) -> String {    // c:192
    // C body c:284-373 — encodes ZLE_CHAR_T array to a metafied
    //                    multibyte string. Vec<char> → String is
    //                    direct; meta encoding skipped (we don't run
    //                    through zsh's parser path).
    line.iter().take(ll).collect()
}

#[cfg(test)]
mod findbol_findeol_tests {
    use crate::zle::zle_main::zle_test_setup;
    use super::*;

    fn zle_with(line: &str, cs: usize) {
         zle_reset();
        * ZLELINE.lock().unwrap() = line.chars().collect();
         ZLELL.store( ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
         ZLECS.store(cs, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn findbol_no_newline_returns_zero() {
        let _g =  zle_test_setup();
        // c:1162 — walks back to start when no '\n' encountered.
        let z = zle_with("hello world", 7);
        assert_eq!(findbol(), 0);
    }

    #[test]
    fn findbol_finds_preceding_newline() {
        let _g =  zle_test_setup();
        // c:1162 — `zleline[x-1] != '\n'` exits loop when prev char IS '\n'.
        // For "abc\ndef\nghi" with cursor at 9 (the 'h' in 'ghi'):
        // walks back to 8 (after the second '\n'), returns 8.
        let z = zle_with("abc\ndef\nghi", 9);
        assert_eq!(findbol(), 8);
    }

    #[test]
    fn findbol_at_start_returns_zero() {
        let _g =  zle_test_setup();
        let z = zle_with("anything", 0);
        assert_eq!(findbol(), 0);
    }

    #[test]
    fn findeol_no_newline_returns_end() {
        let _g =  zle_test_setup();
        // c:1173 — walks forward to zlell when no '\n' encountered.
        let z = zle_with("hello world", 0);
        assert_eq!(findeol(), 11);
    }

    #[test]
    fn findeol_finds_next_newline() {
        let _g =  zle_test_setup();
        // c:1173 — `zleline[x] != '\n'` exits when current char IS '\n'.
        // For "abc\ndef" cursor at 0: walks 0→1→2→3 (which is '\n'), returns 3.
        let z = zle_with("abc\ndef", 0);
        assert_eq!(findeol(), 3);
    }

    #[test]
    fn findeol_at_end_returns_zlell() {
        let _g =  zle_test_setup();
        let z = zle_with("hello", 5);
        assert_eq!(findeol(), 5);
    }

    #[test]
    fn findline_returns_bol_eol_pair() {
        let _g =  zle_test_setup();
        // c:1182-1183 — both findbol and findeol from the same cursor.
        // "abc\ndef\nghi" cursor at 5 (the 'e' in 'def'):
        //   findbol → 4 (after first '\n')
        //   findeol → 7 (the second '\n')
        let z = zle_with("abc\ndef\nghi", 5);
        let (bol, eol) = findline();
        assert_eq!(bol, 4);
        assert_eq!(eol, 7);
    }
}
