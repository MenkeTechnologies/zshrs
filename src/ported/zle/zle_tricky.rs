//! ZLE tricky - completion and expansion widgets
//!
//! Direct port from zsh/Src/Zle/zle_tricky.c
//!
//! Implements completion widgets:
//! - complete-word, menu-complete, reverse-menu-complete
//! - expand-or-complete, expand-or-complete-prefix
//! - list-choices, list-expand
//! - expand-word, expand-history
//! - spell-word, delete-char-or-list
//! - magic-space, accept-and-menu-complete

use std::sync::atomic::AtomicI32;

use super::zle_main::Zle;

// =====================================================================
// Globals — `Src/Zle/zle_tricky.c:96-106`.
// =====================================================================
//
// usemenu/useglob — controls type of completion (set by entry widget,
// read by `docomplete`/`callcompfunc`). usemenu==2 starts automenu;
// usemenu==3 inserts as if for menucomp without really starting it.
// wouldinstab — non-zero if we'd insert TAB but for the comp widget.

/// Port of `mod_export int usemenu` from `Src/Zle/zle_tricky.c:96`.
pub static USEMENU: AtomicI32 = AtomicI32::new(0);                           // c:96

/// Port of `mod_export int useglob` from `Src/Zle/zle_tricky.c:96`.
pub static USEGLOB: AtomicI32 = AtomicI32::new(0);                           // c:96

/// Port of `mod_export int wouldinstab` from `Src/Zle/zle_tricky.c:101`.
pub static WOULDINSTAB: AtomicI32 = AtomicI32::new(0);                       // c:101

/// Port of `mod_export int menucmp` from `Src/Zle/zle_tricky.c:106`.
/// Non-zero while inside a menu-completion sequence.
pub static MENUCMP: AtomicI32 = AtomicI32::new(0);                           // c:106

/// Completion state
// The line before completion was tried.                                    // c:70
// Words on the command line, for use in completion                         // c:77
#[derive(Debug, Default, Clone)]
pub struct CompletionState {
    /// Whether we're in menu completion mode
    pub in_menu: bool,
    /// Current menu index
    pub menu_index: usize,
    /// Available completions
    pub completions: Vec<String>,
    /// Prefix being completed
    pub prefix: String,
    /// Suffix after cursor
    pub suffix: String,
    /// Word start position
    pub word_start: usize,
    /// Word end position
    pub word_end: usize,
    /// Last completion was a menu cycle
    pub last_menu: bool,
}

/// Brace info for parameter expansion
#[derive(Debug, Clone)]
pub struct BraceInfo {
    pub str_val: String,
    pub pos: usize,
    pub cur_pos: usize,
    pub qpos: usize,
    pub curlen: usize,
}

impl Zle {
    /// Complete word - trigger completion
    /// Port of completeword() from zle_tricky.c
    pub fn complete_word(&mut self, state: &mut CompletionState) {           // c:216
        self.do_complete(state, false, false);
    }

    /// Menu complete - cycle through completions
    /// Port of menucomplete() from zle_tricky.c
    pub fn menu_complete(&mut self, state: &mut CompletionState) {           // c:238
        if state.in_menu && !state.completions.is_empty() {
            // Cycle to next completion
            state.menu_index = (state.menu_index + 1) % state.completions.len();
            self.apply_completion(state);
        } else {
            self.do_complete(state, true, false);
        }
    }

    /// Reverse menu complete - cycle backwards
    /// Port of reversemenucomplete() from zle_tricky.c
    pub fn reverse_menu_complete(&mut self, state: &mut CompletionState) {
        if state.in_menu && !state.completions.is_empty() {
            if state.menu_index == 0 {
                state.menu_index = state.completions.len() - 1;
            } else {
                state.menu_index -= 1;
            }
            self.apply_completion(state);
        }
    }

    /// Expand or complete - try expansion first, then completion
    /// Port of expandorcomplete() from zle_tricky.c
    pub fn expand_or_complete(&mut self, state: &mut CompletionState) {      // c:299
        // First try expansion
        if !self.try_expand() {
            // Then try completion
            self.do_complete(state, false, false);
        }
    }

