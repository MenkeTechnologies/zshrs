//! Port of `_delimiters` from `Completion/Zsh/Type/_delimiters`.
//!
//! Full upstream body (16 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Simple function to offer delimiters for modifiers and qualifiers.
//! sh: 4  # Single argument is tag to use.
//! sh: 5
//! sh: 6  local expl
//! sh: 7  local -a list
//! sh: 8
//! sh: 9  zstyle -a ":completion:${curcontext}:$1" delimiters list ||
//! sh:10    list=(: + / - %)
//! sh:11
//! sh:12  if (( ${#list} )); then
//! sh:13    _wanted delimiters expl delimiter compadd -S '' -a list
//! sh:14  else
//! sh:15    _message delimiter
//! sh:16  fi
//! ```
//!
//! Strict Rust port: takes the `tag` (used in the style lookup
//! key); falls back to the upstream default list `: + / - %`.



use crate::compsys::base::MainCompleteState;
use crate::compsys::completion::{Completion, CompletionFlags};

/// Default delimiter list (matches upstream).
pub const DEFAULT_DELIMITERS: &[&str] = &[":", "+", "/", "-", "%"];

/// `_delimiters` — emit candidate delimiter chars under the
/// `delimiters` tag.
pub fn _delimiters(state: &mut MainCompleteState, tag: &str) -> bool {
    // shell:4-5 — `zstyle -a … delimiters list || list=(: + / - %)`.
    let style_ctx = format!(":completion:{}:{}", state.ctx.context, tag);
    let list: Vec<String> = state
        .styles
        .lookup_values(&style_ctx, "delimiters")
        .map(|v| v.to_vec())
        .unwrap_or_else(|| DEFAULT_DELIMITERS.iter().map(|s| s.to_string()).collect());

    if list.is_empty() {
        // shell:9 — `_message delimiter` (delegate to our ported
        // `_message`).
        let styles = state.styles.clone();
        crate::compsys::ported::_message::_message(
            &mut state.comp,
            &styles,
            &state.ctx.context.clone(),
            "messages",
            "delimiter",
        );
        return false;
    }

    // shell:7 — `_wanted delimiters expl delimiter compadd -S '' -a list`
    let prefix = state.comp.params.prefix.clone();
    crate::compsys::ported::_wanted::_wanted(state, "delimiters", "delimiter", |s| {
        let mut any = false;
        for d in &list {
            if !d.starts_with(&prefix) {
                continue;
            }
            // `compadd -S ''` = empty auto-suffix → NOSPACE.
            let mut comp = Completion::new(d.clone());
            comp.flags |= CompletionFlags::NOSPACE;
            s.add_match(comp, Some("delimiters"));
            any = true;
        }
        any
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compsys::base::TagManager;

    fn seed(state: &mut MainCompleteState) {
        state.tags = TagManager::new();
        state.tags.init(&["delimiters".into()]);
        state.tags.add_try(&["delimiters".into()]);
        let _ = state.tags.start();
    }

    #[test]
    fn default_list_emitted_when_style_unset() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        assert!(_delimiters(&mut state, "modifiers"));
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        for d in [":", "+", "/", "-", "%"] {
            assert!(names.contains(&d), "missing default delim `{d}`");
        }
    }

    #[test]
    fn custom_delimiters_style_overrides() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.ctx.context = ":t:".into();
        state.styles.set(
            ":completion::t::quals",
            "delimiters",
            vec!["@".into(), "#".into()],
            false,
        );
        let _ = _delimiters(&mut state, "quals");
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"@"));
        assert!(names.contains(&"#"));
        assert!(!names.contains(&":"));
    }

    #[test]
    fn each_emit_carries_nospace_flag() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let _ = _delimiters(&mut state, "x");
        for m in &state.comp.groups[0].matches {
            assert!(m.flags.contains(CompletionFlags::NOSPACE));
        }
    }

    #[test]
    fn untagged_call_skips_emission() {
        let mut state = MainCompleteState::new("", 0);
        // No seed → _wanted gate blocks the emission.
        assert!(!_delimiters(&mut state, "modifiers"));
    }

    #[test]
    fn prefix_filter_applies() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "/".into();
        let _ = _delimiters(&mut state, "x");
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["/"]);
    }

    #[test]
    fn default_list_is_five_chars() {
        assert_eq!(DEFAULT_DELIMITERS, [":", "+", "/", "-", "%"]);
    }
}
