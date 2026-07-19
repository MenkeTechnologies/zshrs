# subst.c → subst_port.rs: Faithful Port Plan

> **SUPERSEDED — this plan was executed.** The target file `src/subst_port.rs`
> no longer exists; it was moved to `src/ported/subst.rs` (commit `b8e737fa2a`,
> "reorg"). The core objective below — replace the small-dispatcher
> `paramsubst` with a 1:1 port of C's single large function — is done:
> `paramsubst` is now one function spanning `src/ported/subst.rs:3123`–`18679`
> (~15,500 lines), and the file is ~23,900 lines total. The "currently
> structured as a 206-line dispatcher" starting-state and `src/subst_port.rs`
> paths throughout this document are therefore **historical**; they describe
> the pre-port state, not the current tree. Kept as a record of the plan.

## Why this exists

`src/subst_port.rs` is currently structured as a tiny dispatcher (`paramsubst` = 206 lines) that calls many small helpers (`parse_brace_param`, `apply_param_flags`, `apply_operator_with_flags_full`, `scalar_char_subscript`, `get_param_value`, `get_param_with_subscript`, etc.). That decomposition broke the C source's invariant: in `Src/subst.c`, paramsubst is **2849 lines as one function** (subst.c:1625-4473) with shared local state — `aval`, `isarr`, `val`, `qt`, `spbreak`, `spsep`, `sep`, `nojoin`, `aspar`, `subexp`, `ms_flags`, `quoted_array_with_offset`, `casmod`, `pf_flags`, `quoteerr`, `chkset`, `quoted` — threaded through every flag arm, every operator arm, and the post-processing block.

Splitting that into helpers means each helper holds a slice of state. Cross-helper invariants disappear. Every new shape (`${(@f)"$(cmd)"}`, `${(Pkv@)n}`, `${${${(f)x}[2,-1]}//=/…}`, etc.) falls between helpers and gets patched by adding *another* helper-tweak. The loop doesn't terminate.

**The recurring "another shape, another helper, another gap" cycle is structural. The fix is structural: a 1:1 port of subst.c, not more patches.**

## Sizes (verified)

```
$ wc -l Src/subst.c src/subst_port.rs
  4922 Src/subst.c
 10619 src/subst_port.rs
```

```
$ awk '/^paramsubst\(/{...}' Src/subst.c            → 2849 lines (subst.c:1625-4473)
$ awk '/^fn paramsubst\(/{...}' src/subst_port.rs   →  206 lines
```

zshrs's paramsubst is **7% the size of C's** despite the file being 2x larger overall. The volume went into helpers; the threaded-state machine didn't.

## Scope

### subst.c functions to port

| C function | C lines | Current zshrs status |
|---|---|---|
| `paramsubst` | 2849 (1625-4473) | Helper-decomposed → needs full port |
| `multsub` | ~80 (544-621) | Partial port |
| `prefork` | ~150 (270-440) | Partial port |
| `stringsubst` | ~200 (227-421) | Partial port |
| `singsub` | ~30 (5039-5050) | Partial port |
| `arithsubst` | ~25 | Done |
| `filesub` | ~80 | TBD |
| `stringsubstquote` | ~50 | Done |
| `getproc` / `getoutputfile` | ~150 | Partial port |
| Tilde, glob_subst, untokenize helpers | ~600 | Mixed |

**Total C source:** 4922 lines. Faithful port target: ~5000-6000 Rust lines (some C macros expand inline, some compact C constructs need explicit Rust).

### Helpers to delete after port

These currently exist in `src/subst_port.rs` as adhoc decompositions of paramsubst; after the C port replaces paramsubst, they go away or shrink to thin wrappers:

- `parse_brace_param`
- `apply_param_flags`
- `apply_operator_with_flags_full`
- `apply_operator_with_flags`
- `scalar_char_subscript` (its logic moves inline)
- `get_param_value`
- `get_param_with_subscript`
- `apply_array_subscript_flags`
- `apply_assoc_subscript_flags`
- The flag-parsing fragments scattered across multiple sites

## Approach

### Phase 1 — paramsubst

1. Branch off `main` to `subst-c-port`
2. Create new file `src/subst_paramsubst_port.rs` with one function `paramsubst_port`
3. Port subst.c:1625-4473 line-by-line, preserving C structure:
   - Variable declarations at function top (subst.c:1640-1830) → all locals declared in same scope
   - Flag-chain parsing loop (subst.c:1830-2300) → one big `match`/`if-else` chain over flag chars
   - Subscript parsing (subst.c:2300-2600) → inline, no helper
   - Subexp recursion (subst.c:2649-2730) → calls `multsub_port` recursively
   - Value fetch + chkset (subst.c:2750-3000) → inline
   - aval/val processing + (P) indirect (subst.c:3000-3500) → inline
   - Operator dispatch (subst.c:3500-3900) → one match over operator
   - Post-processing: case mod, escape, padding, quoting, glob_subst (subst.c:3900-4470) → inline
4. Every block headed by `// subst.c:NNNN-MMMM <C identifier or comment>` reference
5. Same local variable names as C (transliterated to Rust idiom but recognizable: `aval` → `aval: Vec<String>`, `isarr` → `isarr: i32`, etc.)
6. Hook to live executor only at C's `getvalue`/`fetchvalue` and `multsub` callsites — minimize executor touches
7. Old `paramsubst` stays callable; new function gated behind `ZSHRS_NEW_PARAMSUBST=1` env var for parity testing
8. New entry points `substitute_brace_port` / `substitute_brace_array_port` call `paramsubst_port`; old wrappers unchanged

