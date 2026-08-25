//! `intercept <kind> <pattern> { code }` — the block-body form.
//!
//! `}` cannot be a bare argument in zsh (`echo }` is "parse error near
//! `}'"), so the brace form documented for `intercept` is unreachable
//! through ordinary word lexing, and the body's own operators would be
//! lexed as operators of the OUTER command: in
//!
//!     intercept before git { echo hi >> ~/git.log }
//!
//! `>>` becomes a redirection OF `intercept` and never reaches argv, so
//! rejoining the words downstream cannot rebuild the body. The lexer
//! instead captures the span between the braces as raw source
//! (src/extensions/intercepts.rs::scan_block_body) and hands it over as a
//! single quoted STRING token.
//!
//! What these tests pin, in order of what would actually break:
//!   1. the body survives registration UNEXPANDED — `$INTERCEPT_ARGS`
//!      must resolve when the advice fires, not when it is registered;
//!   2. the scanner ends on the brace a reader would pick, not the first
//!      `}` it sees — quotes, comments, nesting, here-documents;
//!   3. `--zsh` reproduces `/bin/zsh` exactly, extension off;
//!   4. an unterminated body is an error, not a half-registered advice.

use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// Run `zshrs -f -c <script>` → (stdout, stderr, exit-code). `-f` skips
/// rc files so nothing in the environment can reach the result.
fn run(script: &str) -> (String, String, i32) {
    let out = Command::new(zshrs_bin())
        .args(["-f", "-c", script])
        .output()
        .expect("zshrs failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Same, with `--zsh` — the identical-behaviour drop-in, where every
/// zshrs-only syntax extension is expected to be off.
fn run_zsh_dropin(script: &str) -> (String, String, i32) {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .output()
        .expect("zshrs --zsh failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// `/bin/echo`, not the builtin: the intercept has to sit on a real
/// external command for `run_original_command` to be exercised, and the
/// absolute path keeps `$PATH` out of it.
const ECHO: &str = "/bin/echo";

#[test]
fn block_body_fires_before_the_command() {
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ echo ADVICE }}\n{ECHO} REAL"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "ADVICE\nREAL\n");
}

#[test]
fn registration_prints_nothing() {
    // Registration is not user-requested output. A .zshrc arming a few
    // intercepts used to print a banner line each, on every shell start.
    let (out, err, rc) = run(&format!("intercept before {ECHO} {{ : }}"));
    assert_eq!(rc, 0);
    assert_eq!(out, "", "registration must be silent on stdout");
    assert_eq!(err, "", "registration must be silent on stderr");
}

#[test]
fn body_is_stored_unexpanded_and_expands_at_fire_time() {
    // The regression this guards: without quote framing on the captured
    // token, the body is glob- and parameter-expanded at REGISTRATION,
    // so `$INTERCEPT_ARGS` is empty before any command is intercepted
    // and `*` explodes into a "no matches found" error.
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ echo \"args=[$INTERCEPT_ARGS] name=[$INTERCEPT_NAME]\" }}\n\
         {ECHO} one two"
    ));
    assert_eq!(rc, 0);
    assert_eq!(
        out,
        format!("args=[one two] name=[{ECHO}]\none two\n"),
        "advice parameters must resolve when the advice runs"
    );
}

#[test]
fn body_keeps_its_own_redirection() {
    // `>>` inside the body belongs to the body. If the outer command
    // lexes it, the advice silently loses the redirect — the failure
    // that made the documented one-liner unusable.
    let dir = std::env::temp_dir().join("zshrs_intercept_redirect_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    let log = dir.join("cmd.log");
    let log_s = log.display().to_string();

    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ echo \"ran $INTERCEPT_ARGS\" >> {log_s} }}\n\
         {ECHO} first\n{ECHO} second"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "first\nsecond\n", "advice output went to the file");

    let logged = std::fs::read_to_string(&log).expect("advice must have written the log");
    assert_eq!(logged, "ran first\nran second\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_substitution_in_body_runs_at_fire_time() {
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ echo \"sub=$(echo inner)\" }}\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "sub=inner\nx\n");
}

#[test]
fn close_brace_inside_single_quotes_is_not_the_terminator() {
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ echo 'a}}b' }}\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "a}b\nx\n");
}

#[test]
fn close_brace_inside_double_quotes_is_not_the_terminator() {
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ echo \"a}}b\" }}\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "a}b\nx\n");
}

