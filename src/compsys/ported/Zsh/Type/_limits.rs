//! Port of `_limits` from `Completion/Zsh/Type/_limits`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh: 1  #compdef unlimit
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _wanted limits expl 'process limit' compadd "$@" - ${${(f)"$(limit)"}%% *}
//! ```
//!
//! Faithful re-port: mirrors shell-side `_wanted limits expl 'process
//! limit' compadd "$@" - <names>` by calling our ported [`_wanted`]
//! with `tag = "limits"` and `descr = "process limit"`. Inner action
//! is the equivalent of `compadd "$@" - <names>`: add each `<name>`
//! as a match, honouring `"$@"` passthrough opts.
//!
//! Shell-side `${${(f)"$(limit)"}%% *}` runs the `limit` builtin,
//! splits stdout on newlines (`(f)`), and strips everything from the
//! first whitespace onward (`%% *`). That yields the bare limit names
//! ("cputime", "filesize", ...).
//!
//! Rust-side divergence (`// rust:`): the leaf can't fork-exec `limit`
//! from inside compsys; instead callers either accept the static
//! [`LIMIT_NAMES`] (default) or inject their own list via the
//! `names` parameter. Behaviour is otherwise identical.
//!
//! Shell local `expl` is the description-array name threaded into
//! `_wanted`; in Rust it's implicit (the description is `_wanted`'s
//! third argument).

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
