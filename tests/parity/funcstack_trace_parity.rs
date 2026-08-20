//! Parity pins for the `funcstack` subsystem — the `FS_FUNC` frame
//! `doshfunc` pushes (`Src/exec.c:6005-6019`), the `FS_EVAL` frame `eval`
//! pushes (`Src/builtin.c:6155-6193`), and the four parameters that read
//! them back (`$funcstack`, `$functrace`, `$funcsourcetrace`,
//! `$funcfiletrace` — `Src/Modules/parameter.c:627-766`).
//!
//! Why these are worth pinning rather than obvious: completion code sizes
//! its own nesting off `$#funcstack` (`_all_labels` / `_alternative` weigh
//! it against `_tags_level`), so a frame that silently fails to go on the
//! stack changes what completion offers — it does not merely misreport a
//! trace. The `(eval)` frame was exactly that bug: the `_dispatch` port
//! called its completer by name where the shell writes `eval "$comp"`.
//!
//! Every assertion is differential against the system zsh. The one place
//! that cannot be is `$0`: zshrs reports its own binary path and
//! deliberately does not impersonate `/bin/zsh`, so the argzero test pins
//! each shell against its OWN argv[0] instead.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

fn run_zsh(script: &str) -> String {
    let o = Command::new(zsh_path())
        .args(["-f", "-c", script])
        .output()
        .expect("zsh");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn run_zshrs(script: &str) -> String {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// `eval` opens a funcstack frame named `(eval)`
/// (`Src/builtin.c:6157-6189`: `fstack.name = scriptname` where
/// `scriptname == "(eval)"`, `fstack.tp = FS_EVAL`).
///
/// Regression: the frame was pushed only by the `eval` BUILTIN, so a
/// caller that ran shell text through the executor without going through
/// that builtin — the compsys `_dispatch` port, whose upstream runs
/// `eval "$comp"` at sh:63 — produced a `$funcstack` one frame shallower
/// than zsh's for every completer it dispatched.
#[test]
fn eval_opens_a_frame_named_eval() {
    if !zsh_available() {
        return;
    }
    let script = r#"f(){ print -l $funcstack }; eval f"#;
    let z = run_zsh(script);
    assert_eq!(z, "f\n(eval)\n", "zsh sanity: {z:?}");
    assert_eq!(run_zshrs(script), z, "zshrs dropped the (eval) frame");
}

/// The `(eval)` frame carries a real caller line, not a placeholder:
/// `fstack.lineno = lineno` (`Src/builtin.c:6161`) is read back by
/// `functracegetfn` as `"<caller>:<lineno>"`
/// (`Src/Modules/parameter.c:663`).
///
/// Regression: the inlined push left `lineno` at 0, so every entry of
/// `$functrace` under an eval read `<caller>:0`.
#[test]
fn functrace_carries_the_line_the_eval_was_called_on() {
    if !zsh_available() {
        return;
    }
    // `eval f` sits on line 1 OF g's body, so zsh reports `g:1`; the frame
    // above it is the body of the eval itself, whose lines start at 1.
    let script =
        "f() {\n  print -r -- \"[$functrace[1]][$functrace[2]]\"\n}\ng() {\n  eval f\n}\ng";
    let z = run_zsh(script);
    assert_eq!(z.trim(), "[(eval):1][g:1]", "zsh sanity: {z:?}");
    assert_eq!(
        run_zshrs(script).trim(),
        z.trim(),
        "zshrs lost the caller line recorded at the eval push"
    );
}

/// An eval nested inside another eval subtracts one from the derived file
/// line, because eval bodies are numbered from 1 rather than 0:
/// `if (funcstack->tp == FS_EVAL) fstack.flineno--`
/// (`Src/builtin.c:6183-6184`), mirrored in `funcfiletracegetfn`
/// (`Src/Modules/parameter.c:757-758`).
#[test]
fn nested_eval_offsets_the_derived_file_line_by_one() {
    if !zsh_available() {
        return;
    }
    let script = "f() { print -r -- \"[$funcsourcetrace[1]][$funcfiletrace[1]]\" }\neval 'eval f'";
    // Only the LINE halves are comparable: the file halves resolve back to
    // each shell's own `-c` source name, and zshrs reports its own binary
    // there rather than impersonating `/bin/zsh`.
    let line_halves = |out: &str| -> Vec<String> {
        out.trim()
            .split(']')
            .filter(|g| g.contains(':'))
            .map(|g| g.rsplit(':').next().unwrap_or("").to_string())
            .collect()
    };
    let z = run_zsh(script);
    assert_eq!(
        line_halves(&z),
        // funcsourcetrace: the `(eval)` frame's own flineno.
        // funcfiletrace:   derived from the parent `(eval)` frame, hence
        //                  `parent.flineno + lineno - 1`.
        vec!["1".to_string(), "2".to_string()],
        "zsh sanity: {z:?}"
    );
    let r = run_zshrs(script);
    assert_eq!(
        line_halves(&r),
        line_halves(&z),
        "nested-eval line derivation diverged\nzsh:   {z:?}\nzshrs: {r:?}"
    );
}

/// `$funcsourcetrace` names the `$fpath` file an autoloaded function came
/// from, at line 0: `loadautofnsetfile` (`Src/exec.c:5713`) stamps the
/// load directory on the shfunc, `doshfunc` copies it into the frame via
/// `getshfuncfile` (`Src/exec.c:6019`), and the def line stays 0 because
/// `Src/exec.c:5384-5388` only stamps a `name() { … }` STATEMENT.
///
/// This is load-bearing beyond tracing: `_git` derives its
/// git-completion.bash search path from `${funcsourcetrace[1]%:*}`, so a
/// frame that reports `zsh` instead of the file resolves to the cwd.
#[test]
fn funcsourcetrace_names_the_fpath_file_of_an_autoloaded_function() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("fst_probe"),
        "print -r -- \"[$funcsourcetrace[1]]\"\n",
    )
    .unwrap();
    let dir = d.path().display().to_string();
    let script = format!("fpath=({dir}); autoload -Uz fst_probe; fst_probe");
    let z = run_zsh(&script);
    assert_eq!(
        z.trim(),
        format!("[{dir}/fst_probe:0]"),
        "zsh sanity: {z:?}"
    );
    assert_eq!(
        run_zshrs(&script).trim(),
        z.trim(),
        "zshrs did not name the fpath file the function was loaded from"
    );
}

