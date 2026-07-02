# PORT.md — Rules for Bots Contributing to `zshrs`

`zshrs` is a **1:1 Rust port of zsh**. The goal is 100% behavioral parity
with upstream zsh. This is **not** a reimplementation, not a rewrite, not
"inspired by" zsh. Every line of Rust code must trace back to a specific
line of upstream C code in `src/zsh/Src/`.

If you are a bot (Copilot, Claude, GPT, Cursor, Aider, any LLM agent),
**read this file before writing a single line of code**. Violations are
deleted on sight by the maintainer. No exceptions.

---

## READ THIS FIRST — The Four Rules in One Screen

If you read nothing else in this file, read this. Every violation is
deleted on sight; the maintainer does not negotiate.

Do not use fully qualified names that are not in C.  C imports the names.  So Rust does too.  No imports inside functions, only imports at top of file organized by file.

### Rule 0 — ASK BEFORE INVENTING ANY NEW FN/STRUCT/STATIC NAME

**This rule overrides every other rule below.** If you (the bot) catch
yourself about to write a `fn`, `struct`, `enum`, `type`, or `static`
under `src/ported/` whose name does NOT exist in upstream zsh C
source, you must **STOP and ASK THE MAINTAINER FIRST**. You do not
get to:

- "just add a tiny helper because it's only 3 lines"
- "factor out a Rust-only wrapper for borrow-checker reasons"
- "add a `_take`/`_set`/`_get`/`_clear`/`_is_some`/`_fill_*`/
  `_check_*` accessor for a thread_local"
- "split one C function into `foo` + `foo_impl` for argument routing"
- "add a Rust-only sentinel like `LEX_TABS_INITED` or `PARSER_*_DEPTH`"
- "introduce a `*State`/`*Table`/`*Builder`/`*Config`/`*Context`
  aggregate"
- "add an `error()`/`set_error()`/`check_limit()`/`check_recursion()`
  paranoia helper"

even if the helper looks "obviously useful," "trivially small,"
"locally scoped," "obviously safe," or "what any reasonable Rust
programmer would do." **None of those are reasons. Permission is
the only reason.**

**The required flow when you think a Rust-only helper is needed:**

1. **STOP**. Do not write the helper.
2. State to the maintainer: *"I'm about to add `fn <name>` (or
   `struct <Name>` / `static <NAME>`) under `src/ported/<file>.rs`
   because <one-sentence reason>. This name does not exist in
   upstream zsh C. May I proceed?"*
3. **Wait for explicit permission.** Phrases that count as
   permission: "yes", "y", "ok", "go", "approved", "fine". Anything
   else — silence, "let me think", "why?", "what about X instead?"
   — is NOT permission.
4. If permission is granted, add the name AND immediately also add
   it to `tests/data/fake_fn_allowlist.txt` with the maintainer's
   approval recorded in the commit message ("approved 2026-MM-DD").
5. If permission is denied, the work goes back to either (a) using a
   real C-named port, (b) inlining the logic at call sites, or
   (c) abandoning the change.

**Past damage caused by skipping this gate** (each of these was a
bot inventing a fn without asking; all deleted in 2026-05 cleanup
sessions):

- `process_heredocs`, `read_line` (lex.rs) — fake heredoc body
  collector, replaced by real port of `gethere()` (exec.c:4573)
- `heredocs_take`/`heredocs_set`/`heredocs_clear`/`heredocs_len`/
  `heredocs_clone`/`heredocs_push`/`heredocs_is_empty`/
  `heredocs_fill_next_unprocessed` — 8 fake thread_local accessors
- `hdocs_push_back`/`hdocs_pop_front`/`hdocs_clear` — 3 fake
  linked-list helpers (C inlines the walk at call sites)
- `check_limit`/`check_recursion` + `PARSER_RECURSION_DEPTH`/
  `PARSER_GLOBAL_ITERATIONS` thread_locals — Rust-only paranoia
  counters; C has none
- `tokstr_is_none`/`tokstr_is_some`/`tokstr_eq`/`tokstr_take` —
  4 fake convenience wrappers around `LEX_TOKSTR.with_borrow(...)`
- `error`/`set_error`/`tokens_to_printable`/`read_cond_num`/
  `par_cmd_wordcode_noargs`/`resolve_redir`/`peek_inoutpar` —
  7 more fakery wrappers
- 14 `*Table` / `*State` aggregates in parameter.rs (2026-05-10
  dissolution) — bag-of-globals anti-pattern

Total cleanup cost of past unauthorized helpers: **30+ fns deleted,
700+ lines of dead Rust-only paranoia + glue, weeks of audit work.**
The permission gate exists because that cost is real.

**Test enforcement:** `tests/ported_fn_names_match_c.rs` rejects any
fn under `src/ported/` whose name is neither in
`docs/zsh_c_functions.txt` nor in
`tests/data/fake_fn_allowlist.txt`. Adding a new name to the
allowlist without prior maintainer approval is itself a violation —
the allowlist is not a free pass, it's the audit trail of granted
exceptions.

---

**Rule A — Names must exist in upstream zsh C.** This applies to
**every declaration** in `src/ported/`, not just functions:

| Rust decl                                  | Must exist in C as                                          | Verify with                                                                                |
|--------------------------------------------|-------------------------------------------------------------|--------------------------------------------------------------------------------------------|
| `fn <name>`                                | `<name>(` function definition                               | `grep -xF '<name>' docs/zsh_c_functions.txt`                                               |
| `struct <Name>` / `enum <Name>`            | `struct <name>` / `union <name>` / `enum <name>` / `typedef` | `grep -nE '(struct\|union\|enum)[[:space:]]+<name>' src/zsh/Src/**/*.{c,h}`                |
| `static <NAME>` / `thread_local! { static <NAME> }` | `static <name>` (file-scope) or `extern <name>` in C  | `grep -nE 'static[[:space:]].*[^a-zA-Z0-9_]<name>[^a-zA-Z0-9_]' src/zsh/Src/**/*.c`         |
| `type <Name>`                              | `typedef ... <name>`                                        | `grep -nE 'typedef[[:space:]].*[^a-zA-Z0-9_]<name>[[:space:]]*;' src/zsh/Src/**/*.{c,h}`   |

If `grep` returns nothing, the name is invented. **Delete or rename.**

**Rule B — Signatures must be identical to C (Rule S1).**

C `paramtypestr(Param pm)` → Rust `paramtypestr(pm: &param)`. NOT
`paramtypestr(ptype: ParamType, flags: &ParamFlags)`. No threading
state through as extra params. No splitting one C fn into many Rust
fns. No merging many C fns into one. No reordering params. No renaming
params for "Rust idiom" (`nam`, not `name`; `argv`, not `args`; `ops`,
not `opts`). The C→Rust type map is in `## EXACT TRANSLATION` below.

**Rule C — Every decl lives in the file that mirrors its C
definition file.**

| C definition is in...     | Rust port goes in...                  |
|---------------------------|---------------------------------------|
| `Src/foo.c`               | `src/ported/foo.rs`                   |
| `Src/Zle/foo.c`           | `src/ported/zle/foo.rs`               |
| `Src/Modules/foo.c`       | `src/ported/modules/foo.rs`           |
| `Src/zsh.h`               | `src/ported/zsh_h.rs`                 |
| `Src/Zle/zle.h`           | `src/ported/zle/zle_h.rs`             |
| `Src/Zle/comp.h`          | `src/ported/zle/comp_h.rs`            |
| `Src/Zle/compctl.h`       | `src/ported/zle/compctl_h.rs`         |

