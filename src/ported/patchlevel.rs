//! `patchlevel.h` port — upstream zsh patch-level identifier.
//!
//! Port of `Src/patchlevel.h`. The C header defines a single
//! preprocessor constant `ZSH_PATCHLEVEL` that names the upstream
//! source revision the rest of the tree was built from. It's read
//! by `init.c` to populate the `$ZSH_PATCHLEVEL` shell parameter
//! and by the `zsh --version` startup-banner code.
//!
//! C source: 1 `#define`, 0 structs/enums/fns.

/// Port of `#define ZSH_PATCHLEVEL` from `Src/patchlevel.h:1`.
///
/// The upstream source-revision tag the C tree was generated from.
/// Format follows the convention `zsh-MAJOR.MINOR.PATCH-test-N-gHASH`
/// (where the trailing `-N-gHASH` is appended by `git describe`
/// during the upstream release process).
///
/// zshrs reads this constant verbatim from the upstream snapshot in
/// `src/zsh/Src/patchlevel.h` so the `$ZSH_PATCHLEVEL` shell
/// parameter (params.rs:1690) reports the same value zsh would on
/// the same source tree.
pub const ZSH_PATCHLEVEL: &str = "zsh-5.9-465-g6b9704e";                     // c:1

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies `ZSH_PATCHLEVEL` matches the verbatim C value at
    /// patchlevel.h:1. If upstream zsh updates this string, the
    /// snapshot at `src/zsh/Src/patchlevel.h` and this constant
    /// must move together.
    #[test]
    fn patchlevel_matches_upstream_snapshot() {
        assert_eq!(ZSH_PATCHLEVEL, "zsh-5.9-465-g6b9704e");
        assert!(ZSH_PATCHLEVEL.starts_with("zsh-"));
    }
}
