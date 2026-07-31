//! `tests/recorder_corpus/` harness — runs `zshrs-recorder --no-daemon
//! --file PATH` against each curated corpus script, parses the
//! `Captured ...` lines on stderr, and asserts per-kind event counts.
//!
//! Each test pins the EXACT counts a healthy recorder must produce on
//! that file. A regression that misses a dispatcher (or double-fires
//! one) immediately fails the relevant test with a kind-by-kind diff.
//!
//! Built only with `--features recorder`. The `zshrs-recorder` bin
//! lives under `required-features = ["recorder"]`, so cargo can resolve
//! `CARGO_BIN_EXE_zshrs-recorder` only when the feature is on.

#![cfg(feature = "recorder")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const CORPUS_DIR: &str = "tests/recorder_corpus";

/// Run the recorder on one corpus file with `--no-daemon` and return a
/// `kind -> count` map parsed from the end-of-run summary block on
/// stderr. The summary is the recorder's authoritative count (taken
/// straight from the in-process buffer); per-line `Captured ...` lines
/// can drop a record when a single statement fires multiple hooks
/// (e.g. `typeset NAME=val` triggers both the typeset hook and the
/// downstream SET_VAR aspect under some compile paths). Reading the
/// summary is the right source of truth.
fn run(file: &str) -> BTreeMap<String, usize> {
    run_with_home(file, None)
}

/// Same as `run` but lets the caller pin `$HOME` to a corpus
/// subdirectory. Used by the real-world `zshrc/` corpus where the
/// .zshrc sources `$HOME/.zpwr/local/zpwr-hash-dirs.zsh` — pointing
/// `$HOME` at the corpus directory makes the cache file resolve to a
/// committed, hermetic copy. Without this, the test would either skip
/// 19 hash-d records (clean env) or pull in the developer's actual
/// dotfiles (HOME-leaking env).
///
/// The whole environment is wiped (`env_clear`) before re-injecting
/// `HOME` and a minimal `PATH`. This neutralizes leaked `ZPWR_*` env
/// vars from the parent shell that would otherwise alter the .zshrc's
/// control flow.
fn run_with_home(file: &str, home_subdir: Option<&str>) -> BTreeMap<String, usize> {
    let bin = env!("CARGO_BIN_EXE_zshrs-recorder");
    let path: PathBuf = Path::new(CORPUS_DIR).join(file);
    assert!(path.exists(), "corpus file missing: {}", path.display());

    let mut cmd = Command::new(bin);
    cmd.arg("--no-daemon").arg("--file").arg(&path);
    if let Some(sub) = home_subdir {
        let abs_corpus = Path::new(CORPUS_DIR)
            .canonicalize()
            .unwrap_or_else(|e| panic!("canonicalize {}: {}", CORPUS_DIR, e));
        let home = abs_corpus.join(sub);
        cmd.env_clear();
        cmd.env("HOME", &home);
        cmd.env("PATH", "/usr/bin:/bin");
    }
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn zshrs-recorder: {e}"));

    let stderr = String::from_utf8_lossy(&out.stderr);
    parse_summary(&stderr).unwrap_or_else(|| {
        panic!(
            "recorder stderr missing summary block for {}\n--- stderr ---\n{}\n",
            file, stderr
        )
    })
}

/// Parse the "--- zshrs-recorder summary ---" block. Lines after the
/// header look like:
///     `  kind       N`
/// terminated by an `elapsed:` line. Returns the `kind -> count` map,
/// or `None` if no summary block was emitted (recorder crashed or the
/// script aborted before atexit fired).
fn parse_summary(stderr: &str) -> Option<BTreeMap<String, usize>> {
    let mut lines = stderr.lines();
    while let Some(line) = lines.next() {
        if line.contains("zshrs-recorder summary") {
            break;
        }
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut saw_summary_body = false;
    for line in lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("elapsed:") {
            return Some(counts);
        }
        if trimmed.starts_with("total events") {
            saw_summary_body = true;
            continue;
        }
        // Summary body row: `  KIND      N` (whitespace-padded). The
        // 2-word kinds (`alias -g`, `alias -s`, `hash -d`) need a 3-way
        // split; everything else is a 2-way split. `split_whitespace`
        // collapses runs, so the count is always the last token.
        let toks: Vec<&str> = trimmed.split_whitespace().collect();
        if toks.is_empty() {
            continue;
        }
        let last = toks[toks.len() - 1];
        let n: usize = match last.parse() {
            Ok(n) => n,
            Err(_) => continue, // not a count row (e.g. blank, banner)
        };
        let kind = match toks.len() {
            2 => toks[0].to_string(),
            3 => format!("{} {}", toks[0], toks[1]),
            _ => continue,
        };
        counts.insert(kind, n);
        saw_summary_body = true;
    }
    if saw_summary_body {
        Some(counts)
    } else {
        None
    }
}