**Same layout rule for the compsys shell-function ports** (the second
strict-port directory):

| Shell source                              | Rust port goes in...                       |
|-------------------------------------------|--------------------------------------------|
| `Completion/Base/Completer/_<NAME>`       | `compsys/ported/Base/Completer/_<NAME>.rs` |
| `Completion/Base/Core/_<NAME>`            | `compsys/ported/Base/Core/_<NAME>.rs`      |
| `Completion/Base/Utility/_<NAME>`         | `compsys/ported/Base/Utility/_<NAME>.rs`   |
| `Completion/Base/Widget/_<NAME>`          | `compsys/ported/Base/Widget/_<NAME>.rs`    |
| `Completion/Unix/Type/_<NAME>`            | `compsys/ported/Unix/Type/_<NAME>.rs`      |
| `Completion/Zsh/Type/_<NAME>`             | `compsys/ported/Zsh/Type/_<NAME>.rs`       |

`compsys/ported/` is governed by the same four rules as `src/ported/`:
no invented helpers, no bag-of-globals aggregators, one file per
upstream shell function, header doc-comment cites the upstream source
(`//! Port of _<NAME>` + path), faithful flag/zstyle/algorithm match.
Stub bodies fail the per-fn unit tests under
`cargo test -p compsys --lib 'ported::'`.

A struct declared in `Src/zsh.h` does not belong in
`src/ported/modules/parameter.rs` just because parameter.c uses it.
Same for fns: `Src/utils.c` fns → `src/ported/utils.rs`, never
re-homed to "wherever they're called from."

**Rule D — Bag-of-globals aggregator types are banned.**

❌ C declares N file-`static`s → Rust aggregates them into one struct.

If C has `static int foo; static int bar; static int baz;` (three
separate file-statics), the Rust port has three separate
`thread_local!` entries (bucket 1) or three separate
`Arc<RwLock<…>>` holders (bucket 2). **No struct `BagState { foo,
bar, baz }`** unless C declares `struct bag_state { ... }`.

This is the failure mode that produced parameter.rs's 14 deleted
`*Table` structs (`CommandsTable`, `FunctionsTable`, `AliasesTable`,
`BuiltinsTable`, `DirStack`, `OptionsTable`, `NamedDirsTable`,
`JobsTable`, `ModulesTable`, plus `ParamFlags`, `ParamType`,
`FunctionDef`, `AliasDef`, `ModuleInfo`) — none of those names
existed in `Src/Modules/parameter.c`; all 14 were invented Rust
abstractions; all 14 were deleted in commit "port: dissolve
bag-of-globals from modules/parameter.rs" on 2026-05-10.

See PORT_PLAN.md "Anti-pattern 1" and Phase 2 dissolution checklist
for the active list of remaining bag-of-globals targets (`SubstState`,
`ShellState`, `JobState` in parameter.rs/jobs, `GlobState` in glob.rs
needing field-parity verification). `CompletionState`, `BindState`,
`RefreshState`, `CompState` in computil.rs landed 2026-05-10.

**Rule E — Local variables must use the C source's exact names.**

This applies to **every variable inside a function body**, not just
function names and file-level declarations:

```c
static void setfunction(char *name, char *val, int dis)
{
    char *value = dupstring(val);
    Shfunc shf;
    Eprog prog;
    int sn;
    ...
    val = metafy(val, strlen(val), META_REALLOC);
    ...
}
```

Rust port — same names, same order, same scope:

```rust
pub fn setfunction(name: &str, mut val: String, dis: i32) {
    let value: String;             // c:286 char *value
    let shf: ShFunc;               // c:287 Shfunc shf
    // c:288 — Eprog prog (skipped: parse_string not yet ported)
    // c:289 — int sn (used inside the TRAP branch only)
    value = val.clone();           // c:286
    val = metafy(&val);            // c:291 val reassigned, NOT a new `metafied`
    ...
}
```

❌ **Forbidden renames inside function bodies**:
- `nam` → `name`, `argv` → `args`, `ops` → `opts` (params)
- `val` → `metafied`, `value` → `dupval`, `s` → `string` (locals)
- `hn` → `hn_opt` / `hn_node` (loop iterators — even when the Rust
  type is `Option<T>`, keep the C name and pattern-match on it)
- `i` → `idx`, `n` → `count`, `ret` → `result`
- Combining multiple C locals into a tuple or struct
- Reordering local declarations
- Using a `for i in 0..N` loop when C declared `int i;` at function
  top — `i` must be a function-scope local, not a loop-scope local
- Dropping a local just because Rust doesn't force you to declare it

**The C name is the canonical name.** If the C author chose `val` for
a variable that holds a meta-fied string, the Rust port keeps `val`.
"Rust idiom" is not an excuse to rename anything — same convention
that applies to function params per Rule B.

The Option<T> case: when C has `HashNode hn` iterated via
`hn = ht->nodes[i]; hn; hn = hn->next`, the Rust port keeps `hn` as
an `Option<HashNode>` and unwraps inside the loop body without
renaming:

```rust
let mut hn: Option<HashNode>;                  // c:347 HashNode hn
hn = ht_ref.nodes.get(i as usize).and_then(|n| n.clone());  // c:353
while let Some(node) = &hn {
    // process node.nam, node.next, ...
    hn = node.next.clone();                    // c:353 hn = hn->next
}
```

The inner `node` binding is the unwrap, not a rename. `hn` stays the
function-scope name C uses.

---

## ABSOLUTE FREEZE on `src/ported/`

**`src/ported/` is FROZEN. No new files. No new functions whose name is
not already in zsh's C source. No exceptions.**

These are hard, blanket bans — they override every previous "you may
create the matching Rust file" or "find a C file with no Rust
counterpart yet" instruction in this document. If older text in this
file conflicts with the freeze below, the freeze wins.

### File freeze (`src/ported/**.rs`)

- ❌ **You may NOT create any new `.rs` file under `src/ported/`.** Not
  for a port that doesn't have a Rust home yet, not for a "tiny one,"
  not even if the matching `Src/<x>.c` is sitting right there.
- The legal file set is the **106 files currently checked in** under
  `src/ported/` (run `find src/ported -name '*.rs' | sort` to see the
  exhaustive list). That set is the universe. New ports either land in
  one of those 106 files or they do not land at all.
- ❌ No new directories under `src/ported/` either. The directory tree
  is frozen identically to the file tree.
- If a C function genuinely belongs in `src/ported/<x>.rs` and that file
  doesn't exist, **stop**. Do not create the file. Do not "temporarily"
  put the port in a different file. Raise it with the maintainer; the
  freeze is intentional. New file creation is a one-line PR by the
  maintainer, not a side-effect of a port.

### Function freeze (`fn` names in `src/ported/**.rs`)

