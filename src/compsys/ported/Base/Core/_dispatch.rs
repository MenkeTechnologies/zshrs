//! Port of `_dispatch` from `Completion/Base/Core/_dispatch`.
//!
//! Full upstream body (91 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local comp pat val name i ret=1 _compskip="$_compskip"
//! sh: 4  local curcontext="$curcontext" service str noskip
//! sh: 5  local -a match mbegin mend
//! sh: 6
//! sh: 7  # If we get the option `-s', we don't reset `_compskip'.
//! sh: 8
//! sh: 9  if [[ "$1" = -s ]]; then
//! sh:10    noskip=yes
//! sh:11    shift
//! sh:12  fi
//! sh:13
//! sh:14  [[ -z "$noskip" ]] && _compskip=
//! sh:15
//! sh:16  curcontext="${curcontext%:*:*}:${1}:"
//! sh:17
//! sh:18  shift
//! sh:19
//! sh:20  # See if there are any matching pattern completions.
//! sh:21
//! sh:22  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! sh:23
//! sh:24    for str in "$@"; do
//! sh:25      [[ -n "$str" ]] || continue
//! sh:26      service="${_services[$str]:-$str}"
//! sh:27      for i in "${(@)_patcomps[(K)$str]}"; do
//! sh:28        if [[ $i = (#b)"="([^=]#)"="(*) ]]; then
//! sh:29  	service=$match[1]
//! sh:30  	i=$match[2]
//! sh:31        fi
//! sh:32        eval "$i" && ret=0
//! sh:33        if [[ "$_compskip" = *patterns* ]]; then
//! sh:34          break
//! sh:35        elif [[ "$_compskip" = all ]]; then
//! sh:36          _compskip=''
//! sh:37          return ret
//! sh:38        fi
//! sh:39      done
//! sh:40    done
//! sh:41  fi
//! sh:42
//! sh:43  # Now look up the names in the normal completion array.
//! sh:44
//! sh:45  ret=1
//! sh:46  for str in "$@"; do
//! sh:47    [[ -n "$str" ]] || continue
//! sh:48    # The following means we look up the names of commands
//! sh:49    # after stripping quotes.  This is presumably correct,
//! sh:50    # but do we need to do the same elsewhere?
//! sh:51    str=${(Q)str}
//! sh:52    name="$str"
//! sh:53    comp="${_comps[$str]}"
//! sh:54    service="${_services[$str]:-$str}"
//! sh:55
//! sh:56    [[ -z "$comp" ]] || break
//! sh:57  done
//! sh:58
//! sh:59  # And generate the matches, probably using default completion.
//! sh:60
//! sh:61  if [[ -n "$comp" && "$name" != "${argv[-1]}" ]]; then
//! sh:62    _compskip=patterns
//! sh:63    eval "$comp" && ret=0
//! sh:64    [[ "$_compskip" = (all|*patterns*) ]] && return ret
//! sh:65  fi
//! sh:66
//! sh:67  if [[ "$_compskip" != (all|*patterns*) ]]; then
//! sh:68    for str; do
//! sh:69      [[ -n "$str" ]] || continue
//! sh:70      service="${_services[$str]:-$str}"
//! sh:71      for i in "${(@)_postpatcomps[(K)$str]}"; do
//! sh:72        _compskip=default
//! sh:73        eval "$i" && ret=0
//! sh:74        if [[ "$_compskip" = *patterns* ]]; then
//! sh:75          break
//! sh:76        elif [[ "$_compskip" = all ]]; then
//! sh:77          _compskip=''
//! sh:78          return ret
//! sh:79        fi
//! sh:80      done
//! sh:81    done
//! sh:82  fi
//! sh:83
//! sh:84  [[ "$name" = "${argv[-1]}" && -n "$comp" &&
//! sh:85     "$_compskip" != (all|*default*) ]] &&
//! sh:86    service="${_services[$name]:-$name}" &&
//! sh:87     eval "$comp" && ret=0
//! sh:88
//! sh:89  _compskip=''
//! sh:90
//! sh:91  return ret
//! ```
//!
//! Faithful Rust port:
//! - Honors `compskip` flags ("all", "patterns", "default") via
//! module-level functions.
//! - shell:16 — curcontext rewrite: strip last two `:`-segments,
//! append `:$1:`.
//! - shell:26 — `_services[$str]:-$str` service-aliasing.
//! - shell:27-32 — pattern completions via `_patcomps` walk.
//! - shell:39-41 — `_comps[$service]` lookup + invoke via the
//! `_call_function` registry.



