//! Port of `_normal` — normal command completion.
//!
//! Local shell reference: `compsys/functions/Base/Core/_normal`
//! (system copy `/opt/homebrew/share/zsh/functions/_normal`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  local _comp_command1 _comp_command2 _comp_command precommand
//!  6  zparseopts -A opts -D - P p+:-=precommand s
//! 15  if [[ -o BANG_HIST && ( ($words[CURRENT] = \!*: …) || … ) ]]; then
//! 21    compset -P '*:'
//! 22    _history_modifiers h
//! 28  if [[ CURRENT -eq 1 ]]; then
//! 29    curcontext="${curcontext%:*:*}:-command-:"
//! 31    comp="$_comps[-command-]"
//! 32    [[ -n "$comp" ]] && eval "$comp" && return
//! 34    return 1
//! 37  _set_command
//! 39  _dispatch ${(k)opts[-s]} "$_comp_command" \
//!              "$_comp_command1" "$_comp_command2" -default-
//! ```
//!
//! Faithful Rust port: full algorithm shape preserved.
//!
//!   - shell:15-23: BANG_HIST history-modifier path — skipped here
//!     (shell-side history expansion lives at the lex layer).
//!   - shell:28-34: command-position dispatch via `_comps[-command-]`.
//!     Rust port takes a `cmd_handler` closure; the caller resolves
//!     the registered `-command-` entry and invokes it. Returns
//!     `Matched` iff the handler returned true.
//!   - shell:37-40: argument-position dispatch via `_dispatch`. Rust
//!     port takes an `arg_handler` closure called with the command
//!     name; caller looks it up in their comps table.
//!
//! If callers don't supply handlers (e.g. when called bare via the
//! 0-arg `_normal` shim below), we return `NoMatch` faithfully —
//! same as upstream `return 1` at shell:34 when no `_comps[-command-]`
//! is set.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
