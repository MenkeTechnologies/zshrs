//! #1123 — control transfer out of a loop that lives inside the try arm of a
//! `{ … } always { … }` block, and the `compinit` autoload-stub contract that
//! makes a `zle -C` widget's completion entry point resolvable.
//!
//! Both defects were found together: zsh's `_main_complete` wraps its whole
//! body in `{ … } always { … }` and leaves its completer loop with `break 2`
//! (Completion/Base/Core/_main_complete sh:216), so the compiler bug below
//! abandoned the completion widget — and the calling ZLE widget — halfway
//! through. The compinit half is what made `_main_complete` exist at all.
//!
//! Every assertion is measured against the reference zsh, and nothing here
//! needs a pty, a tty, or a network: the completion side is exercised through
//! `${(k)functions}` rather than through a real Tab keystroke.

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

fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Assert stdout AND exit status match the reference shell.
fn assert_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit divergence on script:\n{script}");
}

// ───────── `break` out of a loop nested inside a try-block ─────────

/// The compiler armed the BREAKS atomic for EVERY `break` compiled inside a
/// try arm, then jumped to the target loop's `loop_exit` — which is reached
/// without passing the post-body drain that would clear the flag again. The
/// next escape check after the loop then read the stale flag as a foreign
/// break and jumped to the always-arm, so the rest of the try body never ran.
mod break_inside_try_block {
    use super::*;

    /// The exact shape at `_main_complete` sh:216, minus the completion:
    /// a `break` whose target loop is entirely inside the try arm must be an
    /// ordinary loop exit. zshrs printed only `ALWAYS`.
    #[test]
    fn break_targets_a_loop_inside_the_try_arm() {
        assert_parity(
            r#"f(){ { for i in 1 2; do print $i; break; done; print AFTER } always { print ALWAYS }; print END }; f"#,
        );
    }

    /// `break 2` out of two nested loops, both still inside the try arm.
    #[test]
    fn break_2_leaves_both_inner_loops_but_not_the_block() {
        assert_parity(
            r#"f(){ { for i in 1 2; do for j in a b; do print $i$j; break 2; done; print INNER; done; print AFTER } always { print ALWAYS }; print END }; f"#,
        );
    }

    /// The complementary case, which the guard must NOT break: the try block
    /// sits inside the loop, so the `break` genuinely leaves the construct and
    /// still has to arm the flag for the enclosing loop to see.
    #[test]
    fn break_that_leaves_the_try_block_still_exits_the_outer_loop() {
        assert_parity(
            r#"f(){ for i in 1 2 3; do { print $i; break } always { print A$i }; print NOTREACHED$i; done; print AFTER }; f"#,
        );
    }

    /// `while` reaches `loop_exit` through the same path as `for`.
    #[test]
    fn break_inside_a_while_loop_in_a_try_arm() {
        assert_parity(
            r#"f(){ { integer i=0; while (( i++ < 3 )); do print $i; break; done; print AFTER } always { print ALWAYS }; print END }; f"#,
        );
    }

    /// A `break` reached through an `if` is compiled at a deeper cmd-stack
    /// depth; the drain must not change the verdict.
    #[test]
    fn break_reached_through_an_if_inside_a_try_arm() {
        assert_parity(
            r#"f(){ { for i in 1 2; do if (( i == 1 )); then break; fi; print $i; done; print AFTER } always { print ALWAYS }; print END }; f"#,
        );
    }

    /// Nested try-blocks each measure against their own loop base, so an
    /// inner-loop break in the inner arm must not escape the outer arm.
    #[test]
    fn nested_try_blocks_each_scope_their_own_break() {
        assert_parity(
            r#"f(){ { { for i in 1 2; do print $i; break; done; print INNER } always { print IA }; print OUTER } always { print OA }; print END }; f"#,
        );
    }

    /// `continue` took the same `SET_CONTINUE` path and was already correct
    /// (its jump lands on the loop's continue target, which drains the flag).
    /// Pinned so the break-side guard is never copied over to it blindly.
    #[test]
    fn continue_inside_a_try_arm_is_unaffected() {
        assert_parity(
            r#"f(){ { for i in 1 2; do print $i; continue; done; print AFTER } always { print ALWAYS }; print END }; f"#,
        );
    }

