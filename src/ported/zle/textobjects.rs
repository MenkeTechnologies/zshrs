//! ZLE text objects — port of `Src/Zle/textobjects.c`.
//!
//! Three C functions, zero structs/enums. The Rust port matches:
//! three free fns over a `&mut Zle`, no Rust-only types.

use super::zle_main::Zle;
use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};

/// Port of `blankwordclass()` from `Src/Zle/textobjects.c:34`. The
/// vi blank-word class predicate. Returns 0 for blanks, 1 otherwise.
pub fn blankwordclass(x: char) -> i32 {                                  // c:34
    // C: `return (ZC_iblank(x) ? 0 : 1);`
    if x == ' ' || x == '\t' { 0 } else { 1 }                            // c:36
}

/// Port of `selectword()` from `Src/Zle/textobjects.c:41`.
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
/// flag during the live `vi`-operator-pending key-read loop on
/// the Zle struct; standalone widget invocation reaches this fn
/// with the flag clear, which is the only state the cursor-
/// adjustment needs to handle (the `range`-set branch only fires
/// from inside `getvirange`, which has its own copy).
pub fn selectword(zle: &mut Zle) -> i32 {                                // c:41
    let mut n: i32 = if zle.zmod.flags & MOD_MULT != 0 {   // c:42 zmult
        zle.zmod.mult
    } else {
        1
    };
    let widget = zle.bindk.as_ref().map(|t| t.nam.as_str()).unwrap_or("");
    let is_aword       = widget == "select-a-word";
    let is_inword      = widget == "select-in-word";
    let is_ablankword  = widget == "select-a-blank-word";
    let mut all: i32 = (is_aword || is_ablankword) as i32;               // c:43-44
    let viclass: fn(char) -> i32 = if is_aword || is_inword {
        crate::ported::zle::zle_word::wordclass                          // c:46-47
    } else {
        blankwordclass
    };
    if zle.zlell == 0 {
        return 1;
    }
    let cur = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
    let mut sclass: i32 = viclass(cur);                                  // c:48
    let mut doblanks: i32 = all & ((sclass != 0) as i32);                // c:49 all && sclass

    let region_active = zle.region_active != 0;                          // c:51 (read once)

    // C's `mark == -1` sentinel doesn't exist in the Rust port (mark
    // is `usize`); the equivalent "mark is unset" condition collapses
    // into `!region_active` since mark is only meaningful when the
    // region is active. Drop the `mark == -1` disjunct.
    if !region_active || zle.zlecs == zle.mark {                         // c:51
        // search back to first character of same class as the start
        // position; also stop at the beginning of the line.
        zle.mark = zle.zlecs;                                            // c:54
        while zle.mark != 0 {                                            // c:55
            let pos = zle.mark - 1;                                      // c:56-57 DECPOS
            let cp = zle.zleline.get(pos).copied().unwrap_or('\n');
            if cp == '\n' || viclass(cp) != sclass {                     // c:58
                break;                                                   // c:59
            }
            zle.mark = pos;                                              // c:60
        }
        // similarly scan forward over characters of the same class.
        while zle.zlecs < zle.zlell {                                    // c:63
            zle.zlecs += 1;                                              // c:64 INCCS
            let mut pos = zle.zlecs;                                     // c:65
            // single newlines within blanks are included.
            if all != 0 && sclass == 0 && pos < zle.zlell                // c:67
                && zle.zleline.get(pos).copied() == Some('\n')
            {
                pos += 1;                                                // c:68 INCPOS(pos)
            }
            let pc = zle.zleline.get(pos).copied().unwrap_or('\n');
            if pc == '\n' || viclass(pc) != sclass {                     // c:70
                break;                                                   // c:71
            }
        }

        if all != 0 {                                                    // c:74
            let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
            let nclass = viclass(cc);                                    // c:75
            // if either start or new position is blank advance over a
            // new block of characters of a common type.
            if nclass == 0 || sclass == 0 {                              // c:78
                while zle.zlecs < zle.zlell {                            // c:79
                    zle.zlecs += 1;                                      // c:80 INCCS
                    let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
                    if cc == '\n' || viclass(cc) != nclass {             // c:81
                        break;                                           // c:82
                    }
                }
                if n < 2 {                                               // c:85
                    doblanks = 0;                                        // c:86
                }
            }
        }
    } else {                                                             // c:89
        // For visual mode, advance one char so repeated invocations
        // select subsequent words.
        if zle.zlecs > zle.mark {                                        // c:92
            if zle.zlecs < zle.zlell {                                   // c:93
                zle.zlecs += 1;                                          // c:94 INCCS
            }
        } else if zle.zlecs != 0 {                                       // c:95
            zle.zlecs -= 1;                                              // c:96 DECCS
        }
        if zle.zlecs < zle.mark {                                        // c:97
            // visual mode with the cursor before the mark: move
            // cursor back.
            while {
                let cont = n > 0;
                n -= 1;
                cont
            } {                                                          // c:99 while (n-- > 0)
                let mut pos = zle.zlecs;                                 // c:100
                let zc_pos = zle.zleline.get(pos).copied().unwrap_or('\n');
                // first over blanks
                if all != 0 && (viclass(zc_pos) == 0 || zc_pos == '\n') {  // c:102
                    all = 0;                                             // c:104
                    while pos != 0 {                                     // c:105
                        pos -= 1;                                        // c:106 DECPOS
                        let pc = zle.zleline.get(pos).copied().unwrap_or('\n');
                        if pc == '\n' {                                  // c:107
                            break;                                       // c:108
                        }
                        zle.zlecs = pos;                                 // c:109
                        if viclass(pc) != 0 {                            // c:110
                            break;                                       // c:111
                        }
                    }
                } else if zle.zlecs != 0
                    && zle.zleline.get(zle.zlecs).copied() == Some('\n')
                {                                                        // c:114
                    // for 'in' widgets pass over one newline
                    pos -= 1;                                            // c:116 DECPOS(pos)
                    let pc = zle.zleline.get(pos).copied().unwrap_or('\n');
                    if pc != '\n' {                                      // c:117
                        zle.zlecs = pos;                                 // c:118
                    }
                }
                pos = zle.zlecs;                                         // c:121
                let cur = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
                sclass = viclass(cur);                                   // c:122
                // now retreat over non-blanks
                loop {                                                   // c:124
                    let pc = zle.zleline.get(pos).copied().unwrap_or('\n');
                    if pc == '\n' || viclass(pc) != sclass {
                        break;
                    }
                    zle.zlecs = pos;                                     // c:126
                    if pos == 0 {                                        // c:127
                        zle.zlecs = 0;                                   // c:128
                        break;                                           // c:129
                    }
                    pos -= 1;                                            // c:131 DECPOS
                }
                // blanks again but only if there were none first time
                if all != 0 && zle.zlecs != 0 {                          // c:134
                    pos = zle.zlecs;
                    pos -= 1;                                            // c:136 DECPOS
                    let pc = zle.zleline.get(pos).copied().unwrap_or('\n');
                    if viclass(pc) == 0 {                                // c:137
                        while pos != 0 {                                 // c:138
                            pos -= 1;                                    // c:139 DECPOS
                            let pc = zle.zleline.get(pos).copied().unwrap_or('\n');
                            if pc == '\n' || viclass(pc) != 0 {          // c:140
                                break;                                   // c:142
                            }
                            zle.zlecs = pos;                             // c:143
                        }
                    }
                }
            }
            return 0;                                                    // c:147
        }
        n += 1;                                                          // c:148
        doblanks = 0;                                                    // c:149
    }
    // force to character-wise — c:152
    zle.region_active = if region_active { 1 } else { 0 };

    // for each digit argument, advance over a further block of one class
    while {
        n -= 1;
        n > 0
    } {                                                                  // c:155
        if zle.zlecs < zle.zlell
            && zle.zleline.get(zle.zlecs).copied() == Some('\n')
        {                                                                // c:156
            zle.zlecs += 1;                                              // c:157 INCCS
        }
        let cur = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
        sclass = viclass(cur);                                           // c:158
        while zle.zlecs < zle.zlell {                                    // c:159
            zle.zlecs += 1;                                              // c:160 INCCS
            let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
            if cc == '\n' || viclass(cc) != sclass {                     // c:161
                break;                                                   // c:163
            }
        }
        // for 'a' widgets, advance extra block if either consists of blanks
        if all != 0 {                                                    // c:165
            if zle.zlecs < zle.zlell
                && zle.zleline.get(zle.zlecs).copied() == Some('\n')
            {                                                            // c:166
                zle.zlecs += 1;                                          // c:167 INCCS
            }
            let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
            let cls_here = viclass(cc);
            if sclass == 0 || cls_here == 0 {                            // c:168
                sclass = cls_here;                                       // c:169
                if n == 1 && sclass == 0 {                               // c:170
                    doblanks = 0;                                        // c:171
                }
                while zle.zlecs < zle.zlell {                            // c:172
                    zle.zlecs += 1;                                      // c:173 INCCS
                    let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
                    if cc == '\n' || viclass(cc) != sclass {             // c:174
                        break;                                           // c:176
                    }
                }
            }
        }
    }

    // if we didn't remove blanks at either end we remove some at the start
    if doblanks != 0 {                                                   // c:181
        let mut pos = zle.mark;                                          // c:182
        while pos != 0 {                                                 // c:183
            pos -= 1;                                                    // c:184 DECPOS
            // don't remove blanks at the start of the line, i.e. indentation
            let pc = zle.zleline.get(pos).copied().unwrap_or('\n');
            if pc == '\n' {                                              // c:186
                break;                                                   // c:187
            }
            if !(pc == ' ' || pc == '\t') {                              // c:188 !ZC_iblank
                pos += 1;                                                // c:189 INCPOS
                zle.mark = pos;                                          // c:190
                break;                                                   // c:191
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
    let virangeflag = crate::ported::zle::zle_vi::VIRANGEFLAG
        .load(std::sync::atomic::Ordering::Relaxed) != 0;
    if !virangeflag {                                                    // c:196
        if !zle.in_vi_cmd_mode() {                                       // c:197
            zle.region_active = 1;                                       // c:198
        } else if zle.zlecs != 0 && zle.zlecs > zle.mark {               // c:199
            zle.zlecs -= 1;                                              // c:200 DECCS
        }
    }

    0                                                                    // c:204
}

/// Port of `selectargument()` from `Src/Zle/textobjects.c:212`.
///
/// **NOT FULLY PORTED — see TODO.md.** The C body uses the shell's
/// `ctxtlex()` lexer-walk machinery (textobjects.c:233-257) to drive
/// real shell tokenisation over the buffer. zshrs lowers the lexer
/// through fusevm bytecode and does not expose a free-running
/// `ctxtlex` style scanner; a faithful port is blocked on porting
/// the lexer-context save/restore + standalone tokenisation API
/// from `Src/lex.c`.
///
/// Until that's done, this entry is a whitespace-split approximation
/// that handles only the simple case (no quoting, no expansion, no
/// here-docs). Returns 1 when `n` is out of range matching C
/// (textobjects.c:225).
pub fn selectargument(zle: &mut Zle) -> i32 {                            // c:212
    let n: i32 = if zle.zmod.flags & MOD_MULT != 0 {
        zle.zmod.mult                                                    // c:222 zmult
    } else {
        1
    };
    if n < 1 || (2 * n as usize) > zle.zlell + 1 {                       // c:225
        return 1;
    }
    if !zle.in_vi_cmd_mode() {                                           // c:228
        zle.region_active = 1;                                           // c:229
        zle.mark = zle.zlecs;                                            // c:230
    }
    // Approximation pending ctxtlex port — see TODO.md.
    let mut starts: Vec<usize> = Vec::with_capacity(n as usize);
    let mut in_word = false;
    let mut word_start = 0usize;
    starts.push(0);
    for (i, &c) in zle.zleline.iter().enumerate() {
        if c.is_whitespace() {
            if in_word {
                in_word = false;
                if starts.len() < n as usize { starts.push(i + 1); }
            }
        } else if !in_word {
            in_word = true;
            word_start = i;
            if i >= zle.zlecs { break; }
        }
    }
    let arg_idx = (n - 1) as usize;
    let s = starts.get(arg_idx).copied().unwrap_or(word_start);
    let e = (s..zle.zlell)
        .find(|&i| zle.zleline.get(i).copied().map_or(true, |c| c.is_whitespace()))
        .unwrap_or(zle.zlell);
    zle.mark = s;
    zle.zlecs = e;
    if zle.in_vi_cmd_mode() && zle.zlecs > 0 {
        zle.zlecs -= 1;
    }
    0
}
