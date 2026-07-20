//! Port of fish-shell's autosuggestion state machine from `reader/reader.rs`
//! (vendor/fish/reader/reader.rs:5231-5760) — native fish-style autosuggestions.
//!
//! zsh-autosuggestions is a script-level recreation of this fish machinery; this
//! ports the origin directly and reads the plugin's config surface
//! (`ZSH_AUTOSUGGEST_HIGHLIGHT_STYLE`, `ZSH_AUTOSUGGEST_STRATEGY`,
//! `ZSH_AUTOSUGGEST_BUFFER_MAX_SIZE`) so existing user config applies unchanged.
//!
//! zshrs substrate swaps (each cited at its site):
//!   * fish `Reader` fields         → module-level `AUTOSUGGEST_STATE` (the
//!     zle_param_sync extension-state pattern)
//!   * fish `History`/`HistorySearch` → `HistoryEngine::search_prefix`
//!     (extensions/history.rs:349) + the in-memory `hist_ring` (newest at index 0,
//!     ported/hist.rs:2617-2621)
//!   * background debouncer          → synchronous compute in the post-widget slot
//!     (the SQLite prefix probe is a single indexed LIKE; fish went async because of
//!     file-IO validation, which here is budgeted by the OperationContext cancel flag)
//!   * rendering                     → the suggestion suffix is published to
//!     POSTDISPLAY by the ZLE wiring; this module owns only the state machine
//!
//! fish strategies vs zsh-autosuggestions strategies: fish searches history then
//! falls back to completions (reader.rs:5462-5495). zsh-autosuggestions names these
//! `history`, `completion`, and adds `match_prev_cmd`. `history` and
//! `match_prev_cmd` are implemented; `completion` falls back to `history` until the
//! native completion engine exposes an autosuggest-grade entry point.

#![allow(non_snake_case)]

use crate::ported::params::getsparam;
use crate::zle_file_tester::OperationContext;
use std::ops::Range;
use std::sync::Mutex;

/// fish:reader.rs:5231-5245 — `Autosuggestion`.
#[derive(Default, Clone, Debug)]
pub struct Autosuggestion {
    /// fish:5232-5233 — The text to use, as an extension/replacement of the current
    /// line.
    pub text: String,

    /// fish:5235-5237 — The range within the commandline that was searched. Always
    /// at least whole line. (Char indices.)
    pub search_string_range: Range<usize>,

    /// fish:5239-5241 — If the autosuggestion is a case insensitive (prefix) match,
    /// this indicates the number of code points we matched in the lowercase mapping
    /// of the suggestion.
    pub icase_matched_codepoints: Option<usize>,

    /// fish:5243-5244 — Whether the autosuggestion is a whole match from history.
    pub is_whole_item_from_history: bool,
}

impl Autosuggestion {
    // fish:5248-5251 — Clear our contents.
    pub fn clear(&mut self) {
        self.text.clear();
    }

    // fish:5253-5256 — Return whether we have empty text.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The suffix beyond what the user typed — what POSTDISPLAY shows.
    pub fn suffix(&self) -> &str {
        let typed = self.search_string_range.len();
        match self.text.char_indices().nth(typed) {
            Some((byte, _)) => &self.text[byte..],
            None => "",
        }
    }
}

/// fish:reader.rs:5259-5303 — `AutosuggestionResult`.
#[derive(Default)]
pub struct AutosuggestionResult {
    // fish:5262-5263 — The autosuggestion.
    pub autosuggestion: Autosuggestion,

    // fish:5265-5266 — The commandline this result is based off.
    pub command_line: String,
}

impl std::ops::Deref for AutosuggestionResult {
    type Target = Autosuggestion;
    fn deref(&self) -> &Self::Target {
        &self.autosuggestion
    }
}

impl AutosuggestionResult {
    /// fish:5280-5297 — `new`.
    fn new(
        command_line: String,
        search_string_range: Range<usize>,
        text: String,
        icase_matched_codepoints: Option<usize>,
        is_whole_item_from_history: bool,
    ) -> Self {
        Self {
            autosuggestion: Autosuggestion {
                text,
                search_string_range,
                icase_matched_codepoints,
                is_whole_item_from_history,
            },
            command_line,
        }
    }

