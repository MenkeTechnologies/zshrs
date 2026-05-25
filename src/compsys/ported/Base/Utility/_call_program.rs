//! Port of `_call_program` from `Completion/Base/Utility/_call_program`.
//!
//! Full upstream body (40 lines verbatim):
//! ```text
//! sh: 1  #autoload +X
//! sh: 2
//! sh: 3  local -xi COLUMNS=999
//! sh: 4  local curcontext="${curcontext}" tmp err_fd=-1 clocale='_comp_locale;'
//! sh: 5  local -a prefix
//! sh: 6
//! sh: 7  if [[ "$1" = -p ]]; then
//! sh: 8    shift
//! sh: 9    if (( $#_comp_priv_prefix )); then
//! sh:10      curcontext="${curcontext%:*}/${${(@M)_comp_priv_prefix:#^*[^\\]=*}[1]}:"
//! sh:11      zstyle -t ":completion:${curcontext}:${1}" gain-privileges &&
//! sh:12  	prefix=( $_comp_priv_prefix )
//! sh:13    fi
//! sh:14  elif [[ "$1" = -l ]]; then
//! sh:15    shift
//! sh:16    clocale=''
//! sh:17  fi
//! sh:18
//! sh:19  if (( ${debug_fd:--1} > 2 )) || [[ ! -t 2 ]]
//! sh:20  then exec {err_fd}>&2	# debug_fd is saved stderr, 2 is trace or redirect
//! sh:21  else exec {err_fd}>/dev/null
//! sh:22  fi
//! sh:23
//! sh:24  {	# Begin "always" block
//! sh:25
//! sh:26  if zstyle -s ":completion:${curcontext}:${1}" command tmp; then
//! sh:27    if [[ "$tmp" = -* ]]; then
//! sh:28      eval $clocale "$tmp[2,-1]" "$argv[2,-1]"
//! sh:29    else
//! sh:30      eval $clocale $prefix "$tmp"
//! sh:31    fi
//! sh:32  else
//! sh:33    eval $clocale $prefix "$argv[2,-1]"
//! sh:34  fi 2>&$err_fd
//! sh:35
//! sh:36  } always {
//! sh:37
//! sh:38  exec {err_fd}>&-
//! sh:39
//! sh:40  }
//! ```
//!
//! Three behaviors the previous stubs missed:
//! 1. `-p` flag → consult `$_comp_priv_prefix` (sudo/doas state) and,
//! gated by the `gain-privileges` zstyle, PREFIX the command with
//! the priv-prefix. The previous stub had no -p at all.
//! 2. `-l` flag → skip the `_comp_locale` C-locale env trampoline.
//! The previous stub never set locale env vars regardless.
//! 3. `command` zstyle override starting with `-` → REPLACE the
//! command wholesale with `$tmp[2,-1]` (strip the leading `-`).
//! Otherwise the style value is treated as a complete replacement
//! command. The previous stub treated every override as a
//! wholesale replacement, missing the prefix-append branch.
//!
//! Also sets `COLUMNS=999` so commands that wrap output (ls, ps,
//! …) emit one-per-line when invoked from completion.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
