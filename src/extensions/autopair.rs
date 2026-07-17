//! Port of hlissner/zsh-autopair (`autopair.zsh`, 227 lines) — auto-insert,
//! skip-over, and pair-delete for brackets/quotes/spaces in ZLE.
//!
//! The zsh plugin source is the spec; citations are `// ap:NNN` line refs.
//! The predicates are ported as PURE functions over (lbuffer, rbuffer, key)
//! so they unit-test without editor state; `decide()` returns an Action the
//! zle_fx wiring applies. Where the plugin rebinds keys to wrapper widgets
//! (ap:200-226 autopair-init), zshrs intercepts the equivalent widgets in
//! `zle_fx::on_pre_widget` — same dispatch points, no bindkey mutation.
//!
//! Config compat (all read live): `AUTOPAIR_PAIRS`, `AUTOPAIR_LBOUNDS`,
//! `AUTOPAIR_RBOUNDS` (assoc params), `AUTOPAIR_BETWEEN_WHITESPACE` (scalar).
//! When the script plugin itself is loaded (its `autopair-insert` widget is
//! registered), the native engine yields.

#![allow(non_snake_case)]

use crate::ported::params::{gethparam, getsparam};
use std::collections::HashMap;
use std::sync::Mutex;

/// ap:9-10 — the default pair table.
/// `typeset -gA AUTOPAIR_PAIRS; AUTOPAIR_PAIRS=('`' '`' "'" "'" '"' '"' '{' '}' '[' ']' '(' ')' ' ' ' ')`
const DEFAULT_PAIRS: &[(char, char)] = &[
    ('`', '`'),
    ('\'', '\''),
    ('"', '"'),
    ('{', '}'),
    ('[', ']'),
    ('(', ')'),
    (' ', ' '),
];

/// ap:12-19 — default left-boundary regexps per group. The zsh originals are
/// POSIX EREs (`[]})…]`, `[^{([]`, `[.:/\!]`); these are the same classes
/// respelled for the Rust regex crate (leading/inner brackets escaped).
fn default_lbound(group: &str) -> Option<&'static str> {
    match group {
        "all" => Some(r"[.:/!]"),
        "quotes" => Some(r"[\]})a-zA-Z0-9]"),
        "spaces" => Some(r"[^{(\[]"),
        "braces" => Some(""),
        "`" => Some("`"),
        "\"" => Some("\""),
        "'" => Some("'"),
        _ => None,
    }
}

/// ap:21-25 — default right-boundary regexps per group (respelled, see above).
fn default_rbound(group: &str) -> Option<&'static str> {
    match group {
        "all" => Some(r"[\[{(<,.:?/%$!a-zA-Z0-9]"),
        "quotes" => Some("[a-zA-Z0-9]"),
        "spaces" => Some(r"[^\]})]"),
        "braces" => Some(""),
        _ => None,
    }
}

/// User overrides arrive as zsh/POSIX EREs, where `]` right after `[`/`[^`
/// and `[` inside a class are literals. The Rust regex crate rejects both;
/// escape them so plugin-authored patterns keep working.
fn zsh_ere_to_rust(pat: &str) -> String {
    let mut out = String::with_capacity(pat.len() + 4);
    let mut chars = pat.chars().peekable();
    let mut in_class = false;
    let mut class_start = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                }
                class_start = false;
            }
            '[' if !in_class => {
                in_class = true;
                out.push('[');
                if chars.peek() == Some(&'^') {
                    out.push(chars.next().unwrap());
                }
                class_start = true;
            }
            '[' if in_class => {
                out.push_str(r"\[");
                class_start = false;
            }
            ']' if in_class && class_start => {
                out.push_str(r"\]"); // POSIX: leading ] is literal
                class_start = false;
            }
            ']' if in_class => {
                in_class = false;
                out.push(']');
            }
            _ => {
                out.push(c);
                class_start = false;
            }
        }
    }
    out
}

/// Live view of the plugin's config params.
pub struct AutopairConfig {
    pub pairs: Vec<(char, char)>,
    pub between_whitespace: bool,
    lbounds: HashMap<String, String>,
    rbounds: HashMap<String, String>,
}

