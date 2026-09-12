//! A gsu `setfn` that reads a parameter must complete, not deadlock.
//!
//! `paramtab()` is a `std::sync::RwLock` and is not reentrant. C holds no lock
//! at all around a setfn dispatch — `setnumvalue` hands `pm->gsu.i->setfn` a
//! pointer to the live node (`Src/params.c:2870`) and the import loop calls
//! `assignsparam(..., ASSPM_ENV_IMPORT)` (`Src/params.c:907-908`) — so nothing
//! in the C source hints that a setfn must not touch the parameter table. It
//! routinely does: `intsetfn` name-dispatches `$COLUMNS` / `$LINES` to
//! `zlevarsetfn` (`Src/params.c:4226`), which re-enters `adjustwinsize`
//! (`Src/utils.c:1889`), whose geometry probe falls back to reading
//! `$COLUMNS` / `$LINES` back out of the table when the tty reports nothing.
//!
//! Dispatching that under the write guard parks the shell on itself. It is not
//! slowness and not a loop — `sample` on the hung process showed 2665 of 2665
//! samples in `semaphore_wait_trap`, down:
//!
//! ```text
//! ShellExecutor::new -> intsetfn -> zlevarsetfn -> adjustwinsize
//!   -> adjustlines -> getsparam -> RwLock::lock_contended
//! ```
//!
//! The same chain already shipped once, from `setupvals` instead of the
//! environment import, and took out every `zsh/zpty`-spawned shell with it. It
//! was fixed in 5929e82b81 by stopping `adjustwinsize` re-entering — the
//! callee, not the pattern — which left this entry point wide open:
//!
//! ```text
//! COLUMNS=200 zshrs -f -c 'print $COLUMNS'
//! ```
//!
//! printed nothing and never exited.
//!
//! So every probe here runs under a hard timeout and REPORTS the hang instead
//! of wedging the suite. A test that merely blocks tells you nothing about
//! which change broke it.
//!
//! Skip pattern: tests no-op silently when zsh isn't available.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Generous enough that a loaded box never trips it, short enough that a real
/// deadlock reports in under a minute.
const DEADLINE: Duration = Duration::from_secs(45);

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

/// Run `script` under zshrs with `name=value` in the environment, and give up
/// after `DEADLINE`. `Err` carries the elapsed time so the assertion can say
/// "deadlocked" rather than "no output".
fn run_zshrs_with_env(name: &str, value: &str, script: &str) -> Result<String, Duration> {
    let mut child = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env(name, value)
        .env_remove("ZSHRS_CACHE")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn zshrs");

    let started = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None if started.elapsed() >= DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(started.elapsed());
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    Ok(out)
}

