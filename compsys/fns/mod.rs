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
pub mod _canonical_paths;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _combination;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _command_names;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _completers;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _dir_list;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _email_addresses;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _multi_parts;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _numbers;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _pick_variant;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _sequence;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _tags;
pub(crate) mod shared;
#[allow(non_snake_case, non_camel_case_types)]
pub mod _widgets;
pub mod _arguments;
pub mod _describe;
pub mod _files;

pub use _call_program::{CallProgramOpts, CallProgramResult, _call_program};
pub use _canonical_paths::{CanonicalPathsOpts, _canonical_paths};
pub use _combination::{CombinationOpts, _combination, _combination_mcs};
pub use _command_names::{ShellInventory, _command_names, _command_names_with_ctx};
pub use _completers::{_completers, CANONICAL_COMPLETER_NAMES};
pub use _dir_list::{DirListOpts, _dir_list};
pub use _email_addresses::{EmailAddressesOpts, _email_addresses};
pub use _multi_parts::{MultiPartsOpts, _multi_parts};
pub use _numbers::{NumbersOpts, _numbers};
pub use _pick_variant::{PickVariantOpts, PickVariantResult, _pick_variant};
pub use _sequence::{SequenceOpts, _sequence};
pub use _tags::{TagSortHook, TagsOpts, _tags, _tags_mcs, _tags_next};
pub use _widgets::_widgets;
