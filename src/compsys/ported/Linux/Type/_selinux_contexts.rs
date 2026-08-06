//! Port of `_selinux_contexts` from `Completion/Linux/Type/_selinux_contexts`.
//!
//! Full upstream body (21 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local -a parts users roles types
//! sh: 5  zparseopts -E -D a:=types P:=users
//! sh: 7  if ! compset -S ':*'; then
//! sh: 8    users+=( -qS : )
//! sh: 9    roles+=( -qS : )
//! sh:10    [[ $(</sys/fs/selinux/mls) = 0 ]] 2>/dev/null || types+=( -qS : )
//! sh:11  fi
//! sh:13  parts=( users roles types )
//! sh:14  while compset -P 1 '*:' && (( $+parts[1] )) ; do
//! sh:15    shift parts
//! sh:16  done
//! sh:17  if (( $+parts[1] )); then
//! sh:18    _selinux_$parts[1] ${(P)parts[1]}
//! sh:19  else
//! sh:20    _message -e selinux-ranges 'selinux range'
//! sh:21  fi
//! ```
//!
//! `parts` only ever holds the fixed literal names `users`/`roles`/
//! `types` (sh:13), so the indirect-call idiom `_selinux_$parts[1]
//! ${(P)parts[1]}` (sh:18) is ported as a direct dispatch on an index
//! into a fixed 3-slot table rather than generic name indirection —
//! the same style `_fuse_arguments` uses for its own fixed sub-calls.

use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_selinux_roles::_selinux_roles;
use crate::compsys::ported::_selinux_types::_selinux_types;
use crate::compsys::ported::_selinux_users::_selinux_users;
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

/// sh:5 — `zparseopts -E -D a:=types P:=users`: pull `-a VALUE` pairs
/// into `types`, `-P VALUE` pairs into `users`.
/// `-E` tolerates/leaves unrecognized options alone (upstream never
/// references `$@` again, so unmatched args are simply dropped here
/// too); `-D` removes matched options from the source array, which is
/// moot since `args` isn't consulted again after this call.
fn zparse_a_p(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut types = Vec::new();
    let mut users = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-a" if i + 1 < args.len() => {
                types.push("-a".to_string());
                types.push(args[i + 1].clone());
                i += 2;
            }
            "-P" if i + 1 < args.len() => {
                users.push("-P".to_string());
                users.push(args[i + 1].clone());
                i += 2;
            }
            _ => i += 1,
        }
    }
    (types, users)
}

/// sh:10 — `[[ $(</sys/fs/selinux/mls) = 0 ]] 2>/dev/null || types+=(
/// -qS : )`: append the `-qS :` range-colon guard to `types` UNLESS
/// the kernel reports MLS disabled (`mls` file contents exactly
/// `"0"`). Split out as a pure predicate over the (possibly missing)
/// file contents so the decision is unit-testable without touching
/// the filesystem.
fn types_needs_qs(mls_contents: Option<&str>) -> bool {
    mls_contents.map(|s| s.trim()) != Some("0")
}

/// Read the real `/sys/fs/selinux/mls` file, mirroring the shell's
/// `$(</sys/fs/selinux/mls) 2>/dev/null` (missing file / read error →
/// `None`, exactly like the suppressed-stderr command substitution
/// yielding an empty result).
fn read_mls_file() -> Option<String> {
    std::fs::read_to_string("/sys/fs/selinux/mls").ok()
}

/// The fixed sh:13 `parts=( users roles types )` table: each entry is
/// the sub-completer name paired with its accumulated arg-array.
const PART_NAMES: [&str; 3] = ["users", "roles", "types"];

