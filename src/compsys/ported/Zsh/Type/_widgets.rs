//! Port of `_widgets` from `Completion/Zsh/Type/_widgets`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl pattern
//! sh: 4
//! sh: 5  pattern=( -g \* )
//! sh: 6  zparseopts -D -K -E g:=pattern
//! sh: 7
//! sh: 8  _description widgets expl widget
//! sh: 9  compadd "$@" "$expl[@]" -M 'r:|-=* r:|=*' - "${(@k)widgets[(R)${pattern[2]}]}"
//! ```
//!
//! `(@k)widgets[(R)pat]` — keys of the `widgets` assoc-array whose
//! VALUE matches `pat`. The assoc lives in `thingytab` (real ZLE
//! widget registry at `src/ported/zle/zle_thingy.rs:1891`). Each
//! entry's value-string follows zsh's convention:
//!   `builtin`               — internal widget
//!   `user:<fnname>`         — user-defined widget bound to fn
//!   `completion:<wid>:<fn>` — completion widget

use crate::compsys::ported::_description::_description;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, setaparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zle::zle_h::WidgetImpl;
use crate::ported::zle::zle_thingy::thingytab;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:6 — bridge to real `bin_zparseopts -D -K -E g:=pattern` via
/// `-v <name>`. `g` takes a value, accumulated into `pattern`
/// alongside the `-g` literal.
fn run_zparseopts_widgets(
    args: &[String],
    seed_pattern: Vec<String>,
) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("pattern", seed_pattern);
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-K".to_string(),
            "-E".to_string(),
            "-v".to_string(),
            src.to_string(),
            "g:=pattern".to_string(),
        ],
        &make_ops(),
        0,
    );
    let pattern = getaparam("pattern").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, pattern)
}

/// `_widgets` — complete ZLE widget names. `-g <pat>` (default `*`)
/// filters by widget value-string (e.g. `user:*` for user widgets).
pub fn _widgets(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_widgets");
    // sh:5
    let seed = vec!["-g".to_string(), "*".to_string()];
    // sh:6
    let (argv, pattern_arr) = run_zparseopts_widgets(args, seed);
    // `pattern_arr[1]` is the value of -g (after the literal -g).
    let pat = pattern_arr
        .get(1)
        .cloned()
        .unwrap_or_else(|| "*".to_string());

    // Collect widget keys whose value-string matches the pattern.
    let prog = patcompile(
        &{
            let mut __pat_tok = (&pat).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        0,
        None,
    );
    let mut keys: Vec<String> = Vec::new();
    if let Ok(tab) = thingytab().lock() {
        for (name, t) in tab.iter() {
            let value = match t.widget.as_ref() {
                None => continue,
                Some(w) => match &w.u {
                    WidgetImpl::UserFunc(fn_name) => format!("user:{}", fn_name),
                    WidgetImpl::Comp { wid, func, .. } => {
                        format!("completion:{}:{}", wid, func)
                    }
                    WidgetImpl::Internal(_) => "builtin".to_string(),
                },
            };
            let matches = match prog.as_ref() {
                Some(p) => pattry(p, &value),
                None => pat == value,
            };
            if matches {
                keys.push(name.clone());
            }
        }
    }
    keys.sort();

    // sh:8
    let _ = _description(&[
        "widgets".to_string(),
        "expl".to_string(),
        "widget".to_string(),
    ]);

    // sh:9  compadd "$@" "$expl[@]" -M 'r:|-=* r:|=*' - <names>
    let expl = getaparam("expl").unwrap_or_default();
    let mut compadd_argv: Vec<String> = argv;
    compadd_argv.extend(expl);
    compadd_argv.push("-M".to_string());
    compadd_argv.push("r:|-=* r:|=*".to_string());
    compadd_argv.push("-".to_string());
    compadd_argv.extend(keys);
    bin_compadd("compadd", &compadd_argv, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_compadd_status() {
        // bin_compadd without active completion returns 1 (no
        //   matches added); when INCOMPFUNC=1 it accepts the call.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _r = _widgets(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }
}
