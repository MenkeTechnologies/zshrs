//! Port of `_default` from `Completion/Zsh/Context/_default`.
//!
//! Full upstream body (27 lines verbatim):
//! ```text
//! sh: 1  #compdef -default-
//! sh: 3  local ctl
//! sh: 5  if { zstyle -s ":completion:${curcontext}:" use-compctl ctl ||
//! sh: 6       zmodload -e zsh/compctl } && [[ "$ctl" != (no|false|0|off) ]]; then
//! sh: 7    local opt
//! sh: 9    opt=()
//! sh:10    [[ "$ctl" = *first* ]] && opt=(-T)
//! sh:11    [[ "$ctl" = *default* ]] && opt=("$opt[@]" -D)
//! sh:12    compcall "$opt[@]" || return 0
//! sh:13  fi
//! sh:15  _files "$@" && return 0
//! sh:17  # magicequalsubst allows arguments like <any-old-stuff>=~/foo
//! sh:21  if [[ -o magicequalsubst && "$PREFIX" = *\=* ]]; then
//! sh:22    compstate[parameter]="${PREFIX%%\=*}"
//! sh:23    compset -P 1 '*='
//! sh:24    _value "$@"
//! sh:25  else
//! sh:26    return 1
//! sh:27  fi
//! ```
//!
//! `compcall` is a compctl-bridge builtin; if unavailable / inactive
//! the shell skips it. `_files` and `_value` are siblings dispatched
//! via `exec accessors`.

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::getsparam;
use crate::ported::zle::compcore::set_compstate_str;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{isset, options, MAGICEQUALSUBST, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_default` — `-default-` context: try compctl bridge, then
/// `_files`, then `_value` (when MAGICEQUALSUBST + `=` in PREFIX).
pub fn _default(args: &[String]) -> i32 {
    // sh:5-13  use-compctl branch
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctl = lookupstyle(&format!(":completion:{}:", curcontext), "use-compctl")
        .first()
        .cloned()
        .unwrap_or_default();
    if !ctl.is_empty() && !matches!(ctl.as_str(), "no" | "false" | "0" | "off") {
        let mut opt: Vec<String> = Vec::new();
        if ctl.contains("first") {
            opt.push("-T".to_string());
        }
        if ctl.contains("default") {
            opt.push("-D".to_string());
        }
        // sh:12 — compcall via dispatch (no Rust bin_compcall)
        if dispatch_function_call("compcall", &opt).unwrap_or(1) == 0 {
            return 0;
        }
    }

    // sh:15
    if dispatch_function_call("_files", args).unwrap_or(1) == 0 {
        return 0;
    }

    // sh:21
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if isset(MAGICEQUALSUBST) && prefix.contains('=') {
        let param = prefix.splitn(2, '=').next().unwrap_or("").to_string();
        set_compstate_str("parameter", &param);
        let _ = bin_compset(
            "compset",
            &["-P".to_string(), "1".to_string(), "*=".to_string()],
            &make_ops(),
            0,
        );
        dispatch_function_call("_value", args).unwrap_or(1)
    } else {
        // sh:26
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_default(&[]), 1);
    }
}
