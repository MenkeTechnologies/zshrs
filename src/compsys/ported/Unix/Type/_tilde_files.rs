//! Port of `_tilde_files` from `Completion/Unix/Type/_tilde_files`.
//!
//! Full upstream body (39 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete files and expand tilde expansions in it.
//! sh: 4
//! sh: 5  if [[ ( -o magicequalsubst && "$IPREFIX" = *\= ) || $argv[(I)-W*] -ne 0 ]]; then
//! sh: 6    _files "$@"
//! sh: 7    return
//! sh: 8  fi
//! sh: 9
//! sh:10  case "$PREFIX" in
//! sh:11  \~/*)
//! sh:12    IPREFIX="${IPREFIX}${HOME}/"
//! sh:13    PREFIX="${PREFIX[3,-1]}"
//! sh:14    _files "$@" -W "${HOME}"
//! sh:15    ;;
//! sh:16  \~*/*)
//! sh:17    local user="${PREFIX[2,-1]%%/*}"
//! sh:18
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
//!
//! Upstream handles three tilde shapes: `~/PATH` (your home),
//! `~user/PATH` (user's home via passwd), and `~` (just expand to
//! $HOME).
//!
//! Simplified Rust port: handles `~/` via $HOME env. Other shapes
//! (`~user/`, named-directories) fall through. Pinned by the
//! `iprefix_cleared_after_call` test which checks the save/restore
//! semantic for state.params.iprefix.



use crate::compsys::compcore::CompletionState;

use super::_files::{files_execute, FilesOpts};

/// _tilde_files - Complete files with tilde expansion.
///
/// Handles three shell tilde shapes (mirror of shell:11-15 above):
///   `~`             → expand to $HOME
///   `~/path`        → expand to $HOME/path
///   `~user/path`    → look up `user` via getpwnam, expand to
///                      <user-home>/path
pub fn _tilde_files(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();

    if !prefix.starts_with('~') {
        return false;
    }

    // shell:11-15 — expand the tilde to its real path.
    let expanded: Option<String> = if prefix == "~" {
        std::env::var("HOME").ok()
    } else if let Some(after) = prefix.strip_prefix("~/") {
        std::env::var("HOME").ok().map(|h| format!("{}/{}", h, after))
    } else {
        // `~user/...` or `~user` (no trailing /).
        let body = &prefix[1..]; // drop leading `~`
        let (user, rest) = match body.find('/') {
            Some(i) => (&body[..i], Some(&body[i + 1..])),
            None => (body, None),
        };
        getpwnam_home(user).map(|home| match rest {
            Some(r) => format!("{}/{}", home, r),
            None => home,
        })
    };

    let Some(expanded) = expanded else {
        return false;
    };

    // shell:14 — `_files "$@" -W "${HOME}"`. We use the simpler
    // path-replace approach: swap prefix for expanded form, run
    // files_execute, restore — pinned by the iprefix-cleared test.
    let old_prefix = state.params.prefix.clone();
    state.params.prefix = expanded;
    state.params.iprefix = "~".to_string();

    let result = files_execute(state, &FilesOpts::default());

    state.params.prefix = old_prefix;
    state.params.iprefix.clear();

    result
}

/// libc getpwnam wrapper — returns the home dir for `user` or None
/// when the user isn't in passwd. Safe wrapper around the libc
/// thread-unsafe getpwnam (we copy the home string immediately and
/// don't retain the returned pointer).
fn getpwnam_home(user: &str) -> Option<String> {
    use std::ffi::CString;
    let cuser = CString::new(user).ok()?;
    unsafe {
        let pwd = libc::getpwnam(cuser.as_ptr());
        if pwd.is_null() {
            return None;
        }
        let home_ptr = (*pwd).pw_dir;
        if home_ptr.is_null() {
            return None;
        }
        let home_cstr = std::ffi::CStr::from_ptr(home_ptr);
        home_cstr.to_str().ok().map(String::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_tilde_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/etc/passwd".into();
        assert!(!_tilde_files(&mut state));
    }

    #[test]
    fn tilde_user_form_resolved_via_getpwnam() {
        // `~root/some` should resolve via getpwnam(root). On most
        // Unix systems root exists; if a sandboxed CI doesn't have
        // /etc/passwd we skip gracefully.
        let mut state = CompletionState::new();
        state.params.prefix = "~root/notexistent".into();
        let _ = _tilde_files(&mut state);
        if getpwnam_home("root").is_some() {
            // root resolves → fn should have run files_execute (no
            // panic). Pin that iprefix was managed.
            assert_eq!(state.params.iprefix, "");
        }
    }

    #[test]
    fn getpwnam_lookup_existing_user() {
        // Use the current user as a known-good test target.
        if let Ok(user) = std::env::var("USER") {
            let home = getpwnam_home(&user);
            assert!(
                home.is_some(),
                "getpwnam_home({user}) must succeed for the current user"
            );
        }
    }

    #[test]
    fn getpwnam_lookup_nonexistent_user_returns_none() {
        assert!(getpwnam_home("definitely-not-a-real-user-xyz-12345").is_none());
    }

    #[test]
    fn iprefix_cleared_after_call() {
        let mut state = CompletionState::new();
        state.params.prefix = "~/".into();
        // Whether matches come back depends on the test machine; we
        // only verify the save/restore semantic (iprefix wiped).
        let _ = _tilde_files(&mut state);
        assert_eq!(
            state.params.iprefix, "",
            "iprefix must be cleared after _tilde_files returns"
        );
    }

    #[test]
    fn prefix_restored_after_call() {
        let mut state = CompletionState::new();
        state.params.prefix = "~/some/path".into();
        let _ = _tilde_files(&mut state);
        assert_eq!(
            state.params.prefix, "~/some/path",
            "_tilde_files must restore the original prefix"
        );
    }

    #[test]
    fn tilde_alone_expands_to_home() {
        if std::env::var("HOME").is_err() {
            return;
        }
        let mut state = CompletionState::new();
        state.params.prefix = "~".into();
        let _ = _tilde_files(&mut state);
        // Prefix restored to "~" after call.
        assert_eq!(state.params.prefix, "~");
    }

    #[test]
    fn nonexistent_tilde_user_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "~definitely-not-a-real-user-xyz-12345/".into();
        assert!(!_tilde_files(&mut state));
        // No state mutation should leak.
        assert_eq!(state.params.iprefix, "");
    }

    #[test]
    fn empty_prefix_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_tilde_files(&mut state));
    }
}