- ❌ **You may NOT introduce any new `fn` under `src/ported/` whose
  name does not already exist as a function in upstream zsh C source**
  (`src/zsh/Src/**/*.c`). Verify every name with:

  ```sh
  grep -nE '^<name>\(' src/zsh/Src/**/*.c
  # or use the cached allowlist:
  grep -xF '<name>' docs/zsh_c_functions.txt
  ```

- If `grep` returns nothing, the name is invented. Invented names are
  the drift signature — they are how `shell_quote`, `get_user_home`,
  `is_directory`, `find_in_path`, etc. ended up in the tree. Do not
  add another one.
- The Rule 3 exemptions for trait-impl methods (`fn new`, `fn drop`,
  `fn fmt`, `fn clone`, `fn from`, `fn next`, `fn poll`, etc.) and for
  `#[test]` functions still apply. Nothing else is exempt.
- ❌ Do not "rename to fit." Reusing a real zsh name like `metafy` for
  a function that does something different is worse than inventing a
  new name. Cite the C source line; if the behavior doesn't match, the
  port is wrong.

### Struct / enum / typedef / static freeze

Equally non-negotiable as the function freeze, and the failure mode
that produced parameter.rs's 14 bag-of-globals `*Table` structs (all
deleted 2026-05-10):

- ❌ **You may NOT introduce any `struct`, `enum`, `type`, `union`,
  or top-level `static` under `src/ported/` whose name does not
  already exist as a `struct` / `enum` / `typedef` / `static` in
  upstream zsh C source.** Verify with:

  ```sh
  # Does this struct exist anywhere in upstream zsh C?
  grep -nE '(typedef[[:space:]]+)?(struct|union|enum)[[:space:]]+<name>[[:space:]]*[{;]' \
    src/zsh/Src/**/*.c src/zsh/Src/**/*.h
  ```

- ❌ Invented Rust-only aggregator structs are the **bag-of-globals
  anti-pattern** documented in PORT_PLAN.md. If C declares N
  file-`static`s, Rust declares N `thread_local!` entries (bucket 1)
  or N `Arc<RwLock<…>>` holders (bucket 2). **No aggregation
  struct unless C has a matching `struct foo` to point at.**
- ❌ Invented Rust-only "convenience" types — `OptionsTable`,
  `JobsTable`, `CommandsTable`, `*State`, `*Builder`, `*Config`,
  `*Context` — are deleted on sight when their C counterpart does
  not exist in the matching `Src/<file>.c` (or in the canonical
  header file for header-defined types).
- ❌ Trivial wrapper `enum`s that re-encode `PM_*` / `OPT_*` /
  `BIT_*` constants as Rust enum variants. The C source uses bit
  constants directly — the Rust port reads the same `u32` flag bits
  via `PM_TYPE(f)` and `(f & PM_LEFT) != 0`, NOT
  `match ParamType::Scalar => …`.

#### The legal struct/enum name set

A Rust `struct` / `enum` / `type` / `static` name under `src/ported/`
is **legal** if and only if it is one of:

1. **Identical** to a name declared as `struct <name>` / `union <name>`
   / `enum <name>` / `typedef ... <name>` / `static ... <name>` in
   any `src/zsh/Src/**/*.c` or `src/zsh/Src/**/*.h`.
2. **Uppercased** version of a C file-`static` (Rust SCREAMING_SNAKE
   convention) — e.g. C `static int unary` → Rust `static UNARY` or
   `thread_local! { static UNARY: ... }`.
3. A standard Rust derive-required impl (`Default`, `Clone`,
   `Debug`, etc.) on a struct that already exists in C.
4. A `#[cfg(test)]` test fixture inside `mod tests { ... }`.

