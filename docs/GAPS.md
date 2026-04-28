# `man zshall` gap audit — current status

Originally probed 47 constructs from the `man zshall` reference. Each entry below was verified by running zshrs (`./target/debug/zshrs -f -c '...'`) against expected zsh behavior. Source-only audit false-positives that already worked (e.g. `${(j: :)arr}`, `${(t)var}`, `${(P)x}`, `<<<`, short-loop `for x in y; { ... }`, `repeat N ( ... )`, `zparseopts`) are not listed.

## Closed (verified against binary)

### Parameter expansion

- `${(f)str}` — split on newlines into array.
- `${(z)str}` / `${(w)str}` — array-producing flags. Handler returns `Value::Array` so `print -l ${(z)a}` splits one-per-line.
- `${(B)x}` — backslash-escape shell metas. New 'B' arm in `BUILTIN_PARAM_FLAG`.
- `${(flags)"literal"}` / `${(flags)'literal'}` — flag operand may be a quoted string literal. `parse_zsh_flag_literal` runs untokenize_preserve_quotes on the lexer-marked word, detects `${(F)"…"}`, emits a `\u{01}`-prefixed operand to `BUILTIN_PARAM_FLAG`. Verified for `(U)`/`(z)`/`(s)`/`(f)` literal forms.
- `RC_EXPAND_PARAM` — `X${arr}Y` → cartesian distribution (`XaY XbY XcY`) when option set; default scalar join (`Xa b cY`) without. New `BUILTIN_CONCAT_DISTRIBUTE` (id 318) handles cartesian; `BUILTIN_CONCAT_SPLICE` (id 319) handles default `${arr[@]}` first/last sticking. `BUILTIN_GET_VAR` returns `Value::Array` when option is set.
- `${arr[@]}` first/last sticking — `print -l X${arr[@]}Y` produces 3 args ("Xa", "b", "cY"). Same path as RC_EXPAND_PARAM but with splice semantics instead of cartesian.

### Special parameters

- `$argv` — array alias for positional params. `set -- a b c; echo $argv` → "a b c".
- `$EPOCHREALTIME` — sub-second epoch. Emits `SECS.UUUUUU`.
- `$RANDOM_FILE` — not a bug; mainline zsh also leaves it empty without `zmodload zsh/random`.

### Test operators

- `[[ a -ef b ]]` — same-inode test. New `BUILTIN_SAME_FILE` (id 315) compares `(dev, inode)` via `fs::metadata`.

### Glob qualifiers

- `*(D)` — per-pattern dotglob. `expand_glob` activates dotglob when 'D' appears in the qualifier string.

### `typeset`

- `typeset -Z N x=val` / `-L N` / `-R N` — width as a separate arg now parsed (in-flag form `-Z5` was already working). Width applied at assignment time.
- `typeset -T VAR var [SEP]` — initial bind splits current `$VAR` (or `=VAL` form) on SEP into array.
- `typeset -T` bidirectional sync — `tied_scalar_to_array` / `tied_array_to_scalar` HashMaps record `(peer, sep)`. `BUILTIN_SET_VAR` mirrors scalar→array; `BUILTIN_SET_ARRAY` mirrors array→scalar. `PATH=/a:/b; typeset -T PATH path; path=(/x /y); echo $PATH` → `/x:/y`.
- `declare -g x=val` from inside a function — `-g` opts out of `local_save_stack` push.

### Grammar

- `time { compound; ... }` — new `BUILTIN_TIME_SUBLIST` (id 316) runs the sublist as a sub-chunk, prints elapsed wall-clock time.
- `{ try } always { finally }` — `compile_zsh`'s `ZshCommand::Try` arm compiles both blocks sequentially; finally runs unconditionally.
- `for var (a b c) cmd` and `for var (a b c) { ... }` — `parse_for` handles the lexer-port quirk that emits parens as a single String token (`\u{88}a b c\u{8a}`).
- `exec {fd}>file` — parser detects `{NAME}` followed by redirop and pops it as varid. New `BUILTIN_OPEN_NAMED_FD` (id 317) opens path with libc flags, dups to fd ≥ 10 via `F_DUPFD_CLOEXEC`, stores fd number in `$varid`.

### Process substitution

- `>(...)` — `process_sub_out` creates real named pipe via mkfifo and forks a child that reads it. `untokenize` was missing OUTANGPROC → '>' mapping (caused `compile_word_str` detection to fail). Both fixed; `tee >(cat)`, `echo > >(cat)` work.

### Stub builtins routed and fixed

