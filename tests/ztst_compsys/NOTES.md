# Upstream ztst suites run against zshrs

zsh ships its own test suite with its own expected output. This directory runs
that suite against zshrs and pins the result, so a later run can tell "this
regressed" from "this was always failing" — and, just as importantly, "this was
quietly fixed and the fixture asserting it is now a false claim".

The runner is `scripts/ztst_compsys.py`. Every number below is derived from the
artifacts in this directory or from a command whose output is quoted next to it;
none is typed by hand.

Two suites are covered, and they must not be mixed:

| | files | what it measures |
|---|---|---|
| **Y series** (`Y01`-`Y06`) | 6 | the completion system, driven through a pty by `Test/comptest` |
| **core** (everything else) | 70 | shell parity — grammar, builtins, expansion, globbing, options, modules, ZLE |

`ls <zsh-tree>/Test/*.ztst | wc -l` = 76 = 6 + 70.

## Why upstream's suite

Every other compsys harness here (`compsys_parity.py`, `comptab_parity.py`,
`compsys_spec_fuzz.py`) compares against cases we invented. `Test/comptest`
takes `comptestinit -z <shell>` to choose which shell runs under the pty, so
pointing it at zshrs makes upstream the oracle: a failure is a compatibility gap
stated in upstream's own terms.

The non-Y files have no `comptestinit`. Their assertions run *in* the ztst
harness, so `--core` makes the shell under test its own harness. zshrs can
interpret `Test/ztst.zsh`, which is what makes that possible at all.

## Setup

| | |
|---|---|
| oracle tree | `~/forkedRepos/zsh` @ `599af4604f`, copied to scratch and built (`Util/preconfig && configure && make`) |
| oracle version | `5.9.999.3-test` (`Config/version.mk`) |
| baseline shell | that tree's `Src/zsh`; also the harness that interprets `ztst.zsh` for the Y series |
| candidate shell | `target/debug/zshrs` |
| date | 2026-08-30 |

The oracle tree must be *built*, and its `Completion/` must match its `Test/`: a
5.9.999-era `Completion/compinit` uses `${ ... }` nofork command substitution,
which zsh 5.9.2 rejects with `bad substitution`, killing `compdef` and hanging
the suite. Homebrew zsh 5.9.2 is therefore not usable as the baseline.

`zsh/zpty` is loaded by the *harness*, never by the shell under test, so zshrs
needing no zpty of its own is not a blocker for the Y series.

## How to run it

    P=/opt/homebrew/bin/python3            # pythonrs cannot parse this script
    B=/path/to/built/zsh/tree

    # Y series, and the zsh baseline
    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B --timeout 180
    $P scripts/ztst_compsys.py --baseline --zsh-build $B

    # gate: pin once, then compare
    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B --timeout 180 --pin
    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B --timeout 180 --gate

    # reduce a failing assertion to a standalone repro
    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B \
        --minimize Y03arguments#4 --repro-dir tests/ztst_compsys/repros

    # wider suite (shell parity, its own pin file)
    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B --core --pin

Gate exit codes: `0` unchanged, `1` something regressed, `2` something moved
without regressing, `3` the runner itself failed. They are distinct on purpose —
argparse usage errors and missing-pin errors also exit `3`, never `1` or `2`.

## Y series

### Baseline: the oracle is clean on this host

    files=6 assertions=229 pass=222 xfail=7

Every file exits 0; the whole baseline takes 8.6 s. The 7 non-passes are
upstream's own `f`-flagged expected-to-fail assertions in Y02compmatch
(`r:[^.]||.=*` and friends), which fail identically for zshrs. There is no
host-specific baseline failure to subtract.

### zshrs (`--fx off`)

`tests/ztst_compsys/zshrs_0.12.49_fxoff.txt`:

    files=6 assertions=229 fail=93 notrun=1 pass=32 unknown=96 xfail=7

| file | baseline | zshrs |
|---|---|---|
| Y01completion | pass 33 | pass 10, fail 23 |
| Y02compmatch | pass 51, xfail 7 | pass 18, fail 33, xfail 7 |
| Y03arguments | pass 100 | pass 1, fail 2, then **timed out** (96 unrun) |
| Y04regexargs | pass 6 | pass 1, fail 5 |
| Y05describe | pass 7 | fail 7 |
| Y06values | pass 25 | pass 2, fail 23 |

