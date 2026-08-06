//! Port of `_dynamic_directory_name` from
//! `Completion/Zsh/Context/_dynamic_directory_name`.
//!
//! Full upstream body (29 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  local -a dirfuncs=(
//! sh: 3      ${(k)functions[zsh_directory_name]}
//! sh: 4      $zsh_directory_name_functions
//! sh: 5  )
//! sh: 6  local descr='dynamically named directory'
//! sh: 8  if (( $#dirfuncs )); then
//! sh: 9    local -a expl
//! sh:10    local -i ret
//! sh:11    local func suf tag=dynamically-named-directories
//! sh:13    [[ $ISUFFIX != \]* ]] &&
//! sh:14        suf=-S]
//! sh:16    _tags "$tag"
//! sh:17    while _tags; do
//! sh:18      while _next_label "$tag" expl "$descr" $suf; do
//! sh:19        for func in $dirfuncs; do
//! sh:20          $func c && ret=0
//! sh:21        done
//! sh:22      done
//! sh:23      (( ret )) || break
//! sh:24    done
//! sh:25    return ret
//! sh:26  else
//! sh:27    _message "${descr}: implement as zsh_directory_name c"
//! sh:28  fi
//! ```

use crate::compsys::ported::_message::message_byname;
use crate::compsys::ported::_next_label::next_label_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::utils::getshfunc;

/// Reach `_dynamic_directory_name` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_dynamic_directory_name` (Completion/Zsh/Context/_subscript sh:24) — so the normal function lookup runs.
///
/// A plain Rust call to the sibling port skips both of
/// [`crate::compsys::ported::shared::call_compfn`]'s effects: `$fpath` /
/// shfunc arbitration (the user's own copy of the function is inert) and
/// the `doshfunc` frame (no `FUNCSTACK` entry, and the callee's
/// `declare_locals` land in the CALLER's param scope instead of its own).
///
/// The direct call stays as the fallback: it runs only when neither a shell
/// function nor a registered port claims the name — i.e. in unit tests with
/// no executor installed.
pub fn dynamic_directory_name_byname() -> i32 {
    crate::compsys::ported::shared::call_compfn("_dynamic_directory_name", &[], || {
        _dynamic_directory_name()
    })
}

/// `_dynamic_directory_name` — `~[name]` lookup via user-defined
/// `zsh_directory_name` function + `$zsh_directory_name_functions`.
pub fn _dynamic_directory_name() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_dynamic_directory_name");
    // sh:2-5  gather dispatch functions
    let mut dirfuncs: Vec<String> = Vec::new();
    if getshfunc("zsh_directory_name").is_some() {
        dirfuncs.push("zsh_directory_name".to_string());
    }
    let extra = getaparam("zsh_directory_name_functions").unwrap_or_default();
    dirfuncs.extend(extra);

    // sh:6
    let descr = "dynamically named directory";

    if !dirfuncs.is_empty() {
        // sh:11
        let tag = "dynamically-named-directories".to_string();
        let isuffix = getsparam("ISUFFIX").unwrap_or_default();
        // sh:13-14
        let suf: Vec<String> = if !isuffix.starts_with(']') {
            vec!["-S]".to_string()]
        } else {
            Vec::new()
        };

        // sh:16
        let _ = tags_byname(&[tag.clone()]);
        let mut ret: i32 = 1;

        // sh:17-24
        loop {
            if tags_byname(&[]) != 0 {
                break;
            }
            // sh:18
            loop {
                let mut nl_args = vec![tag.clone(), "expl".to_string(), descr.to_string()];
                nl_args.extend(suf.iter().cloned());
                if next_label_byname(&nl_args) != 0 {
                    break;
                }
                // sh:19-21
                for func in &dirfuncs {
                    if dispatch_function_call(func, &["c".to_string()]).unwrap_or(1) == 0 {
                        ret = 0;
                    }
                }
            }
            // sh:23
            if ret == 0 {
                break;
            }
        }
        // sh:25
        ret
    } else {
        // sh:27
        message_byname(&[format!("{}: implement as zsh_directory_name c", descr)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_message_path_without_dirfuncs() {
        // sh:27 — no dirfuncs → _message path. _message returns 1
        //   when `messages` tag isn't registered.
        let _g = crate::test_util::global_state_lock();
        let _r = _dynamic_directory_name();
    }
}
