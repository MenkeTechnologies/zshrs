//! `ztest` — shell-level unit test framework.
//!
//! Port of the strykelang unit-test framework
//! (`../strykelang/strykelang/builtins.rs::test_pass/_fail/_skip`,
//!  `builtin_assert_*`, `builtin_test_run`, `builtin_test_skip` +
//!  `../strykelang/strykelang/cli_runners.rs::{run_one_inproc,
//!  run_test_worker_loop, run_tests_pool, WorkerRequest, WorkerResponse}`).
//!
//! All `zassert_*`/`ztest_*` names use the `z*` prefix so they don't
//! collide with POSIX `test`/`[` (which are `BUILTIN_TEST` already at
//! id 33 in fusevm/src/shell_builtins.rs).
//!
//! API surface (mirrors strykelang names with `z`-prefix):
//!
//!   * `zassert_eq A B [MSG]`           — strykelang assert_eq
//!   * `zassert_ne A B [MSG]`           — strykelang assert_ne
//!   * `zassert_ok V [MSG]`             — passes if V is truthy (non-empty / non-"0")
//!   * `zassert_err V [MSG]`            — passes if V is falsy/empty/"0"
//!   * `zassert_true V` / `zassert_false V` — aliases for ok/err
//!   * `zassert_gt|lt|ge|le A B [MSG]`  — numeric comparators
//!   * `zassert_match PATTERN STRING [MSG]`
//!   * `zassert_contains HAYSTACK NEEDLE [MSG]`
//!   * `zassert_near A B [EPS [MSG]]`
//!   * `zassert_dies CMD [MSG]`         — passes if the shell command exits non-zero
//!                                        (shell-native variant of strykelang's
//!                                        assert_dies-on-coderef)
//!   * `ztest_run` (alias `run_tests`)  — print summary, roll counters into totals
//!   * `ztest_skip MSG`                 — mark current assertion skipped (yellow ↷)
//!
//! Runner CLI (wired in `bins/zshrs.rs`):
//!
//!   * `zshrs --ztest [paths...]`       — worker-pool runner (`run_ztests_pool`)
//!   * `zshrs --ztest-worker`           — worker subprocess (`run_ztest_worker_loop`)
//!
//! Test discovery rules match strykelang: `test_*` / `t_*` prefix,
//! `.zsh` / `.sh` / `.zshrs` suffix, under `t/` or `tests/`.

use crate::ported::vm_helper::ShellExecutor;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

/// Canonical name list of every ztest builtin in this module. Used by
/// `extensions/ext_builtins.rs::EXT_BUILTIN_NAMES` and the LSP
/// reflection dump so the IntelliJ Extensions tab + reference docs
/// stay in sync. Kept hand-sorted.
pub const ZTEST_BUILTIN_NAMES: &[&str] = &[
    "run_tests",
    "zassert_contains",
    "zassert_dies",
    "zassert_eq",
    "zassert_err",
    "zassert_false",
    "zassert_ge",
    "zassert_gt",
    "zassert_le",
    "zassert_lt",
    "zassert_match",
    "zassert_ne",
    "zassert_near",
    "zassert_ok",
    "zassert_true",
    "ztest_run",
    "ztest_skip",
];

// ── Pass/fail/skip primitives ───────────────────────────────────────────
//
// stryke: builtins.rs:22292-22311. The terminal-output side prints a
// green ✓ / red ✗ / yellow ↷ glyph + the assertion label; the counter
// side is what `ztest_run` reads. `ztest_suppress_stdout` mirrors
// VMHelper::suppress_stdout — the worker child sets it after fd 2 has
// already been redirected to its per-test tmp file so we don't double-
// emit the glyph lines.

fn ztest_pass(exec: &ShellExecutor, msg: &str) {
    exec.ztest_pass_count.fetch_add(1, Ordering::Relaxed);
    if !exec.ztest_suppress_stdout {
        eprintln!("  \x1b[32m\u{2713}\x1b[0m {}", msg);
    }
}

fn ztest_fail(exec: &ShellExecutor, msg: &str, detail: &str) {
    exec.ztest_fail_count.fetch_add(1, Ordering::Relaxed);
    if !exec.ztest_suppress_stdout {
        eprintln!("  \x1b[31m\u{2717}\x1b[0m {} \u{2014} {}", msg, detail);
    }
}

fn ztest_skip_count(exec: &ShellExecutor, msg: &str) {
    exec.ztest_skip_count.fetch_add(1, Ordering::Relaxed);
    if !exec.ztest_suppress_stdout {
        eprintln!("  \x1b[33m\u{21B7}\x1b[0m skipped: {}", msg);
    }
}

