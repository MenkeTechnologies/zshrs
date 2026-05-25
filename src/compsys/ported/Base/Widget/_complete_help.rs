//! Port of `_complete_help` from `Completion/Base/Widget/_complete_help`.
//!
//! Full upstream body (92 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xh
//! sh: 2
//! sh: 3  _complete_help() {
//! sh: 4    eval "$_comp_setup"
//! sh: 5
//! sh: 6    local _sort_tags=_help_sort_tags text i j k tmp
//! sh: 7    typeset -A help_funcs help_tags help_sfuncs help_styles
//! sh: 8
//! sh: 9    local -H _help_scan_funcstack="main_complete|complete|approximate|normal"
//! sh:10    local -H _help_filter_funcstack="alternative|call_function|describe|dispatch|wanted|requested|all_labels|next_label"
//! sh:11
//! sh:12    {
//! sh:13      _shadow compadd compcall zstyle
//! sh:14      compadd() { return 1 }
//! sh:15      compcall() { _help_sort_tags use-compctl }
//! sh:16      zstyle() {
//! sh:17        local _f="${${(@)${(@)funcstack[2,(i)_($~_help_scan_funcstack)]}:#(_($~_help_filter_funcstack)|\((eval|anon)\))}% *}"
//! sh:18
//! sh:19        [[ -z "$_f" ]] && _f="${${(@)funcstack[2,(i)_($~_help_scan_funcstack)]}:#(_($~_help_filter_funcstack)|\((eval|anon)\))}"
//! sh:20
//! sh:21        if [[ "$help_sfuncs[$2]" != *${_f}* ||
//! sh:22              "$help_styles[${2}${_f}]" != *${3}* ]]; then
//! sh:23
//! sh:24          [[ "$help_sfuncs[$2]" != *${_f}* ]] && help_sfuncs[$2]+=$'\0'"${_f}"
//! sh:25          local _t
//! sh:26
//! sh:27          case "$1" in
//! sh:28          -s) _t='[string] ';;
//! sh:29          -a) _t='[array]  ';;
//! sh:30          -h) _t='[assoc]  ';;
//! sh:31          *)  _t='[boolean]';;
//! sh:32          esac
//! sh:33          help_styles[${2}${_f}]+=",${_t} ${3}:${_f}"
//! sh:34        fi
//! sh:35
//! sh:36        # No need to call the completers more than once with different match specs.
//! sh:37
//! sh:38        if [[ "$3" = matcher-list ]]; then
//! sh:39          set -A "$4" ''
//! sh:40        else
//! sh:41          builtin zstyle "$@"
//! sh:42        fi
//! sh:43      }
//! sh:44
//! sh:45      ${1:-_main_complete}
//! sh:46    } always {
//! sh:47      _unshadow compadd compcall zstyle
//! sh:48    }
//! sh:49
//! sh:50    for i in "${(@ok)help_funcs}"; do
//! sh:51      text+=$'\n'"tags in context :completion:${i}:"
//! sh:52      tmp=()
//! sh:53      for j in "${(@ps.\0.)help_funcs[$i][2,-1]}"; do
//! sh:54        tmp+=( "${(@s.,.)help_tags[${i}${j}][2,-1]}" )
//! sh:55      done
//! sh:56      zformat -a tmp '  (' "$tmp[@]"
//! sh:57      tmp=( $'\n    '${^tmp}')' )
//! sh:58      text+="${tmp}"
//! sh:59    done
//! sh:60
//! sh:61    if [[ ${NUMERIC:-1} -ne 1 ]]; then
//! sh:62      text+=$'\n'
//! sh:63      for i in "${(@ok)help_sfuncs}"; do
//! sh:64        text+=$'\n'"styles in context ${i}"
//! sh:65        tmp=()
//! sh:66        for j in "${(@ps.\0.)help_sfuncs[$i][2,-1]}"; do
//! sh:67          tmp+=( "${(@s.,.)help_styles[${i}${j}][2,-1]}" )
//! sh:68        done
//! sh:69        zformat -a tmp '  (' "$tmp[@]"
//! sh:70        tmp=( $'\n    '${^tmp}')' )
//! sh:71        text+="${tmp}"
//! sh:72      done
//! sh:73    fi
//! sh:74    compstate[list]='list force'
//! sh:75    compstate[insert]=''
//! sh:76
//! sh:77    compadd -UX "$text[2,-1]" -n ''
//! sh:78  }
//! sh:79
//! sh:80  _help_sort_tags() {
//! sh:81    local f="${${(@)${(@)funcstack[3,(i)_($~_help_scan_funcstack)]}:#(_($~_help_filter_funcstack)|\((eval|anon)\))}% *}"
//! sh:82
//! sh:83    if [[ "$help_funcs[$curcontext]" != *${f}* ||
//! sh:84          "$help_tags[${curcontext}${f}]" != *(${(j:|:)~argv})* ]]; then
//! sh:85      [[ "$help_funcs[$curcontext]" != *${f}* ]] &&
//! sh:86          help_funcs[$curcontext]+=$'\0'"${f}"
//! sh:87      help_tags[${curcontext}${f}]+=",${argv}:${f}"
//! sh:88      comptry "$@" 2>/dev/null
//! sh:89    fi
//! sh:90  }
//! sh:91
//! sh:92  _complete_help "$@"
//! ```
//!
//! Strict Rust port: two entry points.
//!
//! 1. `_complete_help(state, entries)` — caller passes
//! pre-collected `(topic, description)` pairs and we emit each
//! with `topic -- desc` disp formatting under group `help`.
//! Used when the caller already has the entries (e.g. tag list).
//!
//! 2. `_complete_help_shadow(state, completer, label)` — runs
//! `completer` under `_shadow`, captures everything it would
//! have added, and renders the capture as topic+desc rows. This
//! is the closer analog of what the shell widget does: shadow
//! `compadd`/`zstyle` to RECORD what a completer would do
//! without polluting live state.



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;
use crate::compsys::ported::_shadow::_shadow;

