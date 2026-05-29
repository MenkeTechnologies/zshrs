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

Of the original 7, two are now fixed (5 + 7), two remain open as
port-bugs (4 + 8 surfaced during 7's diagnosis), and four were
zsh-correct behavior misframed by the demos.
