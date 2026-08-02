//! Port of `_jobs` from `Completion/Zsh/Type/_jobs`.
//!
//! Full upstream body (84 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  if [[ "$1" = -t ]]; then …
//! sh: 8  zstyle -t … prefix-hidden && pfx=''
//! sh: 9  zstyle -T … verbose       && desc=yes
//! sh:11  if [[ "$1" = -r ]]; then jids=( "${(@k)jobstates[(R)running*]}" )
//! sh:15  elif [[ "$1" = -s ]]; then jids=( "${(@k)jobstates[(R)suspended*]}" )
//! sh:18  else jids=( "${(@k)jobtexts}" )
//! sh:21  fi
//! sh:24  if zstyle -T … how-many; then how=$expls fi
//! sh:30  for job in $jids do … build display lines …
//! sh:80  if [[ -n "$desc" ]]; then
//! sh:81    _wanted -V jobs expl "$expls" compadd -d disp "$@" - "$jids[@]"
//! sh:82  else
//! sh:83    _wanted jobs expl "$expls" compadd "$@" "$pfx$jids[@]"
//! sh:84  fi
//! ```
//!
//! Reads `$jobtexts` / `$jobstates` assoc-arrays. Supports `-r`
//! (running only), `-s` (suspended only), `-t` (prefix-needed
//! guard).

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::testforstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::compcore::get_compstate_str;

/// Helper: assoc lookup in flat key/value layout.
fn assoc_chunks(name: &str) -> Vec<(String, String)> {
    let arr = getaparam(name).unwrap_or_default();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < arr.len() {
        out.push((arr[i].clone(), arr[i + 1].clone()));
        i += 2;
    }
    out
}

/// `_jobs` — complete job-id specs from `$jobtexts`/`$jobstates`.
/// `-r` running only; `-s` suspended only; `-t` enables
/// prefix-needed check (returns 1 if not starting with `%`).
pub fn _jobs(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_jobs");
    let mut argv = args.to_vec();
    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:5-7  -t prefix-needed guard
    if argv.first().map(|s| s == "-t").unwrap_or(false) {
        let prefix_needed =
            testforstyle(&format!(":completion:{}:jobs", curcontext), "prefix-needed") == 0;
        let prefix = getsparam("PREFIX").unwrap_or_default();
        let nm: i64 = get_compstate_str("nmatches")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if prefix_needed && !prefix.starts_with('%') && nm != 0 {
            return 1;
        }
        argv.remove(0);
    }

    // sh:8-9  styles
    let mut pfx = String::from("%");
    if testforstyle(&format!(":completion:{}:jobs", curcontext), "prefix-hidden") == 0 {
        pfx.clear();
    }
    let verbose = testforstyle(&format!(":completion:{}:jobs", curcontext), "verbose") == 0
        || crate::ported::modules::zutil::lookupstyle(
            &format!(":completion:{}:jobs", curcontext),
            "verbose",
        )
        .is_empty();

    // sh:11-21  filter
    let jobstates = assoc_chunks("jobstates");
    let jobtexts = assoc_chunks("jobtexts");
    let (jids, expls): (Vec<String>, String) = match argv.first().map(|s| s.as_str()) {
        Some("-r") => {
            argv.remove(0);
            (
                jobstates
                    .iter()
                    .filter(|(_, v)| v.starts_with("running"))
                    .map(|(k, _)| k.clone())
                    .collect(),
                "running job".to_string(),
            )
        }
        Some("-s") => {
            argv.remove(0);
            (
                jobstates
                    .iter()
                    .filter(|(_, v)| v.starts_with("suspended"))
                    .map(|(k, _)| k.clone())
                    .collect(),
                "suspended job".to_string(),
            )
        }
        _ => (
            jobtexts.iter().map(|(k, _)| k.clone()).collect(),
            "job".to_string(),
        ),
    };

    if jids.is_empty() {
        return 1;
    }

    // sh:30+  build display lines (verbose only)
    if verbose {
        let mut disp: Vec<String> = Vec::new();
        for j in &jids {
            let text = jobtexts
                .iter()
                .find(|(k, _)| k == j)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            disp.push(format!("%{}\t{}", j, text));
        }
        setaparam("disp", disp);
        // sh:81
        let mut w_args: Vec<String> = vec![
            "-V".to_string(),
            "jobs".to_string(),
            "expl".to_string(),
            expls,
            "compadd".to_string(),
            "-d".to_string(),
            "disp".to_string(),
        ];
        w_args.extend(argv);
        w_args.push("-".to_string());
        w_args.extend(jids);
        _wanted(&w_args)
    } else {
        // sh:83
        let mut w_args: Vec<String> = vec![
            "jobs".to_string(),
            "expl".to_string(),
            expls,
            "compadd".to_string(),
        ];
        w_args.extend(argv);
        for j in &jids {
            w_args.push(format!("{}{}", pfx, j));
        }
        _wanted(&w_args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn empty_jobs_returns_one() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        setaparam("jobtexts", Vec::new());
        setaparam("jobstates", Vec::new());
        assert_eq!(_jobs(&[]), 1);
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }
}
