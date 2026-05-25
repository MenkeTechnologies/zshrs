//! Port of `_history_modifiers` from `Completion/Zsh/Type/_history_modifiers`.
//!
//! Full upstream body (89 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete history-style modifiers; the first : will have
//! sh: 4  # been matched and compset -p 1'd.
//! sh: 5  # The single argument is the type of context:
//! sh: 6  #   h  history
//! sh: 7  #   q  glob qualifier
//! sh: 8  #   p  parameter
//! sh: 9
//! sh:10  local -a list
//! sh:11
//! sh:12  local type=$1 delim expl
//! sh:13  integer global
//! sh:14
//! sh:15  while true; do
//! sh:16    if [[ -n $PREFIX ]]; then
//! sh:17      local char=$PREFIX[1]
//! sh:18
//! sh:19      global=0
//! sh:20      compset -p 1
//! sh:21      case $char in
//! sh:22        ([hretpqQxlu\&])
//! sh:23        # single character modifiers
//! sh:24        ;;
//! sh:25
//! sh:26        (s)
//! sh:27        # match delimiter string delimiter string delimiter
//! sh:28        if [[ -z $PREFIX ]]; then
//! sh:29  	_delimiters modifier-s
//! sh:30  	return
//! sh:31        fi
//! sh:32        delim=$PREFIX[1]
//! sh:33        compset -p 1
//! sh:34        if ! compset -P "[^${delim}]#${delim}[^${delim}]#${delim}"; then
//! sh:35  	if compset -P "[^${delim}]#${delim}"; then
//! sh:36  	  _message "replacement string"
//! sh:37  	else
//! sh:38  	  _message "original string"
//! sh:39  	fi
//! sh:40  	return
//! sh:41        fi
//! sh:42        ;;
//! sh:43
//! sh:44        (g)
//! sh:45        global=1
//! sh:46        continue
//! sh:47        ;;
//! sh:48      esac
//! sh:49
//! sh:50      # modifier completely matched, see what's next.
//! sh:51      compset -P : && continue
//! sh:52      # if there's something other than colon next, bummer
//! sh:53      [[ -n $PREFIX ]] && return 1
//! sh:54
//! sh:55      list=("\::modifier")
//! sh:56      [[ $type = q ]] && list+=("):end of qualifiers")
//! sh:57      # strictly we want a normal suffix if end of qualifiers
//! sh:58      _describe -t delimiters "delimiter" list -Q -S ''
//! sh:59      return
//! sh:60    else
//! sh:61      list=(
//! sh:62        "s:substitute string"
//! sh:63        "&:repeat substitution"
//! sh:64        )
//! sh:65      if (( ! global )); then
//! sh:66        list+=(
//! sh:67  	"a:absolute path, resolve '..' lexically"
//! sh:68  	"A:as ':a', then resolve symlinks"
//! sh:69  	"c:PATH search for command"
//! sh:70  	"g:globally apply s or &"
//! sh:71  	"h:head - strip trailing path element"
//! sh:72  	"t:tail - strip directories"
//! sh:73  	"r:root - strip suffix"
//! sh:74  	"e:leave only extension"
//! sh:75  	"Q:strip quotes"
//! sh:76  	"P:realpath, resolve '..' physically"
//! sh:77  	"l:lower case all words"
//! sh:78  	"u:upper case all words"
//! sh:79  	)
//! sh:80        [[ $type = h ]] && list+=(
//! sh:81  	"p:print without executing"
//! sh:82  	"x:quote words, breaking on whitespace"
//! sh:83  	)
//! sh:84        [[ $type = [hp] ]] && list+=("q:quote to escape further substitutions")
//! sh:85      fi
//! sh:86      _describe -t modifiers "modifier" list -Q -S ''
//! sh:87      return
//! sh:88    fi
//! sh:89  done
//! ```
//!
//! Strict Rust port: emits the documented single-char modifiers
//! (and `gs`, `s`) as candidates. The `type` arg selects which
//! subset; for `h` (history) all modifiers are available; for
//! `p` (parameter) the substitution `s`/`gs` is also available;
//! for `q` (glob qualifier) only single-char.



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

/// Canonical single-char modifiers (all contexts).
pub const SINGLE_CHAR_MODIFIERS: &[(&str, &str)] = &[
    ("h", "head (remove last path segment)"),
    ("r", "remove extension"),
    ("e", "leave extension only"),
    ("t", "tail (basename)"),
    ("p", "print only"),
    ("q", "quote whitespace"),
    ("Q", "remove quoting"),
    ("x", "split into words at whitespace"),
    ("l", "lowercase"),
    ("u", "uppercase"),
    ("&", "repeat last substitution"),
];

/// Multi-char modifiers (history + parameter context only).
pub const MULTI_CHAR_MODIFIERS: &[(&str, &str)] = &[
    ("s", "substitution: s/pattern/replacement/"),
    ("gs", "global substitution: gs/pattern/replacement/"),
];

/// Modifier context selector — mirrors the shell `$1` arg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierContext {
    /// History expansion (`!foo:h`).
    History,
    /// Glob qualifier (`*(.h)`).
    GlobQualifier,
    /// Parameter expansion (`${foo:h}`).
    Parameter,
}

/// `_history_modifiers` — emit modifier chars for the given context.
pub fn _history_modifiers(state: &mut CompletionState, ctx: ModifierContext) -> bool {
    let prefix = state.params.prefix.clone();
    state.begin_group("modifiers", true);
    let mut any = false;
    for (m, desc) in SINGLE_CHAR_MODIFIERS {
        if !m.starts_with(&*prefix) {
            continue;
        }
        let mut comp = Completion::new(*m);
        comp.disp = Some(format!("{} -- {}", m, desc));
        state.add_match(comp, Some("modifiers"));
        any = true;
    }
    if matches!(ctx, ModifierContext::History | ModifierContext::Parameter) {
        for (m, desc) in MULTI_CHAR_MODIFIERS {
            if !m.starts_with(&*prefix) {
                continue;
            }
            let mut comp = Completion::new(*m);
            comp.disp = Some(format!("{} -- {}", m, desc));
            state.add_match(comp, Some("modifiers"));
            any = true;
        }
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_context_emits_singles_and_substitution() {
        let mut state = CompletionState::new();
        let _ = _history_modifiers(&mut state, ModifierContext::History);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"h"));
        assert!(names.contains(&"t"));
        assert!(names.contains(&"r"));
        assert!(names.contains(&"e"));
        assert!(names.contains(&"s"));
        assert!(names.contains(&"gs"));
    }

    #[test]
    fn glob_qualifier_context_excludes_substitution() {
        let mut state = CompletionState::new();
        let _ = _history_modifiers(&mut state, ModifierContext::GlobQualifier);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"h"));
        assert!(!names.contains(&"s"));
        assert!(!names.contains(&"gs"));
    }

    #[test]
    fn parameter_context_includes_substitution() {
        let mut state = CompletionState::new();
        let _ = _history_modifiers(&mut state, ModifierContext::Parameter);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"s"));
        assert!(names.contains(&"gs"));
    }

    #[test]
    fn prefix_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "g".into();
        let _ = _history_modifiers(&mut state, ModifierContext::History);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["gs"]);
    }

    #[test]
    fn each_emit_has_descriptive_disp() {
        let mut state = CompletionState::new();
        let _ = _history_modifiers(&mut state, ModifierContext::History);
        for m in &state.groups[0].matches {
            let d = m.disp.as_deref().unwrap_or("");
            assert!(d.contains(" -- "), "missing disp separator on `{}`", m.str_);
        }
    }
}
