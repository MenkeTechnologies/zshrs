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

#[test]
fn advice_fires_when_the_command_is_a_shell_function() {
    // `intercept before git` used to fire only when `git` spawned an
    // external; a user function named `git` (a wrapper, a plugin's override)
    // bypassed it, so the log file was never created.
    let (out, _, rc) = run(
        "wrap() { print body \"$@\"; }\n\
         intercept before wrap { print \"advice[$INTERCEPT_ARGS]\" }\n\
         wrap 1 2",
    );
    assert_eq!(rc, 0);
    assert_eq!(out, "advice[1 2]\nbody 1 2\n");
}

#[test]
fn after_advice_on_a_function_sees_its_status() {
    let (out, _, rc) = run(
        "wrap() { print body; return 3; }\n\
         intercept after wrap { print \"st=$INTERCEPT_STATUS\" }\n\
         wrap; print rc=$?",
    );
    assert_eq!(rc, 0);
    assert_eq!(out, "body\nst=3\nrc=3\n");
}

#[test]
fn advice_that_runs_its_own_command_does_not_recurse() {
    // The advice calls the very function it advises; intercepts are off
    // while advice runs, so this terminates with one advice per outer call.
    let (out, _, rc) = run(
        "wrap() { print body; }\n\
         intercept before wrap { print advice; wrap }\n\
         wrap",
    );
    assert_eq!(rc, 0);
    assert_eq!(out, "advice\nbody\nbody\n");
}

#[test]
fn advice_fires_for_builtins() {
    // `echo`, `cd` and `typeset` take three different dispatch routes
    // (opcode builtin, ported builtin table); all of them honour advice.
    for (advice_on, cmd, own_output) in [("echo", "echo hi", "hi\n"), ("cd", "cd /", ""), ("typeset", "typeset zz=1", "")] {
        let (out, _, rc) = run(&format!(
            "intercept before {advice_on} {{ print -r \"advice({advice_on})\" }}\n{cmd}\nprint end"
        ));
        assert_eq!(rc, 0, "{advice_on}");
        assert_eq!(out, format!("advice({advice_on})\n{own_output}end\n"), "{advice_on}");
    }
}

#[test]
fn advice_that_calls_a_builtin_does_not_recurse() {
    let (out, _, rc) = run("intercept before echo { echo inner }\necho outer");
    assert_eq!(rc, 0);
    assert_eq!(out, "inner\nouter\n");
}

#[test]
fn around_advice_wraps_a_builtin() {
    let (out, _, rc) = run("intercept around echo { print pre; intercept_proceed; print post }\necho mid");
    assert_eq!(rc, 0);
    assert_eq!(out, "pre\nmid\npost\n");
}

/// An alias is expanded while the line is LEXED, so by the time the command
/// runs only its expansion is left. `eval` re-parses the line each time, which
/// is what makes an alias defined a line earlier take effect inside `-c`.
const ALIAS_ECHO: &str = "alias ee='/bin/echo X'\n";

#[test]
fn before_advice_fires_for_an_alias_with_the_typed_arguments() {
    let (out, _, rc) = run(&format!(
        "{ALIAS_ECHO}intercept before ee {{ print \"B[$INTERCEPT_NAME|$INTERCEPT_ARGS]\" }}\neval 'ee a b'"
    ));
    assert_eq!(rc, 0);
    // INTERCEPT_ARGS is what the user typed after the alias, not the alias body.
    assert_eq!(out, "B[ee|a b]\nX a b\n");
}

