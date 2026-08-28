//! C-style `for ((init; cond; step))` parity for the ARITHMETIC BACKING
//! STORE.
//!
//! zsh evaluates all three sections with the one math evaluator over the
//! one parameter table — `matheval(str)` for init (c:Src/loop.c:77),
//! `val = mathevali(str)` for the condition (c:Src/loop.c:135) and
//! `matheval(str)` for the advance (c:Src/loop.c:191). zshrs used to
//! compile sections without `,`/`$` down a second engine (ArithCompiler
//! → VM slots, pre-loaded via `getmathparam`) whose store desynchronised
//! from the first: `getmathparam` (src/ported/math.rs:222) consults the
//! Rust-only `M_VARIABLES` shadow map before the parameter table, and
//! `matheval` (src/ported/math.rs:4272) leaves that map populated when it
//! returns. A preceding `(( i++ ))` therefore froze the loop counter and
//! `for ((i=1; i<=3; i++))` looped forever.
//!
//! Every case here is a NON-TERMINATION test as much as an output test,
//! so the runners below enforce a wall-clock deadline and kill the child:
//! a regression must fail as "timed out", never hang the suite.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Generous enough that a loaded CI box never trips it, short enough
/// that a genuinely non-terminating loop fails fast.
const DEADLINE: Duration = Duration::from_secs(20);

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

/// Wait for `child`, killing it once `DEADLINE` elapses. Returns `None`
/// on timeout so the caller can report WHICH shell failed to terminate.
fn wait_bounded(mut child: Child, label: &str) -> Option<R> {
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                let out = child.wait_with_output().expect("wait_with_output");
                return Some(R {
                    stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                    exit: status.code().unwrap_or(-1),
                });
            }
            None => {
                if start.elapsed() >= DEADLINE {
                    let _ = child.kill();
                    let _ = child.wait();
                    eprintln!("{label} did not terminate within {DEADLINE:?}");
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn run_zsh(s: &str) -> Option<R> {
    let child = Command::new(zsh_path())
        .args(["-fc", s])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn zsh");
    wait_bounded(child, "zsh")
}

fn run_zshrs(s: &str) -> Option<R> {
    let child = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn zshrs");
    wait_bounded(child, "zshrs")
}

/// Both shells must TERMINATE and agree on stdout + exit status.
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s).unwrap_or_else(|| panic!("zsh itself hung on:\n{s}"));
    let r = run_zshrs(s).unwrap_or_else(|| {
        panic!("zshrs did not terminate (infinite loop) on:\n{s}\nzsh printed: {:?}", z.stdout)
    });
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit-status divergence on:\n{s}");
}

/// Case A of the original report — a `(( ))` command that WRITES the
/// variable, then a C-style loop over that same variable. The write left
/// a stale entry in the math evaluator's shadow map, the loop's own
/// increment went to the parameter table, and the condition kept reading
/// the shadow: `x=1` forever.
#[test]
fn arith_command_write_then_c_style_for_same_var() {
    assert_parity("i=0\n(( i++ ))\nfor ((i = 1; i <= 3; i++)); do echo \"x=$i\"; done");
}

/// Case B — same increment reached through a `while` condition. The
/// while-cond is itself a `(( ))`, and its `matheval` entry cleared the
/// shadow map on the way in, so this case masked the bug.
#[test]
fn while_loop_arith_increment_then_c_style_for() {
    assert_parity(
        "i=0; while (( i < 2 )); do (( i++ )); done; \
         for ((i=1;i<=3;i++)); do echo \"x=$i\"; done",
    );
}

/// Case C — the increment lives in a preceding C-style loop's BODY. The
/// second loop's init (`i=1`) went to the parameter table while its
/// condition still read the shadow's `2`, so it printed `x=2` forever.
#[test]
fn c_style_for_body_arith_increment_then_second_c_style_for() {
    assert_parity(
        "i=0; for ((; i < 2;)); do (( i++ )); done; \
         for ((i=1;i<=3;i++)); do echo \"x=$i\"; done",
    );
}