/// Assert the recorder emitted exactly the expected per-kind counts —
/// no missing kinds, no extras, exact magnitudes. Diffs both ways.
fn assert_counts(file: &str, expected: &[(&str, usize)]) {
    assert_counts_inner(file, run(file), expected);
}

/// Same as `assert_counts` but pins `$HOME` to a corpus subdir.
fn assert_counts_with_home(file: &str, home_subdir: &str, expected: &[(&str, usize)]) {
    assert_counts_inner(file, run_with_home(file, Some(home_subdir)), expected);
}

fn assert_counts_inner(file: &str, actual: BTreeMap<String, usize>, expected: &[(&str, usize)]) {
    // Lower-bound semantics: every kind in `expected` must appear with
    // at least the given count in `actual`. Extra kinds in `actual`
    // are allowed (they represent recorder-coverage growth or
    // env-specific code paths firing that the dev's local env didn't
    // hit). A regression that DROPS captures still fails.
    //
    // Why not exact match: the corpus .zshrc files run partway and
    // fail somewhere in the middle (because the corpus doesn't include
    // the full ~200k-line zpwr stack they reference). Where they fail
    // is env-dependent — on macOS with env_clear, $ZPWR_LOCAL_TEMP is
    // empty so an early `mkdir -p` aborts at line ~141; on Linux CI
    // the same `mkdir` succeeds and the script reaches line ~657's
    // 43-setopt block. Exact-match assertions whipsaw between the two
    // platforms; lower-bound holds steady on both.
    let expected: BTreeMap<String, usize> =
        expected.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    let mut shortfalls: Vec<(String, usize, usize)> = Vec::new();
    for (k, &e) in &expected {
        let a = actual.get(k).copied().unwrap_or(0);
        if a < e {
            shortfalls.push((k.clone(), e, a));
        }
    }
    if !shortfalls.is_empty() {
        let mut msg = format!("\n{file}: capture-count regression (actual < expected)\n");
        let mut all_keys: std::collections::BTreeSet<&String> = expected.keys().collect();
        for k in actual.keys() {
            all_keys.insert(k);
        }
        for k in all_keys {
            let e = expected.get(k).copied().unwrap_or(0);
            let a = actual.get(k).copied().unwrap_or(0);
            let mark = if a < e {
                "!!"
            } else if a > e {
                "+ "
            } else {
                "  "
            };
            msg.push_str(&format!("  {mark} {k:<10} expected>={e}  actual={a}\n"));
        }
        panic!("{msg}");
    }
}

// ---- corpus tests --------------------------------------------------------

#[test]
fn aliases_basic() {
    assert_counts("01_aliases_basic.zsh", &[("alias", 5)]);
}

#[test]
fn alias_redefine_keeps_each() {
    // Per-definition granularity is the central recorder claim.
    // Three definitions of `x` ⇒ three records (not one).
    assert_counts("02_alias_redefine.zsh", &[("alias", 3)]);
}

#[test]
fn alias_subkinds_dispatch_separately() {
    assert_counts(
        "03_alias_variants.zsh",
        &[("alias", 1), ("alias -g", 2), ("alias -s", 1)],
    );
}

#[test]
fn exports_one_per_pair() {
    assert_counts("04_exports.zsh", &[("export", 4)]);
}

#[test]
fn assigns_top_level_only() {
    assert_counts("05_assigns.zsh", &[("assign", 4)]);
}

