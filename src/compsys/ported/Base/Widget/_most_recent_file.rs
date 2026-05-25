//! Port of `_most_recent_file` from `Completion/Base/Widget/_most_recent_file`.
//!
//! Full upstream body (24 lines verbatim):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xm
//! sh: 2
//! sh: 3  # Complete the most recently modified file matching the pattern on the line
//! sh: 4  # so far: globbing is active, i.e. *.txt will be expanded to the most recent
//! sh: 5  # file ending in .txt
//! sh: 6  #
//! sh: 7  # With a prefix argument, select the Nth most recent matching file;
//! sh: 8  # negative arguments work in the opposite direction, so for example
//! sh: 9  # `Esc - \C-x m' gets you the oldest file.
//! sh:10
//! sh:11  local file tilde etilde
//! sh:12  if [[ $PREFIX = \~*/* ]]; then
//! sh:13    tilde=${PREFIX%%/*}
//! sh:14    etilde=${~tilde} 2>/dev/null
//! sh:15    # PREFIX and SUFFIX have full command line quoting in, but we want
//! sh:16    # any globbing characters which are quoted to stay quoted.
//! sh:17    eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"
//! sh:18    file=(${file/#$etilde})
//! sh:19    file=($tilde${(q)^file})
//! sh:20  else
//! sh:21    eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"
//! sh:22    file=(${(q)file})
//! sh:23  fi
//! sh:24  (( $#file )) && compadd -U -i "$IPREFIX" -I "$ISUFFIX" -f -Q -- $file
//! ```
//!
//! Strict Rust port: handles `~/`/`~user/` expansion (shell:12-19),
//! sorts by mtime descending, and honors `numeric_prefix` (mirrors
//! `${NUMERIC:-1}` → `om[N]` index, 1-based: 1=newest, 2=second-
//! newest, …). The pattern arg corresponds to `$PREFIX*$SUFFIX`
//! shell expansion.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