Anything else — `CommandsTable`, `FunctionsTable`, `AliasesTable`,
`OptionsTable`, `JobsTable`, `ParamFlags`, `ParamType`, `ShellState`,
`SubstState`, `BindState`, `RefreshState`, `CompletionState`, etc. —
**will be deleted**. The 14-struct dissolution of parameter.rs on
2026-05-10 (commit "port: dissolve bag-of-globals from
modules/parameter.rs") is the reference example of how this rule is
enforced retroactively.

#### Header-defined types live in the header port

Structs and typedefs declared in `Src/zsh.h`, `Src/Zle/zle.h`,
`Src/Zle/comp.h`, `Src/Zle/compctl.h`, etc. port to their canonical
header file:

| C header              | Rust port                              |
|-----------------------|----------------------------------------|
| `Src/zsh.h`           | `src/ported/zsh_h.rs`                  |
| `Src/Zle/zle.h`       | `src/ported/zle/zle_h.rs`              |
| `Src/Zle/comp.h`      | `src/ported/zle/comp_h.rs`             |
| `Src/Zle/compctl.h`   | `src/ported/zle/compctl_h.rs`          |
| `Src/Modules/tcp.h`   | `src/ported/modules/tcp_h.rs`          |

A struct defined in `Src/zsh.h` (e.g. `struct funcstack` at
`zsh.h:1348`) does NOT belong in `parameter.rs` just because
`parameter.c` uses it. It belongs in `zsh_h.rs`. Cross-file
misplacement is a violation.

### Why this freeze exists

Bots have produced 114 distinct duplicate function names across
`src/ported/`, with 148 extra copies and 1,638 self-confessed
`// c:N/A` adhoc functions. Every drift originated as either (a) a new
file the bot created to "have somewhere to put" a helper, (b) a new
function name the bot invented because no real zsh fn matched, or (c)
a new `*State` / `*Table` / `*Builder` struct invented to aggregate
file-statics into "one parameter to thread through." The freeze
closes all three vectors at the source. From this point forward, the
only edits permitted under `src/ported/` are: (i) modifying an
existing fn in an existing file to be a more faithful port, (ii)
adding a fn whose name already exists in upstream C code to an
existing Rust file that already maps to that C file's `Src/*.c`
counterpart, (iii) deleting drift, (iv) dissolving bag-of-globals
`*State`/`*Table` aggregates into per-static `thread_local!` sets
(see PORT_PLAN.md Phase 2).

Genuinely new code (features zsh C does not have) goes to
`src/extensions/`. The recorder goes to `src/recorder/`. Nothing else
moves.

---

## Scope: `src/ported/` Is Strict-Port Territory

**Every Rust file currently under `src/ported/` (and every file ever
added under `src/ported/`) is bound by every rule in this document —
no grandfathering, no "legacy" exemptions, no "we'll fix it later".**

If a file lives under `src/ported/`:

- It **must** mirror a real C file under `src/zsh/Src/` (same stem, same
  relative subpath — see the mapping table below).
- Every `fn` in it **must** carry the `/// Port of <c_name>() from
  Src/...:NNNN` doc-comment (Rule 2).
- Every `fn` name **must** appear in `docs/zsh_c_functions.txt` or be one
  of the narrow Rule 3 exemptions (trait-impls, tests).
- Every line **must** trace back to a specific upstream C line. No
  invented helpers, no "cleaner" abstractions, no idiomatic-Rust
  refactors, no convenience wrappers.

A file under `src/ported/` that fails any of these tests is treated as
adhoc code and is deleted on sight regardless of when it was added,
who added it, or how much of the build depends on it. Fix it by porting
it properly, move it to `src/extensions/` if it implements a feature
zsh C does not have, or delete it. There is no fourth option.

The only two locations in the entire tree where non-ported code may
exist are `src/extensions/` and `src/recorder/` (see Rule 1). The two
crate-root files `src/exec.rs` and `src/fusevm_bridge.rs` are the only
sanctioned non-port files outside those two directories — they are
explicitly carved out below and are **not** a precedent for adding more.

---

## NO SHORTCUTS — 100% LINE-BY-LINE COVERAGE

When the maintainer asks to port a C source file, the result is a
**complete 1:1 port**, not a partial one. "Faithful port" means
every function, struct, enum, `#define`, table, and module-static
the C source defines has a real Rust counterpart with matching
name, signature, and control flow.

The forbidden pattern (caught repeatedly in earlier rounds):

❌ Port `bin_<name>` and the dispatch table → ship the rest as
   `WARNING: NOT IN <FILE>.C` stubs / `*_notavail` placeholders /
   bare `0`-returning entries → claim "faithful port".

This is the failure mode that triggered "what do I have to fucking
check every LOC to make sure its a real port b/c ur lazy/liar?"

### When stubs ARE acceptable

The `// WARNING: NOT IN <FILE>.C` marker is **only** appropriate
when the C definition genuinely lives in a *different* `Src/*.c`
file. Examples that pass review:

- `zsetlimit` / `setlimits` ported alongside rlimits.rs because
  their canonical home is `Src/exec.c`, not `rlimits.c`. Marker
  with file:line citation is correct.
- `addtimedfn` / `deltimedfn` referenced by sched.rs because they
  live in `Src/utils.c`. Marker with file:line citation is correct.

The marker is **not** appropriate for any function defined in the
*same* C source file the port covers. If `Src/Modules/curses.c`
defines `zcurses_validate_window` and the Rust port skips it, that
is the lazy pattern. Port it.

### Audit requirement before declaring a port done

Before writing the commit message, run a sanity check:

```sh
# List every function defined in the C source file:
grep -nE '^[a-zA-Z_][a-zA-Z_0-9]*\(' src/zsh/Src/Modules/<file>.c

# List every fn in the Rust port:
grep -nE '^(pub )?(pub\(crate\) )?fn ' src/ported/modules/<file>.rs

# Walk both lists side-by-side. Every C name must appear in the
# Rust list. Stubs that don't match a different-file definition
# are blockers — not "follow-up commits".
```

Plus the bag-of-globals anti-pattern from PORT_PLAN.md applies in
full. C `static` fields → thread_local!/OnceLock<Mutex>, never a
Rust struct that aggregates them all.

---

## EXACT TRANSLATION — Same Names, Same Types, Same Calls, Every Line Cited

A true port is a **LINE-BY-LINE EXACT TRANSLATION** of the C source.
"Faithful port" is not a vibe; it is a checklist of literal
correspondences any reviewer can audit in seconds. If your port fails
any of the rules below, it is not a port — it is a paraphrase, and it
gets deleted.

### 1. Argument names match C exactly (case-sensitive)

C `bin_unlimit(char *nam, char **argv, Options ops, UNUSED(int func))`
ports as Rust `bin_unlimit(nam: &str, argv: &[String], ops: &options, _func: i32)`.

- `nam`, NOT `name`. `argv`, NOT `args`. `ops`, NOT `opts`. `lim`,
  NOT `limit`. `func`, NOT `funcid`. The C name is the canonical
  name; renaming for "Rust idiom" is a violation.
- UNUSED parameters in C stay as parameters in Rust, with a leading
  underscore: `_func: i32`. Never delete a parameter just because
  it is unused — call sites bind to position.

### 2. Argument datatypes match C through the canonical type map

| C type        | Rust type                          |
|---------------|------------------------------------|
| `char *`      | `&str`                             |
| `char **`     | `&[String]`                        |
| `int`         | `i32`                              |
| `Options`     | `&options` (`= struct options *`)  |
| `rlim_t`      | `rlim_t` (libc-typed)              |
| `uid_t`       | `libc::uid_t`                      |
| `void *`      | context-dependent — match nearest semantic |
| `struct foo *`| `&foo` (read) / `&mut foo` (write) |

If the right-hand-side type doesn't exist as a Rust port yet, port
the underlying `struct` first (matching name + fields), then use it.
Do NOT substitute a `[bool; 256]` for `Options ops`, do NOT pass split
`hard: bool, soft: bool` instead of the real `&options` — the port
must round-trip through the same data shape C operates on.

### 3. Called function names match C exactly

If C calls `zstrtol(s, NULL, 10)`, Rust calls `zstrtol(s, 10)`.
NEVER `parse_number_i32`, NEVER `parse_leading_decimal`, NEVER
`atoi_safe`. Same for every helper: `getrlimit`, `setrlimit`,
`geteuid`, `OPT_ISSET`, `zwarnnam`, `zsetlimit`, `setlimits`,
`do_limit`, `do_unlimit`, `printrlim`, `printulimit`, `showlimits`,
`showlimitvalue`, `find_resource`, `set_resinfo`, `free_resinfo`,
`zstrtorlimt`. The drift gate (build.rs) rejects fn names with no C
counterpart for exactly this reason — if you find yourself wanting a
new helper name, the right move is either to find the existing C
function it duplicates or to stub the missing C function in its
proper file (per Rule 1 below) and call THAT.

### 4. Every line carries a `// c:NNN` citation

```rust
hard = OPT_ISSET(ops, b'h');                                       // c:526
if OPT_ISSET(ops, b's') && argv.is_empty() {
    return setlimits("");                                          // c:527-528
}
/* without arguments, display limits */                            // c:529
if argv.is_empty() {
    return showlimits(nam, hard, -1);                              // c:531
}
```

Every Rust statement that ports a C statement carries a comment with
the C source line. Block-level `// c:NNN-MMM` is acceptable for
contiguous chunks. The doc-comment above the `fn` cites the function
origin (existing Rule 2 below); the inline `// c:NNN` comments cite
each statement. Both are required.

### 5. Local variables: same names, same order, same scope

C declares locals at function top:

```c
char *s;
int hard, limnum, lim;
rlim_t val;
int ret = 0;
```

Rust mirrors that block:

```rust
let hard: bool;
let mut limnum: i32;
let mut lim: i32;
let mut val: rlim_t;
let mut ret: i32 = 0;
```

Same names (`hard`, `limnum`, `lim`, `val`, `ret`), same order, same
visibility (function-scope, not block-scope). Don't combine into
tuples, don't reorder, don't `let mut` only the ones Rust forces you
to — be conservative.

### 6. Control flow keeps C idioms

| C construct                              | Rust mirror                                      |
|------------------------------------------|--------------------------------------------------|
| `while ((s = *argv++))`                  | `let mut argi = argv.iter(); while let Some(s_owned) = argi.next() { let s = s_owned.as_str(); ... }` |
| `for (i = 0; i != N; i++)`               | `i = 0; while i != N { ...; i += 1; }` (preserve `!=`) |
| `for (i = 0; i < N; i++)`                | `for i in 0..N` (preserve `<`)                   |
| `do { ... } while (cond);`               | `loop { ...; if !(cond) { break; } }`            |
| `goto label;` / `label:`                 | labelled `'label: loop { ... break 'label; }`    |
| `if (a && (b = f()))`                    | preserve order — assignment-in-condition becomes a let-binding immediately before the if |

Don't "improve" C control flow into iterator chains. The structure of
the C code IS the structure of the Rust code.

### 7. C source comments port over

**This is a hard rule, not a "nice to have."** The C source's inline
comments encode load-bearing context that the code alone doesn't:
- WHY a flag combination is rejected ("see comment in optlookup()")
- WHICH bug a workaround fixes ("Apparently SunOS does X")
- WHEN a branch is reachable ("only if SUSER_RECURSE is set")
- WHAT the C author considered (and rejected) elsewhere
- The **WHY behind the WHAT** — every comment is a load-bearing
  intent record. Stripping them and reconstructing intent later is
  archaeology, not porting.

`/* without arguments, display limits */` becomes
`// without arguments, display limits` (Rust `//`) in the same
position, on the same line or block, as the C source. Don't drop
them, don't paraphrase them, don't translate idioms ("get the limit
in question" stays as-is).

**Required:**
- Every C inline comment in the function body MUST appear in the
  Rust port. Translate `/* ... */` to `//` line-form; preserve
  multi-line block comments as `//`-prefixed blocks at the same
  indentation.
- C function-header comments (the descriptive paragraph above the C
  signature) become Rust doc-comments (`///`) on the Rust port,
  alongside the `/// Port of <name>() from Src/<file>.c:<line>.`
  citation. Both go on the port, not just the citation.
- The C struct + global comments (e.g. the comment above `struct
  rmmagic` at jobs.c:537) carry over verbatim onto the Rust struct.
- File-header comments (the multi-line block at top of every C
  file describing copyright + purpose) become Rust `//!` module
  doc-comments at the top of the corresponding `.rs` file.

**Rust-only architectural notes** (e.g. "uses thread_local because
the C source's file-static doesn't survive Rayon workers", or
"!!! WARNING: RUST-ONLY HELPER !!!" blocks for borrow-checker
adapters) go in their **OWN** comment block, separately from the
verbatim C comment carry-overs. Don't conflate the two — a reader
must be able to tell at a glance whether a comment originated in
the C source or is a Rust-port-specific note.

**Checking for completeness:** before declaring a port faithful,
diff the C function's comment density against the Rust port. A
faithful port has comparable comment density. A C body with 30
inline comments porting to a Rust body with 3 comments is a
warning sign — comments were dropped.

### 8. Top-level declaration order matches C exactly

The order of `enum` / `struct` / `typedef` / `#define` / `static`
table / `static` global / function definitions in the Rust port
mirrors the order in the C source file, top to bottom.

- If `Src/Builtins/rlimits.c` declares `enum zlimtype` at line 35,
  `struct resinfo_T` at line 43, `static const resinfo_T known_resources[]`
  at line 60, `static const resinfo_T **resinfo` at line 190,
  then `set_resinfo()`, `free_resinfo()`, `find_resource()`,
  `printrlim()`, `zstrtorlimt()`, `showlimitvalue()`, `showlimits()`,
  `printulimit()`, `do_limit()`, `bin_limit()`, `do_unlimit()`,
  `bin_unlimit()`, `bin_ulimit()`, then the module loaders — the Rust
  port declares them in **that exact order**.
- This makes side-by-side review trivial: a reviewer with `rlimits.c`
  open in one pane and `rlimits.rs` in the other can scroll both at
  the same rate and check correspondence by eye.
- It also lets the `// c:NNN` citations climb monotonically down the
  Rust file. If two adjacent Rust fns cite `c:670` then `c:519`, the
  ordering is wrong — fix it before committing.
- Internal-helper Rust-only fns (allowlisted ones like `nlimits`,
  `ensure_limits_initialized`) sit alongside the C fn that needs them,
  not piled at the top or bottom. Same principle: a reviewer skimming
  the Rust file sees content in the same logical sequence as the C.
- Reorder ONLY when forced by Rust's compiler (e.g., a `pub use`
  re-export that must precede its consumers); document the deviation
  with a `// reordered from c:NNN — Rust requires X` comment.

### 9. Function bodies port too — never bare-`return` a fn whose body is unported

When porting a fn whose body depends on subsystems not yet ported
(param-table, locallevel, funcstack, options, ZLE globals), DO NOT
take the shortcut of "this depends on X which isn't ported, so the
whole body returns 1 / 0 / no-op." The body lives in the same C file
as the function declaration — by Rule 1 above, it must be ported in
full.

The correct approach for an in-file fn body:

1. Port the FULL C body line-by-line, every C statement → matching
   Rust statement with `// c:NNN` citation.
2. Stub the EXTERN dependencies (fns / globals from OTHER C files)
   locally with file:line citations to their home file. Stubs are
   minimal: `fn createparam(_n, _f) -> *mut param { std::ptr::null_mut() }`,
   `static funcstack: Mutex<usize> = Mutex::new(0);`, etc.
3. The body STILL EXECUTES — branches still take, increments still
   happen, mutations to module-local statics (e.g., `sh_edmode`,
   `sh_edchar`) still apply. The extern-dep return values produce a
   degenerate runtime trace until the real ports land.
4. Add a test that exercises the full body (e.g., flip the relevant
   global into the "execute everything" state) to prove the body
   actually runs without panicking and that increment/decrement
   bookkeeping nets to zero.

**Why the body shortcut is structurally worse than the file shortcut:**
the file-shortcut leaves a clearly-named gap. The body-shortcut
produces a fn that LOOKS like a complete port — same signature, same
name, drift gate green — but silently elides 80 lines of
state-mutating logic. When the dependent subsystem later lands, the
engineer has to re-examine the original C body to figure out what
should have been there. By porting the full body NOW, the eventual
integration is "swap the stubs for real impls," not "translate the C
from scratch a second time."

**Reference example:** `src/ported/modules/ksh93.rs::ksh93_wrapper`
after the 2026-05 fix — full c:152-227 ported with
`funcstack`/`locallevel`/`emulation`/`curkeymapname`/`varedarg` as
`Mutex`/`Atomic` statics matching the C global names, and
`createparam`/`setloopvar`/`setsparam`/`setiparam`/`getsparam`/
`getaparam`/`getiparam`/`isset` as local stubs citing their home C
file. Body executes top-to-bottom; branches take based on stub
return values; nothing is skipped.

### 10. The canonical reference: `src/ported/builtins/rlimits.rs`

When in doubt, read `src/ported/builtins/rlimits.rs` after the
2026-05 rewrite. It is the worked example. Every fn there shows the
above rules applied: exact arg names + types from `Src/Builtins/rlimits.c`,
called fns named identically (`OPT_ISSET`, `zwarnnam`, `zstrtol`,
`getrlimit`, `setrlimit`, `geteuid`, `do_limit`, `do_unlimit`,
`zsetlimit`, `setlimits`, `printrlim`, `printulimit`, `showlimits`,
`showlimitvalue`, `find_resource`, `set_resinfo`, `zstrtorlimt`),
locals at top mirroring the C `int hard, limnum, lim;` pattern, every
non-trivial statement carrying `// c:NNN`, dispatcher bridge code
extracted out to `src/extensions/ext_builtins.rs` (because it is the
analogue of `Src/builtin.c:execbuiltin` + `parseopts`, not part of
rlimits.c itself).

---

## The Three Hard Rules

### 1. PORT-ONLY. NO ADHOC IMPLEMENTATIONS.

You are translating C → Rust. You are not designing software.

- You **may** write a Rust function if and only if it is a port of a
  specific C function that exists in `src/zsh/Src/**/*.c`.
- You **may not** invent helper functions, utility wrappers, "cleaner"
  abstractions, traits, builders, or any other code that does not have
  a direct C counterpart in upstream zsh.
- "Refactoring for idiomatic Rust" is **forbidden**. The structure of
  the C code is the structure of the Rust code. Same function names
  (modulo the renaming rules below), same control flow, same globals,
  same field layout where feasible.
- If a C function uses `goto`, your Rust port uses labelled `loop`/
  `break` to mirror it. Do not "improve" it.
- If you cannot find a matching C function for code you want to write,
  **stop and do not write it**. Ask the maintainer or pick a different
  task.

#### The TWO and ONLY TWO exceptions

There are exactly two locations in the tree where new, non-ported code
is permitted to exist. **Nowhere else.** No matter how clean, useful,
or "obviously needed" your idea is — if it doesn't live in one of these
two places, it doesn't belong in the repo:

1. **`src/extensions/`** — the **only** place for features that zsh C
   does not have. This is where genuinely new functionality lives:
   anything that goes beyond upstream zsh's behavior (AOT compilation,
   daemon coordination, autoload caches, fish-style features, plugin
   caches, persistent worker pools, etc.). Code here is **not** a port
   and is not expected to map to any C function. Two strict rules
   still apply:
   - Every file under `src/extensions/` must implement a feature that
     zsh C demonstrably does **not** have. If a similar feature exists
     in zsh, port it instead — the port belongs under `src/ported/`.
   - Files under `src/extensions/` may not duplicate or shadow any
     port. They are additive only. If your "extension" is really a
     reimplementation of something zsh already does, delete it and
     port the C version.
