//! Port of `_add-zsh-hook` from `Completion/Zsh/Function/_add-zsh-hook`.
//!
//! `#compdef add-zsh-hook`. A thin `_arguments` wrapper: it offers the
//! `add-zsh-hook` flags (`-L -d -D -U -z -k` with their mutual-exclusion
//! groups), a fixed set of hook classes, and — for the hook function —
//! either the already-installed hooks (under `-d`) or all functions.
//!
//! The upstream file defines a nested helper `_add-zsh-hook_hooks` at top
//! level and then calls `_add-zsh-hook "$@"`. `_arguments` reaches that
//! helper by name via its action `:hook function:_add-zsh-hook_hooks`, so
//! this port re-installs the helper as a real shell function (mirroring the
//! file being sourced) before delegating to the ported `_arguments`.
//!
//! Full upstream body (28 lines):
//! ```text
//! sh: 1  #compdef add-zsh-hook
//! sh: 3  _add-zsh-hook_hooks() {
//! sh: 4    local expl
//! sh: 5    if (( $+opt_args[-d] )); then
//! sh: 6      _wanted functions expl "installed hook" compadd -a - "$line[1]_functions" && return 0
//! sh: 7    else
//! sh: 8      _functions && return 0
//! sh: 9    fi
//! sh:10    return 1
//! sh:11  }
//! sh:13  _add-zsh-hook() {
//! sh:14    local context state state_descr line
//! sh:15    typeset -A opt_args
//! sh:16    _arguments -s -w -S : \
//! sh:17      "(-d -D -U -z -k)-L[output in form of 'typeset' commands]" \
//! sh:18      '(-L -D -U -z -k)-d[remove HOOK from the array]' \
//! sh:19      '(-L -d -U -z -k)-D[interpret HOOK as pattern to remove from the array]' \
//! sh:20      '(-L -d -D)-U[suppress alias expansion for functions]' \
//! sh:21      '(-L -d -D -k)-z[mark function for zsh-style autoloading]' \
//! sh:22      '(-L -d -D -z)-k[mark function for ksh-style autoloading]' \
//! sh:23      ':hook class:(chpwd precmd preexec periodic zshaddhistory zshexit zsh_directory_name)' \
//! sh:24      ':hook function:_add-zsh-hook_hooks'
//! sh:25  }
//! sh:27  _add-zsh-hook "$@"
//! ```

use crate::compsys::ported::_arguments::_arguments;
use crate::ported::exec::execute_script;

/// sh:3-11 — the nested `_add-zsh-hook_hooks` helper, verbatim. Installed as
/// a shell function so the `:hook function:_add-zsh-hook_hooks` action can
/// reach it through `_arguments`' by-name dispatch.
const HOOKS_FN_SOURCE: &str = r#"_add-zsh-hook_hooks() {
  local expl
  if (( $+opt_args[-d] )); then
    _wanted functions expl "installed hook" compadd -a - "$line[1]_functions" && return 0
  else
    _functions && return 0
  fi
  return 1
}"#;