- `sched`, `echotc`, `echoti`, `getln`, `zpty`, `ztcp`, `zsocket`, `private`, `zformat`, `zregexparse` — defined as `builtin_*` handlers but absent from fusevm's `shell_builtins::builtin_id` table. Script-level dispatch fell through to external command spawn ("command not found"). `host_exec_external` now intercepts these names before the OS-level exec attempt.
- `zformat -f` / `zformat -a` — printed result to stdout instead of assigning to named variable/array. Fixed: now uses `self.variables.insert` / `self.arrays.insert`.
- `private` — routes to `builtin_local` (zsh `private` has the same local-scope semantics as `local`).
- `zregexparse` — already worked correctly; earlier probe used wrong flags.

## Closed (this session — man zshall pass)

### Special parameters

- `${commands[ls]}`, `${aliases[ll]}`, `${galiases[…]}`, `${saliases[…]}`, `${functions[foo]}`, `${builtins[echo]}`, `${reswords[for]}`, `${options[interactive]}`, `${parameters[PATH]}`, `${jobtexts[N]}`, `${jobdirs[N]}`, `${jobstates[N]}`, `${nameddirs[name]}`, `${userdirs[user]}` (libc getpwnam), `${modules[zsh/datetime]}`, `${dis_functions[…]}` — magic shell-introspection assocs synthesized at lookup time via `magic_assoc_lookup` in `BUILTIN_ARRAY_INDEX`.
- `$TTY` (libc ttyname), `$TTYIDLE` (st_atime delta), `$TRY_BLOCK_ERROR` (set via new `BUILTIN_SET_TRY_BLOCK_ERROR` between try / always arms), `$patchars`, `$RANDOM_FILE` (/dev/urandom).

### Builtins

- `printf -v VAR fmt args...` — bash-compat var-assign mode. `builtin_printf` is now `&mut self`; `-v VAR` strips the flag and inserts the formatted output into `self.variables[VAR]`.
- `[[ -o option ]]` — shell-option-set test via new `BUILTIN_OPTION_SET` (id 321). Normalizes name (strip _, lowercase). Verified with both `RC_EXPAND_PARAM` and `rc_expand_param` forms.
- `setopt -p` / `setopt -L` — emit `setopt OPTION` lines for every currently-set non-default option, source-replayable.
- `read -n N` — bash-compat alias for zsh's `-k N` (read N characters).
- `private`, `zformat`, `zregexparse` — routed through `host_exec_external` interception so script-level dispatch hits handlers instead of "command not found".
- `zformat -f` / `zformat -a` — fixed var-assign bug; previously printed result to stdout, now uses `self.variables.insert` / `self.arrays.insert`.

### Parameter expansion

- `${(u)arr}` — unique flag, preserve first occurrence drop dupes.
- `${(C)str}` — capitalize first letter of each word, lowercase rest.
- `${arr/old/new}` / `${arr//old/new}` — per-element replacement on arrays. `BUILTIN_PARAM_REPLACE` checks `exec.arrays` first.
- `${arr:#pattern}` — array filter remove matching. New `ParamModifierKind::FilterRemoveMatching` + `BUILTIN_PARAM_FILTER` (id 322) using `glob_match`.
- `${(kv)assoc}` / `${(vk)assoc}` — interleaved key/value pair output. 'k' / 'v' arms in `BUILTIN_PARAM_FLAG` peek for partner flag.

### Brace expansion

- `{01..10}` zero-padding. `expand_brace_sequence` detects leading-0 bounds and pads each output to max(start.len, end.len). Negative-aware.

### Glob qualifiers

- `*(L0)` / `*(L+10k)` / `*(L-1m)` — size qualifier with full zsh syntax `L[+-]N[k|m|g|p]`. Default unit 512-byte blocks; suffix maps to KB/MB/GB/bytes.

### Word concatenation (RC_EXPAND_PARAM)

- `X${arr[@]}Y` first/last sticking — new `BUILTIN_CONCAT_SPLICE` (id 319): `print -l X${arr[@]}Y` → 3 args ("Xa", "b", "cY").
- `X${arr}Y` with `RC_EXPAND_PARAM` cartesian — new `BUILTIN_CONCAT_DISTRIBUTE` (id 318): same input → 3 args ("XaY", "XbY", "XcY"). Without option, joins to scalar (zsh default).

## Closed (this session — subscript pass)

Discovered as gaps when re-probing `man zshall` chapter 14 (Parameters → Array Subscripts). All implemented inside `BUILTIN_ARRAY_INDEX` and a small set of module-level helpers in `src/exec.rs`.

### Array slice `${arr[N,M]}`

- Indexed array slice with positive, negative, and mixed bounds. `${arr[2,4]}`, `${arr[-2,-1]}`, `${arr[1,-1]}`. Returns `Value::Array` so downstream `print -l` / `for` consumes per-element.
- `slice_indexed_array` helper: zsh 1-based inclusive semantics, negative-from-end, out-of-range clamp.

