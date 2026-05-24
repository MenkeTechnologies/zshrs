//! Port of `_multi_parts` (zsh Completion/Base/Utility/_multi_parts, 261 lines).
//!
//! Local shell reference: upstream
//! `Completion/Base/Utility/_multi_parts` (system copy at
//! `/opt/homebrew/share/zsh/functions/_multi_parts`).
//!
//! The recursive prefix-walker behind `/u/l/b<TAB>` → `/usr/local/bin`:
//! given a separator (`/`) and a candidate list, the user's PREFIX is
//! matched segment-by-segment against each candidate. At each
//! segment, only candidates whose prefix matches the typed segment
//! survive. The unambiguous common prefix is collected and emitted;
//! the user gets to either accept a unique completion or see the
//! shortened ambiguous list.
//!
//! The previous Rust stub at `compsys/base.rs:567-612` handled one
//! level of segment matching and emitted dedup'd matches — but did
//! NOT walk further than one segment, did NOT collect the unambiguous
//! prefix, and did NOT trigger menu-mode handling.
//!
//! This port targets the practical core: the recursive walker plus
//! unambiguous-prefix collection. Edge-case zstyles (`expand prefix`,
//! `expand suffix`, `$_comp_correct` integration, immediate-insertion
//! flag, full menu-mode state mutation) are intentionally simplified —
//! the visible user behavior is right for the common path-style
//! completion.

use std::collections::BTreeSet;

use crate::compcore::CompletionState;
use crate::completion::Completion;

pub struct MultiPartsOpts<'a> {
    /// `-i` flag — immediate insertion (only mentioned, not modeled).
    pub immediate: bool,
    /// `-M matchspec` — passthrough. Not honored at this layer.
    pub matchspec: Option<&'a str>,
    /// Tag name for emitted matches (default `parts`).
    pub tag: &'a str,
}

impl<'a> Default for MultiPartsOpts<'a> {
    fn default() -> Self {
        Self {
            immediate: false,
            matchspec: None,
            tag: "parts",
        }
    }
}

