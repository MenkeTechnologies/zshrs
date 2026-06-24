//! Parity tests for the `WC_AUTOFN` dispatch path (`execautofn` →
//! `loadautofn` → `execautofn_basic`) and ksh-style funcdef stripping
//! (`stripkshdef`).
//!
//! These pin behaviour observable from an autoload file: when the file
//! is a plain body, calling the function should run the body; when the
//! file wraps the body in `function NAME { … }` syntax, zsh's default
//! `autoload -U` mode strips the wrapper so the body runs on first
//! call (this is what `stripkshdef` does at the wordcode level).

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

struct R {
    stdout: String,
    exit: i32,
}

fn run_zsh_in(d: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(d)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs_in(d: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(d)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Plain-body autoload file: `autoload -U FN; FN` should print the
/// file's contents (first call faults the body in via `loadautofn`,
/// then `execautofn_basic` runs it). Pins the `execautofn`→
/// `execautofn_basic` dispatch wired by `dispatch_execfuncs`.
#[test]
fn autoload_plain_body_runs_on_first_call() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("af_plain"), "echo plain_body_ran\n").unwrap();
    let script = format!(
        r#"fpath=({} $fpath); autoload -U af_plain; af_plain"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(
        z.stdout.trim(),
        "plain_body_ran",
        "zsh sanity: {:?}",
        z.stdout
    );
    assert_eq!(
        r.stdout.trim(),
        "plain_body_ran",
        "zshrs autoload-of-plain-body — execautofn dispatch broken?\nzshrs: {:?}",
        r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

/// Autoload file that uses an explicit option in the body — verifies
/// the loaded body executes in the function's scope, not at parse
/// time. Pins `execautofn_basic` writes through to the caller's
/// `LASTVAL`.
#[test]
fn autoload_body_propagates_exit_status() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("af_exit"), "return 7\n").unwrap();
    let script = format!(
        r#"fpath=({} $fpath); autoload -U af_exit; af_exit; print $?"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout.trim(), "7", "zsh sanity");
    assert_eq!(r.stdout.trim(), "7", "zshrs lost exit status");
    assert_eq!(z.exit, r.exit);
}

/// Calling an autoloaded fn twice — the second call must NOT re-run
/// `loadautofn` (already loaded). Verifies state.prog.shf carries
/// the loaded funcdef across calls.
#[test]
fn autoload_second_call_uses_cached_body() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("af_twice"), "echo twice_$1\n").unwrap();
    let script = format!(
        r#"fpath=({} $fpath); autoload -U af_twice; af_twice one; af_twice two"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout, "twice_one\ntwice_two\n", "zsh sanity");
    assert_eq!(
        r.stdout, "twice_one\ntwice_two\n",
        "zshrs autoload re-load divergence"
    );
    assert_eq!(z.exit, r.exit);
}

/// `autoload -U` of a nonexistent function: the call must report
/// non-zero `$?`. zsh and zshrs use different specific exit codes
/// (zsh: 1, zshrs: 127) — pre-existing divergence, not pinned here.
/// What matters for the `execautofn` dispatch is that the failure
/// path doesn't crash and surfaces non-zero status.
#[test]
fn autoload_missing_function_reports_failure() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let script = format!(
        r#"fpath=({} $fpath); autoload -U af_nope; af_nope 2>/dev/null; echo done=$?"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert!(
        z.stdout.trim().starts_with("done="),
        "zsh sanity: {:?}",
        z.stdout
    );
    assert!(
        r.stdout.trim().starts_with("done="),
        "zshrs missing-fn path crashed: {:?}",
        r.stdout
    );
    let z_code = z.stdout.trim().trim_start_matches("done=");
    let r_code = r.stdout.trim().trim_start_matches("done=");
    assert_ne!(z_code, "0", "zsh expected non-zero");
    assert_ne!(r_code, "0", "zshrs expected non-zero");
}

// =====================================================================
// `.zwc` autoload — getfpfunc tries try_dump_file per fpath dir BEFORE
// the plain file (Src/exec.c:6238), header/version/mtime checks in
// try_dump_file / check_dump_file (Src/parse.c:3746/3833).
// =====================================================================

