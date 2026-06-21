# RECORDER_PROTOCOL.md — Wire contract for third-party shell recorders

**Status:** v1, stable for the lifetime of the v1.x daemon protocol.
**Audience:** anyone implementing a recorder client for a shell other
than zshrs (fish, bash, ksh, dash, nushell, elvish, pwsh, …).

This doc is the federation contract. Implement it and your shell's
state mutations land in the same canonical catalog `zshrs-recorder`
writes to. Cross-shell `zd defs query / diff` becomes possible.

For background on *why* recorders exist (vs the AST-walk approach the
daemon used to take), see `docs/RECORDER.md`. For the daemon's full
HTTP API, see `docs/DAEMON_AS_SERVICE.md`. For the registry of
reserved `shell_id` strings, see `docs/SHELL_IDS.md`.

## At a glance

A recorder is a shell-side instrumentation layer that captures every
state mutation during shell init (or any other window) and ships the
captures to `zshrs-daemon` as a bundle of records. Two transports:

| Transport | Op | Use case | Example |
|---|---|---|---|
| **Batch** | `POST /op/recorder_ingest` | End-of-run; one bundle = the entire init pass. Replaces all rows for `shell_id` in every kind the bundle touches. | `zshrs-recorder` (zsh AOP), a `fish-recorder` runner |
| **Single** | `POST /op/definitions_emit` | Live; one record per call, fired from inside a shadowed builtin / DEBUG trap. Idempotent per `(kind, name, shell_id)`. | bash `function alias { … }` shadow that emits as it sees each `alias` |

Both transports write to the same canonical catalog and respect the
same federation rules (`(subsystem, name, shell_id)` → row). They are
freely mixable: a bash session can emit live during init, then a
fish session can ingest a bundle, then `zd defs diff bash fish` shows
the delta between them.

## Endpoints

```
POST {DAEMON_URL}/op/recorder_ingest    # batch
POST {DAEMON_URL}/op/definitions_emit   # single record
GET  {DAEMON_URL}/stream/definitions    # SSE — fired on every ingest
```

Default `DAEMON_URL` = `http://127.0.0.1:7733`. Auth: bearer token in
`Authorization: Bearer <tok>` if `[http.tokens]` is set in
`~/.zshrs/daemon.toml`. See `docs/DAEMON_AS_SERVICE.md` §"Authentication".

## Bundle wire format (`recorder_ingest`)

JSON object. Required fields are starred.

```jsonc
{
  "started_at_ns":  1777777000000000000,    // *u64 — capture window start (Unix ns)
  "finished_at_ns": 1777777005000000000,    // *u64 — capture window end (Unix ns)
  "cmdline":        "fish-recorder --init", // string — what launched the recorder
  "zdotdir":        "/home/jdoe/.config/zsh", // string|null — zsh ZDOTDIR (or analogue)
  "home":           "/home/jdoe",           // string|null — $HOME at capture time
  "shell_id":       "fish",                 // string|null — federated identity (see below)
  "events": [                               // *array — see "Event wire format" below
    { /* … */ },
    { /* … */ }
  ]
}
```

**Bundle is end-state.** When the daemon receives a bundle, it
*replaces* every row tagged with this `shell_id` in every kind the
bundle touches. Existing rows from OTHER shells are untouched
(scope-clear by `shell_id` — see `daemon/canonical.rs:replace_subsystem_tagged`).

**`shell_id` precedence:** per-event `shell_id` overrides the
bundle-level one. Both null = `"zshrs"` (the default for backwards
compatibility with pre-federation bundles).

## Event wire format

Each element of `events[]` is a JSON object. Required fields starred:

```jsonc
{
  "order_idx":   42,                          // *u64 — 0-based monotonic
                                              //  index in capture order
  "ts_ns":       1777777002500000000,         // *u64 — emit time (Unix ns)
  "kind":        "alias",                     // *string — see "Kinds" below
  "name":        "ll",                        // *string — definition name
  "value":       "ls -al",                    // string|null — scalar form
  "file":        "/home/jdoe/.zshrc",         // string|null — defining file
  "line":        42,                          // u32|null — line in file
  "fn_chain":    "_main > _setup_aliases",    // string|null — call chain
  "attrs":       0x0040,                      // u16 (default 0) — ParamAttrs
                                              //  bitset (assign/typeset only)
  "value_array": ["foo", "bar", "baz"],       // array|null — array elements
                                              //  in declaration order
  "value_assoc": [["k1","v1"], ["k2","v2"]],  // array|null — assoc pairs
                                              //  in insertion order
  "shell_id":    "fish"                       // string|null — per-event
                                              //  override of bundle.shell_id
}
```

`order_idx` MUST be unique within a bundle and MUST be monotonic
(later events = larger index). The daemon uses it to disambiguate
ts_ns ties when two events fire in the same nanosecond.

