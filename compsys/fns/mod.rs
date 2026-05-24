//! Per-function ports of zsh compsys shell functions — one file per
//! function, mirroring the upstream `/usr/share/zsh/functions/_NAME`
//! layout (kept locally as `compsys/functions/Base/Utility/_NAME`
//! etc. for reference).
//!
//! Each file is the Rust port of exactly one shell function. They
//! re-export through this `mod.rs` and through `compsys/lib.rs` so
//! callers can use the short paths `compsys::fns::command_names`.

pub mod command_names;
pub mod completers;
pub mod dir_list;
pub(crate) mod shared;
pub mod widgets;

pub use command_names::{command_names, command_names_with_ctx, ShellInventory};
pub use completers::{completers, CANONICAL_COMPLETER_NAMES};
pub use dir_list::{dir_list, DirListOpts};
pub use widgets::widgets;
