//! Port of `_alternative` from `Completion/Base/Utility/_alternative`.
//!
//! Full upstream body (83 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local tags def expl descr action mesgs nm="$compstate[nmatches]" subopts
//! sh: 4  local opt ws curcontext="$curcontext"
//! sh: 5
//! sh: 6  subopts=()
//! sh: 7  while getopts 'O:C:' opt; do
//! sh: 8    case "$opt" in
//! sh: 9    O) subopts=( "${(@P)OPTARG}" ) ;;
//! sh:10    C) curcontext="${curcontext%:*}:$OPTARG" ;;
//! sh:11    esac
//! sh:12  done
//! sh:13
//! sh:14  shift OPTIND-1
//! sh:15
//! sh:16  [[ "$1" = -(|-) ]] && shift
//! sh:17
//! sh:18  mesgs=()
//! sh:19
//! sh:20  _tags "${(@)argv%%:*}"
//! sh:21
//! sh:22  while _tags; do
//! sh:23    for def; do
//! sh:24      if _requested "${def%%:*}"; then
//! sh:25        descr="${${def#*:}%%:*}"
//! sh:26        action="${def#*:*:}"
//! sh:27
//! sh:28        _description "${def%%:*}" expl "$descr"
//! sh:29
//! sh:30        if [[ "$action" = \ # ]]; then
//! sh:31
//! sh:32          # An empty action means that we should just display a message.
//! sh:33
//! sh:34          mesgs=( "$mesgs[@]" "${def%%:*}:$descr")
//! sh:35        elif [[ "$action" = \(\(*\)\) ]]; then
//! sh:36
//! sh:37          # ((...)) contains literal strings with descriptions.
//! sh:38
//! sh:39          eval ws\=\( "${action[3,-3]}" \)
//! sh:40
//! sh:41          _describe -t "${def%%:*}" "$descr" ws -M 'r:|[_-]=* r:|=*' "$subopts[@]"
//! sh:42        elif [[ "$action" = \(*\) ]]; then
//! sh:43
//! sh:44          # Anything inside `(...)' is added directly.
//! sh:45
//! sh:46          eval ws\=\( "${action[2,-2]}" \)
//! sh:47
//! sh:48          _all_labels "${def%%:*}" expl "$descr" \
//! sh:49              compadd "$subopts[@]" -a - ws
//! sh:50        elif [[ "$action" = \{*\} ]]; then
//! sh:51
//! sh:52          # A string in braces is evaluated.
//! sh:53
//! sh:54          while _next_label "${def%%:*}" expl "$descr"; do
//! sh:55            eval "$action[2,-2]"
//! sh:56          done
//! sh:57        elif [[ "$action" = \ * ]]; then
//! sh:58
//! sh:59          # If the action starts with a space, we just call it.
//! sh:60
//! sh:61          eval "action=( $action )"
//! sh:62          while _next_label "${def%%:*}" expl "$descr"; do
//! sh:63            "$action[@]"
//! sh:64          done
//! sh:65        else
//! sh:66
//! sh:67          # Otherwise we call it with the description-arguments built above.
//! sh:68
//! sh:69          eval "action=( $action )"
//! sh:70  	while _next_label "${def%%:*}" expl "$descr"; do
//! sh:71            "$action[1]" "$subopts[@]" "$expl[@]" "${(@)action[2,-1]}"
//! sh:72          done
//! sh:73        fi
//! sh:74      fi
//! sh:75    done
//! sh:76    [[ nm -ne compstate[nmatches] ]] && return 0
//! sh:77  done
//! sh:78
//! sh:79  for descr in "$mesgs[@]"; do
//! sh:80    _message -e "${descr%%:*}" "${descr#*:}"
//! sh:81  done
//! sh:82
//! sh:83  return 1
//! ```
//!
//! Upstream walks `tag:description:action` specs; for each tag in
//! the active set, dispatches the action.
//!
//! Faithful Rust port: parses specs via `Alternative::parse`, calls
//! `TagManager` to drive the iteration (matching shell's
//! `while _tags`), and invokes the caller-supplied `action_handler`
//! once per requested alternative.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
