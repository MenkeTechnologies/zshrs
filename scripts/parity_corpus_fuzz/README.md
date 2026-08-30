# parity_corpus_fuzz — the completion fuzzer's persistent corpus

Inputs for `scripts/comptab_parity.py --mutate N`, and the place all three fuzz
modes (`--mutate`, `--style-fuzz`, `--guided`) write what they find. One small
JSON file per input; an input is a complete cell:

```json
{
  "buffer": "CC ",
  "keys": ["tab"],
  "statements": ["zstyle ':completion:*' completer _expand _ignored ..."],
  "origin": "promoted/mut003",
  "fingerprint": "19376c78e1",
  "note": "cell/row0/col0 | ref/$ CC _main_complete:#: command not found: ..."
}
```

`buffer` is typed, then `keys` are sent one at a time, with `statements`
sourced as the zstyle config. `fingerprint` is set only on a **promoted**
entry — one the fuzzer produced itself.

## Why the directory exists

A one-shot random sweep re-rolls the same dice every run. It cannot get better
at finding bugs, and everything it found dies with the process. This directory
is the memory: when a run hits a failure fingerprint that is not already in
here, the reproducer is minimised in all three dimensions (zstyle statements,
key sequence, buffer) and written back as `fp_<hash>.json`, so the next run
starts from it and mutates around known-bad territory instead of around the
seeds.

Mutation parents are drawn with weights, not uniformly — evidence earns weight:

| entry                                | weight | what it is                              |
| ------------------------------------ | -----: | --------------------------------------- |
| carries a `fingerprint`              |     12 | a reproducer a fuzz run mined and shrank — `origin` is `promoted/*` from `--mutate`, `style-fuzz/*` from `--style-fuzz`, `guided/*` from `--guided` |
| `origin` starts `divergent-cases/`   |      6 | a buffer from `comptab_divergent_cases.txt`, each serially confirmed |
| `origin` starts `cov/`               |      3 | kept by `--guided` because it reached a screen shape or an engine path nothing else in the corpus reached. It did not fail, so it has no fingerprint — but it is the only entry that goes there |
| `origin` starts `CASES/`             |      1 | a hand-written case from `parity_corpus.py` |

Measured caveat on that table: in the back-to-back runs recorded below, the
weighted draw did not beat a uniform one at a 14-cell budget — it lost on one
seed and won on the other. The 12x on a promoted reproducer concentrates the
fuzzer on the neighbourhood of bugs it already has, which is the intent for a
long run and may be the wrong trade for a short one. Treat the weights as
unvalidated at small budgets.

## What is tracked in git, and what is not

* **Tracked: `fp_*.json` (+ its `*_styles.zsh` sidecar).** These are the
  accumulated findings. They cannot be regenerated from anything — they are
  what the fuzzer learned.
* **Tracked: `cov_*.json`.** Inputs `--guided` kept for INFORMATION — each one
  was the first input in the corpus's history to produce some feature. Capped
  at `--cov-corpus-max` (160); at the cap the least informative one is evicted,
  never an `fp_*`.
* **Not tracked: `seed_*.json`.** 500+ files that are a mechanical product of
  `parity_corpus.CASES` x `--seed-sequences` plus
  `scripts/comptab_divergent_cases.txt`. Regenerate them in one command; there
  is nothing in them git does not already hold.

```sh
scripts/comptab_parity.py --corpus-seed                       # no zstyles
scripts/comptab_parity.py --corpus-seed --zstyle scripts/parity_zstyle.zsh
```

## Running the fuzzer

Three fuzzers write here. `--mutate` starts from what the corpus already holds
and steps just off it. `--style-fuzz` GENERATES zstyle statements instead of
sampling a fixture, which is the only way the VALUE grammar of a style
(match specifications, completer chain order, tag-order, menu, format,
list-colors, ...) gets exercised at all — subset sampling can only ever replay
values that were already in `scripts/parity_zstyle.zsh`.

