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

## Rust chain — same shape, three architectural divergences

Every link is ported. The Rust port splits one C function, replaces
another's lookup mechanism, and — as a consequence of the second —
reports a shorter `$zsh_eval_context` inside a live completion. The
first two are bucket-2 architectural divergence (functionally
equivalent end-state, see `PORT_CALL_COVERAGE.md`); the third is an
observable divergence that is deliberately left open.

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

### Divergence C — `$ZSH_EVAL_CONTEXT` is shorter inside a completion

Observable, deliberate, and not going to be closed.

C pushes a `zsh_eval_context` frame from every `execode(prog, …,
"label")` call site (`Src/exec.c:1245-1282`). Two of those labels are
produced by machinery the Rust chain does not run: `loadautofunc`
(`Src/exec.c:5626`, "this body is running on the call that read its
file out of `$fpath`") and the `eval` from `_complete`'s
`eval "$comp"` / `_dispatch`'s equivalent. Divergence B replaces the
`getshfunc` → `runshfunc` lookup with `router::try_rust_dispatch`, so
a stock `_main_complete` / `_complete` / `_dispatch` never autoloads
and never evals — it is already resident Rust.

Measured on `ectxprobe <TAB>` against a stock fpath (fixture completer
printing `${(j.+.)zsh_eval_context}` and `${(j.+.)funcstack}`):

| | value |
|---|---|
| zsh `funcstack` | `_ectxprobe (eval) _dispatch _normal _complete _megacomplete _megacomplete _main_complete` |
| zsh context (14) | `shfunc loadautofunc` \| `shfunc loadautofunc` \| `shfunc` \| `shfunc loadautofunc` \| `shfunc loadautofunc` \| `shfunc loadautofunc` \| `eval` \| `shfunc loadautofunc` |
| zshrs `funcstack` | `_ectxprobe _dispatch _complete _megacomplete _megacomplete _main_complete` |
| zshrs context (8) | `shfunc` \| `shfunc loadautofunc` \| `shfunc` \| `shfunc` \| `shfunc` \| `shfunc loadautofunc` |

The six-frame deficit is: `loadautofunc` for `_main_complete`,
`_complete` and `_dispatch`; `shfunc`+`loadautofunc` for `_normal`;
and the `eval`. Note `_megacomplete` and the user's fixture completer
DO carry a real `loadautofunc` — they are shell functions with no Rust
port, so they take the ordinary autoload path.

Why the frames are not synthesized:

1. **Two of the six describe a call that does not happen at all.**
   `_complete`'s port calls `_normal` as a direct Rust call
   (`src/compsys/ported/Base/Completer/_complete.rs:129`), so `_normal`
   is absent from `$funcstack` too. Faking its context frames would put
   the two stacks into disagreement; faking the funcstack entry as well
   would drag `$0`, `$LINENO`, `funcsourcetrace` and `_call_function`'s
   diagnostics into the fiction.
2. **Nothing reads them.** Every `zsh_eval_context` consumer surveyed
   on a fully loaded install — the `[-1] == loadautofunc` idiom in ~30
   generated completion files, the looser `== *func` form, upstream
   `add-zle-hook-widget`'s three-way `case`, `bracketed-paste-magic`,
   `chpwd_recent_dirs`' whole-string `toplevel(:[a-z]#func|)#`, and
   zinit's `[1] = file` — reads the LAST frame, or the first, or the
   whole string. None reads a middle frame or a count. The last-frame
   semantics are already correct: a completion file autoloaded through
   the Rust chain sees `loadautofunc`, verified live and pinned by
   `consumer_idioms_match_zsh`.
3. **`loadautofunc` has no truthful trigger in a port.** It means
   "undefined until this call". A port is never undefined, so
   synthesizing it needs a per-name pretend-loaded flag that would also
   have to model `kshautoload` (`evalautofunc`), `autoload +X` (no
   frame) and re-`autoload`ing — a state machine whose only output is a
   string.
4. **It would mislead.** `loadautofunc` and `eval` assert that a file
   was read and a string was parsed. Neither happens, and anyone
   tracing a completion in zshrs would go looking for both.

Pinned by `tests/zsh_eval_context_frames.rs::compsys_ports_synthesize_no_eval_context_frames`.

Outside a completion the stacks are byte-identical; see
`eval_context_frames_match_zsh` in the same file.

## Test coverage of the chain

- `src/ported/zle/zle_tricky.rs` — completecall, completeword,
  expandorcomplete unit tests around line 4500 (in
  `compcore.rs::tests`): `callcompfunc_empty_fn_no_panic`,
  `callcompfunc_sets_compstate_context`.
- `tests/parity/bindkey_parity.rs` — full bindkey round-trip (verifies
  the Comp widget registration path that compinit uses).
- ZTST `test_corpus/Y0*.ztst` files — end-to-end completion
  scenarios that exercise the full chain via the test harness.
- `tests/zsh_eval_context_frames.rs` — `consumer_idioms_match_zsh`
  pins the four `$zsh_eval_context` tests real completion files
  perform; `compsys_ports_synthesize_no_eval_context_frames` pins
  Divergence C against being closed by fabrication.

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
