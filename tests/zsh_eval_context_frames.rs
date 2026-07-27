//! `$ZSH_EVAL_CONTEXT` frame parity with zsh.
//!
//! C maintains one `char **zsh_eval_context` that every
//! `execode(prog, …, "label")` pushes onto (`Src/exec.c:1245-1282`).
//! zshrs does not route nested execution through `execode`, so each C
//! call site has to push its own label. Two were missing:
//!
//!   * `"loadautofunc"` — `execautofn_basic`'s
//!     `execode(shf->funcdef, 1, 0, "loadautofunc")` (`Src/exec.c:5626`),
//!     the frame a function body runs in on the call that autoloaded it.
//!   * `"evalautofunc"` — the ksh-autoload branch's
//!     `execode(prog, 1, 0, "evalautofunc")` (`Src/exec.c:5739`).
//!
//! Without them a chain of freshly autoloaded functions reported a flat
//! `shfunc:shfunc:shfunc` where zsh reports
//! `shfunc:loadautofunc:shfunc:loadautofunc:…` — visible to any script
//! that branches on `$ZSH_EVAL_CONTEXT`, and to `echo $ZSH_<TAB>` whose
//! completion listing renders the value.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("target/debug/zshrs"),
        manifest.join("target/release/zshrs"),
    ]
    .into_iter()
    .find(|cand| cand.exists())
}

/// An fpath dir holding three autoloadable functions that print
/// `$ZSH_EVAL_CONTEXT` at increasing nesting, with an `eval` in the
/// middle. Returned dir is deleted when the `TempDir` drops.
fn fixture_fpath() -> Option<tempfile::TempDir> {
    let dir = tempfile::tempdir().ok()?;
    for (name, body) in [
        (
            "ectxA",
            "ectxA() { print -r -- \"A=[$ZSH_EVAL_CONTEXT]\"; ectxB }\n",
        ),
        (
            "ectxB",
            "ectxB() { print -r -- \"B=[$ZSH_EVAL_CONTEXT]\"; \
             eval 'print -r -- \"E=[$ZSH_EVAL_CONTEXT]\"; ectxC' }\n",
        ),
        ("ectxC", "ectxC() { print -r -- \"C=[$ZSH_EVAL_CONTEXT]\" }\n"),
    ] {
        let mut f = std::fs::File::create(dir.path().join(name)).ok()?;
        f.write_all(body.as_bytes()).ok()?;
    }
    Some(dir)
}

fn script_for(fpath: &std::path::Path) -> String {
    format!(
        "fpath=( {} $fpath )\n\
         autoload -Uz ectxA ectxB ectxC\n\
         print -r -- \"T=[$ZSH_EVAL_CONTEXT]\"\n\
         ectxA\n\
         ectxA\n",
        fpath.display()
    )
}

fn run(bin: &std::path::Path, script: &str) -> String {
    let out = Command::new(bin)
        .args(["-f", "-c", script])
        .env_remove("ZDOTDIR")
        .env_remove("ZSHRS_CACHE")
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin:?}: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The whole frame trace across a three-deep autoloaded call chain with
/// an `eval` in the middle. What makes this more than a string check is
/// the second half: calling the same three functions AGAIN must drop
/// every `loadautofunc` frame, because they are already loaded. A fix
/// that pushed the frame on every call would pass the first five lines
/// and fail the last four.
#[test]
fn eval_context_frames_match_zsh() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skip: zshrs binary not built");
        return;
    };
    let Some(dir) = fixture_fpath() else {
        eprintln!("skip: could not create fixture fpath");
        return;
    };
    let script = script_for(dir.path());

    // Literal expectation rather than a live diff against the system
    // zsh: this has to pin behaviour identically on a headless box with
    // no zsh installed, and on one whose zsh is a different build. The
    // values were captured from `zsh -f -c` (zsh 5.9.2) against this
    // exact fixture.
    let expected = "\
T=[cmdarg]
A=[cmdarg:shfunc:loadautofunc]
B=[cmdarg:shfunc:loadautofunc:shfunc:loadautofunc]
E=[cmdarg:shfunc:loadautofunc:shfunc:loadautofunc:eval]
C=[cmdarg:shfunc:loadautofunc:shfunc:loadautofunc:eval:shfunc:loadautofunc]
A=[cmdarg:shfunc]
B=[cmdarg:shfunc:shfunc]
E=[cmdarg:shfunc:shfunc:eval]
C=[cmdarg:shfunc:shfunc:eval:shfunc]
";
    assert_eq!(
        run(&bin, &script),
        expected,
        "ZSH_EVAL_CONTEXT frame stack diverged from zsh"
    );
}

/// A single `eval` must contribute exactly ONE frame. The label is
/// pushed on the live `BUILTIN_EVAL` path; a second push in `bin_eval`
/// would double it, which this pins.
#[test]
fn eval_pushes_exactly_one_frame() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skip: zshrs binary not built");
        return;
    };
    assert_eq!(
        run(&bin, r#"eval 'print -r -- $ZSH_EVAL_CONTEXT'"#).trim(),
        "cmdarg:eval"
    );
    assert_eq!(
        run(&bin, r#"f(){ eval 'print -r -- $ZSH_EVAL_CONTEXT' }; f"#).trim(),
        "cmdarg:shfunc:eval"
    );
}
