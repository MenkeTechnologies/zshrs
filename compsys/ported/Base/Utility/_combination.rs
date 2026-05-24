//! Port of `_combination` (zsh Completion/Base/Utility/_combination, 102 lines).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_combination`.
//!
//! Multi-axis completion driven by a tag/style holding a list of
//! `:`-separated (by default) tuples. The marquee use case is telnet:
//!
//! ```text
//! zstyle ':completion:*:*:telnet:*:my-accounts' users-hosts-ports \
//!   @host0: user1@host1: user2@host2: \
//!   @mail-server:{smtp,pop3} \
//!   @news-server:nntp \
//!   @proxy-server:8000
//!
//! _combination my-accounts users-hosts-ports \
//!   hosts="$opt_args[-l]" \
//!   ports
//! ```
//!
//! The previous Rust stub took `specs: &[(&str, Vec<String>)]` and
//! emitted `key=value` strings — entirely wrong shape. Re-port from
//! scratch.
//!
//! Algorithm (mirrors shell:69-101):
//!   1. Split `style` by `-` → axis-key list (`users / hosts / ports`).
//!   2. Init patterns to `*` per axis (matches anything).
//!   3. Walk `K[:N]=Pattern` fixed-axis args, install Pattern at the
//!      N-th occurrence of K in the axis list.
//!   4. Last positional `K[:N]` (no `=`) is the **target axis** —
//!      this is what the user wants completed.
//!   5. Look up `zstyle ":completion:$curcontext:$tag" $style` for a
//!      list of tuple strings.
//!   6. Keep only tuples where each axis matches its pattern (using
//!      `${(j(sep))pats}` joined glob).
//!   7. Strip the first `(target_axis_position - 1)` axis fields from
//!      each tuple (so what remains starts at the target axis).
//!   8. Take the first axis value of each remaining tuple — these are
//!      the candidates.
//!   9. compadd them, or call `_$target_key` as fallback if nothing.

use crate::base::MainCompleteState;
use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::glob_matches;

pub struct CombinationOpts<'a> {
    /// `-s sep` flag. Default `:`.
    pub separator: &'a str,
    /// Tag (shell `$1` after flag consumption).
    pub tag: &'a str,
    /// Style key (shell `$2`). Hyphen-separated axis names.
    pub style: &'a str,
    /// Fixed-axis constraints, mirroring shell `K[:N]=Pattern` args.
    /// `(axis_key, occurrence_num, pattern)`. `occurrence_num` is
    /// 1-based — picks the N-th occurrence of `axis_key` in the
    /// axis list.
    pub fixed: &'a [(String, usize, String)],
    /// The target axis-key the user wants completed (shell's last
    /// `K[:N]` positional). Pattern is the same:
    /// `(target_key, occurrence_num)`. `occurrence_num` is 1-based.
    pub target_key: &'a str,
    pub target_num: usize,
}