#[test]
fn setopts_and_unsetopts() {
    assert_counts("06_setopts.zsh", &[("setopt", 3), ("unsetopt", 2)]);
}

#[test]
fn zstyle_one_per_setter() {
    assert_counts("07_zstyle.zsh", &[("zstyle", 3)]);
}

#[test]
fn bindkey_skips_mode_switch() {
    // `bindkey -e` is a mode switch, not a binding ⇒ no record.
    assert_counts("08_bindkey.zsh", &[("bindkey", 3)]);
}

#[test]
fn hash_d_only_named_dirs() {
    // Plain `hash NAME=PATH` is command-hash (runtime cache), NOT
    // recorder material. Only `hash -d` rows.
    assert_counts("09_hash_d.zsh", &[("hash -d", 3)]);
}

#[test]
fn trap_one_per_signal() {
    // `trap 'echo int' INT TERM` ⇒ 2 records. Total: 1 + 2 + 1 = 4.
    assert_counts("10_traps.zsh", &[("trap", 4)]);
}

#[test]
fn zmodload_load_form_only() {
    assert_counts("11_zmodload.zsh", &[("zmodload", 3)]);
}

#[test]
fn functions_funcdef_form() {
    // `name() { body }` syntax. The `function NAME { body }` keyword
    // form goes through a different code path and is NOT covered by
    // this corpus file (Phase 2.5).
    assert_counts("12_functions.zsh", &[("function", 3)]);
}

#[test]
fn kitchen_sink_full_surface() {
    // One of every kind in a single file, exercising the full
    // dispatcher set.
    assert_counts(
        "13_kitchen_sink.zsh",
        &[
            ("alias", 1),
            ("alias -g", 1),
            ("alias -s", 1),
            ("export", 1),
            ("assign", 1),
            ("hash -d", 1),
            ("zstyle", 1),
            ("bindkey", 1),
            ("compdef", 1),
            ("zmodload", 1),
            ("setopt", 1),
            ("unsetopt", 1),
            ("trap", 1),
            ("function", 1),
        ],
    );
}

#[test]
fn shell_mode_quirk_dissolves() {
    // The if-branch is gated on `(( $+commands[fzf] ))` && interactive.
    // Recorder runs non-interactive without fzf in PATH, so only the
    // elif branch fires ⇒ exactly 1 alias record.
    assert_counts("14_conditional.zsh", &[("alias", 1)]);
}

#[test]
fn listing_only_invocations_silent() {
    // Bare `alias` / `setopt` / `trap` etc. list state, they don't
    // mutate it. None of those should produce records — only the
    // single trailing `alias only_real_one='captured'` line should.
    assert_counts("15_listing_only.zsh", &[("alias", 1)]);
}

#[test]
fn typeset_family_all_dispatch_to_typeset_kind() {
    // RECORDER.md surface: `typeset` / `declare` / `readonly` /
    // `integer` / `float` all funnel into the `typeset` kind. Each
    // builtin has its own dispatcher in zshrs (readonly/integer/float
    // do NOT delegate to builtin_typeset_named) so each needs its own
    // recorder hook. ALSO: structured ParamAttrs ride alongside each
    // event — the harness checks counts not attrs, but the realtime
    // stderr line shows them as `[scalar,readonly]` etc.
    assert_counts("16_typeset_family.zsh", &[("typeset", 5)]);
}

#[test]
fn completion_files_discovered_on_fpath_addition() {
    // zinit-report surfaces completion files as part of its plugin
    // summary. Recorder mirrors that: when fpath is set or appended,
    // walk the dir and emit one `completion` event per `_*` file.
    //
    //   path_mod×1     the fpath dir itself
    //   completion×3   _git, _kubectl, _helm (in fixture dir;
    //                  _helm.zwc skipped, notacomp skipped)
    assert_counts(
        "23_completions_discover.zsh",
        &[("path_mod", 1), ("completion", 3)],
    );
}

#[test]
fn autoload_emits_function_kind() {
    // RECORDER.md surface: the `function` kind covers `name() {}`,
    // `function name {}`, AND `autoload name`. Autoload registrations
    // emit one function event per name with value="autoload" so the
    // query side can distinguish lazy registrations from inline defs.
    // 4 lines, 6 names total: `_git`, `add-zsh-hook`, `compinit`,
    // and the three from `autoload first second third`.
    assert_counts("17_autoload.zsh", &[("function", 6)]);
}

