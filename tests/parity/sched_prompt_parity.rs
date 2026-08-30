//! `zsh/sched` prompt-loop parity — does a due `sched` entry actually
//! RUN when the shell returns to the prompt?
//!
//! `sched` is not a timer thread. C arms `SIGALRM` and, at the top of
//! the input loop, `preprompt()` (`Src/init.c`) walks the schedule list
//! and executes every entry whose time has passed, then unlinks it
//! (`Src/Modules/sched.c`). A shell that only STORES entries looks
//! completely healthy from a script — `sched +5 …; sched` lists it, the
//! builtin returns 0 — and silently never runs any of them.
//!
//! Why this matters far beyond `sched` itself: **zinit's turbo mode is
//! built on it.** Every `zinit ice wait'0a'` plugin is deferred to
//! `@zinit-scheduler`, which is driven from the prompt. If due entries
//! never fire, the entire turbo set never loads — in this repo's daily
//! driver config that is zsh-autosuggestions, zsh-syntax-highlighting,
//! history-substring-search, and the `atload'… zpwrBindZstyle'` that
//! registers ~160 of the 204 `zstyle` statements. Completion then runs
//! under a config the user never wrote: no `list-prompt` (so a long list
//! asks "do you wish to see all N possibilities?" instead of paging), no
//! `format`, no `descriptions`.
//!
//! Measured in a PTY with the real config: zsh reaches 204 zstyles and 2
//! loaded plugins, zshrs 43 and 0.
//!
//! The probe cannot be a plain `-c` script — there is no prompt in one.
//! Both shells drive an interactive copy of THEMSELVES through
//! `zsh/zpty` (the same mechanism zsh's own `Test/Y*.ztst` completion
//! tests use), so the comparison is engine-to-engine with no external
//! harness. The verdict is a single boolean, because the two shells'
//! raw PTY bytes legitimately differ (prompt escapes, OSC sequences,
//! the PROMPT_SP marker).
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

/// The driver, run by whichever shell is under test. `$UNDER_TEST` is
/// that same shell, so the interactive loop being measured is its own.
///
/// The marker is assembled at RUN time (`SCHEDM${:-}ARK`) so the
/// terminal's ECHO of the lines typed into the pty can never be
/// mistaken for the scheduled command's OUTPUT — without that, the
/// echoed `sched +0 print SCHEDMARK` alone would satisfy the match and
/// the probe would report success in a shell that fires nothing.
const DRIVER: &str = r#"
zmodload zsh/zpty || { print "NOZPTY"; return 0 }
zpty -b w $UNDER_TEST -f -i || { print "NOZPTY"; return 0 }
sleep 3
zpty -w w 'PS1="RDY> "'
zpty -w w 'zmodload zsh/sched'
zpty -w w 'zzfire(){ print SCHEDM${:-}ARK }'
zpty -w w 'sched +0 zzfire'
zpty -w w 'print TURN1'
sleep 2
zpty -w w 'print TURN2'
sleep 2
local out all=
integer i=0
while (( i++ < 40 )); do
  if zpty -r -t w out 2>/dev/null; then
    all+="$out"
  else
    sleep 0.1
  fi
done
zpty -d w
if [[ $all == *SCHEDMARK* ]]; then print "SCHED_FIRES=yes"; else print "SCHED_FIRES=no"; fi
"#;

/// Returns `Some(true)` when a due `sched` entry ran at the next
/// prompt, `Some(false)` when it did not, `None` when `zsh/zpty` was
/// unavailable so the cell proves nothing either way.
fn sched_fires(shell: &Path, zshrs: bool) -> Option<bool> {
    let mut cmd = Command::new(shell);
    if zshrs {
        cmd.arg("--zsh");
        // The native autosuggest/highlight ports paint history into the
        // buffer of the inner shell; that is not what this measures.
        cmd.env("ZSHRS_NATIVE_ZLE_FX", "0");
    }
    let out = cmd
        .args(["-f", "-c", DRIVER])
        .env("UNDER_TEST", shell)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke shell");
    let text = String::from_utf8_lossy(&out.stdout);
    if text.contains("NOZPTY") {
        return None;
    }
    if text.contains("SCHED_FIRES=yes") {
        Some(true)
    } else if text.contains("SCHED_FIRES=no") {
        Some(false)
    } else {
        panic!(
            "driver produced no verdict from {}:\nstdout={text:?}\nstderr={:?}",
            shell.display(),
            String::from_utf8_lossy(&out.stderr)
        )
    }
}

/// zshrs gap: a due `sched` entry is never executed. It is registered
/// (the `sched` listing still shows it, with a timestamp in the past,
/// after several prompts have gone by) but the prompt loop never runs
/// it — where zsh prints the scheduled command's output just before the
/// next prompt and unlinks the entry.
///
/// Observed, twice in a row on each side:
///
///     zsh    SCHED_FIRES=yes
///     zshrs  SCHED_FIRES=no
///
/// Fix location: the port's equivalent of C's `preprompt()` schedule
/// walk (`Src/init.c` → `Src/Modules/sched.c`). Drop the `#[ignore]`
/// when it lands; the test then becomes the regression pin.
#[test]
#[ignore = "zshrs gap: a due `sched` entry never runs at the prompt (blocks zinit turbo)"]
fn a_due_sched_entry_runs_at_the_next_prompt() {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let reference = sched_fires(Path::new(zsh_path()), false);
    let Some(reference) = reference else {
        eprintln!("skip: zsh/zpty unavailable");
        return;
    };
    let under_test = sched_fires(&zshrs_bin(), true);
    let Some(under_test) = under_test else {
        eprintln!("skip: zsh/zpty unavailable in zshrs");
        return;
    };
    assert!(
        reference,
        "reference zsh did not fire a due sched entry — the probe itself is broken, \
         not the shell under test"
    );
    assert_eq!(
        reference, under_test,
        "a due `sched` entry must run when the shell returns to the prompt \
         (zsh fired it, zshrs did not); this is what zinit turbo's \
         `wait'0a'` deferral rides on"
    );
}