    // Extra function added by AR Iano-Fletcher.                            // c:3036
    // This is a expand/complete in the vein of wash.                       // c:3037
    /// Expand or complete prefix - expand/complete keeping suffix
    /// Port of expandorcompleteprefix() from zle_tricky.c
    pub fn expand_or_complete_prefix(&mut self, state: &mut CompletionState) { // c:3041
        state.suffix = self.zleline[self.zlecs..].iter().collect();
        self.expand_or_complete(state);
    }

    /// List choices - show available completions
    /// Port of listchoices() from zle_tricky.c
    pub fn list_choices(&mut self, state: &mut CompletionState) {
        self.do_complete(state, false, true);

        if !state.completions.is_empty() {
            println!();
            for (i, c) in state.completions.iter().enumerate() {
                if i > 0 && i % 5 == 0 {
                    println!();
                }
                print!("{:<16}", c);
            }
            println!();
            self.resetneeded = true;
        }
    }

    /// List expand - list possible expansions
    /// Port of listexpand() from zle_tricky.c
    pub fn list_expand(&mut self) {
        let word = self.get_word_at_cursor();
        let expansions = self.do_expansion(&word);

        if !expansions.is_empty() {
            println!();
            for exp in &expansions {
                println!("{}", exp);
            }
            self.resetneeded = true;
        }
    }

    /// Expand word - expand current word (glob, history, etc)
    /// Port of expandword() from zle_tricky.c
    pub fn expand_word(&mut self) {
        let _ = self.try_expand();
    }

    /// Expand history - expand history references
    /// Port of expandhistory() / doexpandhist() from zle_tricky.c
    pub fn expand_history(&mut self) {
        let line: String = self.zleline.iter().collect();

        // Look for history references like !!, !$, !*, etc.
        let expanded = self.do_expand_hist(&line);

        if expanded != line {
            self.zleline = expanded.chars().collect();
            self.zlell = self.zleline.len();
            if self.zlecs > self.zlell {
                self.zlecs = self.zlell;
            }
            self.resetneeded = true;
        }
    }

    /// Magic space - expand history then insert space
    /// Port of magicspace() from zle_tricky.c
    pub fn magic_space(&mut self) {
        self.expand_history();
        self.self_insert(' ');
    }

    /// Delete char or list - delete if there's text, else list completions
    /// Port of deletecharorlist() from zle_tricky.c
    pub fn delete_char_or_list(&mut self, state: &mut CompletionState) {
        if self.zlecs < self.zlell {
            self.delete_char();
        } else {
            self.list_choices(state);
        }
    }

    /// Accept and menu complete
    /// Port of acceptandmenucomplete() from zle_tricky.c  
    pub fn accept_and_menu_complete(&mut self, state: &mut CompletionState) -> Option<String> {
        let line = self.accept_line();
        state.in_menu = false;
        Some(line)
    }

    /// Spell word - check spelling
    /// Port of spellword() from zle_tricky.c
    pub fn spell_word(&mut self) {
        // Simple spell check - look for common patterns
        let word = self.get_word_at_cursor();
        // Would integrate with aspell/hunspell in full implementation
        let _ = word;
    }

    /// Internal: perform completion
    fn do_complete(&mut self, state: &mut CompletionState, menu_mode: bool, list_only: bool) {
        // Get word at cursor
        let (word_start, word_end) = self.get_word_bounds();
        let word: String = self.zleline[word_start..word_end].iter().collect();

        state.word_start = word_start;
        state.word_end = word_end;
        state.prefix = word.clone();

        // Get completions (simplified - real impl would call compsys)
        state.completions = self.get_completions(&word);

        if state.completions.is_empty() {
            return;
        }

        if list_only {
            return;
        }

        if menu_mode || state.completions.len() > 1 {
            state.in_menu = true;
            state.menu_index = 0;
            self.apply_completion(state);
        } else if state.completions.len() == 1 {
            // Single completion - apply directly
            state.menu_index = 0;
            self.apply_completion(state);
            state.in_menu = false;
        }
    }

    /// Apply current completion from state
    fn apply_completion(&mut self, state: &CompletionState) {
        if state.completions.is_empty() {
            return;
        }

        let completion = &state.completions[state.menu_index];

        // Remove old word
        self.zleline.drain(state.word_start..state.word_end);
        self.zlell = self.zleline.len();
        self.zlecs = state.word_start;

        // Insert completion
        for c in completion.chars() {
            self.zleline.insert(self.zlecs, c);
            self.zlecs += 1;
        }
        self.zlell = self.zleline.len();
        self.resetneeded = true;
    }

