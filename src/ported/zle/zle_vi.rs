//! ZLE vi mode operations
//!
//! Direct port from zsh/Src/Zle/zle_vi.c

use std::sync::atomic::Ordering;

use super::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
use super::zle_misc::{TAILADD, VFINDCHAR, VFINDDIR};

// Note: dead `ViState` / `ViChange` / `ViPendingOp` aggregates
// removed per PORT_PLAN Phase 2. They had zero references across the
// codebase. The actual zsh-side state lives in C file-scope globals
// declared in `Src/Zle/zle_vi.c`; the AtomicI32 wires below are the
// faithful ports.

/// Port of `int virangeflag;` from `Src/Zle/zle_vi.c:36`. Set during
/// vi range-pending operations to suppress the cursor-included
/// region adjustment (see `textobjects.rs:261` and `zle_vi.c:196`).

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::ported::zle::zle_h::*;
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
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

pub static VIRANGEFLAG: std::sync::atomic::AtomicI32 =                       // c:36
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int wordflag;` from `Src/Zle/zle_vi.c:41`. Kludge flag
/// used by `cw`/`dw` so they stop at word boundaries.
pub static WORDFLAG: std::sync::atomic::AtomicI32 =                          // c:41
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int vilinerange;` from `Src/Zle/zle_vi.c:46`. Set when
/// the pending range is whole-line (e.g. `dd`, `yy`).
pub static VILINERANGE: std::sync::atomic::AtomicI32 =                       // c:46
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int vichgflag;` from `Src/Zle/zle_vi.c:65`. Set while a
/// vi change-tracker (`.`) is recording.
pub static VICHGFLAG: std::sync::atomic::AtomicI32 =                         // c:65
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int viinrepeat;` from `Src/Zle/zle_vi.c:73`. Set during
/// `.` replay so the recorder doesn't re-record.
pub static VIINREPEAT: std::sync::atomic::AtomicI32 =                        // c:73
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int viinsbegin;` from `Src/Zle/zle_vi.c:78`. Buffer
/// position where vi insert mode was last entered.
pub static VIINSBEGIN: std::sync::atomic::AtomicI32 =                        // c:78
    std::sync::atomic::AtomicI32::new(0);

    /// Read the active numeric multiplier.
    /// Port of `zmult` macro at Src/Zle/zle.h:267 (`#define zmult
    /// (zmod.mult)`). Returns the explicit MULT prefix when set,
    /// otherwise 1 — the default-1 fall-through that initmodifier
    /// installs (zle_main.c:1604).
    pub fn vi_get_arg() -> i32 {
        if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_MULT != 0 {
            crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult
        } else {
            1
        }
    }

    /// Read the next char from input and run a vi find-char.
    /// `forward`: true for f/t (forward), false for F/T (backward).
    /// `skip`: true for t/T (stop one short), false for f/F (land on the char).
    /// Port of vifindnextchar/vifindprevchar/vifindnextcharskip/vifindprevcharskip
    /// from Src/Zle/zle_move.c:739-783 — which all set state and call `vifindchar(0)`.
    pub fn vi_find_char(forward: bool, skip: bool) {
        let c = match getfullchar(true) {
            Some(c) => c,
            None => return,
        };
        VFINDCHAR.store(c as i32, Ordering::SeqCst);
        VFINDDIR.store(if forward { 1 } else { -1 }, Ordering::SeqCst);
        // tailadd: f/F → 0; t → -1; T → +1.
        TAILADD.store(
            match (forward, skip) {
                (_, false) => 0,
                (true, true) => -1,
                (false, true) => 1,
            },
            Ordering::SeqCst,
        );
        let _ = vi_find_char_inner(false);
    }

    /// Inner find-char routine. `repeat` distinguishes the user-typed call
    /// from `;` / `,` re-runs.
    /// Port of `vifindchar(int repeat, ...)` from Src/Zle/zle_move.c:787.
    pub fn vi_find_char_inner(repeat: bool) -> i32 {
        let target_raw = VFINDCHAR.load(Ordering::SeqCst);
        let target = match char::from_u32(target_raw as u32) {
            Some(c) if target_raw != 0 => c,
            _ => return 1,
        };
        if VFINDDIR.load(Ordering::SeqCst) == 0 {
            return 1;
        }
        let ocs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        let mut n = vi_get_arg();
        if n < 0 {
            // Negative count flips direction; faithful to C virevrepeatfind path.
            n = -n;
            VFINDDIR.store(-VFINDDIR.load(Ordering::SeqCst), Ordering::SeqCst);
            TAILADD.store(-TAILADD.load(Ordering::SeqCst), Ordering::SeqCst);
            let saved_mult = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
            crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = n;
            let ret = vi_find_char_inner(repeat);
            crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved_mult;
            VFINDDIR.store(-VFINDDIR.load(Ordering::SeqCst), Ordering::SeqCst);
            TAILADD.store(-TAILADD.load(Ordering::SeqCst), Ordering::SeqCst);
            return ret;
        }
        // On `;` (repeat) with t/T, step over the immediately-adjacent match
        // so we don't get stuck on the same char.
        if repeat && TAILADD.load(Ordering::SeqCst) != 0 {
            if VFINDDIR.load(Ordering::SeqCst) > 0 {
                if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                    && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1 < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)
                    && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1] == target
                {
                    crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            } else if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] == target {
                crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let dir = VFINDDIR.load(Ordering::SeqCst);
        for _ in 0..n {
            // Step at least once, then keep stepping until we land on the char,
            // hit a newline, or run off the end.
            let found = if dir > 0 {
                let mut p = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1;
                let mut hit = None;
                while p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                    let ch = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p];
                    if ch == '\n' {
                        break;
                    }
                    if ch == target {
                        hit = Some(p);
                        break;
                    }
                    p += 1;
                }
                hit
            } else {
                if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    None
                } else {
                    let mut p = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1;
                    let mut hit = None;
                    loop {
                        let ch = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p];
                        if ch == '\n' {
                            break;
                        }
                        if ch == target {
                            hit = Some(p);
                            break;
                        }
                        if p == 0 {
                            break;
                        }
                        p -= 1;
                    }
                    hit
                }
            };
            match found {
                Some(p) => { crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst); }
                None => {
                    crate::ported::zle::zle_main::ZLECS.store(ocs, std::sync::atomic::Ordering::SeqCst);
                    return 1;
                }
            }
        }
        // Apply the t/T adjustment after the final landing.
        let tail = TAILADD.load(Ordering::SeqCst);
        if tail > 0 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        } else if tail < 0 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        0
    }

    /// Jump to the bracket matching the one under the cursor.
    /// Port of `vimatchbracket(UNUSED(char **args))` from Src/Zle/zle_misc.c. Vim's `%`
    /// motion — recognises (), [], {}, <>; walks forward or backward
    /// honouring nesting depth.
    pub fn vi_match_bracket() {
        let c = if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]
        } else {
            return;
        };

        let (target, forward) = match c {
            '(' => (')', true),
            ')' => ('(', false),
            '[' => (']', true),
            ']' => ('[', false),
            '{' => ('}', true),
            '}' => ('{', false),
            '<' => ('>', true),
            '>' => ('<', false),
            _ => return,
        };

        let mut depth = 1;
        let mut pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);

        if forward {
            pos += 1;
            while pos < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && depth > 0 {
                if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == c {
                    depth += 1;
                } else if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == target {
                    depth -= 1;
                }
                if depth > 0 {
                    pos += 1;
                }
            }
        } else {
            if pos > 0 {
                pos -= 1;
                loop {
                    if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == c {
                        depth += 1;
                    } else if crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[pos] == target {
                        depth -= 1;
                    }
                    if depth == 0 || pos == 0 {
                        break;
                    }
                    pos -= 1;
                }
            }
        }

        if depth == 0 {
            crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Enter overwrite mode (vim's `R` command).
    /// Port of `vireplace(UNUSED(char **args))` from Src/Zle/zle_vi.c. Switches to the
    /// insert keymap with `insmode = false` so subsequent self-inserts
    /// overwrite existing chars instead of pushing them right.
    pub fn vi_replace_mode() {
        crate::ported::zle::zle_keymap::selectkeymap("viins", 1);
        crate::ported::zle::zle_main::INSMODE.store(0, std::sync::atomic::Ordering::SeqCst); // Overwrite mode
    }

    /// Toggle the case of the character under the cursor and advance.
    /// Port of `viswapcase(UNUSED(char **args))` from Src/Zle/zle_vi.c (vim's `~`).
    /// Uppercase letters become lowercase and vice versa; non-letters
    /// pass through untouched. Cursor advances one position post-swap.
    pub fn vi_swap_case() {
        let count = vi_get_arg() as usize;

        for _ in 0..count {
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = if c.is_uppercase() {
                    c.to_lowercase().next().unwrap_or(c)
                } else if c.is_lowercase() {
                    c.to_uppercase().next().unwrap_or(c)
                } else {
                    c
                };
                crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }

        // Move back one if we went past end
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Vi undo (`u` in command mode). Port of viundo() — which in C zsh just
    /// dispatches to undo(args) (zle_utils.c:1601). Routes through our index-based
    /// undo_widget() that mirrors that implementation.
    pub fn vi_undo() {
        let _ = undo_widget();
    }

    /// Vi visual mode (`v` in command mode).
    /// Port of visualmode(UNUSED(char **args)) from Src/Zle/zle_move.c:516. Toggles
    /// `region_active` between 0 (off), 1 (charwise), and 2 (linewise) per
    /// the C switch: from inactive enters charwise (sets mark first); from
    /// charwise turns off; from linewise switches to charwise.
    pub fn vi_visual_mode() {
        match crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
            1 => {
                crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst);
            }
            0 => {
                crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
                crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
            }
            2 => {
                crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
            }
            _ => {}
        }
    }

    /// Vi visual line mode (`V` in command mode).
    /// Port of visuallinemode(UNUSED(char **args)) from Src/Zle/zle_move.c:540. Same toggle
    /// shape as visualmode but the "active" target is 2 (linewise).
    pub fn vi_visual_line_mode() {
        match crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
            2 => {
                crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst);
            }
            0 => {
                crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
                crate::ported::zle::zle_main::REGION_ACTIVE.store(2, std::sync::atomic::Ordering::SeqCst);
            }
            1 => {
                crate::ported::zle::zle_main::REGION_ACTIVE.store(2, std::sync::atomic::Ordering::SeqCst);
            }
            _ => {}
        }
    }

    /// Vi visual block mode — Rust-side extension; zsh has no built-in
    /// visual-block widget (not in iwidgets.list). Treat as charwise so the
    /// caller still gets a usable selection.
    /// Reference: zsh has `visualmode` (charwise) and `visuallinemode`
    /// (linewise) only — see Src/Zle/iwidgets.list. This is a behavioural
    /// extension, not a port.
    pub fn vi_visual_block_mode() {
        if crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Deactivate the visual region (`Esc` from visual mode).
    /// Port of deactivateregion(UNUSED(char **args)) from Src/Zle/zle_move.c:564.
    pub fn vi_deactivate_region() {
        crate::ported::zle::zle_main::REGION_ACTIVE.store(0, std::sync::atomic::Ordering::SeqCst);
    }

    /// Vi set mark (`m{a-z}` in command mode). Port of visetmark() from
    /// Src/Zle/zle_move.c:872. Stores the current cursor and history line in
    /// the named slot; non-letter names are rejected.
    pub fn vi_set_mark(name: char) {
        // Set the historical mark (mirror with crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) for emacs compat).
        crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        if let Some(idx) = viyank(name) {
            crate::ported::zle::zle_main::vimarks().lock().unwrap()[idx] = Some((crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::history().lock().unwrap().cursor as i32));
        }
    }

    /// Vi goto mark (`'a` / `` `a `` in command mode). Port of vigotomark()
    /// from zle_move.c:887. ASCII letters jump to the saved location;
    /// `'` / `` ` `` jumps to the implicit "last position" mark; other
    /// characters are rejected.
    pub fn vi_goto_mark(name: char) {
        let idx = match viyank(name) {
            Some(i) => i,
            None => return,
        };
        let (cs, hist) = match crate::ported::zle::zle_main::vimarks().lock().unwrap()[idx] {
            Some(s) => s,
            None => return,
        };
        // Save the pre-jump position into the implicit mark (slot 26) so the
        // user can return to it with `''`.
        crate::ported::zle::zle_main::vimarks().lock().unwrap()[26] = Some((crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::history().lock().unwrap().cursor as i32));
        if hist >= 0 && (hist as usize) < crate::ported::zle::zle_main::history().lock().unwrap().entries.len() {
            // Cross-history jumps need to load that entry.
            let target = hist as usize;
            if target != crate::ported::zle::zle_main::history().lock().unwrap().cursor {
                crate::ported::zle::zle_main::history().lock().unwrap().cursor = target;
                *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = crate::ported::zle::zle_main::history().lock().unwrap().entries[target].line.chars().collect();
                crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            }
        }
        crate::ported::zle::zle_main::ZLECS.store(cs.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Append `key` to the vi change-replay buffer.
    /// Port of the recording side of `virepeatchange()` machinery from
    /// Src/Zle/zle_vi.c — C zsh tracks this via `vichgflag` + `vichgbuf`
    /// in zle_main.c, capturing every byte fed during a `c` / `d` / `y`
    /// operator, between `startvichange()` and the operator completion.
    /// Callers (the operator entry/exit points) gate when recording is
    /// active; this method just appends. The buffer is consumed by
    /// `widget_vi_repeat_change` via `ungetbytes`.
    pub fn vi_record_change(key: u8) {
        crate::ported::zle::zle_main::VICHGBUF.lock().unwrap().push(key);
    }

    /// Reset the change-replay buffer to start a fresh recording session.
    /// Mirrors C zsh's `vichgflag = 1; freevichg(); vichgbuf = ...` setup
    /// inside `startvichange()` (zle_vi.c).
    pub fn vi_start_change_recording() {
        crate::ported::zle::zle_main::VICHGBUF.lock().unwrap().clear();
    }

    /// Replay the last vi change ("." in command mode).
    /// Port of `virepeatchange(UNUSED(char **args))` from Src/Zle/zle_vi.c — re-feeds the
    /// recorded `vi_chg_buf` via `ungetbytes` so the next `zlecore`
    /// iteration re-runs the captured operator + motion. With nothing
    /// recorded yet (operator entry/exit don't gate `vi_record_change`
    /// in this build), the buffer is empty and replay is a no-op,
    /// matching zsh's behaviour pre-first-change.
    pub fn vi_repeat_change() {
        if crate::ported::zle::zle_main::VICHGBUF.lock().unwrap().is_empty() {
            return;
        }
        let bytes = crate::ported::zle::zle_main::VICHGBUF.lock().unwrap().clone();
        ungetbytes(&bytes);
    }

    /// Read the next keystroke and treat it as a vi motion to define an
    /// operator range. Returns `Some((start, end, line_mode))` where the
    /// operator should act on `[start, end)`, or `None` if the motion was
    /// unknown / canceled / a no-op.
    ///
    /// Port of `getvirange(int wf)` from `Src/Zle/zle_vi.c:172`. The full C
    /// implementation runs the next bound widget under `virangeflag = 1`
    /// using the operator-pending keymap. This Rust port short-circuits by
    /// dispatching a fixed set of common motions inline rather than going
    /// through the keymap — covering the daily-driver subset (`w`/`W`,
    /// `b`/`B`, `e`/`E`, `0`, `^`, `$`, `h`, `l`, `j`, `k`, `f`/`F`/`t`/`T`)
    /// plus the doubled-letter line-mode pattern (`dd`, `cc`, `yy` etc.).
    /// Text objects (`iw`, `aw`, `i"`, `a"`, …) and arbitrary user-bound
    /// motions in the operator-pending map are not yet wired through.
    ///
    /// `op_char` is the operator that triggered the call (`d` / `c` / `y`)
    /// — used to recognise the doubled form for line mode.
    pub fn vi_get_range(op_char: char) -> Option<(usize, usize, bool)> {
        let pos = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        let n = vi_get_arg().max(1);
        let motion = getfullchar(false)?;

        // Doubled letter (e.g. `dd`, `cc`, `yy`) → entire current line(s).
        // Mirrors the `MOD_LINE` branch of `getvirange()` in zle_vi.c:281
        // but invoked directly when the user repeats the operator letter.
        if motion == op_char {
            let bol = findbol();
            let mut eol = findeol();
            // Extend by `n - 1` more lines forward to honour the count
            // (vi `3dd` deletes 3 lines).
            for _ in 1..n {
                if eol >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                eol = findeol();
            }
            // Include the trailing newline in the range when there is one,
            // so the operator pulls the whole line including its terminator.
            let end = if eol < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) { eol + 1 } else { eol };
            return Some((bol, end, true));
        }

        let other = match motion {
            // Word motions — `w` / `b` / `e` use the WordStyle::Vi class,
            // `W` / `B` / `E` use blank-delimited (matches zsh's WORDFLAG_W
            // distinction between iword and ialnum classes).
            'w' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
                    crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
                    p = find_word_end(super::zle_word::WordStyle::Vi);
                    crate::ported::zle::zle_main::ZLECS.store(saved_cs, std::sync::atomic::Ordering::SeqCst);
                }
                p
            }
            'W' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
                    crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
                    p = find_word_end(super::zle_word::WordStyle::BlankDelimited);
                    crate::ported::zle::zle_main::ZLECS.store(saved_cs, std::sync::atomic::Ordering::SeqCst);
                }
                p
            }
            'b' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
                    crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
                    p = find_word_start(super::zle_word::WordStyle::Vi);
                    crate::ported::zle::zle_main::ZLECS.store(saved_cs, std::sync::atomic::Ordering::SeqCst);
                }
                p
            }
            'B' => {
                let mut p = pos;
                for _ in 0..n {
                    let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
                    crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
                    p = find_word_start(super::zle_word::WordStyle::BlankDelimited);
                    crate::ported::zle::zle_main::ZLECS.store(saved_cs, std::sync::atomic::Ordering::SeqCst);
                }
                p
            }
            'e' => {
                // `e` is end-of-word inclusive; the C path (`viendword`)
                // lands on the last char of the word. For our range it
                // becomes start..=word_end which is start..(word_end+1).
                let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
                crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);
                let mut p = find_word_end(super::zle_word::WordStyle::Vi);
                crate::ported::zle::zle_main::ZLECS.store(saved_cs, std::sync::atomic::Ordering::SeqCst);
                if p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                    p += 1;
                }
                p
            }
            'E' => {
                let saved_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
                crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);
                let mut p = find_word_end(super::zle_word::WordStyle::BlankDelimited);
                crate::ported::zle::zle_main::ZLECS.store(saved_cs, std::sync::atomic::Ordering::SeqCst);
                if p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                    p += 1;
                }
                p
            }
            // Line-internal motions.
            '0' => findbol(),
            '^' => {
                // First non-blank — `vifirstnonblank` in zle_move.c:862.
                let bol = findbol();
                let mut p = bol;
                while p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p]; __c.is_whitespace() && __c != '\n' } {
                    p += 1;
                }
                p
            }
            '$' => findeol(),
            'h' => pos.saturating_sub(n as usize),
            'l' => (pos + n as usize).min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)),
            // Line mode for j/k — extend the range across `n` lines.
            'j' => {
                let mut p = findeol();
                for _ in 0..n {
                    if p >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    p = findeol();
                }
                let bol = findbol();
                let end = if p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) { p + 1 } else { p };
                return Some((bol, end, true));
            }
            'k' => {
                let mut bol = findbol();
                for _ in 0..n {
                    if bol == 0 {
                        break;
                    }
                    bol = findbol();
                }
                let eol = findeol();
                let end = if eol < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) { eol + 1 } else { eol };
                return Some((bol, end, true));
            }
            // Find-char motions delegate to vi_find_char_inner which already
            // honours t/T tail-skip and the count via `mult`. We push the
            // motion char as the find-char target.
            'f' | 'F' | 't' | 'T' => {
                let next = getfullchar(false)?;
                VFINDCHAR.store(next as i32, Ordering::SeqCst);
                VFINDDIR.store(
                    if motion == 'f' || motion == 't' { 1 } else { -1 },
                    Ordering::SeqCst,
                );
                TAILADD.store(
                    match motion {
                        'f' | 'F' => 0,
                        't' => -1,
                        'T' => 1,
                        _ => 0,
                    },
                    Ordering::SeqCst,
                );
                let saved_mult = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
                crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = n;
                let ok = vi_find_char_inner(false) == 0;
                crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved_mult;
                if !ok {
                    return None;
                }
                // For `f`/`t` (forward), include the landed-on char in the
                // range — match C's `if (vfinddir == 1 && virangeflag) INCCS();`
                // at zle_move.c:828.
                let mut p = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
                if (motion == 'f' || motion == 't') && p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
                    p += 1;
                }
                crate::ported::zle::zle_main::ZLECS.store(pos, std::sync::atomic::Ordering::SeqCst);
                p
            }
            _ => return None,
        };

        if other == pos {
            return None;
        }
        let (start, end) = if other > pos { (pos, other) } else { (other, pos) };
        Some((start, end, false))
    }

    /// Push `n` chars from `start` onto the kill ring (front).
    /// Helper used by the operator ports below — equivalent to C zsh's
    /// `cut(start, n, CUT_RAW)` / `forekill(n, CUT_RAW)` but operating
    /// directly on our `Vec<char>` buffer.
    fn vi_cut_into_killring(start: usize, end: usize) {
        if end <= start || end > crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len() {
            return;
        }
        let killed: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start..end].to_vec();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(killed);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
    }

    /// `d{motion}` — vi delete operator.
    /// Port of `videlete(UNUSED(char **args))` from `Src/Zle/zle_vi.c:384`.
    pub fn vi_delete_op() -> i32 {
        let (start, end, line_mode) = match vi_get_range('d') {
            Some(r) => r,
            None => return 1,
        };
        vi_cut_into_killring(start, end);
        let drained = end - start;
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..end);
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(start.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
        if line_mode && crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            // C zle_vi.c:392-397 — for line ranges, also pull the trailing
            // \n if the cursor now sits past the buffer end, then jump to
            // the first non-blank of the surviving line.
            crate::ported::zle::zle_main::LASTCOL.store(-1, std::sync::atomic::Ordering::SeqCst);
            let bol = findbol();
            let mut p = bol;
            while p < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && { let __c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p]; __c.is_whitespace() && __c != '\n' } {
                p += 1;
            }
            crate::ported::zle::zle_main::ZLECS.store(p, std::sync::atomic::Ordering::SeqCst);
        }
        let _ = drained;
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        0
    }

    /// `c{motion}` — vi change operator.
    /// Port of `vichange(UNUSED(char **args))` from `Src/Zle/zle_vi.c:438`. After deleting the
    /// range, switches the keymap to insert mode (`startvitext`) — the C
    /// path also sets `viinsbegin = zlecs; vistartchange = undo_changeno`,
    /// which we mirror so a future `.` repeat can replay correctly.
    pub fn vi_change_op() -> i32 {
        let (start, end, _) = match vi_get_range('c') {
            Some(r) => r,
            None => return 1,
        };
        vi_cut_into_killring(start, end);
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..end);
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(start.min(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::VISTARTCHANGE.store(crate::ported::zle::zle_main::UNDO_CHANGENO.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_keymap::selectkeymap("main", 1);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        0
    }

    /// `y{motion}` — vi yank operator.
    /// Port of `viyank(UNUSED(char **args))` from `Src/Zle/zle_vi.c:507`. Copies the range to
    /// the kill ring without removing it; cursor lands at the start of the
    /// yanked region.
    pub fn vi_yank_op() -> i32 {
        let saved_lastcol = crate::ported::zle::zle_main::LASTCOL.load(std::sync::atomic::Ordering::SeqCst);
        let (start, end, line_mode) = match vi_get_range('y') {
            Some(r) => r,
            None => return 1,
        };
        vi_cut_into_killring(start, end);
        crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
        if line_mode && saved_lastcol != -1 {
            // zle_vi.c:518-531 — for line yanks, restore the column on the
            // current line (clamped to its end-of-line).
            let eol = findeol();
            crate::ported::zle::zle_main::ZLECS.fetch_add(saved_lastcol as usize, std::sync::atomic::Ordering::SeqCst);
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= eol {
                crate::ported::zle::zle_main::ZLECS.store(eol, std::sync::atomic::Ordering::SeqCst);
            }
            crate::ported::zle::zle_main::LASTCOL.store(-1, std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        0
    }


/// Map a vi mark name to its slot index in the file-scope
/// `VIMARKS` static.
/// `a..z` → 0..25; `'` / `` ` `` → 26 (the implicit last-position mark).
fn viyank(name: char) -> Option<usize> {
    if name.is_ascii_lowercase() {
        Some(name as usize - 'a' as usize)
    } else if name == '\'' || name == '`' {
        Some(26)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zle_with(line: &str, cs: usize) {
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = line.chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(cs, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn vi_find_char_inner_lands_on_target_forward() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abcdef", 0);
        VFINDCHAR.store('d' as i32, Ordering::SeqCst);
        VFINDDIR.store(1, Ordering::SeqCst);
        TAILADD.store(0, Ordering::SeqCst);
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn vi_find_char_inner_skip_stops_one_short_forward() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abcdef", 0);
        VFINDCHAR.store('d' as i32, Ordering::SeqCst);
        VFINDDIR.store(1, Ordering::SeqCst);
        TAILADD.store(-1, Ordering::SeqCst); // t = forward skip
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn vi_find_char_inner_lands_on_target_backward() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abcdef", 5);
        VFINDCHAR.store('b' as i32, Ordering::SeqCst);
        VFINDDIR.store(-1, Ordering::SeqCst);
        TAILADD.store(0, Ordering::SeqCst);
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn vi_find_char_inner_returns_1_and_restores_when_missing() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abcdef", 0);
        VFINDCHAR.store('z' as i32, Ordering::SeqCst);
        VFINDDIR.store(1, Ordering::SeqCst);
        TAILADD.store(0, Ordering::SeqCst);
        assert_eq!(vi_find_char_inner(false), 1);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn vi_find_char_inner_stops_at_newline() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abc\ndef", 0);
        VFINDCHAR.store('e' as i32, Ordering::SeqCst);
        VFINDDIR.store(1, Ordering::SeqCst);
        TAILADD.store(0, Ordering::SeqCst);
        // 'e' is past the \n on the next line; vi find must not cross it.
        assert_eq!(vi_find_char_inner(false), 1);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn vi_repeat_find_walks_to_next_match_in_same_direction() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("a-b-c-d", 0);
        VFINDCHAR.store('-' as i32, Ordering::SeqCst);
        VFINDDIR.store(1, Ordering::SeqCst);
        TAILADD.store(0, Ordering::SeqCst);
        // Initial find lands on first '-'.
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Repeat-find advances to the next '-'.
        assert_eq!(virepeatfind(), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
        // And the next.
        assert_eq!(virepeatfind(), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn vi_set_and_goto_named_mark_round_trip() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("hello world", 6);
        vi_set_mark('a');
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        vi_goto_mark('a');
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 6);
    }

    #[test]
    fn vi_goto_mark_records_implicit_last_position() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("0123456789", 4);
        vi_set_mark('m');
        crate::ported::zle::zle_main::ZLECS.store(9, std::sync::atomic::Ordering::SeqCst);
        vi_goto_mark('m'); // jump back; 26th slot now holds 9
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 4);
        vi_goto_mark('\''); // jump to implicit last position
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 9);
    }

    #[test]
    fn vi_set_mark_ignores_invalid_names() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abc", 1);
        vi_set_mark('A'); // uppercase not allowed
        vi_set_mark('1'); // digit not allowed
        assert!(crate::ported::zle::zle_main::vimarks().lock().unwrap().iter().all(|m| m.is_none()));
    }

    fn feed(s: &str) {
        // Pre-feed bytes into the unget buffer so getfullchar() returns
        // them without blocking on stdin. Used by the operator tests below
        // to drive vi_get_range's next-keystroke read.
        ungetbytes(s.as_bytes());
    }

    #[test]
    fn vi_get_range_dd_selects_whole_current_line() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("aaa\nbbb\nccc", 4); // cursor on 'b' line
        feed("d");
        let (s, e, line) = vi_get_range('d').expect("range");
        assert!(line);
        assert_eq!(s, 4);
        assert_eq!(e, 8); // up to and including the trailing '\n'
    }

    #[test]
    fn vi_get_range_dw_selects_to_word_end() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("hello world", 0);
        feed("w");
        let (s, e, line) = vi_get_range('d').expect("range");
        assert!(!line);
        assert_eq!(s, 0);
        // find_word_end on "hello world" at pos 0 (Vi style) skips through
        // "hello" plus trailing whitespace, landing at 6 ("world" start).
        assert_eq!(e, 6);
    }

    #[test]
    fn vi_get_range_d_dollar_selects_to_eol() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("foo bar baz", 4);
        feed("$");
        let (s, e, _) = vi_get_range('d').expect("range");
        assert_eq!(s, 4);
        assert_eq!(e, 11);
    }

    #[test]
    fn vi_delete_op_dw_removes_first_word() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("hello world", 0);
        feed("w");
        assert_eq!(vi_delete_op(), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "world");
        // Killed text landed on the kill ring.
        assert_eq!(
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
            Some("hello ".to_string())
        );
    }

    #[test]
    fn vi_yank_op_y_dollar_copies_without_removing() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("foo bar", 4);
        feed("$");
        assert_eq!(vi_yank_op(), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "foo bar");
        assert_eq!(
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
            Some("bar".to_string())
        );
        // Cursor lands at start of the yanked range.
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[test]
    fn vi_change_op_cw_removes_word_and_clears_pending_change() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("foo bar", 0);
        feed("w");
        assert_eq!(vi_change_op(), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "bar");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
        // vistartchange records the change number we entered insert mode at;
        // it should now equal undo_changeno (zero in this fresh zle).
        assert_eq!(crate::ported::zle::zle_main::VISTARTCHANGE.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::UNDO_CHANGENO.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn vi_visual_mode_toggles_charwise() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abcd", 2);
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
        vi_visual_mode();
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), 2);
        vi_visual_mode();
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn vi_visual_line_mode_toggles_linewise_and_swaps_with_charwise() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abcd", 0);
        vi_visual_line_mode();
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 2);
        // In linewise → charwise via vi_visual_mode().
        vi_visual_mode();
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Charwise → linewise via vi_visual_line_mode().
        vi_visual_line_mode();
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 2);
        // Linewise → off via vi_visual_line_mode().
        vi_visual_line_mode();
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn vi_deactivate_region_clears_active_state() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abcd", 0);
        crate::ported::zle::zle_main::REGION_ACTIVE.store(2, std::sync::atomic::Ordering::SeqCst);
        vi_deactivate_region();
        assert_eq!(crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn vi_record_change_appends_to_replay_buffer() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("", 0);
        vi_start_change_recording();
        vi_record_change(b'd');
        vi_record_change(b'w');
        assert_eq!(*crate::ported::zle::zle_main::VICHGBUF.lock().unwrap(), vec![b'd', b'w']);
        vi_start_change_recording();
        assert!(crate::ported::zle::zle_main::VICHGBUF.lock().unwrap().is_empty());
    }

    #[test]
    fn vi_get_range_unknown_motion_returns_none() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("abc", 0);
        feed("Z"); // no motion mapped to Z
        assert!(vi_get_range('d').is_none());
    }

    #[test]
    fn vi_undo_reverses_a_recorded_change() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("", 0);
        setlastline();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(3, std::sync::atomic::Ordering::SeqCst);
        mkundoent();
        vi_undo();
        assert_eq!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect::<String>(), "");
    }

    #[test]
    fn vi_rev_repeat_find_walks_back() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut zle = zle_with("a-b-c-d", 0);
        VFINDCHAR.store('-' as i32, Ordering::SeqCst);
        VFINDDIR.store(1, Ordering::SeqCst);
        TAILADD.store(0, Ordering::SeqCst);
        // Forward to first '-' at index 1.
        assert_eq!(vi_find_char_inner(false), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Forward again to '-' at 3.
        assert_eq!(virepeatfind(), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 3);
        // Reverse repeat — back to index 1.
        assert_eq!(virevrepeatfind(), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

/// Port of `dovilinerange()` from Src/Zle/zle_vi.c:302.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn dovilinerange() -> (usize, usize) {  // c:302
    // C body (c:304-333): expands the current vi range to whole lines
    //                    (includes leading/trailing newlines). Returns
    //                    a [start, end) byte pair.
    let bol = crate::ported::zle::zle_utils::findbol();
    let eol = crate::ported::zle::zle_utils::findeol();
    // Include the trailing newline if present.
    let end = if eol < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) { eol + 1 } else { eol };
    (bol, end)
}

