# tests/compsys_fixtures — the durable completion-parity evidence base

One JSON file per confirmed finding: a zsh-vs-zshrs completion divergence, or —
clearly marked as such — a defect in the REFERENCE shell that the harnesses run
into. `scripts/compsys_regressions.py` replays the whole set and is the single
gate that says whether the pinned evidence still holds.

## Running it

```sh
scripts/compsys_regressions.py                     # the full sweep (fixtures + controls)
scripts/compsys_regressions.py --quick             # the short subset, one cell per harness
scripts/compsys_regressions.py --variants          # the extra witnesses too
scripts/compsys_regressions.py --list              # what is pinned; boots no shells
scripts/compsys_regressions.py --only cc_match_set # one fixture (repeatable)
scripts/compsys_regressions.py --json out.json     # machine-readable document
scripts/compsys_regressions.py --reference-defects # incl. the upstream-zsh-crash guard
```

Exit status:

| code | meaning |
| ---- | ------- |
| 0 | every attempted cell still behaves the way the fixtures record |
| 1 | something MOVED — a fixture no longer diverges, a control that used to agree now diverges, a fingerprint drifted, or the pinned upstream zsh crash stopped happening |
| 2 | the RUNNER could not answer — a harness errored, a cell ran out of budget, the zshrs binary is missing, a fixture is unreadable |

2 wins over 1 when both happen: an incomplete run cannot certify the rest.

A change in *either* direction is non-zero. A fixture asserting a bug that no
longer exists is a false claim under the author's name, and a fixture that starts
failing in a new shape is no longer describing the bug it was written for.

