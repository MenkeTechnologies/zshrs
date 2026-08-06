//! Port of `_file_modes` from `Completion/Unix/Type/_file_modes`.
//!
//! Full upstream body (37 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  local -a … copts=( "${@}" ) privs
//! sh: 8  privs=( 'r[read]' 'w[write]' 'x[execute]' 's[set uid/gid]'
//! sh:        't[sticky]' 'X[…]' 'u[…]' 'g[…]' 'o[…]' )
//! sh:17  [[ $OSTYPE == solaris* ]] && privs+=( 'l[mandatory locking]' )
//! sh:20  compset -P '*,'
//! sh:21  compset -S ',*'
//! sh:23  if [[ -prefix [0-7] ]]; then
//! sh:24    _message -e number 'numeric mode'
//! sh:25  elif compset -P '[a-z]#[+-=]'; then
//! sh:26    _values -O copts -S '' privilege $privs && return 0
//! sh:27  else
//! sh:28    compset -P '*'
//! sh:29    copts=( -S '' )
//! sh:30    _alternative -O copts \
//! sh:31      'who:who:((a\:all u\:owner g\:group o\:others))' \
//! sh:32      'operators:operator:(+ - =)' \
//! sh:33    && return 0
//! sh:34  fi
//! sh:36  return 1
//! ```

use crate::compsys::ported::_alternative::alternative_byname;
use crate::compsys::ported::_message::message_byname;
use crate::compsys::ported::_values::_values;
use crate::ported::params::{getsparam, setaparam};
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

fn compset(argv: &[&str]) -> i32 {
    let v: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_compset("compset", &v, &make_ops(), 0)
}

/// `_file_modes` — complete symbolic / numeric file mode specs
/// (`chmod`-style: `u+rwx`, `0755`, …).
pub fn _file_modes(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_file_modes");
    // sh:8-17 — privilege letters.
    let mut privs: Vec<String> = [
        "r[read]",
        "w[write]",
        "x[execute]",
        "s[set uid/gid]",
        "t[sticky]",
        "X[execute only if directory or executable to another]",
        "u[owner's current permissions]",
        "g[group's current permissions]",
        "o[others' current permissions]",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if getsparam("OSTYPE")
        .unwrap_or_default()
        .starts_with("solaris")
    {
        privs.push("l[mandatory locking]".to_string());
    }

    // sh:20-21
    let _ = compset(&["-P", "*,"]);
    let _ = compset(&["-S", ",*"]);

    // sh:23 — [[ -prefix [0-7] ]]: PREFIX begins with an octal digit.
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if prefix.starts_with(|c: char| ('0'..='7').contains(&c)) {
        // sh:24
        let _ = message_byname(&[
            "-e".to_string(),
            "number".to_string(),
            "numeric mode".to_string(),
        ]);
        return 1;
    }

    // sh:25 — compset -P '[a-z]#[+-=]' consumed a `u+`/`g-`/`=`… prefix.
    if compset(&["-P", "[a-z]#[+-=]"]) == 0 {
        // sh:26 — copts stays the passed-through args.
        setaparam("copts", args.to_vec());
        let mut v: Vec<String> = vec![
            "-O".to_string(),
            "copts".to_string(),
            "-S".to_string(),
            "".to_string(),
            "privilege".to_string(),
        ];
        v.extend(privs);
        // By NAME so `_values` gets its own `comp_wrapper` frame (c:1556):
        // it rewrites PREFIX/SUFFIX/IPREFIX (`_values.rs:235-283`) and sets
        // `compstate[restore]=''` (`_values.rs:388`), both of which must be
        // undone when it returns rather than leaking through this function.
        if crate::compsys::ported::shared::call_compfn("_values", &v, || _values(&v)) == 0 {
            return 0;
        }
    } else {
        // sh:28-33
        let _ = compset(&["-P", "*"]);
        setaparam("copts", vec!["-S".to_string(), "".to_string()]);
        let r = alternative_byname(&[
            "-O".to_string(),
            "copts".to_string(),
            "who:who:((a\\:all u\\:owner g\\:group o\\:others))".to_string(),
            "operators:operator:(+ - =)".to_string(),
        ]);
        if r == 0 {
            return 0;
        }
    }

    // sh:36
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam_reset();
        assert_eq!(_file_modes(&[]), 1);
    }

    fn setsparam_reset() {
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = crate::ported::params::setsparam("OSTYPE", "linux-gnu");
    }
}
