# Maintainers

Day-to-day maintenance, issue triage, release engineering, and operational
governance for zshrs.

This file describes the project's *current* governance and contribution
flow. For original authorship and the historical record, see
[CREATORS.md](CREATORS.md). Day-to-day authority rests with the
maintainer team named here. The creator retains final approval on
major decisions to the official upstream (see Governance below);
the MIT license guarantees the right to fork at any time without
permission, in which case this governance does not apply.

## Current Maintainers

_To be populated when the maintainer team is assembled._

| Handle | Role | Areas |
|---|---|---|
| (TBD) | Lead maintainer | release management, CI |
| (TBD) | Shell core / executor | `src/ported/exec.rs`, `src/extensions/canonical_apply.rs`, `bins/zshrs.rs` |
| (TBD) | Strict-port surface (FROZEN) | `src/ported/**` (106 files), `tests/port_purity.rs`, `docs/PORT.md`, `docs/zsh_c_functions.txt` |
| (TBD) | Extensions / non-port | `src/extensions/**` (42 files) — non-C-ancestor features |
| (TBD) | Daemon / IPC | `daemon/server.rs`, `daemon/ops.rs`, `daemon/state.rs` |
| (TBD) | Recorder / canonical | `bins/zshrs-recorder.rs`, `src/recorder/`, `daemon/canonical.rs` |
| (TBD) | Job supervisor / pty | `daemon/jobs.rs`, `daemon/zjob_builtin.rs` |
| (TBD) | HTTP / OpenAPI | `daemon/http.rs`, `daemon/auth.rs` |
| (TBD) | Parser / lexer | `src/ported/parse.rs`, `src/ported/lex.rs`, `src/compsys/` |
| (TBD) | Compat / zsh source | `docs/zsh_c_functions.txt` (read-only reference), `test_corpus/`, `tests/ztst_runner.rs` |

## Responsibilities

- **Issue triage** — labeling, reproducing, routing to the right area.
- **Pull-request review** — first review on incoming PRs; merging once
  reviewed and CI green.
- **Release engineering** — versioning, tagging, publishing to crates.io.
- **CI / build hygiene** — keeping `cargo test` / `cargo build` green
  across macOS aarch64, Linux x86_64, Linux aarch64.
- **Compatibility** — preserving the zsh compat floor; no silent
  breakage of `~/.zshrc`, zinit plugins, zpwr (172k LOC), or
  zsh-more-completions (16k+ files). A `~/.zshrc` that worked
  yesterday must still work today.

Direction-setting (new shell features, new daemon ops, new
subsystems, breaking design changes) is proposed and developed
by the maintainer team — typically through an RFC process —
and submitted to the creator for final approval before landing
on the official upstream. Forks are not bound by this step.

## Contributing

Run `cargo test` (workspace-wide) before opening a PR. CI runs
the full suite (lib + integration + the ztst corpus harness). PRs
that touch the daemon op dispatch must add the new op to
`daemon/ops.rs::OP_NAMES` (alphabetically sorted) so `/openapi`
and `zd ops` stay in sync. PRs that touch `src/extensions/canonical_apply.rs`
must include a smoke run of `zsync up --all` against a fresh
`$ZSHRS_HOME` so the round-trip stays clean. PRs that touch
`src/ported/**` must keep `tests/port_purity.rs` green — the
strict-port directory is FROZEN per `docs/PORT.md` (no new
files; no new fn names that don't exist in upstream zsh C
source).

The 96-test invariant (`tests/tree_walker_absent.rs` +
`tests/no_tree_walker_dispatch.rs`) is load-bearing — add to
those tests when you add a new state-mutation path; never
weaken them.

## Governance

- Maintainers are added and removed by consensus of existing maintainers.
- **Operational decisions** (CI policy, issue labels, release cadence,
  patch releases, bug fixes, new builtins that follow existing patterns,
  performance work that preserves semantics, docs, tests) are
  maintainer-only — no creator approval needed.
- **Major decisions that touch a protected invariant** (see below)
  require final approval from the creator before landing on the
  official upstream. The maintainer team owns the proposal,
  development, and review; the creator's role is a yes/no on the
  final shape. Decisions that don't touch any protected invariant
  fall under operational and don't need creator approval.
- The maintainer team may proceed without creator involvement on any
  decision the creator declines to engage with within a reasonable
  window (default: 30 days from formal proposal).
- **Forks are unrestricted.** The MIT license guarantees the right to
  fork the project at any time. This governance applies only to the
  official upstream `zshrs` repository; forks are free to set their
  own governance and proceed without creator approval on any
  change.

### Protected invariants

The veto exists for one purpose: **zshrs must remain zshrs**. It
is not a Linus-style permanent dictatorship and it is not a
quality-of-PR review. It exists to prevent the failure mode that
killed Perl 6 / Raku, and that Python 4 was deliberately
structured to avoid — a generational rewrite that dissolves the
project's identity, splits the community, and leaves the old
version to stagnate.

