mod history_search;

mod input;
/// `iothreads` submodule.
pub mod iothreads;
/// `reader` submodule.
#[allow(clippy::module_inception)]
pub mod reader;

mod word_motion;

pub use reader::*;
