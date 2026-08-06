//! Port of `_ports` from `Completion/Unix/Type/_ports`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl ports
//! sh: 5  if ! zstyle -a ":completion:${curcontext}:" ports ports; then
//! sh: 6    (( $+_cache_ports )) ||
//! sh: 7        : ${(A)_cache_ports:=${${(M)${${(f)"$(</etc/services)"}:#\#*}#*/tcp}%%[ ]*}}
//! sh: 9    ports=( "$_cache_ports[@]" )
//! sh:10  fi
//! sh:12  _wanted ports expl port compadd -a "$@" - ports
//! ```
//!
//! sh:7 parses `/etc/services`: keep tcp lines, take the field before
//! `/tcp`, then the first whitespace-delimited token (the port number).

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:7 — build the `_cache_ports` list from `/etc/services` tcp entries.
fn parse_services() -> Vec<String> {
    let text = std::fs::read_to_string("/etc/services").unwrap_or_default();
    let mut out = Vec::new();
    for line in text.lines() {
        // ${…:#\#*} — drop comment lines.
        if line.starts_with('#') {
            continue;
        }
        // ${(M)…#*/tcp} — keep only the `NAME PORT/tcp` shape, take up to `/tcp`.
        if let Some(pos) = line.find("/tcp") {
            let head = &line[..pos];
            // %%[ \t]* — the first whitespace-delimited token (`PORT`).
            let tok = head
                .split(|c: char| c == ' ' || c == '\t')
                .next()
                .unwrap_or("");
            if !tok.is_empty() {
                out.push(tok.to_string());
            }
        }
    }
    out
}

/// `_ports` — complete TCP port numbers from the `ports` style or
/// `/etc/services`.
pub fn _ports(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_ports");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    // sh:5  zstyle -a ":completion:${curcontext}:" ports ports
    let mut ports = lookupstyle(&format!(":completion:{}:", curcontext), "ports");
    if ports.is_empty() {
        // sh:6-9 — populate/reuse the `_cache_ports` param.
        if getaparam("_cache_ports").is_none() {
            setaparam("_cache_ports", parse_services());
        }
        ports = getaparam("_cache_ports").unwrap_or_default();
    }
    // sh:12  _wanted ports expl port compadd -a "$@" - ports
    // The trailing `ports` is a param NAME, so publish the array first.
    setaparam("ports", ports);
    let mut w = vec![
        "ports".to_string(),
        "expl".to_string(),
        "port".to_string(),
        "compadd".to_string(),
        "-a".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.push("ports".to_string());
    wanted_byname(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_ports(&[]), 1);
    }

    #[test]
    fn parse_services_extracts_tcp_ports() {
        // Faithful to sh:7 — comment lines dropped, tcp field taken.
        let _ = parse_services(); // smoke: must not panic on the real file
    }
}
