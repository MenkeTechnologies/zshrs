//! Port of `_history_modifiers` from
//! `Completion/Zsh/Type/_history_modifiers`.
//!
//! Full upstream body (89 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 8  local type=$1 delim expl
//! sh: 9  integer global
//! sh:11  while true; do
//! sh:12    if [[ -n $PREFIX ]]; then
//! sh:13      local char=$PREFIX[1]
//! sh:14      global=0
//! sh:15      [[ $char = g ]] && { global=1; compset -P 1 } && char=$PREFIX[1]
//! sh:17      case $char in
//! sh:18        s|S) extract delim; compset -P "[gsS]$delim*$delim*$delim"; build list of modifier flags
//! sh:80      esac
//! sh:84    else
//! sh:85      build top-level list of all modifier letters
//! sh:88    fi
//! sh:89  done
//! ```
//!
//! Emits the catalog of `:s/PAT/REPL/`-style history modifiers per
//! context. Approximation: emit the top-level letter list +
//! delegates to `_describe`.

use crate::ported::exec::dispatch_function_call;
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

/// `_history_modifiers` — complete history modifier letters.
/// `$1` is the context (`h`=history, `q`=glob qualifier,
/// `p`=parameter).
pub fn _history_modifiers(args: &[String]) -> i32 {
    let typ = args.first().cloned().unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();

    // sh:14-15  global `g` prefix swallow
    let prefix = if prefix.starts_with('g') {
        let _ = bin_compset(
            "compset",
            &["-P".to_string(), "1".to_string()],
            &make_ops(),
            0,
        );
        prefix[1..].to_string()
    } else {
        prefix
    };

    let first_char = prefix.chars().next();
    if first_char.is_some() && matches!(first_char.unwrap(), 's' | 'S') {
        // sh:18 — substitution mode, delimiter follows
        //   Defer to _describe with the standard substitution
        //   flag set (l/q/x/u, plus type-specific p/&).
        let mut flags: Vec<String> = vec![
            "l:lower case all words".to_string(),
            "q:quote words".to_string(),
            "u:upper case all words".to_string(),
        ];
        if typ == "h" {
            flags.push("p:print without executing".to_string());
            flags.push("x:quote words, breaking on whitespace".to_string());
            flags.push("&:repeat previous substitution".to_string());
        }
        setaparam("list", flags);
        let mut argv: Vec<String> = vec![
            "-t".to_string(),
            "history-modifiers".to_string(),
            "history modifier".to_string(),
            "list".to_string(),
        ];
        return dispatch_function_call("_describe", &argv).unwrap_or(1);
    }

    // sh:85 — top-level catalog
    let mut list: Vec<String> = vec![
        "h:remove last filename component".to_string(),
        "t:select tail (last filename component)".to_string(),
        "r:remove last suffix".to_string(),
        "e:select last suffix".to_string(),
        "s:substitute on the word".to_string(),
        "&:repeat previous substitution".to_string(),
        "g:apply modifier globally".to_string(),
        "l:convert words to lowercase".to_string(),
        "u:convert words to uppercase".to_string(),
        "q:quote words".to_string(),
        "x:quote breaking on whitespace".to_string(),
    ];
    if typ == "h" {
        list.push("p:print without executing".to_string());
    }
    setaparam("list", list);
    let describe_argv: Vec<String> = vec![
        "-t".to_string(),
        "history-modifiers".to_string(),
        "history modifier".to_string(),
        "list".to_string(),
    ];
    dispatch_function_call("_describe", &describe_argv).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        assert_eq!(_history_modifiers(&["h".to_string()]), 1);
    }

    #[test]
    fn publishes_list_for_describe() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = _history_modifiers(&["h".to_string()]);
        let list = crate::ported::params::getaparam("list").unwrap_or_default();
        assert!(!list.is_empty());
    }
}
