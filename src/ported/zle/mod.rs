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

// Core ZLE types (old API for exec.rs compatibility)

// New comprehensive ZLE port from C

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::ported::zle::zle_h::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

pub mod zle_bindings;
pub mod compctl;
pub mod compcore;
pub mod complete;
pub mod complist;
pub mod compmatch;
pub mod compresult;
pub mod computil;
pub mod deltochar;
pub mod zle_hist;
pub mod zle_keymap;
pub mod zle_main;
pub mod zle_misc;
pub mod zle_move;
pub mod zle_params;
pub mod zle_refresh;
pub mod termquery;
pub mod textobjects;
pub mod zle_thingy;
pub mod zle_tricky;
pub mod zle_utils;
pub mod zle_vi;
pub mod zle_word;
pub mod zleparameter;
pub mod compctl_h;
pub mod comp_h;
pub mod zle_h;

// Canonical re-exports. `Widget` / `WidgetFunc` legacy aliases live on
// `zle_h` itself (via `pub type Widget = widget; pub use
// WidgetImpl as WidgetFunc;`) so existing call sites keep resolving.
pub use zle_keymap::Keymap;
pub use zle_thingy::Thingy;
pub use zle_h::{widget as Widget, WidgetImpl as WidgetFunc};
