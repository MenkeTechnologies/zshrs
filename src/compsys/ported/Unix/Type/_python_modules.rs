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
//! sh:32      typeset -agU $array_name
//! sh:32      set -A $array_name $(_call_program modules $python -c ${(q)script} 2>/dev/null)
//! sh:36      _store_cache $cache_id $array_name
//! sh:37    fi
//! sh:39    _wanted modules expl module compadd "$@" -a -- $array_name
//! sh:40  }
//! sh:42  _python_modules "$@"
//! ```
//!
//! sh:21-22 the cache-policy zstyle registration is a no-op here (the
//! caching-policy predicate is a shell fn); the cache read/write path
//! (`_cache_invalid` / `_retrieve_cache` / `_store_cache`) is preserved.

use crate::compsys::ported::_cache_invalid::_cache_invalid;
use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_retrieve_cache::_retrieve_cache;
use crate::compsys::ported::_store_cache::_store_cache;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

const SCRIPT: &str =
    "import pkgutil\nfor importer, name, ispkg in pkgutil.iter_modules(): print(name)";

/// `_python_modules` — complete importable Python module names.
pub fn _python_modules(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_python_modules");
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
        // sh:34 `_call_program modules $python -c ${(q)script}` — the SCRIPT
        // is (q)-quoted, the interpreter is not. `_call_program` sh:33 evals
        // `"$argv[2,-1]"`, joining its words with spaces BEFORE parsing, so
        // handing SCRIPT over raw let eval re-split it on its own spaces and
        // embedded newline: ``(eval):2: parse error near `importer,'``.
        // Same defect as `_perl_modules` sh:86 (fixed in a18f8e671f).
        let _ = call_program_capture(&[
            "modules".to_string(),
            python.clone(),
            "-c".to_string(),
            crate::ported::utils::quotestring(SCRIPT, crate::ported::zsh_h::QT_BACKSLASH),
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
        // sh:36 — persist.
        let _ = _store_cache(&[cache_id.clone(), array_name.clone()]);
    }

    // sh:39 — _wanted modules expl module compadd "$@" -a -- $array_name
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
        // The interpreter is taken from `words[0]`. Name one that does not
        // exist, so the enumeration genuinely yields nothing.
        //
        // This used to say `python3`, and passed only BECAUSE the port handed
        // `_call_program` an unquoted SCRIPT: eval re-split it and python died
        // on ``parse error near `importer,'``, so no modules were ever found.
        // Quoting the script (as sh:34's `${(q)script}` does) makes python
        // really run and return ~2952 bytes of names, and the function then
        // correctly returns 0 — matches were added. The assertion below is
        // about the NO-CANDIDATES path, so force that condition honestly
        // instead of relying on a bug to produce it.
        setaparam("words", vec!["python3-no-such-interpreter-zzz".to_string()]);
        let r = _python_modules(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    /// sh:34 — the script reaches `_call_program` (q)-quoted.
    ///
    /// `_call_program` evals `"$argv[2,-1]"`, which JOINS its words with
    /// spaces before parsing, so a word containing raw whitespace is re-split
    /// by eval. SCRIPT holds both spaces and a newline; unquoted it produced
    /// ``(eval):2: parse error near `importer,'`` and the completer returned
    /// nothing. Pin the property that makes it survive: no unescaped
    /// whitespace remains in the word handed over.
    #[test]
    fn script_is_quoted_so_eval_cannot_resplit_it() {
        let quoted = crate::ported::utils::quotestring(
            SCRIPT,
            crate::ported::zsh_h::QT_BACKSLASH,
        );
        assert!(SCRIPT.contains(' ') && SCRIPT.contains('\n'), "fixture assumption");
        let mut prev = '\0';
        for c in quoted.chars() {
            if c.is_whitespace() {
                assert_eq!(prev, '\\', "unescaped whitespace in {quoted:?}");
            }
            prev = c;
        }
    }
}
