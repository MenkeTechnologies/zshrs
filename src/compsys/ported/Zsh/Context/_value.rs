//! Port of `_value` from `Completion/Zsh/Context/_value`.
//!
//! Full upstream body (50 lines verbatim):
//! ```text
//! sh: 1  #compdef -value- -array-value- -value-,-default-,-default-
//! sh: 2
//! sh: 3  # You can customize completion for different parameters by writing
//! sh: 4  # functions with the tag-line `#compdef -value-,<name>,<command>' where
//! sh: 5  # <name> is the name of the parameter (or name-key when completing an
//! sh: 6  # associative array value) and <command> is either `-default-' or the
//! sh: 7  # name of the command from the command-line.
//! sh: 8
//! sh: 9  if [[ "$service" != -value-,* ]]; then
//! sh:10    local strs ctx=
//! sh:11
//! sh:12    strs=( -default- )
//! sh:13
//! sh:14    if [[ "$compstate[context]" != *value && -n "$_comp_command1" ]]; then
//! sh:15      ctx="${_comp_command}"
//! sh:16      strs=( "${_comp_command1}" "$strs[@]" )
//! sh:17      [[ -n "$_comp_command2" ]] &&
//! sh:18          strs=( "${_comp_command2}" "$strs[@]" )
//! sh:19    fi
//! sh:20
//! sh:21    _dispatch -value-,${compstate[parameter]},$ctx \
//! sh:22              -value-,{${compstate[parameter]},-default-},${^strs}
//! sh:23  else
//! sh:24    if [[ "$compstate[parameter]" != *-* &&
//! sh:25          "$compstate[context]" = array_value &&
//! sh:26          "${(Pt)${compstate[parameter]}}" = assoc* ]]; then
//! sh:27      local expl
//! sh:28      if (( CURRENT & 1 )); then
//! sh:29        _wanted association-keys expl 'association key' \
//! sh:30            compadd -k "$compstate[parameter]"
//! sh:31      else
//! sh:32        compstate[parameter]="${compstate[parameter]}-${words[CURRENT-1]}"
//! sh:33
//! sh:34        _dispatch -value-,${compstate[parameter]}, \
//! sh:35                  -value-,{${compstate[parameter]},-default-},-default-
//! sh:36      fi
//! sh:37    else
//! sh:38      local pats
//! sh:39
//! sh:40      if { zstyle -a ":completion:${curcontext}:" assign-list pats &&
//! sh:41           [[ "$compstate[parameter]" = (${(j:|:)~pats}) ]] } ||
//! sh:42         [[ "$PREFIX$SUFFIX" = *:* ]]; then
//! sh:43        compset -P '*:'
//! sh:44        compset -S ':*'
//! sh:45        _default -r '\-\n\t /:' "$@"
//! sh:46      else
//! sh:47        _default "$@"
//! sh:48      fi
//! sh:49    fi
//! sh:50  fi
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the `_dispatch` key list
//! from `_comp_command{,1,2}` (populated by upstream `_set_command`
//! call site), then dispatches each `-value-,<param>,<cmd>` key
//! via our ported [`_dispatch`].

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