2. **`src/recorder/` and `bins/zshrs-recorder.rs`** — the
   Plugin-Framework-Agnostic State-Modification Recorder, gated
   behind the `recorder` cargo feature. This subsystem has no zsh C
   counterpart by design (it is a development/debug tool that records
   shell-state mutations); it is sanctioned as a separate, feature-
   gated extension. Even here, code must be self-contained inside the
   `recorder` module — recorder code may not leak into other modules,
   and other modules may not depend on recorder code at compile time
   without the `recorder` feature flag.

Everything outside these two locations is a **port**. No exceptions.
No "this one little helper." No "just a quick utility module." No.

### 2. EVERY FUNCTION MUST CITE ITS C SOURCE.

Every `fn` in the Rust tree must carry a doc-comment of this exact form
immediately above the signature:

```rust
/// Port of `<c_function_name>()` from `Src/<subdir>/<file>.c:<line>`.
///
/// <one-line summary mirroring the C function's purpose>
pub fn <rust_name>(...) -> ... {
    ...
}
```

Required:
- The C function name in backticks with `()`.
- The path **relative to `src/zsh/`** (so `Src/builtin.c:1234`, not
  `src/zsh/Src/builtin.c:1234`).
- The line number of the C function's definition (the line with the
  return type / opening of the function, not the brace).