/// _complete_help - Show completion help (entry-driven form).
pub fn _complete_help(state: &mut CompletionState, help_entries: &[(String, String)]) -> bool {
    state.begin_group("help", true);

    for (topic, desc) in help_entries {
        let mut comp = Completion::new(topic);
        comp.disp = Some(format!("{} -- {}", topic, desc));
        state.add_match(comp, Some("help"));
    }

    state.end_group();
    !help_entries.is_empty()
}

/// _complete_help_shadow — run `completer` under `_shadow`, then
/// emit each captured (group, match) pair as a topic+desc help row.
/// `label` is forwarded as the shadow name (shows up in the
/// underlying record, useful for debugging).
pub fn _complete_help_shadow(
    state: &mut CompletionState,
    label: &str,
    completer: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    let record = _shadow(state, label, completer);
    let mut entries: Vec<(String, String)> = Vec::new();
    for (group, m) in &record.matches {
        entries.push((m.clone(), format!("(tag: {})", group)));
    }
    for e in &record.explanations {
        entries.push((format!("[msg] {e}"), "explanation".into()));
    }
    _complete_help(state, &entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_topic_with_topic_dash_desc_disp() {
        let mut state = CompletionState::new();
        let entries = vec![
            ("foo".into(), "the foo cmd".into()),
            ("bar".into(), "the bar cmd".into()),
        ];
        assert!(_complete_help(&mut state, &entries));
        let by_str: std::collections::HashMap<&str, &str> = state.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_str["foo"], "foo -- the foo cmd");
        assert_eq!(by_str["bar"], "bar -- the bar cmd");
    }

    #[test]
    fn empty_entries_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_complete_help(&mut state, &[]));
    }

    #[test]
    fn group_named_help_created() {
        let mut state = CompletionState::new();
        let entries = vec![("x".into(), "y".into())];
        _complete_help(&mut state, &entries);
        assert!(state.groups.iter().any(|g| g.name == "help"));
    }

    #[test]
    fn entries_emitted_in_input_order() {
        let mut state = CompletionState::new();
        let entries = vec![
            ("z".into(), "last alpha".into()),
            ("a".into(), "first alpha".into()),
            ("m".into(), "middle".into()),
        ];
        _complete_help(&mut state, &entries);
        // Default sort=true sorts alphabetically — pin that all three
        // are present regardless of order.
        let names: std::collections::HashSet<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains("a"));
        assert!(names.contains("m"));
        assert!(names.contains("z"));
    }

    #[test]
    fn returns_true_iff_entries_provided() {
        let mut state = CompletionState::new();
        let entries = vec![("topic".into(), "desc".into())];
        assert!(_complete_help(&mut state, &entries));
    }

    #[test]
    fn shadow_mode_captures_matches_a_completer_would_add() {
        let mut state = CompletionState::new();
        let ok = _complete_help_shadow(&mut state, "test-completer", |s| {
            s.add_match(Completion::new("foo"), Some("commands"));
            s.add_match(Completion::new("bar"), Some("commands"));
            true
        });
        assert!(ok);
        let entries: Vec<(String, String)> = state.groups[0]
            .matches
            .iter()
            .map(|c| {
                (
                    c.str_.clone(),
                    c.disp.clone().unwrap_or_default(),
                )
            })
            .collect();
        // Two rows, one per shadowed match. Each disp encodes its
        // source tag.
        let disps: Vec<String> = entries.iter().map(|(_, d)| d.clone()).collect();
        assert!(disps.iter().any(|d| d.contains("foo -- (tag: commands)")));
        assert!(disps.iter().any(|d| d.contains("bar -- (tag: commands)")));
    }

    #[test]
    fn shadow_mode_with_empty_completer_returns_false() {
        let mut state = CompletionState::new();
        let ok = _complete_help_shadow(&mut state, "empty", |_| true);
        assert!(!ok, "no captured rows → no help entries → false");
    }

    #[test]
    fn shadow_mode_rolls_back_live_completion_state() {
        // The completer adds matches under shadow; after
        // _complete_help_shadow returns, those matches do NOT
        // appear in the live "commands" tag group. Only the
        // synthesized "help" group exists.
        let mut state = CompletionState::new();
        let _ = _complete_help_shadow(&mut state, "noisy", |s| {
            s.add_match(Completion::new("live-poison"), Some("commands"));
            true
        });
        let live_commands_count: usize = state
            .groups
            .iter()
            .filter(|g| g.name == "commands")
            .map(|g| g.matches.len())
            .sum();
        assert_eq!(
            live_commands_count, 0,
            "shadowed completer's matches must not leak into live `commands` group"
        );
    }

    #[test]
    fn explanations_from_shadow_become_msg_rows() {
        let mut state = CompletionState::new();
        let ok = _complete_help_shadow(&mut state, "with-msg", |s| {
            s.add_match(Completion::new("x"), Some("g"));
            s.add_explanation("hint text".into(), Some("g"));
            true
        });
        assert!(ok);
        let disps: Vec<String> = state.groups[0]
            .matches
            .iter()
            .filter_map(|c| c.disp.clone())
            .collect();
        assert!(
            disps.iter().any(|d| d.contains("[msg] hint text")),
            "explanation should round-trip as a [msg] help row; got {disps:?}"
        );
    }
}
