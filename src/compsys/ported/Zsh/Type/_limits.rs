//! Port of `_limits` from `Completion/Zsh/Type/_limits`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #compdef unlimit
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _wanted limits expl 'process limit' compadd "$@" - ${${(f)"$(limit)"}%% *}
//! ```
//!
//! sh:5's `$(limit)` shell-out enumerates the configured rlimits. We
//! bypass the fork and the `${…%% *}` first-column parse by reading the
//! same authoritative table `bin_limit` consults — `known_resources`,
//! `src/ported/builtins/rlimits.rs:92`.
//!
//! **Why the raw table and not `limit`'s literal output.** They are not
//! the same list, and the difference is load-bearing in both directions:
//!
//!   * `limit` prints `resinfo[rt]->name` for every resource number
//!     `0 .. RLIM_NLIMITS` (c:372-374 → c:311). `set_resinfo()`
//!     (c:200-202) projects `known_resources` onto those numbers, so an
//!     entry whose `res` COLLIDES with another's vanishes from `limit`'s
//!     output while surviving in the raw table. Offering such a name
//!     completes something `limit NAME` then rejects — this is the
//!     `resident` bug (macOS aliases `RLIMIT_RSS` to `RLIMIT_AS`).
//!     Fixed at the source: the c:79 guard now keeps the table free of
//!     duplicate `res`, pinned by
//!     `rlimits::tests::known_resources_have_no_duplicate_resource_numbers`.
//!     With no collisions the two name sets agree, so reading the raw
//!     table is sound.
//!   * The converse: `set_resinfo()` also FILLS every resource number
//!     the table does not cover with a synthetic `UNKNOWN-<n>` name
//!     (c:206-214), and `limit` prints those too — so enumerating
//!     `limit`'s output verbatim can offer a placeholder where zsh
//!     offers a real name. That is what used to happen on Linux
//!     (`RLIM_NLIMITS` 16): C's Linux block — `RLIMIT_LOCKS` …
//!     `RLIMIT_RTTIME`, c:102-126 — was unported, so numbers 10-15 came
//!     back as `UNKNOWN-10` … `UNKNOWN-15` where zsh has
//!     `maxfilelocks` … `rt_time`. Those six entries are now in
//!     `known_resources` under `cfg(target_os = "linux")`, so the two
//!     lists agree there as well; the raw table stays the safer source
//!     because it can never contain a placeholder.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::builtins::rlimits::known_resources;
#[cfg(test)]
use crate::ported::builtins::rlimits::{set_resinfo, RESINFO};

/// `_limits` — `unlimit` command completion: list process-resource
/// limit names.
pub fn _limits(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_limits");
    // sh:3 — `local expl`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_limits` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:3 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:5 — first column of `limit` output = resource name.
    let names: Vec<String> = known_resources.iter().map(|r| r.name.to_string()).collect();

    let mut wanted_argv: Vec<String> = vec![
        "limits".to_string(),
        "expl".to_string(),
        "process limit".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    wanted_argv.extend(names);
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
    /// The candidate list is `known_resources`, a compiled-in table, so
    /// nothing here reads the machine. What used to make this flaky was
    /// purely leaked completion state — see `reset_completion_state`.
    #[test]
    fn returns_zero_because_wanted_registers_its_own_tag() {
        let _g = crate::test_util::global_state_lock();
        crate::test_util::reset_completion_state();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _limits(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 0);
    }

    /// sh:5 is `compadd … ${${(f)"$(limit)"}%% *}`, so every candidate
    /// must be a name the `limit` builtin actually prints. Reading the
    /// raw table is only equivalent to that while no two entries share a
    /// resource number — `showlimits()` prints `resinfo[rt]->name`
    /// (c:372-374 → c:311) and `set_resinfo()` (c:200-202) keys by
    /// `res`, so a collision hides a name from the builtin without
    /// hiding it from us.
    ///
    /// This asserts the direction that broke: nothing offered here may
    /// be absent from `limit`'s output. It does NOT assert the reverse.
    /// The reverse held only because of the c:102-126 gap — `limit`
    /// printed six `UNKNOWN-<n>` placeholders on Linux that this
    /// completer rightly withheld — and those entries are now ported, so
    /// on both platforms the sets are expected to coincide. Keeping the
    /// assertion one-directional still catches the failure that matters:
    /// completing a name `limit NAME` would reject.
    #[test]
    fn every_offered_name_is_one_limit_prints() {
        let _g = crate::test_util::global_state_lock();

        let offered: Vec<&str> = known_resources.iter().map(|r| r.name).collect();
        assert!(!offered.is_empty(), "known_resources is empty");

        // The first column of `limit` with no arguments: one name per
        // resource number, in the order showlimits() walks them.
        set_resinfo();
        let printed: Vec<String> = {
            let lock = RESINFO.get().unwrap();
            let v = lock.lock().unwrap();
            v.iter().map(|r| r.name.to_string()).collect()
        };

        for name in &offered {
            assert!(
                printed.iter().any(|p| p == name),
                "`{}` would be completed but `limit` never prints it \
                 (its resource number is taken by another entry). \
                 limit prints: {:?}",
                name,
                printed
            );
        }

        // The specific regression: where `RLIMIT_RSS` aliases
        // `RLIMIT_AS` (macOS) the builtin prints `addressspace` at that
        // resource number and rejects `limit resident`, so `resident`
        // must not be offered. Where RSS is distinct (Linux) it must be.
        assert_eq!(
            offered.contains(&"resident"),
            libc::RLIMIT_RSS as i32 != libc::RLIMIT_AS as i32,
            "offered names: {:?}",
            offered
        );
    }
}
