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
//! sh:30  local expl
//! sh:32  _wanted pods expl 'perl base pod' compadd -a "$@" - _perl_basepods
//! ```

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:25 — `${podpath}/*.pod(:r:t)`: basenames (`:t`) with the `.pod`
/// extension stripped (`:r`) of every pod file under `podpath`.
/// sh:12 `typeset -agU _perl_basepods` — the `-U`.
///
/// Stamped AFTER the assignment because `setaparam` creates the node and
/// would not carry the bit through. `pods_in` already sorts and dedups, so
/// the flag changes no value today; it matters because `_perl_basepods` is a
/// cross-invocation GLOBAL cache, so a later append by anything else must
/// dedup too — and because `${(t)_perl_basepods}` reads `array-unique` in
/// zsh. Mirrors the attribute-only update in compinit.rs's `declare_global`
/// (c:Src/builtin.c:2575), inlined because that helper is private to
/// compinit.
fn mark_unique(name: &str) {
    if let Ok(mut tab) = crate::ported::params::paramtab().write() {
        if let Some(pm) = tab.get_mut(name) {
            pm.node.flags |= crate::compsys::ported::shared::PM_UNIQUE as i32;
        }
    }
}

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
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_perl_basepods");
    // sh:11  (( ! $+_perl_basepods ))
    if getaparam("_perl_basepods").is_none() {
        // sh:14-16 — `if (( ${+commands[basepods]} )); then
        //              _perl_basepods=( ${$(basepods):t:r} )`.
        // Was never ported: the body jumped straight from sh:11 to sh:19, so
        // a host WITH `basepods` installed silently took the perl-Config
        // probe instead of the tool upstream prefers. `${…:t:r}` is tail then
        // root, i.e. basename with the extension stripped, applied to each
        // word of the command substitution.
        //
        // Upstream spells it as a bare `$(basepods)`, not `_call_program`;
        // routing it through `_call_program` matches how this file already
        // treats sh:19's equally-bare `$(perl -MConfig …)`, and so picks up
        // the `command` zstyle for both. Deliberate, and consistent within
        // the file rather than literal to upstream.
        if crate::ported::exec::findcmd("basepods", 0, 0).is_some() {
            let (out, _) = call_program_capture(&[
                "perl-basepods".to_string(),
                "basepods".to_string(),
            ]);
            let _ = out;
            let pods: Vec<String> = getsparam("REPLY")
                .unwrap_or_default()
                .split_whitespace()
                .map(|w| {
                    // `:t` — strip everything through the last `/`.
                    let tail = w.rsplit('/').next().unwrap_or(w);
                    // `:r` — strip the last `.` and what follows.
                    match tail.rfind('.') {
                        Some(i) if i > 0 => tail[..i].to_string(),
                        _ => tail.to_string(),
                    }
                })
                .collect();
            setaparam("_perl_basepods", pods);
            mark_unique("_perl_basepods");
        } else {
        // sh:19  podpath=$(perl -MConfig -e 'print "$Config{installprivlib}/pod"')
        let _ = call_program_capture(&[
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
        mark_unique("_perl_basepods");
        }
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
