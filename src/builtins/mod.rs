//! Ports of zsh's built-in builtins from `Src/builtin.c` (and the
//! sub-builtin code that lives elsewhere in `Src/`). Distinct from
//! `crate::modules`, which mirrors `Src/Modules/*.c` (the *loadable*
//! modules).
//!
//! Builtins land here when their C source is in `Src/builtin.c`,
//! `Src/exec.c`, etc. — i.e. things zsh has compiled into the core
//! shell, not loaded via `zmodload`.

pub mod rlimits;
pub mod sched;
