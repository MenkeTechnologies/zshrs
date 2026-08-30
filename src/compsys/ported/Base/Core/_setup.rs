//! Port of `_setup` from `Completion/Base/Core/_setup`.
//!
//! Full upstream body (79 lines verbatim, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local val nm="$compstate[nmatches]"
//! sh: 5  [[ $# -eq 1 ]] && 2="$1"
//! sh: 7  if zstyle -a ":completion:${curcontext}:$1" list-colors val; then
//! sh: 8    zmodload -i zsh/complist
//! sh: 9    if [[ "$1" = default ]]; then
//! sh:10      _comp_colors=( "$val[@]" )
//! sh:11    else
//! sh:12      _comp_colors+=( "(${2})${(@)^val:#(|\(*\)*)}" "${(M@)val:#\(*\)*}" )
//! sh:13    fi
//! sh:21  elif [[ "$1" = default ]]; then
//! sh:22    unset ZLS_COLORS ZLS_COLOURS
//! sh:23  fi
//! sh:25  zstyle -s … show-ambiguity val → _ambiguous_color
//! sh:33  zstyle -t … list-packed → compstate[list] += packed
//! sh:42  zstyle -t … list-rows-first → compstate[list] += rows
//! sh:50  zstyle -t … last-prompt → compstate[last_prompt]=yes/empty
//! sh:58  zstyle -t … accept-exact → compstate[exact]=accept/empty
//! sh:67  zstyle -a … menu val → _last_menu_style stash
//! sh:74  zstyle -s … force-list val → _comp_force_list
//! ```
//!
//! Updates `$compstate` per-tag styles (list-packed, list-rows-first,
//! last-prompt, accept-exact, menu, force-list, show-ambiguity,
//! list-colors). Called by `_description` per tag-spec.

use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam, unsetparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// Reach `_setup` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_setup "$1" "${gname:--default-}"` (Completion/Base/Core/_description sh:19) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_setup_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _setup(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_setup", args, || _setup_impl(args))
}

/// `_setup` — apply per-tag style settings to compstate. Args:
///   `[$tag, $group_name?]`. If group_name omitted, equals $1.
pub fn _setup_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_setup");
    let tag = args.first().cloned().unwrap_or_default();
    let group = args.get(1).cloned().unwrap_or_else(|| tag.clone());
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:{}", curcontext, tag);

    // sh:3 — snapshot nmatches for sh:60 stash decision
    let nm: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // sh:7-23  list-colors. `_comp_colors` is declared PM_UNIQUE in
    // `_main_complete` (sh:54 `typeset -U`), so setaparam dedupes these
    // appends/assignments automatically — no manual dedup needed here.
    let lc = lookupstyle(&ctx, "list-colors");
    if !lc.is_empty() {
        // sh: 9
        if tag == "default" {
            setaparam("_comp_colors", lc);
        } else {
            // sh:13 — wrap each non-paren entry in `(group)` prefix
            let mut existing = getaparam("_comp_colors").unwrap_or_default();
            for v in &lc {
                if v.starts_with('(') {
                    existing.push(v.clone());
                } else {
                    existing.push(format!("({}){}", group, v));
                }
            }
            setaparam("_comp_colors", existing);
        }
    } else if tag == "default" {
        // sh:22
        unsetparam("ZLS_COLORS");
        unsetparam("ZLS_COLOURS");
    }

    // sh:25-29  show-ambiguity
    let sa = lookupstyle(&ctx, "show-ambiguity")
        .first()
        .cloned()
        .unwrap_or_default();
    if !sa.is_empty() {
        let val = if matches!(sa.as_str(), "yes" | "true" | "on") {
            "4".to_string()
        } else {
            sa
        };
        let _ = setsparam("_ambiguous_color", &val);
    }

    // sh:33-39  list-packed
    apply_list_flag(&ctx, "list-packed", "packed");
    // sh:42-48  list-rows-first
    apply_list_flag(&ctx, "list-rows-first", "rows");

    // sh:50-56  last-prompt
    let lp = testforstyle(&ctx, "last-prompt") == 0;
    let lp_vals = lookupstyle(&ctx, "last-prompt");
    if lp {
        set_compstate_str("last_prompt", "yes");
    } else if !lp_vals.is_empty() {
        set_compstate_str("last_prompt", "");
    } else {
        // restore saved
        let saved = getsparam("_saved_lastprompt").unwrap_or_default();
        set_compstate_str("last_prompt", &saved);
    }

    // sh:58-64  accept-exact
    let ae = testforstyle(&ctx, "accept-exact") == 0;
    let ae_vals = lookupstyle(&ctx, "accept-exact");
    if ae {
        set_compstate_str("exact", "accept");
    } else if !ae_vals.is_empty() {
        set_compstate_str("exact", "");
    } else {
        let saved = getsparam("_saved_exact").unwrap_or_default();
        set_compstate_str("exact", &saved);
    }

    // sh:66-67  menu-style stash (when nmatches grew since last)
    let last_nm: i64 = getsparam("_last_nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1);
    if last_nm >= 0 && last_nm != nm {
        let mut combined = getaparam("_last_menu_style").unwrap_or_default();
        let cur = getaparam("_menu_style").unwrap_or_default();
        combined.extend(cur);
        setaparam("_menu_style", combined);
    }

    // sh:69-72  menu style
    let menu_vals = lookupstyle(&ctx, "menu");
    if !menu_vals.is_empty() {
        let _ = setsparam("_last_nmatches", &nm.to_string());
        setaparam("_last_menu_style", menu_vals);
    } else {
        let _ = setsparam("_last_nmatches", "-1");
    }

    // sh:74-79  force-list — and, being the LAST statement of the upstream
    // body, the statement whose status IS `_setup`'s return value:
    //
    //   [[ "$_comp_force_list" != always ]] &&
    //     zstyle -s ":completion:${curcontext}:$1" force-list val &&
    //       [[ "$val" = always ||
    //          ( "$val" = [0-9]## &&
    //            ( -z "$_comp_force_list" || _comp_force_list -gt val ) ) ]] &&
    //       _comp_force_list="$val"
    //
    // Every link that fails short-circuits the `&&` chain to status 1, and
    // only the trailing assignment produces 0. No `force-list` style is set
    // in the overwhelmingly common case, so upstream `_setup` RETURNS 1.
    // The port returned 0 unconditionally.
    let force_existing = getsparam("_comp_force_list").unwrap_or_default();
    if force_existing == "always" {
        return 1; // sh:74 — `[[ "$_comp_force_list" != always ]]` false
    }
    // sh:75 — `zstyle -s … force-list val`. `-s` succeeds whenever the style
    // pattern matches, even for an empty value; `lookupstyle` mirrors that
    // with an empty Vec for "unset" (same convention as `_description.rs`),
    // and collapses the value array to a scalar by joining with a space.
    let fl_vals = lookupstyle(&ctx, "force-list");
    if fl_vals.is_empty() {
        return 1; // sh:75 — style unset
    }
    let val = crate::ported::utils::zjoin(&fl_vals, ' ');
    // sh:76-78 — `always`, or an all-digit value that beats the one already
    // recorded (`_comp_force_list -gt val` is an arithmetic compare inside
    // `[[ ]]`, so it reads the CURRENT value against the new one).
    let accept = val == "always"
        || (!val.is_empty()
            && val.chars().all(|c| c.is_ascii_digit())
            && (force_existing.is_empty()
                || match (force_existing.parse::<i64>(), val.parse::<i64>()) {
                    (Ok(cur), Ok(new)) => cur > new,
                    _ => false,
                }));
    if !accept {
        return 1; // sh:76-78 — test false
    }
    // sh:79
    let _ = setsparam("_comp_force_list", &val);
    0
}