/// Case D — the same increment written as an arithmetic SUBSTITUTION.
/// `$(( ))` assigns nothing inside the evaluator, so it never seeded the
/// shadow map. Pins the control.
#[test]
fn arith_substitution_increment_then_c_style_for() {
    assert_parity(
        "i=0; for ((; i < 2;)); do i=$((i+1)); done; \
         for ((i=1;i<=3;i++)); do echo \"x=$i\"; done",
    );
}

/// Case E — the loop on its own, with nothing to pollute the store.
#[test]
fn c_style_for_alone() {
    assert_parity("for ((i=1;i<=3;i++)); do echo \"x=$i\"; done");
}

/// Case F — the increment done by a C-style loop's own STEP section,
/// which never touched the math evaluator at all.
#[test]
fn c_style_for_step_increment_then_second_c_style_for() {
    assert_parity(
        "i=0; for ((; i < 2; i++)); do :; done; \
         for ((i=1;i<=3;i++)); do echo \"x=$i\"; done",
    );
}

/// A plain `(( i = N ))` assignment poisons the loop in the other
/// direction: the stale `5` made `i <= 3` false immediately and the loop
/// body never ran (silent wrong answer rather than a hang).
#[test]
fn arith_command_assignment_then_c_style_for_runs_body() {
    assert_parity("(( i = 5 )); for ((i=1;i<=3;i++)); do echo \"x=$i\"; done");
}

/// A `(( ))` in the loop's OWN body assigning the loop variable. This is
/// why clearing the store once before the loop is not enough.
#[test]
fn arith_command_in_body_assigning_loop_var() {
    assert_parity("for ((i=1;i<=3;i++)); do (( i = i )); echo \"x=$i\"; done");
}

/// A `(( ))` in the body assigning a DIFFERENT variable must keep
/// working — the shadow entry is per-name, so this case always passed
/// and would catch an over-broad fix that broke body assignments.
#[test]
fn arith_command_in_body_assigning_other_var() {
    assert_parity("for ((i=1;i<=3;i++)); do (( j = i * 2 )); echo \"x=$i j=$j\"; done");
}

/// c:Src/math.c:1505-1509 — `mathevali` coerces the result to `zlong`
/// before c:Src/loop.c:143 tests it, so a fractional condition truncates
/// toward zero and `0.5` is FALSE. Comparing the printed value against
/// the string "0" made it true.
#[test]
fn fractional_condition_truncates_to_false() {
    assert_parity("for ((;0.5;)); do echo hi; break; done; echo done");
}

/// The same truncation over a float counter — without it the countdown
/// runs past zero forever.
#[test]
fn float_counter_countdown_terminates() {
    assert_parity("f=2.5; for ((;f;f-=1)); do echo \"f=$f\"; done");
}

/// A whole-number float condition is still true (`2.0` → `2`).
#[test]
fn whole_float_condition_is_true() {
    assert_parity("for ((;2.0;)); do echo hi; break; done; echo done");
}

/// The loop BODY reassigning the counter must be seen by the condition
/// and the step — zsh re-reads the parameter every iteration, so this
/// pins that the counter is not cached across iterations.
#[test]
fn body_reassignment_of_counter_is_visible_to_cond_and_step() {
    assert_parity("for ((i=0; i<10; i++)); do echo \"x=$i\"; if (( i == 2 )); then i=8; fi; done");
}

/// Comma sections and `$`-bearing sections were already routed through
/// the math evaluator; keep them pinned now that every section is.
#[test]
fn comma_sections_still_work() {
    assert_parity("for ((i=0,j=5; i<3; i++,j--)); do echo \"$i $j\"; done");
}

#[test]
fn dollar_bearing_condition_still_works() {
    assert_parity("a=(x y z); for ((i=1; i<=$#a; i++)); do echo \"${a[i]}\"; done");
}