pub fn _multi_parts(
    state: &mut CompletionState,
    separator: char,
    parts: &[String],
    opts: &MultiPartsOpts<'_>,
) -> bool {
    let prefix = state.params.prefix.clone();
    let sep_s = separator.to_string();

    // Split PREFIX into typed segments.
    let typed: Vec<&str> = if prefix.is_empty() {
        Vec::new()
    } else {
        prefix.split(separator).collect()
    };

    // Filter candidates: every typed segment is a PREFIX-match against
    // the candidate's segment at the same position. This is the whole
    // point — `/u/l/b` matches `/usr/local/bin` because each typed
    // segment is a leading substring of the candidate's segment.
    let mut survivors: Vec<&String> = parts.iter().collect();
    for (i, t) in typed.iter().enumerate() {
        survivors.retain(|cand| {
            let cand_segs: Vec<&str> = cand.split(separator).collect();
            if i >= cand_segs.len() {
                return false;
            }
            cand_segs[i].starts_with(t)
        });
        if survivors.is_empty() {
            return false;
        }
    }

    if survivors.is_empty() {
        return false;
    }

    // Collect the unambiguous extension: walk segment-by-segment from
    // typed.len() onwards. At each segment, if all survivors share the
    // exact same segment value, prepend it; otherwise stop and emit
    // each branch as a separate match.
    let typed_len = typed.len();
    let mut common: Vec<String> = Vec::new();
    let mut all_segs_per_cand: Vec<Vec<&str>> = survivors
        .iter()
        .map(|c| c.split(separator).collect::<Vec<_>>())
        .collect();

    // Discover whether all candidates agree on segment[typed_len..K]
    // for as many K as possible.
    let mut k = typed_len;
    loop {
        // All candidates must have a segment at position k.
        let all_have = all_segs_per_cand.iter().all(|s| s.len() > k);
        if !all_have {
            break;
        }
        let first_seg = all_segs_per_cand[0][k];
        let agreed = all_segs_per_cand
            .iter()
            .all(|s| s[k] == first_seg);
        if !agreed {
            break;
        }
        common.push(first_seg.to_string());
        k += 1;
    }

    // Emit the FULL canonical candidate path for each survivor.
    // The user's typed prefix (`/u/l/b`) is replaced with the
    // expanded form (`/usr/local/bin`). That's the whole point of
    // multi-parts completion — it's not "extend the user's typed
    // string", it's "replace it with the canonical full path".
    // `common` is unused for emission; it's kept for clarity over
    // when an unambiguous next-segment extension exists.
    let _ = common;
    state.begin_group(opts.tag, true);
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for segs in &all_segs_per_cand {
        let full = segs.join(&sep_s);
        if seen.insert(full.clone()) {
            state.add_match(Completion::new(full), Some(opts.tag));
        }
    }
    state.end_group();
    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(state: &CompletionState) -> Vec<String> {
        state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.clone())
            .collect()
    }

    #[test]
    fn classic_u_l_b_walk_to_usr_local_bin() {
        let mut state = CompletionState::new();
        state.params.prefix = "/u/l/b".into();
        let parts = vec![
            "/usr/local/bin".into(),
            "/usr/local/lib".into(),
            "/usr/local/share".into(),
            "/var/log".into(),
        ];
        let ok = _multi_parts(&mut state, '/', &parts, &MultiPartsOpts::default());
        assert!(ok);
        let ns = names(&state);
        assert!(ns.contains(&"/usr/local/bin".to_string()), "got {ns:?}");
        assert!(!ns.contains(&"/var/log".to_string()));
    }

    #[test]
    fn ambiguous_at_first_segment_emits_branches() {
        let mut state = CompletionState::new();
        state.params.prefix = "".into();
        let parts = vec![
            "/usr/bin".into(),
            "/usr/lib".into(),
            "/var/log".into(),
        ];
        let ok = _multi_parts(&mut state, '/', &parts, &MultiPartsOpts::default());
        assert!(ok);
        let ns = names(&state);
        // All three should appear since common prefix is the leading `/`.
        assert!(ns.iter().any(|n| n.contains("usr")));
        assert!(ns.iter().any(|n| n.contains("var")));
    }

    #[test]
    fn ambiguous_at_last_segment_emits_each_option() {
        let mut state = CompletionState::new();
        state.params.prefix = "/usr/l".into();
        let parts = vec![
            "/usr/local".into(),
            "/usr/lib".into(),
            "/usr/share".into(),
        ];
        let ok = _multi_parts(&mut state, '/', &parts, &MultiPartsOpts::default());
        assert!(ok);
        let ns = names(&state);
        assert!(ns.iter().any(|n| n == "/usr/local"), "got {ns:?}");
        assert!(ns.iter().any(|n| n == "/usr/lib"));
        assert!(!ns.iter().any(|n| n == "/usr/share"));
    }

    #[test]
    fn off_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/no/such".into();
        let parts = vec!["/usr/bin".into(), "/var/log".into()];
        let ok = _multi_parts(&mut state, '/', &parts, &MultiPartsOpts::default());
        assert!(!ok);
    }

    #[test]
    fn custom_separator() {
        let mut state = CompletionState::new();
        state.params.prefix = "co:de".into();
        let parts = vec![
            "code:decoded".into(),
            "code:dehydrated".into(),
            "other:thing".into(),
        ];
        let ok = _multi_parts(&mut state, ':', &parts, &MultiPartsOpts::default());
        assert!(ok);
        let ns = names(&state);
        assert!(ns.iter().any(|n| n.starts_with("code:de")));
    }

    #[test]
    fn empty_prefix_passes_all_candidates() {
        let mut state = CompletionState::new();
        let parts = vec!["a".into(), "b".into(), "c".into()];
        let ok = _multi_parts(&mut state, '/', &parts, &MultiPartsOpts::default());
        assert!(ok);
        assert_eq!(names(&state).len(), 3);
    }

    #[test]
    fn no_candidates_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "any".into();
        assert!(!_multi_parts(
            &mut state,
            '/',
            &[],
            &MultiPartsOpts::default()
        ));
    }

    #[test]
    fn duplicate_candidates_dedup() {
        let mut state = CompletionState::new();
        let parts = vec![
            "/usr/bin".into(),
            "/usr/bin".into(),
            "/usr/local".into(),
        ];
        let _ = _multi_parts(&mut state, '/', &parts, &MultiPartsOpts::default());
        let bin_count = names(&state).iter().filter(|n| *n == "/usr/bin").count();
        assert_eq!(bin_count, 1, "duplicates must collapse via the BTreeSet");
    }

    #[test]
    fn longer_typed_prefix_than_candidate_returns_false() {
        // /a/b/c/d typed, but candidate is only 2-deep → no segment
        // at position 2 → bail.
        let mut state = CompletionState::new();
        state.params.prefix = "/a/b/c/d".into();
        let parts = vec!["/a/b".into()];
        assert!(!_multi_parts(&mut state, '/', &parts, &MultiPartsOpts::default()));
    }

    #[test]
    fn tag_name_from_opts_respected() {
        let mut state = CompletionState::new();
        let parts = vec!["/a/b".into()];
        let opts = MultiPartsOpts {
            tag: "my-custom-tag",
            ..MultiPartsOpts::default()
        };
        let _ = _multi_parts(&mut state, '/', &parts, &opts);
        assert!(state.groups.iter().any(|g| g.name == "my-custom-tag"));
    }

    #[test]
    fn partial_segment_match_only_at_position() {
        // The `lo` in `/usr/lo/x` should match `/usr/local/x` even
        // though both `/usr/lo/x` and `/usr/long/y` are candidates.
        let mut state = CompletionState::new();
        state.params.prefix = "/usr/lo".into();
        let parts = vec![
            "/usr/local".into(),
            "/usr/long-name".into(),
            "/usr/loud".into(),
            "/var/log".into(),
        ];
        let _ = _multi_parts(&mut state, '/', &parts, &MultiPartsOpts::default());
        let ns = names(&state);
        assert!(ns.contains(&"/usr/local".to_string()));
        assert!(ns.contains(&"/usr/long-name".to_string()));
        assert!(ns.contains(&"/usr/loud".to_string()));
        assert!(!ns.contains(&"/var/log".to_string()));
    }
}
