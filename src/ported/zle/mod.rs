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

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_h::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};

pub mod comp_h;
pub mod compcore;
pub mod compctl;
pub mod compctl_h;
pub mod complete;
pub mod complist;
pub mod compmatch;
pub mod compresult;
pub mod computil;
pub mod deltochar;
pub mod termquery;
pub mod textobjects;
pub mod zle_bindings;
pub mod zle_h;
pub mod zle_hist;
pub mod zle_keymap;
pub mod zle_main;
pub mod zle_misc;
pub mod zle_move;
pub mod zle_params;
pub mod zle_refresh;
pub mod zle_thingy;
pub mod zle_tricky;
pub mod zle_utils;
pub mod zle_vi;
pub mod zle_word;
pub mod zleparameter;

// Canonical re-exports. `Widget` / `WidgetFunc` legacy aliases live on
// `zle_h` itself (via `pub type Widget = widget; pub use
// WidgetImpl as WidgetFunc;`) so existing call sites keep resolving.
pub use zle_h::{widget as Widget, WidgetImpl as WidgetFunc};
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
