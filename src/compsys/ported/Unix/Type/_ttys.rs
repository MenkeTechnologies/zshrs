//! Port of `_ttys` from `Completion/Unix/Type/_ttys`.
//!
//! Full upstream body (25 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # -d strip /dev/;  -D allow with/without /dev/;  -o only attached ttys
//! sh: 9  local -a ttys expl pre
//! sh: 10  local stripdev optdev open
//! sh:12  zparseopts -D -K -E d=stripdev D=optdev o=open
//! sh:14  if [[ -n $open ]]; then
//! sh:15    ttys=( ${(u)${${(f)"$(_call_program open-ttys ps -Ao tty=)"}:#\?*}%% *} )
//! sh:16    _description open-ttys expl 'open tty'
//! sh:17  else
//! sh:18    ttys=( /dev/tty?*(N) /dev/pts/^ptmx(N) )
//! sh:19    ttys=( ${ttys#/dev/} )
//! sh:20    _description ttys expl 'tty'
//! sh:21  fi
//! sh:22  [[ -z $stripdev ]] && pre=( -p /dev/ )
//! sh:24  [[ -n $optdev ]] && compadd "$@" "$expl[@]" -M 'r:|/=* r:|=*' -a ttys && return
//! sh:25  compadd "$@" "$expl[@]" "$pre[@]" -M 'r:|/=* r:|=*' -a ttys
//! ```

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_description::_description;
use crate::ported::glob::{tokenize, zglob};
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

/// Nullglob a pattern, dropping entries that never matched.
fn glob_n(pat: &str) -> Vec<String> {
    let mut list = {
        let mut s = pat.to_string();
        tokenize(&mut s);
        vec![s]
    };
    zglob(&mut list, 0, 0);
    list.into_iter()
        .filter(|e| {
            !e.as_bytes()
                .iter()
                .any(|&b| matches!(b, b'*' | b'?' | b'[' | b']' | b'^'))
        })
        .collect()
}

/// sh:24-25 — the compadd calls `_ttys` makes, in the order it makes them.
///
/// Upstream is two statements, not one:
///
/// ```text
/// sh:24  [[ -n $optdev ]] && compadd "$@" "$expl[@]" -M '…' -a ttys && return
/// sh:25  compadd "$@" "$expl[@]" "$pre[@]" -M '…' -a ttys
/// ```
///
/// sh:24 is a CONJUNCTION: its `return` fires only when the compadd
/// SUCCEEDED. When that call adds nothing the `&&` chain stops and control
/// reaches sh:25, which offers the same names again behind `-p /dev/`
/// (sh:22). That fallthrough is the entire meaning of `-D` — "matches
/// allowed with or without the /dev/ prefix" (sh:6) — because the bare names
/// in `ttys` (`ttys000`, `pts/0`; sh:19 strips `/dev/`) can never match a
/// word the user typed as an absolute path. So `-D` yields TWO planned
/// calls; every other flag combination yields one.
///
/// Measured under a pty, `zsh -f -i` vs a pinned `zshrs --zsh -f -i`, both
/// with `fpath=( /usr/share/zsh/5.9/functions )` + `compinit -u`,
/// `setopt menu_complete`:
///
/// ```text
///   _ttys -D, word `/dev/tt`  zsh  -> /dev/tty.Bluetooth-Incoming-Port
///                             was  -> /dev/tt          (nothing added)
///   _ttys -D, word `tt`       both -> tty.Bluetooth-Incoming-Port
///   _ttys    , word `/dev/tt` both -> /dev/tty.Bluetooth-Incoming-Port
/// ```
fn compadd_plan(
    rest: &[String],
    expl: &[String],
    stripdev: bool,
    optdev: bool,
) -> Vec<Vec<String>> {
    // The tail every call shares: sh:24/25 `-M 'r:|/=* r:|=*' -a ttys`.
    let build = |pre: bool| -> Vec<String> {
        let mut cadd: Vec<String> = rest.to_vec();
        cadd.extend(expl.iter().cloned());
        if pre {
            // sh:22  [[ -z $stripdev ]] && pre=( -p /dev/ )
            cadd.push("-p".to_string());
            cadd.push("/dev/".to_string());
        }
        cadd.push("-M".to_string());
        cadd.push("r:|/=* r:|=*".to_string());
        cadd.push("-a".to_string());
        cadd.push("_ttys_list".to_string());
        cadd
    };
    let mut plan = Vec::with_capacity(2);
    // sh:24 — the `-D` attempt carries no `-p`, so it matches bare names.
    if optdev {
        plan.push(build(false));
    }
    // sh:25 — the unconditional call, with `-p /dev/` unless `-d` was given.
    plan.push(build(!stripdev));
    plan
}

