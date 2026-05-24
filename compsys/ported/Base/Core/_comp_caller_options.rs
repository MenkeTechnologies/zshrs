//! Port of `_comp_caller_options` — get shell options from calling
//! context.
//!
//! Local shell reference: this name appears in compsys completers
//! (e.g. `_complete`) as a snapshot of the `$options` associative
//! array captured at compsys-entry. The shell-side definition lives
//! in zsh's compsys init (`compinit` sets `_comp_caller_options`
//! before dispatching). No standalone `_comp_caller_options` file
//! exists in the system functions tree — it's a shell parameter
//! initialised inline.
//!
//! Rust port: returns an empty HashMap. The shell parameter has no
//! direct analog at the leaf — option capture is plumbed through
//! `CompletionState.params.compstate` upstream. Pinning the empty
//! contract here lets call sites that DON'T need real option capture
//! continue working; a future change to populate from a real option
//! table surfaces deliberately via the existing test below.

use std::collections::HashMap;

/// _comp_caller_options - Get options from calling context
pub fn _comp_caller_options() -> HashMap<String, bool> {
    // Returns shell options that were set when completion was invoked
    // This is stored in $_comp_caller_options in zsh
    HashMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_empty_until_shell_integration_wires_options() {
        // The shell-side `_comp_caller_options` reflects `$options`
        // captured at compsys-entry. The Rust leaf has no shell to
        // read from, so the contract is "empty map" — pin that so any
        // future change to a populated default surfaces deliberately.
        let opts = _comp_caller_options();
        assert!(opts.is_empty());
    }
}
