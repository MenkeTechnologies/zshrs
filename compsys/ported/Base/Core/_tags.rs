//! Port of `_tags` (zsh Completion/Base/Core/_tags, 67 lines).
//!
//! Local shell reference: `compsys/functions/Base/Core/_tags`.
//!
//! Two-mode entry point used pervasively by every per-command
//! completer:
//!
//!   1. **Setup mode**: `_tags TAG1 TAG2 ...` registers the offered
//!      tag set for this completion, applies `group-order` /
//!      `tag-order` zstyles to decide the iteration order, and arms
//!      the underlying `TagManager`.
//!   2. **Iteration mode**: `_tags` (no args) advances to the next
//!      try-set. Returns true while more sets remain.
//!
//! Faithful semantics added vs the previous `TagManager`:
//!
//!   - `-C name`              — push a context segment onto curcontext
//!                              for the lookups (shell:21-27).
//!   - `--` separator         — eaten silently between flags and tags.
//!   - `group-order` zstyle   — drives the display-group ordering on
//!                              the receiver (shell:32 `compgroups`).
//!   - `tag-order` zstyle     — drives the try-set order. Honors:
//!       * `-`                — suppress the implicit default-all try
//!       * `!pat`             — try every offered tag EXCEPT those
//!                              matching one of the comma-separated
//!                              patterns (shell:48 negate branch)
//!       * `pat`              — try the offered tags matching pat
//!                              (shell:49 `comptry -m`)
//!   - `$_sort_tags` hook     — exposed as `TagSortHook` closure
//!                              passed in by the caller; if Some,
//!                              `tag-order` is NOT consulted (matches
//!                              shell:38-41).

use crate::base::{MainCompleteState, TagManager};
use crate::compcore::CompletionState;
use crate::zstyle::ZStyleStore;

/// Optional `$_sort_tags` hook. When Some, replaces all `tag-order` /
/// negate-pattern processing — the hook is called with the offered
/// tag list and is expected to call `add_try` directly on the
/// TagManager. Mirrors shell:38-41 `"$_sort_tags" "$@"`.
pub type TagSortHook<'a> = Option<&'a mut dyn FnMut(&mut TagManager, &[String])>;

pub struct TagsOpts<'a> {
    /// `-C context-suffix` — pushed onto curcontext for the lookups.
    pub context_suffix: Option<&'a str>,
}

impl<'a> Default for TagsOpts<'a> {
    fn default() -> Self {
        Self { context_suffix: None }
    }
}