/// stryke: builtins.rs::assert_label (22313-22318). Pull the optional
/// trailing MSG arg out of `args` when its position is past the
/// minimum-required slot; otherwise fall back to the default label.
fn assert_label(args: &[String], required: usize, default: &str) -> String {
    if args.len() > required {
        args[required].clone()
    } else {
        default.to_string()
    }
}

/// Shell-truthy: non-empty AND not literal "0". Matches the convention
/// `[[ -n $V && $V != 0 ]]` that most zsh assertion helpers use, and
/// stays compatible with `zassert_ok $?` (where `$?` of a successful
/// command is "0"). The strykelang version uses `is_true()` on a
/// dynamic value; we collapse to string semantics since shell args are
/// always strings.
fn shell_truthy(s: &str) -> bool {
    !s.is_empty() && s != "0"
}

fn parse_num(s: &str) -> f64 {
    s.trim().parse::<f64>().unwrap_or(0.0)
}

impl ShellExecutor {
    // ── Assertion builtins ─────────────────────────────────────────────
    //
    // Each is a faithful port of strykelang's `builtin_assert_*`
    // (builtins.rs:22328-22593). Return code matches shell convention:
    // 0 on pass, 1 on fail. The counter side is bumped via
    // ztest_pass/_fail regardless.

    /// stryke: builtin_assert_eq (builtins.rs:22328).
    /// `zassert_eq A B [MSG]`
    pub(crate) fn builtin_zassert_eq(&self, args: &[String]) -> i32 {
        let a = args.first().map(String::as_str).unwrap_or("");
        let b = args.get(1).map(String::as_str).unwrap_or("");
        let msg = assert_label(args, 2, "zassert_eq");
        if a == b {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("got '{}', expected '{}'", a, b));
            1
        }
    }

    /// stryke: builtin_assert_ne (builtins.rs:22346).
    /// `zassert_ne A B [MSG]`
    pub(crate) fn builtin_zassert_ne(&self, args: &[String]) -> i32 {
        let a = args.first().map(String::as_str).unwrap_or("");
        let b = args.get(1).map(String::as_str).unwrap_or("");
        let msg = assert_label(args, 2, "zassert_ne");
        if a != b {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("both equal '{}'", a));
            1
        }
    }

    /// stryke: builtin_assert_ok (builtins.rs:22364).
    /// `zassert_ok V [MSG]` — passes if V is truthy (non-empty, != "0").
    pub(crate) fn builtin_zassert_ok(&self, args: &[String]) -> i32 {
        let v = args.first().map(String::as_str).unwrap_or("");
        let msg = assert_label(args, 1, "zassert_ok");
        if shell_truthy(v) {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("got falsy: '{}'", v));
            1
        }
    }

    /// stryke: builtin_assert_err (builtins.rs:22381).
    /// `zassert_err V [MSG]` — passes if V is falsy/empty/"0".
    pub(crate) fn builtin_zassert_err(&self, args: &[String]) -> i32 {
        let v = args.first().map(String::as_str).unwrap_or("");
        let msg = assert_label(args, 1, "zassert_err");
        if !shell_truthy(v) {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("expected falsy, got '{}'", v));
            1
        }
    }

    /// stryke: builtin_assert_true (builtins.rs:22398) — alias for ok.
    pub(crate) fn builtin_zassert_true(&self, args: &[String]) -> i32 {
        self.builtin_zassert_ok(args)
    }

    /// stryke: builtin_assert_false (builtins.rs:22407) — alias for err.
    pub(crate) fn builtin_zassert_false(&self, args: &[String]) -> i32 {
        self.builtin_zassert_err(args)
    }

    /// stryke: builtin_assert_gt (builtins.rs:22416).
    /// `zassert_gt A B [MSG]`
    pub(crate) fn builtin_zassert_gt(&self, args: &[String]) -> i32 {
        let a = parse_num(args.first().map(String::as_str).unwrap_or(""));
        let b = parse_num(args.get(1).map(String::as_str).unwrap_or(""));
        let msg = assert_label(args, 2, "zassert_gt");
        if a > b {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("{} not > {}", a, b));
            1
        }
    }

    /// stryke: builtin_assert_lt (builtins.rs:22436).
    pub(crate) fn builtin_zassert_lt(&self, args: &[String]) -> i32 {
        let a = parse_num(args.first().map(String::as_str).unwrap_or(""));
        let b = parse_num(args.get(1).map(String::as_str).unwrap_or(""));
        let msg = assert_label(args, 2, "zassert_lt");
        if a < b {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("{} not < {}", a, b));
            1
        }
    }

    /// stryke: builtin_assert_ge (builtins.rs:22456).
    pub(crate) fn builtin_zassert_ge(&self, args: &[String]) -> i32 {
        let a = parse_num(args.first().map(String::as_str).unwrap_or(""));
        let b = parse_num(args.get(1).map(String::as_str).unwrap_or(""));
        let msg = assert_label(args, 2, "zassert_ge");
        if a >= b {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("{} not >= {}", a, b));
            1
        }
    }

    /// stryke: builtin_assert_le (builtins.rs:22476).
    pub(crate) fn builtin_zassert_le(&self, args: &[String]) -> i32 {
        let a = parse_num(args.first().map(String::as_str).unwrap_or(""));
        let b = parse_num(args.get(1).map(String::as_str).unwrap_or(""));
        let msg = assert_label(args, 2, "zassert_le");
        if a <= b {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("{} not <= {}", a, b));
            1
        }
    }

    /// stryke: builtin_assert_match (builtins.rs:22496).
    /// `zassert_match PATTERN STRING [MSG]` — passes if regex matches.
    pub(crate) fn builtin_zassert_match(&self, args: &[String]) -> i32 {
        let pattern = args.first().map(String::as_str).unwrap_or("");
        let string = args.get(1).map(String::as_str).unwrap_or("");
        let msg = assert_label(args, 2, "zassert_match");
        match regex::Regex::new(pattern) {
            Ok(re) => {
                if re.is_match(string) {
                    ztest_pass(self, &msg);
                    0
                } else {
                    ztest_fail(self, &msg, &format!("'{}' !~ /{}/", string, pattern));
                    1
                }
            }
            Err(e) => {
                ztest_fail(self, &msg, &format!("bad regex: {}", e));
                1
            }
        }
    }

    /// stryke: builtin_assert_contains (builtins.rs:22519).
    pub(crate) fn builtin_zassert_contains(&self, args: &[String]) -> i32 {
        let haystack = args.first().map(String::as_str).unwrap_or("");
        let needle = args.get(1).map(String::as_str).unwrap_or("");
        let msg = assert_label(args, 2, "zassert_contains");
        if haystack.contains(needle) {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(
                self,
                &msg,
                &format!("'{}' does not contain '{}'", haystack, needle),
            );
            1
        }
    }

    /// stryke: builtin_assert_near (builtins.rs:22544).
    /// `zassert_near A B [EPS [MSG]]` — float approx equality.
    pub(crate) fn builtin_zassert_near(&self, args: &[String]) -> i32 {
        let a = parse_num(args.first().map(String::as_str).unwrap_or(""));
        let b = parse_num(args.get(1).map(String::as_str).unwrap_or(""));
        let eps = args
            .get(2)
            .map(|s| s.trim().parse::<f64>().unwrap_or(1e-9))
            .unwrap_or(1e-9);
        let msg = assert_label(args, 3, "zassert_near");
        if (a - b).abs() <= eps {
            ztest_pass(self, &msg);
            0
        } else {
            ztest_fail(self, &msg, &format!("{} not near {} (eps={})", a, b, eps));
            1
        }
    }

    /// stryke: builtin_assert_dies (builtins.rs:22566).
    ///
    /// Shell-native variant: instead of taking a code ref (no first-
    /// class blocks in zsh), takes a string of shell code and runs it
    /// through `execute_script` in a scratch executor. Passes if the
    /// command exits non-zero OR returns Err; fails if it exits 0.
    /// The scratch executor's pass/fail counters are discarded — only
    /// the outer self's counter is bumped via ztest_pass/_fail.
    pub(crate) fn builtin_zassert_dies(&self, args: &[String]) -> i32 {
        let cmd = args.first().map(String::as_str).unwrap_or("");
        let msg = assert_label(args, 1, "zassert_dies");
        if cmd.is_empty() {
            ztest_fail(self, &msg, "first arg must be a shell command string");
            return 1;
        }
        let mut scratch = ShellExecutor::new();
        // Mirror the host's option/intercept-relevant flags so the
        // command runs under the same compat mode the test was invoked
        // in (--zsh / --bash / --posix). Without this, eval'ing
        // bash-isms inside a strict-zsh test would die for the wrong
        // reason and the assertion would pass spuriously.
        scratch.zsh_compat = self.zsh_compat;
        scratch.bash_compat = self.bash_compat;
        scratch.posix_mode = self.posix_mode;
        match scratch.execute_script(cmd) {
            Ok(0) => {
                ztest_fail(self, &msg, "expected die but command succeeded");
                1
            }
            Ok(_) | Err(_) => {
                ztest_pass(self, &msg);
                0
            }
        }
    }

    /// stryke: builtin_test_run (builtins.rs:22596).
    /// `ztest_run` / `run_tests` — print per-block summary and roll the
    /// per-block counters into the run-wide totals. Returns 0 on
    /// all-pass / 1 if any assertion in this block failed (shell
    /// convention; strykelang returns 1/0 reversed because it's a
    /// truthy-Rust-value).
    pub(crate) fn builtin_ztest_run(&self, _args: &[String]) -> i32 {
        let pass = self.ztest_pass_count.load(Ordering::Relaxed);
        let fail = self.ztest_fail_count.load(Ordering::Relaxed);
        let skip = self.ztest_skip_count.load(Ordering::Relaxed);
        let total = pass + fail + skip;
        if !self.ztest_suppress_stdout {
            eprintln!();
            if fail == 0 {
                if skip == 0 {
                    eprintln!("\x1b[32m  \u{2713} All {} tests passed\x1b[0m", total);
                } else {
                    eprintln!(
                        "\x1b[32m  \u{2713} {} passed\x1b[0m, \x1b[33m{} skipped\x1b[0m (of {})",
                        pass, skip, total
                    );
                }
            } else {
                let skip_tail = if skip > 0 {
                    format!(" (\x1b[33m{} skipped\x1b[0m)", skip)
                } else {
                    String::new()
                };
                eprintln!(
                    "\x1b[31m  \u{2717} {} of {} tests failed\x1b[0m{}",
                    fail, total, skip_tail
                );
            }
        }
        // stryke: 22629-22641 — roll per-block into totals BEFORE reset
        // so the embedder (runner) sees the cumulative numbers via
        // total + count, even though count gets zeroed.
        self.ztest_pass_total.fetch_add(pass, Ordering::Relaxed);
        self.ztest_fail_total.fetch_add(fail, Ordering::Relaxed);
        self.ztest_skip_total.fetch_add(skip, Ordering::Relaxed);
        self.ztest_pass_count.store(0, Ordering::Relaxed);
        self.ztest_fail_count.store(0, Ordering::Relaxed);
        self.ztest_skip_count.store(0, Ordering::Relaxed);
        // stryke: 22646-22652 — sticky run_failed flag for the CLI driver.
        if fail > 0 {
            self.ztest_run_failed.store(true, Ordering::Relaxed);
        }
        if fail == 0 {
            0
        } else {
            1
        }
    }

    /// stryke: builtin_test_skip (builtins.rs:22659).
    /// `ztest_skip MSG` — mark the surrounding assertion skipped.
    /// Typical usage: `ztest_skip "needs --compat" || zassert_eq ...`
    /// — the `||` short-circuits past the assertion when this returns 0.
    pub(crate) fn builtin_ztest_skip(&self, args: &[String]) -> i32 {
        let msg = args.first().cloned().unwrap_or_else(|| "skipped".into());
        ztest_skip_count(self, &msg);
        0
    }
}

