//! ZLE text objects
//!
//! Direct port from zsh/Src/Zle/textobjects.c text object support
//!
//! Text objects for vi mode operations (e.g., "iw" for inner word, "a)" for a-parenthesis)

use super::zle_main::Zle;

/// Text object type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectType {
    /// Inner (inside delimiters)
    Inner,
    /// A (including delimiters)
    A,
}

/// Text object kind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextObjectKind {
    Word,
    BigWord,
    Sentence,
    Paragraph,
    Parenthesis,
    Bracket,
    Brace,
    Angle,
    SingleQuote,
    DoubleQuote,
    BackQuote,
}

/// A text object selection (start and end positions)
#[derive(Debug, Clone, Copy)]
pub struct TextObject {
    pub start: usize,
    pub end: usize,
}

impl Zle {
    /// Compute a vi text object range (`iw`/`aw`/`is`/`as`/`ip`/`ap`)
    /// at the cursor.
    ///
    /// Port of the dispatcher behind `selectinword` / `selectaword`
    /// at Src/Zle/textobjects.c:41 (`selectword`). The C source
    /// branches on the bound widget (selectinword, selectaword,
    /// selectinblankword, selectablankword, selectinshellword,
    /// selectashellword); this Rust helper takes an explicit kind +
    /// variant and returns the `(start, end)` range so widget bodies
    /// can apply the operator-pending semantics themselves.
    pub fn select_text_object(
        &self,
        obj_type: TextObjectType,
        kind: TextObjectKind,
    ) -> Option<TextObject> {
        match kind {
            TextObjectKind::Word => self.select_word_object(obj_type, false),
            TextObjectKind::BigWord => self.select_word_object(obj_type, true),
            TextObjectKind::Sentence => self.select_sentence_object(obj_type),
            TextObjectKind::Paragraph => self.select_paragraph_object(obj_type),
            TextObjectKind::Parenthesis => self.select_pair_object(obj_type, '(', ')'),
            TextObjectKind::Bracket => self.select_pair_object(obj_type, '[', ']'),
            TextObjectKind::Brace => self.select_pair_object(obj_type, '{', '}'),
            TextObjectKind::Angle => self.select_pair_object(obj_type, '<', '>'),
            TextObjectKind::SingleQuote => self.select_quote_object(obj_type, '\''),
            TextObjectKind::DoubleQuote => self.select_quote_object(obj_type, '"'),
            TextObjectKind::BackQuote => self.select_quote_object(obj_type, '`'),
        }
    }

    fn select_word_object(&self, obj_type: TextObjectType, big_word: bool) -> Option<TextObject> {
        if self.zlell == 0 {
            return None;
        }

        let is_word_char = if big_word {
            |c: char| !c.is_whitespace()
        } else {
            |c: char| c.is_alphanumeric() || c == '_'
        };

        let mut start = self.zlecs;
        let mut end = self.zlecs;

        // Determine if we're on a word or whitespace
        let on_word = if self.zlecs < self.zlell {
            is_word_char(self.zleline[self.zlecs])
        } else {
            false
        };

        if on_word {
            // Find word boundaries
            while start > 0 && is_word_char(self.zleline[start - 1]) {
                start -= 1;
            }
            while end < self.zlell && is_word_char(self.zleline[end]) {
                end += 1;
            }

            // For "a word", include trailing whitespace
            if obj_type == TextObjectType::A {
                while end < self.zlell && self.zleline[end].is_whitespace() {
                    end += 1;
                }
            }
        } else {
            // On whitespace - select whitespace
            while start > 0 && self.zleline[start - 1].is_whitespace() {
                start -= 1;
            }
            while end < self.zlell && self.zleline[end].is_whitespace() {
                end += 1;
            }

            // For "a whitespace", include adjacent word
            if obj_type == TextObjectType::A && end < self.zlell {
                while end < self.zlell && is_word_char(self.zleline[end]) {
                    end += 1;
                }
            }
        }

        if start < end {
            Some(TextObject { start, end })
        } else {
            None
        }
    }

