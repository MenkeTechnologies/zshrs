# zshrs AOT — design doc

**Status:** Draft. No code yet. This doc decides the architectural calls before implementation begins.

**Companion docs:** [`DESIGN_GOALS.md`](./DESIGN_GOALS.md) (project bar + endgame constraints), [`ROADMAP.md`](./ROADMAP.md) (phase plan), [`RFC.md`](./RFC.md) (system-shell pitch).

---

## [0x00] Goal

The killer capability: **AOP + nanosecond profiling baked into native binaries for shell scripts (zshrs) and Perl scripts (stryke `--compat`).** Two legacy ecosystems get the same AOT + advice-weaving + profiling stack via the shared fusevm codegen pass. No prior art on either side.

`zshrs build script.zsh -o script` produces a static native binary that:

- Runs on a target machine with **zero zshrs install**, **zero shell install**, **zero runtime dependency** (POSIX-compliant kernel + libc on Linux is sufficient; static-musl removes even libc).
- Contains **actual machine code** for the script's logic — no parse, no compile, no bytecode interpretation, no JIT warmup at launch.
- Behaves identically to running the source script under `zshrs` — same builtins, same expansion semantics, same `eval`/dynamic-dispatch support.

Workflow: `zshrs build script.zsh && scp script user@server:/usr/local/bin/ && ssh user@server script`. No `apt install`, no `brew install`, no version management on the target.

---

## [0x01] World-first claim

Every existing shell ships scripts as source. The runtime (bash, zsh, dash, fish, nu, elvish) must be installed on the target box. Even shells with internal JIT still parse and compile at every invocation.

**No shell has ever shipped an "AOT-compiled-to-machine-code" deployment artifact for scripts.** zshrs is the first.

This satisfies both legs of the [`DESIGN_GOALS.md`](./DESIGN_GOALS.md) project bar:

| Leg | Status |
|-----|--------|
| World's first capability | Yes — no prior art across any shell tradition |
| World's fastest in category | Yes — native machine code, zero parse/compile/interp on launch, parity with Go/Rust binaries |

---

## [0x02] Why now: bytecode is one rung from AOT

The fusevm `Chunk` is the pivot point. Three deployment rungs use the same Chunk and differ only in payload format:

| Rung | Payload | Launch cost | Status |
|------|---------|-------------|--------|
| 1 — source trailer | UTF-8 source appended to binary copy | parse + compile + JIT (~1-2 ms) | What stryke ships today |
| 2 — bytecode trailer | bincode `Chunk` appended to binary copy | JIT or interp only (~µs) | Available to zshrs because Chunks are serde-ready (`BUILTIN_REGISTER_COMPILED_FN` already round-trips them via base64-bincode) |
| 3 — native object | Cranelift-emitted `.o` linked into a static binary | Zero codegen at launch | Target |

Stryke chose rung 1 because its `Arc<HeapObject>` runtime values aren't serde-ready. zshrs has no such constraint, so rungs 1 and 2 are stepping stones we don't need. **Ship rung 3 directly.**

The fusevm side already has `cranelift-jit` for runtime codegen. Adding `cranelift-object` is a parallel output sink on the same IR pass — the same IR that JIT consumes is what AOT consumes.

---

## [0x03] Pipeline

```
script.zsh
  │
  ├─ ZshLexer  ─────────────────────────────┐
  ├─ ZshParser ─────────────────────────────│  (existing)
  ├─ ZshCompiler → fusevm::Chunk ───────────┘
  │
  ├─ cranelift-object → script.o  ─── (NEW: AOT codegen pass)
  │
  ├─ ld script.o + libzshrs_runtime.a  ─── (NEW: link step)
  │
  └─ script  (static native binary)
```

The new components are the **codegen pass** (Chunk → `.o`) and the **runtime stub library** (`libzshrs_runtime.a`).

---

## [0x04] Codegen pass

**Input:** `fusevm::Chunk` (the same artifact JIT consumes today).

**Output:** Object file (`.o` on Linux, `.o`/Mach-O on macOS) containing:

- One symbol per compiled function.
- One `_zshrs_main` entry symbol per script (or per-script in multi-script bundles).
- Calls to runtime ABI symbols (`zshrs_v1_builtin_*`, `zshrs_v1_executor_*`) that ld resolves at link time.
- Relocations + DWARF for debugging (optional v1, recommended).

**Architecture:** Cranelift handles target triples natively. No LLVM, no host C toolchain. `cranelift-object` is the standard backend for AOT use.

**Where the pass lives:** Two options.

