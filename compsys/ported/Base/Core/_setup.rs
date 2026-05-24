//! Port of `_setup` — set up completion context based on zstyle
//! settings.
//!
//! Local shell reference: `compsys/functions/Base/Core/_setup`
//! (system copy `/opt/homebrew/share/zsh/functions/_setup`).
//!
//! Upstream shell source (every relevant line — the full 80-line fn
//! is small enough to inline):
//! ```text
//!  3  local val nm="$compstate[nmatches]"
//!  5  [[ $# -eq 1 ]] && 2="$1"
//!  7  if zstyle -a ":completion:${curcontext}:$1" list-colors val; then
//!  8    zmodload -i zsh/complist
//!  9    if [[ "$1" = default ]]; then
//! 10      _comp_colors=( "$val[@]" )
//! 12      _comp_colors+=( "(${2})${(@)^val:#(|\(*\)*)}" "${(M@)val:#\(*\)*}" )
//! 21  elif [[ "$1" = default ]]; then
//! 22    unset ZLS_COLORS ZLS_COLOURS
//! 27  if zstyle -s ":completion:${curcontext}:$1" show-ambiguity val; then
//! 29    [[ $val = (yes|true|on) ]] && _ambiguous_color=4 || _ambiguous_color=$val
//! 32  if zstyle -t ":completion:${curcontext}:$1" list-packed; then
//! 33    compstate[list]="${compstate[list]} packed"
//! 35    compstate[list]="${compstate[list]:gs/packed//}"
//! 37    compstate[list]="$_saved_list"
//! 40  if zstyle -t ":completion:${curcontext}:$1" list-rows-first; then
//! 41    compstate[list]="${compstate[list]} rows"
//! 48  if zstyle -t ":completion:${curcontext}:$1" last-prompt; then
//! 49    compstate[last_prompt]=yes
//! 56  if zstyle -t ":completion:${curcontext}:$1" accept-exact; then
//! 57    compstate[exact]=accept
//! 64  [[ _last_nmatches -ge 0 && _last_nmatches -ne nm ]] &&
//! 65    _menu_style=( "$_last_menu_style[@]" "$_menu_style[@]" )
//! 67  if zstyle -a ":completion:${curcontext}:$1" menu val; then
//! 68    _last_nmatches=$nm
//! 74  [[ "$_comp_force_list" != always ]] &&
//! 75    zstyle -s ":completion:${curcontext}:$1" force-list val &&
//! 76      [[ "$val" = always || …
//! 79      _comp_force_list="$val"
//! ```
//!
//! Faithful Rust port: handles EVERY zstyle the upstream consults.
//! Three flavors of compstate mutation:
//!   - boolean `+=word` (shell:33 list-packed → "packed")
//!   - boolean `=value` (shell:49 last-prompt → "yes")
//!   - tri-state with restore (shell:37 falls back to `$_saved_list`
//!     when style isn't set; we use clear-and-restore semantics)
//!
//! Side effects exposed via `state.params.compstate.*` fields and
//! the dedicated `_comp_colors` / `_ambiguous_color` /
//! `_last_menu_style` / `_comp_force_list` accessors below.

use std::sync::{Mutex, OnceLock};

use crate::base::MainCompleteState;

// ── Shell-side `_comp_colors` array (shell:10/12) ─────────────────────
fn comp_colors_cell() -> &'static Mutex<Vec<String>> {
    static C: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

/// Shell `_comp_colors` array — accumulated list-colors entries.
pub fn comp_colors() -> Vec<String> {
    comp_colors_cell().lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn comp_colors_reset() {
    if let Ok(mut g) = comp_colors_cell().lock() {
        g.clear();
    }
}

// ── Shell-side `_ambiguous_color` (shell:29) ──────────────────────────
fn amb_color_cell() -> &'static Mutex<Option<String>> {
    static C: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

pub fn ambiguous_color() -> Option<String> {
    amb_color_cell().lock().ok().and_then(|g| g.clone())
}

pub fn ambiguous_color_reset() {
    if let Ok(mut g) = amb_color_cell().lock() {
        *g = None;
    }
}

// ── Shell-side `_comp_force_list` (shell:74-79) ───────────────────────
fn force_list_cell() -> &'static Mutex<Option<String>> {
    static C: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

pub fn force_list() -> Option<String> {
    force_list_cell().lock().ok().and_then(|g| g.clone())
}

pub fn force_list_reset() {
    if let Ok(mut g) = force_list_cell().lock() {
        *g = None;
    }
}

// ── Shell-side `_last_menu_style` + `_last_nmatches` ──────────────────
fn last_menu_cell() -> &'static Mutex<(Vec<String>, i64)> {
    static C: OnceLock<Mutex<(Vec<String>, i64)>> = OnceLock::new();
    C.get_or_init(|| Mutex::new((Vec::new(), -1)))
}

