//! Port of `_comp_caller_options` — get shell options from calling
//! context. Moved from `compsys/library.rs`.

use std::collections::HashMap;

/// _comp_caller_options - Get options from calling context
pub fn _comp_caller_options() -> HashMap<String, bool> {
    // Returns shell options that were set when completion was invoked
    // This is stored in $_comp_caller_options in zsh
    HashMap::new()
}
