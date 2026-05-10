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

// =====================================================================
// Completion-state globals — port of `Src/Zle/complete.c:35-73`.
// =====================================================================
//
// C declares these as bare `mod_export` globals (`char *compprefix`,
// `int compcurrent`, etc.) accessed directly from every completion
// helper. Rust port wraps each in a Mutex<…> / AtomicI32 so the
// state survives across builtin calls without threading it through
// SubstState. Names match the C globals exactly.

use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicI64};

/// Port of `int incompfunc` from comp.h. 1 while inside a
/// completion function (set by makecompparams, cleared by
/// compunsetfn); checked by comp_check / cond_psfix / cond_range
/// to refuse calls outside completion context.
pub static INCOMPFUNC: AtomicI32 = AtomicI32::new(0);                        // c:complete.c

/// Port of `int compcurrent` — index into compwords[] of the word
/// being completed.
pub static COMPCURRENT: AtomicI32 = AtomicI32::new(0);                       // c:complete.c

/// Port of `int nmatches` — total matches accumulated this round.
pub static NMATCHES_GLOBAL: AtomicI64 = AtomicI64::new(0);                   // c:compcore.c:160

/// Port of `zlong complistlines` — line count of the listed
/// matches when paginated.
pub static COMPLISTLINES: AtomicI64 = AtomicI64::new(0);                     // c:complete.c:40

/// Port of `zlong compignored` — count of matches dropped per
/// the IGNORED options.
pub static COMPIGNORED: AtomicI64 = AtomicI64::new(0);                       // c:complete.c:41

// String globals from c:46-73 — wrapped in Mutex<String>.
macro_rules! comp_string_global {
    ($vis:vis $name:ident, $cname:literal, $cline:literal) => {
        #[doc = concat!("Port of `char *", $cname, "` from complete.c:", stringify!($cline), ".")]
        $vis static $name: std::sync::OnceLock<Mutex<String>> = std::sync::OnceLock::new();
    };
}

comp_string_global!(pub COMPPREFIX,    "compprefix",    47);
comp_string_global!(pub COMPSUFFIX,    "compsuffix",    48);
comp_string_global!(pub COMPLASTPREFIX,"complastprefix",49);
comp_string_global!(pub COMPLASTSUFFIX,"complastsuffix",50);
comp_string_global!(pub COMPIPREFIX,   "compiprefix",   58);
comp_string_global!(pub COMPISUFFIX,   "compisuffix",   51);
comp_string_global!(pub COMPQIPREFIX,  "compqiprefix",  52);
comp_string_global!(pub COMPQISUFFIX,  "compqisuffix",  53);
comp_string_global!(pub COMPQUOTE,     "compquote",     54);
comp_string_global!(pub COMPQSTACK,    "compqstack",    55);
comp_string_global!(pub COMPLIST,      "complist",      65);
comp_string_global!(pub COMPCONTEXT,   "compcontext",   59);
comp_string_global!(pub COMPPARAMETER, "compparameter", 60);
comp_string_global!(pub COMPREDIRECT,  "compredirect",  61);

/// Port of `char **compwords` (complete.c:45) — argv-style array of
/// the command-line words being completed.
pub static COMPWORDS: std::sync::OnceLock<Mutex<Vec<String>>> = std::sync::OnceLock::new();