/// Cheap name probe — does `name` match one of our framework
/// builtins? The bridge uses this for its match-arm guard so the
/// dispatch path only borrows the executor when the name is ours.
/// Kept inline + branch-free (a slice binary-search would be a
/// micro-pessimization at 17 entries).
pub fn try_dispatch_known(name: &str) -> bool {
    matches!(
        name,
        "zassert_eq"
            | "zassert_ne"
            | "zassert_ok"
            | "zassert_err"
            | "zassert_true"
            | "zassert_false"
            | "zassert_gt"
            | "zassert_lt"
            | "zassert_ge"
            | "zassert_le"
            | "zassert_match"
            | "zassert_contains"
            | "zassert_near"
            | "zassert_dies"
            | "ztest_run"
            | "run_tests"
            | "ztest_skip"
    )
}

/// Single dispatch entry-point for every `ztest_*` / `zassert_*` /
/// `run_tests` name. Returns `Some(status)` when `name` matches one of
/// our framework builtins, `None` otherwise — the caller (the
/// `ZshrsHost::call_function` match arm in `fusevm_bridge.rs`) uses the
/// `None` arm to fall through to the regular function-or-external
/// dispatch path.
///
/// Same shape as the zmv/zcp/zln/zcalc short-circuit pattern already
/// in `call_function`. Keeps the bridge file from having to know about
/// 17 individual builtin names.
pub fn try_dispatch(exec: &ShellExecutor, name: &str, args: &[String]) -> Option<i32> {
    let s = match name {
        "zassert_eq" => exec.builtin_zassert_eq(args),
        "zassert_ne" => exec.builtin_zassert_ne(args),
        "zassert_ok" => exec.builtin_zassert_ok(args),
        "zassert_err" => exec.builtin_zassert_err(args),
        "zassert_true" => exec.builtin_zassert_true(args),
        "zassert_false" => exec.builtin_zassert_false(args),
        "zassert_gt" => exec.builtin_zassert_gt(args),
        "zassert_lt" => exec.builtin_zassert_lt(args),
        "zassert_ge" => exec.builtin_zassert_ge(args),
        "zassert_le" => exec.builtin_zassert_le(args),
        "zassert_match" => exec.builtin_zassert_match(args),
        "zassert_contains" => exec.builtin_zassert_contains(args),
        "zassert_near" => exec.builtin_zassert_near(args),
        "zassert_dies" => exec.builtin_zassert_dies(args),
        "ztest_run" | "run_tests" => exec.builtin_ztest_run(args),
        "ztest_skip" => exec.builtin_ztest_skip(args),
        _ => return None,
    };
    Some(s)
}