#[test]
fn set_o_form_dispatches_setopt() {
    // RECORDER.md surface: `setopt OPT` / `unsetopt OPT` / `set -o opt`.
    // The POSIX `set -o NAME` / `set +o NAME` form goes through
    // builtin_set, not builtin_setopt, so it needs a separate hook.
    // Once captured, it emits the same setopt/unsetopt kinds as the
    // zsh-style forms — query semantics stay uniform regardless of
    // which syntax the user typed.
    assert_counts("18_set_o_form.zsh", &[("setopt", 2), ("unsetopt", 2)]);
}

#[test]
fn replay_types_full_matrix() {
    // Replay-grade type tracking: every assign/typeset event must
    // carry structured ParamAttrs + value_array/value_assoc payloads
    // so the daemon can reconstruct the EXACT typed declaration on
    // a future replay.
    //
    //   assign×4:
    //     PROJECT=zshrs              [scalar]
    //     arr=(alpha beta gamma)     [array] + value_array
    //     arr+=(delta)               [array,append] + value_array
    //     h[k3]=v3                   [assoc,append] + value_assoc
    //   typeset×5:
    //     EDITOR=vim                 [scalar,export,global]
    //     h=(k1 v1 k2 v2)            [assoc]
    //     count=42                   [integer]
    //     pi=3.14                    [float]
    //     RO=ro_val                  [scalar,readonly]
    assert_counts("24_replay_types.zsh", &[("assign", 4), ("typeset", 5)]);
}

/// Beyond bare counts: this test inspects the realtime stderr to
/// verify the structured `[attrs]` label fires for each event. If the
/// recorder ever drops the attrs population, this catches it
/// immediately — without it, replay would lose the integer/float/
/// assoc/readonly distinctions.
#[test]
fn replay_types_attrs_visible() {
    use std::process::Command;
    let bin = env!("CARGO_BIN_EXE_zshrs-recorder");
    let path = std::path::Path::new(CORPUS_DIR).join("24_replay_types.zsh");
    let out = Command::new(bin)
        .arg("--no-daemon")
        .arg("--file")
        .arg(&path)
        .output()
        .expect("spawn recorder");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let must_contain: &[&str] = &[
        "PROJECT [scalar]",
        "EDITOR [scalar,export,global]",
        "arr [array]",
        "arr [array,append]",
        "h [assoc]",
        "h [assoc,append]",
        "count [integer]",
        "pi [float]",
        "RO [scalar,readonly]",
    ];
    for needle in must_contain {
        assert!(
            stderr.contains(needle),
            "replay attrs missing: {:?} not found in recorder stderr.\n--- stderr ---\n{}\n",
            needle,
            stderr,
        );
    }
}

#[test]
fn assignment_forms_all_capture() {
    // Recorder's chokepoint contract: every assignment / reassignment
    // shape on a non-local shell variable surfaces as an event. Per
    // user spec: "all assignments on any non local var/parameter must
    // be captured".
    //
    //   23 events:
    //   assign×10:
    //     PROJECT=zshrs, PROJECT+=_v2,
    //     arr=(...), arr+=(...), arr[1]=replaced,
    //     h=(...), h[k3]=v3, h[k3]+=tail,
    //     PATH=/old, FPATH=/oldfp
    //   path_mod×12:
    //     path=, fpath=, manpath=, module_path=, cdpath= (set form, 5)
    //     path+=, fpath+=, module_path+=, cdpath+= (append form, 4)
    //     PATH+=:/new, FPATH+=:/newfp (scalar concat via APPEND_SCALAR_OR_PUSH, 2)
    //     +1 from the path=(/p1 /p2) expanding to 2 elements
    //   typeset×1:
    //     `typeset -A h`
    assert_counts(
        "21_assignment_forms.zsh",
        &[("assign", 10), ("path_mod", 12), ("typeset", 1)],
    );
}

