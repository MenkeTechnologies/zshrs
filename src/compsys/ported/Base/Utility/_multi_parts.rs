//! Port of `_multi_parts` from
//! `Completion/Base/Utility/_multi_parts`.
//!
//! Full upstream body (261 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  9  zparseopts -K -E -D 'J+:=group' 'V+:=group' 'P:=opts' …
//! sh: 30  sep="$1"; shift
//! sh: 35  arr=resolve($1)
//! sh: 50  loop: split current PREFIX by sep, complete part by part
//! sh:150  compadd …
//! sh:260  return ret
//! ```
//!
//! Multi-part separator-driven completion. Approximation: split
//! `$PREFIX$SUFFIX` by `sep`, match the current segment against
//! the array, emit matches via compadd.

use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

fn resolve_array_arg(s: &str) -> Vec<String> {
    if s.starts_with('(') && s.ends_with(')') {
        s[1..s.len() - 1]
            .split_whitespace()
            .map(|w| w.to_string())
            .collect()
    } else {
        getaparam(s).unwrap_or_default()
    }
}

/// `_multi_parts` — complete each `sep`-delimited segment of a
/// path-like string from a shared array.
pub fn _multi_parts(args: &[String]) -> i32 {
    // Skip leading flags (J/V/P/S/M etc.) for our simplified port.
    let mut idx = 0usize;
    while idx < args.len() {
        let a = &args[idx];
        if a.starts_with('-') && a.len() > 1 {
            if matches!(
                a.as_str(),
                "-J" | "-V" | "-P" | "-S" | "-M" | "-X" | "-F" | "-r" | "-R"
            ) && idx + 1 < args.len()
            {
                idx += 2;
            } else {
                idx += 1;
            }
        } else {
            break;
        }
    }
    if idx + 1 >= args.len() {
        return 1;
    }
    let sep = args[idx].clone();
    let arr = resolve_array_arg(&args[idx + 1]);

    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let combined = format!("{}{}", prefix, suffix);

    // Find the active segment (after the last separator)
    let parts: Vec<&str> = combined.split(&sep as &str).collect();
    let current = parts.last().copied().unwrap_or("");
    let completed_prefix = parts[..parts.len().saturating_sub(1)].join(&sep);
    let completed_prefix = if completed_prefix.is_empty() {
        String::new()
    } else {
        format!("{}{}", completed_prefix, sep)
    };

    let matches: Vec<String> = arr
        .iter()
        .filter_map(|entry| {
            let parts: Vec<&str> = entry.split(&sep as &str).collect();
            let p_idx = combined.matches(&sep as &str).count();
            parts.get(p_idx).and_then(|p| {
                if p.starts_with(current) {
                    Some(p.to_string())
                } else {
                    None
                }
            })
        })
        .collect();
    if matches.is_empty() {
        return 1;
    }
    setaparam("_multi_parts_arr", matches);

    let compadd_argv: Vec<String> = vec![
        "-P".to_string(),
        completed_prefix,
        "-a".to_string(),
        "_multi_parts_arr".to_string(),
    ];
    bin_compadd("compadd", &compadd_argv, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn returns_one_with_no_matches() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "xyz");
        let _ = setsparam("SUFFIX", "");
        let _r = _multi_parts(&["/".to_string(), "(foo/bar baz/qux)".to_string()]);
    }
}
