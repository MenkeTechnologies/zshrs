//! Port of `_list` from `Completion/Base/Completer/_list`.
//!
//! Full upstream body (37 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This completer function makes the other completer functions used
//! sh: 4  # insert possible completions only after the list has been shown at
//! sh: 5  # least once.
//! sh: 6
//! sh: 7  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 8
//! sh: 9  local pre suf expr
//! sh:10
//! sh:11  # Get the strings to compare.
//! sh:12
//! sh:13  if zstyle -t ":completion:${curcontext}:" word; then
//! sh:14    pre="$HISTNO$LBUFFER"
//! sh:15    suf="$RBUFFER"
//! sh:16  else
//! sh:17    pre="$PREFIX"
//! sh:18    suf="$SUFFIX"
//! sh:19  fi
//! sh:20
//! sh:21  # Should we only show a list now?
//! sh:22
//! sh:23  if zstyle -T ":completion:${curcontext}:" condition &&
//! sh:24     [[ "$pre" != "$_list_prefix" || "$suf" != "$_list_suffix" ]]; then
//! sh:25
//! sh:26    # Yes. Tell the completion code about it and save the new values
//! sh:27    # to compare the next time.
//! sh:28
//! sh:29    compstate[insert]=''
//! sh:30    compstate[list]='list force'
//! sh:31    _list_prefix="$pre"
//! sh:32    _list_suffix="$suf"
//! sh:33  fi
//! sh:34
//! sh:35  # We always return one, because we don't really do any work here.
//! sh:36
//! sh:37  return 1
//! ```
//!
//! Strict Rust port:
//! - shell:7 — bail when matcher_num > 1 (caller-supplied).
//! - shell:13-18 — choose `pre`: `word` style true → use `$HISTNO$LBUFFER`
//! (caller-supplied), else use `$PREFIX`.
//! - shell:23-33 — gate on `condition` style being truthy AND
//! (pre or suf differs from the last-seen pair). On hit, set
//! `compstate[insert]=''`, `compstate[list]='list force'`,
//! and record (pre, suf) for the next call's diff check.
//! - shell:37 — `return 1` → mapped to Rust `false`. Caller
//! interprets false as "no matches added".

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