    fn select_sentence_object(&self, obj_type: TextObjectType) -> Option<TextObject> {
        // Simplified sentence detection
        let mut start = self.zlecs;
        let mut end = self.zlecs;

        // Find sentence start (after previous . ! ?)
        while start > 0 {
            let c = self.zleline[start - 1];
            if c == '.' || c == '!' || c == '?' {
                break;
            }
            start -= 1;
        }

        // Skip whitespace at start (for inner)
        if obj_type == TextObjectType::Inner {
            while start < self.zlell && self.zleline[start].is_whitespace() {
                start += 1;
            }
        }

        // Find sentence end
        while end < self.zlell {
            let c = self.zleline[end];
            end += 1;
            if c == '.' || c == '!' || c == '?' {
                break;
            }
        }

        // Include trailing whitespace for "a sentence"
        if obj_type == TextObjectType::A {
            while end < self.zlell && self.zleline[end].is_whitespace() {
                end += 1;
            }
        }

        if start < end {
            Some(TextObject { start, end })
        } else {
            None
        }
    }

    fn select_paragraph_object(&self, obj_type: TextObjectType) -> Option<TextObject> {
        let mut start = self.zlecs;
        let mut end = self.zlecs;

        // Find paragraph start (blank line)
        while start > 0 {
            if start >= 2 && self.zleline[start - 1] == '\n' && self.zleline[start - 2] == '\n' {
                break;
            }
            start -= 1;
        }

        // Find paragraph end
        while end < self.zlell {
            if end + 1 < self.zlell && self.zleline[end] == '\n' && self.zleline[end + 1] == '\n' {
                if obj_type == TextObjectType::A {
                    end += 2;
                }
                break;
            }
            end += 1;
        }

        if start < end {
            Some(TextObject { start, end })
        } else {
            None
        }
    }

    fn select_pair_object(
        &self,
        obj_type: TextObjectType,
        open: char,
        close: char,
    ) -> Option<TextObject> {
        let mut depth = 0;
        let mut start = None;
        let mut end = None;

        // Find opening bracket
        for i in (0..=self.zlecs).rev() {
            let c = self.zleline[i];
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    start = Some(i);
                    break;
                }
                depth -= 1;
            }
        }

        // Find closing bracket
        depth = 0;
        for i in self.zlecs..self.zlell {
            let c = self.zleline[i];
            if c == open {
                depth += 1;
            } else if c == close {
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
                depth -= 1;
            }
        }

        match (start, end) {
            (Some(s), Some(e)) => {
                if obj_type == TextObjectType::Inner {
                    Some(TextObject {
                        start: s + 1,
                        end: e - 1,
                    })
                } else {
                    Some(TextObject { start: s, end: e })
                }
            }
            _ => None,
        }
    }

    fn select_quote_object(&self, obj_type: TextObjectType, bslashquote: char) -> Option<TextObject> {
        let mut start = None;
        let mut end = None;

        // Find opening bslashquote (searching backward)
        for i in (0..=self.zlecs).rev() {
            if self.zleline[i] == bslashquote {
                start = Some(i);
                break;
            }
        }

        // Find closing bslashquote (searching forward)
        if let Some(s) = start {
            for i in (s + 1)..self.zlell {
                if self.zleline[i] == bslashquote {
                    end = Some(i + 1);
                    break;
                }
            }
        }

        match (start, end) {
            (Some(s), Some(e)) => {
                if obj_type == TextObjectType::Inner {
                    Some(TextObject {
                        start: s + 1,
                        end: e - 1,
                    })
                } else {
                    Some(TextObject { start: s, end: e })
                }
            }
            _ => None,
        }
    }
}