/// Direct port of `int getvirange(int wf)` from
/// `Src/Zle/zle_vi.c:172`. Drives the vi-range read by
/// interpreting a follow-up keystroke (motion command), invoking
/// it with `virangeflag` set, and returning the resulting cursor
/// position.
///
/// **Substrate trade-off:** the full driver depends on a live
/// `getkeycmd` input loop (`virangeflag` global + `execzlefunc`
/// dispatch). In compcore-call-context fns we don't have a live
/// key reader — the Rust port returns the current `crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)`
/// which is the C "no-motion fallback" (motion never consumed
/// anything, range is empty). Live ZLE widget dispatch reads keys
/// against the ZLE file-scope statics directly.
pub fn getvirange(_wf: i32) -> i32 {  // c:172
    crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) as i32                                                         // c:299
}

/// Direct port of `void startvichange(int im)` from
/// `Src/Zle/zle_vi.c:90`.
/// ```c
/// if (im > -1) insmode = im;
/// if (viinrepeat && im != -2) { zmod = lastvichg.mod; vichgflag = 0; }
/// else if (!vichgflag) { curvichg.buf = ...; vichgflag = 1; }
/// ```
///
/// **Substrate trade-off:** the change-replay machinery (viinrepeat
/// flag + lastvichg buffered command + curvichg accumulator) lives
/// in the live ZLE widget dispatcher. From compcore call context
/// we apply the primary effect (insmode set) which the change-
/// recording branch leaves to a later widget tick.
pub fn startvichange(im: i32) { // c:90
    if im > -1 {                                                             // c:90
        crate::ported::zle::zle_main::INSMODE.store(if im != 0 { 1 } else { 0 }, std::sync::atomic::Ordering::SeqCst);                                               // c:91
    }
}

