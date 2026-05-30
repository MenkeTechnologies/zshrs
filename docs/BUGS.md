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

## #46 — Nested backquotes `` `echo \`echo X\`` `` mishandled

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo `echo \`echo deep\``'
deep

$ zshrs --zsh -c 'echo `echo \`echo deep\``'
echo deep``
```

Backquote command substitution with backslash-escaped inner
backquotes (the classic POSIX way to nest before `$()` was added)
doesn't evaluate the inner level. zshrs treats the inner ``\` ...\` ``
as literal text including the trailing `` `` ``.

The `$(...)` form works correctly in both shells for nesting:
  - `$(echo $(echo deep))` → both: `deep`
  - `$(echo \`echo deep\`)` → both: `deep` (mixed)
  - `` `echo $(echo deep)` `` → both: `deep` (mixed reverse)
  - `` `echo \`echo deep\`` `` → **zshrs: `echo deep\`\``; zsh: `deep`**

**Where** — `src/ported/lex.rs::lex_bq` (the backquote-substitution
lexer; ports `Src/lex.c::bquote`). Escape handling for `\`` inside
a backquote context appears to suppress the backquote interpretation
without then re-treating the escaped backquote as the START of a
nested cmd-sub.

POSIX-style nested backquotes are rarely written today (most code
uses `$()` since the 1990s), but legacy `~/.zshrc` files and
shellcheck-flagged third-party scripts still contain them. The
`$()` workaround is universal.

**Workaround** — use `$(...)`:
```sh
$(echo $(echo deep))    # works everywhere
```

---

## #47 — `${(b)str}` quote-special-chars flag escapes more than C-zsh

**Status:** `port-bug` — surfaced 2026-05-30 hunting. Possibly a
spec compliance divergence rather than a clear bug — see analysis.

```sh
$ /opt/homebrew/bin/zsh -fc 'a="hello world"; echo "${(b)a}"'
hello world           # zsh does NOT escape space

$ zshrs --zsh -c 'a="hello world"; echo "${(b)a}"'
hello\ world          # zshrs escapes space
```

Char-by-char comparison of escaping:

  | char     | zshrs           | zsh           | shell-special? |
  |----------|-----------------|---------------|----------------|
  | space    | `hello\ world`  | `hello world` | yes (IFS)      |
  | `;`      | `with\;semi`    | `with;semi`   | yes (cmd sep)  |
  | `*`      | `with\*glob`    | `with\*glob`  | yes (glob)     |
  | `(` `)`  | `with\(paren\)` | `with\(paren\)` | yes (paren)  |

Per `man zshexpn` for `(b)`:
> Quote any characters from the resulting string that are special to
> filename generation or shell syntax.

By that spec wording, space and `;` ARE shell-syntax special, so
zshrs's escaping is MORE-correct per the documentation. But the
de facto C-zsh implementation doesn't escape them, so any script
that relies on zsh's actual behavior (most do) breaks under zshrs.

**Where** — `src/ported/subst.rs::apply_b_flag` (port of
`Src/subst.c::quotestring` with `QT_BACKSLASH`). The escape set
includes whitespace and command separators that the C
implementation excludes from `quotestring`'s `QT_BACKSLASH` set.

**Impact** — code that uses `(b)` to build literal patterns for
later expansion:
```sh
file="my document.pdf"
ls **/$~"${(b)file}"    # zsh: searches for "my document.pdf"
                        # zshrs: searches for "my\ document.pdf" (no match)
```

**Workaround** — pre-process the escaped value to strip the unwanted
backslash escapes, or use single quoting:
```sh
ls **/$~"$file"        # alt: skip the (b) flag entirely
```

---

## #48 — `typeset -m PATTERN` rejects pattern argument

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=1; b=2; aa=3; typeset -m "a*"'
a=1
aa=3

$ zshrs --zsh -c 'a=1; b=2; aa=3; typeset -m "a*"'
zsh:typeset:1: not valid in this context: a*
```

The `-m` flag to `typeset` should accept a glob pattern and list
matching parameters. zshrs rejects the pattern as "not valid in
this context". Same with `typeset -mp PAT` (patterned print).

`unset -m PAT` and `unalias -m PAT` also potentially affected
(not yet exhaustively tested).

**Where** — `src/ported/builtin.rs::bin_typeset` `-m` flag handler
fails to switch the argument parser into pattern mode and instead
runs the standard "name[=value]" parser, which rejects `*`.

**Impact** — `typeset -m` is THE standard way to enumerate
parameters by pattern. Used heavily for introspection scripts,
shellcheck-like tools, debug helpers:

```sh
# Common pattern: dump all DEBUG_* vars
typeset -m 'DEBUG_*'   # zshrs: parse error

# Common pattern: remove all temp vars
unset -m '_TMP_*'     # zshrs: same issue if affected
```

Demo 305 uses `typeset -m 'report_*'` with `2>/dev/null` suppressing
the error — the demo silently produces empty output instead of
the matched variables.

**Workaround** — iterate manually:
```sh
for name in ${(k)parameters}; do
    [[ $name == a* ]] && echo "$name=${(P)name}"
done
```

---

## #49 — Quoted string comparison `(( "abc" == "abc" ))` returns false

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '(( "abc" == "abc" )); echo "exit=$?"'
exit=0

$ zshrs --zsh -c '(( "abc" == "abc" )); echo "exit=$?"'
exit=1
```

Inside `(( ... ))` arithmetic, both literal strings should be
looked up as variable names. `abc` is unset (= 0), so the
comparison is `0 == 0` = true (exit 0). zshrs returns false
(exit 1), treating the quoted strings as literal strings whose
value differs from numbers.

Unquoted form works identically in both shells:
  - `(( abc == abc ))` → both: exit 0 ✓

So the bug is specific to the QUOTED form's name-resolution path.

C-zsh `Src/math.c::mathevali` treats both `abc` and `"abc"`
identically — strips quotes during tokenization, then looks up as
variable name.

**Where** — `src/ported/math.rs` arith tokenizer should strip
double-quotes around identifiers and treat them as bare identifiers
for variable lookup.

**Impact** — defensive arithmetic with quoted variable names (used
when the variable might contain a name with spaces, even though
arith identifiers can't have spaces):
```sh
(( "$varname" == 5 ))    # zsh resolves $varname then lookups
                          # zshrs may fail depending on quoting timing
```

**Workaround** — drop the quotes:
```sh
(( abc == 5 ))         # works in both
```

---

## #50 — Trap set in outer scope doesn't fire on signal received inside function

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ cat > /tmp/t.zsh <<'EOF'
trap "echo OUTER" USR1
fn() {
    kill -USR1 $$
    sleep 0.1
}
fn
echo done
EOF

$ /opt/homebrew/bin/zsh /tmp/t.zsh
OUTER
done

$ ./target/debug/zshrs --zsh /tmp/t.zsh
done
```

A `trap` installed at the script's top level is not invoked when
the targeted signal arrives during a function call. zshrs swallows
the signal silently — `done` prints but `OUTER` never fires.

Signal traps installed inside subshells also don't fire reliably.

**Where** — `src/ported/signals.rs` signal-dispatch routine
checks the trap table in the current function frame but doesn't
fall back to the parent frame's trap table for inherited traps.
C-zsh's `Src/signals.c::dotrap` walks the frame stack.

**Impact** — critical for cleanup logic that depends on traps:
```sh
trap "rm -rf $tmpdir" EXIT INT TERM HUP
fn_that_might_be_killed
# zsh: cleanup runs on INT/TERM
# zshrs: cleanup may NOT run, leaving orphan tmpdirs
```

**Workaround** — re-install the trap inside each function that
might receive the signal:
```sh
fn() {
    trap "echo OUTER" USR1   # re-install in fn scope
    kill -USR1 $$
    sleep 0.1
}
```

---

## #51 — `${#*}` access corrupts `$@`/`$*` for subsequent use in same function

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ cat > /tmp/t.zsh <<'EOF'
fn() {
    echo "${#*}"
    for x in "$@"; do echo "[$x]"; done
}
fn a b c
EOF

$ /opt/homebrew/bin/zsh /tmp/t.zsh
3
[a]
[b]
[c]

$ ./target/debug/zshrs --zsh /tmp/t.zsh
3
[]
[]
[]
```

Accessing `${#*}` (positional-param count via `*` reference)
inside a function corrupts `$@` for the rest of the function.
The for-loop iterates the correct NUMBER of times (3) but each
iteration's `$x` is empty.

`${#@}` (with `@` instead of `*`) and bare `$#` both work
correctly — the bug is specific to the `${#*}` form.

Other access patterns through `${#*}`:
  - `local n=${#*}` → corrupts (3 empty loop iters)
  - `echo "${#*}"` → corrupts (3 empty loop iters)
  - `echo "${#*}suffix"` → corrupts
  - `echo "prefix${#*}"` → corrupts
  - `echo "X: ${#*}"` → corrupts (sometimes 0 iters instead of 3)
  - `${#@}` (alternative) → works correctly

**Where** — `src/ported/subst.rs` parameter-name resolution path
for `*` special parameter. The `${#*}` form likely shares state
with `$*` substitution and inadvertently empties the cached
positional-params array.

C-zsh's `Src/subst.c paramsubst` treats `${#@}` and `${#*}` as
synonyms for the COUNT (both should be `$#`), without mutating
the underlying parameter list.

**Impact** — quite narrow because most scripts use `${#@}` or
`$#`. But subtle for anyone copying bash patterns that use
`${#*}` (bash treats both identically and doesn't have this bug).

**Workaround** — use `${#@}` or `$#`:
```sh
fn() {
    echo "$#"           # zshrs: works
    # echo "${#@}"      # zshrs: also works
    # echo "${#*}"      # zshrs: BREAKS $@/$* below
    for x in "$@"; do
        echo "[$x]"
    done
}
```

---

## #52 — `${(q)arr}` on array doesn't join+quote — per-element quote only

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(foo "bar baz" qux); echo "${(q)a}"'
foo\ bar\ baz\ qux       # zsh: join then quote ALL spaces

$ zshrs --zsh -c 'a=(foo "bar baz" qux); echo "${(q)a}"'
foo bar\ baz qux         # zshrs: per-element quote only
```

Per zsh docs, the `(q)` flag (without `@` modifier) on an array
should:
  1. Join elements with the first character of `$IFS` (space)
  2. Quote ALL shell-special chars in the resulting STRING

So `(foo)(bar baz)(qux)` → join → `foo bar baz qux` → quote spaces
→ `foo\ bar\ baz\ qux`.

zshrs only quotes spaces that were INSIDE elements, treating the
join-separator spaces as unquotable. This makes the result
ambiguous if it's later re-split — the boundaries between original
elements are lost.

`${(@q)arr}` (per-element form) works identically in both shells —
that's not affected.

**Where** — `src/ported/subst.rs::paramsubst` should apply `(q)`
AFTER joining, not before. The early per-element quoting bypasses
the post-join quoting pass.

**Workaround** — explicitly use `(@q)` (per-element) and then
`(j)` to join:
```sh
echo "${(j: :)${(@q)a}}"   # explicit join after per-element quote
```

---

## #53 — `${(P)$ref}` indirect doesn't resolve `name[subscript]` references

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(x y z); ref="a[2]"; echo "${(P)ref}"'
y

$ zshrs --zsh -c 'a=(x y z); ref="a[2]"; echo "${(P)ref}"'
            # empty
```

The `(P)` flag dereferences a variable name held in another
variable (indirect expansion). When `$ref` holds a bare name
(`"a"`), both shells correctly expand to the array's value(s).
When `$ref` holds a subscripted name (`"a[2]"`, `"m[key]"`), zsh
parses the subscript and returns the element; zshrs returns empty.

Affects:
  - `${(P)$ref}` where `ref="arr[N]"` → empty in zshrs
  - `${(P)$ref}` where `ref="hash[key]"` → empty in zshrs

Works:
  - `${(P)$ref}` where `ref="varname"` → both shells correct

**Where** — `src/ported/subst.rs::paramsubst` `(P)` flag handler
should parse the indirect string as a full parameter expression
(including `[idx]`), not just a bare identifier. The subscript
part needs to flow through the parameter-lookup path.

**Impact** — limits the usefulness of `(P)` for building dynamic
references to array elements:
```sh
# Common pattern: store "config.PORT" type references:
keys=(server.host server.port db.name)
typeset -A config=(server.host localhost server.port 8080 db.name myapp)

for k in "${keys[@]}"; do
    ref="config[$k]"
    echo "$k = ${(P)ref}"   # zsh: works; zshrs: prints empty
done
```

**Workaround** — assign through eval:
```sh
ref="a[2]"
eval "val=\${$ref}"
echo "$val"           # works in both shells
```

---

## #54 — `setopt warn_create_global` and `warn_nested_var` don't emit warnings

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt warn_create_global
fn() { x=10; }
fn'
fn: scalar parameter x created globally in function fn

$ zshrs --zsh -c 'setopt warn_create_global
fn() { x=10; }
fn'
            # silent — no warning
```

Same with `warn_nested_var`:

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt warn_nested_var
outer() { local y=outer
    inner() { y=changed; }
    inner
}
outer'
inner: scalar parameter y set in enclosing scope in function inner

$ zshrs --zsh -c 'setopt warn_nested_var
outer() { local y=outer
    inner() { y=changed; }
    inner
}
outer'
            # silent
```

Both options exist to catch ACCIDENTAL globals/shadowing — a class
of bug very common in shell scripts. Real zsh emits a stderr
warning naming the function and variable. zshrs accepts the
option setopt silently (no error) but never produces any warning.

**Where** — `src/ported/exec.rs::add_param_to_scope` should check
`opts.warn_create_global` and `opts.warn_nested_var` flags
when creating/modifying parameters across scope boundaries. Likely
the option flags are recognized by `setopt` but never consulted
elsewhere in the codebase.

**Impact** — defensive scripts using these warnings catch nothing.
Common pattern:
```sh
setopt warn_create_global
source big_legacy_script.sh
# zsh: emits warnings for every accidental global
# zshrs: silent — scripts ship undetected globals
```

**Workaround** — adopt strict `local` discipline in code, since
the safety net is missing. Or run periodic audits via real zsh.

---

## #55 — `setopt err_return` doesn't return from function on command failure

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt err_return
fn() {
    false
    echo unreached
}
fn
echo "after"'
            # both prints suppressed — fn returns on false, script then
            # exits on the propagated non-zero status

$ zshrs --zsh -c 'setopt err_return
fn() {
    false
    echo unreached
}
fn
echo "after"'
unreached
after
```

`err_return` is the function-level equivalent of `set -e` /
`err_exit`: any command returning non-zero inside a function
should immediately return from that function. Real zsh implements
this; zshrs ignores the option entirely.

`err_exit` (`set -e`) is partially implemented in zshrs (works
for `false`, see bug #33 where it doesn't work for `(( ))`), but
`err_return` is silently inert.

**Where** — `src/ported/exec.rs::execcmd` after every command
execution should check `opts.err_return` and `funcdepth > 0`,
unwinding to the function frame on non-zero exit.

**Impact** — defensive functions relying on `err_return` to
short-circuit on internal failure run their full body anyway:
```sh
setopt err_return
deploy() {
    validate || return 1     # works
    build                     # zsh: bails if false; zshrs: continues
    test                      # zsh: skipped if build failed
    push                      # zsh: skipped
}
```

**Workaround** — explicit `|| return $?` after every command:
```sh
deploy() {
    build  || return $?
    test   || return $?
    push   || return $?
}
```

Or use `set -e` (`err_exit`) which DOES work in zshrs (with the
`(( ))` exception of bug #33).

---

## #56 — Signal trap fires INSIDE `$(...)` subshell instead of parent shell

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'trap "echo X" USR1
result=$(kill -USR1 $$; sleep 0.05; echo done)
echo "RESULT=[$result]"'
X
RESULT=[done]

$ zshrs --zsh -c 'trap "echo X" USR1
result=$(kill -USR1 $$; sleep 0.05; echo done)
echo "RESULT=[$result]"'
RESULT=[X
done]
```

When `$$` (parent PID) is signaled from inside `$(...)`, real zsh
delivers the signal to the PARENT — the trap fires there and `X`
prints to the parent's stdout, leaving only `done` in `$result`.

zshrs delivers/processes the signal inside the cmd-sub child
group, so the trap output gets captured into `$result` along with
the explicit echo.

**Where** — `src/ported/exec.rs::cmd_subst` should set up the
cmd-sub child as a separate process group such that `kill $$`
from inside still targets the parent process, not the cmd-sub
child group.

**Impact** — corrupts captured-output processing when a script
combines cmd-sub with defensive signal handling:
```sh
trap "echo 'CLEAN UP'" INT
config=$(load_config_with_timeout)
# zsh:  CLEAN UP prints to terminal if INT fires; config gets data
# zshrs: CLEAN UP mixed INTO config; subsequent parse breaks
```

**Workaround** — guard against unexpected stderr/stdout content
in cmd-sub results:
```sh
config=$(load_config_with_timeout 2>/dev/null)
[[ $config == CLEAN\ UP* ]] && fail "interrupted"
```

---

## #57 — `setopt octal_zeroes` doesn't trigger octal arith parsing

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt octal_zeroes; echo $((08))'
zsh:1: bad math expression: operator expected at `8'

$ zshrs --zsh -c 'setopt octal_zeroes; echo $((08))'
8
```

`setopt octal_zeroes` makes the arith parser treat `0NN` prefixes
as **octal**. So `08` is invalid (8 isn't a valid octal digit) and
real zsh correctly errors. zshrs ignores the option and parses
`08` as decimal 8.

  - `$((010))` → zsh `8` (octal), zshrs `10` (decimal)
  - `$((0755))` → zsh `493`, zshrs `755`

Default (no `octal_zeroes`): both treat `0NN` as decimal — that's
correct because `octal_zeroes` defaults off for ksh compatibility.

**Where** — `src/ported/math.rs::parse_number` doesn't consult
`opts.octal_zeroes` flag at the leading-`0` decision point.

**Impact** — POSIX-strict scripts doing UNIX permission math get
wrong values silently:
```sh
setopt octal_zeroes
perms=$((0755 | 0010))   # zsh: 0765 octal = 501 decimal
                          # zshrs: 755 | 10 decimal = 767 (wrong)
chmod $(printf "%o" $perms) /tmp/file   # zshrs: wrong perms applied
```

**Workaround** — use explicit base prefix `8#`:
```sh
perms=$(( 8#755 | 8#10 ))   # both shells: 501 decimal
```

---

## #58 — Quoted `*` on RHS of `[[ == ]]` still treated as glob

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '[[ "a*b" == "a*b" ]] && echo literal; [[ "a*b" == a\*b ]] && echo bksl'
literal
bksl

$ zshrs --zsh -c '[[ "a*b" == "a*b" ]] && echo literal; [[ "a*b" == a\*b ]] && echo bksl'
bksl
```

The first test compares `"a*b"` against the quoted RHS `"a*b"`. In
real zsh, quoting the RHS makes `*` literal — match succeeds, prints
`literal`. zshrs treats the `*` as a glob metachar even though it's
in double quotes, so it tries to match `"a*b"` (the lhs literal)
against the pattern `a*b` (with `*` = any). That should succeed too,
but zshrs evidently strips quotes too late and the match fails
silently.

The second test uses backslash escape (`a\*b`) and both shells
agree — `*` is literal, match succeeds.

**Where** — `src/ported/parse.rs` / `src/ported/cond.rs`: the `[[ ]]`
RHS pattern compiler doesn't honor quote-state from the lexer when
deciding whether `*` is metachar or literal. Real zsh keeps a
per-character "was this in quotes" flag (the Meta-char convention)
that survives into pattern compilation; zshrs flattens quotes too
early.

**Impact** — every `[[ "$x" == "$pat" ]]` test where `$pat` happens
to contain `*` / `?` / `[` from user data behaves differently than
zsh. Config validators, path-component sanity checks, regex-lite
matchers all fail or false-positive.

```sh
expected='log_*.txt'
got='log_*.txt'
[[ "$got" == "$expected" ]]   # zsh: true   zshrs: depends on what's in $log_
```

**Workaround** — escape metachars on RHS explicitly:
```sh
[[ "$got" == "${expected//\*/\\*}" ]]
```
Or use the pattern as bare-unquoted with deliberate backslashes:
`[[ "$got" == a\*b ]]`.

---

## #59 — `setopt no_clobber` allows `>>` to CREATE new file

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt no_clobber; echo a >> /tmp/zexa; echo b >> /tmp/zexa; cat /tmp/zexa'
zsh:1: no such file or directory: /tmp/zexa
zsh:1: no such file or directory: /tmp/zexa
cat: /tmp/zexa: No such file or directory

$ zshrs --zsh -c 'setopt no_clobber; echo a >> /tmp/zexa; echo b >> /tmp/zexa; cat /tmp/zexa'
a
b
```

Per `man zshoptions`:
> **NO_CLOBBER**  Prevents `>` redirection from truncating existing
> files. `>>` to a non-existent file is also an error unless
> `APPEND_CREATE` is set.

zshrs creates the file on `>>` even with `no_clobber` set and
`append_create` unset. Real zsh refuses, errors out.

**Where** — `src/ported/exec.rs` redirect handler for `O_APPEND`
mode: doesn't check `opts.no_clobber && !opts.append_create` before
adding `O_CREAT` to the open flags. Should `open(O_APPEND|O_WRONLY)`
without `O_CREAT`, then ENOENT propagates as the "no such file"
error.

**Impact** — POSIX-strict log-append scripts that rely on
`no_clobber` to catch typo'd paths silently create new garbage files
instead of erroring.

```sh
# author thinks they're appending to existing /var/log/svc.log
echo "$msg" >> /var/log/svc.lgo    # zsh: error (file doesn't exist)
                                    # zshrs: silently creates the typo'd file
```

**Workaround** — explicit existence check:
```sh
[[ -f $logf ]] || { echo "no such log: $logf" >&2; return 1; }
echo "$msg" >> $logf
```
Or set `setopt append_create` to explicitly opt-in to the
zshrs/permissive behavior.

---

## #60 — `function {body}` (no name) parses + echoes stray `}`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'function {echo "empty name"}'
zsh:1: parse error near `}'

$ zshrs --zsh -c 'function {echo "empty name"}'
empty name}
```

`function` keyword in zsh requires either a name or zero names
(anonymous function uses `() { body }` syntax, NOT `function
{ body }`). Real zsh treats `function {…}` as a parse error because
`{` is parsed as the start of the name, not as the body brace.

zshrs evidently splits `function` from `{echo`, treats `{echo` as
the function name, then runs the rest as if `echo` was a literal
command — but it leaks the closing `}` into the output, suggesting
the brace-parser is in some intermediate state.

**Where** — `src/ported/parse.rs::parse_function_def`: doesn't
require a name token between `function` and `{` brace. Should reject
when next token after `function` is `{`.

**Impact** — accepts malformed input that zsh rejects. Worse, the
stray `}` is echoed to stdout, so scripts that defensively wrap
`function foo { … }` in nested constructs may produce surprising
output if `function` is mis-typed without a name.

```sh
# typo: forgot the function name
function {
    echo "set up"
}                     # zsh: parse error on line 1
                       # zshrs: prints "set up\n}\n" to stdout
```

**Workaround** — always use `funcname() { … }` syntax (the POSIX
form) which both shells parse identically. Or use the explicit
anonymous form `() { body }` (no `function` keyword).

---

## #61 — `h["key"]=v` subscript form embeds literal quotes in the key

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h["k2"]=v2; typeset -p h'
typeset -A h=( ['"k2"']=v2 )

$ zshrs --zsh -c 'typeset -A h; h["k2"]=v2; typeset -p h'
typeset -A h=( [k2]=v2 )
```

When using the `name[subscript]=value` LHS-assignment form, real zsh
treats the `"` characters in the subscript as PART of the key string.
So `h["k2"]=v2` creates a key whose literal value is `"k2"` (5 chars
including quotes), retrievable only via `${h[\"k2\"]}`. The
parenthesised init form `h=( k2 v2 )` stores `k2` (2 chars).

zshrs strips quotes from the subscript first, so `h["k2"]=v2` stores
the same key as `h=( k2 v2 )` — divergent from the C source.

Confirmation that the keys round-trip in zsh:
```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h["k1"]=hello; echo "[${h[\"k1\"]}]"; echo "[${h[k1]}]"'
[hello]
[]
```

**Where** — `src/ported/lex.rs` / `src/ported/parse.rs`: subscript
lexer for `name[…]=` LHS strips matching quote pairs before storing
the key. The C source (`Src/subst.c::strpfx`/`Src/params.c::sethparam`)
treats the subscript bytes verbatim because quote-removal happens
during expansion, not parsing.

**Impact** — assoc tables populated via the bracketed-assignment
form behave differently between zsh and zshrs. Any code that assumes
`h["key"]` and `h[key]` are equivalent (the natural assumption)
works in zshrs and fails in zsh — but more importantly, code copied
FROM real-zsh that intentionally uses the literal-quote key trick
breaks under zshrs.

**Workaround** — always use the parenthesised init form
`typeset -A h=( k1 v1 k2 v2 )` or assign via a variable
`key=k1; h[$key]=v1` to avoid the ambiguity. Both work identically
in zsh and zshrs.

---

## #62 — `setopt extended_glob` doesn't recognize `~` (and-not) operator

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extended_glob; echo a*~ab'
zsh:1: no matches found: a*~ab

$ zshrs --zsh -c 'setopt extended_glob; echo a*~ab'
a*~ab
```

The `pat1~pat2` syntax in `extended_glob` means "match `pat1` but
exclude anything that matches `pat2`". Real zsh sees `a*~ab` as a
glob pattern, fails to match (no such files), and errors per the
default `nomatch` option.

zshrs doesn't recognize `~` as a glob metachar even with
`extended_glob` set, so it treats `a*~ab` as a literal string and
prints it.

**Where** — `src/ported/pattern.rs`: pattern compiler's
extended-glob token table missing `~` (PAT_TILDE) handling. C source
in `Src/pattern.c::patcompile` adds tilde-exclusion when
`isset(EXTENDEDGLOB)`.

**Impact** — any extended_glob script using `~` to exclude
sub-patterns silently degrades. Example:

```sh
setopt extended_glob
# delete all .log files except current.log
rm /var/log/*.log~current.log    # zsh: deletes everything except current.log
                                  # zshrs: tries to rm a literal "*.log~current.log" file
```

**Workaround** — invert with explicit loop:
```sh
for f in /var/log/*.log; do
    [[ "$f" == */current.log ]] && continue
    rm "$f"
done
```
Or use `find` with `-not`.

---

## #63 — `${(j:s:)${(s:t:)var}}` nested split-then-join returns first element only

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a="A B C"; echo "${(j:-:)${(s: :)a}}"'
A-B-C

$ zshrs --zsh -c 'a="A B C"; echo "${(j:-:)${(s: :)a}}"'
A
```

The inner `${(s: :)a}` splits `"A B C"` on spaces → `(A B C)` array.
The outer `${(j:-:)…}` joins that array on `-` → `A-B-C`.

zshrs collapses the inner array to its first element before the
outer flag sees it, returning just `A`.

**Where** — `src/ported/paramsubst.rs`: nested parameter expansion
doesn't propagate array-context flag (`PM_HASHED`/`PM_ARRAY`) from
the inner expansion to the outer. C source uses the
`Param->u.arr`/`scalarsplit` chain so the outer expansion sees the
inner as an array.

**Impact** — pipeline-style string transforms break:
```sh
# convert CSV to TSV by split-then-join
out="${(j:\t:)${(s:,:)csv_line}}"   # zsh: tab-separated
                                      # zshrs: first field only
```

Also breaks:
- `${(j:/:)${(s:.:)path}}` — replace `.` with `/`
- `${(@)${(s::)str}}` — convert string to char-array
- Any `${(flag2)${(flag1)var}}` two-stage transform

**Workaround** — intermediate array variable:
```sh
arr=( "${(s: :)a}" )
echo "${(j:-:)arr}"
```

---

## #64 — `$PIPESTATUS` (uppercase, bash-style) exists when it shouldn't

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'true | false | true; echo "pipestatus=[${pipestatus[@]}] PIPESTATUS=[${PIPESTATUS[@]}]"'
pipestatus=[0 1 0] PIPESTATUS=[]

$ zshrs --zsh -c 'true | false | true; echo "pipestatus=[${pipestatus[@]}] PIPESTATUS=[${PIPESTATUS[@]}]"'
pipestatus=[0 1 0] PIPESTATUS=[0 1 0]
```

zsh exports only the **lowercase** `$pipestatus`. The uppercase
`$PIPESTATUS` is the bash convention. zshrs populates BOTH, which:

1. Hides bugs in code that checks `$PIPESTATUS` (uppercase) and
   silently works under zshrs but breaks under real zsh.
2. Pollutes the user namespace — `PIPESTATUS=...` in user code is
   no longer a legal user-defined name in zshrs (it gets clobbered
   after every pipeline).

`${+PIPESTATUS}` proof:
```sh
$ /opt/homebrew/bin/zsh -fc 'echo "${+PIPESTATUS}"'
0
$ zshrs --zsh -c 'echo "${+PIPESTATUS}"'
1
```

Wait — actually zsh DOES report `${+PIPESTATUS}` as 0 only after no
pipeline has run. After a pipeline:

```sh
$ /opt/homebrew/bin/zsh -fc 'true | false; echo "${+PIPESTATUS}"'
0
```

So real zsh genuinely doesn't define `PIPESTATUS`. zshrs adds it as
an alias to `pipestatus`.

**Where** — `src/ported/exec.rs` pipeline epilogue / `src/ported/params.rs`
special-parameter table includes both `pipestatus` and `PIPESTATUS`.
C-zsh `Src/exec.c::execpline` only writes to `pipestatus` (the
PM_ARRAY param).

**Impact** — bash-compat scripts that check `$PIPESTATUS` work in
zshrs but not in real zsh. False sense of cross-shell portability.
Also, user code that intentionally sets `PIPESTATUS=...` for their
own purposes gets clobbered.

**Workaround** — explicitly use `${pipestatus[@]}` (lowercase) in
any code that needs to be zsh-portable, and never read
`$PIPESTATUS` in zsh.

---

## #65 — `${+EPOCHSECONDS}` returns 0 even after `zmodload zsh/datetime`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'zmodload zsh/datetime; echo "EPOCH=$EPOCHSECONDS plus=${+EPOCHSECONDS}"'
EPOCH=1780154288 plus=1

$ zshrs --zsh -c 'zmodload zsh/datetime; echo "EPOCH=$EPOCHSECONDS plus=${+EPOCHSECONDS}"'
EPOCH=1780154288 plus=0
```

`$EPOCHSECONDS` produces a value in both shells, but `${+VAR}`
membership check returns 0 (not-set) in zshrs even though
`zmodload zsh/datetime` was called.

Different from bug #31 (which was about `:-` default fallback being
ignored). This is the simpler `${+VAR}` parameter-defined-test
returning the wrong boolean.

**Where** — `src/ported/zmodload.rs::load_datetime`: the module-load
adds `EPOCHSECONDS` to the param table as a "computed" param but
doesn't set the `PM_SPECIAL`/`PM_DEFINED` flag that `${+VAR}` queries.
C-source `Src/Modules/datetime.c::bin_zmodload` calls
`createspecialhash`/`createparam` which sets `PM_TIED|PM_SPECIAL`.

**Impact** — every script doing the standard "is this module's API
available" check fails:

```sh
zmodload zsh/datetime 2>/dev/null
if (( ${+EPOCHSECONDS} )); then
    timestamp=$EPOCHSECONDS
else
    timestamp=$(date +%s)
fi
# zshrs: always takes the fallback path even though $EPOCHSECONDS
# would produce a usable value
```

Affects all module-provided special params: `EPOCHSECONDS`,
`EPOCHREALTIME`, `epochtime` (datetime), `mapfile` (mapfile),
`zstat` symbols, etc.

**Workaround** — direct access with `:-` won't work due to bug #31.
Use `whence EPOCHSECONDS` check or guard with module-load success:
```sh
if zmodload zsh/datetime 2>/dev/null; then
    ts=$EPOCHSECONDS
else
    ts=$(date +%s)
fi
```

---

## #66 — `time` builtin ignores `TIMEFMT` and omits `%J` (command name)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'TIMEFMT="USER=%U SYS=%S CPU=%P"; time sleep 0.05'
USER=0.00s SYS=0.00s CPU=3%

$ zshrs --zsh -c 'TIMEFMT="USER=%U SYS=%S CPU=%P"; time sleep 0.05'
0.04s user 0.01s system 80% cpu 0.064 total
```

zsh respects `$TIMEFMT` formatter for `time` builtin output. zshrs
ignores it entirely and uses a hardcoded English string.

Default format mismatch as well (no TIMEFMT override):
```sh
$ /opt/homebrew/bin/zsh -fc 'time sleep 0.05'
sleep 0.05  0.00s user 0.00s system 7% cpu 0.059 total

$ zshrs --zsh -c 'time sleep 0.05'
0.06s user 0.01s system 80% cpu 0.088 total
```

zsh prefixes with the command being timed (`%J` formatter:
`sleep 0.05  …`). zshrs drops `%J` entirely.

Default `$TIMEFMT` is the same string in both shells:
```
%J  %U user %S system %P cpu %*E total
```
But zshrs's `time` doesn't consume that format string at all.

**Where** — `src/ported/builtin_time.rs` (or wherever `time` lives):
output uses a fixed `format!` macro instead of consulting
`opts.timefmt`. C-source `Src/exec.c::printtime` walks the TIMEFMT
string interpreting `%J/%U/%S/%P/%*E/%K/%M/%X` etc.

**Impact** — every script that:
1. Customizes `$TIMEFMT` for parseable output (CSV-style, JSON,
   etc.) gets human-readable English instead.
2. Pipes `time foo 2>&1 | awk` looking for the command name field
   gets empty/wrong column data.
3. Reports timing for compound commands like `time { … }` — no
   command name to identify which block was timed.

**Workaround** — explicit `/usr/bin/time -f "..."` (the GNU
external) instead of the shell builtin:
```sh
/usr/bin/time -f "USER=%U SYS=%S" sleep 0.05
```
Or capture and reformat:
```sh
secs=$(zmodload zsh/datetime; t=$EPOCHREALTIME; sleep 0.05; \
       echo $(( EPOCHREALTIME - t )))
```

---

## #67 — `pushd` with no args doesn't swap top of dir stack

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'cd /tmp; pushd $HOME 2>&1; dirs; pushd 2>&1; dirs'
~ /tmp
/tmp ~

$ zshrs --zsh -c 'cd /tmp; pushd $HOME 2>&1; dirs; pushd 2>&1; dirs'
~ /tmp
/tmp ~ /tmp
```

`pushd` with no arguments should **swap** the top two entries on the
directory stack (POSIX/zsh semantics). zshrs PUSHES a duplicate
instead.

Also broken when stack has only one entry:
```sh
$ /opt/homebrew/bin/zsh -fc 'cd /tmp; pushd 2>&1; dirs'
~ /tmp                    # pushed HOME and swapped

$ zshrs --zsh -c 'cd /tmp; pushd 2>&1; dirs'
/tmp                      # no-op, exit 1
```

Per `man zshbuiltins/pushd`:
> `pushd` (without arguments) — exchange the top two entries of the
> directory stack. If only one entry exists, an alternative to
> `pushd $HOME` is performed.

**Where** — `src/ported/builtin_pushd.rs::no_args_branch`: missing
the "swap top two" code path and missing the "1-entry → push HOME"
fallback. Implementation appears to treat `pushd` with no args as
push-current-dir (which is wrong).

**Impact** — interactive workflow `cd /a; pushd /b; pushd` to bounce
between two directories doesn't work. Common shell ergonomics
broken.

**Workaround** — explicit two-arg `pushd $OLDPWD` to swap back:
```sh
cd /a
pushd /b
pushd $OLDPWD    # both shells: bounce back to /a
```

---

## #68 — `trap` listing prints in insertion order, not signal number

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'trap "echo h" HUP; trap "echo u" USR1; trap "echo c" INT; trap "echo t" TERM; trap'
trap -- 'echo h' HUP
trap -- 'echo c' INT
trap -- 'echo t' TERM
trap -- 'echo u' USR1

$ zshrs --zsh -c 'trap "echo h" HUP; trap "echo u" USR1; trap "echo c" INT; trap "echo t" TERM; trap'
trap -- 'echo u' USR1
trap -- 'echo h' HUP
trap -- 'echo c' INT
trap -- 'echo t' TERM
```

Signal numbers: HUP=1, INT=2, TERM=15, USR1=30 (macOS). zsh prints
the trap table sorted by signal number, so HUP→INT→TERM→USR1.
zshrs prints in some other order — possibly hash-iteration or
insertion (the USR1 entry came out first despite being defined
third).

**Where** — `src/ported/builtin_trap.rs::list_traps`: iterates
`HashMap<signal_name, action>` directly. C-source `Src/jobs.c`
prints by walking signal-number array `SIGCOUNT` order.

**Impact** — diff-based comparison of `trap` output between zsh
and zshrs fails. Tests/scripts that capture `trap` for inspection
or persistence (`trap_snapshot=$(trap)`) get non-deterministic
results in zshrs (HashMap iteration is unspecified order in Rust).

```sh
# scripts comparing trap state across runs
expected_traps=$(trap)
... do stuff ...
new_traps=$(trap)
[[ "$expected_traps" == "$new_traps" ]] || alert "traps changed"
# zsh: stable                  zshrs: false-positive on every run
```

**Workaround** — pipe through `sort` to normalize order:
```sh
new_traps=$(trap | sort)
```

---

## #69 — `$sysparams` auto-loaded without `zmodload zsh/system`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "${+sysparams}"'
0

$ zshrs --zsh -c 'echo "${+sysparams}"'
1
```

`$sysparams` is provided by the `zsh/system` module. In real zsh
it's undefined until `zmodload zsh/system` runs. zshrs has it
populated at startup unconditionally — exposing `sysparams[pid]`,
`sysparams[ppid]`, etc., without the explicit module load.

Same family as bug #64 (`$PIPESTATUS`): zshrs eagerly exports
module-provided params into the global namespace.

`(t)` type confirms:
```sh
$ zshrs --zsh -c 'echo "${(t)sysparams}"'
association-hide-hideval-special

$ /opt/homebrew/bin/zsh -fc 'echo "${(t)sysparams:-NOT_DEFINED}"'
NOT_DEFINED
```

**Where** — `src/ported/init.rs` / `src/ported/params.rs`: special
param table seeds `sysparams` (and likely `mapfile`, `usergroups`,
etc.) at shell init instead of waiting for the `zmodload` of their
parent module.

**Impact** — same as #64. Code that defensively does:

```sh
if (( ! ${+sysparams} )); then
    zmodload zsh/system 2>/dev/null
fi
```

never enters the `then` branch under zshrs and may not realize the
module-load step was needed. Cross-shell scripts that explicitly
gate on the module-defined flag misbehave.

Also: user code that uses `sysparams` as an own variable name gets
clobbered by the auto-defined special.

**Workaround** — always call `zmodload zsh/system` regardless of
`${+sysparams}` check. The zmodload is idempotent and ensures
correct semantics in both shells.

---

## #70 — Filesystem watcher leaks newly-created paths to stderr

**Status:** `port-bug` — surfaced 2026-05-30 hunting. Severe.

Reproducer:
```sh
$ ./target/debug/zshrs --zsh -c '
cd /tmp/zwatch
touch a b c
sleep 0.3
touch d e f
sleep 0.3
echo done' 2>/tmp/zee 1>/dev/null

$ cat /tmp/zee
/tmp/zee
/tmp/zwatch/a
/tmp/zwatch/f
/tmp/zwatch/c
/tmp/zwatch/d
/tmp/zwatch/e
/tmp/zwatch/b
/tmp/zwatch
```

A background filesystem watcher (likely fed into the completion
cache or some indexing service) writes every newly-created file
path under watched directories to **stderr**. The redirect target
file `/tmp/zee` itself appears in the output, as do all files the
script created during its run.

zsh writes nothing to stderr for the same command.

**Where** — `src/index/watcher.rs` (or wherever the FS watcher
lives): the `notify::Event` debug printer is emitting via
`eprintln!` instead of `tracing::debug!`. CLAUDE.md "no info to
stdout/stderr" rule explicitly forbids this: *"Informational
chatter goes to log only. If you find yourself adding a `println!` /
`eprintln!` outside of (a) error reporting on stderr, (b) explicit
user-requested output, or (c) what the user's script printed —
convert it to `tracing::info!` / `tracing::debug!` instead."*

Same family as #23 (worker-pool shutdown INFO leaks to stdout) but
with severe stderr pollution, not just at shutdown.

**Impact** — every script that does `cmd 2>/var/log/err.log` to
capture errors gets a flood of file paths the indexer touched.
Beyond noise: this is a **privacy leak** — any file path the shell
process creates, accesses, or watches gets exposed to wherever
stderr lands (cron mail, sentry, log files, CI logs, IDE terminals).

```sh
# user expects only error output
gpg --encrypt secret_doc.txt 2>/tmp/audit.log
# /tmp/audit.log now contains "secret_doc.txt" path AND all the
# temp files gpg created during encryption
```

Also breaks `2>&1 | grep ERROR` pipelines (every file path matches
spurious greps).

**Workaround** — none from user side (can't disable a background
watcher externally). Must be fixed in zshrs: route notify events
through `tracing::debug!` so they go to `~/.cache/zshrs/zshrs.log`
instead of stderr.

---

## #71 — `${var:N:M}` substring accepts non-digit-leading offset (bashism)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 's="abcdef"; n=1; echo "${s:n:3}"'
zsh:1: unrecognized modifier `n'

$ zshrs --zsh -c 's="abcdef"; n=1; echo "${s:n:3}"'
bcd
```

zsh's `${var:start:length}` substring syntax requires `start` to
begin with a digit, `+`, `-`, or `(`. Otherwise `:n` is parsed as a
**history modifier** (which is what `n` is — the
"un-anchored-substring" modifier of `:s`/`:r`/`:t` etc.) and
errors.

zshrs treats any expression as a substring offset, accepting
variable names as offsets directly. This is a bash extension.

Per `man zshparam`:
> `${name:offset}` `${name:offset:length}` — both forms require
> offset to be an arithmetic expression that begins with a digit
> or one of the characters `+`, `-`, `(`.

**Where** — `src/ported/paramsubst.rs::parse_substring_offset`:
should require the leading byte to be in `[0-9+\-(]` set, and fall
through to modifier-parsing otherwise. C-source
`Src/subst.c::getstring` does this dispatch.

**Impact** — bashisms work silently in zshrs that don't work in zsh.
Cross-shell scripts that rely on zsh's stricter parsing to catch
typos won't catch them in zshrs. Worse: an intentional zsh
modifier like `${path:r}` would be mis-parsed by zshrs as a
substring if user writes `${path:r:3}` expecting "the `r`
modifier"; zshrs would silently treat `r:3` as substring spec.

```sh
# zsh-style modifier — works in zsh, mis-interpreted by zshrs
path=/usr/local/bin/cmd.txt
echo "${path:r:3}"    # zsh: errors           zshrs: substring "loc"
```

**Workaround** — always wrap variable offsets in `$(( ))` or `(())`:
```sh
n=1
echo "${s:$((n)):3}"    # both shells: bcd
```
Or use parens: `${s:(n):3}` works in zsh.

---

## #72 — `log` builtin registered but execution dispatches to `/usr/bin/log`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'log; echo "exit=$?"'
exit=0

$ ./target/debug/zshrs --zsh -c 'log'
usage:
    log <command>

global options:
    -?, --help
    ...
$ echo "exit=$?"
exit=64
```

Both shells report `log` as a builtin via `whence -v log`:
```sh
$ /opt/homebrew/bin/zsh -fc 'whence -v log'
log is a shell builtin

$ ./target/debug/zshrs --zsh -c 'whence -v log'
log is a shell builtin
```

But when actually executed, zshrs runs the macOS system utility
`/usr/bin/log` (which shows its own usage and exits with code 64
because no args), while zsh runs its own builtin that simply shows
the `$WATCH` variable contents (exit 0 with no args).

So the builtin **table entry exists** but the **dispatch logic** is
broken — invocation falls through to PATH lookup instead of the
registered builtin function.

zsh's `log` builtin shows the value of `$WATCH` (a list of users to
watch for login activity), per `man zshbuiltins`:
> `log` — list users currently logged on who are affected by the
> current setting of the `watch` parameter.

**Where** — `src/ported/builtin_log.rs` is registered in the builtin
table (which is why `whence` reports it) but its exec function is
either stubbed-out or absent, so the dispatcher falls through to
external lookup. C-source `Src/builtin.c::bin_log` does the actual
WATCH display.

**Impact** — every script that calls `log` on macOS unexpectedly
runs the system log-archive utility instead of the zsh builtin.
Exit codes are wildly different (0 vs 64), output is wildly
different, and on Linux/BSD where `/usr/bin/log` doesn't exist,
zshrs errors with command-not-found while zsh runs cleanly.

**Workaround** — explicitly use `print -- $watch` to inspect the
WATCH list. Or `disable log` to remove the builtin from zshrs's
table so external lookup is the deliberate behavior (already-broken
state).

---

## #73 — `$ZSH_VERSION` reports `5.9.0.3-test` (custom suffix), not `5.9`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[$ZSH_VERSION]"'
[5.9]

$ ./target/debug/zshrs --zsh -c 'echo "[$ZSH_VERSION]"'
[5.9.0.3-test]
```

zshrs appends `.0.3-test` to its reported `$ZSH_VERSION`. zsh sets
this exactly to `5.9` (or whatever upstream is installed).

**Where** — `src/ported/version.rs` / `src/ported/init.rs`: the
`ZSH_VERSION` special-param initializer uses zshrs's `CARGO_PKG_*`
build metadata instead of mirroring upstream zsh's `VERSION` macro
from `Src/zsh.h`.

**Impact** — every `.zshrc`-style guard parsing `$ZSH_VERSION` to
gate features fails:

```sh
# common idiom in dotfiles
ver=${ZSH_VERSION%%.*}     # major version
[[ $ver -ge 5 ]] && setopt ...
# zsh: ver=5 -> ok
# zshrs: ver=5 -> ok (lucky in this case)
```

But finer parses break:
```sh
# parse all four version components
IFS=. read -r maj min pat <<< "$ZSH_VERSION"
# zsh: maj=5 min=9 pat=
# zshrs: maj=5 min=9 pat=0  (extra component captured into pat)
```

And:
```sh
# detect zsh-vs-zshrs
[[ "$ZSH_VERSION" == *test* ]] && echo "zshrs"  # works for zshrs
[[ "$ZSH_VERSION" == "5.9" ]] && echo "zsh exact"  # fails on zshrs
```

If the goal is identity-divergence (so `.zshrc` can detect zshrs vs
zsh), the variable to inspect should be `$ZSH_NAME` or a dedicated
`$ZSHRS_VERSION` — NOT clobbering the upstream-compat
`$ZSH_VERSION`.

**Where** — `src/ported/init.rs::set_zsh_version`: should set the
value verbatim to the C-zsh version string. zshrs identity should
land in a separate parameter so the compat-floor invariant holds.

**Workaround** — code that needs the "real" version can parse
`${ZSH_VERSION%%.*-test*}` or use `${ZSH_VERSION%%.[0-9].[0-9]-test}`.
But the right fix is upstream.

---

## #74 — `local -r` violation in function doesn't abort, continues execution

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { local -r x=5; x=10; echo "after assign in fn"; }; f; echo "after fn call"'
f: read-only variable: x

$ ./target/debug/zshrs --zsh -c 'f() { local -r x=5; x=10; echo "after assign in fn"; }; f; echo "after fn call"'
f:1: read-only variable: x
after fn call
```

Two bugs in one:

1. **Error format**: zsh prints `f: read-only ...` (function name).
   zshrs prints `f:1: read-only ...` (function name + line number).
   The `:1:` is a zshrs addition that doesn't match upstream format.

2. **Execution continuation**: zsh aborts the script after the
   read-only violation (no `after fn call` line). zshrs prints the
   trailing `after fn call` line, meaning the function returned
   normally (and the script continued).

Per zsh semantics, a read-only assignment inside a function should:
- print the error
- return from the function with non-zero status
- under `set -e`, also abort the script

zshrs prints, sets exit status, but doesn't abort the function
itself. The remaining body of `f` (the `echo "after assign in fn"`
line) IS skipped in both shells — but only zsh propagates the
abort up to the script level.

Global readonly violations DO abort correctly in zshrs:
```sh
$ ./target/debug/zshrs --zsh -c 'readonly Y=5; Y=10; echo "still alive"'
zsh:1: read-only variable: Y
# (no "still alive" — abort works)
```

So the bug is specifically the function-scoped `local -r` path.

**Where** — `src/ported/builtin_typeset.rs::assign_to_readonly`:
the function-level readonly violation path returns instead of
propagating an abort flag. C-source `Src/params.c::setsparam` sets
`errflag |= ERRFLAG_ERROR` which the executor checks.

**Impact** — scripts relying on `local -r` to enforce invariants
during function execution can silently continue past a violation,
producing wrong results downstream.

**Workaround** — `set -e` and check return status after every
function call — but `set -e` interactions with `local -r` aren't
fully tested either (related to bug #33).

---

## #75 — `typeset -i x; x="bad math"` silently coerces to 0

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -i x=42; x="abc def"; echo "still alive: x=$x"'
zsh:1: bad math expression: operator expected at `def'

$ ./target/debug/zshrs --zsh -c 'typeset -i x=42; x="abc def"; echo "still alive: x=$x"'
still alive: x=0
```

Assigning a string that's not a valid arithmetic expression to an
integer-typed variable should:
- in zsh: print "bad math expression" and abort the script (since
  not in an `if`/`while` test context).
- in zshrs: silently coerce to 0 and continue.

Single-token non-numeric strings ARE handled the same in both
(treated as variable name → 0):
```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -i x=42; x="abc"; echo "[$x]"'
[0]

$ ./target/debug/zshrs --zsh -c 'typeset -i x=42; x="abc"; echo "[$x]"'
[0]
```

But multi-token strings like `"abc def"` or `"not a number"` should
trigger the arith parser's "operator expected" error, which zshrs
silently swallows.

**Where** — `src/ported/math.rs::parse_expression`: doesn't return
an error when arith parsing fails to consume the entire input
(remaining tokens after a successful primary expression). C-source
`Src/math.c::matheval` checks `*str != '\0'` after parsing and
flags the leftover as "operator expected".

**Impact** — silent data corruption. Scripts that defensively type
their counters as `typeset -i` and feed them from user input lose
the type-safety net:

```sh
typeset -i counter=0
read user_input <<< "abc def"
counter=$user_input    # zsh: errors      zshrs: counter=0 silently
# downstream loops run 0 times instead of erroring out
```

**Workaround** — explicit arith eval with validation:
```sh
if [[ "$user_input" =~ '^-?[0-9]+$' ]]; then
    counter=$user_input
else
    echo "bad input: $user_input" >&2
    return 1
fi
```

---

## #76 — `zmodload` (no args) reports 32 auto-loaded modules vs zsh's 1

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'zmodload | wc -l'
       1

$ ./target/debug/zshrs --zsh -c 'zmodload | wc -l'
      32
```

zsh-bare reports only `zsh/main` loaded. zshrs reports 32 modules
eagerly loaded at shell init:

```
zsh/zpty zsh/datetime zsh/zselect zsh/zutil zsh/attr zsh/curses
zsh/files zsh/watch zsh/langinfo zsh/pcre zsh/regex zsh/zftp
zsh/mapfile zsh/zprof zsh/termcap zsh/parameter zsh/computil
zsh/net/tcp zsh/complist zsh/net/socket zsh/mathfunc zsh/cap
zsh/clone zsh/param/private zsh/terminfo zsh/nearcolor
zsh/db/gdbm zsh/complete zsh/stat zsh/sched zsh/zleparameter
zsh/system
```

Also, the `zsh/main` entry is missing from zshrs's list — every
zsh shell reports `zsh/main` as the always-present base module.

This is the **master bug** for the eager-loading family — #64
(PIPESTATUS from `zsh/pipestatus`-like), #65 (EPOCHSECONDS without
explicit zmodload), #69 (sysparams from zsh/system). All those
manifest because zshrs auto-loads everything.

**Where** — `src/ported/init.rs::register_modules`: builds the
module registry by pre-loading every module the binary has linked
in. C-source `Src/init.c::init_main` only registers `zsh/main`;
other modules wait for explicit `zmodload`.

**Impact** — startup time penalty (loading 32 modules vs 1 — TCP,
FTP, curses, GDBM all loaded even for shell scripts that need none
of them). Plus all the namespace pollution from special params
(#64, #69) and option flags from those modules.

Real cost on macOS with debug build: every `zshrs -c '...'`
invocation pays the 32-module init time, which is significant for
sub-100ms shell-script orchestration loops.

**Workaround** — none. The eager loading is at binary init time.
Must be fixed in zshrs by switching to lazy-load (init module
metadata table but defer dlopen/symbol-bind until first
`zmodload zsh/xxx`).

---

## #77 — `${h[(k)-key]}` flag-lookup of leading-dash key returns empty

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h=( [-a]=val1 ); echo "1: ${h[-a]}"; echo "2: ${h[(k)-a]}"'
1: val1
2: val1

$ ./target/debug/zshrs --zsh -c 'typeset -A h=( [-a]=val1 ); echo "1: ${h[-a]}"; echo "2: ${h[(k)-a]}"'
1: val1
2:
```

Direct subscript `${h[-a]}` works in both — `val1` retrieved.
But the **(k)-flag form** `${h[(k)-a]}` returns empty in zshrs.

`(k)` is "find the key by name match" — it's redundant for an
assoc unless used with patterns or negative indexing. In zsh it's
robust enough to look up `-a` directly.

zshrs's `(k)` subscript-flag handler appears to interpret `-a` as
a flag/option (because it starts with `-`) rather than a key
literal.

**Where** — `src/ported/paramsubst.rs::parse_subscript_flags`: the
`(k)` flag's key argument is consumed by an `extern crate clap`-
style arg parser that treats `-a` as a flag name, not as a string.
Should be a positional argument capture, not flag parsing.

**Impact** — anything that uses `zparseopts -A opts` then reads
back via the `(k)` flag (the documented portable way to test
membership) fails to find dash-prefixed option keys:

```sh
zmodload zsh/zutil
zparseopts -E -A opts a: b c
# opts contains [-a]=val, [-b]='', [-c]=''
for opt in -a -b -c; do
    if (( ${+opts[(k)$opt]} )); then
        echo "saw $opt"
    fi
done
# zsh: prints "saw -a", "saw -b", "saw -c"
# zshrs: prints nothing
```

**Workaround** — direct subscript without `(k)` flag works:
```sh
[[ -n "${opts[$opt]+set}" ]] && echo "saw $opt"
```

---

## #78 — `echoti` output emitted AFTER next command's stdout (buffer flush ordering)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echoti cup 0 0; echo done' | od -c
0000000  033   [   1   ;   1   H   d   o   n   e  \n
                                ^^^ cursor sequence FIRST
                                                ^^^ then "done"

$ ./target/debug/zshrs --zsh -c 'echoti cup 0 0; echo done' | od -c
0000000    d   o   n   e  \n 033   [   1   ;   1   H
                ^^^ "done" FIRST
                                ^^^ then cursor sequence (wrong order!)
```

`echoti` writes terminfo escape sequences to stdout. zshrs's
implementation buffers the output and flushes AFTER the next
command's stdout writes, reversing the intended order.

Anything depending on order — terminal positioning, color codes
followed by content — comes out scrambled.

**Where** — `src/ported/builtin_echoti.rs::output_termcap`:
writes via a buffered `BufWriter` that doesn't flush before
returning control to the next builtin. C-source
`Src/Modules/termcap.c::output_termcap` writes via the `outsh`
unbuffered output func.

**Impact** — terminal manipulation idioms broken:

```sh
echoti cup 5 10    # move cursor to row 5, col 10
print "label"      # write label at that position
# zsh: label appears at (5,10)
# zshrs: label appears at original cursor, then jumps to (5,10) at end
```

```sh
echoti setaf 1     # set foreground red
print "red text"
echoti sgr0        # reset
print "normal"
# zsh: red text, then normal
# zshrs: all text appears before any color codes — output garbled
```

`echotc` (the termcap variant) has the same problem since both
share the buffered-writer path.

**Workaround** — explicit fflush via `<&0` no-op or pipe through
`cat -u`:
```sh
echoti cup 5 10
print -u 2 ""       # write to stderr to bypass stdout buffering
```
Or use `printf '\e[%d;%dH' 6 11` directly to avoid the builtin.

---

## #79 — Job control table empty: `jobs`, `wait %N`, `kill %N`, `disown` all fail

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'sleep 0.5 & jobs; wait'
[1]  + running    sleep 0.5

$ ./target/debug/zshrs --zsh -c 'sleep 0.5 & jobs; wait'
(empty)
```

zshrs spawns the background process correctly (and `$!` returns
its PID), but doesn't register the process in the **jobs table**.
This breaks every `%N` jobspec-based builtin:

```sh
$ ./target/debug/zshrs --zsh -c 'sleep 0.5 & wait %1; echo "exit=$?"'
zsh:wait:1: %1: no such job
exit=1

$ ./target/debug/zshrs --zsh -c 'sleep 1 & kill %1' 2>&1
zsh:kill:1: %1: no such job

$ ./target/debug/zshrs --zsh -c 'sleep 0.5 & disown %1' 2>&1
(empty — should still error on "no such job" if table absent)
```

zsh's behavior:
```sh
$ /opt/homebrew/bin/zsh -fc 'sleep 0.5 & wait %1; echo "exit=$?"'
exit=0

$ /opt/homebrew/bin/zsh -fc 'sleep 1 & kill %1' 2>&1
(no output, kill succeeds)
```

**Where** — `src/ported/exec.rs::spawn_background`: should populate
`shell.jobs[next_job_id] = JobEntry { pid, cmd, state }` after
fork. C-source `Src/jobs.c::addproc` builds the job entry. zshrs's
forker only updates `$!` and exits.

**Impact** — job-control idioms broken:
- `sleep 1 &` then `kill %1` to cancel a timer
- `cmd &` then `wait $!` works (by PID) but `wait %1` doesn't
- `jobs -p` returns no PIDs
- Shell prompt `%j` count of jobs always 0

Most non-interactive scripts work around this with `$!`, but
interactive job-control workflows (Ctrl-Z, `fg`, `bg`, `jobs -l`)
are entirely broken.

**Workaround** — use `$!` to capture PID at backgrounding time,
then `wait $pid` / `kill $pid` by PID:
```sh
sleep 1 &
bgpid=$!
kill $bgpid    # works in both shells
```

---

## #80 — `trap EXIT` inside fn fires at SCRIPT exit, not fn exit (and lost entirely in nested fns)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { trap "echo bye-fn" EXIT; echo "inside"; }; f; echo "after"'
inside
bye-fn
after

$ ./target/debug/zshrs --zsh -c 'f() { trap "echo bye-fn" EXIT; echo "inside"; }; f; echo "after"'
inside
after
bye-fn
```

Per zsh semantics, a `trap "..." EXIT` set inside a function fires
when **that function exits** (return path), not at shell exit. zsh
prints `bye-fn` between `inside` and `after`. zshrs delays it until
after `after` (i.e., treats it as the global shell EXIT trap).

Worse, in nested functions, zshrs **loses** the inner trap entirely:

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { trap "echo f-exit" EXIT; g; }; g() { trap "echo g-exit" EXIT; echo "in g"; }; f; echo "post"'
in g
g-exit
f-exit
post

$ ./target/debug/zshrs --zsh -c 'f() { trap "echo f-exit" EXIT; g; }; g() { trap "echo g-exit" EXIT; echo "in g"; }; f; echo "post"'
in g
post
g-exit
```

zsh fires both traps in LIFO at each function return: `in g →
g-exit (g returns) → f-exit (f returns) → post`. zshrs prints only
one trap (`g-exit`) at script exit, and **`f-exit` is silently
dropped**.

**Where** — `src/ported/builtin_trap.rs::set_trap_exit`: traps
installed inside function scope should be registered to a
**function-local trap stack** (per-frame). C-source
`Src/exec.c::execfuncdef` saves the prior EXIT trap on function
entry and restores+fires the new one on function return. zshrs
appears to globally clobber.

**Impact** — cleanup code in functions never runs in the right
order. Patterns broken:

```sh
with_lock() {
    local lockfile=$1
    touch $lockfile
    trap "rm -f $lockfile" EXIT
    real_work
}
# zsh: rm fires at end of with_lock
# zshrs: rm fires at end of SHELL (if at all); lockfile leaks
#        between with_lock invocations
```

```sh
with_tmp() {
    local tmp=$(mktemp)
    trap "rm -f $tmp" EXIT
    process_into $tmp
}
# zsh: tmp removed at end of with_tmp
# zshrs: tmp never removed by trap; manual cleanup needed
```

**Workaround** — explicit cleanup at function epilogue:
```sh
with_tmp() {
    local tmp=$(mktemp)
    process_into $tmp
    rm -f $tmp     # manual cleanup, no trap reliance
}
```
Or `zshexit` hook for shell-level cleanup, function-local cleanup
via early-return wrappers.

---

## #81 — Glob with `extended_glob ~` exclusion produces duplicates + matches dir itself

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extended_glob; print -l /tmp/zgq/*~*b*'
/tmp/zgq/a
/tmp/zgq/c

$ ./target/debug/zshrs --zsh -c 'setopt extended_glob; print -l /tmp/zgq/*~*b*'
/tmp/zgq/a
/tmp/zgq/c
/tmp/zgq/a
/tmp/zgq/c
/tmp/zgq/b
/tmp/zgq
```

Files in `/tmp/zgq/`: `a b c`. Pattern `*~*b*` should match
"anything except names containing b" → `a c`.

zsh returns exactly that. zshrs returns:
1. `a c` (correct match) — but DUPLICATED
2. `b` — the supposedly-excluded file
3. `/tmp/zgq` — the directory itself

The glob engine appears to run the match TWICE (yielding the dups)
AND the `~` exclusion is partially honored (b appears once not
twice, but still appears) AND there's a separate code path that
matches the parent directory.

Related to #62 (extended_glob `~` operator not honored at all in
some contexts) but a different and worse manifestation.

**Where** — `src/ported/glob.rs::expand_pattern`: pattern with
embedded `~` is parsed into multiple sub-patterns that are matched
independently and unioned. C-source `Src/pattern.c::pattrylit`
applies `~` as a single AND-NOT operator on the same set.

**Impact** — every `extended_glob` use case with `~` returns wrong
results. Common idiom for "delete everything except current.log":

```sh
rm /var/log/*.log~current.log
# zsh: deletes all *.log except current.log (correct)
# zshrs: deletes all *.log INCLUDING current.log (and possibly
#        the /var/log dir too)
```

That's destructive — data loss potential.

**Workaround** — explicit loop with `[[ ... == *...* ]]` skip:
```sh
for f in /var/log/*.log; do
    [[ "$f" == */current.log ]] && continue
    rm "$f"
done
```

---

## #82 — `"PREFIX${(s.X.)var}"` repeats prefix per split element inside double quotes

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 's="a b c"; echo "P:${(s. .)s}"'
P:a b c

$ ./target/debug/zshrs --zsh -c 's="a b c"; echo "P:${(s. .)s}"'
P:a P:b P:c
```

Inside double quotes, `${(s.X.)var}` should produce a **scalar**
joined back by the default IFS (per zsh's quoted-expansion rules
for split flags). zsh joins as `a b c` (one word). zshrs treats it
as an **array** and applies the literal prefix to EACH element,
producing `P:a P:b P:c` (three words).

This is the fundamental "split flag inside `"..."` produces array
context" bug. For normal `"${a[@]}"` patterns the behavior matches
(both produce word-joined scalars or per-element words depending
on `[@]` vs `[*]`):

```sh
$ both-shells -fc 'a=(red blue green); echo "P:$a"'
P:red blue green        # zsh and zshrs agree
```

But the `(s.X.)` flag specifically diverges:

```sh
$ /opt/homebrew/bin/zsh -fc 's="a b c"; print -r -- "P:${(s. .)s}"'
P:a b c

$ ./target/debug/zshrs --zsh -c 's="a b c"; print -r -- "P:${(s. .)s}"'
P:a P:b P:c
```

**Where** — `src/ported/paramsubst.rs::apply_split_flag`: split
flag produces a true `Vec<String>` even inside `"..."` context;
should collapse back to scalar (join by space) when expansion site
is within a quote scope. C-source `Src/subst.c::dosplit` checks
the `IS_INSIDE_QUOTES` flag and joins accordingly.

**Impact** — every string-transform idiom using `(s)` inside `"..."`
prefix construction breaks:

```sh
# build sql IN clause from comma-separated list
csv="a,b,c"
sql="VALUES (${(s.,.)csv})"
# zsh: VALUES (a b c)
# zshrs: VALUES (a VALUES (b VALUES (c)
```

Same family as bug #63 (`${(j:s:)${(s:t:)var}}` nested
split-then-join returns first element only) — both are
context-propagation bugs in paramsubst.

**Workaround** — explicit array assignment then expansion:
```sh
arr=("${(s. .)s}")
echo "P:${arr[*]}"     # both shells: P:a b c
```

---

## #83 — `${a[(s.,.)N,M]}` array slice with subscript flag returns full array

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(1 2 3 4 5 6 7 8 9 10); echo "${a[(s.,.)3,7]}"'
3 4 5 6 7

$ ./target/debug/zshrs --zsh -c 'a=(1 2 3 4 5 6 7 8 9 10); echo "${a[(s.,.)3,7]}"'
1 2 3 4 5 6 7 8 9 10
```

The `(s.X.)` subscript flag specifies a separator for the array
when it's joined into a string before subscripting. With `3,7` it
should still produce elements 3 through 7. zsh respects the range.
zshrs ignores the `N,M` range entirely and returns the full array.

Without the flag, both shells return `3 4 5 6 7`. So the bug is
specifically the interaction of subscript flag + range — zshrs
appears to discard the integer-pair indices when a string-flag is
present.

**Where** — `src/ported/paramsubst.rs::parse_subscript_with_flags`:
when a flag like `(s.X.)` is detected, the subsequent `N,M` pair
isn't parsed as a range; the entire expansion falls through to
"return all".

**Impact** — defensive code that combines flag-based array
operations with explicit ranges (a common zsh idiom) silently
returns wrong results:

```sh
log_lines=(${(f)"$(< /var/log/syslog)"})    # split file by lines
recent=( "${log_lines[(s.\n.)-100,-1]}" )    # last 100 lines
# zsh: recent has 100 entries
# zshrs: recent has ALL entries
```

**Workaround** — compute range without the flag:
```sh
recent=("${log_lines[-100,-1]}")           # both shells: last 100
```

---

## #84 — Default `bindkey -L` outputs 117 individual entries vs zsh's 31 ranged entries

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'bindkey -L | wc -l'
31

$ ./target/debug/zshrs --zsh -c 'bindkey -L | wc -l'
117
```

zsh prints compact `-R` range entries:
```
bindkey -R "^A"-"^C" self-insert
bindkey "^D" list-choices
bindkey -R "^E"-"^F" self-insert
bindkey "^G" list-expand
```

zshrs prints one binding per key, no ranges:
```
bindkey "^@" set-mark-command
bindkey "^A" beginning-of-line
bindkey "^B" backward-char
bindkey "^D" delete-char-or-list
bindkey "^E" end-of-line
```

Two divergences:
1. **Default keymap content**: zsh-bare with `-f` has only a few
   bindings (mostly self-insert + a handful of named widgets).
   zshrs has a full emacs-style keymap installed (Ctrl-A =
   beginning-of-line, etc.).
2. **Output format**: zshrs's `bindkey -L` doesn't collapse
   contiguous same-binding ranges into `-R` entries.

The default keymap difference is the bigger issue — `-f` is
supposed to skip rc files AND most option-state initialization,
giving a "vanilla" shell. zshrs's `-f` (or `--zsh` mode) installs
emacs defaults regardless.

**Where** — `src/ported/zle/keymap.rs::default_emacs_keymap`:
populated at init regardless of `-f` flag. C-source `Src/Zle/init.c::
selectkeymap` only installs the emacs map after rc files run.
zshrs initializes earlier in the boot sequence.

**Impact** — scripts that snapshot `bindkey` output to compare or
restore differ between shells. ZLE-aware tests fail. Plus the
output-format difference means line-count comparisons (`bindkey
-L | wc -l` as a sanity check) diverge.

**Workaround** — when needing portable bindkey output, normalize:
```sh
# expand -R into individual entries
bindkey -L | awk '/^bindkey -R/ { for (i in range) ...; next } { print }'
```
But the right fix is parity with zsh's `-R` collapsed output.

---

## #85 — `"${(s. .)s[@]}"` on scalar with `[@]` subscript returns empty

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 's="a b c"; for x in "${(s. .)s[@]}"; do echo "[$x]"; done'
[a]
[b]
[c]

$ ./target/debug/zshrs --zsh -c 's="a b c"; for x in "${(s. .)s[@]}"; do echo "[$x]"; done'
[]
```

The `"${(s.X.)var[@]}"` form is a documented zsh idiom for
splitting a scalar with explicit per-element capture. zsh splits
into 3 elements and `[@]` enumerates them. zshrs returns a single
empty element.

The equivalent `"${(@s.X.)var}"` form (flag-first, no subscript)
works in both shells:

```sh
$ /opt/homebrew/bin/zsh -fc 's="a b c"; for x in "${(@s. .)s}"; do echo "[$x]"; done'
[a]
[b]
[c]

$ ./target/debug/zshrs --zsh -c 's="a b c"; for x in "${(@s. .)s}"; do echo "[$x]"; done'
[a]
[b]
[c]
```

So the bug is specifically the `[@]` subscript path on a
split-flagged scalar — applying `[@]` after the split should yield
all elements, not collapse to empty.

**Where** — `src/ported/paramsubst.rs::apply_subscript`: when a
split flag has produced an array and `[@]` is then applied, the
code returns the array's length-1 empty placeholder instead of the
actual elements. Related to #63, #82, #83 in the
context-propagation family.

**Impact** — common splitting idioms break:

```sh
for ip in "${(s. .)ips[@]}"; do
    ping -c 1 "$ip"
done
# zsh: pings each IP        zshrs: silent (0 iterations)
```

**Workaround** — `(@s.X.)` flag-first form:
```sh
for ip in "${(@s. .)ips}"; do ping -c 1 "$ip"; done
```

---

## #86 — `${1:?msg}` parameter-required error format has spurious `:1:` line number

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { echo "${1:?required}"; }; f'
f: 1: required

$ ./target/debug/zshrs --zsh -c 'f() { echo "${1:?required}"; }; f'
f:1: 1: required
```

zsh's error format for `${param:?message}` in a function:
`<funcname>: <param>: <message>`. zshrs adds an extra `:1:` line
number: `<funcname>:<lineno>: <param>: <message>`.

The format string is documented in `man zshparam`:
> If `message` is missing, a default message such as `parameter
> null or not set` is printed. Otherwise, `message` is printed,
> preceded by the name of the parameter.

zshrs's format includes the function-relative line number, which
isn't in the upstream format spec.

**Where** — `src/ported/paramsubst.rs::format_required_error`:
includes a `:line:` token. C-source `Src/subst.c::sferror` uses
plain `"%s: %s: %s\n"` format without line.

**Impact** — error-message-parsing scripts that grep for the
specific format fail under zshrs. Diff-based test fixtures fail.
Aesthetic noise in error reports.

**Workaround** — strip with sed/awk if parity matters:
```sh
output=$(f 2>&1 | sed 's/:[0-9]*: \([0-9]*\):/: \1:/')
```

---

## #87 — `setopt` (no args) outputs nothing under `-fc`; zsh shows defaults

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt'
nohashdirs
norcs

$ ./target/debug/zshrs --zsh -c 'setopt'
(empty)
```

zsh's `setopt` (no args) lists options whose state **differs from
the default**. Under `-fc`:
- `nohashdirs` — the `-f` flag disables HASH_DIRS (and lots of
  others), so this differs from the default-on `hash_dirs`.
- `norcs` — the `-f` flag sets NO_RCS (skip rc files), differs
  from the default `rcs`.

zshrs lists nothing, meaning it doesn't track the `-fc`-induced
option flips. Either:
1. `-f` doesn't actually flip those options (so they're at their
   normal default state and don't show up).
2. The default state itself differs from zsh's defaults.
3. `setopt` listing logic is broken (lists only user-set, not
   "differs from default").

Most likely (1) + (3): `-fc` isn't fully wired AND the listing
logic ignores `-f`'s side effects.

**Where** — `src/ported/builtin_setopt.rs::list_options`: should
walk option table comparing each option's current value to its
zsh-spec default and emit non-defaults. `src/ported/init.rs::
apply_f_flag` should flip RCS, HASH_DIRS, USE_ZLE etc. to off.

**Impact** — diagnostic scripts can't tell what option state the
shell is in. `setopt`-snapshot-based config persistence (a common
pattern) returns empty, defeating the purpose. Plus `-f` may not be
honored across other paths, breaking the contract that `-fc` means
"plain shell, no user config".

**Workaround** — query specific options directly via `$options[...]`:
```sh
echo "rcs=${options[rcs]} hashdirs=${options[hashdirs]}"
```

---

## #88 — `setopt nounset` doesn't fire on undefined var in arith `$((x+1))`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt nounset; unset x; echo $((x+1)); echo "after"'
zsh:1: x: parameter not set
(no "after" — aborted)

$ ./target/debug/zshrs --zsh -c 'setopt nounset; unset x; echo $((x+1)); echo "after"'
1
after
```

`setopt nounset` (aka `set -u`) should make any reference to an
unset parameter an error. zsh enforces this in arithmetic
contexts: `$((x+1))` with `x` unset errors out and aborts.

zshrs treats unset arithmetic variables as 0 even under `nounset`,
so `x+1` becomes `0+1=1` and execution continues.

Note: outside arith context, `nounset` does work in zshrs:
```sh
$ ./target/debug/zshrs --zsh -c 'setopt nounset; echo "[$UNDEFINED]"'
zshrs: UNDEFINED: parameter not set
```

So the bug is specifically the arith path.

**Where** — `src/ported/math.rs::lookup_var`: returns 0 for
unset vars without consulting `opts.nounset`. C-source
`Src/math.c::getmathparam` checks the `NOUNSET` option and
emits "parameter not set" via `zerr()`.

**Impact** — defensive code relying on `nounset` to catch
typos/uninitialized counters silently produces wrong values:

```sh
setopt nounset
total=0
for line in "${log_lines[@]}"; do
    # typo: should be $countt = $count
    (( total += countt ))    # zsh: errors on first iter
                              # zshrs: silently uses 0, total stays 0
done
echo "total: $total"          # zshrs reports 0, never warns
```

**Workaround** — guard arith with explicit defined check:
```sh
[[ -v countt ]] || { echo "countt unset"; return 1; }
```

---

## #89 — Extended glob `#` and `##` quantifiers not recognized

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extended_glob; print -l /tmp/zh/a#'
/tmp/zh/a
/tmp/zh/aa
/tmp/zh/aaa

$ ./target/debug/zshrs --zsh -c 'setopt extended_glob; print -l /tmp/zh/a#'
/tmp/zh/a#
```

Files: `a aa aaa`. Pattern `a#` (extended_glob: "0 or more of the
preceding character/group") should match all three. zsh expands
correctly. zshrs treats `#` as literal, returns the pattern
verbatim.

Same for `##` (one-or-more quantifier):
```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extended_glob; print -l /tmp/zh/a##'
/tmp/zh/a
/tmp/zh/aa
/tmp/zh/aaa

$ ./target/debug/zshrs --zsh -c 'setopt extended_glob; print -l /tmp/zh/a##'
/tmp/zh/a##
```

Related to #62 (extended_glob `~` and-not operator) and #81
(extended_glob exclusion duplicates). The whole extended_glob
operator set (`#`, `##`, `~`, `^`) is partially or fully missing.

Per `man zshexpn` § FILENAME GENERATION:
> If the `EXTENDED_GLOB` option is set, the following also have
> special meaning:
> `x#` — match zero or more occurrences of `x`
> `x##` — match one or more occurrences of `x`

**Where** — `src/ported/pattern.rs::compile_extended_glob`: token
table missing `PAT_HASH` (one-or-more) and `PAT_HASH2` (zero-or-
more). C-source `Src/pattern.c::patcompile` adds quantifier
handling when `isset(EXTENDEDGLOB)`.

**Impact** — any extended_glob script using `#`/`##` quantifiers
silently fails. Idioms:

```sh
setopt extended_glob
# match log files with all-numeric basenames
ls /var/log/[0-9]##.log
# zsh: matches 5.log, 42.log, 99999.log
# zshrs: tries to match the literal string "[0-9]##.log"
```

**Workaround** — use `*` plus character class:
```sh
ls /var/log/<0-9>*.log    # zsh-specific numeric range
ls /var/log/[0-9]*.log    # POSIX, both shells
```

---

## #90 — `$ZSH_PATCHLEVEL` set to literal `"unknown"` vs zsh's git-described commit

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[${ZSH_PATCHLEVEL:-unset}]"'
[zsh-5.9-0-g73d3173]

$ ./target/debug/zshrs --zsh -c 'echo "[${ZSH_PATCHLEVEL:-unset}]"'
[unknown]
```

zsh sets `$ZSH_PATCHLEVEL` to a git-describe-style version string
identifying the exact upstream commit (`zsh-5.9-0-g73d3173`).
zshrs hardcodes `"unknown"`.

Related to bug #73 (`$ZSH_VERSION` has `.0.3-test` suffix). Both
are compat-floor parameter-value bugs: zshrs should mirror
upstream values where possible.

For zshrs's own identity, a separate parameter is appropriate
(`$ZSHRS_PATCHLEVEL` or the git hash of the zshrs build). Don't
overload upstream's `$ZSH_PATCHLEVEL`.

**Where** — `src/ported/init.rs::set_compat_params`: hardcodes
the string. Should either:
1. Reflect a specific upstream zsh commit the port targets (e.g.,
   `zsh-5.9-0-g73d3173` to match what zshrs claims compat with).
2. Provide it as a build-time const updated from upstream tags.

**Impact** — scripts that fingerprint zsh by `$ZSH_PATCHLEVEL`
(common in dotfile detection) see `unknown` and can't determine
feature availability:

```sh
# dotfile-pattern: feature gating by patch level
case "$ZSH_PATCHLEVEL" in
    zsh-5.9-*)  HAS_SIXEL=1 ;;
    zsh-5.10*)  HAS_SIXEL=2 ;;
    *)          HAS_SIXEL=0 ;;
esac
# zsh: HAS_SIXEL=1
# zshrs: HAS_SIXEL=0  (always falls to wildcard, features assumed absent)
```

**Workaround** — guard with `$ZSH_VERSION` fallback (which is at
least populated, though wrongly per #73):
```sh
zv=${ZSH_PATCHLEVEL:-${ZSH_VERSION}}
case "$zv" in
    *5.9*)   HAS_SIXEL=1 ;;
    ...
esac
```

---

## #91 — `:t` modifier ignored when applied to `${(j:X:)arr:t}` joined array

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'paths=(/a/b/c.txt /d/e/f.log); echo "${(j: :)paths:t}"'
f.log

$ ./target/debug/zshrs --zsh -c 'paths=(/a/b/c.txt /d/e/f.log); echo "${(j: :)paths:t}"'
/a/b/c.txt /d/e/f.log
```

The `${(j: :)paths:t}` form should join the array with space, then
apply the `:t` modifier (tail/basename) to the resulting scalar.
zsh applies `:t` after the join, yielding `f.log` (basename of
the joined string).

zshrs joins but ignores the `:t` modifier entirely.

Other modifier+expansion combinations work in both shells:
```sh
${paths[@]:t}    # both: c.txt f.log
${paths:t}       # both: f.log
${(j: :)paths}   # both: /a/b/c.txt /d/e/f.log
```

So the bug is specifically the `(j:X:)` flag + trailing modifier
combo — flag consumes the parse context and modifier never fires.

**Where** — `src/ported/paramsubst.rs::apply_modifiers_after_flags`:
when an expansion flag like `(j:X:)` is present, the trailing
`:modifier` token isn't dispatched to the modifier-handler. C-source
`Src/subst.c::modify` runs modifier dispatch regardless of preceding
flag.

**Impact** — path-manipulation idioms break:

```sh
# build a colon-separated list of just basenames
PATHs=(/usr/local/bin /usr/bin /opt/homebrew/bin)
basenames="${(j.:.)PATHs:t}"
# zsh: bin   (joined-then-tailed)
# zshrs: /usr/local/bin:/usr/bin:/opt/homebrew/bin (modifier dropped)
```

**Workaround** — split the two operations:
```sh
tailed=("${PATHs[@]:t}")
joined="${(j.:.)tailed}"
```

---

## #92 — `$PS4` default is empty in zshrs; zsh defaults to `\e[34m%x\t%0N\t%I\t%_\e[0m\t`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo -n "$PS4"' | od -c | head -2
0000000  033   [   3   4   m   %   x  \t   %   0   N  \t   %   I  \t   %
0000020    _ 033   [   0   m  \t

$ ./target/debug/zshrs --zsh -c 'echo -n "$PS4"' | od -c | head -2
0000000
```

zsh-bare ships a default `$PS4` of `%F{blue}%x\t%0N\t%I\t%_%f\t`
(blue color + filename + funcname + line + cmd + tab) for `set -x`
output.

zshrs has `$PS4` initialized to an empty string. So `set -x` output
has no prefix at all (related to bug #44 where the prompt escapes
don't expand either).

The fact that PS4 is **empty by default** is even worse than #44
suggested — even if the escapes worked, there'd be nothing to
expand.

**Where** — `src/ported/init.rs::set_default_ps_params`: doesn't
initialize PS4. C-source `Src/init.c::setupvals` calls
`createparam("PS4", "\033[34m%x\t%0N\t%I\t%_\033[0m\t", ...)`.

**Impact** — `set -x` debugging output is unformatted, just bare
command lines. Tracing identical-named commands across files
impossible (no `%x` filename, no `%I` line):

```sh
$ /opt/homebrew/bin/zsh -fxc 'f() { echo hi; }; f'
+zsh:1<2>	f	1	echo hi
hi

$ ./target/debug/zshrs --zsh -xc 'f() { echo hi; }; f'
echo hi
hi
```

(zshrs misses the file/line/funcname annotations.)

**Workaround** — set explicit PS4:
```sh
export PS4='+%N:%i> '   # filename:line in plain ASCII
set -x
```

---

## #93 — Empty-string assoc key: `typeset -A h=( "" val )` swaps key/value; `h[""]` lookup fails

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

Two sub-bugs in zshrs's empty-assoc-key handling:

**Sub-bug A**: `typeset -A h=( "" "value" )` swaps key and value:
```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h=( "" "empty-val" ); typeset -p h'
typeset -A h=( ['']=empty-val )

$ ./target/debug/zshrs --zsh -c 'typeset -A h=( "" "empty-val" ); typeset -p h'
typeset -A h=( [empty-val]='' )
```

zsh stores key `""` (empty) with value `empty-val`. zshrs treats
the empty arg as missing (key skipped) and the next arg becomes
the key with the FOLLOWING arg (or default empty) as the value.
Effectively the empty key shifts the key/value alignment.

**Sub-bug B**: subscript-form assignment `h[""]=` stored but
`h[""]` lookup returns empty:
```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h[""]="empty-key"; echo "[${h[\"\"]}]"; typeset -p h'
[empty-key]
typeset -A h=( ['""']=empty-key )

$ ./target/debug/zshrs --zsh -c 'typeset -A h; h[""]="empty-key"; echo "[${h[\"\"]}]"; typeset -p h'
[]
typeset -A h=( []=empty-key )
```

In zsh, `h[""]` stores key `""` (literally two chars: quote + quote,
per the quote-embedding behavior from #61). zshrs strips quotes to
empty-key, then lookup `${h[\"\"]}` also strips to empty and
should match — but returns empty anyway.

So zshrs's empty-key path is broken in both directions:
- Paren-init: misaligns key/value pairs around the empty string.
- Subscript-init: stores but can't retrieve.

**Where** — `src/ported/params.rs::set_hash_value`: empty key
treated as sentinel for "no key", advancing to next arg. C-source
`Src/params.c::sethparam` accepts empty as a valid key.

**Impact** — sparse hash patterns using empty as a sentinel
(common for "default" or "unmarked" entries) silently break.
Plus the misalignment on paren-init corrupts the entire hash for
any data including empty strings.

**Workaround** — never use empty string as a hash key. Reserve a
sentinel like `__EMPTY__` or `\0`.

---

## #94 — `(exec cmd); cmd2` — parent shell terminates with subshell

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo o1; ( exec true ); echo o2'
o1
o2

$ ./target/debug/zshrs --zsh -c 'echo o1; ( exec true ); echo o2'
o1
```

`exec cmd` inside a subshell `(...)` should only replace the
**subshell process**, not the parent. After the subshell exits, the
parent should continue. zsh prints `o1\no2`. zshrs prints only `o1`
— the parent shell exits when the subshell does.

More elaborate test:
```sh
$ /opt/homebrew/bin/zsh -fc 'echo "outer-1"; (echo "sub-1"; exec echo "sub-replaced"; echo "sub-not-reached"); echo "outer-2"'
outer-1
sub-1
sub-replaced
outer-2

$ ./target/debug/zshrs --zsh -c 'echo "outer-1"; (echo "sub-1"; exec echo "sub-replaced"; echo "sub-not-reached"); echo "outer-2"'
outer-1
sub-1
sub-replaced
(no outer-2)
```

The subshell correctly skips `sub-not-reached` (exec'd away), but
the parent's `outer-2` also doesn't print.

**Where** — `src/ported/exec.rs::exec_builtin`: doesn't check
whether the current frame is a subshell vs the parent. `exec`
should replace the current process image — for a subshell, that's
the forked child; for the parent shell, that's the parent itself.
zshrs treats them the same way and terminates both. C-source
`Src/exec.c::execcmd_exec` is gated on `forked` flag.

**Impact** — pattern of "fork off a one-shot child via subshell+
exec" silently terminates the parent shell:

```sh
# replace a subshell stdout with another command's, then continue
echo "starting"
(exec curl https://api.example.com/data)
echo "continuing"    # zsh prints; zshrs doesn't
process_data
```

This breaks daemon/server scripts that exec sub-tasks in subshells
expecting to continue.

**Workaround** — don't use `exec` inside subshell unless you mean
to terminate. Use plain command call:
```sh
(curl https://api.example.com/data)   # both shells: continue
```

---

## #95 — Signal trap from subshell-internal `kill $$` fires immediately, not after subshell

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'trap "echo [TRAP-FIRED]" USR1; (echo "sub-start"; kill -USR1 $$; echo "sub-mid"; sleep 0.05; echo "sub-end"); echo "main-after"'
sub-start
sub-mid
sub-end
zsh:1: no matches found: [TRAP-FIRED]
main-after

$ ./target/debug/zshrs --zsh -c 'trap "echo [TRAP-FIRED]" USR1; (echo "sub-start"; kill -USR1 $$; echo "sub-mid"; sleep 0.05; echo "sub-end"); echo "main-after"'
sub-start
zsh:1: no matches found: [TRAP-FIRED]
sub-mid
sub-end
main-after
```

(The `no matches found` error is from `[TRAP-FIRED]` being
unfortunately glob-expanded under -fc default nomatch. Ignore it —
look at the ordering of `sub-start`/`sub-mid`/`sub-end` and
`[TRAP-FIRED]` line.)

zsh:
1. Subshell starts, prints `sub-start`.
2. `kill -USR1 $$` sends USR1 to parent (the shell process).
3. Parent is currently **waiting on subshell** — the signal is
   queued.
4. Subshell continues to completion: `sub-mid`, `sleep`, `sub-end`.
5. Subshell exits, parent wakes up.
6. Pending USR1 trap fires → `[TRAP-FIRED]`.
7. Parent continues to `main-after`.

zshrs:
1. Subshell starts, prints `sub-start`.
2. `kill -USR1 $$` sends USR1 to parent.
3. Parent's signal handler **fires immediately**, prints
   `[TRAP-FIRED]` (interleaved with subshell's stdout, before its
   `sub-mid` line).
4. Subshell continues: `sub-mid`, `sleep`, `sub-end`.
5. Parent's `main-after`.

The trap fires asynchronously in zshrs, whereas zsh defers signal
processing until any synchronous command (including the wait on
subshell) completes.

**Where** — `src/ported/signal.rs::handle_signal_async`: signal
handler runs immediately on receipt. C-source `Src/signals.c::
zhandler` queues the signal and runs the trap at next instruction
boundary (effectively at end of current synchronous command).

**Impact** — race conditions in signal-driven cleanup. Patterns
where subshell sends signal to parent for "notify me when this is
done" semantics break — zsh's deferred handling gives clean
ordering; zshrs's immediate handling produces interleaved output
and possibly race-corrupted state if the trap mutates shared
variables the subshell is still writing.

**Workaround** — none from user side. Signal-driven IPC across
subshell boundary not safe in zshrs.

---

## #96 — `%N/` prompt escape doesn't truncate path to last N components

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'cd /Users/wizard/RustroverProjects/zshrs; print -P "[%1/]"; print -P "[%2/]"; print -P "[%3/]"'
[zshrs]
[RustroverProjects/zshrs]
[wizard/RustroverProjects/zshrs]

$ ./target/debug/zshrs --zsh -c 'cd /Users/wizard/RustroverProjects/zshrs; print -P "[%1/]"; print -P "[%2/]"; print -P "[%3/]"'
[/Users/wizard/RustroverProjects/zshrs]
[/Users/wizard/RustroverProjects/zshrs]
[/Users/wizard/RustroverProjects/zshrs]
```

The `%N/` prompt escape limits path display to the last N path
components. zsh:
- `%1/` → last 1 component (`zshrs`)
- `%2/` → last 2 (`RustroverProjects/zshrs`)
- `%3/` → last 3 (`wizard/RustroverProjects/zshrs`)

zshrs ignores the numeric prefix entirely and prints the full
`$PWD` for all variants. Same family as #38 (prompt escapes
missing) but specifically the numeric-modifier-on-`/` and `~`
escapes.

Per `man zshmisc` § PROMPT EXPANSION:
> `%/` — Current working directory.
> `%~` — Current working directory with `$HOME` shortened to `~`.
> An integer may follow the `%` to specify how many trailing path
> components to keep.

**Where** — `src/ported/prompt.rs::handle_path_escape`: numeric
prefix to `/` and `~` not parsed. C-source `Src/prompt.c::
promptpath` walks the path backwards counting separators.

**Impact** — every `.zshrc` PROMPT setting using `%N/` or `%N~`
for compact path display falls back to full-path display. Vintage
prompt themes (`oh-my-zsh`'s `robbyrussell`, `agnoster`, p10k
default modes) all rely on `%~` with the modifier — wrong in
zshrs.

**Workaround** — manual truncation in prompt:
```sh
PROMPT='%# '
precmd() {
    local short=${PWD/#$HOME/\~}
    local depth=2
    psvar=("${(s:/:)short}")
    # walk last $depth components manually
    ...
}
```

---

## #97 — `typeset -r` listing doesn't include shell-internal readonly params

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -r' | grep -E '^!=|^#='
!=0
'#'=0

$ ./target/debug/zshrs --zsh -c 'typeset -r' | grep -E '^!=|^#='
(empty)
```

`typeset -r` (no args) lists every readonly parameter. zsh
includes shell-internal readonly params like `$!` (last
background PID, `!=0` before any bg job), `$#` (positional count,
`'#'=0`), `$$` (PID), etc.

zshrs's listing omits these internal-readonly params entirely. The
flag is set internally (you can't assign `$$=42`) but the listing
output doesn't reflect them.

Full comparison:
```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -r' | head -5
!=0
'#'=0

$ ./target/debug/zshrs --zsh -c 'typeset -r' | head -5
(empty until user sets readonly explicitly)
```

**Where** — `src/ported/builtin_typeset.rs::list_readonly`: walks
user-set readonly params only, doesn't iterate the shell-internal
special-param table. C-source `Src/builtin.c::bin_typeset` walks
the unified param table with `PM_READONLY` bit check.

**Impact** — diagnostic scripts that audit shell readonly state
miss the internal readonly set. Scripts that snapshot/restore
shell state via `typeset -p` get inconsistent output between
zsh and zshrs.

Also: there's no easy way for a user to discover "what's readonly
in this shell?" since the listing path is incomplete.

**Workaround** — accept that `$!`, `$#`, `$$`, `$?` etc. are
implicitly readonly per zsh spec; don't rely on `typeset -r`
listing them.

---

## #98 — `[ "a" \< "b" ]` lexicographic comparison accepted (bash extension)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '[ a \< b ]; echo "exit=$?"'
zsh:1: condition expected: <
exit=2

$ ./target/debug/zshrs --zsh -c '[ a \< b ]; echo "exit=$?"'
exit=0
```

The `\<` / `\>` operators in single-bracket `[ ]` for lexical
string comparison are a **bash extension**, not POSIX or zsh
standard. zsh refuses with "condition expected: <" syntax error.
zshrs accepts and evaluates correctly (returns 0 for `a < b`,
1 for `b < a`).

```sh
$ ./target/debug/zshrs --zsh -c '[ a \< b ]; echo "exit=$?"; [ b \< a ]; echo "exit=$?"'
exit=0
exit=1
```

In zsh you must use `[[ ... ]]` (double-bracket) for lex compare
with `<`/`>`, no backslash needed:
```sh
[[ "a" < "b" ]] && echo "yes"   # zsh: works
```

**Where** — `src/ported/builtin_test.rs::parse_condition`: the
single-bracket `[` test parser includes bash's `\<` and `\>`
operators in the recognized-operators table. C-source
`Src/test.c::test_expr` errors on `<` / `>` outside `[[ ]]`.

**Impact** — bash-compat scripts using `[ \< ]` work in zshrs but
break in zsh. False sense of cross-shell portability — scripts
developed under zshrs (because "zshrs accepted it") then fail in
production zsh.

Worse: a `[ a \< b ]` that should be a hard syntax error in zsh
becomes a silent runtime no-op (returns true even when meant
otherwise) in zshrs if the author got the semantics wrong.

**Workaround** — always use double-bracket for lex compare in
zsh-portable code:
```sh
[[ "$a" < "$b" ]] && echo "less"
```

---

## #99 — Extended-glob `(#cN,M)` count quantifier not recognized

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extended_glob; touch /tmp/zg/a /tmp/zg/aa /tmp/zg/aaa; print -l /tmp/zg/a(#c1,2)'
/tmp/zg/a
/tmp/zg/aa

$ ./target/debug/zshrs --zsh -c 'setopt extended_glob; touch /tmp/zg/a /tmp/zg/aa /tmp/zg/aaa; print -l /tmp/zg/a(#c1,2)'
(empty)
```

The `(#cN,M)` extended_glob quantifier matches the preceding
character/group N-to-M times. zsh matches `a` and `aa` (1 to 2 a's).
zshrs returns nothing — the count syntax isn't recognized.

Same family as bug #89 (extended_glob `#`/`##` quantifiers
missing) — the entire `(#...)` flag family is partially or fully
unimplemented:
- `(#c)` — count
- `(#a)` — approximate match
- `(#i)` — case-insensitive
- `(#l)` — case-loose
- `(#s)` — anchor at start
- `(#e)` — anchor at end
- `(#m)` — record match

Per `man zshexpn` § FILENAME GENERATION:
> `(#cN,M)` — Matches the preceding character or group between N
> and M times.

**Where** — `src/ported/pattern.rs::compile_extended_glob`: the
`(#flag)` glob-flag parser table is missing entries for `c`, `a`,
`i`, `l`, `s`, `e`, `m`. C-source `Src/pattern.c::patcompile`
handles each `(#X...)` flag specifically.

**Impact** — every script using `(#c)` count or other
zsh-specific extended_glob flags silently fails to match:

```sh
setopt extended_glob
# match passwords with 8-16 chars
[[ "$pw" == [a-zA-Z0-9]##(#c8,16) ]] && echo "valid"
# zsh: validates correctly
# zshrs: always fails (no match)
```

**Workaround** — `[[ ... =~ ... ]]` regex with explicit character
class and `{N,M}` quantifier:
```sh
[[ "$pw" =~ "^[a-zA-Z0-9]{8,16}$" ]] && echo "valid"
```

---

## #100 — `typeset -R N x="hello"` doesn't right-truncate the value

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -L 3 x="hello"; echo "L: [$x]"; typeset -R 3 y="hello"; echo "R: [$y]"'
L: [hel]
R: [llo]

$ ./target/debug/zshrs --zsh -c 'typeset -L 3 x="hello"; echo "L: [$x]"; typeset -R 3 y="hello"; echo "R: [$y]"'
L: [hel]
R: [hello]
```

`typeset -L N` (left-justify, truncate at N) — both shells return
`hel` (correct).

`typeset -R N` (right-justify, truncate at N) — zsh returns `llo`
(the rightmost 3 chars). zshrs returns the unmodified full
`hello` — the `-R N` attribute is recorded but not enforced on
assignment.

Per `man zshbuiltins`:
> `-L NUM` — Left-justify the value in a field of width NUM. If
> NUM is non-zero, longer strings are truncated.
> `-R NUM` — Right-justify and truncate similarly.

**Where** — `src/ported/builtin_typeset.rs::apply_attrs`: the
`-R N` truncation code path is missing or no-op. `-L N` works,
suggesting only the right-justify branch is unimplemented.
C-source `Src/params.c::sethparam` calls `padstring(..., right)`
based on the flag.

**Impact** — fixed-width columnar output formatting via
`typeset -R N` is broken. Any code formatting tabular data:

```sh
typeset -R 5 num=42
typeset -L 8 name="alice"
echo "$num | $name"     # zsh:    "   42 | alice   "
                         # zshrs:  "42 | alice   "   (num not padded/truncated)
```

**Workaround** — explicit printf width specifier:
```sh
printf "%5s | %-8s\n" "$num" "$name"   # both shells: same output
```

---

## #101 — `exec funcname` (shell function) errors "not found" instead of running

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { echo "in fn"; }; exec f'
in fn

$ ./target/debug/zshrs --zsh -c 'f() { echo "in fn"; }; exec f'
zshrs: exec: f: not found
```

`exec cmd` should replace the shell with `cmd`. When `cmd` is a
shell-defined function, zsh runs the function in the current
process (no fork, no PATH lookup) — effectively the last act of
the shell, after which it exits.

zshrs's `exec` does PATH-only lookup, errors when `f` isn't an
external binary. Loses the "exec a shell function as last act"
semantic.

Subshell variant (`(exec fn)`) is the same:
```sh
$ /opt/homebrew/bin/zsh -fc 'f() { echo "in fn"; }; (exec f); echo "after"'
in fn
after

$ ./target/debug/zshrs --zsh -c 'f() { echo "in fn"; }; (exec f); echo "after"' 2>&1
zshrs: exec: f: not found
after
```

The subshell runs at all in zshrs, but immediately errors on
exec. (The `after` line still prints because the subshell exits
on error, and `#94`'s parent-terminates-with-subshell only fires
on clean exec success.)

**Where** — `src/ported/builtin_exec.rs::resolve_target`: looks
up `target` in PATH only. Should check the function table first
(`shell.functions.get(target)`) and call the function in the
current process if found. C-source `Src/exec.c::execcmd` falls
through to `Builtin/External/Function` dispatch even from `exec`
context.

**Impact** — `exec` chaining of shell functions is broken. Common
pattern: define a wrapper function, then `exec` it at end-of-init
to swap into it:

```sh
init_setup() { ...; }
init_setup    # do setup
exec main    # become main loop forever
# zsh: process becomes main()
# zshrs: errors out
```

**Workaround** — drop `exec` and just call the function:
```sh
main    # last line of script
```
Or use a real external command if the goal is genuine process
replacement.

---

## #102 — `$-` (current option flags) doesn't include `f` from `-f` startup flag

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[$-]"'
[569Xf]

$ ./target/debug/zshrs --zsh -c 'echo "[$-]"'
[569X]
```

`$-` is the special parameter listing currently-set shell options
as single-letter codes. When zsh is invoked with `-f` (skip rc
files), the `f` flag appears in `$-`. zshrs's `$-` omits it.

Per `man zsh` § STARTUP:
> `-f` — Equivalent to `--no-rcs`. Suppresses sourcing of all
> startup files.

And per `man zshparam`:
> `$-` — Flags supplied to the shell on invocation or by the set
> builtin.

Runtime-toggled options DO show up:
```sh
$ ./target/debug/zshrs --zsh -c 'set -x; echo "[$-]"; set +x' 2>&1 | grep '\['
[569Xx]
```

`x` is properly added when `set -x` runs. So the bug is
specifically: startup-time `-f` doesn't propagate into `$-`.

**Where** — `src/ported/init.rs::apply_startup_flags`: parses
`-f` and sets internal NO_RCS state but doesn't add `f` to the
`$-` letter set. `src/ported/params.rs::compute_dash_param` walks
the option table looking for letters; the `-f` startup-only flag
might not be in the option table at all (it's a CLI-only
shortcut for `--no-rcs`).

**Impact** — defensive code checking `$-` for `f` to detect
"plain shell" mode doesn't work:

```sh
case "$-" in
    *f*) echo "shell started with -f (no rc files)" ;;
    *)   echo "rc files were sourced" ;;
esac
# zsh: prints "started with -f" under -fc invocation
# zshrs: always prints "rc files were sourced" (false)
```

Related to bug #87 (`setopt` no-args listing missing `nohashdirs`/
`norcs` under `-fc`) — both stem from `-f` not fully wired
through.

**Workaround** — none portable; rely on `[[ -o no_rcs ]]` direct
option test instead:
```sh
[[ -o no_rcs ]] && echo "no rc files"
```

---

## #103 — `$0` inside sourced script returns shell binary path, not sourced file

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ echo 'echo "0=$0"' > /tmp/zsrc.zsh
$ /opt/homebrew/bin/zsh -fc 'source /tmp/zsrc.zsh'
0=/tmp/zsrc.zsh

$ ./target/debug/zshrs --zsh -c 'source /tmp/zsrc.zsh'
0=./target/debug/zshrs
```

Inside a sourced script, `$0` should reflect the path of the
currently-sourced file (per zsh behavior, controlled by
`POSIX_ARGZERO`). zsh returns `/tmp/zsrc.zsh`. zshrs returns the
shell binary `./target/debug/zshrs`.

Per `man zshparam`:
> `$0` — Inside a function, $0 is the name of the function. When
> a shell script is sourced via the `source` or `.` builtins, $0
> is normally the name of the script.

**Where** — `src/ported/builtin_source.rs::source_file`: doesn't
push/pop the `$0` parameter to the script's name during sourcing.
C-source `Src/builtin.c::bin_dot` saves the prior `$0`, sets to
script path, executes, restores on return.

**Impact** — common idiom in sourced library scripts:

```sh
# /lib/utils.sh
SELF_DIR=${0:A:h}
SELF_NAME=${0:t}
# zsh: SELF_DIR=/lib, SELF_NAME=utils.sh
# zshrs: SELF_DIR=., SELF_NAME=zshrs (wrong)
```

Breaks every sourced library that uses `$0` to find its own
companion files (config, themes, sub-modules).

**Workaround** — use `${(%):-%x}` (prompt expansion `%x` = current
file) to get the script's own path:
```sh
SELF=${(%):-%x}    # both shells: path to the script being sourced
```

---

## #104 — Signal sent via `kill -X $$` from inside function is lost (trap never fires)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'trap "echo TRAP" USR1; f() { kill -USR1 $$; }; f; echo "post-fn"'
TRAP
post-fn

$ ./target/debug/zshrs --zsh -c 'trap "echo TRAP" USR1; f() { kill -USR1 $$; }; f; echo "post-fn"'
post-fn
```

Direct `kill -USR1 $$` at top-level works in both shells (trap
fires). But when wrapped in a function, the signal is delivered
but zshrs's trap **never fires** — even after the function
returns and execution continues.

```sh
# Direct (both shells work):
$ /opt/homebrew/bin/zsh -fc 'trap "echo TRAP" USR1; kill -USR1 $$; echo post'
TRAP
post

$ ./target/debug/zshrs --zsh -c 'trap "echo TRAP" USR1; kill -USR1 $$; echo post'
TRAP
post
```

So the bug is specifically the function-context path: signal
delivered while executing a function gets lost in zshrs's signal
handling.

Related to bug #95 (signal from inside subshell fires immediately
instead of being deferred). Both indicate zshrs's signal-handling
state machine doesn't track function vs subshell vs top-level
context correctly.

**Where** — `src/ported/signal.rs::deliver_signal`: when receiving
a signal while in function execution, the signal-pending flag is
set but never checked at function-return boundary. C-source
`Src/exec.c::execfuncdef` checks `errflag & ERRFLAG_SIGNAL` at
each statement, runs trap if pending.

**Impact** — signal-driven event handling broken whenever the
signal source is inside a function call. Common pattern:

```sh
trap "save_state" USR1
update_state() {
    refresh_data
    kill -USR1 $$    # tell ourselves to save after refresh
}
update_state
# zsh: save_state runs after update_state returns
# zshrs: save_state NEVER runs (signal lost)
```

Long-running daemons using self-signaling for "checkpoint after
this section completes" silently never checkpoint.

**Workaround** — explicit invocation after function call:
```sh
update_state
save_state    # call directly instead of relying on signal
```

---

## #105 — `(f<NNN>)` file-permission glob qualifier ignored (returns all files)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ mkdir -p /tmp/zgp; touch /tmp/zgp/{a,b,c}; chmod 644 /tmp/zgp/a; chmod 755 /tmp/zgp/b; chmod 600 /tmp/zgp/c

$ /opt/homebrew/bin/zsh -fc 'print -l /tmp/zgp/*(f644)'
/tmp/zgp/a

$ ./target/debug/zshrs --zsh -c 'print -l /tmp/zgp/*(f644)'
/tmp/zgp/a
/tmp/zgp/b
/tmp/zgp/c
```

The `(f<perms>)` glob qualifier filters files by exact permission
match. zsh returns only the file with 644 perms (`a`). zshrs
ignores the `f644` qualifier and returns all matching files.

Per `man zshexpn` § Glob Qualifiers:
> `f spec` — files with access rights matching `spec`. `spec` is
> a numeric mode or a `chmod`-style mode spec.

**Where** — `src/ported/glob.rs::apply_qualifier`: the `f` (and
related `r`/`w`/`x` and `u`/`g`/`o` user/group/other perm
qualifiers) appear to be unimplemented. C-source
`Src/glob.c::qualflags::QC_MODE` filters by `st_mode & MASK`.

**Impact** — permission-based file selection silently returns
everything:

```sh
# find all world-readable config files
print -l /etc/*(.f+004)
# zsh: just files with world-read bit set
# zshrs: everything in /etc

# find executable files
print -l /usr/local/bin/*(*)
# zsh: only executables
# zshrs: returns all (unverified, but same family — see #105's u/g/x cousins)
```

Related to #41 (`Yn` limit qualifier ignored), #62 (~ exclusion),
#89/#99 (extended_glob quantifiers) — pattern of glob-qualifier
implementations being thin or absent.

**Workaround** — pipe through `test` or `stat`:
```sh
for f in /tmp/zgp/*; do
    [[ "$(stat -f '%p' "$f" 2>/dev/null)" == *644 ]] && echo "$f"
done
```

---

## #106 — `disable BUILTIN` doesn't actually disable the builtin

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'disable echo; type echo; echo hi'
echo is /bin/echo
hi

$ ./target/debug/zshrs --zsh -c 'disable echo; type echo; echo hi'
echo is a shell builtin
hi
```

After `disable echo`, zsh removes the builtin from the dispatch
table — `type echo` reports the external path `/bin/echo`, and
`echo hi` invokes the external. zshrs's `disable` doesn't actually
disable the builtin — `type echo` still reports "shell builtin"
and execution still uses the builtin.

Same for `cd`:
```sh
$ /opt/homebrew/bin/zsh -fc 'disable cd; cd /tmp 2>&1; pwd'
/Users/wizard/RustroverProjects/zshrs   # cd failed silently, no movement

$ ./target/debug/zshrs --zsh -c 'disable cd; cd /tmp 2>&1; pwd'
/tmp   # cd still works
```

`disable cd` should make `cd` undefined (since there's no
external `cd`), so subsequent `cd /tmp` should fail with
command-not-found. zshrs ignores the disable.

Per `man zshbuiltins`:
> `disable [ -afmrs ] name ...` — disables the hash table elements
> with the given names. By default, names are disabled as
> builtins.

**Where** — `src/ported/builtin_disable.rs::disable_builtin`:
sets a flag in the builtin table but the dispatcher doesn't
consult it. C-source `Src/builtin.c::execbuiltin` checks
`PM_DISABLED` flag on lookup and falls through to PATH if set.

**Impact** — `disable` is the standard mechanism to:
1. Force PATH lookup for a name that's shadowed by a builtin
   (e.g., `disable echo` to use GNU echo from coreutils).
2. Test code paths that should error when a builtin is absent.
3. Plugin systems that temporarily disable a builtin to provide
   their own wrapper.

All three patterns broken in zshrs.

**Workaround** — `command` prefix forces external lookup:
```sh
command echo hi    # bypasses builtin, calls /bin/echo
```

---

## #107 — `autoload -U +X funcname` doesn't validate function exists in fpath

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'autoload -U +X totally_fake_function_name; echo "ec=$?"'
zsh:1: totally_fake_function_name: function definition file not found
ec=1

$ ./target/debug/zshrs --zsh -c 'autoload -U +X totally_fake_function_name; echo "ec=$?"'
ec=0
```

`autoload -U +X funcname` should:
1. Search `$fpath` for a file matching `funcname`.
2. Load it immediately (the `+X` flag).
3. Error if not found.

zsh errors with "function definition file not found" + exit 1.
zshrs silently succeeds with exit 0.

Per `man zshbuiltins`:
> `autoload ... +X` — Load the function immediately. If the
> function definition is not found, the function is not defined
> and `autoload` returns an error.

Without `+X`, both shells register the function lazily without
immediate validation (which IS correct — error only fires on
first call). Bug is specifically the `+X` immediate-load path
skipping the existence check.

**Where** — `src/ported/builtin_autoload.rs::immediate_load`:
calls the function-table registration regardless of file
existence. C-source `Src/builtin.c::bin_autoload` calls
`loadautofn(fmark, +1)` which fails when no fpath match exists.

**Impact** — config validation scripts that use `autoload +X` to
verify their function dependencies are available at startup get
false positives:

```sh
# .zshrc snippet checking required functions
for fn in compinit promptinit add-zsh-hook; do
    autoload -U +X $fn 2>/dev/null || {
        echo "WARN: $fn not in fpath; check zsh install"
    }
done
# zsh: warns on missing fns      zshrs: never warns (all pass)
```

**Workaround** — explicit `[[ -f $fpath/funcname ]]` check before
autoload:
```sh
local found=0
for dir in "$fpath[@]"; do
    [[ -f "$dir/$fn" ]] && { found=1; break; }
done
(( found )) || echo "WARN: $fn not found"
```

---

## #108 — `${array/pat/repl}` treats as per-element instead of scalar-joined

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(red blue green); echo "[${a/b*/X}]"'
[red X]

$ ./target/debug/zshrs --zsh -c 'a=(red blue green); echo "[${a/b*/X}]"'
[red X green]
```

`${array/pat/X}` (no `[@]` subscript) — in zsh, the array is
joined to a scalar first (`red blue green`), then `b*` matches
"blue green" (greedy) → `[red X]`.

zshrs applies the substitution per-element (treats it the same as
`${array[@]/pat/X}`): each element substituted independently.
"red" doesn't match. "blue" matches → "X". "green" doesn't match
"b*". Result: `[red X green]`.

For comparison, `${a[@]/b*/X}` IS per-element in both shells:
```sh
$ /opt/homebrew/bin/zsh -fc 'a=(red blue green); echo "[${a[@]/b*/X}]"'
[red X green]
```

So the bug is specifically: zshrs treats `${a/...}` (without
`[@]`) as if it had `[@]` implicit. zsh requires `[@]` for
per-element semantics; without it, the array is collapsed to a
scalar first.

**Where** — `src/ported/paramsubst.rs::apply_pattern_substitution`:
forks into per-element mode based on the underlying parameter
being an array, regardless of subscript form. C-source
`Src/subst.c::getmatch` distinguishes `${arr/...}` (scalar context)
from `${arr[@]/...}` (array context).

**Impact** — same family as #82 (quoted-context array vs scalar)
and #63 (nested split-then-join). Pattern-substitution-based
transforms that intentionally use the scalar form produce wrong
results:

```sh
csv=(red blue green)
# replace the FIRST blue/green pair occurrence (whichever comes first as a substring)
echo "${csv/blue green/COMBINED}"
# zsh: "red COMBINED"            (matches across join)
# zshrs: "red blue green"        (no per-element match)
```

**Workaround** — explicit scalar join + substitute:
```sh
joined="${csv[*]}"
echo "${joined/blue green/COMBINED}"
```

---

## #109 — `${assoc[@]}` returns empty; cannot enumerate associative array values

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h=(a 1 b 2 c 3); echo "${h[@]}"; for v in ${h[@]}; do echo "$v"; done'
1 2 3
1
2
3

$ ./target/debug/zshrs --zsh -c 'typeset -A h=(a 1 b 2 c 3); echo "${h[@]}"; for v in ${h[@]}; do echo "$v"; done'
(empty for ${h[@]})
(no iteration)
```

For associative arrays, `${h[@]}` should enumerate VALUES (just
like the indexed-array `[@]` enumerates elements). zsh produces
the 3 values; zshrs returns empty.

The `${(v)h[@]}` explicit-value form DOES work in both:
```sh
$ ./target/debug/zshrs --zsh -c 'typeset -A h=(a 1 b 2); echo "${(v)h[@]}"'
1 2
```

So zshrs only enumerates assoc values when the `(v)` flag is
explicitly given. zsh treats `[@]` on an assoc as implicit-`(v)`.

**Where** — `src/ported/paramsubst.rs::expand_subscript_at`:
the `[@]` subscript handler for `PM_HASHED` params returns empty
instead of iterating values. C-source `Src/subst.c::getmatch`
calls `gethkparam`/`gethvparam` based on flag, defaulting to
value enumeration when no flag is given.

**Impact** — every assoc-array iteration idiom is broken:

```sh
typeset -A scores=(alice 95 bob 87)
total=0
for score in "${scores[@]}"; do
    (( total += score ))
done
echo "total=$total"
# zsh: total=182          zshrs: total=0  (loop body never executed)
```

`${(@k)h}` (explicit keys) and `${(@v)h}` (explicit values) work
correctly — the bug is only `${h[@]}`'s default-value semantic.

**Workaround** — always use `${(v)h[@]}` explicitly for values
or `${(k)h[@]}` for keys:
```sh
for score in "${(v)scores[@]}"; do ... done
```

---

## #110 — `a[0]=val` silently accepted instead of erroring (zsh is 1-indexed)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(); a[0]=val; echo "[${a[*]}] len=${#a}"' 2>&1
zsh:1: a: assignment to invalid subscript range

$ ./target/debug/zshrs --zsh -c 'a=(); a[0]=val; echo "[${a[*]}] len=${#a}"'
[val] len=1
```

zsh arrays are **1-indexed**. `a[0]` is invalid — zsh aborts with
"assignment to invalid subscript range". zshrs silently accepts
the assignment, storing `val` at index 0 (effectively a bash-
style 0-indexed array).

Verification of 1-index semantics elsewhere:
```sh
$ /opt/homebrew/bin/zsh -fc 'a=(red blue green); echo "[${a[0]}] [${a[1]}]"'
[] [red]                   # a[0] empty, a[1] first

$ ./target/debug/zshrs --zsh -c 'a=(red blue green); echo "[${a[0]}] [${a[1]}]"'
[] [red]                   # same — read-side honors 1-indexing
```

So zshrs's READ side is 1-indexed (correct). But WRITE side
accepts `a[0]=...` silently. Asymmetric.

The `KSH_ARRAYS` option makes both shells 0-indexed:
```sh
$ /opt/homebrew/bin/zsh -fc 'setopt KSH_ARRAYS; a=(red blue green); echo "[${a[0]}] [${a[1]}]"'
[red] [blue]
```

But the bug here is in the default 1-indexed mode.

**Where** — `src/ported/params.rs::set_array_element`: write path
doesn't validate `index >= 1` under default mode. C-source
`Src/params.c::setiparam` calls `subscript_check` which errors on
0.

**Impact** — code copied from bash (0-indexed) works in zshrs but
fails under real zsh. Cross-shell scripts silently behave
differently.

**Workaround** — always use 1-indexed arrays in zsh-portable
code, or explicitly `setopt KSH_ARRAYS` if 0-indexed semantics
are desired throughout.

---

## #111 — `%y` prompt escape (current tty) not expanded

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'print -P "%y"'
()

$ ./target/debug/zshrs --zsh -c 'print -P "%y"'
%y
```

`%y` in prompt expansion = "current tty without `/dev/`". When
not attached to a tty (script via `-c`), zsh prints `()` as a
placeholder. zshrs returns the literal `%y` (escape not
recognized).

Per `man zshmisc` § PROMPT EXPANSION:
> `%y` — The line (tty) the user is logged in on, without
> `/dev/` prefix.
> `%l` — The line (tty) the user is logged in on, with the
> `/dev/tty` prefix removed.

Both `%y` and `%l` (already noted in similar batches) are missing.
Same family as #38 (prompt escapes coverage gap), #96 (`%N/`
truncation), #111 (this one).

**Where** — `src/ported/prompt.rs::expand_escape`: lookup table
for prompt escape characters missing entries for `y`, `l`, `M`,
`v`, etc. C-source `Src/prompt.c::putprompt` has a dispatch
switch covering all of them.

**Impact** — any `.zshrc` PROMPT using terminal-identifying
escapes:

```sh
PROMPT='%n@%m %y %# '
# zsh: "wizard@laptop ttys003 $ "
# zshrs: "wizard@laptop %y $ "
```

User has to either avoid `%y`/`%l` or implement them manually
via `${TTY##*/}`:

**Workaround** — substitute via `$TTY`:
```sh
PROMPT="%n@%m ${TTY##*/} %# "
```

---

## #112 — Builtin error format leaks Rust's `io::Error` "(os error N)" suffix

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'cd /nonexistent_xyz 2>&1'
zsh:cd:1: no such file or directory: /nonexistent_xyz

$ ./target/debug/zshrs --zsh -c 'cd /nonexistent_xyz 2>&1'
zsh:cd:1: No such file or directory (os error 2): /nonexistent_xyz
```

zsh's standard error format: `zsh:cd:1: no such file or directory:
/path`. zshrs's version capitalizes 'N' AND appends ` (os error 2)`
— the Rust `std::io::Error` Display implementation leaks into
user-visible output.

Same for `mkdir`:
```sh
$ /opt/homebrew/bin/zsh -fc 'mkdir /no/such/path 2>&1'
mkdir: /no/such: No such file or directory

$ ./target/debug/zshrs --zsh -c 'mkdir /no/such/path 2>&1'
zsh:mkdir:1: cannot make directory `/no/such/path': No such file or directory (os error 2)
```

zshrs's `mkdir` builtin (per #28) has the Rust error leak; also
the format `cannot make directory '...'` is GNU coreutils style
not the BSD-style `mkdir: /path: msg` zsh inherits.

**Where** — `src/ported/builtin_cd.rs::run` / `src/ported/builtin_mkdir.rs`:
errors formatted via `format!("{}", io_err)` or `.to_string()`.
Should map known errno values to zsh-canonical lowercase strings:
- ENOENT → "no such file or directory"
- EACCES → "permission denied"
- EEXIST → "file exists"
- ENOTDIR → "not a directory"
- EISDIR → "is a directory"

C-source `Src/builtin.c::cd_try_chdir` calls `zwarnnam(name,
"%e: %s", strerror(errno), path)`.

**Impact** — error-message-parsing scripts that grep for specific
zsh format fail under zshrs. CI test fixtures that compare error
output across shells break. The `(os error 2)` suffix is
implementation-detail leak that zsh-compat code wouldn't expect.

**Workaround** — none portable; user must accept different error
strings or grep loosely (`*: no such file*` works for both shells
modulo case).

---

## #113 — `$'\C-X'` ANSI-C control-character escape not honored (literal)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc "echo \$'\\C-a'" | od -c | head -1
0000000  001  \n

$ ./target/debug/zshrs --zsh -c "echo \$'\\C-a'" | od -c | head -1
0000000    C   -   a  \n
```

`$'\C-X'` is the bash/zsh ANSI-C quoting form for "Ctrl-X"
control characters. `\C-a` = byte 0x01 (Ctrl-A), `\C-h` = 0x08
(backspace), etc. zsh produces the actual control byte. zshrs
outputs the literal three characters `C-a`.

Other `$'...'` escapes work in both shells:
- `$'\n'` → newline (works)
- `$'\t'` → tab (works)
- `$'\x41'` → 'A' (works per earlier tests)
- `$'\041'` → '!' (works)

So the bug is specifically the `\C-X` notation, which zsh
recognizes per `man zshmisc` § QUOTING:
> `\C-x` — control character with the value of `x XOR @`.

**Where** — `src/ported/lex.rs::parse_dollar_quote`: ANSI-C
escape sequence table missing the `\C-X` form. C-source
`Src/utils.c::getkeystring` recognizes `\C` followed by a
character.

**Impact** — keybinding scripts that use `$'\C-X'` to specify
key combos break:

```sh
bindkey "$'\C-x\C-e'" edit-command-line
# zsh: binds Ctrl-X Ctrl-E
# zshrs: tries to bind the literal string "C-xC-e"
```

User-defined readline-style key macros also fail.

**Workaround** — use `\xNN` hex escapes for the same byte values:
```sh
bindkey "$'\x18\x05'" edit-command-line   # Ctrl-X (0x18), Ctrl-E (0x05)
```

---

## #114 — `${(l.W.)s}` left-pad width must be literal; variable name parses as "bad substitution"

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'w=5; s=hi; echo "[${(l.w.)s}]"'
[   hi]

$ ./target/debug/zshrs --zsh -c 'w=5; s=hi; echo "[${(l.w.)s}]"' 2>&1
zsh:1: bad substitution
```

In zsh, the width argument to `(l.WIDTH.)` or `(r.WIDTH.)` can be
either a literal numeric or a **parameter name** (which gets
expanded to a number). `w=5; ${(l.w.)s}` pads `s` to 5 chars
using the variable `w`'s value.

zshrs requires a literal numeric — `${(l.5.)s}` works, but
`${(l.w.)s}` errors with "bad substitution".

`${(l.$w.)s}` (with explicit `$` expansion) also fails:
```sh
$ /opt/homebrew/bin/zsh -fc 'w=5; s=hi; echo "[${(l.$w.)s}]"'
[   hi]

$ ./target/debug/zshrs --zsh -c 'w=5; s=hi; echo "[${(l.$w.)s}]"' 2>&1
zsh:1: bad substitution
```

Per `man zshexpn` § Parameter Expansion Flags:
> `l:expr:string1:string2:` — Pad the resulting words on the
> left. Each word is truncated if required and placed in a field
> `expr` characters wide. ... `expr` can be a math expression.

So even `${(l.((w*2)).)s}` math expression should work; zshrs only
accepts bare literals.

**Where** — `src/ported/paramsubst.rs::parse_pad_width`: parses
only `[0-9]+` literals; doesn't fall through to math evaluator
for the width spec. C-source `Src/subst.c::getargnum` runs
`mathevali` on the whole expression.

**Impact** — dynamic padding/justification idioms break:

```sh
# right-align price column to widest entry
typeset -i width
for p in "${prices[@]}"; do
    (( ${#p} > width )) && width=${#p}
done
for p in "${prices[@]}"; do
    echo "${(r.width.)p}"
done
# zsh: clean right-aligned column
# zshrs: bad substitution error on every iteration
```

**Workaround** — use `printf` width spec with explicit expansion:
```sh
printf "%${width}s\n" "$p"   # both shells: same output
```

---

## #115 — Prompt `%s`/`%b`/`%u` use full reset `\e[0m` instead of selective

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'print -P "%Sstandout%s normal %Bbold%b plain %Uunder%u final"' | od -c
... \033 [ 7 m standout \033 [ 2 7 m  normal \033 [ 1 m bold \033 [ 0 m  plain ...

$ ./target/debug/zshrs --zsh -c 'print -P "%Sstandout%s normal %Bbold%b plain %Uunder%u final"' | od -c
... \033 [ 7 m standout \033 [ 0 m  normal \033 [ 1 m bold \033 [ 0 m  plain ...
```

zsh's prompt attribute-off escapes emit **selective reset codes**:
- `%s` (standout off) → `\033[27m` (SGR 27 = standout-off)
- `%b` (bold off)     → `\033[22m` (SGR 22 = bold-off)
- `%u` (underline off) → `\033[24m` (SGR 24 = underline-off)

zshrs emits the **full reset** `\033[0m` for all three, which
clobbers every other active attribute (color, italic, etc.) along
with the one being un-set.

Per `man zshmisc` § VISUAL EFFECTS:
> `%S` — Start (set) standout mode. `%s` — End standout mode.
> `%U` — Start underline mode. `%u` — End underline mode.
> `%B` — Start bold mode. `%b` — End bold mode.

The expected semantic is "end THIS mode only", not "reset all".

**Where** — `src/ported/prompt.rs::format_attr_end`: emits
`"\x1b[0m"` for `%s`, `%b`, `%u`. Should emit `"\x1b[27m"` /
`"\x1b[22m"` / `"\x1b[24m"` respectively. C-source
`Src/prompt.c::putprompt` has these as `tcout(TCSTANDOUTEND)`,
`tcout(TCALLATTRIBUTESOFF)` and `tcout(TCUNDERLINEEND)` mapped
to termcap caps.

**Impact** — prompts that combine attributes break visually:

```sh
PROMPT='%F{red}%Bbold%b regular%f'
# zsh: red-bold "bold" then red "regular" (red preserved)
# zshrs: red-bold "bold" then default-color "regular" (red killed by %b)
```

Every multi-attribute prompt theme produces wrong rendering.

**Workaround** — manual SGR codes instead of `%`-escapes:
```sh
PROMPT=$'\e[31m\e[1mbold\e[22m regular\e[0m'
```
Or re-apply the desired attributes after each "off":
```sh
PROMPT='%F{red}%Bbold%b%F{red}regular%f'
```

---

## #116 — `GLOB_SUBST` option default is ON in zshrs (zsh: OFF by default)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'pat="hel*"; s=hello; [[ "$s" == $pat ]] && echo "match"; echo done'
done

$ ./target/debug/zshrs --zsh -c 'pat="hel*"; s=hello; [[ "$s" == $pat ]] && echo "match"; echo done'
match
done
```

`GLOB_SUBST` controls whether parameter values containing glob
metachars get glob-expanded when substituted into pattern
contexts (like the RHS of `[[ ==`).

zsh's default: **OFF**. So `$pat` (containing `*`) is treated as
literal when expanded into `[[ "$s" == $pat ]]`, hence the no-match.

zshrs's default: **ON**. So `*` in `$pat` becomes a glob metachar,
matches `hello`.

Setting `setopt glob_subst` in zsh makes it match zshrs's default:
```sh
$ /opt/homebrew/bin/zsh -fc 'setopt glob_subst; pat="hel*"; s=hello; [[ "$s" == $pat ]] && echo "match"'
match
```

So both shells agree on behavior given the same option state —
zshrs just defaults differently.

Per `man zshoptions`:
> `GLOB_SUBST` <K> <S> — Treat any characters resulting from
> parameter expansion as being eligible for filename generation
> and pattern matching. Without the option, no characters acquire
> special meaning during expansion.
> Default: off.

**Where** — `src/ported/init.rs::default_options`: `GLOB_SUBST`
should be initialized to off, mirroring `Src/options.c::
ksh_compat_options` and the upstream default. zshrs has it on.

**Impact** — silent behavior divergence:

```sh
# user code expecting literal-pattern match
expected='*.log'
if [[ "$file" == "$expected" ]]; then
    echo "matches literal *.log"
fi
# zsh: only matches the literal filename "*.log"
# zshrs: matches ANY .log file (silent over-match)
```

Cross-shell scripts using pattern matching get false positives in
zshrs that catch in zsh.

**Workaround** — explicit `unsetopt glob_subst` at start of any
zsh-portable script:
```sh
emulate -L zsh
unsetopt glob_subst
```

---

## #117 — Extended_glob `(group)#` / `(group)##` group quantifier not recognized

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extended_glob; print -l /tmp/zgq/(ab)#'
/tmp/zgq/ab
/tmp/zgq/abab
/tmp/zgq/ababab

$ ./target/debug/zshrs --zsh -c 'setopt extended_glob; print -l /tmp/zgq/(ab)#'
/tmp/zgq/(ab)#
```

Files: `ab abab ababab xy`. Pattern `(ab)#` (group followed by
`#`) = "match the group zero or more times". zsh matches all
three `ab`-repeating names. zshrs returns the literal pattern.

The single-char `a#` quantifier was documented in bug #89; this
is the **group form** `(group)#`. Same root cause but a different
parser path (single-char vs group).

Family with:
- #62 (extended_glob `~` and-not)
- #81 (`~` exclusion produces duplicates)
- #89 (`#` / `##` single-char quantifier)
- #99 (`(#cN,M)` count flag)
- #117 (this — group quantifier)

The whole extended_glob quantifier/flag family is partial or
absent.

**Where** — `src/ported/pattern.rs::compile_group_quantifier`:
when `(...)` is followed by `#` or `##`, the parser should attach
the quantifier to the group, not treat them as literal. C-source
`Src/pattern.c::patcompile` handles this in the group-postfix
path.

**Impact** — log-rotation/repetition patterns silently fail:

```sh
setopt extended_glob
# match files with prefix that repeats "log" zero or more times
ls /var/(log)#/messages
# zsh: matches /var/messages, /var/log/messages, /var/log/log/messages
# zshrs: returns literal "/var/(log)#/messages"
```

**Workaround** — explicit loop with `**/` recursive glob if the
intent is matching at varying depths.

---

## #118 — `(( y = x ))` doesn't coerce string `x` to integer; stores raw string

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'x=hello; (( y = x )); echo "type: $(typeset -p y) val=[$y]"'
type: typeset -i y=0 val=[0]

$ ./target/debug/zshrs --zsh -c 'x=hello; (( y = x )); echo "type: $(typeset -p y) val=[$y]"'
type: typeset y=hello val=[hello]
```

In arithmetic context `(( ))`, zsh treats `x` as a variable
reference. When the value isn't a number, zsh recursively
resolves until it bottoms out at 0 (the unset-resolves-to-0
semantic). `y` gets typed `integer` with value `0`.

zshrs's `(( ))` assignment skips the arith-context coercion —
stores the raw string and types `y` as scalar (not integer).

Note: `integer y; y=$x` works correctly in both shells (forces
integer assignment).

Per `man zshmisc` § ARITHMETIC EVALUATION:
> Variables ... are used by name. ... If the variable does not
> contain a number, the value is considered to be zero.

**Where** — `src/ported/math.rs::eval_assignment`: the LHS-of-`=`
declaration path doesn't promote the target to `PM_INTEGER` type,
and doesn't coerce the RHS value via the arith-recurse rule.
C-source `Src/math.c::matheval` sets `PM_INTEGER` and runs
`getnparam` on RHS, which coerces.

**Impact** — code that intentionally uses arith-context for
auto-typing breaks:

```sh
parse_count() {
    local val=$1
    (( count = val ))
    # zsh: count is integer, val coerced to 0 if non-numeric
    # zshrs: count is scalar, may hold non-numeric content
    (( count > 0 )) && process_records $count
    # zshrs: errors on the arith comparison or runs with wrong type
}
```

**Workaround** — explicit `integer` declaration:
```sh
integer count=$val
```
Or pre-validate:
```sh
[[ "$val" =~ "^[0-9]+$" ]] || val=0
(( count = val ))
```

---

## #119 — `setopt glob_subst` doesn't trigger filename expansion of substituted patterns in for-loop

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt glob_subst; pat="*.txt"; for f in /tmp/zgs/$pat; do echo "$f"; done'
/tmp/zgs/a.txt

$ ./target/debug/zshrs --zsh -c 'setopt glob_subst; pat="*.txt"; for f in /tmp/zgs/$pat; do echo "$f"; done'
/tmp/zgs/*.txt
```

With `setopt glob_subst` explicitly set, zsh treats characters in
expanded parameter values as glob metacharacters — `$pat="*.txt"`
expands to glob-pattern in the `for` loop's word list, matches
`/tmp/zgs/a.txt`.

zshrs returns the literal `/tmp/zgs/*.txt` even with `glob_subst`
set. So the GLOB_SUBST option doesn't actually trigger filename
expansion of substituted patterns in this context.

Contrasts with bug #116 (`GLOB_SUBST` default ON in zshrs for
`[[ == ]]` context). So zshrs has the option default ON for
pattern-match context but ignores it for filename-expansion
context — inverted from zsh.

Per `man zshoptions`:
> `GLOB_SUBST` — Treat any characters resulting from parameter
> expansion as being eligible for filename generation and pattern
> matching.

Should apply to BOTH filename and pattern contexts equally; zshrs
splits the behavior.

**Where** — `src/ported/exec.rs::expand_words`: the `for` loop
word-expansion path doesn't consult `opts.glob_subst` when
deciding whether substituted glob chars trigger filename gen.
C-source `Src/exec.c::execfor` walks `args` and runs `globlist`
when `GLOBSUBST` is set.

**Impact** — common pattern of "parameterized glob in for loop"
silently fails:

```sh
setopt glob_subst
patterns=("*.log" "*.txt" "*.bak")
for pattern in "${patterns[@]}"; do
    for f in /var/data/$pattern; do
        process "$f"
    done
done
# zsh: iterates matched files
# zshrs: iterates literal pattern strings (process gets "*.log" etc.)
```

**Workaround** — explicit `eval` to force re-expansion:
```sh
for f in $(eval "echo /var/data/$pattern"); do ... done
```

---

## #120 — `a=("${a[@]:0:-1}")` on empty array creates 1-element array

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(); a=("${a[@]:0:-1}"); echo "len=${#a}"'
len=0

$ ./target/debug/zshrs --zsh -c 'a=(); a=("${a[@]:0:-1}"); echo "len=${#a}"'
len=1
```

`${a[@]:0:-1}` on an empty array — zsh returns no elements; the
self-assignment leaves `a` empty (`len=0`).

zshrs returns one element (likely an empty string) — the self-
assignment results in a one-element array containing `""`. From
`len=1`.

This is the "drop-last-element" idiom (bug #16) family — but
specifically the **empty-array** edge case is broken.

```sh
# pop the last element repeatedly until empty
while (( ${#a} > 0 )); do
    last=${a[-1]}
    a=("${a[@]:0:-1}")
    process "$last"
done
# zsh: terminates cleanly (a becomes empty, loop exits)
# zshrs: infinite loop (a always has 1 element, ${#a} > 0 always)
```

**Where** — `src/ported/paramsubst.rs::array_slice_negative_end`:
when `end_offset` results in a length that should be 0, returns
`[""]` (single empty) instead of `[]` (empty array). C-source
`Src/subst.c::arrslice` returns `NULL`/empty list when start ==
end.

**Impact** — pop-until-empty patterns infinite-loop. Same root
shape as #16 (already documented as no-shrink in fn context).

**Workaround** — explicit length check:
```sh
while (( ${#a} > 0 )); do
    last=${a[-1]}
    if (( ${#a} == 1 )); then
        a=()
    else
        a=("${a[@]:0:-1}")
    fi
    process "$last"
done
```

---

## #121 — `[[ -N -op -M ]]` with negative numbers errors "unknown condition"

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '[[ -5 -lt -3 ]] && echo "yes" || echo "no"'
yes

$ ./target/debug/zshrs --zsh -c '[[ -5 -lt -3 ]] && echo "yes" || echo "no"' 2>&1
zshrs:1: unknown condition: -5
zsh:1: command not found: -3
no
```

`[[ ... ]]` test with negative-number operands. zsh correctly
parses `-5 -lt -3` as the integer comparison `-5 < -3` (true).
zshrs's test parser sees `-5` as an unrecognized test-flag (like
`-d`, `-f`, `-n`) and errors out, then tries to execute `-3` as a
command (also fails).

Verified across all numeric ops:
```sh
$ /opt/homebrew/bin/zsh -fc '[[ -1 -lt 0 ]] && echo "lt0"; [[ -1 -gt 0 ]] && echo "gt0"; echo "done"'
lt0
done

$ ./target/debug/zshrs --zsh -c '[[ -1 -lt 0 ]] && echo "lt0"; [[ -1 -gt 0 ]] && echo "gt0"; echo "done"' 2>&1
zshrs:1: unknown condition: -1
zsh:1: command not found: 0
done
```

The test parser tries to dispatch `-1` as a test flag before
recognizing it could be an integer literal in the operand position.

Workaround using a variable doesn't help:
```sh
$ ./target/debug/zshrs --zsh -c 'a=-5; b=-3; [[ $a -lt $b ]] && echo "yes"' 2>&1
zshrs:1: unknown condition: -5
```

**Where** — `src/ported/cond.rs::parse_condition`: token
classifier sees a `-` prefix and routes to unary-flag handler
without checking whether the next pos is a binary integer op.
C-source `Src/test.c::test_expr` looks ahead 1 token: if it sees
`-eq`/`-lt`/etc., treats both sides as integer operands.

**Impact** — every numeric comparison involving negative numbers
fails. Pattern that runs fine in zsh:

```sh
delta=$((current - threshold))
if [[ $delta -lt 0 ]]; then
    echo "below threshold by ${delta#-}"
fi
# zsh: works
# zshrs: errors on the [[ comparison
```

**Workaround** — use arith context `(( ))` for numeric tests:
```sh
(( delta < 0 )) && echo "below threshold by ${delta#-}"
```

---

## #122 — Exit status of `$()` inside `${x:-$(...)}` not propagated to `$?`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'unset x; y="${x:-$(false)}"; echo "1:ec=$?"; y="${x:-$(exit 7)}"; echo "2:ec=$?"'
1:ec=1
2:ec=7

$ ./target/debug/zshrs --zsh -c 'unset x; y="${x:-$(false)}"; echo "1:ec=$?"; y="${x:-$(exit 7)}"; echo "2:ec=$?"'
1:ec=0
2:ec=0
```

When `$()` runs as the default-value branch of `${x:-...}`, zsh
preserves its exit status — `$?` after the assignment reflects
the cmdsub's status. zshrs loses it (always 0).

Standalone cmdsub exit propagates correctly:
```sh
$ ./target/debug/zshrs --zsh -c 'x=$(exit 7); echo "ec=$?"'
ec=7
```

So the bug is specifically about cmdsub inside parameter-expansion
default `${:-$()}`.

**Where** — `src/ported/paramsubst.rs::default_branch`: doesn't
capture and propagate the exit status of the cmdsub run during
the `:-` default-evaluation. C-source `Src/subst.c::dosubst`
threads `lastval` through the substitution.

**Impact** — fallback patterns that use cmdsub status as a
diagnostic break:

```sh
config_path="${CONFIG:-$(detect_config)}"
if (( $? != 0 )); then
    echo "warn: detect_config failed, using default"
    config_path=/etc/default.conf
fi
# zsh: warns when detect_config fails
# zshrs: never warns (status always 0)
```

**Workaround** — explicit pre-eval the cmdsub before assignment:
```sh
if [[ -z "$CONFIG" ]]; then
    config_path=$(detect_config)
    cmdsub_ec=$?
else
    config_path=$CONFIG
fi
```

---

## #123 — `${arr[@]}` inside heredoc body returns only first element

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'arr=(a b c); cat <<END
${arr[@]}
END'
a b c

$ ./target/debug/zshrs --zsh -c 'arr=(a b c); cat <<END
${arr[@]}
END'
a
```

`${arr[@]}` should enumerate all array elements joined by space
when expanded inside a heredoc body. zsh produces `a b c`. zshrs
returns only `a` (first element).

Same context-propagation family as #82, #83, #108, #109, #120 —
array vs scalar context handling in paramsubst.

Verified other forms:
- `"${arr[@]}"` quoted form: same bug (only first element)
- `${arr[*]}` (star form): same bug
- `${arr}` (bare): same bug
- `${(j: :)arr}` (explicit join): works correctly

So heredoc context loses the array-iteration behavior entirely;
only explicit join-flag works.

**Where** — `src/ported/lex.rs::expand_heredoc_body`: array
expansion in heredoc-token context doesn't recognize `[@]`/`[*]`
to expand to multiple elements; treats as scalar single-element.
C-source `Src/parse.c::gettokstr` walks the heredoc body and
calls `paramsubst` with full array-context flags.

**Impact** — heredocs are the canonical way to emit multi-line
templates containing array contents (env file generation,
config templates, etc.):

```sh
hosts=(web1.local web2.local db.local)
cat > /etc/hosts.d/allowed <<END
allowed-hosts=${hosts[*]}
backends=${hosts[@]}
END
# zsh: writes "allowed-hosts=web1.local web2.local db.local\n
#               backends=web1.local web2.local db.local"
# zshrs: writes only "allowed-hosts=web1.local\nbackends=web1.local"
```

**Workaround** — pre-join into a scalar variable:
```sh
hosts_str="${hosts[*]}"
cat > /etc/hosts.d/allowed <<END
allowed-hosts=$hosts_str
backends=$hosts_str
END
```
Or use `${(j: :)hosts}` explicit-join form (works in heredoc).

---

## #124 — `typeset -f` returns source-as-typed; zsh pretty-prints with indentation

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'outer() { inner() { echo "in inner"; }; inner; }; typeset -f outer'
outer () {
	inner () {
		echo "in inner"
	}
	inner
}

$ ./target/debug/zshrs --zsh -c 'outer() { inner() { echo "in inner"; }; inner; }; typeset -f outer'
outer () {
	inner() { echo "in inner"; }; inner
}
```

`typeset -f` should dump function bodies in zsh's canonical
pretty-printed form: one statement per line, nested functions
indented by tabs, brace placement consistent.

zsh's output is the **reformatted/normalized** representation
(parsed → AST → emitted with indent). zshrs's output is the
**original source text** as the user typed it (semicolon-
separated inline form preserved).

The contents are semantically equivalent but the byte stream
differs significantly — any test fixture comparing function
output byte-for-byte fails.

Per `man zshbuiltins`:
> `typeset -f` — Print each function definition.

The implicit convention (per all extant zsh installations) is
that `-f` output goes through the prompt-output formatter, which
indents nested constructs.

**Where** — `src/ported/builtin_typeset.rs::print_function`:
emits stored source text directly. C-source
`Src/builtin.c::printfuncdef` calls `getpermtext` which walks
the parsed AST and re-emits via `outputblock`/`outputblock_pr`
with `\t` indentation.

**Impact** — config-snapshot tools that dump+diff function
definitions across shell versions get false positives:

```sh
# pre-flight check: snapshot installed fns
expected=$(typeset -f compinit)
... do work that may patch compinit ...
new=$(typeset -f compinit)
[[ "$expected" == "$new" ]] || alert "compinit changed"
# zsh: stable (always-formatted)
# zshrs: fluctuates based on source whitespace (inline vs multi-line)
```

**Workaround** — normalize whitespace before comparing:
```sh
norm_fn() { typeset -f "$1" | tr -s '[:space:]' ' '; }
[[ "$(norm_fn fn)" == "$(norm_fn fn)" ]] && echo "same"
```

---

## #125 — `var=${a[-1]}` assignment from negative subscript returns empty

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(1 2 3 4); last=${a[-1]}; echo "[$last]"'
[4]

$ ./target/debug/zshrs --zsh -c 'a=(1 2 3 4); last=${a[-1]}; echo "[$last]"'
[]
```

Standalone `echo "${a[-1]}"` works in both shells — returns the
last element. The bug appears specifically when the negative-
subscript expansion is the **RHS of a variable assignment**:
zshrs assigns empty string to `last`.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(1 2 3 4); echo "[${a[-1]}]"'
[4]

$ ./target/debug/zshrs --zsh -c 'a=(1 2 3 4); echo "[${a[-1]}]"'
[4]
```

Direct expansion works. Only the assignment-context form `var=`
fails.

Verified with multiple assignment forms — same failure:
```sh
last=${a[-1]}       # empty
last="${a[-1]}"     # empty
declare last=${a[-1]} # empty
```

Related to bug #17 (`var=${arr[-1]}` unquoted in fn while
loops) — that documented the symptom in fn-loop context;
this bug shows the issue is more general.

**Where** — `src/ported/paramsubst.rs::eval_subscript_neg`:
when the negative-subscript evaluation happens in an
assignment-RHS context, the value gets dropped before storage.
Likely a context-flag check that's wrong — the `assignment`
context path doesn't run the negative-subscript translation.
C-source `Src/params.c::sethparam` runs the same `getarrelt`
that the expansion side uses.

**Impact** — every "pop last element" pattern silently
produces empty values:

```sh
a=(host1 host2 host3 host4)
while (( ${#a} > 0 )); do
    last=${a[-1]}
    a=("${a[@]:0:-1}")
    process "$last"
done
# zsh: processes each host
# zshrs: passes empty string each iteration
```

Combined with bug #120 (`a=("${a[@]:0:-1}")` on empty doesn't
shrink), this loop also infinite-loops in zshrs.

**Workaround** — explicit positive index using `${#a}`:
```sh
last=${a[${#a}]}    # positive subscript = ${#a}, gets last
```

---

## #126 — `${s:N:}` (empty length suffix) silently empty; zsh errors

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 's="hello"; echo "[${s:0:2}${s:2:}]"' 2>&1
zsh:1: closing brace expected

$ ./target/debug/zshrs --zsh -c 's="hello"; echo "[${s:0:2}${s:2:}]"'
[he]
```

`${s:N:}` with empty length specifier is malformed in zsh (zsh
errors "closing brace expected" because length must be present
when colon follows offset). zshrs silently treats it as
zero-length substring (returns empty) instead of erroring.

Variants of malformed substring syntax all behave like this:
- `${s::}` — empty offset + empty length
- `${s:2:}` — present offset, empty length
- `${s:}` — only colon, no offset

zsh refuses all; zshrs silently accepts and returns empty.

**Where** — `src/ported/paramsubst.rs::parse_substring`:
permissive parser accepts empty operands as zero-length. Should
match C-source `Src/subst.c::getsubstr` strictness — error on
empty operand after `:`.

**Impact** — typos in substring expressions silently produce
empty values instead of errors:

```sh
parse_field() {
    local s=$1 n=${2:?length required}
    echo "${s:0:n}"     # typo: missing variable expansion $
}
parse_field "hello" 3
# zsh: errors on bad substring (or undefined behavior)
# zshrs: returns empty silently → caller gets wrong data
```

**Workaround** — none from user side; the user must be careful
with substring syntax in zshrs to avoid silent failures.

---

## #127 — `$'\xNN'` interpreted as Unicode codepoint, then re-encoded as UTF-8

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo $'\''\xff'\''' | od -An -c
   377  \n

$ ./target/debug/zshrs --zsh -c 'echo $'\''\xff'\''' | od -An -c
   ÿ  **  \n
```

The `$'\xNN'` ANSI-C quoting form should produce the **raw byte**
with value `0xNN`. zsh emits 1 byte `0xff` (octal `377`). zshrs
treats `0xff` as Unicode codepoint U+00FF (`ÿ`) and re-encodes as
UTF-8 → 2 bytes `\xC3\xBF`.

Same for `\xc3\xa9` (which happens to BE the UTF-8 of `é`):
```sh
$ /opt/homebrew/bin/zsh -fc 'echo $'\''\xc3\xa9'\''' | od -An -c
   é  **  \n              # 2 bytes 0xC3 0xA9 (display as é)

$ ./target/debug/zshrs --zsh -c 'echo $'\''\xc3\xa9'\''' | od -An -c
   Ã  **   ©  **  \n      # 4 bytes (UTF-8 of "Ã©")
```

For pure-ASCII `\x41\x42`, both shells produce same 2 bytes
(`A`, `B`) — the divergence only appears with values ≥ 0x80.

Per `man zshmisc` § QUOTING:
> `\xNN` — character with hex value `NN`.

The convention (matching bash, ksh, all `printf '\xNN'`
implementations) is **raw byte**, not Unicode codepoint.

**Where** — `src/ported/lex.rs::parse_hex_escape`: treats the
parsed integer as a Unicode codepoint (`char::from_u32`), then
the resulting string is UTF-8 encoded on output. C-source
`Src/utils.c::getkeystring` calls `*p++ = (char)hex_value` —
single-byte write.

**Impact** — binary-data-handling scripts produce garbage:

```sh
# write a binary header
header=$'\xff\xfe\x00\x01\x00\x02'   # 6 bytes
printf '%s' "$header" > /tmp/img.bin
# zsh: 6-byte file with exact bytes
# zshrs: 8+ byte file with UTF-8-encoded chars
```

Cryptographic key handling, network protocol byte construction,
binary file generation all silently corrupt.

**Workaround** — `printf '\xNN'` directly (the printf builtin
may have a different code path):
```sh
printf '\xff\xfe\x00\x01' > /tmp/img.bin
```
Verify with `od -c` after to confirm bytes are correct.

---

## #128 — `${(C)arr[N]}` capitalize on indexed array element errors "bad substitution"

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(red blue green); echo "[${(C)a[2]}]"'
[Blue]

$ ./target/debug/zshrs --zsh -c 'a=(red blue green); echo "[${(C)a[2]}]"' 2>&1
zsh:1: bad substitution
```

The `(C)` capitalize flag works on scalars and on bare-array
expansions in both shells:
```sh
$ both-shells -fc 'a=(red blue); echo "[${(C)a}]"'
[Red Blue]                     # zsh and zshrs agree

$ both-shells -fc 's=hello; echo "[${(C)s}]"'
[Hello]                        # zsh and zshrs agree
```

But the **indexed-element form** `${(C)a[N]}` fails in zshrs with
"bad substitution".

`(L)` (lowercase) and `(U)` (uppercase) likely have the same
issue when combined with `[N]` subscript.

**Where** — `src/ported/paramsubst.rs::parse_flag_then_subscript`:
when a case-flag like `(C)`/`(L)`/`(U)` is followed by a
subscripted array reference, the parser fails to recognize the
combo. C-source `Src/subst.c::dosubst` handles flag + subscript
in any order.

**Impact** — capitalize/lowercase a specific array element fails:

```sh
items=(red blue green yellow)
echo "Selected: ${(C)items[2]}"
# zsh: "Selected: Blue"          zshrs: bad substitution error
```

**Workaround** — assign to scalar then apply flag:
```sh
sel="${items[2]}"
echo "Selected: ${(C)sel}"
```

---

## #129 — `local -a a=("$@")` splits quoted args on whitespace (without `local -a` works)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { local -a a=("$@"); echo "len=${#a}"; for x in "${a[@]}"; do printf "[%s]\n" "$x"; done; }; f one "two three" four'
len=3
[one]
[two three]
[four]

$ ./target/debug/zshrs --zsh -c 'f() { local -a a=("$@"); echo "len=${#a}"; for x in "${a[@]}"; do printf "[%s]\n" "$x"; done; }; f one "two three" four'
len=4
[one]
[two]
[three]
[four]
```

`local -a a=("$@")` should preserve each positional arg as a
distinct element. zsh gets 3 elements (the quoted `"two three"`
stays as one). zshrs gets 4 (word-splits "two three").

Without `local -a` (just `a=("$@")`), both shells produce 3:
```sh
$ both-shells -fc 'f() { a=("$@"); echo "len=${#a}"; }; f one "two three" four'
len=3
```

So the bug is specifically the combination `local -a` with
`"$@"` in initializer.

**Where** — `src/ported/builtin_typeset.rs::declare_array_init`:
when `-a` flag is present and the initializer is `("$@")`, the
positionals get re-tokenized via word-split instead of being
copied verbatim. C-source `Src/builtin.c::typeset_single` walks
positional array element-by-element without splitting.

**Impact** — function argument forwarding via `local -a a=("$@")`
silently corrupts argument boundaries:

```sh
runner() {
    local -a cmd=("$@")
    ssh remote "${cmd[@]}"
}
runner ls "-la" "/tmp with space"
# zsh: ssh runs `ls -la "/tmp with space"`
# zshrs: ssh runs `ls -la /tmp with space`  (4 args, path broken)
```

**Workaround** — assign separately:
```sh
local -a a
a=("$@")
```

---

## #130 — `${var@X}` bash parameter-transform notation accepted (zsh errors)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'x="a b"; echo "[${x@Q}]"' 2>&1
zsh:1: bad substitution

$ ./target/debug/zshrs --zsh -c 'x="a b"; echo "[${x@Q}]"' 2>&1
[a b]
```

`${var@X}` is **bash's** parameter-transformation notation
(`@Q`, `@U`, `@L`, `@P`, `@A`, etc.). zsh has no such syntax —
parses as "bad substitution". zshrs accepts and processes the
form, often incorrectly:

```sh
$ ./target/debug/zshrs --zsh -c 'x="hello"; echo "@U=[${x@U}]"; echo "@L=[${x@L}]"' 2>&1
@U=[hello]    # @U should uppercase but returns original
@L=[HELLO]    # @L should lowercase but returns uppercase
```

So zshrs both accepts the bash extension AND implements its
semantics incorrectly (transforms inverted).

**Where** — `src/ported/paramsubst.rs::parse_transform_op`:
recognizes `@X` bash grammar; zsh-compat should reject as
"bad substitution". C-source `Src/subst.c::dosubst` errors on
unknown form.

**Impact** — bash-only scripts using `${var@X}` work in zshrs
but fail in real zsh — false sense of cross-shell portability.
Plus incorrect implementations produce silently-wrong values.

**Workaround** — use zsh-canonical flag syntax:
```sh
echo "${(U)x}"     # uppercase, both shells
echo "${(L)x}"     # lowercase
echo "${(q)x}"     # quote
```

---

## #131 — `%(N~.A.B)` prompt-conditional evaluates path-depth wrong

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ pwd
/Users/wizard/RustroverProjects/zshrs   # 2 components under $HOME

$ /opt/homebrew/bin/zsh -fc 'print -P "%(1~.A.B)"'
A

$ ./target/debug/zshrs --zsh -c 'print -P "%(1~.A.B)"'
B
```

`%(N~.A.B)` ternary: outputs `A` if PWD has at least N components
relative to `$HOME`, else `B`. PWD is 2 levels under HOME, so
condition is true (≥1) → zsh prints `A`. zshrs evaluates as false,
prints `B`.

Same for `%(0~...)`:
```sh
$ /opt/homebrew/bin/zsh -fc 'print -P "%(0~.A.B)"'
A

$ ./target/debug/zshrs --zsh -c 'print -P "%(0~.A.B)"'
B
```

Per `man zshmisc` § CONDITIONAL SUBSTRINGS:
> `%(x.true.false)` — Ternary expression based on test `x`.
> `~` — PWD home-relative has at least `n` components.

**Where** — `src/ported/prompt.rs::eval_ternary`: the `~`
discriminator returns wrong boolean. C-source
`Src/prompt.c::pmptest` walks the home-stripped path counting
components.

**Impact** — every prompt theme using `%(N~...)` conditional
chooses wrong branch. Common dotfile snippet:

```sh
PROMPT='%(3~.%F{red}.%F{green})%~%f $ '
# zsh: red when 3+ levels deep, green otherwise
# zshrs: always one branch
```

Family with #96 (`%N/`/`%N~` truncation), #115 (selective vs
full reset), #38/#111 (prompt-escape coverage) — prompt
expansion has systematic gaps.

**Workaround** — manual depth check in `precmd`:
```sh
precmd() {
    local n=${#${(s:/:)PWD#$HOME/}}
    if (( n >= 3 )); then prompt_clr="%F{red}"
    else prompt_clr="%F{green}"; fi
}
```

---

## #132 — `(( x = "5" + "3" ))` doesn't coerce quoted numeric strings to int

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '(( x = "5" + "3" )); echo "[$x]"'
[8]

$ ./target/debug/zshrs --zsh -c '(( x = "5" + "3" )); echo "[$x]"'
[0]
```

In arith context `(( ))`, zsh strips quotes from `"5"` and
`"3"` and recognizes the contents as numerics → `5 + 3 = 8`.
zshrs treats quoted strings as opaque, returns 0.

Bare numerics work in both:
```sh
$ both-shells -fc '(( x = 5 + 3 )); echo $x'
8
```

So the bug is the **quoted-numeric-string** path inside `(( ))`.

Related to bug #118 (`(( y = x ))` where `x` is non-numeric
var). Both stem from arith-context coercion gaps.

**Where** — `src/ported/math.rs::parse_quoted_string`: returns
string-as-is; should call numeric-parse on the inner contents.
C-source `Src/math.c::lexconstant` strips quotes and calls
`zstrtol_underscore`.

**Impact** — arithmetic from quoted-string sources (cmdsub
output, json/csv parsed values) silently produces 0:

```sh
csv_field='42'
total=0
(( total += "$csv_field" ))   # zsh: 42  zshrs: 0
```

```sh
prices=("19" "29" "39")
total=0
for p in "${prices[@]}"; do
    (( total += "$p" ))
done
# zsh: total=87        zshrs: total=0
```

**Workaround** — drop quotes when arg is known-numeric:
```sh
(( total += $csv_field ))   # both shells: works
```

---

## #133 — `zstat -F "fmt"` format flag ignored; output uses default date string

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'zmodload zsh/stat; zstat -F "%Y" +mtime /etc/hosts'
2026

$ ./target/debug/zshrs --zsh -c 'zmodload zsh/stat; zstat -F "%Y" +mtime /etc/hosts' 2>/dev/null
Thu May 21  0:14:26 EDT 2026
```

`zstat -F "FMT"` should format time fields via `strftime` using
the given format string. zsh returns the year only (`%Y`). zshrs
returns the default `date(1)`-style string, ignoring `-F`.

Other formats also ignored:
```sh
$ /opt/homebrew/bin/zsh -fc 'zmodload zsh/stat; zstat -F "%Y-%m-%d" +mtime /etc/hosts'
2026-05-21

$ ./target/debug/zshrs --zsh -c 'zmodload zsh/stat; zstat -F "%Y-%m-%d" +mtime /etc/hosts' 2>/dev/null
Thu May 21  0:14:26 EDT 2026
```

Per `man zshmodules` § The zsh/stat Module:
> `-F fmt` — Specify a `strftime` format for time-valued fields.

**Where** — `src/ported/builtin_zstat.rs::format_time_field`:
ignores the `-F` flag value and uses a hardcoded default
formatter. C-source `Src/Modules/stat.c::stat_print` calls
`strftime` with the `-F` argument.

**Impact** — date-parseable output from `zstat` requires custom
format strings (`-F "%s"` for epoch, `-F "%Y-%m-%d"` for ISO).
zshrs's hardcoded output isn't parseable by downstream tools
expecting epoch or ISO format.

**Workaround** — use `stat`(1) external command with `-f` (BSD)
or `--format` (GNU):
```sh
stat -f "%m" /etc/hosts          # BSD/macOS: epoch
stat --format "%Y" /etc/hosts    # GNU: epoch
```

---

## #134 — `${var:h}` modifier on empty string returns `/` (zsh: `.`)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f=""; echo "${f:h}"'
.

$ ./target/debug/zshrs --zsh -c 'f=""; echo "${f:h}"'
/
```

`${var:h}` modifier is the "head" (directory-portion) equivalent
of `dirname`. Per POSIX `dirname` semantics, the head of an
empty string is `.` (current directory). zsh follows this.
zshrs returns `/` (root directory).

Edge-case test confirms only the empty-string case diverges:
```sh
$ /opt/homebrew/bin/zsh -fc 'for p in "" "/" "foo" "a/b" "/a/b"; do printf "p=%s h=%s\n" "$p" "${p:h}"; done'
p= h=.        # zsh: empty → .
p=/ h=/       # both: /
p=foo h=.     # both: .
p=a/b h=a     # both: a
p=/a/b h=/a   # both: /a

$ ./target/debug/zshrs --zsh -c 'for p in "" "/" "foo" "a/b" "/a/b"; do printf "p=%s h=%s\n" "$p" "${p:h}"; done'
p= h=/        # zshrs: empty → /  (DIFFERS)
p=/ h=/
p=foo h=.
p=a/b h=a
p=/a/b h=/a
```

So all non-empty cases match; only empty input diverges.

Per `man zshexpn` § HISTORY EXPANSION (which documents modifier
semantics):
> `h` — Remove a trailing pathname component, leaving the head.

POSIX `dirname` spec: `dirname ""` is `.`.

**Where** — `src/ported/modifier.rs::apply_h`: when input is
empty, returns `/` instead of `.`. C-source
`Src/subst.c::removepathname` returns "." when no path
separator found.

**Impact** — defensive code using `:h` on potentially-empty
input picks wrong default:

```sh
file=$1
dir="${file:h}"
[[ -d "$dir" ]] || mkdir "$dir"
# If $1 is unset/empty:
#   zsh: $dir = "."  (current dir, likely exists)
#   zshrs: $dir = "/" (root, mkdir / fails)
```

**Workaround** — explicit empty-check:
```sh
[[ -z "$file" ]] && dir=. || dir="${file:h}"
```

---

## #135 — `*(om)` glob qualifier mtime ordering broken (zsh: newest→oldest sorted)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ touch /tmp/zgs/old; sleep 0.5; touch /tmp/zgs/new; sleep 0.5; touch /tmp/zgs/newest

$ /opt/homebrew/bin/zsh -fc 'print -l /tmp/zgs/*(om)'
/tmp/zgs/newest
/tmp/zgs/new
/tmp/zgs/old

$ ./target/debug/zshrs --zsh -c 'print -l /tmp/zgs/*(om)' 2>/dev/null
/tmp/zgs/newest
/tmp/zgs/old
/tmp/zgs/new
```

`(om)` glob qualifier sorts results by modification time, newest
first. zsh produces `newest, new, old` (correct descending mtime
order). zshrs returns `newest, old, new` — first entry correct
but rest unordered.

The reverse direction `(Om)` (oldest-first) works in zshrs but
contaminated by FS-watcher leak (#70) on subsequent reads.

**Where** — `src/ported/glob.rs::sort_by_mtime`: the sort
comparator returns inconsistent ordering — possibly partial
sort or stable-sort with mismatched comparator semantics.
C-source `Src/glob.c::sorter` uses qsort with strict ordering.

**Impact** — every "log rotation"/"newest-N files" idiom breaks:

```sh
# show 5 most-recent log files
for f in /var/log/*(om[1,5]); do
    echo "$f"
done
# zsh: 5 newest in mtime order
# zshrs: 1st newest + 4 in random order
```

Related to bug #36 (glob ordering DFS vs depth-first) — glob-result
ordering has multiple distinct issues.

**Workaround** — pipe through external `sort` with `ls -t` for
mtime-ordered results:
```sh
for f in $(ls -t /var/log/ | head -5); do
    echo "/var/log/$f"
done
```

---

## #136 — `%E` prompt escape (clear-to-EOL) not expanded; returns literal `%E`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'print -P "%E"' | od -c
\033 [ K \n

$ ./target/debug/zshrs --zsh -c 'print -P "%E"' | od -c
% E \n
```

`%E` should emit the terminal "erase to end of line" escape
sequence (`\033[K` aka `tcout(TCLR_LINE)`). zsh emits the
expected 3-byte sequence. zshrs returns literal `%E`.

Other prompt escapes that work in both: `%M` (host), `%.`
(basename), `%K{color}` (background), `%F{color}` (foreground).
`%E` is specifically missing.

Family with the prompt-escape coverage gap series:
- #38 (`%m`/`%C`/`%i`/`%l`/`%y`/`%E`/`%v`/etc. missing)
- #92 (PS4 default empty)
- #96 (`%N/`/`%N~` truncation broken)
- #111 (`%y` tty escape)
- #131 (`%(N~.A.B)` conditional broken)
- #136 (this — `%E` clear-EOL)

The prompt-escape implementation has at least 6 distinct gaps.

**Where** — `src/ported/prompt.rs::handle_escape`: missing the
`'E'` arm in the escape-dispatch switch. C-source
`Src/prompt.c::putprompt` case `'E'` calls
`tcout(TCCLEAREOL)`.

**Impact** — prompts that use `%E` for line-end clearing leak
trailing characters when redrawn:

```sh
PROMPT='%~ %E$ '
# zsh: ANSI clear-EOL emitted, redraws clean
# zshrs: literal "%E" appears in prompt, no clearing
```

**Workaround** — manual termcap call:
```sh
PROMPT='%~ '$'\e[K''$ '
```

---

## #137 — `(( "str" == "str" ))` returns false (string-equality coercion in arith)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '(( "abc" == "abc" )); echo "$?"'
0

$ ./target/debug/zshrs --zsh -c '(( "abc" == "abc" )); echo "$?"'
1
```

In arith context `(( ))`, zsh coerces non-numeric strings to 0,
then evaluates `0 == 0` as true → `(( ))` returns exit 0
(success because expression is non-zero/truthy in arith sense).

zshrs treats the strings differently — either preserves them as
opaque values where equality fails, or coerces but then the
boolean conversion is wrong. Result: exit 1 (failure).

```sh
# Both shells agree on integers:
$ both-shells -fc '(( 5 == 5 )); echo $?'
0
$ both-shells -fc '(( 5 == 6 )); echo $?'
1

# Diverge on quoted strings:
$ /opt/homebrew/bin/zsh -fc '(( "abc" == "abd" )); echo $?'
0      # coerces both to 0, 0==0 → true → success
$ ./target/debug/zshrs --zsh -c '(( "abc" == "abd" )); echo $?'
1
```

Related to bug #118 (`(( y = x ))` doesn't coerce to int) and
#132 (`(( "5" + "3" ))` doesn't coerce quoted nums). Same root
gap: arith parser doesn't strip quotes and run numeric-coerce
on string operands.

**Where** — `src/ported/math.rs::eval_string_operand`: strings
in arith context return string-type values; equality on them
isn't the integer-equality semantic. C-source `Src/math.c::
mathevall` coerces all values to `mnumber` (numeric type) at
operand-read time.

**Impact** — defensive arith-based string comparison broken:

```sh
status_str="OK"
(( "$status_str" == "OK" )) && echo "good"
# zsh: prints "good"  (both coerce to 0, 0==0 true, but the
#                       comparison gives nonsense meaning anyway)
# zshrs: prints nothing
```

(The zsh behavior is itself questionable — comparing strings in
arith context shouldn't generally be meaningful — but the
divergence is real.)

**Workaround** — use proper string-comparison test:
```sh
[[ "$status_str" == "OK" ]] && echo "good"
```

---

## #138 — `%i` prompt escape returns `0` instead of current input line

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'print -P "%i"'
1

$ ./target/debug/zshrs --zsh -c 'print -P "%i"'
0
```

`%i` prompt escape = "current line number of input". When this
is the first/only line of a `-c` invocation, the value should
be `1`. zshrs returns `0`.

Per `man zshmisc` § SIMPLE PROMPT ESCAPES:
> `%i` — The line number currently being executed in the script
> source or function.

Inside a function, `%i` would track the local line. At top level
of a `-c` invocation, the line is conceptually `1`.

**Where** — `src/ported/prompt.rs::expand_line_num`: returns
zero-indexed line counter. C-source `Src/exec.c` tracks
`current_lineno` starting from 1.

**Impact** — tracing-format `$PS4` and prompt diagnostics that
include `%i` report off-by-one line numbers:

```sh
PS4='%N:%i> '
set -x
foo() { echo hi; }
foo
# zsh: "foo.zsh:1> echo hi" or similar
# zshrs: "foo.zsh:0> echo hi"  (line 0 doesn't exist)
```

Family with the prompt-escape gaps (#38, #92, #96, #111, #131,
#136) — zshrs's prompt-expansion has systematic issues.

**Workaround** — `$LINENO` parameter (per-line update) instead
of `%i` escape:
```sh
PS4='+$LINENO> '
```

---

## #139 — Sourced-file error location reports `zsh:1:` instead of `/sourced/file:N`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ echo 'definitely_not_a_cmd_xyz' > /tmp/zsl
$ /opt/homebrew/bin/zsh -fc 'source /tmp/zsl' 2>&1
/tmp/zsl:1: command not found: definitely_not_a_cmd_xyz

$ ./target/debug/zshrs --zsh -c 'source /tmp/zsl' 2>&1
zsh:1: command not found: definitely_not_a_cmd_xyz
```

When an error occurs inside a sourced file, zsh prepends the
**sourced file's path and line number** (`/tmp/zsl:1:`) to the
error message. zshrs always uses the literal `zsh:1:` regardless
of which file is being sourced.

For `/etc/hosts`-as-source (gibberish input):
```sh
$ /opt/homebrew/bin/zsh -fc '. /etc/hosts' 2>&1 | head -2
/etc/hosts:4: command not found: 127.0.0.1
/etc/hosts:5: command not found: 127.0.0.1

$ ./target/debug/zshrs --zsh -c '. /etc/hosts' 2>&1 | head -2
zsh:1: command not found: 127.0.0.1
zsh:1: command not found: 127.0.0.1
```

zshrs reports every error from a sourced file as `zsh:1:`,
losing both filename context and line number.

**Where** — `src/ported/builtin_source.rs::source_file`: doesn't
push the sourced filename and per-line counter into the error-
formatter context. C-source `Src/builtin.c::bin_dot` updates
`scriptname` and `lineno` for the duration of the source call.

**Impact** — debugging sourced libraries is much harder. Stack
traces from `.zshrc` errors show `zsh:1:` for every error
regardless of which plugin file caused it.

Plus: tools that parse shell error output by file:line (linters,
IDE plugins) can't locate the actual error site.

**Workaround** — none from user side; errors from sourced files
must be diagnosed manually by reading the source.

---

## #140 — `exec /not/found` uses generic "not found" with wrong program prefix

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'exec /no/such/cmd 2>&1'
zsh:1: no such file or directory: /no/such/cmd

$ ./target/debug/zshrs --zsh -c 'exec /no/such/cmd 2>&1'
zshrs: exec: /no/such/cmd: not found
```

zsh's error format: `zsh:LINE: ERRNO_MSG: PATH` — specifically
"no such file or directory" (the ENOENT errno string).

zshrs's format issues:
1. **Wrong program prefix**: `zshrs:` instead of `zsh:` — zshrs
   doesn't mirror the C-zsh convention that even the zshrs port
   should use the `zsh:` prefix for compat.
2. **No line number**: `zshrs: exec: ...` without `:LINE:`.
3. **Wrong error string**: "not found" instead of the
   errno-specific "no such file or directory".

Combined with #112 (Rust `io::Error` "(os error N)" leak), error
formatting has multiple distinct gaps from zsh's canonical
format.

**Where** — `src/ported/builtin_exec.rs::dispatch_target`: error
emitted via `eprintln!("zshrs: exec: {}: not found", target)`
hardcodes both the prefix and the generic message. Should use
`format_zsh_error(line, errno_str, path)` matching the canonical
`zsh:LINE: NAMEFROM_ERRNO: TARGET` pattern.

**Impact** — error-parsing tools (CI failure parsers, linters,
script wrappers) that grep for `zsh:` prefix or specific errno
strings can't match zshrs output. Cross-shell-compatible tooling
needs special-case for zshrs.

**Workaround** — wrap exec calls with explicit existence check:
```sh
[[ -x /no/such/cmd ]] || { echo "zsh: exec: no such file: /no/such/cmd" >&2; exit 1; }
exec /no/such/cmd
```

---

## #141 — `;;` outside case context not a parse error in zshrs

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo a;; echo b' 2>&1
zsh:1: parse error near `;;'

$ ./target/debug/zshrs --zsh -c 'echo a;; echo b' 2>&1
a
```

`;;` is the case-pattern terminator inside `case ... esac`
constructs. Outside of `case`, encountering `;;` should be a
parse error. zsh errors with "parse error near `;;`". zshrs
silently parses the first `echo a`, discards the rest, and
returns success.

```sh
$ ./target/debug/zshrs --zsh -c 'echo a;; echo b; echo c' 2>&1
a
```

Only `echo a` runs; `echo b` and `echo c` are silently dropped.

**Where** — `src/ported/parse.rs::parse_command`: doesn't
reject `;;` token outside of `case` context. C-source
`Src/parse.c::par_cmd` errors on `DSEMI` outside `case`.

**Impact** — typos that include accidental `;;` (e.g., from
copy-paste of case-arm code into a regular block) silently
truncate the script:

```sh
# user accidentally pasted `;; ` from case-arm
process_input;;
finalize
# zsh: parse error (caught immediately)
# zshrs: runs process_input, silently skips finalize
```

Real data-loss potential. zsh's strict parsing catches this;
zshrs's permissive parsing hides the typo.

**Workaround** — none portable; rely on careful review of
scripts before running under zshrs.

---

## #142 — Orphan terminator parse error uses generic "orphan terminator" + double-print

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'esac 2>&1'
zsh:1: parse error near `esac'

$ ./target/debug/zshrs --zsh -c 'esac 2>&1' 2>&1
zsh:1: parse error near orphan terminator
zshrs: parse error
```

When a control-flow terminator (`esac`, `fi`, `done`) appears
outside its matching block, zsh emits a single error naming the
specific token: `parse error near 'esac'`. zshrs emits:
1. Generic descriptor "orphan terminator" instead of naming the
   actual token.
2. A second redundant `zshrs: parse error` line.

Same for `fi` and `done`:
```sh
$ /opt/homebrew/bin/zsh -fc 'fi 2>&1'
zsh:1: parse error near `fi'

$ ./target/debug/zshrs --zsh -c 'fi 2>&1' 2>&1
zsh:1: parse error near orphan terminator
zshrs: parse error
```

**Where** — `src/ported/parse.rs::handle_orphan_terminator`:
emits a hardcoded "orphan terminator" string instead of the
specific keyword. Also wraps the error in two layers of error
reporting (parser-level + outer "zshrs: parse error" wrapper).
C-source `Src/parse.c::par_event` reports the exact token from
the offending position.

**Impact** — error-parsing tools (linters, editor-integrated
checkers) that extract token-specific error positions can't
identify the actual unmatched terminator. CI pipelines that grep
for the offending keyword fail.

The double-print also confuses log-parsing tools expecting one
error message per parse failure.

**Workaround** — none portable.

---

## #143 — `$TRY_BLOCK_ERROR` initial value is `0` in zshrs (zsh: `-1`)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[$TRY_BLOCK_ERROR]"'
[-1]

$ ./target/debug/zshrs --zsh -c 'echo "[$TRY_BLOCK_ERROR]"'
[0]
```

`$TRY_BLOCK_ERROR` is the integer parameter set by the `{ try }
always { handler }` construct to indicate whether an error
occurred in the try block:
- `-1` = no exception (initial/normal state)
- `0` = exception cleared
- `>0` = exception number (errno-like)

zsh initializes it to `-1` (no exception). zshrs initializes
to `0`, which conflates "uninitialized" with "exception just
cleared".

Inside `always` block (no error in try):
```sh
$ /opt/homebrew/bin/zsh -fc '{ true } always { echo "[$TRY_BLOCK_ERROR]"; }'
[0]

$ ./target/debug/zshrs --zsh -c '{ true } always { echo "[$TRY_BLOCK_ERROR]"; }'
[0]
```

So the `always` block value is the same; only the initial value
differs. Code that checks `(( TRY_BLOCK_ERROR == -1 ))` to
distinguish "never been in try block" from "in always block,
no error" gives wrong results under zshrs.

Per `man zshparam`:
> `TRY_BLOCK_ERROR` — In a `{ try } always { handler }`
> construct, the value of this parameter is set to:
> `-1` before the try block runs (no exception in flight),
> `0` in the always block if no exception occurred,
> `n>0` in the always block if an exception with code `n` was
> raised.

**Where** — `src/ported/init.rs::init_special_params`: sets
`TRY_BLOCK_ERROR` to 0 at startup. Should set to -1 per zsh
convention. C-source `Src/init.c::setupvals` creates with
initial `-1`.

**Impact** — exception-aware scripts using `(( TRY_BLOCK_ERROR
== -1 ))` to distinguish initial state vs cleared-after-try
get false positives:

```sh
if (( TRY_BLOCK_ERROR == -1 )); then
    # zsh: only true before any try block
    # zshrs: never true (always 0 or positive)
    setup_initial_state
fi
```

**Workaround** — track try-block state via explicit flag instead
of relying on `TRY_BLOCK_ERROR`'s initial value.

---

## #144 — `${(q)str}` with embedded newline uses `\<newline>` form instead of `$'\n'`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'x=$'\''a\nb'\''; echo "[${(q)x}]"'
[a$'
'b]                  # uses $'...' form to encode the newline

$ ./target/debug/zshrs --zsh -c 'x=$'\''a\nb'\''; echo "[${(q)x}]"'
[a\
b]                   # uses backslash-newline (line continuation)
```

The `(q)` quote flag produces a shell-safe quoted form of the
string. For a string containing a literal newline byte:
- zsh: uses `$'\n'` ANSI-C escape syntax, producing `a$'\n'b`
- zshrs: uses `\<actual-newline>` (backslash line-continuation)

Both forms parse back to the same string when fed through `eval`,
but they LOOK different and have different downstream behaviors:
- `$'\n'` form: stays as 4 bytes (`$`, `'`, `\n`, `'`) in the
  output — easily greppable, fits on one line.
- `\<newline>` form: emits 2 bytes (`\`, actual newline) —
  the line-break IS in the output, breaks line-oriented tools.

**Where** — `src/ported/paramsubst.rs::quote_string`: for
newline character, emits `\\\n` instead of `$'\\n'`. C-source
`Src/utils.c::quotestring` uses `$'...'` form for any non-printable
byte.

**Impact** — output of `${(q)x}` containing a newline:
- Breaks tools that count quoted args by line.
- Can't be reused as a single-line literal in zsh scripts
  (the linebreak is real, requires multi-line context).
- `eval "$(echo "${(q)x}")"` still works in both shells because
  the parser accepts both forms, but the intermediate string
  differs.

```sh
config="$(read_value)"
# encode for storage in a single-line config file:
echo "key=${(q)config}" >> /etc/myapp.conf
# zsh: produces "key=a$'\n'b" — one line, parseable
# zshrs: produces "key=a\<newline>b" — TWO lines, breaks config parser
```

**Workaround** — use `(qq)` (double-quote form) which doesn't
have this issue:
```sh
echo "key=${(qq)config}" >> /etc/myapp.conf
# both shells: produces "key="a<newline>b"" — explicit dquote
```

---

## #145 — `${(k)h[name]}` key-existence query errors "bad substitution"

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h=(red 1 blue 2); echo "[${(k)h[red]}]"; echo "[${(k)h[nope]}]"'
[red]
[]

$ ./target/debug/zshrs --zsh -c 'typeset -A h=(red 1 blue 2); echo "[${(k)h[red]}]"' 2>&1
zsh:1: bad substitution
```

`${(k)h[name]}` is the documented zsh form for "if `name` is a
key in assoc array `h`, return the key; otherwise return empty".
This is a common test idiom for "does key exist" without
ambiguity around empty-value keys.

zshrs's `(k)` flag parser doesn't handle the `[name]` subscript
form — only `${(k)h[@]}` (all-keys) and `${(@k)h}` (all-keys
array) work.

Per `man zshparam`:
> `(k)` — When this flag is followed by a subscript, the keys
> matching the subscript are returned instead of the values.

**Where** — `src/ported/paramsubst.rs::parse_k_flag`: doesn't
accept named-subscript after `(k)` flag. C-source
`Src/subst.c::dosubst` handles `[name]` as a key-existence
lookup in the `PM_HASHED|(k)` code path.

**Impact** — assoc-array existence-test idioms break:

```sh
typeset -A user_perms=(alice read bob write)
for u in alice bob charlie; do
    if [[ -n "${(k)user_perms[$u]}" ]]; then
        echo "$u has perms"
    fi
done
# zsh: prints "alice has perms", "bob has perms"
# zshrs: bad substitution error
```

**Workaround** — use `${+h[name]}` (parameter-defined test) or
`(( ${+h[name]} ))`:
```sh
if (( ${+user_perms[$u]} )); then
    echo "$u has perms"
fi
```

---

## #146 — `{ cmd; } arg arg` compound-with-trailing-args silently accepted

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '{ echo a; } b c' 2>&1
zsh:1: parse error near `b'

$ ./target/debug/zshrs --zsh -c '{ echo a; } b c' 2>&1
a
zshrs: command not found: b
```

zsh rejects `{ cmd; } arg arg` syntax at parse time (compound
groups can't take trailing args). zshrs splits it into two
commands: runs `{ echo a; }` (printing `a`) then runs `b c` as
a separate command (which fails with "command not found").

The split behavior masks the syntax error — the user might not
notice the parser is treating their code differently than
intended.

Per zsh grammar, `{ ... }` is a `sublist_terminator` and
shouldn't be followed by additional words on the same logical
line. zsh strictly enforces this.

**Where** — `src/ported/parse.rs::parse_compound`: doesn't
require newline/`;`/`&` after `}` before next command starts.
Treats whitespace after `}` as command separator. C-source
`Src/parse.c::par_cmd` requires explicit terminator.

**Impact** — typos that accidentally place tokens after a
compound group don't get caught:

```sh
# user typo: forgot to wrap "echo b" in the braces
{ echo a; } echo b
# zsh: parse error (caught immediately)
# zshrs: runs `echo a`, then fails to find command `echo b`
#   — partial execution + unclear error
```

Similar to bug #141 (`;;` outside case silently accepted) — permissive
parser hides programming errors.

**Workaround** — none — be careful with brace syntax.

---

## #147 — `${(@)arr:mod}` modifier dropped when applied with `(@)` flag

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(/a/b.txt /c/d.log); echo "${(@)a:t}"'
b.txt d.log

$ ./target/debug/zshrs --zsh -c 'a=(/a/b.txt /c/d.log); echo "${(@)a:t}"'
/a/b.txt /c/d.log
```

`${(@)arr:t}` should apply the `:t` (tail) modifier to each
element of the array after `@` array-context flag. zsh applies
the modifier to each element → `b.txt d.log`. zshrs ignores
the modifier entirely → full paths returned.

`${arr[@]:t}` (subscript-style array context) works in both:
```sh
$ both-shells -fc 'a=(/a/b.txt /c/d.log); echo "${a[@]:t}"'
b.txt d.log
```

So the bug is the **flag-style `(@)`** form when combined with
a trailing modifier. Same family as #91 (modifier dropped after
`(j)`), #82, #83, #108 — flag+modifier combination consistently
broken.

**Where** — `src/ported/paramsubst.rs::parse_flag_then_modifier`:
when the expansion has both a leading flag (like `(@)`, `(j)`,
`(s)`) and a trailing `:modifier`, the modifier parse path
isn't reached. C-source `Src/subst.c::modify` dispatches
modifiers regardless of preceding flag.

**Impact** — array transforms break in the flag-first form:

```sh
paths=(/var/log/a.log /var/log/b.log /var/log/c.log)
echo "${(@)paths:t}"
# zsh: a.log b.log c.log
# zshrs: /var/log/a.log /var/log/b.log /var/log/c.log
```

**Workaround** — use subscript form `${arr[@]:mod}`:
```sh
echo "${paths[@]:t}"   # both shells: tailed
```

---

## #148 — `zsh/mathfunc` missing many functions (cbrt, asinh, erfc, gamma, j0, rand48, ...)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'zmodload zsh/mathfunc; echo $((asinh(1)))'
0.88137358701954305

$ ./target/debug/zshrs --zsh -c 'zmodload zsh/mathfunc; echo $((asinh(1)))' 2>&1
(empty — function not found)
```

zsh/mathfunc provides a standard set of `libm` math functions for
arithmetic context. zshrs's module is missing many of them:

Missing in zshrs (present in zsh):
- `cbrt(x)` — cube root
- `asinh(x)`, `acosh(x)`, `atanh(x)` — inverse hyperbolic
- `expm1(x)` — exp(x) - 1
- `log1p(x)` — log(1+x)
- `erf(x)`, `erfc(x)` — error functions
- `gamma(x)`, `lgamma(x)` — gamma / log-gamma
- `j0(x)`, `j1(x)`, `y0(x)`, `y1(x)` — Bessel functions
- `rand48()` — random double in [0,1)

Present in both: sin, cos, tan, asin, atan, sqrt, exp, log,
log10, floor, ceil, int, abs, sinh, cosh, tanh.

Per `man zshmodules` § The zsh/mathfunc Module:
> Loads the math functions: `abs`, `acos`, `acosh`, `asin`,
> `asinh`, `atan`, `atanh`, `cbrt`, `ceil`, `cos`, `cosh`,
> `erf`, `erfc`, `exp`, `expm1`, `fabs`, `floor`, `gamma`, `j0`,
> `j1`, `lgamma`, `log`, `log10`, `log1p`, `logb`, `sin`, `sinh`,
> `sqrt`, `tan`, `tanh`, `y0`, `y1`. Two-argument functions:
> `atan2`, `copysign`, `fmod`, `hypot`, etc.

**Where** — `src/ported/mathfunc.rs`: only registers a subset of
the C-library `libm` functions. C-source
`Src/Modules/mathfunc.c::math_funcs` table lists ~30 functions.

**Impact** — scientific scripts using zsh's mathfunc fail
silently or produce 0:

```sh
zmodload zsh/mathfunc
(( pi = 4 * atan(1) ))   # works (atan present)
(( e = exp(1) ))          # works (exp present)
(( bessel = j0(1) ))      # zsh: 0.7651...   zshrs: 0 or error
```

**Workaround** — external `bc`/`python` for missing functions:
```sh
pi=$(echo 'scale=10; 4*a(1)' | bc -l)
```

---

## #149 — `${(q)str}` with tab uses `\<tab>` (extending #144 to all control chars)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'x=$'\''a\tb'\''; echo "[${(q)x}]"'
[a$'	'b]              # uses $'\t' encoded form

$ ./target/debug/zshrs --zsh -c 'x=$'\''a\tb'\''; echo "[${(q)x}]"'
[a\	b]               # uses literal \<tab>
```

Extends bug #144 (newline case) to ALL non-printable characters.
The `(q)` quote flag should encode tab/CR/null/etc. using `$'\X'`
ANSI-C escape syntax. zsh does this for newline, tab, and other
control bytes. zshrs uses backslash-literal form.

```sh
$ /opt/homebrew/bin/zsh -fc 'x=$'\''line\ttab\nnl'\''; echo "${(q)x}"'
line$'	'tab$'
'nl                    # $'\t' and $'\n' both encoded

$ ./target/debug/zshrs --zsh -c 'x=$'\''line\ttab\nnl'\''; echo "${(q)x}"'
line\	tab\
nl                     # literal backslash + char
```

Both forms parse back equivalently, but the visual representation
and downstream parseability differ.

**Where** — `src/ported/paramsubst.rs::quote_string`: maps all
control chars to `\X` instead of `$'\X'`. C-source
`Src/utils.c::quotedzputs` distinguishes printable/non-printable
and uses ANSI-C encoding for the latter.

**Impact** — same as #144 — single-line config-file generation
via `key=${(q)val}` breaks when value contains tabs.

**Workaround** — use `(qq)` double-quote form (works for all
control chars in both shells).

---

## #150 — `$OPTERR` initialized to `1` in zshrs (zsh: empty/unset)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[$OPTERR]"'
[]

$ ./target/debug/zshrs --zsh -c 'echo "[$OPTERR]"'
[1]
```

`$OPTERR` is the getopts error-reporting flag. Per POSIX/bash
convention, `1` = print errors (default), `0` = silent. zsh
LEAVES IT UNSET until `getopts` is called for the first time
(then auto-sets to `1` per POSIX).

zshrs pre-initializes `OPTERR=1` at shell startup before any
`getopts` is called. Causes:
1. `${OPTERR-default}` fallback never fires (parameter is set).
2. `[[ -v OPTERR ]]` test reports `set` from the start.
3. Code that intentionally checks "was getopts ever called?" via
   `OPTERR` presence breaks.

Same family as #69 (sysparams auto-loaded), #64 (PIPESTATUS),
#65 (EPOCHSECONDS pre-populated) — zshrs has multiple eager-
initialization gaps that violate the "param appears only when
used" convention.

**Where** — `src/ported/init.rs::init_getopts_state`: sets
`OPTERR=1` in the global env. Should defer to first `getopts`
call. C-source `Src/builtin.c::bin_getopts` lazy-initializes
on first invocation.

**Impact** — getopts-detection code fails:

```sh
# pre-check: did any earlier code already use getopts?
if [[ -v OPTERR ]]; then
    echo "getopts already ran somewhere"
fi
# zsh: only true if getopts has been called
# zshrs: always true (even at fresh shell start)
```

**Workaround** — `(( OPTIND > 1 ))` to detect prior getopts use,
or pair-check both:
```sh
[[ -v OPTERR && OPTIND -gt 1 ]] && echo "getopts ran"
```

---

## #151 — `${(@qq)arr}` only quotes the first array element

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=("hello" "world"); echo "${(@qq)a}"'
'hello' 'world'

$ ./target/debug/zshrs --zsh -c 'a=("hello" "world"); echo "${(@qq)a}"'
'hello' world
```

`${(@qq)arr}` is "preserve as array (`@`), quote each element
with single-quote form (`qq`)". zsh quotes each element. zshrs
quotes only the FIRST element; subsequent elements are output
without quotes.

Iterating to confirm per-element form:
```sh
$ /opt/homebrew/bin/zsh -fc 'a=("hello" "world"); for x in "${(@qq)a}"; do echo "[$x]"; done'
['hello']
['world']

$ ./target/debug/zshrs --zsh -c 'a=("hello" "world"); for x in "${(@qq)a}"; do echo "[$x]"; done'
['hello']
[world]      ← second element unquoted
```

So the `(qq)` flag is applied to element 1 but skipped for
element 2+.

**Where** — `src/ported/paramsubst.rs::apply_quote_flag_per_elem`:
the per-element quote loop only processes the first element.
Likely an early-break or missing iteration. C-source
`Src/subst.c::quotedzputs` is called for each element.

**Impact** — serializing array values for re-input via
`${(@qq)arr}` produces malformed output:

```sh
hosts=("server-1" "server-2" "server-3")
echo "valid_hosts=( ${(@qq)hosts} )" > /etc/myapp.conf
# zsh: writes 'valid_hosts=( '\''server-1'\'' '\''server-2'\'' '\''server-3'\'' )'
# zshrs: writes 'valid_hosts=( '\''server-1'\'' server-2 server-3 )'  (parse error on re-load)
```

**Workaround** — explicit loop:
```sh
quoted_parts=()
for h in "${hosts[@]}"; do
    quoted_parts+=("${(qq)h}")
done
echo "valid_hosts=( ${quoted_parts[*]} )" > /etc/myapp.conf
```

---

## #152 — `${(qq)arr}` (no @) per-element-quotes when zsh joins-then-quotes

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=("hello" "world"); echo "${(qq)a}"'
'hello world'

$ ./target/debug/zshrs --zsh -c 'a=("hello" "world"); echo "${(qq)a}"'
'hello' 'world'
```

`${(qq)arr}` (no `@` flag) — in zsh, the array is first joined
with space into a scalar `"hello world"`, then quoted as a
single token → `'hello world'`. zshrs quotes each element
separately → `'hello' 'world'`.

Compare with `(@qq)` (which is supposed to be per-element):
```sh
$ /opt/homebrew/bin/zsh -fc 'a=("hello" "world"); echo "${(@qq)a}"'
'hello' 'world'              # per-element when @ is present
```

So in zsh:
- `(qq)` alone: join-then-quote
- `(@qq)`: quote-each-element

In zshrs both produce element-by-element output (with the
`@-flag` form being buggy per #151).

Inverse of bug #82 family — there, scalar context wasn't applied
inside `"..."`; here, scalar context isn't applied for `(qq)`
without `@`.

**Where** — `src/ported/paramsubst.rs::apply_quote_flag`:
default-array-no-@-flag should collapse to scalar before
applying quote. C-source `Src/subst.c::dosubst` chooses
behavior based on whether `(@)` is in the flag list.

**Impact** — config-file serialization that depends on the
single-string form gets array-form output:

```sh
parts=("a b" "c d")
echo "config=${(qq)parts}"
# zsh: config='a b c d'
# zshrs: config='a b' 'c d'  (different shape, may not parse)
```

**Workaround** — explicit scalar join:
```sh
echo "config=${(qq)${(j: :)parts}}"
```

---

## #153 — `${#${(z)s}}` returns wrong count (5 vs 4 for 4-word input)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 's="hello world foo bar"; echo "${#${(z)s}}"'
4

$ ./target/debug/zshrs --zsh -c 's="hello world foo bar"; echo "${#${(z)s}}"'
5
```

`${(z)s}` shell-token-splits "hello world foo bar" into 4 words.
`${#...}` should return the count: 4. zsh does this. zshrs
returns 5 (off-by-one or counting an extra phantom token).

Iterating the split explicitly shows both shells produce 4
elements:
```sh
$ both-shells -fc 's="hello world foo bar"; words=("${(@z)s}"); echo "${#words}"'
4    # both shells agree
```

So the bug is specifically the inline-nested `${#${(z)s}}` form
— intermediate array isn't materialized correctly for `${#...}`
count.

Same family as #63 (nested `${(j:s:)${(s:t:)var}}` returns first
element only), #82 (`(s)` flag in quoted context), #108
(`${arr/pat/X}` per-element).

**Where** — `src/ported/paramsubst.rs::nested_count`: when
`${#X}` wraps a `${(z)X}` expansion, the inner expansion's
result count includes a trailing empty token. C-source
`Src/subst.c::nrtokens` counts non-empty tokens.

**Impact** — defensive count-based loop bounds are off:

```sh
cmdline="ls -la /tmp /var"
tokens=("${(@z)cmdline}")
echo "$((${#${(z)cmdline}}))"   # one-liner count
# zsh: 4
# zshrs: 5  (try to access token 5 fails)
```

**Workaround** — assign to intermediate array first:
```sh
tokens=("${(@z)cmdline}")
echo "$((${#tokens}))"
```

---

## #154 — Readonly variable modifiable via `(( ))` / `let` arith ops

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'readonly x=5; (( x++ )); echo "x=$x"' 2>&1
zsh:1: read-only variable: x
x=5

$ ./target/debug/zshrs --zsh -c 'readonly x=5; (( x++ )); echo "x=$x"' 2>&1
x=6
```

Readonly enforcement is bypassed in arithmetic contexts. zsh
blocks `(( x++ ))`, `(( x += 5 ))`, `let "x = 10"` and similar
on readonly variables. zshrs silently allows the modification.

```sh
$ /opt/homebrew/bin/zsh -fc 'readonly y=10; let "y = 100"; echo "y=$y"' 2>&1
zsh:1: read-only variable: y
y=10

$ ./target/debug/zshrs --zsh -c 'readonly y=10; let "y = 100"; echo "y=$y"' 2>&1
y=100
```

Direct assignment `y=100` IS blocked correctly:
```sh
$ ./target/debug/zshrs --zsh -c 'readonly y=10; y=100' 2>&1
zsh:1: read-only variable: y
```

So the bug is specifically the arith-context paths (`(( ))`,
`let`, `(( var op ))`).

**Where** — `src/ported/math.rs::assign_result`: doesn't check
the `PM_READONLY` flag on the target variable before writing.
C-source `Src/math.c::matheval` calls `setiparam` which respects
the readonly bit.

**Impact** — security/invariant code that relies on readonly
to guarantee constants is silently bypassable:

```sh
readonly MAX_ATTEMPTS=3
for ((i = 0; i < MAX_ATTEMPTS; i++)); do
    if try_login; then
        (( MAX_ATTEMPTS = 0 ))   # malicious or buggy code
        # zsh: errors, MAX stays 3
        # zshrs: silently sets to 0, loop exits early
        break
    fi
done
```

**Workaround** — `if [[ "$var" != "$original" ]]; then` post-check.

---

## #155 — `${str[N,M+1]}` slice subscript doesn't evaluate variable/arith expressions

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 's=hello; n=2; echo "${s[1,n+1]}"'
hel

$ ./target/debug/zshrs --zsh -c 's=hello; n=2; echo "${s[1,n+1]}"'
hello
```

`${str[N,M]}` is the string-slice operator. zsh evaluates BOTH
N and M as arithmetic expressions, so `[1,n+1]` with `n=2`
becomes `[1,3]` and returns `hel`.

zshrs only evaluates literal numerics in subscript; variable
names and arith expressions are silently ignored, returning the
full string.

Confirmed all three forms diverge:
- `${s[1,3]}` (literal): both `hel` ✓
- `${s[1,n]}` (bare var): zsh `he`, zshrs `hello` ✗
- `${s[1,n+1]}` (arith): zsh `hel`, zshrs `hello` ✗

Per `man zshparam`:
> The subscript syntax for arrays and strings is `[exp]` or
> `[exp1,exp2]`, where each `exp` is an arithmetic expression.

**Where** — `src/ported/paramsubst.rs::parse_subscript`: only
accepts `[0-9]+` literals; doesn't fall through to arith
evaluator. C-source `Src/params.c::getarg` runs `mathevali`
on the subscript.

**Impact** — dynamic-bounds slicing breaks:

```sh
text="The quick brown fox"
end=${#text}
mid=$((end / 2))
echo "${text[1,mid]}"     # zsh: "The quick " (first half)
                           # zshrs: full text
```

**Workaround** — pre-compute the index value:
```sh
m=$((n + 1))
echo "${s[1,m]}"
```

---

## #156 — `[[ -e /path/*.glob ]]` glob-expands in test (zsh: literal match)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ mkdir -p /tmp/zgt; touch /tmp/zgt/a.txt
$ /opt/homebrew/bin/zsh -fc '[[ -e /tmp/zgt/*.txt ]] && echo y || echo n'
n

$ ./target/debug/zshrs --zsh -c '[[ -e /tmp/zgt/*.txt ]] && echo y || echo n'
y
```

Inside `[[ ... ]]` conditional, zsh treats the argument literally
(no glob expansion). The path `/tmp/zgt/*.txt` is checked for
existence literally — no file named `*.txt` exists → `-e`
returns false.

zshrs glob-expands the argument: `*.txt` matches `/tmp/zgt/a.txt`,
which exists → `-e` returns true.

This is consistent with bug #116 (`GLOB_SUBST` default on in
zshrs for pattern contexts) but also affects file-test contexts
where zsh doesn't expand at all.

Per zsh semantics, `[[ ]]` conditional arguments are NOT subject
to filename generation. zshrs ignores this.

**Where** — `src/ported/cond.rs::expand_arg`: applies glob
expansion to file-test operands. C-source `Src/cond.c::evalcond`
treats `[[ ]]` arguments as literal strings (no `globlist`
call).

**Impact** — `[[ -e $pattern ]]` checks where `$pattern` may
contain glob chars give wrong results. Common idiom of "exact
path test":

```sh
log="/var/log/*"   # literal asterisk in filename (rare but valid)
[[ -e "$log" ]] && echo "found"
# zsh: tests literal file "/var/log/*"
# zshrs: globs, matches anything in /var/log
```

Even with quoted `"$log"`, zshrs's glob-expansion behavior
differs from zsh.

**Workaround** — explicit `ls "$path" >/dev/null 2>&1` or
loop-over-glob with explicit pattern handling.

---

## #157 — `TRAP<SIG>()` function-named trap handlers not recognized

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'TRAPEXIT() { echo "EXIT-handler"; }; echo "main"'
main
EXIT-handler

$ ./target/debug/zshrs --zsh -c 'TRAPEXIT() { echo "EXIT-handler"; }; echo "main"'
main
```

zsh has a documented convention: functions named `TRAP<SIG>`
(where `<SIG>` is a signal name like `EXIT`, `INT`, `USR1`,
`DEBUG`, etc.) are automatically registered as trap handlers
for that signal. zshrs doesn't recognize this convention.

Same for signal traps:
```sh
$ /opt/homebrew/bin/zsh -fc 'TRAPUSR1() { echo got; }; kill -USR1 $$; sleep 0.05; echo done'
got
done

$ ./target/debug/zshrs --zsh -c 'TRAPUSR1() { echo got; }; kill -USR1 $$; sleep 0.05; echo done'
done            # signal sent but TRAPUSR1 not called
```

Per `man zshmisc` § TRAP FUNCTIONS:
> If a function with one of the trap-name forms (e.g., `TRAPINT`,
> `TRAPEXIT`, `TRAPZERR`, etc.) is defined, it is run when the
> corresponding signal/event occurs. Equivalent to `trap` builtin
> registration.

**Where** — `src/ported/builtin_typeset.rs::define_function`:
when registering a function, doesn't check if the name matches
the `TRAP<SIG>` pattern and register it as a signal handler.
C-source `Src/exec.c::execfuncdef` calls `trapprog_install` for
matching function names.

**Impact** — every signal-handling idiom using the function-name
form fails. This is the canonical zsh-idiomatic form (more
ergonomic than `trap 'cmd' SIG`):

```sh
TRAPEXIT() {
    rm -f /tmp/lockfile
}
# zsh: cleanup runs on shell exit
# zshrs: cleanup never runs
```

`TRAPZERR()` (run on any non-zero exit), `TRAPDEBUG()` (before
each command) — all broken.

**Workaround** — use explicit `trap` builtin:
```sh
trap 'rm -f /tmp/lockfile' EXIT
trap 'echo got' USR1
```

---

## #158 — Function-def redirect `f() { ... } < file` not honored at call time

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "test_line" > /tmp/zfd; f() { read line; echo "[$line]"; } < /tmp/zfd; f; rm /tmp/zfd'
[test_line]

$ ./target/debug/zshrs --zsh -c 'echo "test_line" > /tmp/zfd; f() { read line; echo "[$line]"; } < /tmp/zfd; f; rm /tmp/zfd'
(empty - read got no input)
```

zsh allows attaching redirects at function-definition time. When
the function is later called, those redirects are applied as
the default. `f() { read line; ...} < file` means "every call to
`f` uses `file` as stdin unless overridden".

zshrs parses the redirect but doesn't store it with the function
definition; it has no effect at call time.

Per zsh shell grammar:
```
fn_def = name "()" "{" body "}" [redirect_list]
```
where the redirect list is per-call default for the body.

**Where** — `src/ported/parse.rs::parse_function_def`: parses
the trailing redirects but doesn't attach them to the function's
exec context. C-source `Src/exec.c::execfuncdef` builds a
`Shfunc` with a `redir` chain that's applied each invocation.

**Impact** — library functions using attached redirects to
provide default input/output streams break:

```sh
log_with_timestamp() {
    while IFS= read -r line; do
        echo "$(date): $line"
    done
} < /var/log/messages
log_with_timestamp
# zsh: streams /var/log/messages through the function
# zshrs: log_with_timestamp reads from terminal stdin (or hangs)
```

**Workaround** — explicit redirect at call site:
```sh
log_with_timestamp() {
    while IFS= read -r line; do
        echo "$(date): $line"
    done
}
log_with_timestamp < /var/log/messages
```

---

## #159 — `while [[ $((i++)) -lt N ]]` only iterates once (post-increment in `[[ ]]` cond)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'i=0; while [[ $((i++)) -lt 3 ]]; do echo "iter: i=$i"; done'
iter: i=1
iter: i=2
iter: i=3

$ ./target/debug/zshrs --zsh -c 'i=0; while [[ $((i++)) -lt 3 ]]; do echo "iter: i=$i"; done'
iter: i=2
```

zsh evaluates the post-increment `$((i++))` correctly per
iteration:
- Iter 1: `i` was 0, post-inc → arith returns 0, compare `0<3` true,
  body sees `i=1`.
- Iter 2: `i` was 1, returns 1, `1<3` true, body sees `i=2`.
- Iter 3: `i` was 2, returns 2, `2<3` true, body sees `i=3`.
- Iter 4: `i` was 3, returns 3, `3<3` false, exit.

zshrs runs only once. The `[[ $((i++)) -lt 3 ]]` condition is
evaluated wrong — possibly the cmdsub/arith inside `[[ ]]` is
cached or i++ runs multiple times in one iteration.

The arith-form works correctly:
```sh
$ both-shells -fc 'i=0; while (( i++ < 3 )); do echo "iter: i=$i"; done'
iter: i=1
iter: i=2
iter: i=3
```

So the bug is the specific combination of `[[ ]]` test +
`$((i++))` arith expansion.

**Where** — `src/ported/cond.rs::eval_arith_in_test`: the arith
expansion is evaluated multiple times during a single iteration,
double-incrementing `i`. C-source `Src/cond.c::evalcond` runs
the arith once and caches the result for the comparison.

**Impact** — count-based loops using post-increment in `[[ ]]`
break silently:

```sh
i=0
while [[ $((i++)) -lt ${#array} ]]; do
    process "${array[i]}"
done
# zsh: iterates array length times
# zshrs: runs once with wrong i value
```

**Workaround** — use `(( ))` arith for the condition:
```sh
while (( i++ < ${#array} )); do ...
```
Or external counter:
```sh
for ((i=0; i < ${#array}; i++)); do ...
```

---

## #160 — `autoload -U +X funcname` doesn't actually load the function body

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'autoload -U +X compinit; type compinit'
compinit is a shell function from /opt/homebrew/Cellar/zsh/5.9/share/zsh/functions/compinit

$ ./target/debug/zshrs --zsh -c 'autoload -U +X compinit; type compinit'
compinit is an autoload shell function
```

`autoload -U +X` should **immediately load** the function from
its file in `$fpath`. zsh's `type` after `+X` shows the source
file path, confirming the function body has been loaded. zshrs
shows "autoload shell function" — meaning the function is still
in lazy/marker state, not actually loaded.

Different from bug #107 (`autoload -U +X` doesn't validate
existence). This is the next step: even when the file IS in
fpath, `+X` doesn't load the body.

Per `man zshbuiltins`:
> `-X` — Trigger function loading immediately without waiting
> for first invocation.

**Where** — `src/ported/builtin_autoload.rs::handle_X_flag`: the
`+X` immediate-load path marks the function as "to-be-loaded"
but doesn't read+parse+install the file contents. C-source
`Src/builtin.c::bin_autoload` calls `loadautofn` which evaluates
the file and registers the body.

**Impact** — completion-system setup that depends on `autoload
+X` pre-loading breaks:

```sh
autoload -Uz +X compinit
compinit -i    # zsh: pre-loaded body runs
              # zshrs: still in lazy state, runs the marker
```

**Workaround** — drop `+X`; let lazy-load trigger on first call:
```sh
autoload -Uz compinit
compinit -i    # both shells: lazy-loads then runs
```

---

## #161 — `case x in) ... ;; esac` (empty pattern) silently accepted

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'case x in) echo b;; esac; echo "after"' 2>&1
zsh:1: parse error near `)'

$ ./target/debug/zshrs --zsh -c 'case x in) echo b;; esac; echo "after"' 2>&1
after
```

The `case` construct requires a pattern between `in` and `)`.
zsh rejects `in)` (empty pattern) at parse time. zshrs silently
treats the empty arm as a no-op and continues to "after".

Family with permissive-parser bugs:
- #141 (`;;` outside case)
- #146 (`{ cmd; } arg` trailing args)
- #161 (this — empty case pattern)

**Where** — `src/ported/parse.rs::parse_case_arm`: doesn't
require at least one pattern token before `)`. C-source
`Src/parse.c::par_case` errors on empty arm-pattern.

**Impact** — typos with empty patterns silently match nothing:

```sh
# user typo: missing pattern between `in` and `)`
case $cmd in)
    echo "default"
    ;;
esac
# zsh: parse error (caught immediately)
# zshrs: falls through silently, default never prints
```

**Workaround** — careful syntax review before running.

---

## #162 — `${(l.5)x}` (missing close-delimiter) silently padded; zsh errors

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'x=42; echo "[${(l.5)x}]"' 2>&1
(empty - syntax error or unmatched delimiter)

$ ./target/debug/zshrs --zsh -c 'x=42; echo "[${(l.5)x}]"' 2>&1
[   42]
```

The pad-flag syntax is `(l.<expr>.<str1>.<str2>.)` — delimiters
must match on both sides AND closing delimiter must be present.

`(l.5)` lacks the closing `.` — should be parse error. zsh
rejects. zshrs accepts and applies pad with `5` as expression.

Properly-closed forms work in both:
```sh
$ both-shells -fc 'x=42; echo "[${(l.5..0.)x}]"'
[00042]
```

So the bug is the permissive parser accepting incomplete
flag-arg syntax.

**Where** — `src/ported/paramsubst.rs::parse_pad_flag_args`:
doesn't enforce balanced delimiters on `(l...)`/`(r...)` flag
contents. C-source `Src/subst.c::parsesubst` errors on missing
close-delimiter.

**Impact** — typos in pad-syntax silently produce different
results:

```sh
# user typo: missing close-delimiter, intending zero-pad
echo "${(l.10)num}"
# zsh: error (caught immediately)
# zshrs: applies space-pad (default), wrong format silently
```

Same family as #141, #146, #161 — permissive parser hides
malformed input.

**Workaround** — careful syntax review; always close the pad-flag
delimiter.

---

## #163 — `${(t)1}` positional parameter returns `scalar` instead of `array-special`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'set -- a b; echo "${(t)1}"'
array-special

$ ./target/debug/zshrs --zsh -c 'set -- a b; echo "${(t)1}"'
scalar
```

`$1`, `$2`, etc. are zsh's POSITIONAL parameters — they're
elements of the readonly special-array `$argv`. zsh's `(t)`
flag returns `array-special` to reflect this. zshrs returns
`scalar` (treating each positional as an independent scalar
var).

`${(t)@}` and `${(t)*}` (full positional array) are correct in
both shells (`array-readonly-special`). Only the indexed form
`${(t)N}` diverges.

Per `man zshparam`:
> `$argv` — Same as `$@` and `$*`. The positional parameters,
> indexed from 1.
> `$1, $2, ...` — Aliases for `${argv[1]}`, `${argv[2]}`, etc.

**Where** — `src/ported/paramsubst.rs::type_of_positional`:
returns `scalar` for `${(t)N}` when N is numeric. Should
return `array-special` because positional indices reference
elements of the `argv` special array. C-source
`Src/subst.c::paramtype` walks back through the parameter
descriptor and returns the parent type.

**Impact** — type-introspection scripts that need to identify
positional params vs regular scalars get wrong classification.
Rarely-used but documented zsh feature broken.

**Workaround** — check via `[[ "$#" -gt 0 ]]` or similar
positional-presence test instead of relying on `(t)` for
positional detection.

---

## #164 — Extended_glob `^pattern` (negation prefix) not recognized

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extended_glob; mkdir -p /tmp/zg; touch /tmp/zg/{a,b,c}; print -l /tmp/zg/^a'
/tmp/zg/b
/tmp/zg/c

$ ./target/debug/zshrs --zsh -c 'setopt extended_glob; mkdir -p /tmp/zg; touch /tmp/zg/{a,b,c}; print -l /tmp/zg/^a'
/tmp/zg/^a
```

The `^pattern` extended_glob form matches "anything NOT matching
pattern". zsh expands `^a` to "all files except `a`" → `b c`.
zshrs returns the literal `^a` pattern.

Family with the extended_glob coverage gap:
- #62 (`~` and-not operator)
- #81 (`~` exclusion duplicates)
- #89 (`#`/`##` quantifiers)
- #99 (`(#cN,M)` count flag)
- #117 (`(group)#` group quantifier)
- #164 (this — `^` negation prefix)

The whole extended_glob operator set has at least 6 distinct
gaps.

**Where** — `src/ported/pattern.rs::compile_extglob_neg`:
missing the `^` prefix handler. C-source `Src/pattern.c::patcompile`
recognizes `^` as `PAT_NOT` when `isset(EXTENDEDGLOB)`.

**Impact** — every script using `^` for negation fails to
filter properly:

```sh
setopt extended_glob
# delete all files except backup.log
rm /var/log/^backup.log
# zsh: deletes everything except backup.log
# zshrs: tries to remove literal "^backup.log" file
```

Data-loss adjacent (like #81).

**Workaround** — loop with `[[` continue:
```sh
for f in /var/log/*; do
    [[ "$f" == */backup.log ]] && continue
    rm "$f"
done
```

---

## #165 — `${$((expr))}` arith-result-as-name returns empty (zsh: returns expr value)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "${$((5 + 3))}"'
8

$ ./target/debug/zshrs --zsh -c 'echo "${$((5 + 3))}"'
(empty)
```

`${$((expr))}` is the form "evaluate arithmetic, treat result as
the value of the expansion". zsh evaluates `5 + 3 = 8` and
returns `"8"`. zshrs returns empty (the inner expansion fails
or returns nothing usable).

Per `man zshexpn` § PARAMETER EXPANSION:
> Inside the `${...}` form, any of the parameter expansion
> syntax can appear, including arithmetic expansion. The result
> is treated as if it were the value of the parameter.

So `${$((5 + 3))}` should produce the same as `echo $((5 + 3))`.

**Where** — `src/ported/paramsubst.rs::expand_nested_arith`:
when the inner expansion is an arith `$((...))`, the result
isn't propagated to the outer `${...}`. C-source
`Src/subst.c::dosubst` handles nested arith as a special case.

**Impact** — defensive arith-coerce patterns using
`${$((expr))}` to force numeric-then-treat-as-string fail:

```sh
# zsh-idiomatic: stringify an arith result for further use
result="${$((bytes * 8))}"
log "size in bits: $result"
# zsh: result = "8 * bytes_value"
# zshrs: result = empty
```

**Workaround** — direct arith expansion:
```sh
result="$((bytes * 8))"
```

---

## #166 — `for x in $@` (unquoted) keeps empty elements (zsh: removes)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'set -- a "" b; for x in $@; do printf "[%s]" "$x"; done; echo'
[a][b]

$ ./target/debug/zshrs --zsh -c 'set -- a "" b; for x in $@; do printf "[%s]" "$x"; done; echo'
[a][][b]
```

Per shell convention, unquoted `$@` undergoes word-splitting,
which removes empty elements. zsh removes the empty middle arg.
zshrs preserves it as an empty word.

The quoted form `"$@"` correctly preserves all elements in both:
```sh
$ both-shells -fc 'set -- a "" b; for x in "$@"; do printf "[%s]" "$x"; done; echo'
[a][][b]    # both keep the empty element
```

So the bug is specifically the unquoted `$@` IFS-split path
not stripping empties.

Per POSIX:
> Unquoted `$@`/`$*` undergoes field splitting via IFS. Empty
> fields resulting from such splitting are eliminated.

**Where** — `src/ported/paramsubst.rs::split_unquoted_positionals`:
emits each positional including empty strings as separate
tokens. C-source `Src/subst.c::wordsplit` skips zero-length
fields.

**Impact** — defensive iteration patterns that rely on
empty-skipping behavior process unwanted empty entries:

```sh
for arg in $@; do
    [[ -z "$arg" ]] && continue   # workaround
    process "$arg"
done
```

Cross-shell scripts get different iteration counts.

**Workaround** — explicit empty-check inside loop:
```sh
for arg in "$@"; do
    [[ -z "$arg" ]] && continue
    process "$arg"
done
```

---

## #167 — Unclosed `{ cmd` silently runs without error (zsh: parse error)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '{ echo a' 2>&1
zsh:1: parse error near `a'

$ ./target/debug/zshrs --zsh -c '{ echo a' 2>&1
a
```

A `{` opens a compound-command grouping that requires a
matching `}` close. Missing close should be parse error. zsh
errors. zshrs silently treats it as if the `{` weren't there
and runs the body.

Same permissive-parser family:
- #141 (`;;` outside case)
- #146 (`{ cmd; } arg` trailing args)
- #161 (`case x in)` empty pattern)
- #162 (`(l.5)` missing close delim)
- #167 (this — unclosed `{`)

**Where** — `src/ported/parse.rs::parse_brace_group`: doesn't
require matching `}` at end-of-input. C-source
`Src/parse.c::par_subsh` errors on unmatched bracket.

**Impact** — common typo (forgetting `}` on a multi-line block)
silently passes through and runs partial code:

```sh
# accidentally truncated function definition
foo() {
    cleanup
# forgot closing brace
# zsh: parse error (caught immediately)
# zshrs: cleanup runs as top-level command, function never defined
```

**Workaround** — none — careful syntax review.

---

## #168 — Extra `}` after command silently ignored (zsh: parse error)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo a }' 2>&1
zsh:1: parse error near `}'

$ ./target/debug/zshrs --zsh -c 'echo a }' 2>&1
a
```

An orphan `}` (no matching `{`) should be a parse error. zsh
errors. zshrs silently ignores the `}` and runs `echo a`.

Same permissive-parser family as #167 etc.

**Where** — `src/ported/parse.rs::handle_close_brace`: ignores
stray `}` tokens when not in brace-group context. Should error
like orphan terminators (cf. #142). C-source `Src/parse.c::par_event`
errors on `}` not closing a `{`.

**Impact** — copy-paste mistakes that leave stray `}` chars
silently pass:

```sh
# pasted code from another file, accidentally left trailing }
echo "main work" }
# zsh: parse error
# zshrs: runs as "echo main work" + ignores }
```

**Workaround** — careful review before running.

---

## #169 — `{...} always {...} always {...}` chained-always silently accepted (zsh: parse error)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '{ echo a } always { echo b } always { echo c }' 2>&1
zsh:1: parse error near `always'

$ ./target/debug/zshrs --zsh -c '{ echo a } always { echo b } always { echo c }' 2>&1
a
b
```

The `{ try } always { handler }` construct in zsh allows
exactly one `always` block. Chaining multiple `always` blocks
is a parse error. zshrs silently runs the first try-block + the
first always-block, then ignores the second `always`.

Same permissive-parser family:
- #141 (`;;` outside case)
- #146 (`{ cmd; } arg` trailing args)
- #161 (empty case pattern)
- #162 (unclosed pad-delim)
- #167 (unclosed `{`)
- #168 (extra `}`)
- #169 (this — chained `always`)

Even worse: `{ a } always { b } extra` (trailing tokens after
always) is also silently accepted in zshrs.

**Where** — `src/ported/parse.rs::parse_always_block`: parses
a single `always` block then returns, ignoring trailing tokens
without erroring. C-source `Src/parse.c::par_event` errors on
the second `always`.

**Impact** — typos in `always`-chained code silently produce
partial execution:

```sh
{ critical_section
} always { cleanup1
} always { cleanup2 }    # typo: meant to nest, not chain
# zsh: parse error caught immediately
# zshrs: cleanup1 runs, cleanup2 silently dropped
```

**Workaround** — careful syntax review.

---

## #170 — Unclosed `echo (abc` treated as literal arg (zsh: bad pattern error)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo (abc' 2>&1
zsh:1: bad pattern: (abc

$ ./target/debug/zshrs --zsh -c 'echo (abc' 2>&1
(abc
```

In zsh, `(...)` opens either a subshell or a pattern-grouping
context. An unclosed `(` is a parse/pattern error. zsh reports
"bad pattern: (abc". zshrs treats `(abc` as a literal string
argument to `echo` and prints it.

Same permissive-parser family.

**Where** — `src/ported/parse.rs::parse_paren`: when an
unclosed `(` is encountered, falls back to treating it as a
literal character instead of erroring. C-source
`Src/parse.c::par_subsh` / `Src/pattern.c::patcompile` errors.

**Impact** — common typo of missing close-paren passes silently:

```sh
echo "first" (forgot to close
echo "second"
# zsh: parse error caught immediately
# zshrs: echo runs both lines literally
```

**Workaround** — careful syntax review.

---

## #171 — Empty pipeline/and-or operands (`a | | b`, `a && && b`) silently accepted

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo a | | echo b' 2>&1
zsh:1: parse error near `|'

$ ./target/debug/zshrs --zsh -c 'echo a | | echo b' 2>&1
a
```

Doubled pipeline/conditional operators with empty operands
should be parse errors. zsh errors. zshrs runs the first command
(`echo a`) and silently drops the rest.

Same for `&&` and `||`:
```sh
$ /opt/homebrew/bin/zsh -fc 'echo a && && echo b' 2>&1
zsh:1: parse error near `&&'

$ ./target/debug/zshrs --zsh -c 'echo a && && echo b' 2>&1
a

$ /opt/homebrew/bin/zsh -fc 'echo a || || echo b' 2>&1
zsh:1: parse error near `||'

$ ./target/debug/zshrs --zsh -c 'echo a || || echo b' 2>&1
a
```

All three operators have the same permissive behavior in zshrs.

**Where** — `src/ported/parse.rs::parse_pipeline` / `::parse_andor`:
allows zero-token expression between consecutive operators.
C-source `Src/parse.c::par_pline` requires at least one command
between pipe-operators.

**Impact** — typos with extra `|`/`&&`/`||` silently truncate
script execution:

```sh
# user typo: extra | from copy-paste
process_input | | filter_data | output
# zsh: parse error (caught immediately)
# zshrs: process_input runs, rest dropped — filter_data and
#        output never execute
```

Cascading silent failures.

**Workaround** — careful syntax review.

---

## #172 — `${ }` (whitespace-only parameter name) silently empty

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[${ }]"' 2>&1
(empty - error)

$ ./target/debug/zshrs --zsh -c 'echo "[${ }]"' 2>&1
[]
```

A `${...}` parameter expansion requires a parameter name (or
arithmetic/cmdsub). An empty or whitespace-only `${...}` is
malformed. zsh rejects (with stderr error). zshrs silently
treats it as empty expansion.

Same with tab-only or other whitespace:
```sh
$ ./target/debug/zshrs --zsh -c 'echo "[${	}]"'
[]
```

Permissive-parser family:
- #141 (`;;`), #146 (`{} args`), #161 (case `in)`), #162 (pad
  delim), #167 (unclosed `{`), #168 (extra `}`), #169 (chained
  always), #170 (unclosed paren), #171 (empty pipe/and-or),
  #172 (this — `${ }`).

The list keeps growing — zshrs's parser is consistently more
permissive than zsh's across multiple constructs.

**Where** — `src/ported/paramsubst.rs::parse_param_name`: doesn't
require a non-whitespace name token between `{` and `}`.
C-source `Src/subst.c::parse_dollar_subst` errors on
empty/whitespace name.

**Impact** — typos that produce empty `${...}` silently return
empty strings instead of catching the error.

**Workaround** — careful syntax review.

---

## #173 — `${(t)$(cmdsub)}` returns `scalar` instead of cmdsub output

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "${(t)$(echo hi)}"'
hi

$ ./target/debug/zshrs --zsh -c 'echo "${(t)$(echo hi)}"'
scalar
```

`${(t)X}` returns the type of X. When X is `$(cmdsub)` (a value,
not a parameter), zsh has special-case behavior: returns the
cmdsub's value itself (since there's no parameter to type-check).
zshrs returns the literal `scalar` string, treating the cmdsub
result as having scalar type.

Per `man zshparam`:
> `t` — Reports the type of the parameter being expanded as a
> colon-separated list of attributes.

The behavior on non-parameter expansion isn't documented, so
this is an undocumented corner case where the shells diverge.
zsh's behavior is more useful in practice (passes the value
through).

**Where** — `src/ported/paramsubst.rs::apply_t_flag`: when the
operand is a cmdsub result (no actual parameter), returns
`"scalar"` instead of passing the value through. C-source
`Src/subst.c::dosubst` skips the `(t)` flag when there's no
parameter to type.

**Impact** — defensive code that uses `${(t)...}` as a no-op
filter for safe value passthrough gets `"scalar"` substituted
instead.

**Workaround** — explicit cmdsub without `(t)`:
```sh
echo "${$(echo hi)}"   # zsh: hi, zshrs: empty (see #165)
```

Actually `${$(...)}` is itself buggy (#165), so the workaround
is just `$(cmdsub)` directly.

---

## #174 — `type fn` (user-defined function) shows "from zsh" suffix

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'foo() { echo v1; }; functions[bar]="$functions[foo]"; type bar'
bar is a shell function

$ ./target/debug/zshrs --zsh -c 'foo() { echo v1; }; functions[bar]="$functions[foo]"; type bar'
bar is a shell function from zsh
```

After cloning a function via the `$functions[fn]` magic
assoc-array, `type bar` should report it as a regular shell
function. zsh: `bar is a shell function`. zshrs: appends
`from zsh` suffix.

The `from zsh` suffix is normally used by zsh for autoloaded
functions to show their source file (e.g., `compinit is a shell
function from /usr/share/zsh/...`). For functions defined
directly via `foo() {}`, no suffix is added in zsh. zshrs
appends `from zsh` unconditionally for cloned functions.

**Where** — `src/ported/builtin_type.rs::format_function`:
appends `from zsh` for functions created via assoc-array
assignment. Should distinguish user-defined (no suffix) from
autoloaded (`from /path/to/file`). C-source
`Src/builtin.c::printfunc` checks `Shfunc.filename` and emits
the source-path only when non-NULL.

**Impact** — `type`-output-parsing scripts that distinguish
user-defined vs autoloaded functions misclassify cloned
functions. Cross-shell test fixtures fail on the extra suffix.

**Workaround** — match loosely against `*shell function*`
prefix in any output-parsing.

---

## #175 — `(( x = 0xFF ))` doesn't preserve integer base in display

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'integer x=5; (( x = 0xFF )); echo "[$x]"; typeset -p x'
[16#FF]
typeset -i16 x=255

$ ./target/debug/zshrs --zsh -c 'integer x=5; (( x = 0xFF )); echo "[$x]"; typeset -p x'
[255]
typeset -i x=255
```

When an integer variable is assigned a hexadecimal literal via
`(( var = 0xN ))`, zsh tracks the base in the variable's
metadata: `typeset -p` shows `-i16` (base-16) and `$var` displays
as `16#FF` (radix notation).

zshrs stores only the decimal value (255), losing the base
information. Display is decimal-only.

Direct assignment `x=0xff` (without `(( ))`) DOES preserve base
in both shells:
```sh
$ both-shells -fc 'integer x; x=0xff; echo "[$x]"'
[16#FF]
```

So the bug is specifically the `(( var = HEX ))` path.

Per `man zshparam`:
> `-i[BASE]` — Use an integer numeric type. ... The argument to
> `-i` specifies the output base; with no argument, the same
> base used to assign.

**Where** — `src/ported/math.rs::assign_with_base_track`: the
arith-assignment path stores the result but doesn't update the
target's `base` attribute when the source was hex/octal/binary.
C-source `Src/math.c::matheval` calls `setiparam` with the
base from the most-recent literal.

**Impact** — visual aid for hex bitmasks lost. Permission /
mask manipulation scripts that use hex for clarity get decimal
output:

```sh
integer perm=0
(( perm = 0o644 ))   # zsh: shows "8#644" — readable
                      # zshrs: shows "420" — must convert to verify
```

**Workaround** — explicit `typeset -i 16 x=$(( 0xFF ))` to
force base display.

---

## #176 — Bare `echo "\033"` doesn't interpret backslash escapes by default

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "\033"' | od -c
033  \n

$ ./target/debug/zshrs --zsh -c 'echo "\033"' | od -c
\ 0 3 3  \n
```

zsh's `echo` builtin interprets backslash escape sequences
(`\033`, `\t`, `\n`, `\NNN`, `\xNN`) by **default**. The
`BSD_ECHO` option (default off) would change this to require
`-e` for interpretation. So zsh-default = interpret.

zshrs's `echo` doesn't interpret escapes by default — only with
explicit `-e` flag. This matches bash's default behavior, not
zsh's.

Per `man zshoptions`:
> `BSD_ECHO` — Makes `echo` behave like the BSD version, i.e.,
> the `-e` flag is required to interpret backslash escapes.
> Default: off (zsh DOES interpret by default).

Tested across forms:
```sh
# Without BSD_ECHO (default):
$ /opt/homebrew/bin/zsh -fc 'echo "\033"; echo "\t"' | od -c
033 \n \t \n             # both interpreted

$ ./target/debug/zshrs --zsh -c 'echo "\033"; echo "\t"' | od -c
\ 0 3 3 \n \ t \n        # literal, no interpretation
```

The `-E` flag (no-interpret), `print -r`, and `printf "%s"` all
preserve literals in both shells. The divergence is specifically
in **bare `echo` default mode**.

**Where** — `src/ported/builtin_echo.rs::process_args`: defaults
to no-interpret unless `-e` flag. Should default to interpret
unless `-E` flag OR `BSD_ECHO` option is set. C-source
`Src/builtin.c::bin_echo` checks `isset(BSDECHO)` to invert
the default.

**Impact** — terminal-control scripts that use `echo "\033[K"`
to emit escape sequences produce literal `\033` instead of the
escape byte:

```sh
echo "\033[2K\r"  # clear line + carriage return
# zsh: prints actual ANSI codes
# zshrs: prints literal "\033[2K\r" text
```

**Workaround** — use `print` (zsh-canonical, default-interprets
in both shells) or `printf '%b'`:
```sh
print '\033[2K\r'
printf '%b' '\033[2K\r'
```

---

## #177 — `vared` (without -c flag) doesn't emit "can't access terminal" error

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'vared -c X' 2>&1
zsh:vared:1: can't access terminal

$ ./target/debug/zshrs --zsh -c 'vared -c X' 2>&1
(empty - no error, returns)
```

The `vared` builtin is interactive — it requires a tty to edit
the value. When run in a non-interactive context (no tty), zsh
emits `can't access terminal` error and returns non-zero.

zshrs silently returns with no error and no editing.

**Where** — `src/ported/builtin_vared.rs::edit_param`: doesn't
check `isatty(stdin)` and emit error when terminal isn't
available. C-source `Src/Modules/zutil.c` errors on no-tty
condition.

**Impact** — scripts that `vared` for value editing inside
non-interactive contexts (CI, pipes, `-c` mode) silently skip
the edit step. User expects to see an error and a chance to
fall back to default; instead the variable is unchanged
silently.

```sh
echo "Please review the config:"
vared -p "Edit > " config_var   # in CI: silent no-op in zshrs
                                 # in zsh: errors, user sees it
```

**Workaround** — explicit tty check:
```sh
if [[ -t 0 ]]; then
    vared -p "Edit > " config_var
else
    echo "No terminal — skipping editor"
fi
```

---

## #178 — `IFS` doesn't affect cmdsub field-splitting in `for`/array context

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'IFS=$'\''\n'\''; for x in $(printf "a\nb\nc"); do echo "[$x]"; done'
[a]
[b]
[c]

$ ./target/debug/zshrs --zsh -c 'IFS=$'\''\n'\''; for x in $(printf "a\nb\nc"); do echo "[$x]"; done'
[a b c]
```

Setting `IFS` to a custom value (newline, tab, colon, etc.)
should control how unquoted command-substitution output is
field-split. zsh respects `IFS` for the split. zshrs always
joins with space, ignoring `IFS`.

Verified across multiple IFS values:
```sh
# IFS=":"
$ /opt/homebrew/bin/zsh -fc 'IFS=":"; for x in $(echo "a:b:c"); do echo "[$x]"; done'
[a]
[b]
[c]

$ ./target/debug/zshrs --zsh -c 'IFS=":"; for x in $(echo "a:b:c"); do echo "[$x]"; done'
[a b c]
```

Per POSIX/zsh semantics:
> Word splitting on the result of command substitution uses the
> current value of `$IFS` as the delimiter set.

**Where** — `src/ported/exec.rs::expand_for_args`: doesn't
consult `$IFS` when word-splitting cmdsub output for `for`-loop
list. C-source `Src/exec.c::execfor` calls `wordsplit` with
current IFS chars.

**Impact** — every "iterate over multi-line cmdsub output"
idiom breaks:

```sh
IFS=$'\n'
for line in $(grep error /var/log/syslog); do
    process "$line"
done
# zsh: iterates each matching line
# zshrs: passes all matches concatenated as single iteration
```

This is a fundamental POSIX-compatibility gap.

**Workaround** — explicit `${(f)$(...)}` (split on newlines via
flag) or `${(@s.X.)$(...)}` (split on X via flag):
```sh
for line in "${(@f)$(grep error /var/log/syslog)}"; do
    process "$line"
done
```

---

## #179 — `${(S)pat}` shortest-match flag treated as no-op

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 's="hello.world.txt"; echo "[${s%.*}]"; echo "[${(S)s%.*}]"; echo "[${s%%.*}]"'
[hello.world]
[hello.worldtxt]
[hello]

$ ./target/debug/zshrs --zsh -c 's="hello.world.txt"; echo "[${s%.*}]"; echo "[${(S)s%.*}]"; echo "[${s%%.*}]"'
[hello.world]
[hello.world]    # (S) flag ignored
[hello]
```

The `(S)` flag with `%`/`%%`/`#`/`##` operators changes their
match anchoring. Per `man zshexpn`:
> `(S)` — In combination with `${name#pat}` etc., specifies
> non-greedy match. With `%` and `%%`, the match searches the
> shortest occurrence at any position (instead of anchored at
> end).

zsh's behavior:
- `${s%.*}` (anchored at end): removes `.txt` → `hello.world`
- `${(S)s%.*}` (search anywhere, shortest): removes shortest `.X`
  found → `hello.worldtxt` (removes the middle `.` only, keeps
  the boundary chars)
- `${s%%.*}` (anchored, longest): removes `.world.txt` → `hello`

zshrs's `(S)` returns the same result as without it — flag is
ignored.

More tests confirm:
```sh
$ /opt/homebrew/bin/zsh -fc 's="abXcdXef"; echo "[${(S)s%X*}]"; echo "[${(S)s%%X*}]"'
[abXcdef]      # S+% = remove shortest "X*" at any position
[abXcd]         # S+%% = remove longest "X*" at any position

$ ./target/debug/zshrs --zsh -c 's="abXcdXef"; echo "[${(S)s%X*}]"; echo "[${(S)s%%X*}]"'
[abXcd]         # S not honored
[ab]            # S not honored
```

**Where** — `src/ported/paramsubst.rs::apply_S_flag`: not
implemented; falls through to default `%`/`%%` behavior.
C-source `Src/subst.c::getmatch` checks `(S)` flag bit and
inverts the anchor semantics.

**Impact** — substring removal patterns that intentionally use
`(S)` for "remove first match anywhere" produce wrong results
(zsh-specific extension, not portable).

**Workaround** — use explicit `${var/pat/}` substitution which
has independent anchoring control via `#`/`%` prefixes.

---

## #180 — `${(C)-text}` bash-style default with case-flag silently accepted

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "${(C)-from-bash-style}"' 2>&1
569x

$ ./target/debug/zshrs --zsh -c 'echo "${(C)-from-bash-style}"' 2>&1
From-Bash-Style
```

The `${(FLAG)-default}` form (without colon between FLAG and `-`)
is a bash-extension form that zsh doesn't recognize as a default
expression. zsh interprets `(C)-from-bash-style` as "apply (C)
to the parameter named `-from-bash-style`" — and `$-` is the
shell-flags special, returning `569x`.

zshrs parses it as "apply (C) to the default `from-bash-style`"
yielding `From-Bash-Style`.

Per `man zshexpn`:
> `${name:-default}` (with colon) — apply default if unset.
> `${name-default}` (no colon) — apply default if unset, but
> not if empty.

The form `${(C)-text}` (with the FLAG and `-`) is non-standard;
zsh treats `-text` as part of the parameter name.

**Where** — `src/ported/paramsubst.rs::parse_flag_then_default`:
treats `-text` after `)` as default-text. C-source
`Src/subst.c::dosubst` requires colon-separator.

**Impact** — bash-style `${(C)-default}` works in zshrs but
not zsh. Cross-shell scripts that use this syntax (originally
bash) silently produce different results. Falls in the
permissive-parser family — zshrs accepts bash-isms that zsh
rejects.

**Workaround** — always use colon-form `${(FLAG):-default}`:
```sh
echo "${(C):-default}"   # both shells: "Default"
```

---

## #181 — `typeset -p` doesn't quote array elements containing spaces

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(red "blue green" yellow); typeset -p a'
typeset -a a=( red 'blue green' yellow )

$ ./target/debug/zshrs --zsh -c 'a=(red "blue green" yellow); typeset -p a'
typeset -a a=( red blue green yellow )
```

`typeset -p` should produce **re-parseable** output. Elements
containing whitespace must be quoted so the result can be fed
back to `eval` and reconstruct the same array. zsh wraps
`"blue green"` in single quotes → `'blue green'` so it remains
one element when parsed. zshrs omits quoting; the output looks
like a 4-element array (`red`, `blue`, `green`, `yellow`).

Same for elements containing quotes/specials:
```sh
$ /opt/homebrew/bin/zsh -fc 'a=("with \"quotes\"" "with '\''single'\''"); typeset -p a'
typeset -a a=( 'with "quotes"' 'with '\''single'\'' )

$ ./target/debug/zshrs --zsh -c 'a=("with \"quotes\"" "with '\''single'\''"); typeset -p a'
typeset -a a=( with "quotes" with 'single' )
```

zshrs's output is unparseable — eval'ing it produces wrong
array.

**Where** — `src/ported/builtin_typeset.rs::format_array`:
emits elements verbatim with space-separation. Should
single-quote-wrap any element containing whitespace or shell
metacharacters. C-source `Src/builtin.c::printparam` calls
`quotedzputs` for each array element.

**Impact** — snapshot/restore patterns break:

```sh
# save state
output=$(typeset -p config_array)
... do work ...
# restore
eval "$output"
# zsh: config_array restored exactly
# zshrs: array re-parsed with wrong element boundaries
```

Cross-shell test fixtures that compare typeset output also fail.

**Workaround** — manual re-quoting via `${(qq)elem}` loop:
```sh
out="$(typeset -a config_array=("
for el in "${config_array[@]}"; do
    out+=" ${(qq)el}"
done
out+=" )"
```

---

## #182 — `${${(P)name}[N]}` after-deref indexing returns full array

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(red blue green); name=a; echo "${${(P)name}[1]}"; echo "${${(P)name}[2]}"'
red
blue

$ ./target/debug/zshrs --zsh -c 'a=(red blue green); name=a; echo "${${(P)name}[1]}"; echo "${${(P)name}[2]}"'
red blue green
red blue green
```

When `${(P)name}` returns an array (because `$name`=`a` and `$a`
is an array), the outer `[N]` subscript should index into the
resolved array. zsh extracts element N correctly. zshrs ignores
the subscript and returns the full array for any index.

Same family as #53/#77 indirect-deref bugs and #63/#82 nested-
expansion bugs — context propagation through nested `${...}`
forms consistently broken.

**Where** — `src/ported/paramsubst.rs::nested_index`: outer
subscript applied to inner expansion not honored when the
inner is array-context. C-source `Src/subst.c::dosubst`
threads the subscript through to apply after the inner
expansion resolves.

**Impact** — indirect access patterns for picking out elements:

```sh
arrname=user_emails
echo "primary: ${${(P)arrname}[1]}"
# zsh: first email
# zshrs: comma-or-space-joined all emails (wrong format)
```

**Workaround** — assign deref result to a temp array first:
```sh
local -a deref=("${(@P)arrname}")
echo "primary: ${deref[1]}"
```

---

## #183 — `"${@:1:2}"` positional slice returns full args instead of slicing

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'set -- "a b" c; for x in "${@:1:2}"; do echo "[$x]"; done'
[a b]
[c]

$ ./target/debug/zshrs --zsh -c 'set -- "a b" c; for x in "${@:1:2}"; do echo "[$x]"; done'
[a b c]
```

`${@:START:LENGTH}` slices positional parameters: starting at
position START, take LENGTH elements. zsh: 2 elements (preserves
quoting of `"a b"` as first element). zshrs: returns a single
joined string for all positional parameters.

Same family as #155 (string slice ignores arith) and #147 (`@`
flag + modifier dropped) — array-slice handling broken across
multiple paths.

Verified other slice forms:
```sh
$ both-shells -fc 'set -- a b c d e; echo "[${@:2:3}]"'
[b c d]               # both shells: works (without quotes)

$ /opt/homebrew/bin/zsh -fc 'set -- "x y" "z" w; echo "[${@:1:2}]"'
[x y z]               # zsh: works with spaces preserved

$ ./target/debug/zshrs --zsh -c 'set -- "x y" "z" w; echo "[${@:1:2}]"'
[x y z w]             # zshrs: doesn't slice, returns all
```

So the bug specifically manifests when LENGTH < $#: zshrs
returns all positional params instead of taking just LENGTH.

**Where** — `src/ported/paramsubst.rs::slice_positionals`:
ignores LENGTH parameter for `${@:N:M}` form. C-source
`Src/subst.c::getarrelt` honors start+length pair.

**Impact** — defensive positional-arg processing breaks:

```sh
my_wrapper() {
    local -a opts=("${@:1:2}")     # take first 2 args as options
    local -a rest=("${@:3}")        # rest as additional args
    process_opts "${opts[@]}"
    process_rest "${rest[@]}"
}
# zsh: correctly splits args
# zshrs: opts gets all args; rest gets all args (overlap, no split)
```

**Workaround** — manual loop:
```sh
local -a opts=()
local i
for (( i=1; i<=2 && i<=$#; i++ )); do
    opts+=("${argv[$i]}")
done
```

---

## #184 — `$((${a[@]} + 0))` arith with array spread silently uses first element

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(1 2 3); echo "$((${a[@]} + 0))"' 2>&1
zsh:1: bad math expression: operator expected at `2 3 + 0'

$ ./target/debug/zshrs --zsh -c 'a=(1 2 3); echo "$((${a[@]} + 0))"'
1
```

Inside `$((...))` arith context, `${a[@]}` spreads array as
space-joined string. zsh tries to parse `1 2 3 + 0` as arith,
fails with "operator expected". zshrs silently uses only the
first element (`1 + 0 = 1`).

Different failure modes:
- zsh: clear syntax error, exit non-zero, user notices
- zshrs: silent wrong result, no error

Per zsh arith semantics, `${a[@]}` in arith should either:
1. Expand to first element only (per shell-history convention),
2. Error (zsh's choice),
3. Sum all elements (Python-like).

zshrs picks option 1 silently. zsh picks option 2 explicitly.
Both behaviors are defensible; the silent-wrong-result is the bug.

**Where** — `src/ported/math.rs::eval_array_token`: takes only
the first element when an array expansion appears in arith.
Should either error or process the entire spread. C-source
`Src/math.c::mathevalreal` errors on multi-token arith input.

**Impact** — common pattern of summing array via arith silently
produces wrong totals:

```sh
prices=(10 20 30)
total=$((${prices[@]}))    # user intended sum
# zsh: errors (caught)
# zshrs: total=10 (silently wrong)
```

**Workaround** — explicit loop:
```sh
total=0
for p in "${prices[@]}"; do
    (( total += p ))
done
```

---

## #185 — `[[ -z "${arr[@]}" ]]` returns false for single-empty array (zsh: true)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'b=(""); [[ -z "${b[@]}" ]] && echo "empty" || echo "not-empty"'
empty

$ ./target/debug/zshrs --zsh -c 'b=(""); [[ -z "${b[@]}" ]] && echo "empty" || echo "not-empty"'
not-empty
```

An array with a single empty-string element: `b=("")`. When
expanded via `"${b[@]}"` in `-z` test context, the result is
a single empty string. zsh's `-z` returns true (the joined
result is empty). zshrs's `-z` returns false (perhaps testing
whether the array has elements rather than the joined content).

Comparison with `${b[*]}` (star form):
```sh
$ both-shells -fc 'b=(""); [[ -z "${b[*]}" ]] && echo "empty"'
empty                    # both shells agree on star form
```

So only the `[@]` form diverges.

**Where** — `src/ported/cond.rs::eval_z`: when operand is
`"${arr[@]}"`, checks array length instead of joined content.
C-source `Src/cond.c::evalcond` joins the array first then
checks string length.

**Impact** — defensive emptiness checks misclassify:

```sh
config_lines=("")    # placeholder for "no config"
if [[ -z "${config_lines[@]}" ]]; then
    use_defaults
fi
# zsh: use_defaults runs
# zshrs: skips the branch (treats as non-empty)
```

---

## #186 — `${(@)b:-default}` for single-empty array returns default (zsh: returns empty)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'b=(""); echo "[${(@)b:-default}]"'
[]

$ ./target/debug/zshrs --zsh -c 'b=(""); echo "[${(@)b:-default}]"'
[default]
```

Inverse of #185 — same `b=("")` array, but here the `:-default`
fallback behavior:
- zsh: array has one element (even if empty) → not "unset" → no
  default. Returns `[]` (the empty element).
- zshrs: treats array as empty → applies default. Returns
  `[default]`.

Verified against other states:
```sh
$ both-shells -fc 'a=(); echo "[${(@)a:-default}]"; c=("x"); echo "[${(@)c:-default}]"'
[default]                # truly empty → default
[x]                       # non-empty → element
```

So the divergence is specifically the single-empty-element case.

Combined with #185 (where zsh thinks "${b[@]}" IS empty for -z),
this is logically inconsistent in zsh too — but the divergence
from zshrs is real and reproducible.

**Where** — `src/ported/paramsubst.rs::default_branch`: checks
array length only, not effective string content. Should apply
default only when length is 0. C-source `Src/subst.c::dosubst`
distinguishes "no elements" from "elements all empty".

**Impact** — fallback patterns for "use this OR default" misfire
on intentionally-empty-element arrays:

```sh
custom_args=("")     # placeholder: no extra args wanted
cmd=("my-tool" "${(@)custom_args:-default-args}")
# zsh: cmd has 2 elements ("my-tool", "")
# zshrs: cmd has 2 elements ("my-tool", "default-args")
```

**Workaround** — explicit `${#arr}` check:
```sh
if (( ${#custom_args} > 0 )); then
    cmd=("my-tool" "${custom_args[@]}")
else
    cmd=("my-tool" "default-args")
fi
```

---

## #187 — `f() { :; } > /file` redirect on fn-def creates file at definition time

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { :; } > /tmp/zfr'
$ ls /tmp/zfr 2>&1
ls: /tmp/zfr: No such file or directory

$ ./target/debug/zshrs --zsh -c 'g() { :; } > /tmp/zfr'
$ ls /tmp/zfr 2>&1
-rw-r--r--@ 1 wizard wheel 0 ... /tmp/zfr
```

Function-definition redirects (`f() { ... } > file`) should
attach the redirect to the function as part of its execution
context. zsh defers the actual open() until the function is
called. zshrs opens the file at definition time, creating a
zero-byte file even if `f` is never called.

This is the inverse of #158 — there, redirects didn't apply
at call time. This bug shows the redirect creates side effects
at def time. Combined: redirects are processed at the WRONG
time entirely.

Same for stderr:
```sh
$ ./target/debug/zshrs --zsh -c 'g() { :; } 2> /tmp/zfy' && ls /tmp/zfy
/tmp/zfy
```

Both stdout and stderr redirects affected.

**Where** — `src/ported/parse.rs::parse_function_def`: when
encountering trailing redirects, opens the FDs immediately
instead of storing the redirect spec in the function struct.
C-source `Src/parse.c::par_funcdef` stores `Redir` nodes in
`Shfunc.redir` chain for deferred application.

**Impact** — fn-def files leak in `/tmp` (or wherever the
redirect points) even when fns are never called. Worse:
truncates existing files if the path exists:

```sh
my_logger() {
    echo "$@"
} > /var/log/app.log      # zsh: log file created on first call
                           # zshrs: log file TRUNCATED at def time
                           #        even if my_logger never called
```

Data loss possible.

**Workaround** — never use redirect-on-fn-def in zshrs. Move
the redirect to the call site:
```sh
my_logger() { echo "$@"; }
my_logger "msg" > /var/log/app.log
```

---

## #188 — Empty-array slice `${a[@]:0:1}` iterates once with empty value

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(); count=0; for x in "${a[@]:0:1}"; do ((count++)); echo "iter[$x]"; done; echo "iterations: $count"'
iterations: 0

$ ./target/debug/zshrs --zsh -c 'a=(); count=0; for x in "${a[@]:0:1}"; do ((count++)); echo "iter[$x]"; done; echo "iterations: $count"'
iter[]
iterations: 1
```

Slicing an empty array `${a[@]:0:1}` should yield no elements.
zsh iterates 0 times. zshrs iterates ONCE with an empty value.

Related to #120 (`a=("${a[@]:0:-1}")` on empty creates
1-element array) — both stem from empty-array slice yielding
spurious empty element.

**Where** — `src/ported/paramsubst.rs::slice_array`: returns
`[""]` for empty array slice instead of `[]`. C-source
`Src/subst.c::arrslice` returns `NULL` (empty list) when start
≥ end or array is empty.

**Impact** — defensive iteration on possibly-empty arrays
runs one phantom iteration with empty value:

```sh
configs=()
for cfg in "${configs[@]:0:3}"; do
    process_config "$cfg"
done
# zsh: skip entirely
# zshrs: process_config called once with empty string
```

**Workaround** — explicit length check:
```sh
if (( ${#configs} > 0 )); then
    for cfg in "${configs[@]:0:3}"; do ...
fi
```

---

## #189 — `${()-default}` empty-flag-paren silently accepted

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[${()-default}]"' 2>&1
zsh:1: bad substitution

$ ./target/debug/zshrs --zsh -c 'echo "[${()-default}]"' 2>&1
[569X]
```

`${()-default}` — empty parameter-flag parentheses, then default
`-default`. zsh rejects as bad substitution (empty `(...)` flag
is malformed). zshrs treats it as if the parameter name were `$-`
(shell flags), returning the value of `$-` (`569X`).

Permissive-parser family. The `()` empty flag should be a
syntax error.

**Where** — `src/ported/paramsubst.rs::parse_flags`: doesn't
require at least one flag character between `(` and `)`. Treats
empty parens as no-flag, then `${-default}` parses as `$-`
(shell flags) with `-default` consumed.

**Impact** — typos with empty flag parens silently return shell
flags instead of erroring:

```sh
# user typo: forgot to specify flag (e.g., L)
echo "${()-fallback}"
# zsh: errors immediately
# zshrs: returns "$-" value, confusing
```

**Workaround** — careful syntax review.

---

## #190 — `kill -L` lists signals (zsh: errors "unknown signal: SIGL")

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'kill -L 2>&1'
zsh:kill:1: unknown signal: SIGL
zsh:kill:1: type kill -l for a list of signals

$ ./target/debug/zshrs --zsh -c 'kill -L 2>&1'
 1 HUP         2 INT         3 QUIT        4 ILL
 5 TRAP        6 ABRT        7 EMT         8 FPE
 9 KILL       10 BUS        11 SEGV       12 SYS
...
```

zsh's `kill` builtin recognizes only `-l` (lowercase) as the
"list signals" flag. `-L` (uppercase) is treated as a signal
specifier — parses as `SIGL` (unknown).

zshrs accepts `-L` as if it were `-l`. Different option-parsing
behavior.

Per `man zshbuiltins`:
> `kill -l [ SIG ... ]` — List signal names, or print a signal
> number's corresponding name.

Only `-l` is documented. `-L` may be a Linux-procps extension
that zshrs imports but isn't zsh-standard.

**Where** — `src/ported/builtin_kill.rs::parse_flags`: accepts
both `-l` and `-L` as the list flag. C-source
`Src/Modules/kill.c::bin_kill` accepts only `-l`.

**Impact** — scripts using `kill -L` work in zshrs but fail in
zsh. Bash also has only `-l`, so cross-shell scripts shouldn't
use `-L` (and the lowercase form works in both).

**Workaround** — always use `kill -l` (lowercase).

---

## #191 — `${(l.5..)s}` empty-fill character accepted with garbage output

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 's=hi; echo "[${(l.5..)s}]"' 2>&1
zsh:1: error in flags near position 8 in '${(l.5..)s}]'

$ ./target/debug/zshrs --zsh -c 's=hi; echo "[${(l.5..)s}]"' 2>&1
[s)s)s]
```

The `(l.<width>.<fill1>.<fill2>.)` pad-flag syntax requires both
fill characters between the dots. Empty fills `(l.5..)` are
malformed. zsh rejects with clear error. zshrs accepts the
malformed form and produces nonsense output `[s)s)s]` (apparently
substituting `)s` repeatedly).

Same family as #162 (pad-flag missing close delim).

**Where** — `src/ported/paramsubst.rs::parse_pad_flag_args`:
empty fill chars treated as zero-length string, padding logic
uses bytes from the wrong source. C-source
`Src/subst.c::parsesubst` errors on empty fill.

**Impact** — typos in pad-flag syntax produce garbage values
instead of clear errors:

```sh
echo "${(l.${col}..)val}"     # forgot to specify fill char
# zsh: errors with position info
# zshrs: produces nonsense output silently
```

**Workaround** — always specify both fill positions even if just
spaces: `(l.5. . .)` or use the simpler `(l.5.X.)`.

---

## #192 — `${(P)name[N]:mod}` indirect-array-elem with modifier works in zshrs (zsh: errors)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'arr=(/a/b /c/d); name=arr; echo "${(P)name[1]:t}"; echo "${(P)name[2]:t}"' 2>&1
(empty - errors)

$ ./target/debug/zshrs --zsh -c 'arr=(/a/b /c/d); name=arr; echo "${(P)name[1]:t}"; echo "${(P)name[2]:t}"'
b
d
```

`${(P)name[N]:mod}` — indirect deref via `name`, take element N,
apply modifier. zsh errors on this combined form. zshrs computes
correctly and returns the tail of each indexed element.

zshrs's behavior is arguably MORE useful (zsh's error seems
overly strict). But the divergence is real — cross-shell scripts
that work in zshrs error in zsh.

This is the OPPOSITE polarity of most other bugs in this set:
zshrs is more permissive AND more correct semantically, while
zsh is strict-and-broken.

**Where** — `src/ported/paramsubst.rs::indirect_array_modifier`:
chains the modifier after the indirect-deref correctly. C-source
`Src/subst.c::dosubst` doesn't allow modifier after `(P)+[N]`
combo.

**Impact** — code written for zshrs that uses this idiom fails
in zsh. Cross-shell scripts should avoid:

```sh
# zshrs-only working form:
basename="${(P)arr_name[1]:t}"
# zsh equivalent (works in both):
local deref="${(@P)arr_name}"
basename="${deref[1]:t}"
```

**Workaround** — split into two steps via temp variable.

---

## #193 — `(( y = ${x:?msg} ))` continues after required-param error in arith context

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'unset x; (( y = ${x:?msg} )); echo "after: ec=$?"' 2>&1
zsh:1: x: msg
(no "after" — aborted)

$ ./target/debug/zshrs --zsh -c 'unset x; (( y = ${x:?msg} )); echo "after: ec=$?"' 2>&1
zsh:1: x: msg
after: ec=2
```

When `${x:?msg}` errors (x is unset), zsh aborts script
execution. zshrs prints the error but continues to next
statement.

Specifically the arith-context `(( ))` path is affected. The
standalone `${x:?msg}` outside arith correctly aborts in both
shells:

```sh
$ both-shells -fc 'unset x; echo "${x:?msg}"; echo "after"' 2>&1
zsh:1: x: msg
# (no "after" in either shell)
```

Related to bug #74 (`local -r` violation in fn doesn't abort)
— both stem from error-propagation gaps in non-statement
contexts.

**Where** — `src/ported/math.rs::eval_required_substitution`:
when the substitution errors via `:?`, prints the error but
doesn't set `errflag |= ERRFLAG_ERROR`. C-source
`Src/math.c::matheval` propagates the substitution error
through the arith evaluator.

**Impact** — defensive arith patterns with required-param
guards silently continue past errors:

```sh
(( max_size = ${MAX_SIZE_BYTES:?MAX_SIZE_BYTES must be set} ))
# zsh: aborts at error
# zshrs: max_size left as scalar default (0), continues
allocate_buffer $max_size   # zsh: never reached  zshrs: allocates 0
```

**Workaround** — explicit guard before arith:
```sh
[[ -v MAX_SIZE_BYTES ]] || { echo "MAX_SIZE_BYTES required" >&2; exit 1; }
(( max_size = MAX_SIZE_BYTES ))
```

---

## #194 — `function f { :; } > /file` keyword-form fn-def redirect creates file at def time

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'function f { echo body; } > /tmp/zfk' && ls /tmp/zfk
ls: /tmp/zfk: No such file or directory

$ ./target/debug/zshrs --zsh -c 'function f { echo body; } > /tmp/zfk' && ls /tmp/zfk
/tmp/zfk     (file created with 0 bytes)
```

Same bug as #187 (paren-form `f() { :; } > file`), but for the
`function`-keyword fn-definition form. Both syntactic variants
suffer from the same issue: redirect on fn-def takes effect at
definition time instead of at function-call time.

zsh defers the open() until function call. zshrs opens
immediately at definition, creating/truncating the file
without any call.

**Where** — `src/ported/parse.rs::parse_function_keyword`:
inherits the bug from `parse_function_def` (paren form). Both
forms route through the same execution path that immediately
processes trailing redirects.

**Impact** — same as #187 — data-loss risk for `function
my_logger { ... } > /var/log/app.log` patterns where the log
file is truncated at definition time before any logging.

**Workaround** — same as #187 — redirect at call site, not at
definition.

---

## #195 — `${(C)${(P)name}[N]}` flag applied to full array, outer subscript ignored

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'arr=("hello world" "foo bar"); name=arr; echo "${(C)${(P)name}[1]}"'
Hello World

$ ./target/debug/zshrs --zsh -c 'arr=("hello world" "foo bar"); name=arr; echo "${(C)${(P)name}[1]}"'
Hello World Foo Bar
```

`${(C)${(P)name}[N]}` should:
1. Deref `(P)name` → returns array `("hello world", "foo bar")`.
2. Index `[1]` → first element `"hello world"`.
3. Apply `(C)` capitalize → `"Hello World"`.

zsh produces `"Hello World"`. zshrs applies `(C)` to the FULL
array (capitalizing all elements: "Hello World Foo Bar") and
ignores the outer `[1]` subscript.

The inner-subscript form `${(C)${(P)name[1]}}` works in zshrs
but errors in zsh (bug #192).

Same family as #182 (after-deref indexing returns full array)
and #147 (modifier dropped after flag) — context propagation
through nested `${...}` forms broken.

**Where** — `src/ported/paramsubst.rs::nested_subscript`: outer
subscript applied to inner expansion not honored when inner is
array-context after flag. C-source `Src/subst.c::dosubst`
threads subscript through to apply after the inner expansion
resolves.

**Impact** — code intending to apply case flag to a specific
indirect-array element gets the flag applied to all elements:

```sh
arrname=user_emails
echo "Primary: ${(C)${(P)arrname}[1]}"
# zsh: "Primary: Alice@Example.com"
# zshrs: "Primary: Alice@Example.com Bob@Example.com Charlie@Example.com"
```

**Workaround** — temp variable:
```sh
local -a deref=("${(@P)arrname}")
echo "Primary: ${(C)deref[1]}"
```

---

## #196 — Anonymous function output not captured by `$(...)` or `(...)` subshell

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'x=$(() { echo hi }); echo "[$x]"'
[hi]

$ ./target/debug/zshrs --zsh -c 'x=$(() { echo hi }); echo "[$x]"'
[]
```

Same gap in plain subshell:
```sh
$ /opt/homebrew/bin/zsh -fc '(() { echo subhi })'
subhi

$ ./target/debug/zshrs --zsh -c '(() { echo subhi })'
(empty)
```

And in array-context:
```sh
$ /opt/homebrew/bin/zsh -fc 'arr=($(() { echo "a b c" })); echo "${#arr}"'
3

$ ./target/debug/zshrs --zsh -c 'arr=($(() { echo "a b c" })); echo "${#arr}"'
(empty)
```

Anonymous function `() { ... }` invoked inside any forking
construct loses stdout entirely. The function appears to
execute (no error printed) but its output never propagates
out of the fork.

Direct invocation at top level works:
```sh
$ both-shells -fc '() { echo hi }'
hi
```

**Where** — `src/ported/exec.rs::execpline` (subshell-fork path):
anonymous-fn dispatch via `Wordcode::Funcdef-inline` doesn't
inherit the forked child's stdout. C-source `Src/exec.c::execpline`
forks then `execlist` runs the anon-fn with normal stdout fd
inheritance.

**Impact** — common zsh idiom for scoped helpers is broken in
capture contexts:

```sh
# Common pattern: scoped logic with no name pollution
config=$(() {
  local IFS=:
  for dir in $path; do
    [[ -f "$dir/myconfig" ]] && { cat "$dir/myconfig"; return }
  done
})
# zsh: $config contains the first matching file's contents
# zshrs: $config is empty regardless of what files exist
```

**Workaround** — use a named function and call it inside the
subshell:
```sh
_get_config() { ... }
config=$(_get_config)
unfunction _get_config
```

---

## #197 — `typeset -f` function-body display collapses statement newlines into semicolons

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { echo b1; echo b2; }; typeset -f f'
f () {
	echo b1
	echo b2
}

$ ./target/debug/zshrs --zsh -c 'f() { echo b1; echo b2; }; typeset -f f'
f () {
	echo b1; echo b2
}
```

zsh normalizes the function body by splitting `;`-separated
statements onto separate lines (each indented with tab) for
display. zshrs preserves the original `;` separator and keeps
both statements on one line.

`functions f` and `which f` share the same display path and
likely suffer the same bug.

**Where** — `src/ported/text.rs::gettext` (text-reconstruction
of parsed Eprog node). zsh's C-source `Src/text.c::gettext2`
emits `\n\t` instead of `; ` between sibling pline nodes.

**Impact** — anyone parsing/diffing function bodies via `typeset
-f` (config management, security audits, golden-file tests)
sees diff output where there should be none:

```sh
# Diff golden function body vs current
diff <(typeset -f my_fn) golden/my_fn.txt
# zsh: no diff (matches golden captured from zsh)
# zshrs: diff every line because separators differ
```

**Workaround** — pipe through sed to normalize:
```sh
typeset -f f | sed 's/; /\n\t/g'
```

---

## #198 — `bindkey -L` output format diverges entirely from zsh

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'bindkey -L | head -1'
bindkey -R "^A"-"^C" self-insert

$ ./target/debug/zshrs --zsh -c 'bindkey -L | head -1'
bindkey "^@" set-mark-command
```

zsh's `-L` emits range-compressed output using `-R` flag and
`"^A"-"^C"` style notation — runs of keys bound to the same
widget are collapsed into one range line. zshrs emits one line
per individual key with no range compression and a different
sort order.

For someone parsing `bindkey -L` to round-trip bindings
(`bindkey -L > saved; ...; source saved`), the formats differ
not just cosmetically — zshrs output saved from zshrs may
re-parse correctly but the diff against zsh's output is total.

**Where** — `src/ported/zle/bindings.rs::print_bindings`:
no range-detection pass. C-source `Src/Zle/zle_keymap.c::scanbindings`
collapses adjacent same-binding entries into `keymap -R lo-hi widget`
format.

**Impact** — config/golden tests that capture `bindkey -L`
output for diff fail wholesale on zshrs. Migration tools that
parse the output expecting zsh's range format get individual
entries instead.

**Workaround** — when round-tripping, save and source via
zshrs's own output (which it parses back correctly), don't
cross between zsh and zshrs.

---

## #199 — `${(qq)x}` with embedded newline emits mixed `'a'$'\n''b'` instead of literal newline

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc $'x=$\'a\\nb\'; echo "${(qq)x}"'
'a
b'

$ ./target/debug/zshrs --zsh -c $'x=$\'a\\nb\'; echo "${(qq)x}"'
'a'$'
''b'
```

zsh's `(qq)` flag produces single-quote-style quoting and
embeds literal newline inside the single quotes (single quotes
preserve newlines in zsh syntax). zshrs splits at the newline
and inserts `$'\n'` between segments — that's `(q+)`-style
behavior, not `(qq)`.

Both forms re-parse to the same value, but tools that
checksum/golden-test serialized output see a diff:

```sh
typeset -p multi_line_var | sha256sum
# zsh and zshrs disagree even though the underlying data is identical
```

**Where** — `src/ported/paramflags.rs::quote_qq`: emits
`$'\n'` escape for newline within `(qq)`. C-source
`Src/utils.c::quotestring` with `QT_DOUBLE` keeps newlines
literal inside single quotes.

**Impact** — golden-file diff tests, config-migration scripts,
and `typeset -p`-based serialization round-trips show
unexpected drift between zsh and zshrs even when the
underlying values are byte-equal.

**Workaround** — use `(q+)` flag explicitly (which both shells
implement consistently), or normalize via:
```sh
typeset -p var | tr -d $'\n' | ... # before checksumming
```

---

## #200 — `${(k)assoc[key]}` key-flag with subscript returns empty (zsh: returns key when subscript matches)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h[k1]=v1; echo "${(k)h[k1]}"'
k1

$ ./target/debug/zshrs --zsh -c 'typeset -A h; h[k1]=v1; echo "${(k)h[k1]}"'
(empty)
```

The `(k)` key-flag combined with subscript `[key]` is zsh's
idiom for "return the key if it exists in the assoc array".
zshrs returns empty regardless of whether the key exists.

This is the assoc-array analog of "test existence":
```sh
[[ -n "${(k)config[$user_input]}" ]] && echo "key exists"
# zsh: detects existence correctly
# zshrs: always reports key as missing because (k) returns empty
```

The full-array form `${(k)h}` (return all keys) works:
```sh
$ both-shells -fc 'typeset -A h; h[k1]=v1; h[k2]=v2; echo "${(k)h}"'
k1 k2
```

Just the with-subscript form is broken.

**Where** —
`src/ported/paramsubst.rs::apply_k_flag_with_subscript`:
doesn't honor `(k)` when subscript is present. C-source
`Src/subst.c::paramsubst` checks `PRINT_KIND_KEY` flag after
subscript resolution and re-emits the key string.

**Impact** — assoc-array key-existence idiom returns
false-negative for every key in zshrs, breaking common
existence-test patterns:

```sh
if [[ -n "${(k)valid_options[$arg]}" ]]; then
    use_option "$arg"
else
    error "unknown option: $arg"
fi
# zsh: takes the right branch
# zshrs: always falls through to error branch
```

**Workaround** — use `(v)` value flag or `[[ -v ]]` test:
```sh
[[ -v "valid_options[$arg]" ]] && use_option "$arg" || error "..."
```

---

## #201 — `typeset +x VAR` doesn't remove VAR from environment

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -x V=1; typeset +x V; env | grep "^V="'
(empty — V removed from env)

$ ./target/debug/zshrs --zsh -c 'typeset -x V=1; typeset +x V; env | grep "^V="'
V=1
```

`typeset +x VAR` is supposed to clear the export attribute —
the var stays as a shell parameter but is removed from the
environment passed to child processes. zsh removes it. zshrs
leaves it in the environment.

This is a basic POSIX-compatible feature (the `+` reverses an
attribute flag).

**Where** — `src/ported/builtins/typeset.rs::handle_plus_flag`:
clears the `PM_EXPORTED` bit on the Param struct but doesn't
call `unsetenv()`/`envc.remove()` for the underlying os-env
entry. C-source `Src/builtin.c::typeset_single` calls
`removeenv(value)` when the `-x` flag is being removed.

**Impact** — exported variables can't be un-exported without
fully unsetting. Common cleanup patterns leak into child
processes:

```sh
typeset -x SECRET=$(get_secret)
do_work_with_secret
typeset +x SECRET
# zsh: child processes won't see SECRET
# zshrs: SECRET still leaks into env of every subsequent child
exec my_untrusted_program  # gets SECRET in env on zshrs
```

This is a security-relevant gap for credential-handling code
that toggles export state mid-script.

**Workaround** — `unset VAR; typeset VAR=$saved_value` to fully
clear from env then re-bind as non-exported parameter.

---

## #202 — `set -eo pipefail` doesn't abort on failed pipe component

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'set -eo pipefail; false | true; echo never'
(no output — aborted on pipefail status)
$ echo $?
1

$ ./target/debug/zshrs --zsh -c 'set -eo pipefail; false | true; echo never'
never
$ echo $?
0
```

With `set -e` (errexit) AND `pipefail` enabled, zsh aborts the
script when the pipeline returns non-zero (because pipefail
makes `false | true` return 1, and errexit aborts on 1).
zshrs's pipefail handling returns 0 from `false | true`, so
errexit doesn't fire.

Tested without errexit:
```sh
$ /opt/homebrew/bin/zsh -fc 'setopt pipefail; false | true; echo "ec=$?"'
ec=1

$ ./target/debug/zshrs --zsh -c 'setopt pipefail; false | true; echo "ec=$?"'
ec=0
```

The root cause is that `pipefail` is being ignored entirely —
the pipeline exit code is always the last command's status,
not the leftmost-failing one.

**Where** — `src/ported/exec.rs::execpline` (pipeline status
finalization): no consultation of `PIPEFAIL` opt when setting
`lastval`. C-source `Src/exec.c::execpline` checks
`isset(PIPEFAIL)` and walks the pipeline status array for the
leftmost non-zero.

**Impact** — every script using `set -eo pipefail` (a near-
universal idiom for "strict mode" zsh/bash scripts) silently
loses error detection on pipelines. Critical for CI scripts,
deployment pipelines, anything that processes external data
through filters where the source might fail:

```sh
set -eo pipefail
curl -sf https://example.com/data | jq '.field'
# zsh: aborts if curl fails
# zshrs: continues with jq's exit code (which may parse "" as null and succeed)
```

**Workaround** — explicit `${pipestatus[1]}` check after
each pipeline:
```sh
cmd1 | cmd2
[[ ${pipestatus[1]} -eq 0 ]] || exit 1
```

---

## #203 — `trap EXIT` set inside function runs at shell exit instead of function return

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { trap "echo BYE" EXIT; echo "in-fn"; }; f; echo "after-call"; exit 0'
in-fn
BYE
after-call

$ ./target/debug/zshrs --zsh -c 'f() { trap "echo BYE" EXIT; echo "in-fn"; }; f; echo "after-call"; exit 0'
in-fn
after-call
BYE
```

zsh documents (`man zshbuiltins`): "A trap on EXIT set inside
a function is executed before the function completes." zshrs
treats `trap EXIT` inside a function as a shell-level trap,
firing on shell exit instead.

This breaks the common "scoped cleanup" idiom — set a trap at
function entry to clean up temp resources, knowing it'll fire
on every exit path from the function:

```sh
process_file() {
    local tmpfile=$(mktemp)
    trap "rm -f $tmpfile" EXIT
    work_with $tmpfile
    # zsh: tmpfile gets removed when fn returns
    # zshrs: tmpfile not removed until shell exit (leaks across calls)
}
process_file f1   # creates tmp1, doesn't clean up
process_file f2   # creates tmp2, prior trap fires? unclear semantics
```

**Where** — `src/ported/builtins/trap.rs::install`: doesn't
check function-scope when binding EXIT — installs into global
sig-trap table. C-source `Src/signals.c::settrap` with
`in_function` checks current func depth and pushes the trap
onto the function-scoped LIFO that fires on `doshfunc` return.

**Impact** — scoped-cleanup idioms (the standard zsh pattern
for safe resource handling) don't work. Long-running shells
(or interactive zshrs) accumulate trap actions from every
function call, all firing in unpredictable order at shell
exit.

For interactive sessions, this means trap-based "temp file
cleanup" patterns leak files for the entire session lifetime.

**Workaround** — use explicit cleanup at all return points
within the function, or use `local`-scoped traps via a
sub-shell wrapper:
```sh
process_file() {
    (
        local tmpfile=$(mktemp)
        trap "rm -f $tmpfile" EXIT
        work_with $tmpfile
    )  # subshell exit fires trap correctly
}
```

---

## #204 — `setopt promptsubst` doesn't enable `$(...)` / `${var}` expansion in prompts

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt promptsubst; PS1="\$(echo hi)"; print -P "$PS1"'
hi

$ ./target/debug/zshrs --zsh -c 'setopt promptsubst; PS1="\$(echo hi)"; print -P "$PS1"'
$(echo hi)
```

With `promptsubst` enabled, zsh applies parameter/command/arith
substitution to the prompt string before prompt-escape
expansion. zshrs ignores the option — prompt strings only get
`%` escape expansion, never `$()`/`${}`/`$((... ))` expansion.

The bug also affects variable expansion in prompts:
```sh
$ both-shells -fc 'setopt promptsubst; x=hello; PS1="$x"; print -P "$PS1"'
# zsh:   hello
# zshrs: $x (literal — but actually $x expands at PS1 assignment, so this works)

# Tighter test forcing late expansion:
$ /opt/homebrew/bin/zsh -fc 'setopt promptsubst; PS1='"'"'${MY_VAR}'"'"'; MY_VAR=now; print -P "$PS1"'
now

$ ./target/debug/zshrs --zsh -c 'setopt promptsubst; PS1='"'"'${MY_VAR}'"'"'; MY_VAR=now; print -P "$PS1"'
${MY_VAR}
```

**Where** — `src/ported/prompt.rs::promptexpand`: no
`PROMPT_SUBST` opt check; never calls the param/cmdsub/arith
substitution pre-pass. C-source `Src/prompt.c::promptexpand`
runs `parsestr()` over the prompt when `isset(PROMPTSUBST)` is
true, returning a substitution-applied string that then feeds
the `%`-escape parser.

**Impact** — every dynamic prompt setup that relies on
promptsubst is broken. p10k, starship, oh-my-zsh themes, and
nearly every modern .zshrc use this option to inject git
status, exit code, async results, etc. All of those features
go dead in zshrs:

```sh
# Common .zshrc pattern (broken in zshrs)
setopt promptsubst
PS1='%n@%m $(git_branch_indicator) %# '
# zsh: shows current branch via dynamic cmdsub
# zshrs: shows literal "$(git_branch_indicator)"
```

This is a blocker for daily-driver replacement on the user's
real .zshrc, which uses promptsubst-based dynamic prompt
construction extensively.

**Workaround** — pre-compute prompt content in `precmd` and
assign to a static-content variable; reference it via `%v`
escape in PS1.

---

## #205 — `read -u FD` doesn't read from numeric fd opened via `exec`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'exec 3<<<"line"; read -u 3 x; echo "[$x]"; exec 3<&-'
[line]

$ ./target/debug/zshrs --zsh -c 'exec 3<<<"line"; read -u 3 x; echo "[$x]"; exec 3<&-'
[]
```

`read -u N` reads from fd N. zshrs's `read -u` either reads
from stdin regardless or reads nothing when fd is not stdin/0.

Workaround using stdin works:
```sh
$ both-shells -fc 'exec 3<<<"line"; read x <&3; echo "[$x]"; exec 3<&-'
[line]
```

So the fd-redirect `<&3` path works, but the explicit `-u 3`
flag is broken.

**Where** — `src/ported/builtins/read.rs::handle_u_flag`:
doesn't honor the user-supplied fd. C-source
`Src/builtin.c::bin_read` uses the parsed `-u FD` value to
substitute the fd in the `zread()` call.

**Impact** — explicit-fd read idiom broken. Common in scripts
that pipe between coprocess-like fd setups:

```sh
exec 4< <(some_data_source)
while read -u 4 line; do
    process "$line"
done
# zsh: iterates correctly
# zshrs: read returns empty/EOF on every iteration
```

**Workaround** — use redirect-form `read x <&4` instead of
`read -u 4 x`.

---

## #206 — `setopt multios` doesn't fan-out — only last redirection target receives output

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt multios; echo hi > /tmp/za > /tmp/zb'
$ cat /tmp/za  # zsh
hi
$ cat /tmp/zb  # zsh
hi

$ ./target/debug/zshrs --zsh -c 'setopt multios; echo hi > /tmp/za > /tmp/zb'
$ cat /tmp/za  # zshrs
(empty)
$ cat /tmp/zb  # zshrs
hi
```

`multios` is one of zsh's distinguishing features over POSIX
sh: when set, `cmd > a > b` writes to both `a` and `b` (zsh
forks an internal `tee`-like duplicator). zshrs's redirect
parser accepts the multi-target syntax but the last redirect
wins, exactly like POSIX sh.

Same gap applies to multi-input:
```sh
# Should concatenate: cat < a < b
$ /opt/homebrew/bin/zsh -fc 'setopt multios; cat < /tmp/za < /tmp/zb'
contents-of-a
contents-of-b

$ ./target/debug/zshrs --zsh -c 'setopt multios; cat < /tmp/za < /tmp/zb'
contents-of-b
```

**Where** — `src/ported/exec.rs::setup_redirects`: each new
target overwrites prior fd binding instead of inserting an
internal tee. C-source `Src/exec.c::addfd` checks
`isset(MULTIOS)` and when set, spawns the multio process and
chains fds.

**Impact** — every zsh script that uses the multios idiom for
tee-like output (logging to multiple files without invoking
external `tee`) loses output silently. The data ends up in
only one of the listed targets.

```sh
# Common multios pattern
setopt multios
run_build 2>&1 > build.log > /dev/tty
# zsh: tees output to file AND tty
# zshrs: output goes only to /dev/tty, build.log left empty
```

**Workaround** — pipe through `tee` explicitly:
```sh
run_build 2>&1 | tee build.log
```

---

## #207 — `emulate ksh` doesn't switch arrays to 0-indexed mode

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'emulate ksh; arr=(a b c); echo "[0]=${arr[0]} [1]=${arr[1]}"'
[0]=a [1]=b

$ ./target/debug/zshrs --zsh -c 'emulate ksh; arr=(a b c); echo "[0]=${arr[0]} [1]=${arr[1]}"'
[0]= [1]=a
```

`emulate ksh` should set `KSH_ARRAYS` opt (per zsh manual:
"Emulation modes set options to match the target shell").
zshrs accepts `emulate ksh` without error but doesn't apply
the option bundle — arrays stay 1-indexed.

Direct `setopt KSH_ARRAYS` works in zshrs:
```sh
$ ./target/debug/zshrs --zsh -c 'setopt ksh_arrays; arr=(a b c); echo "[0]=${arr[0]}"'
[0]=a
```

So the option itself works; `emulate ksh` just doesn't set it.

**Where** — `src/ported/emulate.rs::apply_emulation_set`: ksh
emulation table is incomplete — missing `KSH_ARRAYS`,
`KSH_TYPESET`, `KSH_AUTOLOAD`, `BSD_ECHO`, etc. C-source
`Src/options.c::emulate` reads from `emulations[]` table
which has the full bundle.

**Impact** — scripts that rely on `emulate -L ksh` to make a
ksh-targeted script work inside zsh see array-index off-by-one
errors. Same applies to `emulate sh`, `emulate csh` — every
non-zsh emulation mode has incomplete option bundles in zshrs.

```sh
# Ksh script being sourced/eval'd with emulate
emulate -L ksh
for i in $(seq 0 2); do echo "${arr[i]}"; done
# zsh: prints a, b, c (0-indexed via ksh emulation)
# zshrs: prints (empty), a, b (still 1-indexed, off-by-one)
```

**Workaround** — set the specific options explicitly:
```sh
emulate -L ksh
setopt ksh_arrays ksh_typeset bsd_echo
# ...
```

---

## #208 — Function defined inside `(...)` subshell leaks into parent shell

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '(f() { echo defined; }; f); type f 2>&1'
defined
f not found

$ ./target/debug/zshrs --zsh -c '(f() { echo defined; }; f); type f 2>&1'
defined
f is a shell function from zsh
```

In zsh, `(...)` runs in a subshell — state changes inside
(functions, variables, options, current directory, traps) do
not affect the parent. zshrs honors this isolation for plain
variables (control test passes: `(X=value); echo "$X"` → empty
in both shells), but NOT for function definitions.

A function defined inside `(...)` leaks back into the parent.
This is the function analog of "the subshell is not really a
subshell" — likely the subshell is implemented as a saved/
restored state context that doesn't include the function
table.

**Where** — `src/ported/exec.rs::run_subshell`: state save/
restore doesn't snapshot the global function table. C-source
`Src/exec.c::entersubsh` forks (or in `nofork` paths still
saves shfunctab). The Rust impl skips fn-table cloning.

**Impact** — common zsh idioms break in two ways:

1. Helper functions defined for one-time use leak globally:
```sh
process_dir() {
    (
        helper() { ... }  # intended to be scoped
        for x in *; do helper $x; done
    )
}
# zsh: helper gone after process_dir returns
# zshrs: helper visible to all subsequent code
```

2. Subshell function overrides bleed back:
```sh
(
    g() { echo overridden; }  # intended subshell-local override
    g
)
g  # zsh: original; zshrs: still overridden version from subshell
```

**Workaround** — explicit cleanup after subshell:
```sh
(helper() { ...; }; do_stuff)
unfunction helper 2>/dev/null
```

---

## #209 — Alias defined inside `(...)` subshell leaks into parent shell

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '(alias foo=bar); alias foo 2>&1; echo "ec=$?"'
ec=1

$ ./target/debug/zshrs --zsh -c '(alias foo=bar); alias foo 2>&1; echo "ec=$?"'
foo=bar
ec=0
```

Same subshell-isolation gap as #208, but for the alias table.
`(alias foo=bar)` should not register `foo` in the parent
shell's alias table. zshrs registers it permanently.

**Where** — `src/ported/exec.rs::run_subshell`: alias table
not snapshotted on subshell entry. Same fix scope as #208 (the
entire shell state context — funcs, aliases, named refs,
options — needs a proper clone on subshell enter).

**Impact** — alias snippets that intentionally scope short-
lived aliases (common in scripts that do `(alias rm=...; some
work)` to avoid touching interactive rm) leak persistent
aliases:

```sh
# Common zinit/oh-my-zsh pattern
(alias install="zinit install"; install some-plugin)
# zsh: install alias gone after the (...)
# zshrs: install alias permanently in alias table, may shadow real install commands later
```

**Workaround** — explicit `unalias` after the subshell, or use
a function instead of an alias.

---

## #210 — `(unfunction f)` / `(zmodload module)` mutate parent shell state

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { echo body; }; (unfunction f); echo "after"; type f 2>&1'
after
f is a shell function from zsh

$ ./target/debug/zshrs --zsh -c 'f() { echo body; }; (unfunction f); echo "after"; type f 2>&1'
after
f not found
```

`(unfunction f)` in a subshell should remove `f` only within
the subshell. zsh correctly preserves `f` in the parent.
zshrs's subshell `unfunction` mutates the parent function
table — `f` is gone after the subshell.

Same gap exists for `zmodload` (module loading leaks back):
```sh
$ /opt/homebrew/bin/zsh -fc '(zmodload zsh/datetime); zmodload | grep datetime; echo "ec=$?"'
ec=1                              # module not loaded in parent

$ ./target/debug/zshrs --zsh -c '(zmodload zsh/datetime); zmodload | grep datetime; echo "ec=$?"'
zsh/datetime
ec=0                              # module loaded in parent (leak)
```

This is the destructive-mutation surface of the [[#208]]/[[#209]]
family — subshell scope isolation is missing for all shell-
state operations, not just creation. Both add (function/alias
definition) and remove (unfunction) operations leak across.

**Where** — `src/ported/exec.rs::run_subshell`: subshell
context doesn't save/restore the function table, alias table,
or module-registration list. Every mutating builtin executed
in the subshell hits the parent's tables directly.

**Impact** — most destructive of the three:

```sh
# User intent: probe whether a fn is callable without affecting state
(unfunction my_critical_fn; type my_critical_fn) 2>/dev/null
# zsh: my_critical_fn still callable after this line
# zshrs: my_critical_fn permanently gone — critical script function lost
```

Module side: scripts that test-load modules in a subshell to
check availability silently leave the module loaded in the
parent, potentially affecting subsequent option/feature
checks.

**Workaround** — never use `(...)` subshell for state-
mutating operations on zshrs; do explicit save/restore around
the mutation in the parent shell.

---

## #211 — `$-` (current option chars) missing one or more flag letters

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "$-"'
569Xf

$ ./target/debug/zshrs --zsh -c 'echo "$-"'
569X
```

`$-` reports the single-char codes of currently-set shell
options. zsh's no-rcs default outputs `569Xf`. zshrs outputs
`569X` — missing the `f` character.

In zsh's option-letter map, `f` corresponds to one of the
default-on options for non-interactive shells (likely `-f`
itself, i.e. no-rcs-startup mode, which is set by the `-f`
flag we passed). zshrs's `$-` parameter generation omits this
letter.

The leading `569X` are unusual entries that don't map to
typical POSIX option letters either — both shells emit them
but with the `f` divergence, indicating zshrs's option-to-char
table is partial.

**Where** — `src/ported/params/autovars.rs::compute_dash_dollar`:
walks `OPTS` table but doesn't include the `NO_RCS` (`-f`)
letter mapping. C-source `Src/params.c::dashgetfn` iterates
all set opts and emits their option char from `optns[]`.

**Impact** — POSIX-portable scripts that introspect `$-` to
detect shell state (e.g., `case $- in *i*) interactive ;; *)
not ;; esac`) get false negatives for any opt-char zshrs
hasn't mapped:

```sh
case $- in
    *f*) echo "started with -f / norcs" ;;
    *)   echo "interactive or full-startup" ;;
esac
# zsh: prints "started with -f / norcs" when invoked with -f
# zshrs: prints "interactive or full-startup" (false negative)
```

**Workaround** — check option directly via `setopt | grep
norcs` instead of `$-`.

---

## #212 — `${(b)str}` over-escapes non-glob characters (tab)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc $'s=$\'a\\tb\'; echo "${(b)s}"' | od -c | head -1
0000000    a  \t   b  \n

$ ./target/debug/zshrs --zsh -c $'s=$\'a\\tb\'; echo "${(b)s}"' | od -c | head -1
0000000    a   \  \t   b  \n
```

The `(b)` flag is supposed to make a string safe to use as a
glob pattern by escaping glob metacharacters: `* ? [ ] ( ) | <
> ^ # ~ = -`. Tab (`\t`) is NOT a glob metacharacter, so zsh
leaves it unescaped. zshrs inserts a redundant `\` before tab.

The bug is over-escaping — affecting any string containing
non-metachar whitespace or control chars (tab, newline,
carriage return).

**Where** — `src/ported/paramflags.rs::escape_for_pattern`:
escape table is too broad — includes characters that aren't
zsh-glob metachars. C-source `Src/utils.c::quotestring` with
`QT_BACKSLASH | QT_GLOB` only escapes documented metachars.

**Impact** — when `(b)`-escaped strings are concatenated into
patterns, the extra backslashes change pattern semantics:

```sh
suffix=$'\t'      # literal tab
pattern="prefix${(b)suffix}*"
[[ "prefix"$'\t'"x" == $pattern ]] && echo match
# zsh: matches (pattern is "prefix\tab*", real tab in input)
# zshrs: no match (pattern is "prefix\\\t*", requires literal backslash-tab)
```

**Workaround** — for whitespace-safe escaping, use `(q)`
instead, which targets shell quoting (not glob escaping).

---

## #213 — `${assoc[(R)value]}` reverse-search-by-value rejected as bad substitution

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h[a]=1; h[b]=2; h[c]=2; print -l "${(@k)h[(R)2]}"' | sort
b
c

$ ./target/debug/zshrs --zsh -c 'typeset -A h; h[a]=1; h[b]=2; h[c]=2; print -l "${(@k)h[(R)2]}"' 2>&1
zsh:1: bad substitution
```

zsh's `${assoc[(R)value]}` syntax does reverse-search of an
associative array: returns all elements where the *value*
matches the pattern. Combined with `(@k)` it returns the keys
of the matching elements — a common idiom for "find all keys
whose value equals X."

zshrs parses this as an invalid subscript and errors with
"bad substitution."

**Where** — `src/ported/paramsubst.rs::parse_assoc_subscript`:
doesn't recognize `(R)`/`(I)` flags inside an assoc subscript.
C-source `Src/subst.c::dosubst` recognizes `SCANPM_WANTVALS`
when `(R)` flag is present in assoc context and runs the
value-pattern scan.

**Impact** — reverse-search-by-value, key/value scan idioms
fail at runtime. Common patterns broken:

```sh
typeset -A status_map
status_map=(api1 ok api2 fail api3 ok api4 fail)
failed_services=("${(@k)status_map[(R)fail]}")
# zsh: failed_services=(api2 api4)
# zshrs: bad substitution error, script aborts
```

This is a meaningful gap for monitoring/config scripts that
use assoc arrays as ad-hoc lookup tables and want to query
them by value.

**Workaround** — manual loop:
```sh
failed_services=()
for key val in "${(@kv)status_map}"; do
    [[ "$val" == "fail" ]] && failed_services+=("$key")
done
```

---

## #214 — `chpwd` hook and `chpwd_functions` array not fired on `cd`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'chpwd() { echo "cwd: $PWD"; }; builtin cd /tmp; builtin cd /'
cwd: /tmp
cwd: /

$ ./target/debug/zshrs --zsh -c 'chpwd() { echo "cwd: $PWD"; }; builtin cd /tmp; builtin cd /'
(empty)
```

The `chpwd` function — and the array-form `chpwd_functions=(fn1
fn2 ...)` — is supposed to be called automatically after every
successful `cd`. zshrs's `cd` builtin doesn't invoke either.

Array form also broken:
```sh
$ /opt/homebrew/bin/zsh -fc 'my_h() { echo "h: $PWD"; }; chpwd_functions=(my_h); builtin cd /tmp; builtin cd /'
h: /tmp
h: /

$ ./target/debug/zshrs --zsh -c 'my_h() { echo "h: $PWD"; }; chpwd_functions=(my_h); builtin cd /tmp; builtin cd /'
(empty)
```

**Where** — `src/ported/builtins/cd.rs::after_chdir`: doesn't
call the post-cd hook dispatch. C-source `Src/builtin.c::bin_cd`
calls `callhookfunc("chpwd", NULL, 1, NULL)` after a successful
chdir, which fires both the `chpwd` function and walks the
`chpwd_functions` array.

**Impact** — broad daily-driver blocker. `chpwd` is the
standard zsh hook used by:
- `p10k` / `powerlevel10k` for dynamic prompt segments
- `oh-my-zsh` themes (`auto_ls`, project detection, vcs-status
  refresh)
- `zoxide` / `z` / autojump for path-frequency tracking
- `direnv` for `.envrc` discovery on cd
- Virtual-environment auto-activation plugins (pyenv-virtualenv,
  rbenv-aware, etc.)
- Window-title auto-update functions

All of these go dead in zshrs. Visible breakage: the prompt
won't update on directory change, frequented-dirs tracking
breaks, env auto-activation stops working.

**Workaround** — wrap `cd` in a function:
```sh
cd() {
    builtin cd "$@" && {
        [[ -n $functions[chpwd] ]] && chpwd
        for f in "${chpwd_functions[@]}"; do "$f"; done
    }
}
```

---

## #215 — `zshexit` hook function not fired on shell exit

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'zshexit() { echo "EXIT-HOOK"; }; echo "before"; exit 0'
before
EXIT-HOOK

$ ./target/debug/zshrs --zsh -c 'zshexit() { echo "EXIT-HOOK"; }; echo "before"; exit 0'
before
```

The `zshexit` special function — and presumably the array form
`zshexit_functions=(fn1 ...)` — is supposed to run when the
shell exits (similar to `trap EXIT` but a zsh-specific hook
mechanism). zsh fires it; zshrs doesn't.

This is the same shell-event-hook gap as [[#214]] (chpwd), just
on the exit-path side. Likely a single missing dispatch point
in the shell-shutdown sequence.

**Where** — `src/ported/exec.rs::shell_exit` / `main::cleanup`:
no `callhookfunc("zshexit", ...)` invocation. C-source
`Src/init.c::zexit` calls the hook before `_exit()`.

**Impact** — paired with #214, every "hook on every event"
plugin pattern is dead:
- Cleanup-on-exit code (temp file removal, daemon shutdown,
  session-state save)
- Logging shells session-end time for productivity tracking
  (zsh-history-database, etc.)
- `znotify` / `auto-notify` end-of-shell summary plugins

Combined with the `trap EXIT` scope bug ([[#203]]), zshrs's
shell-exit lifecycle is severely degraded — neither the trap-
form nor the hook-form fires correctly.

**Workaround** — use `trap "cleanup_fn" EXIT` at shell
top-level (not inside a fn, per #203).

---

## #216 — `${(t)var:-default}` after `unset var` returns empty instead of falling through to default

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h[k]=v; unset h; echo "[${(t)h:-NONE}]"'
[NONE]

$ ./target/debug/zshrs --zsh -c 'typeset -A h; h[k]=v; unset h; echo "[${(t)h:-NONE}]"'
[]
```

`${(t)var}` returns the type string ("association", "array",
"integer", etc.). For an unset variable, the type lookup
should produce no value — and the `:-NONE` default
substitution should kick in.

zsh returns the default. zshrs returns an empty string but
doesn't apply the `:-` default. This means zshrs's `(t)` flag
returns an empty *value* (not unset state) — the `:-` operator
distinguishes "unset" from "empty string" by colon presence,
and zshrs is producing "empty" not "unset" after the type
lookup of an unset var.

```sh
# Common type-test idiom
[[ "${(t)h:-none}" == "association"* ]] && process_assoc h || error "h is not assoc: ${(t)h:-none}"
# zsh: error path prints "h is not assoc: none"
# zshrs: error path prints "h is not assoc: " (truncated)
```

**Where** — `src/ported/paramflags.rs::apply_t_flag`: when var
is unset, returns `""` (empty string state) instead of
preserving unset state for downstream `:-`/`:=`/`:?` operators.
C-source `Src/subst.c::dosubst` with `PARAMSUBST_TYPESET`
sentinels unset state so subsequent default operators trigger.

**Impact** — defensive type-checking idioms produce
incorrect/empty output when the variable being introspected
is unset. Errors become harder to diagnose because the type
string in the error message is silently empty.

**Workaround** — explicit existence check:
```sh
if (( ${+h} )); then echo "type: ${(t)h}"
else echo "type: none"; fi
```

---

## #217 — `setopt cdable_vars` doesn't allow `cd VAR` to use VAR's value as path

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt cdable_vars; mytmp=/tmp; cd /; cd mytmp; pwd'
/tmp

$ ./target/debug/zshrs --zsh -c 'setopt cdable_vars; mytmp=/tmp; cd /; cd mytmp; pwd'
zsh:cd:1: no such file or directory: mytmp
/
```

`cdable_vars` is a zsh option that makes `cd NAME` (where
NAME is not a literal directory) first check whether NAME is
a parameter holding a directory path — if so, cd to that
path. Common idiom for "named directories" without using
`~name` syntax.

zshrs's `cd` builtin doesn't consult the parameter table for
non-existent paths even with `cdable_vars` set.

**Where** — `src/ported/builtins/cd.rs::resolve_target`: no
`cdable_vars` opt check in the fallback path. C-source
`Src/builtin.c::cd_get_dest` walks parameter table when
target isn't a literal dir and `CDABLEVARS` opt is set.

**Impact** — workflows that use parameter-as-directory-name
patterns (common in dotfile setups: `proj=/Users/x/work; cd
proj`) all break. Same with `~name` style "named directory"
hash table integration via cdable_vars.

```sh
# Common .zshrc pattern
typeset projects=$HOME/code
typeset config=$HOME/.config
setopt cdable_vars
# Now:
cd projects   # zsh: cd's to ~/code; zshrs: errors "no such file"
cd config     # zsh: cd's to ~/.config; zshrs: errors
```

**Workaround** — explicit deref:
```sh
cd "$projects"
```

---

## #218 — `typeset NAME` for assoc array prints nothing (zsh: shows `h=( [k]=v ... )` literal)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h[k]=v; h[k2]=v2; typeset h'
h=( [k]=v [k2]=v2 )

$ ./target/debug/zshrs --zsh -c 'typeset -A h; h[k]=v; h[k2]=v2; typeset h'
(empty)
```

`typeset NAME` (no other flags) should display the variable
in its zsh-syntax assignment form. For an assoc array, that's
the `h=( [k]=v ... )` form. zshrs prints nothing.

`typeset -p h` works in both shells (also tested):
```sh
$ both-shells -fc 'typeset -A h; h[k]=v; typeset -p h'
typeset -A h=( [k]=v )
```

So the `-p` (POSIX-like portable-syntax) path works, just the
bare `typeset h` display path doesn't dispatch the assoc-array
formatter.

**Where** — `src/ported/builtins/typeset.rs::display_var`:
when `-p` flag absent, formats scalars and arrays but assoc
case falls through to an empty branch. C-source
`Src/builtin.c::typeset_single` uses `printparamnode()` which
handles assoc via `print_assoc_var()`.

**Impact** — config-introspection scripts that dump bare
`typeset h` to capture all current state lose all assoc-array
data. Less severe than `-p` form working, but still a parity
gap.

```sh
# Dump everything for debug
for v in "${(@k)parameters}"; do typeset "$v"; done > state.dump
# zsh: state.dump contains all scalars + arrays + assocs
# zshrs: assocs missing from dump
```

**Workaround** — use `typeset -p h` explicitly.

---

## #219 — `typeset -i "h[k]"` integer-typeset on assoc element silently accepted (zsh: rejects)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; typeset -i "h[k]"; h[k]="100"; echo "[${h[k]}]"' 2>&1
zsh:typeset:1: h[k]: inconsistent array element or slice assignment

$ ./target/debug/zshrs --zsh -c 'typeset -A h; typeset -i "h[k]"; h[k]="100"; echo "[${h[k]}]"'
[100]
```

zsh rejects per-element attribute setting on assoc arrays —
attributes (`-i`, `-l`, `-u`, `-r`, etc.) apply to the whole
array, not individual elements. The error message
"inconsistent array element or slice assignment" indicates
zsh's parser/type-checker explicitly disallows this form.

zshrs silently accepts and applies the integer attribute to
the element. Same family as the permissive-parser bugs
(#141/#146/#161/#162/#167/#168/#169/#170/#171/#172/#189) —
zshrs accepts invalid syntax that zsh rejects.

**Where** — `src/ported/builtins/typeset.rs::apply_attrs`:
when subscript is present on target name, doesn't reject the
per-element-attribute form. C-source `Src/builtin.c::typeset_single`
checks `param->node.flags & PM_HASHELEM` and errors via
`zwarnnam("inconsistent array element or slice assignment")`.

**Impact** — scripts that accidentally write
`typeset -i "h[k]"` instead of `typeset -i h` (with intent
to make the whole assoc integer-valued) succeed in zshrs but
fail in zsh — producing divergent runtime behavior. Less
critical than crashes but still a parity gap that affects
script portability.

```sh
# Bug-prone code that "works" in zshrs but errors in zsh
typeset -A scores
typeset -i "scores[alice]"   # zshrs: ok; zsh: error
scores[alice]=95
```

**Workaround** — apply attribute to the whole assoc at
declaration:
```sh
typeset -iA scores
scores[alice]=95
```

---

## #220 — `setopt err_return` doesn't abort function on failed command

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt err_return; f() { false; echo "in-fn"; }; f; echo "after"'
(empty — function aborted on false, return propagated)

$ ./target/debug/zshrs --zsh -c 'setopt err_return; f() { false; echo "in-fn"; }; f; echo "after"'
in-fn
after
```

`err_return` is the zsh-extension equivalent of `set -e` but
scoped to function bodies — when a command returns non-zero
inside a function, the function returns immediately (without
killing the whole shell, unlike `errexit`).

zsh aborts the function on the failed `false`; zshrs ignores
the option and continues executing the function body.

**Where** — `src/ported/exec.rs::run_function_body`: doesn't
check `ERR_RETURN` opt after each command's exit status.
C-source `Src/exec.c::doshfunc` checks `isset(ERRRETURN)` and
sets `retflag` to bubble out of the function call.

**Impact** — "function-scoped strict mode" pattern broken:

```sh
setopt err_return
do_critical_workflow() {
    validate_inputs || return 1   # zsh: redundant with err_return
    fetch_remote
    transform_data
    upload_results
    # zsh: err_return aborts immediately on any failure
    # zshrs: continues through all four even if validate_inputs returns 1
}
```

This is the function-scoped analog of #202 (`set -eo pipefail`
ignored). Same root cause likely — error-propagation flags
not consulted in command-completion path.

**Workaround** — explicit `|| return $?` on every command
inside the function (back to pre-err_return defensive style).

---

## #221 — `disable -f FNAME` doesn't actually disable the function

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'g() { echo body; }; disable -f g; echo "dis=[${dis_functions[g]:-empty}]"; g 2>&1'
dis=[	echo body]
zsh:1: command not found: g

$ ./target/debug/zshrs --zsh -c 'g() { echo body; }; disable -f g; echo "dis=[${dis_functions[g]:-empty}]"; g 2>&1'
dis=[empty]
body
```

`disable -f FN` is supposed to:
1. Move the function out of `functions` and into the
   `dis_functions` table (preserving the body for later
   re-enabling).
2. Make subsequent calls to FN fail with "command not found".

zshrs does neither — the function stays callable, and
`dis_functions[FN]` remains empty.

**Where** — `src/ported/builtins/disable.rs::disable_function`:
doesn't transfer the entry to the disabled-function table.
C-source `Src/builtin.c::bin_enable` (which handles both
`enable`/`disable`) flips the `DISABLED` bit on the shfunc
node and the `parameters` introspection routes the entry to
`dis_functions[]`.

**Impact** — `disable -f` is the standard zsh idiom for
temporarily neutralizing a function (e.g., disabling a heavy
chpwd hook during a bulk-cd operation, or testing a script's
fallback path when a helper is missing). All such patterns
silently no-op:

```sh
disable -f my_slow_hook
heavy_directory_traversal  # zsh: hook doesn't fire; zshrs: still fires
enable -f my_slow_hook
```

Combined with #214 (chpwd doesn't fire anyway), the impact on
chpwd-disable patterns is double-broken — zshrs neither fires
the hook NOR can disable it.

**Workaround** — `unfunction FN` (destructive) and re-`source`
the definition later to "re-enable".

---

## #222 — `zmodload -a` lists no auto-loaded modules (zsh: lists ~27 builtin→module bindings)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'zmodload -a | wc -l'
      27

$ ./target/debug/zshrs --zsh -c 'zmodload -a | wc -l'
       0
```

`zmodload -a` (no args) lists all builtin → module auto-load
bindings. zsh's output:
```
bindkey (zsh/zle)
compadd (zsh/complete)
comparguments (zsh/computil)
...
zstyle (zsh/zutil)
```

zshrs prints nothing — the auto-load binding table is either
empty or the introspection command doesn't dispatch.

**Where** — `src/ported/builtins/zmodload.rs::list_auto_binds`:
either the binding registry is empty (modules are loaded
on-demand without ever pre-registering builtin-to-module
bindings) or the `-a` flag handler doesn't iterate the
registry. C-source `Src/init.c::add_autobindings` populates
the table at startup; `Src/builtin.c::bin_zmodload` with `-a`
iterates it.

**Impact** — config-introspection workflows lose a major piece
of "what features are available" data. Tools like p10k's
optional-feature detector use `zmodload -a` to check whether
a module's builtins (e.g., `vared`, `zformat`, `zparseopts`)
are available before invoking them.

Without this info, plugins can't reliably probe shell
capabilities. The fallback (calling the builtin and catching
the error) is more expensive and uglier.

**Workaround** — try-and-catch:
```sh
if zmodload -e zsh/zutil 2>/dev/null; then
    use_zutil_features
fi
```

(Note: `zmodload -e` for "is this module loadable" may also
need verification — could be a separate gap.)

---

## #223 — `setopt warn_create_global` doesn't emit warning on implicit global creation

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt warn_create_global; f() { X=1; }; f'
f: scalar parameter X created globally in function f

$ ./target/debug/zshrs --zsh -c 'setopt warn_create_global; f() { X=1; }; f'
(empty)
```

`warn_create_global` is one of zsh's "strict-mode" options:
when a function body assigns to a parameter that doesn't
exist in any enclosing scope, zsh emits a warning to stderr
("X created globally in function f"). This catches the common
bug of forgetting `local` inside a function body.

zshrs silently allows the implicit global creation with no
warning.

**Where** — `src/ported/params/assign.rs::assign_in_function`:
no `WARNCREATEGLOBAL` opt check before creating a parameter
at outer scope. C-source `Src/params.c::setsparam` checks
`isset(WARNCREATEGLOBAL)` when `locallevel > 0` and a new
param is being created at level 0.

**Impact** — the strict-mode option that catches accidental
global pollution is dead. Functions that should use `local`
but don't get a free pass:

```sh
setopt warn_create_global
process_records() {
    record_count=0      # zsh: warns about implicit global
    for r in $@; do
        ((record_count++))
    done
}
# zshrs: no warning, record_count silently becomes a global
# affecting subsequent function calls in unexpected ways
```

**Workaround** — `setopt local_options` + audit each function
body manually, or use a lint pass (`zsh -nx script.zsh` won't
catch this either since the option isn't honored).

---

## #224 — `unsetopt typeset_silent` + `typeset X` doesn't print existing value (zsh: prints `X=value`)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'X=1; unsetopt typeset_silent; typeset X'
X=1

$ ./target/debug/zshrs --zsh -c 'X=1; unsetopt typeset_silent; typeset X'
(empty)
```

When `typeset_silent` is unset, declaring/redeclaring an
existing parameter via `typeset NAME` (no value) should print
the current value to stdout (`NAME=value`). zshrs treats
`typeset_silent` as always-on regardless of the option state.

This is referenced in the user's own CLAUDE.md note: *"bare
`local var` (no assignment) re-declaration prints the current
value to stdout when `typeset_silent` is off. With default
`typeset_silent` on, `local var=value` re-declaration is
silent."* — describing exactly the behavior zsh exhibits and
zshrs doesn't.

**Where** — `src/ported/builtins/typeset.rs::redeclare_existing`:
no `TYPESET_SILENT` opt check on the "no assignment, var exists"
path. C-source `Src/builtin.c::typeset_single` checks
`!isset(TYPESETSILENT)` before printing the existing value.

**Impact** — the option exists primarily to make
script-debugging easier (turn off `typeset_silent` to see what
state functions inherit when they declare via bare
`local`/`typeset`). With the option dead, scripts that depend
on this behavior to dump state (or `set -x` reveal) get no
info:

```sh
unsetopt typeset_silent
f() {
    local PATH       # zsh: prints PATH=... when entering f
    PATH=/restricted
    do_stuff
}
# zshrs: nothing printed, no visibility into inherited PATH
```

Lower-impact than #223 but same root cause family — strict-
mode/diagnostic options not consulted by the assignment path.

**Workaround** — explicit `echo "PATH=$PATH"` before
re-declaration when debugging.

---

## #225 — `printf "%(%Y)T\n"` silently treats `%(...)T` as literal (zsh: errors "invalid directive")

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'printf "%(%Y)T\n" 1000000000'
zsh:printf:1: %(: invalid directive
$ echo $?
1

$ ./target/debug/zshrs --zsh -c 'printf "%(%Y)T\n" 1000000000'
%(%Y)T
$ echo $?
0
```

Neither zsh nor zshrs support bash's `%(date-format)T` printf
directive (a bash 4.x extension). But the error-handling
behavior differs:

- zsh errors loudly: "%(: invalid directive", exit code 1.
- zshrs silently echoes the format as literal text, exit
  code 0.

Same permissive-parser family as #141/#146/#161/#162/#167/
#168/#169/#170/#171/#172/#189/#219 — zshrs accepts invalid
input silently, zsh rejects.

**Where** — `src/ported/builtins/printf.rs::parse_directive`:
unknown `%(...)X` directive falls through to "echo literal"
branch instead of erroring. C-source `Src/builtin.c::bin_printf`
calls `zwarnnam("invalid directive: %s")` and returns 1.

**Impact** — scripts ported from bash that try to use
`printf "%(%F)T"` for date formatting get garbage output in
zshrs without any error indication, vs zsh which errors
immediately and exits non-zero (giving the script-author a
chance to notice and replace with `strftime` from zsh/datetime
module).

```sh
log_line=$(printf "%(%F %T)T  %s\n" -1 "$message")
# zsh: errors, script aborts (caller knows to use strftime)
# zshrs: log_line becomes literal "%(%F %T)T  hello message"
# downstream parsing of the "timestamped" log corrupts silently
```

**Workaround** — use the zsh/datetime module's `strftime`
builtin (which both shells support):
```sh
zmodload zsh/datetime
strftime -s log_line "%F %T  %s" $EPOCHSECONDS "$message"
```

---

## #226 — `alias -s SUFFIX=cmd` doesn't populate `saliases` introspection assoc

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'alias -s py=python; echo "saliases=[${saliases[py]}]"; alias -s'
saliases=[python]
py=python

$ ./target/debug/zshrs --zsh -c 'alias -s py=python; echo "saliases=[${saliases[py]}]"; alias -s'
saliases=[]
py=python
```

zsh maintains three parallel introspection assoc arrays:
`aliases` (regular), `galiases` (global, `alias -g`), and
`saliases` (suffix, `alias -s`). zshrs's suffix-alias
registration via the builtin works (`alias -s` lists them
correctly) but doesn't mirror into the `saliases` parameter
introspection table.

**Where** — `src/ported/builtins/alias.rs::define_suffix_alias`:
no insert into the `saliases` parameter table. C-source
`Src/builtin.c::bin_alias` with `-s` flag inserts into the
shared aliases table with `ALIAS_SUFFIX` flag; the
`parameters` introspection routes flagged entries to the
appropriate hash.

**Impact** — plugin frameworks that probe `saliases` for
"what file extensions are registered as suffix aliases" get
empty results, even when `alias -s` shows the aliases working
at runtime:

```sh
# Plugin discovery pattern
for ext in "${(@k)saliases}"; do
    echo "Registered suffix: .$ext → ${saliases[$ext]}"
done
# zsh: iterates py, sh, md, etc.
# zshrs: iterates nothing — plugin sees no suffix aliases
```

**Workaround** — parse `alias -s` output instead of querying
the assoc:
```sh
alias -s | while IFS='=' read suffix cmd; do
    echo "$suffix → $cmd"
done
```

---

## #227 — `disable -a NAME` neither populates `dis_aliases` nor removes from active `aliases` table

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'alias foo=bar; disable -a foo; echo "dis_aliases=[${dis_aliases[foo]}]"; alias foo; foo 2>&1'
dis_aliases=[bar]
zsh:1: command not found: foo

$ ./target/debug/zshrs --zsh -c 'alias foo=bar; disable -a foo; echo "dis_aliases=[${dis_aliases[foo]}]"; alias foo; foo 2>&1'
dis_aliases=[]
foo=bar
zsh:1: command not found: foo
```

Three behaviors should change when `disable -a foo` runs:
1. `dis_aliases[foo]` gets the saved body.
2. `aliases[foo]` is cleared (alias no longer in active table).
3. `foo` becomes uncallable.

zshrs gets (3) right but fails (1) and (2). The alias
silently stays in the active `aliases` table, and the disabled
table is never populated.

Same root cause as #221 (`disable -f` for functions): the
"move to disabled table" transition isn't implemented for
either functions or aliases. zshrs's `disable` flips an
internal flag that affects callability but skips the
table-migration step.

**Where** — `src/ported/builtins/disable.rs::disable_alias`:
flips a disabled bit but doesn't insert into `dis_aliases`
parameter table OR remove from the `aliases` parameter table.
C-source `Src/builtin.c::bin_enable` for `-a` mode flags the
aliasnode and `parameters` introspection routes it.

**Impact** — same as #221 for aliases. Re-enable patterns
break:

```sh
disable -a my_destructive_alias
do_safe_operations
enable -a my_destructive_alias  # zsh: re-enables; zshrs: re-enables but
                                # was never really disabled in introspection
```

Combined with #221 (functions) and the missing `dis_builtins`
behavior (presumably also broken — wasn't tested in this
batch), zshrs's entire `disable`/`enable` round-trip is
table-mirror-broken.

**Workaround** — `unalias NAME` (destructive) and re-define
later from a saved copy.

---

## #228 — `hash -d NAME=path` doesn't populate `nameddirs` introspection assoc

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'hash -d foo=/tmp; echo "nameddirs=[${nameddirs[foo]}]"; echo "~foo"; hash -d'
nameddirs=[/tmp]
~foo
foo=/tmp

$ ./target/debug/zshrs --zsh -c 'hash -d foo=/tmp; echo "nameddirs=[${nameddirs[foo]}]"; echo "~foo"; hash -d'
nameddirs=[]
~foo
foo=/tmp
```

`hash -d` registers a "named directory" — `~foo` then expands
to `/tmp` in path contexts. zshrs's registration works (the
`hash -d` listing shows it, and `~foo` expands correctly), but
the `nameddirs` introspection assoc isn't populated.

Same introspection-table-mirror bug pattern as #226/#227 —
the data structure that backs the builtin works, but the
parameter-introspection mapping is incomplete.

Note: `~foo` itself does NOT expand to `/tmp` in this test —
both shells output the literal `~foo` because echo gets the
unexpanded string (the test echoes the literal text). To see
the expansion both shells need a different context:

```sh
$ both-shells -fc 'hash -d foo=/tmp; cd ~foo; pwd'
/tmp
```

The bug is specifically about the introspection assoc.

**Where** — `src/ported/builtins/hash.rs::register_named_dir`:
inserts into the named-dir table backing `~name` expansion,
but doesn't mirror into `nameddirs` parameter. C-source
`Src/hashtable.c::createnameddir` updates the shared table
that `parameters` introspection reads.

**Impact** — code that introspects registered named
directories to build menus / config dumps / migration scripts
sees no entries:

```sh
# Dump all named dirs for a config backup
for name dir in "${(@kv)nameddirs}"; do
    echo "hash -d $name=$dir" >> ~/.zsh_dirs_backup
done
# zsh: produces backup file with all named dirs
# zshrs: empty backup file
```

**Workaround** — parse `hash -d` output:
```sh
hash -d | while IFS='=' read name dir; do
    echo "$name → $dir"
done
```

---

## #229 — `${(j: :)${(@kv)h}}` join-on-kv drops values (only keys joined)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h[k1]=v1; echo "${(j: :)${(@kv)h}}"'
k1 v1

$ ./target/debug/zshrs --zsh -c 'typeset -A h; h[k1]=v1; echo "${(j: :)${(@kv)h}}"'
k1
```

`${(@kv)h}` produces the assoc array as an interleaved `k v k
v ...` flat array. When this flat array is joined via outer
`(j: :)`, zsh joins all elements (`k1 v1`). zshrs drops the
values during the join — only `k1` makes it through.

The direct iteration form works:
```sh
$ both-shells -fc 'typeset -A h; h[k1]=v1; for k v in "${(@kv)h}"; do print "[$k]=[$v]"; done'
[k1]=[v1]
```

And `print -l "${(@kv)h}"` works:
```sh
$ both-shells -fc 'typeset -A h; h[k1]=v1; print -l "${(@kv)h}"'
k1
v1
```

So `(@kv)` produces the flat array correctly. The bug is in
the nested-flag composition path: when `(@kv)` is the inner
expansion and an outer flag like `(j)` processes the result,
half the elements get dropped.

Likely related family: nested-expansion bugs (#147, #182,
#195) where context propagation through `${...}` boundaries
loses information.

**Where** — `src/ported/paramsubst.rs::nested_expansion`: outer
`(j)` flag re-reads the inner expansion's array but only sees
the keys side. C-source `Src/subst.c::dosubst` preserves the
full interleaved array through the nested-substitution dispatch.

**Impact** — `(j)`-flatten of `(@kv)` is the standard idiom
for serializing an assoc array to a single string. Breaks
config-dump and debug-output patterns:

```sh
typeset -A config=(host example.com port 443 timeout 30)
echo "Config: ${(j: :)${(@kv)config}}"
# zsh: "Config: host example.com port 443 timeout 30"
# zshrs: "Config: host port timeout"  (values dropped, output meaningless)
```

**Workaround** — manual loop to build serialization:
```sh
serialized=""
for k v in "${(@kv)config}"; do
    serialized+="$k $v "
done
```

---

## #230 — `echo "a\0b"` doesn't interpret `\0` as null byte (zsh: outputs literal NUL char)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "a\0b"' | od -c | head -1
0000000    a  \0   b  \n

$ ./target/debug/zshrs --zsh -c 'echo "a\0b"' | od -c | head -1
0000000    a   \   0   b  \n
```

`echo` in zsh interprets a fixed set of backslash escapes by
default (per the `echo`-builtin documentation): `\a`, `\b`,
`\c`, `\e`, `\f`, `\n`, `\r`, `\t`, `\v`, `\\`, `\0NNN`.
zshrs interprets `\t`, `\n`, etc. (the common ones tested
elsewhere work) but not `\0NNN` numeric octal — which should
produce a NUL byte.

Tested control: `echo "a\tb"` works in both shells (both emit
real tab). The bug is specific to the `\0` octal escape form.

**Where** — `src/ported/builtins/echo.rs::interpret_escapes`:
escape table missing `\0NNN` octal sequence. C-source
`Src/builtin.c::bin_echo` calls `getkeystring()` which
handles `\0NNN` as octal byte.

**Impact** — scripts that emit binary data via `echo` (rare
but valid use case) produce wrong output. More commonly,
scripts that emit NUL-separated lists (`printf '%s\0' ...`-like
patterns done with echo) fail to produce parseable output:

```sh
# Emit null-separated for xargs -0
for file in "${(@)files}"; do
    echo "$file\0"
done | xargs -0 process
# zsh: xargs sees NUL-separated args
# zshrs: xargs sees literal "\0" as part of each filename
```

**Workaround** — use `print` (which handles `\0` correctly) or
`printf '\0'`:
```sh
print "a\0b"   # works in zshrs for NUL
```

---

## #231 — `${(t)tied_var}` returns `scalar`/`scalar` instead of `scalar-tied`/`array-tied` (tied pair concept missing)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -T STR str ":"; STR="a:b"; echo "STR=${(t)STR}; str=${(t)str}; n=${#str}"'
STR=scalar-tied; str=array-tied; n=2

$ ./target/debug/zshrs --zsh -c 'typeset -T STR str ":"; STR="a:b"; echo "STR=${(t)STR}; str=${(t)str}; n=${#str}"'
STR=scalar; str=scalar; n=0
```

`typeset -T NAME ARR sep` creates a tied scalar/array pair —
`NAME` is the scalar form (sep-joined), `ARR` is the array
form. The two views are kept in sync (modifying one updates
the other).

Three sub-bugs in zshrs:
1. `${(t)NAME}` returns `scalar` instead of `scalar-tied`.
2. `${(t)ARR}` returns `scalar` (!!) instead of `array-tied`.
3. `${#ARR}` returns 0 instead of 2 — the tied array doesn't
   reflect the scalar's content at all.

(3) means the tying isn't happening — `ARR` is empty even
though `STR` has content. Same family as #24 (tied-array
no-sync) but exposes additional surfaces:
- The type-string introspection (`(t)` flag) loses the `-tied`
  suffix.
- The array-side of the tied pair isn't even recognized as an
  array — typed as `scalar`, which is the default.

**Where** — `src/ported/builtins/typeset.rs::handle_tied_T`:
`-T` flag is parsed and accepted but doesn't establish the
scalar↔array tie at all. C-source `Src/params.c::tiedarrayparam`
creates the linked pair via the `Param->ename` field and
synced get/set methods.

**Impact** — `typeset -T` is the standard zsh idiom for
working with separator-joined env vars (the canonical example
being `PATH`/`path`, `MANPATH`/`manpath`, `FPATH`/`fpath`).
With this bug:

```sh
typeset -T MY_PATH my_path ":"
MY_PATH="/usr/local/bin:/usr/bin:/bin"
echo "First entry: ${my_path[1]}"
# zsh: "/usr/local/bin"
# zshrs: empty (my_path is not actually tied)
```

Combined with #24, every user idiom that depends on `-T` is
broken: indexing into the array, appending to either form,
type-checking, iteration. Compatibility floor for `path`-style
parameters is at risk if `$path[(I)/some/dir]` (a common .zshrc
idiom) doesn't reflect `$PATH` correctly.

**Workaround** — manual sync via convert-on-read:
```sh
my_path=("${(@s/:/)MY_PATH}")
```

---

## #232 — `TRAPEXIT()` named-function form of EXIT trap not fired on shell exit

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'TRAPEXIT() { echo "TRAPEXIT-FIRED"; }; echo "before"; exit 0'
before
TRAPEXIT-FIRED

$ ./target/debug/zshrs --zsh -c 'TRAPEXIT() { echo "TRAPEXIT-FIRED"; }; echo "before"; exit 0'
before
```

zsh recognizes two parallel ways to set up signal trap
handlers:
1. `trap "cmd" SIGNAL` — POSIX-style.
2. `TRAPNAME() { ... }` — zsh extension where a function named
   `TRAP<SIGNAL>` is auto-bound as the handler for that signal.

zshrs supports (1) but not (2) for `EXIT` (and presumably all
other signals — `TRAPINT`, `TRAPUSR1`, `TRAPTERM`, etc.).

This is a different surface from the previously-documented
`zshexit` hook (#215) and `trap EXIT` in fn (#203):
- `trap "..." EXIT` at top-level works.
- `zshexit() { ... }` does NOT work (#215).
- `TRAPEXIT() { ... }` does NOT work (this bug).
- `trap` inside a fn has scope bug (#203).

Three of the four exit-handling mechanisms are broken.

**Where** — `src/ported/builtins/function_def.rs::register_function`:
no special-case detection for `TRAP*` named functions to
auto-bind to the signal-handler table. C-source `Src/signals.c::settrap`
is called from `Src/parse.c::parsefuncname` when a fn matches
the `TRAP<signame>` pattern.

**Impact** — combined with #215 and #203, the zsh signal-
hook system is largely dead. The third common idiom for
shell-exit cleanup (`TRAPEXIT() { rm $tmpfile; }`) joins the
list of broken approaches:

```sh
# Three patterns all broken differently:
zshexit() { echo via_zshexit_fn; }       # #215 — silently doesn't fire
TRAPEXIT() { echo via_trapexit; }        # this bug — silently doesn't fire
trap "echo via_trap_exit" EXIT           # works only at top-level (#203 broken inside fn)
```

For signal handlers other than EXIT (`TRAPINT`, `TRAPTERM`,
etc.), the same bug likely exists — meaning zshrs scripts
can't use the zsh-idiomatic named-fn signal binding for any
signal.

**Workaround** — explicit `trap "..." EXIT` at top level
(survives if not inside a function — per #203).

---

## #233 — `typeset -H VAR=val` then `typeset -p VAR` shows value (zsh: hides via `typeset H`)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -H H=v; typeset -p H'
typeset H

$ ./target/debug/zshrs --zsh -c 'typeset -H H=v; typeset -p H'
typeset H=v
```

`typeset -H` ("hidden") tells zsh not to display the value
when introspecting via `typeset -p` or `set`. The variable
still works normally (`echo $H` returns `v`) but `typeset -p`
elides the value to avoid leaking sensitive data to debug
output.

zshrs's `typeset -p` ignores the `-H` flag and shows the value
anyway.

**Where** — `src/ported/builtins/typeset.rs::print_param_form`:
doesn't check the `PM_HIDEVAL` flag before emitting the
`=value` portion. C-source `Src/builtin.c::printparamnode`
checks `pm->node.flags & PM_HIDEVAL` and emits only the name.

**Impact** — secret-handling code that relies on `-H` to keep
credentials out of `set`/`typeset -p`/`env`-dump output
silently leaks them in zshrs:

```sh
typeset -H DB_PASSWORD=$(read_secret)
# ... script runs ...

# Debug dump for support
typeset -p > /tmp/state.dump
# zsh: DB_PASSWORD shows as "typeset -H DB_PASSWORD" (value hidden)
# zshrs: DB_PASSWORD shows as "typeset -H DB_PASSWORD=actualpasswordhere"
# anyone seeing /tmp/state.dump now has the credential
```

Security-relevant gap — `-H` is specifically designed for
credential isolation in debug/dump paths.

**Workaround** — use `set | grep -v '^DB_PASSWORD='` filter
when dumping, or store secrets in environment variables
explicitly marked for exclusion.

---

## #234 — `typeset +U arr` doesn't disable dedup AND `+=` corrupts the array

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -aU u=(a b); typeset +U u; u+=(a); echo "[${u[@]}] count=${#u}"'
[a b a] count=3

$ ./target/debug/zshrs --zsh -c 'typeset -aU u=(a b); typeset +U u; u+=(a); echo "[${u[@]}] count=${#u}"'
[a] count=1
```

`typeset -U` sets the unique-only (dedup) flag on an array.
`typeset +U` removes it. After `+U`, appending a duplicate
should succeed (array grows with the dup).

zshrs's `+U` not only fails to remove the flag (so dedup still
kicks in on append) but ALSO somehow corrupts the array —
reducing `(a b)` + dup `a` to just `(a)` instead of either
`(a b)` (dedup kept) or `(a b a)` (dedup removed).

Two distinct bugs in one repro:
1. `+U` doesn't clear the dedup attribute.
2. `+=` on a `-U` array with a duplicate destroys the rest of
   the array instead of either keeping it (with dedup) or
   appending (without dedup).

Controls:
```sh
# Regular array append (no -U) — works:
$ both-shells -fc 'u=(a b); u+=(a); echo "[${u[@]}]"'
[a b a]

# -U dedup-on append a dup — works (keeps dedup):
$ both-shells -fc 'typeset -aU u=(a b); u+=(a); echo "[${u[@]}]"'
[a b]
```

The bug is specifically the `+U`-then-append-dup path.

**Where** — `src/ported/builtins/typeset.rs::clear_attr`:
doesn't clear `PM_UNIQUE` when `+U` is applied. PLUS
`src/ported/params/assign.rs::append_to_unique_array`:
incorrect destructive replacement on dup-append. C-source
`Src/params.c::setarrvalue` with `ASSPM_AUGMENT` correctly
merges with dedup applied per the current PM_UNIQUE state.

**Impact** — code that toggles dedup mid-script (set `-U` for
initial dedup, then `+U` to allow duplicates during a later
phase) doesn't work, and worse, silently destroys data. Less
common pattern, but the destructive corruption surface is
serious.

**Workaround** — copy to a new non-unique array:
```sh
typeset -aU u=(a b)
# ... initial dedup phase ...
typeset -a u2=("${u[@]}")     # u2 has no -U flag
u2+=(a)                       # works correctly
```

---

## #235 — `typeset -m "glob"` errors instead of glob-matching parameter names

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'foo1=a; foo2=b; bar=c; typeset -m "foo*"'
foo1=a
foo2=b

$ ./target/debug/zshrs --zsh -c 'foo1=a; foo2=b; bar=c; typeset -m "foo*"'
zsh:typeset:1: not valid in this context: foo*
```

`typeset -m PATTERN` is supposed to operate on parameters
whose names match the given glob pattern — displaying them
(when no other action specified) or applying attribute flags
to all matches. zsh handles this; zshrs rejects the pattern
as "not valid in this context."

The `-m` flag is supported on many builtins for pattern-based
batch operations:
- `unset -m "pattern"` — unset all matching params
- `unalias -m "pattern"` — remove matching aliases
- `unfunction -m "pattern"` — remove matching functions
- `unhash -dm "pattern"` — remove matching named-dirs
- `typeset -m "pattern"` — display/modify matching params
- `disable -m`, `enable -m` — toggle by pattern

zshrs may have similar gaps on the other builtins; the test
batch only exercised `typeset -m`.

**Where** — `src/ported/builtins/typeset.rs::handle_m_flag`:
either the `-m` flag isn't parsed, or pattern-matching against
the parameter table isn't dispatched. C-source
`Src/builtin.c::typeset_single` with `OPT_MFLAG` calls
`scanmatchtable()` over the params hash with the glob pattern.

**Impact** — config dumps that want to inspect all parameters
matching a naming convention break:

```sh
# Common pattern for prefixed config vars
typeset -m "MYAPP_*"   # show all app config
# zsh: dumps every MYAPP_* var
# zshrs: errors out, script aborts (or fails silently if 2>/dev/null)
```

Same impact for batch-cleanup patterns:
```sh
unset -m "TEMP_*"   # cleanup all TEMP_* vars at script end
# Probably also broken if unset -m has the same gap
```

**Workaround** — explicit loop with manual glob:
```sh
for v in "${(@k)parameters[(I)foo*]}"; do
    typeset "$v"
done
```

---

## #236 — `${(@)arr:#}` empty-pattern filter doesn't filter out empty elements

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(x "" y); print -l "${(@)a:#}"'
x
y

$ ./target/debug/zshrs --zsh -c 'a=(x "" y); print -l "${(@)a:#}"'
x
(empty line)
y
```

The pattern `${arr:#pat}` filters out elements matching `pat`.
With an EMPTY pattern (`${arr:#}`), zsh interprets this as
"filter out elements equal to empty string" — empties are
removed. zshrs preserves empty elements.

Common idiom for cleaning sparse arrays:
```sh
arr=("${(@)source_arr:#}")   # remove empty/unset slots
```

**Where** — `src/ported/paramsubst.rs::filter_with_pattern`:
when pattern is empty, doesn't match empty elements. C-source
`Src/subst.c::filtsubstr` treats empty pattern as matching
empty string explicitly.

**Impact** — array-cleaning idiom doesn't compact sparse
arrays. Subsequent indexed iteration sees empty slots:

```sh
read_lines() {
    local IFS=$'\n'
    arr=("${(@f)$(cat file)}")
    arr=("${(@)arr:#}")   # zsh: removes blank lines
                          # zshrs: blank lines kept
    process "${arr[@]}"
}
```

**Workaround** — explicit pattern matching empty:
```sh
arr=("${(@)arr:#(#e)}")   # extended-glob anchor-at-end empty
```

or manual loop:
```sh
new=()
for x in "${arr[@]}"; do [[ -n "$x" ]] && new+=("$x"); done
arr=("${new[@]}")
```

---

## #237 — `${funcfiletrace}` and `${functrace}` format diverges from zsh (missing file path + line)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { echo "ft=${funcfiletrace[1]}"; }; f'
ft=/opt/homebrew/bin/zsh:1

$ ./target/debug/zshrs --zsh -c 'f() { echo "ft=${funcfiletrace[1]}"; }; f'
ft=zsh
```

`funcfiletrace[N]` is supposed to contain
`<file>:<line>` for the source location that called the
function at funcstack level N. zsh produces the full binary
path + line number. zshrs produces just the program name
(`zsh`) with no line number and no path.

Same broken format applies to `${functrace}`:
```sh
$ /opt/homebrew/bin/zsh -fc 'f() { echo "${functrace[1]}"; }; f'
/opt/homebrew/bin/zsh:1

$ ./target/debug/zshrs --zsh -c 'f() { echo "${functrace[1]}"; }; f'
f:1
```

(`functrace` is the function-name version while
`funcfiletrace` is the file-path version — both are broken
but in different ways: zshrs's `functrace` produces `<fn>:<line>`,
zsh produces `<file>:<line>`.)

**Where** — `src/ported/params/autovars.rs::funcfiletrace_get`
and `functrace_get`: format and source-data lookups don't
match zsh. C-source `Src/parameter.c::funcfiletracegetfn`
builds entries from the active funcstack's `caller_file` and
`caller_line` fields.

**Impact** — error-reporting code that uses `funcfiletrace`
to print "called from FILE:LINE" gets wrong info:

```sh
die() {
    echo "ERROR at ${funcfiletrace[1]}: $*" >&2
    exit 1
}
# zsh: prints "ERROR at /path/to/script.zsh:42: ..."
# zshrs: prints "ERROR at zsh: ..." (no useful trace location)
```

Stack-trace plugins (zsh-better-error / zsh-utils style) lose
their source-location info entirely.

**Workaround** — use `${(%):-%x:%I}` prompt-escape form for
script:line if [[#204]] (promptsubst) and the relevant prompt
escapes (#38/#92) are addressed; otherwise no clean
workaround.

---

## #238 — `setopt promptbang` doesn't enable `!` → history-number expansion in prompts

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt promptbang; PS1="!"; print -P "$PS1"'
0

$ ./target/debug/zshrs --zsh -c 'setopt promptbang; PS1="!"; print -P "$PS1"'
!
```

With `promptbang` opt set, a literal `!` in PS1 expands to the
current history event number during prompt expansion. zsh
substitutes the count (0 here since no commands have run);
zshrs leaves the `!` literal.

Same family as #204 (`promptsubst` opt not honored) — prompt-
related option flags broadly ignored. Other prompt-state opts
likely similar (`prompt_subst`, `prompt_percent`,
`prompt_sp`, `prompt_cr`).

**Where** — `src/ported/prompt.rs::expand_prompt`: doesn't
check `PROMPTBANG` opt to decide whether to substitute `!`.
C-source `Src/prompt.c::promptexpand` walks the prompt char-
by-char and substitutes `!` only when `isset(PROMPTBANG)`.

**Impact** — themes/prompts that use `!` for history number
(common in minimal/bash-style PS1) get literal `!` instead of
a number:

```sh
# Common bash-compatible PS1
PS1='\!: '
# Bash: "1: ", "2: ", ...
# Zsh with promptbang: "0: ", "1: ", ...
# zshrs: "!: " on every line (no count)
```

Combined with #204 (promptsubst), all `!`/`$()`/`${}`-based
dynamic-prompt mechanisms are dead in zshrs.

**Workaround** — use `%h` or `%!` prompt-percent escape
instead (both work in both shells when no opt changes are
needed for them — though watch out for the chained prompt-
escape coverage gaps in the #38 family).

---

## #239 — `print -P "%J"` (jobs-count prompt escape) treats as literal

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'print -P "[%J]"'
[]

$ ./target/debug/zshrs --zsh -c 'print -P "[%J]"'
[%J]
```

`%J` is the prompt-escape for "number of currently-active
jobs in the shell." With no jobs running, zsh expands it to
empty. zshrs leaves the literal `%J`.

The bug extends the prompt-escape gap documented in #38
(which lists `%m/%C/%i/%l/%y/%E/%v/%b/%u/%s/%f/%k` as missing).
`%J` joins that list as a separate missing escape.

**Where** — `src/ported/prompt.rs::escape_J`: not implemented.
C-source `Src/prompt.c::promptexpand` maps `%J` to the job
count from `jobtab[]`.

**Impact** — prompts that display job count visually
(common in minimalist PS1 designs: `[%j] $ ` or RPS1 showing
background-job count) show literal `%J`/`%j` instead.

Note `%j` may also be affected (same character class — lower-
case version, same internals). Test batch didn't compare
`%j` explicitly.

**Workaround** — use `${#jobstates}` if `jobstates`
introspection works (uncertain — could be broken per the
introspection-table family).

---

## #240 — `setopt err_exit` + `{ false } always { :; }` doesn't trigger errexit after always block

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc '(setopt err_exit; { false } always { :; }; echo never)'
$ echo $?
1

$ ./target/debug/zshrs --zsh -c '(setopt err_exit; { false } always { :; }; echo never)'
never
$ echo $?
1
```

`{ try } always { cleanup }` is zsh's try-finally construct.
The `cleanup` block runs unconditionally; if the `try` block
errored, the error state should propagate after cleanup
completes — so `errexit` should still fire and abort.

zshrs runs cleanup correctly but clears the error state, so
subsequent statements (here, `echo never`) execute despite
errexit being set.

This is the always-block analog of #202 (`set -eo pipefail`
ignored) and #220 (`err_return` ignored) — error-propagation
flags broadly broken across compound-statement boundaries.

**Where** — `src/ported/exec.rs::run_try_always`: clears
`errflag` after the always block instead of preserving it for
the surrounding errexit check. C-source `Src/exec.c::execlist`
saves errflag before always, restores it after, then checks
`isset(ERREXIT)` post-restore.

**Impact** — strict-mode error handling broken at any
try-always boundary:

```sh
setopt err_exit
{
    risky_operation
} always {
    cleanup_temp_files
}
# zsh: aborts if risky_operation failed (after cleanup)
# zshrs: continues regardless of risky_operation's exit code
do_more_critical_work   # runs even after failure on zshrs
```

This means try-always blocks effectively neuter errexit for
their enclosing scope — defensive scripts that use the
pattern for cleanup paradoxically lose error-handling
guarantees.

**Workaround** — explicit propagation:
```sh
{
    risky_operation || _err=$?
} always {
    cleanup_temp_files
}
[[ -n "$_err" ]] && exit "$_err"
```

---

## #241 — `${assoc[@]}` and `${assoc[*]}` return empty (basic assoc value iteration broken)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h[a]=1; h[b]=2; echo "star=[${h[*]}] at=[${h[@]}]"'
star=[1 2] at=[1 2]

$ ./target/debug/zshrs --zsh -c 'typeset -A h; h[a]=1; h[b]=2; echo "star=[${h[*]}] at=[${h[@]}]"'
star=[] at=[]
```

For a regular (non-assoc) array, `${arr[*]}` and `${arr[@]}`
expand to all elements (joined by IFS or word-split,
respectively). For an associative array, zsh applies the same
semantics — all *values* are returned. zshrs returns empty
for both forms.

The explicit `(@v)` flag form works:
```sh
$ both-shells -fc 'typeset -A h; h[a]=1; h[b]=2; print -l "${(@v)h}"'
1
2
```

So zshrs has the value-iteration mechanism but doesn't trigger
it via the bare `[@]`/`[*]` subscript on assoc arrays.

**Where** — `src/ported/paramsubst.rs::handle_at_star_subscript`:
when the parameter is an assoc array, doesn't dispatch to the
value-collection path. C-source `Src/subst.c::dosubst` checks
`PM_HASHED` flag and treats `[@]`/`[*]` as equivalent to the
`(v)` flag.

**Impact** — the standard idiom for "iterate values of an
assoc" is broken. Most scripts work with values more than
keys (or together as kv pairs), so this is a hot path:

```sh
typeset -A endpoints=(api1 https://x api2 https://y)

# Common pattern: iterate values
for url in "${endpoints[@]}"; do
    health_check "$url"
done
# zsh: iterates the two URLs
# zshrs: iterates zero times — silent no-op
```

Combined with #213 (assoc reverse-by-value errors) and #229
(kv-join drops values), assoc-array support has multiple
foundational gaps.

**Workaround** — use `${(@v)h}` (explicit value flag):
```sh
for url in "${(@v)endpoints}"; do
    health_check "$url"
done
```

---

## #242 — Introspection assocs (`builtins`/`parameters`/`modules`/etc.) not read-only — writes silently accepted

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'builtins[xx]=foo'
zsh:1: read-only variable: builtins
$ echo $?
1

$ ./target/debug/zshrs --zsh -c 'builtins[xx]=foo'
$ echo $?
0
```

`builtins`, `parameters`, `modules`, `aliases`, `dis_aliases`,
`functions`, `commands`, and similar introspection assocs are
marked read-only in zsh — writing to them errors out. They
exist for read-only introspection of internal shell tables.

zshrs allows writes silently. This makes the variables
mutable user state, which:
- Diverges from zsh's protection guarantee.
- May or may not affect the underlying table (the test
  doesn't probe whether the write also corrupts the real
  table — uncertain).

Verified missing read-only marker via `(t)` flag (see #243
companion bug).

**Where** — `src/ported/params/init.rs::register_special_assocs`:
doesn't set the `PM_READONLY` flag on these special params.
C-source `Src/parameter.c::createspecialhash` initializes
each with `PM_READONLY | PM_SPECIAL | PM_HIDE | PM_HIDEVAL`.

**Impact** — silent acceptance of writes to read-only
introspection vars is a security/correctness concern:
1. Scripts that mistakenly write to `aliases[foo]=bar`
   thinking they're defining an alias get no error feedback —
   they succeed at corrupting state and then `alias foo` may
   or may not work.
2. Tests/probes that check write-protection (e.g.
   "verify we can't accidentally pollute parameters table")
   get false negatives in zshrs.

```sh
# Defensive pattern: try to write, expect failure
if ! aliases[CRITICAL]=injected 2>/dev/null; then
    echo "alias table protected"
else
    echo "WARNING: alias table writeable - aborting"
    exit 1
fi
# zsh: prints "alias table protected"
# zshrs: prints "WARNING: ..." — false positive abort
```

**Workaround** — none; depends on zshrs flagging these as
`PM_READONLY` at init.

---

## #243 — `${(t)var}` returns empty for `TIMEFMT` and other special parameters

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "type=${(t)TIMEFMT}; val=[$TIMEFMT]"'
type=scalar; val=[%J  %U user %S system %P cpu %*E total]

$ ./target/debug/zshrs --zsh -c 'echo "type=${(t)TIMEFMT}; val=[$TIMEFMT]"'
type=; val=[%J  %U user %S system %P cpu %*E total]
```

`TIMEFMT` has a valid default scalar value in both shells, but
zshrs's `(t)` flag introspection returns empty — meaning the
type-introspection machinery considers it unset/missing even
though `$TIMEFMT` returns the value.

This affects multiple built-in special parameters. Related to
#216 (`${(t)var:-default}` after unset returns empty) but
exposes a different surface: even on params with non-empty
values, `(t)` returns empty if the param isn't registered
properly in the introspection map.

The introspection-assoc family (`builtins`, `parameters`,
`modules`) also misses the `-readonly` attribute in the type
string:
```sh
$ /opt/homebrew/bin/zsh -fc 'echo "${(t)builtins}"'
association-readonly-hide-hideval-special

$ ./target/debug/zshrs --zsh -c 'echo "${(t)builtins}"'
association-hide-hideval-special   # missing -readonly
```

**Where** — `src/ported/params/init.rs::register_autovar`:
autovars (TIMEFMT, NULLCMD, READNULLCMD, WATCHFMT, etc.) are
populated with default values but not registered in the
parameter-type-introspection map. Special assocs are
registered but missing the `PM_READONLY` flag in their type
string (companion bug to #242).

C-source `Src/parameter.c::paramtypestr` walks the
`Param->node.flags` bits and emits each as a hyphenated
suffix.

**Impact** — type-aware code that decides behavior based on
`(t)` introspection (e.g., "is this an integer? if so use
arith") gets wrong answers:

```sh
case "${(t)$1}" in
    *integer*) echo "Got integer: $(($1 * 2))" ;;
    scalar*)   echo "Got string: $1" ;;
    *)         echo "Unknown type for $1: ${(t)$1}" ;;
esac
# zsh: matches the right branch for TIMEFMT (scalar)
# zshrs: falls into "Unknown type" branch since (t) returns empty
```

**Workaround** — explicit `(( ${+VAR} ))` for existence check,
or check value emptiness — but neither replicates the full
type-string info.

---

## #244 — `${(Z+c+)cmd}` Z-flag word-split with `c` option doesn't split

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'cmd="echo a # comment"; arr=("${(Z+c+)cmd}"); print -l "${arr[@]}"'
echo
a
# comment

$ ./target/debug/zshrs --zsh -c 'cmd="echo a # comment"; arr=("${(Z+c+)cmd}"); print -l "${arr[@]}"'
echo a # comment
```

`${(Z+OPTS+)str}` is the parser-aware word-split form. With
`c` option, comments are preserved as their own token. zsh
produces 3 tokens (`echo`, `a`, `# comment`). zshrs returns
the entire input string as a single token — splitting isn't
happening at all.

The simpler `${(z)cmd}` form also exhibits issues but is
partially functional in some cases. The `(Z+c+)`/`(Z+n+)`/
`(Z+q+)` parameterized forms are uniformly broken (also seen
in batch testing: `Z+n+` for newlines returns same single-
token result).

**Where** — `src/ported/paramflags.rs::parse_Z_options`:
either the `+OPT+` parameter parsing for Z flag is incomplete
or the resulting parsed-options aren't passed to the actual
word-splitter. C-source `Src/utils.c::getoutput` walks the
option chars after `Z+` and passes them as flags to the
parser.

**Impact** — completion-helper scripts and zsh-style command-
line parsers that rely on `(Z+c+)` for safe tokenization of
user input lose the ability to extract command words:

```sh
# Common completion pattern
words=("${(Z+c+)BUFFER}")
cmd=${words[1]}
case "$cmd" in
    git) _git_completion ;;
    *) ;;
esac
# zsh: extracts cmd correctly even with comments in BUFFER
# zshrs: words[1] is the whole BUFFER string, $cmd is the whole input
```

**Workaround** — manual split for simple cases:
```sh
words=(${(s: :)cmd%%\#*})   # strip comment, split on spaces
```

---

## #245 — Extended_glob `(#cN,M)` range-repetition not supported

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extendedglob; [[ "abc" == ?(#c2,5) ]]; echo "ec=$?"'
ec=0

$ ./target/debug/zshrs --zsh -c 'setopt extendedglob; [[ "abc" == ?(#c2,5) ]]; echo "ec=$?"'
ec=1
```

zsh's extended_glob `(#cN,M)` syntax repeats the preceding
pattern between N and M times. The test pattern `?(#c2,5)`
means "any char, 2 to 5 times" — `"abc"` matches (3 chars in
the range).

zshrs's extended_glob accepts the syntax (no parse error) but
the matcher doesn't honor the repetition counter — match
fails.

Bare `(#cN)` form (single count) likely works (not tested
here in this batch). The bug is the comma-separated range
form specifically.

**Where** — `src/ported/glob/extended.rs::parse_hash_c_range`:
parses `(#cN,M)` but doesn't apply the count to the
quantifier. C-source `Src/glob.c::patcompswitch` reads N and
M and emits a `P_COUNT` opcode for the matcher.

**Impact** — range-quantified patterns common in
file-validation scripts don't match:

```sh
# Match identifiers with 3-12 alphanumeric chars
setopt extendedglob
case "$input" in
    [[:alnum:]](#c3,12)) echo "valid id" ;;
    *) echo "invalid: $input" ;;
esac
# zsh: matches "abc", "abcdef", etc. — rejects "ab" and "abcdefghijklmnop"
# zshrs: never matches the range — falls through to invalid branch always
```

**Workaround** — explicit repetition with character classes:
```sh
case "$input" in
    [[:alnum:]][[:alnum:]][[:alnum:]]*) echo "at least 3 alnum" ;;
esac
```

(no clean upper-bound check without the range form.)

---

## #246 — `setopt rc_expand_param` applied inside double quotes (zsh: unquoted-only)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt rc_expand_param; a=(1 2); echo "prefix$a"'
prefix1 2

$ ./target/debug/zshrs --zsh -c 'setopt rc_expand_param; a=(1 2); echo "prefix$a"'
prefix1 prefix2
```

`rc_expand_param` makes `prefix$array suffix` expand to
`prefix1 suffix prefix2 suffix` — but ONLY when the array
reference is in *unquoted* context.

Inside double quotes, normal scalar-concat semantics apply
(no per-element distribution). zsh respects this. zshrs
applies rc_expand_param even inside double quotes.

Control:
```sh
# Both shells produce same output without quotes:
$ both-shells -fc 'setopt rc_expand_param; a=(1 2); echo prefix$a'
prefix1 prefix2

# Both shells produce same output without the opt:
$ both-shells -fc 'a=(1 2); echo "prefix$a"'
prefix1 2
```

The bug is only in the `setopt rc_expand_param` × `"$a"`
intersection — zshrs incorrectly applies the per-element
distribution even inside the quotes.

**Where** — `src/ported/exec.rs::expand_word`: rc_expand_param
check fires regardless of quote context. C-source
`Src/glob.c::xpandredir` only applies the rcexpand
transformation when the param ref is *not* inside double
quotes (tracked via expansion-context flags).

**Impact** — scripts that intentionally suppress
rc_expand_param via double-quoting (the canonical "I want
the array as a single space-joined string" pattern) get
silently distributed:

```sh
setopt rc_expand_param
files=(a.txt b.txt c.txt)
log "Processing files: $files"   # intended: "Processing files: a.txt b.txt c.txt"
# zsh:   logs "Processing files: a.txt b.txt c.txt" (one message)
# zshrs: logs three messages: "Processing files: a.txt", "Processing files: b.txt", "Processing files: c.txt"
```

The bug is high-impact for any user `.zshrc` that uses
`rc_expand_param` (it's a popular `rc-shell`-compat option)
because every prior double-quoted array reference now
silently fans out.

**Workaround** — use `${a[*]}` (star) which forces scalar-
join regardless of `rc_expand_param`:
```sh
log "Processing files: ${a[*]}"
```

---

## #247 — `read line` doesn't strip leading/trailing IFS whitespace

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "  hello  " | { read line; echo "[$line]"; }'
[hello]

$ ./target/debug/zshrs --zsh -c 'echo "  hello  " | { read line; echo "[$line]"; }'
[  hello  ]
```

POSIX `read` semantics: words are split on IFS characters,
with leading/trailing IFS whitespace removed. zsh implements
this — `[hello]` with no padding spaces. zshrs preserves the
whitespace as part of the assigned value.

This affects every interactive prompt parsing pattern and
every config-file reader that loops over `read line`:

```sh
while read line; do
    [[ -z "$line" ]] && continue
    process "$line"
done < config.txt
# zsh: lines with trailing whitespace trim cleanly
# zshrs: trailing whitespace becomes part of "$line", potentially breaking
#        path-as-key lookups or comparison-based logic
```

**Where** — `src/ported/builtins/read.rs::strip_ifs`: doesn't
strip leading/trailing IFS chars before assigning to the
variable. C-source `Src/builtin.c::bin_read` calls
`zwsplit_for_read()` which handles IFS-whitespace stripping
per POSIX.

**Impact** — broad — every script that reads user input or
config-file lines may see unexpected leading/trailing
whitespace in the captured value, affecting downstream
processing.

**Workaround** — explicit strip after read:
```sh
read line
line="${line## }"   # strip leading spaces
line="${line%% }"   # strip trailing spaces
```

Note: this still doesn't handle multi-char IFS whitespace
(tabs, etc.) — that needs a more comprehensive trim.

---

## #248 — `read "?prompt" var` prints prompt to stdout when stdin is not a tty

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "ans" | read "?Question: " line; echo "[$line]"'
[ans]

$ ./target/debug/zshrs --zsh -c 'echo "ans" | read "?Question: " line; echo "[$line]"'
Question: [ans]
```

`read "?prompt" var` displays `prompt` to stderr/tty before
reading the variable. zsh detects non-tty stdin and skips the
prompt display (the prompt is meant for interactive use only).

zshrs prints the prompt regardless, AND prints it to stdout
(not stderr) — making it appear in the captured output of
the pipeline.

Two sub-bugs:
1. Should suppress prompt when stdin isn't a tty.
2. Should write prompt to stderr, not stdout (even when
   displayed).

**Where** — `src/ported/builtins/read.rs::handle_prompt_arg`:
unconditionally writes to fd 1 (stdout). C-source
`Src/builtin.c::bin_read` checks `isatty(0)` before
attempting to display the prompt, and writes to `shfderror`
(fd 2).

**Impact** — scripts that wrap `read "?prompt"` in pipes
(e.g., `expect`-like automation, test harnesses, scripted
deployments) get the prompt text mixed into their captured
output:

```sh
result=$(echo "yes" | read "?Confirm: " ans; echo "$ans")
# zsh:   result = "yes"
# zshrs: result = "Confirm: yes" — prompt contaminates result
```

The stdout-vs-stderr aspect also affects interactive use:
shell redirection of prompts (`script 2>/dev/null`) doesn't
suppress them in zshrs.

**Workaround** — separate the prompt from the read:
```sh
[[ -t 0 ]] && print -n "Confirm: " >&2   # only print when interactive
read ans
```

---

## #249 — `${arr//pat/repl}` on bare array applies replacement per-element instead of scalar-join (zsh: joins first)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'a=(abc def); echo "[${a//?/X}]"'
[XXXXXXX]

$ ./target/debug/zshrs --zsh -c 'a=(abc def); echo "[${a//?/X}]"'
[XXX XXX]
```

For bare array reference `$a` (no `[@]` or `[*]` subscript)
in a scalar context (inside `"[...]"` here, since echo gets a
single argument), zsh joins the array with IFS into a scalar
first, then applies the substitution. Result: 6 chars + 1
space = 7 chars, all replaced with X → `XXXXXXX`.

zshrs applies the substitution per-element first, then joins:
`abc → XXX`, `def → XXX`, joined → `XXX XXX`.

Semantically different: zshrs's version preserves array
structure through the substitution; zsh flattens first.

For comparison, both shells handle the explicit `[@]` form
the same way (per-element):
```sh
$ both-shells -fc 'a=(abc def); echo "[${a[@]//?/X}]"'
[XXX XXX]
```

So the bug is specifically the no-subscript scalar-context
path treating array as still-array.

**Where** — `src/ported/paramsubst.rs::dispatch_array_in_scalar`:
when an array is referenced without `[@]`/`[*]` in scalar
context, the replacement should happen on the joined scalar
form, not per-element. C-source `Src/subst.c::dosubst`
checks the SCALAR_CTX flag and joins via `getsparam()` before
applying the replacement.

**Impact** — moderate. Most scripts use explicit `[@]`/`[*]`
forms, but the bare `$arr` form is common in older or
quickly-written code. Substitution results differ subtly:

```sh
# Replace any character with .
log_line="$arr"
log_clean="${log_line//[^a-z]/.}"   # mask non-alpha for logging
# zsh: replacement applied to space-separated joined string
# zshrs: replacement applied per-element, spaces preserved structurally
```

**Workaround** — explicit `[*]`:
```sh
log_clean="${${arr[*]}//[^a-z]/.}"
```

---

## #250 — `typeset -ia` / `typeset -Fa` silently accepted (zsh: rejects "inconsistent type for assignment")

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -ia arr=(10 20 30); echo "[${arr[2]}]"'
zsh:typeset:1: arr: inconsistent type for assignment
$ echo $?
1

$ ./target/debug/zshrs --zsh -c 'typeset -ia arr=(10 20 30); echo "[${arr[2]}]"'
[20]
$ echo $?
0
```

zsh treats `-i` (integer) and `-a` (array) as mutually
exclusive at the variable level — you can't have both at
once. The error message "inconsistent type for assignment"
indicates the type system explicitly rejects mixing scalar-
type modifiers (`-i`, `-l`, `-u`, `-F`, `-E`) with array/assoc
attributes (`-a`, `-A`).

zshrs silently accepts. Same family as #219 (`typeset -i
"h[k]"` per-element attr on assoc) — zshrs's typeset doesn't
enforce zsh's attribute-conflict rules.

Same bug applies to `-Fa` (float array):
```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -Fa farr=(1.5 2.5)' 2>&1
zsh:typeset:1: farr: inconsistent type for assignment

$ ./target/debug/zshrs --zsh -c 'typeset -Fa farr=(1.5 2.5); echo "[${farr[1]}]"'
[1.5]
```

**Where** — `src/ported/builtins/typeset.rs::validate_attr_combo`:
no rejection of conflicting type-attribute combinations.
C-source `Src/builtin.c::typeset_single` checks the flag bits
for `(PM_INTEGER|PM_EFLOAT|...) & (PM_ARRAY|PM_HASHED)` and
errors when both are present.

**Impact** — scripts ported from bash (which doesn't have
these constraints in the same way) may try to declare typed
arrays. In zsh, that's a hard error caught at declaration. In
zshrs, the declaration succeeds and the array stores
seemingly-typed values, but the integer/float coercion is
applied differently (or not at all) — subtle behavior
divergence that's hard to trace later.

```sh
# Bash-portable but illegal-in-zsh declaration
typeset -ia counts=(10 20 30)
counts[2]="not a number"
# zsh: would have errored at declaration, never reaches the bad assignment
# zshrs: stored as-is, scripts downstream get unexpected non-integer in counts[2]
```

**Workaround** — declare array first, then per-element
integer attribute (if needed) via separate assignment with
arith forcing:
```sh
typeset -a counts=(10 20 30)
(( counts[1] = counts[1] ))   # forces integer evaluation per element
```

---

## #251 — `command -- echo hi` doesn't recognize `--` end-of-options separator

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'command -- echo hi'
hi

$ ./target/debug/zshrs --zsh -c 'command -- echo hi'
zsh:1: command not found: --
$ echo $?
127
```

`command -- ARG...` is POSIX-defined: the `--` marker
indicates "all following args are command + args, ignore any
options that look like flags". zsh handles it; zshrs treats
`--` as a literal command name.

This is a common pattern when the command-to-execute might
start with a `-`:
```sh
command -- "$user_provided_cmd" "$@"
# zsh: safely runs the user's command even if cmd name starts with -
# zshrs: tries to execute "--" as a command, fails with 127
```

**Where** — `src/ported/builtins/command.rs::parse_opts`:
doesn't terminate option parsing on `--`. C-source
`Src/builtin.c::bin_command` parses flags via standard
`getopt`-style loop that recognizes `--` as the option
terminator.

**Impact** — defensive scripts that wrap user input through
`command --` for safety lose the safety. Bare `command CMD`
without `--` still works (and is more common), so the bug
only surfaces in scripts written for paranoid input handling
or POSIX-portable style.

**Workaround** — drop the `--`:
```sh
command "$user_cmd" "$@"
```

(Trades safety for compatibility — `$user_cmd` starting with
`-` will be parsed as command flags.)

---

## #252 — `exec -` (dash with no command) errors instead of being a no-op

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'exec -; echo after'
after

$ ./target/debug/zshrs --zsh -c 'exec -; echo after'
zshrs: exec: -: not found
$ echo $?
127
```

`exec -` (or `exec - cmd`, or `exec -l cmd`, etc.) is the
zsh form of "exec with login shell convention": the dash
either marks the next arg as login-style or, when used alone
with no command, is a no-op. zsh treats bare `exec -` as a
no-op and continues script execution; zshrs treats `-` as a
command name to exec, fails to find it, and errors with
"command not found".

**Where** — `src/ported/builtins/exec.rs::parse_args`:
doesn't recognize bare `-` as a flag marker. C-source
`Src/builtin.c::bin_exec` checks `ops[OPT_DASH]` (`-` flag)
and either applies the login-shell semantics or treats it as
no-op when there's no following command.

**Impact** — POSIX/zsh-portable scripts that use `exec -` as a
guard or marker (uncommon but valid) fail in zshrs. The
script-killing exit-code 127 makes the failure hard to miss
but unhelpful — `exec -` is supposed to be safe to call.

```sh
# Defensive: ensure we're in login-shell-like state for nested exec
exec -   # no-op marker
exec my_cmd "$@"
# zsh: runs my_cmd as planned
# zshrs: errors before reaching my_cmd, script aborts
```

**Workaround** — remove `exec -` (it's a no-op in zsh and
unnecessary):
```sh
exec my_cmd "$@"
```

---

## #253 — `$0` inside sourced file isn't updated to sourced-file path (at top-level scope of the file)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ cat > /tmp/zs0.sh <<'EOF'
echo "outside: [$0]"
f() { echo "in-fn: [$0]"; }
f
EOF

$ /opt/homebrew/bin/zsh -fc 'source /tmp/zs0.sh'
outside: [/tmp/zs0.sh]
in-fn: [f]

$ ./target/debug/zshrs --zsh -c 'source /tmp/zs0.sh'
outside: [/Users/wizard/RustroverProjects/zshrs/target/debug/zshrs]
in-fn: [f]
```

When zsh sources a file, `$0` inside the file's top-level
scope reflects the sourced file's path. zshrs leaves `$0` as
the original shell binary path.

Inside functions defined in the sourced file, `$0` is the
function name in BOTH shells (correct). The bug is only the
top-level scope context.

Same gap applies to `.` (dot-source) form:
```sh
$ both-shells -fc '. /tmp/zs0.sh'
outside: [/tmp/zs0.sh]   # zsh
outside: [...zshrs]      # zshrs (same bug)
```

**Where** — `src/ported/builtins/source.rs::run_sourced_file`:
doesn't push a new value onto the `argv0` stack for the
duration of the source. C-source `Src/exec.c::source`
temporarily sets `argv0 = filename` and restores on exit.

**Impact** — common idioms in `.zshrc` / plugin files that
introspect `$0` to find the file's own directory break:

```sh
# Plugin discovery pattern — VERY common in zsh ecosystem
plugin_dir="${0:A:h}"   # directory of this sourced file
source "$plugin_dir/helpers.zsh"
# zsh: plugin_dir = directory of the sourced .zsh file
# zshrs: plugin_dir = directory of the zshrs binary — wrong!
```

This is a major plugin-compatibility blocker — most zsh
plugins (oh-my-zsh themes, zinit-loaded plugins, prezto
modules, p10k) use `${0:A:h}` or similar to find their
co-located resources. With this bug, every plugin will fail
to locate its own resource files.

**Workaround** — pass file path explicitly via parameter:
```sh
plugin_init() {
    local plugin_dir="${1:A:h}"
    # ... use plugin_dir ...
}
plugin_init "$1"   # caller passes the source path
```

(Awkward — requires plugin API change.)

---

## #254 — `UID=N` / `EUID=N` writes silently accepted (zsh: attempts setuid syscall)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'UID=999'
zsh:1: failed to change user ID: operation not permitted
$ echo $?
1

$ ./target/debug/zshrs --zsh -c 'UID=999'
$ echo $?
0
```

zsh's `UID` and `EUID` parameters are special — writing to
them attempts to change the actual user ID via `setuid()` /
`seteuid()` syscalls. zshrs treats them as plain writable
parameters and silently accepts assignments without any
side effect.

Same likely gap for `GID`/`EGID` (group IDs).

**Where** — `src/ported/params/uid.rs::uid_setfn`: assignment
hook doesn't call the system setuid functions. C-source
`Src/params.c::uidsetfn` calls `setuid(value)` or
`seteuid(value)` and errors if it fails.

**Impact** — security-sensitive code that relies on the
setuid behavior is broken:
1. Privilege-drop patterns (`EUID=$UID` to drop suid escalation)
   silently no-op — script keeps elevated privileges.
2. Setuid-related sanity checks (`UID=X || handle_error`)
   pass without doing anything in zshrs.

```sh
# Privilege-drop pattern
if (( EUID == 0 && UID != 0 )); then
    EUID=$UID   # drop privilege
    do_safer_work
fi
# zsh: drops privilege via seteuid
# zshrs: no-op, do_safer_work still runs with elevated EUID
```

Real security gap if anyone uses zshrs for setuid scripts.

**Workaround** — use external `setuid` C wrapper or `su -c`
to drop privileges instead of relying on `UID=`/`EUID=`
assignment.

---

## #255 — `local -h NAME` doesn't hide parent scope's value (`-h` flag ignored)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { local -h PATH; echo "[${PATH}]"; }; f'
[]

$ ./target/debug/zshrs --zsh -c 'f() { local -h PATH; echo "[${PATH}]"; }; f'
[/opt/homebrew/anaconda3/bin:/Users/.../bin:...]
```

`local -h NAME` creates a fresh local parameter named NAME
that *hides* the outer-scope value. Default zsh behavior
(without `-h`) inherits the outer value into the local. With
`-h`, the inherited value is suppressed — the local starts
unset/empty.

zshrs ignores the `-h` flag — the local PATH inherits the
parent's value as if `-h` wasn't specified.

This is the function-local analog of `typeset -H` (hidden in
typeset -p output, see #233) but specifically for the
inheritance behavior on function entry.

**Where** — `src/ported/builtins/typeset.rs::handle_h_flag`:
local declaration with `-h` doesn't initialize the new param
empty. C-source `Src/builtin.c::typeset_single` checks
`OPT_HIDE` and sets `PM_HIDE` flag which suppresses outer-
scope inheritance via `PM_UPPERS_OFF`.

**Impact** — sandboxing patterns broken. Common use:

```sh
# Run a subscript with a clean PATH
run_isolated() {
    local -h PATH
    local -h LD_LIBRARY_PATH
    PATH="/sbin:/bin:/usr/bin"
    # ... do work ...
}
# zsh: $PATH inside fn is exactly what we set, no inherited extras
# zshrs: $PATH inside fn starts with parent's full PATH inherited, then prepended
```

Common in tooling scripts that want hermetic environments
(test runners, CI helpers, security-sensitive code paths).

**Workaround** — explicit overwrite without `-h`:
```sh
run_isolated() {
    local PATH=""
    PATH="/sbin:/bin:/usr/bin"   # any prior value lost via explicit assignment
}
```

---

## #256 — `zstyle -e PATTERN KEY CODE` dynamic-eval form not honored

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'zmodload zsh/zutil; zstyle -e ":x" key "reply=(dynamic)"; zstyle -a ":x" key arr; echo "[${arr[1]}]"'
[dynamic]

$ ./target/debug/zshrs --zsh -c 'zmodload zsh/zutil; zstyle -e ":x" key "reply=(dynamic)"; zstyle -a ":x" key arr; echo "[${arr[1]}]"'
[]
```

`zstyle -e CONTEXT STYLE CODE` registers a style value computed
dynamically at lookup time — the `CODE` runs when the style
is queried and its result (via `$reply` array) is returned as
the style's value. Used heavily by completion-system and
prompt-theme code for context-aware values.

zshrs's `zstyle -e` either doesn't run the code on lookup or
runs it without exposing `$reply` to the caller — retrieval
via `zstyle -a` returns empty.

The plain `zstyle CONTEXT STYLE VALUE` (no `-e`) static form
likely works (#73 family — would need separate verification).

**Where** — `src/ported/builtins/zstyle.rs::register_dynamic`:
flag `-e` is recognized but the deferred evaluation
mechanism doesn't dispatch on lookup OR the lookup path
doesn't exec the registered code block. C-source
`Src/Modules/zutil.c::bin_zstyle` stores the code body in the
style entry with a deferred-eval marker; the lookup path
runs it via `execstring()` and harvests `reply[]`.

**Impact** — completion-system breakage. Common pattern:

```sh
# Set completion menu colors dynamically based on current context
zstyle -e ':completion:*' menu 'reply=("yes select=long")'
zstyle -e ':completion:*' file-list 'reply=("all")'
# zsh: dynamic eval works, completions get correct menu config
# zshrs: queries return empty, completions use defaults or fail
```

Also breaks p10k's many `-e`-based style definitions for
prompt segments. This is a daily-driver blocker for any setup
using modern completion or prompt themes.

**Workaround** — use static `zstyle` if the value is known
at registration time:
```sh
zstyle ':completion:*' menu 'yes select=long'
```

(Loses the dynamism — values frozen at zshrc-eval time.)

---

## #257 — `jobtexts[N]` introspection assoc not populated for running background jobs

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'sleep 0.5 & sleep 0.05; echo "jobtexts1=[${jobtexts[1]}]"; wait 2>/dev/null'
jobtexts1=[sleep 0.5]

$ ./target/debug/zshrs --zsh -c 'sleep 0.5 & sleep 0.05; echo "jobtexts1=[${jobtexts[1]}]"; wait 2>/dev/null'
jobtexts1=[]
```

`jobtexts[N]` is supposed to return the literal command line
of background job N. zsh populates it; zshrs returns empty
for every job.

Same family of "introspection-table-mirror" bugs as #226
(`saliases`), #227 (`dis_aliases`), #228 (`nameddirs`),
#243 (`(t)TIMEFMT` empty type). Jobs are tracked correctly
internally (the `wait` works, `jobs` command shows them) but
the parameter-introspection table `jobtexts` (also presumably
`jobstates`, `jobdirs`) isn't kept in sync.

**Where** — `src/ported/jobs/register.rs::start_bg_job`:
doesn't insert the job's command text into the `jobtexts`
parameter table. C-source `Src/jobs.c::makerunning` updates
all three job-tracking parameter assocs (`jobtexts`,
`jobstates`, `jobdirs`) when adding a new job.

**Impact** — code that introspects current jobs via parameter
interface (vs the `jobs` command output) gets empty data:

```sh
# Decorate prompt with running job count and current names
for n in "${(@k)jobtexts}"; do
    echo "[$n: ${jobtexts[$n]}]"
done
# zsh: lists all active jobs with their command lines
# zshrs: silent (no entries in the assoc)
```

Tools that hook into shell-state for job tracking
(notification daemons, status-bar plugins) see empty job
state.

**Workaround** — parse `jobs` output:
```sh
jobs -l | while read jobline; do echo "$jobline"; done
```

---

## #258 — `printf "%d" $hugenum` silently overflows to 0 (zsh: errors "number truncated after 19 digits")

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'printf "%d\n" 99999999999999999999 2>&1 | head -1'
zsh:1: number truncated after 19 digits: 99999999999999999999

$ ./target/debug/zshrs --zsh -c 'printf "%d\n" 99999999999999999999 2>&1 | head -1'
0
```

For numbers exceeding int64 range, zsh emits a warning to
stderr and refuses the conversion. zshrs silently writes `0`
to stdout without indicating the overflow.

This is a silent-data-corruption bug — code that processes
the printf output sees 0, has no way to distinguish from a
legitimate zero input, and propagates the wrong value.

**Where** — `src/ported/builtins/printf.rs::convert_to_int`:
catches the overflow but doesn't emit a warning or set exit
code. C-source `Src/builtin.c::bin_printf` calls
`zwarnnam("number truncated after 19 digits: %s")` and the
return value is non-zero.

**Impact** — data-processing scripts that handle large
integers from external sources (epoch nanoseconds, file
sizes, IDs from large databases) silently get 0 instead of
overflow warnings:

```sh
# Parse byte count from external API
size=$(curl -s api/file_info | jq .size_bytes)
printf "Size in bytes: %d\n" "$size"
# zsh: emits warning if size exceeds int64, leaves clear signal in stderr
# zshrs: prints "Size in bytes: 0" for large files — silently wrong
```

Combined with #225 (printf %( silently treats as literal),
the `printf` builtin has multiple data-integrity gaps where
zsh's defensive checks are missing.

**Workaround** — pre-validate range:
```sh
[[ "$size" -gt 9223372036854775807 ]] && { echo "size overflow" >&2; exit 1; }
printf "Size: %d\n" "$size"
```

---

## #259 — `${jobstates[N]}` and `${jobdirs[N]}` introspection assocs not populated

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'cd /tmp; sleep 0.5 & sleep 0.05; echo "states=[${jobstates[1]}] dirs=[${jobdirs[1]}]"; wait 2>/dev/null'
states=[running:+:19848=running] dirs=[/tmp]

$ ./target/debug/zshrs --zsh -c 'cd /tmp; sleep 0.5 & sleep 0.05; echo "states=[${jobstates[1]}] dirs=[${jobdirs[1]}]"; wait 2>/dev/null'
states=[] dirs=[]
```

Companion bug to #257 (`jobtexts` empty). All three job-
introspection assocs share the same population path in zsh:

- `jobtexts[N]` — command text of job N (#257)
- `jobstates[N]` — state string (`running:+:PID=running`)
- `jobdirs[N]` — directory job was started in

All three are empty in zshrs even when the underlying job is
tracked (the `wait` works, `jobs` command shows the job).

**Where** — `src/ported/jobs/register.rs::start_bg_job`: same
location as #257 — `jobtexts/jobstates/jobdirs` parameter
tables aren't updated. C-source `Src/jobs.c::makerunning`
fans out to all three.

**Impact** — same family as #257 — any tool that reads job
state through the parameter interface (vs the `jobs` command)
sees empty data. Status-bar plugins, async-job-completion
notifiers, and shell-state inspectors all blind to job state.

Specifically additional impact for `jobstates`: format
`"<status>:<curr/prev>:<pid>=<status>"` is parseable by
plugins for fine-grained job introspection — replacing this
with a `jobs -l` parse loses the PID and current/previous
markers.

**Workaround** — same as #257 — parse `jobs -l` output.

---

## #260 — `${widgets[NAME]}` zle widget introspection assoc not populated (386 widgets missing)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "self-insert=[${widgets[self-insert]}]"; echo "count=$(echo "${(@k)widgets}" | wc -w)"'
self-insert=[builtin]
count=     386

$ ./target/debug/zshrs --zsh -c 'echo "self-insert=[${widgets[self-insert]}]"; echo "count=$(echo "${(@k)widgets}" | wc -w)"'
self-insert=[empty]
count=       0
```

`widgets` is the parameter-introspection assoc that maps zle
widget names to their definition. zsh has ~386 built-in
widgets pre-registered (`self-insert`, `backward-delete-char`,
`expand-or-complete`, etc.). zshrs has 0 in the assoc.

Per the existing #198 (`bindkey -L` divergence) and #214
(`chpwd` hook not firing), the broader zle subsystem has
multiple introspection/integration gaps. This is another
surface.

**Where** — `src/ported/zle/init.rs::register_builtin_widgets`:
widget table is populated for binding lookup (since `bindkey`
can bind keys to widget names that exist), but the parameter-
introspection `widgets` assoc is empty. C-source
`Src/Zle/zle_main.c::printbinding` reads from the same
internal table that `parameters` introspection routes to
`widgets`.

**Impact** — zle plugins that probe widget existence before
defining custom ones (the conventional pattern for plugin
loaders) can't:

```sh
# Conventional plugin guard
[[ -z "${widgets[my-custom-widget]}" ]] && zle -N my-custom-widget _my_custom_fn
# zsh: registers the widget only if not already defined
# zshrs: always registers (since widgets[] always empty), risk of double-define errors
```

Also blocks `zle -la` output discovery, which is what tools
like fzf-tab and zsh-autosuggestions use to check what widgets
they can intercept.

**Workaround** — use `zle -la NAME` to test existence (may
also be broken — needs separate verification):
```sh
zle -la my-widget 2>/dev/null && echo exists
```

---

## #261 — `${functions_source[fn]}` format diverges (`zsh` vs `zsh:0`)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { :; }; echo "[${functions_source[f]}]"'
[zsh]

$ ./target/debug/zshrs --zsh -c 'f() { :; }; echo "[${functions_source[f]}]"'
[zsh:0]
```

`functions_source[FN]` returns the filename where function
`FN` was defined. For functions defined on the command line
(`-c` mode), zsh returns just the shell name (`zsh`). zshrs
appends `:0` (treating `0` as the line number).

This is a format-string parity bug. Affects code that exact-
matches on `functions_source` values:

```sh
# Check whether fn was defined interactively
if [[ "${functions_source[my_fn]}" == "zsh" ]]; then
    echo "defined at cmdline"
fi
# zsh: takes the branch correctly
# zshrs: comparison fails (value is "zsh:0"), branch missed
```

The `:0` line-suffix probably shows up for any
`functions_source` lookup of a `-c`-defined function — `:0`
is the zshrs-added "line 0" marker (since the cmdline has no
real line number).

**Where** — `src/ported/params/functions_source.rs::format_entry`:
emits `<file>:<line>` for all entries, including cmdline.
C-source `Src/parameter.c::funcsourcegetfn` emits bare
filename when line is 0 (cmdline/builtin context).

**Impact** — equality-based introspection of function
definition location fails. Similar format bugs exist in
`funcfiletrace`/`functrace` (#237) — both share the
"file-with-line-suffix" formatting routine that zshrs's
machinery applies more eagerly than zsh.

**Workaround** — strip the `:N` suffix before comparison:
```sh
src="${functions_source[my_fn]%%:*}"
[[ "$src" == "zsh" ]] && echo "defined at cmdline"
```

---

## #262 — `zsh_eval_context` array always empty (zsh: tracks outer→inner eval context)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "ctx=[${zsh_eval_context[*]}]"; f() { echo "in-fn=[${zsh_eval_context[*]}]"; }; f'
ctx=[cmdarg]
in-fn=[cmdarg shfunc]

$ ./target/debug/zshrs --zsh -c 'echo "ctx=[${zsh_eval_context[*]}]"; f() { echo "in-fn=[${zsh_eval_context[*]}]"; }; f'
ctx=[]
in-fn=[]
```

`zsh_eval_context` is an array tracking the current
evaluation context — each entry describes one level of the
context stack. Values include:
- `cmdarg` — outermost cmdline arg context (`zsh -c "..."`)
- `toplevel` — top of script execution
- `shfunc` — inside a shell function
- `loadeval` — inside `source`/`.` evaluation
- `cmdsubst` — inside `$(...)` command substitution
- `evalpos` — inside `eval` call
- `globsubst` — inside glob-related substitution

zshrs's `zsh_eval_context` is always empty, regardless of
nesting level. The parameter exists (no error on access) but
never gets populated.

Verified across multiple contexts:
- Top-level: zsh `[cmdarg]`, zshrs `[]`
- Inside function: zsh `[cmdarg shfunc]`, zshrs `[]`
- Inside `(...)`: zsh `[cmdarg]`, zshrs `[]`
- Inside `$(...)`: zsh `[cmdarg cmdsubst]`, zshrs `[]`

**Where** — `src/ported/params/eval_context.rs::push_context`:
the autovar exists but nothing pushes entries onto it during
context transitions. C-source `Src/init.c::zsh_main` and
various `Src/exec.c` entry points push/pop via
`zsh_eval_context_stack[]` updates.

**Impact** — debug-output helpers and "where am I" plugins
that key off the context stack get no info:

```sh
# Trace helper: show full eval context on each call
trace() {
    echo "[$(date +%T)] [${zsh_eval_context[*]}] $*" >&2
}
trace "doing X"
# zsh: "[...] [cmdarg toplevel shfunc] doing X"  (full call stack context)
# zshrs: "[...] [] doing X"  (no context info)
```

Some prompt themes also use `zsh_eval_context` to determine
"am I in a function or interactive prompt" — those checks
silently default to wrong path.

**Workaround** — manually track via `${funcstack[*]}` (already
broken per #237 though — file:line format diverges).

---

## #263 — `$ZSH_DEBUG_CMD` empty when DEBUG trap fires (zsh: contains the command being executed)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'trap "echo \"CMD=[\$ZSH_DEBUG_CMD]\"" DEBUG; echo hi'
CMD=[echo hi]
hi

$ ./target/debug/zshrs --zsh -c 'trap "echo \"CMD=[\$ZSH_DEBUG_CMD]\"" DEBUG; echo hi'
CMD=[]
hi
```

`ZSH_DEBUG_CMD` is the parameter set by zsh just before
firing the DEBUG trap — contains the text of the command
about to be executed. Used by step-debuggers and tracing
plugins.

zshrs's DEBUG trap fires (the trap body runs) but
`ZSH_DEBUG_CMD` is empty when read inside the trap. The
parameter exists but isn't populated with the command text.

(Side observation: in zsh's `-c` cmdline mode, the DEBUG
trap may or may not fire depending on the command form —
the output above shows it firing once for the `echo hi`
command. zshrs fires it consistently. Both behaviors are
present but the missing `ZSH_DEBUG_CMD` content is the
clear bug.)

**Where** — `src/ported/exec.rs::fire_debug_trap`: invokes
trap body but doesn't set `ZSH_DEBUG_CMD` parameter
beforehand. C-source `Src/exec.c::trapcmd` updates the
parameter from `dupstring(text)` before calling the trap.

**Impact** — step-debuggers, command-tracers, and slow-
command-logging plugins (common in zsh user setups) can't
introspect what command is about to run:

```sh
# Log every command before execution
trap '[[ "$ZSH_DEBUG_CMD" =~ "slow_op" ]] && start_timer' DEBUG
# zsh: timer starts whenever a slow_op command is about to run
# zshrs: ZSH_DEBUG_CMD always empty, regex never matches, timer never starts
```

Combined with #237 (funcfiletrace format broken), zshrs's
debugging/introspection infrastructure has multiple gaps that
together make custom debug tooling unusable.

**Workaround** — none direct — `ZSH_DEBUG_CMD` is the
authoritative source. Could use `read -A history -p` to read
just-typed lines but that's only interactive.

---

## #264 — `${widgets[fn]}` returns `builtin` for ALL queries (registration ignored)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'my-fn() { :; }; zle -N my-fn; echo "[${widgets[my-fn]}]"'
[user:my-fn]

$ ./target/debug/zshrs --zsh -c 'my-fn() { :; }; zle -N my-fn; echo "[${widgets[my-fn]}]"'
[builtin]
```

After registering a custom widget via `zle -N name [fn]`,
zsh's `widgets[name]` introspection returns `user:fnname`
indicating it's a user-defined widget backed by function
`fnname`. zshrs returns the literal string `builtin` for
ALL queries — including ones that were never registered.

Combined with #260 (which showed `widgets[]` introspection
returns empty for built-in widgets and `(@k)widgets` returns
0 keys), the widgets-assoc semantics in zshrs are doubly
broken:

1. Pre-registered builtin widgets don't appear in the
   introspection table at all (#260).
2. User-registered widgets via `zle -N` aren't reflected in
   the table either — every query returns the literal
   `builtin` string regardless of the actual widget state.

The `builtin` literal appears to be a hardcoded default
return — there's no actual lookup happening against a real
widgets-table.

**Where** — `src/ported/params/widgets.rs::widgets_getfn`:
returns a hardcoded `"builtin"` string instead of consulting
the registered widget table. C-source `Src/Zle/zle_thingy.c::widgetsgetfn`
iterates `thingytab[]` and returns the formatted entry
(`user:fnname` for user widgets, `completion:context` for
completion widgets, etc.).

**Impact** — completion frameworks (fzf-tab, zsh-completions,
zsh-syntax-highlighting) that probe widget types to decide
how to interact with them get wrong type info on every
query:

```sh
# fzf-tab-style guard: only wrap user widgets, not builtins
case "${widgets[expand-or-complete]}" in
    user:*) wrap_with_fzf expand-or-complete ;;
    *)      : ;;
esac
# zsh: matches user:* only after compsys/fzf-tab has bound the widget
# zshrs: always matches *) branch (value is "builtin"), wrapping never happens
```

This is a critical introspection gap for any zsh plugin that
inspects widget types.

**Workaround** — track widget registration in a parallel
user-managed assoc instead of relying on `widgets[]`.

---

## #265 — `$MATCH` not populated by `(#m)` flag in `${var/pat/...}` substitution

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extendedglob; s=hello; echo "[${s/(#m)*/MATCH=$MATCH}]"'
[MATCH=hello]

$ ./target/debug/zshrs --zsh -c 'setopt extendedglob; s=hello; echo "[${s/(#m)*/MATCH=$MATCH}]"'
[MATCH=]
```

The `(#m)` extended_glob flag enables `$MATCH`, `$MBEGIN`,
and `$MEND` parameters during pattern matching. In zsh, these
populate during `${var/pat/repl}` substitution — the
replacement string can reference `$MATCH` to incorporate the
matched text.

zshrs's substitution machinery doesn't populate `$MATCH`
inside the replacement context. The replacement runs but
reads `$MATCH` as empty.

`$MATCH` works correctly in `=~` regex match context:
```sh
$ both-shells -fc '[[ "hello" =~ "([a-z]+)" ]] && echo "MATCH=[$MATCH]"'
MATCH=[hello]
```

So the parameter-machinery exists; it just isn't connected to
the parameter-substitution-with-replacement code path.

**Where** — `src/ported/paramsubst.rs::do_replacement`:
doesn't call into the `set_match_params()` hook before
expanding the replacement string. C-source
`Src/subst.c::dosubst` populates the params via
`patmatch()` before passing the replacement to the
expansion engine.

**Impact** — common pattern for "wrap matched text" idioms
fails:

```sh
# Highlight URLs by wrapping with ANSI codes
log_line="${input/(#m)https:\/\/[^ ]##/${'$'\e''[36m'}$MATCH${'$'\e''[0m'}}"
# zsh: surrounds the matched URL with color codes
# zshrs: $MATCH is empty, surrounds with empty string (just emits color codes)
```

**Workaround** — use `=~` regex with `$MATCH`:
```sh
if [[ "$input" =~ "(https://[^ ]+)" ]]; then
    log_line="${input/$MATCH/\e[36m$MATCH\e[0m}"
fi
```

---

## #266 — `$match[N]` backref array not populated by `(#b)` flag in substitution

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt extendedglob; s="abc123"; echo "[${s/(#b)([a-z]##)/[$match[1]]}]"'
[[abc]123]

$ ./target/debug/zshrs --zsh -c 'setopt extendedglob; s="abc123"; echo "[${s/(#b)([a-z]##)/[$match[1]]}]"'
[[]123]
```

`(#b)` enables backreference capture into the `$match[]`,
`$mbegin[]`, and `$mend[]` arrays. Parenthesized groups in
the pattern populate `$match[1]`, `$match[2]`, etc., for use
in the replacement.

zsh populates `$match` correctly. zshrs's substitution
doesn't trigger the backref capture — replacement reads
`$match[1]` as empty.

Companion bug to #265 — both `(#m)`/`(#b)` flag-driven
parameter population paths are missing in substitution
context.

**Where** — `src/ported/paramsubst.rs::do_replacement` (same
location as #265): doesn't call into the backref-capture
hook. C-source `Src/subst.c::dosubst` calls `patmatch()`
which fills `match[]`/`mbegin[]`/`mend[]` arrays before the
replacement is expanded.

**Impact** — major loss for sed-style "capture and reorder"
patterns:

```sh
# Reformat date: 2026-05-30 → 30/05/2026
input="2026-05-30"
output="${input/(#b)(####)-(##)-(##)/$match[3]/$match[2]/$match[1]}"
# zsh: "30/05/2026"
# zshrs: "//" — all matches are empty
```

**Workaround** — use `=~` match then build the replacement
manually:
```sh
[[ "$input" =~ "([0-9]+)-([0-9]+)-([0-9]+)" ]]
output="${match[3]}/${match[2]}/${match[1]}"
```

---

## #267 — Bare `setopt` (no args) prints nothing instead of listing currently-set options

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'setopt' | head -10
nohashdirs
norcs

$ ./target/debug/zshrs --zsh -c 'setopt' | head -10
(empty)
```

`setopt` with no args is the introspection form — lists all
currently-set options, one per line. zsh emits 2 lines for
`-f` mode (`nohashdirs`, `norcs`). zshrs prints nothing.

The same likely applies to `unsetopt` with no args (lists
unset options).

The `setopt -p` (POSIX-style `set -o`-equivalent) likely
works since it goes through a different code path — but
bare `setopt` is the more common idiom.

**Where** — `src/ported/builtins/setopt.rs::list_active_opts`:
no-args path doesn't iterate the option table. C-source
`Src/options.c::bin_setopt` walks `optns[]` and prints
each option whose state matches the "set" filter.

**Impact** — diagnostics and `.zshrc` introspection scripts
that capture current option state break:

```sh
# Save current option state for restoration
saved_opts=$(setopt)
# ... do stuff that might change opts ...
unsetopt $(setopt)            # reset all
eval "$saved_opts"            # restore
# zsh: saves a snapshot, restores correctly
# zshrs: saved_opts is empty, restore does nothing
```

`p10k` and several oh-my-zsh themes use this pattern to
preserve user-set options across their initialization paths.

**Workaround** — use `set -o` (POSIX form) which likely works:
```sh
saved=$(set -o)
```

---

## #268 — Autovars `LISTMAX`/`MAILCHECK`/`KEYTIMEOUT`/`PERIOD` typed as `scalar` instead of `integer`

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "LISTMAX=[${(t)LISTMAX}] KEYTIMEOUT=[${(t)KEYTIMEOUT}] MAILCHECK=[${(t)MAILCHECK}]"'
LISTMAX=[integer] KEYTIMEOUT=[integer-export] MAILCHECK=[integer]

$ ./target/debug/zshrs --zsh -c 'echo "LISTMAX=[${(t)LISTMAX}] KEYTIMEOUT=[${(t)KEYTIMEOUT}] MAILCHECK=[${(t)MAILCHECK}]"'
LISTMAX=[scalar] KEYTIMEOUT=[scalar-export] MAILCHECK=[scalar]
```

zsh's automatic parameters that hold integer values
(`LISTMAX` for completion list size, `KEYTIMEOUT` for key-
sequence timeout, `MAILCHECK` for mail-check interval,
`PERIOD` for periodic hook interval, etc.) are typed as
`integer` from initialization. zshrs initializes them as
`scalar`.

Affects which arithmetic semantics apply — `LISTMAX=abc` in
zsh fails with "bad math expression," while in zshrs it
silently accepts a string into a "scalar" variable.

Note: some autovars are correctly typed:
- `HISTSIZE`: `integer-export-special` (correct in both)
- `RANDOM`/`SECONDS`/`LINENO`: integer in both (presumably)

So zshrs has a partial autovar-type table — about half of
the integer autovars are correctly typed, half are scalar.

**Where** — `src/ported/params/autovars.rs::register_autovars`:
the autovar registry initializes `LISTMAX/KEYTIMEOUT/MAILCHECK/
PERIOD` without the `-i` integer flag. C-source
`Src/params.c::IPDEF_INT_BLOCK` macro applies `PM_INTEGER` to
each entry in the integer-typed autovar list.

**Impact** — arithmetic-aware semantics broken on these
params:

```sh
LISTMAX="bogus"      # zsh: errors "bad math expression"
                     # zshrs: accepts, stores literal string
                     # next completion that reads LISTMAX gets garbage
```

Also breaks defensive code that introspects via `(t)` flag:

```sh
case "${(t)LISTMAX}" in
    *integer*) listmax_arith=$LISTMAX ;;
    *)         listmax_arith=10 ;;
esac
# zsh: takes integer branch
# zshrs: takes default branch
```

**Workaround** — explicitly re-cast with `typeset -i` after
declaration:
```sh
typeset -i LISTMAX
typeset -i KEYTIMEOUT
```

---

## #269 — `$SPROMPT` autovar default empty (zsh: spell-check prompt template)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[${SPROMPT-NONE}]"'
[zsh: correct '%R' to '%r' [nyae]? ]

$ ./target/debug/zshrs --zsh -c 'echo "[${SPROMPT-NONE}]"'
[]
```

`SPROMPT` is the prompt template used when zsh's spell-
correction asks the user to confirm a substitution.
zsh has a built-in default: `zsh: correct '%R' to '%r'
[nyae]? `. zshrs has it set to empty (which is what `${X-NONE}`
sees as "set but empty," distinct from "unset").

Combined with various other autovar default-value gaps
(some empty, some scalar-instead-of-integer, etc.), zshrs's
autovar table is missing initialization data for several
parameters.

Note: `${SPROMPT-NONE}` distinguishes "unset" from "empty";
the test confirms SPROMPT is "set but empty" in zshrs (not
"unset"), so the autovar exists in the symbol table but
without its zsh-default content.

**Where** — `src/ported/params/autovars.rs::default_sprompt`:
either omitted from the default-value table or set to empty
string. C-source `Src/zsh.h`'s `PROMPT_S` constant provides
the default: `"zsh: correct '%R' to '%r' [nyae]? "`.

**Impact** — spell-correction (`setopt correct`/`correct_all`)
shows nothing when prompting the user — silent prompt asking
for confirmation, no visible cue what's being asked.

Less common than other prompts (most users disable
`correct`), but for users relying on `correct`, this makes
the feature unusable.

Likely-similar gaps for other autovar defaults: `WATCHFMT`
(watch-output format), `TIMEFMT` (already verified mostly OK
in #243 — value present, type empty), `NULLCMD`, `READNULLCMD`.

**Workaround** — explicit init in `.zshrc`:
```sh
SPROMPT="zsh: correct '%R' to '%r' [nyae]? "
```

---

## #270 — `${(t)watch}` autovar absent entirely (zsh: `array-special`)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "[${(t)watch}]"'
[array-special]

$ ./target/debug/zshrs --zsh -c 'echo "[${(t)watch}]"'
[]
```

`watch` (lowercase, parallels `WATCH`) is the array autovar
holding the list of users to watch for login/logout activity.
zsh registers it as a special array; zshrs's introspection
shows empty type — the autovar isn't registered at all.

Same gap exists for the scalar/array twin: `WATCH` (uppercase
scalar form, tied to `watch` array via `typeset -T`). With
#231 (tied-array no-sync), even if `WATCH` got registered,
the tie wouldn't work.

This is a feature-set gap (the `watch` builtin and the
`watch` array work together — without the parameter, the
`watch` builtin has nothing to read).

**Where** — `src/ported/params/init.rs::register_array_autovars`:
no entry for `watch`. C-source `Src/Modules/zsh_watch.c`
(part of `zsh/system` module's watch functionality) defines
the parameter when loaded — possibly the module isn't loaded
in zshrs, or the init order is wrong.

**Impact** — `watch` user-tracking feature dead. Tools that
introspect available special arrays for "what state can I
read" see fewer entries:

```sh
# Plugin checking available watch capability
[[ "${(t)watch}" == array* ]] || echo "no watch parameter"
# zsh: takes the affirmative path
# zshrs: takes the negative path (no watch support)
```

Combined with other watch-related gaps (`WATCHFMT` likely
also empty/default, `LOGCHECK` autovar status unknown), the
entire watch-for-logins feature is non-functional in zshrs.

**Workaround** — none for the watch feature itself. For type-
checking code, fall back to `(( ${+watch} ))` existence test
(though this may also report wrong for unregistered autovars).

---

## #271 — `h=([k]=v)` bash-style assoc init treated as glob pattern (errors "no matches found")

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -A h; h=([a]=1 [b]=2); echo "[${h[a]}][${h[b]}]"'
[1][2]

$ ./target/debug/zshrs --zsh -c 'typeset -A h; h=([a]=1 [b]=2); echo "[${h[a]}][${h[b]}]"'
zsh:1: no matches found: [a]=1
```

zsh supports two assoc-init syntaxes:
1. Flat-pairs (zsh classic): `h=(k1 v1 k2 v2)` — works in both shells.
2. Bash/ksh `[k]=v` form: `h=([a]=1 [b]=2)` — works in zsh, errors in zshrs.

zshrs's parser treats `[a]=1` as a glob expression `[a]` followed
by literal `=1`, which fails the glob match (no files match
the bracket pattern), triggering "no matches found".

Append form `h+=([b]=2)` has the same bug — also errors.

**Where** — `src/ported/parse.rs::parse_array_literal`: when
seeing `[...]=value` inside an array-init `(...)` block, no
special-case to disable glob expansion and treat as assoc
key-value pair. C-source `Src/parse.c::par_array` checks the
target parameter's type — if assoc, `[k]=v` is parsed as
key-value; if regular array or unknown type, glob-expanded.

**Impact** — major bash-porting blocker. Most bash/ksh
scripts using assoc arrays use the `[k]=v` form:

```sh
# Common bash pattern
typeset -A config=(
    [host]=example.com
    [port]=443
    [timeout]=30
)
# zsh: defines config correctly
# zshrs: errors on first [host]=example.com, script aborts
```

This affects every ported bash script with assoc arrays, every
config-as-assoc pattern, every multi-step assoc init.

**Workaround** — convert to flat-pairs syntax:
```sh
typeset -A config=(
    host example.com
    port 443
    timeout 30
)
```

(Less readable, but works in both shells.)

---

## #272 — `typeset -axU` combined flags — `-U` dedup not applied when combined with `-x` (export)

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'typeset -axU u=(/bin /usr/bin /bin); echo "[${u[@]}]"'
[/bin /usr/bin]

$ ./target/debug/zshrs --zsh -c 'typeset -axU u=(/bin /usr/bin /bin); echo "[${u[@]}]"'
[/bin /usr/bin /bin]
```

`typeset -axU` declares an exported unique-only array. Both
attributes should compose — the array dedups on assignment
AND is exported to the environment. zsh applies both. zshrs
applies only `-a` and `-x`, silently dropping `-U`.

Control: `typeset -aU` (no `-x`) works in both — `[/bin
/usr/bin]`. So `-U` is honored when not combined with `-x`.

**Where** — `src/ported/builtins/typeset.rs::parse_combined_flags`:
flag combination parse picks `-x` but drops `-U` when both
present. Could be flag-mask collision or option-string parsing
order issue. C-source `Src/builtin.c::typeset_single` accumulates
all flags as bits in a single `pmflags` value, with no
mutual-exclusion check.

**Impact** — exported deduped arrays (the canonical pattern
for `path`-like vars) silently keep duplicates:

```sh
# Common .zshrc pattern
typeset -xU path manpath fpath
# zsh: each array is exported AND deduped
# zshrs: each array exported, but duplicates accumulate across
#        edits — fpath bloat, manpath bloat
```

Affects the user's daily-driver setup directly — `typeset
-xU path` is one of the very first lines in many .zshrc
files.

**Workaround** — separate the operations:
```sh
typeset -aU u=(/bin /usr/bin /bin)   # dedup applied
typeset -x u                          # then export
```

---

## #273 — `TIMEFMT` autovar in inconsistent state — value present, but `(t)` and `${-NONE}` both signal "unset"

**Status:** `port-bug` — surfaced 2026-05-30 hunting.

```sh
$ /opt/homebrew/bin/zsh -fc 'echo "val=[$TIMEFMT]"; echo "default=${TIMEFMT-NONE}"; echo "type=${(t)TIMEFMT}"'
val=[%J  %U user %S system %P cpu %*E total]
default=%J  %U user %S system %P cpu %*E total
type=scalar

$ ./target/debug/zshrs --zsh -c 'echo "val=[$TIMEFMT]"; echo "default=${TIMEFMT-NONE}"; echo "type=${(t)TIMEFMT}"'
val=[%J  %U user %S system %P cpu %*E total]
default=NONE                       # ← unset-default triggers
type=                              # ← no type
```

`TIMEFMT` autovar is in a "schrodinger-set" state in zshrs:
- Direct read returns the default content (works).
- `${TIMEFMT-NONE}` (no-colon default — triggers only on
  truly-unset) returns `NONE` — meaning the parameter is
  detected as unset.
- `${(t)TIMEFMT}` returns empty — meaning the parameter has
  no registered type.

Three different code paths disagree on whether TIMEFMT is
"set":
1. Value-read path: yes (returns the content).
2. Existence/unset-default path: no.
3. Type-introspection path: no.

This is more severe than #243 (which noted `(t)` returns empty
for special vars) — `TIMEFMT` literally has a value AND has
an "unset" state simultaneously.

**Where** — `src/ported/params/autovars.rs::TIMEFMT_handler`:
likely a hardcoded `getfn` returning the default string, but
no `Param` registered in the symbol table at all. C-source
`Src/params.c::createparam` would register the entry with
proper `PM_SCALAR` type and visibility to `${X-default}`.

**Impact** — code paths that check parameter existence diverge
from code paths that read the value:

```sh
# Common pattern: "use custom format if user hasn't overridden"
: "${TIMEFMT:=user override default}"
# zsh: TIMEFMT was already set, no change
# zshrs: TIMEFMT detected as unset, := assigns new value
#        breaking the user-override-default idiom
```

Combined with #243 and #269, the autovar machinery has
multiple consistency gaps — the symbol table, type table,
default-value table, and unset-state table are all separate
in zshrs but should be unified.

**Workaround** — explicit check before defaulting:
```sh
[[ -z "$TIMEFMT" ]] && TIMEFMT="%J  %U user %S system %P cpu %*E total"
```

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
| 46 | nested `` `\`...\`` `` backquotes mishandled | **port-bug** | use `$(...)` instead |
| 47 | `${(b)str}` escapes space/semi (C-zsh doesn't) | **port-bug** | drop `(b)` flag |
| 48 | `typeset -m PAT` rejects pattern arg | **port-bug** | iterate `${(k)parameters}` |
| 49 | `(( "abc" == "abc" ))` quoted strings → false | **port-bug** | drop quotes |
| 50 | Trap inherited from outer doesn't fire in fn | **port-bug** | re-install trap in fn |
| 51 | `${#*}` access corrupts `$@`/`$*` for rest of fn | **port-bug** | use `${#@}` or `$#` |
| 52 | `${(q)arr}` per-element quote, doesn't quote join-sep | **port-bug** | `${(j: :)${(@q)a}}` explicit |
| 53 | `${(P)$ref}` doesn't resolve `name[idx]` indirect | **port-bug** | `eval "val=\\${$ref}"` |
| 54 | `warn_create_global` / `warn_nested_var` warnings silent | **port-bug** | strict `local` discipline |
| 55 | `setopt err_return` doesn't fire on command failure | **port-bug** | explicit `\|\| return $?` |
| 56 | Signal trap output captured into `$(...)` result | **port-bug** | guard cmd-sub output |
| 57 | `setopt octal_zeroes` ignored by arith parser | **port-bug** | `8#NNN` explicit base |
| 58 | `[[ "x*" == "x*" ]]` quoted-RHS-star still globbed | **port-bug** | escape `\*` on RHS |
| 59 | `setopt no_clobber` allows `>>` to create new file | **port-bug** | pre-`touch` the file |
| 60 | `function {body}` (no name) parses + stray `}` echo | **port-bug** | use `funcname() { body }` form |
| 61 | `h["key"]=v` subscript quotes not embedded in key | **port-bug** | use `h=( k v )` paren init |
| 62 | `extended_glob` `~` (and-not) operator not honored | **port-bug** | iterate + skip with `[[` |
| 63 | `${(j:s:)${(s:t:)var}}` nested split-then-join → first element only | **port-bug** | intermediate `arr=(...)` |
| 64 | `$PIPESTATUS` (bash-style upper) exists in zshrs but not zsh | **port-bug** | use lowercase `$pipestatus` |
| 65 | `${+EPOCHSECONDS}` returns 0 after `zmodload zsh/datetime` | **port-bug** | guard by `zmodload` rc |
| 66 | `time` builtin ignores `TIMEFMT`, omits `%J` cmd name | **port-bug** | `/usr/bin/time -f` instead |
| 67 | `pushd` no-args doesn't swap top of dir stack | **port-bug** | explicit `pushd $OLDPWD` |
| 68 | `trap` listing in insertion order, not signal-number | **port-bug** | pipe through `sort` |
| 69 | `$sysparams` auto-loaded w/o `zmodload zsh/system` | **port-bug** | call `zmodload` regardless |
| 70 | FS watcher leaks newly-created paths to stderr | **port-bug** | none — must fix in zshrs |
| 71 | `${var:N:M}` accepts non-digit offset (bashism) | **port-bug** | wrap offset in `$(( ))` |
| 72 | `log` builtin registered but dispatch → `/usr/bin/log` | **port-bug** | `print -- $watch` instead |
| 73 | `$ZSH_VERSION` includes `.0.3-test` suffix vs `5.9` | **port-bug** | parse `${ZSH_VERSION%%.0*}` |
| 74 | `local -r` violation in fn doesn't abort script | **port-bug** | check fn exit status |
| 75 | `typeset -i x; x="bad math"` silently coerces to 0 | **port-bug** | regex-validate input first |
| 76 | `zmodload` lists 32 auto-loaded modules vs zsh's 1 | **port-bug** | none — startup-time bloat |
| 77 | `${h[(k)-key]}` flag-lookup of dash key returns empty | **port-bug** | direct `${h[$opt]+set}` |
| 78 | `echoti` output emitted AFTER next stdout (buf flush) | **port-bug** | direct `printf '\e[...'` |
| 79 | Job control table empty: `jobs`/`wait %N`/`kill %N`/`disown` fail | **port-bug** | use `$!` PID instead |
| 80 | `trap EXIT` in fn fires at script exit, lost in nested fns | **port-bug** | explicit cleanup at fn epilogue |
| 81 | `extended_glob *~b` returns duplicates + matches dir | **port-bug** | loop with `[[ == ... ]] continue` |
| 82 | `"PREFIX${(s.X.)var}"` repeats prefix per element | **port-bug** | `arr=("${(s.X.)v}"); "P:${arr[*]}"` |
| 83 | `${a[(s.,.)N,M]}` slice with flag returns full array | **port-bug** | drop subscript flag |
| 84 | `bindkey -L` 117 entries vs zsh's 31 (default keymap differs) | **port-bug** | normalize via post-process |
| 85 | `"${(s.X.)s[@]}"` on scalar with `[@]` returns empty | **port-bug** | `(@s.X.)s` flag-first form |
| 86 | `${1:?msg}` error format has spurious `:1:` line | **port-bug** | sed-strip the line number |
| 87 | `setopt` (no args) empty under `-fc`; zsh shows `nohashdirs/norcs` | **port-bug** | `$options[rcs]` direct query |
| 88 | `setopt nounset` doesn't fire on unset var in arith `$((x+1))` | **port-bug** | `[[ -v var ]]` guard |
| 89 | `extended_glob #`/`##` quantifiers not recognized (literal) | **port-bug** | use `*` + char class |
| 90 | `$ZSH_PATCHLEVEL` = literal `"unknown"` vs zsh's commit | **port-bug** | fallback to `$ZSH_VERSION` |
| 91 | `:t` modifier dropped on `${(j:X:)arr:t}` joined-then-modifier | **port-bug** | split the two ops |
| 92 | `$PS4` default is empty; zsh's is `%x\t%0N\t%I\t%_` colored | **port-bug** | explicit `export PS4=...` |
| 93 | Empty assoc key broken: paren-init misaligns, subscript stores but no retrieve | **port-bug** | reserve `__EMPTY__` sentinel |
| 94 | `(exec cmd); cmd2` parent shell terminates with subshell | **port-bug** | drop `exec` inside subshell |
| 95 | Signal trap from `kill -X $$` in subshell fires immediately | **port-bug** | avoid signal-IPC across sub |
| 96 | `%N/` `%N~` prompt escape doesn't truncate path | **port-bug** | manual `precmd` truncation |
| 97 | `typeset -r` listing omits shell-internal readonly params (`!=0` etc.) | **port-bug** | n/a — semantic still readonly |
| 98 | `[ "a" \< "b" ]` lex-compare bash ext accepted (zsh errors) | **port-bug** | `[[ < ]]` double-bracket |
| 99 | `(#cN,M)` count quantifier + other `(#x)` flags not recognized | **port-bug** | `=~ {N,M}` regex form |
| 100 | `typeset -R N x="hello"` doesn't right-truncate (full string kept) | **port-bug** | `printf "%Ns"` instead |
| 101 | `exec funcname` errors "not found" instead of running shell fn | **port-bug** | drop `exec`, call fn directly |
| 102 | `$-` doesn't include `f` from `-f` startup flag | **port-bug** | `[[ -o no_rcs ]]` direct option test |
| 103 | `$0` inside sourced script returns shell binary, not sourced file | **port-bug** | `${(%):-%x}` prompt-expansion |
| 104 | Signal `kill -X $$` from inside fn is lost (trap never fires) | **port-bug** | direct invocation post-fn |
| 105 | `(f<NNN>)` permission glob qualifier ignored | **port-bug** | `stat`-based loop |
| 106 | `disable BUILTIN` doesn't actually disable (echo/cd still work) | **port-bug** | `command BUILTIN` prefix |
| 107 | `autoload -U +X funcname` doesn't validate fpath existence | **port-bug** | manual `[[ -f $fpath/fn ]]` check |
| 108 | `${array/pat/X}` per-element (zsh treats as scalar-joined) | **port-bug** | `${arr[*]}` explicit join |
| 109 | `${assoc[@]}` returns empty (no value enumeration) | **port-bug** | use `${(v)h[@]}` explicit |
| 110 | `a[0]=val` silently accepted (zsh 1-indexed, errors) | **port-bug** | use 1-indexed throughout |
| 111 | `%y` (and `%l`) prompt escape for tty not expanded | **port-bug** | `${TTY##*/}` substitution |
| 112 | Builtin error format leaks Rust's `(os error N)` suffix | **port-bug** | grep loosely for portability |
| 113 | `$'\C-X'` ANSI-C ctrl-char escape not honored (literal) | **port-bug** | `$'\xNN'` hex escape |
| 114 | `${(l.W.)s}` width must be literal; variable errors | **port-bug** | `printf "%${w}s"` instead |
| 115 | Prompt `%s`/`%b`/`%u` use full reset `\e[0m` not selective | **port-bug** | re-apply attrs after `%x` |
| 116 | `GLOB_SUBST` defaults ON in zshrs (zsh: off) | **port-bug** | `unsetopt glob_subst` explicit |
| 117 | Extended_glob `(group)#` quantifier not recognized | **port-bug** | `**/` recursive glob |
| 118 | `(( y = x ))` doesn't coerce non-numeric string to 0 | **port-bug** | `integer y; y=$x` |
| 119 | `glob_subst` doesn't trigger filename expansion in for-loop | **port-bug** | `eval "echo ..."` force-expand |
| 120 | `a=("${a[@]:0:-1}")` on empty arr produces 1-element arr | **port-bug** | length-gated branch |
| 121 | `[[ -N -op -M ]]` negative-number operands error "unknown condition" | **port-bug** | use `(( ))` arith |
| 122 | Exit status of `$()` inside `${x:-$()}` not propagated | **port-bug** | pre-eval cmdsub |
| 123 | `${arr[@]}` inside heredoc returns only first element | **port-bug** | `${(j: :)arr}` or pre-join |
| 124 | `typeset -f` source-as-typed vs zsh pretty-printed | **port-bug** | normalize whitespace |
| 125 | `var=${a[-1]}` assignment returns empty (echo works) | **port-bug** | `var=${a[${#a}]}` positive idx |
| 126 | `${s:N:}` empty length silently returns empty (zsh errors) | **port-bug** | careful syntax |
| 127 | `$'\xNN'` interpreted as Unicode codepoint + UTF-8 re-encode | **port-bug** | `printf '\xNN'` direct |
| 128 | `${(C)arr[N]}` indexed-element case-flag errors "bad substitution" | **port-bug** | assign to scalar first |
| 129 | `local -a a=("$@")` splits quoted args (without `-a` works) | **port-bug** | `local -a a; a=("$@")` |
| 130 | `${var@X}` bash parameter-transform accepted (zsh errors) | **port-bug** | `${(U)x}`/`${(L)x}`/`${(q)x}` |
| 131 | `%(N~.A.B)` prompt conditional evaluates path-depth wrong | **port-bug** | manual `precmd` depth check |
| 132 | `(( x = "5" + "3" ))` quoted numeric strings not coerced | **port-bug** | drop quotes for known-numeric |
| 133 | `zstat -F "fmt"` format flag ignored | **port-bug** | external `stat -f`/`stat --format` |
| 134 | `${"":h}` empty-string head modifier returns `/` (zsh: `.`) | **port-bug** | explicit empty-check |
| 135 | `*(om)` glob qualifier mtime ordering broken | **port-bug** | external `ls -t` |
| 136 | `%E` prompt escape (clear-EOL) not expanded; literal `%E` | **port-bug** | manual `$'\e[K'` |
| 137 | `(( "str" == "str" ))` returns false (no string coerce) | **port-bug** | use `[[ == ]]` for strings |
| 138 | `%i` prompt escape returns `0` instead of current line | **port-bug** | `$LINENO` parameter |
| 139 | Sourced-file errors report `zsh:1:` instead of `/file:N` | **port-bug** | none — debug manually |
| 140 | `exec /no/such` uses generic "not found" + wrong `zshrs:` prefix | **port-bug** | pre-check `[[ -x cmd ]]` |
| 141 | `;;` outside case context not a parse error (silent drop) | **port-bug** | careful review |
| 142 | Orphan-terminator parse error: "orphan terminator" + double-print | **port-bug** | none |
| 143 | `$TRY_BLOCK_ERROR` initial value is `0` in zshrs (zsh: `-1`) | **port-bug** | explicit state-flag |
| 144 | `${(q)str}` with newline uses `\<newline>` not `$'\n'` form | **port-bug** | `(qq)` double-quote form |
| 145 | `${(k)h[name]}` key-existence query errors "bad substitution" | **port-bug** | `(( ${+h[name]} ))` |
| 146 | `{ cmd; } arg` trailing args silently accepted (zsh: parse error) | **port-bug** | careful braces |
| 147 | `${(@)arr:mod}` modifier dropped after `(@)` flag | **port-bug** | `${arr[@]:mod}` subscript form |
| 148 | `zsh/mathfunc` missing cbrt/asinh/erfc/gamma/j0/rand48/... | **port-bug** | external `bc`/`python` |
| 149 | `${(q)str}` with tab/control chars uses `\X` not `$'\X'` form | **port-bug** | `(qq)` double-quote form |
| 150 | `$OPTERR` initialized to `1` (zsh: empty/unset) | **port-bug** | `(( OPTIND > 1 ))` check |
| 151 | `${(@qq)arr}` only quotes first element (rest unquoted) | **port-bug** | explicit per-element loop |
| 152 | `${(qq)arr}` per-element when zsh joins-then-quotes | **port-bug** | `${(qq)${(j: :)arr}}` |
| 153 | `${#${(z)s}}` returns 5 vs 4 (off-by-one count) | **port-bug** | intermediate `arr=(...)` |
| 154 | Readonly var modifiable via `(( ))` / `let` arith | **port-bug** | post-assignment check |
| 155 | `${str[N,M+1]}` slice subscript ignores var/arith | **port-bug** | pre-compute index |
| 156 | `[[ -e /path/*.glob ]]` glob-expands in test (zsh: literal) | **port-bug** | external `ls` test |
| 157 | `TRAP<SIG>()` function-named trap handlers not recognized | **port-bug** | explicit `trap` builtin |
| 158 | Function-def redirect `f() {} < file` not honored | **port-bug** | redirect at call site |
| 159 | `while [[ $((i++)) -lt N ]]` only iterates once | **port-bug** | `while (( i++ < N ))` |
| 160 | `autoload -U +X funcname` doesn't actually load function body | **port-bug** | drop `+X`, lazy load |
| 161 | `case x in)` empty pattern silently accepted (zsh: parse error) | **port-bug** | careful syntax |
| 162 | `${(l.5)x}` missing close-delim silently accepted (zsh: error) | **port-bug** | careful syntax |
| 163 | `${(t)1}` positional type returns `scalar` (zsh: `array-special`) | **port-bug** | `[[ "$#" -gt 0 ]]` test |
| 164 | Extended_glob `^pat` (negation prefix) not recognized | **port-bug** | loop with `[[ == ]] continue` |
| 165 | `${$((expr))}` arith-as-name returns empty (zsh: expr value) | **port-bug** | direct `$((expr))` |
| 166 | `for x in $@` keeps empty elements (zsh: removes via IFS-split) | **port-bug** | `[[ -z $arg ]] continue` |
| 167 | Unclosed `{ cmd` silently runs (zsh: parse error) | **port-bug** | careful review |
| 168 | Extra `}` after command silently ignored (zsh: parse error) | **port-bug** | careful review |
| 169 | `{} always {} always {}` chained-always silently accepted | **port-bug** | careful review |
| 170 | `echo (abc` unclosed paren treated as literal | **port-bug** | careful review |
| 171 | `cmd \| \| cmd`/`&& &&`/`\|\| \|\|` empty operands silently accepted | **port-bug** | careful review |
| 172 | `${ }` whitespace-only param name silently empty (zsh: error) | **port-bug** | careful review |
| 173 | `${(t)$(cmdsub)}` returns `scalar` (zsh: cmdsub output) | **port-bug** | drop `(t)` flag |
| 174 | `type fn` for user-defined function shows "from zsh" suffix | **port-bug** | match `*shell function*` loosely |
| 175 | `(( x = 0xFF ))` doesn't preserve integer base in display | **port-bug** | `typeset -i 16 x` explicit |
| 176 | Bare `echo "\033"` doesn't interpret backslash escapes by default | **port-bug** | `print` or `printf '%b'` |
| 177 | `vared -c X` no-tty silent (zsh: "can't access terminal") | **port-bug** | `[[ -t 0 ]]` tty pre-check |
| 178 | `IFS` doesn't affect cmdsub field-splitting in `for`/array | **port-bug** | `${(@f)$(...)}` explicit |
| 179 | `${(S)pat}` shortest-match flag treated as no-op | **port-bug** | use `${var/pat/}` explicit |
| 180 | `${(C)-text}` no-colon default with flag silently accepted | **port-bug** | use `${(C):-text}` colon-form |
| 181 | `typeset -p` doesn't quote array elements with spaces | **port-bug** | manual `${(qq)}` loop |
| 182 | `${${(P)name}[N]}` after-deref indexing returns full array | **port-bug** | temp `deref=("${(@P)name}")` |
| 183 | `"${@:1:2}"` positional slice returns all instead of slicing | **port-bug** | manual loop |
| 184 | `$((${a[@]} + 0))` arith with array spread silently uses first elem | **port-bug** | explicit loop summation |
| 185 | `[[ -z "${arr[@]}" ]]` for single-empty arr returns false (zsh: true) | **port-bug** | `${arr[*]}` star form |
| 186 | `${(@)b:-default}` for single-empty arr returns default (zsh: empty) | **port-bug** | `(( ${#arr} > 0 ))` check |
| 187 | `f() { :; } > /file` redirect on fn-def creates file at def time | **port-bug** | redirect at call site |
| 188 | Empty-array slice `${a[@]:0:1}` iterates once with empty val | **port-bug** | `${#a} > 0` pre-check |
| 189 | `${()-default}` empty-flag-paren silently returns `$-` | **port-bug** | careful syntax |
| 190 | `kill -L` lists signals (zsh: errors "unknown signal: SIGL") | **port-bug** | always use `-l` lowercase |
| 191 | `${(l.5..)s}` empty-fill silently accepted with garbage output | **port-bug** | always specify fill chars |
| 192 | `${(P)name[N]:mod}` indirect-arr-elem with modifier works (zsh: errors) | **port-bug** | temp var split |
| 193 | `(( y = ${x:?msg} ))` continues after required-param error | **port-bug** | explicit `[[ -v ]]` guard before arith |
| 194 | `function f { :; } > /file` keyword-form fn-def redirect at def time | **port-bug** | redirect at call site |
| 195 | `${(C)${(P)name}[N]}` flag applied to full array, outer subscript ignored | **port-bug** | temp `deref=("${(@P)name}")` |
| 196 | Anonymous fn output lost in `$(() { :; })` cmdsub or `(() { :; })` subshell | **port-bug** | named fn called inside subshell |
| 197 | `typeset -f` function-body display collapses statement newlines into `; ` | **port-bug** | sed `'s/; /\n\t/g'` post-filter |
| 198 | `bindkey -L` output uses individual entries instead of `-R` range-compressed | **port-bug** | round-trip via zshrs's own output |
| 199 | `${(qq)x}` with embedded newline emits mixed `'a'$'\n''b'` instead of literal | **port-bug** | use `(q+)` flag explicitly |
| 200 | `${(k)assoc[key]}` key-flag with subscript returns empty (zsh: key when exists) | **port-bug** | `[[ -v assoc[key] ]]` existence test |
| 201 | `typeset +x VAR` doesn't remove VAR from environment (security-relevant) | **port-bug** | `unset VAR; typeset VAR=$saved` re-bind |
| 202 | `set -eo pipefail; false \| true` doesn't abort — pipefail ignored entirely | **port-bug** | `${pipestatus[1]}` explicit check |
| 203 | `trap EXIT` in fn runs at shell exit instead of fn return (scope leak) | **port-bug** | wrap fn body in subshell `(...)` |
| 204 | `setopt promptsubst` doesn't enable `$()`/`${}` substitution in prompts | **port-bug** | precompute prompt in `precmd` |
| 205 | `read -u N` doesn't read from numeric fd opened via `exec N<...` | **port-bug** | `read x <&N` redirect-form |
| 206 | `setopt multios` only last target receives `cmd > a > b` (zsh: both) | **port-bug** | explicit `tee` pipe |
| 207 | `emulate ksh` doesn't apply `KSH_ARRAYS`/`BSD_ECHO`/etc option bundle | **port-bug** | explicit `setopt ksh_arrays` after |
| 208 | Function defined inside `(...)` subshell leaks into parent shell | **port-bug** | explicit `unfunction` after |
| 209 | Alias defined inside `(...)` subshell leaks into parent shell | **port-bug** | explicit `unalias` after |
| 210 | `(unfunction f)`/`(zmodload m)` in subshell mutate parent state (destructive) | **port-bug** | never use subshell for state-mutating builtins |
| 211 | `$-` (option chars) missing `f`/other letters (option-to-char table incomplete) | **port-bug** | use `setopt \| grep ...` instead |
| 212 | `${(b)str}` over-escapes non-glob chars (tab, etc) — extra `\` inserted | **port-bug** | use `(q)` for shell quoting instead |
| 213 | `${assoc[(R)value]}` reverse-search-by-value errors `bad substitution` | **port-bug** | manual `for k v in "${(@kv)h}"` loop |
| 214 | `chpwd`/`chpwd_functions` hook not fired on `cd` — breaks p10k/oh-my-zsh/zoxide/direnv | **port-bug** | wrap `cd` in function dispatching hooks |
| 215 | `zshexit` hook function not fired on shell exit (also breaks combined with #203) | **port-bug** | `trap "..." EXIT` at top-level |
| 216 | `${(t)var:-default}` after `unset var` returns empty instead of default | **port-bug** | explicit `(( ${+var} ))` existence check |
| 217 | `setopt cdable_vars` ignored — `cd VAR` doesn't deref VAR as path | **port-bug** | explicit `cd "$VAR"` deref |
| 218 | `typeset NAME` for assoc array prints nothing (zsh: `h=( [k]=v ... )` form) | **port-bug** | `typeset -p h` explicit |
| 219 | `typeset -i "h[k]"` integer-on-assoc-element silently accepted (zsh: rejects) | **port-bug** | apply attribute to whole assoc `typeset -iA` |
| 220 | `setopt err_return` doesn't abort function on failed command (fn-scoped strict mode dead) | **port-bug** | explicit `\|\| return $?` per command |
| 221 | `disable -f FN` doesn't disable — `FN` still callable, `dis_functions[FN]` empty | **port-bug** | `unfunction FN` destructive |
| 222 | `zmodload -a` lists 0 auto-loaded modules (zsh: 27 builtin→module bindings) | **port-bug** | try-and-catch with `zmodload -e` |
| 223 | `setopt warn_create_global` no warning on implicit global creation in fn | **port-bug** | manual audit / explicit `local` always |
| 224 | `unsetopt typeset_silent` + `typeset X` doesn't print existing value | **port-bug** | explicit `echo "X=$X"` for debug |
| 225 | `printf "%(%Y)T"` silently treats as literal (zsh: errors "invalid directive") | **port-bug** | use `zsh/datetime` `strftime` builtin |
| 226 | `alias -s SUFFIX=cmd` doesn't populate `saliases` introspection assoc | **port-bug** | parse `alias -s` output instead |
| 227 | `disable -a NAME` neither populates `dis_aliases` nor removes from active `aliases` table | **port-bug** | `unalias NAME` destructive |
| 228 | `hash -d NAME=path` doesn't populate `nameddirs` introspection assoc | **port-bug** | parse `hash -d` output instead |
| 229 | `${(j: :)${(@kv)h}}` join-on-kv drops values (only keys joined) | **port-bug** | manual `for k v in "${(@kv)h}"` loop |
| 230 | `echo "a\0b"` doesn't interpret `\0` octal NUL escape (other escapes work) | **port-bug** | use `print "..."` instead |
| 231 | `${(t)tied_var}` returns `scalar` instead of `scalar-tied`/`array-tied` — tied pair concept missing | **port-bug** | manual sync via `(@s/:/)` |
| 232 | `TRAPEXIT()` named-function form of EXIT trap not fired (3rd broken exit-handler form) | **port-bug** | explicit `trap "..." EXIT` at top-level |
| 233 | `typeset -H VAR=val` then `typeset -p VAR` shows value (security-relevant credential leak) | **port-bug** | filter dump output explicitly |
| 234 | `typeset +U arr` doesn't clear dedup AND `+=` of dup destroys array (corrupting) | **port-bug** | copy to fresh non-`-U` array |
| 235 | `typeset -m "glob"` errors "not valid in this context" instead of pattern-matching | **port-bug** | loop with `${(@k)parameters[(I)pat]}` |
| 236 | `${(@)arr:#}` empty-pattern filter doesn't filter empty elements (sparse-arr cleanup broken) | **port-bug** | `:#(#e)` extended-glob anchor |
| 237 | `${funcfiletrace}`/`${functrace}` format diverges — missing file path + line | **port-bug** | (no clean workaround) |
| 238 | `setopt promptbang` doesn't enable `!` → history-number in prompts | **port-bug** | use `%h`/`%!` percent-escape |
| 239 | `print -P "%J"` (jobs count prompt escape) treats as literal — extends #38 family | **port-bug** | use `${#jobstates}` introspection |
| 240 | `{ false } always { :; }` clears errflag — errexit doesn't fire after always block | **port-bug** | explicit `\|\| _err=$?` propagation |
| 241 | `${assoc[@]}`/`${assoc[*]}` return empty — basic assoc value iteration broken | **port-bug** | `${(@v)h}` explicit value flag |
| 242 | Introspection assocs (`builtins`/`parameters`/etc.) not read-only — writes silently accepted | **port-bug** | (none — needs `PM_READONLY` at init) |
| 243 | `${(t)TIMEFMT}` etc. return empty for autovar/special params (type intro broken) | **port-bug** | `(( ${+VAR} ))` existence check only |
| 244 | `${(Z+c+)cmd}` word-split with c option returns whole string as 1 token (parser-aware split broken) | **port-bug** | manual `${(s: :)cmd%%\\#*}` for simple cases |
| 245 | `(#cN,M)` extended_glob range-repetition not honored (count syntax accepted but matcher ignores) | **port-bug** | explicit char-class repetition |
| 246 | `setopt rc_expand_param` applied inside double quotes — silently fans out `"prefix$arr"` per-element | **port-bug** | use `${a[*]}` star to force scalar-join |
| 247 | `read line` doesn't strip leading/trailing IFS whitespace from input | **port-bug** | explicit `${var## }`/`${var%% }` strip |
| 248 | `read "?prompt" var` writes prompt to stdout when stdin isn't tty (contaminates output) | **port-bug** | conditional `print -n "p: " >&2` |
| 249 | `${arr//pat/repl}` bare-array form applies replacement per-element (zsh: joins to scalar first) | **port-bug** | use `${${arr[*]}//pat/repl}` |
| 250 | `typeset -ia`/`typeset -Fa` accepted (zsh: errors "inconsistent type for assignment") | **port-bug** | split into `-a` then per-elem arith |
| 251 | `command -- echo hi` doesn't recognize `--` end-of-options separator | **port-bug** | drop the `--` |
| 252 | `exec -` (dash, no command) errors "exec: -: not found" instead of no-op | **port-bug** | drop the `exec -` |
| 253 | `$0` inside sourced file (top-level) isn't updated — breaks `${0:A:h}` plugin-dir pattern | **port-bug** | pass file path as parameter explicitly |
| 254 | `UID=N`/`EUID=N` writes silently accepted (zsh: attempts setuid syscall) — privilege-drop silent no-op | **port-bug** | use external setuid wrapper |
| 255 | `local -h NAME` doesn't hide parent scope's value — `-h` flag ignored | **port-bug** | explicit `local NAME=""` before set |
| 256 | `zstyle -e CONTEXT STYLE CODE` dynamic-eval not honored — completion-system breakage | **port-bug** | use static `zstyle` form |
| 257 | `jobtexts[N]` introspection assoc not populated for bg jobs (introspection-mirror family) | **port-bug** | parse `jobs -l` output instead |
| 258 | `printf "%d" $hugenum` silently overflows to 0 (zsh: errors "number truncated") | **port-bug** | pre-validate range before printf |
| 259 | `${jobstates[N]}` and `${jobdirs[N]}` empty (companion to #257 — all 3 job-intro assocs broken) | **port-bug** | parse `jobs -l` output |
| 260 | `${widgets[NAME]}` zle widget intro assoc empty (zsh: 386 builtin widgets) | **port-bug** | `zle -la NAME` for existence test |
| 261 | `${functions_source[fn]}` format diverges — `zsh:0` instead of bare `zsh` for cmdline fn | **port-bug** | strip `:N` suffix before compare |
| 262 | `zsh_eval_context` array always empty (zsh: `cmdarg`, `cmdarg shfunc`, etc. — eval context stack) | **port-bug** | manual `${funcstack[*]}` (also broken) |
| 263 | `$ZSH_DEBUG_CMD` empty when DEBUG trap fires (parameter never populated with cmd text) | **port-bug** | (none — step-debug not possible) |
| 264 | `${widgets[fn]}` after `zle -N fn` returns literal `builtin` for all queries (no real lookup) | **port-bug** | parallel user-managed assoc |
| 265 | `$MATCH` not populated by `(#m)` flag in `${var/pat/repl}` substitution (works in `=~`) | **port-bug** | use `=~` then manually substitute |
| 266 | `$match[N]` backref array not populated by `(#b)` in substitution | **port-bug** | use `=~` to capture, build repl manually |
| 267 | Bare `setopt` (no args) prints nothing instead of listing currently-set options | **port-bug** | use `set -o` (POSIX form) |
| 268 | Autovars `LISTMAX`/`MAILCHECK`/`KEYTIMEOUT`/`PERIOD` typed `scalar` instead of `integer` | **port-bug** | explicit `typeset -i NAME` after init |
| 269 | `$SPROMPT` autovar default empty (zsh: `zsh: correct '%R' to '%r' [nyae]?`) | **port-bug** | explicit `SPROMPT=...` init in .zshrc |
| 270 | `${(t)watch}` autovar absent — zsh: `array-special`; `watch` user-tracking feature dead | **port-bug** | (none — feature not implemented) |
| 271 | `h=([k]=v)` bash-style assoc init treated as glob — errors "no matches found" | **port-bug** | use flat-pairs `h=(k v k v)` instead |
| 272 | `typeset -axU` `-U` dedup not applied when combined with `-x` (export) | **port-bug** | separate into `typeset -aU` then `typeset -x` |
| 273 | `TIMEFMT` autovar in inconsistent state — value reads correctly but `(t)`/`${-NONE}` signal "unset" | **port-bug** | explicit `[[ -z ]]` check before := defaulting |

Of two hundred and seventy-three entries, two are fixed (5, 7), two
hundred and sixty-seven remain open port-bugs/perf-issues (4, 8, 9, 10,
11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44,
45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61,
62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78,
79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95,
96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109,
110, 111, 112, 113, 114, 115, 116, 117, 118, 119, 120, 121, 122,
123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135,
136, 137, 138, 139, 140, 141, 142, 143, 144, 145, 146, 147, 148,
149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161,
162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174,
175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187,
188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200,
201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213,
214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226,
227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239,
240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252,
253, 254, 255, 256, 257, 258, 259, 260, 261, 262, 263, 264, 265,
266, 267, 268, 269, 270, 271, 272, 273), and four were zsh-correct
behavior misframed by demos (1, 2, 3, 6).
