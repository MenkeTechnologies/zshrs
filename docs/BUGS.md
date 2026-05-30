# BUGS.md — Demo-surfaced zshrs port gaps

This file logs port gaps and demo-authoring errors uncovered while
writing `examples/demos/*.zsh`. Each entry includes the exact
reproducer, observed zshrs and `/opt/homebrew/bin/zsh` output side
by side, the upstream C reference for the correct behavior, and the
status as one of:

- **`port-bug`** — zshrs diverges from C-zsh behavior; needs porting.
- **`demo-error`** — the demo was wrong; real zsh fails identically.
- **`fixed`** — port-bug already patched; entry left for posterity.

Status is the source of truth for "does this need to be fixed?" —
not the demo's behavior, which may have been worked around to keep
CI green pending the underlying fix.

---

## #1 — `${(j:: :)arr}` empty-then-space joiner

**Status:** `demo-error` (real zsh also rejects)

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(x y z); echo "${(j:: :)a}"'
zsh:1: error in flags near position 7 in '${(j:: :)a}'

$ zshrs --zsh -c 'a=(x y z); echo "${(j:: :)a}"'
zsh:1: bad substitution
```

The `(j:str:)` flag expects exactly one delimiter pair; the form
`(j:: :)` (empty separator immediately followed by a literal space)
is not a valid flag syntax. Both shells reject; only the error
message wording differs. The demo (63_param_flags_join_split.zsh)
was rewritten to use `(j:,:)` / `(j: -> :)` instead.

---

## #2 — `"${(o)arr}"` quoted form does not sort

**Status:** `demo-error` (real zsh also does not sort here)

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(c b a); echo "${(o)a}"'
c b a

$ zshrs --zsh -c 'a=(c b a); echo "${(o)a}"'
c b a
```

The `(o)` flag operates per-array-element, not on the joined string.
When `"${(o)arr}"` is double-quoted, the array collapses into a single
field BEFORE the sort flag has anything to operate on — so the sort
applies to a one-element list and is a no-op. To sort, use bare
`${(o)arr}` (unquoted) or `print -l ${(o)arr}`. Both shells behave
identically. Demo (16_parameter_flags.zsh, 65_param_flags_sort.zsh)
already uses the bare form.

---

## #3 — `[(i)pattern]` is case-sensitive (not case-folded)

**Status:** `demo-error` (zsh's `(i)` is case-sensitive by design)

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(Apple Banana CHERRY); echo "${a[(i)apple]}"'
4

$ zshrs --zsh -c 'a=(Apple Banana CHERRY); echo "${a[(i)apple]}"'
4
```

`4` means "not found" (array length + 1). `(i)pat` in zsh is the
first-matching INDEX (case-sensitive); `(I)pat` is the last-matching
index (also case-sensitive). For case-insensitive search, the demo
would need to lowercase both pattern and elements explicitly, or use
glob qualifiers. Both shells agree. Demo (71) reworded to use
exact-case lookups.

---

## #4 — `$(() { … } arg)` silently drops without newline before `()`

**Status:** `port-bug` — still open after 2026-05-29 lexer attempt.

```sh
$ /opt/homebrew/bin/zsh -fc 'r=$(() { echo $(( $1 * $1 )) } 7); echo $r'
49

$ zshrs --zsh -c 'r=$(() { echo $(( $1 * $1 )) } 7); echo $r'
# silent: no output, EC=0, parse-time abort

# WORKS with leading space:
$ zshrs --zsh -c 'r=$( () { echo $(( $1 * $1 )) } 7); echo $r'
49
```

Diagnosis: zshrs's `src/ported/lex.rs::cmd_or_math_sub` (port of
`Src/lex.c:540`) enters its math-disambiguation when it sees `$((`.
It calls `dquote_parse(')', false)` which, for the empty body of
`()`, matches the first `)` immediately and returns success. The
next char is `{` (not `)`) so math fails. The rewind path then has
to push back the matched `)` so `skipcomm` sees the original literal
closing paren — but the Rust port drops it. The outer `$(...)` body
then never re-balances and the whole script aborts at parse time
with no output and EC=0.

Attempted 2026-05-29 fix: hungetc the matched `)` in cmd_or_math_sub.
Result: still silent. The pop loop in cmd_or_math_sub additionally
re-injects the Inpar+`(` bytes that cmd_or_math_sub itself appended
to lexbuf — bytes which the C source discards silently (`lexbuf.ptr
-= 2; lexbuf.len -= 2;` at Src/lex.c:564-565). Suppressing those
hungets fixed bug 4 but regressed `$(($((2+3))*5))` from 25 to a
garbled `(5*5))`. The clean fix needs cmd_or_math_sub to thread
the dquote-content-vs-Inpar/`(`-padding distinction through the
hungetc queue precisely — a wider lexer rework than this pass
covers.

**Workaround** — demo 80 routes through a named helper:
`square_fn() { echo $(( $1 * $1 )); }; r=$(square_fn 7)`. Or insert
a space: `$( () { … } 7)` parses correctly today.

---

## #5 — `print -P` `%j` / `%T` / `%D{…}` not handled

**Status:** `fixed` 2026-05-29 (`src/ported/prompt.rs`).

{% raw %}
```sh
$ /opt/homebrew/bin/zsh -fc 'print -P "%j"; print -P "%T"; print -P "%D{%Y}"'
0
14:52
2026

$ zshrs --zsh -c 'print -P "%j"; print -P "%T"; print -P "%D{%Y}"'
%j
%T
%D{%Y}
```
{% endraw %}

The prompt-character dispatcher (`putpromptchar` in `Src/prompt.c`,
ported to `src/ported/prompt.rs`) is missing the case branches for:

- `%j` — current job count (`jobtab` walk, c:894-901)
- `%T` — local time HH:MM (`strftime "%H:%M"` over `time(NULL)`)
- `%t` / `%@` — 12-hour time variants
- `%D{...}` — arbitrary `strftime` format applied to current time
- `%!` — current history event number
- `%i` / `%I` — line number

Other escapes that already work: `%n`, `%M`, `%d`, `%~`, `%F{…}`,
`%f`, `%B`, `%b`, `%U`, `%u`, `%{…%}`, `%(?…)`, `%%`.

**Fix** — added case branches for `%j`, `%!`, `%h`, `%t`, `%T`, `%@`,
`%*`, `%w`, `%W`, `%D`, `%D{…}`, `%i` to `putpromptchar`. `%j`
ports Src/prompt.c:563-570 (jobtab walk with STAT_NOPRINT skip);
`%!`/`%h` port c:558-562 (`curhist` from hist.rs); `%T`/`%*`/`%w`/
`%W`/`%D`/`%D{…}` port c:703-770 (`ztrftime` over `SystemTime::now()`
with the C-spec format strings). End-to-end parity verified
byte-for-byte with `/opt/homebrew/bin/zsh` on all 7 forms.

Demo 74 still uses the same escape set; the previously-omitted
forms are now usable but the demo file isn't re-expanded in this
pass (keeps demo register stable).

---

## #6 — `${var:gs|X|Y|}` followed by `print -l` → `bad option`

**Status:** `demo-error` (real zsh fails identically)

```sh
$ /opt/homebrew/bin/zsh -fc 'paths=(/foo/bar); print -l ${paths:gs|/|--|}'
zsh:print:3: bad option: -q

$ zshrs --zsh -c 'paths=(/foo/bar); print -l ${paths:gs|/|--|}'
zsh:print:3: bad option: -q
```

`:gs|/|--|` is a valid global-substitution modifier and DOES produce
`--foo--bar`. The failure is downstream: `print -l --foo--bar`
treats the leading `--` as end-of-options and the rest of the
characters as a flag block. Both shells fail. Demo (83) rewritten
to use chained `:t:r:s` instead, which doesn't produce a leading
`--`.

---

## #7 — `local arr=( $=s )` skips word-split inside fn

**Status:** `fixed` 2026-05-29 (`src/ported/builtin.rs::bin_typeset`).

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { local IFS=:; local s=a:b:c; local arr=( $=s ); echo "n=${#arr[@]} = ${arr[@]}"; }; f'
n=3 = a b c

$ zshrs --zsh -c 'f() { local IFS=:; local s=a:b:c; local arr=( $=s ); echo "n=${#arr[@]} = ${arr[@]}"; }; f'
n=1 = $=s
```

Specifically: `local arr=( $=s )` inside a function leaves
`$=s` LITERAL — the `$=var` word-split-on-IFS operator is not
applied. Outside a function (`arr=(…)` at top level), or inside a
function WITHOUT `local` decl, the operator works.

**Where** — `src/ported/builtin.rs::bin_typeset`'s `=( ... )`
paren-init handling at the `is_paren_init` branch. The plain
`arr=( ... )` path routes through `addvars` (Src/exec.c port at
exec.rs:5050-5104) which calls `prefork(vl, PREFORK_SINGLE |
PREFORK_ASSIGN, …)` on tokenized list elements; `$=name` becomes
spbreak=2 inside paramsubst per Src/subst.c:2558-2571. The typeset
path arrives AFTER the outer prefork has already run against the
whole `arr=( … )` string (treating the parens as literal), so any
`$=name` inside the parens stayed untouched.

**Fix** — mirror C's `spbreak=2` semantics inline: for each
whitespace-split element of the form `$=NAME`, look up `NAME` via
`getsparam`, split on the current IFS, and substitute the resulting
fields in place of the single element. IFS source is `getsparam("IFS")`
(matches the live PM_SPECIAL IFS getter) with the `" \t\n"` default
fallback when unset. Other elements pass through unchanged so plain
`$x`, `${x}`, `"$x"` continue to work via the upstream prefork.

End-to-end parity verified byte-for-byte with `/opt/homebrew/bin/zsh`
on the colon-IFS form, default-IFS form, and the no-`$=` regression
cases.

---

---

## #8 — `local IFS=:` leaks past function return

**Status:** `port-bug` — surfaced while testing bug 7.

```sh
$ /opt/homebrew/bin/zsh -fc '
> f() { local IFS=:; echo "inside: $IFS"; }
> echo "before: $IFS"; f; echo "after: $IFS"
> '
before:  \t\n
inside: :
after:  \t\n

$ zshrs --zsh -c '
> f() { local IFS=:; echo "inside: $IFS"; }
> echo "before: $IFS"; f; echo "after: $IFS"
> '
before:  \t\n
inside: :
after: :          # ← leaked!
```

`local IFS=:` inside `f` correctly establishes the local binding,
but the unwind-on-return path doesn't restore the parent-scope
value. Any subsequent code that depends on IFS sees the wrong
separator.

**Where** — `src/ported/builtin.rs::bin_typeset`'s PM_LOCAL path
calls `createparam(name, PM_LOCAL|…)` which pushes the previous pm
onto `pm.old` (params.rs:1132-1147 port of params.c). The pop
should happen via `endparamscope` (Src/params.c:5279) when
`locallevel` decrements on function return. For `IFS` specifically,
the gsu_s vtable's `ifssetfn` updates the global `ifs` char buffer
on EACH local-decl assignment, but the symmetric restore on scope
exit isn't wiring through to the global. The end-of-fn unwind walks
pm.old but doesn't re-fire setfn for the restored value.

**Workaround** — explicitly restore: `f() { local IFS=:; …; }; old=$IFS; f; IFS=$old`.

Demos that depend on IFS scoping use `${(s/:/)var}` (flag-driven
split, doesn't touch global IFS) instead of `$=var` (depends on
IFS).

---

## #9 — `var=${arr[(expr)*N + M]}` returns empty when unquoted-assigned

**Status:** `port-bug` — surfaced 2026-05-29 while writing demo 239.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(p q r s t); r=1; c=1; v=${a[(r-1)*2 + c]}; echo "[$v]"'
[p]

$ zshrs --zsh -c 'a=(p q r s t); r=1; c=1; v=${a[(r-1)*2 + c]}; echo "[$v]"'
[]
```

`(r-1)*2 + c` is a valid arithmetic subscript and the C-zsh path
returns `a[1]` = `p`. zshrs returns empty when assigned to a
variable unquoted. The same expression works as an interpolation
target and as a quoted-assigned RHS:

```sh
$ zshrs --zsh -c 'a=(p q r s t); r=1; c=1; echo "${a[(r-1)*2 + c]}"'
p

$ zshrs --zsh -c 'a=(p q r s t); r=1; c=1; v2="${a[(r-1)*2 + c]}"; echo "[$v2]"'
[p]
```

**Where** — `${arr[…]}` subscript parsing in `src/ported/subst.rs`
(port of `Src/subst.c::paramsubst`). The leading `(` of `(r-1)`
collides with the C parser's subscript-flag dispatch (e.g. `(r)`,
`(R)`, `(i)`, `(I)`, `(k)`, `(v)`, `(w)`, `(W)`, `(e)`, `(n)`).
C-zsh's subscript flag parser (`Src/subst.c::strstartsfn` chain at
c:3650-3750) rejects malformed flag content with a hard error AND
then falls through to math-eval the entire subscript body. The
Rust port appears to take a "no valid flag → bail with empty" path
when assigning unquoted to a scalar — but treats the same
expression as math correctly in interpolated/quoted contexts.

**Workaround** — compute the index in a separate `$((...))` step
first: `idx=$(( (r-1)*2 + c )); v="${arr[idx]}"`. Demo 239 is
written this way pending the underlying fix.

---

## #10 — `v=$(...)` inside nested for-loop in function corrupts inner iteration

**Status:** `port-bug` — surfaced 2026-05-29 writing demos 239/240.

```sh
$ /opt/homebrew/bin/zsh -fc '
> fn() { for r in 1 2 3; do for c in 1 2 3; do v=$(echo 0); printf "[%s]" "$v"; done; echo; done; }
> fn
> '
[0][0][0]
[0][0][0]
[0][0][0]

$ zshrs --zsh -c '
> fn() { for r in 1 2 3; do for c in 1 2 3; do v=$(echo 0); printf "[%s]" "$v"; done; echo; done; }
> fn
> '
[[[0]0]0]
[[[0]0]0]
[[[0]0]0]
```

The C-zsh path produces `[0][0][0]` per row (9 well-formed iterations).
zshrs interleaves the assignments and printfs into `[[[0]0]0]`,
implying the cmd-sub result lands one or two iterations late — the
first 2 printfs see an empty `$v` and emit only `[` (no closing
`]`); the third sees the catch-up value and emits `0]0]0]`. With
an `if (( v )); then …; else …; fi` branch in the loop, the
inner loop falls through after one iteration: 3 rows of single-cell
output instead of 3×3.

**Where** — `src/ported/exec.rs` execution of nested for-lists when
the function frame holds a deferred cmd-sub completion. The C path
in `Src/exec.c::execcmd` runs cmd-sub to completion (sets the param
via `setsparam`) before the next statement; the Rust port appears to
schedule the substitution result onto a queue that drains one
statement late inside a function frame. Outside a function (top-level
nested for + cmd-sub) the bug doesn't reproduce.

**Workaround** — restructure to one of:

1. Replace `v=$(get_cell $r $c)` with inline subscript:
   `idx=$(( (r-1)*SIZE + c )); v="${BOARD[idx]}"`.
2. Move the cmd-sub OUTSIDE the inner loop (pre-compute a row array).
3. Build the entire row as a string with `+=` and `echo` once after
   the inner loop instead of printf-per-cell with a branch.

Demos 239, 240 use workaround 1 (inline subscript).

---

## #11 — `printf "%d" "'<space>"` returns 0 instead of 32

**Status:** `port-bug` — surfaced 2026-05-29 writing demo 279.

```sh
$ /opt/homebrew/bin/zsh -fc 'printf "%d" "'\'' "'
32