// ── Worker-pool runner ──────────────────────────────────────────────────
//
// stryke: cli_runners.rs:153-694. Two halves:
//   * `run_ztest_worker_loop` (--ztest-worker) — stdin JSON loop, fork
//     per test, child runs in-process, writes one JSON line per result.
//   * `run_ztests_pool` (--ztest) — parent runner. Spawns N persistent
//     worker processes, dispatches paths via stdin, reads results via
//     stdout. Workers stay alive across the whole run; they fork per
//     request so no state accumulates.

/// stryke: cli_runners.rs:173-178.
#[derive(serde::Serialize, serde::Deserialize)]
struct WorkerRequest {
    path: String,
    #[serde(default)]
    chdir: Option<String>,
}

/// stryke: cli_runners.rs:180-195.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct WorkerResponse {
    name: String,
    passes: usize,
    fails: usize,
    failed: bool,
    detail: Option<String>,
    /// Captured stderr from the grandchild — the `✓`/`✗` checkmark lines
    /// plus any diagnostic prints the test makes. Captured per-test in the
    /// child so concurrent grandchildren can't tear each other's lines at
    /// the terminal; the parent prints this verbatim under `print_lock`
    /// so each test's output stays a contiguous block.
    #[serde(default)]
    stderr: String,
}