Useful flags: `--jobs N` (default **1**, deliberately — see below),
`--confirm-moved N` (default 2 — see below), `--timeout`, `--keep` (keep each
cell's temp dir), `--no-controls`, `--no-binary-hash`, and `--harness-dir DIR`
for when the sibling harnesses are mid-edit:

```sh
mkdir -p /tmp/h && for f in compsys_parity comptab_parity compsys_spec_fuzz parity_corpus; do
    git show HEAD:scripts/$f.py > /tmp/h/$f.py; done
scripts/compsys_regressions.py --harness-dir /tmp/h
```

`--jobs` defaults to 1 on evidence, not on caution: the ledger this directory
replaces recorded that at `--jobs 8..10` roughly 80% of its `failures` were the
debug build missing a harness per-key budget under load rather than divergences,
and `compsys_parity.py` refuses `--jobs > 1` outright while measuring. The value
used is written into the `--json` document, along with the machine's load
average, so a result gathered under load says so.

### Why a "moved" verdict is replicated

`--confirm-moved` (default 2) re-runs any cell that scores as MOVED, or that
could not be scored at all. That is not a softener; a single-shot verdict is
demonstrably not reliable on this machine. The first round-4 full sweep ran at a
load average of 45 (sixteen peer sessions, cargo builds, other fuzzers) and
scored the live fixture `narrow_terminal_error_redraw` `NOW-PASSES` with a
**zero-row** diff — both shells drew the same incomplete screen. Re-run three
times immediately afterwards it went `TIMEOUT`, `STILL-DIVERGES`,
`STILL-DIVERGES`. Reporting that first result would have retired a real finding
on a scheduling artefact.

So the asymmetry is deliberate: a verdict that the evidence is UNCHANGED is
taken at face value — it is the harder result to produce by accident, since the
two shells had to disagree in the recorded shape — while a verdict that
something moved has to survive replication. Every attempt is reported, and a
cell that needed a re-run is named in the summary (`NEEDED A RE-RUN`) even when
it ends up unchanged, because intermittency is itself a fact about the
measurement. `--confirm-moved 0` disables it.

## Why this directory exists

The completion fuzzers write their minimised reproducers to `target/` and to
`$TMPDIR`. `.gitignore:1` is `/target`, so `cargo clean` destroys them — and
during round 4 a peer instance deleted `target/` wholesale to recover a full
disk, taking every round-3 reproducer with it *mid-session*. The three fixtures
whose reproducers died that way were rebuilt by hand from the run output and
re-confirmed against the rebuilt binary; that is exactly the work this directory
exists to make unnecessary next time.

A fixture here carries everything needed to reproduce the cell with no other
file present: the buffer, the key sequence, the zstyle statements or the
completer source and its widget declaration, the geometry when it is
load-bearing, and the difference **as it was observed** — not as it was
theorised.

**Harnesses should write new reproducers here rather than under `target/` or
`$TMPDIR`.** That change belongs in the harnesses themselves
(`compsys_spec_fuzz.py`, `comptab_parity.py`, `compsys_parity.py`) and is
recorded as a recommendation, not made here.

## Format (`"schema": "compsys-fixture/2"`)

Schema 1 files are still read; they simply carry no `controls` and no binary
stamp.

| field | meaning |
| ----- | ------- |
| `id` | stable name; also the filename |
| `title` | one line, what the two shells do differently |
| `harness` | `compsys_spec_fuzz`, `comptab_parity`, `compsys_parity` or `zsh_reference_probe` — which one owns the replay |
| `run.buffer` / `run.keys` | typed, then each key sent in order. Only `compsys_parity` buffers may contain a newline (the continuation cells) |
| `run.flags` | extra harness flags the finding needs (`--strict-stream`, `--compare-attrs`, `--strict-cursor`) |
| `run.zstyle` | statements written to a temp file and sourced by both shells |
| `run.zstyle_file` | repo-relative zstyle fixture (e.g. `scripts/parity_zstyle.zsh`) when the finding needs the real config. Neither field present means **no** styles — `compsys_parity` is passed `--no-zstyle`, because its own default is the repo fixture |
| `run.rows` / `run.cols` | geometry, only when the finding depends on it |
| `run.spec` | `compsys_spec_fuzz` only: `cmd`, `kind`, `widget`, `setup` and the completer source, re-materialised into a `--replay` file. `widget` and `setup` are load-bearing — three fixtures ARE about the widget, and a reproducer without them replays the default TAB binding, which is those fixtures' own control |
| `run.word` / `run.control_word` / `run.trials` | `zsh_reference_probe` only |
| `expect` | `diverges`, or `reference-crash` for an upstream defect |
| `fingerprint` | `comptab_parity`'s stable id for the failure shape, or `null` when the owning harness does not compute one |
| `observed` | the recorded verdict, the differing rows verbatim, one-sided diagnostics, raw-stream fragments, and prose notes |
| `controls` | cells the fixture pins as **agreeing**, replayed by default. A control is what makes the fixture's variable the variable: each continuation fixture carries the same completion on one physical line, each widget fixture carries the same completer through the default TAB binding. A control that starts diverging (`CONTROL-MOVED`) fails the run |
| `variants` | further confirmed witnesses of the same finding; replayed under `--variants`. Each may override any `run` field, and carries its OWN `fingerprint` — the shape id takes the surrounding context in, so the same defect reached through a different command legitimately produces a different one |
| `default_run` | `false` on a fixture the gate does not attempt by default; `default_run_reason` says why |
| `origin` | where the reproducer was harvested from |
| `confirmed` | date, commit, harness file hashes, both shell versions — and, from schema 2, the **binary identity** of the zshrs under test |

`observed` is evidence, not a target. It is never edited to make a run look
better; when behaviour changes, the fixture is re-confirmed or moved out of the
bug set with the run that shows the change.

### The binary stamp, and what a `NOW-PASSES` means

`confirmed.zshrs_binary` records path, size, mtime, `--version` and a sha256
prefix of the binary the fixture was confirmed against, plus whether it was the
debug build under `target/` or the Homebrew-installed one — they are **not**
interchangeable, and this round proved it: a peer instance's `cargo clean`
briefly left `target/debug/zshrs` absent and another agent's run silently fell
back to `/opt/homebrew/bin/zshrs`. `--version` alone does not distinguish two
builds of the same version.

That matters because peers commit zshrs fixes to `main` all day. When a fixture
flips to `NOW-PASSES`, the runner compares the binary it just ran against the
stamp and says which of two very different things happened:

* **binary unchanged** — the fixture is asserting something the shells do not
  do. It was wrong, or it was written against a transient state.
* **binary rebuilt** — somebody most likely fixed the bug. Retire the fixture
  with the run that shows it passing; that is a result, not a failure of the
  evidence base.

Both exit 1. The diagnosis is what differs, and it is free to compute.

## The reference-defect fixture

`reference_crash_uppercase_autoload.json` is a different kind of fixture: it
asserts that **the reference shell crashes**. `zsh 5.9.2`, under the init the
harnesses build (the full user `fpath` plus `compinit -C -d` the zpwr dump),
dies on `BATCH <TAB>` — measured here 3/3 SIGSEGV at ~2.8s, with
`BootCacheControl -` 3/3 (2× SIGSEGV, 1× SIGBUS) and the lowercase control
`batch ` 0/3, completing in ~13s. `scripts/comptab_parity.py` independently
labels the same cell `REF-CRASHED`.

It is replayed by `tests/compsys_fixtures/zsh_reference_probe.py`, which boots
**only** zsh: a claim about the reference shell must not depend on a second
shell being present to make it.

**It is not attempted by default** (`default_run: false`). Running it crashes a
shell on purpose and asks macOS to write a crash report every trial, and it can
prove nothing about zshrs. It is not silently treated as a pass either: the
runner reports it `SKIPPED`, names it in the summary, and counts it as neither
unchanged nor moved. Run it deliberately — after a zsh upgrade, say — with
`--reference-defects`, to find out whether the seven cells it covers can
re-enter the sweep.

Why pin it at all: a round-2 sweep scored those seven cells `TIMEOUT`, i.e.
"nothing was proven", when what actually happened is that zsh died. Without this
fixture the next sweep makes the same mistake, and a dead shell keeps being
read as a slow one.

## What is deliberately not here

* `target/spec-fuzz-probe/unstable_ref.zsh` — a completer whose own output is
  random. It exists to prove `compsys_spec_fuzz`'s `SKIP(unstable-reference)`
  branch fires on evidence, which is an assertion about the harness, not a
  zshrs divergence.
* The `seed_*.json` corpus under `scripts/parity_corpus_fuzz/` — mechanically
  regenerable, and already excluded there.
* `ssh -`, which the round-2 notes listed with the `+N possibilities` family.
  It was re-run on 2026-08-30 and PASSED (byte-identical, exit 0), so there is
  nothing to pin. Recorded here rather than dropped silently: a claim that
  stopped reproducing is a result.
* `CC -` — one of the seven round-2 `TIMEOUT` cells, but NOT a crash: the shell
  stays alive and produces no output at all for 180s. That is a genuine hang
  and needs a different kind of fixture than either of the two kinds here.
* `combo_7_1` and `combo_11_0_1` from `$TMPDIR/compsys_parity_failing_combos_*`
  — their zstyle subsets survived but their buffers did not, and the RNG stream
  that generated those buffers has since drifted, so the original cells cannot
  be reconstructed. Both subsets were probed against a buffer matching the
  surface recorded in their header and both PASSED, which is not evidence of
  anything except that the probe buffer was not the original one. The third,
  `combo_7_2`, did reproduce a divergence under a probe buffer and is pinned as
  `narrow_terminal_error_redraw` on that buffer's own evidence.
* The round-3 claim that the heredoc continuation cell diverges on
  `ZSH_EVAL_CONTEXT` by a `loadautofunc` frame. The cell itself reproduces and
  is pinned (`multiline_heredoc_terminator`), but that specific comparison did
  not survive contact: zsh's listing does show
  `ZSH_EVAL_CONTEXT ///////// shfunc:loadautofunc:shfunc:shfunc:`, while zshrs
  produced no parameter listing at all, so there was no zshrs value to compare
  against. The fixture pins the screen divergence that was observed and says so.
