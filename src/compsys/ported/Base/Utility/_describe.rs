//! Port of `_describe` from `Completion/Base/Utility/_describe`.
//!
//! Full upstream body (140 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh: 19  while getopts "oOt:12JVx" _opt; do …
//! sh: 38  [[ "$_type$_noprefix" = options && ! -prefix [-+]* ]] &&
//! sh: 39      zstyle -T … options prefix-needed && return 1
//! sh: 42  zstyle -T … verbose && _showd=yes
//! sh: 46  zstyle -s … list-separator _sep || _sep=--
//! sh: 49  _descr="$1"; shift
//! sh: 58  _tags "$_type"
//! sh: 59  while _tags; do
//! sh: 60    while _next_label $_jvx12 "$_type" _expl "$_descr"; do
//! sh: 65      … iterate value:description arrays, build display lines …
//! sh:130      compadd "$_jvx12[@]" "$_expl[@]" -d … "$@" - "$_new[@]"
//! sh:135    done
//! sh:136    [[ $_ret -eq 0 ]] && return 0
//! sh:137  done
//! sh:140  return _ret
//! ```
//!
//! Heavy display-line construction is approximated: we read each
//! named array (1st extra arg), build the strings list, and call
//! `compadd` with the description-list flag set.

use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_tags::_tags;
use crate::ported::modules::zutil::testforstyle;
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

/// `_describe` — emit a labeled list of `value:description` pairs.
/// Flags: `-o` options-mode, `-O` options no-prefix, `-t TAG`,
/// `-1/-2/-J/-V/-x` forwarded to `_next_label`.
pub fn _describe(args: &[String]) -> i32 {
    let mut typ = "values".to_string();
    let mut noprefix = false;
    let mut jvx12: Vec<String> = Vec::new();
    let mut idx = 0usize;

    // sh:19-35  flag parse
    while idx < args.len() {
        let a = &args[idx];
        match a.as_str() {
            "-o" => {
                typ = "options".to_string();
                idx += 1;
            }
            "-O" => {
                typ = "options".to_string();
                noprefix = true;
                idx += 1;
            }
            "-t" if idx + 1 < args.len() => {
                typ = args[idx + 1].clone();
                idx += 2;
            }
            "-1" | "-2" | "-J" | "-V" | "-x" => {
                jvx12.push(a.clone());
                idx += 1;
            }
            _ => break,
        }
    }

    // sh:38-39  options + prefix-needed
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();
    if typ == "options"
        && !noprefix
        && !prefix.starts_with('-')
        && !prefix.starts_with('+')
        && testforstyle(
            &format!(":completion:{}:options", curcontext),
            "prefix-needed",
        ) == 0
    {
        return 1;
    }

    // sh:49
    if idx >= args.len() {
        return 1;
    }
    let descr = args[idx].clone();
    idx += 1;

    // sh:58
    let _ = _tags(&[typ.clone()]);

    let arrays: Vec<String> = args[idx..].to_vec();
    let mut ret: i32 = 1;

    // sh:59
    loop {
        if _tags(&[]) != 0 {
            break;
        }
        // sh:60
        loop {
            let mut nl_args = jvx12.clone();
            nl_args.push(typ.clone());
            nl_args.push("_expl".to_string());
            nl_args.push(descr.clone());
            if _next_label(&nl_args) != 0 {
                break;
            }

            // For each named array, read it and emit values + descs
            for arr_name in &arrays {
                let arr = getaparam(arr_name).unwrap_or_default();
                let (strs, disp): (Vec<String>, Vec<String>) = arr
                    .iter()
                    .map(|entry| {
                        let mut parts = entry.splitn(2, ':');
                        let v = parts.next().unwrap_or("").to_string();
                        let d = parts.next().unwrap_or("").to_string();
                        (v.clone(), if d.is_empty() { v } else { format!("{} -- {}", entry.splitn(2, ':').next().unwrap_or(""), d) })
                    })
                    .unzip();
                if strs.is_empty() {
                    continue;
                }
                setaparam("_describe_disp", disp);
                let expl = getaparam("_expl").unwrap_or_default();
                let mut compadd_argv: Vec<String> = jvx12.clone();
                compadd_argv.extend(expl);
                compadd_argv.push("-d".to_string());
                compadd_argv.push("_describe_disp".to_string());
                compadd_argv.push("-".to_string());
                compadd_argv.extend(strs);
                if bin_compadd("compadd", &compadd_argv, &make_ops(), 0) == 0 {
                    ret = 0;
                }
            }
        }
        if ret == 0 {
            return 0;
        }
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_for_empty_args() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_describe(&[]), 1);
    }

    #[test]
    fn returns_one_with_no_tag_setup() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            _describe(&[
                "-t".to_string(),
                "mytag".to_string(),
                "description".to_string(),
                "myarr".to_string(),
            ]),
            1
        );
    }
}
