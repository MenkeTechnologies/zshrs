//! Port of `_sequence` from `Completion/Base/Utility/_sequence`.
//!
//! Full upstream body (40 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # a separated list where each component of the list uses the same
//! sh: 4  # function.
//! sh: 5
//! sh: 6  # -n num : number of items in list [default is unlimited]
//! sh: 7  # -s sep : specify separator [defaults to comma]
//! sh: 8  # -d     : duplicate values allowed
//! sh: 9
//! sh:10  local curcontext="$curcontext" nm="$compstate[nmatches]" pre qsep nosep minus
//! sh:11  local -a opts sep num pref suf cont end uniq dedup garbage
//! sh:12
//! sh:13  zparseopts -D -a opts s:=sep n:=num p:=pref i:=pref P:=pref I:=suf S:=suf \
//! sh:14      q=suf r:=suf R:=suf C:=cont F:=garbage d=uniq M+: J+: V+: 1 2 o+: X+: x+:
//! sh:15  (( $#cont )) && curcontext="${curcontext%:*}:$cont[2]"
//! sh:16  (( $#sep )) || sep[2]=,
//! sh:17
//! sh:18  if (( $+suf[(r)-S] )); then
//! sh:19    end="${(q)suf[suf[(i)-S]+1]}"
//! sh:20    (( $#end )) && compset -S ${end}\* && suf=() && nosep=1
//! sh:21  fi
//! sh:22
//! sh:23  qsep="${sep[2]}"
//! sh:24  compquote -p qsep
//! sh:25  if (( ! $#uniq )); then
//! sh:26    (( $+pref[(r)-P] )) && pre="${(q)pref[pref[(i)-P]+1]}"
//! sh:27    dedup=( "${(@)${(@ps.$qsep.)PREFIX#$pre}[1,-2]}" "${(@)${(@ps.$qsep.)SUFFIX}[2,-1]}" )
//! sh:28    [[ -n $compstate[quoting] ]] || dedup=( ${(Q)dedup} )
//! sh:29  fi
//! sh:30
//! sh:31  if (( $#num )) && compset -P $(( num[2] - 1 )) \*${(q)qsep}; then
//! sh:32    pref=()
//! sh:33  else
//! sh:34    (( ! nosep && (!$#num || num[2] > 1) )) && suf=( -S ${qsep} -r "$end[1]${(q)qsep[1]} \t\n\-" )
//! sh:35    compset -S ${(q)qsep}\* && suf=()
//! sh:36    compset -P \*${(q)qsep} && pref=()
//! sh:37  fi
//! sh:38
//! sh:39  (( minus = argv[(ib:2:)-] ))
//! sh:40  "${(@)argv[1,minus-1]}" "$opts[@]" -F dedup "$pref[@]" "$suf[@]" "${(@)argv[minus+1,-1]}"
//! ```



use crate::compsys::compcore::CompletionState;

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

pub fn _sequence<F>(state: &mut CompletionState, opts: &SequenceOpts<'_>, completer: F) -> bool
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
    use crate::compsys::completion::Completion;

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
        _sequence(&mut state, &opts, fake_inner);
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
        _sequence(&mut state, &opts, fake_inner);
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
        _sequence(&mut state, &opts, fake_inner);
        // After chewing, prefix is "g" and iprefix carries "alpha,beta,".
        assert_eq!(state.params.iprefix, "alpha,beta,");
        assert_eq!(state.params.prefix, "g");
    }

    #[test]
    fn auto_suffix_appends_separator_for_next_entry() {
        let mut state = CompletionState::new();
        state.params.prefix = "alpha,".into();
        let opts = SequenceOpts::default();
        _sequence(&mut state, &opts, fake_inner);
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
        _sequence(&mut state, &opts, fake_inner);
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
        _sequence(&mut state, &opts, fake_inner);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn fixed_prefix_set_on_emitted_matches() {
        let mut state = CompletionState::new();
        let opts = SequenceOpts {
            fixed_prefix: Some("PFX_"),
            ..Default::default()
        };
        _sequence(&mut state, &opts, fake_inner);
        for m in &state.groups[0].matches {
            assert_eq!(m.pre.as_deref(), Some("PFX_"));
        }
    }

    #[test]
    fn fixed_suffix_set_on_emitted_matches_when_no_sep_auto() {
        // fixed_suffix only fires when there's no auto-separator
        // suffix (no entries yet, max=1 so we're already at the LAST
        // entry — no auto-sep). Then fixed_suffix shines through.
        let mut state = CompletionState::new();
        let opts = SequenceOpts {
            fixed_suffix: Some("__END"),
            max_entries: Some(1),
            ..Default::default()
        };
        _sequence(&mut state, &opts, fake_inner);
        for m in &state.groups[0].matches {
            assert_eq!(m.suf.as_deref(), Some("__END"));
        }
    }

    #[test]
    fn empty_prefix_no_dedup_no_chew() {
        let mut state = CompletionState::new();
        let opts = SequenceOpts::default();
        _sequence(&mut state, &opts, fake_inner);
        // All three candidates should appear.
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names.len(), 3);
        // iprefix unchanged.
        assert_eq!(state.params.iprefix, "");
    }

    #[test]
    fn dedup_with_multiple_existing_entries() {
        let mut state = CompletionState::new();
        state.params.prefix = "alpha,beta,g".into();
        let opts = SequenceOpts::default();
        _sequence(&mut state, &opts, fake_inner);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // alpha + beta already typed → only gamma survives.
        assert_eq!(names, vec!["gamma"]);
    }

    #[test]
    fn suffix_already_separator_blocks_auto_suffix() {
        // SUFFIX="rest,end" already starts with sep → don't add a
        // second sep after the match.
        let mut state = CompletionState::new();
        state.params.prefix = "".into();
        state.params.suffix = ",rest".into();
        let opts = SequenceOpts::default();
        _sequence(&mut state, &opts, fake_inner);
        for m in &state.groups[0].matches {
            assert!(
                m.suf.is_none(),
                "suffix already starts with sep → no auto-sep; got `{:?}`",
                m.suf
            );
        }
    }

    #[test]
    fn dedup_uses_both_prefix_and_suffix_sides() {
        // PREFIX=alpha,X SUFFIX=Y,gamma — both `alpha` (from PREFIX)
        // and `gamma` (from SUFFIX) are already chosen.
        let mut state = CompletionState::new();
        state.params.prefix = "alpha,".into();
        state.params.suffix = ",gamma".into();
        let opts = SequenceOpts::default();
        _sequence(&mut state, &opts, fake_inner);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"alpha"));
        assert!(!names.contains(&"gamma"));
        assert!(names.contains(&"beta"));
    }
}
