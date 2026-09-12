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

// ---------------------------------------------------------------------------
// The three dispatch sites 267ec6ea73 left open.
//
// That commit fixed the sites that could hang and deliberately left three that
// reached only a leaf callee. Two of the three turned out not to: the callee
// they reach, `unsetparam_pm`, calls `delenv` on an exported parameter
// (`Src/params.c:3872`), and this port's `delenv` re-takes `paramtab().write()`.
// One of those two — `bin_ztie` — had no caller-side precondition standing in
// the way and hung outright.
// ---------------------------------------------------------------------------

/// Run `script` under zshrs with no extra environment, under the same deadline.
fn run_zshrs(script: &str) -> Result<String, Duration> {
    run_zshrs_with_env("ZSHRS_LOCKREST_PROBE", "1", script)
}

/// `bin_ztie` unset any existing parameter before taking over its name
/// (`Src/Modules/db_gdbm.c:157` `if (unsetparam_pm(tied_param, 0, 1)) return 1;`)
/// while holding the paramtab write guard. For a parameter imported from the
/// environment — which is where `pm.env` gets set, `Src/params.c:907-914` —
/// `unsetparam_pm` took its `if (pm->env) delenv(pm)` arm (c:3872), and
/// `delenv` re-took the same non-reentrant write lock.
///
/// `sample` on the hung process, 2660 of 2660 in `semaphore_wait_trap`:
///
/// ```text
/// bin_ztie -> unsetparam_pm -> delenv -> RwLock::write -> lock_contended
/// ```
///
/// The tie itself is expected to FAIL here (this build reports "GDBM support
/// not compiled in"), and that is fine: the unset at c:157 runs first, which is
/// the part that hung. What is pinned is that the shell answers at all.
///
/// No oracle comparison: `/opt/homebrew/bin/zsh` is built without
/// `zsh/db/gdbm`, so there is nothing to compare against.
#[test]
fn ztie_over_an_imported_parameter_completes() {
    let script = "zmodload zsh/db/gdbm 2>/dev/null || { print -r -- NOMODULE; return }\n\
                  ztie -d db/gdbm -f ${TMPDIR:-/tmp}/zshrs_lockrest_$$.gdbm MYIMPORTED 2>/dev/null\n\
                  print -r -- REACHED";
    let got = match run_zshrs_with_env("MYIMPORTED", "hello", script) {
        Ok(out) => out,
        Err(elapsed) => panic!(
            "DEADLOCK: `MYIMPORTED=hello zshrs -f -c '<ztie>'` never finished ({elapsed:?}).\n\
             bin_ztie dispatched unsetparam_pm while holding the paramtab write guard; the \
             parameter came from the environment, so unsetparam_pm took its delenv arm \
             (Src/params.c:3872) and delenv re-took the same non-reentrant lock.\n\
             Diagnose with `sample <pid>`: every sample will be in semaphore_wait_trap under \
             bin_ztie -> unsetparam_pm -> delenv -> RwLock::write."
        ),
    };
    assert!(
        got.contains("REACHED") || got.contains("NOMODULE"),
        "expected the shell to get past the ztie, got {got:?}"
    );
}

/// `setopt sunkeyboardhack` is `Src/options.c:874-877`, whose whole body is
/// `keyboardhackchar = (value ? '`' : '\0');` — a plain assignment to a global,
/// with no paramtab lookup and no gsu dispatch in it at all. This port used to
/// look `KEYBOARD_HACK` up and call `keyboardhacksetfn` under the write guard.
/// It could not hang as written, only because neither of that setfn's `zwarn`
/// arms (c:5041, c:5045) is reachable from a one-byte ASCII literal — and
/// `zwarn` is not a leaf: zwarning -> zleentry(ZLE_CMD_TRASH) -> zrefresh ->
/// `getaparam("zle_highlight")` -> the same lock.
///
/// The values are the oracle's own answers.
#[test]
fn sunkeyboardhack_sets_the_hack_character() {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    for script in [
        "setopt sunkeyboardhack; printf '[%s]' \"$KEYBOARD_HACK\"",
        "unsetopt sunkeyboardhack; printf '[%s]' \"$KEYBOARD_HACK\"",
        "setopt sunkeyboardhack; unsetopt sunkeyboardhack; printf '[%s]' \"$KEYBOARD_HACK\"",
        // A user-set value is overwritten by the option, per c:876's
        // unconditional assignment.
        "KEYBOARD_HACK=';'; setopt sunkeyboardhack; printf '[%s]' \"$KEYBOARD_HACK\"",
        // The option's own state must still be recorded (c:878 `new_opts[optno]
        // = value`), which is downstream of the arm this change rewrote.
        "setopt sunkeyboardhack; [[ -o sunkeyboardhack ]] && print -n on",
    ] {
        let got = match run_zshrs(script) {
            Ok(out) => out,
            Err(elapsed) => panic!(
                "DEADLOCK: `zshrs -f -c {script:?}` never finished ({elapsed:?}). \
                 setopt's SUNKEYBOARDHACK arm dispatched a gsu setfn under the paramtab \
                 write guard and the setfn reached back into the table."
            ),
        };
        let want = zsh_with_env("ZSHRS_LOCKREST_PROBE", "1", script);
        assert_eq!(got, want, "`{script}`:\n  zsh   {want:?}\n  zshrs {got:?}");
    }
}

/// `restore_params` (`Src/exec.c:4464`) unsets every name a `VAR=val cmd`
/// prefix touched, via `unsetparam_pm(pm, 0, 0)` at c:4474 — which this port
/// also ran under the write guard, behind a `drop(tab)` that the very next line
/// undid. Both of `unsetparam_pm`'s non-leaf arms happen to be disarmed from
/// here, and neither by the callee: the `zerr` arm by c:4473 clearing
/// PM_READONLY, the `delenv` arm by `save_params` having already run
/// `if (pm->env) delenv(pm)` over the same name on the way in (c:4423-4424).
///
/// So this pins the behaviour rather than a hang: the prefix value must be
/// visible to the command and the previous binding must come back after it,
/// with the oracle's answers.
#[test]
fn a_prefix_assignment_restores_the_previous_binding() {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    for script in [
        // Name did not exist: restore_params must remove it (c:4470-4476).
        "f(){ print -n \"in=$V|\"; }; V=2 f; print -n \"out=${V-gone}\"",
        // Name existed and was exported: the saved copy comes back (c:4478).
        "export V=1; f(){ print -n \"in=$V|\"; }; V=2 f; print -n \"out=$V\"",
        // allexport makes the prefix-created parameter itself exported, which
        // is the state that decides whether unsetparam_pm reaches delenv.
        "setopt allexport; f(){ print -n \"in=$W|\"; }; W=2 f; print -n \"out=${W-gone}\"",
        // A builtin rather than a shell function — the other caller of the
        // same save/restore pair.
        "V=2 typeset -p V; print -n \"out=${V-gone}\"",
    ] {
        let got = match run_zshrs(script) {
            Ok(out) => out,
            Err(elapsed) => panic!(
                "DEADLOCK: `zshrs -f -c {script:?}` never finished ({elapsed:?}). \
                 restore_params dispatched unsetparam_pm under the paramtab write guard \
                 and it reached delenv, which re-takes the same lock."
            ),
        };
        let want = zsh_with_env("ZSHRS_LOCKREST_PROBE", "1", script);
        assert_eq!(got, want, "`{script}`:\n  zsh   {want:?}\n  zshrs {got:?}");
    }
}
