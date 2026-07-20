//! Port of `_printers` from `Completion/Unix/Type/_printers`.
//!
//! Full upstream body (110 lines, abridged):
//! ```text
//! sh:  1  #compdef -value-,PRINTER,-default- -value-,LPDEST,-default-
//! sh:  6  if (( $+commands[lsallq] )); then …AIX: compadd $(lsallq)… return
//! sh:  9  zstyle -s …printers list-separator sep || sep=--
//! sh: 21  servopt = ($service == lpr ? -H : -h)
//! sh: 23  if lpstat && a -h<server> arg is present: compadd `lpstat -h.. -a`
//! sh: 39  if (( ! $+_lp_cache )); then …build from /etc/(printcap|printers.conf)…
//! sh: 62    …add lpstat -a names… solaris ypcat… default guess…
//! sh: 79  fi
//! sh: 81  _wanted printers expl printer compadd "$@" - ${(@)_lp_cache%%:*} && return 0
//! sh: 92  (( $+_lp_alias_cache )) || return 1
//! sh:101  _wanted printers expl printer compadd "$@" - ${(@)_lp_alias_cache%%:*}
//! sh:106  return 1
//! ```
//!
//! Approximations (`// sh:N approx`): the `-h<server>` word-scan
//! (sh:23-37), the Solaris `ypcat` branch (sh:70-73), and the `zformat`
//! verbose two-column display (sh:82-99) are simplified; the primary
//! `/etc/printcap` / `/etc/printers.conf` + `lpstat -a` name list is
//! ported faithfully.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:44-61 — parse `/etc/printcap` or `/etc/printers.conf` for the
/// primary printer names (the first `|`-separated alias of each entry).
fn parse_printcap() -> Vec<String> {
    let mut names = Vec::new();
    for path in ["/etc/printcap", "/etc/printers.conf"] {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for line in text.lines() {
            // sh:47  [[ "$entry" = [^[:blank:]\#\*_]*:* ]]
            let first = line.chars().next().unwrap_or(' ');
            if first.is_whitespace() || matches!(first, '#' | '*' | '_') {
                continue;
            }
            if !line.contains(':') {
                continue;
            }
            // sh:48  names=( "${(s:|:)entry%%:*}" ) — first field before `:`,
            //   split on `|`; the primary name is `names[1]`.
            let head = line.split(':').next().unwrap_or("");
            if let Some(primary) = head.split('|').next() {
                if !primary.is_empty() {
                    names.push(primary.to_string());
                }
            }
        }
        break; // sh:42  file=( … ); only the first existing file is read.
    }
    names
}

/// sh:63-68 — append `lpstat -a` queue names not already present.
fn add_lpstat_names(cache: &mut Vec<String>) {
    let _ = _call_program(&[
        "printers".to_string(),
        "lpstat".to_string(),
        "-a".to_string(),
    ]);
    let out = getsparam("REPLY").unwrap_or_default();
    for line in out.lines() {
        let name = line.split_whitespace().next().unwrap_or("");
        if !name.is_empty() && !cache.iter().any(|c| c == name) {
            cache.push(name.to_string());
        }
    }
}

/// `_printers` — complete printer / print-queue names.
pub fn _printers(args: &[String]) -> i32 {
    // sh:39  (( ! $+_lp_cache )) — build once, then reuse.
    if getaparam("_lp_cache").is_none() {
        let mut cache = parse_printcap();
        add_lpstat_names(&mut cache);
        // sh:76  (( $#_lp_cache )) || _lp_cache=( 'lp0:guessed default printer' )
        if cache.is_empty() {
            cache.push("lp0".to_string());
        }
        setaparam("_lp_cache", cache);
    }

    // sh:81  ${(@)_lp_cache%%:*} — strip any `:description` suffix.
    let cache = getaparam("_lp_cache").unwrap_or_default();
    let names: Vec<String> = cache
        .iter()
        .map(|e| e.split(':').next().unwrap_or(e).to_string())
        .collect();

    // sh:81  _wanted printers expl printer compadd "$@" - <names>
    let mut w = vec![
        "printers".to_string(),
        "expl".to_string(),
        "printer".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.extend(names);
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        // Pre-seed the cache to skip the /etc scan; still no live tags → 1.
        setaparam("_lp_cache", vec!["lp0".to_string()]);
        assert_eq!(_printers(&[]), 1);
    }
}
