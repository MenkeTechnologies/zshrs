# parity_corpus_fuzz — the completion fuzzer's persistent corpus

Inputs for `scripts/comptab_parity.py --mutate N`, and the place the fuzz modes
(`--mutate`, `--style-fuzz`, `--guided`) write what they find. One small JSON
file per input; an input is a complete cell:

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

## Regression guards (`reg_*.json`, `reg_*_init.zsh`)

The `fp_*` and `fn_*` entries are things that were BROKEN. The `reg_*` entries
are things that were FIXED, kept as the cell that would notice them coming
back. Same idea as flipping a `tests/compsys_fixtures` entry to `agrees`,
applied to the axis this harness actually drives.

Each one names the commit it guards and quotes the mechanism from that commit's
own message, so a reader can tell what the cell is FOR without a git archaeology
session. `origin` is `regression/2026-08-30/<commit>`, which means
`corpus_weight` gives it 1 — the same as a hand-written case, and deliberately
not the 12x a mined reproducer gets. A guard is known-good territory; it is not
where the next bug is.

FIVE of them are marked `DIVERGES TODAY` in their own `note` or comment header.
Those are not guards, they are open divergences this round found while building
the guards, and the file says so rather than letting a reader assume every
`reg_*` passes.

Verified not vacuous, which matters more for a guard than for a finding: a
guard that would pass even with the bug back is worse than no guard.
`reg_complist_lc_rc_markers` renders `<LC>1ls /usr/share/zsh/5.9/` / `>/<EC>`
on both shells, so the `lc=`/`rc=` config is demonstrably live in the cell;
`reg_printfmt_percent_escapes_init.zsh` renders all eight explanation strings.
Both PASS.

| entry | guards | shape |
| --- | --- | --- |
| `reg_argv_append_pparams` | `d555917f07` | `argv+=` must append to pparams and not leak across calls — `_description:83` builds its `zformat` spec list with it |
| `reg_hashed_local_shadow_commands` | `40982fc067` + `d555917f07` | `local -A commands` is served from its own table (`_command_names:70`) |
| `reg_hashed_nonhash_shadow_hides` | `40982fc067` control | `local -a options` still HIDES the magic row |
| `reg_tilde_pattern_operand_no_glob` | `d30f91f8ed` | `${v%%${~cm}*}` must not glob the enclosing expansion |
| `reg_tilde_pattern_operand_still_applies` | `d30f91f8ed` control | the nested `~` must still make the pattern a pattern |
| `reg_chained_subscript_search_bound` | `d7058390e7` 1+2 | `${a[1,(r)d][(I)C]}` returns the element, `${a[1,4][(I)C]}` the index |
| `reg_chained_subscript_comma_token` | `d7058390e7` 3 | the `Comma` token inside a nested expansion |
| `reg_ssub_split_and_quote` | `d7058390e7` 4 | `PREFORK_SINGLE` reaching paramsubst for a `${(flags)NAME[SUB]}` RHS |
| `reg_zparseopts_long_spec_unguarded` | `93de3309af` | `-move=opt_move` unguarded — verbatim `zinit-install.zsh:1528` |
| `reg_magic_equal_quoted_not_expanded` | `c7cd9ee0f1` | a QUOTED `=`/`~` is not a lexer token, so `--a='b:=c'` survives |
| `reg_magic_equal_unquoted_still_expands` | `c7cd9ee0f1` control | `P=/bin:=ls` must still expand |
| `reg_paramsubst_colon_not_pathlist` | `b26c3ccd61` | the colon of `${VAR:=default}` is not a path-list separator |
| `reg_transpose_words_eol` | `44a3b4841c` 4 | `M-t` at end of line — the panic AND the wrong transposition |
| `reg_complist_lc_rc_markers` | `6fa67cb221` | `lc=`/`rc=` honoured when listing matches — the single gap behind 73 of 88 failing Y assertions |
| `reg_nested_anon_fn_compile_depth40` | `b4ad35079c` 2 | 40 nested `() { }` compile in 0.06 s, not `(4/3)^N` |
| `reg_zparseopts_stacked_flags` | — | **DIVERGES TODAY** — see below |
| `reg_zparseopts_n_before_array` | — | **DIVERGES TODAY** — see below |
| `reg_typeset_to_unset_pattern_hide` | — | **DIVERGES TODAY** — see below |
| `reg_multibyte_closure_c_locale` | — | **DIVERGES TODAY** — see below |
| `reg_zle_pre_redraw_hook_init.zsh` | — | **DIVERGES TODAY** — `zle-line-pre-redraw` does not fire; see below |

### The completer-shaped guards need `--init-extra`

Three of today's fixes are in code no host completer reaches, so their guard is
a completer written for the occasion. Those cannot be corpus entries — a corpus
entry is a buffer, keys and zstyle statements, and none of those can define a
completion function. They ship as `reg_*_init.zsh` sidecars instead, in the
same spirit as the `fp_*_styles.zsh` sidecars, each carrying its own replay
command in a comment at the top:

```sh
scripts/comptab_parity.py --init-extra scripts/parity_corpus_fuzz/reg_printfmt_percent_escapes_init.zsh \
                          --case 'true ' --keys ctrl-d --compare-attrs --strict-stream
scripts/comptab_parity.py --init-extra scripts/parity_corpus_fuzz/reg_zle_pre_redraw_hook_init.zsh \
                          --case 'ls /usr' --keys tab
scripts/comptab_parity.py --init-extra scripts/parity_corpus_fuzz/reg_zle_line_init_control_init.zsh \
                          --case 'ls /usr' --keys tab -v
```

They cover `44a3b4841c` items 2 and 3 (printfmt's `%` escape switch, and the
erase-to-EOL every listing row is terminated with) and `b122d9cbe1`'s
`zle-line-pre-redraw` hook plus its control.

