/// `shared` submodule.
pub mod shared;
/// `abbr` submodule.
pub mod abbr;
/// `argparse` submodule.
pub mod argparse;
/// `bg` submodule.
pub mod bg;
/// `bind` submodule.
pub mod bind;
/// `block` submodule.
pub mod block;
/// `r` submodule.
pub mod r#break;
/// `breakpoint` submodule.
pub mod breakpoint;
/// `builtin` submodule.
pub mod builtin;
/// `cd` submodule.
pub mod cd;
/// `command` submodule.
pub mod command;
/// `commandline` submodule.
pub mod commandline;
/// `complete` submodule.
pub mod complete;
/// `contains` submodule.
pub mod contains;
/// `r` submodule.
pub mod r#continue;
/// `count` submodule.
pub mod count;
/// `disown` submodule.
pub mod disown;
/// `echo` submodule.
pub mod echo;
/// `emit` submodule.
pub mod emit;
/// `eval` submodule.
pub mod eval;
/// `exit` submodule.
pub mod exit;
/// `r` submodule.
pub mod r#false;
/// `fg` submodule.
pub mod fg;
/// `fish_indent` submodule.
pub mod fish_indent;
/// `fish_key_reader` submodule.
pub mod fish_key_reader;
/// `function` submodule.
pub mod function;
/// `functions` submodule.
pub mod functions;
/// `r` submodule.
pub mod r#gettext;
/// `history` submodule.
pub mod history;
/// `jobs` submodule.
pub mod jobs;
/// `math` submodule.
pub mod math;
/// `path` submodule.
pub mod path;
/// `printf` submodule.
pub mod printf;
/// `pwd` submodule.
pub mod pwd;
/// `random` submodule.
pub mod random;
/// `read` submodule.
pub mod read;
/// `realpath` submodule.
pub mod realpath;
/// `r` submodule.
pub mod r#return;
/// `set` submodule.
pub mod set;
/// `set_color` submodule.
pub mod set_color;
/// `source` submodule.
pub mod source;
/// `status` submodule.
pub mod status;
/// `string` submodule.
pub mod string;
/// `test` submodule.
pub mod test;
/// `r` submodule.
pub mod r#true;
/// `r` submodule.
pub mod r#type;
/// `ulimit` submodule.
pub mod ulimit;
/// `wait` submodule.
pub mod wait;

mod prelude {
    pub use super::shared::*;
    pub use libc::c_int;
    pub use std::borrow::Cow;

    #[allow(unused_imports)]
    pub(crate) use crate::{
        flog::{flog, flogf},
        io::{IoStreams, SeparationType},
        parser::Parser,
        prelude::*,
        wutil::{fish_wcstoi, fish_wcstol, fish_wcstoul},
    };
    pub(crate) use fish_wgetopt::{
        wopt,
        ArgType::{self, *},
        WGetopter, WOption, NON_OPTION_CHAR,
    };
}

pub use shared::*;
