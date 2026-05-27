//! Port of `_sep_parts` from `Completion/Base/Utility/_sep_parts`.
//!
//! Full upstream body (146 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:10  zparseopts -D -a opts 'J+:=group' 'V+:=group' P: F: S: r: R: q 1 2 o+: n \
//! sh:11      'x+:=expl' 'X+:=expl' 'M+:=matcher'
//! sh:19  opre="$PREFIX"; osuf="$SUFFIX"
//! sh:28  while [[ $# -gt 1 ]]; do
//! sh:30    arr="$1"; sep="$2"
//! sh:33    [[ $arr[1] == "(" ]] && tmparr=( ${=arr[2,-2]} ); arr=tmparr
//! sh:38    [[ "$str" != *${sep}* ]] && break
//! sh:42    PREFIX="${str%%(|\\)${sep}*}"
//! sh:43    compadd -O testarr "$matcher[@]" -a "$arr"
//! sh:50    (( $#testarr )) || return 1
//! sh:51    [[ $#testarr -gt 1 ]] && break
//! sh:67    str="${str#*${sep}}"; prefix+="${PREFIX}${sep}"
//! sh:71  done
//! sh:96  suffixes=("")
//! sh:140  compadd "$opts[@]" "$expl[@]" -P "$prefix" -a "$arr"
//! ```
//!
//! Splits a string by alternating-separator arrays + emits matches
//! for the active part. Heavy at full faithfulness; this port
//! covers the common ipv4/email-like form: walk pairs, locate the
//! active segment, emit from the matching array.

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

/// Parse `(elem1 elem2 …)` literal-array syntax, else lookup
/// shell-side array by name.
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

/// `_sep_parts` — complete each part of a separator-delimited
/// string from the paired (array, sep) arguments.
pub fn _sep_parts(args: &[String]) -> i32 {
    // sh:10 — opts pass-through (skip strict zparseopts here; opts
    //   forwards are passed to bin_compadd as-is at the end)
    let mut idx = 0usize;
    let mut compadd_opts: Vec<String> = Vec::new();
    while idx < args.len() {
        let a = &args[idx];
        if a.starts_with('-') && a.len() > 1 {
            // Treat any leading dash arg as a compadd option pass-
            //   through with optional value (the upstream zparseopts
            //   handles tagging, but we forward verbatim).
            compadd_opts.push(a.clone());
            if matches!(
                a.as_str(),
                "-J" | "-V" | "-P" | "-F" | "-S" | "-r" | "-R" | "-x" | "-X" | "-M" | "-o"
            ) && idx + 1 < args.len()
            {
                compadd_opts.push(args[idx + 1].clone());
                idx += 2;
                continue;
            }
            idx += 1;
        } else {
            break;
        }
    }

    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let mut str = format!("{}{}", prefix, suffix);
    let mut completed_prefix = String::new();

    // sh:28-71  walk (arr, sep) pairs
    let mut last_arr: Vec<String> = Vec::new();
    while idx + 1 < args.len() {
        let arr_spec = &args[idx];
        let sep = &args[idx + 1];
        idx += 2;
        let arr = resolve_array_arg(arr_spec);
        last_arr = arr.clone();
        if !str.contains(sep as &str) {
            break;
        }
        // sh:42-43 — count testarr matches against current segment
        let head = str.splitn(2, sep as &str).next().unwrap_or("").to_string();
        let matches: Vec<&String> = arr.iter().filter(|a| a.starts_with(&head)).collect();
        if matches.is_empty() {
            return 1;
        }
        if matches.len() > 1 {
            break;
        }
        // single match → advance
        completed_prefix.push_str(&head);
        completed_prefix.push_str(sep);
        str = str.splitn(2, sep as &str).nth(1).unwrap_or("").to_string();
    }
    // sh:96  trailing array (when last arg unpaired)
    if idx < args.len() {
        last_arr = resolve_array_arg(&args[idx]);
    }

    // sh:140  compadd from final array with composed prefix
    setaparam("_sep_parts_arr", last_arr);
    let mut compadd_argv = compadd_opts;
    compadd_argv.push("-P".to_string());
    compadd_argv.push(completed_prefix);
    compadd_argv.push("-a".to_string());
    compadd_argv.push("_sep_parts_arr".to_string());
    bin_compadd("compadd", &compadd_argv, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn returns_compadd_status() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _r = _sep_parts(&[
            "(foo bar)".to_string(),
            "@".to_string(),
            "(host1 host2)".to_string(),
        ]);
    }
}
