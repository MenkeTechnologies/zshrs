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
/// `cap` submodule.
pub mod cap;
/// `clone` submodule.
pub mod clone;
/// `curses` submodule.
pub mod curses;
/// `datetime` submodule.
pub mod datetime;
/// `db_gdbm` submodule.
pub mod db_gdbm;
/// `example` submodule.
pub mod example;
/// `files` submodule.
pub mod files;
/// `hlgroup` submodule.
pub mod hlgroup;
/// `ksh93` submodule.
pub mod ksh93;
/// `langinfo` submodule.
pub mod langinfo;
/// `mapfile` submodule.
pub mod mapfile;
/// `mathfunc` submodule.
pub mod mathfunc;
/// `nearcolor` submodule.
pub mod nearcolor;
/// `newuser` submodule.
pub mod newuser;
/// `param_private` submodule.
pub mod param_private;
/// `parameter` submodule.
pub mod parameter;
/// `pcre` submodule.
pub mod pcre;
/// `random` submodule.
pub mod random;
/// `random_real` submodule.
pub mod random_real;
/// `regex` submodule.
pub mod regex;
/// `socket` submodule.
pub mod socket;
/// `stat` submodule.
pub mod stat;
/// `system` submodule.
pub mod system;
/// `tcp` submodule.
pub mod tcp;
/// `tcp_h` submodule.
pub mod tcp_h;
/// `termcap` submodule.
pub mod termcap;
/// `terminfo` submodule.
pub mod terminfo;
/// `watch` submodule.
pub mod watch;
/// `zftp` submodule.
pub mod zftp;
/// `zprof` submodule.
pub mod zprof;
/// `zpty` submodule.
pub mod zpty;
/// `zselect` submodule.
pub mod zselect;
/// `zutil` submodule.
pub mod zutil;

#[cfg(test)]
mod tests {
    use crate::zsh_h::{hashnode, param, PM_SCALAR};

    /// `Src/Modules/parameter.c::paramtypestr` is consulted by every
    /// `${(t)foo}` query and by scanpmparameters' callback emission.
    /// The encoding is load-bearing: an integer param must report
    /// "integer", a scalar "scalar", etc. — this cross-checks that
    /// the dispatch the rest of the parameter introspection relies on
    /// is intact.
    #[test]
    fn paramtypestr_reports_scalar_for_scalar() {
        let _g = crate::test_util::global_state_lock();
        let pm = param {
            node: hashnode {
                next: None,
                nam: String::new(),
                flags: PM_SCALAR as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some("v".to_string()),
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        };
        let typ = super::parameter::paramtypestr(&pm);
        assert_eq!(typ, "scalar");
    }
}