/// Setup-mode entry point. Mirrors `_tags TAG ...` with no `--`-prefix
/// (the `--` switch is consumed by the caller before invocation; we
/// accept the already-cleaned offered list).
///
/// Returns `true` when at least one try-set was armed (i.e. tag
/// iteration should proceed).
pub fn _tags(
    state: &mut CompletionState,
    styles: &ZStyleStore,
    tags: &mut TagManager,
    curcontext: &str,
    offered: &[String],
    opts: &TagsOpts<'_>,
    mut sort_hook: TagSortHook<'_>,
) -> bool {
    // shell:21-27 `-C` context push.
    let effective_ctx = match opts.context_suffix {
        Some(suf) => {
            let base = curcontext.trim_end_matches(':');
            let stripped = match base.rfind(':') {
                Some(i) => &base[..i],
                None => base,
            };
            format!("{}:{}", stripped, suf)
        }
        None => curcontext.to_string(),
    };

    // Register the offered tag set.
    tags.init(offered);

    // shell:30-32 group-order zstyle → drive the receiver's display
    // ordering.
    let style_ctx = format!(":completion:{}:", effective_ctx);
    if let Some(group_order) = styles.lookup_values(&style_ctx, "group-order") {
        state.apply_group_order(group_order);
    }

    // shell:38-41: `$_sort_tags` hook short-circuits tag-order.
    if let Some(hook) = sort_hook.as_deref_mut() {
        hook(tags, offered);
        return tags.start();
    }

    // shell:42-45 default tag-order when the style isn't set AND
    // `options` is part of the offered list — push the magical
    // implicit ordering: `(|*-)argument-* (|*-)option[-+]* values`
    // FIRST, then `options`. Matches shell:42-45.
    let style_order = styles.lookup_values(&style_ctx, "tag-order");
    let order_vec: Vec<String> = match style_order {
        Some(v) => v.to_vec(),
        None => {
            if offered.iter().any(|t| t == "options") {
                vec![
                    "(|*-)argument-* (|*-)option[-+]* values".into(),
                    "options".into(),
                ]
            } else {
                Vec::new()
            }
        }
    };

    let mut suppress_default = false;
    for tag_spec in &order_vec {
        if tag_spec == "-" {
            // shell:47 → flag: don't try the implicit default-all set.
            suppress_default = true;
        } else if let Some(rest) = tag_spec.strip_prefix('!') {
            // shell:48: `!pat[,pat...]` — try every offered tag NOT
            // matching any of these patterns.
            let pats: Vec<&str> = rest.split('|').collect();
            let try_set: Vec<String> = offered
                .iter()
                .filter(|t| !pats.iter().any(|p| match_glob(p, t)))
                .cloned()
                .collect();
            if !try_set.is_empty() {
                tags.add_try(&try_set);
            }
        } else {
            // shell:49: `comptry -m tag` → try the offered tags
            // matching the (whitespace-separated) patterns.
            let pats: Vec<&str> = tag_spec.split_whitespace().collect();
            let try_set: Vec<String> = offered
                .iter()
                .filter(|t| pats.iter().any(|p| match_glob(p, t)))
                .cloned()
                .collect();
            if !try_set.is_empty() {
                tags.add_try(&try_set);
            }
        }
    }

    // shell:53 `[[ -z "$nodef" ]] && comptry "$@"` — default-all try
    // unless suppressed.
    if !suppress_default {
        tags.add_try(offered);
    }

    tags.start()
}

/// Iteration-mode entry: `_tags` with no args. Returns true while
/// more try-sets remain.
pub fn _tags_next(tags: &mut TagManager) -> bool {
    tags.next()
}

/// Convenience for callers that own a `MainCompleteState` — uses its
/// `styles`, `tags`, and `ctx.context`.
pub fn _tags_mcs(
    state: &mut MainCompleteState,
    offered: &[String],
    opts: &TagsOpts<'_>,
    sort_hook: TagSortHook<'_>,
) -> bool {
    // Borrow gymnastics: we want to call into state.tags while
    // reading from state.styles. Clone the styles for the lookup.
    let styles = state.styles.clone();
    let ctx = state.ctx.context.clone();
    _tags(
        &mut state.comp,
        &styles,
        &mut state.tags,
        &ctx,
        offered,
        opts,
        sort_hook,
    )
}

/// Match a shell glob pattern against a tag. Supports `*` / `?`,
/// alternation `(a|b)`, and character classes `[chars]` —
/// sufficient for the patterns in `tag-order` zstyle
/// (`argument-*`, `(|*-)option[-+]*`, `(|*-)values`, etc.).
fn match_glob(pat: &str, text: &str) -> bool {
    if pat == text {
        return true;
    }
    // Expand top-level `(alt|alt|...)` groups via recursion.
    if let Some(open) = pat.find('(') {
        if let Some(close) = pat[open..].find(')') {
            let abs_close = open + close;
            let prefix = &pat[..open];
            let alts: Vec<&str> = pat[open + 1..abs_close].split('|').collect();
            let suffix = &pat[abs_close + 1..];
            return alts
                .iter()
                .any(|alt| match_glob(&format!("{}{}{}", prefix, alt, suffix), text));
        }
    }
    glob_with_classes(pat, text)
}

fn glob_with_classes(pat: &str, text: &str) -> bool {
    glob_helper(&pat.chars().collect::<Vec<_>>(), &text.chars().collect::<Vec<_>>())
}