/// sh:16-24 — the fixed `_arguments` spec list. The `'…'` and `"…"` quoting
/// in the source is only shell quoting; the resulting words are these.
fn build_specs() -> Vec<String> {
    [
        // sh:17
        "(-d -D -U -z -k)-L[output in form of 'typeset' commands]",
        // sh:18
        "(-L -D -U -z -k)-d[remove HOOK from the array]",
        // sh:19
        "(-L -d -U -z -k)-D[interpret HOOK as pattern to remove from the array]",
        // sh:20
        "(-L -d -D)-U[suppress alias expansion for functions]",
        // sh:21
        "(-L -d -D -k)-z[mark function for zsh-style autoloading]",
        // sh:22
        "(-L -d -D -z)-k[mark function for ksh-style autoloading]",
        // sh:23
        ":hook class:(chpwd precmd preexec periodic zshaddhistory zshexit zsh_directory_name)",
        // sh:24
        ":hook function:_add-zsh-hook_hooks",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// sh:16 — assemble the full `_arguments` argv: the leading flags `-s -w -S :`
/// followed by the spec list.
fn build_arguments_call() -> Vec<String> {
    let mut call: Vec<String> = vec![
        "-s".to_string(),
        "-w".to_string(),
        "-S".to_string(),
        ":".to_string(),
    ];
    call.extend(build_specs());
    call
}

/// `_add-zsh-hook` — completion for the `add-zsh-hook` function. The
/// completion-system positional args (`"$@"`, sh:27) are unused by the body,
/// exactly as in the shell source, which drives everything through the fixed
/// spec list and the compsys globals `_arguments` reads (`words`,
/// `curcontext`, `opt_args`, `line`).
pub fn _add_zsh_hook(_args: &[String]) -> i32 {
    // sh:3-11 — install the nested helper as a shell function (the file
    // defines it at top level before calling `_add-zsh-hook`).
    let _ = execute_script(HOOKS_FN_SOURCE);

    // sh:16-24 — _arguments -s -w -S : <specs…>
    _arguments(&build_arguments_call())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_match_upstream_list() {
        let specs = build_specs();
        // sh:17-24 — six flag specs + two positional specs.
        assert_eq!(specs.len(), 8);

        // Mutual-exclusion groups and descriptions are preserved verbatim.
        assert_eq!(
            specs[0],
            "(-d -D -U -z -k)-L[output in form of 'typeset' commands]"
        );
        assert_eq!(specs[1], "(-L -D -U -z -k)-d[remove HOOK from the array]");
        assert_eq!(
            specs[2],
            "(-L -d -U -z -k)-D[interpret HOOK as pattern to remove from the array]"
        );
        assert_eq!(
            specs[3],
            "(-L -d -D)-U[suppress alias expansion for functions]"
        );
        assert_eq!(
            specs[4],
            "(-L -d -D -k)-z[mark function for zsh-style autoloading]"
        );
        assert_eq!(
            specs[5],
            "(-L -d -D -z)-k[mark function for ksh-style autoloading]"
        );
    }

    #[test]
    fn hook_class_positional_lists_all_classes() {
        // sh:23 — the hook classes offered as a literal `(…)` action.
        let specs = build_specs();
        assert_eq!(
            specs[6],
            ":hook class:(chpwd precmd preexec periodic zshaddhistory zshexit zsh_directory_name)"
        );
    }

    #[test]
    fn hook_function_positional_uses_nested_helper() {
        // sh:24 — the second positional dispatches to the nested helper.
        let specs = build_specs();
        assert_eq!(specs[7], ":hook function:_add-zsh-hook_hooks");
    }

    #[test]
    fn arguments_call_has_leading_flags_then_specs() {
        // sh:16 — `_arguments -s -w -S :` prefix, then the eight specs.
        let call = build_arguments_call();
        assert_eq!(&call[..4], &["-s", "-w", "-S", ":"]);
        assert_eq!(call.len(), 4 + 8);
        assert_eq!(call.last().unwrap(), ":hook function:_add-zsh-hook_hooks");
    }

    #[test]
    fn hooks_fn_source_is_faithful() {
        // sh:3-11 — helper body branches on `$+opt_args[-d]`: installed hooks
        // via `_wanted`, otherwise all functions via `_functions`.
        assert!(HOOKS_FN_SOURCE.starts_with("_add-zsh-hook_hooks() {"));
        assert!(HOOKS_FN_SOURCE.contains("(( $+opt_args[-d] ))"));
        assert!(HOOKS_FN_SOURCE.contains(
            r#"_wanted functions expl "installed hook" compadd -a - "$line[1]_functions""#
        ));
        assert!(HOOKS_FN_SOURCE.contains("_functions && return 0"));
        assert!(HOOKS_FN_SOURCE.contains("return 1"));
    }
}
