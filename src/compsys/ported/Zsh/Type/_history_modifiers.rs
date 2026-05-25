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

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
