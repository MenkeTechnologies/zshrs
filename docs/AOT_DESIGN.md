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
  ├─ lex module  ─────────────────────────────┐
  ├─ parse.rs free fns ────────────────────────│  (existing)
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

- **(a) Upstream in fusevm.** Correct long-term — stryke shares fusevm and benefits automatically. Requires fusevm version bump (workspace pins **0.17.0** on crates.io today; see root `Cargo.toml`).
- **(b) zshrs-side wrapper.** Faster v1 — call into fusevm's existing IR-emit code from a zshrs-specific cranelift-object module. Refactor upstream later.

**Decision: (a).** Reasoning: stryke is priority #2 in the project order. Building the codegen as a fusevm module aligns both projects on a single pipeline. The version bump is required regardless once we add cranelift-object as a fusevm dep.

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

- `cargo install zshrs` ships with `libzshrs_runtime-{x86_64-linux-musl, x86_64-linux-gnu, aarch64-linux-musl, aarch64-linux-gnu, aarch64-darwin}.a` blobs in `~/.zshrs/runtime/`.
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
| 8 | What goes in `~/.zshrs/runtime/`? | **Pre-built stubs per target triple** | One-time install cost, zero per-build cost. |
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

- **All 160 `no_tree_walker_dispatch` tests** run against an AOT-built `test_runner` binary, not source-mode zshrs.
- **All 393 corpus tests** (plugin compat, parameter expansion, etc.) run against AOT.
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

## [0x13] Daily-driver cache architecture — rkyv image + worker-hydrated catalog

**Scope:** Sections §0x00–§0x12 cover script-AOT — `zshrs build script.zsh -o script` producing a static deployment binary. That capability stands. This section addresses a **different** question: how the daily-driver shell stores plugin / compsys / autoload bytecode for fast lookup at every prompt.

The earlier "binary IS the database" framing — bake every plugin and every completion into the daily-driver binary's `.text` segment — was wrong on memory. At zpwr-scale (172k LOC + 27,387 zsh-more-completions + 100-200 zinit plugins ≈ 200-400 MB of bytecode), `.text` pre-faulting puts 50-100 MB resident on every shell launch and the kernel cannot evict `PROT_EXEC` pages. The working set must scale with **what's actually called**, not **what's installed**.

### Decision

The daily-driver bytecode lives in a **sharded layout** under a single directory `~/.zshrs/`. One image file per source root, plus a top-level index, plus the SQLite catalog, history, and tracing log — all colocated. No XDG split, no second persistent-data directory. User curates via filesystem ops (`cd ~/.zshrs && rm <whatever>`); no `--clean-cache` subcommand — `rm` is the API.

```
~/.zshrs/
├── index.rkyv                              ← top-level fq_name → (shard_id, generation, byte_offset)
├── images/
│   ├── {hash8}-system.rkyv                 ← /usr/share/zsh, dist completions
│   ├── {hash8}-completions-corpus.rkyv     ← zsh-more-completions
│   ├── {hash8}-zpwr.rkyv                   ← his framework
│   ├── {hash8}-plugin-{name}.rkyv …        ← one per zinit plugin
│   └── {name}.rkyv.lock                    ← per-shard advisory flock
├── catalog.db                              ← daemon-hydrated mirror: entries / hooks /
│                                             plugins / entry_stats — dbview target
├── history.db                              ← shell history
├── zshrs.log                               ← tracing output (10 MB cap, rotation)
├── daemon.sock                             ← Unix domain socket for client → daemon IPC
└── daemon.pid                              ← singleton lock + daemon process ID
```

**Why sharded, not monolithic:** a single `image.rkyv` would have to be rebuilt every time any source file changed. With user-frequency commits to zpwr + plugin-update churn (`zinit update foo`, `git pull` in any plugin tree), this loops in seconds-to-minutes of full-corpus rebuild that's >99% redundant. Sharding the image by source root reduces the rebuild blast radius to "just the touched root" — typically 100-500ms vs 30s full.

The cache directory has bounded litter (one image per plugin, capped naturally at install count). That's an explicit trade — bounded litter for per-shard rebuild and parallelism. The user's iterate-on-zpwr loop is the load-bearing constraint that drove this trade.

**Why one directory, not split by durability:** previous draft proposed splitting persistent data (history.db, entry_stats) into `~/.local/share/zshrs/` so `rm -rf ~/.zshrs/` would be safe. User overruled — single dir wins on simplicity. Trade is explicit: `rm -rf ~/.zshrs/` nukes everything including history + accumulated stats. User responsibility, same as `rm -rf ~/.zsh_history` today. Surgical resets via per-file `rm` (see table below). Matches the global "user is root, no friction, no safety prompts" rule — `rm` is the user's tool, not zshrs's responsibility to wrap.

**Reset tiers via filesystem ops** (always available — `rm` is a valid path):

| Goal | Operation |
|------|-----------|
| Force one shard to rebuild | `rm ~/.zshrs/images/plugin-foo.rkyv` |
| Force all shards to rebuild | `rm -rf ~/.zshrs/images/` |
| Re-derive catalog (loses entry_stats — they live in catalog.db) | `rm ~/.zshrs/catalog.db` |
| Force re-link without re-shard | `rm ~/.zshrs/index.rkyv` |
| Full nuke including history + stats | `rm -rf ~/.zshrs/` |
| Truncate log | `rm ~/.zshrs/zshrs.log` (or `: > zshrs.log` for in-place) |

### Cache management builtins (`zcache <verb>`)

Raw `rm` is supported but doesn't coordinate with the running shell — mmap handles need releasing before unlink, and the catalog needs a re-hydrate after a clean. The `zcache` builtin family is the supervised version that handles both.

**Naming:** `zcache` follows the zsh `z*` convention (`zmv`, `zparseopts`, `zformat`, `zstyle`, `zprof`, `zstat`) without colliding with any upstream zsh `z*` builtin. `cache` was rejected as a name due to high collision risk with user PATH commands and zpwr subcommands.

**Sync vs async:** all write operations dispatch to the worker pool and return immediately (per the "nothing blocks the shell" invariant). Read-only operations stay synchronous since they're fast (mmap stat, SQLite SELECT). Pass `--wait` to any async verb to explicitly block until completion.

```
zcache                          # SYNC, read-only — info: shard sizes, entry counts,
                                # hot/cold breakdown, in-flight worker jobs
zcache info                     # same as bare `zcache`
zcache jobs                     # SYNC — list active worker jobs (compile, hydrate, vacuum)

zcache clean [--wait]           # ASYNC — regenerable only (preserves history + entry_stats)
zcache clean --all [--wait]     # ASYNC — everything (no prompt; user is root)
zcache clean shards [--wait]    # ASYNC — rm -rf images/ ; index marked stale
zcache clean shard <name> [--wait]   # ASYNC — rm one shard, mark its index entries stale
zcache clean catalog [--wait]   # ASYNC — rm catalog.db, preserve entry_stats via dump+reimport
zcache clean catalog --no-stats # ASYNC — rm catalog.db wholesale (loses entry_stats)
zcache clean index [--wait]     # ASYNC — rm index.rkyv, force re-link from existing shards
zcache clean stats              # SYNC — DELETE FROM entry_stats only (single SQL stmt, fast)
zcache clean log                # SYNC — truncate zshrs.log (single fs op, fast)

zcache rebuild [--wait]         # ASYNC — full corpus rebuild (worker pool fan-out)
zcache rebuild shard <name> [--wait]  # ASYNC — surgical
zcache rebuild --parallel N     # ASYNC — control worker pool fan-out for this rebuild

zcache verify                   # SYNC — integrity scan: shard hashes vs catalog header,
                                # PRAGMA integrity_check on catalog.db, report drift /
                                # corruption / orphaned .tmp files; suggested recovery cmds
zcache compact [--wait]         # ASYNC — SQLite VACUUM on catalog.db + history.db
```

Default behavior of every async verb: enqueue job(s) into the worker pool, print job ID(s) to stdout, return immediately. Subsequent `zcache jobs` shows progress; `tail -f ~/.zshrs/zshrs.log` shows worker output. `--wait` is the explicit opt-in for blocking semantics (useful in scripts that need ordering guarantees).

**Custom-builtin namespace convention:** all zshrs-introduced builtins use the `z` prefix to match zsh's existing `z*` family (`zmv`, `zparseopts`, etc.) but never overlap a name already used by upstream zsh. Anti-collision check is part of the build: if upstream zsh adds a `z*` builtin in a future release that we shadow, our build fails until we rename. Current zshrs `z*` additions: `zcache`. Future zshrs additions follow the same rule.

**Why builtins beat raw `rm` for routine maintenance:**