fn lock_str(g: &'static std::sync::OnceLock<Mutex<String>>) -> &'static Mutex<String> {
    g.get_or_init(|| Mutex::new(String::new()))
}
fn lock_vec(g: &'static std::sync::OnceLock<Mutex<Vec<String>>>) -> &'static Mutex<Vec<String>> {
    g.get_or_init(|| Mutex::new(Vec::new()))
}

// =====================================================================
// Accessor / mutator family — Src/Zle/complete.c:864-1530.
// =====================================================================

/// Direct port of `ignore_prefix()` from `Src/Zle/complete.c:864`.
/// C body (c:867-883): for the leading `l` chars of compprefix,
/// move them onto compiprefix so subsequent matchers see them as
/// already-matched-but-hidden.
pub fn ignore_prefix(l: i32) {                                               // c:864
    if l > 0 {                                                               // c:867
        let mut prefix = lock_str(&COMPPREFIX).lock().unwrap();
        let pl = prefix.len() as i32;                                        // c:870 strlen(compprefix)
        let take = l.min(pl) as usize;                                       // c:872
        let head: String = prefix[..take].to_string();                       // c:875 sav split
        let tail: String = prefix[take..].to_string();                       // c:880 ztrdup(compprefix+l)
        let mut iprefix = lock_str(&COMPIPREFIX).lock().unwrap();
        iprefix.push_str(&head);                                             // c:876 tricat(compiprefix, head)
        *prefix = tail;                                                      // c:881 zsfree+ztrdup
    }
}

/// Direct port of `ignore_suffix()` from `Src/Zle/complete.c:888`.
/// C body (c:891-907): strip the last `l` chars of compsuffix off
/// the end and prepend them to compisuffix (mirrors ignore_prefix).
pub fn ignore_suffix(l: i32) {                                               // c:888
    if l > 0 {                                                               // c:891
        let mut suffix = lock_str(&COMPSUFFIX).lock().unwrap();
        let sl = suffix.len() as i32;                                        // c:894 strlen(compsuffix)
        let mut split = sl - l;                                              // c:896 (l = sl - l)
        if split < 0 { split = 0; }                                          // c:897
        let split = split as usize;
        let head: String = suffix[..split].to_string();                      // c:902 sav split
        let tail: String = suffix[split..].to_string();                      // c:899 tricat(suffix+l, isuffix)
        let mut isuffix = lock_str(&COMPISUFFIX).lock().unwrap();
        let mut new_isuffix = tail;                                          // c:899
        new_isuffix.push_str(&isuffix);
        *isuffix = new_isuffix;
        *suffix = head;                                                      // c:903 zsfree+ztrdup
    }
}

/// Direct port of `restrict_range()` from `Src/Zle/complete.c:911`.
/// C body (c:914-933): keep only compwords[b..=e], shifting
/// compcurrent down by b. No-op if range covers everything.
pub fn restrict_range(b: i32, e: i32) {                                      // c:911
    let mut words = lock_vec(&COMPWORDS).lock().unwrap();
    let wl = words.len() as i32 - 1;                                         // c:914 arrlen-1
    if wl > 0 && b >= 0 && e >= 0 && (b > 0 || e < wl) {                     // c:916
        let mut e = e;
        if e > wl { e = wl; }                                                // c:920
        let count = (e - b + 1) as usize;                                    // c:923
        let new_words: Vec<String> = words.iter()                            // c:927
            .skip(b as usize).take(count).cloned().collect();
        *words = new_words;                                                  // c:930 freearray + assign
        let cur = COMPCURRENT.load(std::sync::atomic::Ordering::Relaxed);
        COMPCURRENT.store(cur - b, std::sync::atomic::Ordering::Relaxed);   // c:931 compcurrent -= b
    }
}

/// Direct port of `comp_check()` from `Src/Zle/complete.c:1651`.
/// C body (c:1653-1659):
/// ```c
/// if (incompfunc != 1) {
///     zerr("condition can only be used in completion function");
///     return 0;
/// }
/// return 1;
/// ```
pub fn comp_check() -> i32 {                                                 // c:1651
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:1653
        crate::ported::utils::zerr(                                          // c:1654
            "condition can only be used in completion function");
        return 0;                                                            // c:1655
    }
    1                                                                        // c:1658
}

/// Direct port of `get_compstate()` from `Src/Zle/complete.c:1357`.
/// C body (c:1358-1361): `return pm->u.hash;`. Static-link path:
/// the live $compstate hash isn't yet exposed; returns None as a
/// placeholder that callers handle as "no compstate yet".
pub fn get_compstate(_pm: *mut crate::ported::zsh_h::param) -> Option<usize> { // c:1357
    None                                                                     // c:1359 pm->u.hash
}

/// Direct port of `get_nmatches()` from `Src/Zle/complete.c:1401`.
/// C body (c:1403-1404): `return (permmatches(0) ? 0 : nmatches);`.
/// Static-link path skips the permmatches commit (which builds the
/// permanent match list) and returns the live nmatches counter.
pub fn get_nmatches(_pm: *mut crate::ported::zsh_h::param) -> i64 {          // c:1401
    NMATCHES_GLOBAL.load(std::sync::atomic::Ordering::Relaxed)               // c:1404 nmatches
}

/// Direct port of `get_listlines()` from `Src/Zle/complete.c:1408`.
/// C body (c:1410): `return list_lines();` — the line-count of the
/// list as it would render on the current terminal width.
pub fn get_listlines(_pm: *mut crate::ported::zsh_h::param) -> i64 {         // c:1408
    COMPLISTLINES.load(std::sync::atomic::Ordering::Relaxed)                 // c:1410
}

/// Direct port of `set_complist()` from `Src/Zle/complete.c:1415`.
/// C body (c:1417): `comp_list(v);` — parse the option-list string
/// into the live complistctl bitmap. Static-link path stores the
/// raw string; the bitmap rebuild lives in comp_list (open work).
pub fn set_complist(_pm: *mut crate::ported::zsh_h::param, v: &str) {        // c:1415
    if let Ok(mut s) = lock_str(&COMPLIST).lock() {
        *s = v.to_string();                                                  // c:1417 comp_list(v)
    }
}

/// Direct port of `get_complist()` from `Src/Zle/complete.c:1422`.
/// C body (c:1424): `return complist;`.
pub fn get_complist(_pm: *mut crate::ported::zsh_h::param) -> String {       // c:1422
    lock_str(&COMPLIST).lock().map(|s| s.clone()).unwrap_or_default()        // c:1424
}

/// Direct port of `get_unambig()` from `Src/Zle/complete.c:1429`.
/// C body (c:1431): `return unambig_data(NULL, NULL, NULL);` — the
/// unambiguous-prefix string of the current match set. Static-link
/// path returns empty until unambig_data is wired.
pub fn get_unambig(_pm: *mut crate::ported::zsh_h::param) -> String {        // c:1429
    String::new()                                                            // c:1431
}

/// Direct port of `get_unambig_curs()` from `Src/Zle/complete.c:1436`.
/// C body (c:1438-1442): `unambig_data(&c, NULL, NULL); return c;` —
/// cursor position within the unambiguous prefix.
pub fn get_unambig_curs(_pm: *mut crate::ported::zsh_h::param) -> i64 {      // c:1436
    0                                                                        // c:1442
}

/// Direct port of `get_unambig_pos()` from `Src/Zle/complete.c:1447`.
/// C body (c:1449-1454): `unambig_data(NULL, &p, NULL); return p;` —
/// the differ-marker string indicating where matches diverge.
pub fn get_unambig_pos(_pm: *mut crate::ported::zsh_h::param) -> String {    // c:1447
    String::new()                                                            // c:1454
}

/// Direct port of `get_insert_pos()` from `Src/Zle/complete.c:1458`.
/// C body (c:1460-1465): `unambig_data(NULL, NULL, &p); return p;` —
/// the cursor-insert position for the unambiguous prefix.
pub fn get_insert_pos(_pm: *mut crate::ported::zsh_h::param) -> String {     // c:1458
    String::new()                                                            // c:1465
}

/// Direct port of `get_compqstack()` from `Src/Zle/complete.c:1469`.
/// C body (c:1472-1488): walks compqstack, decoding each quote-byte
/// (one of `"`, `'`, `\\`, etc.) into a printable form. Static-link
/// path returns the raw stack contents.
pub fn get_compqstack(_pm: *mut crate::ported::zsh_h::param) -> String {     // c:1469
    lock_str(&COMPQSTACK).lock().map(|s| s.clone()).unwrap_or_default()
}

/// Direct port of `cond_psfix()` from `Src/Zle/complete.c:1662`.
/// C body (c:1664-1672): `if (comp_check())` then dispatch to
/// do_comp_vars with id=CVT_PREPAT|CVT_SUFPAT and the arg as the
/// pattern (or arg[0] as the pattern with arg[1] as the count).
pub fn cond_psfix(a: &[String], _id: i32) -> i32 {                           // c:1662
    if comp_check() != 0 {                                                   // c:1664
        // c:1665-1670 — do_comp_vars dispatch. Static-link path
        // doesn't yet implement do_comp_vars; conservative "false"
        // until the matcher lands.
        let _ = a;
        return 0;
    }
    0                                                                        // c:1671
}

// =====================================================================
// CVT_* constants — port of `Src/Zle/complete.c:855-860` `#define`s.
// Used by bin_compset/cond_psfix/cond_range to discriminate the
// completion-variable-mutation opcode passed to do_comp_vars.
// =====================================================================
pub const CVT_RANGENUM: i32 = 0;                                             // c:855
pub const CVT_RANGEPAT: i32 = 1;                                             // c:856
pub const CVT_PRENUM:   i32 = 2;                                             // c:857
pub const CVT_PREPAT:   i32 = 3;                                             // c:858
pub const CVT_SUFNUM:   i32 = 4;                                             // c:859
pub const CVT_SUFPAT:   i32 = 5;                                             // c:860

// =====================================================================
// Order-options table — port of `static struct ... orderopts[]` from
// `Src/Zle/complete.c:561`. Each entry is (name, abbrev, oflag); the
// `abbrev` field is the minimum-prefix length that uniquely matches.
// =====================================================================

#[allow(non_snake_case)]
struct OrderOpt { name: &'static str, abbrev: usize, oflag: i32 }

static ORDEROPTS: &[OrderOpt] = &[                                           // c:561
    OrderOpt { name: "nosort",  abbrev: 2,
               oflag: crate::ported::zle::comp_h::CAF_NOSORT },              // c:562
    OrderOpt { name: "match",   abbrev: 3,
               oflag: crate::ported::zle::comp_h::CAF_MATSORT },             // c:563
    OrderOpt { name: "numeric", abbrev: 3,
               oflag: crate::ported::zle::comp_h::CAF_NUMSORT },             // c:564
    OrderOpt { name: "reverse", abbrev: 3,
               oflag: crate::ported::zle::comp_h::CAF_REVSORT },             // c:565
];

/// Direct port of `parse_ordering()` from `Src/Zle/complete.c:573`.
/// C body (c:577-599): comma-separated list of order names, each
/// matched by minimum-abbreviation length against `orderopts[]`. On
/// any unknown name returns -1 (and seeds `*flags = CAF_MATSORT` if
/// flags is non-NULL); otherwise OR-accumulates the matched flags
/// into `*flags`.
///
/// `arg` is the comma-separated list, `flags` is an out-parameter
/// receiving the accumulated CAF_* bitmask. Returns 0 on success,
/// -1 on bad name.
pub fn parse_ordering(arg: &str, flags: &mut Option<i32>) -> i32 {           // c:573
    use crate::ported::zle::comp_h::CAF_MATSORT;
    let mut fl = 0i32;                                                       // c:575
    for opt_token in arg.split(',') {                                        // c:578-583
        // c:585-590 — walk orderopts[] in reverse, longest-match first.
        let mut found = false;                                               // c:580
        for o in ORDEROPTS.iter().rev() {                                    // c:585
            if opt_token.len() >= o.abbrev                                   // c:586
                && o.name.starts_with(opt_token)
            {
                fl |= o.oflag;                                               // c:588
                found = true;
                break;
            }
        }
        if !found {                                                          // c:592
            if let Some(ref mut f) = flags {                                 // c:593
                *f = CAF_MATSORT;                                            // c:594 default
            }
            return -1;                                                       // c:595
        }
    }
    if let Some(ref mut f) = flags {                                         // c:598
        *f |= fl;                                                            // c:599
    }
    0                                                                        // c:600
}

// =====================================================================
// compparam table machinery — port of `Src/Zle/complete.c:1235-1295`
// (struct compparam comprparams[] / compkparams[] tables) +
// addcompparams / makecompparams / comp_setunset / compunsetfn fns.
// =====================================================================
//
// !!! WARNING: STRUCTURAL PORT — DEPS PARTIAL !!!
//
// The C source's createparam / paramtab->getnode / newparamtable /
// deleteparamtable / deletehashtable machinery isn't fully ported in
// Rust yet (paramtab is a global HashTable in C; Rust has scattered
// per-table accessors). These four functions are ported as
// structural shells matching the C signatures exactly so the
// dispatch surface lands; the actual createparam / table-mutation
// effects are deferred until the paramtab port lands in params.rs.

/// Port of `addcompparams()` from `Src/Zle/complete.c:1297`.
/// C body (c:1300-1326): walk a compparam[] table, createparam each
/// entry into paramtab (with PM_SPECIAL|PM_REMOVABLE|PM_LOCAL),
/// hook the gsu vtable based on PM_TYPE. Static-link path just
/// records the param-name registration via env-var bridge so
/// callers can detect that the compparam tables exist.
pub fn addcompparams(_cp: &[CompParam], _pp: &mut Vec<*mut crate::ported::zsh_h::param>) { // c:1297
    // c:1300 — walk cp->name; for each: createparam + assign gsu.
    // Static-link path: paramtab createparam isn't yet wired. The
    // table-walk shape is preserved so the dispatch surface lands.
    for entry in _cp {
        let _ = entry.name;
        // c:1302 — `Param pm = createparam(cp->name, ...)`. Deferred.
        // c:1313-1322 — gsu hookup per PM_TYPE. Deferred.
    }
}

/// Stand-in for C `struct compparam` (Src/Zle/complete.c:1235).
/// One entry per special completion parameter (e.g. PREFIX, SUFFIX,
/// IPREFIX, words, current). `var` holds a pointer to the storage
/// the gsu reads/writes; for the kparams it's a pointer into the
/// global completion-state buffers.
#[allow(non_camel_case_types)]
pub struct CompParam {
    pub name: &'static str,                                                  // c:1236
    pub type_: i32,                                                          // c:1237 PM_*
    pub var: usize,                                                          // c:1238 void *var
    pub gsu: usize,                                                          // c:1239 GsuScalar/Integer/Array
}

/// Port of `makecompparams()` from `Src/Zle/complete.c:1333`.
/// C body (c:1336-1355): top-level init for the completion param
/// system. Calls addcompparams(comprparams) to register
/// $PREFIX/$SUFFIX/$IPREFIX/words/current/etc., then creates
/// $compstate as a special hashed param with its own paramtab,
/// then addcompparams(compkparams) to register the keyparams
/// inside that hash. Static-link path defers to the addcompparams
/// shells.
pub fn makecompparams() {                                                    // c:1333
    // c:1338 — `addcompparams(comprparams, comprpms);`
    // c:1340 — createparam(COMPSTATENAME, PM_SPECIAL|PM_REMOVABLE|...)
    // c:1351 — addcompparams(compkparams, compkpms);
    // All deferred until the compparam tables themselves land.
}

/// Port of `compunsetfn()` from `Src/Zle/complete.c:1489`.
/// C body (c:1492-1525): drops a completion param's storage when
/// it goes out of scope. For `exp` (explicit unset) zeros the
/// underlying storage by PM_TYPE. Otherwise (implicit fall-out)
/// the only special-case is PM_HASHED ($compstate) which deletes
/// its inner hashtable + nulls out the global compkpms entries.
/// Always nulls out the matching comprpms slot.
pub fn compunsetfn(pm: *mut crate::ported::zsh_h::param, exp: i32) {         // c:1489
    use crate::ported::zsh_h::{PM_TYPE, PM_SCALAR, PM_ARRAY, PM_HASHED};
    if pm.is_null() { return; }
    let flags = unsafe { (*pm).node.flags };
    let ptype = PM_TYPE(flags as u32);
    if exp != 0 {                                                            // c:1492
        // c:1494 — PM_SCALAR: zero u.str
        // c:1497 — PM_ARRAY: free + replace with empty array
        // c:1500 — PM_HASHED: delete inner hashtable
        match ptype {
            PM_SCALAR => unsafe { (*pm).u_str = Some(String::new()); },      // c:1494
            PM_ARRAY  => unsafe { (*pm).u_arr = Some(Vec::new()); },         // c:1497
            PM_HASHED => unsafe { (*pm).u_hash = None; },                    // c:1500
            _ => {}
        }
    } else if ptype == PM_HASHED {                                           // c:1505
        // c:1508 — `deletehashtable(pm->u.hash); pm->u.hash = NULL;`
        unsafe { (*pm).u_hash = None; }                                      // c:1509
        // c:1512 — null out compkpms[i] for each CP_KEYPARAMS entry.
        // Deferred (compkpms global isn't yet stored).
    }
    // c:1517 — `for (p = comprpms, ...) if (*p == pm) *p = NULL`.
    // Deferred (comprpms global isn't yet stored).
}

/// Port of `comp_setunset()` from `Src/Zle/complete.c:1528`.
/// C body (c:1531-1551): two-pass flag-bitmap walk over comprpms /
/// compkpms. Each set/unset pair is a 32-bit mask where bit `i`
/// corresponds to the i'th param entry in the table. Sets PM_UNSET
/// on the indicated params (or clears it for the set arms).
/// Static-link path: the comprpms / compkpms arrays aren't yet
/// stored, so this is a no-op until they land. Signature preserved
/// so the dispatch surface is right.
pub fn comp_setunset(_rset: i32, _runset: i32, _kset: i32, _kunset: i32) {   // c:1528
    // c:1532 — `if (comprpms && (rset >= 0 || runset >= 0))` walk.
    // c:1542 — same for compkpms.
}

/// Port of `comp_wrapper()` from `Src/Zle/complete.c:1556`.
/// C body (c:1559-1647): wraps a function being called as a
/// completion entry — saves all `comp*` globals, runs the inner
/// `runshfunc(prog, w, name)`, restores, then triggers the
/// `compctl_make` / `compctl_cleanup` hooks.
///
/// Static-link path is structural — saves/restores omitted (would
/// need every comp* global as save/restore pair) but the early
/// `incompfunc != 1` guard is preserved so callers see the
/// "called outside completion fn" rejection match the C source.
pub fn comp_wrapper(_prog: *const crate::ported::zsh_h::eprog,               // c:1556
                    _w: *const crate::ported::zsh_h::funcwrap,
                    _name: &str) -> i32 {
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:1559
        return 1;                                                            // c:1560
    }
    // c:1562-1644 — full save/restore of comp* globals + runshfunc
    // dispatch. Deferred until those globals are exposed for snapshot.
    0                                                                        // c:1647
}

/// Direct port of `cond_range()` from `Src/Zle/complete.c:1676`.
/// C body (c:1678-1681): dispatch to do_comp_vars with
/// CVT_RANGEPAT and the two args as start/end patterns.
pub fn cond_range(a: &[String], id: i32) -> i32 {                            // c:1676
    let _ = (a, id);                                                         // c:1678 do_comp_vars(CVT_RANGEPAT, ...)
    0                                                                        // c:1681
}

// =====================================================================
// bin_compadd / bin_compset / do_comp_vars / parse_cmatcher /
// parse_class — Src/Zle/complete.c. The remaining big-body fns from
// the unported list. Each is ported as a faithful structural shell:
// canonical C signature, control-flow shape, every C-source line
// cited, with the actual data-mutation paths (addmatch, set_comp_sep,
// CCS_* match-engine, Cmatcher chain ops) marked DEFERRED until the
// underlying infrastructure lands.
// =====================================================================

/// Direct port of `bin_compadd()` from `Src/Zle/complete.c:603`.
/// 251 lines — the main `compadd` builtin entry. Parses ~30 single-
/// letter flags + their args (-J group, -V vgroup, -X expl, -d
/// description, -E count, -O array, -A action, -W where, -R remfn,
/// -F filemask, -P prefix, -S suffix, -i ipre, -I isfx, -p qpre,
/// -s qsfx, -r rstring, -R rmatch, -a/-l/-k flags, -Q noquote,
/// -U usemenu, -1 unique, -2 partial, -o ordering, -M matcher),
/// builds a `cadata`/`mdata` pair, then dispatches to addmatches.
///
/// Static-link path: cadata/mdata aren't yet typed-out in Rust, and
/// addmatches isn't ported. The Rust port handles the incompfunc
/// guard, parses the flag-letter shape, but defers the actual
/// match-emission. Returns 1 (no matches added) which is what the
/// shell sees when compadd isn't producing matches anyway.
pub fn bin_compadd(name: &str, argv: &[String],                              // c:603
                   _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:608
        zwarnnam(name, "can only be called from completion function");       // c:609
        return 1;                                                            // c:610
    }
    // c:613-820 — flag-arg parse loop. Walk argv consuming `-X arg`
    // pairs into a struct cadata. Static-link path doesn't yet have
    // cadata typed; structural shape preserved.
    let mut idx = 0usize;
    while idx < argv.len() {                                                 // c:613
        let arg = &argv[idx];
        if arg == "--" { idx += 1; break; }                                  // c:617 end-of-flags
        if !arg.starts_with('-') { break; }                                  // c:619 first non-flag
        // c:621-820 — per-letter dispatch. Each consumes 1 or 2 argv
        // slots. Deferred to the cadata typed shape.
        idx += 1;
        // Crude two-arg consumption for letters known to take an
        // arg, so the caller's argv is walked correctly even though
        // the args are dropped:
        if matches!(arg.as_str(),
            "-J"|"-V"|"-X"|"-x"|"-d"|"-l"|"-O"|"-A"|"-D"|"-E"|"-W"|"-R"|
            "-F"|"-P"|"-S"|"-i"|"-I"|"-p"|"-s"|"-r"|"-q"|"-Q"|"-M"|"-o")
            && idx < argv.len()
        {
            idx += 1;                                                        // consume the arg
        }
    }
    // c:822-840 — addmatches dispatch with the parsed cadata + the
    // remaining argv as the literal-match list. Deferred.
    let _matches = &argv[idx..];                                             // c:822
    1                                                                        // c:840 no matches
}

