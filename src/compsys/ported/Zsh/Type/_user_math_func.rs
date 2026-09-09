//! Port of `_user_math_func` from `Completion/Zsh/Type/_user_math_func`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  local expl
//! sh:4  local -a funcs
//! sh:5
//! sh:6  funcs=(${${${(f)"$(functions -M)"}##functions -M }%% *})
//! sh:7
//! sh:8  _wanted user-math-functions expl 'user math function' \
//! sh:9      compadd -S '(' -q "$@" -a funcs
//! ```
//!
//! sh:6 parses the TEXT `functions -M` prints, and the field it takes is
//! not the math function's name — it is the first word of whatever is left
//! after `##functions -M ` fails or succeeds. A previous revision read the
//! live `MATHFUNCS` registry instead, filtering on `MFF_USERFUNC` (the
//! predicate the builtin's own listing arm uses, builtin.rs:8459), on the
//! reasoning that the two agree. They agree for one of the two kinds of
//! entry the registry holds and disagree for the other, because
//! `listusermathfunc` writes the `s` of `functions -Ms` INSIDE the prefix
//! sh:6 strips (c:Src/builtin.c:3252):
//!
//! ```c
//! printf("functions -M%s %s", (p->flags & MFF_STR) ? "s" : "", p->name);
//! ```
//!
//! so an `MFF_STR` entry prints `functions -Ms NAME 1`, which does not
//! start with `functions -M ` — `##` strips nothing, `%% *` cuts at the
//! first space, and the candidate zsh offers for it is the literal word
//! `functions`. Measured against zsh 5.9.2 with `functions -Ms zzqstr` and
//! `functions -M zzqnum 1 1 zzq` defined:
//!
//! ```text
//! echo $((zzqs<TAB>   zsh: unchanged        zshrs: echo $((zzqstr(
//! echo $((zzq<TAB>    zsh: echo $((zzqnum(  zshrs: lists zzqnum zzqstr
//! ```
//!
//! The second line is the one that bites: an `-Ms` function the user
//! cannot reach by name in zsh made every OTHER user math function
//! ambiguous here.
//!
//! So run the builtin and parse what it prints, the way sh:6 does. The
//! `$( … )` fork is still skipped ([`capture_builtin_stdout`]); the
//! FILTERING and the FORMATTING are the builtin's, which is the whole
//! point.

use crate::compsys::ported::_wanted::_wanted;
use crate::compsys::ported::shared::capture_builtin_stdout;
use crate::ported::builtin::bin_functions;
use crate::ported::params::setaparam;
use crate::ported::zsh_h::{options, MAX_OPS};

/// sh:6's `${${${(f)"$(functions -M)"}##functions -M }%% *}` applied to the
/// builtin's captured output.
///
/// `##functions -M ` is a LITERAL prefix strip — no pattern characters — and
/// `%% *` removes the longest suffix starting at a space, i.e. keeps the
/// first word. Both are per-element under `(f)`. The array literal at sh:6
/// is unquoted, so an element that ends up empty is dropped.
fn parse_functions_m(text: &str) -> Vec<String> {
    text.lines()
        .map(|l| l.strip_prefix("functions -M ").unwrap_or(l)) // ##functions -M
        .map(|l| l.split(' ').next().unwrap_or("")) // %% *
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// `_user_math_func` — complete user-defined math function names
/// (those added via `functions -M`).
pub fn _user_math_func(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_user_math_func");
    // sh:6 — `functions -M` with no arguments takes c:3478-3484's listing
    //   arm: every `MFF_USERFUNC` entry through `listusermathfunc`. `-M` is
    //   `OPT_MINUS`, i.e. `ind[c] & 1` (c:Src/zsh.h:1402); `functions`'
    //   builtin-table funcid is 0 (c:Src/builtin.c:77).
    let text = capture_builtin_stdout(false, || {
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'M' as usize] = 1;
        let _ = bin_functions("functions", &[], &ops, 0);
    });
    let funcs = parse_functions_m(&text);

    // Publish under the shell-side name `funcs` so `compadd -a funcs`
    // picks them up.
    setaparam("funcs", funcs);

    // sh:8-9
    let mut wanted_argv: Vec<String> = vec![
        "user-math-functions".to_string(),
        "expl".to_string(),
        "user math function".to_string(),
        "compadd".to_string(),
        "-S".to_string(),
        "(".to_string(),
        "-q".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-a".to_string());
    wanted_argv.push("funcs".to_string());
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_nonzero_with_no_user_funcs() {
        // sh:9 — no user math funcs registered AND no tag → _wanted
        //   returns 1.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _user_math_func(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn publishes_funcs_array() {
        // sh:6 — verify the `funcs` array gets published (even when
        //   empty, the publish itself happens).
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = _user_math_func(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        let funcs = crate::ported::params::getaparam("funcs");
        assert!(funcs.is_some(), "funcs array should be published");
    }

    /// sh:6 on the exact bytes `listusermathfunc` writes (c:3252-3277).
    ///
    /// The `-Ms` line is the regression: `functions -Ms zzqstr 1` does NOT
    /// start with `functions -M ` (the `s` is inside the prefix), so `##`
    /// strips nothing and the candidate is the word `functions`. Reading
    /// `MATHFUNCS` directly offered `zzqstr` instead — a name zsh never
    /// offers, and one that made `zzq<TAB>` ambiguous where zsh completed
    /// it outright.
    #[test]
    fn parses_the_field_sh6_takes_not_the_math_function_name() {
        // Verbatim `functions -M` output from zsh 5.9.2 after
        // `functions -Ms zzqstr; functions -M zzqnum 1 1 zzq`.
        let text = "functions -Ms zzqstr 1\nfunctions -M zzqnum 1 1 zzq\n";
        assert_eq!(
            parse_functions_m(text),
            vec!["functions".to_string(), "zzqnum".to_string()]
        );
    }

    /// A `functions -M NAME` with no min/max prints the bare name
    /// (c:3247-3256 leaves `showargs` at 0), and no output at all means no
    /// candidates — `${(f)""}` yields one empty element and sh:6's unquoted
    /// array literal drops it.
    #[test]
    fn bare_name_and_empty_output() {
        assert_eq!(
            parse_functions_m("functions -M plain\n"),
            vec!["plain".to_string()]
        );
        assert!(parse_functions_m("").is_empty());
    }
}
