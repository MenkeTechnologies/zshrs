//! Port of `_comp_priv_prefix` — prefix for privilege escalation (sudo,
//! doas, etc.). Moved from `compsys/library.rs`. Renamed from
//! `comp_priv_prefix` to mirror zsh shell function name `_comp_priv_prefix`.

/// _comp_priv_prefix - Prefix for privilege escalation (sudo, doas, etc.)
pub fn _comp_priv_prefix() -> Vec<String> {
    // Returns the privilege prefix if any
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_priv_prefix_until_caller_populates() {
        // The shell-side equivalent is set by `sudo`/`doas` completion
        // wrappers; the leaf default is the empty vector. Pin so any
        // future drift surfaces as a test diff.
        assert!(_comp_priv_prefix().is_empty());
    }
}
