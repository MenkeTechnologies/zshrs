//! Port of `_tilde` from `Completion/Zsh/Context/_tilde`.
//!
//! Full upstream body (32 lines verbatim):
//! ```text
//! sh: 1  #compdef -tilde-
//! sh: 7  [[ -n "$compstate[quote]" ]] && return 1
//! sh: 9  local expl suf ret=1
//! sh:11  if [[ "$SUFFIX" = */* ]]; then
//! sh:12    ISUFFIX="/${SUFFIX#*/}$ISUFFIX"
//! sh:13    SUFFIX="${SUFFIX%%/*}"
//! sh:14    suf=(-S '')
//! sh:15  else
//! sh:16    suf=(-qS/)
//! sh:17  fi
//! sh:19  _tags users named-directories directory-stack
//! sh:21  while _tags; do
//! sh:22    _requested users && _users "$suf[@]" "$@" && ret=0
//! sh:24    _requested named-directories expl 'named directory' \
//! sh:25        compadd "$suf[@]" "$@" -k nameddirs && ret=0
//! sh:27    _requested directory-stack && _directory_stack "$suf[@]" && ret=0
//! sh:29    (( ret )) || return 0
//! sh:30  done
//! sh:31
//! sh:32  return ret
//! ```

use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setsparam};
use crate::ported::zle::compcore::get_compstate_str;

/// `_tilde` — complete `~user` / `~name` / `~+N` tilde expansions.
pub fn _tilde(args: &[String]) -> i32 {
    // sh:7
    if !get_compstate_str("quote").unwrap_or_default().is_empty() {
        return 1;
    }

    let mut ret: i32 = 1;

    // sh:11-17
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let suf: Vec<String> = if suffix.contains('/') {
        let head = suffix.splitn(2, '/').next().unwrap_or("").to_string();
        let tail = suffix.splitn(2, '/').nth(1).unwrap_or("").to_string();
        let isuf = getsparam("ISUFFIX").unwrap_or_default();
        let _ = setsparam("ISUFFIX", &format!("/{}{}", tail, isuf));
        let _ = setsparam("SUFFIX", &head);
        vec!["-S".to_string(), "".to_string()]
    } else {
        vec!["-qS/".to_string()]
    };

    // sh:19
    let _ = _tags(&[
        "users".to_string(),
        "named-directories".to_string(),
        "directory-stack".to_string(),
    ]);

    // sh:21-30
    loop {
        if _tags(&[]) != 0 {
            break;
        }
        // sh:22  _requested users && _users
        if _requested(&["users".to_string()]) == 0 {
            let mut users_args: Vec<String> = suf.clone();
            users_args.extend(args.iter().cloned());
            if dispatch_function_call("_users", &users_args).unwrap_or(1) == 0 {
                ret = 0;
            }
        }
        // sh:24-25  _requested named-directories
        let mut nd_args: Vec<String> = vec![
            "named-directories".to_string(),
            "expl".to_string(),
            "named directory".to_string(),
            "compadd".to_string(),
        ];
        nd_args.extend(suf.iter().cloned());
        nd_args.extend(args.iter().cloned());
        nd_args.push("-k".to_string());
        nd_args.push("nameddirs".to_string());
        if _requested(&nd_args) == 0 {
            ret = 0;
        }
        // sh:27  _requested directory-stack && _directory_stack
        if _requested(&["directory-stack".to_string()]) == 0 {
            if dispatch_function_call("_directory_stack", &suf).unwrap_or(1) == 0 {
                ret = 0;
            }
        }
        // sh:29
        if ret == 0 {
            return 0;
        }
    }

    // sh:32
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::compcore::set_compstate_str;

    #[test]
    fn quote_set_returns_one() {
        // sh:7 — $compstate[quote] non-empty short-circuits.
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("quote", "'");
        assert_eq!(_tilde(&[]), 1);
        set_compstate_str("quote", "");
    }
}