/// `_selinux_contexts` — complete a `user:role:type:range` SELinux
/// security context component-by-component, delegating each `:`
/// separated segment to `_selinux_users`/`_selinux_roles`/
/// `_selinux_types` in turn; once all three are consumed, offer the
/// `selinux range` message instead (sh:17-21).
pub fn _selinux_contexts(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_selinux_contexts");
    // sh:5
    let (mut types, mut users) = zparse_a_p(args);
    let mut roles: Vec<String> = Vec::new();

    // sh:7-11
    if bin_compset(
        "compset",
        &["-S".to_string(), ":*".to_string()],
        &make_ops(),
        0,
    ) != 0
    {
        // sh:8-9
        users.push("-qS".to_string());
        users.push(":".to_string());
        roles.push("-qS".to_string());
        roles.push(":".to_string());
        // sh:10
        if types_needs_qs(read_mls_file().as_deref()) {
            types.push("-qS".to_string());
            types.push(":".to_string());
        }
    }

    // sh:13-16 — `parts` starts at index 0 (`users`); each successful
    // `compset -P 1 '*:'` that still leaves a slot shifts the index
    // forward by one (`shift parts`).
    let mut idx = 0usize;
    loop {
        let stripped = bin_compset(
            "compset",
            &["-P".to_string(), "1".to_string(), "*:".to_string()],
            &make_ops(),
            0,
        ) == 0;
        if !stripped {
            break;
        }
        if idx >= PART_NAMES.len() {
            // sh:14 — `(( $+parts[1] ))` false: parts already empty.
            break;
        }
        idx += 1; // sh:15 — shift parts
    }

    // sh:17-21
    if idx < PART_NAMES.len() {
        // sh:18
        match PART_NAMES[idx] {
            "users" => _selinux_users(&users),
            "roles" => _selinux_roles(&roles),
            "types" => _selinux_types(&types),
            _ => unreachable!(),
        }
    } else {
        // sh:20
        _message(&[
            "-e".to_string(),
            "selinux-ranges".to_string(),
            "selinux range".to_string(),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_a_p_pulls_a_and_p_leaving_rest_unused() {
        let (types, users) = zparse_a_p(&["-a".to_string(), "file_type".to_string()]);
        assert_eq!(types, vec!["-a".to_string(), "file_type".to_string()]);
        assert!(users.is_empty());

        let (types2, users2) = zparse_a_p(&["-P".to_string(), "someuser".to_string()]);
        assert!(types2.is_empty());
        assert_eq!(users2, vec!["-P".to_string(), "someuser".to_string()]);
    }

    #[test]
    fn zparse_a_p_ignores_unknown_flags() {
        let (types, users) = zparse_a_p(&["-x".to_string(), "domain".to_string()]);
        assert!(types.is_empty());
        assert!(users.is_empty());
    }

    #[test]
    fn types_needs_qs_false_only_when_mls_is_exactly_zero() {
        // sh:10 — `[[ $(</sys/fs/selinux/mls) = 0 ]]` → range guard
        // skipped only when the file's contents are exactly "0".
        assert!(!types_needs_qs(Some("0")));
        assert!(!types_needs_qs(Some("0\n"))); // trailing newline trimmed
        assert!(types_needs_qs(Some("1")));
        assert!(types_needs_qs(None)); // missing file / read error
    }

    #[test]
    fn returns_one_without_completion_context() {
        // sh:7 `compset -S` fails outside completion context (INCOMPFUNC
        // != 1) so `!compset` is true → both -qS guards get appended;
        // sh:14's `compset -P` likewise always fails, so `parts` never
        // shifts and dispatch falls to `_selinux_users`, whose own
        // `bin_compadd` call fails outside completion context too.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_selinux_contexts(&[]), 1);
    }

    #[test]
    fn dispatches_to_users_first_by_default() {
        // sh:13 — parts[1] starts at "users" when no `:` segment has
        // been consumed yet (compset always fails outside completion
        // context, so the while loop never shifts).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        // Both _selinux_users (idx 0) and the eventual fallthrough
        // return 1 outside completion context; assert the dispatch
        // path itself picks index 0 by checking PART_NAMES directly.
        assert_eq!(PART_NAMES[0], "users");
        assert_eq!(
            _selinux_contexts(&["-a".to_string(), "file_type".to_string()]),
            1
        );
    }
}
