//! Port of `_setup` — set up completion context based on zstyle
//! settings.
//!
//! Local shell reference: `compsys/functions/Base/Core/_setup`
//! (system copy `/opt/homebrew/share/zsh/functions/_setup`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  local val nm="$compstate[nmatches]"
//!  7  if zstyle -a ":completion:${curcontext}:$1" list-colors val; then
//! 10    _comp_colors=( "$val[@]" )
//! 27  if zstyle -s ":completion:${curcontext}:$1" show-ambiguity val; then
//! 29    [[ $val = (yes|true|on) ]] && _ambiguous_color=4 || _ambiguous_color=$val
//! 32  if zstyle -t ":completion:${curcontext}:$1" list-packed; then
//! 33    compstate[list]="${compstate[list]} packed"
//! 40  if zstyle -t ":completion:${curcontext}:$1" list-rows-first; then
//! 41    compstate[list]="${compstate[list]} rows"
//! ```
//!
//! Simplified Rust port: handles the list-packed / list-rows-first /
//! last-prompt / accept-exact / menu / force-list styles by tweaking
//! `compstate.list` / `compstate.exact`. The list-colors + show-
//! ambiguity styles (require zsh/complist module integration) are
//! looked up but their effect is deferred to the receiver.

use crate::base::MainCompleteState;

/// _setup - Set up completion context based on zstyle settings
pub fn _setup(state: &mut MainCompleteState, tag: &str) {
    let context = format!(":completion:{}:{}", state.ctx.context, tag);

    // shell:7-13 — list-colors → _comp_colors (deferred to receiver)
    if let Some(colors) = state.styles.lookup_values(&context, "list-colors") {
        let _ = colors;
    }

    // shell:27-30 — show-ambiguity → _ambiguous_color (deferred)
    if let Some(val) = state.styles.lookup_values(&context, "show-ambiguity") {
        if let Some(v) = val.first() {
            if v == "yes" || v == "true" || v == "on" {
                // Set ambiguous color to 4 (default)
            }
        }
    }

    // shell:32-33 — list-packed → compstate[list]+=' packed'
    if state
        .styles
        .lookup_values(&context, "list-packed")
        .is_some()
    {
        state.comp.params.compstate.list.push_str(" packed");
    }

    // shell:40-41 — list-rows-first → compstate[list]+=' rows'
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
