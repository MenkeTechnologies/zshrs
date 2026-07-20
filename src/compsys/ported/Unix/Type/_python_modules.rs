//! Port of `_python_modules` from `Completion/Unix/Type/_python_modules`.
//!
//! Full upstream body (42 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  _python_module_caching_policy () { newer=( "$1"(Nmw-1) ); return $#newer }
//! sh:10  _python_modules () {
//! sh:13    case $words[1] in (python*) python=$words[1];; (pydoc*) python=${words[1]/#pydoc/python};; (*) python="python";; esac
//! sh:18    local cache_id=${${python//[^[:alnum:]]/_}#_}_modules
//! sh:19    local array_name=_${cache_id}
//! sh:21    zstyle -s … cache-policy update_policy
//! sh:22    [[ -z $update_policy ]] && zstyle … cache-policy _python_module_caching_policy
//! sh:25    if ( [[ ${(P)+array_name} -eq 0 ]] || _cache_invalid $cache_id ) && ! _retrieve_cache $cache_id; then
//! sh:28      local script='import pkgutil\nfor …: print(name)'
//! sh:31      typeset -agU $array_name
//! sh:32      set -A $array_name $(_call_program modules $python -c ${(q)script} 2>/dev/null)
//! sh:34      _store_cache $cache_id $array_name
//! sh:35    fi
//! sh:37    _wanted modules expl module compadd "$@" -a -- $array_name
//! sh:38  }
//! sh:40  _python_modules "$@"
//! ```
//!
//! sh:21-22 the cache-policy zstyle registration is a no-op here (the
//! caching-policy predicate is a shell fn); the cache read/write path
//! (`_cache_invalid` / `_retrieve_cache` / `_store_cache`) is preserved.

use crate::compsys::ported::_cache_invalid::_cache_invalid;
use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_retrieve_cache::_retrieve_cache;
use crate::compsys::ported::_store_cache::_store_cache;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

const SCRIPT: &str =
    "import pkgutil\nfor importer, name, ispkg in pkgutil.iter_modules(): print(name)";

/// `_python_modules` — complete importable Python module names.
pub fn _python_modules(args: &[String]) -> i32 {
    // sh:13 — pick the interpreter from the command word.
    let cmd = getaparam("words")
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default();
    let python = if cmd.starts_with("python") {
        cmd.clone()
    } else if let Some(rest) = cmd.strip_prefix("pydoc") {
        format!("python{}", rest)
    } else {
        "python".to_string()
    };

    // sh:18-19 — cache id / backing array name.
    let sanitized: String = python
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let cache_id = format!("{}_modules", sanitized.trim_start_matches('_'));
    let array_name = format!("_{}", cache_id);

    // sh:25-35 — populate the cache if missing/invalid and not retrievable.
    let present = getaparam(&array_name).is_some();
    if (!present || _cache_invalid(&[cache_id.clone()]) != 0)
        && _retrieve_cache(&[cache_id.clone()]) != 0
    {
        // sh:32 — set -A $array_name $(python -c $script)
        let _ = _call_program(&[
            "modules".to_string(),
            python.clone(),
            "-c".to_string(),
            SCRIPT.to_string(),
        ]);
        let mods: Vec<String> = getsparam("REPLY")
            .unwrap_or_default()
            .split_whitespace()
            .map(String::from)
            .collect();
        // typeset -U — unique, preserving first-seen order.
        let mut seen: Vec<String> = Vec::with_capacity(mods.len());
        for m in mods {
            if !seen.contains(&m) {
                seen.push(m);
            }
        }
        setaparam(&array_name, seen);
        // sh:34 — persist.
        let _ = _store_cache(&[cache_id.clone(), array_name.clone()]);
    }

    // sh:37 — _wanted modules expl module compadd "$@" -a -- $array_name
    let mut wanted_argv: Vec<String> = vec![
        "modules".to_string(),
        "expl".to_string(),
        "module".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-a".to_string());
    wanted_argv.push("--".to_string());
    wanted_argv.push(array_name);
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        setaparam("words", vec!["python3".to_string()]);
        let r = _python_modules(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