impl Default for AutopairConfig {
    fn default() -> Self {
        Self {
            pairs: DEFAULT_PAIRS.to_vec(),
            between_whitespace: false,
            lbounds: HashMap::new(),
            rbounds: HashMap::new(),
        }
    }
}

impl AutopairConfig {
    /// Read the live param overrides (ap:3-25 — every table is a user-visible
    /// global the plugin lets users mutate at any time).
    pub fn from_params() -> Self {
        let mut cfg = Self::default();
        if let Some(flat) = gethparam("AUTOPAIR_PAIRS") {
            let pairs: Vec<(char, char)> = flat
                .chunks_exact(2)
                .filter_map(|kv| {
                    let mut kc = kv[0].chars();
                    let mut vc = kv[1].chars();
                    match (kc.next(), kc.next(), vc.next(), vc.next()) {
                        (Some(k), None, Some(v), None) => Some((k, v)),
                        _ => None,
                    }
                })
                .collect();
            if !pairs.is_empty() {
                cfg.pairs = pairs;
            }
        }
        if let Some(flat) = gethparam("AUTOPAIR_LBOUNDS") {
            for kv in flat.chunks_exact(2) {
                cfg.lbounds.insert(kv[0].clone(), kv[1].clone());
            }
        }
        if let Some(flat) = gethparam("AUTOPAIR_RBOUNDS") {
            for kv in flat.chunks_exact(2) {
                cfg.rbounds.insert(kv[0].clone(), kv[1].clone());
            }
        }
        cfg.between_whitespace = getsparam("AUTOPAIR_BETWEEN_WHITESPACE")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        cfg
    }

    fn lbound(&self, group: &str) -> Option<String> {
        self.lbounds
            .get(group)
            .cloned()
            .or_else(|| default_lbound(group).map(str::to_owned))
    }

    fn rbound(&self, group: &str) -> Option<String> {
        self.rbounds
            .get(group)
            .cloned()
            .or_else(|| default_rbound(group).map(str::to_owned))
    }
}

/// ap:30-40 — `_ap-get-pair`: the closer for an opener (`by_open`), or the
/// opener for a closer (`by_close`, the `$2` branch).
pub fn get_pair_by_open(cfg: &AutopairConfig, open: char) -> Option<char> {
    cfg.pairs.iter().find(|(o, _)| *o == open).map(|(_, c)| *c)
}

pub fn get_pair_by_close(cfg: &AutopairConfig, close: char) -> Option<char> {
    cfg.pairs.iter().find(|(_, c)| *c == close).map(|(o, _)| *o)
}

/// Compiled-regex cache — the boundary tables are matched on every candidate
/// keystroke; compiling per keypress would dwarf the match cost.
static REGEX_CACHE: Mutex<Option<HashMap<String, Option<regex::Regex>>>> = Mutex::new(None);

fn cached_regex_matches(pattern: &str, hay: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    let mut guard = REGEX_CACHE.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() > 256 {
        map.clear();
    }
    let entry = map.entry(pattern.to_owned()).or_insert_with(|| {
        regex::Regex::new(pattern)
            .ok()
            .or_else(|| regex::Regex::new(&zsh_ere_to_rust(pattern)).ok())
    });
    entry.as_ref().map(|re| re.is_match(hay)).unwrap_or(false)
}

/// ap:42-45 — `_ap-boundary-p`: cursor's surroundings match either regexp —
/// `$1` anchored at LBUFFER's end, `$2` anchored at RBUFFER's start.
fn boundary_p(l_pat: Option<&str>, r_pat: Option<&str>, lbuf: &str, rbuf: &str) -> bool {
    let l_hit = l_pat
        .filter(|p| !p.is_empty())
        .map(|p| cached_regex_matches(&format!("(?:{p})$"), lbuf))
        .unwrap_or(false);
    let r_hit = r_pat
        .filter(|p| !p.is_empty())
        .map(|p| cached_regex_matches(&format!("^(?:{p})"), rbuf))
        .unwrap_or(false);
    l_hit || r_hit
}

