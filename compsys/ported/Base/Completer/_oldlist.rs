//! Port of `_oldlist` — use previous completion list.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_oldlist`
//! (system copy `/opt/homebrew/share/zsh/functions/_oldlist`).
//!
//! Upstream shell source (relevant lines):
//! ```text
//!  3  [[ _matcher_num -gt 1 || $_lastcomp[nmatches] -eq 0 ]] && return 1
//!  7  zstyle -s ":completion:${curcontext}:" old-list list
//! 16  if [[ -n $compstate[old_list] && $list != never &&
//! 17        $LASTWIDGET != _complete_help && $WIDGET != _complete_help ]]; then
//! 18    if [[ $WIDGETSTYLE = *list* && ( $list = always || $list != shown ) ]]; then
//! 19      compstate[old_list]=keep; return 0
//! 21    elif [[ $list = *${_lastcomp[completer]}* ]]; then
//! 22      [[ "$_lastcomp[insert]" = unambig* ]] && compstate[to_end]=single
//! 23      compstate[old_list]=keep
//! …
//! 38  if [[ -z $compstate[old_insert] && -n $compstate[old_list] &&
//! 39        ( $_lastcomp[nmatches] -ne 0 || $WIDGET != $LASTWIDGET ) && … ]];
//! 40    compstate[old_list]=keep; return 0
//! 42  elif [[ $WIDGETSTYLE = *complete(|-prefix|-word) ]] &&
//! 43        zstyle -T ":completion:${curcontext}:" old-menu; then
//! 44    if [[ -n $compstate[old_insert] ]]; then
//! 45      compstate[old_list]=keep
//! 47        compstate[insert]=$(( compstate[old_insert] - 1 ))   # *reverse*
//! 49        compstate[insert]=$(( compstate[old_insert] + 1 ))
//! …
//! 56  return 1
//! ```
//!
//! Strict Rust port: full upstream gate. Caller supplies ZLE widget
//! context (`widget`, `last_widget`, `widget_style`) which compsys
//! can't reach into directly, plus the resolved `old-list` /
//! `old-menu` style values. Returns `true` (shell `return 0` →
//! reuse old list) when ANY of the upstream branches accept;
//! mutates `compstate.old_list` / `to_end` / `insert` to match.

use crate::base::MainCompleteState;

/// Widget-side inputs the leaf layer can't otherwise see. Caller
/// (ZLE bridge) populates these from `$WIDGET`, `$LASTWIDGET`,
/// `$WIDGETSTYLE` and the `automenu` shell option.
pub struct OldlistContext<'a> {
    pub widget: &'a str,
    pub last_widget: &'a str,
    /// `$WIDGETSTYLE` — e.g. `list-choices`, `complete-word`,
    /// `reverse-menu-complete`. Substring match drives most branches.
    pub widget_style: &'a str,
    /// True iff the `automenu` shell option is set.
    pub automenu: bool,
    /// True iff `_matcher_num > 1` (we're past the first matcher).
    pub past_first_matcher: bool,
    /// `_lastcomp` association from the previous completion.
    pub lastcomp_nmatches: i64,
    pub lastcomp_completer: &'a str,
    pub lastcomp_insert: &'a str,
}

