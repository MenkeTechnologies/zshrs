//! Port of `_perl_basepods` from `Completion/Unix/Type/_perl_basepods`.
//!
//! Full upstream body (32 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:11  if (( ! $+_perl_basepods )); then
//! sh:12    typeset -agU _perl_basepods
//! sh:14    if (( ${+commands[basepods]} )); then
//! sh:15      _perl_basepods=( ${$(basepods):t:r} )
//! sh:16    else
//! sh:19      podpath=$(perl -MConfig -e 'print "$Config{installprivlib}/pod"')
//! sh:21      if [[ ! -e $podpath/perl.pod ]]; then
//! sh:22        _message "can't find perl.pod from Config.pm; giving up"
//! sh:23        return 1
//! sh:24      else
//! sh:25        _perl_basepods=( ${podpath}/*.pod(:r:t) )
//! sh:26      fi
//! sh:27    fi
//! sh:28  fi
//! sh:31  local expl
//! sh:32  _wanted pods expl 'perl base pod' compadd -a "$@" - _perl_basepods
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:25 — `${podpath}/*.pod(:r:t)`: basenames (`:t`) with the `.pod`
/// extension stripped (`:r`) of every pod file under `podpath`.
fn pods_in(podpath: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(podpath) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(base) = name.strip_suffix(".pod") {
                out.push(base.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `_perl_basepods` — complete installed Perl base pod names
/// (`perlfunc`, `perlfaq`, …). Result cached in `_perl_basepods`.
pub fn _perl_basepods(args: &[String]) -> i32 {
    // sh:11  (( ! $+_perl_basepods ))
    if getaparam("_perl_basepods").is_none() {
        // sh:19  podpath=$(perl -MConfig -e 'print "$Config{installprivlib}/pod"')
        let _ = _call_program(&[
            "perl-basepods".to_string(),
            "perl".to_string(),
            "-MConfig".to_string(),
            "-e".to_string(),
            "print \"$Config{installprivlib}/pod\"".to_string(),
        ]);
        let podpath = getsparam("REPLY").unwrap_or_default();
        // sh:21  [[ ! -e $podpath/perl.pod ]]
        if podpath.is_empty() || !std::path::Path::new(&format!("{}/perl.pod", podpath)).exists() {
            // sh:22-23
            let _ = _message(&["can't find perl.pod from Config.pm; giving up".to_string()]);
            return 1;
        }
        // sh:25
        setaparam("_perl_basepods", pods_in(&podpath));
    }

    // sh:32  _wanted pods expl 'perl base pod' compadd -a "$@" - _perl_basepods
    let mut w = vec![
        "pods".to_string(),
        "expl".to_string(),
        "perl base pod".to_string(),
        "compadd".to_string(),
        "-a".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.push("_perl_basepods".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_with_cached_empty_pods() {
        let _g = crate::test_util::global_state_lock();
        // Pre-seed the cache so the perl probe is skipped; empty → no tags.
        setaparam("_perl_basepods", Vec::new());
        assert_eq!(_perl_basepods(&[]), 1);
    }
}
