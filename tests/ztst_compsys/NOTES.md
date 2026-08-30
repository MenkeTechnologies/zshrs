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

    # reduce a failing Y assertion to a standalone repro
    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B \
        --minimize Y03arguments#4 --repro-dir tests/ztst_compsys/repros

    # wider suite (shell parity, its own pin file)
    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B --core --pin

    # reduce every pinned core failure (no pty, ~40 ms a probe)
    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B \
        --core-minimize-from tests/ztst_compsys/gate_core.json \
        --core-minimize-skip X02zlevi,X04zlehighlight,X06termquery,W02jobs,V08zpty \
        --minimize-budget 60 --minimize-timeout 6 \
        --repro-dir /tmp/repros_core \
        --out tests/ztst_compsys/core_minimization.txt \
        --json tests/ztst_compsys/core_minimization.json

    # group failing assertions by root cause
    $P scripts/ztst_compsys.py --zsh-build $B \
        --cluster tests/ztst_compsys/core_minimization.json --cluster-min 2

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

`tests/ztst_compsys/zshrs_0.12.49_29ee728e_fxoff.txt`:

    files=6 assertions=229 fail=88 notrun=1 pass=37 unknown=96 xfail=7

| file | baseline | zshrs |
|---|---|---|
| Y01completion | pass 33 | pass 10, fail 23 |
| Y02compmatch | pass 51, xfail 7 | pass 22, fail 29, xfail 7 |
| Y03arguments | pass 100 | pass 1, fail 2, then **timed out** (96 unrun) |
| Y04regexargs | pass 6 | pass 1, fail 5 |
| Y05describe | pass 7 | fail 7 |
| Y06values | pass 25 | pass 3, fail 22 |

Y03arguments does not fail, it hangs: at assertion 4 the shell under test exits
and the driver blocks forever waiting for its finish widget. `--timeout 180`
reproduces exactly the same per-assertion statuses as round 5's `--timeout 900`
(verified by gating one against the other, below), so the pin uses 180 s and
records the timeout as part of the pinned state.

### Which binary

`target/debug/zshrs` is rewritten by peer instances of this repo while a run is
in progress, so every number here names the binary it came from:

| tag | mtime | what it is |
|---|---|---|
| `0.12.46` | round 5 | the first pin |
| `0.12.49 @06:25` | 2026-08-30 06:25 | round 6's pin, `gate.json` / `gate_core.json` |
| `0.12.49 @29ee728e` | 2026-08-30 07:58 | this round's runs |

Between @06:25 and @29ee728e a peer landed a fix for `argv+=( ... )`, which
round 5 and 6 had recorded as the top Y-series divergence:

    f() { argv+=( h:desc ); print -r -- "n=$# 1=<$1> 2=<$2>" }; f desc
    @06:25       n=1 1=<h:desc> 2=<>
    @29ee728e    n=2 1=<desc>   2=<h:desc>      (= zsh)

Both pins were retaken against @29ee728e, so the pin and every number in this
file describe the same bytes. The identity block records `sha256_prefix`,
`size` and `mtime`; the `path` it records is the copy in scratch the run used,
because `target/debug/zshrs` moved again (to `97c9df54`) 50 minutes later. The
sha256 is what makes the pin checkable, not the path.

All five named root causes below were re-checked directly against the *next*
binary again, `97c9df54` (08:48), and all five still reproduce there, so the
findings are not tied to the snapshot even though the counts are.

Y series moved `pass 32 -> 37`, `fail 93 -> 88`. The wider suite barely moved:
of the 431 assertions `gate_core.json` pinned as failing, exactly 3 now pass
(`A01grammar#29`, `E01options#18`, `V04features#21`) and none regressed, so the
reductions in `core_minimization.txt` -- taken against @06:25 -- still describe
@29ee728e for 428 of the 431.

### The pin has not gone stale

`gate.json` records every assertion's status plus the identity of the binary
that produced it. Three gate runs, three different binaries:

| pinned | run against | verdict |
|---|---|---|
| zshrs 0.12.46 (round 5) | 0.12.48 | `UNCHANGED=229`, binary moved: yes |
| zshrs 0.12.48 | 0.12.49 `6420c23c` | `UNCHANGED=229`, binary moved: yes |
| zshrs 0.12.49 `92740ec4` | itself, twice | `UNCHANGED=229` both times, exit 0 |

Through @06:25, none of round 5's 93 Y-series findings had been fixed out from
under the ledger. @29ee728e is the first binary where that stopped being true:
`argv+=` was fixed and five assertions moved to pass. That is exactly what the
gate exists to report — exit `2`, "something moved without regressing" — and
the pin has been retaken against @29ee728e, so a later gate run compares
against the current truth rather than against a claim the shell has outgrown.

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

