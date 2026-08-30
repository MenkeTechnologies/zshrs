//! Interactive prompt-loop parity — the work a shell does *between*
//! commands, which no `-c` script can observe because a `-c` script has
//! no prompt.
//!
//! C splits that work in two:
//!
//!   * **Hook functions** run from `preprompt()` / `execcmd` directly —
//!     `precmd`, `preexec`, `periodic` (gated on `$PERIOD`), `chpwd`.
//!   * **SIGALRM-driven work** — `preprompt()` (`Src/init.c`) walks the
//!     `sched` list and runs every entry whose time has passed
//!     (`Src/Modules/sched.c`), and `$TMOUT` arms the alarm that fires
//!     `TRAPALRM` while the line editor sits idle.
//!
//! Measured here: zshrs runs the first group and none of the second. A
//! shell in that state looks completely healthy from a script — `sched
//! +5 …; sched` lists the entry, the builtin returns 0, `TMOUT=1`
//! assigns fine — and silently never executes any of it.
//!
//! Why it matters far beyond `sched` itself: **zinit's turbo mode is
//! built on it.** Every `zinit ice wait'0a'` plugin is deferred to
//! `@zinit-scheduler`, which is driven from the prompt. If due entries
//! never fire, the whole turbo set never loads — in this repo's daily
//! driver that is zsh-autosuggestions, zsh-syntax-highlighting,
//! history-substring-search, and the `atload'… zpwrBindZstyle'` that
//! registers ~160 of the 204 `zstyle` statements. Completion then runs
//! under a config the user never wrote: no `list-prompt` (so a long
//! list asks "do you wish to see all N possibilities?" instead of
//! paging), no `format`, no `descriptions`. Measured in a PTY with the
//! real config: zsh reaches 204 zstyles and 2 loaded plugins, zshrs 43
//! and 0.
//!
//! **Method.** Both shells drive an interactive copy of THEMSELVES
//! through `zsh/zpty` — the same mechanism zsh's own `Test/Y*.ztst`
//! completion tests use — so the comparison is engine-to-engine with no
//! external harness. Three rules make that reliable:
//!
//!   1. Every verdict is a BOOLEAN. The two shells' raw PTY bytes
//!      legitimately differ (prompt escapes, OSC sequences, the
//!      PROMPT_SP marker), so byte comparison would only ever measure
//!      cosmetics.
//!   2. Markers are assembled at RUN time (`print FOOM${:-}ARK`). The
//!      terminal echoes every line typed into the pty, so a literal
//!      marker in the typed text would satisfy the match in a shell
//!      that fires nothing at all.
//!   3. Each test asserts the REFERENCE shell fired before comparing.
//!      A probe broken by a timing change then reports itself instead
//!      of passing green.
//!
//! Probes are kept in separate pty sessions. `TMOUT` in particular
//! cannot share one: if `TRAPALRM` is not honoured the timeout kills
//! the inner shell, and every later probe in that session silently
//! reports "no".
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

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

/// Boilerplate every driver shares: open a pty on `$UNDER_TEST` (the
/// shell running this script, so the loop under test is its own), let
/// it reach a prompt, and give it a prompt with no escapes in it.
const OPEN: &str = r#"
zmodload zsh/zpty || { print "NOZPTY"; return 0 }
zpty -b w $UNDER_TEST -f -i || { print "NOZPTY"; return 0 }
sleep 3
zpty -w w 'PS1="RDY> "'
"#;

/// Drain whatever the pty has produced into `$all` and close it.
const DRAIN: &str = r#"
local out all=
integer i=0
while (( i++ < 60 )); do
  if zpty -r -t w out 2>/dev/null; then all+="$out"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
"#;