$ zshrs --zsh -c 'printf "%d" "'\'' "'
0
```

POSIX `printf "%d" "'X"` returns the byte value of `X` (the leading
single-quote tells `printf` to treat the next char as a character
constant; spec at [POSIX printf](https://pubs.opengroup.org/onlinepubs/9699919799/utilities/printf.html)).
zshrs's `printf` builtin (port of `Src/builtin.c bin_print`) returns
the correct value for printable letters/digits/punctuation but
returns 0 for space and likely other whitespace. The result silently
corrupts XOR-cipher and other byte-level computation that relies on
the operator to read ASCII codes.

**Workaround** — use zsh's arithmetic `#var` operator, which works
correctly for whitespace too:
```sh
$ zshrs --zsh -c 'c=" "; echo $(( #c ))'
32
```

Demos 279, 280 switched to the `$(( #ch ))` form.

---

## #12 — `${var%|*}` treats `|` as glob alternation, matches empty

**Status:** `port-bug` — surfaced 2026-05-29 writing demo 279.

```sh
$ /opt/homebrew/bin/zsh -fc 'p="hello|key"; echo "${p%|*}"'
hello

$ zshrs --zsh -c 'p="hello|key"; echo "${p%|*}"'
hello|key
```

The trailing-suffix-strip `${var%pattern}` should treat bare `|`
as a literal character; alternation requires the `(a|b)` parens
form per `Src/pattern.c::patcompile` (or the `EXTENDED_GLOB`
extension grouping). zshrs's `paramsubst` modifier path (port of
`Src/subst.c::paramsubst` `:%` branch) appears to register the
pattern `|*` as "empty | <anything>", successfully matching the
empty prefix and stripping NOTHING (since `%` strips the shortest
suffix matching pattern, and empty matches at the very end).

Affected operators: `${var%pattern}`, `${var%%pattern}`,
`${var#pattern}`, `${var##pattern}` — all use the same compiled
pattern from `Src/pattern.c`.

**Workaround** — escape the pipe: `${var%\|*}` works correctly in
zshrs (and zsh — escaped form is universally portable). Or use
a different separator. Demo 279 switched from `|` to `~` separator.

---

## #13 — `[[ "$x" == "?" ]]` fails for literal `?` (double-quoted RHS)

**Status:** `port-bug` — surfaced 2026-05-29 writing demo 301.

```sh
$ /opt/homebrew/bin/zsh -fc 'c="?"; [[ "$c" == "?" ]] && echo MATCH'
MATCH

$ zshrs --zsh -c 'c="?"; [[ "$c" == "?" ]] && echo MATCH'
# silent — no match!
```

The C-zsh parser strips quotes from the RHS of `[[ ==`, leaving a
literal `?` to match. zshrs's port (`Src/cond.c::cond_match` /
`Src/pattern.c::patcompile`) appears to honor the wildcard meaning
of `?` even when the RHS was double-quoted — only single-quoted or
backslash-escaped forms work:

```sh
[[ "$c" == '?' ]] && echo MATCH      # works
[[ "$c" == \? ]]  && echo MATCH      # works
[[ "$c" == "?" ]] && echo MATCH      # FAILS in zshrs
```

Same failure mode likely affects other glob meta-chars (`*`, `[`)
when used as RHS in `[[ == ]]` with double quotes.

**Workaround** — use single quotes or backslash escape for any glob
meta-character on the RHS of `[[ == ]]`. Demo 301 switched all
double-quoted ops (`"?"`, `"+"`, `"#"`, `"&"`) to single-quoted
form. The check on `"/"` (not a glob char) was unaffected.

---

## #14 — `[[ $ch == "{" ]]` causes parse error "unterminated if"

**Status:** `port-bug` — surfaced 2026-05-29 writing demo 301.

```sh
$ /opt/homebrew/bin/zsh -fc 'c="{"; [[ "$c" == "{" ]] && echo MATCH'
MATCH

$ zshrs --zsh -c 'c="{"; [[ "$c" == "{" ]] && echo MATCH'
zsh:1: parse error: unterminated if
```

