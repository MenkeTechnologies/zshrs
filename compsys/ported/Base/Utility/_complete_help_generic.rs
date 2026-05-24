//! Port of `_complete_help_generic` — generic help completion. Moved
//! from `compsys/functions.rs`. Renamed from `complete_help_generic`
//! to mirror zsh shell function name `_complete_help_generic`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _complete_help_generic - Generic help completion
pub fn _complete_help_generic(state: &mut CompletionState, help_text: &str) -> bool {
    let prefix = state.params.prefix.clone();

    // Parse --option lines from help text
    let mut options = Vec::new();

    for line in help_text.lines() {
        let line = line.trim();
        if line.starts_with('-') {
            // Extract option and description
            let parts: Vec<&str> = line.splitn(2, |c: char| c.is_whitespace()).collect();
            if let Some(opt) = parts.first() {
                let desc = parts.get(1).unwrap_or(&"").trim();
                if opt.starts_with(&prefix) || prefix.is_empty() {
                    options.push((opt.to_string(), desc.to_string()));
                }
            }
        }
    }

    if options.is_empty() {
        return false;
    }

    state.begin_group("options", true);
    for (opt, desc) in options {
        let mut comp = Completion::new(&opt);
        if !desc.is_empty() {
            comp.disp = Some(format!("{} -- {}", opt, desc));
        }
        state.add_match(comp, Some("options"));
    }
    state.end_group();

    true
}
