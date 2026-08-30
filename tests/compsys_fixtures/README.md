# tests/compsys_fixtures — the durable completion-parity evidence base

One JSON file per confirmed finding: a zsh-vs-zshrs completion divergence, or —
clearly marked as such — a defect in the REFERENCE shell that the harnesses run
into, or a divergence that has since been FIXED and is kept here as a guard
against its return. `scripts/compsys_regressions.py` replays the whole set and
is the single gate that says whether the pinned evidence still holds.

Which of the three a file is, is `expect`: `diverges`, `reference-crash`, or
`agrees`. A fixed finding is never deleted — flipped to `agrees`, the same cell
that once demonstrated the bug becomes the regression test for the fix, and the
gate fails if the two shells stop matching again.

## Where the set stands

Every number below is read off `last_gate.json` — the full gate run checked in
beside these fixtures — and none of it is typed from memory. It describes THAT
run and nothing else; re-derive it any time with
`scripts/compsys_regressions.py --json out.json`, or read the document.

The run it records: 2026-08-30, source at `44a3b4841c`, zshrs 0.12.49
(`723088f811cdd366`) against zsh 5.9.2, `--jobs 1`, load average 5.8, 379.0s,
exit 0.

| | |
| --- | --- |
| fixtures pinned | 29 |
| cells attempted | 47 — 29 fixture cells and 18 controls |
| still diverging | 22 |
| retired as guards, now asserting the two shells AGREE | 6 |
| controls holding | 18 |
| moved, or could not be scored | 0, and 0 |
| opt-in and skipped | 1 — the upstream zsh crash |
| cells that needed a re-run | 0 |

The six guards, and the commit each one now protects: `d555917f07` for
`argv_append_discards_positionals`; `44a3b4841c` for
`explanation_percent_escapes`, `format_style_percent_escapes`,
`listing_row_erase_to_eol`, `listing_lost_on_window_shrink` and
`transpose_words_panic`. Each was re-verified three times before its `expect`
was flipped, and each was then run against a zshrs that still carries its bug
to check that it fails there — see `guard_verified` in the fixture.

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
| `title` | one line, what the two shells do differently. A retired fixture keeps the description of the bug and prefixes it `GUARD (fixed in COMMIT) — was:` |
| `harness` | `compsys_spec_fuzz`, `comptab_parity`, `compsys_parity`, `zsh_reference_probe`, `shell_probe` or `winch_probe` — which one owns the replay |
| `run.buffer` / `run.keys` | typed, then each key sent in order. Only `compsys_parity` buffers may contain a newline (the continuation cells) |
| `run.flags` | extra harness flags the finding needs (`--strict-stream`, `--compare-attrs`, `--strict-cursor`) |
| `run.zstyle` | statements written to a temp file and sourced by both shells |
| `run.zstyle_file` | repo-relative zstyle fixture (e.g. `scripts/parity_zstyle.zsh`) when the finding needs the real config. Neither field present means **no** styles — `compsys_parity` is passed `--no-zstyle`, because its own default is the repo fixture |
| `run.rows` / `run.cols` | geometry, only when the finding depends on it |
| `run.spec` | `compsys_spec_fuzz` only: `cmd`, `kind`, `widget`, `setup` and the completer source, re-materialised into a `--replay` file. `widget` and `setup` are load-bearing — three fixtures ARE about the widget, and a reproducer without them replays the default TAB binding, which is those fixtures' own control |
| `run.word` / `run.control_word` / `run.trials` | `zsh_reference_probe` only |
| `run.script` / `run.files` / `run.dirs` / `run.argv` / `run.env` / `run.compare_stderr` | `shell_probe` only: the script both shells run under `-f`, plus any files or directories (with modes) it needs materialised beside it. `compare_stderr` is off by default — see below |
| `run.new_rows` / `run.new_cols` | `winch_probe` only: the geometry the window is changed TO, mid-cell, after the completion has been drawn |
| `expect` | `diverges`, `agrees` for a fixed finding kept as a guard, or `reference-crash` for an upstream defect |
| `retired` | present only on an `agrees` fixture: the date, the `fix_commit` and its subject, `why` that commit fixed this cell, and the `evidence` — the verdict sequence the flip was made on |
| `guard_verified` | the run that proves the guard guards: an older zshrs that still carries the bug, the `CONTROL-MOVED` it produced there, and the harness detail — which must be the same one `observed_when_diverging` records, or the guard is failing on something else |
| `fingerprint` | `comptab_parity`'s stable id for the failure shape, or `null` when the owning harness does not compute one. A retired fixture's shape id moves to `fingerprint_when_diverging`: a passing cell has no failure shape, and leaving one there would ask the runner to match a shape that can no longer occur |
| `observed` | the recorded verdict, the differing rows verbatim, one-sided diagnostics, raw-stream fragments, and prose notes |
| `observed_when_diverging` | on a retired fixture, the `observed` block exactly as it was measured before the fix, plus `recorded_under` naming the commit and binary it was measured against. It is history; `observed` always describes what the shells do NOW |
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

### Retiring a fixed fixture as a guard

