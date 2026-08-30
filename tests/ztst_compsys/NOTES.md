# Upstream completion suite (Test/Y0*.ztst) run against zshrs

This directory pins one full run of **zsh's own** completion test suite against
both zsh and zshrs, so a later run can tell "this regressed" from "this was
always failing". The runner is `scripts/ztst_compsys.py`; the numbers below are
derived from the pinned `.json` files in this directory, not typed by hand.

Recompute any of them with:

    python3 scripts/ztst_compsys.py --sut <shell> --zsh-build <built zsh tree> \
        --compare-to tests/ztst_compsys/baseline_zsh_5.9.999.3-test.json

## Why this suite

Every other compsys harness here (`compsys_parity.py`, `comptab_parity.py`,
`compsys_spec_fuzz.py`) compares against cases we invented. zsh ships six
completion test files with its own expected output, and its driver
(`Test/comptest`) takes `comptestinit -z <shell>` to choose which shell runs
under the pty. Pointing that at zshrs makes upstream the oracle: a failure here
is a compatibility gap stated in upstream's own terms.

## Setup used for the pinned run

| | |
|---|---|
| oracle tree | `~/forkedRepos/zsh` @ `599af4604f`, copied to scratch and built (`Util/preconfig && configure && make`) |
| oracle version | `5.9.999.3-test` |
| baseline shell | that tree's `Src/zsh` (also the harness that interprets `ztst.zsh`) |
| candidate shell | `target/debug/zshrs`, `zshrs 0.12.46`, reports `ZSH_VERSION=5.9.2` / `zsh-5.9.2-0-gddee3e7` |
| test files | Y01completion Y02compmatch Y03arguments Y04regexargs Y05describe Y06values |
| date | 2026-08-30 |

The oracle tree must be *built*, and its `Completion/` must be the same version
as its `Test/`: a 5.9.999-era `Completion/compinit` uses `${ ... }` nofork
command substitution, which zsh 5.9.2 rejects with `bad substitution`, killing
`compdef` and hanging the suite. Homebrew zsh 5.9.2 is therefore **not** usable
as the baseline for these test files. zshrs does support `${ ... }`.

`zsh/zpty` is loaded by the *harness*, never by the shell under test, so zshrs
needing no zpty of its own is not a blocker for this suite.

With no `--zsh-build` the runner falls back to the gitignored in-tree
`src/zsh` (built, `5.9.0.3-test`), which needs no setup but only carries
Y01-Y03. Its Y01 baseline is also clean (`pass=33`).

## Files

| file | what it is |
|---|---|
| `baseline_zsh_5.9.999.3-test.{txt,json}` | the zsh baseline, per assertion |
| `zshrs_0.12.46_fxoff.{txt,json}` | zshrs with `ZSHRS_NATIVE_ZLE_FX=0` |
| `zshrs_0.12.46_fxoff.failures.txt` | expected-vs-actual diff for every failure |
| `comparison_baseline_vs_zshrs.txt` | per-assertion baseline-vs-candidate verdicts |
| `zshrs_0.12.46_fxon_Y01_Y05_Y06.txt` | zshrs as shipped (native ZLE effects on) |

## Baseline (the oracle is clean on this host)

    files=6 assertions=229 pass=222 xfail=7

Every file exits 0. The 7 non-passes are upstream's own `f`-flagged
expected-to-fail assertions in Y02compmatch (`r:[^.]||.=*` and friends), which
fail identically for zshrs, so they are excluded from every zshrs score below.
**There is no host-specific baseline failure to subtract.**

## zshrs (`--fx off`)

    files=6 assertions=229 pass=32 fail=93 xfail=7 notrun=1 unknown=96

Per assertion against the baseline: `both-pass=32  REGRESSION=190  both-nonpass=7`.