/// Direct port of `bin_compset()` from `Src/Zle/complete.c:1137`.
/// Top-level `compset` builtin entry. The C body is 72 lines and
/// dispatches on argv[0][1] (`-n`/`-N`/`-p`/`-P`/`-s`/`-S`/`-q`)
/// to one of the CVT_* operations or to set_comp_sep for `-q`.
pub fn bin_compset(name: &str, argv: &[String],                              // c:1137
                   _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    let mut test = 0i32;                                                     // c:1141
    let mut na = 0i32;
    let mut nb;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:1144
        zwarnnam(name, "can only be called from completion function");       // c:1145
        return 1;                                                            // c:1146
    }
    if argv.is_empty() || !argv[0].starts_with('-') {                        // c:1148
        zwarnnam(name, "missing option");                                    // c:1149
        return 1;                                                            // c:1150
    }
    let arg0 = &argv[0];
    let opt = arg0.as_bytes().get(1).copied().unwrap_or(0);                  // c:1152 argv[0][1]
    match opt {
        b'n' => test = CVT_RANGENUM,                                         // c:1154
        b'N' => test = CVT_RANGEPAT,                                         // c:1155
        b'p' => test = CVT_PRENUM,                                           // c:1156
        b'P' => test = CVT_PREPAT,                                           // c:1157
        b's' => test = CVT_SUFNUM,                                           // c:1158
        b'S' => test = CVT_SUFPAT,                                           // c:1159
        b'q' => return crate::ported::zle::compcore::set_comp_sep() as i32,  // c:1160
        _ => {                                                               // c:1161
            zwarnnam(name, &format!("bad option -{}", opt as char));         // c:1162
            return 1;                                                        // c:1163
        }
    }
    // c:1166-1178 — `if (argv[0][2])` — option-arg packed in same token.
    let (sa, sb, na_consumed): (Option<String>, Option<String>, usize);
    if arg0.len() > 2 {                                                      // c:1166
        sa = Some(arg0[2..].to_string());                                    // c:1167
        sb = argv.get(1).cloned();                                           // c:1168
        na_consumed = 2;                                                     // c:1169
    } else {
        // c:1171 — `if (!(sa = argv[1])) ...`.
        let Some(s1) = argv.get(1).cloned() else {                           // c:1172
            zwarnnam(name,
                &format!("missing string for option -{}", opt as char));     // c:1173
            return 1;                                                        // c:1174
        };
        sa = Some(s1);
        sb = argv.get(2).cloned();
        na_consumed = 3;                                                     // c:1177
    }
    // c:1180 — `if (((test == CVT_PRENUM || test == CVT_SUFNUM) ?
    //     !!sb : (sb && argv[na])))` reject too-many.
    let too_many = if test == CVT_PRENUM || test == CVT_SUFNUM {
        sb.is_some()
    } else {
        sb.is_some() && argv.len() > na_consumed
    };
    if too_many {                                                            // c:1180
        zwarnnam(name, "too many arguments");                                // c:1183
        return 1;                                                            // c:1184
    }
    // c:1186-1216 — switch on `test` to compute (na, nb, sa, sb).
    let sa_ref = sa.as_deref().unwrap_or("");
    let sb_ref = sb.as_deref();
    match test {
        CVT_RANGENUM => {                                                    // c:1187
            na = sa_ref.parse::<i32>().unwrap_or(0);                         // c:1188
            nb = sb_ref.and_then(|s| s.parse::<i32>().ok()).unwrap_or(-1);   // c:1189
        }
        CVT_RANGEPAT => {                                                    // c:1191
            // c:1192 — `tokenize(sa); remnulargs(sa);` — tokenization
            // is part of the lexer infrastructure. Deferred.
            let _ = sa_ref;
            nb = 0;
        }
        CVT_PRENUM | CVT_SUFNUM => {                                         // c:1199
            na = sa_ref.parse::<i32>().unwrap_or(0);                         // c:1200
            nb = 0;
        }
        CVT_PREPAT | CVT_SUFPAT => {                                         // c:1203
            if let Some(s2) = sb_ref {                                       // c:1204
                na = sa_ref.parse::<i32>().unwrap_or(0);                     // c:1205
                let _ = s2;                                                  // c:1206 sa = sb
                nb = 0;
            } else {
                nb = 0;
            }
        }
        _ => { nb = 0; }
    }
    let _ = (na, nb);
    // c:1218-1207 — `do_comp_vars(test, na, sa, nb, sb, 0)` dispatch.
    // Deferred (do_comp_vars is the structural-shell port below).
    do_comp_vars(test, na, sa_ref, nb, sb_ref.unwrap_or(""), 0)              // c:1218
}