| Operation | Raw `rm` | `cache clean` builtin |
|-----------|----------|------------------------|
| Releases shell's mmap handles before unlink | no — unlinked-but-mapped file lingers in inode table until shell exits | yes — shell drops handles first |
| Triggers worker re-hydrate of catalog | no — catalog goes stale until next plugin event | yes — enqueues hydrate after clean |
| Preserves entry_stats across catalog reset | no — atomic file delete loses everything | yes — dumps stats, drops catalog, re-imports |
| Discoverable | needs the docs | `cache <Tab>` shows verbs |
| Logs to `zshrs.log` for audit | no | yes — `cache_op { verb, target, duration_ms }` |
| Works without `cd` to cache dir | needs path or cwd | works from anywhere |

`rm` remains a valid path for users who want to bypass coordination. The cache directory is just a directory.

**`cache info` output sketch:**

```
$ cache
Cache: ~/.zshrs/                           total: 612 MB
  index.rkyv                  1.2 MB    50,234 entries  (mmap: hot)
  images/                   247.0 MB       143 shards
    system.rkyv              12.0 MB     2,341 entries
    completions-corpus.rkyv  82.0 MB    27,387 entries
    zpwr.rkyv                45.0 MB       506 entries  (mtime 2m ago)
    plugin-zsh-syntax-h…      1.2 MB        18 entries
    plugin-…                   …
  catalog.db                 18.0 MB    50,234 entries / 5,492 hooks
                                       2,847 entry_stats rows (843 with calls)
  history.db                340.0 MB   478,231 commands
  zshrs.log                   4.2 MB    last hydrate: 18s ago (12ms)
```

Per the global "no startup banner / no progress to terminal" rule: `cache` builtins emit their result to stdout (this is explicit user-requested output, not informational chatter), but log the operation details to `zshrs.log` via `tracing::info!`. `cache clean --all` runs without confirmation prompt — user is root, friction is zero.

### Personality mode gating (one binary, three shells)

zshrs is **one codebase** that ships **three personality modes**, selected at startup. Cache machinery is one of many features gated by mode. The framing matters because it explains why cache (and async/await, and AOP intercepts, and stryke `@` prefix) all live behind runtime checks, not behind separate binaries.

| Mode | Trigger | Identity | Extensions | Cache layer |
|------|---------|----------|------------|-------------|
| **POSIX** | `--posix`, `emulate sh`, argv[0] basename `sh` / `dash` / `bash` | strict POSIX `/bin/sh` drop-in | none — POSIX-only | OFF |
| **Vanilla zsh** | argv[0] basename `zsh`, or `--zsh-compat` flag | byte-compatible mainline-zsh replacement | zsh extensions ON (params, expansions, autoloads, completions) | OFF (matches what mainline zsh users expect — no surprise daemons or cache dirs) |
| **Turbocharged zshrs** | argv[0] basename `zshrs`, or default when invoked by name | zshrs-native — superset of zsh | zsh extensions + zshrs extensions (`@` stryke, async/await, AOP intercepts, anti-fork builtins, parallel primitives) | ON — full sharded rkyv image + worker hydrate + `zcache <verb>` builtins |

**Why one binary, not three:** ship targets, build complexity, and code reuse all collapse. POSIX mode gets the same fast bytecode VM as turbocharged mode — what changes is the surface area of available features. The internal pipeline (`parse → compile → JIT/interp`) is shared; the gating is at feature-entry points.

**What gets gated in each mode:**

- **POSIX:** strip everything zsh-specific. No `(s:sep:)` flags, no `${arr[@]}` array splice, no `[[ ]]` (just `[ ]`), no globbing flags, no autoloads, no completion system, no aliases mid-line, no precmd/preexec hooks. And no cache layer — see below.
- **Vanilla zsh:** all zsh semantics, but no zshrs-extensions (no `@` stryke calls, no AOP weaving, no async/await, no anti-fork builtin overlays for `cat`/`head`/etc. since vanilla zsh would shell out). Cache layer also OFF — vanilla mode is "zshrs as a fast vanilla zsh," not "zshrs as a daily-driver upgrade."
- **Turbocharged zshrs:** everything on. This is the default invocation, the daily-driver experience, the workload the cache layer targets.

**Cache-layer specifics under POSIX / vanilla zsh:**

- No image lookup. No `index.rkyv` mmap.
- No catalog.db open. No `entry_stats` writes.
- No `~/.zshrs/` directory created or touched.
- No `zcache <verb>` builtins available — `zcache` resolves as a normal command name.
- Worker pool may still exist for other things (background command pipelines) but no cache jobs scheduled.

**Why this matters operationally:**

- `/bin/sh → zshrs` symlink in a fleet of containers: POSIX mode kicks in via argv[0]; no `~/.zshrs/` created in `/root`, no cache machinery running per process. Pure POSIX shell, just faster than dash.
- `/bin/zsh → zshrs` symlink for users who want "fast vanilla zsh" without committing to the full turbocharged stack: vanilla mode kicks in; full zsh feature set, no zshrs extensions or cache surprises.
- Daily driver (`exec zshrs`): turbocharged mode kicks in; full feature surface, cache layer active.

**Personality vs emulation scope:** the **personality mode** above (POSIX / vanilla / turbocharged) is set ONCE at process startup and is immutable for the lifetime of the process. It controls cache activation, builtin availability, worker pool behavior — anything that needs a stable answer from a long-lived subsystem.

This is **distinct** from zsh's `emulate -L sh` / `emulate -LR zsh` / `emulate -L ksh` — a per-function, per-scope **emulation switch** that controls only the parser/expander surface for the current scope. zinit and a lot of zsh plugins use `emulate -LR zsh` constantly to scope their language assumptions; that mechanism is fully supported and unchanged from mainline zsh. `emulate -L` does NOT tear down the cache, restart the worker pool, or change builtin availability. It only flips parser flags for that scope and restores them on scope exit.

So: turbocharged-zshrs sourcing a zinit plugin that `emulate -LR zsh`s itself works exactly as it does in mainline zsh. The two concepts are orthogonal:

| Concept | Lifetime | Controls | Mutable? |
|---------|----------|----------|----------|
| Personality mode | process | cache, fsnotify (none, removed), workers, builtin set | no — set at startup |
| Emulation scope | per-function via `emulate -L` | parser flags, expander flags | yes — standard zsh per-scope |

**Implementation rule:** every code path that's gated by personality mode checks the runtime mode flag at its entry point. Single check, cheap, impossible to drift. `emulate -L` operates on a separate parser-flags struct that's saved/restored on scope entry/exit per zsh semantics.

The non-negotiable contract: a binary identifying as `sh` / `dash` / `bash` MUST behave as a strict POSIX shell with zero zshrs surface. A binary identifying as `zsh` MUST behave as mainline zsh with zero zshrs cache surface. A binary identifying as `zshrs` MUST be the full turbocharged daily driver. These three are the marketed personalities and any cross-contamination is a regression.

### Hard invariant: nothing blocks the shell

The main shell thread NEVER blocks on compile work, image writes, hydration, or any other cache-management operation. **All rkyv blob building runs in the worker pool.** This is non-negotiable — first-prompt latency, prompt-firing cadence, and Tab response all stay sub-millisecond regardless of what plugin install / git pull / explicit `zcache rebuild` is happening in the background.

**Source code is the source of truth. Image is an opportunistic accelerator.** This is the architectural safety net that makes the non-blocking invariant possible. Every function in the user's daily-driver corpus has a complete source-interp execution path (the existing parser → bytecode-compile-on-demand → JIT/interp pipeline). The image cache is a fast lookup that may or may not hit; if it doesn't hit (or the blob is malformed, version-mismatched, mid-rewrite, or the file is missing entirely), the main thread silently falls through to the source-interp path and the user is none the wiser. Worker quietly catches up the cache so subsequent calls hit.

Implications:

- The image is an **opportunistic cache**, not the source of truth. Source files (zsh scripts in plugin dirs, fpath, zpwr) remain authoritative; deleting the entire `images/` directory mid-session does not break the shell — every subsequent call falls through to source interp until workers rebuild.
- Image lookup miss → main thread falls back to interp-from-source for that one function, AND enqueues a compile job for the worker pool. Function returns; user is unblocked. The image fills in over time.
- **Malformed shard / version mismatch / corruption** → treated identically to a miss. Main thread falls through to source interp; worker enqueues a re-shard job. Cache corruption is never user-visible beyond "this call took the slow path."
- Worker pool owns: all shard compile, all `images/*.rkyv` writes, `index.rkyv` rewrites, `catalog.db` hydration, `cache compact` SQLite VACUUM, `cache verify` hash checks.
- Cold-install / fresh-machine launch: image doesn't exist yet → first shell falls back to source interp for everything → user gets first prompt within a few hundred ms (interp-only path, slow but not blocked). User runs `zcache rebuild` when ready to warm the cache; subsequent shell launches are fully fast.

