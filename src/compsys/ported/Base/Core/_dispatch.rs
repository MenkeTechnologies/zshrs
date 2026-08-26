//! Port of `_dispatch` from `Completion/Base/Core/_dispatch`.
//!
//! Full upstream body (91 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local comp pat val name i ret=1 _compskip="$_compskip"
//! sh: 4  local curcontext="$curcontext" service str noskip
//! sh:11  if [[ "$1" = -s ]]; then noskip=yes; shift; fi
//! sh:14  [[ -z "$noskip" ]] && _compskip=
//! sh:16  curcontext="${curcontext%:*:*}:${1}:"
//! sh:18  shift
//! sh:22  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! sh:24    for str in "$@"; do
//! sh:26      service="${_services[$str]:-$str}"
//! sh:26      for i in "${(@)_patcomps[(K)$str]}"; do … patterns dispatch …
//! sh:38  fi
//! sh:45  ret=1
//! sh:44  for str in "$@"; do … look up $_comps[$str] …
//! sh:57  done
//! sh:61  if [[ -n "$comp" && "$name" != "${argv[-1]}" ]]; then
//! sh:62    _compskip=patterns
//! sh:63    eval "$comp" && ret=0
//! sh:67  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! sh:67    for str; do … _postpatcomps …
//! sh:84  [[ "$name" = "${argv[-1]}" && -n "$comp" &&
//! sh:85     "$_compskip" != (all|*default*) ]] &&
//! sh:86    service="${_services[$name]:-$name}" &&
//! sh:87     eval "$comp" && ret=0
//! sh:89  _compskip=''
//! sh:91  return ret
//! ```

use crate::ported::params::{getsparam, setsparam};

// sh:31 / sh:63 / sh:76 / sh:87 — `eval "$comp"`. The shared port of that
// construct lives in `compsys::ported::shared` because `_normal` sh:32 runs
// the identical statement; see its doc comment for the C citations.
use crate::compsys::ported::shared::eval_comp;

