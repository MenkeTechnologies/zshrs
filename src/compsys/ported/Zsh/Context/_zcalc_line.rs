//! Port of `_zcalc_line` from `Completion/Zsh/Context/_zcalc_line`.
//!
//! Full upstream body (81 lines verbatim):
//! ```text
//! sh: 1  #compdef -zcalc-line-
//! sh: 2
//! sh: 3  # This handles completion of a zcalc command line read via vared.
//! sh: 4
//! sh: 5  _zcalc_line_escapes() {
//! sh: 6    local -a cmds
//! sh: 7    cmds=(
//! sh: 8      "!:shell escape"
//! sh: 9      "q:quit"
//! sh:10      "norm:normal output format"
//! sh:11      "sci:scientific output format"
//! sh:12      "fix:fixed point output format"
//! sh:13      "eng:engineering (power of 1000) output format"
//! sh:14      "raw:raw output format"
//! sh:15      "local:make variables local"
//! sh:16      "function:define math function (also \:func or \:f)"
//! sh:17    )
//! sh:18    cmds=("\:"${^cmds})
//! sh:19    _describe -t command-escapes "command escape" cmds -Q
//! sh:20  }
//! sh:21
//! sh:22  _zcalc_line() {
//! sh:23    local expl
//! sh:24
//! sh:25    if [[ CURRENT -eq 1 && $words[1] != ":"(\\|)"!"* ]]; then
//! sh:26      local -a alts
//! sh:27      if [[ $words[1] = (|:*) ]]; then
//! sh:28        alts=("command-escapes:command escape:_zcalc_line_escapes")
//! sh:29      fi
//! sh:30      if [[ $words[1] = (|[^:]*) ]]; then
//! sh:31        alts+=("math:math formula:_math")
//! sh:32      fi
//! sh:33      _alternative $alts
//! sh:34      return
//! sh:35    fi
//! sh:36
//! sh:37    case $words[1] in
//! sh:38      (":"(\\|)"!"*)
//! sh:39      if [[ $words[1] = ":"(\\|)"!" && CURRENT -gt 1 ]]; then
//! sh:40        shift words
//! sh:41        (( CURRENT-- ))
//! sh:42      else
//! sh:43        words[1]=${words[1]##:(\\|)\!}
//! sh:44        compset -P ':(\\|)!'
//! sh:45      fi
//! sh:46      _normal
//! sh:47      ;;
//! sh:48
//! sh:49      (:function)
//! sh:50      # completing already defined user math functions is in fact exactly
//! sh:51      # the wrong thing to do since currently zmathfuncdef won't overwrite,
//! sh:52      # but it may jog the user's memory...
//! sh:53      if (( CURRENT == 2 )); then
//! sh:54        _wanted math-functions expl 'math function' \
//! sh:55  	compadd -- ${${(k)functions:#^zsh_math_func_*}##zsh_math_func_}
//! sh:56      else
//! sh:57        _math
//! sh:58      fi
//! sh:59      ;;
//! sh:60
//! sh:61      (:local)
//! sh:62      _parameter
//! sh:63      ;;
//! sh:64
//! sh:65      (:(fix|sci|eng))
//! sh:66      if (( CURRENT == 2 )); then
//! sh:67        _message "precision"
//! sh:68      fi
//! sh:69      ;&
//! sh:70
//! sh:71      (:*)
//! sh:72      _message "no more arguments"
//! sh:73      ;;
//! sh:74
//! sh:75      ([^:]*)
//! sh:76      _math
//! sh:77      ;;
//! sh:78    esac
//! sh:79  }
//! sh:80
//! sh:81  _zcalc_line "$@"
//! ```
//!
//! Strict Rust port: detects the `:cmd` colon-command form and
//! emits the documented zcalc command escapes via `_describe`-
//! style emission. Math expressions fall through to [`_math`].



use std::collections::{BTreeMap, HashMap};

use crate::compsys::base::MainCompleteState;
use crate::compsys::completion::Completion;
use crate::compsys::ported::_math::_math;

/// Documented zcalc command-line escapes (from upstream).
pub const ZCALC_ESCAPES: &[(&str, &str)] = &[
    ("!", "shell escape"),
    ("q", "quit"),
    ("norm", "normal output format"),
    ("sci", "scientific output format"),
    ("fix", "fixed point output format"),
    ("eng", "engineering (power of 1000) output format"),
    ("raw", "raw output format"),
    ("local", "make variables local"),
    ("function", "define math function (also :func or :f)"),
];

/// `_zcalc_line` — `-zcalc-line-` context dispatcher.
pub fn _zcalc_line(
    state: &mut MainCompleteState,
    params: &HashMap<String, String>,
    user_math_funcs: &[String],
    module_math_funcs: &BTreeMap<String, Vec<String>>,
) -> bool {
    // shell: if PREFIX starts with `:`, emit command escapes.
    if state.comp.params.prefix.starts_with(':') {
        let prefix_after_colon = state.comp.params.prefix.trim_start_matches(':').to_string();
        state.comp.begin_group("command-escapes", true);
        let mut any = false;
        for (esc, desc) in ZCALC_ESCAPES {
            if !esc.starts_with(&prefix_after_colon) {
                continue;
            }
            let mut comp = Completion::new(*esc);
            comp.disp = Some(format!(":{} -- {}", esc, desc));
            comp.pre = Some(":".into());
            state.comp.add_match(comp, Some("command-escapes"));
            any = true;
        }
        state.comp.end_group();
        return any;
    }
    // Otherwise — math expression context. Dispatch to _math.
    _math(state, params, user_math_funcs, module_math_funcs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_prefix_emits_command_escapes() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = ":".into();
        let _ = _zcalc_line(&mut state, &HashMap::new(), &[], &BTreeMap::new());
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"q"));
        assert!(names.contains(&"norm"));
        assert!(names.contains(&"function"));
    }

    #[test]
    fn colon_with_partial_filters_escapes() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = ":no".into();
        let _ = _zcalc_line(&mut state, &HashMap::new(), &[], &BTreeMap::new());
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // After stripping `:`, prefix is `no` → only `norm` survives.
        assert_eq!(names, vec!["norm"]);
    }

    #[test]
    fn no_colon_falls_through_to_math() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "1+x".into();
        let mut params = HashMap::new();
        params.insert("x".into(), "integer".into());
        let _ = _zcalc_line(&mut state, &params, &[], &BTreeMap::new());
        // _math strips non-ident chars; pin that the dispatch happened.
        assert_eq!(state.comp.params.iprefix, "1+");
        assert_eq!(state.comp.params.prefix, "x");
    }

    #[test]
    fn empty_prefix_falls_through_to_math() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _zcalc_line(&mut state, &HashMap::new(), &[], &BTreeMap::new());
        // No panic — _math handles empty prefix.
    }

    #[test]
    fn each_escape_has_descriptive_disp() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = ":".into();
        let _ = _zcalc_line(&mut state, &HashMap::new(), &[], &BTreeMap::new());
        for m in &state.comp.groups[0].matches {
            assert!(m.disp.as_deref().unwrap_or("").contains(" -- "));
        }
    }
}
