//! Port of `_in_vared` from `Completion/Zsh/Context/_in_vared`.
//!
//! Full upstream body (35 lines verbatim):
//! ```text
//! sh: 1  #compdef -vared-
//! sh: 2
//! sh: 3  local also
//! sh: 4
//! sh: 5  # Completion inside vared.
//! sh: 6
//! sh: 7  if [[ $compstate[vared] = *\[* ]]; then
//! sh: 8    if [[ $compstate[vared] = *\]* ]]; then
//! sh: 9      # vared on an array-element
//! sh:10      compstate[parameter]=${${compstate[vared]%%\]*}//\[/-}
//! sh:11      compstate[context]=value
//! sh:12      also=-value-
//! sh:13    else
//! sh:14      # vared on an array-value
//! sh:15      compstate[parameter]=${compstate[vared]%%\[*}
//! sh:16      compstate[context]=value
//! sh:17      also=-value-
//! sh:18    fi
//! sh:19  else
//! sh:20    # vared on a parameter, let's see if it is an array
//! sh:21    compstate[parameter]=$compstate[vared]
//! sh:22    if [[ ${(tP)compstate[vared]} = *(array|assoc)* ]]; then
//! sh:23      compstate[context]=array_value
//! sh:24      also=-array-value-
//! sh:25    else
//! sh:26      compstate[context]=value
//! sh:27      also=-value-
//! sh:28    fi
//! sh:29  fi
//! sh:30
//! sh:31  # Don't insert TAB in first column. Never.
//! sh:32
//! sh:33  compstate[insert]="${compstate[insert]//tab /}"
//! sh:34
//! sh:35  _dispatch "$also" "$also"
//! ```
//!
//! Strict Rust port: parses the `$compstate[vared]` shape, sets
//! `compstate.parameter` + `compstate.context` accordingly, then
//! dispatches via [`_dispatch`].

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
