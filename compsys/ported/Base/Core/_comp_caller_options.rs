//! Port of `_comp_caller_options` — get shell options from calling
//! context. Moved from `compsys/library.rs`.

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
