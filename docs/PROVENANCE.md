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
$ provenance -m REPORT
$ REPORT=$(date +%Y-%m-%d)
$ ARCHIVE=${REPORT}.tar.gz
$ provenance -m ARCHIVE
$ tar czf $ARCHIVE .
$ provenance REPORT
REPORT
  origin: cmdsubst "date +%Y-%m-%d" (line 2)
  ops:
     1. assign     REPORT "2026-08-18"                      line 2
     2. expand     $REPORT "2026-08-18"                     line 3
     3. concat     "2026-08-18" ".tar.gz"                   line 3
```

The chain starts at an **origin** — the event that created bytes that did
not exist in the shell before — and extends with one **op** per bytecode
event afterwards.

Origins:

| Origin        | Created by                                        |
|---------------|---------------------------------------------------|
| `cmdsubst`    | `$(…)` / `` `…` ``                                 |
| `procsubst`   | `<(…)`, `>(…)`, `=(…)`                             |
| `glob`        | a pathname expansion of at most 32 matches         |
| `heredoc`     | `<<EOF` body                                       |
| `herestring`  | `<<<` body                                         |
| `param NAME`  | the value the parameter held when `-m` armed it    |
| `assign …`    | an assignment whose value has no earlier lineage   |

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
| `unset`  | `unset NAME`                                                      |

## Surface

```
provenance                  list every tracked parameter with its lineage
provenance NAME…            print NAME's lineage
provenance -m NAME…         start tracking NAME
provenance -u NAME…         stop tracking NAME and drop its lineage
provenance -j NAME          print NAME's lineage as JSON
provenance -l               same as no arguments
provenance -c               clear every lineage and disarm the engine
```

Exit status is 1 when a named parameter is not tracked, when an option
is malformed, or when the engine is disabled.

The JSON form is stable and flat:

```json
{"name":"REPORT","origin":"cmdsubst \"date +%Y-%m-%d\"","origin_line":2,
 "ops":[{"op":"assign","args":["REPORT","\"2026-08-18\""],"line":2}],
 "dropped_ops":0}
```

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
| `singsub` / `multsub` | `src/ported/subst.rs` | word expansion |
| `assignsparam` / `assignaparam` / `sethparam` / `unsetparam` | `src/ported/params.rs` | parameter writes |

Every tap is guarded by `provenance::active()` at the call site.

### Cost

Disarmed: one relaxed atomic load per tap; no allocation, no lock, no
map lookup. Armed: one mutex acquisition and an O(1) map lookup per tap,
plus an O(tracked-names) scan in the word-expansion tap — tracked names
are user-armed and few.

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
