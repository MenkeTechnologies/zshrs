//! Parse-error diagnostics against real zsh.
//!
//! C's parser never words its own syntax errors: every construct bails with
//! `YYERROR`/`YYERRORV` (Src/parse.c:87-88), which only sets `tok = LEXERR`,
//! and the single message is `yyerror`'s "parse error near `<zshlextext>'"
//! (Src/parse.c:2733), naming the last token the lexer read. These tests pin
//! stderr and status byte-for-byte, for the AST parser (`-c`, `eval`) and the
//! wordcode parser (`emulate -c`, autoload), which reach `yyerror` by
//! different routes.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// (stdout, stderr, status) of `script` run with `-fc` in a scratch cwd.
fn run(cmd: &mut Command, script: &str, cwd: &Path) -> (String, String, i32) {
    let out = cmd
        .arg(script)
        .current_dir(cwd)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn shell");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let dir = std::env::temp_dir().join(format!(
        "zshrs-parse-error-{}-{:x}",
        std::process::id(),
        script.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64))
    ));
    let (zdir, rdir) = (dir.join("zsh"), dir.join("zshrs"));
    std::fs::create_dir_all(&zdir).unwrap();
    std::fs::create_dir_all(&rdir).unwrap();
    let z = run(Command::new(zsh_path()).arg("-fc"), script, &zdir);
    let r = run(Command::new(zshrs_bin()).args(["--zsh", "-f", "-c"]), script, &rdir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(z, r, "divergence on script:\n{script}\n(stdout, stderr, status)");
}

/// par_case (c:1209): every YYERRORV names the last token read.
#[test]
fn case_errors_name_the_last_token() {
    assert_parity("case");
    assert_parity("case x");
    assert_parity("case x foo");
    assert_parity("case x in a) echo");
    assert_parity("case x in a) echo;;");
    assert_parity("case x in a echo;; esac");
    assert_parity("case x { a) echo;;");
    assert_parity("case x in a) :;; b");
    assert_parity("case x in x) echo x; }");
    assert_parity("case x in (a|b) echo ab;; (x) echo x;; esac");
}

/// par_for / par_repeat / the shared DOLOOP-INBRACE-ZEND body ladder
/// (c:1087-1194, c:1565-1606).
#[test]
fn loop_errors_name_the_last_token() {
    assert_parity("for");
    assert_parity("for; do :; done");
    assert_parity("for a-b in x; do :; done");
    assert_parity("for x in a; do");
    assert_parity("for x in a; do echo");
    assert_parity("for ((i=0;i<1;i++)) do");
    assert_parity("select");
    assert_parity("repeat");
    assert_parity("repeat 3 do");
    assert_parity("while true; do");
    assert_parity("until true; do :");
    assert_parity("foreach x (a b) echo");
    assert_parity("for x in");
    assert_parity("for x in a b");
    assert_parity("for x (a b");
    assert_parity("for x in a b; do print $x; done");
}

/// dbparens (lex.c:631-648) leaves the partial expression in tokstr when it
/// hits end of input, so the diagnostic quotes it.
#[test]
fn unterminated_arith_for_quotes_the_partial_expression() {
    assert_parity("for ((i=0");
    assert_parity("for ((i=0;i<1");
}

/// par_if (c:1411) and par_subsh (c:1619).
#[test]
fn if_and_brace_group_errors_name_the_last_token() {
    assert_parity("if true; then");
    assert_parity("if true; then :; else");
    assert_parity("{ print");
    assert_parity("{ print; ");
    assert_parity("{ : } always");
    assert_parity("{ : } always echo");
    assert_parity("{ : } always { :");
    assert_parity("{ : } always { print ok }; print rc=$?");
}

/// c:1471-1472 — only a SEPER ends a brace-bodied `if`; end of input loops
/// back to the arm test and YYERRORs.
#[test]
fn brace_if_needs_a_separator_after_the_body() {
    assert_parity("if [[ 1 = 1 ]] { echo y }");
    assert_parity("if [[ 1 = 1 ]] { echo y };");
    assert_parity("if [[ 1 = 1 ]] { echo y } else { echo n }");
    assert_parity("{ if [[ 1 = 1 ]] { echo y } }");
}

/// par_funcdef / par_simple's INOUTPAR arm (c:1672, c:2054-2116).
#[test]
fn funcdef_errors_name_the_last_token() {
    assert_parity("f() {");
    assert_parity("f() { :");
    assert_parity("function f {");
    assert_parity("function f { :");
    assert_parity("() { :");
    assert_parity("x=1 f() { :; }");
    assert_parity("function {");
    assert_parity("f() ( :");
    assert_parity("\n\nf() if");
    assert_parity("f()");
    assert_parity("setopt nomultifuncdef; a b() { :; }");
}

/// The wordcode parser behind `emulate -c` and autoload printed its own
/// `par_case: expected scrutinee` / `missing \`done\`` text and still ran the
/// body; `parse_list` must see LEXERR and report through `yyerror`.
#[test]
fn wordcode_parser_errors_go_through_yyerror() {
    assert_parity("emulate sh -c 'case'; print rc=$?");
    assert_parity("emulate zsh -c 'for x in a; do'; print rc=$?");
    assert_parity("emulate zsh -c 'f() { :'; print rc=$?");
    assert_parity("emulate sh -c 'f() { :'; print rc=$?");
    assert_parity("emulate zsh -c '[[ a < ]]'; print rc=$?");
    assert_parity("emulate zsh -c '[[ ( a ]]'; print rc=$?");
    assert_parity("emulate zsh -c '[['; print rc=$?");
}

/// c:Src/exec.c:6329 — getfpfunc parses the file with `parse_string(d, 1)`,
/// so an error is numbered within the function file, not at the caller's
/// line (C04funcdef.ztst:319).
#[test]
fn autoload_parse_error_counts_lines_in_the_file() {
    assert_parity(
        "setopt ignorebraces\nfpath=(.)\n\n\nprint \"{ echo OK }\\n[[ -o ignorebraces ]] || print off\" >emufunctest\n(autoload -z emufunctest; emufunctest) 2>&1",
    );
}

/// c:Src/parse.c:1856-1865 — par_simple NUL-terminates an assignment's name
/// inside `tokstr`, which `zshlextext` aliases; at ENDINPUT zshlex does not
/// refresh `zshlextext` (c:Src/lex.c:276), so the error names only the
/// name. The mid-command (typeset) arm has no `+=` case, so `x+` survives.
#[test]
fn eof_error_after_assignment_names_the_truncated_name() {
    assert_parity("f() { x=1}");
    assert_parity("f() { x+=1}");
    assert_parity("f() { a[1]=2}");
    assert_parity("f() { typeset x+=1}");
    assert_parity("f() { typeset x=1}");
    assert_parity("{ x=1; y=abc");
    assert_parity("if x=1");
}
