//! Native ZLE effects wiring — connects the fish-ported engines (autosuggest,
//! syntax_highlight, history_search) to the ZLE core loop and renderer.
//!
//! Rust-only extension glue (no C or fish counterpart file): the engines are
//! faithful ports; this module is the zshrs-specific plumbing that fish keeps
//! inside `Reader` (reader.rs) and zsh script plugins keep in widget wrappers.
//!
//! Call sites (each a single line in ported code, matching the zle_param_sync
//! boundary-call pattern):
//!   * `zlecore()` pre-dispatch  → `on_pre_widget(&thingy.nam)` — accept-suggestion
//!     and history-search key routing (fish handles these as reader commands,
//!     reader.rs:2600+; zsh plugins wrap widgets)
//!   * `zlecore()` post-dispatch → `on_post_widget(&thingy.nam)` — recompute
//!     suggestion + highlight before `zrefresh()` (fish: `update_autosuggestion` +
//!     `super_highlight_me_plenty` on each readline command)
//!   * `compute_render_attrs()`  → `native_render_attrs(...)` — the native layer,
//!     painted between HighlightManager regions and user `$region_highlight` so
//!     script plugins always win over the native engine
//!   * `domenuselect()` entry    → `on_completion_takeover()` — drop the ghost
//!     before a key loop that never returns to `zlecore()` starts rewriting the
//!     buffer (fish: `can_autosuggest()` is false while the pager owns the line,
//!     reader.rs:5519-5531)
//!
//! All three engines are ON by default in interactive mode (bare `zshrs -f` gets
//! the full fish experience). Opt-out: `ZSHRS_NATIVE_ZLE_FX=0`. When a script
//! plugin is detected at runtime (it defines its marker parameter), the native
//! engine for that feature yields to the plugin — no double rendering, existing
//! .zshrc setups keep working unchanged.

#![allow(non_snake_case)]

use crate::autosuggest::{self, AutosuggestionPortion};
use crate::history_search::{with_history_search, SearchDirection, SearchMode};
use crate::ported::params::getsparam;
use crate::ported::zle::zle_main::{ZLECS, ZLELINE, ZLELL};
use crate::ported::zle::zle_refresh::TextAttr;
use crate::syntax_highlight::{highlight_shell, HighlightColorResolver, HighlightSpec};
use crate::zle_file_tester::OperationContext;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::Mutex;

/// The native highlight overlay: one optional attr per BUFFER char (buffer-relative;
/// the renderer shifts by predisplay length), plus the ghost-text attr applied to
/// the whole POSTDISPLAY span while a suggestion is showing.
#[derive(Default)]
struct FxState {
    line_attrs: Vec<Option<TextAttr>>,
    /// cache key: the line the attrs were computed for
    hl_line: String,
    /// attr for the POSTDISPLAY ghost span (None = no native ghost active)
    ghost_attr: Option<TextAttr>,
    /// the line this module last wrote into the buffer from a history search,
    /// used to detect user edits that must end the search
    search_placed: Option<String>,
}

static FX: Mutex<Option<FxState>> = Mutex::new(None);

fn with_fx<R>(f: impl FnOnce(&mut FxState) -> R) -> R {
    let mut guard = FX.lock().unwrap();
    f(guard.get_or_insert_with(Default::default))
}

/// Kill switch: `ZSHRS_NATIVE_ZLE_FX=0` (param or environment) disables all three.
pub fn enabled() -> bool {
    match getsparam("ZSHRS_NATIVE_ZLE_FX") {
        Some(v) => v != "0",
        None => std::env::var("ZSHRS_NATIVE_ZLE_FX")
            .map(|v| v != "0")
            .unwrap_or(true),
    }
}

/// OPT-IN POLICY (user ruling): `zshrs -f` must behave IDENTICALLY to
/// `zsh -f` for parity purposes — every native engine defaults OFF and is
/// enabled per-feature in `~/.zshrs/zshrs.toml` `[zle]`. The env kill
/// switch ZSHRS_NATIVE_ZLE_FX=0 still force-disables everything.
///
/// When enabled, NATIVE WINS over loaded script plugins (a full rc loads
/// zsh-autosuggestions/z-sy-h/substring-search, whose zpty/async paths
/// don't function under zshrs). Coexistence is benign by construction:
///   * suggestions: native fills $POSTDISPLAY in the post-widget hook —
///     after any plugin widget ran — and the plugin's accept path reads
///     $POSTDISPLAY, so either accept route works;
///   * search: the plugin's widget NAMES are intercepted pre-dispatch and
///     handled natively — its shfuncs simply never run;
///   * highlighting: user $region_highlight (what z-sy-h writes) paints
///     ABOVE the native layer, so a functioning plugin overrides cleanly.
fn autosuggest_active() -> bool {
    enabled() && crate::config::current().zle.autosuggest
}

