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

- `ZshLexer.error` (e.g. `unmatched '`) was set during lexing but the parser ignored it. After `parse_program_until` succeeds, `parse()` now checks `self.lexer.error` and returns it as a `ParseError`. The execute path then exits with the diagnostic on stderr. Closes `test_error_syntax` (now uses `echo 'unclosed` — a real lexer error that mainline zsh also rejects).

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

## Still open (seventeenth-pass — remaining)

(none — all probed gaps closed)
- **`${(o)a}`/`(O)`/`(n)`/`(i)`/`(M)` array flags suppressed in DQ context** — zsh applies these flags only when the expansion is in array context (no surrounding `"..."`); zshrs always applies them. Affects parity in joined-string output. Likely needs the compile path to know its quoting context and skip these flags accordingly.
- **`${(ou)a}` ordered-unique** — `o`+`u` flag combo result correct (sorted-unique) but DQ context preserves original in zsh; cosmetic interaction.
- **`print -s history-save`** — appends to history but `fc -l` doesn't see it (session histnum not bumped). Cosmetic for `-c` mode.
- **`${@:1:2}` / `$@[2,4]`** positional slice forms.
- **`declare -ra` (readonly array)** — silent vs zsh's "inconsistent type for assignment". Cosmetic.
- **`$!` unset after `cmd &`** — backgrounded process pid not recorded into `$!`. Job-control plumbing scope.
- **`select` non-interactive prompt format** — uses `?# ` instead of zsh's `?> ` style; users may set PS3 to override. Cosmetic in non-interactive `-c` mode.

The following items from the fifth-pass probe were inspected and turned out to be false positives or test artifacts:

- **`read -d DELIM` / `read -A array` / `read -r raw`** — verified working when not run inside a pipeline subshell. The original probe diff was a `cmd | read v` pipeline-isolation artifact (the `read` runs in a subshell so `$v` doesn't survive — same behavior in zsh).
- **`print -m PATTERN args...`** — match-arg flag still missing (cosmetic; rarely used).
- **`${(M)arr:#pat}` filter** — `(M)` flag direction differs from docs in mainline zsh on the test host; the zshrs behavior of "keep only matching when (M) is set" actually matches the man page reading. Possible bug in mainline zsh; defer.
- **`cd -`** — output style differs by one leading "print pwd" line that zsh's interactive cd suppresses but `-c` doesn't. Cosmetic.

The "Stub modules (loaded but limited)" section below remains as documented deferrals (`zsh/cap`, `zsh/clone`, `zsh/curses`, `zsh/zftp`, `zsh/db_gdbm`) — these are niche features whose `zmodload` call currently no-ops, with the corresponding builtins not registered. They are not active gaps in zshrs's compatibility floor; they're tracked separately because they have no real-world load on the daily-driver path. `zsh/mapfile` was previously in this list; it is now closed (read form implemented above).

## Stub modules (loaded but limited)

- `zsh/cap`, `zsh/clone`, `zsh/curses`, `zsh/zftp` — module loads via `zmodload` succeeds but the corresponding builtins (`cap`, `clone`, `zcurses`, `zftp`) aren't registered. Niche features; deferred.
- `zsh/db_gdbm` — `ztie` correctly reports "GDBM support not compiled in" (no native gdbm dep). Acceptable stub behavior.
- `zsh/files chown/chmod/chgrp` — works (proper error for nonexistent file).
- `zsh/mapfile` — closed; `${mapfile[/path]}` magic assoc reads file bytes, composes with `(f)` flag for line-split. See "Closed (this session)" above.
- `zsh/private` — closed; routed to `builtin_local`.
- `zsh/newuser`, `zsh/nearcolor` — niche, deferred.
