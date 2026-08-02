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
//! sh:14      elif (( ${+commands[getent]} )); then
//! sh:15        getent group            → ${…%%:*}
//! sh:16      else
//! sh:17        </etc/group             → ${${…%%:*}:#+}   (+ ypcat)
//! sh:      fi
//! sh:24    groups=( "$_cache_groups[@]" )
//! sh:25  fi
//! sh:27  _wanted groups expl group compadd -a "$@" - groups
//! ```
//!
//! sh:8 approx — the `$+_cache_groups` presence check maps to
//! `getaparam("_cache_groups").is_none()`.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
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
                if _call_program(&[
                    "groups".to_string(),
                    "dscacheutil".to_string(),
                    "-q".to_string(),
                    "group".to_string(),
                ]) == 0
                {
                    c = parse_dscacheutil(&getsparam("REPLY").unwrap_or_default());
                }
            } else if _call_program(&[
                "groups".to_string(),
                "getent".to_string(),
                "group".to_string(),
            ]) == 0
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
        // sh:24
        groups = cache.unwrap_or_default();
    }

    // sh:27
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
