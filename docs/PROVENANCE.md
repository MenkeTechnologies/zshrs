# PROVENANCE.md — value lineage over bytecode execution

**Status:** shipped
**Layer:** bytecode host taps + parameter write funnels + `provenance` builtin
**Engine:** `src/extensions/provenance.rs`
**Config:** `[provenance] enabled` in `~/.zshrs/zshrs.toml`, `ZSHRS_PROVENANCE=0`
**Relation to the recorder (PFA-SMR):** none. Separate subsystem, separate
storage, no daemon, no catalog. The recorder answers *"what state did this
shell define, and where?"*; provenance answers *"how was this value built?"*

## What it does

`provenance -m NAME` arms tracking for a parameter. From that point the
engine records, in order, every bytecode-level event that produces or
consumes that parameter's value:

```console
$ cat build.zsh
provenance -m REPORT
REPORT=$(date +%Y-%m-%d)
ARCHIVE=${REPORT}.tar.gz
provenance REPORT
$ zshrs build.zsh
REPORT
  origin: cmdsubst "date +%Y-%m-%d" (build.zsh:2, 2026-08-18 11:27:50.486)
  ops:
     1. assign     REPORT "2026-08-18"                      build.zsh:2              11:27:50.486
     2. expand     $REPORT "2026-08-18"                     build.zsh:3              11:27:50.486
     3. concat     "2026-08-18" ".tar.gz"                   build.zsh:3              11:27:50.486
```

The chain starts at an **origin** — the event that created bytes that did
not exist in the shell before — and extends with one **op** per bytecode
event afterwards.

Every row carries where and when it happened: the file and line, and the
local wall clock to the millisecond. The origin line spells the date out;
op lines print the time alone, and add the date only when the op happened
on a different day than the origin. Where there is no file — `zshrs -c`
input, an interactive line — the row reads `line N` instead.

The chain is the whole life of the parameter, not of one value: every
reassignment appends its own op and nothing earlier is dropped. When a
later assignment brings a value that carries its own lineage, that
lineage is spliced in — an `origin` op naming it, then its ops — rather
than replacing what the parameter already recorded:

```console
$ zshrs build.zsh
z
  origin: param z = "seed" (build.zsh:2, 2026-08-18 11:27:54.601)
  ops:
     1. assign     z "23"                                   build.zsh:3              11:27:54.601
     2. origin     cmdsubst "echo built"                    build.zsh:4              11:27:54.602
     3. assign     z "built"                                build.zsh:4              11:27:54.602
```

Origins:

| Origin        | Created by                                        |
|---------------|---------------------------------------------------|
| `cmdsubst`    | `$(…)` / `` `…` ``                                 |
| `procsubst`   | `<(…)`, `>(…)`, `=(…)`                             |
| `glob`        | a pathname expansion of at most 32 matches         |
| `heredoc`     | `<<EOF` body                                       |
| `herestring`  | `<<<` body                                         |
| `param NAME`  | the value the parameter held when `-m` armed it    |
| `assign …`    | the first assignment to a parameter armed while unset, when the value has no earlier lineage |

Ops:

| Op       | Recorded when                                                    |
|----------|------------------------------------------------------------------|
| `assign` | `NAME=value` (scalar), incl. subscripted forms                    |
| `append` | `NAME+=value`                                                     |
| `array`  | `NAME=(…)`                                                        |
| `assoc`  | an associative-array write                                        |
| `expand` | a `$NAME` / `${NAME…}` bytecode read (`BUILTIN_GET_VAR`)          |
| `concat` | a word-segment concat that consumed a value with a lineage        |
| `exec`   | the value was passed to an external command, with its argv slot   |
| `call`   | same, for a shell function                                        |
| `origin` | a later assignment brought a value carrying its own lineage — that lineage's origin, followed by its ops |
| `unset`  | `unset NAME`                                                      |

## Surface

```
provenance                  list every tracked parameter and function
provenance NAME…            print NAME's lineage
provenance -m NAME…         start tracking NAME
provenance -u NAME…         stop tracking NAME and drop its lineage
provenance -j NAME          print NAME's lineage as JSON
provenance -f …             act on shell functions instead of parameters
provenance -a               track everything from here on
provenance -ua              stop arming new names (keeps what is recorded)
provenance -l               same as no arguments
provenance -c               clear every lineage and disarm the engine
```

Flags bundle: `-mf NAME` arms a function, `-jf NAME` prints one as JSON,
`-uf NAME` drops one.

## Tracking everything

`provenance -a`, or `track_all` in the config, arms every parameter the
shell writes and every shell function it defines or calls — no `-m` at
all. The config form is armed before the first rc file runs, so it
covers the whole life of the shell:

```toml
# ~/.zshrs/zshrs.toml
[provenance]
track_all = true
```

