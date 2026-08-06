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

/// Reach `_default` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_default "${suf[@]}" && ret=0` (Completion/bashcompinit sh:36) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_default_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _default(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_default", args, || _default_impl(args))
}

/// `_default` — `-default-` context: try compctl bridge, then
/// `_files`, then `_value` (when MAGICEQUALSUBST + `=` in PREFIX).
pub fn _default_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_default");
    // sh:5-13  use-compctl branch
    //
    // sh:5-6 is `{ zstyle -s … use-compctl ctl || zmodload -e
    // zsh/compctl } && [[ "$ctl" != (no|false|0|off) ]]`. Two pieces
    // were missing here:
    //   * the `|| zmodload -e zsh/compctl` disjunct — with the style
    //     unset (the normal case) upstream still enters the branch
    //     whenever the legacy compctl module is loaded, with `ctl`
    //     left at its `local ctl` empty value;
    //   * the empty-`ctl` test: upstream compares `$ctl` against
    //     `(no|false|0|off)`, and an unset `ctl` is none of those, so
    //     an empty value ENTERS the branch. The port's extra
    //     `!ctl.is_empty()` inverted that.
    // sh:12 is `compcall "$opt[@]" || return 0` — `_default` gives up
    // (rc 0) when compcall FAILS and otherwise falls through to
    // `_files`. The port had the test the other way round.
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let styleval = lookupstyle(&format!(":completion:{}:", curcontext), "use-compctl");
    // `zstyle -s` succeeds iff the style is set, joining its values
    // into `ctl`; on failure `ctl` keeps the empty `local ctl` value.
    let styled = !styleval.is_empty();
    let ctl = if styled {
        styleval.join(" ")
    } else {
        String::new()
    };
    // `zmodload -e NAME` — c:Src/module.c:2637 bin_zmodload_exist:
    // module found, its handle slot filled (static-link analog:
    // MOD_INIT_B, i.e. boot_ ran) and not mid-unload.
    let compctl_loaded = crate::ported::module::MODULESTAB
        .lock()
        .map(|t| {
            t.modules
                .get("zsh/compctl")
                .map(|m| {
                    (m.node.flags & crate::ported::zsh_h::MOD_INIT_B) != 0
                        && (m.node.flags & crate::ported::zsh_h::MOD_UNLOAD) == 0
                })
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if (styled || compctl_loaded) && !matches!(ctl.as_str(), "no" | "false" | "0" | "off") {
        let mut opt: Vec<String> = Vec::new();
        if ctl.contains("first") {
            opt.push("-T".to_string());
        }
        if ctl.contains("default") {
            opt.push("-D".to_string());
        }
        // sh:12 — `compcall "$opt[@]" || return 0`.
        // `compcall` is the zsh/compctl BUILTIN (c:Src/Zle/compctl.c:1676),
        // not a shell function, so `dispatch_function_call` — which only
        // resolves shfuncs / Rust completer ports — never reached it and
        // the branch was a silent no-op even when it was entered.
        if crate::ported::zle::compctl::bin_compcall("compcall", &opt, &make_ops(), 0) != 0 {
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
        assert_eq!(_default_impl(&[]), 1);
    }
}