```sh
# 20 mutated inputs, anywhere in the corpus
scripts/comptab_parity.py --mutate 20

# fuzz around the mined divergences only
scripts/comptab_parity.py --mutate 20 --corpus-origin divergent-cases

# fuzz around what the fuzzer already found
scripts/comptab_parity.py --mutate 20 --corpus-origin promoted

# 20 GENERATED zstyle configurations, 5 statements each
scripts/comptab_parity.py --style-fuzz 20 --style-fuzz-styles 5

# hammer one surface — every statement is a generated matcher-list
scripts/comptab_parity.py --style-fuzz 40 --style-fuzz-only matcher-list

# compose: a generated config layered on a subset of the real fixture
scripts/comptab_parity.py --style-fuzz 20 --style-fuzz-mix 0.3 \
                          --zstyle scripts/parity_zstyle.zsh

# see what the generator emits, and what zsh makes of it — no shells booted
scripts/comptab_parity.py --style-fuzz-list 40

# replay one promoted reproducer exactly
scripts/comptab_parity.py --zstyle scripts/parity_corpus_fuzz/fp_<hash>_styles.zsh \
                          --case '<buffer>' --keys <keys>
```

Each cell boots two real shells (`zsh -f -i` and `zshrs -f -i`) on their own
pty, so budget ~5-30s per cell and expect a shrink to spend up to
`--shrink-probes` cells more per new fingerprint.

## Verdicts

`PASS` / `FAIL` are the comparison. `TIMEOUT` means a side ran out of
measurement budget, so the two screens were never both final — counted and
printed separately, re-measured once serially, and never scored as a pass.
`SKIP` means the case's command is not installed on this host, so neither shell
can reach a completer; also never scored as a pass, and ON by default
(`--no-skip-missing` reverts to scoring "both rendered nothing" as a pass).

`--style-fuzz` adds two more, both of which exist so that a config the
REFERENCE shell will not accept can never be dressed up as evidence:

* `INVALID-CONFIG` — zsh's own `zstyle` refused the generated statement at
  definition time (an invalid context pattern). The cell is not run at all,
  because comparing two shells on a config neither can hold says nothing. This
  is a bug in the generator, not in zshrs.
* `REF-REFUSED` — the statement parsed, but the reference zsh complained about
  the VALUE at completion time (an unknown match-specification character, an
  unterminated character class, a completer that does not exist). The cell IS
  still run and still compared — zshrs is required to refuse it identically —
  but it is tallied apart from the clean passes.

Two more exist for the case where a shell does not survive the cell:

* `REF-CRASHED` — the REFERENCE zsh died (SIGSEGV / SIGBUS / SIGABRT / ..., or
  a crash marker on its own terminal). That is an upstream zsh bug, not a
  zshrs divergence, and it is emphatically not a TIMEOUT: a dead shell is not
  a slow shell. Before this category existed, seven such cells were labelled
  "budget exhausted, not a divergence", which is how a real zsh crash
  (`stripkshdef` <- `loadautofn`, faulting on a large `.zwc` digest in
  `fpath` — `BATCH <TAB>` reproduces it 5/5) sat unnoticed across two rounds
  of sweeps.
* `TEST-CRASHED` — zshrs died. A zshrs bug, and the repro is printed.

Neither is fingerprinted: there was no second screen to diverge from, so
grouping one as a bug shape would invent a divergence out of a dead process.
Both are counted, printed with the signal and a pointer to the OS's diagnostic
report, and keep the exit status non-zero. A cell where nothing crashed but a
side ran out of budget is still a `TIMEOUT` — `CC -` (zero bytes for 180s,
process alive) is the reference example of that.

Only a `PASS` under a config zsh accepted is evidence of parity, and every
other category keeps the exit status non-zero.


## Coverage-guided fuzzing (`--guided`)

`--mutate` and `--style-fuzz` are BLIND: they keep an input only if it FAILED,
so an input that was the first ever to reach `_approximate`, render a described
listing or draw a menu is thrown away the moment it passes. `--guided` keeps an
input that produced INFORMATION, which is what makes the corpus a seed pool
rather than a bug list.

```sh
# 20 cells, guided by output shape alone — no instrumentation, any --jobs
scripts/comptab_parity.py --guided 20

# ...plus zshrs's own execution trace. Serial only; see below
scripts/comptab_parity.py --guided 20 --cov-log --jobs 1

# the blind control, for measuring whether guidance is worth anything
scripts/comptab_parity.py --guided 20 --guide-off --seed 9090
```

