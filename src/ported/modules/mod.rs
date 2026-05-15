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
pub mod socket;
pub mod stat;
pub mod system;
pub mod tcp;
pub mod tcp_h;
pub mod termcap;
pub mod terminfo;
pub mod watch;
pub mod zftp;
pub mod zprof;
pub mod zpty;
pub mod zselect;
pub mod zutil;

#[cfg(test)]
mod tests {
    /// `Src/Modules/parameter.c::paramtypestr` is consulted by every
    /// `${(t)foo}` query and by scanpmparameters' callback emission.
    /// The encoding is load-bearing: an integer param must report
    /// "integer", a scalar "scalar", etc. — this cross-checks that
    /// the dispatch the rest of the parameter introspection relies on
    /// is intact.
    #[test]
    fn paramtypestr_reports_scalar_for_scalar() {
        use crate::ported::zsh_h::{hashnode, param, PM_SCALAR};
        let pm = param {
            node: hashnode { next: None, nam: String::new(), flags: PM_SCALAR as i32 },
            u_data: 0, u_arr: None, u_str: Some("v".to_string()),
            u_val: 0, u_dval: 0.0, u_hash: None,
            gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
            base: 0, width: 0, env: None, ename: None, old: None, level: 0,
        };
        let typ = super::parameter::paramtypestr(&pm);
        assert_eq!(typ, "scalar");
    }
}