/// stryke: cli_runners.rs::run_one_inproc (58-151).
///
/// Run a single test file in-process: read source → `execute_script` on
/// a fresh `ShellExecutor`. Per-test isolation comes from the fresh
/// executor (own scope, sub registry, parameter table, ztest counters,
/// ztest_run_failed flag).
///
/// Returns `(passes, fails, file_failed, failure_detail)`.
fn run_one_inproc(script_abs: &Path) -> (usize, usize, bool, Option<String>) {
    let file_str = script_abs.to_string_lossy().to_string();
    let source = match std::fs::read_to_string(script_abs) {
        Ok(s) => s,
        Err(e) => return (0, 0, true, Some(format!("read failed: {}", e))),
    };

    let mut exec = ShellExecutor::new();
    exec.scriptname = Some(file_str.clone());
    exec.scriptfilename = Some(file_str.clone());
    // stryke: 22605-22628 → ztest_run inside the script will print its
    // glyph lines to stderr (we WANT these — they go into the per-test
    // capture file fd 2 was dup2'd to). suppress stays off here.
    exec.ztest_suppress_stdout = false;

    let exec_result = exec.execute_script(&source);
    let passes = exec.ztest_pass_total.load(Ordering::Relaxed)
        + exec.ztest_pass_count.load(Ordering::Relaxed);
    let fails = exec.ztest_fail_total.load(Ordering::Relaxed)
        + exec.ztest_fail_count.load(Ordering::Relaxed);
    let test_marked_failed = exec.ztest_run_failed.load(Ordering::Relaxed);

    match exec_result {
        Ok(rc) => {
            let f = test_marked_failed || fails > 0 || rc != 0;
            let detail = if f {
                if fails > 0 {
                    Some(format!("{} assertion(s) failed", fails))
                } else if rc != 0 {
                    Some(format!("exited with code {}", rc))
                } else {
                    None
                }
            } else {
                None
            };
            (passes, fails, f, detail)
        }
        Err(e) => (passes, fails, true, Some(e)),
    }
}