`ts_ns` is a wall clock timestamp. Monotonic-clock-based recorders
should resolve `ts_ns` from `CLOCK_REALTIME` (or equivalent) at emit
time, NOT compute it from a monotonic delta — the latter loses
correlation with `started_at_ns` and breaks cross-bundle merging.

## Kinds

The `kind` discriminant maps 1:1 to a recorder DefKind in
`src/recorder/mod.rs:DefKind` and to a canonical-engine subsystem in
`daemon/definitions.rs:KNOWN_KINDS`. Use exactly these strings:

| `kind` | Meaning | Typical shell construct(s) |
|---|---|---|
| `alias`       | Plain alias               | bash/zsh `alias name=value`, fish `alias name value` |
| `g_alias`     | Global alias              | zsh `alias -g` |
| `s_alias`     | Suffix alias              | zsh `alias -s ext=value` |
| `function`    | Function definition       | bash/zsh `function name { … }`, fish `function name; …; end` |
| `assign`      | Plain variable assignment | `name=value`; record EVERY assign of a non-local variable |
| `typeset`     | Declared/typed assignment | zsh `typeset -aU`, bash `declare -A`, fish `set -gx` |
| `export`      | Exported env var          | `export NAME=value` |
| `path_mod`    | Edit to `$PATH` / `$FPATH`/ `$MANPATH` | `path+=(…)`, `export PATH=…` |
| `hash_d`      | Named directory           | zsh `hash -d name=path` |
| `zstyle`      | zstyle pattern            | zsh `zstyle ':completion:*' …` |
| `bindkey`     | Key sequence bind         | `bindkey '^R' history-search` |
| `compdef`     | Completion binding        | `compdef _git git`, fish `complete -c …` |
| `zmodload`    | Module load               | `zmodload zsh/zle` |
| `setopt`      | Shell option enable       | `setopt extended_glob`, bash `shopt -s …` |
| `unsetopt`    | Shell option disable      | `unsetopt nomatch`, bash `shopt -u …` |
| `trap`        | Signal handler            | `trap 'echo bye' EXIT` |
| `sched`       | Scheduled callback        | zsh `sched +5 'do_thing'` |
| `source`      | File sourced              | `source FILE`, `. FILE` |
| `unalias`     | Alias removal             | `unalias ll` |
| `unset`       | Variable removal          | `unset NAME` |
| `zle`         | ZLE widget definition     | `zle -N my-widget` |
| `completion`  | Discovery of `_completion-name` file | when an fpath dir is added |

If your shell has a construct that doesn't fit one of these, **don't
invent a new kind** — open an issue. Adding kinds is a daemon-side
change (must update `KNOWN_KINDS` + the canonical store + replay).

## ParamAttrs (assign / typeset only)

`attrs` is a u16 bitset. Default `0` means "unspecified" (the daemon
treats it as scalar). Set the bits that apply to your construct so
replay can reconstruct typed declarations exactly:

| Bit | Mask | Meaning |
|---|---|---|
| 0  | `0x0001` | SCALAR    |
| 1  | `0x0002` | INTEGER   (`typeset -i`, `declare -i`) |
| 2  | `0x0004` | FLOAT     (`typeset -F`/`-E`) |
| 3  | `0x0008` | ASSOC     (`typeset -A`, `declare -A`) |
| 4  | `0x0010` | ARRAY     (`typeset -a`, `declare -a`) |
| 5  | `0x0020` | READONLY  (`readonly`, `typeset -r`) |
| 6  | `0x0040` | EXPORT    (`export`, `typeset -x`) |
| 7  | `0x0080` | GLOBAL    (`typeset -g`) |
| 8  | `0x0100` | UNIQUE    (`typeset -aU` — array-element dedupe) |
| 9  | `0x0200` | TIED      (`typeset -T NAME var SEP` — tied scalar/array pair) |
| 10 | `0x0400` | HIDE      (`typeset -h`) |
| 11 | `0x0800` | HIDE_VAL  (`typeset -H`) |
| 12 | `0x1000` | APPEND    (`name+=value` / `name+=(…)`) |

The exact bit layout lives in `src/recorder/mod.rs:ParamAttrs` —
treat that file as the wire spec if a future bit is added (the daemon
ignores unknown bits, so adding bits is wire-compat).

## fn_chain encoding

Outermost call first, innermost last, separated by ` > `:

```
"fn_chain": "_main > _setup_aliases > _company_overrides"
```

Translation: `_main` was called, which called `_setup_aliases`, which
called `_company_overrides` (which is the function that fired the
event). This format matches `src/extensions/recorder.rs:59 recorder_ctx()` and is
what `zwhere -l` / `zd defs query` consumers expect.

If your shell exposes call chain as a stack (innermost-first), reverse
it before emitting. If you can't get the chain at all, leave the field
`null` (the catalog still works; just lose lineage detail).

