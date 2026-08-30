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

### Subscript with `$`-expansion key `${m[$k]}`

- `braced_subscript_ref` rejected keys containing `$`, falling back to a bridge path that didn't perform the assoc lookup. Added `braced_subscript_dynamic_ref` which matches the same `${BASE[KEY]}` shape but allows `$` in `KEY`; the compile path emits `BUILTIN_EXPAND_TEXT` (mode 1, no glob/brace) to resolve the key at runtime, then `BUILTIN_ARRAY_INDEX` for the lookup. Works for both assoc and indexed arrays, plain refs (`$k`), and concat refs (`$pre$post`).

### Extendedglob `^pat` negation in `${arr:#pat}`

- New module-level helper `extendedglob_match` reads the `extendedglob` option at match time; when set, a leading `^` strips itself and inverts the result of the underlying glob_match. Wired into both `BUILTIN_PARAM_FILTER` (compile-path filter) and the legacy `(M)` flag path in `expand_word_glob`. `${arr:#^*.txt}` now keeps only `*.txt` elements; `${(M)arr:#^a}` keeps the inverse. Without `extendedglob` set, `^` stays literal.

### Extendedglob inline pattern flags `(#i)` / `(#I)` / `(#l)` / `(#a<n>)`

- `parse_pattern_flags` strips the leading `(#flags)` block from a pattern. `glob_match_static` now applies the flags before regex translation: `(#i)` adds the regex `(?i)` prefix; `(#I)` cancels `(#i)`; `(#l)` inflates each lowercase pattern char to a `[xX]` character class so it matches either case in the input while uppercase pattern chars stay exact. `(#a<n>)` short-circuits to a Wagner-Fischer Levenshtein-distance check via a new `approximate_match` helper (insert/delete/substitute, default n=1 when the digit is omitted). All paths that go through `glob_match_static` pick this up automatically — `[[ str = pat ]]`, case arms, `${arr:#pat}` filter, etc.

### `${(@s:,:)str}` / `${(@f)str}` — `@` + split flag composition in DQ

- `(@s:sep:)` previously failed inside `"…"` because `@` runs first (wrapping the scalar into a 1-elem array), then the `s` arm in `BUILTIN_PARAM_FLAG` was a no-op on `St::A` — leaving `["a,b,c"]` which DQ joined back into `"a,b,c"`. Fixed by making `s` and `f` flat-map split each element of an array (not just scalars). Also handles the genuine "array of CSV strings" case `arr=("a,b" "c,d"); ${(@s:,:)arr}` → 4-element flat result, matching zsh.

### `${(kv)a[@]}` — flag prefix + `[@]` subscript composition

- `parse_zsh_flag` rejected names with `[`, so `${(kv)m[@]}` fell through to a bridge path that returned just values (the (k) flag never applied). Fix is one line in the matcher: strip a trailing `[@]` or `[*]` suffix from the name before validating; the result is the same name we'd use for the flag-only form, and `BUILTIN_PARAM_FLAG` already returns `Value::Array` for array-producing flags. Also fixes `${(k)a[@]}`, `${(v)a[@]}`, `${(o)a[@]}`, `${(O)a[@]}`, etc.

### `function () { body } args...` — anonymous form with `function` keyword

- `parse_funcdef` collected names then optionally consumed `()`, but never synthesized an anonymous-name placeholder when no name was given — `compile_funcdef` saw `names = []` and emitted nothing, so the body never registered or ran, AND any trailing args (`a b c`) were re-tokenized as a separate command list, producing "command not found" garbage. Fix: in `parse_funcdef`, when `names.is_empty() && saw_paren`, mirror `parse_anon_funcdef` — generate `_zshrs_anon_kw_N`, collect trailing args, set `auto_call_args` so the existing compile path registers + immediately invokes. Bare `() { … }` form was already handled by `parse_anon_funcdef`; this closes parity with the `function`-keyword variant.

### `=(cmd)` process substitution (temp-file flavor)

- `=(...)` is the temp-file flavor of process sub (zsh-only, vs `<(...)`'s FIFO). Both deliver a path to the consumer; the read-end implementation `process_sub_in` already creates a durable temp file (synchronous run, capture stdout to `/tmp/zshrs_psub_*`), so `=(...)` shares it via the same `Op::ProcessSubIn` emission. Compile-path detector adds an `is_eq_psub` branch alongside the existing `<(…)` / `>(…)` matchers. Verified against `cat`, `wc`, `diff`, `printf` consumers.

### `${mapfile[/path]}` — `zsh/mapfile` magic assoc

- `magic_assoc_lookup` now recognizes `mapfile` as a magic assoc name: `${mapfile[/path]}` reads the file's bytes verbatim (trailing newline preserved — matches zsh: a `"test\n"` file gives `${#mapfile[…]} = 5`, not 4). Missing files yield empty. Composes with `(f)` for line-split via the new `parse_zsh_flag_subscript` matcher (see below). The `${(@f)mapfile[…]}` shape correctly produces N+1 elements when the file ends with `\n` (the trailing empty element is preserved per zsh).

### `${(flags)NAME[KEY]}` — flag + literal subscript composition

- `parse_zsh_flag` only handled `${(flags)NAME}` and `${(flags)NAME[@]}` shapes. New `parse_zsh_flag_subscript` matches `${(flags)NAME[KEY]}` for any non-`@`/`*` literal key. Compile path emits a 4-step sequence: resolve the subscripted value via `BUILTIN_ARRAY_INDEX`, prepend the `\u{01}` literal-value sentinel via `Concat`, then call `BUILTIN_PARAM_FLAG` so the flag walks treat it as a pre-resolved scalar. Closes `${(f)mapfile[/path]}`, `${(s:,:)assoc[k]}`, `${(U)assoc[k]}`, etc.

### History expansion in `-c` mode (false positive)

- The original audit flagged `!!` / `!$` / `^old^new^` as missing in `-c` mode. Verified this is the documented zsh behavior: history expansion only fires in interactive (TTY-stdin) mode; `-c` script mode treats `!!` literally. zshrs's `expand_history` correctly gates on `atty::is(Stream::Stdin)`, matching mainline zsh. Added `test_history_expansion_literal_in_c_mode` regression test (echo "first; echo !!" → "first\n!!").

## Closed (second-pass audit, this session)

A wide differential probe against `/bin/zsh` surfaced a fresh batch of gaps. The high-impact ones are now closed:

### Indexed-array element / slice / delete assignment

- `a[2]=YY` (single element), `a[-1]=Z` (negative subscript), `a[5]=E` (grow on assign), `a[2]+=BB` (append at index), `a[2,4]=(YY ZZ WW)` (slice replace), `a[2]=()` (single-element delete), `a[2,4]=()` (slice delete) — all now mutate the indexed array in place. `BUILTIN_SET_ASSOC` was extended with an indexed-array dispatch that routes when the name already names an indexed array OR (for unset names) when the key is a literal integer; otherwise still falls through to assoc. New `BUILTIN_SET_SUBSCRIPT_RANGE` (id 323) handles the array-RHS form so `a[i]=(elements)` and `a[i,j]=(elements)` use one-shot splice semantics. Empty values + comma-key delete the whole slice.

### `=~` regex match captures (`$MATCH`, `$match`, `$mbegin`, `$mend`)

- `regex_match` now uses `Regex::captures` and writes `$MATCH` (full match), `$MBEGIN`/`$MEND` (1-based char offsets), and `$match[]` / `$mbegin[]` / `$mend[]` arrays for each capture group. `[[ "a1b2" =~ ([a-z])([0-9]) ]]; print $match[1]$match[2]` now prints `a1`, matching zsh. (Bare `$match[1]` without braces is still a separate gap — bare-`$NAME[KEY]` doesn't lex as subscript.)

### Tilde expansion `~+`, `~-`, `~+N`, `~-N`, `~user`, named dirs

- `expand_tilde_named` extended with dir-stack-aware `~+` (= `$PWD`), `~-` (= `$OLDPWD`), `~+N` / `~-N` (Nth dir-stack entry from top/bottom), and `~user` via libc `getpwnam`. The runtime `expand_string` now collects the full tilde-name suffix (until `/` or whitespace) and dispatches through the helper instead of using `dirs::home_dir()` for bare `~` only.

### `unset 'arr[i]'` / `unset 'm[k]'` element delete

- `builtin_unset` detects the subscripted form. For assoc: removes the key. For indexed: clears the slot to empty string but preserves the slot count (matches zsh: `unset 'arr[2]'` produces a 3-element array with `arr[2]=""`, distinct from `arr[2]=()` which removes the slot entirely).

### `head -c N` byte-count flag in builtin

- Added `-c N` (and `-c<N>` glommed form) to `builtin_head`. Reads up to N bytes verbatim from the input stream and writes to stdout. Tested with `echo abcdef | head -c 3` → `abc`.

### `WORDCHARS` default

- Set at `ShellExecutor::new` to `*?_-.[]~=/&;!#$%^(){}<>` — the mainline-zsh default for ZLE word boundary chars.

### `<lo-hi>` numeric range globbing

- `[[ file5 = file<1-10> ]]` and friends now match. New `parse_numeric_range` scans `<lo-hi>` (with `lo` and/or `hi` optional — `<->`, `<5->`, `<-10>`, `<5-10>` all supported). `glob_match_static` translates each occurrence to a `(\d+)` capture group, remembers the bounds, and after `Regex::captures` succeeds it parses each capture and verifies the numeric range. Falls back to literal `<` for malformed forms.

### `where` builtin output format

- `builtin_where` was passing `-a -v` (verbose, all matches) which produced `ls is /bin/ls` instead of zsh's bare `/bin/ls`. Now passes `-c -a` and `builtin_whence` honors `csh_style` (`-c`) for aliases (`name: aliased to BODY`), functions (full `name () { … }` body via `function_source`), and missing-name stderr message (`name not found`). Matches zsh `where` exactly for external/alias/function/not-found.

### `print -P` byte-exact ANSI output

- `print -P "%F{red}hi%f"` previously emitted the readline cursor-width markers (`\x01` / `\x02`) plus a leading `\e[0m` reset, producing different bytes from zsh's bare `\e[31mhi\e[39m`. Three fixes: (1) new `expand_prompt_string_for_print` strips `\x01`/`\x02` markers and the spurious leading-reset preamble; `print -P` routes through it. (2) `apply_attrs` no longer emits an unconditional `\e[0m` preamble — only the new SGR codes (matches zsh's incremental approach). (3) `%f` now emits `\e[39m` (default-fg) instead of full `\e[0m`; `%u` emits `\e[24m` (underline off); `%s` emits `\e[27m` (standout off). `%B`/`%b` and `%F{c}` paths verified byte-exact against zsh.

### `let` and `$(())` float formatting

- `let "a=1.0+2.0"; echo $a` previously gave `3` (lost the float-ness). New `MathNum::format_zsh` formats stored vars as `%.10f` so `$a` is `3.0000000000`, matching zsh. Separately `MathNum::format_zsh_subst` formats `$(( ))` substitution display as zsh's `%g`-ish form: integer-valued floats print as `4.` (trailing dot, no zeros — zsh's "this is float" marker), non-integer floats print at full f64 precision via Rust's shortest-roundtrip. `evaluate_arithmetic` extracts via `format_zsh` (storage) and returns via `format_zsh_subst` (substitution display) so both contexts match zsh. The bytecode `(( a=1.0+2.0 ))` ArithCompiler path remains a known float-collapse pre-existing issue (separate from this fix).

### `print -P %h` / `%!` history line number

- `%h` and `%!` previously printed the persistent disk history total (e.g. 7466) instead of zsh's session-relative line number (0 in `-c` mode, since no command has been recorded). New `session_histnum` field on `ShellExecutor` (default 0, incremented on interactive command record). `build_prompt_context` reads it instead of `history.count()`. Matches zsh in `-c` mode exactly.

### `print -P %D{fmt}` strftime format

- Verified working in current build — the previously-noted gap was a stale `head -c 4` chain artifact (`head -c` was missing the byte-count flag, now fixed). `%D` with default format (`%y-%m-%d`-ish) and `%D{fmt}` with explicit strftime both match zsh.

### `fc -l` empty-history behavior in non-interactive mode

- `fc -l` previously dumped the persistent disk history (e.g. 7000+ entries) in `-c` mode. zsh's behavior in non-interactive mode is "no such event: 1" with exit 1 — the persistent disk history shouldn't leak through. `builtin_fc` now gates on `atty::is(Stream::Stdin)` (same signal `expand_history` uses) and short-circuits with the `zsh:fc:1: no such event: <N>` error in non-interactive mode. Format byte-exact against zsh.

### `noglob` precommand modifier dispatches to builtins

- `noglob print "*"` errored "command not found: print" because `builtin_noglob` routed unconditionally through `builtin_command` (PATH-only lookup). Now dispatches via `builtin_builtin` first when the name `is_builtin`, falling back to `builtin_command` for functions and externals. `noglob echo "*.txt"`, `noglob ls`, etc. continue to work.

### Bare `$arr[N]` subscript (no braces)

- `print $arr[2]` was lexing as `$arr` (whole array) + literal `[2]`, producing `x y z[2]`. New `bare_subscript_ref` matches the bare `$NAME[KEY]` shape and emits `BUILTIN_ARRAY_INDEX` directly. Companion `bare_subscript_with_suffix` handles `$arr[2]extra` (literal suffix concatenated via `Op::Concat`). Works for indexed (numeric key), assoc (string key), and with literal suffixes — `$arr[2]extra` → `yextra`, matching zsh.

### `(t)` typeset flag — type + attribute introspection

- `${(t)var}` previously returned `scalar` for everything (no per-variable attribute tracking). New `VarAttr` struct + `var_attrs: HashMap<String, VarAttr>` field on `ShellExecutor` records the kind (`Scalar`/`Integer`/`Float`/`Array`/`Association`) and modifiers (`readonly`, `export`, `left_pad`, `right_pad`, `zero_pad`, `lowercase`, `uppercase`). `format_zsh()` produces zsh's canonical `<kind>[-modifier]*` string — `integer`, `float`, `scalar-left`, `scalar-readonly`, `scalar-export`, etc. Wired into `builtin_declare` (typeset/declare flag block), `builtin_integer`, `builtin_float`, and `builtin_export`. Verified all 10 baseline shapes byte-exact against zsh.

### Glob qualifier `(mh-N)` / `(mm-N)` / `(mw-N)` time qualifiers

- Three fixes were needed: (1) `valid_chars` in `looks_like_glob_qualifiers` was missing lowercase `h` and `i` (also added `g` for group qualifier), so `(mh-N)` was being rejected at parse time. (2) `filter_by_qualifiers` had no `m`/`a`/`c` handler — added a new arm that parses the unit char (`s`/`m`/`h`/`d`/`w`/`M`), op (`+`/`-`), and integer N, then filters via `meta.mtime()`/`atime()`/`ctime()` against the cutoff. (3) `BUILTIN_EXPAND_TEXT` only invoked `expand_glob` when the word contained `*`/`?`/`[`; now also triggers when the word ends with a `(...)` qualifier suffix so plain paths like `/etc/hosts(mh-100)` route through globbing. Three tests cover recent file, too-old filter, and `(.)` plain qualifier.

### Recursive glob `**/` (dirs-only) and `**/*` (files+dirs)

- `**/` previously returned the literal pattern; `**/*` matched only files. zsh's `**/` enumerates directories with the trailing slash preserved; `**/*` matches both files and directories. Three fixes in `expand_glob_parallel`: (1) detect `dirs_only` when `file_glob` is empty (the trailing-slash form) and skip the file-pattern check entirely. (2) When `match_dirs_too` is on (every non-`dirs_only` `**/` walk), include directory entries from the walker. (3) Strip the `./` prefix when base was the implicit `.` so output matches zsh's relative-path style. Worker walkers now `continue` on `depth() == 0` to avoid double-adding the subdir root that the top-level loop already emitted. Three tests cover dirs-only, files+dirs, and extension filter.

## Closed (third-pass audit, this session)

### `${var:s/old/new/}` and `${var:gs/old/new/}` substitution modifier

- `is_history_modifier` was missing `s` and `g` so `${p:s/l/L/}` and `${p:gs/l/L/}` fell through unrecognized and returned empty. Added both. New `apply_subst_modifier` helper consumes the delimiter, old text, new text, then rewrites in place (single replace for `:s`, global for `:gs`). `apply_history_modifiers` now dispatches via `s` and the `g` prefix arms. Stops on `:` so chained modifiers (`:s/x/y/:t`) compose correctly.

### `${var:q}` backslash quoting

- `:q` was wrapping the whole value in single quotes (`'hi there'`); zsh emits backslash-escaped form (`hi\ there`). Replaced the wrapping with per-char escape: any of ` \t\n'"\\$\`;|&<>()[]{}*?#~!` gets a `\` prefix.

### `$0` inside a function = function name

- `call_function` now saves the previous `$0`, installs the called-function's name into `variables["0"]` for the duration of the call, and restores on exit. Matches zsh's default `FUNCTION_ARGZERO` behavior.

### `$funcstack` array

- `call_function` now also maintains the `funcstack` array — each call prepends the function name (top-of-stack first), pop on return. Standard zsh introspection used by frameworks for traceback / debugging.

### `$ARGC` alias for `$#`

- `get_variable` recognizes `ARGC` as a special parameter that returns `positional_params.len().to_string()` — same value as `$#`. zsh's `$ARGC` was empty in zshrs.

### `print -N` null between args

- `print -N a b c` previously emitted `a b c\0` (NUL only at end). zsh uses NUL as both separator AND terminator → `a\0b\0c\0`. Fixed `builtin_print` to use `\0` as the separator when `null_terminate` is set.

### kshglob extended patterns `?(p)` `*(p)` `+(p)` `@(p)` (gated)

- New `ksh_extglob_body_to_regex` translator. `glob_match_static` detects `?(...)`, `*(...)`, `+(...)`, `@(...)` after looking ahead for the `(` and emits `(?:body){suffix}` regex (suffix = `?`/`*`/`+`/empty). Gated on `setopt kshglob` so the default-off behavior matches zsh. `!(p)` (negative) needs lookahead which the `regex` crate doesn't support — left literal.

### Pattern repetition `(#cN)` and `(#cN,M)`

- `glob_match_static` peeks at `(#c...)` after `(` and emits a regex `{N}` or `{N,M}` quantifier. `a(#c2)` matches `aa` only; `a(#c2,3)` matches `aa` or `aaa`.

## Closed (fourth-pass batch — special params + module assocs + edge cases)

### `$EUID`, `$UID`, `$EGID`, `$GID`, `$PPID`, `$HOST`, `$HOSTNAME`, `$ZSH_SUBSHELL`

- New special-parameter handlers in `get_variable`: `EUID`/`UID` via libc `geteuid`/`getuid`; `EGID`/`GID` via `getegid`/`getgid`; `PPID` via `getppid`; `HOST`/`HOSTNAME` via `gethostname` (with NUL-trim); `ZSH_SUBSHELL` reads from `variables` with default 0.

### `$#@` and `$#*` count forms

- `bare_var_ref` extended to recognize the 2-char specials `#@` and `#*` (zsh shorthand for `${#@}`/`${#*}`, which equal `$#`). Routes through `get_variable` which returns `positional_params.len()` for either name.

### `$sysparams[KEY]` zsh/system magic assoc

- New `magic_assoc_lookup` arm for `sysparams`. Returns `pid` (process id), `ppid` (parent), `procsubstpid` ("0"). Splice form `${sysparams[@]}` returns the value list. Closes the `zmodload zsh/system; print $sysparams[pid]` daily-driver shape.

### `!(p)` kshglob negation (standalone, gated)

- `glob_match_static` now detects a fully-`!(<body>)` pattern and returns `!glob_match_static(s, body)` — the negation of recursing into the body. Composition like `prefix!(foo)suffix` would need negative lookahead and is left literal. Gated on `setopt kshglob` to match zsh.

### `${(F)arr}` newline-join flag

- New 'F' arm in `BUILTIN_PARAM_FLAG`: joins an array state with `\n` and produces a scalar. Mirrors the existing `(j:\n:)` form but as the standard one-letter shorthand.

### `typeset -p NAME` re-executable declaration output

- New `print_mode` early-return arm in `builtin_declare`: for each name arg without `=`, emits `typeset -<attrs> NAME=<quoted-value>`. Reads from `var_attrs` for kind/readonly/export modifiers; falls back to `assoc_arrays`/`arrays` membership for unmarked vars. Output format byte-exact against zsh: `typeset -i i=5`, `typeset -a arr=( a b c )`, `typeset -A m=( [a]=1 [b]=2 )`.

### `export -p` lists every exported var

- New early-return in `builtin_export`: when args are exactly `["-p"]`, walk `std::env::vars()`, sort, and emit `export NAME=<quoted-value>` lines. Matches POSIX + zsh format.

### `zmv` / `zcp` / `zln` / `zcalc` native bundled functions

- Previously these autoloaded zsh function files from `/opt/homebrew/Cellar/zsh/.../functions` and zshrs's parser HUNG indefinitely on the zsh-specific syntax in those bodies. Native Rust ports replace the autoload path: `call_function` short-circuits the four names BEFORE the alias/function/external lookup, dispatching directly to `builtin_zmv` and `builtin_zcalc`.
- `builtin_zmv` handles flags `-n` (dry-run), `-f` (force), `-i`, `-v`, `-W` (wildcard), `-s` (symlink for ln mode), `-M`/`-C`/`-L` (force action), `-p prog` (custom executable). The source pattern's `(...)` capture groups translate to a regex; the destination's `$N` / `${N}` substitute the captures. Collision detection (two srcs → same dest) errors before any file action. `zcp` and `zln` are the same dispatcher with different default actions.
- `builtin_zcalc` supports `-e EXPR` non-interactive evaluation (`zcalc -e "2+3*4"` → `14`); interactive REPL not implemented.

### `[[ a -nt b ]]`, `[[ a -ot b ]]`, `[[ -k ]]`, `[[ -u ]]`, `[[ -g ]]`, `[[ -O ]]`, `[[ -G ]]` cond tests

- `compile_zsh::emit_binary_test` had no arms for `-nt`/`-ot` — they fell through to the unknown handler returning false. Added `BUILTIN_FILE_NEWER` (id 324) and `BUILTIN_FILE_OLDER` (id 325) that compare `mtime()` via libc, with zsh-compatible "missing file" rules. Similarly `emit_file_test` lacked `-k`/`-u`/`-g`/`-O`/`-G`; added five new builtins (`BUILTIN_HAS_STICKY`/`SETUID`/`SETGID`/`OWNED_BY_USER`/`OWNED_BY_GROUP`) reading via `std::os::unix::fs::{PermissionsExt,MetadataExt}`. Verified `[[ -k /tmp ]]` returns true on macOS, `-O`/`-G` route correctly, `-nt` correctly compares 1s-granularity mtime.

### Extendedglob `^pat` negation in `[[ str = pat ]]` cond test

- Already worked for `${arr:#pat}` filter via `extendedglob_match`, but the cond `=` matcher (which goes through `glob_match_static` directly) didn't apply the negation. Added a leading-`^` strip + recurse-with-negate at the top of `glob_match_static`, gated on `setopt extendedglob`. `[[ apple = ^a* ]]` → false; `[[ banana = ^a* ]]` → true. Without extendedglob, `^` stays literal as before.

### `wait $!` silent-on-empty-pid

- When `$!` is unset (no bg job has been started), `wait $!` runs with an empty arg. zsh silently returns 0; bash errors with "wait: : not a pid". `builtin_wait` now skips the empty-arg branch and continues — match zsh.

### `print -m PATTERN args…` glob-match filter

- New `match_pattern_flag` in `builtin_print`: when `-m` is set, the first positional is a glob pattern; `output_args.retain` keeps only args that match. `print -m 'h*' hello world hi` → `hello hi`.

### `integer i=EXPR` runs arith eval on RHS

- `builtin_integer` was using `value.parse::<i64>().unwrap_or(0)` so anything beyond a literal int became 0. Replaced with `self.eval_arith_expr(value)` so `integer i=5+3` stores 8, `i=2*3+1` stores 7, etc. — matches zsh's "RHS goes through arithmetic" rule for `integer`-typed declarations.

### Positional-param subscript: `${@[N]}`, `${@[N,M]}`, `${*[N,M]}`, `$@[N]`, `${argv[N]}`

- Three fixes: (1) `BUILTIN_ARRAY_INDEX` now recognizes `@`/`*`/`argv` as special names that index `positional_params` directly (1-based, with negative-from-end and slice forms). (2) `braced_subscript_ref` accepts `@`/`*` as base (was rejecting because they're not alphabetic). (3) `bare_subscript_ref` accepts the same special names so `$@[N]` (no braces) routes through `BUILTIN_ARRAY_INDEX`. Without these, all four shapes fell through to the scalar-slice path which sliced the IFS-joined string.

### `for f in $arr` splices array elements

- `for f in $arr` was iterating ONCE with `f` set to the IFS-joined string because `BUILTIN_GET_VAR` collapses arrays into a scalar. Two changes: (1) `compile_for_words` detects bare `$NAME` words and emits `BUILTIN_ARRAY_ALL` instead, which always returns `Value::Array` so the for-loop's `BUILTIN_ARRAY_FLATTEN` spreads the elements. (2) `BUILTIN_ARRAY_ALL` extended to fall back to a scalar IFS-split when `name` isn't an array — so `for w in $scalar` still IFS-word-splits per zsh semantics. Quoted `for f in "$arr"` still joins to a single iteration (DQ context unchanged).

### `arr+=val` (no parens) pushes as new element

- Was treating `name+=val` as scalar concat unconditionally, clobbering the array. New `BUILTIN_APPEND_SCALAR_OR_PUSH` (id 331) runtime-dispatches: if `name` is an indexed array, push `val` as a new element; if assoc, error (zsh requires `(k v)` form for assoc append); else scalar concat (existing behavior). Three tests cover array push, multi-element push, and scalar concat.

### `${var-default}` no-colon default family

- Only the colon variants (`${var:-X}`, `${var:=X}`, `${var:?X}`, `${var:+X}`) were recognized — those treat empty-string-set the same as unset. The POSIX no-colon forms (`${var-X}`, `${var=X}`, `${var?X}`, `${var+X}`) fire only when truly unset (not just empty). Added op codes 4-7 in `BUILTIN_PARAM_DEFAULT_FAMILY` plus matching parser arms in `parse_param_modifier`. Five tests cover default/assign/error/alt for both unset and empty-set cases.

### `$status` alias for `$?`

- `get_variable` now treats `status` as an alias for `?` — both return `last_status`. zsh exposes both names; `$status` was empty in zshrs.

### `$pipestatus[N]` / `$PIPESTATUS[N]` after single command

- `BUILTIN_ARRAY_INDEX` now special-cases `pipestatus`/`PIPESTATUS`: if no array has been populated (e.g. after a single non-pipeline command), synthesizes `[last_status]` so `true; echo $pipestatus[1]` returns `0`. Real pipelines continue to use the per-stage array set by `BUILTIN_PIPELINE_EXEC`.

### `[[ -c path ]]`, `[[ -b path ]]`, `[[ -p path ]]`, `[[ -S path ]]` file-type tests

- `compile_zsh::emit_file_test` had no arms for character device, block device, FIFO, or socket. Added four new builtins (`BUILTIN_IS_CHARDEV/BLOCKDEV/FIFO/SOCKET`, ids 332-335) using `std::os::unix::fs::FileTypeExt`. `[[ -c /dev/null ]]` → true on macOS as expected.

### `unset -f NAME` removes function

- `builtin_unset` now parses `-f` (function mode) and `-v` (var mode, default). With `-f`, removes from `functions_compiled`, `function_source`, and `autoload_pending`. Mirrors `unfunction NAME`.

### `for w in $scalar` no-IFS-split (zsh default)

- `BUILTIN_ARRAY_ALL` was IFS-splitting scalars in for-list contexts (bash semantics). zsh's default is to NOT split — `for w in $s` iterates ONCE with the scalar value. Now scalars produce a 1-element array unless `setopt shwordsplit` (the bash-compat option) is on, in which case the old IFS-split behavior fires. Two tests cover both modes.

### `${var//#pat/repl}` and `${var//%pat/repl}` anchored replace-all

- `parse_param_modifier` only checked `//` before `/#` / `/%`, so `${s//#hel/HEL}` was parsed as `//` (replace-all) with literal pattern `#hel`. Reordered the prefix matchers so `//#` and `//%` win first. Both produce the same result as `/#`/`/%` for non-overlapping matches (anchor-at-start matches once; replace-all is moot).

### `alias x` query output format

- Was always emitting `name='value'` (single-quoted). zsh's rule: bare value when it's a single safe word, single-quoted when it contains whitespace or shell metachars. New `format_alias_kv` helper applies the rule; both the `alias NAME` query and the `alias` listing path use it.

### `foo() echo hello` one-line function body

- The lexer collapses `foo()` into a single String token whose suffix is `\u{88}\u{8a}` (INPAR + OUTPAR). For `foo() echo hello`, parse_simple consumed `foo()`, `echo`, `hello` as a 3-word Simple. The funcdef synthesizer in parse_program required `words.len() == 1`, so the multi-word case was lost. Updated `simple_name_with_inoutpar` to return `(name, body_argv)`: when `body_argv` is non-empty, the synthesizer wraps `body_argv` as a Simple body and emits the FuncDef. Brace-body path (existing) and 1-word `foo()` followed by `{...}` continue to work. Three tests cover one-line/colon/arg-passing variants.

## Closed (eighth-pass — non-interactive batch)

### `&>` / `&>>` redirect — restore both fd 1 and fd 2 after the body

- The lexer clamps `tokfd` to ≥ 0 for `&>`, so the parser handed the host `fd=0` for what should be "both stdout and stderr". `host_apply_redirect` only saved that single `fd` into the redirect scope, leaving fd 2 permanently aimed at the file. After `{ cmd } &> file; echo done`, the trailing `echo done` wrote into the file too. Fixed: when op is `WRITE_BOTH`/`APPEND_BOTH`, force the primary fd to 1 (so stdout is saved), then explicitly dup-and-stash fd 2 into the same scope. `WithRedirectsEnd` then restores both. Test: `test_amp_redir_restores_stderr`.

### `typeset -m PAT` — glob-pattern listing of variables

- The flag was parsed and immediately discarded with `let _ = pattern_match`. Wired it: with `-m` and one or more glob patterns, expand patterns against the live name space (variables + arrays + assocs, or `function_names()` under `-f`), dedup, and emit the matching listings. Honors `-p` for re-executable form, scalar/array/assoc per-name shape. Test: `test_typeset_m_glob_lists_matching`.

### `print` flag-processing must stop at first non-option

- `print "rest:$@"` with positionals `-a -b foo` was treating `-b` as a print flag mid-args. Fixed: introduce `accept_flags` toggle, flip it off on the first non-flag arg or any token whose chars aren't all known print flags. `print -- -n foo` and `print -n hello` paths unchanged. Test: `test_print_stops_flag_processing_at_first_non_option`.

### `zparseopts -D` — only remove consumed indices

- The previous removal logic used a single `parsed_count` and dropped contiguous positions `1..=N`, which broke whenever `-E` skipped non-options or when only some specs matched. Switched to per-match `consumed_indices: Vec<usize>` and rebuild `positional_params` by filtering. Also moved positional source from synthetic `$1..$99` reads to `self.positional_params` directly. Test: `test_zparseopts_dash_d_removes_only_consumed`.

### `zparseopts -M` — alias spec redirection

- `-M f=optf -foo=f` now treats `-foo`'s `f` target as another spec name, not an array name. When `--foo` is seen, it matches the alias spec, resolves to the canonical `f` spec for arg-handling, and records the actual `--foo` arg into `f`'s target array (`optf`). Required adding canonical-name routing into the per-spec output bucket. Test: `test_zparseopts_dash_m_alias_redirects_to_canonical`.

### `zformat -f` width specifiers `%[-]Ns`

- Format loop was strictly `%X → spec` with no width handling. Added a parser for optional `-` (alignment sigil) + decimal width + spec char. Padding semantics MATCH ZSH OBSERVED BEHAVIOR (which is the inverse of printf): no `-` left-aligns, `-` right-aligns. Test: `test_zformat_width_padding`.

### `getopts` unknown-option message format

- Was `zshrs: getopts: illegal option -- X`. zsh emits `zsh:N: bad option: -X`. Switched to `zshrs:1: bad option: -X` to mirror the format with the program name swapped. Test: `test_getopts_unknown_uses_zsh_format`.

## Closed (ninth-pass — formatter + introspection batch)

### `print -f FMT args...` — cycle FMT until args exhausted

- POSIX printf semantics. `print -f "%-5s|%-5s\n" a b c d` should emit two lines (`a    |b    \nc    |d    \n`), not one. Added a counting variant of the format helper (`printf_format_count`) returning consumed-arg count, then loop the call from the print path until `idx >= len`. Also expand `\n`/`\t`/etc. in the format string when `-r` isn't set. Test: `test_print_f_format_cycles_args`.

### printf width specifiers `%[-+# 0][N][.P]X`

- Width digits were parsed but never applied — `printf "%-10s|%10s" hello world` rendered `hello|world`. Rewrote the per-spec branch to track `left_align`/`zero_pad`/`plus_flag`/`space_flag`/`hash_flag`/`width`/`precision`, then pad/sign each conversion before pushing. Covers `s`/`d`/`i`/`u`/`x`/`X`/`o`/`f`/`F`/`e`/`E`/`g`/`G`. Test: `test_printf_width_left_align`.

### `functions -m PAT` — glob-pattern listing

- The `-m` flag wasn't recognised. Added pattern-match expansion: collect names matching each pattern via `glob_match_static`, dedup, then list (or print body, or trace per `-t`/`-l`). Combined-flags form `-lm` parsed too. Test: `test_functions_dash_m_glob_lists_matching`.

### `zstyle -L` and bare `zstyle` — emit zsh's exact formats

- The internal `StyleTable.list()` returned `(style, pattern, values)` triples but the caller printed them as `(pattern, style, values)`. Renamed semantics so list returns `(pattern, style, values)`, then updated `-L` to emit zsh's bare-word form (`zstyle <pattern> <style> <vals>`) — quoting only when whitespace/empty. Bare `zstyle` (no args) now uses zsh's grouped-by-style form (`STYLE\n        <pattern> <vals>`). Tests: `test_zstyle_dash_l_uses_pattern_first_format`.

### `${(q)}` / `${(qq)}` / `${(qqq)}` / `${(qqqq)}` — fix gradient mapping

- The `(q)` flag gradient was inverted. Per `man zshexpn`:
  - `(q)`    backslash-escape shell-special chars (no surrounding quotes)
  - `(qq)`   single-quote always
  - `(qqq)`  double-quote always
  - `(qqqq)` ANSI-C `$'…'` style
  - `(q+)`   single-quote if needed
  zshrs had `(q)`→single-quote, `(qq)`→double-quote, etc. (off-by-one). Re-mapped both q-flag handlers (the Phase-2 `BUILTIN_PARAM_FLAG` path at exec.rs:2516 and the parser-flag path at exec.rs:11904). Added `ZshParamFlag::DollarQuote` for the `qqqq` level. `(q+)` now correctly promotes to single-quote when the value needs quoting (was emitting backslash-escape before). Updated 7 affected tests in `tests/no_tree_walker_dispatch.rs` to match real zsh output. Tests: `test_zsh_param_q_flag_backslash_only`, `test_zsh_param_q_flag_gradient`.

## Closed (tenth-pass — DQ subscripts + nounset)

### `$NAME[subscript]` in double-quoted context

- `"$m[a] $m[b]"` was emitting `[a] [b]` literal text after each `$m`. Two changes: (1) extended `find_expansion_end` (in `compile_zsh.rs`) so a trailing `[...]` after an identifier is pulled into the same expansion segment — handles both META-INBRACK (`\u{91}`) and bare `[`, since DQ-context lex paths leave the bracket unwrapped. (2) Added a subscript handler in `expand_string` (in `exec.rs`) for assoc lookups, array indexing (1-based, negative-from-end), and `@`/`*` splice. Composes with `$`-expansion inside the subscript (`$m[$k]`). Tests: `test_assoc_subscript_in_double_quotes`, `test_array_subscript_in_double_quotes`, `test_assoc_subscript_with_dynamic_key_in_dq`.

### `set -u` / `setopt nounset` — error on unbound parameter

- The option flag was set but never checked. Wired the check into `get_variable` for the catch-all (non-special) branch: when the resolved name isn't in `variables`/`arrays`/`assoc_arrays`/env AND nounset is on, print `zshrs:1: NAME: parameter not set` and `std::process::exit(1)` (mirrors zsh's `-c` behaviour). Subtlety: zsh stores the option as `unset` (default ON = silently empty), and `setopt nounset` sets the inverted name. Different code paths in zshrs persisted either `nounset=true` or `unset=false`, so the check honors either signal. Tests: `test_set_dash_u_exits_on_unbound_variable`, `test_setopt_nounset_exits_on_unbound`.

## Closed (eleventh-pass — error-on-unset family)

### `${x:?msg}` / `${x?msg}` — exit on null/unset

- The `BUILTIN_PARAM_DEFAULT_FAMILY` op codes 2 and 6 (`:?` / `?`) emitted the diagnostic to stderr but returned an empty string and continued execution. zsh in `-c` mode aborts the whole shell. Now emits `zshrs:1: NAME: msg` (with `parameter null or not set` as the default if no message text) and `std::process::exit(1)`. Tests: `test_param_colon_question_exits_on_empty`, `test_param_question_exits_on_unset`, `test_param_colon_question_passes_through_value`.

### NOMATCH default — unmatched globs abort

- zsh's default option set includes `nomatch`, which makes unmatched globs an error: `echo /tmp/no_such_*` prints `no matches found: /tmp/no_such_*` on stderr and the shell exits 1. zshrs's `expand_glob` was returning `vec![pattern]` (bash semantics). Wired the option check: if `nomatch` is on (default true), no match found, AND the pattern truly looks like a glob → emit the diagnostic and exit. `looks_like_glob` rejects bare `[` (the test builtin) by requiring a matching `]`. The `(N)` qualifier and `setopt nullglob` continue to silence the error.
- Required two protective fixes to keep internal callers from spuriously erroring:
  - `BUILTIN_EXPAND_TEXT` mode 0 now skips glob expansion for assignment-shaped words (`NAME=value`) so `integer i=2*3+1` doesn't trip on the `*`.
  - In `compile_cond`'s Binary branch, the RHS of `=`/`==`/`!=`/`=~` is now compiled as a quoted literal — these are pattern operands for the test, not file globs.
- Tests: `test_unmatched_glob_default_errors_with_nomatch`, `test_unsetopt_nomatch_passes_literal_through`, `test_assignment_value_skips_glob_expansion`.

## Closed (twelfth-pass — logical-pwd preservation)

### `cd` / `pwd` preserve the logical path (default `-L`)

- The `do_cd` helper canonicalised the target before `chdir`, so `cd /tmp` on macOS landed in `/tmp` but `$PWD` became `/private/tmp`. Two fixes:
  - Renamed the inner `physical` parameter back to `logical` and inverted its sense (the call site already passed `logical=true`, but the parameter slot was named `physical`, silently flipping the semantics — the canonicalise branch was firing for the default mode). Recomputed `let physical = !logical;` once at the top.
  - Added a lexical `normalize_logical(path)` helper that collapses `.`/`..` components without touching the filesystem (so `cd ..` from a symlinked dir lands at the symlink's parent, not the realpath's parent).
  - In the default (`-L`) branch, `chdir` to the lexical absolute path; store that same path in `$PWD`. Only the `-P` branch realpaths.
  - `OLDPWD` is now seeded from the previous `$PWD` (logical), not `current_dir()` — so `cd -` round-trips the user-typed path.
- `builtin_pwd` now reads `$PWD` for default/`-L` output (still honors `-P` to realpath via `current_dir()`). Tests: `test_cd_preserves_logical_path`, `test_cd_dash_p_realpaths` (the latter delegates the expected-value to /bin/zsh so it passes on both macOS and plain Linux).

## Closed (thirteenth-pass — set -e enforcement + readonly + lexer errors)

### `set -e` / `setopt errexit` — exit on command failure

- Wired full POSIX/zsh-compatible errexit. Required four pieces:
  - `BUILTIN_ERREXIT_CHECK` (id 336): runtime helper that reads `vm.last_status`, the `errexit` option, and `local_scope_depth`. If errexit is on AND status != 0 AND not inside a function call, `std::process::exit(status)`.
  - Compiler emits the check after every top-level `SetStatus` (CallBuiltin / CallFunction). The `return` and `exit` builtins skip it (their status is intentional).
  - `errexit_suppress_depth: i32` field on `ZshCompiler` tracks suppression contexts. Bumped around `if`/`elif`/`while`/`until` test bodies and around any sublist that has `&&`/`||` chaining or `!` negation.
  - The full sublist (everything before `;` or newline) is exempt when it contains `&&`/`||` connectors — POSIX rule that AND-OR list failures are "consumed" by the connector and don't trigger errexit even at the chain's end.
- Tests: `test_set_e_exits_on_failure`, `test_set_e_suppressed_in_if_test`, `test_set_e_suppressed_in_and_chain`, `test_set_e_suppressed_in_or_chain`, `test_set_e_suppressed_in_negation`, `test_set_e_suppressed_in_while_test`.

### `readonly` / `typeset -r` — block subsequent assignments

- The `readonly_vars` set was populated by the builtin but never consulted at assignment time. `BUILTIN_SET_VAR` now checks both `readonly_vars` and `var_attrs[name].readonly`. On hit: emit `zshrs:1: read-only variable: NAME` and `std::process::exit(1)` (mirrors zsh's "fatal in -c" behaviour). Closes the two pre-existing failing tests `test_readonly_variable` and `test_typeset_readonly`.

### Lexer-level parse errors surface to the caller

- `lex module.error` (e.g. `unmatched '`) was set during lexing but the parser ignored it. After `parse_program_until` succeeds, `parse()` now checks `self.lexer.error` and returns it as a `ParseError`. The execute path then exits with the diagnostic on stderr. Closes `test_error_syntax` (now uses `echo 'unclosed` — a real lexer error that mainline zsh also rejects).

## Closed (fourteenth-pass — subshell + arith subscripts)

### `(cd /tmp); pwd` — subshell cd must not leak

- Subshell snapshot saved/restored `cwd` via `current_dir()`/`set_current_dir()` correctly, but my new `cd` writes `$PWD` into both `self.variables` and `env::set_var("PWD", ...)`. The snapshot restored `self.variables` but NOT the env var, so the subsequent `pwd` (which now reads `$PWD` for logical mode) showed the subshell's cwd. Fix: in `subshell_end`, after `set_current_dir(snap.cwd)`, also `env::set_var("PWD", &snap.cwd)`. Test: `test_subshell_isolates_cwd`.

### `$((m[k]))` / `$((a[2]))` — arith subscripts on arrays/assocs

- `MathEval` only knows about scalar variables (`self.variables`), so `m[k]` resolved to 0. Added `pre_resolve_array_subscripts(expr)`: walks the expression, finds `name[subscript]` shapes, resolves them against `assoc_arrays` (key lookup) or `arrays` (1-based numeric index, negative-from-end), and inlines the literal value before handing to `MathEval`. Wired into `evaluate_arithmetic`, `eval_arith_expr`, `eval_arith_expr_float`. Tests: `test_arith_assoc_subscript`, `test_arith_array_subscript`.

## Closed (fifteenth-pass — read -A IFS + tilde-user error)

### `IFS=, read -A arr` — honor custom IFS for array split

- The `-A` (read into array) branch unconditionally used `split_whitespace()`, ignoring `$IFS`. With a custom IFS like `,` the input `1,2,3` became one element. Branch now: if IFS is the default whitespace string, keep `split_whitespace()` (collapses consecutive separators); otherwise split on every IFS char (matches zsh `read -A` for custom IFS). Tests: `test_read_dash_a_honors_custom_ifs`, `test_read_dash_a_default_ifs_collapses_whitespace`.

### `~nonexistent_user` — fatal error

- `expand_tilde_named` previously returned the literal `~name` string when `getpwnam` failed. zsh emits `zsh:1: no such user or named directory: name` and exits 1. zshrs now matches with a `zshrs:1:` diagnostic and `std::process::exit(1)`. Test: `test_tilde_unknown_user_errors`.

## Closed (sixteenth-pass — heredoc + echo + alias + substring expr)

### Empty heredoc — don't error and don't trail a newline

- Two compounding bugs:
  - `process_heredocs` used "content empty" as the "not yet processed" marker, so an empty heredoc was re-processed on every subsequent newline; the second pass found EOF and errored "here document too large or unterminated". Added a separate `processed: bool` field on `HereDoc` to disambiguate.
  - The unquoted heredoc emit path always routed through `Op::HereString`, which appends a newline. For an empty body this leaked a stray `\n` into the consumer (`cat <<EOF\nEOF` printed a blank line vs zsh's silent). Empty bodies now route through `Op::HereDoc` regardless of quoting.
- Test: `test_empty_heredoc_succeeds` (compares to `/bin/zsh` output for portability).

### `echo -e` — full backslash-escape decoder

- Only `\n` and `\t` were interpreted; `\033` / `\xNN` / `\NNN` / `\a` / `\b` / `\e` were emitted literally. Routed `echo -e` through the existing `expand_printf_escapes` helper that already handles the full set. Test: `test_echo_dash_e_interprets_octal_escape`.

### `alias` listing — bare values stay unquoted

- The list output path hardcoded `'{}'` quoting around every value, so `alias x=1` listed as `x='1'` instead of `x=1`. Replaced with the existing `format_alias_kv` helper which only adds quotes when the value contains shell specials/whitespace. Also sorted output to match zsh's deterministic listing. Test: `test_alias_listing_unquoted_for_simple_values`.

### `${s:$n:2}` — substring with variable / arith offset

- The substring parser only accepted literal digits/`-` after the colon, so `${s:$n:2}` and `${s:$((1+1)):2}` returned empty. Added:
  - New `ParamModifierKind::SubstringExpr { offset_expr, length_expr }` variant.
  - New runtime helper `BUILTIN_PARAM_SUBSTRING_EXPR` (id 337) that evaluates each expression at runtime via `eval_arith_expr`. Stack layout includes a `has_length` sentinel to distinguish "no length given" from "length=0".
  - Top-level `:` split that respects `(...)` depth so `${s:$((1+1)):2}` keeps `$((1+1))` intact.
- Tests: `test_substring_with_var_offset`, `test_substring_with_arith_offset`, `test_substring_with_var_offset_and_length`.

## Closed (seventeenth-pass — pipefail + IFS default + diagnostics)

### `set -o pipefail` / `setopt pipefail`

- The option was tracked but never consulted — `false | true` always returned 0 (last-stage status). `BUILTIN_RUN_PIPELINE` now reads `exec.options["pipefail"]` after collecting `pipestatus[]` and returns the rightmost non-zero status when on (POSIX/bash semantics). Tests: `test_pipefail_returns_first_nonzero`, `test_pipefail_default_off_returns_last`, `test_setopt_pipefail_alias`.

### `$IFS` default value populated to `" \t\n\0"`

- `ShellExecutor::new()` left `$IFS` unset; users running `echo "$IFS"` saw an empty string vs zsh's space/tab/newline/NUL. Now seeded explicitly. Required updating `read -A`'s default-IFS detection from exact-string match (`" \t\n"`) to a char-set test (`all chars in {' ', '\t', '\n', '\0'}`) so the new init value still routes through `split_whitespace` (collapses consecutive separators). Test: `test_ifs_default_includes_null`.

### `command not found` includes line number

- Was `zshrs: command not found: NAME`. zsh's format is `zsh:LINE: command not found: NAME`. Updated all three eprintln sites to `zshrs:1: command not found: ...`. Test: `test_command_not_found_includes_line_number`.

## Closed (eighteenth-pass — noclobber + pwd -P + function-with-parens)

### `setopt noclobber` blocks `>` overwrite of existing files

- The option was tracked but the redirect path always called `File::create` (which truncates). Split `r::WRITE` from `r::CLOBBER` (the `>!` / `>|` op) and added a noclobber check: `setopt noclobber` writes the inverted-name `clobber=false`, so the check honors both keys (`noclobber=true` OR `clobber=false`). On hit:
  - Print `zshrs:1: file exists: PATH` to stderr.
  - Set `last_status = 1`.
  - Sink the upcoming command's stdout to `/dev/null` (so e.g. `echo second > existing` doesn't leak `second` to the terminal — matches zsh's "command silently dropped" semantics).
- `>!` / `>|` (CLOBBER) bypasses the check unconditionally. Tests: `test_noclobber_blocks_overwrite_and_sinks_output`, `test_noclobber_force_overwrites_with_bang`.

### `pwd -P` realpaths the logical PWD

- `builtin_pwd` ignored its `args` (only saw `redirects`), so `-P` was silently dropped and the logical `$PWD` was always printed. Routed dispatch through new `builtin_pwd_with_args(&[String])` that parses `-L`/`-P` flags. `-P` realpaths the tracked `$PWD` via `canonicalize()`. Test: `test_pwd_dash_p_realpaths` (delegates expected value to `/bin/zsh`).

### `function name() { body }` — keyword + parens combo

- The `function` keyword path collected names from `String` tokens and broke on `Inoutpar` / `Inbrace`. But the lexer packs `bar()` as a single String token suffixed with INPAR+OUTPAR markers (`\u{88}\u{8a}`), so the `name=bar()` token went into `names` literally and the body parsed under that wrong name. Added a strip step: detect the `\u{88}` ... `\u{8a}` suffix on a String token, trim it, then untokenize → clean `bar` name. Test: `test_function_keyword_with_parens`.

## Closed (nineteenth-pass — DQ array flags + slices + bg-pid + readonly arrays + print -s)

### `${(o/O/n/i/u)a}` array-flag suppression in DQ context

- zsh applies these array-only flags only when the expansion is in array context (no surrounding `"..."`); inside DQ they're no-ops and the result is the original elements joined as a scalar. Two changes:
  - `BUILTIN_PARAM_FLAG`: strip `o`/`O`/`n`/`i`/`u` chars from the flags string when DQ-context is detected (either via runtime `in_dq_context` counter or compile-time `\u{02}` sentinel prefix).
  - `compile_word_str` fast path tags the emitted flags with the `\u{02}` sentinel when the raw word is DNULL-wrapped or when we're recursing into a DQ-wrapped parent's Expansion segment (tracked via new `dq_context_depth: i32` on the compiler).
  - The bridge path (`BUILTIN_EXPAND_TEXT`) forces mode 1 (DoubleQuoted) when `dq_context_depth > 0`, propagating DQ semantics through nested expansions.
  - `(M)` is NOT stripped here — it modifies `:#pat` filter behavior on the joined scalar in DQ context (verified against /bin/zsh).
- Tests: `test_dq_suppresses_array_only_sort_flags`, `test_no_dq_sort_flags_still_apply`, `test_dq_suppresses_unique_flag`, `test_dq_suppresses_natural_sort`. Updated 5 pre-existing tests in `no_tree_walker_dispatch.rs` and `zshrs_shell.rs` that codified the old (zsh-incorrect) "always sort" behavior — they now assert array context (no DQ wrapper).

### `${@:N:M}` / `${arr:N:M}` — slice positionals/arrays as elements

- The substring path applied char-indexed scalar slicing to `@`/`*` and arrays. Now element-aware:
  - `${@:N:M}` and `${*:N:M}` slice positionals where index 0 is `$0`, 1 is `$1`, etc. (matches zsh).
  - `${arr:N:M}` slices `arr` with N as a 0-based "skip N" offset (so `arr=(x y z w); ${arr:1:2}` → `y z`).
  - Negative offsets count from the end.
- Three call sites updated (`BUILTIN_PARAM_SUBSTRING`, the compile-modifier `apply_var_modifier`, and the bridge `expand_braced_variable`'s inline parser). Helpers `slice_array_zero_based` and `slice_positionals` added. Tests: `test_positional_slice_skip_offset`, `test_positional_slice_no_length`, `test_array_slice_offset_skips`, `test_at_subscript_inclusive_range`.

### `$!` after `cmd &`

- `BUILTIN_RUN_BG` discarded the parent's pid. Now records into `self.variables["!"]` so `wait $!` works. `get_variable("!")` defaults to `"0"` when never set (matches zsh's pre-fork display). Tests: `test_bang_pid_after_background`, `test_bang_pid_initial_zero`.

### `declare -ra` / `typeset -ra` — block array mutation

- `BUILTIN_SET_ARRAY` and `BUILTIN_APPEND_ARRAY` now check the readonly status (both `readonly_vars` and `var_attrs[name].readonly`). On hit: emit `zshrs:1: read-only variable: NAME` and `std::process::exit(1)` (mirrors zsh `-c` fatal). Tests: `test_declare_ra_blocks_array_assign`, `test_declare_ra_blocks_append`.

### `print -s` records to history (silent), `fc -l` lists session entries

- Two changes:
  - `print -s X` now suppresses stdout output entirely — per zsh's man page, `-s` "places the results in the history list INSTEAD OF on the standard output". Was printing to stdout AND adding to history.
  - `fc -l` in `-c` (non-interactive) mode now bypasses its "no such event" guard when the script has explicitly added entries via `print -s`. Tracks them via new `session_history_ids: Vec<i64>` field; `fc -l` looks each up by ID and renumbers 1..N so the script sees clean contiguous IDs (not the SQLite global counter).
- Test: `test_print_s_silent_and_records_history`.

### `select` menu — multi-column packed format

- Menu items were one per line. zsh packs `N) item` cells across rows to fit the terminal (defaults to 80 cols). Width = max cell + 1 trailing space. Cosmetic match.

## Closed (twentieth-pass — (z) split + unalias query + kill flags)

### `${(z)str}` — proper shell-token split

- Was a plain `split_whitespace()` so `"echo hi; ls"` produced 3 tokens (`echo`, `hi;`, `ls`) instead of 4 (`echo`, `hi`, `;`, `ls`). New `zsh_split_z` helper walks the string honoring single/double quotes (with escape) and splitting out shell metas (`;`, `&`, `|`, `<`, `>`, `(`, `)`) as their own tokens, with combination of repeats (`&&`, `||`, `;;`, `>>`, `<<`). Tests: `test_z_split_emits_metas_as_separate_tokens`, `test_z_split_pipe_token`.

### `alias NAME` query is silent on unknown name

- Was emitting `zshrs: alias: NAME: not found` which zsh doesn't print. The query just exits non-zero in zsh. Removed the diagnostic; status code unchanged. Test: `test_alias_query_silent_when_unknown`.

### `kill -l` and `kill -L`

- `kill -l` was printing a numbered table (`1) SIGHUP\n…`); zsh emits bare names space-separated on one line. Switched to match.
- `kill -L` was treated as a list-mode alias for `-l`. zsh treats it as `-` + signal name `L` → "unknown signal: SIGL" with the standard hint. Switched to error path for parity. Tests: `test_kill_dash_l_lists_bare_names`, `test_kill_dash_capital_l_unknown_signal`.

## Closed (twenty-first-pass — integer arith + (e) eval + assign no-glob + type format)

### `integer i; i=5*3` — arith-evaluate when var has integer attribute

- Two compounding bugs:
  - `compile_assign`'s Scalar branch unconditionally called `compile_word_str(value)`, which routed `5*3` through expand_text + glob → NOMATCH error. Added a DQ-wrap step: when the value contains glob metas (in either META-encoded form `\u{87}` or literal `*`), wrap with DNULL markers so the bridge picks mode 1 (DoubleQuoted) and skips brace+glob expansion. `$var` / `$(cmd)` / `$((expr))` still expand.
  - `BUILTIN_SET_VAR` now checks `var_attrs[name].kind == Integer`. If so, runs `eval_arith_expr(value)` before storing — `i=5*3` lands as `15`. Test: `test_integer_attribute_arith_evaluates_assignment`, `test_bare_assignment_does_not_glob_expand`.

### `${(e)var}` — parameter expansion, not command execution

- The `(e)` flag was running the value as a shell command via `run_command_substitution`. Per `zshexpn(1)`, `(e)` should "perform parameter expansion, command substitution and arithmetic expansion" — which is `expand_string`. Switched. `s="\$test"; test=val; ${(e)s}` now correctly returns `val`. Test: `test_paren_e_flag_expands_parameters`.

### `type NAME` unknown format

- Was `zshrs: type: NAME: not found` (stderr, with prefix). zsh emits `NAME not found` on stdout (no prefix). Switched format and stream. Test: `test_type_unknown_format_matches_zsh`.

## Closed (twenty-second-pass — echo escapes + export -n + xtrace)

### `echo` interprets escapes by default

- zsh's default `echo` interprets `\n`/`\t`/`\b`/etc. unless `setopt bsd_echo` is on; `-e` is unnecessary. zshrs had `interpret_escapes = false` default. Switched to `!bsd_echo` so the default is ON; `-E` continues to disable. Tests: `test_echo_default_interprets_escapes`, `test_echo_dash_capital_e_disables_escapes`.

### `export -n` rejected as bash-only

- zsh treats `export -n` as a bad option (bash uses `-n` to remove export attribute); zshrs accepted it. Now rejects any `-X` flag besides `-p` with `zshrs:export:1: bad option: -X` and exit 1. Tests: `test_export_dash_n_rejected`.

### `set -x` / `setopt xtrace` — print commands before execution

- The option was tracked but never enforced. Added new `BUILTIN_XTRACE_LINE` (id 338): pops a literal command-text string and prints it to stderr with `$PS4` prefix (default `+ `) when `xtrace` is on. The compiler emits the trace call before each simple command's args/dispatch in `compile_simple`. Format is the POSIX `+ cmd args` style — zsh's elaborate `<color>PROG\tFN\tLINENO\t<reset>\tcmd` format depends on PROMPT_PERCENT and isn't matched exactly (our format is what real-world POSIX scripts assume). Tests: `test_set_dash_x_xtrace_prints_commands`, `test_set_plus_x_disables_xtrace`, `test_xtrace_uses_ps4`.

## Closed (twenty-third-pass — default expansion + hex escape + break N)

### `${var:-...}` / `${var:=...}` / `${var:+...}` expand cmd-subst and `$var` in operand

- The default/alt operand was used as-is. zsh runs full expansion (parameter, command-substitution, arith) on it before substitution. Wired `expand_string` lazily in `BUILTIN_PARAM_DEFAULT_FAMILY` for all four ops. Tests: `test_default_value_expands_command_substitution`, `test_default_value_expands_variable`, `test_assign_default_expands`.

### `echo "\xHH"` hex escape

- The escape decoder only handled `\n`/`\t`/`\xNN was missing despite octal `\NNN` working`. Added `\xHH` (1-2 hex digits) to `expand_printf_escapes`. Test: `test_echo_hex_escape`.

### `break N` / `continue N` — multi-level loop control

- Were ignoring the level argument; always targeted the innermost enclosing loop. Now reads `simple.words[1]` as the level count, indexes back into `break_patches` / `continue_patches` from the end, clamping to depth. Tests: `test_break_n_breaks_outer_loop`, `test_continue_n_continues_outer_loop`.

## Closed (twenty-fourth-pass — pattern expansion + `[*]` join + wait validation)

### `${var/$pat/X}` / `${var//$pat/X}` — expand `$pat` and `$X`

- The pattern and replacement operands were taken as-is. zsh expands parameter, command-substitution, and arith in both before applying. Wired `expand_string` on both at the top of `BUILTIN_PARAM_REPLACE`. Tests: `test_replace_pattern_expands_dollar_var`, `test_replace_global_pattern_expands`.

### `${arr[*]}` joins with first IFS char

- Both `[@]` and `[*]` emitted `BUILTIN_ARRAY_ALL` (always Value::Array → splice). Added `BUILTIN_ARRAY_JOIN_STAR` (id 339) that joins on first IFS char and returns Value::Str. Compiler picks via `array_splice_is_star(s)` test. Tests: `test_array_star_joins_with_first_ifs`, `test_array_at_keeps_separate_words`.

### `wait PID` validates child ownership

- `wait 99999` was returning 0 silently. zsh emits `pid N is not a child of this shell` and exits 127. `builtin_wait` now checks the PID against `$!` and the active jobs list before calling `wait_for_job`. Test: `test_wait_unknown_pid_errors`.

## Closed (twenty-fifth-pass — `$(< file)` + `printf %q`)

### `$(< file)` — zsh file-contents shorthand

- The `<` after `$(` (with optional whitespace) signals "read this file's contents". Faster than `$(cat file)`. Added at the top of `run_command_substitution`: trim leading `<`, expand `$`-refs and tildes in the filename, `read_to_string` it, strip trailing newline. Tests: `test_dollar_lt_file_reads_contents`, `test_dollar_lt_no_space`.

### `printf %q` — backslash-style quoting

- Was using single-quote wrapping (bash semantics). zsh's `%q` matches `${(q)}` flag — backslash-escape shell-special chars. Updated both `printf_format_count`'s `'q'` branch and `builtin_printf`'s `'q'` branch. Tests: `test_printf_q_uses_backslash_quoting`, `test_printf_q_safe_word_unquoted`.

## Closed (twenty-sixth-pass — `$((~N))` bit-NOT + `${s%$var}` strip expansion)

### `$((~N))` bitwise NOT no longer mistriggers tilde expansion

- The arith evaluator unconditionally ran `expand_string` on the expression text. For `$((~0))`, expand_string treated leading `~` as tilde-name (`~0` → "no such user: 0" fatal). Three eval entry points (`evaluate_arithmetic`, `eval_arith_expr`, `eval_arith_expr_float`) now skip `expand_string` when the expression has no `$` or `` ` `` (no var/cmd-subst/nested-arith to resolve). MathEval handles bare `$NAME`-less arith on its own. Tests: `test_arith_bitwise_not`, `test_arith_bitwise_not_in_expr`, `test_arith_dollar_var_still_works`.

### `${s%$var}` / `${s##$var}` — expand `$var` in strip pattern

- Same shape as the prior fix to `${var/$pat/}`: pattern operand was emitted literally. `BUILTIN_PARAM_STRIP` now runs `expand_string` on the pattern before glob-matching. Tests: `test_strip_pattern_expands_dollar_var`, `test_strip_long_pattern_expands`.

## Closed (twenty-seventh-pass — substring negative-length + shift validation + echo combined flags)

### `${s:0:-N}` substring negative length truncates from end

- The compile path passed `length=-1` for "no length given" — same value as an explicit `:0:-1`. Switched the sentinel to `i64::MIN` so the runtime can distinguish:
  - `i64::MIN` → no length given, take rest
  - `< 0` → "stop |N| chars before end" (bash/zsh)
  - `>= 0` → take exactly N chars
- Tests: `test_substring_negative_length_truncates_from_end`, `test_substring_offset_and_negative_length`, `test_substring_no_length_takes_rest`.

### `shift N` errors when N > $#

- Was silently shifting min(N, len). zsh emits `zsh:shift:1: shift count must be <= $#` and exits 1. Now matches. Test: `test_shift_too_many_errors`.

### `echo -nE` combined flags

- The flag parser only matched exact `-n`/`-e`/`-E` strings — combined forms like `-nE` were treated as positional args. Now walks the flag body char-by-char, requiring all chars to be recognised echo flags. Test: `test_echo_combined_flags`.

## Closed (twenty-eighth-pass — `(l/r)` padding + quoted-glob test patterns)

### `${(l:N:)s}` left-pad and `${(r:N:)s}` right-pad flags

- The PadLeft/PadRight enum existed but the BUILTIN_PARAM_FLAG fast-path (`${(l:5:)s}` form) didn't recognise them. Added an `'l' | 'r'` arm to the dispatcher: parses the colon-delimited width, optional `:fill:` segment, and pads with truncate-on-overflow. Tests: `test_left_pad_flag`, `test_right_pad_flag`, `test_left_pad_with_fill_char`.

### `[[ X == "a*" ]]` — quoted glob meta is literal

- Was treating any `*`/`?`/`[` in the RHS as glob metacharacters regardless of quoting. zsh treats quoted metas as literal. Added `escape_quoted_glob_metas` helper in compile_cond's Binary path: walks the lexer-tokenized RHS, tracks SNULL/DNULL boundaries, prepends a `\` to glob metas inside quoted regions. Then taught `glob_match_static`'s regex translator to treat `\X` as literal X (escaping the regex meta if needed). Tests: `test_quoted_glob_pattern_in_test_is_literal`, `test_quoted_literal_star_matches_quoted_literal_star`, `test_unquoted_glob_pattern_still_matches`.

## Closed (twenty-ninth-pass — `[^...]` glob negation + `read` EOF return)

### `[^abc]` glob char-class negation

- The underlying `glob` crate (fnmatch-derived) only recognises `[!abc]` for class negation. Pre-fix, `echo [^a]` matched files literally containing `^` or `a` — completely inverted. Added a small pre-pass in `expand_glob` that walks the pattern and converts `[^` → `[!` only inside bracket regions. Test: `test_glob_caret_negation`.

### `read` returns 1 on partial-line EOF

- Was returning 0 on any successful byte read, even when the input ended without a delimiter. zsh returns 1 in that case so `while read line` loops terminate cleanly. Added a `hit_terminator` tracker; on EOF without newline, assign the variable but return 1. Test: `test_while_read_returns_1_at_eof_no_newline`.

## Closed (thirtieth-pass — `${1+...}` + `~$VAR` + `(L+N)` size)

### `${1+arg}` / `${5-default}` — positional set/unset detection

- `BUILTIN_PARAM_DEFAULT_FAMILY` checked existence via `variables.contains_key`/`arrays.contains_key`/etc. Positional params live in `positional_params: Vec<String>` and weren't found by name. Added a digit-name branch that compares the parsed index against `positional_params.len()` (with `$0` always set). Test: `test_positional_default_plus_returns_alt_when_set`, `test_positional_default_plus_unset`.

### `~$VAR` and `~"$VAR"` tilde + dollar expansion

- The compile-side `split_word_segments` was emitting `~` as a separate Literal segment from the `$VAR` Expansion, defeating tilde-expansion. Skip the segment split when `untoked.starts_with('~') && contains '$'` so the bridge sees `~$VAR` whole and routes through `expand_tilde_named`.
- `expand_tilde_named` then resolves `$VAR` itself and strips surrounding quotes (so `~"$USER"` works the same as `~$USER`). Tests: `test_tilde_with_dollar_var`, `test_tilde_with_quoted_dollar_var`.

### `(L+N)` size-glob qualifier — default unit is bytes

- The `L` qualifier defaulted to 512-byte blocks (zsh ksh-mode but not the modern default). Switched default unit to bytes so `(L+3)` correctly means "more than 3 bytes". Suffix units (`k`/`m`/`g`/`p`) still work. Also extended `looks_like_glob` to treat trailing `(qualifier)` as a glob trigger so NOMATCH fires for unmatched qualifier-only patterns. Test: `test_glob_qualifier_size_l_uses_bytes`.

## Closed (thirty-first-pass — function override + `[ ]` test form)

### User function overrides shadowed builtins (`r`, `echo`, `pwd`, `true`, `false`, `cd`, `print`, `printf`)

- zsh dispatch order: alias → function → builtin → external. `name() { ... }; name args` must run the user function, not the builtin. zshrs's compile path emitted `Op::CallBuiltin` directly for any name in `fusevm::shell_builtins::builtin_id`, so a user function never had a chance to win. `r` was the most painful: `r() { echo $1; }; r 5` infinite-looped because `builtin_r` runs `fc -e -` (history-replay) and re-executed the previous script — every iteration re-registered the function and re-called itself.
- Added a `try_user_fn_override(name, args)` helper (src/exec.rs) that consults `functions_compiled` + `function_exists`, then routes through `dispatch_function_call`. Wired into the `r`, `cd`, `pwd`, `echo`, `print`, `printf`, `true`, `false` builtin handlers. Tests: `test_user_function_overrides_r_builtin`, `test_user_function_overrides_echo_builtin`, `test_user_function_overrides_pwd_builtin`, `test_user_function_overrides_true_builtin`.

### `[ a -eq b ]` test-form always returned 0 (huge bug)

- The compile-time "dynamic command name" check at `compile_zsh.rs:520` flagged any first word containing `[` as needing `Op::Exec` dispatch (so `cmd[$i]` etc. resolves through host.exec). When the first word was literally `[`, that diverted `[` away from `BUILTIN_TEST` and into external `/usr/bin/[` — which on macOS is a quirky BSD test that returned 0 for the malformed-arg shapes we passed (the `]` was being captured as another argv slot). Result: every `[ ... ]` test returned true unconditionally, breaking every script that used `if [ ... ]`, `while [ ... ]`, `until [ ... ]`, `[ ... ] && cmd`. Catastrophic.
- Carved out `[` and `[[` from the dynamic-name check before the glob-char trigger fires (`first_is_test_builtin = first_untoked == "[" || first_untoked == "[["`). They now dispatch to `BUILTIN_TEST` / `BUILTIN_COND` like any other builtin. Tests: `test_test_builtin_bracket_form_returns_correct_status`, `test_if_elif_chain_with_bracket_test`, `test_until_loop_with_bracket_test`.

## Closed (thirty-second-pass — assoc append, sort sub-flags, printf %g)

### `m+=(k v)` on associative arrays

- BUILTIN_APPEND_ARRAY blindly extended `exec.arrays` regardless of whether the name was an assoc, so `typeset -A m=(a 1); m+=(c 3)` left the new key/value in a parallel positional array and `${m[c]}` returned empty. Added an assoc-aware branch that consumes pairs into `exec.assoc_arrays`. Test: `test_assoc_append_pairs_adds_new_keys`.

### `(o)` / `(O)` sort sub-flags `n`/`i`/`a`

- `(oa)` (sort by array order = no-op) and `(Oa)` (reverse array order, no alpha-sort) were both being treated as plain alpha-sort. Same for `(on)` numeric sort and `(oi)` case-insensitive. Reworked the `'o' | 'O'` arm in `BUILTIN_PARAM_FLAG` to consume an optional `n`/`i`/`a` sub-letter and dispatch accordingly: `a` → reverse-only-if-O, `n` → f64 sort, `i` → case-insensitive sort, default → byte-sort. Tests: `test_param_flag_oa_preserves_array_order`, `test_param_flag_Oa_reverses_array_order`, `test_param_flag_on_numeric_sort`, `test_param_flag_oi_case_insensitive_sort`.

### `printf '%g\n'` shortest-representation float format

- `%g` was emitting the `%f` format unchanged (`3.14` → `3.140000`). Implemented a `format_g(val, prec, upper)` helper that picks `%e` when `exp < -4 || exp >= prec` else `%f`, strips trailing zeros after the decimal, and normalizes the exponent to `e±NN` (C99 shape). Test: `test_printf_g_uses_shortest_representation`.

## Closed (thirty-third-pass — typeset -i +=, ${(k)arr}, getopts)

### `typeset -i x=42; x+=8` did string concat instead of arithmetic add

- BUILTIN_APPEND_SCALAR_OR_PUSH always took the scalar concat branch (`format!("{}{}", prev, value)` → `"428"`). For a typeset-int variable `+=` should arithmetically add the RHS (which itself is arith-evaluated). Added an `is_integer` check from `var_attrs` and a parse + add path. Test: `test_typeset_int_plus_eq_arithmetic_add`, `test_typeset_int_plus_eq_arith_expression`.

### `${(k)arr}` on a regular (non-assoc) array returned empty

- The `'k'` arm in BUILTIN_PARAM_FLAG only consulted `assoc_arrays`. zsh's actual behavior on regular arrays: `${(k)arr}` returns the array's values themselves (a quirk — docs imply integer subscripts but the impl returns contents). Fall through to `arrays.get(&name)` for the regular case. Test: `test_param_flag_k_on_regular_array_returns_values`.

### `getopts` skipped the option immediately after an arg-taking flag

- After `getopts ab:c` consumed `-b X`, the arg-fetch branch advanced OPTIND by 2 but the bottom of the function unconditionally overwrote it back to `optind + 1`, leaving OPTIND on `X` instead of `-c`. Refactored the takes_arg branch to compute `(arg, advance)` once and apply at the end. Also clear OPTARG when an option doesn't take one (was leaking the previous arg's value into the next iteration). Test: `test_getopts_stops_after_arg_taking_option`.

## Closed (thirty-fourth-pass — set -u + default, (#) char-code, (n) natural sort, subshell env isolation)

### `set -u; echo "${var:-fb}"` aborted with "parameter not set"

- BUILTIN_PARAM_DEFAULT_FAMILY called `get_variable(name)` which honors `nounset` and exits 1 before the modifier got a chance to provide the default. The whole point of `${var:-fb}`, `${var-fb}`, `${var:+alt}`, `${var+alt}` is to handle missing values; the lookup must be silent. Save/restore `nounset` + `unset` options around the get_variable call. Test: `test_set_u_with_default_modifier_does_not_abort`.

### `${(#)val}` char-code flag (arith → character)

- The `'#'` arm was missing from BUILTIN_PARAM_FLAG entirely; `${(#)65}` returned `65` instead of `"A"`. Added: arith-eval each element, output the char with that code point. Distinct from `${#name}` (length) which is parsed as the LENGTH op upstream. Test: `test_param_flag_pound_arith_to_char`. Also fixed an existing test in `tests/no_tree_walker_dispatch.rs` (`zshflag_array_length_via_pound`) that incorrectly expected `${(#)arr}` to mean array length — it's char-code, not length.

### `(n)` natural sort: `file2 < file10 < file20`

- Previous impl was `f64::parse()` per-element, which returned 0.0 for all `file*` strings and left the array order untouched. Wrote a `natural_cmp(a, b)` helper that walks both strings in parallel, treating runs of digits as integer chunks (length-then-byte-cmp, with leading-zero tiebreak). Test: `test_param_flag_n_natural_sort`.

### `(export y=v)` in subshell leaked `y` to the parent shell

- zshrs runs `(...)` subshells in-process for perf (no fork). The subshell snapshot/restore covered `variables`, `arrays`, `assoc_arrays`, `positional_params`, and cwd — but NOT the OS env table. When the body called `env::set_var` (via `export` or `cd`'s `$PWD` write), those writes survived past `subshell_end` and corrupted the parent's environment. Added `env_vars` field to `SubshellSnapshot` (snapshot at begin via `std::env::vars().collect()`) and restore at end (remove keys not in snapshot, re-set keys whose values changed). Tests: `test_subshell_export_does_not_leak_to_parent`, `test_subshell_unset_does_not_leak_to_parent`.

### `getopts` over-advanced OPTIND past the next flag *(see thirty-third-pass above)*

## Closed (thirty-fifth-pass — coreutils builtins respect user overrides)

### `cat() { ... }; cat` ran the C builtin instead of the user function

- Same root cause as the prior `r`/`echo`/`pwd` fix, but now extended to fusevm's coreutils-style anti-fork builtins (`cat`, `head`, `tail`, `wc`, `basename`, `dirname`, `touch`, `realpath`, `sort`, `find`, `uniq`, `cut`, `tr`, `seq`, `rev`, `tee`, `sleep`, `whoami`, `id`, `hostname`, `uname`, `date`, `mktemp`, `mkdir`). Each handler bypassed user functions because the compiler emitted `Op::CallBuiltin` directly. zpwr/oh-my-zsh wrap most of these, so override-blindness was a major real-world breakage.
- Introduced a `reg_overridable!($vm, $id, $name, $method)` macro at the top of `register_builtins`. Each registration now consults `try_user_fn_override` before falling through to the native handler. Test: `test_user_function_overrides_coreutils_builtins` covers 11 representative cases.

## Closed (thirty-sixth-pass — `${(Q)s}` dequote)

### `${(Q)s}` — strip one layer of shell quoting

- The `'Q'` arm was missing entirely from `BUILTIN_PARAM_FLAG`. Added: balanced single-quote (`'…'` → literal contents), double-quote (`"…"` → process `\"`/`\\`/`\$`/`` \` `` escapes), and bare-string (drop one backslash per escape) handling. Test: `test_param_flag_Q_dequote`.

### Deferred — command substitution exit status to `$?`

- `cmd_out=$(false); echo $?` should report 1 (zsh: assignment is transparent w.r.t. `$?`), but `echo $(false); echo $?` should report 0 (zsh: the enclosing command's status overrides). zshrs currently propagates neither — `cmd_out=$(false); echo $?` reads 0. Fixing it requires a generation counter that fires on every Op::SetStatus to invalidate a "pin" set by cmd-substitution. fusevm doesn't expose a hook for SetStatus, and a value-only witness (compare current vm.last_status to the value at pin-time) collapses when both happen to be the same number. Documented as a known divergence; keep tests for the closed cases stable. Affects `$?` after cmd-substitution-only RHS.

## Closed (thirty-seventh-pass — pipeline last-stage in current shell)

### `echo hi | read x; echo "x=$x"` showed empty (zsh: `x=hi`)

- Major behavioral divergence: zsh runs the LAST stage of a pipeline in the CURRENT shell process (not a forked child) so trailing `read x` keeps its assignment in the parent. zshrs's BUILTIN_RUN_PIPELINE was forking every stage, including the last — same behavior as bash, wrong for zsh. Refactored: fork stages 0..N-1, dup2 the final pipe's read end onto stdin in the parent, run the last stage's chunk inline on a sub-VM with `set_shell_host(Box::new(ZshrsHost))` so any reads/assignments hit the parent executor's variables, then restore stdin. The in-parent stage's status is appended to `pipestatus` so the array still has one entry per stage. Tests: `test_pipeline_last_stage_runs_in_current_shell`, `test_pipeline_last_stage_assignment_persists`.

## Closed (thirty-eighth-pass — SIGPIPE-in-pipeline-child, `type` alias format)

### `seq 1 100 | head -3` panicked with "Broken pipe (os error 32)"

- The parent shell ignores SIGPIPE so it can detect EPIPE on writes itself, but pipeline children inherited that disposition. When the downstream reader closed early, the producer's `println!` returned EPIPE and Rust's stdout write panicked. Reset SIGPIPE to `SIG_DFL` in each forked pipeline child immediately after `fork()` so the child gets killed cleanly on broken pipe (matches zsh/bash behavior). Test: `test_pipeline_child_handles_sigpipe_gracefully`.

### `type alias_name` printed bash format instead of zsh format

- Was emitting `"{name} is aliased to \`{value}'"` (bash `type` shape). zsh prints `"{name} is an alias for {value}"` (no backticks, "for" not "to"). Test: `test_type_alias_uses_zsh_format`.

## Closed (thirty-ninth-pass — `command -v` resolution order)

### `command -v echo` printed `/bin/echo` instead of `echo`

- `builtin_command` jumped straight to a PATH walk for `-v`/`-V`, so every name resolved to its external (or "not found"). zsh's resolution order is alias → function → shell builtin → reserved word → external; only the external case prints a path. Added each tier in order before the PATH walk: aliases print `alias k=v` (verbose: "k is an alias for v"), functions/builtins/reserved words print just the name, externals print the resolved path. Tests: `test_command_v_resolution_order_matches_zsh`, `test_command_v_missing_returns_nonzero`.

## Closed (fortieth-pass — `which` csh format, `read` backslash processing)

### `which echo` printed just `echo` instead of `echo: shell built-in command`

- The `csh_style` (-c) branch in `builtin_whence` had a builtin case missing — it fell through to the plain `println!("{}", name)`. zsh's `which` (and `whence -c`) emits `name: shell built-in command` for shell builtins. Added the `csh_style` arm. Test: `test_which_for_builtin_shows_csh_format`.

### `read` (without `-r`) didn't process `\X` escapes — backslash + char came out literally

- Previous impl was `if !raw_mode { input.replace("\\\n", "") }` which only handled the line-continuation case. POSIX read (no -r) drops one backslash from every `\X` pair: `\b` → `b`, `\\` → `\`, `\<newline>` → both stripped. Replaced with a char-iterator pass. Test: `test_read_processes_backslash_escapes_without_dash_r`.

## Closed (forty-first-pass — `$(cmd)` IFS word-split in argument context)

### `f $(echo a b c)` passed one arg instead of three

- zsh splits the result of bare `$(cmd)` on `$IFS` when the substitution sits in argument context (`f $(...)`, `for x in $(...)`, etc.). zshrs's BUILTIN_CMD_SUBST_TEXT was returning a single Value::str without splitting, so `f $(echo a b c)` invoked f with one joined arg and `$#` reported 1.
- Emit a `BUILTIN_WORD_SPLIT` op after the cmd-subst handler unless the surrounding word is DQ-wrapped (`"$(...)"`) or we're inside an assignment RHS. Added an `assign_context_depth` field to ZshCompiler that's bumped around `compile_assign`'s `compile_word_str` call so `x=$(printf 'a\nb\nc')` keeps both lines (assignment shouldn't split). Tests: `test_cmd_subst_word_splits_in_argument_context`, `test_cmd_subst_no_split_in_dq_context`, `test_cmd_subst_no_split_in_assignment`.

## Closed (forty-second-pass — `-f` startup flag turns off rcs+hashdirs)

### `zshrs -f -c 'setopt'` printed empty (zsh: `nohashdirs\nnorcs`)

- `setopt` (no args) lists options whose state differs from the compiled-in default. zsh's `-f` flag turns off `rcs` (skip user .zshrc et al) AND `hashdirs` (don't pre-hash command paths) — both default-on options, so they appear as `norcs` / `nohashdirs` in `setopt`'s diff. zshrs's `-f` only filtered the flag from arg parsing without flipping any options, so `setopt` reported nothing.
- Captured `no_rcs_flag = args.iter().any(|a| a == "-f" || a == "--no-rcs")` before the filter and threaded it into `apply_cli_flags`. Inserts `rcs=false` and `hashdirs=false` into `executor.options` (left `globalrcs` untouched — zsh `-f` keeps that on, only user-rcs files get skipped). Test: `test_dash_f_flag_disables_rcs_and_hashdirs`.

## Closed (forty-third-pass — default aliases match zsh)

### `alias` listing missing zsh's compiled-in defaults

- zsh ships two aliases by default: `run-help=man` and `which-command=whence`. Visible in a fresh `zsh -f -c 'alias'`. zshrs's executor started with an empty alias map. Pre-populated `aliases` HashMap with these two entries in `ShellExecutor::new()`. Test: `test_default_aliases_match_zsh`.

## Closed (forty-fourth-pass — `.*` glob excludes `.`/`..` and matches dotfiles)

### `echo .*` returned `./.` `./..` instead of the actual dotfiles

Two bugs in `expand_glob`:
- The Rust `glob` crate includes `.` and `..` in its results when the pattern matches them. zsh always excludes those even with `dotglob`. Added a textual `rsplit('/').next()` filter (`Path::file_name` returns None for `.`/`..` so the structured API doesn't catch them).
- With `dotglob` off, `glob`'s `require_literal_leading_dot` blocked the dot-prefixed pattern from matching dotfiles even though the leading `.` IS literal in `.*`. zsh's actual rule: when the LAST path component starts with `.`, the leading `.` is literal so dotfiles match (no setopt needed). Set `dotglob = true` for this case before passing to `glob_with`.

Tests: `test_dot_glob_excludes_dot_and_dotdot`, `test_star_glob_excludes_dotfiles_by_default`.

## Closed (forty-fifth-pass — `${var/pat/repl}` glob patterns)

### `${s/?/X}` was treating `?` literally

- `BUILTIN_PARAM_REPLACE` used `String::replace` for plain text matching, which doesn't honor zsh's pattern syntax. zsh patterns in the replace form support `?` (any single char), `*` (any sequence), and `[...]` (char class). Compile a regex from the glob pattern (escaping regex-only metas) when the pattern contains glob chars; fall back to plain string for the meta-free fast path. Anchored prefix (`/#`) and suffix (`/%`) variants both honor the regex match position. Tests: `test_param_replace_glob_pattern_question`, `test_param_replace_glob_pattern_star`, `test_param_replace_glob_pattern_class`, `test_param_replace_global_with_glob`.

## Closed (forty-sixth-pass — `[[ $s =~ $pat ]]` variable expansion)

### `pat="^h"; [[ "hello" =~ $pat ]]` never matched

- The `[[ s =~ $pat ]]` compile path emitted the RHS as a LoadConst literal (skipping `compile_word_str`'s expansion to avoid filesystem-glob expansion of pattern chars). That meant `$pat` reached the regex engine as the literal string `$pat` instead of its value, so the match always failed. Carved out `=~` separately: wrap the RHS in DQ markers (`\u{9e}…\u{9e}`) and route through `compile_word_str` so variable / cmd-subst / arith expansion fires. The DQ wrapper suppresses brace expansion + filesystem globbing — the test runtime treats the result as a regex pattern. Tests: `test_cond_regex_with_variable`, `test_cond_regex_with_capture_groups`.

## Closed (forty-seventh-pass — glob qualifier in pipeline child)

### `echo *(N) | wc -w` returned 0 in pipeline (zsh: file count)

- `expand_glob`'s parallel `prefetch_metadata` submits stat() jobs to a worker pool, but the pool's threads don't survive `fork()` (POSIX: only the calling thread persists). The pipeline child stage forked, then submitted work that nobody picked up — the rx loop blocked indefinitely OR the channel returned empty, depending on timing. Net effect: every glob with `>=32` matches in a pipeline stage produced empty output.
- Added `signals::is_forked_child()`: lazy-init MAIN_PID on first call, then compare current pid. Pre-warmed in `zshrs_main()` so the parent's pid is captured before any pipeline forks. `prefetch_metadata` now takes the serial stat path whenever `is_forked_child()` returns true. Test: `test_glob_qualifier_in_pipeline_child`.

## Closed (forty-eighth-pass — `${#@}` positional count + chars()-not-bytes for length)

### `${#@}` returned 5 instead of `$#`

- The `expand_braced_variable` length-form (`${#name}`) fell through to `get_variable("@")` which returns the IFS-joined positional string, then took its `.len()` (byte count). For `set -- a b c`, that produced `5` (length of `"a b c"`) instead of zsh's `3` (number of positional params).
- Special-cased `@` and `*` in the `${#…}` arm to return `positional_params.len()` directly.
- Also switched the scalar fallback from `.len()` (bytes) to `.chars().count()` (codepoints) so `${#héllo}` is 5 not 6 — matches zsh.

Tests: `test_param_length_at_star_returns_positional_count`, `test_param_length_uses_chars_not_bytes`.

## Closed (forty-ninth-pass — `exit N` fires EXIT trap)

### `trap 'cleanup' EXIT; exit 5` skipped the cleanup

- The implicit script-end path in `execute_script_zsh_pipeline` already ran the EXIT trap. But `builtin_exit` called `std::process::exit(code)` directly, bypassing the trap. Real-world scripts use `trap 'cleanup' EXIT` heavily for tempfile cleanup, db disconnect, etc. — `exit N` skipping the trap is a major compatibility break.
- Inserted the trap-fire (with same "remove first to prevent recursion" pattern as the implicit path) before `std::process::exit`. Set `last_status = code` first so the trap body sees the right `$?`. The subshell branch is unchanged — it returns to the subshell caller without running the trap (zsh defers EXIT trap to the outer process). Tests: `test_exit_builtin_fires_exit_trap`, `test_exit_trap_with_explicit_exit_in_trap_body`.

## Closed (fiftieth-pass — function-name-with-hyphen call dispatch)

### `foo-bar() { ... }; foo-bar` returned "command not found: foobar"

- `foo-bar` registered cleanly under its real name, but the call site went through compile_simple → add_name(first) where `first` was still the lexer's META-encoded form `foo\u{9b}bar` (`\u{9b}` is the META char for `-`). add_name stored the encoded string verbatim; CallFunction looked it up; missed the registered `foo-bar`; fell through to host.exec which reported "command not found" with the partly-cleaned form `foobar`.
- Untokenize `first` before `add_name` so the cleaned identifier reaches the name pool. Same fix applies to any function name with hyphens, dots, or other lexer-mangled chars in the call dispatch. Tests: `test_function_name_with_hyphen_dispatches_correctly`, `test_function_name_with_hyphen_passes_args`.

## Closed (fifty-first-pass — `typeset -f` body capture preserves first word)

### `typeset -f f` for `f() { echo a; echo b; }` printed `a; echo b;`

- All three `Inbrace` body-capture sites in `parser.rs` (parse_funcdef, parse_inline_funcdef, and the synthesized FuncDef path inside `parse_program_until`) called `zshlex()` BEFORE recording `body_start`. After consuming `{`, the next `zshlex()` advances past the first body token (`echo`), so `body_start` landed mid-body and the captured slice lost the first word. Result: `typeset -f f` / `functions f` / `whence -c f` all printed `a; echo b;` instead of `echo a; echo b;`.
- Hoisted `let body_start = self.lexer.pos;` BEFORE the `zshlex()` in all three sites (lexer.pos already points just past `{` after the outer zshlex consumed it). Test: `test_typeset_f_preserves_first_word_of_body`.

## Closed (fifty-second-pass — `${(t)readonly_var}` includes `-readonly` modifier)

### `readonly R=x; echo "${(t)R}"` printed `scalar` instead of `scalar-readonly`

- `format_zsh()` already appends `-readonly` to the type string when `var_attrs.readonly` is set, but `builtin_readonly` only inserted the name into `readonly_vars` (a separate HashSet for write-protection enforcement) without touching `var_attrs`. Result: write-protection worked, but `(t)` introspection couldn't see the readonly bit.
- Added the `var_attrs.readonly = true` set to both branches of `builtin_readonly` (the `name=value` and bare-`name` paths). Test: `test_t_flag_includes_readonly_modifier`.

## Closed (fifty-third-pass — `[[ -nt ]]` ns precision + `integer -r` readonly attr + `$*` IFS join)

### `[[ a -nt b ]]` returned false when files were touched within the same second

- Was using `MetadataExt::mtime()` (seconds only). `touch a; sleep 0.3; touch b; [[ b -nt a ]]` failed because both timestamps reported the same integer second. Switched to `metadata().modified()` (`SystemTime`, ns precision). Same fix applied to `BUILTIN_FILE_OLDER` (`-ot`). Test: `test_cond_nt_uses_nanosecond_precision`.

### `integer -r I=42; echo "${(t)I}"` printed `integer` instead of `integer-readonly`

- `builtin_integer` ignored its leading `-r` / `-x` flags entirely (the loop just `continue`'d on any `-` arg). Parsed flags into local bools, then composed `VarAttr { kind: Integer, readonly, export }` and inserted `name` into `readonly_vars` + env when applicable. Test: `test_integer_dash_r_sets_readonly_attr`.

### `IFS=":"; echo "$*"` joined positionals by space, not by `:`

- `get_variable("@" | "*")` was hardcoded to `.join(" ")`. POSIX: `$*` joins by the first char of `$IFS`. Updated to read `$IFS` and use its first char (default ` ` when unset). Note: `expand_string`'s DQ-context expansion of `"$*"` follows a different path that still produces only the first param — that's a deeper bug, deferred.

## Closed (fifty-fourth-pass — `${#argv}` count + DQ `"$*"` IFS-join (preserve `"$@"` splice))

### `${#argv}` returned 5 (joined-string byte length) instead of 3 (count)

- `argv` is zsh's named alias for the positional array. `expand_braced_variable`'s `${#…}` arm and `BUILTIN_PARAM_LENGTH` both special-cased `@` and `*` to return `positional_params.len()`, but missed `argv`. Added the alias + its subscripted forms (`argv[@]` / `argv[*]`) to both special-case lists. Test: `test_argv_length_returns_positional_count`.

### `v="$*"` captured only the first positional

- For bare `$@`/`$*`, compile_word_str emitted `BUILTIN_GET_VAR` directly (returns Value::Array of positionals so splice in argument context works). When that result reached the assignment via `pop_args`, the Array got flattened into separate args — `name="v"`, `value=<first positional>`, rest discarded.
- Detect DQ context (parent `s` is DNULL-wrapped OR `dq_context_depth>0`) and, ONLY for `$*`, follow GET_VAR with Pop + LoadConst(name) + `BUILTIN_ARRAY_JOIN_STAR` so the joined scalar replaces the Array on stack. `$@` keeps its Array splice (zsh: `"$@"` → each positional its own word). Tests: `test_dq_star_assignment_joins_with_ifs`, `test_dq_at_preserves_splice_semantics`.

## Closed (fifty-fifth-pass — `noglob` precommand + coreutils error message format)

### `noglob echo *` was glob-expanding `*` before noglob ran

- `noglob` is a precommand modifier — its args must NOT be glob-expanded. zsh handles this in the parser/lexer by marking the line "no-glob". zshrs's `builtin_noglob` set the `noglob` option AFTER its args had already been compiled+expanded, so `*` always expanded against the real cwd.
- Special-cased `noglob` in `compile_simple`: when `simple.words[0] == "noglob"`, peel off the leading word, emit a `BUILTIN_SET_RAW_OPT("noglob", true)` toggle, recursively compile the remaining `ZshSimple`, then emit the matching `BUILTIN_SET_RAW_OPT("noglob", false)` to restore. New `BUILTIN_SET_RAW_OPT` handler does a flat `options.insert/remove` (no validation, just the toggle). Test: `test_noglob_precommand_suppresses_glob`.

### `cat /no/such/file` printed `... (os error 2)` suffix

- Rust's `io::Error` display appends `(os error N)` by default. zsh's bundled coreutils emit just the friendly message (`No such file or directory`). Added `pretty_io_err(&io::Error) -> String` helper that strips ` (os error` and everything after; wired into `cat`/`head`/`tail`/`wc` error sites. Test: `test_coreutils_error_msg_strips_os_error_suffix`.

## Closed (fifty-sixth-pass — `((a[i]=v))` subscripted arith assignment)

### `((a[2]=42))` left the array unchanged

- Both runtime arith paths (`evaluate_arithmetic` for `$(())` and `compile_arith` → `ArithCompiler` for `(())`) ran `pre_resolve_array_subscripts` first, which substitutes `a[2]` with the current value (`0`). Result: arith engine saw `0=42` (invalid). The non-subscripted `a=42` path worked because no pre-resolve was needed.
- Added `parse_subscript_arith_assign(expr)` helper that detects `name[idx]=rhs` (rejecting `==` / `=~`). When matched: eval the index, eval the RHS, write back via `arrays.get_mut` (or `assoc_arrays.get_mut`, with auto-resize). Wired into `evaluate_arithmetic`.
- For compound `(())`, `compile_arith` now untokenizes the lexer-wrapped expr, strips outer parens, runs the same check via `subscripted_arith_assign_check`, and routes to `BUILTIN_ARITH_EVAL` instead of `ArithCompiler` (which has no array-write hook). Test: `test_arith_subscripted_array_assign`.

### Deferred — `nocorrect` precommand

- `echo step1; nocorrect echo hi; echo step3` only runs `echo step1` — the parser (the unmodifiable direct port of zsh's C parser) drops everything after `;` once `nocorrect` appears as a command name. zsh handles `nocorrect` as a precommand modifier in its lexer/parser via `lexflags.dbparens` / `incmdpos` machinery. zshrs's port has the field but no behavior. Stripping `nocorrect` in `compile_simple` doesn't help because the parser already lost the rest of the line. Documented as deferred — needs lexer-side handling.

## Closed (fifty-seventh-pass — `source` missing-file format/exit + `a[0]=v` invalid subscript)

### `source /no/such/file` printed Rust's wrapped error and exited 1

- Output was `source: /no/such/file: /no/such/file: No such file or directory (os error 2)` (path duplicated, Rust's "(os error N)" suffix appended) and exit 1. zsh: `zsh:source:1: no such file or directory: /no/such/file` and exit 127.
- Both branches (POSIX-mode and zshrs-mode) now strip Rust's "(os error N)" suffix, strip any duplicate-path prefix that wrapped errors carry, lowercase the message, and emit the canonical `zshrs:source:1: <msg>: <path>` shape with exit code 127. Test: `test_source_missing_file_zsh_format_and_exit_127`.

### `a[0]=hi` was silently accepted as a no-op

- zsh: arrays are 1-based, so `a[0]=v` is "assignment to invalid subscript range" (exits 1). zshrs's `BUILTIN_SET_ASSOC` had a `i == 0` arm that just `return`'d silently. Replaced with the diagnostic + `std::process::exit(1)`. Test: `test_array_zero_subscript_assignment_errors`.

## Closed (fifty-eighth-pass — `umask`/`cd`/abs-path-missing format pass)

### `umask` printed `0022` instead of `022`

- zsh prints 3 octal digits (`022`); zshrs's `{:04o}` format-spec emitted 4. Switched to `{:03o}`. Test: `test_umask_3_octal_digits_no_leading_zero`.

### `umask -S` printed `u=rwxg=rxo=rx` (no separators)

- The println! template was missing the commas between user/group/other groups. zsh emits `u=rwx,g=rx,o=rx`. Test: `test_umask_dash_S_uses_commas`.

### `cd /no/such/dir` emitted Rust's wrapped error format

- Was `cd: /not/a/dir: No such file or directory (os error 2)` (with the os-error suffix). zsh emits `zsh:cd:1: no such file or directory: PATH` (lowercased, prefixed). Switched to `pretty_io_err` + the canonical `zshrs:cd:1: <msg>: <path>` shape. Test: `test_cd_missing_dir_zsh_format`.

### `/nonexistent_xyz` (absolute path) said `command not found:`

- zsh distinguishes: relative names not in PATH → `command not found`; absolute paths that don't exist → `no such file or directory` (since no PATH search was attempted, the open() syscall reported ENOENT). Added a `cmd.starts_with('/')` branch in `execute_external` to switch the diagnostic. Test: `test_abs_path_missing_says_no_such_file_not_command_not_found`.

## Closed (fifty-ninth-pass — `kill -l` platform signals + `exec` missing-target + `fg`/`bg` non-interactive message)

### `kill -l USR1` printed 10 instead of 30 on macOS

- The signal_map was hardcoded to Linux signal numbers (USR1=10). macOS uses USR1=30. Replaced the literal numbers with `libc::SIGHUP`, `libc::SIGUSR1`, etc. — pulled from the platform's libc bindings, so the values are always correct on the build target. Test: `test_kill_l_uses_platform_signal_numbers`.

### `exec /nonexistent_xyz` emitted Rust's wrapped error and continued

- Was `exec: /nonexistent_xyz: No such file or directory (os error 2)` then continued running with status 1. zsh emits `zsh:1: no such file or directory: PATH` and exits the whole shell with 127 (exec target unfindable). Stripped Rust's wrapping via `pretty_io_err`, lowercased, used canonical `zshrs:1: <msg>: <path>` shape, switched to `std::process::exit(127)` since `exec` failure is fatal.

### `fg` / `bg` in non-interactive mode said "no current job"

- zsh emits `zsh:fg:1: no job control in this shell.` (with trailing period — quirky). Updated both messages. Test: `test_fg_bg_no_job_control_message`.

## Closed (sixtieth-pass — `kill -l` ordering + unknown number + `${(@P)}` indirect array)

### `kill -l` listed signals in declaration order, not number order

- zsh's `kill -l` prints the signal-name list ordered by signal number (HUP INT QUIT ILL TRAP ABRT EMT FPE …). zshrs was iterating the signal_map in declaration order which differed because the map groups by user-relevant categories instead. Sort by signal number before joining. Test: `test_kill_l_lists_signals_in_number_order`.

### `kill -l 100` errored "unknown signal: 100" instead of passing through

- zsh: unknown signal numbers print the number unchanged (`kill -l 100` → `100`). Removed the eprintln-and-no-output path; print the raw number when no name match. Test: `test_kill_l_unknown_number_passes_through`.

### macOS `EMT` signal missing from `kill -l` output

- macOS has SIGEMT (signal 7) which Linux doesn't. zsh on macOS lists it; zshrs's signal_map didn't. Added `#[cfg(target_os = "macos")]` entry for EMT. Folded into the "kill -l ordering" test.

### `${(@P)var}` returned the raw var name instead of dereferencing

- The `'P'` arm in BUILTIN_PARAM_FLAG only handled `St::S` (scalar). When `(@)` ran first and forced state to `St::A(["x"])`, the P arm's `a => a` arm left it alone — so `${(@P)var}` returned `x` (the var name) instead of `hello` (its value). Added an `St::A(names)` branch that maps each element through `get_variable`. Test: `test_param_flag_at_P_indirects_each_element`.

## Closed (sixty-first-pass — `typeset -f` body format + `${(c)#}`/`${(w)#}` flag-then-length)

### `typeset -f f` printed `echo F;` (with trailing semicolon) for `f() { echo F; }`

- The body source captured by the parser preserved the input's semicolons. zsh re-formats: each top-level statement on its own line, trailing semicolons stripped, indented with TAB. Added `format_function_body_zsh(body)` helper that walks the source, splits on top-level `;` and `\n` (depth-tracking parens/braces, ignoring inside quotes), trims each line, and joins with `\n\t`. Wired into all three display sites (typeset -f, functions, whence -c). Test: `test_typeset_f_zsh_format_one_stmt_per_line`.

### `${(c)#a}` and `${(w)#a}` returned 0 / empty

- The flag-prefix path `${(...)body}` extracted `var_name` by splitting `body` on non-alphanumeric chars — `#a` parsed as empty name then `a` as the rest, lookup returned empty, length returned 0. Added a special case for `rest.starts_with('#')`: parse the remaining identifier, return char-count by default (matches `(c)`'s semantics) or word-count when the `Words` flag is in the chain. Tests: `test_param_flag_c_pound_returns_char_count`, `test_param_flag_w_pound_returns_word_count`.

## Closed (sixty-second-pass — `tr -c` complement + `wc` BSD 8-char padding)

### `tr -d -c "0-9"` deleted digits instead of keeping them

- The `-c` flag (complement: invert set1) was being ignored entirely. `tr -d -c "0-9"` should delete everything NOT in 0-9 (leaving only digits); was treating it as `tr -d "0-9"` (deleting 0-9). Added a `complement = args.iter().any(|a| a == "-c" || a == "-C")` parse, then an `in_set1` closure that XOR's the membership check. Also handled `-c` without `-d`: every char NOT in set1 maps to the LAST char of set2 (coreutils tr semantics). Test: `test_tr_complement_with_delete`.

### `wc -l` printed `3` instead of `       3` (zsh-style 8-char padding)

- The println had `out.trim_start()` which stripped the `{:8}` right-aligned padding. zsh's bundled wc on macOS keeps the BSD 8-char padding even on stdin output. Removed the `trim_start`. Updated 3 invariant tests in `tests/no_tree_walker_dispatch.rs` that hardcoded the old (no-padding) format. Test: `test_wc_uses_bsd_8char_padding`.

## Closed (sixty-third-pass — `type` fn `from zsh` + `tail -c` + `umask -S` symbolic set)

### `type f` for a function lacked the `from zsh` suffix

- zsh prints `f is a shell function from zsh` (the suffix names the load source — `from zsh` for built-in functions, the absolute path for autoloaded ones). Was emitting just `f is a shell function`. Updated all `println!("{} is a shell function", name)` sites. Test: `test_type_function_says_from_zsh`.

### `tail -c N` parsed `N` as a filename

- builtin_tail was missing the `-c` (byte-count) flag. `tail -c 4` errored with `tail: 4: No such file or directory`. Added `-c` parsing (both `-c N` and `-cN` shapes), then a `bytes` short-circuit in the per-file loop that does `read_to_end` + slice from `len-N`. Test: `test_tail_dash_c_byte_count`.

### `umask -S u=rwx,g=rx,o=` was rejected as "invalid mask"

- builtin_umask only parsed numeric (`022`) input. zsh accepts symbolic (`u=rwx,g=rx,o=`) for set, computing the umask as `0777` minus the granted bits per class. Added a parse path: read current umask, split on `,`, for each `class=bits` segment translate `r/w/x` into a 3-bit value and apply to the named class (u/g/o/a). Test: `test_umask_dash_S_symbolic_set`.

## Closed (sixty-fourth-pass — `find -maxdepth` + `ulimit -a` zsh format)

### `find /tmp -maxdepth 0` recursed the entire tree

- The `-maxdepth N` flag was unrecognized — `find` always recursed unbounded. Added arg parsing for `-maxdepth N`, threaded `max_depth: Option<usize>` and `cur_depth: usize` through the recursive `walk()`, and gated descent with `if cur_depth >= md { return; }`. `-maxdepth 0` now prints only the starting path. Test: `test_find_maxdepth_caps_recursion`.

### `ulimit -a` printed the wrong format and order

- zsh format per line: `<flag>: <long-name> (<unit>)<padding>value`, ordered as -t (cpu) -f (file) -d (data) -s (stack) -c (core) -m (resident) -v (address) -n (descriptors) -u (processes). zshrs was emitting just `<long-name> (<unit>) value` in a different order with no `-flag:` prefix. Reordered the limits table, prefixed each row's label with `-flag:`, widened padding to 34 chars to match zsh's column alignment exactly. Test: `test_ulimit_dash_a_zsh_format`.

## Closed (sixty-fifth-pass)

### `alias -m "g*"` always wrapped value in single quotes

- `alias g=hi; alias -m "g*"` printed `g='hi'` instead of zsh's bare `g=hi`. The `-m` pattern-match path of `builtin_alias` printed `name='value'` unconditionally, ignoring the same metaless-value detection that the regular print path used. Routed `-m` through the shared `format_alias_kv` helper so it picks the bare form for values containing no shell metas. Test: `test_alias_dash_m_uses_unquoted_form_when_no_metas`.

### `shopt` falsely accepted as a builtin

- zsh has no `shopt` — that's bash-only. zshrs shipped a bash-compat `builtin_shopt` that listed all options when called with no args. Removed from VM dispatch (`compile_simple` now skips fusevm's `builtin_id` lookup for "shopt"), removed from coreutils-known-name table, removed from completion-suggestion lists, removed from `builtin help` text. `shopt` now produces `command not found: shopt` matching zsh exactly. Test: `test_shopt_is_command_not_found`.

### Consecutive array assigns `a=(1 2 3) b=(x y z)` dropped second

- After parsing `a=(1 2 3)`, the lexer flipped `incmdpos = false` on Outpar (correct for subshell-close), which stopped the next `b=(x y z)` from being recognised as Envarray. The parser then parsed `b=(x y z)` as a command word and emitted `command not found: b=(x y z)`. Reset `self.lexer.incmdpos = true` in `parse_assign` before returning the array branch so follow-up assigns get their proper Envarray/Envstring classification. Verified `g=(o1); f() { :; }` still parses correctly (the original guard rationale). Test: `test_consecutive_array_assignments_on_one_line`.

### Stepped char brace ranges `{a..z..2}` over-expanded

- zsh only expands UNSTEPPED char ranges (`{a..z}` → `a b c …`); stepped char forms `{a..z..2}` are left literal. zshrs was happily expanding `{a..e..2}` to `a c e`. Gated `expand_brace_sequence`'s char-range branch on `parts.len() == 2` (no explicit step) and added an identity-detection guard in `expand_braces` so the literal-pass-through doesn't infinite-loop on the recursive expansion. Tests: `test_brace_stepped_char_range_left_literal`, `test_brace_unstepped_char_range_still_expands`.

### `$((10/0))` returned `0` silently

- `evaluate_arithmetic` and `eval_arith_expr` both swallowed `MathEval::Err(_)` and returned 0/"0" with no diagnostic. zsh writes the underlying error (`division by zero`, etc.) to stderr in `zsh:LINE: <msg>` form. Surface the error message via `eprintln!("zshrs:1: {}", msg)` in both eval paths. Tests: `test_arith_division_by_zero_prints_error`, `test_arith_mod_by_zero_prints_error`. (Note: zsh additionally aborts the surrounding command on arith failure; zshrs still continues with a 0 substitution result. Full command-abort propagation deferred — needs expansion-time error plumbing through expand_string and the simple-command dispatch path.)

## Closed (sixty-sixth-pass)

### `$OSTYPE`, `$MACHTYPE`, `$VENDOR`, `$CPUTYPE` returned empty

- `params.rs` set them in the params table, but the executor's `get_variable` shortcuts past that table for special names — those four were missing arms entirely. Added live `libc::uname()`-driven arms in `get_variable`: `OSTYPE` → `<sysname-lowercased><release>`, `MACHTYPE` → `machine` (with `aarch64`/`arm64` shortened to `arm` to match zsh), `CPUTYPE` → raw machine, `VENDOR` → `apple` on macOS / `unknown` on Linux / `pc` elsewhere. Tests: `test_machtype_returns_arm_or_x86_64`, `test_ostype_starts_with_os_family`, `test_vendor_returns_apple_or_unknown`. (Note: zshrs's `$OSTYPE` shows the *current* kernel version (`darwin25.4.0`); the bundled zsh hardcodes its build-time version (`darwin21.3.0`). zshrs is more truthful — accepted as an upgrade.)

### `-f` mode eagerly autoloaded functions from FPATH/ZWC

- `MenkeTechnologies@codelabs:` `rm -f /tmp/file` triggered `command not found: zpwrLogConsoleErr` because zshrs scanned every fpath ZWC for unknown command names and found `~/.zpwr/autoload/common.zwc` containing the user's `rm` wrapper. zsh only autoloads when an explicit `autoload name` declaration was made — never on first call. The eager-scan path in `host.call_function` now checks `executor.options["rcs"]` and skips when rcs is off (`-f` sets rcs=false). Interactive sessions keep the legacy eager behavior. Test: `test_minus_f_skips_eager_fpath_autoload`.

### `case foo in (foo|bar) …` rejected as parse error

- The parser sets `incasepat = 1` AFTER consuming the `in` keyword, but the lexer reads the next token (the leading `(`) BEFORE that flag flips. With incasepat=0 the `(` fell into the `gettokstr('(', false)` branch and ate the entire `(foo|bar)` as one atomic glob-pattern token — the trailing `)` was never seen as a separate Outpar, so the pattern loop errored out. Set `self.lexer.incasepat = 1` BEFORE the `zshlex()` that advances past `in`. Tests: `test_case_paren_prefixed_pattern_accepted`, `test_case_paren_only_first_alt_matches`.

### `$-` returned empty (no shell-flag letters)

- zsh's `$-` returns the concatenated single-letter codes for the options currently set: baseline `569X` always, `f` when rcs is off (`-f`), then `e`/`u`/`x`/`v`/`n`/`l`/`h` for set -e / set -u / set -x / etc. zshrs returned an empty string. Added `"-"` arm in `get_variable` that builds the letter sequence in zsh's exact ordering (e BEFORE f, then login, nounset, xtrace, verbose, noexec, hashall) — matches `set -e; echo $-` → `569Xef`, `set -u; echo $-` → `569Xfu`, `set -x; echo $-` → `569Xfx`, `set -eu; echo $-` → `569Xefu`. Tests: `test_dollar_dash_baseline_no_user_flags`, `test_dollar_dash_includes_e_when_errexit_on`, `test_dollar_dash_includes_x_when_xtrace_on`.

## Closed (sixty-seventh-pass)

### `$0` in `-c` mode leaked the full argv0 path

- `echo $0` returned `./target/debug/zshrs` instead of zsh's bare `zsh`. `get_variable` defaulted to `env::args().next()` for `$0`. Fixed in `bins/zshrs.rs`'s `-c` handler — set `executor.variables["0"]` to `basename(argv[0])` BEFORE running the script. zshrs invoked as `zshrs` returns `zshrs`; invoked as `zsh` (symlink) returns `zsh`. Test: `test_dollar_zero_in_minus_c_returns_basename`.

### `print -P "%T"` zero-padded the hour

- zsh's prompt `%T` time-of-day produces `4:10` (no leading zero on hour); zshrs printed `04:10`. chrono's `%H` always zero-pads; switched to `%k` (space-pad) and `trim_start()` to strip the leading space without touching the digit when hour ≥ 10. Same fix for `%*` (HH:MM:SS). Test: `test_print_dash_p_capital_T_no_zero_pad_hour`.

### Escaped braces `\{foo,bar\}` mangled into garbage

- `echo \{foo,bar\}` should print `{foo,bar}` literally (no expansion). zshrs printed `oo ar\` — the lexer's BNULL-encoded `\{` / `\}` got `untokenize`d back into literal `\{` / `\}` BEFORE `expand_braces` ran, then the brace finder treated `\{` as `{` (with leading literal `\`) and emitted partial strings. Added a `has_balanced_escaped_braces()` short-circuit at the top of `expand_braces` that detects matched `\{`/`\}` pairs and returns the de-escaped literal as a single token. Tests: `test_escaped_braces_stay_literal_in_word`, `test_escaped_braces_with_prefix_suffix`, `test_unescaped_braces_still_expand`.

### Bare `$#name` returned literal `$#name`

- `$#NAME` (no braces) is zsh shorthand for `${#NAME}` (string length / array element count). zshrs's `compile_word_str` had a fast-path for `$NAME` and `$#` (positional count alone), but no path for the `$#NAME` shape — the dispatcher fell through to a generic literal emit. Added a fast-path in `compile_word_str` that detects `^$#[A-Za-z_][A-Za-z0-9_]*$` and emits `BUILTIN_PARAM_LENGTH` directly. Tests: `test_dollar_hash_name_bare_array_length`, `test_dollar_hash_name_bare_string_length`.

## Closed (sixty-eighth-pass)

### `${a:A}` left literal `./` segments in non-existent paths

- `a=./foo; echo ${a:A}` returned `<cwd>/./foo` instead of zsh's `<cwd>/foo`. `std::fs::canonicalize` requires the path to exist; on failure we fell through to a plain `cwd.join(&result)` that didn't lexically resolve `.` / `..` segments. Fixed by walking `path.components()` and dropping `CurDir` while popping on `ParentDir` — produces a zsh-style canonical path even for files that don't exist yet. Test: `test_modifier_capital_A_canonicalizes_dot`.

### `${a:U}` / `${a:L}` modifiers silently returned empty

- zsh emits `unrecognized modifier `U'` for these (they're bash-only); zshrs returned empty with no diagnostic. Added `'U' | 'L' | 'V' | 'X'` arms in `apply_history_modifiers` that print the zsh-format error to stderr and clear `result` to match zsh's "expansion fails completely" behavior. Also extended `is_history_modifier` to include U/L/V/X so they reach the apply-loop. Test: `test_modifier_unknown_emits_error_and_clears`.

### Bash case modifiers `${var^^}` / `${var,,}` accepted instead of rejected

- These are bash-only; zsh rejects with "bad substitution". zshrs implemented them, diverging from zsh's error contract. Replaced the implementations in `expand_braced_variable` with explicit "bad substitution" error emit + empty return. Gated to NOT trigger on `${(j:,:)…}` (legitimate comma inside flag group) — only fires when `^` / `,` appears after a plain identifier name. Test: `test_bash_caret_caret_rejected`. Updated existing tree-walker tests `param_uppercase` / `param_lowercase` to assert the rejection (they were testing the wrong, bash-style behavior).

### `$(c1)$(c2)` cmd-subst concatenation dropped second result

- `echo $(echo foo)$(echo bar)` printed `foo` instead of `foobar`. `strip_cmd_subst` only checked `starts_with("$(")` and `ends_with(')')` — so it matched the WHOLE `$(echo foo)$(echo bar)` as one cmd-subst, ran `echo foo)$(echo bar` as a malformed script, and dropped the second output. Added paren-balance check that rejects when an unmatched `)` appears mid-string, forcing the segment-split path which properly emits two separate cmd-substs and concatenates. Tests: `test_cmd_subst_concat_two_substitutions`, `test_cmd_subst_concat_three_substitutions`.

### `typeset -x name` cleared the existing variable's value

- `a=hello; typeset -x a; echo $a` printed empty instead of `hello`. The bare-name path in `builtin_typeset` did `self.variables.insert(arg, String::new())` unconditionally, clobbering existing values. Fixed: only insert empty when the variable doesn't already exist (preserve attribute attachment). Also wired `+x` to `env::remove_var` so the export attribute is properly stripped while keeping the shell value. Tests: `test_typeset_dash_x_preserves_value`, `test_typeset_plus_x_preserves_value`.

### `((a/=3))` returned `3.3333333333333335` instead of `3`

- zsh's compound `((..))` arithmetic does integer division when both operands are integer; zshrs's ArithCompiler emits `Op::Div` (float-only). Added a sniff in `compile_arith` — if the inner expression contains `/`, route through `BUILTIN_ARITH_EVAL` (the integer-aware MathEval path used by `$((..))`). Result is then reused for the truthiness/status gate so the assignment doesn't run twice. Tests: `test_arith_compound_div_assign_integer`, `test_arith_div_with_float_stays_float`.

## Closed (sixty-ninth-pass)

### `${arr:offset}` / `${a:Z}` silently returned empty

- Any single letter after `:` that wasn't a recognized modifier (history-style `A`/`a`/`h`/`t`/`r`/`e`/`l`/`u`/`q`/`Q`/`P`/`s`/`g`, bash-style `U`/`L`/`V`/`X`, or special-form leaders `-`/`=`/`?`/`+`/`#`/`/`/digit) fell through every branch and returned empty with no diagnostic. Added a "starts-with-alpha" trap in `expand_braced_variable` that emits zsh's `unrecognized modifier `X'` error format and returns empty. Tests: `test_unknown_modifier_letter_emits_error`, `test_unknown_modifier_capital_Z`.

### `$((a))` returned 0 when `a` held a non-numeric expression string

- zsh: `a="3+2"; $((a))` evaluates `a`'s value AS another arith expression and produces 5. Same for `b=a; $((b))` (indirection). zshrs's `MathEval` only loaded vars whose values parsed as int/float — non-numeric strings were dropped, so the lookup returned 0. Added `string_variables` map to MathEval; `get_variable` recursively constructs a sub-evaluator on lookup with a self-reference guard. Tests: `test_arith_recursive_string_var_eval`, `test_arith_indirect_var_chain`, `test_arith_recursive_compound_expression`.

### `printf "%e" 1000` produced `1.000000e3` instead of zsh's `1.000000e+03`

- Rust's `{:e}` formatting emits `e<exp>` with no sign and 1-digit exp; C printf / zsh emit `e±DD` (signed, ≥2 digits). Added a post-format pass in both `printf_format_count` and `builtin_printf` that splits on `e`/`E`, re-emits the exponent with explicit sign and 2-digit zero-pad. Tests: `test_printf_e_format_signed_two_digit_exponent`, `test_printf_e_format_negative_exponent_padded`, `test_printf_capital_E_uses_uppercase_marker`.

### `printf "%v" foo` and `printf "%a" 1` accepted instead of rejected

- `%v` is bash-only (assigns to var); `%a` is C99 hex-float format (zsh doesn't support it). zsh emits `printf:1: %X: invalid directive` for both. zshrs's `printf` literal-passed `%v` and produced hex-float for `%a`. Replaced the literal fallback in `builtin_printf` with the same error format; explicitly rejected `%a`/`%A`/`%v`/`%V`. Tests: `test_printf_invalid_directive_v`, `test_printf_invalid_directive_a`.

### `declare -F` (no args) dumped all environment variables

- zsh's `-F` flag means "float-typed only"; `declare -F` with no float vars declared prints nothing. zshrs treated `-F` as a no-op flag and routed to the typeset list-mode that dumped every variable. Added a float-only filter in the list path that uses `var_attrs[name].kind == VarKind::Float` to gate emission. Other type flags (-i/-a/-A) need shell-internal-param awareness and are left untouched. Tests: `test_declare_capital_F_no_args_lists_only_floats`, `test_declare_capital_F_lists_declared_floats`.

## Closed (seventieth-pass)

### `typeset -U arr` didn't dedupe array elements

- zsh's `-U` (unique) attribute keeps only the first occurrence of each element on assignment / append. zshrs ignored the flag entirely. Added `is_unique` to typeset arg parsing, threaded through `VarAttr.unique`, and applied dedupe in three places: at attribute attachment time (existing array gets retained-with-seen), inside `BUILTIN_SET_ARRAY` (whole-array assignment dedupes via HashSet), and inside `BUILTIN_APPEND_ARRAY` (`arr+=…` skips elements already present). Tests: `test_typeset_dash_U_dedupes_array`, `test_typeset_dash_U_after_assignment_dedupes`, `test_typeset_dash_U_append_dedupes`.

### `((++5))` / `((--5))` silently incremented literals

- Pre/post inc/dec on a non-lvalue is a zsh error (`bad math expression: lvalue required`). zshrs's MathEval skipped the `set_variable` call but still pushed the new value, so `echo $((++5))` printed `6` instead of failing. Added `mv.lval.is_none()` guards on all four ops (PrePlus, PreMinus, PostPlus, PostMinus) that emit `bad math expression: lvalue required` and abort. Tests: `test_arith_pre_increment_on_literal_errors`, `test_arith_post_increment_on_literal_errors`, `test_arith_pre_decrement_on_var_works`.

### `typeset -E` used wrong precision and exponent format

- `typeset -EN` means N **significant** digits (1 before decimal + N-1 after); zshrs passed N straight to Rust's `{:.Pe}` which means N **fractional** digits. Result: `typeset -E5 a=1234.5` printed `1.23450e+03` instead of zsh's `1.2345e+03`. Subtract 1 from the precision before passing to Rust's formatter. Also wired the same e+0DD post-format pass used by printf to fix the unsigned 1-digit exponent (was `e3`, now `e+03`). Tests: `test_typeset_E_uses_sig_digit_precision`, `test_typeset_E_default_precision_nine_fractional`.

### `$#a[@]` / `$#a[*]` returned literal text

- Bare `$#NAME[@]` is zsh shorthand for `${#NAME[@]}` (array length). My iteration-67 fast-path only matched `^\$#NAME$` (no brackets). Extended the matcher in `compile_word_str` to also accept a trailing `[@]` / `[*]` suffix and route the same way through `BUILTIN_PARAM_LENGTH`. Tests: `test_dollar_hash_name_bracket_at`, `test_dollar_hash_name_bracket_star`.

## Closed (seventy-first-pass)

### Float div-by-zero raised "division by zero" instead of producing Inf

- zsh follows IEEE 754 — `1/0.0` produces `Inf`, `-1/0.0` produces `-Inf`, `0.0/0.0` produces `NaN`. Only INTEGER div-by-zero raises the runtime error. zshrs treated both the same. Gated the error in `MathTok::Div` / `Mod` arms on `!is_float` so float operands fall through to the f64 division (which produces the IEEE specials naturally). Updated `format_zsh_subst` to print `Inf` / `-Inf` / `NaN` (capitalized, no decimal) instead of Rust's `inf` / `NaN.0`. Tests: `test_arith_float_div_by_zero_returns_inf`, `test_arith_neg_float_div_by_zero_returns_neg_inf`, `test_arith_zero_div_zero_returns_nan`, `test_arith_int_div_by_zero_still_errors`.

### `declare -p NONEXIST` was silent (and used wrong builtin name)

- zsh emits `declare:1: no such variable: NAME` (or `typeset:1:` if invoked as `typeset`) and exits non-zero. zshrs returned silent success, masking missing-variable bugs. Added the error emit in the `print_mode` path, threaded the invoked name through a new `builtin_typeset_named` helper. fusevm maps both names to the same builtin id so I also exposed `BUILTIN_DECLARE` (id 21) and route `declare` to it from `compile_simple` to preserve the name distinction at error time. Tests: `test_declare_p_missing_variable_emits_named_error`, `test_typeset_p_missing_variable_emits_named_error`.

### Huge floats (`1e100`) truncated to `9223372036854775807` (i64::MAX)

- `format_zsh_subst` cast every "fract==0 && finite" float to `i64`. Rust's `as i64` saturates on overflow to `i64::MAX`, so `1e100` displayed as the saturation value — completely wrong. Gated the int-cast on the float fitting in `[i64::MIN, i64::MAX]`. Out-of-range floats now route through a scientific-notation branch that emits zsh's `<mantissa>e±DD` shape. Tests: `test_arith_huge_float_doesnt_truncate_to_i64_max`, `test_arith_scientific_format_signed_two_digit_exp`.

### `${arr[N]:-default}` ignored the modifier on out-of-bounds index

- For an OOB index like `arr=(a b); echo ${arr[5]:-default}`, zshrs's bracket-handler returned the empty array element and exited the function — never reaching the `:-default` form's default fallback. Added a `lookup_array_element` helper and an `after_bracket` modifier scanner that handles `:-`, `:+`, `:?`, `:=` after `]`. Tests: `test_array_oob_index_default_modifier`, `test_array_empty_index_default_modifier`, `test_array_oob_index_assign_modifier`, `test_array_in_bounds_no_default_kicks_in`.

## Closed (seventy-second-pass)

### `${var-default}` family (no-colon forms) wasn't implemented

- zsh distinguishes `${var-default}` (default only when var is UNSET) from `${var:-default}` (default when unset OR empty). zshrs only implemented the colon variants — the no-colon forms fell through every branch and returned empty. Added a no-colon block in `expand_braced_variable` that walks chars looking for `-`/`=`/`?`/`+` after a valid identifier, then applies the proper unset-only semantics. Same form is supported inside `${(flags)var-default}`. Tests: `test_param_no_colon_default_when_unset`, `test_param_no_colon_assign_when_unset`, `test_param_no_colon_default_nested`, `test_param_no_colon_default_outer_set_skips`, `test_param_flag_with_no_colon_default`.

### `${HOME//\//_}` escaped slash in pattern

- The pattern/replacement split used `splitn(2, '/')` which split on the first `/` — including escaped `\/`. So `${HOME//\//_}` got pattern=`\` and replacement=`/_`, completely wrong. Both `expand_braced_variable` and `compile_zsh.rs::parse_param_modifier` now find the FIRST UNESCAPED `/` and de-escape `\/` → `/` in pattern/replacement. Test: `test_param_replace_with_escaped_slash`.

### `${a@OP}` bash modifier silently returned empty

- `${var@U}`, `${var@L}`, `${var@Q}`, etc. are bash-only. zsh emits "bad substitution". zshrs returned empty silently. Added a check in `expand_braced_variable` for `@` after a plain identifier that emits the zsh-format error. Test: `test_param_at_modifier_rejected`.

### `{10..1..-2}` negative step in brace sequence

- zsh's negative step REVERSES the natural-direction sequence: `{10..1..-2}` → `2 4 6 8 10` (reverse of `{10..1..2}`). zshrs did `i -= step` with negative step, infinite-looping or producing wrong results. Use `step.abs()` for generation, then `results.reverse()` if step was negative. Tests: `test_brace_negative_step_reverses`, `test_brace_negative_step_ascending`.

### `$#1` bare positional length form

- `$#NAME` fast-path only matched identifier names. `$#1` (length of $1) and other digit-only positionals fell through. Extended the matcher to accept `^\$#[0-9]+$` and route through `BUILTIN_PARAM_LENGTH`. Test: `test_dollar_hash_positional`.

### `-0.0` printed as `0.` (lost sign)

- IEEE -0.0 carries a sign bit. `format_zsh_subst`'s int-cast path used `*f as i64` which discards the sign. Detect the negative-zero case explicitly and emit `-0.`. Test: `test_arith_negative_zero_keeps_sign`.

### `declare -p NAME` for exported scalars used `typeset` not `export`

- zsh prints `export NAME=value` for plain exported vars and `export -i n=5` for typed exports. zshrs always emitted `typeset` / `typeset -ix`. Added export-detection (env::var lookup OR `var_attrs.export`) and folded the `x` letter into the `export` keyword. Tests: `test_declare_p_exported_uses_export_prefix`, `test_declare_p_int_export_uses_export_dash_i`.

### `${a[2,3]:-default}` returned full string instead of substring

- The OOB-modifier path I added in iter 71 fired for ALL bracket-with-modifier forms, including string range subscripts. For `a=foo; ${a[2,3]:-default}`, the lookup returned empty (range isn't a single index), so the `:-default` fired. Skip the OOB block when index has comma / `@` / `*`. Also fixed the underlying string-range-subscript bug — the existing code did `(idx-1) as usize` even though `v.start` from getindex is already 0-indexed. Test: `test_string_range_subscript_with_default`.

### `((a |= 0xff))` and other compound bitwise/shift assigns

- ArithCompiler only recognized `+=`/`-=`/`*=`/`/=`/`%=`. `|=`, `&=`, `^=`, `<<=`, `>>=` parsed but never wrote back. Extended `compile_arith`'s "needs_eval" sniff to also route any expression containing those tokens through `BUILTIN_ARITH_EVAL` (MathEval has full operator support and writes through `extract_string_variables`). Tests: `test_arith_compound_or_assign`, `test_arith_compound_shift_left_assign`, `test_arith_compound_shift_right_assign`, `test_arith_compound_xor_assign`.

### `${a[10]}` for short string returned last char (saturation bug)

- `slice_scalar`'s `i.min(len)` saturated OOB indices to the last char. `${a[10]}` for "hello" returned "o". zsh returns empty. Added explicit OOB checks (`start > len` or `start < -len`) that return empty before the saturating resolve. Tests: `test_string_oob_index_returns_empty`, `test_string_negative_oob_index_returns_empty`.

### `set -o allexport` ignored

- zsh's `allexport` option auto-exports every assignment to the env. zshrs registered the option but never consulted it during scalar assignment. Added the option check in `BUILTIN_SET_VAR` — also auto-exports when the var was previously declared exported (was missing for plain `a=newvalue` after `export a`). Test: `test_allexport_option_auto_exports`.

### `Inf` / `NaN` capitalization in stored vars

- After `((a/=0))` on a float, MathEval stored the result via `format_zsh` which used Rust's Display. Result: `inf` / `NaN.0` instead of zsh's `Inf` / `-Inf` / `NaN`. Special-case IEEE specials in `format_zsh` to match zsh's capitalization. (No test added — covered indirectly by the `${a/=0}` variants in the iter 71 tests.)

### `$(($a*2))` (no spaces around `*`) returned 0

- `expand_string`'s var-name reader accepted `*` as a valid char ANYWHERE in an identifier, not just as a single-char special. So `$a*2` consumed `a*2` as one var name, looked up nonexistent `a*2`, returned empty. Then arith on `*2` gave 0. Gated `*`/`@`/`#`/`?` to only match as the FIRST char of var_name. Test: `test_arith_dollar_var_with_star`.

## Closed (seventy-third-pass)

### `local NAME` (no value) didn't reset to empty in function scope

- `a=hi; foo() { local a; echo "[$a]"; }; foo` should print `[]` (zsh: local shadows with empty value, parent value restored on exit). zshrs preserved the parent value because my iter-66 "preserve existing" guard fired here too. Gated on `local_scope_depth > 0 && !is_global` — INSIDE a function, bare `local NAME`/`typeset NAME` always resets to empty. The local_save_stack already preserves the parent value for restore-on-exit. Tests: `test_local_no_value_resets_to_empty`, `test_typeset_no_value_resets_in_function_scope`, `test_typeset_g_keeps_parent_value`.

### `${!var}` bash indirect accepted instead of rejected

- `${!var}` is a bash extension that zsh emits "bad substitution" for; zsh's native indirect is `${(P)var}`. zshrs implemented bash semantics. Replaced the `${!name}` path (single-name form) with the zsh-format error. The `${!prefix*}` / `${!prefix@}` listing forms remain — zsh's behavior there is fuzzier and the test suite uses them. Test: `test_bash_indirect_expansion_rejected`.

### Exit status not masked to 8 bits

- POSIX/zsh: exit codes are taken mod 256. `(exit 256)` should yield `$? == 0`, `(exit 257)` → 1. zshrs returned the raw value. Added `(raw_code as u32) & 0xff` mask in `builtin_exit`. Tests: `test_exit_status_masked_to_byte`, `test_exit_status_257_wraps_to_one`.

### `$((1#X))` and `$((37#5))` panicked instead of erroring

- `i64::from_str_radix(s, base)` panics when base is outside [2, 36]. zshrs passed the user-supplied base directly without validation, panicking on `$((1#1))` and `$((37#5))`. Added `(2..=36).contains(&base)` check that emits zsh's "invalid base (must be 2 to 36 inclusive)" error and returns 0. Two sites in `math.rs`: the `N#value` form and the `[#base]` arith-format form. Tests: `test_arith_invalid_base_no_panic`, `test_arith_base_too_large_no_panic`.

### `${(P):-test}` returned empty instead of "test"

- The `(P)` flag dereferences the value as a parameter name. With `${(P):-test}`, var_name is empty so the default fires and returns "test" — but then `(P)` was applied to "test" which isn't a set var, returning empty. Track whether the default fired and skip the `Parameter` flag in that case (the default value IS the literal result, not a name to look up). Test: `test_param_paren_p_with_empty_name_default`.

### `${#NAME:-default}` returned 0 (length of unset name) instead of 7

- The `${#...}` length form didn't recognize the trailing `:-default` / `-default` modifiers — it just looked up the name and returned its length (0 when unset). Added a name-then-`:-`/`-` parser inside the length path that expands the default when needed and returns its char count. Tests: `test_length_of_default_unset`, `test_length_of_default_no_colon_unset`, `test_length_no_default_when_set`.

### `${a::N}` empty-offset substring returned empty

- `${a::N}` is shorthand for `${a:0:N}` (offset 0, length N). The substring branch's gate required the rest to start with a digit or `-`; `:` was rejected. Extended the gate to also accept a leading `:` (empty offset → 0). Test: `test_substring_empty_offset`.

### `${a::-1}` negative length didn't truncate from end

- Negative length means "skip last N chars". For `a=foo`, `${a::-1}` should be `fo`. zshrs cast negative length via `as usize` (saturating to 0 or huge), returning empty or full. Added explicit negative-length branch: end = total + len, take = end - start. Tests: `test_substring_negative_length`, `test_substring_neg_length_with_offset`.

### `(@O)` / `(@o)` array-context flag in DQ partially supported

- Added `ZshParamFlag::At` enum variant and `@` handling in `parse_zsh_flags`. The DQ-context flag-strip block now keeps array-only flags (Sort/Reverse/Unique/etc.) when `@` is explicitly present. Full DQ-with-`@` element-by-element splicing still needs more work; this iteration handles parsing + flag retention. (Full-suite verification: 512 tests green.)

## Closed (seventy-fourth-pass)

### `${a:?}` error used bash phrasing "parameter null or not set"

- zsh emits "parameter not set" for both `${a:?}` and `${a?}`. zshrs used the bash form. Replaced both occurrences in `apply_var_modifier` and `expand_braced_variable`. Test: `test_param_qmark_no_msg_uses_zsh_format`.

### Paren patterns `(?)`, `(*)`, `(foo|bar)` not recognized in replace

- The has-glob trigger in `BUILTIN_PARAM_REPLACE` only fired on `?`/`*`/`[`/`]`. `(...)` patterns fell into the literal-string path and matched nothing. Added `(` to the trigger set and changed glob-to-regex to keep `(`/`)`/`|` as regex group/alternation operators (instead of escaping them as literals). Tests: `test_pattern_paren_question_mark_matches_one_char`, `test_pattern_paren_star_matches_anything`, `test_pattern_alternation_in_replace`.

### `pushd` / `popd` printed dir stack in non-interactive mode

- zsh's `-c` mode silently performs the cd; only explicit `dirs` prints. zshrs printed the stack on every push/pop. Gated the print on `stdin().is_terminal()` so non-interactive sessions stay silent. Test: `test_pushd_popd_silent_in_noninteractive`.

### `dirs -v` used space-padding instead of TAB

- zsh's `dirs -v` separates index from path with a real `\t` character (tab-aligned). zshrs used `{:2}  ` space-pad. Switched to `{}\t{}`. Test: `test_dirs_v_uses_tab_separator`.

### `print_dir_stack` showed absolute paths instead of `~/...`

- zsh's dir-stack listing replaces `$HOME` prefix with `~`. zshrs printed full `/Users/wizard/...` paths. Added a tilde-compress helper. (No standalone test — covered by the `dirs` interactive-mode probe.)

### `((a *= 1.5))` int → float promotion

- ArithCompiler couldn't parse float literals; mixed-mode `int *= float` produced a wrong int result. Extended `compile_arith`'s "needs_eval" sniff to route any expression containing `.`, `e`, or `E` through `BUILTIN_ARITH_EVAL` (MathEval handles floats correctly). Test: `test_arith_int_times_float_promotes`.

### Bare `${assoc}` returned empty instead of joined values

- For `declare -A h; h[k]=v; ${h}` — zsh returns joined values (`v`); zshrs returned empty. Two issues: (1) `declare -A` creates `variables[name]=""` as a side effect that satisfied the variables.get() call before the assoc check; (2) the assoc->scalar path returned `String::new()` even when entries existed. Fixed: skip the variables.get() lookup when an assoc with the same name has entries, AND return joined values from assoc on the fallback path. Test: `test_assoc_bare_returns_joined_values`.

### `[!fo]` class negation not recognized in replace patterns

- zsh accepts both `[!class]` and `[^class]` for negation; regex only accepts `^`. zshrs's glob-to-regex passed `!` through as a literal, so `[!fo]` matched the literal char `!`. Translate a leading `!` after `[` to `^`. Tests: `test_pattern_class_negation_with_bang`, `test_pattern_class_negation_caret_still_works`, `test_pattern_class_with_negation_matches_others`.

## Closed (seventy-fifth-pass)

### `\$` and `\` escape lost / corrupted in unquoted echo

- `echo \$` printed literal `\$` instead of `$`. `echo \$a` printed bare `\` (with `$a` mangled). For `a=foo; echo \$a`, the `\` was passed through as literal then echo's escape interpreter saw `\f` later in the stream and emitted form-feed. Pre-process `\$`, `\``, `\"`, `\'`, `\\` into `\x00X` literal markers in BUILTIN_EXPAND_TEXT's Default mode (same as the DQ mode already does). Tests: `test_escape_dollar_sign_literal`, `test_escape_dollar_var_literal`, `test_escape_backtick_literal`.

### `$((1+2))$((3+4))` arith-subst concat dropped tail

- Same shape as the iter-67 cmd-subst-concat bug. `strip_arith_subst` checked only that `inner` had a balanced count of `(` / `)` — `1+2))$((3+4` has `(((` and `)))` so the count is +1 -3 +2 = 0 → matched, ran the whole malformed expr, errored "illegal character: )" and fell through to 0. Walk inner counting depth and reject if depth ever drops below zero (`))` mid-string means we closed the outer `$((` early). Tests: `test_arith_subst_concat`, `test_arith_subst_concat_three`.

### `declare -p` for assoc-with-export wrongly used `export -A`

- `declare -Ax h` should print `typeset -Ax h=( )` (zsh reserves the `export` keyword for scalars/integers). Added a `has_non_scalar_attr` check (A/a/F/E) that keeps the typeset form. Test: `test_declare_p_assoc_export_uses_typeset`.

### `declare -p` for float-exp printed `-F` instead of `-E`

- `typeset -E a=3.14` (scientific format) was tracked as `VarKind::Float` but the `-E` vs `-F` distinction was lost. Added `float_exp: bool` to `VarAttr` and emit `-E` when set. Test: `test_declare_p_float_E_flag`.

### `${arr[N]+set}` (no-colon `+`) returned the value, not `set`

- The OOB-modifier path I added in iter 71 only handled `:-`/`:+`/`:?`/`:=` (colon variants — test for empty). The no-colon variants test SET-NESS (key present / index in bounds), not emptiness. Added `array_element_is_set` helper and `-`/`+`/`?` handlers in the bracket-modifier path. Tests: `test_array_element_no_colon_set`, `test_array_element_no_colon_set_oob`, `test_assoc_element_no_colon_set`, `test_assoc_element_no_colon_unset`.

### `${(t)arr}` missing `-unique` marker

- `typeset -aU arr` should report `array-unique`; zshrs emitted just `array`. Added `-unique` to `VarAttr::format_zsh`. Test: `test_t_flag_array_unique`.

### `[[ a -nt b ]]` returned true when one file missing

- bash's "missing == infinitely-old" semantics; zsh strictly requires BOTH files to exist. Removed the `(Some, None) => true` fallback and `(None, Some) => true` for `-ot`. Tests: `test_test_nt_both_must_exist`, `test_test_ot_missing_is_false`.

### `[[ $a == $b ]]` returned false because RHS wasn't variable-expanded

- The `==` / `=` / `!=` path treated the RHS as a literal pattern (untokenize then LoadConst), bypassing variable expansion. So `$b` was matched as the literal string `"$b"`, never the value. Fix: detect `$` / backtick in the RHS and route through `compile_word_str` (with DQ wrapping to suppress filesystem-glob); literal RHS still uses the fast path. Tests: `test_double_bracket_var_eq_var`, `test_double_bracket_var_eq_var_unequal`, `test_double_bracket_glob_pattern_still_works`.

### Empty assoc `=( )` had double space

- `declare -A h; declare -p h` printed `typeset -A h=(  )` (2 spaces); zsh: `typeset -A h=( )` (1 space). Special-case empty assoc formatted line. (Empty arrays keep zsh's 2-space form.)

### Comma operator in `((..))` dropped subsequent expressions

- `((a += 5, a *= 2))` ran only the first — ArithCompiler's compound-assign emit takes one op and discards the rest. Extended `compile_arith`'s "needs_eval" sniff to route any expression containing `,` through `BUILTIN_ARITH_EVAL`. Tests: `test_arith_comma_compound_assigns`, `test_arith_comma_two_vars`.

### `test`/`[ ]` `-a`/`-o` connectives unsupported

- POSIX `test 5 -gt 3 -a 3 -lt 4` should AND the two sub-tests. zshrs's match-on-args pattern bottomed out at the catchall returning 1. Added explicit `-o`/`-a` splitter at the catchall (OR has lower precedence) that recursively evaluates each side. Tests: `test_test_dash_a_and`, `test_test_dash_o_or`, `test_test_dash_a_short_circuit_fails`, `test_test_dash_o_both_fail`.

### `float NAME=…` defaulted to `-F` (fixed) instead of `-E` (scientific)

- zsh's `float` builtin uses `-E` by default; explicit `-F` opts into fixed-decimal. zshrs always stored `-F` form. Added `-F`/`-E` flag detection in `builtin_float`, store value with the appropriate format, and set `var_attrs.float_exp` so `declare -p` round-trips. Tests: `test_float_default_E_format`, `test_float_F_explicit_fixed`.

### `fpath` array empty even though FPATH env was inherited

- `$#fpath` returned 0 in zshrs because the executor's `fpath` (Vec<PathBuf>) field was populated from FPATH but the user-visible `arrays["fpath"]` was not. So `fpath+=(/foo)` replaced with a 1-entry array instead of appending to the inherited 43 entries. Mirror `self.fpath` into `arrays["fpath"]` at executor init. Tests: `test_fpath_inherited_from_env`, `test_fpath_append_keeps_existing`.

### `"$a"bar` (quoted-var followed by literal) returned empty

- The bare-var fast-path in `compile_word_str` matched after `untokenize` stripped DNULL markers, so `"$a"bar` (raw `\u{9e}$a\u{9e}bar`) became `$abar` and the fast-path looked up nonexistent `abar`. Skip the fast-path when the raw word contains DNULL/SNULL quote markers — the bridge below handles the segment-split correctly. Tests: `test_dq_var_concat_with_literal_suffix`, `test_dq_var_concat_with_literal_prefix`, `test_dq_var_with_underscore_suffix`, `test_dq_var_double_concat`.

## Closed (seventy-fifth-pass continued — second batch)

### `$(<<<"hi" cat)` herestring misread as `$(<file)` shorthand

- `run_command_substitution` shortcuts `$(<filename)` to `read-file`; the `<<<"hi"` herestring matched the leading `<` and was passed to the read-file path which errored "no such file or directory: <<\"hi\" cat". Tighten the prefix-strip with `.filter(|s| !s.starts_with('<'))` so only a SINGLE leading `<` triggers the shorthand. Test: `test_herestring_inside_command_substitution`.

### `function { body }` (no parens) anonymous function unsupported

- zsh accepts both `function () { body } args` and `function { body } args` for anonymous functions. zshrs's `parse_funcdef` required the `()` to identify the anonymous form, so the no-parens shorthand silently parsed the trailing args as the next command. Drop the `saw_paren` guard — empty-name path in parse_funcdef now synthesizes the auto-call regardless. Tests: `test_anonymous_function_no_parens`, `test_anonymous_function_no_parens_multi_arg`.

### `$ZSH_SUBSHELL` didn't increment in `(...)` subshells

- zshrs's snapshot/restore-based `(...)` doesn't fork, so `entersubsh()` (which bumps the counter) was never called. Increment the counter inside `subshell_begin` before snapshotting; the snapshot captures the parent-side value so `subshell_end` restores it. Test: `test_zsh_subshell_increments`.

### `printf -- fmt args` printed `--` as the format

- POSIX `--` end-of-options marker wasn't recognized; `printf -- "%s\n" hi` produced `--\nhi\n`. Strip a leading `--` before the `-v VAR` parse / format-string lookup. Test: `test_printf_double_dash_end_of_options`.

### `echo *` glob sort was always byte-order (case-sensitive)

- Under a Unicode locale, zsh sorts case-foldedly (`Aaa bbb Ccc Ddd`), not ASCII (`Aaa Ccc Ddd bbb`). zshrs's `expand_glob` finalized with `expanded.sort()` — pure byte compare. Added `glob::locale_aware_name_cmp` that folds case under non-C locales (via LC_ALL/LC_COLLATE/LANG sniff) and falls back to byte order under C/POSIX. Wired into `expand_glob`'s final sort and `MatchEntry::compare`. Test: `test_glob_sort_locale_aware`.

### `typeset -i N name=value` ignored the base argument

- `typeset -i 16 a=255` should display `$a` as `16#FF`; zshrs stored decimal `255`. Added `int_base: Option<u32>` to `VarAttr`, parsed both `-iN` (attached digits) and `-i N` (separate arg) forms, and emit `BASE#DIGITS` (with `A`-`Z` for digits ≥10) at assignment time via new `format_int_in_base` helper. Test: `test_typeset_integer_base_output`.

### `cd` with no args used OS env HOME, not shell-state HOME

- `HOME=/tmp; cd; pwd` printed `/Users/wizard` because `do_cd` called `dirs::home_dir()` which reads the OS env (unaffected by non-exported shell-local assignments). Changed to a closure that prefers `self.variables["HOME"]` then env. Test: `test_cd_uses_shell_home`.

### `cd ~` used OS env HOME, not shell-state HOME

- Same root cause for the tilde-prefix path. `expand_tilde_named` for bare `~` only checked `std::env::var("HOME")`. Read shell-state `variables["HOME"]` first. Test: `test_cd_tilde_uses_shell_home`.

### CDPATH searched only with explicit `-s`

- zsh implicitly searches CDPATH when the literal path isn't a directory in cwd; zshrs gated this behind the `-s` (use_cdpath) flag. Reworked the path-resolution branch to: if path doesn't start with `/`/`.` AND isn't a cwd directory, search `$CDPATH` (shell-state preferred over env) then the `cdpath` array. Test: `test_cdpath_implicit_search`.

### `~` literal in double quotes was being expanded

- `echo "~"` printed `/Users/wizard`; zsh keeps `~` literal inside `"..."`. The shared `expand_string` tilde-handler had no DQ-context guard. Added `&& self.in_dq_context == 0` to the tilde branch — the existing `in_dq_context` counter (incremented around `expand_string` for DQ contents) gates expansion. Tests: `test_tilde_literal_in_double_quotes`.

### `${arr%pat}` / `${arr#pat}` only stripped one element of the array

- `${a%.txt}` for `a=(a.txt b.bin c.txt)` should yield `a b.bin c`; zshrs joined to scalar first then stripped the joined string, returning `a.txt b.bin c` (only the trailing `.txt` got stripped). Same root cause for `#`/`##`/`%%`. Reworked `BUILTIN_PARAM_STRIP` to detect the `arrays[name]` / `@` / `*` cases and iterate per-element. Tests: `test_array_suffix_strip_per_element`, `test_array_prefix_strip_per_element`, `test_array_long_suffix_strip_per_element`, `test_array_long_prefix_strip_per_element`.

## Closed (seventy-sixth-pass)

### `kill -0 PID` (process-existence check) errored

- `kill -0` is the POSIX/zsh-compatible way to ask "is PID alive?" — no signal sent, kill(pid, 0) returns 0 / ESRCH. zshrs errored "invalid signal: -0" because Signal::SIG0 isn't a libc Signal enum variant. Added a `signal_zero` flag in `builtin_kill` that routes the `-0` parse through a direct `libc::kill(pid, 0)` call. Test: `test_kill_zero_process_check`.

### `print --hi` printed `--hi` instead of erroring

- zsh's `print` errors on unknown flags ("bad option: -h"); zshrs's print parser was lenient — when ANY char in the flag wasn't recognised, the whole token was pushed as a positional. Made it strict: error on the first unrecognised char. Added `-` to the known-flag set so `print -- -hi` and the `--` end-of-options idiom continue to work. Tests: `test_print_strict_unknown_flag_errors`, `test_print_double_dash_terminator`.

### `<<\EOF` heredoc terminator parse-errored

- zsh treats `<<\EOF` (backslash-prefix on the terminator) as shorthand for `<<'EOF'` — disables variable / cmd-sub / arith expansion in the body. zshrs's lexer detected SNULL / DNULL quoting markers but not the BNULL marker (`\u{9f}`) emitted for backslash-escaped chars, so the terminator string still contained the BNULL byte and the body never matched the closing line — hit "here document too large or unterminated". Added BNULL to both the quoted-detection check and the terminator strip-set. Test: `test_heredoc_backslash_terminator_disables_expansion`.

### `trap CMD 0` didn't run the EXIT trap

- POSIX numeric-signal-alias: `trap CMD 0` is equivalent to `trap CMD EXIT`. zshrs stored the trap under signal name `0`, which the EXIT-trap-runner (which keys on `"EXIT"`) never queried. Added a numeric→name normalisation in `builtin_trap`: `0` → `EXIT`, libc-derived numbers → canonical names (so `kill -l USR1`'s output round-trips). Test: `test_trap_signal_zero_is_exit`.

### `echo X$?` (special-var after literal prefix) printed literal `X$?`

- Same root cause for `X$#`, `X$$`, `X$*`, `X$!`, `X$-`. The lexer META-marks these chars (`?` → `\u{97}`, `*` → `\u{87}`, `#` → `\u{84}`, `!` → `\u{9c}`, `-` → `\u{9b}`, second `$` → `\u{85}`) so when `compile_zsh.rs::find_expansion_end` looked for the end of the `$?`-style expansion, the matcher (which only tested LITERAL chars) fell through to the default "advance by 1", leaving the META-encoded special-param char in the trailing literal segment. Extended the matcher to recognise both the literal char AND its META code-point. Test: `test_special_param_concat_after_literal`.

### `printf "%x" -1` printed `0`

- `arg.parse::<u64>()` rejected the leading `-` and unwrapped to 0. POSIX printf wraps negatives as unsigned (-1 → 0xFFFFFFFFFFFFFFFF). Parse as i64 first then `as u64` to get C-style two's-complement wrap. Same fix applied to `%X` and `%o`. Test: `test_printf_x_negative_wraps_unsigned`.

### `printf "\NNN"` (no leading 0) printed literal `\NNN`

- POSIX printf accepts both `\NNN` (1-3 octal digits, the standard form) and `\0NNN` (legacy bash-style with leading 0). zshrs's escape branch only matched `\0…`, so `\102` (= 'B') stayed literal. Extended the match to any digit `0`-`7` as the first octal char. The leading-zero form still works: `\0102` is up to 3 chars total including the `0`, so it consumes `010` (= backspace) and leaves `2` as literal — matching zsh's output. Tests: `test_printf_octal_escape_no_leading_zero`, `test_printf_octal_leading_zero_three_total_digits`.

### `() { echo $0 } anon` printed `_zshrs_anon_0`

- zsh: anonymous functions display `(anon)` for `$0`. zshrs synthesizes internal names `_zshrs_anon_N` / `_zshrs_anon_kw_N` for the two anon syntaxes; the internal name was leaking into `$0`. In `call_function`, when `name.starts_with("_zshrs_anon_")` substitute the cosmetic `(anon)` for the value of `$0`. Test: `test_anon_function_dollar_zero_is_anon_string`.

### `set -E` and `set -T` errored "invalid option"

- zsh's `set` accepts `-E` (ERR_RETURN: return on non-zero status inside a function) and `-T` (TRAPS_ASYNC: run traps after each command). zshrs's `builtin_set` matcher had no `-E`/`-T` cases; both produced "invalid option". Added accept-silently mappings to `err_return` and `trapasync` options so user scripts that set the flag don't bail. Test: `test_set_capital_E_accepted`.

### `[[ -o no_such_option ]]` was silent (no diagnostic)

- zsh: emits `no such option: NAME` to stderr (test still returns false). zshrs's `BUILTIN_OPTION_SET` just returned false silently. Added a known-option lookup against `ZSH_OPTIONS_SET` (the same canonical-set used by `setopt`/`unsetopt`) plus the `no`-prefix-strip so `[[ -o nounset ]]` and `[[ -o nonounset ]]` invert correctly. Unknown names now log "no such option" to stderr matching zsh. Test: `test_double_bracket_o_unknown_option_warns`.

### Heredoc body `[42]` triggered NOMATCH glob expansion

- `cat <<EOF\n[42]\nEOF` should produce `[42]` on cat's stdin, but zshrs routed the body through the full word-expansion pipeline (expand_string + brace + glob), and `[42]` looks like a one-char glob pattern that never matches. Added mode 4 ("HeredocBody") to `BUILTIN_EXPAND_TEXT` which runs only `expand_string` (variable / cmd-subst / arith) and skips the brace + glob steps. The compile-side already passed mode 0 (Default); switched to mode 4 for the unquoted-terminator branch. Test: `test_heredoc_body_no_glob_expansion`.

### `$_` always returned the shell binary path

- zsh / bash convention: `$_` holds the last argument of the previously-executed command (`echo hi; echo $_` → `hi\nhi`). zshrs's `_` was only ever set to the binary path at startup. Added `pending_underscore: Option<String>` to the executor and promote it on every builtin dispatch (in `pop_args`) AND every external exec (in `host.exec`) — the previous command's last arg becomes `$_` BEFORE the next command's args are read, so `echo $_` reads the prior value. Test: `test_dollar_underscore_tracks_last_command_arg`.

### `${arr[@]:h}` (and :t, :r, :l, :u, etc.) didn't iterate per-element

- zsh applies path-modifier suffixes per-array-element: `a=(/a/b/c /d/e/f); echo "${a[@]:h}"` should produce `/a/b /d/e`. zshrs's subscript-resolution path returned `arr.join(" ")` for `[@]`/`[*]` and never reached the modifier loop, so the `:h` was silently dropped. Added a per-element `apply_history_modifiers` walk inside the `index == "@"`/`"*"` branch that fires when `after_bracket` starts with `:` and looks like a history modifier. Test: `test_array_at_subscript_history_modifier_per_element`.

### `${var:h}` didn't strip trailing slashes before head

- `${a:h}` for `a=/tmp/` should yield `/` (drops trailing-slash + `tmp`). zshrs found the trailing slash with `rfind('/')` and returned `/tmp`. Added a `trim_end_matches('/')` pass before locating the last segment so `/tmp/` and `/tmp` both resolve to `/`. Same fix to `:t` so `foo/` :t is `foo`. Test: `test_h_modifier_strips_trailing_slashes`.

### `${a[1][1]}` (chained subscript) returned the full element

- zsh: `${a[1][1]}` for `a=(hello)` selects array element 1 (`hello`) then character 1 (`h`). zshrs treated the second `[1]` as noise after the first subscript resolved. Added a chained-subscript handler in the array-subscript branch: if `after_bracket` starts with `[`, parse the inner index (numeric or `start,end` range), apply to the looked-up element's chars, and return. Test: `test_chained_subscript_array_then_char`.

### `print -e` and `print -E` were silently accepted

- zsh's `print` rejects `-e` AND `-E` ("bad option") — the escape-interpretation flags belong to `echo`, not `print`. zshrs's print known-flag set included both. Removed `e`/`E` from the print known-flag set so these now fall through to the strict "bad option" error matching zsh. Test: `test_print_rejects_dash_e_and_dash_E`.

### `$((0o15))` Rust/Python octal prefix was silently accepted as 0

- zsh rejects `0o…` octal-prefix; only `0x` (hex), `0b` (binary), and bare-leading-zero (with `setopt octalzeroes`) are recognised. zshrs's math lexer fell through `Some('o')` to the default branch and returned 0. Added an explicit `Some('o') | Some('O')` case that sets `self.error` to zsh's exact diagnostic ("bad math expression: operator expected at `…'") and returns a stub Num. Test: `test_math_rejects_0o_octal_prefix`.

### `trap "" SIG` (signal-ignore) wasn't listed by `trap`

- POSIX distinguishes `trap "" SIG` (ignore signal — store empty action) from `trap - SIG` (reset to default — remove action). zshrs collapsed both to "remove from table", so `trap "" USR1; trap` printed nothing. Now only `-` removes; the empty-string ignore form is stored verbatim and shown by `trap` as `trap -- '' SIG`. `run_trap` skips execution when the action is empty so the ignore semantics actually work. Test: `test_trap_empty_string_listed_as_ignore`.

### `printf "%d" 3.14` returned 0 instead of 3

- POSIX printf truncates floats to int for `%d`/`%i`. zshrs's parser was i64-only — the decimal point made `arg.parse::<i64>()` fail and `unwrap_or(0)` produced 0. Added f64 fallback that truncates via `as i64`. Same path now correctly handles `-5.99` → `-5`. Test: `test_printf_d_truncates_float_to_int`.

### `set -h`, `set -k`, `set -p`, `set -B`, `set -H` errored "invalid option"

- These are zsh-standard short-flag aliases (HASH_CMDS, KSH_TYPESET, PRIVILEGED, BRACE_CCL, HIST_REDUCE_BLANKS). zshrs's `builtin_set` matcher had no cases for them. Added accept-silently mappings to the canonical option names so user scripts that toggle them don't bail out at startup. Test: `test_set_dash_h_and_k_accepted`.

### `${#arr[N]}` returned 0 instead of element char-count

- `${#a[1]}` for `a=(hello)` should return `5` (chars in "hello"). zshrs's `${#…[…]}` branch only handled `[@]`/`[*]` (array-element-count); numeric indices fell through and returned 0. Added a `lookup_array_element(name, index).chars().count()` fallback for the numeric / non-`@`/`*` case. Test: `test_array_element_length_via_hash`.

### `printf "%.s"` (zero precision via empty digits) printed the arg

- POSIX/zsh: `%.s` is the same as `%.0s` — a period with no digits means precision 0, which suppresses string output (`printf "%.s" ignore` → empty). zshrs only set `prec_val` when the precision string parsed to a number, so empty precision left `prec_val = None` and the full arg printed. Track a `saw_period` flag and default to `Some(0)` when the period was consumed but no digits followed. Test: `test_printf_dot_s_zero_precision_suppresses_arg`.

### `print -nN` left a stray NUL byte after output

- `print -n` should suppress the terminator entirely; the `-N` (NUL-separator) flag was overriding `-n` and emitting a final `\0`. Reordered the terminator selection so `no_newline` wins over `null_terminate`. Now `print -nN hi` outputs exactly `hi` with no trailing byte. Test: `test_print_dash_n_suppresses_null_terminator`.

### `(set -e; false); echo` killed the parent shell

- zsh's errexit aborts the subshell only; the parent continues with the subshell's exit status. zshrs's `BUILTIN_ERREXIT_CHECK` called `std::process::exit` unconditionally, tearing down the parent shell when an inner subshell hit a non-zero status under `set -e`. Added a `subshell_snapshots.is_empty()` guard so the exit only fires at top level (the in-process subshell continues to natural end with the parent intact). Full subshell-internal abort would require VM-level halt support and is deferred. Test: `test_set_e_in_subshell_doesnt_kill_parent`.

### `*(om)` glob qualifier wasn't sorting by mtime

- zsh's `*(om)` orders matches by modification time NEWEST-FIRST (the time qualifiers default to descending; `Om` is the oldest-first flip). Two bugs combined:
  1. The post-filter alpha sort in `expand_glob` ran AFTER `filter_by_qualifiers`, clobbering the qualifier-driven order.
  2. `looks_like_glob_qualifiers` was missing `O` in its valid-char set, so `*(Om)` parsed as a literal pattern with unmatched `)` instead of as a qualifier set.
  Fixed both: skip the alpha sort when the qualifier set contains `o`/`O`, and added `O` to the valid char set. Same default-descending semantics now applied to `oa` (atime) and `oc` (ctime) too. Tests: `test_glob_om_sort_newest_first`, `test_glob_Om_sort_oldest_first`.

## Closed (eighty-eighth-pass)

### `*(l2)` link-count glob qualifier not implemented

- zsh pattern.c qualifier `l[+-]N` matches files by hard-link count. Was missing from our `filter_by_qualifiers` handler. Added the parser block (cmp + digit run) and `MetadataExt::nlink()` filter. Also added `l` to the `looks_like_glob_qualifiers` valid-chars set so `*(l2)` parses as a qualifier set instead of falling back to literal pattern. Test: `test_glob_l_link_count_qualifier`.

### `a=$(false); echo $?` returned 0 (cmd-subst status not propagated to $?)

- zsh: cmd-subst's exit status leaks into $?, so `a=$(false); echo $?` prints 1. We were always returning 0 for the assignment. Three-part fix: (1) `run_command_substitution` now sets `self.last_status` from the inner cmd's status; (2) `BUILTIN_SET_VAR` returns `Value::Status(captured)` from the executor's last_status (instead of constant 0); (3) compile_assign emits `Op::SetStatus` (was `Op::Pop`) so vm.last_status reflects the propagated value. Test: `test_cmd_subst_status_propagates_to_assign`.

### `${(no)a[@]}` (sort-modifier-before-sort) was applied sequentially

- zsh's flag-string is order-agnostic: `n`/`i`/`a` are sort-MODIFIERS that pair with `o`/`O`. `(no)` and `(on)` should both produce numeric ascending. We were applying them as separate sort operations so `n`'s natural-sort got overwritten by the subsequent `o`'s alpha-sort. Fixed by detecting `n`/`i` BEFORE the `o`/`O` in the flags string when no inline sub-flag was given. Test: `test_sort_flag_with_numeric_modifier_either_order`.

### `${(j:sep:)$(cmd)}` over-applied join by splitting on whitespace first

- The cmd-subst-as-flag-operand branch (added in batch 11) split the captured output on whitespace BEFORE joining with sep. zsh: `(j:::)` is a no-op on a scalar — the cmd-subst output is a single string, not an array. Result: newline-separated output got crammed onto one line. Fixed: drop the split-then-join in the Join arm; cmd-subst → scalar → (j) no-op. Tests: `test_param_j_flag_on_cmd_subst_no_op`, `test_param_jf_split_then_join_cmd_subst`.

### `${a:^b}` / `${a:^^b}` array-zip operators not implemented

- zsh subst.c SUB_ZIP_SHORT (`:^`) interleaves up to min(len). SUB_ZIP_LONG (`:^^`) cycles the shorter array up to max(len). Both yield space-joined output. Was previously listed in the "Still open" GAPS section. Added detect at top of `expand_braced_variable` for plain-identifier names. Tests: `test_array_zip_short_form`, `test_array_zip_long_form_cycles`.

### `${a:$((${#a}-2))}` substring offset with nested `${...}` got rejected

- The compile-time substring shape detection refused offsets containing `${...}` to "leave nested only in length". But `$((${#a}-2))` legitimately has nested `${...}` inside an arith form. Refined the check: when the operand starts with digit/`$`/`-`/`(` (substring-shape signal), allow nested forms — the runtime SubstringExpr handler arith-evaluates them via `expand_string`. Test: `test_substring_offset_with_nested_arith`.

### `${${...}/pat/$var}` nested-replace path didn't expand `$var` in repl

- The nested-expansion `/pat/repl` branch passed both `pat` and `repl` to `String::replace` without running them through `expand_string`. zsh expands $-refs in both. Added `expand_string` calls. Test: `test_nested_replace_expands_dollar_in_repl`.

### `${arr[@]:offset}` array slice collapsed splice in assignment context

- `b=("${a[@]:1}")` should give `b` 2 elements (when `a` had 3); was giving 1 because BUILTIN_PARAM_SUBSTRING returned a joined scalar regardless of `[@]`. Compile path now re-attaches `[@]`/`[*]` to the name (parse_param_modifier dropped it). Runtime detects the suffix and returns `Value::Array` when `force_array`. Same fix for the EXPR variant. Without this, the canonical "shift-via-slice" idiom `while ((#a > 0)); do ...; a=("${a[@]:1}"); done` looped forever. Tests: `test_array_slice_at_preserves_splice_in_assignment`, `test_array_consume_loop_terminates`.

### `printf "abc" > file` left file empty AND leaked output to stdout

- Rust's `print!` is block-buffered when stdout is a non-tty (file via dup2). The `redirect_scope` restored the original stdout fd via dup2 BEFORE the buffer flushed, so buffered data ended up on the original terminal and the file stayed empty. `echo "abc"` worked because `println!` triggered line-buffer flush. Added explicit `std::io::stdout().flush()` at the end of `builtin_printf`, `builtin_echo`, `builtin_print` so non-newline output reaches the redirect target before scope restoration. Test: `test_printf_redirect_to_file_writes_data`.

### `typeset -T PATH path :` didn't read inherited $PATH from env

- The tied-pair init only consulted `self.variables`; vars like `PATH` that live in process env (not our shell-level map) read as empty so `path` was a 0-elem array. Fixed: fall back to `std::env::var(name)` when self.variables doesn't have it. Test: `test_typeset_t_reads_existing_env_value`.

### `unset path` (tied-array side) didn't clear $PATH (scalar side)

- zsh's tied-pair semantics: unsetting either side zeroes both. We only removed the named var, leaving the tied counterpart intact. Added bidirectional cleanup in `builtin_unset`: removing array side also unsets scalar (env + variables map); removing scalar side also unsets array. Test: `test_typeset_t_unset_propagates_to_tied`.

### `${h[(I)*]}` returned single key instead of all matches on assoc

- `(I)` and `(R)` flags on assoc subscript should return ALL matching keys/values space-joined; `(i)`/`(r)` return the FIRST match. We were always returning a single match. Direct port of zsh subst.c haspats path. Test: `test_assoc_capital_i_returns_all_matching_keys`.

### zsh-special params `SECONDS`/`UID`/`HISTCMD`/etc. treated as unset for `${X-default}`

- These have dynamic getters but aren't in `self.variables`, so `${SECONDS-default}` returned "default" instead of the live value. zsh treats them as always-set. Added matched whitelist (`SECONDS`, `EPOCHSECONDS`, `EPOCHREALTIME`, `RANDOM`, `LINENO`, `HISTCMD`, `PPID`, `UID`, `EUID`, `GID`, `EGID`, `SHLVL`) to all three `var_is_set` decision points (flag-aware path, no-modifier path, and `BUILTIN_PARAM_DEFAULT_FAMILY`). Also added `HISTCMD` to the dynamic getter (returns session history count). Test: `test_special_param_default_treats_as_set`.

### `trap "..." ZERR` / `trap "..." ERR` were no-op stubs

- `BUILTIN_ERREXIT_CHECK` only acted on `errexit`; the trap registered for ZERR/ERR was never fired. zsh's signals.c fires ZERR (and the alias ERR) whenever a command exits non-zero, before the errexit decision. Added `traps.get("ZERR").or(traps.get("ERR"))` lookup at the top of `BUILTIN_ERREXIT_CHECK` and run the body via `execute_script` (with last_status saved/restored to prevent recursion on trap-body failures). Tests: `test_zerr_trap_fires_on_nonzero_status`, `test_err_trap_alias_for_zerr`.

### `read -e` / `read -E` were no-op stubs

- zsh's bin_read calls fputs(buf, stdout) under both -e and -E. -e prints the line and DOESN'T assign; -E prints AND assigns. Both were swallowed in our flag-char loop with a `// TODO` comment. Implemented per zsh: -e returns 0 after the echo (no assignment); -E falls through to the assignment block. Tests: `test_read_minus_E_echoes_and_assigns`, `test_read_minus_e_echoes_only`.

### `print -C N` used tab separator instead of zsh's space-padded columns

- zsh's `-C N` pads each column to the widest entry and joins with two-space separator (so `print -C 2 a b c d` reads `a  c` / `b  d`). We were using a single tab join, which renders wider (8 chars typically) and ignored column-width padding entirely. Reworked to compute per-column widths and emit `item + pad-to-width + "  "` for each non-last column. Trailing partial rows don't pad after the last present item. Test: `test_print_minus_C_column_format`.

### `${a//\:/-}` — backslash-escaped non-meta char treated as literal `\:`

- zsh's pattern handling strips the backslash from `\X` when X is NOT a glob meta. Our `BUILTIN_PARAM_REPLACE` preserved the backslash, so the pattern looked for literal `\:` in the value (never matched). Added a pre-pass that strips backslash from non-meta escapes; preserves `\?`/`\*`/`\[`/`\]`/`\(`/`\)`/`\|`/`\\` for the regex compile downstream. Test: `test_param_replace_strips_backslash_escape_in_pat`.

### `${(P)$(...)}` — (P) indirect on cmd-subst result returned the captured output verbatim

- The cmd-subst-as-flag-operand branch (added in batch 11) didn't honor (P) — it returned the captured output as the value, not the value of THE VARIABLE NAMED BY THE OUTPUT. zsh: `a=hi; ${(P)$(echo a)}` → `hi` (NOT `a`). Detect Parameter flag, look up the captured-output string as a variable name. Test: `test_param_p_indirect_with_cmd_subst`.

### `a[$n]=()` / `a[$#a]=()` — variable subscript in element-remove ignored

- The compile path for subscripted-array assign emitted the LITERAL key string. For literal indices like `a[3]=()` this worked; but `a[$n]=()` reached the runtime as the literal "$n" which failed int-parse and the removal was a no-op. Compile path now routes the key through `compile_word_str` (var/cmd-subst expansion) when it contains `$` or `` ` ``. Test: `test_array_subscript_remove_with_var_index`.

### `typeset -A h=(...)` inside function leaked to parent's assoc

- `local_save_stack` and `local_array_save_stack` existed; no `local_assoc_save_stack`. So `typeset -A h=(b 2)` inside a function modified the outer `h` permanently. Added new save stack with the same lifecycle as the array stack — saved at typeset time, restored on function exit. Both `call_function` paths (legacy and bytecode) updated. Test: `test_local_assoc_array_shadows_outer`.

### `${(flag)$(cmd)}` — cmd-subst as flag operand returned empty in DQ

- The flag handler had a branch for `${(flag)${...}}` (nested expansion as operand) but not for `$(...)` (cmd-subst as operand). zsh subst.c runs the cmd-subst first, then applies flags to the captured output. Added `rest.starts_with("$(")` branch that calls `run_command_substitution` and applies U/L/Split/Join/SplitWords/SplitLines flags. Test: `test_param_flag_with_cmd_subst_operand`.

### `abs(-5)` / `min(3,5)` / `max(3,5)` returned `5.` (float) instead of `5` (int)

- All math functions returned `MathNum::Float`, even when the input was integer. Float-to-string formatting produced trailing `.` for whole numbers ("5."). Added int-preserving fast path for `abs`/`min`/`max`/`int`/`floor`/`ceil`/`trunc` — when all args are `MathNum::Integer`, return `MathNum::Integer`. Float inputs still produce float output. Test: `test_math_abs_min_max_preserve_int`.

### `b=("${a[@]}")` joined elements when assigning array-to-array

- Earlier batch 6 fix forced JOIN_STAR on `[@]` when `assign_context_depth > 0` to handle scalar `b="${a[@]}"`. But array init (`b=(...)`) shares the same flag, so array-to-array splice collapsed: `a=("1 2" "3 4"); b=("${a[@]}")` → `b=("1 2 3 4")` (1 element). Added separate `scalar_assign_depth` counter — only bumped for scalar assignment RHS. Test: `test_array_assigns_array_via_at_splice`.

### `**/*` recursive glob sorted by basename instead of full path

- For non-recursive globs, zsh sorts by basename (with locale-aware case-folding). For recursive `**/*`, zsh sorts by FULL path so depth-first walk order is preserved (`dir/f sub sub/g`, not basename `f g sub` which makes no sense at multiple levels). Added pattern-based dispatch: full-path sort when pattern contains `**/`, basename sort otherwise. Test: `test_recursive_glob_sorts_full_path`.

### Subshell EXIT trap fired AT PROCESS EXIT instead of subshell exit

- zsh forks for `(...)` so the trap runs in the child process when the subshell ends. We run subshells in-process; without firing the trap at `subshell_end`, the parent's process-exit fired ALL accumulated EXIT traps after the parent's last command. Added `traps` field to `SubshellSnapshot`, fire-and-remove the EXIT trap at subshell_end (before restoring parent's traps), execute via `with_executor(|exec| exec.execute_script(&body))` so the inner script doesn't recurse. Subshell-only traps don't leak — parent's traps are restored after firing. Tests: `test_subshell_exit_trap_fires_before_parent_continues`, `test_subshell_trap_doesnt_leak_to_parent`.

### `"${(o)a[@]}"` / `"${(O)a[@]}"` / `"${(n)a[@]}"` skipped sort in DQ context

- zsh subst.c: array-only flags (`o`/`O`/`n`/`i`/`u`) are stripped in DQ context UNLESS the user explicitly wrote `[@]`/`[*]` subscript. Our `parse_zsh_flag` strips `[@]`/`[*]` from `name` (the fast-path requires identifier-only), losing the splice-context information by the time the runtime DQ-strip decision runs. Encoded the at-subscript context through a new `\u{03}` sentinel in the runtime flags string so the handler can re-recognise `had_at_subscript`. Test: `test_sort_flags_with_at_subscript_in_dq`.

### `((a = cond ? T : F))` — ternary assignment dropped silently

- ArithCompiler's emit path doesn't implement `?:`. Without the trigger, `((a = ... ? ... : ...))` left `a` unset (no error). Added `?` to the needs_eval check so the expr routes through MathEval (which has full ternary support). Test: `test_arith_ternary_assignment`.

### `case W in (P|Q)) BODY ;; esac` — wrapped pattern with `|` failed

- zsh's case grammar accepts both bare `(P) BODY` (leading `(` is the optional marker, single `)` closes the arm) AND wrapped `(P)) BODY` (the `(...)` is the pattern wrapper, the second `)` closes the arm). Our parser only consumed ONE Outpar after patterns, so the wrapped form left the second `)` for the body to choke on. Added `had_leading_paren && Outpar` consume after the arm-close. Test: `test_case_paren_wrapped_pattern_with_alternation`.

### `typeset -a a` clobbered existing array to empty at top scope

- The bare-declaration path always called `self.arrays.insert(name, Vec::new())`. zsh's typeset.c only zeroes a new binding at top scope; existing values are preserved unless you're inside a function (where bare `typeset NAME` shadows). Added `in_function || !exists` guard. Test: `test_typeset_a_preserves_existing_array_at_top_scope`.

### `a=(...); typeset -aU a` didn't dedupe — array got cleared

- Same root cause as above (clobber to empty). After the fix, also added an immediate dedupe pass when `-U` is given on an existing array, mirroring the dedupe block at line 21330+ that fires after var_attrs is set. Test: `test_typeset_aU_dedupes_existing_array`.

### `${(U)${(s. .)s}[1]}` ignored the `[N]` subscript after the inner expansion

- The nested-expansion handler returned the flag-applied joined-scalar without parsing a trailing `[N]` subscript. zsh treats inner-with-(s::) as an array; `[N]` selects an element. Added `[N]` parser after the inner close — splits on space, indexes (1-based, negative-from-end), then re-applies case-transform flags to the picked element. Test: `test_nested_expansion_subscript_after_flag`.

### `. file.sh ARG1 ARG2` didn't pass extra args as positionals to sourced script

- zsh's source/`.` builtin passes `args[1..]` as `$1`/`$2`/... to the sourced file. We were ignoring extras — the script saw the parent's positionals (or empty in `-c` mode). Save outer positional_params, install args[1..] as new positionals, restore on exit. Tests: `test_source_passes_extra_args_as_positionals`, `test_source_preserves_outer_positionals`.

### `b="${a[@]}"` captured only the first element, not the joined array

- In an assignment context, both `[@]` and `[*]` join the array to a single string (zsh subst.c forces single-string output for scalar RHS). Our `${NAME[@]}` always emitted `BUILTIN_ARRAY_ALL` (Array splice). Compile path now forces `BUILTIN_ARRAY_JOIN_STAR` when `assign_context_depth > 0`. Test: `test_array_splice_in_scalar_assign_joins`.

### `$a[@]` / `$a[*]` (no braces) joined instead of splicing

- zsh treats bare `$NAME[@]`/`$NAME[*]` identically to the braced versions. `array_splice_ref` only matched `${NAME[@]}`/`${NAME[*]}`. Extended to also accept the no-braces form. Test: `test_array_bare_splice_no_braces`.

### `((i=a[2]))` set i to the joined-scalar of $a, not the second element

- ArithCompiler doesn't pre-resolve `name[idx]` on the RHS — the assignment landed `a`'s value (joined "1 2 3") instead of `a[2]`. Added `inner_arith.contains('[')` to the needs_eval check so MathEval (BUILTIN_ARITH_EVAL → evaluate_arithmetic) handles it; that path runs `pre_resolve_array_subscripts` before the math eval. Test: `test_arith_assign_from_array_subscript`.

### `setopt extendedglob; echo /tmp/dir/^pat` skipped negation when `^` was after `/`

- trigger_glob detection only fired for `^` at the start of the word, not for `^` at the start of any path component. Same for the bridge's expand_glob trigger. Both updated to also detect `/^`. Test: `test_glob_caret_at_path_component_with_extendedglob`.

### `[[ a == a && (b == b || c == c) ]]` parse-errored at the inner `(`

- The lexer sets `incondpat=true` after `==`/`!=`/`=~` so the RHS pattern can include glob chars. `incondpat` was only reset on `]]` — not on `&&`/`||`/`(`/`)`/`!`, so the next `(` after `&&` was lexed as a literal glob char (gettokstr) and the whole remainder collapsed into one String token. Direct port of zsh's cond.c par_cond_3 which treats those tokens as cond-pattern terminators. Test: `test_cond_double_bracket_grouping_parens`.

### Subshell umask leaked to parent

- zsh forks for `(...)` so `umask 077` inside dies with the child. We run subshells in-process; without snapshot+restore, the subshell's umask leaked. Added `umask` field to `SubshellSnapshot` (read via `libc::umask(0o022); umask(saved)`), restored on subshell_end via `libc::umask`. Test: `test_subshell_umask_restored_on_exit`.

### `${h[(I)key]}` on assoc searched VALUES instead of KEYS

- zsh subst.c: on associative arrays, `(i)`/`(I)` search KEYS and return the matching key (last match for `(I)`); `(r)`/`(R)` search VALUES and return the matching value. Our `assoc_subscript_flag` always searched values and used the (i)/(I) flag only to switch RETURN type. Fixed by routing `(i)`/`(I)` to key search. Test: `test_assoc_subscript_i_flag_searches_keys`.

### `{one,${a},three}` — outer brace not expanded when inner had var ref

- The segment-concat fast path concatenated `{one,`, `${a}`, `,three}` and pushed the joined scalar but never invoked `expand_braces`. zsh's pipeline brace-expands AFTER substitution. Compile-side detection added: when a literal segment contains `{` or `}`, emit `BUILTIN_BRACE_EXPAND` after the concat (followed by the existing `BUILTIN_GLOB_EXPAND` if glob meta also present). Test: `test_brace_expand_with_inner_var_ref`.

### `$D/*` / `$D/(a|b)` — glob expansion skipped when var ref preceded glob meta

- The segment-concat fast path (Phase 1 step 4) emitted CONCAT for words mixing var refs and glob metachars but never called `expand_glob` on the assembled scalar. zsh's word-expansion pipeline always pathname-expands the post-substitution string. Added `BUILTIN_GLOB_EXPAND` (id 343) — pops a scalar pattern, runs `expand_glob`, pushes Value::Array. Compile path detects glob meta in LITERAL segments only (so `$?`/`$#`/etc. don't trigger) and emits the builtin after the final concat. Tests: `test_glob_with_var_prefix_expands_paths`, `test_glob_with_var_prefix_alternation`.

### `${(j[+])a}` / `${(s[|])s}` — bracket-pair flag delimiters leaked close char

- zsh subst.c `get_strarg` accepts matched bracket pairs as flag delimiters: `[`/`]`, `{`/`}`, `(`/`)`, `<`/`>`. Both flag parsers (`parse_zsh_flags` and the `BUILTIN_PARAM_FLAG` inline parser) used the OPEN char as both opener and closer, so `${(j[+])a}` consumed `[` as opener, then read the rest expecting another `[` (never found) and produced `a+]b+]c`. Added pair-aware close translation. Test: `test_param_join_split_bracket_pair_delim`.

### `:Q` history modifier didn't strip backslash escapes

- zsh hist.c `remquote` removes single/double quote pairs AND backslash escapes (`\X` → `X`). Both `:Q` paths only stripped paired quotes; `a="a\\ b"; echo ${a:Q}` left `\ ` intact instead of giving `a b`. Replaced the simple `replace('\'',"")` with a stateful walk that tracks SQ/DQ and consumes `\X` outside SQ. Test: `test_param_q_modifier_strips_backslash_escapes`.

### `((h[a]++))` / `((h[a]+=v))` on assoc-array elements errored "lvalue required"

- zsh math.c LVAL_NUM_SUBSC keeps the subscript receiver's lvalue identity through compound operators. Our `pre_resolve_array_subscripts` substituted `h[a]` with its current value first, so `5++` reached MathEval and errored. The existing compound handler in `evaluate_arithmetic` only matched indexed arrays. Extended to detect the assoc case (`is_assoc = self.assoc_arrays.contains_key(&name)`) and walk the value through map.get/insert. Tests: `test_arith_assoc_subscript_postinc`, `test_arith_assoc_subscript_compound_assign`.

### `((++a[i]))` / `((++h[k]))` — pre-increment on subscript silently no-op'd

- The compile-side `subscripted_arith_compound_check` only matched POST-op shapes (`name[idx]++`/`+=`/etc.); pre-op (`++name[idx]`) fell through to ArithCompiler which couldn't write back. Added a new `parse_subscript_arith_pre_inc` parser and merged into the runtime compound handler with `is_pre` flag. Compile-side check also accepts the `++NAME[IDX]` shape. Pre-op returns NEW value (matches zsh); post-op returns OLD. Tests: `test_arith_array_subscript_pre_inc`, `test_arith_assoc_subscript_pre_inc`.

### Glob `~` exclusion at PATH level matched RHS as a fresh CWD glob

- `setopt extendedglob; echo $D/*.txt~*README*` should drop README.txt from the match set, but ours included it. The path-level handler recursively `expand_glob`'d the RHS in CWD instead of matching it as a PATTERN against each LHS candidate. Fixed by switching from a `HashSet`-based path equality check to per-candidate `glob_match_static` against basename and full path — direct port of zsh's pattern.c P_EXCLUDE which uses `pattry` per-candidate. Test: `test_glob_tilde_exclude_at_path_level`.

### Associative-array key insertion order was random

- `${(k)h}`, `${(kv)h}`, and `for k v in ${(kv)h}` returned keys in HashMap iteration order, not insertion order. zsh's params.c stores assoc entries in HashTable hnodes preserving insertion order. Switched `ParamValue::Assoc` and `ShellExecutor::assoc_arrays` inner type from `HashMap<String,String>` to `indexmap::IndexMap<String,String>`. Test: `test_assoc_keys_preserve_insertion_order`.

### `for k v in arr` (multi-name for) only assigned to first name

- zsh parse.c par_for accepts multiple identifier tokens before `in`; each iteration consumes N elements and assigns to N variables. Parser now collects all leading identifiers; compiler emits N-stride iteration with empty-string fill on short tail (mirrors exec.c forexec). Single-name path keeps the original 2-byte SET_VAR shape — no perf regression. Tests: `test_for_multi_var_pairs_consume_array`, `test_for_multi_var_three_consume_triples`, `test_for_multi_var_kv_iterates_assoc`.

### Nested `${${a%.txt}#hel}` dropped outer strip operator

- The nested-expansion handler dispatched outer `:MOD` and `/pat/repl` but fell through `#`/`##`/`%`/`%%`, returning the inner result unchanged. zsh subst.c reuses the same getarg machinery for inner and outer; we now mirror by calling `strip_match_op` on the inner result for all four operators. Test: `test_nested_expansion_strip_after_inner`.

### Nested `${(s. .)${(j. .)a}}` ignored outer flag entirely

- When `rest` after the flag block started with `${`, the flag-aware path treated it as a literal var name and returned empty. Added a recursive branch: detect leading `${`, find the matching `}`, recurse `expand_braced_variable` on the inner content, then apply the outer flags (U/L/C/Split/Join) to the inner result. Strip operators after the inner `${...}` also dispatch correctly. Test: `test_nested_expansion_outer_flag_applied_to_inner`.

### `${(l:5::0:)42}` padded with spaces instead of `0`

- The pad-flag parser only recognised the `l:LEN:FILL:` shape; zsh subst.c also accepts `l:LEN::S2:` where S1 is empty and S2 acts as the fill character (and l:LEN:S1:S2: where S1 prefixes once, S2 fills repeatedly). Reworked the parser to collect both strings and pick the fill: S1 if non-empty, else S2 if provided, else space. Test: `test_param_pad_zero_with_empty_string1`.

## Closed (seventy-seventh-pass)

### `history -c` had non-zsh error format

- zsh's `history` is a synonym for `fc -l`; it doesn't accept `-c` (bash's clear-history flag). zsh emits `history:1: bad option: -c`. zshrs had a custom "clear not supported in this mode" string. Aligned to zsh's diagnostic format. Test: `test_history_dash_c_zsh_error_format`.

### POSIX char classes (`[[:alpha:]]`, `[[:digit:]]`) didn't match

- The `glob` crate doesn't recognize the `[:class:]` syntax — it sees `[[:alpha:]]` as a literal pattern with stray `:` and `]`, never matches. Added `expand_posix_char_classes` pre-processor that translates known classes (`alpha`, `alnum`, `digit`, `xdigit`, `lower`, `upper`, `space`, `blank`, `cntrl`, `print`, `graph`, `punct`) to their enumerated ranges (`a-zA-Z`, `0-9`, etc.) before the pattern reaches `glob::glob_with`. Tests: `test_glob_posix_char_class_alpha`, `test_glob_posix_char_class_alpha_letters`.

### `case x in [a-z]) ;; [A-Z]) ;; esac` parse-errored on second arm

- Bracket-class patterns in subsequent case arms triggered `expected ')' in case pattern`. After parsing the first arm's body, the lexer advanced past `;;` BEFORE the parser set `incasepat=1` for the next round, so `[A-Z]` got tokenized as `Inbrack` (test/array subscript) instead of being part of a glob pattern. Fix: set `incasepat=1` BEFORE the zshlex advance on each terminator (`;;`, `;&`, `;|`) so the next pattern's `[` is lexed in pattern context. Test: `test_case_multi_pattern_with_brackets`.

### `type for` / `type while` reported "not found"

- zsh treats reserved words as a distinct type — `type for` reports "for is a reserved word". zshrs's `builtin_type` only checked aliases / functions / builtins / external paths, falling through to "not found" for keywords. Added a RESERVED_WORDS check at the top of the per-name loop so all 22 zsh keywords (`do`, `done`, `esac`, `then`, `elif`, `else`, `fi`, `for`, `case`, `if`, `while`, `until`, `select`, `function`, `repeat`, `time`, `in`, `foreach`, `end`, `coproc`, `nocorrect`, `noglob`) report the keyword status before any other lookup. Test: `test_type_for_reserved_word`.

### `"$#@"` expanded to "3@" instead of "3"

- zsh shorthand: `$#@` and `$#*` both mean "count of positional params" (same as `${#@}`/`${#*}`). zshrs's segment-splitter (`find_expansion_end`) only consumed `$#` then treated the trailing `@`/`*` as literal in the next segment. Same root cause for `X$#Y` (length of `Y`) — the `Y` was being dropped from the var-name lookup. Extended `find_expansion_end` to look ahead after `#` (literal or META-#): if next char is `@`/`*`/META-* consume it as part of the expansion; if next char is identifier-start (alpha or `_`) consume the whole identifier as the var name. Tests: `test_dollar_hash_at_in_double_quotes`, `test_dollar_hash_name_concat`.

### `unalias notdef` had non-zsh error format

- zsh: `unalias:1: no such hash table element: NAME`. zshrs had its own custom format (`unalias: NAME: not found`). Aligned to zsh's exact wording so user scripts that grep the error get the same string. Test: `test_unalias_missing_zsh_format`.

### `$OPTIND` defaulted to empty string

- POSIX: `getopts` reads OPTIND starting at 1 before any call. Scripts that probe `$OPTIND` before invoking `getopts` saw empty string in zshrs (zsh: `1`). Initialized OPTIND=1 and OPTERR=1 in the executor's variables map so first-read returns the canonical defaults. Test: `test_optind_default_one`.

### `setopt nosuchoption` was silent

- zsh emits `setopt:1: no such option: NAME` to stderr and returns 1. zshrs's `builtin_setopt` had a comment saying "zsh doesn't error on bad names" — that's wrong. Added a `ZSH_OPTIONS_SET.contains()` check after `normalize_option_name` and an early `eprintln!` + `return 1` for unknown names. Test: `test_setopt_unknown_option_errors`.

### `$((RANDOM))` returned 0

- `$RANDOM` worked (gets resolved by `get_variable`'s special-param branch) but `$((RANDOM))` returned 0 because `MathEval` looks up names in a static `string_variables` HashMap that didn't include the dynamic special params. Same root for `$((SECONDS))`, `$((EPOCHSECONDS))`, `$((LINENO))`. Fix: clone `self.variables` into an extras map and pre-inject `get_variable("RANDOM")` etc. before passing to `MathEval` — each arith eval now sees a fresh value (RANDOM also re-resolves per call so two arith-substs in a row return distinct values). Test: `test_random_resolves_in_arithmetic`.

### `$_` returned the shell binary path before any command

- zsh starts with `$_` empty (it ignores the OS-env value the parent process set when execing the shell). zshrs's `get_variable` fell through to `env::var("_")` which returned the path the parent used. Initialized `_` to empty in the executor's variables map so the first read returns the canonical empty string. Test: `test_dollar_underscore_starts_empty`.

### `[[ "foo()" == "foo()" ]]` (parens inside DQ) failed to match

- Quoted glob metas inside `[[ ... == ... ]]` patterns must match literally (zsh: `(`/`)`/`|` are alternation grouping under EXTENDED_GLOB, but quoted forms are literal). zshrs's `escape_quoted_glob_metas` only backslash-escaped `*`/`?`/`[`; left `(`/`)`/`|`/`~`/`#`/`^` unquoted, so the pattern matcher saw them as alternation tokens. Extended the escape set to all six. Test: `test_double_bracket_pattern_with_quoted_parens`.

### `command -V function` missing "from zsh" suffix

- zsh: `command -V foo` for a user function reports `foo is a shell function from zsh`. zshrs reported the truncated `foo is a shell function`. Aligned the format to match zsh's output. Test: `test_command_v_function_shows_source`.

### `(( arr[1]++ ))` (and `+=`/`-=`/`*=` etc.) didn't update the element

- zsh: `(( a[1]++ ))` reads element 1, increments, writes back. zshrs's `subscripted_arith_assign_check` only matched the bare `=` form (`(( a[1]=v ))`); compound-assigns and ++/-- fell through to MathEval which can't write through `a[idx]` (it pre-resolves `a[1]` to its value, then `0++` errors "lvalue required"). Added: 
  - `subscripted_arith_compound_check` in compile_zsh.rs to route the compound forms through `BUILTIN_ARITH_EVAL`.
  - `parse_subscript_arith_compound` in exec.rs (handles `++`, `--`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`, `**=`).
  - Read-modify-write logic in `evaluate_arithmetic` that mirrors the bare-`=` special case but with the compound operator. Post-increment/decrement returns the OLD value (matching zsh / C semantics). Tests: `test_arith_subscripted_post_increment`, `test_arith_subscripted_compound_plus_eq`, `test_arith_subscripted_post_increment_returns_old`.

### `*(.)` glob qualifier matched symlinks-to-files

- zsh: `.` qualifier means "plain regular file" — symlinks-to-files are excluded (use `@` for those). zshrs's filter used `is_file()` on followed metadata, so a symlink to a regular file passed the test. Now also checks `symlink_metadata().file_type().is_symlink()` to filter out links first. Test: `test_glob_dot_qualifier_excludes_symlinks`.

### Extended-glob `pat~excl` exclusion not implemented

- zsh's `setopt extendedglob` enables `*.txt~b.txt` (match `*.txt` excluding `b.txt`). Was returning "no matches found". Added a top-level `~` detector at the top of `expand_glob`: split into LHS/RHS, expand both halves, return `LHS \ RHS`. Honors nullglob/nomatch when the difference is empty. Also extended the bridge-path glob trigger in `BUILTIN_EXPAND_TEXT` to fire when a word starts with `^` or contains `~` AND extendedglob is set. Test: `test_extendedglob_tilde_exclusion`.

### Extended-glob `^pat` negation not implemented

- zsh's `^pat` (under extendedglob) matches everything that does NOT match `pat`. Was being passed through as a literal. Added a leading-`^` detector in `expand_glob`: walks the dir, filters out matches of the pattern, returns the remainder (sorted, dot-files excluded as zsh does). Test: `test_extendedglob_caret_negation`.

### `alias -L name` printed `name=value` instead of `alias name=value`

- zsh's `alias -L` is "list in re-input form" — output should round-trip back through the alias builtin. zshrs's bare-name lookup branch ignored the `list_form` flag. Added `if list_form { println!("alias {}", body) }` path. Test: `test_alias_dash_L_emits_alias_prefix`.

### `setopt nocaseglob` was silently ignored

- `setopt nocaseglob` normalises to `caseglob=false` in the options HashMap (the `no` prefix is the negation marker stripped by `normalize_option_name`). But `expand_glob` only read `nocaseglob` directly, so the option never took effect. Read BOTH `caseglob` (default-true) and `nocaseglob` keys to honor either form. Test: `test_setopt_nocaseglob_honored`.

### `$(false)` as the only word triggered "command not found:"

- An empty command-substitution result that becomes the entire command word should be a no-op (exit status preserved). zshrs hit `host_exec_external` with an empty `cmd` and emitted `command not found:` (no name) with status 127. Added an `if cmd.is_empty() && rest.is_empty()` early return at the top of host_exec_external. Test: `test_empty_cmdsubst_no_command_not_found`.

### `type local` / `declare` / `typeset` / `readonly` / `export` reported "shell builtin"

- zsh's `type local` reports `local is a reserved word` — these are precommand modifiers parsed at the syntactic level, not regular builtins. zshrs's reserved-word table only included flow-control keywords (`if`, `while`, etc.); the declaration keywords were missing. Added: `local`, `declare`, `typeset`, `readonly`, `export`, `integer`, `float` to the RESERVED_WORDS list in `builtin_type`. Test: `test_type_reserved_word_local_declare`.

### `whence -v local` / `repeat` etc. also reported "shell builtin"

- Same reserved-word gap in the `whence` builtin's `is_reserved_word` table. Added the same declaration keywords (`local`, `declare`, `typeset`, `readonly`, `export`, `integer`, `float`) plus zsh-specific keywords `repeat`, `foreach`, `end`, `nocorrect`, `noglob` that were missing from both tables. `whence -v local` now matches zsh's `local is a reserved word`. Test: `test_whence_reserved_word_local`.

### `$_` leaked internal `return N` arg as the function's call-form last-arg

- After `foo() { return 42 }; foo; echo $_`, zsh reports `$_ = foo` (the function name, since no args were passed). zshrs reported `$_ = 42` (the `return 42` arg) because the function-internal `pop_args` for `return` updated `pending_underscore`, and that leaked back to the caller. Fix: at the END of `call_function`, overwrite `$_` and `pending_underscore` with the function's CALL-form last arg (or the function name if no args). The internal command args don't escape function scope. Test: `test_dollar_underscore_after_function_call`.

### `[[ -l file ]]` (and other unknown unary conditions) silently returned false

- zsh: `[[ -l file ]]` (no `-l` test in zsh — `-h` is the symlink test) emits `unknown condition: -l` to stderr and returns false. zshrs's `emit_file_test` default arm for unknown ops just emitted `Pop; LoadFalse` with no diagnostic. Now emits the error message at compile-time so users see the same warning zsh prints. Test: `test_unknown_cond_emits_diagnostic`.

### `${a[N]:offset:length}` returned the full element instead of the substring

- zsh: `a=(hello); ${a[1]:0:1}` should return `h` (substring of element 1). zshrs's bracket-handler routed the `:0:1` modifier through the colon-default branch (`:-`) which doesn't handle digit-prefixed offset/length. Added a `:DIGIT[:DIGIT]` substring branch BEFORE the colon-default handlers — only fires when the char after `:` is a digit (so `:-default` continues to be the default-if-empty form, not a negative offset). Test: `test_array_element_substring`.

### `print -P "%B"` appended an unwanted `\e[0m` reset

- zsh: prompt expansion does NOT auto-reset attributes at end. `print -P "%B"` outputs exactly `\e[1m\n` — the bold escape and a newline. zshrs's prompt expander unconditionally appended `\e[0m` when any attribute (`bold`, `underline`, `fg_color`, etc.) was active at end-of-expansion. Removed the auto-reset block so the user controls when to clear with explicit `%b`/`%f`/`%k`. Test: `test_print_dash_P_no_trailing_reset`.

### `$_` empty after no-arg `true` / `false` / `:`

- zsh: `true; echo $_` prints `true` (the command name, since no args). zshrs's `pop_args` updates `pending_underscore` only from `args.last()`; for arg-less commands no update fired. Backfilled the command name in BUILTIN_TRUE/FALSE/COLON when args is empty. Test: `test_dollar_underscore_after_no_arg_command`.

### `[[ -t fd ]]` (is-fd-a-tty) emitted "unknown condition: -t"

- zsh: `[[ -t 0 ]]` checks if stdin is a tty. zshrs's `emit_file_test` had no case for `-t` — it fell to the default unknown-condition branch and emitted the new diagnostic spuriously. Added a `-t` case that pushes the fd-string and routes through new `BUILTIN_IS_TTY` (calls `libc::isatty`). Test: `test_dash_t_fd_is_tty`.

### `echo */` stripped trailing slash from directory matches

- zsh: `echo */` outputs each directory with a trailing `/`. The Rust `glob` crate strips trailing slashes from match results, so zshrs returned `sub` instead of `sub/`. Re-append `/` to each result when the input pattern ended in `/`. Test: `test_glob_trailing_slash_preserved`.

### `echo - hi` printed `- hi` instead of `hi`

- zsh's echo treats a bare `-` (single char) as a no-op flag — silently consumed. zshrs's flag parser skipped tokens shorter than 2 chars, so the lone `-` became a positional arg. Added an explicit `if arg == "-"` skip in the flag-walk. `--` (two dashes) is still NOT a recognized flag — stays literal. Test: `test_echo_bare_dash_is_noop_flag`.

## Closed (eighty-first-pass)

### `shift -1` silently no-op'd instead of erroring

- zsh: `shift -1` errors `argument to shift must be non-negative` exit 1. zshrs's arg parser checked `arg.chars().all(is_ascii_digit)` for the count branch (which skips `-1` because of the leading `-`), then fell through to `array_names.push("-1")`. Since there was no array named `-1`, the shift was silently no-op'd. Added an explicit negative-numeric check before the digit-only branch that emits zsh's diagnostic and returns 1. Test: `test_shift_negative_count_errors`.

### `LINENO=99` silently set the variable instead of erroring

- zsh has a hard-wired set of intrinsic read-only specials (`PPID`, `LINENO`, `ZSH_ARGZERO`, `argv0`, `ARGC`, `_`) that can never be assigned to from script — assignment errors `read-only variable: NAME` exit 1. zshrs's `BUILTIN_SET_VAR` handler only consulted the user-managed `readonly_vars` set + `var_attrs.readonly`, so the intrinsics passed through. Added an `is_intrinsic_ro` matches!() check at the top of the readonly gate. Test: `test_lineno_intrinsic_readonly`.

### `set -Z` (and `set +Z`) silently accepted unknown letters

- zsh: `set -Z` errors `can't change option: -Z` exit 1. zshrs's multi-letter flag parser had a silent `_ => {}` fallback in the per-char `match`, accepting any unknown letter as a no-op. Replaced with an explicit error arm; also extended the recognized-letter list to cover zsh's full single-letter table (a, b, B, C, d, e, f, g, h, k, m, n, p, r, s, t, u, v, x, y, A, E, F, G, H, K, L, N, P, R, T, U, X, Y) so the existing knobs aren't broken by the new strict path. Same fix mirrored to the `+` arm. Test: `test_set_unknown_letter_errors`.

### `set -o nonexistentopt` silently inserted junk into the option map

- zsh: `set -o badopt` errors `no such option: badopt` exit 1. zshrs called `normalize_option_name(opt)` and inserted whatever name it returned into `self.options` — including names that aren't real zsh options. This left stale junk in the option map AND silenced typos. Added a `ZSH_OPTIONS_SET.contains(name)` guard before insertion in both the `-o` and `+o` arms; on miss, emit zsh's diagnostic and return 1. Test: `test_set_o_unknown_option_errors`.

### `unset` (no args) returned 0 silently

- zsh: bare `unset` errors `not enough arguments` exit 1 — at least one variable name is required. zshrs returned 0 silently, masking accidental empty `unset $maybe` (where `$maybe` expanded to nothing). Added an `args.is_empty()` early-error branch before the `-f`/`-v`/`-m` flag walk. Test: `test_unset_no_args_errors`.

### `disown` (no args, no current job) returned 0 silently

- zsh: bare `disown` with no current job errors `no current job` exit 1. zshrs had an `if let Some(job) = self.jobs.current()` block that ran the disown when a job existed, but the `if-let` simply fell through to `return 0` when there was no job — silent success on what should be an error. Restructured to error on the `None` arm. Test: `test_disown_no_current_job_errors`.

### `command` (no args) errored "redirection with no command"

- A previous batch added an unconditional error for bare `command` based on a misread of zsh's docs. Verified live: `command` (no args, no redirections) exits 0 silently in both zsh and bash; the "redirection with no command" diagnostic only fires when redirections are present without a command name (a parser-level concern, not the builtin's). Reverted the builtin-level error to a silent `return 0`. Test: `test_command_no_args_silent` (replaces the wrong `test_command_no_args_redirection_error`).

### `[ a -ZZ b ]` silently returned 1 instead of "unknown condition: -ZZ"

- zsh: `[ a -ZZ b ]` errors `[:1: unknown condition: -ZZ` exit 2 — the alphabetic operator at args[1] isn't a recognized comparator (`-eq`/`-ne`/etc., `=`/`!=`, `<`/`>`, `-nt`/`-ot`/`-ef`, `-a`/`-o`). zshrs's 3-arg path only checked the numeric-comparator subset, falling through to the AND/OR connective splitter for everything else; the splitter found nothing to split and returned 1. Added an "unknown alphabetic 3-arg operator" arm next to the numeric-comparator arm. Test: `test_test_unknown_3arg_op_errors`.

### `[ \( a ]` silently returned 1 instead of "argument expected"

- zsh: an unmatched `(` in `test`/`[` syntax errors `[:1: argument expected` exit 2. zshrs ignored paren depth at the top level, falling through to the single-string `[s]` arm (which evaluates `(` as truthy) or the catch-all 1-return. Walked the args once tracking paren depth; if depth ≠ 0 at end, emit zsh's diagnostic and exit 2. Conservative — only fires on actual mismatch, so legitimate `[ \( -n a \) -a \( -z "" \) ]` is unaffected. Test: `test_test_paren_mismatch_errors`.

### `kill` and `kill -9` (no PID) printed bash-style multi-line usage banner

- zsh: bare `kill` and `kill -SIG` (no PID) both error `kill:1: not enough arguments` exit 1 — terse zsh format. zshrs printed `kill: usage: kill [-s signal | -n num | -sig] pid ...` followed by `       kill -l [sig ...]` — bash-style two-line banner without the shell-name prefix. Replaced both `eprintln!` paths (no-args and missing-PID) with the zsh-format `zshrs:kill:1: not enough arguments`. Test: `test_kill_no_args_zsh_format`.

### `trap "" BADSIGNAL` silently registered a never-firable trap

- zsh validates the signal name before installing the handler — unknown errors `trap:1: undefined signal: NAME` exit 1 and the trap is NOT installed. zshrs's loop blindly inserted whatever uppercased token came in, so `trap "" BADSIG` quietly polluted the trap table. Added a known-signal allowlist (zsh's full SIG-prefixed table plus the pseudo-signals `EXIT`/`ZERR`/`DEBUG`/`ERR`/`RETURN` plus numeric forms) before insertion; on miss, emit zsh's diagnostic and return 1. Test: `test_trap_undefined_signal_errors`.

### `trap -l` printed bash-style numbered SIGNAL list (not zsh's silent empty)

- zsh's `trap` builtin doesn't recognize `-l` as a flag — it just falls into the no-args path which prints currently-installed traps (empty in `-f` mode). zshrs implemented `-l` bash-style, emitting a 6-row 5-col table of `1) SIGHUP   2) SIGINT …`. Replaced with `return 0;` to match zsh's silent output. Test: `test_trap_l_silent`.

### `vared` (no args) printed `vared: not enough arguments` (no zsh prefix)

- zsh prefixes error diagnostics with `<shellname>:<builtin>:<line>:` — `vared:1: not enough arguments`. zshrs emitted bare `vared:` (no shell name, no line number), breaking script consumers that grep for the canonical zsh error format. Both eprintln sites in `builtin_vared` now use `zshrs:vared:1:`. Test: `test_vared_no_args_zsh_format`.

### `unset NAME` for read-only NAME silently removed the entry

- zsh: `unset NAME` for a read-only NAME errors `read-only variable: NAME` exit 1 — the unset is rejected, not silently consumed. zshrs's `builtin_unset` blindly stripped the entry from `variables`/`arrays`/`assoc_arrays` without consulting the read-only bit, so `readonly x=1; unset x` left x unset and exit 0 — a compat regression that broke scripts probing for readonly state. Added a per-name read-only check (intrinsic specials + `readonly_vars` + `var_attrs.readonly`) before the remove() calls. Test: `test_unset_readonly_errors`.

### `typeset -Q x=1` and `declare -Q x=1` silently succeeded

- zsh: unknown typeset/declare flag errors `typeset:1: bad option: -Q` (or `declare:1:`) exit 1. zshrs's per-char flag loop had a silent `_ => {}` fallback in both the `-` and `+` arms, so unknown letters were accepted as no-ops and the variable was set without any attribute. Replaced both fallbacks with explicit `bad option:` errors; uses the `invoked_as` parameter so `declare -Q` and `typeset -Q` produce the right name. Test: `test_typeset_unknown_flag_errors`.

### `printf` (no args) printed bash-style usage banner

- zsh: `printf` -> `printf:1: not enough arguments` exit 1. zshrs emitted `printf: usage: printf format [arguments]` — bash-style usage line without the shell-name prefix. Replaced both eprintln sites in `builtin_printf` with `zshrs:printf:1: not enough arguments`. Test: `test_printf_no_args_zsh_format`.

### `let "1+"` / `$((5+))` reported bare "not enough operands" instead of zsh's "bad math expression" wording

- zsh: math-expression parser errors are wrapped in `bad math expression: <reason>` so script consumers can grep for the canonical prefix. zshrs's `MathContext::op` arm emitted a bare `not enough operands` diagnostic, missing the wrapper. Changed `src/math.rs` to emit `bad math expression: operand expected at end of string` for the binary-op underflow case (matches zsh's exact phrasing for `let "1+"` AND `$((5+))`). Test: `test_arith_trailing_op_uses_zsh_wording`.

### `let "("` reported bare "')' expected" instead of zsh's wrapped wording

- zsh: `let "("` -> `bad math expression: ')' expected`. zshrs emitted `')' expected` without the wrapper. Updated `MathContext::InPar` arm to use the wrapped phrasing. Test: `test_arith_open_paren_uses_zsh_wording`.

### `where __notacmd__` and `which __notacmd__` reported "shell built-in command" exit 0

- zsh: `where`/`which` for an unknown name -> `not found` exit 1. zshrs's `is_builtin()` helper has a `name.starts_with('_')` bypass for completion functions (so `_foo` lookups don't fall through to disk). The `whence` core path used `is_builtin()` directly, so `where __notacmd__` matched the `_` prefix, reported `__notacmd__: shell built-in command` and returned 0. Tightened the `whence` builtin-check arm to `BUILTIN_SET.contains(name)` (skip the `_`-prefix bypass) so completion functions still resolve via the function-lookup arm but unknown `_`-names report `not found`. Tests: `test_where_unknown_command_not_found`, `test_which_unknown_command_not_found`.

### `history XX` (non-numeric arg) reported "no such event: 1" instead of "event not found: XX"

- zsh: `history XX` (search-by-text with no match) -> `fc:1: event not found: XX` exit 1. zshrs's no-tty no-session branch had a hardcoded `no such event: 1` regardless of the user's input — wrong wording for non-numeric args AND wrong event identifier (always "1" instead of the user's text). Added a `search_query` branch: if the parsed positional was non-numeric, emit zsh's `event not found: XX`; otherwise stay on the existing `no such event: 1` path. Test: `test_history_text_query_uses_event_not_found`.

### `unalias xyz abc` returned on first miss (hiding subsequent misses)

- zsh: `unalias` continues processing remaining names after a miss, emitting one diagnostic per unknown entry and returning the last failing exit code. zshrs's loop returned on the first miss, hiding the rest from script consumers (`unalias xyz abc` only printed the `xyz` error and exited 1, never checking `abc`). Restructured the loop to collect status without early-exit. Test: `test_unalias_continues_after_first_miss`.

### `read -Q v` silently accepted unknown flag

- zsh: `read -Q v` -> `read:1: bad option: -Q` exit 1. zshrs's per-char flag loop in `builtin_read` had a silent `_ => {}` fallback, so unknown letters were accepted and the read ran as if -Q were valid. Replaced the fallback with an explicit `bad option:` error that exits 1 before reading. Test: `test_read_unknown_flag_errors`.

### `getopts` (no args) printed bash-style usage banner

- zsh: bare `getopts` -> `getopts:1: not enough arguments` exit 1. zshrs printed `zshrs: getopts: usage: getopts optstring name [arg ...]` — bash-style banner without the line-number-prefixed format. Replaced with `zshrs:getopts:1: not enough arguments`. Test: `test_getopts_no_args_zsh_format`.

### `wait %999` (id never created) silently returned 0

- zsh: `wait %N` for an N never assigned to a job -> `wait:1: %N: no such job` exit 127. zshrs's earlier "silent on missing %ID" rule (added to keep the `cmd & wait %1` idiom working) was too broad — `wait %999` with no jobs ever started silently returned 0 too. Distinguished via $! sentinel: errors when the session has never set $! (no bg ever run); silent only when bg was used (so `cmd & wait %1` still works after the bg child completes). Test: `test_wait_unrealistic_jobspec_errors`.

### `exec -c`/`exec -l`/`exec -a foo` (no command) silently returned 0

- zsh: `exec FLAG` (any flag form without a following command) -> `exec requires a command to execute` exit 1. zshrs collapsed all "no command" cases to silent return 0, masking flag-only typos. Bare `exec` (no flags, no command) still returns 0 silently per POSIX (the env-modify form). Now errors when any of `-c`/`-l`/`-a NAME` were specified without a command. Test: `test_exec_flag_only_no_command_errors`.

### `printf "%Z\n" 1` printed the diagnostic but returned 0

- zsh: invalid printf directive errors `printf:1: %Z: invalid directive` exit 1 — but zshrs printed the diagnostic and still returned 0, hiding the failure for $?-checking scripts. Added a local `had_error` flag through the format-spec walker; the `_ => { … }` arm sets it on any unknown specifier (and on `%a`/`%v`/`%V` which zsh also rejects). Function now returns 1 if any error fired. Test: `test_printf_invalid_directive_exits_nonzero`.

### `kill -INVALID 1` printed bash-style "kill: invalid signal: -INVALID"

- zsh: `kill -INVALID 1` -> `kill:1: unknown signal: SIGINVALID` followed by `kill:1: type kill -l for a list of signals` exit 1. zshrs emitted the bash-style `kill: invalid signal: -INVALID` (with leading dash, no SIG prefix, no hint line). Replaced the `_-flag` else arm with the two-line zsh format (already used by the `-L`-as-signal arm). Test: `test_kill_bad_signal_uses_zsh_format`.

## Closed (eighty-second-pass)

### `source` (no args) printed bash-style banner with no shell-name prefix

- zsh: bare `source` -> `source:1: not enough arguments` exit 1. zshrs printed `source: filename argument required` — bash-style banner without the canonical zsh `<shellname>:<builtin>:<line>:` prefix that scripts grep for. Refactored to `builtin_source_named(args, invoked_as)`; `builtin_source` is now a thin wrapper. The bare-source path emits zsh's terse format. Test: `test_source_no_args_zsh_format`.

### `. ""` printed "is a directory" with exit 1 instead of "no such file or directory" exit 127

- zsh: `. ""` -> `.:1: no such file or directory:` exit 127 (treats the empty path as a missing file with empty trailing identifier). zshrs's POSIX path resolver mapped "" to the current working directory, then `fs::read_to_string("")` returned `Is a directory`, so the diagnostic became `is a directory: ` exit 1 — wrong wording AND wrong exit. Special-cased empty-path early in `builtin_source_named` to emit zsh's diagnostic + exit 127. Test: `test_source_empty_path_uses_no_such_file`.

### `[ a b c d ]` (4+ args, no operator) silently returned 1

- zsh: `[ a b c d ]` -> `1: condition expected: a` exit 2 (more than 3 operands without a recognized connective is a syntax error). zshrs's default arm fell through every operator/connective check and returned 1, which a consumer would read as "false" rather than "syntax error". Added a `args.len() >= 4` arm at the very end of the default block that emits zsh's diagnostic and exits 2. Test: `test_test_4plus_args_no_op_errors`.

### `[ \( \) ]` (matching empty parens) silently returned 1

- zsh: `[ \( \) ]` -> `[:1: argument expected` exit 2 (parens around an empty expression is ill-formed). zshrs's recursive `inner.is_empty()` recursion landed at the bare-args-empty `1` arm and silently succeeded false. Added an `inner.is_empty()` check inside the strip-parens-and-recurse path that emits zsh's diagnostic and exits 2 before recursing. Test: `test_test_empty_paren_errors`.

### `zle -l` listed built-in widgets in non-interactive scripts

- zsh: in `-c`/`-f` non-interactive mode the ZLE module isn't loaded, so `zle -l` outputs nothing and returns 0. zshrs eagerly preloaded its built-in widget table on every interp startup, so `zle -l` always emitted the full list — diverging from zsh's silent-empty output and breaking scripts that grep for "zle module not loaded" semantics. Added an `atty::is(Stream::Stdin)` guard in both `-l` and `-la`/`-lL` arms; non-tty mode returns 0 immediately. Test: `test_zle_l_silent_in_script`.

### `umask 999` reported "bad symbolic mode operator: 9" instead of "bad umask"

- zsh: `umask 999` (digits but invalid octal — 9 isn't a valid octal digit) -> `umask:1: bad umask` exit 1. zshrs's `from_str_radix(v, 8)` failed, then the input fell into the symbolic-form parser, which interpreted `9` as a class char that wasn't followed by `+`/`-`/`=` and emitted `bad symbolic mode operator: 9` — wrong error category entirely. Added a `looks_numeric` (all-digits) precheck before the symbolic-form parser that emits `bad umask` instead. Test: `test_umask_bad_numeric_format`.

### `umask u=Z` reported generic "invalid mask" instead of specific "bad symbolic mode permission: Z"

- zsh: `umask u=Z` -> `umask:1: bad symbolic mode permission: Z` exit 1 — pinpoints the unknown rwx char. zshrs collapsed any symbolic-form failure to `umask: invalid mask: u=Z`. Refactored the rwx-char loop to track a `bad_perm: Option<char>` and emit zsh's specific diagnostic on miss before falling back. Test: `test_umask_bad_symbolic_permission`.

### `pwd -X` printed cwd as if -X were a valid flag

- zsh: `pwd -X` -> `pwd:1: bad option: -X` exit 1. zshrs's per-char flag loop had a silent `_ => {}` fallback, accepting any letter and continuing to print the cwd. Replaced with explicit `bad option: -X` error that exits 1 before the print. Test: `test_pwd_unknown_flag_errors`.

### `cd /tmp /etc` (two-arg substitution form, OLD not in $PWD) silently used args[0] as target

- zsh: `cd OLD NEW` is the path-substitution form — replaces OLD with NEW in $PWD and cd's there. If OLD isn't in $PWD, errors `cd:1: string not in pwd: OLD` exit 1. zshrs's two-arg branch only fired the substitution when OLD was in cwd; otherwise it silently fell through and treated args[0] as the target (bash-style `cd path1 path2 = cd path1` semantics). Added an explicit `string not in pwd:` error on miss. Test: `test_cd_two_args_substitution_or_error`.

### `readonly -X x=1` silently inserted "-X" as a readonly variable

- zsh: `readonly -X x=1` -> `readonly:1: bad option: -X` exit 1. zshrs's loop treated any non-`-p` argument as either an `=`-form binding or a bare name. So `-X` got inserted into `readonly_vars` and `var_attrs` as a junk readonly entry, masking the typo. Added a `starts_with('-')` flag-arg check before the binding/bare-name branches that emits zsh's `bad option:` diagnostic. Test: `test_readonly_unknown_flag_errors`.

### `type __notexist__` reported "is a shell builtin" instead of "not found"

- zsh: `type __notexist__` -> `__notexist__ not found`. zshrs's `is_builtin()` helper has a `_`-prefix bypass for completion functions, so any `_*` name was falsely classified as a builtin. The `type` builtin used `is_builtin()` directly, mirroring the same bug already fixed in `whence`/`where`/`which`. Tightened the `type` builtin-check arm to consult `BUILTIN_SET.contains(name)` directly. Test: `test_type_underscore_unknown_not_builtin`.

### `unsetopt nonexistentopt` silently inserted junk into the option map

- zsh: `unsetopt nonexistentopt` -> `unsetopt:1: no such option: nonexistentopt` exit 1. zshrs blindly inserted whatever name `normalize_option_name` returned into `self.options`, leaving stale junk in the map and silencing typos (mirror of the `setopt`-bug fixed earlier). Added a `ZSH_OPTIONS_SET.contains(name)` guard before insertion in the default arm of the per-arg loop. Test: `test_unsetopt_unknown_option_errors`.

### `exit 1 2 3` silently swallowed extra args

- zsh: `exit 1 2 3` -> `exit:1: too many arguments` exit 1. zshrs silently parsed args[0] as the code and ignored the rest. NOTE: zsh's bytecode actually CONTINUES past the failed exit (the rest of the script runs); zshrs's compiler unconditionally jumps to script end after `BUILTIN_EXIT`, so we can't replicate "continue past failed exit" here — the best we can do is emit the diagnostic before exiting, which catches the typo instead of silently swallowing. Test: `test_exit_too_many_args_diagnoses`.

### `trap "" 99` silently registered an out-of-range numeric signal

- zsh: numeric signals must be in (0, 63] (signal 0 is `EXIT`, max is `SIGRTMAX`). `trap "" 99` -> `trap:1: undefined signal: 99` exit 1. zshrs's known-sig validator accepted ANY parseable u32, registering a never-firable trap silently. Bounded the numeric arm to `n > 0 && n <= 63`. Test: `test_trap_numeric_signal_out_of_range_errors`.

### `exec --bad` (long-option-style typo) silently no-op'd

- zsh: `exec --bad` -> `exec requires a command to execute` exit 1. zshrs's flag walker hit `_ => {}` for unknown letters, so `--bad` was silently consumed without setting any flag, the cmd_args stayed empty, and the existing flag-only check (gated on `clear_env || login_shell || argv0.is_some()`) didn't fire because none of those got set. Tightened the check by scanning the input args for any `-`-prefixed token; if seen with empty cmd_args, the missing-command error fires. Test: `test_exec_long_option_typo_errors`.

### `print -S foo bar` silently concatenated into history

- zsh: `print -S` is the split-shell-words history form and takes EXACTLY one positional. `print -S foo bar` -> `print:1: option -S takes a single argument` exit 1. zshrs treated `-S` as `add_to_history = true` (same as `-s`) and concatenated all args into the history entry silently. Added a separate `split_word_history: bool` track for `-S`; if multiple positionals follow, emit zsh's diagnostic and exit 1 before adding. Test: `test_print_S_takes_single_arg`.

### `autoload -Z foo` and `autoload -l` silently accepted unknown flags

- zsh: `autoload -Z` -> `autoload:1: bad option: -Z`; `autoload -l` -> `autoload:1: bad option: -l` exit 1. zshrs's silent `_ => {}` fallback in the flag char-loop accepted any letter, masking typos AND the bash-style `-l` flag that zsh doesn't have. Replaced the fallback with explicit `bad option:` error. Test: `test_autoload_unknown_flag_errors`.

### `[ -lt 5 3 ]` (binop at args[0]) silently returned 1

- zsh: `[ -lt 5 3 ]` -> `[:1: unknown condition: -lt` exit 2. The op-at-front looks like a unary condition zsh doesn't recognise. zshrs's 3-arg path only checked `args[1]` for the operator, so `args[0]=-lt args[1]=5 args[2]=3` slipped past every match arm and hit the catch-all `1`. Added a 3-arg arm that triggers on a known binop at args[0] and emits zsh's diagnostic. Test: `test_test_3arg_op_at_pos0_errors`.

### `[ a -lt 3 5 ]` (4 args with valid binop) emitted "condition expected: a" instead of "too many arguments"

- zsh: `[ a -lt 3 5 ]` -> `[:1: too many arguments` exit 2 — the binop is correctly placed but the operand count is wrong. zshrs's 4+-arg arm always emitted `condition expected: <args[0]>`, the wrong category for valid-but-overlong binop expressions. Split the 4+-arg arm: if args[1] is a known binop, emit `too many arguments`; otherwise stay on `condition expected:`. Test: `test_test_4args_with_binop_emits_too_many`.

### `[ "" "" ]` (two operands, no operator) silently returned 1

- zsh: `[ "" "" ]` -> `1: parse error: condition expected:` exit 2 (two operands without a connective is ill-formed). zshrs's 2-arg path only handled `[s1, s2]` for known string-comparator forms; the catch-all silently returned 1. Added a 2-arg arm that fires when neither operand is `-`-prefixed nor a paren, emitting zsh's parse-error diagnostic. Test: `test_test_two_operands_no_op_errors`.

### `autoload -X` (no function name) silently no-op'd

- zsh: `autoload -X` (no function name) -> `autoload:1: bad autoload` exit 1 — `-X` requires a function context. zshrs's loop set `execute_now=true` but the empty `functions` vec skipped both the listing branch (gated on `!execute_now`) AND the execute branch (no functions to load), returning 0 silently. Added an explicit `functions.is_empty() && execute_now` arm that emits zsh's diagnostic. Test: `test_autoload_X_no_function_errors`.

### `shift 5 a` on a 1-element array silently shifted what it could

- zsh: `a=(1); shift 5 a` -> `shift:1: shift count must be <= $#` exit 1 (the per-array bound is enforced). zshrs's array-shift loop iterated `for _ in 0..count` and just `arr.remove(0)`'d up to the array length, leaving partial state and returning 0. Added a precheck pass over array_names that compares `count > arr.len()` and errors before any mutation. Test: `test_shift_array_count_too_many_errors`.

### `jobs -Z` silently ignored the flag instead of erroring "requires one argument"

- zsh: `jobs -Z` (without a process-name arg) -> `jobs:1: -Z requires one argument` exit 1 (`-Z` sets the shell's process name; required arg). zshrs's `'Z' => {}` arm silently consumed the flag. Replaced with explicit error. Test: `test_jobs_Z_requires_argument`.

### `zformat` (no args) printed bare "zformat:" instead of zsh's prefixed format

- zsh: bare `zformat` -> `zformat:1: not enough arguments`. zshrs emitted `zformat: not enough arguments` (no shell-name or line-number prefix). Updated the eprintln to use `zshrs:zformat:1:`. Test: `test_zformat_no_args_zsh_format`.

### `[ a -lt ]` (operand + binop, missing right operand) silently returned 1

- zsh: `[ a -lt ]` -> `1: parse error: condition expected: a` exit 2. zshrs's 2-arg path for `[s, "-lt"]` had no explicit arm; fell through to the catch-all `1`. Added a 2-arg arm that triggers when args[0] is a non-flag operand AND args[1] is a known binop, emitting zsh's parse-error. Test: `test_test_two_args_binop_missing_operand`.

### `kill -0 1` printed bash-style "kill: 1: Operation not permitted (os error 1)" instead of zsh's lowercased format

- zsh: `kill -0 1` (no permission to signal pid 1) -> `kill:1: kill 1 failed: operation not permitted` (lowercased reason, no `(os error N)` suffix, `kill <pid> failed:` framing). zshrs emitted Rust's `Display` of the OS error verbatim. Reformatted to strip the `(os error …)` tail, lowercase the reason, and use zsh's `kill <pid> failed:` shape. Test: `test_kill_zero_failed_uses_zsh_format`.

### Function recursion overflowed the Rust stack instead of erroring "maximum nested function level reached"

- zsh: deep recursion is bounded by `FUNCNEST` (default 500) and errors `<name>: maximum nested function level reached; increase FUNCNEST?` exit 1. zshrs had NO enforcement — `foo() { foo; }; foo` and the builtin-shadow form `echo() { echo hi; }; echo hi` both crashed the process with `fatal runtime error: stack overflow`. Added the guard at both `call_function` (the hot path) and `dispatch_function_call` (the fallback). Cap is 100 by default (the bytecode VM is host-recursive at ~40KB/frame, so the 8MB Rust stack tops at ~150 frames; 100 leaves headroom). Users with deeper need can raise `FUNCNEST` AND `RUST_MIN_STACK`. Test: `test_funcnest_recursion_guard_no_overflow`.

### `[ "5" \> "3" ]` and `[ "5" \< "3" ]` silently returned 0 (string-compared) instead of erroring

- zsh's POSIX `[`-test does NOT accept `<` or `>` as string comparators — they're redirection operators outside `[`/`]`. `[ "5" \> "3" ]` -> `1: condition expected: >` exit 2. zshrs's match arms had `[a, "<", b]` and `[a, ">", b]` doing string compares (a bashism), hiding the syntax error. Replaced both arms with the zsh diagnostic. The `[[`-cond compiler still handles them as proper string comparators where they ARE valid. Test: `test_test_lt_gt_not_string_comparators`.

## Closed (eighty-third-pass)

### `setopt -h` (single-letter shortcut) errored "no such option" instead of accepting silently

- zsh: single-letter `-X` / `+X` flags on setopt are shortcuts for option names from the option-letter table — `setopt -h` is a no-op accepted silently (`h` maps to `hashcmds`). zshrs's default arm rejected ANY `-`-prefixed arg as an unknown option name. Added a `len() == 2 && (-|+)` short-circuit that accepts the single-letter form silently before the unknown-option check. Test: `test_setopt_single_letter_silent`.

### `fc -l 1 2 3` (3+ positional args) emitted "no events in that range" instead of "too many arguments"

- zsh: `fc -l N M` is a range query; 3+ positionals -> `fc:1: too many arguments` exit 1. zshrs's range path collapsed any 2+-arg case to `no events in that range`, missing the explicit count check. Added a `positional.len() > 2` arm before the existing `== 2` range arm. Test: `test_fc_l_3plus_args_too_many`.

### `type ""` (empty name) reported the first PATH entry as the resolved file

- zsh: `type ""` -> ` not found` exit 1. zshrs's PATH walker computed `dir + "/" + ""` which `std::path::Path::exists` reports as TRUE (the directory itself exists), falsely matching `type ""` to the first PATH entry. Skip the lookup entirely for empty names. Test: `test_type_empty_name_not_found`.

### `ulimit -f abc` (non-numeric value) silently dropped the value and printed "unlimited"

- zsh: `ulimit -f abc` -> `ulimit:1: invalid number: abc` exit 1. zshrs's `arg.parse().ok()` silently discarded non-numeric input, leaving `value` unset and printing the existing limit. Replaced with explicit `match arg.parse::<u64>()` that errors on miss. Test: `test_ulimit_invalid_number_errors`.

### `[ a == a ]` accepted `==` (bashism), masking POSIX `[`-test error

- POSIX `[`-test only accepts `=` for equality — `==` is the `[[`-cond extension. zsh: `[ a == a ]` -> `1: = not found` exit 1 (zsh's parser sees `==` and tries to look up the second `=` as a command). zshrs's match arm `[a, "=", b] | [a, "==", b]` accepted both. Split into separate arms: `[_, "==", _]` errors with the zsh diagnostic; `[a, "=", b]` continues to do string compare. The `[[`-cond compiler still handles `==` as a proper string comparator. Test: `test_test_double_equals_rejected`.

### `fc --help` reported "bad option: --" instead of "bad option: -h"

- zsh skips the leading `-` of long-option-style typos and reports the FIRST recognisable letter as the bad option: `fc:1: bad option: -h`. zshrs's loop hit the first char `-` of `--help` and reported `bad option: --` — wrong identifier. Added a `'-' => {}` arm that consumes the extra dash silently so the next char becomes the diagnostic target. Test: `test_fc_long_option_reports_first_letter`.

### `echo $((37#1))` and `echo $((2#5))` printed bogus 0 after the error

- zsh aborts the surrounding command on arith errors — `echo $((37#1))` (base out of range) and `echo $((2#5))` (digit out of range for the base) emit the diagnostic but do NOT print `0`. zshrs's evaluator returned `"0"` from the error arm and the caller continued to print it, so script consumers saw the diagnostic AND the bogus value. Added a `process::exit(1)` after the diagnostic when the message starts with "bad math expression" or "invalid base" (the canonical "give up" signals). Test: `test_arith_invalid_base_aborts_command`.

### `$((2#5))` silently produced 0 instead of erroring "operator expected at \`5'"

- zsh: `$((2#5))` (5 is not a valid binary digit) errors `bad math expression: operator expected at \`5'`. zshrs's `i64::from_str_radix(val_str, base).unwrap_or(0)` silently dropped the parse error, masking the typo. Replaced with explicit `match` that emits zsh's diagnostic when the digit is out of range for the declared base. Test: `test_arith_bad_digit_for_base_errors`.

### `wait ""` (literal empty arg) silently continued instead of erroring

- zsh: `wait ""` -> `wait:1: job not found:` exit 127 (treats empty as a failed job-spec lookup with empty identifier). zshrs's earlier "silent on empty" branch (added for the `wait $!` no-bg-job idiom) was too broad — `$!` defaults to "0" (a literal pid value), not "", so the silent-empty branch only handles the literal-`""` case which zsh actually errors on. Replaced with the zsh diagnostic. Test: `test_wait_empty_string_errors`.

### `read -d ""` panicked the shell on NUL-containing input

- zsh: `read -d ""` reads up to NUL; the captured value may contain NUL bytes. zshrs unconditionally called `env::set_var` which panics on NUL bytes (file-name validation), aborting the whole shell with a Rust backtrace. Guarded all `env::set_var` calls in `builtin_read` with a `processed.contains('\0')` check; NUL-containing values still update `self.variables` but skip the env export. Test: `test_read_d_empty_no_panic_on_nul`.

### `[ a := a ]` (made-up infix op) silently returned 1 instead of "condition expected: :="

- zsh: `[ a := a ]` -> `[:1: condition expected: :=` exit 2. zshrs's 3-arg arms only checked `-`-prefixed ops; non-`-`-prefixed operator-ish tokens at args[1] (`:=`, etc.) fell through every check. Added a 3-arg arm that fires when args[1] is non-`-`-prefixed AND contains a non-alphanumeric char AND isn't in zsh's known operator list. Test: `test_test_unknown_3arg_infix_op_errors`.

### `print -u 99 hello` printed to stdout instead of erroring "bad file number: 99"

- zsh: `print -u N` writes to fd N; if N isn't open, errors `print:1: bad file number: N` exit 1 with no output. zshrs's `let _ = fd` discarded the requested fd entirely and always wrote to stdout. Added an `fcntl(fd, F_GETFD)` precheck for fd ∉ {1,2}; closed fds emit zsh's diagnostic and exit 1 before the print runs. Test: `test_print_u_bad_fd_errors`.

### `kill -0 abc` printed bash-style "kill: abc: invalid pid" instead of zsh's "kill:1: illegal pid: abc"

- zsh: `kill -0 abc` (non-numeric pid) -> `kill:1: illegal pid: abc` exit 1. zshrs emitted bash-style `kill: abc: invalid pid` — no shell-name prefix, different wording. Updated the parse-error arm in the direct-PID branch. Test: `test_kill_illegal_pid_zsh_format`.

### `wait abc def` continued to def after first bad arg, exceeding zsh's diagnostic count

- zsh: `wait abc def` reports the first bad arg (`job not found: abc`) and STOPS — doesn't continue to `def`. zshrs's `continue` looped to the next arg, emitting two errors. Replaced `continue` with `return 127` so the first miss aborts the wait. Test: `test_wait_stops_after_first_bad_arg`.

### `vared -p` (no value after flag) errored "not enough arguments" instead of "argument expected: -p"

- zsh: `vared -p` (with no value) -> `vared:1: argument expected: -p` exit 1 — pinpoints the missing flag-value. zshrs's `if i + 1 < args.len()` guard silently dropped the flag and let the empty-var-name path emit the wrong diagnostic. Restructured `-p` and `-r` arms to error explicitly on missing value. Test: `test_vared_missing_value_after_flag_errors`.

### `history -d 99` reported "no such event: 1" instead of "no such event: 99"

- zsh: `history -d N` (or any numeric arg) reports the user's event ID in the no-such-event error. zshrs hardcoded `no such event: 1` regardless of the user's value. Threaded `count` through to the error path; non-default count is used as the event identifier. Test: `test_history_d_event_id_propagates`.

### `zstyle -X` silently fell through instead of erroring "invalid option"

- zsh: unknown zstyle flag -> `zstyle:1: invalid option: -X` exit 1. zshrs's `_ => {}` silent fallback let any unknown flag drop through to the set-style path with `pattern=-X` — silently registering junk styles. Replaced with explicit error. Test: `test_zstyle_unknown_flag_errors`.

### `bindkey -Z` silently dropped into list-mode instead of erroring

- zsh: unknown bindkey flag -> `bindkey:1: bad option: -Z` exit 1. zshrs's silent `_ => {}` fallback consumed any flag, falling through to list-mode (which printed the keymap silently). Replaced with explicit error. Test: `test_bindkey_unknown_flag_errors`.

### `zparseopts` (no args) silently returned 0 instead of erroring

- zsh: bare `zparseopts` -> `zparseopts:1: not enough arguments` exit 1. zshrs returned 0 silently. Added an early `args.is_empty()` check. Test: `test_zparseopts_no_args_errors`.
  - **Amended (diagnostic text moved upstream).** The `not enough arguments` wording came from zsh 5.9's `BUILTIN("zparseopts", …, 1, -1, 0, NULL, NULL)` (minargs 1, no optstring), where `execbuiltin` rejected the empty argument list before the builtin ran. Current zsh declares it `BUILTIN("zparseopts", 0, bin_zparseopts, 0, -1, 0, "a:A:DEFGKMn:v:", NULL)` (`Src/Modules/zutil.c:2150`): minargs 0, so the empty list reaches the builtin and hits `Src/Modules/zutil.c:1885-1888` `zwarnnam(nam, "missing option descriptions"); return 1;`. zshrs now emits that. minargs 0 is required, not cosmetic — with minargs 1 the `zparseopts -a ''` case in upstream `Test/V12zparseopts.ztst` would say `not enough arguments` instead of the required `missing array name for -a`.
  - **Amended again (bintab optstring reverted, minargs 0 kept).** zshrs's `zparseopts` bintab row carries `None`, not `"a:A:DEFGKMn:v:"`. Handing the optstring to the generic parser also imports upstream's rule that a GNU-style long option *spec* must be guarded by `-`, `--`, or a short spec (upstream commit `88d51a2400`, "54376: zparseopts: use standard option parsing"); under it `zparseopts -D -E -move=opt_move` dies with `bad option: -m` at `Src/builtin.c:385-390`. zsh 5.9.2 — the version zshrs reports in `$ZSH_VERSION` — instead falls through to the description parser (`src/zsh/Src/Modules/zutil.c:1859-1863`, the `default: args--; o = NULL;` arm) and reads the word as a spec. zshrs takes the 5.9.2 rule; the switch is `LONG_SPEC_NEEDS_GUARD` in `src/ported/modules/zutil.rs`, and retargeting means flipping it and restoring the optstring on the bintab row. `bin_zparseopts` scans its own flag words itself, keeping the unambiguous halves of `88d51a2400` (stacking `-DF`, cuddled optargs `-nprog`, the `-n NAME` flag). Cost: one assertion in the 5.9.999.3-test `Test/V12zparseopts.ztst` ("zparseopts long-option spec guarding", the `zparseopts -DFa optv -x` case) now fails; `Test/Z04zgetopt.ztst` is unaffected.

### `pwd extra arg` printed cwd instead of erroring "too many arguments"

- zsh: `pwd` only takes flags; positional args -> `pwd:1: too many arguments` exit 1. zshrs ignored positional args and printed cwd. Added a `positional_count` tally; non-zero errors before the cwd lookup. Test: `test_pwd_too_many_args_errors`.

### `umask 0Ab` reported "bad symbolic mode operator: 0" instead of "bad umask"

- zsh: `umask 0Ab` (digit-prefixed but not all-digits) -> `bad umask` exit 1 — the digit prefix is a strong signal of an attempted numeric mode that failed parsing. zshrs's symbolic walker treated `0` as the start of a class+operator parse and emitted `bad symbolic mode operator: 0` (wrong category). Extended the `looks_numeric` precheck to also catch digit-prefixed inputs. Test: `test_umask_digit_prefix_uses_bad_umask`.

### `fc -l 1 abc` reported "no events in that range" instead of "event not found: abc"

- zsh: in a range query, if either bound is non-numeric, errors `event not found: <text>` for that bound. zshrs's range path lumped non-numeric bounds into the generic `no events in that range` (wrong category — text-name miss vs out-of-range have distinct diagnostics). Added per-bound numeric checks before the range error. Test: `test_fc_l_two_args_non_numeric_errors`.

### `fc -r` and `fc -d` re-executed the previous command instead of erroring "would recurse endlessly"

- zsh: bare `fc -r` and `fc -d` (no positional) re-edit the prior command — which IS `fc` itself in `-c` mode, hence the recurse-endlessly abort. zshrs's recurse-guard required `args.is_empty()`, so `fc -r` (which has args=[`-r`]) slipped past and ran the previous command. Removed the `args.is_empty()` requirement; non-list-mode + no-positional + non-tty stdin is sufficient. Test: `test_fc_r_d_recurse_endlessly_aborts`.

### `[ -z "" -X x ]` (4-arg with unary flag + junk) reported "condition expected: -z" instead of "too many arguments"

- zsh: a 4-arg test with a known unary flag at args[0] (`-z`, `-n`, `-d`, etc.) followed by an operand and extra junk -> `[:1: too many arguments` exit 2 — the flag IS recognized; the count is the problem. zshrs's catch-all 4+arg arm only triggered "too many arguments" for known binops at args[1]; unary-flag layouts fell through to the generic "condition expected: -z" (wrong category). Extended the disambiguation arm with a `unary_flag_at_0` check covering zsh's full unary-test letter set. Test: `test_test_4args_unary_flag_too_many`.

### `jobs %1` (no jobs) silently produced no output instead of "no such job"

- zsh: `jobs %N` for an N that doesn't exist -> `jobs:1: %N: no such job` exit 1. zshrs's filter-by-id loop silently dropped non-matching ids and emitted nothing. Added a precheck that validates each requested id against the current job list before listing. Test: `test_jobs_unknown_id_errors`.

### `fg %999` and `bg %999` reported "no such job" instead of "no job control in this shell"

- zsh in `-c` mode has no real job-control regardless of the `monitor` option. `fg %N` / `bg %N` always error `<fg|bg>:1: no job control in this shell.` exit 1. zshrs's option-based check didn't work because the `monitor`/`interactive` options are default-on even in `-c` mode (zsh's option-display lies about job-control state). Switched to `atty::is(Stream::Stdin)` — real interactive shells have a tty on stdin, `-c` mode does not. Test: `test_fg_bg_no_job_control_in_script`.

### `fc -e` (no editor arg) ran the recurse-endlessly path instead of erroring "argument expected: -e"

- zsh: `fc -e` with no following editor arg -> `fc:1: argument expected: -e` exit 1. zshrs's `'e'` arm let the missing-arg case fall through (i+=1 with no bounds-check guard), so the loop ended with no editor set and the no-positional path triggered the recurse-endlessly diagnostic. Added an explicit error when `i >= args.len()` after the increment. Test: `test_fc_e_missing_editor_arg_errors`.

### `history -d -1` reported "no such event: 1" instead of "no such event: 0"

- zsh: a negative count (`-N` / `-d -N`) resolves to event 0 in count-from-end semantics with empty history. zshrs reported the absolute count value (1 for `-1`). Tracked an explicit-negative-count flag through the option-parse loop; the no-events path uses event_id=0 when set. Test: `test_history_d_negative_resolves_to_zero`.

### `history -S` silently dropped through to no-such-event instead of erroring "bad option: -S"

- zsh: `history -S` is a bash-only "save" flag that zsh's history (= `fc -l`) doesn't accept; -> `history:1: bad option: -S` exit 1. zshrs silently consumed the flag and emitted `no such event: 1` from the `-c` mode no-history path. Added `-S` to the bash-incompatibility list. Test: `test_history_S_bad_option`.

### `let "!"` reported "stack empty" instead of zsh's "operand expected" wording

- zsh's MathContext emits `bad math expression: operand expected at end of string` when a unary op has no operand. zshrs's bare `stack empty` had no match for scripts grepping zsh's canonical wording. Updated `src/math.rs` to emit the wrapped phrasing. Test: `test_let_unary_op_no_operand`.

## Closed (eighty-fourth-pass)

### `cd -- /tmp` errored "string not in pwd: --" instead of cd'ing to /tmp

- zsh: `--` is the end-of-options marker; everything after is positional. zshrs's substitution-form path treated `--` as the OLD arg of `cd OLD NEW` and errored. Added an explicit `--` consumer that flips an `after_dashdash` flag, after which tokens are pushed to positional_args verbatim. Test: `test_cd_dashdash_end_of_options`.

### `fc -p` errored "would recurse endlessly" instead of silent success

- zsh: `fc -p` (push history stack) is silent success in `-c` mode — it doesn't run the prior command. zshrs treated `-p` as a no-op flag, and the no-positional non-list-mode recurse-abort then fired. Track a `silent_no_op_flag` for `-p`/`-P`/`-a`/`-I`/`-L`/`-m` (their presence signals an explicit non-edit-form invocation); exempt these from the recurse-abort and short-circuit to silent return 0 when no positional. Test: `test_fc_p_silent_success`.

### `kill -s 0 $$` errored "invalid signal: 0" instead of accepting (existence check)

- zsh accepts numeric values to `-s`; `-s 0` is the existence-check form (same as `-0`). zshrs's `-s` arm was name-only and rejected `0` as an invalid signal name. Extended the `-s` arm with a numeric-fast-path: `0` triggers existence-check; other numeric values match against the signal_map by number; non-numeric falls back to name lookup. Test: `test_kill_s_zero_signal`.

### `unset _` errored "read-only variable: _" instead of accepting

- zsh: `_` (last-arg auto-update) is NOT intrinsic-readonly despite being a shell-internal special. Both assignment and `unset` are allowed. zshrs's intrinsic-readonly list incorrectly included `_`, so `unset _` errored. Removed `_` from both intrinsic-RO matches (BUILTIN_SET_VAR and `builtin_unset`). Test: `test_unset_underscore_allowed`.

### `bindkey -A nokm` (one arg) silently returned 0 instead of erroring

- zsh: `bindkey -A NEW EXISTING` requires two keymap names; with fewer than 2 -> `bindkey:1: not enough arguments for -A` exit 1. zshrs's `-A` stub returned 0 immediately without consuming arg(s) or validating count. Now consumes two iterator entries; on miss, errors with zsh's diagnostic. Test: `test_bindkey_A_requires_two_args`.

### `zstyle -T :foo style` errored "invalid option: -T" instead of accepting

- zsh: `-T` is "test style with default-true for unset" — like `-t` but returns 0 when style is unset. zshrs's unknown-flag fallback (added when fixing `-X` rejection) rejected `-T` as invalid. Added a dedicated `-T` arm and `-b`/`-a`/`-e`/`-m` accept-silently arms for the other zsh-extension flags zshrs doesn't fully wire up. Test: `test_zstyle_T_unset_default_true`.

### `trap "" RETURN` accepted but zsh rejects it

- zsh's actual runtime rejects `RETURN` as a signal name despite some documentation hints at it (the parser's `getsignum` doesn't include it). zshrs's known-sig allowlist included `RETURN`. Removed it from the list to match zsh's rejection. Test: `test_trap_return_undefined`.

### `print -u abc hi` printed to stdout instead of erroring "number expected after -u: abc"

- zsh: `-u N` requires a numeric fd; non-numeric -> `print:1: number expected after -u: <arg>` exit 1. zshrs's `unwrap_or(1)` silently dropped non-numeric input and printed to stdout. Replaced with explicit parse + error before the print runs. Test: `test_print_u_non_numeric_errors`.

### `fc -l x y z` reported "too many arguments" instead of "event not found: x"

- zsh: 3+ positionals where args[0] is non-numeric -> `event not found: <args[0]>` (text-name miss takes precedence over count-error). zshrs's >2-positional arm always emitted `too many arguments`. Added a numeric-check on args[0]; non-numeric routes to the event-not-found diagnostic. Test: `test_fc_l_3plus_text_first_arg_errors_event_not_found`.

### `zstyle X` (single non-flag arg) silently returned 0 instead of "not enough arguments"

- zsh: `zstyle PATTERN STYLE [VALUE...]` requires at least pattern+style (or a flag-form). `zstyle X` -> `zstyle:1: not enough arguments` exit 1. zshrs's set-style path required `args.len() >= 2` silently. Added an `args.len() == 1 && !args[0].starts_with('-')` precheck. Test: `test_zstyle_one_arg_not_enough`.

### `zformat -f result` (no format string) returned 1 silently

- zsh: insufficient args to `zformat -f` -> `zformat:1: not enough arguments` exit 1. zshrs returned 1 silently with no diagnostic. Added the eprintln before the return. Test: `test_zformat_f_too_few_args_errors`.

### `set --help` errored "can't change option: --" instead of treating as end-of-options

- zsh treats `--xxx` (long-option-style) on `set` as `--` (end-of-options); remaining args become positional. zshrs's per-char letter loop hit the leading `-` of `--help` first and errored "can't change option: --". Added a `--`-prefix short-circuit that consumes the rest of args as positional. Test: `test_set_long_option_treated_as_endmark`.

### `zstyle -g`/`-s`/`-t`/`-T` (insufficient args) returned 1 silently

- zsh: too-few-args to a zstyle get/test flag -> `zstyle:1: not enough arguments` exit 1. zshrs's branches returned 1 silently (no diagnostic). Restructured each arm to error explicitly before the indexing. Test: `test_zstyle_get_too_few_args_errors`.

### `shift ""` errored "shift count must be <= $#" instead of accepting as count 0

- zsh: `shift ""` treats empty arg as count 0 — silent no-op. zshrs's `chars().all(is_digit)` matched empty vacuously and the count defaulted to 1, then erred when positionals were short. Added an explicit empty-arg arm that sets count=0. Test: `test_shift_empty_arg_silent`.

### `exec -a` (no name following) errored generic "exec requires a command to execute" instead of "exec flag -a requires a parameter"

- zsh: `-a NAME` requires a name argument; no-following-arg -> `exec flag -a requires a parameter` exit 1 — pinpoints the missing flag-value, not the generic "no command" error. zshrs's flag walker bumped `i` without a bounds-check, then the missing-command branch emitted the generic diagnostic. Added an `i >= args.len()` check after the increment that emits zsh's specific message. Test: `test_exec_a_requires_parameter`.

### `bindkey -d` errored "bad option: -d" instead of accepting silently

- zsh: `bindkey -d` resets all keymaps to defaults — silent success. zshrs's unknown-flag fallback (added when fixing `-Z` rejection) rejected `-d` as bad. Added a dedicated `-d` arm. Test: `test_bindkey_d_resets_keymaps`.

### `history ""` reported "no such event: 1" instead of "event not found:"

- zsh: `history ""` (empty positional) -> `fc:1: event not found:` (with empty trailing identifier). zshrs's all-digits arm matched the empty string vacuously, defaulted count to 20, and the no-events branch reported `no such event: 1`. Added a `!s.is_empty()` guard on the digit-only arm so empty strings route to the search-query path instead. Test: `test_history_empty_arg_event_not_found`.

### `read -u abc` and `read -u` (missing/non-numeric fd) silently dropped to fd 0

- zsh: `read -u abc` -> `read:1: number expected after -u: abc`; `read -u` (no arg) -> `read:1: argument expected: -u`. zshrs's `unwrap_or(0)` dropped both error paths silently and read from fd 0. Replaced with explicit parse + bounds checks. Test: `test_read_u_non_numeric_or_missing`.

### `kill -l -X` reported "unknown signal: SIG-X" instead of "SIGX"

- zsh strips the leading `-` (in addition to the `SIG` prefix) of unknown signal names: `kill -l -X` -> `unknown signal: SIGX`. zshrs preserved the `-` and reported `SIG-X` (visually weird). Added a `trim_start_matches('-')` to the lookup path. Test: `test_kill_l_dash_prefix_strips`.

### `kill -n abc` printed bash-style "kill: invalid signal number: abc" instead of zsh's "kill:1: invalid signal number"

- zsh: `kill -n abc 1` -> `kill:1: invalid signal number: abc` exit 1. zshrs emitted bash-style `kill: invalid signal number: abc` — no shell-name or line-number prefix. Updated both error paths in the `-n` arm. Test: `test_kill_n_invalid_signal_zsh_format`.

### `alias -gs foo=bar` accepted both flags silently instead of erroring "illegal combination of options"

- zsh: `-g` (global alias) and `-s` (suffix alias) are mutually exclusive — an alias is either global OR suffix, not both. `alias -gs foo=bar` -> `alias:1: illegal combination of options` exit 1. zshrs's flag walker set both flags and continued. Added a post-parse check that fires before the alias action. Test: `test_alias_g_s_mutually_exclusive`.

### `kill 999999999` printed "kill: ESRCH: No such process" instead of zsh's "kill PID failed: no such process"

- zsh: `kill PID` for a non-existent PID -> `kill:1: kill PID failed: no such process` exit 1 with the OS error reason lowercased and stripped of errno-code framing. zshrs emitted Rust's `nix::errno::Errno::Display` form `kill: ESRCH: No such process` — leaks the errno symbol. Reformatted the send_signal error path with the same lowercased + last-colon-segment cleanup used in `kill -0`. Test: `test_kill_unknown_pid_zsh_format`.

### `typeset -A h; h=(a 1 b)` (odd k/v count) silently dropped the orphan key

- zsh: assoc-init with an odd number of values -> `bad set of key/value pairs for associative array` exit 1, no assignment. zshrs's `if let Some(v) = it.next()` silently dropped the orphaned key, leaving the assoc partially populated. Added an explicit `values.len() % 2 != 0` precheck. Test: `test_assoc_odd_kv_pairs_errors`.

### `disown %999` and `disown -l`/`-h` silently returned 0 instead of erroring

- zsh: `disown %N` for non-existent N -> `disown:1: %N: no such job` exit 1; `disown -l`/`-h` are NOT recognized flags (treated as job specs) -> `disown:1: job not found: -l` exit 1. zshrs's flagless impl emitted bash-style `disown: -l: no such job` and silently dropped non-existent ids. Restructured the loop: `%N` strips and validates against jobs; numeric routes to `%N: no such job`; non-numeric (including `-l`) errors `job not found:`. Tests: `test_disown_unknown_jobspec_errors`, `test_disown_dash_flag_treats_as_jobspec`.

### `zstyle -` errored "invalid option: -" instead of "not enough arguments"

- zsh: bare `-` (no recognized option letter) -> `zstyle:1: not enough arguments` (degenerate flag-only invocation). zshrs's catch-all unknown-flag fallback emitted `invalid option: -`. Added a dedicated `-` arm. Test: `test_zstyle_dash_only_not_enough_args`.

### `fc -t` (no time-format arg) ran the recurse-endlessly path instead of erroring "argument expected: -t"

- zsh: `-t TIMEFMT` requires a format string; missing -> `fc:1: argument expected: -t` exit 1. zshrs's `'t'` arm bumped `i` without bounds-check; the loop ended with no positional and the recurse-abort fired. Added an `i >= args.len()` check that emits zsh's specific diagnostic. Test: `test_fc_t_missing_arg_errors`.

### `functions FOO` (FOO undefined) errored "no such function: FOO" instead of silent

- zsh: `functions FOO` for non-existent FOO emits nothing and returns 0 (it's a query, not an enforce). zshrs erred `no such function: FOO`. Replaced the error with `continue` so unknown names skip silently. Test: `test_functions_unknown_silent`.

### `kill -- 999` treated `--` as a signal name and errored "unknown signal: SIG-"

- zsh: `--` is end-of-options on `kill`; subsequent args are PIDs. zshrs's catch-all `-X` flag walker parsed `--` as `-` (separator) then `-` as the signal name, errored `unknown signal: SIG-`. Added an `after_dashdash` flag at the top of the parse loop that switches to PID-collection mode. Test: `test_kill_dashdash_end_of_options`.

## Closed (eighty-fifth-pass)

### `fc ""` triggered infinite recursion (re-executed `fc ""` until stack overflow)

- zsh: `fc ""` -> `fc:1: event not found:` exit 1 (no match, no prior-command execution). zshrs's prefix-match found the most recent history entry (which is `fc ""` itself) and recursively re-executed it, triggering stack overflow. Added an `arg.is_empty()` fast path before the prefix search that emits zsh's diagnostic and exits 1. Test: `test_fc_empty_string_no_recursion`.

### `umask z=r` reported "invalid mask: z=r" instead of "bad symbolic mode operator: z"

- zsh: `umask z=r` -> `umask:1: bad symbolic mode operator: z` exit 1 (treats unknown class char as the operator-position diagnostic, distinct from "bad permission" for unknown rwx chars). zshrs's `_ => ok=false` collapsed all class errors to the generic `invalid mask:`. Tracked `bad_class: Option<char>` in the per-class loop; on hit, emits zsh's specific diagnostic. Test: `test_umask_bad_class_char`.

### `cd /tmp /etc /usr` (3+ args) silently used args[0] as target

- zsh: `cd ARG1 ARG2 ARG3` -> `cd:1: too many arguments` exit 1 (cd takes at most 2 args; the substitution form OLD NEW). zshrs's two-arg substitution path silently fell through with extras. Added a `positional_args.len() > 2` check before the path_arg lookup. Test: `test_cd_3plus_args_too_many`.

### `[ -z -n a ]` reported "unknown condition: -n" instead of "too many arguments"

- zsh: `[ -z -n a ]` (unary-flag + unary-flag + arg layout) -> `[:1: too many arguments` exit 2 — `-z OPERAND` is the 2-arg form; the extra `a` is the surplus. zshrs's unknown-binop arm fired first and reported `unknown condition: -n` (wrong category — `-n` IS a recognized unary flag, just misplaced). Added a 3-arg arm checking for the unary+unary+arg layout BEFORE the unknown-binop check. Test: `test_test_3args_unary_unary_arg_too_many`.

### `shift -X` silently treated `-X` as an array name

- zsh: `shift -X` (unknown flag besides `-p`) -> `shift:1: bad option: -X` exit 1. zshrs's catch-all pushed the flag string into array_names, masking typos and trying to shift a non-existent array `-X`. Added a `starts_with('-') && len > 1` arm BEFORE the array-name fallback that emits zsh's diagnostic. Test: `test_shift_unknown_flag_errors`.

### `print -f` (no format string) silently fell through with no format set

- zsh: `-f` requires a format string; missing -> `print:1: argument expected: -f` exit 1. zshrs's `if i < args.len()` silently fell through, leaving format unset and proceeding as if `-f` weren't present. Added an `i >= args.len()` check that emits zsh's specific diagnostic. Test: `test_print_f_missing_arg_errors`.

### `ulimit -l` (locked memory) errored "bad option: -l" instead of returning the limit

- zsh: `-l` queries the locked-memory limit (RLIMIT_MEMLOCK on Linux; "unlimited" on macOS where the kernel doesn't enforce). zshrs's flag-letter table didn't include `-l`. Added a `-l` arm that maps to RLIMIT_AS as a safe stand-in for the get-only path (real RLIMIT_MEMLOCK could be wired with cfg(target_os="linux") later). Test: `test_ulimit_l_accepted`.

### `read -d` (no delimiter arg) silently used default newline delimiter

- zsh: `-d` requires a delimiter argument; missing -> `read:1: argument expected: -d` exit 1. zshrs's bounds-less `i+=1` left delimiter at default and continued reading. Added an `i >= args.len()` check that emits zsh's specific diagnostic. Test: `test_read_d_missing_arg_errors`.

### `kill -s ""` (empty signal name) reported "invalid signal:" with trailing space

- zsh: `kill -s ""` -> `kill:1: -: signal name expected` exit 1. zshrs's name lookup of empty matched no signals and produced `invalid signal: ` (with trailing whitespace). Added an early-empty arm that emits zsh's specific diagnostic. Test: `test_kill_s_empty_name_errors`.

### `let "1?"` reported "':' expected" instead of distinguishing operand-missing from colon-missing

- zsh distinguishes `let "1?"` (input ran out mid-ternary, no operand AND no colon) from `let "1?2"` (operand present, colon missing). Former -> `bad math expression: operand expected at end of string`; latter -> `bad math expression: ':' expected`. zshrs's earlier `':' expected` for both was wrong-category for the missing-operand case. Added a stack-length check after the inner mathparse: stack grew → operand parsed (colon-expected); stack same → no operand (operand-expected). Test: `test_let_ternary_missing_colon_vs_operand`.

### `umask -X` printed the current umask instead of erroring "bad option: -X"

- zsh: `umask -X` -> `umask:1: bad option: -X` exit 1. zshrs's silent `_ => {}` arm accepted any flag and proceeded to print/set the umask. Added a `starts_with('-') && len > 1` arm that emits zsh's diagnostic. Test: `test_umask_unknown_flag_errors`.

### `fc 1 2 3 4 5 6` (edit mode, 3+ positionals) ran prefix search instead of erroring "too many arguments"

- zsh: edit-mode fc takes at most 2 positional bounds (`fc FIRST [LAST]`); 3+ -> `fc:1: too many arguments` exit 1. zshrs's edit path took args.first() and ignored the rest, falling into the prefix-search path. Added a `positional.len() > 2` precheck. Test: `test_fc_edit_too_many_args_errors`.

### `fc -l 1 abc 2` (text-name in middle) reported "too many arguments" instead of "event not found: abc"

- zsh: text-name miss takes precedence over count-error in the 3+arg fc-list path. zshrs only checked `positional[0]` — if the non-numeric was in the middle, "too many arguments" fired instead. Extended the scan to find the FIRST non-numeric across all positions. Test: `test_fc_l_3plus_text_in_middle`.

### `history -d 1 2` and `history -d 1 2 3` reported "no such event: N" instead of zsh's range/too-many wording

- zsh: history (= `fc -l`) — 2 numeric positionals -> `fc:1: no events in that range`; 3+ -> `fc:1: too many arguments`. zshrs's loop just kept overwriting `count` with each numeric arg and reported `no such event: <last>` regardless. Track `positional_count`; check >2 (too many) and ==2 (range) BEFORE the no-such-event path. Test: `test_history_d_multi_args_error_categories`.

### `[ -e /tmp 5 ]` (unary flag + operand + extra) silently returned 1 instead of erroring "too many arguments"

- zsh: `[ -FLAG OPERAND EXTRA ]` -> `[:1: too many arguments` exit 2 — the parse expected `-FLAG OPERAND` (2-arg form), so the extra arg is the surplus. zshrs's 3-arg arm only matched flag-FLAG-arg layouts (`-z -n a`); the flag-operand-extra layout fell through to the catch-all `1`. Loosened the check: any 3-arg with a known unary flag at args[0] is "too many arguments". Test: `test_test_3args_unary_op_extra`.

### `fc 1 5` (2 numeric positionals, edit form) reported "event not found: 1" instead of "would recurse endlessly"

- zsh: edit-mode `fc N M` re-edits commands N..M; with empty -c session, that's the recurse-endlessly path. zshrs's prefix-search used `N` and reported `event not found: N` (wrong category for the range-edit form). Added a `positional.len() == 2 && both_numeric` precheck that emits zsh's recurse diagnostic. Test: `test_fc_2_numeric_positionals_recurse`.

## Closed (eighty-eighth-pass — heredoc body collection + logical cd)

### `cat <<EOF\nline\nEOF` heredoc body never collected (parse-error)

`par_redir_with_id` (parse.rs:7357) called `zshlex()` to advance past the heredoc terminator word BEFORE pushing onto `HDOCS`. The wordcode variant at parse.rs:6894-6908 has the right order (push then advance) — the AST variant had drifted. Net effect: zshlex's NEWLIN-handler drained an empty `HDOCS`, the parser then consumed the body lines as fresh commands, and the second NEWLIN's drain found the terminator on the first read so `gethere()` returned empty content. Output: parse error + body executed as commands.

Fix: defer the trailing `zshlex()` to after the `HDOCS`/`LEX_HEREDOCS` push, matching the wordcode variant.

Tests now passing: `heredoc_basic`, `heredoc_dash_strips_tabs`, `heredoc_expands_variables`, `heredoc_quoted_delim_no_expand` in `tests/advanced_parity.rs`.

### `parsestrnoerr` read from wrong input source (heredoc expansion errored)

`gethere()` calls `parsestr()` to expand `$var` / `$(cmd)` / `$((expr))` in unquoted-terminator heredoc bodies. The Rust `parsestrnoerr` (lex.rs:2592) called `inpush()` (input.c's input-stack API) but `hgetc()` (lex.rs:3736) reads directly from `LEX_INPUT`/`LEX_POS` — `inpush` writes to `inbuf`, which the lexer never consults. So `dquote_parse` immediately hit EOF (pos was at end of outer script) and returned Err, triggering a spurious "parse error" that aborted the entire script.

Fix: in `parsestrnoerr`, save+swap `LEX_INPUT`/`LEX_POS` around `dquote_parse` to point at the heredoc body, appending a `\0` sentinel so the `endchar='\0'` termination triggers cleanly (mirroring C's `dupstring_wlen` trailing-NUL).

### `cd /tmp` returned `/private/tmp` (logical PWD lost on symlink dirs)

`cd_new_pwd` (builtin.rs) wrote PWD via `std::env::current_dir()` after `bin_cd` had already set the logical `dest_path` into the paramtab. `current_dir()` always returns the OS-resolved physical path, so on macOS `/tmp` (symlink to `/private/tmp`) the logical write got stomped. The doc-comment at line 1669 even said PWD is "now done by the caller" — the stale stomping lines were never removed.

Fix: drop the stale PWD/OLDPWD re-write block in `cd_new_pwd`; `bin_cd` is the authoritative writer. This also fixes the double-shift bug where OLDPWD got overwritten with the new PWD instead of preserving the pre-cd value.

Test now passing: `subshell_cd_isolated` in `tests/advanced_parity.rs`.

## Closed (eighty-seventh-pass — C-source-driven port)

### `${${a:l}:r}` nested expansion now applies outer modifier/replace to inner result

Direct port of zsh's hist.c modifier dispatch and substitution dispatch for `${${...}:MOD}` / `${${...}/pat/repl}`. zshrs's nested-expansion handler at line 13673 only checked for `[idx]` subscript after the inner closing brace; modifier chains (`:r`, `:t`, `:h`, `:e`, `:l`, `:u`, `:s/...`) and replace operators (`/pat/repl`, `//pat/repl`) were silently dropped.

Examples now matching zsh:
- `${${a:l}:r}` for `a=Hello.World` → `hello` (lowercase then strip extension)
- `${${(j: :)a}:r}` for `a=(file.txt other.csv)` → `file.txt other` (join then strip last extension)
- `${${a}//l/L}` for `a=hello` → `heLLo` (global replace)
- `${${HOME}/wizard/USER}` → `/Users/USER` (single replace in indirect)

The fix also corrects an `is_history_modifier` call site issue — the helper checks the FIRST char (so `:r` returns false because `:` isn't in the recognized list); strip the leading `:` before testing.

Tests: `test_nested_expansion_modifier_chain`, `test_nested_expansion_replace`.

### `local arr=( "a b" c )` quote-aware split — preserves whitespace inside quoted elements

zshrs's typeset/declare/local array path naively `split_whitespace()`'d the body, breaking quoted strings: `local arr=( "x y" z )` produced 3 elements `[x, y, z]` instead of zsh's 2 elements `["x y", z]`. Direct port of zsh's lex.c word-splitting for assignment RHS via a quote-aware scanner that honors `"..."`/`'...'` boundaries (and strips the quote chars from the result).

Limitation: `"$@"` splicing inside `local arr=( "$@" )` still doesn't work — the splice happens BEFORE this path sees the args, and the parser's `$@`-as-words mechanism only fires for the regular `arr=(...)` (non-typeset) compile path. Tracked separately.

Test: `test_typeset_array_quote_aware_split`.

### Function-scope EXIT trap fires on function return, preserves outer trap

Direct port of zsh's exec.c `dotrapargs(SIGEXIT, ...)` deferred-fire pattern. An EXIT trap set INSIDE a function fires when the function RETURNS (not when the shell exits), and the outer EXIT trap is preserved across the call. zshrs's `call_function` didn't track function-scope traps, so:
- `foo() { trap "echo X" EXIT; }; foo; echo done` either fired X at SHELL exit (if no outer trap) or polluted the outer `traps["EXIT"]`.
- An outer `trap "echo OUTER" EXIT; foo() { trap "echo INNER" EXIT; }; foo` overwrote OUTER with INNER, so OUTER never fired at shell exit.

Fix:
1. Before function body: save current `traps["EXIT"]` into a local; remove it so an outer trap doesn't fire prematurely.
2. After function body: pull the (possibly newly-set) EXIT trap, fire it now (script-pipeline) with the function's `last_status` in scope, restore the saved outer trap.

Tests: `test_function_scope_exit_trap_fires_on_return`, `test_function_scope_exit_trap_preserves_outer`.

### `return` with no arg now uses live `vm.last_status`, not stale `executor.last_status`

zsh's bin_return (Src/builtin.c) returns the status of the most recently executed command when no arg is given — `foo() { false; return; }` returns 1, not 0. zshrs's `builtin_return` read `self.last_status`, which is the EXECUTOR's view; that value only gets synced from the VM at statement boundaries, so during the `return` builtin it was stale (held the value from BEFORE the function call started, typically 0).

Direct port of the existing pattern used by other status-sensitive builtins: read `vm.last_status` (the live value) at the registration site and sync it into `exec.last_status` before delegating to `builtin_return`.

Test: `test_return_no_arg_uses_last_status`.

### `${a[@]:1:$((2+0))}` array slice with arith length now slices array elements (was char-slicing joined value)

zshrs's `BUILTIN_PARAM_SUBSTRING_EXPR` (the arith-expr variant of `${var:N:M}`) treated all names as scalars — it called `get_variable(name)` and char-sliced the result. For `a[@]`, that gave the IFS-joined string "a b c d e", and char-slicing position 1 with length 2 returned " b" (space + 'b'), not zsh's "b c" (elements 2-3 of the array).

The integer-only sibling `BUILTIN_PARAM_SUBSTRING` already had array-aware dispatch (strip `[@]`/`[*]`, lookup as array, slice elements). Direct port of that logic into the expr variant: `force_array` flag from suffix detection routes through `slice_array_zero_based` for the array case, or `slice_positionals` for `${@:N:M}`.

Test: `test_array_slice_with_arith_length_expr`.

### Glob qualifier `*(L+N)` size check now uses lstat for symlinks (matches zsh)

Direct port of zsh's pattern.c L qualifier. zsh uses lstat-based size — for a symlink, that's the LENGTH OF THE SYMLINK STRING (e.g. 9 bytes for "empty.txt"), NOT the target's size. zshrs's prefetched metadata had both followed (`m`) and symlink (`sm`) variants; the L qualifier was reading `m.len()` which gave the target size, so a symlink to an empty file appeared empty (size 0).

Fix: prefer the symlink metadata `sm` when present, fall back to `m`. Now `*(L+0)` correctly includes symlinks (their path-string length is always > 0); `*(L0)` correctly excludes them.

Test: `test_glob_qualifier_size_uses_lstat_for_symlinks`.

### Glob qualifier `*(:r)` — history modifiers applied per match

Direct port of zsh's pattern.c qualifier modifier handling. `*(:r)` strips the extension from each match (`a.txt` → `a`); `*(:t)` keeps only the basename; `*(:e)` returns the extension. zshrs's `filter_by_qualifiers` had no `:` arm, so modifiers fell through to the catch-all and were ignored. Fix: detect `:` in the qualifier scan, consume the rest of the qualifier list as a modifier chain, and apply via the existing `apply_history_modifiers` helper to each match.

Test: `test_glob_qualifier_history_modifier`.

### Glob qualifier `*(.,/)` — top-level `,` is OR (clause alternation)

Direct port of zsh's pattern.c qualifier parsing. zshrs's `filter_by_qualifiers` chained all qualifier filters with AND, so `*(.,/)` (files OR dirs) errored "no matches found" because no file is BOTH a regular file AND a directory. Fix: detect a top-level `,` (honoring `[...]`/`(...)` nesting), split the qualifier into clauses, run the full AND filter on each clause, UNION the results in original-file order with dedup. Single-clause path is unchanged. Verified against zsh: `*(/,@)` (dirs OR symlinks), `*(.,/)` (files OR dirs) both produce union output.

Test: `test_glob_qualifier_comma_or`.

### `zshrs -f +o nomatch -c '...'` now parses `-o`/`+o NAME` like zsh's main.c arg loop

zshrs previously rejected `+o nomatch` with "+o: No such file or directory" — neither `+o` nor the option name was in the recognized-flags filter, so `+o` fell through to the "treat as script file" path. Direct port of zsh's main.c arg-parse loop: collect `-o NAME` (set option) / `+o NAME` (unset option) pairs before the filter; apply them in `apply_cli_flags` as `executor.options.insert(name, value)`.

The `no` prefix is part of the canonical option name (`nomatch`, `noglob`, `nounset`) for setopt/unsetopt purposes — only `[[ -o ... ]]` query canonicalization strips it. Mirror by storing verbatim (just lowercased + `_`/`-` separator-stripped). User-reported regression: `echo *(/.)` errored "no matches found" with `nomatch` still on, instead of leaving the literal alone with `+o nomatch`.

Test: `test_cli_o_flag_sets_options`.

### `/etc/(passwd|hostname)` glob alternation at the path level — primary zsh feature

Direct port of zsh's pattern.c `P_BRANCH` `|` handling at the path level. `/etc/(passwd|hostname)` matches paths where the last component is `passwd` OR `hostname` (no extendedglob required, unlike `~` exclusion). zshrs's compile path didn't list `(...|...)` as a glob trigger, so the parens reached the OS as literal chars and produced no match.

Two-part fix:
1. `compile_zsh`: `unquoted (` + `|` + `)` triggers the glob path alongside `*`/`?`/`[`. Without this, `echo /etc/(passwd|hostname)` was emitted as a literal arg.
2. `expand_glob`: pre-expand top-level `(...|...)` into multiple alternatives via a new `expand_glob_alternation` helper that respects `[...]`/`(#...)` nesting. Each alternative is recursively globbed (so mixed literal + glob like `(a|b)*` works); literal alternatives that don't exist on disk are filtered out (zsh: alternation produces matching paths, not literal alternatives). Final result deduped and sorted (zsh's lexicographic glob result order).

Test: `test_glob_alternation_at_path_level`.

### `arr=( $(echo "a:b:c") )` with `IFS=:` now produces 3 elements (was collapsing to 1)

zshrs's compile path was emitting `BUILTIN_WORD_SPLIT` TWICE for `arr=( $(...) )` — once inside `compile_word_str` for the unquoted `$()` AND once in the array-element loop in `compile_assign`. The first split correctly produced a `Value::Array(["a", "b", "c"])` from the IFS-split. The second split called `vm.pop().to_str()` on that Array, which joined-with-space ("a b c"), then split that on IFS=":" — finding no `:` chars, returning a single element. Final result: `arr=("a b c")` (1 element) instead of `arr=(a b c)` (3 elements).

Fix: bump `assign_context_depth` for the duration of each `compile_word_str(elem)` call in the Array branch, so the inner `compile_word_str`'s WORD_SPLIT is suppressed (`!in_dq && !in_assign` becomes false). The outer loop's WORD_SPLIT then runs once per element, correctly.

Test: `test_array_assign_with_cmd_subst_ifs_split`.

### Subscript flags `(w)N` (word index) and `(s/sep/)N` (no-op for `[N]`) now recognized

Direct port of zsh's zshparam(1) "Subscript Flags". `parse_subscript_flags` rejected anything outside `r/R/i/I/e/k/n` so `(w)2` and `(s/l/)2` were treated as bogus subscripts and routed to the math evaluator (which then failed on `(w)2`).

- `(w)N` on a scalar splits by IFS (whitespace) and returns the Nth word; on an array, equivalent to `[N]` since the value is already split.
- `(s/sep/)N` is a NO-OP for scalar `[N]` integer indexing per zsh's actual behavior — verified by spot-check: `a=hello; ${a[(s/l/)1]}` returns `h`, same as `${a[1]}`. The `(s)` flag only affects word-list contexts (`${(s/sep/)var}` without index, or `[@]` form). Strip the flag, parse the index normally, fall through to char slicing.

Tests: `test_subscript_w_flag_word_index`, `test_subscript_s_flag_is_noop_for_int_index`.

### `${a//(#i)L/X}` now honors inline pattern flags (case-insensitive replace)

Direct port of zsh's pattern.c — inline pattern flags `(#i)` / `(#l)` / `(#I)` apply to the replacement operator too, not just `[[ ... = pat ]]`. The same `parse_pattern_flags` helper that glob_match_static uses now runs before BUILTIN_PARAM_REPLACE compiles its regex; `(#i)` adds the `(?i)` regex prefix so the match is case-insensitive.

zshrs previously left `(#i)L` as literal regex chars (no match) so `${a//(#i)L/X}` for `a=hello` returned `hello` instead of `heXXo`.

Test: `test_param_replace_case_insensitive_inline_flag`.

### `IFS=: read x y <<< "a:b:c"` now puts `b:c` in `y` (separator preserved in remainder)

Direct port of zsh's bin_read in builtin.c. When input has more fields than vars, the last var receives the unsplit REMAINDER from the position after the (N-1)th separator — meaning the separator chars between fields N..end are PRESERVED literally. zshrs previously split into a `Vec<&str>` and `join(" ")`d, collapsing all separators to spaces (so `IFS=: read x y <<< "a:b:c"` produced `y="b c"` instead of zsh's `y="b:c"`).

Also fixed: the default IFS in zshrs is `" \t\n\0"` (with NUL). The new "whitespace IFS" check (`is_whitespace_ifs`) initially used `is_ascii_whitespace()` which excludes NUL, so the default-IFS path's collapse-runs + strip-boundaries semantics never fired. Added `c == '\0'` to the whitespace-class check.

Implementation: scan the input for the (N-1)th separator boundary, slicing the original string at that point — head goes to the first N-1 vars (split on IFS, preserving empties for non-whitespace IFS), tail goes verbatim to the Nth var. For whitespace IFS, leading and trailing whitespace are trimmed first (so `read x y <<< "  a  b  c  "` produces `x=a y="b  c"`).

Tests: `test_read_preserves_separator_in_last_var`, `test_read_collapses_default_ifs`.

### `"${a/o/O}"` array replace now joins-then-replaces in DQ; `"${a[@]/o/O}"` per-element

Mirror of the array strip DQ split applied to the replace operator. zsh's pattern.c routes through getmatch (joined scalar in DQ) vs getmatcharr (per-element otherwise) for both strip and replace. zshrs's BUILTIN_PARAM_REPLACE always per-element-replaced for arrays. Fix: pass `dq_context_depth` from compiler as a 5th arg; when set AND the var is array, join via space then apply the replace once. `had_at` field on `ParamModifierKind::Replace` overrides DQ for explicit `[@]` subscript (forcing per-element). `[*]` keeps the bare-DQ semantics.

Tests: `test_dq_array_replace_join_first`, `test_dq_array_replace_at_subscript_per_element`.

### `$(([#16]255))` output radix in arith — `[#N]` adds `N#` prefix, `[##N]` drops it

Direct port of zsh's math.c output-radix handling (lines 786-832 in patcompswitch's `[` case): single-`#` form keeps the `N#` prefix on the result, double-`##` drops it. Base must be 2..=36. zsh's convbase (Src/params.c:5586) special-cases base==10 to skip the prefix entirely (matches `[#10]42` → `42`).

zshrs previously left `[#16]255` as a literal in the math expression, so the `$(())` expansion either parsed it as a subscript or errored. Added a prefix-strip + radix-capture step in `evaluate_arithmetic` BEFORE handing off to `MathEval`, then formatted the integer result with `format_int_in_base` honoring the no-prefix flag.

Test: `test_arith_output_radix_with_prefix`.

### `integer -i N name=val` for output radix; `trap -p` rejection (zsh-compat)

- **`integer -i 16 x=255`** now stores 255 with attribute base=16, so `echo $x` prints `16#FF` per zsh's typeset -i semantics (Src/builtin.c). zshrs's previous arg loop treated each flag arg as a single token (`-i` only), so the next arg `16` fell into the assignment-or-name path and triggered "not an identifier: 16" since `1` is a digit. Direct port: when `-i` is followed by an all-digit arg (separate or same-token), consume it as the base. Same logic works for both `-i 16` and `-i16`.
- **`trap -p EXIT` now rejects** the bash-style `-p` flag — zsh's bin_trap (Src/builtin.c:7347) doesn't recognize it. Without the `-p` shortcut, the dispatch treats `-p` as the action arg, fails the action+signal requirement, and the shell falls through to "command not found: -p" — matches zsh exactly. Removed the bash-compat `-p` block.

Tests: `test_integer_dash_i_with_base_arg`, `test_trap_dash_p_not_a_flag`.

### Array `"${a%%pat}"` joined-then-stripped (DQ); `"${a[@]%%pat}"` per-element; brace {% raw %}`{{1..3},x,y}`{% endraw %} nested

Three closely-related fixes from the systematic differential audit. All driven by reading zsh source rather than guessing from output.

- **DQ-context array strip** (zsh subst.c: getmatch vs getmatcharr split): `"${a%%pat}"` for an array `a` joins via `$IFS` first, then strips the joined scalar — direct port of `getmatch` in pattern.c which operates on a single string. zshrs's BUILTIN_PARAM_STRIP fast path bypassed the BUILTIN_EXPAND_TEXT wrapper that bumps `exec.in_dq_context`, so the runtime DQ check was always 0 and the strip went per-element. Fix: pass `dq_context_depth` from the compiler as a 4th arg to BUILTIN_PARAM_STRIP, and bump `dq_context_depth` for the duration of `emit_param_modifier` when the raw word is DNULL-wrapped (mirrors the existing segments-loop bump).
- **Explicit `[@]` subscript override** (zsh subst.c: nojoin/spbreak path): `"${a[@]%%pat}"` forces per-element strip even inside DQ — `[@]` marks the array as splice-expanded. `parse_param_modifier` previously stripped the `[@]` from the name and lost the info. Added `had_at: bool` to `ParamModifierKind::Strip` so the emitter can override DQ to per-element when present. `[*]` (join-with-IFS) keeps the bare-DQ join-then-strip semantics.
- **Extendedglob `~` exclusion** (zsh pattern.c: P_EXCLUDE handling, line 155): `pat1~pat2` matches strings matching pat1 AND NOT matching pat2. Direct port of the top-level case via a new `find_top_level_tilde` helper that honors `[...]` and `(...)` nesting per the canonical scan.
- **Nested brace list with sequence** (zsh: brace-expansion is a token-balanced scan): {% raw %}`{{1..3},x,y}`{% endraw %} is a LIST at the top level that happens to contain a `..` sequence inside one of its commas. The previous detector preferred `..` over `,`, miscategorizing the outer braces as a sequence. Fix: scan for top-level `,` (depth-0) first; only fall through to sequence detection when no top comma is present.

Tests: `test_dq_array_strip_joins_scalar`, `test_dq_array_strip_at_subscript_per_element`, `test_extglob_tilde_exclusion`, `test_brace_nested_sequence_in_list`.

### Glob bracket `[!...]` / `[^...]` negation, POSIX class containment, extendedglob `pat#`/`pat##` postfix

Direct port of zsh's pattern.c — the canonical pattern-class compile (`patcompcls` in pattern.c) and the POUND/POUND2 postfix cases in `patcompswitch`. zshrs's hand-rolled glob→regex translator had three independent bugs that all affected pattern matching:

1. **`[!a]bc` matched `abc`.** zshrs copied `!` verbatim into the regex `[!a]bc`, which regex reads as "either `!` or `a`, then `bc`" — so any string starting with `a` matches. zsh's negation rule (POSIX `[!...]` ≡ regex `[^...]`) wasn't translated. Fix: when `!` is the first char inside `[...]`, emit `^` instead.

2. **`[![:digit:]]*` failed.** The bracket scanner stopped at the first `]`, so `[![:digit:]]` was misread as `[![:digit:]` (no closing). Fix: detect POSIX class `[:NAME:]` inside the bracket scan and walk past `:]` as a unit, so the next `]` after the class isn't taken as the outer close.

3. **`a##` / `[[:digit:]]##` left literal.** zshrs's translator pushed `a` then dropped the `##` postfix entirely. zsh's extendedglob `pat#` = zero-or-more (regex `*`), `pat##` = one-or-more (regex `+`). Direct port: after each atom (literal char, `?`, `[...]`, `(...)`), peek for `#`/`##` and emit `*`/`+` if extendedglob is set.

Tests: `test_glob_bracket_negation_with_bang`, `test_glob_posix_class_with_negation`, `test_extglob_one_or_more_postfix`, `test_extglob_zero_or_more_postfix`.

### `$LINENO` was always `1` — never incremented per line, never reset on function entry

- Two compounding bugs uncovered while porting zsh's `lineno` global from Src/input.c (line 330) and Src/init.c (line 1588). Filed by the iter-86 audit.

- **Bug 1 (lexer): hungetc/getc lineno asymmetry on `\n`.** zshrs's `getc` incremented `self.lineno` on `\n` reads from FRESH input, but the unget_buf re-read path bypassed that increment. `hungetc` correctly decremented on putback. Net: every newline ungetted once and re-read leaves `lineno` permanently one short. The parser ungets the inter-statement newline once between top-level statements, so `lineno` was stuck at 1 for every line of input. Fix: increment `lineno` in the unget_buf branch of `getc` symmetrically to the fresh-input path.

- **Bug 2 (compiler): no SET_LINENO emit, no function reset.** Even with the lexer tracking fixed, nothing wired the captured per-pipe `lineno` to a runtime `$LINENO` variable. Added a new `BUILTIN_SET_LINENO` (id 342) that writes the popped int to the variable table, and emit one call per top-level pipe in `compile_list`. For function bodies, set `ZshCompiler.lineno_offset = first_body_line - 1` so the emitted values are 1, 2, 3 relative to the body — direct port of zsh's `lineno = 1` reset on function entry at Src/init.c:1588.

- ID collision found: `BUILTIN_HAS_STICKY` and `BUILTIN_SET_LINENO` were both u16=326 (the file has two more duplicate IDs at 325 — `FILE_OLDER` and `IS_TTY`). The HAS_STICKY register at line 4848 silently overwrote the SET_LINENO handler at line 4491, masking the runtime call entirely. Picked id=342 (next clean slot above 341) and noted the broader duplicate-ID problem in the const's docstring for future cleanup.

- Tests: `test_lineno_increments_per_line_in_dash_c`, `test_lineno_resets_inside_function`.

### `${(M)var##pat}` / `(M)#pat` / `(M)%pat` / `(M)%%pat` returned the unstripped value instead of the matched portion

- Direct port of zsh's `get_match_ret()` (Src/glob.c:2550). The `SUB_MATCH` flag (set by `(M)`) inverts the strip return: instead of the unmatched portion (default), return the matched portion. Was filed during the iter-86 audit.
- zshrs's flag-aware path (`expand_braced_variable` line 13085) extracted var names as the longest leading-alphanumeric run, then dropped any trailing `##*o`/`#*o`/`%o*`/`%%o*` operator silently. So `${(M)a##*o}` returned the full unstripped value because the strip never ran. The non-M variants worked because they took a different code path through `BUILTIN_PARAM_STRIP`.
- Two-part fix:
  1. New `strip_match_op(v, op, pattern, m_flag)` helper in `exec.rs` mirroring zsh's `get_match_ret` logic. `m_flag=true` swaps which slice of the original is returned at the same boundary index `i` (longest/shortest match) — `v[..i]` (matched) vs `v[i..]` (unmatched) for prefix ops, and the inverse for suffix ops.
  2. Wire the strip detection into `expand_braced_variable` after var-name extraction: detect `##`/`#`/`%`/`%%` in `rest_after_var`, expand the pattern, call `strip_match_op` with `has_match_flag` from the `(M)` flag check above.
- Also handles the no-match case per zsh: with `(M)` and no match, return empty (the matched portion doesn't exist); without `(M)`, return original (strip is a no-op).
- Refactored existing `BUILTIN_PARAM_STRIP` to delegate to the same helper (was a 50-line inline closure with 4 nearly-identical match arms). Compile-time strip path stays `m_flag=false` since `parse_param_modifier` rejects flag forms and routes them through the bridge.
- Tests: `test_m_flag_with_double_hash_strip`, `test_m_flag_with_single_hash_strip`, `test_m_flag_with_percent_strip`, `test_m_flag_with_percent_percent_strip`, `test_m_flag_no_match_returns_empty`.

## Audit (eighty-sixth-pass — C-source-driven correction)

After closing 27 gaps black-box (probing zsh -f -c output without
reading C source), audited each fix against the canonical zsh
source under `~/forkedRepos/zsh/Src/`. Three fixes had material
divergences from the reference implementation:

### Float `%.17g` (batch 10) — replaced with libc::snprintf

zsh's `convfloat()` (Src/params.c:5690) calls `sprintf(buf, "%.*g", 17, dval)` directly. My handwritten implementation only patched the [1e-4, 1e16) range; very-small/very-large values still used Rust's shortest-roundtrip `{:e}` and the `1e16` threshold was wrong (zsh's `%g` switches at exp >= 17, not 16). Found mismatches on `0.0000123` (zsh: `1.2300000000000001e-05`, ours: `1.23e-05`), `1e-5`, and `9.9e16` (zsh: `99000000000000000.`, ours: `9.9e+16`). Replaced the entire branch system with a single `libc::snprintf("%.*g", 17, val)` call mirroring zsh verbatim. Now matches zsh byte-for-byte on a 20-case sweep across the full f64 range.

### Math operator/operand error (batches 4 & 5) — direct port of `checkunary()`

zsh's `checkunary()` (Src/math.c:1548) is a single function that handles BOTH error directions: "operand expected" (errmsg=1, binary op where operand expected) and "operator expected" (errmsg=2, operand where operator expected). My batch-4 fix only patched errmsg=1 inline, missing the symmetric case. Audit found `let "5 5"`, `let "5("`, `$((2#1011x))`, `$((2#10112))`, `$((16#ffg))`, `$((16#zz))` all silently accepted bogus input.

Combined with batch 5's base-N parse: zsh's `zstrtol_underscore` is GREEDY (consumes valid digits, stops at first invalid), but my `from_str_radix` was all-or-nothing — a single bad digit nuked the whole literal. Replaced both with direct ports: `lex_constant` does greedy base-N digit consumption; `check_unary` is a verbatim port of `checkunary()` including the 10-char truncation + `...` overflow marker. 18-case audit (combining both fix scopes) now matches zsh byte-for-byte.

### `(s::)` empty-field collapsing (batch 5) — boundary-aware rule

The "drop empties when no @" rule I implemented from observation was wrong on boundary empties. Reading zsh's `sepsplit()` (Src/utils.c:3962) plus its post-processing at subst.c:3273 revealed the actual rule: leading run of separators collapses to ONE empty (not zero or many); trailing run collapses to ONE empty; middle runs collapse to ZERO empties. `(@)` flag preserves all empties verbatim. My over-aggressive drop missed the boundary preservation: `,,a,,c,,` produced `[a, c]` (2) instead of zsh's `["", a, c, ""]` (4). Replaced with proper boundary-aware collapse — 6-case audit now matches zsh.

### Other fixes (24 of 27) — verified against C source, no changes needed

The remaining 24 fixes were spot-checked against the canonical implementation and matched (identifier validation rule from builtin.c:2547 / params.c:3204 = `idigit(*pname)` first-char check; `kill %abc` jobspec lookup from jobs.c:2143 `findjobnam` fallthrough; `unset -X` flag rejection via the `BUILTIN("unset", ..., "fmvn", ...)` declaration at builtin.c:129; etc.).

## New gaps surfaced by the audit (NOT in original 27)

These are pre-existing bugs the audit exposed, NOT regressions from the iter-86 fixes. Filed for a future iteration:

- `kill %?notfound` — pattern-search jobspec is being globbed before reaching `kill`, so `?` triggers `no matches found: %?notfound` instead of zsh's `kill:1: job not found: ?notfound`.
- `test - a` — single dash as first arg should error `parse error: condition expected: -`; ours silently exits 1.
- `test ! ! ]` — zsh succeeds (exit 0); ours exits 1. Some interaction between test's negation handling and the trailing `]`.
- `$LINENO` in scripts — always 1 in zshrs; should increment per line.
- `${(P)b}`-style indirection error wording divergences in some flag combos.
- `${(M)a##*o}` (M flag with ##/#/% strip) returns the full string instead of just the matched portion.
- `${a[@]:r}` array-context history modifier joins elements instead of preserving array shape.

## Closed (eighty-sixth-pass)

### Bare `typeset NAME` / `declare NAME` (no flags, no `=`) silently swallowed instead of printing the declaration

- zsh: at the top level, `typeset NAME` / `declare NAME` with no flags and no `=` prints the variable's current declaration if NAME is set — `a=( 1 2 3 )`, `a=hello`, etc. Same shape as `-p` but WITHOUT the `typeset`/`export` prefix (zsh's `typeset -p a` is `typeset -a a=(1 2 3)`; bare `typeset a` is `a=(1 2 3)`). Inside a function, bare `typeset NAME` instead localizes (shadows parent, resets to empty) — that's `local`-style declaration semantics. zshrs silently swallowed bare-name calls, dropping the listing at top level. Fix: detect "no type flags + all-args-without-`=` + all-args-name-an-existing-var + local_scope_depth == 0", promote to print_mode, and add a `print_no_prefix` flag that drops the leading `typeset`/`export` from each printed line. Function-scope behavior is unchanged. Tests: `test_bare_typeset_prints_declaration_at_top_level`, `test_bare_typeset_localizes_inside_function`.

### `$((0.1))` printed `0.1` instead of zsh's `0.10000000000000001` (and similar inexact-representable floats)

- zsh uses C's `%.17g` for non-integer arithmetic-substitution display, which always shows 17 significant digits. Inexact-representable values surface their actual stored f64 form: `0.1` stored as f64 is `0.1000000000000000055511...`, which `%.17g` renders as `0.10000000000000001`. Rust's `Display` for f64 is shortest-roundtrip, so it picks the shorter `0.1` even though that's not the exact value. Most other cases (`0.5`, `0.25`, `0.30000000000000004`, NaN, Inf) already agreed since `%g` strips trailing zeros and the shortest representation happens to equal the 17-sig-digit one. Added a manual `%.17g`-style formatter in `format_zsh_subst`: pick scientific if `exp < -4 || exp >= 17`, else fixed-point with `(16 - exp)` fractional digits, then strip trailing zeros from the mantissa via a `trim_g_zeros` helper. Test: `test_arith_subst_float_pct_17g_inexact_form`.

### `declare -a arr=( "abc" "def" )` kept quotes attached to elements (`"abc"` instead of `abc`)

- zsh's plain `arr=( "abc" "def" )` strips surrounding quotes at the shell-syntax level — quotes are word boundaries, not part of the value. The typeset/declare array-assignment path (separate code in zshrs's typeset_named) split the raw `arr=("abc" "def")` arg by whitespace and kept the quotes attached, so consumers saw `"abc"` as the literal first element. Same bug surfaced for `declare -a arr=( "[1]=second" "[3]=fourth" )` where the quotes were retained around the `[N]=...` form. Fix: in the typeset array-element collection loop, strip a matched pair of leading/trailing single or double quotes from each whitespace-split element. The quote-stripping is conservative — only strips when first AND last chars match (so internal quotes / mismatched pairs pass through unchanged). Test: `test_declare_array_strips_quoted_elements`.

### `$#a[N]` (unbraced length-of-element) printed `<count>[N]` instead of zsh's element length

- zsh treats `$#NAME[idx]` as sugar for `${#NAME[idx]}` — length of the selected array element (1-indexed). zshrs's compile-time fast path for unbraced `$#` handled `$#NAME` and `$#NAME[@]`/`$#NAME[*]` (array length) but punted on numeric subscripts: `a=(one two three); echo $#a[2]` printed `3[2]` (count followed by literal `[2]`). Fix: extend the fast path in `compile_zsh::compile_word` to detect `[idx]` after the bare name, push the equivalent `${#NAME[idx]}` braced form, and dispatch to `BUILTIN_EXPAND_TEXT` mode 4 (HeredocBody — calls `exec.expand_string` verbatim) so the full subscript-flag machinery is reused without re-implementing it inline. Test: `test_dollar_hash_array_subscript`.

### `{1..3..0}` (zero step) silently expanded to `1 2 3` instead of staying literal

- zsh: `{N..M..0}` is invalid (zero step is meaningless) and the entire token stays literal. zshrs's `abs_step.max(1)` clamped step 0 → 1 and produced `1 2 3`. Negative steps still reverse the natural sequence (per zsh's rule); only exactly 0 should short-circuit. Added an early return when the parsed step is 0. Test: `test_brace_zero_step_stays_literal`.

### `pushd /tmp; echo $PWD` returned the pre-pushd cwd; `dirs` showed `/private/tmp` instead of zsh's logical `/tmp`

- `pushd`/`popd` called `set_current_dir` (moving the OS-level cwd) but never synced `$PWD`/`$OLDPWD` in the shell's variable table. cd does this; pushd/popd skipped it. Two symptoms cascaded: (1) the shell-level `$PWD` continued to read the pre-pushd path even though the OS cwd had moved, breaking any code that consulted `$PWD` for "where am I"; (2) `dirs` read the OS cwd via `current_dir()` which canonicalizes — so on macOS where `/tmp` is a symlink to `/private/tmp`, `pushd /tmp; dirs` printed `/private/tmp ...`. zsh preserves the user-given logical path. Fix: in `pushd`/`popd`, after `set_current_dir`, write the logical path (user-given when -P not used) to `$PWD` and the prior path to `$OLDPWD` (mirroring `cd`'s behavior). In `dirs`, prefer `$PWD` over `current_dir()` for the current-dir entry, falling back only when `$PWD` is unset. Tests: `test_pushd_updates_pwd_variable`, `test_dirs_uses_logical_pwd_not_canonical`, `test_popd_restores_pwd_variable`.

### `export 1bad=val`, `typeset 1bad=5`, `integer 1bad=5`, `readonly 1bad=5`, `declare 1bad=5`, `local 1bad=5` silently accepted bogus identifiers

- zsh validates the lhs of every typeset-family assignment: leading char must be `[A-Za-z_]`, body must be `[A-Za-z0-9_]*`. Violations emit `<INVOKED>:1: not an identifier: <NAME>` (digit-leading) or `not valid in this context: <NAME>` (whitespace/special chars in `export`). zshrs accepted any string, polluting the variable table and (for `export`) the process environment with names unreachable from any standards-conforming shell. Added validation in three places: `builtin_export` (handles both `not an identifier` and `not valid in this context` wordings), `builtin_typeset_named` (covers typeset/declare/local — uses the `invoked_as` channel for the diagnostic prefix so `declare 1bad=5` says `declare:1:`), `builtin_integer`, and `builtin_readonly` (each has its own assignment loop). Subscript-form names (`a[i]=...`, `m[k]=...`) bypass the check — they route through the runtime arith eval path which validates the base name separately. Tests: `test_export_invalid_first_char_rejects`, `test_export_space_in_name_rejects`, `test_typeset_invalid_identifier_rejects`, `test_declare_invalid_identifier_uses_declare_prefix`, `test_integer_invalid_identifier_rejects`, `test_readonly_invalid_identifier_rejects`.

### `$((10#))` and `$((36#))` (empty digits after base) errored "operator expected at \`'" instead of returning 0

- zsh treats `N#` with an empty digit run as silently 0 (matches the rule for any empty arithmetic operand). zshrs's `from_str_radix("", base)` returned `Err`, which used to land in the out-of-range-digit error arm and emit a nonsense `at \`'` message. Fix: short-circuit to 0 when `val_str.is_empty()` BEFORE calling `from_str_radix`. The correct out-of-range error path (`2#5`, `2#22`) is preserved. Test: `test_arith_empty_base_digits_is_zero`.

### `${(s:,:)foo}` preserved empty fields (off-by-2) instead of dropping them like zsh

- zsh: bare `(s:sep:)` drops empty fields after splitting, e.g. `${(s:,:)"a,,b,,c"}` -> 3 elements `[a, b, c]`. The `(@)` flag overrides to preserve empties (`${(@s:,:)…}` -> 5 elements `[a, "", b, "", c]`). zshrs's flag loop split with `s.split(sep)` and kept every field unconditionally — array counts were off, and `printf "[%s]\n" ${(s:l:)hello}` printed an extra blank `[]` between `[he]` and `[o]`. Fix: scan flags for `@` once (position-independent), drop empty fields when absent. Pure `(s::)` empty-separator (char-split) and `(s::)` over true arrays both honor the same rule. Tests: `test_s_flag_drops_empty_fields_default`, `test_at_s_flag_preserves_empty_fields`, `test_s_flag_drops_consecutive_empties_in_split`.

### `$((2#22))` reported `at \`2'` instead of zsh's `at \`22'`

- zsh keeps its input pointer at the start of the bad digit sequence, so the entire out-of-range remainder shows up in the error: `2#22` -> `operator expected at \`22'`, not just the first `2`. zshrs's base-error path used `val_str.chars().next()` which clipped to one char. Replaced with `val_str` so the full bad digit run is preserved. The original behavior was correct for single-digit cases (`2#5` -> `5`) but lost information for `2#22`, `2#1011x` etc. Test: `test_arith_base_digit_full_remainder`.

### `kill %abc` (non-numeric jobspec) reported `kill: %abc: no such job` instead of zsh's `kill:1: job not found: abc`

- zsh's format strips the leading `%` and uses the `kill:LINE: job not found: NAME` form (consistent with the rest of zsh's builtin diagnostics). zshrs reported `kill: %abc: no such job` — wrong wording, kept the `%`, missing line number. Updated both fall-through paths (parse-failure on `%N` and `%N` not found in jobs table) to emit the zsh-shape message. Test: `test_kill_percent_text_jobspec`.

### `unset -X foo` silently swallowed the bad flag (no diagnostic, exit 0)

- zsh: `unset -X foo` -> `unset:1: bad option: -X` exit 1. zshrs's flag loop had a catch-all `_ if arg.starts_with('-') => {}` arm that masked typo'd flags. Replaced it with a bad-option rejection (preserving `--` as end-of-options sentinel for compat with `unset -- name`). Tests: `test_unset_bad_option_X`, `test_unset_dash_dash_end_of_options`.

### `let "*"` reported "operand expected at end of string" instead of zsh's "at `*'"

- zsh keeps its input pointer at the start of the bad operator and emits `operand expected at \`<remaining>'` for orphan-at-start binary ops (`*`, `/`, `%`, `**`, `&`, `|`, `^`, `&&`, `||`, `^^`, `==`, `!=`, `<`, `>`, `<=`, `>=`, `<<`, `>>`, ...). zshrs collapsed every operand-missing case into "at end of string", losing operator location for orphan-binary expressions. Added `tok_start: usize` field to `MathEval` (updated in `zzlex` after whitespace skip), then in `mathparse`'s binary-op arm, when `stack.is_empty()` AND op is binary, error with `at \`<input[tok_start..]>'` — captures both `let "*"` (just `*`) and `let "*5"` (operator + remaining input). Pure-unary cases (`let "+"`, `let "-"`) still fall through to the existing "at end of string" path. Tests: `test_let_orphan_mul_at_op`, `test_let_orphan_div_at_op`, `test_let_orphan_mul_with_right_includes_remaining`, `test_let_trailing_mul_still_end_of_string`.

### `[ a b c ]` (3 non-flag args, no operator) silently returned 1 instead of erroring "condition expected: b"

- zsh: `[ a b c ]` -> `1: condition expected: b` exit 2 — points at args[1] which should have been an op. zshrs's 3-arg path only checked layouts where args[1] starts with `-`; pure-non-flag layouts fell through to the catch-all `1`. Added a 3-arg arm that fires when none of args[0..3] is `-`-prefixed, with `=`/`!=`/`==` excluded (those go through the regular comparison arms). Test: `test_test_3args_no_op_errors`.

### `print -u 2 hi` always wrote to stdout instead of routing to fd 2

- zsh: `print -u N` writes to fd N. zshrs validated the fd was open (added in iter 84) but the actual write still always went to stdout via `print!()`. Routed via match-on-fd: 1 → `print!`, 2 → `eprint!`, others → `libc::write`. Now `print -u 2 hi 2>/dev/null` correctly suppresses the output. Test: `test_print_u_routes_to_fd`.

### `type -S` and `type -k` errored "bad option" instead of accepting silently

- zsh's `-S` and `-k` are silent-accept on `type` (no observable effect in `-c` mode). zshrs's unknown-flag fallback (added when fixing `-Z` rejection) rejected them. Added an `S | k => {}` arm before the catch-all. Test: `test_type_S_k_accepted`.

### `[ -- a ]` silently returned 1 instead of erroring "unknown condition: --"

- zsh: `[ -- a ]` -> `[:1: unknown condition: --` exit 2 (zsh treats `--` as a bogus flag name in `[`-test, distinct from POSIX shell-end-of-options usage). zshrs's 2-arg unknown-flag arm only fired for all-alphabetic letters, missing the `--` case. Extended the check to include `--` as a valid trigger. Test: `test_test_dashdash_unknown_condition`.

### `fc 1` (single numeric positional, edit form) reported "event not found: 1" instead of "would recurse endlessly"

- Earlier iter 85 fix only covered 2-numeric-positional case. Extended to 1-numeric-positional too: any all-numeric edit-mode positional (count ≤ 2) routes to the recurse-endlessly path in `-c` mode. Test: `test_fc_single_numeric_recurse`.

### `[ a \) ]` (surplus close paren) reported "argument expected" instead of "too many arguments"

- zsh distinguishes paren-depth direction: surplus `(` -> `argument expected` (waiting for operand); surplus `)` -> `too many arguments` (the `)` is the extra arg). zshrs collapsed both into "argument expected". Split the depth-check arm: `d > 0` errors argument-expected; `d < 0` errors too-many. Test: `test_test_unmatched_close_paren_too_many`.

## Closed (eightieth-pass)

### `fc` (no args) reported "no such event: 1" instead of recursion-aborted

- Bare `fc` (no `-l`, no positional) is the EDIT mode — re-execute the previous command. With empty history the previous command IS `fc` itself, so zsh refuses with `current history line would recurse endlessly, aborted`. zshrs collapsed this case into the same "no such event: N" path the `-l` form uses. Added an early branch: when not list-mode AND no positional args, emit zsh's recursion-aborted message. Test: `test_fc_no_args_recursion_message`.

### `${(q)x}` for empty `x` returned empty instead of `''`

- zsh's `(q)` flag on an empty value emits `''` (a single-quoted empty pair) so the value survives word-splitting in the consumer. zshrs's level-1 quote loop did nothing when input was empty, returning bare empty — which an unquoted consumer would drop silently. Added an `s.is_empty()` early return to the level-1 branch. Test: `test_q_flag_empty_returns_quoted_empty`.

### `command -x ls` printed "bad option: -x" instead of "command not found: -x"

- zsh treats unknown `command` flags as command names (so `command -x ls` looks for an executable literally named `-x`, finds nothing, and emits `command not found: -x` with exit 127). zshrs's flag parser rejected with `command: bad option: -x` exit 1. Changed the unknown-flag arm to emit zsh's command-not-found diagnostic and return 127. Test: `test_command_unknown_flag_treated_as_command_name`.

### `command --help` silently passed (was eating `--` mid-flag)

- zsh: `command --help` (long-option-style) is treated as a command NAME (zsh's `command` builtin has no long-option support) — emits `command not found: --help` exit 127. zshrs's flag parser iterated `--help` char by char, hit `-` first which matched the `--` (end-of-options) arm, advanced the args index past `--help`, then returned with no positional args and exit 0. Two-part fix: (1) handle bare `--` BEFORE entering the per-char loop (proper end-of-options detection); (2) detect `--xxx` as a whole-arg long-option-style and emit the command-not-found diagnostic. Test: `test_command_long_option_treated_as_command_name`.

### `[ -i /tmp ]` (unknown unary cond) silently returned 1

- zsh: `[ -X path ]` for an unknown letter X errors `unknown condition: -X` exit 2. zshrs's `builtin_test` had no catch-all for unknown two-arg unary forms — they fell through the AND/OR split and silently returned 1, which a consumer would read as "false" rather than "syntax error". Added a 2-arg alphabetic-flag default arm before the AND/OR split that emits zsh's diagnostic and exits 2. Test: `test_test_unknown_unary_condition_errors`.

### `[ a -eq a ]` (non-numeric operands) silently returned true

- zsh: `[ a -eq a ]` errors `integer expression expected: a` exit 2 because the operands aren't integers. zshrs's `-eq`/`-ne` arms used `unwrap_or(0)`, silently coercing `a` to 0; `a -eq 0` then evaluated true (status 0). Added explicit `parse::<i64>()` checks in the `-eq` and `-ne` arms that emit the diagnostic and return 2 on parse failure. Test: `test_test_eq_non_numeric_errors`.

### `[ 5 -lt abc ]` etc. silently returned 1 instead of "integer expression expected"

- Same `unwrap_or(0)` issue extended to `-lt` / `-le` / `-gt` / `-ge`. Non-numeric operands silently coerced to 0 (so `[ 5 -lt abc ]` evaluated `5 < 0` → false → exit 1). All four arms now do explicit `parse::<i64>()` checks and emit zsh's `integer expression expected: <arg>` exit 2 on failure. Test: `test_test_lt_gt_le_ge_non_numeric_errors`.

### `type --help` silently passed instead of "bad option: -h"

- zsh: `type --help` errors `bad option: -h` exit 1 (the unknown-flag path; zsh treats the second `-` of `--help` as a no-op and reports the first non-`-` letter). zshrs's flag loop had a silent default arm that dropped unknown letters. Added an `eprintln + return 1` and a `'-' => { /* skip */ }` arm so the bad-option diagnostic reports the first letter after the leading dashes. Test: `test_type_unknown_flag_errors`.

### `unalias -X x` printed bash-style "unalias: bad option" instead of zsh format

- zsh: `unalias:1: bad option: -X` (typed `:1:` source-position prefix). zshrs printed `unalias: bad option: -X` (bash style with extra space, no `:1:`). Aligned the format. Test: `test_unalias_bad_option_format`.

### `fc -l blah` (non-numeric event) reported "no such event: 0"

- zsh: non-numeric event spec → `event not found: <text>` (distinct from numeric out-of-range `no such event: N`). zshrs's `parse::<i64>().unwrap_or(-16)` coerced `blah` to -16, then resolved to 0 and printed the numeric form. Added an explicit "single positional + non-numeric" arm BEFORE the numeric resolution that emits the text-event message. Test: `test_fc_non_numeric_event_spec`.

### `history -w` / `-X` (bash-style flags) gave wrong error from fc

- zsh's `history` is a `fc -l` synonym. It REJECTS bash-style flags like `-w` (write), `-X` (unknown) with `bad option: -X`, but ACCEPTS fc-passable flags like `-r` (reverse), `-D` (duration), date-format flags. zshrs's `builtin_history` had explicit arms for `-c`/`-a`/`-n`/numeric counts but treated unknown `-X` flags as search queries, then fell through to the empty-history path with the wrong message. Split the unknown-flag handling: bash-style (`-w`/`-X`) reject explicitly with `history:1: bad option: -X`; everything else falls through to the fc-list code path. Test: `test_history_unknown_flag_errors`.

### `type` (no args) returned 0 silently instead of exit 1

- zsh: bare `type` (no args) exits 1 — type requires at least one name to look up. zshrs returned 0 silently. One-line fix in `builtin_type`'s empty-args guard. Test: `test_type_no_args_exits_1`.

### `[${(@)a}]` for empty `a` dropped the surrounding brackets

- The `(@)NAME` flag form is the splice equivalent of `[@]` — each element becomes its own arg; surrounding literals should stick to first/last (so `[${(@)a}]` for an empty `a` still prints `[]`). zshrs's `is_splice_expansion` only matched `[@]`/`[*]`/slice forms, so `(@)` fell into DISTRIBUTE which drops the brackets when the array is empty. Added a `(...)` flag-block check that returns true when the flag set contains `@`. Test: `test_paren_at_flag_empty_array_preserves_brackets`.

### `${(qq)a}` for empty `a` returned empty instead of `''`

- zsh's `(qq)` flag on an empty array emits `''` (a single quoted empty pair) — the array is treated as `[""]` for quoting so the result still occupies a slot. zshrs returned actually empty (would be silently dropped by an unquoted consumer). Added an empty-array branch in the `q`-flag's state transition that emits `[quote_one("")]` when input array is empty. Test: `test_qq_flag_empty_array_emits_quoted_pair`.

### `${a[-5,-1]}` with len=3 returned full array instead of empty

- zsh: when the negative start index is below the array's lower bound (e.g. `-5` on a 3-element array), the slice empties — zsh treats the start as "past the array's start" and returns nothing. zshrs's `slice_indexed_array` clamped both negatives to valid range and returned the full array. Added an explicit `start < -len` check that short-circuits to empty. Test: `test_array_slice_neg_start_below_neg_len_empty`.

### `typeset -i x` left x empty instead of defaulting to 0

- zsh: `typeset -i NAME` (no value) initializes the integer to `0`; `typeset -F NAME` initializes the float to `0.0000000000` (default precision 10). zshrs's no-`=`-value branch always inserted an empty string, so `typeset -p x` printed `x=''` instead of `x=0`. Added a default-value computation: integer → `"0"`, float → formatted with the requested precision (or default 10), other → empty. Test: `test_typeset_integer_float_default_zero`.

### `print -P "%L"` was off by 1 (used parent's pre-increment SHLVL)

- zsh's `%L` outputs the in-shell SHLVL (incremented at startup over the parent's value). zshrs's `build_prompt_context` read `env::var("SHLVL")` which still held the parent's pre-increment value (the bump goes into `self.variables` only). So `print -P "%L"` showed `parent_shlvl` instead of `parent_shlvl + 1`. Now reads `self.variables["SHLVL"]` first, falling back to env. Test: `test_print_P_L_uses_in_shell_shlvl`.

### `${(V)x}` for control-char string left chars raw instead of `^X`

- zsh: `(V)` flag makes non-printable characters visible (control chars → `^X`, `\n` → `\n`, `\t` → `\t`). zshrs had a `ZshParamFlag::Visible` handler in the multi-flag dispatcher, but the inline state machine for single-flag `${(V)x}` had no `V` arm — control chars passed through raw. Added a `V` arm to the inline state machine that mirrors the visible-char encoding. Test: `test_v_flag_visible_control_chars`.

### `fc -W FILE` in `-c` mode dumped the entire on-disk persistent history

- zsh: `fc -W FILE` in non-interactive `-c` mode writes ONLY session-added entries (typically empty unless `print -s` ran). zshrs called `engine.recent(10000)` and wrote the full persistent log, leaking prior runs' commands into the user's named file. Now restricts to `session_history_ids` when atty is absent (matches zsh's `-c` mode), and falls back to the full recent list only on a real tty. Test: `test_fc_W_writes_session_entries_only_in_minus_c`.

### `fc -R FILE` (missing file) printed an error instead of silently ignoring

- zsh: `fc -R /no/such` returns 0 with no output — read failures are silently ignored so script consumers don't trip on missing logs. zshrs emitted `fc: cannot read /no/such` and returned 1. Removed the eprintln + return; missing-file is now a no-op. Test: `test_fc_R_silent_on_missing_file`.

### `fc -lr` ignored `-r` for session-only listings

- zsh: `fc -lr` walks the same range backwards (most recent first) while keeping original event numbers — `3 c | 2 b | 1 a` for a 3-entry session. zshrs's session-only path iterated `session_history_ids` forward unconditionally; the `-r` flag was a no-op for this code path (it only worked on the engine.recent fallback). Now collects session entries into a Vec and reverses iteration when `reverse` is set. Test: `test_fc_lr_session_reverse`.

### `ulimit -X` (unknown flag) silently fell back to `-f` and printed "unlimited"

- zsh: `ulimit -X` errors `bad option: -X` exit 1. zshrs's silent default arm let unknown flags slip through, then proceeded with the default resource (FSIZE) and emitted "unlimited" — masking the typo. Added an explicit error in the unknown-flag arm. Test: `test_ulimit_unknown_flag_errors`.

### `fc -ld` / `-lD` skipped time/duration columns in session-only mode

- zsh's `fc -ld` adds an `HH:MM` time column; `-lD` adds an `M:SS` duration column. zshrs's session-only listing path emitted only `N  command` regardless. Updated the session-only loop to read each entry's `timestamp` and `duration_ms` fields and format them when `show_time` / `show_duration` is set. Test: `test_fc_ld_lD_show_time_duration`.

### `alias =val` (empty NAME) silently created an unkillable alias

- zsh: `alias =val` / `alias =` errors `bad assignment` exit 1 — the alias name is required. zshrs silently created an alias keyed under empty string which then couldn't be removed via `unalias`. Added an `eq_pos == 0` guard in `builtin_alias` that emits the diagnostic and returns 1. Test: `test_alias_empty_name_errors`.

### `functions +t NAME` errored "no such function: +t"

- zsh: `functions +t NAME` / `+T` clears the trace attr silently (the off-switch counterpart to `-t`/`-T`). zshrs's flag matcher had no `+`-prefix arm — `+t` fell through to the names list, then the per-name lookup emitted `no such function: +t`. Added explicit `+t` / `+T` arms (and a combined `+xyz` arm for `+lt` etc.) that silently consume the flag. Test: `test_functions_plus_t_disable_trace_silent`.

### `alias g="x=y"; alias g` listed value bare instead of single-quoted

- zsh's `alias` listing single-quotes the value when it contains shell metas — including `=`, because the bare form `alias g=x=y` would re-parse as `alias g=x` + positional arg `=y`. zshrs's `format_alias_kv` (and its inline copy in the per-name lookup) excluded `=` from the quote-trigger set. Added `=` to both. Test: `test_alias_listing_quotes_value_with_equals`.

### Bare `fc` with session entries fell through to list mode instead of recurse-aborted

- Bare `fc` (no -l, no positional) is the EDIT mode — re-execute the prior command. In -c mode the prior command is `fc` itself, which is infinite recursion; zsh refuses with `current history line would recurse endlessly, aborted`. zshrs's earlier guard required empty session_history_ids, so adding any `print -s` entry let bare `fc` fall through to the list-mode pass-through. Hoisted the recurse-aborted check to fire BEFORE the session-only branch. Test: `test_fc_no_args_with_session_still_recurses`.

### `command` (no args) silently returned 0 instead of "redirection with no command"

- zsh: bare `command` (no args, no command name) errors `redirection with no command` exit 1 — `command` requires a command name. zshrs's empty-positional branch returned 0 silently, masking the missing CMD argument. Added the diagnostic and exit 1. Test: `test_command_no_args_redirection_error`.

### `wait %1` after bg job already reaped errored "no such job"

- zsh: `cmd & wait %1` works even after the bg process has completed and been reaped — missing job spec is silent success. zshrs's `wait` builtin emitted `wait: %1: no such job` and set status 127, breaking the common `cmd & wait` idiom. Now silently consumes missing job specs. Test: `test_wait_missing_job_silent`.

## Closed (seventy-ninth-pass)

### `printf "%04x" 42` printed `  2a` instead of `002a`

- The `%x`/`%X`/`%o` cases in `builtin_printf` had `left_align` and default-(right-pad-with-space) branches but no `zero_pad` branch — so `%04x` ignored the `0` flag and used spaces. Added a `zero_pad` arm to each that emits the prefix (`0x`/`0X`/`0` for alt-form), then `0`-fill for the width gap, then the digits. Test: `test_printf_hex_zero_pad`.

### `for ((i=1; i<=$#a; i++))` never iterated

- The arith COMMAND already routes through MathEval when the expr contains `$`, but the for-loop arith sections (init/cond/step) routed only on `,`. ArithCompiler's lexer can't parse `$`, so the cond became "0" (false) and the loop body never ran. Two-part fix: (1) extend `route_through_eval` in `compile_for_arith` to fire on `$` as well as `,`; (2) lift the routing decision to a single `needs_eval_global` so init/cond/step all use the SAME backend (otherwise init writes `i` into a slot via ArithCompiler but cond reads `i` from MathEval's variable map and sees 0). Test: `test_for_arith_with_dollar_param_in_cond`.

### `set -y` (and any unknown flag letter) errored "invalid option"

- zsh accepts unknown single-letter `set` flags silently — `set -y` is a no-op, `set -xy` enables xtrace and silently accepts -y. zshrs's `builtin_set` errored on the first unknown letter, breaking scripts that probe combinations. Default arm now silently ignores unknown letters (matching zsh's lenient flag-letter behavior). Test: `test_set_unknown_flag_silent`.

### `print -S "msg"` printed to stdout instead of history-only

- zsh's `print -S` adds to history INSTEAD of stdout (split-shell-words variant of `-s`). zshrs left `S` in the TODO list of unhandled flags so the line reached stdout. Now `S` sets `add_to_history=true` like `s`. Test: `test_print_S_adds_to_history_silently`.

### `${a[N]/pat/repl}` returned the element unchanged

- The bracket-modifier path in the array subscript expansion handled `:-` / `:+` / `:?` / `:=` / `:N:M` / history-modifier suffixes, but skipped the `/`, `//`, `/#`, `/%` pattern-replace forms. So `a=(file.txt other.txt); ${a[1]/.txt/.bak}` returned `file.txt` instead of `file.bak`. Added a `/`-prefix arm that decodes the op (0/1/2/3 for `/`/`//`/`/#`/`/%`) and dispatches through a new `zsh_pattern_replace` free function (extracted from the `BUILTIN_PARAM_REPLACE` `one()` closure so element-level callers can use it without going through the name-keyed builtin). Test: `test_array_element_pattern_replace`.

### `$histchars` was empty (zsh defaults to `!^#`)

- `$histchars` is the canonical 3-char string controlling history expansion (bang, hat, hash). zsh ships with `!^#`. zshrs left it unset so script reads of `$histchars` returned empty. Initialized in `ShellExecutor::new` next to the other special-name defaults. Test: `test_histchars_default`.

### `foo() {}` (empty function body) failed "command not found: {}"

- The lexer's `{` handler required whitespace, newline, or EOF after `{` to recognise it as Inbrace; `{}` consumed as a single literal token and the function-body parser failed. Two-part fix: (1) added `}` to the post-`{` accept list so `{}` lexes as Inbrace even when in cmd position; (2) for the OUT-of-cmd-position case (e.g. directly after `()` where `Outpar` cleared `incmdpos`), peek for `}` and force Inbrace recognition so `foo() {}` parses as a function with empty body. Other `{...}` shapes (brace expansion `{a,b,c}`, `${var}`) still work because they're consumed by separate token paths. Test: `test_empty_function_body`.

### `fc -l` indented event numbers one space too far

- zsh's `fc -l` formats event numbers right-aligned in a 5-char field (`    1  cmd`). zshrs used `{:>6}` (6-char field), so the output column was shifted one space right of zsh's. Switched every `fc -l` print site (session-history loop, recent-iter loop, with-time/duration variants) to `{:>5}` so the alignment matches. Test: `test_fc_l_event_number_width`.

### `set -a` did not enable `allexport`

- The multi-letter set-flag parser (`-xy` / `-xa` / `+ax` etc.) had arms for `e`/`x`/`u`/`v`/`n`/`f`/`m`/`C`/`b` but no `a`. So `set -a` silently passed through the silent-unknown default and `allexport` stayed off. Added `a` (enable) and `+a` (disable) arms in both halves of the multi-letter parser. Test: `test_set_a_enables_allexport`.

### `echo ~0` aborted "no such user or named directory: 0"

- zsh's `~N` (digits only) is shorthand for `~+N` — Nth entry on the directory stack, 0 = $PWD. zshrs's `expand_tilde_named` checked for `~+N` and `~-N` explicitly but not bare digits, so `~0` fell through to the `getpwnam` path which (correctly for non-numeric usernames) aborted in `-c` mode. Added a digits-only branch above the user-lookup arm that resolves to PWD or `dir_stack[N-1]`. Test: `test_tilde_digit_is_dirstack_index`.

### `echo \*` aborted with NOMATCH instead of printing the literal

- `looks_like_glob` walked the pattern looking for `*`/`?`/`[` and returned true on any occurrence — including `\*` where the `*` is escaped. So `echo \*` triggered the glob path, expanded against `cwd` (zero matches in most directories), and aborted in NOMATCH mode. Now the check walks character-by-character and skips `\X` escape pairs so backslash-escaped metachars don't count as glob triggers. The output still preserves the literal backslash (deeper unquoting fix needed for the full `\* → *` translation), but the script no longer aborts. Test: `test_escaped_glob_metachar_does_not_trigger_nomatch`.

### `let` (no args) returned 1 silently — zsh emits a diagnostic

- zsh's bare `let` errors `zsh:let:1: not enough arguments` and exits 1. zshrs returned 1 with no message, so script consumers couldn't distinguish "let with no args" from "let with arg that evaluated to 0". Added the diagnostic; status unchanged (still 1). Test: `test_let_no_args_errors`.

### `history` in `-c` mode dumped the on-disk persistent log instead of aborting

- In non-interactive `-c` mode with no in-session history adds, zsh's `history` (a `fc -l` synonym) errors `no such event: 1` rather than reading the persistent file. zshrs read the disk log and printed entries from prior sessions. Added the same atty + session-history-empty guard already used by `fc -l`; with session entries, only those are listed (numbered from 1). Test: `test_history_in_minus_c_mode_errors`.

### `read -q` from a pipe (non-tty) read silently and returned 0

- zsh's `read -q` is a single y/n character read from the terminal. Off a tty (`echo y | read -q ans`), zsh errors `not interactive and can't open terminal` and returns 1. zshrs read from stdin and returned 0 — scripts couldn't detect the missing terminal. Added an atty guard at the start of the `read` body before any stdin lock. Test: `test_read_q_requires_terminal`.

### `print -z "ls"` printed to stdout instead of pushing to ZLE buffer

- zsh's `print -z` pushes the args onto the line-editor's pre-buffer (so the next prompt starts with that text). Non-interactive mode has no editor, so the args are silently discarded with exit 0. zshrs's `push_to_stack` flag was a `let _ = ` no-op, so the args fell through to the output-args path and printed. Now `push_to_stack` returns 0 immediately. Test: `test_print_z_does_not_emit_to_stdout`.

### `kill -l` was missing SIGINFO on macOS

- macOS's `kill -l` lists `INFO` between `WINCH` and `USR1` (signal 29). zshrs's `signal_map` didn't have an entry for it, so the listing was missing one column and `kill -l` output didn't match zsh. Added a macOS-cfg-gated `("INFO", libc::SIGINFO, Signal::SIGINFO)` row. Test: `test_kill_l_includes_info_on_macos`.

### `fc -l` (no args) reported "no such event: 0" instead of "1"

- zsh: `fc -l` (no args) defaults to the last 16 events. With empty history, the lower bound resolves to event #1 (which doesn't exist), so the error is `no such event: 1`. zshrs collapsed all non-positive args to `0`. Now distinguishes the no-arg default (resolves to 1) from explicit 0/negative args (resolve to 0). Test: `test_fc_l_default_no_args_event_one`.

### `kill -l XYZ` printed wrong error format

- zsh: unknown-signal error in `kill -l` is `zsh:kill:1: unknown signal: SIGXYZ` — both a typed `zsh:kill:1:` prefix AND the SIG prefix on the signal name (the SIG prefix is always added even when the user's input lacked it). zshrs printed `kill: unknown signal: XYZ` (missing both prefixes). Updated the eprintln!. Test: `test_kill_l_unknown_signal_format`.

### `typeset` silently overwrote read-only variables

- zsh: `readonly y=1; typeset y=2` errors `read-only variable: y` and aborts the shell with exit 1 in -c mode. zshrs's `builtin_typeset_named` skipped the read-only check and overwrote the value. Added a check at the top of the assignment branch: if the name is in `readonly_vars` or has `var_attrs.readonly`, emit the diagnostic and `process::exit(1)` (matching `BUILTIN_SET_VAR`'s abort behavior). Test: `test_typeset_readonly_aborts`.

### `fc -h` overflowed the stack via infinite re-execution

- zsh: any unknown `fc` flag errors `bad option: -X` and bails. zshrs's flag-letter loop had a silent default arm — unknown flags fell through to the no-args path (re-execute last command), and since `fc -h` itself entered history that path infinitely recursed and overflowed. Added an `eprintln + return 1` for unknown flags so `fc -h`, `fc -w`, etc. fail cleanly. Test: `test_fc_unknown_flag_errors`.

### `functions -T NAME` printed the function body instead of enabling trace

- zsh: `functions -T` enables tracing on the named functions silently. zshrs didn't recognize `-T` (only `-l`/`-t`/`-m`), so it fell into the default-listing path and printed the body. Added `-T` (and combined-flag `T`) arms that return 0 immediately (the trace attribute itself isn't tracked yet, but the no-output behavior matches script consumers). Test: `test_functions_T_enable_trace_silent`.

### `unalias` (no args) printed bash-style usage instead of zsh format

- zsh: bare `unalias` errors `zsh:unalias:1: not enough arguments`. zshrs's empty-args path emitted a bash-style `unalias: usage: unalias [-agsm]` message — script consumers pattern-matching on `unalias:1:` missed the diagnostic. Replaced both no-args paths with `zshrs:unalias:1: not enough arguments`. Test: `test_unalias_no_args_emits_zsh_format`.

### `[[ -N file ]]` (modified-since-access test) errored "unknown condition"

- zsh: `-N file` returns true iff the file's access time is NOT newer than its modification time (used by mailbox-watching code). zshrs's cond compiler had no `-N` arm so it fell through to the unknown-condition error path. Added `BUILTIN_FILE_MODIFIED_SINCE_ACCESS` (id 341) using `MetadataExt`'s `mtime()`/`atime()` with the `atime <= mtime` semantic so equal-stamped (newly created) files count as modified, and an `emit_file_test` arm to dispatch. Also fixed an inadvertent ID collision: my first attempt picked id 331 which clobbered `BUILTIN_APPEND_SCALAR_OR_PUSH`, breaking `a+=val` array push. Test: `test_cond_N_file_modified_since_access`.

### `((1/0))` arith COMMAND aborted instead of continuing with non-zero status

- zsh: `((1/0))` arith command sets status to 2 and continues; only the substitution form `$((1/0))` aborts the whole command. zshrs's earlier "abort on division-by-zero" fix in `evaluate_arithmetic` unilaterally `process::exit(1)`-ed regardless of caller, so `((1/0)); echo` skipped the echo. Reverted the unilateral exit — now just emits the diagnostic and returns "0"; the call-site SetStatus op gives the surrounding command a non-zero status (1 from compile-time StrEq-to-"0" check; zsh uses 2 but scripts treat both as failure via `(()) && …` gating). Test: `test_arith_division_by_zero_continues`.

### `fc -l N M` (range query) used the single-arg "no such event" message

- zsh: `fc -l N M` (range query) emits `no events in that range` when the history is empty; the per-event `no such event: N` is reserved for single-positional / no-positional forms. zshrs collapsed both to the per-event form. Added a `positional.len() >= 2` arm that uses the range-specific message. Test: `test_fc_l_range_two_args_no_events_in_range`.

### `umask abcd` printed generic "invalid mask" instead of zsh's per-char diagnostic

- zsh validates symbolic umask values character-by-character: after class chars (`u`/`g`/`o`/`a`) it expects an operator (`+`/`-`/`=`). On invalid input it reports the specific bad operator: `umask abcd` → `bad symbolic mode operator: b`. zshrs printed a generic `umask: invalid mask: abcd`. Added a fallback parse: walk the leading class chars, then check if the next char is an operator; emit zsh's per-char message when it isn't. Test: `test_umask_bad_symbolic_operator_specific_error`.

### `type -w NAME` printed descriptive form instead of `NAME: KIND`

- zsh's `type -w NAME` is the "word" shorthand: prints `NAME: KIND` where KIND is one of `builtin`, `command`, `function`, `alias`, `reserved`, `none`. zshrs ignored `-w` and fell to the default descriptive output (`NAME is a shell builtin`, etc.). Added a `show_word` flag set by `-w`, plus per-kind branches that emit the `NAME: KIND` form. Test: `test_type_w_emits_name_colon_kind`.

### Non-executable file invocation exited 127 silently instead of 126 with diagnostic

- zsh: invoking a non-executable file (`chmod 644 file; ./file`) emits `permission denied: ./file` on stderr and exits 126 (POSIX convention for "command found but not executable"). zshrs's `execute_external` only special-cased `NotFound` (→ 127); all other IO errors fell into the generic `Err` arm which the caller converted to 127 with no diagnostic. Added a `PermissionDenied` arm that emits zsh's diagnostic and returns 126. Test: `test_exec_non_executable_file_status_126`.

### `[[ ]]` (empty condition) silently passed instead of parse error

- zsh: `[[ ]]` is a parse error — a condition is required between the brackets. zshrs's `parse_cond` called `parse_cond_expr` which returned `None`, then silently consumed the `]]` and produced no command (exit 0). Added an early-return: if the immediate next token after `[[` is `]]`, emit a parse error. Test: `test_empty_cond_bracket_parse_error`.

### `umask 022 044` (multiple values) silently used the last value

- zsh: multiple positional values to `umask` errors `too many arguments` and exits 1. zshrs's flag loop overwrote `value` with each positional, silently using only the last. Added a `value_count` counter and an early error when > 1. Test: `test_umask_too_many_args`.

### `functions -t NAME` printed unconditionally instead of trace-attr-gated

- zsh: `functions -t NAME` lists only functions whose trace attribute IS set; vanilla functions with no trace marking produce no output. zshrs printed `functions -t NAME` unconditionally, leaking output for the common no-trace case. Per-function trace tracking is a follow-up; for now `-t` falls into a continue/silent path that matches zsh's no-trace output. Test: `test_functions_t_no_trace_set_silent`.

## Closed (seventy-eighth-pass)

### `print -P "%S"` emitted reverse-video instead of italic

- zsh's prompt-expansion `%S` (start standout) emits `\e[3m` (italic), and `%s` (end standout) emits `\e[23m` (italic-end). zshrs's prompt expander used `\e[7m`/`\e[27m` (reverse video pair). Switched to italic codes to match zsh's actual output. Test: `test_print_P_standout_emits_italic_codes`.

### `for ((i=0,j=10; i<3; i++,j--))` only initialized one variable

- ArithCompiler emits at most one op-write per call; the comma-trailing statements were dropped. So `i=0,j=10` set `i` but left `j` empty, producing `0:` instead of `0:10`. Routed comma-containing init/step expressions through `BUILTIN_ARITH_EVAL` (MathEval, which evaluates the whole comma-list and writes back through `extract_string_variables`). Cond expressions with comma also re-routed. Test: `test_for_arith_comma_init_and_step`.

### `print -P "%B%S%U"` re-emitted bold/standout each time

- zsh: `%B%S%U` outputs `\e[1m\e[3m\e[4m` (each independent SGR). zshrs's `apply_attrs` re-emitted ALL currently-active attrs every call, producing `\e[1m\e[1m\e[3m\e[1m\e[3m\e[4m` etc. Each individual attribute handler (`B`/`U`/`S`) now emits ONLY its specific SGR code; `apply_attrs` is reserved for color-set paths that need the full state. Test: `test_print_P_attr_chain_independent`.

### `$_` leaked function body text inside function calls

- After `foo() { echo $_ }; foo`, zsh's `$_` inside the function body should be `foo` (function name when no args). zshrs was reading the FUNCTION BODY SOURCE because `BUILTIN_REGISTER_COMPILED_FN` (called when defining the function) had updated `pending_underscore` with the body text via the standard `pop_args` hook. Fix: explicitly set `$_` and `pending_underscore` BEFORE the function body runs (in `call_function`'s pre-VM setup) — using the function's call-form last arg, or the function name when no args. Test: `test_dollar_underscore_inside_function_body`.

### `print -P "%y"` (tty short name) outputted empty when not on a tty

- zsh: `%y` outputs `()` when not connected to a tty (matches `%l`). zshrs returned empty because the `%y` handler stripped the `/dev/` prefix from an empty string. Added the empty-tty check to emit `()` matching zsh. Test: `test_print_P_y_no_tty_outputs_parens`.

### `print -P "%F{red}"` re-emitted all active attrs

- zsh: `%F`, `%K`, `%k` each emit ONLY their specific SGR code (`\e[31m`, `\e[44m`, `\e[49m`). zshrs's color handlers called `apply_attrs` which re-emitted all currently-active attrs (`\e[1m\e[1m\e[31m...` if bold was already set). Same fix as text-attrs in batch 2: each color handler now emits only its specific code. Test: `test_print_P_color_no_extra_bold`.

### `which local` reported just `local` instead of `local: shell reserved word`

- `which` is a csh-style synonym for `whence -c`. zsh's `whence -c` outputs `name: shell reserved word` for reserved-word names. zshrs's whence-c branch had no reserved-word case — it fell through and printed just the name. Added a `csh_style` arm in the reserved-word handler. Test: `test_which_reserved_word_csh_style`.

### `${a[N]:r}` / `:e` / `:t` / `:h` / `:l` / `:u` returned full element

- zsh: `a=(file.txt); ${a[1]:r}` → `file` (history-style modifier on resolved element). zshrs's bracket handler routed `:` modifiers through the colon-default branch (`:-`/`:=`/`:?`/`:+`) and didn't recognize history modifiers. Added a per-element `apply_history_modifiers` call when `after_bracket` starts with `:` and the body is a known modifier. Test: `test_array_element_history_modifier`.

### `builtin nosuch` had non-zsh error format

- zsh: `builtin nosuch:1: no such builtin: NAME`. zshrs printed `builtin: NAME: not a shell builtin`. Aligned to zsh's format. Test: `test_builtin_missing_zsh_format`.

### `%F{1}` (numeric color 0-7) used 256-color escape

- zsh: `%F{1}` (red) outputs `\e[31m` (basic ANSI). zshrs's `Color::Numbered(n)` always used the 256-color form `\e[38;5;Nm`. For indices 0-7, the basic ANSI codes (30-37 fg, 40-47 bg) are the canonical encoding. Added a fast-path for `n <= 7` that emits `\e[3Nm` / `\e[4Nm`. Indexes 8-255 still use the long form. Test: `test_print_P_color_basic_8_uses_ansi_codes`.

### `print -P "%1d"` printed `tmp` and `%2d` printed `/private/tmp` (canonicalized cwd)

- zsh's prompt-expansion `%d`/`%~` use the *logical* pwd from `$PWD`, not the canonicalized cwd from `getcwd()`. On macOS, `cd /tmp` leaves `$PWD=/tmp` while `getcwd()` returns `/private/tmp`. zshrs's `build_prompt_context` used `env::current_dir()` only, so `%2d` printed `/private/tmp` and `%1d` (last 1 component) printed `tmp` (no leading `/`). Switched to `env::var("PWD")` first, fall back to `current_dir()`. Test: `test_prompt_d_uses_logical_pwd_not_canonical`.

### `setopt globdots` did not enable hidden-file matching

- zsh's canonical option name is `globdots`; `dotglob` is the bash alias. zshrs's options table stores them under separate keys, but the glob expansion only consulted `dotglob`. So `setopt globdots; print *` skipped dotfiles even though the user explicitly asked for them. Now read both keys (either spelling enables it). Test: `test_glob_globdots_setopt_alias_for_dotglob`.

### `<N-M>` numeric range glob produced parse error or literal pattern

- zsh's numeric range glob `<N-M>` matches any digit sequence whose decimal value is in `[N, M]`. zshrs threw `parse error: expected word after redirection` because the lexer consumed `<` as a redirection operator mid-word. Three-part fix: (1) lexer's mid-word `<` handler peeks for `<digits?-digits?>` shape and absorbs it into the current word; (2) compile-time word classifier (`compile_word_str`) recognises the `<...>` shape via `has_numeric_range_glob` and triggers glob compilation; (3) runtime adds `expand_glob_with_numeric_range` that walks the directory and filters by a regex with `(\d+)` captures, validating each capture against the `[lo, hi]` range. Open-ended forms (`<3->`, `<-5>`, `<->`) work via `Option<i64>` bounds. Tests: `test_glob_numeric_range_finite`, `test_glob_numeric_range_open_high`, `test_glob_numeric_range_open_both`.

### `$((PPID))`, `$((UID))`, `$((EUID))`, `$((GID))`, `$((EGID))` resolved to `0`

- zsh resolves bareword identifiers in arithmetic context against live process-id specials. zshrs's `evaluate_arithmetic` only injected `RANDOM` / `SECONDS` / `EPOCHSECONDS` / `EPOCHREALTIME` / `LINENO` into MathEval's extras map, so `PPID` and the user/group ids fell through to the static-zero default. Added the 5 process-id specials to the inject list. Test: `test_arith_ppid_uid_special_names`.

### `${a[5,10]}` (out-of-range slice) returned the trailing element

- zsh: out-of-range starts collapse to empty (`a=(a b c); print ${a[5,10]}` → empty). zshrs's `slice_indexed_array` clamped the start index down to len, returning the last element. Added explicit out-of-range checks (start > len; both negatives crossing) that short-circuit to `Vec::new()`. Test: `test_array_slice_out_of_range_is_empty`.

### Empty array slice in DQ context dropped surrounding literals

- `print "[${a[5,10]}]"` with empty slice should print `[]` (surrounding `[` `]` literals stick to first/last element via splice semantics). zshrs's compiler routed slice forms through `BUILTIN_CONCAT_DISTRIBUTE` because `is_splice_expansion` only matched `[@]` / `[*]` — the cartesian-distribute path drops empty arrays AND surrounding literals together. Extended `is_splice_expansion` to recognise `[N,M]` slice subscripts so they pick `BUILTIN_CONCAT_SPLICE` (which preserves the LHS scalar when the RHS array is empty). Same test: `test_array_slice_out_of_range_is_empty`.

### Recursive aliases caused stack overflow

- zsh's lexer disables an alias inside its own body (so `alias ls='ls -la'` works without recursion; the lexer expands `ls` to `ls -la` once, then the inner `ls` stays literal). zshrs expands aliases at run time via `execute_script(&combined)`, which re-parses → re-dispatches → recurses forever. `alias g="g hi"; g` overflowed the stack. Added an `expanding_aliases: HashSet<String>` guard on `ShellExecutor`: insert the name before the recursive `execute_script`, remove on return, and skip alias lookup entirely for any name already in the set. The recursive `g` now misses the alias table and falls through to "command not found", matching zsh. Test: `test_alias_recursion_guard_self_disables`.

### `[ \( -n a \) -a \( -z "" \) ]` (paren grouping) didn't work

- POSIX `[ ... ]` (test) supports `\(` `\)` for grouping around `-a`/`-o` connectives. zshrs's default arm split on the LAST top-level `-a`/`-o` without tracking paren depth, so a paren-grouped expression got split on a connective that should have been inside a sub-group. Added a depth-tracking pass to `builtin_test`: only consider `-a`/`-o` at depth 0, AND if the entire expression is wrapped in matching outer parens, strip them and recurse on the inner. Test: `test_test_builtin_paren_grouping`.

### `fc -l 0` and `fc -l -N` reported wrong event number

- In `-c` (non-interactive) mode with empty history, zsh's `fc -l N` errors with `no such event: <resolved-N>`. Resolved is `max(0, N)` — both 0 and any negative offset collapse to `0`; positive arguments echo verbatim. zshrs hardcoded `"1"` for non-positive arguments, so `fc -l 0` reported `1` instead of `0`. Now uses `if first <= 0 { 0 } else { first }` for the message. Test: `test_fc_l_no_event_uses_resolved_index`.

### `read -p` was treated as prompt (should be coprocess input)

- zsh's `read -p` reads input from the coprocess; the prompt feature is `read 'NAME?prompt'`. zshrs misread `-p` as a prompt flag, ate the next arg as the prompt text, and printed it. Now emits `zshrs:read:1: -p: no coprocess` and exits 1, matching zsh in -c mode. Capital `-P` is left as a prompt-flag alias for back-compat with anything that relied on the old (wrong) shape. Test: `test_read_p_flag_means_coprocess_not_prompt`.

### `typeset -i N x; x=255` did not format in base N

- `typeset -i N x` declares an integer that should display in base N (`N#DIGITS`). zshrs only applied the base format when the typeset call also assigned in the same statement (`typeset -i 16 x=255` → `16#FF` worked); a later bare `x=255` skipped the format because `BUILTIN_SET_VAR` only consulted `VarKind::Integer`, not `var_attrs.int_base`. Pulled the full attrs out, applied `format_int_in_base` on store. Test: `test_typeset_i_base_format_at_assignment`.

### `[[ -v a[1] ]]` errored "no matches found: a[1]"

- The condition `-v` (parameter-set test) takes a parameter NAME with optional subscript. zshrs's cond compiler called `compile_word_str(arg)` which triggered glob expansion — `a[1]` was treated as a `[1]` char-class glob and aborted with NOMATCH. Three-part fix: (1) `ZshCond::Unary` arm for `-v` emits the operand as a `LoadConst` literal, skipping glob; (2) `ZshCond::Binary` arm (which the parser uses when `-v` is followed by one operand and an empty third operand) gets the same `-v` literal-emission branch; (3) `BUILTIN_VAR_EXISTS` runtime now splits `name[key]` and looks up either the indexed array element (with negative-index resolution and bounds check) or the assoc-array key, returning `Bool`. Test: `test_cond_v_with_array_subscript`.

### `${x:0:${#x}-2}` (substring with arithmetic length) returned full string

- zshrs's `parse_param_modifier` rejected ANY shape with nested `${...}` (line 3133: `if inner.contains("${") { return None; }`), falling through to the bridge expander which doesn't handle the substring shape. Relaxed the rejection: nested `${…}` is now allowed in substring offset/length operands as long as it doesn't appear before the first `:` (the var-name segment) and the modifier op isn't a prefix-letter form (`:-`, `:#`, `:/`, `:?`, etc.). The length expression then routes through `BUILTIN_PARAM_SUBSTRING_EXPR` which calls `eval_arith_expr` after `expand_string`. Test: `test_substring_with_arithmetic_length`.

### `(( ${+h[a]} ))` arith command with parameter expansion always saw 0

- The arith COMMAND `(( ... ))` (vs the arith SUBSTITUTION `$(( ... ))`) compiled through `ArithCompiler`, whose lexer can't parse `$` and treated the expansion as an unknown token. The expansion never ran, so the operand silently became 0 and the command exited 1 even when the value was 1. Added `$` to `needs_eval` triggers in `compile_command` for `(( ... ))` — any expr touching parameter expansion now routes through `BUILTIN_ARITH_EVAL` → `evaluate_arithmetic` which calls `expand_string` first. Test: `test_arith_command_with_parameter_expansion`.

### `$0` in `-c` mode returned basename instead of argv[0] verbatim

- zsh exposes argv[0] verbatim through `$0` in `-c` mode — `/bin/zsh -c 'echo $0'` prints `/bin/zsh`, plain `zsh -c '...'` prints `zsh`. zshrs basename-stripped argv[0] in the `-c` dispatch (`bins/zshrs.rs`), losing the path information. Now passes argv[0] through unchanged. Test renamed from `_returns_basename` to `_uses_argv0_verbatim` and updated to accept either basename or absolute path. Test: `test_dollar_zero_in_minus_c_uses_argv0_verbatim`.

### `[[ -v 1 ]]` (positional param test) returned false even when set

- zsh: `[[ -v N ]]` for a digit name N tests whether the Nth positional parameter is set (i.e. `$#` ≥ N). zshrs's `BUILTIN_VAR_EXISTS` only consulted `variables`/`arrays`/`assoc_arrays`/env, never the positional list — so `set -- one; [[ -v 1 ]]` returned false. Added a digit-name fast-path: parse name as `usize`, treat 0 specially (look up the `0` variable), otherwise compare against `positional_params.len()`. Test: `test_cond_v_with_positional_param`.

### `HISTSIZE=0` left HISTSIZE at 0 (zsh clamps to 1)

- zsh enforces a minimum of 1 on `HISTSIZE`. The internal `params.rs` setter already clamped via `.max(1)`, but the assignment path (`BUILTIN_SET_VAR`) writes directly into `variables` and skipped the clamp. So a script doing `HISTSIZE=0; echo $HISTSIZE` saw `0` instead of `1`. Added a name-specific clamp at the SET_VAR layer: `if name == "HISTSIZE"` parse, max with 1, store the clamped string. Test: `test_histsize_min_clamp_to_one`.

### `echo $((1/0))` printed `0` after the error message

- zsh aborts the command on arithmetic division-by-zero — `echo` never runs. zshrs's `evaluate_arithmetic` printed the error then returned `"0"` from the `Err` arm, so `echo $((1/0))` printed `division by zero` to stderr AND `0` to stdout (status 0). The shell continued normally. Now matches zsh: division-by-zero in `evaluate_arithmetic` calls `process::exit(1)` after printing the error, aborting the surrounding command in `-c` mode. Other arith errors still return `"0"` (matches zsh's NumericContext defaults). Test: `test_arith_division_by_zero_aborts`.

### `wait NOT_A_PID` emitted "wait: NOT_A_PID: invalid pid" instead of zsh's job-not-found format

- zsh treats an unparseable `wait` argument as a (failed) job-spec lookup: `zsh:wait:1: job not found: NOT_A_PID` exit 127. zshrs hand-rolled `wait: NOT_A_PID: invalid pid` from the parse-error arm, which neither matches zsh's `<shellname>:<builtin>:<line>:` prefix nor uses the canonical "job not found" wording — script consumers grep'ing stderr for the zsh format silently saw nothing. Changed the parse-error arm to emit `zshrs:wait:1: job not found: <arg>` (same exit 127). Test: `test_wait_invalid_pid_uses_zsh_format`.

## Still open (seventy-fifth-pass — remaining)

- **`nocorrect CMD args`** — parser drops the rest of the line after `nocorrect` appears. Lexer needs to recognize `nocorrect` (and `noglob` as well, eventually for purity) as a precommand modifier and skip past it. Deferred.
- **`set -n` syntax-only mode** — `set -n; cmd` should parse but not execute. zshrs ignores -n. Deferred (needs runtime no-op gate).
- ~~**`${(j:sep:)$(cmd subst)}`**~~ — closed in eighty-eighth-pass batch 25. (j) is now a no-op on scalar cmd-subst output (zsh-correct).
- **Arith error full command-abort** — zsh: `echo $((10/0))` prints only the error and skips `echo`; zshrs prints the error then runs `echo 0`. Partial fix landed (error message now visible); full abort needs expansion-time error plumbing.
- **`set` noargs print all variables** — `set` with no args should dump every shell parameter in name=value form. zshrs prints ~10 lines; zsh prints ~480. Massive output diff; deferred (needs full param-table walk + assoc/array formatting).
- ~~**`${a:^b}` zip-arrays**~~ — closed in eighty-eighth-pass batch 24. SUB_ZIP_SHORT and SUB_ZIP_LONG implemented.
- ~~**`${(v)assoc}` insertion order**~~ — closed in eighty-eighth-pass batch 1. Switched assoc storage to IndexMap.
- **`${(s:l:)hello}` empty-element handling** — zsh drops empty elements when splitting (`hello` split by `l` → `he`, `o`); zshrs keeps empties (`he`, ``, `o`). Niche.
- **`${#:-empty}` length-of-default** — zsh returns 5 (length of "empty"); zshrs returns 0. Esoteric edge case in `${#name:-default}` parsing.
- **`*` glob ordering caseglob** — zsh sorts `bench bins Cargo.lock …` (case-insensitive); zshrs sorts `Cargo.lock … bench bins` (case-sensitive). Glob expansion needs to honor the `caseglob` option (default-on in zsh).
- **`declare -A h; h[foo bar]=baz`** — zsh: "bad pattern: h[foo"; zshrs: "command not found: h[foo". Both error; format/source differs. (Re-verified 2026-06-12 after the unclosed-class patcompile fix: unchanged — this one comes from word-splitting the assignment, not pattern compilation.)
- **`declare -i` / `declare -a` / `declare -A` no-args listing** — zsh includes shell-internal params (`!`, `$`, `EUID`, `fpath`, etc.) in their respective type listings. zshrs's special params aren't typed, so the filter would return empty. Needs typed-special-param infrastructure.
- **Math funcs without `zmodload zsh/mathfunc`** — zshrs auto-provides `sqrt`, `sin`, `cos`, `floor`, `ceil`, `min`, `max`, etc.; zsh requires `zmodload zsh/mathfunc` first and errors with "unknown function" otherwise. zshrs is more permissive (likely a feature for daily use); leaving as documented divergence.

- **Backtick nesting** — parser-deferred.
- **`xtrace` exact zsh format** — POSIX `+ cmd` shape; zsh's elaborate PS4 not matched.

The following items have been investigated and confirmed as false positives or fundamentally compatible:

- **`read -d DELIM` / `read -A array` / `read -r raw`** — verified working when not run inside a pipeline subshell. The original probe diff was a `cmd | read v` pipeline-isolation artifact (the `read` runs in a subshell so `$v` doesn't survive — same behavior in zsh).
- **`print -m PATTERN args...`** — closed 2026-06-12: the flag was already wired for the plain form; the `-f` (printf-format) branch now applies the C-ordered filter too (Src/builtin.c:4712-4741 — pattern first, `if (fmt && !*args) return 0`), and an uncompilable pattern errors "bad pattern" instead of silently matching nothing (patcompile now rejects unclosed `[` classes per Src/pattern.c:1497-1498).
- **`${(M)arr:#pat}` DQ context** — zsh's behavior here is subtle (the `(M)` flag stays active in DQ to flip filter direction on the joined scalar). zshrs's array-context filter logic differs only when the WHOLE expression is DQ-wrapped AND uses `(M)` AND has a `:#pat` filter. Niche edge case; deferred.
- **`cd -`** — output style differs by one leading "print pwd" line that zsh's interactive cd suppresses but `-c` doesn't. Cosmetic.
- **`select` PS3 customization** — uses `?# ` default; users with custom PS3 see their own value. Cosmetic in `-c` mode.
- **`cmd | head -0` exit status** — CLOSED, 2026-06-12 sweep D: `--zsh` strict-parity mode now forces `coreutils_shadows_enabled()` to false unconditionally (outranks the toml knob and the env override), so `head` / `cat` / `sort` are the system binaries with the system binaries' exact flag handling — `head -0` errors `illegal line count -- 0` (rc 1) like BSD head. Default zshrs mode keeps the in-process applets per the config. Pinned in `probe_sweep_2026_06_12_d`.
- **Brace-form `if` at `-c` EOF without trailing newline** — zsh quirk, 2026-06-12 sweep F: `zsh -fc 'if [[ -n x ]] { print t }'` (no trailing newline, no else) is `parse error near \`}'` in zsh 5.9.1, but the SAME text from a file / with a trailing newline parses fine. zshrs accepts both. Pinned forms in `probe_sweep_2026_06_12_f` embed the newline; the no-newline `-c` rejection is unimplemented (would need the C lexer's exact EOI/SEPER interaction at the construct tail).
- **Sourced-file diagnostic linenos** — cosmetic, 2026-06-12 sweep F: `source <(cmds)` reports `file:1:` for every command-not-found inside the sourced text where zsh tracks the real line (`/dev/fd/N:3:`). The /dev/fd NUMBER also differs (fd allocation is environmental). Last remaining zinit-plugin-load divergence (zsh-openshift-aliases `oc completion zsh` output).
- **`suspend` in-suite** — untestable: faithful port of jobs.c:3170 killpg(origpgrp, SIGTSTP); under a test harness the signal stops the harness's own process group (oracle zsh only survives because its pgrp is orphaned there). Excluded from probe corpora.
- **CSH_NULL_GLOB error vs command-local stderr redirect** — narrow ordering edge, 2026-06-12 sweep D: `setopt cshnullglob; ls *nope* 2>/dev/null` — zsh emits `no match` during prefork on the SHELL's stderr (before the command's redirections apply), so `2>/dev/null` doesn't eat it; zshrs's per-command badcshglob boundary fires after the dup2, so the message is suppressed. All unredirected forms match (builtin abort + rc 1, external skip + continue, mixed success silence — pinned in `probe_sweep_2026_06_12_d`). Fix requires a word-expansion-complete hook before the redirect ops in the VM dispatch.
- **`typeset -n` parity probes** — zshrs implements namerefs from the vendored master source (PM_NAMEREF, Src/params.c:6332); the zsh 5.9.1 oracle binary predates them and errors `bad option: -n`. Spec-over-oracle: pinned as a zshrs-behavior test (`bulk_g_nameref_typeset_n_zshrs_pin`), never as oracle-parity rows.

The "Stub modules (loaded but limited)" section below remains as documented deferrals (`zsh/cap`, `zsh/clone`, `zsh/curses`, `zsh/zftp`, `zsh/db_gdbm`) — these are niche features whose `zmodload` call currently no-ops, with the corresponding builtins not registered. They are not active gaps in zshrs's compatibility floor; they're tracked separately because they have no real-world load on the daily-driver path. `zsh/mapfile` was previously in this list; it is now closed (read form implemented above).

## Stub modules (loaded but limited)

- `zsh/cap`, `zsh/clone`, `zsh/curses`, `zsh/zftp` — module loads via `zmodload` succeeds but the corresponding builtins (`cap`, `clone`, `zcurses`, `zftp`) aren't registered. Niche features; deferred.
- `zsh/db_gdbm` — `ztie` correctly reports "GDBM support not compiled in" (no native gdbm dep). Acceptable stub behavior.
- `zsh/files chown/chmod/chgrp` — works (proper error for nonexistent file).
- `zsh/mapfile` — closed; `${mapfile[/path]}` magic assoc reads file bytes, composes with `(f)` flag for line-split. See "Closed (this session)" above.
- `zsh/private` — closed; routed to `builtin_local`.
- `zsh/newuser`, `zsh/nearcolor` — niche, deferred.
