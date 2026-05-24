//! Per-function ports of zsh compsys shell functions — one file per
//! function, file name `_<NAME>.rs` exactly matching the upstream
//! shell function names under
//! `/opt/homebrew/share/zsh/functions/_<NAME>`. The shell sources
//! are kept locally for reference at
//! `compsys/functions/Base/Utility/_<NAME>` etc.
//!
//! Both the modules AND the public function names retain the leading
//! `_` so Rust call sites read identically to zsh:
//! `compsys::fns::_command_names(...)` mirrors `_command_names ...`.

#[allow(non_snake_case, non_camel_case_types)]
pub mod _call_program;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _combination;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _command_names;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _completers;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _dir_list;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _pick_variant;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _sequence;
pub(crate) mod shared;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _widgets;

pub use _call_program::{_call_program, CallProgramOpts, CallProgramResult};
pub use _combination::{_combination, _combination_mcs, CombinationOpts};
pub use _command_names::{_command_names, _command_names_with_ctx, ShellInventory};
pub use _completers::{_completers, CANONICAL_COMPLETER_NAMES};
pub use _dir_list::{_dir_list, DirListOpts};
pub use _pick_variant::{_pick_variant, PickVariantOpts, PickVariantResult};
pub use _sequence::{_sequence, SequenceOpts};
pub use _widgets::_widgets;
