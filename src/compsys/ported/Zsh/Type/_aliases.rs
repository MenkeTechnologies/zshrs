//! Port of `_aliases` from `Completion/Zsh/Type/_aliases`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #compdef unalias
//! sh: 2
//! sh: 3  local expl sel args opts
//! sh: 4
//! sh: 5  zparseopts -E -D s:=sel
//! sh: 6
//! sh: 7  [[ -z $sel ]] && sel=rgs
//! sh: 8
//! sh: 9  opts=( "$@" )
//! sh:10
//! sh:11  args=()
//! sh:12  [[ $sel = *r* ]] && args=( $args 'aliases:regular alias:compadd -k aliases' )
//! sh:13  [[ $sel = *g* ]] && args=( $args 'global-aliases:global alias:compadd -k galiases' )
//! sh:14  [[ $sel = *s* ]] && args=( $args 'suffix-aliases:suffix alias:compadd -k saliases' )
//! sh:15  [[ $sel = *R* ]] && args=( $args 'disabled-aliases:disabled regular alias:compadd -k dis_aliases' )
//! sh:16  [[ $sel = *G* ]] && args=( $args 'disabled-global-aliases:disabled global alias:compadd -k dis_galiases' )
//! sh:17  [[ $sel = *S* ]] && args=( $args 'disabled-suffix-aliases:disabled suffix alias:compadd -k dis_saliases' )
//! sh:18
//! sh:19  _alternative -O opts $args
//! ```
//!
//! `zparseopts -E -D s:=sel` — `s` takes a value, stored under
//! `sel`. After parse, `sel` contains the value paired with `-s`
//! (e.g. `("-s", "rgs")`); we read `sel[1]` (the value after the
//! flag).

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, setaparam};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:5 — bridge to real `bin_zparseopts -E -D s:=sel` via
/// `-v <name>` (-E = extract, allows mixing with non-options).
fn run_zparseopts_aliases(args: &[String]) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    crate::compsys::ported::shared::set_bridge_argv(src, args);
    setaparam("sel", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-E".to_string(),
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "s:=sel".to_string(),
        ],
        &make_ops(),
        0,
    );
    let sel = getaparam("sel").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down `__compsys_argv` — the zparseopts-bridge scratch array, not a
    // real zsh identifier (zsh operates on positional $argv). It is declared
    // FUNCTION-LOCAL by `shared::set_bridge_argv`; this unset is what clears it
    // when the port runs outside any function scope. Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, sel)
}

/// `_aliases` — `unalias` command completion: list alias-name
/// categories per `-s` flag (`r`=regular, `g`=global, `s`=suffix,
/// upper variants for disabled).
pub fn _aliases(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_aliases");
    // sh:3 — `local expl sel args opts`.
    //
    // `sel` and `opts` are written as shell parameters below (sh:5's
    // `zparseopts … s:=sel` and sh:9's `opts=( "$@" )`), and `expl` is
    // the array `_alternative`'s `_description` fills through its `$2`.
    // `args` stays Rust-side as `alt_args`. Measured on `unalias <TAB>`:
    //
    //   zsh  : opts=[][0]        zshrs: opts=[array][0]
    //
    // `__compsys_argv` — this port's own scratch array, with no sh
    // counterpart — is deliberately NOT in this list: it carries its own
    // declaration and teardown where it is used (`run_zparseopts_aliases`
    // above, via `shared::set_bridge_argv`, bug #657), so it never reaches
    // the caller.
    crate::compsys::ported::shared::declare_locals(&["expl", "sel", "opts"], 0);
    // sh:5-7
    let (opts, sel_arr) = run_zparseopts_aliases(args);
    // `sel_arr` is `[-s, <value>]` when -s given; the value is at index 1.
    let sel: String = sel_arr.get(1).cloned().unwrap_or_else(|| "rgs".to_string());

    // sh:9
    setaparam("opts", opts);

    // sh:11-17
    let mut alt_args: Vec<String> = Vec::new();
    for (chr, spec) in [
        ('r', "aliases:regular alias:compadd -k aliases"),
        ('g', "global-aliases:global alias:compadd -k galiases"),
        ('s', "suffix-aliases:suffix alias:compadd -k saliases"),
        (
            'R',
            "disabled-aliases:disabled regular alias:compadd -k dis_aliases",
        ),
        (
            'G',
            "disabled-global-aliases:disabled global alias:compadd -k dis_galiases",
        ),
        (
            'S',
            "disabled-suffix-aliases:disabled suffix alias:compadd -k dis_saliases",
        ),
    ] {
        if sel.contains(chr) {
            alt_args.push(spec.to_string());
        }
    }

    // sh:19
    let mut full: Vec<String> = vec!["-O".to_string(), "opts".to_string()];
    full.extend(alt_args);
    dispatch_function_call("_alternative", &full).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_aliases(&[]), 1);
    }

    #[test]
    fn default_sel_is_rgs() {
        // sh:7 — when -s not given, sel defaults to "rgs". The opts
        //   array publication shows the parser stripped nothing.
        let _g = crate::test_util::global_state_lock();
        let _ = _aliases(&["hello".to_string()]);
        let opts = getaparam("opts").unwrap_or_default();
        assert_eq!(opts, vec!["hello"]);
    }
}