/// `_ttys` — complete terminal device names.
pub fn _ttys(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_ttys");
    // sh:12  zparseopts -D -K -E d=stripdev D=optdev o=open
    let stripdev = args.iter().any(|a| a == "-d");
    let optdev = args.iter().any(|a| a == "-D");
    let open = args.iter().any(|a| a == "-o");
    let rest: Vec<String> = args
        .iter()
        .filter(|a| !matches!(a.as_str(), "-d" | "-D" | "-o"))
        .cloned()
        .collect();

    // sh:14-21
    let ttys: Vec<String> = if open {
        // sh:15 — ps -Ao tty=, drop `?*` (no-tty) lines, strip trailing
        //   fields, unique.
        let _ = call_program_capture(&[
            "open-ttys".to_string(),
            "ps".to_string(),
            "-Ao".to_string(),
            "tty=".to_string(),
        ]);
        let out = getsparam("REPLY").unwrap_or_default();
        let mut seen: Vec<String> = Vec::new();
        for line in out.lines() {
            if line.starts_with('?') {
                continue;
            }
            let name = line.split(' ').next().unwrap_or("").to_string();
            if !name.is_empty() && !seen.contains(&name) {
                seen.push(name);
            }
        }
        let _ = _description(&[
            "open-ttys".to_string(),
            "expl".to_string(),
            "open tty".to_string(),
        ]);
        seen
    } else {
        // sh:18-19 — /dev/tty?*(N) /dev/pts/^ptmx(N), strip /dev/.
        let mut t = glob_n("/dev/tty?*");
        t.extend(
            glob_n("/dev/pts/*")
                .into_iter()
                .filter(|p| p.rsplit('/').next().map(|b| b != "ptmx").unwrap_or(true)),
        );
        let _ = _description(&["ttys".to_string(), "expl".to_string(), "tty".to_string()]);
        t.into_iter()
            .map(|p| p.strip_prefix("/dev/").unwrap_or(&p).to_string())
            .collect()
    };

    setaparam("_ttys_list", ttys);
    let expl = getaparam("expl").unwrap_or_default();

    // sh:24-25 — run each planned compadd until one adds a match.
    let plan = compadd_plan(&rest, &expl, stripdev, optdev);
    let mut ret = 1;
    for cadd in &plan {
        ret = bin_compadd("compadd", cadd, &make_ops(), 0);
        // sh:24's `&& return`: the conjunction only returns when the compadd
        //   SUCCEEDED, so a failed `-D` attempt falls through to sh:25.
        if ret == 0 {
            break;
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(_ttys(&[]), 1);
    }

    /// sh:24-25 — `-D` is TWO compadds, and only the second carries
    /// `-p /dev/`. Collapsing them into one (either by dropping the
    /// fallthrough or by giving the `-D` attempt the prefix) is the
    /// regression this pins: with only the prefix-less call, a word already
    /// typed as `/dev/tt` matches nothing.
    #[test]
    fn optdev_plans_a_bare_call_then_the_prefixed_fallthrough() {
        let expl = vec!["-J".to_string(), "-default-".to_string()];
        let plan = compadd_plan(&[], &expl, false, true);
        assert_eq!(
            plan.len(),
            2,
            "-D must plan sh:24 AND the sh:25 fallthrough"
        );
        assert!(
            !plan[0].iter().any(|w| w == "-p"),
            "sh:24 carries no $pre[@]: {:?}",
            plan[0]
        );
        assert_eq!(
            plan[1]
                .iter()
                .position(|w| w == "-p")
                .map(|i| plan[1][i + 1].clone()),
            Some("/dev/".to_string()),
            "sh:25 must offer the same names behind /dev/: {:?}",
            plan[1]
        );
        // Both calls share the sh:24/25 tail verbatim.
        for cadd in &plan {
            assert!(cadd.ends_with(&[
                "-M".to_string(),
                "r:|/=* r:|=*".to_string(),
                "-a".to_string(),
                "_ttys_list".to_string(),
            ]));
            assert_eq!(&cadd[0..2], &expl[..], "$expl[@] precedes the options");
        }
    }

    /// sh:22/24/25 — without `-D` there is exactly one call, and `-d`
    /// suppresses the `/dev/` prefix on it.
    #[test]
    fn without_optdev_one_call_and_stripdev_drops_the_prefix() {
        let plain = compadd_plan(&[], &[], false, false);
        assert_eq!(plain.len(), 1);
        assert!(plain[0].iter().any(|w| w == "-p"));

        let stripped = compadd_plan(&[], &[], true, false);
        assert_eq!(stripped.len(), 1);
        assert!(
            !stripped[0].iter().any(|w| w == "-p"),
            "-d must drop `-p /dev/`: {:?}",
            stripped[0]
        );
    }

    /// sh:24/25 — the caller's own compadd options (`"$@"` after zparseopts
    /// removed `-d`/`-D`/`-o`) are forwarded to BOTH calls, so a `-S ''` a
    /// caller passed is not lost on the fallthrough.
    #[test]
    fn caller_options_reach_both_planned_calls() {
        let rest = vec!["-S".to_string(), String::new()];
        let plan = compadd_plan(&rest, &[], false, true);
        assert_eq!(plan.len(), 2);
        for cadd in &plan {
            assert_eq!(&cadd[0..2], &rest[..]);
        }
    }
}
