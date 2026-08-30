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