This decoupling — image as accelerator, source as ground truth — is what makes "nothing blocks the shell" achievable. The alternative (image is required for execution) would force the shell to wait on compile during cold start and would crash on any cache corruption. Neither is acceptable.

**zshrs-daemon: singleton compiler, N thin clients.** Per-process fsnotify is fatal at 60-shell scale (60 watchers thundering-herd on every edit). Per-process explicit-rebuild is friction. Solution: a **single daemon** owns all cache mutation; the N regular zshrs clients are thin readers that mmap the daemon's output and notify the daemon of runtime config changes via IPC. Best of both — auto-rebuild without 60× duplication, hands-off without losing user agency.

```
┌─────────────────────────────────────────────────────────────────┐
│  zshrs-daemon (singleton; spawned lazily by first client)       │
│                                                                  │
│  ┌──────────┐  ┌────────────┐  ┌──────────────────────────┐    │
│  │ fsnotify │→ │ work queue │→ │ cache worker pool (1-2 t)│    │
│  │ thread   │  │ (lock-free)│  │ compile / hydrate / vacuum│    │
│  └──────────┘  └────────────┘  └────────────┬─────────────┘    │
│       ↑              ↑                       ↓                   │
│  ┌──────────┐   ┌──────────┐         ┌──────────────────┐       │
│  │ accept() │   │ ticker   │         │ atomic writes to │       │
│  │ Unix sock│   │ (compact,│         │ ~/.zshrs/  │       │
│  │ thread   │   │  rotate) │         │ images/, index,  │       │
│  └────┬─────┘   └──────────┘         │ catalog.db       │       │
│       │                              └──────────────────┘       │
└───────┼─────────────────────────────────────────────────────────┘
        │ ~/.zshrs/daemon.sock
        │
   ┌────┴───┬────────┬────────┬──────────── … 60 ─────────┐
   ↓        ↓        ↓        ↓                            ↓
┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐                    ┌──────┐
│ zshrs│ │ zshrs│ │ zshrs│ │ zshrs│        …           │ zshrs│
│ #1   │ │ #2   │ │ #3   │ │ #4   │                    │ #60  │
│ mmap │ │ mmap │ │ mmap │ │ mmap │                    │ mmap │
│ only │ │ only │ │ only │ │ only │                    │ only │
└──────┘ └──────┘ └──────┘ └──────┘                    └──────┘
```

**Each client does zero compile work AND owns minimal resources.** User regularly runs **100+ zshrs inside tmux**. Per-client overhead must be near-zero or the workstation collapses on CPU/RAM.

**Strict client resource budget** (per-shell, beyond the bare interpreter):

| Resource | Per-client cost |
|----------|-----------------|
| mmap regions | 1 for `index.rkyv` (~1-2 MB VSZ) + LRU cache of actually-touched shards (typical <10) |
| File descriptors | 1 Unix socket to daemon (closed after each IPC; reopened on demand) |
| Background threads | **ZERO cache-related threads.** No fsnotify, no compile workers, no SQLite connection pool. |
| SQLite handles | **ZERO.** Clients do NOT open `catalog.db`. dbview queries go through daemon via IPC. Runtime stats batched in-memory, flushed to daemon every 30s. |
| Heap allocation for stats | Small (~KB) per-client buffer for accumulated `entry_stats` deltas before flush |
| RAM at idle | <5 MB per client beyond the existing zsh interpreter footprint |
| RAM under load | mmap'd shard regions are page-cache shared across all 100 clients (single physical-RAM cost, not 100×) |

**What's explicitly OFF in clients (zero cache-related polling, zero cache-related background loops):**