Y03arguments does not fail, it hangs: at assertion 4 the shell under test exits
and the driver blocks forever waiting for its finish widget. `--timeout 180`
reproduces exactly the same per-assertion statuses as round 5's `--timeout 900`
(verified by gating one against the other, below), so the pin uses 180 s and
records the timeout as part of the pinned state.

### The pin has not gone stale

`gate.json` records every assertion's status plus the identity of the binary
that produced it. Three gate runs, three different binaries:

| pinned | run against | verdict |
|---|---|---|
| zshrs 0.12.46 (round 5) | 0.12.48 | `UNCHANGED=229`, binary moved: yes |
| zshrs 0.12.48 | 0.12.49 `6420c23c` | `UNCHANGED=229`, binary moved: yes |
| zshrs 0.12.49 `92740ec4` | itself, twice | `UNCHANGED=229` both times, exit 0 |

So none of round 5's 93 Y-series findings has been fixed out from under the
ledger. The `argv+=` divergence was re-checked directly against 0.12.48 as well:

    f() { argv+=( X ); print -r -- "n=$# argv=<${(j:,:)argv}>" }; f 1 2 3; f 1 2 3
    zsh    n=4 argv=<1,2,3,X>   (twice)
    zshrs  n=1 argv=<X> / n=2 argv=<X,X>

### What the gate caught while it was being built

One gate run reported `REGRESSED=1 CHANGED=5` — Y04regexargs went from
`pass 1 fail 5` in 8.2 s to a 180 s hang with nothing run. The identity block
said `mtime: 06:21 -> 06:25`: a peer instance had relinked `target/debug/zshrs`
*during* the run. The next run over the finished binary was `UNCHANGED`.

That is a false positive the harness caused, so the runner now copies the shell
binary into the run directory once, up front, and runs every file against that
copy (`--no-sut-snapshot` opts out). Identity is still read from the original
path. The same copy is taken for a `--minimize` session, which is hundreds of
probes long.

## Assertion → minimal standalone repro

`--minimize FILE#N` turns one failing assertion into a runnable `.zsh` that
drives `comptestinit`/`comptest` directly — no ztst.zsh, no expected-output
block, just "run this against each shell and diff". Reduction is differential:
it shrinks the `%prep` setup, the preceding assertions and the keystroke string
while the two shells keep disagreeing.

Two properties keep it honest:

* **Anchored.** The first probe fixes a *divergence signature* — which kinds of
  comptest line (`line`, `DESCRIPTION`, `NO`, `INSERT_POSITIONS`, …) the two
  shells disagree about — and a candidate that diverges over different kinds is
  rejected. Without it the reducer happily deletes the setup that produces the
  interesting bug and reports whatever unrelated divergence is left: measured,
  Y01completion #16 slid off the `..` path bug onto the unrelated `h:`
  description bug.
* **Scoped to the assertion.** A marker is printed between the context and the
  target, and only output after it is compared. Without that, the earlier
  assertions' output is part of the comparison and no context chunk can ever be
  dropped.

Reduction is budgeted and the report says `converged` or `budget exhausted` —
never silently the former when it was the latter.

`minimization.txt` has the full before/after and both shells' output for five
assertions; `repros/` has the generated scripts. All five converged inside a
100-probe-pair budget. The bugs they isolate:

| repro | probes | what it shows |
|---|---|---|
| `y03arguments_004.zsh` | 41 | 3 positional specs + `\t\t^W^W^D`: zsh lists `arg1`/`arg2`; zshrs **hangs** |
| `y05describe_001.zsh` | 25 | `_describe` emits `DESCRIPTION:{h:desc}` instead of `{desc}` |
| `y06values_001.zsh` | 27 | `_values`, same `h:` leak |
| `y02compmatch_049.zsh` | 79 | `INSERT_POSITIONS:{4:5:6}` collapses to `{6}` |
| `y01completion_016.zsh` | 8 | completion stops at `cd "A(B)/` instead of `cd "A(B)/C/` |

Each was re-checked by hand against the live `target/debug/zshrs`; all five still
diverge there. `y03arguments_004.zsh` reproduces a hang, so wrap the second
command in `timeout 30` — zsh exits 0 with four lines, zshrs never returns.

Known limitation: removal is one element at a time, so a group of context
statements that only makes sense together cannot be dropped. That is why
`y02compmatch_049.zsh` still carries three earlier assertions — the later ones
reuse a variable an earlier one sets.

## Wider suite (shell parity — NOT compsys numbers)

