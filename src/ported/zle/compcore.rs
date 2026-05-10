//! Completion core for ZLE
//!
//! Port from zsh/Src/Zle/compcore.c (3,638 lines)
//!
//! The full completion engine is implemented in the `compsys` crate
//! (compsys/compcore.rs, 644 lines). This module provides the ZLE-side
//! interface that connects the editor to the completion system.
//!
//! Key C functions and their Rust locations:
//! - do_completion     → compsys::compcore::do_completion()
//! - before_complete   → compsys::compcore::before_complete()
//! - after_complete    → compsys::compcore::after_complete()
//! - callcompfunc      → compsys::shell_runner (completion function eval)
//! - makecomplist      → compsys::compcore::make_comp_list()
//! - addmatch          → compsys::compadd::add_match()
//! - addmatches        → compsys::compadd::add_matches()
//! - comp_str          → compsys::compset (word extraction)
//! - set_comp_sep      → compsys::compset::set_comp_sep()
//! - check_param       → compsys::base (parameter completion)
//! - multiquote        → compsys::base::multiquote()
//! - tildequote        → compsys::base::tildequote()
//! - ctokenize         → compsys::base::ctokenize()

/// Completion state passed between ZLE and the completion system
#[derive(Debug, Clone, Default)]
pub struct CompState {
    /// Current word being completed
    pub current_word: String,
    /// Words on the command line
    pub words: Vec<String>,
    /// Index of current word (1-based, zsh style)
    pub current: usize,
    /// Cursor position within current word
    pub cursor_pos: usize,
    /// Prefix before cursor in current word
    pub prefix: String,
    /// Suffix after cursor in current word
    pub suffix: String,
    /// The complete command line
    pub buffer: String,
    /// Whether we're in a special context (redirect, assignment, etc.)
    pub context: CompContext,
    /// Matches found
    pub matches: Vec<CompMatch>,
    /// Whether completion is active
    pub active: bool,
    /// Whether to show listing
    pub list: bool,
    /// Whether to insert immediately
    pub insert: bool,
    /// Number of matches
    pub nmatches: usize,
}

/// Completion context
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CompContext {
    #[default]
    Command,
    Argument,
    Redirect,
    Assignment,
    Subscript,
    Math,
    Condition,
    Array,
    Brace,
}

/// A completion match
#[derive(Debug, Clone)]
pub struct CompMatch {
    pub word: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub prefix: String,
    pub suffix: String,
    pub display: Option<String>,
}

impl CompMatch {
    /// Construct a bare match with the given word and no metadata.
    /// Equivalent to a freshly-allocated `Cmatch` from
    /// `mkmatch()` at Src/Zle/compcore.c — every other field
    /// (description, group, prefix, suffix, display) defaults to
    /// empty until a `comp* -d desc` / `comp* -J group` etc.
    /// invocation populates it.
    pub fn new(word: &str) -> Self {
        CompMatch {
            word: word.to_string(),
            description: None,
            group: None,
            prefix: String::new(),
            suffix: String::new(),
            display: None,
        }
    }

    /// Builder helper to set the match description (`compadd -d desc`).
    /// Equivalent to writing `cm->disp` in Src/Zle/compcore.c after
    /// `mkmatch()` — the description is later rendered to the right
    /// of the match in the completion listing.
    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = Some(desc.to_string());
        self
    }
}