Two reducers, because the two suites need different drivers.

`--minimize FILE#N` (Y series) turns a failing assertion into a runnable `.zsh`
that drives `comptestinit`/`comptest` directly -- no ztst.zsh, no expected-output
block, just "run this against each shell and diff". Reduction is differential:
it shrinks the `%prep` setup, the preceding assertions and the keystroke string
while the two shells keep disagreeing.

`--core-minimize FILE#N` (the other 70 files) does the same without a pty, and
so reduces to a *construct* rather than to a keystroke sequence. It replays each
chunk through the same anonymous-function wrapper `ZTST_execchunk` uses
(`ztst.zsh:301-305`) and reproduces the parts of `ztst.zsh` a chunk can observe
-- module path (48-50), `$fpath` (112-114), `zsh/parameter` (54), the
un-exported `WORDCHARS` (46), the option state captured at line 59, and the
`Src/zsh` symlink next to the test directory that `$ZTST_exe` and
`$ZTST_testdir/../Src/zsh` resolve through. Without that last piece the reducer
anchors on "no such file or directory" and reduces the driver instead of the
bug. A probe costs about 40 ms, against seconds for a pty probe.

Both keep two properties:

* **Anchored.** The first probe fixes what "still the same bug" means and a
  candidate that diverges differently is rejected. Y uses a *divergence
  signature* (which kinds of comptest line the shells disagree about); core
  uses a *witness* (one normalised line that must keep appearing on the same
  side and only on that side, preferring a status difference, then a
  diagnostic, then output). Without an anchor a reducer deletes the setup that
  produces the interesting bug and reports whatever unrelated divergence is
  left: measured, Y01completion#16 slid off the `..` path bug onto the
  unrelated `h:` description bug.
* **Scoped to the assertion.** A marker is printed between the context and the
  target, and only output after it is compared. Without that, the earlier
  assertions' output is part of the comparison and no context chunk can ever be
  dropped.

Reduction is budgeted and the report says `converged` or `budget exhausted` --
never silently the former when it was the latter.

### Coverage, stated honestly

**Wider suite.** `gate_core.json` pins 433 assertions as `fail`/`xpass`. The
reducer was pointed at all of them except the five pty-driven files, whose
probes cost seconds rather than milliseconds:

| | |
|---|---|
| pinned `fail`/`xpass` | 433 |
| less the five pty-driven files (`X02zlevi`, `X04zlehighlight`, `X06termquery`, `W02jobs`, `V08zpty`) | −83 |
| **attempted** | **350** |
| reduced to a diverging standalone script | 323 (92%) |
| ...of those, converged (1-minimal within a 60-probe budget) | 311 (89% of attempted) |
| ...budget exhausted, result not 1-minimal | 12 |
| not reproducible standalone | 27 |

51 of the 433 are in files the zsh baseline skips entirely (`V07pcre`,
`V10private`, `V11db_gdbm` — modules this zsh build was configured without and
zshrs has natively), so they are reduced and clustered here but are not part of
the 2381-relative score.

"Not reproducible standalone" is a result, not a gap: those assertions fail in
the suite but agree when replayed outside `ztst.zsh`, which places the
divergence in the harness path or in state the driver does not recreate. They
concentrate in `V01zmodload` (9 -- the module symlink farm this runner stages
rather than `make install.modules`) and `C03traps` (8 -- trap and subshell
interaction with the driver); the other 10 are spread over six files.

The excluded pty files were not left unexplained: `X04zlehighlight` (20 of the
83) and `X02zlevi` (one representative of 58) were reduced individually with a
small budget, and `X04zlehighlight`'s single root cause is above. `W02jobs` and
`X06termquery` are hangs, handled as such.

The reductions were taken against @06:25; see "Which binary" for why that is
still valid for 428 of the 431 core assertions.

**Y series.** 88 failing assertions, sampled rather than reduced wholesale --
a pty probe costs seconds and Y03arguments's probes cost the full timeout.
The sampling rule: cluster first, then reduce one representative of every
cluster with two or more members, plus every singleton. 11 assertions, all
converged. That gives a repro for each *distinct* shape rather than 88 repros
of the same three bugs. Coverage: 11 of 88 reduced (13%), but the 9 clusters
they represent cover all 88.

### The repros