- **(a) Upstream in fusevm.** Correct long-term — stryke shares fusevm and benefits automatically. Requires fusevm version bump (currently 0.10.1, crates.io dep, not local-path).
- **(b) zshrs-side wrapper.** Faster v1 — call into fusevm's existing IR-emit code from a zshrs-specific cranelift-object module. Refactor upstream later.

**Decision: (a).** Reasoning: stryke is priority #2 per [`CLAUDE.md`](../../.claude/CLAUDE.md). Building the codegen as a fusevm module aligns both projects on a single pipeline. The version bump is required regardless once we add cranelift-object as a fusevm dep.

---

## [0x05] Runtime stub: `libzshrs_runtime.a`

A `staticlib` crate (`crate-type = ["staticlib"]`) that exposes every zshrs runtime primitive as a C-ABI symbol. The compiled `.o` calls into it; ld resolves at link time; the result is a single self-contained static binary.

**What's in the stub:**

- All ~317 `BUILTIN_*` handlers (echo, cd, set, typeset, read, print, …).
- `ShellExecutor` state (variables, arrays, assoc_arrays, options, jobs, traps, …) accessible via `zshrs_v1_executor_*` accessors.
- Fork/exec dispatcher (`host_exec_external`).
- File descriptor + redirect machinery.
- Pattern matching, glob, parameter expansion, history, completion (interactive paths can be excluded for non-interactive AOT binaries — TBD).
- **Embedded parser + compiler + VM** for `eval`, `${(e)…}`, `$(varname args)`, dynamic command names. These are cold-paths but present so AOT mode is a strict superset of interp mode (no compat-floor regression).

**Size estimate:** 5-10 MB static.a. Per-binary overhead. Acceptable — Go binaries are similar.

**ABI versioning:** Every symbol prefixed with `zshrs_vN_` where N is the runtime ABI version. Multiple versions can coexist in the same `.a`. Old binaries built against `zshrs_v1_*` keep running when zshrs ships `zshrs_v2_*` — the v1 symbols stay in the runtime forever per [`DESIGN_GOALS.md`](./DESIGN_GOALS.md) "compat-floor regressions are catastrophic." This is the load-bearing decision for the 30-year horizon.

---

## [0x06] CLI surface

```
zshrs build script.zsh                 # default: ./script (current platform)
zshrs build script.zsh -o myapp        # specify output name
zshrs build a.zsh b.zsh c.zsh -o app   # multi-script bundle, dispatched by argv[0] or argv[1]
zshrs build .                          # walk cwd, pack all *.zsh, entry = main.zsh
zshrs build --target linux-x86_64 ...  # cross-arch
zshrs build --musl ...                 # static-link with musl (Linux only)
zshrs build --strip ...                # strip symbols (smaller binary)
zshrs build --debug ...                # keep DWARF for debugging
```

**Multi-script dispatch.** Two patterns supported:

- **busybox-style.** Symlink target binary as `./app`, `./app-lint`, `./app-test` — argv[0] picks the entry symbol. Matches "many scripts in one binary" framing.
- **subcommand-style.** `./app lint`, `./app test` — argv[1] picks the entry. Simpler for users.

argv[0] takes precedence (basename != binary name → dispatch by basename); fallback to argv[1].

**`zshrs build .` semantics.** Walk `pwd` recursively (configurable depth), collect all `*.zsh`, identify entry as:

1. `main.zsh` if present
2. `__main__.zsh` if present
3. Single non-lib `.zsh` (heuristic: top-level files, not `lib/`)
4. Error: ambiguous, must specify `--entry`

Library files (`lib/foo.zsh`, `lib/bar.zsh`) get packed and made resolvable via embedded VFS so `source ./lib/foo.zsh` from inside the bundle works.

---

## [0x07] Cross-arch deployment

**Daily target:** Mac aarch64 → Linux x86_64 SFTP server. Plus Linux aarch64 servers. Future RISC-V.

**Cranelift handles target triples natively** (no LLVM, no cross-toolchain). The constraint is `libzshrs_runtime-{target}.a` must exist for each target.

**Strategy: pre-shipped target stubs.**

- `cargo install zshrs` ships with `libzshrs_runtime-{x86_64-linux-musl, x86_64-linux-gnu, aarch64-linux-musl, aarch64-linux-gnu, aarch64-darwin}.a` blobs in `~/.cache/zshrs/runtime/`.
- `zshrs build --target X` picks the matching stub.
- ~50-100 MB additional install size. One-shot install does it.
- CI cross-builds (`cross` or `cargo-zigbuild`) populate the stubs at release time.

**v1 fallback.** If pre-shipped stub for target is missing, error with `zshrs build --build-runtime <target>` to compile the stub on demand (requires Rust toolchain). v1 ships pre-built stubs for the daily-driver matrix; v1.1 adds on-demand.