The C-zsh lexer treats `{` inside double-quoted RHS of `[[ ==` as a
literal character. zshrs's lexer (port of `Src/lex.c::gettokstr` /
`gettok`) appears to enter brace-expansion or compound-cmd parsing
mode when it sees the `{`, never returning to close the `[[`. Same
applies to `}` on the RHS.

**Workaround** — store the brace character in a variable and use
unquoted variable substitution:
```sh
local close_char='}'
[[ $c == $close_char ]] && echo MATCH
```
Demo 301 uses this pattern.

---

## #15 — `set -- ${=var}` doesn't reliably split inside functions

**Status:** `port-bug` — surfaced 2026-05-29 writing demo 298.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { local l="1 2 3"; for x in "a b" "c d"; do set -- ${=x}; echo "$# $1 $2"; done; }; f'
2 a b
2 c d

$ zshrs --zsh -c 'f() { local l="1 2 3"; for x in "a b" "c d"; do set -- ${=x}; echo "$# $1 $2"; done; }; f'
1 a 
1 b 
1 c 
1 d 
```

`for x in "a b" "c d"; do … done` should iterate twice (x="a b",
then x="c d"). The C-zsh path preserves the quoting and iterates
correctly. zshrs's port iterates the WORD-SPLIT array instead —
4 iterations with x="a", x="b", x="c", x="d", losing the field
boundaries entirely. The `${=x}` split then has nothing to split
because each x is already a single word.

**Where** — likely `src/ported/exec.rs::execfor` mishandling the
`for var in "${arr[@]}"` quoting when arr is a literal element list
inside a function frame.

**Workaround** — index the array explicitly instead of `for x in
"${arr[@]}"`:
```sh
for ((i=1; i<=${#arr}; i++)); do
    x="${arr[i]}"
    set -- ${=x}
done
```
Or use parallel arrays for the parsed parts (demo 298 takes this
approach: `la=(1 4 7 …); lb=(2 5 8 …)` then index by `$i`).

---

## #16 — `arr=("${arr[@]:0:-1}")` doesn't shrink array inside function

**Status:** `port-bug` — surfaced 2026-05-29 writing demos 311/314.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { s=(a b c); s=("${s[@]:0:-1}"); echo "${#s}: ${s[*]}"; }; f'
2: a b

$ zshrs --zsh -c 'f() { s=(a b c); s=("${s[@]:0:-1}"); echo "${#s}: ${s[*]}"; }; f'
1:
```

Inside a function, `s=("${s[@]:0:-1}")` (drop last element via slice
to -1) clears the visible content but reports `${#s}` as 1 instead of
2. At top level it works correctly. Likely a port issue in
`src/ported/subst.rs` `paramsubst` slicing path interacting with the
function-frame `pm.old` push/pop in `src/ported/params.rs`.

Same root cause: `s=("${s[@]:0:$(( ${#s} - 1 ))}" )` (explicit length)
ALSO fails: after the last shrink-to-zero, `${#s}` stays at 1 and
the loop never exits.

**Workaround** — use `arr[${#arr}]=()` to delete the last element
(works reliably) or use a counter variable instead of `${#arr}`:

```sh
arr[${#arr}]=()        # decrements correctly
# or:
local top=${#arr}
while (( top > 0 )); do
    cur="${arr[top]}"
    (( top-- ))
done
```

Demos 311/312/314 use the `arr[${#arr}]=()` workaround.

---

## #17 — `var=${arr[-1]}` (unquoted) loses value inside while-loop in fn

**Status:** `port-bug` — surfaced 2026-05-29 writing demo 311.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { s=(7); while (( ${#s} > 0 )); do n=${s[-1]}; echo "[$n]"; s[${#s}]=(); s+=("$((n+1))"); [[ $n -gt 10 ]] && break; done; }; f'
[7]
[8]
[9]
[10]
[11]

$ zshrs --zsh -c 'f() { s=(7); while (( ${#s} > 0 )); do n=${s[-1]}; echo "[$n]"; s[${#s}]=(); s+=("$((n+1))"); [[ $n -gt 10 ]] && break; done; }; f'
[7]
[]
[]
[]
...
```

`n=${s[-1]}` (unquoted assignment from negative-index subscript)
inside a `while` loop in a function reads correctly on the first
iteration only — subsequent iterations capture empty. At top level
or with quotes it works correctly.

**Workaround** — quote the RHS or declare local:
```sh
n="${s[-1]}"    # works
local n=${s[-1]} # also works
```
Demos 311/312/314 use the quoted form.

---

## #18 — `arr[a + 1]=val` parsed as command "arr[a" with space

**Status:** `port-bug` — surfaced 2026-05-29 writing demo 332.

```sh
$ /opt/homebrew/bin/zsh -fc 'dp=(); for ((a=0; a<3; a++)); do dp[a + 1]=$a; done; echo "${dp[@]}"'
0 1 2

$ zshrs --zsh -c 'dp=(); for ((a=0; a<3; a++)); do dp[a + 1]=$a; done; echo "${dp[@]}"'
zsh:1: command not found: dp[a
zsh:1: command not found: dp[a
zsh:1: command not found: dp[a
```

C-zsh accepts `arr[expr]=val` where `expr` contains spaces (parsed
in arith context). zshrs's lexer (port of `Src/lex.c::gettok`)
appears to terminate the assignment target at the first whitespace,
treating `dp[a` as a command name.

**Workaround** — pre-compute the index into a variable:
```sh
local idx=$(( a + 1 ))
dp[idx]=$val
```
Demos 332 uses the pre-compute pattern.

---

## #19 — Quoted special-char / reserved-word case patterns fail in non-first branch

**Status:** `port-bug` — surfaced 2026-05-29 writing demos 363, 365.
Empirically narrowed 2026-05-30: the trigger condition is "non-first
branch", NOT "inside function" (the latter was a misdiagnosis).

```sh
$ /opt/homebrew/bin/zsh -fc 'case x in plain) echo p;; "!") echo b;; *) echo o;; esac'
o

$ zshrs --zsh -c 'case x in plain) echo p;; "!") echo b;; *) echo o;; esac'
zsh:1: expected ')' in case pattern
zshrs: parse error

# But move "!" to FIRST branch and it works:
$ zshrs --zsh -c 'case x in "!") echo b;; plain) echo p;; *) echo o;; esac'
o
```

**Affected tokens** (verified by enumeration):

  | quoted char | first branch | non-first branch |
  |-------------|--------------|-------------------|
  | `"!"`       | works        | **FAILS**         |
  | `"{"`       | works        | **FAILS**         |
  | `"}"`       | works        | **FAILS**         |
  | `'if'`      | works        | **FAILS**         |
  | `'while'`   | works        | **FAILS**         |
  | `'do'`      | works        | **FAILS**         |
  | `'then'`    | works        | **FAILS**         |
  | `'else'`    | works        | **FAILS**         |
  | `'let'`     | works        | **FAILS**         |
  | `"?"`       | works        | works (treated as glob char) |
  | `"*"`       | works        | works (treated as glob char) |
  | `"["` `"]"` | works        | works              |
  | `"("` `")"` | works        | works              |
  | `";"`       | works        | works              |
  | `"@"` `"~"` `"^"` `"#"` | works | works              |
  | `'foo'` (non-keyword) | works | works              |
  | `plain` (bare) | works     | works              |

The failure is identical at top-level and inside functions (the
earlier hypothesis that it required a function context was wrong —
the simple `case x in plain) ... ;; "!") ... esac` at top level
also triggers).

Tested zsh 5.9 (`/opt/homebrew/bin/zsh`) handles all forms correctly.

**Where** — `src/ported/parse.rs::par_case`'s second-or-later
branch parsing. After consuming `;;` and starting the next pattern,
the lexer (`Src/lex.c::gettok` port at `src/ported/lex.rs`) appears
to dequote the pattern and re-tokenize as if at command position,
which re-recognizes reserved words and special tokens like `!`,
`{`, `}` as keyword/grouping operators. The first branch escapes
this because the parser is still in "fresh pattern" mode after the
opening `in`.

Related to bugs #13 (quoted `"?"` ignored in `[[ == ]]`) and #14
(`[[ $ch == "{" ]]` parse error) — all share the same root cause:
zshrs's lexer not respecting quote boundaries for tokens that have
keyword/operator meaning at command position.

**Workarounds**:

  1. Move the affected branch to the FIRST position in the case:
     ```sh
     case $v in
         'if') ...;;           # works at first position
         plain) ...;;
         *) ...;;
     esac
     ```

  2. Use `if/elif` chain (always works):
     ```sh
     if [[ $v == "if" ]]; then ...
     elif [[ $v == "!" ]]; then ...
     elif [[ $v == plain ]]; then ...
     fi
     ```

  3. Store the brace/special character in a variable and use the
     variable as the pattern:
     ```sh
     local LBRC=$'\x7b'
     case $v in
         plain) ...;;
         $LBRC) ...;;
         *) ...;;
     esac
     ```

Demo 363 dispatches unary `'!'`/`'~'` via `if/elif`. Demo 365 moves
all reserved-word case branches before any other branches and uses
a leading `if [[ $head == ... ]]; then ...; return; fi` guard chain
for `quote`, `if`, `let`. Demos 361/362 use `local LC=$'\x7b'`
variables to dodge the `'{' '}'` pattern crash.

---

## #20 — Large recursive parsers run very slowly under zshrs

**Status:** `perf-issue` — surfaced 2026-05-29 writing demos 361/362.

```sh
# Demo 361 (JSON parser) on a 1.5kb input:
$ time ./target/debug/zshrs --zsh demo_361.zsh
real    0m12s

# Demo 362 (XML parser) on a 2kb input:
$ time ./target/debug/zshrs --zsh demo_362.zsh
real    0m45s+    # often times out
```

Parsers with deep recursion (e.g. recursive-descent over nested
JSON/XML structures) execute orders of magnitude slower in zshrs
vs C-zsh. Hot loops involving:

  - many small function calls (recursive parse_element)
  - hash-table writes/reads (AST_TYPE[$id] etc.)
  - cmd-sub inside the recursion (`var=$(...)`)

are particularly affected. The combination of:
  - bug #10 (cmd-sub doesn't propagate side effects in fn)
  - bug #16 (array shrink doesn't update size)
  - bug #17 (var=${arr[-1]} loses value in fn while)

forces workarounds that further slow the inner loops.

**Where** — likely the fusevm dispatch path through hash-table
parameter set/get is slower than expected, or there's per-function
overhead in the call frame push/pop.

**Workaround** — for demos: (a) use smaller test inputs, (b) replace
recursive descent with iterative state machines, (c) use parallel
arrays instead of nested hash structures. Demo 362 was trimmed from
2kb to ~100 bytes of test XML to fit the 30s CI budget.

---

## #21 — Nested `$(( expr1 + $((expr2)) ))` garbles outer expansion

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'n=5; m=$(( n + $((n * 2)) )); echo "m: $m"'
m: 15

$ zshrs --zsh -c 'n=5; m=$(( n + $((n * 2)) )); echo "m: $m"'
m: ( n + 10 ))
```

A `$(( ))` expression containing a nested `$(( ))` does not parse
the inner correctly into the outer. The inner expression evaluates
fine in isolation (`$((n * 2))` prints `10`), but when embedded
inside the outer `$(( n + ... ))`, the outer expression captures
the literal text `( n + 10 ))` instead of evaluating to `15`.

**Where** — `src/ported/lex.rs::cmd_or_math_sub` (port of
`Src/lex.c:540`'s math-vs-cmd-sub disambiguation). When the outer
arith scan encounters `$((`, it tries to recursively parse a math
expression but the inner `$((...))` confuses the bracket counter,
leading to the outer `))` being consumed at the wrong nesting depth.
Related to bug #4 (`$(() { … } arg)` silent abort) which lives in
the same disambiguation path.

**Workaround** — extract the inner expression to a temporary
variable:
```sh
inner=$((n * 2))      # evaluate inner first
m=$(( n + inner ))    # use as plain var in outer math
echo "m: $m"          # → 15
```

Numerous demos that originally used nested arith were rewritten to
this two-step form to fit zshrs.

---

## #22 — Heredoc `\$VAR` escape not honored (variable still expands)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'N=world; cat <<END
> escaped: \$N
> END'
escaped: $N

$ zshrs --zsh -c 'N=world; cat <<END
escaped: \$N
END'
escaped: world
```

In a non-quoted heredoc (`<<END`, NOT `<<'END'`), variable expansion
is normally enabled but `\$` should be honored as an escape to
preserve the literal `$` character. C-zsh respects this:
`\$N` → `$N`. zshrs strips the backslash AND expands the variable,
producing `world` instead of `$N`.

Same expected behavior for `\\` (backslash literal), `\\` , and
`\<newline>` (line continuation).

**Where** — `src/ported/lex.rs::lex_heredoc` or
`src/ported/subst.rs::stringsubst`'s heredoc-body path. The
backslash-escape consumer should fire BEFORE the `$VAR` expansion
when the heredoc delimiter is unquoted; the escape recognition
appears to be missing or fires in the wrong order.

**Workaround** — use a quoted heredoc to suppress all expansion,
then use cmd-sub for the parts that need it:
```sh
cat <<'END'
literal: $N (preserved by quoted delim)
END
```
Or escape via `\$` after first replacing `$` with a placeholder.
Demos in batches 8-15 that use heredocs avoid the `\$` pattern;
they either embed variables directly or use quoted delimiters.

---

## #23 — Worker-pool shutdown INFO leaks to stdout when fd is duped

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
# Minimal repro: script file mode + duped fd:
$ cat > /tmp/min.zsh <<'EOF'
exec 3>&1
echo hi >&3
EOF
$ ./target/debug/zshrs --zsh /tmp/min.zsh 2>&1
hi
2026-05-30T05:14:29.343346Z  INFO main zsh::worker: worker pool shut down tasks_completed=0
```

The `tracing` INFO message `worker pool shut down tasks_completed=0`
appears on stdout when a script duplicates stdout to a higher fd
(e.g. `exec 3>&1`) without closing it before shell exit. Real zsh
prints nothing extra at shutdown.

C-zsh equivalent:
```sh
$ /opt/homebrew/bin/zsh /tmp/min.zsh
hi
```
(no shutdown chatter)

Per project rules (CLAUDE.md `## INVARIANTS` — "Informational chatter
goes to log only", "No `println!`/`eprintln!` outside of error
reporting/explicit-user-output"), this message should be routed to
`~/.cache/zshrs/zshrs.log` via `tracing::info!`, not the real
stdout/stderr.

**Where** — the worker pool's `Drop` impl likely emits a
`tracing::info!` that, when the global subscriber routes to
stdout/stderr by default, finds the fd still open via the
duplicated descriptor and writes there. The fix is either:
  - Configure the tracing subscriber to write ONLY to the log file
  - Suppress the shutdown info entirely (it's debug-grade)
  - Use `tracing::debug!` so it's filtered by default

**Doesn't trigger**:
  - When using `-c` mode (no script file frame)
  - When script closes the duped fd before exit (`exec 3>&-`)
  - When stdout is plain (no `exec >fd` shenanigans)

**Workaround** — close the duplicated fd before script ends:
```sh
exec 3>&1
echo hi >&3
exec 3>&-     # ← close to prevent the leak
```

---

## #24 — `typeset -T` tied colon-array doesn't sync (string side stays empty)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -T XPATH xpath=(/x /y /z) :
echo "XPATH: $XPATH"
echo "xpath: ${xpath[@]}"
xpath+=(/w)
echo "after append: $XPATH"
XPATH="/a:/b:/c"
echo "after assign: ${xpath[@]}"'
XPATH: /x:/y:/z
xpath: /x /y /z
after append: /x:/y:/z:/w
after assign: /a /b /c

$ zshrs --zsh -c 'typeset -T XPATH xpath=(/x /y /z) :
echo "XPATH: $XPATH"
echo "xpath: ${xpath[@]}"
xpath+=(/w)
echo "after append: $XPATH"
XPATH="/a:/b:/c"
echo "after assign: ${xpath[@]}"'
XPATH:
xpath: /x /y /z
after append:
after assign: /x /y /z /w
```

`typeset -T VAR var :` creates a "tied" pair: `VAR` is a
colon-separated string view of array `var`. Modifications to either
side should propagate. C-zsh implements this via `PM_TIED` in
`Src/params.c`.

zshrs's port:
  - The array side (`xpath`) is populated correctly from the `=(...)` init
  - The string side (`XPATH`) is ALWAYS empty
  - Modifying the array doesn't update the string
  - Modifying the string doesn't update the array (it stays at the array's
    pre-assignment value)

**Where** — `src/ported/params.rs::create_tied_var` or the PM_TIED
getter/setter chain. The string-side `intgetfn` / `strgetfn`
callback may be missing the array-join step (`join_arr_with_sep`),
and the assign path is missing the split-on-sep step
(`split_string_on_sep`).

The reverse case (built-in `PATH` ↔ `path`) appears to work
correctly, so the bug is specific to user-declared `typeset -T`
pairs, not the kernel-builtin tied pairs.

**Workaround** — use built-in `PATH`/`path` for path-like vars, or
manually sync the two sides:
```sh
xpath=(/x /y /z)
XPATH="${(j.:.)xpath}"     # rejoin on each mutation
```

---

## #25 — `$ZSH_SCRIPT` unset and `$ZSH_ARGZERO` wrong in script mode

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ cat > /tmp/d.zsh <<'EOF'
echo "ZSH_SCRIPT=${ZSH_SCRIPT:-N/A}"
echo "ZSH_ARGZERO=${ZSH_ARGZERO:-N/A}"
echo "\$0=$0"
EOF

$ /opt/homebrew/bin/zsh /tmp/d.zsh
ZSH_SCRIPT=/tmp/d.zsh
ZSH_ARGZERO=/tmp/d.zsh
$0=/tmp/d.zsh

$ ./target/debug/zshrs --zsh /tmp/d.zsh
ZSH_SCRIPT=N/A
ZSH_ARGZERO=./target/debug/zshrs
$0=/tmp/d.zsh
```

Two separate divergences:

  1. **`$ZSH_SCRIPT`**: zsh sets this to the path of the currently
     running script. zshrs leaves it unset.

  2. **`$ZSH_ARGZERO`**: zsh sets this to `argv[0]` as the script
     intended (the script path when invoked as `zsh /path/to.zsh`).
     zshrs sets it to the path of the zshrs binary itself
     (`./target/debug/zshrs`).

Only `$0` is set correctly to the script path in both shells.

**Where** — `src/ported/init.rs::init_special_vars` (port of
`Src/init.c`'s special-param setup). Both `ZSH_SCRIPT` and
`ZSH_ARGZERO` need to be assigned when zshrs detects script-mode
invocation (`argv[1]` is a `.zsh` file or `--zsh script.zsh`).

**Affected callers** — tooling that introspects "which script am I
in":
  - autoload bookkeeping
  - error messages (`script:line: error`)
  - shellcheck-style linters
  - test harnesses that need the script path

**Workaround** — fall back to `$0` (which works correctly):
```sh
script_path="${ZSH_SCRIPT:-$0}"
arg_zero="${ZSH_ARGZERO:-$0}"
```

---

## #26 — `emulate -L sh` doesn't switch arrays to 0-indexed (KSH_ARRAYS missing)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'inner() {
    emulate -L sh
    a=(1 2 3)
    echo "[0]: ${a[0]}"
    echo "[1]: ${a[1]}"
}
inner'
[0]: 1
[1]: 2

$ zshrs --zsh -c 'inner() {
    emulate -L sh
    a=(1 2 3)
    echo "[0]: ${a[0]}"
    echo "[1]: ${a[1]}"
}
inner'
[0]:
[1]: 1
```

`emulate -L sh` (sticky local emulation) should switch the function
into POSIX-shell mode, which includes 0-indexed array access (the
`KSH_ARRAYS` option). zshrs's `emulate` correctly does some option
adjustment (the leading `in sh emulation` print proves the call
itself works), but doesn't enable `KSH_ARRAYS` — arrays stay
1-indexed inside the sh-emulated function.

C-zsh's emulate dispatch in `Src/init.c::zsh_emulate` sets
`KSH_ARRAYS`, `SH_NULLCMD`, `SH_GLOB`, `SH_WORD_SPLIT`, and others
when the target emulation is `sh`. zshrs's port at
`src/ported/init.rs` appears to skip the `KSH_ARRAYS` step.

Verify with `emulate -R ksh` (similar issue):
```sh
$ /opt/homebrew/bin/zsh -fc 'emulate -L ksh; a=(x y z); echo "${a[0]}"'
x
$ zshrs --zsh -c 'emulate -L ksh; a=(x y z); echo "${a[0]}"'
            # empty
```

**Workaround** — explicitly set `KSH_ARRAYS` after emulate:
```sh
fn() {
    emulate -L sh
    setopt ksh_arrays    # ← required workaround
    a=(1 2 3)
    echo "${a[0]}"
}
```

---

## #27 — Extra `caller` and `help` builtins shadow user functions silently

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'caller() { echo my-fn; }; caller'
my-fn

$ zshrs --zsh -c 'caller() { echo my-fn; }; caller'
0 main
```

zshrs has two bash-specific builtins that real zsh does not:
  - **`caller`** — prints `<index> <fn>` stack info (bash compat)
  - **`help`** — prints `zshrs shell builtins:` listing (bash compat)

User scripts that define functions with those names get silently
shadowed in zshrs. Real zsh treats those names as ordinary
identifiers and runs the user function correctly.

Compounding the issue: `type caller` and `whence caller` both
report **"not found"** — so the user has no way to discover that
the name is taken by a hidden builtin.

**Where** — `src/ported/builtin.rs` registers `bin_caller` and
`bin_help` at startup. These should either:
  - be removed (they don't exist in zsh)
  - be guarded behind a `--bash-compat` flag
  - at minimum: register with `type` / `whence` so users can detect them

**Workaround** — for now, use different names for user functions
(`my_caller`, `usage` instead of `help`), OR shadow with `disable`:
```sh
disable caller help
caller() { echo my-fn; }   # now works
```

---

## #28 — Coreutils mkdir/rm/mv/ln/chmod/chown shadowed as shell builtins

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'type mkdir rm mv ln chmod chown'
mkdir is /bin/mkdir
rm is /bin/rm
mv is /bin/mv
ln is /bin/ln
chmod is /bin/chmod
chown is /usr/sbin/chown

$ zshrs --zsh -c 'type mkdir rm mv ln chmod chown'
mkdir is a shell builtin
rm is a shell builtin
mv is a shell builtin
ln is a shell builtin
chmod is a shell builtin
chown is a shell builtin
```

zshrs registers coreutils-like builtins (`mkdir`, `rm`, `mv`, `ln`,
`chmod`, `chown`, `chgrp`, `cap`, `getcap`, `setcap`, `stat`, `sync`)
that shadow the system commands. Per project rules these are
"anti-fork" extensions intended to avoid `fork+exec` overhead for
common filesystem operations.

The trade-off: any flag that the zshrs builtin doesn't implement,
or any subtle behavior difference vs `/bin/rm`, becomes invisible
to the user — `rm -I` (BSD) or `rm --interactive=never` (GNU) may
silently behave differently.

In zsh proper these become available only when explicitly loaded
via `zmodload zsh/files`, and even then they go to a separate
namespace (`zf_rm`, `zf_mv`, `zf_chmod`) — never shadowing the
system commands by default.

zshrs's diff against real zsh on this front:
```
extra zshrs builtins (50+):
  cap chgrp chmod chown clone example getcap hashinfo ln
  mem mkdir mv nameref patdebug pcre_compile pcre_match
  pcre_study rm rmdir setcap stat strftime sync syserror
  sysopen sysread sysseek syswrite zcurses zdelattr
  zf_chgrp zf_chmod zf_chown zf_ln zf_mkdir zf_mv zf_rm
  zf_rmdir zf_sync zftp zgdbmpath zgetattr zlistattr
  zprof zpty zselect zsetattr zsocket zstat zsystem
  ztcp ztie zuntie
```

Most are from modules that real zsh keeps unloaded by default
(`zsh/pcre`, `zsh/stat`, `zsh/curses`, `zsh/net/tcp`, `zsh/zftp`,
`zsh/zpty`, `zsh/zselect`, `zsh/system`, `zsh/files`). zshrs
pre-loads them all.

**Where** — `src/ported/builtin.rs::register_builtins` initializes
the full set unconditionally.

**Workaround**:
```sh
# Force external command path:
command rm /path/to/file
# Or fully qualify:
/bin/rm /path/to/file
# Or disable the builtin:
disable rm
rm /path/to/file    # now /bin/rm
```

The decision to keep these as default-on builtins is a deliberate
zshrs design choice for perf (anti-fork). The bug here is the
**namespace collision** with system commands — at minimum the
`zf_*` aliases should be the only names registered, with the bare
names (`mkdir`, `rm`, etc.) opt-in.

---

## #29 — Literal `argv[N]` inside double quotes gets stripped when string also contains `$other[idx]`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(x y z); echo "argv[1]=$a[1]"'
argv[1]=x

$ zshrs --zsh -c 'a=(x y z); echo "argv[1]=$a[1]"'
x
```

The literal text `argv[1]=` inside the double-quoted string is
silently consumed by zshrs. The output drops the prefix and shows
only the value of `$a[1]`.

Triggers when ALL of:
  1. inside double quotes
  2. literal text contains the exact name `argv[<idx>]`
  3. the same string also contains a separate `$var[idx]` expansion

Does NOT trigger for any other parameter name. Tested
`funcstack[1]=$a[1]`, `path[1]=$a[1]`, `pipestatus[1]=$a[1]`,
`fpath[1]=$a[1]`, `functrace[1]=$a[1]`, `funcfiletrace[1]=$a[1]`,
`funcsourcetrace[1]=$a[1]` — all work correctly. Only `argv` fails.

zshrs's lexer (port of `Src/lex.c` + `Src/subst.c paramsubst`)
appears to have a special-case for `argv` that triggers
parameter-style subscript expansion on a bare (no `$`) identifier
within double quotes, but only when followed somewhere in the same
string by a legitimate `$var[idx]` reference (the second expansion
probably re-tickles the parser's "we're in subscript mode" flag).

**Where** — `src/ported/subst.rs::stringsubst` (port of
`Src/subst.c::stringsubst`) — the double-quoted-string state
machine's identifier scanner reuses the same code path for `$argv`
and bare `argv` recognition.

**Workaround** — escape the `[`:
```sh
echo "argv\[1\]=$a[1]"   # zsh: argv[1]=x ; zshrs: argv[1]=x
```
Or use `\$argv[1]` literal:
```sh
echo "\\argv[1]=$a[1]"
```
Or just don't use the literal name `argv` in messages.

---

## #30 — `setopt no_clobber` rejects `> /dev/null` (and any char-special device)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt no_clobber; echo hi > /dev/null'
# (no error — succeeds)

$ zshrs --zsh -c 'setopt no_clobber; echo hi > /dev/null'
zsh:1: file exists: /dev/null
```

`setopt no_clobber` (a.k.a. `>!` requirement) should only protect
REGULAR files from being overwritten. Real zsh exempts character
special, block special, FIFO, and symlinks-to-non-regular targets
from the check — overwriting `/dev/null` or `/dev/stdout` always
works. zshrs's port treats `/dev/null` as a protected file.

Affects EVERY common idiom that uses `> /dev/null` or `2> /dev/null`
once a script sets `no_clobber`:

```sh
setopt no_clobber

# All of these fail in zshrs, work in zsh:
some_cmd > /dev/null
some_cmd 2> /dev/null
some_cmd &> /dev/null
echo "fd 1 to stdout dup" >&1
```

**Where** — `src/ported/exec.rs::add_fd_or_open` or the redirection
opener path that calls `O_CREAT | O_EXCL` style flags. Should
`stat()` the target first and skip the existence check for
non-regular files.

**Workaround** — use `>|` to force-clobber:
```sh
setopt no_clobber
some_cmd >| /dev/null    # force overwrite, bypasses noclobber
```

---

## #31 — `${EPOCHSECONDS:-default}` always uses default — `:-` and `:+` think dynamic params are unset

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'zmodload zsh/datetime
echo "1: ${EPOCHSECONDS:-DEFAULT}"
echo "2: ${+EPOCHSECONDS}"
echo "3: ${EPOCHSECONDS:+set}"
echo "4: ${EPOCHSECONDS}"'
1: 1780150194
2: 1
3: set
4: 1780150194

$ zshrs --zsh -c 'zmodload zsh/datetime
echo "1: ${EPOCHSECONDS:-DEFAULT}"
echo "2: ${+EPOCHSECONDS}"
echo "3: ${EPOCHSECONDS:+set}"
echo "4: ${EPOCHSECONDS}"'
1: DEFAULT      ← wrong, should be the value
2: 0            ← wrong, should be 1
3:              ← wrong, should be "set"
4: 1780150194   ← correct via direct access
```

The four standard ways to check if `EPOCHSECONDS` is set ALL report
unset. Direct value access works. This breaks every script that
uses defensive `${var:-fallback}` patterns around dynamic special
parameters from `zsh/datetime`.

Tested all dynamic-special parameters — divergence is specific to
**module-loaded** dynamic params:

  | parameter         | direct | `:-default` | `${+x}` | source                  |
  |-------------------|--------|-------------|---------|-------------------------|
  | `EPOCHSECONDS`    | works  | **FAILS**   | 0/1 wrong | `zsh/datetime`        |
  | `EPOCHREALTIME`   | works  | **FAILS**   | 0/1 wrong | `zsh/datetime`        |
  | `RANDOM`          | works  | works       | works   | built-in PM_SPECIAL    |
  | `SECONDS`         | works  | works       | works   | built-in PM_SPECIAL    |
  | `LINENO`          | works  | works       | works   | built-in PM_SPECIAL    |
  | `HISTCMD`         | works  | works       | works   | built-in PM_SPECIAL    |
  | `SHLVL`           | works  | works       | works   | environment            |

**Where** — `src/ported/modules/datetime.rs` registers
`EPOCHSECONDS`/`EPOCHREALTIME` with the `set` flag not being
properly set on the `Param` struct, so the `is_param_set` check
in `paramsubst` for `:-` / `:+` / `${+x}` returns false.

**Workaround** — fall back to direct access without the `:-` check:
```sh
ts=$EPOCHSECONDS
[[ -n $ts ]] && echo "set to $ts"

# Or assign-then-test:
local epoch="$EPOCHSECONDS"
local fallback="${epoch:-default}"   # epoch is a REGULAR var, the :- works
```

Demos 77, 254, 260, 285, 300, 310, 335, 350, 360, 367 all use
`${EPOCHSECONDS:-N/A}` patterns that silently fall back to the
default. The display works because the value still gets printed
via `strftime $EPOCHSECONDS` (direct access path).

---

## #32 — `hash -d name=~` doesn't expand `~` in the value

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'hash -d zh=~; hash -d'
zh=/Users/wizard

$ zshrs --zsh -c 'hash -d zh=~; hash -d'
zh='~'
```

`hash -d` (named directory hash) should expand `~`, `~user`, `~+`,
`~-` as paths when the value is being stored. zshrs stores the
LITERAL tilde character — `'~'` quoted in the listing output
proves the value is the unprocessed string.

Same with `hash -d name=$VAR`:
```sh
$ zshrs --zsh -c 'foo=/tmp; hash -d zh=$foo; hash -d'
zh=/tmp     # this works (variable expansion happens at parse)

$ zshrs --zsh -c 'hash -d zh=~root; hash -d'
zh='~root'  # but tilde expansion is skipped
```

**Where** — `src/ported/builtin.rs::bin_hash` `-d` branch should
call the filename-expansion path (`tilde_expand` from
`Src/glob.c::tilde_expand`) before storing the value in the named
directory table.

**Affected callers** — every user shell `~/.zshrc` that uses the
common pattern:
```sh
hash -d proj=~/projects/main
hash -d dl=~/Downloads
cd ~proj    # works in zsh; in zshrs, "~proj" stays unexpanded literal
```

**Workaround** — pre-expand:
```sh
hash -d proj=$HOME/projects/main    # works
# or use $HOME directly instead of ~
```

---

## #33 — `set -e` (errexit) doesn't fire on `(( false_cond ))`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'set -e; (( 0 )); echo "still here"'
# (no output — shell exits with status 1 BEFORE "still here")

$ zshrs --zsh -c 'set -e; (( 0 )); echo "still here"'
still here
```

`set -e` (errexit) should cause the shell to exit on any command
returning non-zero status, including `(( expr ))` when `expr`
evaluates to 0/false. zshrs treats `(( false_cond ))` as not
triggering errexit.

Confirmed working with same scaffolding:
  - `set -e; false; echo "after"`           → both exit before "after" ✓
  - `set -e; fn() { return 1; }; fn; echo`  → both exit ✓
  - `set -e; let "0"; echo "after"`         → both exit ✓
  - `set -e; (( 0 )); echo "after"`         → **zsh exits, zshrs continues**
  - `set -e; (( 1 == 0 )); echo "after"`    → **zsh exits, zshrs continues**

**Where** — `src/ported/exec.rs::exec_arith_or_test` (the `(( ))`
execution path) doesn't propagate non-zero exit status to the
errexit checker. The `let` and `false` paths do.

**Affected scripts** — defensive scripts using `(( var > 0 ))`
patterns under `set -euo pipefail` to abort early when a counter
or guard variable is wrong:

```sh
set -euo pipefail
load_config

# zsh: exits here if count == 0 (correct)
# zshrs: continues anyway (silent failure mode)
(( count > 0 ))
do_thing
```

**Workaround** — wrap in if/then with explicit exit:
```sh
set -e
(( count > 0 )) || exit 1   # || branch fires correctly
```

---

## #34 — `case` `(a*|b*))` paren-grouped alternation doesn't match with `extended_glob`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extended_glob
case "abc" in (a*|b*)) echo "alt match" ;; *) echo "default" ;; esac'
alt match

$ zshrs --zsh -c 'setopt extended_glob
case "abc" in (a*|b*)) echo "alt match" ;; *) echo "default" ;; esac'
default
```

A case pattern wrapped in parentheses with internal alternation
`(a*|b*))` should match either alt under `extended_glob`. zshrs
treats it as not matching, falling through to the `*` default.

Works without the outer parens (`a*|b*)`), so the bug is in the
combination of the leading `(` opener and the case-arm closing
`)`. The lexer probably consumes the opening `(` as if starting a
new case arm, then trips on the closing `))`.

Tested:
  - `case x in (x)) echo ok ;; esac`           → both: `ok`     ✓
  - `case x in (x|y)) echo ok ;; esac`         → zshrs: empty; zsh: `ok` ✗
  - `case x in x|y) echo ok ;; esac`           → both: `ok`     ✓
  - `case x in ((x|y))) echo ok ;; esac`       → both: `ok`     ✓ (extra paren)

**Where** — `src/ported/parse.rs::par_case` interprets `(pat))` as
an empty pattern followed by extra `)`. C-zsh strips the outer
parens and matches against the inner pattern.

**Workaround** — drop the outer parens or double them up:
```sh
case x in
    a*|b*) ...        # bare form works
    ((a*|b*))) ...    # doubled outer works