fn highlight_active() -> bool {
    enabled() && crate::config::current().zle.syntax_highlight
}

fn search_active() -> bool {
    enabled() && crate::config::current().zle.history_search
}

/// Native autopair yields to the zsh-autopair script plugin when loaded
/// (its `autopair-insert` widget is registered), and stays out of
/// incremental search exactly as the plugin's isearch bindings do
/// (autopair.zsh:211/216/223-225).
fn autopair_engine_active() -> bool {
    enabled()
        && crate::config::current().zle.autopair
        && crate::ported::zle::zle_hist::ISEARCH_ACTIVE.load(SeqCst) == 0
        && !crate::ported::zle::zle_thingy::rthingy_nocreate("autopair-insert")
}

fn current_line() -> String {
    ZLELINE.lock().unwrap().iter().collect()
}

fn current_cursor() -> usize {
    ZLECS.load(SeqCst)
}

/// Replace the whole editor buffer (history search result placement, accept).
fn set_buffer(text: &str, cursor: usize) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    *ZLELINE.lock().unwrap() = chars;
    ZLELL.store(n, SeqCst);
    ZLECS.store(cursor.min(n), SeqCst);
    // zle_utils.rs:565-569 (zsh Src/Zle/zle_utils.c:827-828): every buffer
    // edit clamps `viinsbegin > zlecs → 0`, or vi insert mode refuses to
    // delete text left of a stale insertion mark. Whole-line replacement
    // here must do the same or backspace dies after normal→insert.
    if crate::ported::zle::zle_main::VIINSBEGIN.load(SeqCst) > ZLECS.load(SeqCst) {
        crate::ported::zle::zle_main::VIINSBEGIN.store(0, SeqCst);
    }
}

/// Everything right of the cursor is autopair-inserted closers (`)`, `]`,
/// `}`, quotes, pad spaces) — see the accept-gate comment at the call site.
fn tail_is_autopair_closers() -> bool {
    let line = current_line();
    let cursor = current_cursor().min(line.chars().count());
    let tail: Vec<char> = line.chars().skip(cursor).collect();
    if tail.is_empty() {
        return false; // that's cursor_at_end's case
    }
    let cfg = crate::autopair::AutopairConfig::from_params();
    tail.iter()
        .all(|&c| cfg.pairs.iter().any(|(_, close)| *close == c))
}

/// Cursor sits at line end (vi command mode's inclusive cursor counts the last
/// cell as "at end" — fish CursorEndMode::Inclusive, reader.rs:5612-5618).
fn cursor_at_end() -> bool {
    let cs = ZLECS.load(SeqCst);
    let ll = ZLELL.load(SeqCst);
    if crate::ported::zle::zle_main::in_vi_cmd_mode() {
        cs + 1 >= ll
    } else {
        cs >= ll
    }
}

/// Widgets that accept the whole suggestion when the cursor is at line end
/// (zsh-autosuggestions ZSH_AUTOSUGGEST_ACCEPT_WIDGETS defaults).
const ACCEPT_FULL_WIDGETS: &[&str] = &[
    "forward-char",
    "vi-forward-char",
    "end-of-line",
    "vi-end-of-line",
    "vi-add-eol",
];

/// Widgets that accept one word of the suggestion
/// (zsh-autosuggestions ZSH_AUTOSUGGEST_PARTIAL_ACCEPT_WIDGETS defaults).
const ACCEPT_PARTIAL_WIDGETS: &[&str] = &[
    "forward-word",
    "emacs-forward-word",
    "vi-forward-word",
    "vi-forward-word-end",
    "vi-forward-blank-word",
    "vi-forward-blank-word-end",
];

