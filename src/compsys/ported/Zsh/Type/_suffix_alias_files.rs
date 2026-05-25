//! Port of `_suffix_alias_files` from `Completion/Zsh/Type/_suffix_alias_files`.
//!
//! Full upstream body (22 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete files for which a suffix alias exists.
//! sh: 4
//! sh: 5  local expl pat
//! sh: 6
//! sh: 7  (( ${#saliases} )) || return 1
//! sh: 8
//! sh: 9  if (( ${#saliases} == 1 )); then
//! sh:10      pat="*.${(kq)saliases}"
//! sh:11  else
//! sh:12      local -a tmpa
//! sh:13      # This is so we can quote the alias names against expansion
//! sh:14      # without quoting the `|' which needs to be active in the pattern
//! sh:15      # --- remember that an alias name can be pretty much anything.
//! sh:16      tmpa=(${(kq)saliases})
//! sh:17      pat="*.(${(kj.|.)tmpa})"
//! sh:18  fi
//! sh:19  [[ -o autocd ]] || pat+='(#q^/)'
//! sh:20
//! sh:21  # _wanted is called for us by _command_names
//! sh:22  _path_files "$@" -g $pat
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the glob pattern
//! exactly as upstream does (single-key: `*.ext`; multi-key:
//! `*.(ext1|ext2|…)`), then dispatches via our ported
//! [`_path_files`] with `-g $pat`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
