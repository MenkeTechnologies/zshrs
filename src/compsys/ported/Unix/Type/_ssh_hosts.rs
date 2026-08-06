//! Port of `_ssh_hosts` from `Completion/Unix/Type/_ssh_hosts`.
//!
//! Full upstream body (51 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local -a config_hosts; local config; integer ind
//! sh: 8  if [[ "$IPREFIX" == *@ ]]; then
//! sh: 9    _combination -s '[:@]' my-accounts users-hosts "users=${IPREFIX/@}" hosts "$@" && return
//! sh:10  else
//! sh:10    _combination -s '[:@]' my-accounts users-hosts ${opt_args[-l]:+"users=${opt_args[-l]:q}"} hosts "$@" && return
//! sh:13  fi
//! sh:12  if (( ind = ${words[(I)-F]} )); then config=${~words[ind+1]}
//! sh:14  else config="$HOME/.ssh/config"; fi
//! sh:19  if [[ -r $config ]]; then
//! sh:17    … parse Host/Hostname (and Match host…, Include) lines …
//! sh:48    _wanted hosts expl 'remote host name' \
//! sh:49      compadd -M 'm:{a-zA-Z}={A-Za-z} r:|.=* r:|=*' "$@" $config_hosts
//! sh:50  fi
//! ```
//!
//! sh:17-44 the ssh_config parser (Match keyword rewrite, Include, the
//! `(Z.C.)` word split) is done with straight line/token ops
//! (`// sh:17 approx`); Host/Hostname and Include are handled, Match is
//! reduced to its `host`/`canonical`/`final` host list.

use crate::compsys::ported::_combination::_combination;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam};

/// sh:17 approx — collect non-glob host names from an ssh_config file
/// (following `Include` one level, relative to `~/.ssh`).
fn parse_config_hosts(path: &str, home: &str, depth: u8, out: &mut Vec<String>) {
    if depth > 4 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // key is delimited by `=`, tab or space (sh:19 IFS=$'=\t ').
        let mut it = line.splitn(2, |c| c == '=' || c == '\t' || c == ' ');
        let key = it.next().unwrap_or("").to_ascii_lowercase();
        let val = it.next().unwrap_or("").trim();
        match key.as_str() {
            "host" | "hostname" => {
                for host in val.split_whitespace() {
                    if !host.contains(['*', '?', '%']) {
                        out.push(host.to_string());
                    }
                }
            }
            "match" => {
                // sh:22-32 — Match canonical|final|(|original)host <list>.
                let mut toks = val.split_whitespace();
                while let Some(k) = toks.next() {
                    let kl = k.to_ascii_lowercase();
                    if matches!(kl.as_str(), "canonical" | "final" | "host" | "originalhost") {
                        if let Some(list) = toks.next() {
                            for host in list.split(',') {
                                if !host.is_empty() && !host.contains(['*', '?', '%']) {
                                    out.push(host.to_string());
                                }
                            }
                        }
                        break;
                    }
                }
            }
            "include" => {
                for inc in val.split_whitespace() {
                    let p = if inc.starts_with('/') {
                        inc.to_string()
                    } else {
                        format!("{}/.ssh/{}", home, inc)
                    };
                    parse_config_hosts(&p, home, depth + 1, out);
                }
            }
            _ => {}
        }
    }
}

/// `_ssh_hosts` — complete host names from user/host styles and ssh_config.
pub fn _ssh_hosts(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_ssh_hosts");
    let iprefix = getsparam("IPREFIX").unwrap_or_default();

    // sh:7-11 — user@host combination first; if it completes, we're done.
    let mut comb: Vec<String> = vec![
        "-s".to_string(),
        "[:@]".to_string(),
        "my-accounts".to_string(),
        "users-hosts".to_string(),
    ];
    if let Some(u) = iprefix.strip_suffix('@') {
        comb.push(format!("users={}", u));
    } else if let Some(l) = getaparam("opt_args")
        .unwrap_or_default()
        .chunks(2)
        .find(|kv| kv.first().map(|k| k == "-l").unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
    {
        comb.push(format!("users={}", l));
    }
    comb.push("hosts".to_string());
    comb.extend(args.iter().cloned());
    if _combination(&comb) == 0 {
        return 0;
    }

    // sh:12-14 — config path: `-F <file>` from words, else ~/.ssh/config.
    let home = getsparam("HOME").unwrap_or_default();
    let words = getaparam("words").unwrap_or_default();
    let config = match words.iter().position(|w| w == "-F") {
        Some(i) => words.get(i + 1).cloned().unwrap_or_default(),
        None => format!("{}/.ssh/config", home),
    };

    // sh:19-49 — parse the config and offer the host names.
    let mut config_hosts: Vec<String> = Vec::new();
    parse_config_hosts(&config, &home, 0, &mut config_hosts);
    if config_hosts.is_empty() {
        return 1;
    }
    let mut wanted_argv: Vec<String> = vec![
        "hosts".to_string(),
        "expl".to_string(),
        "remote host name".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "m:{a-zA-Z}={A-Za-z} r:|.=* r:|=*".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.extend(config_hosts);
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_host_and_skips_globs() {
        // sh:17 approx
        let dir = std::env::temp_dir().join(format!("zshrs_ssh_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let cfg = dir.join("config");
        std::fs::write(
            &cfg,
            "Host myserver gw\nHostName real.example.com\nHost *.wild\n",
        )
        .unwrap();
        let mut out = Vec::new();
        parse_config_hosts(cfg.to_str().unwrap(), "/nonexistent", 0, &mut out);
        assert!(out.contains(&"myserver".to_string()));
        assert!(out.contains(&"gw".to_string()));
        assert!(out.contains(&"real.example.com".to_string()));
        assert!(!out.iter().any(|h| h.contains('*')));
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        let _ = getsparam("HOME"); // touch
        assert_eq!(_ssh_hosts(&[]), 1);
    }
}
