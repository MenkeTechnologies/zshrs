//! Port of `_pick_variant` (zsh Completion/Base/Utility/_pick_variant, 49 lines).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_pick_variant`.
//!
//! Detects which "variant" of a command is installed by running the
//! command once and pattern-matching the output. Used heavily by
//! per-command completers (GNU ls vs BSD ls, GNU grep vs BSD grep,
//! schedtool vs taskset, …) to switch implementations.
//!
//! Shell flags:
//!   - `-b builtin-label` : when the command is a shell builtin, use
//!                          this label as the variant (skips exec).
//!   - `-c command`       : command to run for the version probe.
//!                          Defaults to `$words[1]`.
//!   - `-r namevar`       : write the detected variant name into the
//!                          parameter named `namevar` (`(P)`-style
//!                          indirect assignment).
//!
//! Rest args: `label=pattern` pairs followed by argv to pass to the
//! probe command. First positional that does NOT contain `=` ends
//! the label list; everything from there on is appended to the
//! probe argv.
//!
//! Caching: writes the detected variant into `_cmd_variant[$cmd]`
//! so subsequent calls hit the cache instead of re-execing.
//! Modeled here with a process-global Mutex<HashMap>.
//!
//! Before-port stub: ran the probe command ONCE PER label tuple
//! (super wasteful) and had a different arg shape — caller couldn't
//! use it from the shell-port call sites.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::base::MainCompleteState;
use crate::ported::_call_program::{_call_program, CallProgramOpts, CallProgramResult};

/// Process-global `_cmd_variant` cache. Maps `cmd → variant-label`.
/// Lives for the lifetime of the process (matches shell `typeset -gA
/// _cmd_variant`).
fn cmd_variant_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<String, String>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Reset the cache — only used by tests.
#[doc(hidden)]
pub fn _reset_cache_for_tests() {
    cmd_variant_cache().lock().unwrap().clear();
}

pub struct PickVariantOpts<'a> {
    /// `-b builtin-label` — treat the command as a shell builtin
    /// and return this label without exec'ing.
    pub builtin_label: Option<&'a str>,
    /// `-c command` — probe command (defaults to the caller's
    /// command name passed via `default_cmd`).
    pub probe_cmd: Option<&'a str>,
    /// Default command name from `$words[1]`. Used when `-c` not set.
    pub default_cmd: &'a str,
    /// True if the user invoked via `builtin foo` (forces builtin
    /// resolution). Shell-side `$precommands[(I)builtin]` test.
    pub forced_builtin: bool,
    /// True if invoked via `command foo` (forces external command).
    /// Shell-side `${#precommands:|builtin_precommands}`.
    pub forced_command: bool,
    /// True if `default_cmd` is registered as a shell builtin.
    /// Caller looks this up since compsys can't reach the builtins
    /// table directly.
    pub cmd_is_builtin: bool,
}

impl<'a> PickVariantOpts<'a> {
    fn effective_cmd(&self) -> &str {
        self.probe_cmd.unwrap_or(self.default_cmd)
    }
}

#[derive(Debug, PartialEq)]
pub enum PickVariantResult {
    /// A label matched. The string is the matched variant label.
    Matched(String),
    /// No label matched. The string is the fallback that becomes
    /// the variant identifier (the first label in the input).
    Unmatched(String),
    /// The command was treated as a shell builtin and returned the
    /// `-b` label.
    Builtin(String),
}

impl PickVariantResult {
    pub fn label(&self) -> &str {
        match self {
            Self::Matched(s) | Self::Unmatched(s) | Self::Builtin(s) => s.as_str(),
        }
    }
}