If the C function is large and split across helpers, each Rust helper
must cite the same C function and indicate the chunk:

```rust
/// Port of `bin_print()` from `Src/builtin.c:4521`
/// (chunk 3/7 — option parsing).
```

If the Rust code is a port of a *macro*, cite it the same way and note
`(macro)`:

```rust
/// Port of `STRINGIFY()` macro from `Src/zsh.h:128` (macro).
```

### 3. NAMES MUST EXIST IN UPSTREAM ZSH.

The allowlist of legal function names is in:

- **`docs/zsh_c_functions.txt`** — 2,484 unique C function names.
- **`docs/zsh_c_functions_with_locations.txt`** — same names with
  `Src/path.c:line` for cross-reference.

A Rust function name is **legal** if and only if it is one of:

1. **Identical** to a name in `zsh_c_functions.txt`
   (e.g. C `bin_print` → Rust `bin_print`).
2. A standard Rust trait-impl method (`fn new`, `fn drop`, `fn fmt`,
   `fn clone`, `fn default`, `fn from`, `fn into`, `fn as_ref`,
   `fn deref`, `fn eq`, `fn hash`, `fn partial_cmp`, `fn cmp`,
   `fn next`, `fn poll`, `fn serialize`, `fn deserialize`) — and only
   when it directly wraps a C function call or struct layout.
3. A Rust `#[test]` or `#[cfg(test)]` function — tests are exempt from
   the C-name rule but must still describe what C behavior they verify.

Anything else — `make_pretty_helper`, `parse_args_v2`, `init_state_new`,
`fancy_iter`, `RustyOptions::build`, etc. — **will be deleted**.

---

## File Layout: 1:1 with zsh — NO NEW FILES EVER

The Rust source tree is split into exactly **three** top-level
directories under `src/`:

| dir                | purpose                                                            |
|--------------------|--------------------------------------------------------------------|
| `src/ported/`      | The 1:1 port. Every file here mirrors a `Src/<...>.c`.             |
| `src/extensions/`  | Features zsh C does **not** have. The only sanctioned non-port dir.|
| `src/recorder/`    | Recorder subsystem (cargo `recorder` feature only).                |

> **Exception: `src/exec.rs` is NOT a ported file.** zshrs replaces
> zsh's tree-walking interpreter (`Src/exec.c::execlist` /
> `execpline` / `execcmd`) with a fusevm bytecode VM. There is no 1:1
> port of `Src/exec.c`. `src/exec.rs` instead holds the
> `ShellExecutor` runtime-state struct that everything (the VM, every
> ported builtin, every utility) threads through; the actual VM
> bridge lives in `src/fusevm_bridge.rs`. Both files live at the
> crate root, **not** under `src/ported/`, precisely because they
> aren't ports. `crate::ported::exec` is kept as a path alias
> (`pub use crate::exec;` in `ported/mod.rs`) so existing call-sites
> compile unchanged. This is the **only** sanctioned exception to
> the "every file in `src/ported/` mirrors a `Src/*.c`" rule.

`src/lib.rs` re-exports `pub use ported::*;` so call sites (`crate::exec::…`,
`crate::subst::…`, etc.) continue to resolve without churn — but **all
ports, new or existing, must land under `src/ported/` and obey every
rule in this document**. Existing files are not grandfathered: if an
audit finds a file under `src/ported/` that lacks a matching C file, or
a function under `src/ported/` that lacks a `/// Port of …` citation or
uses an out-of-allowlist name, it is treated as adhoc and deleted on
sight.

**Inside `src/ported/`, you may not create any Rust file at all** —
not even one that mirrors a real `Src/<x>.c`. The 106 files currently
checked in are the closed legal set (see ABSOLUTE FREEZE above). New
non-port files belong only in `src/extensions/` (features zsh lacks)
or `src/recorder/` (the recorder feature). See Rule 1.

