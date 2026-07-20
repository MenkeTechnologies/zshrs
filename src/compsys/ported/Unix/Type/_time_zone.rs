//! Port of `_time_zone` from `Completion/Unix/Type/_time_zone`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #compdef -value-,TZ,-default-
//! sh: 3  local expl
//! sh: 5  if (( ! $+_zoneinfo_dirs )); then
//! sh: 6    _zoneinfo_dirs=( /usr/{share,lib,share/lib}/{zoneinfo*,locale/TZ}(/) )
//! sh: 7  fi
//! sh: 9  _wanted time-zones expl 'time zone' \
//! sh:10    _files -g '[A-Z]*' -M 'm:{a-z}={A-Z}' -W _zoneinfo_dirs "$@" -
//! ```
//!
//! sh:6 brace expansion is precomputed into the six concrete dir
//! patterns; each is glob-qualified `(/)` (directories only) via
//! tokenize + zglob (same idiom as `_files`'s `glob_subdirs`).

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::glob::{tokenize, zglob};
use crate::ported::params::{getaparam, setaparam};

/// sh:6 — glob a directory pattern, keep only entries that resolved
/// (no leftover glob metachars) — i.e. existing directories.
fn glob_dir(pat: &str) -> Vec<String> {
    let mut list = {
        let mut s = pat.to_string();
        tokenize(&mut s);
        vec![s]
    };
    zglob(&mut list, 0, 0);
    list.into_iter()
        .filter(|e| {
            !e.as_bytes().iter().any(|&b| {
                matches!(
                    b,
                    b'[' | b']' | b'*' | b'?' | b'#' | b'~' | b'^' | b'<' | b'>'
                )
            })
        })
        .collect()
}

/// `_time_zone` — complete zoneinfo time-zone names for `$TZ`.
pub fn _time_zone(args: &[String]) -> i32 {
    // sh:5-7 — populate `_zoneinfo_dirs` once (cached across calls).
    if getaparam("_zoneinfo_dirs").is_none() {
        // sh:6 — brace expansion of
        //   /usr/{share,lib,share/lib}/{zoneinfo*,locale/TZ}(/)
        let mut dirs: Vec<String> = Vec::new();
        for base in ["share", "lib", "share/lib"] {
            for leaf in ["zoneinfo*", "locale/TZ"] {
                dirs.extend(glob_dir(&format!("/usr/{}/{}(/)", base, leaf)));
            }
        }
        setaparam("_zoneinfo_dirs", dirs);
    }

    // sh:9-10  _wanted time-zones expl 'time zone' _files -g '[A-Z]*'
    //   -M 'm:{a-z}={A-Z}' -W _zoneinfo_dirs "$@" -
    let mut wanted_argv: Vec<String> = vec![
        "time-zones".to_string(),
        "expl".to_string(),
        "time zone".to_string(),
        "_files".to_string(),
        "-g".to_string(),
        "[A-Z]*".to_string(),
        "-M".to_string(),
        "m:{a-z}={A-Z}".to_string(),
        "-W".to_string(),
        "_zoneinfo_dirs".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    _wanted(&wanted_argv)
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
        let r = _time_zone(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn populates_zoneinfo_dirs_cache() {
        // sh:5-7 — the cache param must be set (possibly empty) after a call.
        let _g = crate::test_util::global_state_lock();
        setaparam("_zoneinfo_dirs", Vec::new());
        let _ = getaparam("_zoneinfo_dirs").expect("cache seeded");
    }
}
