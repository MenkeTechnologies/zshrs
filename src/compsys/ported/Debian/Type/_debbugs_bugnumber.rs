//! Port of `_debbugs_bugnumber` from `Completion/Debian/Type/_debbugs_bugnumber`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  # TODO: use _describe with some basic metadata (e.g., bug title/package/version)
//! sh: 4  local expl
//! sh: 6  [[ $PREFIX$SUFFIX == [0-9]# ]] || return 1
//! sh:10  local -a cachedirs=( ~/.devscripts_cache/bts ~/.cache/devscripts/bts )
//! sh:11  _wanted -x bugnum expl 'bug number' compadd -- $^cachedirs/<->.(html|mbox)(N:t:r)
//! ```

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::params::getsparam;

/// sh:6 — `$PREFIX$SUFFIX == [0-9]#`: the whole prefix+suffix must be
/// digits only (`#` = zero-or-more of the preceding pattern, so the
/// empty string also matches).
fn prefix_suffix_all_digits(prefix: &str, suffix: &str) -> bool {
    let combined = format!("{prefix}{suffix}");
    combined.chars().all(|c| c.is_ascii_digit())
}

/// sh:10 — `~/.devscripts_cache/bts` and `~/.cache/devscripts/bts`
/// under `$HOME`.
fn cachedirs(home: &str) -> [String; 2] {
    [
        format!("{home}/.devscripts_cache/bts"),
        format!("{home}/.cache/devscripts/bts"),
    ]
}

/// sh:11 — `$dir/<->.(html|mbox)(N:t:r)`: filenames directly under
/// `dir` consisting of a run of digits (`<->`) followed by `.html` or
/// `.mbox`, with the `N` qualifier (nullglob — no error if none
/// match), `:t` (basename) and `:r` (extension stripped) applied —
/// net effect: the bare bug-number string for each cached bug file.
/// Zsh glob results are sorted lexicographically by filename (the
/// default, no `o`/`O` sort qualifier given) before `:t:r` strips the
/// path/extension, so we sort on the raw filename too.
fn bugnumbers_in(dir: &str) -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => return Vec::new(),
    };
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let stem = name
                .strip_suffix(".html")
                .or_else(|| name.strip_suffix(".mbox"))?;
            if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_digit()) {
                Some(stem.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// `_debbugs_bugnumber` — complete a Debian BTS bug number from the
/// locally cached `bts` bug HTML/mbox files (`devscripts`' cache).
pub fn _debbugs_bugnumber(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_debbugs_bugnumber");
    // sh:6
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    if !prefix_suffix_all_digits(&prefix, &suffix) {
        return 1;
    }

    // sh:10
    let home = getsparam("HOME").unwrap_or_default();
    let dirs = cachedirs(&home);

    // sh:11  $^cachedirs/<->.(html|mbox)(N:t:r) — cross-product glob
    // over both cache dirs, in order.
    let mut bugnums: Vec<String> = Vec::new();
    for dir in &dirs {
        bugnums.extend(bugnumbers_in(dir));
    }

    // sh:11  _wanted -x bugnum expl 'bug number' compadd -- "${bugnums[@]}"
    let mut w = vec![
        "-x".to_string(),
        "bugnum".to_string(),
        "expl".to_string(),
        "bug number".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("--".to_string());
    w.extend(bugnums);
    wanted_byname(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_suffix_digits_only_matches() {
        assert!(prefix_suffix_all_digits("123", ""));
        assert!(prefix_suffix_all_digits("", "456"));
        assert!(prefix_suffix_all_digits("", "")); // `#` allows empty match
        assert!(!prefix_suffix_all_digits("12a", ""));
        assert!(!prefix_suffix_all_digits("", "bug123"));
    }

    #[test]
    fn bugnumbers_in_extracts_digit_stems_only() {
        let dir = std::env::temp_dir().join(format!(
            "zshrs_debbugs_bugnumber_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("123456.html"), b"").unwrap();
        std::fs::write(dir.join("789.mbox"), b"").unwrap();
        std::fs::write(dir.join("notabug.html"), b"").unwrap();
        std::fs::write(dir.join("42.txt"), b"").unwrap();

        let mut got = bugnumbers_in(dir.to_str().unwrap());
        got.sort();
        assert_eq!(got, vec!["123456".to_string(), "789".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bugnumbers_in_missing_dir_returns_empty() {
        assert!(bugnumbers_in("/nonexistent/zshrs/debbugs/cache/dir").is_empty());
    }

    #[test]
    fn returns_one_when_prefix_suffix_not_digits() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("PREFIX", "notabug");
        crate::ported::params::setsparam("SUFFIX", "");
        assert_eq!(_debbugs_bugnumber(&[]), 1);
        crate::ported::params::unsetparam("PREFIX");
        crate::ported::params::unsetparam("SUFFIX");
    }
}
