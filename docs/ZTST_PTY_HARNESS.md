# ZTST PTY Harness — Design Doc

Status: design, not yet built
Author: zshrs team
Target: 100% green parity with upstream zsh's `Test/ztst.zsh` on the same `.ztst` corpus

---

## 1. Goal

Build a pseudo-terminal-driven, per-file-persistent test harness that runs zsh's own `.ztst` integration corpus against the `zshrs` binary and gates passing on byte-exact match with upstream zsh. The current `tests/ztst_runner.rs` is a fake-pass wrapper (per-block fresh-spawn, never asserts) and must be replaced.

Acceptance: `cargo test --test ztst_pty_harness` runs every `.ztst` file in `test_corpus/`, asserts zero per-block failures, and the diff between zshrs and upstream `zsh` on the same machine is empty.

### 1.1 Non-goals

- Cross-platform Windows support. Headless Linux + macOS only.
- Cross-shell test reuse — this harness is zsh-grammar-specific.
- Speed parity with upstream `make test`. Correctness wins.
- Replacing the existing 392-test construct corpus (`tests/zsh_construct_corpus.rs`). That layer is non-interactive and stays as-is. ZTST is the *interactive* and *stateful* layer above it.

---

## 2. Why a real PTY

`.ztst` blocks exercise three modes the current `-c <code>` runner can't reach:

1. **`isatty()`-gated code paths.** ZLE bootstraps only when stdin is a tty. Completion (`compsys`) only fires from inside ZLE. `read -k1`, `read -t`, `vared`, `select`, prompt rendering, ZLE widgets, signal-driven redraw — all sit behind tty checks. Without a real (or pty-emulated) tty, those code paths are unreachable.
2. **Job control.** `fg`, `bg`, `tcsetpgrp`, `setsid`, SIGTSTP/SIGCONT round-tripping, `wait %1` — require a controlling terminal and process-group leadership.
3. **Signal traps with terminal-driven input.** Tests like C03traps deliver SIGINT during a `read` and assert specific output; without a tty the read never blocks the right way.

The pty pair is allocated programmatically (`openpty(3)` / `forkpty(3)`); no controlling terminal in the parent is required. Headless Linux GitHub Actions and macOS runners both support this.

---

## 3. Why per-file-persistent

`.ztst` blocks share state. Inside one file:

- `%prep` declares functions, sets variables, creates files, redirects fds — and every subsequent `%test` block depends on those side effects.
- A `%test` block can declare a function that a later `%test` block calls.
- A `%test` block can write a file that a later `%test` block reads.
- `EXIT` traps registered in `%prep` must fire only when the file's `%clean` (or end) runs.

The current runner re-prepends `%prep` to every block but cannot replay the side effects of intermediate `%test` blocks. The only correct architecture is one shell process per file, all blocks driven through it in source order.

This matches upstream zsh's own `Test/ztst.zsh`, which `eval`s each block inside a single zsh interpreter.

---

## 4. Architecture

```
┌─────────────────────────┐         ┌───────────────────────────┐
│  ztst_pty_harness       │   pty   │  zshrs -i -f              │
│  (cargo test binary)    │◄───────►│  (interactive child)      │
│                         │         │                           │
│  • parses .ztst         │         │  • ZLE active             │
│  • drives blocks via    │         │  • compsys autoloaded     │
│    side-channel fd 9    │         │  • PROMPT='' (silenced)   │
│  • captures via vte     │         │  • side-channel fd 9 →    │
│  • diffs vs expected    │         │    pipe back to harness   │
└─────────────────────────┘         └───────────────────────────┘
```

### 4.1 Process model

- **One** `zshrs` child per `.ztst` file. Spawned with `-i -f` (interactive, no rcs).
- Pty pair allocated via `nix::pty::openpty` (already in deps; `feature = "term"` enabled). Child inherits the slave fd as 0/1/2 with `forkpty` semantics.
- A separate **side-channel fd** (fd 9 in the child) is a regular pipe whose read end is owned by the harness. The harness uses fd 9 for control plane (sentinels, exit codes, stream framing) so it never collides with whatever the test prints to stdout/stderr.
- **One** `tempfile::TempDir` per file used as `HOME`, `TMPDIR`, `ZTST_tmp` in the child env. Cleanup unconditional on test exit.

### 4.2 Block boundary protocol

