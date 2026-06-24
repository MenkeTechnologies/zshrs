//! Port of `_dispatch` from `Completion/Base/Core/_dispatch`.
//!
//! Full upstream body (91 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  local comp pat val name i ret=1 _compskip="$_compskip"
//! sh: 6  local curcontext="$curcontext" service str noskip
//! sh:11  if [[ "$1" = -s ]]; then noskip=yes; shift; fi
//! sh:14  [[ -z "$noskip" ]] && _compskip=
//! sh:16  curcontext="${curcontext%:*:*}:${1}:"
//! sh:17  shift
//! sh:21  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! sh:23    for str in "$@"; do
//! sh:25      service="${_services[$str]:-$str}"
//! sh:26      for i in "${(@)_patcomps[(K)$str]}"; do … patterns dispatch …
//! sh:39  fi
//! sh:43  ret=1
//! sh:44  for str in "$@"; do … look up $_comps[$str] …
//! sh:54  done
//! sh:58  if [[ -n "$comp" && "$name" != "${argv[-1]}" ]]; then
//! sh:60    _compskip=patterns
//! sh:61    eval "$comp" && ret=0
//! sh:65  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! sh:67    for str; do … _postpatcomps …
//! sh:81  [[ "$name" = "${argv[-1]}" && -n "$comp" &&
//! sh:82     "$_compskip" != (all|*default*) ]] &&
//! sh:83    service="${_services[$name]:-$name}" &&
//! sh:84     eval "$comp" && ret=0
//! sh:86  _compskip=''
//! sh:88  return ret
//! ```

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setsparam};

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

fn assoc_get(name: &str, key: &str) -> Option<String> {
    let arr = getaparam(name)?;
    arr.chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

/// `_dispatch` — central context dispatcher. `-s` skips the
/// `_compskip` reset; first non-flag arg becomes the curcontext
/// field; remaining args are tried in order against `$_comps`.
pub fn _dispatch(args: &[String]) -> i32 {
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
            let patcomps = getaparam("_patcomps").unwrap_or_default();
            for i in patcomps.chunks(2) {
                if let (Some(pat), Some(action)) = (i.first(), i.get(1)) {
                    // sh:26 — `(K)$str` matches the key (a zsh pattern)
                    //   against `$str`. Compile via the real pattern
                    //   engine; fall back to literal equality on a
                    //   malformed pattern.
                    let matched = pattern_match(pat, str_arg);
                    if matched {
                        let parts: Vec<String> =
                            action.split_whitespace().map(|s| s.to_string()).collect();
                        if let Some((cmd, rest)) = parts.split_first() {
                            if dispatch_function_call(cmd, rest).unwrap_or(1) == 0 {
                                ret = 0;
                            }
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
    // sh:58-63
    if !comp.is_empty() && name != last_arg {
        let _ = setsparam("_compskip", "patterns");
        if dispatch_function_call(&comp, &[]).unwrap_or(1) == 0 {
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
            let pp = getaparam("_postpatcomps").unwrap_or_default();
            for i in pp.chunks(2) {
                if i.first()
                    .map(|k| pattern_match(k, str_arg))
                    .unwrap_or(false)
                {
                    if let Some(action) = i.get(1) {
                        let _ = setsparam("_compskip", "default");
                        let parts: Vec<String> =
                            action.split_whitespace().map(|s| s.to_string()).collect();
                        if let Some((cmd, rest)) = parts.split_first() {
                            if dispatch_function_call(cmd, rest).unwrap_or(1) == 0 {
                                ret = 0;
                            }
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

    // sh:81-84  fallback to last-arg's comp
    let cs2 = getsparam("_compskip").unwrap_or_default();
    if name == last_arg && !comp.is_empty() && !(cs2 == "all" || cs2.contains("default")) {
        let service = assoc_get("_services", &name).unwrap_or_else(|| name.clone());
        let _ = setsparam("service", &service);
        if dispatch_function_call(&comp, &[]).unwrap_or(1) == 0 {
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
}