#[test]
fn close_brace_inside_a_comment_is_not_the_terminator() {
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{\n  # a }} in a comment\n  echo AFTER_COMMENT\n}}\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "AFTER_COMMENT\nx\n");
}

#[test]
fn nested_braces_close_in_the_right_order() {
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ if true; then {{ echo INNER }}; fi }}\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "INNER\nx\n");
}

#[test]
fn heredoc_body_is_literal_text_including_braces() {
    // A `}` inside a here-document is content, not a terminator. Getting
    // this wrong ends the advice early and leaves the rest of the body
    // to be parsed as the outer script.
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{\ncat <<EOF\nliteral }} brace\nEOF\necho TAIL\n}}\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "literal } brace\nTAIL\nx\n");
}

#[test]
fn here_string_is_not_mistaken_for_a_heredoc() {
    // `<<<` takes a word, not a body — treating it as a here-document
    // would swallow the rest of the advice looking for a terminator.
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ cat <<< \"hs }} ok\" }}\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "hs } ok\nx\n");
}

#[test]
fn multiline_body_keeps_every_statement() {
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{\n  echo one\n  echo two\n  echo three\n}}\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "one\ntwo\nthree\nx\n");
}

#[test]
fn after_advice_sees_status_and_timing() {
    // $INTERCEPT_STATUS is the intercepted command's status. `$?` will
    // not do: the advice body's own first command overwrites it.
    let (out, _, rc) = run("intercept after /usr/bin/false { echo \"st=$INTERCEPT_STATUS\" }\n/usr/bin/false");
    assert_eq!(rc, 1);
    assert_eq!(out, "st=1\n");

    let (out, _, _) =
        run("intercept after /usr/bin/true { echo \"st=$INTERCEPT_STATUS\" }\n/usr/bin/true");
    assert_eq!(out, "st=0\n");
}

#[test]
fn around_advice_wraps_via_intercept_proceed() {
    let (out, _, rc) = run(&format!(
        "intercept around {ECHO} {{ echo PRE; intercept_proceed; echo POST }}\n{ECHO} MID"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "PRE\nMID\nPOST\n");
}

#[test]
fn quoted_form_still_registers() {
    // The pre-existing string form is what every current caller uses; the
    // lexer capture must not have displaced it.
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} 'echo QUOTED'\n{ECHO} x"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "QUOTED\nx\n");
}

#[test]
fn intercept_list_reports_the_captured_body() {
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ echo BODY_TEXT }}\nintercept list"
    ));
    assert_eq!(rc, 0);
    assert!(
        out.contains("echo BODY_TEXT"),
        "list must show the captured body, got: {out}"
    );
}

#[test]
fn a_later_brace_expansion_is_untouched() {
    // Arming is per command word and re-decided at the next one, so an
    // `intercept` earlier in the script must not capture a subsequent
    // command's brace expansion.
    let (out, _, rc) = run(&format!(
        "intercept before {ECHO} {{ : }}\nprint -r -- {{a,b}}c"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "ac bc\n", "brace expansion must still expand");
}

#[test]
fn intercept_as_an_argument_does_not_arm_the_capture() {
    let (out, _, rc) = run("print -r -- intercept; print -r -- {a,b}c");
    assert_eq!(rc, 0);
    assert_eq!(out, "intercept\nac bc\n");
}

#[test]
fn zsh_dropin_rejects_the_block_form_like_real_zsh() {
    // `--zsh` promises identical behaviour to /bin/zsh, which cannot
    // parse a bare `}`. The extension must stand down and let zsh's own
    // diagnostic through.
    let (out, err, rc) = run_zsh_dropin("intercept before git { echo hi }");
    assert_eq!(rc, 1, "must fail the way zsh fails");
    assert_eq!(out, "");
    assert!(
        err.contains("parse error near `}'"),
        "expected zsh's own diagnostic, got: {err:?}"
    );
}

#[test]
fn zsh_dropin_still_accepts_the_quoted_form() {
    // Only the SYNTAX extension is gated; the builtin itself is not.
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", &format!("intercept before {ECHO} 'echo QUOTED'\n{ECHO} x")])
        .output()
        .expect("spawn");
    assert_eq!(out.status.code().unwrap_or(-1), 0);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "QUOTED\nx\n");
}

#[test]
fn unterminated_body_is_a_parse_error_not_a_registration() {
    // Falling through on EOF used to register `{` as the advice body —
    // a silently broken intercept that fires on every matching command.
    let (out, err, rc) = run("intercept before git { echo hi");
    assert_ne!(rc, 0, "unterminated body must fail");
    assert!(err.contains("parse error"), "expected a parse error, got: {err:?}");
    assert!(
        !out.contains("intercept #"),
        "must not report a registration, got: {out:?}"
    );
}