Round 6's five Y reductions are in `minimization.txt` (against @06:25; the
`h:desc` output it records for `y05describe_001.zsh` and `y06values_001.zsh` is
from before the `argv+=` fix — both scripts still diverge on @29ee728e, on their
missing `NO:` lines). This round's eleven, one per Y cluster plus every
singleton, are in `minimization_round7.txt` (@29ee728e), plus two more for the
`insert_positions` family once the stale one was found. All thirteen converged.

`repros/y02compmatch_049.zsh` has been **deleted**: `Y02compmatch#49` now passes
(`INSERT_POSITIONS:{4:5:6}` is correct on @29ee728e) and the script is
byte-identical between the shells, so keeping it would be exactly the false
claim this directory exists to prevent. Five Y assertions moved fail→pass
between @06:25 and @29ee728e: `Y02compmatch#49`, `#53`, `#54`, `#56` and
`Y06values#14`. `insert_positions` still diverges on seven others
(`Y02compmatch#10`, `#11`, `#12`, `#27`, `#40`, `#52`, `#57`).

| repro | probes | what it shows |
|---|---|---|
| `y01completion_001.zsh` | 24 | `mkdir dir2; touch file2; comptest $': \t\t'` -- no `DI:`/`FI:` lines, and the buffer never advances to `: dir2/` |
| `y01completion_006.zsh` | 25 | `_expand`: no `DI:`/`FI:`/`NO:` lines, and `*` expands to `dir2` not `file2 ` |
| `y01completion_007.zsh` | 31 | `typeset -g tst=(*)` then `$tst^D`: `DESCRIPTION:{expansions}` arrives, `NO:{file2}` does not |
| `y01completion_009.zsh` | 24 | `comptest $'[\t\t'`: three descriptions, zero matches, buffer stuck at `[` |
| `y01completion_011.zsh` | 30 | same, before a `;:` separator |
| `y01completion_016.zsh` | 8 | completion stops at `cd "A(B)/` instead of `cd "A(B)/C/` |
| `y01completion_023.zsh` | 59 | sorting ignores backslashes: seven `FI:` lines missing |
| `y01completion_028.zsh` | 35 | `_arguments ':desc:_sequence compadd - 1 2 3'`: no description and no matches |
| `y02compmatch_002.zsh` | 25 | `compadd -M m:` says `missing word pattern`; zsh says `missing patterns` |
| `y02compmatch_010.zsh` | 26 | `compadd -M 'm:{0-9}={'`: zsh rejects it (`unterminated character class`) and completes nothing; zshrs accepts it and inserts `IndianRed` |
| `y02compmatch_057.zsh` | 55 | `r:|.=**` matcher: `INSERT_POSITIONS:{5:14}` reported as `{6}`, and the buffer loses its prefix |
| `y03arguments_004.zsh` | 41 | 3 positional specs + `\t\t^W^W^D`: zsh lists `arg1`/`arg2`; zshrs **hangs** |
| `y04regexargs_003.zsh` | 39 | `_regex_arguments` optional field after a suffix: `DESCRIPTION:{version}` missing |
| `y05describe_001.zsh` | 25 | `_describe`: matches missing from the listing |
| `y05describe_003.zsh` | 29 | `_describe` with two `(...)` groups: no matches, buffer never reaches `tst bx` |
| `y06values_001.zsh` | 27 | `_values`: matches missing from the listing |

`y03arguments_004.zsh` reproduces a hang, so wrap the second command in
`timeout 30` -- zsh exits 0 with four lines, zshrs never returns.

Known limitations, both measured:

* Removal is one element at a time, so a group of context statements that only
  makes sense together cannot be dropped, and a statement can be removed from
  the middle of a compound command. `y01completion_023.zsh` keeps a
  syntactically broken `if` whose `else` branch is the part that matters, and
  Y02compmatch reductions keep the earlier assertions that set the variables
  the later ones reuse.
* A loaded host can make a probe time out when nothing is hanging. If that
  happens on the *first* probe it fixes the anchor on `<<TIMED-OUT>>` and the
  whole reduction chases a hang that does not exist -- measured on
  Y02compmatch#2, which anchored on a 12 s timeout and finishes in both shells
  given 90 s. The reducer now retries a one-sided timeout on the anchoring
  probe, the same way it already retried a one-sided timeout of the reference.
  Re-run with the fix, Y02compmatch#2 anchors on `COMPADD` and reduces to
  `test_code m:` plus one TAB.

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

@06:25, of the 2381 assertions zsh passes, zshrs passed 1816 (76.3%), failed
376, and never reached 189 because the file hung or aborted first. It also
passes 36 assertions the baseline could not attempt (`zsh/pcre` 11,
`zsh/param/private` 23, `zsh/db/gdbm` 2 — modules zshrs has natively and this
zsh build does not).

