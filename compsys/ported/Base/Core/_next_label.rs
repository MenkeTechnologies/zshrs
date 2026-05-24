//! Port of `_next_label` — get next label for a tag (for iteration).
//!
//! Extracted from `compsys/base.rs` (was `pub fn next_label`, lines
//! ~370-376). Renamed to `_next_label` to match the upstream zsh shell
//! function name at `Completion/Base/Core/_next_label`.

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
}
