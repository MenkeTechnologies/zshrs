//! Port of `_most_recent_file` from
//! `Completion/Base/Widget/_most_recent_file`.
//!
//! Full upstream body (24 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xm
//! sh: 3  # Complete the most recently modified file matching the pattern
//! sh:11  local file tilde etilde
//! sh:12  if [[ $PREFIX = \~*/* ]]; then
//! sh:13    tilde=${PREFIX%%/*}
//! sh:14    etilde=${~tilde} 2>/dev/null
//! sh:17    eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"
//! sh:18    file=(${file/#$etilde})
//! sh:19    file=($tilde${(q)^file})
//! sh:20  else
//! sh:21    eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"
//! sh:22    file=(${(q)file})
//! sh:23  fi
//! sh:24  (( $#file )) && compadd -U -i "$IPREFIX" -I "$ISUFFIX" -f -Q -- $file
//! ```
//!
//! sh:17 / sh:21 run through the real `eval` ([`eval_comp`]): the word is
//! `$PREFIX*$SUFFIX(om[N]N)`, so everything the globber does — `GLOB_DOTS`
//! (a bare `*` skips dot files), pattern characters typed on the line,
//! `~`/`=` expansion, path components, the `om` sort and its `[N]` index —
//! comes from the engine instead of a re-implementation of it. An earlier
//! version listed the directory with `std::fs` and matched names by
//! `starts_with`, so `^Xm` on an empty word offered `.git` where zsh offers
//! the newest non-dot file.

use crate::compsys::ported::shared::{declare_locals, eval_comp};
use crate::ported::glob::shtokenize;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::subst::filesubstr;
use crate::ported::utils::{errflag, quotestring};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, ERRFLAG_ERROR, MAX_OPS, QT_BACKSLASH};
use std::sync::atomic::Ordering;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:17 / sh:21 — `eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"`.
/// The string is built from the RAW parameter values, exactly as the
/// double-quoted `eval` argument is, so quoting typed on the line reaches
/// the parser intact (sh:15-16).
fn eval_glob(prefix: &str, suffix: &str, line: u64) -> Vec<String> {
    // `${NUMERIC:-1}` — unset OR empty falls back to 1.
    let numeric = getsparam("NUMERIC")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "1".to_string());
    eval_comp(&format!("file=({prefix}*{suffix}(om[{numeric}]N))"), line);
    getaparam("file").unwrap_or_default()
}

/// `_most_recent_file` — `\C-xm` widget: complete the Nth most recently
/// modified file matching the word on the line.
pub fn _most_recent_file() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_most_recent_file");
    // sh:11
    declare_locals(&["file", "tilde", "etilde"], 0);
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();

    // sh:12 — `[[ $PREFIX = \~*/* ]]`: a literal `~`, then a `/` somewhere.
    let file: Vec<String> = if prefix.starts_with('~') && prefix[1..].contains('/') {
        // sh:13 — `tilde=${PREFIX%%/*}`
        let tilde = prefix[..prefix.find('/').unwrap()].to_string();
        let _ = setsparam("tilde", &tilde);
        // sh:14 — `etilde=${~tilde} 2>/dev/null`. `~` turns on GLOB_SUBST,
        // so the value is shtokenized and then file-expanded like any
        // unquoted word; the redirection only hides the diagnostic. A
        // failed expansion (unknown `~user` under NOMATCH) still raises the
        // error, and the function stops there.
        let mut tok = tilde.clone();
        shtokenize(&mut tok);
        let etilde = filesubstr(&tok, true).unwrap_or_else(|| tilde.clone());
        if errflag.load(Ordering::SeqCst) & ERRFLAG_ERROR != 0 {
            return 1;
        }
        let _ = setsparam("etilde", &etilde);
        // sh:17
        let globbed = eval_glob(&prefix, &suffix, 17);
        // sh:18 — `file=(${file/#$etilde})`: `$etilde` is not `~`-flagged,
        // so it is matched literally, anchored at the start.
        // sh:19 — `file=($tilde${(q)^file})`
        globbed
            .iter()
            .map(|f| {
                let rest = f.strip_prefix(etilde.as_str()).unwrap_or(f);
                format!("{tilde}{}", quotestring(rest, QT_BACKSLASH))
            })
            .collect()
    } else {
        // sh:21-22 — `file=(${(q)file})`
        eval_glob(&prefix, &suffix, 21)
            .iter()
            .map(|f| quotestring(f, QT_BACKSLASH))
            .collect()
    };
    setaparam("file", file.clone());

    // sh:24 — `(( $#file )) && compadd -U -i "$IPREFIX" -I "$ISUFFIX" -f -Q -- $file`
    if file.is_empty() {
        return 1;
    }
    let mut argv: Vec<String> = vec![
        "-U".to_string(),
        "-i".to_string(),
        getsparam("IPREFIX").unwrap_or_default(),
        "-I".to_string(),
        getsparam("ISUFFIX").unwrap_or_default(),
        "-f".to_string(),
        "-Q".to_string(),
        "--".to_string(),
    ];
    argv.extend(file);
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
