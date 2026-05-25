//! Port of `_all_matches` from `Completion/Base/Completer/_all_matches`.
//!
//! Full upstream body (47 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  _all_matches() {
//! sh: 4    local old
//! sh: 5
//! sh: 6    zstyle -s ":completion:${curcontext}:" old-matches old
//! sh: 7
//! sh: 8    if [[ "$old" = (only|true|yes|1|on) ]]; then
//! sh: 9
//! sh:10      if [[ -n "$compstate[old_list]" ]]; then
//! sh:11        compstate[insert]=all
//! sh:12        compstate[old_list]=keep
//! sh:13        return 0
//! sh:14      fi
//! sh:15
//! sh:16      [[ "$old" = *only* ]] && return 1
//! sh:17    fi
//! sh:18
//! sh:19    (( $comppostfuncs[(I)_all_matches_end] )) ||
//! sh:20        comppostfuncs=( "$comppostfuncs[@]" _all_matches_end )
//! sh:21
//! sh:22    _all_matches_context=":completion:${curcontext}:"
//! sh:23
//! sh:24    return 1
//! sh:25  }
//! sh:26
//! sh:27  _all_matches_end() {
//! sh:28    local not
//! sh:29
//! sh:30    zstyle -s "$_all_matches_context" avoid-completer not ||
//! sh:31        not=( _expand _old_list _correct _approximate )
//! sh:32
//! sh:33    if [[ "$compstate[nmatches]" -gt 1 && $not[(I)(|_)$_completer] -eq 0 ]]; then
//! sh:34      local expl
//! sh:35
//! sh:36      if zstyle -t "$_all_matches_context" insert; then
//! sh:37        compstate[insert]=all
//! sh:38      else
//! sh:39        _description all-matches expl 'all matches'
//! sh:40        compadd "$expl[@]" -C
//! sh:41      fi
//! sh:42    fi
//! sh:43
//! sh:44    unset _all_matches_context
//! sh:45  }
//! sh:46
//! sh:47  _all_matches "$@"
//! ```
//!
//! Strict Rust port:
//! - shell:6 — resolve `old-matches` style.
//! - shell:8-17 — when truthy AND have_old_list → insert=all,
//! old_list=keep, return TRUE (success). When `only` AND no
//! old list → return false (shell `return 1`).
//! - shell:19-22 — arm `_all_matches_end` as a postfunc by
//! appending to `state.postfuncs` if not already present, AND
//! record the context in process-global state for the postfunc
//! to read.
//! - shell:24 — default `return 1` → Rust false.
//!
//! `_all_matches_end` (the postfunc, 16 lines) is also ported below
//! and registered on first use.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
