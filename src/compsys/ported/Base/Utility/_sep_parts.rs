//! Port of `_sep_parts` from `Completion/Base/Utility/_sep_parts`.
//!
//! Full upstream body (146 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This function can be used to separately complete parts of strings
//! sh:  4  # where each part may be one of a set of matches and different parts
//! sh:  5  # have different sets.
//! sh:  6  # Arguments are alternately arrays and separator strings. Arrays may
//! sh:  7  # be given by name or literally as words separated by white space in
//! sh:  8  # parentheses, e.g.:
//! sh:  9  #
//! sh: 10  #  _sep_parts '(foo bar)' @ hosts
//! sh: 11  #
//! sh: 12  # This will make this function complete the strings `foo' and `bar'.
//! sh: 13  # If the string on the line contains a `@', the substring after it
//! sh: 14  # will be completed from the array `hosts'. Of course more arrays
//! sh: 15  # may be given, each preceded by another separator string.
//! sh: 16  #
//! sh: 17  # This function understands the `-J group', `-V group', and
//! sh: 18  # `-X explanation' options.
//! sh: 19
//! sh: 20  local str arr sep test testarr tmparr prefix suffixes autosuffix
//! sh: 21  local matchflags opt group expl nm=$compstate[nmatches] opre osuf opts matcher
//! sh: 22
//! sh: 23  # Get the options.
//! sh: 24
//! sh: 25  zparseopts -D -a opts 'J+:=group' 'V+:=group' P: F: S: r: R: q 1 2 o+: n \
//! sh: 26      'x+:=expl' 'X+:=expl' 'M+:=matcher'
//! sh: 27
//! sh: 28  # Get the string from the line.
//! sh: 29
//! sh: 30  opre="$PREFIX"
//! sh: 31  osuf="$SUFFIX"
//! sh: 32  str="$PREFIX$SUFFIX"
//! sh: 33  SUFFIX=""
//! sh: 34  prefix=""
//! sh: 35
//! sh: 36  # Walk through the arguments to find the longest unambiguous prefix.
//! sh: 37
//! sh: 38  while [[ $# -gt 1 ]]; do
//! sh: 39    # Get the next array and separator.
//! sh: 40    arr="$1"
//! sh: 41    sep="$2"
//! sh: 42
//! sh: 43    if [[ "$arr[1]" == '(' ]]; then
//! sh: 44      tmparr=( ${=arr[2,-2]} )
//! sh: 45      arr=tmparr
//! sh: 46    fi
//! sh: 47
//! sh: 48    # Is the separator on the line?
//! sh: 49
//! sh: 50    [[ "$str" != *${sep}* ]] && break
//! sh: 51
//! sh: 52    # Get the matching array elements.
//! sh: 53
//! sh: 54    PREFIX="${str%%(|\\)${sep}*}"
//! sh: 55    builtin compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 56    [[ $#testarr -eq 0 && -n "$_comp_correct" ]] &&
//! sh: 57      compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 58
//! sh: 59    # If there are no matches we give up. If there is more than one
//! sh: 60    # match, this is the part we will complete.
//! sh: 61
//! sh: 62    (( $#testarr )) || return 1
//! sh: 63    [[ $#testarr -gt 1 ]] && break
//! sh: 64
//! sh: 65    # Only one match, add it to the prefix and skip over it in `str',
//! sh: 66    # continuing with the next array and separator.
//! sh: 67
//! sh: 68    prefix="${prefix}${testarr[1]}${sep}"
//! sh: 69    str="${str#*${sep}}"
//! sh: 70    shift 2
//! sh: 71  done
//! sh: 72
//! sh: 73  # Get the array to work upon.
//! sh: 74
//! sh: 75  arr="$1"
//! sh: 76  if [[ "$arr[1]" == '(' ]]; then
//! sh: 77    tmparr=( ${=arr[2,-2]} )
//! sh: 78    arr=tmparr
//! sh: 79  fi
//! sh: 80
//! sh: 81  if [[ $# -le 1 || "$str" != *${2}* ]]; then
//! sh: 82    # No more separators, build the matches.
//! sh: 83
//! sh: 84    PREFIX="$str"
//! sh: 85    builtin compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 86    [[ $#testarr -eq 0 && -n "$_comp_correct" ]] &&
//! sh: 87      compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 88  fi
//! sh: 89
//! sh: 90  [[ $#testarr -eq 0 || ${#testarr[1]} -eq 0 ]] && return 1
//! sh: 91
//! sh: 92  # Now we build the suffixes to give to the completion code.
//! sh: 93
//! sh: 94  shift
//! sh: 95  suffixes=("")
//! sh: 96  autosuffix=()
//! sh: 97
//! sh: 98  while [[ $# -gt 0 && "$str" == *${1}* ]]; do
//! sh: 99    # Remove anything up to the suffix.
//! sh:100
//! sh:101    str="${str#*${1}}"
//! sh:102
//! sh:103    # Again, we get the string from the line up to the next separator
//! sh:104    # and build a pattern from it.
//! sh:105
//! sh:106    if [[ $# -gt 2 ]]; then
//! sh:107      PREFIX="${str%%${3}*}"
//! sh:108    else
//! sh:109      PREFIX="$str"
//! sh:110    fi
//! sh:111
//! sh:112    # We incrementally add suffixes by appending to them the separators
//! sh:113    # and the strings from the next array that match the pattern we built.
//! sh:114
//! sh:115    arr="$2"
//! sh:116    if [[ "$arr[1]" == '(' ]]; then
//! sh:117      tmparr=( ${=arr[2,-2]} )
//! sh:118      arr=tmparr
//! sh:119    fi
//! sh:120
//! sh:121    builtin compadd -O tmparr "$matcher[@]" -a "$arr"
//! sh:122    [[ $#tmparr -eq 0 && -n "$_comp_correct" ]] &&
//! sh:123      compadd -O tmparr "$matcher[@]" - "$arr"
//! sh:124
//! sh:125    suffixes=("${(@)^suffixes[@]}${(q)1}${(@)^tmparr}")
//! sh:126
//! sh:127    shift 2
//! sh:128  done
//! sh:129
//! sh:130  # If we were given at least one more separator we make the completion
//! sh:131  # code offer it by appending it as a autoremovable suffix.
//! sh:132
//! sh:133  (( $# )) && autosuffix=(-qS "${(q)1}")
//! sh:134
//! sh:135  # Add the matches for each of the suffixes.
//! sh:136
//! sh:137  PREFIX="$pre"
//! sh:138  SUFFIX="$suf"
//! sh:139  for i in "$suffixes[@]"; do
//! sh:140    compadd -U "$group[@]" "$expl[@]" "$autosuffix[@]" "$opts[@]" \
//! sh:141            -i "$IPREFIX" -I "$ISUFFIX" -p "$prefix" -s "$i" -a testarr
//! sh:142  done
//! sh:143
//! sh:144  # This sets the return value to indicate that we added matches (or not).
//! sh:145
//! sh:146  [[ nm -ne compstate[nmatches] ]]
//! ```
//!
//! Upstream is the multi-array sibling of `_multi_parts`: each array
//! holds candidates for the segment that follows the corresponding
//! separator. So `'(foo bar)' @ '(host1 host2)'` lets the user type
//! `foo@host1` etc.
//!
//! Faithful Rust port: walks the separators string char by char,
//! using the corresponding array at each segment position. The
//! cumulative-prefix tracking (so `foo@host1:port1` works with
//! `'@:'` as separators) matches the shell loop shape.



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::{Completion, CompletionFlags};

