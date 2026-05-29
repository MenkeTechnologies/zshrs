//! ZLE - Zsh Line Editor
//!
//! Direct port from zsh/Src/Zle/*.c
//!
//! This module implements the full Zsh line editor with:
//! - Vi and Emacs editing modes
//! - Programmable keymaps
//! - Widgets (commands)
//! - Completion integration
//! - History navigation
//! - Multi-line editing

// Core ZLE types (old API for vm_helper compatibility)

// New comprehensive ZLE port from C
/// `comp_h` submodule.

pub mod comp_h;
/// `compcore` submodule.
pub mod compcore;
/// `compctl` submodule.
pub mod compctl;
/// `compctl_h` submodule.
pub mod compctl_h;
/// `complete` submodule.
pub mod complete;
/// `complist` submodule.
pub mod complist;
/// `compmatch` submodule.
pub mod compmatch;
/// `compresult` submodule.
pub mod compresult;
/// `computil` submodule.
pub mod computil;
/// `deltochar` submodule.
pub mod deltochar;
/// `termquery` submodule.
pub mod termquery;
/// `textobjects` submodule.
pub mod textobjects;
/// `zle_bindings` submodule.
pub mod zle_bindings;
/// `zle_h` submodule.
pub mod zle_h;
/// `zle_hist` submodule.
pub mod zle_hist;
/// `zle_keymap` submodule.
pub mod zle_keymap;
/// `zle_main` submodule.
pub mod zle_main;
/// `zle_misc` submodule.
pub mod zle_misc;
/// `zle_move` submodule.
pub mod zle_move;
/// `zle_params` submodule.
pub mod zle_params;
/// `zle_refresh` submodule.
pub mod zle_refresh;
/// `zle_thingy` submodule.
pub mod zle_thingy;
/// `zle_tricky` submodule.
pub mod zle_tricky;
/// `zle_utils` submodule.
pub mod zle_utils;
/// `zle_vi` submodule.
pub mod zle_vi;
/// `zle_word` submodule.
pub mod zle_word;
/// `zleparameter` submodule.
pub mod zleparameter;

pub use zle_h::{widget, WidgetImpl};
pub use zle_keymap::Keymap;
pub use zle_thingy::Thingy;

#[cfg(test)]
mod tests {
    use super::*;

    /// Keymap default initialises with all 256 first[] slots as None
    /// (the `t_undefinedkey` sentinel) and empty multi table. Verifies
    /// the new() / Default::default() contract used by every newkeymap
    /// + bindkey path — a regression breaking this would mean every
    /// keymap starts with random state.
    #[test]
    fn empty_keymap_has_all_unbound_first_slots() {
        let _g = crate::test_util::global_state_lock();
        let km = zle_keymap::Keymap::default();
        assert!(km.first.iter().all(|t| t.is_none()), "all 256 slots None");
        assert!(km.multi.is_empty(), "no multi-byte bindings yet");
        assert_eq!(km.flags, 0);
    }

    /// Thingy::new builds an immortal stub with rc=1 and no widget.
    /// Verifies the calloc-equivalent baseline the rest of the thingy
    /// pipeline (refthingy / unrefthingy) is allowed to assume.
    #[test]
    fn freshly_minted_thingy_has_rc_one_and_no_widget() {
        let _g = crate::test_util::global_state_lock();
        let t = zle_thingy::Thingy::new("test-widget");
        assert_eq!(t.rc, 1);
        assert!(t.widget.is_none());
        assert_eq!(t.nam, "test-widget");
    }
}