@29ee728e three of those 376 pass (`A01grammar#29`, `E01options#18`,
`V04features#21`, each present in `core_zshrs_0.12.48_fxoff.failures.txt` and
absent from `core_zshrs_0.12.49_29ee728e_fxoff.failures.txt`) and nothing
regressed, so the score is **1819 pass, 373 fail, 189 unreached**. Raw run
counts, both binaries, all 2523 assertions:

    @06:25      fail=431 notrun=5 pass=1852 skip=2 unknown=212 xfail=19 xpass=2
    @29ee728e   fail=428 notrun=5 pass=1855 skip=2 unknown=212 xfail=19 xpass=2

Per-file numbers are in `core_scoreboard.txt` (@06:25); per-assertion verdicts
in `core_comparison_baseline_vs_zshrs.txt` (@06:25); failure detail in
`core_zshrs_0.12.49_29ee728e_fxoff.failures.txt`.

Four files hang rather than fail — D04parameter, D07multibyte, W02jobs,
X06termquery all hit the 150 s per-file timeout — and they swallow 152 of the
212 assertions the run never announces. They have their own section below; a
hang is not scored as a failure.

### Core failures and compsys: proven, refuted, unproven

Round 6 listed five core files whose failures "plausibly underlie" compsys
divergences and said so explicitly. Plausibility is not cause, so each was
tested two ways: does the core bug's signature actually appear in the compsys
failure, and does the compsys failure disappear when the construct is avoided.

**Proven.**

* `argv+=( ... )` -> every description in the completion system.
  `Completion/Base/Core/_description:83` builds its `zformat` spec list with
  `argv+=( h:$1 )`. Under @06:25 that replaced the positional parameters
  instead of appending, so `$1` itself became `h:desc` and line 89 formatted
  `d:h:desc`. Counterfactual: a copy of `_description` with line 83-86 changed
  to the equivalent `set -- "$@" h:...`, put first on `$fpath`, turned zshrs's
  `DESCRIPTION:{h:desc}` into `DESCRIPTION:{desc}` -- upstream's expected value.
  Independently confirmed by the @29ee728e fix. Note the same repro still fails
  after that: its `NO:` lines are the separate `lc=`/`rc=` bug, so one assertion
  carried two causes.
* `${~var}` in a nested pattern operand -> all 20 X04zlehighlight assertions,
  via that file's own `zpty_line` (`X04zlehighlight.ztst:44`). Reduction
  anchored on `zpty_line:19: bad pattern`, and the construct reproduces on its
  own -- see "Ranked by blast radius" below.
* `TYPESET_TO_UNSET` + `typeset -h +g -m '*'` -> all 10 E03posix assertions,
  by destroying `PATH` in the file's first chunk -- same section below.

**Refuted.**

* **V13zformat (21 of 35 failing) does not explain the description bug, or any
  other Y-series failure.** `Completion/` and `Functions/` use exactly three
  forms -- `zformat -f` (24 sites), `-a` (15), `-F` (2) -- and all three are
  byte-identical between the shells, including `_description:89`'s own call:

      zformat -F format '<DESCRIPTION>%d</DESCRIPTION>' 'd:desc' 'h:desc'
      zsh    <DESCRIPTION>desc</DESCRIPTION>
      zshrs  <DESCRIPTION>desc</DESCRIPTION>

  Ten of the 21 failures are `-q`/`-Q`, which appear nowhere under
  `Completion/`; the rest are spec-error and usage-error handling, which
  compsys never triggers because its format strings come from zstyle and it
  always passes exactly one of `-f`/`-F`/`-a`.