esac
```

---

## #35 — `${(v)h[key]}` flag on single subscript errors with `bad substitution`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h=(a 1 b 2); echo "${(v)h[a]}"'
1

$ zshrs --zsh -c 'typeset -A h=(a 1 b 2); echo "${(v)h[a]}"'
zsh:1: bad substitution
```

The `(v)` flag is supposed to return values when applied to an
associative array. For a single-key access `${(v)h[key]}`, real
zsh returns the value at `key`. zshrs rejects the syntax entirely.

Works without the flag:
  - `${h[a]}`        → both: `1`
  - `${(v)h}`        → both: `1 2` (all values)
  - `${(@v)h}`       → both: `1 2`
  - `${(v)h[a]}`     → **zshrs: error; zsh: `1`** ← the bug

**Where** — `src/ported/subst.rs::paramsubst` doesn't handle the
combination of a value-extraction flag with a subscript expression.
The `[a]` parses as a normal subscript but the `(v)` then errors
trying to apply value-flag to a result that's already a scalar.

**Workaround** — drop the `(v)` since for single-subscript access
the result IS the value:
```sh
echo "${h[a]}"      # works, equivalent
```
Or extract via key-then-value chain:
```sh
local val="${h[a]}"
echo "$val"
```

---

## #36 — MULTIOS not implemented: `> a > b` and `< a < b` don't tee/cat

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
# Real zsh fans out (MULTIOS default-on):
$ /opt/homebrew/bin/zsh -fc 'echo hi > /tmp/a > /tmp/b
cat /tmp/a
cat /tmp/b'
hi
hi