`ZSHRS_PROVENANCE_ALL=1` (or `=0`) overrides that field from the
environment; `enabled = false` / `ZSHRS_PROVENANCE=0` still wins over
both and refuses to arm anything.

Two bounds keep a track-everything session finite:

* **Volatile parameters are never armed by themselves** — `LINENO`,
  `RANDOM`, `SECONDS`, `EPOCHSECONDS`, `EPOCHREALTIME`, `HISTCMD`,
  `COLUMNS`, `LINES`, `status`, `pipestatus`, `_`, `?`, `!`, `$`, `#`,
  and the positionals. The shell rewrites these on its own, once or more
  per command; their chains would record its bookkeeping and nothing the
  user did. `provenance -m LINENO` still arms one by hand.
* **A ceiling of 4096 auto-armed names.** Past it, new names are counted
  and ignored, and the listing ends with
  `… N more names not tracked (cap 4096)`.

## Shell functions

A function has a lineage of its own — where it was defined, every
redefinition, every call, and the removal that ended it:

```console
$ cat greet.zsh
provenance -a
greet() { MSG="hi $1"; }
greet world
greet again
provenance -f greet
$ zshrs greet.zsh
greet()
  origin: function greet (greet.zsh:2, 2026-08-18 11:48:01.139)
  ops:
     1. call       greet()                                  greet.zsh:3              11:48:01.139
     2. call       greet()                                  greet.zsh:4              11:48:01.140
```

| Op           | Recorded when                                                |
|--------------|--------------------------------------------------------------|
| `call`       | the function is about to run — at the *caller's* site         |
| `redefine`   | the name is defined again, at the new body's site             |
| `unfunction` | `unfunction NAME` / `unset -f NAME`                           |

The origin is the definition site `shfunctab` recorded
(`Src/exec.c:5383-5388`): the defining file and the line the definition
starts on. Functions and parameters are separate namespaces — `path` and
`path()` keep separate chains — so function subcommands need `-f`.

Exit status is 1 when a named parameter is not tracked, when an option
is malformed, or when the engine is disabled.

The JSON form is stable and flat. `file` and `function` are `null` where
there is none; `time` is RFC 3339 local time with milliseconds:

```json
{"name":"REPORT","origin":"cmdsubst \"date +%Y-%m-%d\"","origin_line":2,
 "origin_file":"build.zsh","origin_function":null,
 "origin_time":"2026-08-18T11:27:50.486-04:00",
 "ops":[{"op":"assign","args":["REPORT","\"2026-08-18\""],"line":2,
         "file":"build.zsh","function":null,
         "time":"2026-08-18T11:27:50.486-04:00"}],
 "dropped_ops":0}
```

(printed on one line; wrapped here for the page.)

## Turning it off

The engine records nothing until the first `provenance -m`; before that
every tap is a single relaxed `AtomicBool` load. To refuse arming
entirely — so no ledger can exist in the process at all — either:

```toml
# ~/.zshrs/zshrs.toml
[provenance]
enabled = false
```

or set `ZSHRS_PROVENANCE=0` in the environment, which overrides the
config file. With the engine disabled, `provenance -m` reports

```
zshrs: provenance: disabled by config
```

and exits 1; every other subcommand still works against an empty ledger.

## How it works

The engine is a port of stryke's `strykelang/provenance.rs` (the
`mark` / `provenance` / `unmark` trio). stryke keys a value's lineage on
the `Arc<HeapObject>` pointer, which works there because a stryke value
stays one `Arc` from creation to use. A shell value does not: the VM/host
boundary passes `String`, and the parameter table stores `String`, so
pointer identity dies at the first assignment — the hole stryke's own
docs record for its string results is the *common* case in a shell. The
port therefore keys on three things:

* **Ptr** — `Arc` identity of an in-flight `fusevm::Value`
  (`Str(Arc<String>)`, `Array(Arc<Vec<Value>>)`). This is stryke's
  mechanism, and it is exact while the value stays inside a chunk. Each
  row stores a `Weak` next to the address, and a lookup that cannot
  upgrade it (or upgrades to a different address) reaps the row rather
  than reporting a lineage belonging to a recycled allocation — stryke's
  v1.1 staleness fix, ported as-is.
* **Name** — the tracked parameter. Survives every assignment and
  expansion round trip.
* **Content** — the exact bytes of a value that crossed a `String`-typed
  boundary, so a command substitution's output is still recognisable when
  it reaches `assignsparam` a few ops later. These rows are speculative
  (recorded before anyone knows whether the bytes will reach a tracked
  name) and are bounded by a FIFO of 8192 entries.

### Tap sites