The printfmt one is a real guard, verified not vacuous — `-v` shows all eight
explanation strings rendered and byte-identical on both shells, including the
one that named the bug:

```
   0| @CT@ true          7| fg              12| h1  h2
   1| bold               8| f1  f2          13| Xzero
   2| b1  b2             9| bg              14| z1  z2
   3| under             10| k1  k2          15| 100% pct
   4| u1  u2            11| hi
```

`hi`, not `Hhih`; `100% pct`, not `100%% pct`. PASS in 4.7 s.

A fourth sidecar for `b122d9cbe1` item 1 (`zle -T tc`) was written and then
DELETED rather than shipped. A `tc` hook suppresses terminal capabilities by
design, so with zsh's own `tcfunc(){ REPLY="" }` the reference shell could not
draw its own prompt and the cell FAILED on `codelabs-arm% source <path>`
instead of the `@CT@` sentinel (3 rows, fingerprint `5af453e279`). Narrowing it
to a single capability (`[[ $1 == le ]]`) produced the same garbled reference
screen. That is a probe defect, and a file that always FAILs for the wrong
reason is worse than no file: someone would read it as a divergence. `zle -T
tc` is structurally out of reach for a grid-diff harness — it deliberately
breaks the terminal it would be measured on.

**The pre-redraw one is not a guard — it found the bug still there.** First run
of `--init-extra`, FAIL in 46.8 s, fingerprint `0602148f26`:

```
zsh   : @CT@ ls /usr [PRD]
zshrs : @CT@ ls /usr
```

with the widget being `_f() { POSTDISPLAY=' [PRD]' }; zle -N zle-line-pre-redraw _f`.
The control rules out the obvious alternative explanation: the same body bound
to `zle-line-init` renders ` [LI]` on BOTH shells (PASS, 23.3 s, `-v` shows
`@CT@ ls /usr [LI]` on each). So `POSTDISPLAY` works and the HOOK is what does
not fire. `redrawhook` (`zle_main.c:1066`) is where every zsh syntax
highlighter repaints `$region_highlight`.

### Which of the day's fixes are OUT OF REACH here, and why

Not a failure list — a blind-spot list. This harness compares two terminal
grids after a typed buffer and a key sequence, so anything whose only evidence
is not on a grid cannot be guarded from here.

| fix | why it is out of reach |
| --- | --- |
| `44a3b4841c` 1 — SIGWINCH redraw | needs the window to be RESIZED mid-cell. The pty is created once at `--rows`x`--cols` and never resized; `tests/compsys_fixtures/listing_lost_on_window_shrink` has its own `winch_probe` for exactly this reason |
| `44a3b4841c` 5 — `insert_positions`' second `cline_str` walk | its only observable is upstream's `INSERT_POSITIONS:{…}` line, which `Test/comptest` synthesises and no ordinary terminal ever shows |
| `188f88cd98` — a corrupt `plugins.db` heals once | needs a corrupt database file staged before boot. `--init-extra` runs after `compinit`, which is far too late |
| `8d3e39201e` — the six stock-utility fixes | reachable, but NOT from a corpus entry: they need the utility called directly. They are guarded by `--fn-sweep`, and the thirteen `fn/fn_*.json` findings ARE the guards — every one of them should now pass |
| `b122d9cbe1` 2 and 3 — one attribute-off cap, `applytextattributes` | the divergence is in WHICH escape bytes are emitted, and two different escape sequences can paint an identical grid. `--compare-attrs` narrows it, `--strict-stream` sees only diagnostics; the raw stream is diffed on FAIL but not asserted on |
| `2573335336` — `gethparam` held a read lock across the magic-hash scan | a deadlock, so the only expression available here is a TIMEOUT, which this harness (correctly) refuses to score as a divergence |
| `b122d9cbe1` 1 — `zle -T tc` | a `tc` hook suppresses terminal capabilities by design, so the reference shell cannot draw a comparable screen at all. Measured both ways — suppress-everything and suppress-one — and both garble the reference. Out of reach here by construction |
| `93de3309af`/`693889cb68` stacking | reachable, and it is one of the four `DIVERGES TODAY` rows — see below |

### The four open divergences these entries pin

**`zparseopts` stacking, against the 5.9.2 reference** (`reg_zparseopts_stacked_flags`,
`reg_zparseopts_n_before_array`). Measured non-pty, `LC_ALL=C`, zshrs 0.12.49
@10:38:

```
f(){ local -a o; zparseopts -DF - o:=o && print -r -- "o=<$o[*]>" }; f -o ab
  /opt/homebrew/bin/zsh 5.9.2  rc=1  f:zparseopts: no default array defined: -DF
  target/debug/zshrs           rc=0  o=<-o ab>

f(){ local -a o; zparseopts -n nm - o:=o && print -r -- "o=<$o[*]>" }; f -o ab
  /opt/homebrew/bin/zsh 5.9.2  rc=1  f:zparseopts: no default array defined: -n
  target/debug/zshrs           rc=0  o=<-o ab>
```

This is a REFERENCE-CHOICE divergence, and it is the clearest single example of
why two instruments disagree. `693889cb68` deliberately adopted upstream
`88d51a2400`'s standard option parsing (stacking, `-n NAME`), and `93de3309af`
then reverted only the LONG-SPEC half back to 5.9.2 because zshrs reports
`$ZSH_VERSION=5.9.2`. So zshrs is 5.9.999 on stacking and 5.9.2 on long specs.
`tests/ztst_compsys` compares against a built 5.9.999.3-test tree and sees
agreement; this harness compares against `zsh` on `PATH`, which is Homebrew
5.9.2, and sees a divergence. Neither instrument is wrong; they are answering
different questions, and only running both asks both.

