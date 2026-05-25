//! Port of `_arguments` from
//! `Completion/Base/Utility/_arguments`.
//!
//! Full upstream body (589 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh: 10  flag-parse: -A/-M/-O/-C/-R/-S/-W/-n/-s/-w …
//! sh: 30  long=$argv[(I)--]; build long-option cache from -i/-s if --
//! sh:200  parse the spec list into per-flag descriptors
//! sh:300  classify current word position (option vs positional)
//! sh:400  dispatch the matching spec's action
//! sh:589  return ret
//! ```
//!
//! `_arguments` is the workhorse spec engine used by hundreds of
//! per-command completion functions. Full faithful port requires a
//! dense spec parser; this version covers the common forms:
//!   * `-flag[description]:msg:action`        (option with arg)
//!   * `-flag[description]`                    (boolean option)
//!   * `N:msg:action`                          (positional)
//!   * `*:msg:action`                          (rest)
//! Plus `:` as separator inside option args. Action forms accepted:
//!   * `_files` / `_users` / etc.   (bare shell-fn dispatch)
//!   * `(val1 val2 val3)`            (literal list)
//!   * `->state`                     (sets $state for caller)

use crate::compsys::ported::_alternative::_alternative;
use crate::compsys::ported::_message::_message;
use crate::ported::exec_hooks::dispatch_function_call;
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam, setsparam};

/// Spec classification.
#[derive(Debug, Clone)]
enum Spec {
    /// `-flag[desc]:msg:action` or bare `-flag[desc]`
    Option {
        flag: String,
        desc: String,
        msg: Option<String>,
        action: Option<String>,
    },
    /// `N:msg:action` — N is a positional index (1-based)
    Positional {
        idx: usize,
        msg: String,
        action: String,
    },
    /// `*:msg:action` — rest
    Rest { msg: String, action: String },
}

/// Parse one spec string into a `Spec` enum.
fn parse_spec(s: &str) -> Option<Spec> {
    if let Some(rest) = s.strip_prefix('-') {
        // Option spec. Look for `[desc]` brace.
        let (flag, rest2) = if let Some(open) = rest.find('[') {
            let close = rest[open..].rfind(']').map(|c| c + open)?;
            let flag = rest[..open].to_string();
            let desc_part = &rest[open + 1..close];
            (flag, &rest[close + 1..])
        } else {
            // bare `-flag`
            let end = rest.find(':').unwrap_or(rest.len());
            (rest[..end].to_string(), &rest[end..])
        };
        // After `[desc]`, look for `:msg:action`
        let after_brace = rest2;
        if after_brace.is_empty() {
            return Some(Spec::Option {
                flag: format!("-{}", flag),
                desc: String::new(),
                msg: None,
                action: None,
            });
        }
        let mut parts = after_brace.trim_start_matches(':').splitn(2, ':');
        let msg = parts.next().map(|s| s.to_string());
        let action = parts.next().map(|s| s.to_string());
        return Some(Spec::Option {
            flag: format!("-{}", flag),
            desc: String::new(),
            msg,
            action,
        });
    }
    if let Some(rest) = s.strip_prefix('*') {
        let body = rest.trim_start_matches(':');
        let mut parts = body.splitn(2, ':');
        let msg = parts.next().unwrap_or("").to_string();
        let action = parts.next().unwrap_or("").to_string();
        return Some(Spec::Rest { msg, action });
    }
    // N:msg:action
    let mut parts = s.splitn(3, ':');
    let idx_str = parts.next().unwrap_or("");
    let idx: usize = idx_str.parse().ok()?;
    let msg = parts.next().unwrap_or("").to_string();
    let action = parts.next().unwrap_or("").to_string();
    Some(Spec::Positional { idx, msg, action })
}

/// Dispatch the action portion of a spec.
fn dispatch_action(action: &str, msg: &str) -> i32 {
    let action = action.trim();
    if action.is_empty() {
        return _message(&["-e".to_string(), msg.to_string()]);
    }
    // sh:240 — `(literal list)` form
    if action.starts_with('(') && action.ends_with(')') {
        let body = &action[1..action.len() - 1];
        let items: Vec<String> = body.split_whitespace().map(|s| s.to_string()).collect();
        if items.is_empty() {
            return _message(&["-e".to_string(), msg.to_string()]);
        }
        setaparam("_arguments_action_list", items);
        return crate::ported::zle::complete::bin_compadd(
            "compadd",
            &[
                "-X".to_string(),
                msg.to_string(),
                "-a".to_string(),
                "_arguments_action_list".to_string(),
            ],
            &crate::ported::zsh_h::options {
                ind: [0u8; crate::ported::zsh_h::MAX_OPS],
                args: Vec::new(),
                argscount: 0,
                argsalloc: 0,
            },
            0,
        );
    }
    // `->state` form — set $state for caller
    if let Some(state) = action.strip_prefix("->") {
        let _ = setsparam("state", state);
        return 0;
    }
    // Bare shell-fn dispatch (e.g. `_files`, `_users`)
    let parts: Vec<String> = action.split_whitespace().map(|s| s.to_string()).collect();
    if let Some((cmd, rest)) = parts.split_first() {
        return dispatch_function_call(cmd, rest).unwrap_or(1);
    }
    1
}

