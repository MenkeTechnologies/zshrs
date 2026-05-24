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