/// Run `driver` under `shell` and hand back its stdout. `zshrs` also
/// gets `ZSHRS_NATIVE_ZLE_FX=0`: the native autosuggest/highlight ports
/// paint history into the INNER shell's buffer, which is not what any
/// of these measure.
fn drive(shell: &Path, zshrs: bool, driver: &str) -> String {
    let mut cmd = Command::new(shell);
    if zshrs {
        cmd.arg("--zsh");
        cmd.env("ZSHRS_NATIVE_ZLE_FX", "0");
    }
    let out = cmd
        .args(["-f", "-c", driver])
        .env("UNDER_TEST", shell)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke shell");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `Some(verdict)` from a driver that prints `KEY=yes` / `KEY=no`;
/// `None` when `zsh/zpty` was unavailable, so the cell proves nothing.
fn verdict(shell: &Path, zshrs: bool, driver: &str, key: &str) -> Option<bool> {
    let text = drive(shell, zshrs, driver);
    if text.contains("NOZPTY") {
        return None;
    }
    if text.contains(&format!("{key}=yes")) {
        Some(true)
    } else if text.contains(&format!("{key}=no")) {
        Some(false)
    } else {
        panic!(
            "driver produced no `{key}` verdict from {}:\n{text:?}",
            shell.display()
        )
    }
}

/// Compare one boolean probe across the two shells, refusing to pass
/// when the reference shell did not exhibit the behaviour at all.
fn assert_same_verdict(driver: &str, key: &str, what: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let Some(reference) = verdict(Path::new(zsh_path()), false, driver, key) else {
        eprintln!("skip: zsh/zpty unavailable");
        return;
    };
    let Some(under_test) = verdict(&zshrs_bin(), true, driver, key) else {
        eprintln!("skip: zsh/zpty unavailable in zshrs");
        return;
    };
    assert!(
        reference,
        "reference zsh did not exhibit `{what}` — the probe itself is broken, \
         not the shell under test"
    );
    assert_eq!(
        reference, under_test,
        "{what}: zsh did it, zshrs did not"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Hook functions — these DO run, and must keep running
// ═══════════════════════════════════════════════════════════════════════

/// `precmd` before each prompt, `preexec` before each command,
/// `periodic` every `$PERIOD` seconds at the prompt, `chpwd` on a
/// directory change. One session, because none of them can wedge the
/// inner shell the way `TMOUT` can.
const HOOKS: &str = r#"
zpty -w w 'precmd(){ print PRECMDM${:-}ARK }'
zpty -w w 'print T1'
sleep 1
zpty -w w 'preexec(){ print PREEXECM${:-}ARK }'
zpty -w w 'print T2'
sleep 1
zpty -w w 'periodic(){ print PERIODICM${:-}ARK }; PERIOD=1'
sleep 2
zpty -w w 'print T3'
sleep 2
zpty -w w 'print T4'
sleep 1
zpty -w w 'unfunction periodic; unset PERIOD'
zpty -w w 'chpwd(){ print CHPWDM${:-}ARK }'
sleep 1
zpty -w w 'cd /tmp'
sleep 2
"#;

fn hooks_driver() -> String {
    format!(
        "{OPEN}{HOOKS}{DRAIN}
for m in PRECMD PREEXEC PERIODIC CHPWD; do
  if [[ $all == *${{m}}MARK* ]]; then print \"$m=yes\"; else print \"$m=no\"; fi
done
"
    )
}

#[test]
fn precmd_runs_before_each_prompt() {
    assert_same_verdict(&hooks_driver(), "PRECMD", "precmd ran before a prompt");
}

#[test]
fn preexec_runs_before_each_command() {
    assert_same_verdict(&hooks_driver(), "PREEXEC", "preexec ran before a command");
}

/// `periodic` is the OTHER preprompt-timed hook, and it works — which
/// is what localises the two failures below to the SIGALRM path rather
/// than to `preprompt()` as a whole.
#[test]
fn periodic_runs_on_its_period() {
    assert_same_verdict(&hooks_driver(), "PERIODIC", "periodic ran on its $PERIOD");
}

#[test]
fn chpwd_runs_on_a_directory_change() {
    assert_same_verdict(&hooks_driver(), "CHPWD", "chpwd ran after cd");
}

// ═══════════════════════════════════════════════════════════════════════
// SIGALRM-driven work — neither of these runs in zshrs
// ═══════════════════════════════════════════════════════════════════════

/// zshrs gap: a due `sched` entry is never executed. It IS registered —
/// the `sched` listing still shows it, timestamped in the past, after
/// several prompts have gone by — but the prompt loop never runs it,
/// where zsh prints the scheduled command's output just before the next
/// prompt and unlinks the entry. Deterministic, 2/2 runs each side.
///
/// Fix location: the port's equivalent of C's `preprompt()` schedule
/// walk (`Src/init.c` → `Src/Modules/sched.c`). Drop the `#[ignore]`
/// when it lands; the test then becomes the regression pin.
#[test]
#[ignore = "zshrs gap: a due `sched` entry never runs at the prompt (blocks zinit turbo)"]
fn a_due_sched_entry_runs_at_the_next_prompt() {
    let driver = format!(
        "{OPEN}
zpty -w w 'zmodload zsh/sched'
zpty -w w 'zzfire(){{ print SCHEDM${{:-}}ARK }}'
zpty -w w 'sched +0 zzfire'
zpty -w w 'print TURN1'
sleep 2
zpty -w w 'print TURN2'
sleep 2
{DRAIN}
if [[ $all == *SCHEDMARK* ]]; then print \"SCHED=yes\"; else print \"SCHED=no\"; fi
"
    );
    assert_same_verdict(&driver, "SCHED", "a due sched entry ran at the prompt");
}

/// zshrs gap, same family: `$TMOUT` arms no alarm, so `TRAPALRM` never
/// runs while the editor is idle. Over five idle seconds zsh fires it
/// twice and zshrs zero times (deterministic, 2/2 runs each side).
///
/// This probe gets its OWN pty session on purpose: with `TRAPALRM`
/// unhonoured, a `TMOUT` that IS honoured would kill the inner shell
/// and make every later probe in a shared session report "no" for the
/// wrong reason — which is exactly what happened while writing these.
#[test]
#[ignore = "zshrs gap: $TMOUT arms no alarm, so TRAPALRM never runs at the prompt"]
fn tmout_drives_trapalrm_while_idle_at_the_prompt() {
    let driver = format!(
        "{OPEN}
zpty -w w 'TRAPALRM(){{ print ALRMM${{:-}}ARK }}'
zpty -w w 'TMOUT=1'
sleep 5
{DRAIN}
if [[ $all == *ALRMMARK* ]]; then print \"ALRM=yes\"; else print \"ALRM=no\"; fi
"
    );
    assert_same_verdict(&driver, "ALRM", "TRAPALRM ran while idle under $TMOUT");
}