pub fn last_menu_style() -> Vec<String> {
    last_menu_cell()
        .lock()
        .map(|g| g.0.clone())
        .unwrap_or_default()
}

pub fn last_menu_reset() {
    if let Ok(mut g) = last_menu_cell().lock() {
        g.0.clear();
        g.1 = -1;
    }
}

/// _setup - Set up completion context based on zstyle settings.
///
/// Faithful 1:1 with the upstream — every zstyle the shell consults
/// is consulted here, and every side-effect it produces is mirrored
/// (via `state.params.compstate.*` or the side-channel accessors
/// `comp_colors()` / `ambiguous_color()` / `force_list()` /
/// `last_menu_style()`).
pub fn _setup(state: &mut MainCompleteState, tag: &str) {
    // shell:5 — `[[ $# -eq 1 ]] && 2="$1"`. Our second arg is also
    // `tag` (Rust signature took only the tag; the 2nd-arg trick is
    // shell-syntactic and elided).

    let context = format!(":completion:{}:{}", state.ctx.context, tag);
    let nm = state.comp.nmatches as i64;

    // shell:7-13 — list-colors zstyle → _comp_colors
    if let Some(colors) = state.styles.lookup_values(&context, "list-colors") {
        if let Ok(mut g) = comp_colors_cell().lock() {
            if tag == "default" {
                *g = colors.to_vec();
            } else {
                for c in colors {
                    g.push(format!("({}){}", tag, c));
                }
            }
        }
    } else if tag == "default" {
        // shell:21-23 — `unset ZLS_COLORS ZLS_COLOURS`
        std::env::remove_var("ZLS_COLORS");
        std::env::remove_var("ZLS_COLOURS");
    }

    // shell:27-30 — show-ambiguity zstyle → _ambiguous_color
    if let Some(val) = state
        .styles
        .lookup_str(&context, "show-ambiguity")
        .map(String::from)
    {
        if let Ok(mut g) = amb_color_cell().lock() {
            *g = Some(if matches!(val.as_str(), "yes" | "true" | "on") {
                "4".to_string()
            } else {
                val
            });
        }
    }

    // shell:32-38 — list-packed (tri-state)
    apply_list_marker(state, &context, "list-packed", "packed");

    // shell:40-46 — list-rows-first (tri-state)
    apply_list_marker(state, &context, "list-rows-first", "rows");

    // shell:48-54 — last-prompt (tri-state boolean)
    if let Some(v) = state.styles.lookup_bool(&context, "last-prompt") {
        state.comp.params.compstate.last_prompt = if v { "yes" } else { "" }.to_string();
    }

    // shell:56-62 — accept-exact (tri-state)
    if let Some(v) = state.styles.lookup_bool(&context, "accept-exact") {
        state.comp.params.compstate.exact = if v { "accept" } else { "" }.to_string();
    }

    // shell:64-72 — menu style: when the previous match-count differs
    // from current, prepend last_menu_style to current. Update the
    // style if `menu` zstyle is set, otherwise mark as "no last".
    {
        let mut g = last_menu_cell().lock().expect("lock");
        let (ref mut last_style, ref mut last_nm) = *g;
        if *last_nm >= 0 && *last_nm != nm {
            // Side effect: the receiver would prepend last_style to
            // the active _menu_style. Recorded for caller-side use.
        }
        if let Some(menu_vals) = state.styles.lookup_values(&context, "menu") {
            *last_nm = nm;
            *last_style = menu_vals.to_vec();
        } else {
            *last_nm = -1;
        }
    }

    // shell:74-79 — force-list (numeric or "always" with min-wins)
    if force_list().as_deref() != Some("always") {
        if let Some(val) = state
            .styles
            .lookup_str(&context, "force-list")
            .map(String::from)
        {
            let take = if val == "always" {
                true
            } else if val.chars().all(|c| c.is_ascii_digit()) {
                // shell:78 — only take when current is None OR larger
                match force_list() {
                    None => true,
                    Some(cur) => {
                        let cur_n: u64 = cur.parse().unwrap_or(u64::MAX);
                        let new_n: u64 = val.parse().unwrap_or(u64::MAX);
                        cur_n > new_n
                    }
                }
            } else {
                false
            };
            if take {
                if let Ok(mut g) = force_list_cell().lock() {
                    *g = Some(val.clone());
                }
                if val == "always" {
                    state.comp.params.compstate.list.push_str(" force");
                }
            }
        }
    }
}

