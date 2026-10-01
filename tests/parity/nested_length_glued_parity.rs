//! `${#${arr}}` glued to literal text in a word.
//!
//! The nested `${arr}` is a `multsub` (`c:Src/subst.c:2681`) that preforks
//! its OWN list (`c:625`), so `c:183-186` prunes that list's empty nodes
//! before the outer `#` counts it: an empty array is 0 elements wherever the
//! expansion sits. zshrs compiles a word with literal text into segments and
//! opens `PARAMSUBST_AFFIXES_DEFERRED` around them; the nested multsub ran
//! with that deferral still open, kept the empty node, and counted one
//! element — `a=(); print x${#${a}}` gave `x1` where zsh gives `x0`. The bare
//! word, the quoted word and the scalar assignment never opened the deferral
//! and were already right, so they are pinned here as the controls.
//!
//! Every case asserts the literal expected output AND zsh's output, in both
//! zshrs modes. Skip pattern: no-ops silently when `zsh` isn't on PATH.

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

fn stdout_of(cmd: &mut Command) -> String {
    let o = cmd.output().expect("invoke shell");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// One line per form, for an array of `elems` elements.
fn script(elems: &str) -> String {
    format!(
        r#"a=({elems})
print -r -- "bare=" ${{#${{a}}}}
print -r -- glued=x${{#${{a}}}} x${{#${{a[@]}}}} x${{#${{(@)a}}}}
print -r -- trailing=${{#${{a}}}}y
print -r -- subscript=x${{#${{a}}[@]}}
print -r -- "quoted=x${{#${{a}}}}"
print -r -- arith=$(( ${{#${{a}}}} )) x$(( ${{#${{a}}}} ))
s=x${{#${{a}}}}; print -r -- assign=$s
w=(x${{#${{a}}}} ${{#${{a}}}}y); print -r -- words=$#w $w
"#
    )
}

fn check(elems: &str, expected: &str) {
    let s = script(elems);
    if Command::new(zsh_path()).arg("--version").output().is_err() {
        eprintln!("skip: zsh not found");
        return;
    }
    let reference = stdout_of(Command::new(zsh_path()).args(["-fc", &s]));
    assert_eq!(reference, expected, "zsh itself disagrees with the pinned output:\n{s}");
    for zsh_mode in [false, true] {
        let mut cmd = Command::new(zshrs_bin());
        if zsh_mode {
            cmd.arg("--zsh");
        }
        cmd.args(["-f", "-c", &s]).env_remove("ZSHRS_CACHE");
        assert_eq!(
            stdout_of(&mut cmd),
            expected,
            "zshrs (--zsh={zsh_mode}) diverges on:\n{s}"
        );
    }
}

#[test]
fn nested_length_of_empty_array() {
    check(
        "",
        "bare= 0\n\
         glued=x0 x0 x0\n\
         trailing=0y\n\
         subscript=x0\n\
         quoted=x0\n\
         arith=0 x0\n\
         assign=x0\n\
         words=2 x0 0y\n",
    );
}

#[test]
fn nested_length_of_one_element_array() {
    check(
        "q",
        "bare= 1\n\
         glued=x1 x1 x1\n\
         trailing=1y\n\
         subscript=x1\n\
         quoted=x1\n\
         arith=1 x1\n\
         assign=x1\n\
         words=2 x1 1y\n",
    );
}

#[test]
fn nested_length_of_two_element_array() {
    // Inside `"…"` and `$(( ))` the nested array is joined first (c:3032),
    // so the outer counts the characters of `q r`: 3.
    check(
        "q r",
        "bare= 2\n\
         glued=x2 x2 x2\n\
         trailing=2y\n\
         subscript=x2\n\
         quoted=x3\n\
         arith=3 x3\n\
         assign=x2\n\
         words=2 x2 2y\n",
    );
}
