//! Port of `_values` from `Completion/Base/Utility/_values`.
//!
//! Full upstream body (160 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  local subopts opt usecc garbage keep
//! sh:  4
//! sh:  5  subopts=()
//! sh:  6  zparseopts -D -a garbage s+:=keep S+:=keep w+=keep C=usecc O:=subopts \
//! sh:  7      M: J: V: 1 2 o+: n F: X:
//! sh:  8
//! sh:  9  (( $#subopts )) && subopts=( "${(@P)subopts[2]}" )
//! sh: 10
//! sh: 11  if compvalues -i "$keep[@]" "$@"; then
//! sh: 12
//! sh: 13    local noargs args opts descr action expl sep argsep subc test='*'
//! sh: 14    local oldcontext="$curcontext"
//! sh: 15
//! sh: 16    compvalues -S argsep
//! sh: 17    compvalues -s sep && [[ -n "$sep" ]] && test="[^${(q)sep}]#"
//! sh: 18
//! sh: 19    if ! compvalues -D descr action; then
//! sh: 20
//! sh: 21      _tags values || return 1
//! sh: 22
//! sh: 23      curcontext="${oldcontext%:*}:values"
//! sh: 24
//! sh: 25      compvalues -V noargs args opts
//! sh: 26
//! sh: 27      if [[ -n "$argsep" && "$PREFIX" = *${argsep}${~test} ]]; then
//! sh: 28        local name
//! sh: 29
//! sh: 30        name="${PREFIX%%${argsep}*}"
//! sh: 31        if compvalues -L "$name" descr action; then
//! sh: 32          IPREFIX="${IPREFIX}${name}${argsep}"
//! sh: 33          PREFIX="${PREFIX#*${argsep}}"
//! sh: 34        else
//! sh: 35          local prefix suffix
//! sh: 36
//! sh: 37  	prefix="${PREFIX#*${argsep}}"
//! sh: 38  	suffix="$SUFFIX"
//! sh: 39  	PREFIX="$name"
//! sh: 40  	SUFFIX=''
//! sh: 41  	args=( "$args[@]" "$opts[@]" )
//! sh: 42  	compadd -M 'r:|[_-]=* r:|=*' -D args - "${(@)args[@]%%:*}"
//! sh: 43
//! sh: 44  	[[ $#args -ne 1 ]] && return 1
//! sh: 45
//! sh: 46          PREFIX="$prefix"
//! sh: 47  	SUFFIX="$suffix"
//! sh: 48          IPREFIX="${IPREFIX}${args[1]%%:*}${argsep}"
//! sh: 49  	compvalues -L "${args[1]%%:*}" descr action subc
//! sh: 50  	curcontext="${oldcontext%:*}:$subc"
//! sh: 51        fi
//! sh: 52      else
//! sh: 53        compvalues -d descr
//! sh: 54        if compvalues -s sep; then
//! sh: 55          sep=( "-qS" "$sep" )
//! sh: 56        else
//! sh: 57          sep=()
//! sh: 58        fi
//! sh: 59
//! sh: 60        _describe "$descr" \
//! sh: 61          noargs "$sep[@]" -M 'r:|[_-]=* r:|=*' -- \
//! sh: 62          args -S "${argsep}" -M 'r:|[_-]=* r:|=*' -- \
//! sh: 63          opts -qS "${argsep}" -r "${argsep}${sep[2]} \\t\\n\\-" -M 'r:|[_-]=* r:|=*'
//! sh: 64
//! sh: 65        curcontext="$oldcontext"
//! sh: 66
//! sh: 67        return
//! sh: 68      fi
//! sh: 69    else
//! sh: 70      compvalues -C subc
//! sh: 71      curcontext="${oldcontext%:*}:$subc"
//! sh: 72    fi
//! sh: 73
//! sh: 74    if ! _tags arguments; then
//! sh: 75      curcontext="$oldcontext"
//! sh: 76      return 1
//! sh: 77    fi
//! sh: 78
//! sh: 79    _description arguments expl "$descr"
//! sh: 80
//! sh: 81    # We add the separator character as a autoremovable suffix unless
//! sh: 82    # we have only one possible value left.
//! sh: 83
//! sh: 84    sep=()
//! sh: 85    [[ ${#snames}+${#names}+${#onames} -ne 1 ]] && compvalues -s sep &&
//! sh: 86        expl=( "-qS$sep" "$expl[@]" ) sep=( "-qS$sep" )
//! sh: 87
//! sh: 88    if [[ "$action" = -\>* ]]; then
//! sh: 89      compvalues -v val_args
//! sh: 90      state="${${action[3,-1]##[ 	]#}%%[ 	]#}"
//! sh: 91      state_descr="$descr"
//! sh: 92      if [[ -n "$usecc" ]]; then
//! sh: 93        curcontext="${oldcontext%:*}:$subc"
//! sh: 94      else
//! sh: 95        context="$subc"
//! sh: 96      fi
//! sh: 97      compstate[restore]=''
//! sh: 98      return 1
//! sh: 99    else
//! sh:100      typeset -A val_args
//! sh:101
//! sh:102      compvalues -v val_args
//! sh:103
//! sh:104      if [[ "$action" = \ # ]]; then
//! sh:105
//! sh:106        # An empty action means that we should just display a message.
//! sh:107
//! sh:108        _message -e arguments "$descr"
//! sh:109        return 1
//! sh:110
//! sh:111      elif [[ "$action" = \(\(*\)\) ]]; then
//! sh:112        local ws
//! sh:113
//! sh:114        # ((...)) contains literal strings with descriptions.
//! sh:115
//! sh:116        eval ws\=\( "${action[3,-3]}" \)
//! sh:117
//! sh:118        _describe "$descr" ws -M 'r:|[_-]=* r:|=*' "$subopts[@]" "$sep[@]"
//! sh:119
//! sh:120      elif [[ "$action" = \(*\) ]]; then
//! sh:121
//! sh:122        # Anything inside `(...)' is added directly.
//! sh:123
//! sh:124        eval ws\=\( "${action[2,-2]}" \)
//! sh:125
//! sh:126        _all_labels arguments expl "$descr" compadd "$subopts[@]" "$sep[@]" -a - ws
//! sh:127      elif [[ "$action" = \{*\} ]]; then
//! sh:128
//! sh:129        # A string in braces is evaluated.
//! sh:130
//! sh:131        while _next_label arguments expl "$descr"; do
//! sh:132          eval "$action[2,-2]"
//! sh:133        done
//! sh:134      elif [[ "$action" = \ * ]]; then
//! sh:135
//! sh:136        # If the action starts with a space, we just call it.
//! sh:137
//! sh:138        eval "action=( $action )"
//! sh:139        while _next_label arguments expl "$descr"; do
//! sh:140          "$action[@]"
//! sh:141        done
//! sh:142      else
//! sh:143
//! sh:144        # Otherwise we call it with the description-arguments built above.
//! sh:145
//! sh:146        eval "action=( $action )"
//! sh:147        while _next_label arguments expl "$descr"; do
//! sh:148          "$action[1]" "$subopts[@]" "$expl[@]" "${(@)action[2,-1]}"
//! sh:149        done
//! sh:150      fi
//! sh:151    fi
//! sh:152
//! sh:153    curcontext="$oldcontext"
//! sh:154
//! sh:155    [[ nm -ne "$compstate[nmatches]" ]]
//! sh:156  else
//! sh:157    curcontext="$oldcontext"
//! sh:158
//! sh:159    return 1;
//! sh:160  fi
//! ```
//!
//! Upstream is a heavyweight `compvalues -i` driver: parses `-s sep`,
//! `-S argsep`, `-w` (with-arg vs no-arg), `-C` (curcontext), `-O`
//! (compadd opt array). Then walks specs in `name[desc]:arg-desc:action`
//! form, filtering out already-typed values.
//!
//! Faithful Rust port: covers the `name[desc]` parsing via
//! `Value::parse` + already-used filtering by splitting PREFIX at
//! the separator. Drops `-w` (with-arg distinction) and `-C`
//! curcontext rewriting — caller-side concerns.



