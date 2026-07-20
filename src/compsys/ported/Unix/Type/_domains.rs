//! Port of `_domains` from `Completion/Unix/Type/_domains`.
//!
//! Full upstream body (20 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl domains tmp
//! sh: 5  if ! zstyle -a ":completion:${curcontext}:domains" domains domains; then
//! sh: 6    if (( ! $+_cache_domains )); then
//! sh: 7      _cache_domains=()
//! sh: 8      if [[ -f /etc/resolv.conf ]]; then
//! sh: 9        while read tmp; do
//! sh:10          [[ "$tmp" = (domain|search)* ]] &&
//! sh:11              _cache_domains=( "$_cache_domains[@]" "${=${tmp%%[ \t]#}#*[ \t]}" )
//! sh:12        done < /etc/resolv.conf
//! sh:13        _cache_domains=( "${(@)_cache_domains:#[ \t]#}" )
//! sh:14      fi
//! sh:15    fi
//! sh:16    domains=( "$_cache_domains[@]" )
//! sh:17  fi
//! sh:19  _wanted domains expl domain \
//! sh:20      compadd -M 'm:{a-zA-Z}={A-Za-z} r:|.=* r:|=*' -a "$@" - domains
//! ```
//!
//! sh:11 approx — the `${=${tmp%%[ \t]#}#*[ \t]}` expansion (strip
//! trailing whitespace, drop the leading keyword up to the first
//! whitespace, then IFS-split the remainder) is done with string ops.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// `_domains` — complete DNS domain names from `/etc/resolv.conf`
/// (`domain` / `search` lines), cached in `$_cache_domains`.
pub fn _domains(args: &[String]) -> i32 {
    // sh:5
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:domains", curcontext);
    let mut domains = lookupstyle(&ctx, "domains");
    if domains.is_empty() {
        // sh:6 — cache presence check (approx: None == unset).
        let mut cache = getaparam("_cache_domains");
        if cache.is_none() {
            // sh:7-14
            let mut c: Vec<String> = Vec::new();
            if let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") {
                for line in content.lines() {
                    // sh:10
                    if line.starts_with("domain") || line.starts_with("search") {
                        // sh:11 approx
                        let trimmed = line.trim_end_matches([' ', '\t']);
                        if let Some(pos) = trimmed.find([' ', '\t']) {
                            for w in trimmed[pos..].split_whitespace() {
                                c.push(w.to_string());
                            }
                        }
                    }
                }
                // sh:13 — drop blank-only elements.
                c.retain(|s| !s.chars().all(|ch| ch == ' ' || ch == '\t'));
            }
            setaparam("_cache_domains", c.clone());
            cache = Some(c);
        }
        // sh:16
        domains = cache.unwrap_or_default();
    }

    // sh:19-20
    setaparam("domains", domains);
    let mut w: Vec<String> = vec![
        "domains".to_string(),
        "expl".to_string(),
        "domain".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "m:{a-zA-Z}={A-Za-z} r:|.=* r:|=*".to_string(),
        "-a".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.push("domains".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _domains(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
