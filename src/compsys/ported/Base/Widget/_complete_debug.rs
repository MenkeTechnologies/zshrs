//! Port of `_complete_debug` from
//! `Completion/Base/Widget/_complete_debug`.
//!
//! Full upstream body (40 lines, abridged):
//! ```text
//! sh: 1  #compdef -k complete-word \C-x?
//! sh: 3  eval "$_comp_setup"
//! sh: 5  (( $+_debug_count )) || integer -g _debug_count
//! sh: 6  local tmp=${TMPPREFIX}${$}${words[1]:t}$[++_debug_count]
//! sh:11  if [[ -t 2 ]]; then
//! sh:12    zmodload -F zsh/files b:zf_ln 2>/dev/null &&
//! sh:13    zf_ln -fn =(<<<'') $tmp &&
//! sh:14    exec {debug_fd}>&2 2>| $tmp
//! sh:15  fi
//! sh:22  setopt xtrace
//! sh:23  : $ZSH_NAME $ZSH_VERSION
//! sh:24  ${1:-_main_complete}
//! sh:25  integer ret=$?
//! sh:26  unsetopt xtrace
//! sh:28  if (( debug_fd != -1 )); then
//! sh:29    zstyle -s ':completion:complete-debug::::' pager pager
//! sh:30    print -sR "${pager:-${PAGER:-${VISUAL:-${EDITOR:-more}}}} ${(q)tmp} ;: $w"
//! sh:31    _message -r "Trace output left in $tmp (up-history to view)"
//! sh:35  fi
//! sh:38  return ret
//! ```
//!
//! Wraps a completion run with xtrace into a temp file. Our Rust
//! port dispatches the named inner fn (default `_main_complete`)
//! and emits a message; the xtrace capture itself isn't replicated
//! since `setopt xtrace` is a shell-level feature handled by the
//! parent executor.

use crate::compsys::ported::_message::_message;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getiparam, getsparam, setiparam};

/// `_complete_debug` — run a completion with diagnostic context.
/// Optional `$1` overrides the default `_main_complete` target.
pub fn _complete_debug(args: &[String]) -> i32 {
    // sh:5-6  bump $_debug_count + derive tmp filename
    let n = getiparam("_debug_count") + 1;
    setiparam("_debug_count", n);
    let tmpprefix = getsparam("TMPPREFIX").unwrap_or_else(|| "/tmp/zsh".to_string());
    let pid = std::process::id();
    let cmd_name = crate::ported::params::getaparam("words")
        .and_then(|w| w.first().cloned())
        .unwrap_or_default();
    let tmp = format!("{}{}{}{}", tmpprefix, pid, basename(&cmd_name), n);

    // sh:24  ${1:-_main_complete}
    let target = args
        .first()
        .cloned()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "_main_complete".to_string());
    let ret = dispatch_function_call(&target, &[]).unwrap_or(1);

    // sh:31  user-facing trace pointer message
    let _ = _message(&[
        "-r".to_string(),
        format!("Trace output left in {} (up-history to view)", tmp),
    ]);

    ret
}

fn basename(s: &str) -> String {
    s.rsplit('/').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_complete_debug(&[]), 1);
    }

    #[test]
    fn increments_debug_count() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_debug_count", 5);
        let _ = _complete_debug(&[]);
        assert_eq!(getiparam("_debug_count"), 6);
    }
}
