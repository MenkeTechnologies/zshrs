//! Port of `_wanted` from `Completion/Base/Core/_wanted`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local -a __targs __gopt
//! sh: 4
//! sh: 5  zparseopts -D -a __gopt 1 2 V J x C:=__targs
//! sh: 6
//! sh: 7  _tags "$__targs[@]" "$1"
//! sh: 8
//! sh: 9  while _tags; do
//! sh:10    _all_labels "$__gopt[@]" "$@" && return 0
//! sh:11  done
//! sh:12
//! sh:13  return 1
//! ```
//!
//! Calls real `bin_zparseopts` with extended spec `C:=__targs`
//! (value-taking `-C <subcontext>` flag stored in `__targs`).
//! Delegates to sibling ports `_tags::_tags_impl` and
//! `_all_labels::_all_labels_impl` for the iteration.
//!
//! Those two deliberately name the raw bodies rather than the dispatching
//! `_tags` / `_all_labels`, unlike the `_description` calls in `_next_label`
//! / `_all_labels` / `_requested`. Both callees drive
//! `comptags`, which is indexed by `locallevel` (`Src/Zle/computil.c:3782`
//! "Array of tag-set infos. Index is the locallevel", `:3873` `level =
//! locallevel - (args[0][2] ? 1 : 0)`). Calling the raw bodies skips
//! `doshfunc`'s `inc_locallevel` (`src/ported/exec.rs`), leaving BOTH the
//! `comptags -i` in `_tags` and the `comptags -A` in `_all_labels` at
//! `_wanted`'s own level instead of one below it — so `_wanted` REPLACED its
//! caller's tag set rather than nesting under it.
//!
//! The level shift is now applied explicitly (`inc_locallevel` /
//! `dec_locallevel` around the block in [`_wanted_impl`]), same shape as the
//! guard in `_message.rs`. The live completion-context evidence the earlier
//! note asked for was obtained: dropping zsh's own
//! `Completion/Base/Core/_wanted` into `$fpath` ahead of the port — which
//! restores the real `doshfunc` frames and changes nothing else — makes
//! `diff3 -<TAB>` byte-identical to the reference shell, while the port
//! without the shift offered `$PWD` file names in place of the option list.
//!
//! CONSEQUENCE, since RESOLVED (2026-08-28): eight `returns_one_without_registered_tags`
//! cases in `_bind_addresses`, `_completers`, `_domains`, `_file_systems`,
//! `_global_tags`, `_hosts`, `_limits` and `_x_visual` asserted the pre-shift
//! answer. They call the `_impl` bodies with no `doshfunc` frame at all, so
//! their "no tags registered" premise only held while `_wanted`'s own `_tags`
//! registration silently landed on (and was read back from) the caller's
//! level. With the shift, `_wanted` registers its tag and `_all_labels` adds
//! matches, so 0 is the correct return — confirmed against real zsh 5.9.2
//! driven through a PTY inside a live completion widget, which returns 0 for
//! all eight. They are now `returns_zero_because_wanted_registers_its_own_tag`
//! and assert 0.
//!
//! This module's own `returns_zero_once_wanted_registers_the_tag_itself` (was
//! `no_matching_tagset_returns_one`) is the NINTH case from the same list and
//! the same mechanism — it passes a tag plus a `compadd` action with a match,
//! so post-shift `_wanted` registers `mytag`, the match is added, and 0 is
//! returned. Its old comment ("the while loop short-circuits and we return 1")
//! described the pre-shift behaviour. Note the PTY confirmation above covered
//! the eight siblings; this one rests on the identical documented mechanism
//! rather than its own measurement.
//!
//! All nine were ORDER-DEPENDENT after the flip: each passed alone and failed
//! inside a full `--lib` run, in a different combination every time. The
//! reason was never the shift — it was that `compadd` filters every candidate
//! against `$PREFIX` (`compcore.rs:4334-4373`) and `comptags` is indexed by
//! `locallevel` (`Src/Zle/computil.c:3782`), and a dozen unrelated completion
//! tests leave both dirty. They now call `crate::test_util::reset_completion_state`
//! first, and the ones whose candidate list came from the host machine
//! (`_hosts`, `_domains`, `_bind_addresses`, `_global_tags`) pin that input
//! too.

use super::_all_labels::_all_labels_impl;
use super::_tags::_tags_impl;
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

/// sh:5 — bridge to real `bin_zparseopts -D -a __gopt 1 2 V J x
/// C:=__targs` via `-v <name>` (named array source). Returns
/// `(remaining_argv, __targs, __gopt)`.
fn run_zparseopts_wanted(args: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    crate::compsys::ported::shared::set_bridge_argv(src, args);
    setaparam("__gopt", Vec::new());
    setaparam("__targs", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "__gopt".to_string(),
            "1".to_string(),
            "2".to_string(),
            "V".to_string(),
            "J".to_string(),
            "x".to_string(),
            "C:=__targs".to_string(),
        ],
        &make_ops(),
        0,
    );
    let gopt = getaparam("__gopt").unwrap_or_default();
    let targs = getaparam("__targs").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down `__compsys_argv` — the zparseopts-bridge scratch array, not a
    // real zsh identifier (zsh operates on positional $argv). It is declared
    // FUNCTION-LOCAL by `shared::set_bridge_argv`; this unset is what clears it
    // when the port runs outside any function scope. Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, targs, gopt)
}

