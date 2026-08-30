# tests/compsys_fixtures — the durable completion-parity divergence set

One JSON file per confirmed zsh-vs-zshrs completion divergence. Replay the
whole set with:

```sh
scripts/compsys_regressions.py            # every fixture
scripts/compsys_regressions.py --list     # what is pinned, boots no shells
scripts/compsys_regressions.py --variants # the extra witnesses too
```

Exit status is 0 only when every fixture still diverges with the fingerprint it
recorded. A change in either direction is non-zero, because a fixture asserting
a bug that no longer exists is a false claim, and a fixture that starts failing
in a new shape is no longer describing the bug it was written for.

## Why this directory exists

The completion fuzzers write their minimised reproducers to `target/` and to
`$TMPDIR`. `.gitignore:1` is `/target`, so `cargo clean` — or the OS reaping
`/var/folders` — destroyed every reproducer two rounds of fuzzing produced. The
findings survived only as prose in `scripts/comptab_divergent_cases.txt` and as
the `fp_*.json` inputs under `scripts/parity_corpus_fuzz/`, neither of which
carries a generated completer's source.

A fixture here carries everything needed to reproduce the cell with no other
file present: the buffer, the key sequence, the zstyle statements or the
completer source, the geometry when it is load-bearing, and the difference **as
it was observed** — not as it was theorised.

**Harnesses should write new reproducers here rather than under `target/` or
`$TMPDIR`.** That change belongs in the harnesses themselves
(`compsys_spec_fuzz.py`, `comptab_parity.py`, `compsys_parity.py`) and is
recorded as a recommendation, not made here.

## Format (`"schema": "compsys-fixture/1"`)

| field | meaning |
| ----- | ------- |
| `id` | stable name; also the filename |
| `title` | one line, what the two shells do differently |
| `harness` | `compsys_spec_fuzz` or `comptab_parity` — which one owns the replay |
| `run.buffer` / `run.keys` | typed, then each key sent in order |
| `run.flags` | extra harness flags the divergence needs (`--strict-stream`, `--compare-attrs`, `--strict-cursor`) |
| `run.zstyle` | statements sourced by both shells (`comptab_parity`) |
| `run.rows` / `run.cols` | geometry, only when the divergence depends on it |
| `run.spec` | `compsys_spec_fuzz` only: `cmd`, `kind`, `setup`, and the completer source, re-materialised into a `--replay` file |
| `expect` | `diverges` |
| `fingerprint` | `comptab_parity`'s stable id for the failure shape, or `null` when the owning harness does not compute one |
| `observed` | the recorded verdict, the differing rows verbatim, one-sided diagnostics, raw-stream fragments, and prose notes |
| `variants` | further confirmed witnesses of the same divergence; replayed only under `--variants`, and each may override any `run` field. A variant carries its OWN `fingerprint`, because `comptab_parity` fingerprints the failure shape and the same defect reached through a different command legitimately produces a different one |
| `origin` | where the reproducer was harvested from |
| `confirmed` | date, commit, harness file hashes, and both shell versions at the last confirmation |

`observed` is evidence, not a target. It is never edited to make a run look
better; when behaviour changes, the fixture is re-confirmed or moved out of the
bug set with the run that shows the change.

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
* `BATCH `, `BATCH -`, `BootCacheControl `, `BootCacheControl -`, `CC -`,
  `FileInfo `, `FileInfo -` — the seven cells a round-2 sweep scored `TIMEOUT`.
  These are not zshrs divergences: the REFERENCE shell dies. Under the exact
  init the harness builds (the full user `fpath` plus
  `compinit -C -d ~/.zpwr/local/.zcompdump-zpwr-MenkeTechnologies`), `zsh 5.9.2`
  takes SIGSEGV (occasionally SIGBUS) in `stripkshdef` <- `loadautofn` while
  autoloading the completer — `BATCH ` did so on 5 of 5 trials,
  `BootCacheControl -` on 3 of 3, with a matching `.ips` crash report per run
  under `~/Library/Logs/DiagnosticReports/`. `CC -` does not crash; it produces
  no output at all for 180s with the process still alive. The trigger is the
  `.zwc` digest: holding the dump constant and replacing the `more_src`
  directory with a plain copy of the same completer files and no `.zwc`
  alongside, `BATCH <TAB>` completes in 1.05s on 3 of 3 trials. The lowercase
  spellings (`batch `, `bootcachecontrol `, `fileinfo `, `cc -`) never crash.
  Nothing here can be pinned as a parity fixture, because there is no reference
  behaviour to compare against.
* `combo_7_1` and `combo_11_0_1` from `$TMPDIR/compsys_parity_failing_combos_*`
  — their zstyle subsets survived but their buffers did not, and the RNG stream
  that generated those buffers has since drifted, so the original cells cannot
  be reconstructed. Both subsets were probed against a buffer matching the
  surface recorded in their header and both PASSED, which is not evidence of
  anything except that the probe buffer was not the original one. The third,
  `combo_7_2`, did reproduce a divergence under a probe buffer and is pinned as
  `narrow_terminal_error_redraw` on that buffer's own evidence.
