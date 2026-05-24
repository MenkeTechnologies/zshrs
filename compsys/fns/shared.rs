//! Tiny helpers shared between per-fn ports. Kept here (not in
//! library.rs) so the `fns/` tree stands alone.

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

/// Trivial shell-glob matcher — supports `*` and `?` only, no
/// character classes / extended globs. Sufficient for the `[(R)pat]`
/// value-match in `_widgets` and the prefix filters used by other
/// ports. For full glob semantics use `crate::matching`.
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_helper(&pat, &txt)
}

fn glob_helper(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
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