**`TYPESET_TO_UNSET` + `typeset -h +g -m '*'` destroys `PATH`**
(`reg_typeset_to_unset_pattern_hide`). Named in `tests/ztst_compsys/NOTES.md`
as the highest-severity shape in the core corpus — it runs in `E03posix#1` and
every later assertion in that file then fails with `command not found`. Still
reproduces on 0.12.49 @10:38. Nothing in this corpus had ever typed it, which
is precisely why this harness never saw it.

**A literal multibyte character in a closure pattern, in the C locale**
(`reg_multibyte_closure_c_locale`) — NEW, found 2026-08-30 while writing the
guard for `b4ad35079c`. That commit fixed the HANG (`[[ éé = é# ]]` spun at
100% CPU in `patcomppiece`'s literal-run backtrack). The ANSWER is still wrong:

```
LC_ALL=C <shell> -f -c 'setopt extendedglob; g=é; [[ éé = ${g}# ]]; print $?'
  zsh 5.9.2   1
  zshrs       0
```

It is source-representation specific rather than a pattern-matcher bug: the
same bytes written `$'\xc3\xa9'` give 1 on BOTH shells, `${#s}` is 4 on both,
and pure-ASCII multi-character runs agree (`[[ abab = ab# ]]` is 1 on both).
Under `en_US.UTF-8` both answer 0. It matters to this harness specifically
because `child_env` pins `LC_ALL=C`, so every cell here runs in the one locale
where it is live.


## What is tracked in git, and what is not

* **Tracked: `fp_*.json` (+ its `*_styles.zsh` sidecar).** These are the
  accumulated findings. They cannot be regenerated from anything — they are
  what the fuzzer learned.
* **Tracked: `cov_*.json`.** Inputs `--guided` kept for INFORMATION — each one
  was the first input in the corpus's history to produce some feature. Capped
  at `--cov-corpus-max` (160); at the cap the least informative one is evicted,
  never an `fp_*`.
* **Tracked: `fn/fn_*.json`.** One per divergence `--fn-sweep` found calling a
  stock utility function directly. Kept in a SUBDIRECTORY because `--mutate`
  reads every `*.json` in this directory as a mutation parent, and a fn-sweep
  finding is not one: its identity is (function, call label) and its buffer and
  keys replay nothing without the probe init the sweep generates. Its `replay`
  field is the command that reproduces it.
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

Three run-wide flags apply to every mode and are new this round:

```sh
--dump FILE            # ALWAYS pass one on a shared machine; see --check-dump
--init-extra FILE      # zsh appended to the init both shells source
--silence-recheck 3    # separate a silent reference from a slow one
```

`--check-dump` is on by default and refuses a run whose dump registers no
completions. Build a private one first and every number becomes reproducible:

```sh
zsh -fc 'autoload -Uz compinit; compinit -u -d $TMPDIR/dump'
scripts/comptab_parity.py --dump $TMPDIR/dump ...
```

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

`--layout-fuzz` adds three more, for the same reason applied to the layer
underneath a config — where the completers physically live and how `compinit`
is told to find them:

* `INVALID-LAYOUT` — the REFERENCE zsh refused the layout: `compinit` aborted,
  or it registered no completions at all. Detected before any pty boots, by
  sourcing the same init file under `zsh -f -c`. The cell is not run. A
  corrupt `.zcompdump` under `-C` and a world-writable directory under the
  default (prompting) `compinit` both land here.
* `REF-WARNED` — zsh initialised but printed a diagnostic (an insecure
  directory, an `invalid zwc file`). The cell IS run and IS compared — zshrs
  has to complain identically — but it is tallied apart from the clean passes.
* `UNBUILDABLE` — this host cannot construct the layout. Only one condition is
  in that state today: a completer owned by ANOTHER user, which needs
  privileges this process does not have. It is named and counted rather than
  quietly replaced by a layout that can be built.

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


## The stock utility functions (`--fn-sweep`)

`--mutate` varies what is TYPED, `--style-fuzz` varies how the shell is
CONFIGURED, `--layout-fuzz` varies where completers are STORED. All three reach
a utility function like `_description` only by accident — something has to be
typed that happens to route through it. `--fn-sweep` calls the utility
DIRECTLY, inside a real completion context, with arguments derived from its own
documented interface, and compares what each shell observed.

That matters because zshrs replaces most of these functions with a native Rust
port that intercepts the name. `--fn-list` prints the map, derived from the
router's own dispatch table plus the arbitration it applies to the live
`$fpath`:

```
# router table  : 246 `_NAME` arms in src/compsys/router.rs
# stock tree    : 998 `_NAME` file(s) on this $fpath; 240 of them have a port, 758 have none and are SHELL-only
# backend split : 243 served by the NATIVE RUST PORT, 3 by the SHELL function
#   SHELL  _command_names     fpath override at position 17: ~/.zpwr/autoload/comp_utils
#   SHELL  _files             fpath override at position 17: ~/.zpwr/autoload/comp_utils
#   SHELL  _parameters        fpath override at position 17: ~/.zpwr/autoload/comp_utils
```

Method: the arms of `fn rust_compsys_lookup` (`src/compsys/router.rs:258`) are
the registry — `dispatch_compsys` consults nothing else. A registered name
still stands down when `$fpath`'s FIRST file for it sits in a non-stock
directory ahead of the shipped tree (`has_fpath_override`, router.rs:186-215).
A third gate, `has_shfunc_override` (router.rs:89), fires for a shfunc body the
user DEFINED; nothing in the harness's init defines one, so it is inert here
and is printed as a caveat rather than assumed away.

### How a call is observed

`compdef _zpf_probe true` makes a generated probe the completer for a real
command, so the utility runs where `compadd`, `comptags`, `$compstate` and
`$curcontext` are all live — none of these functions can be called from an
ordinary command line. The probe writes its observations to a file, and `^O` is
rebound to a widget that puts `cat -- $ZPF_OUT` on the line and accepts it, so
the report reaches the terminal as ordinary output and is compared by the same
grid diff as every other cell.

Two axes, selected by `--fn-keys`:

```sh
# every call, both shells, per-function verdict table
scripts/comptab_parity.py --fn-sweep

# the split map and the probe table; no shell is booted
scripts/comptab_parity.py --fn-list

# one call, which is what a failure prints as its replay
scripts/comptab_parity.py --fn-only _setup --fn-call one-arg

# the LISTING axis: no report, the two grids ARE the completion listings
scripts/comptab_parity.py --fn-sweep --fn-keys ctrl-d
```

The default keys compare the REPORT — return status, the arrays the function
filled, its `$compstate` delta, its stderr — because `^O`'s accept-line scrolls
the listing off the final grid. `--fn-keys ctrl-d` compares the listing itself.
The two answer different questions and disagree in practice: see below.

`--fn-sweep` adds one verdict of its own, for the same reason `--style-fuzz`
has `INVALID-CONFIG`:

* `INVALID-CALL` — the reference zsh could not even PARSE the generated call
  (`zsh -n`). A bug in the harness's argument generation, never a finding; the
  cell is not run. The check is parse-only on purpose: an executing check would
  report `can only be called from completion function` for nearly every probe
  and throw away real cells.

`REF-REFUSED` keeps its meaning, widened to runtime complaints about a CALL
(`bad substitution`, `invalid argument`, ...). Those cells are still run and
still compared — zshrs is required to complain identically — but they are
tallied apart so a green sweep cannot be assembled out of calls zsh itself
rejects.

A new fingerprint is written to `parity_corpus_fuzz/fn/fn_<hash>.json` with its
call, its two rows and its replay. It is a SUBDIRECTORY on purpose: `--mutate`
takes every `*.json` in the corpus root as a parent, and a fn-sweep finding is
not one — its buffer and keys replay nothing without the probe init.

### What the first full run found

99 calls across 24 functions: 62 PASS, 36 FAIL over 13 fingerprints, 1 TIMEOUT,
2 REF-REFUSED, 0 INVALID-CALL. Ten of the 24 functions diverge. The same six
match-adding functions re-run on the LISTING axis (28 calls) were 28/28
byte-identical — so every divergence below is in the STATE a utility leaves
behind, not yet in what it displays.

| fingerprint | cells | what differs |
| --- | ---: | --- |
| `3de4771a42` | 9 | `_alternative`, `_path_files`: the caller's `expl` comes back holding the callee's value (`'-J' '-default-'`). zsh leaves it untouched — both declare `local … expl` |
| `b3c97da2eb` + 3 siblings | 8 | `_description`: `_lastdescr` gains one element, zsh's gains two. `_main_complete:54` declares `typeset -U _lastdescr` with no `-a`, so it is a SCALAR when `_description:14` runs `_lastdescr=( "$_lastdescr[@]" "$3" )`, and `"$scalar[@]"` on an empty scalar is one empty word. The port appends to an array instead |
| `df47ae1cbe` | 5 | `_values`: the caller's `expl` comes back UNSET. `_values.rs:128-143` emulates zsh's `local` by `unsetparam` on return, which DELETES the caller's binding instead of restoring it |
| `5e5ca020b9` | 4 | `_setup` returns 0; zsh returns 1. zsh's last statement is the `force-list` `&&` chain, which fails when no `force-list` style is set — the common case. The port has a unit test named `returns_zero` |
| `4a4f853fa6` | 2 | `_all_labels` / `_wanted` with no action word: zsh runs the leftover group options as a command and says `command not found: -J`; zshrs accepts silently |
| `5c6b1d2ee2` | 2 | `_pick_variant`: the probed command's stdout reaches the TERMINAL. zsh captures it in `$output` |
| `3dd81d7db8` | 2 | `_normal`: leaves `precommands` as `('')`; zsh leaves it empty |
| `8379d2af48` | 1 | nested diagnostic loses its line number: `_all_labels:comptags:26:` vs `_all_labels:comptags:` |
| `15256af8f2` | 1 | `_call_program` runs the command with `Command::new("sh")` (`_call_program.rs:104`); zsh uses its own `eval` (sh:26-33). Different diagnostic (`sh: …` vs `(eval):1: …`) AND different status (1 vs 127) |
| `21055f3134` | 1 | `_files -/` (the SHELL function on both shells, via the ported `_path_files`) leaves `expl` as `('')` |

Three of these are one bug family — a native port that does not reproduce the
shell function's `local` shadowing of `expl`, in both directions (leaking a
value out, and deleting the caller's binding). The base shell is not at fault:
`local -a expl` in a caller with `local`/`set -A`/`eval`/`${(P)}` writes in a
callee is byte-identical on both shells.

The TIMEOUT (`_wanted -2 -V g options expl opt compadd -o`) is one-sided
silence from the REFERENCE zsh — no output for 10s, twice, including the serial
re-run. Not scored as a pass, and named rather than folded into FAIL.


### The rest of the router table (`--fn-derive`)

The 24 probes above are hand-written because their interface is an argument
GRAMMAR that has to be read out of the function's own `zparseopts` line. That
left the rest of the table unswept — measured on this host, **24 of 246 router
arms had a probe, so 223 were never called by this harness at all**.

Most of the 223 do not HAVE a grammar. They are value generators — `_users`,
`_file_modes`, `_locales`, `_terminals` — whose body ends in
`_wanted`/`_description`/`compadd` and forwards `"$@"` as compadd options. For
those a BARE call is the documented use: it is exactly what
`_alternative 'users:user:_users'` writes. So the call is not invented, it is
DERIVED, and the derivation is a scan of the function's own stock source:

* resolve the name against `$fpath` the same way `fn_backend_map` does, so the
  citation names the file that will really run;
* strip comment lines and look for `$1`..`$9`, `${1`..`${9`, `$argv[` or
  `shift`. Any of those means the function needs arguments this file cannot
  invent, and NO probe is derived — the name is reported with the construct
  that disqualified it, never quietly dropped;
* `$@` / `$*` do not disqualify: forwarding an empty `"$@"` to compadd is the
  passthrough shape and is well defined with no arguments;
* a body that drives ZLE (`zle`, `vared`, `read -k`) is excluded and named — it
  is a widget, and inside a completion widget it waits for input that never
  arrives, so every such cell would be a TIMEOUT measuring the harness.

Measured on this host, 2026-08-30:

| | before | after |
| --- | ---: | ---: |
| functions with a probe | 24 | **170** |
| generated calls | 99 | **391** |
| router arms with no probe | 222 | **76**, each named with its reason |

Of the 76: 70 use a positional, 4 have no file on this `$fpath`, and 2
(`_complete_help_generic`, `_read_comp`) drive ZLE.

```sh
scripts/comptab_parity.py --fn-list --fn-derive          # the table and the 76 exclusions
scripts/comptab_parity.py --fn-sweep --fn-derive 20      # the 24 + 20 derived
scripts/comptab_parity.py --fn-sweep --fn-derive-only _users,_file_modes \
                          --fn-only _users,_file_modes   # ONLY those two
```

`--fn-derive-only` ADDS to the table; it does not restrict the run. Pair it
with `--fn-only` when the point is to drive just the derived ones, or the 24
hand-written probes come along and the run is 103 cells instead of 4.

`--fn-derive` is OPT-IN and a plain `--fn-sweep` still drives exactly the 24
hand-written probes. That is deliberate: a full derived sweep is 391 calls,
each booting two shells.

**First derived cells, and they found something.** Two functions, four calls,
35.4 s, 2026-08-30 over the private dump:

```
FAIL   _users/bare       _users                  [3de4771a42]  1 row differs
FAIL   _users/group-J    _users '-J' 'zpfgrp'    [3de4771a42]  1 row differs
PASS   _file_modes/bare      _file_modes
PASS   _file_modes/group-J   _file_modes '-J' 'zpfgrp'
```

`3de4771a42` is an EXISTING fingerprint — the caller's `expl` comes back
holding the callee's value, which the first sweep found on `_alternative` and
`_path_files`. So no new fingerprint was written (correctly: the bug shape is
already on file), but a new FUNCTION carries it, and it was found by the first
two derived probes ever run. That is the whole argument for deriving the rest:
the 24 hand-written probes were the framework, and the bug family they exposed
lives in the 149 that were never called.

### The return-status axis (`--fn-repeat`, `--fn-propagate`)

`-- rc:` has always been in the report, and it is what caught `_setup`
returning 0 where zsh returns 1 (`fn/fn_5e5ca020b9.json`) — so "the sweep does
not compare return status" is not true of the default axis, and saying so would
be the opposite mistake. What the report could NOT say is anything about status
beyond the FIRST call, or about what the status DOES:

* `--fn-repeat N` calls the utility N times in one probe and records
  `-- rc[i]:` for each. `_next_label`, `_all_labels` and `_requested` are
  written for `while _next_label …; do … done` and are documented to change
  status across iterations; a port that gets the first answer right and never
  changes it is invisible to a single-call probe. `N=1` (the default) emits the
  report byte-identically to before, so no existing fingerprint moves.
* `--fn-propagate` makes `_zpf_probe` RETURN the utility's status instead of a
  hard 0. The hard 0 is load-bearing by default — it stops the completer chain
  so a second completer cannot re-run the probe — but it also means the status
  is only ever observed, never acted on. Propagated, `_main_complete` does what
  it really does with a non-zero completer and moves down the `completer`
  style, so a status divergence becomes a divergence in the SCREEN, including
  on `--fn-keys ctrl-d` where there is no report at all.

```sh
scripts/comptab_parity.py --fn-only _next_label,_all_labels,_requested \
                          --fn-sweep --fn-repeat 3
scripts/comptab_parity.py --fn-only _setup --fn-sweep --fn-propagate --fn-keys ctrl-d
```

**Measured, and it found nothing — stated as such rather than left implied.**
Both commands above were run 2026-08-30 over the private dump against zshrs
0.12.49 @10:38:

| run | cells | result |
| --- | ---: | --- |
| `--fn-repeat 3` over `_next_label`, `_requested`, `_all_labels` | 11 | 11 PASS, 68.6 s, 0 fingerprints |
| `--fn-propagate --fn-keys ctrl-d` over `_setup` | 4 | 4 PASS, 18.1 s, 0 fingerprints |

Three of those cells are pinned fn findings that this confirms are FIXED:
`_all_labels/no-action` (`4a4f853fa6`) and `_all_labels/dash-prev`
(`8379d2af48`) now pass, as do `_description/plain` (`b3c97da2eb`) and
`_values/plain` (`df47ae1cbe`) on the baseline run. So the axis works and the
functions it was pointed at are clean; it has not yet earned its cost on any
function, and 146 derived probes have not been through it.


## What each instrument can and cannot see

Five harnesses now compare completion behaviour, and a fixing round on
2026-08-30 established that they do not overlap the way their descriptions
suggest: the 36 caller-state corruptions `--fn-sweep` found moved zsh's own
Y-series oracle by **exactly zero** assertions, and the Y oracle's own top
finding (`argv+=` discarding the positionals) has no upstream `.ztst` coverage
of its own — `grep 'argv+=' Test/*.ztst` finds nothing. So the question worth
measuring is not "how many bugs does each find" but "what is each one
structurally unable to see".

What follows is measured against this harness. Every MISS names the mechanism.

### The dump has to be checked FIRST, or none of this measures anything

The first attempt at this matrix produced six "one-sided silence" misses and
one fixture that appeared to have been FIXED. All seven were artefacts, and the
cause is worth more than the matrix was:

```
$ ls -l ~/.zpwr/local/.zcompdump-zpwr-MenkeTechnologies
-rw-r--r--  1 wizard staff  1 Aug 30 10:44

$ zsh -f -c 'fpath=(...); autoload -Uz compinit
             compinit -C -d ~/.zpwr/local/.zcompdump-zpwr-MenkeTechnologies
             print "comps=$#_comps"'
comps=0
```

A peer instance truncated the shared dump to ONE BYTE at 10:44. `resolve_dump`
globs that file out of `$HOME` by default, so from 10:44 onward every cell was
comparing two shells with no completion system at all — and the harness scored
the agreement:

```
--case 'ls /usr/sha' --keys tab      PASS      (buffer still `ls /usr/sha`)
```

That is the `--skip-missing` fake-pass class one layer down: not "the command
is missing" but "the completion system is missing". It also manufactures the
one-sided silences, because a shell that completes nothing emits nothing.

`--check-dump` (ON by default) now asks real zsh for `$#_comps` before any pty
boots and refuses the run if the answer is zero, naming the file and its size.
`--allow-empty-dump` overrides. Refusing can only ever remove fake evidence.
Every number below was taken against a private dump built into `$TMPDIR`
(`comps=51704`), passed with `--dump`, so nothing in this section depends on a
file other processes rewrite.

### The 29 pinned fixtures, replayed through this harness

Fifteen cells replayed 2026-08-30 against `target/debug/zshrs` 0.12.49 @10:38
and Homebrew zsh 5.9.2, `--confirm 1 --silence-recheck 3`, over the private
dump. The rest are reasoned about from the harness's interface and marked so.

| fixture | this harness | evidence / reason |
| --- | --- | --- |
| `cc_match_set` | **DETECTED** | FAIL, 4 rows differ, 9.4 s — the same detail the fixture pins |
| `equals_word_line_rewrite` | **DETECTED** | FAIL, 5 rows, 9.0 s |
| `equals_word_arg_position` | **DETECTED** | FAIL, 5 rows, 8.8 s |
| `completer_style_missing_function` | **DETECTED** | FAIL, 4 rows, 9.4 s |
| `match_count_ask_prompt` | **DETECTED** | FAIL, 1 row, 9.1 s |
| `path_assign_menu_next_trailing_slash` | **DETECTED** | FAIL, 1 row, 10.5 s |
| `last_prompt_false_no_redraw` | **DETECTED** | FAIL, 1 row, 9.8 s |
| `multiline_array_literal` | **DETECTED** | FAIL, 4 rows, 9.7 s — a `compsys_parity` fixture this harness turns out to see, because a `\n` in `--case` is sent raw and both shells continue on PS2 |
| `multiline_heredoc_terminator` | **DETECTED** | FAIL, 8 rows, 8.7 s |
| `multiline_backslash_continuation_cd` | **DETECTED** | FAIL, 4 rows, 9.7 s (needs its `list-grouped false` zstyle; without it, PASS) |
| `reference_crash_uppercase_autoload` | **DETECTED** | `REF-CRASHED`, zsh died on signal 10 (SIGBUS), 14.4 s. The upstream defect reproduces here under the private dump — over the shared 1-byte one it passed 3/3, which is the same artefact |
| `transpose_words_panic` (guard) | DETECTED, holds | PASS, 24.6 s. Now `reg_transpose_words_eol.json` |
| `argv_append_discards_positionals` (guard) | DETECTED, holds | PASS, 4.1 s, as a typed script plus `cr`. Now `reg_argv_append_pparams.json` |
| `dotted_parameter_name_accepted` | DETECTED | FAIL, 1 row differs, 8.1 s, as a typed script plus `cr` |
| `multiline_squote_corrections` | MISSED — one-sided silence | TIMEOUT, 62.1 s. The reference produced NOTHING after `tab` at 10 s and again at 30 s while zshrs drew a screen — `--silence-recheck 3` confirms silence, not budget. A real divergence the harness cannot score |
| `multiline_dquote_parameter_list` | MISSED — one-sided silence | TIMEOUT, 63.0 s, same shape, same confirmation |
| `fpath_underscore_name_runs_undeclared` | MISSED, structural | needs a file STAGED in a scratch `$fpath` before boot. `--init-extra` runs after `compinit`, far too late; `--layout-fuzz` can stage one and its first run found this exact bug independently |
| the 8 `compsys_spec_fuzz` fixtures | MISSED, now REACHABLE | each needs a completer generated for the occasion — an `_alternative` spec, a `compset -P`, a `zle -C` widget, a `compdef -K` binding. No mode here could define one; that is what `--init-extra` was added for |
| `listing_lost_on_window_shrink` | MISSED, structural | the pty is sized once at `--rows`x`--cols` and never resized |
| `listing_row_erase_to_eol` | MISSED, structural | the assertion is on `\e[K` in the raw STREAM. `--strict-stream` compares diagnostics, not escapes, and two different escape sequences can paint an identical grid |

Read plainly: with a healthy dump this harness detects **11 of the 13 pinned
divergences it can express**, including four `compsys_parity` multiline
fixtures and one `zsh_reference_probe` crash that were attributed to other
harnesses. The two it misses are one-sided silences, and the eight
`compsys_spec_fuzz` ones were unreachable until `--init-extra`.

### The named ztst causes, against this harness

| cause | this harness | reason |
| --- | --- | --- |
| `ZLS_COLORS` `lc=`/`rc=` ignored (73 of 88 Y assertions) | MISSED — now CLOSED | `GEN_LIST_COLORS`, the `--style-fuzz` value table, held `ma di ln ex no fi so or sp ec` and pattern forms but **no `lc=` or `rc=`**. The hardcoded `\e[`+cap+`m` in `zlrputs` was byte-correct for every value the generator could produce, so no amount of style fuzzing could ever have reached it. `lc=`/`rc=`/marker forms added to the table, and `reg_complist_lc_rc_markers.json` pins it |
| `zparseopts` mis-parses its own option words (20) | MISSED — now CLOSED, and it DIVERGES against 5.9.2 | nothing in the corpus typed a `zparseopts` call. Three `reg_zparseopts_*` entries; two of them are open divergences against the 5.9.2 reference (above) |
| `${~var}` in a nested pattern operand (20) | MISSED — now CLOSED | same reason: no corpus entry typed the construct. `reg_tilde_pattern_operand_*` |
| `TYPESET_TO_UNSET` + `typeset -h +g -m '*'` (10) | MISSED — now CLOSED, still DIVERGES | same reason. `reg_typeset_to_unset_pattern_hide` |
| `argv+=` (Y series, no upstream coverage) | MISSED — now CLOSED | same reason. `reg_argv_append_pparams` |
| the four hangs (D04, D07, W02, X06) | PARTIAL | a hang can only ever be a TIMEOUT here, and a TIMEOUT is deliberately not scored as a divergence. Two of the four are typed-buffer reachable and are now pinned as timing guards |
| GDBM `ztie` (27), `zsh/random` (8) | NOT APPLICABLE | module availability on the build, not shell behaviour |
| `zformat -q`/`-Q` (6) | NOT APPLICABLE to completion | `Completion/` uses only `-f`/`-F`/`-a`, and `NOTES.md` refutes V13zformat as a compsys cause. `zformat -F` with `_description:89`'s exact call is byte-identical on both shells |

### The reverse: what this harness finds that the ztst oracle cannot

| finding | why upstream's suite cannot see it |
| --- | --- |
| the 13 `fn/fn_*.json` caller-state divergences | upstream drives completion through `comptest` KEYSTROKES. It never calls a utility directly, so `expl` leaked out of a callee, `expl` deleted from the caller, `_lastdescr` short by one element, or `_setup`'s status are invisible unless they change a rendered match line. Demonstrated, not argued: fixing all 36 of those state checks moved the Y-series oracle by zero assertions |
| the `fp_*.json` style-fuzz reproducers | upstream's Y files set one fixed style set in `comptestinit`. They never vary the VALUE grammar of `matcher-list`, `tag-order`, `group-order`, `format` or `list-colors`, so a config-shaped divergence has nothing to come from |
| the `--layout-fuzz` findings — an undeclared `_name` in `$fpath` being executable, `compinit -i` not dropping an insecure directory, doubled-colon error frames | `Test/comptest` builds ONE layout: one fpath, one dump, one `compinit` invocation. Where a completer is stored and how it is found is not an axis upstream varies at all |
| the `zparseopts` stacking divergence | upstream's oracle is a BUILT 5.9.999.3-test tree, which has `88d51a2400`; this harness's oracle is `zsh` on `PATH`, which is 5.9.2, which does not. The two references disagree, so only running both asks both questions |
| the C-locale multibyte closure answer | upstream's suite runs in the host locale; `child_env` here pins `LC_ALL=C`, which is the one setting where this bug is live |

### The two gaps this round measured

**1. An input the harness never checked.** The dump section above is the
bigger of the two: a shared file, rewritten by other processes, silently
turning every cell into a comparison of two shells with no completion system.
It produced seven wrong rows in the first draft of the matrix above — six false
"one-sided silence" misses and one fixture that appeared to have been fixed —
and it would have shipped as a finding. `--check-dump` refuses that run now.

**2. A reference shell that legitimately produces nothing is
indistinguishable from one that is hanging**, so the cell becomes a TIMEOUT and
is never compared. Two of the fifteen cells replayed above survive a healthy
dump and still die on this: `multiline_squote_corrections` and
`multiline_dquote_parameter_list`, where the reference draws nothing after
`tab` while zshrs draws a screen.

The TIMEOUT rule is not wrong — `timeout_reasons` only fires for a ONE-SIDED
silence, precisely so that a key which legitimately draws nothing on BOTH
shells is still compared, and the "it might just be slow" hypothesis has to be
disproved before the label can be dropped. So it was disproved, by measurement:
`--silence-recheck 3` re-measured both cells at 30 s and both were still
silent.

Two additions came out of that, both of which only ever make the harness
measure MORE:

* `--key-budget SECONDS` — how long to wait for the first byte after a key
  (default 10.0, unchanged). It can only make the harness wait longer.
* `--silence-recheck X` — on a one-sided-silence TIMEOUT, re-measure once at X
  times the budget. If the side is still silent the detail line says
  `ONE-SIDED SILENCE, not budget` and the JSON carries
  `one_sided_silence: true`. The STATUS deliberately stays `TIMEOUT`: six run
  modes each keep their own counters and exit expression over a fixed label
  set, and a new label missed in one of them would drop the cell out of the
  exit status. Non-pass and non-zero exit are inherited unchanged. OFF by
  default, and it hangs off the existing serial TIMEOUT re-check, so
  `--no-timeout-recheck` disables it too.

Order matters between the two: run the silence recheck over an empty dump and
it will confidently report silence that is really a missing completion system.
`--check-dump` is a precondition for the rest of the instrument, not a
convenience.


## Storage and lookup (`--layout-fuzz`)

`--mutate` varies what is TYPED, `--style-fuzz` varies how the shell is
CONFIGURED. Neither touches the layer underneath: every other mode runs one
fpath (the user's real directories), one dump, loaded one way
(`compinit -C -d`). `--layout-fuzz` varies that layer — where a completer is
stored and how it is found — and holds both shells to what the documentation
says, quoted in the source next to each rule
(`Doc/Zsh/func.yo:93-130`, `Doc/Zsh/compsys.yo:154-201`,
`Completion/compinit:469-528`, `Completion/compaudit:125-163`).

It is not a hypothetical axis: round 3 traced seven "timeouts" to the reference
zsh SEGFAULTING while autoloading out of a 35MB `.zwc` digest, which vanished
when the same completers were plain files.

```sh
# the catalog, the axes and the documented rule each entry pins — no shells
scripts/comptab_parity.py --layout-list

# the first N layouts of the catalog
scripts/comptab_parity.py --layout-fuzz 8

# one named layout, which is also what a failure prints as its replay
scripts/comptab_parity.py --layout-only insecure-i --layout-keep

# seeded random combinations instead of the catalog
scripts/comptab_parity.py --layout-random 6 --seed 7

# the cross-shell .zcompdump report (each shell writes one, both read both)
scripts/comptab_parity.py --dump-xshell
```

### The axes

| axis | values |
| --- | --- |
| store | `plain`, `digest` (only inside `<dir>.zwc`), `digest-stale` (digest older than the directory — the plain file must win), `digest-shadow` (digest newer — the digest must win), `digest-explicit` (the fpath element IS the `.zwc`), `digest-corrupt` (truncated mid-file) |
| fpath | `single`, `dup`, `missing` (a nonexistent dir first), `unreadable` (mode 000), `symlink`, `two-dirs` (same completer twice, leftmost must win), `tag-mismatch` (a file whose `#compdef` claims a command another file is named for) |
| compinit | `-C -d`, `-i -d`, `-u -d`, `-d`, `-D`, bare (`$ZDOTDIR/.zcompdump`), and the default prompting path |
| dump | written by zsh, written by ZSHRS, missing, stale (`#files:` count rewritten), corrupt, none |
| security | secure, world-writable completer directory, file owned by another user |

Both shells always get the byte-identical layout and the byte-identical init
file, and the init file itself resets the dump to the layout's defined state —
otherwise a `compinit` that autodumps hands whichever shell runs second a dump
the first one wrote, which is a different input under the same name.

The scratch tree lives under `$TMPDIR` and is removed at the end
(`--layout-keep` keeps it). It contains a private copy of the zsh distribution
functions at 0755/0644, because the installed copy on this host is mode 0777 —
which `compaudit` calls insecure, which would have made every layout insecure
and the security axis meaningless.

A PASS is only counted as an OBSERVED pass when at least one of the two screens
carries a marker from the layout's own completer; a pass where neither shell ran
it means the two shells agreed about something else and the layout was never
exercised, so it is counted and printed apart. On the 30-layout run below all 22
passes were observed, and `-v` prints the screen for each.

### What it found on its first run

30 layouts, 27 run: 22 PASS, 5 FAIL over 4 fingerprints, 2 INVALID-LAYOUT,
2 REF-WARNED, 1 UNBUILDABLE.

* **An undeclared `_name` in `$fpath` is executable in zshrs.** With
  `fpath=(DIR)` and `DIR/_zzimpl` (or `DIR.zwc` containing it) and no
  `autoload` anywhere, zsh answers `command not found: _zzimpl` and zshrs runs
  it. A non-underscore name is `command not found` on both, so the rule zshrs
  is applying is "unknown command starting with `_` → search fpath and
  autoload". Two layouts (`digest-only`, `digest-explicit`).
* **`compinit -i` does not drop an insecure directory.** With a 0777 completer
  directory, zsh removes it from `fpath` (`Completion/compinit:452`) and
  completes nothing; zshrs keeps it and runs the completer. `-u` and `-C` agree
  on both shells, so it is `-i` specifically (`insecure-i`).
* **Error locations from inside a completion carry a doubled colon and the
  wrong frame.** On a truncated digest, zsh emits `_complete:117:`,
  `_main_complete:218:`, `(eval):1:`; zshrs emits `_main_complete::117:`,
  `_main_complete::`, `(eval)::1:` — always the outermost frame, and one colon
  too many. Visible from the shell as well: `zsh:1: invalid zwc file:` versus
  `zshrs::1: invalid zwc file:` (`digest-corrupt`, `digest-corrupt-D`).

### Cross-shell `.zcompdump` compatibility (`--dump-xshell`)

Measured, both directions, on one controlled layout:

* **Reading is compatible.** Each shell reads either shell's dump to the same
  state: `comps=1855 services=48 patcomps=1` from either file, in either shell.
* **Writing is not identical.** zsh's dump is 52559 bytes, zshrs's 57771, and
  they differ on 670 lines — all of them `_compautos` entries with empty
  values. `Completion/compinit:524` stores a `#autoload` file in `_compautos`
  only when its tag line carries options; zshrs stores every one. Read back,
  that is `compautos=1` from the zsh-written dump against `compautos=178` from
  the zshrs-written one, in both shells.
* **zshrs's `compinit -d FILE` writes no dump at all.** zsh produces it from
  `compinit` itself (`Completion/compinit:532-535`); on zshrs the file only
  appears if `compdump` is called by hand afterwards, and `compdump` on its own
  works. The report prints which path produced each dump rather than papering
  over the difference.