/// Deleting widgets suppress the suggestion until the next insert
/// (fish reader.rs:679-680; zsh-autosuggestions clear-on-modify set).
const DELETE_WIDGETS: &[&str] = &[
    "backward-delete-char",
    "vi-backward-delete-char",
    "delete-char",
    "delete-char-or-list",
    "backward-kill-word",
    "vi-backward-kill-word",
    "backward-kill-line",
    "kill-word",
    "kill-line",
    "kill-whole-line",
    "kill-buffer",
    "vi-delete",
    "vi-delete-char",
    "vi-kill-line",
    "vi-kill-eol",
    "vi-change",
    "vi-change-eol",
    "vi-change-whole-line",
];

const INSERT_WIDGETS: &[&str] = &[
    "self-insert",
    "self-insert-unmeta",
    "magic-space",
    "bracketed-paste",
    "quoted-insert",
    "vi-put-after",
    "vi-put-before",
    "yank",
];

/// History-search-up widget names → mode. `up-line-or-history` is the stock
/// up-arrow binding: with text typed it searches SUBSTRING-wise — the
/// zsh-history-substring-search behavior the user's fingers know ("{1..5}"
/// typed mid-command must match entries CONTAINING it, not starting with
/// it). Only the explicitly prefix-named widgets keep prefix semantics.
/// An empty line still falls through to the plain history walk.
fn up_search_mode(widget: &str) -> Option<SearchMode> {
    match widget {
        "up-line-or-search" | "history-beginning-search-backward" => Some(SearchMode::Prefix),
        "history-substring-search-up" => Some(SearchMode::Line),
        "up-line-or-history" | "up-history" => Some(SearchMode::Line),
        _ => None,
    }
}

fn down_search_widget(widget: &str) -> bool {
    matches!(
        widget,
        "down-line-or-search"
            | "history-beginning-search-forward"
            | "history-substring-search-down"
            | "down-line-or-history"
            | "down-history"
    )
}

