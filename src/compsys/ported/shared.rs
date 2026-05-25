//! Tiny helpers shared between per-fn ports. Kept here (not in
//! library.rs) so the `ported/` tree stands alone.

use std::path::Path;

pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            let mode = meta.permissions().mode();
            return mode & 0o111 != 0;
        }
    }
    #[cfg(not(unix))]
    {
        if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            return matches!(ext.as_str(), "exe" | "bat" | "cmd" | "com");
        }
    }
    false
}

/// Shell-glob matcher — supports `*`, `?`, and `(a|b|c)`
/// alternation (zsh extended-glob's `(…|…)` form). Sufficient for
/// the patterns end-user completion files use (e.g.
/// `*.(md|rs|toml)` from `_suffix_alias_files`).
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    // Handle leading `(alt1|alt2|…)` at the top level — split at the
    // matching close paren, try each alternative concatenated with
    // the remainder.
    if let Some(rest) = pattern.strip_prefix('(') {
        if let Some(close) = find_top_close_paren(rest) {
            let group = &rest[..close];
            let after = &rest[close + 1..];
            return group.split('|').any(|alt| {
                let combined = format!("{}{}", alt, after);
                glob_matches(&combined, text)
            });
        }
    }
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_helper(&pat, &txt)
}

fn find_top_close_paren(s: &str) -> Option<usize> {
    let mut depth: i32 = 1;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn glob_helper(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    // Inline alternation at any position: when we encounter `(...)`,
    // re-route through `glob_matches` on the remainder.
    if pat[0] == '(' {
        let rest: String = pat[1..].iter().collect();
        let txt_str: String = txt.iter().collect();
        if let Some(close) = find_top_close_paren(&rest) {
            let group = &rest[..close];
            let after = &rest[close + 1..];
            return group.split('|').any(|alt| {
                let combined = format!("{}{}", alt, after);
                glob_matches(&combined, &txt_str)
            });
        }
    }
    match pat[0] {
        '*' => {
            for i in 0..=txt.len() {
                if glob_helper(&pat[1..], &txt[i..]) {
                    return true;
                }
            }
            false
        }
        '?' => !txt.is_empty() && glob_helper(&pat[1..], &txt[1..]),
        c => !txt.is_empty() && txt[0] == c && glob_helper(&pat[1..], &txt[1..]),
    }
}

/// Shell-glob matcher mirror of the helper that used to live in
/// `compsys/functions.rs` — kept as a separate symbol because callers
/// were spelled `functions::glob_match(...)`, distinct from
/// `glob_matches` above (which the `library.rs`/`ported/_path_files`
/// code used). Both share semantics; the duplicate is intentional for
/// API-shape compat with both call-site ()/* styles */.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    glob_matches(pattern, text)
}

/// Levenshtein edit distance, used by `_approximate`, `_correct`,
/// `_correct_filename`, and `_correct_word`. Moved out of
/// `compsys/functions.rs` so it can be shared across the per-fn ports
/// without introducing a circular dependency between them.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    let mut dp = vec![vec![0; n + 1]; m + 1];

    // Levenshtein DP base row/col init — needless_range_loop trips here
    // but the index IS the value being written, not a positional access.
    #[allow(clippy::needless_range_loop)]
    for i in 0..=m {
        dp[i][0] = i;
    }
    #[allow(clippy::needless_range_loop)]
    for j in 0..=n {
        dp[0][j] = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }

    dp[m][n]
}

/// Check if a string matches any ignored pattern. Extracted from
/// `compsys/base.rs::is_ignored`. Uses the same `glob_match` helper
/// as the rest of the per-fn ports.
pub fn is_ignored(s: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if glob_match(pattern, s) {
            return true;
        }
    }
    false
}

/// `get_ignored_patterns(state, context)` — collect `ignored-patterns`
/// zstyle values for `context`. STUB 2026-05-24: the real lookup goes
/// through `bin_zstyle` / `lookupstyle` from `src/ported/modules/zutil.rs`;
/// pending re-port, this returns an empty list.
pub fn get_ignored_patterns(
    _state: &crate::compsys::base::MainCompleteState,
    _context: &str,
) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    // glob_match coverage migrated from `compsys/base.rs` when the
    // local glob_match helper there was removed in favor of this
    // single shared implementation.

    #[test]
    fn test_glob_match_simple() {
        assert!(glob_match("*.txt", "file.txt"));
        assert!(glob_match("*.txt", ".txt"));
        assert!(!glob_match("*.txt", "file.rs"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(glob_match("file?.txt", "fileX.txt"));
        assert!(!glob_match("file?.txt", "file.txt"));
        assert!(!glob_match("file?.txt", "file12.txt"));
    }

    #[test]
    fn test_glob_match_star_middle() {
        assert!(glob_match("foo*bar", "foobar"));
        assert!(glob_match("foo*bar", "foo123bar"));
        assert!(glob_match("foo*bar", "fooXYZbar"));
        assert!(!glob_match("foo*bar", "foobaz"));
    }

    #[test]
    fn test_glob_match_multiple_stars() {
        assert!(glob_match("*foo*", "foo"));
        assert!(glob_match("*foo*", "afoo"));
        assert!(glob_match("*foo*", "foob"));
        assert!(glob_match("*foo*", "afoob"));
        assert!(!glob_match("*foo*", "bar"));
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exacty"));
        assert!(!glob_match("exact", "xact"));
    }
}