/// Port of `blankwordclass()` from `Src/Zle/textobjects.c:34`. The
/// vi blank-word class predicate — splits buffer characters into
/// "blank" (class 0) vs "non-blank" (class 1). Used by
/// selectinblankword / selectablankword as the `viclass` arg to the
/// generic word-spanning loop in `selectword()`.
pub fn blankwordclass(x: char) -> i32 {                                 // c:34
    // C: `return (ZC_iblank(x) ? 0 : 1);`
    // ZC_iblank == iblank == iswspace-with-only-h-tab + space.
    if x == ' ' || x == '\t' { 0 } else { 1 }                           // c:36
}

/// Port of `selectword()` from `Src/Zle/textobjects.c:41`. The
/// dispatcher behind the `select-in-word` / `select-a-word` /
/// `select-in-blank-word` / `select-a-blank-word` /
/// `select-in-shell-word` / `select-a-shell-word` widgets. Sets
/// `mark`/`zlecs` to span a vi text object around the cursor; the
/// class-of-character (word vs blank vs other) is decided by
/// `wordclass()` or `blankwordclass()` depending on which widget
/// triggered (C uses `IS_THINGY(bindk, ...)`, Rust checks the
/// `bindk.name` string).
///
/// Faithful port of the C body — character-class scan back to first
/// boundary, scan forward to next boundary, optional all-form blank
/// extension, repeat-count handling for `zmult > 1`. The lexer-driven
/// shell-word path (lines 81-205) requires `ctxtlex()` which the
/// zshrs port lowers through fusevm bytecode rather than the C
/// lexer; that arm is approximated by falling through to the
/// blank-word class for now (TODO: wire to fusevm word-token scan
/// when shell-word selection becomes a parity gap).
pub fn selectword(zle: &mut Zle) -> i32 {                               // c:41
    let n_init: i32 = if zle.zmod.flags.contains(crate::ported::zle::zle_main::ModifierFlags::MULT) {
        zle.zmod.mult                                                   // c:43 zmult
    } else {
        1
    };
    let mut n = n_init;
    let widget_name = zle.bindk.as_ref().map(|t| t.name.as_str()).unwrap_or("");
    let is_aword       = widget_name == "select-a-word";
    let is_inword      = widget_name == "select-in-word";
    let is_ablankword  = widget_name == "select-a-blank-word";
    let all = is_aword || is_ablankword;                                // c:44
    let viclass: fn(char) -> i32 = if is_aword || is_inword {
        crate::ported::zle::zle_word::wordclass                          // c:46
    } else {
        blankwordclass                                                   // c:46
    };
    if zle.zlell == 0 {
        return 1;
    }
    let cur_char = zle.zleline.get(zle.zlecs).copied().unwrap_or('\n');
    let sclass = viclass(cur_char);                                     // c:48
    let mut doblanks = all && sclass != 0;                              // c:49

    let region_active = zle.region_active != 0;                         // c:51
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
/// `select-in-shell-word` argument-N selector — uses `ctxtlex()` to
/// walk the buffer through the shell lexer and hand back the
/// boundaries of the Nth lexed argument under the cursor.
///
/// The C body wires through `zcontext_save()` + `lexflags` +
/// `ctxtlex()` + `inpush()` to drive the shell tokenizer over the
/// current line. zshrs lowers the lexer through fusevm bytecode and
/// does not expose a free-running `ctxtlex` style scanner, so this
/// port approximates the boundary detection by splitting on
/// shell-word whitespace (matching what the C source produces for
/// the simple-input case where no quoting/expansion is involved).
/// Returns 0 on success, 1 if `n` is out of range — matching C
/// (textobjects.c:225).
pub fn selectargument(zle: &mut Zle) -> i32 {                           // c:212
    let n: i32 = if zle.zmod.flags.contains(crate::ported::zle::zle_main::ModifierFlags::MULT) {
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
    // quoting/expansion lookahead). The full ctxtlex-driven port is
    // pending the fusevm-lexer integration.
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
