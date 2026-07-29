//! Port of `_vcs_info` from `Completion/Zsh/Function/_vcs_info`.
//!
//! `#compdef vcs_info_hookadd vcs_info_hookdel` — completes the two
//! vcs_info hook-management functions: a hook *type* (first argument) and
//! then any number of hook *function* names (`_vcs_info_hooks`).
//!
//! Full upstream body (32 lines, abridged — head is the `#compdef` line):
//! ```text
//! sh: 3  local -a hook_types=( gen-applied-string … start-up )
//! sh:20  local -a specs
//! sh:21  case $service in
//! sh:22    (vcs_info_hookdel)
//! sh:23      specs=( '-a[remove all occurrences, not just the first]' )
//! sh:24      ;;
//! sh:25  esac
//! sh:28  _arguments : \
//! sh:29    $specs \
//! sh:30    ":hook type:($hook_types)" \
//! sh:31    '*:hook function:_vcs_info_hooks'
//! ```

use crate::compsys::ported::_arguments::_arguments;
use crate::ported::params::getsparam;

/// sh:3-18 — the fixed list of vcs_info hook types, in source order.
const HOOK_TYPES: [&str; 14] = [
    "gen-applied-string",
    "gen-hg-bookmark-string",
    "gen-mqguards-string",
    "gen-unapplied-string",
    "no-vcs",
    "post-backend",
    "post-quilt",
    "pre-addon-quilt",
    "pre-get-data",
    "set-branch-format",
    "set-hgrev-format",
    "set-message",
    "set-patch-format",
    "start-up",
];

/// sh:20-31 — assemble the argument vector passed to `_arguments`.
///
/// `service` is `$service`: `vcs_info_hookdel` gets the extra `-a` spec
/// (sh:22-24). `":hook type:($hook_types)"` splices the hook-type array
/// into a single `(w1 w2 …)` action group (default IFS = space), and the
/// trailing `*:hook function:_vcs_info_hooks` accepts any number of hook
/// function names.
fn build_call(service: &str) -> Vec<String> {
    let mut call: Vec<String> = Vec::new();
    call.push(":".to_string()); // sh:28 — leading `:` (no-op separator arg)

    // sh:21-25 — specs=( … ) only for vcs_info_hookdel.
    if service == "vcs_info_hookdel" {
        call.push("-a[remove all occurrences, not just the first]".to_string());
        // sh:23
    }

    // sh:30 — ":hook type:($hook_types)"
    call.push(format!(":hook type:({})", HOOK_TYPES.join(" ")));
    // sh:31 — '*:hook function:_vcs_info_hooks'
    call.push("*:hook function:_vcs_info_hooks".to_string());

    call
}

/// `_vcs_info` — completion for `vcs_info_hookadd` / `vcs_info_hookdel`.
pub fn _vcs_info(_args: &[String]) -> i32 {
    // $service selects the dispatched compdef name (sh:21).
    let service = getsparam("service").unwrap_or_default();
    let call = build_call(&service);
    _arguments(&call) // sh:28-31
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hookadd_has_no_extra_spec() {
        let call = build_call("vcs_info_hookadd");
        assert_eq!(call[0], ":");
        assert!(!call.iter().any(|s| s.starts_with("-a[")));
        // Leading `:`, hook-type spec, hook-function spec.
        assert_eq!(call.len(), 3);
        assert_eq!(call.last().unwrap(), "*:hook function:_vcs_info_hooks");
    }

    #[test]
    fn hookdel_gets_dash_a_spec() {
        let call = build_call("vcs_info_hookdel");
        assert_eq!(call.len(), 4);
        assert_eq!(call[1], "-a[remove all occurrences, not just the first]");
    }

    #[test]
    fn hook_type_spec_joins_all_types_with_spaces() {
        let call = build_call("");
        let spec = call.iter().find(|s| s.starts_with(":hook type:")).unwrap();
        assert_eq!(
            spec,
            ":hook type:(gen-applied-string gen-hg-bookmark-string gen-mqguards-string \
             gen-unapplied-string no-vcs post-backend post-quilt pre-addon-quilt pre-get-data \
             set-branch-format set-hgrev-format set-message set-patch-format start-up)"
        );
    }
}