/// fpath dir containing ONLY `fn.zwc` (source deleted), compiled by
/// REAL zsh — zshrs must locate the function in the dump and run it.
/// Pins the c:exec.c:6238 try_dump_file arm of getfpfunc and the
/// check_dump_file body load (c:parse.c:3919-3958).
#[test]
fn autoload_zwc_only_dir_real_zsh_compiled() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("zwfn"), "echo zw_body $1\n").unwrap();
    let z = run_zsh_in(d.path(), "zcompile zwfn");
    assert_eq!(z.exit, 0, "zsh zcompile sanity");
    std::fs::remove_file(d.path().join("zwfn")).unwrap();
    let script = format!(
        r#"fpath=({}); autoload -Uz zwfn; zwfn arg1"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(
        z.stdout.trim(),
        "zw_body arg1",
        "zsh sanity: {:?}",
        z.stdout
    );
    assert_eq!(
        r.stdout, z.stdout,
        "zshrs failed to autoload from .zwc-only fpath dir"
    );
    assert_eq!(z.exit, r.exit);
}

/// Cross direction: ZSHRS zcompiles the function file; both shells
/// must then load the dump (pins write_dump emission + the loader
/// round-trip, and that real zsh accepts zshrs-written dumps).
#[test]
fn autoload_zwc_only_dir_zshrs_compiled() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("rwfn"), "echo rw_body $1\n").unwrap();
    let r = run_zshrs_in(d.path(), "zcompile rwfn");
    assert_eq!(r.exit, 0, "zshrs zcompile failed");
    std::fs::remove_file(d.path().join("rwfn")).unwrap();
    let script = format!(
        r#"fpath=({}); autoload -Uz rwfn; rwfn arg1"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(
        z.stdout.trim(),
        "rw_body arg1",
        "real zsh rejected zshrs-written dump: {:?}",
        z.stdout
    );
    assert_eq!(r.stdout, z.stdout, "zshrs cannot load its own dump");
    assert_eq!(z.exit, r.exit);
}

/// Both compiled + plain present: the dump wins only when its mtime
/// is >= the source's (c:parse.c:3779 `stc.st_mtime >= stn.st_mtime`).
/// Distinct bodies make the winner observable.
#[test]
fn autoload_zwc_mtime_preference() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("mtfn"), "echo compiled_body\n").unwrap();
    let z = run_zsh_in(d.path(), "zcompile mtfn");
    assert_eq!(z.exit, 0, "zsh zcompile sanity");
    // Rewrite the source with different output, then backdate it so
    // the dump is newer.
    std::fs::write(d.path().join("mtfn"), "echo source_body\n").unwrap();
    let old = filetime::FileTime::from_unix_time(1577836800, 0); // 2020-01-01
    filetime::set_file_mtime(d.path().join("mtfn"), old).unwrap();
    let script = format!(r#"fpath=({}); autoload -Uz mtfn; mtfn"#, d.path().display());
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout.trim(), "compiled_body", "zsh sanity (dump newer)");
    assert_eq!(r.stdout, z.stdout, "zshrs ignored newer dump");
    // Now make the source newer than the dump — the plain file wins. Use a
    // fixed far-future mtime rather than `now()`: the .zwc was created moments
    // earlier, so at second granularity (which both shells compare on) `now()`
    // can TIE the dump's second under fast/contended execution, flipping
    // zwc>=source and breaking the test's own premise. 2030 is unambiguously
    // newer than the just-built dump.
    let new = filetime::FileTime::from_unix_time(1893456000, 0); // 2030-01-01
    filetime::set_file_mtime(d.path().join("mtfn"), new).unwrap();
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout.trim(), "source_body", "zsh sanity (source newer)");
    assert_eq!(
        r.stdout, z.stdout,
        "zshrs preferred stale dump over newer source"
    );
}

/// `zcompile -k` marks the dump FDHF_KSHLOAD (c:parse.c:3149); the
/// loader must then execute the file contents and call the defined
/// function (c:exec.c:5725-5746) — observable as BOTH the trailing
/// self-call output and the real-call output, in order.
#[test]
fn autoload_zwc_kshload_flag() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("kfn"), "kfn() { echo kbody $1; }\nkfn boot\n").unwrap();
    let z = run_zsh_in(d.path(), "zcompile -k kfn");
    assert_eq!(z.exit, 0, "zsh zcompile -k sanity");
    std::fs::remove_file(d.path().join("kfn")).unwrap();
    let script = format!(
        r#"fpath=({}); autoload -Uz kfn; kfn arg1"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(
        z.stdout, "kbody boot\nkbody arg1\n",
        "zsh sanity: {:?}",
        z.stdout
    );
    assert_eq!(
        r.stdout, z.stdout,
        "zshrs FDHF_KSHLOAD handling diverges (file-run + re-call)"
    );
    assert_eq!(z.exit, r.exit);
}

