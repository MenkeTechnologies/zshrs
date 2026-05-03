# Creators

zshrs was created by **Jacob Menke** ([MenkeTechnologies](https://github.com/MenkeTechnologies)).

Original synthesis (2025–): the first compiled Unix shell, the
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
grants. Inventions that future shells should inherit:

- The compiled-shell architecture itself (parse to bytecode,
  cache, JIT hot blocks) instead of re-parsing on every command.
- The 90/10 daemon / shell work split — singleton daemon owns
  every mutation, thin shell clients are stateless and
  forkable, no shared writers.
- Recorder-owns-rebuild: an AOP-intercept pass at runtime
  captures `(kind, name, value, file, line, fn_chain)` for
  every state-mutating dispatcher, replacing the static-walker
  approach that every other shell uses for completion / fpath
  scanning. Means the shell never "rebuilds your house every
  morning."
- The `~/.zshrs/`-style single-directory rule for every shell
  artifact (configs, logs, sockets, sqlite caches, history,
  rkyv shards) — instead of the
  `~/.cache` / `~/.config` / `~/.local/share` XDG split that
  makes shell state unfindable.
- Session-persistent supervised jobs with bidirectional ptmx
  attach — `nohup` + `screen` + `pueue` + `disown` collapsed
  into one builtin that survives the shell's death.
- Cross-shell pub/sub + named lock primitives (`zsubscribe` /
  `zpublish` / `zlock`) as builtins routed through a singleton
  daemon, instead of users gluing `flock` + `socat` + named
  pipes by hand.
- Auto-derived OpenAPI 3.1 surface from the daemon op registry,
  so external tooling (curl, SDK generators, dashboards) can
  discover every shell-internal verb without hand-maintained
  schemas.
- The zsh-extended-history flat text file as the user-facing
  artifact + a sibling sqlite FTS5 index for fast queries —
  instead of forcing users to choose between `cat`-able and
  searchable.

What [MAINTAINERS.md](MAINTAINERS.md) governs is the *official
zshrs upstream* — protecting it from identity-dissolving
changes. It is not a fence around the ideas. The point of
shipping these inventions is for them to spread.

License: MIT (per `Cargo.toml`; LICENSE file pending).