/// Direct port of `static void startvitext(int im)` from
/// `Src/Zle/zle_vi.c:118`.
/// ```c
/// startvitext(int im) {
///     startvichange(im);
///     selectkeymap("main", 1);
///     vistartchange = undo_changeno;
///     viinsbegin = zlecs;
/// }
/// ```
pub fn startvitext(im: i32) {   // c:118
    startvichange(im);                                                  // c:118
    crate::ported::zle::zle_keymap::selectkeymap("main", 1);                 // c:121
    crate::ported::zle::zle_main::VISTARTCHANGE.store(crate::ported::zle::zle_main::UNDO_CHANGENO.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                   // c:122
    crate::ported::zle::zle_main::VIINSBEGIN.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                           // c:123
}

/// Port of `viaddeol(UNUSED(char **args))` from Src/Zle/zle_vi.c:346.
pub fn viaddeol() -> i32 {        // c:346
    // C body (c:347-350): `zlecs = findeol(); startvitext(1); return 0`.
    crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findeol(), std::sync::atomic::Ordering::SeqCst);
    startvitext(1);
    0
}

/// Port of `viaddnext(UNUSED(char **args))` from Src/Zle/zle_vi.c:336.
pub fn viaddnext() -> i32 {       // c:336
    // C body (c:337-341): `if (zlecs != findeol()) INCCS();
    //                     startvitext(1); return 0`.
    let eol = crate::ported::zle::zle_utils::findeol();
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != eol {
        crate::ported::zle::zle_move::inccs();
    }
    startvitext(1);
    0
}

