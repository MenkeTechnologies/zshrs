//! `${(z)…}` word splitting — the shell lexer run over a string.
//!
//! Line-editor plugins parse the command line with `(z)` to find command
//! position, the word under the cursor and the statement it belongs to
//! (zsh-expand, zsh-autosuggestions strategies, fzf-tab, abbreviation
//! plugins). Each token has to come back exactly as zsh's `bufferwords()`
//! returns it: an operator that is split in two, or glued to the word
//! before it, moves every later word into the wrong statement.

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

/// Print every `(z)` word of each input, quoted, one input per line, and
/// compare the two shells' output.
fn assert_same_words(inputs: &[&str]) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let mut script = String::from("for zqin in");
    for i in inputs {
        script.push_str(&format!(" '{}'", i.replace('\'', "'\\''")));
    }
    script.push_str("; do print -r -- ${(qqq)${(z)zqin}}; done");
    let z = Command::new(zsh_path())
        .args(["-fc", &script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", &script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let z_out = String::from_utf8_lossy(&z.stdout).into_owned();
    let r_out = String::from_utf8_lossy(&r.stdout).into_owned();
    assert_eq!(
        z_out, r_out,
        "(z) divergence\n--- script ---\n{script}\n--- zsh ---\n{z_out}\n--- zshrs ---\n{r_out}"
    );
}

/// Redirection operators are tokens of their own (c:Src/lex.c:826-913),
/// printed as their `tokstrings[]` text, with a leading fd digit attached
/// (c:Src/hist.c:3563-3566). zshrs had no redirection arm at all: `>|`
/// came back as `>` + `|`, and a `|` is a pipeline separator, so every
/// plugin reading the result saw the target file in command position.
#[test]
fn redirection_operators_are_single_tokens() {
    assert_same_words(&[
        "zstyle >| /tmp/out",
        "zstyle>|/tmp/out",
        "a >! b",
        "a >>| b",
        "a >& 2",
        "a 2>&1",
        "a &> b",
        "a &>> b",
        "a >&| b",
        "a <> b",
        "a <<< b",
        "a <<- b",
        "a <& 3",
        "a 2<&3",
        "a 10>x",
        "a 2&>x",
        "x>y",
        "x<y",
    ]);
}

/// A `(`, a numeric glob and a digit that turns out not to be an fd stay
/// word text (c:Src/lex.c:828-836, c:858-861, c:1171-1211).
#[test]
fn process_substitution_and_numeric_glob_stay_in_the_word() {
    assert_same_words(&[
        "x>(y)",
        "a >>(x)",
        "a <<(x)",
        "a 2>(x)",
        "a <1-5>",
        "b<1-5>x",
        "a 2<1-3>",
        "echo $(b > c) d",
    ]);
}

/// Nothing ends a word while a `${…}` is open (c:Src/lex.c:960, 966, 991,
/// 1001, 1172, 1209): not a blank, `;`, `|`, `)`, `<` or `>`. zshrs split
/// `${a:-x y}` into two words.
#[test]
fn an_open_brace_parameter_keeps_its_body_in_one_word() {
    assert_same_words(&[
        "print ${a:-x y} z",
        "a ${b:-${c:-d e}} f",
        "print ${x//>/y}",
        "print ${x//</y}",
        "print ${a:-p|q}",
        "print ${a;b}",
        "a{b>c}d",
    ]);
}

/// `|&` is one token, BARAMP (c:Src/lex.c:777).
#[test]
fn bar_amp_is_one_token() {
    assert_same_words(&["a |& b", "a || b | c"]);
}