# zshrs only honors the LAST redirect:
$ zshrs --zsh -c 'echo hi > /tmp/a > /tmp/b
cat /tmp/a
cat /tmp/b'
(empty)    ← /tmp/a never written
hi         ← only /tmp/b

# Same for < (multios concatenates inputs):
$ echo a1 > /tmp/a; echo a2 > /tmp/b
$ /opt/homebrew/bin/zsh -fc 'cat < /tmp/a < /tmp/b'
a1
a2

$ zshrs --zsh -c 'cat < /tmp/a < /tmp/b'
a2          ← only the last input read
```

zsh's `MULTIOS` option (default-on) makes `> a > b` write to BOTH
files (via an internal tee fanout) and `< a < b` concatenate
both inputs. zshrs's redirection handler only honors the last
file mentioned for each direction.

This breaks common zsh idioms:
```sh
# log both stdout and stderr to file AND tty:
some_cmd > log.txt > /dev/tty
# zsh: log.txt has output + tty shows it
# zshrs: only /dev/tty shows it, log.txt is empty
```

**Where** — `src/ported/exec.rs::exec_redirs` should detect
multiple redirects to the same fd and create a tee/cat
intermediate process per `Src/exec.c::addmultio`. zshrs uses the
last redirect to overwrite the prior fd binding.

**Workaround** — explicit tee/cat:
```sh
echo hi | tee /tmp/a > /tmp/b      # explicit tee
cat /tmp/a /tmp/b | other_cmd      # explicit concat
```

---

## #37 — `${(z)str}` inside double quotes splits fields unexpectedly

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=" foo bar baz "; echo "[${(z)a}]"'
[foo bar baz]

$ zshrs --zsh -c 'a=" foo bar baz "; echo "[${(z)a}]"'
[foo] [bar] [baz]
```