#[test]
fn after_advice_on_an_alias_runs_the_expanded_command_first() {
    let (out, _, rc) = run(&format!(
        "{ALIAS_ECHO}intercept after ee {{ print \"A[$INTERCEPT_STATUS]\" }}\neval 'ee a'"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "X a\nA[0]\n");
}

#[test]
fn around_advice_on_an_alias_proceeds_into_the_expansion() {
    let (out, _, rc) = run(&format!(
        "{ALIAS_ECHO}intercept around ee {{ print pre; intercept_proceed; print post }}\neval 'ee a'"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "pre\nX a\npost\n");
}

#[test]
fn an_alias_named_like_its_command_is_advised_once() {
    // `alias ls='ls -G'`: the alias and the command it expands to share a name.
    let (out, _, rc) = run(&format!(
        "alias {ECHO}='{ECHO} X'\nintercept before {ECHO} {{ print \"B[$INTERCEPT_ARGS]\" }}\neval '{ECHO} a'"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "B[X a]\nX a\n");
}

#[test]
fn a_plain_command_is_not_mistaken_for_an_alias() {
    // The alias marker is left by the compiler for one command only.
    let (out, _, rc) = run(&format!(
        "{ALIAS_ECHO}intercept before ee {{ print B }}\n{ECHO} plain\neval 'ee a'"
    ));
    assert_eq!(rc, 0);
    assert_eq!(out, "plain\nB\nX a\n");
}

#[test]
fn advice_fires_for_an_alias_of_a_builtin() {
    let (out, _, rc) = run(
        "alias pp='print -r --'\nintercept before pp { print \"B[$INTERCEPT_ARGS]\" }\neval 'pp hi'",
    );
    assert_eq!(rc, 0);
    assert_eq!(out, "B[hi]\nhi\n");
}

#[test]
fn advice_fires_for_an_alias_of_a_function() {
    let (out, _, rc) = run(
        "f() { print \"body $*\"; }\nalias ff='f one'\nintercept before ff { print \"B[$INTERCEPT_ARGS]\" }\neval 'ff two'",
    );
    assert_eq!(rc, 0);
    assert_eq!(out, "B[two]\nbody one two\n");
}

/// One row per behaviour: (name, script, expected stdout+stderr). Each runs in
/// its own `zshrs -f -c`, so rows cannot see each other's intercepts.
const AOP_MATRIX: &[(&str, &str, &str)] = &[
    // ── registration and management
    ("usage", "intercept", "Usage: intercept <before|after|around> <pattern> { code }\n       intercept list | remove <id> | clear\n"),
    ("list empty", "intercept list", "no intercepts registered\n"),
    ("remove", "intercept before a { : }; intercept remove 1", "removed intercept 1\n"),
    ("remove unknown id", "intercept remove 9; print rc=$?", "rc=1\nzshrs:intercept:1: no intercept with ID 9\n"),
    ("remove without id", "intercept remove; print rc=$?", "rc=1\nintercept remove: requires ID\n"),
    ("remove bad id", "intercept remove abc; print rc=$?", "rc=1\nintercept remove: invalid ID\n"),
    ("clear counts", "intercept before a { : }; intercept after b { : }; intercept clear", "cleared 2 intercepts\n"),
    ("clear disarms", "intercept before /bin/echo { print adv }; intercept clear >/dev/null; /bin/echo plain", "plain\n"),
    ("unknown subcommand", "intercept bogus; print rc=$?", "rc=1\nintercept: unknown subcommand 'bogus'. Use before|after|around|list|remove|clear\n"),
    ("missing code", "intercept before; print rc=$?", "rc=1\nintercept before: requires <pattern> { code }\n"),
    ("ids keep increasing", "intercept before a { : }; intercept before b { : }; intercept remove 1; intercept before c { : }; intercept remove 3", "removed intercept 1\nremoved intercept 3\n"),
    // ── command kinds
    ("function", "f() { print body $1 }; intercept before f { print adv }; f 1", "adv\nbody 1\n"),
    ("opcode builtin :", "intercept before : { print adv }; :", "adv\n"),
    ("opcode builtin test", "intercept before test { print adv }; test 1 -eq 1; print $?", "adv\n0\n"),
    ("opcode builtin [", "intercept before [ { print adv }; [ 1 -eq 1 ]; print $?", "adv\n0\n"),
    ("opcode builtin eval", "intercept before eval { print adv }; eval 'print inner'", "adv\ninner\n"),
    ("opcode builtin true", "intercept before true { print adv }; true; print $?", "adv\n0\n"),
    ("dynamic head", "intercept before /bin/echo { print adv }; c=/bin/echo; $c x", "adv\nx\n"),
    ("command prefix bypasses advice", "intercept before /bin/echo { print adv }; command /bin/echo x", "x\n"),
    ("builtin prefix bypasses advice", "intercept before echo { print adv }; builtin echo x", "x\n"),
    // ── patterns
    ("glob", "intercept before '/bin/e*' { print adv }; /bin/echo x", "adv\nx\n"),
    ("question mark", "intercept before '/bin/ech?' { print adv }; /bin/echo x", "adv\nx\n"),
    ("all", "intercept before all { print adv }; /bin/echo x", "adv\nx\n"),
    ("star reaches functions", "f() { print b }; intercept before '*' { print adv }; f", "adv\nadv\nb\n"),
    ("full command glob", "intercept before '/bin/echo x*' { print adv }; /bin/echo x y", "adv\nx y\n"),
    ("full command glob miss", "intercept before '/bin/echo x*' { print adv }; /bin/echo z y", "z y\n"),
    ("no match", "intercept before nope { print adv }; /bin/echo x", "x\n"),
    ("invalid pattern matches nothing", "intercept before '[invalid' { print adv }; /bin/echo x", "x\n"),
    ("underscore glob", "_f() { print b }; intercept before '_*' { print adv }; _f", "adv\nb\n"),
    // ── variables
    ("name args cmd", "intercept before /bin/echo { print \"[$INTERCEPT_NAME|$INTERCEPT_ARGS|$INTERCEPT_CMD]\" }; /bin/echo a b", "[/bin/echo|a b|/bin/echo a b]\na b\n"),
    ("variables do not outlive a before-only advice", "intercept before /bin/echo { : }; /bin/echo >/dev/null; print \"[$INTERCEPT_NAME][$INTERCEPT_ARGS][$INTERCEPT_CMD]\"", "[][][]\n"),
    ("variables do not outlive an after advice", "intercept after /bin/echo { : }; /bin/echo >/dev/null; print \"[$INTERCEPT_NAME][$INTERCEPT_STATUS]\"", "[][]\n"),
    ("timing is numeric", "intercept after /bin/echo { print \"${INTERCEPT_MS%%.*}|${INTERCEPT_US}\" | grep -Ec '^[0-9]+\\|[0-9]+$' }; /bin/echo", "\n1\n"),
    ("status of a failing function", "f() { return 3 }; intercept after f { print st=$INTERCEPT_STATUS }; f", "st=3\n"),
    // ── ordering and control
    ("befores run in registration order", "intercept before /bin/echo { print 1 }; intercept before /bin/echo { print 2 }; /bin/echo x", "1\n2\nx\n"),
    ("before then after", "intercept before /bin/echo { print B }; intercept after /bin/echo { print A }; /bin/echo x", "B\nx\nA\n"),
    ("afters run in registration order", "intercept after /bin/echo { print 1 }; intercept after /bin/echo { print 2 }; /bin/echo x", "x\n1\n2\n"),
    ("around without proceed suppresses", "intercept around /bin/echo { print around }; /bin/echo hidden", "around\n"),
    ("around with proceed", "intercept around /bin/echo { print pre; intercept_proceed; print post }; /bin/echo x", "pre\nx\npost\n"),
    ("first around wins", "intercept around /bin/echo { print A1; intercept_proceed }; intercept around /bin/echo { print A2; intercept_proceed }; /bin/echo x", "A1\nx\n"),
    ("proceed status", "f() { return 3 }; intercept around f { intercept_proceed; print post=$? }; f", "post=3\n"),
    ("after keeps the command's status", "f() { return 3 }; intercept after f { : }; f; print rc=$?", "rc=3\n"),
    ("before keeps the command's status", "intercept before /bin/echo { false }; /bin/echo x >/dev/null; print rc=$?", "rc=0\n"),
    ("failing advice does not stop the command", "intercept before /bin/echo { false; nonexistent_cmd_zz 2>/dev/null }; /bin/echo x", "x\n"),
    ("empty body", "intercept before /bin/echo { }; /bin/echo x", "x\n"),
    ("advice does not recurse into itself", "intercept before /bin/echo { /bin/echo adv }; /bin/echo x", "adv\nx\n"),
    ("exit keeps its status", "intercept before exit { print bye }; false; exit", "bye\n"),
    // ── contexts
    ("pipeline", "intercept before /bin/echo { print adv }; /bin/echo x | tr x X", "adv\nX\n"),
    ("subshell", "intercept before /bin/echo { print adv }; ( /bin/echo x )", "adv\nx\n"),
    ("command substitution", "intercept before /bin/echo { print adv }; print got:$(/bin/echo x)", "got:adv x\n"),
    ("background", "intercept before /bin/echo { print adv }; /bin/echo x & wait", "adv\nx\n"),
    ("and-list", "intercept before /bin/echo { print adv }; /bin/echo x && print y", "adv\nx\ny\n"),
    ("loop fires every iteration", "intercept before /bin/echo { print a }; for i in 1 2; do /bin/echo x; done", "a\nx\na\nx\n"),
    ("inside a function", "g() { /bin/echo x }; intercept before /bin/echo { print adv }; g", "adv\nx\n"),
    ("command not found is advised first", "intercept before nonexistent_zz { print adv }; nonexistent_zz 2>&1 | head -1", "adv\n"),
];

#[test]
fn aop_matrix() {
    let mut failures = Vec::new();
    for (name, script, want) in AOP_MATRIX {
        let (out, err, _) = run(script);
        let got = format!("{out}{err}");
        if got != *want {
            failures.push(format!("{name}\n  script: {script}\n  want:   {want:?}\n  got:    {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{} AOP rows failed:\n{}", failures.len(), failures.join("\n"));
}
