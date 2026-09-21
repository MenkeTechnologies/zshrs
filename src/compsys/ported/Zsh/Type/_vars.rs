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

use crate::compsys::ported::shared::dispatch_action_command;
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

        // sh:14 — `_subscript -q` is a plain COMMAND WORD, so it resolves the
        // way `execcmd` resolves one (`Src/exec.c:3105-3109` shfunc, then
        // builtin, then `$PATH`, then c:903 `command not found` + 127).
        // `dispatch_action_command` (shared.rs:1407) IS that resolution and it
        // publishes the caller line first: `FnScope` zeroes `lineno` for every
        // port body (shared.rs, mirroring `Src/exec.c:1429`), and the callee's
        // frame records the caller's line at push time (`doshfunc`,
        // `Src/exec.c:6013`), so without it `$functrace`/`$funcfiletrace` read
        // `_vars:0`.
        return dispatch_action_command("_subscript", &["-q".to_string()], 14);
    }

    // sh:16
    let mut p_args: Vec<String> = vec!["-g".to_string(), "^a*".to_string()];
    p_args.extend(args.iter().cloned());
    // sh:16 — the line the first `_parameters` call sits on; the frame it
    // pushes records it as the caller line (`Src/exec.c:6013`), and a name
    // that resolves NOWHERE must reach c:903's diagnostic rather than return
    // a silent 1. Measured on this host, where `$fpath`'s first `_parameters`
    // (`~/.zpwr/autoload/comp_utils/_parameters`) is UNTAGGED so `compinit`
    // registers nothing for the name (compinit sh:507-526) and the Rust port
    // steps aside for the file (router.rs `has_fpath_override`), on `unset
    // <TAB>`:
    //     zsh    `_vars:16: command not found: _parameters`
    //            `_vars:23: command not found: _parameters`
    //     zshrs  (nothing at all)
    if dispatch_action_command("_parameters", &p_args, 16) == 0 {
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
    // sh:23 — the line the second `_parameters` call sits on. Re-published
    // because the sh:18-22 block ran in between; the callee's frame records
    // this as its caller line.
    if dispatch_action_command("_parameters", &p2_args2, 23) == 0 {
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