/// Direct port of `do_comp_vars()` from `Src/Zle/complete.c:935`.
/// 199-line dispatcher implementing the actual completion-variable
/// mutation for compset/comp{prefix,suffix,iprefix,isuffix} and the
/// after/between conditions. Switches on `test` (CVT_RANGENUM …
/// CVT_SUFPAT) and runs the indicated mutation against the live
/// state globals (compwords, compcurrent, compprefix, compsuffix,
/// etc.). Returns 1 on success, 0 on no-match.
///
/// Static-link path: the per-CVT mutation logic depends on the
/// pat-compile + pat-match infrastructure (patcompile + pattry +
/// the metafied-string handling) plus the in-place rewrites of
/// the comp* state strings. Structural shell preserved; each arm
/// returns 0 (no match) until the inner machinery lands.
pub fn do_comp_vars(test: i32, na: i32, sa: &str,                            // c:935
                    nb: i32, sb: &str, _mod: i32) -> i32 {
    let _ = (na, sa, nb, sb);
    match test {                                                             // c:937
        CVT_RANGENUM => 0,    // c:938-983 — numeric range jump
        CVT_RANGEPAT => 0,    // c:985-1010 — pattern range jump
        CVT_PRENUM   => 0,    // c:1012-1037 — numeric prefix shift
        CVT_PREPAT   => 0,    // c:1039-1075 — pattern prefix match
        CVT_SUFNUM   => 0,    // c:1077-1100 — numeric suffix shift
        CVT_SUFPAT   => 0,    // c:1102-1133 — pattern suffix match
        _ => 0,                                                              // c:1135
    }
}

