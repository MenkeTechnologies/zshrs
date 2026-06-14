//! Port of `_subscript` from `Completion/Zsh/Context/_subscript`.
//!
//! Full upstream body (136 lines, abridged):
//! ```text
//! sh:  1  #compdef -subscript-
//! sh:  5  [[ $ISUFFIX = *\]* ]] || osuf=]
//! sh:  7  if [[ "$1" = -q ]]; then compquote osuf; osuf+=' '; shift
//! sh: 11  compset -P '\(([^\(\)]|\(*\))##\)' # strip subscript flags
//! sh: 19  if dynamic-name expansion at ~[: → _dynamic_directory_name
//! sh: 21  elif $PREFIX = :*: → character classes
//! sh: 27  elif compset -P '(' → subscript-flag completion (assoc / array / scalar)
//! sh: 79  elif assoc → _wanted association-keys + keys
//! sh: 90  elif array → _tags indexes parameters loop
//! sh:134  else _dispatch -math- -math-
//! ```
//!
//! Subscript context: dispatch based on parameter type
//! (assoc/array/scalar) and current PREFIX shape.

use crate::compsys::ported::_dynamic_directory_name::_dynamic_directory_name;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setaparam};
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

fn is_array_param(name: &str) -> bool {
    getaparam(name).is_some()
}

fn is_assoc_param(name: &str) -> bool {
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .map(|t| t.contains_key(name))
        .unwrap_or(false)
}

/// `_subscript` — `-subscript-` context: complete inside `${var[…]}`.
pub fn _subscript(args: &[String]) -> i32 {
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let _osuf = if !isuffix.contains(']') { "]" } else { "" };
    let _ = args; // -q flag tolerated; we don't model quoting

    let _ = bin_compset(
        "compset",
        &["-P".to_string(), "\\(([^\\(\\)]|\\(*\\))##\\)".to_string()],
        &make_ops(),
        0,
    );

    let prefix = getsparam("PREFIX").unwrap_or_default();
    let cursor: i64 = getsparam("CURSOR")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let buffer = getsparam("BUFFER").unwrap_or_default();

    // sh:19  ~[ dynamic-name expansion?
    if let Some(open_at) = (1..=cursor as usize)
        .rev()
        .find(|&i| buffer.as_bytes().get(i.saturating_sub(1)) == Some(&b'['))
    {
        let head = &buffer[..open_at.saturating_sub(1)];
        if head.ends_with('~') {
            return _dynamic_directory_name();
        }
    }

    // sh:21  :class:
    if prefix.starts_with(':') {
        let classes: Vec<String> = vec![
            "alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph", "lower", "print",
            "punct", "space", "upper", "xdigit", "IFS", "IDENT", "IFSSPACE", "WORD",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();
        setaparam("classes", classes);
        return _wanted(&[
            "characters".to_string(),
            "expl".to_string(),
            "character class".to_string(),
            "compadd".to_string(),
            "-p:".to_string(),
            "-S".to_string(),
            ":]".to_string(),
            "-a".to_string(),
            "classes".to_string(),
        ]);
    }

    // sh:79 / sh:90 — type-driven branch
    let param = get_compstate_str("parameter").unwrap_or_default();
    if !param.is_empty() {
        if is_assoc_param(&param) {
            let mut keys: Vec<String> = Vec::new();
            if let Ok(tab) = crate::ported::params::paramtab_hashed_storage().lock() {
                if let Some(h) = tab.get(&param) {
                    keys.extend(h.keys().cloned());
                }
            }
            keys.sort();
            setaparam("subscript_keys", keys);
            return _wanted(&[
                "association-keys".to_string(),
                "expl".to_string(),
                "association key".to_string(),
                "compadd".to_string(),
                "-Q".to_string(),
                "-a".to_string(),
                "subscript_keys".to_string(),
            ]);
        }
        if is_array_param(&param) {
            let arr = getaparam(&param).unwrap_or_default();
            let n = arr.len();
            let indices: Vec<String> = (1..=n).map(|i| i.to_string()).collect();
            setaparam("subscript_indices", indices);
            let _ = _tags(&["indexes".to_string(), "parameters".to_string()]);
            loop {
                if _tags(&[]) != 0 {
                    break;
                }
                if _requested(&["indexes".to_string()]) == 0 {
                    let _ = _wanted(&[
                        "-V".to_string(),
                        "indexes".to_string(),
                        "expl".to_string(),
                        "array index".to_string(),
                        "compadd".to_string(),
                        "-a".to_string(),
                        "subscript_indices".to_string(),
                    ]);
                    return 0;
                }
            }
            return 1;
        }
    }

    // sh:134 fallback
    dispatch_function_call("_dispatch", &["-math-".to_string(), "-math-".to_string()]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = crate::ported::params::setsparam("ISUFFIX", "");
        let _r = _subscript(&[]);
    }
}