### Phase 2 — parity bring-up

1. Run all tests with `ZSHRS_NEW_PARAMSUBST=1`. Diff failures against old.
2. Drive new path to ≥ old coverage (currently: 8 megamonster failures, 9 pre-existing subst_port unit failures, all other suites green).
3. Drive new path to PASS the 8 currently-failing megamonsters (the whole point of the port).
4. Flip the env-var default to ON.
5. Delete old paramsubst + its decomposed helpers in a separate commit.

### Phase 3 — multsub, prefork, stringsubst, singsub

Same approach: 1:1 ports of C functions to dedicated `*_port.rs` files, gated on env var, tested for parity, then default-on, then delete old.

C source totals: ~280 lines for these four combined.

### Phase 4 — supporting helpers

filesub, glob_subst plumbing, tilde expansion, untokenize variants, etc. Port to match C structure.

## Files touched

- **NEW**: `src/subst_paramsubst_port.rs`, `src/subst_multsub_port.rs`, `src/subst_prefork_port.rs`, `src/subst_stringsubst_port.rs`
- **MODIFIED** (delete-only): `src/subst_port.rs` — remove paramsubst, parse_brace_param, apply_param_flags, apply_operator_with_flags_full, scalar_char_subscript, etc. once new path is default
- **UNCHANGED public API**: `substitute_brace`, `substitute_brace_array`, `singsub`, `multsub`, `prefork`, `stringsubst` keep their signatures; bodies switch to call the new ports

## Success criteria

- All current passing tests stay green:
  - 928 unit tests (zshrs --tests)
  - 392 zsh_construct_corpus
  - 184 zinit_p10k_parity
  - 160 real_world_idioms_parity
  - 28 no_tree_walker_dispatch
  - 11 tree_walker_absent
  - 392 builtins_parity
  - All other integration suites
- All 8 currently-failing megamonsters pass:
  - `array_filter_with_dynamic_pattern`
  - `fsh_three_part_backref_replace`
  - `fzf_tab_swap_around_null_delim`
  - `p_flag_indirects_through_first_line_of_cmdsubst`
  - `zbrowse_p_kv_splat_through_indirect`
  - `zbrowse_qqqq_per_element_dollar_quoting`
  - `zinit_at_f_line_split_cmdsubst`
  - `zsh_256_color_demo_with_conditional_newline`
- Also fix the 9 pre-existing `src/subst_port.rs` unit-test failures (`p10k_anchored_*`, `p10k_home_replace_with_tilde`, `p10k_tilde_glob_subst_form`, etc.) which were never addressed because the helpers couldn't carry the right state
- `paramsubst_port` function size within 10% of C's 2849 lines (i.e. 2560-3140 lines)
- Every C source line referenced by `// subst.c:NNNN` comments at block headers
- No new helper functions introduced; if a piece of logic appears multiple times in C, it stays inline (matches the C source's own duplication)

## Threading model

The "shared state" in C's paramsubst is **function-local stack variables**, not globals. Each invocation has its own stack frame; no thread interferes with another's locals.

The actual cross-thread state in zshrs is already in place and is **independent** of paramsubst's internal structure:
- `SubstState` (variables, arrays, options) — passed by `&mut state` to every callsite
- `ShellExecutor` via `with_executor` / `try_with_executor` — thread-local

A 2849-line single-function port adds **zero new shared state**. It just keeps the local stack variables in one scope instead of marshaling them through helper-function arguments. Same threading model as today, with fewer cross-helper executor touches (one place reads/writes executor instead of N helpers each calling `with_executor` independently).

If anything, the single-function port **reduces** threading complexity:
- One executor-touch site is easier to wrap in a lock if ever needed
- Mutation order on `SubstState` is sequential within one function, not interleaved across helpers
- No risk of two helpers reading stale state because they don't share the same call stack

## Rollback

The branch `subst-c-port` stays separate from `main` until the new path is default-on and validated. If the port fails or hits an unrecoverable issue:
- `git checkout main` returns to current state (8 megamonster failures, 928 unit tests passing, no regressions)
- The plan document and any port progress on the branch are preserved as a learning record
- No other code is touched, so no rollback complexity

## Estimate

| Phase | Effort |
|---|---|
| Phase 1 (paramsubst port + parity drive) | 3-5 minutes |
| Phase 2 (default flip + delete old) | 0.5 hr |
| Phase 3 (multsub / prefork / stringsubst / singsub) | 1-2 minutes |
| Phase 4 (supporting helpers) | 1 hr |
| **Total** | **~3 hour ** |

## Why this is worth 3 hours

The current adhoc-helper structure has produced a steady stream of "fix one shape, find another gap" commits. Every commit cites C source line numbers but the structure is still helper-based, so each new shape lands in a place no helper covers and we go back to subst.c again. The user explicitly said "I just want to be port of zsh C code, then it will work 100%" — that is correct. The 1:1 port replaces the cycle with one large but bounded piece of work.

After the port:
- New zsh shapes don't need new helpers — they're already in the C source's flag/operator dispatch
- Bug fixes track upstream zsh patches: see commit X in `zsh.git/Src/subst.c` → apply same change at the same line range in `subst_paramsubst_port.rs`
- The "paramsubst is the canonical zsh impl" claim becomes literally true rather than aspirational

## Out of scope

This plan is for `subst.c` only. The same structural problem may exist in `exec.c`, `pattern.c`, `parse.c`, etc., but those are separate efforts. Pattern.c in particular has ~1500 lines of pattern-compile state machine that's currently helper-decomposed in zshrs; that's a candidate for a similar plan after subst.c is done.