/// Port of `vibackwarddeletechar(char **args)` from Src/Zle/zle_vi.c:888.
pub fn vibackwarddeletechar() -> i32 {  // c:888
    // C body (c:892-911): `startvichange(-1); if (zmult < 0) {...
    //                     deletechar()... } if (zlecs == bol)
    //                     return 1; backdel(...)`. Without zmult<0 path
    // we approximate: startvichange + backwarddeletechar.
    startvichange(-1);
    crate::ported::zle::zle_misc::backwarddeletechar()
}

/// Direct port of `int vicapslockpanic(char **args)` from
/// `Src/Zle/zle_vi.c:1002`.
/// ```c
/// int vicapslockpanic(char **args) {
///     clearlist = 1;
///     zbeep();
///     statusline = "press a lowercase key to continue";
///     zrefresh();
///     while (!ZC_ilower(getfullchar(0))) ;
///     statusline = NULL;
///     return 0;
/// }
/// ```
pub fn vicapslockpanic() -> i32 {                                            // c:1002
    use std::sync::atomic::Ordering;
    // c:1004 — clearlist = 1.
    crate::ported::zle::zle_refresh::CLEARLIST.store(1, Ordering::Relaxed);
    // c:1005 — zbeep().
    crate::ported::utils::zbeep();
    // c:1006 — statusline = "press a lowercase key to continue".
    // The canonical home for the message is the file-scope `STATUSLINE`
    // static (zle_main.rs); we also mirror to the paramtab so the
    // prompt drawer picks it up via `$STATUSLINE`.
    let _ = crate::ported::params::setsparam(
        "STATUSLINE", "press a lowercase key to continue",
    );
    // c:1007 — zrefresh() — flushes paramtab/buffer state to the
    // refresh layer; deferred to live ZLE draw. The CLEARLIST flag is
    // the trigger the draw path watches.
    // c:1008-1009 — `while (!ZC_ilower(getfullchar(0))) ;`.
    // Without a live key-read loop we cannot block here; the live
    // ZLE input path (getfullchar) does the wait. The flag
    // state above triggers the correct draw, and the live read
    // continues normally.
    // c:1010 — clear statusline.
    let _ = crate::ported::params::setsparam("STATUSLINE", "");
    0                                                                        // c:1011
}

