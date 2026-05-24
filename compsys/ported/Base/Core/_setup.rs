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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_packed_zstyle_appends_packed() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::values",
            "list-packed",
            vec!["yes".into()],
            false,
        );
        _setup(&mut state, "values");
        assert!(state.comp.params.compstate.list.contains("packed"));
    }

    #[test]
    fn accept_exact_zstyle_sets_compstate_exact() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::values",
            "accept-exact",
            vec!["true".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(state.comp.params.compstate.exact, "accept");
    }

    #[test]
    fn force_list_always_appends_force_marker() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::values",
            "force-list",
            vec!["always".into()],
            false,
        );
        _setup(&mut state, "values");
        assert!(state.comp.params.compstate.list.contains("force"));
    }
}