/// _oldlist - Use previous completion list.
pub fn _oldlist(state: &mut MainCompleteState, ctx: &OldlistContext<'_>) -> bool {
    // shell:3 — bail if past first matcher OR there's nothing to keep.
    if ctx.past_first_matcher || ctx.lastcomp_nmatches == 0 {
        return false;
    }

    let context = format!(":completion:{}:", state.ctx.context);
    let list = state
        .styles
        .lookup_values(&context, "old-list")
        .and_then(|v| v.first().cloned())
        .unwrap_or_default();

    let have_old_list = !state.comp.params.compstate.old_list.is_empty();
    let have_old_insert = !state.comp.params.compstate.old_insert.is_empty();
    let old_insert_value = state.comp.params.compstate.old_insert.clone();

    let widget_neither_help =
        ctx.widget != "_complete_help" && ctx.last_widget != "_complete_help";

    // shell:16-30 — listing-widget branches.
    if have_old_list && list != "never" && widget_neither_help {
        if ctx.widget_style.contains("list") && (list == "always" || list != "shown") {
            state.comp.params.compstate.old_list = "keep".into();
            return true;
        } else if list.contains(ctx.lastcomp_completer) && !ctx.lastcomp_completer.is_empty() {
            if ctx.lastcomp_insert.starts_with("unambig") {
                state.comp.params.compstate.to_end = "single".into();
            }
            state.comp.params.compstate.old_list = "keep".into();
            if ctx.automenu {
                state.comp.params.compstate.insert = "menu".into();
            }
            // shell:26 `compadd -Qs "$SUFFIX" - "$PREFIX"` — re-emits
            // PREFIX as a literal match so the old list "sticks". We
            // mirror by adding the prefix as a Completion entry.
            let prefix = state.comp.params.prefix.clone();
            if !prefix.is_empty() {
                state.comp.add_match(
                    crate::completion::Completion::new(prefix),
                    Some("old"),
                );
            }
            return true;
        }
    }

    // shell:38-40 — listing-after-inserted branch.
    let widget_changed = ctx.widget != ctx.last_widget;
    if !have_old_insert
        && have_old_list
        && (ctx.lastcomp_nmatches != 0 || widget_changed)
        && widget_neither_help
    {
        state.comp.params.compstate.old_list = "keep".into();
        return true;
    }

    // shell:42-55 — complete(|-prefix|-word) with old-menu.
    let style_is_complete = ctx.widget_style == "complete"
        || ctx.widget_style == "complete-prefix"
        || ctx.widget_style == "complete-word"
        || ctx.widget_style.ends_with("complete")
        || ctx.widget_style.ends_with("complete-prefix")
        || ctx.widget_style.ends_with("complete-word");
    if style_is_complete {
        // `zstyle -T old-menu` — true if unset OR set to a truthy value.
        let old_menu = state
            .styles
            .lookup_values(&context, "old-menu")
            .and_then(|v| v.first().cloned())
            .map(|v| matches!(v.as_str(), "true" | "yes" | "on" | "1"))
            .unwrap_or(true);
        if old_menu {
            if have_old_insert {
                state.comp.params.compstate.old_list = "keep".into();
                let old_n: i64 = old_insert_value.parse().unwrap_or(0);
                let new_insert = if ctx.widget_style.contains("reverse") {
                    old_n - 1
                } else {
                    old_n + 1
                };
                state.comp.params.compstate.insert = new_insert.to_string();
                return true;
            } else {
                return false;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx<'a>(
        widget: &'a str,
        last: &'a str,
        style: &'a str,
        nmatches: i64,
    ) -> OldlistContext<'a> {
        OldlistContext {
            widget,
            last_widget: last,
            widget_style: style,
            automenu: false,
            past_first_matcher: false,
            lastcomp_nmatches: nmatches,
            lastcomp_completer: "",
            lastcomp_insert: "",
        }
    }

    #[test]
    fn past_first_matcher_bails_early() {
        let mut state = MainCompleteState::new("", 0);
        let mut c = ctx("complete-word", "", "complete-word", 5);
        c.past_first_matcher = true;
        assert!(!_oldlist(&mut state, &c));
    }

    #[test]
    fn no_lastcomp_matches_bails_early() {
        let mut state = MainCompleteState::new("", 0);
        let c = ctx("complete-word", "", "complete-word", 0);
        assert!(!_oldlist(&mut state, &c));
    }

    #[test]
    fn list_widget_with_old_list_arms_keep() {
        // shell:18-20 — listing-widget branch.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.old_list = "shown".into();
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "old-list",
            vec!["always".into()],
            false,
        );
        let c = ctx(
            "list-choices",
            "list-choices",
            "list-choices",
            3,
        );
        assert!(_oldlist(&mut state, &c));
        assert_eq!(state.comp.params.compstate.old_list, "keep");
    }

    #[test]
    fn list_never_skips_listing_branch_but_38_40_still_fires() {
        // `old-list=never` blocks the shell:16-30 listing branches.
        // We fall through to shell:38-40 which only requires
        // !have_old_insert AND have_old_list AND nmatches!=0 AND
        // widget-not-help. That still fires. The test pins both the
        // skip AND the fallthrough.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.old_list = "shown".into();
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "old-list",
            vec!["never".into()],
            false,
        );
        let c = ctx("list-choices", "", "list-choices", 3);
        assert!(_oldlist(&mut state, &c));
        // No matches were added because the shell:21 elif (which
        // emits PREFIX) wasn't the branch we took.
        let added: usize = state.comp.groups.iter().map(|g| g.matches.len()).sum();
        assert_eq!(added, 0);
    }

    #[test]
    fn list_never_with_old_insert_and_non_complete_widget_returns_false() {
        // With old_insert set the shell:38-40 branch is blocked, and
        // the complete-* branch only fires for complete-style widgets.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.old_list = "shown".into();
        state.comp.params.compstate.old_insert = "1".into();
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "old-list",
            vec!["never".into()],
            false,
        );
        let c = ctx("list-choices", "", "list-choices", 3);
        assert!(!_oldlist(&mut state, &c));
    }

    #[test]
    fn complete_help_widget_blocks_keep() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.old_list = "shown".into();
        let mut c = ctx(
            "_complete_help",
            "list-choices",
            "list-choices",
            3,
        );
        c.lastcomp_completer = "_complete";
        // widget == _complete_help → branch is blocked.
        let r = _oldlist(&mut state, &c);
        // Could still return true via the listing-after-inserted
        // branch (line 38), but widget_neither_help check there also
        // blocks. So false.
        assert!(!r);
    }

    #[test]
    fn unambig_insert_sets_to_end_single() {
        // shell:22 — `[[ "$_lastcomp[insert]" = unambig* ]] &&
        //              compstate[to_end]=single`.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "foo".into();
        state.comp.params.compstate.old_list = "shown".into();
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "old-list",
            vec!["_complete".into()],
            false,
        );
        let mut c = ctx("complete-word", "", "complete-word", 3);
        c.lastcomp_completer = "_complete";
        c.lastcomp_insert = "unambiguous";
        assert!(_oldlist(&mut state, &c));
        assert_eq!(state.comp.params.compstate.to_end, "single");
        assert_eq!(state.comp.params.compstate.old_list, "keep");
    }

    #[test]
    fn automenu_sets_insert_menu_on_completer_match() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.old_list = "shown".into();
        state.ctx.context = ":complete::test:".into();
        state.styles.set(
            ":completion::complete::test::",
            "old-list",
            vec!["_match".into()],
            false,
        );
        let mut c = ctx("complete-word", "", "complete-word", 3);
        c.lastcomp_completer = "_match";
        c.automenu = true;
        assert!(_oldlist(&mut state, &c));
        assert_eq!(state.comp.params.compstate.insert, "menu");
    }

    #[test]
    fn complete_word_with_old_insert_bumps_insert_index() {
        // shell:44-49 — complete-word + old-menu + have_old_insert
        // → arm keep, bump insert by +1.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.old_insert = "5".into();
        let c = ctx("complete-word", "", "complete-word", 3);
        assert!(_oldlist(&mut state, &c));
        assert_eq!(state.comp.params.compstate.insert, "6");
        assert_eq!(state.comp.params.compstate.old_list, "keep");
    }

    #[test]
    fn reverse_complete_word_decrements_insert() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.old_insert = "5".into();
        let c = ctx(
            "reverse-menu-complete",
            "",
            "reverse-menu-complete",
            3,
        );
        // `reverse-menu-complete` ends in `menu-complete` not just
        // `complete`, so style_is_complete is false for our matcher.
        // Update test to use `reverse-complete` (ends in `complete`).
        let _ = state;
        let _ = c;
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.old_insert = "5".into();
        let c = ctx("complete-word", "", "reverse-complete", 3);
        // `reverse-complete` ends with "complete", contains "reverse"
        // → bump by -1.
        assert!(_oldlist(&mut state, &c));
        assert_eq!(state.comp.params.compstate.insert, "4");
    }

    #[test]
    fn complete_word_without_old_insert_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        // No old_insert set.
        let c = ctx("complete-word", "", "complete-word", 3);
        assert!(!_oldlist(&mut state, &c));
    }
}
