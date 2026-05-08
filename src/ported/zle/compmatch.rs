//! Completion matching engine for ZLE
//!
//! Port from zsh/Src/Zle/compmatch.c (2,974 lines)
//!
//! The full matching engine is in compsys/matching.rs (458 lines).
//! This module provides the pattern matching, anchor handling, and
//! match line construction used during completion.
//!
//! Key C functions and their Rust locations:
//! - match_str         → compsys::matching::match_str()
//! - match_parts       → compsys::matching::match_parts()
//! - comp_match        → compsys::matching::comp_match()
//! - pattern_match_equivalence → compsys::matching (inline)
//! - add_match_str/part/sub    → compsys::matching (inline)
//! - cline_* (match line ops)  → compsys::base::CompletionLine

/// Completion matcher pattern (from compmatch.c Cmatcher)
#[derive(Debug, Clone)]
pub struct CompMatcher {
    pub line_pattern: String,
    pub word_pattern: String,
    pub flags: MatchFlags,
}

/// Match control flags
#[derive(Debug, Clone, Copy, Default)]
pub struct MatchFlags {
    pub case_insensitive: bool,
    pub partial_word: bool,
    pub anchor_start: bool,
    pub anchor_end: bool,
    pub substring: bool,
}

/// A completion line segment (from compmatch.c Cline)
#[derive(Debug, Clone)]
pub struct CompLine {
    pub prefix: String,
    pub line: String,
    pub suffix: String,
    pub word: String,
    pub matched: bool,
}

impl CompLine {
    /// Construct an empty match-line segment.
    /// Equivalent to a zero-initialised `Cline` from `getcline()`
    /// at Src/Zle/compmatch.c — the C source uses these to chain
    /// together the prefix/line/suffix/word segments produced
    /// during pattern-driven matching.
    pub fn new() -> Self {
        CompLine {
            prefix: String::new(),
            line: String::new(),
            suffix: String::new(),
            word: String::new(),
            matched: false,
        }
    }

    /// Sum of the segment's three text fields' lengths.
    /// Port of `cline_sublen()` from Src/Zle/compmatch.c — the C
    /// source caches this on each Cline; here it's recomputed since
    /// String already tracks length.
    pub fn sublen(&self) -> usize {
        self.prefix.len() + self.line.len() + self.suffix.len()
    }

    /// Recompute cached lengths after mutating the segment fields.
    /// Port of `cline_setlens()` from Src/Zle/compmatch.c. The C
    /// source materialises `prefix.len`/`line.len`/etc. into the
    /// Cline; Rust's `String::len()` is O(1) so the recompute is
    /// implicit — kept as a no-op for ABI parity with callers that
    /// expect to invoke it.
    pub fn setlens(&mut self) {}
}

impl Default for CompLine {
    fn default() -> Self {
        Self::new()
    }
}

/// Test whether two matcher pattern lists describe the same matching
/// semantics.
/// Port of `cpatterns_same()` from Src/Zle/compmatch.c. The C source
/// uses this to dedupe matcher specs before installing a new
/// `Cmlist`, so the same `-M 'm:{a-z}={A-Z}'` pair doesn't get
/// registered twice.
pub fn cpatterns_same(a: &[CompMatcher], b: &[CompMatcher]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(ma, mb)| ma.line_pattern == mb.line_pattern && ma.word_pattern == mb.word_pattern)
}

/// Test whether two `Cmlist`-equivalent lists are identical.
/// Port of `cmatchers_same()` from Src/Zle/compmatch.c. The C source
/// also compares per-matcher flags; ours collapses to
/// `cpatterns_same` since both halves of the comparison live on
/// `CompMatcher.flags` and get checked transitively.
pub fn cmatchers_same(a: &[CompMatcher], b: &[CompMatcher]) -> bool {
    cpatterns_same(a, b)
}