fn zsh_with_env(name: &str, value: &str, script: &str) -> String {
    let o = Command::new(zsh_path())
        .args(["-fc", script])
        .env(name, value)
        .output()
        .expect("invoke zsh");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Importing `name=value` must finish, and must agree with zsh.
fn assert_import_completes(name: &str, value: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let script = format!("print -r -- ${name}");
    let got = match run_zshrs_with_env(name, value, &script) {
        Ok(out) => out,
        Err(elapsed) => panic!(
            "DEADLOCK: `{name}={value} zshrs -f -c '{script}'` produced nothing in {elapsed:?} \
             and had to be killed.\nThe environment import dispatched this parameter's gsu setfn \
             while holding the paramtab write guard; the setfn read a parameter back and blocked \
             against its own caller. std's RwLock is not reentrant.\nDiagnose with `sample <pid>`: \
             every sample will be in semaphore_wait_trap under \
             intsetfn -> zlevarsetfn -> adjustwinsize -> getsparam."
        ),
    };
    let want = zsh_with_env(name, value, &script);
    assert_eq!(
        got, want,
        "`{name}={value}` imported differently:\n  zsh   {want:?}\n  zshrs {got:?}"
    );
}

/// The case that hung. `$COLUMNS` is `IPDEF5("COLUMNS", &zterm_columns,
/// zlevar_gsu)` (`Src/params.c:362`), so importing it from the environment is
/// the shortest path to a setfn that reads the parameter table back.
#[test]
fn importing_COLUMNS_from_the_environment_completes() {
    assert_import_completes("COLUMNS", "200");
}

/// `$LINES` is the same `zlevar_gsu` row (`Src/params.c:363`) and exits
/// `adjustwinsize` through `adjustlines` instead of `adjustcolumns` — a
/// separate branch of the same re-entry, so it is pinned separately.
#[test]
fn importing_LINES_from_the_environment_completes() {
    assert_import_completes("LINES", "99");
}

/// Zero is the value that reaches furthest into the geometry code: every
/// `adjust*` fast path is gated on a positive cached value
/// (`Src/utils.c:1861`, `1836`), so only `0` runs the probe and its parameter
/// fallback all the way down. Correctness of the resulting number is not what
/// is pinned here — only that the shell answers at all.
#[test]
fn importing_a_zero_geometry_completes() {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    for name in ["COLUMNS", "LINES"] {
        let script = format!("print -r -- ${name}");
        if let Err(elapsed) = run_zshrs_with_env(name, "0", &script) {
            panic!(
                "DEADLOCK: `{name}=0 zshrs -f -c '{script}'` never finished ({elapsed:?}). \
                 A zero geometry defeats every cached-value fast path in adjustlines / \
                 adjustcolumns, so the parameter fallback runs and re-enters the paramtab \
                 lock the import loop is holding."
            );
        }
    }
}

/// The import's storage write has to stay synchronous even though its setfn no
/// longer does: `Src/params.c:948-951` computes `++shlvl` from the value the
/// loop just imported, and the C comment on it ("shlvl value in environment
/// needs updating unconditionally") sits on an `addenv` of the incremented
/// number. Deferring the storage along with the dispatch made this answer `1`.
#[test]
fn SHLVL_still_increments_the_imported_value() {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let got = run_zshrs_with_env("SHLVL", "5", "print -r -- $SHLVL").expect("SHLVL import hung");
    assert_eq!(
        got.trim(),
        "6",
        "SHLVL must be the imported value plus one (Src/params.c:949); \
         a bare `1` means the import's own store was deferred past the increment"
    );
    assert_eq!(got, zsh_with_env("SHLVL", "5", "print -r -- $SHLVL"));
}

/// Every other integer special goes through the identical dispatch, so a
/// regression that re-couples the guard to the setfn would silence these too.
/// They are cheap and they are the rest of the import loop's PM_INTEGER
/// surface.
#[test]
fn the_other_imported_integer_specials_survive_the_deferral() {
    for (name, value) in [
        ("HISTSIZE", "4321"),
        ("SAVEHIST", "77"),
        ("KEYTIMEOUT", "55"),
        ("FUNCNEST", "9"),
        ("LISTMAX", "3"),
        ("MAILCHECK", "11"),
    ] {
        assert_import_completes(name, value);
    }
}

/// `Src/utils.c:2452-2461` — the import parses with `zstrtol_underscore(val,
/// &ptr, 0, 1)`, base 0, so `0x10` is 16. Pinned because it exercises the same
/// `$COLUMNS` setfn through a value that is neither zero nor a plain decimal.
#[test]
fn a_based_geometry_literal_imports_and_completes() {
    assert_import_completes("COLUMNS", "0x10");
}

/// The scalar half of the same loop. `$HOME` and `$TERM` have cached-storage
/// setfns (`homesetfn`, `termsetfn`) that were also being dispatched under the
/// guard; `termsetfn` reaches terminal setup, which is exactly the shape of
/// callee that reads parameters. Their values must survive being queued.
#[test]
fn the_cached_storage_scalar_specials_survive_the_deferral() {
    assert_import_completes("HOME", "/tmp");
    assert_import_completes("TERM", "xterm");
    assert_import_completes("TERMINFO_DIRS", "/x");
}