- ❌ No `src/ported/helpers.rs`, `src/ported/common.rs`,
  `src/ported/types.rs`, `src/ported/error.rs`, `src/ported/prelude.rs`,
  `src/ported/macros.rs`, `src/ported/ffi.rs`, `src/ported/state.rs`,
  `src/ported/runtime.rs`, `src/ported/wrapper.rs`,
  `src/ported/safe_*.rs`, `src/ported/rusty_*.rs`, etc. (None of these
  names correspond to any `Src/*.c` in upstream zsh — they are
  invented helper-bucket names by definition.)
- ✅ `src/ported/utils.rs` and `src/ported/context.rs` are **legal**
  because `Src/utils.c` and `Src/context.c` both exist upstream and are
  their canonical 1:1 homes. They are **not** catch-basins: the only
  functions that may live in them are ports of fns whose C definition
  is in `Src/utils.c` or `Src/context.c` respectively (verify via
  `grep -nE '^<name>\(' src/zsh/Src/utils.c` etc.). Anything else in
  those files is drift and must be moved to its canonical home or
  deleted.
- ❌ No new `mod` directories under `src/ported/` that don't exist as a
  directory under `src/zsh/Src/`.
- ❌ No "support crate," no `zshrs-core`, no `zshrs-utils`,
  no workspace splits that don't mirror zsh's `Src/` subdirectories.
- ❌ **No new Rust files under `src/ported/`, period.** See the
  ABSOLUTE FREEZE section above. The previous "find a C file with no
  Rust counterpart yet, then create the mirror file" workflow is
  RESCINDED. The 106 existing files in `src/ported/` are the complete,
  closed legal set. If a port needs a Rust file that doesn't exist in
  that set, raise it with the maintainer; do not create it yourself.
- ✅ The only legal way to add a new file under `src/extensions/` is:
  the file implements a feature that zsh C demonstrably does **not**
  have, and does not duplicate or shadow any existing port.