---

## [0x08] eval & dynamic dispatch

`eval`, `${(e)…}`, `$(varname args)`, dynamic command names like `$cmd arg1 arg2` all need a parser+compiler+VM at runtime. AOT can't pre-compile these — by definition the input is unknown until runtime.

**Options:**

| Option | Pros | Cons | Verdict |
|--------|------|------|---------|
| Full embed (parser + compiler + VM in runtime stub) | Strict superset of interp mode, zero compat regression | Stub size +2-3 MB | Yes |
| AOT-strict mode (reject scripts using eval) | Smallest possible binary | Breaks zsh compat-floor | No |

**Decision:** Full embed. Per [`DESIGN_GOALS.md`](./DESIGN_GOALS.md) "Compat with his existing world is sacred." The cold-path size cost is acceptable; breaking eval is not.

---

## [0x09] Stryke alignment

Stryke shares fusevm. The codegen pass added in [§4](#0x04-codegen-pass) is upstreamed into fusevm, so stryke gets it free. Stryke's `aot.rs` (currently rung-1 source-trailer) can be deprecated in favor of the same cranelift-object pipeline once stryke's `Arc<HeapObject>` literals are made serde-ready.

**Shared pipeline:**

```
Chunk → cranelift-object → .o
  ├─ linked with libzshrs_runtime.a → zshrs binaries
  └─ linked with libstryke_runtime.a → stryke binaries
```

The runtime stubs differ (zshrs has zsh builtins, stryke has stryke builtins) but the codegen pass is shared. Single point of investment, two product wins.

### Capability matrix

Stryke gets rung 3 free from the shared codegen pass. AOP and profiling are added to stryke in parallel with the zshrs AOT work — both projects ship the full stack. Stryke's `--compat` Perl mode means **Perl scripts also flow through the same pipeline** to native binary with AOP + profiling baked in.

| Input | Frontend | Capability matrix |
|-------|----------|-------------------|
| `*.zsh` | zshrs parser | AOT + AOP weaving + profile bake-in |
| `*.stk` | stryke parser | AOT + AOP weaving + profile bake-in (parallel runtime) |
| `*.pl` | stryke `--compat` Perl parser | AOT + AOP weaving + profile bake-in (via stryke pipeline) |

| Capability | zshrs | stryke |
|------------|-------|--------|
| Cranelift-object native AOT | Has | Gets free via shared pass |
| Compile-time AOP weaving | Has runtime; gets bake-in | New: add `intercepts: Vec<Intercept>` runtime, mirror zshrs model, share codegen weaver |
| Nanosecond profiling bake-in | Has runtime; gets bake-in | Existing `profiler.rs` extended with same prologue/epilogue codegen |

Stryke advice syntax + `intercept_proceed` semantics: TBD by the stryke maintainer at implementation time. Engineering hooks above are sufficient for the AOT-side spec.

---

## [0x0A] Versioning & ABI durability

Per [`DESIGN_GOALS.md`](./DESIGN_GOALS.md): "Bytecode and SQLite formats must be versioned and migration-safe." Same applies to the AOT artifact.

**Three version axes:**

1. **Runtime ABI version.** `zshrs_v1_*` symbols stay forever. New runtime features add `zshrs_v2_*` alongside. Multiple ABIs coexist in the runtime `.a`. Old binaries always link against the version they were built with.
2. **fusevm IR version.** Recorded in the `.o` for diagnostics; not needed at link time (the IR is already lowered to native code by then).
3. **zsh feature flags.** Recorded in a `.note.zshrs` section in the binary so `file ./script` / `zshrs introspect ./script` can report what features were used. Helps debug "why does this binary need posix vs zsh-extension mode."

**Acceptance criterion:** A binary built today by `zshrs build` must keep running on the same kernel + libc 30 years from now (`DESIGN_GOALS.md` 30-year horizon). Achieved by: pure-Rust runtime, no LLVM dep, static link, libc syscalls only, ABI symbols never removed.

---

## [0x0B] Performance targets

| Metric | Target | Why |
|--------|--------|-----|
| Cold launch (first run, page-cache cold) | < 5 ms | Beats every existing shell on script startup. zsh + interp ~30-100 ms typical. |
| Warm launch (page-cache hot) | < 1 ms | Native binary, no parse, no compile. Same order as Go binaries. |
| Build time (single 1k-LOC script) | < 1 s | Parse + compile + cranelift-object + ld. Not the optimization target but should not surprise. |
| Binary size (stripped, hello-world) | < 8 MB static | Acceptable per Go-binary baseline. `--musl --strip` on Linux. |
| Binary size (large script, 50k-LOC zpwr-equivalent) | < 12 MB | Linear in script size after the static stub. |

These are "first paint = full functionality" per [`DESIGN_GOALS.md`](./DESIGN_GOALS.md). No optimization tricks needed; if the binary takes longer than these, the architecture has failed and we don't ship lipstick.

---

## [0x0C] Out of scope (v1)

Explicitly **not** in v1:

- **Dynamic shared library output.** All v1 binaries are static. `.so` shipping is a separate axis.
- **Hot-reload of compiled binaries.** Build → ship → run. No incremental rebuild on the target.
- **Source debugger that maps machine PCs back to `.zsh` line numbers.** v1 emits DWARF tied to the Chunk's source spans; full source-level debugger is v2.
- **Profile-guided optimization.** Cranelift doesn't do PGO; if needed, that's an LLVM swap, separate axis.
- ~~**Embed the interactive shell**~~ — **REVERSED.** Per §0x14 unified-AOT pivot, the interactive daily-driver shell is also AOT-compiled. ZLE, completion, plugins, and runtime state all live in the same binary as the script-runner mode. There is no longer a "source-form runtime" — one product, one artifact, one binary.

---

## [0x0D] Open architectural calls

These are the load-bearing decisions to lock down before any code is written.

| # | Question | Default | Reasoning |
|---|----------|---------|-----------|
| 1 | Codegen pass: upstream in fusevm or zshrs-side wrapper? | **Upstream** | Stryke benefits free. Single pipeline. |
| 2 | Runtime ABI versioning scheme? | **`zshrs_vN_*` symbol prefix, all versions coexist** | Endgame durability — old binaries always link. |
| 3 | eval/dynamic-dispatch handling? | **Full embed in runtime stub** | Compat-floor preservation > 2-3 MB size cost. |
| 4 | Static-link strategy on Linux? | **musl by default, glibc opt-in** | True zero-deps deployment. |
| 5 | Cross-arch via pre-shipped stubs or on-demand build? | **Pre-shipped for v1, on-demand for v1.1** | Ship the deployment-story winner first. |
| 6 | Multi-script dispatch: argv[0] (busybox) or argv[1] (subcommand)? | **Both, argv[0] takes precedence** | Match user framing + ergonomics. |
| 7 | `zshrs build .` entry-point heuristic? | **`main.zsh` → `__main__.zsh` → single non-lib → error** | Predictable, configurable via `--entry`. |
| 8 | What goes in `~/.cache/zshrs/runtime/`? | **Pre-built stubs per target triple** | One-time install cost, zero per-build cost. |
| 9 | DWARF in v1 binaries? | **Yes, --strip removes** | Cheap to include, expensive to add later. |
| 10 | First implementation language for runtime stub? | **Rust (existing zshrs code as `staticlib`)** | Reuse 100% of current implementation. No rewrite. |

If any of these defaults is wrong, fix it here before code starts. Once code exists, changing #2 or #3 is a breaking change; the others are local refactors.

---

## [0x0E] Implementation phases

1. **Phase A — fusevm cranelift-object output.** Add `cranelift-object` as fusevm dep; implement Chunk → `.o` codegen alongside the existing JIT path. Ship as fusevm 0.11.
2. **Phase B — `libzshrs_runtime.a` stub.** Add `[lib] crate-type = ["staticlib"]` target to zshrs. Expose all BUILTIN_* + executor accessors as `zshrs_v1_*` C-ABI symbols. CI builds for {mac-aarch64, linux-x86_64-musl, linux-aarch64-musl}.
3. **Phase C — `zshrs build` CLI.** Wire up `cargo` subcommand → cranelift-object → ld → output binary. Single-script first.
4. **Phase D — Multi-script + bundle support.** v2 trailer format equivalent for native, busybox dispatch, `zshrs build .` walker.
5. **Phase E — Cross-arch.** Pre-shipped target stubs in install. `--target` flag.
6. **Phase F — eval embed verification.** Build full zpwr (172k LOC, 506+ subcommands) as one AOT binary. Run on clean Linux box with no zsh installed. Pass = ship.

Each phase has its own load-bearing tests added to [`tests/`](./tests/) per [`DESIGN_GOALS.md`](./DESIGN_GOALS.md) "96-test invariant is load-bearing." Phase F is the acceptance criterion for v1.

---

## [0x0F] Test plan

The AOT binary must pass every existing zshrs behavioral test that doesn't require interactivity. Specifically:

- **All 158 `no_tree_walker_dispatch` tests** run against an AOT-built `test_runner` binary, not source-mode zshrs.
- **All 392 corpus tests** (plugin compat, parameter expansion, etc.) run against AOT.
- **All 70 ztst tests** (zsh test suite ports) run against AOT.
- **New AOT-specific tests:**
  - Build and run the same script under interp and AOT, compare stdout/stderr/exit/side-effects.
  - Build hello-world, scp to clean Linux container, run.
  - Build zpwr's full plugin set as one binary, exercise 100+ entry points.
  - Cross-build mac→linux, run on Docker linux container.

**Acceptance:** AOT mode is byte-identical to interp mode for every test. If AOT diverges from interp on any test, AOT has a bug — interp is the spec.

---

## [0x10] Memory model — no GC, ever

**Hard rule: zshrs and stryke are GC-free at every tier.** No tracing GC, no compacting collector, no GC'd dependencies, no convenience GC layer. The user's framing — "fuck GC" — is a hard architectural constraint, not a preference.

### The model

- **Owned values** (`String`, `Vec`, `HashMap`) — dropped deterministically at scope exit.
- **Refcount for cross-scope shared state** (`Arc<str>`, `Arc<HeapObject>` in stryke) — predictable drop on last reference release. No tracing.
- **Per-call arena allocation** for fusevm chunk locals — bumped at chunk entry, freed in O(1) at exit.
- **Closed-world AOT** lets escape analysis prove which allocations can stay on the stack vs need refcount; Rust's allocator already handles this without a GC layer.

### Why no GC

Latency predictability is load-bearing for AOP intercepts on hot paths. A shell where `intercept around git { … }` could trigger a GC pause every 10000th invocation isn't shippable for production scripts. Closed-world AOT + Rust ownership = provable maximum heap size, no stop-the-world events, real-time-suitable.

This property is **inherited for free from the implementation language**, not built. Rust's ownership model is the no-GC layer. There is no work to do here — only work to AVOID (don't import GC'd dependencies, don't introduce GC for niche concerns like cycle collection, don't propose a "convenience" GC layer).

### World-first verification

| Runtime | Memory model |
|---------|--------------|
| bash | libc malloc/free + custom arena |
| zsh (C) | libc malloc/free + refcount on shared strings |
| dash | libc malloc/free |
| fish (C++) | smart pointers (refcount) |
| nu (Rust) | Rust Arc — but not designed for the no-GC pitch |
| Perl 5 | refcount + arena |
| Raku (MoarVM) | full tracing GC |
| **zshrs / stryke** | **Rust ownership + Arc; no tracing, no arena GC, no managed runtime** |

**zshrs is the first shell where the runtime never traces, never compacts, never stops the world.** Every prior shell mixes manual + arena + refcount + (in Raku's case) full tracing GC. Inheriting GC-freedom from the implementation language is a deliberate architectural choice with downstream consequences:

- Memory ceilings are statically provable (closed-world + Rust ownership).
- Hot reload swaps function chunks without GC barriers — old chunk drops cleanly via Drop, new chunk's arenas/Arcs work identically.
- AOP advice on hot paths has no pause-spike risk.
- Real-time-suitable distribution: shell scripts running in latency-critical pipelines (audio, network gateways, robotics) become viable.

### Implications for dependency policy

Every new crate added to zshrs or stryke must be vetted against this rule. Reject crates that:

- Bind to Boehm GC (`bdwgc-sys`, etc.).
- Embed managed runtimes (V8, JVM, .NET).
- Use tracing internally (rare in Rust ecosystem; happens in some FFI-heavy crates).

Convenience GC layers for cycle collection, leak prevention, etc. are also rejected — handle cycles via static analysis or explicit weak refs (`Weak<T>`), not by introducing a tracer.

### What this rules out

- **No `gc` crate.** No future "let's add a GC because it's convenient for X."
- **No tracing for cycle collection.** Static cycle proof or Weak refs only.
- **No managed runtimes embedded.** `eval` embed is fusevm interp + parser + compiler, all Rust ownership.
- **No "convenience" arena GC.** Arenas are explicit + scope-bounded, not a generational tracer.

If a future architectural proposal demands GC, the proposal is wrong by construction — the implementation language already gives us deterministic memory, and the design constraints are downstream of that.

---

## [0x11] Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Cranelift bugs on rare opcodes | Existing JIT path uses same Cranelift — bugs surface in interp testing first. AOT inherits the same coverage. |
| Stub size grows past 10 MB | Conditional compilation: exclude interactive paths (ZLE, completion) from non-interactive AOT stub. Already excluded above. |
| Runtime ABI churn breaks old binaries | Versioned symbols (call #2). Old symbols never removed. Tested via "build with v1, link against v2 runtime, run" CI gate. |
| eval embed too large | Embed only what's needed for runtime parse — drop syntax-error pretty-printing, debug helpers. Accept 2-3 MB ceiling. |
| Cross-compile toolchain bit-rot | Pre-shipped stubs ARE the cross-compile output. No live cross-compile dep at user `build` time. |
| Mac codesigning breaks AOT binaries | Sign at `cargo install` time on Mac, document `codesign -s -` for users distributing builds. |

---

## [0x12] AOP + profiling bake-in

**World-first stack:** AOT-compiled shell script + AOP intercepts compiled into the binary + nanosecond-accuracy profiling emitted as native code prologue/epilogue. No shell has shipped any one of these. zshrs ships all three in the same artifact.

### AOP at compile time

zshrs already has runtime AOP via the `intercepts: Vec<Intercept>` field on `ShellExecutor`. Today they're loaded from `.zshrc` and dispatched at runtime. For AOT, intercepts move to **build time**:

```
zshrs build script.zsh --aop intercepts.zsh -o script
```

The `intercepts.zsh` file declares advice (around/before/after/error) by glob pattern (`git *`, `db_*`, `*`). The codegen pass:

1. Compiles intercepts to fusevm Chunks alongside the script.
2. At every call site that matches an intercept's glob, emits a direct call into the advice chunk instead of dispatching through `intercepts: Vec<Intercept>` at runtime.
3. Inlines the advice where it's small enough — the emitted machine code becomes `[before-advice] → [original call] → [after-advice]` with zero dispatch overhead.

**World-first claim:** No shell supports AOP at all (zshrs is first runtime). No shell supports compile-time AOP weaving (zshrs AOT is first). This sits one rung above what runtime AOP can do — zero indirect-dispatch cost because the weaving happens at build time.

**Trade-off:** Once baked in, intercepts can't be reconfigured without rebuilding. For dev iteration, source-mode zshrs runs the same intercepts with runtime cost. Production binaries are immutable per `DESIGN_GOALS.md` "build → ship → run" model.

### Nanosecond profiling at codegen time

```
zshrs build script.zsh --profile -o script           # all functions
zshrs build script.zsh --profile=hot -o script       # only functions tagged @hot
zshrs build script.zsh --profile=glob:db_* -o script # glob-selected
```

Codegen pass emits prologue + epilogue around each profiled function:

```
function_entry:
  rdtsc          ; x86: serializing variant rdtscp; aarch64: mrs CNTVCT_EL0
  mov [tls_buf+offset_start], rax
  ; ... function body ...
function_exit:
  rdtscp
  sub rax, [tls_buf+offset_start]
  add [tls_buf+cumulative], rax
  inc qword [tls_buf+call_count]
  ret
```

**Why nanosecond accuracy:** rdtscp on modern x86 is ~10ns to read with serialization. aarch64 `mrs CNTVCT_EL0` is ~5ns. No syscall, no `gettimeofday`, no microsecond clock. Per-function timing is sub-µs accurate.

**Per-thread storage:** Profile counters live in TLS to avoid contention. The runtime stub provides `zshrs_v1_profile_alloc` / `zshrs_v1_profile_flush`.

**Output formats** (selectable via `--profile-format=`):

- `text` — flame-graph-style ranked list on exit (default)
- `pprof` — Go-style protobuf for tooling integration
- `chrome` — chrome://tracing JSON
- `inferno` — flamegraph.pl input
- `live` — bind to `127.0.0.1:9099` and serve runtime stats over HTTP

Flush triggers: process exit (default), `SIGUSR1` (for long-running scripts), or programmatic `zprof -d` from inside the script.

**Compile-time selectivity** matters: profiling all 50k functions in a zpwr-equivalent binary adds ~10-15% binary size and ~5ns per call-edge. `--profile=hot` (annotation-driven) or `--profile=glob:` (pattern) keeps the cost localized.

### CLI surface additions

```
zshrs build --aop FILE script.zsh -o script              # bake in AOP intercepts
zshrs build --profile[=SELECTOR] script.zsh -o script    # bake in profiling
zshrs build --profile-format=FMT script.zsh -o script    # output format
zshrs build --aop FILE --profile script.zsh -o script    # both stacks
```

Both flags produce additional `.note.zshrs-{aop,profile}` ELF sections so `zshrs introspect script` reports what was woven in.

### Test plan additions

- **AOP weaving correctness.** Build a script with intercepts, run it, compare interp-mode AOP output vs AOT-mode AOP output — must be byte-identical.
- **Profiling overhead.** Measure cost-of-instrumentation on a hot loop. Acceptance: < 10ns per call-edge on Mac aarch64, < 15ns on Linux x86_64.
- **Format round-trip.** `zshrs build --profile=text script` must produce text output identical to `zprof` running on source-mode.
- **Live endpoint.** Build with `--profile-format=live`, hit `localhost:9099/stats`, verify JSON shape matches the live spec.

### Open architectural calls (additions to §0x0D)

| # | Question | Default | Reasoning |
|---|----------|---------|-----------|
| 11 | AOP intercepts: file-based or in-script `@aop` decorators? | **Both, file-based primary** | File matches existing zshrs runtime AOP. Decorators are stryke-territory. |
| 12 | Profiling default `--profile` behavior? | **All compiled functions** | Catches everything; `--profile=hot` opts down. |
| 13 | TLS profile buffer size? | **64 KiB per thread** | Enough for 1000s of distinct functions. Resizable on overflow. |
| 14 | Live endpoint port? | **9099 default, `--profile-port=N` to override** | Clear of common ports, easy to remember. |
| 15 | Profile format default? | **`text` (zprof-compatible)** | Matches existing zprof output, no learning curve. |

---

## [0x13] Unified AOT — the binary IS the database

The architectural pivot: zshrs is **one product** — a single AOT-compiled binary that is your daily shell, your deploy artifact, your plugin host, your completion engine, and your state persistence layer. Everything that today lives in adjacent files (`.zshrc`, `.zcompdump`, `~/.cache/zshrs/*.db`, `~/.zinit/plugins/`, fpath scripts) collapses into one binary.

### What moves into the binary

| State | Today | Unified AOT |
|-------|-------|-------------|
| `.zshrc` + plugin source trees | Source replay each launch | Native code in `.text` |
| zinit / oh-my-zsh / syntax-highlighting / autosuggestions | Async source load + cache replay | Native machine code, woven into ZLE redraw / hooks |
| fpath autoloads (16,806+ files in zsh-more-completions) | Lazy load on first call | All native at build time |
| Completion functions (`_git`, `_kubectl`, etc.) | Source autoload + parse | Native + perfect-hash dispatch table |
| `.zcompdump` | Generated by compinit | Baked at build, gone |
| `compsys.db` (SQLite) | Runtime B-tree queries on Tab | Gone — completion dispatch is native |
| `plugin_cache.db` (SQLite) | Bytecode cache + delta replay | Gone — code is already native |
| zstyle config | Vec<ZStyle> linear scan per query | `&'static [(pat, val)]` — direct branch |
| Bindkey table | HashMap built each launch | `&'static [(keycode, action)]` jump table |
| Named directories (`hash -d`) | HashMap | `.rodata` table |
| AOP intercepts | Vec<Intercept> dispatched at runtime | Woven into `.text`; runtime additions in writable section |
| User-defined runtime vars / state | HashMap | Persistent writable section (image-style) |

### What stays external

- **`history.db`** — primary mutable data, frequently appended, B-tree-indexed. Belongs as SQLite. History is your data, not the shell's identity.
- **Working files** the user is editing (source code, docs).
- **Runtime IPC** (sockets, FIFOs).

### Image-as-binary persistence (Smalltalk model)

Prior art that proves it works: Pharo/Squeak Smalltalk images (30+ years), SBCL `save-lisp-and-die`, HyperCard stacks. Each shipped one piece. Combining all of them and applying to shell+Perl is structurally world-first.

**Layout:** read-only `.text` + `.rodata` (signable, immutable) + writable section (or sibling `<binary>.image` file on macOS to preserve codesigning) for runtime mutable state.

**Save:** graceful shutdown flushes writable section; `zshrs save` for explicit checkpoint; SIGKILL preserves last graceful flush.

**Load:** auto-deserialize on launch; schema mismatch on rebuild forward-migrates or rejects with clear error.

**Backup:** `cp ~/bin/zshrs ~/bin/zshrs.bak` is atomic backup of code + state. Git-track if you want history of config evolution.

### Engineering realities

1. **macOS codesigning** forbids mutable binaries. Resolution: split read-only artifact + sibling `<binary>.image` for writable state. Two files cosmetically; one unit semantically.
2. **Single-writer enforcement** via `flock` on the image. First shell wins; subsequent shells are read-only on the image.
3. **Schema migration** for the writable section: stable serialization (CBOR or postcard) + per-version readers in the runtime stub. Per `DESIGN_GOALS.md` "bytecode and SQLite formats must be versioned and migration-safe."
4. **Image growth** mitigated by `zshrs vacuum` for compaction + configurable retention for profile data.
5. **Atomic update**: `zshrs build` writes temp file + atomic rename. Currently-running shell stays alive on old code; new shells get the new build.

### What dies

The pivot kills entire optimization layers that exist today only to paper over the source-load model. Code paths that become archaeology:

- `plugin_cache.db` — bytecode cache for autoload
- `compsys.db` — completion SQLite cache
- `.zcompdump` — compinit-generated completion db
- Plugin source delta cache — zinit-style side-effect replay
- Background `compinit` worker — async fpath scanner
- `.zwc` (zsh wordcode) reading paths
- `zinit turbo` equivalent — async plugin defer
- `p10k instant prompt` equivalent — fake-prompt-while-loading hack
- `compaudit` — security check on completion files
- `autoload -U` — becomes a no-op (or build-time tag)

Estimated LOC eliminated from zshrs: 5,000-10,000 across `plugin_cache.rs`, compsys cache code, autoload machinery, .zwc readers, async load workers. Net simplification.

### Daily workflow (Rust-source-equivalent)

```
$ vim ~/.config/zshrs/zshrc     # edit shell config
$ zshrs build                    # ~500ms incremental rebuild (target)
$ exec ~/bin/zshrs               # atomic replacement
```

The investment shifts from "make runtime cache faster" (dead direction) to "make incremental rebuild faster" (now load-bearing). Cargo-incremental-equivalent for fusevm IR is the v2 perf project.

### Acceptance criterion (revised, supersedes §0x0B)

The unified-AOT pivot has succeeded when:
- `zshrs build` of full daily-driver config (zshrc + zinit plugin set + zsh-more-completions corpus) produces one binary in <30 seconds clean / <500ms incremental.
- That binary launches in <5ms cold, <1ms warm.
- Tab completion latency: sub-µs end-to-end.
- No SQLite cache files referenced at runtime (only `history.db` remains).
- Same binary scps to a Linux server with zero zshrs install and runs as deployment artifact.
- Daily-driver mode and deploy mode are byte-identical artifacts. **One product, two use cases.**

### World-first verification (revised)

| Capability | Prior art |
|------------|-----------|
| AOT-compiled shell + plugins + completions in one binary | None — every shell loads plugins/completions as source |
| Single-file shell artifact + state + config + plugin host | Pharo (Smalltalk only, interp), SBCL (Lisp only), HyperCard (dead since 2000) — none for shell |
| Daily-driver shell == deploy artifact (byte-identical) | None — every shell has source-mode runtime separate from deployment |
| SQLite cache layers entirely eliminated from shell hot path | None — every modern shell has at least one cache layer |

**Five world-firsts stack on this pivot alone, on top of the AOT + AOP + ns-profiling + no-GC + cross-language stack from §0x00–§0x12.**

---

## [0x14] Decision log

This doc is the source of truth for AOT design. Changes require an entry below.

| Date | Decision | Author |
|------|----------|--------|
| 2026-04-28 | Doc created. Defaults in §0x0D adopted as initial spec pending review. | initial draft |
| 2026-04-28 | §0x11 added — AOP + nanosecond profiling bake-in. Cumulative world-first stack: AOT-compiled shell + compile-time AOP weaving + native-emitted nanosecond profiling. | initial draft |
| 2026-04-28 | §0x09 sharpened — verified stryke has no AOP runtime today. Plan revised: stryke gets parallel `intercepts: Vec<Intercept>` runtime added alongside the AOT work, both projects ship the full AOT + AOP + profile stack. | initial draft |
| 2026-04-28 | §0x10 Memory model added — no GC, ever. Hard rule. Inherited from Rust ownership for free; verified against bash/zsh/dash/fish/nu/Perl5/Raku as world-first GC-free shell. Closed-world AOT lets escape analysis prove allocations stay scope-local. Locked in feedback memory: any future proposal that introduces GC is wrong by construction. | initial draft |
| 2026-04-28 | §0x13 Unified AOT pivot — collapsed source-mode shell + AOT deploy into one product. Daily shell is the AOT binary; plugins/compsys/zstyle/bindkeys/intercepts all bake in at build time; SQLite caches die (`plugin_cache.db`, `compsys.db`, `.zcompdump`); only `history.db` remains external. Image-as-binary persistence model (Pharo/Smalltalk lineage). Acceptance criterion supersedes §0x0B perf targets — one binary, <5ms cold launch, sub-µs Tab, byte-identical between daily-driver and deploy use cases. Estimated 5-10k LOC eliminated. Five additional world-firsts stack on this pivot. §0x0C "interactive shell out of scope" bullet reversed. Memory rule "AOT-deploy vs JIT-source-mode are separate products" superseded — they are now one product. | initial draft |

---

## [0x15] Next step

Review and lock-in §0x0D defaults. If all green, Phase A (fusevm cranelift-object output) starts with a version bump to fusevm 0.11 and a new `aot` module added to that crate. If any default is wrong, edit this doc first; code follows the doc, never the other way around.