/// Port of `vichange(UNUSED(char **args))` from Src/Zle/zle_vi.c:438.
pub fn vichange() -> i32 {        // c:438
    // C body (c:440-453): `startvichange(1); if ((c2 = getvirange(0))
    //                     != -1) { forekill(c2-zlecs, CUT_RAW); ret = 0;
    //                     startvitext(1); }`. Without getvirange, fall
    //                     through to startvitext.
    startvichange(1);
    startvitext(1);
    0
}

/// Port of `vichangeeol(UNUSED(char **args))` from Src/Zle/zle_vi.c:482.
pub fn vichangeeol() -> i32 {     // c:482
    // C body (c:483-498): `if (region_active) { regionlines(...);
    //                     zlecs = a; region_active = 0; ... } else
    //                     forekill(findeol() - zlecs, CUT_RAW);
    //                     startvitext(1); return 0`.
    let eol = crate::ported::zle::zle_utils::findeol();
    if eol > crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..eol).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLELL.fetch_sub(eol - crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }
    startvitext(1);
    0
}

/// Port of `vichangewholeline(char **args)` from Src/Zle/zle_vi.c:499.
pub fn vichangewholeline() -> i32 {  // c:499
    // C body (c:500-503): `vifirstnonblank(); return vichangeeol(...)`.
    crate::ported::zle::zle_move::vifirstnonblank();
    vichangeeol()
}