For non-function call sites (top-level of `.zshrc`, `eval`, etc.),
emit `fn_chain: null` rather than an empty string.

## Sensitive content

Recorders run during shell init; init sources `.tokens.sh`, `.env`,
and similar secret-bearing files. The daemon's `source_resolver.rs`
flags those (see `is_sensitive` at `daemon/source_resolver.rs:183`)
using the same heuristics you should mirror:

**File-name patterns (case-insensitive substring):**
`token`, `secret`, `credential`, `password`, ends with `.env`, contains `.env.`

**Content patterns (first 64 KiB, case-insensitive):**
`AWS_SECRET`, `API_KEY=`, `PASSWORD=`, `PRIVATE_KEY`, `SECRET_ACCESS_KEY`

**Recommended recorder behavior when the source file matches:**

1. Still emit the event (so `unset $SENSITIVE_VAR` lineage is visible)
2. Set `value: null` and `value_array: null` and `value_assoc: null`
   — the value never leaves the shell process.
3. Set `attrs |= 0x0800` (HIDE_VAL) so downstream consumers can tell
   the value was redacted vs genuinely unset.
4. Keep `name`, `kind`, `file`, `line`, `fn_chain` — those carry no
   secret content and are essential for cross-shell auditing.

The daemon does NOT re-redact values for you. If you ship a value
that's a secret, it lands in the catalog. Treat redaction as a
recorder responsibility.

## Batching rules

- **Bundle granularity = one shell invocation.** Each `recorder_ingest`
  call should represent one logical recording window (typically a full
  shell init). Don't merge captures from multiple shell sessions into
  one bundle — they have distinct `started_at_ns`/`finished_at_ns`
  and the canonical store treats the bundle as authoritative
  end-state for that window.
- **Empty bundles are valid.** Sending `events: []` is allowed and
  intentional — it lets a recorder signal "I ran but observed no
  state changes" so the SSE `defs` listeners still fire.
- **Replay-grade ordering.** `events[]` SHOULD be in `order_idx`
  order (the daemon doesn't sort; downstream consumers may rely on
  array order). The recorder is free to defer emit until end-of-run
  (collect into a Vec, ship at exit) or stream over IPC and let a
  helper bundle them.
- **Single-record emits don't batch.** Each `definitions_emit`
  POST is one record. There's no "drain N records" multiplexer
  endpoint in v1; if you have more than ~1000 records, switch to
  `recorder_ingest`.

## Single-record wire format (`definitions_emit`)

Same field set as one `events[]` element, plus a required `shell_id`
at the top level. Use this path when:

- Your shell can't easily collect end-of-run state (no atexit hook,
  no Drop, no equivalent — fire-and-forget bash DEBUG trap).
- You want each definition to be visible to other clients
  immediately, not at end-of-run.
- You're prototyping a recorder and don't want to wire up the full
  bundle assembly yet.

```jsonc
{
  "shell_id": "bash",                 // *string — required at top level
  "kind":     "alias",                // *string — see Kinds table
  "name":     "ll",                   // *string
  "value":    "ls -al",               // string  — optional but typical
  "file":     "/home/jdoe/.bashrc",   // string  — optional
  "line":     42,                     // u32     — optional
  "fn_chain": "main_setup",           // string  — optional
  "attrs":    64,                     // u16     — optional, defaults to 0
  "value_array": null,                // array   — optional, omit if scalar
  "value_assoc": null                 // array   — optional, omit if scalar
}
```

Response shape:

```json
{
  "ok": true,
  "kind": "alias",
  "name": "ll",
  "shell_id": "bash",
  "wrote_rows": 1,
  "file": "/home/jdoe/.bashrc",
  "line": 42,
  "fn_chain": "main_setup"
}
```

## SSE (`/stream/definitions`)

Every successful `recorder_ingest` fires a `defs` SSE event on the
`/stream/definitions` endpoint. Subscribers receive:

```
event: defs
data: {"events_ingested": 142, "rows_written": 138, "elapsed_ms": 3, "shell_id": "fish", "started_at_ns": 1777777000000000000}
```

Use this to drive cache invalidation in clients that have already
queried the catalog (`zd defs query`, editor extensions, etc.).
`definitions_emit` does NOT currently fire SSE — single-record writes
are silent. (Tracked: see DAEMON_AS_SERVICE.md audit item #3 —
`definitions_subscribe` IPC op deferred.)

## End-to-end example: minimal fish recorder

A complete fish recorder takes ~50 lines. The shape:

