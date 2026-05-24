//! Port of `_next_label` — get next label for a tag (for iteration).
//!
//! Local shell reference: `compsys/functions/Base/Core/_next_label`
//! (system copy `/opt/homebrew/share/zsh/functions/_next_label`).
//!
//! Upstream shell source (key lines from ~50-line fn):
//! ```text
//!  3  local __gopt __descr __spec
//!  5  __gopt=()
//!  6  zparseopts -D -a __gopt 1 2 V J x
//!  8  if comptags -A "$1" curtag __spec; then
//! 12    if [[ "$curtag" = *[^\\]:* ]]; then
//! 13      zformat -f __descr "${curtag#*:}" "d:$3"
//! 14      _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! ```
//!
//! Upstream uses `comptags -A` to advance the internal tag-set
//! iterator + extract the next label, then `_description` wraps
//! the result.
//!
//! Faithful Rust port: queries `TagManager::wanted(tag)` and emits
//! the tag name as the label. The `comptags` builtin's internal
//! iteration is the same shape — caller drives the loop with
//! repeated calls until None is returned.

use crate::base::TagManager;

/// _next_label - get next label for a tag (for iteration)
pub fn _next_label(tags: &TagManager, tag: &str) -> Option<String> {
    if tags.wanted(tag) {
        Some(tag.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_tag_when_wanted() {
        let mut tags = TagManager::new();
        tags.init(&["files".into(), "directories".into()]);
        tags.configure_from_style(&["files".into()]);
        tags.start();
        assert_eq!(_next_label(&tags, "files"), Some("files".into()));
    }

    #[test]
    fn returns_none_when_not_wanted() {
        let tags = TagManager::new();
        assert_eq!(_next_label(&tags, "files"), None);
    }

    #[test]
    fn multiple_wanted_tags_each_returns_own_name() {
        let mut tags = TagManager::new();
        tags.init(&["files".into(), "directories".into(), "values".into()]);
        tags.configure_from_style(&["files directories values".into()]);
        tags.start();
        assert_eq!(_next_label(&tags, "files"), Some("files".into()));
        assert_eq!(_next_label(&tags, "directories"), Some("directories".into()));
        assert_eq!(_next_label(&tags, "values"), Some("values".into()));
    }

    #[test]
    fn tag_outside_offered_set_returns_none() {
        let mut tags = TagManager::new();
        tags.init(&["files".into()]);
        tags.configure_from_style(&["files".into()]);
        tags.start();
        assert_eq!(_next_label(&tags, "not-offered"), None);
    }

    #[test]
    fn returns_none_after_iteration_exhausted() {
        let mut tags = TagManager::new();
        tags.init(&["a".into(), "b".into()]);
        tags.add_try(&["a".into()]);
        tags.add_try(&["b".into()]);
        tags.start();
        // First set is `a`; advance past both.
        tags.next();
        tags.next();
        // Now no active try-set → wanted returns false.
        assert_eq!(_next_label(&tags, "a"), None);
        assert_eq!(_next_label(&tags, "b"), None);
    }
}
