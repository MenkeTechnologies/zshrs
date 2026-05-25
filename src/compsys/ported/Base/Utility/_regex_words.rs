//! Port of `_regex_words` from `Completion/Base/Utility/_regex_words`.
//!
//! Full upstream body (52 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local opt OPTARG matches end
//! sh: 4  local term=$'\0'
//! sh: 5
//! sh: 6  while getopts "t:" opt; do
//! sh: 7    case $opt in
//! sh: 8      (t)
//! sh: 9      term=$OPTARG
//! sh:10      ;;
//! sh:11
//! sh:12      (*)
//! sh:13      return 1
//! sh:14      ;;
//! sh:15    esac
//! sh:16  done
//! sh:17  shift $(( OPTIND - 1 ))
//! sh:18
//! sh:19  local tag=$1
//! sh:20  local desc=$2
//! sh:21  shift 2
//! sh:22
//! sh:23  if (( $# )); then
//! sh:24    reply=(\()
//! sh:25  else
//! sh:26    # ### Is this likely to happen in callers?  Should we warn?
//! sh:27    reply=()
//! sh:28    return
//! sh:29  fi
//! sh:30
//! sh:31  integer i
//! sh:32  local -a wds
//! sh:33
//! sh:34  if [[ $term = $'\0' ]]; then
//! sh:35    matches=":${tag}:${desc}:(( "
//! sh:36    end="))"
//! sh:37  else
//! sh:38    matches=":${tag}:${desc}:_values -s ${(q)term} ${(q)desc}"
//! sh:39  fi
//! sh:40
//! sh:41  for (( i = 1; i <= $#; i++ )); do
//! sh:42    wds=(${(s.:.)argv[i]})
//! sh:43    reply+=(/${wds[1]//\**/"[^$term]#"}"$term"/)
//! sh:44    if [[ $term = $'\0' ]]; then
//! sh:45      matches+="${wds[1]//\*}${wds[2]:+\\:${wds[2]//(#m)[: \(\)]/\\$MATCH}} "
//! sh:46    else
//! sh:47      matches+=" ${(q)${${wds[1]//\*}//(#m)[:\[\]]/\\$MATCH}}\\[${(q)${wds[2]//(#m)[:\[\]]/\\$MATCH}}\\]"
//! sh:48    fi
//! sh:49    eval "reply+=($wds[3])"
//! sh:50    reply+=(\|)
//! sh:51  done
//! sh:52  reply+=( /'[]'/ "${matches}${end}" \) )
//! ```
//!
//! Strict Rust port: takes `(word, description, action)` triples.
//! The action is a Rust fn registered under a name (mirrors shell's
//! `action` arg position, which can be a shell expression or
//! `_action_name`). When the user selects a matching word AT
//! completion time, the registered action fires via
//! `_call_function`. Emission semantics: prefix-filter each word,
//! attach `word -- description` disp, return true iff any survived.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
