//! Port of `_extensions` from `Completion/Base/Completer/_extensions`.
//!
//! Full upstream body (33 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This completer completes filename extensions when completing
//! sh: 4  # after *. or ^*. It can be used anywhere in the completer list
//! sh: 5  # but if used after _expand, patterns that already match a file
//! sh: 6  # will be expanded before it is called.
//! sh: 7
//! sh: 8  compset -P '(#b)([~$][^/]#/|)(*/|)(\^|)\*.' || return 1
//! sh: 9
//! sh:10  local -aU files
//! sh:11  local -a expl suf mfiles
//! sh:12
//! sh:13  files=( ${(e)~match[1]}${match[2]}*.* ) || return 1
//! sh:14  eval set -A files '${(MSI:'{1..${#${(O)files//[^.]/}[1]}}':)files%%.[^/]##}'
//! sh:15  files=( ${files:#.<->(.*|)} )
//! sh:16
//! sh:17  if zstyle -t ":completion:${curcontext}:extensions" prefix-hidden; then
//! sh:18    files=( ${files#.} )
//! sh:19  else
//! sh:20    PREFIX=".$PREFIX"
//! sh:21    IPREFIX="${IPREFIX%.}"
//! sh:22  fi
//! sh:23
//! sh:24  zstyle -T ":completion:${curcontext}:extensions" add-space ||
//! sh:25    suf=( -S '' )
//! sh:26
//! sh:27  _description extensions expl 'file extension'
//! sh:28
//! sh:29  # for an exact match, fail so as to give _expand or _match a chance.
//! sh:30  compadd -O mfiles "$expl[@]" -a files
//! sh:31  [[ $#mfiles -gt 1 || ${mfiles[1]} != $PREFIX ]] &&
//! sh:32      compadd "$expl[@]" "$suf[@]" -a files &&
//! sh:33      [[ -z $compstate[exact_string] ]]
//! ```
//!
//! Shell version is triggered by a typed `*.` (or `^*.`) prefix and
//! computes the set of distinct extensions present in the target
//! directory.
//!
//! Strict Rust port: in addition to the per-call extension
//! whitelist, honors the `prefix-hidden` zstyle (shell:16-17):
//! when truthy, strips a leading `.` from each emitted name.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