    /// The two behaviours the guard sits next to and must not disturb:
    /// `exectry` returns the TRY list's status even when the always arm
    /// returns (A01grammar.ztst:723), and ERR_EXIT applies to the whole
    /// construct (BUGS #240).
    #[test]
    fn try_block_status_and_errexit_are_unchanged() {
        assert_parity(r#"f(){ { return 2 } always { return 3 } }; f; print "rc=$?""#);
        assert_parity(r#"setopt err_exit; { false } always { : }; print after"#);
    }

    /// A `break` in the ALWAYS arm targets the loop enclosing the construct —
    /// the arm is compiled with the outer patch lists restored.
    #[test]
    fn break_in_the_always_arm_exits_the_enclosing_loop() {
        assert_parity(
            r#"f(){ for i in 1 2 3; do { print try$i } always { break }; print NOTREACHED; done; print AFTER }; f"#,
        );
    }
}

// ───────── compinit registers an autoload stub per completer ─────────

/// compinit sh:333 (`compdef -na` → `autoload -rUz "$func"`) and sh:541 (the
/// `#autoload` scan arm) leave a stub in `${(k)functions}` for every completer
/// registered. zshrs's cold path published `$_comps` and returned without
/// registering any, so `${(k)functions}` held no `_*` name at all and the
/// `zle -C` widgets bound at sh:556-560 pointed at an undefined function.
mod compinit_autoload_stubs {
    use super::*;

    /// zsh's own Completion tree is the fpath the ztst comptest files use.
    /// Skipped rather than failed when that tree is not checked out, so this
    /// file stays green on a machine without the C sources.
    fn zsh_completion_fpath() -> Option<String> {
        // Same lookup order as `ztst_runner::ztst_zsh_source`, so a CI box
        // points both at the checkout with one variable.
        let root = std::env::var("ZTST_ZSH_SOURCE")
            .map(PathBuf::from)
            .ok()
            .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join("forkedRepos/zsh")))
            .map(|p| p.join("Completion"))
            .filter(|p| p.is_dir())?;
        let root = root.as_path();
        let mut dirs = vec![root.to_path_buf()];
        for top in std::fs::read_dir(root).ok()? {
            let top = top.ok()?.path();
            if !top.is_dir() {
                continue;
            }
            dirs.push(top.clone());
            for sub in std::fs::read_dir(&top).ok()? {
                let sub = sub.ok()?.path();
                if sub.is_dir() {
                    dirs.push(sub);
                }
            }
        }
        Some(
            dirs.iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    /// After a cold `compinit` (no `-C`), the completion entry point and the
    /// completers it dispatches to must all be defined. Measured before the
    /// fix: `$_comps` held 1822 entries while `${(k)functions}` held none.
    #[test]
    fn a_cold_compinit_defines_the_completion_entry_points() {
        let Some(fpath) = zsh_completion_fpath() else {
            return;
        };
        let dump = std::env::temp_dir().join(format!("zshrs-stub-probe-{}", std::process::id()));
        let script = format!(
            r#"fpath=({fpath})
autoload -U compinit
compinit -u -d {dump}
print -r -- "mc=${{+functions[_main_complete]}} cp=${{+functions[_complete]}} nm=${{+functions[_normal]}} ds=${{+functions[_dispatch]}}""#,
            dump = dump.display()
        );
        let r = run_zshrs(&script);
        let _ = std::fs::remove_file(&dump);
        assert_eq!(
            r.stdout.trim(),
            "mc=1 cp=1 nm=1 ds=1",
            "compinit must leave an autoload stub for every completer it registers"
        );
    }

    /// The stub count has to be in the same league as the reference shell's,
    /// not just non-zero — a handful of hardcoded names would satisfy the
    /// test above while still leaving `_tmux`'s `${(M)${(k)functions}:#_tmux-*}`
    /// census (and every other name-derived lookup) empty.
    #[test]
    fn the_stub_set_matches_the_reference_shell() {
        if !zsh_available() {
            return;
        }
        let Some(fpath) = zsh_completion_fpath() else {
            return;
        };
        let script_for = |dump: &str| {
            format!(
                r#"fpath=({fpath})
autoload -U compinit
compinit -u -d {dump}
print -r -- ${{#${{(M)${{(k)functions}}:#_*}}}}"#
            )
        };
        let d1 = std::env::temp_dir().join(format!("zshrs-stub-a-{}", std::process::id()));
        let d2 = std::env::temp_dir().join(format!("zshrs-stub-b-{}", std::process::id()));
        let z: usize = run_zsh(&script_for(&d1.display().to_string()))
            .stdout
            .trim()
            .parse()
            .unwrap_or(0);
        let r: usize = run_zshrs(&script_for(&d2.display().to_string()))
            .stdout
            .trim()
            .parse()
            .unwrap_or(0);
        let _ = std::fs::remove_file(&d1);
        let _ = std::fs::remove_file(&d2);
        assert!(z > 500, "reference shell scan looks wrong: {z} names");
        assert_eq!(
            r, z,
            "zshrs registered {r} completer stubs where zsh registers {z}"
        );
    }
}