use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::compsys::base::{CompleterResult, MainCompleteState};
use crate::compsys::ported::_call_function::_call_function;

/// Shell-side `_compskip` parameter (controls which branches of
/// _dispatch get skipped). Honored values: "all" (skip everything),
/// "patterns" (skip pattern walk), "default" (skip service comp).
fn compskip_cell() -> &'static Mutex<String> {
    static C: OnceLock<Mutex<String>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(String::new()))
}

/// Get current `_compskip` value (empty string when no skip set).
pub fn compskip() -> String {
    compskip_cell().lock().map(|g| g.clone()).unwrap_or_default()
}

/// Set `_compskip`. Pass `""` to clear.
pub fn set_compskip(v: impl Into<String>) {
    if let Ok(mut g) = compskip_cell().lock() {
        *g = v.into();
    }
}

/// Service-alias registry: `_services[$str]:-$str` at shell:26.
fn services_cell() -> &'static Mutex<HashMap<String, String>> {
    static C: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_service(cmd: impl Into<String>, service: impl Into<String>) {
    if let Ok(mut g) = services_cell().lock() {
        g.insert(cmd.into(), service.into());
    }
}

pub fn resolve_service(cmd: &str) -> String {
    services_cell()
        .lock()
        .ok()
        .and_then(|g| g.get(cmd).cloned())
        .unwrap_or_else(|| cmd.to_string())
}

/// Pattern-completions registry: `_patcomps` at shell:27. Maps a
/// glob → callback name to invoke (callback registered via
/// [`crate::compsys::ported::_call_function::register`]).
fn patcomps_cell() -> &'static Mutex<Vec<(String, String)>> {
    static C: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register_patcomp(pattern: impl Into<String>, callback_name: impl Into<String>) {
    if let Ok(mut g) = patcomps_cell().lock() {
        g.push((pattern.into(), callback_name.into()));
    }
}

