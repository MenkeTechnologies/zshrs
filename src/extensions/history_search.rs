//! Port of fish-shell's `reader/history_search.rs` (vendor/fish/reader/history_search.rs)
//! — the up-arrow / prefix / token / substring history search state machine.
//!
//! fish:1 — Encapsulation of the reader's history search functionality.
//!
//! zsh-users/zsh-history-substring-search is a script-level recreation of this fish
//! machinery (its README says as much); `SearchMode::Line` with a mid-line needle is
//! exactly its substring behavior, and `SearchMode::Prefix` is fish's up-arrow
//! (`pw`⭡ walks only `pw*` entries) — what up-line-or-beginning-search recreates.
//!
//! zshrs substrate swaps (each cited at its site):
//!   * fish `History`/`HistorySearch` (history/history.rs) → a newest-first snapshot
//!     over `HistoryEngine` (SQLite, extensions/history.rs) + the in-memory
//!     `hist_ring` (ported/hist.rs:4841, newest at index 0)
//!   * fish `Tokenizer` (Token modes)  → `lex_line_tokens` (extensions/syntax_highlight.rs)
//!   * `ifind`                          → char-wise lowercase scan
//!
//! Widget wiring (task: zlecore hook) binds this to up-line-or-search /
//! down-line-or-search / history-beginning-search-{backward,forward} /
//! history-substring-search-{up,down}, honoring the
//! `HISTORY_SUBSTRING_SEARCH_HIGHLIGHT_{FOUND,NOT_FOUND}` styles via
//! `search_range_if_active`.

#![allow(non_snake_case)]

use std::collections::HashSet;
use std::ops::Range;
use std::sync::Mutex;

/// fish history/history.rs `SearchDirection`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchDirection {
    Forward,
    Backward,
}

/// fish history/history.rs `SearchType` (the two variants this file uses).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchType {
    Prefix,
    Contains,
}

/// fish:12-20 — Make the search case-insensitive unless we have an uppercase
/// character.
pub fn smartcase_ignores_case(query: &str) -> bool {
    query == query.to_lowercase()
}

/// !!! WARNING: RUST-ONLY ADAPTER — fish's `HistorySearch` is an incremental cursor
/// over its history file; zshrs snapshots the matching entries newest-first at
/// search start (SQLite indexed probe + in-memory ring union) and walks the
/// snapshot. `prepare_to_search_after_deletion` is a no-op on a snapshot. !!!
pub struct HistorySearch {
    original_term: String,
    search_type: SearchType,
    ignore_case: bool,
    /// Matching commands, newest first.
    items: Vec<String>,
    /// Current position; usize::MAX = before the first match.
    idx: usize,
    current: String,
}

impl HistorySearch {
    /// fish history.rs `HistorySearch::new_with` — snapshot constructor reading the
    /// live history stores.
    pub fn new_with(term: String, search_type: SearchType, ignore_case: bool) -> Self {
        let items = Self::snapshot(&term, search_type, ignore_case);
        Self {
            original_term: term,
            search_type,
            ignore_case,
            items,
            idx: usize::MAX,
            current: String::new(),
        }
    }

    /// Test constructor over a fixed item list (newest first).
    pub fn from_items(
        term: String,
        search_type: SearchType,
        ignore_case: bool,
        items: Vec<String>,
    ) -> Self {
        let items = items
            .into_iter()
            .filter(|c| entry_matches(c, &term, search_type, ignore_case))
            .collect();
        Self {
            original_term: term,
            search_type,
            ignore_case,
            items,
            idx: usize::MAX,
            current: String::new(),
        }
    }