/// `fallback` runs when (a) the zstyle isn't set OR (b) the style is
/// set but our filtered list is empty AND compadd produces no matches.
/// Shell calls `_$target_key` — caller wires this to the right Rust
/// completer.
pub fn _combination<F>(
    state: &mut CompletionState,
    style_store: &crate::zstyle::ZStyleStore,
    curcontext: &str,
    opts: &CombinationOpts<'_>,
    fallback: F,
) -> bool
where
    F: FnOnce(&mut CompletionState) -> bool,
{
    let sep = opts.separator;
    let keys: Vec<&str> = opts.style.split('-').collect();
    if keys.is_empty() {
        return fallback(state);
    }

    // Initialise patterns = ["*"; keys.len()].
    let mut pats: Vec<String> = vec!["*".into(); keys.len()];

    // Walk fixed args. Find the n-th occurrence of `axis_key` in keys
    // (1-based) and install `pattern` at that position.
    for (axis, num, pattern) in opts.fixed {
        let n = (*num).max(1);
        let mut seen = 0;
        for (i, k) in keys.iter().enumerate() {
            if k == axis {
                seen += 1;
                if seen == n {
                    pats[i] = pattern.clone();
                    break;
                }
            }
        }
    }

    // Find target axis position (1-based for shell-compat math).
    let mut target_pos = 0usize;
    {
        let n = opts.target_num.max(1);
        let mut seen = 0;
        for (i, k) in keys.iter().enumerate() {
            if *k == opts.target_key {
                seen += 1;
                if seen == n {
                    target_pos = i;
                    break;
                }
            }
        }
    }

    // shell:88 `zstyle -a ":completion:${curcontext}:$tag" "$style" tmp`
    let style_ctx = format!(":completion:{}:{}", curcontext, opts.tag);
    let values = match style_store.lookup_values(&style_ctx, opts.style) {
        Some(v) => v.to_vec(),
        None => {
            return fallback(state);
        }
    };

    // shell:89 `tmp=( "${(@M)tmp:#${(j(sep))~pats}}" )`
    // — keep only tuples whose JOINED-by-sep form matches the joined
    // patterns under glob semantics.
    let joined_pat = pats.join(sep);
    let mut filtered: Vec<String> = values
        .into_iter()
        .filter(|v| glob_matches(&joined_pat, v))
        .collect();

    // shell:90-92: if target isn't the first axis (target_pos > 0),
    // strip the leading `target_pos` axis-segments + the separator.
    if target_pos > 0 {
        let drop_prefix_seg_count = target_pos;
        filtered = filtered
            .into_iter()
            .filter_map(|s| {
                let mut iter = s.splitn(drop_prefix_seg_count + 1, sep);
                // Skip target_pos segments, take the rest.
                for _ in 0..drop_prefix_seg_count {
                    iter.next()?;
                }
                iter.next().map(String::from)
            })
            .collect();
    }

    // shell:93 `tmp=( ${tmp%%${~sep}*} )` — keep only the FIRST
    // remaining axis value. Dedup mirrors shell's `compadd`
    // implicit unique-match table (same candidate emitted twice
    // collapses to one).
    let mut seen = std::collections::HashSet::new();
    let candidates: Vec<String> = filtered
        .into_iter()
        .map(|s| match s.find(sep) {
            Some(i) => s[..i].to_string(),
            None => s,
        })
        .filter(|s| !s.is_empty())
        .filter(|s| seen.insert(s.clone()))
        .collect();

    if candidates.is_empty() {
        return fallback(state);
    }

    // compadd them. Use the target_key as the tag/group name.
    state.begin_group(opts.target_key, true);
    let prefix = state.params.prefix.clone();
    let mut added = false;
    for c in &candidates {
        if c.starts_with(&prefix) {
            state.add_match(Completion::new(c.clone()), Some(opts.target_key));
            added = true;
        }
    }
    state.end_group();

    if !added {
        return fallback(state);
    }
    true
}