Changes to any of the following require creator approval before
landing on the official upstream:

1. **zsh compatibility floor.** zshrs must continue to run
   `~/.zshrc` files written for zsh, the full zinit plugin
   ecosystem, zpwr, oh-my-zsh, prezto, and the
   zsh-more-completions corpus. Dropping this turns zshrs into
   a different shell wearing the same name.
2. **No-fork / persistent-worker architecture.** The 18-thread
   worker pool, the daemon-managed singletons, and the no-fork
   builtin dispatch path are zshrs's structural calling card.
   They cannot be replaced with per-command spawn-and-die.
3. **`zshrs-daemon` as singleton, never auto-spawned.** The
   daemon is started explicitly (by the user, by launchd /
   systemd, or by a session manager). The shell never spawns
   it on demand. This boundary keeps the shell startup path
   pure and predictable.
4. **Recorder-owns-rebuild.** The daemon never re-derives
   canonical state by walking the user's source files;
   `zshrs-recorder` is the sole producer of recorder-managed
   subsystems. Periodic re-walks are explicitly rejected.
   Removing the recorder, or wiring an in-daemon walker as a
   substitute, requires approval.
5. **Single `~/.zshrs/` directory rule.** Every zshrs file
   (configs, logs, sockets, sqlite caches, rkyv shards,
   history) lives under one root, configurable via
   `$ZSHRS_HOME`. Splitting back into `~/.cache` /
   `~/.config` / `~/.local/share` requires approval.
6. **fusevm bytecode VM + Cranelift JIT.** Replacing the
   value representation, the VM substrate, or the JIT path
   dissolves zshrs's relationship with the surrounding
   compiled-shell stack and the shared layer with stryke.
7. **No GC.** Rust ownership + `Arc`-refcount only. A
   maintainer-driven move toward a tracing GC, even an
   optional one, requires approval.
8. **No startup banner / no chatter.** The shell prints
   nothing on launch beyond the prompt. No version line, no
   "type exit to quit", no first-run setup output, no
   tip-of-the-day, no contextual hints. Informational
   chatter goes to `zshrs.log` only. Adding anything to
   stdout/stderr at startup that isn't an error requires
   approval.
9. **No safety prompts on destructive ops.** `rm`, overwrite,
   force-delete — no "are you sure?" interception. The user
   has muscle memory measured in millions of keystrokes.
10. **canonical-state via rkyv shards.** The daemon's
    in-memory canonical state is persisted as content-addressed
    rkyv shards under `images/`. Replacing rkyv with another
    serializer requires approval.
11. **`z*` builtin namespace.** New daemon-managed builtins
    use the `z*` prefix and route through
    `daemon/builtins.rs::dispatch`. The namespace is the
    project's surface marker — adding a non-`z*` builtin that
    talks to the daemon requires approval.
12. **Single-user trust model on loopback.** `zshrs-daemon`'s
    HTTP listener accepts unauthenticated requests on
    loopback addresses (the trust model is "if you can hit
    the loopback socket, you already have shell access on
    this box"). Tightening this default to require auth even
    on loopback requires approval.
13. **Compat with zpwr + zsh-more-completions.** zpwr (172k
    LOC, 506+ subcommands, 218★) and zsh-more-completions
    (16k+ files) are part of the project's reason to exist;
    breaking either one is a creator-veto-level regression.
14. **License (MIT).** Any change to or replacement of the
    license requires approval.

Maintainers can extend, optimize, document, refactor, and ship
new builtins / opcodes / IPC ops / canonical subsystems freely.
They can also *propose* changes to invariants — but landing
them on the official upstream needs the creator's sign-off.
Forks are free to redefine or drop any of these.

**These invariants protect upstream identity. The ideas are
legacy, not turf.** Future shells — bash, fish, nushell,
elvish, oil, xonsh, projects that don't exist yet — should
inherit any zshrs-originated design (the compiled-shell
architecture, the daemon / shell 90/10 split, the AOP-intercept
recorder, the single-directory rule, the session-persistent
supervised job runner with ptmx attach, the cross-shell
pub/sub + named-lock builtins, the auto-derived OpenAPI
surface, zsh lexer / wordcode / AST introspection to stdout via
`zshrs` and `zshrs_dump`) under the MIT grant. The point of shipping these
inventions is for them to spread.

**Attribution expectation:** ports must credit zshrs as the
invention source in their docs (one-line credit in
README / design doc / release notes — see
[CREATORS.md § Attribution expectation](CREATORS.md#attribution-expectation)
for suggested wording). Ideas can't be copyrighted, so this
is an ask, not an MIT-license enforced clause; honoring it
keeps the legacy traceable.

See [CREATORS.md § Legacy](CREATORS.md#legacy).