### Scalar slice `${str[N,M]}` / `${str[N]}`

- Char-aware (UTF-8 char count, not byte index). Both single-index `${str[1]}` and slice forms supported. Falls through from `BUILTIN_ARRAY_INDEX` when `name` isn't an indexed/assoc array. New `slice_scalar` helper.

### Bare-variable / arithmetic subscript `${arr[i]}`

- Subscript context is arithmetic in zsh — bare names resolve as variables, full expressions evaluate. `${arr[i]}`, `${arr[i+1]}`, `${arr[len-1]}` all work. Implemented by replacing `idx.parse::<i64>()` Err arm with `eval_arith_expr` fallback.

### Subscript flags `(r)` `(R)` `(i)` `(I)` `(e)` (combinable)

- `(r)pat` — first matching value; `(R)pat` — last matching value (reverse).
- `(i)pat` — first matching index (1-based; len+1 if no match); `(I)pat` — last matching index (0 if no match).
- `(e)str` — exact (literal) instead of glob match. Combinable: `(re)`, `(ie)`, `(Ie)`, etc.
- For assoc arrays, `r`/`R` searches values; `i`/`I` returns the matching key. Implementation: `parse_subscript_flags` + `array_subscript_flag` / `assoc_subscript_flag`.

### `typeset -A m; m=(k v ...)` two-statement assoc init

- After `typeset -A` declares an empty HashMap entry in `assoc_arrays`, the array literal in the next statement is now interpreted as alternating k/v pairs and stored as assoc — previously the array assignment overwrote it as indexed and silently dropped the `-A` attribute. Implemented in `BUILTIN_SET_ARRAY` by checking `assoc_arrays.contains_key(&name)` before the indexed-array path.

## Still open

- **History expansion** (`!!`, `!$`, `^old^new^`) — `expand_history` is wired into `execute_script` but gated on `atty::is(Stream::Stdin)`. Works in interactive mode; correctly no-op in `-c` mode (where the original audit claimed broken — false positive).
- **`^pat` extendedglob negation** — pattern-prefix `^` for "match all NOT matching pat" needs glob-matcher support. Verified still missing: `${arr:#^*.txt}` returns all elements unfiltered instead of dropping non-`.txt`.
- **`${(kv)a[@]}` with `[@]` subscript** — flag prefix + `[@]` subscript composition: `[@]` goes through `BUILTIN_ARRAY_INDEX` which doesn't apply (kv) flag. Without subscript (`${(kv)a}`) works.
- **`${(@s.,.)str}` literal split with `@` flag** — `(s)` alone works; combined with `(@)` doesn't split. Edge case.
- **`function () { ... }` anonymous form with `function` keyword** — bare `() { ... }` form already works; the `function` keyword variant compiles to a no-op (parser drops the body when no name follows the keyword). Fix needs parser pass that recognizes the empty-name shape.
- **`=(...)` process substitution** — temp-file process sub still missing (only `<(...)` / `>(...)` named-pipe form is implemented). `cat =(echo hi)` errors with "No such file or directory".
- **Assoc subscript with `$`-expansion key**: `${m[$k]}` returns empty for assocs even though `$k` resolves correctly elsewhere. The `braced_subscript_ref` fast-path rejects keys containing `$`, falling back to a bridge that doesn't perform the assoc lookup. Indexed-array form (`${arr[$i]}`) does work.
- **Extendedglob inline flags** `(#i)`, `(#l)`, `(#a)` — case/approx-insensitive pattern flags inside a pattern. `[[ ABC = (#i)abc ]]` returns nomatch.
- **Stub modules** — `zsh/cap`, `zsh/clone`, `zsh/curses`, `zsh/zftp` builtins not registered (zmodload no-ops). `zsh/mapfile` assoc-array form not implemented (the bash-compat `readarray` builtin works for the common case).

## Stub modules (loaded but limited)

- `zsh/cap`, `zsh/clone`, `zsh/curses`, `zsh/zftp` — module loads via `zmodload` succeeds but the corresponding builtins (`cap`, `clone`, `zcurses`, `zftp`) aren't registered. Niche features; deferred.
- `zsh/db_gdbm` — `ztie` correctly reports "GDBM support not compiled in" (no native gdbm dep). Acceptable stub behavior.
- `zsh/files chown/chmod/chgrp` — works (proper error for nonexistent file).
- `zsh/mapfile` — `${mapfile[/path/to/file]}` assoc-array form not implemented. Niche feature; the `readarray` / `mapfile` builtin (bash-compat) DOES work for the common case.
- `zsh/private` — closed; routed to `builtin_local`.
- `zsh/newuser`, `zsh/nearcolor` — niche, deferred.