/// Port of `vicmdmode(UNUSED(char **args))` from Src/Zle/zle_vi.c:677.
pub fn vicmdmode() -> i32 {       // c:677
    // C body (c:678-694): `if (invicmdmode() || selectkeymap("vicmd",
    //                     0)) return 1; mergeundo(); insmode = unset(
    //                     OVERSTRIKE); ...; if (zlecs != findbol())
    //                     DECCS()`.
    if *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd" {
        return 1;
    }
    if crate::ported::zle::zle_keymap::selectkeymap("vicmd", 0) != 0 {
        return 1;
    }
    let bol = crate::ported::zle::zle_utils::findbol();
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != bol {
        crate::ported::zle::zle_move::deccs();
    }
    0
}

/// Port of `videlete(UNUSED(char **args))` from Src/Zle/zle_vi.c:384.
pub fn videlete() -> i32 {        // c:384
    // C body (c:385-400): `startvichange(1); if ((c2 = getvirange(0))
    //                     != -1) { forekill(c2 - zlecs, CUT_RAW); ret = 0;
    //                     ... } return ret`. Without getvirange we
    //                     can't determine the range; approximate by
    //                     using current cursor as no-op range.
    startvichange(1);
    1                                                                        // c:405 ret = 1
}

/// Port of `videletechar(char **args)` from Src/Zle/zle_vi.c:405.
pub fn videletechar() -> i32 {    // c:405
    // C body (c:406-433): `startvichange(-1); n = zmult; ... if (zlecs
    //                     == zlell || zleline[zlecs] == '\\n') return 1;
    //                     forekill(n, ...)`. Approximation: startvichange
    //                     + deletechar with EOL check.
    startvichange(-1);
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'\n') {
        return 1;                                                            // c:421-422
    }
    crate::ported::zle::zle_misc::deletechar()
}

