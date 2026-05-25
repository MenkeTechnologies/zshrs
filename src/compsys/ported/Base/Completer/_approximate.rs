//! Port of `_approximate` from
//! `Completion/Base/Completer/_approximate`.
//!
//! Full upstream body (121 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh: 14  [[ _matcher_num -gt 1 || "${#:-$PREFIX$SUFFIX}" -le 1 ]] && return 1
//! sh: 17  local _comp_correct …
//! sh: 21  if [[ "$1" = -a* ]]; then cfgacc="${1[3,-1]}"
//! sh: 24  elif [[ "$1" = -a ]]; then cfgacc="$2"
//! sh: 27  else zstyle -s … max-errors cfgacc || cfgacc='2 numeric'
//! sh: 32  if [[ "$cfgacc" = *numeric* && ${NUMERIC:-1} -ne 1 ]]; then …
//! sh: 41  comax="${NUMERIC:-1}"
//! sh: 44  comax="${cfgacc//[^0-9]}"
//! sh: 47  [[ "$comax" -lt 1 ]] && return 1
//! sh: 50  _tags corrections original
//! sh: 56  _shadow -s _approximate compadd
//! sh: 57  compadd() { … inject (#a${_comp_correct}) prefix into match … }
//! sh: 74  _comp_correct=1
//! sh: 76  [[ -z "$compstate[pattern_match]" ]] && compstate[pattern_match]='*'
//! sh: 79  while [[ _comp_correct -le comax ]]; do
//! sh: 84    if _complete; then …emit corrections list … break …
//! sh:117  (( _comp_correct++ ))
//! sh:118  done
//! sh:120  _unshadow; return ret
//! ```
//!
//! Approximate-match completer: shadows `compadd` to inject the
//! `(#a$n)`-error glob prefix into each match, walks `n` from 1
//! up to the max-errors style. The shadowing dance can't be
//! faithfully ported without zsh's func-table machinery; this port
//! does the outer loop + delegates the per-pass to `_complete`.

use crate::compsys::ported::_complete::_complete;
use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getiparam, getsparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
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

/// `_approximate` — spell-correction completer.
pub fn _approximate(args: &[String]) -> i32 {
    // sh:14
    if getiparam("_matcher_num") > 1 {
        return 1;
    }
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    if prefix.len() + suffix.len() <= 1 {
        return 1;
    }

    // sh:21-29  -a / max-errors style
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let cfgacc = if let Some(a) = args.first() {
        if let Some(rest) = a.strip_prefix("-a") {
            if !rest.is_empty() {
                rest.to_string()
            } else if args.len() > 1 {
                args[1].clone()
            } else {
                "2 numeric".to_string()
            }
        } else {
            lookupstyle(&format!(":completion:{}:", curcontext), "max-errors")
                .first()
                .cloned()
                .unwrap_or_else(|| "2 numeric".to_string())
        }
    } else {
        lookupstyle(&format!(":completion:{}:", curcontext), "max-errors")
            .first()
            .cloned()
            .unwrap_or_else(|| "2 numeric".to_string())
    };

    // sh:32-44
    let numeric = getiparam("NUMERIC");
    let comax: i64 = if cfgacc.contains("numeric") && numeric != 1 {
        if cfgacc.contains("not-numeric") {
            return 1;
        }
        if numeric < 1 {
            1
        } else {
            numeric
        }
    } else {
        cfgacc
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    };
    if comax < 1 {
        return 1;
    }

    // sh:50
    let _ = _tags(&["corrections".to_string(), "original".to_string()]);

    // sh:74-77
    let opm = get_compstate_str("pattern_match").unwrap_or_default();
    if opm.is_empty() {
        set_compstate_str("pattern_match", "*");
    }

    let mut ret: i32 = 1;
    let mut comp_correct: i64 = 1;
    let oldcontext = curcontext.clone();
    while comp_correct <= comax {
        let _ = setsparam("_comp_correct", &comp_correct.to_string());
        // sh:80  swap context to include the error-count suffix
        let new_ctx = format!("{}-{}", oldcontext, comp_correct);
        let _ = setsparam("curcontext", &new_ctx);

        let _ = _description(&[
            "corrections".to_string(),
            "_correct_expl".to_string(),
            "corrections".to_string(),
            format!("e:{}", comp_correct),
            format!("o:{}{}", prefix, suffix),
        ]);

        if _complete() == 0 {
            // sh:85  insert-unambiguous?
            let unambig = get_compstate_str("unambiguous").unwrap_or_default();
            let pre_suf = format!("{}{}", prefix, suffix);
            if testforstyle(&format!(":completion:{}:", new_ctx), "insert-unambiguous") == 0
                && unambig.len() >= pre_suf.len()
            {
                set_compstate_str("pattern_insert", "unambiguous");
            } else if _requested(&["original".to_string()]) == 0 {
                let nm: i64 = get_compstate_str("nmatches")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                let orig_on = lookupstyle(&format!(":completion:{}:", new_ctx), "original")
                    .first()
                    .map(|v| matches!(v.as_str(), "yes" | "true" | "1" | "on"))
                    .unwrap_or(false);
                if nm > 1 || orig_on {
                    let _ = _description(&[
                        "-V".to_string(),
                        "original".to_string(),
                        "expl".to_string(),
                        "original".to_string(),
                    ]);
                    let expl = getaparam("expl").unwrap_or_default();
                    let mut compadd_argv = expl;
                    compadd_argv.push("-U".to_string());
                    compadd_argv.push("-Q".to_string());
                    compadd_argv.push("-".to_string());
                    compadd_argv.push(pre_suf);
                    let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);

                    let list = get_compstate_str("list").unwrap_or_default();
                    if !list.starts_with("list") {
                        set_compstate_str("list", &format!("{} force", list).trim().to_string());
                    }
                }
            }
            set_compstate_str("pattern_match", &opm);
            ret = 0;
            break;
        }
        comp_correct += 1;
    }

    let _ = setsparam("curcontext", &oldcontext);
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_input_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "a");
        let _ = setsparam("SUFFIX", "");
        assert_eq!(_approximate(&[]), 1);
    }
}
