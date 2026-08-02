//! Port of `_tilde_files` from `Completion/Unix/Type/_tilde_files`.
//!
//! Full upstream body (39 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  if [[ ( -o magicequalsubst && "$IPREFIX" = *\= ) || $argv[(I)-W*] -ne 0 ]]; then
//! sh: 4    _files "$@"
//! sh: 5    return
//! sh: 6  fi
//! sh: 8  case "$PREFIX" in
//! sh: 9  \~/*)
//! sh:10    IPREFIX="${IPREFIX}${HOME}/"
//! sh:11    PREFIX="${PREFIX[3,-1]}"
//! sh:12    _files "$@" -W "${HOME}"
//! sh:13    ;;
//! sh:14  \~*/*)
//! sh:15    local user="${PREFIX[2,-1]%%/*}"
//! sh:17    if (( $+userdirs[$user] )); then
//! sh:18      user="$userdirs[$user]"
//! sh:19    elif (( $+nameddirs[$user] )); then
//! sh:20      user="$nameddirs[$user]"
//! sh:21    else
//! sh:22      _message "unknown user \`$user'"
//! sh:23      return 1
//! sh:24    fi
//! sh:25    IPREFIX="${IPREFIX}${user%/}/"
//! sh:26    PREFIX="${PREFIX#*/}"
//! sh:27    _files "$@" -W "$user"
//! sh:28    ;;
//! sh:29  \~*)
//! sh:30    compset -p 1
//! sh:31    local -a expl=( "$@" )
//! sh:32    _alternative -O expl users:user:_users named-directories:'named directory':'compadd -k nameddirs'
//! sh:33    ;;
//! sh:34  *)
//! sh:35    _files "$@"
//! sh:36    ;;
//! sh:37  esac
//! ```

use crate::compsys::ported::_message::_message;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
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

/// sh:3 — assoc lookup helper for the flat key/value layout used in
/// the Rust port.
fn assoc_get(name: &str, key: &str) -> Option<String> {
    let arr = getaparam(name)?;
    arr.chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

/// `_tilde_files` — file completion with `~user` / `~name`
/// expansion. Dispatches `_files` (sibling) for the underlying path
/// emission.
pub fn _tilde_files(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_tilde_files");
    // sh:3
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let has_w = args.iter().any(|a| a.starts_with("-W"));
    if (isset(MAGICEQUALSUBST) && iprefix.ends_with('=')) || has_w {
        // sh:4
        return dispatch_function_call("_files", args).unwrap_or(1);
    }

    // sh:8
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if prefix.starts_with("~/") {
        // sh:9-13
        let home = getsparam("HOME").unwrap_or_default();
        let _ = setsparam("IPREFIX", &format!("{}{}/", iprefix, home));
        let _ = setsparam("PREFIX", &prefix[2..]);
        let mut a: Vec<String> = args.to_vec();
        a.push("-W".to_string());
        a.push(home);
        return dispatch_function_call("_files", &a).unwrap_or(1);
    }

    if prefix.starts_with('~') && prefix[1..].contains('/') {
        // sh:14-28
        let user = prefix[1..].splitn(2, '/').next().unwrap_or("").to_string();
        let resolved = if let Some(v) = assoc_get("userdirs", &user) {
            v
        } else if let Some(v) = assoc_get("nameddirs", &user) {
            v
        } else {
            let _ = _message(&[format!("unknown user `{}'", user)]);
            return 1;
        };
        let user_trim = resolved.trim_end_matches('/').to_string();
        let _ = setsparam("IPREFIX", &format!("{}{}/", iprefix, user_trim));
        let after_slash = prefix.splitn(2, '/').nth(1).unwrap_or("").to_string();
        let _ = setsparam("PREFIX", &after_slash);
        let mut a: Vec<String> = args.to_vec();
        a.push("-W".to_string());
        a.push(resolved);
        return dispatch_function_call("_files", &a).unwrap_or(1);
    }

    if prefix.starts_with('~') {
        // sh:29-33
        let _ = bin_compset(
            "compset",
            &["-p".to_string(), "1".to_string()],
            &make_ops(),
            0,
        );
        setaparam("expl", args.to_vec());
        return dispatch_function_call(
            "_alternative",
            &[
                "-O".to_string(),
                "expl".to_string(),
                "users:user:_users".to_string(),
                "named-directories:named directory:compadd -k nameddirs".to_string(),
            ],
        )
        .unwrap_or(1);
    }

    // sh:34
    dispatch_function_call("_files", args).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "/tmp");
        let _ = setsparam("IPREFIX", "");
        assert_eq!(_tilde_files(&[]), 1);
    }

    #[test]
    fn unknown_user_returns_one_with_message() {
        // sh:21-23
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "~nonexistentuser/path");
        let _ = setsparam("IPREFIX", "");
        setaparam("userdirs", Vec::new());
        setaparam("nameddirs", Vec::new());
        assert_eq!(_tilde_files(&[]), 1);
    }
}