/// Helper for sh:33/sh:42 — toggle `compstate[list]` += `<flag>`
/// based on style.
fn apply_list_flag(ctx: &str, style: &str, flag: &str) {
    let test = testforstyle(ctx, style) == 0;
    let vals = lookupstyle(ctx, style);
    let mut list_val = get_compstate_str("list").unwrap_or_default();
    if test {
        if !list_val.contains(flag) {
            if !list_val.is_empty() {
                list_val.push(' ');
            }
            list_val.push_str(flag);
        }
        set_compstate_str("list", &list_val);
    } else if !vals.is_empty() {
        let stripped = list_val.replace(flag, "");
        set_compstate_str("list", stripped.trim());
    } else {
        let saved = getsparam("_saved_list").unwrap_or_default();
        set_compstate_str("list", &saved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The last statement of `Completion/Base/Core/_setup` is the sh:74-79
    /// `&&` chain, so `_setup`'s exit status is that chain's. With no
    /// `force-list` style registered the `zstyle -s` at sh:75 fails and the
    /// chain short-circuits to 1 — which is what `_setup` returns for
    /// essentially every real call.
    ///
    /// This test previously asserted 0 (and was named `returns_zero`),
    /// pinning the port's unconditional `0` return. The zsh reference
    /// disagrees: `_setup files` reports `-- rc: 1` in the stock-utility
    /// sweep. Both the code and the assertion are corrected here.
    #[test]
    fn returns_one_when_no_force_list_style_is_set() {
        let _g = crate::test_util::global_state_lock();
        // sh:74 reads `$_comp_force_list`; clear it so the first link of the
        // chain is the `!= always` it is in a fresh completion.
        let _ = setsparam("_comp_force_list", "");
        assert_eq!(_setup_impl(&["default".to_string()]), 1);
    }

    /// sh:79 — the one path that reaches the trailing assignment and so
    /// returns 0: a `force-list` style whose value is accepted by sh:76-78.
    #[test]
    fn returns_zero_when_force_list_style_is_accepted() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("_comp_force_list", "");
        let _ = setsparam("curcontext", "zpf:zpf:zpf");
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        crate::ported::modules::zutil::bin_zstyle(
            "zstyle",
            &[
                ":completion:zpf:zpf:zpf:setuptag".to_string(),
                "force-list".to_string(),
                "always".to_string(),
            ],
            &ops,
            0,
        );
        let rc = _setup_impl(&["setuptag".to_string()]);
        let landed = getsparam("_comp_force_list");
        // Put the shared globals back BEFORE asserting: `_comp_force_list`
        // and `curcontext` are process-wide, and a panic between the call and
        // a trailing cleanup would leave `_comp_force_list=always` for every
        // later test in the binary (sh:74 then short-circuits `_setup` for
        // all of them). The zstyle itself is keyed on a context no other test
        // uses.
        let _ = setsparam("_comp_force_list", "");
        crate::ported::params::unsetparam("curcontext");
        assert_eq!(rc, 0);
        assert_eq!(
            landed.as_deref(),
            Some("always"),
            "sh:79 assignment must land"
        );
    }

    #[test]
    fn list_packed_style_appends_to_compstate_list() {
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("list", "");
        // We don't actually set the zstyle here — covers the "no
        //   style set" fall-through path. The assertion just checks
        //   no panic + integer return.
        let _ = _setup_impl(&["tag1".to_string()]);
    }
}
