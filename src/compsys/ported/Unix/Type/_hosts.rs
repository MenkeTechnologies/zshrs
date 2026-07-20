//! Port of `_hosts` from `Completion/Unix/Type/_hosts`.
//!
//! Full upstream body (78 lines, abridged):
//! ```text
//! sh: 1  #compdef ftp rwho rup xping traceroute aaaa zone mx ns soa txt
//! sh: 5  local expl _hosts tmp useip
//! sh: 7  if ! zstyle -a ":completion:${curcontext}:hosts" hosts _hosts; then
//! sh: 8    if (( $+_cache_hosts == 0 )); then
//! sh:      # use-ip style → keep IP addresses (default: strip them)
//! sh:      getent hosts  ||  </etc/hosts   (+ ypcat)   — strip leading IP,
//! sh:                                                    split on ws/tab
//! sh:      known-hosts-files style (default /etc/ssh/ssh_known_hosts,
//! sh:        ~/.ssh/known_hosts): cut at ` |#`, comma-split, drop
//! sh:        wildcards, unwrap [host]:port, drop IPs unless use-ip.
//! sh:    fi
//! sh:    _hosts=( "$_cache_hosts[@]" )
//! sh:  fi
//! sh:76 _wanted hosts expl host \
//! sh:77     compadd -a "$@" -M 'm:{a-zA-Z}={A-Za-z} r:|.=* r:|=*' - _hosts
//! ```
//!
//! sh approx — the layered `${(s: :)${(ps:\t:)${…##${~ipstrip}}}}`
//! expansions are implemented with explicit string scanning.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};

fn zstyle_t(ctx: &str, style: &str) -> bool {
    matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("yes") | Some("true") | Some("on") | Some("1")
    )
}

/// True for a pure IPv4 (`n.n.n.n`) or bare IPv6-ish (`[0-9a-f:]+`)
/// token — dropped unless `use-ip` is set (sh known_hosts filter).
fn is_ip(tok: &str) -> bool {
    let v4 = {
        let parts: Vec<&str> = tok.split('.').collect();
        parts.len() == 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let v6 = !tok.is_empty()
        && tok.bytes().all(|b| b.is_ascii_hexdigit() || b == b':')
        && tok.contains(':');
    v4 || v6
}

/// Parse a hosts-file body (getent / /etc/hosts). Each line: strip a
/// `#` comment, then the leading IP token (unless `useip`), then split
/// the remaining names on whitespace.
fn parse_hosts_body(body: &str, useip: bool) -> Vec<String> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("");
        let names = if useip {
            line.trim()
        } else {
            // strip leading whitespace + first non-blank token (the IP).
            let t = line.trim_start();
            match t.find(|c: char| c == ' ' || c == '\t') {
                Some(p) => t[p..].trim_start(),
                None => "",
            }
        };
        for tok in names.split_whitespace() {
            out.push(tok.to_string());
        }
    }
    out
}

/// Parse an ssh known_hosts file into host names.
fn parse_known_hosts(body: &str, useip: bool) -> Vec<String> {
    let mut out = Vec::new();
    for raw in body.lines() {
        // %%[ |#]* — keep text up to the first space, `|` or `#`.
        let head: String = raw
            .chars()
            .take_while(|&c| c != ' ' && c != '|' && c != '#')
            .collect();
        for entry in head.split(',') {
            if entry.is_empty() {
                continue;
            }
            // drop wildcard entries.
            if entry.contains('*') || entry.contains('?') {
                continue;
            }
            // unwrap `[hostname]:port`.
            let host = if entry.starts_with('[') {
                if let Some(close) = entry.find(']') {
                    entry[1..close].to_string()
                } else {
                    entry.to_string()
                }
            } else {
                entry.to_string()
            };
            if !useip && is_ip(&host) {
                continue;
            }
            out.push(host);
        }
    }
    out
}

/// `_hosts` — complete host names from `/etc/hosts` (or `getent`) and
/// ssh `known_hosts` files, cached in `$_cache_hosts`.
pub fn _hosts(args: &[String]) -> i32 {
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:hosts", curcontext);

    // sh:7
    let mut hosts = lookupstyle(&ctx, "hosts");
    if hosts.is_empty() {
        // sh:8 — build cache when absent.
        let mut cache = getaparam("_cache_hosts");
        if cache.is_none() {
            let useip = zstyle_t(&ctx, "use-ip");
            let mut c: Vec<String> = Vec::new();

            // getent hosts, else /etc/hosts.
            if _call_program(&[
                "hosts".to_string(),
                "getent".to_string(),
                "hosts".to_string(),
            ]) == 0
            {
                c.extend(parse_hosts_body(
                    &getsparam("REPLY").unwrap_or_default(),
                    useip,
                ));
            } else if let Ok(body) = std::fs::read_to_string("/etc/hosts") {
                c.extend(parse_hosts_body(&body, useip));
            }

            // known-hosts-files style (default list).
            let mut khostfiles = lookupstyle(&ctx, "known-hosts-files");
            if khostfiles.is_empty() {
                let home = getsparam("HOME").unwrap_or_default();
                khostfiles = vec![
                    "/etc/ssh/ssh_known_hosts".to_string(),
                    format!("{}/.ssh/known_hosts", home),
                ];
            }
            for kf in &khostfiles {
                if let Ok(body) = std::fs::read_to_string(kf) {
                    c.extend(parse_known_hosts(&body, useip));
                }
            }

            // typeset -gUa — unique, first-seen order.
            let mut seen = std::collections::HashSet::new();
            c.retain(|s| seen.insert(s.clone()));

            setaparam("_cache_hosts", c.clone());
            cache = Some(c);
        }
        hosts = cache.unwrap_or_default();
    }

    // sh:76-77
    setaparam("_hosts", hosts);
    let mut w: Vec<String> = vec![
        "hosts".to_string(),
        "expl".to_string(),
        "host".to_string(),
        "compadd".to_string(),
        "-a".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-M".to_string());
    w.push("m:{a-zA-Z}={A-Za-z} r:|.=* r:|=*".to_string());
    w.push("-".to_string());
    w.push("_hosts".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hosts_strips_ip_by_default() {
        // sh — leading IP dropped, names kept.
        let got = parse_hosts_body("127.0.0.1  localhost foo.local\n", false);
        assert_eq!(got, vec!["localhost".to_string(), "foo.local".to_string()]);
    }

    #[test]
    fn parse_hosts_keeps_ip_with_useip() {
        let got = parse_hosts_body("127.0.0.1 localhost\n", true);
        assert_eq!(got, vec!["127.0.0.1".to_string(), "localhost".to_string()]);
    }

    #[test]
    fn known_hosts_unwraps_bracket_port_and_drops_wildcards() {
        let got = parse_known_hosts(
            "[myhost.example]:2222 ssh-rsa AAA\n*.wild ssh-rsa BBB\n",
            false,
        );
        assert_eq!(got, vec!["myhost.example".to_string()]);
    }

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _hosts(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