/// Reach `_wanted` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_wanted fonts expl font \` (Completion/X/Type/_x_font sh:15)
/// — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_wanted_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _wanted(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_wanted", args, || _wanted_impl(args))
}

/// `_wanted` — register tag `$1` and (if requested) loop through
/// `_tags` calling `_all_labels` per round. Returns 0 if any
/// iteration succeeded, 1 if none.
pub fn _wanted_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_wanted");
    // sh:3  local -a __targs __gopt
    crate::compsys::ported::shared::declare_locals(
        &["__targs", "__gopt"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    // sh:3-5
    let (argv, targs, gopt) = run_zparseopts_wanted(args);

    // `comptags` is indexed by `locallevel` (`Src/Zle/computil.c:3782`
    // "Array of tag-set infos. Index is the locallevel"), and in zsh `_tags`
    // / `_all_labels` are real shell functions, so their `comptags -i` /
    // `comptags -A` register ONE level below `_wanted` and are discarded when
    // they return. Calling the raw `_impl` bodies skips `doshfunc`'s
    // `inc_locallevel` (`src/ported/exec.rs:6171`), so `_tags` here REPLACED
    // the caller's tag set instead of nesting under it. Concretely: an
    // `_arguments` spec whose positional action is a bare
    // ` _wanted files expl "…" _files` call (`_diff3` sh:44) wiped the
    // pending `options` tag set, so `diff3 -<TAB>` offered file names from
    // `$PWD` (matched via the `m:{a-z\-}={A-Z\_}` matcher, `-` → `_`)
    // instead of the option list. Same guard, and same reasoning, as
    // `_message.rs:187` and `_requested.rs`.
    //
    // Live evidence that this is what zsh does: dropping zsh's own
    // `Completion/Base/Core/_wanted` into `$fpath` ahead of the port — which
    // restores the real `doshfunc` frames and nothing else — makes
    // `diff3 -<TAB>` byte-identical to the reference shell.
    crate::ported::utils::inc_locallevel();

    // sh:7  _tags "$__targs[@]" "$1"
    let arg1 = argv.first().cloned().unwrap_or_default();
    let mut tags_args: Vec<String> = targs.clone();
    tags_args.push(arg1);
    let _ = _tags_impl(&tags_args);

    // sh:9-11  while _tags; do _all_labels … && return 0; done
    let mut ret = 1; // sh:13
    loop {
        if _tags_impl(&[]) != 0 {
            // sh:9 — _tags's switch-to-next-tag-set form returns
            //   non-zero when no more tag sets exist.
            break;
        }
        let mut labels_args: Vec<String> = gopt.clone();
        labels_args.extend(argv.iter().cloned());
        if _all_labels_impl(&labels_args) == 0 {
            // sh:10
            ret = 0;
            break;
        }
    }

    crate::ported::utils::dec_locallevel();
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    fn with_incompfunc<T, F: FnOnce() -> T>(f: F) -> T {
        let _g = crate::test_util::global_state_lock();
        // `_wanted_impl` drives `comptags` (indexed by `locallevel`,
        // `Src/Zle/computil.c:3782`) and `compadd` (which filters every
        // candidate against `$PREFIX`, `compcore.rs:4334-4373`), both of
        // which the previous test in this binary leaves behind. Without
        // this the return below depended on test ORDER, not on the body.
        crate::test_util::reset_completion_state();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    /// Post-shift, `_wanted` registers `mytag` itself and the `compadd
    /// alpha` action adds a match, so this returns 0. The old name and the
    /// old comment below described the pre-shift answer; see the module doc.
    #[test]
    fn returns_zero_once_wanted_registers_the_tag_itself() {
        // sh:13 — PRE-SHIFT: with no comptags state pre-loaded the
        //   switch-to-next never matched, the while loop short-circuited
        //   and this returned 1.
        let r = with_incompfunc(|| {
            _wanted_impl(&[
                "mytag".to_string(),
                "name".to_string(),
                "descr".to_string(),
                "compadd".to_string(),
                "alpha".to_string(),
            ])
        });
        assert_eq!(r, 0);
    }

    #[test]
    fn c_flag_takes_value_into_targs() {
        // sh:5 — `C:=__targs` means -C is value-taking; the value
        //   accumulates into __targs alongside the literal `-C`.
        let _g = crate::test_util::global_state_lock();
        let (rem, targs, gopt) = run_zparseopts_wanted(&[
            "-C".to_string(),
            "subctx".to_string(),
            "tagname".to_string(),
        ]);
        assert_eq!(targs, vec!["-C", "subctx"]);
        assert_eq!(gopt, Vec::<String>::new());
        assert_eq!(rem, vec!["tagname"]);
    }

    #[test]
    fn boolean_flags_into_gopt() {
        // sh:5 — `1 2 V J x` boolean, stored into __gopt; rest stays
        //   in argv.
        let _g = crate::test_util::global_state_lock();
        let (rem, targs, gopt) =
            run_zparseopts_wanted(&["-V".to_string(), "-1".to_string(), "mytag".to_string()]);
        assert_eq!(targs, Vec::<String>::new());
        assert_eq!(gopt, vec!["-V", "-1"]);
        assert_eq!(rem, vec!["mytag"]);
    }
}