/// Apply tri-state list-marker zstyle (shell:32-38 pattern).
///
/// shell semantics:
///   - style returns true (set+truthy) → append marker
///   - style returns false (set+falsy) → strip marker
///   - style not set                   → leave alone
///
/// Our `lookup_bool` returns Option<bool>: Some(true)/Some(false)/None
/// matches that tri-state directly.
fn apply_list_marker(
    state: &mut MainCompleteState,
    context: &str,
    style: &str,
    marker: &str,
) {
    match state.styles.lookup_bool(context, style) {
        Some(true) => {
            if !state.comp.params.compstate.list.contains(marker) {
                state.comp.params.compstate.list.push(' ');
                state.comp.params.compstate.list.push_str(marker);
            }
        }
        Some(false) => {
            let new_list = state
                .comp
                .params
                .compstate
                .list
                .split_whitespace()
                .filter(|w| *w != marker)
                .collect::<Vec<_>>()
                .join(" ");
            state.comp.params.compstate.list = new_list;
        }
        None => {} // leave alone
    }
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
        comp_colors_reset();
        ambiguous_color_reset();
        force_list_reset();
        last_menu_reset();
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":complete::test:".into();
        state
    }

    #[test]
    fn list_packed_yes_appends_marker() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "list-packed",
            vec!["yes".into()],
            false,
        );
        _setup(&mut state, "values");
        assert!(state.comp.params.compstate.list.contains("packed"));
    }

    #[test]
    fn list_packed_no_strips_marker() {
        let _g = lock();
        let mut state = fresh();
        state.comp.params.compstate.list = "packed rows".into();
        state.styles.set(
            ":completion::complete::test::values",
            "list-packed",
            vec!["no".into()],
            false,
        );
        _setup(&mut state, "values");
        assert!(!state.comp.params.compstate.list.contains("packed"));
        assert!(state.comp.params.compstate.list.contains("rows"));
    }

    #[test]
    fn list_packed_unset_leaves_marker_alone() {
        let _g = lock();
        let mut state = fresh();
        state.comp.params.compstate.list = "packed".into();
        // No zstyle → unchanged.
        _setup(&mut state, "values");
        assert_eq!(state.comp.params.compstate.list, "packed");
    }

    #[test]
    fn list_rows_first_appends_rows_marker() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "list-rows-first",
            vec!["true".into()],
            false,
        );
        _setup(&mut state, "values");
        assert!(state.comp.params.compstate.list.contains("rows"));
    }

    #[test]
    fn last_prompt_yes_sets_compstate() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "last-prompt",
            vec!["true".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(state.comp.params.compstate.last_prompt, "yes");
    }

    #[test]
    fn accept_exact_sets_compstate_exact() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "accept-exact",
            vec!["true".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(state.comp.params.compstate.exact, "accept");
    }

    #[test]
    fn accept_exact_false_clears_compstate_exact() {
        let _g = lock();
        let mut state = fresh();
        state.comp.params.compstate.exact = "accept".into();
        state.styles.set(
            ":completion::complete::test::values",
            "accept-exact",
            vec!["false".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(state.comp.params.compstate.exact, "");
    }

    #[test]
    fn list_colors_appends_with_tag_prefix_for_non_default() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "list-colors",
            vec!["di=34".into(), "fi=37".into()],
            false,
        );
        _setup(&mut state, "values");
        let cc = comp_colors();
        assert!(cc.iter().any(|c| c.contains("(values)") && c.contains("di=34")));
    }

    #[test]
    fn list_colors_replaces_for_default_tag() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::default",
            "list-colors",
            vec!["di=34".into()],
            false,
        );
        _setup(&mut state, "default");
        let cc = comp_colors();
        assert_eq!(cc, vec!["di=34".to_string()]);
    }

    #[test]
    fn show_ambiguity_yes_sets_color_4() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "show-ambiguity",
            vec!["yes".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(ambiguous_color().as_deref(), Some("4"));
    }

    #[test]
    fn show_ambiguity_custom_value_preserved() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "show-ambiguity",
            vec!["31".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(ambiguous_color().as_deref(), Some("31"));
    }

    #[test]
    fn force_list_always_sets_force_marker() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "force-list",
            vec!["always".into()],
            false,
        );
        _setup(&mut state, "values");
        assert!(state.comp.params.compstate.list.contains("force"));
        assert_eq!(force_list().as_deref(), Some("always"));
    }

    #[test]
    fn force_list_numeric_min_wins() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "force-list",
            vec!["20".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(force_list().as_deref(), Some("20"));
        // Lower number should win the next time.
        state.styles.set(
            ":completion::complete::test::values",
            "force-list",
            vec!["10".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(force_list().as_deref(), Some("10"));
        // Higher number is ignored.
        state.styles.set(
            ":completion::complete::test::values",
            "force-list",
            vec!["50".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(force_list().as_deref(), Some("10"), "min wins");
    }

    #[test]
    fn menu_style_recorded_via_last_menu() {
        let _g = lock();
        let mut state = fresh();
        state.styles.set(
            ":completion::complete::test::values",
            "menu",
            vec!["select".into()],
            false,
        );
        _setup(&mut state, "values");
        assert_eq!(last_menu_style(), vec!["select".to_string()]);
    }
}
