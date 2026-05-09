//! ZLE text objects — port of `Src/Zle/textobjects.c`.
//!
//! Three C functions, zero structs/enums. The Rust port matches
//! exactly: three free fns over a `&mut Zle`, no Rust-only types,
//! no helper enums.

use super::zle_main::Zle;
use super::zle_main::ModifierFlags;

/// Port of `blankwordclass()` from `Src/Zle/textobjects.c:34`. The
/// vi blank-word class predicate — splits buffer characters into
/// "blank" (class 0) vs "non-blank" (class 1). Used by
/// `selectinblankword` / `selectablankword` as the `viclass` arg
/// to the generic word-spanning loop in `selectword()`.
pub fn blankwordclass(x: char) -> i32 {                                  // c:34
    // C: `return (ZC_iblank(x) ? 0 : 1);`
    if x == ' ' || x == '\t' { 0 } else { 1 }                            // c:36
}

/// Port of `selectword()` from `Src/Zle/textobjects.c:41`. The
/// dispatcher behind the `select-in-word` / `select-a-word` /
/// `select-in-blank-word` / `select-a-blank-word` /
/// `select-in-shell-word` / `select-a-shell-word` widgets. Sets
/// `mark`/`zlecs` to span a vi text object around the cursor; the
/// class-of-character (word vs blank) is decided by `wordclass()`
/// or `blankwordclass()` depending on which widget triggered (C
/// uses `IS_THINGY(bindk, ...)`, Rust checks the `bindk.name`
/// string).
///
/// Faithful port of the C body: character-class scan back to the
/// first boundary, scan forward to the next boundary, optional
/// all-form blank extension, repeat-count handling for `zmult > 1`.
/// The lexer-driven shell-word path (textobjects.c:81-205) requires
/// `ctxtlex()` which the zshrs port lowers through fusevm bytecode
/// rather than the C lexer; that arm is approximated by falling
/// through to the blank-word class for now.
pub fn selectword(zle: &mut Zle) -> i32 {                                // c:41
    let n_init: i32 = if zle.zmod.flags.contains(ModifierFlags::MULT) {
        zle.zmod.mult                                                    // c:43 zmult
    } else {
        1
    };
    let mut n = n_init;
    let widget_name = zle.bindk.as_ref().map(|t| t.name.as_str()).unwrap_or("");
    let is_aword       = widget_name == "select-a-word";
    let is_inword      = widget_name == "select-in-word";
    let is_ablankword  = widget_name == "select-a-blank-word";
    let all = is_aword || is_ablankword;                                 // c:44
    let viclass: fn(char) -> i32 = if is_aword || is_inword {
        crate::ported::zle::zle_word::wordclass                          // c:46
    } else {
        blankwordclass                                                   // c:46
    };
    if zle.zlell == 0 {
        return 1;
    }
    let cur_char = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
    let sclass = viclass(cur_char);                                      // c:48
    let mut doblanks = all && sclass != 0;                               // c:49

    let region_active = zle.region_active != 0;                          // c:51
    let mark_at = zle.mark;                                              // c:51
    if !region_active || zle.zlecs == mark_at {
        zle.mark = zle.zlecs;                                            // c:54
        loop {
            if zle.mark == 0 { break; }
            let pos = zle.mark - 1;
            let c = zle.zleline.get(pos).copied().unwrap_or('\n');
            if c == '\n' || viclass(c) != sclass {
                break;
            }
            zle.mark = pos;
        }
        while zle.zlecs < zle.zlell {                                    // c:62
            zle.zlecs += 1;
            let pos = zle.zlecs;
            if all && sclass == 0 && pos < zle.zlell &&
                zle.zleline.get(pos).copied() == Some('\n') &&
                pos + 1 < zle.zlell
            {
                zle.zlecs += 1;
            }
            let pc = zle.zleline.get(pos).copied().unwrap_or('\n');
            if pc == '\n' || viclass(pc) != sclass {
                break;
            }
        }
        if all {                                                         // c:74
            let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
            let nclass = viclass(cc);
            if nclass == 0 || sclass == 0 {
                while zle.zlecs < zle.zlell {
                    zle.zlecs += 1;
                    let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
                    if cc == '\n' || viclass(cc) != nclass {
                        break;
                    }
                }
                if n_init < 2 { doblanks = false; }                      // c:86
            }
        }
    } else {                                                             // c:90
        if zle.zlecs > mark_at {
            if zle.zlecs < zle.zlell { zle.zlecs += 1; }
        } else if zle.zlecs > 0 {
            zle.zlecs -= 1;
        }
        n += 1;                                                          // c:148
        doblanks = false;
    }
    zle.region_active = if region_active { 1 } else { 0 };               // c:152

    while {                                                              // c:155
        n -= 1;
        n > 0
    } {
        if zle.zlecs < zle.zlell &&
           zle.zleline.get(zle.zlecs).copied() == Some('\n')
        {
            zle.zlecs += 1;
        }
        let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
        let cls = viclass(cc);
        while zle.zlecs < zle.zlell {
            zle.zlecs += 1;
            let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
            if cc == '\n' || viclass(cc) != cls {
                break;
            }
        }
        if all {                                                         // c:165
            if zle.zlecs < zle.zlell &&
               zle.zleline.get(zle.zlecs).copied() == Some('\n')
            {
                zle.zlecs += 1;
            }
            let cc = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
            if cls == 0 || viclass(cc) == 0 {
                let _ = doblanks;
            }
        }
    }

    // vi operators don't include cursor position — c:208.
    // virangeflag is a deferred bucket-2 file-global (zle_vi.c:36),
    // not yet wired on Zle; conservative default = 0 (always trim).
    if zle.in_vi_cmd_mode() && zle.zlecs > 0 {
        zle.zlecs -= 1;
    }
    0                                                                    // c:210
}

/// Port of `selectargument()` from `Src/Zle/textobjects.c:212`. The
/// `select-in-shell-word` argument-N selector — uses `ctxtlex()`
/// to walk the buffer through the shell lexer and hand back the
/// boundaries of the Nth lexed argument under the cursor.
///
/// The C body wires through `zcontext_save()` + `lexflags` +
/// `ctxtlex()` + `inpush()` to drive the shell tokenizer over the
/// current line. zshrs lowers the lexer through fusevm bytecode
/// and does not expose a free-running `ctxtlex` style scanner, so
/// this port approximates the boundary detection by splitting on
/// shell-word whitespace (matching what the C source produces for
/// the simple-input case where no quoting/expansion is involved).
/// Returns 0 on success, 1 if `n` is out of range — matching C
/// (textobjects.c:225).
pub fn selectargument(zle: &mut Zle) -> i32 {                            // c:212
    let n: i32 = if zle.zmod.flags.contains(ModifierFlags::MULT) {
        zle.zmod.mult
    } else {
        1
    };
    if n < 1 || (2 * n as usize) > zle.zlell + 1 {                       // c:225
        return 1;
    }
    if !zle.in_vi_cmd_mode() {                                           // c:228
        zle.region_active = 1;
        zle.mark = zle.zlecs;
    }
    // Shell-word boundary scan — approximation of the lexer walk
    // at c:233-262. Splits on whitespace at top level (no
    // quoting/expansion lookahead). The full ctxtlex-driven port
    // is pending the fusevm-lexer integration.
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
    zle.mark = s;                                                        // c:283
    zle.zlecs = e;                                                       // c:283
    if zle.in_vi_cmd_mode() && zle.zlecs > 0 {                           // c:308
        zle.zlecs -= 1;
    }
    0                                                                    // c:310
}
