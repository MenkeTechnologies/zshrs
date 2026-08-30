//! Shared harness for INTERACTIVE parity tests.
//!
//! A `-c` script can never observe the line editor or the prompt loop:
//! there is no terminal and no prompt. `zsh/zpty` gives both shells one
//! without any external harness — each drives an interactive copy of
//! ITSELF, which is the same mechanism zsh's own `Test/Y*.ztst`
//! completion tests use.
//!
//! Three rules make that reliable, and every helper here enforces one
//! of them:
//!
//! 1. **Verdicts are BOOLEANS.** The two shells' raw PTY bytes
//!    legitimately differ — prompt escapes, OSC sequences, the
//!    PROMPT_SP marker — so a byte comparison would only ever measure
//!    cosmetics. A driver prints `KEY=yes` / `KEY=no` and nothing else
//!    is compared.
//! 2. **Markers are assembled at RUN time**, e.g. `print FOOM${:-}ARK`.
//!    The terminal echoes every line typed into the pty, so a literal
//!    marker in the typed text would satisfy the match in a shell that
//!    executed nothing at all.
//! 3. **The REFERENCE is asserted first.** `assert_same_verdict` fails
//!    loudly when zsh itself did not exhibit the behaviour, so a probe
//!    broken by a timing change reports itself instead of passing
//!    green.
//!
//! One more rule that lives in the drivers rather than here: keep a
//! probe that can WEDGE the inner shell in its own session. An
//! unhonoured `$TMOUT` kills it, and every later probe in a shared
//! session then reports "no" for the wrong reason.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

pub fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

pub fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Boilerplate every driver starts with: open a pty on `$UNDER_TEST`
/// (the shell running the driver, so the loop under test is its own),
/// let it reach a prompt, and give it a prompt with no escapes in it.
pub const OPEN: &str = r#"
zmodload zsh/zpty || { print "NOZPTY"; return 0 }
zpty -b w $UNDER_TEST -f -i || { print "NOZPTY"; return 0 }
sleep 3
zpty -w w 'PS1="RDY> "'
"#;

/// Drain whatever the pty has produced into `$all`, then close it.
pub const DRAIN: &str = r#"
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
/// interactive probe here is measuring.
pub fn drive(shell: &Path, zshrs: bool, driver: &str) -> String {
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
/// `None` when `zsh/zpty` was unavailable, so the cell proves nothing
/// either way.
pub fn verdict(shell: &Path, zshrs: bool, driver: &str, key: &str) -> Option<bool> {
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
pub fn assert_same_verdict(driver: &str, key: &str, what: &str) {
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
    assert_eq!(reference, under_test, "{what}: zsh did it, zshrs did not");
}
