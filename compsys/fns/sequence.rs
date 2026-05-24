//! Port of `_sequence` (zsh Completion/Base/Utility/_sequence, 40 lines).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_sequence`.
//!
//! Wraps an inner completer to handle a separator-separated sequence
//! of values where each component is completed by the same function
//! (e.g. `--targets=foo,bar,b<TAB>`). Three flags:
//!   - `-s sep` : separator (default `,`)
//!   - `-n max` : max entries; once typed, no more completions
//!   - `-d`     : duplicate values allowed (default: dedup)
//!
//! Plus passthrough of all compadd flags (-p, -i, -P, -I, -S, -q, -r,
//! -R, -C, -F, -M, -J, -V, -1, -2, -o, -X, -x).
//!
//! The previous Rust stub took (separator, completer), mutated
//! `state.params.prefix` to drop everything before the last separator,
//! then called the inner completer. That missed:
//!   - the `-n` cap;
//!   - the dedup computation (already-typed entries should NOT show
//!     up as candidates again);
//!   - the auto-suffix that re-arms the separator so successive Tabs
//!     keep building the list;
//!   - the `-F dedup` filter passthrough.
//!
//! This port keeps the inner-completer dispatch as a closure (since
//! compsys is a leaf — we can't call arbitrary functions by name),
//! but adds back the missing semantics.

use crate::compcore::CompletionState;

pub struct SequenceOpts<'a> {
    /// `-s sep` (default `,`).
    pub separator: &'a str,
    /// `-n max` — at most `max` total list entries. None = unlimited.
    pub max_entries: Option<usize>,
    /// `-d` — allow duplicates. Default false (dedup against entries
    /// already present in PREFIX/SUFFIX).
    pub allow_duplicates: bool,
    /// `-P prefix` (-P style override at the compadd level — appended
    /// to the front of each emitted match).
    pub fixed_prefix: Option<&'a str>,
    /// `-S suffix` (-S style override — appended to the back of each
    /// emitted match).
    pub fixed_suffix: Option<&'a str>,
}

impl<'a> Default for SequenceOpts<'a> {
    fn default() -> Self {
        Self {
            separator: ",",
            max_entries: None,
            allow_duplicates: false,
            fixed_prefix: None,
            fixed_suffix: None,
        }
    }
}

