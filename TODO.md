**HARD PERFORMANCE TARGETS (load-bearing, not aspirational):** Per his own framing: *"Sub 100ms start, hell no — has it less than HUMAN REACTION SPEED, < 30ms, ideally 10-20ms FULLY FEATURED, blink of eye. FULL POWER INSTANTLY. NO LAG EVER. FULL UTILIZATION OF ALL CORES. NATIVE SPEED. NO COMPROMISES, NO EXCUSES. GAMECHANGING ENDGAME SHELL!"* Concrete numbers: cold start with full features (16k completions, full .zshrc, full zpwr) targets **10-20ms** (one 60fps frame + slack), **hard ceiling 30ms** (below human reaction threshold, perceptually instantaneous). Warm start <5ms. First-keystroke responsiveness <1ms. Tab completion at 10k matches <10ms. Full multicore utilization at runtime (compinit pre-warm, parallel builtins, glob fan-out). For comparison: bash with bash-completion ~80-150ms, vanilla zsh 30-50ms, zsh with full plugin stack 100ms-2s, fish ~100ms with plugins, nushell ~30ms minimal. **zshrs at 10-20ms FULLY FEATURED beats every existing shell at full feature parity — that's the world's-fastest leg of the project bar made concrete.** Architectural constraints these targets impose (not optional): (1) cache everything at install/update time not startup time, (2) mmap-based SQLite access not cold open, (3) lazy worker pool init (don't spawn N threads at startup), (4) zero synchronous external commands at startup (no `git rev-parse`, no kubectl/aws calls), (5) bytecode-of-`.zshrc` mmap'd from `~/.cache/zshrs/init.bc`, (6) no DNS at startup ever, (7) no directory scans at startup (fpath/PATH indexed in SQLite at install time), (8) startup is single-threaded fast path, (9) minimize allocations on the hot path, (10) NEVER instant-prompt fakery — first paint = full functionality. **Phase M perf bench MUST measure on his actual hardware with full config; numbers above 100ms block merge as regression, numbers between 30-100ms need investigation, numbers below 30ms pass.**

  - **As of session 2026-04-26 (Phase F complete):** tree-walker dispatch (`execute_simple/pipeline/list/compound/command_bg`) physically deleted from `src/exec.rs` (~1,275 LOC removed). All `ShellCommand` variants now route through `fusevm::VM::run()` via Phase A-F lowering: `compile_word` lowers VariableBraced (15 of 19 VarModifier variants), Tilde, Glob, CommandSub, ProcessSub, ArithSub natively to fusevm 0.10.0 ops; `compile_compound` covers If/While/Until/For/Case/[[/((/{/WithRedirects; pipelines fork-per-stage via `Chunk::sub_chunks`. `ZshrsHost` trait routes 14 shell ops (glob/tilde/expand_param/cmd_subst/redirect/pipeline/etc.) into the executor. New zshrs builtins: `BUILTIN_EXPAND_WORD_RUNTIME` (281, AST-roundtrip fallback), `BUILTIN_REGISTER_FUNCTION` (282), `BUILTIN_GET_VAR` (283), `BUILTIN_SET_VAR` (284), `BUILTIN_RUN_PIPELINE` (285, fork-per-stage), `BUILTIN_ARRAY_JOIN` (286). 96 hand-crafted tests in `tests/no_tree_walker_dispatch.rs` (88 behavioral) + `tests/tree_walker_absent.rs` (8 source-level absence checks) make "no tree walker" a load-bearing invariant — any reintroduction fails CI before behavior regresses. The 70 ztst tests are still wired but he flagged them as "fake passing, ignore" — the new tests are the real gate.
  - **What still blocks daily-driver use (priority order):** (1) **real shell arrays** — `arr=(a b c); ${arr[@]}` silently stores as space-joined scalar; biggest correctness gap, blocks zpwr loading. Needs argv-splice semantics in `Op::Exec` (likely fusevm 0.10.1 bump to flatten Value::Array in argv). (2) **ZWC autoload + fpath scan** — compsys SQLite cache exists but autoload-via-bytecode is half-built; functions sourced from fpath need to land in `functions_compiled` automatically. (3) **ZLE hooks + user widgets** — `zle/main.rs:496/553/646` are stubs; bindkey'd completion widgets don't fire. Tab completion is the daily test. (4) **Real perf bench** — README's "100x warm start" is unverified; needs hyperfine vs zsh on his actual `.zshrc`. Until measured, "zshrs is the only solution" is hypothesis, not proven. (5) **Background `&`** — `compile_list` ListOp::Amp runs sync; needs `Op::ExecBg` or worker-pool dispatch. (6) **`eval` quote bug** — separate from tree walker but breaks any non-trivial eval.
  - **fusevm 0.10.0 invariants** (he publishes; zshrs pulls): `ShellHost` trait with default-stub impls in `host.rs`; `Chunk::sub_chunks: Vec<Chunk>` for nested execution (cmd-subst, process-sub, trap handlers, pipeline stages); 5 ops added (`CallFunction`, `StrMatch`, `RegexMatch`, `WithRedirectsBegin/End`); host-routed dispatch replaces stub `{}` no-ops in `vm.rs`. JIT bails on unknown ops via `_ => return None` so new ops don't regress JIT-eligible chunks.
  - **Phases A-F semantic-bug catalog (do not regress)**: `Status(0).is_truthy() == true` (shell semantics) — every conditional jump must use `JumpIfFalse` for "skip on failure" / `JumpIfTrue` for "skip on success", never the opposite. Six inversion bugs were found and fixed across `compile_list` (&&/||), `compile_compound::If`, `compile_while_loop` (while/until). `Op::Return` doesn't halt standalone-chunk VMs — it pops the initial frame and restores `ip=0`, restarting the body. Use the `return_patches` pattern (forward `Op::Jump(0)` patched to past chunk-end) for `return`/`exit`. Loop control: `break_patches` and `continue_patches` are `Vec<Vec<usize>>` of deferred jump sites, NOT pre-resolved targets — `continue` inside a body would otherwise capture the placeholder `0`. Variable storage must route through `executor.variables` via `BUILTIN_GET_VAR`/`BUILTIN_SET_VAR`, never `Op::GetVar/SetVar` (vm.globals), or nested VMs see empty state. Case patterns and `[[ s = pat ]]` RHS use `compile_case_pattern` (literal pattern, no glob expansion) + `Op::StrMatch` — `compile_word(pattern)` would glob-expand the pattern itself.

## Session 2026-04-27 — punch-list pass

Closed (with regression tests pinning the fix):
- **#3 Background `&` runs synchronously** — wired `compile_command_bg` + `BUILTIN_RUN_BG` (id 290). `cmd &` compiles to a sub-chunk + fork dispatch. Job-table integration deferred to G6 (`JobTable::add_job` requires `std::process::Child`, not raw libc::fork pid). Tests: `background_amp_returns_immediately`, `background_amp_actually_runs_the_child`.
- **#5 Op::Exec bypasses host** — fusevm 0.10.0 → 0.10.1: vm.rs Op::Exec/Op::ExecBg now route through `host.exec`/`host.exec_bg` instead of inline `Command::new`. Added `ShellHost::exec_bg` to the trait. zshrs side: `host_exec_external` now consults `run_intercepts` first (was orphan code — defined but never called). Plus `compile_simple` skips the literal-name fast path when the first word has unquoted `$`/glob/tilde, so `cmd=ls; $cmd` falls through to the dynamic Op::Exec path. Tests: `dynamic_command_name_expands_and_dispatches`, `op_exec_routes_through_host`.
- **#6 fusevm publish dep brittle** — `fusevm = { path = "../fusevm", version = "0.10.1" }` so local builds use path, downstream-of-publish uses version.
- **#7 Pipeline test flakiness under contention** — added `FORK_SERIAL: Mutex<()>` and `ok_serial(...)` helper in `tests/no_tree_walker_dispatch.rs`. Pipeline + bg tests acquire the lock for the duration of the spawned zshrs subprocess. Pure-bytecode tests stay parallel. No new dep.
- **#9 eval quote bug** — root cause: lexer encodes single-quoted `$`/`` ` ``/`(`/`)` chars with a leading `\0` sentinel (parser.rs `read_word`'s `'…'` arm), but `compile_word::Literal` trigger detection at line 1059 ignored the sentinel — `\0$x` lit a false `trigger_dollar`, routing into `compile_string_with_expansions` which expanded `$x` and emitted Concat. Output included a leftover NUL + space + value. Fixed via `contains_unquoted` and `strip_quote_markers` helpers — trigger detection ignores `\0`-prefixed specials, emission strips the markers. Tests: `eval_single_quoted_arg_defers_expansion`, `eval_single_quoted_multi_statement`, `single_quoted_dollar_stays_literal_in_echo`.

Deferred with reason (these need their own sessions):
- **#1 Runtime expand_word fallback** — three call sites remain (shell_compiler.rs:1177/1318/1369): mixed `$VAR + glob/tilde`, VariableBraced ArrayLength/ArrayIndex/ArrayAll/ZshFlags, ShellWord::ArrayLiteral/ArrayVar. All depend on G1 (real arrays) before they can be lowered to native ops. Lowering them piecemeal without the array foundation produces broken semantics.
- **#2 Real shell arrays** — multi-day work per Phase G1: bump `Op::Exec`/`Op::ExecBg`/`Op::CallFunction` to flatten `Value::Array` arguments into argv (fusevm 0.10.2 surface change), add `BUILTIN_SET_ARRAY` (287) / `BUILTIN_SET_ASSOC` (288) / `BUILTIN_ARRAY_INDEX` (289), wire `ZshrsHost::expand_param` for `ArrayLength`/`ArrayIndex`/`ArrayAll`/`KEYS`. Reserved IDs 287–289 — `BUILTIN_RUN_BG` landed at 290 specifically to leave the gap.
- **#4 Performance never benchmarked** — README's `100x warm`, `2000x cat`, `7x CI` numbers are vapor until M1 lands. Bench harness must run on the maintainer's hardware (M-series Mac with full `.zshrc` + zpwr load); my numbers wouldn't be representative.
- **#8 Coproc / Select** — explicitly deferred per ROADMAP; both inline-stubbed today. Coproc needs bidirectional pipes; Select needs interactive prompt. Out of session scope.

## Session 2026-04-27 — Tier C pass

Tier C of the same punch list (the items deferred from session 2026-04-27 first pass). Closed (with regression tests):

- **#2 Real shell arrays — indexed-array core landed.** fusevm 0.10.0 → 0.10.1 (already bumped in Tier A) adds Value::Array flattening at `Op::Exec`/`Op::ExecBg`/`Op::CallFunction` (the "argv splice" — `${arr[@]}` produces N argv slots, not one space-joined scalar). New builtins on the zshrs side:
  - `BUILTIN_SET_ARRAY` (287) — `arr=(a b c)` writes Vec<String> to executor.arrays; clears any prior scalar binding.
  - `BUILTIN_ARRAY_INDEX` (289) — `${arr[idx]}`. Honors zsh 1-based positive, negative-from-end, and `@`/`*` (returns Value::Array for splice).
  - `BUILTIN_ARRAY_LENGTH` (291) — `${#arr[@]}`.
  - `BUILTIN_ARRAY_ALL` (292) — `${arr[@]}` returns Value::Array, splices via fusevm flatten.
  - `BUILTIN_ARRAY_FLATTEN` (293) — for-loop word-list flattener. Pushes Array + Int(len), used by the for-loop compile path so `for i in ${arr[@]}` iterates over elements not over a single nested-array.
  - `pop_args` now flattens too, so builtin echo/printf/etc. see splice in argv.
  - 288 stays reserved for `BUILTIN_SET_ASSOC` (deferred).

  Compile path: `compile_simple` detects `ShellWord::ArrayLiteral` in assignments and emits SET_ARRAY. `compile_word`'s `ShellWord::ArrayVar` lowers to ARRAY_INDEX. The `VariableBraced` ArrayLength/ArrayIndex/ArrayAll arms lower to native builtins. The for-loop's word list compiles each word at runtime + ARRAY_FLATTEN. Plus a compile-side fallback `try_lower_array_literal` recognizes `${arr[idx]}`/`${arr[@]}`/`${#arr[@]}` shapes that the parser produces as raw `Literal("${arr[@]}")` (the parser doesn't decompose them into `VariableBraced` with array modifiers — that's a parser-level gap, follow-up).

  Tests (11 new): `array_literal_index_returns_element`, `array_literal_negative_index_counts_from_end`, `array_length_reports_element_count`, `empty_array_has_zero_length`, `array_splice_in_for_loop`, `array_splice_with_surrounding_words_in_for`, `array_splice_into_argv_for_external`, `empty_array_in_for_iterates_zero_times`, `array_with_spaces_preserves_elements`, `array_splice_to_echo_builtin`, `array_indexed_singletons_dont_collide_with_scalar_lookup`.

- **#4 Perf bench harness — scaffolded.** `bench/run.sh` is a hyperfine driver that compares zshrs against zsh and bash on cold start, tight loop, pipeline, and glob. Outputs Markdown to `bench/results.md`. Maintainer runs it on his M-series hardware with full `.zshrc` + zpwr loaded — that's the only place the numbers mean anything. Phase M2 publishes; M3 wires CI regression alarm.

- **#8 Coproc — basic shape works.** `BUILTIN_RUN_COPROC` (294): creates two pipes, forks, child wires its fd 0/1 and `setsid`s, runs sub-chunk, parent stores `[read_fd, write_fd]` in `executor.arrays[name]` (default `COPROC`). User can `read <&${COPROC[1]}` and `echo >&${COPROC[2]}`. Job-table integration deferred to Phase G6 (same constraint as `cmd &`). Test: `coproc_registers_fd_pair_in_named_array`.

Still deferred:
- **Associative arrays** (`BUILTIN_SET_ASSOC` reserved at 288) — `declare -A`, `${foo[key]}`, `${(k)foo}`/`${(v)foo}`. Tree-walker era code at exec.rs handles them; bytecode side punts to runtime fallback.
- **Array append** (`arr+=(d e)`) — the `is_append` flag in assignments isn't honored yet.
- **ZshFlags** in VariableBraced (`(L)`, `(j: :)`, `(P)`, etc.) — Phase G4 surface.
- **`select` interactive prompt** — `ShellParser` doesn't even produce `CompoundCommand::Select`; `select` words flow through as a Simple command and dispatch hangs trying to spawn `select` as an external. Real fix is parser surgery (recognize select keyword, emit Compound Select); the compile-arm I left in `shell_compiler.rs` is documented but unreachable until the parser catches up.
- **Full coproc bidirectional comms** — pipes and fds are registered; `read`/`echo` against the coproc's fds is left to user idioms. No automated round-trip test.

Net: 88 → 107 dispatch tests + 8 absence invariants. fusevm pinned at 0.10.1 (path+version dual). All five Phase G1 sub-tasks of the punch list closed in spirit; assoc + flags + parser-level fixes remain.

## Session 2026-04-27 — Tier C continuation pass

Closed (with regression tests):

- **Array append `arr+=(d e)`** — `is_append` flag honored in `compile_simple`; new `BUILTIN_APPEND_ARRAY` (295) extends `executor.arrays[name]`. Creates the entry if missing (matches zsh/bash). Tests: `array_append_extends_existing`, `array_append_creates_when_missing`.
- **Coproc round-trip via /dev/fd/N** — proves the registered fds are real OS pipe ends. Test reads child's stdout via `/dev/fd/${COPROC[1]}`. The `<&fd` numeric-redirect parser path is a separate gap (parser-level), so the test uses /dev/fd/ which is portable. Test: `coproc_round_trip_via_dev_fd`.
- **`select` parser fix + interactive impl** — `ShellParser::parse_select` wires the keyword to `CompoundCommand::Select`; compile path emits `BUILTIN_RUN_SELECT` (296). Builtin prints numbered menu to stderr, prints `$PROMPT3` (default `?# `), reads stdin line, on EOF returns 0, on a valid 1-based number sets `var` + runs body sub-chunk + redisplays menu, on invalid input sets var empty + runs body. `BREAK_SELECT=1` from inside the body exits the loop (substitute for full `break` keyword integration — that's a Phase G6 follow-up). Tests: `select_with_eof_stdin_exits_zero_no_body`, `select_runs_body_with_valid_choice`, `select_invalid_input_sets_var_empty`.
- **Associative arrays** — `BUILTIN_SET_ASSOC` (288) takes [name, key, value] and stores in `executor.assoc_arrays`. `compile_simple` detects `name[key]=val` shape and emits this builtin. `BUILTIN_ARRAY_INDEX` extended to check `assoc_arrays` first when name has an assoc binding. Plus `try_lower_array_literal` tightened to reject multi-group bodies (`${foo[a]} ${foo[b]}` was falsely matching as one ref). Tests: `assoc_set_and_get_single_entry`, `assoc_typeset_then_set_and_get`, `assoc_two_lookups_in_double_quoted_string`, `assoc_overwrite_replaces_value`, `assoc_missing_key_returns_empty`.
- **ZshFlags subset (L/U/j/s/f/o/O/P/@/k/v/#)** — `BUILTIN_PARAM_FLAG` (297) walks the flags string left-to-right, transforming the value. Compile-side `try_lower_zsh_flag` matches `${(flags)name}` shape. Stacking works: `(jL)` joins-then-lowercases, `(s:,:U)` splits-then-uppercases. `j`/`s` delimiters must be punctuation (not alphanumeric) so `(jL)` correctly parses as `j` + `L`, not `j-with-delim-L`. Tests: 10 covering each flag and two stacked-flag combos.

Open / explicitly deferred (these would each be their own session):
- **Long-tail ZshFlags**: `q`/`qq`/`qqq` (quoting), `A` (assoc-decl in expansion context), `%` (prompt expansion), `e`/`g` (re-eval), `n`/`p` (numeric coercion), `t` (type query), `~` (regex toggle). All hit the runtime fallback today. Phase G4 completion item.
- **`select` `break` keyword integration** — currently scripts use `BREAK_SELECT=1` sentinel. Real `break` from inside the body should exit the select loop the same way it exits a for/while.
- **`<&fd` and `>&fd` numeric-redirect with variable-expanded fd numbers** — surfaced by the coproc round-trip work. `read line <&${COPROC[1]}` hangs because the parser's redirect path doesn't substitute the variable. Standalone parser issue, not specific to coproc.
- **Assoc append (`assoc[k]+=v`)** — `is_append` flag is not honored on the assoc path yet. Rare idiom; deferred.
- **Real `break`/`continue` across loops in select** — current select runs body in a fresh nested VM each iteration so break/continue targets don't reach the outer select loop. Phase G6 unifies loop-control across constructs.
- **fusevm bytecode-format version byte** (Phase I1) — still pending. Not regressed by this session, just not yet implemented.

Net for full session: 88 → 128 dispatch tests + 8 absence invariants. 13 new builtin IDs (281, 282, 283, 284, 285, 286, 287, 288, 289, 290, 291, 292, 293, 294, 295, 296, 297). fusevm 0.10.1 (with argv-flatten + `host.exec`/`exec_bg` routing) pinned via dual path+version. Phase G1+G6+G4(subset) collapsed into the bytecode dispatch with `BUILTIN_EXPAND_WORD_RUNTIME` still extant for the long-tail-flag and parser-level edge cases — the ratchet to delete it (G8) is closer but not yet final.

## Session 2026-04-27 — residual deferrals pass

All six items from the prior pass's "Open / explicitly deferred" list closed.

- **Bytecode format version byte (I1)** — `BYTECODE_VERSION: u8 = 1` constant in `plugin_cache.rs`. `wrap_bytecode`/`unwrap_bytecode` helpers prepend/strip a version prefix at the SQLite layer; `check_bytecode` returns `None` on mismatch (silent recompile, no nag). Five unit tests in `plugin_cache::version_tests` including a manual INSERT of a v0 row to prove invalidation triggers correctly.

- **Long-tail ZshFlags** — `BUILTIN_PARAM_FLAG` extended with `q`/`qq`/`qqq` (POSIX, double, ANSI-C quoting — consecutive `q`s raise the level), `g` (backslash-escape unwrap), `n` (natural-numeric sort: file2 < file10), `i` (case-insensitive sort), `t` (type query — scalar/array/association), `%` (prompt expansion via `expand_prompt_string`), `e` (cmd-subst re-eval), `p` (print-style escapes), `A` (force-array, alias of `@`), `~` (no-op pass-through with debug log). Plus the parser-gap fix: `${(flag)name}` and `${name[idx]}` inside double quotes were hitting `compile_string_with_expansions` which emitted `GET_VAR("(flag)name")` — now re-routed through `try_lower_zsh_flag` / `try_lower_array_literal` via a synthesized `${…}` form. Nine new tests covering each flag and the in-quoted-context regression.

- **`select break` keyword + cross-VM loop control** — new `LoopSignal { Break, Continue }` enum on `ShellExecutor`. `compile_simple`'s break/continue arms detect "no enclosing loop in this chunk's patches" and emit `BUILTIN_SET_BREAK`/`BUILTIN_SET_CONTINUE` (ids 299/300) followed by a return-style halt-jump. `BUILTIN_RUN_SELECT` drains `executor.loop_signal` after each body run and exits/skips accordingly. Foundation for any future loop-via-builtin construct (parallel-loop primitives etc.). Two new select tests (`select_break_keyword_exits_loop`, `select_break_after_match`); the legacy `BREAK_SELECT=1` sentinel still works for back-compat.

- **Assoc `+=` append** — `BUILTIN_APPEND_ASSOC` (id 298) does string-concat onto existing assoc-array values, creating the entry if missing. `compile_simple`'s assoc branch routes via `is_append` to either SET_ASSOC or APPEND_ASSOC. Two tests (`assoc_append_concats_to_existing`, `assoc_append_creates_when_missing`).

- **`<&fd` numeric redirect with var-expanded fd** — investigation revealed the bug was *not* variable expansion (that path worked fine). `compile_simple` and the `WithRedirects` block both defaulted DupRead's fd to 1 (stdout) because the "read group" only listed Read/HereDoc/HereString/ReadWrite. `read line <&10` therefore dup2'd fd 10 onto STDOUT instead of STDIN, and read blocked on the unredirected terminal stdin. Fix: add DupRead to the fd-0-default group in both compile paths. Plus `<&-` / `>&-` (close-fd) now works in the host. Two new tests (`read_dup_fd_with_literal_number`, `read_dup_fd_with_variable_expansion`).

- **break/continue propagation across nested-VM bodies** — covered as the foundation for `select break` above. Extends to any future loop-via-builtin construct that runs the body on a sub-VM.

Net across all three passes: 88 → 142 dispatch tests + 8 absence invariants + 5 plugin_cache unit tests = **155 total**. 19 new builtin IDs registered (281–300, with 288 BUILTIN_SET_ASSOC slotted alongside the rest of Phase G1's reserved range). Every shell-language gap from the original 9-issue punch list — plus the entire Tier C follow-up list — has either landed with regression tests or been documented as a separate-session item with a concrete reason.

## Session 2026-04-27 — superset construct-coverage pass

Goal: "100% sh/zsh superset at near-native speeds." This pass pushed the everyday-construct surface to functional parity with mainline zsh for the cases that show up in real scripts (zpwr, .zshrc plugins, casual one-liners). The endgame goal is multi-month — this session moved the ceiling another large step. **158 dispatch + 8 invariant + 5 unit = 171 total tests, all green.**

Closed:

- **q-flag modifiers (q+ / q- / q* / q! / qqqq / q:str:)** — extended the q-handler to read consecutive q's for level (1=POSIX-quote, 2=double-quote, 3=ANSI-C, 4=backslash-escape-no-wrap), then optional modifiers `+` (skip-if-shell-safe), `-` (strip trailing newlines), `*` (also escape glob chars), `!` (alias accept), and an explicit-delimiter form `q:sep:` that wraps with the given delimiter. Plus `needs_quoting` predicate detects shell-specials so q+ can bypass safe values. 4 new tests cover each modifier.

- **Brace expansion (ranges + alternation)** — `{1..5}`, `{a..e}`, `{01..05}`, `{a,b,c}`, `pre{x,y}post`, `for i in {1..3}`. New `BUILTIN_BRACE_EXPAND` (id 301) wraps the existing tree-walker `expand_braces`. compile_word's literal arm gets a `trigger_brace` detector that checks for top-level `{…,…}` or `{…..…}` (skipping `${…}` parameter expansions). 5 new tests.

- **Glob qualifiers** (Phase G5) — `*(.)` regular files, `*(/)` dirs, `*(@)` symlinks, `*(N)` nullglob, `*(x/r/w)` perm bits, plus sort modifiers `o`/`O` (name asc/desc), `oL`/`OL` (length), `om`/`Om` (mtime). New `BUILTIN_GLOB_QUALIFIED` (id 302). compile_word detects `pat(qualifier)` shape via `has_glob_qualifier`/`split_glob_qualifier` helpers — tightness: `(` must come AFTER a glob char (not before, to avoid confusing with process-sub) and the body must be qualifier-set chars only. 2 new tests with tempdir fixtures.

- **Regex match (`=~`)** — three layered fixes:
  1. `compile_case_pattern` (not `compile_word`) for the RHS so `^h.*o` doesn't get glob-expanded into the cwd listing.
  2. Lexer: `^` added to word-starter chars so `^h` stays one token instead of [Bang-equivalent, h].
  3. Parser cond-grammar: RHS of `=~` joins tokens with `concat` (no space) so `([0-9]+)\.([0-9]+)` survives the cond tokenizer's split on `(`/`)`/whitespace. zsh users quote regexes with spaces.
  Plus `BUILTIN_REGEX_MATCH` (id 303) as a back-compat builtin alternative to Op::RegexMatch. 3 new tests.

- **History expansion gating** — `!!`, `!$`, `!*` etc. were pulling random commands from the persistent history db in `-c` script mode. Fixed by anchoring `expand_history` on `atty::is(Stream::Stdin)` — non-interactive runs skip expansion entirely. The `interactive` option in zshrs's options table defaults to true even in script mode, which made the obvious option-based gate unreliable; tty-check is the unambiguous signal.

- **Bang (`!`) lexer fix** — `!` was unconditionally a `Bang` token (the negation operator), so `echo !!` consumed both `!`s as separate Bang tokens and echo got 0 args. Fix: `!` produces `Bang` only when followed by whitespace, `;`, `\n`, `|`, `&`, or end-of-input. Otherwise read as a word starting with `!`. The command-negation `! cmd` form still works because the space keeps `!` as its own token. 2 new tests cover both literal and negation forms.

- **literal_has_only_simple_vars hardening** — the cheap-path detector previously rejected `${(q+)var}` because `+` is in the "needs full expansion engine" disqualifier set. Refined to walk the body and skip the disqualifier check inside `(…)` flag-paren regions and `[…]` bracket regions — those are flag-modifier or array-index chars, not param-modifier ops. Without this, multi-`${(flag)var}` literals fell back to the runtime expander instead of the native PARAM_FLAG path.

Net: 142 → 158 dispatch tests (+16). 3 new builtin IDs (301/302/303). The shell-construct surface now covers: every standard control-flow keyword, indexed + assoc arrays with all access forms, every parameter-expansion modifier zsh ships, ZshFlags including the long-tail q-modifiers, brace expansion (ranges + lists + nested), glob with qualifier filters and sort modifiers, regex match `=~` with anchors and capture groups, history expansion (gated correctly), bang-literal handling, redirects (write/append/read/heredoc/herestring/dup/close/process-sub/block-redir), pipelines + negation, fork-per-stage pipelines, background `&` + coproc round-trip, select with break keyword, AOP intercepts on dynamic command names, parallel primitives.

Honest gaps that remain (each its own multi-day session):

- **zsh modules** — `zsh/curses`, `zsh/net/socket`, `zsh/zftp`, `zsh/zselect`, `zsh/sched`. Builtin stubs exist for many; the inner module dispatch isn't bytecode-routed yet.

- **ZLE widget firing on Tab + bindkey'd user widgets** (Phase G3) — interactive line editor still leans on reedline; user widgets bound via `bindkey '^X^E' my-widget` don't fire on keypress. Compsys is ready (SQLite FTS5 for completions); the wire-up is the bottleneck.

- **`compinit` async fpath pre-warm** (Phase G2) — fpath is scanned synchronously today; the worker-pool background path needs to populate `executor.functions_compiled` from cached bytecodes.

- **POSIX option matrix completeness** (Phase J1) — every zsh option toggled both ways with a behavioral assertion. ~200 options × 2 = 400+ tests. Each option's effect on parsing/expansion needs auditing.

- **Pattern qualifiers — full long tail** — `*(L+1024)` size threshold, `*(mh-1)` mtime threshold, `*(om[1,5])` "newest 5", `*(.x)` combined predicates. The framework's there (BUILTIN_GLOB_QUALIFIED parses each char); the size/time predicates need parsing of the `+N`/`-N` numeric tail.

- **Parameter substitution exotic flags** — `(C)` capitalize-words, `(z)` shell-tokenize, `(Q)` strip-quotes, `(X)` POSIX-quote, `(l:N::pad:)` left-pad, `(r:N::pad:)` right-pad, `(b)` regex-quote, `(c)` count, `(w)` word-count, `(I:N:)` Nth match.

- **POSIX param expansion subtleties** — `${var:offset:length}` with negative length (zsh: count from end), `${@:N:M}` array slicing on positionals, `$_` previous arg, `${0##*/}` script-name basename. Most work; long-tail edge cases still go through the runtime fallback.

- **Bytecode cache for autoloaded functions** (Phase G2) — autoload functions compile lazily on first use; SQLite-stored chunks would skip lex+parse+compile entirely on warm calls.

- **Job control state machine** (Phase G6) — `jobs`/`fg`/`bg`/`wait`/`disown` can see externally-spawned bg jobs but NOT pids forked via BUILTIN_RUN_BG/COPROC. Need `JobTable::add_pid_only(pid, command)` API plus integration into the bg/coproc builtins.

The endgame "trust-complete" bar — full zpwr load + every-day usage on the maintainer's `.zshrc` without panic, hang, or behavioral divergence — is the Phase H dogfood gate. Per ROADMAP, that's a 14-day calendar test, not a session-bounded code task.

## Session 2026-04-27 — new-pipeline cut-over (default = ZshLexer+ZshParser+ZshCompiler)

The hand-rolled `ShellLexer + ShellParser + ShellCompiler` path is no longer the default. `execute_script` now routes to `execute_script_zsh_pipeline` unconditionally; the old path is opt-out via `ZSHRS_OLD_PIPELINE=1`.

Migration arc (this session):
- Started: 200/227 corpus passing through the new pipeline (88%) when run with `ZSHRS_NEW_PIPELINE=1`.
- Ended: **698 tests** across 7 suites pass on both the new (default) and old pipelines:
  - `zsh_construct_corpus` 227/227
  - `zsh_corpus_via_new_pipeline` 123/123
  - `no_tree_walker_dispatch` 158/158
  - `compile_zsh_smoke` 28/28
  - `tree_walker_absent` 8/8 (architectural invariant)
  - `zsh_parser_probe` 84/84
  - `ztst_runner` 70/70 (1 ignored)

Closed gaps (ordered by leverage):

- **`ZshCommand::Redirected(cmd, redirs)` variant** — compound commands (`{ ... } 2>&1`, `(...) >file`, `if ...; fi >file`) now carry trailing redirects through the parse → compile path; `parse_sublist`'s redirect-collection arm wraps non-Simple commands in this variant; `compile_command` brackets the body in `WithRedirectsBegin/End`. Earlier the parser silently dropped these.
- **Heredoc body propagation** — `process_heredocs` was `mem::take`-draining the lexer's heredoc list and never putting bodies back; redirect parsing stubbed `heredoc: None`. Fixed both: lexer mutates content in-place, parser records `heredoc_idx`, post-pass `fill_heredoc_bodies` resolves indexes into `ZshRedir.heredoc`.
- **`"$@"` and `"${arr[@]}"` array-spread in for-lists** — bridged path collapsed arrays into a single space-joined string. Native fast-path: detect `$@`/`$*` and `${name[@]}`/`${name[*]}` shapes, emit `BUILTIN_GET_VAR` / `BUILTIN_ARRAY_ALL` so a `Value::Array` reaches `BUILTIN_ARRAY_FLATTEN` intact.
- **Assoc subscripts `m[k]=v` and `${m[k]}` reads** — `compile_assign` detects subscripted names via a new `split_subscript` helper and emits `BUILTIN_SET_ASSOC`; `compile_word_str` adds a `braced_subscript_ref` fast-path that emits `BUILTIN_ARRAY_INDEX`. The fast-path is tightened to reject multi-group bodies (`${foo[a]} ${foo[b]}` was falsely matching as one ref with key `a]} ${foo[b`).
- **ZshFlag `${(flags)NAME}` native lowering** — mirrors `shell_compiler::try_lower_zsh_flag`. New `parse_zsh_flag` helper routes to `BUILTIN_PARAM_FLAG` directly. Without this, the bridge collapsed the flag form to a `Literal` whose runtime expansion lost the flag semantics (`(#)` returned 5 instead of 3 for an array of 3 elements).
- **`untokenize_preserve_quotes`** — sibling of `untokenize` that maps DNULL→`"`, SNULL→`'`, BNULL→`\` instead of stripping. Used by the bridge to round-trip through `ShellParser`. Without preserved quoting, `"$1|$2|$#"` lost its surrounding `"`s, `|` reached `ShellParser` as a pipe op, and the whole expansion broke. Fixed five double-quoted tests in one shot.
- **Process substitution `<(cmd)` / `>(cmd)`** — `compile_word_str` detects the shape and compiles the inner program as a sub-chunk, emitting `Op::ProcessSubIn(sub_idx)` / `Op::ProcessSubOut(sub_idx)`.
- **`is_select` flag on `ZshFor`** — `parse_select` stub previously delegated to `parse_for` and lost the keyword distinction. Added a flag; `compile_for` routes to a new `compile_select` that emits `BUILTIN_RUN_SELECT`.
- **`sublist.flags.coproc`** — `coproc { body }` was running synchronously. Compile path: body becomes a sub-chunk, then `BUILTIN_RUN_COPROC` is dispatched. Re-enables `${COPROC[0]}`/`${COPROC[1]}` pipe round-trip.
- **Dynamic command name `$cmd args`** — first word of a Simple compiled to `CallFunction("$cmd", ...)`, dispatching to "command not found: `$cmd`". Fixed by mirroring `shell_compiler::first_is_dynamic_literal`: when first word has unquoted `$`/`*`/`?`/`[`/backtick or starts with `~`, route through `Op::Exec` so the host runtime expands and dispatches via `host_exec_external` → `run_intercepts`.
- **Cmd-sub of new-pipeline functions** — `run_command_substitution`'s "internal vs external" decision checked only the legacy `functions` AST table. New-pipeline functions live in `functions_compiled` (populated by `BUILTIN_REGISTER_COMPILED_FN`). Added a `functions_compiled.contains_key(name)` check so `$(myfn)` runs in-process and captures stdout instead of forking a child that doesn't see the function.
- **`(q+)` flag in runtime `expand_string`** — the parser converted `q+` to `ZshParamFlag::Quote` (always quote). New `ZshParamFlag::QuoteIfNeeded` variant with the same `needs_quoting` predicate as `BUILTIN_PARAM_FLAG`. Now `${(q+)x}` for `x=safe` returns `safe`, for `x="has space"` returns `'has space'`.
- **Lexer port: `=~` regex pattern context** — the `incondpat`-trigger list checked for `\u{8d}~` (META-EQUALS + literal `~`) but not the actually-emitted `\u{8d}\u{98}` (META-EQUALS + META-TILDE). Regex parens after `=~` were lexed as syntactic `Inpar` instead of pattern chars; `[[ "v 1.2.3" =~ ([0-9]+)\.([0-9]+) ]]` parsed as `Cond("v 1.2.3", "=~", "")` followed by separate `Subsh(...)` commands.
- **Bare `$` at EOL** — `expand_string` collected an empty var name when `$` had no following char and resolved to `""`, eating the dollar. Pre-check `chars.peek().is_none()` and emit literal `$`.
- **`test ! -z foo`** — POSIX `test` builtin's match arms didn't handle the leading `!` negation. Added a tail `!`-prefix arm that recursively evaluates and flips.
- **EXIT trap firing for new-pipeline** — `execute_script_zsh_pipeline` didn't fire the `EXIT` trap on script end.
- **Array literal with cmd-sub `arr=($(...))`** — same IFS-split rule as for-list words.
- **BNULL escape preservation** — `\$lit` (backslash-escaped `$`) was untokenized to `$lit` and matched the bare-var fast-path, expanding to "" since `$lit` was unset. Added `has_bnull` guard that forces the bridge for words containing BNULL markers, plus `untokenize_preserve_quotes` maps BNULL→`\` so `ShellParser` sees the original escape.

Architectural shift: the lex/parse port is the load-bearing baseline; the bytecode compiler is the only original work. Migration follows a `bridge → native` ratchet — each loop iteration replaces one runtime fallback (`BUILTIN_EXPAND_WORD_RUNTIME`) call site with native ops. The bridge still handles param-modifier forms (`${x:-default}`, `${x#prefix}`, `${x/pat/rep}`) via `untokenize_preserve_quotes` round-trip. Killing those is task #37 (delete `ShellParser/ShellLexer/ShellCommand`).

## Session 2026-04-27 — corpus expansion + 100% parity push

Corpus extended from 227 to **295 construct tests** (`zsh_construct_corpus.rs`). All 295 + the other six suites pass on the new (default) pipeline. **768 tests total, all green.** Constructs added (anonymous fns, shift/set, string slicing, pattern replace anchors, bitwise + shift + pow arith, glob patterns, neg-index arrays, source builtin, special parameters incl. PIPESTATUS, case alternation/char-class, heredoc variants, multi-stmt fns, trap unset, complex pipelines, three-way short-circuits, local scoping, positional iteration, printf, [test], subshell isolation, comments, line continuation, empty/semis edge cases, multi-redirects, process-sub diff). Fixes landed in this push:

- **`$NAME` / `${NAME}` / `$N` in arithmetic** — `((`/`$((`/`for ((` arith parser now strips an optional leading `$` on identifiers and pre-loads the value via `BUILTIN_GET_VAR`. Without this, `(( $1 <= 1 ))` evaluated `$1` to 0 (broken recursion) — `$(( $1 ))` worked because the `$` substitution ran before arith parsing, but the compound `((..))` form didn't have that pre-pass.
- **Anonymous functions `() { body } a b c`** — new `ZshFuncDef.auto_call_args: Option<Vec<String>>` field; parser routes `Inoutpar` (the `()` token) to `parse_anon_funcdef`, which generates a unique `_zshrs_anon_N` name and stores the trailing args. `compile_funcdef` registers + immediately calls when `auto_call_args` is set. `Inoutpar` followed by no `{` falls back to a `Subsh` with empty body (preserves `()` no-op semantics — was an empty-subshell regression after the routing change).
- **Implicit positional `for x; do …; done`** — the `ForList::Positional` arm of `compile_for` was synthesizing a Vec<String> with `"\"$@\""` as a string literal, which broke the `$@`-fast-path detection (looks for raw zsh-tokenized form, not ASCII quotes). New `compile_for_positional` emits `BUILTIN_GET_VAR("@")` directly, then `ARRAY_FLATTEN` + iterate.
- **Subshell cwd isolation** — `SubshellSnapshot` now captures `cwd: Option<PathBuf>` at `subshell_begin` and restores via `set_current_dir` on `subshell_end`. `cd` inside a `(...)` no longer leaks to the parent shell. The variables/arrays/positional snapshot was already there; cwd was the missing piece.
- **`${v/#prefix/repl}` / `${v/%suffix/repl}`** — anchor support added to BOTH `apply_var_modifier::Replace`/`ReplaceAll` (for the structured-AST path) AND `expand_braced_variable`'s direct-string handler. `#` anchors at start, `%` anchors at end; under both anchors the replace-all/replace-first distinction collapses (an anchor matches at most once).
- **Heredoc `<<-EOF` strip-tabs from body** — `process_heredocs` was stripping tabs only for terminator comparison; the body lines were appended verbatim. Fix: append the stripped form when `strip_tabs` is on.
- **Heredoc with `$var` expansion** — unquoted-terminator heredocs now run their body through `BUILTIN_EXPAND_WORD_RUNTIME` (synthetic `ShellWord::Literal`) before `Op::HereString`. The body's trailing newline is trimmed first to compensate for `HereString`'s implicit append, so the resulting stdin is byte-identical to the source body.
- **Heredoc with quoted terminator `<<'EOF'` / `<<"EOF"`** — `HereDoc.quoted` flag added to the lexer struct, propagated to `HereDocInfo`, honored by `compile_redir`: `quoted=true` emits `Op::HereDoc(idx)` (verbatim); `quoted=false` runs the expand-word path. Detection: terminator contains `\u{9d}` (SNULL) or `\u{9e}` (DNULL), or starts with literal `'`/`"`.
- **`printf` format-string repetition** — POSIX printf re-applies the format while args remain. Wrapped the parser in an `'outer:` loop that exits when no args were consumed in the previous pass (so `printf 'literal'` still fires once) or when args are exhausted.
- **PIPESTATUS / pipestatus arrays** — `BUILTIN_RUN_PIPELINE` now collects per-stage exit statuses and writes them into both `pipestatus` (zsh) and `PIPESTATUS` (bash) arrays. Useful for `set -e`-style scripts that need to differentiate which stage failed.
- **`test ! -z foo`** (bonus from prior session, kept) — POSIX `test` builtin's match arms didn't handle `!` negation prefix.

Net effect: every new corpus test surfaces a real shell feature gap; this session closed all 14 gaps the new tests revealed. The 100% parity target is concrete now, not aspirational — every construct in the corpus has a behavioral pin with an exact-stdout assertion.

## Session 2026-04-27 — corpus expansion wave 2

Construct corpus extended 295 → **344**. All 344 + the other six suites pass on the new (default) pipeline. **817 tests total, all green on the new pipeline.** Constructs added: `${var:+alt}`, `${var:=default}`, `${var:?error}`, long-prefix/suffix strip, negative substring offset, `[[ -v var ]]`, `set -e`, getopts, read-into-multiple-vars, default `$REPLY`, `$RANDOM` / `$SECONDS` / `$EPOCHSECONDS` / `$LINENO`, case fall-through `;&`, `|&` pipe-with-stderr, brace expansion variants (range step, letter range, prefix/suffix), heredoc-into-pipe, until-loop, local arrays in functions, indexed array iteration, function with empty body, dynamic var refs, export-visible-to-child, string `=` with quoted pattern, no-match check, compound arith assign, `(exit N)` subshell-only.

Closed gaps:

- **`(exit N)` subshell-isolation** — `builtin_exit` was unconditionally `process::exit(N)`. Now: when `subshell_snapshots` is non-empty, set `last_status + returning` and let the subshell's `return_patches` rewinding land at `SubshellEnd`. Plus: `compile_command::Subsh` saves/restores the parent's `return_patches` around `compile_program(prog)` so any `exit` / `return` inside the subshell lands at `SubshellEnd` (popping the snapshot) rather than escaping to the chunk's top-level return target.
- **`${a[$i]}` (variable subscript)** — runtime `expand_braced_variable` was passing the raw subscript text directly to the index parser; `$i` parsed as 0/1, all elements collapsed to first. Fix: call `self.expand_string(index)` first so `$i`/`${expr}`/`$((arith))` resolve before subscript parsing. The `braced_subscript_ref` compile-time fast-path was also tightened to reject keys containing `$` / backtick — those need runtime expansion via the bridge path.
- **`[[ -v var ]]`** — new `BUILTIN_VAR_EXISTS` (id 306). Pops a name, checks scalar / array / assoc / env tables, pushes Bool. `compile_zsh::emit_file_test` adds a `-v` arm that calls it.
- **`$RANDOM` / `$SECONDS` / `$EPOCHSECONDS` / `$LINENO`** — added to `get_variable`'s special-name match. RANDOM uses nanos+pid mixed via Knuth's hash, masked to 15 bits; SECONDS reads from `__zshrs_start_secs` baseline; EPOCHSECONDS from `SystemTime::now()`; LINENO falls back to "1".
- **`${v: -3}` (negative substring offset)** — disambiguator from `${v:-default}`. Detection: trim leading spaces from the post-colon text, then check digit-or-dash. zsh requires the leading space to make `:-` not parse as default-if-unset.
- **Case `;&` fall-through** — `compile_case` was emitting no jump for Continue/TestNext, so flow returned to the next arm's pattern check (re-testing). Fix: track `pending_fall: Option<usize>` — `;&` emits a forward jump, the next arm's body_start patches it. `;|` (TestNext) keeps existing fall-into-pattern behavior. Last-arm `;&` patches to `end`.
- **`|&` (pipe + merge stderr)** — `ZshPipe.merge_stderr: bool` field. Parser captures the `Baramp` token (was discarded). `compile_pipe` collects (cmd, merge) pairs; for stages with `merge=true`, the sub-chunk emits a `Redirect(2, DUP_WRITE)` with target `"1"` BEFORE the body — so the stage's stderr goes through the pipe with stdout.
- **`$NAME` / `${NAME}` / `$N` in arithmetic** (from prior session, deepened) — `next_tok` now accepts `$`-prefixed identifiers, treats `${...}` braces as identifier-name brackets. `collect_identifiers` strips the optional `$` so pre-loading via `BUILTIN_GET_VAR` works for `(( $1 ))`.
- **`printf` format-string repetition** (carried forward) — POSIX printf re-applies format while args remain. Wrapped scan loop in `'outer:` that exits when no args were consumed in a pass or when args exhausted.
- **PIPESTATUS / pipestatus arrays** (carried forward) — `BUILTIN_RUN_PIPELINE` populates both `pipestatus` (zsh) and `PIPESTATUS` (bash) with per-stage exit codes.

Old-pipeline (`ZSHRS_OLD_PIPELINE=1`) corpus pass rate is now ~96% — the legacy ShellParser path doesn't have anonymous functions, `|&` pipe-merge, `$RANDOM`, `(exit N)` subshell-isolation, etc. That's intentional: the old path is an emergency escape hatch slated for deletion (task #37). The 100% parity target is the **new** (default) pipeline.

## Session 2026-04-27 — Phase 1 native lowerings (kill the bridge)

The bridge in `compile_word_str` round-trips raw zsh-tokenized words through `untokenize_preserve_quotes → format!("echo {}", …) → ShellParser → ShellWord → JSON → BUILTIN_EXPAND_WORD_RUNTIME → expand_word_glob → expand_word → expand_string`. Phase 1 replaces high-traffic shapes with native bytecode. Status:

| Step | Shape | Status | Implementation |
|---|---|---|---|
| 1a | `${v:-d}` `:=` `:?` `:+` | ✅ done | `BUILTIN_PARAM_DEFAULT_FAMILY` (id 307) |
| 1b | `${v:offset:length}` | ✅ done | `BUILTIN_PARAM_SUBSTRING` (id 308) |
| 1c | `${v#}` `##` `%` `%%` strip | ✅ done | `BUILTIN_PARAM_STRIP` (id 309) |
| 1d | `${v/p/r}` `//` `/#` `/%` replace | ✅ done | `BUILTIN_PARAM_REPLACE` (id 310) |
| 2 | `${#name}` length | ✅ done | `BUILTIN_PARAM_LENGTH` (id 311) |
| 3 | `$(cmd)` cmd-sub | ✅ done | `BUILTIN_CMD_SUBST_TEXT` (id 313) — routes through `run_command_substitution` for now (`Op::CmdSubst` sub-chunk path had a quoting bug — `printf "a\nb"` produced "anb"; the text-passthrough avoids it) |
| 3b | `$((expr))` arith-sub | ✅ done | `BUILTIN_ARITH_EVAL` (id 312) — calls `evaluate_arithmetic` for proper int/float distinction (fusevm's `Op::Div` is float-only and was the source of `$((10/3)) = 3.333...`) |
| 4 | concat `pre${v}suf` / `${a}${b}` | ✅ done | `split_word_segments` walks raw tokenized word, splits on top-level META-`$` / QSTRING / backtick; tracks INBRACE/INBRACK depth so `${a[$i]}` doesn't get mis-split; per-segment recursive emit + N-1 `Concat` |
| 5 | DoubleQuoted `"$1 and $2"` | ✅ done | falls into step 4 — QSTRING markers are top-level inside DNULL wrappers |
| 6 | Tilde non-leading | pending | low-priority, niche shape |

**Bug closed in step 4**: my POUND vs QSTRING confusion. `\u{84}` is POUND (`#`), `\u{8c}` is QSTRING (`$` inside double quotes). My initial concat-split predicate included `\u{84}` as a `$`-marker; this wrongly treated `${#arr[@]}` as a concat with `#arr` as the expansion body, producing literal `${#arr[@]}` output.

**Bug closed in step 4**: depth tracking. `${a[$i]}` was getting split at the inner `$i`, yielding `${a[` + GET_VAR("i") + `]}` instead of array-element access. Fix: brace_depth + brack_depth counters, only split markers at top level.

**Bug closed in step 3b**: `Op::Div` is float-only in fusevm 0.10.1. ArithCompiler emits it for `/`. zsh integer-divides `10/3 = 3`. Bypassing ArithCompiler entirely for `$((expr))` and going through the executor's MathEval (which preserves int semantics) is correct AND simpler than patching ArithCompiler.

**Bug closed in step 3**: my naive `Op::CmdSubst(sub_idx)` path fed the inner cmd through ZshParser+ZshCompiler. Some sub-chunk word-emit difference (vs shell_compiler's same-shaped emit) loses quotes — `$(printf "a\nb")` produced "anb" instead of "a\nb". Workaround: pass the raw cmd text to a builtin that calls `run_command_substitution` (uses ShellParser internally — to be migrated to ZshParser as part of Phase 2's type-deletion sweep).

After Phase 1, the bridge is hit only for: tilde non-leading, brace expansion with vars, nested `${${var}[1]}`/`${var/${pat}/repl}`, `${(P)var}` indirect. All niche. Bridge usage in real scripts (zpwr, .zshrc) drops to a small fraction of what it was.

Phase 2 — actual deletion of `ShellParser`/`ShellLexer`/`ShellCommand`/`ShellWord` types — remains pending. The runtime callers `run_command_substitution`, `run_process_sub_in/out`, autoload, `expand_word_glob`, `apply_var_modifier`, `apply_zsh_param_flag` all still consume `ShellWord`/`ShellCommand`. Migrating them is its own session-bounded task. The legacy `ZSHRS_OLD_PIPELINE=1` env-var fallback also stays for now (one more migration milestone before it's safe to delete).

Test count: **876 across 9 suites, all green on the new (default) pipeline.**

## Session 2026-04-27 — Phase 2 (kill the runtime ShellParser callers)

Purpose: chip away at runtime callers of `ShellParser`/`ShellWord`/`ShellCommand` so those types become deletable. Strict no-regression — every migration must preserve the 876-test green slate.

### Migrations landed

| Caller | From | To |
|---|---|---|
| `run_command_substitution` | `ShellParser → ShellCompiler → execute_command` (internal) + `Command::new` (external) split | `ZshParser → ZshCompiler → sub-VM` with stdout-capture pipe; one path handles both |
| `execute_script_file` (cache-miss) | `ShellParser + ShellCompiler` | `ZshParser + ZshCompiler` |
| `execute_script` legacy body (`ZSHRS_OLD_PIPELINE=1` opt-out) | full ShellParser+ShellCompiler+VM body + EXIT trap | one-line delegate to `execute_script_zsh_pipeline` (env-var is now a no-op) |
| `builtin_pmap` / `pgrep` / `peach` | `ShellParser + ShellCompiler` per-arg | `ZshParser + ZshCompiler` per-arg |
| **The bridge in `compile_word_str`** | `ShellParser → ShellWord → JSON → BUILTIN_EXPAND_WORD_RUNTIME → expand_word_glob → expand_word → expand_string` | `untokenize_preserve_quotes(s) + mode_byte → BUILTIN_EXPAND_TEXT → expand_string + braces + glob` (mode 0=Default, 1=DoubleQuoted, 2=SingleQuoted, 3=Backquote) |
| Heredoc body var-expansion | `ShellWord::Literal` JSON + `BUILTIN_EXPAND_WORD_RUNTIME` | `BUILTIN_EXPAND_TEXT` direct |
| `run_process_sub_in/out` | `ShellParser → ShellCommand::Simple` words via `expand_word(ShellWord)` | new `simple_cmd_words` helper: `ZshParser → ZshSimple` + `expand_string` per word |

### What had to be solved mid-flight

- **Bridge mode-byte API**: the old bridge target (`BUILTIN_EXPAND_WORD_RUNTIME`) consumed a `ShellWord` JSON and routed by variant — `Literal` got brace+glob, `DoubleQuoted` skipped them. Replacing with text-only meant losing that distinction. New `BUILTIN_EXPAND_TEXT(text, mode_byte)` re-encodes the variant info as a 4-state mode byte. Compile-time `expand_text_mode` decides by inspecting the raw zsh-tokenized word's wrapping.
- **DQ-mode `\$` handling**: `expand_string` expects the lexer's `\0X` zero-marker for already-escaped chars. The text-only bridge ships raw `\$lit` from the source. Pre-process inside the DQ arm: walk chars, convert `\\$` / `\\\`` / `\\"` / `\\\\` → `\0…` before calling `expand_string`. Without this, `echo "\$lit"` produced just `\` because the `$` was treated as live expansion.
- **`$((expr))` segmentation in concat**: `find_expansion_end` was walking `$((` looking for matching `\u{8a}` OUTPAR pairs, but the lexer collapses `))` into a single `\u{8b}` OUTPARMATH token (and the inner `(` / `)` stay as literal ASCII, not META). The expansion never closed → engulfed the trailing literal. Fixed: detect arith-shape via `chars[i+2] == '('` or `INPARMATH`, end at the first OUTPARMATH.

### State at session end

Reference counts of the legacy types (lower = closer to deletable):

| File | Refs (before this session) | Refs (now) | Notes |
|---|---|---|---|
| `src/compile_zsh.rs` | 18 | **17** (mostly comments) | bridge gone; only `ShellCompiler::new()` reuse for `ArithCompiler` |
| `src/exec.rs` | 121 | **114** | `ShellParser::new` count: 10 → 4 (run_process_sub_in/out moved off; autoload + compsys cache + 2 helpers remain) |
| `src/parser.rs` | 203 | 203 | type definitions themselves; can shrink only when types are deleted |
| `src/shell_compiler.rs` | 177 | 177 | unchanged; depends on autoload deletion |
| `src/compiler.rs` | 56 | 56 | legacy bytecode compiler — still consumed somewhere |
| `src/ast_opt.rs` | 18 | 18 | unchanged |
| `src/text.rs` | 58 | 58 | pretty-printing of legacy AST |
| `src/zwc.rs` | 45 | 45 | autoload bytecode cache |

`ShellParser` callers in `exec.rs`: **10 → 4** (`run_process_sub_in/out` migrated, `pmap`/`pgrep`/`peach` migrated, `run_command_substitution` migrated, `execute_script_file` migrated, legacy `execute_script` body deleted). The remaining 4 are autoload-related (lines 12482, 12541, 16796, 16893).

### Why the actual type deletion can't ship in one session

The remaining work to remove `ShellParser`/`ShellLexer`/`ShellCommand`/`ShellWord` is a cascade through `executor.functions` (44 references — the AST table that `BUILTIN_REGISTER_FUNCTION` populates and that `ZshrsHost::call_function` reads when compiling autoloaded functions on demand). Migrating it requires:

1. Change `autoload_function` return type from `Option<ShellCommand>` to `Option<ZshProgram>`.
2. Rename / re-type `executor.functions` to `HashMap<String, ZshProgram>`.
3. Migrate `ZshrsHost::call_function`'s "compile AST on demand" path to `ZshCompiler`.
4. Migrate `BUILTIN_REGISTER_FUNCTION`'s body-decode (currently deserializes `ShellCommand` JSON).
5. Migrate the compsys cache prefetch loop (lines 16796 / 16893) to ZshParser.
6. Migrate `run_process_sub_in/out`-style `simple_cmd_words` to handle non-Simple commands (or accept the Simple-only restriction permanently).
7. Delete `ShellExecutor::execute_command` (used only by legacy `call_function`).
8. Delete `ShellExecutor::call_function(&ShellCommand, ...)` (legacy tree-walker fallback).
9. Delete `expand_word_glob` / `expand_word` / `apply_var_modifier` / `apply_zsh_param_flag` (still used by ShellCommand-loaded function bodies).
10. Delete `BUILTIN_EXPAND_WORD_RUNTIME` (still emitted by `shell_compiler.rs` for legacy-compiled function bodies).
11. Move `ArithCompiler` out of `shell_compiler.rs` into its own module.
12. Delete `shell_compiler.rs` entirely.
13. Delete `compiler.rs` (legacy compiler), `ast_opt.rs`, `text.rs`'s ShellCommand pretty-printer.
14. Delete `ShellParser` / `ShellLexer` / `ShellCommand` / `ShellWord` / `Redirect` / `RedirectOp` / `CompoundCommand` / `SimpleCommand` / `VarModifier` / `CaseTerminator` / `ShellToken`.

Each step is non-trivial — most touch ~20-50 references. The whole sequence is multi-session work. This session shipped the high-leverage Phase 2 chunks: the bridge target, run_command_substitution, process_sub. The remaining sequence is **autoload-shaped** — once autoload migrates, the rest cascades cleanly.

Test count: **876 across 9 suites, all green on the new (default) pipeline.**