```fish
# ~/.config/fish/conf.d/zshrs-recorder.fish
#
# Loaded only when $ZSHRS_RECORDING is set (so non-recording fish
# sessions pay zero cost). Shadow `alias`, `function`, `set`, `bind`,
# `complete`, and `source`. Each shadow captures file:line via
# `status filename` / `status line-number`, then delegates to the
# real builtin.

if not set -q ZSHRS_RECORDING
    return
end

set -g _ZSHRS_REC_BUFFER  # cleared at start
set -g _ZSHRS_REC_IDX 0
set -g _ZSHRS_REC_T0 (date +%s%N)

function _zshrs_emit --argument-names kind name value
    set -g _ZSHRS_REC_IDX (math $_ZSHRS_REC_IDX + 1)
    set -l file (status filename)
    set -l line (status line-number)
    set -l chain (status stack-trace | string trim)
    # Append one JSON line to the in-memory buffer.
    set -ga _ZSHRS_REC_BUFFER "{\"order_idx\":$_ZSHRS_REC_IDX,\"ts_ns\":"(date +%s%N)",\"kind\":\"$kind\",\"name\":\"$name\",\"value\":\"$value\",\"file\":\"$file\",\"line\":$line,\"fn_chain\":\"$chain\"}"
end

function alias --no-scope-shadowing
    _zshrs_emit alias $argv[1] $argv[2]
    builtin alias $argv
end

function set --no-scope-shadowing
    if contains -- -gx $argv
        _zshrs_emit env $argv[3] $argv[4]
    end
    builtin set $argv
end

# … same pattern for function / bind / complete / source …

function _zshrs_flush_at_exit --on-event fish_exit
    set -l events (string join , $_ZSHRS_REC_BUFFER)
    set -l t1 (date +%s%N)
    set -l body "{\"started_at_ns\":$_ZSHRS_REC_T0,\"finished_at_ns\":$t1,\"cmdline\":\"fish-recorder\",\"shell_id\":\"fish\",\"events\":[$events]}"
    set -l auth ""
    if set -q DAEMON_TOKEN
        set auth -H "Authorization: Bearer $DAEMON_TOKEN"
    end
    curl -sf $auth -H 'Content-Type: application/json' --data-raw "$body" "$DAEMON_URL/op/recorder_ingest" >/dev/null
end
```

This is the structure `examples/daemon-shell.fish` (the wrapper
library) implements at runtime — the actual fish recorder would
package this conf.d snippet plus a launcher script that sets
`$ZSHRS_RECORDING` and reruns fish.

## Reference implementations

| Recorder | Status | Path |
|---|---|---|
| `zshrs-recorder` (zsh AOP) | shipped | `bins/zshrs-recorder.rs` + `src/recorder/` |
| `fish-recorder`            | not shipped | template above is the spec |
| `bash-recorder`            | not shipped | function-shadowing + DEBUG trap, same shape |

The wrapper libraries under `examples/daemon-shell.{sh,bash,zsh,fish,nu,elv,ksh,ps1}`
already implement `daemon-record-*` (or `daemon record …` /
`Daemon-RecordAlias`) per shell — they're a single-record
`definitions_emit` driver, not a full recorder. A full recorder adds
the AOP layer to capture every call automatically; these wrappers
just give shell users a manual `daemon-record-alias gst 'git status'`
command.

## Conformance checklist

If you're building a new recorder, verify each item before claiming
conformance:

- [ ] Bundle MUST include `started_at_ns`, `finished_at_ns`,
      `events[]` and SHOULD include `shell_id`.
- [ ] Every event MUST include `order_idx`, `ts_ns`, `kind`, `name`.
- [ ] `kind` MUST be one of the 22 enumerated DefKind strings.
- [ ] `shell_id` MUST be from the registry at `docs/SHELL_IDS.md`
      (open a PR adding it before shipping if your shell isn't listed).
- [ ] `order_idx` MUST be monotonic and unique within the bundle.
- [ ] `fn_chain` MUST be outermost-first, ` > ` separated, or null.
- [ ] `attrs` bits MUST follow the `ParamAttrs` layout above; unknown
      bits MUST be set to 0.
- [ ] Sensitive-source events MUST redact `value`/`value_array`/
      `value_assoc` (set to null) and SHOULD set `attrs |= 0x0800`
      (HIDE_VAL).
- [ ] Recorder SHOULD complete within 5× the equivalent un-instrumented
      shell init time. (zshrs achieves <0% via runtime AOP; fish/bash
      shadowing typically 5-10×.)

## Wire-format versioning

This document describes v1.0 of the protocol. Compatible additions
(new optional fields, new bits in `attrs`, new entries in the kinds
table, new shell_ids in the registry) are made WITHOUT a version bump
— they're forward-compatible by design (the daemon ignores unknown
fields; recorders can ignore unknown response fields).

Breaking changes (renaming a field, removing a kind, changing a
required field's type) trigger a v2.0 bump and the daemon will speak
both v1 and v2 for a minimum 12-month deprecation window per
`docs/DAEMON_AS_SERVICE.md` §"Versioning".
