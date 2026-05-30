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

## #19 — Quoted special-char / reserved-word case patterns fail (non-first branch in fn)

**Status:** `port-bug` — surfaced 2026-05-29 writing demos 363, 365.

```sh
$ /opt/homebrew/bin/zsh -fc 'f() { case $1 in plain) echo p;; "!") echo b;; *) echo o;; esac; }; f "!"'
b

$ zshrs --zsh -c 'f() { case $1 in plain) echo p;; "!") echo b;; *) echo o;; esac; }; f "!"'
zsh:1: expected ')' in case pattern
zshrs: parse error

$ /opt/homebrew/bin/zsh -fc 'f() { case $1 in plain) echo p;; '\''if'\'') echo b;; *) echo o;; esac; }; f if'
b

$ zshrs --zsh -c 'f() { case $1 in plain) echo p;; '\''if'\'') echo b;; *) echo o;; esac; }; f if'
zsh:1: expected ')' in case pattern
zshrs: parse error
```

Two distinct fail-triggers share the same error message:

  1. **Quoted special chars**: `"!"`, `"?"`, `"*"`, `"["`, `"{"`, `"}"`
  2. **Quoted reserved words**: `'if'`, `'while'`, `'do'`, `'then'`, `'else'`, `'let'`

Both fail **only when** the case branch is NOT the first branch in
the `case` block AND the surrounding code is inside a function.
The exact same patterns parse correctly when:
  - the branch is FIRST in the case (no prior `;;` before it), OR
  - the case is at top level outside any function

| pattern    | inside fn, first branch | inside fn, after another | top level |
|------------|------------------------|---------------------------|-----------|
| `"!")`     | works                   | **FAILS**                  | works      |
| `'if')`    | works                   | **FAILS**                  | works      |
| `'while')` | works                   | **FAILS**                  | works      |
| `'foo')`   | works                   | works                      | works      |
| `plain)`   | works                   | works                      | works      |

Tested zsh 5.9 (`/opt/homebrew/bin/zsh`) handles all forms correctly.

**Where** — likely `src/ported/parse.rs::par_case` interaction with
the case-branch separator (`;;` or newline) processing inside the
function-frame parser path. The lexer (`Src/lex.c::gettok` port at
`src/ported/lex.rs`) appears to forget that a quoted token in case
position should not be interpreted as a keyword/operator, but the
forgetting only happens after the first branch boundary inside a
function.

Related to bugs #13 (quoted `"?"` ignored in `[[ == ]]`) and #14
(`[[ $ch == "{" ]]` parse error) — all share the same root cause:
zshrs's lexer not respecting quote boundaries for special tokens
in certain contexts.

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

Demo 363 dispatches unary `'!'`/`'~'` via `if/elif`. Demo 365
moves all reserved-word case branches before any other branches and
uses a leading `if [[ $head == ... ]]; then ...; return; fi` guard
chain for `quote`, `if`, `let` to bypass the bug entirely.

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
| 19 | quoted special/keyword case pat (non-first branch in fn) | **port-bug** | 363/365 reorder branches or if/elif |
| 20 | recursive parsers very slow vs C-zsh | **perf-issue** | 362 trimmed test inputs |

Of twenty entries, two are fixed (5, 7), fourteen remain open
port-bugs/perf-issues (4, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
19, 20), and four were zsh-correct behavior misframed by demos
(1, 2, 3, 6).
