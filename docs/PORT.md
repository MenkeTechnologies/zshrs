# PORT.md — Rules for Bots Contributing to `zshrs`

`zshrs` is a **1:1 Rust port of zsh**. The goal is 100% behavioral parity
with upstream zsh. This is **not** a reimplementation, not a rewrite, not
"inspired by" zsh. Every line of Rust code must trace back to a specific
line of upstream C code in `src/zsh/Src/`.

If you are a bot (Copilot, Claude, GPT, Cursor, Aider, any LLM agent),
**read this file before writing a single line of code**. Violations are
deleted on sight by the maintainer. No exceptions.

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
- The legal file set is the **89 files currently checked in** under
  `src/ported/` (run `find src/ported -name '*.rs' | sort` to see the
  exhaustive list). That set is the universe. New ports either land in
  one of those 89 files or they do not land at all.
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

### Why this freeze exists

Bots have produced 114 distinct duplicate function names across
`src/ported/`, with 148 extra copies and 1,638 self-confessed
`// c:N/A` adhoc functions. Every drift originated as either (a) a new
file the bot created to "have somewhere to put" a helper, or (b) a new
function name the bot invented because no real zsh fn matched. The
freeze closes both vectors at the source. From this point forward, the
only edits permitted under `src/ported/` are: (i) modifying an
existing fn in an existing file to be a more faithful port, (ii)
adding a fn whose name already exists in upstream C code to an
existing Rust file that already maps to that C file's `Src/*.c`
counterpart, (iii) deleting drift.

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

- **`docs/zsh_c_functions.txt`** — 2,488 unique C function names.
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
not even one that mirrors a real `Src/<x>.c`. The 89 files currently
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
  RESCINDED. The 89 existing files in `src/ported/` are the complete,
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
   The destination MUST be one of the 89 existing files under
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
  `src/ported/` is frozen at its current 89-file set. Even when a real
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
  [`docs/port_report.html`](file:///Users/wizard/RustroverProjects/zshrs/docs/port_report.html)
  (generated by `scripts/gen_port_report.py`). This is the single
  source of truth for which C function lives in which Rust file,
  what's ported vs unported, and where placement is misplaced or
  split. Always consult it before adding or moving a port.
- **Adhoc detector**: `scripts/match_or_warn_modules.py`.

---

## TL;DR

> **`src/ported/` is FROZEN. The 89 existing files are the legal set —
> no new files, no new directories, ever. The only `fn` names allowed
> are ones that already exist as functions in `src/zsh/Src/**/*.c`
> (verify with `grep` before writing). Every file is a strict 1:1 port
> of its `Src/*.c`. Every function cites its C source. No
> grandfathering, no helpers, no "legacy" exemptions — old files and
> new edits alike must comply. Genuinely new features go under
> `src/extensions/`. The recorder goes under `src/recorder/`. The only
> non-port files sanctioned outside those two dirs are `src/exec.rs`
> and `src/fusevm_bridge.rs`. Adhoc code anywhere else is deleted on
> sight.**
