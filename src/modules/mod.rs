//! Ports of zsh's loadable modules from `Src/Modules/*.c`.
//!
//! Each submodule maps 1:1 to its C-source counterpart. The names
//! match `zsh/<modname>` as exposed by `zmodload`. See each file's
//! header for the specific source-file citation and feature scope.
//!
//! For backwards-compat with the previous flat `crate::<modname>`
//! layout, every entry is re-exported at this module's root so
//! `crate::modules::datetime::...` and `crate::modules::*` both
//! resolve. Top-level call sites should migrate to
//! `crate::modules::<modname>::...` going forward.

pub mod attr;
pub mod cap;
pub mod clone;
pub mod curses;
pub mod datetime;
pub mod db_gdbm;
pub mod example;
pub mod files;
pub mod hlgroup;
pub mod ksh93;
pub mod langinfo;
pub mod mapfile;
pub mod mathfunc;
pub mod nearcolor;
pub mod newuser;
pub mod param_private;
pub mod parameter;
pub mod pcre;
pub mod random;
pub mod random_real;
pub mod regex;
pub mod sched;
pub mod socket;
pub mod stat;
pub mod system;
pub mod tcp;
pub mod termcap;
pub mod terminfo;
pub mod watch;
pub mod zftp;
pub mod zprof;
pub mod zpty;
pub mod zselect;
pub mod zutil;