/// An `eval` inside an autoloaded function inherits that function's file
/// and derives its line from the parent frame:
/// `fstack.flineno = funcstack->flineno + lineno;
///  fstack.filename = funcstack->filename;` (`Src/builtin.c:6178-6187`).
#[test]
fn eval_inside_an_autoloaded_function_inherits_its_file() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("fev_probe"),
        "eval 'print -r -- \"[$funcsourcetrace[1]][$functrace[1]]\"'\n",
    )
    .unwrap();
    let dir = d.path().display().to_string();
    let script = format!("fpath=({dir}); autoload -Uz fev_probe; fev_probe");
    let z = run_zsh(&script);
    assert_eq!(
        z.trim(),
        format!("[{dir}/fev_probe:1][fev_probe:1]"),
        "zsh sanity: {z:?}"
    );
    assert_eq!(
        run_zshrs(&script).trim(),
        z.trim(),
        "zshrs lost the parent frame's file/line under an eval"
    );
}

/// `parseargs` seeds `argzero` from argv[0] at startup
/// (`Src/init.c:282` — `argv0 = argzero = posixzero = *argv++;`).
///
/// Regression: only `argv0` was seeded, so `$0` read EMPTY for a shell
/// that was neither given `-c` nor a script file. The empty value did not
/// stay cosmetic — `doshfunc` reads it as `funcsave->argv0`
/// (`Src/exec.c:6011`) for the OUTERMOST frame's `caller`, and the
/// None also skipped the restore at `Src/exec.c:6116`, so each function
/// call leaked its own name into the shell's `$0`.
///
/// Compared per-shell rather than differentially: zshrs reports its own
/// binary and deliberately does not claim to be `/bin/zsh`.
#[test]
fn argzero_is_seeded_from_argv0_when_reading_from_stdin() {
    if !zsh_available() {
        return;
    }
    let probe = "print -r -- \"[${0:t}]\"\nf() { : }\nf\nprint -r -- \"[${0:t}]\"\n";
    let feed = |cmd: &mut Command| -> String {
        let mut c = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn");
        use std::io::Write;
        c.stdin
            .as_mut()
            .unwrap()
            .write_all(probe.as_bytes())
            .unwrap();
        let o = c.wait_with_output().unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    let z = feed(Command::new(zsh_path()).arg("-f"));
    assert_eq!(z, "[zsh]\n[zsh]", "zsh sanity: {z:?}");

    let bin = zshrs_bin();
    let want = format!(
        "[{n}]\n[{n}]",
        n = bin.file_name().unwrap().to_string_lossy()
    );
    let r = feed(
        Command::new(&bin)
            .args(["--zsh", "-f"])
            .env_remove("ZSHRS_CACHE"),
    );
    assert_eq!(
        r, want,
        "$0 must be argv[0] at startup and survive a function call"
    );
}
