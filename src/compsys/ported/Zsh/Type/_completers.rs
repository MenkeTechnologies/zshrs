//! Port of `_completers` from `Completion/Zsh/Type/_completers`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # option: -p - needs a `_' prefix
//! sh: 4  local us
//! sh: 5  local -a disp list expl
//! sh: 6
//! sh: 7  list=( complete approximate correct match expand list menu oldlist
//! sh: 8         ignored prefix history )
//! sh: 9  zparseopts -D -K -E 'p=us'
//! sh:10  [[ -n "$us" ]] && us='_'
//! sh:11  zstyle -t ":completion:${curcontext}:completers" prefix-hidden &&
//! sh:12      disp=(-d list)
//! sh:13  _wanted completers expl 'completer' \
//! sh:14      compadd "$@" "$disp[@]" - "$us${^list[@]}"
//! ```
//!
//! `${^list[@]}` — rcexpand brace-expands across array elements with
//! the prefix `$us` (either empty or `_`), yielding e.g.
//! `_complete _approximate _correct …` when `-p` is given.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::{bin_zparseopts, testforstyle};
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:9 — bridge to real `bin_zparseopts -D -K -E p=us`. Boolean
/// `p` redirected into `us` array.
fn run_zparseopts_p(args: &[String]) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("us", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-K".to_string(),
            "-E".to_string(),
            "-v".to_string(),
            src.to_string(),
            "p=us".to_string(),
        ],
        &make_ops(),
        0,
    );
    let us = getaparam("us").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, us)
}

/// `_completers` — complete completer-fn names (the dispatcher
/// chain). `-p` flag prepends `_` to each name (autoload-style).
pub fn _completers(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_completers");
    // sh:4 — `local us`; sh:5 — `local -a disp list expl`.
    //
    // Both names are published as shell parameters: `us` is the
    // `zparseopts 'p=us'` target (sh:9) and `list` is the completer-name
    // table `_wanted` reads by name (sh:12-14). `disp`/`expl` are left out
    // — the port never writes them. Without the declaration
    // `zstyle ':completion:*' completer <TAB>` left both standing:
    //
    //   zsh  : list=[][0]         us=[][0]
    //   zshrs: list=[array][11]   us=[array][1]
    //
    // `us` is declared as a plain scalar, exactly as sh:4 does; zparseopts
    // converts it to an array the same way it does upstream.
    crate::compsys::ported::shared::declare_locals(&["us"], 0);
    crate::compsys::ported::shared::declare_locals(
        &["list"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    // sh:7-8
    let list: Vec<&str> = vec![
        "complete",
        "approximate",
        "correct",
        "match",
        "expand",
        "list",
        "menu",
        "oldlist",
        "ignored",
        "prefix",
        "history",
    ];

    // sh:9-10
    let (argv, us_arr) = run_zparseopts_p(args);
    let us: &str = if us_arr.is_empty() { "" } else { "_" };

    // sh:11-12
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let style_ctx = format!(":completion:{}:completers", curcontext);
    let prefix_hidden = testforstyle(&style_ctx, "prefix-hidden") == 0;

    // sh:14  expand `$us${^list[@]}` into prefixed names; publish
    //   the BARE list for `-d list` (display column) usage.
    let prefixed: Vec<String> = list.iter().map(|s| format!("{}{}", us, s)).collect();
    let bare_list: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    setaparam("list", bare_list);

    let disp: Vec<String> = if prefix_hidden {
        vec!["-d".to_string(), "list".to_string()]
    } else {
        Vec::new()
    };

    // sh:13-14
    let mut wanted_argv: Vec<String> = vec![
        "completers".to_string(),
        "expl".to_string(),
        "completer".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(argv.iter().cloned());
    wanted_argv.extend(disp);
    wanted_argv.push("-".to_string());
    wanted_argv.extend(prefixed);
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    /// `_wanted` registers its OWN tag, so the "without registered tags"
    /// premise never holds: with the `doshfunc`-frame shift (see
    /// `Base/Core/_wanted.rs:45-54`) `_wanted` registers and `_all_labels`
    /// adds matches, making 0 the correct return. Confirmed against real
    /// zsh 5.9.2 driven through a PTY inside a live completion widget —
    /// all of these return 0, not 1. The old name and `assert_eq!(r, 1)`
    /// encoded the pre-shift answer.
    ///
    /// `reset_completion_state` is what makes that answer STABLE. The
    /// assertion used to pass alone and fail inside a full run because a
    /// leftover `$PREFIX` from an earlier test filtered out every candidate
    /// `compadd` was offered, so `compadd` returned 1 for a tag set that had
    /// been registered perfectly well — see that helper for the mechanism.
    #[test]
    fn returns_zero_because_wanted_registers_its_own_tag() {
        let _g = crate::test_util::global_state_lock();
        crate::test_util::reset_completion_state();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _completers(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 0);
    }

    #[test]
    fn dash_p_flag_extracted_into_us() {
        // sh:9 — boolean `p` flag stored as `us` array (single
        //   "-p" sentinel when present, empty when absent).
        let _g = crate::test_util::global_state_lock();
        let (rem, us) = run_zparseopts_p(&["-p".to_string()]);
        assert_eq!(us, vec!["-p"]);
        assert!(rem.is_empty());

        let (rem, us) = run_zparseopts_p(&[]);
        assert!(us.is_empty());
        assert!(rem.is_empty());
    }

    #[test]
    fn list_published_under_shell_name() {
        // sh:14 — `list` array must be set so `-d list` display
        //   column resolution works.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = _completers(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        let list = crate::ported::params::getaparam("list").unwrap_or_default();
        assert_eq!(list.len(), 11);
        assert_eq!(list[0], "complete");
        assert_eq!(list[10], "history");
    }
}