/// `_arguments` — primary spec-engine entry. Args after any flag
/// section are spec strings.
pub fn _arguments(args: &[String]) -> i32 {
    // sh:10 — flag-parse
    let mut idx = 0usize;
    let mut subopts: Vec<String> = Vec::new();
    let mut use_cc = false;
    while idx < args.len() {
        let a = &args[idx];
        match a.as_str() {
            "-0" | "-R" | "-S" | "-W" | "-n" | "-w" => idx += 1,
            "-s" => idx += 1,
            "-C" => {
                use_cc = true;
                idx += 1;
            }
            "-O" if idx + 1 < args.len() => {
                subopts = getaparam(&args[idx + 1]).unwrap_or_default();
                idx += 2;
            }
            "-A" | "-M" if idx + 1 < args.len() => idx += 2,
            s if s.starts_with('-')
                && s.len() > 1
                && matches!(
                    s.chars().nth(1).unwrap_or(' '),
                    '0' | 'A' | 'C' | 'M' | 'O' | 'R' | 'S' | 'W' | 'n' | 's' | 'w'
                ) =>
            {
                idx += 1;
            }
            ":" => {
                idx += 1;
                break;
            }
            _ => break,
        }
    }
    let _ = subopts;
    let _ = use_cc;

    // Parse remaining args into specs (drop everything from `--` on
    //   — that's the long-option subsection we don't model fully).
    let mut specs: Vec<Spec> = Vec::new();
    while idx < args.len() {
        let a = &args[idx];
        if a == "--" {
            break;
        }
        if let Some(s) = parse_spec(a) {
            specs.push(s);
        }
        idx += 1;
    }

    // Determine current word + position
    let words = getaparam("words").unwrap_or_default();
    let current = getiparam("CURRENT") as usize;
    let curword = if current >= 1 && current <= words.len() {
        words[current - 1].clone()
    } else {
        String::new()
    };

    // sh:300 — if current word starts with `-`, dispatch option-spec
    if curword.starts_with('-') {
        // First emit the option catalog via _describe-style; or
        //   when curword fully matches a flag, dispatch its action.
        let mut option_strs: Vec<String> = Vec::new();
        let mut matched_action: Option<(String, String)> = None;
        for s in &specs {
            if let Spec::Option { flag, msg, action, .. } = s {
                if curword == *flag {
                    if let (Some(m), Some(a)) = (msg.as_ref(), action.as_ref()) {
                        matched_action = Some((m.clone(), a.clone()));
                    }
                    break;
                }
                option_strs.push(format!(
                    "{}:{}",
                    flag,
                    msg.as_deref().unwrap_or("")
                ));
            }
        }
        if let Some((m, a)) = matched_action {
            return dispatch_action(&a, &m);
        }
        if !option_strs.is_empty() {
            setaparam("_arguments_options", option_strs);
            let mut alt_args: Vec<String> =
                vec!["options:option:_arguments_options".to_string()];
            let _ = alt_args.split_off(1);
            return _alternative(&alt_args);
        }
    }

    // sh:400  positional dispatch
    // Compute the positional index = CURRENT - 1 (subtract command name)
    let pos = if current > 1 { current - 1 } else { 1 };
    for s in &specs {
        match s {
            Spec::Positional { idx: i, msg, action } if *i == pos => {
                return dispatch_action(action, msg);
            }
            _ => {}
        }
    }
    // Fall back to `*:` rest spec
    for s in &specs {
        if let Spec::Rest { msg, action } = s {
            return dispatch_action(action, msg);
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn empty_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_arguments(&[]), 1);
    }

    #[test]
    fn parses_option_spec() {
        let s = parse_spec("-v[verbose mode]").unwrap();
        match s {
            Spec::Option { flag, .. } => assert_eq!(flag, "-v"),
            _ => panic!("expected Option"),
        }
    }

    #[test]
    fn parses_positional_spec() {
        let s = parse_spec("1:filename:_files").unwrap();
        match s {
            Spec::Positional { idx, msg, action } => {
                assert_eq!(idx, 1);
                assert_eq!(msg, "filename");
                assert_eq!(action, "_files");
            }
            _ => panic!("expected Positional"),
        }
    }

    #[test]
    fn parses_rest_spec() {
        let s = parse_spec("*:argument:_files").unwrap();
        match s {
            Spec::Rest { msg, action } => {
                assert_eq!(msg, "argument");
                assert_eq!(action, "_files");
            }
            _ => panic!("expected Rest"),
        }
    }

    #[test]
    fn positional_dispatch_with_no_executor_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("CURRENT", "2");
        setaparam("words", vec!["cmd".to_string(), "".to_string()]);
        let r = _arguments(&["1:file:_files".to_string()]);
        assert_eq!(r, 1);
    }
}