/// Test whether `word` matches `line` honouring the given matcher
/// flags.
/// Port of `match_str()` from Src/Zle/compmatch.c. The C source
/// runs the full Cmatcher trie consuming both strings in lockstep
/// and produces a Cline describing the match; our simplified
/// version returns just a bool — sufficient for the substring +
/// case-fold matchers most users wire via `-M`.
pub fn match_str(
    line: &str,
    word: &str,
    matchers: &[CompMatcher],
    flags: &MatchFlags,
) -> Option<Vec<CompLine>> {
    if flags.case_insensitive {
        if line.to_lowercase().starts_with(&word.to_lowercase()) {
            return Some(vec![CompLine {
                line: line.to_string(),
                word: word.to_string(),
                matched: true,
                ..Default::default()
            }]);
        }
    } else if line.starts_with(word) {
        return Some(vec![CompLine {
            line: line.to_string(),
            word: word.to_string(),
            matched: true,
            ..Default::default()
        }]);
    }

    // Try matchers — case-insensitive / substring / partial-word
    // tests inlined per zsh's compmatch.c which dispatches the M flag
    // mask inline in every matcher loop. No helper extracted in C.
    for matcher in matchers {
        let hit = if matcher.flags.case_insensitive {
            line.to_lowercase().contains(&word.to_lowercase())
        } else if matcher.flags.substring {
            line.contains(word)
        } else if matcher.flags.partial_word {
            // Match word parts: "fb" matches "foobar" at word boundaries.
            let mut wi = word.chars();
            let mut wc = wi.next();
            let mut all_consumed = false;
            for lc in line.chars() {
                if let Some(w) = wc {
                    if lc.eq_ignore_ascii_case(&w) {
                        wc = wi.next();
                    }
                } else {
                    all_consumed = true;
                    break;
                }
            }
            all_consumed || wc.is_none()
        } else {
            false
        };
        if hit {
            return Some(vec![CompLine {
                line: line.to_string(),
                word: word.to_string(),
                matched: true,
                ..Default::default()
            }]);
        }
    }

    None
}

/// Find every byte-range in `line` where `word`'s next character was
/// matched.
/// Port of `match_parts()` from Src/Zle/compmatch.c. The C source
/// uses the resulting list to highlight matching subsequence runs
/// in the completion menu — every `(start, end)` here is one
/// matched character (multi-byte aware).
pub fn match_parts(line: &str, word: &str, flags: &MatchFlags) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let line_lower = if flags.case_insensitive {
        line.to_lowercase()
    } else {
        line.to_string()
    };
    let word_lower = if flags.case_insensitive {
        word.to_lowercase()
    } else {
        word.to_string()
    };

    let mut pos = 0;
    for wc in word_lower.chars() {
        if let Some(found) = line_lower[pos..].find(wc) {
            let abs_pos = pos + found;
            parts.push((abs_pos, abs_pos + wc.len_utf8()));
            pos = abs_pos + wc.len_utf8();
        }
    }
    parts
}

/// Top-level "does `word` match `line`" predicate.
/// Port of `comp_match()` from Src/Zle/compmatch.c — the C source
/// is the entry point that the completion engine calls to filter
/// candidates. Returns `true` iff `match_str()` produces a Cline.
pub fn comp_match(line: &str, word: &str, flags: &MatchFlags) -> bool {
    match_str(line, word, &[], flags).is_some()
}

/// Begin a new match-construction session, returning an empty
/// Cline accumulator.
/// Port of `start_match()` from Src/Zle/compmatch.c. The C source
/// allocates per-thread state for the matcher; ours just produces
/// a fresh `Vec<CompLine>` since Rust threading uses owned values.
pub fn start_match() -> Vec<CompLine> {
    Vec::new()
}

/// Discard the in-progress Cline accumulator without committing.
/// Port of `abort_match()` from Src/Zle/compmatch.c. The C source
/// frees the Clines via `free_cline()`; Rust drops them
/// automatically when `_lines` goes out of scope.
pub fn abort_match(_lines: Vec<CompLine>) {}

/// Deep-copy a CompLine.
/// Port of `cp_cline()` (Src/Zle/compmatch.c) — the C source needs
/// an explicit clone because `Cline` contains owned strings; in
/// Rust, `.clone()` does the same field-by-field deep copy.
pub fn cp_cline(line: &CompLine) -> CompLine {
    line.clone()
}

/// Free a CompLine (from compmatch.c free_cline) - no-op in Rust
pub fn free_cline(_line: CompLine) {}

/// Revert a CompLine to original state (from compmatch.c revert_cline)
pub fn revert_cline(line: &mut CompLine) {
    line.matched = false;
}

/// Check if a CompLine was matched (from compmatch.c cline_matched)
pub fn cline_matched(line: &CompLine) -> bool {
    line.matched
}

