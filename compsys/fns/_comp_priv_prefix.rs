//! Port of `_comp_priv_prefix` — prefix for privilege escalation (sudo,
//! doas, etc.). Moved from `compsys/library.rs`. Renamed from
//! `comp_priv_prefix` to mirror zsh shell function name `_comp_priv_prefix`.

/// _comp_priv_prefix - Prefix for privilege escalation (sudo, doas, etc.)
pub fn _comp_priv_prefix() -> Vec<String> {
    // Returns the privilege prefix if any
    Vec::new()
}
