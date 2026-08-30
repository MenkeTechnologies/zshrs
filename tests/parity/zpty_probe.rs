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

/// Drain whatever the pty has produced into `$all`, close the pty, and
/// STRIP THE ESCAPE SEQUENCES.
///
/// The stripping is not cosmetic. A shell redrawing the command line
/// interleaves CSI sequences with the text it is redrawing, so the
/// bytes for `print fxa1` can arrive as `print` + `\e[…m` + ` fxa1`.
/// A driver matching the literal string then reports "no" while the
/// transcript, read by a human through an escape-stripping pager, plainly
/// shows the text — which is exactly how a correct probe was mistaken
/// for a shell bug while these were being written. Strip once here so
/// every driver matches against what was DISPLAYED.
///
/// Carriage returns are deliberately KEPT: they are the only thing
/// separating one redraw of the line from the next, and folding them
/// away would let two unrelated fragments match as one string.
pub const DRAIN: &str = r#"
local out all=
integer i=0
while (( i++ < 60 )); do
  if zpty -r -t w out 2>/dev/null; then all+="$out"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
setopt extended_glob
all="${all//$'\e'\[[0-9;?]#[a-zA-Z]/}"
all="${all//$'\e'\][0-9]#;[^$'\a'$'\e']#($'\a'|$'\e'\\)/}"
all="${all//$'\e'[()][A-Za-z0-9]/}"
"#;

/// Wrap `s` in single quotes for embedding in a driver script,
/// escaping any single quote the standard way (`'` → `'\''`).
///
/// Drivers hand setup lines to the inner shell as
/// `zpty -w w '<setup>'`, so a setup containing its own single quotes —
/// `PS1=$'A\nB> '` is the common one — silently TERMINATES the wrapper
/// and the rest of the line is parsed as something else entirely. Two
/// multiline-prompt cases failed at the reference shell for exactly
/// that reason before this existed.
pub fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

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
        // cargo runs tests with no controlling terminal, so `$TERM` may be
        // absent or `dumb`. ZLE then takes its TERM_UNKNOWN path — single
        // line, no cursor addressing — and a probe would be pinning that
        // degraded mode instead of the editor people use. Pin a normal
        // terminal for both shells.
        .env("TERM", "xterm-256color")
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke shell");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `Some(verdict)` from a driver that prints `KEY=yes` / `KEY=no`;
/// `None` when `zsh/zpty` was unavailable, so the cell proves nothing
/// either way.
pub fn verdict(shell: &Path, zshrs: bool, driver: &str, key: &str) -> Option<bool> {
    probe(shell, zshrs, driver, key).0
}

/// The verdict plus the raw driver output behind it. A `false` from the
/// REFERENCE shell means the probe is broken rather than the shell, and
/// the only way to see how is to look at what the pty actually said —
/// so keep it and hand it to the assertion.
pub fn probe(shell: &Path, zshrs: bool, driver: &str, key: &str) -> (Option<bool>, String) {
    let text = drive(shell, zshrs, driver);
    if text.contains("NOZPTY") {
        return (None, text);
    }
    if text.contains(&format!("{key}=yes")) {
        (Some(true), text)
    } else if text.contains(&format!("{key}=no")) {
        (Some(false), text)
    } else {
        panic!(
            "driver produced no `{key}` verdict from {}:\n{text:?}",
            shell.display()
        )
    }
}

/// Setup line installing `dumpbuf`, a widget that writes the LINE
/// EDITOR'S OWN STATE to `$OUTFILE` instead of leaving it to be read
/// off the screen.
///
/// This is a strictly stronger verdict than matching the transcript: a
/// redraw interleaves escapes with text and overwrites earlier output,
/// so screen matching can only ever answer "did this string appear".
/// `$BUFFER` and `$CURSOR` read from inside a widget are exact, which
/// makes off-by-one cursor bugs visible — the class that screen
/// matching structurally cannot see.
///
/// Bound in `main` AND `vicmd`, because a vi-mode probe dumps from
/// command mode and an unbound key there silently produces nothing.
pub const DUMP_WIDGET: &str = concat!(
    r#"zpty -w w 'dumpbuf(){ print -r -- "BUF=[$BUFFER] CUR=[$CURSOR]" >! $OUTFILE }; "#,
    r#"zle -N dumpbuf; bindkey "^X^G" dumpbuf'"#,
    "\n",
    r#"zpty -w w 'bindkey -M vicmd "^X^G" dumpbuf'"#,
);