/// ap:47-62 — `_ap-next-to-boundary-p`: the surrounding text matches any of
/// the AUTOPAIR_*BOUNDS groups that apply to this delimiter.
fn next_to_boundary_p(cfg: &AutopairConfig, key: char, lbuf: &str, rbuf: &str) -> bool {
    let mut groups: Vec<String> = vec!["all".to_owned()];
    match key {
        '\'' | '"' | '`' => groups.push("quotes".to_owned()),
        '{' | '[' | '(' | '<' => groups.push("braces".to_owned()),
        ' ' => groups.push("spaces".to_owned()),
        _ => (),
    }
    groups.push(key.to_string()); // ap:56
    for group in &groups {
        if boundary_p(
            cfg.lbound(group).as_deref(),
            cfg.rbound(group).as_deref(),
            lbuf,
            rbuf,
        ) {
            return true;
        }
    }
    false
}

/// Count occurrences of `ch` in `s` after removing every backslash-escaped
/// occurrence (ap:67-70 — `${LBUFFER//\\$1}` then `${#lbuf//[^$1]}`).
fn unescaped_count(s: &str, ch: char) -> usize {
    let stripped: String = {
        // remove every `\<ch>` two-char sequence
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' && chars.peek() == Some(&ch) {
                chars.next();
                continue;
            }
            out.push(c);
        }
        out
    };
    stripped.chars().filter(|&c| c == ch).count()
}

/// ap:64-97 — `_ap-balanced-p`: same number of openers as closers.
fn balanced_p(open: char, close: char, lbuf: &str, rbuf: &str) -> bool {
    let llen = unescaped_count(lbuf, open);
    let rlen = unescaped_count(rbuf, close);
    if rlen == 0 && llen == 0 {
        return true; // ap:71-72
    }
    if open == close {
        if open == ' ' {
            // ap:74-82 — Balancing spaces is unnecessary. If there is at least
            // one space on either side of the cursor, it is considered
            // balanced: LBUFFER =~ "[^'\"]([ \t]+)$" and RBUFFER starts with
            // the same whitespace run.
            let lchars: Vec<char> = lbuf.chars().collect();
            let mut i = lchars.len();
            while i > 0 && matches!(lchars[i - 1], ' ' | '\t') {
                i -= 1;
            }
            let run: String = lchars[i..].iter().collect();
            let preceded_ok = !run.is_empty()
                && (i == 0 || !matches!(lchars[i - 1], '\'' | '"'));
            return preceded_ok && !run.is_empty() && rbuf.starts_with(&run);
        } else if llen == rlen || (llen + rlen) % 2 == 0 {
            return true; // ap:83-84
        }
    } else {
        // ap:86-94
        let l2len = unescaped_count(lbuf, close);
        let r2len = unescaped_count(rbuf, open);
        let ltotal = (llen as i64 - l2len as i64).max(0);
        let rtotal = rlen as i64 - r2len as i64;
        return ltotal >= rtotal;
    }
    false // ap:96
}

/// ap:99-124 — `_ap-can-pair-p`: the last keypress can be auto-paired.
fn can_pair_p(cfg: &AutopairConfig, key: char, lbuf: &str, rbuf: &str) -> bool {
    let Some(rchar) = get_pair_by_open(cfg, key) else {
        return false; // ap:103
    };

    if rchar != ' ' {
        // ap:105-110 — Force pair if surrounded by space/[BE]OL, regardless of
        // boundaries/balance.
        if cfg.between_whitespace
            && lbuf
                .chars()
                .next_back()
                .map(|c| matches!(c, ' ' | '\t'))
                .unwrap_or(true)
            && rbuf
                .chars()
                .next()
                .map(|c| matches!(c, ' ' | '\t'))
                .unwrap_or(true)
        {
            return true;
        }
        // ap:112-113 — Don't pair quotes if the delimiters are unbalanced.
        if !balanced_p(key, rchar, lbuf, rbuf) {
            return false;
        }
    } else if rbuf.chars().all(|c| matches!(c, ' ' | '\t')) {
        // ap:114-116 — Don't pair spaces surrounded by whitespace.
        return false;
    }

    // ap:119-121 — Don't pair when in front of characters that likely signify
    // the start of a string, path or undesirable boundary.
    if next_to_boundary_p(cfg, key, lbuf, rbuf) {
        return false;
    }

    true
}