/// `labels_and_argv` — pairs `(label, pattern)` followed by an argv
/// to append to the probe command (after the `--`, conceptually).
/// Shape: every two consecutive entries are `(label, pattern)` until
/// we run out; remainder is argv-to-append.
///
/// Returns the variant label and (when matched) caches it.
pub fn _pick_variant(
    state: &MainCompleteState,
    labels: &[(String, String)],
    probe_argv: &[String],
    opts: &PickVariantOpts<'_>,
) -> PickVariantResult {
    let cmd = opts.effective_cmd().to_string();

    // shell:17-21: builtin-precommand fast path. When forced builtin
    // AND -b given AND (precommand `builtin` OR the cmd IS a builtin),
    // return the -b label without exec.
    if let Some(b) = opts.builtin_label {
        if opts.forced_builtin || opts.cmd_is_builtin {
            return PickVariantResult::Builtin(b.to_string());
        }
    }

    // shell:31-34: cache hit when NOT in `builtin` precommand mode.
    if !opts.forced_builtin {
        if let Ok(g) = cmd_variant_cache().lock() {
            if let Some(cached) = g.get(&cmd) {
                // shell:33 `[[ $_cmd_variant[$opts[-c]] = "$1" ]] && return 1`
                // returns Unmatched(first) when the cached value
                // matches the first label literally. Either way the
                // cached value IS the variant — the return-code split
                // is for shell-side branch logic. Model: report the
                // cached value as Matched and let the caller decide.
                return PickVariantResult::Matched(cached.clone());
            }
        }
    }

    // Determine command-prefix for the probe call (shell:17-29).
    // We don't dispatch to `command foo` / `builtin foo` here — at
    // the Rust layer we just exec the program directly. The shell
    // version's prefix is only relevant for shell-builtin lookups,
    // which we already short-circuited above.

    // shell:36: probe via `_call_program variant $pre $opts[-c]
    //              "${@[2,-1]}" </dev/null 2>&1`
    // Build argv = [cmd, *probe_argv].
    let mut argv = vec![cmd.clone()];
    argv.extend(probe_argv.iter().cloned());
    let res: Option<CallProgramResult> = _call_program(
        state,
        "variant",
        &argv,
        &CallProgramOpts {
            skip_locale: false,
            ..Default::default()
        },
    );
    let output = res.map(|r| r.stdout).unwrap_or_default();

    // shell:38-43: walk label/pattern pairs, first match wins.
    for (label, pat) in labels {
        if simple_pattern_contains(&output, pat) {
            // Cache + return.
            if !opts.forced_builtin {
                if let Ok(mut g) = cmd_variant_cache().lock() {
                    g.insert(cmd.clone(), label.clone());
                }
            }
            return PickVariantResult::Matched(label.clone());
        }
    }

    // shell:45-47: no match — cache the FIRST label as the fallback
    // variant identifier (shell uses `$1` which is the first label
    // arg in the original positional list).
    let fallback = labels
        .first()
        .map(|(l, _)| l.clone())
        .unwrap_or_default();
    if !opts.forced_builtin {
        if let Ok(mut g) = cmd_variant_cache().lock() {
            g.insert(cmd.clone(), fallback.clone());
        }
    }
    PickVariantResult::Unmatched(fallback)
}

