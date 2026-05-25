//! Port of `_aliases` from `Completion/Zsh/Type/_aliases`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #compdef unalias
//! sh: 2
//! sh: 3  local expl sel args opts
//! sh: 4
//! sh: 5  zparseopts -E -D s:=sel
//! sh: 6
//! sh: 7  [[ -z $sel ]] && sel=rgs
//! sh: 8
//! sh: 9  opts=( "$@" )
//! sh:10
//! sh:11  args=()
//! sh:12  [[ $sel = *r* ]] && args=( $args 'aliases:regular alias:compadd -k aliases' )
//! sh:13  [[ $sel = *g* ]] && args=( $args 'global-aliases:global alias:compadd -k galiases' )
//! sh:14  [[ $sel = *s* ]] && args=( $args 'suffix-aliases:suffix alias:compadd -k saliases' )
//! sh:15  [[ $sel = *R* ]] && args=( $args 'disabled-aliases:disabled regular alias:compadd -k dis_aliases' )
//! sh:16  [[ $sel = *G* ]] && args=( $args 'disabled-global-aliases:disabled global alias:compadd -k dis_galiases' )
//! sh:17  [[ $sel = *S* ]] && args=( $args 'disabled-suffix-aliases:disabled suffix alias:compadd -k dis_saliases' )
//! sh:18
//! sh:19  _alternative -O opts $args
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the `tag:desc:action`
//! spec strings exactly as upstream does, then dispatches via our
//! ported [`_alternative`]. The action string is the `compadd -k
//! <tablename>` invocation; the action_handler closure resolves
//! `<tablename>` to the right alias slice we've been handed and
//! emits via `add_match`.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