/// Helper: assoc lookup in flat key/value layout.
/// sh:26 — zsh `(K)pat` key-pattern match. Uses the real
/// `crate::ported::pattern` engine (`patcompile` + `pattry`).
/// Falls back to literal equality when the key isn't a valid
/// glob pattern.
fn pattern_match(pat: &str, s: &str) -> bool {
    match crate::ported::pattern::patcompile(
        &{
            let mut __pat_tok = (pat).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        0,
        None,
    ) {
        Some(prog) => crate::ported::pattern::pattry(&prog, s),
        None => pat == s,
    }
}

/// Single-key read of an associative array (`$assoc[key]`).
///
/// `_comps` / `_services` / `_patcomps` are PM_HASHED assocs.
/// `getaparam` only returns PM_ARRAY params (it yields `None` for a
/// hash — c:Src/params.c:3108 `PM_TYPE(...) == PM_ARRAY`), so the prior
/// `getaparam(name).chunks(2)` layout was always empty: `$_comps[cat]`
/// came back "" and `_dispatch` never `eval`'d the command's completer,
/// so compsys produced no matches. Read the hash backing directly, the
/// same store `_subscript` / `is_assoc_param` use.
fn assoc_get(name: &str, key: &str) -> Option<String> {
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .ok()?
        .get(name)?
        .get(key)
        .cloned()
}

/// Flatten an assoc into `[k, v, k, v, …]` in insertion order, for the
/// `${(@)_patcomps[(K)$str]}` / `_postpatcomps` pattern-dispatch walks.
/// Same PM_HASHED-vs-getaparam issue as `assoc_get`.
fn assoc_flat(name: &str) -> Vec<String> {
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .ok()
        .and_then(|t| {
            t.get(name)
                .map(|h| h.iter().flat_map(|(k, v)| [k.clone(), v.clone()]).collect())
        })
        .unwrap_or_default()
}

/// `_dispatch` — central context dispatcher. `-s` skips the
/// `_compskip` reset; first non-flag arg becomes the curcontext
/// field; remaining args are tried in order against `$_comps`.
pub fn _dispatch(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_dispatch");
    // sh:3  local comp pat val name i ret=1 _compskip="$_compskip"
    // sh:4  local curcontext="$curcontext" service str noskip
    // sh:5  local -a match mbegin mend
    {
        use crate::compsys::ported::shared::{
            declare_locals, declare_locals_keeping_value, PM_ARRAY,
        };
        declare_locals(
            &["comp", "pat", "val", "name", "i", "ret", "str", "noskip"],
            0,
        );
        declare_locals_keeping_value(&["_compskip", "curcontext"]);
        declare_locals(&["service"], 0);
        declare_locals(&["match", "mbegin", "mend"], PM_ARRAY);
    }
    let mut ret: i32 = 1;
    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let saved_compskip = getsparam("_compskip").unwrap_or_default();
    let saved_service = getsparam("service").unwrap_or_default();

    let mut argv = args.to_vec();
    // sh:11
    let mut noskip = false;
    if argv.first().map(|s| s == "-s").unwrap_or(false) {
        noskip = true;
        argv.remove(0);
    }
    // sh:14
    if !noskip {
        let _ = setsparam("_compskip", "");
    }
    // sh:16-17
    if argv.is_empty() {
        return 1;
    }
    let ctx_tail = argv.remove(0);
    // Replace last two `:`-fields of curcontext with `:${1}:`
    let mut curcontext = saved_curcontext.clone();
    if let Some(i) = curcontext.rfind(':') {
        if let Some(j) = curcontext[..i].rfind(':') {
            curcontext.truncate(j);
        }
    }
    curcontext.push(':');
    curcontext.push_str(&ctx_tail);
    curcontext.push(':');
    let _ = setsparam("curcontext", &curcontext);

    // sh:21-39  patterns pass
    let compskip = getsparam("_compskip").unwrap_or_default();
    if !(compskip == "all" || compskip.contains("patterns")) {
        for str_arg in &argv {
            if str_arg.is_empty() {
                continue;
            }
            let service = assoc_get("_services", str_arg).unwrap_or_else(|| str_arg.clone());
            let _ = setsparam("service", &service);
            let patcomps = assoc_flat("_patcomps");
            for i in patcomps.chunks(2) {
                if let (Some(pat), Some(action)) = (i.first(), i.get(1)) {
                    // sh:26 — `(K)$str` matches the key (a zsh pattern)
                    //   against `$str`. Compile via the real pattern
                    //   engine; fall back to literal equality on a
                    //   malformed pattern.
                    let matched = pattern_match(pat, str_arg);
                    if matched {
                        // sh:32 `eval "$i" && ret=0`
                        if eval_comp(action, 32) == 0 {
                            ret = 0;
                        }
                        let cs = getsparam("_compskip").unwrap_or_default();
                        if cs.contains("patterns") {
                            break;
                        }
                        if cs == "all" {
                            let _ = setsparam("_compskip", "");
                            let _ = setsparam("curcontext", &saved_curcontext);
                            let _ = setsparam("service", &saved_service);
                            return ret;
                        }
                    }
                }
            }
        }
    }

    // sh:43-54  $_comps lookup
    ret = 1;
    let mut comp = String::new();
    let mut name = String::new();
    for str_arg in &argv {
        if str_arg.is_empty() {
            continue;
        }
        // sh:48 `${(Q)str}` — quote-strip (no-op here; our argv has no
        //   shell-quoting layer)
        name = str_arg.clone();
        comp = assoc_get("_comps", str_arg).unwrap_or_default();
        let service = assoc_get("_services", str_arg).unwrap_or_else(|| str_arg.clone());
        let _ = setsparam("service", &service);
        if !comp.is_empty() {
            break;
        }
    }

    let last_arg = argv.last().cloned().unwrap_or_default();
    tracing::debug!(target: "compsys_args", ?argv, %comp, %name, %last_arg, "_dispatch resolution");
    // sh:58-63
    if !comp.is_empty() && name != last_arg {
        let _ = setsparam("_compskip", "patterns");
        // sh:63 `eval "$comp" && ret=0`
        let drc = eval_comp(&comp, 63);
        tracing::debug!(target: "compsys_args", %comp, ?drc, "_dispatch ran completer");
        if drc == 0 {
            ret = 0;
        }
        let cs = getsparam("_compskip").unwrap_or_default();
        if cs == "all" || cs.contains("patterns") {
            let _ = setsparam("_compskip", "");
            let _ = setsparam("curcontext", &saved_curcontext);
            let _ = setsparam("service", &saved_service);
            return ret;
        }
    }

    // sh:65-79  postpatcomps pass
    let cs = getsparam("_compskip").unwrap_or_default();
    if !(cs == "all" || cs.contains("patterns")) {
        for str_arg in &argv {
            if str_arg.is_empty() {
                continue;
            }
            let service = assoc_get("_services", str_arg).unwrap_or_else(|| str_arg.clone());
            let _ = setsparam("service", &service);
            let pp = assoc_flat("_postpatcomps");
            for i in pp.chunks(2) {
                if i.first()
                    .map(|k| pattern_match(k, str_arg))
                    .unwrap_or(false)
                {
                    if let Some(action) = i.get(1) {
                        let _ = setsparam("_compskip", "default");
                        // sh:73 `eval "$i" && ret=0`
                        if eval_comp(action, 73) == 0 {
                            ret = 0;
                        }
                        let cs2 = getsparam("_compskip").unwrap_or_default();
                        if cs2.contains("patterns") {
                            break;
                        }
                        if cs2 == "all" {
                            let _ = setsparam("_compskip", "");
                            let _ = setsparam("curcontext", &saved_curcontext);
                            let _ = setsparam("service", &saved_service);
                            return ret;
                        }
                    }
                }
            }
        }
    }

    // sh:84-87  fallback to last-arg's comp
    let cs2 = getsparam("_compskip").unwrap_or_default();
    if name == last_arg && !comp.is_empty() && !(cs2 == "all" || cs2.contains("default")) {
        let service = assoc_get("_services", &name).unwrap_or_else(|| name.clone());
        let _ = setsparam("service", &service);
        // sh:87 `eval "$comp" && ret=0`
        if eval_comp(&comp, 87) == 0 {
            ret = 0;
        }
    }

    let _ = setsparam("_compskip", "");
    let _ = setsparam("curcontext", &saved_curcontext);
    let _ = setsparam("service", &saved_service);
    let _ = saved_compskip;
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_dispatch(&[]), 1);
    }

    #[test]
    fn dash_s_consumed_and_ctx_appended() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "a:b:c:d");
        let _ = _dispatch(&["-s".to_string(), "mycmd".to_string()]);
        // After return, curcontext is restored to its pre-call value.
        assert_eq!(getsparam("curcontext").as_deref(), Some("a:b:c:d"));
    }

    #[test]
    fn assoc_get_reads_hashed_assoc_not_getaparam() {
        // Regression: `_comps`/`_services` are PM_HASHED assocs. The old
        // `getaparam(name).chunks(2)` layout returned `None` for a hash,
        // so `$_comps[cat]` came back "" and `_dispatch` never eval'd the
        // command completer — compsys produced zero matches. assoc_get
        // must read the hash backing.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::sethparam(
            "_comps",
            vec![
                "cat".to_string(),
                "_cat".to_string(),
                "ls".to_string(),
                "_ls".to_string(),
            ],
        );
        assert_eq!(assoc_get("_comps", "cat").as_deref(), Some("_cat"));
        assert_eq!(assoc_get("_comps", "ls").as_deref(), Some("_ls"));
        assert_eq!(assoc_get("_comps", "nope"), None);
        // getaparam still returns None for the hash — proves the bug the
        // fix routes around.
        assert!(crate::ported::params::getaparam("_comps").is_none());
        // assoc_flat exposes the [k,v,…] pairs the pattern walks need.
        let flat = assoc_flat("_comps");
        assert!(flat.windows(2).any(|w| w == ["cat", "_cat"]));
    }
}
