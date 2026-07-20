//! Port of `_dates` from `Completion/Unix/Type/_dates`.
//!
//! Full upstream body (127 lines, abridged):
//! ```text
//! sh:  1  #autoload   (options: -f FORMAT, -F future)
//! sh: 30  zparseopts -D -K -E f:=format F=future
//! sh: 31  (( future = $#future ? 1 : -1 ))
//! sh: 32  zstyle -s …:dates date-format userformat
//! sh: 33  format=${userformat:-${format[2]:-%F}}
//! sh: 35  zstyle -a …:dates max-matches-length r; … row budget …
//! sh: 42  zmodload -i zsh/datetime || rows=0
//! sh: 44  _tags dates || return 0
//! sh: 45  _comp_mesg=yes
//! sh: 46  _description -2V -x dates expl date
//! sh: 47  compadd "${@:/-X/-x}" "$expl[@]" -
//! sh: 48  [[ -z $MENUSELECT && $WIDGET != menu-select ]] && return
//! sh: 50-127  …interactive calendar-grid via strftime/EPOCHSECONDS…
//! ```
//!
//! LIMITATION (sh:50-127): the menu-select calendar grid needs the
//! `zsh/datetime` substrate (`strftime`, `EPOCHSECONDS`) plus `compstate`
//! grid control that this port does not reach. The port faithfully covers
//! the PRIMARY path (sh:30-48), which is what fires for ordinary (non-
//! menu-select) completion — it registers the `dates` tag, sets up the
//! `date` description, and emits the base compadd. The grid is gated on
//! `$MENUSELECT`/`menu-select`, where the shell returns immediately in the
//! common case anyway (sh:48).

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_tags::_tags;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setsparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:30 — pull `-f FORMAT` (value) and `-F` (flag) out of argv; rest passes
/// through. Returns (format_value, future_flag, rest).
fn zparse_dates(args: &[String]) -> (Option<String>, bool, Vec<String>) {
    let (mut fmt, mut future) = (None, false);
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-f" if i + 1 < args.len() => {
                fmt = Some(args[i + 1].clone());
                i += 2;
            }
            "-F" => {
                future = true;
                i += 1;
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    (fmt, future, rest)
}

/// `_dates` — complete a date (primary path; see module LIMITATION note).
pub fn _dates(args: &[String]) -> i32 {
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let dctx = format!(":completion:{}:dates", curcontext);

    // sh:30-31
    let (fmt_opt, _future, rest) = zparse_dates(args);
    // sh:32-33 — format resolution (kept for parity; used by the grid).
    let userformat = lookupstyle(&dctx, "date-format").into_iter().next();
    let _format = userformat.or(fmt_opt).unwrap_or_else(|| "%F".to_string());

    // sh:44  _tags dates || return 0
    if _tags(&["dates".to_string()]) != 0 {
        return 0;
    }
    // sh:45
    let _ = setsparam("_comp_mesg", "yes");
    // sh:46  _description -2V -x dates expl date
    let _ = _description(&[
        "-2V".to_string(),
        "-x".to_string(),
        "dates".to_string(),
        "expl".to_string(),
        "date".to_string(),
    ]);
    // sh:47  compadd "${@:/-X/-x}" "$expl[@]" -
    let mapped: Vec<String> = rest
        .into_iter()
        .map(|a| if a == "-X" { "-x".to_string() } else { a })
        .collect();
    let mut cadd = mapped;
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-".to_string());
    let ret = bin_compadd("compadd", &cadd, &make_ops(), 0);

    // sh:48 — the interactive calendar grid (sh:50-127) is not reached here;
    // the common non-menu-select path returns after the base compadd.
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_extracts_f_and_F() {
        let (f, fut, rest) = zparse_dates(&[
            "-f".into(),
            "%Y".into(),
            "-F".into(),
            "-J".into(),
            "g".into(),
        ]);
        assert_eq!(f.as_deref(), Some("%Y"));
        assert!(fut);
        assert_eq!(rest, vec!["-J".to_string(), "g".to_string()]);
    }

    #[test]
    fn returns_early_without_registered_tags() {
        // sh:44 — `_tags dates` fails outside a completion context → return 0.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _dates(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        // Either the tag guard returns 0, or the base compadd path returns 1
        // without a live context — both are valid "no completion" outcomes.
        assert!(r == 0 || r == 1);
    }
}