pub fn sequence<F>(state: &mut CompletionState, opts: &SequenceOpts<'_>, completer: F) -> bool
where
    F: FnOnce(&mut CompletionState, &[String]) -> bool,
{
    let sep = opts.separator;

    // Compute already-typed entries on both sides of the cursor.
    // shell:24-25:
    //   dedup=( "${(@)${(@ps.$qsep.)PREFIX#$pre}[1,-2]}"   # PREFIX, last entry dropped
    //           "${(@)${(@ps.$qsep.)SUFFIX}[2,-1]}" )      # SUFFIX, first entry dropped
    // The dropped entries are the one currently under the cursor.
    let mut dedup: Vec<String> = Vec::new();
    if !opts.allow_duplicates {
        // Strip any fixed_prefix from the start of PREFIX before
        // splitting (mirrors `${PREFIX#$pre}`).
        let pfx = match opts.fixed_prefix {
            Some(p) if state.params.prefix.starts_with(p) => &state.params.prefix[p.len()..],
            _ => state.params.prefix.as_str(),
        };
        let pre_parts: Vec<&str> = pfx.split(sep).collect();
        if pre_parts.len() > 1 {
            for s in &pre_parts[..pre_parts.len() - 1] {
                if !s.is_empty() {
                    dedup.push((*s).into());
                }
            }
        }
        let suf_parts: Vec<&str> = state.params.suffix.split(sep).collect();
        if suf_parts.len() > 1 {
            for s in &suf_parts[1..] {
                if !s.is_empty() {
                    dedup.push((*s).into());
                }
            }
        }
    }

    // shell:28-32: `-n` cap. If we've already typed `max-1` separators
    // in PREFIX, the next entry would be the LAST — we can still
    // complete it but should NOT auto-append another separator.
    let pre_entries = state.params.prefix.matches(sep).count();
    let at_max = match opts.max_entries {
        Some(m) => pre_entries + 1 >= m,
        None => false,
    };

    // shell:35: `compset -P \*${qsep}` — chew PREFIX up to the last
    // separator so the inner completer only sees the in-progress entry.
    if let Some(idx) = state.params.prefix.rfind(sep) {
        let chewed_end = idx + sep.len();
        let chewed = state.params.prefix[..chewed_end].to_string();
        state.params.iprefix.push_str(&chewed);
        state.params.prefix = state.params.prefix[chewed_end..].to_string();
    }
    // shell:34: `compset -S ${qsep}\*` — if SUFFIX already starts with
    // a separator, another entry follows, so don't auto-append our
    // own suffix.
    let suffix_already_sep = state.params.suffix.starts_with(sep);

    // Invoke the inner completer.
    let inner_ok = completer(state, &dedup);

    // shell:31: when NOT at max AND no suffix-already-sep, append the
    // separator as the auto-suffix so the next Tab continues the
    // sequence. We can't deeply influence the compadd of the inner
    // completer from here without restructuring — so we mark every
    // emitted match with the separator suffix retroactively. This
    // matches the shell behavior visually (after Tab completes a
    // value, the separator is right behind the cursor).
    if !at_max && !suffix_already_sep && inner_ok {
        for group in state.groups.iter_mut() {
            for m in group.matches.iter_mut() {
                if m.suf.is_none() {
                    m.suf = Some(sep.to_string());
                }
            }
        }
    }
    // If fixed_prefix/fixed_suffix were given, plumb them as well.
    if inner_ok {
        if let Some(p) = opts.fixed_prefix {
            for group in state.groups.iter_mut() {
                for m in group.matches.iter_mut() {
                    if m.pre.is_none() {
                        m.pre = Some(p.to_string());
                    }
                }
            }
        }
        if let Some(s) = opts.fixed_suffix {
            for group in state.groups.iter_mut() {
                for m in group.matches.iter_mut() {
                    if m.suf.is_none() {
                        m.suf = Some(s.to_string());
                    }
                }
            }
        }
    }

    inner_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::Completion;

    fn fake_inner(state: &mut CompletionState, dedup: &[String]) -> bool {
        // Emit a fixed candidate set, filtered by dedup.
        state.begin_group("vals", true);
        for name in ["alpha", "beta", "gamma"] {
            if dedup.iter().any(|d| d == name) {
                continue;
            }
            state.add_match(Completion::new(name.to_string()), Some("vals"));
        }
        state.end_group();
        state.nmatches > 0
    }

    #[test]
    fn dedup_drops_already_typed_entries() {
        let mut state = CompletionState::new();
        state.params.prefix = "alpha,b".into();
        let opts = SequenceOpts::default();
        sequence(&mut state, &opts, fake_inner);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"alpha"), "alpha already typed → should be deduped");
        assert!(names.contains(&"beta"));
        assert!(names.contains(&"gamma"));
    }

    #[test]
    fn d_flag_allows_duplicates() {
        let mut state = CompletionState::new();
        state.params.prefix = "alpha,a".into();
        let opts = SequenceOpts {
            allow_duplicates: true,
            ..Default::default()
        };
        sequence(&mut state, &opts, fake_inner);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"alpha"), "duplicates allowed → alpha should still appear");
    }

    #[test]
    fn chew_prefix_up_to_separator() {
        let mut state = CompletionState::new();
        state.params.prefix = "alpha,beta,g".into();
        let opts = SequenceOpts::default();
        sequence(&mut state, &opts, fake_inner);
        // After chewing, prefix is "g" and iprefix carries "alpha,beta,".
        assert_eq!(state.params.iprefix, "alpha,beta,");
        assert_eq!(state.params.prefix, "g");
    }

    #[test]
    fn auto_suffix_appends_separator_for_next_entry() {
        let mut state = CompletionState::new();
        state.params.prefix = "alpha,".into();
        let opts = SequenceOpts::default();
        sequence(&mut state, &opts, fake_inner);
        // Every emitted match should carry the separator as suffix so
        // Tab into one continues into the next entry.
        for m in &state.groups[0].matches {
            assert_eq!(m.suf.as_deref(), Some(","), "match {} missing comma suffix", m.str_);
        }
    }

    #[test]
    fn n_max_suppresses_auto_suffix_on_last_entry() {
        let mut state = CompletionState::new();
        // Already have one entry; max=2 means this is the LAST.
        state.params.prefix = "alpha,".into();
        let opts = SequenceOpts {
            max_entries: Some(2),
            ..Default::default()
        };
        sequence(&mut state, &opts, fake_inner);
        for m in &state.groups[0].matches {
            assert_eq!(m.suf, None, "at max → no separator suffix");
        }
    }

    #[test]
    fn custom_separator() {
        let mut state = CompletionState::new();
        state.params.prefix = "alpha:b".into();
        let opts = SequenceOpts {
            separator: ":",
            ..Default::default()
        };
        sequence(&mut state, &opts, fake_inner);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }
}