/// Pre-dispatch hook. Returns true when the key was handled natively and the
/// bound widget must NOT run.
pub fn on_pre_widget(widget: &str) -> bool {
    if !enabled() {
        return false;
    }

    // ---- Autopair (port of hlissner/zsh-autopair, extensions/autopair.rs).
    // The plugin rebinds pair keys + backspace to wrapper widgets
    // (autopair.zsh:200-226); zshrs intercepts the same widgets here.
    if matches!(
        widget,
        "self-insert"
            | "backward-delete-char"
            | "vi-backward-delete-char"
            | "backward-delete-word"
            | "backward-kill-word"
    ) && autopair_engine_active()
    {
        // KEYS: the invoking byte — pair chars are all ASCII, so a multibyte
        // lastchar can never name a pair and falls through.
        let key = {
            let lc = crate::ported::zle::compcore::LASTCHAR.load(SeqCst);
            (0..=0x7f).contains(&lc).then(|| lc as u8 as char)
        };
        let line = current_line();
        let cursor = current_cursor().min(line.chars().count());
        let lbuf: String = line.chars().take(cursor).collect();
        let rbuf: String = line.chars().skip(cursor).collect();
        let cfg = crate::autopair::AutopairConfig::from_params();
        match crate::autopair::decide(&cfg, widget, key, &lbuf, &rbuf) {
            crate::autopair::Action::InsertPair(open, close) => {
                // ap:159-163 — LBUFFER+=$1; RBUFFER="$2$RBUFFER".
                let new_line = format!("{lbuf}{open}{close}{rbuf}");
                set_buffer(&new_line, cursor + 1);
                return true;
            }
            crate::autopair::Action::SkipOver => {
                // ap:171/183 — zle forward-char.
                ZLECS.store((cursor + 1).min(ZLELL.load(SeqCst)), SeqCst);
                return true;
            }
            crate::autopair::Action::DeleteRightThenPassthrough => {
                // ap:190/195 — RBUFFER=${RBUFFER:1}; then the original
                // deleting widget runs. The passthrough MUST be able to
                // actually delete leftward, or the pair collapses
                // asymmetrically (right char gone, left refused):
                // vi-backward-delete-char stops at the insertion start
                // (viinsbegin, zle_vi.c:78 classic-vi rule), so gate on it.
                let vi_blocked = widget == "vi-backward-delete-char"
                    && cursor <= crate::ported::zle::zle_main::VIINSBEGIN.load(SeqCst);
                if !vi_blocked {
                    let rest: String = rbuf.chars().skip(1).collect();
                    let new_line = format!("{lbuf}{rest}");
                    set_buffer(&new_line, cursor);
                }
                // fall through: return false so the bound widget deletes left
            }
            crate::autopair::Action::Passthrough => (),
        }
        // No autopair action (or delete fall-through): continue into the
        // remaining pre-widget checks and the originally bound widget.
    }

    // ---- Suggestion accept (fish reader.rs:3607-3621 ForwardChar →
    // is_at_autosuggestion → accept_autosuggestion(Count(MAX))).
    //
    // fish gates on is_at_autosuggestion (cursor == search range end,
    // reader.rs:5620-5633) — with no autopair, `echo "` puts the cursor AT
    // the end, so ONE right-arrow accepts to EOL. zshrs autopair inserts the
    // closer, parking the cursor before it; a literal end-gate would demand
    // a second press. A tail made purely of pair-closers is transparent for
    // the gate — `echo "|"` + → accepts the whole ghost, exactly like fish.
    if autosuggest_active() && (cursor_at_end() || tail_is_autopair_closers()) {
        let portion = if ACCEPT_FULL_WIDGETS.contains(&widget) {
            Some(AutosuggestionPortion::Count(usize::MAX))
        } else if ACCEPT_PARTIAL_WIDGETS.contains(&widget) {
            Some(AutosuggestionPortion::Word)
        } else {
            None
        };
        if let Some(portion) = portion {
            let line = current_line();
            let accepted = autosuggest::with_state(|st| {
                if !autosuggest::is_at_line_with_autosuggestion(st, &line, current_cursor()) {
                    return None;
                }
                autosuggest::accept_autosuggestion(st, portion)
            });
            if let Some((range, replacement)) = accepted {
                let chars: Vec<char> = line.chars().collect();
                let mut new_line: String = chars[..range.start].iter().collect();
                new_line.push_str(&replacement);
                new_line.extend(chars[range.end.min(chars.len())..].iter());
                let new_cursor = range.start + replacement.chars().count();
                set_buffer(&new_line, new_cursor);
                crate::ported::zle::zle_params::set_postdisplay(Some(""));
                with_fx(|fx| fx.ghost_attr = None);
                return true;
            }
        }
    }

    // ---- History search (fish reader.rs up-or-search; plugin:
    // zsh-history-substring-search widgets).
    if search_active() {
        if let Some(mode) = up_search_mode(widget) {
            let line = current_line();
            // Stock up-arrow on an empty line: plain history walk, not a search
            // (fish up-or-search does exactly this).
            if line.is_empty() && matches!(widget, "up-line-or-history" | "up-history") {
                return false;
            }
            let placed = with_history_search(|hs| {
                if !hs.active() {
                    hs.reset_to_mode(line.clone(), mode, 0);
                }
                if hs.move_in_direction(SearchDirection::Backward) {
                    Some(hs.current_result().to_owned())
                } else {
                    None // no older match: keep the buffer as-is
                }
            });
            autosuggest::with_state(|st| st.history_search_active = true);
            if let Some(text) = placed {
                let end = text.chars().count();
                set_buffer(&text, end);
                with_fx(|fx| fx.search_placed = Some(text));
            }
            return true;
        }
        if down_search_widget(widget) {
            let active = with_history_search(|hs| hs.active());
            if !active {
                return false; // stock down-arrow behavior
            }
            let placed = with_history_search(|hs| {
                if hs.move_in_direction(SearchDirection::Forward) || hs.is_at_present() {
                    Some(hs.current_result().to_owned())
                } else {
                    None
                }
            });
            if let Some(text) = placed {
                let end = text.chars().count();
                set_buffer(&text, end);
                let at_present = with_history_search(|hs| hs.is_at_present());
                with_fx(|fx| fx.search_placed = Some(text));
                if at_present {
                    // Back at the original line: the search ends.
                    with_history_search(|hs| hs.reset());
                    autosuggest::with_state(|st| st.history_search_active = false);
                    with_fx(|fx| fx.search_placed = None);
                }
            }
            return true;
        }
    }

    false
}

