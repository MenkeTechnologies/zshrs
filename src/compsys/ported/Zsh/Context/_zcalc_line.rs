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

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