| Site | File | Event |
|------|------|-------|
| `BUILTIN_SET_LINENO` | `src/fusevm_bridge.rs` | mirrors `$LINENO` into the ledger's own counter |
| `BUILTIN_GET_VAR` / `_DQ` | `src/fusevm_bridge.rs` | parameter read |
| `concat_splice_prov` / `concat_plan9_prov` | `src/fusevm_bridge.rs` | word-segment concat |
| `paramsubst_to_value_pf` | `src/fusevm_bridge.rs` | `${…}` fast-path expansion |
| `ShellHost::glob` / `heredoc` / `herestring` / `exec` / `call_function` / `cmd_subst` / `process_sub_*` | `src/fusevm_bridge.rs` | origins and consumption |
| `ShellExecutor::run_command_substitution` | `src/vm_helper.rs` | in-process `$(…)` |
| `assignsparam` / `assignaparam` / `sethparam` / `unsetparam` | `src/ported/params.rs` | parameter writes |
| `execfuncdef` install | `src/ported/exec.rs` | function definition / redefinition |
| `doshfunc` entry | `src/ported/exec.rs` | function call, at the caller's site |
| `bin_unhash` shfunc removal | `src/ported/builtin.rs` | `unfunction` / `unset -f` |

Every tap is guarded by `provenance::active()` at the call site.

### Where and when a row happened

Each origin and each op stores a site: line, file, enclosing function,
and a millisecond wall clock, captured at hook entry before the ledger
lock is taken.

* **Line** comes from the ledger's own `$LINENO` mirror, fed by the
  `BUILTIN_SET_LINENO` tap — the hooks never read the parameter table,
  which the param-write hooks run inside.
* **File** is `scriptfilename` (what `PS4`'s `%x` prints) at top level,
  and the file the running function was *defined* in while a function
  frame is on top of `funcstack`. `zsh -c` stamps the literal `zsh`
  there (`Src/init.c:479`); that is a shell name, not a file, so the row
  reads `line N`.
* **Inside a function**, `$LINENO` counts from the function's own first
  line, so the site adds the frame's definition line to it — the
  `f->prev->flineno + f->lineno` sum `funcfiletrace` reports
  (`Src/Modules/parameter.c:747`) — and the report names the function:

  ```console
  NAME
    origin: cmdsubst "echo 42" (lib.zsh:2 (build_name), 2026-08-18 11:25:31.862)
    ops:
       1. concat     "42" ".log"                              lib.zsh:3 (build_name)   11:25:31.862
       2. assign     NAME "42.log"                            lib.zsh:3 (build_name)   11:25:31.862
  ```

  A `source` or `eval` frame above the function makes `$LINENO` count in
  *that* text instead, so no definition line is added and no function is
  named.
* **Clock** is `SystemTime::now()` at hook entry, rendered in local time.
  Two ops with the same op name, operands, file and line collapse into
  one row even when their clocks differ — the clock never defeats the
  repeat collapse, or nothing would ever collapse.

### Cost

Disarmed: one relaxed atomic load per tap; no allocation, no lock, no
map lookup. `track_all` arms the engine for the life of the shell, so
every tap does real work from startup: measured at +0.9 ms on a
`zshrs -f -c true` debug-build startup (18.2 → 19.1 ms, 20 runs each).
Armed: one mutex acquisition and an O(1) map lookup per tap,
plus the site capture — a `scriptfilename` read, a `funcstack` `try_lock`
(never a blocking one: a hook can fire anywhere the VM runs) and a clock
read.

Every tap keys on an exact identity — an `Arc` address, a parameter
name, or the full value bytes. No tap infers a link by scanning word
text for a parameter's name: an earlier revision did, and `eval "x=\$F"`
(where `$F` is escaped and never expanded) was attributed to `F`.

## Limitations

These are deliberate and match the shape of stryke's documented v1 list.

* **Parameter-granular, not value-granular.** Lineage is anchored to
  parameters the user armed. An intermediate value that never lands in a
  tracked parameter keeps a lineage only for as long as its content row
  survives the FIFO.
* **A write is recorded at the funnel head.** `assignsparam` and friends
  tap before the write is committed, so an assignment rejected further
  down (`read-only variable`) still appears in the chain as an attempted
  `assign`.
* **Chains are capped at 256 ops.** Past the cap ops are counted, not
  stored, and the report ends with `… N more ops (chain capped at 256)`.
  Without the cap, tracking a parameter the shell itself reads on every
  prompt would grow without bound.
* **Immediate repeats collapse.** An op identical to the chain's last op
  — same op, operands and line — is not appended twice, so a value read
  several times while evaluating one statement records once.
* **High-fanout globs are skipped.** A pathname expansion of more than 32
  matches records nothing; recording it would evict the whole content
  FIFO for matches nobody is tracking.
* **Subshells do not merge back.** A lineage created inside `(…)` or a
  forked pipeline stage lives in that process's ledger and dies with it,
  exactly as its parameter values do.