    /// fish:5299-5302 — The line which was searched for.
    fn search_string(&self) -> String {
        self.command_line
            .chars()
            .skip(self.search_string_range.start)
            .take(self.search_string_range.len())
            .collect()
    }
}

/// fish:reader.rs:5509-5516 — `AutosuggestionPortion`.
pub enum AutosuggestionPortion {
    Count(usize),
    Line,
    /// fish's PerMoveWordStyle, zsh spelling: one forward-word per `$WORDCHARS`.
    Word,
}

/// The zsh-autosuggestions strategy config (`ZSH_AUTOSUGGEST_STRATEGY`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strategy {
    History,
    MatchPrevCmd,
}

/// !!! WARNING: RUST-ONLY ADAPTER — NO DIRECT FISH COUNTERPART SHAPE !!!
/// fish keeps these on `Reader` (reader.rs:631-692); zshrs has no reader object, so
/// the extension owns them module-globally (one editor per process), following the
/// zle_param_sync extension-state pattern.
#[derive(Default)]
pub struct AutosuggestState {
    /// fish:reader.rs:671-672 — The current autosuggestion.
    pub autosuggestion: Autosuggestion,
    /// fish:reader.rs:673-674 — A previously valid autosuggestion (restored when a
    /// deleted char is retyped).
    pub saved_autosuggestion: Option<Autosuggestion>,
    /// fish:reader.rs:679-680 — When backspacing, we temporarily suppress
    /// autosuggestions.
    pub suppress_autosuggestion: bool,
    /// fish:reader.rs:748-752 — the text of the most recent request; used to skip
    /// duplicate recomputes (the sync analog of in_flight_autosuggest_request).
    pub last_request_line: String,
    /// Set by the history-search module while a search is active
    /// (fish:5525 `history_search.is_at_present()`).
    pub history_search_active: bool,
}

/// Module-global state.
pub static AUTOSUGGEST_STATE: Mutex<Option<AutosuggestState>> = Mutex::new(None);

/// Run f over the state, initializing on first touch.
pub fn with_state<R>(f: impl FnOnce(&mut AutosuggestState) -> R) -> R {
    let mut guard = AUTOSUGGEST_STATE.lock().unwrap();
    f(guard.get_or_insert_with(Default::default))
}

/// fish:wcstringutil `string_prefixes_string_maybe_case_insensitive`.
fn string_prefixes_string_maybe_case_insensitive(icase: bool, prefix: &str, value: &str) -> bool {
    if icase {
        let mut vc = value.chars();
        prefix.chars().all(|pc| match vc.next() {
            Some(c) => c.to_lowercase().eq(pc.to_lowercase()),
            None => false,
        })
    } else {
        value.starts_with(prefix)
    }
}

/// A history source for the suggestion search: yields commands newest-first.
/// Injected so tests can drive the state machine without a live history engine;
/// the ZLE wiring passes `history_commands_newest_first`.
pub type HistorySource<'a> = dyn Fn(&str, usize) -> Vec<String> + 'a;

/// Default source: SQLite session engine prefix query (history.rs:349), falling
/// back to the in-memory ring (hist.rs:4841, newest at index 0).
pub fn history_commands_newest_first(prefix: &str, limit: usize) -> Vec<String> {
    // SQLite prefix probe — the case-sensitive GLOB variant; the LIKE-based
    // search_prefix full-scans (no index) and is too slow per keystroke.
    let from_db = crate::history::with_session_engine(|eng| {
        eng.search_prefix_cs(prefix, limit)
            .map(|entries| entries.into_iter().map(|e| e.command).collect::<Vec<_>>())
            .unwrap_or_default()
    })
    .unwrap_or_default();
    if !from_db.is_empty() {
        return from_db;
    }
    // In-memory ring fallback: newest at position 0 (hist.rs:2617-2621), skip
    // foreign entries like hconsearch does (hist.rs:2614-2631).
    let ring = crate::ported::hist::hist_ring.lock().unwrap();
    ring.iter()
        .filter(|e| !e.node.nam.is_empty() && e.node.nam.starts_with(prefix))
        .take(limit)
        .map(|e| e.node.nam.clone())
        .collect()
}

/// The last executed command, for the `match_prev_cmd` strategy.
fn previous_command() -> Option<String> {
    let ring = crate::ported::hist::hist_ring.lock().unwrap();
    ring.first().map(|e| e.node.nam.clone())
}