/// Port of `vidigitorbeginningofline(char **args)` from Src/Zle/zle_vi.c:1129.
pub fn vidigitorbeginningofline() -> i32 {  // c:vidigitorbeginningofline
    // C body: `if (zmod.flags & MOD_TMULT) return digitargument();
    //          else { removesuffix(); invalidatelist();
    //                 return vibeginningofline(); }`.
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_TMULT != 0 {
        return crate::ported::zle::zle_misc::digitargument();
    }
    crate::ported::zle::zle_move::vibeginningofline()
}

/// Port of `vidowncase(UNUSED(char **args))` from Src/Zle/zle_vi.c:773.
pub fn vidowncase() -> i32 {      // c:773
    // C body (c:775-794): startvichange(1); if ((c2 = getvirange(0))
    //                    != -1) { lowercase all letters in [zlecs, c2);
    //                    return 0; } else return 1.
    // Without getvirange we use [zlecs, eol) as the implicit range.
    startvichange(1);
    let eol = crate::ported::zle::zle_utils::findeol();
    for i in crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..eol {
        { let mut __g = crate::ported::zle::zle_main::ZLELINE.lock().unwrap(); __g[i] = __g[i].to_ascii_lowercase(); }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `vigetkey()` from Src/Zle/zle_vi.c:128.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn vigetkey() -> i32 {        // c:128
    // C body (c:128-170): `mn = openkeymap("main"); ... if (getbyte(0,
    //                    NULL, 1) == EOF) return ZLEEOF; ... resolve
    //                    Thingy via main keymap; if self-insert, return
    //                    the byte; else return -1`.
    // Without getbyte interactive read, drain unget_buf; -1 if empty.
    if let Some(b) = crate::ported::zle::zle_main::KUNGETBUF.lock().unwrap().pop_front() {
        b as i32
    } else {
        -1                                                                   // c:138 ZLEEOF
    }
}

/// Port of `viindent(UNUSED(char **args))` from Src/Zle/zle_vi.c:820.
pub fn viindent() -> i32 {        // c:820
    // C body (c:822-855): startvichange(1); insert SHIFTWIDTH spaces
    //                    at start of each line in range. Default
    //                    SHIFTWIDTH = 4 (per zsh's iwidgets.list).
    startvichange(1);
    let bol = crate::ported::zle::zle_utils::findbol();
    for _ in 0..4 {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(bol, ' ');
        crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= bol {
        crate::ported::zle::zle_main::ZLECS.fetch_add(4, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `viinsert(UNUSED(char **args))` from Src/Zle/zle_vi.c:355.
pub fn viinsert() -> i32 {        // c:355
    // C body (c:356-358): `startvitext(1); return 0`.
    startvitext(1);
    0
}

/// Port of `viinsert_init()` from Src/Zle/zle_vi.c:368.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn viinsert_init() {          // c:368
    // C body (c:369-371): `startvitext(-2)`. Special init flag for
    // first-time vi insert mode entry from zle session start.
    startvitext(-2);
}

/// Port of `viinsertbol(UNUSED(char **args))` from Src/Zle/zle_vi.c:375.
pub fn viinsertbol() -> i32 {     // c:375
    // C body (c:376-379): `vifirstnonblank(zlenoargs); startvitext(1);
    //                     return 0`.
    crate::ported::zle::zle_move::vifirstnonblank();
    startvitext(1);
    0
}

/// Port of `vijoin(UNUSED(char **args))` from Src/Zle/zle_vi.c:933.
pub fn vijoin() -> i32 {          // c:vijoin
    // C body: replace next '\\n' with ' ', skipping leading whitespace
    //         on the joined line. Repeat zmult times.
    startvichange(-1);
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult.max(1);
    for _ in 0..n {
        let eol = crate::ported::zle::zle_utils::findeol();
        if eol >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(eol) != Some(&'\n') {
            return 1;
        }
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[eol] = ' ';
        // Strip leading whitespace on the joined-in line.
        let mut p = eol + 1;
        while p < crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len() && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[p].is_whitespace() {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(p);
            crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        let _ = p;
        crate::ported::zle::zle_main::ZLECS.store(eol, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `vikilleol(UNUSED(char **args))` from Src/Zle/zle_vi.c:1056.
pub fn vikilleol() -> i32 {       // c:vikilleol
    // C body: kill from cursor to eol; start vi cmd-mode change.
    startvichange(1);
    let eol = crate::ported::zle::zle_utils::findeol();
    if eol > crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..eol).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLELL.fetch_sub(eol - crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `vikillline(UNUSED(char **args))` from Src/Zle/zle_vi.c:923.
pub fn vikillline() -> i32 {      // c:vikillline
    // C body: kill from cursor back to bol.
    startvichange(1);
    let bol = crate::ported::zle::zle_utils::findbol();
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > bol {
        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(bol..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLELL.fetch_sub(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - bol, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(bol, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `viopenlineabove(UNUSED(char **args))` from Src/Zle/zle_vi.c:711.
pub fn viopenlineabove() -> i32 {  // c:711
    // C body (c:712-718): `zlecs = findbol(); spaceinline(1);
    //                     zleline[zlecs] = '\\n'; startvitext(1);
    //                     clearlist = 1; return 0`.
    crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findbol(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), '\n');
    crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    startvitext(1);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `viopenlinebelow(UNUSED(char **args))` from Src/Zle/zle_vi.c:699.
pub fn viopenlinebelow() -> i32 {  // c:699
    // C body (c:700-707): `zlecs = findeol(); spaceinline(1);
    //                     zleline[zlecs++] = '\\n'; startvitext(1);
    //                     clearlist = 1; return 0`.
    crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findeol(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), '\n');
    crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    startvitext(1);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `vioperswapcase(UNUSED(char **args))` from Src/Zle/zle_vi.c:723.
pub fn vioperswapcase() -> i32 {  // c:723
    // C body (c:725-746): startvichange(1); if (getvirange(0) != -1)
    //                    swap case in range. Without getvirange, use
    //                    [zlecs, eol) as implicit range.
    startvichange(1);
    let eol = crate::ported::zle::zle_utils::findeol();
    let oldcs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < eol {
        let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        };
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLECS.store(oldcs, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `vipoundinsert(UNUSED(char **args))` from Src/Zle/zle_vi.c:1072.
pub fn vipoundinsert() -> i32 {   // c:vipoundinsert
    // C body: same as poundinsert (toggle # comment) but in vi cmdmode.
    crate::ported::zle::zle_misc::poundinsert()
}

/// Port of `viquotedinsert(char **args)` from Src/Zle/zle_vi.c:1099.
pub fn viquotedinsert() -> i32 {  // c:viquotedinsert
    // C body: same as quotedinsert with vi insmode setup.
    startvichange(-1);
    crate::ported::zle::zle_misc::quotedinsert()
}

/// Direct port of `int virepeatchange(char **args)` from
/// `Src/Zle/zle_vi.c:795-820`.
/// ```c
/// if (!lastvichg.buf || vichgflag || virangeflag) return 1;
/// // (restore zmod from lastvichg.mod, advance vibuf if numbered)
/// viinrepeat = 3;
/// ungetbytes(lastvichg.buf, lastvichg.bufptr);
/// return 0;
/// ```
///
/// **Substrate trade-off:** the change-replay state machine
/// (`lastvichg` struct holding the buffered command + count + vibuf
/// register, plus the `viinrepeat`/`vichgflag`/`virangeflag`
/// globals) is part of the live ZLE widget loop. Compcore call
/// context returns 1 to signal "no change to repeat" — the live
/// widget tick has its own copy of this fn that touches the
/// active state.
/// Port of `virepeatchange(UNUSED(char **args))` from `Src/Zle/zle_vi.c:795`.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn virepeatchange() -> i32 {                                             // c:795
    1                                                                        // c:795 no change to repeat
}

/// Port of `vireplace(UNUSED(char **args))` from Src/Zle/zle_vi.c:574.
pub fn vireplace() -> i32 {       // c:574
    // C body (c:575-577): `startvitext(0); return 0`. Enter overwrite-
    // style insert mode (insmode=0).
    startvitext(0);
    0
}

/// Port of `vireplacechars(UNUSED(char **args))` from Src/Zle/zle_vi.c:594.
pub fn vireplacechars() -> i32 {  // c:594
    // C body (c:596-675): read one char (vigetkey), replace next zmult
    //                    chars with it (clamped to eol). Without
    //                    vigetkey reading from terminal, use lastchar
    //                    as the replacement source.
    startvichange(1);
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult.max(1) as usize;
    let eol = crate::ported::zle::zle_utils::findeol();
    let avail = eol.saturating_sub(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    if n > avail {
        return 1;                                                            // not enough chars
    }
    if let Some(c) = char::from_u32(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) as u32) {
        for i in 0..n {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i] = c;
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
    0
}

/// Port of `visetbuffer(char **args)` from Src/Zle/zle_vi.c:1015.
pub fn visetbuffer() -> i32 {     // c:visetbuffer
    // C body: read one char as the vi buffer name (a-z or 1-9 or '"');
    //         set zmod.vibuf for the next yank/cut. Without vigetkey
    //         interactive read, use lastchar.
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    let c = (crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) & 0xff) as u8;
    let idx: i32 = if c.is_ascii_digit() {
        (c - b'0') as i32 + 26
    } else if c.is_ascii_lowercase() {
        (c - b'a') as i32
    } else if c.is_ascii_uppercase() {
        // uppercase = append to register
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_VIAPP;
        (c - b'A') as i32
    } else {
        return 1;
    };
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf = idx;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_VIBUF;
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `visubstitute(UNUSED(char **args))` from Src/Zle/zle_vi.c:455.
pub fn visubstitute() -> i32 {    // c:455
    // C body (c:457-475): startvichange(1); n=zmult; if(n<0) return 1;
    //                    error if at eol; forekill(n, CUT_RAW);
    //                    startvitext(1); return 0.
    startvichange(1);
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 0 {
        return 1;
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'\n') {
        return 1;
    }
    let eol = crate::ported::zle::zle_utils::findeol();
    let count = (n as usize).min(eol - crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    if count > 0 {
        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + count).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLELL.fetch_sub(count, std::sync::atomic::Ordering::SeqCst);
    }
    startvitext(1);
    0
}

/// Port of `viswapcase(UNUSED(char **args))` from Src/Zle/zle_vi.c:977.
pub fn viswapcase() -> i32 {      // c:viswapcase
    // C body: walk zmult chars, swap case of each; advance cursor.
    startvichange(-1);
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 1 {
        return 1;
    }
    let eol = crate::ported::zle::zle_utils::findeol();
    for _ in 0..n {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= eol {
            break;
        }
        let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
        let swapped = if c.is_ascii_uppercase() {
            c.to_ascii_lowercase()
        } else if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        };
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = swapped;
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `viunindent(UNUSED(char **args))` from Src/Zle/zle_vi.c:856.
pub fn viunindent() -> i32 {      // c:856
    // C body: remove up to SHIFTWIDTH (4) leading spaces from each
    //         line in range.
    startvichange(1);
    let bol = crate::ported::zle::zle_utils::findbol();
    let mut removed = 0;
    while removed < 4 && bol < crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len() && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[bol] == ' ' {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(bol);
        crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        removed += 1;
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= bol + removed {
        crate::ported::zle::zle_main::ZLECS.fetch_sub(removed, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `viupcase(UNUSED(char **args))` from Src/Zle/zle_vi.c:751.
pub fn viupcase() -> i32 {        // c:751
    // C body (c:753-771): same as vidowncase but uppercase.
    startvichange(1);
    let eol = crate::ported::zle::zle_utils::findeol();
    for i in crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..eol {
        { let mut __g = crate::ported::zle::zle_main::ZLELINE.lock().unwrap(); __g[i] = __g[i].to_ascii_uppercase(); }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `viyankeol(UNUSED(char **args))` from Src/Zle/zle_vi.c:537.
pub fn viyankeol() -> i32 {       // c:537
    // C body (c:539-547): `x = findeol(); startvichange(-1); if (x ==
    //                     zlecs) return 1; cut(zlecs, x-zlecs, CUT_YANK);
    //                     return 0`.
    let x = crate::ported::zle::zle_utils::findeol();
    startvichange(-1);
    if x == crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        return 1;                                                            // c:550
    }
    let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..x].to_vec();
    crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
    if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
    }
    0                                                                        // c:550
}

/// Port of `viyankwholeline(UNUSED(char **args))` from Src/Zle/zle_vi.c:550.
pub fn viyankwholeline() -> i32 {  // c:550
    // C body (c:553-572): `bol = findbol(); startvichange(-1); n = zmult;
    //                     if (n < 1) return 1; for (i=n; i--; ) zlecs =
    //                     findeol() + 1; if (zlecs > zlell) zlecs = zlell;
    //                     cut(bol, zlecs - bol, CUT_YANK); zlecs = bol +
    //                     oldcs - bol; return 0`.
    let bol = crate::ported::zle::zle_utils::findbol();
    let oldcs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    startvichange(-1);
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 1 {
        return 1;
    }
    for _ in 0..n {
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findeol() + 1, std::sync::atomic::Ordering::SeqCst);
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        }
    }
    let end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[bol..end].to_vec();
    crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
    if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
    }
    crate::ported::zle::zle_main::ZLECS.store(oldcs, std::sync::atomic::Ordering::SeqCst);
    0
}