/// shell `[[ $output = *$~pat* ]]` — substring containment with
/// shell-glob semantics on `pat`. We support `*` / `?` only since
/// the patterns used in actual completer files are simple substrings
/// like `GNU coreutils` / `BSD ls` / `taskset \(.*\) version`.
fn simple_pattern_contains(haystack: &str, pat: &str) -> bool {
    use super::shared::glob_matches;
    // Walk every starting offset and try a glob_matches on a
    // length-bounded slice. Bounded because shell `$~pat` is anchored
    // to start-of-haystack-segment.
    if pat.is_empty() {
        return true;
    }
    // Fast path: if pattern has no glob chars, just contains().
    if !pat.contains('*') && !pat.contains('?') {
        return haystack.contains(pat);
    }
    // Slow path: brute-force scan.
    let hs: Vec<char> = haystack.chars().collect();
    let pat_with_anchors = format!("*{pat}*");
    glob_matches(&pat_with_anchors, &hs.iter().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_state() -> MainCompleteState {
        let mut s = MainCompleteState::new("", 0);
        s.ctx.context = ":complete::test:".into();
        s
    }

    #[test]
    fn builtin_label_short_circuits_when_cmd_is_builtin() {
        _reset_cache_for_tests();
        let state = mk_state();
        let opts = PickVariantOpts {
            builtin_label: Some("zsh-bin"),
            probe_cmd: Some("nonexistent-cmd"),
            default_cmd: "nonexistent-cmd",
            forced_builtin: false,
            forced_command: false,
            cmd_is_builtin: true,
        };
        let r = _pick_variant(&state, &[("gnu".into(), "GNU".into())], &[], &opts);
        assert_eq!(r, PickVariantResult::Builtin("zsh-bin".into()));
    }

    #[test]
    fn matches_pattern_in_command_output() {
        _reset_cache_for_tests();
        let state = mk_state();
        let opts = PickVariantOpts {
            builtin_label: None,
            probe_cmd: Some("echo"),
            default_cmd: "echo",
            forced_builtin: false,
            forced_command: false,
            cmd_is_builtin: false,
        };
        // echo "GNU coreutils style output" → pattern "GNU" matches.
        let r = _pick_variant(
            &state,
            &[
                ("gnu".into(), "GNU".into()),
                ("bsd".into(), "BSD".into()),
            ],
            &["GNU coreutils style output".into()],
            &opts,
        );
        assert_eq!(r, PickVariantResult::Matched("gnu".into()));
    }

    #[test]
    fn falls_back_to_first_label_when_no_pattern_matches() {
        _reset_cache_for_tests();
        let state = mk_state();
        let opts = PickVariantOpts {
            builtin_label: None,
            probe_cmd: Some("echo"),
            default_cmd: "echo",
            forced_builtin: false,
            forced_command: false,
            cmd_is_builtin: false,
        };
        let r = _pick_variant(
            &state,
            &[("gnu".into(), "WONT-MATCH".into())],
            &["irrelevant".into()],
            &opts,
        );
        assert_eq!(r, PickVariantResult::Unmatched("gnu".into()));
    }

    #[test]
    fn cache_avoids_re_exec_on_repeated_calls() {
        _reset_cache_for_tests();
        let state = mk_state();
        let opts = PickVariantOpts {
            builtin_label: None,
            probe_cmd: Some("echo"),
            default_cmd: "echo",
            forced_builtin: false,
            forced_command: false,
            cmd_is_builtin: false,
        };
        let _ = _pick_variant(
            &state,
            &[("gnu".into(), "GNU".into())],
            &["GNU x".into()],
            &opts,
        );
        // Second call with DIFFERENT probe args should still return
        // the cached value (no re-exec).
        let r2 = _pick_variant(
            &state,
            &[("bsd".into(), "BSD".into())],
            &["this would normally match bsd".into()],
            &opts,
        );
        assert_eq!(r2, PickVariantResult::Matched("gnu".into()));
    }

    #[test]
    fn forced_builtin_bypasses_cache() {
        _reset_cache_for_tests();
        let state = mk_state();
        // Seed cache for `weird-cmd` so we can prove forced_builtin
        // path takes priority over the cache.
        cmd_variant_cache()
            .lock()
            .unwrap()
            .insert("weird-cmd".into(), "should-not-win".into());
        let opts = PickVariantOpts {
            builtin_label: Some("bin-label"),
            probe_cmd: Some("weird-cmd"),
            default_cmd: "weird-cmd",
            forced_builtin: true,
            forced_command: false,
            cmd_is_builtin: false,
        };
        let r = _pick_variant(&state, &[("x".into(), "X".into())], &[], &opts);
        assert_eq!(r, PickVariantResult::Builtin("bin-label".into()));
    }

    #[test]
    fn glob_pattern_with_star_in_label_matches() {
        _reset_cache_for_tests();
        let state = mk_state();
        let opts = PickVariantOpts {
            builtin_label: None,
            probe_cmd: Some("echo"),
            default_cmd: "echo",
            forced_builtin: false,
            forced_command: false,
            cmd_is_builtin: false,
        };
        // `taskset *version` matches `taskset (util-linux) version`.
        let r = _pick_variant(
            &state,
            &[("util-linux".into(), "taskset *version".into())],
            &["taskset (util-linux) version 2.39".into()],
            &opts,
        );
        assert_eq!(r, PickVariantResult::Matched("util-linux".into()));
    }

    #[test]
    fn first_matching_label_wins_when_multiple_could_match() {
        _reset_cache_for_tests();
        let state = mk_state();
        let opts = PickVariantOpts {
            builtin_label: None,
            probe_cmd: Some("echo"),
            default_cmd: "echo",
            forced_builtin: false,
            forced_command: false,
            cmd_is_builtin: false,
        };
        // Both `GNU` and `coreutils` appear in the output; first label
        // declared wins.
        let r = _pick_variant(
            &state,
            &[
                ("gnu".into(), "GNU".into()),
                ("cu".into(), "coreutils".into()),
            ],
            &["GNU coreutils 9.0".into()],
            &opts,
        );
        assert_eq!(r, PickVariantResult::Matched("gnu".into()));
    }

    #[test]
    fn empty_labels_returns_unmatched_with_empty_fallback() {
        _reset_cache_for_tests();
        let state = mk_state();
        let opts = PickVariantOpts {
            builtin_label: None,
            probe_cmd: Some("echo"),
            default_cmd: "echo",
            forced_builtin: false,
            forced_command: false,
            cmd_is_builtin: false,
        };
        let r = _pick_variant(&state, &[], &["nothing".into()], &opts);
        assert_eq!(r, PickVariantResult::Unmatched("".into()));
    }

    #[test]
    fn simple_pattern_contains_no_glob_chars_uses_substring() {
        assert!(simple_pattern_contains("GNU coreutils 9.0", "coreutils"));
        assert!(!simple_pattern_contains("BSD ls", "GNU"));
    }

    #[test]
    fn simple_pattern_contains_empty_pattern_always_true() {
        assert!(simple_pattern_contains("anything", ""));
        assert!(simple_pattern_contains("", ""));
    }
}
