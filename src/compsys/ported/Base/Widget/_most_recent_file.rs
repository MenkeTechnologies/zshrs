//! Port of `_most_recent_file` from
//! `Completion/Base/Widget/_most_recent_file`.
//!
//! Full upstream body (24 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xm
//! sh: 3  # Complete the most recently modified file matching the pattern
//! sh: 9  local file tilde etilde
//! sh:10  if [[ $PREFIX = \~*/* ]]; then
//! sh:11    tilde=${PREFIX%%/*}
//! sh:12    etilde=${~tilde} 2>/dev/null
//! sh:15    eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"
//! sh:16    file=(${file/#$etilde})
//! sh:17    file=($tilde${(q)^file})
//! sh:18  else
//! sh:21    eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"
//! sh:22    file=(${(q)file})
//! sh:23  fi
//! sh:24  (( $#file )) && compadd -U -i "$IPREFIX" -I "$ISUFFIX" -f -Q -- $file
//! ```
//!
//! `(om[N])` is the `o`rder-by-`m`odification-time glob qualifier
//! selecting the Nth oldest match. We replicate via std::fs +
//! mtime sort instead of evaluating zsh-glob expressions inline.

use crate::ported::params::{getiparam, getsparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};
use std::fs;
use std::path::Path;
use std::time::SystemTime;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:15/sh:21 — glob `$PREFIX*$SUFFIX` then pick the Nth by mtime
/// (N from `$NUMERIC`, default 1, negative = oldest direction).
fn pick_nth_recent(prefix: &str, suffix: &str, n: i64) -> Option<String> {
    // Determine search dir from PREFIX.
    let combined = format!("{}{}", prefix, suffix);
    let (dir, fname_prefix) = match combined.rfind('/') {
        Some(i) => (combined[..i].to_string(), combined[i + 1..].to_string()),
        None => (".".to_string(), combined.clone()),
    };
    let (real_prefix, real_suffix) = if let Some(slash) = prefix.rfind('/') {
        (prefix[slash + 1..].to_string(), suffix.to_string())
    } else {
        (prefix.to_string(), suffix.to_string())
    };

    let _ = fname_prefix; // used for context only

    let entries = fs::read_dir(Path::new(&dir)).ok()?;
    let mut matches: Vec<(SystemTime, String)> = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        if name.starts_with(&real_prefix) && name.ends_with(&real_suffix) {
            if let Ok(meta) = ent.metadata() {
                if let Ok(mtime) = meta.modified() {
                    let full = if dir == "." {
                        name
                    } else {
                        format!("{}/{}", dir, name)
                    };
                    matches.push((mtime, full));
                }
            }
        }
    }
    if matches.is_empty() {
        return None;
    }
    // `o`m = ascending mtime (oldest first); `O`m would be descending.
    //   `om[N]` indexes 1-based from oldest. Negative N (zsh `om[-1]`)
    //   is newest. We mirror by sorting newest-first then taking
    //   N-1 for positive, or |N|-1 from oldest for negative.
    matches.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
    let idx = if n >= 0 {
        (n.saturating_sub(1)) as usize
    } else {
        matches.len().saturating_sub((-n) as usize)
    };
    matches.get(idx).map(|t| t.1.clone())
}

/// `_most_recent_file` — `\C-xm` widget: insert the Nth most-recent
/// file matching the glob on the current line.
pub fn _most_recent_file() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_most_recent_file");
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let numeric = getiparam("NUMERIC");
    let n = if numeric == 0 { 1 } else { numeric };

    let picked = match pick_nth_recent(&prefix, &suffix, n) {
        Some(f) => f,
        None => return 1,
    };

    // sh:24
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let argv: Vec<String> = vec![
        "-U".to_string(),
        "-i".to_string(),
        iprefix,
        "-I".to_string(),
        isuffix,
        "-f".to_string(),
        "-Q".to_string(),
        "--".to_string(),
        picked,
    ];
    bin_compadd("compadd", &argv, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn no_match_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "/nonexistent/path/prefix");
        let _ = setsparam("SUFFIX", "");
        assert_eq!(_most_recent_file(), 1);
    }
}
