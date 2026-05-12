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
        if self.assoc_arrays.contains_key(name) {
            a.set(crate::recorder::ParamAttrs::ASSOC);
        } else if self.arrays.contains_key(name) {
            a.set(crate::recorder::ParamAttrs::ARRAY);
        } else if let Some(va) = self.var_attrs.get(name) {
            match va.kind {
                VarKind::Integer => a.set(crate::recorder::ParamAttrs::INTEGER),
                VarKind::Float => a.set(crate::recorder::ParamAttrs::FLOAT),
                VarKind::Association => a.set(crate::recorder::ParamAttrs::ASSOC),
                VarKind::Array => a.set(crate::recorder::ParamAttrs::ARRAY),
                VarKind::Scalar => a.set(crate::recorder::ParamAttrs::SCALAR),
            }
        } else {
            a.set(crate::recorder::ParamAttrs::SCALAR);
        }
        // Compose modifier bits regardless of shape.
        if let Some(va) = self.var_attrs.get(name) {
            if va.readonly {
                a.set(crate::recorder::ParamAttrs::READONLY);
            }
            if va.export {
                a.set(crate::recorder::ParamAttrs::EXPORT);
            }
            if va.unique {
                a.set(crate::recorder::ParamAttrs::UNIQUE);
            }
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
        let fn_chain = self.arrays.get("funcstack").and_then(|s| {
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
