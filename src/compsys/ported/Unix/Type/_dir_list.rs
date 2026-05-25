//! Port of `_dir_list` from `Completion/Unix/Type/_dir_list`.
//!
//! Full upstream body (29 lines verbatim):
//! ```text
//! sh: 1  #compdef -value-,TERMINFO_DIRS,-default- -P -value-,*PATH,-default-
//! sh: 2
//! sh: 3  # options:
//! sh: 4  #  -s <sep> to specify the separator (default is a colon)
//! sh: 5  #  -S       to say that the separator should be added as a suffix (instead
//! sh: 6  #           of the default slash)
//! sh: 7  # any description passed should apply to an individual directory and not
//! sh: 8  # to the entire list
//! sh: 9
//! sh:10  local sep=: dosuf suf
//! sh:11
//! sh:12  while [[ "$1" = -(s*|S) ]]; do
//! sh:13    case "$1" in
//! sh:14    -s)  sep="$2"; shift 2;;
//! sh:15    -s*) sep="${1[3,-1]}"; shift;;
//! sh:16    -S)  dosuf=yes; shift;;
//! sh:17    esac
//! sh:18  done
//! sh:19
//! sh:20  compset -P "*${sep}"
//! sh:21  compset -S "${sep}*" || suf="$sep"
//! sh:22
//! sh:23  if [[ -n "$dosuf" ]]; then
//! sh:24    suf=(-S "$suf")
//! sh:25  else
//! sh:26    suf=()
//! sh:27  fi
//! sh:28
//! sh:29  _directories "$suf[@]" -r "${sep}"' /\t\t\-' "$@"
//! ```
//!
//! The previous Rust stub did the wrong thing: it tried to scan
//! directories itself instead of chewing the prefix with compset and
//! delegating to `_directories`. That meant
//! 1. all the `_directories` styles (`list-dirs-first`,
//! `special-dirs`, etc.) were silently ignored;
//! 2. the trailing-separator handling for the rest of the list was
//! wrong;
//! 3. the `-S` suffix-mode branch didn't exist.
//!
//! Faithful re-port:
//! - chews any `(*sep)` prefix already typed
//! (sets `iprefix`/strips `prefix`);
//! - checks for a sep already present in `suffix`;
//! - delegates to `directories_execute` from `compsys::files` which
//! IS the Rust `_directories` and honors all the file zstyles;
//! - threads the right suffix-removal char (`-r ${sep}/\t\-`) so
//! Tab into a value followed by `:` / Enter strips the trailing
//! `/` and re-arms the separator for the next item.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
