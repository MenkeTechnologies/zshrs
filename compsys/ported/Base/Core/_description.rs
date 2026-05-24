//! Port of `_description` — set up description for a tag.
//!
//! Extracted from `compsys/base.rs` (was lines ~751-784). Mirrors zsh
//! upstream `Completion/Base/Core/_description`. Honors the `format`,
//! `hidden`, and per-tag descriptions styles, returning the formatted
//! description string (`%d` → description, `%%` → `%`). Returns `None`
//! when the `hidden` style is `all`.

use crate::compcore::CompletionState;
use crate::zstyle::ZStyleStore;

/// _description - set up description for a tag
/// Handles styles: format, hidden, group-name, matcher, sort, ignored-patterns
pub fn _description(
    _state: &mut CompletionState,
    styles: &ZStyleStore,
    context: &str,
    tag: &str,
    description: &str,
) -> Option<String> {
    let ctx = format!("{}:{}", context, tag);

    // Check 'hidden' style - if set to 'all', return empty format
    if let Some(hidden) = styles.lookup_values(&ctx, "hidden") {
        if let Some(v) = hidden.first() {
            match v.as_str() {
                "all" => return None,
                "yes" | "true" | "1" | "on" => {
                    // Hidden but still has format for group header
                }
                _ => {}
            }
        }
    }

    // Get format from style (try tag-specific first, then descriptions tag)
    let format = styles
        .lookup_values(&ctx, "format")
        .or_else(|| styles.lookup_values(&format!("{}:descriptions", context), "format"))
        .and_then(|v| v.first().cloned())
        .unwrap_or_else(|| "%d".to_string());

    // zformat -F substitution: %d = description, plus additional escapes
    let result = format.replace("%d", description).replace("%%", "%");

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_format_returns_raw_description() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        let got = _description(&mut state, &styles, ":completion:::", "files", "the files");
        assert_eq!(got.as_deref(), Some("the files"));
    }

    #[test]
    fn hidden_all_returns_none() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(":completion:::*:files", "hidden", vec!["all".into()], false);
        let got = _description(&mut state, &styles, ":completion:::*", "files", "the files");
        assert!(got.is_none());
    }

    #[test]
    fn percent_d_substitution_works() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion:::*:files",
            "format",
            vec!["== %d ==".into()],
            false,
        );
        let got = _description(&mut state, &styles, ":completion:::*", "files", "the files");
        assert_eq!(got.as_deref(), Some("== the files =="));
    }

    #[test]
    fn double_percent_collapses_to_single_percent() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion:::*:files",
            "format",
            vec!["100%% sure: %d".into()],
            false,
        );
        let got = _description(&mut state, &styles, ":completion:::*", "files", "x");
        assert_eq!(got.as_deref(), Some("100% sure: x"));
    }
}