/// Post-dispatch hook: runs after every widget, before `zrefresh()`.
pub fn on_post_widget(widget: &str) {
    if !enabled() {
        return;
    }
    let line = current_line();
    let cursor = current_cursor();

    // ---- History search invalidation: any edit to the placed line ends the
    // search (the plugin resets when the buffer changes; fish when a non-search
    // command runs).
    if search_active() {
        let edited = with_fx(|fx| {
            fx.search_placed
                .as_ref()
                .map(|placed| placed != &line)
                .unwrap_or(false)
        });
        if edited {
            with_history_search(|hs| hs.reset());
            autosuggest::with_state(|st| st.history_search_active = false);
            with_fx(|fx| fx.search_placed = None);
        }
    }

    // ---- Line handed to the executor: ghost/search teardown BEFORE the
    // final repaint, and NO recompute. Without this, the post-accept-line
    // pass recomputed a suggestion for the just-accepted buffer and re-set
    // POSTDISPLAY, so the terminal scrolled with ghost text baked into every
    // executed line ("l" + ghost "ocal -a a=(…)" rendered as if typed).
    // DONE is the widget-set editing-finished flag (zle_misc.rs:2124) —
    // covers accept-line wrappers and user widgets calling `zle accept-line`.
    //
    // line_attrs are deliberately KEPT: zlecore's final zrefresh (the paint
    // that scrolls the accepted line into history) still runs after this
    // hook, and clearing the overlay here erased the syntax colors of every
    // executed line in scrollback. The full clear happens in on_line_finish
    // at zleread exit, after the display is finalized.
    if crate::ported::zle::zle_misc::DONE.load(SeqCst) != 0
        || matches!(
            widget,
            "accept-line"
                | "vi-accept-line"
                | "accept-line-and-down-history"
                | "accept-and-hold"
                | "accept-and-infer-next-history"
                | "send-break"
        )
    {
        with_fx(|fx| {
            fx.ghost_attr = None;
            fx.search_placed = None;
        });
        autosuggest::with_state(autosuggest::on_line_finish);
        with_history_search(|hs| hs.reset());
        crate::ported::zle::zle_params::set_postdisplay(Some(""));
        return;
    }

    // ---- Autosuggestion (fish reader.rs:5575 update_autosuggestion per command).
    if autosuggest_active() {
        let t0 = std::time::Instant::now();
        autosuggest::with_state(|st| {
            if DELETE_WIDGETS.contains(&widget) {
                autosuggest::on_delete(st);
            } else if INSERT_WIDGETS.contains(&widget) {
                autosuggest::on_insert(st, &line);
            }
        });
        // Budgeted: fish computes on a background thread with unbounded time;
        // the synchronous pass self-cancels so typing latency stays bounded
        // (candidate validation degrades to first-match, plugin parity).
        let ctx = OperationContext::with_budget(std::time::Duration::from_millis(budget_ms(
            "ZSHRS_ZLE_AUTOSUGGEST_BUDGET_MS",
            8,
        )));
        autosuggest::update_autosuggestion(
            &line,
            cursor,
            &autosuggest::history_commands_newest_first,
            &ctx,
        );
        let suffix = autosuggest::with_state(|st| st.autosuggestion.suffix().to_owned());
        crate::ported::zle::zle_params::set_postdisplay(Some(&suffix));
        let ghost = if suffix.is_empty() {
            None
        } else {
            Some(zattr_to_text_attr(
                HighlightColorResolver::resolve_spec_uncached(&HighlightSpec::with_fg(
                    crate::syntax_highlight::HighlightRole::autosuggestion,
                )),
            ))
        };
        with_fx(|fx| fx.ghost_attr = ghost);
        let dt = t0.elapsed();
        if dt.as_millis() >= 20 {
            tracing::debug!(target: "zle_fx", ms = dt.as_millis() as u64, "slow autosuggest pass");
        }
    }

    // ---- Syntax highlighting (fish reader.rs super_highlight_me_plenty).
    if highlight_active() {
        let t0 = std::time::Instant::now();
        with_fx(|fx| {
            if fx.hl_line != line {
                if line.is_empty() {
                    fx.line_attrs.clear();
                } else {
                    // The lexer wants the metafied form; map spans back to raw
                    // char indices when metafication changed anything.
                    let meta = crate::ported::utils::metafy(&line);
                    let mut colors: Vec<HighlightSpec> = Vec::new();
                    // Budgeted (see autosuggest above): directory walks and
                    // PATH scans self-cancel; a cancelled pass still colors
                    // everything token-classified so far.
                    let ctx = OperationContext::with_budget(std::time::Duration::from_millis(
                        budget_ms("ZSHRS_ZLE_HIGHLIGHT_BUDGET_MS", 8),
                    ));
                    highlight_shell(&meta, &mut colors, &ctx, /*io_ok=*/ true, Some(cursor));

                    let mut resolver = HighlightColorResolver::new();
                    let raw_len = line.chars().count();
                    let mut attrs: Vec<Option<TextAttr>> = vec![None; raw_len];
                    if meta == line {
                        for (i, spec) in colors.iter().enumerate().take(raw_len) {
                            attrs[i] = spec_to_attr(&mut resolver, spec);
                        }
                    } else {
                        // metafied → raw index map: each raw char occupies 1 or 2
                        // metafied cells.
                        let mut mi = 0usize;
                        for (ri, ch) in line.chars().enumerate() {
                            let w = crate::ported::utils::metafy(&ch.to_string())
                                .chars()
                                .count();
                            if let Some(spec) = colors.get(mi) {
                                attrs[ri] = spec_to_attr(&mut resolver, spec);
                            }
                            mi += w;
                        }
                    }
                    fx.line_attrs = attrs;
                }
                fx.hl_line = line.clone();
            }
        });

        // Substring-search found-range emphasis
        // (HISTORY_SUBSTRING_SEARCH_HIGHLIGHT_FOUND compat; default per plugin).
        if search_active() {
            let found = with_history_search(|hs| hs.search_range_if_active());
            if let Some((start, len)) = found {
                let spec = getsparam("HISTORY_SUBSTRING_SEARCH_HIGHLIGHT_FOUND")
                    .unwrap_or_else(|| "bg=magenta,fg=white,bold".to_owned());
                let (mask_on, _off) = crate::ported::prompt::match_highlight(&spec);
                let attr = zattr_to_text_attr(mask_on);
                with_fx(|fx| {
                    for i in start..(start + len).min(fx.line_attrs.len()) {
                        fx.line_attrs[i] = Some(attr);
                    }
                });
            }
        }
        let dt = t0.elapsed();
        if dt.as_millis() >= 20 {
            tracing::debug!(target: "zle_fx", ms = dt.as_millis() as u64, "slow highlight pass");
        }
    }
}