| file | baseline | zshrs |
|---|---|---|
| Y01completion | pass 33 | pass 10, fail 23 |
| Y02compmatch | pass 51, xfail 7 | pass 18, fail 33, xfail 7 |
| Y03arguments | pass 100 | pass 1, fail 2, then **timed out** (96 unrun) |
| Y04regexargs | pass 6 | pass 1, fail 5 |
| Y05describe | pass 7 | fail 7 |
| Y06values | pass 25 | pass 2, fail 23 |

Y03arguments does not fail, it **hangs**: at assertion 4 the shell under test
exits and the driver blocks forever waiting for its finish widget, so the file
hit the 900 s per-file timeout with 96 assertions never run. See divergence 3.

zshrs is also 10-20x slower through this suite than zsh (Y01: 25.6 s vs 1.4 s).

## Adaptations (declared)

* `--fx off` exports `ZSHRS_NATIVE_ZLE_FX=0`, disabling zshrs's own autosuggest
  and syntax-highlight overlays, which are on by default even under `-f` and
  paint escape sequences and phantom text into the captured pty stream.
  Measured, not assumed: an `--fx on` run of Y01/Y05/Y06 produced **identical**
  per-file counts (`pass 10 fail 23`, `fail 7`, `pass 2 fail 23`), so this knob
  did not move the score for the pinned run.
* The shell under test is reached through a symlink, never a wrapper script:
  `/bin/sh` drops the exported `PS1`, and `comptest` keys every read on
  `<PROMPT>`, so a wrapper hangs the suite outright.
* No `.ztst` file and no driver file was modified.

## Attributed divergences

Clustering the 93 failing assertions by what the diff shows: 75 are fully
explained by divergences 1 and/or 2 (48 by both, 25 by 2 alone, 2 by 1 alone);
18 are something else and are itemised below.

### 1. `argv+=( ... )` does not append to the positional parameters, and leaks

*compsys divergence, shell-level.* This one bug reprices every description in
the completion system, because `_description` (`Completion/Base/Core/_description:83`)
builds its `zformat` spec list with `argv+=( h:... )`.

    f() { argv+=( X ); print -r -- "n=$# argv=<${(j:,:)argv}>" }
    f 1 2 3; f 1 2 3; f 1 2 3

    zsh    n=4 argv=<1,2,3,X>   (three times)
    zshrs  n=1 argv=<X>
           n=2 argv=<X,X>
           n=3 argv=<X,X,X>

zshrs replaces `$@` with only the appended elements, and the array persists
across calls instead of being per-frame. `argv=( $argv X )` behaves correctly,
so the bug is specific to the `+=` append form on `argv`.

Effect in the suite: `_description globbed-files expl file` hands compadd
`-X '<DESCRIPTION>h:file</DESCRIPTION>'` instead of `<DESCRIPTION>file</DESCRIPTION>`,
because `$1` at `_description:89` has become `h:file`. Every `DESCRIPTION:{...}`
assertion in Y01/Y03/Y05/Y06 sees the `h:` prefix.

### 2. `ZLS_COLORS` `lc=` / `rc=` are ignored when listing matches