These are the 70 non-Y `.ztst` files with zshrs as its own ztst harness. They
say nothing directly about completion; they measure the language compsys is
written in. Reported separately, and deliberately not folded into the Y-series
score.

### The zsh baseline is not clean here

`core_baseline_zsh_5.9.999.3-test.txt`:

    files=70 assertions=2523 fail=4 pass=2381 skip=2 unknown=115 xfail=21

Five files are skipped by their own `%prep` on this host — process substitution
unavailable, PRIVILEGED needs root, and `zsh/pcre`, `zsh/param/private`,
`zsh/db/gdbm` were disabled by `configure`. The 4 baseline failures are all in
V01zmodload and come from the module symlink farm this runner stages rather than
a real `make install.modules` (e.g. `no such module zsh/regex`). Everything is
scored against this baseline, never against 2523.

Two staging fixes were needed to get here, both of them things `make check`
already does and this runner was not doing (round 5 never ran these files):

* `ZTST_exe=../Src/zsh` in the environment (`Test/Makefile.in:56`). Without it,
  C03traps and friends run `$ZTST_exe -fc ...` as `-fc` and exit 127 — 6
  baseline failures that were the runner's fault, not the shell's.
* a `config.modules` symlink in the run root. Without it V01zmodload's whole
  `%prep` aborts and all 41 of its assertions never run.

### Scores

Of the 2381 assertions zsh passes, zshrs passes 1816 (76.3%), fails 376, and
never reaches 189 because the file hung or aborted first. It also passes 36
assertions the baseline could not attempt (`zsh/pcre` 11, `zsh/param/private`
23, `zsh/db/gdbm` 2 — modules zshrs has natively and this zsh build does not).

Per-file numbers are in `core_scoreboard.txt`; per-assertion verdicts in
`core_comparison_baseline_vs_zshrs.txt`; failure detail in
`core_zshrs_0.12.48_fxoff.failures.txt`.

Four files hang rather than fail — D04parameter, D07multibyte, W02jobs,
X06termquery all hit the 150 s per-file timeout — which is where most of the
189 unreached assertions come from.

The 0.12.49 pin in `gate_core.json` is per-assertion identical to the 0.12.48
run those `.txt` reports were produced from (checked across all 2523).

### Core failures that plausibly underlie compsys divergences

These are plausibility claims from the failure lists plus call-site counts, not
proven causes. In rough order of how directly compsys leans on them:

* **V13zformat — 21 of 35 failing.** `Completion/Base/Core/_description:89` is
  `zformat -F format "$format" "d:$1" ...`; `zformat` appears in 21 files under
  `Completion/`. The failures cluster on the `%(...)` ternary and on spec-error
  handling — exactly the machinery the `format` zstyle drives.
* **V12zparseopts — 12 of 32 failing** (`-M`, `-G`, long options, option
  stacking, missing optargs). `zparseopts` appears in 61 files under
  `Completion/` out of 1041.
* **D06subscript — 9 of 37 failing**, on pattern subscripts and on associative
  keys containing quotes. compsys indexes `$_comps[...]` and uses `(r)`/`(R)`
  subscripts throughout; the memory ledger already carries two open subscript
  entries.
* **B02typeset — 27 of 88 failing**, including #82-#87 on parameter hiding
  (`-h`, local-vs-special, autoload variables). `Test/comptest` itself runs
  `local +h -a comppostfuncs=( comptest-postfunc )` inside the widget that every
  Y-series assertion goes through.
* **X04zlehighlight — 20 of 20 failing**, `region_highlight`. Relevant to the
  ZLE-side ledger entries rather than to compsys proper.

And the honest counterpoint: the single biggest compsys-visible core bug —
`argv+=( ... )` discarding the positional parameters, which is what puts `h:` in
front of every description — has **no coverage anywhere in the 76-file corpus**
(`grep -n 'argv+=' Test/*.ztst` finds nothing). It took a completion test to
surface it, and the wider suite would never have found it.

## Adaptations (declared)

Nothing here edits, filters or relaxes an upstream assertion. `.ztst` files,
`Test/comptest` and `Test/ztst.zsh` are read-only inputs.

* `--fx off` exports `ZSHRS_NATIVE_ZLE_FX=0`, disabling zshrs's own autosuggest
  and syntax-highlight overlays, which are on by default even under `-f`.
  Measured in round 5: an `--fx on` run of Y01/Y05/Y06 produced identical
  per-file counts, so the knob does not move the score.