    fn snapshot(term: &str, search_type: SearchType, ignore_case: bool) -> Vec<String> {
        // The search cursor walks strictly NEWEST-FIRST (the plugin/fish
        // contract): in-memory ring first (this session, newest at 0 —
        // hist.rs:2617-2621), then SQLite matches re-sorted by timestamp —
        // the FTS query orders frequency-first, which surfaced years-old
        // high-frequency entries before the line executed seconds ago.
        let mut out: Vec<String> = Vec::new();
        {
            let ring = crate::ported::hist::hist_ring.lock().unwrap();
            for e in ring.iter() {
                let cmd = &e.node.nam;
                if !cmd.is_empty() && entry_matches(cmd, term, search_type, ignore_case) {
                    out.push(cmd.clone());
                }
            }
        }

        // SQLite probe: prefix uses the indexed GLOB (the LIKE-based
        // search_prefix full-scans at 500k+ rows); substring uses the FTS
        // trigram mirror (history.rs:315).
        let mut db: Vec<(i64, String)> = crate::history::with_session_engine(|eng| {
            let res = match search_type {
                SearchType::Prefix => eng.search_prefix_cs(term, 512),
                SearchType::Contains => eng.search(term, 512),
            };
            res.map(|v| {
                v.into_iter()
                    .map(|e| (e.timestamp, e.command))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        })
        .unwrap_or_default();
        db.sort_by(|a, b| b.0.cmp(&a.0)); // newest first
        out.extend(db.into_iter().map(|(_, c)| c));

        // Exact dedup happens in ReaderHistorySearch::skips.
        out.retain(|c| entry_matches(c, term, search_type, ignore_case));
        out
    }

    /// fish history.rs `go_to_next_match`.
    pub fn go_to_next_match(&mut self, dir: SearchDirection) -> bool {
        match dir {
            SearchDirection::Backward => {
                let next = if self.idx == usize::MAX {
                    0
                } else {
                    self.idx + 1
                };
                if next < self.items.len() {
                    self.idx = next;
                    self.current = self.items[next].clone();
                    true
                } else {
                    false
                }
            }
            SearchDirection::Forward => {
                if self.idx != usize::MAX && self.idx > 0 {
                    self.idx -= 1;
                    self.current = self.items[self.idx].clone();
                    true
                } else {
                    false
                }
            }
        }
    }

    /// fish history.rs `current_string`.
    pub fn current_string(&self) -> &str {
        &self.current
    }

    /// fish history.rs `original_term`.
    pub fn original_term(&self) -> &str {
        &self.original_term
    }

    /// fish history.rs `ignores_case`.
    pub fn ignores_case(&self) -> bool {
        self.ignore_case
    }

    /// fish history.rs `prepare_to_search_after_deletion` — snapshot no-op.
    pub fn prepare_to_search_after_deletion(&mut self) {}
}

fn entry_matches(cmd: &str, term: &str, search_type: SearchType, ignore_case: bool) -> bool {
    match (search_type, ignore_case) {
        (SearchType::Prefix, false) => cmd.starts_with(term),
        (SearchType::Prefix, true) => {
            let mut cc = cmd.chars();
            term.chars().all(|tc| match cc.next() {
                Some(c) => c.to_lowercase().eq(tc.to_lowercase()),
                None => false,
            })
        }
        (SearchType::Contains, false) => cmd.contains(term),
        (SearchType::Contains, true) => ifind(cmd, term).is_some(),
    }
}

/// fish:7 `ifind` — case-insensitive substring find, returning the char offset.
fn ifind(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = haystack.chars().flat_map(|c| c.to_lowercase()).collect();
    let ndl: Vec<char> = needle.chars().flat_map(|c| c.to_lowercase()).collect();
    hay.windows(ndl.len()).position(|w| w == ndl.as_slice())
}

/// Case-respecting char-offset find.
fn find_char_offset(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .find(needle)
        .map(|byte| haystack[..byte].chars().count())
}

/// fish:22-33 — `SearchMatch`.
struct SearchMatch {
    /// fish:23-24 — The text of the match.
    pub text: String,
    /// fish:25-26 — The offset (chars) of the current search string in this match.
    offset: usize,
}

impl SearchMatch {
    fn new(text: String, offset: usize) -> Self {
        Self { text, offset }
    }
}

/// fish:34-48 — `SearchMode`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchMode {
    #[default]
    /// no search
    Inactive,
    /// searching by line (substring — the zsh-history-substring-search behavior)
    Line,
    /// searching by prefix (the fish up-arrow / up-line-or-beginning-search behavior)
    Prefix,
    /// searching by token
    Token,
    /// search by the last token of the command
    LastToken,
}

/// fish:50-70 — Encapsulation of the reader's history search functionality.
#[derive(Default)]
pub struct ReaderHistorySearch {
    /// fish:53-54 — The type of search performed.
    mode: SearchMode,

    /// fish:56-57 — Our history search itself.
    search: Option<HistorySearch>,

    /// fish:59-60 — The ordered list of matches. This may grow long.
    matches: Vec<SearchMatch>,

    /// fish:62-63 — A set of new items to skip, corresponding to matches_ and
    /// anything added in skip().
    skips: HashSet<String>,

    /// fish:65-66 — Index into our matches list.
    match_index: usize,

