//! Port of `_comp_locale` — set locale for completion. Moved from
//! `compsys/functions.rs`. Renamed from `comp_locale` to mirror zsh
//! shell function name `_comp_locale`.

/// _comp_locale - Set locale for completion
pub fn _comp_locale() {
    // Would set LC_ALL=C or similar
    // In Rust, this is handled differently
}
