//! `cow_map` — copy-on-write HashMap wrapper for cheap subshell
//! snapshot/restore (Rust-only helper).
//!
//! NOTE: This file was missing from the working tree at commit
//! `8a31b121bf` (the `mod cow_map;` decl in `src/lib.rs` and the
//! `SubshellSnapshot::paramtab_hashed_storage` field in
//! `src/vm_helper.rs` were committed without it). Reconstructed to
//! match the exact call-site contract: every constructor clones the
//! canonical `HashMap` store into the field, and restore assigns the
//! field back into the live `HashMap` (`*m = snap...`), with no
//! `.into()` conversions — so the type must be layout-identical to
//! `std::collections::HashMap`. A real copy-on-write wrapper can be
//! swapped in behind this alias later without touching call sites.

use std::collections::HashMap;

/// Copy-on-write associative store used for subshell snapshot/restore.
///
/// Currently a transparent alias over the standard `HashMap` so the
/// `SubshellSnapshot` clone-in / assign-out call sites typecheck. The
/// name is stable so the backing type can become a genuine structural
/// CoW map (Arc-shared until first mutation) without a call-site churn.
pub type CowHashMap<K, V> = HashMap<K, V>;
