//! Port of `_globflags` from `Completion/Zsh/Type/_globflags`.
//!
//! Full upstream body (62 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 6  local ret=1
//! sh: 7  local -a flags
//! sh: 8  local preprefix=$IPREFIX
//! sh:10  compset -P '([ilIUubBmMcq]|a(|<->))##'
//! sh:12  preprefix=${IPREFIX[$#preprefix,-1]}
//! sh:11  if [[ $preprefix = *\#q* ]]; then _globquals; return; fi
//! sh:13  elif [[ $preprefix = *q* ]]; then _message 'q flag has to be specified by itself'; return; fi
//! sh:15  elif [[ $preprefix = *a(|<->) ]]; then _message -e number 'errors'; if a alone return else compset -P '<->'
//! sh:21  elif [[ $preprefix = *\#c ]]; then _message -e range 'repetitions (min,max) or (exact)'; return
//! sh:24  flags=( i l I s e U u ); add b B m M when condition context
//! sh:38  filter out already-used flags via :#[...] pattern
//! sh:40  if [[ $IPREFIX != *# ]]; then flags=( ${flags:#[se]*} ); fi
//! sh:50  _describe -t globflags "glob flag" flags -Q -S ')' && ret=0
//! sh:44  flags=( a q c ); filter; _describe -t globflags … -S '' && ret=0
//! sh:62  return ret
//! ```

use crate::compsys::ported::_message::_message;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setaparam};
use crate::ported::zle::compcore::get_compstate_str;
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

/// `_globflags` — complete `(#x)` glob-flag characters inside a
/// glob expression.
pub fn _globflags() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_globflags");
    let preprefix_initial = getsparam("IPREFIX").unwrap_or_default();
    let mut ret: i32 = 1;

    // sh:10
    let _ = bin_compset(
        "compset",
        &["-P".to_string(), "([ilIUubBmMcq]|a(|<->))##".to_string()],
        &make_ops(),
        0,
    );
    // sh:12  preprefix = portion of IPREFIX consumed by compset
    let new_iprefix = getsparam("IPREFIX").unwrap_or_default();
    let preprefix = if new_iprefix.len() >= preprefix_initial.len() {
        new_iprefix[preprefix_initial.len()..].to_string()
    } else {
        String::new()
    };

    // sh:11
    if preprefix.contains("#q") {
        return dispatch_function_call("_globquals", &[]).unwrap_or(1);
    }
    // sh:13
    if preprefix.contains('q') {
        return _message(&["q flag has to be specified by itself".to_string()]);
    }
    // sh:15
    if preprefix.starts_with('a') || preprefix.contains(|c: char| c == 'a') {
        // approximation: trailing `a` or `a<digits>`
        let tail_a_only = preprefix == "a";
        let tail_a_count =
            preprefix.starts_with('a') && preprefix[1..].chars().all(|c| c.is_ascii_digit());
        if tail_a_only || tail_a_count {
            let _ = _message(&["-e".to_string(), "number".to_string(), "errors".to_string()]);
            if tail_a_only {
                return 0;
            } else {
                let _ = bin_compset(
                    "compset",
                    &["-P".to_string(), "<->".to_string()],
                    &make_ops(),
                    0,
                );
            }
        }
    }
    // sh:21
    if preprefix.ends_with("#c") {
        return _message(&[
            "-e".to_string(),
            "range".to_string(),
            "repetitions (min,max) or (exact)".to_string(),
        ]);
    }

    // sh:24
    let mut flags: Vec<&str> = vec![
        "i:case insensitive",
        "l:lower case characters match uppercase",
        "I:case sensitive matching",
        "s:match start of string",
        "e:match end of string",
        "U:consider all characters to be one byte",
        "u:support multibyte characters in pattern",
    ];
    let context = get_compstate_str("context").unwrap_or_default();
    if context == "condition" {
        flags.extend(&[
            "b:activate backreferences",
            "B:deactivate backreferences",
            "m:set reference to entire matched data",
            "M:deactivate m flag",
        ]);
    }

    // sh:38  filter out flag letters already used
    let used: std::collections::HashSet<char> = preprefix.chars().collect();
    let mut emit: Vec<String> = flags
        .iter()
        .filter(|f| {
            f.chars()
                .next()
                .map(|c| !used.contains(&c))
                .unwrap_or(false)
        })
        .map(|s| s.to_string())
        .collect();
    // sh:40
    if !getsparam("IPREFIX").unwrap_or_default().ends_with('#') {
        emit.retain(|f| {
            let c = f.chars().next().unwrap_or(' ');
            c != 's' && c != 'e'
        });
    }

    // sh:50
    setaparam("flags", emit.clone());
    let mut describe_argv: Vec<String> = vec![
        "-t".to_string(),
        "globflags".to_string(),
        "glob flag".to_string(),
        "flags".to_string(),
        "-Q".to_string(),
        "-S".to_string(),
        ")".to_string(),
    ];
    if dispatch_function_call("_describe", &describe_argv).unwrap_or(1) == 0 {
        ret = 0;
    }

    // sh:44  qualifier-only flags
    let mut flags2: Vec<&str> = vec![
        "a:approximate matching",
        "q:introduce glob qualifier",
        "c:match repetitions of preceding pattern",
    ];
    if !getsparam("IPREFIX").unwrap_or_default().ends_with('#') {
        flags2.retain(|f| {
            let c = f.chars().next().unwrap_or(' ');
            c != 'c' && c != 'q'
        });
    }
    let emit2: Vec<String> = flags2
        .iter()
        .filter(|f| {
            f.chars()
                .next()
                .map(|c| !used.contains(&c))
                .unwrap_or(false)
        })
        .map(|s| s.to_string())
        .collect();
    setaparam("flags", emit2);
    describe_argv = vec![
        "-t".to_string(),
        "globflags".to_string(),
        "glob flag".to_string(),
        "flags".to_string(),
        "-Q".to_string(),
        "-S".to_string(),
        "".to_string(),
    ];
    if dispatch_function_call("_describe", &describe_argv).unwrap_or(1) == 0 {
        ret = 0;
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("IPREFIX", "");
        assert_eq!(_globflags(), 1);
    }
}
