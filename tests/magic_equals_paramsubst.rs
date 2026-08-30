//! MAGIC_EQUAL_SUBST must not treat a `:` inside `${...}` as a path
//! separator.
//!
//! With `setopt magic_equal_subst`, a word shaped like `--flag=VALUE` goes
//! through `filesub` in assign context, which walks `:`-separated path
//! components and applies `=cmd` / `~user` expansion to each. The colon of
//! `${VAR:=default}` is not such a separator — it belongs to the parameter
//! expansion — but it was being treated as one, so `${V:=x}` handed `=x}` to
//! the `=cmd` lookup and the shell printed `x} not found` for a word real zsh
//! leaves untouched.
//!
//! This is not a synthetic shape. fzf-tab builds its command array with
//! `--height='${FZF_TMUX_HEIGHT:=75%}'` and zpwr's `zpwrBindFZFLate` uses the
//! same construct with `100%`, so an interactive shell with both loaded
//! printed `75%} not found` / `100%} not found` on every startup.

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target/debug/zshrs");
    p
}

/// Run a script under `setopt magic_equal_subst`, returning stdout+stderr.
fn run(script: &str) -> String {
    let bin = zshrs_bin();
    let body = format!("setopt magic_equal_subst\n{script}\n");
    let out = Command::new(&bin)
        .args(["-f", "-c", &body])
        .output()
        .expect("run zshrs");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn colon_inside_a_parameter_expansion_is_not_a_path_separator() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs not built");
        return;
    }
    for (script, want) in [
        // The two shapes from the real plugins.
        (
            r#"a=( --height='${FZF_TMUX_HEIGHT:=75%}' ); print -r -- $a"#,
            "--height=${FZF_TMUX_HEIGHT:=75%}",
        ),
        (
            r#"a=( --layout=reverse --height='${V:=100%}' ); print -r -- $a[2]"#,
            "--height=${V:=100%}",
        ),
        // `:-` never had the defect; pin it so a future change keeps both.
        (
            r#"a=( --x='${V:-y}' ); print -r -- $a"#,
            "--x=${V:-y}",
        ),
    ] {
        let got = run(script);
        assert!(
            got.trim() == want,
            "script: {script}\n  got:  {got:?}\n  want: {want:?}"
        );
        assert!(
            !got.contains("not found"),
            "a parameter expansion's own text must never reach the =cmd lookup: {got:?}"
        );
    }
}

/// The colon walk still has to do its real job: a genuine path list with an
/// `=cmd` component expands, which is the behaviour the guard must preserve.
#[test]
fn real_path_list_components_still_expand() {
    if !zshrs_bin().exists() {
        eprintln!("skip: zshrs not built");
        return;
    }
    // `=ls` in a colon list resolves through $PATH.
    let got = run("P=/bin:=ls; print -r -- $P");
    assert!(
        got.contains("/bin:") && got.contains("ls") && !got.contains("not found"),
        "an =cmd component of a real path list must still expand: {got:?}"
    );
    // A tilde component likewise.
    let got = run("P=/usr/bin:~root; print -r -- $P");
    assert!(
        !got.contains('~') && !got.contains("not found"),
        "a ~user component of a real path list must still expand: {got:?}"
    );
    // And plain `=cmd` at the head of a word is untouched by any of this.
    let got = run("print -r -- =ls");
    assert!(
        got.trim().ends_with("ls") && got.starts_with('/'),
        "=cmd expansion must still resolve through $PATH: {got:?}"
    );
}
