//! Port of `_complete_help_generic` from
//! `Completion/Base/Utility/_complete_help_generic`.
//!
//! Full upstream body (17 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # Note this is a normal ZLE widget, not a completion widget.
//! sh: 4  # A completion widget can't call another widget, while a normal
//! sh: 5  # widget can.
//! sh: 7  [[ $WIDGET = *noread* ]] || local ZSH_TRACE_GENERIC_WIDGET
//! sh: 9  if [[ $WIDGET = *debug* ]]; then
//! sh:10    ZSH_TRACE_GENERIC_WIDGET=_complete_debug
//! sh:11  else
//! sh:12    ZSH_TRACE_GENERIC_WIDGET=_complete_help
//! sh:13  fi
//! sh:15  if [[ $WIDGET != *noread* ]]; then
//! sh:16    zle read-command && zle $REPLY -w
//! sh:17  fi
//! ```
//!
//! `zle read-command && zle $REPLY -w` requires an active ZLE read
//! loop — compsys-side has no ZLE bridge. The trace-widget env var
//! setup IS portable; the zle-read step degrades to a no-op when
//! the dispatch hook isn't wired.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setsparam};

/// `_complete_help_generic` — ZLE widget that arms the trace-widget
/// hook and reads the next zle command. Returns 0.
pub fn _complete_help_generic() -> i32 {
    let widget = getsparam("WIDGET").unwrap_or_default();

    // sh:9-13
    let trace = if widget.contains("debug") {
        "_complete_debug"
    } else {
        "_complete_help"
    };
    let _ = setsparam("ZSH_TRACE_GENERIC_WIDGET", trace);

    // sh:15-17  read-command via dispatch hook (zle-aware code path).
    //   In non-zle contexts this is a no-op.
    if !widget.contains("noread") {
        if let Some(rc) = dispatch_function_call("zle", &["read-command".to_string()]) {
            if rc == 0 {
                let reply = getsparam("REPLY").unwrap_or_default();
                let _ = dispatch_function_call("zle", &[reply, "-w".to_string()]);
            }
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_trace_widget_to_help_by_default() {
        // sh:12
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("WIDGET", "some-widget");
        let _ = _complete_help_generic();
        assert_eq!(
            getsparam("ZSH_TRACE_GENERIC_WIDGET").as_deref(),
            Some("_complete_help")
        );
    }

    #[test]
    fn sets_trace_widget_to_debug_when_name_contains_debug() {
        // sh:10
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("WIDGET", "complete-debug-something");
        let _ = _complete_help_generic();
        assert_eq!(
            getsparam("ZSH_TRACE_GENERIC_WIDGET").as_deref(),
            Some("_complete_debug")
        );
    }
}