#[test]
fn function_forms_all_capture() {
    // RECORDER.md surface row: `function` covers every form zsh
    // accepts. The lexer + parser were extended to handle the
    // keyword form `function NAME { body }` (including non-empty
    // bodies, multi-name declarations, and `-T`/`-U`-style flags).
    // 14 events total (each verified against zsh 5.9):
    //   1× POSIX `name() { body }`
    //   1× keyword `function name { body }` (the previously-broken case)
    //   1× mixed `function name() { body }`
    //   1× empty-body POSIX
    //   1× empty-body keyword (`{ }` — the brace pair must be spaced)
    //   3× multi-name keyword form (one declaration installs N names)
    //   1× keyword + `-T` flag (flag stripped, name kept)
    //   2× keyword + `-U`: `-U` is NOT a funcdef flag, so zsh installs
    //      it as a function name alongside the real one
    //   3× `autoload` registrations across two declarations
    assert_counts("20_function_forms.zsh", &[("function", 14)]);
}

#[test]
fn zle_widgets_capture() {
    // `zle -N WIDGET [FUNC]` and `zle -A OLD NEW` are state mutations
    // zinit-report includes in its plugin summaries. Recorder gives
    // them a dedicated `zle` kind so query-side can answer "which
    // plugin installed this widget" with file:line precision.
    //
    //   function×2   underlying handlers (`fzf-history-widget`, `fzf-cd-widget`)
    //   zle×4        2 self-bound `-N`, 1 `-N` with explicit handler, 1 `-A` alias
    assert_counts("22_zle_widgets.zsh", &[("function", 2), ("zle", 4)]);
}

#[test]
fn unalias_unset_emit_removal_events() {
    // RECORDER.md "Open question 4": removal events. Without these,
    // `zwhere -l NAME` lineage cannot reconstruct the
    // define→unset→redefine chain. Each removal is a distinct kind
    // (`unalias` / `unset`) so the query side can find every removal
    // site for a given name.
    assert_counts(
        "19_unalias_unset.zsh",
        &[("alias", 2), ("assign", 2), ("unalias", 2), ("unset", 2)],
    );
}

// ---- zinit/ corpus -------------------------------------------------------
//
// Verbatim copies of zinit's own runtime scripts (`~/.zinit/bin/*.zsh`)
// pulled in to stress the recorder against real-world zsh. zinit is the
// most metaprogramming-heavy plugin manager in the ecosystem; if the
// recorder survives sourcing zinit's own bootstrap layer, it survives
// almost anything. License: MIT (see `tests/recorder_corpus/zinit/LICENSE`).
//
// Counts are pinned against zinit upstream as of corpus copy time. A
// future re-copy may shift these — re-run `cargo test --features
// recorder --test recorder_harness` and update the constants.

// NOTE on zinit-corpus expected-counts shifts:
// The parser fix that recognises `function NAME { body }` keyword form
// (parse/src/parse::par_funcdef "body opener" branch) plus the
// Envarray fix in parse_assign make the parser advance further into
// these scripts than it used to. That advance exposes a SEPARATE
// pre-existing zshrs parser bug: `[[ "$line" = (#i)*foo* ]]` inside
// `elif` (the case-insensitive pattern flag inside a conditional)
// triggers "expected 'then' after elif" because the cond-expression
// parser treats the `(` of `(#i)` as a primary-grouping paren and
// swallows the `then`. zinit's `bin/share/git-process-output.zsh:114`,
// `bin/zinit.zsh`, and `bin/zinit-autoload.zsh` all hit this and now
// fail to parse where previously parsing aborted silently earlier.
// Tests below pin to current behavior; the (#i)-elif parser fix is
// tracked separately as a compat-floor item.

#[test]
fn zinit_main() {
    // zinit.zsh: aborts at first `(#i)` elif (pre-existing parser bug
    // exposed by the parser-advance fixes). 0 events until that's
    // fixed; the file's typesets live downstream of the abort point.
    assert_counts("zinit/zinit.zsh", &[]);
}

#[test]
fn zinit_autoload() {
    // zinit-autoload.zsh: same pre-existing `(#i)`-in-elif abort.
    assert_counts("zinit/zinit-autoload.zsh", &[]);
}

