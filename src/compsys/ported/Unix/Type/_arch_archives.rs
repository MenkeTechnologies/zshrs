//! Port of `_arch_archives` from `Completion/Unix/Type/_arch_archives`.
//!
//! Full upstream body (14 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local ARCHCMD="$1"
//! sh: 4  shift
//! sh: 5  local expl completions library name_arg='-n'
//! sh: 6  if [[ -n $argv[(r)--library] ]]; then
//! sh: 7    library='library-'
//! sh: 9    argv[(r)--library]=()
//! sh:10    name_arg=
//! sh:11  fi
//! sh:12  completions=($(_call_program ${ARCHCMD} ${ARCHCMD} ${library:-}archives $name_arg))
//! sh:13  _description -V archives expl "${library:-}archives"
//! sh:14  compadd "$@" "$expl[@]" -- "$completions[@]"
//! ```
//!
//! `_call_program` publishes the helper's stdout in `$REPLY`; the `$(…)`
//! capture is reproduced by reading `REPLY` and whitespace-splitting.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, getsparam};
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

/// Reach `_arch_archives` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_arch_archives "$ARCHCMD" -S / ${library:+--library}` (Completion/Unix/Type/_arch_namespace sh:37) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_arch_archives_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _arch_archives(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_arch_archives", args, || {
        _arch_archives_impl(args)
    })
}

/// `_arch_archives` — complete arch/tla archive names via `$ARCHCMD archives`.
pub fn _arch_archives_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_arch_archives");
    // sh:3-4  ARCHCMD="$1"; shift
    let Some(archcmd) = args.first().cloned() else {
        return 1;
    };
    let mut argv: Vec<String> = args[1..].to_vec();

    // sh:6-10  --library toggles a `library-` command prefix and drops the flag.
    let mut library = String::new();
    let mut name_arg = "-n".to_string();
    if let Some(pos) = argv.iter().position(|a| a == "--library") {
        library = "library-".to_string();
        argv.remove(pos);
        name_arg.clear();
    }

    // sh:12  completions=($(_call_program ARCHCMD ARCHCMD ${library}archives $name_arg))
    let mut cp: Vec<String> = vec![archcmd.clone(), archcmd, format!("{}archives", library)];
    if !name_arg.is_empty() {
        cp.push(name_arg);
    }
    let _ = _call_program(&cp);
    let completions: Vec<String> = getsparam("REPLY")
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();

    // sh:12  _description -V archives expl "${library}archives"
    let _ = _description(&[
        "-V".to_string(),
        "archives".to_string(),
        "expl".to_string(),
        format!("{}archives", library),
    ]);

    // sh:14  compadd "$@" "$expl[@]" -- "$completions[@]"
    let mut cadd: Vec<String> = argv;
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("--".to_string());
    cadd.extend(completions);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_archcmd_returns_one() {
        // sh:3 — with no $1 there is nothing to query.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_arch_archives_impl(&[]), 1);
    }

    #[test]
    fn library_flag_is_consumed() {
        // sh:6-10 — `--library` must not survive into compadd's argv, and
        // without a completion context compadd refuses (returns 1).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            _arch_archives_impl(&["true".to_string(), "--library".to_string()]),
            1
        );
    }
}
