//! End-to-end contract pins for the `ai` builtin (`src/extensions/ai.rs`),
//! driven through the real shell binary with mock responses.
//!
//! Modelled on stryke's mock-driven AI suite
//! (`strykelang/tests/suite/ai_extract_contract.rs`), which installs a
//! regex mock and asserts on the call's RESULT rather than on the
//! provider wire. Same idea here, one layer out: the unit tests inside
//! `src/extensions/ai.rs` cover the engine (pricing, SSE decoding, cache
//! keys, flag parsing), so these cover the half that only exists once
//! the builtin is wired into a shell — argv reaching `run`, the result
//! reaching a parameter, exit status reaching `$?`, and errors reaching
//! stderr rather than stdout.
//!
//! # Every test is offline, and structurally cannot not be
//!
//! `ZSHRS_AI_MODE=mock-only` is set on EVERY spawn. A prompt that
//! matches no mock then fails with exit 1 instead of reaching a
//! provider, so a regression that breaks mock matching turns into a red
//! test rather than a live API call from CI — one that would need a key
//! it does not have, and would cost money if it had one.
//!
//! Mocks are per-process, and each helper spawns a fresh `zshrs -f -c`,
//! so a mock installed by one test cannot leak into another. That is
//! stronger isolation than stryke gets: its registry is in-process
//! global, which is why its tests open with `ai_mock_clear()`.

use std::io::Write;
use std::process::{Command, Stdio};

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// Run `script` under `zshrs -f -c` in mock-only mode → (stdout, stderr,
/// exit code). `-f` skips rc files so nothing in the developer's
/// environment can reach the result.
fn run(script: &str) -> (String, String, i32) {
    run_with_args(&[], script, None)
}