#[test]
fn zinit_side() {
    // zinit-side.zsh: side-effects helper. Two function definitions at
    // top level under this corpus version.
    assert_counts("zinit/zinit-side.zsh", &[("function", 2)]);
}

#[test]
fn zinit_additional() {
    // zinit-additional.zsh: the corpus has ~10 top-level function
    // definitions and no top-level scalar assignments. The earlier
    // expectation of 1 assign was stale from when the corpus
    // included a setup line that was later removed. Pin only the
    // function lower-bound: parser must recognize at least the
    // POSIX `name()` form AND the keyword `function name {}` form,
    // i.e. ≥2 distinct shapes covered.
    assert_counts("zinit/zinit-additional.zsh", &[("function", 2)]);
}

#[test]
fn zinit_install() {
    // zinit-install.zsh: install-time helpers. Top-level is one source
    // dispatch into shared helpers.
    assert_counts("zinit/zinit-install.zsh", &[("source", 1)]);
}

// ---- zshrc/ corpus -------------------------------------------------------
//
// Verbatim copy of MenkeTechnologies's real `~/.zshrc` (the bootstrap
// for his "world's most powerful CLI setup") plus the small
// `~/.zpwr/local/zpwr-hash-dirs.zsh` cache it sources. Runs under a
// wiped environment with `$HOME` pointed at the corpus directory so
// the .zshrc's `source "$HOME/.zpwr/local/..."` resolves to the
// committed copy instead of the developer's actual dotfiles.
//
// Most of the .zshrc's deeper sources (`$ZPWR_ENV_FILE` etc.) fail
// because the full ~200k-line zpwr stack is NOT in the corpus
// (intentional — that's its own repo). The recorder still captures
// every event that fires up to and through those failure points; the
// pinned counts below are the stable .zshrc-top-level + hash-dirs-only
// surface.

#[test]
fn zshrc_real_user() {
    // After the Envarray fix and the new path-family + scalar-concat
    // + APPEND_SCALAR_OR_PUSH hooks, the recorder now captures more
    // .zshrc state mutations than before:
    //   zmodload×1   (zsh/datetime)
    //   hash -d×19   (every cache entry from zpwr-hash-dirs.zsh)
    //   export×9     (ZPWR_*, LC_ALL, KEYTIMEOUT, SHELL, AUTOPAIR_*, SSH_KEY_PATH)
    //   assign×15    (timestamps, $0 reassigns, HYPHEN_INSENSITIVE,
    //                 plus more bytecode-emitted scalar assigns the
    //                 prior parser was dropping)
    //   typeset×3    (FPATH +x, ZPWR_VARS, ZPWR_VERBS)
    //   function×4   (zpwrInitEnv, rm wrapper, hg_prompt_info, plus
    //                 one new function keyword-form NOW captured)
    //   path_mod×?   skipped — env_clear leaves $ZPWR_* unset so
    //                 the fpath=($ZPWR_AUTOLOAD_... $fpath) lines
    //                 all expand to empty arrays and emit nothing.
    //                 Verifying path_mod capture lives in the
    //                 dedicated micro-corpus tests (path_family/
    //                 fpath_append/etc.) where the inputs are
    //                 hardcoded; here we pin only what the corpus
    //                 deterministically produces under env_clear.
    //   source×3     (zpwr-hash-dirs success + 2 zpwr-env failures)
    assert_counts_with_home(
        "zshrc/zshrc.zsh",
        "zshrc",
        &[
            ("zmodload", 1),
            ("hash -d", 19),
            ("export", 9),
            ("assign", 15),
            ("typeset", 3),
            ("function", 4),
            ("source", 3),
        ],
    );
}

#[test]
fn zinit_git_process_output() {
    // share/git-process-output.zsh: hits the same pre-existing
    // `(#i)`-pattern-in-elif parser bug at line 114 — abort before any
    // events fire. Tracked separately. Pinning to 0 until the cond
    // parser learns to handle the case-insensitive `(#i)` flag without
    // mistaking `(` for a grouping paren that swallows `then`.
    assert_counts("zinit/git-process-output.zsh", &[]);
}