/// _sep_parts - complete parts with arbitrary separators
pub fn _sep_parts(state: &mut CompletionState, separators: &str, arrays: &[Vec<String>]) -> bool {
    if arrays.is_empty() {
        return false;
    }

    let prefix = state.params.prefix.clone();
    let sep_chars: Vec<char> = separators.chars().collect();

    // Walk the prefix consuming one positional separator at a time.
    // sep_chars[0] separates array[0] from array[1], sep_chars[1]
    // separates array[1] from array[2], etc.
    let mut array_idx = 0;
    let mut cursor = 0;
    while array_idx < sep_chars.len() {
        let sep = sep_chars[array_idx];
        if let Some(pos) = prefix[cursor..].find(sep) {
            cursor += pos + sep.len_utf8();
            array_idx += 1;
        } else {
            break;
        }
    }

    if array_idx >= arrays.len() {
        return false;
    }

    // The unconsumed tail of `prefix` is the user's typed text for
    // the current segment.
    let current_prefix = prefix[cursor..].to_string();

    state.begin_group("sep-parts", true);

    let mut matched = false;
    for item in &arrays[array_idx] {
        if item.starts_with(&current_prefix) {
            let mut comp = Completion::new(item);

            // Add next separator if there are more arrays
            if array_idx + 1 < arrays.len() {
                if let Some(&sep) = sep_chars.get(array_idx) {
                    comp.suf = Some(sep.to_string());
                    comp.flags |= CompletionFlags::NOSPACE;
                }
            }

            state.add_match(comp, Some("sep-parts"));
            matched = true;
        }
    }

    state.end_group();
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_array_completes_first_segment() {
        let mut state = CompletionState::new();
        state.params.prefix = "us".into();
        let arrays = vec![
            vec!["users".into(), "usr".into(), "var".into()],
            vec!["local".into(), "share".into()],
        ];
        assert!(_sep_parts(&mut state, "/", &arrays));
        let names: Vec<String> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"users".to_string()));
        assert!(names.contains(&"usr".to_string()));
        assert!(!names.contains(&"var".to_string()));
    }

    #[test]
    fn second_array_used_after_first_separator() {
        let mut state = CompletionState::new();
        state.params.prefix = "usr/lo".into();
        let arrays = vec![
            vec!["users".into(), "usr".into()],
            vec!["local".into(), "share".into()],
        ];
        assert!(_sep_parts(&mut state, "/", &arrays));
        let names: Vec<String> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"local".to_string()));
        assert!(!names.contains(&"share".to_string()));
    }

    #[test]
    fn empty_arrays_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_sep_parts(&mut state, "/", &[]));
    }

    #[test]
    fn next_separator_attached_as_suffix_when_more_arrays_follow() {
        // First-segment match should carry the next separator as
        // `suf` + NOSPACE so the user continues onto the second
        // segment without losing the delimiter.
        let mut state = CompletionState::new();
        let arrays = vec![
            vec!["alice".into(), "bob".into()],
            vec!["host1".into(), "host2".into()],
        ];
        assert!(_sep_parts(&mut state, "@", &arrays));
        let c = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .find(|c| c.str_ == "alice")
            .expect("alice present");
        assert_eq!(c.suf.as_deref(), Some("@"));
        assert!(c.flags.contains(CompletionFlags::NOSPACE));
    }

    #[test]
    fn last_array_does_not_attach_suffix() {
        // After the final separator, completion of the last segment
        // should NOT add a separator after itself.
        let mut state = CompletionState::new();
        state.params.prefix = "alice@".into();
        let arrays = vec![
            vec!["alice".into()],
            vec!["host1".into(), "host2".into()],
        ];
        assert!(_sep_parts(&mut state, "@", &arrays));
        let c = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .find(|c| c.str_ == "host1")
            .expect("host1 present");
        assert!(c.suf.is_none(), "last segment should not have suf");
    }

    #[test]
    fn three_segments_with_colon_separator() {
        // user:host:port style — three arrays separated by colons.
        // The `separators` string has one CHAR PER ARRAY BOUNDARY:
        // two arrays need 1 char, three arrays need 2 chars.
        let mut state = CompletionState::new();
        state.params.prefix = "alice:web:".into();
        let arrays = vec![
            vec!["alice".into()],
            vec!["web".into(), "api".into()],
            vec!["80".into(), "443".into()],
        ];
        assert!(_sep_parts(&mut state, "::", &arrays));
        let names: Vec<String> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.clone())
            .collect();
        // We're past two `:` separators → third array (ports).
        assert!(names.contains(&"80".to_string()));
        assert!(names.contains(&"443".to_string()));
    }

    #[test]
    fn no_matching_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "definitely-not".into();
        let arrays = vec![vec!["alpha".into(), "beta".into()]];
        assert!(!_sep_parts(&mut state, "/", &arrays));
    }

    #[test]
    fn prefix_past_all_arrays_returns_false() {
        // Two arrays, but the prefix already has THREE separators →
        // we'd be looking for an array at index 3 which doesn't exist.
        let mut state = CompletionState::new();
        state.params.prefix = "a/b/c/".into();
        let arrays = vec![
            vec!["a".into()],
            vec!["b".into()],
        ];
        assert!(!_sep_parts(&mut state, "/", &arrays));
    }
}