/// ap:126-141 — `_ap-can-skip-p`: the adjacent character (on the right) can be
/// safely skipped over.
fn can_skip_p(open: Option<char>, close: Option<char>, lbuf: &str, rbuf: &str) -> bool {
    if lbuf.is_empty() {
        return false; // ap:128-129
    }
    let Some(close) = close else {
        return false;
    };
    if open == Some(close) {
        if close == ' ' {
            return false; // ap:131-132
        }
        if !balanced_p(close, close, lbuf, rbuf) {
            return false; // ap:133-135
        }
    }
    // ap:137-139 — next char must BE the closer, and not backslash-escaped.
    rbuf.starts_with(close) && !lbuf.ends_with('\\')
}

/// ap:143-157 — `_ap-can-delete-p`: the adjacent character (on the right) can
/// be safely deleted along with the one about to be backspaced.
fn can_delete_p(cfg: &AutopairConfig, lbuf: &str, rbuf: &str) -> bool {
    let Some(lchar) = lbuf.chars().next_back() else {
        return false;
    };
    let Some(rchar) = get_pair_by_open(cfg, lchar) else {
        return false; // ap:147
    };
    if !rbuf.starts_with(rchar) {
        return false; // ap:147
    }
    if lchar == rchar {
        if lchar == ' ' {
            // ap:149-151 — Don't collapse spaces unless in delimiters:
            // LBUFFER =~ "[^{([] +$" or RBUFFER =~ "^ +[^]})]".
            if cached_regex_matches(r"[^{(\[] +$", lbuf)
                || cached_regex_matches(r"^ +[^\]})]", rbuf)
            {
                return false;
            }
        } else if !balanced_p(lchar, rchar, lbuf, rbuf) {
            return false; // ap:152-154
        }
    }
    true
}

/// What the wiring should do with the pending keypress/widget.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Insert `open` + `close` and park the cursor between (ap:159-163
    /// `_ap-self-insert`).
    InsertPair(char, char),
    /// Move right over the already-present closer (ap:171/183 `zle forward-char`).
    SkipOver,
    /// Delete the char right of the cursor, THEN run the original deleting
    /// widget (ap:189-197 — `RBUFFER=${RBUFFER:1}` before delegating).
    DeleteRightThenPassthrough,
    /// Run the originally bound widget untouched.
    Passthrough,
}

