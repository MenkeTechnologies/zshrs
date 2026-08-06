//! Port of `_tilde_files` from `Completion/Unix/Type/_tilde_files`.
//!
//! Full upstream body (39 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 5  if [[ ( -o magicequalsubst && "$IPREFIX" = *\= ) || $argv[(I)-W*] -ne 0 ]]; then
//! sh: 6    _files "$@"
//! sh: 7    return
//! sh: 8  fi
//! sh:10  case "$PREFIX" in
//! sh:11  \~/*)
//! sh:12    IPREFIX="${IPREFIX}${HOME}/"
//! sh:13    PREFIX="${PREFIX[3,-1]}"
//! sh:14    _files "$@" -W "${HOME}"
//! sh:15    ;;
//! sh:16  \~*/*)
//! sh:17    local user="${PREFIX[2,-1]%%/*}"
//! sh:19    if (( $+userdirs[$user] )); then
//! sh:20      user="$userdirs[$user]"
//! sh:21    elif (( $+nameddirs[$user] )); then
//! sh:22      user="$nameddirs[$user]"
//! sh:23    else
//! sh:24      _message "unknown user \`$user'"
//! sh:25      return 1
//! sh:26    fi
//! sh:27    IPREFIX="${IPREFIX}${user%/}/"
//! sh:28    PREFIX="${PREFIX#*/}"
//! sh:29    _files "$@" -W "$user"
//! sh:30    ;;
//! sh:31  \~*)
//! sh:32    compset -p 1
//! sh:33    local -a expl=( "$@" )
//! sh:34    _alternative -O expl users:user:_users named-directories:'named directory':'compadd -k nameddirs'
//! sh:35    ;;
//! sh:36  *)
//! sh:37    _files "$@"
//! sh:38    ;;
//! sh:39  esac
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

/// sh:5 — assoc lookup helper for the flat key/value layout used in
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
    // sh:5
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let has_w = args.iter().any(|a| a.starts_with("-W"));
    if (isset(MAGICEQUALSUBST) && iprefix.ends_with('=')) || has_w {
        // sh:4
        return dispatch_function_call("_files", args).unwrap_or(1);
    }

    // sh:10
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if prefix.starts_with("~/") {
        // sh:11-15
        let home = getsparam("HOME").unwrap_or_default();
        let _ = setsparam("IPREFIX", &format!("{}{}/", iprefix, home));
        let _ = setsparam("PREFIX", &prefix[2..]);
        let mut a: Vec<String> = args.to_vec();
        a.push("-W".to_string());
        a.push(home);
        return dispatch_function_call("_files", &a).unwrap_or(1);
    }

    if prefix.starts_with('~') && prefix[1..].contains('/') {
        // sh:16-30
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
        // sh:31-35
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

    // sh:36
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
        // sh:23-25
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "~nonexistentuser/path");
        let _ = setsparam("IPREFIX", "");
        setaparam("userdirs", Vec::new());
        setaparam("nameddirs", Vec::new());
        assert_eq!(_tilde_files(&[]), 1);
    }
}