    /// fish:68-69 — The offset of the current token in the command line. Only
    /// non-zero for a token search.
    token_offset: usize,
}

/// Module-global instance (fish keeps this on `Reader`; one editor per process).
pub static HISTORY_SEARCH: Mutex<Option<ReaderHistorySearch>> = Mutex::new(None);

/// Run f over the global search state.
pub fn with_history_search<R>(f: impl FnOnce(&mut ReaderHistorySearch) -> R) -> R {
    let mut guard = HISTORY_SEARCH.lock().unwrap();
    f(guard.get_or_insert_with(Default::default))
}

impl ReaderHistorySearch {
    /// fish:73-76 — `active`.
    pub fn active(&self) -> bool {
        self.mode != SearchMode::Inactive
    }
    /// fish:77-80 — `by_token`.
    pub fn by_token(&self) -> bool {
        matches!(self.mode, SearchMode::Token | SearchMode::LastToken)
    }
    /// fish:81-86 — `by_line`. Included for completeness.
    pub fn by_line(&self) -> bool {
        self.mode == SearchMode::Line
    }
    /// fish:87-90 — `by_prefix`.
    pub fn by_prefix(&self) -> bool {
        self.mode == SearchMode::Prefix
    }
    /// fish:91-94 — `mode`.
    pub fn mode(&self) -> SearchMode {
        self.mode
    }

    /// fish:96-102 — Move the history search in the given direction `dir`.
    pub fn move_in_direction(&mut self, dir: SearchDirection) -> bool {
        match dir {
            SearchDirection::Forward => self.move_forwards(),
            SearchDirection::Backward => self.move_backwards(),
        }
    }

    /// fish:104-110 — Go to the oldest match (last match) of the search.
    pub fn go_to_oldest(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.match_index = self.matches.len() - 1;
    }

    /// fish:112-115 — Go to the youngest match (original search string) of the
    /// search.
    pub fn go_to_present(&mut self) {
        self.match_index = 0;
    }

    /// fish:117-120 — Return the current search result.
    pub fn current_result(&self) -> &str {
        &self.matches[self.match_index].text
    }

    /// fish:122-125 — Return the string we are searching for.
    pub fn search_string(&self) -> &str {
        self.search().original_term()
    }

    /// fish:127-131 — Return the range of the current match in the command line
    /// (char offsets).
    pub fn search_result_range(&self) -> Range<usize> {
        assert!(self.active());
        self.token_offset..self.token_offset + self.matches[self.match_index].text.chars().count()
    }

    /// fish:133-142 — Return the range of the original search string in the new
    /// command line: (start, len) in chars. This is what the wiring paints with
    /// `HISTORY_SUBSTRING_SEARCH_HIGHLIGHT_FOUND`.
    pub fn search_range_if_active(&self) -> Option<(usize, usize)> {
        if !self.active() || self.is_at_present() {
            return None;
        }
        Some((
            self.token_offset + self.matches[self.match_index].offset,
            self.search_string().chars().count(),
        ))
    }

    /// fish:144-147 — Return whether we are at the youngest match (original search
    /// string) in our search.
    pub fn is_at_present(&self) -> bool {
        self.match_index == 0
    }

    // fish:149-154 — Add an item to skip. Return true if it was added, false if
    // already present.
    pub fn add_skip(&mut self, s: String) -> bool {
        self.skips.insert(s)
    }

    /// fish:155-162 — `handle_deletion`.
    pub fn handle_deletion(&mut self) {
        assert!(!self.is_at_present());
        self.matches.remove(self.match_index);
        self.match_index -= 1;
        self.search_mut().prepare_to_search_after_deletion();
        self.move_backwards();
    }

    /// fish:164-195 — Reset, beginning a new line or token mode search.
    pub fn reset_to_mode(&mut self, text: String, mode: SearchMode, token_offset: usize) {
        assert_ne!(
            mode,
            SearchMode::Inactive,
            "mode cannot be inactive in this setter"
        );
        self.skips = HashSet::from([text.clone()]);
        self.matches = vec![SearchMatch::new(text.clone(), 0)];
        self.match_index = 0;
        self.mode = mode;
        self.token_offset = token_offset;
        // fish:182-183 — We can skip dedup in HistorySearch because we do it
        // ourselves in skips_.
        let ignore_case = smartcase_ignores_case(&text);
        self.search = Some(HistorySearch::new_with(
            text,
            if self.by_prefix() {
                SearchType::Prefix
            } else {
                SearchType::Contains
            },
            ignore_case,
        ));
    }