- No fsnotify watcher (daemon owns the only one)
- **No cache-related worker pool** in clients. No periodic tickers for cache flush, no background tasks for cache management. 100 clients × any cache-side background loop = death by a thousand wakeups.
- No per-client compile worker pool (daemon has its own)
- No catalog.db SQLite handle (clients don't read or write the catalog directly)
- No per-client `entry_stats` write path. Stats accumulate in process-local memory; **flush is event-driven**, not timer-driven — piggybacks on existing shell events (next prompt fire, next builtin invocation, shell exit). No separate flush thread, no `setitimer`, no async tick.
- No per-client `zcache verify` integrity-check pass (daemon's job)
- No per-client log-rotation worker (daemon's job)
- No per-client orphaned-tmp cleanup (daemon's job)
- No per-client subscription poll loop. If a client wants daemon push notifications (`shard_updated`), the IPC subscribe model uses an `epoll`-able fd that the existing single-threaded event loop in the shell already manages — no dedicated thread.

**What clients DO have:** a **general worker pool for concurrent primitives** — `async`/`await`, `pmap`, parallel pipelines, anti-fork builtins running in worker threads. This pool is part of the existing shell runtime, used for user-driven concurrency in shell scripts. It is **not** used for cache work, fsnotify, polling, or any background task that would multiply across the 100-shell scale.

**Strict rule:** clients run the shell interpreter event loop + general worker pool for concurrent primitives + mmap reads + opportunistic IPC sends piggybacking on existing wake-ups. Anything cache-related that would require spawning a dedicated thread or scheduling a timer in a client is the daemon's job, full stop.

Lookup path is identical to the no-daemon version (~150-200ns hash → index → typed pointer). Cache mutation is exclusively the daemon's job.

**100-shell resource projection (vs alternatives):**

| Architecture | RAM | fd | threads | SQLite handles |
|--------------|-----|----|---------|--------------:|
| Per-client fsnotify (rejected) | 100 × (5 MB + fsnotify state) ≈ 1 GB | 100 × (sock + N watch fds) | 100 × 2 threads | 100 |
| Per-client worker pool + SQLite (rejected) | 100 × (5 MB + 1-2 cache threads + 5 MB SQLite cache) ≈ 1.5 GB | 100 × 5+ | 100 × 2-3 | 100 |
| **Daemon model (accepted)** | 1 × ~30 MB (daemon) + 100 × <5 MB (clients) ≈ 530 MB total | 1 × (watch fds + N) + 100 × 1 | 1 × ~5 (daemon) + 100 × 0 = 5 cache threads total | 1 (daemon-only) |

Daemon model wins by ~3× RAM, ~10× fd, and ~40× thread count vs the per-client-fsnotify approach. At 100 shells the difference is between "comfortable" and "the workstation thrashes."

**Daemon lifecycle:**

- **Spawn on demand by the first client:** first zshrs client to launch checks for `~/.zshrs/daemon.sock`; if absent or unresponsive, fork-spawns the daemon (same binary, `--daemon` flag), waits ~50ms, retries the connect. Subsequent clients (the other 99 in the tmux scenario) just connect to the already-running daemon — no spawn race, no second daemon, no per-client startup cost beyond the connect. **First zshrs spawns it; the next N use it.**
- **Singleton enforcement:** daemon takes `flock(LOCK_EX)` on `~/.zshrs/daemon.pid` at startup. Second daemon instance sees the lock held, logs "daemon already running", exits. Race-safe.
- **Lifetime:** persists across shell sessions. Survives logout. Killed only by explicit `zcache daemon stop` or `pkill zshrs-daemon` or system shutdown.
- **Crash recovery:** if daemon dies (crash, SIGKILL, OOM), next client to fail socket connect kills the stale pidfile and respawns. No state loss — daemon owns no in-memory data that isn't reproducible from sources + on-disk shards.
- **Degraded mode (no daemon):** clients fall back to source-interp for everything. Cache stops updating but shells stay functional. Per the source-truth fallback rule. Edge case for users who explicitly disable the daemon.

**IPC protocol** (Unix domain socket at `~/.zshrs/daemon.sock`):

Length-prefixed JSON messages (small volume, easy debug, fits the message rate). Client → daemon:

| Message | Effect |
|---------|--------|
| `{"op": "info"}` | daemon returns shard sizes, entry counts, in-flight jobs (powers `zcache info`) |
| `{"op": "rebuild", "shard": "zpwr"}` | daemon enqueues compile job (powers `zcache rebuild shard <name>`) |
| `{"op": "rebuild"}` | daemon rebuilds full corpus (powers `zcache rebuild`) |
| `{"op": "clean", "target": "shards"}` | daemon unlinks + re-derives (powers `zcache clean shards`) |
| `{"op": "verify"}` | daemon runs integrity scan (powers `zcache verify`) |
| `{"op": "fpath_changed", "paths": ["/opt/...", "..."]}` | daemon re-discovers source roots, watches new ones |
| `{"op": "stats_flush", "deltas": [{"fq_name": "...", "count": 5, "total_ns": 12000}, ...]}` | client batched runtime stats; daemon merges into `entry_stats` |
| `{"op": "subscribe_shard", "shard": "zpwr"}` | optional: client requests push notification on shard update |

Daemon → client (async, optional, only to subscribed clients):

| Message | Effect |
|---------|--------|
| `{"event": "shard_updated", "shard": "zpwr", "generation": 42}` | client invalidates its cached shard mmap, re-mmaps on next lookup |
| `{"event": "rebuild_complete", "job_id": 17, "duration_ms": 4200}` | reply to async `rebuild` invocation |

Sync vs async same as before: client `zcache rebuild` returns after enqueue ack from daemon (fast); `--wait` blocks on the `rebuild_complete` event.

**Solves the 60-shell problem cleanly:**

| Concern | Per-process fsnotify (rejected) | zshrs-daemon (accepted) |
|---|---|---|
| Number of fsnotify watchers | 60 | 1 |
| Compile contention on shared `git pull` | 60 workers race | 1 worker, 59 clients see result |
| Cross-process coordination | DBus / lockfile / election | trivial — one process owns it |
| Client startup overhead | full fsnotify init per shell | mmap + 1 socket connect |
| Memory overhead per client | ~MB for fsnotify state | none beyond mmaps |
| `git pull zpwr` thundering herd | 60 redundant compiles | 1 compile, 60 transparent re-mmaps |

**Forwarded benefits:**

- **Runtime fpath changes propagate.** Earlier flaw: a client adding to `$fpath` mid-session (zinit, zpwr) wouldn't get the new dir watched. Solved: client sends `fpath_changed` IPC; daemon re-discovers roots and sets up watches.
- **Stats batching is centralized.** Each client accumulates runtime call stats in memory, flushes deltas to daemon (event-driven, on existing shell wake-ups); daemon aggregates and writes to `entry_stats`. No per-client SQLite contention on the warm-write table.
- **`zcache verify` runs once globally** instead of per-client. Daemon owns the catalog; clients query it through daemon if needed.
- **Cache worker pool centralized** — daemon has its own dedicated cache pool; clients have only the general pool for concurrent primitives.

### World-first: shell with a dedicated daemon process

No mainstream shell ships with a dedicated companion daemon. Survey:

| Shell | Daemon? | Notes |
|-------|---------|-------|
| bash | none | |
| zsh | none | |
| fish | none | brief `fishd` for shared history in early versions; removed |
| nu | none | |
| elvish | none | |
| dash / ash | none | |
| ksh / tcsh | none | |

Closest analogs are non-shell: `tmux` / `screen` (terminal multiplexers, not shells; the shells run inside), `emacs --daemon` + `emacsclient` (editor server model), `ssh-agent` / `gpg-agent` (credential caches, not shells).

zshrs is the first shell to ship a dedicated companion daemon. This is an additional world-first on top of the AOT + AOP + nanosecond-profiling + no-GC + cross-language + sharded-rkyv-image stack. Beyond the cache-management role, the daemon is the substrate for a **whole class of future capabilities** that no shell has today — see "Daemon as session-persistent supervisor" below.

### Daemon as cross-shell coordinator (zconvey replacement and beyond)

The shell registry already exists inside the daemon by definition — every connected client is in the daemon's table (pid, tty, cwd, login_time, user-set tags). That registry + the existing IPC channel subsume an entire class of cross-shell coordination plugins that the zsh community built over 20 years on top of filesystem-as-IPC and per-prompt polling.

**Why the daemon model wins structurally over zconvey-style plugins:**

zconvey is the canonical zsh cross-shell-messaging plugin: each shell registers itself by writing into `/tmp/zconvey/<id>/`, polls its inbox via a `precmd` hook, executes received commands. At 100 zshrs in tmux:

- 100 shells × `precmd` per command typed = thousands of `stat()` calls per second on `/tmp/zconvey/*`
- Filesystem-based registry → no atomic enrollment; race conditions on concurrent shell launch
- Local-machine only — zero federation
- Polling latency = up to one full prompt cycle for messages to arrive
- Per-shell polling overhead violates the "no per-client polling" rule

The daemon model gets all of zconvey's features for free:

| Feature | zconvey today | zshrs daemon |
|---------|---------------|--------------|
| Shell registry | filesystem scan | authoritative in-memory table |
| Enroll race | possible | atomic via socket connect |
| Message dispatch | filesystem write + poll | socket push, event-loop wake |
| Latency | ~1 prompt cycle | sub-ms |
| Polling overhead | per-shell precmd `stat()` flurry | zero (event-driven) |
| Broadcast | N filesystem writes | single daemon fan-out |
| Tagged dispatch | not supported | `ztag` + `zsend --tag` |
| Subscriptions | not supported | native pub/sub |
| Federation across hosts | impossible | daemon-to-daemon over SSH multiplex |

**New `z*` builtins for cross-shell coordination** (all checked clean against upstream zsh):

```
zls                    # list active shells: id, pid, tty, cwd, tags, login_time
zid                    # print this shell's daemon-assigned ID
zping                  # daemon liveness + roundtrip latency probe
ztag <name…>           # self-tag this shell (multiple tags allowed: `ztag prod laptop`)
zuntag <name…>         # remove tags

zsend <shell_id> <cmd>            # dispatch a command/string to one shell
zsend --all <cmd>                 # broadcast to every connected shell
zsend --tag <name> <cmd>          # send to shells matching tag
zsend --user <user> <cmd>         # cross-user (root only)

znotify <shell_id> <message>      # status-line / OSC-9 notification
                                  # queues if target is busy; pops on next prompt

zsubscribe <pattern>              # subscribe to events from other shells:
                                  #   zsubscribe shell:42.commands
                                  #   zsubscribe *.git_changes
                                  #   zsubscribe *.chpwd
zunsubscribe <pattern>
```

Each builtin is a **thin IPC wrapper** — sends a JSON message over `daemon.sock`, prints the daemon's response, returns. Zero background threads, zero polling, zero state in clients. Daemon owns the registry, the routing table, the event bus, the subscription map.

**Plugins / patterns this replaces** (alongside cache layer replacements above):

| Plugin / pattern | Replaced by |
|------------------|-------------|
| zconvey (`zc-send`, `zc-list`, `zc-bg-cmd`) | `zsend`, `zls`, `zjob submit` |
| Atuin (cross-machine shell history) | daemon federation + history broker |
| direnv (per-dir env via shell hooks) | daemon `chpwd` subscription, set once globally |
| autoenv | same |
| zsh-history-substring-search shared state | daemon brokers history |
| `nohup` / `disown` / `setsid` for detached jobs | `zjob submit` (see "Daemon as session-persistent supervisor" below) |
| `pueue` (standalone job queue daemon) | `zjob` family — same role, native to the shell, one less daemon |
| zinit's cross-shell completion warm cache | `catalog.db` is already daemon-shared |

**Subscription model (pub/sub for shell events):**

Shell event names follow `<shell_id_or_tag>.<event>` glob patterns:
- `shell:42.commands` — every command run in shell #42
- `shell:42.chpwd` — every `cd` in shell #42
- `*.commands` — every command in every shell (audit / pair programming)
- `tag:prod.commands` — every command in any shell tagged `prod`

Daemon brokers events through the same socket subscribers connect on. Clients receive `{"event": "command", "shell_id": 42, "cmd": "git push"}` etc. on their epoll-able fd; existing event loop dispatches without spawning a thread. Zero per-client cost when not subscribed; sub-µs when subscribed.

**Killer use cases this enables:**

- **Pair programming** — shell A: `zsubscribe shell:7.commands`. Watch your colleague's session in real time.
- **Multi-host orchestration** — `zsend --tag prod 'kubectl rollout restart deployment/foo'` lands on every prod-tagged shell across machines.
- **Smart `cd` mirroring** — `zsubscribe shell:1.chpwd` then `precmd { [[ $event_chpwd ]] && cd $event_chpwd_dir }` keeps a side shell in sync with your main.
- **Cross-shell job queues** — shell A is a builder, shell B a runner. `zsend builder 'compile foo'` from anywhere; result streams back via `zjob output`.

This is the **third world-first** stacking on the daemon: native cross-shell pub/sub + dispatch as first-class shell primitives. zconvey was a community workaround; building it into the substrate eliminates the polling overhead and unlocks federation that filesystem-IPC could never reach.

### Daemon as session-persistent supervisor (forward-looking)

The cache-management role is the v1 use of the daemon. The architecture **generalizes to a session-persistent shell-level supervisor** that no shell currently provides. User framing: *"clients can push long running jobs to server etc. you can exit your shell and your job is still running, like tmux really but at shell level"*.

Future expansion of the same daemon (post-v1; not in current scope but the architecture must not preclude it):

- **`zjob submit <cmd>`** — client hands a long-running command to the daemon; daemon spawns it, captures stdout/stderr, returns a job ID. User exits the shell; the command keeps running under the daemon.
- **`zjob status [id]` / `zjob list`** — query running, completed, failed jobs. Daemon retains job state for a configurable retention period.
- **`zjob attach <id>`** — open a stream of the live job's output to the calling client.
- **`zjob output <id>`** — print captured output of a (possibly completed) job.
- **`zjob kill <id>`** — terminate a job under the daemon.
- **Survives logout** — daemon owns the job; the originating shell exiting doesn't kill it. Same property `tmux` provides for terminal sessions, but at the *shell-process* level — no extra terminal multiplexer needed.
- **Same daemon, expanded role** — no second binary, no new IPC socket. The protocol just gains job-management messages alongside cache messages.

**Why this is a world-first capability:** `nohup`, `disown`, `&`, `setsid` all detach a process but lose supervision (no central status, no output capture, no cross-shell visibility). `tmux` / `screen` provide session persistence but at the *terminal* level — they multiplex terminals, not shell-level jobs. `systemd-run --user --scope` comes closest but requires explicit setup, lives outside the shell's mental model, and is Linux-only. zshrs's daemon supervises shell-launched jobs natively, with one interface, on every platform zshrs runs.

This is the **second world-first that the daemon model unlocks** — beyond "shell with a daemon," it's "shell with native session-persistent job supervision." Both world-firsts compound on the daemon being there in the first place.

**Force-wipe is always available.** User can invalidate any/all cache state at any time via either path:

| Method | Effect |
|---|---|
| `rm -rf ~/.zshrs/` | nukes everything including history + stats |
| `rm -rf ~/.zshrs/images/` | nukes all shards; index goes stale until rebuild |
| `rm ~/.zshrs/images/foo.rkyv` | surgical: nukes one shard |
| `cache clean --all` | builtin equivalent of full `rm -rf` |
| `cache clean shards` | builtin equivalent of `rm -rf images/` |
| `cache clean shard <name>` | builtin equivalent of single-shard `rm` |

The non-blocking invariant covers force-wipe too — if user nukes the cache mid-session, every subsequent function call falls through to source interp; an explicit `zcache rebuild` repopulates whenever the user invokes it; the shell never stops responding regardless.

### How runtime lookup works (two-level, non-blocking)

1. Shell process opens `index.rkyv` once at startup, mmap's it (small, ~1-2 MB for 50k entries). If `index.rkyv` doesn't exist yet (cold install), main thread proceeds without it; worker enqueues full-corpus build.
2. Function call → hash `fq_name` → `index.rkyv` lookup → `(shard_id, generation, byte_offset)` (generation counter handles atomic shard swaps mid-session).
3. **Hit:** get-or-mmap the shard image (LRU cache of shard handles; close fd after mmap since POSIX keeps the mapping alive); compare cached shard's generation against index entry; if stale, munmap + remmap; typed pointer at offset; JIT or interp runs the bytecode chunk directly from the mmap region.
4. **Miss** (function not in any shard, or shard file missing, or main thread doesn't want to wait on a stale-shard re-mmap): fall back to interp-from-source via the existing parser path; enqueue a compile job for that source root; main thread continues without blocking.
5. Kernel demand-pages cold regions, evicts under pressure. Working set tracks actual call hot-set, per-shard.

**Lookup cost (cache hit):** ~150-200ns end-to-end (one extra index dereference vs the monolithic ~100ns). Negligible — Tab completion and prompt firing both fit well under any human-perceptible budget.

**Lookup cost (cache miss):** falls back to source interp. Same speed as zsh today (no regression). Worker quietly catches up the cache so the next call hits.

**fd / mmap pressure:** 100-200 shards × 1 mmap region each = trivial. macOS has no `vm.max_map_count`; Linux defaults to 65530. Closing fds after mmap drops fd cost to zero. Cold shards never get mmap'd; LRU-evicted handles get unmapped under memory pressure (rarely needed; mmaps are cheap to keep).

Per-shell RSS attributable to images:

| Scenario | Resident |
|----------|----------|
| Cold script (`zshrs -c 'echo hi'`) | 1-3 MB (index + 1-2 shards touched) |
| Empty interactive shell after first prompt | 5-15 MB (index + system + zpwr + a few plugin shards) |
| 1hr interactive session | 10-30 MB (warm shards stay; cold ones never mmap'd) |
| 16 parallel zshrs (Cursor workflow) | **same 10-30 MB total** — page cache shares physical pages across all 16 |

VSZ ~250 MB across all mmap'd shards (free on 64-bit). Same property AOT-into-binary advertised but with kernel-managed eviction the AOT route can't do.

### Per-shard rebuild semantics (daemon-owned)

All compile work runs in the **zshrs-daemon** worker pool. Trigger paths:

- **fsnotify (daemon-side, ONE watcher across the machine):** daemon's fsnotify thread sees source change → enqueues compile job into its work queue → continues. No 60×-fsnotify thundering herd; the daemon is the only watcher.
- **Explicit user invocation (`zcache rebuild`):** any client builtin sends `{"op": "rebuild", "shard": "..."}` over the daemon socket; daemon enqueues the same job. Multiple clients sending concurrent rebuild requests for the same shard → daemon coalesces into one job.
- **Runtime config push (`fpath_changed` IPC):** client whose `.zshrc` does `fpath+=(/opt/...)` sends `fpath_changed` to daemon; daemon re-discovers source roots, registers fsnotify watches, and may enqueue a rebuild for the new root.

**Daemon worker job:**

1. Worker takes `flock(LOCK_EX)` on `images/{name}.rkyv.lock` (still useful — guards against an external `zcache` invocation against a no-daemon mode, or a second daemon mid-handoff).
2. Worker re-walks the source root, recompiles its functions, writes to `images/{name}.rkyv.tmp.{pid}.{tid}`.
3. **Strict ordering: shard rename FIRST, then index update.**
   - atomic-rename `{name}.rkyv.tmp.{pid}.{tid}` → `{name}.rkyv` (new generation in header)
   - rewrite `index.rkyv` to point at new offsets + bump shard generation in index
   - reverse order would let clients read new-index → new-offsets → deref into OLD mmap → corrupt reads. Strict ordering prevents the race.
4. Worker chains `hydrate_shard {name}` job: `DELETE FROM entries WHERE plugin_id = '{shard}'` + INSERT new rows. `entry_stats` survives via `ON CONFLICT DO UPDATE` keyed on `fq_name`.
5. Worker releases flock.
6. **Optional push:** if any client subscribed via `subscribe_shard {name}`, daemon sends `{"event": "shard_updated", "shard": "{name}", "generation": N}` to those clients. Subscribed clients invalidate their cached shard mmap immediately. Non-subscribed clients pick up the new shard transparently on their next lookup via the generation-mismatch re-mmap path.

Clients participate in **none** of this. If a function call comes in while the rebuild is mid-flight, the client reads the OLD index, gets old offsets, derefs into its old mmap (still alive via unlink-but-mapped semantics), runs that bytecode, and continues. Atomic rename + strict ordering = swap invisible at the syscall level.

Rebuild cost matrix:

| Event | Old (monolithic image.rkyv) | New (sharded) |
|---|---|---|
| `git pull` in zpwr tree | ~30s full corpus rebuild | ~3-5s zpwr.rkyv only |
| `zinit update foo` (single plugin) | ~30s | ~100-500ms plugin-foo.rkyv only |
| `zinit update-all` (200 plugins) | ~30s batched, much worse if serial | parallel: ~10-30s for all 200 (worker pool) |
| Edit one .zshrc helper | ~30s | ~100ms (zpwr.rkyv or wherever it lives) |
| Fresh install on new machine | ~30s | ~30s same — full corpus must compile once |
| `zshrs --rebuild-cache --shard zpwr` | n/a | ~3-5s, surgical |

**Plugin install/update commands** (zinit/oh-my-zsh wrappers) trigger the affected shard's rebuild + hydrate atomically.

### How the catalog stays fresh — per-shard worker-pool hydration

After any shard rebuild, the worker pool enqueues a hydrate job scoped to that shard:

1. Worker thread mmap's `images/{shard}.rkyv`, walks its entries.
2. `DELETE FROM entries WHERE plugin_id = '{shard}'` then INSERT the new rows. Same for `hooks`. Plugin metadata in `plugins` table is untouched (orchestrator manages that directly).
3. SQLite WAL absorbs the per-shard rewrite without blocking concurrent dbview readers.
4. All steps log to `~/.zshrs/zshrs.log` via `tracing::info!` — `hydrate_shard_start { shard, entries: N }`, `hydrate_shard_complete { shard, duration_ms }`. Never to terminal (per `DESIGN_GOALS.md` "informational chatter goes to log only").

Per-shard hydrate cost: ~10-100ms typical (single zinit plugin) vs 100-500ms for a large shard like `completions-corpus`. 17 other worker threads remain available; multiple shard rebuilds parallelize naturally.

`entry_stats` (call counts, ns timing) is partition-stable — entries keyed by `fq_name` survive shard rebuilds since the fq_name doesn't change unless the source did. Worker preserves stats across rebuilds via `ON CONFLICT DO UPDATE`.

### catalog.db schema

| Table | Source | Purpose |
|-------|--------|---------|
| `plugins` (name, version, source, installed_at, enabled) | written directly by install/update commands | orchestrator state |
| `plugin_deps` (plugin, dep, constraint) | install-time | dep graph for resolution |
| `entries` (fq_name, plugin_id, kind, image_offset, source_loc) | hydrated from rkyv | what's compiled, where it lives |
| `hooks` (kind, name → fq_name) | hydrated from rkyv | precmd / preexec / widget / completion ownership |
| `entry_stats` (fq_name, last_called_at, call_count, total_ns) | runtime increment via worker, batched flush every N seconds | "which completions am I actually using" — drives dbview perf reports |

`entry_stats` is the only warm-write table; runtime batches in-memory deltas and flushes via a worker thread on a configurable cadence (default 30s). Lookups never hit the catalog on the hot path.

### What dies

The pivot kills entire optimization layers that exist today only to paper over the source-load model:

- `.zcompdump` — compinit-generated completion db. Replaced by `image.rkyv` entries.
- `plugin_cache.db` (legacy SQLite bytecode cache) — replaced by `image.rkyv`.
- `compsys.db` (legacy) — replaced by `image.rkyv` entries.
- `.zwc` (zsh wordcode) reading paths — daily-driver doesn't read .zwc; rkyv image is the compiled form.
- `zinit turbo` equivalent — async plugin defer. Image holds everything; no defer needed.
- `p10k instant prompt` equivalent — fake-prompt-while-loading hack. First paint = full functionality.
- `compaudit` — security check on completion files. Image rebuild step does the audit once at compile time.
- `autoload -U` — becomes a no-op (or build-time tag).

Estimated LOC eliminated from zshrs: 5,000-10,000 across legacy `plugin_cache.rs`, compsys cache code, autoload machinery, .zwc readers, async load workers. Net simplification.

### What stays external

- **`history.db`** — primary mutable data, frequently appended, B-tree-indexed. SQLite is correct for this workload.
- **Working files** the user is editing (source code, docs).
- **Runtime IPC** (sockets, FIFOs).
- **Source trees** (zinit plugin dirs, fpath, user `.zshrc`) — source-of-truth for human inspection / git tracking. The compile pass reads them; runtime only reads `image.rkyv`.

### Daily workflow

```
$ vim ~/zpwr/zpwrTop                          # edit a zpwr subcommand
$ zcache rebuild shard zpwr                   # explicit; ~3-5s (zpwr only, not full corpus)
                                              # builtin returns immediately; worker pool
                                              # compiles + hydrates catalog.db in background
$ exec ~/bin/zshrs                            # picks up new index + zpwr shard
```

Same pattern after `zinit update`, `git pull` in any source tree, fresh install. User-driven, predictable, matches existing `zpwr regen` workflow. `zcache rebuild` (no shard) does the full corpus rebuild for a fresh install or full reset.

In a 60-shell environment: any one shell can run `zcache rebuild`; the per-shard flock ensures only one process at a time rebuilds a given shard; the other 59 shells see the new shards transparently on next lookup via the generation-counter re-mmap path.

### Engineering details (flaws found and locked)

These were red-team findings on earlier drafts; specifying them here so they can't drift.

**Shard naming convention — collision-proof.** Shard files are named `images/{hash8}-{slug}.rkyv` where `hash8` is the first 8 hex chars of the source-root absolute-path hash (BLAKE3 or SipHash; doesn't matter as long as it's stable). `slug` is a human-readable kebab-case derivation of the path's last component (e.g. `4f2a91c3-zpwr.rkyv`, `a8c1d402-zsh-syntax-highlighting.rkyv`). Hash prefix makes collision impossible; slug suffix keeps `ls` output readable. Reserved prefixes (`system`, `user`, `completions-corpus`) get the same hash treatment for uniformity — no special-casing.

**Cross-shard JIT inlining is disabled in v1.** Cranelift JIT inlines within a shard's bytecode chunk freely. Inlining a callee from a different shard would create a hot-path dependency that becomes stale when the callee's shard rebuilds; tracking caller→callee dependencies for invalidation adds significant complexity. v1 rule: **no cross-shard inlining.** Cross-shard calls go through the index lookup (~150-200ns) every time. Revisit only if benchmark shows this as a measurable hot path. Not expected to be one.

**Multi-process rebuild coordination — per-shard flock.** Each shard has an advisory `images/{name}.rkyv.lock` file. Worker takes `flock(LOCK_EX)` before compile, releases after atomic rename + index update. Cross-process (60-shell scenario): only one process actively rebuilds a given shard at any moment; others either wait (with `--wait`) or skip (default async — the contending worker sees the lock held, logs "rebuild already in progress", returns).

**catalog.db corruption recovery via `zcache verify`.** SQLite `PRAGMA integrity_check` runs only when `zcache verify` is invoked (not on every startup — it's seconds-long for a 100MB db). On detected corruption, `zcache verify` prints recommended recovery: `zcache clean catalog && zcache rebuild`. Documented trade: `entry_stats` are lost on corruption recovery (they live inside catalog.db; no separate persistence layer per the single-dir constraint). User accepts this as the price of single-dir simplicity.

**Orphaned `.tmp` cleanup at startup.** Worker writes `images/{name}.rkyv.tmp.{pid}.{tid}` then atomic-renames. If a worker is killed mid-write (SIGKILL, OOM, power loss), the `.tmp.*` file orphans. On every shell startup, a worker job scans `images/` for `.tmp.*` files older than 1 minute and unlinks them. Also surfaced via `zcache verify` ("3 orphaned tmp files; run `zcache clean shards` or wait for next cleanup pass").

**Log rotation — built-in size cap.** `~/.zshrs/zshrs.log` is capped at 10 MB by default. On reaching the cap, current log rotates to `zshrs.log.1`, oldest is dropped. Configurable via `ZSHRS_LOG_MAX_SIZE` and `ZSHRS_LOG_MAX_FILES` env vars. Avoids needing external `logrotate` config and prevents unbounded growth.

**Worker pool partitioning.** Two logical pools sharing the underlying worker threads:
- **General pool** — handles user command pipelines (`xargs -P 16`, async commands, etc.). High-priority, sized to `nproc - 1` by default.
- **Cache pool** — handles `zcache` jobs (compile, hydrate, verify, compact). Low-priority, sized to 1-2 threads. Cache jobs explicitly yield between work units.
This prevents `zcache rebuild` from starving an interactive `find / | xargs ...` and vice versa. Implementation: priority field on each queued job + a small CPU-budget cap on cache jobs.

**Concurrent shell access to `index.rkyv` during rewrite.** The `index.rkyv` rewrite uses the same atomic-rename pattern as shards (write to `index.rkyv.tmp.{pid}.{tid}`, atomic rename). Main thread mmap of `index.rkyv` survives the rename via unlink-but-mapped semantics — readers see the OLD index until they re-mmap. Re-mmap is triggered when a shard lookup yields a generation mismatch; at that point main also re-opens `index.rkyv` to pick up the latest mappings.

### Acceptance criterion (revised, supersedes §0x0B for daily-driver mode)

The cache architecture has succeeded when:
- Full-corpus rebuild (`zshrs --rebuild-cache`) over zshrc + zinit plugins + zsh-more-completions: <30 seconds clean.
- Per-shard rebuild (`zshrs --rebuild-cache --shard zpwr`): ~3-5s for a large shard, ~100-500ms for a single plugin shard.
- `git pull` in zpwr → only `zpwr.rkyv` rebuilds (~3-5s, not 30s).
- `zinit update foo` → only `plugin-foo.rkyv` rebuilds (~100-500ms).
- Cold shell launch: <5ms (index.rkyv mmap + first-prompt hook shards mmap'd lazily).
- 100 parallel shells (in tmux) share <30 MB total RSS attributable to images (page-cache hot-set, not 100× per-process). Per-client cache overhead beyond zsh interpreter <5 MB.
- Tab completion lookup: ~150-200ns end-to-end (index lookup + shard-cache hit + JIT call).
- No SQLite hit on the hot path. Only `history.db` (append) and `catalog.db` (when dbview is invoked or per-shard worker hydrate is running) are touched.
- `dbview` always opens to a current view (worker keeps it warm per-shard; staleness window ≤ shard rebuild duration).
- `entry_stats` survives shard rebuilds — `fq_name`-keyed rows preserved via `ON CONFLICT DO UPDATE` so personal usage analytics aren't reset every plugin update.

### Why not AOT-into-binary at this layer

| Property | AOT-into-binary | rkyv image + catalog |
|----------|-----------------|----------------------|
| Working set scales with | total installed corpus | functions actually dispatched |
| Page eviction | no — `.text` PROT_EXEC pinned | yes — kernel demand-pages |
| Per-shell RSS (zpwr scale) | 50-100 MB constant overhead | 5-30 MB, scales with usage |
| 16 parallel shells | shared but pinned 50-100 MB | shared and evictable, ~10-30 MB total |
| Plugin install cost | full zshrs rebuild + relink | ~500ms image rebuild; binary unchanged |
| Plugin upgrade rollback | reinstall whole binary | swap a single file |
| Crash isolation from bad plugin bytecode | none — same address space, signals shell | fusevm sandbox; bad bytecode trapped, not SIGSEGV'd |

The script-AOT capability (§0x00–§0x12) deploys *individual scripts* as static binaries — that workload has a bounded code size per artifact. The daily-driver corpus does not have a bounded size; its working set discipline must come from the kernel page cache, not from the linker.

### World-first verification (revised, daily-driver layer)

| Capability | Prior art |
|------------|-----------|
| Shell with rkyv-mmap zero-copy bytecode dispatch + worker-hydrated SQLite query mirror | None — bash/zsh/fish/nu/elvish all use either source replay or single-store cache without a queryable mirror |
| Daily-driver shell where install-set / hot-set / lookup-cost are all decoupled (install can be huge, hot is small, lookup is constant) | None — every existing shell ties at least two of the three |
| Per-completion call-count + ns-timing telemetry queryable via SQL on a live shell | None — even profilers like zprof don't persist; this is always-on |

The script-AOT world-firsts from §0x13 prior version (deploy-as-binary, one-product) still stand for that workload — they were never the daily-driver story. Combining script-AOT for deployment with rkyv-image cache for daily-driver is the correct two-track architecture.

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
| 2026-04-28 | §0x13 corrected — "binary IS the database" reversed for the daily-driver layer. AOT-into-binary for plugins/compsys fails the working-set test at zpwr scale (200-400 MB bytecode → 50-100 MB pinned `.text` per shell, kernel can't evict `PROT_EXEC` pages). Replaced with rkyv-mmap'd `image.rkyv` + worker-hydrated `catalog.db` mirror. Three-file cache layout (`image.rkyv`, `catalog.db`, `history.db`); zero per-plugin litter; ~10-30 MB RSS in normal use; 16 parallel shells share via page cache. `entry_stats` table tracks call counts + ns timing for always-on profiling queryable via dbview. Hydration runs on idle worker thread, logs to `~/.zshrs/zshrs.log` via `tracing::info!`. Script-AOT (§0x00–§0x12) capability unchanged — that's a different workload (deploy individual scripts as static binaries) where bounded code size makes AOT-into-binary correct. | correction |
| 2026-04-28 | §0x13 sharded — monolithic `image.rkyv` reversed in favor of one-image-per-source-root layout under `~/.zshrs/images/`. Reason: monolithic image forces full-corpus rebuild on any source change, killing the iterate-on-zpwr loop (~30s per `git pull` even for a one-line change). Sharded layout reduces rebuild blast radius to the touched root: `git pull` in zpwr → ~3-5s `zpwr.rkyv` only; `zinit update foo` → ~100-500ms `plugin-foo.rkyv` only; `zinit update-all` parallelizes across the worker pool. New top-level `index.rkyv` provides fq_name → (shard_id, byte_offset) lookup; per-shard mmap with LRU shard-handle cache. Lookup cost ~150-200ns vs the monolithic ~100ns (extra index dereference, negligible). Cache directory now has bounded litter (one image per plugin) — explicit trade for rebuild speed and parallelism. Per-shard worker hydration keys catalog `entries`/`hooks` by `plugin_id` partition; `entry_stats` survives rebuilds via `ON CONFLICT DO UPDATE`. fsnotify schedules per-shard rebuilds. | correction |
| 2026-04-28 | §0x13 single-dir layout locked — earlier draft proposed splitting persistent data (`history.db`, `entry_stats`) into `~/.local/share/zshrs/` so `rm -rf ~/.zshrs/` would be nuke-safe. User overruled — single dir wins on simplicity ("litter is fine but only in 1 dir that is easily rm" / "user can cd into dir and delete individual subfolders, fine"). All cache + persistent data colocated under `~/.zshrs/`. Trade explicit: full `rm -rf` nukes history + accumulated stats; user responsibility. Surgical resets via per-file `rm` (table added in §0x13). Matches the global "user is root, no friction" rule. | correction |
| 2026-04-28 | §0x13 `zcache <verb>` builtins added — supervised counterparts to raw `rm`/manual rebuild. User: "yeah, we should add builtins to cover the cache clean as well". Verbs: info / jobs / clean / rebuild / verify / compact, with sub-verbs and `--all` / `--no-stats` / `--wait` flags. Builtins release running shell's mmap handles before unlink (raw `rm` doesn't), trigger worker re-hydrate after clean, preserve `entry_stats` across catalog reset by default. Operations log to `zshrs.log` via `tracing::info!`. Raw `rm` remains valid bypass path. | addition |
| 2026-04-28 | §0x13 non-blocking invariant locked — user: "nothing can block the shell, rykv blob building must all occur in worker pool" + "yes, if blob not built/misshaped then we are forced to use source code, until blob built". All rkyv shard compilation, image writes, `index.rkyv` rewrites, catalog hydration, fsnotify-triggered rebuilds run in the worker pool. Main shell thread only does mmap reads and falls through to existing source-interp path on any image miss / malformed shard / version mismatch / cache corruption. Source files are source of truth; image is opportunistic accelerator. Cold-install case: first shell uses source interp for everything (slow but not blocked); workers fill in image; subsequent shells fast. All `cache` write verbs are async (return immediately); `--wait` is the explicit opt-in for blocking. fsnotify runs on its own thread, dispatches into worker queue lock-free. This decoupling is what makes "nothing blocks" achievable — alternative (image required for execution) would force cold-start blocking and crash on any cache corruption. | architectural rule |
| 2026-04-28 | §0x13 fsnotify default + force-wipe semantics — user: "fsnotify is cool concept tho, no need to explicitly recache, but user should always be able to forcewipe the cache at any time." Earlier draft had `cache_auto_rebuild` defaulting OFF for muscle-memory parity with `zpwr regen`. Reversed — fsnotify auto-rebuild ON by default, no explicit recache for routine edits. Users who want manual control opt out via `unsetopt cache_auto_rebuild`. Force-wipe is always available regardless of the setting: raw `rm -rf ~/.zshrs/` (full nuke), `rm -rf ~/.zshrs/images/` (shards only), per-file `rm`, or `cache clean --all` / `cache clean shards` / `cache clean shard <name>` builtins. First-time cache build remains "part of the game" — same upfront cost as zpwr regen recompiling .zwc/.zcompdump today. | refinement |
| 2026-04-28 | §0x13 POSIX-mode gating — user: "oh yeah b/c targetting replacement of all shells even sh, all of this must be gated behind --posix etc." Entire cache layer (image lookup, catalog open, fsnotify daemon, worker cache jobs, `zcache <verb>` builtins, even creation of `~/.zshrs/`) is gated behind zsh-mode. Under `--posix` / `emulate sh` / argv[0] basename of `sh`/`dash`/`bash`, zshrs runs `parse → bytecode → interp/JIT` straight-through with no cache machinery. Drop-in `/bin/sh` replacement: no surprise daemon threads, no watch fds, no cache dirs created in containers / cron / init scripts. Cache-layer activation lives only in default zsh mode where the daily-driver workload (zinit + 17k completions + zpwr) lives. Non-negotiable per zshrs's "drop-in for sh/bash/dash" positioning. | constraint |
| 2026-04-28 | §0x13 generalized to three personality modes — user: "the idea is 1 codebase for all different shell variants, sh vs vanilla zsh vs turbocharged zshrs, etc". POSIX gating expanded into a three-tier scheme: POSIX (strict `/bin/sh`, no extensions, no cache), Vanilla zsh (mainline-zsh-compatible, zsh extensions on, cache OFF — no surprise daemons/dirs for users who just want "fast zsh"), Turbocharged zshrs (default, full extensions + cache + fsnotify + AOP + async). Mode selected at startup via argv[0] basename or flag; immutable for process lifetime. One binary, three personalities — ship targets and code reuse collapse, while marketed personalities have non-negotiable contracts (sh basename = POSIX-only; zsh basename = vanilla zsh; zshrs basename = turbocharged). Cross-contamination between modes is a regression. | constraint |
| 2026-04-28 | §0x13 fsnotify REMOVED — user runs 60+ parallel zshrs (Cursor workflow); fsnotify daemon per process = 60+ watchers, all firing on every source edit, all enqueueing rebuild jobs. Even with debounce + per-shard flock, the thundering-herd cost is fatal (59 wasted compiles per edit). Single-process fsnotify with cross-process election (DBus / lockfile) adds moving parts not worth the convenience. Cache rebuild reverts to **explicit-only** via `zcache rebuild [shard <name>]` — matches the existing `zpwr regen` muscle memory. Per-shard flock still required to coordinate multiple shells doing concurrent explicit rebuilds. Quotes: "remove the whole fsnotify, too much conflict with 60x zshrs" / "i run 60x zsh regularly" / "we cant have 60x fs notify running". | correction (superseded by daemon model below) |
| 2026-04-28 | §0x13 zshrs-daemon client/server model — user: "best idea, have a zshrs-daemon process that runs on loop, 1x for N zshrs regular shells, it runs on minimal loop to update all rykv blobs and hydrates sql ite, zshrs clients can send messages to update on updates to configuration like fpath changes" + "yep, like a client server model, the zshrs-daemon handles all cache management, client push changes to it if needed". Singleton daemon owns ALL cache mutation: fsnotify (one watcher across the machine), compile workers, image writes, index rewrites, catalog hydration, log rotation, orphaned-tmp cleanup, integrity scans. Clients are paper-thin readers that mmap daemon outputs + send IPC over `~/.zshrs/daemon.sock`. Daemon spawned on demand by first client; `flock` on `daemon.pid` enforces singleton; survives shell-session boundaries. `zcache <verb>` builtins become thin clients sending JSON over the socket. Restores fsnotify-driven auto-rebuild without 60×-watcher thundering herd. Resolves runtime fpath-change propagation (`fpath_changed` IPC). Centralizes stats aggregation, dbview, verify, compact. Two new files in cache dir: `daemon.sock`, `daemon.pid`. | architecture |
| 2026-04-28 | §0x13 strict thin-client rule — user: "we have to keep clients light, I usually ran 100x zsh inside tmux. we cant been doing heavy work in clients. it will kill CPU/MEM" + "no worker pool polling or whatever in thin clients". Per-client cost cap: <5 MB beyond zsh interpreter, ZERO cache-related background threads / polling loops / timers / SQLite handles. Stats flush is event-driven (piggybacks on existing shell events, no separate flush thread). IPC subscribe uses epoll-able fd in existing event loop. Anything cache-related requiring a thread or timer in a client = daemon's job. 100-shell projection: daemon model uses ~530 MB total vs ~1-1.5 GB for per-client-fsnotify alternatives — ~3× RAM, ~10× fd, ~40× thread savings. Scale of 100 in tmux is the load-bearing constraint, not 60. | refinement |
| 2026-04-28 | §0x13 client worker-pool clarification — user: "they need worker pool tho, but only for concurrent primitives, we cant be polling, fsnotify etc in thin client worker pool. first zshrs will spawn the daemon, N zshrs will not, only 1 daemon, it will monitor all plugins/completions etc, update caches, hydrate sqlite for viewing etc.". Clients DO have a worker pool — the existing general pool for `async`/`await`/`pmap` and other concurrent primitives. What they don't have is a CACHE-related pool, fsnotify watcher, or any background polling/timer. First zshrs spawns daemon; subsequent N just connect. Daemon monitors plugins/completions, updates caches, hydrates SQLite for dbview. | clarification |
| 2026-04-28 | §0x13 daemon = world-first AND foundation for session-persistent jobs — user: "its a world first, just another, a shell with a dedicated daemon process" + "we can expand that concept in the future, the clients and daemon concept. clients can push long running jobs to server etc. you can exit your shell and your job is still running, like tmux really but at shell level". No mainstream shell (bash/zsh/fish/nu/elvish/dash/ksh/tcsh) ships with a dedicated companion daemon. Closest analogs are non-shell (tmux, emacs --daemon, ssh-agent). zshrs is the first. Beyond cache management (v1 use), daemon is the substrate for `zjob submit/status/attach/output/kill` — long-running detached jobs that survive shell exit ("tmux at shell level", but at process granularity not terminal granularity, native to the shell, cross-platform). Two stacked world-firsts on the same daemon: (1) shell with dedicated daemon, (2) shell with native session-persistent job supervision. v1 ships cache-management role only; architecture must not preclude the future job-supervisor expansion. | architecture + world-first |
| 2026-04-28 | §0x13 daemon as cross-shell coordinator (third world-first) — user: "basically, the daemon thing coupld replace zconvey plugin, daemon knows about all shells and can push/pull data" + "next level add this to docs". Daemon's authoritative shell registry + IPC channel subsume zconvey-class plugins (filesystem-IPC + per-prompt polling) at strictly better latency, scale, and architecture. New `z*` builtins for cross-shell dispatch and subscription: `zls`, `zid`, `zping`, `ztag`/`zuntag`, `zsend [--all|--tag|--user]`, `znotify`, `zsubscribe`/`zunsubscribe`. All thin IPC wrappers — zero background work in clients. Subscription pub/sub uses `<shell_id_or_tag>.<event>` glob patterns. Plugins/patterns dying alongside zconvey: Atuin (cross-machine history), direnv (chpwd hooks), autoenv, pueue (job queue daemon), zsh-history-substring-search shared state, zinit cross-shell completion cache. Killer use cases unlocked: pair-programming via subscribe, multi-host orchestration via `--tag`, smart cd mirroring, cross-shell job queues. Third stacked world-first: native cross-shell pub/sub + dispatch as first-class shell primitives. v1 cache management lays the substrate; v2+ adds these capabilities. | architecture + world-first |
| 2026-04-28 | §0x13 personality vs emulation scope distinction — earlier draft said "modes are immutable for the lifetime of the process" without distinguishing personality mode (process-wide, controls cache/workers/builtins) from emulation scope (per-function `emulate -L`, controls parser flags only). zinit and many zsh plugins use `emulate -LR zsh` constantly; treating that as a "mode switch" would have broken them. Personality is immutable; emulation scope is per-function and standard-zsh. Two separate concepts in two separate code paths. | flaw fix |
| 2026-04-28 | §0x13 `cache <verb>` → `zcache <verb>` rename — "cache" is a high-collision command name (existing PATH tools, likely zpwr subcommand). Renamed to `zcache` to follow the zsh `z*` convention (`zmv`, `zparseopts`, `zformat`, `zstat`, `zstyle`, `zprof`) without colliding with any upstream zsh `z*` builtin. Build-time anti-collision check against upstream zsh's z-namespace. Locked rule: all zshrs-introduced builtins use `z` prefix and must not shadow any upstream zsh builtin. User: "we namespace prefix all our custom builins with z, but no clash with zsh z*". | flaw fix |
| 2026-04-28 | §0x13 engineering details locked — red-team pass found 10 design flaws; all fixed and specified in new "Engineering details" subsection: shard hash-prefix naming (`{hash8}-{slug}.rkyv`); cross-shard JIT inlining disabled in v1; per-shard flock for multi-process coordination; `zcache verify` for catalog corruption recovery (entry_stats loss documented as the trade); orphaned `.tmp.{pid}.{tid}` cleanup at startup; built-in log rotation (10 MB cap, configurable); worker pool partitioned into general (high-priority) and cache (low-priority, 1-2 threads); strict shard-rename-then-index-update ordering with generation-counter-driven re-mmap on stale shard handles. User: "fix them all". | flaw fix |

---

## [0x15] Next step

Review and lock-in §0x0D defaults. If all green, Phase A (fusevm cranelift-object output) starts with a version bump to fusevm 0.11 and a new `aot` module added to that crate. If any default is wrong, edit this doc first; code follows the doc, never the other way around.