fn glob_helper(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    match pat[0] {
        '*' => (0..=txt.len()).any(|i| glob_helper(&pat[1..], &txt[i..])),
        '?' => !txt.is_empty() && glob_helper(&pat[1..], &txt[1..]),
        '[' => {
            // Find closing `]`.
            let close = match pat.iter().position(|&c| c == ']') {
                Some(i) => i,
                None => return false,
            };
            if txt.is_empty() {
                return false;
            }
            let class: &[char] = &pat[1..close];
            // Members can be literal chars or `a-z` ranges. For the
            // tag patterns we see, the class is just a couple of
            // literal chars (`[-+]`), so handle that and ranges.
            let mut matched = false;
            let mut i = 0;
            while i < class.len() {
                if i + 2 < class.len() && class[i + 1] == '-' {
                    if txt[0] >= class[i] && txt[0] <= class[i + 2] {
                        matched = true;
                        break;
                    }
                    i += 3;
                } else {
                    if txt[0] == class[i] {
                        matched = true;
                        break;
                    }
                    i += 1;
                }
            }
            matched && glob_helper(&pat[close + 1..], &txt[1..])
        }
        c => !txt.is_empty() && txt[0] == c && glob_helper(&pat[1..], &txt[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tag_order_when_style_unset_options_present() {
        // Offered with `options` → implicit default-order injects
        // `(|*-)argument-* (|*-)option[-+]* values` first, then
        // `options`, then default-all (offered).
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        let mut tags = TagManager::new();
        let offered = vec![
            "argument-1".into(),
            "option-1".into(),
            "options".into(),
            "values".into(),
        ];
        let ok = _tags(
            &mut state,
            &styles,
            &mut tags,
            ":complete::test:",
            &offered,
            &TagsOpts::default(),
            None,
        );
        assert!(ok);
        // First try-set should be the implicit `argument/option/values`
        // grouping.
        assert!(tags.wanted("argument-1"));
        assert!(tags.wanted("option-1"));
        assert!(tags.wanted("values"));
        assert!(!tags.wanted("options"), "options is in a LATER try-set");
    }

    #[test]
    fn tag_order_negate_pattern_excludes_matching_tags() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::complete::test::",
            "tag-order",
            vec!["!option*".into()],
            false,
        );
        let mut tags = TagManager::new();
        let offered = vec![
            "argument-1".into(),
            "option-1".into(),
            "option-2".into(),
            "values".into(),
        ];
        _tags(
            &mut state,
            &styles,
            &mut tags,
            ":complete::test:",
            &offered,
            &TagsOpts::default(),
            None,
        );
        // First try-set: !option* → argument-1, values (NOT option-*).
        assert!(tags.wanted("argument-1"));
        assert!(tags.wanted("values"));
        assert!(!tags.wanted("option-1"));
        assert!(!tags.wanted("option-2"));
    }

    #[test]
    fn tag_order_dash_suppresses_default_all() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::complete::test::",
            "tag-order",
            vec!["argument-1".into(), "-".into()],
            false,
        );
        let mut tags = TagManager::new();
        let offered = vec!["argument-1".into(), "values".into()];
        _tags(
            &mut state,
            &styles,
            &mut tags,
            ":complete::test:",
            &offered,
            &TagsOpts::default(),
            None,
        );
        // Try-set: just argument-1 (no default-all fallback).
        assert!(tags.wanted("argument-1"));
        assert!(!tags.wanted("values"));
        // Advance — no more try-sets because `-` suppressed default.
        let more = _tags_next(&mut tags);
        assert!(!more, "`-` should have suppressed the default-all fallback");
    }

    #[test]
    fn sort_hook_short_circuits_tag_order() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        // tag-order says argument-1 only, but the sort hook will
        // override to push EVERYTHING.
        styles.set(
            ":completion::complete::test::",
            "tag-order",
            vec!["argument-1".into()],
            false,
        );
        let mut tags = TagManager::new();
        let offered = vec!["argument-1".into(), "values".into()];
        let mut hook = |t: &mut TagManager, offered: &[String]| {
            t.add_try(offered);
        };
        _tags(
            &mut state,
            &styles,
            &mut tags,
            ":complete::test:",
            &offered,
            &TagsOpts::default(),
            Some(&mut hook),
        );
        assert!(tags.wanted("argument-1"));
        assert!(tags.wanted("values"), "sort hook should have included values");
    }
}