/// Read `$ZSH_AUTOSUGGEST_STRATEGY` (defaults to `history`). The plugin
/// declares it as an ARRAY (`ZSH_AUTOSUGGEST_STRATEGY=( match_prev_cmd )`) —
/// read it as one, falling back to a scalar spelling.
pub fn configured_strategies() -> Vec<Strategy> {
    let raw = crate::ported::params::getaparam("ZSH_AUTOSUGGEST_STRATEGY")
        .map(|v| v.join(" "))
        .or_else(|| getsparam("ZSH_AUTOSUGGEST_STRATEGY"))
        .unwrap_or_default();
    let mut out: Vec<Strategy> = raw
        .split_whitespace()
        .filter_map(|s| match s {
            "history" | "completion" => Some(Strategy::History),
            "match_prev_cmd" => Some(Strategy::MatchPrevCmd),
            _ => None,
        })
        .collect();
    if out.is_empty() {
        out.push(Strategy::History);
    }
    out
}

/// fish:reader.rs:5305-5507 — `get_autosuggestion_performer`, collapsed to a
/// synchronous compute (see module header). Whole-line Prefix search; the
/// LinePrefix continuation-line pass (fish:5329-5350) is skipped — the ZLE buffer
/// hands us single logical lines.
pub fn compute_autosuggestion(
    command_line: &str,
    cursor_pos: usize,
    source: &HistorySource<'_>,
    ctx: &OperationContext,
) -> AutosuggestionResult {
    let nothing = AutosuggestionResult::default();
    if ctx.check_cancel() {
        return nothing; // fish:5320-5322
    }

    let line_len = command_line.chars().count();
    let range = 0..line_len;
    if range.is_empty() {
        return nothing; // fish:5333-5335
    }
    let search_string = command_line;

    // fish:5324-5325 — Only to be used if no case-sensitive suggestions are found.
    let mut icase_history_result: Option<AutosuggestionResult> = None;

    // `match_prev_cmd` (zsh-autosuggestions): restrict candidates to entries that
    // followed the previously executed command in history.
    let strategies = configured_strategies();
    let prev_cmd = if strategies.contains(&Strategy::MatchPrevCmd) {
        previous_command()
    } else {
        None
    };

    let working_directory = getsparam("PWD").unwrap_or_else(|| ".".to_owned());

    // fish:5351-5359 — walk matching history newest-first.
    let mut candidates = source(search_string, 64);
    // zsh-autosuggestions `match_prev_cmd` is a PREFERENCE, not a filter: it
    // picks the most recent match whose PRECEDING history item equals the
    // last executed command, falling back to the plain most-recent match
    // when no neighbor-match exists (the plugin's own code path).
    // Implementing it as a hard filter killed all suggestions for configs
    // like `ZSH_AUTOSUGGEST_STRATEGY=( match_prev_cmd )`.
    if let Some(prev) = &prev_cmd {
        let preferred: Option<String> = {
            let ring = crate::ported::hist::hist_ring.lock().unwrap();
            ring.windows(2)
                .find(|w| &w[1].node.nam == prev && w[0].node.nam.starts_with(search_string))
                .map(|w| w[0].node.nam.clone())
        };
        if let Some(preferred) = preferred {
            candidates.retain(|c| c != &preferred);
            candidates.insert(0, preferred);
        }
    }

    // Budget-degradation fallback: fish validates candidates with unbounded
    // background time; the synchronous pass caps validation work. If the
    // budget dies before anything validates, suggest the first (newest)
    // prefix match UNVALIDATED — exactly what zsh-autosuggestions does on
    // every keystroke, so the degraded mode is still plugin-parity.
    let mut unvalidated_fallback: Option<AutosuggestionResult> = None;

    // Single-line ghost rule: when the typed buffer is single-line, a
    // multiline history candidate is suggested only up to its first newline
    // (fish's completion suggestions do the same via line_at_cursor,
    // reader.rs:5494). POSTDISPLAY newlines render as real line breaks and
    // collide with multi-row prompts (p10k) — the ghost scrambled the
    // prompt block until the renderer learns multiline ghosts.
    let single_line_buffer = !search_string.contains('\n');

    for full_item in &candidates {
        let full: &str = if single_line_buffer {
            match full_item.split_once('\n') {
                Some((first, _)) => first,
                None => full_item.as_str(),
            }
        } else {
            full_item.as_str()
        };
        let full = &full.to_string();

        // fish:5362-5375 — case-sensitive prefix first, then one icase fallback.
        let (matches, icase) = if full.starts_with(search_string) {
            (true, false)
        } else if icase_history_result.is_none()
            && string_prefixes_string_maybe_case_insensitive(true, search_string, full)
        {
            (true, true)
        } else {
            (false, false)
        };
        if !matches || full.chars().count() <= line_len {
            continue;
        }

        let is_whole = full == full_item; // false when truncated at a newline
        let make_result = || {
            AutosuggestionResult::new(
                command_line.to_owned(),
                range.clone(),
                full.clone(),
                icase.then(|| search_string.chars().count()),
                is_whole,
            )
        };
        if !icase && unvalidated_fallback.is_none() {
            unvalidated_fallback = Some(make_result());
        }
        if ctx.check_cancel() {
            break;
        }

        // fish:5413-5419 — validate (cd targets must exist, command must resolve).
        if crate::syntax_highlight::autosuggest_validate_from_history(
            full,
            &[],
            &working_directory,
            ctx,
        ) {
            // fish:5420-5433
            let result = make_result();
            if icase {
                icase_history_result = Some(result);
            } else {
                return result;
            }
        }
    }

    // fish:5468-5472 — no case-sensitive result: fall back to icase history.
    if let Some(result) = icase_history_result {
        return result;
    }
    // Budget exhausted before any validation succeeded: plugin-parity fallback.
    if ctx.check_cancel() {
        if let Some(result) = unvalidated_fallback {
            return result;
        }
    }

    // fish:5443-5495 — completion-based suggestions: not yet wired (see module
    // header). cursor_pos is unused until then.
    let _ = cursor_pos;
    nothing
}

