# Completion Dispatch Chain — C ↔ Rust Parity

How a Tab keypress reaches `_main_complete` (or doesn't) in zsh, and
how the Rust port mirrors each link. C line numbers are against the
zsh fork at `8cc5eade5f` (`Src/Zle/*.c`, `Src/exec.c`, `Src/utils.c`,
`Src/module.c`, `Src/builtin.c`); Rust references are symbol paths so
they survive edits. Re-verified 2026-10-27 at zshrs v0.13.56.

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
    → docomplete(COMP_EXPAND_COMPLETE)            zle_tricky.c:599
      → docompletion(s, lst, lincmd)              zle_tricky.c:851/868
        → runhookdef(COMPLETEHOOK, &dat)          zle_tricky.c:2350
          → do_completion(...)                    compcore.c:287
                                                    (registered handler
                                                    via addhookfunc)
            → callcompfunc(s, compfunc)           compcore.c:543
                                                    compfunc is NULL —
                                                    no completecall ever
                                                    ran to set it
              → shfunc = getshfunc(NULL)          compcore.c:551
                                                    returns NULL
              → if-branch SKIPPED — doshfunc not
                reached
            → returns 0, do_completion falls
              through to legacy compctl path      (compctl.c)
```

## C chain — post-compinit (`complete-word`)

```
compinit loop                                     Completion/compinit:542
  → zle -C $_i_line .$_i_line _main_complete
    → registers Comp widget per name:
      { fn: <base C fn>, wid: $_i_line,
        func: "_main_complete" }
    e.g. for complete-word: fn = completeword

Tab pressed (rebound by compinit to complete-word)
  → completecall(args)                            zle_tricky.c:202
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
          → docompletion(s, lst, lincmd)          zle_tricky.c:868
            → runhookdef(COMPLETEHOOK, &dat)      zle_tricky.c:2350
              → do_completion(...)                compcore.c:287
                → callcompfunc(s, compfunc)       compcore.c:543
                                                    compfunc =
                                                    "_main_complete"
                  → shfunc = getshfunc(fn)        compcore.c:551
                                                    finds the autoloaded
                                                    _main_complete entry
                                                    in shfunctab
                  → ... param setup
                    ($compstate, comprpms,
                     compkpms) ...
                  → cfret = doshfunc(shfunc,
                                     largs, 1)    compcore.c:834
                                                    actually runs the
                                                    _main_complete body
                                                    as a shell function
```

The literal string `"_main_complete"` never appears in any `.c`
file. It enters the shell-function table (`shfunctab`) when
`Completion/Base/Core/_main_complete` autoloads, and the C side
reaches it only through the `compfunc` indirection that
`completecall` plants from `compwidget->u.comp.func`.

`getshfunc` only *looks up* a Shfunc by name. `doshfunc` is what
actually *executes* the body — both calls are required.

## Rust chain — same shape, two divergences

Every link is ported with the same function boundaries as C,
including the `docomplete` → `docompletion` → `runhookdef` split.
`callcompfunc` resolves the body differently (B, functionally
equivalent end-state; see `PORT_CALL_COVERAGE.md`), and as a
consequence a live completion reports a shorter `$zsh_eval_context`
(C, observable).

| C function | Rust symbol | Status |
|---|---|---|
| `expandorcomplete` (zle_tricky.c:299) | `ported::zle::zle_tricky::expandorcomplete` | identical |
| `completecall` (zle_tricky.c:202) | `ported::zle::zle_tricky::completecall` | identical |
| `completeword` (zle_tricky.c:216) | `ported::zle::zle_tricky::completeword` | identical |
| `docomplete` (zle_tricky.c:599) | `ported::zle::zle_tricky::docomplete` | identical |
| `docompletion` (zle_tricky.c:2342) | `ported::zle::zle_tricky::docompletion` | identical |
| `do_completion` (compcore.c:287) | `ported::zle::compcore::do_completion` | identical |
| `callcompfunc` (compcore.c:543) | `ported::zle::compcore::callcompfunc` | divergent body resolution — see (B) |
| `getshfunc` (utils.c:4000) | `ported::utils::getshfunc` | identical |
| `doshfunc` (exec.c:5892) | `ported::exec::doshfunc` | identical |
| `runhookdef` (module.c:989) | `ported::module::runhookdef` | identical |

### Divergence B — `callcompfunc` resolves the body through the router

C `callcompfunc(char *s, char *fn)` (compcore.c:543) does:

1. `shfunc = getshfunc(fn)` (c:551).
2. `if (shfunc) { ... param setup ... cfret = doshfunc(shfunc, largs, 1); ... }` (c:834).
3. Empty/NULL `fn` returns from getshfunc as NULL → branch skipped.

Rust `ported::zle::compcore::callcompfunc(s, fn_name)` does:

1. Early return if `fn_name.is_empty()` — the same gate as C's
   `getshfunc(NULL)` → NULL → skip-branch behavior.
2. Calls `getshfunc(fn_name)` only for the node's flags (minus
   `PM_LOADDIR`), and builds a `synth_shf` carrying the name and those
   flags. Keeping `PM_UNDEFINED` matters: it stops `doshfunc` from
   setting `scriptname` before the autoload, so load errors are
   reported under the caller's name as in C.
3. Builds a `body_runner` closure that calls
   `ported::exec::run_function_body(fn_name, args)` — body only, no
   second `doshfunc` scope.
4. Calls `doshfunc(&mut synth_shf, largs, true, body_runner)`, which
   wraps the body in the C prologue/epilogue (param scope, lastval,
   sfcontext, `$funcstack` frame, return-flag handling).

`run_function_body` runs `ShellExecutor::run_function_body_only` on the
live executor, which tries, in order:

1. `compsys::router::dispatch_compsys(name, args)` — a plugin-registered
   `CompFn` (`zmodload -R`, `extensions::plugin_host::dispatch_compfn`)
   first, then `router::try_rust_dispatch`, which returns the Rust port
   from `rust_compsys_lookup` unless `[compsys] backend = "shell"`, the
   name is already defined as a shell function, or `$fpath` resolves it
   to a non-stock file (`has_fpath_override`). The port runs through
   `router::run_port`.
2. `compsys::ported::_regex_arguments::dispatch_if_registered(name)`.
3. The ordinary autoload + fusevm body dispatch.

End-state matches C: `_main_complete` executes inside one `doshfunc`
scope with the same params populated. The divergence is in how the
body is resolved — C fetches the Eprog with `getshfunc` and runs it via
`runshfunc`; Rust either runs a native port or the bytecode body.

Ports call sibling completers by name through
`compsys::ported::shared::call_compfn`, so the router (and its
`$fpath` arbitration) runs for every nested call, and each nested
completer gets its own `doshfunc` frame. `$_comps` values are run
through `shared::eval_comp`, which ports `eval` (builtin.c:6122) and
pushes the `(eval)` funcstack frame via
`ported::exec::EvalFuncstackFrame`.

### Divergence C — `$zsh_eval_context` is shorter inside a completion

C pushes a `zsh_eval_context` entry from every `execode(prog, …,
"label")` call (`execode`, exec.c:1319). Two labels come from machinery
the Rust chain does not run, or does not label:

- `loadautofunc` (exec.c:5692) — "this body is running on the call
  that read its file out of `$fpath`". A stock `_main_complete` /
  `_complete` / `_normal` / `_dispatch` is a resident Rust port and is
  never autoloaded.
- `eval` (builtin.c:6180) — `_dispatch` / `_normal` evaluating the
  `$_comps` entry. `shared::eval_comp` does parse and run that string,
  and pushes the `(eval)` funcstack frame, but deliberately pushes no
  `eval` context entry.

#### Measurement

Fixture completer on a stock `$fpath`, `FPATH` unset in the
environment, default `completer` style:

```zsh
#compdef ectxprobe
print -r -- "funcstack=${(j: :)funcstack}" >> $ECTX_OUT
print -r -- "ZSH_EVAL_CONTEXT=$ZSH_EVAL_CONTEXT" >> $ECTX_OUT
print -r -- "context(${#zsh_eval_context})=${(j:|:)zsh_eval_context}" >> $ECTX_OUT
compadd foo
```

Driven through a pty with `expect`: spawn `<shell> -f -i`, send
`fpath=(<fixture-dir> $fpath); autoload -Uz compinit; compinit -D -u`,
then `ectxprobe <Tab>`. Run on 2026-10-27, zsh 5.9.2 vs
zshrs 0.13.56 (`cargo build` debug):

| | value |
|---|---|
| zsh `funcstack` | `_ectxprobe (eval) _dispatch _normal _complete _main_complete` |
| zsh context (11) | `shfunc:loadautofunc:shfunc:loadautofunc:shfunc:loadautofunc:shfunc:loadautofunc:eval:shfunc:loadautofunc` |
| zshrs `funcstack` | `_ectxprobe (eval) _dispatch _normal _complete _main_complete` |
| zshrs context (6) | `shfunc:shfunc:shfunc:shfunc:shfunc:loadautofunc` |

`$funcstack` is identical. The context is five entries short:
`loadautofunc` for `_main_complete`, `_complete`, `_normal` and
`_dispatch`, and the `eval`. The fixture completer has no Rust port,
so it takes the ordinary autoload path and carries a real
`loadautofunc`.

#### Why the entries are not synthesized

1. **Nothing reads them.** Every `zsh_eval_context` consumer surveyed
   on a fully loaded install — the `[-1] == loadautofunc` idiom in ~30
   generated completion files, the looser `== *func` form, upstream
   `add-zle-hook-widget`'s three-way `case`, `bracketed-paste-magic`,
   `chpwd_recent_dirs`' whole-string `toplevel(:[a-z]#func|)#`, and
   zinit's `[1] = file` — reads the LAST entry, or the first, or the
   whole string. None reads a middle entry or a count. The last-entry
   semantics are correct: a completion file autoloaded through the Rust
   chain sees `loadautofunc`, pinned by `consumer_idioms_match_zsh`.
2. **`loadautofunc` has no truthful trigger in a port.** It means
   "undefined until this call". A port is never undefined, so
   synthesizing it needs a per-name pretend-loaded flag that would also
   have to model `kshautoload` (`evalautofunc`), `autoload +X` (no
   entry) and re-`autoload`ing — a state machine whose only output is a
   string — and would send anyone tracing a completion looking for a
   file read that never happened.

The `eval` entry does not rest on those reasons: `eval_comp` performs a
real parse-and-execute, and `$funcstack` already shows its `(eval)`
frame, so the two stacks now disagree on it. The comment in
`shared::eval_comp` records that pushing it was tried (9e55378587),
broke the pinning test, and was left out pending a maintainer decision.
That decision is open.

Pinned by `tests/zsh_eval_context_frames.rs::compsys_ports_synthesize_no_eval_context_frames`.

Outside a completion the stacks are byte-identical; see
`eval_context_frames_match_zsh` in the same file.

## Test coverage of the chain

- `src/ported/zle/compcore.rs` (`tests` module):
  `callcompfunc_empty_fn_no_panic`, `callcompfunc_sets_compstate_context`.
- `tests/parity/bindkey_parity.rs` — full bindkey round-trip (verifies
  the Comp widget registration path that compinit uses).
- ZTST `test_corpus/Y0*.ztst` files — end-to-end completion
  scenarios that exercise the full chain via the test harness.
- `tests/zsh_eval_context_frames.rs` — `consumer_idioms_match_zsh`
  pins the four `$zsh_eval_context` tests real completion files
  perform; `compsys_ports_synthesize_no_eval_context_frames` pins
  Divergence C.

## Why this matters for the port

1. **Adding a built-in completion** (a Rust port of `_git`,
   `_kubectl`, …) means adding its arm to `rust_compsys_lookup` in
   `src/compsys/router.rs`, pointing at the body (`_NAME_impl` when the
   port has a `call_compfn` wrapper). It fires when `compfunc` or a
   nested by-name call reaches that name, unless the user's `$fpath`
   or a defined shell function overrides it.
2. **Shell-side `_main_complete` keeps working unchanged.** With
   `[compsys] backend = "shell"`, or a user copy earlier in `$fpath`,
   the upstream file is autoloaded and run by the fusevm body path —
   same `doshfunc` scope, same param population.
