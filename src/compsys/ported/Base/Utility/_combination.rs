//! Port of `_combination` from
//! `Completion/Base/Utility/_combination`.
//!
//! Full upstream body (102 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  # Usage: _combination [-s S] TAG STYLE Ki=Pi ... Kj EXPL...
//! sh:60  zstyle queries with multi-key matching
//! sh:90  if zstyle -a … "$style" tmp; then
//! sh:91    filter tmp by all (Ki=Pi) patterns
//! sh:96    compadd "$@" -a tmp || _$key "$@"
//! sh:98  else
//! sh:99    _$key "$@"
//! sh:100 fi
//! ```
//!
//! Multi-field zstyle-key composer. Approximation: read the style
//! value list, filter each line by the `Ki=Pi` patterns against
//! the `sep`-joined fields, and emit the matching entries' final
//! field via compadd. Falls back to `_$key` dispatch on miss.

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getsparam, setaparam};
use crate::ported::pattern::{patcompile, pattry};
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

/// `_combination` — multi-key zstyle-driven completer. See upstream
/// docstring for the spec language (e.g. `users-hosts-ports`).
pub fn _combination(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_combination");
    let mut idx = 0usize;

    // sh:30 — `-s SEP` flag
    let mut sep = ":".to_string();
    if let Some(a) = args.first() {
        if a == "-s" && args.len() >= 2 {
            sep = args[1].clone();
            idx = 2;
        } else if let Some(rest) = a.strip_prefix("-s") {
            sep = rest.to_string();
            idx = 1;
        }
    }

    if args.len() < idx + 2 {
        return 1;
    }
    let tag = args[idx].clone();
    let style = args[idx + 1].clone();
    idx += 2;

    // sh:38  keys = split style on '-'
    let keys: Vec<&str> = style.split('-').collect();
    let mut pats: Vec<String> = keys.iter().map(|_| "*".to_string()).collect();

    // sh:42-55  parse Ki[:Ni]=Pi pairs
    let mut key_arg = String::new();
    let mut key_num: usize = 1;
    while idx < args.len() {
        let a = &args[idx];
        if a.contains('=') {
            let eq = a.find('=').unwrap();
            let tmp = &a[..eq];
            let pat = &a[eq + 1..];
            let (key_part, num_str) = if let Some(c) = tmp.find(':') {
                (&tmp[..c], &tmp[c + 1..])
            } else {
                (tmp, "1")
            };
            let num: usize = num_str.parse().unwrap_or(1);
            // Find the nth occurrence of key_part in keys
            let mut found_count = 0usize;
            for (i, k) in keys.iter().enumerate() {
                if *k == key_part {
                    found_count += 1;
                    if found_count == num {
                        pats[i] = pat.to_string();
                        break;
                    }
                }
            }
            idx += 1;
        } else {
            // Key terminator
            let (k_part, n_str) = if let Some(c) = a.find(':') {
                (&a[..c], &a[c + 1..])
            } else {
                (a.as_str(), "1")
            };
            key_arg = k_part.to_string();
            key_num = n_str.parse().unwrap_or(1);
            idx += 1;
            break;
        }
    }
    let extras: &[String] = &args[idx..];

    // sh:90  read the style; filter to matching combinations
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let style_ctx = format!(":completion:{}:{}", curcontext, tag);
    let style_vals = lookupstyle(&style_ctx, &style);
    if style_vals.is_empty() {
        // sh:99  fallback dispatch
        return dispatch_function_call(&format!("_{}", key_arg), extras).unwrap_or(1);
    }

    // Build a single combined glob pattern: `pat1{sep}pat2{sep}...`
    let combined_pat = pats.join(&sep);
    let prog = patcompile(
        &{
            let mut __pat_tok = (&combined_pat).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        0,
        None,
    );

    let mut matches: Vec<String> = Vec::new();
    for entry in &style_vals {
        let head = entry.split(&sep).next().unwrap_or(entry);
        let _ = head;
        let matches_combined = match prog.as_ref() {
            Some(p) => pattry(p, entry),
            None => false,
        };
        if matches_combined {
            // Extract the field corresponding to the key_arg ordinal
            let fields: Vec<&str> = entry.split(&sep).collect();
            // Find key_arg's nth occurrence in `keys`, return that field
            let mut occ = 0usize;
            for (i, k) in keys.iter().enumerate() {
                if *k == key_arg {
                    occ += 1;
                    if occ == key_num {
                        if let Some(f) = fields.get(i) {
                            matches.push(f.to_string());
                        }
                        break;
                    }
                }
            }
        }
    }
    matches.sort();
    matches.dedup();

    if matches.is_empty() {
        return dispatch_function_call(&format!("_{}", key_arg), extras).unwrap_or(1);
    }

    setaparam("tmp", matches);
    let mut compadd_argv: Vec<String> = extras.to_vec();
    compadd_argv.push("-a".to_string());
    compadd_argv.push("tmp".to_string());
    if bin_compadd("compadd", &compadd_argv, &make_ops(), 0) == 0 {
        0
    } else {
        dispatch_function_call(&format!("_{}", key_arg), extras).unwrap_or(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_style() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            _combination(&[
                "mytag".to_string(),
                "users-hosts".to_string(),
                "users".to_string(),
            ]),
            1
        );
    }
}
