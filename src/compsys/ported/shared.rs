//! Tiny helpers shared between per-fn ports. Kept here (not in
//! library.rs) so the `ported/` tree stands alone.

use std::path::Path;

// =====================================================================
// Function-local parameter declarations for the Rust ports.
// =====================================================================
//
// An upstream completion function opens with a `local`/`typeset` line
// (`Completion/Base/Core/_main_complete:11,27-54`). Every scratch name
// it touches is therefore created at `locallevel`, reads back as
// `…-local` through `${(t)name}`, and is unwound by `endparamscope`
// when the function returns.
//
// A Rust port assigns the same names with `setsparam`/`setaparam`,
// which routes through `createparam(name, PM_SCALAR)` — no PM_LOCAL,
// so the parameter is born at level 0 and both properties are lost:
// `${(t)_comp_tags}` reads `scalar` and the name survives the call.
// That is observable: the user's `_parameters`
// (`~/.zpwr/autoload/comp_utils/_parameters:34`) filters candidates
// with `${(@k)parameters[(R)${pattern[2]}~*local*]}`, i.e. it drops
// every parameter whose type string contains `local`. Leaked port
// scratch names slipped through that filter and `unset <TAB>` offered
// them alongside the user's real parameters.
//
// `declare_locals` is the one place that gap is closed: it is the
// Rust-port spelling of the upstream `local NAME …` line and mirrors
// the PM_LOCAL branch of `bin_typeset` (`src/ported/builtin.rs:5570`,
// port of `Src/builtin.c:2469-2575`).

pub use crate::ported::zsh_h::{PM_ARRAY, PM_HASHED, PM_UNIQUE};

/// Declare `names` local to the CURRENT function scope — the Rust-port
/// equivalent of an upstream completion function's `local NAME …` line.
///
/// `kind` carries the type/attribute bits the shell source spells out
/// (`PM_ARRAY` for `local -a`, `PM_HASHED` for `local -A`, `PM_UNIQUE`
/// for `typeset -U`); pass `0` for a plain scalar `local`.
///
/// Mirrors `Src/builtin.c:2469-2575` (`typeset_single`'s PM_LOCAL arm):
/// only allocate a shadow when the visible parameter lives at a LOWER
/// scope than the current `locallevel`, then stamp `pm->level =
/// locallevel` so `endparamscope` unwinds it. At top level
/// (`locallevel == 0`) C's `pm->level < locallevel` can never hold, so
/// nothing is declared — the port then behaves exactly as it does today
/// when run outside a function (unit tests, `--doctor`).
pub fn declare_locals(names: &[&str], kind: u32) {
    use crate::ported::params::{createparam, locallevel, paramtab};
    use crate::ported::zsh_h::PM_LOCAL;
    use std::sync::atomic::Ordering;

    let cur = locallevel.load(Ordering::Relaxed); // c:2469 locallevel
    if cur == 0 {
        return;
    }
    for name in names {
        // c:2469 — `(!pm || pm->level < locallevel)`.
        let needs_shadow = paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(*name).map(|pm| pm.level < cur))
            .unwrap_or(true);
        if !needs_shadow {
            continue;
        }
        // c:2470 — `createparam(pname, on | PM_LOCAL)`.
        let _ = createparam(name, (kind | PM_LOCAL) as i32);
        // c:2575 — `else if (on & PM_LOCAL) pm->level = locallevel;`
        // plus the attribute stamp so `typeset -U` keeps PM_UNIQUE.
        if let Ok(mut tab) = paramtab().write() {
            if let Some(pm) = tab.get_mut(*name) {
                pm.level = cur;
                pm.node.flags |= (kind & PM_UNIQUE) as i32;
            }
        }
    }
}

/// `local NAME="$NAME"` — declare `names` local while carrying the
/// enclosing scope's scalar value into the shadow.
///
/// Upstream spells this out where the completer chain must keep
/// reading an inherited value it is also allowed to overwrite:
/// `_main_complete:31` (`curcontext="$curcontext"`), `_tags:19`,
/// `_dispatch:4`. A bare [`declare_locals`] would hand the port an
/// empty parameter instead.
pub fn declare_locals_keeping_value(names: &[&str]) {
    for name in names {
        let inherited = crate::ported::params::getsparam(name);
        declare_locals(&[name], 0);
        if let Some(v) = inherited {
            let _ = crate::ported::params::setsparam(name, &v);
        }
    }
}
/// `is_executable` — see implementation.
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

/// `get_ignored_patterns(context)` — collect `ignored-patterns`
/// zstyle values for `context` via the real `lookupstyle` in
/// `src/ported/modules/zutil.rs`.
pub fn get_ignored_patterns(context: &str) -> Vec<String> {
    crate::ported::modules::zutil::lookupstyle(context, "ignored-patterns")
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