/// Initialise completion state for a buffer + cursor pair.
/// Port of the front of `do_completion()` from Src/Zle/compcore.c —
/// the C source splits the line via `getbufferwords()` and stages
/// the surrounding context into the `CompCtl`/`compstate` globals.
/// This Rust port produces a self-contained `CompState` (with
/// quoted-string awareness) that the rest of the completion engine
/// consumes without touching globals.
pub fn do_completion(buffer: &str, cursor: usize) -> CompState {
    let mut state = CompState {
        buffer: buffer.to_string(),
        active: true,
        ..CompState::default()
    };

    // Split into words
    let mut words = Vec::new();
    let mut current = 0;
    let mut word_start = 0;
    let mut in_word = false;
    let mut in_quote = false;
    let mut quote_char = '\0';

    for (i, c) in buffer.char_indices() {
        if in_quote {
            if c == quote_char {
                in_quote = false;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            in_quote = true;
            quote_char = c;
            if !in_word {
                word_start = i;
                in_word = true;
            }
            continue;
        }
        if c.is_whitespace() {
            if in_word {
                words.push(buffer[word_start..i].to_string());
                if cursor >= word_start && cursor <= i {
                    current = words.len();
                }
                in_word = false;
            }
        } else if !in_word {
            word_start = i;
            in_word = true;
        }
    }
    if in_word {
        words.push(buffer[word_start..].to_string());
        if cursor >= word_start {
            current = words.len();
        }
    }
    if words.is_empty() || cursor >= buffer.len() {
        words.push(String::new());
        current = words.len();
    }

    state.words = words;
    state.current = current;
    if current > 0 && current <= state.words.len() {
        state.current_word = state.words[current - 1].clone();
    }

    state
}

/// Append a match to the in-progress completion state.
/// Port of `addmatch()` from Src/Zle/compcore.c. The C source
/// allocates a new `Cmatch`, fills it from `add_match_data()`'s
/// computed buckets, and pushes onto the per-group `matches` linked
/// list. Our Rust shape collapses that to a `Vec<CompMatch>` push +
/// a count update on the surrounding `CompState`.
pub fn addmatch(state: &mut CompState, m: CompMatch) {
    state.matches.push(m);
    state.nmatches = state.matches.len();
}

/// Look up a user-set parameter for the completion engine.
/// Port of `get_user_var()` from Src/Zle/compcore.c. The C source
/// reads the `Param` table directly via `getstrvalue()`; our shape
/// takes an explicit `vars` map so completion functions can be
/// called outside a live shell session (e.g. tests).
pub fn get_user_var(
    name: &str,
    vars: &std::collections::HashMap<String, String>,
) -> Option<String> {
    vars.get(name).cloned()
}

/// Quote a string for safe insertion into the buffer.
/// Port of `multiquote()` from Src/Zle/compcore.c. The C source
/// switches between heavy quoting (escape every special char) and
/// light quoting (escape just `'` / `\\`) based on whether the
/// surrounding context is already inside single quotes — `in_quotes`
/// here mirrors that flag.
pub fn multiquote(s: &str, in_quotes: bool) -> String {
    if in_quotes {
        s.replace('\\', "\\\\").replace('\'', "\\'")
    } else {
        crate::ported::utils::quotedzputs(s)
    }
}

/// Escape a leading `~` so the inserted completion isn't tilde-expanded
/// against a username on next pass.
/// Port of `tildequote()` from Src/Zle/compcore.c.
pub fn tildequote(s: &str) -> String {
    if s.starts_with('~') {
        format!("\\{}", s)
    } else {
        s.to_string()
    }
}

/// Strip backslash escapes from a token, treating `\\X` as `X`.
/// Port of `rembslash()` from Src/Zle/compcore.c — used when the
/// completion engine has already quoted a candidate but a later
/// stage needs the raw form for matching.
pub fn rembslash(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut escape = false;
    for c in s.chars() {
        if escape {
            result.push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_do_completion() {
        let state = do_completion("git commit -m ", 14);
        assert_eq!(state.words, vec!["git", "commit", "-m", ""]);
        assert!(state.active);
    }

    #[test]
    fn test_addmatch() {
        let mut state = CompState::default();
        addmatch(&mut state, CompMatch::new("hello"));
        addmatch(&mut state, CompMatch::new("world"));
        assert_eq!(state.nmatches, 2);
    }

    #[test]
    fn test_multiquote() {
        assert_eq!(multiquote("it's", false), "'it'\\''s'");
    }

    #[test]
    fn test_tildequote() {
        assert_eq!(tildequote("~user"), "\\~user");
        assert_eq!(tildequote("/home"), "/home");
    }

    #[test]
    fn test_rembslash() {
        assert_eq!(rembslash("hello\\ world"), "hello world");
        assert_eq!(rembslash("no\\\\slash"), "no\\slash");
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

/// Port of `add_match_data()` from Src/Zle/compcore.c:2643. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn add_match_data() -> i32 { 0 }

/// Port of `addexpl()` from Src/Zle/compcore.c:3140. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn addexpl() -> i32 { 0 }

/// Port of `addmatches()` from Src/Zle/compcore.c:2080. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn addmatches() -> i32 { 0 }

/// Port of `after_complete()` from Src/Zle/compcore.c:503. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn after_complete() -> i32 { 0 }

/// Port of `before_complete()` from Src/Zle/compcore.c:461. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn before_complete() -> i32 { 0 }

/// Port of `begcmgroup()` from Src/Zle/compcore.c:3073. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn begcmgroup() -> i32 { 0 }

/// Port of `callcompfunc()` from Src/Zle/compcore.c:544. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn callcompfunc() -> i32 { 0 }

/// Port of `check_param()` from Src/Zle/compcore.c:1113. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn check_param() -> i32 { 0 }

/// Port of `comp_quoting_string()` from Src/Zle/compcore.c:1435. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn comp_quoting_string() -> i32 { 0 }

/// Port of `comp_str()` from Src/Zle/compcore.c:1403. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn comp_str() -> i32 { 0 }

/// Port of `ctokenize()` from Src/Zle/compcore.c:1366. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ctokenize() -> i32 { 0 }

/// Port of `dupmatch()` from Src/Zle/compcore.c:3370. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn dupmatch() -> i32 { 0 }

/// Port of `endcmgroup()` from Src/Zle/compcore.c:3131. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn endcmgroup() -> i32 { 0 }

/// Port of `freematch()` from Src/Zle/compcore.c:3575. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freematch() -> i32 { 0 }

/// Port of `freematches()` from Src/Zle/compcore.c:3605. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freematches() -> i32 { 0 }

/// Port of `get_data_arr()` from Src/Zle/compcore.c:2022. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_data_arr() -> i32 { 0 }

/// Port of `makearray()` from Src/Zle/compcore.c:3224. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makearray() -> i32 { 0 }

/// Port of `makecomplist()` from Src/Zle/compcore.c:946. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn makecomplist() -> i32 { 0 }

/// Port of `matchcmp()` from Src/Zle/compcore.c:3173. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn matchcmp() -> i32 { 0 }

/// Port of `matcheq()` from Src/Zle/compcore.c:3207. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn matcheq() -> i32 { 0 }

/// Port of `permmatches()` from Src/Zle/compcore.c:3423. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn permmatches() -> i32 { 0 }

/// Port of `remsquote()` from Src/Zle/compcore.c:1343. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn remsquote() -> i32 { 0 }

/// Port of `set_comp_sep()` from Src/Zle/compcore.c:1460. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_comp_sep() -> i32 { 0 }

/// Port of `set_list_array()` from Src/Zle/compcore.c:1947. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_list_array() -> i32 { 0 }

// =====================================================================
// Cmlist / Cmatcher / Cpattern allocators + freers — Src/Zle/complete.c.
// Ported here (rather than a non-existent complete.rs) because
// PORT.md freezes new src/ported/ file creation; compcore.rs is the
// canonical home for completion-machinery internals.
// =====================================================================

/// Direct port of `freecmlist()` from `Src/Zle/complete.c:98`.
/// C body (c:101-110): walk the linked list freeing each Cmatcher
/// via `freecmatcher()` and the per-entry `str` via `zsfree()`.
/// Rust drop handles the deallocation; this wrapper iterates so
/// callers can name-match the C entry point.
pub fn freecmlist(l: Option<Box<crate::ported::zle::comp_h::Cmlist>>) {      // c:98
    let mut cur = l;
    while let Some(node) = cur {                                             // c:101
        // c:103 — `freecmatcher(l->matcher);` — Rust Box drop frees.
        // c:104 — `zsfree(l->str);` — String drop frees.
        cur = node.next;                                                     // c:102 n = l->next
    }
}

/// Direct port of `freecmatcher()` from `Src/Zle/complete.c:115`.
/// C body (c:118-132):
/// ```c
/// if (!m || --(m->refc)) return;
/// while (m) {
///     n = m->next;
///     freecpattern(m->line); freecpattern(m->word);
///     freecpattern(m->left); freecpattern(m->right);
///     zfree(m, sizeof(struct cmatcher));
///     m = n;
/// }
/// ```
/// The C source uses refcounting (`refc`); Rust port relies on Box
/// ownership semantics — when the last reference drops, every
/// Box-owned Cpattern in the chain drops with it.
pub fn freecmatcher(m: Option<Box<crate::ported::zle::comp_h::Cmatcher>>) {  // c:115
    // c:120 — `if (!m || --(m->refc)) return;` — refcount handled by
    // Rust ownership; the function is a name-parity wrapper.
    let mut cur = m;
    while let Some(node) = cur {                                             // c:122
        // c:124-127 — `freecpattern(m->line/word/left/right)` — Rust
        // drop chains via Option<Box<Cpattern>> fields.
        cur = node.next;                                                     // c:123
    }
}

/// Direct port of `freecpattern()` from `Src/Zle/complete.c:137`.
/// C body (c:141-149):
/// ```c
/// while (p) {
///     n = p->next;
///     if (p->tp <= CPAT_EQUIV) free(p->u.str);
///     zfree(p, sizeof(struct cpattern));
///     p = n;
/// }
/// ```
pub fn freecpattern(p: Option<Box<crate::ported::zle::comp_h::Cpattern>>) {  // c:137
    let mut cur = p;
    while let Some(node) = cur {                                             // c:141
        // c:144 — `if (p->tp <= CPAT_EQUIV) free(p->u.str)` — String
        // drop in Option<String> handles the conditional free.
        cur = node.next;                                                     // c:142
    }
}

/// Direct port of `cpcmatcher()` from `Src/Zle/complete.c:155`.
/// C body (c:158-179): walks the source matcher chain, allocating a
/// fresh Cmatcher per node with `refc = 1`, copying flags / llen /
/// wlen / lalen / ralen, deep-copying each Cpattern via
/// `cpcpattern()`. Returns the new chain head.
pub fn cpcmatcher(m: Option<&crate::ported::zle::comp_h::Cmatcher>)
    -> Option<Box<crate::ported::zle::comp_h::Cmatcher>>                     // c:155
{
    use crate::ported::zle::comp_h::Cmatcher;
    let mut head: Option<Box<Cmatcher>> = None;                              // c:158
    let mut tail_ref: *mut Option<Box<Cmatcher>> = &mut head;
    let mut cur = m;
    while let Some(src) = cur {                                              // c:160
        let n = Box::new(Cmatcher {                                          // c:161 zalloc
            refc:  1,                                                        // c:163
            next:  None,                                                     // c:164
            flags: src.flags,                                                // c:165
            line:  cpcpattern(src.line.as_deref()),                          // c:166
            llen:  src.llen,                                                 // c:167
            word:  cpcpattern(src.word.as_deref()),                          // c:168
            wlen:  src.wlen,                                                 // c:169
            left:  cpcpattern(src.left.as_deref()),                          // c:170
            lalen: src.lalen,                                                // c:171
            right: cpcpattern(src.right.as_deref()),                         // c:172
            ralen: src.ralen,                                                // c:173
        });
        unsafe {
            *tail_ref = Some(n);
            if let Some(ref mut new_node) = *tail_ref {                      // c:175 p = &(n->next)
                tail_ref = &mut new_node.next as *mut _;
            }
        }
        cur = src.next.as_deref();                                           // c:176
    }
    head                                                                     // c:178
}

/// Direct port of `cp_cpattern_element()` from `Src/Zle/complete.c:187`.
/// C body (c:189-216): allocates a fresh Cpattern, sets `next = NULL`,
/// copies `tp`, then dispatches on `tp` to copy `u.str` (CCLASS /
/// NCLASS / EQUIV) or `u.chr` (CHAR). Default keeps the union zero.
pub fn cp_cpattern_element(o: &crate::ported::zle::comp_h::Cpattern)
    -> Box<crate::ported::zle::comp_h::Cpattern>                             // c:187
{
    use crate::ported::zle::comp_h::{Cpattern, CPAT_CCLASS, CPAT_NCLASS,
                                       CPAT_EQUIV, CPAT_CHAR};
    let mut n = Cpattern::default();                                         // c:189 zalloc
    n.next = None;                                                           // c:191
    n.tp = o.tp;                                                             // c:193
    match o.tp {                                                             // c:194
        CPAT_CCLASS | CPAT_NCLASS | CPAT_EQUIV => {                          // c:196-198
            n.str_ = o.str_.clone();                                         // c:199 ztrdup(o->u.str)
        }
        CPAT_CHAR => {                                                       // c:202
            n.chr = o.chr;                                                   // c:203 o->u.chr
        }
        _ => {}                                                              // c:206
    }
    Box::new(n)                                                              // c:212 return n
}

/// Direct port of `cpcpattern()` from `Src/Zle/complete.c:218`.
/// C body (c:222-231): walk the source Cpattern chain, copying each
/// element via `cp_cpattern_element()`. Returns the new chain head.
pub fn cpcpattern(o: Option<&crate::ported::zle::comp_h::Cpattern>)
    -> Option<Box<crate::ported::zle::comp_h::Cpattern>>                     // c:218
{
    use crate::ported::zle::comp_h::Cpattern;
    let mut head: Option<Box<Cpattern>> = None;                              // c:222
    let mut tail_ref: *mut Option<Box<Cpattern>> = &mut head;
    let mut cur = o;
    while let Some(src) = cur {                                              // c:224
        unsafe {
            *tail_ref = Some(cp_cpattern_element(src));                      // c:225
            if let Some(ref mut new_node) = *tail_ref {                      // c:226 p = &((*p)->next)
                tail_ref = &mut new_node.next as *mut _;
            }
        }
        cur = src.next.as_deref();                                           // c:227
    }
    head                                                                     // c:229
}
