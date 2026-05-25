//! Port of `_parameters` from `Completion/Zsh/Type/_parameters`.
//!
//! Full upstream body (58 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This should be used to complete parameter names if you need some of the
//! sh: 4  # extra options of compadd. It completes only non-local parameters.
//! sh: 5
//! sh: 6  # If you specify a -g option with a pattern, the pattern will be used to
//! sh: 7  # restrict the type of parameters matched.
//! sh: 8
//! sh: 9  local i pfilt
//! sh:10  local -i nm=$compstate[nmatches]
//! sh:11  local -a expl pattern=( -g \* ) normal described verbose faked fakes tmp
//! sh:12
//! sh:13  # parameter names that match the pattern $pfilt are removed
//! sh:14  zstyle -t ":completion:${curcontext}:parameters" prefix-needed &&
//! sh:15      [[ $PREFIX != [_.]* ]] &&
//! sh:16          pfilt='[_.]*'
//! sh:17  # names containing a dot are not allowed after '$'
//! sh:18  [[ $IPREFIX = *\$ ]] && pfilt+='|*.*'
//! sh:19
//! sh:20  _description parameters expl parameter
//! sh:21  zparseopts -D -K -E g:=pattern
//! sh:22
//! sh:23  if zstyle -t ":completion:${curcontext}:parameters" extra-verbose; then
//! sh:24    described=(
//! sh:25        ${(k)parameters[(R)$~pattern[2]~*(hideval|local|special)*]:#$~pfilt}
//! sh:26    )
//! sh:27    compadd "$@" "$expl[@]" -D described -a - described
//! sh:28    if (( $#described )); then
//! sh:29      # Normally, calling typeset without flags would print the values of its
//! sh:30      # arguments. However, inside a function, it instead declare its arguments
//! sh:31      # as local variables and outputs nothing. Thus, to force it print out
//! sh:32      # parameter values, we pass it the -m flag.
//! sh:33      verbose=(
//! sh:34          ${${${(f@)"$( typeset -m ${(@b)described} )"}/=/:}[@]//'\'/'\\'}
//! sh:35      )
//! sh:36      _describe -t parameters parameter verbose "$@" "$expl[@]"
//! sh:37    fi
//! sh:38
//! sh:39    normal=(
//! sh:40        ${(k)parameters[(R)$~pattern[2]~^(*(hideval|special)*)~*local*]:#$~pfilt}
//! sh:41    )
//! sh:42  else
//! sh:43    normal=( ${(k)parameters[(R)${~pattern[2]}~*local*]:#$~pfilt} )
//! sh:44  fi
//! sh:45
//! sh:46  if zstyle -a ":completion:${curcontext}:" fake-parameters tmp; then
//! sh:47    for i in "$tmp[@]"; do
//! sh:48      if [[ "$i" = *:* ]]; then
//! sh:49        faked=( "$faked[@]" "$i" )
//! sh:50      else
//! sh:51        fakes=( "$fakes[@]" "$i" )
//! sh:52      fi
//! sh:53    done
//! sh:54  fi
//! sh:55  compadd "$@" "$expl[@]" - "$normal[@]" "${(@)fakes:|described}" \
//! sh:56      "${(@)${(@)${(@M)faked:#${~pattern[2]}}%%:*}:|described}"
//! sh:57
//! sh:58  (( compstate[nmatches] > nm ))
//! ```
//!
//! Upstream pulls names from `${(k)parameters}` (built-in assoc
//! array mapping name→type) with optional `-g pattern` type filter
//! on the value side.
//!
//! Faithful Rust port: takes a `&HashMap<String, String>` from the
//! caller (caller pulls live names from runtime paramtab) and emits
//! names prefix-filtered. NEW: honors `type_filter: Option<&str>`
//! for the `-g pattern` behavior — when set, only emit names
//! whose type matches the glob.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