The `(z)` shell-words split flag inside double quotes should give
a single space-joined string. zshrs splits the result into
separate fields even inside quotes.

When assigned to an array, both behave the same (`b=( ${(z)a} );
echo "[${b[@]}]"` → `[foo bar baz]` in both).

The divergence is specific to `"${(z)a}"` interpolation.

**Where** — `src/ported/subst.rs::paramsubst` treats the `(z)`
result as an array even when the surrounding context is a quoted
scalar. The IFS-rejoin step is missing for quoted `(z)` results.

Related: `(s/sep/)` likely has the same issue:
```sh
$ zshrs --zsh -c 'a="x,y,z"; echo "[${(s/,/)a}]"'
[x] [y] [z]

$ /opt/homebrew/bin/zsh -fc 'a="x,y,z"; echo "[${(s/,/)a}]"'
[x y z]
```

**Workaround** — use `(j: :)` to rejoin explicitly:
```sh
echo "[${(j: :)${(z)a}}]"   # both: [foo bar baz]
```

---

## #38 — Many prompt escapes missing / printed-literally (extends bug #5)

**Status:** `port-bug` — surfaced 2026-05-30 hunting. Extends bug
#5 which only listed `%j`, `%T`, `%D{}`.

Direct sweep against `/opt/homebrew/bin/zsh` 5.9:

  | escape   | zshrs                | zsh                    | meaning                  |
  |----------|----------------------|------------------------|--------------------------|
  | `%m`     | **literal `%m`**     | `codelabs-arm`         | hostname (short)         |
  | `%C`     | **literal `%C`**     | `zshrs`                | last segment of $PWD     |
  | `%i`     | **`0`**              | `1`                    | line number              |
  | `%I`     | **literal `%I`**     | `1`                    | line in source           |
  | `%l`     | **literal `%l`**     | `()`                   | tty line                 |
  | `%y`     | **literal `%y`**     | `()`                   | controlling tty          |
  | `%H`     | **literal `%H`**     | (empty)                | highlight ON / partial   |
  | `%E`     | **literal `%E`**     | `\x1b[K`               | clear to EOL             |
  | `%v`     | **literal `%v`**     | (empty)                | $psvar[1]                |
  | `%s`     | (empty)              | `\x1b[27m`             | standout OFF             |
  | `%u`     | (empty)              | `\x1b[24m`             | underline OFF            |
  | `%b`     | (empty)              | `\x1b[0m`              | bold OFF                 |
  | `%f`     | (empty)              | `\x1b[39m`             | foreground default       |
  | `%k`     | (empty)              | `\x1b[49m`             | background default       |