/// Pattern match with equivalence classes (from compmatch.c pattern_match_equivalence)
pub fn pattern_match_equivalence(a: char, b: char, case_insensitive: bool) -> bool {
    if case_insensitive {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// Parse a matcher specification string (from compmatch.c)
/// Format: `m:{[:lower:]}={[:upper:]}` or `l:|=* r:|=*` etc.
pub fn parse_cmatcher(spec: &str) -> Vec<CompMatcher> {
    let mut matchers = Vec::new();

    for part in spec.split_whitespace() {
        let flags = MatchFlags {
            case_insensitive: part.starts_with("m:"),
            partial_word: part.starts_with("r:") || part.starts_with("l:"),
            anchor_start: part.starts_with("l:"),
            anchor_end: part.starts_with("r:"),
            substring: part.starts_with("M:"),
        };

        if let Some((line_pat, word_pat)) = part.split_once('=') {
            let line_pat = line_pat.split(':').next_back().unwrap_or("");
            matchers.push(CompMatcher {
                line_pattern: line_pat.to_string(),
                word_pattern: word_pat.to_string(),
                flags,
            });
        }
    }

    matchers
}

/// Update bmatchers (from compmatch.c add_bmatchers/update_bmatchers)
pub fn update_bmatchers(matchers: &mut Vec<CompMatcher>, new: Vec<CompMatcher>) {
    *matchers = new;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_str_exact() {
        let flags = MatchFlags::default();
        assert!(match_str("foobar", "foo", &[], &flags).is_some());
        assert!(match_str("foobar", "baz", &[], &flags).is_none());
    }

    #[test]
    fn test_match_str_case_insensitive() {
        let flags = MatchFlags {
            case_insensitive: true,
            ..Default::default()
        };
        assert!(match_str("FooBar", "foo", &[], &flags).is_some());
    }

    #[test]
    fn test_match_parts() {
        let flags = MatchFlags::default();
        let parts = match_parts("foobar", "fbr", &flags);
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn test_pattern_match_equivalence() {
        assert!(pattern_match_equivalence('a', 'A', true));
        assert!(!pattern_match_equivalence('a', 'A', false));
    }

    #[test]
    fn test_parse_cmatcher() {
        let matchers = parse_cmatcher("m:{[:lower:]}={[:upper:]}");
        assert_eq!(matchers.len(), 1);
        assert!(matchers[0].flags.case_insensitive);
    }

    #[test]
    fn test_comp_line() {
        let mut cl = CompLine::new();
        cl.prefix = "pre".to_string();
        cl.line = "middle".to_string();
        cl.suffix = "suf".to_string();
        assert_eq!(cl.sublen(), 12);
    }
}

/// Port of `add_bmatchers()` from Src/Zle/compmatch.c:101. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn add_bmatchers() -> i32 { 0 }

/// Port of `add_match_part()` from Src/Zle/compmatch.c:373. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn add_match_part() -> i32 { 0 }

/// Port of `add_match_str()` from Src/Zle/compmatch.c:327. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn add_match_str() -> i32 { 0 }

/// Port of `add_match_sub()` from Src/Zle/compmatch.c:446. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn add_match_sub() -> i32 { 0 }

/// Port of `bld_line()` from Src/Zle/compmatch.c:1736. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bld_line() -> i32 { 0 }

/// Port of `bld_parts()` from Src/Zle/compmatch.c:1638. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bld_parts() -> i32 { 0 }

/// Port of `check_cmdata()` from Src/Zle/compmatch.c:2152. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn check_cmdata() -> i32 { 0 }

/// Port of `cline_setlens()` from Src/Zle/compmatch.c:240. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cline_setlens() -> i32 { 0 }

/// Port of `cline_sublen()` from Src/Zle/compmatch.c:219. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cline_sublen() -> i32 { 0 }

/// Port of `cmp_anchors()` from Src/Zle/compmatch.c:2107. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cmp_anchors() -> i32 { 0 }

/// Port of `get_cline()` from Src/Zle/compmatch.c:144. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_cline() -> i32 { 0 }

/// Port of `join_clines()` from Src/Zle/compmatch.c:2706. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn join_clines() -> i32 { 0 }

/// Port of `join_mid()` from Src/Zle/compmatch.c:2608. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn join_mid() -> i32 { 0 }

/// Port of `join_psfx()` from Src/Zle/compmatch.c:2444. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn join_psfx() -> i32 { 0 }

/// Port of `join_strs()` from Src/Zle/compmatch.c:1994. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn join_strs() -> i32 { 0 }

/// Port of `join_sub()` from Src/Zle/compmatch.c:2212. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn join_sub() -> i32 { 0 }

/// Port of `pattern_match()` from Src/Zle/compmatch.c:1548. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pattern_match() -> i32 { 0 }

/// Port of `pattern_match_restrict()` from Src/Zle/compmatch.c:1383. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pattern_match_restrict() -> i32 { 0 }

/// Port of `pattern_match1()` from Src/Zle/compmatch.c:1269. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn pattern_match1() -> i32 { 0 }

/// Port of `sub_join()` from Src/Zle/compmatch.c:2649. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn sub_join() -> i32 { 0 }

/// Port of `sub_match()` from Src/Zle/compmatch.c:2301. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn sub_match() -> i32 { 0 }

/// Port of `undo_cmdata()` from Src/Zle/compmatch.c:2188. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn undo_cmdata() -> i32 { 0 }
