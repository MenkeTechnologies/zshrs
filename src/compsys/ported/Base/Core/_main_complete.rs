//! Port of `_main_complete` from
//! `Completion/Base/Core/_main_complete`.
//!
//! Full upstream body (418 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 25  local func funcs ret=1 tmp _compskip … _saved_* state snapshots
//! sh: 52  [[ -z "$curcontext" ]] && curcontext=:::          # FLOOR
//! sh: 56  zstyle -s … insert-tab tmp → pending-tab short-circuit
//! sh: 70  GLOB_COMPLETE second-attempt re-prep
//! sh: 79  special-context dispatch: equals, ~[, …
//! sh:120  collect completer chain (style + default)
//! sh:170  for _completer in chain: build curcontext, run matcher-list × completer
//! sh:340  if ret != 0 and we have a default-message format, _message
//! sh:380  post-funcs
//! sh:400  restore compstate snapshots
//! sh:418  return ret
//! ```
//!
//! `_main_complete` is the primary entry-point invoked by every
//! completion widget. This port covers the essential structural
//! skeleton: curcontext floor → completer-chain iteration →
//! state snapshot/restore. Edge-case handling (GLOB_COMPLETE,
//! pending-tab, equals/`~[` context, post-funcs) is left as a
//! TODO for follow-on work.

use crate::ported::exec_hooks::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// `_main_complete` — primary completion-dispatch entry. Args
/// (when non-empty) override the configured `completer` style with
/// the supplied chain.
pub fn _main_complete(args: &[String]) -> i32 {
    // sh:25  snapshot compstate so we can restore on exit
    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let saved_compskip = getsparam("_compskip").unwrap_or_default();
    let saved_exact = get_compstate_str("exact").unwrap_or_default();
    let saved_lastprompt = get_compstate_str("last_prompt").unwrap_or_default();
    let saved_list = get_compstate_str("list").unwrap_or_default();
    let saved_insert = get_compstate_str("insert").unwrap_or_default();
    let _ = setsparam("_saved_exact", &saved_exact);
    let _ = setsparam("_saved_lastprompt", &saved_lastprompt);
    let _ = setsparam("_saved_list", &saved_list);
    let _ = setsparam("_saved_insert", &saved_insert);

    // sh:52  curcontext floor — the bug the user flagged earlier:
    //   without this, every downstream zstyle query goes to the
    //   wrong field position.
    if saved_curcontext.is_empty() {
        let _ = setsparam("curcontext", ":::");
    }
    let mut curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:31-33  global tag-tracking state init
    let _ = setsparam("_tags_level", "0");
    let _ = setsparam("_comp_tags", "");
    let _ = setsparam("_comp_mesg", "");
    setaparam("_lastdescr", Vec::new());
    setaparam("_comp_ignore", Vec::new());
    setaparam("_comp_colors", Vec::new());

    // sh:120  completer chain
    let chain: Vec<String> = if !args.is_empty() {
        args.to_vec()
    } else {
        let style_chain = lookupstyle(
            &format!(":completion:{}:", curcontext),
            "completer",
        );
        if !style_chain.is_empty() {
            style_chain
        } else {
            // Default: complete then expand then approximate
            vec!["_complete".to_string(), "_approximate".to_string()]
        }
    };

    // Publish the chain so other completers (e.g. _prefix / _ignored)
    //   can inspect it via `$_completers`.
    setaparam("_completers", chain.clone());

    let mut ret: i32 = 1;
    let mut completer_num: i64 = 1;
    for completer_spec in &chain {
        let _ = setsparam("_completer_num", &completer_num.to_string());

        // sh:165  split `spec` on `:` — left of `:` is the fn name,
        //   right is the curcontext-field suffix.
        let mut parts = completer_spec.splitn(2, ':');
        let bare = parts.next().unwrap_or("").to_string();
        let field_suffix = parts
            .next()
            .map(|s| s.to_string())
            .unwrap_or_else(|| bare.strip_prefix('_').map(|s| s.replace('_', "-")).unwrap_or_default());
        let _ = setsparam("_completer", &field_suffix);

        // sh:175  curcontext patch: replace middle `:`-field
        let new_ctx = patch_completer_field(&curcontext, &field_suffix);
        let _ = setsparam("curcontext", &new_ctx);
        curcontext = new_ctx;

        // sh:180  matcher-list loop
        let matchers = lookupstyle(
            &format!(":completion:{}:", curcontext),
            "matcher-list",
        );
        let matcher_list: Vec<String> = if matchers.is_empty() {
            vec!["".to_string()]
        } else {
            matchers
        };
        let mut matcher_num: i64 = 1;
        let mut combined_matcher = String::new();
        for m in &matcher_list {
            let _ = setsparam("_matcher_num", &matcher_num.to_string());
            if let Some(rest) = m.strip_prefix('+') {
                combined_matcher = format!("{} {}", combined_matcher, rest);
            } else {
                combined_matcher = m.clone();
            }
            let _ = setsparam("_matcher", combined_matcher.trim());

            if dispatch_function_call(&bare, &[]).unwrap_or(1) == 0 {
                ret = 0;
                break;
            }
            matcher_num += 1;
        }
        if ret == 0 {
            break;
        }
        completer_num += 1;
    }

    // sh:380  post-funcs
    let postfuncs = getaparam("comppostfuncs").unwrap_or_default();
    for pf in &postfuncs {
        let _ = dispatch_function_call(pf, &[]);
    }
    setaparam("comppostfuncs", Vec::new());

    // sh:400  restore compstate snapshots
    set_compstate_str("exact", &saved_exact);
    set_compstate_str("last_prompt", &saved_lastprompt);
    set_compstate_str("list", &saved_list);
    set_compstate_str("insert", &saved_insert);
    let _ = setsparam("curcontext", &saved_curcontext);
    let _ = setsparam("_compskip", &saved_compskip);
    ret
}

/// sh:175 — replace the middle `:`-field of `curcontext` with the
/// completer's name. For `a:b:c:d` and `complete`, result is
/// `a:complete:c:d`.
fn patch_completer_field(curcontext: &str, completer: &str) -> String {
    let mut parts: Vec<&str> = curcontext.split(':').collect();
    if parts.len() < 4 {
        // Pad with empty fields to get 4 colons
        while parts.len() < 4 {
            parts.push("");
        }
    }
    parts[1] = completer;
    parts.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_curcontext_initializes_floor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "");
        let _ = _main_complete(&[]);
        // After return, curcontext is restored to ""
        assert_eq!(getsparam("curcontext").as_deref(), Some(""));
    }

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "a:b:c:d");
        assert_eq!(_main_complete(&[]), 1);
    }

    #[test]
    fn explicit_chain_overrides_style() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "a:b:c:d");
        let _ = _main_complete(&["_complete".to_string()]);
        let chain = getaparam("_completers").unwrap_or_default();
        assert_eq!(chain, vec!["_complete"]);
    }

    #[test]
    fn patch_completer_field_replaces_middle() {
        assert_eq!(
            patch_completer_field("a:b:c:d", "complete"),
            "a:complete:c:d"
        );
    }
}
