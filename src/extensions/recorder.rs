//! Recorder helpers — extension; no zsh C counterpart.
#![cfg(feature = "recorder")]
#[allow(unused_imports)]
use crate::ported::exec::{ShellExecutor, VarKind};

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Look up the structured `ParamAttrs` for an existing parameter
    /// name in the current executor state. Used by every assign hook
    /// to populate the recorder event so replay can faithfully
    /// reconstruct typed declarations (`typeset -i`, `typeset -gx`,
    /// `typeset -A`, etc.) instead of emitting plain `NAME=val` and
    /// losing the readonly / export / integer / float bits.
    pub(crate) fn recorder_attrs_for(&self, name: &str) -> crate::recorder::ParamAttrs {
        let mut a = crate::recorder::ParamAttrs::NONE;
        // Shape: assoc > array > existing var-attrs declared shape > scalar default.
        // Shape + modifiers — read PM_* bits directly from canonical
        // paramtab (mirrors C's `pm->node.flags & PM_*` checks).
        use crate::ported::zsh_h::{
            PM_INTEGER, PM_EFLOAT, PM_FFLOAT, PM_EXPORTED, PM_READONLY, PM_UNIQUE,
        };
        let flags = self.param_flags(name) as u32;
        if self.has_assoc(name) {
            a.set(crate::recorder::ParamAttrs::ASSOC);
        } else if self.has_array(name) {
            a.set(crate::recorder::ParamAttrs::ARRAY);
        } else if flags & PM_INTEGER != 0 {
            a.set(crate::recorder::ParamAttrs::INTEGER);
        } else if flags & (PM_EFLOAT | PM_FFLOAT) != 0 {
            a.set(crate::recorder::ParamAttrs::FLOAT);
        } else {
            a.set(crate::recorder::ParamAttrs::SCALAR);
        }
        if flags & PM_READONLY != 0 {
            a.set(crate::recorder::ParamAttrs::READONLY);
        }
        if flags & PM_EXPORTED != 0 {
            a.set(crate::recorder::ParamAttrs::EXPORT);
        }
        if flags & PM_UNIQUE != 0 {
            a.set(crate::recorder::ParamAttrs::UNIQUE);
        }
        if self.readonly_vars.contains(name) {
            a.set(crate::recorder::ParamAttrs::READONLY);
        }
        if std::env::var_os(name).is_some() {
            a.set(crate::recorder::ParamAttrs::EXPORT);
        }
        a
    }
    /// Snapshot the executor's current source position for an
    /// outgoing recorder event. Phase 1 sources `$LINENO` and
    /// `$funcstack`; current source-file tracking is wired in
    /// Phase 2 alongside the source-stack push/pop in bin_dot.
    pub(crate) fn recorder_ctx(&self) -> crate::recorder::RecordCtx {
        let line = self
            .scalar("LINENO")
            .and_then(|s| s.parse::<u32>().ok());
        let fn_chain = self.array("funcstack").and_then(|s| {
            if s.is_empty() {
                None
            } else {
                let mut parts: Vec<&str> = s.iter().map(String::as_str).collect();
                parts.reverse();
                Some(parts.join(" > "))
            }
        });
        let file = crate::ported::params::getsparam("ZSH_SCRIPT")
            .or_else(|| crate::ported::params::getsparam("ZSH_ARGZERO"))
            .or_else(|| crate::ported::params::getsparam("0"));
        crate::recorder::RecordCtx {
            file,
            line,
            fn_chain,
        }
    }
}
// END moved-from-exec-rs