/// As [`run`], with extra leading shell arguments (`--zsh`) and optional
/// stdin.
fn run_with_args(args: &[&str], script: &str, stdin: Option<&str>) -> (String, String, i32) {
    let mut cmd = Command::new(zshrs_bin());
    cmd.args(args)
        .args(["-f", "-c", script])
        .env("ZSHRS_AI_MODE", "mock-only")
        // A key must not be needed, and must not be reachable: if a
        // regression ever routed one of these prompts at a provider, an
        // inherited key would let it succeed silently.
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("GOOGLE_API_KEY")
        .env_remove("GEMINI_API_KEY")
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("zshrs failed to spawn");
    if let Some(input) = stdin {
        child
            .stdin
            .as_mut()
            .expect("stdin was piped")
            .write_all(input.as_bytes())
            .expect("write stdin");
    }
    let out = child.wait_with_output().expect("zshrs failed to run");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

// ── The reason this is a builtin ──────────────────────────────────────

#[test]
fn dash_v_assigns_the_response_to_a_scalar_with_no_fork() {
    // The whole justification for `ai` being a builtin: the request and
    // the assignment happen in one process. If this ever regresses to
    // requiring `$(ai ...)`, the builtin has no reason to exist.
    let (out, err, code) = run(
        r#"ai -M 'capital=Paris'; ai -v reply 'the capital of France?'; print -r -- "[$reply]""#,
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[Paris]\n");
}

#[test]
fn dash_a_splits_the_response_into_array_elements_one_per_line() {
    let (out, err, code) = run(r#"ai -M 'list=one
two
three'; ai -a items 'list them'; print -r -- "$#items:$items[1]:$items[2]:$items[3]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "3:one:two:three\n");
}

#[test]
fn dash_a_does_not_make_a_trailing_empty_element_from_a_trailing_newline() {
    // A provider response almost always ends in a newline. `str::lines`
    // is what keeps that from becoming a phantom 3rd element, and a
    // switch to `split('\n')` would silently reintroduce it.
    let (out, err, code) = run(r#"ai -M 'L=x
y
'; ai -a arr L; print -r -- "$#arr""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "2\n");
}

#[test]
fn an_assigned_response_is_not_word_split_or_glob_expanded() {
    // The response is arbitrary model output and reaches the parameter
    // table through `set_scalar`/`set_array` directly, never through the
    // word-splitting path. A response of `a  b *` must survive with both
    // spaces and an unexpanded `*` — the `$(...)`-based equivalent this
    // builtin replaces would have mangled both.
    let (out, err, code) =
        run(r#"ai -M 'g=a  b *'; ai -v r g; print -r -- "[$r]"; ai -a arr g; print -r -- "$#arr""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[a  b *]\n1\n");
}

#[test]
fn a_multi_line_response_keeps_its_newlines_in_a_scalar() {
    let (out, err, code) = run(r#"ai -M 'm=l1
l2'; ai -v r m; print -r -- "[$r]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[l1\nl2]\n");
}

// ── Prompt assembly ───────────────────────────────────────────────────

#[test]
fn prompt_words_are_joined_with_a_single_space() {
    // Anchored both ends, so a change to the join (empty string, tab,
    // per-word newline) breaks the match and fails the test rather than
    // quietly sending a differently-shaped prompt to a paid API.
    let (out, err, code) =
        run(r#"ai -M '^foo bar baz$=joined'; ai -v r foo bar baz; print -r -- "[$r]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[joined]\n");
}

#[test]
fn the_prompt_comes_from_stdin_when_no_words_are_given() {
    let (out, err, code) = run_with_args(
        &[],
        r#"ai -M 'hello=HI'; ai -v r; print -r -- "[$r]""#,
        Some("hello\n"),
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[HI]\n");
}

#[test]
fn a_lone_dash_also_reads_the_prompt_from_stdin() {
    let (out, err, code) = run_with_args(
        &[],
        r#"ai -M 'hello=HI'; ai -v r -; print -r -- "[$r]""#,
        Some("hello\n"),
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[HI]\n");
}

#[test]
fn double_dash_lets_a_prompt_start_with_a_dash() {
    // Without `--`, `-m` is the model flag. After it, it is prompt text.
    let (out, err, code) = run(r#"ai -M '^-m$=dashed'; ai -v r -- -m; print -r -- "[$r]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[dashed]\n");
}

// ── Exit status and stream discipline ─────────────────────────────────

#[test]
fn a_successful_call_exits_zero_so_it_chains_with_and_and() {
    let (out, err, code) = run(r#"ai -M 'k=ok'; ai -b k >/dev/null && print -r -- rc0"#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "rc0\n");
}

#[test]
fn a_bad_flag_exits_two_and_writes_nothing_to_stdout() {
    // Status 2 is the usage arm. The stdout half matters more than it
    // looks: `x=$(ai ...)` must never capture an error message as if it
    // were the model's answer.
    let (out, err, code) = run(r#"x=$(ai -Z 2>/dev/null); print -r -- "rc=$? cap=[$x]""#);
    assert_eq!(code, 0, "the print itself succeeds; stderr: {err}");
    assert_eq!(out, "rc=2 cap=[]\n");
}

#[test]
fn errors_are_reported_on_stderr_in_zsh_format() {
    let (out, err, code) = run("ai -Z hello");
    assert_eq!(code, 2);
    assert_eq!(out, "", "nothing may reach stdout");
    assert_eq!(err.trim_end(), "zshrs: ai: bad option: -Z");
}

#[test]
fn a_failed_call_exits_one_not_two() {
    // Usage errors and call failures have to be distinguishable by
    // status alone — a script retrying on a transport failure must not
    // also retry a typo.
    let (out, err, code) = run("ai nothing matches this prompt");
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("mock-only"), "stderr: {err}");
}

#[test]
fn v_and_a_together_are_a_usage_error() {
    let (_out, err, code) = run("ai -v s -a arr hello");
    assert_eq!(code, 2);
    assert!(
        err.contains("mutually exclusive"),
        "stderr should say why: {err}"
    );
}

#[test]
fn an_unparseable_mock_pattern_is_a_usage_error_not_a_silent_no_op() {
    // A mock that fails to compile and is silently dropped turns every
    // later assertion in a test into a vacuous pass.
    let (_out, err, code) = run(r#"ai -M '[unclosed=x'"#);
    assert_eq!(code, 2);
    assert!(err.contains("bad pattern"), "stderr: {err}");
}

// ── Streaming and buffering ───────────────────────────────────────────

#[test]
fn an_unassigned_response_reaches_stdout_with_exactly_one_trailing_newline() {
    // The API never sends a trailing newline, so the builtin adds one —
    // and must not add a second when the response already ends in one.
    let (out, err, code) = run(r#"ai -M 'n=nonl'; ai n"#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "nonl\n");

    let (out2, _err2, code2) = run(r#"ai -M 'n=withnl
'; ai n"#);
    assert_eq!(code2, 0);
    assert_eq!(out2, "withnl\n", "must not double the newline");
}

#[test]
fn buffered_and_streamed_paths_produce_identical_bytes() {
    let script = |flag: &str| format!(r#"ai -M 'q=same answer'; ai {flag} q"#);
    let (streamed, _, c1) = run(&script(""));
    let (buffered, _, c2) = run(&script("-b"));
    assert_eq!((c1, c2), (0, 0));
    assert_eq!(streamed, buffered);
    assert_eq!(streamed, "same answer\n");
}

#[test]
fn the_builtin_works_inside_command_substitution() {
    // `-v` is the reason it is a builtin, but `$(ai ...)` still has to
    // work — it is what every existing script reaches for first.
    let (out, err, code) = run(r#"ai -M 's=SUB'; v=$(ai s); print -r -- "[$v]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[SUB]\n");
}

// ── Mock registry semantics ───────────────────────────────────────────

#[test]
fn the_first_matching_mock_wins() {
    // Registration order, not specificity. Pinned because "most
    // specific wins" is the plausible alternative someone might
    // implement, and it would silently change which fixture a suite of
    // overlapping mocks resolves to.
    let (out, err, code) =
        run(r#"ai -M 'ab=FIRST'; ai -M 'a=SECOND'; ai -v r abc; print -r -- "[$r]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[FIRST]\n");
}

#[test]
fn a_mock_spec_splits_on_the_first_equals_so_the_response_may_contain_one() {
    // `split_once('=')`: the pattern may not contain `=`, the response
    // may contain any number. That is the right way round — regexes
    // rarely need `=`, and responses (JSON, config, code) routinely do.
    let (out, err, code) = run(r#"ai -M 'a=b=c'; ai -v r xax; print -r -- "[$r]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[b=c]\n");
}

#[test]
fn a_mock_is_matched_as_a_regex_not_a_literal() {
    let (out, err, code) =
        run(r#"ai -M '^[0-9]+ items?$=numeric'; ai -v r 42 items; print -r -- "[$r]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[numeric]\n");
}

#[test]
fn a_mock_answer_is_free_and_is_not_recorded_as_a_cache_hit() {
    // Ordering contract, ported from stryke's `ai_prompt`: the mock
    // check runs BEFORE the cache and before any billing. A mocked run
    // must therefore leave every counter at zero — which is also why the
    // cache itself is unreachable from this file and is covered by the
    // unit tests in `src/extensions/ai.rs` instead.
    let (out, err, code) = run(r#"ai -M 'c=C'; ai -b c >/dev/null; ai -b c >/dev/null; ai -c"#);
    assert_eq!(code, 0, "stderr: {err}");
    for line in [
        "usd=0.000000",
        "input_tokens=0",
        "output_tokens=0",
        "cache_hits=0",
        "cache_misses=0",
    ] {
        assert!(out.contains(line), "expected {line:?} in:\n{out}");
    }
}

// ── Session configuration and reports ─────────────────────────────────

#[test]
fn dash_s_changes_a_default_for_the_rest_of_the_session() {
    let (out, err, code) = run(
        r#"ai -S model=claude-haiku-4-5 -S max_tokens=7; ai -G | grep -E '^(model|max_tokens)='"#,
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "model=claude-haiku-4-5\nmax_tokens=7\n");
}

#[test]
fn dash_s_rejects_an_unknown_key_rather_than_dropping_it() {
    // A typo'd `-S modle=...` that no-ops would leave the caller
    // believing a default changed when it had not.
    let (_out, err, code) = run("ai -S modle=claude-opus-5");
    assert_eq!(code, 2);
    assert!(err.contains("unknown config key"), "stderr: {err}");
}

#[test]
fn the_default_model_is_the_current_opus() {
    // Pins the divergence from the stryke source, whose default is still
    // `claude-opus-4-5` (`strykelang/strykelang/ai.rs:66`). A silent
    // revert to a superseded model id is exactly the kind of drift this
    // catches.
    let (out, err, code) = run("ai -G");
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        out.contains("model=claude-opus-5\n"),
        "unexpected config:\n{out}"
    );
    assert!(out.contains("provider=anthropic\n"), "config:\n{out}");
    // The key itself is never stored — only the name of the variable to
    // read it from. `ai -G` output must stay safe to paste in a bug
    // report.
    assert!(
        out.contains("api_key_env=ANTHROPIC_API_KEY\n"),
        "config:\n{out}"
    );
    assert!(!out.contains("sk-ant"), "-G must never print a key:\n{out}");
}

#[test]
fn history_records_one_tab_separated_row_per_call() {
    // Tab-separated so the report is `cut`-able and `read -A`-able,
    // which is the shell-native stand-in for stryke's `ai_history`
    // returning an arrayref of hashrefs.
    let (out, err, code) =
        run(r#"ai -S model=claude-haiku-4-5; ai -M 'h=H'; ai -b h >/dev/null; ai -H"#);
    assert_eq!(code, 0, "stderr: {err}");
    let row: Vec<&str> = out.trim_end().split('\t').collect();
    assert_eq!(row.len(), 6, "row: {out:?}");
    assert_eq!(row[0], "anthropic");
    assert_eq!(row[1], "claude-haiku-4-5");
    assert_eq!(row[4], "miss");
    assert_eq!(row[5], "h", "the prompt is the last column");
}

#[test]
fn a_multi_line_prompt_cannot_forge_extra_history_rows_or_columns() {
    // The prompt is attacker-ish text as far as this report is
    // concerned — it is whatever the user piped in. Tabs and newlines in
    // it are flattened, so one call stays one row with six columns.
    let (out, err, code) = run_with_args(
        &[],
        r#"ai -M 'x=X'; ai -b - >/dev/null; ai -H"#,
        Some("x\tforged\nsecond row\n"),
    );
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out.lines().count(), 1, "one call is one row:\n{out}");
    assert_eq!(out.trim_end().split('\t').count(), 6, "row: {out:?}");
}

#[test]
fn help_lists_every_flag_the_completion_offers() {
    // `completions/_ai` says it mirrors this table. Drift between the
    // two is invisible until someone tab-completes a flag that no longer
    // exists, so pin the flags themselves.
    let (out, _err, code) = run("ai -h");
    assert_eq!(code, 0);
    for flag in [
        "-m", "-s", "-P", "-t", "-T", "-o", "-n", "-k", "-b", "-v", "-a", "-c", "-K", "-H", "-G",
        "-S", "-M",
    ] {
        assert!(out.contains(flag), "usage text is missing {flag}:\n{out}");
    }
}

// ── Dispatch wiring ───────────────────────────────────────────────────

#[test]
fn ai_resolves_as_a_builtin_not_an_external_command() {
    // Guards the `LOCAL_ONLY_BUILTINS` entry. The pinned fusevm release
    // does not know the name yet, so without that entry `whence -w`
    // reports `none`/`command` while calling `ai` still runs the
    // builtin — the shell running its own implementation while every
    // tool you could ask about it says otherwise.
    let (out, err, code) = run("whence -w ai; print -r -- ${+builtins[ai]}");
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "ai: builtin\n1\n");
}

#[test]
fn ai_dispatches_through_a_run_time_resolved_head() {
    // `builtin ai` and `$var` indirection both reach
    // `try_run_registered_builtin` rather than the compiled-literal
    // path, and must land on the same implementation.
    let (out, err, code) = run(r#"builtin ai -M 'z=B'; c=ai; $c -v r z; print -r -- "[$r]""#);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "[B]\n");
}

#[test]
fn zsh_emulation_hides_the_name_but_still_dispatches_it() {
    // `--zsh` must not invent a builtin real zsh has never had, so the
    // name leaves `$builtins`. Dispatch is deliberately untouched: the
    // flag is about the emulated NAMESPACE, not about disabling the
    // shell's own features.
    let (hidden, _err, c1) = run_with_args(&["--zsh"], "print -r -- ${+builtins[ai]}", None);
    assert_eq!(c1, 0);
    assert_eq!(hidden, "0\n");

    let (out, err, c2) = run_with_args(
        &["--zsh"],
        r#"ai -M 'z=Z'; ai -v r z; print -r -- "[$r]""#,
        None,
    );
    assert_eq!(c2, 0, "stderr: {err}");
    assert_eq!(out, "[Z]\n");
}
