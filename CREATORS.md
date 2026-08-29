# Creators

zshrs was created by [MenkeTechnologies](https://github.com/MenkeTechnologies).

Original synthesis (2025–): the first JIT-compiled Unix shell, the
no-fork architecture, the 90/10 work split between
`zshrs-daemon` and the shell, the AOP-intercept recorder
(`zshrs-recorder`), the `~/.zshrs/` single-directory rule, the
`fusevm` bytecode VM (shared with stryke), the rkyv-backed
canonical-state shards, the `z*` builtin family
(`zcache` / `zls` / `zsend` / `znotify` / `zsubscribe` / `zjob` /
`zsync` / `zask` / `zlock` / `zpublish` / `zwhere` / `zd`), the
session-persistent supervised job runner with bidirectional ptmx
attach, the daemon-as-service HTTP surface with auto-derived
OpenAPI 3.1, the FTS5-indexed shared-history sqlite, the
zsh-extended-history flat-text mirror, and the
zpwr / zsh-more-completions compatibility floor — all originated
here.

For governance, contribution flow, and current maintainers, see
[MAINTAINERS.md](MAINTAINERS.md).

## Legacy

This is a legacy, not a battle. The synthesis above is meant
to outlast this codebase — to flow into bash, fish, nushell,
elvish, oil, xonsh, murex, projects that don't exist yet,
shell ideas that take twenty years to mature. None of this
is defended turf; the protected invariants in `MAINTAINERS.md`
guard upstream identity, not the ideas themselves.

The zshrs corpus is offered as **prior art for the
shell-design commons**. Take what helps. Drop what doesn't.
No permission needed beyond what the MIT license already
grants.

The canonical register of what originated here is
[`docs/INVENTIONS.md`](docs/INVENTIONS.md) — twenty-nine entries,
each filtered by three tests: it exists in the tree with a name you
can type, no other shell does the thing at all (with the near misses
named), and it is an idea another project could inherit rather than a
file layout or a flag. That document also lists what was cut and why,
including claims that turned out to be false on a prior-art check.

The short form, grouped as the register groups them:

- **Execution** — bytecode + Cranelift JIT with both the bytecode and
  the native code persisted across processes; AOT compilation of the
  completion corpus keyed on compiler identity; JIT tier introspection
  (`--tiers`) that tells you why a chunk stayed interpreted; the
  anti-fork architecture (worker pool + 23 in-process coreutils);
  parallelism as VM-dispatched builtins (`pmap`, `pgrep`, `peach`,
  `barrier`, `async`, `await`) rather than as a library.
- **State** — recorder-owns-rebuild, an AOP-intercept pass that
  captures `(kind, name, value, file, line, fn_chain)` for every
  state-mutating dispatcher instead of static-walking your dotfiles;
  the split between configuration that can be cached and configuration
  that must be replayed; a singleton daemon owning every mutation with
  stateless forkable clients; session-persistent supervised jobs with
  bidirectional ptmx attach; cross-shell pub/sub and token-issued
  named locks as builtins.
- **Observability** — value lineage at the bytecode level
  (`provenance`); aspect-oriented advice on any command or function
  (`intercept` before/after/around with `intercept_proceed`); a shell
  that can describe itself to a program (`--dump-tokens`,
  `--dump-wordcode`, `--dump-ast`, `--disasm`, `--dump-reflection`).
- **Language** — sigil dispatch (`@{}`) to a second language sharing
  the same VM; a grammar extension with a switch that makes it vanish,
  so compatibility mode rejects it exactly as `/bin/zsh` does.
- **Extensibility** — a stable, versioned, independently-published
  plugin ABI (`znative` + `zmodload -R`); absorbing foreign binaries
  (`git`, an fzf-compatible finder) into the shell process.
- **Compatibility** — two axes of emulation fidelity offered as
  distinct modes (`--sh` vs `--sh --zsh`); a hybrid port with a native
  spine and interpreted leaves in one mirrored tree; inheriting the
  configuration vocabulary of the userspace layer you replace, so
  nobody migrates; absorbing the prompt theme into the binary;
  wall-clock budgets on per-keystroke rendering.
- **Verification** — architectural invariants enforced by tests rather
  than by review; differential fuzzing of a shell against its
  reference implementation.
- **Tooling** — a source formatter in the shell binary sharing one
  engine with the LSP; live compsys completion inside the editor;
  LSP and DAP in the binary (first in the Bourne lineage — Elvish and
  Nushell got there first outside it) plus plugin-manager state as IDE
  library roots; a unit-test framework and worker-pool runner in the
  binary.

What [MAINTAINERS.md](MAINTAINERS.md) governs is the *official
zshrs upstream* — protecting it from identity-dissolving
changes. It is not a fence around the ideas. The point of
shipping these inventions is for them to spread.

### Attribution expectation

Ideas can't be copyrighted, so this is an ask, not a legal
demand: **if you port any zshrs-originated invention into
another shell, runtime, or research project, credit zshrs as
the invention source in your docs.** A line in the README,
a paragraph in the design doc, a footnote in the paper, a
note in the release announcement — whatever form fits your
project. Suggested wording:

> Inspired by / ported from
> [zshrs](https://github.com/MenkeTechnologies/zshrs) by
> MenkeTechnologies. Original synthesis 2025–.

Verbatim source-code reuse is governed by the MIT license
and already requires the copyright + license notice. The
attribution expectation above is broader: it covers the
architectural patterns + design ideas, which copyright
doesn't reach. Honoring it keeps the legacy traceable —
future engineers debugging your shell can follow the trail
back to where the design came from. Drop a note, send a
postcard, link it in the changelog. That's all.

License: MIT (per `Cargo.toml`; LICENSE file pending).