### Two signals

**Shape** (always on) is computed from the two `Capture`s the harness already
takes: how many rows were drawn, whether a listing appeared, whether it had
description columns and how many, how many prompts are on screen, what the
command line itself became, where the cursor landed, how many distinct SGR
signatures the screen carries. It costs nothing, cannot perturb the run, works
at any `--jobs`, and describes the reference shell as well as zshrs. It is a
proxy for the code path, not the code path.

**Engine trace** (`--cov-log`) is the real thing. `ZSHRS_LOG=compsys_args=debug`
turns on 39 `tracing::debug!` sites that name the completer that resolved, the
tag context, the `_arguments` gate outcome, the `addmatches` candidate loop and
the `do_completion` branch; `ftime` on the same variable makes a Drop guard in
`docomplete` dump the per-function inclusive times to `/tmp/ftime.log`, whose
NAMES are the set of compsys shell functions the completion entered. Measured
on `--case 'git ' --keys tab`: 8306 bytes of trace, ~56 engine features and ~50
function features per cell, and no measurable cost (4.74/4.72/4.73 s without,
4.61/4.73/4.77 s with).

It is opt-in and forces `--jobs 1` because zshrs's log is a single shared
append-only file (`$ZSHRS_HOME/zshrs.log` — the path is not separately
configurable, `src/extensions/log.rs:53-67`) and only its `zshrs starting`
line carries a pid, so a byte slice can be attributed to a cell only while this
harness is the log's sole writer. A slice containing more shell boots than the
run started is counted and reported rather than silently trusted.

### The instrumentation is never allowed to become evidence

`--cov-log` has to put `ZSHRS_LOG` into the child environment, and a completion
that enumerates the environment renders it as a MATCH — `tee >(<TAB><TAB>`
reaches the parameter listing and did exactly that, manufacturing a
"fingerprint" whose own text was `ZSHRS_LOG -- compsys_args=debug,ftime`. Both
shells get the identical env so a PASS is still a valid PASS, but that
reproducer would not have replayed from the command printed beside it. So every
new fingerprint is re-measured once on the un-instrumented env before it is
written down, and the shrink runs on that env too. A divergence that does not
survive is reported as an artefact and withheld. This can only ever remove a
reproducer the fuzzer would have claimed.

### What a run reports

Distinct features seen, the discovery curve, features per cell, how many inputs
were retained for information versus promoted for failing, and a per-class
yield table — cells, seconds, features and fingerprints per input class (style
axis, case class, key path, mutation kind), sorted by reward rate. That table
is a finding in its own right: it says where the budget went and what it
bought.

### Does guidance actually win? Measured: at 14 cells, it is noise

Protocol, both seeds: identical corpus state at the start of each pair (the
control's own promotions were moved aside before the guided run), identical
seed, `--cov-log` on for BOTH so the same signal is measured on both sides and
only the guided side ACTS on it, 14 cells each.

| seed | run | diverging cells | new fingerprints | features | features/cell |
| ---: | --- | ---: | ---: | ---: | ---: |
| 9090 | blind (`--guide-off`, uniform draw) | 4 | 3 | 270 | 19.29 |
| 9090 | guided                              | 0 | 0 | 234 | 16.71 |
| 3131 | blind (`--guide-off`, uniform draw) | 1 | 1 | 255 | 18.21 |
| 3131 | guided                              | 3 | 1 | 278 | 19.86 |

The two seeds disagree in opposite directions, so at this budget the difference
is sampling noise and **no claim that guidance wins is supported by this data**.
14 cells across ~20 input classes leaves most classes at n=1, which is not
enough to estimate a rate; and 75% of draws are exploit draws weighted by
`corpus_weight` x yield, where `corpus_weight` already puts 12x on promoted
reproducers — so early on, guidance mostly re-expresses the existing weighting.

What IS established, and does not depend on the comparison: the retention half
works and is nearly free. Guided runs kept 12, 14 and 11 inputs for information
across the three runs above; blind runs kept none, by construction. Those
entries are territory the corpus did not previously reach, and they persist.
Settling whether the scheduling half pays needs a run one or two orders of
magnitude larger — at ~15 s/cell that is hours, which is why `--guide-off`
exists rather than an assertion.