/// fish:reader.rs:5519-5531 — `can_autosuggest`.
pub fn can_autosuggest(state: &AutosuggestState, line: &str) -> bool {
    // We autosuggest if suppress_autosuggestion is not set, if we're not doing a
    // history search, and our command line contains a non-whitespace character.
    // zsh-autosuggestions config: a buffer longer than
    // ZSH_AUTOSUGGEST_BUFFER_MAX_SIZE disables suggestion compute.
    let max_size = getsparam("ZSH_AUTOSUGGEST_BUFFER_MAX_SIZE")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(usize::MAX);
    !state.suppress_autosuggestion
        && !state.history_search_active
        && line.chars().count() <= max_size
        && line
            .chars()
            .any(|c| !matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0B'))
}

/// fish:reader.rs:5534-5573 — `autosuggest_completed` (sync path: staleness can't
/// happen, but the prefix re-check is kept — the widget may have edited the line
/// between compute and store in future async use).
pub fn autosuggest_completed(
    state: &mut AutosuggestState,
    line: &str,
    result: AutosuggestionResult,
) {
    if result.command_line != line {
        // fish:5539-5542 — This autosuggestion is stale.
        return;
    }
    if !result.is_empty()
        && string_prefixes_string_maybe_case_insensitive(
            result.icase_matched_codepoints.is_some(),
            &result.search_string(),
            &result.text,
        )
    {
        // fish:5559-5572 — Autosuggestion is active and the search term has not
        // changed, so we're good to go.
        state.autosuggestion = result.autosuggestion;
    }
}

/// fish:reader.rs:5575-5610 — `update_autosuggestion`. Returns true when the stored
/// suggestion changed (the caller repaints POSTDISPLAY).
pub fn update_autosuggestion(
    line: &str,
    cursor: usize,
    source: &HistorySource<'_>,
    ctx: &OperationContext,
) -> bool {
    with_state(|state| {
        let before = state.autosuggestion.text.clone();

        // fish:5576-5581 — If we can't autosuggest, just clear it.
        if !can_autosuggest(state, line) {
            state.last_request_line.clear();
            state.autosuggestion.clear();
            return state.autosuggestion.text != before;
        }

        // fish:5583-5592 — still at a line with a valid suggestion: keep it.
        if is_at_line_with_autosuggestion(state, line, cursor) {
            return false;
        }

        // fish:5594-5597 — Do nothing if we've already kicked off this request.
        if line == state.last_request_line {
            // A stale suggestion that no longer prefixes must still be dropped.
            if !state.autosuggestion.is_empty() {
                state.autosuggestion.clear();
            }
            return state.autosuggestion.text != before;
        }
        state.last_request_line = line.to_owned();

        // fish:5600-5609 — Clear the autosuggestion and (synchronously) recompute.
        state.autosuggestion.clear();
        let result = compute_autosuggestion(line, cursor, source, ctx);
        autosuggest_completed(state, line, result);
        state.autosuggestion.text != before
    })
}

/// fish:reader.rs:5620-5633 — `is_at_autosuggestion` (cursor exactly at the end of
/// the searched range; CursorEndMode::Exclusive — the vi-cmdmode inclusive case is
/// handled by the wiring passing an adjusted cursor).
pub fn is_at_autosuggestion(state: &AutosuggestState, cursor: usize) -> bool {
    if state.autosuggestion.is_empty() {
        return false;
    }
    cursor == state.autosuggestion.search_string_range.end
}

/// fish:reader.rs:5635-5650 — `is_at_line_with_autosuggestion`.
pub fn is_at_line_with_autosuggestion(state: &AutosuggestState, line: &str, cursor: usize) -> bool {
    if state.autosuggestion.is_empty() {
        return false;
    }
    let range = &state.autosuggestion.search_string_range;
    // Suggestion ranges are whole-line here; the line must still BE the search
    // string (fish asserts the prefix relation at reader.rs:5586-5590).
    let line_len = line.chars().count();
    (range.start == 0 && range.end == line_len && cursor <= range.end)
        && string_prefixes_string_maybe_case_insensitive(
            state.autosuggestion.icase_matched_codepoints.is_some(),
            line,
            &state.autosuggestion.text,
        )
}

/// fish:reader.rs:5652-5760 — Accept any autosuggestion. Returns the (range,
/// replacement) edit to apply to the command line: `range` is the char-index span
/// to replace, `replacement` the text to splice in. None = nothing to accept.
pub fn accept_autosuggestion(
    state: &mut AutosuggestState,
    amount: AutosuggestionPortion,
) -> Option<(Range<usize>, String)> {
    let autosuggestion = &state.autosuggestion;
    if autosuggestion.is_empty() {
        return None;
    }
    let autosuggestion_text: Vec<char> = autosuggestion.text.chars().collect();
    let search_string_range = autosuggestion.search_string_range.clone();

    // fish:5664-5719 — Accept the autosuggestion.
    let (range, replacement): (Range<usize>, String) = match amount {
        AutosuggestionPortion::Count(count) => {
            if count == usize::MAX {
                // full accept: replace the whole searched range with the suggestion
                (search_string_range, autosuggestion_text.iter().collect())
            } else {
                let pos = search_string_range.end;
                let available = autosuggestion_text.len() - search_string_range.len();
                let count = count.min(available);
                if count == 0 {
                    return None;
                }
                let start = autosuggestion_text.len() - available;
                (
                    pos..pos,
                    autosuggestion_text[start..start + count].iter().collect(),
                )
            }
        }
        AutosuggestionPortion::Line => {
            // fish:5682-5695
            let suggested = &autosuggestion_text[search_string_range.len()..];
            let line_end = suggested
                .iter()
                .position(|&c| c == '\n')
                .unwrap_or(suggested.len());
            if line_end == 0 {
                return None;
            }
            (
                search_string_range.end..search_string_range.end,
                suggested[..line_end].iter().collect(),
            )
        }
        AutosuggestionPortion::Word => {
            // fish:5696-5719 — fish consumes MoveWordStateMachine chars; the zsh
            // spelling is one forward-word over $WORDCHARS (word chars are
            // alphanumerics plus $WORDCHARS members).
            let wordchars =
                getsparam("WORDCHARS").unwrap_or_else(|| "*?_-.[]~=/&;!#$%^(){}<>".to_owned());
            let is_word = |c: char| c.is_alphanumeric() || wordchars.contains(c);
            let have = search_string_range.len();
            let mut want = have;
            // skip any leading non-word chars, then consume the word
            while want < autosuggestion_text.len() && !is_word(autosuggestion_text[want]) {
                want += 1;
            }
            while want < autosuggestion_text.len() && is_word(autosuggestion_text[want]) {
                want += 1;
            }
            if want == have {
                return None;
            }
            (
                search_string_range.end..search_string_range.end,
                autosuggestion_text[have..want].iter().collect(),
            )
        }
    };

    // fish:5722-5759 — full accept clears the suggestion; partial keeps the rest
    // (the next update re-anchors it because the line now prefixes the text).
    if range == (0..0) && replacement.is_empty() {
        return None;
    }
    if matches!(amount, AutosuggestionPortion::Count(usize::MAX)) {
        state.autosuggestion.clear();
        state.last_request_line.clear();
    }
    Some((range, replacement))
}

/// fish deletion paths set `suppress_autosuggestion` and save the suggestion so
/// retyping restores it (reader.rs:679-680 + delete_char). The wiring calls this
/// after any deleting widget.
pub fn on_delete(state: &mut AutosuggestState) {
    if !state.autosuggestion.is_empty() {
        state.saved_autosuggestion = Some(state.autosuggestion.clone());
    }
    state.suppress_autosuggestion = true;
    state.autosuggestion.clear();
    state.last_request_line.clear();
}

/// Inserting text lifts the suppression (fish clears suppress_autosuggestion on
/// insert); a saved suggestion that still prefixes is restored without a search.
pub fn on_insert(state: &mut AutosuggestState, line: &str) {
    state.suppress_autosuggestion = false;
    if let Some(saved) = state.saved_autosuggestion.take() {
        let line_len = line.chars().count();
        if string_prefixes_string_maybe_case_insensitive(
            saved.icase_matched_codepoints.is_some(),
            line,
            &saved.text,
        ) && line_len < saved.text.chars().count()
        {
            let mut restored = saved;
            restored.search_string_range = 0..line_len;
            state.autosuggestion = restored;
            state.last_request_line = line.to_owned();
        }
    }
}

/// Line finished (accept-line/send-break): drop all suggestion state.
pub fn on_line_finish(state: &mut AutosuggestState) {
    state.autosuggestion.clear();
    state.saved_autosuggestion = None;
    state.suppress_autosuggestion = false;
    state.last_request_line.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_util::global_state_lock()
    }

    fn src(items: &[&str]) -> impl Fn(&str, usize) -> Vec<String> {
        let owned: Vec<String> = items.iter().map(|s| s.to_string()).collect();
        move |prefix: &str, limit: usize| {
            owned
                .iter()
                .filter(|c| c.to_lowercase().starts_with(&prefix.to_lowercase()))
                .take(limit)
                .cloned()
                .collect()
        }
    }

    // fish:5362-5375 — case-sensitive match wins over icase.
    #[test]
    fn compute_prefers_case_sensitive() {
        let _g = lock();
        let ctx = OperationContext::empty();
        let source = src(&["Echo one", "echo two"]);
        let r = compute_autosuggestion("echo", 4, &source, &ctx);
        assert_eq!(r.text, "echo two");
        assert!(r.icase_matched_codepoints.is_none());
        assert!(r.is_whole_item_from_history);
    }

    #[test]
    fn compute_falls_back_to_icase() {
        let _g = lock();
        let ctx = OperationContext::empty();
        let source = src(&["Echo one"]);
        let r = compute_autosuggestion("echo", 4, &source, &ctx);
        assert_eq!(r.text, "Echo one");
        assert_eq!(r.icase_matched_codepoints, Some(4));
    }

    #[test]
    fn compute_rejects_invalid_command() {
        let _g = lock();
        let ctx = OperationContext::empty();
        // fish:5413-5419 — a history entry whose command no longer resolves is not
        // suggested.
        let source = src(&["nonexistent_zshrs_cmd_xyz --flag"]);
        let r = compute_autosuggestion("nonex", 5, &source, &ctx);
        assert!(
            r.is_empty(),
            "invalid command must not be suggested: {:?}",
            r.text
        );
    }

    #[test]
    fn compute_empty_line_suggests_nothing() {
        let _g = lock();
        let ctx = OperationContext::empty();
        let source = src(&["echo hi"]);
        let r = compute_autosuggestion("", 0, &source, &ctx);
        assert!(r.is_empty()); // fish:5333-5335 / 5443-5446
    }

    // fish:5654-5719 — accept portions.
    fn state_with_suggestion(line: &str, text: &str) -> AutosuggestState {
        AutosuggestState {
            autosuggestion: Autosuggestion {
                text: text.to_owned(),
                search_string_range: 0..line.chars().count(),
                icase_matched_codepoints: None,
                is_whole_item_from_history: true,
            },
            ..Default::default()
        }
    }

    #[test]
    fn accept_full_replaces_line() {
        let mut st = state_with_suggestion("git s", "git status --short");
        let (range, repl) =
            accept_autosuggestion(&mut st, AutosuggestionPortion::Count(usize::MAX)).unwrap();
        assert_eq!(range, 0..5);
        assert_eq!(repl, "git status --short");
        assert!(
            st.autosuggestion.is_empty(),
            "full accept clears suggestion"
        );
    }

    #[test]
    fn accept_count_appends_chars() {
        let mut st = state_with_suggestion("git s", "git status --short");
        let (range, repl) =
            accept_autosuggestion(&mut st, AutosuggestionPortion::Count(3)).unwrap();
        assert_eq!(range, 5..5);
        assert_eq!(repl, "tat");
    }

    #[test]
    fn accept_word_stops_at_boundary() {
        let mut st = state_with_suggestion("git s", "git status --short");
        let (range, repl) = accept_autosuggestion(&mut st, AutosuggestionPortion::Word).unwrap();
        assert_eq!(range, 5..5);
        // consumes "tatus" then stops before the space→"--short" run begins…
        assert!(repl.starts_with("tatus"), "got {repl:?}");
        assert!(
            !repl.contains("short"),
            "must stop at word boundary: {repl:?}"
        );
    }

    #[test]
    fn accept_on_empty_suggestion_is_none() {
        let mut st = AutosuggestState::default();
        assert!(accept_autosuggestion(&mut st, AutosuggestionPortion::Count(usize::MAX)).is_none());
    }

    // fish:679-680 — backspace suppression + retype restore.
    #[test]
    fn delete_suppresses_and_insert_restores() {
        let _g = lock();
        let mut st = state_with_suggestion("git s", "git status");
        on_delete(&mut st);
        assert!(st.suppress_autosuggestion);
        assert!(st.autosuggestion.is_empty());
        assert!(!can_autosuggest(&st, "git "));

        // Retyping a prefix of the saved suggestion restores it.
        on_insert(&mut st, "git st");
        assert!(!st.suppress_autosuggestion);
        assert_eq!(st.autosuggestion.text, "git status");
        assert_eq!(st.autosuggestion.search_string_range, 0..6);
    }

    #[test]
    fn insert_discards_saved_when_no_longer_prefix() {
        let _g = lock();
        let mut st = state_with_suggestion("git s", "git status");
        on_delete(&mut st);
        on_insert(&mut st, "ls -");
        assert!(st.autosuggestion.is_empty());
    }

    // fish:5534-5542 — staleness check.
    #[test]
    fn stale_result_is_dropped() {
        let mut st = AutosuggestState::default();
        let result = AutosuggestionResult::new(
            "old line".to_owned(),
            0..8,
            "old line more".to_owned(),
            None,
            true,
        );
        autosuggest_completed(&mut st, "new line", result);
        assert!(st.autosuggestion.is_empty());
    }

    #[test]
    fn update_clears_when_line_no_longer_matches() {
        let _g = lock();
        let ctx = OperationContext::empty();
        let source = src(&["echo hello"]);
        // Seed state via a real update.
        let changed = update_autosuggestion("echo h", 6, &source, &ctx);
        assert!(changed);
        with_state(|st| {
            assert_eq!(st.autosuggestion.text, "echo hello");
        });
        // A line that matches nothing clears the suggestion.
        let changed2 = update_autosuggestion("zzz_nothing", 11, &source, &ctx);
        assert!(changed2);
        with_state(|st| {
            assert!(st.autosuggestion.is_empty());
            st.autosuggestion.clear();
            st.last_request_line.clear();
            st.saved_autosuggestion = None;
        });
    }

    #[test]
    fn suggestion_suffix() {
        let s = Autosuggestion {
            text: "git status".to_owned(),
            search_string_range: 0..5,
            icase_matched_codepoints: None,
            is_whole_item_from_history: true,
        };
        assert_eq!(s.suffix(), "tatus");
    }
}