Allowed Rust source dirs (because they mirror zsh's layout):

| upstream zsh dir       | Rust dir                    |
|------------------------|-----------------------------|
| `Src/`                 | `src/ported/`               |
| `Src/Zle/`             | `src/ported/zle/`           |
| `Src/Modules/`         | `src/ported/modules/`       |
| `Src/Builtins/` (n/a)  | `src/ported/builtins/`      |

If zsh doesn't have a directory, neither do you. If zsh doesn't have a
file, neither do you. If your port needs "a place to put this helper,"
**the helper does not exist** — see Rule 1.

Every C file maps to exactly one Rust file. There is no "splitting for
clarity," no "grouping related helpers," no `mod utils`.

| upstream C path                  | Rust path                              |
|----------------------------------|----------------------------------------|
| `Src/builtin.c`                  | `src/ported/builtin.rs`                |
| `Src/exec.c`                     | `src/ported/exec.rs`                   |
| `Src/subst.c`                    | `src/ported/subst.rs`                  |
| `Src/Zle/zle_main.c`             | `src/ported/zle/zle_main.rs`           |
| `Src/Zle/compcore.c`             | `src/ported/zle/compcore.rs`           |
| `Src/Modules/cap.c`              | `src/ported/modules/cap.rs`            |
| `Src/Modules/files.c`            | `src/ported/modules/files.rs`          |

No renames of any kind. No `_port`, no `_rs`, no `_impl`, no `_v2`,
no stripping of any prefix or suffix. The Rust file stem is **byte-for-byte
identical** to the C file stem.

If your port of `bin_foo()` (defined in `Src/builtin.c`) ends up in
`src/ported/anything_other_than_builtin.rs`, you have done it wrong. Move it.

If it ends up anywhere outside `src/ported/` (e.g. `src/foo.rs` at the
crate root, or under `src/extensions/`), it will be deleted on sight.

---

## Adhoc Code: 100% Banned, Deleted on Sight

Adhoc implementation is **forbidden absolutely**. Not "discouraged."
Not "should be ported eventually." **Banned.**

The maintainer runs purges that delete any function or file which:

- Has **no** `/// Port of ... from Src/...` doc-comment, **or**
- Carries the `/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A
  FAITHFUL PORT` marker (added by `scripts/match_or_warn_modules.py`
  when no matching C function can be located), **or**
- Has a name that is not in `docs/zsh_c_functions.txt` and is not one
  of the allowed exemptions in Rule 3, **or**
- Lives in a Rust file under `src/ported/` that has no corresponding C
  file under `src/zsh/Src/`, **or**
- Lives in the wrong file per the 1:1 mapping (e.g. a port of
  `bin_print` outside `src/ported/builtin.rs`), **or**
- Lives outside `src/ported/`, `src/extensions/`, or `src/recorder/`.

If your PR adds adhoc code (non-port code outside `src/extensions/` or
`src/recorder/`), **all of it will be deleted** — the function, the
file, the module declaration. Without discussion. Do not argue. Do not
"polish" the adhoc code. Either port the corresponding C function
properly into the matching Rust file under `src/ported/`, place a
genuinely new feature under `src/extensions/`, or do not write the
code at all.

---

## Workflow for Bots

Before writing any code:

1. Identify the C function you intend to port. Get its exact name,
   file, and line. Confirm it appears in `docs/zsh_c_functions.txt`.
2. Identify the destination Rust file using the 1:1 mapping table.
   The destination MUST be one of the 106 existing files under
   `src/ported/`. If it isn't — STOP. Do not create a new file. Do not
   reroute the port to a "close enough" file. Raise it with the
   maintainer. (See the ABSOLUTE FREEZE section.)
3. Read the C function in full. Read every helper it calls. Read the
   relevant `struct` definitions in headers.
4. Translate line-by-line. Preserve identifier names where legal in
   Rust. Where C names collide with Rust keywords, use `r#name`
   (e.g. `r#loop`, `r#type`).
5. Add the `/// Port of ... from Src/...:NNNN` doc-comment.
6. Add inline `// C: <line>` tags on non-obvious translations so the
   next bot can verify.
7. Run `cargo build --lib` and `cargo test --lib`. Do not regress the
   baseline.
8. Run `scripts/gen_port_report.py` to refresh `docs/port_report.html`.

---

## What You Must Never Do

- ❌ **Create ANY new Rust file under `src/ported/`.** Period.
  `src/ported/` is frozen at its current 106-file set. Even when a real
  `Src/<x>.c` exists with no Rust counterpart, you do not get to add
  the file. (See the ABSOLUTE FREEZE section.)
- ❌ **Create any Rust file outside `src/ported/`** other than the two
  sanctioned exceptions (`src/extensions/`, `src/recorder/`).
- ❌ **Create any directory under `src/ported/`.** The directory tree
  is frozen alongside the file tree.
- ❌ **Add any `fn` under `src/ported/` whose name does not already
  exist as a function in `src/zsh/Src/**/*.c`.** Verify with `grep` or
  `docs/zsh_c_functions.txt` before writing the signature. Trait-impl
  and `#[test]` exemptions from Rule 3 are the only carve-outs.
- ❌ **Add any `struct`, `enum`, `type`, `union`, or top-level
  `static` under `src/ported/` whose name does not already exist as
  a `struct` / `enum` / `typedef` / `static` in upstream zsh C
  source.** Verify with `grep -nE '(struct|union|enum)[[:space:]]+<name>'
  src/zsh/Src/**/*.{c,h}` before writing the type. The bag-of-globals
  `*State` / `*Table` / `*Builder` / `*Config` / `*Context` pattern
  is **deleted on sight**.
- ❌ **Place a struct in the wrong file.** Header-defined types
  (anything declared in `Src/zsh.h`, `Src/Zle/zle.h`, etc.) port to
  the header file (`zsh_h.rs`, `zle/zle_h.rs`, etc.), NOT to the
  module that consumes them. `struct funcstack` is in `Src/zsh.h:1348`
  → `src/ported/zsh_h.rs`, not `src/ported/modules/parameter.rs`.
- ❌ **Change a function signature.** Rule S1 — Rust signature is
  identical to C. C `paramtypestr(Param pm)` → Rust
  `paramtypestr(pm: &param)`, NOT `paramtypestr(ptype: ParamType,
  flags: &ParamFlags)`. No threading state as extra params. No
  splitting one fn into many. No merging many into one. No reordering.
  No renaming. Same name, same arity, same order.
- ❌ **Rename a local variable inside a function body.** Rule 5 / E
  — C `int hard, limnum, lim;` → Rust `let hard; let limnum; let
  lim;`, NOT `idx`/`count`/`result`. C `HashNode hn` iterating
  `for (hn = ...; hn; hn = hn->next)` → Rust `let mut hn: Option<HashNode>;`,
  NOT `hn_opt`/`hn_node`/`current`. C `val = metafy(val, ...)` → Rust
  `val = metafy(&val);`, NOT `let metafied = metafy(&val);`. The C
  name is the canonical name everywhere — params, locals, loop
  iterators, temporaries.
- ❌ Invent a function with a name not in `docs/zsh_c_functions.txt`.
- ❌ Write "helper" / "utility" / "convenience" functions or files.
- ❌ Add new modules like `helpers`, `common`, `prelude`, `error`,
  `state`, `runtime`, `ffi`, `macros`, `types`, `safe_*`, `rusty_*` —
  none correspond to any `Src/*.c`.
- ⚠️ The modules `utils` and `context` are legal **only** as 1:1
  mirrors of `Src/utils.c` and `Src/context.c`. Treating them as
  catch-basins for orphan helpers is a violation: if a fn's C
  definition lives in a different `Src/<other>.c`, it ports to
  `src/ported/<other>.rs`, never to `utils.rs`/`context.rs`.
- ❌ Refactor C control flow into Rust iterators / combinators / traits
  unless the C code already does the equivalent.
- ❌ Add abstraction layers (traits, generics, builders) that aren't in
  the C source.
- ❌ Split one C function across multiple Rust files.
- ❌ Combine multiple C functions into one Rust function.
- ❌ Add `_port`, `_rs`, `_impl`, `_v2`, `_new`, `_safe`, `_ext` suffixes.
- ❌ Skip the `/// Port of ...` doc-comment.
- ❌ Cite a C function that doesn't exist or doesn't actually correspond.
- ❌ "Stub" a function with `unimplemented!()` and call it ported.
- ❌ Translate from your memory of zsh's behavior. Read the C source.

---

## What You Should Do

- ✅ Pick one C function, port it faithfully, cite it precisely.
- ✅ Mirror C identifier names, struct field names, file layout.
- ✅ Mirror C control flow (`goto` → labelled `loop`/`break`).
- ✅ Mirror globals as `static mut` / `Mutex<...>` / thread-locals as
  needed for parity, not Rust elegance.
- ✅ Cross-reference `docs/zsh_c_functions_with_locations.txt` to verify
  every name and location.
- ✅ Keep the build green. Keep the test baseline.

---

## Sources of Truth

- **C source**: `src/zsh/Src/**/*.c` (and headers `*.h`, `*.epro`,
  `*.pro`).
- **Function allowlist**: `docs/zsh_c_functions.txt` (regenerate with
  ctags — see `docs/zsh_c_functions_with_locations.txt` for locations).
- **Port progress — DEFINITIVE C↔Rust mapping guide**:
  [`docs/port_report.html`](port_report.html)
  (generated by `scripts/gen_port_report.py`). This is the single
  source of truth for which C function lives in which Rust file,
  what's ported vs unported, and where placement is misplaced or
  split. Always consult it before adding or moving a port.
- **Adhoc detector**: `scripts/match_or_warn_modules.py`.
- **Stub detector**: `scripts/gen_port_stubs.py` — compares each Rust
  fn's body line count against its same-named C counterpart in
  upstream zsh. Flags any fn whose Rust body is < 30% of C body
  (with C body ≥ 10 lines) as a likely stub. Output:
  [`docs/audits/PORT_STUBS.md`](docs/audits/PORT_STUBS.md). Use to
  pick up incomplete ports; the report is regenerated by running
  the script manually (not gated by CI).

---

## TL;DR

> **Rule 0: ASK FIRST.** Adding any `fn`/`struct`/`enum`/`static`
> under `src/ported/` whose name does NOT exist in upstream zsh C
> requires EXPLICIT MAINTAINER PERMISSION before you write the
> code. No "tiny helpers," no "obvious wrappers," no "thread_local
> accessors." Stop, ask, wait for an explicit yes. See Rule 0 at
> the top of this file for the required flow + past-damage list.
>
> **`src/ported/` is FROZEN. The 106 existing files are the legal set —
> no new files, no new directories, ever.**
>
> Inside the frozen 106, **every name must exist in upstream zsh C**:
> - **Functions**: name appears in `src/zsh/Src/**/*.c` (verify with
>   `grep` or `docs/zsh_c_functions.txt`).
> - **Structs / enums / typedefs / unions / statics**: name appears
>   as `struct <name>` / `enum <name>` / `typedef ... <name>` /
>   `static ... <name>` in `src/zsh/Src/**/*.{c,h}`. Bag-of-globals
>   `*Table` / `*State` / `*Builder` / `*Config` aggregates are
>   deleted on sight.
> - **Local variables inside fn bodies**: same names as C, in the
>   same order, at the same scope. `int hard, limnum, lim;` → `let
>   hard; let limnum; let lim;`. NEVER rename for "Rust idiom" — not
>   for params, not for locals, not for loop iterators, not for
>   temporaries. `val` stays `val`, `hn` stays `hn`, `i` stays `i`.
> - **Signatures**: identical to C (Rule S1). Same name, same arity,
>   same param order, no threading state as extra params.
> - **File placement**: every fn / struct lives in the Rust file that
>   mirrors its C definition file. Header-defined types
>   (`Src/zsh.h`, `Src/Zle/zle.h`, etc.) port to the corresponding
>   `*_h.rs`, NOT to the module that consumes them.
> - **Code order**: top-to-bottom order of decls/structs/fns/comments
>   matches C (Rule S2).
> - **Citations**: every fn carries `/// Port of <c_fn>() from
>   Src/<file>.c:<NNNN>`; every non-trivial statement carries an
>   inline `// c:NNN` comment.
>
> Every file is a strict 1:1 port of its `Src/*.c`. No grandfathering,
> no helpers, no "legacy" exemptions — old files and new edits alike
> must comply. Genuinely new features go under `src/extensions/`. The
> recorder goes under `src/recorder/`. The only non-port files
> sanctioned outside those two dirs are `src/exec.rs` and
> `src/fusevm_bridge.rs`. Adhoc code anywhere else is deleted on
> sight.
