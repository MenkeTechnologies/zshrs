//! Port of `_terminals` from `Completion/Unix/Type/_terminals`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #compdef infocmp -value-,TERM,-default-
//! sh:3  local desc expl
//! sh:5  desc=( $TERMINFO ~/.terminfo $TERMINFO_DIRS /usr/{,share/}{,lib/}terminfo /{etc,lib}/terminfo )
//! sh:7  _wanted terminals expl 'terminal name' \
//! sh:8      compadd "$@" - $desc/*/*(N:t)
//! ```
//!
//! sh:5 brace expansion is precomputed into the concrete terminfo
//! directory list; sh:8 globs `dir/*/*` two levels deep with the `N`
//! (nullglob) qualifier and takes the basename (`:t`) of each match.

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::glob::{tokenize, zglob};
use crate::ported::params::getsparam;

/// Glob `pat` (nullglob) and return the basename of each match.
fn glob_basenames(pat: &str) -> Vec<String> {
    let mut list = {
        let mut s = pat.to_string();
        tokenize(&mut s);
        vec![s]
    };
    zglob(&mut list, 0, 0);
    list.into_iter()
        .filter(|e| {
            // Drop entries that never matched (glob metachars survive).
            !e.as_bytes()
                .iter()
                .any(|&b| matches!(b, b'*' | b'?' | b'[' | b']'))
        })
        .map(|p| {
            p.rsplit('/')
                .next()
                .map(str::to_string)
                .unwrap_or(p.clone())
        })
        .collect()
}

/// `_terminals` — complete terminal names from the terminfo database.
pub fn _terminals(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_terminals");
    // sh:5 — desc=( $TERMINFO ~/.terminfo $TERMINFO_DIRS
    //   /usr/{,share/}{,lib/}terminfo /{etc,lib}/terminfo )
    let home = getsparam("HOME").unwrap_or_default();
    let mut desc: Vec<String> = Vec::new();
    if let Some(ti) = getsparam("TERMINFO").filter(|s| !s.is_empty()) {
        desc.push(ti);
    }
    desc.push(format!("{}/.terminfo", home));
    if let Some(tid) = getsparam("TERMINFO_DIRS").filter(|s| !s.is_empty()) {
        // `$TERMINFO_DIRS` is colon-separated in the environment.
        desc.extend(tid.split(':').filter(|s| !s.is_empty()).map(String::from));
    }
    // /usr/{,share/}{,lib/}terminfo — the four brace-cross members.
    for mid in ["", "share/", "lib/", "share/lib/"] {
        desc.push(format!("/usr/{}terminfo", mid));
    }
    // /{etc,lib}/terminfo
    desc.push("/etc/terminfo".to_string());
    desc.push("/lib/terminfo".to_string());

    // sh:8 — compadd "$@" - $desc/*/*(N:t)
    let mut names: Vec<String> = Vec::new();
    for d in &desc {
        names.extend(glob_basenames(&format!("{}/*/*", d)));
    }

    let mut wanted_argv: Vec<String> = vec![
        "terminals".to_string(),
        "expl".to_string(),
        "terminal name".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    wanted_argv.extend(names);
    wanted_byname(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _terminals(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