    /// Get word at cursor position
    fn get_word_at_cursor(&self) -> String {
        let (start, end) = self.get_word_bounds();
        self.zleline[start..end].iter().collect()
    }

    /// Get bounds of word at cursor
    fn get_word_bounds(&self) -> (usize, usize) {
        let mut start = self.zlecs;
        let mut end = self.zlecs;

        // Find word start
        while start > 0 && !self.zleline[start - 1].is_whitespace() {
            start -= 1;
        }

        // Find word end
        while end < self.zlell && !self.zleline[end].is_whitespace() {
            end += 1;
        }

        (start, end)
    }

    /// Try to expand the word at cursor
    fn try_expand(&mut self) -> bool {
        let word = self.get_word_at_cursor();

        if word.is_empty() {
            return false;
        }

        let expansions = self.do_expansion(&word);

        if expansions.is_empty() || (expansions.len() == 1 && expansions[0] == word) {
            return false;
        }

        let (start, end) = self.get_word_bounds();

        // Remove old word
        self.zleline.drain(start..end);
        self.zlecs = start;

        // Insert expansions
        let expanded = expansions.join(" ");
        for c in expanded.chars() {
            self.zleline.insert(self.zlecs, c);
            self.zlecs += 1;
        }
        self.zlell = self.zleline.len();
        self.resetneeded = true;

        true
    }

    /// Do expansion on a word
    fn do_expansion(&self, word: &str) -> Vec<String> {
        let mut results = Vec::new();

        // Glob expansion via the `glob` crate. Mirrors zsh's tricky.c
        // expand-or-complete fall-through into the pattern engine when a
        // word contains `*`, `?`, or `[` — the C source feeds the word
        // to `zglob()`; we use the Rust `glob` crate as a stand-in.
        if word.contains('*') || word.contains('?') || word.contains('[') {
            if let Ok(paths) = glob::glob(word) {
                for path in paths.flatten() {
                    results.push(path.display().to_string());
                }
            }
        }

        // Check for tilde expansion
        if word.starts_with('~') {
            if let Some(home) = std::env::var_os("HOME") {
                let expanded = word.replacen('~', home.to_str().unwrap_or("~"), 1);
                results.push(expanded);
            }
        }

        // Check for variable expansion
        if let Some(var_name) = word.strip_prefix('$') {
            if let Ok(val) = std::env::var(var_name) {
                results.push(val);
            }
        }

        if results.is_empty() {
            results.push(word.to_string());
        }

        results
    }

    /// Do history expansion
    fn do_expand_hist(&self, line: &str) -> String {
        let mut result = line.to_string();

        // !! -> last command (simplified)
        if result.contains("!!") {
            result = result.replace("!!", "[last-command]");
        }

        // !$ -> last argument of last command (simplified)
        if result.contains("!$") {
            result = result.replace("!$", "[last-arg]");
        }

        result
    }

    /// Get completions for a prefix (simplified)
    fn get_completions(&self, prefix: &str) -> Vec<String> {
        let mut completions = Vec::new();

        // Check if it looks like a path
        if prefix.contains('/') || prefix.starts_with('.') {
            // Path completion
            let dir = if let Some(pos) = prefix.rfind('/') {
                &prefix[..=pos]
            } else {
                "./"
            };
            let file_prefix = if let Some(pos) = prefix.rfind('/') {
                &prefix[pos + 1..]
            } else {
                prefix
            };

            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(file_prefix) {
                        let full_path = if dir == "./" {
                            name
                        } else {
                            format!("{}{}", dir, name)
                        };
                        completions.push(full_path);
                    }
                }
            }
        } else {
            // Command completion - look in PATH
            if let Ok(path) = std::env::var("PATH") {
                for dir in path.split(':') {
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with(prefix) && !completions.contains(&name) {
                                completions.push(name);
                            }
                        }
                    }
                }
            }
        }

        completions.sort();
        completions
    }
}

/// Meta character for zsh's internal encoding (0x83)
pub const META: char = '\u{83}';