* **V12zparseopts (12 of 32) does not explain any compsys failure.** compsys
  uses `-D -E -a -A -K` and, once, `-F`. Every one of those shapes is identical
  between the shells, including `_description:11`'s exact call and
  `Completion/compinit:83`'s `zparseopts -A _i_opth -D -F - C d:: D i u w`.
  The only divergence on that call site is the text of a diagnostic
  (`f: bad option: -X` vs `f:zparseopts: bad option: -X`, V12#8) on a usage
  error compsys never reaches; parsed values and exit status match. Four of the
  12 failures are `-M`/`-G`, which `Completion/` never uses. The `-n` and
  option-stacking failures (#29, #30) are real and do matter, but to
  `Functions/Misc/zgetopt` rather than to compsys — see the cluster section.
* **B02typeset's parameter-hiding failures are not the hiding `Test/comptest`
  relies on.** #82-#87 are all about `zsh/random`'s autoloaded `SRANDOM`; they
  fail for one reason, `zmodload -u zsh/random` -> `no such module`, plus
  `zmodload -ap` listing nothing. The construct comptest actually uses in the
  widget every Y assertion goes through is identical in both shells:

      comppostfuncs=(global)
      f(){ local +h -a comppostfuncs=( inner ); print -r -- "in=<$comppostfuncs>" }
      f; print -r -- "out=<$comppostfuncs>"
      both:  in=<inner>  out=<global>

  B02typeset#66 is a genuine hiding bug (`typeset -p` drops the `-h`), but
  `typeset -p` appears 0 times under `Completion/`.

**Unproven.**

* **D06subscript (9 of 37).** Refuted for everything measured, but not
  exhaustively. The 9 failures are scalar pattern subscripts (#2, #4, #38),
  associative keys quoted or containing `*` (#11, #13, #17-#19) and a
  command-substitution subscript (#33). `Completion/` has 673 pattern
  subscripts, and the sample tested -- `words[(I)-X]`, `words[(i)--]`,
  `words[(r)-[CP]]`, `opt_args[(I)a|b]`, `words[(R)b]` -- is identical in both
  shells; it has 0 double-quoted associative keys, and its five `[$(` hits are
  all `$((` arithmetic, not the #33 shape. Five samples out of 673 is not a
  proof, so this stays unproven rather than refuted.
* **X04zlehighlight and the ZLE ledger.** The `${~var}` cause is proven for the
  20 assertions; whether `region_highlight` itself also diverges is not
  measured, because the file never gets far enough to say.

## Clustering: which bug, not which file

`--cluster` groups failing assertions by a key computed from the data, never by
a hand-written "these look similar" rule. Two keys, because the two suites give
different evidence:

* a **reduced** assertion (from `--core-minimize`) is keyed on its *witness* --
  the one normalised line the reducer anchored on. Diagnostics are keyed on the
  first few words of the message with the `(eval):N:` prefix and the offending
  value stripped, so `illegal pid: a` and `illegal pid: b` are one bug; a
  status-only difference is keyed on the status pair; ordinary output is keyed
  on which side lost or gained lines, because the text itself is data and not a
  cause.
* an **un-reduced** assertion (from a `--json` run) is keyed on the shape of its
  expected-vs-actual diff: which kinds of line went missing and which appeared.
  Coarser, and reported as such.

    $P scripts/ztst_compsys.py --zsh-build $B --cluster tests/ztst_compsys/zshrs_0.12.49_29ee728e_fxoff.json
    $P scripts/ztst_compsys.py --zsh-build $B --cluster tests/ztst_compsys/core_minimization.json --cluster-min 2

### Y series: 88 failures, 9 clusters, one cause dominates

`cluster_Y.txt`: 9 clusters, 6 of them with two or more members. The top
three keys cover 76 of the 88, and every one of the nine is a missing- or
wrong-match-line shape.

The dominant one is measurable directly rather than by cluster key. `comptest`
turns a listed match into a `NO:{...}` / `DI:{...}` / `FI:{...}` line by
matching `<LC><(??)><RC>(*)<EC>` (`Test/comptest:159`), where the markers come
from the `list-colors` zstyle `comptestinit` sets (`Test/comptest:44`). So:

| | |
|---|---|
| failing assertions | 88 |
| ...whose expected output contains at least one colour-tagged match line | **73** |
| ...of those, how many produced any colour-tagged line under zshrs | **0** |
| ...whose diff is *only* those lines, i.e. this one fix would make them pass | **53** |

Spread across every file: Y01 22, Y02 17, Y03 2, Y04 4, Y05 6, Y06 22.

The mechanism, from a raw pty capture of the same completion in both shells:

    zsh    ^[[J<LC><NO><RC>alpha<EC>^M
    zshrs  ^[[J^[[<NO>malpha<EC>^M

`ec=` is honoured; `lc=`/`rc=` are not -- the `\e[` and `m` around the colour
are emitted literally instead (`src/ported/zle/complist.rs:738-743` documents
why). Upstream's assertions therefore see no match lines at all.

### Wider suite: the failures do NOT cluster tightly

`cluster_core.txt`, over the 350 reduced assertions: **103 clusters, 27 of them
with two or more members and 76 singletons.** The largest single key is a
*shape* -- "same number of lines, different values", 74 members across 24 files
-- not a cause. The honest reading is that the non-Y suite is a long tail of
individually distinct language gaps, which is why the per-assertion repro, not
the cluster, is the deliverable there. The clusters that do carry blast radius:

| n | files | key |
|---|---|---|
| 27 | V11db_gdbm | `ztie: error opening database file ... (GDBM support not compiled in)` |
| 20 | V12zparseopts, Z04zgetopt | `zparseopts: no default array defined: <its own option word>` |
| 19 | 11 files | one output line missing |
| 15 | 11 files | exit status 1 -> 0, output otherwise identical |
| 12 | 7 files | identical outputs standalone (harness-path failures) |
| 10 | E03posix | `command not found: rm` -- `PATH` destroyed by assertion #1 |
| 8 | B02typeset, K01nameref | `zmodload: no such module zsh/random` |
| 6 | V13zformat | `zformat: invalid argument: -Fq` |

### Ranked by blast radius

| assertions | suite | root cause | evidence |
|---|---|---|---|
| 73 (53 alone) | Y | `ZLS_COLORS` `lc=`/`rc=` ignored when listing matches | raw pty capture, below |
| 27 | core | no GDBM: `ztie` reports `GDBM support not compiled in` | `repros_core/v11db_gdbm_002.zsh` |
| 20 | core | `zparseopts` mis-parses its own option words | below |
| 20 | core | `${~var}` in a nested pattern operand marks the *outer* expansion for filename generation | below |
| 18 | core | (subset of the 20 above) `zgetopt` unusable because of it | `repros_core/z04zgetopt_001.zsh` |
| 10 | core | `TYPESET_TO_UNSET` + `typeset -h +g -m '*'` destroys `PATH` | below |
| 8 | core | no loadable `zsh/random`, `zmodload -ap` lists no autoloaded parameters | `repros_core/b02typeset_082.zsh` |
| 6 | core | `zformat -q`/`-Q` unsupported | `repros_core/v13zformat_029.zsh` |

The V11db_gdbm 27 sit *outside* the baseline-relative score: this zsh build has
`zsh/db/gdbm` disabled, so upstream skips the file and only zshrs runs it.
Counted here because the pin counts them, not counted against 2381.

**`zparseopts` mis-parses its own option words** -- two shapes, one message:

    f(){ local -a o; zparseopts -DF -   o:=o && print "o=<$o[*]>" }; f -o ab
    zsh    o=<-o ab>
    zshrs  zparseopts: no default array defined: -DF        # stacked -D -F

    f(){ local -a o; zparseopts -n nm - o:=o && print "o=<$o[*]>" }; f -o ab
    zsh    o=<-o ab>
    zshrs  zparseopts: no default array defined: -n         # -n before -a/-A

`-D -F` written separately works, and `-n` written *after* `-a`/`-A` works, so
the option word is being reclassified as a spec rather than rejected outright.
`Functions/Misc/zgetopt:22` opens with `zparseopts -n $errname -D -F -G -`, so
`zgetopt` cannot run at all -- that is all 18 Z04zgetopt assertions, the whole
file, from this one gap.

**`${~var}`** -- found by reducing X04zlehighlight#2:

    cm=X; v="a|b"; print -r -- ${v%%${~cm}*}
    zsh    a|b
    zshrs  no matches found: a|b

The `~` is meant to make the *pattern* operand a pattern; zshrs lets it escape
onto the result, which is then globbed. Same for `##`, `:#` and `//`; does not
need `extendedglob`; suppressed by quoting the whole expansion. It is what
breaks all 20 X04zlehighlight assertions, through that file's own `zpty_line`
helper (`X04zlehighlight.ztst:44`). 28 sites under `Completion/` and
`Functions/` use the construct.

**`TYPESET_TO_UNSET`** -- E03posix's first chunk poisons the whole file:

    setopt TYPESET_TO_UNSET
    fn() { typeset -h +g -m '*' }
    fn
    print "PATH=<$PATH>"
    zsh    PATH=</bin:/usr/bin>
    zshrs  PATH=<>

`-m` (pattern) form only; the named form is fine, and without
`TYPESET_TO_UNSET` both shells agree. Because it runs in E03posix#1 and the
suite keeps one shell for the whole file, every later assertion fails with
`command not found` for any external command. That is the highest-severity
shape in the corpus: one assertion corrupting the rest.

## Hangs: their own category, never scored as failures

Four files stop rather than fail. A hang hides every assertion after it, so it
is worth more than any single failure: the four together swallow 152 of the 212
assertions the run never announces, plus the four hanging assertions
themselves. (Of the remaining 60, 32 are in files whose own `%prep` skips them
on this host -- `D03procsubst` 20, `P01privileged` 8, `V15nearcolor` 4 -- and 28
are behind `D10nofork`, which aborts rather than hangs.)

The hang point is read straight out of the pin: `parse_run` marks an assertion
`notrun` once ztst has printed `Running test:` for it, and `unknown` if it was
never announced at all, so the single `notrun` in a timed-out file *is* the
assertion that hung.

| file | hangs at | assertions lost | kind | minimal input |
|---|---|---|---|---|
| D04parameter | #135 `zsh_eval_context resizing` | 118 | superlinear parse, not a loop | `repros_hang/d04parameter_135_nested_anon_fn.zsh` |
| D07multibyte | #47 `Raw bytes don't match multibyte characters part 2` | 9 | infinite loop, CPU-bound | `repros_hang/d07multibyte_047_closure_over_multibyte.zsh` |
| W02jobs | #11 `various kill signals with multiple running jobs` | 15 | blocked read in the harness | `repros_hang/w02jobs_011_no_kill_notification.zsh` |
| X06termquery | #1 `foot response to terminal queries` | 10 | blocked read in the harness | `repros_hang/x06termquery_001_no_query_burst.zsh` |

None of the four is a deadlock, and none is in the harness itself -- in the two
`zpty` cases the harness blocks, but it blocks because the shell under test
never writes what upstream waits for.

**D04parameter#135 -- exponential parse of nested anonymous functions.**
Upstream builds `() { () { ... } }` 49 deep. `zshrs -n` (parse, do not execute)
blows up too, so the cost is in parse/compile:

| depth | zsh | zshrs `-c` | zshrs `-n` |
|---|---|---|---|
| 24 | 0.008 s | 0.396 s | 0.150 s |
| 28 | 0.008 s | 1.164 s | 0.415 s |
| 32 | 0.017 s | 3.509 s | 1.433 s |
| 36 | 0.008 s | 13.182 s | 3.722 s |

About 3x per added level, from a 253-byte program. Braces, subshells and nested
`eval` at the same depth are flat in both shells; only `() { ... }` is
superlinear.

**D07multibyte#47 -- infinite loop in the pattern matcher.** `[[ éé = é# ]]`
never returns (4.95 s of 5.00 s wall was user time). Locale-independent, does
not need the raw `\xa9` byte upstream uses, and specific to the bare literal:
`(é)#` returns immediately and ASCII `a#` is fine.

**W02jobs#11 -- no job-termination notification.** An interactive zshrs in a
pty prints `[1] <pid>` when the job starts and nothing when it is killed, so
`zpty -r zsh REPLY` never returns. zsh prints
`[1]  + terminated  sleep 30`. `zsh/zpty` is loaded by the harness, never by
the shell under test, so nothing about zshrs's own zpty is involved.

**X06termquery#1 -- no terminal-capability query burst.** The very first thing
the file's `termresp()` does is `zpty -r zsh REPLY $'\e*\r'`. zsh emits
`^[]11;? ^[]10;? ^[]12;? ^[P+q524742 ^[[>0q ^[[c` and a CR on startup; zshrs
emits none of it, with or without `ZSHRS_NATIVE_ZLE_FX=0`, so the read blocks
and all 11 assertions are lost.

**Not one of the four, but the same shape:** D10nofork stops after #35 with 28
assertions unreached and no timeout. #35 reduced standalone does *not* diverge
(`${| ... return 7 ... }` prints `INNER OUTER 7` in both shells), so that one is
in the harness path, not in the construct.

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

The 88 failing assertions on @29ee728e, in blast-radius order. Full detail and
expected-vs-actual diffs are in
`zshrs_0.12.49_29ee728e_fxoff.failures.txt`; the grouping is
`cluster_Y.txt`.

1. **`ZLS_COLORS` `lc=`/`rc=` are ignored when listing matches.** 73 of the 88,
   in every one of the six files; 53 of them would pass on this fix alone.
   `ec=` is honoured and the `\e[`/`m` around the colour are hardcoded
   (`src/ported/zle/complist.rs:738-743` documents why). Because upstream keys
   every listed match on `<LC><xx><RC>...<EC>`, no match line survives into the
   captured output at all. Measured directly, same completion, both shells:
   zsh `^[[J<LC><NO><RC>alpha<EC>`, zshrs `^[[J^[[<NO>malpha<EC>`.
2. **The buffer does not advance on repeated TAB.** 23 assertions have a
   `line:` divergence, and the shape is the same: zsh inserts the next match
   (`line: {: dir1/}{}`) where zshrs re-lists the same buffer
   (`line: {: }{}` plus another `DESCRIPTION`).
3. **`compadd -M` validates match specs differently.** 11 assertions,
   Y02compmatch #2-#8 and #10-#13, in two shapes. Wrong message: zsh's
   `missing patterns` comes back as `missing word pattern` (#2, #3),
   `missing right anchor` (#4, #5, #13) or `missing line pattern` (#6, #7), and
   its `unterminated character class` as `missing word pattern` (#8). No
   message at all: for `m:{0-9}={` zsh reports `unterminated character class`
   and completes nothing, zshrs accepts the spec silently and inserts a match
   (#10, #11, #12). Repros: `repros/y02compmatch_002.zsh`,
   `repros/y02compmatch_010.zsh`.
4. **`$compstate[insert_positions]` is wrong.** 7 assertions in Y02compmatch
   carry an `INSERT_POSITIONS` divergence, but they are not all one bug:
   #10, #11 and #12 report `{13}` where zsh reports `{}` only because zshrs
   accepted a malformed `-M` spec that zsh rejected, so they belong to (3).
   The rest are genuine: `{5:14}` reported as `{6}` (#57), `{5:7}` as `{9}`
   (#40), `{22}` as `{}` (#52), `{5:7}` as `{5:6}` (#27). Repro:
   `repros/y02compmatch_057.zsh`.
5. **`_arguments` does not advance past the first positional spec**, and hangs
   when the test then backs the buffer out. Repro:
   `repros/y03arguments_004.zsh`.
6. **Path completion through a `..` component stops at the `../`.** Repro:
   `repros/y01completion_016.zsh`.
7. **`_describe` with the unsupported `((...))` form** loses a space
   (Y05describe #2, cosmetic).

Fixed since round 6, kept here because the ledger recorded it as live:
**`argv+=( ... )` did not append to the positional parameters**, which put `h:`
in front of every description built by `_description`. Fixed between @06:25 and
@29ee728e; the counterfactual that proved it was the cause is under
"Core failures and compsys" above.

### Not a divergence

* **`zsh/zpty` in zshrs.** The driver's zpty is loaded by the harness shell, not
  by the shell under test, so this suite exercises none of it.
* **OSC 133 shell-integration escapes.** zshrs emits `\e]133;...`; so does
  upstream (`Src/Zle/termquery.c:688,754-757,781-782`).

## Files

| file | what it is |
|---|---|
| `baseline_zsh_5.9.999.3-test.{txt,json}` | the zsh Y-series baseline, per assertion |
| `zshrs_0.12.49_29ee728e_fxoff.{txt,json,failures.txt}` | zshrs Y series on the current binary, and every failure's diff |
| `cluster_Y.txt` | the 88 Y failures grouped by root-cause key |
| `zshrs_0.12.49_fxoff.{txt,failures.txt}` | the @06:25 run, kept for provenance |
| `zshrs_0.12.46_fxoff.{txt,json,failures.txt}` | round 5's run, kept for provenance |
| `zshrs_0.12.46_fxon_Y01_Y05_Y06.txt` | round 5's `--fx on` control |
| `comparison_baseline_vs_zshrs.txt` | per-assertion baseline-vs-candidate verdicts |
| `gate.json` | the Y-series pin: per-assertion status + binary identity |
| `gate_0.12.46_to_0.12.48.txt` | the gate run proving round 5's pin is not stale |
| `minimization.txt` | Y assertion → minimal repro, with before/after and budgets |
| `repros/*.zsh` | the generated standalone Y repros |
| `repros_hang/*.zsh` | one minimal hanging input per hanging file |
| `core_baseline_zsh_5.9.999.3-test.txt` | zsh on the 70 non-Y files |
| `core_zshrs_0.12.49_29ee728e_fxoff.{txt,failures.txt}` | zshrs on the same, current binary |
| `core_zshrs_0.12.48_fxoff.{txt,failures.txt}` | the @06:25 run, kept for provenance |
| `core_comparison_baseline_vs_zshrs.txt` | per-assertion verdicts, core suite |
| `core_scoreboard.txt` | per-file: zsh-passing, zshrs ok / FAIL / not-run |
| `gate_core.json` | the core pin |
| `core_minimization.{txt,json}` | core assertion → minimal repro, one entry per attempted assertion |
| `cluster_core.txt` | the reduced core assertions grouped by witness |
| `repros_core/*.zsh` | the generated core repros for the named root causes |

The reducer emits one `.zsh` per assertion; only the ones named in this file are
committed, because the preamble is ~2 KB and identical in all of them and the
rest regenerate in one command:

    $P scripts/ztst_compsys.py --sut target/debug/zshrs --zsh-build $B \
        --core-minimize-from tests/ztst_compsys/gate_core.json \
        --repro-dir /tmp/repros_core --out /tmp/core_minimization.txt