* The shell under test is reached through a symlink, never a wrapper script, and
  **that symlink's basename must be `zsh`**. zsh picks its emulation from the
  first character of `argv[0]` (`Src/options.c:533-548`: `s` or `b` means
  `EMULATE_SH`), so a link named `sut-00001` starts the shell under test in sh
  emulation and `comptestinit` then waits for a prompt forever. Measured: the
  same probe script returned 24 lines through a link named `abc` and nothing at
  all through `sut-a`. A `/bin/sh` wrapper fails for a second reason — it drops
  the exported `PS1`, which `comptest` keys every read on.
* The shell binary is copied into the run directory once per run (see above).
* `ZTST_exe` and `config.modules` are supplied as `make check` supplies them.
* Modules are symlinked from the build tree rather than `make install.modules`;
  V01zmodload notices the difference (4 baseline failures).
* A hung probe's shell escapes `killpg` — `zpty`'s child `setsid()`s to take the
  pty as its controlling terminal — so it is killed by the unique path it was
  launched through. Without that it survives at ~100% CPU and slows every later
  probe.

## Attributed divergences (Y series)

Round 5's clustering of the 93 failing assertions, all still reproducing on
0.12.49. Full detail and expected-vs-actual diffs are in
`zshrs_0.12.49_fxoff.failures.txt`.

1. **`argv+=( ... )` does not append to the positional parameters, and leaks.**
   Reprices every description in the completion system, because `_description`
   builds its `zformat` spec list with `argv+=( h:... )`. Repros:
   `repros/y05describe_001.zsh`, `repros/y06values_001.zsh`.
2. **`ZLS_COLORS` `lc=`/`rc=` are ignored when listing matches.** `ec=` is
   honoured; the `\e[` and `m` around the colour are hardcoded
   (`src/ported/zle/complist.rs:738-743` documents why). Upstream's assertions
   match on `<LC><xx><RC>...<EC>`, so every listed match disappears from the
   captured output — the largest single contributor to the failure count.
3. **`_arguments` does not advance past the first positional spec**, and hangs
   when the test then backs the buffer out. Repro:
   `repros/y03arguments_004.zsh`.
4. **`$compstate[insert_positions]` reports only the last position.** Repro:
   `repros/y02compmatch_049.zsh`.
5. **`compadd -M` reports one generic message for every malformed spec**
   (`missing word pattern` where zsh distinguishes `missing patterns`,
   `unterminated character class`, …). Y02compmatch #2-#8, #10-#13.
6. **Path completion through a `..` component stops at the `../`.** Repro:
   `repros/y01completion_016.zsh`.
7. **`_describe` with the unsupported `((...))` form** loses a space
   (Y05describe #2, cosmetic).

### Not a divergence

* **`zsh/zpty` in zshrs.** The driver's zpty is loaded by the harness shell, not
  by the shell under test, so this suite exercises none of it.
* **OSC 133 shell-integration escapes.** zshrs emits `\e]133;...`; so does
  upstream (`Src/Zle/termquery.c:688,754-757,781-782`).

## Files

| file | what it is |
|---|---|
| `baseline_zsh_5.9.999.3-test.{txt,json}` | the zsh Y-series baseline, per assertion |
| `zshrs_0.12.49_fxoff.{txt,failures.txt}` | zshrs Y series, and every failure's diff |
| `zshrs_0.12.46_fxoff.{txt,json,failures.txt}` | round 5's run, kept for provenance |
| `zshrs_0.12.46_fxon_Y01_Y05_Y06.txt` | round 5's `--fx on` control |
| `comparison_baseline_vs_zshrs.txt` | per-assertion baseline-vs-candidate verdicts |
| `gate.json` | the Y-series pin: per-assertion status + binary identity |
| `gate_0.12.46_to_0.12.48.txt` | the gate run proving round 5's pin is not stale |
| `minimization.txt` | assertion → minimal repro, with before/after and budgets |
| `repros/*.zsh` | the generated standalone repros |
| `core_baseline_zsh_5.9.999.3-test.txt` | zsh on the 70 non-Y files |
| `core_zshrs_0.12.48_fxoff.{txt,failures.txt}` | zshrs on the same |
| `core_comparison_baseline_vs_zshrs.txt` | per-assertion verdicts, core suite |
| `core_scoreboard.txt` | per-file: zsh-passing, zshrs ok / FAIL / not-run |
| `gate_core.json` | the core pin |