/// stryke: cli_runners.rs::run_test_worker_loop (197-410).
///
/// `zshrs --ztest-worker` entry point. Loops on stdin reading
/// `WorkerRequest` JSON lines. Per request: `fork()`. Child runs the
/// test in-process (fresh `ShellExecutor`), writes `WorkerResponse`
/// JSON to stdout, `_exit(0)`. Parent worker waits for the child,
/// then loops.
///
/// Returns the exit code zshrs should use when `--ztest-worker` mode
/// finishes (always 0 on normal stdin EOF).
pub fn run_ztest_worker_loop() -> i32 {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = BufReader::new(stdin.lock());
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break, // pipe closed
        };
        if line.is_empty() {
            continue;
        }
        let req: WorkerRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = WorkerResponse {
                    name: line.chars().take(60).collect(),
                    passes: 0,
                    fails: 0,
                    failed: true,
                    detail: Some(format!("worker: malformed request: {}", e)),
                    stderr: String::new(),
                };
                let mut out = stdout.lock();
                let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
                let _ = out.flush();
                continue;
            }
        };

        // SAFETY (stryke: 234-239): the --ztest-worker mode is invoked
        // before any threads spawn in the worker process — only the
        // main thread is alive at fork() time, so raw libc::fork is
        // POSIX-safe here. If you change worker init to spin up
        // threads, switch to a thread-aware fork strategy.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            let err = std::io::Error::last_os_error();
            let resp = WorkerResponse {
                name: req.path.clone(),
                passes: 0,
                fails: 0,
                failed: true,
                detail: Some(format!("worker: fork failed: {}", err)),
                stderr: String::new(),
            };
            let mut out = stdout.lock();
            let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
            let _ = out.flush();
            continue;
        }
        if pid == 0 {
            // ── Child: run one test, write result, exit ───────────────
            //
            // Wire-protocol firewall + per-test stderr capture
            // (stryke: 275-318):
            //   (1) Stdout (fd 1) is the worker→parent JSON pipe. Test
            //       code's `echo "hi"` would corrupt the wire; redirect
            //       fd 1 to /dev/null and save the original pipe end to
            //       a private fd for the JSON write at the end.
            //   (2) Stderr (fd 2) is where the ✓/✗ glyph lines emit.
            //       18 concurrent grandchildren writing in parallel
            //       would tear each other's lines. Redirect fd 2 to a
            //       per-test tmp file, then slurp it back into the
            //       JSON response so the parent prints it verbatim
            //       under print_lock.
            let (saved_stdout, stderr_capture_fd, mut stderr_path): (
                libc::c_int,
                libc::c_int,
                [u8; 32],
            ) = unsafe {
                let saved = libc::dup(1);
                if saved >= 0 {
                    let devnull = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
                    if devnull >= 0 {
                        libc::dup2(devnull, 1);
                        libc::close(devnull);
                    }
                }
                // mkstemp template — must be writable and end in XXXXXX.
                let mut tmpl: [u8; 32] = *b"/tmp/zshrs-ztest-XXXXXX\0\0\0\0\0\0\0\0\0";
                let cap_fd = libc::mkstemp(tmpl.as_mut_ptr() as *mut libc::c_char);
                if cap_fd >= 0 {
                    libc::unlink(tmpl.as_ptr() as *const libc::c_char);
                    libc::dup2(cap_fd, 2);
                }
                (saved, cap_fd, tmpl)
            };
            let _ = &mut stderr_path;

            // stryke: 322-325. chdir for `source ./lib/foo.zsh` style
            // resolution relative to the project root.
            if let Some(cd) = req.chdir.as_deref() {
                let _ = std::env::set_current_dir(cd);
            }
            let script_abs = PathBuf::from(&req.path);
            let (passes, fails, file_failed, detail) = run_one_inproc(&script_abs);
            let name = script_abs
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| req.path.clone());

            // stryke: 334-356. Slurp captured stderr back from the
            // unlink'd tmp file.
            let captured_stderr: String = unsafe {
                if stderr_capture_fd >= 0 {
                    libc::lseek(stderr_capture_fd, 0, libc::SEEK_SET);
                    let mut buf = Vec::with_capacity(4096);
                    let mut chunk = [0u8; 8192];
                    loop {
                        let n = libc::read(
                            stderr_capture_fd,
                            chunk.as_mut_ptr() as *mut libc::c_void,
                            chunk.len(),
                        );
                        if n <= 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n as usize]);
                    }
                    libc::close(stderr_capture_fd);
                    String::from_utf8_lossy(&buf).into_owned()
                } else {
                    String::new()
                }
            };

            let resp = WorkerResponse {
                name,
                passes,
                fails,
                failed: file_failed,
                detail,
                stderr: captured_stderr,
            };
            let line = format!(
                "{}\n",
                serde_json::to_string(&resp).unwrap_or_else(|_| String::new())
            );
            unsafe {
                if saved_stdout >= 0 {
                    let _ = libc::write(
                        saved_stdout,
                        line.as_ptr() as *const libc::c_void,
                        line.len(),
                    );
                    libc::close(saved_stdout);
                }
            }

            // stryke: 399-403. `_exit` skips Rust's atexit / Drop chain
            // — running parent destructors in a forked child would
            // close fds the parent still needs and double-free shared
            // allocations.
            unsafe { libc::_exit(0) };
        }
        // ── Worker (parent of grandchild): wait, then loop ────────────
        let mut status: libc::c_int = 0;
        unsafe { libc::waitpid(pid, &mut status, 0) };
    }
    0
}