When the diagnosis is "binary rebuilt", the fixture is retired — **flipped, not
deleted**. `expect` goes `diverges` -> `agrees`, and from then on the gate
asserts the two shells MATCH on that cell: a `PASS` scores `CONTROL-HOLDS`, and
a `FAIL` scores `CONTROL-MOVED` and fails the run. The cell that demonstrated
the bug becomes the regression test for the fix, at no extra runtime and with
the reproducer already written.

What a retirement must carry, and why each part:

* **A first-hand re-verification**, not a report of one. The flip is a claim
  that a bug is gone; it is worth exactly as much as the run behind it.
* **The divergent `observed` block preserved** as `observed_when_diverging`,
  with `recorded_under` naming the commit and binary it was measured against.
  Deleting it would throw away the only description of the bug; leaving it in
  `observed` would present a measurement from a previous binary as current
  fact.
* **The fix commit named** in `retired.fix_commit`, with `why` saying what in
  that commit reaches this cell. "It passes now" is not an attribution.
* **The previous stamp pushed into `history`**, and a new `confirmed` for the
  run that showed it passing.
* **Every variant and control checked too.** A variant can still diverge when
  the parent passes — that is a PARTIAL fix and is the most interesting thing
  a retirement run can find. Variants inherit the fixture's `expect`, so
  flipping a fixture whose variants still fail turns a live finding into an
  intermittent gate failure. Measure them before flipping, and say what they
  did.

To check that a guard actually guards, point the runner at an older build:

```sh
scripts/compsys_regressions.py --zshrs /opt/homebrew/bin/zshrs \
    --only argv_append_discards_positionals
```

The binary is passed through to every harness, so this really does run the old
one; a guard whose bug is present there reports `CONTROL-MOVED` and exits 1. A
guard that stays green against a binary known to carry the bug is not a guard,
and the flip that produced it was wrong.

## The two fixtures that are not pty cells

Most findings here are a screen: a buffer, some keys, and the rows the two
shells drew. Two are not, and forcing them into a pty cell would have made them
worse evidence.

`shell_probe.py` — one script, two shells, **no pty**. `argv+=( ... )` losing
the positional parameters was a parameter bug in the shell core (fixed in
`d555917f07`; the cell is now a guard); it earned its
place in a *completion* evidence base only because
`Completion/Base/Core/_description:83` builds its `zformat` spec list with
exactly that append, so it reprices every description compsys renders. Pinned
through a pty harness the cell would assert a screen — and a screen carries the
prompt, the geometry, the listing layout and the rest of the completion system
with it, a dozen ways to change shape for reasons that have nothing to do with
the bug. Two lines of `print -l` isolate the same defect in **0.2s** against
8-25s, and cannot be moved by a layout change. STDERR is compared only when the
fixture asks (`compare_stderr`): the two shells prefix diagnostics differently
(`probe.zsh:2:` against `zsh:2:`), a real divergence but not the one any of
these cells is about, and one that would otherwise fail every stderr-carrying
cell for the same uninteresting reason.

`winch_probe.py` — completes, then **changes the window size** mid-session
(`TIOCSWINSZ`, then an explicit `SIGWINCH`, so a shell that only reacts to the
signal and one that re-reads the size on its next redraw get the same chance).
The three sibling harnesses each set the geometry once, before the shell boots,
and never touch it again, so none of them can reach a defect that needs a
resize. What it compares is how many non-blank rows survive the resize on each
shell, not the exact rendering — two shells legitimately re-lay a listing out
once the width changes, and that argument belongs in a different cell.

Both emit the same `{"results": [ ... ]}` document the pty harnesses emit, with
a `status` of `PASS`/`FAIL`, so `compsys_regressions.py` scores them through
the identical code path: there is no second verdict implementation to get
wrong. Both live HERE rather than in `scripts/`, because they are part of the
evidence base and are versioned with it — `--harness-dir` does not point at
them.

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
* Two round-5 `compinit` claims that did **not** reproduce in round 6, against
  `zshrs 0.12.48`: that `compinit -i` fails to drop an insecure (0777)
  directory, and that `compinit -d FILE` writes no dump. Probed with zsh's own
  shipped `compinit`/`compaudit`/`compdump` (copied to a 0755 directory, since
  Homebrew's own `functions` directory is mode 0777 here and `-i` prunes it out
  from under the run — which is its own trap), an fpath of
  `(fns good bad)` and `bad` at 0777: **both shells** report
  `nfpath=2 bad_in_fpath=0 zz01=<unset> secure=yes dump_exists=yes` under `-i`,
  and both report `nfpath=3 ... zz01=_zz01 secure=<unset> dump_exists=yes`
  under `-u` and under `-C`. Five configurations agreed: that one, the same
  with the world-writable Homebrew functions directory in `fpath` (both shells
  fail identically, `compinit:483: compdump: function definition file not
  found`), a group-writable (0775) bad directory, an insecure 0777 *file* in a
  secure directory (neither shell flags it), and no `compinit` in `fpath` at
  all (neither shell has a native one: `command not found` on both). No commit
  in the log names a `compinit -i` fix, so whether a peer fixed this since
  round 5 or the original probe was mis-built is **not established** — what is
  established is that it does not reproduce now.

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
