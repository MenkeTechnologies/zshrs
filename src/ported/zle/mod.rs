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
use crate::extensions::widget::*;
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

#[path = "../../extensions/keymaps.rs"] pub mod keymaps;
#[path = "../../extensions/widgets.rs"] pub mod widgets;
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
#[path = "../../extensions/widget.rs"] pub mod widget;
pub mod zle_word;
pub mod zleparameter;
pub mod compctl_h;
pub mod comp_h;
pub mod zle_h;

// Re-export old API for compatibility with exec.rs
pub use keymaps::{zle, Keymap as LegacyKeymap, KeymapName, ZleManager, ZleState};
pub use widgets::{BuiltinWidget, Widget as LegacyWidget, WidgetResult};

// Re-export new API
pub use zle_keymap::Keymap;
pub use zle_thingy::Thingy;
pub use widget::{Widget, WidgetFlags, WidgetFunc};