/// Same widget, but dumping an ARBITRARY expression — `$KEYMAP`,
/// `$LBUFFER`/`$RBUFFER`, `$PREBUFFER`, `$LASTWIDGET`, `$NUMERIC` — so
/// a probe can measure whichever piece of editor state it is about
/// rather than only the buffer and cursor.
///
/// `expr` is inserted into `print -r -- <expr>` inside the widget, so
/// it must be a shell word: `"KM=[$KEYMAP]"`, quotes included.
pub fn dump_widget(expr: &str) -> String {
    format!(
        concat!(
            r#"zpty -w w 'dumpbuf(){{ print -r -- {expr} >! $OUTFILE }}; "#,
            r#"zle -N dumpbuf; bindkey "^X^G" dumpbuf'"#,
            "\n",
            r#"zpty -w w 'bindkey -M vicmd "^X^G" dumpbuf'"#,
            "\n",
            r#"zpty -w w 'bindkey -M viins "^X^G" dumpbuf'"#,
        ),
        expr = expr
    )
}

/// Keystrokes that fire `dumpbuf`.
pub const DUMP_KEY: &str = "zpty -w -n w $'\\C-x\\C-g'\nsleep 2";

/// Read back what `dumpbuf` wrote, having run `driver` under `shell`.
///
/// The driver MUST drain the pty before reading the file: an inner
/// shell whose output buffer fills blocks on write, and the widget then
/// never runs at all. That failure looks identical to "the key was not
/// bound" — empty output from both shells, which reads as agreement.
fn dump(shell: &Path, zshrs: bool, driver: &str, tag: &str) -> String {
    // The path has to be unique per CALL, not per shell: cargo runs the
    // test binary multi-threaded, and two cases sharing one file race —
    // a `$` motion case once read back the `dw` case's buffer and
    // reported a divergence that did not exist.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let out_path = std::env::temp_dir().join(format!(
        "zshrs-parity-dump-{}-{}-{tag}.txt",
        std::process::id(),
        n
    ));
    let _ = std::fs::remove_file(&out_path);
    let mut cmd = Command::new(shell);
    if zshrs {
        cmd.arg("--zsh");
        cmd.env("ZSHRS_NATIVE_ZLE_FX", "0");
    }
    let out = cmd
        .args(["-f", "-c", driver])
        .env("UNDER_TEST", shell)
        .env("OUTFILE", &out_path)
        .env("TERM", "xterm-256color")
        // Pin a UTF-8 locale so the multibyte cases measure CHARACTER
        // indices rather than whatever the ambient locale implies. Both
        // shells get the same value, so a host without this locale
        // degrades identically on both sides rather than diverging.
        .env("LC_ALL", "en_US.UTF-8")
        .env("LANG", "en_US.UTF-8")
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke shell");
    let _ = out;
    std::fs::read_to_string(&out_path).unwrap_or_default().trim().to_string()
}

/// Compare the editor state both shells dumped, refusing to pass when
/// the REFERENCE dumped nothing — an unbound key, a wedged inner shell
/// or an undrained pty all produce empty on both sides, and that is
/// false agreement rather than parity.
pub fn assert_same_dump(driver: &str, what: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    // An EMPTY dump means the widget never ran — a keystroke dropped
    // while the inner shell was still redrawing, which this box does
    // under build load. That is the probe failing to take a
    // measurement, not a shell disagreeing, so it is retried ONCE.
    //
    // A mismatch between two NON-EMPTY dumps is never retried. Retrying
    // a real disagreement is how a pin quietly turns into a
    // rubber stamp, and the whole point of these is to fail when the
    // shells differ.
    let mut reference = dump(Path::new(zsh_path()), false, driver, "zsh");
    let mut under_test = dump(&zshrs_bin(), true, driver, "zshrs");
    if reference.is_empty() || under_test.is_empty() {
        reference = dump(Path::new(zsh_path()), false, driver, "zsh-retry");
        under_test = dump(&zshrs_bin(), true, driver, "zshrs-retry");
    }
    assert!(
        !reference.is_empty(),
        "reference zsh dumped no editor state for `{what}` — the probe is broken, \
         not the shell under test.\n--- driver ---\n{driver}"
    );
    assert!(
        !under_test.is_empty(),
        "zshrs dumped no editor state for `{what}` while zsh reported \
         `{reference}` — the widget never ran.\n--- driver ---\n{driver}"
    );
    assert_eq!(
        reference, under_test,
        "{what}\n--- zsh ---\n{reference}\n--- zshrs ---\n{under_test}"
    );
}

/// Compare one boolean probe across the two shells, refusing to pass
/// when the reference shell did not exhibit the behaviour at all.
pub fn assert_same_verdict(driver: &str, key: &str, what: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let (reference, reference_text) = probe(Path::new(zsh_path()), false, driver, key);
    let Some(reference) = reference else {
        eprintln!("skip: zsh/zpty unavailable");
        return;
    };
    assert!(
        reference,
        "reference zsh did not exhibit `{what}` — the probe itself is broken, \
         not the shell under test.\n--- driver ---\n{driver}\n--- zsh said ---\n{reference_text}"
    );
    let Some(under_test) = verdict(&zshrs_bin(), true, driver, key) else {
        eprintln!("skip: zsh/zpty unavailable in zshrs");
        return;
    };
    assert_eq!(reference, under_test, "{what}: zsh did it, zshrs did not");
}