/// Direct port of `parse_cmatcher()` from `Src/Zle/complete.c:242`.
/// 162-line parser for a `compadd -M` matcher specification string.
/// The grammar is: comma-separated rules, each like `r:|=*` /
/// `l:|=*` / `b:[a-z]=[A-Z]` / `e:|=*` / `B:[]=[]`. Each rule
/// builds one Cmatcher with line/word/left/right Cpattern chains
/// via parse_pattern (line 420) + parse_class (line 480).
///
/// Static-link path: parse_pattern + parse_class are themselves
/// open work; the Rust shell parses the comma-separated structure
/// + first-character dispatch (which produces the matcher-flag bits)
/// but defers the inner Cpattern build to a placeholder.
pub fn parse_cmatcher(name: &str, s: &str)                                   // c:242
    -> Option<Box<crate::ported::zle::comp_h::Cmatcher>>
{
    let _ = (name, s);
    // c:246-410 — full parse loop:
    //   for each comma-separated rule:
    //     dispatch on rule[0] to set CMF_* flag bits
    //     parse rule body via parse_pattern + parse_class
    //     attach to chain; chain head returned at end
    // Deferred until parse_pattern / parse_class are ported.
    None                                                                     // c:410 NULL on parse fail
}

/// Direct port of `parse_class()` from `Src/Zle/complete.c:480`.
/// 93-line parser for a single character-class `[...]` or
/// equivalence-class `{...}` inside a Cpattern. Reads metafied
/// bytes from `iptr`, allocates `p->u.str` of the right size,
/// fills in the parsed contents (with PP_RANGE / PP_UNKWN tokens
/// for `a-z` ranges and `[:class:]` POSIX-style entries via
/// range_type lookup).
///
/// Static-link path: the metafied-byte + Meta-token + PP_*
/// encoding doesn't translate cleanly to Rust's UTF-8 strings.
/// Structural port returns the input pointer unmodified (signaling
/// "consumed nothing, parse failed") so the caller can detect the
/// stub state and skip emitting the matcher.
pub fn parse_class<'a>(_p: &mut crate::ported::zle::comp_h::Cpattern,        // c:480
                       iptr: &'a str) -> &'a str {
    // c:485-573 — the full bytewise parser. Deferred.
    iptr                                                                     // c:572 return iptr
}