Working: `%n`, `%M`, `%~`, `%/`, `%d`, `%c`, `%j`, `%T`, `%t`, `%@`,
`%w`, `%W`, `%D`, `%D{}`, `%!`, `%h`, `%F{}`, `%K{}`, `%B`, `%U`,
`%S`, `%%`, `%?`, `%(?...)`, `%{...%}`.

The most impactful gap is **`%m`** (hostname). Used in almost
every customized zsh prompt (`%n@%m:%~$`). zshrs prints the literal
`%m` text where the hostname should appear.

The escape-OFF pairs (`%b`/`%u`/`%s`/`%f`/`%k`) emit empty in zshrs
where zsh emits the corresponding ANSI/terminfo escape sequence —
this leaves users with prompts where colors/styles don't reset
after `%F{red}...%f` etc.

**Where** — `src/ported/prompt.rs::putpromptchar` (port of
`Src/prompt.c::putpromptchar` switch table). Each missing case
needs to:
  - `%m`: short hostname from `gethostname()`
  - `%C`: last segment of `$PWD`
  - `%i`: current line number (`$LINENO`)
  - `%l`/`%y`: tty name from `ttyname(0)` or `()` when no tty
  - `%E`: ANSI clear-to-EOL (`\x1b[K`)
  - `%v`: `$psvar[1]`
  - `%b`/`%u`/`%s`/`%f`/`%k`: terminfo string for "off" of B/U/S/F/K

**Workaround** — substitute manually in prompts:
```sh
PS1='%n@$HOST:%~%# '       # use $HOST instead of %m
PS1='%F{red}error%f'        # zshrs: red text, but %f doesn't reset
PS1='%F{red}error%F{default}' # workaround: explicit "default" color
```

---

## #39 — `${arr:#"literal pattern"}` doesn't honor quoting (still globs)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(cat "[bc]*" bird); echo "[${(M)a:#"[bc]*"}]"'
[[bc]*]   # quoted pattern is literal, only matches "[bc]*" element

