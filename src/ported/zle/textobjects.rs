//! ZLE text objects — port of `Src/Zle/textobjects.c`.
//!
//! Three C functions, zero structs/enums. The Rust port matches:
//! three free fns over a `&mut Zle`, no Rust-only types.

use std::sync::atomic::Ordering;

use crate::ported::zle::zle_h::{ZC_iblank, MOD_MULT};

#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
/// Port of `blankwordclass(ZLE_CHAR_T x)` from `Src/Zle/textobjects.c:34`. The
/// vi blank-word class predicate. Returns 0 for blanks, 1 otherwise.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;

pub fn blankwordclass(x: char) -> i32 {
    // c:34
    // c:36 — `return (ZC_iblank(x) ? 0 : 1);`. `ZC_iblank` routes
    // through `wcsiblank` (Src/Zle/zle.h:62 → Src/utils.c:4302-4307):
    // `iswspace(wc) && wc != L'\n'`.
    if ZC_iblank(x) {
        0
    } else {
        1
    } // c:36
}

/// Port of `selectword(UNUSED(char **args))` from `Src/Zle/textobjects.c:41`.
/// Faithful 1:1 port of the C body. Variable names track the C
/// source where possible.
///
/// `INCCS()` / `DECCS()` / `INCPOS()` / `DECPOS()` collapse to
/// `+= 1` / `-= 1` in the Rust port because zshrs's buffer is
/// `Vec<char>` (already multibyte-aware at the storage layer; no
/// per-char byte-walk needed).
///
/// `virangeflag` is a `Src/Zle/zle_vi.c:36` file-global. The
/// cursor-adjustment arm at `c:196-203` reads it. zshrs sets the
/// flag during the live `vi`-operator-pending key-read loop in the
/// ZLE file-scope statics; standalone widget invocation reaches this
/// fn with the flag clear, which is the only state the cursor-
/// adjustment needs to handle (the `range`-set branch only fires
/// from inside `getvirange`, which has its own copy).
pub fn selectword() -> i32 {
    // c:41
    let mut n: i32 = if ZMOD.lock().unwrap().flags & MOD_MULT != 0 {
        // c:41 zmult
        ZMOD.lock().unwrap().mult
    } else {
        1
    };
    let widget = BINDK
        .lock()
        .unwrap()
        .as_ref()
        .map(|t| t.nam.clone())
        .unwrap_or_default();
    let widget = widget.as_str();
    let is_aword = widget == "select-a-word";
    let is_inword = widget == "select-in-word";
    let is_ablankword = widget == "select-a-blank-word";
    let mut all: i32 = (is_aword || is_ablankword) as i32; // c:43-44
    let viclass: fn(char) -> i32 = if is_aword || is_inword {
        wordclass // c:46-47
    } else {
        blankwordclass
    };
    if ZLELL.load(Ordering::SeqCst) == 0 {
        return 1;
    }
    let cur = ZLELINE
        .lock()
        .unwrap()
        .get(ZLECS.load(Ordering::SeqCst))
        .copied()
        .unwrap_or('\n');
    let mut sclass: i32 = viclass(cur); // c:48
    let mut doblanks: i32 = all & ((sclass != 0) as i32); // c:49 all && sclass

    let region_active =
        REGION_ACTIVE.load(Ordering::SeqCst) != 0; // c:51 (read once)

    // C's `mark == -1` sentinel doesn't exist in the Rust port (mark
    // is `usize`); the equivalent "mark is unset" condition collapses
    // into `!region_active` since mark is only meaningful when the
    // region is active. Drop the `mark == -1` disjunct.
    if !region_active
        || ZLECS.load(Ordering::SeqCst)
            == MARK.load(Ordering::SeqCst)
    {
        // c:51
        // search back to first character of same class as the start
        // position; also stop at the beginning of the line.
        MARK.store(
            ZLECS.load(Ordering::SeqCst),
            Ordering::SeqCst,
        ); // c:54
        while MARK.load(Ordering::SeqCst) != 0 {
            // c:55
            let pos =
                MARK.load(Ordering::SeqCst) - 1; // c:56-57 DECPOS
            let cp = ZLELINE
                .lock()
                .unwrap()
                .get(pos)
                .copied()
                .unwrap_or('\n');
            if cp == '\n' || viclass(cp) != sclass {
                // c:58
                break; // c:59
            }
            MARK.store(pos, Ordering::SeqCst);
            // c:60
        }
        // similarly scan forward over characters of the same class.
        while ZLECS.load(Ordering::SeqCst)
            < ZLELL.load(Ordering::SeqCst)
        {
            // c:63
            ZLECS.fetch_add(1, Ordering::SeqCst); // c:64 INCCS
            let mut pos =
                ZLECS.load(Ordering::SeqCst); // c:65
                                                                                               // single newlines within blanks are included.
            if all != 0 && sclass == 0 && pos < ZLELL.load(Ordering::SeqCst)                // c:67
                && ZLELINE.lock().unwrap().get(pos).copied() == Some('\n')
            {
                pos += 1; // c:68 INCPOS(pos)
            }
            let pc = ZLELINE
                .lock()
                .unwrap()
                .get(pos)
                .copied()
                .unwrap_or('\n');
            if pc == '\n' || viclass(pc) != sclass {
                // c:70
                break; // c:71
            }
        }

        if all != 0 {
            // c:74
            let cc = ZLELINE
                .lock()
                .unwrap()
                .get(ZLECS.load(Ordering::SeqCst))
                .copied()
                .unwrap_or('\n');
            let nclass = viclass(cc); // c:75
                                      // if either start or new position is blank advance over a
                                      // new block of characters of a common type.
            if nclass == 0 || sclass == 0 {
                // c:78
                while ZLECS.load(Ordering::SeqCst)
                    < ZLELL.load(Ordering::SeqCst)
                {
                    // c:79
                    ZLECS
                        .fetch_add(1, Ordering::SeqCst); // c:80 INCCS
                    let cc = ZLELINE
                        .lock()
                        .unwrap()
                        .get(
                            ZLECS
                                .load(Ordering::SeqCst),
                        )
                        .copied()
                        .unwrap_or('\n');
                    if cc == '\n' || viclass(cc) != nclass {
                        // c:81
                        break; // c:82
                    }
                }
                if n < 2 {
                    // c:85
                    doblanks = 0; // c:86
                }
            }
        }
    } else {
        // c:89
        // For visual mode, advance one char so repeated invocations
        // select subsequent words.
        if ZLECS.load(Ordering::SeqCst)
            > MARK.load(Ordering::SeqCst)
        {
            // c:92
            if ZLECS.load(Ordering::SeqCst)
                < ZLELL.load(Ordering::SeqCst)
            {
                // c:93
                ZLECS
                    .fetch_add(1, Ordering::SeqCst); // c:94 INCCS
            }
        } else if ZLECS.load(Ordering::SeqCst) != 0
        {
            // c:95
            ZLECS.fetch_sub(1, Ordering::SeqCst);
            // c:96 DECCS
        }
        if ZLECS.load(Ordering::SeqCst)
            < MARK.load(Ordering::SeqCst)
        {
            // c:97
            // visual mode with the cursor before the mark: move
            // cursor back.
            while {
                let cont = n > 0;
                n -= 1;
                cont
            } {
                // c:99 while (n-- > 0)
                let mut pos =
                    ZLECS.load(Ordering::SeqCst); // c:100
                let zc_pos = ZLELINE
                    .lock()
                    .unwrap()
                    .get(pos)
                    .copied()
                    .unwrap_or('\n');
                // first over blanks
                if all != 0 && (viclass(zc_pos) == 0 || zc_pos == '\n') {
                    // c:102
                    all = 0; // c:104
                    while pos != 0 {
                        // c:105
                        pos -= 1; // c:106 DECPOS
                        let pc = ZLELINE
                            .lock()
                            .unwrap()
                            .get(pos)
                            .copied()
                            .unwrap_or('\n');
                        if pc == '\n' {
                            // c:107
                            break; // c:108
                        }
                        ZLECS
                            .store(pos, Ordering::SeqCst); // c:109
                        if viclass(pc) != 0 {
                            // c:110
                            break; // c:111
                        }
                    }
                } else if ZLECS
                    .load(Ordering::SeqCst)
                    != 0
                    && ZLELINE
                        .lock()
                        .unwrap()
                        .get(
                            ZLECS
                                .load(Ordering::SeqCst),
                        )
                        .copied()
                        == Some('\n')
                {
                    // c:114
                    // for 'in' widgets pass over one newline
                    pos -= 1; // c:116 DECPOS(pos)
                    let pc = ZLELINE
                        .lock()
                        .unwrap()
                        .get(pos)
                        .copied()
                        .unwrap_or('\n');
                    if pc != '\n' {
                        // c:117
                        ZLECS
                            .store(pos, Ordering::SeqCst); // c:118
                    }
                }
                pos = ZLECS.load(Ordering::SeqCst); // c:121
                let cur = ZLELINE
                    .lock()
                    .unwrap()
                    .get(
                        ZLECS
                            .load(Ordering::SeqCst),
                    )
                    .copied()
                    .unwrap_or('\n');
                sclass = viclass(cur); // c:122
                                       // now retreat over non-blanks
                loop {
                    // c:124
                    let pc = ZLELINE
                        .lock()
                        .unwrap()
                        .get(pos)
                        .copied()
                        .unwrap_or('\n');
                    if pc == '\n' || viclass(pc) != sclass {
                        break;
                    }
                    ZLECS
                        .store(pos, Ordering::SeqCst); // c:126
                    if pos == 0 {
                        // c:127
                        ZLECS
                            .store(0, Ordering::SeqCst); // c:128
                        break; // c:129
                    }
                    pos -= 1; // c:131 DECPOS
                }
                // blanks again but only if there were none first time
                if all != 0
                    && ZLECS.load(Ordering::SeqCst)
                        != 0
                {
                    // c:134
                    pos = ZLECS
                        .load(Ordering::SeqCst);
                    pos -= 1; // c:136 DECPOS
                    let pc = ZLELINE
                        .lock()
                        .unwrap()
                        .get(pos)
                        .copied()
                        .unwrap_or('\n');
                    if viclass(pc) == 0 {
                        // c:137
                        while pos != 0 {
                            // c:138
                            pos -= 1; // c:139 DECPOS
                            let pc = ZLELINE
                                .lock()
                                .unwrap()
                                .get(pos)
                                .copied()
                                .unwrap_or('\n');
                            if pc == '\n' || viclass(pc) != 0 {
                                // c:140
                                break; // c:142
                            }
                            ZLECS
                                .store(pos, Ordering::SeqCst);
                            // c:143
                        }
                    }
                }
            }
            return 0; // c:147
        }
        n += 1; // c:148
        doblanks = 0; // c:149
    }
    // force to character-wise — c:152
    REGION_ACTIVE.store(
        if region_active { 1 } else { 0 },
        Ordering::SeqCst,
    );

    // for each digit argument, advance over a further block of one class
    while {
        n -= 1;
        n > 0
    } {
        // c:155
        if ZLECS.load(Ordering::SeqCst)
            < ZLELL.load(Ordering::SeqCst)
            && ZLELINE
                .lock()
                .unwrap()
                .get(ZLECS.load(Ordering::SeqCst))
                .copied()
                == Some('\n')
        {
            // c:156
            ZLECS.fetch_add(1, Ordering::SeqCst);
            // c:157 INCCS
        }
        let cur = ZLELINE
            .lock()
            .unwrap()
            .get(ZLECS.load(Ordering::SeqCst))
            .copied()
            .unwrap_or('\n');
        sclass = viclass(cur); // c:158
        while ZLECS.load(Ordering::SeqCst)
            < ZLELL.load(Ordering::SeqCst)
        {
            // c:159
            ZLECS.fetch_add(1, Ordering::SeqCst); // c:160 INCCS
            let cc = ZLELINE
                .lock()
                .unwrap()
                .get(ZLECS.load(Ordering::SeqCst))
                .copied()
                .unwrap_or('\n');
            if cc == '\n' || viclass(cc) != sclass {
                // c:161
                break; // c:163
            }
        }
        // for 'a' widgets, advance extra block if either consists of blanks
        if all != 0 {
            // c:165
            if ZLECS.load(Ordering::SeqCst)
                < ZLELL.load(Ordering::SeqCst)
                && ZLELINE
                    .lock()
                    .unwrap()
                    .get(
                        ZLECS
                            .load(Ordering::SeqCst),
                    )
                    .copied()
                    == Some('\n')
            {
                // c:166
                ZLECS
                    .fetch_add(1, Ordering::SeqCst); // c:167 INCCS
            }
            let cc = ZLELINE
                .lock()
                .unwrap()
                .get(ZLECS.load(Ordering::SeqCst))
                .copied()
                .unwrap_or('\n');
            let cls_here = viclass(cc);
            if sclass == 0 || cls_here == 0 {
                // c:168
                sclass = cls_here; // c:169
                if n == 1 && sclass == 0 {
                    // c:170
                    doblanks = 0; // c:171
                }
                while ZLECS.load(Ordering::SeqCst)
                    < ZLELL.load(Ordering::SeqCst)
                {
                    // c:172
                    ZLECS
                        .fetch_add(1, Ordering::SeqCst); // c:173 INCCS
                    let cc = ZLELINE
                        .lock()
                        .unwrap()
                        .get(
                            ZLECS
                                .load(Ordering::SeqCst),
                        )
                        .copied()
                        .unwrap_or('\n');
                    if cc == '\n' || viclass(cc) != sclass {
                        // c:174
                        break; // c:176
                    }
                }
            }
        }
    }

    // if we didn't remove blanks at either end we remove some at the start
    if doblanks != 0 {
        // c:181
        let mut pos = MARK.load(Ordering::SeqCst); // c:182
        while pos != 0 {
            // c:183
            pos -= 1; // c:184 DECPOS
                      // don't remove blanks at the start of the line, i.e. indentation
            let pc = ZLELINE
                .lock()
                .unwrap()
                .get(pos)
                .copied()
                .unwrap_or('\n');
            if pc == '\n' {
                // c:186
                break; // c:187
            }
            if !ZC_iblank(pc) {
                // c:188 !ZC_iblank
                pos += 1; // c:189 INCPOS
                MARK.store(pos, Ordering::SeqCst); // c:190
                break; // c:191
            }
        }
    }
    // Adjustment: vi operators don't include the cursor position; in
    // insert or emacs mode the region also doesn't, but for vi visual
    // mode it is included.
    //
    // c:196 — `virangeflag` file-global (zle_vi.c:36). When non-zero
    // a vi range operation is pending, in which case the region
    // adjustment below is suppressed because the operator already
    // handled it.
    let virangeflag =
        VIRANGEFLAG.load(Ordering::Relaxed) != 0;
    if !virangeflag {
        // c:196
        if !in_vi_cmd_mode() {
            // c:197
            REGION_ACTIVE
                .store(1, Ordering::SeqCst); // c:198
        } else if ZLECS.load(Ordering::SeqCst) != 0
            && ZLECS.load(Ordering::SeqCst)
                > MARK.load(Ordering::SeqCst)
        {
            // c:199
            ZLECS.fetch_sub(1, Ordering::SeqCst);
            // c:200 DECCS
        }
    }

    0 // c:204
}