/// stryke: cli_runners.rs::run_tests_pool (412-694).
///
/// Worker-pool runner. Spawns `n_workers` persistent zshrs
/// `--ztest-worker` processes once, then dispatches test paths to
/// them via stdin, reads results from stdout. Workers stay alive
/// across the whole run — they fork per request, so no state
/// accumulates.
///
/// Discovery: empty `targets` → look for `t/` then `tests/`. Test
/// files match `test_*` / `t_*` prefix and `.zsh` / `.sh` / `.zshrs`
/// suffix.
pub fn run_ztests_pool(targets: &[String], j_threads: Option<&str>, quiet: bool) -> i32 {
    // stryke: 422-442. Discovery / target normalization.
    let targets: Vec<String> = if targets.is_empty() {
        if Path::new("t").is_dir() {
            vec!["t".to_string()]
        } else if Path::new("tests").is_dir() {
            vec!["tests".to_string()]
        } else {
            eprintln!("zshrs --ztest: no t/ or tests/ directory found");
            return 1;
        }
    } else {
        targets
            .iter()
            .filter(|t| Path::new(t.as_str()).exists())
            .cloned()
            .collect()
    };
    if targets.is_empty() {
        eprintln!("zshrs --ztest: no valid paths found");
        return 1;
    }

    // stryke: 443-465. Expand directories into the matching test files.
    let mut test_files: Vec<String> = Vec::new();
    for target in &targets {
        let target_path = Path::new(target);
        if target_path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(target_path) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path().to_string_lossy().to_string();
                    let name = entry.file_name().to_string_lossy().to_string();
                    if (name.starts_with("test_") || name.starts_with("t_"))
                        && (name.ends_with(".zsh")
                            || name.ends_with(".sh")
                            || name.ends_with(".zshrs"))
                    {
                        test_files.push(path);
                    }
                }
            }
        } else {
            test_files.push(target.clone());
        }
    }
    test_files.sort();
    test_files.dedup();
    if test_files.is_empty() {
        eprintln!("zshrs --ztest: no test files found");
        return 1;
    }
    let total = test_files.len();

    // stryke: 472-479.
    let n_workers = j_threads
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        });

    // stryke: 481-491. Resolve our own binary path for the worker spawn.
    let exe: PathBuf = std::env::current_exe()
        .ok()
        .filter(|p| p.exists())
        .or_else(|| {
            std::env::args()
                .next()
                .and_then(|a| std::fs::canonicalize(&a).ok())
        })
        .unwrap_or_else(|| {
            PathBuf::from(std::env::args().next().unwrap_or_else(|| "zshrs".into()))
        });

    if !quiet {
        eprintln!(
            "\x1b[36mRunning {} test file{} ({} workers)\x1b[0m\n",
            total,
            if total == 1 { "" } else { "s" },
            n_workers
        );
    }

    // stryke: 502-522. Pre-resolve canonical paths + project_root per
    // file (the worker child chdirs to project_root so `source ./lib/...`
    // works).
    let jobs: Vec<(String, String, String)> = test_files
        .iter()
        .map(|f| {
            let abs = std::fs::canonicalize(f)
                .unwrap_or_else(|_| PathBuf::from(f))
                .to_string_lossy()
                .to_string();
            let chdir = Path::new(&abs)
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string());
            let name = Path::new(&abs)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| abs.clone());
            (name, abs, chdir)
        })
        .collect();

    let (job_tx, job_rx) = crossbeam_channel::unbounded::<(String, String, String)>();
    for j in jobs {
        job_tx.send(j).expect("send job");
    }
    drop(job_tx);

    let total_pass = Arc::new(AtomicUsize::new(0));
    let total_fail = Arc::new(AtomicUsize::new(0));
    let failed_count = Arc::new(AtomicUsize::new(0));
    let failure_details: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let print_lock: Arc<Mutex<()>> = Arc::new(Mutex::new(()));

    // stryke: 536-633. One OS thread per worker process — the thread
    // is a "pump" shuttling JSON over the worker's stdin/stdout pipes
    // (NOT the worker itself). The actual test execution is in the
    // forked grandchildren.
    let mut handles = Vec::with_capacity(n_workers);
    for _ in 0..n_workers {
        let job_rx = job_rx.clone();
        let total_pass = Arc::clone(&total_pass);
        let total_fail = Arc::clone(&total_fail);
        let failed_count = Arc::clone(&failed_count);
        let failure_details = Arc::clone(&failure_details);
        let print_lock = Arc::clone(&print_lock);
        let exe = exe.clone();
        let h = thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let mut child = match process::Command::new(&exe)
                    .arg("--ztest-worker")
                    .stdin(process::Stdio::piped())
                    .stdout(process::Stdio::piped())
                    .stderr(process::Stdio::inherit())
                    .spawn()
                {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("zshrs --ztest: failed to spawn worker: {}", e);
                        return;
                    }
                };
                let mut stdin = child.stdin.take().expect("worker stdin");
                let stdout = child.stdout.take().expect("worker stdout");
                let mut reader = BufReader::new(stdout);
                let mut line_buf = String::new();

                while let Ok((_short_name, abs, chdir)) = job_rx.recv() {
                    let req = WorkerRequest {
                        path: abs.clone(),
                        chdir: Some(chdir),
                    };
                    let req_json = serde_json::to_string(&req).expect("serialize");
                    if writeln!(stdin, "{}", req_json).is_err() {
                        break;
                    }
                    if stdin.flush().is_err() {
                        break;
                    }

                    line_buf.clear();
                    if reader.read_line(&mut line_buf).is_err() || line_buf.is_empty() {
                        break;
                    }
                    let resp: WorkerResponse = match serde_json::from_str(line_buf.trim()) {
                        Ok(r) => r,
                        Err(e) => WorkerResponse {
                            name: abs.clone(),
                            passes: 0,
                            fails: 0,
                            failed: true,
                            detail: Some(format!("malformed worker response: {}", e)),
                            stderr: String::new(),
                        },
                    };

                    total_pass.fetch_add(resp.passes, Ordering::Relaxed);
                    total_fail.fetch_add(resp.fails, Ordering::Relaxed);
                    if resp.failed {
                        failed_count.fetch_add(1, Ordering::Relaxed);
                        if let Some(d) = &resp.detail {
                            failure_details
                                .lock()
                                .unwrap()
                                .push((resp.name.clone(), d.clone()));
                        }
                    }

                    if !quiet {
                        let _g = print_lock.lock().unwrap();
                        eprintln!("\x1b[1m\u{2500}\u{2500} {} \u{2500}\u{2500}\x1b[0m", resp.name);
                        if !resp.stderr.is_empty() {
                            eprint!("{}", resp.stderr);
                        }
                        eprintln!(
                            "  {} passed, {} failed{}",
                            resp.passes,
                            resp.fails,
                            if resp.failed { " (file FAILED)" } else { "" }
                        );
                        eprintln!();
                    }
                }
                drop(stdin); // close worker stdin → worker exits its loop
                let _ = child.wait();
            })
            .expect("spawn worker thread");
        handles.push(h);
    }
    for h in handles {
        let _ = h.join();
    }

    let failed = failed_count.load(Ordering::Relaxed);
    let total_pass = total_pass.load(Ordering::Relaxed);
    let total_fail = total_fail.load(Ordering::Relaxed);
    let failure_details = Arc::try_unwrap(failure_details)
        .expect("workers dropped failure_details Arc")
        .into_inner()
        .unwrap();
    let grand_total = total_pass + total_fail;
    if !quiet {
        eprintln!("\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}");
    }
    if failed == 0 {
        if !quiet {
            eprintln!(
                "\x1b[32m\u{2713} All {} test file{} passed ({} assertions)\x1b[0m",
                total,
                if total == 1 { "" } else { "s" },
                grand_total
            );
        }
        0
    } else {
        if !quiet {
            let bar = "\u{2550}".repeat(64);
            eprintln!();
            eprintln!("\x1b[1;31m{}\x1b[0m", bar);
            eprintln!("\x1b[1;31m                        FAILURES SUMMARY\x1b[0m");
            eprintln!("\x1b[1;31m{}\x1b[0m", bar);
            for (file_name, details) in &failure_details {
                eprintln!();
                eprintln!("\x1b[1;33m\u{2500}\u{2500} {} \u{2500}\u{2500}\x1b[0m", file_name);
                for line in details.lines().take(20) {
                    eprintln!("  {}", line);
                }
                if details.lines().count() > 20 {
                    eprintln!("  ... ({} more lines)", details.lines().count() - 20);
                }
            }
            eprintln!();
            eprintln!("\x1b[1;31m{}\x1b[0m", bar);
            eprintln!(
                "\x1b[31m\u{2717} {} of {} test file{} failed ({} passed, {} failed)\x1b[0m",
                failed,
                total,
                if total == 1 { "" } else { "s" },
                total_pass,
                total_fail
            );
        }
        1
    }
}