    /// Test variant of reset_to_mode over a fixed item list.
    pub fn reset_to_mode_with_items(
        &mut self,
        text: String,
        mode: SearchMode,
        token_offset: usize,
        items: Vec<String>,
    ) {
        self.reset_to_mode(text.clone(), mode, token_offset);
        let ignore_case = smartcase_ignores_case(&text);
        self.search = Some(HistorySearch::from_items(
            text,
            if self.by_prefix() {
                SearchType::Prefix
            } else {
                SearchType::Contains
            },
            ignore_case,
            items,
        ));
    }

    /// fish:197-205 — Reset to inactive search.
    pub fn reset(&mut self) {
        self.matches.clear();
        self.skips.clear();
        self.match_index = 0;
        self.mode = SearchMode::Inactive;
        self.token_offset = 0;
        self.search = None;
    }

    /// fish:207-212 — Adds the given match if we haven't seen it before.
    fn add_if_new(&mut self, search_match: SearchMatch) {
        if self.add_skip(search_match.text.clone()) {
            self.matches.push(search_match);
        }
    }

    /// fish:214-254 — Attempt to append matches from the current history item.
    /// Return true if something was appended.
    fn append_matches_from_search(&mut self) -> bool {
        let icase = self.search().ignores_case();
        let find = |haystack: &str, needle: &str| -> Option<usize> {
            if icase {
                ifind(haystack, needle)
            } else {
                find_char_offset(haystack, needle)
            }
        };
        let before = self.matches.len();
        let text = self.search().current_string().to_owned();
        let needle = self.search_string().to_owned();
        if matches!(self.mode, SearchMode::Line | SearchMode::Prefix) {
            // fish:228-230 — the search itself guaranteed a hit.
            let offset = find(&text, &needle).unwrap_or(0);
            self.add_if_new(SearchMatch::new(text, offset));
        } else if matches!(self.mode, SearchMode::Token | SearchMode::LastToken) {
            // fish:231-243 — fish Tokenizer(TOK_ACCEPT_UNFINISHED); zsh spelling:
            // the same tolerant lexer the highlighter uses.
            let toks = crate::syntax_highlight::lex_line_tokens(&text);

            let mut local_tokens = vec![];
            for token in &toks {
                if token.tok != crate::ported::zsh_h::STRING_LEX {
                    continue;
                }
                let tok_text = token.clean_text();
                if let Some(offset) = find(&tok_text, &needle) {
                    local_tokens.push(SearchMatch::new(tok_text, offset));
                }
            }

            // fish:245-251 — Make sure tokens are added in reverse order. See
            // fish#5150.
            for tok in local_tokens.into_iter().rev() {
                self.add_if_new(tok);
                if self.mode == SearchMode::LastToken {
                    break;
                }
            }
        }
        self.matches.len() > before
    }

    /// fish:256-264 — `move_forwards`.
    fn move_forwards(&mut self) -> bool {
        // Try to move within our previously discovered matches.
        if self.match_index > 0 {
            self.match_index -= 1;
            true
        } else {
            false
        }
    }

    /// fish:266-290 — `move_backwards`.
    fn move_backwards(&mut self) -> bool {
        // Try to move backwards within our previously discovered matches.
        if self.match_index + 1 < self.matches.len() {
            self.match_index += 1;
            return true;
        }

        // Add more items from our search.
        while self
            .search_mut()
            .go_to_next_match(SearchDirection::Backward)
        {
            if self.append_matches_from_search() {
                self.match_index += 1;
                assert!(
                    self.match_index < self.matches.len(),
                    "Should have found more matches"
                );
                return true;
            }
        }

        // fish:288-289 — Here we failed to go backwards past the last history item.
        false
    }

    fn search(&self) -> &HistorySearch {
        self.search.as_ref().unwrap()
    }

