//! Port of `_options` — complete shell options. Moved from
//! `compsys/library.rs`. Renamed from `options` to mirror zsh shell
//! function name `_options`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _options - Complete shell options
pub fn _options(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("options", true);

    for (opt, is_set) in shell_options {
        if opt.starts_with(&prefix) {
            let mut comp = Completion::new(opt.to_string());
            comp.disp = Some(format!(
                "{} ({})",
                opt,
                if *is_set { "set" } else { "unset" }
            ));
            state.add_match(comp, Some("options"));
        }
    }

    state.end_group();
    state.nmatches > 0
}
