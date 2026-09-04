//! Port of `_groups` from `Completion/Unix/Type/_groups`.
//!
//! Full upstream body (28 lines, abridged):
//! ```text
//! sh: 1  #compdef newgrp groupdel
//! sh: 3  local expl groups tmp
//! sh: 5  _tags groups || return 1
//! sh: 7  if ! zstyle -a ":completion:${curcontext}:" groups groups; then
//! sh: 8    (( $+_cache_groups )) ||
//! sh: 9      if [[ $OSTYPE = darwin* ]]; then
//! sh:10-13     lookupd / dscacheutil -q group  → names after "name: "
//! sh:15      elif (( ${+commands[getent]} )); then
//! sh:15        getent group            → ${…%%:*}
//! sh:17      else
//! sh:17        </etc/group             → ${${…%%:*}:#+}   (+ ypcat)
//! sh:      fi
//! sh:25    groups=( "$_cache_groups[@]" )
//! sh:26  fi
//! sh:28  _wanted groups expl group compadd -a "$@" - groups
//! ```
//!
//! sh:8 approx — the `$+_cache_groups` presence check maps to
//! `getaparam("_cache_groups").is_none()`.

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::compsys::ported::shared::LocalScope;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:10-13 — darwin `dscacheutil -q group` block parse: keep lines
/// beginning `name` and strip up to the last `: `.
fn parse_dscacheutil(out: &str) -> Vec<String> {
    out.lines()
        .filter(|l| l.starts_with("name"))
        .filter_map(|l| l.rsplit(": ").next().map(|s| s.to_string()))
        .collect()
}

/// `_groups` — complete user group names (getent / dscacheutil /
/// `/etc/group`), cached in `$_cache_groups`.
pub fn _groups(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_groups");
    // sh:3 — `local expl groups tmp`.
    //
    // Every name this port writes is one of those three: `groups` holds
    // the candidate list this function builds and `expl` is the array
    // `_wanted` fills in through its `$2`. Without the declaration the
    // port's `setaparam` calls create them at level 0, so a `newgrp
    // <TAB>` left a 327-element `groups` array standing in the user's
    // shell where zsh leaves the name unset:
    //
    //   zsh  : groups=[][0]        _cache_groups=[array][327]
    //   zshrs: groups=[array][327] _cache_groups=[array][327]
    //
    // Declared as scalars, exactly as upstream does; `groups` becomes an
    // array on assignment the same way `groups=( "$_cache_groups[@]" )`
    // (sh:25) converts the scalar upstream. `_cache_groups` is
    // deliberately NOT in the list — it is upstream's cross-invocation
    // cache and its whole purpose is to outlive the call (sh:8).
    //
    // [`LocalScope`] rather than a bare `declare_locals`: `_groups` is
    // also reached as a DIRECT Rust call (`_alternative` -> the action
    // dispatcher), and those never run `endparamscope`, so the shadow
    // would otherwise stay for the rest of the caller's body.
    let _locals = LocalScope::declare(&["expl", "groups", "tmp"], 0);

    // sh:5
    if _tags(&["groups".to_string()]) != 0 {
        return 1;
    }

    // sh:7
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    let mut groups = lookupstyle(&ctx, "groups");
    if groups.is_empty() {
        // sh:8 — build cache when absent.
        let mut cache = getaparam("_cache_groups");
        if cache.is_none() {
            let mut c: Vec<String> = Vec::new();
            if cfg!(target_os = "macos") {
                // sh:10-13
                if call_program_capture(&[
                    "groups".to_string(),
                    "dscacheutil".to_string(),
                    "-q".to_string(),
                    "group".to_string(),
                ])
                .1 == 0
                {
                    c = parse_dscacheutil(&getsparam("REPLY").unwrap_or_default());
                }
            } else if call_program_capture(&[
                "groups".to_string(),
                "getent".to_string(),
                "group".to_string(),
            ])
            .1 == 0
            {
                // sh:15 — first colon-field of each line.
                c = getsparam("REPLY")
                    .unwrap_or_default()
                    .lines()
                    .filter_map(|l| l.split(':').next())
                    .map(String::from)
                    .collect();
            } else if let Ok(content) = std::fs::read_to_string("/etc/group") {
                // sh:17 — first colon-field, dropping NIS `+` entries.
                c = content
                    .lines()
                    .filter_map(|l| l.split(':').next())
                    .filter(|n| *n != "+")
                    .map(String::from)
                    .collect();
            }
            setaparam("_cache_groups", c.clone());
            cache = Some(c);
        }
        // sh:25
        groups = cache.unwrap_or_default();
    }

    // sh:28
    setaparam("groups", groups);
    let mut w: Vec<String> = vec![
        "groups".to_string(),
        "expl".to_string(),
        "group".to_string(),
        "compadd".to_string(),
        "-a".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.push("groups".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _groups(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn dscacheutil_parse_extracts_names() {
        // sh:10-13 — "name: wheel" → "wheel".
        let got = parse_dscacheutil("name: wheel\npassword: *\nname: staff\n");
        assert_eq!(got, vec!["wheel".to_string(), "staff".to_string()]);
    }
}