    fn search_mut(&mut self) -> &mut HistorySearch {
        self.search.as_mut().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_with(text: &str, mode: SearchMode, items: &[&str]) -> ReaderHistorySearch {
        let mut s = ReaderHistorySearch::default();
        s.reset_to_mode_with_items(
            text.to_owned(),
            mode,
            0,
            items.iter().map(|s| s.to_string()).collect(),
        );
        s
    }

    /// fish up-arrow semantics: prefix search walks only matching entries,
    /// newest first ("pw"⭡ → pwd-things only).
    #[test]
    fn prefix_search_walks_matches_newest_first() {
        let _g = crate::test_util::global_state_lock();
        let mut s = search_with(
            "pw",
            SearchMode::Prefix,
            &["pwgen 16", "ls -la", "pwd", "echo done"],
        );
        assert!(s.move_in_direction(SearchDirection::Backward));
        assert_eq!(s.current_result(), "pwgen 16");
        assert!(s.move_in_direction(SearchDirection::Backward));
        assert_eq!(s.current_result(), "pwd");
        // No more matches.
        assert!(!s.move_in_direction(SearchDirection::Backward));
        // Forward returns toward present.
        assert!(s.move_in_direction(SearchDirection::Forward));
        assert_eq!(s.current_result(), "pwgen 16");
        assert!(s.move_in_direction(SearchDirection::Forward));
        // fish:112-115 — youngest match is the original search string.
        assert!(s.is_at_present());
        assert_eq!(s.current_result(), "pw");
    }

    /// zsh-history-substring-search semantics: Line mode matches mid-line.
    #[test]
    fn line_mode_is_substring_search() {
        let _g = crate::test_util::global_state_lock();
        let mut s = search_with(
            "grep",
            SearchMode::Line,
            &["ls | grep foo", "echo x", "grep -r bar ."],
        );
        assert!(s.move_in_direction(SearchDirection::Backward));
        assert_eq!(s.current_result(), "ls | grep foo");
        // The found-range highlight points at the needle inside the match.
        let (start, len) = s.search_range_if_active().unwrap();
        assert_eq!(len, 4);
        assert_eq!(start, 5); // "ls | " = 5 chars
        assert!(s.move_in_direction(SearchDirection::Backward));
        assert_eq!(s.current_result(), "grep -r bar .");
    }

    /// fish:12-20 — smartcase: lowercase query matches case-insensitively;
    /// mixed-case query is exact.
    #[test]
    fn smartcase_behavior() {
        let _g = crate::test_util::global_state_lock();
        let mut s = search_with("make", SearchMode::Prefix, &["Makefile-gen", "make test"]);
        assert!(s.move_in_direction(SearchDirection::Backward));
        assert_eq!(s.current_result(), "Makefile-gen"); // icase hit

        let mut s2 = search_with("Make", SearchMode::Prefix, &["make test", "Makefile-gen"]);
        assert!(s2.move_in_direction(SearchDirection::Backward));
        assert_eq!(s2.current_result(), "Makefile-gen"); // exact-case only
        assert!(!s2.move_in_direction(SearchDirection::Backward));
    }

    /// fish:62-63/207-212 — duplicate history entries surface once
    /// (HISTORY_SUBSTRING_SEARCH_ENSURE_UNIQUE behavior, always on like fish).
    #[test]
    fn duplicates_are_skipped() {
        let _g = crate::test_util::global_state_lock();
        let mut s = search_with(
            "git",
            SearchMode::Prefix,
            &["git status", "git push", "git status"],
        );
        assert!(s.move_in_direction(SearchDirection::Backward));
        assert_eq!(s.current_result(), "git status");
        assert!(s.move_in_direction(SearchDirection::Backward));
        assert_eq!(s.current_result(), "git push");
        assert!(
            !s.move_in_direction(SearchDirection::Backward),
            "dup must be skipped"
        );
    }

    /// fish:231-251 — Token mode surfaces individual words, reverse order per item.
    #[test]
    fn token_mode_matches_words() {
        let _g = crate::test_util::global_state_lock();
        let mut s = search_with(
            "conf",
            SearchMode::Token,
            &["vim ~/.config/foo.conf other.conf"],
        );
        assert!(s.move_in_direction(SearchDirection::Backward));
        // Reverse order: last matching token of the item comes first (fish#5150).
        assert_eq!(s.current_result(), "other.conf");
        assert!(s.move_in_direction(SearchDirection::Backward));
        assert_eq!(s.current_result(), "~/.config/foo.conf");
    }

    /// fish:104-115 — oldest/present jumps.
    #[test]
    fn oldest_and_present_navigation() {
        let _g = crate::test_util::global_state_lock();
        let mut s = search_with("e", SearchMode::Prefix, &["echo 1", "echo 2", "echo 3"]);
        while s.move_in_direction(SearchDirection::Backward) {}
        s.go_to_oldest();
        assert_eq!(s.current_result(), "echo 3");
        s.go_to_present();
        assert!(s.is_at_present());
        assert_eq!(s.current_result(), "e");
    }

    /// fish:155-162 — deleting the current match steps back into the sequence.
    #[test]
    fn handle_deletion_steps_back() {
        let _g = crate::test_util::global_state_lock();
        let mut s = search_with("g", SearchMode::Prefix, &["git a", "git b", "git c"]);
        assert!(s.move_in_direction(SearchDirection::Backward)); // git a
        assert!(s.move_in_direction(SearchDirection::Backward)); // git b
        s.handle_deletion();
        assert_eq!(s.current_result(), "git c");
    }
}