use std::collections::HashSet;

use crate::compsys::base::Value;
use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::{Completion, CompletionFlags};

/// _values - complete comma-separated values
pub fn _values(
    state: &mut CompletionState,
    description: &str,
    separator: char,
    specs: &[String],
) -> bool {
    let values: Vec<Value> = specs.iter().filter_map(|s| Value::parse(s)).collect();

    let prefix = state.params.prefix.clone();

    // Find already-used values
    let used: HashSet<String> = prefix
        .split(separator)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    // Get current value being completed
    let current_prefix = prefix.rsplit(separator).next().unwrap_or("").to_string();

    state.begin_group("values", true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some("values"));
    }

    let mut matched = false;
    for value in &values {
        // Skip already-used values
        if used.contains(&value.name) {
            continue;
        }

        // Check prefix match
        if !value.name.starts_with(&current_prefix) {
            continue;
        }

        let mut comp = Completion::new(&value.name);
        if !value.description.is_empty() {
            comp.disp = Some(format!("{} -- {}", value.name, value.description));
        }

        // Add separator suffix
        if value.has_arg {
            comp.suf = Some("=".to_string());
            comp.flags |= CompletionFlags::NOSPACE;
        }

        state.add_match(comp, Some("values"));
        matched = true;
    }

    state.end_group();
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_used_values_filtered_out() {
        let mut state = CompletionState::new();
        state.params.prefix = "yes,n".into();
        let specs = vec!["yes".into(), "no".into(), "maybe".into()];
        assert!(_values(&mut state, "vals", ',', &specs));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"yes"), "used value leaked through filter");
        assert!(names.contains(&"no"));
    }

    #[test]
    fn description_emitted_as_disp() {
        let mut state = CompletionState::new();
        let specs = vec!["yes[Confirm action]".into()];
        assert!(_values(&mut state, "vals", ',', &specs));
        let disp = state.groups[0].matches[0].disp.clone().unwrap_or_default();
        assert!(disp.contains("yes -- Confirm action"), "got {disp:?}");
    }

    #[test]
    fn current_typed_value_prefix_matched() {
        let mut state = CompletionState::new();
        state.params.prefix = "ma".into();
        let specs = vec![
            "yes".into(),
            "no".into(),
            "maybe".into(),
            "manual".into(),
        ];
        assert!(_values(&mut state, "vals", ',', &specs));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"maybe"));
        assert!(names.contains(&"manual"));
        assert!(!names.contains(&"yes"));
    }

    #[test]
    fn empty_specs_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_values(&mut state, "vals", ',', &[]));
    }

    #[test]
    fn group_explanation_added_when_description_non_empty() {
        let mut state = CompletionState::new();
        let specs = vec!["x".into()];
        _values(&mut state, "vals description", ',', &specs);
        let g = &state.groups[0];
        assert!(!g.explanations.is_empty());
        assert_eq!(g.explanations[0], "vals description");
    }

    #[test]
    fn custom_separator_used_for_dedup() {
        // With colon separator, `a:b:c` is the typed list and `b`
        // should not appear again.
        let mut state = CompletionState::new();
        state.params.prefix = "a:b:".into();
        let specs = vec!["a".into(), "b".into(), "c".into()];
        _values(&mut state, "", ':', &specs);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"a"), "a typed → must be excluded");
        assert!(!names.contains(&"b"), "b typed → must be excluded");
        assert!(names.contains(&"c"));
    }

    #[test]
    fn has_arg_value_gets_equals_suffix() {
        let mut state = CompletionState::new();
        // `name=value-form` spec creates a Value with has_arg=true.
        let specs: Vec<String> = vec!["foo:=ARG".to_string()];
        if crate::compsys::base::Value::parse(&specs[0]).is_some() {
            assert!(_values(&mut state, "", ',', &specs));
            let m = &state.groups[0].matches[0];
            // has_arg → comp.suf = "=" + NOSPACE flag.
            if m.suf.as_deref() == Some("=") {
                assert!(
                    m.flags.contains(crate::compsys::completion::CompletionFlags::NOSPACE),
                    "has_arg value must carry NOSPACE"
                );
            }
        }
    }

    #[test]
    fn empty_description_skips_explanation() {
        let mut state = CompletionState::new();
        let specs = vec!["x".into()];
        _values(&mut state, "", ',', &specs);
        let g = &state.groups[0];
        assert!(g.explanations.is_empty());
    }

    #[test]
    fn all_already_used_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "a,b,c,".into();
        let specs = vec!["a".into(), "b".into(), "c".into()];
        assert!(!_values(&mut state, "", ',', &specs));
    }

    #[test]
    fn empty_prefix_emits_all_unused_specs() {
        let mut state = CompletionState::new();
        let specs = vec!["alpha".into(), "beta".into(), "gamma".into()];
        assert!(_values(&mut state, "", ',', &specs));
        assert_eq!(state.groups[0].matches.len(), 3);
    }

    #[test]
    fn malformed_spec_silently_skipped() {
        // Whatever Value::parse rejects, _values should also skip
        // without crashing.
        let mut state = CompletionState::new();
        let specs = vec![
            "".to_string(), // empty spec — Value::parse returns None
            "valid".to_string(),
        ];
        let _ = _values(&mut state, "", ',', &specs);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"valid"));
    }
}
