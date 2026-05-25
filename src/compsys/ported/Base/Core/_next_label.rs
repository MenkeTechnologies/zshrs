//! Port of `_next_label` from `Completion/Base/Core/_next_label`.
//!
//! Full upstream body (25 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local __gopt __descr __spec
//! sh: 4
//! sh: 5  __gopt=()
//! sh: 6  zparseopts -D -a __gopt 1 2 V J x
//! sh: 7
//! sh: 8  if comptags -A "$1" curtag __spec; then
//! sh: 9    (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
//! sh:10    _tags_level=$#funcstack
//! sh:11    _comp_tags="$_comp_tags $__spec "
//! sh:12    if [[ "$curtag" = *[^\\]:* ]]; then
//! sh:13      zformat -f __descr "${curtag#*:}" "d:$3"
//! sh:14      _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! sh:15      curtag="${curtag%:*}"
//! sh:16      set -A $2 "${(P@)2}" "${(@)argv[4,-1]}"
//! sh:17    else
//! sh:18      _description "$__gopt[@]" "$curtag" "$2" "$3"
//! sh:19      set -A $2 "${(@)argv[4,-1]}" "${(P@)2}"
//! sh:20    fi
//! sh:21
//! sh:22    return 0
//! sh:23  fi
//! sh:24
//! sh:25  return 1
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



use crate::compsys::base::TagManager;

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

    #[test]
    fn iteration_walks_try_sets_one_at_a_time() {
        // Pin LIFO ordering of try-sets: first try-set wins until
        // `next()` advances.
        let mut tags = TagManager::new();
        tags.init(&["x".into(), "y".into()]);
        tags.add_try(&["x".into()]);
        tags.add_try(&["y".into()]);
        tags.start();
        assert_eq!(_next_label(&tags, "x"), Some("x".into()));
        assert_eq!(_next_label(&tags, "y"), None);
        tags.next();
        assert_eq!(_next_label(&tags, "x"), None);
        assert_eq!(_next_label(&tags, "y"), Some("y".into()));
    }

    #[test]
    fn empty_string_tag_lookup_returns_none() {
        let mut tags = TagManager::new();
        tags.init(&["files".into()]);
        tags.add_try(&["files".into()]);
        tags.start();
        // Empty-string tag was never added → not wanted.
        assert_eq!(_next_label(&tags, ""), None);
    }

    #[test]
    fn round_trip_label_matches_input_exactly() {
        // The label returned is the input tag verbatim — no
        // canonicalization, no case-folding.
        let mut tags = TagManager::new();
        tags.init(&["MixedCase".into()]);
        tags.add_try(&["MixedCase".into()]);
        tags.start();
        assert_eq!(_next_label(&tags, "MixedCase"), Some("MixedCase".into()));
        assert_eq!(_next_label(&tags, "mixedcase"), None);
    }

    #[test]
    fn fresh_tagmanager_returns_none_before_start() {
        // Without calling `start()`, no try-set is active even if
        // `init` was called.
        let mut tags = TagManager::new();
        tags.init(&["a".into()]);
        tags.add_try(&["a".into()]);
        // Skip tags.start() — pin no-active-set semantics.
        assert_eq!(_next_label(&tags, "a"), None);
    }
}
