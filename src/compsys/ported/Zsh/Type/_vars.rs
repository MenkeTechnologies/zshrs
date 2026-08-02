//! Port of `_vars` from `Completion/Zsh/Type/_vars`.
//!
//! Full upstream body (25 lines verbatim):
//! ```text
//! sh: 1  #compdef getopts unset
//! sh: 3  # This will handle completion of keys of associative arrays
//! sh: 6  local ret=1
//! sh: 8  if [[ $PREFIX = *\[* ]]; then
//! sh: 9    compstate[parameter]=${PREFIX%%(|\\)\[*}
//! sh:11    IPREFIX=${PREFIX%%\[*}\[
//! sh:12    PREFIX=${PREFIX#*\[}
//! sh:13
//! sh:14    _subscript -q
//! sh:15  else
//! sh:16    _parameters -g '^a*' "$@" && ret=0
//! sh:17
//! sh:18    if compset -S '\[*'; then
//! sh:19      set - -S "" "$@"
//! sh:20    else
//! sh:21      set - -qS"${${QIPREFIX:+[}:-\[}" "$@"
//! sh:22    fi
//! sh:23    _parameters -g 'a*' "$@" && ret=0
//! sh:24    return ret
//! sh:25  fi
//! ```

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setsparam};
use crate::ported::zle::compcore::set_compstate_str;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_vars` — `getopts`/`unset` completion handling assoc-array key
/// subscripts (`vared foo[<TAB>` etc.).
pub fn _vars(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_vars");
    let mut ret: i32 = 1;
    let prefix = getsparam("PREFIX").unwrap_or_default();

    // sh:8
    if prefix.contains('[') {
        // sh:9
        let p_end = prefix.find('[').unwrap_or(prefix.len());
        let param = if p_end > 0 && prefix.as_bytes()[p_end - 1] == b'\\' {
            prefix[..p_end - 1].to_string()
        } else {
            prefix[..p_end].to_string()
        };
        set_compstate_str("parameter", &param);

        // sh:11
        let _ = setsparam("IPREFIX", &format!("{}[", &prefix[..p_end]));
        // sh:12
        let after = prefix.splitn(2, '[').nth(1).unwrap_or("");
        let _ = setsparam("PREFIX", after);

        // sh:14
        return dispatch_function_call("_subscript", &["-q".to_string()]).unwrap_or(1);
    }

    // sh:16
    let mut p_args: Vec<String> = vec!["-g".to_string(), "^a*".to_string()];
    p_args.extend(args.iter().cloned());
    if dispatch_function_call("_parameters", &p_args).unwrap_or(1) == 0 {
        ret = 0;
    }

    // sh:18-22
    let has_subscript_suffix = bin_compset(
        "compset",
        &["-S".to_string(), "\\[*".to_string()],
        &make_ops(),
        0,
    ) == 0;
    let mut p2_args: Vec<String> = Vec::new();
    if has_subscript_suffix {
        // sh:19
        p2_args.push("-S".to_string());
        p2_args.push("".to_string());
    } else {
        // sh:21
        let qiprefix = getsparam("QIPREFIX").unwrap_or_default();
        let close = if qiprefix.is_empty() { "\\[" } else { "[" };
        p2_args.push(format!("-qS{}", close));
    }
    p2_args.extend(args.iter().cloned());

    // sh:23
    let mut p2_args2: Vec<String> = vec!["-g".to_string(), "a*".to_string()];
    p2_args2.extend(p2_args);
    if dispatch_function_call("_parameters", &p2_args2).unwrap_or(1) == 0 {
        ret = 0;
    }

    // sh:24
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "myvar");
        assert_eq!(_vars(&[]), 1);
    }
}