*compsys divergence, already flagged in our own source.* With
`zmodload zsh/complist` and `lc`/`rc`/`ec` set (which is exactly what
`Test/comptest` configures), listing a match emits:

    zsh    <LC><NO><RC>aaa<EC>
    zshrs  \e[<NO>maaa<EC>

`ec=` is honoured; `lc=` and `rc=` are not — the hardcoded `\e[` and `m` are
used instead. `src/ported/zle/complist.rs:738-743` documents this:
"COL_LC / COL_RC are hardcoded to their defaults ... A config that overrides
`lc=`/`rc=` is therefore not honoured here yet", because every caller already
holds the `MCOLORS` lock and `std::sync::Mutex` is not reentrant.

Because upstream's assertions match on `<LC><xx><RC>...<EC>`, every listed
match silently disappears from the captured output — the single largest
contributor to the failure count.

Repro (needs a pty and `zsh/complist`):

    ZLS_COLORS='lc=<LC>:rc=<RC>:ec=<EC>:no=<NO>'
    _tst() { compadd -J g -X '<HDR>' aaa bbb }

### 3. `_arguments` does not advance past the first positional spec

*compsys divergence.* Cause of the Y03arguments hang.

    _tst() { _arguments ':d1:(arg1)' ':d2:(arg2)' ':d3:(arg3)' }
    # then: tst <TAB><TAB><TAB>

    zsh    tst arg1 / tst arg1 arg2 / tst arg1 arg2 arg3
    zshrs  tst arg1 / tst arg1      / tst arg1

A single positional spec works (Y03 #1 passes). With two or more, the second
and later specs never complete.

The hang is downstream of this: Y03 #4 sends `<TAB><TAB><TAB>` then three
`^W`s and a `^D`. zsh's buffer is `tst arg1 arg2 arg3 `, so three `^W`s leave
`tst ` and `^D` runs the rebound `list-choices` widget. zshrs's buffer is only
`tst arg1 `, so three `^W`s empty it and `^D` on an empty line exits the shell.
`^D`-on-empty-line was checked separately under the same harness and is
**identical** in both shells (`\e[?2004l\r\r`), so it is not itself a
divergence — only the buffer state that reaches it is.

### 4. `$compstate[insert_positions]` reports only the last position

*compsys divergence.* Y02compmatch #49, #53, #54, #56.

    zsh    INSERT_POSITIONS:{4:5:6}   {9:27}   {8:10}
    zshrs  INSERT_POSITIONS:{6}       {27}     {10}

The colon-joined list of ambiguous insertion points is collapsed to its final
element.

### 5. `compadd -M` reports one generic message for every malformed spec

*compsys divergence.* Y02compmatch #2-#8, #10-#13.

| spec | zsh | zshrs |
|---|---|---|
| `m:` `M:` `r:` `R:` `l:` `L:` | `compadd: missing patterns` | `compadd: missing word pattern` |
| `m:{0-9` | `compadd: unterminated character class` | `compadd: missing word pattern` |
| `z:` | `compadd: unknown match specification character `z'` | same (passes) |

The unknown-character branch is right; the later parse-error branches all fall
through to `missing word pattern`.

### 6. Path completion through a `..` component

*compsys divergence.* Two assertions, same shape: zshrs stops at the `../`.

Y01completion #16 ("directory name is not a glob qualifier"): after
`cd "A(B)/` zsh completes to `cd "A(B)/C/` and then `cd ../C/`; zshrs leaves
`cd "A(B)/` unchanged and produces `cd ../`.

Y02compmatch #52 ("Second test from workers 12995"), with a matcher spec and
the word `../com/cor`: zsh completes the whole thing to
`tst ../Completion/Core `; zshrs inserts nothing, leaving
`line: {tst ../}{com/cor}` and an empty `INSERT_POSITIONS`.

### 7. `_describe` with the unsupported `((...))` form

*compsys divergence, cosmetic.* Y05describe #2:
`_describe desc '(( a b:descb "c\:c:descc" ))'` then `<TAB>`:

    zsh    line: {tst  }{}    (two spaces)
    zshrs  line: {tst }{}

### Not a divergence

* **`zsh/zpty` in zshrs.** The driver's zpty is loaded by the harness shell, not
  by the shell under test, so this suite does not exercise it and says nothing
  about whether zshrs implements it.
* **OSC 133 shell-integration escapes.** zshrs emits `\e]133;...` sequences;
  so does upstream (`Src/Zle/termquery.c:688,754-757,781-782`). Not a
  divergence, and the driver tolerates them.
* **Harness/driver incompatibility.** None found. Once the shell under test is
  reached by symlink rather than by a `/bin/sh` wrapper, every file runs to
  completion or hangs for a reason attributable to the shell.
