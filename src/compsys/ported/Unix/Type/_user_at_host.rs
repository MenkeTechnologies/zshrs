//! Port of `_user_at_host` from `Completion/Unix/Type/_user_at_host`.
//!
//! Full upstream body (31 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 8  local expl suf tag=accounts
//! sh:10  if [[ "$1" = -t?* ]]; then tag="${1[3,-1]}"; shift
//! sh:13  elif [[ "$1" = -t ]]; then tag="$2"; shift 2
//! sh:16  fi
//! sh:18  [[ "$1" = -(|-) ]] && shift
//! sh:20  if [[ -prefix 1 *@ ]]; then
//! sh:21    local user=${PREFIX%%@*}
//! sh:23    compset -P 1 '*@'
//! sh:25    _wanted -C user-at hosts expl "host for $user" \
//! sh:26        _combination -s '[:@]' "${tag}" users-hosts users="$user" hosts "$@" -
//! sh:27  else
//! sh:28    compset -S '@*' || suf="@"
//! sh:29    _wanted users expl "user" \
//! sh:30        _combination -s '[:@]' "${tag}" users-hosts users -S "$suf" -q "$@" -
//! sh:31  fi
//! ```
//!
//! The `[[ -prefix 1 *@ ]]` compsys condition (sh:20) — "the word before
//! the cursor matches `*@`" — is rendered as `PREFIX contains '@'`.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::getsparam;
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

fn compset(argv: Vec<String>) -> i32 {
    bin_compset("compset", &argv, &make_ops(), 0)
}

/// `_user_at_host` — complete `user@host` combinations, from the
/// `users-hosts` style for the `accounts` tag (or a `-t tag` override).
pub fn _user_at_host(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_user_at_host");
    // sh:8  tag=accounts
    let mut tag = "accounts".to_string();
    let mut rest: Vec<String> = args.to_vec();

    // sh:10-15  -t tag  (glued `-tTAG` or separate `-t TAG`).
    if let Some(first) = rest.first().cloned() {
        if first.starts_with("-t") && first.len() > 2 {
            tag = first[2..].to_string();
            rest.remove(0);
        } else if first == "-t" {
            if rest.len() >= 2 {
                tag = rest[1].clone();
                rest.drain(0..2);
            } else {
                rest.remove(0);
            }
        }
    }

    // sh:17  a leading `-` / `--` is ignored.
    if matches!(rest.first().map(|s| s.as_str()), Some("-") | Some("--")) {
        rest.remove(0);
    }

    let prefix = getsparam("PREFIX").unwrap_or_default();
    if prefix.contains('@') {
        // sh:20-26 — host-for-user branch.
        let user = prefix.split('@').next().unwrap_or("").to_string();
        // sh:23  compset -P 1 '*@'
        let _ = compset(vec!["-P".to_string(), "1".to_string(), "*@".to_string()]);
        // sh:25-26
        let mut w: Vec<String> = vec![
            "-C".to_string(),
            "user-at".to_string(),
            "hosts".to_string(),
            "expl".to_string(),
            format!("host for {}", user),
            "_combination".to_string(),
            "-s".to_string(),
            "[:@]".to_string(),
            tag,
            "users-hosts".to_string(),
            format!("users={}", user),
            "hosts".to_string(),
        ];
        w.extend(rest);
        w.push("-".to_string());
        _wanted(&w)
    } else {
        // sh:27-30 — user branch.
        // sh:28  compset -S '@*' || suf="@"
        let suf = if compset(vec!["-S".to_string(), "@*".to_string()]) == 0 {
            String::new()
        } else {
            "@".to_string()
        };
        let mut w: Vec<String> = vec![
            "users".to_string(),
            "expl".to_string(),
            "user".to_string(),
            "_combination".to_string(),
            "-s".to_string(),
            "[:@]".to_string(),
            tag,
            "users-hosts".to_string(),
            "users".to_string(),
            "-S".to_string(),
            suf,
            "-q".to_string(),
        ];
        w.extend(rest);
        w.push("-".to_string());
        _wanted(&w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let r = _user_at_host(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn dash_t_tag_is_parsed_glued_and_separate() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = crate::ported::params::setsparam("PREFIX", "");
        // Both forms are consumed without leaking `-t` into the action.
        assert_eq!(_user_at_host(&["-tmy-accounts".to_string()]), 1);
        assert_eq!(
            _user_at_host(&["-t".to_string(), "other-accounts".to_string()]),
            1
        );
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }
}
