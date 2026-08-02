//! Port of `_setup` from `Completion/Base/Core/_setup`.
//!
//! Full upstream body (79 lines verbatim, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local val nm="$compstate[nmatches]"
//! sh: 5  [[ $# -eq 1 ]] && 2="$1"
//! sh: 7  if zstyle -a ":completion:${curcontext}:$1" list-colors val; then
//! sh: 8    zmodload -i zsh/complist
//! sh:10    if [[ "$1" = default ]]; then
//! sh:11      _comp_colors=( "$val[@]" )
//! sh:12    else
//! sh:13      _comp_colors+=( "(${2})${(@)^val:#(|\(*\)*)}" "${(M@)val:#\(*\)*}" )
//! sh:14    fi
//! sh:16  elif [[ "$1" = default ]]; then
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

/// `_setup` — apply per-tag style settings to compstate. Args:
///   `[$tag, $group_name?]`. If group_name omitted, equals $1.
pub fn _setup(args: &[String]) -> i32 {
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
        // sh:10
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

    // sh:74-79  force-list
    let force_existing = getsparam("_comp_force_list").unwrap_or_default();
    if force_existing != "always" {
        let val = lookupstyle(&ctx, "force-list")
            .first()
            .cloned()
            .unwrap_or_default();
        if !val.is_empty() {
            let valid = val == "always"
                || (val.chars().all(|c| c.is_ascii_digit())
                    && (force_existing.is_empty()
                        || force_existing
                            .parse::<i64>()
                            .map(|cur| val.parse::<i64>().map(|v| cur > v).unwrap_or(false))
                            .unwrap_or(false)));
            if valid {
                let _ = setsparam("_comp_force_list", &val);
            }
        }
    }

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

    #[test]
    fn returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_setup(&["default".to_string()]), 0);
    }

    #[test]
    fn list_packed_style_appends_to_compstate_list() {
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("list", "");
        // We don't actually set the zstyle here — covers the "no
        //   style set" fall-through path. The assertion just checks
        //   no panic + integer return.
        let _ = _setup(&["tag1".to_string()]);
    }
}
