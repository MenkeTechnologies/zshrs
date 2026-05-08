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
#[path = "../../extensions/keymaps.rs"] pub mod keymaps;
#[path = "../../extensions/widgets.rs"] pub mod widgets;
pub mod zle_bindings;
pub mod compctl;
pub mod compcore;
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

// Re-export old API for compatibility with exec.rs
pub use keymaps::{zle, Keymap as LegacyKeymap, KeymapName, ZleManager, ZleState};
pub use widgets::{BuiltinWidget, Widget as LegacyWidget, WidgetResult};

// Re-export new API
pub use zle_keymap::{Keymap, KeymapManager};
pub use zle_main::Zle;
pub use zle_thingy::Thingy;
pub use widget::{Widget, WidgetFlags, WidgetFunc};
