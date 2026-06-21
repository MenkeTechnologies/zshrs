# Completion Dispatch Chain — C ↔ Rust Parity

How a Tab keypress reaches `_main_complete` (or doesn't) in zsh, and
how the Rust port mirrors each link. Verified against
`/Users/wizard/forkedRepos/zsh/Src/Zle/*.c` and
`/Users/wizard/RustroverProjects/zshrs/src/ported/zle/*.rs` on
2026-05-29.

The dispatch has two entry points depending on whether the user
ran `compinit` (the autoload that sets up the compsys widgets):

- **Pre-compinit** (default Tab): `expand-or-complete` →
  `expandorcomplete` → `docomplete` → … → `getshfunc(NULL)` returns
  NULL → no shell fn fires → fall through to legacy `compctl` path.
- **Post-compinit** (loaded `_main_complete`): `complete-word` is a
  `zle -C`-registered Comp widget → `completecall` → sets
  `compfunc = "_main_complete"` → invokes the base C fn
  (`completeword`) → `docomplete` → … → `getshfunc("_main_complete")`
  returns the autoloaded Shfunc → `doshfunc` runs the body.

## C chain — pre-compinit (`expand-or-complete`)

```
Tab pressed (default binding: expand-or-complete)
  → expandorcomplete(args)                        zle_tricky.c:299
    → docomplete(COMP_EXPAND_COMPLETE)            zle_tricky.c:314
      → runhookdef(COMPLETEHOOK, &dat)            zle_tricky.c:2347
        → do_completion(...)                      compcore.c:287
                                                    (registered handler
                                                    via addhookfunc)
          → callcompfunc(s, compfunc)             compcore.c:991
                                                    compfunc is NULL —
                                                    no completecall ever
                                                    ran to set it
            → shfunc = getshfunc(NULL)            compcore.c:551
                                                    returns NULL
            → if-branch SKIPPED — doshfunc not
              reached
          → returns 0, do_completion falls
            through to legacy compctl path        (compctl.c)
```

## C chain — post-compinit (`complete-word`)

```
compinit loop                                     Completion/compinit:558
  → zle -C $_i_line .$_i_line _main_complete
    → registers Comp widget per name:
      { fn: <base C fn>, wid: $_i_line,
        func: "_main_complete" }
    e.g. for complete-word: fn = completeword

Tab pressed (rebound by compinit to complete-word)
  → completecall(args)                            zle_tricky.c:201-207
    → compfunc = compwidget->u.comp.func          zle_tricky.c:206
                                                    sets the global
                                                    compfunc to
                                                    "_main_complete"
    → compwidget->u.comp.fn(zlenoargs)            zle_tricky.c:207
                                                    fn ptr resolves to
                                                    completeword for
                                                    complete-word widget
      → completeword(args)                        zle_tricky.c:216
        → docomplete(COMP_COMPLETE)               zle_tricky.c:231
          → runhookdef(COMPLETEHOOK, &dat)        zle_tricky.c:2347
            → do_completion(...)                  compcore.c:287
              → callcompfunc(s, compfunc)         compcore.c:991
                                                    compfunc =
                                                    "_main_complete"
                → shfunc = getshfunc(fn)          compcore.c:551
                                                    finds the autoloaded
                                                    _main_complete entry
                                                    in shfunctab
                → ... param setup
                  ($compstate, comprpms,
                   compkpms) ...
                → cfret = doshfunc(shfunc,
                                   largs, 1)      compcore.c:835
                                                    actually runs the
                                                    _main_complete body
                                                    as a shell function
```

The literal string `"_main_complete"` never appears in any `.c`
file. It enters the shell-function table (`shfunctab`) when
`Completion/Base/Widget/_main_complete` autoloads, and the C side
reaches it only through the `compfunc` indirection that
`completecall` plants from `compwidget->u.comp.func`.

`getshfunc` only *looks up* a Shfunc by name. `doshfunc` is what
actually *executes* the body — both calls are required.

## Rust chain — same shape, two architectural divergences

Every link is ported. The Rust port splits one C function and
replaces another's lookup mechanism; both are bucket-2
architectural divergence (functionally equivalent end-state, see
`PORT_CALL_COVERAGE.md`).

| C function | Rust file:line | Status |
|---|---|---|
| `expandorcomplete` (zle_tricky.c:299) | `src/ported/zle/zle_tricky.rs:268` | identical |
| `completecall` (zle_tricky.c:201) | `src/ported/zle/zle_tricky.rs:98` | identical |
| `completeword` (zle_tricky.c:216) | `src/ported/zle/zle_tricky.rs:144` | identical |
| `docomplete` (zle_tricky.c:599) | `src/ported/zle/zle_tricky.rs:705` | split — see (A) |
| (in docomplete) `runhookdef(COMPLETEHOOK,…)` (zle_tricky.c:2347) | `src/ported/zle/zle_tricky.rs:1376` (`docompletion`) | identical |
| `do_completion` (compcore.c:287) | `src/ported/zle/compcore.rs:74` | identical |
| `callcompfunc` (compcore.c:544) | `src/ported/zle/compcore.rs:587` | divergent body — see (B) |
| `getshfunc` (utils.c) | `src/ported/utils.rs:5063` | identical |
| `doshfunc` (exec.c) | `src/ported/exec.rs:5610` | identical |
| `runhookdef` (module.c) | `src/ported/module.rs:839` | identical |

### Divergence A — `docomplete` split

C `docomplete(int lst)` (zle_tricky.c:599-880) does:

1. Recursion guard.
2. `runhookdef(BEFORECOMPLETEHOOK, &lst)` (c:621).
3. `doexpandhist` early-return (c:628).
4. `get_comp_string` extraction (c:664-810).
5. Dispatch on `lst` (SPELL / EXPAND_COMPLETE / plain) and call
   the shared `runhookdef(COMPLETEHOOK, &dat)` → `do_completion`
   path (c:2347).
6. `runhookdef(AFTERCOMPLETEHOOK, &dat)` (c:878).

Rust splits this into two fns:

- `docomplete(lst)` (zle_tricky.rs:705) — handles steps 1-4 + 6 +
  the dispatch on `lst`. For the COMP_COMPLETE / COMP_LIST_COMPLETE
  cases it delegates to `docompletion`.
- `docompletion(s, lst, incmd)` (zle_tricky.rs:1363) — handles
  step 5 only: build `compldat`, look up `gethookdef("complete")`,
  fire `runhookdef`, fall through to `do_completion` if no hook
  is registered (matching C's `def`-fallback at module.c:993-994).

End-state matches C exactly. Split is for Rust readability; same
hook fires in the same order with the same payload.

### Divergence B — `callcompfunc` skips explicit `getshfunc`

C `callcompfunc(char *s, char *fn)` (compcore.c:544) does:

1. `shfunc = getshfunc(fn)`.
2. `if (shfunc) { ... param setup ... doshfunc(shfunc, largs, 1); ... }`.
3. Empty/NULL `fn` returns from getshfunc as NULL → branch skipped.

Rust `callcompfunc(s: &str, fn_name: &str)` (compcore.rs:587)
does:

1. Early return if `fn_name.is_empty()` — the same gate as C's
   `getshfunc(NULL)` → NULL → skip-branch behavior.
2. Builds a `synth_shf` Shfunc carrying just the name (no
   autoload table lookup).
3. Constructs a `body_runner` closure that dispatches via
   `compsys::router::try_rust_dispatch(fn_name)` first (in-process
   Rust port of compsys helpers), falling back to
   `ported::exec::dispatch_function_call(fn_name, args)` which is
   the standard fusevm bytecode dispatch.
4. Calls `doshfunc(&mut synth_shf, largs, true, body_runner)` —
   doshfunc wraps the body_runner in the C-faithful prologue /
   epilogue scope (param scope push, lastval save, sfcontext
   push, return-flag handling).

End-state matches C: `_main_complete` body executes inside a
proper Shfunc scope. The divergence is purely in *how* the body
gets resolved — C uses `getshfunc` to fetch the precompiled
Eprog and `runshfunc` to execute it; Rust uses a closure that
either routes to a native Rust port or to the bytecode VM via
`ported::exec::dispatch_function_call`. Both produce identical observable behavior because
`doshfunc`'s scope plumbing is the same.

## Test coverage of the chain

- `src/ported/zle/zle_tricky.rs` — completecall, completeword,
  expandorcomplete unit tests around line 4500 (in
  `compcore.rs::tests`): `callcompfunc_empty_fn_no_panic`,
  `callcompfunc_sets_compstate_context`.
- `tests/parity/bindkey_parity.rs` — full bindkey round-trip (verifies
  the Comp widget registration path that compinit uses).
- ZTST `test_corpus/Y0*.ztst` files — end-to-end completion
  scenarios that exercise the full chain via the test harness.

## Why this matters for the port

Two practical consequences:

1. **Adding new built-in completions** (Rust ports of `_git`,
   `_kubectl`, etc.) goes through `compsys::router::try_rust_dispatch`,
   bypassing the autoload table. Reachable from the chain because
   `callcompfunc` already routes through `body_runner` which checks
   `try_rust_dispatch` first. Add the entry in the router, the
   Rust port fires when `compfunc == "_git"`.
2. **Shell-side `_main_complete` keeps working unchanged**.
   Anyone autoloading the upstream compsys file gets it executed
   via the fusevm body_runner branch — same scope semantics, same
   param population, no behavior shift.