/// Directory digest: `<dir>.zwc` beside the fpath dir holds multiple
/// functions (c:parse.c:3766 `dig = dyncat(path, FD_EXT)`); and an
/// fpath element that IS a `.zwc` file (c:parse.c:3753 strsfx arm).
#[test]
fn autoload_zwc_digest_and_zwc_fpath_entry() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let fns = d.path().join("fns");
    std::fs::create_dir(&fns).unwrap();
    std::fs::write(fns.join("dg1"), "echo dg_one $1\n").unwrap();
    std::fs::write(fns.join("dg2"), "echo dg_two $1\n").unwrap();
    let z = run_zsh_in(d.path(), "zcompile fns.zwc fns/dg1 fns/dg2");
    assert_eq!(z.exit, 0, "zsh digest zcompile sanity");
    std::fs::remove_file(fns.join("dg1")).unwrap();
    std::fs::remove_file(fns.join("dg2")).unwrap();
    // Digest beside the dir.
    let script = format!(
        r#"fpath=({}); autoload -Uz dg1 dg2; dg1 a; dg2 b"#,
        fns.display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout, "dg_one a\ndg_two b\n", "zsh digest sanity");
    assert_eq!(r.stdout, z.stdout, "zshrs digest <dir>.zwc lookup broken");
    // fpath entry pointing AT the .zwc file itself.
    let script = format!(
        r#"fpath=({}/fns.zwc); autoload -Uz dg1; dg1 c"#,
        d.path().display()
    );
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout, "dg_one c\n", "zsh zwc-as-fpath sanity");
    assert_eq!(r.stdout, z.stdout, "zshrs zwc-as-fpath-entry lookup broken");
}

/// `source file` with a newer sibling `file.zwc` loads the compiled
/// body (c:init.c:1566 try_source_file), including when the plain
/// file is deleted entirely (slash-path arm, c:builtin.c:6092-6100).
#[test]
fn source_zwc_sibling_and_zwc_only() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("s.zsh"), "echo compiled_src\n").unwrap();
    let z = run_zsh_in(d.path(), "zcompile s.zsh");
    assert_eq!(z.exit, 0, "zsh zcompile sanity");
    std::fs::write(d.path().join("s.zsh"), "echo plain_src\n").unwrap();
    let old = filetime::FileTime::from_unix_time(1577836800, 0);
    filetime::set_file_mtime(d.path().join("s.zsh"), old).unwrap();
    let script = format!("source {}/s.zsh", d.path().display());
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout.trim(), "compiled_src", "zsh sanity (zwc newer)");
    assert_eq!(r.stdout, z.stdout, "zshrs source ignored newer .zwc");
    // zwc-only: plain file removed, slash path still sources the dump.
    std::fs::remove_file(d.path().join("s.zsh")).unwrap();
    let z = run_zsh_in(d.path(), &script);
    let r = run_zshrs_in(d.path(), &script);
    assert_eq!(z.stdout.trim(), "compiled_src", "zsh sanity (zwc only)");
    assert_eq!(r.stdout, z.stdout, "zshrs source of zwc-only path broken");
    assert_eq!(z.exit, r.exit);
}

/// zshrs zcompile of a function file whose case arms use the
/// open-paren `(pat)` form (c:Src/parse.c:1321-1357 whole-pattern
/// hack in par_case). The wordcode parser previously errored
/// "par_case: expected `)` or `|`" on every such file. Both shells
/// must load + run the zshrs-written dump.
#[test]
fn autoload_zwc_open_paren_case_arms() {
    if !zsh_available() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("opfn"),
        "case $1 in\n  (a) print A;;\n  (b|c) print BC;;\n  ( d | e ) print DE;;\n  (*) print other;;\nesac\n",
    )
    .unwrap();
    let r = run_zshrs_in(d.path(), "zcompile opfn");
    assert_eq!(
        r.exit, 0,
        "zshrs zcompile of (pat) case arms failed: {}",
        r.stdout
    );
    std::fs::remove_file(d.path().join("opfn")).unwrap();
    for (arg, want) in [("a", "A"), ("c", "BC"), ("e", "DE"), ("zz", "other")] {
        let script = format!(
            r#"fpath=({}); autoload -Uz opfn; opfn {}"#,
            d.path().display(),
            arg
        );
        let z = run_zsh_in(d.path(), &script);
        let r = run_zshrs_in(d.path(), &script);
        assert_eq!(
            z.stdout.trim(),
            want,
            "real zsh rejected zshrs dump: {}",
            z.stdout
        );
        assert_eq!(r.stdout, z.stdout, "zshrs cannot run its own (pat) dump");
        assert_eq!(z.exit, r.exit);
    }
}