/// Port of `selectargument(UNUSED(char **args))` from `Src/Zle/textobjects.c:212`.
///
/// The C body uses the shell's `ctxtlex()` lexer-walk machinery
/// (textobjects.c:233-257) to drive real shell tokenisation over
/// the buffer. zshrs lowers the lexer through fusevm bytecode and
/// does not expose a free-running `ctxtlex`-style scanner; this
/// port uses whitespace-split tokenisation against the buffer
/// (matches C output for simple commands without quoting /
/// expansion / heredocs). Returns 1 when `n` is out of range,
/// matching C textobjects.c:225.
pub fn selectargument() -> i32 {
    // c:212
    let n: i32 = if ZMOD.lock().unwrap().flags & MOD_MULT != 0 {
        ZMOD.lock().unwrap().mult // c:222 zmult
    } else {
        1
    };
    if n < 1
        || (2 * n as usize)
            > ZLELL.load(Ordering::SeqCst) + 1
    {
        // c:225
        return 1;
    }
    if !in_vi_cmd_mode() {
        // c:228
        REGION_ACTIVE.store(1, Ordering::SeqCst); // c:229
        MARK.store(
            ZLECS.load(Ordering::SeqCst),
            Ordering::SeqCst,
        ); // c:230
    }
    // Whitespace-split tokenisation (see fn-doc for the ctxtlex
    // tradeoff).
    let mut starts: Vec<usize> = Vec::with_capacity(n as usize);
    let mut in_word = false;
    let mut word_start = 0usize;
    starts.push(0);
    for (i, &c) in ZLELINE
        .lock()
        .unwrap()
        .iter()
        .enumerate()
    {
        if c.is_whitespace() {
            if in_word {
                in_word = false;
                if starts.len() < n as usize {
                    starts.push(i + 1);
                }
            }
        } else if !in_word {
            in_word = true;
            word_start = i;
            if i >= ZLECS.load(Ordering::SeqCst) {
                break;
            }
        }
    }
    let arg_idx = (n - 1) as usize;
    let s = starts.get(arg_idx).copied().unwrap_or(word_start);
    let e = (s..ZLELL.load(Ordering::SeqCst))
        .find(|&i| {
            ZLELINE
                .lock()
                .unwrap()
                .get(i)
                .copied()
                .map_or(true, |c| c.is_whitespace())
        })
        .unwrap_or(ZLELL.load(Ordering::SeqCst));
    MARK.store(s, Ordering::SeqCst);
    ZLECS.store(e, Ordering::SeqCst);
    if in_vi_cmd_mode()
        && ZLECS.load(Ordering::SeqCst) > 0
    {
        ZLECS.fetch_sub(1, Ordering::SeqCst);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// c:36 — `return ZC_iblank(x) ? 0 : 1`. blank → 0, non-blank → 1.
    /// Verifies the boundary cases for the word-classifier helper that
    /// selectword/selectargument iterate against.
    #[test]
    fn blankwordclass_classifies_whitespace_vs_word_chars() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(blankwordclass(' '), 0, "space is iblank");
        assert_eq!(blankwordclass('\t'), 0, "tab is iblank");
        assert_eq!(blankwordclass('a'), 1, "letter is not iblank");
        assert_eq!(blankwordclass('0'), 1, "digit is not iblank");
        assert_eq!(blankwordclass('!'), 1, "punctuation is not iblank");
        assert_eq!(
            blankwordclass('\n'),
            1,
            "newline is NOT iblank per ZC_iblank semantics"
        );
    }

    /// `Src/Zle/textobjects.c:36` — `return (ZC_iblank(x) ? 0 : 1)`.
    /// `Src/Zle/zle.h:62` aliases `ZC_iblank` to `wcsiblank` in the
    /// MULTIBYTE_SUPPORT build; `Src/utils.c:4304` defines
    /// `wcsiblank(wc)` as `iswspace(wc) && wc != L'\n'`. Non-ASCII
    /// letters are NOT `iswspace` so they fall through to the
    /// non-iblank branch (return 1).
    #[test]
    fn blankwordclass_non_ascii_letters_are_not_iblank() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(blankwordclass('é'), 1, "Latin-1 letter: not iswspace");
        assert_eq!(blankwordclass('字'), 1, "CJK ideograph: not iswspace");
        assert_eq!(blankwordclass('α'), 1, "Greek letter: not iswspace");
    }

    /// `Src/utils.c:4302-4307 wcsiblank` returns true for every
    /// `iswspace` char except `\n`. Per `Src/Zle/textobjects.c:36`
    /// that means CR/FF/VT/NBSP all classify as iblank → return 0.
    /// Pinning this prevents a regression that re-narrows the
    /// classifier back to `space || tab` (which would split words
    /// on non-ASCII whitespace in vi `aW`/`iW` selections).
    #[test]
    fn blankwordclass_wide_whitespace_classes_are_iblank() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(blankwordclass('\r'), 0, "CR is iblank per wcsiblank");
        assert_eq!(blankwordclass('\x0c'), 0, "FF is iblank per wcsiblank");
        assert_eq!(blankwordclass('\x0b'), 0, "VT is iblank per wcsiblank");
        assert_eq!(
            blankwordclass('\u{00A0}'),
            0,
            "NBSP is iblank per wcsiblank"
        );
    }

    /// `Src/Zle/textobjects.c:224-225` — `if (n < 1 || 2*n > zlell+1) return 1`.
    /// With an empty buffer (`zlell == 0`) the predicate `2*1 > 0+1`
    /// is true so the guard fires for any n>=1. Regression dropping
    /// this guard would run the tokeniser over a zero-length buffer
    /// and corrupt `MARK`/`ZLECS` (set to past-end indices) on every
    /// vi `iw`/`aw` over an empty prompt.
    #[test]
    fn selectargument_returns_one_on_empty_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(
            selectargument(),
            1,
            "c:225 — empty buffer fails 2*n > zlell+1"
        );
    }

    /// c:34-36 — `blankwordclass` digits and underscore are word
    /// chars (class 1). Pin the contract because digits' iswspace
    /// status is locale-dependent and a regression could change it.
    #[test]
    fn blankwordclass_digits_and_underscore_are_word_chars() {
        let _g = crate::test_util::global_state_lock();
        for d in '0'..='9' {
            assert_eq!(blankwordclass(d), 1, "digit {:?} must NOT be iblank", d);
        }
        assert_eq!(blankwordclass('_'), 1, "underscore is a word char");
    }

    /// c:36 — `\0` (NUL) is NOT iswspace, so it falls through to
    /// the non-iblank branch → class 1. Pin this so a regression
    /// that special-cases NUL doesn't silently change vi word
    /// selection at the buffer boundary.
    #[test]
    fn blankwordclass_nul_is_not_iblank() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            blankwordclass('\0'),
            1,
            "NUL byte must classify as non-iblank per wcsiblank semantics"
        );
    }

    /// c:36 — emoji and other non-BMP chars are not iswspace, so
    /// they fall through to class 1. Pin this so a regen that
    /// over-narrows the classifier to ASCII doesn't drop them
    /// silently into the wrong vi word group.
    #[test]
    fn blankwordclass_non_bmp_chars_are_word_chars() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(blankwordclass('\u{1F600}'), 1, "emoji is non-iblank");
        assert_eq!(blankwordclass('\u{2603}'), 1, "snowman is non-iblank");
    }

    /// c:225 — `selectargument` with `zlell=0` MUST NOT touch MARK
    /// or ZLECS. Pinning the no-side-effect property protects
    /// against a regression that increments MARK to 1 before the
    /// guard check.
    #[test]
    fn selectargument_empty_buffer_leaves_mark_and_zlecs_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        MARK.store(42, Ordering::SeqCst);
        ZLECS.store(7, Ordering::SeqCst);
        let r = selectargument();
        assert_eq!(r, 1);
        assert_eq!(
            MARK.load(Ordering::SeqCst),
            42,
            "MARK must not be touched on the c:225 guard branch"
        );
        assert_eq!(
            ZLECS.load(Ordering::SeqCst),
            7,
            "ZLECS must not be touched on the c:225 guard branch"
        );
    }

    /// c:225 — `selectargument` with `zmult = 0 && MOD_MULT` triggers
    /// the `n < 1` half of the guard. Pin the second half of the
    /// guard predicate so a regression that simplifies the OR to
    /// just the size check gets caught.
    #[test]
    fn selectargument_zero_mult_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        let mut z = ZMOD.lock().unwrap();
        z.flags = MOD_MULT;
        z.mult = 0;
        drop(z);
        assert_eq!(selectargument(), 1, "n<1 guard branch must fire");
    }

    /// c:225 — `selectargument` with `zmult = -1 && MOD_MULT` also
    /// fires the n<1 guard (negative count). Pin negative-count
    /// rejection.
    #[test]
    fn selectargument_negative_mult_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(11, Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        let mut z = ZMOD.lock().unwrap();
        z.flags = MOD_MULT;
        z.mult = -1;
        drop(z);
        assert_eq!(
            selectargument(),
            1,
            "negative count must fail the n<1 guard"
        );
    }

    /// c:36 — `blankwordclass(' ')` returns 0 (iblank). C dispatch
    /// at `Src/Zle/zle.h:62`: ZC_iblank → wcsiblank → iswspace AND
    /// not '\n'. ASCII space is iswspace AND not '\n' → iblank → 0.
    #[test]
    fn blankwordclass_pure_space_is_iblank() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(blankwordclass(' '), 0, "space MUST be iblank (class 0)");
    }

    /// c:36 — `\t` (tab) is iblank.
    #[test]
    fn blankwordclass_tab_is_iblank() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(blankwordclass('\t'), 0, "tab MUST be iblank (class 0)");
    }

    /// c:36 — `\n` (newline) is NOT iblank per `wcsiblank` def
    /// (`Src/utils.c:4304 — iswspace(wc) && wc != L'\\n'`). Pin
    /// the newline-exclusion because a regen that drops the `!= '\n'`
    /// condition would silently treat newline as whitespace and
    /// break vi `aW` word selection across line boundaries.
    #[test]
    fn blankwordclass_newline_is_NOT_iblank() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            blankwordclass('\n'),
            1,
            "newline must NOT be iblank per wcsiblank's explicit exclusion"
        );
    }

    /// c:34 — `blankwordclass` is a pure function: same input →
    /// same output, no side effects. Verify idempotency by calling
    /// 1000 times with the same input.
    #[test]
    fn blankwordclass_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..1000 {
            assert_eq!(blankwordclass('a'), 1);
            assert_eq!(blankwordclass(' '), 0);
            assert_eq!(blankwordclass('\n'), 1);
        }
    }

    /// c:212 — `selectargument` against a buffer with ONLY whitespace
    /// must NOT panic (defensive). The n=1, 2*n > zlell+1 guard
    /// fires for zlell=0/1; for zlell=3+ (`   `), the guard passes
    /// and the function walks the buffer.
    #[test]
    fn selectargument_whitespace_only_buffer_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "   ".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(1, Ordering::SeqCst);
        let _ = selectargument();
    }
}
