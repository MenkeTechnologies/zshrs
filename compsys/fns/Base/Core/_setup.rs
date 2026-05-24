//! Port of `_setup` — set up completion context based on zstyle
//! settings. Moved from `compsys/functions.rs`. Renamed from `setup`
//! to mirror zsh shell function name `_setup`.

use crate::base::MainCompleteState;

/// _setup - Set up completion context based on zstyle settings
pub fn _setup(state: &mut MainCompleteState, tag: &str) {
    let context = format!(":completion:{}:{}", state.ctx.context, tag);

    // list-colors
    if let Some(colors) = state.styles.lookup_values(&context, "list-colors") {
        // Would set ZLS_COLORS
        let _ = colors;
    }

    // show-ambiguity
    if let Some(val) = state.styles.lookup_values(&context, "show-ambiguity") {
        if let Some(v) = val.first() {
            if v == "yes" || v == "true" || v == "on" {
                // Set ambiguous color to 4 (default)
            }
        }
    }

    // list-packed
    if state
        .styles
        .lookup_values(&context, "list-packed")
        .is_some()
    {
        state.comp.params.compstate.list.push_str(" packed");
    }

    // list-rows-first
    if state
        .styles
        .lookup_values(&context, "list-rows-first")
        .is_some()
    {
        state.comp.params.compstate.list.push_str(" rows");
    }

    // last-prompt
    if state
        .styles
        .lookup_values(&context, "last-prompt")
        .is_some()
    {
        state.comp.params.compstate.last_prompt = true.to_string();
    }

    // accept-exact
    if state
        .styles
        .lookup_values(&context, "accept-exact")
        .is_some()
    {
        state.comp.params.compstate.exact = "accept".to_string();
    }

    // menu style
    if let Some(menu) = state.styles.lookup_values(&context, "menu") {
        // Store menu style for later use
        let _ = menu;
    }

    // force-list
    if let Some(val) = state.styles.lookup_values(&context, "force-list") {
        if let Some(v) = val.first() {
            if v == "always" {
                state.comp.params.compstate.list.push_str(" force");
            }
        }
    }
}