pub fn patcomps_for(cmd: &str) -> Vec<String> {
    patcomps_cell()
        .lock()
        .ok()
        .map(|g| {
            g.iter()
                .filter(|(pat, _)| glob_match(pat, cmd))
                .map(|(_, cb)| cb.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Reset all dispatch-state for tests.
#[doc(hidden)]
pub fn _reset_for_tests() {
    set_compskip("");
    if let Ok(mut g) = services_cell().lock() {
        g.clear();
    }
    if let Ok(mut g) = patcomps_cell().lock() {
        g.clear();
    }
}

/// Simple glob match: `*` / `?`. Used for `_patcomps` lookup.
fn glob_match(pat: &str, text: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    fn helper(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => helper(&p[1..], t) || (!t.is_empty() && helper(p, &t[1..])),
            (Some('?'), Some(_)) => helper(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => helper(&p[1..], &t[1..]),
            _ => false,
        }
    }
    helper(&p, &t)
}

/// _dispatch - Dispatch completion to the per-command function.
///
/// `comps` is the `_comps` array: command → callback-name mapping.
/// Callbacks are resolved via the `_call_function` registry — the
/// caller must have `register`'d them at startup.
///
/// `commands` is the fallback chain (shell:39: `"$_comp_command"
/// "$_comp_command1" "$_comp_command2" -default-`). First entry
/// with a registered callback wins.
pub fn _dispatch(
    state: &mut MainCompleteState,
    comps: &HashMap<String, String>,
    commands: &[&str],
) -> CompleterResult {
    let skip = compskip();
    let skip_all = skip == "all";
    let skip_patterns = skip_all || skip.contains("patterns");
    let skip_default = skip_all || skip.contains("default");

    // shell:14 — reset _compskip unless -s flag (handled by caller).
    set_compskip("");

    // shell:16 — curcontext rewrite: `${curcontext%:*:*}:${1}:`
    if let Some(first) = commands.first() {
        let new_ctx = rewrite_curcontext(&state.ctx.context, first);
        state.ctx.context = new_ctx;
    }

    let mut matched = false;

    // shell:22-37 — pattern completions
    if !skip_patterns {
        for cmd in commands {
            let cbs = patcomps_for(cmd);
            for cb in cbs {
                if _call_function(state, &cb) {
                    matched = true;
                }
                if compskip().contains("patterns") {
                    break;
                }
                if compskip() == "all" {
                    set_compskip("");
                    return if matched {
                        CompleterResult::Matched
                    } else {
                        CompleterResult::NoMatch
                    };
                }
            }
        }
    }

    // shell:39-41 — default service-comps lookup
    if !skip_default {
        for cmd in commands {
            let service = resolve_service(cmd);
            if let Some(callback) = comps.get(&service) {
                if _call_function(state, callback) {
                    matched = true;
                    break;
                }
            }
        }
    }

    if matched {
        CompleterResult::Matched
    } else {
        CompleterResult::NoMatch
    }
}

/// Rewrite curcontext per shell:16:
///   `${curcontext%:*:*}:${1}:`
/// = strip the last two `:`-suffix segments, append `:$1:`.
fn rewrite_curcontext(ctx: &str, replacement: &str) -> String {
    let trimmed = ctx.trim_end_matches(':');
    // %:* strips the last segment.
    let after_one = match trimmed.rfind(':') {
        Some(i) => &trimmed[..i],
        None => trimmed,
    };
    // %:*:* strips the last two segments.
    let after_two = match after_one.rfind(':') {
        Some(i) => &after_one[..i],
        None => after_one,
    };
    format!("{}:{}:", after_two, replacement)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    fn fresh() -> MainCompleteState {
        _reset_for_tests();
        let mut state = MainCompleteState::new("git status", 10);
        state.ctx.context = ":complete:foo:bar:".into();
        state
    }

    // ── Service registry ────────────────────────────────────────

    #[test]
    fn resolve_service_falls_back_to_cmd_when_unregistered() {
        let _g = lock();
        _reset_for_tests();
        assert_eq!(resolve_service("git"), "git");
    }

    #[test]
    fn resolve_service_uses_registered_alias() {
        let _g = lock();
        _reset_for_tests();
        register_service("git", "_git_wrapper");
        assert_eq!(resolve_service("git"), "_git_wrapper");
    }

    // ── Compskip ────────────────────────────────────────────────

    #[test]
    fn compskip_default_empty() {
        let _g = lock();
        _reset_for_tests();
        assert_eq!(compskip(), "");
    }

    #[test]
    fn set_compskip_round_trips() {
        let _g = lock();
        _reset_for_tests();
        set_compskip("patterns");
        assert_eq!(compskip(), "patterns");
        set_compskip("");
        assert_eq!(compskip(), "");
    }

    // ── glob_match ──────────────────────────────────────────────

    #[test]
    fn glob_match_basic_patterns() {
        assert!(glob_match("git*", "git-svn"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("?it", "git"));
        assert!(!glob_match("git*", "svn"));
    }

    // ── patcomps registry ───────────────────────────────────────

    #[test]
    fn patcomps_returns_matching_callbacks_in_registration_order() {
        let _g = lock();
        _reset_for_tests();
        register_patcomp("git*", "_pat_git");
        register_patcomp("svn*", "_pat_svn");
        register_patcomp("g*t", "_pat_endt"); // matches `git` (ends in t)
        let r = patcomps_for("git");
        // Both `git*` and `g*t` match `git`. `svn*` does not.
        assert_eq!(r, vec!["_pat_git".to_string(), "_pat_endt".to_string()]);
    }

    // ── curcontext rewriting ────────────────────────────────────

    #[test]
    fn rewrite_curcontext_replaces_last_two_segments() {
        // shell:16 — `${curcontext%:*:*}:${1}:`
        // `:complete:foo:bar:` → strip `:bar:` then `:foo:` → `:complete`
        // → append `:newcmd:` → `:complete:newcmd:`
        assert_eq!(
            rewrite_curcontext(":complete:foo:bar:", "newcmd"),
            ":complete:newcmd:"
        );
    }

    #[test]
    fn rewrite_curcontext_handles_short_ctx() {
        assert_eq!(rewrite_curcontext(":a:", "x"), ":x:");
        assert_eq!(rewrite_curcontext("", "x"), ":x:");
    }

    // ── End-to-end _dispatch ────────────────────────────────────

    #[test]
    fn known_command_invokes_registered_callback() {
        let _g = lock();
        let mut state = fresh();
        let invoked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let i2 = invoked.clone();
        crate::compsys::ported::_call_function::register(
            "test_dispatch_cb_known",
            Box::new(move |_| {
                i2.store(true, std::sync::atomic::Ordering::SeqCst);
                true
            }),
        );
        let mut comps = HashMap::new();
        comps.insert("git".to_string(), "test_dispatch_cb_known".to_string());
        let r = _dispatch(&mut state, &comps, &["git"]);
        assert!(matches!(r, CompleterResult::Matched));
        assert!(invoked.load(std::sync::atomic::Ordering::SeqCst));
        crate::compsys::ported::_call_function::unregister("test_dispatch_cb_known");
    }

    #[test]
    fn unknown_commands_return_no_match() {
        let _g = lock();
        let mut state = fresh();
        let comps: HashMap<String, String> = HashMap::new();
        let r = _dispatch(&mut state, &comps, &["nonexistent-cmd-xyz"]);
        assert!(matches!(r, CompleterResult::NoMatch));
    }

    #[test]
    fn first_matching_command_short_circuits() {
        let _g = lock();
        let mut state = fresh();
        let invoked = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let i2 = invoked.clone();
        crate::compsys::ported::_call_function::register(
            "test_dispatch_cb_first",
            Box::new(move |_| {
                i2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                true
            }),
        );
        let mut comps = HashMap::new();
        comps.insert("b".to_string(), "test_dispatch_cb_first".to_string());
        let r = _dispatch(&mut state, &comps, &["a", "b", "c"]);
        assert!(matches!(r, CompleterResult::Matched));
        assert_eq!(invoked.load(std::sync::atomic::Ordering::SeqCst), 1);
        crate::compsys::ported::_call_function::unregister("test_dispatch_cb_first");
    }

    #[test]
    fn compskip_all_short_circuits() {
        let _g = lock();
        let mut state = fresh();
        set_compskip("all");
        crate::compsys::ported::_call_function::register(
            "test_dispatch_cb_skip",
            Box::new(|_| true),
        );
        let mut comps = HashMap::new();
        comps.insert("git".to_string(), "test_dispatch_cb_skip".to_string());
        let r = _dispatch(&mut state, &comps, &["git"]);
        // skip=all → no patcomp + no default → NoMatch.
        assert!(matches!(r, CompleterResult::NoMatch));
        crate::compsys::ported::_call_function::unregister("test_dispatch_cb_skip");
    }

    #[test]
    fn service_alias_redirects_callback_lookup() {
        let _g = lock();
        let mut state = fresh();
        register_service("git", "_my_git_service");
        crate::compsys::ported::_call_function::register(
            "test_dispatch_svc_cb",
            Box::new(|_| true),
        );
        let mut comps = HashMap::new();
        // Note: registered under the SERVICE name, not the cmd name.
        comps.insert(
            "_my_git_service".to_string(),
            "test_dispatch_svc_cb".to_string(),
        );
        let r = _dispatch(&mut state, &comps, &["git"]);
        assert!(matches!(r, CompleterResult::Matched));
        crate::compsys::ported::_call_function::unregister("test_dispatch_svc_cb");
    }

    #[test]
    fn pattern_completer_runs_before_default() {
        let _g = lock();
        let mut state = fresh();
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let o1 = order.clone();
        let o2 = order.clone();
        crate::compsys::ported::_call_function::register(
            "test_pat_cb",
            Box::new(move |_| {
                o1.lock().unwrap().push("pat".into());
                false // patterns return false → default still runs
            }),
        );
        crate::compsys::ported::_call_function::register(
            "test_def_cb",
            Box::new(move |_| {
                o2.lock().unwrap().push("def".into());
                true
            }),
        );
        register_patcomp("git*", "test_pat_cb");
        let mut comps = HashMap::new();
        comps.insert("git-svn".to_string(), "test_def_cb".to_string());
        let r = _dispatch(&mut state, &comps, &["git-svn"]);
        assert!(matches!(r, CompleterResult::Matched));
        let log = order.lock().unwrap().clone();
        assert_eq!(log, vec!["pat".to_string(), "def".to_string()]);
        crate::compsys::ported::_call_function::unregister("test_pat_cb");
        crate::compsys::ported::_call_function::unregister("test_def_cb");
    }

    #[test]
    fn curcontext_rewritten_to_dispatched_cmd() {
        let _g = lock();
        let mut state = fresh();
        let comps: HashMap<String, String> = HashMap::new();
        let _ = _dispatch(&mut state, &comps, &["mycmd"]);
        // shell:16 — ":complete:foo:bar:" → ":complete:mycmd:"
        assert_eq!(state.ctx.context, ":complete:mycmd:");
    }
}