/// Convenience wrapper that takes a [`MainCompleteState`] and reuses
/// its curcontext / style store automatically.
pub fn _combination_mcs<F>(
    state: &mut MainCompleteState,
    opts: &CombinationOpts<'_>,
    fallback: F,
) -> bool
where
    F: FnOnce(&mut CompletionState) -> bool,
{
    let ctx = state.ctx.context.clone();
    let styles = state.styles.clone();
    _combination(&mut state.comp, &styles, &ctx, opts, fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zstyle::ZStyleStore;

    fn never_fallback(_state: &mut CompletionState) -> bool {
        panic!("fallback should not be called in this test");
    }

    #[test]
    fn telnet_users_hosts_ports_completes_hosts() {
        // Canonical 3-axis tuples (user:host:port) for the
        // users-hosts-ports style.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::telnet:my-accounts",
            "users-hosts-ports",
            vec![
                "user1:host1:22".into(),
                "user2:host2:22".into(),
                ":mail-server:smtp".into(),
            ],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "my-accounts",
            style: "users-hosts-ports",
            fixed: &[],
            target_key: "hosts",
            target_num: 1,
        };
        let ok = _combination(&mut state, &styles, ":telnet", &opts, never_fallback);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"host1"), "got {names:?}");
        assert!(names.contains(&"host2"));
        assert!(names.contains(&"mail-server"));
    }

    #[test]
    fn fixed_user_constraint_filters_hosts() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::telnet:my-accounts",
            "users-hosts-ports",
            vec![
                "user1:host1:22".into(),
                "user2:host2:22".into(),
                "user1:host3:22".into(),
            ],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "my-accounts",
            style: "users-hosts-ports",
            fixed: &[("users".into(), 1, "user1".into())],
            target_key: "hosts",
            target_num: 1,
        };
        let ok = _combination(&mut state, &styles, ":telnet", &opts, never_fallback);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"host1"));
        assert!(names.contains(&"host3"));
        assert!(!names.contains(&"host2"), "user2 host should be filtered");
    }

    #[test]
    fn fallback_runs_when_style_unset() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();
        let mut fallback_ran = false;
        // Box the closure so we can mutate fallback_ran.
        let opts = CombinationOpts {
            separator: ":",
            tag: "tag",
            style: "a-b",
            fixed: &[],
            target_key: "b",
            target_num: 1,
        };
        _combination(&mut state, &styles, ":x", &opts, |_| {
            fallback_ran = true;
            true
        });
        assert!(fallback_ran);
    }

    #[test]
    fn empty_target_field_skipped_in_results() {
        // Tuples with an empty field at the target position should
        // be skipped (the empty string isn't a useful completion).
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::svc:accounts",
            "users-hosts-ports",
            vec![
                "u1:h1:22".into(),
                "u2::33".into(), // empty host slot
                "u3:h3:44".into(),
            ],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "accounts",
            style: "users-hosts-ports",
            fixed: &[],
            target_key: "hosts",
            target_num: 1,
        };
        let _ = _combination(&mut state, &styles, ":svc", &opts, never_fallback);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"h1"));
        assert!(names.contains(&"h3"));
        assert!(
            !names.iter().any(|n| n.is_empty()),
            "empty target must NOT appear; got {names:?}"
        );
    }

    #[test]
    fn multi_constraint_filter_works() {
        // Constrain both user AND port.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::svc:accounts",
            "users-hosts-ports",
            vec![
                "u1:h1:22".into(),
                "u1:h2:80".into(),
                "u2:h1:22".into(),
                "u1:h3:22".into(),
            ],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "accounts",
            style: "users-hosts-ports",
            fixed: &[
                ("users".into(), 1, "u1".into()),
                ("ports".into(), 1, "22".into()),
            ],
            target_key: "hosts",
            target_num: 1,
        };
        let _ = _combination(&mut state, &styles, ":svc", &opts, never_fallback);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // Only u1+22 tuples: h1, h3
        assert!(names.contains(&"h1"));
        assert!(names.contains(&"h3"));
        assert!(!names.contains(&"h2"), "h2 violates port constraint");
    }

    #[test]
    fn dedup_repeated_target_values() {
        // The same host appearing in multiple tuples shouldn't
        // emit duplicates.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::svc:accounts",
            "users-hosts-ports",
            vec![
                "u1:repeat:22".into(),
                "u2:repeat:80".into(),
                "u3:other:443".into(),
            ],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "accounts",
            style: "users-hosts-ports",
            fixed: &[],
            target_key: "hosts",
            target_num: 1,
        };
        let _ = _combination(&mut state, &styles, ":svc", &opts, never_fallback);
        let repeat_count = state.groups[0]
            .matches
            .iter()
            .filter(|c| c.str_ == "repeat")
            .count();
        assert_eq!(repeat_count, 1, "repeated target value must dedupe");
    }

    #[test]
    fn prefix_filter_narrows_emissions() {
        let mut state = CompletionState::new();
        state.params.prefix = "h1".into();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::svc:accounts",
            "users-hosts-ports",
            vec![
                "u1:h1:22".into(),
                "u2:h10:80".into(),
                "u3:other:443".into(),
            ],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "accounts",
            style: "users-hosts-ports",
            fixed: &[],
            target_key: "hosts",
            target_num: 1,
        };
        let _ = _combination(&mut state, &styles, ":svc", &opts, never_fallback);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"h1"));
        assert!(names.contains(&"h10"), "h10 should also match prefix h1");
        assert!(!names.contains(&"other"));
    }

    #[test]
    fn target_at_last_axis_position() {
        // Style `users-hosts-ports` with target_key="ports"
        // (position 2 — the last axis).
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::svc:accounts",
            "users-hosts-ports",
            vec!["u1:h1:22".into(), "u1:h2:80".into(), "u1:h3:443".into()],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "accounts",
            style: "users-hosts-ports",
            fixed: &[],
            target_key: "ports",
            target_num: 1,
        };
        let _ = _combination(&mut state, &styles, ":svc", &opts, never_fallback);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"22"));
        assert!(names.contains(&"80"));
        assert!(names.contains(&"443"));
    }

    #[test]
    fn target_at_first_axis_no_axis_strip() {
        // target_key="users" (position 0) — target_pos=0, no segment
        // stripping needed.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::svc:accounts",
            "users-hosts-ports",
            vec!["alice:h1:22".into(), "bob:h2:80".into()],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "accounts",
            style: "users-hosts-ports",
            fixed: &[],
            target_key: "users",
            target_num: 1,
        };
        let _ = _combination(&mut state, &styles, ":svc", &opts, never_fallback);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"alice"));
        assert!(names.contains(&"bob"));
    }

    #[test]
    fn glob_in_fixed_pattern_matches_multiple_tuples() {
        // Fixed `users=u*` should match u1, u2, u3 — all rows pass.
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::svc:accounts",
            "users-hosts-ports",
            vec![
                "u1:h1:22".into(),
                "u2:h2:80".into(),
                "admin:h3:443".into(),
            ],
            false,
        );
        let opts = CombinationOpts {
            separator: ":",
            tag: "accounts",
            style: "users-hosts-ports",
            fixed: &[("users".into(), 1, "u*".into())],
            target_key: "hosts",
            target_num: 1,
        };
        let _ = _combination(&mut state, &styles, ":svc", &opts, never_fallback);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"h1"));
        assert!(names.contains(&"h2"));
        assert!(!names.contains(&"h3"));
    }
}