$ zshrs --zsh -c 'a=(cat "[bc]*" bird); echo "[${(M)a:#"[bc]*"}]"'
[cat [bc]* bird]   # treats as glob, matches cat (c*) and bird (b*) and the literal one
```

Per zsh docs, when a pattern inside `${arr:#...}` or `${(M)arr:#...}`
is quoted, the quote chars make the contents LITERAL — `[bc]*` becomes
the exact 5-character string, not a glob pattern.

zshrs ignores the quotes and treats `"[bc]*"` as a glob anyway. This
breaks any code that uses quoted patterns to match literal special
characters.

Backslash-escaped (`\[bc\]\*`) form works in zshrs but doesn't work
identically in zsh either — so quoting is the only reliable way
to mean "literal pattern" and it fails in zshrs.

**Where** — `src/ported/subst.rs::paramsubst` pattern parsing for
`:#` and `(M):#`. Quoted segments should be added to the pattern
as literal-only nodes, not glob nodes.

**Workaround** — match exact strings via per-element iteration:
```sh
local -a result
for el in "${a[@]}"; do
    [[ $el == '[bc]*' ]] && result+=("$el")
done
```

---

## #40 — `print -aC N` ignores `-a` flag — outputs column-major instead of row-major

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'print -aC 3 a b c d e f g h i'
a  b  c     # zsh: row-major (across rows) because -a flag is set
d  e  f
g  h  i

$ zshrs --zsh -c 'print -aC 3 a b c d e f g h i'
a  d  g     # zshrs: column-major (down columns) — ignores -a
b  e  h
c  f  i
```

Per zsh docs, `print -C N` defaults to **column-major** (items
flow down each column, then to next column). The `-a` flag
overrides this to **row-major** (items flow across each row, then
down to next row).

zshrs honors `print -C N` correctly (column-major) but **ignores
the `-a` flag** — both `print -C N` and `print -aC N` produce
column-major output.

**Where** — `src/ported/builtin.rs::bin_print` `-C` branch. After
parsing `-a` flag (likely setting an `across_rows` boolean), the
print path needs to switch the inner/outer loop order. Currently
just always does column-major.

**Affected demos** — 73, 88, 303, 325 use `print -aC N` for tabular
output and show column-major where row-major was intended.

**Workaround** — sort the input order manually before passing:
```sh
# To get row-major a b c / d e f / g h i,
# pass items in zshrs's column-major sort order:
print -aC 3 a d g b e h c f i   # zshrs renders as row-major
```

---

## #41 — Glob qualifier `Yn` (limit count) returns ALL matches

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'mkdir -p /tmp/g && touch /tmp/g/{a..j}; echo /tmp/g/*(Y3)'
/tmp/g/i /tmp/g/g /tmp/g/a   # zsh: returns first 3 only

$ zshrs --zsh -c 'mkdir -p /tmp/g && touch /tmp/g/{a..j}; echo /tmp/g/*(Y3)'
/tmp/g/a /tmp/g/b /tmp/g/c /tmp/g/d /tmp/g/e /tmp/g/f /tmp/g/g /tmp/g/h /tmp/g/i /tmp/g/j   # zshrs: returns all 10
```

The `Yn` glob qualifier limits the number of matches to at most
`n`. zshrs ignores the qualifier and returns all matches.

**Where** — `src/ported/glob.rs::apply_qualifiers` should
recognize the `Y<num>` qualifier (per `Src/glob.c` glob qualifier
table) and slice the result list to the first `n` elements.

**Workaround** — pipe through `head`:
```sh
echo /tmp/g/* | tr ' ' '\n' | head -3 | tr '\n' ' '
# or array slice:
files=(/tmp/g/*); echo "${files[1,3]}"
```

---

## #42 — Bare `typeset` prints `var=val` instead of full declarations

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset' | head -5
integer 10 readonly !=0
integer 10 readonly '#'=0
integer 10 readonly '$'=24105
array readonly '*'=(  )
readonly -=569Xf

$ zshrs --zsh -c 'typeset' | head -5
!=0
#=0
$=0
*=(  )
-=''
```

`typeset` without arguments should print every variable with its
**full declaration** including attributes (`integer`, `readonly`,
`array`, etc.) — the output should be valid shell code that could
recreate the variable.

zshrs prints just `name=value` without the type/attribute prefix.
This breaks any tooling that parses `typeset` output to extract
attributes, and also breaks the "round-trip" property where
`eval "$(typeset)"` recreates the state.

`typeset -p NAME` (with a specific name) DOES print the full
declaration correctly in zshrs — only the bare form is wrong.

**Where** — `src/ported/builtin.rs::bin_typeset`'s no-arg listing
path. Should iterate all params and print using the same logic as
`-p` mode.

**Workaround** — pass explicit `-p`:
```sh
typeset -p   # zshrs: correct full declarations
typeset -m '*' -p   # also works, matches all
```

---

## #43 — `${#var:modifier}` / `${#var/pat/rep}` / `${#arr[a,b]}` length operator ignores the transform

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'p=/foo/bar/baz
echo ":h len=${#p:h}"
echo ":t len=${#p:t}"
echo "subst len=${#p/foo/X}"
echo "slice len=${#p[1,5]}"
a=(one two three)
echo "arr slice len=${#a[1,2]}"'
:h len=8
:t len=3
subst len=10    # "X/bar/baz" length
slice len=5     # "/foo/" length
arr slice len=2

$ zshrs --zsh -c 'p=/foo/bar/baz
echo ":h len=${#p:h}"
echo ":t len=${#p:t}"
echo "subst len=${#p/foo/X}"
echo "slice len=${#p[1,5]}"
a=(one two three)
echo "arr slice len=${#a[1,2]}"'
:h len=12       # WRONG (should be 8) — ignored :h
:t len=12       # WRONG (should be 3) — ignored :t
subst len=12    # WRONG (should be 10)
slice len=12    # WRONG (should be 5)
arr slice len=3 # WRONG (should be 2)
```

zsh's `${#var<modifier>}` computes length of the **transformed**
value. zshrs computes length of the **original** value, ignoring
every kind of transform:

  - history modifiers: `:h` `:t` `:r` `:e` `:a` `:A` `:s`
  - pattern substitution: `${#var/pat/rep}` / `${#var//pat/rep}`
  - string slice: `${#var[i,j]}` / `${#var:i:n}`
  - array slice: `${#arr[i,j]}`
  - prefix/suffix strip: `${#var#pat}` / `${#var%pat}` (likely; not tested)

**Where** — `src/ported/subst.rs::paramsubst` applies the `#`
length-operator BEFORE the modifier chain runs. The length should
be computed as the LAST step after all transforms (or equivalently:
apply `#` to the transformed result, not the raw param).

C-zsh's `paramsubst` runs transforms in order and applies `#` on
the final string value (see `Src/subst.c::singsub` chain).

**Impact** — any defensive script checking "is the resulting path
non-trivial":

```sh
path=/usr/local/bin/script.sh
if (( ${#path:h} > 0 )); then       # zsh: 13, zshrs: 22 (same path length)
    echo "head exists"
fi

# array bounds check after slice:
if (( ${#words[2,5]} == 4 )); then  # zsh: 4 (length of slice)
    process_them                    # zshrs: ${#words} (full array length)
fi
```

**Workaround** — assign to a temporary then take its length:
```sh
local h="${p:h}"
echo "len=${#h}"        # zshrs: correct (8 for /foo/bar)

local slice=("${a[1,2]}")
echo "len=${#slice}"    # correct array slice length
```

This single bug affects ALL parameter expansion modifiers in
length context — a wide compat surface.

---

## #44 — `set -x` xtrace output prints literal `%x %N %I %_` instead of expanding PS4

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'set -x; echo hi'
[34mzsh	zsh	1	[0m	echo hi
hi

$ zshrs --zsh -c 'set -x; echo hi'
[34m%x	%N	%I	%_[0m	echo hi
hi
```

The default `$PS4` is `%F{blue}%x\t%N\t%I\t%_%f\t` — prompt
escapes that should expand to filename, function name, line
number, and parser state. zshrs prints them LITERALLY. Real zsh
expands them as part of `set -x` (xtrace) output.

Custom `PS4="+ "` (no escapes) works in both shells correctly.
The bug is specific to the prompt-escape expansion stage of PS4
during xtrace.

Missing prompt escapes (combining with bug #38):
  - `%x` script/source filename
  - `%N` function/script name
  - `%_` parser state for continuation
  - `%P` `%R` `%V` (various)

**Where** — `src/ported/exec.rs::trace_command` should call
`putpromptchar` (or equivalent) on `$PS4` before printing each
trace line. Currently appears to pass `$PS4` through unprocessed.

The fix should reuse the prompt-expansion code from `print -P`,
since `$PS1` `$PS2` `$PS3` `$PS4` `$RPS1` all share the same
escape syntax.

**Impact** — every script using `set -x` to debug has broken trace
output. The trace line still shows the COMMAND being executed,
which is the main info, but the source-location prefix is lost.

**Workaround** — set a simple `$PS4` before enabling trace:
```sh
PS4="+ "
set -x
my_command
set +x
```

---

## #45 — `${#$}`, `${#PPID}` length operator returns 0 for special-PID params

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "PID=$$; len=${#$}"'
PID=84994; len=5

$ zshrs --zsh -c 'echo "PID=$$; len=${#$}"'
PID=84993; len=0
```

`$$` (current shell PID) expands correctly with `$` but the `${#}`
length-of operator returns 0. The `$$` parameter direct access
works correctly; only the length application breaks.

Likely affects all single-character special parameters that hold
expandable values:
  - `${#$}` (PID length) — wrong
  - `${#?}` (exit status length) — works (tested earlier)
  - `${##}` (positional count length) — works
  - `${#!}` (last bg PID length) — untested

Workaround using a temp:
```sh
pid=$$
echo "len=${#pid}"   # both shells: 5
```

Related to bug #43 but specific to single-char special params
(which are read via different code paths in zsh's parser).

**Where** — `src/ported/subst.rs` parameter-name lookup for the
`${#X}` form, where `X` is a special-char param. The `${#X}`
parse doesn't resolve special single-char params before computing
length.

---

## Aggregate triage

| # | bug | status | covered by demo |
|---|-----|--------|------------------|
| 1 | `(j:: :)` joiner | demo-error | 63 reworded |
| 2 | quoted `(o)` | demo-error | 65 uses bare form |
| 3 | `(i)` case-folding | demo-error | 71 exact-case |
| 4 | anon fn in cmd-sub silent | **port-bug** | 80 uses named fn |
| 5 | prompt `%j %T %D{}` | **fixed** 2026-05-29 | 74 (escape set kept) |
| 6 | `:gs|X|Y|` + `print -l` | demo-error | 83 chained `:t:r:s` |
| 7 | `local arr=( $=s )` | **fixed** 2026-05-29 | uses `${(s/:/)var}` |
| 8 | `local IFS=` leaks | **port-bug** | n/a — workaround in 7's fix |
| 9 | `v=${arr[(expr)*N+M]}` unquoted | **port-bug** | 239 uses `idx=$(…)` step |
| 10 | nested-for + cmd-sub in fn | **port-bug** | 240 uses inline subscript |
| 11 | `printf "%d" "' "` returns 0 | **port-bug** | 279/280 use `$((#c))` |
| 12 | `${var%\|*}` `\|` treated as alt | **port-bug** | 279 uses `~` separator |
| 13 | `[[ "$x" == "?" ]]` ignores quotes | **port-bug** | 301 uses `'?'` single-quotes |
| 14 | `[[ $ch == "{" ]]` parse error | **port-bug** | 301 uses `$close_char` var |
| 15 | `set -- ${=x}` mis-iterates in fn | **port-bug** | 298 uses parallel arrays |
| 16 | `arr=("${arr[@]:0:-1}")` no-shrink in fn | **port-bug** | 311 uses `arr[${#arr}]=()` |
| 17 | `var=${arr[-1]}` unquoted in fn while | **port-bug** | 311 quotes the RHS |
| 18 | `arr[a + 1]=val` with space parsed as cmd | **port-bug** | 332 pre-computes `idx=$((..))` |
| 19 | quoted special/keyword case pat (non-first branch) | **port-bug** | 363/365 reorder branches or if/elif |
| 20 | recursive parsers very slow vs C-zsh | **perf-issue** | 362 trimmed test inputs |
| 21 | nested `$(( a + $((b)) ))` garbles outer expansion | **port-bug** | extract inner to var first |
| 22 | heredoc `\$VAR` escape not honored | **port-bug** | use `<<'END'` quoted form |
| 23 | worker-pool shutdown INFO leaks to stdout | **port-bug** | close duped fd before exit |
| 24 | `typeset -T` tied colon-array no-sync | **port-bug** | manual `${(j.:.)arr}` rejoin |
| 25 | `$ZSH_SCRIPT` unset, `$ZSH_ARGZERO` wrong | **port-bug** | fall back to `$0` |
| 26 | `emulate -L sh` missing KSH_ARRAYS | **port-bug** | `setopt ksh_arrays` explicit |
| 27 | `caller`/`help` extra builtins shadow user fns | **port-bug** | `disable caller help` |
| 28 | `mkdir`/`rm`/`mv`/etc. shadowed as shell builtins | **port-bug** | `command rm` to bypass |
| 29 | `"argv[N]=..."` literal stripped inside double quotes | **port-bug** | escape `\[` `\]` |
| 30 | `setopt no_clobber` rejects `> /dev/null` | **port-bug** | `>\|` force-clobber |
| 31 | `${EPOCHSECONDS:-x}` always uses default | **port-bug** | direct `$EPOCHSECONDS` access |
| 32 | `hash -d name=~` doesn't expand `~` in value | **port-bug** | use `$HOME` literal |
| 33 | `set -e` doesn't fire on `(( false_cond ))` | **port-bug** | `\|\| exit 1` explicit |
| 34 | case `(a*\|b*))` paren-alt doesn't match w/ extended_glob | **port-bug** | drop outer parens or double them |
| 35 | `${(v)h[key]}` errors with bad substitution | **port-bug** | drop the `(v)` for single subscript |
| 36 | MULTIOS not implemented (multiple `>` / `<` redirects) | **port-bug** | explicit `tee`/`cat` |
| 37 | `"${(z)str}"` quoted form splits fields | **port-bug** | `${(j: :)${(z)str}}` rejoin |
| 38 | prompt escapes `%m`/`%C`/`%i`/`%l`/`%y`/`%E`/`%v`/`%b`/`%u`/`%s`/`%f`/`%k` missing | **port-bug** | use `$HOST`/`$PWD` etc |
| 39 | `${arr:#"literal"}` quoted pat still globbed | **port-bug** | per-element iteration |
| 40 | `print -aC N` ignores `-a` (column-major instead of row) | **port-bug** | sort input in advance |
| 41 | Glob qualifier `Yn` (limit) returns all matches | **port-bug** | `head -n` or array slice |
| 42 | Bare `typeset` prints `name=val` only, no attrs | **port-bug** | use `typeset -p` |
| 43 | `${#var:mod}` / `${#var/pat/rep}` / `${#arr[i,j]}` ignores transform | **port-bug** | assign to temp first |
| 44 | `set -x` PS4 doesn't expand `%x %N %I %_` | **port-bug** | `PS4="+ "` simple |
| 45 | `${#$}` returns 0 (length of PID) | **port-bug** | `pid=$$; ${#pid}` |

Of forty-five entries, two are fixed (5, 7), thirty-nine remain
open port-bugs/perf-issues (4, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34,
35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45), and four were
zsh-correct behavior misframed by demos (1, 2, 3, 6).
