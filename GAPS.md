## Session 2026-04-27 — `man zshall` gap audit (verified against binary)

Probe: 47 constructs. Every entry below was verified by running zshrs (`./target/debug/zshrs -f -c '...'`) and comparing to expected zsh behavior. False positives the source-only audit produced (e.g. `${(j: :)arr}`, `${(t)var}`, `${(P)x}`, `<<<`, short-loop `for x in y; { ... }`, `repeat N ( ... )`, `zparseopts`) are NOT listed — they already work.

### Closed (this session, verified against binary)

- `${(f)str}` — split on newlines into array. Works (was always working; audit was wrong).
- `$argv` — array alias for positional params. `set -- a b c; echo $argv` → "a b c".
- `$EPOCHREALTIME` — sub-second epoch. Now emits `SECS.UUUUUU`.
- `[[ a -ef b ]]` — same-inode test. New `BUILTIN_SAME_FILE` (id 315) compares (dev, inode) via `fs::metadata`.
- `*(D)` glob qualifier — per-pattern dotglob. `expand_glob` activates `dotglob` when 'D' appears in the qualifier string.
- `typeset -Z N x=val` / `-L N` / `-R N` — width as a separate arg now parsed (the in-flag form `-Z5` was already working). Width applied at assignment time.
- `${(B)x}` — backslash-escape shell metas. New 'B' arm in BUILTIN_PARAM_FLAG (mirrors 'b').
- `${(z)str}` / `${(w)str}` — array-producing flags. Handler now returns `Value::Array` so `print -l ${(z)a}` splits one-per-line.
- `declare -g x=val` from inside a function — `-g` flag now opt-outs of the local_save_stack push, so the assignment binds at global scope and survives function exit.
- `time { compound; ... }` — new `BUILTIN_TIME_SUBLIST` (id 316) runs the sublist as a sub-chunk and prints elapsed wall-clock time.
- `{ try } always { finally }` — compile_zsh's `ZshCommand::Try` arm now compiles both blocks sequentially; finally runs unconditionally.
- `for var (a b c) cmd` and `for var (a b c) { ... }` — parse_for now handles the lexer-port quirk that emits the parens as a single String token (`\u{88}a b c\u{8a}`).
- `>(...)` output process substitution — process_sub_out now creates a real named pipe via mkfifo and forks a child that reads it. untokenize was missing OUTANGPROC → '>' mapping (causing the detection in compile_word_str to fail). Both fixed; `tee >(cat)`, `echo > >(cat)` work. `tee >(cat) >/dev/null` still silent (child's stdout-vs-redirect interaction edge case).
- `typeset -T VAR var [SEP]` — initial-bind only: takes current $VAR (or =VAL form), splits on SEP (default ":"), stores as array `var`. Bidirectional sync on subsequent assignments still requires a hook into set_variable; common idiom (`typeset -T PATH path`) works.

### Still open (requires deeper work)

- `RC_EXPAND_PARAM` option — `X${arr}Y` element-wise distribution requires changing array-in-string concat semantics; affects compile_word.
- `${(z)"literal string"}` — variable form `${(z)var}` works; literal-string form needs different compile path.
- `typeset -T` bidirectional sync — initial bind works; subsequent assignments to either side don't auto-sync (would require hooking every `variables.insert` site).
- History expansion (`!!`, `!$`, `^old^new^`) — `expand_history` is wired into `execute_script` but gated on `atty::is(Stream::Stdin)`. Works in interactive mode; correctly no-op in `-c` (where the audit claimed broken).
- `$RANDOM_FILE` — not actually a bug; mainline zsh also leaves it empty without `zmodload zsh/random`.

### Closed (this session, continued)

- `exec {fd}>file` — parser detects `{NAME}` followed by a redirop and pops it as varid. New `BUILTIN_OPEN_NAMED_FD` (id 317) opens the path with the right libc flags, dups to fd ≥ 10 via `F_DUPFD_CLOEXEC`, stores the fd number in `$varid`. Verified for read/write/append.
- `sched`, `echotc`, `echoti`, `getln`, `zpty`, `ztcp`, `zsocket` — routed through `host_exec_external` so they hit local handlers instead of "command not found".

### Sched + 6 stubbed builtins routed (this session)

`sched`, `echotc`, `echoti`, `getln`, `zpty`, `ztcp`, `zsocket` were defined as `builtin_*` handlers in exec.rs but never reached because they're absent from fusevm's `shell_builtins::builtin_id` table — script-level dispatch fell through to external command spawn and "command not found". `host_exec_external` now intercepts these names before the OS-level exec attempt and routes to the local handler. Each handler's behavior varies (sched + getln work properly; zpty/ztcp/zsocket are still skeletons), but no more "command not found".

### Grammar (parser-shape gaps)

- `{ body } always { finally }` — try/finally block. Parser doesn't recognize `always`; entire construct silently no-ops, neither body nor finally runs. (zshmisc/Complex Commands.)
- `time { compound; ... }` — compound-form `time` swallows output (only `time simple-cmd` works). Parser doesn't drive sub-block.
- `for var (a b c) cmd` and `for var (a b c) { ... }` — paren-list short-for. Parser misparses; loop body never executes. Note: `for var in a b; { ... }` curly form DOES work.
- `select var in list; do body; done` — body output is suppressed. Parser produces something but the prompt/runtime path is broken.
- `exec {fd}>file` — named-fd LHS allocation. zshrs parses `{fd}` as a literal filename (`No such file or directory`). zsh allocates a fresh fd ≥10 and binds it to `$fd`.

### Parameter expansion flag gaps

- `${(z)str}` — shell-word splitting of `str` honoring quoting. Currently silent.
- `${(f)str}` — split on newlines into array. Currently silent.
- `${(B)x}` — backslash-escape spaces and metas. Currently passes value through unchanged (no escaping).

### Special parameters

- `$argv` — array alias for positional params. zshrs leaves it empty even after `set -- a b`. zsh: `argv` is the same as `*` / `@`.
- `$EPOCHREALTIME` — sub-second epoch (zsh/datetime). zshrs: empty.
- `$RANDOM_FILE` — entropy source path for `$RANDOM`. zshrs: empty.

### `typeset` flag gaps

- `typeset -T VAR var ":"` — tied scalar/array (e.g. PATH↔path). zshrs accepts the syntax but doesn't actually tie; reading `$var` returns empty after `VAR=a:b:c`.
- `typeset -Z N x` — zero-pad numeric to width N. Width ignored.
- `typeset -L N x` — left-justify to width N (truncate/pad). Width ignored.
- `typeset -R N x` — right-justify to width N. Width ignored.
- `declare -g x=val` from inside a function — global scope flag ignored; var stays function-local. Outer scope sees no value.

### Test operator gaps

- `[[ a -ef b ]]` — same-inode test. Silent (treats as false / lexer rejects). zsh: 0 if same file, 1 otherwise.

### Glob qualifier gaps

- `(D)` — include dotfiles in match. `*(D)` returns no results in `/tmp` even when dotfiles exist there.

### Expansion gaps

- `RC_EXPAND_PARAM` — `X${arr}Y` should produce `XaY XbY XcY` element-wise. zshrs joins as `Xa b cY` (treats array as single space-joined scalar in concat context).

### Process substitution

- `>(...)` — output process substitution. `echo data > >(cat)` is silent. Input form `<(...)` works.

### Builtins / runtime constructs not yet probed but flagged for follow-up

These are known stubs identified by `grep` against `src/exec.rs`; they need targeted probes before being closed:

- `bindkey`, `echotc`, `echoti`, `getln`, `sched`, `ttyctl`, `vared`, `zcompile`, `zformat`, `zmodload`, `zprof`, `zpty`, `zregexparse`, `zsocket`, `zstyle`, `ztcp` — many are present as builtin handlers but with stub bodies. Each needs a behavioral probe to confirm where the line is between "runs but does the wrong thing" vs "registered but unimplemented."
- `zsh/cap`, `zsh/clone`, `zsh/curses` (full), `zsh/db_gdbm` (operations), `zsh/files` (chown/chmod/chgrp), `zsh/mapfile`, `zsh/nearcolor`, `zsh/newuser`, `zsh/private`, `zsh/zftp` — module surfaces are stubbed.

### History expansion

- `!!`, `!$`, `!*`, `!N`, `!-N`, `!?str`, `^old^new^` — interactive history-expansion lexer not wired into the main shell flow. The history file is captured and `history` builtin works; the `!` event-designator lexer pass before parse is missing. (In `-c` mode this is academic; matters for interactive use.)
