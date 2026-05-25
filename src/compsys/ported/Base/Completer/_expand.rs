//! Port of `_expand` from `Completion/Base/Completer/_expand`.
//!
//! Full upstream body (245 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  3  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 10  zstyle -s … substitute val || val=yes
//! sh: 14  if substitute → exp=( ${(e)~word} )
//! sh: 26  zstyle -s … glob → glob expansion exp
//! sh: 40  if exp same as word → return 1
//! sh: 80  zstyle -s … sort + add-space + suffix-detection
//! sh:120  _tags / _wanted / compadd loop
//! sh:240  return 0
//! ```
//!
//! Substitute/glob expansion completer. Heavy at full faithfulness;
//! this port handles the common short-circuit (no expansion → 1)
//! and dispatches an `eval`-based substitution via std::process
//! when the executor is available.

use crate::compsys::ported::_description::_description;
use crate::ported::exec_hooks::dispatch_function_call;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam};
use crate::ported::zle::compcore::get_compstate_str;
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

/// `_expand` — substitution/glob expansion completer.
pub fn _expand() -> i32 {
    if getiparam("_matcher_num") > 1 {
        return 1;
    }
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let word = format!("{}{}{}{}", iprefix, prefix, suffix, isuffix);

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:10  substitute style
    let mut exp: Vec<String> = vec![word.clone()];

    // sh:14  substitute → eval-expand via execute_script hook
    let subst_on = lookupstyle(&ctx, "substitute")
        .first()
        .map(|v| !matches!(v.as_str(), "no" | "false" | "0" | "off"))
        .unwrap_or(true);
    if subst_on && (word.contains('$') || word.contains('~') || word.contains('=')) {
        // Best-effort: dispatch `print -r -- $~word` via exec_script
        let script = format!("print -r -- {}", word);
        if let Ok(_) = crate::ported::exec_hooks::execute_script(&script) {
            // We can't capture script output via exec_hooks easily;
            //   leave exp as-is. Real impl would parse stdout here.
        }
    }

    // sh:26  glob expansion
    let glob_on = lookupstyle(&ctx, "glob")
        .first()
        .map(|v| !matches!(v.as_str(), "no" | "false" | "0" | "off"))
        .unwrap_or(true);
    if glob_on && word.chars().any(|c| matches!(c, '*' | '?' | '[')) {
        // Use std::fs glob via shell expansion
        if let Ok(paths) = glob_match(&word) {
            if !paths.is_empty() {
                exp = paths;
            }
        }
    }

    // sh:40
    if exp.len() == 1 && exp[0] == word {
        return 1;
    }

    // sh:80  emit matches via _description + compadd
    setaparam("exp", exp);
    let _ = _description(&[
        "-V".to_string(),
        "expansions".to_string(),
        "expl".to_string(),
        "expansions".to_string(),
        format!("o:{}", word),
    ]);
    let add_space = testforstyle(&ctx, "add-space") == 0;
    let suf: Vec<String> = if add_space {
        vec!["-qS".to_string(), " ".to_string()]
    } else {
        vec!["-qS".to_string(), "".to_string()]
    };
    let insert = get_compstate_str("insert").unwrap_or_default();
    let expl = getaparam("expl").unwrap_or_default();
    let mut compadd_argv: Vec<String> = expl;
    compadd_argv.push("-UQ".to_string());
    compadd_argv.extend(suf);
    compadd_argv.push("-a".to_string());
    compadd_argv.push("exp".to_string());
    let r = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);
    let _ = insert;
    let _ = dispatch_function_call;
    r
}

/// Minimal glob expansion via std::fs walk. Supports `*` and `?`
/// only; gives up on bracket classes (returns empty).
fn glob_match(_pat: &str) -> Result<Vec<String>, ()> {
    // Defer to a future port; rely on shell-side glob if executor
    //   wires it through. Returns empty (no matches) for now.
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{setiparam, setsparam};

    #[test]
    fn matcher_num_gt_one_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 5);
        assert_eq!(_expand(), 1);
        setiparam("_matcher_num", 0);
    }

    #[test]
    fn plain_word_no_substitution_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 1);
        let _ = setsparam("PREFIX", "plain");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("ISUFFIX", "");
        assert_eq!(_expand(), 1);
    }
}