/// ap:168-197 — the widget bodies, folded into one decision function.
/// `widget` is the ZLE widget about to run; `key` the byte that invoked it
/// (KEYS). Buffers are the text around the cursor.
pub fn decide(
    cfg: &AutopairConfig,
    widget: &str,
    key: Option<char>,
    lbuf: &str,
    rbuf: &str,
) -> Action {
    match widget {
        // ap:168-179 + ap:181-187 — self-insert of a pair char.
        "self-insert" => {
            let Some(key) = key else {
                return Action::Passthrough;
            };
            let as_open = get_pair_by_open(cfg, key);
            // ap:168-179 — autopair-insert (openers, incl. identical-pair
            // quotes/space: skip-over first, then pair, else plain insert).
            if let Some(rchar) = as_open {
                if matches!(key, '\'' | '"' | '`' | ' ')
                    && can_skip_p(Some(key), Some(rchar), lbuf, rbuf)
                {
                    return Action::SkipOver; // ap:170-171
                }
                if can_pair_p(cfg, key, lbuf, rbuf) {
                    return Action::InsertPair(key, rchar); // ap:172-173
                }
                return Action::Passthrough; // ap:174-178
            }
            // ap:181-187 — autopair-close (distinct closers: ) ] }).
            if let Some(open) = get_pair_by_close(cfg, key) {
                if can_skip_p(Some(open), Some(key), lbuf, rbuf) {
                    return Action::SkipOver; // ap:182-183
                }
            }
            Action::Passthrough // ap:184-186
        }
        // ap:189-192 — autopair-delete; ap:194-197 — autopair-delete-word.
        "backward-delete-char"
        | "vi-backward-delete-char"
        | "backward-delete-word"
        | "backward-kill-word" => {
            if can_delete_p(cfg, lbuf, rbuf) {
                Action::DeleteRightThenPassthrough
            } else {
                Action::Passthrough
            }
        }
        _ => Action::Passthrough,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AutopairConfig {
        AutopairConfig::default()
    }

    // ap:168-179 — opener inserts the pair, cursor between.
    #[test]
    fn paren_pairs_on_empty_line() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some('('), "", ""),
            Action::InsertPair('(', ')')
        );
        assert_eq!(
            decide(&cfg(), "self-insert", Some('['), "echo ", ""),
            Action::InsertPair('[', ']')
        );
        assert_eq!(
            decide(&cfg(), "self-insert", Some('{'), "x=", ""),
            Action::InsertPair('{', '}')
        );
    }

    // ap:119-121 — no pairing directly before word characters ("all" rbound).
    #[test]
    fn no_pair_before_word_char() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some('('), "", "foo"),
            Action::Passthrough
        );
    }

    // ap:12-19 quotes group — no quote pairing right after a word char.
    #[test]
    fn no_quote_pair_after_word_char() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some('\''), "don", ""),
            Action::Passthrough
        );
    }

    #[test]
    fn quote_pairs_on_fresh_word() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some('"'), "echo ", ""),
            Action::InsertPair('"', '"')
        );
    }

    // ap:181-187 — typing a closer over the auto-inserted closer skips.
    #[test]
    fn closer_skips_over() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some(')'), "echo (", ")"),
            Action::SkipOver
        );
        // No closer present: plain insert.
        assert_eq!(
            decide(&cfg(), "self-insert", Some(')'), "echo (", " x"),
            Action::Passthrough
        );
    }

    // ap:170-171 — identical-pair quote skip.
    #[test]
    fn quote_skips_over_balanced() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some('\''), "echo 'hi", "'"),
            Action::SkipOver
        );
    }

    // ap:137 — escaped closer is not skippable.
    #[test]
    fn escaped_closer_not_skipped() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some(')'), "echo (\\", ")x"),
            Action::Passthrough
        );
    }

    // ap:189-192 — backspace between a pair deletes both.
    #[test]
    fn backspace_deletes_pair() {
        assert_eq!(
            decide(&cfg(), "backward-delete-char", None, "echo (", ")"),
            Action::DeleteRightThenPassthrough
        );
        assert_eq!(
            decide(&cfg(), "backward-delete-char", None, "echo (", "x)"),
            Action::Passthrough
        );
    }

    // ap:112-113 — unbalanced quotes don't pair.
    #[test]
    fn unbalanced_quote_does_not_pair() {
        // One quote already open to the left; typing another closes it
        // (llen=1, rlen=0 → odd sum → unbalanced → passthrough insert).
        assert_eq!(
            decide(&cfg(), "self-insert", Some('\''), "echo 'abc ", ""),
            Action::Passthrough
        );
    }

    // ap:66-70 — escaped delimiters don't count toward balance.
    #[test]
    fn escaped_quote_ignored_in_balance() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some('\''), "echo \\' ", ""),
            Action::InsertPair('\'', '\'')
        );
    }

    // ap:114-116 — space doesn't pair in whitespace-only right context.
    #[test]
    fn space_needs_delimiters() {
        assert_eq!(
            decide(&cfg(), "self-insert", Some(' '), "echo", "  "),
            Action::Passthrough
        );
        // Inside braces: space pads.
        assert_eq!(
            decide(&cfg(), "self-insert", Some(' '), "x={", "}"),
            Action::InsertPair(' ', ' ')
        );
    }

    // ap:149-151 — spaces only collapse inside delimiters.
    #[test]
    fn space_collapse_only_in_delimiters() {
        assert_eq!(
            decide(&cfg(), "backward-delete-char", None, "x={ ", " }"),
            Action::DeleteRightThenPassthrough
        );
        assert_eq!(
            decide(&cfg(), "backward-delete-char", None, "echo a ", " b"),
            Action::Passthrough
        );
    }

    // ap:105-110 — AUTOPAIR_BETWEEN_WHITESPACE forces pairing.
    #[test]
    fn between_whitespace_forces_pair() {
        let mut c = cfg();
        c.between_whitespace = true;
        assert_eq!(
            decide(&c, "self-insert", Some('('), "word ", " word"),
            Action::InsertPair('(', ')')
        );
    }
}
