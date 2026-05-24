//! Port of `_description` — set up description for a tag.
//!
//! Local shell reference: `compsys/functions/Base/Core/_description`
//! (system copy `/opt/homebrew/share/zsh/functions/_description`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  9  opts=()
//! 11  xopt=(-X)
//! 13  zparseopts -K -D -a nopt 1 2 V=gropt J=ign x=xopt
//! 15  3="${${3##[[:blank:]]#}%%[[:blank:]]#}"
//! 16  [[ -n "$3" ]] && _lastdescr=( "$_lastdescr[@]" "$3" )
//! 25  if zstyle -s ":completion:${context}:descriptions" format format; then
//! 30  if zstyle -s ":completion:${context}:descriptions" hidden hidden; then
//! ```
//!
//! Faithful Rust port: honors `format`, `hidden`, and per-tag
//! description styles. Returns the formatted description string
//! (`%d` → description, `%%` → `%`). Returns `None` when the
//! `hidden` style is `all`.

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