/// Millisecond budget from a param/env override, with a default.
fn budget_ms(name: &str, default: u64) -> u64 {
    getsparam(name)
        .or_else(|| std::env::var(name).ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

/// Renderer callback — merge the native overlay into the combined
/// pre+line+post attr array. Called by `compute_render_attrs` between the
/// HighlightManager regions and user `$region_highlight` layers.
pub fn native_render_attrs(attrs: &mut [Option<TextAttr>], pre_len: usize, line_len: usize) {
    if !enabled() {
        return;
    }
    with_fx(|fx| {
        // Line highlight, buffer-relative → +pre_len.
        for (i, a) in fx.line_attrs.iter().enumerate().take(line_len) {
            if let Some(a) = a {
                if let Some(slot) = attrs.get_mut(pre_len + i) {
                    *slot = Some(*a);
                }
            }
        }
        // Ghost text: the whole POSTDISPLAY span.
        if let Some(ghost) = fx.ghost_attr {
            for slot in attrs.iter_mut().skip(pre_len + line_len) {
                *slot = Some(ghost);
            }
        }
    });
}

fn spec_to_attr(resolver: &mut HighlightColorResolver, spec: &HighlightSpec) -> Option<TextAttr> {
    if *spec == HighlightSpec::default() {
        return None;
    }
    let z = resolver.resolve_spec(spec);
    if z == 0 {
        return None;
    }
    Some(zattr_to_text_attr(z))
}

/// Inverse of zle_refresh's `to_zattr` closure (zle_refresh.rs:1339-1361).
fn zattr_to_text_attr(a: crate::ported::zsh_h::zattr) -> TextAttr {
    use crate::ported::zsh_h::{
        zattr, TXTBGCOLOUR, TXTBOLDFACE, TXTFGCOLOUR, TXTSTANDOUT, TXTUNDERLINE,
        TXT_ATTR_BG_COL_SHIFT, TXT_ATTR_FG_COL_SHIFT,
    };
    TextAttr {
        bold: a & TXTBOLDFACE != 0,
        underline: a & TXTUNDERLINE != 0,
        standout: a & TXTSTANDOUT != 0,
        blink: false,
        fg_color: (a & TXTFGCOLOUR != 0)
            .then(|| ((a >> TXT_ATTR_FG_COL_SHIFT) & 0xff as zattr) as u8),
        bg_color: (a & TXTBGCOLOUR != 0)
            .then(|| ((a >> TXT_ATTR_BG_COL_SHIFT) & 0xff as zattr) as u8),
    }
}

/// Completion-takeover reset — called when a completion key loop is about to
/// take over key reading and rewrite the buffer WITHOUT going back through
/// `zlecore()`'s dispatch (and therefore without `on_post_widget`).
///
/// `domenuselect` (complist.rs, port of c:Src/Zle/complist.c:2383) reads keys
/// itself and calls `selfinsert`/`menucomplete` directly (c:2756-2779), so the
/// suggestion computed for the pre-TAB buffer stays in `$POSTDISPLAY` and in
/// `ghost_attr` for the whole menu. The renderer keeps appending it to a buffer
/// that is now longer: typing `g<TAB>i` under `menu select interactive` drew
/// `g` + `i` + grey `it status` as `giit status` while the status line
/// correctly read `interactive: gi[]`.
///
/// fish's equivalent guard is `can_autosuggest()` (vendor/fish/reader/reader.rs:5519-5531),
/// which returns false unless the active edit line is the command line — while
/// the pager owns input there is no suggestion; `update_autosuggestion`
/// (reader.rs:5575-5580) then clears it outright.
///
/// Only the NATIVE ghost is dropped: `$POSTDISPLAY` is cleared only when this
/// module owns it (`ghost_attr` is `Some`), so a user widget's own
/// `$POSTDISPLAY` survives menu selection untouched. The next widget that runs
/// through `zlecore` re-enters `on_post_widget` and recomputes normally.
pub fn on_completion_takeover() {
    if !enabled() {
        return;
    }
    let owned = with_fx(|fx| {
        let owned = fx.ghost_attr.is_some();
        fx.ghost_attr = None;
        owned
    });
    if owned {
        crate::ported::zle::zle_params::set_postdisplay(Some(""));
        // fish reader.rs:5577-5578 — the `!can_autosuggest()` arm of
        // `update_autosuggestion`: drop the in-flight request key AND the
        // suggestion. `last_request_line` is this port's sync analog of
        // `in_flight_autosuggest_request` (autosuggest.rs:151-152); leaving it
        // set would make the recompute after the menu exits a no-op whenever
        // the menu ended on the same buffer text it started with.
        autosuggest::with_state(|st| {
            st.last_request_line.clear();
            st.autosuggestion.clear();
        });
    }
}

/// Line-finish reset — called when ZLE hands the line to the executor so no
/// stale overlay bleeds into the next prompt.
pub fn on_line_finish() {
    with_fx(|fx| {
        fx.line_attrs.clear();
        fx.hl_line.clear();
        fx.ghost_attr = None;
        fx.search_placed = None;
    });
    autosuggest::with_state(autosuggest::on_line_finish);
    with_history_search(|hs| hs.reset());
    crate::ported::zle::zle_params::set_postdisplay(Some(""));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zattr_round_trip() {
        use crate::ported::zsh_h::{TXTBOLDFACE, TXTFGCOLOUR, TXTUNDERLINE, TXT_ATTR_FG_COL_SHIFT};
        let a = TXTBOLDFACE | TXTUNDERLINE | TXTFGCOLOUR | ((2u64) << TXT_ATTR_FG_COL_SHIFT);
        let t = zattr_to_text_attr(a);
        assert!(t.bold && t.underline && !t.standout);
        assert_eq!(t.fg_color, Some(2));
        assert_eq!(t.bg_color, None);
    }

    #[test]
    fn accept_widget_tables_are_disjoint() {
        for w in ACCEPT_FULL_WIDGETS {
            assert!(!ACCEPT_PARTIAL_WIDGETS.contains(w));
            assert!(!DELETE_WIDGETS.contains(w));
        }
        for w in DELETE_WIDGETS {
            assert!(!INSERT_WIDGETS.contains(w));
        }
    }
}
