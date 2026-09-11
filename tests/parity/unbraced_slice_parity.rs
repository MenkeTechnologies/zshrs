//! Unbraced `$name[lo,hi]` slices whose subscript carries a `$`.
//!
//! A subscript with a `$` in it cannot be resolved at compile time, so the
//! word goes to the runtime expander, whose unbraced arm had two defects
//! the braced `${name[lo,hi]}` spelling does not:
//!
//! * after splicing the slice into the word it told stringsubst to resume
//!   scanning at the START of the last element (c:Src/subst.c:323-329 hands
//!   back the offset just past the value), so that element's text was
//!   expanded a second time — `w=(a 'b$c'); print $w[1,$#w]` printed `a b`,
//!   and an element holding `${…` aborted with "bad substitution";
//! * each bound was read with `parse()` and a default instead of
//!   `mathevalarg` (c:Src/params.c:1618, c:2133), so a bound such as `n` or
//!   `n+1` became 1 or the length.
//!
//! zsh-expand's command-line parser (`$mywordsleft[$firstIndex,$#mywordsleft]`)
//! hit the first one on any command line whose last word contains a `$`.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

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

/// stdout + exit-status parity; stderr compared with the shell name
/// normalized (`zsh:` vs `zshrs:`).
fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let z_out = String::from_utf8_lossy(&z.stdout).into_owned();
    let r_out = String::from_utf8_lossy(&r.stdout).into_owned();
    assert_eq!(
        z_out, r_out,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{z_out:?}\n--- zshrs ---\n{r_out:?}"
    );
    let norm = |s: &str| s.replace("zshrs:", "SHELL:").replace("zsh:", "SHELL:");
    let z_err = norm(&String::from_utf8_lossy(&z.stderr));
    let r_err = norm(&String::from_utf8_lossy(&r.stderr));
    assert_eq!(
        z_err, r_err,
        "stderr divergence on script:\n{script}\n--- zsh ---\n{z_err:?}\n--- zshrs ---\n{r_err:?}"
    );
    assert_eq!(
        z.status.code().unwrap_or(-1),
        r.status.code().unwrap_or(-1),
        "exit divergence on script:\n{script}"
    );
}

/// The last element of the slice is text, not source: its `$c` must not be
/// expanded again, as a word of its own or with text on either side.
#[test]
fn the_last_element_of_a_slice_is_not_expanded_again() {
    assert_parity(r#"w=(a 'b$c'); print -r -- $w[1,$#w]"#);
    assert_parity(r#"w=(a 'b$c'); n=2; print -r -- x$w[1,$n]y"#);
    assert_parity(r#"w=('$c'); n=1; print -r -- x$w[1,$n]y"#);
}

/// The shape zsh-expand builds from the command line: an element holding an
/// unfinished `${…`, copied through an array assignment. zshrs aborted the
/// assignment with "bad substitution".
#[test]
fn an_element_holding_a_brace_expansion_is_copied_verbatim() {
    assert_parity(
        r#"w=(print -r -- 'Z=${#${(f)";zstyle -L)"}}'); i=1; x=( $w[$i,$#w] ); print -r -- $#x $x[-1]"#,
    );
}

/// Both bounds are arithmetic, and are evaluated once.
#[test]
fn slice_bounds_are_arithmetic() {
    assert_parity(r#"w=(a b c d); n=2; print -r -- $w[n,$n+1]"#);
    assert_parity(r#"w=(a b c d); n=2; print -r -- $w[$n,n+1]"#);
    assert_parity(r#"s=abcdef; n=2; print -r -- $s[n,$n+1]"#);
    assert_parity(r#"w=(a b c); i=1; print -r -- $w[i++,$#w] $i"#);
    assert_parity(r#"w=(a b c); e=; print -r -- $w[$e,2]"#);
}