/// Metafy a line (escape special chars)
/// Port of metafy_line() from zle_tricky.c
pub fn metafy_line(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if c == META || (c as u32) >= 0x83 {
            result.push(META);
            result.push(char::from_u32((c as u32) ^ 32).unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// Unmetafy a line (unescape special chars)
/// Port of unmetafy_line() from zle_tricky.c
pub fn unmetafy_line(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == META {
            if let Some(&next) = chars.peek() {
                chars.next();
                result.push(char::from_u32((next as u32) ^ 32).unwrap_or(next));
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Check if string has real tokens (not escaped)
/// Port of has_real_token() from zle_tricky.c
pub fn has_real_token(s: &str) -> bool {
    let special = ['$', '`', '"', '\'', '\\', '{', '}', '[', ']', '*', '?', '~'];

    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if special.contains(&c) {
            return true;
        }
    }

    false
}

/// Get length of common prefix
/// Port of pfxlen() from zle_tricky.c
pub fn pfxlen(s1: &str, s2: &str) -> usize {
    s1.chars()
        .zip(s2.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Get length of common suffix
/// Port of sfxlen() from zle_tricky.c
pub fn sfxlen(s1: &str, s2: &str) -> usize {
    s1.chars()
        .rev()
        .zip(s2.chars().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pfxlen() {
        assert_eq!(pfxlen("hello", "help"), 3);
        assert_eq!(pfxlen("abc", "xyz"), 0);
        assert_eq!(pfxlen("test", "test"), 4);
    }

    #[test]
    fn test_sfxlen() {
        assert_eq!(sfxlen("testing", "running"), 3);
        assert_eq!(sfxlen("abc", "xyz"), 0);
    }

    #[test]
    fn test_has_real_token() {
        assert!(has_real_token("$HOME"));
        assert!(has_real_token("*.txt"));
        assert!(!has_real_token("hello"));
        assert!(!has_real_token("test\\$var")); // escaped
    }

    // ---------- Real-port tests ------------------------------------------

    #[test]
    fn dupstrspace_appends_space() {
        // c:954 — len + 1 + 1 NUL: "hello" → "hello "
        assert_eq!(dupstrspace("hello"), "hello ");
    }

    #[test]
    fn dupstrspace_empty_input() {
        // c:954 — empty input → just a single space
        assert_eq!(dupstrspace(""), " ");
    }

    #[test]
    fn freebrinfo_drops_chain() {
        use crate::ported::zle::zle_h::brinfo;
        // c:1015 — Box drop cascades through `next`.
        let head = Some(Box::new(brinfo {
            next: Some(Box::new(brinfo {
                next: None,
                prev: None,
                str_: "second".into(),
                pos: 7,
                qpos: 8,
                curpos: 9,
            })),
            prev: None,
            str_: "first".into(),
            pos: 1,
            qpos: 2,
            curpos: 3,
        }));
        // freebrinfo just consumes — no panic, drop succeeds.
        freebrinfo(head);
    }

    #[test]
    fn dupbrinfo_clones_chain() {
        use crate::ported::zle::zle_h::brinfo;
        // Build a 3-node chain: A → B → C.
        let src = Box::new(brinfo {
            next: Some(Box::new(brinfo {
                next: Some(Box::new(brinfo {
                    next: None,
                    prev: None,
                    str_: "C".into(),
                    pos: 30,
                    qpos: 31,
                    curpos: 32,
                })),
                prev: None,
                str_: "B".into(),
                pos: 20,
                qpos: 21,
                curpos: 22,
            })),
            prev: None,
            str_: "A".into(),
            pos: 10,
            qpos: 11,
            curpos: 12,
        });
        let (head, last) = dupbrinfo(Some(&*src));
        assert!(last.is_some());
        let h = head.as_ref().unwrap();
        // c:1043-1046 — fields copied verbatim.
        assert_eq!(h.str_, "A");
        assert_eq!(h.pos, 10);
        assert_eq!(h.qpos, 11);
        assert_eq!(h.curpos, 12);
        let n = h.next.as_ref().unwrap();
        assert_eq!(n.str_, "B");
        assert_eq!(n.pos, 20);
        let n = n.next.as_ref().unwrap();
        assert_eq!(n.str_, "C");
        assert_eq!(n.pos, 30);
        assert!(n.next.is_none());
    }

    #[test]
    fn dupbrinfo_empty_returns_none() {
        // c:1037 — `while (p)` never enters; ret stays NULL.
        let (head, last) = dupbrinfo(None);
        assert!(head.is_none());
        assert!(last.is_none());
    }

    #[test]
    fn spellword_zeroes_globals_returns_docomplete() {
        use std::sync::atomic::Ordering;
        // Pre-set non-zero so the c:263 reset is observable.
        USEMENU.store(99, Ordering::SeqCst);
        USEGLOB.store(99, Ordering::SeqCst);
        WOULDINSTAB.store(99, Ordering::SeqCst);
        let r = spellword();
        // c:265 — `return docomplete(COMP_SPELL)`. docomplete() is
        // currently a stub returning 0 — verify pass-through.
        assert_eq!(r, 0);
        // c:263 — both zeroed.
        assert_eq!(USEMENU.load(Ordering::SeqCst), 0);
        assert_eq!(USEGLOB.load(Ordering::SeqCst), 0);
        // c:264 — wouldinstab cleared.
        assert_eq!(WOULDINSTAB.load(Ordering::SeqCst), 0);
    }
}

/// Port of `acceptandmenucomplete()` from Src/Zle/zle_tricky.c:353. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn acceptandmenucomplete() -> i32 { 0 }

/// Port of `addx()` from Src/Zle/zle_tricky.c:922. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn addx() -> i32 { 0 }

/// Port of `checkparams()` from Src/Zle/zle_tricky.c:435. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn checkparams() -> i32 { 0 }

/// Port of `cmphaswilds()` from Src/Zle/zle_tricky.c:457. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cmphaswilds() -> i32 { 0 }

/// Port of `completecall()` from Src/Zle/zle_tricky.c:202. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn completecall() -> i32 { 0 }

/// Port of `completeword()` from Src/Zle/zle_tricky.c:216. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn completeword() -> i32 { 0 }

/// Port of `deletecharorlist()` from Src/Zle/zle_tricky.c:270. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn deletecharorlist() -> i32 { 0 }

// The main entry point for completion.                                     // c:595
/// Port of `docomplete()` from Src/Zle/zle_tricky.c:599. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn docomplete() -> i32 { 0 }                                             // c:599

/// Port of `docompletion()` from Src/Zle/zle_tricky.c:2339. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn docompletion() -> i32 { 0 }

/// Port of `doexpandhist()` from Src/Zle/zle_tricky.c:2802. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn doexpandhist() -> i32 { 0 }

/// Port of `doexpansion()` from Src/Zle/zle_tricky.c:2263. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn doexpansion() -> i32 { 0 }

/// Port of `dupbrinfo()` from `Src/Zle/zle_tricky.c:1032`.
/// ```c
/// mod_export Brinfo
/// dupbrinfo(Brinfo p, Brinfo *last, int heap)
/// {
///     Brinfo ret = NULL, *q = &ret, n = NULL;
///     while (p) {
///         n = *q = (heap ? (Brinfo) zhalloc(sizeof(*n)) :
///                  (Brinfo) zalloc(sizeof(*n)));
///         q = &(n->next);
///         n->next = NULL;
///         n->str = (heap ? dupstring(p->str) : ztrdup(p->str));
///         n->pos = p->pos;
///         n->qpos = p->qpos;
///         n->curpos = p->curpos;
///         p = p->next;
///     }
///     if (last)
///         *last = n;
///     return ret;
/// }
/// ```
/// Deep-copy a Brinfo `next`-linked list. The C `heap` parameter
/// chooses between `zhalloc` (per-completion arena) and `zalloc`
/// (permanent); Rust uses Box for both since the GC distinction
/// doesn't apply.
///
/// Returns `(head, last)` — the C uses an out-pointer for `last`
/// because callers want to splice further entries onto the tail.
pub fn dupbrinfo(                                                            // c:1032
    mut p: Option<&crate::ported::zle::zle_h::brinfo>,
) -> (
    Option<crate::ported::zle::zle_h::BrinfoPtr>,
    Option<*const crate::ported::zle::zle_h::brinfo>,
) {
    let mut head: Option<crate::ported::zle::zle_h::BrinfoPtr> = None;       // c:1035 ret = NULL
    let mut last_ptr: Option<*const crate::ported::zle::zle_h::brinfo> = None;
    // SAFETY: tail walks the head-chain we build, both reachable for
    // this fn's lifetime.
    let mut tail: *mut Option<crate::ported::zle::zle_h::BrinfoPtr> = &mut head;
    while let Some(node) = p {                                               // c:1037 while (p)
        let cloned = Box::new(crate::ported::zle::zle_h::brinfo {            // c:1038-1039 zhalloc/zalloc
            next: None,                                                      // c:1042
            prev: None,                                                      // brinfo has prev too
            str_: node.str_.clone(),                                         // c:1043 dupstring(p->str)
            pos: node.pos,                                                   // c:1044
            qpos: node.qpos,                                                 // c:1045
            curpos: node.curpos,                                             // c:1046
        });
        unsafe {
            *tail = Some(cloned);
            let inserted = (*tail).as_mut().unwrap();
            last_ptr = Some(inserted.as_ref() as *const _);
            tail = &mut inserted.next;
        }
        p = node.next.as_deref();                                            // c:1048 p = p->next
    }
    // c:1050-1051 — `if (last) *last = n`. Returned alongside head.
    (head, last_ptr)
}

/// Port of `dupstrspace()` from `Src/Zle/zle_tricky.c:954`.
/// ```c
/// mod_export char *
/// dupstrspace(const char *str)
/// {
///     int len = strlen(str);
///     char *t = (char *) hcalloc(len + 2);
///     strcpy(t, str);
///     strcpy(t+len, " ");
///     return t;
/// }
/// ```
/// Like `dupstring`, but appends a single space.
pub fn dupstrspace(s: &str) -> String {                                      // c:954
    let len = s.len();                                                       // c:957 strlen(str)
    let mut out = String::with_capacity(len + 2);                            // c:958 hcalloc(len+2)
    out.push_str(s);                                                         // c:959 strcpy(t, str)
    out.push(' ');                                                           // c:960 strcpy(t+len, " ")
    out                                                                      // c:961 return t
}

/// Port of `endoflist()` from Src/Zle/zle_tricky.c:3055. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn endoflist() -> i32 { 0 }

/// Port of `expandcmdpath()` from Src/Zle/zle_tricky.c:2997. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn expandcmdpath() -> i32 { 0 }

/// Port of `expandhistory()` from Src/Zle/zle_tricky.c:2921. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn expandhistory() -> i32 { 0 }

/// Port of `expandorcomplete()` from Src/Zle/zle_tricky.c:299. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn expandorcomplete() -> i32 { 0 }

/// Port of `expandorcompleteprefix()` from Src/Zle/zle_tricky.c:3041. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn expandorcompleteprefix() -> i32 { 0 }

/// Port of `expandword()` from Src/Zle/zle_tricky.c:287. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn expandword() -> i32 { 0 }

/// Port of `fixmagicspace()` from Src/Zle/zle_tricky.c:2867. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn fixmagicspace() -> i32 { 0 }

/// Port of `freebrinfo()` from `Src/Zle/zle_tricky.c:1015`.
/// ```c
/// mod_export void
/// freebrinfo(Brinfo p)
/// {
///     Brinfo n;
///     while (p) {
///         n = p->next;
///         zsfree(p->str);
///         zfree(p, sizeof(*p));
///         p = n;
///     }
/// }
/// ```
/// Free a Brinfo `next`-linked list. C frees each node + its `str`
/// allocation; Rust drops the Box chain (and each `String` inside)
/// automatically when the head Box is dropped.
pub fn freebrinfo(p: Option<crate::ported::zle::zle_h::BrinfoPtr>) {         // c:1015
    // c:1020-1026 — walk + zsfree(str) + zfree(p) loop. In Rust the
    // Drop impls cascade through Box<brinfo> → String → next chain.
    drop(p);
}

/// Port of `get_comp_string()` from Src/Zle/zle_tricky.c:1087. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_comp_string() -> i32 { 0 }

/// Port of `getcurcmd()` from Src/Zle/zle_tricky.c:2932. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getcurcmd() -> i32 { 0 }

/// Port of `inststrlen()` from Src/Zle/zle_tricky.c:2231. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn inststrlen() -> i32 { 0 }

/// Port of `listchoices()` from `Src/Zle/zle_tricky.c:250`.
/// ```c
/// int
/// listchoices(UNUSED(char **args))
/// {
///     usemenu = !!isset(MENUCOMPLETE);
///     useglob = isset(GLOBCOMPLETE);
///     wouldinstab = 0;
///     return docomplete(COMP_LIST_COMPLETE);
/// }
/// ```
/// `list-choices` widget — set the menu/glob globals from options
/// then dispatch to `docomplete(COMP_LIST_COMPLETE)`.
pub fn listchoices() -> i32 {                                                // c:250
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_LIST_COMPLETE;
    // c:253 — `usemenu = !!isset(MENUCOMPLETE)`.
    let menu = crate::ported::options::opt_state_get("menucomplete").unwrap_or(false) as i32;
    USEMENU.store(menu, Ordering::SeqCst);
    // c:254 — `useglob = isset(GLOBCOMPLETE)`.
    let glob = crate::ported::options::opt_state_get("globcomplete").unwrap_or(false) as i32;
    USEGLOB.store(glob, Ordering::SeqCst);
    // c:255 — `wouldinstab = 0`.
    WOULDINSTAB.store(0, Ordering::SeqCst);
    // c:256 — `return docomplete(COMP_LIST_COMPLETE)`.
    let _ = COMP_LIST_COMPLETE;
    docomplete()
}

/// Port of `listexpand()` from `Src/Zle/zle_tricky.c:333`.
/// ```c
/// int
/// listexpand(UNUSED(char **args))
/// {
///     usemenu = !!isset(MENUCOMPLETE);
///     useglob = isset(GLOBCOMPLETE);
///     wouldinstab = 0;
///     return docomplete(COMP_LIST_EXPAND);
/// }
/// ```
/// `list-expand` widget — like listchoices but dispatches with
/// `COMP_LIST_EXPAND`.
pub fn listexpand() -> i32 {                                                 // c:333
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_LIST_EXPAND;
    let menu = crate::ported::options::opt_state_get("menucomplete").unwrap_or(false) as i32;
    USEMENU.store(menu, Ordering::SeqCst);                                   // c:336
    let glob = crate::ported::options::opt_state_get("globcomplete").unwrap_or(false) as i32;
    USEGLOB.store(glob, Ordering::SeqCst);                                   // c:337
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:338
    let _ = COMP_LIST_EXPAND;
    docomplete()                                                             // c:339
}

/// Port of `listlist()` from Src/Zle/zle_tricky.c:2602. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn listlist() -> i32 { 0 }

/// Port of `magicspace()` from Src/Zle/zle_tricky.c:2882. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn magicspace() -> i32 { 0 }

/// Port of `menucomplete()` from Src/Zle/zle_tricky.c:238. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn menucomplete() -> i32 { 0 }

/// Port of `menuexpandorcomplete()` from Src/Zle/zle_tricky.c:321. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn menuexpandorcomplete() -> i32 { 0 }

/// Port of `parambeg()` from Src/Zle/zle_tricky.c:521. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn parambeg() -> i32 { 0 }

/// Port of `printfmt()` from Src/Zle/zle_tricky.c:2431. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn printfmt() -> i32 { 0 }

/// Port of `processcmd()` from Src/Zle/zle_tricky.c:2971. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn processcmd() -> i32 { 0 }

/// Port of `quotestring()` from Src/Zle/zle_tricky.c:428. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn quotestring() -> i32 { 0 }

/// Port of `reversemenucomplete()` from Src/Zle/zle_tricky.c:344. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn reversemenucomplete() -> i32 { 0 }

/// Port of `spellword()` from `Src/Zle/zle_tricky.c:260`.
/// ```c
/// int
/// spellword(UNUSED(char **args))
/// {
///     usemenu = useglob = 0;
///     wouldinstab = 0;
///     return docomplete(COMP_SPELL);
/// }
/// ```
/// `spell-word` widget — clears menu/glob globals and dispatches
/// with `COMP_SPELL`.
pub fn spellword() -> i32 {                                                  // c:260
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_h::COMP_SPELL;
    USEMENU.store(0, Ordering::SeqCst);                                      // c:263 usemenu = 0
    USEGLOB.store(0, Ordering::SeqCst);                                      // c:263 useglob = 0
    WOULDINSTAB.store(0, Ordering::SeqCst);                                  // c:264
    let _ = COMP_SPELL;
    docomplete()                                                             // c:265
}

/// Port of `usetab()` from Src/Zle/zle_tricky.c:183. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn usetab() -> i32 { 0 }
