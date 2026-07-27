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

/// The four `$zsh_eval_context` tests that real code on a loaded zsh
/// install actually performs, all of which read the LAST frame:
///
///   * `[[ $zsh_eval_context[-1] == loadautofunc ]]` — the shtab/click
///     completion-generator idiom, "am I being autoloaded by compsys
///     right now, or was my file eval'd?".
///   * `[[ $zsh_eval_context == *func ]]` — the pnpm-generator idiom;
///     looser, so it keeps working on the second call when the frame
///     has decayed to `shfunc`.
///   * upstream `Functions/Misc/add-zle-hook-widget`'s three-way
///     `case` over `*file` / `*evalautofunc` / `*loadautofunc`.
///   * `[[ $ZSH_EVAL_CONTEXT == *:file || … == file ]]` — the
///     sourced-vs-run test, on the colon scalar because `sh`/`ksh`
///     emulation does not expose the lowercase array (`params.c:454`).
///
/// Nothing found in the wild reads a middle frame or a frame count,
/// which is why the completion-time depth divergence documented in
/// `docs/COMPLETION_DISPATCH.md` (Divergence C) is deliberately left
/// open while THIS is pinned: these are the semantics that decide
/// whether a completion file registers itself or completes.
///
/// Expectations captured from `zsh -f -c` (zsh 5.9.2) against this
/// exact fixture, so the test still pins on a box with no zsh.
#[test]
fn consumer_idioms_match_zsh() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skip: zshrs binary not built");
        return;
    };
    let Some(dir) = tempfile::tempdir().ok() else {
        eprintln!("skip: could not create fixture fpath");
        return;
    };
    let probe = "\
print -r -- \"A=${zsh_eval_context[-1]}\"
if [[ $zsh_eval_context == *func ]]; then print -r -- B=func; else print -r -- B=notfunc; fi
case \"$zsh_eval_context\" in
  *file) print -r -- D=file;;
  *evalautofunc) print -r -- D=evalautofunc;;
  *loadautofunc) print -r -- D=loadautofunc;;
  *) print -r -- D=fallback;;
esac
if [[ $ZSH_EVAL_CONTEXT == *:file || $ZSH_EVAL_CONTEXT == file ]]; then
  print -r -- G=sourced
else
  print -r -- G=notsourced
fi
print -r -- \"F=$ZSH_EVAL_CONTEXT\"
";
    let file = dir.path().join("_gidiom");
    if std::fs::File::create(&file)
        .and_then(|mut f| f.write_all(probe.as_bytes()))
        .is_err()
    {
        eprintln!("skip: could not write fixture");
        return;
    }
    let d = dir.path().display();
    let script = format!(
        "fpath=( {d} $fpath )\n\
         print -r -- '== autoload =='\n\
         autoload -Uz _gidiom; _gidiom\n\
         print -r -- '== nested-shfunc =='\n\
         unfunction _gidiom; autoload -Uz _gidiom\n\
         outer() {{ _gidiom }}; outer\n\
         print -r -- '== nested-eval =='\n\
         unfunction _gidiom; autoload -Uz _gidiom\n\
         outer2() {{ eval \"_gidiom\" }}; outer2\n\
         print -r -- '== second-call =='\n\
         _gidiom\n\
         print -r -- '== source =='\n\
         source {d}/_gidiom\n\
         print -r -- '== eval =='\n\
         eval \"$(<{d}/_gidiom)\"\n"
    );

    // The `second-call` block is what makes this more than a string
    // check: once loaded, the same function reports plain `shfunc`, so
    // the strict `[-1] == loadautofunc` consumers stop matching and the
    // loose `== *func` ones keep matching. A frame pushed on EVERY call
    // would pass the first three blocks and fail this one.
    let expected = "\
== autoload ==
A=loadautofunc
B=func
D=loadautofunc
G=notsourced
F=cmdarg:shfunc:loadautofunc
== nested-shfunc ==
A=loadautofunc
B=func
D=loadautofunc
G=notsourced
F=cmdarg:shfunc:shfunc:loadautofunc
== nested-eval ==
A=loadautofunc
B=func
D=loadautofunc
G=notsourced
F=cmdarg:shfunc:eval:shfunc:loadautofunc
== second-call ==
A=shfunc
B=func
D=fallback
G=notsourced
F=cmdarg:shfunc
== source ==
A=file
B=notfunc
D=file
G=sourced
F=cmdarg:file
== eval ==
A=eval
B=notfunc
D=fallback
G=notsourced
F=cmdarg:eval
";
    assert_eq!(
        run(&bin, &script),
        expected,
        "a $zsh_eval_context consumer idiom diverged from zsh"
    );
}

/// No compsys Rust port may push a `$zsh_eval_context` frame.
///
/// The ports stand in for shell functions that C would autoload and
/// `eval`, so inside a live completion zshrs reports a shorter frame
/// stack than zsh — six frames shorter for `ectxprobe <TAB>` on a
/// stock fpath. The frames C produces there describe machinery that
/// does not run in zshrs: `loadautofunc` means "a file was just read
/// from $fpath and parsed", `eval` means "a string was just parsed and
/// executed", and two of the six belong to `_normal`, a function the
/// `_complete` port calls as a direct Rust call so it never appears in
/// `$funcstack` either.
///
/// Emitting them anyway would put `$zsh_eval_context` and `$funcstack`
/// into disagreement and would send anyone debugging a completion
/// looking for a load and an eval that never happened. The survey
/// behind `consumer_idioms_match_zsh` found no code that reads those
/// frames. So the divergence is deliberate, and this test is what
/// stops it being "fixed" by fabrication. Reasoning in
/// `docs/COMPLETION_DISPATCH.md`, Divergence C.
///
/// If a port ever legitimately needs a frame — because it grew a real
/// `eval` or a real autoload — push it from the code that performs the
/// operation and update this list, do not widen the test to a
/// name-based allowlist.
#[test]
fn compsys_ports_synthesize_no_eval_context_frames() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/compsys");
    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().map(|e| e != "rs").unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            scanned += 1;
            for (i, line) in text.lines().enumerate() {
                // Prose in a doc comment is fine; a call is not.
                if line.contains("EvalContextFrame::push") {
                    offenders.push(format!("{}:{}", path.display(), i + 1));
                }
            }
        }
    }
    // Anti-vacuity: a walk that silently found nothing to read would
    // pass forever, including after the tree is moved or renamed.
    assert!(
        scanned > 100,
        "only {scanned} .rs files under {} — the walk is not finding the \
         compsys port tree, so the check below proves nothing",
        root.display()
    );
    assert!(
        offenders.is_empty(),
        "compsys Rust ports must not synthesize $zsh_eval_context frames \
         (see docs/COMPLETION_DISPATCH.md, Divergence C); found: {offenders:?}"
    );
}
