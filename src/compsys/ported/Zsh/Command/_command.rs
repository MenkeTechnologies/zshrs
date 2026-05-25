//! Port of `_command` from `Completion/Zsh/Command/_command`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh: 1  #compdef command
//! sh: 2
//! sh: 3  _arguments \
//! sh: 4    '-v[indicate result of command search]:*:command:_path_commands' \
//! sh: 5    '-V[show result of command search in verbose form]:*:command:_path_commands' \
//! sh: 6    '(-)-p[use default PATH to find command]' \
//! sh: 7    '*:: : _normal -p $service'
//! ```
//!
//! Completion for the POSIX `command` builtin. Three optspecs and a
//! catch-all that dispatches the rest of the line to `_normal` with the
//! `-p` flag (which tells `_normal` to treat the next word as the
//! effective command name for arg-completion lookup).
//!
//! Faithful port: parses out which `command` flag is being completed
//! and what action the caller should take. compsys is a leaf crate
//! and can't itself invoke `_path_commands` or `_normal` (those need
//! the parent shell's path/cmdnam tables), so this port surfaces the
//! decision in a structured `CommandStage` enum and lets the caller
//! dispatch. `_arguments` itself isn't re-run here — we directly model
//! the four-clause _arguments table since it's tiny and immutable.

// GUTTED 2026-05-24 — body removed.
// Previously depended on `crate::compsys::compcore::CompletionState`
// and friends, which were deleted as duplicates of the real shell-side
// state in `src/ported/zle/compcore.rs`. Engine port body must be
// re-implemented to call into `crate::ported::zle::compcore::addmatch`
// against shell-side globals (PREFIX/SUFFIX/matches/etc.).