The hard problem: in pty mode, stdout and stderr both go to the slave tty (one stream from the harness's view). We need to split stdout/stderr per block AND know when each block is done AND capture each block's exit status.

Solution: **side-channel framing on fd 9**. The harness wraps each block in a fixed driver template:

```zsh
__zts_block_id=<NUM>
{
  __zts_block_stdout_start=<sentinel>
  print -u9 "##ZTST-STDOUT-BEGIN-$__zts_block_id##"
  exec 3>&1 4>&2  # stash original tty fds
  exec 1>&9       # stdout → side channel (NOT the tty)
  exec 2>>"$ZTST_tmp/stderr-$__zts_block_id"  # stderr → per-block file
  { eval '<BLOCK CODE>'; } 2>&1
  __zts_status=$?
  exec 1>&3 2>&4 3>&- 4>&-  # restore
  print -u9 "##ZTST-STDOUT-END-$__zts_block_id##"
  print -u9 "##ZTST-EXIT-$__zts_block_id-$__zts_status##"
} 2>/dev/null
```

Per-block protocol:

1. Harness writes the wrapped block to the master pty. Child reads it as keyboard input on stdin (since stdin == slave tty), evaluates it via the running shell.
2. Child redirects fd 1 to fd 9 (side channel) for the duration of the block. Test stdout writes go to the harness's pipe.
3. Child redirects fd 2 to a per-block file under `$ZTST_tmp`. Stderr lands on disk.
4. Block runs. Stdout → side channel. Stderr → file.
5. Child emits BEGIN/END sentinels and EXIT code on fd 9.
6. Harness drains fd 9 until it sees the END sentinel; everything between BEGIN and END is the block's stdout. EXIT code follows.
7. Harness reads `$ZTST_tmp/stderr-<id>` to get block's stderr.
8. Harness compares stdout/stderr/exit against the expected lines parsed from the `.ztst` file. Block pass/fail is byte-exact match (modulo `d`/`D` flags from the ztst spec).

Why route stdout to fd 9 instead of capturing tty output: pty output is contaminated by ZLE redraw, prompt repaint, terminal queries (cursor-position, color-mode), and echo. Side-channel pipe is clean.

Why route stderr to a file instead of fd 9: fd 9 is multiplexed (sentinels + stdout). Mixing stderr in needs another framing layer — files are simpler and stderr volume is bounded.

### 4.3 ZLE/completion blocks

A subset of `.ztst` blocks (X-series, Y-series) drive ZLE keystrokes and assert against captured screen state. For these, the wrapper template differs:

- Stdout stays on tty (so ZLE renders to it).
- Harness sends keystroke bytes directly to the master fd (typing into the line editor).
- A `vte::Parser` consumes the bytes echoed back and reconstructs the terminal screen state.
- Block end is signalled by a custom widget the harness binds at startup (`zle -N __zts_block_done`) which prints `##ZTST-ZLE-DONE-$id##` to fd 9 and accepts the line.
- Comparison is against the screen state at the moment of completion — not raw byte stream — using a small ANSI-aware terminal model.

The `.ztst` file's `comptest` helper directory shows upstream's keystroke convention; we reuse that grammar for X/Y blocks.

### 4.4 Initial handshake

After spawn, before sending any block:

1. Harness writes:
   ```zsh
   PS1=''; PS2=''; RPROMPT=''; setopt no_zle  # quiet startup, defer ZLE arm
   exec 9>&<harness_pipe_fd>  # fd 9 = side channel
   stty -echo -onlcr           # silence keyboard echo + CR translation
   print -u9 "##ZTST-READY##"
   ```
2. Harness drains the master pty until it sees `##ZTST-READY##` on fd 9.
3. Only then does block-driving start.

The `setopt no_zle` happens up-front so non-ZLE blocks aren't affected by line-editor IO. Per-block prologue re-enables ZLE for X/Y blocks.

### 4.5 Stream-stripping pipeline

Even with side-channel routing, captured streams get cleaned through:

| Filter | Why |
|---|---|
| `vte::Parser` (X/Y only) | Resolve cursor moves, scrolling regions, SGR into a flat screen buffer |
| ANSI CSI strip | Drop residual color/cursor escapes from non-ZLE blocks (some tests print `\e[...m`) |
| CR collapse | `\r\n` → `\n` (slave tty does ONLCR by default; we disable but keep filter as belt+suspenders) |
| Trailing newline trim | Match `.ztst` semantics (last `\n` of expected is implicit) |

---

## 5. Sandboxing

Per-file sandbox via `tempfile::TempDir`. Child env:

- `HOME` → sandbox
- `TMPDIR` → sandbox
- `ZTST_tmp` → sandbox
- `ZDOTDIR` → sandbox (no `.zshrc` from real $HOME loads)
- `LANG=C`, `LC_ALL=C`, `TERM=xterm-256color`
- `TERMINFO` → vendored entry checked into `test_corpus/terminfo/` so behavior is identical regardless of host's terminfo db

Sandbox dropped at end of file (success or failure). `Drop` impl on a guard struct ensures it survives panics in the harness.

---

## 6. Block-level expected-output parser

`.ztst` block format (from `man zshtst` / upstream `ztst.zsh`):

```
  <block code, indented>
<status>[<flags>]:<message>
[><expected stdout line>]*
[?<expected stderr line>]*
[*?<pattern stderr line>]*
[*><pattern stdout line>]*
[F:<failure hint, ignored>]*
```

Flags:

- `d` — ignore stdout
- `D` — ignore stderr
- `f` — expected fail (xfail)
- `q` — delayed substitution (re-eval expected lines after run)
- `*` prefix on `>`/`?` — pattern match (zsh glob), not literal

The existing `tests/ztst_runner.rs` already has a working parser for this format. Lift it out into `crates/ztst-harness/src/parse.rs` as a library; the new harness reuses it.

---

## 7. Cargo integration

```
crates/
  ztst-harness/
    Cargo.toml
    src/
      lib.rs           # parse, expected-output model
      parse.rs         # .ztst block parser (lifted from tests/)
      pty.rs           # pty alloc, child spawn, master fd ownership
      driver.rs        # block-driving loop, sentinel protocol
      stream.rs        # vte / ANSI strip / CR collapse pipeline
      compare.rs       # byte-exact / pattern compare with flags
      sandbox.rs       # TempDir guard, env setup
    bin/
      ztst-runner.rs   # standalone runner (cargo run -p ztst-harness)
tests/
  ztst_pty_harness.rs  # cargo test wrapper, one #[test] per .ztst file
```

`tests/ztst_pty_harness.rs` keeps the `ztst_tests! { a01_grammar => "A01grammar.ztst", ... }` macro pattern from the current runner; each generated `#[test]`:

```rust
#[test]
fn a01_grammar() {
    let summary = ztst_harness::run_file("A01grammar.ztst").unwrap();
    assert_eq!(summary.failed, 0, "{} blocks failed:\n{}", summary.failed, summary.detail);
}
```

Hard-asserts. No more lenient mode.

The standalone bin (`cargo run --bin ztst-runner -- A01grammar.ztst`) is for local interactive debugging — runs one file, prints per-block diff, no cargo-test harness in the way.

---

## 8. Phase plan

Each phase ends in cargo green for the listed series; subsequent phases must not regress prior ones.

| Phase | Scope | Acceptance |
|---|---|---|
| **P0** | Delete `tests/ztst_runner.rs` (the lying wrapper). Move parse code to `crates/ztst-harness/parse.rs`. | tree clean, parse round-trip tests pass |
| **P1** | PTY scaffold: spawn `zshrs -i -f` on pty, complete handshake, kill on drop. No block driving yet. | one #[test] proving spawn + ready + clean shutdown |
| **P2** | Side-channel fd 9 wired, block driver template, stdout/stderr/exit capture, byte-exact compare. | A01-A07 + A08 fully green (grammar, alias, quoting, redirect, execution, assign, control, time) |
| **P3** | C-series — arith, cond, traps, funcdef, debug. C03traps proves signal delivery works through pty. | C01-C05 green |
| **P4** | B-series builtins — cd, typeset, print, read, eval, fc, emulate, shift, hash, getopts, kill, limit, whence. | B01-B13 green |
| **P5** | D-series — chdir, glob, parameter, exports, jobs (D04 needs job control + tcsetpgrp), redirect-extra, hash, pcre, brace. | D01-D09 green |
| **P6** | E + V — options, xtrace, modules. V03 zptys-inside-our-pty (nested), V11 db_gdbm, V14 zsystem call require module ports to be wired in. | E01-E02, V01-V14 green |
| **P7** | X-series — ZLE keystroke replay. Bind `__zts_block_done` widget, send keystrokes via master fd, capture screen via `vte`. | X01-X05 green |
| **P8** | Y-series — completion. Drives ZLE to `\t`, captures menu state, asserts. | Y01-Y03 green |
| **P9** | Z-series — misc, runhelp, styles. Likely trivial after P2-P8 land. | Z01-Z03 green |
| **P10** | Diff vs upstream `zsh`. Run the same corpus through real zsh on the same machine; assert identical pass/fail per block. | parity report empty |

P0-P2 are scaffolding. P3-P6 is grinding through deterministic series. P7-P8 is the real ZLE/completion driving — assumes the existing `src/zle/` + `compsys/` ports are functionally complete; gaps surface as test failures with clear diffs into specific zshrs source files.

---

## 9. Risk register

| Risk | Mitigation |
|---|---|
| Some `.ztst` blocks rely on `zpty` (zsh's own pty module). Driving zpty-inside-our-pty is nested. | `src/zle/` doesn't need zpty itself for X/Y; for V03 zpty tests we either skip-and-document or port `zsh/Src/Modules/zpty.c` (own item, ~600 LOC C). |
| Tests pin upstream zsh's exact error-message wording (`"zsh: parse error near "`, `"bad pattern: "` etc.). | Audit error messages in zshrs. Either match upstream exactly or pre-process expected-stderr through a wording-adapter table. Track diffs in `docs/ZTST_ERROR_DRIFT.md`. |
| Terminal capability database differs between host and CI. | Vendor a fixed terminfo entry under `test_corpus/terminfo/`. `TERMINFO` env points at it. Same DB everywhere. |
| Race between block code completion and sentinel emit (e.g., backgrounded job not yet flushed before END sentinel). | Block template ends with `wait` before sentinel. Tests that rely on async output (`&` jobs) explicitly poll. |
| `read -k1` consumes our sentinel bytes. | Side-channel fd is fd 9, NOT stdin — `read` from stdin doesn't see it. |
| Tests use `setopt printexitvalue` or `set -x` — extra output corrupts capture. | Block prologue clears those, restored after block. |
| ZLE tests assume specific terminal width / height. | `stty rows 24 cols 80` after pty alloc, before block driving. |
| Slow bootstrap (compsys autoload, fpath scan) extends startup well past current `ZTST_TIMEOUT_MS`. | Two-tier timeout: `BOOTSTRAP_TIMEOUT_MS` (default 5000) for handshake, `BLOCK_TIMEOUT_MS` (default 200) per block. Override per file via `// ZTST_FILE_TIMEOUT=NN` directive. |
| Upstream zsh and zshrs differ on a behavior we consider a feature (e.g. zshrs adds parallelism that changes order). | Mark such tests with a `zshrs_xfail` table per-file; harness consumes the table and inverts the assertion. Public table = honest diff. |

---

## 10. Why this proves 100% parity (not just runs the suite)

The harness's claim of parity stands on three legs:

1. **Same corpus.** `.ztst` files are byte-for-byte upstream. No local edits. No "skip this on Linux." Track upstream zsh's release tag, sync corpus on bumps.
2. **Same protocol.** Block boundary semantics, expected-output flags, status-line format, pattern-match semantics — all match `Test/ztst.zsh`. We're not inventing a parallel protocol.
3. **Cross-shell diff baseline.** P10 runs the same corpus through real `zsh` on the same machine and demands zero diff. If zshrs passes a block real zsh fails, that's a *bug* (or behavior divergence to be tracked) — not a "win." If real zsh passes a block zshrs fails, we file the gap against zshrs src and don't ship until fixed.

When `cargo test --test ztst_pty_harness && diff <(zsh-ztst-summary) <(zshrs-ztst-summary)` exits zero, parity is proven.

---

## 11. Required deps

Already in tree:
- `nix = { version = "0.29", features = ["signal", "process", "term", "fs"] }` — `pty::openpty`, `forkpty`, signal handling.
- `libc` — direct fcntl, ioctl as needed.
- `tempfile` (already used by zshrs) — sandbox dirs.
- `regex` — block parser.

Add:
- `vte = "0.13"` — terminal escape parser for X/Y blocks.

That's it. No `tokio`, no `portable-pty`, no `expect`-style framework. Kept stdlib-near so the harness builds clean on any host the rest of zshrs builds on.

---

## 12. Out-of-scope follow-ups

- Recording mode: harness optionally captures expected stdout/stderr/exit and writes them back to `.ztst` files. Useful for adding new blocks, dangerous as primary update path.
- TUI dashboard: live per-file pass/fail with diff drilldown. Nice-to-have, not blocking.
- Per-block timing histogram: useful for tracking perf regressions in zshrs against zsh, but separate effort.
- Coverage map: which zshrs source lines fire under which ztst block. Build on `cargo-llvm-cov`. Separate doc.

---

## 13. Decision log

- **Pty over fork+exec.** Forced by `isatty()`-gated tests.
- **Side-channel fd 9 for control plane.** Cleanest way to split stdout vs stderr vs exit-code without re-implementing terminal stream demultiplexing.
- **Per-file persistent shell.** Forced by stateful block dependencies.
- **Hard-assert on first failure.** No more lenient mode; "passed" must mean passed.
- **Cargo integration kept.** A standalone bin exists for debugging, but cargo test is the gate. CI runs `cargo test`.
- **Vendor terminfo.** Eliminates host-DB drift. ~50KB cost.
- **No tokio.** Sync IO with `poll(2)` is enough; less surface area, no async-runtime headaches inside test code.
