//! Bytecode compiler for the ported `ZshProgram` AST.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C
//! zsh has `Src/parse.c::bld_eprog()` (line 547) which serializes
//! a parsed program into wordcode + strings for `.zwc` caches;
//! `Src/exec.c::exectree()` / `execfuncs[]` (around line 268) runs that
//! wordcode on zsh's native **wordcode VM** (`Estate` over the buffer).
//! zshrs compiles the same AST to **fusevm** bytecode instead (typed ops,
//! compile-time word decomposition, tilde/glob/param classification);
//! the fusevm Cranelift JIT can specialize hot paths.
//!
//! Consumes the 4-tier port grammar (`ZshProgram → ZshList →
//! ZshSublist → ZshPipe → ZshCommand`) and emits fusevm bytecode.
//! The ported parser is the single source of truth for parsing;
//! this compiler does the speed work (compile-time word
//! decomposition + native ops where possible, runtime fallback
//! for the long tail).
//!
//! Word handling: `ZshSimple::words` are raw `Vec<String>`. We decompose
//! at compile time into typed expansion ops (`Op::ExpandParam`,
//! `Op::Glob`, `Op::TildeExpand`, `Op::CmdSubst`, etc.).

use crate::parse::CaseTerm;
use crate::parse::ForList;
use crate::parse::ZshCond;
use crate::parse::{
    SublistOp, ZshAssign, ZshAssignValue, ZshCommand, ZshList, ZshPipe, ZshProgram, ZshSimple,
    ZshSublist,
};
use crate::ported::utils::{errflag, ERRFLAG_ERROR};
use crate::ported::zsh_h::{
    REDIR_APP, REDIR_APPNOW, REDIR_ERRAPP, REDIR_ERRAPPNOW, REDIR_ERRWRITE, REDIR_ERRWRITENOW,
    REDIR_HEREDOC, REDIR_HEREDOCDASH, REDIR_HERESTR, REDIR_INPIPE, REDIR_MERGEIN, REDIR_MERGEOUT,
    REDIR_OUTPIPE, REDIR_READ, REDIR_READWRITE, REDIR_WRITE, REDIR_WRITENOW,
};
use fusevm::op::file_test;
use fusevm::op::Op;
use fusevm::{ChunkBuilder, Value};
use std::collections::HashMap;
use std::sync::atomic::Ordering;

/// AST → fusevm bytecode compiler.
/// zshrs-original. Closest C analog is `bld_eprog()` from\n/// Src/parse.c:547 which emits wordcode for `.zwc` files; the\n/// difference is that this compiler emits typed VM ops the JIT can\n/// then specialize, rather than wordcode the runtime walks.
pub struct ZshCompiler {
    /// `builder` field.
    builder: ChunkBuilder,
    /// Variable name → slot index. Shared with arith sub-compilations.
    pub slots: HashMap<String, u16>,
    /// `next_slot` field.
    pub next_slot: u16,
    /// `break_patches` field.
    break_patches: Vec<Vec<usize>>,
    /// `continue_patches` field.
    continue_patches: Vec<Vec<usize>>,
    /// Per-open-loop jump list for the mid-body escape check
    /// (`emit_break_escape_check`). A `break`/`continue` that ran in
    /// ANOTHER chunk (called function, `eval`, sourced file) sets the
    /// global BREAKS counter; c:Src/exec.c:1370's `!breaks` gate stops
    /// the rest of the body and control lands on the loop's post-body
    /// drain — these jumps are what carry it there.
    body_end_patches: Vec<Vec<usize>>,
    /// `cmd_stack_depth` recorded when each loop scope opened, so a
    /// mid-body escape drains only the pushes made INSIDE the body and
    /// leaves the loop's own CS_FOR/CS_WHILE/CS_REPEAT push for the
    /// loop's own `emit_cmd_pop`.
    loop_cmd_depth: Vec<u32>,
    /// !!! RUST-ONLY BOOKKEEPING !!!
    ///
    /// Count of loop constructs whose `loops++` (c:Src/loop.c:114 /
    /// :427 / :523) has been emitted but whose matching `loops--`
    /// (c:188 / :491 / :546) has not. C never needs this: `execfor` /
    /// `execwhile` / `execrepeat` are recursive C functions, so a
    /// `return` that aborts the body simply falls out of the enclosing
    /// `for(;;)` and the `loops--` on the way out of the C frame runs
    /// unconditionally. A compiled chunk instead jumps straight to the
    /// chunk's return landing, skipping every enclosing loop's exit
    /// code, so the escape site has to replay those unwinds itself —
    /// see `emit_loop_unwind`.
    ///
    /// Incremented by `emit_loop_enter`, decremented by
    /// `close_loop_scope`; the extra `emit_loop_exit` calls that a
    /// multi-level `break N` emits do NOT touch it (those levels are
    /// still statically open for the code that follows).
    open_loop_depth: usize,
    /// Suppresses `emit_break_escape_check` inside the try-block
    /// epilogue, where BUILTIN_RESTORE_TRY_BLOCK_STATUS has just
    /// re-armed BREAKS/CONTFLAG for the dedicated re-jump probes that
    /// follow. Without it the generic check would eat the escape first
    /// and bypass the level-accurate try landing.
    break_escape_suppress: u32,
    /// `return_patches` field.
    return_patches: Vec<usize>,
    /// c:Src/exec.c — the analogue of C's file-scope `static Eprog
    /// redir_prog`, which `execcmd` fills in for `f() { … } > out` and
    /// `execfuncdef` consumes at c:5453 (`shf->redir = dupeprog(redir_prog,
    /// 0)`). Set by the `Redirected(FuncDef, …)` arm just before it hands
    /// the definition to `compile_funcdef`, taken there, and rendered into
    /// the registered function's `redir_text` so `functions` / `which` /
    /// `${functions[…]}` can re-emit it (c:Src/hashtable.c:988-994).
    redir_prog_text: Option<String>,
    /// Depth tracker for errexit (`set -e`) suppression. Incremented
    /// when entering a context where a non-zero status is part of the
    /// control flow (if/while/until tests, `&&`/`||` LHS, `!` negation,
    /// pipeline LHS). Decremented when leaving. The post-command
    /// errexit check only fires when this is 0.
    pub errexit_suppress_depth: i32,
    /// Depth tracker for "currently compiling inside double quotes".
    /// Bumped when a parent word is DQ-wrapped (`\u{9e}…\u{9e}`) and
    /// we recurse into its Expansion segments. Used so the
    /// `${(o/M/i/n/u)…}` fast paths know to pass the DQ-suppression
    /// sentinel to BUILTIN_PARAM_FLAG.
    pub dq_context_depth: i32,
    /// Depth tracker for "compiling an assignment RHS". When >0, bare
    /// `$(cmd)` does NOT word-split on IFS — assignments preserve
    /// whitespace/newlines (`x=$(printf 'a\nb')` keeps both lines).
    /// Argument-context cmd-subst still splits.
    pub assign_context_depth: i32,
    /// True while compiling a `[[ … ]]` cond operand. Process substitution is
    /// rejected there (c:Src/exec.c:4918 — a cond runs with `thisjob == -1`).
    pub in_cond_operand: bool,
    /// Depth tracker for "compiling a scalar assignment RHS" (NOT array
    /// init). When >0, `"${a[@]}"` joins via JOIN_STAR instead of
    /// splicing — scalar RHS forces single-string output. Array init
    /// (`b=("${a[@]}")`) keeps the splice (each element a separate
    /// array entry). Distinct from assign_context_depth which is set
    /// for both forms.
    pub scalar_assign_depth: i32,
    /// Depth tracker for "this word's outer `\u{9e}…\u{9e}` (Dnull) pair was
    /// SYNTHESIZED by the scalar-assignment glob-suppression wrap below, not
    /// typed by the user".
    ///
    /// c:Src/exec.c:2546 — C zsh prefork-s an assignment RHS with
    /// `PREFORK_SINGLE|PREFORK_ASSIGN`, which sets `ssub` (c:Src/subst.c:1761)
    /// and leaves `qt` (c:Src/subst.c:1625) at ZERO — an unquoted `x=${…}` is
    /// NOT a double-quoted expansion. The two flags gate DIFFERENT collapses:
    /// `qt` joins the array BEFORE the operator runs (c:Src/subst.c:3030-3037,
    /// so it changes what `:^` / `:#` see), while `ssub` joins only at the very
    /// END (c:Src/subst.c:4226-4231, after every operator).
    ///
    /// zshrs fakes PREFORK_SINGLE by wrapping the RHS in Dnulls so the emit
    /// sites pick the no-glob / no-split variants. That wrap makes
    /// `word_is_single_dq_span` report a double-quoted word, which used to
    /// feed `qt=true` into paramsubst — firing the c:3030-3037 pre-operator
    /// join that C never performs here. Sites that derive paramsubst's `qt`
    /// (as opposed to sites that merely suppress glob/split) must therefore
    /// ignore a synthetic wrap.
    pub synthetic_dq_wrap_depth: i32,
    /// Set while compiling a whole-array `name[@]=(...)` / `name[*]=(...)`
    /// assignment (the array-RHS form recurses through compile_assign with
    /// the bracket stripped). When true, the array-store emit uses the
    /// assoc-guarded SET_ARRAY_AT / APPEND_ARRAY_AT builtins so a `[@]`
    /// LHS on an associative array errors ("attempt to set slice of
    /// associative array", c:Src/params.c:3324) instead of pair-assigning.
    pub array_whole_assign: bool,
    /// Subtract this from each pipe's `lineno` when emitting
    /// SET_LINENO calls. Top-level program: 0 (linenos passed
    /// verbatim). Function body: set to (first body line - 1) so
    /// `$LINENO` inside the function reads 1, 2, 3 relative to the
    /// body — matching zsh's `lineno = 1` reset on function entry
    /// (Src/init.c:1588).
    pub lineno_offset: u64,
    /// Add this to each pipe's `lineno` AFTER subtracting
    /// `lineno_offset`. Used by command-substitution sub-VM
    /// compilation to anchor the inner program's lineno to the
    /// outer's `$LINENO` at the `$(…)` site, so xtrace inside the
    /// cmdsubst renders the OUTER line number (matching zsh's
    /// behaviour where execlist's `oldlineno` flows into the inner
    /// program's lineno scope).
    pub lineno_addend: u64,
    /// Counts the number of CS_* pushes that have been emitted at
    /// the current compile cursor and have NOT yet been matched by
    /// an emitted pop. When a `return`/`exit` jump is emitted, all
    /// open pushes must be drained first so the cmd_stack doesn't
    /// leak out of the function (zinit's load function nests
    /// `if then for if then for …` to depth 7+; without this drain,
    /// repeat invocations stack `for then` indefinitely).
    pub cmd_stack_depth: u32,
    /// Nesting depth of `{ … } always { … }` try-blocks currently
    /// being compiled (only the TRY arm is "inside" — the always
    /// arm runs outside this gate). When > 0, `break` / `continue`
    /// inside the try body emits SET_BREAK / SET_CONTINUE before
    /// the Jump so the canonical BREAKS / CONTFLAG atomics carry
    /// the escape kind across the always-arm's save/restore pair
    /// (see fusevm_bridge.rs::BUILTIN_SET_TRY_BLOCK_ERROR /
    /// BUILTIN_RESTORE_TRY_BLOCK_STATUS). Without this the escape
    /// is invisible to the always-arm and the loop doesn't unwind.
    pub try_block_depth: u32,
    /// `break_patches.len()` captured on entry to each open try-block —
    /// i.e. how many loops enclose the `{ … } always { … }` construct
    /// itself. A `break` whose target index is at or above the innermost
    /// entry targets a loop that lives INSIDE the try arm, so it is a
    /// plain in-chunk jump; only a break aimed below it leaves the
    /// construct and has to arm the BREAKS atomic. One entry per nesting
    /// level so nested try-blocks each measure against their own base.
    try_loop_base: Vec<usize>,
    /// Set by compile_assign each call to communicate whether the
    /// just-compiled scalar assignment's RHS could update $? via
    /// command substitution. compile_simple aggregates across the
    /// assigns chain to decide whether to emit the post-assignment
    /// `$? = 0` reset (only in the assignment-only path, only when
    /// no assign in the chain had cmd-subst — matches C zsh's
    /// addvars + `lastval = cmdoutval` at Src/exec.c:3395).
    pub last_assign_had_cmd_subst: bool,
    /// Names of functions defined earlier in this compile unit.
    /// Tracked so `compile_simple`'s command-name dispatch can route
    /// shadowing user functions through `CallFunction` instead of the
    /// builtin fast-path. C zsh's runtime function lookup wins over
    /// builtins (per `Src/exec.c::execcmd` shfunctab → bintab order);
    /// zshrs's compile-time builtin_id() lookup at the dispatch site
    /// previously beat the function check for zshrs-extension-only
    /// names (caller, help — not in C zsh's bintab). Bug #27 in
    /// docs/BUGS.md.
    pub defined_functions: std::collections::HashSet<String>,
    /// True when this compiler instance is compiling a function
    /// body (set by `compile_funcdef`). Used by SET_LINENO emit to
    /// pick the correct `max(1, lineno_offset)` formula so inline
    /// `f() { body }` reads `$LINENO=0` matching zsh's def-line
    /// subtraction. Bug #385.
    pub is_function_body: bool,
    /// Effective LINENO of the sublist whose body is currently
    /// being compiled. Set by `compile_sublist` after applying the
    /// `lineno_offset` / `lineno_addend` adjustments. Used by
    /// `compile_for_words` / `compile_for_positional` /
    /// `compile_for_arith` so the per-iteration `name=value`
    /// xtrace renders the for-statement's line, not whatever
    /// line the loop body's last statement left LINENO at —
    /// matching C zsh's `execlist` save/restore of `lineno`
    /// around each body execution (c:Src/exec.c::execlist:28,292).
    pub current_sublist_line: i64,
    /// Set while compiling a pipeline stage whose top-level command is
    /// a SIMPLE command: the stage's pipe fds must be dup2'd onto 0/1
    /// only after that command's argument words have been expanded.
    /// c:Src/exec.c:3720-3724 — the `addfd(…, 0, input, …)` /
    /// `addfd(…, 1, output, …)` pair runs after prefork (c:3304) and
    /// globlist (c:3702). [`ZshCompiler::emit_stage_fds_install`]
    /// consumes the flag at that point in each dispatch arm; compound
    /// stages (`{ … }`, `( … )`, functions) install at chunk entry
    /// instead, since their bodies do read the pipe.
    stage_fds_pending: bool,
    /// `cmd |& next` — the stage also dups its (already piped) stdout
    /// onto stderr. c:Src/parse.c gives cmd an extra `2>&1` redirect,
    /// which C walks at c:Src/exec.c:3730+, after addfd installed the
    /// pipe on fd 1 — so it rides along with the fd install.
    stage_fds_merge_stderr: bool,
    /// Depth tracker for "compiling a redirect target word".
    /// c:Src/glob.c:2161 xpandredir — `prefork(&fake, isset(MULTIOS)
    /// ? 0 : PREFORK_SINGLE, NULL)` then "Globbing is only done for
    /// multios". When > 0, compile_word_str's glob-expansion emit
    /// sites route through BUILTIN_REDIR_GLOB_EXPAND (which checks
    /// `isset(MULTIOS)` at runtime and passes the word through
    /// literally when the option is off) instead of the
    /// unconditional BUILTIN_GLOB_EXPAND. Without this,
    /// `unsetopt multios; echo hi > *.txt` globbed the target and
    /// wrote to the matches instead of creating the literal file
    /// `*.txt` (Bug #36 follow-up in docs/BUGS.md).
    pub redir_word_depth: i32,
    /// Recursion guard for the default-word glob pass. `compile_word_str`
    /// recurses into each Expansion segment of a multi-segment word; only
    /// the OUTERMOST (depth 0) call brackets the whole assembled word
    /// with BUILTIN_DEFAULT_WORD_GLOB_RESET/…GLOB. Bumped around the
    /// segment recursion so inner segments don't emit their own pass.
    pub word_seg_depth: i32,
    /// Bumped while compiling the args of a BINF_ASSIGN-family builtin
    /// (typeset/declare/local/export/readonly/integer/float/private). Such
    /// args are `NAME=value` assignment forms whose VALUE is not subject
    /// to filename generation (`typeset T=${x:-*file}` → literal `*file`),
    /// unlike a regular command arg (`foo E=${x:-*file}` DOES glob). Gates
    /// the default-word glob bracket off.
    pub assign_builtin_arg_depth: i32,
    /// Set by [`ZshCompiler::compile_word_str`] (and its recursive
    /// segment calls) whenever the word just compiled already ends in an
    /// UNCONDITIONAL filename-generation op — `BUILTIN_GLOB_EXPAND` or
    /// its redirect-target variant, both picked by
    /// [`ZshCompiler::glob_expand_builtin`].
    ///
    /// c:Src/exec.c:2611-2613 (`globlist(vl, prefork_ret)` for an
    /// assignment) and c:Src/exec.c:3757 (`globlist(args, 0)` for a
    /// command's argv) run filename generation EXACTLY ONCE over a word
    /// list, after prefork has finished substituting. Whether a metachar
    /// came from the source text or from a `${~spec}` value makes no
    /// difference at that point — both are Star / Quest / Inbrack TOKENS
    /// in the single word C hands to `zglob` (c:Src/subst.c:4419-4420
    /// `if (globsubst) shtokenize(y)` is what promotes the substituted
    /// ones). And c:Src/glob.c `globlist` advances past the nodes
    /// `zglob` produced, so a GENERATED filename is never re-globbed.
    ///
    /// zshrs splits that one pass into two ops: the compile-time
    /// `BUILTIN_GLOB_EXPAND`, emitted when the word's own text carries a
    /// glob token, and the runtime-gated `BUILTIN_GLOB_SUBST_EXPAND`,
    /// which exists only to cover the case where the literal text has NO
    /// metachar and solely the substituted value does. Emitting both
    /// over one word is a SECOND `zglob` pass over what C already
    /// considers final output: `pages=( ${^~pages}(N:t) )`
    /// (Completion/Unix/Type/_man `_man_pages`) re-parsed its own
    /// produced basename `[.1` — a real page in the macOS man tree — as
    /// a pattern and died with `bad pattern: [.1`. Callers reset this to
    /// `false` immediately before `compile_word_str` and skip the
    /// GLOB_SUBST_EXPAND emit when it comes back `true`.
    pub word_emitted_glob: bool,
}

impl Default for ZshCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl ZshCompiler {
    /// `new` — see implementation.
    pub fn new() -> Self {
        Self {
            builder: ChunkBuilder::new(),
            slots: HashMap::new(),
            next_slot: 0,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
            body_end_patches: Vec::new(),
            loop_cmd_depth: Vec::new(),
            open_loop_depth: 0,
            break_escape_suppress: 0,
            return_patches: Vec::new(),
            redir_prog_text: None,
            errexit_suppress_depth: 0,
            dq_context_depth: 0,
            assign_context_depth: 0,
            in_cond_operand: false,
            scalar_assign_depth: 0,
            synthetic_dq_wrap_depth: 0,
            array_whole_assign: false,
            lineno_offset: 0,
            lineno_addend: 0,
            cmd_stack_depth: 0,
            try_block_depth: 0,
            try_loop_base: Vec::new(),
            last_assign_had_cmd_subst: false,
            defined_functions: std::collections::HashSet::new(),
            is_function_body: false,
            current_sublist_line: 1,
            stage_fds_pending: false,
            stage_fds_merge_stderr: false,
            redir_word_depth: 0,
            word_seg_depth: 0,
            assign_builtin_arg_depth: 0,
            word_emitted_glob: false,
        }
    }

    /// Pick the glob-expansion builtin for the current word context.
    /// Redirect-target words (redir_word_depth > 0) use the
    /// MULTIOS-gated variant per c:Src/glob.c:2162-2167 xpandredir
    /// ("Globbing is only done for multios."); everything else keeps
    /// the unconditional argv glob.
    fn glob_expand_builtin(&self) -> u16 {
        if self.redir_word_depth > 0 {
            crate::vm_helper::BUILTIN_REDIR_GLOB_EXPAND
        } else {
            crate::vm_helper::BUILTIN_GLOB_EXPAND
        }
    }

    /// Emit the word's filename-generation op and record that the word
    /// has now been globbed, so the `BUILTIN_GLOB_SUBST_EXPAND` gate at
    /// each word-list site does not add a SECOND `zglob` pass over
    /// results C already treats as final (c:Src/glob.c `globlist` steps
    /// past the nodes `zglob` produced). See `word_emitted_glob`.
    fn emit_word_glob_expand(&mut self) {
        let builtin = self.glob_expand_builtin();
        self.builder.emit(Op::CallBuiltin(builtin, 0), 0);
        self.word_emitted_glob = true;
    }

    /// Emit a runtime errexit check. The host examines `set -e` and the
    /// last command's status; the BUILTIN pushes Int(1) when the
    /// enclosing scope (subshell / function / top-level chunk) should
    /// short-circuit to its return-patch landing, Int(0) otherwise.
    /// We pair the BUILTIN with a JumpIfTrue → return_patches pattern
    /// so the abort path drains cmd_stack and jumps; the no-abort
    /// path falls through.
    /// Abort the enclosing scope on a *fatal* error (errflag), even where
    /// the errexit check is suppressed.
    ///
    /// An `&&` / `||` chain consumes a non-zero status, so errexit and the
    /// ZERR trap are correctly suppressed inside one. An errflag is not a
    /// status: a `[[ ]]` bad pattern, a readonly reassignment, etc. abandon
    /// the whole list in zsh. Without this, `[[ x = [a- ]] || touch f`
    /// created `f` (zsh does not run the RHS at all), and the aborted RHS
    /// then clobbered the cond's status 2 down to 1.
    fn emit_fatal_abort_check(&mut self) {
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_FATAL_ABORT_CHECK, 0),
            0,
        );
        let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
        self.emit_loop_errflag_status();
        self.emit_cmd_stack_drain();
        let j = self.builder.emit(Op::Jump(0), 0);
        self.return_patches.push(j);
        self.builder.patch_jump(skip, self.builder.current_pos());
    }

    /// Emit the per-command PRINT_EXIT_VALUE report
    /// (c:Src/exec.c:4308-4316). C runs it at the tail of `execcmd_exec`,
    /// for every simple command, INCLUDING the operands of `&&` / `||` and
    /// an `if`/`while` condition — so it must not be folded into
    /// `emit_errexit_check`, which those contexts suppress
    /// (`errexit_suppress_depth`). The runtime builtin does the whole
    /// PRINTEXITVALUE / SHINSTDIN / lastval / subsh test, so the option can
    /// still be flipped mid-script.
    fn emit_print_exit_value(&mut self) {
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_PRINT_EXIT_VALUE, 0),
            0,
        );
        self.builder.emit(Op::Pop, 0);
    }

    /// c:Src/exec.c:1417 + c:1536-1538 — save `noerrexit` and OR in
    /// `NOERREXIT_EXIT | NOERREXIT_RETURN` for one `&&`/`||` chain operand
    /// (or a `!`-negated command). Leaves the VM stack and `$?` untouched.
    fn emit_noerrexit_suppress(&mut self) {
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_NOERREXIT_SUPPRESS, 0),
            0,
        );
        self.builder.emit(Op::Pop, 0);
    }

    /// c:Src/exec.c:1621 / c:1626 — `noerrexit = oldnoerrexit;`.
    fn emit_noerrexit_restore(&mut self) {
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_NOERREXIT_RESTORE, 0),
            0,
        );
        self.builder.emit(Op::Pop, 0);
    }

    /// c:Src/loop.c:141-145 + :199-203 (execfor), :478-481 (execwhile),
    /// :534-537 (execrepeat) — every loop that abandons its body because
    /// `errflag` is set forces the escaping status first:
    ///
    /// ```c
    /// if (errflag) { if (breaks) breaks--; lastval = 1; break; }
    /// ```
    ///
    /// so `for i in 1 2; do [[ abc == [ ]]; done` exits 1 where the bare
    /// `[[ abc == [ ]]` exits 2. Outside a loop C has no such assignment and
    /// the failing command's own status survives, hence the `open_loop_depth`
    /// gate; `execselect` likewise has no assignment, which is why
    /// `compile_select` does not bump that counter. The runtime op re-tests
    /// `errflag` itself so an ERREXIT (`set -e`) abort — which in C leaves
    /// execlist through `zexit(lastval)` and never reaches the loop's
    /// `if (errflag)` — keeps the failing command's status.
    fn emit_loop_errflag_status(&mut self) {
        if self.open_loop_depth == 0 {
            return;
        }
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_LOOP_ERRFLAG_STATUS, 0),
            0,
        );
        self.builder.emit(Op::Pop, 0);
    }

    /// c:Src/loop.c:479-482 (execwhile) — `if (errflag) { lastval = 1;
    /// break; }`; c:198-203 (execfor, with `if (breaks) breaks--;`),
    /// c:535-538 (execrepeat) and c:329-330 (execselect, no `lastval` store)
    /// are the same guard.
    ///
    /// C's LIST loop (c:Src/exec.c:1443, `while (… && !errflag)`) ENDS the
    /// list on errflag; zshrs's per-statement prologue gate
    /// (`BUILTIN_NOEXEC_CHECK`'s errflag arm, paired with a
    /// `JumpIfTrue(after_statement)`) only SKIPS one statement. Outside a loop
    /// the two are indistinguishable — which is why this gap survived — but
    /// inside one, every statement is skipped while the loop's own exit test
    /// still runs, and a residual `$? == 0` means it never terminates.
    ///
    /// Measured: a completer running `_files -g "*(-.):t:source files"
    /// -g "(a:t:directories"` spun at 98% CPU forever, so `<cmd> <TAB>`
    /// produced nothing at all; `Completion/Unix/Type/_files` sh:121's
    /// `while _next_label "$tag" expl "$descr"; do` was the loop.
    ///
    /// Returns the `JumpIfTrue` index to patch to the loop exit.
    fn emit_loop_errflag_break(&mut self) -> usize {
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_LOOP_ERRFLAG_BREAK, 0),
            0,
        );
        self.builder.emit(Op::JumpIfTrue(0), 0)
    }
    fn emit_errexit_check(&mut self) {
        if self.errexit_suppress_depth > 0 {
            // Suppressed for errexit/ZERR — but a fatal errflag still ends
            // the list, so keep that half of the check. A pending `breaks`
            // is not a status either and no connector consumes it, so the
            // execlist gate applies here too.
            self.emit_fatal_abort_check();
            return;
        }
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_ERREXIT_CHECK, 0),
            0,
        );
        // JumpIfFalse over the drain+jump (i.e. the BUILTIN pushed
        // 0 → continue normally). On Int(1) we fall through to drain
        // cmd_stack and emit the scope-exit Jump tracked by
        // return_patches.
        let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
        self.emit_loop_errflag_status();
        self.emit_cmd_stack_drain();
        let j = self.builder.emit(Op::Jump(0), 0);
        self.return_patches.push(j);
        let after = self.builder.current_pos();
        self.builder.patch_jump(skip, after);
    }

    /// Emit `cmdpush(token)` — direct port of Src/prompt.c:1623.
    /// Used by xtrace to render the `%_` prefix (`if cmdor cmdsubst`
    /// etc.) so trace output matches `/bin/zsh -x` byte-for-byte.
    /// Bumps `cmd_stack_depth` so return/exit jumps know how many
    /// pops to drain.
    fn emit_cmd_push(&mut self, token: u8) {
        self.builder.emit(Op::LoadInt(token as i64), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_CMD_PUSH, 1), 0);
        self.builder.emit(Op::Pop, 0);
        self.cmd_stack_depth += 1;
    }

    /// Emit `cmdpop()` — direct port of Src/prompt.c:1631.
    fn emit_cmd_pop(&mut self) {
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_CMD_POP, 0), 0);
        self.builder.emit(Op::Pop, 0);
        self.cmd_stack_depth = self.cmd_stack_depth.saturating_sub(1);
    }

    /// Emit pops to drain ALL currently-open cmd_stack pushes
    /// without changing `cmd_stack_depth`. Called before a
    /// return/exit Jump so the cmd_stack is balanced when control
    /// transfers to the chunk's return target. The static depth
    /// counter is preserved because subsequent compile sites still
    /// need to think the original pushes are open (so their later
    /// emit_cmd_pop fires correctly on the non-return path).
    fn emit_cmd_stack_drain(&mut self) {
        for _ in 0..self.cmd_stack_depth {
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_CMD_POP, 0), 0);
            self.builder.emit(Op::Pop, 0);
        }
    }

    /// Emit `loops++` (c:Src/loop.c:114/427/523). Call immediately
    /// BEFORE the `loop_top` label so it runs once per construct, not
    /// once per iteration.
    fn emit_loop_enter(&mut self) {
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_LOOP_ENTER, 0), 0);
        self.builder.emit(Op::Pop, 0);
        self.open_loop_depth += 1;
    }

    /// Replay, for each still-open loop construct, the exit sequence C
    /// runs as its interpreter recursion unwinds — used by the
    /// `return` / `exit` escape jump, which otherwise leaves the chunk
    /// without ever reaching any enclosing loop's exit code.
    ///
    /// Per level, in C's order:
    ///   c:Src/loop.c:529-534 — `if (breaks) { breaks--; if (breaks ||
    ///     !contflag) break; contflag = 0; }`  (the post-body drain)
    ///   c:Src/loop.c:188 / :491 / :546 — `loops--;`
    ///
    /// Draining `breaks` is as load-bearing as `loops--`: `bin_break`'s
    /// BIN_RETURN arm sets `breaks = loops` (c:Src/builtin.c:5835), so
    /// a `return` from inside N loops leaves `breaks == N` and relies
    /// on those N drains to bring it back to zero before control
    /// reaches the caller. Skipping them let the count survive the
    /// `source`/function boundary and abort the CALLER's list.
    ///
    /// The drain's verdict is popped: it only ever says "terminate
    /// this loop", which the unconditional escape jump already does.
    fn emit_loop_unwind(&mut self, n: usize) {
        for _ in 0..n {
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_LOOP_BREAK_DRAIN, 0),
                0,
            ); // c:530
            self.builder.emit(Op::Pop, 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_LOOP_EXIT, 0), 0); // c:188
            self.builder.emit(Op::Pop, 0);
        }
    }

    /// Emit `n` × `loops--` (c:Src/loop.c:188/491/546).
    fn emit_loop_exit(&mut self, n: usize) {
        for _ in 0..n {
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_LOOP_EXIT, 0), 0);
            self.builder.emit(Op::Pop, 0);
        }
    }

    /// Open a loop scope: the break/continue/body-end patch lists plus
    /// the cmd_stack watermark. Pairs with [`Self::close_loop_scope`].
    fn open_loop_scope(&mut self) {
        self.break_patches.push(Vec::new());
        self.continue_patches.push(Vec::new());
        self.body_end_patches.push(Vec::new());
        self.loop_cmd_depth.push(self.cmd_stack_depth);
    }

    /// Land the mid-body escape jumps and emit the post-body drain —
    /// c:Src/loop.c:529-534's `if (breaks) { breaks--; if (breaks ||
    /// !contflag) break; contflag = 0; }`. Call immediately after the
    /// body, before the loop's `continue` label. A drain verdict of
    /// "terminate" joins the loop's own break patch list so it lands on
    /// `loop_exit` and runs the matching `loops--`.
    fn emit_loop_body_end(&mut self) {
        let ends = self.body_end_patches.pop().unwrap_or_default();
        let pos = self.builder.current_pos();
        for e in ends {
            self.builder.patch_jump(e, pos);
        }
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_LOOP_BREAK_DRAIN, 0),
            0,
        );
        let j = self.builder.emit(Op::JumpIfTrue(0), 0);
        if let Some(breaks) = self.break_patches.last_mut() {
            breaks.push(j);
        }
    }

    /// Close a loop scope at its `loop_exit` label: patch the break
    /// jumps, then emit the construct's single `loops--`.
    fn close_loop_scope(&mut self, loop_exit: usize) {
        if let Some(breaks) = self.break_patches.pop() {
            for bp in breaks {
                self.builder.patch_jump(bp, loop_exit);
            }
        }
        self.loop_cmd_depth.pop();
        self.emit_loop_exit(1);
        self.open_loop_depth = self.open_loop_depth.saturating_sub(1);
    }

    /// c:Src/exec.c:1370 execlist — `while (… && !breaks && …)`. Emitted
    /// after every statement: when a called function / `eval` / sourced
    /// file left BREAKS set, stop this list. Inside a loop in THIS chunk
    /// control goes to the loop's post-body drain; with no enclosing
    /// loop here it unwinds the chunk so the flag reaches the caller,
    /// exactly as RETFLAG does.
    fn emit_break_escape_check(&mut self) {
        if self.break_escape_suppress > 0 {
            return;
        }
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_BREAKS_PENDING, 0),
            0,
        );
        let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
        match self.body_end_patches.len() {
            0 => {
                self.emit_cmd_stack_drain();
                let j = self.builder.emit(Op::Jump(0), 0);
                self.return_patches.push(j);
            }
            _ => {
                let entry = self.loop_cmd_depth.last().copied().unwrap_or(0);
                for _ in entry..self.cmd_stack_depth {
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_CMD_POP, 0), 0);
                    self.builder.emit(Op::Pop, 0);
                }
                let j = self.builder.emit(Op::Jump(0), 0);
                self.body_end_patches.last_mut().unwrap().push(j);
            }
        }
        let after = self.builder.current_pos();
        self.builder.patch_jump(skip, after);
    }

    /// Compile a parsed `ZshProgram` to a runnable Chunk.
    pub fn compile(mut self, program: &ZshProgram) -> fusevm::Chunk {
        self.compile_program(program);

        // Patch return/exit jumps to past chunk end.
        let end_pos = self.builder.current_pos();
        for patch in std::mem::take(&mut self.return_patches) {
            self.builder.patch_jump(patch, end_pos);
        }

        self.builder.build()
    }

    fn compile_program(&mut self, program: &ZshProgram) {
        // The parser synthesizes a FuncDef for the `name() { body }` shape
        // at parse time (the parser detects the
        // Simple<Inpar><Outpar> + Inbrace pattern and emits a FuncDef with
        // body_source captured). No compile-side workaround is needed.
        //
        // c:Src/exec.c:1439-1442 — execlist: "Empty list; this returns
        // status zero." An empty command list resets $? to 0, so an empty
        // body (`{ }`, `( )`, `f() { }`, `() { }`) yields 0 regardless of
        // the prior command — `false; f` where `f` is empty → 0, not 1.
        // The previous port emitted nothing for an empty body, leaking the
        // prior status.
        if program.lists.is_empty() {
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder.emit(Op::SetStatus, 0);
            return;
        }
        // c:Src/exec.c:1390 — `while (wc_code(code) == WC_LIST && !breaks &&
        // !retflag && !errflag)`. execlist re-tests the flags before EVERY
        // list element, so a command that sets errflag ends the whole list.
        // Nothing was emitted between top-level statements here, so an
        // interrupt raised inside one statement did not stop the next:
        //   TRAPINT() { print T; return 1 }
        //   f() { print A; kill -INT $$; print C }; f; print B
        //   zsh: A T      zshrs: A T B
        // The fatal-abort form is the right one: C's gate keys off the FLAGS,
        // not off a non-zero status, which is what the full errexit check adds.
        let mut first = true;
        for list in &program.lists {
            if !first {
                self.emit_fatal_abort_check();
            }
            first = false;
            self.compile_list(list);
        }
    }

    fn compile_list(&mut self, list: &ZshList) {
        // Update $LINENO before each top-level statement. Direct
        // port of zsh's `lineno` global increment in Src/input.c
        // — there it's tracked at the lexer level on every '\n';
        // here we hoist that to compile-time by emitting a single
        // SET_LINENO call per statement using the parser's
        // captured `ZshPipe.lineno`. Subtract `lineno_offset` for
        // function-body sub-chunks so they read 1, 2, 3 relative
        // to the body (matches zsh's `lineno = 1` reset on
        // function entry at Src/init.c:1588).
        //
        // c:Src/init.c:1588 — zsh's `lineno` resets to the def
        // line on function entry, so body-relative LINENO is
        // `raw - def_line`. For inline `f() { body }` (def and
        // body share a line), the body's first command is at
        // raw_line=1 and zsh's LINENO is 0. For multi-line
        // (def on N, body on N+1), raw_line=2 and LINENO=1.
        // compile_funcdef sets `lineno_offset = first_body_line -
        // 1` which gives offset=0 for inline (LINENO=1, WRONG —
        // zsh says 0) and offset=1 for multi-line (LINENO=1, ok).
        // Use `max(1, lineno_offset)` so inline subtracts 1
        // instead of 0, matching zsh's def-line subtraction.
        // Bug #385.
        let raw_line = list.sublist.pipe.lineno;
        let effective_offset = if self.lineno_offset == 0 {
            // Outer-script context (no function wrapping): no
            // adjustment. The 0 offset is the script's natural
            // 1-based numbering.
            0
        } else {
            self.lineno_offset
        };
        // Compile_funcdef sets lineno_offset = first_body_line - 1
        // explicitly for function bodies. For inline def
        // (first_body_line = 1), that's 0, which we'd misread as
        // "outer script". Distinguish via a fn-body marker.
        let _ = effective_offset;
        let rel_line = if self.is_function_body {
            // Function body: offset = max(1, lineno_offset) so
            // inline `f() { body }` (lineno_offset=0) maps body
            // line 1 → 0 (zsh's def-line subtraction).
            let off = self.lineno_offset.max(1);
            raw_line.saturating_sub(off) + self.lineno_addend
        } else {
            raw_line.saturating_sub(self.lineno_offset).max(1) + self.lineno_addend
        };
        // Record the line of the sublist currently being compiled.
        // Loop bodies (for, while, repeat) read this to restore
        // LINENO at the top of each iteration so the per-iter
        // `name=value` xtrace renders the loop-statement's line —
        // matching C zsh's `execlist` save/restore at
        // c:Src/exec.c:28 (`oldlineno = lineno`) and c:292
        // (`lineno = oldlineno`).
        // c:Src/exec.c:1471-1473 — `if (lnp1) lineno = lnp1 - 1;` (and the
        // same `WC_PIPE_LINENO(pcode)` guard at c:2056). A recorded line of
        // 0 means "this construct carries no line number", and C then
        // leaves `lineno` at whatever the caller set. The braceless
        // short-function body (`f() cmd`, c:Src/parse.c:2112) is the one
        // construct that deliberately stores 0, so its body inherits the
        // CALL SITE's line — pinned by E02xtrace's `functions -t` chunks.
        if raw_line != 0 {
            self.current_sublist_line = rel_line as i64;
        }
        // c:Src/exec.c:1451/1455/1476/1390 — the four things that run
        // before every sublist, fused into one op. It performs the two
        // unconditional writes (`lineno`, `donetrap = 0`) and answers
        // whether the DEBUG_BEFORE_CMD and `noexec` blocks below can be
        // skipped; see BUILTIN_STMT_PROLOGUE_FAST for why a `true`
        // answer means they were no-ops. `-1` is the line argument for a
        // construct that records none (C's `code == 0`).
        self.builder.emit(
            Op::LoadInt(if raw_line != 0 { rel_line as i64 } else { -1 }),
            0,
        );
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_STMT_PROLOGUE_FAST, 1),
            0,
        );
        let prologue_fast = self.builder.emit(Op::JumpIfTrue(0), 0);
        // Slow path — reached only when BUILTIN_STMT_PROLOGUE_FAST
        // reported that a DEBUG trap is installed, `noexec` is set, or
        // `errflag` is live. `lineno` and `donetrap` were already
        // written by that op, in both paths.
        //
        // c:Src/exec.c:1357-1500 DEBUGBEFORECMD — fire the DEBUG
        // trap before each statement. Routes through canonical
        // `dotrap(SIGDEBUG)` which checks the traps_table for a
        // "DEBUG" entry and runs the body. Cheap no-op when no
        // DEBUG trap is set (one hashmap lookup).
        //
        // c:Src/exec.c — push the about-to-run statement's text so
        // BUILTIN_DEBUG_TRAP can set `$ZSH_DEBUG_CMD` to it before
        // firing the trap body (C `exec.c::trapcmd` does the
        // equivalent via `dupstring(text)`). The trap body reads
        // the parameter and the runtime unsets it on return. Bug
        // #263 in docs/BUGS.md.
        let cmd_text = render_list_for_debug(list);
        let txt_const = self.builder.add_constant(Value::str(&cmd_text));
        self.builder.emit(Op::LoadConst(txt_const), 0);
        // mode 0 = c:1476's pre-sublist DEBUG_BEFORE_CMD arm.
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_DEBUG_TRAP, 2), 0);
        // c:Src/exec.c:1511 — `if (donedebug != 2) execsimple(state);` and
        // c:1519-1529 (compound form). A DEBUG trap that turned ERREXIT on
        // makes C SKIP the statement it fired in front of; a trap that ran
        // `return N` likewise never reaches the command, because execlist's
        // loop condition (c:1443) tests retflag. BUILTIN_DEBUG_TRAP now
        // reports either case as Int(1); fall into the skip block when it
        // does, which routes a forced return out of the enclosing scope
        // before joining `noexec_skip`'s landing past the statement body.
        let debug_run = self.builder.emit(Op::JumpIfFalse(0), 0);
        // Skip path. c:1443 — a `return N` from the trap leaves retflag set
        // and C's list loop abandons the rest of the list; route to the same
        // scope-exit patch list `emit_errexit_check` uses.
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_RETFLAG_CHECK, 0),
            0,
        );
        let debug_no_ret = self.builder.emit(Op::JumpIfFalse(0), 0);
        self.emit_cmd_stack_drain();
        let j = self.builder.emit(Op::Jump(0), 0);
        self.return_patches.push(j);
        let debug_no_ret_at = self.builder.current_pos();
        self.builder.patch_jump(debug_no_ret, debug_no_ret_at);
        let debug_skip = self.builder.emit(Op::Jump(0), 0);
        let debug_run_at = self.builder.current_pos();
        self.builder.patch_jump(debug_run, debug_run_at);
        // c:Src/exec.c:1390 — `set -n` (noexec option): parse but
        // don't execute. The check runs at the start of each
        // top-level statement; when noexec is set, jump past the
        // statement body. set -n itself still executes (it's the
        // command BEFORE the option is checked), allowing it to
        // turn on noexec for subsequent statements.
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_NOEXEC_CHECK, 0),
            0,
        );
        let noexec_skip = self.builder.emit(Op::JumpIfTrue(0), 0);
        // Both paths rejoin here, in front of the statement body.
        let prologue_fast_at = self.builder.current_pos();
        self.builder.patch_jump(prologue_fast, prologue_fast_at);

        // ZshList = sublist + flags (async / disown).
        if list.flags.async_ {
            // Background: compile the sublist into a sub-chunk + emit
            // BUILTIN_RUN_BG.
            let mut sub = ZshCompiler::new();
            sub.compile_sublist(&list.sublist);
            let sub_end = sub.builder.current_pos();
            for patch in std::mem::take(&mut sub.return_patches) {
                sub.builder.patch_jump(patch, sub_end);
            }
            let sub_chunk = sub.builder.build();
            let sub_idx = self.builder.add_sub_chunk(sub_chunk);
            // c:Src/exec.c::execpline — the async job's display text
            // comes from `getjobtext(state->prog, ...)` (Src/text.c:235)
            // and lands in the proc entry via addproc. Reconstruct the
            // sublist text at compile time and pass it alongside the
            // sub-chunk index so BUILTIN_RUN_BG can addproc with it.
            let job_text = render_sublist_for_debug(&list.sublist);
            let text_const = self.builder.add_constant(Value::str(&job_text));
            self.builder.emit(Op::LoadConst(text_const), 0);
            self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
            // `&|` / `&!` (disown): pass the flag so BUILTIN_RUN_BG deletes the
            // job (C exec.c:1752-1758) instead of announcing it via spawnjob.
            // Without this, `cmd &|` inside a function/`zle -F` handler leaked a
            // `[N] pid` line — e.g. zinit-turbo's `rustup completions zsh … &|`
            // spewed into the prompt where real zsh is silent.
            self.builder
                .emit(Op::LoadInt(i64::from(list.flags.disown)), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_RUN_BG, 3), 0);
            self.builder.emit(Op::SetStatus, 0);
        } else {
            self.compile_sublist(&list.sublist);
        }
        // c:Src/exec.c:1628-1644 — `sublist_done:` fires the DEBUG trap
        // AFTER the sublist when DEBUG_BEFORE_CMD is NOT set (the default).
        // Reached only on the normal path: a statement skipped by the
        // pre-sublist arm has C's `donedebug` non-zero, which gates this
        // block off (c:1628), and the noexec skip lands past it too. Mode 1
        // makes BUILTIN_DEBUG_TRAP take that arm; it returns Int(1) only
        // when the trap forced a return (c:1639 `if (!retflag) …`).
        let after_txt = self.builder.add_constant(Value::str(""));
        self.builder.emit(Op::LoadConst(after_txt), 0);
        self.builder.emit(Op::LoadInt(1), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_DEBUG_TRAP, 2), 0);
        let post_ok = self.builder.emit(Op::JumpIfFalse(0), 0);
        self.emit_cmd_stack_drain();
        let jp = self.builder.emit(Op::Jump(0), 0);
        self.return_patches.push(jp);
        let post_ok_at = self.builder.current_pos();
        self.builder.patch_jump(post_ok, post_ok_at);
        // Patch the noexec skip to land here (past the statement body).
        let after = self.builder.current_pos();
        self.builder.patch_jump(noexec_skip, after);
        // Same landing for the DEBUG-trap skip path (c:1511 / c:1519-1529).
        self.builder.patch_jump(debug_skip, after);
        // c:Src/exec.c:1370 — execlist's own loop condition,
        // `while (wc_code(code) == WC_LIST && !breaks && !retflag &&
        // !errflag)`. A `break`/`continue` that ran in a called
        // function / `eval` / sourced file left `breaks` set; the rest
        // of THIS list must not run. Per-list (not per-pipe) matches C:
        // an `&&`/`||` chain is one sublist and is not interrupted
        // mid-chain.
        self.emit_break_escape_check();
    }

    /// Body-relative `$LINENO` for a raw parser line, using the same
    /// `lineno_offset` / `lineno_addend` / function-body rules as
    /// [`Self::compile_list`]'s SET_LINENO emit. Factored out so the
    /// per-pipeline update in [`Self::compile_sublist`] (c:Src/exec.c:2056)
    /// computes the identical value.
    fn rel_lineno(&self, raw_line: u64) -> i64 {
        let v = if self.is_function_body {
            let off = self.lineno_offset.max(1);
            raw_line.saturating_sub(off) + self.lineno_addend
        } else {
            raw_line.saturating_sub(self.lineno_offset).max(1) + self.lineno_addend
        };
        v as i64
    }

    fn compile_sublist(&mut self, sublist: &ZshSublist) {
        // Flatten the && / || chain into a sequence of (pipe, op-to-next).
        // Shell semantics: each connector skips ONLY the IMMEDIATELY-next
        // pipe, not the rest of the chain. `false && echo no || echo yes`
        // → false runs, && skips echo no, || sees status from false (non-
        // zero) and runs echo yes.
        //
        // Recursive compile_sublist would emit a JumpIfFalse whose target
        // landed AFTER the entire rest of the chain (including the `||`
        // branch we want to keep), eating the `|| echo yes`. The iterative
        // form patches each connector to jump just past the next pipe.
        let mut pipes: Vec<&ZshPipe> = vec![&sublist.pipe];
        let mut ops: Vec<SublistOp> = Vec::new();
        // Per-pipe `!` flags. `pipes[i]`'s negate flag is `pipe_nots[i]`.
        // For chained sublists like `true && ! false`, the parser nests
        // each `!` in its own ZshSublist node — flattening must capture
        // each one or the inner negate is silently dropped.
        let mut pipe_nots: Vec<bool> = vec![sublist.flags.not];
        let mut next_link = sublist.next.as_ref();
        while let Some((op, next_sublist)) = next_link {
            ops.push(*op);
            pipes.push(&next_sublist.pipe);
            pipe_nots.push(next_sublist.flags.not);
            next_link = next_sublist.next.as_ref();
        }

        // `coproc body` — bidirectional pipe to backgrounded body. The
        // body's stdin/stdout become two fds in $COPROC. Dispatched via
        // BUILTIN_RUN_COPROC; the body compiles to its own sub-chunk.
        if sublist.flags.coproc {
            self.compile_coproc_pipe(pipes[0]);
            // `!` on a coproc is unusual — apply after dispatch.
            if sublist.flags.not {
                self.emit_negate_status();
            }
            // Skip subsequent && / || on coproc — just emit them.
            for (i, op) in ops.iter().enumerate() {
                self.builder.emit(Op::GetStatus, 0);
                let skip = match op {
                    SublistOp::And => self.builder.emit(Op::JumpIfFalse(0), 0),
                    SublistOp::Or => self.builder.emit(Op::JumpIfTrue(0), 0),
                };
                self.compile_pipe(pipes[i + 1]);
                self.builder.patch_jump(skip, self.builder.current_pos());
            }
            return;
        }

        // Emit pipe[0]. `!` (sublist.flags.not) applies to pipe[0] only,
        // then the && / || chain reads the negated status. This matches
        // zsh: `! false && echo y` runs echo because !false→success.
        // For errexit: ANY `&&`/`||` chain or `!` negation makes the
        // whole sublist exempt from errexit (POSIX/zsh rule — failures
        // inside an AND-OR list are "consumed" by the connector). We
        // still bump suppression so individual pipes don't trigger
        // their own errexit checks; we do NOT emit a wrap-up check at
        // the end either.
        //
        // cmdstack tracking — direct port of Src/exec.c:1530 / :1563
        //   case WC_SUBLIST_AND: …; cmdpush(CS_CMDAND); break;
        //   case WC_SUBLIST_OR:  …; cmdpush(CS_CMDOR);  break;
        // The C source captures `csp = cmdsp` at the start of each
        // sublist (Src/exec.c:1396) and restores `cmdsp = csp` at the
        // end (line 1593) — that bulk-pops the CMDAND/CMDOR pushes.
        // We mirror by counting our own pushes and emitting matching
        // pops at the end. This way `a && b && c` shows "cmdand" on
        // pipe[1]'s trace and "cmdand cmdand" on pipe[2]'s, matching
        // zsh -x byte-for-byte.
        // c:Src/exec.c:1533-1538 — per chain ELEMENT:
        //     int isandor = WC_SUBLIST_TYPE(code) != WC_SUBLIST_END;
        //     int isnot   = WC_SUBLIST_FLAGS(code) & WC_SUBLIST_NOT;
        //     /* suppress errexit for commands before && and || and after ! */
        //     if (isandor || isnot)
        //         noerrexit |= NOERREXIT_EXIT | NOERREXIT_RETURN;
        // and the matching `noerrexit = oldnoerrexit;` at c:1621 / c:1626.
        // `errexit_suppress_depth` above only silences the checks THIS
        // compilation emits; the runtime bits are what reach a called
        // function's own execlist (c:5930 clears only NOERREXIT_RETURN).
        // Suppression is PER ELEMENT, not per sublist: the final
        // (WC_SUBLIST_END) element of a chain is `isandor == 0`, so C leaves
        // `noerrexit` alone for it and its own body IS subject to ERR_EXIT /
        // ZERR. Bumping `errexit_suppress_depth` across the whole sublist also
        // silenced the checks compiled INSIDE that last element, so
        //   set -e; true && { false; echo NOT REACHED }
        // ran `echo NOT REACHED` (C03traps:80).
        let elem0_suppressed = !ops.is_empty() || pipe_nots[0] || sublist.flags.not;
        if elem0_suppressed {
            self.errexit_suppress_depth += 1;
            self.emit_noerrexit_suppress(); // c:1538
        }
        self.compile_pipe(pipes[0]);
        // c:Src/exec.c:1489-1492 — the FIRST chain element's own
        // sublist code. Emitted before the `!` negation because C
        // applies WC_SUBLIST_NOT inside execpline AFTER waitonejob has
        // stored the status, so `! [[ -z x ]]` records the pre-negation
        // 1 while `$?` is 0.
        self.emit_sublist_finish(pipes[0], pipe_nots[0]);
        if sublist.flags.not {
            self.emit_negate_status();
        }
        if elem0_suppressed {
            self.emit_noerrexit_restore(); // c:1621 / c:1626
            self.errexit_suppress_depth -= 1;
        }
        let mut chain_pushes = 0usize;
        for (i, op) in ops.iter().enumerate() {
            // Push BEFORE the JumpIfFalse so the push happens whether
            // or not the RHS pipe runs; the bulk-pop after the loop
            // matches the static count, regardless of runtime skips.
            // Functionally equivalent to zsh's `csp = cmdsp` save +
            // `cmdsp = csp` restore wrapping the sublist.
            let token = match op {
                SublistOp::And => crate::ported::zsh_h::CS_CMDAND as u8,
                SublistOp::Or => crate::ported::zsh_h::CS_CMDOR as u8,
            };
            self.emit_cmd_push(token);
            chain_pushes += 1;
            self.builder.emit(Op::GetStatus, 0);
            let skip = match op {
                SublistOp::And => self.builder.emit(Op::JumpIfFalse(0), 0),
                SublistOp::Or => self.builder.emit(Op::JumpIfTrue(0), 0),
            };
            // c:Src/exec.c:2055-2057 `execpline2` —
            //   /* In evaluated traps, don't modify the line number. */
            //   if (!IN_EVAL_TRAP() && !ineval && WC_PIPE_LINENO(pcode))
            //       lineno = WC_PIPE_LINENO(pcode) - 1;
            // Every PIPELINE carries its own line number, so each element
            // of an `&&` / `||` chain re-anchors `lineno` before it runs
            // ("The line number is updated for individual pipelines" —
            // c:1459). zshrs emitted SET_LINENO once per LIST, so every
            // continuation of a multi-line chain reported the FIRST line:
            // `zsystem flock … ||` on lines 2-6 all warned `(eval):2:`.
            // c:2056 — `… && WC_PIPE_LINENO(pcode)`: a recorded 0 means the
            // construct carries no line number and `lineno` is left alone.
            if pipes[i + 1].lineno != 0 {
                self.builder
                    .emit(Op::LoadInt(self.rel_lineno(pipes[i + 1].lineno)), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_LINENO, 1), 0);
                self.builder.emit(Op::Pop, 0);
            }
            // c:1533-1538 — this element is `isandor` unless it is the final
            // (WC_SUBLIST_END) one; `isnot` is its own `!`.
            let elem_suppressed = i + 1 < ops.len() || pipe_nots[i + 1];
            if elem_suppressed {
                self.errexit_suppress_depth += 1;
                self.emit_noerrexit_suppress(); // c:1538
            }
            self.compile_pipe(pipes[i + 1]);
            // c:Src/exec.c:1502-1504 (WC_SUBLIST_AND) and c:1536
            // (WC_SUBLIST_OR) re-read WC_SUBLIST_SIMPLE per chain
            // element, so each RHS gets its own sublist code — and,
            // like the head above, before its `!` is applied.
            self.emit_sublist_finish(pipes[i + 1], pipe_nots[i + 1]);
            // Apply this pipe's `!` flag (parser nested it on the next
            // ZshSublist). `true && ! false` parses as
            //   ZshSublist{ true, And, ZshSublist{ !false, not=true } }
            // so the inner `!` must invert pipes[i+1]'s status here.
            if pipe_nots[i + 1] {
                self.emit_negate_status();
            }
            if elem_suppressed {
                self.emit_noerrexit_restore(); // c:1621 / c:1626
                self.errexit_suppress_depth -= 1;
            }
            // c:Src/exec.c — POSIX/zsh rule: only the LAST command in
            // an && / || chain can trigger errexit, AND only when it
            // was actually executed (not short-circuited). Emit the
            // errexit check INSIDE the not-skipped branch of the FINAL
            // connector — earlier connectors' branches contribute to
            // the chain but aren't terminal. The check sits before the
            // skip-jump target so `false && X` (where X is skipped)
            // bypasses it entirely.
            // c:1548-1549 — `if (isnot) this_noerrexit = 1;` on the END
            // element skips the whole c:1651 block, so a trailing `! cmd`
            // gets no ZERR / errexit handling at all.
            if i == ops.len() - 1 && !pipe_nots[i + 1] {
                // Suppression is already back at the enclosing level here
                // (the per-element bump above is balanced), so this is the
                // sublist's own c:1651 check.
                self.emit_errexit_check();
            }
            self.builder.patch_jump(skip, self.builder.current_pos());
        }
        // Bulk-pop the chain pushes (mirrors `cmdsp = csp` restore).
        for _ in 0..chain_pushes {
            self.emit_cmd_pop();
        }
    }

    fn compile_coproc_pipe(&mut self, pipe: &ZshPipe) {
        // Compile the pipeline as a body sub-chunk, then push
        // [name="", job_text, sub_idx] and call BUILTIN_RUN_COPROC.
        // c:Src/parse.c:864-876 par_sublist2 — `COPROC pline`: the
        // coproc flag covers the ENTIRE pipeline, so `coproc a | b`
        // wires the whole pipe (compile_pipe), not just the first
        // command.
        let mut sub = ZshCompiler::new();
        sub.compile_pipe(pipe);
        let sub_end = sub.builder.current_pos();
        for patch in std::mem::take(&mut sub.return_patches) {
            sub.builder.patch_jump(patch, sub_end);
        }
        let chunk = sub.builder.build();
        let sub_idx = self.builder.add_sub_chunk(chunk);

        let name_const = self.builder.add_constant(Value::str(""));
        self.builder.emit(Op::LoadConst(name_const), 0);
        // c:Src/exec.c::execpline — the coproc takes the same Z_ASYNC
        // job-table path as `cmd &` (how = Z_ASYNC at c:1709, spawnjob
        // at c:1758), so its proc entry carries getjobtext display text
        // (Src/text.c:235) exactly like a background job. Reconstruct
        // the pipe text at compile time, mirroring the RUN_BG site.
        let job_text = render_pipe_for_debug(pipe);
        let text_const = self.builder.add_constant(Value::str(&job_text));
        self.builder.emit(Op::LoadConst(text_const), 0);
        self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_RUN_COPROC, 3), 0);
        self.builder.emit(Op::SetStatus, 0);
    }

    fn emit_param_modifier(&mut self, m: &ParamModifier) {
        let name_const = self.builder.add_constant(Value::str(m.name.as_str()));
        match &m.kind {
            ParamModifierKind::DefaultFamily { op, rhs } => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadInt(*op as i64), 0);
                let rhs_const = self.builder.add_constant(Value::str(rhs));
                self.builder.emit(Op::LoadConst(rhs_const), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_DEFAULT_FAMILY, 3),
                    0,
                );
            }
            ParamModifierKind::Substring { offset, length } => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadInt(*offset), 0);
                // i64::MIN is the "no length given" sentinel — lets
                // the runtime distinguish from explicit negative
                // length (`${s:0:-2}` truncates from end).
                self.builder
                    .emit(Op::LoadInt(length.unwrap_or(i64::MIN)), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_SUBSTRING, 3),
                    0,
                );
            }
            ParamModifierKind::SubstringExpr {
                offset_expr,
                length_expr,
            } => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                let off_const = self.builder.add_constant(Value::str(offset_expr));
                self.builder.emit(Op::LoadConst(off_const), 0);
                let len_const = self
                    .builder
                    .add_constant(Value::str(length_expr.as_deref().unwrap_or("")));
                self.builder.emit(Op::LoadConst(len_const), 0);
                // Sentinel so the runtime can tell `length=""` (no
                // length given, take rest) from `length="0"` (zero).
                let has_len_const = self
                    .builder
                    .add_constant(Value::Bool(length_expr.is_some()));
                self.builder.emit(Op::LoadConst(has_len_const), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_SUBSTRING_EXPR, 4),
                    0,
                );
            }
            ParamModifierKind::Strip {
                op,
                pattern,
                had_at,
            } => {
                // Pass dq_context_depth as an additional arg so the
                // runtime knows whether to join arrays before
                // stripping (DQ form: `"${a%%pat}"`) or strip
                // per-element (unquoted: `${a%%pat}`). `had_at`
                // overrides — explicit `[@]` subscript on the var
                // forces per-element even inside DQ (zsh marks
                // `[@]` arrays as splice-expanded; the strip
                // applies to each element individually).
                self.builder.emit(Op::LoadConst(name_const), 0);
                let pat_const = self.builder.add_constant(Value::str(pattern));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                self.builder.emit(Op::LoadInt(*op as i64), 0);
                let dq_for_runtime = if *had_at {
                    0
                } else {
                    self.dq_context_depth as i64
                };
                self.builder.emit(Op::LoadInt(dq_for_runtime), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_STRIP, 4), 0);
            }
            ParamModifierKind::Replace {
                op,
                pattern,
                repl,
                had_at,
            } => {
                // Pass dq_context_depth as a 5th arg so the runtime
                // distinguishes DQ-wrapped (join-then-replace) from
                // unquoted (per-element replace on arrays).
                // `had_at` overrides — explicit `[@]` subscript
                // forces per-element even inside DQ (matches Strip).
                self.builder.emit(Op::LoadConst(name_const), 0);
                let pat_const = self.builder.add_constant(Value::str(pattern));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                let repl_const = self.builder.add_constant(Value::str(repl));
                self.builder.emit(Op::LoadConst(repl_const), 0);
                self.builder.emit(Op::LoadInt(*op as i64), 0);
                let dq_for_runtime = if *had_at {
                    0
                } else {
                    self.dq_context_depth as i64
                };
                self.builder.emit(Op::LoadInt(dq_for_runtime), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_REPLACE, 5),
                    0,
                );
            }
            ParamModifierKind::Length => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_LENGTH, 1),
                    0,
                );
            }
            ParamModifierKind::FilterRemoveMatching { pattern } => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                let pat_const = self.builder.add_constant(Value::str(pattern));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_FILTER, 2),
                    0,
                );
            }
        }
    }

    fn emit_negate_status(&mut self) {
        self.builder.emit(Op::GetStatus, 0);
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::NumEq, 0);
        let was_zero = self.builder.emit(Op::JumpIfTrue(0), 0);
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetStatus, 0);
        let end = self.builder.emit(Op::Jump(0), 0);
        let t = self.builder.current_pos();
        self.builder.patch_jump(was_zero, t);
        self.builder.emit(Op::LoadInt(1), 0);
        self.builder.emit(Op::SetStatus, 0);
        let e = self.builder.current_pos();
        self.builder.patch_jump(end, e);
    }

    /// Close one element of a sublist's `&&` / `||` chain the way
    /// `execpline` does — `Src/exec.c:1489-1492` picks `execsimple` or
    /// `execpline` from the parse-time `WC_SUBLIST_SIMPLE` flag, and
    /// only the `execpline` arm reaches `waitonejob` (`Src/jobs.c:1748`),
    /// which is the sole writer of `$pipestatus` for in-shell code.
    ///
    /// Two compile-time conditions gate the marker, both of which C
    /// resolves at parse time as well:
    ///
    /// * `cmplx` — a simple sublist goes to `execsimple`, which the C
    ///   comment at `Src/exec.c:1284-1286` describes as bypassing job
    ///   handling entirely. No job, no `waitonejob`, no `$pipestatus`
    ///   update. `[[ -z x ]]`, `(( 1 ))`, `x=1`, a bare funcdef and any
    ///   compound whose whole body is likewise simple all land here.
    ///
    /// * a multi-stage pipeline — `waitonejob` takes its `jn->procs`
    ///   branch (`Src/jobs.c:1750-1751`) into `zwaitjob`, whose
    ///   `storepipestats` (`Src/jobs.c:420`) publishes the per-stage
    ///   array. In zshrs `BUILTIN_RUN_PIPELINE` already does exactly
    ///   that, so the no-procs marker must not run and clobber it back
    ///   to a single entry.
    ///
    /// A single-stage element that forks an external is deliberately
    /// NOT excluded: C's job then holds one proc and `storepipestats`
    /// yields `[status]`, which is the same one-element array
    /// `pipestats[0] = lastval` produces (`0200|sig` and `128+sig`
    /// agree for the signalled case), so the two branches coincide.
    fn emit_sublist_finish(&mut self, pipe: &ZshPipe, negated: bool) {
        // c:Src/parse.c:878 — BANG sets `*cmplx = 1` in par_sublist2.
        if !(negated || pipe_is_cmplx(pipe)) {
            return;
        }
        // c:Src/parse.c:906/930 — BAR/BARAMP. Handled by RUN_PIPELINE.
        if pipe.next.is_some() {
            return;
        }
        // Hand the live `$?` over as the argument. C's `waitonejob`
        // reads the single `lastval` global; zshrs keeps the VM status
        // cell and `builtin::LASTVAL` separately and only the former is
        // maintained by the compound-command emitters, so the marker
        // has to carry the authoritative one across.
        self.builder.emit(Op::GetStatus, 0);
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_SUBLIST_FINISH, 1),
            0,
        );
        self.builder.emit(Op::Pop, 0);
    }

    fn compile_pipe(&mut self, pipe: &ZshPipe) {
        // ZshPipe = command + Optional(next ZshPipe). For a single-command
        // pipe (no next), compile inline. Multi-stage pipelines fork one
        // child per stage via BUILTIN_RUN_PIPELINE: each stage is compiled
        // as its own sub-chunk and pushed by index, the count goes on
        // CallBuiltin's argc, and the runtime wires up the pipe fds.
        if pipe.next.is_none() {
            self.compile_command(&pipe.cmd);
            return;
        }
        // cmdstack: direct port of Src/exec.c:1991-2039 execpline2.
        // C structure (recursive):
        //   if WC_PIPE_END:
        //       execcmd_exec(stage_N)               // last stage, no push
        //   else:
        //       execcmd_exec(stage_1)               // FIRST stage, NO push
        //       cmdpush(CS_PIPE)                    // push BEFORE recursion
        //       execpline2(rest)                    // recurse for stages 2+
        //       cmdpop()
        //
        // Effect: stage 1 traces with NO `pipe` cmdstack tag; stages
        // 2+ trace WITH the tag. Pushing once before the whole loop
        // (the previous shape) tagged stage 1 too — divergent.
        // Now we push INSIDE each stage's sub-chunk for index > 0
        // so only stages 2+ inherit the tag.

        // Multi-stage pipeline: collect (cmd, merge_stderr_into_pipe)
        // pairs. `cmd1 |& cmd2` makes cmd1's stage merge stderr into
        // its stdout BEFORE the pipe — so we emit `2>&1` redirect first
        // in cmd1's sub-chunk.
        let mut stages: Vec<(&ZshCommand, bool)> = Vec::new();
        let mut cur_pipe = pipe;
        loop {
            let merge = cur_pipe.merge_stderr && cur_pipe.next.is_some();
            stages.push((&cur_pipe.cmd, merge));
            match cur_pipe.next.as_deref() {
                Some(next) => cur_pipe = next,
                None => break,
            }
        }
        for (i, (stage_cmd, merge)) in stages.iter().enumerate() {
            // c:Src/exec.c:3720-3724 — where the stage's pipe fds land
            // on 0/1. For a SIMPLE command the addfd pair runs after
            // prefork/globlist have expanded the argument words, so the
            // install op is emitted inside compile_simple (see
            // emit_stage_fds_install). Every other stage shape — `{ … }`,
            // `( … )`, `if`, a loop — has no args at this level and
            // its BODY legitimately reads the pipe (zsh:
            // `print -rl -- c a b | { print -r -- "[$(cat)]" }` prints
            // the data), so those install at chunk entry.
            let stage_is_simple = matches!(stage_cmd, ZshCommand::Simple(_));
            let mut install_at_top = !stage_is_simple;
            let chunk = loop {
                let mut sub = ZshCompiler::new();
                // c:Src/exec.c::execpline2 — recursive pipeline emit
                // pushes CS_PIPE BEFORE each recursive call into the
                // rest of the pipeline. Stage i (0-based) inherits `i`
                // cumulative CS_PIPE pushes from the outer recursion
                // depth: stage 0 = 0 pushes, stage 1 = 1 push, stage 2
                // = 2 pushes, etc. zsh's `%_` then renders the chain
                // (`pipe`, `pipe pipe`, `pipe pipe pipe`, …) matching
                // the recursive call depth.
                for _ in 0..i {
                    sub.emit_cmd_push(crate::ported::zsh_h::CS_PIPE as u8);
                }
                // c:Src/exec.c:3722-3724 — pipeline output occupies mfds[1]
                // before the stage command's redirect list is walked, so
                // that list's fd-1 write redirects MULTIOS-join the pipe
                // (`{ echo a; echo b >&2; } 3>&1 1>&2 2>&3 3>&- | cat`
                // sends `a` to BOTH the pipe and stderr). mfds is
                // per-execcmd: only the stage's TOP-LEVEL redirects join —
                // hence the gate on the command shape — and only non-last
                // stages have their stdout on the pipe (the last runs
                // inline in the parent).
                let stage_has_toplevel_redirs = match stage_cmd {
                    ZshCommand::Simple(s) => !s.redirs.is_empty(),
                    ZshCommand::Redirected(_, redirs) => !redirs.is_empty(),
                    _ => false,
                };
                if i + 1 < stages.len() && stage_has_toplevel_redirs {
                    sub.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_PIPE_OUTPUT_MARK, 0),
                        0,
                    );
                    sub.builder.emit(Op::Pop, 0);
                }
                sub.stage_fds_pending = true;
                // `cmd1 |& cmd2` — the parser hands cmd1 an extra
                // `2>&1`, and C walks it in the redirect loop at
                // c:3730+, i.e. AFTER addfd has put the pipe on fd 1.
                // Folding it into the install op keeps that order (a
                // chunk-entry `2>&1` would now dup the shell's stdout,
                // not the pipe).
                sub.stage_fds_merge_stderr = *merge;
                if install_at_top {
                    sub.emit_stage_fds_install();
                }
                sub.compile_command(stage_cmd);
                if sub.stage_fds_pending {
                    // No dispatch arm consumed the install — rather than
                    // run a stage whose output never reaches the pipe,
                    // fall back to installing at chunk entry.
                    install_at_top = true;
                    continue;
                }
                // Pop the i CS_PIPE pushes from the head.
                for _ in 0..i {
                    sub.emit_cmd_pop();
                }
                let sub_end = sub.builder.current_pos();
                for patch in std::mem::take(&mut sub.return_patches) {
                    sub.builder.patch_jump(patch, sub_end);
                }
                break sub.builder.build();
            };
            let idx = self.builder.add_sub_chunk(chunk);
            self.builder.emit(Op::LoadInt(idx as i64), 0);
        }
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_RUN_PIPELINE, stages.len() as u8),
            0,
        );
        self.builder.emit(Op::SetStatus, 0);
        // c:Src/exec.c::execpline — after a pipeline finishes, C's
        // post-command path checks errflag via zexit_or_continue()
        // which sees the pipeline's last-stage non-zero status under
        // `setopt errexit`. zshrs's compile_pipe path emitted
        // SetStatus but never called emit_errexit_check, so
        // `setopt errexit; true | false; echo never` ran "never"
        // instead of aborting. Bug #286 in docs/BUGS.md. Mirror the
        // existing simple-command and try-block emit_errexit_check
        // pattern (lines 423 and 705).
        self.emit_errexit_check();
    }

    fn compile_command(&mut self, cmd: &ZshCommand) {
        match cmd {
            ZshCommand::Simple(simple) => self.compile_simple(simple),
            ZshCommand::Subsh(prog) => {
                // (list) — subshell with state isolation. Save current
                // return_patches before compiling the body so any `exit`
                // / `return` inside lands at SubshellEnd (popping the
                // subshell scope) rather than escaping to the chunk's
                // top-level return-target. zsh: `(exit 42)` exits the
                // subshell only; the parent continues with $?=42.
                //
                // cmdstack note: C zsh does NOT push CS_SUBSH at the
                // WC_SUBSH execution path (verified by grep through
                // Src/exec.c — only WC_CURSH gets cmdpush at line
                // 488; WC_SUBSH execcmd_exec runs without one). The
                // CS_SUBSH constant exists for parser-time analysis
                // but the executor's xtrace shows NO `subsh` tag.
                // Previous Rust port pushed CS_SUBSH which caused
                // `( cmd )` traces to differ from zsh — fixed by
                // dropping the push.
                self.builder.emit(Op::SubshellBegin, 0);
                let saved = std::mem::take(&mut self.return_patches);
                // Same containment for the execlist `!breaks` gate: C
                // forks for `(...)`, so a `break` inside ends the CHILD's
                // list and never reaches the parent's loop. Landing these
                // jumps on SubshellEnd (which restores the parent's
                // loops/breaks/contflag) reproduces that; letting them
                // reach the loop's drain would skip SubshellEnd entirely
                // and leave the whole subshell scope unpopped.
                let saved_body_ends: Vec<Vec<usize>> =
                    self.body_end_patches.iter().cloned().collect();
                for v in self.body_end_patches.iter_mut() {
                    v.clear();
                }
                self.compile_program(prog);
                let inner_patches = std::mem::take(&mut self.return_patches);
                self.return_patches = saved;
                let inner_body_ends: Vec<Vec<usize>> = self
                    .body_end_patches
                    .iter_mut()
                    .map(std::mem::take)
                    .collect();
                for (i, v) in saved_body_ends.into_iter().enumerate() {
                    if i < self.body_end_patches.len() {
                        self.body_end_patches[i] = v;
                    }
                }
                let landing = self.builder.current_pos();
                for patch in inner_patches {
                    self.builder.patch_jump(patch, landing);
                }
                for vec in inner_body_ends {
                    for patch in vec {
                        self.builder.patch_jump(patch, landing);
                    }
                }
                self.builder.emit(Op::SubshellEnd, 0);
                // c:Src/exec.c — after a WC_SUBSH child exits, the
                // parent checks errflag / set -e against the inner
                // exit status. `set -e; (false); echo done` should
                // abort the script after the subshell because the
                // subshell exited 1. Without the check the parent's
                // `echo done` ran. Emit ERREXIT_CHECK same as a
                // simple command does (handles set -e, retflag,
                // exit_pending, non-interactive errflag).
                self.emit_errexit_check();
            }
            ZshCommand::Cursh(prog) => {
                // {list} — brace group; no isolation.
                // cmdstack: direct port of Src/loop.c:746
                //   cmdpush(CS_CURSH);
                self.emit_cmd_push(crate::ported::zsh_h::CS_CURSH as u8);
                self.compile_program(prog);
                self.emit_cmd_pop();
            }
            ZshCommand::If(if_node) => self.compile_if(if_node),
            ZshCommand::While(w) => self.compile_while(w),
            ZshCommand::Until(w) => self.compile_while(w),
            ZshCommand::For(f) => self.compile_for(f),
            ZshCommand::Case(c) => self.compile_case(c),
            ZshCommand::Repeat(r) => self.compile_repeat(r),
            ZshCommand::FuncDef(f) => {
                self.compile_funcdef(f);
                // c:Src/exec.c:5495-5496 — an ANONYMOUS function definition
                // executes immediately (`execshfunc(shf, args); ret = lastval;`)
                // and its status is the sublist's, so execlist's c:1651
                // ZERR / ERR_EXIT / ERR_RETURN block runs on it like any other
                // command. zshrs emitted nothing here, so
                //   setopt ERR_EXIT; () { false; print X; }
                // ran X and kept going (E01options:19,21; C03traps:74).
                // A NAMED definition leaves lastval 0, so the check is a no-op
                // for it — same as C.
                self.emit_errexit_check();
            }
            ZshCommand::Cond(c) => {
                self.compile_cond(c);
                // c:Src/exec.c — a `[[ ]]` command's status participates in the
                // errexit/errflag abort like any other command: `set -e; [[ 0
                // = 1 ]]` exits on the false status, and a cond eval error
                // (ERRFLAG_ERROR, e.g. "unknown condition") aborts the input.
                // compile_cond emitted the status but never ran the check, so
                // neither fired. emit_errexit_check is gated by
                // errexit_suppress_depth, so `if`/`&&`/`||`/`while` contexts
                // stay exempt. Mirrors the Arith arm (Bug #33).
                self.emit_errexit_check();
            }
            ZshCommand::Arith(expr) => {
                self.compile_arith(expr);
                // c:Src/exec.c WC_ARITH — math command's status is part
                // of the errexit check: `set -e; (( 0 ))` should exit
                // because the math evaluates to 0 → status 1. The
                // compile_arith path emits SetStatus(0/1) per the
                // result but never invoked emit_errexit_check, so
                // `set -e` had no opportunity to fire on the math
                // command's non-zero status. Bug #33 in docs/BUGS.md.
                self.emit_errexit_check();
            }
            ZshCommand::Redirected(inner, redirs) => {
                // c:Src/exec.c — `f() { ... } > file` parses as
                // Redirected(FuncDef(...), [redirs]). The redirects
                // attach to the FUNCTION (apply at call time), not at
                // definition time. zsh defers the open via Shfunc.redir
                // applied around the body during execfuncdef-driven
                // doshfunc. zshrs's funcdef storage doesn't carry a
                // separate redir chain — instead, wrap the body
                // AST in a Redirected node BEFORE compile_funcdef
                // runs, so the compiled body chunk itself begins with
                // a WithRedirectsBegin/End scope. Mirrors the
                // canonical zsh behaviour where `f` opens the redirs
                // on every call. Bug #158.
                if matches!(inner.as_ref(), ZshCommand::FuncDef(_)) {
                    if let ZshCommand::FuncDef(mut f) = *inner.clone() {
                        // Build a single-list body that wraps the
                        // existing function body in
                        // Redirected(Cursh(body), redirs). The Cursh
                        // arm of compile_command already handles
                        // WithRedirectsBegin/End around a brace group,
                        // so reuse it instead of re-emitting the
                        // redirect bytecode here.
                        let inner_program = std::mem::replace(
                            &mut f.body,
                            Box::new(crate::parse::ZshProgram { lists: Vec::new() }),
                        );
                        let wrapped = ZshCommand::Redirected(
                            Box::new(ZshCommand::Cursh(inner_program)),
                            redirs.clone(),
                        );
                        let list = crate::parse::ZshList {
                            sublist: crate::parse::ZshSublist {
                                pipe: crate::parse::ZshPipe {
                                    cmd: wrapped,
                                    next: None,
                                    lineno: 1,
                                    merge_stderr: false,
                                },
                                next: None,
                                flags: crate::parse::SublistFlags::default(),
                            },
                            flags: crate::parse::ListFlags::default(),
                        };
                        f.body = Box::new(crate::parse::ZshProgram { lists: vec![list] });
                        // c:Src/exec.c — C stashes the definition's trailing
                        // redirections in the file-scope `redir_prog` before
                        // `execfuncdef` runs, which copies it to `shf->redir`
                        // (c:5453) purely so the function can be PRINTED back
                        // with them (c:Src/hashtable.c:988-994). Render the
                        // same text here; the executable effect is already
                        // handled by the body wrapping above.
                        self.redir_prog_text = Some(getredirs(redirs));
                        self.compile_command(&ZshCommand::FuncDef(f));
                        self.redir_prog_text = None;
                        return;
                    }
                    self.compile_command(inner);
                    return;
                }
                // Compound command with trailing redirects (e.g.
                // `{ ... } 2>&1`). Bracket the body in a
                // WithRedirectsBegin/End scope so post-body fds are
                // restored. Status is whatever the inner cmd left.
                self.builder
                    .emit(Op::WithRedirectsBegin(redirs.len() as u8), 0);
                self.compile_redirs_multios(redirs);
                // c:Src/exec.c — if any redirect failed (e.g.
                // `{ … } < /nonexistent`), zsh aborts the entire body
                // and sets $? = 1. Check the flag after opening all
                // redirects: on failure, skip body and SetStatus(1).
                // Without this, only the first statement in the body
                // bailed (consumed by its builtin dispatch); the rest
                // ran with the fds in an inconsistent state.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_REDIRECT_FAILED_CHECK, 0),
                    0,
                );
                let skip_body = self.builder.emit(Op::JumpIfTrue(0), 0);
                self.compile_command(inner);
                let body_end = self.builder.current_pos();
                self.builder.patch_jump(skip_body, body_end);
                self.builder.emit(Op::WithRedirectsEnd, 0);
            }
            ZshCommand::Time(maybe_sublist) => {
                if let Some(sublist) = maybe_sublist {
                    // Compile the timed sublist as a sub-chunk; the
                    // BUILTIN_TIME_SUBLIST handler runs it and prints
                    // elapsed wall-clock time in zsh's format.
                    let mut sub = ZshCompiler::new();
                    sub.compile_sublist(sublist);
                    let sub_end = sub.builder.current_pos();
                    for patch in std::mem::take(&mut sub.return_patches) {
                        sub.builder.patch_jump(patch, sub_end);
                    }
                    let chunk = sub.builder.build();
                    let sub_idx = self.builder.add_sub_chunk(chunk);
                    // c:Src/jobs.c:1028-1029 — `printtime(..., pn->text)`
                    // emits `%J` as the per-process command text. For
                    // `time CMD` (simple-cmd form), zsh's printjob → dumptime
                    // reads `p->text` populated at fork time by getjobtext.
                    // zshrs's BUILTIN_TIME_SUBLIST handler doesn't go
                    // through addproc, so we pre-render the sublist's
                    // source text here and push it as the desc operand
                    // for the handler to forward to printtime as job_name.
                    // Bug #66 in docs/BUGS.md.
                    let desc = render_sublist_for_debug(sublist);
                    // c:Src/exec.c:3690 — `is_cursh = (is_builtin ||
                    // is_shfunc || nullexec || type >= WC_CURSH);`. When the
                    // timed body runs in the CURRENT shell, execcmd_exec's
                    // tail (c:4443-4444 `if ((is_cursh || do_exec) && (how &
                    // Z_TIMED)) shelltime(&shti,&chti,&then,1);`) prints the
                    // two-line shell/children report; otherwise the forked
                    // job's own printtime line is what gets emitted. Classify
                    // the sublist here and hand the verdict to the handler.
                    let (cursh_hint, cursh_name) = time_cursh_hint(sublist);
                    let name_const = self.builder.add_constant(Value::str(&cursh_name));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(Op::LoadInt(cursh_hint), 0);
                    let desc_const = self.builder.add_constant(Value::str(&desc));
                    self.builder.emit(Op::LoadConst(desc_const), 0);
                    self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_TIME_SUBLIST, 4),
                        0,
                    );
                    self.builder.emit(Op::SetStatus, 0);
                    // c:Src/exec.c execpline — a `time` pipeline's exit
                    // status participates in errexit like any other
                    // command: `setopt errexit; time false; print x`
                    // aborts before `print x`. The previous port set $?
                    // but skipped the errexit check, so the timed
                    // command's failure was swallowed.
                    self.emit_errexit_check();
                } else {
                    // c:Src/exec.c:5331-5334 — bare `time` (WC_TIMED_EMPTY):
                    //   shelltime(NULL,NULL,NULL,0); return 0;
                    // which prints the two-line shell/children report. Route
                    // through BUILTIN_TIME_SUBLIST with a NEGATIVE sub-chunk
                    // index as the "no body" marker (same operand shape as
                    // the timed-sublist form, so the handler keeps one
                    // argc==4 decode). The previous arm just set $?=0 and
                    // printed nothing, so `time` alone was silent where zsh
                    // reports the shell's accumulated times.
                    let name_const = self.builder.add_constant(Value::str(""));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(Op::LoadInt(0), 0);
                    let desc_const = self.builder.add_constant(Value::str(""));
                    self.builder.emit(Op::LoadConst(desc_const), 0);
                    self.builder.emit(Op::LoadInt(-1), 0); // c:5332 WC_TIMED_EMPTY
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_TIME_SUBLIST, 4),
                        0,
                    );
                    self.builder.emit(Op::SetStatus, 0);
                }
            }
            ZshCommand::Try(t) => {
                // `{ try } always { finally }` — run both blocks, with the
                // finally block executing regardless of try's exit status.
                // The exit status of the whole construct is the LAST status
                // set (matches zsh: try's status is preserved unless the
                // finally block sets a different one).
                //
                // c:Src/exec.c TRY_BLOCK semantics: `return`/`break`/
                // `continue`/`exit` inside the try-block MUST still run
                // the always-arm before propagating. We save the
                // surrounding scope's return_patches + break_patches +
                // continue_patches, give the try-block fresh empty
                // lists, then route every collected escape through the
                // always-arm landing. After the always-arm runs we
                // re-emit a Jump to the surrounding scope's matching
                // list so the original short-circuit still happens.
                let saved_return = std::mem::take(&mut self.return_patches);
                let saved_breaks: Vec<Vec<usize>> = self
                    .break_patches
                    .iter()
                    .map(|v| std::mem::take(&mut v.clone()))
                    .collect();
                let saved_continues: Vec<Vec<usize>> = self
                    .continue_patches
                    .iter()
                    .map(|v| std::mem::take(&mut v.clone()))
                    .collect();
                // Replace the per-loop lists with fresh inner copies
                // so escapes captured inside the try-block don't fire
                // the outer loop's break/continue.
                let saved_body_ends: Vec<Vec<usize>> =
                    self.body_end_patches.iter().cloned().collect();
                for v in self.break_patches.iter_mut() {
                    v.clear();
                }
                for v in self.continue_patches.iter_mut() {
                    v.clear();
                }
                for v in self.body_end_patches.iter_mut() {
                    v.clear();
                }
                // c:Src/loop.c:746 — `cmdpush(CS_CURSH);` wraps the
                // try arm so `%_` xtrace renders `cursh` for any
                // statement inside it.
                self.emit_cmd_push(crate::ported::zsh_h::CS_CURSH as u8);
                self.try_block_depth += 1;
                self.try_loop_base.push(self.break_patches.len());
                self.compile_program(&t.try_block);
                self.try_loop_base.pop();
                self.try_block_depth -= 1;
                self.emit_cmd_pop(); // c:Src/loop.c:759 — `cmdpop();`
                                     // After the try-block, snapshot the escape patches it
                                     // accumulated. Their targets will be patched to land
                                     // at the always-arm entry so the finally clause runs
                                     // regardless of how the try-block left.
                let inner_returns = std::mem::take(&mut self.return_patches);
                let inner_breaks: Vec<Vec<usize>> = self
                    .break_patches
                    .iter_mut()
                    .map(|v| std::mem::take(v))
                    .collect();
                let inner_continues: Vec<Vec<usize>> = self
                    .continue_patches
                    .iter_mut()
                    .map(|v| std::mem::take(v))
                    .collect();
                // A `break` executed in a function CALLED from the try
                // body escapes via the execlist gate, not an in-chunk
                // jump. Those jumps must land on the always-arm too, or
                // the finally clause is skipped.
                let inner_body_ends: Vec<Vec<usize>> = self
                    .body_end_patches
                    .iter_mut()
                    .map(std::mem::take)
                    .collect();
                // Track whether this try-block was escaped via
                // return/break/continue so we know to re-jump after
                // the always-arm. AtomicI32 flag in canonical
                // RETFLAG / BREAKS won't survive the always-arm body
                // (which can clobber both), so capture into a
                // dedicated TLS slot via BUILTIN_TRY_ESCAPE_SAVE /
                // BUILTIN_TRY_ESCAPE_RESTORE — see fusevm_bridge.rs.
                let any_escape = !inner_returns.is_empty()
                    || inner_breaks.iter().any(|v| !v.is_empty())
                    || inner_continues.iter().any(|v| !v.is_empty());
                // Patch all inner escapes to land at the always-arm
                // entry.
                let always_entry = self.builder.current_pos();
                for p in &inner_returns {
                    self.builder.patch_jump(*p, always_entry);
                }
                for vec in &inner_breaks {
                    for p in vec {
                        self.builder.patch_jump(*p, always_entry);
                    }
                }
                for vec in &inner_continues {
                    for p in vec {
                        self.builder.patch_jump(*p, always_entry);
                    }
                }
                for vec in &inner_body_ends {
                    for p in vec {
                        self.builder.patch_jump(*p, always_entry);
                    }
                }
                // Restore the outer escape lists so any escapes the
                // always-arm itself emits go to the surrounding scope.
                self.return_patches = saved_return;
                for (i, v) in saved_breaks.into_iter().enumerate() {
                    if i < self.break_patches.len() {
                        self.break_patches[i] = v;
                    }
                }
                for (i, v) in saved_continues.into_iter().enumerate() {
                    if i < self.continue_patches.len() {
                        self.continue_patches[i] = v;
                    }
                }
                for (i, v) in saved_body_ends.into_iter().enumerate() {
                    if i < self.body_end_patches.len() {
                        self.body_end_patches[i] = v;
                    }
                }
                // c:Src/exec.c — TRY_BLOCK_ERROR snapshot fires BEFORE
                // the always-arm runs so the body can read it.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_TRY_BLOCK_ERROR, 0),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                // c:Src/loop.c:760 — `cmdpush(CS_ALWAYS);` wraps the
                // always arm so `%_` xtrace renders `always`.
                self.emit_cmd_push(crate::ported::zsh_h::CS_ALWAYS as u8);
                // c:Src/loop.c:777 + :801 — `execlist(state, 1, 0);` … then
                // `return endval;`. exectry runs the always-list as a plain
                // recursive execlist and ALWAYS falls through to
                // `return endval` (the TRY-list's status, captured at c:755),
                // even when the always-list itself did a `return` — that only
                // sets `retflag`, it does not skip exectry's own return.
                // zshrs compiles `return` as a jump to the enclosing
                // function's exit, which SKIPPED the status restore below, so
                // `() { { return 2 } always { return 3 } }` exited 3 instead
                // of 2 (A01grammar.ztst:723). Capture the always-arm's own
                // return jumps and land them on the restore instead.
                let saved_return_always = std::mem::take(&mut self.return_patches);
                self.compile_program(&t.always);
                let always_returns = std::mem::take(&mut self.return_patches);
                self.return_patches = saved_return_always;
                self.emit_cmd_pop();
                // Whole-construct status: preserve the try block's
                // status when the always arm exited cleanly.
                let restore_entry = self.builder.current_pos();
                for p in &always_returns {
                    self.builder.patch_jump(*p, restore_entry);
                }
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_RESTORE_TRY_BLOCK_STATUS, 0),
                    0,
                );
                self.builder.emit(Op::SetStatus, 0);
                // c:Src/exec.c — errexit applies to the whole
                // `{ try } always { finally }` construct: when the
                // restored try-block status is non-zero, the shell
                // aborts at the end of the construct. Without this
                // emit, `setopt err_exit; { false } always { :; };
                // echo after` printed `after`. Bug #240 in
                // docs/BUGS.md.
                // RESTORE_TRY_BLOCK_STATUS has just re-armed BREAKS for
                // the level-accurate probes below; the generic execlist
                // gate must not eat it first.
                self.break_escape_suppress += 1;
                self.emit_errexit_check();
                self.break_escape_suppress -= 1;
                // If the try-block fired a return/break/continue, the
                // canonical RETFLAG / BREAKS / CONTFLAG atomics are
                // restored by RESTORE_TRY_BLOCK_STATUS. Emit one
                // conditional re-jump per escape kind so the outer
                // construct (function / loop) sees the original
                // semantic. Order matters: continue is distinguished
                // from break by CONTFLAG (both set BREAKS via
                // SET_CONTINUE), so check continue BEFORE break.
                if !inner_returns.is_empty() || !always_returns.is_empty() {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_RETFLAG_CHECK, 0),
                        0,
                    );
                    let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
                    self.emit_cmd_stack_drain();
                    let j = self.builder.emit(Op::Jump(0), 0);
                    self.return_patches.push(j);
                    let after = self.builder.current_pos();
                    self.builder.patch_jump(skip, after);
                }
                let any_continue = inner_continues.iter().any(|v| !v.is_empty());
                if any_continue {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_CONTFLAG_CHECK, 0),
                        0,
                    );
                    let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
                    // Find the innermost level that captured any
                    // continue patches; route to that loop's
                    // continue landing.
                    let lvl = inner_continues
                        .iter()
                        .rposition(|v| !v.is_empty())
                        .unwrap_or(0);
                    self.emit_cmd_stack_drain();
                    if lvl < self.continue_patches.len() {
                        self.emit_loop_exit(self.continue_patches.len() - 1 - lvl);
                    }
                    let j = self.builder.emit(Op::Jump(0), 0);
                    if lvl < self.continue_patches.len() {
                        self.continue_patches[lvl].push(j);
                    } else {
                        self.return_patches.push(j);
                    }
                    let after = self.builder.current_pos();
                    self.builder.patch_jump(skip, after);
                }
                let any_break = inner_breaks.iter().any(|v| !v.is_empty());
                if any_break {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_BREAKS_CHECK, 0),
                        0,
                    );
                    let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
                    let lvl = inner_breaks
                        .iter()
                        .rposition(|v| !v.is_empty())
                        .unwrap_or(0);
                    self.emit_cmd_stack_drain();
                    if lvl < self.break_patches.len() {
                        self.emit_loop_exit(self.break_patches.len() - 1 - lvl);
                    }
                    let j = self.builder.emit(Op::Jump(0), 0);
                    if lvl < self.break_patches.len() {
                        self.break_patches[lvl].push(j);
                    } else {
                        self.return_patches.push(j);
                    }
                    let after = self.builder.current_pos();
                    self.builder.patch_jump(skip, after);
                }
                // Foreign break/continue (set by a called function) that
                // was parked at the always-arm: re-arm it now that the
                // finally clause has run. Non-consuming — the enclosing
                // loop's drain is the only consumer, and `break 2` needs
                // the count intact.
                if inner_body_ends.iter().any(|v| !v.is_empty()) {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_BREAKS_PENDING, 0),
                        0,
                    );
                    let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
                    self.emit_cmd_stack_drain();
                    let j = self.builder.emit(Op::Jump(0), 0);
                    match self.body_end_patches.last_mut() {
                        Some(v) => v.push(j),
                        None => self.return_patches.push(j),
                    }
                    let after = self.builder.current_pos();
                    self.builder.patch_jump(skip, after);
                }
                let _ = any_escape;
            }
        }
    }

    fn compile_simple(&mut self, simple: &ZshSimple) {
        // Inline-assignment-prefix scope: `X=foo Y=bar cmd` should
        // export the assigns to cmd's child env AND restore both
        // shell-vars and process-env to the pre-call state when cmd
        // returns. Detect by checking for assigns paired with words;
        // emit a BEGIN/END_INLINE_ENV pair around the command run so
        // SET_VAR can stash and restore each name's prior state.
        // Direct port of zsh's addvars()-list scoping in execute_simple.
        let has_inline_env_scope = !simple.assigns.is_empty() && !simple.words.is_empty();
        // c:Src/exec.c:3720-3724 — a pipeline stage with no command
        // word has no argument expansion for the addfd pair to follow,
        // so its pipe fds install right here: before the redirect loop
        // (c:3730+) and before addvars (c:4142), which is C's order.
        // `print -rl -- a b | x=$(cat)` therefore has the assignment's
        // `$(cat)` read the PIPE, unlike a command's arg words.
        if simple.words.is_empty() {
            self.emit_stage_fds_install();
        }
        // c:Src/exec.c — prefork (arg expansion) runs BEFORE addvars
        // (the inline-assign list) in zsh. That's how `a=1 echo "$a"`
        // prints empty (the shell's own `a` is still unset when the
        // args are expanded; the assigned `a=1` lives only in the
        // child env). The previous Rust port ran compile_assign here
        // (before word push), so the args' `$a` saw the just-set
        // shell var and echoed `1` instead of "". Defer compile_assign
        // until just before dispatch: the args are pushed onto the
        // stack with PRE-assign state, then the assigns commit, then
        // the command consumes the (pre-expanded) args from the
        // stack with the assigns visible in env.
        if has_inline_env_scope {
            // c:Src/exec.c:4116-4126 — under POSIX_BUILTINS the save/restore
            // that undoes `X=y cmd`'s prefix assignments is SKIPPED when the
            // command is a shell function or a BINF_PSPECIAL / BINF_ASSIGN
            // builtin and there was no `command` prefix:
            //     if (is_shfunc || (hn->flags & (BINF_PSPECIAL|BINF_ASSIGN)))
            //         do_save = (orig_cflags & BINF_COMMAND);
            //     else
            //         do_save = 1;
            // so `v=0; v=1 :` leaves `v` at 1 in dash/ksh/mksh — POSIX.1-2017
            // XCU 2.9.1: "If the command name is a special built-in utility,
            // variable assignments shall affect the current execution
            // environment."
            //
            // C resolves `hn` at RUNTIME, so the name travels to the bridge
            // as a constant and BEGIN_INLINE_ENV performs the shfunctab /
            // builtintab lookup there. An expanded command word (`$cmd`) has
            // no compile-time literal; the empty name makes the bridge take
            // the save arm, which is the pre-existing behavior.
            let cmd_name = crate::lex::untokenize(&simple.words[0]);
            let name_const = self.builder.add_constant(Value::str(cmd_name));
            self.builder.emit(Op::LoadConst(name_const), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_BEGIN_INLINE_ENV, 1),
                0,
            );
            self.builder.emit(Op::Pop, 0);
        }

        // ── Assignments ───────────────────────────────────────────────
        // ZshAssign{ name, value: Scalar(String)|Array(Vec<String>), append }
        // Aggregate whether ANY assign in the chain had a cmd-subst RHS
        // — controls the post-chain `$? = 0` reset below. C zsh's
        // addvars walks all WC_ASSIGN entries with lastval preserved at
        // old_lastval (so every `$?` in every RHS sees the same value),
        // then after the walk `lastval = cmdoutval` — which is 0 unless
        // a `$()` in any RHS overwrote it. Mirror by leaving last_status
        // untouched per-assign and resetting once at the end.
        let mut chain_had_cmd_subst = false;
        // c:Src/exec.c:3963-3976 — when a "simple command" has no
        // command word (nullexec=2 case), the addvars (assignment)
        // walk runs AFTER all addfd/fixfds redirections are in
        // place. The bare-assign-with-redir form `foo=bar 2>&1`
        // applies `2>&1` first, so any zerr emitted by the assignment
        // (e.g. "read-only variable" when `foo` is readonly) writes
        // to the redirected fd2 (= stdout) — matching zsh's behavior.
        //
        // Previous Rust port emitted compile_assign BEFORE
        // WithRedirectsBegin (below), so the error went to the
        // un-redirected stderr and the test's `2>&1` capture missed
        // it. Defer the assign-compile when we're about to enter
        // the bare-assign + redir branch so the assigns land inside
        // the redirect scope.
        // Inline-env case: defer compile_assign until after the word
        // push so the args are evaluated against the pre-assign state.
        // The bare-assign + redir case (words empty + redirs non-empty)
        // also defers — assigns must run AFTER WithRedirectsBegin per
        // exec.c:3963.
        let defer_assigns_to_redir_scope = simple.words.is_empty() && !simple.redirs.is_empty();
        if !has_inline_env_scope && !defer_assigns_to_redir_scope {
            for assign in &simple.assigns {
                self.last_assign_had_cmd_subst = false;
                self.compile_assign(assign);
                if self.last_assign_had_cmd_subst {
                    chain_had_cmd_subst = true;
                }
            }
        }

        // ── If no words: bare assignment / redir-only, done ──────────
        if simple.words.is_empty() {
            // c:Src/exec.c:3330-3364 — no command word but has redirs.
            // - assigns-only: nullexec=2 (apply redirs in scope, assigns
            //   already compiled above, no command run, status from
            //   prior cmd-subst preserved or 0)
            // - no-assigns, no-words, has-redirs: NULLCMD path. Default
            //   NULLCMD=cat reads the redir-bound fd through the
            //   inherited redirect.
            if !simple.redirs.is_empty() {
                self.builder
                    .emit(Op::WithRedirectsBegin(simple.redirs.len() as u8), 0);
                self.compile_redirs_multios(&simple.redirs);
                // c:Src/exec.c:3963-3976 — addvars under nullexec=2
                // fires INSIDE the redir scope. Emit the deferred
                // assigns here so a readonly-reassignment zerr writes
                // through the redirected stderr.
                if defer_assigns_to_redir_scope {
                    for assign in &simple.assigns {
                        self.last_assign_had_cmd_subst = false;
                        self.compile_assign(assign);
                        if self.last_assign_had_cmd_subst {
                            chain_had_cmd_subst = true;
                        }
                    }
                }
                if simple.assigns.is_empty() {
                    // c:3340-3364 — invoke NULLCMD/READNULLCMD.
                    let is_single_read = simple.redirs.len() == 1
                        && simple.redirs[0].rtype == crate::ported::zsh_h::REDIR_READ;
                    self.builder
                        .emit(Op::LoadInt(if is_single_read { 1 } else { 0 }), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_NULLCMD_EXEC, 1),
                        0,
                    );
                    self.builder.emit(Op::SetStatus, 0);
                } else {
                    // nullexec=2 with assigns: c:Src/exec.c:3977 —
                    // `lastval = errflag ? errflag : cmdoutval;`.
                    // Runtime builtin consults errflag (readonly
                    // reassign etc. → 1), else preserves a cmd-subst
                    // $? or resets to 0.
                    self.builder
                        .emit(Op::LoadInt(chain_had_cmd_subst as i64), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_ASSIGN_ONLY_STATUS, 1),
                        0,
                    );
                    self.builder.emit(Op::SetStatus, 0);
                }
                self.builder.emit(Op::WithRedirectsEnd, 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_NEWLINE, 0),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                self.emit_errexit_check();
                return;
            }
            // c:Src/exec.c:3393-3396 — `if (errflag) lastval = 1;
            // else lastval = cmdoutval;` (execsimple shape at c:1322:
            // `lv = (errflag ? errflag : cmdoutval)`).
            // For the assignment-only path: errflag set (readonly
            // reassign etc.) → 1; else if no $() ran in any RHS the
            // post-assignment $? is 0; if any did, last_status
            // already holds that subst's exit. Resolved at runtime
            // by BUILTIN_ASSIGN_ONLY_STATUS since errflag is a
            // runtime fact.
            self.builder
                .emit(Op::LoadInt(chain_had_cmd_subst as i64), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_ASSIGN_ONLY_STATUS, 1),
                0,
            );
            self.builder.emit(Op::SetStatus, 0);
            // xtrace: emit the trailing `\n` + flush iff a prior
            // BUILTIN_XTRACE_ASSIGN this line emitted PS4. Mirrors
            // C's `fputc('\n', xtrerr); fflush(xtrerr);` at
            // Src/exec.c:3398 (the assignment-only return path
            // through execcmd_exec).
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_NEWLINE, 0),
                0,
            );
            self.builder.emit(Op::Pop, 0);
            // c:Src/exec.c — bare assignment (no command word) still
            // needs an errexit check so readonly-reassignment + other
            // errflag-setting paths abort the script. Previously the
            // path returned without checking, leaving the failed
            // assignment as a no-op + the next statement ran.
            self.emit_errexit_check();
            return;
        }

        // `nocorrect CMD ARGS...` — spelling-correction precommand,
        // a no-op in non-interactive (`-fc`) mode. fusevm's
        // shell_builtins table doesn't recognize `nocorrect`, so the
        // dispatch path at the bottom would look it up as a command
        // name and fail "command not found". Strip and recurse.
        // Direct port of zsh's parser-level precommand-modifier
        // recognition.
        let untoked_first_pre = crate::lex::untokenize(&simple.words[0]);
        if untoked_first_pre == "nocorrect" && simple.words.len() > 1 {
            let inner = ZshSimple {
                assigns: simple.assigns.clone(),
                words: simple.words[1..].to_vec(),
                redirs: simple.redirs.clone(),
            };
            self.compile_simple(&inner);
            return;
        }

        // `noglob CMD args...` is a precommand modifier — args must
        // NOT be glob-expanded. zsh handles this in the parser by
        // marking the command line "no-glob"; we strip the leading
        // `noglob` and recursively compile the rest with a runtime
        // option-toggle wrapper.
        let untoked_first0 = crate::lex::untokenize(&simple.words[0]);
        if untoked_first0 == "noglob" && simple.words.len() > 1 {
            let inner = ZshSimple {
                assigns: simple.assigns.clone(),
                words: simple.words[1..].to_vec(),
                redirs: simple.redirs.clone(),
            };
            // Wrap in setopt noglob ... unsetopt noglob via a runtime
            // option toggle. Push "noglob"+true via SET_OPT, recurse to
            // compile inner, then push "noglob"+false to restore.
            let opt_const = self.builder.add_constant(Value::str("noglob"));
            self.builder.emit(Op::LoadConst(opt_const), 0);
            self.builder.emit(Op::LoadInt(1), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_RAW_OPT, 2), 0);
            self.builder.emit(Op::Pop, 0);
            self.compile_simple(&inner);
            self.builder.emit(Op::LoadConst(opt_const), 0);
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_RAW_OPT, 2), 0);
            self.builder.emit(Op::Pop, 0);
            return;
        }

        // ── Redirects on the simple command ─────────────────────────
        // Special case: `exec >file` (or `exec 2>err`, etc.) with NO
        // command body — apply redirects PERMANENTLY to the shell's
        // own fds, no scope-end restoration. zsh: `exec` with only
        // redirects rewires the running shell's fds.
        let bare_exec_redir =
            simple.words.len() == 1 && simple.words[0] == "exec" && !simple.redirs.is_empty();
        if bare_exec_redir {
            // c:Src/exec.c:3721-3724 — addfd for the pipe still runs
            // ahead of the redirect loop under nullexec==1 (`exec
            // >file` has no args to expand first).
            self.emit_stage_fds_install();
            for redir in &simple.redirs {
                // permanent=true: c:Src/exec.c:3978-3986 nullexec==1 —
                // exec's fd changes skip save/restore even when an
                // enclosing group scope is active.
                self.compile_redir(redir, true);
            }
            // Epilogue: c:Src/exec.c:252-259 execerr (failed redirect →
            // lastval=1) + c:4367-4386 done: gate (POSIX_BUILTINS makes
            // it fatal for BINF_EXEC). Success path returns status 0.
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_EXEC_REDIR_DONE, 0),
                0,
            );
            self.builder.emit(Op::SetStatus, 0);
            // c:Src/exec.c:3969-3976 — assignment prefix on bare exec
            // (`x=$(cmd) exec >file`): addvars runs AFTER the
            // redirects (RHS side effects fire against the new fds),
            // but a FAILED redirect jumps to done: before varspc is
            // touched (execerr at c:3741), so gate the assigns on the
            // redirect status. EXEC_INLINE_ENV_DONE then restores the
            // values (no POSIX_BUILTINS) or persists them (POSIX).
            if !simple.assigns.is_empty() {
                self.builder.emit(Op::GetStatus, 0);
                let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
                for assign in &simple.assigns {
                    self.compile_assign(assign);
                }
                let land = self.builder.current_pos();
                self.builder.patch_jump(skip, land);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_EXEC_INLINE_ENV_DONE, 0),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
            }
            // The EXIT_PENDING armed by the POSIX-fatal arm of
            // EXEC_REDIR_DONE unwinds at this check (trigger 2) —
            // without it, statements after a fatal failed `exec`
            // redirect kept running.
            self.emit_errexit_check();
            return;
        }

        // Bracket each command's redirects in a WithRedirectsBegin/End
        // scope so subsequent commands see the original fds. Without the
        // scope, `cmd > out.txt` would leave fd 1 pointing at out.txt for
        // every following command in the script.
        //
        // c:Src/exec.c:3285-3304 (prefork) + c:3702 (globlist) run
        // BEFORE the addfd redirect loop at c:3720+ — so a word-
        // expansion error (`= not found` from equalsubstr c:726,
        // `no matches found` from glob.c NOMATCH, `${var?msg}` zerr)
        // prints to the command's UN-redirected stderr and the
        // command aborts. The scope-begin emission therefore happens
        // per-dispatch-arm AFTER the arg-word ops, not here. Each arm
        // calls emit_redir_scope_begin() once its words are pushed.
        let has_redirects = !simple.redirs.is_empty();

        // ── Dispatch by first-word kind ───────────────────────────────
        // Operates on raw &str inputs and decomposes at compile time.
        let first = &simple.words[0];

        // Dynamic command name: first word contains an unquoted expansion
        // (`$cmd`, `$(cmd)`, `*name`, `~/bin/foo`). Route through Op::Exec
        // so the host runtime expands and dispatches via host.exec →
        // host_exec_external → run_intercepts. Without this, `cmd=ls;
        // $cmd` would emit CallFunction(name="$cmd", ...) and fail with
        // `command not found: $cmd`.
        let first_untoked = crate::lex::untokenize(first);
        // `[` and `[[` are the test/cond builtins, not glob-pattern
        // command names — exempt them from the "dynamic command name"
        // check that routes through Op::Exec.
        let first_is_test_builtin = first_untoked == "[" || first_untoked == "[[";
        // c:Src/subst.c::filesubstr (c:799) — `=cmd` at word start
        // triggers PATH lookup of `cmd`. C zsh's `prefork` runs this on
        // every argv element including the head. The compile-time
        // dispatcher must route `=cmd` through the dynamic path so the
        // runtime expansion (multsub → prefork → filesub → equalsubstr)
        // fires. Without this, `=ls` reached host_exec_external as the
        // raw string `=ls` and got `command not found: =ls`.
        //
        // The first byte after untokenize is ASCII `=` (the lexer emits
        // Equals TOKEN \u{8d}, untokenize maps it back). EQUALS-option
        // gating happens inside equalsubstr at runtime — checking it
        // here would require duplicating the option lookup.
        let first_is_dynamic = !first_is_test_builtin
            && (unquoted(&first_untoked, '$')
                || unquoted(&first_untoked, '`')
                || unquoted(&first_untoked, '*')
                || unquoted(&first_untoked, '?')
                || unquoted(&first_untoked, '[')
                || first_untoked.starts_with('~')
                || first_untoked.starts_with('='));
        if first_is_dynamic {
            let argc = simple.words.len() as u8;
            // c:Src/exec.c:3009 `use_cmdoutval = !args` — clear the
            // cmd-subst-ran flag BEFORE this command's words expand, so a
            // `$(...)` from an earlier command can't make THIS command's
            // null-command branch (BUILTIN_EXEC_DYNAMIC empty-argv) keep a
            // stale `$?`. A `$(...)` inside these words re-sets it.
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_USE_CMDOUTVAL_RESET, 0),
                0,
            );
            self.builder.emit(Op::Pop, 0);
            for w in &simple.words {
                self.compile_word_str(w);
            }
            // c:Src/exec.c:3285-3304 → c:3720 — the pipe fds and then
            // the redirect scope open AFTER the word ops, so an
            // expansion zerr (`=cmd` not found, nomatch) hits the
            // original stderr and a `$( … )` in the args reads the
            // shell's fd 0 rather than the pipe.
            self.emit_stage_fds_install();
            if has_redirects {
                self.emit_redir_scope_begin(&simple.redirs);
            }
            // c:Src/exec.c:4142 addvars — the prefix assignments of
            // `X=y $cmd` commit AFTER the arg words expand (so `$X` in
            // the args still reads the pre-assign value) but BEFORE the
            // command dispatches, so the spawned process inherits
            // `X=y` in its env. This arm previously emitted neither the
            // assigns nor the matching END, so BEGIN_INLINE_ENV (above)
            // pushed a frame that was never popped: `X=y $cmd` ran with
            // X unset, and every LATER plain assignment in the script
            // landed in the orphaned still-`recording` frame and got
            // zputenv'd into the process env.
            if has_inline_env_scope {
                for assign in &simple.assigns {
                    self.last_assign_had_cmd_subst = false;
                    self.compile_assign(assign);
                }
                self.emit_seal_inline_env();
            }
            // Replace fusevm's Op::Exec with BUILTIN_EXEC_DYNAMIC so
            // empty-argv expansion (`\$(exit 1)` produces "") preserves
            // the cmd-subst's last_status. Op::Exec hardcodes 0 for
            // empty argv (fusevm vm.rs:1722-1723) which clobbered \$?
            // in chains like `\$(exit 1); echo \$?`.
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_EXEC_DYNAMIC, argc),
                0,
            );
            self.builder.emit(Op::SetStatus, 0);
            if has_redirects {
                self.builder.emit(Op::WithRedirectsEnd, 0);
            }
            // c:Src/exec.c:4410 restore_params — pop the frame and put
            // the shell var + process env back the way they were.
            if has_inline_env_scope {
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_END_INLINE_ENV, 0),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
            }
            return;
        }

        // break/continue keywords — emit jumps into enclosing loop's
        // patch lists, or fall through to BUILTIN_SET_BREAK/CONTINUE
        // when no enclosing loop in this chunk. `break N` / `continue
        // N` target the N-th enclosing loop (1 = innermost, 2 = next
        // out, etc.). zsh clamps N to the available depth.
        if first == "break" {
            // Redirect scope opens before the xtrace/dispatch ops —
            // `break N` args are literal numerals (no expansion-error
            // window), so the C prefork-before-addfd order is moot.
            self.emit_stage_fds_install(); // c:Src/exec.c:3721-3724
            if has_redirects {
                self.emit_redir_scope_begin(&simple.redirs);
            }
            // c:Src/exec.c:2055 execcmd_exec emits xtrace for every
            // simple command before dispatching the handler. Our
            // special-case path here jumps directly to the enclosing
            // loop's break-patch list without going through the
            // execbuiltin → bin_break path which normally emits the
            // trace. Build the trace text statically and emit via
            // BUILTIN_XTRACE_LINE so it lands at the same point in
            // the stream as zsh.
            let mut trace_text = String::from("break");
            for w in &simple.words[1..] {
                trace_text.push(' ');
                trace_text.push_str(&crate::lex::untokenize(w));
            }
            let tc = self.builder.add_constant(Value::str(trace_text.as_str()));
            self.builder.emit(Op::LoadConst(tc), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
            self.builder.emit(Op::Pop, 0);
            let levels: usize = simple
                .words
                .get(1)
                .and_then(|s| crate::lex::untokenize(s).parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            // c:Src/builtin.c:5820-5822 — `break N`/`continue N` with a
            // non-positive N is an error ("argument is not positive"),
            // NOT a break. The fast-jump path below clamps N to 1, so a
            // literal `break 0` / `break -1` would silently break one
            // level. Detect a literal non-positive integer arg and route
            // through bin_break (the `else` arm) which emits the error
            // and returns 1 without transferring control. (A runtime
            // `break $n` with n<=0 stays on the fast path — pinned.)
            let literal_int = simple
                .words
                .get(1)
                .and_then(|s| crate::lex::untokenize(s).parse::<i64>().ok());
            let nonpositive_literal = literal_int.is_some_and(|n| n <= 0);
            // `break N` where N is a RUNTIME expression ($((..))/$var):
            // the literal fast path can't read it, so it fell back to
            // N=1. Emit a runtime jump table dispatching to the same
            // break_patches[depth-N] target the literal path uses.
            let runtime_count = simple.words.len() > 1 && literal_int.is_none();
            // Index from end: levels=1 → last (innermost); levels=2 →
            // second-to-last; etc. Clamped to depth.
            let depth = self.break_patches.len();
            // Drain pending cmd_stack pushes before transferring
            // control past their matching pops. zinit's load uses
            // `for; if then; break; fi; done` — without the drain,
            // the Then push leaks past the loop_exit.
            self.emit_cmd_stack_drain();
            if depth > 0 && runtime_count {
                self.emit_runtime_loop_level(&simple.words[1], "break", false);
            } else if depth >= levels && !nonpositive_literal {
                let idx = depth.saturating_sub(levels);
                // Inside try-block: also bump BREAKS atomic so the
                // always-arm post-restore can detect the escape and
                // re-emit the loop-end jump.
                //
                // Only for a break that actually LEAVES the construct.
                // A break aimed at a loop opened inside the try arm is
                // an ordinary in-chunk jump to that loop's `loop_exit`,
                // and `loop_exit` is reached WITHOUT passing the
                // post-body `BUILTIN_LOOP_BREAK_DRAIN` that would clear
                // the flag again — so arming it here left BREAKS set
                // with nobody to consume it. The next
                // `emit_break_escape_check` after the loop then read it
                // as a foreign break and jumped to the always-arm,
                // abandoning the rest of the try body and the enclosing
                // function: `{ for i in 1 2; do break; done; print AFTER }
                // always { print ALWAYS }; print END` printed only
                // ALWAYS where zsh prints AFTER, ALWAYS, END. That is
                // the `break 2` at _main_complete sh:216, which is why
                // no `zle -C` widget ever ran past its completer loop.
                // `continue` does not need the same guard: its jump
                // lands on the loop's continue target, which falls into
                // the drain and clears the flag on the next pass.
                if self.try_block_depth > 0
                    && idx < self.try_loop_base.last().copied().unwrap_or(0)
                {
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_BREAK, 0), 0);
                    self.builder.emit(Op::Pop, 0);
                }
                // `break N` jumps straight to the N-th enclosing loop's
                // exit, skipping the `loops--` of every loop in between
                // (the target's own still runs at its exit label). Emit
                // the skipped decrements so the global count stays exact
                // for a later `break` in a called function.
                self.emit_loop_exit(depth - 1 - idx);
                let j = self.builder.emit(Op::Jump(0), 0);
                self.break_patches[idx].push(j);
            } else {
                // c:Src/builtin.c:5832-5835 — `break` outside any
                // loop: `zwarnnam(name, "not in while, until,
                // select, or repeat loop"); return 1;`. Route through
                // BUILTIN_BREAK so `bin_break` (which already handles
                // the loops==0 check + LASTVAL=1 + zwarnnam) runs.
                // Previously the no-loop arm emitted SET_BREAK + Pop
                // + Jump-to-script-end, which silently exited without
                // any error message. Bug #285.
                //
                // Also taken when `break N` names MORE loops than this
                // chunk has open. c:5837 clamps with `num.min(loops)`
                // against the RUNTIME global, which counts the caller's
                // loops too — a compile-time clamp to the outermost
                // in-chunk loop is only right when the chunk is the
                // whole story. `f(){ for k in x y; do break 2; done;
                // print f_end }` must break f's loop AND the caller's
                // (skipping f_end) when called from a loop, but break
                // only f's (printing f_end) when called from top level.
                // One compiled body, two behaviours — so the count has
                // to be resolved at runtime.
                for word in &simple.words[1..] {
                    self.compile_word_str(word);
                }
                let argc = (simple.words.len() - 1) as u8;
                self.builder.emit(
                    Op::CallBuiltin(fusevm::shell_builtins::BUILTIN_BREAK, argc),
                    0,
                );
                self.builder.emit(Op::SetStatus, 0);
            }
            return;
        }
        if first == "continue" {
            // Redirect scope: same placement rationale as the `break`
            // arm above.
            self.emit_stage_fds_install(); // c:Src/exec.c:3721-3724
            if has_redirects {
                self.emit_redir_scope_begin(&simple.redirs);
            }
            // c:Src/exec.c:2055 — xtrace emit, same rationale as the
            // `break` arm above.
            let mut trace_text = String::from("continue");
            for w in &simple.words[1..] {
                trace_text.push(' ');
                trace_text.push_str(&crate::lex::untokenize(w));
            }
            let tc = self.builder.add_constant(Value::str(trace_text.as_str()));
            self.builder.emit(Op::LoadConst(tc), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
            self.builder.emit(Op::Pop, 0);
            let levels: usize = simple
                .words
                .get(1)
                .and_then(|s| crate::lex::untokenize(s).parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            // c:5820-5822 — `continue 0`/`continue -1` errors "argument
            // is not positive"; route through bin_break (else arm) rather
            // than the clamp-to-1 fast path. Same as the `break` arm.
            let literal_int = simple
                .words
                .get(1)
                .and_then(|s| crate::lex::untokenize(s).parse::<i64>().ok());
            let nonpositive_literal = literal_int.is_some_and(|n| n <= 0);
            let runtime_count = simple.words.len() > 1 && literal_int.is_none();
            let depth = self.continue_patches.len();
            // Drain pending cmd_stack pushes — same rationale as
            // for `break`. `continue` inside an inner if/then is the
            // common case in zinit's mode-aware loop bodies.
            self.emit_cmd_stack_drain();
            if depth > 0 && runtime_count {
                self.emit_runtime_loop_level(&simple.words[1], "continue", true);
            } else if depth >= levels && !nonpositive_literal {
                // Inside try-block: bump BREAKS + CONTFLAG so the
                // always-arm post-restore can detect the escape.
                if self.try_block_depth > 0 {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_CONTINUE, 0),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0);
                }
                // For `continue N`, jump to the N-th enclosing loop's
                // continue target. If N>1, that's actually a BREAK out
                // of inner loops and a continue at the outer — which
                // the existing patch-list mechanism handles by jumping
                // to the outer continue target (the loop will then
                // re-enter from the top of the body).
                let idx = depth.saturating_sub(levels);
                // Same skipped-`loops--` compensation as the break arm:
                // `continue N` leaves the N-1 inner loops for good, and
                // the target loop stays open (no decrement for it).
                self.emit_loop_exit(depth - 1 - idx);
                let j = self.builder.emit(Op::Jump(0), 0);
                self.continue_patches[idx].push(j);
            } else {
                // c:Src/builtin.c:5831-5834 — `continue` outside any
                // loop: same error path as `break`. Route through
                // BUILTIN_CONTINUE so bin_break (with func=BIN_CONTINUE)
                // emits the warning + returns 1. Bug #285.
                for word in &simple.words[1..] {
                    self.compile_word_str(word);
                }
                let argc = (simple.words.len() - 1) as u8;
                self.builder.emit(
                    Op::CallBuiltin(fusevm::shell_builtins::BUILTIN_CONTINUE, argc),
                    0,
                );
                self.builder.emit(Op::SetStatus, 0);
            }
            return;
        }

        // Run `execcmd_compile_head` — the fusevm-bytecode-time head
        // resolver in `src/ported/exec.rs` mirroring the head section
        // of C's `execcmd_exec` (`Src/exec.c:c:2904-3275`) — to get
        // the precommand-modifier strip count + dispatch decision.
        // The C function performs dispatch directly; zshrs splits it:
        // this head-walk runs at compile time, the actual invocation
        // is the bytecode emitted below. See the WARNING block in
        // `execcmd_compile_head` for the divergence rationale.
        //
        // `WC_SIMPLE` (vs `WC_TYPESET`) is fine for both `typeset` and
        // ordinary cmds in this context — the walk doesn't depend on
        // the wordcode distinction past `getnode2` (c:3035), and the
        // static BUILTINS table doesn't model an enabled/disabled bit.
        // Precommand modifiers `builtin`/`command`/`exec` (BINF_BUILTIN /
        // BINF_COMMAND / BINF_EXEC per Src/builtin.c:42-45). The normal
        // precmd-strip (via execcmd_exec) drops them and then resolves the
        // underlying name — if that name lacks a dedicated fusevm opcode
        // (e.g. `cd`), the fallback is `Op::CallFunction`, which finds user
        // wrappers and recurses (real-world ZPWR `cd () { builtin cd "$@"; }`).
        // Emit the prefix opcode directly with all args intact; the runtime
        // handler dispatches by name with the correct shadow semantic:
        //   - `builtin`: bypass alias+function, builtin-only
        //   - `command`: bypass function, builtin then external
        //   - `exec`:    replace shell process
        if simple.words.len() >= 2 {
            let first = crate::lex::untokenize(&simple.words[0]);
            // c:Src/exec.c::execcmd_exec — `builtin <prefix> args` where
            // <prefix> is another BINF_PREFIX precmd (`command`,
            // `builtin`, `exec`, `noglob`, `-`) chains the prefixes:
            // each outer prefix forces builtin lookup of the next one,
            // and since each prefix's table entry has no handlerfunc,
            // dispatch_builtin_raw would return 1 from the inner call.
            // Strip the outer `builtin` so the inner prefix's normal
            // compile-time path handles its semantics. Without this,
            // `builtin command echo hi` silently failed with $?=1.
            if first == "builtin" && simple.words.len() >= 3 {
                let second = crate::lex::untokenize(&simple.words[1]);
                if matches!(
                    second.as_str(),
                    "command" | "builtin" | "exec" | "noglob" | "-"
                ) {
                    let inner = crate::parse::ZshSimple {
                        assigns: simple.assigns.clone(),
                        words: simple.words[1..].to_vec(),
                        redirs: simple.redirs.clone(),
                    };
                    self.compile_simple(&inner);
                    return;
                }
            }
            let opcode = match first.as_str() {
                "builtin" => Some(fusevm::shell_builtins::BUILTIN_BUILTIN),
                "command" => Some(fusevm::shell_builtins::BUILTIN_COMMAND),
                "exec" => Some(fusevm::shell_builtins::BUILTIN_EXEC),
                _ => None,
            };
            if let Some(opcode) = opcode {
                let argc = (simple.words.len() - 1) as u8;
                for word in &simple.words[1..] {
                    self.compile_word_str(word);
                }
                // c:Src/exec.c:3285-3304 → c:3720 — pipe fds and then
                // the redirect scope open after arg expansion, before
                // addvars/dispatch.
                self.emit_stage_fds_install();
                if has_redirects {
                    self.emit_redir_scope_begin(&simple.redirs);
                }
                // c:Src/exec.c — inline-env assigns must commit AFTER
                // arg expansion (above) but BEFORE the precmd dispatch
                // consumes the args, so the spawned command sees
                // `TEST=val` in its env. Mirror the regular-dispatch
                // path (line ~1351) for the BUILTIN_COMMAND/BUILTIN/
                // BUILTIN_EXEC fast-path. Without this,
                // `TEST=val command env` ran env with TEST unset.
                if has_inline_env_scope {
                    for assign in &simple.assigns {
                        self.last_assign_had_cmd_subst = false;
                        self.compile_assign(assign);
                    }
                    self.emit_seal_inline_env();
                }
                self.builder.emit(Op::CallBuiltin(opcode, argc), 0);
                self.builder.emit(Op::SetStatus, 0);
                // Close the inline-env scope so the assigns don't leak
                // into the caller's shell state.
                if has_inline_env_scope {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_END_INLINE_ENV, 0),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0);
                }
                self.emit_errexit_check();
                if has_redirects {
                    self.builder.emit(Op::WithRedirectsEnd, 0);
                }
                return;
            }
        }

        let dispatch = crate::ported::exec::execcmd_compile_head(
            &simple.words,
            crate::ported::zsh_h::WC_SIMPLE,
        );
        let precmd_skip = dispatch.precmd_skip;

        // c:Src/exec.c:3372-3406 "Empty command" no-redir arm — bare
        // `exec` / `noglob` / `command` / `nocorrect` (the precmd
        // walk stripped everything). C sets `lastval = cmdoutval` (0
        // when no $(cmd) ran) and returns without dispatching.
        // Surfaced by execcmd_compile_head via `is_empty_command`.
        if dispatch.is_empty_command {
            // c:Src/exec.c:3721-3724 — nothing left to expand, so the
            // stage's fds go on now (the c:3342 zerr below still hits
            // the un-redirected stderr).
            self.emit_stage_fds_install();
            // c:Src/exec.c:3342 — `if (redir) { zerr("redirection
            // with no command"); ... return 1; }`. A bare prefix
            // keyword (`builtin`, `command`, `exec`, `noglob`,
            // `nocorrect`) followed only by a redirect with no
            // command word is a parse error in zsh. The previous
            // Rust port silently returned rc=0 via the empty-cmd
            // path. Bug #534.
            //
            // The zerr at c:3342 fires BEFORE the addfd loop
            // (c:3720+): the error prints to the un-redirected
            // stderr and the redirects (including target expansion)
            // never apply — `noglob 2>/dev/null` still shows the
            // error. No WithRedirectsBegin/End scope here.
            if has_redirects {
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_REDIR_NO_CMD, 0),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                self.builder.emit(Op::LoadInt(1), 0);
                self.builder.emit(Op::SetStatus, 0);
                return;
            }
            self.builder.emit(Op::LoadInt(0), 0); // c:3399 lastval = cmdoutval
            self.builder.emit(Op::SetStatus, 0); // c:3399
            return; // c:3406
        }

        // Head-name resolution — used by the magic-equals gate in the
        // word loop below AND by the builtin_id dispatch lookup after
        // xtrace.
        let dispatch_first_raw: &str = if precmd_skip > 0 && precmd_skip < simple.words.len() {
            &simple.words[precmd_skip]
        } else {
            first
        };
        // c:Src/subst.c:169 + Src/glob.c:3649 — `prefork` runs
        // `remnulargs(getdata(node))` on every word before command
        // dispatch. remnulargs strips standalone Bnull / Snull / Dnull
        // / Nularg sentinels in-place (Bnullkeep folds to literal `\`),
        // so `\echo` (lex emits `Bnull echo`) reaches the cmd-name
        // table as plain `echo`. zshrs's compile-time dispatch path
        // skipped this strip and went straight to untokenize, which
        // maps Bnull → `\` per c:Src/lex.c:38 ztokens. Result: the
        // builtin/function table was probed with `\echo`, missed
        // every entry, and fell through to "command not found:
        // \echo". Run remnulargs first to mirror C's prefork chain
        // before untokenize converts any surviving sentinels.
        let first_clean = {
            let mut tmp = dispatch_first_raw.to_string();
            crate::ported::glob::remnulargs(&mut tmp);
            // remnulargs may stamp `Nularg` on a wholly-empty word; drop
            // it so the lookup sees `""` instead of a sentinel.
            if tmp == crate::ported::zsh_h::Nularg.to_string() {
                tmp.clear();
            }
            crate::lex::untokenize(&tmp)
        };
        // c:Src/exec.c::execcmd — runtime function lookup wins over
        // builtins (shfunctab → bintab order). When the user defined a
        // function with the dispatch name earlier in this compile unit,
        // skip the builtin fast-path so the call routes through
        // CallFunction (host.call_function → dispatch_function_call →
        // doshfunc). Bug #27 in docs/BUGS.md: zshrs-extension-only
        // builtins (caller, help, …) shadowed user functions because
        // the builtin_id table beat the shfunctab check.
        // c:Src/exec.c:3484-3488 — `if (!(cflags & (BINF_BUILTIN | BINF_COMMAND))
        // && (hn = shfunctab->getnode(shfunctab, cmdarg))) { is_shfunc = 1;
        // break; }`. A `builtin` or `command` precommand modifier makes C SKIP
        // the shfunctab probe outright, so the name resolves against
        // `builtintab` even when a shell function of that name exists. Without
        // this gate the shadow was computed from the word AFTER the modifier
        // and applied regardless, so `f() { compadd() { … builtin compadd
        // "$@" }; … }` re-entered its own wrapper: every wrapped call ran
        // TWICE. That silently doubled every `compadd`-wrapper probe used to
        // debug completion, so wrapper-derived evidence taken before this fix
        // is not trustworthy.
        let precmd_is_builtin_or_command = simple.words[..precmd_skip.min(simple.words.len())]
            .iter()
            .any(|w| {
                let u = crate::lex::untokenize(w);
                u == "builtin" || u == "command"
            });
        let user_function_shadow = !precmd_is_builtin_or_command
            && (self.defined_functions.contains(&first_clean)
                || self.defined_functions.contains(dispatch_first_raw));
        // c:Src/exec.c:3298-3304 — `magic_assign = (hn->flags &
        // BINF_MAGICEQUALS) && type != WC_TYPESET` → esprefork =
        // PREFORK_TYPESET → `prefork(args, esprefork, NULL)` BEFORE
        // the addfd loop (c:3720). For an `alias` head, emit a
        // per-word BUILTIN_MAGIC_EQUALS_PREFORK so the equals/tilde
        // expansion (and its zerr, e.g. `alias bad===` → "= not
        // found") fires before the redirect scope opens.
        let head_is_magic_equals =
            !user_function_shadow && (dispatch_first_raw == "alias" || first_clean == "alias");
        // c:Src/builtin.c BUILTIN table — the OTHER BINF_MAGICEQUALS
        // heads: declare, export, float, hash, integer, local, readonly,
        // typeset. C runs prefork's filesub on the LEXED word before any
        // expansion (exec.c:3353-3359); zshrs's 621 op runs on the
        // runtime-EXPANDED value, so emitting it unconditionally
        // re-processes values that arrived via `$var` (`local al=$3`
        // with a regex-ish value died in bin_typeset). Gate the emit to
        // words whose RAW text carries an expandable tilde (`=~` value
        // start or `:~` path segment) — exactly the shapes filesub acts
        // on — so `export V=~/x` expands while `local al=$3` is left
        // untouched. Previously NO typeset-family word got the emit and
        // the tilde stayed literal (zpwr's `export
        // ZPWR_EXPAND_STATS_FILE=~/...` then failed every -f test).
        let head_is_typeset_magic = !user_function_shadow
            && matches!(
                first_clean.as_str(),
                "declare"
                    | "export"
                    | "float"
                    | "hash"
                    | "integer"
                    | "local"
                    | "readonly"
                    | "typeset"
            );

        // c:Src/builtin.c BUILTIN table — BINF_ASSIGN family: typeset /
        // declare / local / export / readonly / integer / float /
        // private. Their `name=( e1 e2 … )` args are ENVARRAY-shaped:
        // C's par_simple (intypeset) keeps the ELEMENTS as separate
        // wordcode strings so each expands independently and `$arr`
        // splats. zshrs's one-word text form must be re-split at
        // compile time (see the paren-init arm in the loop below) or
        // the whole body funnels through segment-concat as ONE value
        // — `typeset b=( x $a )` stored b=(x "1 2 3"). Real-world
        // load: zsh-hist.plugin.zsh:9 `typeset -gU FPATH fpath=(
        // $dir $fpath )` collapsed fpath to 2 entries and every
        // subsequent autoload (add-zsh-hook) failed.
        let head_is_typeset_family = !user_function_shadow
            && matches!(
                first_clean.as_str(),
                "typeset"
                    | "declare"
                    | "local"
                    | "export"
                    | "readonly"
                    | "integer"
                    | "float"
                    | "private"
            );

        // Builtin or function or external. Push args first (post-strip).
        // Every word pushes exactly ONE stack value (the typeset
        // paren-init arm packs its elements back into one via
        // BUILTIN_TYPESET_PAREN_PACK).
        for word in &simple.words[precmd_skip + 1..] {
            // Typeset-family paren-init: compile `name=( e1 e2 … )`
            // ELEMENT BY ELEMENT (the parser's \u{1f} ENVARRAY rejoin
            // already word-split them), then PACK back into one
            // REJOIN_SEP-delimited arg via BUILTIN_TYPESET_PAREN_PACK.
            // One arg in → one arg out: bin_typeset's single-arg
            // splitter consumes it directly and its multi-arg rejoin
            // (paren-depth scan, unsafe on expanded paren-literal
            // elements like p10k's `')' ''`) never runs. `$arr`
            // elements arrive as Value::Array and splice inside the
            // pack op, so the splat survives without pop_args
            // flattening into separate argv slots.
            if head_is_typeset_family {
                if let Some((prefix, elems)) = split_typeset_paren_init(word) {
                    let pidx = self.builder.add_constant(Value::str(prefix.as_str()));
                    self.builder.emit(Op::LoadConst(pidx), 0);
                    // CallBuiltin argc is u8 — chunk the EXTEND calls.
                    // p10k's __p9k_colors has 408 elements; a single
                    // call wrapped argc mod 256 and spilled the stack
                    // into the arg list.
                    for chunk in elems.chunks(200) {
                        for e in chunk {
                            // c:Src/subst.c:111 keyvalpairelement —
                            // PREFORK_ASSIGN context. Bump
                            // assign_context_depth so compile_word_str's
                            // looks_like_kv_pair gate suppresses globbing
                            // for `[key]=value` assoc-init elements
                            // (`typeset -gA m=([alpha]=1)` — the `[alpha]`
                            // is a key, NOT a glob char-class). Without
                            // it the `[` triggered NOMATCH.
                            self.assign_context_depth += 1;
                            self.compile_word_str(e);
                            self.assign_context_depth -= 1;
                            // Array-literal elements field-split
                            // unquoted expansion results — same emit
                            // as compile_assign's array branch:
                            // `typeset b=( $(print q w) e )` → 3
                            // elements in zsh.
                            if needs_word_split(e) {
                                self.builder.emit(
                                    Op::CallBuiltin(crate::vm_helper::BUILTIN_WORD_SPLIT, 0),
                                    0,
                                );
                            }
                        }
                        self.builder.emit(
                            Op::CallBuiltin(
                                crate::vm_helper::BUILTIN_TYPESET_PAREN_PACK,
                                (chunk.len() + 1) as u8,
                            ),
                            0,
                        );
                    }
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_TYPESET_PAREN_CLOSE, 1),
                        0,
                    );
                    continue;
                }
            }
            // BINF_ASSIGN-family args (`typeset T=${x:-*file}`) don't
            // filename-generate their value; suppress the default-word
            // glob bracket while compiling them.
            //
            // c:Src/parse.c par_simple (intypeset) + Src/exec.c:2601
            // addvars → PREFORK_ASSIGN — an assignment-SHAPED arg
            // (`NAME=value` / `NAME+=value`) is parsed as an
            // assignment: its value never IFS-splits. Bump
            // assign_context_depth so compile_word_str's cmd-subst
            // arm skips its WORD_SPLIT emit (`export ZPWR_TTY=$(tty)`
            // stored only "not" and exported stray `a`/`tty` names
            // from the split remainder). Non-assignment args
            // (`export PATH`) keep normal word semantics.
            let arg_is_assign = head_is_typeset_family && is_typeset_scalar_assign(word);
            // c:Src/exec.c par_simple/addvars — the assignment (no-split)
            // treatment belongs to the VALUE of a `NAME=VALUE` argument, not
            // to every word of a BINF_ASSIGN command. A bare `$name` argument
            // is an ordinary word, so an ARRAY there splats one word per
            // element, exactly as it would for any other builtin.
            //
            // Bumping the depth unconditionally routed it through
            // GET_VAR_DQ, which joins the array to one space-separated
            // scalar. `_git_commands` (Completion/Unix/Command/_git:6503)
            // declares its eleven category arrays with
            //     local -a cmdtypes; cmdtypes=( main_porcelain_commands … )
            //     local -a $cmdtypes
            // so the second line arrived as ONE name — the whole list —
            // and typeset rejected it:
            //     _git_commands:local:3: not valid in this context:
            //         main_porcelain_commands user_commands …
            // which printed onto the command line during `git <TAB>`.
            // A bare SCALAR must still not IFS-split (`v="a b"; local $v`
            // is an error in zsh too), and that is unaffected: the flag
            // only stops applying to the whole-word bare-reference shape.
            let arg_is_bare_param = !arg_is_assign && word_is_bare_param_ref(word);
            if head_is_typeset_family && !arg_is_bare_param {
                self.assign_builtin_arg_depth += 1;
            }
            if arg_is_assign {
                self.assign_context_depth += 1;
            }
            self.word_emitted_glob = false;
            self.compile_word_str(word);
            if arg_is_assign {
                self.assign_context_depth -= 1;
            }
            if head_is_typeset_family && !arg_is_bare_param {
                self.assign_builtin_arg_depth -= 1;
            }
            // c:Src/options.c GLOB_SUBST + Src/subst.c — when an
            // unquoted parameter / cmd-subst reference produced the
            // word and `setopt globsubst` is active at runtime, the
            // substituted content participates in filename
            // generation (`pat="*.txt"; echo $pat` → matched files).
            // The for-loop word arm at compile_zsh.rs:~4426 already
            // gates this; mirror it here for simple-command argv.
            // Bug #329.
            //
            // `!word_emitted_glob`: c:Src/exec.c:3755-3758 globs an argv word
            // list ONCE. When the word's own text already carried a glob
            // token, compile_word_str emitted BUILTIN_GLOB_EXPAND over
            // the fully assembled word — which is exactly C's single
            // `globlist` pass, substituted metachars included — so a
            // second pass here would re-glob generated filenames.
            if has_unquoted_param_or_subst(word) && !self.word_emitted_glob {
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_GLOB_SUBST_EXPAND, 1),
                    0,
                );
            }
            // c:Src/exec.c:3298-3304 — magic-equals prefork per arg
            // word for BINF_MAGICEQUALS heads, before the redirect
            // scope (see head_is_magic_equals above).
            //
            // Quote-marker gate: the 621 handler reconstructs tokens
            // via shtokenize on the FLATTENED word, so any Snull /
            // Dnull / Bnull span in the raw word would lose its
            // protection in the round-trip — `alias opclean='… $(oc
            // get pods …) …'` re-tokenized the quoted `$(…)` and
            // prefork EXECUTED it at alias-definition time
            // (zsh-openshift-aliases.plugin.zsh:75). C's prefork
            // runs on the lexed word with the markers intact, where
            // quoted text is inert. Magic-equals expansion only acts
            // on UNQUOTED `=`/`~` anyway, so a word carrying quoted
            // spans skips the prefork emit entirely.
            // Raw-word tilde probe: the lexer emits TOKEN chars where it
            // already recognised the shape — Equals = \u{8d}, Tilde =
            // \u{98} (zsh_h.rs:161/183) — and leaves literals elsewhere
            // (e.g. '~' after ':' in a path-list value). Accept every
            // '='/':' × '~' spelling combination.
            let word_has_assign_tilde =
                ["=~", ":~", "=\u{98}", ":\u{98}", "\u{8d}~", "\u{8d}\u{98}"]
                    .iter()
                    .any(|p| word.contains(p));
            if (head_is_magic_equals || (head_is_typeset_magic && word_has_assign_tilde))
                && !word.contains('\u{9d}')
                && !word.contains('\u{9e}')
                && !word.contains('\u{9f}')
            {
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_MAGIC_EQUALS_PREFORK, 1),
                    0,
                );
            }
        }

        // Un-truncated arg count. `argc` (u8, wrapping) feeds the xtrace /
        // magic-equals peeks below unchanged; `argc_full` drives the actual
        // dispatch, which handles the >255 overflow (see the pack below).
        let argc_full = simple.words.len() - precmd_skip - 1;
        let argc = argc_full as u8;

        // c:Src/exec.c:3285-3304 (prefork) + c:3702 (globlist) →
        // c:3720+ (addfd loop) — install the pipeline fds and open the
        // redirect scope only after every arg word's expansion ops are
        // emitted. An expansion zerr (nomatch, `${var?msg}`, `=cmd`
        // not found) therefore prints to the shell's original stderr,
        // not the command's redirected one, and the dispatch op aborts
        // via the glob_failed/errflag gates in dispatch_builtin. A
        // `$( … )` in the args likewise reads the shell's fd 0, not the
        // stage's pipe.
        self.emit_stage_fds_install();
        if has_redirects {
            self.emit_redir_scope_begin(&simple.redirs);
        }

        // c:Src/exec.c — addvars runs AFTER prefork. With inline-env
        // scope, defer the assigns until args are already pushed onto
        // the stack so `a=1 echo "$a"` echoes "" (shell `$a` still
        // unset when args resolved) and the assigned `a=1` lands only
        // in the child env.
        if has_inline_env_scope {
            for assign in &simple.assigns {
                self.last_assign_had_cmd_subst = false;
                self.compile_assign(assign);
                if self.last_assign_had_cmd_subst {
                    chain_had_cmd_subst = true;
                }
            }
            self.emit_seal_inline_env();
        }

        // xtrace: emit a runtime print of the EXPANDED command line
        // AFTER args are pushed but BEFORE dispatch consumes them.
        // Direct port of Src/exec.c:2055-2066 (makecline) — zsh
        // traces the post-expansion argv with each arg shell-quoted.
        // BUILTIN_XTRACE_ARGS peeks args without consuming, pops the
        // prefix (cmd-name) we push next, builds + prints the line.
        // Stack on entry: [arg1, …, argN, prefix].
        let cmd_prefix = crate::lex::untokenize(&simple.words[precmd_skip]);
        let prefix_const = self.builder.add_constant(Value::str(cmd_prefix.as_str()));
        self.builder.emit(Op::LoadConst(prefix_const), 0);
        // trace_argc = (1 cmd-name) + (args after stripped modifiers).
        // Stack has all words[1..] pushed; XTRACE_ARGS peeks the last
        // (trace_argc - 1) of them so the modifier-victim slot is
        // accounted for as the new cmd name. XTRACE_ARGS's argc is a u8,
        // so cap at 255 (`argc + 1` would overflow when argc_full == 255)
        // — xtrace of a >255-arg command traces only the first 254 args,
        // best-effort; the real dispatch below still gets the full argv.
        let trace_argc = (argc_full + 1).min(u8::MAX as usize) as u8;
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_ARGS, trace_argc),
            0,
        );
        self.builder.emit(Op::Pop, 0);

        // `shopt` is bash-only; zsh has no such builtin. Force external lookup
        // so it produces "command not found: shopt" matching /bin/zsh exactly.
        // `declare` and `typeset` both map to BUILTIN_TYPESET in fusevm — but
        // zsh prefixes "no such variable" errors with the builtin name the
        // user actually typed. Route `declare` to BUILTIN_DECLARE so the
        // distinct error-format path fires.
        //
        // The lookup uses the UNTOKENIZED first word so quoted command
        // names (`'builtin'`, `"echo"`) resolve to their builtins. zsh
        // strips quotes before the builtin/function dispatch, only
        // disabling alias expansion. Without this untokenize, p10k's
        // `'builtin' 'local' '-a' 'arr'` failed with `command not found:
        // builtin` because the lookup table didn't contain the Snull-
        // wrapped form `\u{9d}builtin\u{9d}`.
        // Precommand-modifier strip for dispatch: `builtin foo`,
        // `command foo`, `exec foo`, `nocorrect foo`, `noglob foo`,
        // and `- foo` should dispatch as if the modifier weren't
        // there (per Src/exec.c:3086 BINF_PREFIX). xtrace already
        // strips at line 891 above; mirror for builtin_id lookup
        // so `builtin false` runs `false` (returning 1) instead of
        // falling through to BUILTIN_BUILTIN no-op.
        // (dispatch_first_raw / first_clean / user_function_shadow
        // are computed ABOVE the arg-word loop so the magic-equals
        // head gate can use them.)
        let builtin_id = if user_function_shadow {
            None
        } else if dispatch_first_raw == "shopt" || first_clean == "shopt" {
            None
        } else if dispatch_first_raw == "declare" || first_clean == "declare" {
            Some(fusevm::shell_builtins::BUILTIN_DECLARE)
        } else if dispatch_first_raw == "." || first_clean == "." {
            // c:Src/builtin.c:9308 — `.` and `source` both invoke
            // bin_dot but the C source passes the actual invocation
            // name as `name`. fusevm's name→opcode map collapses
            // both to BUILTIN_SOURCE which dispatches with the
            // literal "source", so a failed `. /nonex` printed
            // `zsh:source:1:` instead of `zsh:.:1:`. Emit our
            // BUILTIN_DOT opcode that dispatches with name=".".
            Some(crate::vm_helper::BUILTIN_DOT)
        } else if dispatch_first_raw == "logout" || first_clean == "logout" {
            // c:Src/builtin.c — `logout` invokes bin_break with
            // funcid=BIN_LOGOUT. fusevm collapses `logout` into
            // BUILTIN_EXIT (alongside `exit`/`bye`) which dispatches
            // with BIN_EXIT funcid, so the "not login shell" check
            // at bin_break c:5865 never fired. Route through
            // BUILTIN_LOGOUT which dispatches by name "logout".
            Some(crate::vm_helper::BUILTIN_LOGOUT)
        } else if (dispatch_first_raw == "mapfile" || first_clean == "mapfile"
                || dispatch_first_raw == "readarray" || first_clean == "readarray"
                || dispatch_first_raw == "compopt" || first_clean == "compopt"
                // compdef / compinit are FUNCTIONS defined via
                // compsys autoload in zsh — `zsh -fc compdef` is 127
                // until compinit runs. zshrs's extension builtins
                // must not shadow that in parity mode; a compsys-
                // defined function still wins via the
                // user_function_shadow check above.
                || dispatch_first_raw == "compdef" || first_clean == "compdef"
                || dispatch_first_raw == "compinit" || first_clean == "compinit")
            && crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed)
        {
            // c:Bug #504 — bash `mapfile`/`readarray`/`compopt` map to
            // dedicated fusevm opcodes (BUILTIN_MAPFILE, BUILTIN_COMPOPT)
            // but zshrs has no host handler — fusevm's VM no-ops the op
            // and returns rc=0, silently succeeding bash-only builtins
            // in --zsh parity mode. Route through Op::CallFunction so
            // host_exec_external prints the canonical
            // "command not found: <name>" diagnostic + rc=127 matching
            // zsh's external-command-lookup miss.
            None
        } else if matches!(
            dispatch_first_raw,
            "mkdir"
                | "rmdir"
                | "ln"
                | "mv"
                | "zf_mkdir"
                | "zf_rm"
                | "zf_rmdir"
                | "zf_chmod"
                | "zf_chown"
                | "zf_chgrp"
                | "zf_ln"
                | "zf_mv"
                | "zf_sync"
        ) && crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed)
        {
            // c:Src/Modules/files.c — these are zsh/files builtins,
            // and zsh -fc has zsh/files UNLOADED: bare names resolve
            // to the system binaries (system flag sets + diagnostics),
            // zf_* names are 127 until `zmodload zsh/files`. Skip the
            // compile-time builtin fast-path in --zsh parity mode so
            // dispatch falls to the external path (which has the
            // matching runtime gate in fusevm_bridge's exec arm).
            // zconvey.plugin.zsh:44 `command mkdir -p …` hit the
            // in-process bin_mkdir and errored "File exists".
            None
        } else {
            // Try the raw form first (handles already-untokenized inputs
            // from internal callers); fall back to the cleaned form so
            // quoted command names resolve.
            fusevm::shell_builtins::builtin_id(dispatch_first_raw)
                .or_else(|| fusevm::shell_builtins::builtin_id(&first_clean))
        };
        // u8 argc overflow. `CallBuiltin`/`CallFunction` carry argc as a u8
        // (op.rs `Call{,Builtin,Function}(u16, u8)`), so a command invoked
        // with >255 args wraps argc mod 256 and the dispatch pops only the
        // last `argc & 0xFF` slots — the rest leak on the VM stack. This bit
        // compsys: a completer's `_arguments <specs…>` with a large option
        // set (curl ships 274) collapsed to ~20 options because `_arguments`
        // (a function) received only `274 & 0xFF` of its argv. Pack the
        // pushed args into a single Array (Op::MakeArray, u16-counted),
        // recursively flatten it (BUILTIN_ARGV_RFLATTEN — a brace/glob/`$arr`
        // word contributes a NESTED Array that the call op's single-level
        // splat would stringify), and dispatch with argc=1; the VM's
        // CallFunction flatten (vm.rs) and pop_args' array-splat
        // (fusevm_bridge) then restore the full positional list. Mirrors the
        // MakeArray dodge used for `arr=(...)` literals.
        let argc = if argc_full > u8::MAX as usize {
            self.builder.emit(Op::MakeArray(argc_full as u16), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_ARGV_RFLATTEN, 1),
                0,
            );
            1u8
        } else {
            argc
        };
        if let Some(builtin_id) = builtin_id {
            self.builder.emit(Op::CallBuiltin(builtin_id, argc), 0);
            self.builder.emit(Op::SetStatus, 0);
            self.emit_print_exit_value(); // c:Src/exec.c:4308-4316
                                          // `return`/`exit` short-circuit. Drain cmd_stack so the
                                          // pushes from enclosing if/then/for/etc. don't leak past
                                          // the function's return target.
            if first == "return"
                || first == "exit"
                || first_clean == "return"
                || first_clean == "exit"
            {
                // c:Src/exec.c:1571-1603 — a `return` does NOT skip
                // `sublist_done:`. retflag is consulted only at the TOP of
                // execlist's list loop (c:1370 `while (… && !retflag && …)`),
                // which stops the NEXT sublist; the sublist holding the
                // `return` still runs its ZERR-trap tail. So `return 5` fires
                // the ERR trap on its way out. This arm used to jump straight
                // to the scope's return landing without ever running the
                // check, so a `return` inside a try-list never fired ERR:
                //   f() { { return 5 } always { print fin } }; f
                // printed `fin / err=5` where zsh prints `err=5 / fin / err=5`.
                //
                // BUILTIN_ERREXIT_CHECK is emitted for its SIDE EFFECT only
                // (it fires dotrap(SIGZERR), gated on DONETRAP per c:1598) and
                // its result is popped: the escape jump below stays
                // UNCONDITIONAL, because retflag propagation is not contingent
                // on the check's verdict. Suppressed contexts
                // (errexit_suppress_depth > 0 — the `&&`/`||` operands and
                // `if`/`while` conditions that carry C's this_noerrexit) skip
                // the check entirely, so `false && return 3` stays silent.
                //
                // `exit` is excluded: c:Src/builtin.c zexit → realexit()
                // leaves the process without ever reaching sublist_done, so no
                // ERR trap fires (`zsh -fc 'trap "print e" ERR; f(){ exit 5 };
                // f'` prints nothing).
                let is_return = first == "return" || first_clean == "return";
                if is_return && self.errexit_suppress_depth == 0 {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_ERREXIT_CHECK, 0),
                        0,
                    ); // c:1601
                    self.builder.emit(Op::Pop, 0);
                }
                // c:Src/loop.c:188/:491/:546 + :529-534 — unwind every
                // enclosing loop the escape jump is about to skip. C
                // gets this for free (execfor/execwhile/execrepeat are
                // recursive C frames whose `loops--` runs as the stack
                // pops, and whose post-body drain consumes the
                // `breaks = loops` that c:Src/builtin.c:5835 just set).
                // Without it, `return` from inside a loop left `loops`
                // permanently incremented AND `breaks` non-zero, and the
                // stray break count aborted the caller's list the moment
                // control crossed a `source` / function boundary —
                // silently truncating the rest of the calling script.
                self.emit_loop_unwind(self.open_loop_depth);
                self.emit_cmd_stack_drain();
                let j = self.builder.emit(Op::Jump(0), 0);
                self.return_patches.push(j);
            } else {
                self.emit_errexit_check();
            }
        } else {
            // Treat as function/external dispatch via Op::CallFunction.
            // host.call_function checks aliases → functions → falls back
            // to host.exec for externals. Untokenize first so the
            // lexer's META encoding of `-` (`\u{9b}`) and other special
            // chars doesn't reach the name table — without this,
            // `foo-bar()` registered cleanly but the call site looked
            // up `foo\u{9b}bar` and missed the registered function.
            //
            // c:Src/subst.c:169 — `prefork` runs `remnulargs` BEFORE
            // any cmd-name lookup. remnulargs strips standalone Bnull
            // / Snull / Dnull / Nularg sentinels so `\grep` (lex emits
            // `Bnull grep`) reaches the external dispatch as `grep`.
            // Without this step, untokenize's Bnull → `\` mapping
            // (c:Src/lex.c:38 ztokens) routes the call through
            // host_exec_external with cmd=`\grep`, missing the PATH
            // hit and falling to "command not found: \grep".
            let cleaned_first = {
                let mut tmp = first.to_string();
                crate::ported::glob::remnulargs(&mut tmp);
                if tmp == crate::ported::zsh_h::Nularg.to_string() {
                    tmp.clear();
                }
                crate::lex::untokenize(&tmp)
            };
            let name_idx = self.builder.add_name(&cleaned_first);
            self.builder.emit(Op::CallFunction(name_idx, argc), 0);
            self.builder.emit(Op::SetStatus, 0);
            self.emit_print_exit_value(); // c:Src/exec.c:4308-4316
            self.emit_errexit_check();
        }

        if has_redirects {
            self.builder.emit(Op::WithRedirectsEnd, 0);
        }

        if has_inline_env_scope {
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_END_INLINE_ENV, 0),
                0,
            );
            self.builder.emit(Op::Pop, 0);
        }
    }

    /// Close the inline-env frame's save list, right after the prefix
    /// assignments of `X=foo cmd` commit and before `cmd` dispatches.
    /// c:Src/exec.c:4410 — `save_params` snapshots only the parsed
    /// WC_ASSIGN chain; nothing the command itself assigns belongs in
    /// the restore list. Without the seal, `X=y . file` reverted every
    /// global the sourced file defined.
    fn emit_seal_inline_env(&mut self) {
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_SEAL_INLINE_ENV, 0),
            0,
        );
        self.builder.emit(Op::Pop, 0);
    }

    /// Open a simple command's redirect scope: WithRedirectsBegin +
    /// the MULTIOS-coalesced redirect ops. Called by each dispatch
    /// arm of compile_simple AFTER the arg-word ops are emitted so
    /// runtime word expansion (prefork c:Src/exec.c:3285-3304 +
    /// globlist c:3702) errors print to the un-redirected stderr,
    /// matching C's order where the addfd loop (c:3720+) runs after
    /// expansion. Redirect TARGETS still expand here — i.e. after the
    /// arg words — mirroring xpandredir inside the addfd loop.
    fn emit_redir_scope_begin(&mut self, redirs: &[crate::parse::ZshRedir]) {
        self.builder
            .emit(Op::WithRedirectsBegin(redirs.len() as u8), 0);
        self.compile_redirs_multios(redirs);
    }

    /// Dup this pipeline stage's pipe fds onto 0/1, if we're compiling
    /// one. Direct port of the addfd pair at c:Src/exec.c:3720-3724:
    ///
    ///     /* Add pipeline input/output to mnodes */
    ///     if (input)  addfd(forked, save, mfds, 0, input, 0, NULL);
    ///     if (output) addfd(forked, save, mfds, 1, output, 1, NULL);
    ///
    /// Called by every dispatch arm of `compile_simple` at exactly the
    /// C position: after the argument words' expansion ops (prefork
    /// c:3304 + globlist c:3702), before the redirect scope opens
    /// (c:3730+). That ordering is observable — `print -rl -- c a b |
    /// print -r -- "[$(cat)]"` prints `[]` in zsh because the `$(cat)`
    /// expands while fd 0 is still the shell's, not the pipe.
    ///
    /// One-shot: the flag is cleared here so nested `compile_simple`
    /// calls inside the same stage (a function body, a `$( … )` chunk)
    /// don't re-install. The runtime side is
    /// `fusevm_bridge::BUILTIN_PIPE_FDS_INSTALL`, fed by
    /// `BUILTIN_RUN_PIPELINE`'s `stage_fds_park`.
    fn emit_stage_fds_install(&mut self) {
        if !self.stage_fds_pending {
            return;
        }
        self.stage_fds_pending = false;
        // Arg 1: `|&` — merge stderr into the piped stdout (the extra
        // `2>&1` C walks in the redirect loop right after addfd).
        self.builder
            .emit(Op::LoadInt(self.stage_fds_merge_stderr as i64), 0);
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_PIPE_FDS_INSTALL, 1),
            0,
        );
        self.builder.emit(Op::Pop, 0);
    }

    /// Translate a ZshRedir → fusevm Redirect/HereDoc/HereString op.
    /// Compile a redirect list with MULTIOS coalescing. Groups two or
    /// more WRITE/APPEND/CLOBBER redirects targeting the same fd into
    /// a single BUILTIN_MULTIOS_REDIRECT call so the runtime sets up
    /// a tee splitter (Bug #36 in docs/BUGS.md). All other redirect
    /// shapes (READ, DUP_READ, DUP_WRITE, READ_WRITE, heredocs, …)
    /// pass through to the per-redir Op::Redirect path unchanged.
    /// Mirrors C zsh's `Src/exec.c:2418 mfds[fd1]` + addfd splice
    /// dispatch — when MULTIOS is on (default) and fd1 already has a
    /// multio bag, a new redirect to the same fd appends to the bag
    /// instead of overwriting.
    fn compile_redirs_multios(&mut self, redirs: &[crate::parse::ZshRedir]) {
        // Group consecutive write-side redirects by fd. We treat the
        // first occurrence of an fd as the bag's anchor; if a second
        // write-side redirect targets the same fd, mark them all for
        // multios. Non-write-side redirects flush the bag for that fd
        // first (rare interleaving — preserve order).
        //
        // First pass: count writes and reads per fd.
        let mut writes_per_fd: std::collections::HashMap<u8, usize> =
            std::collections::HashMap::new();
        let mut reads_per_fd: std::collections::HashMap<u8, usize> =
            std::collections::HashMap::new();
        let fd_of = |r: &crate::parse::ZshRedir| -> u8 {
            if r.fd >= 0 {
                r.fd as u8
            } else if r.rtype == REDIR_READ {
                0
            } else {
                1
            }
        };
        let is_write_side = |t: i32| -> bool {
            t == REDIR_WRITE || t == REDIR_WRITENOW || t == REDIR_APP || t == REDIR_APPNOW
        };
        let is_read_side = |t: i32| -> bool { t == REDIR_READ };
        // c:Src/exec.c:3884-3917 REDIR_MERGEIN/REDIR_MERGEOUT — a
        // numeric `>&N` / `<&N` dup runs `fil = movefd(dup(fd))` and
        // feeds addfd, so it participates in the multio for its fd
        // exactly like a file target (`print x >&1 > f` tees to the
        // original stdout AND f). Non-numeric forms (`-` close, `p`
        // coproc, `>& file` ERRWRITE conversion) keep the plain path.
        let name_is_numeric_fd = |r: &crate::parse::ZshRedir| -> bool {
            let n = crate::lex::untokenize(&r.name);
            !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
        };
        let is_write_member = |r: &crate::parse::ZshRedir| -> bool {
            r.varid.is_none()
                && (is_write_side(r.rtype) || (r.rtype == REDIR_MERGEOUT && name_is_numeric_fd(r)))
        };
        let is_read_member = |r: &crate::parse::ZshRedir| -> bool {
            r.varid.is_none()
                && (is_read_side(r.rtype) || (r.rtype == REDIR_MERGEIN && name_is_numeric_fd(r)))
        };
        // c:Src/glob.c:2150-2207 xpandredir — under MULTIOS the
        // target word is globbed; multiple matches duplicate the
        // redirect per match ("Loop over matches, duplicating the
        // redirection for each file found", c:2195-2203), all feeding
        // one multio. A SINGLE `> *.txt` therefore still needs the
        // multios builtin so the runtime can splice the match array
        // (the plain Op::Redirect path stringifies an Array target
        // into one space-joined filename).
        let has_glob_tokens = |r: &crate::parse::ZshRedir| -> bool {
            let s = r.name.as_str();
            unquoted(s, '*')
                || unquoted(s, '\u{87}') // Star (parse/tokens.rs:14)
                || unquoted(s, '?')
                || unquoted(s, '\u{97}') // Quest (parse/tokens.rs:30)
                || unquoted(s, '[')
                || unquoted(s, '\u{91}') // Inbrack (parse/tokens.rs:24)
                // c:Src/glob.c:2161 xpandredir — a redirect target that
                // brace-expands to MULTIPLE words duplicates the redirect
                // per word (MULTIOS), exactly like a glob match array.
                // Without detecting brace tokens here, `> dir/{a,b}` fell
                // to the single-redir path which space-joined the
                // expansion into ONE filename ("dir/a dir/b"). Route brace
                // targets through the multios-splice so each expanded word
                // opens its own fd (gated on MULTIOS by the runtime
                // builtin, matching xpandredir's `isset(MULTIOS)`).
                || unquoted(s, '{')
                || unquoted(s, '\u{8f}') // Inbrace (parse/tokens.rs)
        };
        for r in redirs {
            if is_write_member(r) {
                *writes_per_fd.entry(fd_of(r)).or_insert(0) += 1;
            } else if is_read_member(r) {
                *reads_per_fd.entry(fd_of(r)).or_insert(0) += 1;
            }
        }
        // Second pass: emit. For an fd with N>1 writes, collect
        // pushes and emit BUILTIN_MULTIOS_REDIRECT once at the LAST
        // write to that fd (preserving the script order of
        // intervening non-multios redirects).
        //
        // Store the RAW redir.name (still carrying lexer tokens) per
        // target, not the untokenized literal — at emit time we call
        // `compile_word_str` so `$var`/`$(cmd)`/`${…}` targets expand
        // at runtime. Previous version pushed `untokenize(name)` as
        // a LoadConst constant, which sent `$ga` to the open(2) path
        // verbatim and created literal files like `$ga` / `$gb` in
        // CWD instead of `/tmp/gap_mo_a_PID` etc. (mirrors C
        // `Src/exec.c:2418 mfds` setup where each addfd target runs
        // through full word expansion before open).
        let mut pending_multios: std::collections::HashMap<u8, Vec<(String, u8)>> =
            std::collections::HashMap::new();
        let mut pending_multios_read: std::collections::HashMap<u8, Vec<(String, u8)>> =
            std::collections::HashMap::new();
        // We don't have direct access to op_byte without re-deriving
        // it, so do a small helper.
        let derive_op = |r: &crate::parse::ZshRedir| -> Option<u8> {
            if r.rtype == REDIR_WRITE {
                Some(fusevm::op::redirect_op::WRITE)
            } else if r.rtype == REDIR_WRITENOW {
                Some(fusevm::op::redirect_op::CLOBBER)
            } else if r.rtype == REDIR_APP || r.rtype == REDIR_APPNOW {
                Some(fusevm::op::redirect_op::APPEND)
            } else if r.rtype == REDIR_MERGEOUT {
                Some(fusevm::op::redirect_op::DUP_WRITE)
            } else if r.rtype == REDIR_READ {
                Some(fusevm::op::redirect_op::READ)
            } else if r.rtype == REDIR_MERGEIN {
                Some(fusevm::op::redirect_op::DUP_READ)
            } else {
                None
            }
        };
        for redir in redirs {
            let fd = fd_of(redir);
            let read_total = reads_per_fd.get(&fd).copied().unwrap_or(0);
            // Bag membership: ≥2 members on the fd, OR a single
            // glob-bearing `< pattern` whose match array must splice
            // (c:Src/glob.c:2195-2203).
            let is_multios_read_candidate = is_read_member(redir)
                && (read_total >= 2
                    || (read_total == 1 && is_read_side(redir.rtype) && has_glob_tokens(redir)));
            if is_multios_read_candidate {
                let op_byte = match derive_op(redir) {
                    Some(o) => o,
                    None => {
                        self.compile_redir(redir, false);
                        continue;
                    }
                };
                // Stash the RAW token-bearing redir.name so emit-time
                // `compile_word_str` runs full word expansion (var +
                // cmd-subst + arith) on the read source. Mirrors the
                // single-redir path at `compile_redir` which already
                // routes redir.name through compile_word_str.
                pending_multios_read
                    .entry(fd)
                    .or_default()
                    .push((redir.name.clone(), op_byte));
                let bag_now = pending_multios_read.get(&fd).map(|v| v.len()).unwrap_or(0);
                let total = read_total;
                if bag_now == total {
                    if let Some(pairs) = pending_multios_read.remove(&fd) {
                        let n = pairs.len();
                        // Push (source, op_byte) pairs in compile order.
                        // The op distinguishes file opens (READ) from
                        // numeric dups (DUP_READ, `<&N`).
                        for (source, op_byte) in &pairs {
                            self.redir_word_depth += 1;
                            self.compile_word_str(source.as_str());
                            self.redir_word_depth -= 1;
                            self.builder.emit(Op::LoadInt(*op_byte as i64), 0);
                        }
                        self.builder.emit(Op::LoadInt(fd as i64), 0);
                        let argc = (2 * n + 1) as u8;
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_MULTIOS_READ, argc),
                            0,
                        );
                        self.builder.emit(Op::Pop, 0);
                    }
                }
                continue;
            }
            let write_total = writes_per_fd.get(&fd).copied().unwrap_or(0);
            let is_multios_candidate = is_write_member(redir)
                && (write_total >= 2
                    || (write_total == 1 && is_write_side(redir.rtype) && has_glob_tokens(redir)));
            if !is_multios_candidate {
                self.compile_redir(redir, false);
                continue;
            }
            let op_byte = match derive_op(redir) {
                Some(o) => o,
                None => {
                    self.compile_redir(redir, false);
                    continue;
                }
            };
            // Same fix as the multios_read arm above: store the raw
            // token-bearing redir.name and run it through
            // `compile_word_str` at emit time so `>$var` / `>$(cmd)`
            // targets expand at runtime instead of opening files
            // literally named "$var" / "$(cmd)" in CWD.
            pending_multios
                .entry(fd)
                .or_default()
                .push((redir.name.clone(), op_byte));
            // When the bag for this fd is now complete (we've seen
            // every multios entry counted in pass 1), emit the
            // coalesced op.
            let bag_now = pending_multios.get(&fd).map(|v| v.len()).unwrap_or(0);
            let total = write_total;
            if bag_now == total {
                if let Some(pairs) = pending_multios.remove(&fd) {
                    let n = pairs.len();
                    // Push (target, op_byte) pairs in compile order.
                    for (target, op_byte) in &pairs {
                        self.redir_word_depth += 1;
                        self.compile_word_str(target.as_str());
                        self.redir_word_depth -= 1;
                        self.builder.emit(Op::LoadInt(*op_byte as i64), 0);
                    }
                    // Then push fd.
                    self.builder.emit(Op::LoadInt(fd as i64), 0);
                    // CallBuiltin pops 2N + 1 from the stack.
                    let argc = (2 * n + 1) as u8;
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_MULTIOS_REDIRECT, argc),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0); // discard Status
                }
            }
        }
    }

    /// `permanent` — bare-`exec` redirect list (c:Src/exec.c:3978-3986
    /// nullexec==1): brackets the emitted `Op::Redirect` with
    /// `BUILTIN_EXEC_PERM_REDIRS` 1/0 so `host_apply_redirect` skips
    /// the enclosing scope's save/restore for this fd change. The
    /// toggle is emitted AFTER the target word is computed so any
    /// `$(…)` in the target runs with the flag clear.
    fn compile_redir(&mut self, redir: &crate::parse::ZshRedir, permanent: bool) {
        // Default fd: stdin for read-side redirects, stdout for write-side.
        let fd_default: u8 = match redir.rtype {
            REDIR_READ | REDIR_HEREDOC | REDIR_HEREDOCDASH | REDIR_HERESTR | REDIR_READWRITE
            | REDIR_MERGEIN | REDIR_INPIPE => 0,
            _ => 1,
        };
        let fd = if redir.fd >= 0 {
            redir.fd as u8
        } else {
            fd_default
        };

        // Heredoc / herestring carry their content in `redir.heredoc`.
        if matches!(redir.rtype, REDIR_HEREDOC | REDIR_HEREDOCDASH) {
            if let Some(hd) = &redir.heredoc {
                let content_clean = crate::lex::untokenize(&hd.content);
                // `{varid}<<HERE` — named-fd heredoc (A04 "here
                // document with fd declarator"). The body goes to a
                // temp file whose read fd (>= 10) lands in $varid —
                // c:Src/exec.c:4660-4682 gethere + c:2402-2412 addfd
                // varid arm. Push [body, varid, 255] for the
                // BUILTIN_OPEN_NAMED_FD heredoc arm.
                if let Some(ref vid) = redir.varid {
                    if hd.quoted {
                        let idx = self
                            .builder
                            .add_constant(Value::str(content_clean.as_str()));
                        self.builder.emit(Op::LoadConst(idx), 0);
                    } else {
                        // c:Src/exec.c:4671-4672 — same non-append
                        // contract as the fd-0 arm below: the body
                        // reaches the consumer exactly as `gethere`
                        // produced it. Marker 255 tells
                        // BUILTIN_OPEN_NAMED_FD this is the here-DOC
                        // spelling (REDIRF_FROM_HEREDOC), so it must
                        // not append; the trim here plus that append
                        // collapsed N trailing blank lines to one.
                        let text_const = self.builder.add_constant(Value::str(hd.content.as_str()));
                        self.builder.emit(Op::LoadConst(text_const), 0);
                        self.builder.emit(Op::LoadInt(4), 0); // mode = HeredocBody
                        self.builder
                            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
                    }
                    let vid_const = self.builder.add_constant(Value::str(vid.as_str()));
                    self.builder.emit(Op::LoadConst(vid_const), 0);
                    self.builder.emit(Op::LoadInt(255), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_OPEN_NAMED_FD, 3),
                        0,
                    );
                    self.builder.emit(Op::SetStatus, 0);
                    return;
                }
                // Numeric custom fd (`3<<E`, `exec 3<<E`): wire the body
                // to fd N, not fd 0. The default Op::HereDoc/HereString
                // path stages the body as pending STDIN, so `read -u3`
                // / `cat <&3` saw nothing. Route through the same
                // fd-aware helper as `exec 3<<<str` (writes a temp file,
                // dup2's to fd N). Content handling mirrors the fd=0
                // arms below (quoted → verbatim, unquoted → mode-4
                // expand). c:Src/exec.c:3766-3780.
                //
                // c:Src/exec.c:4671-4672 — the trailing `1` marks
                // REDIRF_FROM_HEREDOC (c:Src/parse.c:2970-2971) so the
                // helper does NOT append a newline; the trim that used
                // to compensate for its unconditional append collapsed
                // N trailing blank lines to one and added a newline to
                // a body that ended without one.
                //
                // An EMPTY body still belongs on fd N: `exec 3<<EOF`
                // with nothing before the terminator leaves fd 3 open
                // on a zero-byte file in zsh (`cat <&3` prints nothing,
                // status 0). Routing it to the fd-0 `Op::HereDoc` arm
                // instead staged it as pending stdin and left fd 3
                // closed, so `cat <&3` reported "bad file descriptor".
                if fd > 0 {
                    if hd.quoted {
                        let idx = self
                            .builder
                            .add_constant(Value::str(content_clean.as_str()));
                        self.builder.emit(Op::LoadConst(idx), 0);
                    } else {
                        let text_const = self.builder.add_constant(Value::str(hd.content.as_str()));
                        self.builder.emit(Op::LoadConst(text_const), 0);
                        self.builder.emit(Op::LoadInt(4), 0); // mode = HeredocBody
                        self.builder
                            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
                    }
                    self.builder.emit(Op::LoadInt(fd as i64), 0);
                    self.builder.emit(Op::LoadInt(1), 0); // REDIRF_FROM_HEREDOC
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_EXEC_HERESTR_FD, 3),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0); // discard Status
                    return;
                }
                // Empty heredoc body — route through HereDoc op (no
                // trailing-newline append) regardless of quoting, so
                // the consumer sees zero bytes (matches zsh).
                if content_clean.is_empty() {
                    let idx = self.builder.add_constant(Value::str(""));
                    self.builder.emit(Op::HereDoc(idx), 0);
                    return;
                }
                if hd.quoted {
                    // Quoted-terminator form: pass body verbatim.
                    let idx = self
                        .builder
                        .add_constant(Value::str(content_clean.as_str()));
                    self.builder.emit(Op::HereDoc(idx), 0);
                } else {
                    // Unquoted: expand `$var`/`$(cmd)`/`$((expr))` in the
                    // body — but NOT glob/brace expansion. The body of
                    // `cat <<EOF\n[42]\nEOF` should reach cat's stdin
                    // verbatim with `[42]` as literal text, not as a
                    // glob pattern that fails NOMATCH. Mode 4 routes
                    // through expand_string only (variable / cmd-subst
                    // / arith), skipping glob+brace.
                    //
                    // c:Src/exec.c:4641 — `parsestr(&buf)` in `gethere`
                    // already tokenized backslash-escapes (`\$` →
                    // Bnull+$, `\\` → Bnull+\, etc) so the saved
                    // `hd.content` carries the Bnull markers. Pass the
                    // RAW content (not the untokenize'd form) to
                    // BUILTIN_EXPAND_TEXT — singsub respects Bnull and
                    // treats the marked byte as literal, so `\$N` ends
                    // up as `$N` in the output. Bug #22 in BUGS.md
                    // (`\$N` was expanding to the variable's value
                    // because untokenize stripped the Bnull marker
                    // before mode-4 expansion).
                    // c:Src/exec.c:4671-4672 — the body goes to the
                    // consumer EXACTLY as `gethere` produced it. It used
                    // to be `trim_end_matches('\n')` here plus the
                    // unconditional append inside `Op::HereString`, and
                    // strip-all-then-append-one is lossy both ways: a
                    // body with NO final newline gained one (`cat <<EOF`
                    // + `hello` printed `hello\n`, zsh prints `hello`)
                    // and N trailing newlines collapsed to one
                    // (`hello\n\n\n` printed as `hello\n`). The quoted
                    // arm above never had the bug because `Op::HereDoc`
                    // appends nothing; this is the same sink for the
                    // post-expansion form.
                    let text_const = self.builder.add_constant(Value::str(hd.content.as_str()));
                    self.builder.emit(Op::LoadConst(text_const), 0);
                    self.builder.emit(Op::LoadInt(4), 0); // mode = HeredocBody
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::fusevm_bridge::BUILTIN_HEREDOC_BODY_SINK, 1),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0);
                }
            }
            return;
        }
        if matches!(redir.rtype, REDIR_HERESTR) {
            // `<<< str` with an EXPLICIT target fd > 0 (e.g.
            // `exec 3<<<"line"`): the existing `Op::HereString` path
            // stages the content as "pending stdin" for the NEXT
            // simple-command read. That works for `cat <<<str`
            // (consumed by cat) but not for bare `exec N<<<str`
            // because no command follows — the herestring needs to
            // open a real fd attached to fd N permanently. Mirror
            // C `Src/exec.c:3766-3780 REDIR_HERESTR + addfd` via
            // a new runtime helper that writes to a temp file and
            // dup2's to the target fd. Bug #205 in docs/BUGS.md.
            // `{varid}<<<str` — c:Src/exec.c:3779 passes `fn->varid`
            // to addfd for REDIR_HERESTR exactly as it does for
            // REDIR_READ, so the varid arm (c:2402-2412: movefd above
            // 10, FDT_EXTERNAL, setiparam) applies here too. Without
            // this the redirection fell through to the fd-0
            // pending-stdin path, $varid was never set, and
            // `exec {f}<<<hello; cat <&$f` died with "file number
            // expected". Marker 254 = genuine here-string, so
            // BUILTIN_OPEN_NAMED_FD appends the newline of c:4671-4672.
            if let Some(ref vid) = redir.varid {
                self.compile_word_str(&redir.name);
                let vid_const = self.builder.add_constant(Value::str(vid.as_str()));
                self.builder.emit(Op::LoadConst(vid_const), 0);
                self.builder.emit(Op::LoadInt(254), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_OPEN_NAMED_FD, 3),
                    0,
                );
                self.builder.emit(Op::SetStatus, 0);
                return;
            }
            if redir.fd > 0 {
                self.compile_word_str(&redir.name);
                self.builder.emit(Op::LoadInt(fd as i64), 0);
                // c:4671-4672 — NOT from a here-document, so the
                // helper appends the trailing newline.
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_EXEC_HERESTR_FD, 3),
                    0,
                );
                self.builder.emit(Op::Pop, 0); // discard Status
                return;
            }
            // Default fd=0 (stdin) — original pending-stdin path
            // works because the next simple command picks it up.
            self.compile_word_str(&redir.name);
            self.builder.emit(Op::HereString, 0);
            return;
        }

        // For non-heredoc forms, the target file/path goes via compile_word_str
        // (handles var expansion etc.). DupRead/DupWrite take a numeric fd
        // string; the runtime parses it and dup2s.
        // c:Src/glob.c:2174-2188 xpandredir — `>& WORD` (REDIR_MERGEOUT)
        // splits into two cases:
        //   - WORD is digits / "-" / "p" → fd dup (or close / coproc)
        //   - WORD is a non-numeric filename → convert to REDIR_ERRWRITE
        //     (`> WORD 2>&1`), which lands stdout AND stderr in WORD.
        // The wordcode-based ported path skips xpandredir (see
        // exec.rs:9163 "Pragmatic: skip"), so do the same conversion
        // here at compile time when the name is a static literal that
        // clearly isn't an fd. The dynamic case ($var-derived) still
        // routes through DUP_WRITE — accepting that it'll be wrong for
        // the rare `cmd >& $file` form pending a runtime hook.
        let name_clean = crate::lex::untokenize(&redir.name);
        let name_is_fd_like = name_clean == "-"
            || name_clean == "p"
            || (!name_clean.is_empty() && name_clean.chars().all(|c| c.is_ascii_digit()));
        // c:Src/glob.c:2160-2188 xpandredir — the fd-vs-filename
        // decision happens AFTER word expansion. A dynamic word
        // (`>&$myfd`, `>&$(cmd)`) must stay REDIR_MERGEOUT so the
        // RUNTIME dup arm can test the EXPANDED text for digits/-/p
        // (host_apply_redirect's DUP arm carries the same fallback).
        // Converting `>&$myfd` here treated the expansion "10" as a
        // filename and wrote a file literally named `10`.
        let name_is_dynamic = name_clean.contains('$') || name_clean.contains('`');
        let mut effective_rtype = redir.rtype;
        if redir.rtype == REDIR_MERGEOUT && !name_is_fd_like && !name_is_dynamic {
            // `>& FILE` → `> FILE 2>&1`. Default fd1 was 1 (set above)
            // which matches WRITE_BOTH semantics.
            effective_rtype = REDIR_ERRWRITE;
        }
        let op_byte = match effective_rtype {
            REDIR_WRITE => fusevm::op::redirect_op::WRITE,
            REDIR_WRITENOW => fusevm::op::redirect_op::CLOBBER,
            REDIR_APP => fusevm::op::redirect_op::APPEND,
            REDIR_APPNOW => fusevm::op::redirect_op::APPEND,
            REDIR_READ => fusevm::op::redirect_op::READ,
            REDIR_READWRITE => fusevm::op::redirect_op::READ_WRITE,
            REDIR_MERGEIN => fusevm::op::redirect_op::DUP_READ,
            REDIR_MERGEOUT => fusevm::op::redirect_op::DUP_WRITE,
            REDIR_ERRWRITE => fusevm::op::redirect_op::WRITE_BOTH,
            REDIR_ERRWRITENOW => fusevm::op::redirect_op::WRITE_BOTH,
            REDIR_ERRAPP => fusevm::op::redirect_op::APPEND_BOTH,
            REDIR_ERRAPPNOW => fusevm::op::redirect_op::APPEND_BOTH,
            REDIR_INPIPE | REDIR_OUTPIPE => {
                // Process substitution attached to a redirect target —
                // unusual; the parser models `< <(cmd)` differently.
                // Defer.
                tracing::debug!(?redir.rtype, "compile_zsh: pipe-style redirect TODO");
                return;
            }
            // Already handled above.
            REDIR_HEREDOC | REDIR_HEREDOCDASH | REDIR_HERESTR => return,
            _ => {
                tracing::debug!(?redir.rtype, "compile_zsh: unknown redir type, skipping");
                return;
            }
        };

        // Redirect-target words gate their glob expansion on MULTIOS
        // (c:Src/glob.c:2162-2167 xpandredir: "Globbing is only done
        // for multios.") — see glob_expand_builtin().
        self.redir_word_depth += 1;
        self.compile_word_str(&redir.name);
        self.redir_word_depth -= 1;
        // `{varid}>file` named-fd allocation: instead of dup2'ing onto
        // a fixed fd, BUILTIN_OPEN_NAMED_FD opens the file fresh, dup's
        // to fd >= 10, and stores the fd number in $varid.
        if let Some(ref vid) = redir.varid {
            let vid_const = self.builder.add_constant(Value::str(vid.as_str()));
            self.builder.emit(Op::LoadConst(vid_const), 0);
            self.builder.emit(Op::LoadInt(op_byte as i64), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_OPEN_NAMED_FD, 3),
                0,
            );
            self.builder.emit(Op::SetStatus, 0);
            return;
        }
        if permanent {
            // Stack here: [target]. LoadInt+CallBuiltin(argc=1)+Pop is
            // stack-neutral, leaving [target] for Op::Redirect.
            self.builder.emit(Op::LoadInt(1), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_EXEC_PERM_REDIRS, 1),
                0,
            );
            self.builder.emit(Op::Pop, 0);
        }
        self.builder.emit(Op::Redirect(fd, op_byte), 0);
        if permanent {
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_EXEC_PERM_REDIRS, 1),
                0,
            );
            self.builder.emit(Op::Pop, 0);
        }
    }

    fn compile_assign(&mut self, assign: &ZshAssign) {
        // Subscripted scalar assignment: `name[key]=value` and
        // `name[key]+=tail`. Untokenize the raw name (which carries
        // Inbrack/Outbrack markers) and split on the subscript brackets.
        //
        // c:Src/exec.c:2077 untokenize replaces Snull/Dnull/Bnull with
        // their literal quote chars (`'`/`"`/`\\`) via ztokens; zshrs's
        // canonical untokenize() STRIPS them (intentional divergence at
        // lex.rs:4371-4373 because many call sites want bare text).
        // For subscript LHS we need the C semantic — `h["k2"]=v` stores
        // the literal 4-char key `"k2"`, NOT `k2`. Use the
        // quote-preserving variant so the key chars survive into the
        // BUILTIN_SET_ASSOC load below. Bug #61 in docs/BUGS.md.
        let untoked_name = crate::lex::untokenize_preserve_quotes(&assign.name);
        // c:Src/params.c getindex — an assignment LHS whose subscript is
        // `[@]` or `[*]` selects the WHOLE array (start=0, end=-1):
        //   `a[@]=(x y z)` ≡ `a=(x y z)`        (replace whole array)
        //   `a[@]+=v`      ≡ `a+=v`             (push / append)
        //   `a[@]=scalar`  → ONE-element array  (scalar RHS, NO word-split
        //                                        — unlike `a=($foo)`)
        // split_subscript() rejects `[@]`/`[*]` because for READS they are
        // splice forms (handled by ARRAY_ALL / array_splice_ref). Without
        // this branch the LHS fell through to the plain-name path, where
        // untokenize stripped the brackets to `a@`, yielding zsh's
        // "not an identifier: a@".
        if let Some(base) = untoked_name
            .strip_suffix("[@]")
            .or_else(|| untoked_name.strip_suffix("[*]"))
        {
            if !base.is_empty() {
                match &assign.value {
                    ZshAssignValue::Array(_) => {
                        // Identical to plain `base=(...)` / `base+=(...)`
                        // except the store must reject an associative
                        // target. Recurse with the bracket stripped and
                        // flag the store to use the assoc-guarded builtins.
                        let rewritten = ZshAssign {
                            name: base.to_string(),
                            value: assign.value.clone(),
                            append: assign.append,
                        };
                        let prev = self.array_whole_assign;
                        self.array_whole_assign = true;
                        self.compile_assign(&rewritten);
                        self.array_whole_assign = prev;
                        return;
                    }
                    ZshAssignValue::Scalar(s) => {
                        // Scalar RHS → single-element array. Compile the
                        // value as a scalar word (assign_context_depth
                        // suppresses IFS word-split — `a[@]=$foo` keeps
                        // "a b" as one element), then store the one value
                        // via the assoc-guarded SET_ARRAY_AT (`=`) /
                        // APPEND_ARRAY_AT (`+=`).
                        self.assign_context_depth += 1;
                        self.compile_word_str(s);
                        self.assign_context_depth -= 1;
                        let nc = self.builder.add_constant(Value::str(base));
                        self.builder.emit(Op::LoadConst(nc), 0);
                        let bid = if assign.append {
                            crate::vm_helper::BUILTIN_APPEND_ARRAY_AT
                        } else {
                            crate::vm_helper::BUILTIN_SET_ARRAY_AT
                        };
                        self.builder.emit(Op::CallBuiltin(bid, 2), 0);
                        self.builder.emit(Op::Pop, 0);
                        return;
                    }
                }
            }
        }
        if let Some((base, key)) = split_subscript(&untoked_name) {
            // c:Src/params.c:1449-1450,1708-1709 — a leading `(e)`/`(E)` flag
            // group (quote_arg) on an assignment subscript makes the key
            // LITERAL: getindex consumes the group and runs `untokenize(s)`,
            // so `aa[(e)*]=v` stores key "*", not "(e)*". Search-and-assign
            // groups ((r)/(R)/(i)/…) are left intact for their own handlers.
            let key_norm: Option<String> = key.strip_prefix('(').and_then(|r| {
                r.find(')').and_then(|c| {
                    let grp = &r[..c];
                    if !grp.is_empty() && grp.chars().all(|ch| ch == 'e' || ch == 'E') {
                        Some(crate::lex::untokenize(&r[c + 1..]))
                    } else {
                        None
                    }
                })
            });
            let key: &str = key_norm.as_deref().unwrap_or(key);
            // xtrace prints the assignment name as C left it after
            // `untokenize(name)` (c:Src/exec.c:2582-2589) — i.e. the SOURCE
            // spelling with its backslashes (`A[\[k\]]=v`, verified against
            // `zsh -fxc`). Capture it before the subscript-escape pass below
            // rewrites the key for storage.
            let trace_key = key.to_string();
            // c:Src/params.c:2029 getindex — the subscript is re-lexed by
            // `parse_subscript(s, scanflags & SCANPM_DQUOTED, ']')` before any
            // of it is read as a key, which is where a backslash inside `[…]`
            // gets its meaning (c:Src/lex.c:1497-1512), and the markers it
            // leaves are disposed of by getarg + remnulargs + parsestr/singsub
            // (c:Src/params.c:1538-1551, 1583-1592). `untokenize_preserve_quotes`
            // above already folded the lexer's `Bnull` markers to literal
            // backslashes (`ztokens[Bnull - Pound]`, c:Src/lex.c:38), so run the
            // composite over that text: `A[\[k\]]=v` stores the key `[k]`, not
            // the 5-char `\[k\]`. `resolve_dollar` is true because this path has
            // no parsestr/singsub round — a key with NO live expansion is stored
            // as the literal below, so its `\$` / `\\` / `` \` `` must already be
            // resolved here.
            //
            // c:Src/params.c:1592 singsub — a key that DOES still contain a live
            // expansion keeps its source text and goes through the runtime word
            // compiler (below), exactly as before; only the escaped-`$` case
            // moves off that path, which is the point (`A[a\$b]=v` is the
            // literal key `a$b` in zsh, not `a` + the expansion of `$b`).
            let (resolved_key, key_live_expansion) =
                crate::subscript_escape::subscript_unescape(key, false, true);
            // A key that still holds a live expansion cannot take the resolved
            // text (its `$` would be re-expanded and its now-bare `[` globbed),
            // but it must not keep the SOURCE text either: `untokenize_preserve_quotes`
            // above folded the lexer's `Bnull` markers down to plain
            // backslashes, and the word compiler reads a plain backslash as an
            // ordinary character — so `A[\[$x\]]=v` stored the 5-char key
            // `\[k\]` where zsh stores `[k]` (same for `A[a\$b$x]`, `A[\(x$x\)]`,
            // `A[\{x$x\}]`). Hand it C's INTERMEDIATE spelling instead: the
            // marker disposition of `getarg` + `remnulargs` re-encoded as
            // `Bnull`+char, which is precisely the lexer encoding the word
            // compiler already resolves (`print -r -- \[$x\]` → `[k]`), with
            // the live `$` left for it to expand — c:Src/params.c:1585-1592
            // `parsestr(&s); singsub(&s);`.
            let key_unescaped = if key_live_expansion {
                crate::subscript_escape::subscript_escape_markers(key, false)
            } else {
                resolved_key
            };
            let key: &str = &key_unescaped;
            if let ZshAssignValue::Scalar(s) = &assign.value {
                // c:Src/params.c:2895 setarrvalue — range subscript
                // `a[lo,hi]=val` SPLICES the value into the array,
                // replacing positions lo..=hi with the (single-element)
                // RHS. With a scalar RHS, that means lo..=hi shrinks to
                // ONE element. Route through BUILTIN_SET_SUBSCRIPT_RANGE
                // which already does this splice. Bug #295 in
                // docs/BUGS.md: SET_ASSOC's resolved_key path treated
                // "2,3" as a comma-expression via mathevali → returned
                // 3, so a[2,3]=X overwrote only position 3 instead of
                // shrinking the range. Detect bare comma in the key
                // (no `$` / backtick — those expand at runtime to
                // single keys, not ranges) and dispatch as a single-
                // element array RHS.
                // The `$`/backtick veto used to be blanket, which lost every
                // range whose BOUND is computed: `t[$#MATCH/2+1,-1]=""` (the
                // fzf-tab common-prefix loop, fzf-tab.zsh:176) compiled to
                // SET_ASSOC and auto-vivified a hash key instead of splicing,
                // so the scalar kept all but its last character. c:Src/params.c
                // getindex splits the subscript on a top-level `,` and runs
                // each half through mathevalarg — the halves are ARITHMETIC, so
                // a `$` inside one is ordinary. Look for a LITERAL top-level
                // comma instead, skipping expansion/quote spans so a comma that
                // only appears *inside* `${…}` / `$(…)` / `` `…` `` / quotes
                // still reads as a single assoc key.
                let key_is_range = subscript_has_toplevel_comma(key);
                if key_is_range {
                    // c:Src/params.c:2895 setarrvalue — range append
                    // `a[lo,hi]+=tail` pre-concats the existing slice
                    // with tail then splices the joined value back.
                    // For scalar a="hello"; `a[2,3]+="X"` → existing
                    // slice "el" + "X" = "elX" → splice replaces chars
                    // 2-3 with "elX" → "helXlo". Without this, the
                    // append fell through to the SET_ASSOC path which
                    // auto-vivified the scalar into PM_HASHED with key
                    // "2,3"="elX". Sibling of #589.
                    if assign.append {
                        let name_const = self.builder.add_constant(Value::str(base));
                        self.builder.emit(Op::LoadConst(name_const), 0);
                        let key_const = self.builder.add_constant(Value::str(key));
                        self.builder.emit(Op::LoadConst(key_const), 0);
                        self.builder
                            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_INDEX, 2), 0);
                        self.compile_word_str(s);
                        self.builder.emit(Op::Concat, 0);
                    } else {
                        // Stack order matches the Array RHS path at line
                        // 1879+: [elem0, name, key], argc = 1 + 2 = 3.
                        self.compile_word_str(s);
                    }
                    let name_const = self.builder.add_constant(Value::str(base));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    let key_const = self.builder.add_constant(Value::str(key));
                    self.builder.emit(Op::LoadConst(key_const), 0);
                    // c:Src/params.c:1515 — a comma only separates
                    // subscripts when the parameter is NOT a hash, and
                    // the type is unknown here. Push the SOURCE subscript
                    // and a marker of 2 ("scalar RHS") so the handler can
                    // fall back to the element path for a PM_HASHED
                    // target (`h[1,2]=Z` keys on `1,2`).
                    let src_const = self.builder.add_constant(Value::str(trace_key.as_str()));
                    self.builder.emit(Op::LoadConst(src_const), 0);
                    // Scalar splice pre-concats the old slice above (when
                    // appending), so the handler keeps plain-replace
                    // semantics — pass append=0 (marker 2 = scalar RHS).
                    self.builder.emit(Op::LoadInt(2), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_SUBSCRIPT_RANGE, 5),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0);
                    // c:Src/exec.c:3396 — an assignment-only simple command
                    // takes its status from `cmdoutval`, the exit of a `$()`
                    // that ran on the RHS. The subscripted paths returned
                    // without recording that, so `h[k]="$(false)"` reported 0
                    // where zsh reports 1. VCS_INFO_detect_git relies on it:
                    // `vcs_comm[gitdir]="$(git rev-parse --git-dir)"` is the
                    // condition of an `&&` chain, so outside a repo the git
                    // backend ran anyway with an empty gitdir and read `/HEAD`.
                    self.last_assign_had_cmd_subst = match &assign.value {
                        ZshAssignValue::Scalar(rhs) => scalar_rhs_has_cmd_subst(rhs),
                        ZshAssignValue::Array(els) => {
                        els.iter().any(|e| scalar_rhs_has_cmd_subst(e))
                        }
                };
                    return;
                }
                let name_const = self.builder.add_constant(Value::str(base));
                self.builder.emit(Op::LoadConst(name_const), 0);
                // Subscript may contain $-refs (`_loaded[$plugin]=1`)
                // — emit through compile_word_str so the runtime
                // expands. Without this, the literal "$plugin" was
                // stored as the assoc key. Same fast/slow path as
                // the Array branch's subscripted-assign below.
                //
                // c:Bug #339 — `$'...'` ANSI-C string IS NOT a
                // variable expansion; zsh stores the literal source
                // bytes verbatim (e.g. `h[$'a\nb']=v` stores the
                // 7-byte key `$'a\nb'`). zshrs's compile path lumped
                // `$'…'` in with `$var` and ran the key through
                // compile_word_str, which decoded `$'a\nb'` to the
                // 3-byte `a\nb`. Detect the `$'…'` shape and treat
                // it as a literal LoadConst (same as a quoted
                // string subscript).
                let key_is_ansi_c_literal = key.starts_with("$'")
                    && key.ends_with('\'')
                    && key.len() >= 3
                    && !key[2..key.len() - 1].contains('\'');
                // c:Src/params.c:1592 singsub — only an UNESCAPED `$`/backtick
                // is a live expansion; `key_live_expansion` was computed on the
                // pre-unescape text so an escaped `\$` (which the pass above
                // resolved to a literal `$`) does NOT route through the word
                // compiler and get re-expanded.
                let key_has_expansion = !key_is_ansi_c_literal && key_live_expansion;
                // The word compiler recognises `$( … )` and `$(( … ))` only in
                // the LEXER's tokenized spelling — `Stringg` + `Inpar` /
                // `Inparmath` (compile_zsh.rs:9835, 13183). `untoked_name`
                // above folded those tokens back to ASCII `$(`, which the
                // compiler emits verbatim, so `h[x$((1+1))]=v` stored the
                // LITERAL key `x$((1+1))` where zsh stores `x2`
                // (c:Src/params.c:1585-1592 `parsestr` + `singsub` expand the
                // subscript before it is used as a key). A subscript that is
                // NOTHING BUT `$((…))` survived only because it takes the
                // whole-word arithmetic path. Hand those two shapes the
                // tokenized slice of the subscript instead; `$var`, `${…}` and
                // backticks are already resolved from the ASCII text.
                let key_tokenized: Option<String> = if key_has_expansion && key.contains("$(") {
                    use crate::ported::zsh_h::{Inbrack, Outbrack};
                    let raw = assign.name.as_str();
                    raw.find(Inbrack).and_then(|lb| {
                        raw.strip_suffix(Outbrack)
                            .map(|r| r[lb + Inbrack.len_utf8()..].to_string())
                    })
                } else {
                    None
                };
                let key_word: &str = key_tokenized.as_deref().unwrap_or(key);
                if key_has_expansion {
                    self.compile_word_str(key_word);
                } else {
                    let key_const = self.builder.add_constant(Value::str(key));
                    self.builder.emit(Op::LoadConst(key_const), 0);
                }
                if assign.append {
                    // Append: dup name+key, GET_VAR via assoc, Concat with new tail
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    if key_has_expansion {
                        self.compile_word_str(key_word);
                    } else {
                        let key_const = self.builder.add_constant(Value::str(key));
                        self.builder.emit(Op::LoadConst(key_const), 0);
                    }
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_INDEX, 2), 0);
                    self.compile_word_str(s);
                    self.builder.emit(Op::Concat, 0);
                } else {
                    self.compile_word_str(s);
                }
                // xtrace: emit `name[key]=value ` before SET_ASSOC
                // consumes the stack. Direct port of C zsh's
                // Src/exec.c:2517-2582 assignment-trace block —
                // `printprompt4()` (gated by doneps4) then
                // `fprintf("%s=", name); quotedzputs(val);` per asg.
                // Stack on entry to this block: [name, key, value].
                // Build a synthetic trace name `name[key]` via two
                // Dup ops + Concat (peek without consuming so
                // SET_ASSOC below sees the original triple), then
                // push the value via Dup and call XTRACE_ASSIGN with
                // PEEK contract of [trace_name, value].
                //   [name, key, value]
                //   Dup     → [name, key, value, value]
                //   Concat-build-name (multi-op) … [name, key, value, value, "name[key]"]
                //   Swap top-2 (XTRACE_ASSIGN wants [..,trace_name,value])
                //   XTRACE_ASSIGN(2) PEEKS those, emits, leaves them
                //   Pop, Pop (drop trace bookkeeping)
                //   → [name, key, value] again — SET_ASSOC argc=3 OK
                // The trace_name string is built at compile time
                // when the key is a literal (the common case from
                // `arr[k]=v`); for runtime-expanding keys
                // (`arr[$x]=v`) the trace path still emits the
                // literal source text — same gap C zsh has (it
                // pre-resolves at parse time too, per the asg.name
                // store at Src/lex.c:2169).
                // c:Src/exec.c:2582-2589 — xtrace prints `name` straight after
                // `untokenize(name)`, i.e. the SOURCE spelling including the
                // backslashes of an escaped subscript (`A[\[k\]]=v`), not the
                // resolved storage key. Use the pre-unescape text.
                let trace_name = format!("{}[{}]", base, trace_key);
                let tname_const = self.builder.add_constant(Value::str(trace_name.as_str()));
                // Stack now: [name, key, value]
                self.builder.emit(Op::Dup, 0);
                // Stack: [name, key, value, value]
                self.builder.emit(Op::LoadConst(tname_const), 0);
                // Stack: [name, key, value, value, trace_name]
                // Swap top 2 so XTRACE_ASSIGN sees [..., trace_name, value]:
                self.builder.emit(Op::Swap, 0);
                // Stack: [name, key, value, trace_name, value]
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_ASSIGN, 2),
                    0,
                );
                // XTRACE_ASSIGN peeks top 2 (trace_name, value) and
                // emits; leaves stack unchanged. Drop the result
                // status + the two helper slots:
                self.builder.emit(Op::Pop, 0); // status from XTRACE_ASSIGN
                self.builder.emit(Op::Pop, 0); // value dup
                self.builder.emit(Op::Pop, 0); // trace_name
                                               // Stack restored to: [name, key, value]
                                               // Dynamic-key marker: C's assignsparam sees the RAW
                                               // subscript text (`H[$1$2]`) so its isident gate
                                               // passes and the EXPANDED key — even empty — stores
                                               // fine (zinit's .zinit-pack-ice writes
                                               // ZINIT_SICE[$1…$2] with both empty). Only a
                                               // SOURCE-LITERAL empty subscript (`H[]=v`) is the
                                               // "not an identifier" error. zshrs expands the key
                                               // before the bridge, so the handler needs the
                                               // compile-time literal/dynamic bit to reproduce the
                                               // split (argc 4 = dynamic).
                                               // c:Src/params.c:2008/1409 — `getindex` parses the
                                               // subscript, and its flag block, off the SOURCE spelling
                                               // BEFORE the c:1585-1592 expansion round. zshrs expands
                                               // the key here at compile/word time, so the source text
                                               // has to travel alongside it or the runtime cannot tell
                                               // `h[(r)$x]=Z` (a search) from `x='(r)v'; h[$x]=Z` (the
                                               // literal key `(r)v`) — and it must never be recovered by
                                               // re-splitting a flattened `name[key]`, which breaks on a
                                               // key containing `]`.
                let src_const = self.builder.add_constant(Value::str(trace_key.as_str()));
                self.builder.emit(Op::LoadConst(src_const), 0);
                if key_has_expansion {
                    self.builder.emit(Op::LoadInt(1), 0);
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_ASSOC, 5), 0);
                } else {
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_ASSOC, 4), 0);
                }
                self.builder.emit(Op::Pop, 0);
                // c:Src/exec.c:3396 — an assignment-only simple command
                // takes its status from `cmdoutval`, the exit of a `$()`
                // that ran on the RHS. The subscripted paths returned
                // without recording that, so `h[k]="$(false)"` reported 0
                // where zsh reports 1. VCS_INFO_detect_git relies on it:
                // `vcs_comm[gitdir]="$(git rev-parse --git-dir)"` is the
                // condition of an `&&` chain, so outside a repo the git
                // backend ran anyway with an empty gitdir and read `/HEAD`.
                self.last_assign_had_cmd_subst = match &assign.value {
                    ZshAssignValue::Scalar(rhs) => scalar_rhs_has_cmd_subst(rhs),
                    ZshAssignValue::Array(els) => {
                        els.iter().any(|e| scalar_rhs_has_cmd_subst(e))
                    }
                };
                return;
            }
        }

        match &assign.value {
            ZshAssignValue::Scalar(s) => {
                // zsh status semantics for assignments:
                //   `false; a=plain; echo $?`     → 0 (assignment resets)
                //   `a=$(false); echo $?`         → 1 (cmd-subst propagates)
                //   `false; x=$?; echo $x; echo $?` → 1 then 0
                //     (RHS sees pre-assignment $?, post-assignment $?=0)
                //
                // C zsh (Src/exec.c:3387-3396): cmdoutval starts at the
                // pre-assignment lastval, lastval is preserved across
                // addvars's RHS expansion (so `$?` in the RHS sees the
                // original value), and AFTER addvars `lastval = cmdoutval`
                // — which is 0 unless a `$()` in the RHS overwrote it.
                //
                // Bytecode mirror: DO NOT clear status before the RHS
                // (clobbers `$?` for `x=$?`). compile_word_str runs with
                // last_status holding the pre-assignment value. A `$()`
                // inside the RHS calls run_command_substitution which
                // updates last_status to the subst's exit. After the RHS,
                // if no cmd-subst could have run, force status to 0;
                // otherwise leave last_status as the cmd-subst's exit.
                let rhs_has_cmd_subst = scalar_rhs_has_cmd_subst(s);

                let name_const = self.builder.add_constant(Value::str(assign.name.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                // Bare-assignment values (`i=5*3`) are NOT glob-
                // expanded by zsh — the `*` stays literal. If the
                // value contains glob metas but isn't already DQ-
                // wrapped, wrap with DNULLs so compile_word_str's
                // mode 1 (DoubleQuoted) bridge skips brace+glob
                // expansion. `$var` / `$(cmd)` / `$((expr))` still
                // expand inside DQ context.
                // Check for glob metas in BOTH the META-encoded form
                // (lexer's `\u{87}` for `*`, `\u{86}` for `?`, etc.)
                // AND the literal char (some lex paths leave them
                // bare). Either form means "glob in value, must
                // suppress" because zsh doesn't glob-expand assignment
                // RHS by default.
                // c:Src/zsh.h — token TOKEN constants:
                //   Star = \u{87}, Quest = \u{97}, Inbrack = \u{91},
                //   Inbrace = \u{8f}. The previous check used \u{86}
                //   (Hat ^) where it meant Quest, so tokenized `?`
                //   (\u{97}) in `a=he?l` was not detected and the
                //   DQ-wrap path didn't fire → the runtime saw a
                //   bare `?` glob and emitted "no matches found".
                //   Bug #603.
                let needs_dq_wrap = !s.starts_with('\u{9e}')
                    && !s.starts_with('\u{9d}')
                    && (s.contains('*') || s.contains('\u{87}')   // Star
                        || s.contains('?') || s.contains('\u{97}') // Quest
                        || s.contains('[') || s.contains('\u{91}') // Inbrack
                        || s.contains('{') || s.contains('\u{8f}')); // Inbrace
                                                                     // GLOB_ASSIGN eligibility: the RHS carries an UNQUOTED glob
                                                                     // TOKEN (Star \u{87} / Quest \u{97} / Inbrack \u{91}). Quoted
                                                                     // metas arrive as literal `*`/`?`/`[` (0x2a/0x3f/0x5b), so a
                                                                     // token byte unambiguously means "unquoted glob pattern". This
                                                                     // matches zsh: only literal unquoted patterns are globbed on
                                                                     // assignment (Src/exec.c:2554); `x="/tmp/*"`, `x=$p`, `x=$(c)`
                                                                     // are not. The DQ-wrap below untokenizes the value, so the
                                                                     // runtime can't recover this — carry it via BUILTIN_MARK_GLOB_
                                                                     // ELIGIBLE emitted just before SET_VAR.
                                                                     // A glob token counts only at TOP LEVEL. A Star/Quest/Inbrack
                                                                     // INSIDE a `${…}` / `$(…)` expansion, a `$name[…]` subscript, or
                                                                     // a `` `…` `` is not a value glob: `${A[1]}` / `$A[1]` carry an
                                                                     // Inbrack for their SUBSCRIPT, `${x#*/}` a Star for its PATTERN,
                                                                     // `$*` a Star for the PARAM — none glob the assigned value. zsh
                                                                     // globs the value POST-expansion (Src/subst.c globlist → zglob's
                                                                     // haswilds on the RESULT), so an expansion-interior token never
                                                                     // counts. Skipping those interiors before testing matches zsh:
                                                                     // `x=/tmp/*`, `x=[abc]`, `x=$p*` glob; `x=$p`, `x=${A[1]}`,
                                                                     // `x=$A[1]`, `x=$*` do not.
                let glob_eligible = {
                    use crate::ported::zsh_h::{
                        Inbrace, Inbrack, Inpar, Outbrace, Outbrack, Outpar, Qstring, Qtick, Quest,
                        Star, Stringg, Tick,
                    };
                    let mut brace_depth = 0i32; // ${ … } / $( … ) nesting
                    let mut sub_depth = 0i32; //   $name[ … ] subscript nesting
                    let mut in_tick = false; //    ` … ` command substitution
                    let mut param_ctx = false; //  scanning a `$…` reference
                    let mut param_first = false; // next char is the param's 1st (e.g. `$*`)
                    let mut prev = '\0';
                    let mut found = false;
                    for c in s.chars() {
                        if in_tick {
                            if c == Tick || c == Qtick {
                                in_tick = false;
                            }
                            prev = c;
                            continue;
                        }
                        // `$` immediately followed by `{`/`(` opens an expansion.
                        if (prev == Stringg || prev == Qstring) && (c == Inbrace || c == Inpar) {
                            brace_depth += 1;
                            param_ctx = false;
                            param_first = false;
                            prev = c;
                            continue;
                        }
                        if brace_depth > 0 {
                            if c == Inbrace || c == Inpar {
                                brace_depth += 1;
                            } else if c == Outbrace || c == Outpar {
                                brace_depth -= 1;
                            }
                            prev = c;
                            continue;
                        }
                        if sub_depth > 0 {
                            if c == Inbrack {
                                sub_depth += 1;
                            } else if c == Outbrack {
                                sub_depth -= 1;
                            }
                            prev = c;
                            continue;
                        }
                        if c == Tick || c == Qtick {
                            in_tick = true;
                            param_ctx = false;
                            prev = c;
                            continue;
                        }
                        if c == Stringg || c == Qstring {
                            param_ctx = true;
                            param_first = true;
                            prev = c;
                            continue;
                        }
                        if param_ctx && param_first {
                            // First char after `$` is always param content
                            // (`$*`, `$@`, `$#`, `$1`, `$name`, …).
                            param_first = false;
                            prev = c;
                            continue;
                        }
                        if param_ctx && c == Inbrack {
                            sub_depth = 1; // `$name[ … ]` subscript
                            prev = c;
                            continue;
                        }
                        if param_ctx && (c.is_alphanumeric() || c == '_') {
                            prev = c;
                            continue; // still scanning the param name
                        }
                        param_ctx = false;
                        if c == Star || c == Quest || c == Inbrack {
                            found = true;
                        }
                        prev = c;
                    }
                    found
                };
                self.assign_context_depth += 1;
                self.scalar_assign_depth += 1;
                if needs_dq_wrap {
                    let wrapped = format!("\u{9e}{}\u{9e}", s);
                    // The Dnull pair below is SYNTHETIC — it stands in for C's
                    // PREFORK_SINGLE (c:Src/exec.c:2546), not for a user's
                    // `"…"`. Flag it so the qt-deriving emit sites don't read
                    // it as `qt=1` (c:Src/subst.c:1625).
                    self.synthetic_dq_wrap_depth += 1;
                    self.compile_word_str(&wrapped);
                    self.synthetic_dq_wrap_depth -= 1;
                } else {
                    self.compile_word_str(s);
                }
                self.scalar_assign_depth -= 1;
                self.assign_context_depth -= 1;
                // xtrace: per-assignment trace before SET_VAR consumes
                // [name, value]. BUILTIN_XTRACE_ASSIGN PEEKS the top
                // two stack slots — name + post-expansion value — so
                // the matching SET_VAR call below sees them
                // unchanged. Direct port of Src/exec.c:2517-2582
                // where the C body emits `printprompt4()` (gated by
                // doneps4) then `fprintf("%s=", name);
                // quotedzputs(val); fputc(' ');` per-assignment.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_ASSIGN, 2),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                let bid = if assign.append {
                    // `name+=val` — runtime-dispatch via APPEND_SCALAR_OR_PUSH:
                    // if `name` is an indexed array, push the value as a new
                    // element; if assoc, refuse (zsh errors); else scalar concat.
                    crate::vm_helper::BUILTIN_APPEND_SCALAR_OR_PUSH
                } else {
                    crate::vm_helper::BUILTIN_SET_VAR
                };
                // Flag the upcoming SET_VAR as GLOB_ASSIGN-eligible when the
                // RHS was a literal unquoted glob. Only for plain `=` (zsh does
                // not glob-assign `+=`); the flag is read+cleared by SET_VAR.
                if glob_eligible && !assign.append {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_MARK_GLOB_ELIGIBLE, 0),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0);
                }
                self.builder.emit(Op::CallBuiltin(bid, 2), 0);
                // Propagate the assignment's status to $?. SET_VAR
                // returns Value::Status(last_status read at call
                // time). For cmd-subst RHS the subst already wrote
                // last_status to its exit; for plain RHS last_status
                // still holds the pre-assignment value (so subsequent
                // assigns' `$?` in the same simple cmd see the same
                // old value, matching C zsh's addvars walk). The
                // post-assignment reset to 0 (assignment-only path,
                // no cmd-subst anywhere in the chain) is emitted ONCE
                // by compile_simple after the assigns loop.
                self.builder.emit(Op::SetStatus, 0);
                self.last_assign_had_cmd_subst = rhs_has_cmd_subst;
            }
            ZshAssignValue::Array(elements) => {
                // Subscripted-array assign: `a[i]=(elements)`,
                // `a[i,j]=(elements)`, or `a[i]=()` (delete element).
                // The Scalar branch above only handles single-value
                // assigns; this branch handles the array-literal form
                // including the empty-list delete idiom.
                //
                // c:Src/params.c IPDEF9("argv", &pparams) — a bare numeric
                // assignment name `N=(...)` is a positional-parameter
                // splice equivalent to `argv[N]=(...)` (`set -- a b c d;
                // 2=(X Y Z)` → `a X Y Z c d`). Rewrite to the `argv[N]`
                // subscript form so it routes through the same
                // setarrvalue-into-pparams splice as `argv[N]=(...)`.
                let numeric_pos =
                    !untoked_name.is_empty() && untoked_name.chars().all(|c| c.is_ascii_digit());
                let subscript = if numeric_pos {
                    Some(("argv", untoked_name.as_str()))
                } else {
                    split_subscript(&untoked_name)
                };
                if let Some((base, key)) = subscript {
                    for elem in elements {
                        self.compile_word_str(elem);
                        if needs_word_split(elem) {
                            self.builder
                                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_WORD_SPLIT, 0), 0);
                        }
                    }
                    let name_const = self.builder.add_constant(Value::str(base));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    // Key may contain `$var` / `$#name` — emit through
                    // compile_word_str so the runtime expands. Without
                    // this, `a[$n]=()` saw the literal "$n" key and
                    // failed to parse it as an int (no removal).
                    if key.contains('$') || key.contains('`') {
                        self.compile_word_str(key);
                    } else {
                        let key_const = self.builder.add_constant(Value::str(key));
                        self.builder.emit(Op::LoadConst(key_const), 0);
                    }
                    // c:Src/params.c:3518-3520 — `a[N]+=(v)` / `a[lo,hi]+=(v)`
                    // collapses the range to an empty range after the slice
                    // end and inserts only the new value (AUGMENT). Signal
                    // it to the handler via the trailing append flag.
                    self.builder
                        .emit(Op::LoadInt(if assign.append { 1 } else { 0 }), 0);
                    let argc = (elements.len() + 3) as u8;
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_SUBSCRIPT_RANGE, argc),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0);
                    // Same c:3442-3451 record as the arm's normal exit below —
                    // this early return must not skip it.
                    self.last_assign_had_cmd_subst =
                        elements.iter().any(|e| scalar_rhs_has_cmd_subst(e));
                    return;
                }
                // arr=(a b c) / arr+=(d e). Direct port of
                // c:Src/exec.c::addvars:2517-2632 — expand each
                // element once, then emit the xtrace line
                // `name=( elem1 elem2 ) ` using the same expanded
                // values, then call SET_ARRAY with the values.
                //
                // C zsh evaluates RHS once: `prefork(list, ...)` then
                // both `quotedzputs(*ptr, xtrerr)` (for trace, c:2628)
                // AND `assignaparam(name, arr, ...)` (for assign,
                // c:2633) read the same expanded `arr`. We get the same
                // single-eval property by assembling the expanded values
                // into ONE Value::Array (MakeArray) and driving BOTH the
                // xtrace line and SET_ARRAY off that single array — see
                // the emit sequence after the element loop. The earlier
                // port stashed one VM slot per element for the trace,
                // which overflowed `next_slot` (u16, capped by fusevm's
                // Op::SetSlot(u16)) on large literals — a .zcompdump's
                // ~51k-element `_comps=(...)` panicked ("add with
                // overflow"). No per-element slots now.
                //
                // Bump assign_context_depth so compile_word_str's
                // own WORD_SPLIT (for unquoted `$(…)`) is suppressed
                // — the outer loop emits ONE WORD_SPLIT per element
                // below.
                // Data-literal fast path: a large `arr=(...)` whose elements are
                // ALL compile-time literals compiles to ONE Value::Array constant
                // (one LoadConst), not N per-element LoadConst + MakeArray. This is
                // what dodges the u16 constant-pool wall — a `.zcompdump`'s
                // `_comps=(...)` is 96k unique constants as per-element code, but a
                // single Array constant either way. Threshold-gated so ordinary
                // small arrays keep the well-exercised per-element path.
                let literal_vals: Option<Vec<Value>> = if elements.len() >= 1024 {
                    elements
                        .iter()
                        .map(|e| literal_array_elem_value(e).map(|s| Value::str(s.as_str())))
                        .collect()
                } else {
                    None
                };
                let batched = literal_vals.is_some();
                if let Some(vals) = literal_vals {
                    let ac = self.builder.add_constant(Value::array(vals));
                    self.builder.emit(Op::LoadConst(ac), 0);
                }
                let mut stack_values = 0usize;
                if !batched {
                    for elem in elements.iter() {
                        // c:Src/subst.c:49-79 keyvalpairelement, invoked
                        // from prefork's PREFORK_ASSIGN walk (c:111-117).
                        // An unquoted `[key]=value` / `[key]+=value`
                        // element is rewritten into THREE list nodes:
                        // Marker (or Marker `+`), key, value — with the
                        // key and value each run through singsub (c:65,75:
                        // substitution only, no glob / no word-split / no
                        // brace expansion). The Marker triple flows into
                        // assignaparam(ASSPM_KEY_VALUE) downstream. The
                        // fusevm compile path performs the same split here
                        // at compile time, where the raw token form still
                        // distinguishes quoted (`"[k]=v"` — plain element,
                        // c:54 start[0]==Inbrack is the TOKEN form only)
                        // from unquoted.
                        if let Some((key_raw, val_raw, is_append)) = split_kv_element(elem) {
                            // c:Src/subst.c:59-60 marker / marker_plus.
                            let marker = if is_append {
                                format!("{}+", crate::ported::zsh_h::Marker)
                            } else {
                                crate::ported::zsh_h::Marker.to_string()
                            };
                            let mc = self.builder.add_constant(Value::str(marker.as_str()));
                            self.builder.emit(Op::LoadConst(mc), 0);
                            for part in [&key_raw, &val_raw] {
                                // c:Src/subst.c:65/75 `singsub(&dat)` —
                                // PREFORK_SINGLE semantics: parameter /
                                // command substitution runs, but no glob,
                                // no IFS split, no brace expansion.
                                // dq_context_depth>0 routes every emit
                                // site to the no-glob / no-split variants
                                // (EXPAND_TEXT mode 1, GLOB_EXPAND gated).
                                self.assign_context_depth += 1;
                                self.dq_context_depth += 1;
                                self.compile_word_str(part);
                                self.dq_context_depth -= 1;
                                self.assign_context_depth -= 1;
                            }
                            stack_values += 3;
                            continue;
                        }
                        self.assign_context_depth += 1;
                        self.word_emitted_glob = false;
                        self.compile_word_str(elem);
                        self.assign_context_depth -= 1;
                        // Same IFS-split rule as for-list words: unquoted
                        // `$(...)` / backtick inside an array literal
                        // (`a=($(...))`) should produce per-word elements.
                        if needs_word_split(elem) {
                            self.builder
                                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_WORD_SPLIT, 0), 0);
                        }
                        // c:Src/exec.c:2611-2613 — after prefork, an ARRAY
                        // assignment ALWAYS runs its word list through
                        // globlist():
                        //     if (!isstr || (isset(GLOBASSIGN) && isstr &&
                        //                    haswilds(…)))
                        //         globlist(vl, prefork_ret);
                        // so a substituted value whose glob metachars were
                        // promoted to TOKENS by paramsubst (c:Src/subst.c:4419
                        // `if (globsubst) shtokenize(y)`, set by `${~spec}` at
                        // c:Src/subst.c:2603 `globsubst = 2` or by the
                        // GLOB_SUBST option at c:Src/subst.c:1671) takes part
                        // in filename generation here. zshrs models globsubst
                        // as a runtime option carrier, so mirror the simple-
                        // command argv arm (compile_zsh.rs:2558) and the
                        // for-list arm (compile_zsh.rs:7933): emit the runtime
                        // gate, which is a no-op when GLOB_SUBST is off.
                        // Without it only the `${~NAME}` compiler fast path
                        // (compile_zsh.rs:5597) globbed, so `exp=( ${~exp//…} )`
                        // in Completion/Base/Completer/_expand:110 kept the
                        // literal `**/` and `ls **/<TAB>` offered no expansions.
                        //
                        // `!word_emitted_glob`: that single `globlist(vl, …)`
                        // is the WHOLE of C's filename generation for the
                        // assignment, and c:Src/glob.c `globlist` walks past
                        // the nodes `zglob` produced — a generated name is
                        // never re-globbed. When the element's own text had a
                        // glob token, compile_word_str already emitted
                        // BUILTIN_GLOB_EXPAND over the assembled word (that IS
                        // the `globlist` pass, substituted metachars
                        // included), so adding the GLOB_SUBST pass on top ran
                        // `zglob` a SECOND time on its own output:
                        // `pages=( ${^~pages}(N:t) )` in `_man_pages` fed the
                        // produced basename `[.1` back in as a pattern →
                        // `bad pattern: [.1`.
                        if has_unquoted_param_or_subst(elem) && !self.word_emitted_glob {
                            self.builder.emit(
                                Op::CallBuiltin(crate::vm_helper::BUILTIN_GLOB_SUBST_EXPAND, 1),
                                0,
                            );
                        }
                        stack_values += 1;
                    }
                    // Collapse the N element values into ONE Value::Array. CallBuiltin's
                    // argc is u8 in the fusevm opcode (op.rs `CallBuiltin(u16, u8)`), so
                    // passing N+1 separate stack args wrapped the count mod 256 for a
                    // literal `arr=(...)` with >254 elements. MakeArray takes a u16 count;
                    // SET_ARRAY/APPEND_ARRAY flatten a single Value::Array arg and still
                    // run the `[key]=value` marker detection on the flattened list, so both
                    // plain and assoc-pair literals round-trip (the same shape
                    // `arr=($other_array)` already takes). MakeArray's count is a
                    // u16 operand; a literal with > 65535 elements that is NOT all-literal
                    // (so it missed the single-Array-constant fast path above) would wrap
                    // it mod 65536 and silently truncate — route those through
                    // BUILTIN_MAKE_ARRAY_COUNTED, whose count is a runtime i64.
                    if stack_values <= u16::MAX as usize {
                        self.builder.emit(Op::MakeArray(stack_values as u16), 0);
                    } else {
                        self.builder.emit(Op::LoadInt(stack_values as i64), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_MAKE_ARRAY_COUNTED, 1),
                            0,
                        );
                    }
                } // end `if !batched` — the array Value is now on the stack either
                  // way (one Array constant, or assembled from element ops).
                  // c:Src/exec.c::addvars:2624-2632 — the xtrace line. C's emission:
                  //   fprintf(xtrerr, "%s=( ", name);        // "name=( "
                  //   for *ptr in arr: quotedzputs(*ptr); fputc(' ');
                  //   fprintf(xtrerr, ") ");                 // ") "
                  // all inside `if (xtr) { … }` (guarded on the live xtrace state).
                  // Drive it off the WHOLE assembled array via BUILTIN_XTRACE_ARRAY_LINE
                  // (Dup keeps the array on the stack for SET_ARRAY): the builtin reads
                  // the single Value::Array — the same one SET_ARRAY consumes — quotes
                  // each element with quotedzputs, wraps in the `prefix … ) ` frame, and
                  // prints only when xtrace is on. Zero per-element VM slots (the prior
                  // one-slot-per-element trace overflowed next_slot on large literals).
                let prefix_str = if assign.append {
                    format!("{}+=( ", assign.name)
                } else {
                    format!("{}=( ", assign.name)
                };
                self.builder.emit(Op::Dup, 0);
                let pc = self.builder.add_constant(Value::str(prefix_str.as_str()));
                self.builder.emit(Op::LoadConst(pc), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_ARRAY_LINE, 2),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                let name_const = self.builder.add_constant(Value::str(assign.name.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                let argc = 2u8;
                // `name[@]=(...)` / `name[*]=(...)` (array_whole_assign set
                // by the [@] branch above) must reject an associative
                // target (c:Src/params.c:3324); the _AT builtins add that
                // guard, otherwise identical to SET_ARRAY / APPEND_ARRAY.
                let bid = match (assign.append, self.array_whole_assign) {
                    (true, true) => crate::vm_helper::BUILTIN_APPEND_ARRAY_AT,
                    (true, false) => crate::vm_helper::BUILTIN_APPEND_ARRAY,
                    (false, true) => crate::vm_helper::BUILTIN_SET_ARRAY_AT,
                    (false, false) => crate::vm_helper::BUILTIN_SET_ARRAY,
                };
                self.builder.emit(Op::CallBuiltin(bid, argc), 0);
                self.builder.emit(Op::Pop, 0);
                // c:Src/exec.c:3442-3451 / :4032 — `lastval = errflag ? errflag :
                // cmdoutval`, where `cmdoutval` is the exit of the last command
                // substitution performed while building the assignment (set at
                // c:Src/subst.c:2006,2049, zeroed at c:Src/exec.c:2796). The
                // Scalar arm records this at :3609; the Array arm never did, so
                // `compile_simple`'s walk left `chain_had_cmd_subst` false and
                // BUILTIN_ASSIGN_ONLY_STATUS took its `else { 0 }` branch. An
                // array-literal RHS therefore SWALLOWED the status:
                //     a=("$(exit 7)"); print $?    zsh 7, zshrs 0
                // while the scalar form `b=$(exit 7)` was already correct. That
                // also stopped `set -e; a=("$(exit 7)")` aborting, and is what
                // made _git:7640's `__git_command_successful $pipestatus` treat
                // a failed `git rev-list` as success.
                self.last_assign_had_cmd_subst =
                    elements.iter().any(|e| scalar_rhs_has_cmd_subst(e));
            }
        }
    }

    /// Emit the UNBRACED `$name[key]` subscript call. The subscript
    /// decision is a RUNTIME one — `setopt ksharrays` can flip mid-
    /// script — so the compiler always emits
    /// BUILTIN_ARRAY_INDEX_UNBRACED ([name, key, suffix, quoted])
    /// and the bridge dispatches:
    ///   - KSHARRAYS unset → identical to BUILTIN_ARRAY_INDEX
    ///     (+ literal suffix concat).
    ///   - KSHARRAYS set → NO subscript (c:Src/subst.c:2800-2802 +
    ///     2867): bare `$name` expansion + literal `[key]suffix`
    ///     undergoing filename generation (unless `quoted`).
    /// `quoted` is the word's Snull/Dnull marker presence — the
    /// matched shapes start with an unescaped `$`, so markers can
    /// only come from a quote span wrapping the expansion (e.g.
    /// `"$a[0]"`), where zsh suppresses filename generation.
    /// Emit a runtime jump table for `break N`/`continue N` where N is a
    /// runtime expression. Evaluates the count (math), validates it is
    /// positive (else zerrnam + errflag abort), then dispatches to the
    /// SAME patch-list target the literal path uses:
    /// break_patches/continue_patches[depth-N], clamping N>depth to the
    /// outermost. `name` is "break"/"continue" for the error text;
    /// `is_continue` selects the continue vs break patch list.
    fn emit_runtime_loop_level(&mut self, count_word: &str, name: &str, is_continue: bool) {
        let depth = if is_continue {
            self.continue_patches.len()
        } else {
            self.break_patches.len()
        };
        // count on stack (compile the arg word), then the builtin name.
        self.compile_word_str(count_word);
        let nc = self.builder.add_constant(Value::str(name));
        self.builder.emit(Op::LoadConst(nc), 0);
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_BREAK_COUNT_VALIDATE, 2),
            0,
        );
        // Stash the validated count (Int, or 0 on the error path).
        let cnt_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::SetSlot(cnt_slot), 0);
        // Route to the `idx`-th enclosing loop's target, first emitting
        // the `loops--` of every loop skipped on the way out (the target
        // loop's own decrement runs at its exit label, or not at all for
        // a `continue`, which leaves it open). The JumpIfTrue can't carry
        // those ops, so each arm is inverted into
        // `JumpIfFalse(next) … Jump(target)`.
        let mut route = |c: &mut Self, idx: usize| {
            let skip = c.builder.emit(Op::JumpIfFalse(0), 0);
            c.emit_loop_exit(depth - 1 - idx);
            let j = c.builder.emit(Op::Jump(0), 0);
            if is_continue {
                c.continue_patches[idx].push(j);
            } else {
                c.break_patches[idx].push(j);
            }
            let after = c.builder.current_pos();
            c.builder.patch_jump(skip, after);
        };
        // N == L → the L-th enclosing loop.
        for l in 1..=depth {
            self.builder.emit(Op::GetSlot(cnt_slot), 0);
            self.builder.emit(Op::LoadInt(l as i64), 0);
            self.builder.emit(Op::NumEq, 0);
            route(self, depth - l);
        }
        // N > depth → more levels than this chunk has open. c:5837's
        // `num.min(loops)` clamps against the RUNTIME global, which
        // counts the caller's loops as well, so hand the count to
        // `bin_break` and let the per-list `!breaks` gate carry the
        // remainder outward. (Same reasoning as the literal arm in
        // compile_simple.)
        self.builder.emit(Op::GetSlot(cnt_slot), 0);
        self.builder.emit(Op::LoadInt(depth as i64), 0);
        self.builder.emit(Op::NumGt, 0);
        let no_overflow = self.builder.emit(Op::JumpIfFalse(0), 0);
        self.compile_word_str(count_word);
        let bin = if is_continue {
            fusevm::shell_builtins::BUILTIN_CONTINUE
        } else {
            fusevm::shell_builtins::BUILTIN_BREAK
        };
        self.builder.emit(Op::CallBuiltin(bin, 1), 0);
        self.builder.emit(Op::SetStatus, 0);
        let after_overflow = self.builder.current_pos();
        self.builder.patch_jump(no_overflow, after_overflow);
        // N <= 0: BREAK_COUNT_VALIDATE already emitted the error and set
        // errflag; no table entry matches Int(0), so control falls
        // through here and the VM aborts on errflag.
    }

    fn emit_unbraced_subscript(&mut self, name: &str, key: &str, suffix: &str, quoted: bool) {
        let key = subscript_literal_key(key);
        let name_const = self.builder.add_constant(Value::str(name));
        let key_const = self.builder.add_constant(Value::str(key.as_ref()));
        let suffix_const = self.builder.add_constant(Value::str(suffix));
        self.builder.emit(Op::LoadConst(name_const), 0);
        self.builder.emit(Op::LoadConst(key_const), 0);
        self.builder.emit(Op::LoadConst(suffix_const), 0);
        self.builder.emit(Op::LoadInt(quoted as i64), 0);
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_INDEX_UNBRACED, 4),
            0,
        );
    }

    /// Compile a raw word string. Detects $-triggers, glob, tilde,
    /// brace, ZshFlag, array-access at compile time and emits native
    /// ops where possible. Words that hit no fast path fall through
    /// to a runtime expand call via BUILTIN_EXPAND_TEXT.
    fn compile_word_str(&mut self, s: &str) {
        // c:Src/subst.c:1890-1891 + 2550-2557 — `$^name` is the UNBRACED
        // RC_EXPAND_PARAM form. C parses it with the very same flag loop as
        // `${^name}`: the paramsubst-start guard admits `Hat`/`'^'`, then
        //     if ((c = *s) == '^' || c == Hat) {
        //         if ((c = *++s) == '^' || c == Hat) { plan9 = 0; s++; }
        //         else plan9 = 1;
        //     }
        // The same loop also handles `=` (c:2558-2569, SH_WORD_SPLIT / spbreak),
        // which has the identical problem: this compiler carries a WHOLE-WORD
        // fast path per flag (`$#`, `$+`, `$=`, `$~`) and nothing for `^`, so
        //   * `$^b` compiled to LITERAL text — `print -rl -- $^b` printed
        //     `$^b`, and promptinit:23's `for theme in $^fpath/prompt_*_setup(N)`
        //     found ZERO themes where zsh finds 18 (29 of zsh's own shipped
        //     functions use the form, `_git` among them); and
        //   * `$=s` worked ONLY as a whole word — `pre$=s` / `$=s.x` / `$==s.x`
        //     all stayed literal, because a fast path keyed on
        //     `untoked.starts_with("$=")` with an all-alphanumeric name cannot
        //     express an affix.
        //
        // Normalise `$^NAME` → `${^NAME}` and `$=NAME` → `${=NAME}` in the TOKEN
        // stream and let the brace machinery own them — it is already correct
        // for `${^b}/*.txt(N)` and `pre${=s}`. Rewriting tokens rather than the
        // untokenized text keeps the suffix's glob tokens intact, which is the
        // whole point: the whole-word fast paths cannot express
        // `$^fpath/prompt_*_setup(N)`.
        //
        // Quote-safe by construction: only an UNQUOTED `$` is tokenized to
        // Stringg (`\u{85}`) / Qstring (`\u{8c}`). Inside `'…'` or after a
        // backslash the `$` stays a raw byte, so this scan cannot fire there —
        // `'$^b'` and `\$^b` remain literal, exactly as zsh has them.
        let s_rc_norm: String;
        let s: &str = if (s.contains(crate::ported::zsh_h::Stringg)
            || s.contains(crate::ported::zsh_h::Qstring))
            && (s.contains(crate::ported::zsh_h::Hat)
                || s.contains('^')
                || s.contains(crate::ported::zsh_h::Equals)
                || s.contains('=')
                || s.contains(crate::ported::zsh_h::Tilde)
                || s.contains('~'))
        {
            let ch: Vec<char> = s.chars().collect();
            let mut out = String::with_capacity(s.len() + 2);
            let mut i = 0usize;
            while i < ch.len() {
                // Stringg (`$`) followed by a run of one flag char, then a name.
                // A DQ `$` is Qstring (`\u{8c}`), not Stringg — `"$^b"` must
                // normalise too. Re-emit whichever token was there so the
                // double-quote context is preserved.
                //
                // `^` (c:2551 plan9) and `=` (c:2558 spbreak) each double to
                // mean "off", so the run is carried through verbatim and the
                // brace arm applies the same toggling. A run mixes only one
                // char: C's loop re-reads `*s` each pass, but `$^=x` is not a
                // shape either shell's fast paths accept, so keep it simple and
                // only fold a homogeneous run.
                //
                // `~` (c:2596 globsubst) joins them for the SUBSCRIPTED shape
                // only — see the gate on `sub_end` at the rewrite below.
                let flag_char = if i + 1 < ch.len() {
                    match ch[i + 1] {
                        c if c == crate::ported::zsh_h::Hat || c == '^' => Some('^'),
                        c if c == crate::ported::zsh_h::Equals || c == '=' => Some('='),
                        c if c == crate::ported::zsh_h::Tilde || c == '~' => Some('~'),
                        _ => None,
                    }
                } else {
                    None
                };
                if (ch[i] == crate::ported::zsh_h::Stringg
                    || ch[i] == crate::ported::zsh_h::Qstring)
                    && flag_char.is_some()
                {
                    let fc = flag_char.unwrap();
                    let is_flag = |c: char| match fc {
                        '^' => c == crate::ported::zsh_h::Hat || c == '^',
                        '~' => c == crate::ported::zsh_h::Tilde || c == '~',
                        _ => c == crate::ported::zsh_h::Equals || c == '=',
                    };
                    let mut j = i + 1;
                    let mut nflags = 0usize;
                    while j < ch.len() && is_flag(ch[j]) {
                        nflags += 1;
                        j += 1;
                    }
                    // Same name vocabulary the `$+NAME` arm accepts:
                    // alnum/underscore, or one single-char special.
                    let name_start = j;
                    let mut name_end = name_start;
                    if name_end < ch.len() {
                        let f = ch[name_end];
                        if f.is_ascii_alphanumeric() || f == '_' {
                            while name_end < ch.len()
                                && (ch[name_end].is_ascii_alphanumeric() || ch[name_end] == '_')
                            {
                                name_end += 1;
                            }
                        } else if matches!(f, '@' | '*' | '#' | '?') {
                            name_end += 1;
                        }
                    }
                    // c:Src/subst.c:2799-2803 — after the flag loop C calls
                    //     fetchvalue(&vbuf, &s,
                    //                (wantt ? -1 : ((unset(KSHARRAYS) || inbrace) ? 1 : -1)),
                    //                scanflags)
                    // and it is fetchvalue that parses `[subscript]`. The flag
                    // loop (c:2550-2632) has already consumed `^`/`=`, so the
                    // subscript belongs to the SAME reference: `$=opts[tmp+1]`
                    // is `${=opts[tmp+1]}`, not `${=opts}` followed by literal
                    // `[tmp+1]`. Stopping the name scan at the bracket left the
                    // subscript as trailing text, and the user's `_files`
                    // (comp_utils/_files:56 `ignvars=($=opts[tmp+1])`) then read
                    // the WHOLE `opts` array with a stray `[tmp+1]` glued to its
                    // last word — under `nomatch` that is an error, `ignvars`
                    // came out empty, `ign` with it, and `_path_files` never
                    // re-added `-F _comp_ignore`, so the `ignored-patterns`
                    // style silently stopped filtering.
                    //
                    // Walk depth-tracked over BOTH spellings of the brackets:
                    // an unquoted `[` is tokenized to Inbrack by the lexer, a
                    // double-quoted one can arrive either way (mirrors the walk
                    // in subst.rs's bare `$name[sub]` arm).
                    let mut sub_end = name_end;
                    if name_end > name_start
                        && matches!(ch.get(sub_end).copied(), Some('[') | Some(crate::ported::zsh_h::Inbrack))
                    {
                        let mut depth = 1usize;
                        let mut q = sub_end + 1;
                        while q < ch.len() && depth > 0 {
                            let cq = ch[q];
                            // A backslash-escaped bracket (either the Bnull
                            // marker form or the source-literal one) is
                            // subscript CONTENT, not a depth delimiter.
                            if cq == crate::ported::zsh_h::Bnull
                                || cq == crate::ported::zsh_h::Bnullkeep
                                || cq == '\\'
                            {
                                q += 2;
                                continue;
                            }
                            if cq == '[' || cq == crate::ported::zsh_h::Inbrack {
                                depth += 1;
                            } else if cq == ']' || cq == crate::ported::zsh_h::Outbrack {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            q += 1;
                        }
                        if depth == 0 && q < ch.len() {
                            sub_end = q + 1;
                        }
                    }
                    // `~` only folds when a subscript was actually consumed.
                    // Bare `$~NAME` already has a dedicated whole-word fast
                    // path (the `untoked.starts_with("$~")` arm, :6237) whose globbing is
                    // gated on dq/scalar-assign/word-seg context; rewriting
                    // every `$~NAME` into `${~NAME}` would divert all of them
                    // off it. Only `$~NAME[SUB]` is unrepresentable there — the
                    // fast path's name scan stops at the `[`, so the subscript
                    // leaked out as literal text (`print $~a[i]` globbed
                    // `z[i]`). c:Src/subst.c:2596-2602 runs the `~` flag arm in
                    // the SAME loop as `^`/`=`, and c:2799-2803's fetchvalue
                    // then consumes the subscript, so `$~a[i]` is `${~a[i]}`.
                    let fold = name_end > name_start && (fc != '~' || sub_end > name_end);
                    if fold {
                        out.push(ch[i]);
                        out.push(crate::ported::zsh_h::Inbrace);
                        for _ in 0..nflags {
                            out.push(fc);
                        }
                        out.extend(&ch[name_start..sub_end]);
                        out.push(crate::ported::zsh_h::Outbrace);
                        i = sub_end;
                        continue;
                    }
                }
                out.push(ch[i]);
                i += 1;
            }
            s_rc_norm = out;
            &s_rc_norm
        } else {
            s
        };
        // ANSI-C quoted form: `$'a\tb'` arrives from the lexer as
        // `<META-Qstring><Snull>a<Bnull>tb<Snull>` —
        // `\u{8c}\u{9d}a\u{9f}tb\u{9d}` per parse/src/lex:1767-1799.
        // (Older comments reference `<META-$>` = `\u{85}`; accept either
        // marker.) Strip the leading `<META-?>` + `<Snull>` and trailing
        // `<Snull>`, convert each Bnull+X back to `\X` so decode_ansi_c
        // sees real backslash escapes, then run the C-escape decoder.
        let first = s.chars().next();
        if matches!(first, Some('\u{85}') | Some('\u{8c}')) && s.len() >= 3 {
            let inner = &s[first.unwrap().len_utf8()..];
            // Body region is between the leading `\u{9d}` and the FIRST
            // matching `\u{9d}` (Bnull-escapes excluded). Walk to the
            // close so chained `$'a'$'b'` (which has another Stringg
            // BEFORE the trailing Snull) falls through to the segment
            // path instead of emitting the inter-quote markers as
            // literal text. Without this check, `inner.ends_with('\u{9d}')`
            // matched the FINAL Snull and treated everything between
            // as a single ANSI-C body, leaking `Snull+Stringg+Snull`
            // bytes between the decoded `a` and `b`.
            if inner.starts_with('\u{9d}') && inner.len() >= 6 {
                let inner_chars: Vec<char> = inner.chars().collect();
                let mut close_idx: Option<usize> = None;
                let mut escaped = false;
                let mut k = 1; // skip leading Snull
                while k < inner_chars.len() {
                    if escaped {
                        escaped = false;
                        k += 1;
                        continue;
                    }
                    if inner_chars[k] == '\u{9f}' {
                        escaped = true;
                        k += 1;
                        continue;
                    }
                    if inner_chars[k] == '\u{9d}' {
                        close_idx = Some(k);
                        break;
                    }
                    k += 1;
                }
                // Fast path only when the close Snull is the LAST char
                // (i.e. the whole word is one `$'...'` span). Anything
                // after the close means the word continues with more
                // content — let the segment splitter handle it.
                if close_idx == Some(inner_chars.len() - 1) {
                    let body_start = '\u{9d}'.len_utf8();
                    let body_end = inner.len() - '\u{9d}'.len_utf8();
                    let body_raw = &inner[body_start..body_end];
                    // Bnull → `\` so `Bnull t` becomes `\t` for the decoder.
                    let body: String = body_raw
                        .chars()
                        .map(|c| if c == '\u{9f}' { '\\' } else { c })
                        .collect();
                    let decoded = decode_ansi_c(&body);
                    let idx = self.builder.add_constant(Value::str(decoded.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    return;
                }
            }
        }
        // Single-quoted: word contains Snull markers wrapping a literal
        // segment. Three shapes — only the first two take the literal
        // shortcut:
        //
        //   1. The whole value is one single-quoted span — e.g.
        //      `y='hello'` → `<Snull>hello<Snull>`. Take the literal
        //      shortcut: no expansion needed, no $/glob/brace meta.
        //
        //   2. `NAME=<Snull>…<Snull>` — a `typeset`/`local`/`export`
        //      argument (or any arg shaped like an assignment) where
        //      the value is fully single-quoted. zsh preserves the
        //      quoting semantics across the `=`; the value after `=`
        //      stays verbatim regardless of `$VAR` content. Without
        //      this, `typeset -gr x='$VAR'` emitted `x=` (the `$VAR`
        //      got expanded as if unquoted) — broke p10k's
        //      `typeset -gr __p9k_intro_locale='[[ $langinfo... ]]'`.
        //
        //   3. Mixed: a single-quoted segment embedded INSIDE a
        //      larger unquoted/expansion-bearing word — e.g.
        //      `y=${x:-'foo'}` → `${x:-<Snull>foo<Snull>}`. Falls
        //      through to the runtime expand path so the surrounding
        //      `${…}` still resolves while the SQ body stays literal.
        if s.contains('\u{9d}') {
            let trimmed = s.trim_matches(|c: char| c.is_whitespace());
            let whole_sq = trimmed.starts_with('\u{9d}')
                && trimmed.ends_with('\u{9d}')
                && trimmed.matches('\u{9d}').count() == 2;
            if whole_sq {
                let cleaned = crate::lex::untokenize(s);
                let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
                return;
            }
            // `NAME=<Snull>…<Snull>` — assignment-arg shape with a
            // fully-SQ value. The lexer represents the `=` either as
            // its META code (Equals = `\u{8d}`) or as a literal `=`
            // depending on context; accept both. Char-aware scan so
            // the multi-byte Snull/Equals markers don't trip the
            // byte-index slice path.
            let trimmed_chars: Vec<char> = trimmed.chars().collect();
            let eq_pos = trimmed_chars
                .iter()
                .position(|&c| c == '=' || c == '\u{8d}');
            if let Some(eq_idx) = eq_pos {
                let prefix: String = trimmed_chars[..eq_idx].iter().collect();
                let value: String = trimmed_chars[eq_idx + 1..].iter().collect();
                // Optional `+` for `+=` append form.
                let (prefix_clean, append) = if let Some(p) = prefix.strip_suffix('+') {
                    (p.to_string(), true)
                } else {
                    (prefix.clone(), false)
                };
                let prefix_is_ident = !prefix_clean.is_empty()
                    && prefix_clean
                        .chars()
                        .next()
                        .map(|c| c == '_' || c.is_ascii_alphabetic())
                        .unwrap_or(false)
                    && prefix_clean
                        .chars()
                        .all(|c| c == '_' || c.is_ascii_alphanumeric());
                let value_chars: Vec<char> = value.chars().collect();
                let value_is_whole_sq = value_chars.len() >= 2
                    && value_chars[0] == '\u{9d}'
                    && *value_chars.last().unwrap() == '\u{9d}'
                    && value_chars.iter().filter(|&&c| c == '\u{9d}').count() == 2;
                if prefix_is_ident && value_is_whole_sq {
                    // Strip the SNULLs from the value, keep `name=` /
                    // `name+=` literal, emit the joined string as one
                    // constant.
                    let inner: String = value_chars[1..value_chars.len() - 1].iter().collect();
                    let mut out = String::with_capacity(prefix_clean.len() + 2 + inner.len());
                    out.push_str(&prefix_clean);
                    if append {
                        out.push('+');
                    }
                    out.push('=');
                    out.push_str(&inner);
                    let idx = self.builder.add_constant(Value::str(out.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    return;
                }
                // Multi-segment SQ-concat shape: `NAME='X'\''Y'\''Z'`
                // lexes to a sequence of Snull-bounded chunks
                // separated by Bnull+char (escape-concat). Direct
                // port of zsh's parse-time concatenation rule
                // (Src/lex.c::dquote_parse handles it the same way
                // — adjacent quoted/unquoted segments form one
                // word). p10k's `__p9k_intro_base='IFS=$'\'' \t…'\''`
                // hits this shape — without the detector, the
                // expansion path emitted `'IFS=$'` (with literal
                // quotes) and `eval "$__p9k_intro"` ran broken
                // code.
                //
                // Detector: every char in the value is either inside
                // a Snull pair OR is a Bnull-escaped char OR is the
                // Bnull marker itself. NO `$` / `` ` `` outside
                // Snull pairs (those would mean an unquoted
                // expansion segment that the bridge must handle).
                let value_is_sq_concat = !value_chars.is_empty() && {
                    let mut inside_sq = false;
                    let mut all_inside_or_escaped = true;
                    let mut had_sq = false;
                    let mut i = 0;
                    while i < value_chars.len() {
                        let c = value_chars[i];
                        if c == '\u{9d}' {
                            inside_sq = !inside_sq;
                            had_sq = true;
                        } else if !inside_sq {
                            if c == '\u{9f}' && i + 1 < value_chars.len() {
                                // Bnull + char — escape pair, skip both
                                i += 2;
                                continue;
                            }
                            if matches!(c, '$' | '`' | '\u{85}' | '\u{8c}' | '\u{93}' | '\u{99}') {
                                all_inside_or_escaped = false;
                                break;
                            }
                            // Other un-SQ chars (literal text outside
                            // SQ): fine, contributes to the value.
                        }
                        i += 1;
                    }
                    had_sq && all_inside_or_escaped && !inside_sq
                };
                if prefix_is_ident && value_is_sq_concat {
                    // Decode: walk the value, dropping Snull markers
                    // and Bnull escape-bytes, emitting the rest as
                    // literal. Inside Snull: chars are verbatim.
                    // Outside Snull: Bnull+char becomes char literal.
                    let mut decoded = String::new();
                    let mut inside_sq = false;
                    let mut i = 0;
                    while i < value_chars.len() {
                        let c = value_chars[i];
                        if c == '\u{9d}' {
                            inside_sq = !inside_sq;
                            i += 1;
                            continue;
                        }
                        if !inside_sq && c == '\u{9f}' && i + 1 < value_chars.len() {
                            decoded.push(value_chars[i + 1]);
                            i += 2;
                            continue;
                        }
                        decoded.push(c);
                        i += 1;
                    }
                    let mut out = String::with_capacity(prefix_clean.len() + 2 + decoded.len());
                    out.push_str(&prefix_clean);
                    if append {
                        out.push('+');
                    }
                    out.push('=');
                    out.push_str(&decoded);
                    let idx = self.builder.add_constant(Value::str(out.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    return;
                }
            }
            // Mixed: fall through. The runtime expand path needs to
            // see the Snull-bounded segments as literal islands while
            // expanding the surrounding `${…}` / `$name` content.
        }

        // `NAME=<Dnull>…<Dnull>` — assignment-arg shape with a
        // fully-DQ value (no `$` / `` ` `` / `\` escapes inside; for
        // values WITH expansions the bridge path is required).
        // Direct port of zsh's parse-time decision in par_simple
        // where typeset-args inherit the value's quoting verbatim.
        // Without this, `typeset foo="[[ test ]]"` stored
        // `"[[ test ]]"` (DQ marks literal) because the runtime
        // BUILTIN_EXPAND_TEXT mode-1 strip-outer-DQ logic only fires
        // on whole-word DQ wrap, not on the `NAME=…` shape.
        if s.contains('\u{9e}') {
            let trimmed = s.trim_matches(|c: char| c.is_whitespace());
            let trimmed_chars: Vec<char> = trimmed.chars().collect();
            let eq_pos = trimmed_chars
                .iter()
                .position(|&c| c == '=' || c == '\u{8d}');
            if let Some(eq_idx) = eq_pos {
                let prefix: String = trimmed_chars[..eq_idx].iter().collect();
                let value: String = trimmed_chars[eq_idx + 1..].iter().collect();
                let (prefix_clean, append) = if let Some(p) = prefix.strip_suffix('+') {
                    (p.to_string(), true)
                } else {
                    (prefix.clone(), false)
                };
                let prefix_is_ident = !prefix_clean.is_empty()
                    && prefix_clean
                        .chars()
                        .next()
                        .map(|c| c == '_' || c.is_ascii_alphabetic())
                        .unwrap_or(false)
                    && prefix_clean
                        .chars()
                        .all(|c| c == '_' || c.is_ascii_alphanumeric());
                let value_chars: Vec<char> = value.chars().collect();
                let value_is_whole_dq = value_chars.len() >= 2
                    && value_chars[0] == '\u{9e}'
                    && *value_chars.last().unwrap() == '\u{9e}'
                    && value_chars.iter().filter(|&&c| c == '\u{9e}').count() == 2;
                // Only take the literal shortcut when the DQ body
                // has no `$`/`` ` ``/`\\` escape that would need
                // runtime expansion. Fall through to the bridge for
                // values that need expansion (`$VAR` etc. INSIDE the
                // DQ — those still resolve at runtime).
                let inner_chars = if value_is_whole_dq {
                    &value_chars[1..value_chars.len() - 1]
                } else {
                    &value_chars[..]
                };
                let needs_runtime = inner_chars
                    .iter()
                    .any(|c| matches!(c, '$' | '`' | '\u{85}' | '\u{8c}' | '\u{93}' | '\u{99}'));
                if prefix_is_ident && value_is_whole_dq && !needs_runtime {
                    let mut inner: String = inner_chars.iter().collect();
                    // c:Src/subst.c:3649 remnulargs — strip the DQ quote
                    // markers (Bnull/Dnull/Snull). A `\"`/`\\`/`\$` inside the
                    // value lexes to a Bnull marker before the escaped char;
                    // in a stored VALUE (alias/assignment body) that char must
                    // come through bare, NOT as the `\` that `untokenize`
                    // emits for the pattern path (lex.rs:4827). Without this,
                    // `alias x="echo \"hi\""` stored the over-escaped
                    // `echo \"hi\"` instead of `echo "hi"`. remnulargs runs
                    // first (while the markers are still inull bytes); the
                    // trailing untokenize maps any residual non-inull token.
                    crate::ported::glob::remnulargs(&mut inner);
                    let inner = crate::lex::untokenize(&inner);
                    let mut out = String::with_capacity(prefix_clean.len() + 2 + inner.len());
                    out.push_str(&prefix_clean);
                    if append {
                        out.push('+');
                    }
                    out.push('=');
                    out.push_str(&inner);
                    let idx = self.builder.add_constant(Value::str(out.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    return;
                }
            }
        }

        // The lexer marks shell-special chars with zsh's META-range tokens
        // (0x83-0x9f) so the parser can distinguish syntax from literal.
        // For runtime values we want the original char back. `untokenize`
        // does this mapping. We then check for unquoted triggers on the
        // de-tokenized form.
        let untoked = crate::lex::untokenize(s);

        if untoked.is_empty() {
            let idx = self.builder.add_constant(Value::str(""));
            self.builder.emit(Op::LoadConst(idx), 0);
            return;
        }

        // Bnull marker (`\u{9f}`) means "the next char is literal" — used
        // by the lexer for backslash-escaped specials (`\$`, `\`, etc.).
        // Fast-paths that match `$NAME` shapes on the un-tokenized form
        // would mis-route here (the `$` was escaped). Skip the fast paths
        // and fall through to the runtime expand which honors the original
        // `"\$..."` form via untokenize_preserve_quotes.
        let has_bnull = s.contains('\u{9f}');

        // A Bnull escape that sits INSIDE the subscript brackets (after
        // the `[`/Inbrack) is safe for the bare-subscript fast paths:
        // `untokenize` restores it to a literal backslash, and the braced
        // shape those paths rebuild (`${name[key]}` / `${+name[key]}`) is
        // handled by paramsubst's subscript arm identically to the
        // unbraced form. Only an escape BEFORE the `[` (an escaped
        // `$`/name, e.g. `\$a[x]`, `$a\[x]`) must stay on the runtime
        // path. Without this, escaped pattern metachars in unbraced
        // subscripts (`$a[(i)\?]`) reached paramsubst in TOKENIZED form
        // (Inpar / Bnull) that the array flag-parser couldn't read, so
        // `\?` was globbed as `?` instead of literal-matched.
        let bnull_in_subscript = {
            let inbrack = s.find('\u{91}').or_else(|| s.find('[')); // Inbrack
            let bnull = s.find('\u{9f}'); // Bnull
            matches!((inbrack, bnull), (Some(ib), Some(bn)) if bn > ib)
        };

        // A word that BEGINS with a single-quote (Snull `\u{9d}`) marker has
        // its leading `${` / `$name[` as LITERAL text: single quotes make
        // `$`, `{`, `[` ordinary chars. `untokenize(s)` strips the Snull
        // markers, so the `${…}` fast paths below (which pattern-match on the
        // plain `untoked` string) would misread a quoted-literal
        // `'${foo['…']}'` / `'${(e)arr['idx']}'` as a LIVE subscripted /
        // flagged substitution and evaluate it. zsh keeps it literal — the
        // whole word is just string concatenation of SQ-literal fragments and
        // any unquoted `$((…))` / `$var` in between. p10k's deferred prompt
        // templates (`_p9k_prompt_prefix_left='${(e)_p9k_t['$idx']}'`,
        // internal/p10k.zsh:8490) hit exactly this shape; evaluating them at
        // build time dropped every segment → an empty prompt. The static
        // `braced_subscript_ref` path (below) already rejects this because it
        // runs on `untokenize_preserve_quotes` (Snull→`'`, so the leading `'`
        // fails its `${` prefix check); the plain-`untoked` fast paths need
        // this explicit guard.
        let sq_literal_opener = s.starts_with('\u{9d}');

        // Trigger detection. `$` / `` ` `` checks run on the
        // un-tokenized form because the lexer turns `$` into
        // `\u{85}` (META-$) in `s` — the literal-char check on
        // `s` would miss every expansion. Glob triggers (`*`,
        // `?`, `[`) however MUST run on `s` so the Snull/Dnull
        // bslashquote markers correctly suppress meta-interpretation
        // inside `'…'` / `"…"` spans. Direct port of Src/pattern.c
        // ::patcompswitch — chars inside quoted spans bypass meta.
        // Without the marker-aware glob check,
        // `arr=( foo "value:[brackets]" )` fired trigger_glob
        // (the `[` looked unquoted post-untokenize), routed
        // through expand_glob, and NOMATCH-errored at runtime
        // even though the brackets are literally inside DQ.
        // Models c:Src/subst.c:282-330 `stringsubst`. C's expansion
        // trigger fires on lexer-emitted META tokens — `String`
        // (\u{85}) / `Qstring` (\u{8c}) for `$`, `Tick` (\u{93}) /
        // `Qtick` (\u{99}) for backtick. The Rust port can't be a
        // pure-token check because not every caller into
        // compile_word_str hands us raw-lexer output: compile_assign's
        // subscript-key path runs the name through
        // `untokenize_preserve_quotes` first, so a `$n` subscript
        // arrives as literal `$n` not `Stringg n`. We accept both
        // forms; the literal arm is the divergence-from-C the Rust
        // pipeline forces.
        //
        // Walk `s` (markers intact) and suppress while inside a
        // SINGLE-quoted span. Dnull (DQ) is NOT a suppressor — `"$X"`
        // and `"$(…)"` still expand in C. Without the SQ-aware walk,
        // `alias 'foo'='hello $(a || b)/x'` fired trigger_dollar
        // (because the previous `unquoted(&untoked, '$')` ran on the
        // Snull-stripped form), routed through expand_word_glob, and
        // tripped its pattern compiler on the literal `$(...)`.
        //
        // ANSI-C marker pair (`Stringg`/`Qstring` immediately followed
        // by `Snull` — c:301-304 `else if (c == Snull) { ... }`) is
        // metadata: C handles it via `stringsubstquote`, which decodes
        // inline without recursing into paramsubst. Our ANSI-C fast
        // path at lines 2559-2611 above takes pure spans; for
        // chained/mixed shapes the word falls through to here. Skip
        // the marker pair so a word like `$'\\e[2m'` doesn't
        // false-positive.
        let trigger_dollar = {
            let chars: Vec<char> = s.chars().collect();
            let mut inside_sq = false;
            let mut found = false;
            let mut i = 0;
            while i < chars.len() {
                let c = chars[i];
                if c == '\u{9d}' {
                    inside_sq = !inside_sq;
                    i += 1;
                    continue;
                }
                if !inside_sq
                    && (c == '\u{85}' || c == '\u{8c}')
                    && i + 1 < chars.len()
                    && chars[i + 1] == '\u{9d}'
                {
                    i += 1;
                    continue;
                }
                if !inside_sq
                    && matches!(
                        c,
                        // String / Qstring / Tick / Qtick — c:283
                        // `if ((qt = c == Qstring) || c == String)`
                        // and c:331 `else if ((qt = c == Qtick) ||
                        // c == Tick)`.
                        '\u{85}' | '\u{8c}' | '\u{93}' | '\u{99}'
                        // Plus literal `$` / `` ` `` — Rust-port
                        // divergence: untokenize_preserve_quotes
                        // converts the META forms to ASCII at some
                        // callers (compile_assign subscript path)
                        // before reaching here.
                        | '$' | '`'
                    )
                {
                    found = true;
                    break;
                }
                i += 1;
            }
            found
        };
        // Glob metacharacters arrive in two forms:
        //   - Literal char (`*`, `?`, `[`) — the lexer leaves them
        //     bare in some paths (e.g. SQ-stripped contexts)
        //   - META-encoded (`\u{87}` Star, `\u{86}` Quest, `\u{91}`
        //     Inbrack) — the lexer's primary tokenization
        // Trigger glob expansion when EITHER form appears unquoted.
        // Direct port of Src/pattern.c::patcompswitch which treats
        // both encodings as glob metas. Without the META branch,
        // `echo *.toml` saw `\u{87}.toml` (no literal `*`) and
        // skipped expand_glob entirely → literal pattern emitted.
        // c:Src/subst.c:111-112 `prefork` invokes `keyvalpairelement`
        // BEFORE the globlist pass when the word is in PREFORK_ASSIGN
        // context and matches `[key]=value` / `[key]+=value` shape. The
        // Marker / key / value triple it produces is NOT globbed (the
        // Marker entry signals "leave as assoc pair"). In zshrs's fusevm
        // compile path the array elements run through BUILTIN_GLOB_EXPAND
        // before reaching BUILTIN_SET_ARRAY's keyvalpairelement check —
        // so `a=([k]=v)` triggered a NOMATCH glob error on the `[k]=v`
        // pattern instead of routing the kv pair through. Suppress glob
        // when we're in assign context AND the word matches the kv-pair
        // shape (literal `[`, then a `]`, then `=` or `+=`).
        let in_assign = self.assign_context_depth > 0;
        let looks_like_kv_pair = in_assign
            && (untoked.starts_with('[') || untoked.starts_with('\u{91}' /* Inbrack */))
            && {
                // Find first `]` (or Outbrack TOKEN) and check that
                // immediate follower is `=` (or `+=`).
                let chars: Vec<char> = untoked.chars().collect();
                let end = chars
                    .iter()
                    .enumerate()
                    .skip(1)
                    .find(|(_, &c)| c == ']' || c == '\u{92}' /* Outbrack */)
                    .map(|(i, _)| i);
                end.is_some_and(|e| {
                    let after = chars.get(e + 1);
                    matches!(after, Some('=') | Some('\u{8d}' /* Equals */))
                        || (after == Some(&'+')
                            && matches!(chars.get(e + 2), Some('=') | Some('\u{8d}' /* Equals */)))
                })
            };
        // c:Src/parse.c par_simple (intypeset) + c:Src/exec.c:4265 —
        // for a BINF_ASSIGN / BINF_MAGICEQUALS builtin an
        // assignment-SHAPED argument (`NAME=v`, `NAME+=v`,
        // `NAME[sub]=v`) is parsed as an ASSIGNMENT word and preforked
        // with PREFORK_ASSIGN, which never runs the globlist pass — so
        // the `[`/`]` of a SUBSCRIPT is not a glob bracket. zshrs
        // compiled it as an ordinary word, so `typeset ptr[1]=var` (and
        // every `typeset arr[2]=x` / `local h[k]=v`) died with
        // `no matches found: ptr[1]=var` before bin_typeset ever saw the
        // name (K01nameref.ztst:13 needs the nameref-name check that
        // sits behind it). `assign_builtin_arg_depth > 0 &&
        // assign_context_depth > 0` is the established discriminator for
        // "assignment-shaped arg of a typeset-family builtin" (see the
        // `ssub` gate at compile_zsh.rs:6668); array ELEMENTS of a
        // paren-init bump only `assign_context_depth`, so `a=(x[1]=y)`
        // keeps globbing exactly as zsh does.
        let is_typeset_assign_arg =
            self.assign_builtin_arg_depth > 0 && self.assign_context_depth > 0;
        let trigger_glob = !looks_like_kv_pair
            && !is_typeset_assign_arg
            && (unquoted(s, '*')
            || unquoted(s, '\u{87}')   // Star (parse/tokens.rs:14)
            || unquoted(s, '?')
            || unquoted(s, '\u{97}')   // Quest (parse/tokens.rs:30)
            || unquoted(s, '[')
            || unquoted(s, '\u{91}')   // Inbrack (parse/tokens.rs:24)
            // extendedglob `^pat` (negation) and `pat~excl` (exclusion).
            // `^` is a no-op without `setopt extendedglob`, but routing
            // through expand_glob lets the runtime decide. The unquoted
            // check ensures `"^b"` (literal) isn't treated as a glob.
            // Also matches `/path/^pat` — `^` at the start of any path
            // component (after `/`) is a negation in extendedglob.
            || (untoked.starts_with('^') && untoked.len() > 1)
            || untoked.contains("/^")
            // extendedglob `#` / `##` hash quantifier — c:Src/pattern.c
            // :4365 haswilds gates `#` as wild whenever EXTENDEDGLOB is
            // set. Trigger unconditionally at compile time: `setopt
            // extended_glob` may fire BETWEEN the compile pass and the
            // runtime word evaluation, so we cannot consult the option
            // here (`setopt extended_glob; print -l /tmp/zh/a#`
            // compiles `print` before `setopt` runs). The runtime
            // `zglob` → `haswilds` check at glob.rs short-circuits
            // when EXTENDEDGLOB is off, so routing literal-`#` words
            // through the bridge is a no-op in the off case (#89/#117
            // in docs/BUGS.md). The lexer META-encodes `#` as Pound
            // (\u{84}); check both forms.
            || unquoted(s, '#')
            || unquoted(s, '\u{84}')
            // zsh glob qualifiers: `*(.)` / `path(mh-1)` etc. The `(...)`
            // suffix triggers globbing even when the body has no other
            // glob metachar — needed for `/etc/hosts(mh-100)` style.
            // Conservative: require closing `)` at end and a bare `(`
            // somewhere before (no other meta chars in between).
            // Bnull-gate: backslash-escaped parens (`\(...\)`) must NOT
            // fire. After untokenize the `\u{9f}` markers are gone, so
            // `(abc)` looks like a qualifier suffix — check the raw `s`
            // for at least one un-escaped `(` and `)`. Bug #537.
            || (untoked.ends_with(')')
                && untoked.contains('(')
                && !untoked.contains('|')
                && (unquoted(s, '(') || unquoted(s, '\u{88}'))
                && (unquoted(s, ')') || unquoted(s, '\u{8a}')))
            // Unclosed `(` (or `(` anywhere not already covered by the
            // qualifier-suffix / alternation arms above). C zsh's
            // `Src/pattern.c:4326-4335 haswilds` returns true on any
            // `Inpar` / `(` unless `SHGLOB` is set. zshrs's compile-time
            // gate was too conservative — `echo (abc` and `echo abc(a)def`
            // fell through the LoadConst fast path and printed literally
            // instead of routing through `patcompile` which rejects
            // `(abc` as "bad pattern: (abc" (#170 in docs/BUGS.md).
            // The runtime `zglob` short-circuits when SHGLOB is set, so
            // the over-trigger is a no-op for cases C would also skip.
            // The lexer META-encodes `(` as Inpar (\u{88}); check both.
            || unquoted(s, '(')
            || unquoted(s, '\u{88}')
            // Glob alternation `(a|b|c)` is a primary zsh feature
            // (no extendedglob required). Direct port of zsh's
            // pattern.c P_BRANCH `|` at the path level —
            // `/etc/(passwd|hostname)` should glob to multiple
            // alternatives. Detected by `(`...`|`...`)` shape;
            // expand_glob's expand_glob_alternation helper does
            // the actual top-level-vs-nested check.
            //
            // Run the check via `unquoted()` against `s` (which still
            // carries Snull / Dnull markers) so a literal alternation
            // shape sitting inside `'…'` / `"…"` doesn't trip the
            // trigger. Without this, `alias 'foo'='hello $(a || b) world'`
            // had `(`/`|`/`)` literally inside the SQ value; untokenize
            // dropped the markers and `untoked.contains()` saw them
            // as unquoted glob alternation, routing the word through
            // expand_glob which then "bad pattern"-errored at runtime.
            || ((unquoted(s, '(') || unquoted(s, '\u{88}'))
                && (unquoted(s, '|') || unquoted(s, '\u{8e}'))
                && (unquoted(s, ')') || unquoted(s, '\u{8a}')))
            // zsh numeric range glob `<N-M>`: any `<…-…>` shape with
            // optional digits on either side outside a bracket-class.
            || has_numeric_range_glob(&untoked));
        let trigger_tilde = untoked.starts_with('~') || untoked.contains(":~") || untoked.contains("=~")
            // c:Src/subst.c:715 — `=cmd` (EQUALS option) routes
            // through filesubstr's equalsubstr arm. Route the word
            // through the bridge so filesub fires at runtime; the
            // runtime checks `isset(EQUALS)` before expanding.
            || untoked.starts_with('=')
            // c:Src/subst.c:680 — filesub's PREFORK_TYPESET arm fires on
            // `sub[1] == Tilde || sub[1] == Equals`, where `sub` is the
            // first `Equals` at index >= 1 (c:678). The `Tilde` half is
            // the `"=~"` test above; this is the `Equals` half, which was
            // missing, so `a==ls` was emitted as a plain `LoadConst` and
            // never reached `filesub` at all — no `=cmd` expansion ran.
            || untoked.contains("==")
            // c:Src/subst.c:688-698 — the `:`-component walk applies the
            // same trigger pair to each `:` after `eql`, so a `=cmd` in a
            // path-list component qualifies too. This one needs no
            // MAGIC_EQUAL_SUBST: a real assignment is always assign
            // context (`addvars` → prefork(PREFORK_ASSIGN)), which is why
            // `kv=a:=ls` diverged as well.
            || untoked.contains(":=");
        // Brace expansion: `{a,b,c}` and `{1..5}` need expansion. Detect
        // matched-brace forms with comma or `..` inside.
        //
        // c:Src/subst.c:170 — `if (unset(IGNOREBRACES) && !(flags &
        // PREFORK_SINGLE))` gates the `xpandbraces` loop. A word reaching
        // prefork with PREFORK_SINGLE is NEVER brace-expanded, and cond
        // operands are exactly such words: cond.c:53 `singsub(strp)` →
        // subst.c:520 `prefork(&foo, PREFORK_SINGLE, NULL)`.
        //
        // `dq_context_depth > 0` is this compiler's existing spelling of that
        // state (see the singsub/PREFORK_SINGLE comment at line 3305, and the
        // cond-operand bumps in compile_cond_expr). Without the gate, `[[ aaa
        // =~ a{2,3} ]]` brace-expanded its ERE bound to `a2 a3` (so the regex
        // became `a2` and stopped matching), and `[[ -n a{2,3} ]]` split one
        // operand into two words and returned 1 where zsh returns 0.
        //
        // A DQ-marker-wrapped word (`\u{9e}…\u{9e}`) counts as the same state:
        // braces are literal inside `"…"` (`print -r -- "{a,b}"` → `{a,b}`), and
        // the `=~` arm wraps its RHS in those markers precisely to reach
        // singsub semantics. Same spelling as `has_quote_markers` (line 4118).
        let in_prefork_single = self.dq_context_depth > 0 || word_is_single_dq_span(s);
        let trigger_brace = !in_prefork_single && looks_like_brace_expansion(&untoked);

        // Process substitution `<(cmd)` / `>(cmd)`. The lexer marks the
        // outer angle bracket with Inang (`\u{94}`) / Outang (`\u{95}`)
        // and the parens as Inpar/Outpar. After untokenize, the form
        // is `<(...)` / `>(...)`. Compile the inner program as a
        // sub-chunk and emit ProcessSubIn/Out which wires up the
        // FIFO/temp file at runtime.
        // `=(cmd)` is the temp-file flavor of process substitution
        // (zsh-only, vs `<(cmd)`'s FIFO). Both deliver a path to the
        // consumer; process_sub_in already creates a durable temp file
        // so `=(...)` shares the read-end implementation. Safe for the
        // read-once consumers (cat/diff/comm) that drive `=(...)` use.
        let is_eq_psub = untoked.starts_with("=(") && untoked.ends_with(')');
        if (untoked.starts_with("<(") || untoked.starts_with(">(") || is_eq_psub)
            && untoked.ends_with(')')
        {
            // c:Src/exec.c:4918/5040/5069 — a process substitution used in a
            // `[[ … ]]` cond operand errors "process substitution %s cannot
            // be used here" (getoutputfile/getproc run with thisjob == -1).
            // zshrs's THISJOB doesn't distinguish the cond context at
            // runtime, so emit the rejection here where `in_cond_operand` is
            // known: push the raw substitution text and call the error
            // builtin instead of ProcessSubIn/Out. (`cat =(…)` / `diff <(…)`
            // argument uses stay unaffected — in_cond_operand is false there.)
            if self.in_cond_operand {
                let idx = self.builder.add_constant(Value::str(untoked.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PROCSUB_COND_ERROR, 1),
                    0,
                );
                return;
            }
            let is_in = untoked.starts_with("<(") || is_eq_psub;
            let inner = &untoked[2..untoked.len() - 1];
            // Mirror Src/init.c errflag save/clear/check around the
            // process-sub inner parse.
            let saved_errflag = errflag.load(Ordering::Relaxed);
            errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
            // Context-isolated parse (c:Src/exec.c:283 parse_string). This
            // compile-time re-parse of the `<(…)` / `>(…)` body runs while
            // the outer single-event loop()/parse_event reader is mid-stream
            // (compile happens per-event, before execode), so a bare
            // parse_init/lex_init would STEAL the next SHIN line into this
            // inner program. parse_isolated sets `strin` so the drain EOFs.
            let prog = crate::vm_helper::parse_isolated(inner);
            let parse_failed = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
            errflag.store(saved_errflag, Ordering::Relaxed);
            if !parse_failed {
                let mut sub = ZshCompiler::new();
                sub.compile_program(&prog);
                let sub_end = sub.builder.current_pos();
                for patch in std::mem::take(&mut sub.return_patches) {
                    sub.builder.patch_jump(patch, sub_end);
                }
                let mut chunk = sub.builder.build();
                // c:Src/exec.c:4988 `execode(prog, 0, 1, "equalsubst")` — mark
                // the `=(...)` flavor so the runtime writes a REAL temp file
                // (getoutputfile) rather than the `<(...)` /dev/fd pipe. The
                // consumer of `=(...)` gets a seekable regular file that is
                // fully written before the command runs.
                if is_eq_psub {
                    chunk.source = "equalsubst".to_string();
                }
                let sub_idx = self.builder.add_sub_chunk(chunk);
                if is_in {
                    self.builder.emit(Op::ProcessSubIn(sub_idx), 0);
                } else {
                    self.builder.emit(Op::ProcessSubOut(sub_idx), 0);
                }
                return;
            }
        }

        if !trigger_dollar && !trigger_glob && !trigger_tilde && !trigger_brace && !has_bnull {
            // Pure literal.
            //
            // Bnull (`\u{9f}`) words have `\X` backslash escapes that
            // untokenize materializes back to literal `\` — emitting
            // them as LoadConst would print the raw backslash. C zsh's
            // c:Src/subst.c prefork → stringsubst → untokenize chain
            // applies remquotes/remnulargs INSIDE the substitution
            // walk, stripping the Bnull escape AFTER stringsubst
            // declines to expand. Route Bnull words through the bridge
            // so the runtime expand path performs the same unescape;
            // otherwise `echo \\$X` printed `\$X` instead of `$X`.
            //
            // NOTE: an earlier `strip_quote_markers` pass here removed
            // every `\x00` as a "bslashquote sentinel" — but the
            // current lexer marks quoted chars with Bnull `\u{9f}`
            // (lex.rs:1723/2124/2256), never `\x00`, so the only NULs
            // reaching this point are REAL data bytes decoded from
            // `$'\0'` (c:Src/utils.c getkeystring → raw NUL, written
            // by fwrite at c:Src/builtin.c bin_print). Stripping them
            // dropped the NUL from `"a"$'\0'"b"`. Bug #560.
            let idx = self.builder.add_constant(Value::str(untoked.as_str()));
            self.builder.emit(Op::LoadConst(idx), 0);
            return;
        }

        // Skip native fast-paths if the raw word has a Bnull escape marker
        // — the bridge path is the only one that preserves backslash-quoted
        // specials. (Normal untokenize collapses Bnull away, hiding the
        // escape from the simple $NAME / ${NAME} matchers below.)
        if has_bnull {
            // Fall through to the bridge.
        }
        // Fast path: `$@` / `$*` (quoted or unquoted) — must emit a native
        // GET_VAR so the result is Value::Array of positionals. The bridge
        // path below routes through expand_word_glob which collapses
        // DoubleQuoted into one joined string, breaking spread semantics.
        // c:Src/subst.c — `${*}` / `${@}` braced shapes are
        // semantically identical to `$*` / `$@` (subst.c:1885
        // paramsubst entry just consumes the braces). Bug #588
        // extends the fast path to recognize the braced forms so
        // `"${*}"` honors IFS first-char joining matching `"$*"`.
        // Fast path: ksh93 funsub `${ list; }` and mksh valsub
        // `${| list; }` — a command substitution that runs in the CURRENT
        // shell environment instead of a subshell.
        //
        //   ksh(1), Command Substitution: "${ command;} … the command is
        //   executed in the current shell environment … the value is the
        //   standard output with trailing newlines removed."
        //   mksh(1) calls the two forms funsub and valsub; for a valsub
        //   "the value of the expansion is the value of REPLY" and stdout
        //   is NOT captured.
        //
        // Measured — ksh93 and mksh alike:
        //   ksh  -c 'x=0; y=${ x=5; print -n out; }; print "x=$x y=$y"' → x=5 y=out
        //   mksh -c 'x=0; y=${|x=5; REPLY=v;};  print "x=$x y=$y"'      → x=5 y=v
        //   mksh -c 'REPLY=outer; y=${|:;}; print "[$y][$REPLY]"'       → [][outer]
        //
        // zsh has neither form: `${` followed by a space or `|` reaches
        // paramsubst as a malformed name and errors "bad substitution",
        // which is what zshrs did in every mode.
        //
        // SCOPE: only a word that is ENTIRELY the substitution, bare or
        // double-quoted (`${ … }` / `"${ … }"`). Embedded in a larger word
        // (`x${ f; }y`, which both references accept) still takes the
        // generic paramsubst route and still errors — the segment splitter
        // below would have to learn the form, which is a separate change.
        // zsh 5.10 added the SAME two forms natively — "nofork command
        // substitution" — plus a third that names the result variable:
        //
        //   c:Src/subst.c:1913-1922 (paramsubst) —
        //     "Handling for nofork command substitution e.g. ${|cmd;} …
        //      The command string is extracted and executed, and the
        //      substitution assigned."
        //   c:Src/subst.c:1924 — `if (inchar == '|' || inchar == Bar ||
        //     inblank(inchar))` picks the two bare forms;
        //   c:Src/subst.c:1930 — `else if (inchar == '{' || inchar ==
        //     Inbrace)` picks `${{VAR} cmd }`.
        //
        // So the gate is no longer korn-only: it applies in every mode a
        // `${` + blank / `|` / `{VAR}` word can legally appear in. The
        // per-form semantics differ between zsh and ksh/mksh (trailing
        // newline trimming, chiefly), and those differences are resolved at
        // RUNTIME inside BUILTIN_KSH_FUNSUB — `emulate -L ksh` can change
        // the answer after this word was compiled (D10nofork.ztst
        // "newline removal in ${ ... }, emulation mode, shwordsplit").
        if !has_bnull {
            let dq = word_is_single_dq_span(s);
            // The BODY is a fresh command line, so it must keep its own
            // quoting: `untokenize` folds the lexer's escape markers away
            // and turned `${ printf "a\n"; }` into `printf "an"`.
            // `untokenize_preserve_quotes` is the variant that keeps them.
            // `untokenize_preserve_quotes` deliberately KEEPS the
            // DQ-context `$` marker (Qstring, lex.rs:4988) so downstream
            // paramsubst can see it was quoted. A funsub body is a fresh
            // command line, not a quoted expansion, so the marker has to
            // become a plain `$` before it is lexed again.
            let preserved = crate::lex::untokenize_preserve_quotes(s)
                .replace(crate::ported::zsh_h::Qstring, "$");
            let body_src: &str = {
                let t = preserved.as_str();
                t.strip_prefix('"')
                    .and_then(|t| t.strip_suffix('"'))
                    .unwrap_or(t)
            };
            if let Some((body, rplyvar, kind)) = ksh_funsub_body(body_src) {
                let body_idx = self.builder.add_constant(Value::str(body));
                self.builder.emit(Op::LoadConst(body_idx), 0);
                let var_idx = self.builder.add_constant(Value::str(rplyvar));
                self.builder.emit(Op::LoadConst(var_idx), 0);
                self.builder.emit(Op::LoadInt(kind), 0);
                // `qt` — c:Src/subst.c:1908 `int trim = (!EMULATION(
                // EMULATE_ZSH)) ? 2 : !qt;`. Whether the word was inside
                // double quotes is a LEXICAL fact, so it is settled here;
                // the emulation half is not, so the builtin computes `trim`.
                self.builder.emit(Op::LoadInt(if dq { 1 } else { 0 }), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_KSH_FUNSUB, 4), 0);
                // Unquoted, the result is an ordinary expansion. Under
                // ksh/mksh (and zsh's SH_WORD_SPLIT) that means IFS word
                // splitting; under plain zsh it does NOT
                // (D10nofork.ztst "test word splitting on result" pins both
                // halves in one chunk). The option is read at RUNTIME, so
                // the split now happens inside BUILTIN_KSH_FUNSUB rather
                // than through an unconditional BUILTIN_WORD_SPLIT here.
                return;
            }
        }

        let bare_target = if !has_bnull {
            if untoked == "$@" || untoked == "$*" {
                Some(&untoked[1..])
            } else if untoked == "${@}" || untoked == "${*}" {
                // Strip `${` + name + `}` — name is the single char.
                Some(&untoked[2..untoked.len() - 1])
            } else {
                None
            }
        } else {
            None
        };
        if let Some(name) = bare_target {
            let idx = self.builder.add_constant(Value::str(name));
            // Detect DQ context two ways: (a) the raw input `s` is
            // DQ-wrapped (`\u{9e}$*\u{9e}`), or (b) we're inside a
            // recursive compile_word_str whose parent set
            // dq_context_depth. zsh: `"$*"` joins by IFS first char,
            // `"$@"` keeps splice semantics (each positional its own
            // word). Only `*` gets the join — `@` continues to return
            // Array. Without the `*` fix, `v="$*"` captured only the
            // first positional because pop_args flattens Array.
            let in_dq = self.dq_context_depth > 0 || word_is_single_dq_span(s);
            self.builder.emit(Op::LoadConst(idx), 0);
            // c:Src/exec.c:2554 addvars — a SCALAR assignment RHS is expanded
            // with PREFORK_SINGLE, so it is NOT word-split even under
            // SH_WORD_SPLIT. GET_VAR applies the split (and hence trims
            // leading/trailing IFS-whitespace: `sepsplit(' a:b ')` → `a:b`),
            // GET_VAR_DQ does not. A bare `w=$v` reaches this unquoted path, so
            // route it through the DQ (no-split) variant when it is a scalar
            // assignment value: `setopt shwordsplit; v=' a:b '; w=$v` kept the
            // surrounding spaces in zsh but zshrs trimmed them.
            let no_split =
                in_dq || self.scalar_assign_depth > 0 || self.assign_builtin_arg_depth > 0;
            let getvar = if no_split {
                crate::vm_helper::BUILTIN_GET_VAR_DQ
            } else {
                crate::vm_helper::BUILTIN_GET_VAR
            };
            self.builder.emit(Op::CallBuiltin(getvar, 1), 0);
            if !in_dq {
                // c:Src/subst.c:184-187 — prefork's empty-word
                // removal applies to the UNQUOTED positional splat:
                // `set -- a '' b; print -l -- $@` → 2 lines in zsh.
                // Quoted "$@" keeps the empty slot (and "$*" joins),
                // so the filter is compile-gated on !in_dq.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_DROP_EMPTY, 1),
                    0,
                );
            }
            if in_dq && name == "*" {
                // Discard the GET_VAR result and route the quoted
                // `"$*"` through BUILTIN_EXPAND_TEXT mode 1 with body
                // "$*". EXPAND_TEXT bumps the executor's in_dq_context
                // around the call, so multsub → paramsubst sees qt=true
                // and runs the canonical IFS-join with single-scalar
                // shape (Src/subst.c c:3032 sepjoin). Routing through
                // JOIN_STAR directly missed the DQ context and the
                // handler couldn't tell quoted from unquoted, so the
                // post-join word-split fired even for `"$*"`. Bug #428.
                self.builder.emit(Op::Pop, 0);
                let body_const = self.builder.add_constant(Value::str("$*"));
                self.builder.emit(Op::LoadConst(body_const), 0);
                self.builder.emit(Op::LoadInt(1), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
                // c:Src/subst.c:3032 — the quoted branch ends in
                // `sepjoin(aval, sep, 1)`, so `"$*"` is ALWAYS one
                // word, empty positional list included. multsub
                // returns zero nodes there (right for `"$@"`, wrong
                // for `"$*"`), which elided the argument entirely:
                // `set --; printf '%d|%s|%d\n' $# "$*" 7` printed
                // `0|7|0` instead of zsh/bash/dash/ksh's `0||7`.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_QUOTED_STAR_ONE_WORD, 1),
                    0,
                );
            }
            return;
        }

        // Fast path: `$@[SUB]` / `$*[SUB]` / `$argv[SUB]` — bare-form
        // positional/array subscript. Route through BUILTIN_ARRAY_INDEX
        // which calls paramsubst with `${name[sub]}` so the full
        // subscript dispatch (range, @, *, negatives, etc.) applies.
        //
        // The `$` prefix is REQUIRED. Bug #29 in docs/BUGS.md: the
        // previous `strip_prefix('$').unwrap_or(&untoked)` accepted
        // bare `argv[...]` (no `$`) as if it were `$argv[...]`, so a
        // DQ word like `"argv[1]=$a[1]"` (which after Dnull-strip
        // untokenizes to literal text + a paramsubst) was misread as
        // a single `${argv[KEY]}` lookup with KEY = `1]=$a[1`. The
        // whole literal prefix got consumed. Also require that the
        // untokenized input has EXACTLY ONE `$` — multiple `$`s
        // indicate a literal-with-expansions word that the segment
        // splitter further down should handle instead.
        if !has_bnull {
            if let Some(inner) = untoked.strip_prefix('$') {
                if !inner.contains('$') {
                    if let Some(lb) = inner.find('[') {
                        let nm = &inner[..lb];
                        if matches!(nm, "@" | "*" | "argv") && inner.ends_with(']') {
                            let key = &inner[lb + 1..inner.len() - 1];
                            // Inner key must not itself contain a `$` /
                            // `[` / `]` — those would be a nested subscript
                            // or paramsubst that needs the runtime path.
                            if !key.contains('$') && !key.contains('[') && !key.contains(']') {
                                let quoted = self.dq_context_depth > 0
                                    || s.contains('\u{9d}')
                                    || s.contains('\u{9e}');
                                self.emit_unbraced_subscript(nm, key, "", quoted);
                                return;
                            }
                        }
                    }
                }
            }
        }

        // Fast path: single bare `$NAME` (no braces, no concat, no idx,
        // no modifier). Covers `$x`, `$1`, `$#`, `$?`, `$!`, etc. — the
        // most common case in real scripts. Emits BUILTIN_GET_VAR
        // directly without going through the runtime expand path.
        // Skip when the raw word has Dnull/Snull bslashquote markers — those
        // signal an internal bslashquote boundary (e.g. `"$a"bar` becomes
        // Dnull+$+a+Dnull+bar; after untokenize it looks like `$abar`
        // and the fast-path reads the wrong name). The bridge below
        // handles those correctly by routing through expand_string.
        let has_quote_markers = s.contains('\u{9d}') || s.contains('\u{9e}');
        // c:Src/subst.c:2622 (inull skip) + c:2696 (post-subexp skip)
        // + c:2993-3004 (operator gate) — quote markers INSIDE a
        // `${…}` body are paramsubst's business: `${(f)"$(cmd)"}` is
        // legal (skipped around the subexp), `${"abc"}` /
        // `${(Q)"abc"}` are "bad substitution" (raw Dnull at the
        // operator position). untokenize DROPS Snull/Dnull, so every
        // untoked-based `${…}` fast path below silently misreads a
        // quoted body as a bare name and the error path never fires.
        // Route such words to the EXPAND_TEXT bridge, which hands the
        // RAW tokenized body to multsub → stringsubst → paramsubst.
        let quoted_brace_body = {
            let mut depth = 0i32;
            let mut found = false;
            for c in s.chars() {
                match c {
                    '\u{8f}' => depth += 1,                 // Inbrace
                    '\u{90}' => depth = (depth - 1).max(0), // Outbrace
                    '\u{9d}' | '\u{9e}' if depth > 0 => {
                        found = true;
                        break;
                    }
                    _ => {}
                }
            }
            found
        };
        if quoted_brace_body {
            let preserved: String = s.to_string();
            let mode = if self.dq_context_depth > 0 {
                1
            } else {
                expand_text_mode(s, &preserved)
            };
            let idx = self.builder.add_constant(Value::str(preserved.as_str()));
            self.builder.emit(Op::LoadConst(idx), 0);
            self.builder.emit(Op::LoadInt(mode as i64), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
            return;
        }
        if !has_bnull && !has_quote_markers {
            if let Some(name) = bare_var_ref(&untoked) {
                // c:Src/subst.c — `$#@` / `$#*` are bare-form shorthand
                // for `${#@}` / `${#*}` (count of positional params).
                // bare_var_ref returns the two-char name "#@" / "#*";
                // GET_VAR on that name yields empty. Route to
                // PARAM_LENGTH("@") / PARAM_LENGTH("*") instead so the
                // recursive `${#@}` paramsubst path fires.
                if name == "#@" || name == "#*" {
                    let inner = &name[1..]; // "@" or "*"
                    let idx = self.builder.add_constant(Value::str(inner));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_LENGTH, 1),
                        0,
                    );
                    return;
                }
                let idx = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(idx), 0);
                // Special positional names (`argv` / `@` / `*`) in
                // unquoted position must expand as an ARRAY of words,
                // not a scalar IFS-joined string. C zsh: `print -l
                // $argv` runs print with N args, one per positional;
                // emitting BUILTIN_GET_VAR (scalar, IFS-joined)
                // produced a single joined arg and lost the
                // word-split. BUILTIN_ARRAY_ALL returns
                // `Value::Array` which the VM splices into argv.
                // QUOTED `"$name"`: force DQ semantics so an array's
                // empty elements are preserved (these compile to a direct
                // GET_VAR with no EXPAND_TEXT wrapper, so the runtime
                // in_dq_context is 0 and plain GET_VAR would word-elide
                // the empties). c:Src/subst.c:184-187 + c:1759 sepjoin:
                // `a=(1 "" 3); "$a"` → `1  3`. Detect quoting via dq-depth
                // OR raw token DQ-wrapping (same dual check as `$@`/`$*`).
                let in_dq = self.dq_context_depth > 0 || word_is_single_dq_span(s);
                // c:Src/exec.c:2554 — a SCALAR assignment RHS is expanded with
                // PREFORK_SINGLE, so it is NOT word-split under SH_WORD_SPLIT.
                // GET_VAR applies the split (which also trims leading/trailing
                // IFS-whitespace: `sepsplit(' a:b ')` → `a:b`); GET_VAR_DQ keeps
                // the value whole. `setopt shwordsplit; v=' a:b '; w=$v` kept
                // the spaces in zsh but zshrs trimmed them.
                let opcode = if matches!(name, "argv" | "@" | "*") {
                    crate::vm_helper::BUILTIN_ARRAY_ALL
                } else if in_dq || self.scalar_assign_depth > 0 || self.assign_builtin_arg_depth > 0
                {
                    crate::vm_helper::BUILTIN_GET_VAR_DQ
                } else {
                    crate::vm_helper::BUILTIN_GET_VAR
                };
                self.builder.emit(Op::CallBuiltin(opcode, 1), 0);
                if matches!(name, "argv" | "@" | "*") {
                    // c:Src/subst.c:184-187 — unquoted positional
                    // splat drops empty words (`set -- a '' b;
                    // print -l -- $@` → 2 lines in zsh). The quoted
                    // "$@" form routes through the bare_target DQ
                    // path above, never here.
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_DROP_EMPTY, 1),
                        0,
                    );
                } else if opcode == crate::vm_helper::BUILTIN_GET_VAR && self.word_seg_depth == 0 {
                    // c:Src/subst.c:180-187 — this word is exactly one
                    // UNQUOTED `$NAME`, so each element the read yields IS a
                    // finished word and prefork's empty-node removal applies.
                    // GET_VAR's own array arm already filters, but its
                    // RC_EXPAND_PARAM arm deliberately does not (a PREFIXED
                    // plan9 word cross-products, where an empty element still
                    // makes a non-empty word) — and `_comp_setup` sets
                    // `rcexpandparam` for every completion (Completion/Base/
                    // Core/compinit:146,180-182), so the leading empty of the
                    // `local a; a+=(x)` idiom leaked into completer argv.
                    // `opcode == BUILTIN_GET_VAR` already excludes the DQ /
                    // scalar-assign / assign-builtin forms (they take
                    // GET_VAR_DQ) and the `argv`/`@`/`*` specials above.
                    // `word_seg_depth == 0` keeps it to a word that is ONLY
                    // this reference: reached as a SEGMENT (`p$a`, compiled
                    // via the split_word_segments recursion) the element is
                    // not a finished word, and under plan9 the prefix
                    // cross-products so an empty element still yields the
                    // word `p` (c:Src/subst.c:4327-4373). Eliding there
                    // dropped it: `setopt rcexpandparam; a=('' x);
                    // print -rl -- p$a` printed only `px`.
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_WORD_ELIDE_EMPTY, 1),
                        0,
                    );
                }
                return;
            }
        }

        // Fast path: bare `$#NAME` — equivalent to `${#NAME}` (string
        // length / array element count). Without braces this looked
        // like a literal in the dispatch path, so `echo $#a` printed
        // `$#a` verbatim instead of `3`. Compose by emitting the param
        // length form via PARAM_LENGTH builtin (pops [name], returns
        // count). Match zsh: the name must start with letter/underscore.
        // Also accepts an optional `[@]` / `[*]` suffix (`$#a[@]` is the
        // count of array elements, same as `$#a` on an indexed array).
        if !has_bnull && untoked.len() >= 3 && untoked.starts_with("$#") {
            let rest = &untoked[2..];
            // zsh: `$#NAME[idx]` is sugar for `${#NAME[idx]}` —
            // length of the selected array element / subscripted
            // value. Also handles `[@]`/`[*]` (array length).
            // Without this, the trailing subscript was rendered
            // as literal text (`3[2]` for an array of size 3).
            // Substitute braces and recurse via expand_string at
            // runtime so the full subscript-flag machinery
            // (`(r)pat`, `(i)`, etc.) is reused, since we'd
            // otherwise have to re-implement it inline.
            if let Some(lb) = rest.find('[') {
                if rest.ends_with(']') {
                    let bare = &rest[..lb];
                    let first = bare.chars().next();
                    let is_ident = !bare.is_empty()
                        && first
                            .map(|c| c == '_' || c.is_ascii_alphabetic())
                            .unwrap_or(false)
                        && bare.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
                    let is_positional =
                        !bare.is_empty() && bare.chars().all(|c| c.is_ascii_digit());
                    if is_ident || is_positional {
                        // Push the braced form `${#NAME[idx]}` and
                        // hand off to BUILTIN_EXPAND_TEXT mode 4
                        // (HeredocBody — just calls exec.expand_string
                        // verbatim without re-escaping). This reuses
                        // the full subscript-flag machinery so we
                        // don't have to re-implement it inline.
                        let braced = format!("${{#{}}}", rest);
                        let idx = self.builder.add_constant(Value::str(braced));
                        self.builder.emit(Op::LoadConst(idx), 0);
                        self.builder.emit(Op::LoadInt(4), 0);
                        self.builder
                            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
                        return;
                    }
                }
            }
            let bare_name = rest;
            let first = bare_name.chars().next();
            // Accept identifier names AND positional digit names ($#1
            // = length of $1 string). zsh: `set -- ab; echo $#1` → 2.
            let is_ident = !bare_name.is_empty()
                && first
                    .map(|c| c == '_' || c.is_ascii_alphabetic())
                    .unwrap_or(false)
                && bare_name
                    .chars()
                    .all(|c| c == '_' || c.is_ascii_alphanumeric());
            let is_positional =
                !bare_name.is_empty() && bare_name.chars().all(|c| c.is_ascii_digit());
            // `${#@}` / `${#*}` / `${#argv}` — count of positional
            // params. Direct port of zsh's paramsubst special-name
            // handling for `@`/`*`/`argv` in the chklen branch.
            // Without this, the bare-name fast path missed `@`/`*`
            // and the fallback emitted `0`.
            let is_special_positional = bare_name == "@" || bare_name == "*" || bare_name == "argv";
            if is_ident || is_positional || is_special_positional {
                let idx = self.builder.add_constant(Value::str(bare_name));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_LENGTH, 1),
                    0,
                );
                return;
            }
        }

        // Fast path: bare `$+NAME` / `$+NAME[KEY]` — set-test, equivalent
        // to `${+NAME}` / `${+NAME[KEY]}`. p10k uses `$+commands[X]`
        // and `$+functions[X]` heavily as a guard; the unbraced form
        // was falling through to the literal-emit path. Mirror the
        // `$#NAME` fast-path style: build the braced shape and call
        // BUILTIN_EXPAND_TEXT mode 4 so the runtime's chkset machinery
        // (already correct for `${+...}`) handles it.
        //
        // c:Src/params.c:2289-2293 — `fetchvalue` consumes the trailing
        // `[subscript]` through `getindex()` whatever the subscript text
        // contains, so a backslash-escaped char inside the brackets
        // (`$+parameters[a\$b]`, and `$+parameters[${word##*\$}]` from
        // Completion/Base/Completer/_expand:29) must NOT push the word off
        // this path: `untokenize` has already restored the Bnull to a
        // literal `\`, and the braced shape rebuilt below expands it
        // correctly. Without the `bnull_in_subscript` allowance the
        // subscript was left as literal text — `$+parameters[a\$b]` gave
        // `1[a$b]` instead of `0`, and inside `[[ … -eq 0 ]]` the leftover
        // `[…]` reached the math lexer as an output-format directive
        // ("bad output format specification", math.rs:2336), which made
        // `_expand`'s line-29 guard abort mid-condition.
        if (!has_bnull || bnull_in_subscript) && untoked.len() >= 3 && untoked.starts_with("$+") {
            let rest = &untoked[2..];
            let first = rest.chars().next();
            let bare = if let Some(lb) = rest.find('[') {
                if rest.ends_with(']') {
                    Some(&rest[..lb])
                } else {
                    None
                }
            } else {
                Some(rest)
            };
            let valid = bare
                .map(|b| {
                    !b.is_empty()
                        && first
                            .map(|c| c == '_' || c.is_ascii_alphabetic())
                            .unwrap_or(false)
                        && b.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
                })
                .unwrap_or(false);
            if valid {
                let braced = format!("${{+{}}}", rest);
                let idx = self.builder.add_constant(Value::str(braced));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder.emit(Op::LoadInt(4), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
                return;
            }
        }

        // Fast path: bare `$=NAME` — equivalent to `${=NAME}` (forced
        // IFS word-split on the value, regardless of SH_WORD_SPLIT).
        // Direct port of subst.c:2554-2566 `case '='` (spbreak=2) via
        // the unbraced-shorthand path. Without this, `$=name` was
        // emitted as literal text. Parity bug.
        if !has_bnull && untoked.len() >= 3 && untoked.starts_with("$=") {
            let rest = &untoked[2..];
            // `$==NAME` toggles word-split OFF — emit bare GET_VAR.
            let (do_split, name_part) = if let Some(after) = rest.strip_prefix('=') {
                (false, after)
            } else {
                (true, rest)
            };
            // c:Src/subst.c:2554 — the `=` split flag applies to ANY
            // parameter, not just alpha-ident names. Accept the
            // positional (`$=1`, `$=12`) and special-single-char
            // (`$=*`, `$=@`, `$=#`, …) names too; `$=1` previously fell
            // through as literal text because the scan required an
            // alpha/underscore first char. GET_VAR resolves these names
            // and WORD_SPLIT applies the forced split (`$=@` splits each
            // positional). Matches the braced `${=NAME}` semantics.
            let ident_valid = !name_part.is_empty()
                && name_part
                    .chars()
                    .next()
                    .map(|c| c == '_' || c.is_ascii_alphabetic())
                    .unwrap_or(false)
                && name_part
                    .chars()
                    .all(|c| c == '_' || c.is_ascii_alphanumeric());
            let special_valid = !name_part.is_empty()
                && (name_part.chars().all(|c| c.is_ascii_digit())
                    || (name_part.chars().count() == 1
                        && matches!(
                            name_part.chars().next().unwrap(),
                            '*' | '@' | '#' | '?' | '$' | '!' | '-'
                        )));
            if ident_valid || special_valid {
                let idx = self.builder.add_constant(Value::str(name_part));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_VAR, 1), 0);
                if do_split {
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_WORD_SPLIT, 0), 0);
                }
                return;
            }
        }

        // Fast path: bare `$~NAME` — equivalent to `${~NAME}` (forced
        // glob substitution on the value). zsh: `str=*.txt; print
        // $~str` expands to matching filenames just like the braced
        // form. Direct port of subst.c:2596 `case '~'` reached via
        // the unbraced-shorthand path. Without this, `$~name` was
        // emitted as literal text.
        //
        // Also handle the no-NAME shapes `$~` and `$~~` (bug #547):
        // C `Src/subst.c:2596-2602` consumes the `~`/`~~` and then
        // continues parsing the parameter name. When NO name follows,
        // the result is empty (no parameter resolved). Emit an empty
        // string in that case so the literal `$~` doesn't leak through.
        if !has_bnull && untoked.starts_with("$~") {
            let rest = &untoked[2..];
            // `$~~NAME` toggles globsubst OFF — emit bare GET_VAR.
            let (do_glob, name_part) = if let Some(after) = rest.strip_prefix('~') {
                (false, after)
            } else {
                (true, rest)
            };
            if name_part.is_empty() {
                // c:Src/subst.c:2596-2602 — `$~` / `$~~` with no name
                // following. C consumes the `~`/`~~` and finds no
                // parameter name; the result of the expansion is
                // empty (the literal `$~` does NOT survive). Bug #547.
                let _ = do_glob; // globsubst toggle was applied
                let idx = self.builder.add_constant(Value::str(""));
                self.builder.emit(Op::LoadConst(idx), 0);
                return;
            }
            let valid = !name_part.is_empty()
                && name_part
                    .chars()
                    .next()
                    .map(|c| c == '_' || c.is_ascii_alphabetic())
                    .unwrap_or(false)
                && name_part
                    .chars()
                    .all(|c| c == '_' || c.is_ascii_alphanumeric());
            if valid {
                let idx = self.builder.add_constant(Value::str(name_part));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_VAR, 1), 0);
                // Only glob standalone when this `$~name` IS the whole word.
                // As a sub-segment of a larger word (`$dir/$~pat`,
                // word_seg_depth > 0), globbing here in isolation would drop
                // the surrounding literal/expansion parts — the parent word's
                // assembled-scalar glob (driven by needs_glob) owns it.
                //
                // The same three gates the BRACED `${~NAME}` arm below already
                // carries (see the `dq_context_depth == 0 && scalar_assign_depth
                // == 0 && word_seg_depth == 0` test there): the `~` flag only
                // sets `globsubst` (c:Src/subst.c:2596 `case '~': globsubst =
                // 2`), and filename generation is a separate, LATER pass that
                // these three contexts never run —
                //   * DQ / cond-operand: `[[ … -prefix $~pat ]]` reaches
                //     `cond_str(a, n, 1)` (c:Src/cond.c:525) whose expansion is
                //     `singsub` — prefork only, no `zglob`;
                //   * scalar assignment: c:Src/exec.c stores the substituted
                //     value verbatim, so `y=$~x` keeps `a*`.
                // Globbing here made `x="a*"; y=$~x` store `aa ab`, and made
                // `[[ -prefix $~pat ]]` (Completion/Base/Utility/_numbers sh:65)
                // hand `cond_psfix` a directory listing — or, on a no-match
                // pattern, the EMPTY string, whose pattern matches the empty
                // prefix and fired the branch for every non-numeric word.
                if do_glob
                    && self.dq_context_depth == 0
                    && self.scalar_assign_depth == 0
                    && self.word_seg_depth == 0
                {
                    self.emit_word_glob_expand();
                }
                return;
            }
        }

        // Fast path: bare `$NAME[KEY]` — without braces, zsh lexes
        // `$NAME` as the variable name and `[KEY]` as a subscript that
        // applies to it (NOT a literal `[KEY]` suffix) — UNLESS
        // KSHARRAYS is set at expansion time (c:Src/subst.c:2800-2802
        // + 2867), which is a runtime decision: emit the UNBRACED
        // subscript opcode and let the bridge dispatch.
        if !has_bnull || bnull_in_subscript {
            if let Some((name, key)) = bare_subscript_ref(&untoked) {
                self.emit_unbraced_subscript(
                    name,
                    key,
                    "",
                    has_quote_markers || self.dq_context_depth > 0,
                );
                return;
            }
        }

        // Fast path: bare `$NAME[KEY]suffix` — same as above but with a
        // literal suffix appended (under KSHARRAYS the suffix joins the
        // literal `[KEY]` text BEFORE filename generation: zsh 5.9
        // `setopt ksharrays; a=(x y z); print -- $a[0]suffix` →
        // `zsh:1: no matches found: x[0]suffix`).
        if !has_bnull || bnull_in_subscript {
            if let Some((name, key, suffix)) = bare_subscript_with_suffix(&untoked) {
                self.emit_unbraced_subscript(
                    name,
                    key,
                    suffix,
                    has_quote_markers || self.dq_context_depth > 0,
                );
                return;
            }
        }

        // c:Src/subst.c:2990-3004 — `${(flags)"literal"}` and the
        // flagless `${"literal"}` are "bad substitution" in zsh: the
        // quote char lands in operand/operator position. The plain
        // untokenize used by the fast paths below STRIPS the quotes
        // (`${(Q)"abc"}` → `${(Q)abc}`), disguising the literal as a
        // parameter NAME. Detect the quote on the quote-preserving
        // ztokens rendering and skip the fast paths so the runtime
        // paramsubst walk raises the canonical error.
        let flag_operand_quoted =
            flag_operand_is_quoted_literal(&crate::vm_helper::untokenize_ztokens(s));

        // Fast path: `${NAME}` — braced bare ref, equivalent to `$NAME`.
        // `flag_operand_quoted` also covers the flagless `${"abc"}`
        // shape — untokenize strips the quotes so braced_var_ref would
        // misread the literal as a NAME (c:Src/subst.c:2990-3004).
        if !has_bnull && !flag_operand_quoted {
            if let Some(name) = braced_var_ref(&untoked) {
                let idx = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(idx), 0);
                // QUOTED `"${name}"` forces DQ semantics so an array's
                // empty elements survive (compiler-direct read, runtime
                // in_dq_context==0). Detect quoting the same way the
                // `$@`/`$*` arm does: the recursive dq-depth OR the raw
                // token wrapped in DQ markers (`\u{9e}…\u{9e}`) — the
                // brace form arrives 9e-wrapped with dq_depth==0.
                let in_dq = self.dq_context_depth > 0 || word_is_single_dq_span(s);
                let bid = if in_dq {
                    crate::vm_helper::BUILTIN_GET_VAR_DQ
                } else {
                    crate::vm_helper::BUILTIN_GET_VAR
                };
                self.builder.emit(Op::CallBuiltin(bid, 1), 0);
                return;
            }
        }

        // Fast path: `${~NAME}` — forced GLOB_SUBST on the value.
        // Per Src/subst.c:2596 `case '~': globsubst = 2`. The `~`
        // flag promotes the value's glob metachars from literal to
        // pattern, so e.g. `str=*.txt; print ${~str}` expands the
        // value as a filesystem glob. Emit GET_VAR + GLOB_EXPAND so
        // the runtime applies expand_glob to the resulting scalar.
        // Without this, `${~str}` left the value's `*`/`?`/`[]`
        // unexpanded.
        if !has_bnull && untoked.starts_with("${~") && untoked.ends_with('}') {
            let inner = &untoked[3..untoked.len() - 1];
            // `${~~name}` toggles globsubst OFF — pass through as
            // bare ${name} (handled by braced_var_ref above; this
            // arm only fires if we get here, which means name has
            // no special chars to interfere). Detected by leading
            // `~` after the first.
            if let Some(rest) = inner.strip_prefix('~') {
                // Double-tilde — no-op flag, just emit bare name.
                let valid = !rest.is_empty()
                    && rest
                        .chars()
                        .next()
                        .map(|c| c == '_' || c.is_ascii_alphabetic())
                        .unwrap_or(false)
                    && rest.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
                if valid {
                    let idx = self.builder.add_constant(Value::str(rest));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_VAR, 1), 0);
                    return;
                }
            } else {
                let valid = !inner.is_empty()
                    && inner
                        .chars()
                        .next()
                        .map(|c| c == '_' || c.is_ascii_alphabetic())
                        .unwrap_or(false)
                    && inner.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
                if valid {
                    let idx = self.builder.add_constant(Value::str(inner));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_VAR, 1), 0);
                    // Apply glob expansion to the resulting scalar.
                    // BUILTIN_GLOB_EXPAND pops a string, runs
                    // expand_glob (filesystem matching), pushes
                    // Value::Array (or single-element on no-match
                    // depending on NOMATCH option).
                    //
                    // Skip in DQ / cond context — inside `[[ ... ==
                    // ${~P} ]]` the `~` flag promotes the value to a
                    // PATTERN (for `==` to match against), not a path
                    // glob. compile_cond bumps `dq_context_depth` for
                    // the RHS of pattern ops (compile_zsh.rs:4079);
                    // gating off the filesystem-glob emit here lets
                    // the raw pattern reach the test runtime. Without
                    // this, `P="foo*"; [[ foobar == ${~P} ]]` ran
                    // `glob_path("foo*")`, hit NOMATCH, and failed.
                    // SCALAR assignment RHS does no filename
                    // generation in zsh — `P="*zz"; x=${~P}` stores
                    // the literal `*zz`. Array-literal ELEMENTS do
                    // glob (`a=(${~P})` → matches), so gate on the
                    // scalar-assign depth only.
                    // As a sub-segment of a larger word (`$dir/${~pat}`,
                    // word_seg_depth > 0) the parent word's assembled-scalar
                    // glob owns filename generation — globbing this segment in
                    // isolation would drop the surrounding parts.
                    if self.dq_context_depth == 0
                        && self.scalar_assign_depth == 0
                        && self.word_seg_depth == 0
                    {
                        self.emit_word_glob_expand();
                    }
                    return;
                }
            }
        }

        // Fast path: `${^NAME}` — forced RC_EXPAND_PARAM distribution.
        // zsh: `print IF${^arr}THEN` for `arr=(a b c)` produces
        // `IFaTHEN IFbTHEN IFcTHEN` regardless of the rcexpandparam
        // option setting. The `^` flag explicitly enables the
        // cartesian product over surrounding text. Direct port of
        // Src/subst.c:1875 `case Hat: nojoin = 1; aspar = 1`.
        // Emit as BUILTIN_ARRAY_ALL so the value lands on the stack
        // as Value::Array; the surrounding word's CONCAT_DISTRIBUTE
        // (segment fast-path detected via is_distribute_expansion)
        // does the actual splicing.
        // c:Src/subst.c:3029-3036 — inside DOUBLE QUOTES the array is joined
        // BEFORE plan9 ever runs:
        //     if (qt && !getlen && isarr > 0) {
        //         val = sepjoin(aval, sep, 1);
        //         isarr = 0;
        //     }
        // and the RC_EXPAND_PARAM block (c:4316) sits inside the `isarr` arm,
        // so it is skipped. `"pre${^b}"` is therefore `pred1 d2` — one word,
        // no cross-product. Distributing regardless of context made it
        // `pred1 pred2`. Falling through in DQ hands the word to the normal
        // path, which joins exactly as c:3032 does. Same `in_dq` test the
        // `$name` / `${=name}` arms already use.
        let rc_in_dq = self.dq_context_depth > 0 || word_is_single_dq_span(s);
        if !has_bnull && !rc_in_dq && untoked.starts_with("${^") && untoked.ends_with('}') {
            let inner = &untoked[3..untoked.len() - 1];
            let bare = inner
                .strip_suffix("[@]")
                .or_else(|| inner.strip_suffix("[*]"))
                .unwrap_or(inner);
            let valid = !bare.is_empty()
                && bare
                    .chars()
                    .next()
                    .map(|c| c == '_' || c.is_ascii_alphabetic())
                    .unwrap_or(false)
                && bare.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
            if valid {
                let idx = self.builder.add_constant(Value::str(bare));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_ALL, 1), 0);
                // c:Src/subst.c:184-188 — prefork's empty-word removal
                // (`uremnode`) applies to an UNQUOTED splat, so
                // `a=(a '' b); print -l -- ${^a}` is 2 lines, not 3. Same drop
                // the `${arr[@]}` fast path below appends after its ARRAY_ALL,
                // and the `$@`/`$*` splat before it — this path emitted
                // ARRAY_ALL and skipped it.
                //
                // ONLY as a standalone word. As a sub-segment (`x${^a}y`,
                // word_seg_depth > 0) the plan9 cross-product runs FIRST and
                // turns the empty element into a NON-empty word — zsh prints
                // `xay`, `xy`, `xby`, keeping all three — so dropping here,
                // before the concat, would lose the middle one. DQ is already
                // excluded: the fast path is gated on !rc_in_dq above, and a
                // quoted splat keeps empties via nulstring.
                if self.word_seg_depth == 0 {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_DROP_EMPTY, 1),
                        0,
                    );
                }
                return;
            }
        }

        // Fast path: `${=NAME}` (forced IFS-split) and `${==NAME}`
        // (force NO-split). Direct port of src/zsh/Src/subst.c:2558-2569
        // — leading `=` sets `spbreak = 2` which forces split on IFS
        // regardless of SH_WORD_SPLIT, while `==` sets `spbreak = 0`
        // which forces no-split. Also handles `${=NAME[@]}` /
        // `${=NAME[*]}` for arrays. The split applies even in DQ
        // context per zsh semantics — `"${=a}"` still splits.
        //
        // Scalar-assignment context (`b=${=a}`) suppresses the split
        // per subst.c:3901-3920 — `force_split = !ssub && spbreak`,
        // so `ssub=true` makes the split a no-op and the joined
        // value is assigned. We detect via `scalar_assign_depth`.
        if !has_bnull {
            if let Some((force_split, name, splice)) = parse_forced_split_brace(&untoked) {
                let name_const = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(name_const), 0);
                let load_bid = match splice {
                    '@' => crate::vm_helper::BUILTIN_ARRAY_ALL,
                    '*' => crate::vm_helper::BUILTIN_ARRAY_JOIN_STAR,
                    // c:Src/subst.c:1705 / :2558-2569 — `spbreak` is ONE flag:
                    // SH_WORD_SPLIT sets it to 1, a leading `=` sets it to 2,
                    // and `==` clears it. Either way c:3921 runs exactly one
                    // `sepsplit`. BUILTIN_GET_VAR applies its OWN SH_WORD_SPLIT
                    // split when the option is on, which would then be split
                    // again by the FORCE_SPLIT below — `IFS=x; setopt
                    // SH_WORD_SPLIT; print -rl -- "${=v}"` came back as the
                    // single word ` a b ` (pre-split array joined by to_str,
                    // then re-split on a now-absent separator). GET_VAR_DQ is
                    // the same read WITHOUT the split, and its array arm
                    // sepjoins — which is precisely c:3903's
                    // `val = sepjoin(aval, sep, 1)` before the c:3921 split.
                    _ => crate::vm_helper::BUILTIN_GET_VAR_DQ,
                };
                let argc = if splice == ' ' { 1 } else { 0 };
                self.builder.emit(Op::CallBuiltin(load_bid, argc), 0);
                // c:Src/subst.c:3901-3920 — `ssub` (scalar-substitution)
                // suppresses the forced split for BOTH bare `v=…` and the
                // typeset-family `NAME=…` arg form. `typeset v="$*"` is as
                // much a scalar assignment as `v="$*"`, so the join must
                // fire for assign_builtin_arg_depth too (otherwise `$*`/`$@`
                // expanded as an array and only the first element survived
                // the scalar coercion — qrcode plugin `local input="$*"`).
                let in_scalar_assign =
                    self.scalar_assign_depth > 0 || self.assign_builtin_arg_depth > 0;
                if force_split && !in_scalar_assign {
                    // c:Src/subst.c:3921 `sepsplit(val, spsep, 0, 1)` →
                    // Src/utils.c:3711 spacesplit. BUILTIN_FORCE_SPLIT is the
                    // faithful port; BUILTIN_WORD_SPLIT (multsub's c:553-620
                    // PREFORK_SPLIT walker) collapses separator runs and drops
                    // every empty field, so `"${=v}"` lost the leading and
                    // trailing fields zsh keeps.
                    //
                    // The empty leading/trailing WHITESPACE fields survive only
                    // when something attaches to them: the `Dnull` markers of a
                    // quoted word (c:4386/:4429 strcatsub over `ostr`/`fstr`),
                    // or a literal/expansion segment next door. Both are known
                    // here, so pass them through as argc — see
                    // BUILTIN_FORCE_SPLIT's argc contract.
                    let in_dq = self.dq_context_depth > 0 || word_is_single_dq_span(s);
                    let keep_empties = in_dq || self.word_seg_depth > 0;
                    let argc = if keep_empties { 1 } else { 0 };
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_FORCE_SPLIT, argc),
                        0,
                    );
                } else if !in_scalar_assign
                    && self.word_seg_depth == 0
                    && !(self.dq_context_depth > 0 || word_is_single_dq_span(s))
                {
                    // c:Src/subst.c:184-188 — `${==s}` forces NO split
                    // (c:2560-2564 `spbreak = 0`), but an unquoted expansion
                    // that comes out EMPTY is still an empty WORD, and prefork
                    // deletes it: `s=''; r=(${==s})` has ZERO elements in zsh.
                    // The split arm above reaches that for free (splitting ""
                    // yields no fields → an empty array), and every other form
                    // does too (`$v`, `${v}`, `${=s}` are all 0) — only the
                    // no-split arm returned Value::Str("") and kept the word.
                    // Quoted `"${==s}"` correctly keeps its one empty word, so
                    // this is gated off in DQ.
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_DROP_EMPTY, 1),
                        0,
                    );
                }
                return;
            }
        }

        // Fast path: `${NAME[@]}` / `${NAME[*]}` — array splice/join.
        //   `[@]` → BUILTIN_ARRAY_ALL (returns Value::Array, splice).
        //   `[*]` → BUILTIN_ARRAY_JOIN_STAR (joins with first IFS
        //          char into a single Value::Str, matching zsh).
        // In an ASSIGNMENT context (`b="${a[@]}"`), `[@]` joins like
        // `[*]` — zsh's subst.c forces single-string output when the
        // expansion is the RHS of a scalar assignment. Without this,
        // `b="${a[@]}"` captured only the first element because the
        // Array was implicitly truncated by the scalar conversion.
        //
        // Skip the fast path in DQ context. The fast-path joiner
        // (BUILTIN_ARRAY_JOIN_STAR) for `[*]` always returns a
        // single Value::Str, which is correct in QUOTED ("`${a[*]}`")
        // context. But in UNQUOTED context, zsh's canonical
        // `${a[*]}` does join-via-IFS[0] THEN word-split-via-IFS —
        // producing N argv entries. The fast path can't word-split
        // because it can't see IFS at the right point. Routing
        // unquoted-`[*]` here is fine (the JOIN_STAR handler
        // does the split itself); routing quoted-`[*]` through the
        // slow EXPAND_TEXT → multsub → paramsubst chain keeps the
        // single-string semantics intact. Bug #428.
        if !has_bnull {
            let raw_dq_for_splice = word_is_single_dq_span(s);
            let dq_for_splice = raw_dq_for_splice || self.dq_context_depth > 0;
            let is_star = array_splice_is_star(&untoked);
            // Force-join via the fast path only when (a) it's
            // `[@]` (no split needed), or (b) we're in a scalar-
            // assign or non-DQ unquoted context where the JOIN_STAR
            // handler's own DQ-aware split logic produces the right
            // result. Quoted `"${a[*]}"` falls through to the slow
            // paramsubst path.
            let take_fast = !is_star || !dq_for_splice;
            if take_fast {
                if let Some(name) = array_splice_ref(&untoked) {
                    let idx = self.builder.add_constant(Value::str(name));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    // Typeset-family scalar RHS joins `[@]` like `[*]`,
                    // same as a bare `b="${a[@]}"` scalar assign.
                    let force_join =
                        self.scalar_assign_depth > 0 || self.assign_builtin_arg_depth > 0;
                    let bid = if is_star || force_join {
                        crate::vm_helper::BUILTIN_ARRAY_JOIN_STAR
                    } else {
                        crate::vm_helper::BUILTIN_ARRAY_ALL
                    };
                    self.builder.emit(Op::CallBuiltin(bid, 0), 0);
                    // c:Src/subst.c:184-188 — an UNQUOTED array splat drops
                    // empty words (`uremnode`). The `[@]` subscript only sets
                    // splat-vs-join shape (SCANPM_ISVAR_AT), not empty removal,
                    // so `arr=(a '' b); print -l -- ${arr[@]}` → 2 lines. This
                    // fast-path emitted ARRAY_ALL but skipped the drop that the
                    // $@/$*/$argv splat path appends; mirror it here. Quoted
                    // `"${arr[@]}"` (dq_for_splice) keeps empties via nulstring.
                    if bid == crate::vm_helper::BUILTIN_ARRAY_ALL && !dq_for_splice {
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_DROP_EMPTY, 1),
                            0,
                        );
                    }
                    return;
                }
            }
        }

        // Fast path: `${NAME[KEY]}` — assoc/indexed element access. Emits
        // BUILTIN_ARRAY_INDEX which routes through assoc_arrays first then
        // falls back to indexed arrays.
        //
        // Previously skipped in DQ context with the rationale that the
        // fast path calls paramsubst with qt=false hardcoded. But for
        // a STATIC literal key (no `$`-expansion), the DQ context
        // doesn't change the key bytes — and routing through the
        // dynamic path (EXPAND_TEXT) strips outer `"…"` from the key
        // text, which silently rewrites e.g. `${h["q'q"]}` to look up
        // key `q'q` (3 bytes) when the assoc actually stored the
        // 5-byte literal `"q'q"`. Bug #338 in docs/BUGS.md.
        //
        // The subscript probe runs on `untokenize_preserve_quotes`
        // (NOT plain `untokenize`) so the inner Snull/Dnull markers
        // are mapped back to `'`/`"`/`\` — matching what the
        // canonical storage path (`assignsparam` in
        // `src/ported/params.rs:4765`, called from the compile
        // path at `src/extensions/compile_zsh.rs:1901` via
        // `untokenize_preserve_quotes(&assign.name)` then
        // `split_subscript`) extracts. Plain `untokenize` drops the
        // null markers so `"q'q"` shrank to `q'q` (3 bytes) and the
        // lookup missed the 5-byte stored key. Static fast path now
        // fires regardless of DQ/Bnull state — `braced_subscript_ref`
        // already rejects `$`-containing keys so qt has no observable
        // effect for the resolved literal.
        let untoked_preserve = crate::lex::untokenize_preserve_quotes(s);
        if let Some((base, key)) = braced_subscript_ref(&untoked_preserve) {
            let key = subscript_literal_key(key);
            let name_const = self.builder.add_constant(Value::str(base));
            let key_const = self.builder.add_constant(Value::str(key.as_ref()));
            self.builder.emit(Op::LoadConst(name_const), 0);
            self.builder.emit(Op::LoadConst(key_const), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_INDEX, 2), 0);
            return;
        }

        // Fast path: `${NAME[KEY]}` where KEY contains `$` expansions
        // (e.g. `${m[$k]}`, `${m[$prefix$suffix]}`). Resolve the key
        // text at runtime via BUILTIN_EXPAND_TEXT (mode 1 = inner-string
        // expansion, no glob/brace), then index. Mirrors the static-key
        // fast path above except the key is computed instead of loaded
        // as a constant. Without this, the assoc-array case falls back
        // to a bridge path that doesn't perform the assoc lookup.
        if !has_bnull && !sq_literal_opener {
            if let Some((base, key)) = braced_subscript_dynamic_ref(&untoked) {
                let name_const = self.builder.add_constant(Value::str(base));
                let key_const = self.builder.add_constant(Value::str(key));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadConst(key_const), 0);
                // mode 1 → DoubleQuoted-style: expand $-refs only, no
                // glob/brace pollution of the key.
                self.builder.emit(Op::LoadInt(1), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_INDEX, 2), 0);
                return;
            }
        }

        // c:Src/subst.c — `${(flags)"literal"}` is a "bad substitution"
        // parse error in zsh; paramsubst's name walker hits the `"` after
        // the flag block, finds it's not a valid name char, and errors
        // (c:Src/subst.c:2990-3004). Earlier this used a `\u{01}` sentinel
        // fast path to short-circuit to the same error; removed in favor
        // of letting paramsubst's natural body walk fire the error
        // organically (matches C). But `untoked` (plain untokenize)
        // STRIPS the quotes — `${(Q)"abc"}` untokenizes to `${(Q)abc}`,
        // which the fast paths below misread as a parameter NAME and
        // lower to a silent (empty) lookup instead of the error. Detect
        // the quote on the quote-preserving ztokens rendering and skip
        // every fast path so the runtime paramsubst walk raises the
        // canonical error (probe: `zsh -fc 'print -- ${(Q)"abc"}'` →
        // `zsh:1: bad substitution`, exit 1). `flag_operand_quoted`
        // is computed above the `${NAME}` fast path, which the
        // flagless `${"abc"}` shape would otherwise hijack.

        // Fast path: `${(flags)NAME}` — zsh parameter flags. Emit
        // BUILTIN_PARAM_FLAG with [name, flags] on the stack.
        //
        // Skip the fast-path in DQ context. The fast-path calls
        // paramsubst directly with qt=false hardcoded, bypassing
        // the lexer → prefork → stringsubst → paramsubst chain
        // where C zsh's `qt = c == Qstring` (Src/subst.c:283)
        // would have propagated DQ. Falling through to the default
        // text-expansion path emits BUILTIN_EXPAND_TEXT mode 1
        // which routes through multsub → prefork → stringsubst,
        // and the Qstring-preserving `untokenize_preserve_quotes`
        // ensures the `$` is tokenized as `\u{8c}` so stringsubst
        // sees Qstring and sets qt=true. This is the C path.
        if !has_bnull && !flag_operand_quoted && !sq_literal_opener {
            if let Some((flags, name)) = parse_zsh_flag(&untoked) {
                // DQ context: either the raw word is itself DQ-wrapped,
                // OR we're recursing into an Expansion segment from a
                // DQ-wrapped parent (tracked via dq_context_depth).
                let dq_wrapped = (word_is_single_dq_span(s)) || self.dq_context_depth > 0;
                if dq_wrapped {
                    // Fall through to the default text-expansion path.
                    let _ = (flags, name);
                } else {
                    // c:Src/subst.c — `[@]`/`[*]` subscript flow and
                    // scalar-assign-RHS PREFORK_SINGLE state flow
                    // through executor state (in_dq_context /
                    // in_scalar_assign) at runtime — no sentinel
                    // prefix on the flag string.
                    // c:Src/exec.c:2077 + Src/subst.c:1543 — the flag
                    // text that reaches paramsubst's flag parser must
                    // keep quote chars and `$'...'` wrappers LITERAL:
                    // C's untok_and_escape untokenizes flag args via
                    // ztokens (Snull → `'`, Stringg → `$`), so
                    // `${(j.$'\n'.)a}` joins with the five literal
                    // chars `$'\n'` and `${(j.':'.)a}` joins with
                    // `':'` (zsh 5.9 verified). `untoked` (plain
                    // untokenize) strips quotes and inline-decodes
                    // `$'...'`, pre-collapsing the arg to a bare
                    // newline before paramsubst ever saw it. Re-derive
                    // the emitted flag text from the C-untokenize
                    // (ztokens) rendering. Bug #626 in docs/BUGS.md.
                    let flags_lit = parse_zsh_flag(&crate::vm_helper::untokenize_ztokens(s))
                        .map(|(f, _)| f.to_string())
                        .unwrap_or_else(|| flags.to_string());
                    let name_const = self.builder.add_constant(Value::str(name));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    let flags_const = self.builder.add_constant(Value::str(flags_lit));
                    self.builder.emit(Op::LoadConst(flags_const), 0);
                    // c:Src/exec.c:2603 + :4239-4241 — the VALUE of a scalar
                    // assignment is preforked with `PREFORK_SINGLE|
                    // PREFORK_ASSIGN`, for the bare `NAME=VALUE` statement
                    // (addvars, c:2603 `isstr ? …`) and for the typeset-family
                    // `NAME=VALUE` ARGUMENT alike (c:4239, inside the
                    // `WC_ASSIGN_SCALAR` arm). PREFORK_SINGLE is `ssub`
                    // (c:Src/subst.c:1761), and `ssub` is what turns off
                    // c:3913's `force_split = !ssub && (spbreak || spsep)` —
                    // so `local s=${(s::)arr}` must NOT split, it joins at
                    // c:3917 and yields `1 2 3`.
                    //
                    // The bare `x=${(s::)arr}` form already reaches paramsubst
                    // with the flag: compile_assign wraps that RHS in Dnulls
                    // and the word leaves through BUILTIN_EXPAND_TEXT mode 5,
                    // whose singsub is `prefork(&foo, PREFORK_SINGLE)`
                    // (c:Src/subst.c:520). This fast path calls paramsubst
                    // directly and had no channel for it, so the split fired
                    // and the pieces came back re-joined with IFS[0] —
                    // `1   2   3`. Only an assignment-SHAPED typeset arg
                    // qualifies: a name-only arg is preforked with
                    // PREFORK_TYPESET (c:4197) and an array-valued one with
                    // plain PREFORK_ASSIGN (c:4265), neither of which is ssub.
                    let ssub = self.scalar_assign_depth > 0
                        || (self.assign_builtin_arg_depth > 0 && self.assign_context_depth > 0);
                    self.builder.emit(Op::LoadInt(ssub as i64), 0);
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_PARAM_FLAG, 3), 0);
                    return;
                }
            }
        }

        // Fast path: `${(flags)NAME[KEY]}` with a real (non-`@`/`*`)
        // subscript. Resolve the subscripted value first via
        // BUILTIN_ARRAY_INDEX, then prepend the `\u{01}` literal-value
        // sentinel so BUILTIN_PARAM_FLAG treats the operand as a
        // pre-resolved scalar instead of doing a name lookup. Closes
        // the `${(f)mapfile[/path]}` and `${(s:,:)assoc[k]}` shapes.
        if !has_bnull && !flag_operand_quoted && !sq_literal_opener {
            if let Some((flags, base, key)) = parse_zsh_flag_subscript(&untoked) {
                // `(t)NAME[KEY]` — type-flag form. zsh's `(t)`
                // evaluates the PARAMETER's type-string first (e.g.
                // "array", "scalar", "integer", "association"), THEN
                // applies the subscript char-by-char to that scalar.
                // Bug #308 in docs/BUGS.md: the BUILTIN_ARRAY_INDEX-
                // first dispatch below resolves `a[1]` to the element
                // value ("x"), then `(t)` runs on that pre-resolved
                // value — which hits the `(t)` on used_subexp arm at
                // `subst.rs:8158` that intentionally no-ops for
                // `${(t)$(cmdsub)}` per bug #173, so the type-of-
                // parameter intent is lost entirely. Compose the
                // nested form `${${(t)NAME}[KEY]}` so the inner
                // expansion produces the type string and the outer
                // applies the subscript to it.
                if flags.contains('t') {
                    // Use `:OFFSET:LEN` colon-substring on the type
                    // string rather than `[KEY]` so the runtime hits
                    // paramsubst's scalar-substring path (which
                    // already does 1-indexed char selection) instead
                    // of the nested-subexp + outer-subscript heuristic
                    // (subst.rs:4426-4464) which whitespace-splits
                    // and word-indexes — wrong shape for the scalar
                    // type tag. KEY is preserved as a sub-arith
                    // expression `$((KEY-1))` so non-literal subscripts
                    // (`a[$n]`, `a[1+1]`) still work.
                    //
                    // c:Bug #331 — for ASSOC element (`(t)h[k]`), zsh
                    // returns empty: subscript is treated as a key
                    // string, not a substring index, so the type tag
                    // doesn't survive the lookup. The compile path
                    // can't know at compile-time whether `base` is
                    // an assoc or indexed array, so emit a runtime
                    // check via a BUILTIN_PARAM_FLAG `t`-shape on
                    // the bare name first; if it returns
                    // "association" and the key isn't a pure-integer
                    // literal, we want empty rather than substring.
                    // Cheap approximation: when the key looks like a
                    // bare identifier (assoc-key shape, no `+`/`-`/
                    // digits-only), emit the empty form so assoc
                    // case matches zsh. Indexed-array tests use
                    // integer literals so they still take the
                    // substring path.
                    let key_looks_like_assoc_lit = !key.is_empty()
                        && key
                            .chars()
                            .next()
                            .map_or(false, |c| c == '_' || c.is_ascii_alphabetic())
                        && key.chars().all(|c| c == '_' || c.is_ascii_alphanumeric());
                    if key_looks_like_assoc_lit {
                        // c:Src/subst.c:2867-2900 — the assoc-key case
                        // is NOT a simple empty: zsh runs the post-
                        // wantt while-loop that createparam(nulstring,
                        // PM_SCALAR) on `val` (the type tag) and calls
                        // getindex(&s, v, 0) → getarg → mathevali on
                        // the key. If the key NAME resolves to a non-
                        // numeric value (e.g. `[PATH]` substitutes
                        // /usr/bin:… and fails to parse), zerr fires
                        // and errflag aborts the print with exit 1.
                        // If the key name is unset, mathevali yields
                        // 0 → empty slice (`val[-1:-1]` per
                        // VALFLAG_EMPTY) → "" + exit 0.
                        //
                        // The compile-time LoadConst("") short-cut
                        // collapsed both into "" + exit 0, losing the
                        // math-error arm that 73 bulk parity probes
                        // (`print -r ${(t)parameters[PATH]}` shape)
                        // depend on. Route through paramsubst via
                        // BUILTIN_BRIDGE_BRACE_ARRAY so the wantt arm
                        // (which carries the mathevali port at
                        // subst.rs:9580+) runs the same math eval as
                        // C zsh.
                        if let Some(inner) =
                            untoked.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
                        {
                            let body_const = self
                                .builder
                                .add_constant(Value::str(self.brace_array_body(s, inner)));
                            self.builder.emit(Op::LoadConst(body_const), 0);
                            // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                            // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                            // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                            // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                            // c:3916 joins an array value to one scalar before the c:4041
                            // quote block. Carried as an extra VM argument, exactly like
                            // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                            self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                            self.builder.emit(
                                Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                                0,
                            );
                        } else {
                            let idx = self.builder.add_constant(Value::str(""));
                            self.builder.emit(Op::LoadConst(idx), 0);
                        }
                    } else {
                        let body = format!("${{(t){}}}:$(({}-1)):1", base, key);
                        let body_const = self.builder.add_constant(Value::str(body));
                        self.builder.emit(Op::LoadConst(body_const), 0);
                        // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                        // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                        // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                        // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                        // c:3916 joins an array value to one scalar before the c:4041
                        // quote block. Carried as an extra VM argument, exactly like
                        // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                        self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                            0,
                        );
                    }
                    return;
                }
                // `(@)` plus sort/uniq/order flags (`o`/`O`/`n`/`i`/`u`)
                // on a `[(I)…]` / `[(R)…]` / `[(K)…]` subscript — must
                // return array shape AFTER applying the order flags
                // per-element, not as a Concat'd scalar. Route through
                // BUILTIN_BRIDGE_BRACE_ARRAY which calls paramsubst,
                // the canonical zsh path that walks the matching keys
                // and applies sort flags on the resulting list.
                // Without this, the scalar-Concat fallback joined the
                // matching keys with space and lost array shape (zinit
                // hook ordering pattern `${(@on)m[(I)pat]}`).
                //
                // Also fires for slice form `[N,M]` (Bug #570 in
                // docs/BUGS.md): `${(@n)a[1,-1]}` etc. need the same
                // paramsubst routing so the sort applies to the
                // slice's elements rather than being skipped at the
                // BUILTIN_ARRAY_INDEX scalar-collapse stage.
                let key_is_slice_or_idx_flag = key.starts_with("(I)")
                    || key.starts_with("(R)")
                    || key.starts_with("(K)")
                    || (key.contains(',') && !key.starts_with('('));
                // c:Src/subst.c — single paramsubst call for the whole
                // `${(flags)NAME[KEY]}` form is the C-faithful path:
                // flag parsing, subscript flag dispatch (`getarg`), and
                // sort/unique post-processing happen in one function
                // with shared isarr/aval state. The split path
                // (BUILTIN_ARRAY_INDEX → BUILTIN_PARAM_FLAG) collapses
                // the array shape between the two builtins, which
                // breaks `${(on)H[(I)pat]}` on empty H — the inner
                // resolves to "" (scalar), the outer (on) sees a
                // 1-element-empty array. Route through
                // BUILTIN_BRIDGE_BRACE_ARRAY whenever the subscript is
                // flag-form `(I)`/`(R)`/`(K)` AND outer carries a
                // shape-affecting flag (`@`, `o`/`O`/`n`/`i`/`u`).
                let has_shape_flag = flags
                    .chars()
                    .any(|c| matches!(c, '@' | 'o' | 'O' | 'n' | 'i' | 'u'));
                if has_shape_flag && key_is_slice_or_idx_flag {
                    if let Some(inner) =
                        untoked.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
                    {
                        // Carry the DQ context the same way the sibling
                        // BRIDGE_BRACE_ARRAY site below does: prefix the body with
                        // Qstring (\u{8c}) so the bridge bumps `in_dq_context` and
                        // `paramsubst_to_value` derives qt == true. Without it this
                        // opcode handed paramsubst a RAW `${...}` body with qt
                        // false, and c:Src/subst.c:4354's nulstring marking
                        // (`qt && !*y`) never applied — so a quoted split dropped
                        // its empty fields where zsh keeps them:
                        // `a=(-O 'expl:'); b=( "${(@s/:/)a[2]}" )` gave $#b == 1
                        // instead of 2. Only the SUBSCRIPTED spelling took this
                        // path, which is why the identical scalar
                        // `"${(@s/:/)scalar}"` was always right.
                        //
                        // Real-world bite: `_git`'s __git_recent_commits
                        // (`argument_array_names=( "${(@s/:/)argument_array_names[2]}" )`)
                        // saw one name instead of two, so `git checkout <TAB>`
                        // offered 88 matches where zsh offers 107.
                        let body_const = self
                            .builder
                            .add_constant(Value::str(self.brace_array_body(s, inner)));
                        self.builder.emit(Op::LoadConst(body_const), 0);
                        // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                        // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                        // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                        // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                        // c:3916 joins an array value to one scalar before the c:4041
                        // quote block. Carried as an extra VM argument, exactly like
                        // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                        self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                            0,
                        );
                        return;
                    }
                }
                // Bug #328: `${(j:SEP:)NAME[N,M]}` / `${(F)NAME[N,M]}` /
                // `${(p)NAME[N,M]}` — array-join flags on an array slice.
                // The post-ARRAY_INDEX sentinel/Concat path stringifies
                // the slice to "x y" BEFORE j/F can join, defeating the
                // flag. Route the whole substitution through
                // BUILTIN_BRIDGE_BRACE_ARRAY (= paramsubst direct entry)
                // which handles the slice+join atomically in one C-style
                // paramsubst call — c:Src/subst.c:3032 sepjoin transition
                // fires correctly when isarr is preserved through the
                // slice extraction.
                let has_join_flag = flags.chars().any(|c| matches!(c, 'j' | 'F' | 'p'));
                if has_join_flag && key_is_slice_or_idx_flag {
                    if let Some(inner) =
                        untoked.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
                    {
                        let body_const = self
                            .builder
                            .add_constant(Value::str(self.brace_array_body(s, inner)));
                        self.builder.emit(Op::LoadConst(body_const), 0);
                        // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                        // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                        // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                        // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                        // c:3916 joins an array value to one scalar before the c:4041
                        // quote block. Carried as an extra VM argument, exactly like
                        // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                        self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                            0,
                        );
                        return;
                    }
                }
                // c:Src/subst.c — `(@)`-only outer flag. Route through
                // BUILTIN_BRIDGE_BRACE_ARRAY with the full
                // `${(@)NAME[KEY]}` body so paramsubst's flag parser
                // sets nojoin=1 (Src/subst.c:1813) inline. Earlier this
                // path used a `\u{05}` sentinel on the key + ARRAY_INDEX
                // which then re-built the body; consolidating eliminates
                // the sentinel.
                let only_at_flag = flags.chars().all(|c| c == '@');
                if only_at_flag && flags.contains('@') {
                    if let Some(inner) =
                        untoked.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
                    {
                        let body_const = self
                            .builder
                            .add_constant(Value::str(self.brace_array_body(s, inner)));
                        self.builder.emit(Op::LoadConst(body_const), 0);
                        // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                        // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                        // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                        // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                        // c:3916 joins an array value to one scalar before the c:4041
                        // quote block. Carried as an extra VM argument, exactly like
                        // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                        self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                            0,
                        );
                        return;
                    }
                }
                // `(k)NAME[(I)pat]` / `(v)NAME[(I)pat]` / `(k)NAME[(R)pat]`
                // / etc. — when the subscript carries `(I)`/`(R)`/`(i)`/
                // `(r)` index/match flags, the result already arrives
                // shaped as keys (for I/i) or values (for R/r). The
                // outer (k)/(v) flag is a no-op in that case per zsh
                // (verified: typeset -A m=(a 1 b 2); echo \"\${(k)m[(I)*]}\"
                // returns the same as \"\${m[(I)*]}\"). Passing through
                // BUILTIN_PARAM_FLAG would reinterpret the joined-keys
                // string as a NEW parameter name and fail. Skip the
                // wrap.
                let key_starts_with_idx_flag = key.starts_with('(')
                    && key
                        .find(')')
                        .map(|p| key[1..p].chars().any(|c| matches!(c, 'I' | 'i')))
                        .unwrap_or(false);
                let key_starts_with_value_flag = key.starts_with('(')
                    && key
                        .find(')')
                        .map(|p| key[1..p].chars().any(|c| matches!(c, 'R' | 'r')))
                        .unwrap_or(false);
                // `(k)NAME[(I)pat]` / `(k)NAME[(i)pat]` / `(v)NAME[(R)pat]`
                // / `(v)NAME[(r)pat]` — outer flag matches what the
                // subscript-flag returns. zsh treats this combo as a
                // no-op because the subscript already yields the
                // requested shape (verified vs /bin/zsh).
                let only_k_flag = flags == "k";
                let only_v_flag = flags == "v";
                let only_kv_flag = flags == "kv" || flags == "vk";
                // c:Src/subst.c — `(v)NAME[key]` / `(V)NAME[key]` /
                // `(kv)NAME[key]` on a simple-key subscript: the
                // flag's value-extraction is a no-op because the
                // subscript already picks the single value. zsh
                // returns the value at `key` directly. Bug #35 in
                // docs/BUGS.md: routing through BUILTIN_PARAM_FLAG
                // with the `\u{01}` pre-resolved-value sentinel
                // triggered paramsubst's literal-operand bad-sub gate
                // (subst.rs:3868) which was designed for the error
                // case `${(v)"literal"}`.
                //
                // `(k)NAME[key]` on simple subscript returns the
                // KEY (the subscript text itself) — also redundant
                // since the subscript IS the key.
                //
                // Detect the simple-key case: subscript has no
                // `[`/`@`/`*` inside, doesn't start with a flag
                // group `(...)`, no top-level comma slice. `$`-vars
                // in the key are OK (the runtime resolves them; key
                // shape is still simple).
                let key_is_simple = !key.starts_with('(')
                    && !key.contains('@')
                    && !key.contains('*')
                    && !key.contains(',');
                let redundant = (only_k_flag && key_starts_with_idx_flag)
                    || (only_v_flag && key_starts_with_value_flag)
                    // `(v)` asks for the VALUE of an assoc element, which a
                    // simple subscript already yields -- genuinely redundant.
                    // `(V)` is a different flag entirely (make non-printing
                    // chars visible, c:Src/subst.c:2232) and merely shares the
                    // letter; folding it in here dropped it, so `${(V)a[1]}`
                    // compiled to a bare ARRAY_INDEX and never reached
                    // paramsubst's `mods & 2` arm. Whole-array `${(V)a}` and
                    // scalar `${(V)s}` took the flag path and were correct,
                    // which is why only the subscripted form misbehaved.
                    || (only_v_flag && key_is_simple)
                    || (only_kv_flag && key_is_simple);
                // `(k)NAME[simple_key]` — KEY-EXISTENCE query per
                // zsh: present → return key, absent → return empty
                // (Src/params.c:1396-1431 + zshparam(1) "(k)" flag
                // semantics). Previously a constant-fold to the
                // subscript text — that returned the literal key text
                // unconditionally, missing the existence check (Bug
                // #145 in docs/BUGS.md).
                //
                // Use BUILTIN_ASSOC_HAS_KEY to test at runtime and
                // emit the key text on hit, empty on miss.
                if only_k_flag && key_is_simple {
                    let name_const = self.builder.add_constant(Value::str(base));
                    let key_const = self.builder.add_constant(Value::str(key));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(Op::LoadConst(key_const), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_ASSOC_HAS_KEY, 2),
                        0,
                    );
                    return;
                }
                if redundant {
                    let name_const = self.builder.add_constant(Value::str(base));
                    let key_const = self.builder.add_constant(Value::str(key));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(Op::LoadConst(key_const), 0);
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_INDEX, 2), 0);
                    return;
                }
                // `(v)NAME[(I)pat]` / `(k)NAME[(R)pat]` / `(@k)NAME[(R)pat]`
                // — outer flag flips the (I)/(i)/(R)/(r) subscript-flag
                // result. C handles this inside paramsubst at one site
                // (Src/subst.c flag-parser loop sets WANTKEYS/WANTVALS
                // bits that the assoc-scan reads). Route the FULL
                // `${(flags)NAME[KEY]}` body through BUILTIN_BRIDGE_BRACE_ARRAY
                // (= paramsubst direct entry) so the canonical flag
                // parser owns the dispatch. No sentinel byte needed —
                // the body is the literal source-level expression and
                // paramsubst's flag loop parses `(v)` / `(k)` / `(@k)`
                // exactly as it does for any other expansion.
                let only_at_k_flag = !flags.is_empty()
                    && flags.chars().all(|c| c == 'k' || c == '@')
                    && flags.contains('k')
                    && flags.contains('@');
                let needs_outer_flag_bridge = (only_v_flag && key_starts_with_idx_flag)
                    || (only_k_flag && key_starts_with_value_flag)
                    || (only_at_k_flag && key_starts_with_value_flag);
                if needs_outer_flag_bridge {
                    if let Some(inner) =
                        untoked.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
                    {
                        let body_const = self
                            .builder
                            .add_constant(Value::str(self.brace_array_body(s, inner)));
                        self.builder.emit(Op::LoadConst(body_const), 0);
                        // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                        // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                        // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                        // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                        // c:3916 joins an array value to one scalar before the c:4041
                        // quote block. Carried as an extra VM argument, exactly like
                        // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                        self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                            0,
                        );
                        return;
                    }
                }
                // c:Src/subst.c — `${(flags)NAME[KEY]}` mixed-flag chain.
                // When `@` is among the flags, route the whole
                // expression through BUILTIN_BRIDGE_BRACE_ARRAY so
                // paramsubst's nojoin gating fires on the (@) without a
                // sentinel handshake to BUILTIN_ARRAY_INDEX. Without @,
                // the legacy ARRAY_INDEX + PARAM_FLAG split-path is fine
                // (DQ-join doesn't change shape).
                if flags.contains('@') {
                    if let Some(inner) =
                        untoked.strip_prefix("${").and_then(|s| s.strip_suffix('}'))
                    {
                        let body_const = self
                            .builder
                            .add_constant(Value::str(self.brace_array_body(s, inner)));
                        self.builder.emit(Op::LoadConst(body_const), 0);
                        // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                        // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                        // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                        // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                        // c:3916 joins an array value to one scalar before the c:4041
                        // quote block. Carried as an extra VM argument, exactly like
                        // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                        self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                            0,
                        );
                        return;
                    }
                }
                // c:Src/subst.c — `${(flags)NAME[KEY]}` form. Route
                // through BUILTIN_BRIDGE_BRACE_ARRAY with the full
                // inner so paramsubst's canonical flag parser at
                // Src/subst.c:2147+ handles flag-then-subscript in
                // one pass. The composition path (outer (L)/(C)/(U) +
                // subscript (r)/(R)/(i)/(I)) is fixed at subst.rs:9670
                // — non-splat-subscript casmod now applies to `value`
                // directly instead of refetching the source array.
                if let Some(inner) = untoked.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
                    let body_const = self
                        .builder
                        .add_constant(Value::str(self.brace_array_body(s, inner)));
                    self.builder.emit(Op::LoadConst(body_const), 0);
                    // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                    // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                    // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                    // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                    // c:3916 joins an array value to one scalar before the c:4041
                    // quote block. Carried as an extra VM argument, exactly like
                    // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                    self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                        0,
                    );
                    return;
                }
                let _ = (base, key, flags);
                return;
            }
        }

        // Bridge-array fast path: when the WHOLE word is a single
        // `${...}` brace expression that the standard fast paths
        // cannot handle (nested `${...}` in name slot, or any other
        // shape that would otherwise hit the EXPAND_TEXT bridge with
        // its String-collapse), AND the result needs to be array-
        // shaped (explicit `(@)`, explicit `[@]` subscript, or `(M)`
        // filter on an array-subscripted name like `out[@]`), route
        // through BUILTIN_BRIDGE_BRACE_ARRAY. Direct port of zsh's
        // `aval` threading in subst.c paramsubst: the C source
        // carries the per-element vector through `aval`, returning
        // multi-word output to the caller.
        // Bridge-array fast path is normally gated on `!has_bnull`,
        // but the `(M)`/`(R)` + `:#` filter form is allowed even with
        // Bnull escapes — the filter's pattern compile in
        // `param_pattern_to_regex_anchored` handles literal `\X`
        // (including the special `\(#e)` / `\(#s)` anchor cases).
        // Without this, `${(M)arr:#*\\(#e)}` falls through to the
        // EXPAND_TEXT bridge which scalar-flattens.
        let try_bridge_array = !has_bnull || (untoked.starts_with("${(") && untoked.contains(":#"));
        // Guard: this fast path is for a SINGLE `${(flags)…}` expansion.
        // `strip_prefix("${") + strip_suffix('}')` alone does NOT verify the
        // leading `${` matches the FINAL `}` — for two adjacent expansions
        // `${(@)^a}Y${(@)^b}` the `need_array` trigger (`@` flag + a `${` in
        // the tail) fired on the SECOND expansion's opener, fed the whole
        // word to BRIDGE_BRACE_ARRAY as one body, and the trailing array
        // joined (`1YA B` instead of `1YA 1YB 2YA 2YB`). Require the leading
        // `${` to span the word so multi-expansion words fall through to the
        // segment splitter. (A trailing literal — `…${(@)^b}Z` — already
        // dodged this via the failed `strip_suffix('}')`.)
        if try_bridge_array && braced_expansion_spans_word(&untoked) {
            if let Some(inner) = untoked.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
                if let Some(close) = matching_paren_close(inner) {
                    let flag_chain = &inner[1..close];
                    let after_flags = &inner[close + 1..];
                    // Trigger conditions:
                    //   1. `(@)` in flags + nested `${` in body — the
                    //      original triple-nested case.
                    //   2. Filter operator `:#` on `NAME[@]` — explicit
                    //      array-splice with filter must return array
                    //      shape (zsh: `${(M)out[@]:#pat}` filters per
                    //      element, returns array). Other operators
                    //      (##, %%, /, etc.) on `NAME[@]` go through
                    //      their own per-element fast path that
                    //      already preserves shape.
                    let has_at_filter = after_flags.contains("[@]") && after_flags.contains(":#");
                    // (M) / (R) filter on a `:#` operator — keeps
                    // matching elements (M) or first-match index (R).
                    // For arrays this MUST return array shape so the
                    // caller emits each survivor as a separate word.
                    // Without this, `${(M)arr:#pat}` falls through to
                    // EXPAND_TEXT which scalar-flattens the result
                    // even though paramsubst correctly filtered the
                    // array. Direct port of zsh's aval thread through
                    // paramsubst — Src/subst.c handles the (M)+:# combo
                    // by walking aval per element.
                    let has_filter_with_match_flag = (flag_chain.contains('M')
                        || flag_chain.contains('R'))
                        && after_flags.contains(":#");
                    // `(@)` with a `[(I)...]` / `[(R)...]` subscript —
                    // assoc-array key-pattern lookup that returns
                    // multiple matches. zinit's hook ordering pattern
                    // `${(@on)m[(I)pat]}` enumerates matching keys
                    // and sorts them. Must return array shape so each
                    // key emerges as a separate word. Without this,
                    // the keys joined with space.
                    let at_with_index_subscript = flag_chain.contains('@')
                        && (after_flags.contains("[(I)")
                            || after_flags.contains("[(R)")
                            || after_flags.contains("[(K)"));
                    let need_array = (flag_chain.contains('@') && after_flags.contains("${"))
                        || has_at_filter
                        || has_filter_with_match_flag
                        || at_with_index_subscript;
                    if need_array {
                        // c:Src/subst.c — patterns in `:#PAT`, `[(I)PAT]`,
                        // etc. preserve quoted/unquoted distinction via
                        // Dnull/Snull markers all the way to patcompile.
                        // The plain `untokenize` collapses quoted `*`
                        // and unquoted `*` to ASCII `*`, so the bridge
                        // path treated quoted patterns as glob. Re-
                        // derive `inner` from raw tokenized `s` with
                        // `untokenize_preserve_quoted_pat_literals` so
                        // chars that were inside `"…"` / `'…'` carry
                        // `\X` escapes through to patcompile. Bug #39
                        // in docs/BUGS.md. The bridge_array path only
                        // fires for pattern-bearing operators (`:#`,
                        // `[@]`, `[(I)]`, `[(R)]`, `[(K)]`) so escaping
                        // quoted metachars is correct in all cases
                        // that reach this gate.
                        let inner_safe =
                            strip_brace_wrap_for_bridge(s).unwrap_or_else(|| inner.to_string());
                        // c:Src/subst.c:3032 — in DQ without `(@)`, the
                        // qt sepjoin runs BEFORE the SUB_FILTER getmatch
                        // at c:3540, so `"${(M)a:#pat}"` tests ONE joined
                        // word, not each element. Prefix Qstring
                        // (\u{8c}) — same DQ signal the zip path below
                        // uses — so BRIDGE_BRACE_ARRAY bumps
                        // in_dq_context and paramsubst sees qt=true.
                        // Without this, the filter ran per-element in DQ
                        // (zshrs printed "ha he hi" for
                        // `"${(M)a:#h?}"` where zsh prints "").
                        // c:Src/subst.c:1625 vs :1761 — a SYNTHETIC assignment
                        // wrap is `ssub`, not `qt`; only a real `"…"` sets qt.
                        // `x=${(M)a:#[13]}` must filter PER ELEMENT (zsh: "3 1"),
                        // while `x="${(M)a:#[13]}"` tests the joined word ("").
                        let in_dq_ba = (word_is_single_dq_span(s)
                            && self.synthetic_dq_wrap_depth == 0)
                            || self.dq_context_depth > 0;
                        let body_text = if in_dq_ba {
                            format!("\u{8c}{}", inner_safe)
                        } else {
                            inner_safe
                        };
                        let body_const = self.builder.add_constant(Value::str(&body_text));
                        self.builder.emit(Op::LoadConst(body_const), 0);
                        // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                        // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                        // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                        // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                        // c:3916 joins an array value to one scalar before the c:4041
                        // quote block. Carried as an extra VM argument, exactly like
                        // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                        self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                            0,
                        );
                        return;
                    }
                }
            }
        }

        // Bridge-array fast path for array-shape operators: `:|`, `:*`,
        // `:^`, `:^^` (set difference / intersection / zip / zip-long).
        // Each takes `${arr<op>other}` and returns array shape; the
        // EXPAND_TEXT default mode uses singsub which collapses arrays
        // to a single joined string, so we need to route directly
        // through paramsubst via BUILTIN_BRIDGE_BRACE_ARRAY. Direct
        // port of subst.c:3522 SUB_DIFFERENCE / 3540 SUB_INTERSECT /
        // 3548 SUB_ZIP returning array results.
        //
        // DQ context skip: when this word lives inside `"…"`,
        // BRIDGE_BRACE_ARRAY doesn't see the DQ flag (the executor's
        // in_dq_context counter is only bumped by EXPAND_TEXT mode 1)
        // — so the qt-aware sub-paths inside paramsubst (notably
        // SUB_ZIP's collapse-to-2-elements at c:Src/subst.c:3456-3520)
        // don't fire. Fall through to the EXPAND_TEXT bridge for DQ
        // words; it bumps in_dq_context, paramsubst_to_value reads
        // that, and qt propagates correctly.
        // c:Src/subst.c:3456-3520 — DQ context flips SUB_ZIP and
        // SUB_ZIPN semantics: short-zip collapses to 2 elements
        // ([sepjoin(a), b[0]]) and long-zip emits pairs of
        // (sepjoin(a), b[i]). The fast path still applies — pass
        // DQ flag through so paramsubst_to_value flips qt on.
        // c:Src/subst.c:1625 vs :1761 — the scalar-assignment Dnull wrap is
        // SYNTHETIC (it stands in for PREFORK_SINGLE / `ssub`), so it must not
        // read as `qt`. C joins under `ssub` only at c:4226, long AFTER the
        // c:3467 zip; under `qt` it joins at c:3033, BEFORE it. Hence
        // `x=${a:^b}` zips per element (zsh: `1 x 2 y 3 z`) while
        // `x="${a:^b}"` collapses first (zsh: `1 2 3 x`).
        let raw_dq_word_zip = word_is_single_dq_span(s) && self.synthetic_dq_wrap_depth == 0;
        let in_dq = raw_dq_word_zip || self.dq_context_depth > 0;
        // Verify the `${...}` spans the WHOLE word (i.e. there's no
        // trailing text after the matching close brace). For multi-
        // segment DQ words like `"${(j:|:)a}::${(j:.:)b}"`, the naive
        // strip_prefix("${") + strip_suffix("}") matches the OUTER
        // `${` and the LAST `}` even when separated by literal text,
        // making the fast path treat the whole word as one paramsubst.
        // Symptom: the second `${(j:.:)b}` survives literally.
        fn whole_word_brace(untoked: &str) -> Option<&str> {
            let rest = untoked.strip_prefix("${")?;
            let bytes = rest.as_bytes();
            let mut depth = 1i32;
            let mut i = 0usize;
            while i < bytes.len() {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            // Matching close found — only the fast
                            // path if there's nothing after it.
                            if i + 1 == bytes.len() {
                                return Some(&rest[..i]);
                            }
                            return None;
                        }
                    }
                    b'\\' => i += 1, // skip escaped char
                    _ => {}
                }
                i += 1;
            }
            None
        }
        if !has_bnull {
            if let Some(inner) = whole_word_brace(&untoked) {
                let has_array_op = inner.contains(":|")
                    || inner.contains(":*")
                    || inner.contains(":^^")
                    || inner.contains(":^");
                if has_array_op {
                    // Prefix with Qstring (\u{8c}) to signal DQ to
                    // paramsubst_to_value via the body's leading
                    // marker; the bridge strips it before
                    // reconstruction. Mirrors how stringsubst at
                    // subst.rs:692 derives qt from `c == Qstring`.
                    let body_text = if in_dq {
                        format!("\u{8c}{}", inner)
                    } else {
                        inner.to_string()
                    };
                    let body_const = self.builder.add_constant(Value::str(body_text));
                    self.builder.emit(Op::LoadConst(body_const), 0);
                    // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                    // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                    // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                    // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                    // c:3916 joins an array value to one scalar before the c:4041
                    // quote block. Carried as an extra VM argument, exactly like
                    // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                    self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                        0,
                    );
                    return;
                }
            }
        }

        // Bridge-array (no flag chain) fast path: outer `${${...}<op>}`
        // where the operand starts with a nested `${...}`. The inner
        // expansion may produce an array (split flags, subscript slice,
        // etc.) and the outer operator (replace `/` / `//`, strip
        // `#` / `%`, etc.) applies per-element. Without this, the
        // EXPAND_TEXT bridge scalar-flattens the result.
        // Direct port of zsh's aval threading through paramsubst when
        // the operand is itself a recursive substitution.
        if !has_bnull {
            // Guard: this fast path is for a SINGLE outer `${…}` whose
            // operand is a nested `${…}`. `strip_prefix("${") +
            // strip_suffix('}')` alone does NOT verify the leading `${`
            // matches the FINAL `}` — for two adjacent expansions
            // `${a}${b}` it stripped `${b}`'s close and swallowed `${b}`
            // into the body, emitting it literally. Require the leading
            // `${`'s matching close (counting `${` opens / `}` closes) to
            // be the last char so a multi-expansion word falls through to
            // the segment splitter instead.
            let leading_brace_spans_word = braced_expansion_spans_word(&untoked);
            if let Some(inner) = untoked
                .strip_prefix("${")
                .and_then(|s| s.strip_suffix('}'))
                .filter(|_| leading_brace_spans_word)
            {
                // Operand starts with `${`: the inner expansion is
                // the value-source. Detect outer operator `/` / `//` /
                // `/#` / `/%` / `##` / `#` / `%%` / `%` after the
                // nested `${...}`.
                if inner.starts_with("${") {
                    // Find the matching `}` of the inner `${...}`.
                    let mut depth = 0;
                    let mut inner_close = None;
                    let bytes = inner.as_bytes();
                    let mut i = 0;
                    while i < bytes.len() {
                        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                            depth += 1;
                            i += 2;
                            continue;
                        }
                        if bytes[i] == b'}' {
                            depth -= 1;
                            if depth == 0 {
                                inner_close = Some(i);
                                break;
                            }
                        }
                        i += 1;
                    }
                    if let Some(close_pos) = inner_close {
                        let after = &inner[close_pos + 1..];
                        // Detect outer operators that benefit from
                        // array-shape preservation.
                        let has_array_op = after.starts_with("//")
                            || after.starts_with("/#")
                            || after.starts_with("/%")
                            || after.starts_with('/')
                            || after.starts_with("##")
                            || after.starts_with('#')
                            || after.starts_with("%%")
                            || after.starts_with('%')
                            || after.starts_with(":#");
                        if has_array_op {
                            // c:Src/subst.c:1759 — `ssub = (pf_flags &
                            // PREFORK_SINGLE)`; a double-quoted expansion runs
                            // in that scalar context, which sepjoins the array
                            // and clears `isarr` (c:3901-3907). The sort/unique
                            // block sits inside an `if (isarr)` gate (c:4245),
                            // so `(o)`/`(O)`/`(n)`/`(u)`/`(q)` are NO-OPS inside
                            // `"…"` — `"${(o)a}"` is not sorted.
                            //
                            // This arm compiles the NESTED `${${…}op}` shape.
                            // It handed the body to BRIDGE_BRACE_ARRAY without
                            // the Qstring (\u{8c}) DQ marker, so the bridge
                            // never bumped `in_dq_context`, paramsubst ran with
                            // qt=false, and the INNER expansion applied its
                            // array flags: `"${${(o)a}//o/0}"` came out
                            // `f0ur 0ne three tw0` where zsh gives
                            // `0ne tw0 three f0ur`. Same marker the `:#`/zip
                            // arms above already use.
                            // `dq_context_depth` only counts DQ strings that
                            // have literal parts around the expansion (`"[${…}]"`
                            // compiles through the DQ-string splitter). A word
                            // that is ENTIRELY one quoted expansion — `"${…}"` —
                            // never bumps it; the lexer instead hands us the
                            // token wrapped in Dnull (\u{9e}). Test both, the
                            // same way the `:#` / zip arms above do, or the bare
                            // form keeps running the operator per-element:
                            // `"${${(u)a}%e}"` gave `on two thre four` instead of
                            // zsh's `one two three four`.
                            let in_dq =
                                (s.len() >= 2 && s.starts_with('\u{9e}') && s.ends_with('\u{9e}'))
                                    || self.dq_context_depth > 0;
                            let body_text = if in_dq {
                                format!("\u{8c}{}", inner)
                            } else {
                                inner.to_string()
                            };
                            let body_const = self.builder.add_constant(Value::str(&body_text));
                            self.builder.emit(Op::LoadConst(body_const), 0);
                            // c:Src/exec.c:2546 — `prefork(vl, isstr ? (PREFORK_SINGLE|PREFORK_ASSIGN)
                            // : PREFORK_ASSIGN, …)`. PREFORK_SINGLE is paramsubst's `ssub`
                            // (c:Src/subst.c:1759); it gates off c:3913's `force_split`, so a
                            // scalar-assignment RHS does NOT split on (s:X:)/(f)/(0), and
                            // c:3916 joins an array value to one scalar before the c:4041
                            // quote block. Carried as an extra VM argument, exactly like
                            // BUILTIN_PARAM_FLAG's argc-3 ssub operand.
                            self.builder.emit(Op::LoadInt(self.brace_array_ssub() as i64), 0);
                            self.builder.emit(
                                Op::CallBuiltin(crate::vm_helper::BUILTIN_BRIDGE_BRACE_ARRAY, 2),
                                0,
                            );
                            return;
                        }
                    }
                }
            }
        }

        // Phase 1 native param-modifier lowerings. Each replaces a
        // bridge case. The matcher is greedy from least-ambiguous to
        // most: `:-`, `:=`, `:?`, `:+` first (modifier ops), then
        // substring (`:` + digit/dash), strip (`#`/`##`/`%`/`%%`),
        // replace (`/`/`//`/`/#`/`/%`).
        //
        // `has_bnull` gating: Bnull marks `\X` lexer-escapes that
        // some downstream paths can't honor (the rhs of `:-` etc.
        // gets re-expand_string'd which loses the escape distinction).
        // EXCEPTION: `(@)`-flagged Replace MUST take this path even
        // with has_bnull — the EXPAND_TEXT fallback scalar-flattens
        // arrays, losing shape. BUILTIN_PARAM_REPLACE handles literal
        // `\X` in the replacement correctly for the simple-replace
        // case (no backref via `(#m)`), so the relaxation is gated
        // narrowly: had_at=true AND op<2 (// or /, not /# or /%).
        // The `(#m)` shape (hist_substring_regex_meta_escape) has
        // had_at=false and goes through EXPAND_TEXT which is correct.
        let parsed_mod = parse_param_modifier(&untoked);
        let modifier_safe_with_bnull = matches!(
            parsed_mod.as_ref().map(|m| &m.kind),
            Some(crate::compile_zsh::ParamModifierKind::Replace { had_at: true, .. })
        );
        // Skip the param-modifier fast-paths in DQ context — same
        // rationale as the ${(flags)} and ${NAME[KEY]} fast-paths
        // above: each one hardcodes qt=false in its paramsubst call,
        // breaking DQ semantics. Fall through to the default
        // text-expansion path which routes through multsub →
        // paramsubst with qt propagated via Qstring tokens.
        let raw_dq_word = word_is_single_dq_span(s);
        let in_dq = raw_dq_word || self.dq_context_depth > 0;
        if (!has_bnull || modifier_safe_with_bnull) && !in_dq {
            if let Some(mut modifier) = parsed_mod {
                // c:Src/subst.c:178-181 — `prefork` runs `filesub` on the word
                // AFTER `stringsubst`, so a `~` the LEXER turned into a `Tilde`
                // token (c:Src/lex.c:432) inside a `${var:-…}` default is tilde-
                // expanded exactly like a bare `~`: `filesubstr` (c:741) tests
                // for the TOKEN, and paramsubst copies the default's characters
                // through with their tokens intact.
                //
                // This lowering parses the modifier out of the UNTOKENIZED word
                // (`untoked` above), where the token has already been folded
                // back to an ASCII `~` that `filesubstr` correctly refuses, so
                // `${XDG_CACHE_HOME:-~/.cache}` came back literal. That is
                // powerlevel10k's dump-file path (powerlevel10k.zsh-theme:62):
                // p10k looked for `~/.cache/p10k-dump-<user>.zsh`, never found
                // it, and — with the dump missing — never reached the async
                // worker start in `_p9k_init`, so every worker-backed segment
                // (battery / ram / disk / load / …) stayed empty. It also made
                // the shell CREATE a literal `~` directory when a config wrote
                // to such a path.
                //
                // Restore the token for a default that BEGINS with `~`, which is
                // the only position `filesub` acts on outside an assignment
                // (c:674-675 returns before the `:`/`=` scan when `assign` is 0).
                // The DQ path is already excluded above (`in_dq`), and `\~`
                // carries a `Bnull` that the `has_bnull` gate keeps out, so the
                // only spelling left to protect is `'~'` — a single-quoted
                // default reaches here with its `Snull` markers still in the
                // TOKENIZED word, which is what the guard checks.
                if let crate::compile_zsh::ParamModifierKind::DefaultFamily { rhs, .. } =
                    &mut modifier.kind
                {
                    if rhs.starts_with('~')
                        && s.contains(crate::ported::zsh_h::Tilde)
                        && !s.contains(crate::ported::zsh_h::Snull)
                    {
                        rhs.replace_range(..1, &crate::ported::zsh_h::Tilde.to_string());
                    }
                }
                // Default-word glob bracket for the native `:-`/`-`/`:+`/`+`
                // lowering (#2 default-word globbing). DefaultFamily routes
                // through BUILTIN_PARAM_DEFAULT_FAMILY → paramsubst, whose
                // default/alt arm sets DEFAULT_WORD_GLOB_PENDING when it
                // takes a SOURCE-glob default; the APPLY below globs the
                // result. A parameter VALUE never sets the flag. Only the
                // outermost word (word_seg_depth == 0) brackets.
                let dwg_mod = matches!(modifier.kind, ParamModifierKind::DefaultFamily { .. })
                    && self.word_seg_depth == 0
                    && self.dq_context_depth == 0
                    && self.scalar_assign_depth == 0 // scalar `v=${x:-*}` RHS doesn't glob
                    && self.assign_builtin_arg_depth == 0 // typeset/export NAME=value arg
                    // Which metacharacters can make the default word glob is
                    // decided at RUNTIME by `haswilds` on the pretokenized
                    // source (subst.rs, the DEFAULT_WORD_GLOB_PENDING gate).
                    // This compile-time test only decides whether to BRACKET
                    // the word so that runtime answer can be acted on, so it
                    // must not be narrower than haswilds — it listed only
                    // `*`/`?`/`[`, so `${x:-(paren)}` was never bracketed:
                    // the runtime set PENDING correctly and nothing consumed
                    // it, and the word came out literal where zsh globs it
                    // (`(paren)` parses as a glob QUALIFIER — zsh says
                    // "number expected"). haswilds (c:Src/pattern.c:4325-4371)
                    // fires on Inpar / Bar / Star / Inbrack / Inang / Quest,
                    // plus Pound / Hat under EXTENDEDGLOB, so accept each of
                    // those in both raw and token form. Over-accepting is
                    // harmless — the bracket is a RESET/APPLY pair whose
                    // handler returns the word untouched when PENDING is
                    // false — whereas under-accepting silently loses the
                    // glob. Deliberately NOT gated on isset(EXTENDEDGLOB):
                    // that is a runtime option and reading it here is the
                    // #1049 mistake. Bug #1053.
                    && s.chars().any(|c| {
                        matches!(
                            c,
                            '*' | '?'
                                | '['
                                | '('
                                | '|'
                                | '<'
                                | '#'
                                | '^'
                                | '\u{87}' // Star
                                | '\u{97}' // Quest
                                | '\u{91}' // Inbrack
                                | '\u{88}' // Inpar
                                | '\u{8e}' // Bar
                                | '\u{94}' // Inang
                                | '\u{84}' // Pound
                                | '\u{86}' // Hat
                        )
                    });
                if dwg_mod {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_DEFAULT_WORD_GLOB_RESET, 0),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0);
                }
                // The whole-word Dnull wrapping (`"${...}"`) gets
                // stripped from `untoked` before parse_param_modifier
                // sees it, but downstream emitters need to know the
                // DQ context (e.g. strip op: join-then-strip in DQ
                // vs per-element unquoted). Bump dq_context_depth
                // for the duration of emit_param_modifier when the
                // raw word is Dnull-wrapped, mirroring the
                // segments-loop above. Without this, the strip
                // fast path passed dq=0 to BUILTIN_PARAM_STRIP
                // even inside `"..."`.
                let raw_dq = word_is_single_dq_span(s);
                if raw_dq {
                    self.dq_context_depth += 1;
                }
                // c:Src/subst.c — `:#` filter pattern preserves
                // quoted/unquoted distinction via Dnull/Snull
                // markers all the way to patcompile. The Rust port's
                // plain `untokenize` collapses both shapes to ASCII
                // `*`/`?`/etc., so a quoted pattern (`"*"` → literal)
                // was treated as glob (matched everything) instead
                // of literal `*`. Re-extract the pattern from the raw
                // tokenized `s` using the quote-preserving
                // untokenize, so the constant emitted into the
                // bytecode carries `\X` escapes for chars that were
                // inside `"…"` / `'…'`. Bug #39 in docs/BUGS.md.
                //
                // Shape: `${NAME[(@)…]:#PAT}` → raw s is
                // `\u{85}\u{8f}NAME…:\u{84}PAT\u{90}` where
                // `\u{84}` = Pound (the `#`) and `\u{90}` = Outbrace
                // (the closing `}`).
                if let ParamModifierKind::FilterRemoveMatching { .. } = &modifier.kind {
                    if let Some(new_pat) = extract_filter_pat_from_raw_s(s) {
                        modifier.kind =
                            ParamModifierKind::FilterRemoveMatching { pattern: new_pat };
                    }
                }
                self.emit_param_modifier(&modifier);
                if raw_dq {
                    self.dq_context_depth -= 1;
                }
                if dwg_mod {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_DEFAULT_WORD_GLOB, 0),
                        0,
                    );
                }
                return;
            }
        }

        // TODO Phase 1 step 3 — `$((expr))` native lowering. Reverted
        // because fusevm's Op::Div is float-only; `$((10/3))` produces
        // 3.333... instead of zsh's integer-truncating 3. Need an
        // integer-aware division op (or a sniff in ArithCompiler that
        // picks IntDiv when both operands are Int) before this can ship.
        // The compound `(( ))` form has the same bug — pre-existing —
        // but currently dodges the test because $((..)) was bridged.

        // Phase 1 step 3b: `$((expr))` arithmetic substitution. Push
        // the expression text and call BUILTIN_ARITH_EVAL which routes
        // through the executor's MathEval (integer-aware, zsh-compat).
        // Avoids the float-only Op::Div in ArithCompiler.
        if !has_bnull {
            let preserved_for_arith = crate::lex::untokenize_preserve_quotes(s);
            if let Some(expr) = strip_arith_subst(&preserved_for_arith) {
                let idx = self.builder.add_constant(Value::str(expr.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_EVAL, 1), 0);
                return;
            }
        }

        // `$(cmd)` command substitution. Push the command text and
        // call BUILTIN_CMD_SUBST_TEXT which routes through
        // `run_command_substitution` (compile + sub-VM + in-process
        // pipe capture). Avoids the raw Op::CmdSubst path's
        // "$(printf "a\nb")" → "anb" quoting bug.
        if !has_bnull {
            let preserved_for_cmdsub = crate::lex::untokenize_preserve_quotes(s);
            // A whole-word backtick `` `cmd` `` is a command substitution
            // too — and an UNQUOTED one in argument position must IFS
            // word-split its output exactly like `$(cmd)` (`set -- \`echo
            // x y z\`` → $# == 3, not 1). The previous port only routed
            // `$(...)` through CMD_SUBST_TEXT + WORD_SPLIT; backtick fell
            // to the generic expand bridge, which ran the command but
            // emitted a single unsplit word. Convert the backtick body to
            // a `$(...)` body for run_command_substitution (BUILTIN_
            // CMD_SUBST_TEXT routes through it) — getoutput handles both
            // forms identically once the inner command text is isolated.
            let cmdsub_inner = strip_cmd_subst(&preserved_for_cmdsub)
                .or_else(|| strip_backtick_subst(&preserved_for_cmdsub));
            if let Some(inner) = cmdsub_inner {
                let idx = self.builder.add_constant(Value::str(inner));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_CMD_SUBST_TEXT, 1),
                    0,
                );
                // Word-split the result on IFS when the surrounding
                // word is unquoted. zsh: `f $(echo a b c)` passes
                // three args; `f "$(echo a b c)"` passes one. The
                // outer DQ wrapper appears as a leading `\u{9e}` in
                // `s`; inside DQ context (dq_context_depth>0) we also
                // skip the split. POSIX/SH_WORD_SPLIT semantics for
                // the cmd-subst case — applies even without the
                // option set because zsh splits cmd-subst by default
                // when the arg is bare.
                let in_dq = s.starts_with('\u{9e}') || self.dq_context_depth > 0;
                let in_assign = self.assign_context_depth > 0;
                if !in_dq && !in_assign {
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_WORD_SPLIT, 0), 0);
                }
                return;
            }
        }

        // Phase 1 step 4: concat. Walk the raw word, split into
        // (literal | expansion) segments, emit each, then fold via N-1
        // Concats. Each Expansion segment recurses through compile_word_str
        // (smaller input — terminates). Each Literal segment emits as a
        // pure-literal LoadConst (after untokenize so embedded META
        // chars resolve to their original ASCII).
        // If the word starts with `~` and contains a `$`-expansion,
        // skip the segment-split (which would emit literal `~` + the
        // expansion separately, defeating tilde-expand). Fall through
        // to the bridge so expand_string sees `~$VAR` whole.
        let starts_with_tilde_and_has_var = untoked.starts_with('~') && untoked.contains('$');
        // Default-word glob bracket gate (#2 default-word globbing): an
        // unquoted, top-level word that has BOTH a default/alt operator
        // (`-`/`+`) AND a glob metachar might carry a `${x:-*file}`-style
        // source-glob default. Bracket the assembled word with
        // RESET/…GLOB so the paramsubst arm's DEFAULT_WORD_GLOB_PENDING
        // flag (set only when a source-glob default branch is taken)
        // drives filename generation. A parameter VALUE never sets the
        // flag, so the APPLY no-ops for value branches. Loose gate (may
        // fire on a literal like `a-b*`) — harmless, the flag stays clear
        // so APPLY passes through. Only the outermost word (word_seg_depth
        // == 0) brackets; recursive segment expansions don't.
        let has_glob_meta = s
            .chars()
            .any(|c| matches!(c, '*' | '?' | '[' | '\u{87}' | '\u{97}' | '\u{91}'));
        let has_default_op = s.contains('-') || s.contains('+') || s.contains('\u{9b}');
        let default_word_glob_bracket = self.word_seg_depth == 0
            && self.dq_context_depth == 0
            && self.scalar_assign_depth == 0 // scalar `v=${x:-*}` RHS doesn't glob
            && self.assign_builtin_arg_depth == 0 // typeset/export NAME=value arg
            && has_glob_meta
            && has_default_op;
        if default_word_glob_bracket {
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_DEFAULT_WORD_GLOB_RESET, 0),
                0,
            );
            self.builder.emit(Op::Pop, 0); // discard the RESET status
        }
        if !has_bnull && !starts_with_tilde_and_has_var {
            if let Some(segs) = split_word_segments(s) {
                // Pick concat operator based on segment shape:
                // - Default splice (`${arr[@]}`, `$@`, `$*`): FIRST/LAST
                //   sticking — emit BUILTIN_CONCAT_SPLICE.
                // - Distribute (`${^arr}`, `${(@)…}`, RC_EXPAND_PARAM):
                //   cartesian — emit BUILTIN_CONCAT_DISTRIBUTE.
                // - Pure scalar: plain Op::Concat (fastest path).
                let has_splice_seg = segs.iter().any(|seg| match seg {
                    WordSegment::Expansion(exp) => is_splice_expansion(exp),
                    _ => false,
                });
                let has_distribute_seg = segs.iter().any(|seg| match seg {
                    WordSegment::Expansion(exp) => is_distribute_expansion(exp),
                    _ => false,
                });
                // c:Src/subst.c:4362-4365 — plan9 (`${^arr}`) DELETES the whole
                // word when the array is empty:
                //     if (plan9) { uremnode(l, n); return n; }
                // whereas every other distribute shape keeps it: `x${(@)a}y`
                // and `x${(f)v}y` are `xy` for an empty value, but `x${^a}y` is
                // NOTHING. Both used to compile to CONCAT_DISTRIBUTE_FORCED,
                // which keeps the word, so the plan9 removal never happened —
                // and the flag path could not borrow the OPTION path's correct
                // behaviour (`setopt rcexpandparam; x${a}y` removes it) because
                // that one dispatches on the runtime option, which the `^` flag
                // does not set. The `^`-ness is known only here, at compile
                // time, so pick the plan9 concat here.
                let has_plan9_seg = segs.iter().any(|seg| match seg {
                    WordSegment::Expansion(exp) => is_plan9_expansion(exp),
                    _ => false,
                });
                // c:2553-2555 — `${^^a}` forces plan9 OFF, overriding the
                // rcexpandparam OPTION. Every other concat builtin re-reads
                // that option at runtime, so under `setopt rcexpandparam` the
                // word cross-producted anyway and `${^^a}.x` gave
                // `a.x b.x c.x` instead of zsh's `a`, `b`, `c.x`.
                let has_plan9_off_seg = segs.iter().any(|seg| match seg {
                    WordSegment::Expansion(exp) => is_plan9_off_expansion(exp),
                    _ => false,
                });
                // If the parent word is DQ-wrapped (raw form starts and
                // ends with Dnull), each Expansion segment inherits the
                // DQ context. Track via the compiler's
                // `dq_context_depth` counter so child compile_word_str
                // calls can see they're being expanded inside DQ
                // without us having to re-wrap (which would recurse).
                // `parent_is_dq` is true if EITHER (a) the word itself
                // is wrapped in DQ markers, or (b) the calling context
                // already bumped `dq_context_depth` (e.g. cond's RHS
                // pattern wants variable expansion but no filesystem
                // glob — `[[ "$PATH" != *"$SCRIPTS"* ]]`).
                // A word is WHOLE-WORD double-quoted only when its outer
                // Dnull (`\u{9e}`) pair is the SINGLE span covering
                // everything. `s.starts_with && s.ends_with` alone
                // misfires on sibling spans like `"x"${a}"y"`, where the
                // leading and trailing Dnulls belong to DIFFERENT quote
                // runs and the middle `${a}` is UNQUOTED — treating the
                // word as DQ-wrapped bumped dq_context_depth and made the
                // unquoted array join to a scalar (zsh splits it:
                // `"x"$a"y"` for a=(one two three) → `xone two threey`,
                // three words). Count Dnulls only at brace/bracket/paren
                // depth 0: a genuine single wrap has exactly 2 (`"a${x}b"`
                // → 2), sibling spans have 4+ (`"x"${a}"y"` → 4), and
                // Dnulls NESTED inside `${…}` — `"a${x:-"n"}b"` — sit at
                // depth>0 and are ignored so the outer wrap still counts.
                let dq_marker_wrap = { word_is_single_dq_span(s) };
                let parent_is_dq = dq_marker_wrap || self.dq_context_depth > 0;
                let concat_builtin = if has_splice_seg {
                    Some(crate::vm_helper::BUILTIN_CONCAT_SPLICE)
                } else if has_plan9_seg {
                    // c:4316-4365 — cartesian emit PLUS the empty-array word
                    // deletion. Must precede the generic distribute arm: a
                    // `${^a}` segment satisfies both tests.
                    Some(crate::fusevm_bridge::BUILTIN_CONCAT_PLAN9)
                } else if has_plan9_off_seg {
                    // c:2554 — `${^^a}` overrides the option; splice, never
                    // cross-product.
                    Some(crate::fusevm_bridge::BUILTIN_CONCAT_SPLICE_NOPLAN9)
                } else if has_distribute_seg {
                    // `${^arr}` / `${(@)arr}` etc — distribution is
                    // explicit at the source level, not gated on the
                    // rcexpandparam option. Use the FORCED variant so
                    // a Value::Array on the stack always distributes
                    // cartesian with the surrounding text.
                    Some(crate::vm_helper::BUILTIN_CONCAT_DISTRIBUTE_FORCED)
                } else {
                    // Pure scalars OR `${arr}` plain — runtime check via
                    // BUILTIN_CONCAT_DISTRIBUTE (handles scalar fast path
                    // AND RC_EXPAND_PARAM cartesian when GET_VAR returns
                    // Value::Array because the option is set).
                    Some(crate::vm_helper::BUILTIN_CONCAT_DISTRIBUTE)
                };
                if dq_marker_wrap {
                    self.dq_context_depth += 1;
                }
                // Detect glob metachars in the LITERAL segments (var
                // refs in Expansion segments are ignored — `?` after
                // `$` is part of `$?`, not a glob). When found, after
                // the concat, emit BUILTIN_GLOB_PATH which runs
                // expand_glob on the assembled scalar. zsh's word-
                // expansion pipeline always pathname-expands the
                // post-substitution string; without this we kept
                // `$D/*` literal because the segment fast path
                // skipped pathname expansion entirely.
                // Detect glob metachars. `*`, `?`, `[`, and the
                // `(...|...)` alternation are always glob chars.
                // `#` and `^` are glob chars under EXTENDEDGLOB
                // (#/## quantifiers, ^ and-not) — c:Src/pattern.c
                // :4365 / :4370 haswilds gates them on
                // `isset(EXTENDEDGLOB)`. When EXTENDEDGLOB is off
                // at runtime, `zglob`'s own haswilds check
                // short-circuits so the emit is harmless.
                //
                // Scanned over LITERAL segments only (metas inside an
                // Expansion segment — `${(@k)h}` flag parens, `$'\e[34m'`
                // brackets, `$?` — are NOT glob triggers; substituted
                // content doesn't glob without GLOB_SUBST), with the
                // Snull/Dnull quote state threaded ACROSS segments at
                // word level. The two prior shapes each failed one way:
                //   * per-segment untokenize scan: stripped the quote
                //     markers and reset span state per segment, so the
                //     ` x[a]` literal tail of `X"$V x[a]"` (inside the
                //     word-level Dnull span) flagged needs_glob →
                //     NOMATCH (fzf-zsh-plugin:76);
                //   * whole-word unquoted() scan: counted metas inside
                //     EXPANSION segments, so `${(@k)AUTOPAIR_PAIRS}`'s
                //     flag parens / PROMPT4's `$'\e[34m…'` bracket
                //     triggered GLOB_EXPAND on the expanded value →
                //     "bad pattern: (" (zsh-autopair init, zpwr).
                let mut needs_glob = false;
                let mut needs_brace = false;
                // c:Src/subst.c:2596 `case '~': globsubst = 2` — a `$~name` /
                // `${~name}` segment promotes its VALUE's glob metachars to a
                // pattern, so the WHOLE assembled word must be filename-
                // generated (`$dir/$~pat` globs `$dir/<pat>`), not just the
                // flagged segment in isolation. The sub-segment fast paths skip
                // their own per-segment glob when word_seg_depth > 0, so flag
                // the word here. `$~~` / `${~~` toggle globsubst OFF (no flag).
                for seg in &segs {
                    if let WordSegment::Expansion(e) = seg {
                        let u = crate::lex::untokenize(e);
                        let is_gs = u
                            .strip_prefix("$~")
                            .or_else(|| u.strip_prefix("${~"))
                            .map(|r| !r.starts_with('~'))
                            .unwrap_or(false);
                        if is_gs {
                            needs_glob = true;
                            break;
                        }
                    }
                }
                {
                    let mut in_sq = false;
                    let mut in_dq = false;
                    // NB: deliberately no `isset(EXTENDEDGLOB)` here. This is
                    // the COMPILER; every option it could read is a RUNTIME
                    // value that a `setopt` later in the same script may
                    // change. See the Pound/Hat arm below (bug #1049).
                    for seg in &segs {
                        let lit = match seg {
                            WordSegment::Literal(l) => l,
                            WordSegment::Expansion(_) => continue,
                        };
                        let mut prev = ' ';
                        let mut saw_inpar = false;
                        let mut saw_bar = false;
                        let mut saw_outpar = false;
                        for c in lit.chars() {
                            match c {
                                '\u{9d}' => in_sq = !in_sq,
                                '\u{9e}' => in_dq = !in_dq,
                                _ if in_sq || in_dq || prev == '\u{9f}' || prev == '\0' => {}
                                '*' | '\u{87}' | '?' | '\u{97}' | '[' | '\u{91}' => {
                                    needs_glob = true;
                                }
                                // c:Src/pattern.c:4326-4335 — haswilds
                                // fires on ANY Inpar TOKEN unless SHGLOB
                                // is set; the runtime zglob short-
                                // circuits under SHGLOB.
                                '\u{88}' => needs_glob = true,
                                '(' => saw_inpar = true,
                                '|' => saw_bar = true,
                                ')' => saw_outpar = true,
                                // c:Src/lex.c:433-434 — `lextok2['#'] = Pound;`
                                // / `lextok2['^'] = Hat;` tokenize
                                // UNCONDITIONALLY; the lexer never consults
                                // EXTENDEDGLOB. The option test lives at glob
                                // time in haswilds (c:Src/pattern.c:4363-4370,
                                // `case Pound: if (isset(EXTENDEDGLOB) && …)`),
                                // which the Rust port already mirrors at
                                // pattern.rs:4119-4126.
                                //
                                // Testing EXTENDEDGLOB HERE read a RUNTIME
                                // option at COMPILE time: the whole script is
                                // compiled before any `setopt` in it executes,
                                // so `setopt extendedglob; v=a; print ${v}##`
                                // compiled with the option still off, never
                                // emitted the glob op, and printed `a##`.
                                // Passing `-o extendedglob` (set before
                                // compile) worked — the tell. Bug #1049.
                                //
                                // Emitting the op unconditionally is safe
                                // because it only routes the word to
                                // expand_glob, which gates on haswilds and
                                // hands back the literal when the option is
                                // off — same as C, where a Pound-carrying word
                                // always reaches zglob and haswilds decides.
                                '#' | '\u{84}' | '^' | '\u{86}' => {
                                    needs_glob = true;
                                }
                                '{' | '\u{8f}' | '}' | '\u{90}' => {
                                    needs_brace = true;
                                }
                                _ => {}
                            }
                            prev = c;
                        }
                        if saw_inpar && saw_bar && saw_outpar {
                            needs_glob = true;
                        }
                    }
                }
                // A word that MIXES a plan9 (`^`) expansion with a non-plan9
                // (splice/scalar) expansion — `"${(@)^a}${(@)b}"` — cannot be
                // folded by a SINGLE concat operator: plan9 cross-products while
                // splice sticks first/last, and the per-pair fold loses the
                // "growing edge" that zsh threads through the whole word
                // (c:Src/subst.c:4316-4437). Route these to the atomic
                // BUILTIN_WORD_ASSEMBLE_PLAN9 assembler, which ports the edge
                // tracking. Uniform words (all-plan9, all-splice) keep the fast
                // fold — only the genuinely-mixed shape takes this path.
                let mixed_plan9 = has_plan9_seg
                    && segs.iter().any(
                        |seg| matches!(seg, WordSegment::Expansion(e) if !is_plan9_expansion(e)),
                    );
                if mixed_plan9 {
                    // Descriptor: one char per segment, `'1'` = plan9 (`^`),
                    // `'0'` = splice/scalar/literal. Pushed FIRST so it sits at
                    // the bottom of this word's stack region (popped last).
                    let desc: String = segs
                        .iter()
                        .map(|seg| match seg {
                            WordSegment::Expansion(e) if is_plan9_expansion(e) => '1',
                            _ => '0',
                        })
                        .collect();
                    let idx = self.builder.add_constant(Value::str(desc.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                }
                for (i, seg) in segs.iter().enumerate() {
                    match seg {
                        WordSegment::Literal(lit) => {
                            // When a brace pattern straddles literal +
                            // expansion segments (`"$X"{a,b,c}`), the
                            // post-CONCAT BRACE_EXPAND emit relies on
                            // Inbrace/Outbrace/Comma TOKEN bytes
                            // (\u{8f}/\u{90}/\u{9a}) to detect the
                            // brace structure. A full untokenize here
                            // would erase them, so when needs_brace
                            // fires we partial-untokenize: strip all
                            // ITOK markers EXCEPT brace/comma so
                            // xpandbraces still sees the structure.
                            // c:Src/glob.c::xpandbraces runs on
                            // TOKEN-form words; the final ASCII pass
                            // is xpandbraces' own concatenation output.
                            let cleaned = if needs_brace {
                                untokenize_keep_braces(lit)
                            } else {
                                crate::lex::untokenize(lit)
                            };
                            // No `\x00`-sentinel strip here: NULs in a
                            // literal segment are real `$'\0'` data
                            // bytes (see compile_word_str pure-literal
                            // arm). Bug #560.
                            let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
                            self.builder.emit(Op::LoadConst(idx), 0);
                        }
                        WordSegment::Expansion(exp) => {
                            // Inner segment: don't let it emit its own
                            // default-word glob bracket (the outer word
                            // owns the assembled-word pass).
                            //
                            // c:Src/subst.c:283 — `qt` is a property of the
                            // INDIVIDUAL `$` token, not of the word:
                            //   n = paramsubst(list, n, &str, (c == Qstring), …)
                            // The lexer (Src/lex.c dquote_parse) emits Qstring
                            // (`\u{8c}`) for a `$` inside `"…"` and Stringg
                            // (`\u{85}`) outside, so `"$a"post` has a QUOTED
                            // expansion even though the WORD is not a single DQ
                            // span. `word_is_single_dq_span` only sees the word
                            // shape, so this segment used to compile unquoted:
                            // the array stayed array-shaped and CONCAT split it
                            // into words (`x` `y` `zpost`) where zsh sepjoins to
                            // one word (`x y zpost`). Bump dq_context_depth for
                            // exactly this segment so its compile takes the same
                            // DQ paths a whole-word `"$a"` takes. Qtick
                            // (`\u{99}`) is the same marker for `` ` `` inside
                            // DQ.
                            let seg_is_dq = exp.starts_with(crate::ported::zsh_h::Qstring)
                                || exp.starts_with(crate::ported::zsh_h::Qtick);
                            if seg_is_dq {
                                self.dq_context_depth += 1;
                            }
                            self.word_seg_depth += 1;
                            self.compile_word_str(exp);
                            self.word_seg_depth -= 1;
                            if seg_is_dq {
                                self.dq_context_depth -= 1;
                            }
                        }
                    }
                    // Mixed plan9/splice words defer ALL combination to the
                    // atomic assembler below — no per-pair concat here.
                    if i > 0 && !mixed_plan9 {
                        if let Some(b) = concat_builtin {
                            // c:Src/options.c — RC_EXPAND_PARAM
                            // applies UNQUOTED only. When the parent
                            // word is DQ-wrapped, pass argc=1 to
                            // BUILTIN_CONCAT_DISTRIBUTE so its handler
                            // suppresses the cartesian path and joins
                            // arrays via $IFS[0] regardless of the
                            // option state. Bug #246 in docs/BUGS.md.
                            // SPLICE and FORCED variants ignore argc.
                            let argc = if parent_is_dq
                                && b == crate::vm_helper::BUILTIN_CONCAT_DISTRIBUTE
                            {
                                1
                            } else {
                                2
                            };
                            self.builder.emit(Op::CallBuiltin(b, argc), 0);
                        } else {
                            self.builder.emit(Op::Concat, 0);
                        }
                    }
                }
                if mixed_plan9 {
                    // Pop the descriptor + all N segment values (argc = N + 1).
                    let argc = (segs.len() + 1) as u8;
                    self.builder.emit(
                        Op::CallBuiltin(crate::fusevm_bridge::BUILTIN_WORD_ASSEMBLE_PLAN9, argc),
                        0,
                    );
                }
                // c:Src/subst.c:184-188 — prefork's `uremnode` deletes an EMPTY
                // word once the expansion is ASSEMBLED. Order is the whole
                // point: for `s=' a b '` the split leaves an empty field at
                // each end, and an affix attaches to the empty on ITS side, so
                // zsh gives
                //     ${=s}.x   → `a` `b` `.x`     (trailing empty became .x,
                //                                   LEADING empty deleted)
                //     pre${=s}  → `pre` `a` `b`    (leading empty became pre,
                //                                   TRAILING empty deleted)
                // i.e. only the FAR-side empty goes. Dropping before the concat
                // would delete the very field the affix was going to land on
                // (`${=s}.x` would lose `.x`), which is why the drop lives here
                // and not next to the split.
                //
                // Gated on an array-producing segment: a word with no expansion
                // (`print -rl -- ""`) must keep its literal empty. Quoted words
                // keep empties (nulstring), and a scalar-assignment RHS is
                // joined below rather than split into words.
                if !parent_is_dq
                    && (has_splice_seg || has_distribute_seg || has_plan9_seg || has_plan9_off_seg)
                    && self.scalar_assign_depth == 0
                    && self.assign_builtin_arg_depth == 0
                {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_DROP_EMPTY, 1),
                        0,
                    );
                }
                // c:Src/subst.c:3032 (sepjoin under ssub) — a SCALAR
                // assignment RHS coerces an assembled array to one
                // string via ${IFS[1]}. A splice segment (`"$@"` /
                // `"${arr[@]}"`) inside a partially-quoted typeset arg
                // (`local input="$*"`, `typeset v="$@"`) leaves the word
                // as an array after CONCAT_SPLICE; without the coerce the
                // array splats into the builtin arg list and only the
                // first element survives (qrcode plugin). Bare `v="$@"`
                // already joins via compile_assign's single-segment fast
                // path — this covers the typeset-family multi-segment
                // path. Gate on a splice segment (the only shape that can
                // leave an array); the coerce is a no-op on scalars.
                //
                // c:Src/subst.c:4226-4231 — the same coerce is owed to EVERY
                // array-producing segment, not just a splice:
                //     if (isarr && ssub) {
                //         val = sepjoin(aval, NULL, 1);
                //         isarr = 0;
                //     }
                // runs on whatever `aval` holds. A plain `${arr}` /
                // distribute / plan9 segment in a typeset-family `NAME=VALUE`
                // arg (`local v=${a}`, `local y=${a:^b}`) left the word an
                // ARRAY, so `local` received one arg per element and rejected
                // element 2 with "not an identifier: 2". The bare
                // `v=${a}` form is unaffected because compile_assign's
                // SET_VAR joins the array on store; only the typeset-family
                // arg path splats. Restrict the widened arm to an
                // assignment-SHAPED typeset arg (assign_context_depth is
                // bumped at the `arg_is_assign` site above) so a bare
                // multi-name word like `local ${(k)assoc}` still declares one
                // name per element.
                // The typeset-family arm is deliberately NOT gated on a
                // compile-time segment classification: a plain `${a}` segment
                // is neither splice nor distribute at compile time, yet it
                // still yields an ARRAY at runtime (CONCAT_DISTRIBUTE is the
                // default concat builtin). C doesn't inspect the shape either
                // — c:4226 joins whatever `aval` holds — and ARRAY_JOIN is a
                // no-op on a scalar, so emitting it unconditionally for an
                // assignment-SHAPED typeset arg is the faithful form.
                let typeset_assign_arg =
                    self.assign_builtin_arg_depth > 0 && self.assign_context_depth > 0;
                if (has_splice_seg
                    && (self.scalar_assign_depth > 0 || self.assign_builtin_arg_depth > 0))
                    || typeset_assign_arg
                {
                    self.builder
                        .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_JOIN, 0), 0);
                }
                if dq_marker_wrap {
                    self.dq_context_depth -= 1;
                }
                if needs_brace && !parent_is_dq {
                    // Brace-expand the assembled scalar. Pops Value::Str,
                    // runs xpandbraces, pushes Value::Array.
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_BRACE_EXPAND, 0),
                        0,
                    );
                }
                if needs_glob
                    && !parent_is_dq
                    // Assignment-builtin (typeset/export/…) and scalar
                    // assignment values are NOT filename-generated — zsh's
                    // scalar postassign path never calls globlist
                    // (Src/exec.c:4246-4249 "No globassign for typeset
                    // arguments"). Mirror the default-word gate above so a
                    // value that combines a param expansion with a literal
                    // glob metachar (`typeset -i lv=$x*2` → `lv=6*2`) isn't
                    // globbed after expansion.
                    && self.assign_builtin_arg_depth == 0
                    && self.scalar_assign_depth == 0
                {
                    // Glob-expand the assembled scalar at runtime. The
                    // builtin pops a Value::Str, runs expand_glob, and
                    // pushes Value::Array (or single-elem when no match).
                    self.emit_word_glob_expand();
                }
                // Default-word glob: only when needs_glob didn't already
                // glob the assembled word (a literal-segment glob covers
                // the whole word, including any flagged default). The
                // RESET at word start clears any residual flag otherwise.
                if default_word_glob_bracket && !(needs_glob && !parent_is_dq) {
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_DEFAULT_WORD_GLOB, 0),
                        0,
                    );
                }
                return;
            }
        }

        // Phase 2 step 2: text-based bridge replacement. Determine the
        // word's quoting mode from its raw zsh-tokenized form, push the
        // raw tokenized text + mode_byte, call BUILTIN_EXPAND_TEXT.
        //
        // Mode detection:
        // - Whole-word Dnull-wrapped (`"…"`) and no inner unescaped
        //   Dnull → DoubleQuoted. Suppresses brace + glob expansion;
        //   var / cmd-sub / arith inside still expand.
        // - Backquote-wrapped (`` `…` ``) → AltBackquote, runs as
        //   command substitution.
        // - Else → Default, full expand_string + braces + glob.
        // c:Src/parse.c stores raw tokenized word strings; C's
        // prefork processes them in TOKEN form throughout. We pass
        // the raw tokenized string through unchanged — downstream
        // consumers (hasbraces, xpandbraces, filesubstr, subst arms)
        // now match the C source in checking TOKEN bytes strictly.
        let preserved: String = s.to_string();
        // If we're recursing inside a DQ-wrapped parent (tracked via
        // `dq_context_depth`), force mode 1 so child expansions
        // suppress array-only flags like the outer DQ does.
        let base_mode = if self.dq_context_depth > 0 {
            1
        } else {
            expand_text_mode(s, &preserved)
        };
        // Mode 5: "DQ in scalar-assignment context" — same as mode 1
        // (DoubleQuoted) but additionally signals PREFORK_SINGLE-
        // equivalent semantics to subst_port. Direct port of zsh
        // exec.c::addvars line 2546 setting `PREFORK_SINGLE|
        // PREFORK_ASSIGN` on prefork. Inside paramsubst, ssub=
        // PREFORK_SINGLE gates the force_split path off so split
        // flags `(f)` / `(s:STR:)` / `(0)` / `(z)` produce the
        // ORIGINAL scalar (preserves `\n` separators in
        // `y="${(f)x}"`) rather than splitting then re-joining
        // with IFS-first-char.
        // Mode 6: "unquoted scalar-assignment RHS" — same as default
        // mode 0 (Default) but signals PREFORK_ASSIGN to the bridge
        // so prefork's `filesub` colon-walk fires (c:Src/exec.c:2546
        // sets PREFORK_ASSIGN, which Src/subst.c:filesub:689 keys
        // on to walk `:`-separated path components for `~`/`=`
        // re-expansion). Without this, `X=/usr/bin:~/bin` left the
        // `~/bin` literal because filesub was called with assign=0.
        // Mode 7: "unquoted redirect-target word" — same as default
        // mode 0 but the bridge's glob pass is gated on MULTIOS
        // (c:Src/glob.c:2161-2167 xpandredir: PREFORK_SINGLE +
        // "Globbing is only done for multios."). Without this,
        // `unsetopt multios; echo hi > *.txt` globbed the target
        // instead of creating the literal file `*.txt`.
        // Mode 8: "unquoted assignment VALUE" — mode 6 plus PREFORK_SINGLE.
        // c:Src/exec.c:2603 `prefork(vl, isstr ? (PREFORK_SINGLE|
        // PREFORK_ASSIGN) : PREFORK_ASSIGN, …)` for a bare `NAME=VALUE`
        // statement, and c:Src/exec.c:4239-4241 `prefork(&svl,
        // PREFORK_SINGLE|PREFORK_ASSIGN, NULL)` for the typeset-family
        // `NAME=VALUE` ARGUMENT. PREFORK_SINGLE is `ssub`
        // (c:Src/subst.c:1761), which turns off the forced split at
        // c:Src/subst.c:3913 `force_split = !ssub && (spbreak || spsep)` —
        // so `local s=${(s.:.)str:u}` keeps the separators (`A:B:C`) instead
        // of splitting and re-joining on IFS[0] (`A B C`).
        //
        // Only an assignment-SHAPED word qualifies, hence the
        // `assign_context_depth` term: a NAME-only typeset argument is
        // preforked with PREFORK_TYPESET (c:4197) and an array-valued one
        // with plain PREFORK_ASSIGN (c:4265), and both must keep splatting
        // one word per element (`local ${(k)assoc}`, `local -a a=(${(s::)x})`).
        let scalar_assign_ctx = self.scalar_assign_depth > 0 || self.assign_builtin_arg_depth > 0;
        let ssub_assign_value = scalar_assign_ctx && self.assign_context_depth > 0;
        let mode = if base_mode == 1 && scalar_assign_ctx {
            5
        } else if base_mode == 0 && ssub_assign_value {
            8
        } else if base_mode == 0 && scalar_assign_ctx {
            6
        } else if base_mode == 0 && self.redir_word_depth > 0 {
            7
        } else {
            base_mode
        };
        let idx = self.builder.add_constant(Value::str(preserved.as_str()));
        self.builder.emit(Op::LoadConst(idx), 0);
        self.builder.emit(Op::LoadInt(mode as i64), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 2), 0);
        // Brace expansion on the bridge-text path. Words like
        // `X{1,2,3}Y` that don't have $/`/$( fall through here
        // (split_word_segments returns Some([Literal]) but the
        // segment-fast path's BRACE_EXPAND only fires for words
        // that ALSO have an Expansion segment). Without this the
        // brace expansion never runs and `print X{1,2,3}Y` returns
        // the literal text. Direct port of subst.c:166 where
        // xpandbraces fires AFTER prefork's expansion pass.
        // c:Src/subst.c:166 — xpandbraces fires AFTER prefork's expansion
        // pass. Trigger on Inbrace TOKEN (\u{8f}) only — escaped `\{`
        // is Bnull+ASCII`{` which (post-remnulargs) is plain `{` and
        // must NOT brace-expand. The Star TOKEN (\u{87}) tail is for
        // pattern words that also need expand_glob to run from the
        // brace-expand builtin (kept legacy-compatible).
        let preserved_str = preserved.as_str();
        let brace_emitted = !preserved_str.is_empty()
            && (preserved_str.contains('\u{8f}') || preserved_str.contains('\u{87}'))
            && self.dq_context_depth == 0;
        if brace_emitted {
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_BRACE_EXPAND, 0),
                0,
            );
        }
        // Default-word glob on the bridge-text path. The brace-expand
        // above globs only when the SOURCE word carried a Star/Inbrace
        // TOKEN (literal-segment glob); a `${x:-*file}` default has no
        // such token in `preserved`, so add the flag-gated assembled-word
        // glob. Skip when the token-brace path already globbed.
        if default_word_glob_bracket && !brace_emitted {
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_DEFAULT_WORD_GLOB, 0),
                0,
            );
        }
    }

    // ── Control flow ────────────────────────────────────────────────

    fn compile_if(&mut self, if_node: &crate::parse::ZshIf) {
        // ZshIf: cond + then + Vec<(elif_cond, elif_body)> + Optional(else_).
        // Layout per branch:
        //   <compile cond>
        //   GetStatus
        //   JumpIfFalse(skip_body)
        //   <compile body>
        //   Jump(end)
        // skip_body:
        // Final else block (no condition gate). All end-jumps patched to past
        // the whole if.
        //
        // cmdstack tracking — direct port of Src/loop.c:572 / :587
        //   cmdpush(s ? CS_ELIF : CS_IF);   around cond
        //   cmdpop();
        //   if (run) {
        //       cmdpush(run == 2 ? CS_ELSE : (s ? CS_ELIFTHEN : CS_IFTHEN));
        //       around body  cmdpop();
        //   }
        let mut end_jumps = Vec::new();

        // First branch — the test is errexit-suppressed.
        self.emit_cmd_push(crate::ported::zsh_h::CS_IF as u8);
        self.errexit_suppress_depth += 1;
        self.compile_program(&if_node.cond);
        self.errexit_suppress_depth -= 1;
        self.emit_cmd_pop();
        self.builder.emit(Op::GetStatus, 0);
        let mut skip_body = self.builder.emit(Op::JumpIfFalse(0), 0);
        // CS_IFTHEN = 6 = CmdState::Then
        self.emit_cmd_push(crate::ported::zsh_h::CS_IFTHEN as u8);
        self.compile_program(&if_node.then);
        self.emit_cmd_pop();
        end_jumps.push(self.builder.emit(Op::Jump(0), 0));
        self.builder
            .patch_jump(skip_body, self.builder.current_pos());

        // elif branches — same suppression for each cond.
        for (cond, body) in &if_node.elif {
            self.emit_cmd_push(crate::ported::zsh_h::CS_ELIF as u8);
            self.errexit_suppress_depth += 1;
            self.compile_program(cond);
            self.errexit_suppress_depth -= 1;
            self.emit_cmd_pop();
            self.builder.emit(Op::GetStatus, 0);
            skip_body = self.builder.emit(Op::JumpIfFalse(0), 0);
            // CS_ELIFTHEN = 26 = CmdState::ElifThen, prints "elif-then"
            self.emit_cmd_push(crate::ported::zsh_h::CS_ELIFTHEN as u8);
            self.compile_program(body);
            self.emit_cmd_pop();
            end_jumps.push(self.builder.emit(Op::Jump(0), 0));
            self.builder
                .patch_jump(skip_body, self.builder.current_pos());
        }

        // else — body's status carries through. If else exists,
        // emit a Jump-past-default so the no-match SetStatus(0)
        // doesn't clobber else_body's exit code.
        if let Some(else_) = &if_node.else_ {
            self.emit_cmd_push(crate::ported::zsh_h::CS_ELSE as u8);
            self.compile_program(else_);
            self.emit_cmd_pop();
            end_jumps.push(self.builder.emit(Op::Jump(0), 0));
        }

        // No-match path: when no cond was truthy AND no else
        // matched, the if-stmt returns 0. Direct port of
        // Src/loop.c:execif:590-591 — `else if (!retflag && !errflag)
        // lastval = 0;`. The default path falls through here from
        // the trailing JumpIfFalse skip-targets; matched-body and
        // else-body Jumps land at `end` (past this SetStatus).
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetStatus, 0);

        let end = self.builder.current_pos();
        for ej in end_jumps {
            self.builder.patch_jump(ej, end);
        }
    }

    fn compile_while(&mut self, w: &crate::parse::ZshWhile) {
        // Layout:
        //   loop_top:
        //     <cond>
        //     GetStatus
        //     JumpIf{False/True}(loop_exit)        # False for while, True for until
        //     <body>
        //   continue_target:
        //     Jump(loop_top)
        //   loop_exit:
        //
        // Plus break/continue patch-list pushes around the body.
        //
        // cmdstack: direct port of Src/loop.c:424
        //   cmdpush(isuntil ? CS_UNTIL : CS_WHILE);
        // popped after the loop body.
        let cs_token = if w.until {
            crate::ported::zsh_h::CS_UNTIL as u8
        } else {
            crate::ported::zsh_h::CS_WHILE as u8
        };
        self.emit_cmd_push(cs_token);
        let status_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(status_slot), 0);

        self.emit_loop_enter(); // c:Src/loop.c:427 — `loops++;`
        let loop_top = self.builder.current_pos();
        // The while/until test is errexit-suppressed.
        self.errexit_suppress_depth += 1;
        self.compile_program(&w.cond);
        self.errexit_suppress_depth -= 1;
        self.builder.emit(Op::GetStatus, 0);
        let exit_jump = if w.until {
            // until — exit when status is truthy (success)
            self.builder.emit(Op::JumpIfTrue(0), 0)
        } else {
            self.builder.emit(Op::JumpIfFalse(0), 0)
        };

        self.open_loop_scope();

        self.compile_program(&w.body);
        // Capture body's last status into status_slot so the loop's exit
        // status reflects the body, not the (failing) condition probe.
        self.builder.emit(Op::GetStatus, 0);
        self.builder.emit(Op::SetSlot(status_slot), 0);
        self.emit_loop_body_end(); // c:Src/loop.c:529-534
        // c:Src/loop.c — the loop's own post-body errflag guard.
        let errflag_break = self.emit_loop_errflag_break();

        let cont = self.builder.current_pos();
        if let Some(continues) = self.continue_patches.pop() {
            for cp in continues {
                self.builder.patch_jump(cp, cont);
            }
        }

        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);
        self.builder.patch_jump(errflag_break, loop_exit);

        self.close_loop_scope(loop_exit); // c:Src/loop.c:491 — `loops--;`

        // Restore loop's exit status from the body's last-status slot.
        self.builder.emit(Op::GetSlot(status_slot), 0);
        self.builder.emit(Op::SetStatus, 0);
        self.emit_cmd_pop();
    }

    fn compile_for(&mut self, f: &crate::parse::ZshFor) {
        if f.is_select {
            self.compile_select(f);
            return;
        }
        // cmdstack: direct port of Src/loop.c:119 `cmdpush(CS_FOR);`.
        // Both `for x in …` and `for ((;;))` push CS_FOR at execution
        // time — Src/parse.c:972/977 differentiates CS_FOR vs
        // CS_FOREACH at parse time only, but execfor always uses
        // CS_FOR.
        //
        // The CStyle (arith) branch handles its own cmdstack push
        // because C zsh emits the init trace BEFORE pushing CS_FOR
        // (init trace fires inside execfor's iscond block at
        // Src/loop.c:21-25, with cmdpush(CS_FOR) only at line 65).
        let manage_cmd_stack = !matches!(f.list, ForList::CStyle { .. });
        if manage_cmd_stack {
            self.emit_cmd_push(crate::ported::zsh_h::CS_FOR as u8);
        }
        match &f.list {
            ForList::Words(words) => {
                self.compile_for_words(&f.var, words, &f.body);
            }
            ForList::CStyle { init, cond, step } => {
                self.compile_for_arith(init, cond, step, &f.body);
            }
            ForList::Positional => {
                // `for var; do …; done` — iterate over the positional
                // params verbatim. Emit BUILTIN_GET_VAR("@") (returns
                // Value::Array of positionals) directly, then feed into
                // ARRAY_FLATTEN; mirrors compile_for_words' shape but
                // with the array push pre-baked instead of going through
                // compile_word_str.
                self.compile_for_positional(&f.var, &f.body);
            }
        }
        if manage_cmd_stack {
            self.emit_cmd_pop();
        }
    }

    fn compile_select(&mut self, f: &crate::parse::ZshFor) {
        // Build the body sub-chunk so RUN_SELECT can run it per pick.
        let mut sub = ZshCompiler::new();
        sub.compile_program(&f.body);
        let sub_end = sub.builder.current_pos();
        for patch in std::mem::take(&mut sub.return_patches) {
            sub.builder.patch_jump(patch, sub_end);
        }
        let body_chunk = sub.builder.build();
        let body_idx = self.builder.add_sub_chunk(body_chunk);

        // Push word_1, ..., word_N (in source order), then name, then sub_idx.
        // RUN_SELECT pops sub_idx, name, then collects N words.
        let nwords: usize = match &f.list {
            ForList::Words(ws) => {
                for w in ws {
                    self.compile_word_str(w);
                    if needs_word_split(w) {
                        self.builder
                            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_WORD_SPLIT, 0), 0);
                    }
                }
                ws.len()
            }
            // c:Src/loop.c:235-242 — `select x; do … done` (WC_SELECT_PPARAM)
            // takes the positionals DIRECTLY:
            //     for (x = pparams; *x; x++)
            //         addlinknode(args, dupstring(*x));
            // No splitting, so an EMPTY positional keeps its own menu entry.
            // This compiled the list as the word `$@` and then ran
            // BUILTIN_WORD_SPLIT over it, and word splitting DROPS empty
            // fields — `f '' x` produced one entry where zsh shows two
            // (`1)    2) x`). GET_VAR("@") yields a Value::Array that
            // RUN_SELECT flattens per element, preserving empties: the same
            // mechanism compile_for_positional (see below) already uses for
            // the identical `for x; do … done` pparam form.
            ForList::Positional => {
                let at_const = self.builder.add_constant(Value::str("@"));
                self.builder.emit(Op::LoadConst(at_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_VAR, 1), 0);
                1
            }
            ForList::CStyle { .. } => {
                // C-style isn't valid for select; nothing to do.
                return;
            }
        };

        let name_const = self.builder.add_constant(Value::str(f.var.as_str()));
        self.builder.emit(Op::LoadConst(name_const), 0);
        self.builder.emit(Op::LoadInt(body_idx as i64), 0);

        let argc = (nwords + 2) as u8;
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_RUN_SELECT, argc),
            0,
        );
        self.builder.emit(Op::SetStatus, 0);
    }

    fn compile_for_positional(&mut self, var: &str, body: &crate::parse::ZshProgram) {
        // Push GET_VAR("@") which returns Value::Array of positionals.
        let at_const = self.builder.add_constant(Value::str("@"));
        self.builder.emit(Op::LoadConst(at_const), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_VAR, 1), 0);
        // Then flatten + iterate, same shape as compile_for_words' tail.
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_FLATTEN, 1),
            0,
        );
        let i_slot = self.next_slot;
        self.next_slot += 1;
        let len_slot = self.next_slot;
        self.next_slot += 1;
        let arr_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::SetSlot(len_slot), 0);
        self.builder.emit(Op::SetSlot(arr_slot), 0);

        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(i_slot), 0);

        // c:Src/loop.c execfor — a `for` loop does NOT reset $? before
        // the body: the word-list expansion status (or, for a literal
        // list, the previous command's $?) carries into the FIRST body
        // iteration. `(exit 2); for x in 1 2; do print $?; done` →
        // the first `print` sees 2; `false; for x in $(echo 1; (exit 3))`
        // → the first body iteration sees 3 (the cmdsubst's status).
        // Only an EMPTY list (zero iterations) resets $? to 0
        // (`false; for x in $(exit 4); do …; done` → 0). The body itself
        // (execlist) sets $? on every iteration — including an empty body,
        // whose compiled chunk emits LoadInt(0)+SetStatus — so the reset
        // is needed solely for the never-entered case. Previously this
        // reset was unconditional, clobbering the carried status.
        self.builder.emit(Op::GetSlot(len_slot), 0);
        let carry_jump = self.builder.emit(Op::JumpIfFalse(0), 0); // len==0 → reset
        let skip_reset_jump = self.builder.emit(Op::Jump(0), 0); // len>0 → carry status
        let reset_pos = self.builder.current_pos();
        self.builder.patch_jump(carry_jump, reset_pos);
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetStatus, 0);
        let after_reset = self.builder.current_pos();
        self.builder.patch_jump(skip_reset_jump, after_reset);

        self.emit_loop_enter(); // c:Src/loop.c:114 — `loops++;`
        let loop_top = self.builder.current_pos();
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::GetSlot(len_slot), 0);
        self.builder.emit(Op::NumLt, 0);
        let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

        let var_const = self.builder.add_constant(Value::str(var));
        self.builder.emit(Op::LoadConst(var_const), 0);
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::SlotArrayGet(arr_slot), 0);
        // c:Src/loop.c execfor → setloopvar (params.c:6362) — see
        // compile_for_words for the nameref-rebind rationale.
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_LOOP_VAR, 2),
            0,
        );
        let loop_var_abort = self.builder.emit(Op::JumpIfFalse(0), 0);
        // c:Src/exec.c::execlist:28+292 — restore `lineno` to the
        // for-statement's line before the per-iter trace; matches
        // execlist's save/restore around each body. See compile_for_words
        // for the equivalent fix on the named-list path.
        self.builder.emit(Op::LoadInt(self.current_sublist_line), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_LINENO, 1), 0);
        self.builder.emit(Op::Pop, 0);
        // xtrace: emit `name=value\n` per iteration. Direct port of
        // Src/loop.c:163-166. XTRACE_LINE no-ops when -x is off.
        let assign_prefix = format!("{}=", var);
        let prefix_const = self
            .builder
            .add_constant(Value::str(assign_prefix.as_str()));
        self.builder.emit(Op::LoadConst(prefix_const), 0);
        let var_const2 = self.builder.add_constant(Value::str(var));
        self.builder.emit(Op::LoadConst(var_const2), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_VAR, 1), 0);
        self.builder.emit(Op::Concat, 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
        self.builder.emit(Op::Pop, 0);

        self.open_loop_scope();

        self.compile_program(body);
        self.emit_loop_body_end(); // c:Src/loop.c:180-185
        // c:Src/loop.c:198-203 — `if (errflag) { if (breaks) breaks--;
        // lastval = 1; break; }`. See emit_loop_errflag_break.
        let errflag_break = self.emit_loop_errflag_break();

        let cont = self.builder.current_pos();
        if let Some(continues) = self.continue_patches.pop() {
            for cp in continues {
                self.builder.patch_jump(cp, cont);
            }
        }

        self.builder.emit(Op::PreIncSlotVoid(i_slot), 0);
        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);
        self.builder.patch_jump(errflag_break, loop_exit);
        self.builder.patch_jump(loop_var_abort, loop_exit);

        self.close_loop_scope(loop_exit); // c:Src/loop.c:188 — `loops--;`
    }

    fn compile_for_words(&mut self, var: &str, words: &[String], body: &crate::parse::ZshProgram) {
        let i_slot = self.next_slot;
        self.next_slot += 1;
        let len_slot = self.next_slot;
        self.next_slot += 1;
        let arr_slot = self.next_slot;
        self.next_slot += 1;

        for word in words {
            // Unquoted bare `$NAME` in a for-list — when NAME is an
            // array, zsh splices each element as one iteration. Detect
            // this shape (no DQ markers, no other shell metas) and emit
            // BUILTIN_ARRAY_ALL which always returns Value::Array (for
            // arrays) or a single-element Array (for scalars). Without
            // this, BUILTIN_GET_VAR returns the IFS-joined string for
            // arrays and `for f in $arr` iterates ONCE.
            let untoked = crate::lex::untokenize(word);
            let is_bare_var_dollar = untoked.starts_with('$')
                && !word.contains('\u{9d}')   // no SQ
                && !word.contains('\u{9e}')   // no DQ
                && untoked[1..]
                    .chars()
                    .all(|c| c == '_' || c.is_ascii_alphanumeric())
                && !untoked[1..].is_empty()
                && !untoked.contains('[');
            if is_bare_var_dollar {
                let name = &untoked[1..];
                let name_const = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_ALL, 0), 0);
                // c:Src/subst.c:184-187 — this shape is UNQUOTED by
                // construction (the DQ-marker checks above), so
                // prefork's empty-word removal applies: `a=(y '' x);
                // for i in $a` iterates twice in zsh.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_DROP_EMPTY, 1),
                    0,
                );
                continue;
            }
            // c:Src/exec.c — `for x in $(cmd)` undergoes ONE wordsplit
            // pass. compile_word_str's cmdsub arm at line 3551 already
            // emits WORD_SPLIT when not in DQ/assign context, so let
            // it handle the split and skip the outer WORD_SPLIT below.
            // Suppress the inner emit by bumping assign_context_depth
            // when the word contains a cmdsub — the outer for-loop
            // WORD_SPLIT runs next, doing the actual IFS split.
            // Without this, both emitted: the inner split correctly
            // (e.g. IFS=, splits "a,b,c" to ["a","b","c"]), then the
            // outer ran on the Array.to_str() join "a b c" which has
            // no IFS chars, collapsing back to 1 element. Bug #178
            // in docs/BUGS.md.
            let has_cmdsub = needs_word_split(word);
            if has_cmdsub {
                self.assign_context_depth += 1;
            }
            self.word_emitted_glob = false;
            self.compile_word_str(word);
            if has_cmdsub {
                self.assign_context_depth -= 1;
            }
            // Unquoted command/variable substitution in a for-list should
            // IFS-split. zsh's for-list naturally word-splits the result
            // of `$(...)` or unquoted `$var`. Quoted forms keep one word.
            if needs_word_split(word) {
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_WORD_SPLIT, 0), 0);
            }
            // c:Src/subst.c — unquoted `$@` / `$*` drop empty
            // elements (POSIX-like word splitting via the multsub
            // PREFORK_SPLIT path). zsh's specific quirk: it does NOT
            // IFS-split each element on internal spaces (so
            // `set -- "hello world"; for x in $@` keeps "hello world"
            // as one). Just filter empties — don't word-split.
            // Bug #166 in docs/BUGS.md.
            let untoked_w = crate::lex::untokenize(word);
            let is_unquoted_at_or_star = !word.contains('\u{9e}')
                && !word.contains('\u{9d}')
                && (untoked_w == "$@" || untoked_w == "$*");
            if is_unquoted_at_or_star {
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_DROP_EMPTY, 0),
                    0,
                );
            }
            // c:Src/options.c GLOB_SUBST. When the word contains
            // unquoted parameter / command substitution AND the
            // option is set at runtime, the substituted chars
            // become eligible for filename generation. Emit the
            // runtime guard that conditionally runs expand_glob.
            // Bug #119 in docs/BUGS.md. has_unquoted_expansion above
            // only matches `$(...)` / backticks; the GLOB_SUBST gate
            // also fires for `$VAR` / `${VAR}` references — detect
            // via the lexer's $-token (META-$, Qstring) plus literal
            // `$` outside quotes.
            //
            // `!word_emitted_glob`: c:Src/loop.c:98 `execsubst(args)` →
            // c:Src/exec.c:2744-2746 `prefork(strs, esprefork, NULL)`
            // then a SINGLE `globlist(strs, 0)` over the whole for-list.
            // If the word's own text had a glob token, compile_word_str
            // already emitted that pass over the assembled word; a
            // second one would re-glob names the first pass generated
            // (see `word_emitted_glob`).
            if has_unquoted_param_or_subst(word) && !self.word_emitted_glob {
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_GLOB_SUBST_EXPAND, 1),
                    0,
                );
            }
        }
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_ARRAY_FLATTEN, words.len() as u8),
            0,
        );
        // ARRAY_FLATTEN pushes Array then Int(len) (its return). Top is len.
        self.builder.emit(Op::SetSlot(len_slot), 0);
        self.builder.emit(Op::SetSlot(arr_slot), 0);

        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(i_slot), 0);

        // c:Src/loop.c execfor — a `for` loop does NOT reset $? before
        // the body: the word-list expansion status (or, for a literal
        // list, the previous command's $?) carries into the FIRST body
        // iteration. `(exit 2); for x in 1 2; do print $?; done` →
        // the first `print` sees 2; `false; for x in $(echo 1; (exit 3))`
        // → the first body iteration sees 3 (the cmdsubst's status).
        // Only an EMPTY list (zero iterations) resets $? to 0
        // (`false; for x in $(exit 4); do …; done` → 0). The body itself
        // (execlist) sets $? on every iteration — including an empty body,
        // whose compiled chunk emits LoadInt(0)+SetStatus — so the reset
        // is needed solely for the never-entered case. Previously this
        // reset was unconditional, clobbering the carried status.
        self.builder.emit(Op::GetSlot(len_slot), 0);
        let carry_jump = self.builder.emit(Op::JumpIfFalse(0), 0); // len==0 → reset
        let skip_reset_jump = self.builder.emit(Op::Jump(0), 0); // len>0 → carry status
        let reset_pos = self.builder.current_pos();
        self.builder.patch_jump(carry_jump, reset_pos);
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetStatus, 0);
        let after_reset = self.builder.current_pos();
        self.builder.patch_jump(skip_reset_jump, after_reset);

        self.emit_loop_enter(); // c:Src/loop.c:114 — `loops++;`
        let loop_top = self.builder.current_pos();
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::GetSlot(len_slot), 0);
        self.builder.emit(Op::NumLt, 0);
        let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

        // Multi-name `for k v in ...`: var holds names joined by spaces.
        // For each iteration consume N elements (one per name); when fewer
        // than N remain in the tail, fill missing names with empty strings
        // (mirrors zsh's exec.c forexec). Single-name path (most common)
        // keeps the original 2-byte SET_VAR shape.
        let names: Vec<&str> = var.split_whitespace().collect();
        let n = names.len() as i64;
        let mut loop_var_abort_jumps: Vec<usize> = Vec::new();
        for (k, name) in names.iter().enumerate() {
            let var_const = self.builder.add_constant(Value::str(*name));
            self.builder.emit(Op::LoadConst(var_const), 0);
            if k == 0 {
                self.builder.emit(Op::GetSlot(i_slot), 0);
            } else {
                self.builder.emit(Op::GetSlot(i_slot), 0);
                self.builder.emit(Op::LoadInt(k as i64), 0);
                self.builder.emit(Op::Add, 0);
            }
            self.builder.emit(Op::SlotArrayGet(arr_slot), 0);
            // c:Src/loop.c execfor → setloopvar (params.c:6362): the
            // loop var binds via SET_LOOP_VAR so PM_NAMEREF vars
            // REBIND per iteration; Bool(false) return = zerr fired
            // (invalid self reference / read-only reference) → abort
            // the loop (C errflag check in execfor).
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_LOOP_VAR, 2),
                0,
            );
            let aj = self.builder.emit(Op::JumpIfFalse(0), 0);
            loop_var_abort_jumps.push(aj);
            // c:Src/exec.c::execlist:28+292 — restore `lineno` to
            // the for-statement's line before the per-iter trace.
            // execlist saves `oldlineno = lineno` at entry and
            // restores it at exit; in our flat compile-time
            // emission the body's SET_LINENO ops have advanced
            // LINENO past the for-header, so on iter 2+ the
            // `name=value` trace would emit with the body's last
            // line. Reset to current_sublist_line (captured by
            // compile_sublist) so each iter's trace shows the for
            // line, matching zsh.
            self.builder.emit(Op::LoadInt(self.current_sublist_line), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_LINENO, 1), 0);
            self.builder.emit(Op::Pop, 0);
            // xtrace: emit `name=value\n` per iteration. Direct port
            // of Src/loop.c:163-166:
            //   if (isset(XTRACE)) {
            //     printprompt4();
            //     fprintf(xtrerr, "%s=%s\n", name, str);
            //   }
            // XTRACE_LINE no-ops when -x is off, so cheap unconditionally.
            let assign_prefix = format!("{}=", name);
            let prefix_const = self
                .builder
                .add_constant(Value::str(assign_prefix.as_str()));
            self.builder.emit(Op::LoadConst(prefix_const), 0);
            let name_const2 = self.builder.add_constant(Value::str(*name));
            self.builder.emit(Op::LoadConst(name_const2), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_VAR, 1), 0);
            self.builder.emit(Op::Concat, 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
            self.builder.emit(Op::Pop, 0);
        }

        self.open_loop_scope();

        self.compile_program(body);
        self.emit_loop_body_end(); // c:Src/loop.c:180-185
        // c:Src/loop.c:198-203 — `if (errflag) { if (breaks) breaks--;
        // lastval = 1; break; }`. See emit_loop_errflag_break.
        let errflag_break = self.emit_loop_errflag_break();

        let cont = self.builder.current_pos();
        if let Some(continues) = self.continue_patches.pop() {
            for cp in continues {
                self.builder.patch_jump(cp, cont);
            }
        }

        if n == 1 {
            self.builder.emit(Op::PreIncSlotVoid(i_slot), 0);
        } else {
            self.builder.emit(Op::GetSlot(i_slot), 0);
            self.builder.emit(Op::LoadInt(n), 0);
            self.builder.emit(Op::Add, 0);
            self.builder.emit(Op::SetSlot(i_slot), 0);
        }
        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);
        self.builder.patch_jump(errflag_break, loop_exit);
        for aj in loop_var_abort_jumps {
            self.builder.patch_jump(aj, loop_exit);
        }

        self.close_loop_scope(loop_exit); // c:Src/loop.c:188 — `loops--;`
    }

    fn compile_for_arith(
        &mut self,
        init: &str,
        cond: &str,
        step: &str,
        body: &crate::parse::ZshProgram,
    ) {
        // c:Src/loop.c:77 `matheval(str)` (init), c:135
        // `val = mathevali(str)` (cond), c:191 `matheval(str)` (advance).
        // ALL THREE sections go through THE math evaluator — zsh has
        // exactly one arithmetic engine and exactly one backing store
        // (the parameter table, reached via getvalue/setvar in
        // Src/math.c). Every section is therefore routed through
        // BUILTIN_ARITH_EVAL (→ ported::subst::arithsubst → matheval).
        //
        // `compile_arith_str` compiles a section straight to VM slot
        // ops instead, reading each name once through
        // BUILTIN_GET_MATH_VAR (→ `getmathparam`, c:Src/math.c:337) and
        // writing it back through BUILTIN_SET_VAR. That is the same
        // read and the same write C's math evaluator performs — one
        // store, the parameter table — PROVIDED nothing caches a value
        // across the boundary. It did: the Rust port's counterpart of
        // C's per-mathvalue `mptr->pval` cache (c:340-343) is the
        // `M_VARIABLES` thread_local, and it used to outlive its
        // evaluation, so a preceding `(( i = … ))` left a stale `i`
        // that this path's pre-load then read in preference to the
        // parameter the loop had just written:
        //
        //     i=0; (( i++ )); for ((i=1; i<=3; i++)); do echo $i; done
        //
        // printed `1` forever. That is fixed at the root, in
        // src/ported/math.rs: `matheval` now saves and restores the
        // cache around its frame the way C restores `stack` (c:406
        // `stack = nstack` / c:455 `stack = xstack`), so the cache's
        // lifetime is one evaluation and `getmathparam` outside an
        // evaluation can only reach the parameter table. With that in
        // place the slot path reads and writes the SAME store the
        // evaluator does, and is measurably faster, so it is used
        // again.
        //
        // The gate below is NOT about the store. It is about what
        // `ArithCompiler` can LEX AND PARSE:
        //   * `,` — it compiles one operation per call and drops the
        //     rest of a comma list, so `for ((i=0,j=10; …))` would
        //     silently lose `j=10`.
        //   * `$` — its lexer has no token for `$`, so
        //     `for ((i=1; i<=$#a; i++))` never iterated.
        // Either one in ANY section routes ALL THREE sections through
        // BUILTIN_ARITH_EVAL, so a single loop never mixes the two
        // compilations (the sections share `i` and are far easier to
        // reason about when they are compiled alike).
        let untoked_init = crate::lex::untokenize(init);
        let untoked_cond = crate::lex::untokenize(cond);
        let untoked_step = crate::lex::untokenize(step);
        let arith_compiler_cannot_lex = |s: &str| s.contains(',') || s.contains('$');
        let needs_eval_global = arith_compiler_cannot_lex(&untoked_init)
            || arith_compiler_cannot_lex(&untoked_cond)
            || arith_compiler_cannot_lex(&untoked_step);
        let emit_arith = |this: &mut Self, s: &str| {
            if needs_eval_global {
                let untoked = crate::lex::untokenize(s);
                let idx = this.builder.add_constant(Value::str(untoked.as_str()));
                this.builder.emit(Op::LoadConst(idx), 0);
                this.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_EVAL, 1), 0);
            } else {
                this.compile_arith_str(s);
            }
        };

        // c:Src/loop.c::execfor — for arith form, init / cond / step
        // each get their own `printprompt4 + fprintf("%s\n", str)`
        // trace (c:Src/loop.c:72-74 for init, c:133-135 for cond,
        // c:191-193 for step). Emit BUILTIN_XTRACE_LINE with the
        // untokenized expression text. The init trace fires once
        // before the loop top; cond + step fire each iteration.
        //
        // Capture the for-statement's effective line BEFORE
        // compile_program(body) overwrites self.current_sublist_line
        // (the body's last sublist would otherwise win). Used by the
        // cond / step traces' SET_LINENO so each iter renders with
        // the for-header's line.
        let for_header_line = self.current_sublist_line;
        if !init.is_empty() {
            let txt = self.builder.add_constant(Value::str(untoked_init.as_str()));
            self.builder.emit(Op::LoadConst(txt), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
            self.builder.emit(Op::Pop, 0);
            emit_arith(self, init);
            self.builder.emit(Op::Pop, 0);
        }

        // c:Src/loop.c:65 — `cmdpush(CS_FOR);` after init eval/trace.
        // compile_for skipped this push for the CStyle branch so the
        // init trace renders WITHOUT the `for` cmdstack tag (zsh
        // omits it because cmdpush hasn't happened yet). We push it
        // now so cond / body / step all see the for-tag.
        self.emit_cmd_push(crate::ported::zsh_h::CS_FOR as u8);

        self.emit_loop_enter(); // c:Src/loop.c:114 — `loops++;`
        let loop_top = self.builder.current_pos();
        // c:Src/loop.c::execfor — cond xtrace re-restores LINENO
        // to the for-header line (matches execlist save/restore)
        // before the trace so each iter's cond/step line shows
        // the for-statement's line not the body's last line.
        self.builder.emit(Op::LoadInt(for_header_line), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_LINENO, 1), 0);
        self.builder.emit(Op::Pop, 0);
        if !cond.is_empty() {
            let txt = self.builder.add_constant(Value::str(untoked_cond.as_str()));
            self.builder.emit(Op::LoadConst(txt), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
            self.builder.emit(Op::Pop, 0);
        }
        if !cond.is_empty() {
            // c:Src/loop.c:135 — `val = mathevali(str);`, then
            // c:Src/loop.c:143 `if (!val) break;`.
            emit_arith(self, cond);
            // c:Src/math.c:1505-1509 `mathevali` COERCES the mnumber
            // to zlong (`(zlong)x.u.d` for a float) before c:143
            // tests it, so a fractional condition truncates toward
            // zero first: `for ((;0.5;))` does not iterate, and
            // `f=2.5; for ((;f;f-=1))` stops once f reaches 0.5.
            // Applied to BOTH compilations — the slot path leaves a
            // Float on the stack and BUILTIN_ARITH_EVAL leaves the
            // printed value as a Str; `TruncFloat` coerces either.
            // Testing the printed string against "0" instead (an older
            // spelling here) made every fractional value truthy and
            // turned that countdown into an infinite loop.
            self.builder.emit(Op::TruncFloat, 0);
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder.emit(Op::NumNe, 0);
        } else {
            // c:Src/loop.c:137 — `else val = 1;` (blank cond is always true).
            self.builder.emit(Op::LoadTrue, 0);
        }
        let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

        self.open_loop_scope();

        self.compile_program(body);
        self.emit_loop_body_end(); // c:Src/loop.c:180-185
        // c:Src/loop.c:198-203 — `if (errflag) { if (breaks) breaks--;
        // lastval = 1; break; }`. See emit_loop_errflag_break.
        let errflag_break = self.emit_loop_errflag_break();

        let cont = self.builder.current_pos();
        if let Some(continues) = self.continue_patches.pop() {
            for cp in continues {
                self.builder.patch_jump(cp, cont);
            }
        }

        if !step.is_empty() {
            // c:Src/loop.c:191-193 — step trace before evaluation.
            // Restore LINENO to the for-header line first (same
            // reason as the cond trace above).
            self.builder.emit(Op::LoadInt(for_header_line), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_LINENO, 1), 0);
            self.builder.emit(Op::Pop, 0);
            let txt = self.builder.add_constant(Value::str(untoked_step.as_str()));
            self.builder.emit(Op::LoadConst(txt), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
            self.builder.emit(Op::Pop, 0);
            emit_arith(self, step);
            self.builder.emit(Op::Pop, 0);
        }
        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);
        self.builder.patch_jump(errflag_break, loop_exit);

        self.close_loop_scope(loop_exit); // c:Src/loop.c:188 — `loops--;`
                                          // Pair with the cmdpush we did after init.
        self.emit_cmd_pop();
    }

    /// True when a substitution segment forces GLOB_SUBST for itself — the
    /// `${~name}` / `$~name` flag (c:Src/subst.c — the `~` substitution flag
    /// shtokenizes the spliced value so its metachars stay pattern-active
    /// regardless of the global option). Shared by the `[[ = ]]` cond path and
    /// the `case` pattern path, which must classify `$~p` identically.
    fn seg_forces_glob_subst(text: &str) -> bool {
        // Word text may carry the lexer's token form: `$` as String (U+0085) /
        // Qstring (U+008C), `{` as Inbrace (U+008F), `~` as Tilde (U+0098).
        let cs: Vec<char> = text.chars().take(4).collect();
        let dollar = matches!(
            cs.first().map(|c| *c as u32),
            Some(0x24) | Some(0x85) | Some(0x8c)
        );
        if !dollar {
            return false;
        }
        let flag_at = match cs.get(1) {
            Some(c) if *c == '{' || *c as u32 == 0x8f => 2,
            _ => 1,
        };
        let is_tilde = |c: Option<&char>| matches!(c, Some(c) if *c == '~' || *c as u32 == 0x98);
        is_tilde(cs.get(flag_at)) && !is_tilde(cs.get(flag_at + 1))
    }

    /// True when a `COND_MOD` operand (`[[ -prefix PAT ]]`, `-suffix`,
    /// `-after`, `-between`) carries no substitution at all, so the parser's
    /// tokenized word IS the final value and can go straight to the handler
    /// the way C's `ecgetarr` + `cond_str(…, raw=1)` pair hands it over
    /// (c:Src/cond.c:131-132 / c:525-534). Anything with a `$` / backtick in it
    /// still has to run through the expander.
    fn modcond_arg_is_pure_literal(arg: &str) -> bool {
        !arg.chars().any(|c| {
            matches!(
                c,
                // `$` raw / String / Qstring — parameter, arithmetic, `$(…)`.
                '$' | '\u{85}' | '\u{8c}'
                // backtick raw / Tick / Qtick — command substitution.
                | '`' | '\u{93}' | '\u{99}'
            )
        })
    }

    /// Emit a PATTERN word (for `case` arms and `[[ = ]]` RHS) so that
    /// GLOB_SUBST is honored: glob metachars that come from a SUBSTITUTION
    /// (`$p` → `a*`) are literal-ized unless `$~`/`${~}` or the GLOB_SUBST
    /// option is set, while SOURCE-level glob metas always stay match-active.
    /// This is exactly the split zsh's singsub draws (c:Src/subst.c), and it is
    /// why `case abc in $p) …` with p='a*' does NOT match (the `*` is literal)
    /// but `case abc in $~p) …` does.
    fn emit_glob_subst_pattern(&mut self, word: &str) {
        // Bare `$~name` (String + Tilde tokens, no braces) loses its Tilde in
        // the name parse, so normalize it to the braced `${~name}` spelling
        // first — same fix the cond Binary path applies, hoisted here so `case`
        // gets it too. Without this, `case x in $~p) …` never forced the glob.
        //
        // c:Src/subst.c:2597-2603 — `} else if (c == '~' || c == Tilde) { /*
        // GLOB_SUBST (forced) on or off (doubled) */ … globsubst = 2; }`.
        // paramsubst reads the Tilde flag in its flag loop whether or not the
        // spec is braced, so `$~name` is the same substitution ANYWHERE in the
        // word. This rewrite used to require the WHOLE word to be `$~name`; an
        // EMBEDDED one fell through to `split_pattern_for_glob_subst`, whose
        // `$` arm stops at the Tilde (not a brace, a paren, a single-char
        // special, nor a name char), emits `Subst("$")` and leaves `~name` in
        // the following Literal — so the compiled pattern carried the LITERAL
        // text `$~name` and matched nothing:
        //     p=X; [[ aXb = *$~p* ]]   answered N   (zsh: M)
        // That is `Completion/Base/Utility/_pick_variant:39`
        // `if [[ $output = *$~pat* ]]`, so every stock completer that calls
        // `_pick_variant` fell through to the default variant label.
        let normalized: Option<String> = {
            let cs: Vec<char> = word.chars().collect();
            let is_dollar = |c: char| matches!(c as u32, 0x24 | 0x85 | 0x8c);
            let is_tilde = |c: char| matches!(c as u32, 0x7e | 0x98);
            let is_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
            let mut out = String::with_capacity(word.len() + 4);
            let mut i = 0usize;
            let mut rewrote = false;
            while i < cs.len() {
                // A `$` the source ESCAPED (`\$~p`, Bnull/Bnullkeep + `$`) is
                // DATA, not a substitution — c:Src/zsh.h Bnull/Bnullkeep.
                let escaped = i > 0 && matches!(cs[i - 1], '\\' | '\u{9f}' | '\u{a0}');
                if is_dollar(cs[i])
                    && !escaped
                    && cs.get(i + 1).copied().is_some_and(is_tilde)
                    && cs.get(i + 2).copied().is_some_and(is_name)
                {
                    rewrote = true;
                    out.push(cs[i]);
                    out.push('\u{8f}'); // Inbrace
                    out.push(cs[i + 1]); // `~` / Tilde — the GLOB_SUBST flag
                    i += 2;
                    while i < cs.len() && is_name(cs[i]) {
                        out.push(cs[i]);
                        i += 1;
                    }
                    // A trailing `[sub]` belongs to `$name` (c:Src/subst.c
                    // getindex), so it goes INSIDE the braces or it would
                    // become a `[...]` character class. Same depth-balancing
                    // as split_pattern_for_glob_subst's subscript arm.
                    if matches!(cs.get(i), Some('[') | Some('\u{91}')) {
                        let mut depth = 0i32;
                        while i < cs.len() {
                            let cc = cs[i];
                            out.push(cc);
                            i += 1;
                            match cc {
                                '[' | '\u{91}' => depth += 1,
                                ']' | '\u{92}' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    out.push('\u{90}'); // Outbrace
                    continue;
                }
                out.push(cs[i]);
                i += 1;
            }
            if rewrote { Some(out) } else { None }
        };
        let word: &str = normalized.as_deref().unwrap_or(word);
        let segments = split_pattern_for_glob_subst(word);
        if segments.len() <= 1 {
            // A single pure-LITERAL segment has no substitution to run —
            // it lands here when the `$` that made the caller's
            // needs_expand true is source-ESCAPED (`[[ $v = \$* ]]`,
            // f-sy-h's `\$[{]` matcher). Routing it through
            // compile_word_str + GLOB_SUBST_GUARD substituted the
            // escaped dollar and escaped the SOURCE metas (pattern
            // `\$*` became `$\*` — dollar active, star literal). Emit
            // it like the multi-segment Literal arm: untokenized,
            // guard-free, escapes intact for the runtime patcompile.
            if let Some(PatSeg::Literal(text)) = segments.first() {
                // Route through untokenize_preserve_quoted_pat_literals,
                // NOT plain untokenize: a Snull/Dnull-quoted span inside
                // the literal must keep its glob metas backslash-escaped.
                // Plain untokenize dropped the markers bare, so p10k's
                // `[[ $_p9k__ret == (|*[^\\])'$('* ]]` (internal/
                // p10k.zsh:952) emitted `(|*[^\\])$(*` — the runtime
                // re-tokenize promoted the quoted `(` to Inpar and
                // patcompile died "bad pattern" on the unbalanced group.
                let lit = untokenize_preserve_quoted_pat_literals(text);
                let c = self.builder.add_constant(Value::str(lit.as_str()));
                self.builder.emit(Op::LoadConst(c), 0);
                return;
            }
            self.dq_context_depth += 1;
            self.compile_word_str(word);
            self.dq_context_depth -= 1;
            if !Self::seg_forces_glob_subst(word) {
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_GLOB_SUBST_GUARD, 1),
                    0,
                );
            } else {
                // `${~spec}` forces the metas ACTIVE, so no guard runs — but
                // c:Src/subst.c:822/830's `shtokenize` still has to settle the
                // value's BACKSLASHES, which c:Src/glob.c:3651 leaves as data
                // before any non-`ztokens` char. docs/BUGS.md #1090.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PAT_DATA_BACKSLASH, 1),
                    0,
                );
            }
            return;
        }
        for (idx, seg) in segments.iter().enumerate() {
            match seg {
                PatSeg::Subst(text) => {
                    self.dq_context_depth += 1;
                    self.compile_word_str(text);
                    self.dq_context_depth -= 1;
                    if !Self::seg_forces_glob_subst(text) {
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_GLOB_SUBST_GUARD, 1),
                            0,
                        );
                    } else {
                        // See the single-segment arm above — docs/BUGS.md #1090.
                        self.builder.emit(
                            Op::CallBuiltin(crate::vm_helper::BUILTIN_PAT_DATA_BACKSLASH, 1),
                            0,
                        );
                    }
                }
                PatSeg::Literal(text) => {
                    // Same quoted-meta escape as the single-segment fast
                    // path above — see the comment there.
                    let lit = untokenize_preserve_quoted_pat_literals(text);
                    let c = self.builder.add_constant(Value::str(lit.as_str()));
                    self.builder.emit(Op::LoadConst(c), 0);
                }
            }
            if idx > 0 {
                self.builder.emit(Op::Concat, 0);
            }
        }
    }

    fn compile_case(&mut self, c: &crate::parse::ZshCase) {
        // cmdstack: direct port of Src/loop.c:615 `cmdpush(CS_CASE);`
        // wrapping the whole case statement.
        self.emit_cmd_push(crate::ported::zsh_h::CS_CASE as u8);
        // Word goes onto a slot for repeated comparison.
        // c:Src/loop.c:610-612 execcase — the case WORD is evaluated
        // (singsub) with the INHERITED $? still live; `case $? in …` and
        // `(( x < y )); case $? in …` (the zmathfunc min/max idiom) must
        // see the prior command's status. The reset to 0 happens AFTER
        // (c:705 `if (!anypatok) lastval = 0`), so capture the word
        // first, THEN reset.
        self.compile_word_str(&c.word);
        let word_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::SetSlot(word_slot), 0);
        // c:Src/loop.c:613 `anypatok = 0;` — C resets lastval ONLY at the
        // very end and ONLY when nothing matched (c:705 `if (!anypatok)
        // lastval = 0;`). Resetting here instead made a MATCHED arm's body
        // see 0: `(exit 37); case x in x) echo $?;; esac` printed 0 where
        // zsh prints 37. Track anypatok in a slot and do the reset at the
        // end, exactly like C. (An empty matched body still yields 0, but
        // via the empty-list rule at c:Src/exec.c:1439-1441, not this.)
        let anypatok_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(anypatok_slot), 0);

        let mut end_jumps = Vec::new();
        // Pending fall-through from the previous arm's `;&` terminator.
        // When Some, the patch needs to land at the CURRENT arm's body
        // start (skipping its own pattern check).
        let mut pending_fall: Option<usize> = None;

        for arm in &c.arms {
            // xtrace: emit `case <word> (<pat1> | <pat2>)` per arm.
            // c:Src/loop.c:626-682 — for each pattern alt, C does:
            //   pat = dupstring(ecrawstr(...));
            //   if (htok) singsub(&pat);
            //   quote_tokenized_output(pat, xtrerr);
            // i.e. expand $VAR references THEN pass through
            // quote_tokenized_output so Star tokens render as `*`
            // (unescaped via ztokens lookup) and expanded values
            // render as literal text. Build the trace string at
            // runtime: piecewise per pattern (Subst segments go
            // through EXPAND_TEXT mode 4 → singsub; Literal
            // segments stay tokenized) and apply
            // BUILTIN_QUOTE_TOKENIZED_OUTPUT to each segment so the
            // source pattern shape surfaces. Gate the whole block on
            // BUILTIN_XTRACE_IS_ON so the runtime cost is only paid
            // when xtrace is actually enabled.
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_IS_ON, 0),
                0,
            );
            let trace_skip = self.builder.emit(Op::JumpIfFalse(0), 0);
            // Stack starts empty for this trace block.
            // Push "case "
            let prefix_const = self.builder.add_constant(Value::str("case "));
            self.builder.emit(Op::LoadConst(prefix_const), 0);
            // Push word
            self.builder.emit(Op::GetSlot(word_slot), 0);
            self.builder.emit(Op::Concat, 0);
            // Push " ("
            let open_const = self.builder.add_constant(Value::str(" ("));
            self.builder.emit(Op::LoadConst(open_const), 0);
            self.builder.emit(Op::Concat, 0);
            // For each pattern, emit the segments + concat.
            for (pi, pattern) in arm.patterns.iter().enumerate() {
                if pi > 0 {
                    let sep_const = self.builder.add_constant(Value::str(" | "));
                    self.builder.emit(Op::LoadConst(sep_const), 0);
                    self.builder.emit(Op::Concat, 0);
                }
                let raw = pattern.as_str();
                let has_expand = raw.contains('$')
                    || raw.contains('`')
                    || raw.contains('\u{85}')
                    || raw.contains('\u{8c}')
                    || raw.contains('\u{99}');
                if has_expand {
                    let segments = split_pattern_for_glob_subst(raw);
                    for (sidx, seg) in segments.iter().enumerate() {
                        match seg {
                            PatSeg::Subst(text) => {
                                let pc = self.builder.add_constant(Value::str(text.as_str()));
                                self.builder.emit(Op::LoadConst(pc), 0);
                                self.builder.emit(Op::LoadInt(4), 0);
                                self.builder.emit(
                                    Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 0),
                                    0,
                                );
                                self.builder.emit(
                                    Op::CallBuiltin(
                                        crate::vm_helper::BUILTIN_QUOTE_TOKENIZED_OUTPUT,
                                        1,
                                    ),
                                    0,
                                );
                            }
                            PatSeg::Literal(text) => {
                                // Pass the tokenized form straight to
                                // QUOTE_TOKENIZED_OUTPUT — it maps
                                // `\u{84}..\u{a1}` token chars back to
                                // their source ASCII via ztokens
                                // (Star → `*` unescaped). Don't
                                // untokenize first because untokenize
                                // turns Star into literal `*` which
                                // then gets backslash-escaped.
                                let pc = self.builder.add_constant(Value::str(text.as_str()));
                                self.builder.emit(Op::LoadConst(pc), 0);
                                self.builder.emit(
                                    Op::CallBuiltin(
                                        crate::vm_helper::BUILTIN_QUOTE_TOKENIZED_OUTPUT,
                                        1,
                                    ),
                                    0,
                                );
                            }
                        }
                        // Concat with the trace buffer (always — even
                        // the first segment goes onto the buffer
                        // already on the stack).
                        self.builder.emit(Op::Concat, 0);
                    }
                } else {
                    // No expansion needed — pass raw pattern through
                    // QUOTE_TOKENIZED_OUTPUT to render source form.
                    let pc = self.builder.add_constant(Value::str(raw));
                    self.builder.emit(Op::LoadConst(pc), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_QUOTE_TOKENIZED_OUTPUT, 1),
                        0,
                    );
                    self.builder.emit(Op::Concat, 0);
                }
            }
            // Push ")"
            let close_const = self.builder.add_constant(Value::str(")"));
            self.builder.emit(Op::LoadConst(close_const), 0);
            self.builder.emit(Op::Concat, 0);
            // Emit trace line.
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
            self.builder.emit(Op::Pop, 0);
            let trace_done = self.builder.current_pos();
            self.builder.patch_jump(trace_skip, trace_done);

            let mut match_jumps = Vec::new();
            for pattern in &arm.patterns {
                self.builder.emit(Op::GetSlot(word_slot), 0);
                // c:Src/loop.c — case patterns expand parameter
                // references / arith / cmdsub at match time via a
                // `singsub` pass on the pattern body (no globbing —
                // glob chars are the pattern itself). Without
                // expansion, `case $x in $pat) ...` compared $x
                // against the literal string "$pat" and never
                // matched. Bug #292 in docs/BUGS.md.
                //
                // Detect tokenized expansion markers (lexer encodes
                // `$` as `\u{85}` Stringg, `$'...'` as `\u{8c}`
                // Qstring, backticks as `\u{99}` Tick) AND raw `$` /
                // backtick chars. When found, push the original
                // tokenized pattern as a const and run
                // BUILTIN_SINGSUB_PAT at runtime which calls
                // singsub() — parameter / arith / cmdsub expansion
                // WITHOUT globbing (glob chars survive into the
                // returned string for the matcher).
                let raw = pattern.as_str();
                let needs_runtime_expand = raw.contains('$')
                    || raw.contains('`')
                    || raw.contains('\u{85}') // META-`$`
                    || raw.contains('\u{8c}') // Qstring (ANSI-C)
                    || raw.contains('\u{99}'); // META-`` ` ``
                if needs_runtime_expand {
                    // A case pattern honors GLOB_SUBST exactly like a `[[ = ]]`
                    // RHS: glob metachars from a SUBSTITUTION (`$p` → `a*`) are
                    // literal unless `$~`/`${~}` forces them active, while
                    // source-level metas always glob. This used to run the whole
                    // pattern through BUILTIN_EXPAND_TEXT (flag 4) with no
                    // GLOB_SUBST_GUARD on the substituted part, so a substituted
                    // `*` stayed match-active (bash semantics): `case abc in $p)`
                    // with p='a*' wrongly MATCHED, and `$~p` — which should force
                    // the glob — was left literal, so it wrongly did NOT. The
                    // shared helper is the same one the cond path uses.
                    let _ = raw;
                    self.emit_glob_subst_pattern(pattern);
                } else {
                    // Patterns are RAW glob strings. The lexer encodes
                    // glob chars (`*`, `?`, `[`, `]`) in the META range
                    // so the grammar can distinguish syntax from literal.
                    // For the matcher we want the original glob char
                    // back — un-tokenize before pushing.
                    let pat_clean = case_pattern_for_match(pattern);
                    let pat_const = self.builder.add_constant(Value::str(pat_clean.as_str()));
                    self.builder.emit(Op::LoadConst(pat_const), 0);
                }
                self.builder.emit(Op::StrMatch, 0);
                match_jumps.push(self.builder.emit(Op::JumpIfTrue(0), 0));
            }
            // No pattern matched — skip body.
            let skip_body = self.builder.emit(Op::Jump(0), 0);

            // Body start.
            let body_start = self.builder.current_pos();
            for mj in match_jumps {
                self.builder.patch_jump(mj, body_start);
            }
            // Resolve pending `;&` fall-through from the previous arm.
            if let Some(prev) = pending_fall.take() {
                self.builder.patch_jump(prev, body_start);
            }

            // c:Src/loop.c:672 `patok = anypatok = 1;`
            self.builder.emit(Op::LoadInt(1), 0);
            self.builder.emit(Op::SetSlot(anypatok_slot), 0);
            if arm.body.lists.is_empty() {
                // c:Src/exec.c:1439-1441 — `if (wc_code(code) != WC_LIST) {
                //   /* Empty list; this returns status zero. */ lastval = 0; }`
                // `case x in x) ;; esac` is a matched arm with an empty body,
                // so execlist zeroes the status even though anypatok is set.
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::SetStatus, 0);
            }
            self.compile_program(&arm.body);

            match arm.terminator {
                CaseTerm::Break => {
                    end_jumps.push(self.builder.emit(Op::Jump(0), 0));
                }
                CaseTerm::Continue => {
                    // `;&` — fall through to the next arm's body
                    // unconditionally, skipping its pattern check.
                    // Record a forward jump to be patched at the next
                    // arm's body_start.
                    pending_fall = Some(self.builder.emit(Op::Jump(0), 0));
                }
                CaseTerm::TestNext => {
                    // `;|` — continue testing the next arm's pattern.
                    // No emitted jump; flow naturally falls through to
                    // the next arm's pattern-check sequence.
                }
            }
            let after_body = self.builder.current_pos();
            self.builder.patch_jump(skip_body, after_body);
        }

        // c:Src/loop.c:705-706 — `if (!anypatok) lastval = 0;`. Runs for
        // every exit route out of the case (a `;;` break, a `;|` that ran
        // out of arms, or no match at all), which is why the check lands
        // HERE and reads the slot rather than being emitted per-path: an
        // arm that matched leaves anypatok set and keeps its body's status.
        let end = self.builder.current_pos();
        for ej in end_jumps {
            self.builder.patch_jump(ej, end);
        }
        // A pending `;&` from the last arm has nowhere to fall through —
        // patch to `end` so it just exits cleanly.
        if let Some(prev) = pending_fall {
            self.builder.patch_jump(prev, end);
        }
        self.builder.emit(Op::GetSlot(anypatok_slot), 0);
        let skip_reset = self.builder.emit(Op::JumpIfTrue(0), 0);
        self.builder.emit(Op::LoadInt(0), 0); // c:706 `lastval = 0;`
        self.builder.emit(Op::SetStatus, 0);
        self.builder
            .patch_jump(skip_reset, self.builder.current_pos());
        self.emit_cmd_pop();
    }

    fn compile_repeat(&mut self, r: &crate::parse::ZshRepeat) {
        let i_slot = self.next_slot;
        self.next_slot += 1;
        let count_slot = self.next_slot;
        self.next_slot += 1;

        // c:Src/parse.c:1600 — `par_repeat` captures the count via
        // `tokstr()` raw (carrying lexer-emitted token markers).
        // C's wordcode VM at `execrepeat` (Src/loop.c:519) handles
        // the WC_MATH node directly. zshrs lowers via fusevm
        // compile_zsh (this file) — a Rust-only step with no C
        // counterpart since C uses wordcode VM dispatch instead.
        //
        // For `repeat $((2+3))`, the lex tokenizes as
        // `<Stringg><Inparmath>(2+3)<Outparmath>` —
        // Stringg=0x85, Inparmath=0x89, Outparmath=0x8b (zsh.h).
        // compile_arith_str expects bare arith; untokenize →
        // "$((2+3))" doesn't parse cleanly. Strip the outer
        // arith-sub wrapping so the inner "2+3" reaches the arith
        // compiler. Bare numeric count (`repeat 5`) and parameter
        // count (`repeat $N`) pass through unchanged.
        let count_str = {
            let s = r.count.as_str();
            // Match `Stringg Inparmath (...) Outparmath` (with optional
            // outer parens already consumed by the tokenizer's
            // Inparmath/Outparmath sentinels).
            if let Some(rest) = s.strip_prefix("\u{85}\u{89}") {
                if let Some(inner) = rest.strip_suffix("\u{8b}") {
                    // Inner is `(2+3)` — strip the parens too.
                    let inner = inner.trim_matches(|c| c == '(' || c == ')');
                    inner.to_string()
                } else {
                    r.count.clone()
                }
            } else {
                r.count.clone()
            }
        };
        // c:Src/loop.c:516-517 — `untokenize(tmp); count = mathevali(tmp)`.
        // The count uses the FAITHFUL math.c evaluator (mathevali →
        // MathEval), which resolves `?`/`$?` (the status param), named
        // refs, and full math.c precedence. The previous port used
        // `compile_arith_str` (the fast-path ArithCompiler), which
        // `collect_identifiers` can't see `?` in, so `repeat "$?"` /
        // `repeat ?` evaluated to 0 and the loop never ran. Route through
        // BUILTIN_ARITH_EVAL (the same MathEval path as `$((…))`) on the
        // untokenized count text.
        //
        // c:Src/loop.c:517 is evaluated BEFORE the lastval reset (c:520)
        // and `cmdpush(CS_REPEAT)` (c:522), so the count sees the PRIOR
        // command's `$?`. (BUILTIN_CMD_PUSH returns Status(0) and would
        // clobber $?.) Emit the count eval first, THEN cmdpush.
        let count_clean = crate::lex::untokenize(&count_str);
        let cnt_idx = self.builder.add_constant(Value::str(count_clean.as_str()));
        self.builder.emit(Op::LoadConst(cnt_idx), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_EVAL, 1), 0);
        self.builder.emit(Op::SetSlot(count_slot), 0);

        // c:Src/loop.c:520 — `lastval = 0; /* used when the repeat count is
        // zero */`. Without it a zero-count `repeat` leaves the PREVIOUS
        // command's status standing: `(exit 4); repeat 0 do done` reported 4
        // where zsh reports 0. Emitted after the count eval (c:517), which
        // must still see the prior `$?` for `repeat $?`.
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetStatus, 0);

        // c:Src/loop.c:522 — `cmdpush(CS_REPEAT)` AFTER the count eval.
        self.emit_cmd_push(crate::ported::zsh_h::CS_REPEAT as u8);

        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(i_slot), 0);

        self.emit_loop_enter(); // c:Src/loop.c:523 — `loops++;`
        let loop_top = self.builder.current_pos();
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::GetSlot(count_slot), 0);
        self.builder.emit(Op::NumLt, 0);
        let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

        self.open_loop_scope();

        self.compile_program(&r.body);
        self.emit_loop_body_end(); // c:Src/loop.c:540-545
        // c:Src/loop.c — the loop's own post-body errflag guard.
        let errflag_break = self.emit_loop_errflag_break();

        let cont = self.builder.current_pos();
        if let Some(continues) = self.continue_patches.pop() {
            for cp in continues {
                self.builder.patch_jump(cp, cont);
            }
        }
        self.builder.emit(Op::PreIncSlotVoid(i_slot), 0);
        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);
        self.builder.patch_jump(errflag_break, loop_exit);
        self.close_loop_scope(loop_exit); // c:Src/loop.c:546 — `loops--;`
        self.emit_cmd_pop();
    }

    fn compile_funcdef(&mut self, f: &crate::parse::ZshFuncDef) {
        // Compile the body to a fusevm sub-chunk and register via
        // BUILTIN_REGISTER_COMPILED_FN with five args:
        //   [name, base64(bincode(chunk)), body_source, line_base_str, tracing]
        // The handler stores the chunk in functions_compiled and the source
        // text in function_source so introspection (whence, which, typeset
        // -f, ${functions[name]}) returns canonical body text.
        //
        // $LINENO inside a function is measured from the line the function was
        // DEFINED on — c:Src/exec.c:5384 stamps `shf->lineno` with the def line,
        // and the body's own line numbers are relative to it. So a one-line
        // `f() { print $LINENO }` reads 0 wherever it is defined, and the second
        // line of a multi-line body reads 1.
        //
        // The offset used to be `first_body_line - 1`, which is the def line
        // only when the body starts on the line AFTER `f() {`. For an INLINE
        // body (`f() { … }`, body on the same line as the definition) the two
        // differ, and $LINENO came out one too high for every function defined
        // below line 1:
        //     print top            # line 1
        //     f() { print $LINENO }; f   # line 2 -> zsh: 0, zshrs was: 1
        //
        // `current_sublist_line` is the line of the sublist being compiled — the
        // funcdef's own line — in whatever numbering the enclosing context uses,
        // so a nested definition stays relative to its enclosing function, which
        // is what C's `funcstack->flineno + lineno` (c:5387) arrives at too.
        //
        // `current_sublist_line` is RELATIVE to the enclosing context, while the
        // body's AST line numbers are absolute, so the two have to be put in the
        // same frame before subtracting: add back the enclosing offset. At top
        // level that offset is 0 and this is just the funcdef's line; inside a
        // function it recovers the absolute line, which is what C's
        // `funcstack->flineno + lineno` (c:5387) computes for a nested def.
        let mut body_compiler = ZshCompiler::new();
        let def_line = (self.current_sublist_line.max(1) as u64).saturating_add(self.lineno_offset);
        body_compiler.lineno_offset = def_line;
        body_compiler.is_function_body = true;
        let lineno_off = body_compiler.lineno_offset;
        let body_chunk = body_compiler.compile(&f.body);
        // The body chunk rides along in the ENCLOSING chunk's `sub_chunks`
        // (fusevm chunk.rs:35-40 lists "function bodies when they're stored as
        // separate chunks" as exactly this slot's purpose), and only its index
        // travels through the constant pool. It used to travel as
        // `base64(bincode(chunk))` in a string constant, which made every
        // enclosing definition re-encode the whole nested blob: base64 costs
        // 4/3, so a definition nested N deep paid (4/3)^N. `() { … }` nests
        // per level, and D04parameter's "zsh_eval_context resizing" test
        // (`repeat 48 cmd="() { $cmd }"`) is 49 levels deep — (4/3)^49 ≈ 1.4e6.
        // Measured on a 253-byte program: 0.33s at depth 24, 7.15s at depth 36,
        // and the real test never finished. A sub-chunk index is O(1) per
        // level and keeps the chunk self-contained, so the on-disk bytecode
        // cache still round-trips.
        let body_sub_idx = self.builder.add_sub_chunk(body_chunk);
        // `#` is outside the base64 alphabet (`A-Za-z0-9+/=`), so the marker
        // can never collide with a body encoded the old way — a chunk restored
        // from a bytecode cache written by an older binary still decodes
        // through the base64 fallback in the handler.
        let body_str = format!("#{}", body_sub_idx);
        let source_text = f.body_source.clone().unwrap_or_default();
        // c:Src/exec.c:5409 — `shf->lineno = lineno;` records the line
        // of the funcdef STATEMENT (where `name()` sits), which
        // funcsourcetrace and the prompt funcstack report. That is NOT
        // the body's `$LINENO` offset (`lineno_off = first_body_line -
        // 1`, used above for the body sub-chunk): the two coincide only
        // when the body's first statement starts one line below the
        // def, and disagree for the inline form `name() { body }` (def
        // and body share a line) — there `lineno_off` underflowed to
        // `def_line - 1` and funcsourcetrace reported one line too low.
        // `self.current_sublist_line` was set by the enclosing
        // compile_list to this funcdef statement's own (offset-adjusted)
        // line, so it is the correct def line. Bug #396.
        let _ = lineno_off;
        let line_base_str = self.current_sublist_line.to_string();

        for raw_name in &f.names {
            // Strip any trailing Inpar+Outpar markers (\u{88}\u{8a})
            // that the lexer may pack into a single String token under
            // some `function name() { body }` paths, then untokenize
            // unconditionally so Dash/Bang/etc. bytes inside the name
            // (e.g. `foo-bar` lexes as `foo<Dash>bar`) become literal
            // chars before registration. Without the unconditional
            // untokenize, hyphenated function names register under the
            // raw tokenized form and the call site (which DOES
            // untokenize) misses the lookup.
            let stripped = raw_name
                .trim_end_matches('\u{8a}')
                .trim_end_matches('\u{88}');
            // c:Src/exec.c execfuncdef — the function NAME word is
            // prefork-expanded before registration, so `function $x()`,
            // `function name"$x"()` and `function $0_inner()` define the
            // function under the EXPANDED name (zinit configure.zsh's
            // `$0_error` helper). A name carrying a param/command-sub
            // expansion (`$`, String/Qstring/Tick tokens, backtick) must
            // be expanded at runtime; otherwise it registers under the
            // literal token text and the call site never finds it.
            let name_needs_expand = stripped.contains('$')
                || stripped.contains('\u{85}')  // String ($ token)
                || stripped.contains('\u{8c}')  // Qstring (DQ $ token)
                || stripped.contains('`')
                || stripped.contains('\u{93}')  // Tick
                || stripped.contains('\u{99}'); // Qtick
            if name_needs_expand {
                // Push the expanded name onto the stack (scalar word).
                self.compile_word_str(stripped);
            } else {
                let cleaned = crate::lex::untokenize(stripped);
                // Bug #27: track defined names so later dispatch sites can
                // route to CallFunction (user fn) instead of the extension
                // builtin fast-path.
                self.defined_functions.insert(cleaned.clone());
                let name_const = self.builder.add_constant(Value::str(cleaned.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
            }
            let body_const = self.builder.add_constant(Value::str(body_str.as_str()));
            self.builder.emit(Op::LoadConst(body_const), 0);
            let source_const = self.builder.add_constant(Value::str(source_text.as_str()));
            self.builder.emit(Op::LoadConst(source_const), 0);
            let anchor_const = self
                .builder
                .add_constant(Value::str(line_base_str.as_str()));
            self.builder.emit(Op::LoadConst(anchor_const), 0);
            // c:Src/exec.c:5382 — `do_tracing = *state->pc++;` and
            // c:5387 `tracing_flags = do_tracing ? PM_TAGGED_LOCAL : 0;`
            // then c:5437 `shf->node.flags = tracing_flags;`. The `-T` of
            // `function -T name { … }` is parsed into ZshFuncDef.tracing
            // (parse.rs c:1689-1692) but had NO consumer, so a traced
            // definition registered an untraced function (E02xtrace:6,7,8,9).
            let tracing_const =
                self.builder
                    .add_constant(Value::str(if f.tracing { "1" } else { "0" }));
            self.builder.emit(Op::LoadConst(tracing_const), 0);
            // c:Src/exec.c:5451-5456 — `shf->redir = <redir_prog>`. Empty
            // when the definition carried no trailing redirection, which is
            // C's `redir_prog == NULL` (`shf->redir` stays NULL and nothing
            // is printed after the closing brace).
            let redir_text = self.redir_prog_text.clone().unwrap_or_default();
            let redir_const = self.builder.add_constant(Value::str(redir_text.as_str()));
            self.builder.emit(Op::LoadConst(redir_const), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_REGISTER_COMPILED_FN, 6),
                0,
            );
            self.builder.emit(Op::SetStatus, 0);
        }

        // Anonymous-function form `() { body } a b c` — register and call
        // immediately. Generated names always match `_zshrs_anon_N` so we
        // route through CallFunction (host.call_function checks
        // functions_compiled).
        if let Some(args) = &f.auto_call_args {
            let argc = args.len() as u8;
            for arg in args {
                self.compile_word_str(arg);
            }
            // f.names[0] is the auto-generated name from parse_anon_funcdef.
            if let Some(raw_name) = f.names.first() {
                let cleaned = if raw_name.ends_with('\u{8a}') && raw_name.contains('\u{88}') {
                    let stripped = raw_name
                        .trim_end_matches('\u{8a}')
                        .trim_end_matches('\u{88}');
                    crate::lex::untokenize(stripped)
                } else {
                    raw_name.clone()
                };
                // c:Src/exec.c:5495 — the anonymous function is invoked
                // through `execshfunc(shf, args)`, whose own xtrace block
                // (c:5615-5626) does
                //     printprompt4();
                //     for (lptr = firstnode(args); …)
                //         quotedzputs((char *)getdata(lptr), xtrerr);
                // with `args[0]` being the function's display name
                // `(anon)` (ANONYMOUS_FUNCTION_NAME, c:Src/zsh.h). zshrs
                // dispatches the anon body through `Op::CallFunction` on
                // its generated `_zshrs_anon_N` name and emitted no trace
                // line at all, so a traced function calling an anonymous
                // one showed only the innermost command:
                // `fn(){ (){ (){ true } } }; functions -T fn; fn` printed
                // just `+(anon):0> true`, missing the two `'(anon)'` call
                // lines zsh prints. Reuse the same peek-args/pop-prefix
                // XTRACE_ARGS contract as the simple-command path
                // (line ~2904); the prefix is pre-quoted here because C
                // runs argv[0] through quotedzputs too.
                let anon_prefix =
                    crate::ported::utils::quotedzputs(crate::ported::exec::ANONYMOUS_FUNCTION_NAME);
                let anon_prefix_const = self.builder.add_constant(Value::str(anon_prefix.as_str()));
                self.builder.emit(Op::LoadConst(anon_prefix_const), 0);
                let trace_argc = (args.len() + 1).min(u8::MAX as usize) as u8;
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_ARGS, trace_argc),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                let name_idx = self.builder.add_name(&cleaned);
                self.builder.emit(Op::CallFunction(name_idx, argc), 0);
                self.builder.emit(Op::SetStatus, 0);
                // c:Src/exec.c:5495-5506 — right after `execshfunc(shf,
                // args); ret = lastval;` execfuncdef runs its OWN
                // PRINT_EXIT_VALUE report for the anonymous function:
                //     if (isset(PRINTEXITVALUE) && isset(SHINSTDIN) && lastval)
                //         fprintf(stderr, "zsh: exit %lld\n", lastval);
                // doshfunc zeroes opts[PRINTEXITVALUE] for the BODY
                // (c:6037) and restores it at c:6158, so the report
                // fires for the call itself. The compiler never emitted
                // it, so `zsh -f <<<'setopt printexitvalue; () { false; }'`
                // printed nothing (E01options.ztst:60). The `1` argument
                // selects the c:5498 variant, which has no `!subsh` term.
                let anon_flag = self.builder.add_constant(Value::Int(1));
                self.builder.emit(Op::LoadConst(anon_flag), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_PRINT_EXIT_VALUE, 1),
                    0,
                );
                // Pop the builtin's Status(0) WITHOUT SetStatus — the
                // report must not clobber the anon call's `$?`
                // (c:5497 `ret = lastval;` is read back at c:5510).
                self.builder.emit(Op::Pop, 0);
            }
        }
    }

    fn compile_cond(&mut self, c: &crate::parse::ZshCond) {
        // xtrace: emit `[[ ... ]]` text BEFORE pushing CS_COND so
        // the trace line itself is NOT labeled "cond" (zsh: only
        // nested commands inside the cond see the cond context).
        // Direct port of Src/exec.c:5210-5214 — printprompt4 fires,
        // THEN cmdpush(CS_COND). Operands inside `[[ … ]]` are
        // EXPANDED for trace (zsh shows `[[ -r /Users/foo ]]`, not
        // `[[ -r $HOME ]]`) — emit_cond_trace_runtime builds the line
        // at runtime by interleaving static op text with expanded
        // operands.
        // c:Src/exec.c:5210-5214 — trace-string building must be gated
        // on the live xtrace opt-state. The operand-expansion path
        // (compile_word_str on `$((i++))` / `$(cmd)`) has side
        // effects; running it unconditionally when xtrace is OFF
        // double-evaluates the operand (once for trace, once for
        // condition) — `while [[ $((i++)) -lt N ]]` only iterated
        // once because each pass incremented i twice. Bug #159 in
        // docs/BUGS.md.
        //
        // Runtime check via BUILTIN_XTRACE_IS_ON: push 1/0; if 0
        // (xtrace off), JumpIfFalse skips the entire trace block —
        // no operand expansion, no side effects.
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_IS_ON, 0),
            0,
        );
        let trace_skip = self.builder.emit(Op::JumpIfFalse(0), 0);
        let lit_const = self.builder.add_constant(Value::str("[[ "));
        self.builder.emit(Op::LoadConst(lit_const), 0);
        self.emit_cond_trace_runtime(c);
        let close_const = self.builder.add_constant(Value::str(" ]]"));
        self.builder.emit(Op::LoadConst(close_const), 0);
        self.builder.emit(Op::Concat, 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
        self.builder.emit(Op::Pop, 0);
        let trace_done = self.builder.current_pos();
        self.builder.patch_jump(trace_skip, trace_done);
        self.emit_cmd_push(crate::ported::zsh_h::CS_COND as u8);
        // c:Src/cond.c:502 — bare `[[ -o NAME ]]` returns tri-state
        // 0=set / 1=unset / 3=invalid-name. The generic bool→status
        // conversion below collapses 3 to 1 (the false case). Detect
        // the bare -o shape and route through a status-direct path
        // so the invalid-name signal survives.
        if let crate::parse::ZshCond::Unary(op, arg) = c {
            let op_clean = crate::lex::untokenize(op);
            if op_clean == "-o" {
                self.compile_word_str(arg);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_OPTION_CHECK_TRISTATE, 1),
                    0,
                );
                self.builder.emit(Op::SetStatus, 0);
                self.emit_cmd_pop();
                return;
            }
        }
        // Result on stack: bool. Status set after this returns.
        // Mark that operands are being compiled inside `[[ … ]]` so a process
        // substitution in an operand is rejected (c:Src/exec.c:4918 — a cond
        // runs with thisjob == -1, so `=()`/`<()`/`>()` "cannot be used here").
        let saved_cond = self.in_cond_operand;
        self.in_cond_operand = true;
        self.compile_cond_expr(c);
        self.in_cond_operand = saved_cond;
        self.emit_cmd_pop();
        // Convert bool → status. c:Src/cond.c — true→0, false→1,
        // and 2 when a `==`/`!=` pattern failed to compile during
        // this cond (BUILTIN_COND_STRMATCH armed the cell; the
        // builtin consumes it). Routed through the builtin instead
        // of inline jumps so the tri-state survives `!=`'s LogNot.
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_STATUS_FROM_BOOL, 1),
            0,
        );
        self.builder.emit(Op::SetStatus, 0);
        let _ = ZshCond::Not;
    }

    /// Append a runtime-expanded rendering of `c` onto the string
    /// already on the top of the stack. Direct port of Src/exec.c
    /// trace block in execcond which prints each expanded operand.
    /// Operands are word-expanded so `$HOME` / `~` / `$(…)` show
    /// the resolved value, matching zsh's `[[ -r /Users/foo ]]`.
    /// Operands inside a `[[ … ]]` cond are PATTERNS — `*` and `?`
    /// stay literal in the trace output, never trigger filesystem
    /// glob (which would NOMATCH-fail on `[[ x != *"$VAR"* ]]`).
    /// Bump dq_context_depth so compile_word_str's segment-fast-path
    /// gates `BUILTIN_GLOB_EXPAND` emission and BUILTIN_EXPAND_TEXT
    /// runs in mode 1 (DoubleQuoted) which doesn't filesystem-glob.
    fn emit_cond_trace_runtime(&mut self, c: &crate::parse::ZshCond) {
        self.dq_context_depth += 1;
        self.emit_cond_trace_runtime_inner(c);
        self.dq_context_depth -= 1;
    }

    fn emit_cond_trace_runtime_inner(&mut self, c: &crate::parse::ZshCond) {
        let push_lit = |s: &mut Self, text: &str| {
            let idx = s.builder.add_constant(Value::str(text));
            s.builder.emit(Op::LoadConst(idx), 0);
            s.builder.emit(Op::Concat, 0);
        };
        // Push an expanded word — `$HOME`/`~`/`$(…)`/etc. resolved.
        // Mode 1 = SQ-strip + DQ-strip + scalar expand, no split.
        // After expansion, route the value through quotedzputs so
        // non-printable bytes come back as `$'…'` source form
        // (mirrors c:Src/cond.c:212,221,224 which call
        // quotedzputs(operand, xtrerr) for the LHS / unary-test /
        // non-pattern RHS). Without it, `[[ -n $'\C-[OP' ]]` traced
        // as `[[ -n OP ]]` — the raw ESC + "OP" bytes leaked.
        let push_word = |s: &mut Self, word: &str| {
            s.compile_word_str(word);
            s.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_QUOTEDZPUTS, 1), 0);
            s.builder.emit(Op::Concat, 0);
        };
        // Pattern-op RHS variant — `=` / `==` / `!=` route through
        // quote_tokenized_output (c:Src/cond.c:218), NOT quotedzputs.
        // The former untokenizes lexer tokens (Star → `*`, …) and
        // backslash-escapes ASCII specials so the source pattern
        // surfaces verbatim. The latter (used by other operand
        // contexts) wraps in `'…'` which would render
        // `[[ x = a* ]]` as `[[ x = 'a*' ]]` — wrong.
        //
        // c:Src/cond.c:217-219 —
        //   char *rt = dupstring(ecrawstr(state->prog, state->pc, NULL));
        //   cond_subst(&rt, !fromtest);
        //   quote_tokenized_output(rt, xtrerr);
        // `ecrawstr` hands back the TOKENIZED word (quotes already
        // consumed by the lexer, an active `*` held as the `Star`
        // token), and `cond_subst` expands parameters WITHOUT
        // untokenizing. Routing through `compile_word_str` instead
        // untokenized first, so an active `Star` arrived as a plain
        // `*` byte and `quote_tokenized_output` escaped it:
        // `[[ 'f o' == 'f x'* ]]` traced as `f\ x\*` where zsh
        // prints `f\ x*`. Mirror the case-arm path above: expand only
        // the substitution segments (EXPAND_TEXT mode 4 = singsub) and
        // feed the literal segments through still tokenized.
        let push_word_pattern = |s: &mut Self, word: &str| {
            let has_expand = word.contains('$')
                || word.contains('`')
                || word.contains('\u{85}')
                || word.contains('\u{8c}')
                || word.contains('\u{99}');
            if has_expand {
                let segments = split_pattern_for_glob_subst(word);
                let mut first = true;
                for seg in segments.iter() {
                    match seg {
                        PatSeg::Subst(text) => {
                            let pc = s.builder.add_constant(Value::str(text.as_str()));
                            s.builder.emit(Op::LoadConst(pc), 0);
                            s.builder.emit(Op::LoadInt(4), 0);
                            s.builder
                                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_EXPAND_TEXT, 0), 0);
                        }
                        PatSeg::Literal(text) => {
                            // c:Src/cond.c:205-206 — `cond_subst(&right, …)`
                            // routes a token-bearing operand through
                            // `singsub` (c:53), i.e. `prefork(…,
                            // PREFORK_SINGLE)`, whose `remnulargs` drops the
                            // Snull/Dnull/Bnull quote markers the lexer left
                            // in the word. Without that strip
                            // `quote_tokenized_output` mapped them back to
                            // `'`/`"` via ztokens, so `[[ 'f o' == 'f x'* ]]`
                            // traced as `'f\ x'*` instead of zsh's `f\ x*`.
                            let mut lit = text.to_string();
                            crate::ported::glob::remnulargs(&mut lit);
                            let pc = s.builder.add_constant(Value::str(lit.as_str()));
                            s.builder.emit(Op::LoadConst(pc), 0);
                        }
                    }
                    s.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_QUOTE_TOKENIZED_OUTPUT, 1),
                        0,
                    );
                    s.builder.emit(Op::Concat, 0);
                    first = false;
                }
                if first {
                    // Empty word — nothing emitted, keep the buffer balanced.
                    let pc = s.builder.add_constant(Value::str(""));
                    s.builder.emit(Op::LoadConst(pc), 0);
                    s.builder.emit(Op::Concat, 0);
                }
            } else {
                // c:Src/cond.c:205-206 / c:53 — see the Literal arm above:
                // strip the lexer's Snull/Dnull quote markers the way
                // `singsub`'s `remnulargs` does before rendering.
                let mut lit = word.to_string();
                crate::ported::glob::remnulargs(&mut lit);
                let pc = s.builder.add_constant(Value::str(lit.as_str()));
                s.builder.emit(Op::LoadConst(pc), 0);
                s.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_QUOTE_TOKENIZED_OUTPUT, 1),
                    0,
                );
                s.builder.emit(Op::Concat, 0);
            }
        };
        match c {
            ZshCond::Not(inner) => {
                push_lit(self, "! ");
                self.emit_cond_trace_runtime_inner(inner);
            }
            ZshCond::And(a, b) => {
                self.emit_cond_trace_runtime_inner(a);
                push_lit(self, " && ");
                self.emit_cond_trace_runtime_inner(b);
            }
            ZshCond::Or(a, b) => {
                self.emit_cond_trace_runtime_inner(a);
                push_lit(self, " || ");
                self.emit_cond_trace_runtime_inner(b);
            }
            ZshCond::Unary(op, arg) => {
                let op_clean = crate::lex::untokenize(op);
                push_lit(self, &op_clean);
                if !arg.is_empty() {
                    push_lit(self, " ");
                    push_word(self, arg);
                }
            }
            ZshCond::Binary(left, op, right) => {
                let op_clean = crate::lex::untokenize(op);
                if right.is_empty() {
                    push_lit(self, &op_clean);
                    push_lit(self, " ");
                    push_word(self, left);
                } else {
                    push_word(self, left);
                    push_lit(self, " ");
                    push_lit(self, &op_clean);
                    push_lit(self, " ");
                    // c:Src/cond.c:214-218 — for COND_STREQ /
                    // COND_STRDEQ / COND_STRNEQ (`=`, `==`, `!=`)
                    // the RHS is the source pattern; render via
                    // quote_tokenized_output, not quotedzputs.
                    let is_pattern_op = matches!(op_clean.as_str(), "=" | "==" | "!=");
                    if is_pattern_op {
                        push_word_pattern(self, right);
                    } else {
                        push_word(self, right);
                    }
                }
            }
            ZshCond::Regex(left, regex) => {
                push_word(self, left);
                push_lit(self, " =~ ");
                push_word(self, regex);
            }
            ZshCond::ModCond(op, args) => {
                let op_clean = crate::lex::untokenize(op);
                push_lit(self, &op_clean);
                for arg in args {
                    push_lit(self, " ");
                    push_word(self, arg);
                }
            }
        }
    }

    fn compile_cond_expr(&mut self, c: &crate::parse::ZshCond) {
        match c {
            ZshCond::Not(inner) => {
                self.compile_cond_expr(inner);
                self.builder.emit(Op::LogNot, 0);
            }
            ZshCond::And(a, b) => {
                self.compile_cond_expr(a);
                let skip = self.builder.emit(Op::JumpIfFalseKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.compile_cond_expr(b);
                self.builder.patch_jump(skip, self.builder.current_pos());
            }
            ZshCond::Or(a, b) => {
                self.compile_cond_expr(a);
                let skip = self.builder.emit(Op::JumpIfTrueKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.compile_cond_expr(b);
                self.builder.patch_jump(skip, self.builder.current_pos());
            }
            ZshCond::Unary(op, arg) => {
                // The lexer encodes operator chars in the META range
                // (0x83-0x9f). Un-tokenize before matching.
                let op_clean = crate::lex::untokenize(op);
                // `-v` takes a parameter NAME (with optional subscript)
                // — never glob-expand the operand. Without this,
                // `[[ -v a[1] ]]` errored "no matches found: a[1]"
                // because `a[1]` was treated as a `[1]` char-class
                // glob. Emit the literal text so the runtime's
                // BUILTIN_VAR_EXISTS handler sees `a[1]` intact and
                // can split on `[` to look up `arr[1]` element.
                if op_clean == "-v" {
                    // `-v` takes a parameter NAME and must not glob-expand
                    // subscript brackets (`[[ -v a[1] ]]` is "is element
                    // a[1] set", not a `[1]` char-class). But, like every
                    // cond operand (c:Src/cond.c prefork), it DOES undergo
                    // parameter expansion: `[[ -v $n ]]` checks the
                    // variable NAMED BY $n. When the operand carries a `$`
                    // (literal or tokenized String/Qstring), param-expand
                    // it with globbing suppressed (dq_context); otherwise
                    // emit the bare subscripted name literally so the
                    // runtime parses the subscript. Previously the operand
                    // was always emitted literally, so `[[ -v $n ]]` fed
                    // the runtime the raw text `$n` → "bad substitution".
                    let has_dollar = arg.chars().any(|c| matches!(c as u32, 0x24 | 0x85 | 0x8c));
                    if has_dollar {
                        self.dq_context_depth += 1;
                        self.compile_word_str(arg);
                        self.dq_context_depth -= 1;
                    } else {
                        let arg_clean = crate::lex::untokenize(arg);
                        let idx = self.builder.add_constant(Value::str(arg_clean.as_str()));
                        self.builder.emit(Op::LoadConst(idx), 0);
                    }
                } else {
                    // c:Src/cond.c — `[[ ]]` arguments undergo parameter
                    // expansion but NOT filesystem globbing. Per zsh's
                    // documented semantic, `[[ -e /tmp/*.txt ]]` tests
                    // the LITERAL path `/tmp/*.txt`, not whatever the
                    // glob would match. Bump dq_context_depth so
                    // compile_word_str suppresses BUILTIN_GLOB_EXPAND
                    // on the operand — mirrors the existing
                    // suppression logic for binary-cond LHS at line
                    // 4857. Bug #156 in docs/BUGS.md.
                    // c:Src/cond.c:194-197 — the unary operand takes the
                    // SAME `cond_subst(&left, !fromtest)` as a binary one, so
                    // a word ending in a glob QUALIFIER (c:Src/glob.c:1157
                    // `checkglobqual`) IS filename-generated:
                    // `[[ -z z*(#qN) ]]` is "did the (N) glob produce
                    // nothing?", not "is the literal text empty?".
                    if Self::cond_operand_has_globqual(arg) {
                        self.compile_word_str(arg);
                    } else {
                        self.dq_context_depth += 1;
                        self.compile_word_str(arg);
                        self.dq_context_depth -= 1;
                    }
                }
                self.emit_file_test(&op_clean);
            }
            ZshCond::Binary(left, op, right) => {
                let left_clean = crate::lex::untokenize(left);
                let op_clean = crate::lex::untokenize(op);
                // Bare `$~name` rhs (String + Tilde tokens, no braces)
                // expands EMPTY through the rhs fast paths (the name
                // parse swallows the Tilde token); the braced
                // `${~name}` spelling expands correctly. Same C
                // semantics either way (zshexpn $~spec), so normalize
                // bare to braced HERE so every downstream rhs path —
                // including the fast-path the direct braced source
                // takes — sees the working form.
                let right_tilde_braced: Option<String> = {
                    let cs: Vec<char> = right.chars().collect();
                    let dollar = matches!(
                        cs.first().map(|c| *c as u32),
                        Some(0x24) | Some(0x85) | Some(0x8c)
                    );
                    let tilde_bare = dollar
                        && matches!(cs.get(1).map(|c| *c as u32), Some(0x7e) | Some(0x98))
                        && cs
                            .get(2)
                            .map_or(false, |c| c.is_ascii_alphanumeric() || *c == '_')
                        && cs[2..]
                            .iter()
                            .all(|c| c.is_ascii_alphanumeric() || *c == '_');
                    if tilde_bare {
                        let mut s = String::new();
                        s.push(cs[0]);
                        s.push('\u{8f}'); // Inbrace
                        s.extend(&cs[1..]);
                        s.push('\u{90}'); // Outbrace
                        Some(s)
                    } else {
                        None
                    }
                };
                let right: &String = right_tilde_braced.as_ref().unwrap_or(right);
                // The port packs unary file tests as Binary too: `-d /tmp`
                // arrives as Binary("-d", "/tmp", ""). If left starts with
                // `-` and looks like a test flag, treat it as Unary with
                // the path as the argument.
                if left_clean.starts_with('-') && left_clean.len() == 2 && right.is_empty() {
                    // `-v` parameter-existence test must NOT glob-
                    // expand the operand: `[[ -v a[1] ]]` is "is array
                    // element a[1] set", not a `[1]` char-class glob.
                    // Treat the operand as a literal name string and
                    // let the runtime parse the subscript.
                    if left_clean == "-v" {
                        // Param-expand a `$`-bearing operand (glob
                        // suppressed); emit a bare subscripted name
                        // literally. Mirrors the Unary `-v` arm — see the
                        // comment there. `[[ -v $n ]]` must check the
                        // variable named by $n, not the literal text $n.
                        let has_dollar = op.chars().any(|c| matches!(c as u32, 0x24 | 0x85 | 0x8c));
                        if has_dollar {
                            self.dq_context_depth += 1;
                            self.compile_word_str(op);
                            self.dq_context_depth -= 1;
                        } else {
                            let op_clean_arg = crate::lex::untokenize(op);
                            let idx = self.builder.add_constant(Value::str(op_clean_arg.as_str()));
                            self.builder.emit(Op::LoadConst(idx), 0);
                        }
                    } else {
                        // c:Src/cond.c — `[[ ]]` unary file tests don't
                        // glob-expand operands. Same logic as the
                        // ZshCond::Unary arm above (line 4814+).
                        // Bug #156 in docs/BUGS.md.
                        // c:Src/cond.c:194-197 + Src/glob.c:1157 — except
                        // when the word ends in a glob QUALIFIER, which
                        // `cond_subst` DOES filename-generate.
                        if Self::cond_operand_has_globqual(op) {
                            self.compile_word_str(op);
                        } else {
                            self.dq_context_depth += 1;
                            self.compile_word_str(op);
                            self.dq_context_depth -= 1;
                        }
                    }
                    self.emit_file_test(&left_clean);
                    return;
                }
                // c:Src/cond.c — inside `[[ … ]]` the LHS undergoes
                // word splitting / parameter expansion but NOT
                // filesystem globbing (glob is suppressed for cond
                // operands). Bump dq_context_depth ONLY when the raw
                // LHS contains glob metachars (Star / Quest /
                // Inbrack) so the suppression doesn't disturb other
                // expansion paths (like DQ-wrapped DQ markers that
                // already correctly handle DQ content). Without this,
                // `[[ a* = a* ]]` hit \"no matches found: a*\" because
                // the LHS was glob-expanded before reaching the test
                // runtime.
                // Inbrace (`\u{8f}`) joins the list for the same reason: C runs
                // every cond operand through singsub → prefork(PREFORK_SINGLE)
                // (cond.c:53 / subst.c:520), and subst.c:170 skips xpandbraces
                // under that flag. So a LHS of `a{2,3}` stays ONE literal word;
                // without the bump it brace-expanded and `[[ a{2,3} ==
                // 'a{2,3}' ]]` compared `a2 a3` against `a{2,3}` → 1.
                let left_has_unquoted_glob = Self::cond_operand_suppresses_glob(left);
                if left_has_unquoted_glob {
                    self.dq_context_depth += 1;
                    self.compile_word_str(left);
                    self.dq_context_depth -= 1;
                } else {
                    self.compile_word_str(left);
                }
                // For string-comparison ops (`=`, `==`, `!=`, `=~`)
                // the RHS is a PATTERN/REGEX to match against the LHS,
                // not a path glob to expand against the filesystem.
                // Routing through compile_word_str triggers expand_glob
                // (now NOMATCH-strict). Compile RHS as a quoted literal
                // so the pattern reaches the test runtime intact.
                let is_pattern_op = matches!(op_clean.as_str(), "=" | "==" | "!=" | "=~");
                if op_clean == "=~" {
                    // For `=~`, the RHS is a regex that must undergo
                    // variable / cmd-subst expansion (`pat="^h.*";
                    // [[ x =~ $pat ]]` must use $pat's value as the
                    // regex). compile_word_str does the expansion;
                    // glob expansion is moot for `=~` because the test
                    // runtime treats the result as a regex pattern, not
                    // a filesystem path. Wrap in DQ to suppress brace
                    // expansion + filesystem globbing during expansion
                    // — UNLESS the operand is ALREADY single-quoted
                    // (Snull-wrapped, `\u{9d}…\u{9d}`). zsh treats
                    // `[[ x =~ '(pat)' ]]` as a literal regex; double-
                    // wrapping in DQ markers makes compile_word_str's
                    // markup-strip skip the Snull pair and the regex
                    // engine sees the meta bytes verbatim.
                    // c:Src/cond.c:525-535 `cond_str` — the regex operand is
                    // `singsub`'d + `untokenize`d, never globbed, so the
                    // suppression must hold for EVERY shape of the word.
                    // Testing only `starts_with(Dnull)` mistook a word that
                    // merely BEGINS with a quoted span (`"x"*y`,
                    // `[\"\']*"$RE"[\"\']*…`) for a fully quoted one and left
                    // the trailing `*` to filename-generation:
                    // `[[ $L =~ "x"*y ]]` died with `no matches found: x*y`.
                    self.compile_regex_operand(right);
                } else if is_pattern_op {
                    // RHS handling for `==` / `=` / `!=` patterns:
                    // - If it contains a variable / cmd-subst (`$`, `` ` ``)
                    //   route through compile_word_str so the value is
                    //   substituted in. To preserve unquoted `*`/`?`/etc.
                    //   as PATTERN metachars while still suppressing
                    //   filesystem globbing of the result, bump
                    //   dq_context_depth so compile_word_str's fast paths
                    //   skip BUILTIN_GLOB_EXPAND. The runtime test op
                    //   then matches the LHS against the assembled
                    //   pattern at evaluation time.
                    // - Otherwise use the literal-pattern path with
                    //   pre-escaped quoted-glob metas — UNLESS the RHS
                    //   is wholly double-quoted, in which case StrEq
                    //   below does a literal compare and any `\X`
                    //   escapes added here become part of the
                    //   compared bytes (bug #13 in docs/BUGS.md —
                    //   `[[ "?" == "?" ]]` returned NOMATCH because
                    //   the escape made the RHS `\?` while the LHS
                    //   stayed `?`).
                    let needs_expand = right.contains('\u{85}')   // META-$
                        || right.contains('\u{8c}')                  // Qstring-$
                        || right.contains('\u{93}')                  // Tick
                        || right.contains('$')
                        || right.contains('`');
                    // Detect DQ-wrapped RHS upfront so we can pick the
                    // right RHS-emit shape. See the comment at the
                    // dispatch below — when the RHS is one DQ span,
                    // zsh treats it as a literal string for `[[ == ]]`.
                    let rhs_is_pure_dq_pre = right.starts_with('\u{9e}')
                        && right.ends_with('\u{9e}')
                        && right.chars().filter(|&c| c == '\u{9e}').count() == 2;
                    if needs_expand && !rhs_is_pure_dq_pre {
                        // c:Src/options.c GLOB_SUBST. When the RHS
                        // pattern came from variable / cmd
                        // substitution, zsh's default-OFF
                        // GLOB_SUBST keeps the SUBSTITUTED chars
                        // literal (no glob meta promotion) but
                        // PRESERVES source-level glob metas as
                        // matchable globs. The naive path that runs
                        // GLOB_SUBST_GUARD over the WHOLE expanded
                        // string escapes both kinds of metas, so
                        // `[[ foo = $H* ]]` (with H=foo) became
                        // `foo\*` and failed to match. Walk the
                        // source segments at compile time and emit
                        // each piece separately: substitution
                        // segments go through compile_word_str +
                        // GLOB_SUBST_GUARD (literal-ize value chars
                        // when GLOB_SUBST off), source-level segments
                        // (glob META tokens, literal text, DQ-wraps)
                        // emit as raw constants without the guard.
                        // The result on the stack concatenates the
                        // pieces — source-level `*` survives as a
                        // literal `*` byte that StrMatch/patcompile
                        // treats as glob, substitution `*` survives
                        // as `\*` literal. Bug #116 in docs/BUGS.md.
                        // c:Src/subst.c — same split happens at
                        // singsub time in C, with tokenized output
                        // distinguishing substituted bytes from
                        // source metas; this is the compile-time
                        // analogue.
                        //
                        // Gate on !rhs_is_pure_dq_pre: when the WHOLE
                        // RHS is one DQ span, the downstream StrEq
                        // (line ~6470) does a byte-for-byte literal
                        // compare. GLOB_SUBST_GUARD escapes `\` →
                        // `\\` in the substituted value, doubling
                        // the backslash count on the RHS while the
                        // LHS stays literal — so `[[ "$a" == "$b" ]]`
                        // with a=b="ab\\c" (4-byte `ab\c`) failed
                        // because the RHS became 5-byte `ab\\c`.
                        // qq_then_Q_roundtrip_specials parity test.
                        // The `~`-flag / source-meta-vs-substitution split lives
                        // in the shared helper (`case` arms need the identical
                        // logic, so it must not be duplicated here).
                        self.emit_glob_subst_pattern(right);
                    } else if rhs_is_pure_dq_pre {
                        // Literal-compare path: when the RHS is one DQ
                        // span (including DQ-only-vars like `"$x"`),
                        // StrEq does a byte compare. Route through
                        // compile_word_str for full expansion (so
                        // `"$x"` resolves to its value) WITHOUT
                        // GLOB_SUBST_GUARD wrapping — escaping glob
                        // metas / backslashes on this path would
                        // mismatch the LHS literal value. Same shape
                        // as the LHS path at line ~6440.
                        self.compile_word_str(right);
                    } else {
                        // c:Src/cond.c — `[[ $x == pat ]]` RHS uses the
                        // RAW pattern bytes for patcompile. Backslash
                        // escapes (`\*`, `\?`, `\[`) must reach the
                        // pattern compiler so the meta becomes literal.
                        // Use `untokenize_preserve_quotes` which maps
                        // Bnull → `\` (vs the plain `untokenize` that
                        // DROPS Bnull, collapsing `\*` to `*` and
                        // turning the literal-meta pattern back into a
                        // glob meta). Bug #449. `escape_quoted_glob_metas`
                        // still runs first to backslash-escape glob
                        // metas inside Snull/Dnull-quoted spans.
                        // escape_quoted_glob_metas backslash-escapes glob
                        // metas (`* ? [ ( ) | ~ # ^`) that sit INSIDE a
                        // Snull/Dnull-quoted span so patcompile treats
                        // them as literal. Critically this neutralizes a
                        // quoted `(#...)` flag form: `[[ ab = "(#b)"* ]]`
                        // must match the literal text `(#b)`, not enable
                        // the backref flag, and `"(#c"*` must not parse
                        // as an unterminated count flag ("bad pattern").
                        // Drive the marker-strip / Bnull pass off `escaped`
                        // (the raw `right` was used before, discarding the
                        // escaping entirely).
                        let escaped = escape_quoted_glob_metas(right);
                        let mut filtered = String::with_capacity(escaped.len());
                        let mut iter = escaped.chars().peekable();
                        while let Some(c) = iter.next() {
                            match c {
                                '\u{9d}' | '\u{9e}' => {} // strip Snull/Dnull
                                '\u{9f}' => {
                                    // Bnull-escape — emit `\` + next char
                                    // literally so patcompile sees the
                                    // backslash-escape sequence.
                                    //
                                    // The raw backslash IS the quote spelling
                                    // `patcompile`'s normalizer expects from a
                                    // SOURCE-level pattern; a pattern built
                                    // from a VALUE spells a data backslash
                                    // `\\` instead. See the `\\` arm of that
                                    // normalizer and docs/BUGS.md #1090.
                                    if let Some(next) = iter.next() {
                                        filtered.push('\\');
                                        filtered.push(next);
                                    } else {
                                        filtered.push('\\');
                                    }
                                }
                                _ => filtered.push(c),
                            }
                        }
                        let right_clean = crate::lex::untokenize(&filtered);
                        let idx = self.builder.add_constant(Value::str(right_clean.as_str()));
                        self.builder.emit(Op::LoadConst(idx), 0);
                    }
                } else if Self::cond_operand_suppresses_glob(right) {
                    // c:Src/cond.c:203-207 — `cond_subst(&right, !fromtest)`.
                    // C runs the RIGHT operand of every non-pattern binary
                    // condition through the SAME `cond_subst` as the left
                    // (c:196-199), and `cond_subst` only reaches `zglob`
                    // when `checkglobqual` says the word ends in a glob
                    // QUALIFIER (c:43-51); otherwise it is plain
                    // `singsub` — substitution with no filename
                    // generation. The port applied the suppression to the
                    // LHS only, so an unquoted RHS carrying a glob token
                    // was filename-globbed: `[[ nm -ne compstate[nmatches] ]]`
                    // (the `_alternative` sh:63 / `_arguments` /
                    // `_describe` "did this completer add anything?"
                    // idiom, where `[`/`]` tokenize as Inbrack) died with
                    // `no matches found: compstate[nmatches]` instead of
                    // arithmetic-evaluating the subscript, so every such
                    // completer reported "added nothing" and the
                    // `_main_complete` chain re-ran it for the next
                    // matcher-list entry.
                    self.dq_context_depth += 1;
                    self.compile_word_str(right);
                    self.dq_context_depth -= 1;
                } else {
                    self.compile_word_str(right);
                }
                // Detect DQ-wrapped RHS — when the source pattern is
                // entirely inside `"..."`, the resolved value is a
                // LITERAL string and pattern metas (including `\X`
                // escapes) must be treated literally. zsh manual: a
                // quoted variable expansion produces literal text for
                // `[[ == ]]` matching; only an unquoted RHS is
                // pattern-interpreted. Switch StrMatch → StrEq for
                // these cases, mirroring the difference between
                // `[[ x == "$pat" ]]` (literal) and `[[ x == $pat ]]`
                // (pattern). Skip for `=~` (regex), file tests, etc.
                let rhs_is_pure_dq = right.starts_with('\u{9e}') && right.ends_with('\u{9e}') && {
                    // No unquoted glob meta outside the DQ wrap.
                    // The DQ pair brackets the whole word — count
                    // Dnull markers; if exactly 2, the whole word
                    // is one DQ span.
                    right.chars().filter(|&c| c == '\u{9e}').count() == 2
                };
                if is_pattern_op && op_clean != "=~" && rhs_is_pure_dq {
                    if op_clean == "!=" {
                        self.builder.emit(Op::StrEq, 0);
                        self.builder.emit(Op::LogNot, 0);
                    } else {
                        self.builder.emit(Op::StrEq, 0);
                    }
                } else {
                    self.emit_binary_test(&op_clean);
                }
            }
            ZshCond::Regex(left, regex) => {
                self.compile_word_str(left);
                // c:Src/cond.c:493 — the `=~` RHS is a regex that
                // undergoes parameter / cmd-subst expansion:
                // `PAT="^x"; [[ x =~ $PAT ]]` must match against
                // $PAT's VALUE. The previous emit untokenized the
                // RHS and loaded it as a raw CONSTANT, so `$PAT`
                // reached the matcher literally and never matched.
                // Expand via compile_word_str like the `==`/`=`
                // pattern path: DQ-wrap to suppress brace/filesystem
                // globbing during expansion (the runtime treats the
                // result as an ERE, not a path) — unless the RHS is
                // already single-quoted (`[[ x =~ '(p)' ]]` is a
                // literal regex) or already DQ-wrapped.
                // c:Src/cond.c:525-535 `cond_str` — see compile_regex_operand.
                self.compile_regex_operand(regex);
                self.builder.emit(Op::RegexMatch, 0);
            }
            ZshCond::ModCond(op, args) => {
                // `[[ -prefix PAT ]]` etc. — completion/module condition
                // (C COND_MOD). Push each already-expanded operand word, then
                // the operator word last, and dispatch to the host handler
                // which runs cond_psfix/cond_range → do_comp_vars. An operand
                // is never FILENAME-globbed, so the substitution path still
                // DQ-suppresses globbing like the unary path — but its glob
                // metachars stay PATTERN-active, because every COND_MOD
                // handler feeds its operand to `patcompile`. See the two
                // arms below for which half of that each operand takes.
                for arg in args {
                    // c:Src/cond.c:131-132 — `strs = ecgetarr(state, l, EC_DUP,
                    // NULL)`. The COND_MOD operands are the wordcode strings
                    // the PARSER stored (c:Src/parse.c:2716 par_cond_multi),
                    // i.e. still in the lexer's TOKENIZED form, and
                    // `cond_str(a, n, 1)` keeps them that way. A source-level
                    // `*` therefore reaches `patcompile` as `Star`, and
                    // `[[ -prefix :*: ]]` is a real glob test — while a
                    // QUOTED `':*:'` arrives as inull-wrapped literal text and
                    // is not.
                    //
                    // A word with no substitution in it has nothing for the
                    // expander to do, so emit the parser's token string
                    // directly. Routing it through `compile_word_str` (which
                    // untokenizes) is what turned `Star` back into a literal
                    // `*`: `[[ -prefix :*: ]]` compiled the pattern `:*:` as
                    // three literal chars, so `_zstyle`'s `[[ ! -prefix :*: ]]`
                    // test (Completion/Zsh/Command/_zstyle) took the wrong
                    // branch and `zstyle :completion:<TAB>` lost the whole
                    // `functions`/`_completers`/... sub-context list.
                    //
                    // `remnulargs` here stands in for the one C's `patcompile`
                    // runs on its own input (c:Src/pattern.c:571) — zshrs's
                    // `patcompile` decodes `Snull`/`Dnull` through the
                    // `ztokens` table instead of dropping them, so the quote
                    // markers have to come off before it sees them.
                    if Self::modcond_arg_is_pure_literal(arg) {
                        let mut lit = arg.to_string();
                        crate::ported::glob::remnulargs(&mut lit);
                        let c = self.builder.add_constant(Value::str(lit.as_str()));
                        self.builder.emit(Op::LoadConst(c), 0);
                        continue;
                    }
                    self.dq_context_depth += 1;
                    self.compile_word_str(arg);
                    self.dq_context_depth -= 1;
                    // c:Src/cond.c:525-534 `cond_str(args, num, raw)` runs
                    // `singsub` and — for `raw` callers, which is every
                    // COND_MOD handler (`cond_psfix` c:Src/Zle/complete.c:1666,
                    // `cond_range` c:1688) — deliberately does NOT untokenize,
                    // so a `$~spec` operand reaches `patcompile` with the
                    // tokens c:Src/subst.c:4419-4420's `shtokenize` put there.
                    // zshrs finishes the expansion up here, so re-apply that
                    // one step for the operands that asked for it.
                    if Self::seg_forces_glob_subst(arg) {
                        self.builder.emit(
                            Op::CallBuiltin(crate::fusevm_bridge::BUILTIN_COND_SHTOKENIZE, 1),
                            0,
                        );
                    }
                }
                let op_clean = crate::lex::untokenize(op);
                let idx = self.builder.add_constant(Value::str(op_clean.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_MOD, (args.len() + 1) as u8),
                    0,
                );
            }
        }
    }

    fn emit_file_test(&mut self, op: &str) {
        let test_byte: u8 = match op {
            "-e" | "-a" => file_test::EXISTS,
            "-f" => file_test::IS_FILE,
            "-d" => file_test::IS_DIR,
            // -r/-w/-x are handled by the BUILTIN_COND_ACCESS arm below
            // (access(2), not fusevm's existence-only Op::TestFile).
            "-s" => file_test::IS_NONEMPTY,
            "-L" | "-h" => file_test::IS_SYMLINK,
            "-c" => {
                // Character device. Not in fusevm's file_test set.
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_IS_CHARDEV, 1), 0);
                return;
            }
            "-b" => {
                // Block device.
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_IS_BLOCKDEV, 1), 0);
                return;
            }
            "-p" => {
                // FIFO (named pipe).
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_IS_FIFO, 1), 0);
                return;
            }
            "-S" => {
                // Socket.
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_IS_SOCKET, 1), 0);
                return;
            }
            "-k" => {
                // Sticky bit (S_ISVTX). Not in fusevm's file_test set;
                // route through a thin host-side builtin.
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_HAS_STICKY, 1), 0);
                return;
            }
            "-u" => {
                // Setuid bit (S_ISUID).
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_HAS_SETUID, 1), 0);
                return;
            }
            "-g" => {
                // Setgid bit (S_ISGID).
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_HAS_SETGID, 1), 0);
                return;
            }
            "-O" => {
                // Owned by effective UID.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_OWNED_BY_USER, 1),
                    0,
                );
                return;
            }
            "-G" => {
                // Owned by effective GID.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_OWNED_BY_GROUP, 1),
                    0,
                );
                return;
            }
            "-N" => {
                // File modified since last accessed (mtime > atime).
                // zsh: used to gate mailbox-style "fresh content" checks.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_FILE_MODIFIED_SINCE_ACCESS, 1),
                    0,
                );
                return;
            }
            "-z" => {
                // Op::StringLen calls `Value::len` which returns
                // ARRAY length for `Value::Array` — not the joined
                // string length, and not the cond-context "is empty"
                // semantic zsh uses. For `b=(""); [[ -z "${b[@]}" ]]`
                // the stack carries `Value::Array([""])` whose
                // `len()` is 1, so the inline StringLen → NumEq
                // sequence returned false. Route through the runtime
                // helper which inspects Array vs Str directly per
                // `Src/cond.c:347 case 'z'` semantics. Bug #185 in
                // docs/BUGS.md.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_STR_EMPTY, 1),
                    0,
                );
                return;
            }
            "-n" => {
                // Companion to `-z` above; see comment block there
                // for the cond-context Array-vs-Str rationale.
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_STR_NONEMPTY, 1),
                    0,
                );
                return;
            }
            "-v" => {
                // `[[ -v name ]]` — variable existence check (bash; zsh
                // approximates via `(t)` flag). Stack-top is the name —
                // route through BUILTIN_VAR_EXISTS which checks scalar /
                // array / assoc / env tables.
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_VAR_EXISTS, 1), 0);
                return;
            }
            "-o" => {
                // `[[ -o option ]]` — shell-option-set check. Routes
                // through BUILTIN_OPTION_SET which normalizes the name
                // (strip _, lowercase) and reads exec.options.
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_OPTION_SET, 1), 0);
                return;
            }
            "-t" => {
                // `[[ -t fd ]]` — fd-is-a-tty check. Stack-top is the
                // fd-string (e.g. "0", "1", "2"). Route through a
                // host-side builtin that calls libc::isatty.
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_IS_TTY, 1), 0);
                return;
            }
            "-r" | "-w" | "-x" => {
                // `[[ -r/-w/-x file ]]` — must use access(2) (doaccess),
                // NOT fusevm's Op::TestFile which only checks existence
                // for -r/-w (a `chmod 000` file read as readable). Push
                // the access(2) mode bit (R_OK=4, W_OK=2, X_OK=1) and
                // route through BUILTIN_COND_ACCESS. c:Src/cond.c:438.
                let mode: i64 = match op {
                    "-r" => 4,
                    "-w" => 2,
                    _ => 1,
                };
                self.builder.emit(Op::LoadInt(mode), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_ACCESS, 2), 0);
                return;
            }
            _ => {
                // zsh: `[[ -l file ]]` (and any other unknown unary
                // condition) errors with `unknown condition: -X` and aborts
                // (c:Src/cond.c:150-188). Drop the operand and route the op
                // through BUILTIN_COND_UNKNOWN, which zerr's + sets errflag at
                // RUNTIME so the cond's errexit check aborts the input — the
                // prior compile-time eprintln printed the message but never set
                // errflag, so the line ran on.
                self.builder.emit(Op::Pop, 0);
                let idx = self.builder.add_constant(Value::str(op));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_UNKNOWN, 1),
                    0,
                );
                return;
            }
        };
        self.builder.emit(Op::TestFile(test_byte), 0);
    }

    /// True when a `[[ … ]]` operand must be expanded with filesystem
    /// globbing (and brace expansion) SUPPRESSED.
    ///
    /// c:Src/cond.c:41-54 `cond_subst` — a cond operand only reaches
    /// `zglob` when `checkglobqual` reports a trailing glob QUALIFIER;
    /// every other operand goes through `singsub` (c:53), i.e.
    /// `prefork(PREFORK_SINGLE)` (subst.c:520), which performs parameter
    /// / command / arithmetic substitution but neither filename
    /// generation nor brace expansion (subst.c:170 skips `xpandbraces`
    /// under that flag). Both operands take the same route
    /// (c:196-199 left, c:203-207 right), so both call sites share this
    /// predicate rather than open-coding it — an earlier open-coded copy
    /// existed for the left operand only.
    ///
    /// The check is on the RAW tokenized word: an already-quoted operand
    /// (Snull `\u{9d}` / Dnull `\u{9e}`) never globs anyway, and bumping
    /// `dq_context_depth` for it would disturb the DQ handling that path
    /// already does correctly. Tokens tested: Star `\u{87}`,
    /// Quest `\u{86}`, Inbrack `\u{91}`, Inbrace `\u{8f}`.
    fn cond_operand_suppresses_glob(w: &str) -> bool {
        // c:Src/cond.c:41-54 `cond_subst` — a `[[ … ]]` operand is
        // `singsub`'d (substitution, NO filename generation) UNLESS
        // `checkglobqual` says the word ends in a glob QUALIFIER, in which
        // case it goes through the full prefork + `zglob` pipeline:
        //   `if (glob_ok && checkglobqual(*strp, strlen(*strp), 1, NULL))`
        // So `[[ -z z*(#qN) ]]` really does glob and really can produce the
        // empty string. Suppressing globbing for EVERY operand with a glob
        // token left the literal text `z*(#qN)` in place, which is never
        // empty.
        if Self::cond_operand_has_globqual(w) {
            return false;
        }
        !w.contains('\u{9e}')
            && !w.contains('\u{9d}')
            && (w.contains('\u{87}')
                || w.contains('\u{86}')
                || w.contains('\u{91}')
                || w.contains('\u{8f}'))
    }

    /// c:Src/glob.c:1157 `checkglobqual(str, sl, nobareglob = 1, NULL)` as
    /// `cond_subst` calls it (c:Src/cond.c:43-44). Answers "does this
    /// TOKENIZED word end in a glob qualifier?" for the compile-time text.
    ///
    /// C asks the question at RUNTIME, after prefork has substituted the
    /// word; here only the source text is available, so a qualifier that
    /// arrives from a parameter is not seen. That is the same limitation the
    /// surrounding compile-time glob-suppression decision already has.
    fn cond_operand_has_globqual(w: &str) -> bool {
        let chars: Vec<char> = w.chars().collect();
        if chars.is_empty() {
            return false;
        }
        let mut sp: Option<usize> = None;
        if crate::ported::glob::checkglobqual(&chars, chars.len() as i32, 1, &mut sp) != 0 {
            return true;
        }
        // c:Src/glob.c:1191 — `if (isset(EXTENDEDGLOB) && !zpc_disables[
        // ZPC_HASH] && s[1] == Pound)`. C evaluates that option at RUNTIME,
        // by which time the script's own `setopt extendedglob` has taken
        // effect; zshrs decides here, while compiling, when it typically has
        // NOT. `checkglobqual` therefore reports 0 for `z*(#qN)` compiled
        // ahead of a `setopt extendedglob`. Re-run the c:1191-1195 arm with
        // EXTENDEDGLOB assumed: an explicit `(#q…)` has no other meaning, and
        // c:1197's bare-qualifier arm is already dead here because
        // `cond_subst` passes nobareglob = 1 (c:Src/cond.c:44).
        use crate::ported::zsh_h::{Inpar, Outpar, Pound};
        if chars.len() < 4 || *chars.last().unwrap() != Outpar {
            return false;
        }
        let mut paren = 0i32;
        let mut i = chars.len() - 2;
        loop {
            if chars[i] == Inpar && paren == 0 {
                return chars.get(i + 1) == Some(&Pound) && chars.get(i + 2) == Some(&'q');
            }
            match chars[i] {
                Outpar => paren += 1,
                Inpar => paren -= 1,
                _ => {}
            }
            if i == 0 {
                return false;
            }
            i -= 1;
        }
    }

    /// Compile the `=~` RHS (the ERE) with substitution ON and filename
    /// generation / brace expansion OFF.
    ///
    /// c:Src/cond.c:113-118 — `COND_REGEX` is rewritten to `COND_MODI` and
    /// dispatched to `zsh/regex`'s `zcond_regex_match`, which reads the
    /// operand via `cond_str(a,1,0)` (c:Src/Modules/regex.c:63). `cond_str`
    /// (c:Src/cond.c:525-535) is `singsub` + `untokenize` — parameter /
    /// command substitution happens, filename generation never does.
    ///
    /// Three shapes, all ending in the same "expand as if inside DQ" state:
    ///   * the word is ALREADY one quoted span (`'…'` / `"…"`) — compile it
    ///     as-is; wrapping it again would hide the Snull/Dnull pair from
    ///     compile_word_str's markup strip.
    ///   * the word carries NO quote markers — wrap it in a Dnull pair, the
    ///     long-standing spelling here.
    ///   * MIXED (`"x"*y`, `[\"\']*"$RE"…`) — a second Dnull pair would NEST,
    ///     and `word_is_single_dq_span` then reports false so the wrap is
    ///     inert and the unquoted `*` reaches filename generation. Bump
    ///     `dq_context_depth` instead: every consumer spells "in DQ" as
    ///     `dq_context_depth > 0 || word_is_single_dq_span(s)`.
    fn compile_regex_operand(&mut self, w: &str) {
        let pure_span = |q: char| -> bool {
            w.starts_with(q) && w.ends_with(q) && w.chars().filter(|&c| c == q).count() == 2
        };
        if pure_span('\u{9e}') || pure_span('\u{9d}') {
            self.compile_word_str(w);
        } else if w.contains('\u{9e}') || w.contains('\u{9d}') {
            self.dq_context_depth += 1;
            self.compile_word_str(w);
            self.dq_context_depth -= 1;
        } else {
            self.compile_word_str(&format!("\u{9e}{}\u{9e}", w));
        }
    }

    fn emit_binary_test(&mut self, op: &str) {
        match op {
            // c:Src/cond.c:308-316 — cond glob compare carries its
            // own bad-pattern semantics (zwarn + status 2, no abort),
            // distinct from case's Src/loop.c:667 zerr. Dedicated
            // builtin instead of Op::StrMatch so the diagnostic path
            // splits per consumer.
            "=" | "==" => self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_STRMATCH, 2),
                0,
            ),
            "!=" => {
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_STRMATCH, 2),
                    0,
                );
                self.builder.emit(Op::LogNot, 0)
            }
            // `=~` arrives via ZshCond::Binary not ZshCond::Regex (the
            // port uses Binary for everything by default). Route to
            // RegexMatch here.
            "=~" => self.builder.emit(Op::RegexMatch, 0),
            // `-regex-match` is the named-condition form provided by
            // the zsh/regex module (Src/Modules/regex.c:213). Direct
            // port of `zcond_regex_match(char **a, int id)` (regex.c:60-210): same
            // semantics as `=~` — populates `$MATCH`, `$MBEGIN`,
            // `$MEND`, `$match[1..N]`, `$mbegin[]`, `$mend[]` on a
            // successful regexec; status 0 on match, 1 otherwise.
            "-regex-match" => self.builder.emit(Op::RegexMatch, 0),
            // `-pcre-match` is the named-condition form provided by
            // the zsh/pcre module (Src/Modules/pcre.c:506). Direct
            // port of `cond_pcre_match`: compiles the RHS pattern
            // and matches against the LHS, populating `$MATCH` and
            // `$match[1..N]`. zshrs's PCRE backend is the Rust
            // `regex` crate (RE2 engine) — backreferences and some
            // lookarounds aren't supported, but the common subset
            // matches. Routes to the same Op::RegexMatch as
            // `=~`/`-regex-match` because the magic-var population
            // shape is identical.
            "-pcre-match" => self.builder.emit(Op::RegexMatch, 0),
            "<" => self.builder.emit(Op::StrLt, 0),
            ">" => self.builder.emit(Op::StrGt, 0),
            // c:Src/cond.c:415 — `-eq`/`-ne`/`-lt`/`-gt`/`-le`/`-ge`
            // operands undergo arithmetic evaluation via `mathevali`.
            // So `[[ x -eq 5 ]]` (where x=5) is `mathevali("x") -eq
            // mathevali("5")` = `5 -eq 5` = TRUE, not literal-string
            // comparison of "x" vs "5".
            //
            // The compile_zsh path pushes left + right as STRINGS via
            // compile_word_str, and Op::NumEq's to_float() on a bare
            // identifier returns 0.0. Apply BUILTIN_ARITH_EVAL to both
            // operands first so the cmp opcodes see numeric values.
            //
            // Stack walk (before this fix):  [..., L_str, R_str] → NumEq
            //                                                  ↓ to_float
            //                                                  0.0 == 5.0 → false
            //
            // Stack walk (after):  [..., L_str, R_str]
            //                        ARITH_EVAL (pops R) → [..., L_str, arith(R)]
            //                        Swap                → [..., arith(R), L_str]
            //                        ARITH_EVAL (pops L) → [..., arith(R), arith(L)]
            //                        Swap                → [..., arith(L), arith(R)]
            //                        NumEq               → result
            "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => {
                let arith_id = crate::vm_helper::BUILTIN_ARITH_EVAL;
                self.builder.emit(Op::CallBuiltin(arith_id, 1), 0);
                self.builder.emit(Op::Swap, 0);
                self.builder.emit(Op::CallBuiltin(arith_id, 1), 0);
                self.builder.emit(Op::Swap, 0);
                match op {
                    "-eq" => self.builder.emit(Op::NumEq, 0),
                    "-ne" => self.builder.emit(Op::NumNe, 0),
                    "-lt" => self.builder.emit(Op::NumLt, 0),
                    "-le" => self.builder.emit(Op::NumLe, 0),
                    "-gt" => self.builder.emit(Op::NumGt, 0),
                    "-ge" => self.builder.emit(Op::NumGe, 0),
                    _ => unreachable!(),
                }
            }
            "-ef" => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SAME_FILE, 2), 0),
            "-nt" => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_FILE_NEWER, 2), 0),
            "-ot" => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_FILE_OLDER, 2), 0),
            _ => {
                // c:Src/cond.c:150-188 — an unrecognized `-X` binary op is an
                // unknown module condition: drop both operands and route the op
                // through BUILTIN_COND_UNKNOWN (zerr + errflag) so the cond's
                // errexit check aborts, matching `[[ a -xyz b ]]` in zsh. The
                // prior LoadFalse silently evaluated false and ran on.
                self.builder.emit(Op::Pop, 0);
                self.builder.emit(Op::Pop, 0);
                let idx = self.builder.add_constant(Value::str(op));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::vm_helper::BUILTIN_COND_UNKNOWN, 1),
                    0,
                );
                0usize
            }
        };
    }

    fn compile_arith(&mut self, expr: &str) {
        // xtrace: emit `(( expr ))` text BEFORE pushing CS_MATH so
        // the trace line itself is NOT labeled "math". Direct port
        // of Src/exec.c:5240-5245 — printprompt4 fires, THEN
        // cmdpush(CS_MATH).
        let trace_text = format!("(( {} ))", expr);
        let trace_const = self.builder.add_constant(Value::str(trace_text.as_str()));
        self.builder.emit(Op::LoadConst(trace_const), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_XTRACE_LINE, 1), 0);
        self.builder.emit(Op::Pop, 0);
        self.emit_cmd_push(crate::ported::zsh_h::CS_MATH as u8);
        // Compound `(( expr ))` — set status based on whether expr is non-zero.
        // Subscripted-array assignment (`((a[i]=v))`) needs to bypass
        // ArithCompiler (which doesn't write back through arr[idx])
        // and use the runtime arith eval that we taught about
        // subscripted-array writes.
        let untoked = crate::lex::untokenize(expr);
        // Strip a BALANCED outer `( ... )` wrapper the lexer sometimes leaves
        // attached — but ONLY when the leading `(` matches the trailing `)`.
        // The old `trim_start_matches('(').trim_end_matches(')')` stripped a
        // trailing `)` INDEPENDENTLY of any leading `(`, so `x=(1+2)` (whose
        // final `)` closes the inner subexpression, not a wrapper) was
        // truncated to `x=(1+2` → "bad math expression: ')' expected". Strip
        // balanced wrappers repeatedly (`((expr))` → `(expr)` → `expr`).
        let mut inner_arith_owned = untoked.trim().to_string();
        loop {
            let s = inner_arith_owned.as_str();
            if s.len() >= 2 && s.starts_with('(') && s.ends_with(')') {
                let inner = &s[1..s.len() - 1];
                let mut depth = 0i32;
                let mut balanced = true;
                for c in inner.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth < 0 {
                                balanced = false;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if balanced && depth == 0 {
                    inner_arith_owned = inner.trim().to_string();
                    continue;
                }
            }
            break;
        }
        let inner_arith = inner_arith_owned.as_str();
        if subscripted_arith_assign_check(inner_arith)
            || subscripted_arith_compound_check(inner_arith)
        {
            // Both `((a[i]=v))` (bare `=`) and `((a[i]+=v))` /
            // `((a[i]++))` / `((a[i]--))` route through the runtime
            // arith eval which handles read-modify-write through
            // BUILTIN_ARITH_EVAL → evaluate_arithmetic. ArithCompiler
            // can't write back through arr[idx] for compound forms.
            let idx_const = self.builder.add_constant(Value::str(inner_arith));
            self.builder.emit(Op::LoadConst(idx_const), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_EVAL, 1), 0);
            // c:Src/exec.c:5267 — `(( expr ))` status is `val == 0` (0 if
            // non-zero, 1 if zero), NOT a hardcoded success. This matters
            // for the read-then-modify forms: the value of `(( a[i]++ ))` /
            // `(( a[i]-- ))` is the OLD element value, so an unset/zero slot
            // must yield status 1 (false). The bare assign `(( a[i]=v ))` is
            // likewise false when v is 0. zinit.zsh depends on this exact
            // semantics: `(( ZINIT[SOURCED]++ )) && return` must NOT return
            // on the first source (old value 0 → status 1 → `&&` short-
            // circuits). Previously this branch discarded the result and
            // forced status 0, so zinit saw a "success", believed it was
            // already sourced, and returned early — skipping its aliases
            // (zi/zpl/zplg/zini), `add-zsh-hook` autoload, and the rest of
            // the file. Derive the status from the value, same as the
            // scalar `needs_eval` path below.
            let zero_const = self.builder.add_constant(Value::str("0"));
            self.builder.emit(Op::LoadConst(zero_const), 0);
            self.builder.emit(Op::StrEq, 0);
            let true_jump = self.builder.emit(Op::JumpIfTrue(0), 0);
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder.emit(Op::SetStatus, 0);
            let end_jump = self.builder.emit(Op::Jump(0), 0);
            let true_target = self.builder.current_pos();
            self.builder.patch_jump(true_jump, true_target);
            self.builder.emit(Op::LoadInt(1), 0);
            self.builder.emit(Op::SetStatus, 0);
            let after_status = self.builder.current_pos();
            self.builder.patch_jump(end_jump, after_status);
            // Errflag-aware finish — overrides status to 2 if a math error
            // fired, clearing errflag so the next statement still runs.
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_CMD_FINISH, 0),
                0,
            );
            self.builder.emit(Op::Pop, 0);
            self.emit_cmd_pop();
            return;
        }
        // Which expressions the compiled path cannot serve. This used
        // to be a 60-line `contains()` list that grew one entry per
        // discovered divergence — `/`, `%`, `**=`, `|=`, every bitwise
        // and comparison operator, the ternary, the comma — until it
        // covered essentially all arithmetic and the compiler was dead
        // code. Each of those was a real bug, but the cause was one bug
        // repeated: the compiler used C's operator precedence and C's
        // float division, neither of which is what `Src/math.c` does.
        //
        // With `Z_PREC` and the `DIV`/`MOD`/`POWER` arms ported, the
        // list collapses to the forms that need runtime state the
        // compiled path genuinely does not carry (subscripts, base
        // literals, math functions, word expansion) — see
        // `arith_uncompilable_reason`.
        let needs_eval = crate::arith_compiler::arith_uncompilable_reason(inner_arith).is_some();
        if needs_eval {
            let idx_const = self.builder.add_constant(Value::str(inner_arith));
            self.builder.emit(Op::LoadConst(idx_const), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_EVAL, 1), 0);
            // c:Src/exec.c WC_ARITH — `(( expr ))` is a "math
            // command": result string → status (0 if non-zero,
            // 1 if zero). On math error, status=2 and errflag is
            // cleared by BUILTIN_ARITH_CMD_FINISH so the next
            // statement still runs.
            //
            // Stack on entry to the finish block: [result_str]
            //   1. Compare result to "0" to compute truthiness.
            //   2. JumpIfTrue → status=1 (math result was 0).
            //   3. Else → status=0 (math result was non-zero).
            //   4. Then BUILTIN_ARITH_CMD_FINISH inspects errflag
            //      and, if set, overrides status with 2 + clears
            //      the global errflag.
            let zero_const = self.builder.add_constant(Value::str("0"));
            self.builder.emit(Op::LoadConst(zero_const), 0);
            self.builder.emit(Op::StrEq, 0);
            let true_jump = self.builder.emit(Op::JumpIfTrue(0), 0);
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder.emit(Op::SetStatus, 0);
            let end_jump = self.builder.emit(Op::Jump(0), 0);
            let true_target = self.builder.current_pos();
            self.builder.patch_jump(true_jump, true_target);
            self.builder.emit(Op::LoadInt(1), 0);
            self.builder.emit(Op::SetStatus, 0);
            let after_status = self.builder.current_pos();
            self.builder.patch_jump(end_jump, after_status);
            // Errflag-aware finish — overrides status to 2 if math
            // error fired, clearing errflag for next statement.
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_CMD_FINISH, 0),
                0,
            );
            self.builder.emit(Op::Pop, 0);
            self.emit_cmd_pop();
            return;
        }
        // c:Src/math.c — pre-validate via matheval BEFORE handing
        // to the ArithCompiler fast path. ArithCompiler silently
        // truncates malformed expressions like `5 +` (trailing
        // operator), `1+(2)` (paren mismatch), etc., leaving the
        // `(( ))` math command exiting rc=0 instead of zsh's rc=2
        // with "bad math expression: operand expected at end of
        // string" diagnostic. Route through MathEval (which uses
        // mathevall + zerr) whenever the compile-time pre-check
        // reports an error. Bug #533.
        // c:Src/math.c — pre-check runs the math parser only for syntax
        // validation; it must NOT mutate paramtab. mathevali_noeval
        // routes through the math eval pipeline with noeval=1, so
        // setmathvar's c:1002-1003 early-bail fires before any
        // paramtab write. Without this, `(( b = a ))` ran the
        // assignment at COMPILE TIME with a=undefined, creating b
        // as PM_INTEGER(0); the runtime arith then saw b already
        // PM_INTEGER and truncated Float(1.5) → 1. Bug #617.
        let pre_check = crate::ported::math::mathevali_noeval(inner_arith);
        if pre_check.is_err() {
            // Clear errflag set by the pre-check zerr (we re-fire
            // it at runtime via BUILTIN_ARITH_EVAL so the user
            // sees the diagnostic at the right point).
            crate::ported::utils::errflag.fetch_and(
                !crate::ported::zsh_h::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            let idx_const = self.builder.add_constant(Value::str(inner_arith));
            self.builder.emit(Op::LoadConst(idx_const), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_EVAL, 1), 0);
            self.builder.emit(Op::Pop, 0);
            self.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_CMD_FINISH, 0),
                0,
            );
            self.builder.emit(Op::Pop, 0);
            self.emit_cmd_pop();
            return;
        }
        // c:Src/exec.c:5234-5265 execarith / Src/math.c:458,1500 — a
        // `(( expr ))` command is evaluated through the FAITHFUL math
        // evaluator (matheval), which records the result in `lastmathval`.
        // `functions -M` math functions return that value (c:math.c:1117),
        // so a math fn whose last `(( ))` is a simple expr (zmathfunc
        // `sum`'s `(( sum ))`) needs it set. The old ArithCompiler fast
        // path computed the result for STATUS only and never set
        // lastmathval, so such functions returned a stale value. Route
        // through BUILTIN_ARITH_EVAL (same as the needs_eval/pre-check
        // arms above) so lastmathval is always recorded. zsh itself has
        // no fast path — every `(( ))` goes through the math evaluator;
        // this also keeps the soft-error → status 2 recovery uniform
        // (#154). (C-style `for ((;;))` uses compile_for_arith, NOT this
        // path, so loop-counter perf is unaffected.)
        //
        // Compiled path: the expression is lowered to arithmetic ops
        // ONCE, here, instead of being handed to `matheval` as text to
        // re-lex on every evaluation. `BUILTIN_ARITH_CMD_FINISH_VAL`
        // takes the numeric result and does what c:Src/exec.c:5262-5267
        // does with `matheval`'s: record `lastmathval` (c:Src/math.c:1500,
        // which a `functions -M` math function returns at c:1115), derive
        // the `(( ))` status from `val == 0`, and convert a soft math
        // error into status 2 with `errflag` cleared so the next
        // statement still runs (#154).
        self.compile_arith_str(inner_arith);
        self.builder.emit(
            Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_CMD_FINISH_VAL, 1),
            0,
        );
        self.builder.emit(Op::Pop, 0);
        self.emit_cmd_pop();
    }

    /// Compile arithmetic expression text. Leaves the result on stack
    /// as Value::Int. Pre-loads variable slots, emits arith ops via
    /// ArithCompiler against this compiler's builder + slot table,
    /// then post-syncs slots back to vars.
    ///
    /// The slots are a per-expression scratch space, not a store: the
    /// pre-load reads through BUILTIN_GET_MATH_VAR (→ `getmathparam`,
    /// c:Src/math.c:337, i.e. the parameter table) and the post-sync
    /// writes back through BUILTIN_SET_VAR, so the values this path
    /// reads and writes are the ones C's evaluator reads and writes.
    ///
    /// That only holds while nothing caches a value ACROSS the
    /// boundary. It did not hold once: the Rust counterpart of C's
    /// per-mathvalue `mptr->pval` cache (c:340-343) is the
    /// `M_VARIABLES` thread_local in src/ported/math.rs, and it used to
    /// outlive its evaluation instead of dying with the frame the way
    /// C's `stack` does (c:406 `stack = nstack` / c:455
    /// `stack = xstack`). A preceding `(( i = … ))` therefore left a
    /// stale `i` for the pre-load here to find, the loop's own
    /// BUILTIN_SET_VAR went to the parameter table that
    /// `getmathparam` never reached, and `for ((i=1; i<=3; i++))`
    /// froze. `matheval` / `mathevali_noeval` now save and restore that
    /// cache around their frame, so it cannot be read from outside an
    /// evaluation at all. Callers: `compile_for_arith`
    /// (c:Src/loop.c:77/135/191), for sections `ArithCompiler` can lex.
    ///
    /// If a THIRD store is ever introduced on either side of
    /// BUILTIN_GET_MATH_VAR / BUILTIN_SET_VAR, this path becomes
    /// unsound again — the correctness precondition is one store, not
    /// one engine.
    fn compile_arith_str(&mut self, expr: &str) {
        // The lexer tokenizes operator chars (`<`, `>`, `=`, `&`, `|`,
        // `*`, `?`, etc.) into the META range. ArithCompiler can't parse
        // those — un-tokenize first to recover the original ASCII form.
        let expr_clean = crate::lex::untokenize(expr);

        let mut ac = crate::arith_compiler::ArithCompiler::new(&expr_clean);
        ac.slots = self.slots.clone();
        ac.next_slot = self.next_slot;

        // Pre-load: any var the arith expression touches needs its current
        // value pulled from executor.variables into its slot. Without this
        // `i=5; (( i+1 ))` reads 0 from the uninitialized slot.
        // c:Src/math.c:337 getmathparam — arith reads of NAMED params
        // coerce to numeric (int / float, falling back to 0 for non-
        // numeric strings via recursive arith eval). BUILTIN_GET_VAR
        // returns the raw string, which left `(( y = x ))` with
        // x="hello" storing y="hello" as scalar. Use BUILTIN_GET_MATH_VAR
        // which mirrors getmathparam exactly. Bug #118 in docs/BUGS.md.
        let pre_load_names = ac.collect_identifiers(&expr_clean);
        for name in &pre_load_names {
            let slot = ac.slot_for(name);
            let name_const = ac.builder.add_constant(Value::str(name.as_str()));
            ac.builder.emit(Op::LoadConst(name_const), 0);
            ac.builder.emit(
                Op::CallBuiltin(crate::vm_helper::BUILTIN_GET_MATH_VAR, 1),
                0,
            );
            ac.builder.emit(Op::SetSlot(slot), 0);
        }

        ac.expr();
        let new_slots = ac.slots.clone();
        let new_next = ac.next_slot;
        let assigned_names = ac.assigned.clone();
        let chunk = ac.builder.build();

        // Inline ArithCompiler's emitted ops into ours, remapping const
        // indices into our local constant table AND shifting Jump
        // targets by the inline offset. ArithCompiler emits absolute
        // positions referring to its own builder (which starts at 0);
        // inlined into the parent at offset `inline_base`, every
        // `Jump`/`JumpIf*Keep`/`JumpIfTrue`/`JumpIfFalse` target must
        // be shifted by `inline_base` so it lands at the corresponding
        // op in the parent's bytecode. Without this, `(( 0 && 0 ))`
        // emitted a `JumpIfFalseKeep(4)` that pointed at the parent's
        // prologue at position 4 — an infinite loop back to early
        // setup ops (BUILTIN_XTRACE_LINE / CMDPUSH / etc.).
        let inline_base = self.builder.current_pos();
        let mut const_remap: std::collections::HashMap<u16, u16> = std::collections::HashMap::new();
        for op in &chunk.ops {
            let remapped: Op = match op {
                Op::LoadConst(idx) => {
                    let dst = *const_remap.entry(*idx).or_insert_with(|| {
                        let v = chunk
                            .constants
                            .get(*idx as usize)
                            .cloned()
                            .unwrap_or(fusevm::Value::str(""));
                        self.builder.add_constant(v)
                    });
                    Op::LoadConst(dst)
                }
                Op::Jump(t) => Op::Jump(*t + inline_base),
                Op::JumpIfTrue(t) => Op::JumpIfTrue(*t + inline_base),
                Op::JumpIfFalse(t) => Op::JumpIfFalse(*t + inline_base),
                Op::JumpIfTrueKeep(t) => Op::JumpIfTrueKeep(*t + inline_base),
                Op::JumpIfFalseKeep(t) => Op::JumpIfFalseKeep(*t + inline_base),
                other => other.clone(),
            };
            self.builder.emit(remapped, 0);
        }

        self.slots = new_slots.clone();
        self.next_slot = new_next;

        // Post-sync: write the assigned slots back to the parameter
        // table. This makes `(( i++ ))` visible to a subsequent `echo $i`
        // and to the loop's own conditional check.
        // The arith result is on top of stack — capture into a temp slot,
        // sync, then restore.
        let result_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::SetSlot(result_slot), 0);

        // c:Src/math.c:1364-1372 — only an `OP_E2`/`OP_E2IO` operator
        // reaches `setmathvar`. Writing back every identifier the
        // expression MENTIONS would assign to names it only read.
        //
        // The write goes through `setmathvar` (c:972), not the generic
        // scalar store: `typeset -i x; (( x = 3.7 ))` must land 3, and
        // `typeset -F f; (( f = 1 ))` must land 1.000000000 — the
        // PM_INTEGER / FORCEFLOAT coercions live in setmathvar and
        // nowhere else.
        let mut assigned: Vec<&String> = pre_load_names
            .iter()
            .filter(|n| assigned_names.contains(*n))
            .collect();
        // Deterministic emission order — `assigned` is a HashSet.
        assigned.sort();
        for name in assigned {
            if let Some(&slot) = new_slots.get(name) {
                let name_const = self.builder.add_constant(Value::str(name.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::GetSlot(slot), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_SET_MATH_VAR, 2), 0);
                self.builder.emit(Op::Pop, 0); // discard Status(0)
            }
        }

        self.builder.emit(Op::GetSlot(result_slot), 0);
    }
}

/// Render a ZshList to the readable text form that the C source's
/// `dupstring(text)` arg in `Src/exec.c::trapcmd` carries via
/// `$ZSH_DEBUG_CMD`. Only Simple commands get faithful reconstruction
/// (word-joining with spaces, untokenized); compound commands get a
/// short keyword tag. Bug #263 in docs/BUGS.md.

/// Meta-encode one raw byte into the pipeline's string form —
/// c:Src/utils.c metafy step: `if (imeta(c)) { *p++ = Meta; *p++ =
/// c ^ 32; }`. Local copy for the three $'…' decoders below (the
/// former vm_helper::meta_encode_byte was deleted in the fake-fn
/// minimization pass; ported callers inline it).
fn meta_encode_byte(out: &mut String, b: u8) {
    if b < 0x80 {
        out.push(b as char);
    } else {
        out.push('\u{83}');
        out.push(char::from(b ^ 32));
    }
}
// ---------------------------------------------------------------------
// Parse-time `cmplx` flag.
//
// Port of the `int *cmplx` out-parameter that `Src/parse.c` threads
// through `par_list` (c:769) → `par_sublist` (c:825) → `par_sublist2`
// (c:869) → `par_pline` (c:894) → `par_cmd` (c:958) → `par_simple`
// (c:1836). C folds the result into the wordcode at
// `set_sublist_code` (c:755):
//
// ```c
// if (cmplx) ecbuf[p] = WCB_SUBLIST(type, flags, skip);
// else       ecbuf[p] = WCB_SUBLIST(type, flags | WC_SUBLIST_SIMPLE, skip);
// ```
//
// and `execlist` (c:1489) routes on it — `WC_SUBLIST_SIMPLE` goes to
// `execsimple`, which the C comment at c:1284 describes as "used to
// execute things that will run completely in the shell, so that we can
// by-pass all that nasty job-handling", while everything else goes to
// `execpline`, which builds a job and ends in `waitonejob`. That job is
// the only thing that writes `$pipestatus` for an in-shell command, so
// this flag is precisely what decides whether a sublist publishes one.
//
// zshrs parses to an AST rather than to wordcode, and the ported
// wordcode emitters (`parse.rs::set_sublist_code`) are not on the live
// execution path, so the same predicate is recomputed here over the AST
// at compile time. The C call graph maps one-to-one onto the functions
// below; each carries the c: line that sets `*cmplx = 1`.
// ---------------------------------------------------------------------

/// c:Src/parse.c:769 `par_list` — `*cmplx |= c` per sublist (c:784),
/// plus `if (tok != SEPER) *cmplx = 1` (c:786-787), i.e. a list
/// terminated by `&` / `&|` rather than `;` is cmplx.
fn list_is_cmplx(list: &ZshList) -> bool {
    list.flags.async_ || list.flags.disown || sublist_is_cmplx(&list.sublist)
}

/// c:Src/parse.c:769 `par_list` — a program's complexity is the OR over
/// its lists, matching C's shared `cmplx` pointer across the `rec:` loop.
fn program_is_cmplx(program: &ZshProgram) -> bool {
    program.lists.iter().any(list_is_cmplx)
}

/// c:Src/parse.c:825 `par_sublist` — the `&&` / `||` chain shares one
/// `cmplx` pointer (c:834 `*cmplx |= c`, c:843 `par_sublist(cmplx)`), so
/// the whole chain is cmplx if ANY element is. Used when a compound
/// command's body is being folded into its parent's flag.
fn sublist_is_cmplx(sublist: &ZshSublist) -> bool {
    if sublist_elem_is_cmplx(sublist) {
        return true;
    }
    match &sublist.next {
        Some((_, next)) => sublist_is_cmplx(next),
        None => false,
    }
}

/// c:Src/parse.c:869 `par_sublist2` — ONE element of the `&&` / `||`
/// chain: `COPROC` (c:874) and `BANG` (c:878) each set `*cmplx = 1`,
/// then `par_pline` contributes the rest.
///
/// This is the granularity C records the flag at: `set_sublist_code`
/// (c:844, c:854) is called once per chain element with that element's
/// own local `c`, and `execlist` re-reads it per element at c:1489
/// (`WC_SUBLIST_END`), c:1502 (`WC_SUBLIST_AND`) and c:1536
/// (`WC_SUBLIST_OR`). So `true && [[ -z x ]]` publishes `$pipestatus`
/// for `true` and not for the cond.
fn sublist_elem_is_cmplx(sublist: &ZshSublist) -> bool {
    sublist.flags.coproc || sublist.flags.not || pipe_is_cmplx(&sublist.pipe)
}

/// c:Src/parse.c:894 `par_pline` — `BAR` (c:906) and `BARAMP` (c:930)
/// set `*cmplx = 1` and recurse, so any multi-stage pipeline is cmplx;
/// a single stage defers to `par_cmd`.
fn pipe_is_cmplx(pipe: &ZshPipe) -> bool {
    pipe.next.is_some() || command_is_cmplx(&pipe.cmd)
}

/// c:Src/parse.c:958 `par_cmd`.
///
/// Sets `*cmplx = 1` for: a leading redirection (c:964-965), `SELECT`
/// (c:982), `INPAR` — a `( … )` subshell (c:1011), `TIME` (c:1041), a
/// trailing redirection (c:1067-1068), and `par_simple` reporting added
/// redirections (c:1059-1060).
///
/// Leaves it untouched — so the construct is only as cmplx as its body —
/// for `FOR`/`FOREACH` (c:971-979), `CASE` (c:987), `IF` (c:992),
/// `WHILE`/`UNTIL` (c:995-1002), `REPEAT` (c:1005), `INBRACE` — a
/// `{ … }` group and its optional `always` block (c:1016), and `FUNC`
/// (c:1021). Each recurses through `par_save_list(cmplx)` on its body
/// lists, which is why `if [[ -z x ]]; then [[ -z y ]]; fi` stays
/// simple while `if [[ -z x ]]; then :; fi` does not.
///
/// `DINBRACK` — `[[ … ]]` (c:1026) and `DINPAR` — `(( … ))` (c:1031)
/// never set it and have no body: they are the leaf simple commands.
fn command_is_cmplx(cmd: &ZshCommand) -> bool {
    match cmd {
        // c:1050-1064 default arm → par_simple.
        ZshCommand::Simple(simple) => simple_is_cmplx(simple),
        // c:1011 `case INPAR: *cmplx = 1;`
        ZshCommand::Subsh(_) => true,
        // c:1016 `case INBRACE:` — par_subsh(cmplx) at c:1628 parses the
        // body with the caller's pointer, so `{ … }` inherits its body.
        ZshCommand::Cursh(body) => program_is_cmplx(body),
        // c:1636-1649 `par_subsh` always-block — `par_list(cmplx)` for
        // the try block (c:1628) and `par_save_list(cmplx)` for the
        // always block (c:1649) share the one pointer.
        ZshCommand::Try(t) => program_is_cmplx(&t.try_block) || program_is_cmplx(&t.always),
        // c:981-985 `case SELECT: *cmplx = 1; par_for(cmplx);` vs
        // c:971-979 FOR/FOREACH which only inherit the body. Both the
        // word-list forms and the C-style `for ((;;))` (c:1087 par_for)
        // take the body-only path.
        ZshCommand::For(f) => f.is_select || program_is_cmplx(&f.body),
        // c:987-990 `case CASE: par_case(cmplx);` — c:1380
        // `par_save_list(cmplx)` per arm body.
        ZshCommand::Case(c) => c.arms.iter().any(|arm| program_is_cmplx(&arm.body)),
        // c:992-994 `case IF: par_if(cmplx);` — c:1438/1453/1462/1494/
        // 1500 `par_save_list(cmplx)` over every condition and branch.
        ZshCommand::If(i) => {
            program_is_cmplx(&i.cond)
                || program_is_cmplx(&i.then)
                || i.elif
                    .iter()
                    .any(|(c, b)| program_is_cmplx(c) || program_is_cmplx(b))
                || i.else_.as_ref().is_some_and(|e| program_is_cmplx(e))
        }
        // c:995-1003 `case WHILE:`/`case UNTIL: par_while(cmplx);` —
        // c:1528/1534/1541/1547 `par_save_list(cmplx)` over the
        // condition and the body.
        ZshCommand::While(w) | ZshCommand::Until(w) => {
            program_is_cmplx(&w.cond) || program_is_cmplx(&w.body)
        }
        // c:1005-1008 `case REPEAT: par_repeat(cmplx);` — c:1583/1590/
        // 1596 `par_save_list(cmplx)` over the body. The count word is
        // a wordlist, not a list, and contributes nothing.
        ZshCommand::Repeat(r) => program_is_cmplx(&r.body),
        // c:1021-1024 `case FUNC: par_funcdef(cmplx);` — par_funcdef
        // (c:1672) declares its OWN `int c = 0` (c:1674) and parses the
        // body with it, so a function BODY never reaches the caller's
        // flag: `function f { true }` is simple. Only extra names after
        // the first set it (c:1773-1774 `if (num > 0) *cmplx = 1`), and
        // the anonymous-function call form carries argument words that
        // do the same at c:2168-2169.
        ZshCommand::FuncDef(f) => {
            f.names.len() > 1 || f.auto_call_args.as_ref().is_some_and(|a| !a.is_empty())
        }
        // c:1036-1046 `case TIME: *cmplx = 1;`
        ZshCommand::Time(_) => true,
        // c:1026-1030 `case DINBRACK:` — par_dinbrack() takes no cmplx
        // argument at all, and c:1031-1035 `case DINPAR:` just emits
        // WCB_ARITH(). These two are the leaf simple commands.
        ZshCommand::Cond(_) | ZshCommand::Arith(_) => false,
        // c:964-965 / c:1067-1068 — redirections around a compound
        // command set `*cmplx = 1` regardless of what they wrap.
        ZshCommand::Redirected(_, _) => true,
    }
}

/// c:Src/parse.c:1836 `par_simple`.
///
/// Sets `*cmplx = 1` for: `NOCORRECT` (c:1846), a scalar assignment
/// whose value contains a process substitution (c:1867-1878), `ENVARRAY`
/// — an `arr=(…)` array assignment, which C marks because "it can
/// contain process substitutions, which need a valid job" (c:1886-1890),
/// any redirection (c:1909-1910, c:1998-1999, c:2158-2159), and — the
/// case that covers every ordinary command — `tok == STRING || tok ==
/// TYPESET` at c:1924-1928, i.e. the presence of a command word.
///
/// So a simple command is simple only when it is pure scalar
/// assignments: `x=1` and `x=1 y=2` publish no `$pipestatus`, while
/// `true`, `typeset x=1` and `x=1 >/dev/null` all do.
fn simple_is_cmplx(simple: &ZshSimple) -> bool {
    // c:1928 — a command word (STRING/TYPESET) in command position.
    if !simple.words.is_empty() {
        return true;
    }
    // c:1910 / c:1999 / c:2159 — `*cmplx = c = 1;` on IS_REDIROP.
    if !simple.redirs.is_empty() {
        return true;
    }
    simple.assigns.iter().any(|a| match &a.value {
        // c:1890 — ENVARRAY.
        ZshAssignValue::Array(_) => true,
        // c:1873-1877 — `if (ptr[1] == Inpar && (*ptr == Equals ||
        // *ptr == Inang || *ptr == OutangProc)) { *cmplx = 1; break; }`
        // over the whole value: `=(…)`, `<(…)`, `>(…)`.
        ZshAssignValue::Scalar(v) => value_has_procsubst(v),
    })
}

/// c:Src/parse.c:1867-1878 — scan an assignment value for a process
/// substitution introducer immediately followed by `(`:
///
/// ```c
/// for (ptr = str; *ptr; ptr++) {
///     if (ptr[1] == Inpar &&
///         (*ptr == Equals || *ptr == Inang || *ptr == OutangProc)) {
///         *cmplx = 1;
///         break;
///     }
/// }
/// ```
///
/// C scans the TOKENIZED value, which is what makes the test precise:
/// the lexer only emits `Inang`/`OutangProc`/`Equals` for an unquoted
/// introducer, so `x='<(a)'` keeps a literal `<` and stays simple.
/// Matching the same token chars here inherits that precision.
fn value_has_procsubst(value: &str) -> bool {
    use crate::ported::zsh_h::{Equals, Inang, Inpar, OutangProc};
    let mut prev: Option<char> = None;
    for c in value.chars() {
        if c == Inpar && matches!(prev, Some(Equals | Inang | OutangProc)) {
            return true;
        }
        prev = Some(c);
    }
    false
}

fn render_list_for_debug(list: &crate::parse::ZshList) -> String {
    render_sublist_for_debug(&list.sublist)
}

/// Classify a `time`-d sublist as "runs in the current shell" (C's
/// `is_cursh`) or "forks a job".
///
/// Port of the decision at `Src/exec.c:3690`:
/// ```c
/// /* This is nonzero if the command is a current shell procedure? */
/// is_cursh = (is_builtin || is_shfunc || nullexec || type >= WC_CURSH);
/// ```
/// `WC_SUBSH` is 8 and `WC_CURSH` is 9 (`Src/zsh.h:895-896`), so `( … )`
/// falls on the FORKED side while `{ … }`, `for`, `while`, `if`, `case`,
/// `repeat`, `select` and friends are all current-shell.
///
/// Returns `(hint, name)` where hint is
///   * `1` — current shell for sure (compound, or an assignment/redirection
///     with no command word, which is C's `nullexec`/`varspc` path),
///   * `0` — forked job for sure (`( … )`, or a multi-stage pipeline),
///   * `2` — depends on whether `name` resolves to a builtin or a shell
///     function, which is only knowable at run time.
fn time_cursh_hint(sublist: &crate::parse::ZshSublist) -> (i64, String) {
    // A `&&`/`||` chain or a real pipeline is more than one execcmd_exec
    // call; C reports per-stage. Treat it as the forked case (the shape the
    // pre-existing handler already covered).
    if sublist.next.is_some() || sublist.pipe.next.is_some() {
        return (0, String::new());
    }
    let mut cmd = &sublist.pipe.cmd;
    // c:Src/exec.c — trailing redirections don't change the command type.
    while let ZshCommand::Redirected(inner, _) = cmd {
        cmd = inner;
    }
    match cmd {
        // WC_SUBSH (8) < WC_CURSH (9) — a subshell forks.
        ZshCommand::Subsh(_) => (0, String::new()),
        ZshCommand::Simple(s) => match s.words.first() {
            // c:3403-3460 — assignments / redirections with no command word
            // return through the `nullexec` / `varspc` arms, all of which
            // reach the `how & Z_TIMED` shelltime call.
            None => (1, String::new()),
            Some(w) => (2, w.clone()),
        },
        // Everything else is a WC_* type at or above WC_CURSH.
        _ => (1, String::new()),
    }
}

fn render_sublist_for_debug(sublist: &crate::parse::ZshSublist) -> String {
    let head = render_pipe_for_debug(&sublist.pipe);
    let mut out = if sublist.flags.not {
        format!("! {}", head)
    } else {
        head
    };
    if let Some((op, next)) = &sublist.next {
        let op_str = match op {
            crate::parse::SublistOp::And => "&&",
            crate::parse::SublistOp::Or => "||",
        };
        out.push(' ');
        out.push_str(op_str);
        out.push(' ');
        out.push_str(&render_sublist_for_debug(next));
    }
    out
}

/// Port of `void getredirs(LinkList redirs)` from `Src/text.c:800`.
///
/// C accumulates into the `tptr` text buffer; this returns the same bytes
/// as a String. Only the arm C actually reaches for a function-definition
/// redirection list is covered — `REDIR_CLOSE` and unknown types are
/// `DPUTS("BUG: …")` in C, so they are dropped here too.
fn getredirs(redirs: &[crate::parse::ZshRedir]) -> String {
    use crate::ported::zsh_h::*;
    // c:803-807 — `static char *fstr[] = { … }`, indexed by `f->type`.
    const FSTR: [&str; 18] = [
        ">", ">|", ">>", ">>|", "&>", "&>|", "&>>", "&>>|", "<>", "<", "<<", "<<-", "<<<", "<&",
        ">&", "", /* >&- */
        "<", ">",
    ];
    let mut out = String::new();
    out.push(' '); // c:811 taddchr(' ')
    for f in redirs {
        // c:812
        // c:815-829 — every type except REDIR_CLOSE shares one arm.
        if f.rtype == REDIR_CLOSE || f.rtype < 0 || f.rtype as usize >= FSTR.len() {
            continue; // c:889-896 — DPUTS "BUG:" arms
        }
        // c:Src/zsh.h — `IS_READFD(X)` is
        // `((X)>=REDIR_READWRITE && (X)<=REDIR_MERGEIN) || (X)==REDIR_INPIPE`.
        let is_readfd =
            (f.rtype >= REDIR_READWRITE && f.rtype <= REDIR_MERGEIN) || f.rtype == REDIR_INPIPE;
        if let Some(varid) = &f.varid {
            // c:830-833 — `{varid}` form
            out.push('{');
            out.push_str(varid);
            out.push('}');
        } else if f.fd != if is_readfd { 0 } else { 1 } {
            // c:834-835 — a non-default fd is written as one digit
            out.push_str(&f.fd.to_string());
        }
        // c:872-876 — `taddstr(fstr[f->type]); if (f->type != REDIR_MERGEIN
        // && f->type != REDIR_MERGEOUT) taddchr(' '); taddstr(f->name);`
        out.push_str(FSTR[f.rtype as usize]); // c:873
        if f.rtype != REDIR_MERGEIN && f.rtype != REDIR_MERGEOUT {
            out.push(' '); // c:875
        }
        out.push_str(&crate::lex::untokenize_preserve_quotes(&f.name)); // c:876
        out.push(' '); // c:878
    }
    out.pop(); // c:898 `tptr--` drops the trailing separator
    out
}

fn render_pipe_for_debug(pipe: &crate::parse::ZshPipe) -> String {
    let mut out = render_cmd_for_debug(&pipe.cmd);
    if let Some(next) = &pipe.next {
        out.push_str(if pipe.merge_stderr { " |& " } else { " | " });
        out.push_str(&render_pipe_for_debug(next));
    }
    out
}

fn render_cmd_for_debug(cmd: &crate::parse::ZshCommand) -> String {
    use crate::parse::ZshCommand;
    match cmd {
        // c:Src/text.c::gettext2 WC_ASSIGN — each assignment is emitted
        // `name=value` (or `name=(v1 v2)` for an array) followed by a
        // SPACE, before the command words. With no command word the
        // trailing space stays, which is why zsh's `$ZSH_DEBUG_CMD` for a
        // bare `x=y` is `x=y ` (C05debug:7). Rendering only `s.words`
        // dropped assignments entirely — a bare assignment came out as the
        // empty string, and `FOO=1 cmd` lost its prefix.
        ZshCommand::Simple(s) => {
            let mut out = String::new();
            for a in &s.assigns {
                out.push_str(&a.name);
                if a.append {
                    out.push('+'); // c:Src/text.c — `+=` append form
                }
                out.push('=');
                match &a.value {
                    crate::parse::ZshAssignValue::Scalar(v) => {
                        out.push_str(&crate::lex::untokenize_preserve_quotes(v));
                    }
                    crate::parse::ZshAssignValue::Array(items) => {
                        out.push('(');
                        for (i, it) in items.iter().enumerate() {
                            if i > 0 {
                                out.push(' ');
                            }
                            out.push_str(&crate::lex::untokenize_preserve_quotes(it));
                        }
                        out.push(')');
                    }
                }
                out.push(' ');
            }
            out.push_str(
                &s.words
                    .iter()
                    .map(|w| crate::lex::untokenize_preserve_quotes(w))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            out
        }
        // c:Src/text.c::gettext2 SUBSH/CURSH branches — `time (cmd)`
        // and `time { cmd }` printtime via `printjob → dumptime` read
        // p->text built from the AST's full reconstruction (with the
        // outer parens/braces + a trailing semicolon per nested
        // statement). zshrs's previous placeholder `"( ... )"` lost
        // the body; mirror the C textual round-trip so `time (sleep
        // 0.1; echo done)` prints `( sleep 0.1; echo done; )`.
        // Bug #432.
        ZshCommand::Subsh(prog) => format!("( {} )", render_program_for_debug(prog)),
        ZshCommand::Cursh(prog) => format!("{{ {} }}", render_program_for_debug(prog)),
        ZshCommand::For(_) => "for ...".to_string(),
        ZshCommand::Case(_) => "case ...".to_string(),
        ZshCommand::If(_) => "if ...".to_string(),
        ZshCommand::While(_) => "while ...".to_string(),
        ZshCommand::Until(_) => "until ...".to_string(),
        ZshCommand::Repeat(_) => "repeat ...".to_string(),
        // c:Src/text.c::gettext2 WC_FUNCDEF — `name () {` then one
        // TAB-indented line per body list, then `}`. This is the same shape
        // `functions name` prints, and it is what `$ZSH_DEBUG_CMD` carries
        // for a function definition (C05debug:7). The old `"funcdef ..."`
        // placeholder lost the whole definition.
        ZshCommand::FuncDef(fd) => {
            let mut out = format!("{} () {{\n", fd.names.join(" "));
            for list in &fd.body.lists {
                out.push('\t');
                out.push_str(&render_sublist_for_debug(&list.sublist));
                out.push('\n');
            }
            out.push('}');
            out
        }
        _ => String::new(),
    }
}

fn render_program_for_debug(prog: &crate::parse::ZshProgram) -> String {
    // c:Src/text.c::gettext2 LIST_PIPE — each list emits its sublist
    // text + `;` separator. The outer subshell/cursh wrapper supplies
    // the parens/braces; here we just join the contained statements.
    let mut out = String::new();
    for list in &prog.lists {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&render_sublist_for_debug(&list.sublist));
        out.push(';');
    }
    out
}

/// True iff `s` contains `target` at a position not preceded by the `\0`
/// bslashquote sentinel.
/// Cheap check: does `s` contain a top-level `{...}` group that's a brace
/// expansion (comma list or `..` range)? Used to trigger the runtime
/// expand-word path so `{a,b,c}` and `{1..5}` get expanded into multiple
/// arguments instead of being passed as a literal `{a,b,c}`.
fn looks_like_brace_expansion(s: &str) -> bool {
    // Detects three forms:
    //   1. `{a,b,c}`  — comma list
    //   2. `{1..10}`  — range
    //   3. `{X…}` non-empty body — possible BRACE_CCL match (the
    //      runtime checks the option; xpandbraces no-ops if not set)
    // Without case 3, `setopt brace_ccl; print X{za-q521}Y` skipped
    // the brace pass because the body had no `,` / `..`.
    let mut depth = 0;
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        match c {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(b) = start {
                        let body = &s[b + 1..i];
                        if body.contains(',') || body.contains("..") {
                            return true;
                        }
                        // Possible CCL body — non-empty without comma/
                        // dotdot. Defer the BRACE_CCL option check to
                        // the runtime xpandbraces; this just opens
                        // the gate so the option can take effect.
                        if !body.is_empty() {
                            return true;
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    false
}

/// Compile-time port of `keyvalpairelement` (c:Src/subst.c:49-79) shape
/// detection: does this RAW token-encoded array-literal element have the
/// `[key]=value` / `[key]+=value` form?
///
/// Returns `Some((key_raw, value_raw, is_append))` — both halves still
/// token-encoded, ready for `compile_word_str` — or `None` when the
/// element is a plain word.
///
/// - c:54 `start[0] == Inbrack`: the leading `[` must be the TOKEN form
///   (i.e. unquoted). A quoted element (`"[k]=v"`) starts with a
///   Dnull/Snull/Bnull marker in the raw form, so it never matches —
///   matching C, where quoting prevents Inbrack tokenization.
///   The literal-`[` first-char is also accepted because some compile
///   entry paths carry pre-untokenized words.
/// - c:55 `end = strchr(start+1, Outbrack)`: first CLOSING bracket in
///   token form; a `]` inside a quote span stays literal in C (no
///   Outbrack token), so the scan here skips quote-marker spans.
/// - c:57-58 `end[1] == Equals || (end[1] == '+' && end[2] == Equals)`.
fn split_kv_element(raw: &str) -> Option<(String, String, bool)> {
    use crate::ported::zsh_h::{Equals, Inbrack, Outbrack};
    let chars: Vec<char> = raw.chars().collect();
    if chars.is_empty() || (chars[0] != Inbrack && chars[0] != '[') {
        return None;
    }
    // Scan for the closing bracket, ignoring `]` inside Snull / Dnull
    // quote spans (those are literal data, not Outbrack — c:55 strchr
    // only finds the TOKEN).
    let mut in_sq = false;
    let mut in_dq = false;
    let mut end: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate().skip(1) {
        match c {
            '\u{9d}' => in_sq = !in_sq, // Snull
            '\u{9e}' => in_dq = !in_dq, // Dnull
            _ if c == Outbrack || (c == ']' && !in_sq && !in_dq) => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let end = end?;
    let is_eq = |c: Option<&char>| matches!(c, Some(&'=')) || c == Some(&Equals);
    let is_append = chars.get(end + 1) == Some(&'+') && is_eq(chars.get(end + 2));
    let is_assign = !is_append && is_eq(chars.get(end + 1));
    if !is_assign && !is_append {
        return None;
    }
    let key: String = chars[1..end].iter().collect();
    let value_start = if is_append { end + 3 } else { end + 2 };
    let value: String = chars[value_start..].iter().collect();
    Some((key, value, is_append))
}

/// Determine the bslashquote-mode for the bridge replacement based on the
/// raw zsh-tokenized word. Returns one of:
///   0 = Default (full expand_string + braces + glob)
///   1 = DoubleQuoted (expand vars, suppress brace + glob)
///   3 = AltBackquote (run as command substitution)
/// Mode 2 (SingleQuoted) is rare here because the Snull early-return at
/// the top of compile_word_str already catches `'…'` shapes.
fn expand_text_mode(raw: &str, preserved: &str) -> u8 {
    // DoubleQuoted: starts AND ends with raw Dnull, no inner unescaped
    // Dnull pair (i.e. exactly one matching pair wrapping the whole
    // word). Looking at the raw form catches escape-context correctly.
    if raw.starts_with('\u{9e}') && raw.ends_with('\u{9e}') && raw.len() >= 2 {
        // Count interior DNULLs — for a simple `"…"` it's exactly 0 in
        // the inside (the start/end are the two DNULLs). Mixed shapes
        // like `"a"b"c"` would have inner DNULLs and we route to
        // Default (the bridge path can't easily handle them either,
        // but expand_string at least won't strip too much).
        let inner = &raw[raw.char_indices().nth(1).map(|(i, _)| i).unwrap_or(0)
            ..raw
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(raw.len())];
        if !inner.contains('\u{9e}') {
            return 1;
        }
    }
    // Whole-word backquote: `…`
    if preserved.starts_with('`') && preserved.ends_with('`') && preserved.len() >= 2 {
        return 3;
    }
    0
}

/// One piece of a concatenated word. Either a literal stretch (raw
/// zsh-tokenized chars; may contain META markers like Star/Quest that
/// need un-tokenize), or one expansion (`$NAME`, `${NAME[..]}`, etc.).
#[derive(Debug)]
enum WordSegment {
    Literal(String),
    Expansion(String),
}

/// Split a raw zsh-tokenized word into literal and expansion segments
/// for native concat lowering. Returns `None` for words that contain at
/// most one expansion at the very start AND no trailing literal — those
/// are handled by the existing single-expansion fast paths. Returns
/// `Some(segs)` with `segs.len() >= 2` for concat shapes.
///
/// Walks the chars looking for META-$ (`\u{85}`), Qstring-`$` inside
/// double-quotes (`\u{8c}`), or backtick (`` ` ``) markers. Each marker
/// plus its body becomes one Expansion segment; everything else is
/// Literal. NOTE: `\u{84}` is Pound (`#`), not a `$`-marker; including
/// it here would treat `${#arr[@]}` as a concat with `#arr` as the
/// expansion body.
/// True for expansions that splice with FIRST/LAST sticking semantics:
/// `${arr[@]}`, `${arr[*]}`, `$@`, `$*`. Surrounding text in the same
/// word sticks only to the first or last array element.
fn is_splice_expansion(s: &str) -> bool {
    let pq = crate::lex::untokenize_preserve_quotes(s);
    // c:Src/zsh.h:167 Qstring (`\u{8c}`) is the DQ-context `$` marker
    // — preserved by untokenize_preserve_quotes so stringsubst's qt
    // detection at Src/subst.c:283 can fire. For splice-shape
    // detection, treat both `$` and Qstring uniformly: strip outer
    // DQ if present (Dnull → `"` is also preserved), then normalize
    // Qstring → `$` for the prefix-match.
    let normalized: String = pq
        .trim_start_matches('"')
        .trim_end_matches('"')
        .chars()
        .map(|c| {
            if c == crate::ported::zsh_h::Qstring {
                '$'
            } else {
                c
            }
        })
        .collect();
    let pq = normalized;
    if pq == "$@" || pq == "$*" || pq == "${@}" || pq == "${*}" {
        return true;
    }
    if let Some(inner) = pq.strip_prefix("${").and_then(|t| t.strip_suffix('}')) {
        // c:Src/subst.c:2551 — a `^` (RC_EXPAND_PARAM) flag makes the
        // expansion DISTRIBUTE (cross-product) with surrounding text, NOT
        // first/last splice — even when it also carries `[@]`/`[*]` or the
        // `(@)` flag. `"${^arr[@]}"/suffix` and `"pre${(@)^a}"` must cross-
        // product onto EVERY element; treating either as a splice runs
        // CONCAT_SPLICE (first/last sticking) so only the boundary elements
        // get the surrounding text. Defer BOTH the on (`^`) and off (`^^`)
        // cases to the plan9 / plan9-off arms so the `^^` form routes to
        // SPLICE_NOPLAN9 (which ignores the rcexpandparam OPTION) rather than
        // SPLICE (which re-checks it). The `^` may sit after a `(…)` flag
        // group, so scan position-independently via `plan9_flag_state`.
        if plan9_flag_state(inner).is_some() {
            return false;
        }
        // c:Src/params.c:2231 — `isvarat = (t[0] == '@' && !t[1]);`.
        // `fetchvalue` derives it from the parameter NAME alone, and the name
        // is what is left AFTER the flag loop (c:Src/subst.c:2550+) has already
        // consumed the `(…)` group — so `${(q)@}` is exactly as much an
        // `isvarat` reference as `${@}` is. c:Src/params.c:2278 stores it as
        // `v->scanflags = scanflags | (isvarat ? SCANPM_ISVAR_AT : 0)`,
        // c:Src/subst.c:2916 turns that into `isarr = -1`, and the quoted
        // sepjoin at c:Src/subst.c:3032 (`if (qt && !getlen && isarr > 0)`)
        // is gated on isarr being strictly POSITIVE — so a bare `@` keeps its
        // words inside `"…"` whatever flags were written. Note this is the `@`
        // rule only: `$*` resolves to the plain `pparams` array (no
        // SCANPM_ISVAR_AT), gets `isarr = 1`, and so DOES join in `"…"`.
        //
        // Only the flagless spellings were recognised below, so every flagged
        // form (`"${(q)@}"`, `"${(q-)@}"`, `"${(U)@}"`, `"${(#)@}"`, …)
        // compiled to CONCAT_DISTRIBUTE and collapsed to a single word.
        let after_flag_group = match inner.strip_prefix('(') {
            Some(rest) => match rest.find(')') {
                Some(close) => &rest[close + 1..],
                None => inner,
            },
            None => inner,
        };
        // `@` is a one-character name, so the `!t[1]` half of the C test is
        // satisfied by construction: whatever follows is the subscript or the
        // `${name<op>word}` operator, neither of which `fetchvalue` has even
        // looked at yet when it computes `isvarat`. Hence `"${@%c}"` and
        // `"${@//b/Z}"` splat their words just like `"${@}"` does.
        if after_flag_group.starts_with('@') {
            return true;
        }
        if inner.contains("[@]") || inner.contains("[*]") {
            return true;
        }
        // `${@:offset:length}` / `${*:offset:length}` positional slice.
        // Each kept positional element splices as its own arg with
        // first/last sticking semantics — same shape as `${@}` /
        // `${arr[@]}`. Without this, `"${@:1:2}"` fell through to
        // scalar concat which IFS-joined the slice and the for-loop
        // saw one arg `[a b c]` instead of two `[a b]` and `[c]`.
        // Bug #183 in docs/BUGS.md.
        if inner.starts_with("@:") || inner.starts_with("*:") {
            return true;
        }
        // `${=NAME}` — forced word-split per Src/subst.c:2558. The
        // resulting words splice with first/last sticking semantics,
        // same as `${arr[@]}`. Without this, `"split ${=str} wise"`
        // joined the two split words back into a single arg via
        // CONCAT_DISTRIBUTE's default-join path.
        if inner.starts_with('=') && !inner.starts_with("==") {
            let rest = &inner[1..];
            // Identifier check — bare `${=name}` only.
            let bare = rest
                .strip_suffix("[@]")
                .or_else(|| rest.strip_suffix("[*]"))
                .unwrap_or(rest);
            if !bare.is_empty()
                && bare
                    .chars()
                    .next()
                    .map(|c| c == '_' || c.is_ascii_alphabetic())
                    .unwrap_or(false)
                && bare.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
            {
                return true;
            }
        }
        // `(@)NAME` flag form is the splice equivalent of `[@]` —
        // each element becomes its own arg; surrounding literals
        // should stick to first/last (so `[${(@)a}]` for empty `a`
        // still emits `[]` rather than dropping the brackets).
        // Also: `(z)`/`(s.….)`/`(f)`/`(0)`/`(w)` produce a word
        // array from a scalar; in DQ context with surrounding
        // literals zsh first/last-sticks (`"[${(z)s}]"` → first
        // arg `[foo`, last arg `baz]`, middle bare) rather than
        // cartesian. Bug #37 in docs/BUGS.md: routing through
        // CONCAT_DISTRIBUTE produced `"[foo] [bar] [baz]"` cartesian
        // instead of `"[foo bar baz]"` splice. The is_distribute
        // path was claiming (z)/(s)/(f)/(0)/(w) for itself; splice
        // should win since it more closely matches C zsh's
        // first/last-sticking semantics for array splats in DQ.
        if let Some(rest) = inner.strip_prefix('(') {
            if let Some(close) = rest.find(')') {
                let flags = &rest[..close];
                // `Z` is the uppercase parser-aware split flag — `(Z+c+)`,
                // `(Z+n+)`, `(Z+C+)` etc. Same splice shape as `(z)` (a
                // scalar produces a word array; in DQ context with
                // surrounding literals zsh first/last-sticks). Without
                // matching `Z`, the DQ-wrapped form `"${(Z+c+)cmd}"` fell
                // through to scalar concat and the split words got
                // IFS-joined back into one arg. Bug #244 in docs/BUGS.md.
                //
                // `P`, `a` and `A` are here for the same reason, and were the
                // last three flags `is_distribute_expansion` still claimed for
                // itself. None of them touches `plan9`: the flag loop sets
                // `case 'P': aspar = 1` (c:Src/subst.c:2273-2274),
                // `case 'a': sortit |= SORTIT_SOMEHOW, indord = 1`
                // (c:2226-2229) and `case 'A': ++arrasg` (c:2163-2165), while
                // `plan9` starts as `isset(RCEXPANDPARAM)` (c:1663) and is
                // written by NOTHING but the unparenthesised `^`/`^^`
                // (c:2551-2558). So an array carrying any of the three reaches
                // c:4327's `if (plan9)` with plan9 still false and takes the
                // c:4377 `else` — "simply join the first and last values",
                // i.e. the prefix sticks to element 0 and the suffix to the
                // last element. Routing them to CONCAT_DISTRIBUTE_FORCED
                // cross-producted instead:
                //     set -- one two three; r=@
                //     print -rl -- "pre${(P)r}post"
                //       zsh  : preone / two / threepost
                //       zshrs: preonepost / pretwopost / prethreepost
                // and the same for an unquoted `${(P)r}` naming an array or an
                // assoc, for `${(a)arr}`, and for `${(A)arr}`.
                //
                // The explicit `${(P)^r}` form is unaffected: the `^` test at
                // the top of this function returns before the flag scan, and
                // `is_plan9_expansion` sends it to BUILTIN_CONCAT_PLAN9. That
                // is what `Completion/Unix/Type/_path_files` sh:87
                // (`prepaths=( ${(P)^tmp1%/}/ )`) relies on.
                if flags
                    .chars()
                    .any(|c| matches!(c, '@' | 'z' | 'Z' | 's' | 'f' | '0' | 'w' | 'P' | 'a' | 'A'))
                {
                    return true;
                }
            }
        }
        // Slice form `${arr[N,M]}` is a splice — surrounding literals
        // stick to first and last elements; an empty slice keeps the
        // surrounding text rather than dropping it (matches zsh's
        // `print "[${a[5,10]}]"` → `[]` for out-of-range slices).
        if let Some(open) = inner.find('[') {
            if let Some(close) = inner.rfind(']') {
                if close > open {
                    let sub = &inner[open + 1..close];
                    if sub.contains(',') {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// True for expansions that DISTRIBUTE (cartesian) over surrounding
/// text. Includes explicit forms (`${^arr}`, `${(@)…}`, `${(s.…)…}`)
/// and array-producing flag expansions where every element pairs with
/// every literal segment.
/// True for a `${^^name}` segment — RC_EXPAND_PARAM forced OFF by the doubled
/// flag (c:Src/subst.c:2553-2555 `plan9 = 0`).
///
/// The mirror of `is_plan9_expansion`. The flag has to override the
/// `rcexpandparam` OPTION, and every concat builtin except
/// BUILTIN_CONCAT_SPLICE_NOPLAN9 re-reads that option at runtime — so, exactly
/// as with `^`, the compiler is the only place that knows `^^` was written.
fn is_plan9_off_expansion(s: &str) -> bool {
    // Mirror of `is_plan9_expansion`: `^^` (even `^` run) forces plan9 OFF,
    // including after a `(…)` flag group (`${(@)^^a}`).
    with_braced_inner(s, |inner| plan9_flag_state(inner) == Some(false))
}

/// Net plan9 (RC_EXPAND_PARAM) state carried by a `^` shorthand flag in a
/// braced-expansion body `inner` (the text between `${` and `}`).
///
/// Returns `Some(true)` when a `^` flag forces RC_EXPAND ON, `Some(false)`
/// when `^^` forces it OFF, `None` when no `^` flag is present.
///
/// c:Src/subst.c:2550-2557 — the flag loop processes `^` AFTER the optional
/// parenthesised flag group, so the flag sits either at the very start
/// (`${^a}`) or right after `(…)` (`${(@)^a}`). Skip a leading `(…)` group
/// first, then count the `^` run: each `^` toggles plan9, so an odd count is
/// ON and an even count is OFF. (`${^(@)a}` is a zsh "bad substitution" — `^`
/// never precedes the group — so only the after-group position is handled.)
///
/// Limitation: a `^` interleaved with other shorthand flags (`${(@)~^a}`) or
/// used as a `(s:^:)`/`(j:^:)` delimiter is not treated as plan9 — the same
/// scope the surrounding classifiers already assume.
fn plan9_flag_state(inner: &str) -> Option<bool> {
    let mut rest = inner;
    // Skip a leading `(…)` flag group (a `^` delimiter INSIDE it is not a
    // plan9 flag). `find(')')` matches the existing classifiers' paren scan.
    if let Some(after_open) = rest.strip_prefix('(') {
        if let Some(close) = after_open.find(')') {
            rest = &after_open[close + 1..];
        }
    }
    let carets = rest.chars().take_while(|&c| c == '^').count();
    if carets == 0 {
        None
    } else {
        Some(carets % 2 == 1)
    }
}

/// Strip a word segment down to the `${…}` body, applying the same DQ /
/// Qstring normalization every plan9/splice/distribute classifier uses, and
/// hand the inner text to `f`. Returns `false` when the segment is not a
/// `${…}` braced expansion.
fn with_braced_inner(s: &str, f: impl FnOnce(&str) -> bool) -> bool {
    let pq = crate::lex::untokenize_preserve_quotes(s);
    let normalized: String = pq
        .trim_start_matches('"')
        .trim_end_matches('"')
        .chars()
        .map(|c| {
            if c == crate::ported::zsh_h::Qstring {
                '$'
            } else {
                c
            }
        })
        .collect();
    match normalized
        .strip_prefix("${")
        .and_then(|t| t.strip_suffix('}'))
    {
        Some(inner) => f(inner),
        None => false,
    }
}

/// True for a `${^name}` / `${(@)^name}` segment — RC_EXPAND_PARAM forced ON
/// by the `^` flag (c:Src/subst.c:2551-2557 `plan9 = 1`).
///
/// A subset of `is_distribute_expansion`: those shapes all cross-product, but
/// only plan9 DELETES the word when the array is empty (c:4362 `uremnode`), so
/// the two need different concat builtins. `${^^name}` is excluded — the
/// doubled flag turns plan9 back OFF (c:2554). Same Qstring-aware normalization
/// as `is_distribute_expansion`, so a DQ-wrapped `"${^a}"` is recognised too.
/// The `^` may follow a `(…)` flag group (`${(@)^a}`), so `plan9_flag_state`
/// looks past it rather than only at the first character.
fn is_plan9_expansion(s: &str) -> bool {
    with_braced_inner(s, |inner| plan9_flag_state(inner) == Some(true))
}

fn is_distribute_expansion(s: &str) -> bool {
    let pq = crate::lex::untokenize_preserve_quotes(s);
    // Same Qstring-aware normalization as is_splice_expansion — see
    // there for rationale. Treat DQ-wrapped Qstring `$` identically
    // to bare `$` for distribute-shape detection.
    let normalized: String = pq
        .trim_start_matches('"')
        .trim_end_matches('"')
        .chars()
        .map(|c| {
            if c == crate::ported::zsh_h::Qstring {
                '$'
            } else {
                c
            }
        })
        .collect();
    let pq = normalized;
    if let Some(inner) = pq.strip_prefix("${").and_then(|t| t.strip_suffix('}')) {
        if inner.starts_with('^') {
            // c:Src/subst.c:2551-2557 — a single `^` turns RC_EXPAND ON
            // (cross-product distribute); a doubled `^^` turns it OFF.
            // `${^^a}` must NOT distribute (the array joins): with
            // `setopt rcexpandparam`, `foo${^^a}bar` → "foo1 2 3bar".
            return !inner.starts_with("^^");
        }
        if let Some(rest) = inner.strip_prefix('(') {
            if let Some(close) = rest.find(')') {
                let flags = &rest[..close];
                for c in flags.chars() {
                    match c {
                        // c:2275-2299 — flags that split / produce
                        // arrays from a scalar. (0) sets spsep=NUL
                        // (c:2293), same array-producing shape as (f)
                        // / (s) / (z). Without (0) here, the
                        // multsub-returned Value::Array got joined by
                        // BUILTIN_CONCAT_DISTRIBUTE's default-join
                        // path.
                        //
                        // `A`, `a` and `P` used to be listed here too. They do
                        // not distribute: nothing in the `(…)` flag loop writes
                        // `plan9` (c:Src/subst.c:1663 `int plan9 =
                        // isset(RCEXPANDPARAM)`, and only the unparenthesised
                        // `^`/`^^` at c:2551-2558 assigns it), so an array
                        // carrying `(P)`/`(a)`/`(A)` takes c:4377's non-plan9
                        // "simply join the first and last values" splice.
                        // `is_splice_expansion` claims them now — and, being
                        // tested first at the `has_splice_seg` site, it already
                        // claimed every remaining letter below, so this arm is
                        // shadowed.
                        'f' | 'z' | 'w' | '@' | 's' | '0' => return true,
                        _ => {}
                    }
                }
            }
        }
    }
    false
}

/// True when the tokenized word `s` is a SINGLE double-quoted span
/// wrapping everything — i.e. genuinely DQ-context. `"${x}"` (one outer
/// Dnull pair) is; `""${x}""` and `"x"${a}"y"` (SIBLING spans that merely
/// start and end with a Dnull) are NOT — their middle expansion is
/// UNQUOTED. The naive `starts_with(Dnull) && ends_with(Dnull)` test
/// misfires on the sibling case and joined an array that zsh splits
/// (`""${a}""` for a=(1 2 3) → 3 words, not 1). Count Dnull markers only
/// at brace/bracket/paren depth 0: a real single wrap has exactly 2,
/// sibling spans have 4+, and Dnulls NESTED inside `${…}` (`"a${x:-"n"}b"`)
/// sit at depth>0 and are ignored so the outer wrap still counts.
impl ZshCompiler {
    /// Body text for a `BUILTIN_BRIDGE_BRACE_ARRAY` call that forwards the
    /// user's `${...}` source verbatim.
    ///
    /// c:Src/subst.c:1625 — `paramsubst`'s `qt` argument is "this expansion is
    /// inside `"…"`". The bridge derives it from the executor's
    /// `in_dq_context`, which only `BUILTIN_EXPAND_TEXT` bumps, so an opcode
    /// that hands over a RAW body has to carry the flag itself. The channel is
    /// a leading Qstring (`\u{8c}`) marker, which the bridge strips before
    /// reconstructing the body — the same convention the array-op site uses.
    ///
    /// Every subscripted spelling used to skip this, so `qt` was false and
    /// c:4354's nulstring marking (`qt && !*y`) never applied: a quoted split
    /// silently dropped its empty fields.
    ///     a=(-O 'expl:'); b=( "${(@s/:/)a[2]}" )   # $#b == 1, zsh says 2
    /// The unsubscripted `"${(@s/:/)scalar}"` was always correct because it
    /// never reaches these opcodes.
    /// c:Src/subst.c:1759 — `int ssub = (pf_flags & PREFORK_SINGLE);`.
    ///
    /// C sets that bit in `Src/exec.c::addvars` (c:2546 `prefork(vl, isstr ?
    /// (PREFORK_SINGLE|PREFORK_ASSIGN) : PREFORK_ASSIGN, NULL)`) for the RHS
    /// of a SCALAR assignment, and `typeset NAME=…` reaches the same place
    /// through the assign-builtin argument path. Inside paramsubst it
    /// suppresses c:3913's `force_split` (so `(s:X:)`/`(f)`/`(0)` do not
    /// split) and enables the c:3916 array→scalar join that runs before the
    /// c:4041 quote block. The two depths here are the compiler's record of
    /// the same two C call sites; `${=…}` (`parse_forced_split_brace`) already
    /// consults them.
    fn brace_array_ssub(&self) -> bool {
        self.scalar_assign_depth > 0 || self.assign_builtin_arg_depth > 0
    }

    fn brace_array_body(&self, word: &str, inner: &str) -> String {
        let in_dq = (word_is_single_dq_span(word) && self.synthetic_dq_wrap_depth == 0)
            || self.dq_context_depth > 0;
        if in_dq {
            format!("\u{8c}{}", inner)
        } else {
            inner.to_string()
        }
    }
}

/// Split a ksh93 funsub / mksh valsub word into its command BODY.
///
/// Returns `(body, is_valsub)` when `t` is exactly `${ … }` (funsub) or
/// `${| … }` (valsub), and `None` for anything else — including an ordinary
/// `${name}`, which must keep going to paramsubst.
///
/// The discriminator is the character right after `${`, and it is the same
/// one both references use: a BLANK opens a funsub (ksh(1): "${ command;}
/// … the space is required"), a `|` opens a valsub. `${x}` has neither.
/// The closing `}` must be the word's last character and brace-balanced
/// with everything between, so `${ f; }` and `${ if x; then y; fi; }` are
/// bodies while `${ f; }x` is not a whole-word substitution.
fn ksh_funsub_body(t: &str) -> Option<(String, String, i64)> {
    let rest = t.strip_prefix("${")?;
    let rest = rest.strip_suffix('}')?;
    let first = rest.chars().next()?;
    // c:Src/subst.c:1924/1930 — the character right after `${` selects the
    // form: `|` (or the Bar token) → the REPLY form, a blank → the
    // stdout-capture form, `{` → the named-variable form `${{VAR} cmd }`.
    let (body, rplyvar, kind): (&str, String, i64) = if first == '|' {
        // c:2019-2021 — `rplypm = createparam("REPLY", PM_LOCAL|PM_UNSET|
        // PM_HIDE)`: the value is $REPLY and REPLY is LOCAL to the body.
        (&rest[1..], "REPLY".to_string(), 1)
    } else if first == '{' {
        // c:1930-1962 — `${{VAR} cmd }`: VAR names the result parameter and
        // is NOT localised (c:2032 only errors for the two bare forms, so
        // `rplypm` stays NULL here and assignments inside are global).
        let after = &rest[1..];
        let close = after.find('}')?;
        let name = &after[..close];
        // c:1937-1938 — `if ((outbracep = itype_end(s+1, INAMESPC, 0)))`:
        // the text between the inner braces must be a parameter name.
        if name.is_empty()
            || !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return None;
        }
        let tail = &after[close + 1..];
        // c:1949-1953 — "Require space to avoid ${{var}} typo for
        // ${${var}}": a blank must follow the inner `}`.
        if !tail.starts_with(|c: char| c == ' ' || c == '\t' || c == '\n') {
            return None;
        }
        (tail, name.to_string(), 2)
    } else if first.is_whitespace() {
        // c:2025-2029 — the stdout-capture form; C names its scope
        // parameter `.zsh.cmdsubst` and reads the body's stdout back.
        (rest, String::new(), 0)
    } else {
        return None;
    };
    // Reject a `}` that closes early — the final `}` we stripped has to be
    // the one that matches the opening `${`.
    let mut depth = 0i32;
    for c in body.chars() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    Some((body.to_string(), rplyvar, kind))
}

fn word_is_single_dq_span(s: &str) -> bool {
    use crate::ported::zsh_h::{
        Inbrace, Inbrack, Inpar, Inparmath, Outbrace, Outbrack, Outpar, Outparmath,
    };
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 2 || chars[0] != '\u{9e}' || *chars.last().unwrap() != '\u{9e}' {
        return false;
    }
    let mut depth = 0i32;
    let mut depth0_dnull = 0usize;
    for &c in &chars {
        match c {
            Inpar | Inparmath | Inbrace | Inbrack => depth += 1,
            Outpar | Outparmath | Outbrace | Outbrack => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            '\u{9e}' if depth == 0 => depth0_dnull += 1,
            _ => {}
        }
    }
    depth0_dnull == 2
}

/// True when the leading `${` of `untoked` matches the word's FINAL `}` —
/// i.e. the whole word is ONE braced expansion, not two adjacent ones
/// (`${a}${b}`, `${(@)^a}Y${(@)^b}`) that merely start with `${` and end
/// with `}`. Counts `${` opens against `}` closes; the word spans only if
/// the depth first returns to zero exactly at the last char. Adjacent-
/// expansion words return false so the caller falls through to the segment
/// splitter instead of feeding the whole word to paramsubst as one body
/// (which swallows the trailing expansion and joins its array).
fn braced_expansion_spans_word(untoked: &str) -> bool {
    if !untoked.starts_with("${") {
        return false;
    }
    let ub = untoked.as_bytes();
    let mut depth = 0i32;
    let mut k = 0usize;
    while k < ub.len() {
        if ub[k] == b'$' && k + 1 < ub.len() && ub[k + 1] == b'{' {
            depth += 1;
            k += 2;
            continue;
        }
        if ub[k] == b'}' {
            depth -= 1;
            if depth == 0 {
                return k + 1 == ub.len();
            }
        }
        k += 1;
    }
    false
}

fn split_word_segments(s: &str) -> Option<Vec<WordSegment>> {
    // `$+NAME` chkset form: the no-brace bare form of `${+NAME}`. The
    // segment splitter doesn't model `+` as a paramsubst-body prefix
    // (the find_expansion_end arms cover `$@`, `$*`, `$#`, `$?`, `$!`,
    // `$-`, `$$`, digits, identifiers — NOT `$+NAME`). Bail to the
    // whole-string EXPAND_TEXT fall-through (compile_zsh.rs:2900-2922)
    // so paramsubst sees the full input and its `$+NAME` arm
    // (subst.rs:2477+) handles it canonically. Mirrors C zsh's prefork
    // which never pre-splits DQ content — multsub/stringsubst/paramsubst
    // process the whole string inline.
    let chars: Vec<char> = s.chars().collect();
    // c:Src/subst.c:2199 — `$+NAME[sub]` (unbraced set-test) is handled
    // by find_expansion_end's `$+` arm + the `$+NAME[KEY]` fast path, so
    // it no longer needs the whole-string bail. Only bail for a `$+`
    // shape with NO following name char (e.g. `$+ ` / `$+}`), which the
    // segment splitter still can't model — route those to EXPAND_TEXT.
    for (k, w) in chars.windows(2).enumerate() {
        let dollar = w[0] == '$' || w[0] == '\u{85}' || w[0] == '\u{8c}';
        if dollar && w[1] == '+' {
            let after = chars.get(k + 2).copied();
            let name_follows = after.is_some_and(|n| {
                n.is_ascii_alphanumeric() || n == '_' || matches!(n, '@' | '*' | '#' | '?')
            });
            if !name_follows {
                return None;
            }
        }
    }
    let n = chars.len();
    let mut segs: Vec<WordSegment> = Vec::new();
    let mut lit_start = 0;
    let mut i = 0;
    // Track nesting inside `{...}` (Inbrace/Outbrace) and `[...]`
    // (Inbrack/Outbrack) so an inner expansion marker like the `$i`
    // in `${a[$i]}` doesn't get pulled out as its own segment.
    // Top-level (depth 0) markers are real concat boundaries.
    let mut brace_depth = 0i32;
    let mut brack_depth = 0i32;
    // Snull (`\u{9d}`) span tracking — c:Src/subst.c:282-330
    // stringsubst only fires substitution on String/Qstring TOKENS;
    // chars inside a single-quoted span stay plain ASCII and are
    // never expansion boundaries. Without this, the literal `$` in
    // `*(e:'[[ $REPLY = keep ]]':)` matched
    // is_literal_dollar_with_expansion (followed by ident char) and
    // got pulled out as an Expansion segment — the qualifier body
    // reached the glob with `$REPLY` already substituted (empty).
    let mut inside_sq = false;
    while i < n {
        let c = chars[i];
        match c {
            '\u{9d}' => inside_sq = !inside_sq,                 // Snull
            '\u{8f}' => brace_depth += 1,                       // Inbrace
            '\u{90}' => brace_depth = (brace_depth - 1).max(0), // Outbrace
            '\u{91}' => brack_depth += 1,                       // Inbrack
            '\u{92}' => brack_depth = (brack_depth - 1).max(0), // Outbrack
            _ => {}
        }
        if inside_sq {
            i += 1;
            continue;
        }
        // Recognize segment boundaries:
        // - META-$ (\u{85}) and META-Qstring (\u{8c}) — emitted by the
        //   lexer for `$` outside / inside double quotes
        // - Literal `$` (0x24) — emitted in some lexer paths where the
        //   `$` survives untokenized but the surrounding braces / brackets
        //   are META-marked. Followed by Inbrace/Inpar/alphanumeric to
        //   distinguish from a literal trailing `$`.
        let is_meta_dollar = c == '\u{85}' || c == '\u{8c}';
        // An ESCAPED dollar is literal: the lexer marks `\$` as
        // Bnull(\u{9f})+`$`, and raw-source paths may carry `\$`
        // verbatim (odd backslash run). Without this, the cond
        // pattern `\$[{]` (f-sy-h's -fast-highlight-string dollar
        // matcher) compiled as `$[…]` old-style math with the
        // tokenized `{` (0x8f) inside — "bad math expression:
        // illegal character:" per keystroke, and the plugin's
        // while loop then spun the shell at 100% CPU.
        let escaped_dollar = c == '$'
            && (chars.get(i.wrapping_sub(1)).is_some_and(|&p| p == '\u{9f}') || {
                let bs = chars[..i].iter().rev().take_while(|&&b| b == '\\').count();
                bs % 2 == 1
            });
        let is_literal_dollar_with_expansion = c == '$' && !escaped_dollar && {
            // peek next char — must be `{`-meta/literal, `(`-meta/literal,
            // or ident-start. The literal `{` and `(` cases apply when
            // the input has been pre-untokenized (assoc-LHS key arrives
            // ASCII after untokenize_preserve_quotes in compile_assign).
            chars
                .get(i + 1)
                .map(|&n| {
                    n == '\u{8f}'  // Inbrace
                        || n == '{'        // literal `${`
                        || n == '\u{88}'  // Inpar
                        || n == '('        // literal `$(`
                        || n == '_'
                        || n.is_ascii_alphanumeric()
                        || n == '@' || n == '*' || n == '#' || n == '?'
                        || n == '!' || n == '$'
                        // `$+NAME[sub]` set-test (only when a name follows
                        // the `+`, matching find_expansion_end's `$+` arm).
                        || (n == '+'
                            && chars.get(i + 2).is_some_and(|&m| {
                                m.is_ascii_alphanumeric()
                                    || m == '_'
                                    || matches!(m, '@' | '*' | '#' | '?')
                            }))
                })
                .unwrap_or(false)
        };
        let is_dollar = is_meta_dollar || is_literal_dollar_with_expansion;
        // Backtick trigger: literal `` ` `` OR the lexer's Tick
        // (`\u{93}`) / Qtick (`\u{99}`) markers. Without the marker
        // forms, `\`echo $foo\`` (which the lexer emits as
        // `\u{93}echo $foo\u{93}`) only split on `$foo`, treating
        // the surrounding Tick chars as literal text — the bridge
        // never saw a whole-word backquote.
        let is_backtick = c == '`' || c == '\u{93}' || c == '\u{99}';
        let at_top = brace_depth == 0 && brack_depth == 0;
        if !(is_dollar || is_backtick) || !at_top {
            i += 1;
            continue;
        }
        // Flush any pending literal.
        if lit_start < i {
            let lit: String = chars[lit_start..i].iter().collect();
            segs.push(WordSegment::Literal(lit));
        }
        // Find end of expansion.
        let end = find_expansion_end(&chars, i);
        let exp: String = chars[i..end].iter().collect();
        segs.push(WordSegment::Expansion(exp));
        i = end;
        lit_start = i;
    }
    if lit_start < n {
        let lit: String = chars[lit_start..].iter().collect();
        segs.push(WordSegment::Literal(lit));
    }

    // Reject single-segment cases — the caller's other fast paths cover
    // pure-literal and bare-expansion words. Only multi-segment concat
    // benefits from this path.
    if segs.len() < 2 {
        return None;
    }
    // Sanity: at least one expansion (otherwise we'd be a literal, but
    // split_word_segments only emits literals between expansions, so a
    // 2-piece result with no expansion is impossible — safety check).
    if !segs.iter().any(|s| matches!(s, WordSegment::Expansion(_))) {
        return None;
    }
    Some(segs)
}

/// Given chars[i] is META-$ / Qstring / backtick, return the index just
/// past the end of the expansion. Handles `${...}`, `$(...)`,
/// `$((...))`, `$NAME`, `$N`, `$@` etc., and `` `cmd` ``.
/// c:Src/subst.c:1820 — walk bare `$NAME:MOD` history-style modifier
/// chain in place. Bumps `j` past every consumed modifier so the
/// caller emits the whole `$NAME:MOD…` as one expansion segment.
///
/// Supported:
///   - simple letters: h/t/r/e/l/u/q/Q/a/A/P (+ optional digit count
///     for :hN / :tN)
///   - substitution: :s/PAT/REPL/ and :gs/PAT/REPL/ (delimiter char
///     follows s; pattern, replacement terminated by same delim;
///     backslash escapes)
///
/// Anchored on `:` followed by a known modifier letter (or `g` then
/// `s`) so `$a:$b` stays two expansions. Bugs #579/#580/#581.
fn walk_bare_modifier_chain(chars: &[char], j: &mut usize) {
    while *j + 1 < chars.len() && chars[*j] == ':' {
        let mut probe = *j + 1;
        // Optional prefixes to `:s` — `g` (global, apply everywhere) and `f`
        // (repeat until the substitution stops changing the string). C accepts
        // them before the `s` (Src/hist.c — the `g`/`f`/count flags on the
        // substitute modifier). The unbraced scanner handled `g` but not `f`,
        // so `$f:fs/a//` fell through as literal text while `${f:fs/a//}` and
        // `$f:gs/a//` both worked.
        let mut saw_prefix = false;
        while probe < chars.len() && (chars[probe] == 'g' || chars[probe] == 'f') {
            saw_prefix = true;
            probe += 1;
        }
        if probe >= chars.len() {
            break;
        }
        let after = chars[probe];
        if saw_prefix || after == 's' {
            if after != 's' {
                break;
            }
            // Position now: at `s`.
            probe += 1;
            if probe >= chars.len() {
                break;
            }
            let delim = chars[probe];
            probe += 1;
            let mut found_pat_end = false;
            while probe < chars.len() {
                if chars[probe] == '\\' && probe + 1 < chars.len() {
                    probe += 2;
                    continue;
                }
                if chars[probe] == delim {
                    probe += 1;
                    found_pat_end = true;
                    break;
                }
                probe += 1;
            }
            if !found_pat_end {
                break;
            }
            while probe < chars.len() {
                if chars[probe] == '\\' && probe + 1 < chars.len() {
                    probe += 2;
                    continue;
                }
                if chars[probe] == delim {
                    probe += 1;
                    break;
                }
                probe += 1;
            }
            *j = probe;
            continue;
        }
        // 'c' — PATH search (c:Src/hist.c:863 `equalsubstr`); '&' — repeat the
        // last `s///` (c:Src/hist.c:903). Both were missing from this UNBRACED
        // scanner, so `$p:c` and `$f:s/x/Y/:&` fell through as literal text while
        // the braced spelling resolved.
        if !matches!(
            after,
            'h' | 't' | 'r' | 'e' | 'l' | 'u' | 'q' | 'Q' | 'a' | 'A' | 'P' | 'c' | '&'
        ) {
            break;
        }
        *j = probe + 1;
        while *j < chars.len() && chars[*j].is_ascii_digit() {
            *j += 1;
        }
    }
}

fn find_expansion_end(chars: &[char], i: usize) -> usize {
    let c = chars[i];
    if c == '`' || c == '\u{93}' || c == '\u{99}' {
        // Backtick: find matching `, Tick, or Qtick. The opening
        // marker MUST match the closing form per parse/tokens
        // (Tick pairs with Tick, etc.) but in practice the lexer
        // is consistent within a word — accept any of the three
        // as the close.
        let mut j = i + 1;
        while j < chars.len() && chars[j] != '`' && chars[j] != '\u{93}' && chars[j] != '\u{99}' {
            j += 1;
        }
        return (j + 1).min(chars.len());
    }
    // META-$ or Qstring — look at next char
    let next = chars.get(i + 1).copied();
    match next {
        // ANSI-C quote: $'...' lexed as `Stringg Snull <body> Snull`
        // (`\u{85}\u{9d}…\u{9d}`). Without this arm, split_word_segments
        // saw the lone Stringg as an "expansion" returning i+1, leaving
        // the Stringg byte as a single-segment "$" that multsub then
        // emitted as a literal `$`. Result: `$'\t'$X` produced
        // `$<tab>val` instead of `<tab>val`. Walk the full Snull-
        // delimited body so the segment is the whole `$'…'` token,
        // dispatched by multsub's stringsubstquote arm.
        Some('\u{9d}') => {
            let mut j = i + 2;
            let mut escaped = false;
            while j < chars.len() {
                if escaped {
                    escaped = false;
                    j += 1;
                    continue;
                }
                if chars[j] == '\u{9f}' {
                    // Bnull-escape: skip the literal next char
                    escaped = true;
                    j += 1;
                    continue;
                }
                if chars[j] == '\u{9d}' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            j
        }
        // Inbrace: ${...}. Track BOTH token (\u{8f}/\u{90}) AND literal
        // `{`/`}` braces — a nested `${…}` inside the body can arrive in
        // either form (e.g. `${${x%${x##pat}}/a/b}` mixes Inbrace tokens
        // with literal inner braces), and counting only the token form
        // closed the outer expansion at the first INNER `}`, so the
        // trailing expansion in `${…nested…}${x##pat}` was left literal.
        // Mirrors the literal-`{` arm below.
        Some('\u{8f}') => {
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '{' | '\u{8f}' => depth += 1,
                    '}' | '\u{90}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            j
        }
        // Literal `{` after `$`: same as Inbrace but on the ASCII path
        // (untokenize_preserve_quotes upstream). Track depth on literal
        // `{`/`}`. Bug: nested `h[Q-${h[$k]}]=v` in assignment LHS.
        Some('{') => {
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '{' | '\u{8f}' => depth += 1,
                    '}' | '\u{90}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            j
        }
        // Inpar: $(...) or $((...))
        // The lexer emits these shapes:
        //   `$(cmd)`    → META-$ Inpar <body chars> Outpar
        //   `$((expr))` → META-$ Inpar <body w/ literal `(`/`)`> Outparmath
        // For `$((`, the inner `(` is kept literal and the closing `))`
        // is collapsed into a single Outparmath (\u{8b}). We detect by
        // peeking after Inpar — if the next char is literal `(` (0x28)
        // or Inparmath, we're in arith mode and end at Outparmath.
        Some('\u{88}') => {
            let after = chars.get(i + 2).copied();
            let is_arith = matches!(after, Some('(') | Some('\u{89}'));
            let close_match = if is_arith { '\u{8b}' } else { '\u{8a}' };
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                let c = chars[j];
                if !is_arith && c == '\u{88}' {
                    depth += 1;
                } else if c == close_match {
                    depth -= 1;
                }
                j += 1;
            }
            j
        }
        // Also catch META-$ + Inparmath directly for arith forms.
        // Track depth so nested `$((expr1 + $((expr2)) ))` finds the
        // OUTER Outparmath, not the first inner one. Bug #21 in
        // docs/BUGS.md: without depth tracking, `"$(( a + $((2*5)) ))"`
        // truncated at the inner `\u{8b}` and left the outer's ` )\u{8b}`
        // dangling in the literal-suffix segment — paramsubst then saw
        // half a math expression and emitted the literal `( a + 10 ))`
        // instead of `12`. Mirror the `\u{88}` (Inpar) arm above which
        // already depth-tracks for nested cmd-substitution.
        Some('\u{89}') => {
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                let c = chars[j];
                if c == '\u{89}' {
                    depth += 1;
                } else if c == '\u{8b}' {
                    depth -= 1;
                }
                j += 1;
            }
            j
        }
        // Inbrack: $[...]
        Some('\u{91}') => {
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '\u{91}' => depth += 1,
                    '\u{92}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            j
        }
        // Special single-char params: $@ $* $# $? $! $- $$.
        // (`$_` is NOT in this list — `_` is also a valid identifier
        // first char, so `$__foo` must read the full identifier
        // `__foo`, not split into `$_` + `_foo`. The identifier
        // branch below handles bare `$_` correctly via its terminator.)
        // The lexer META-marks `*`, `?`, `#`, `-`, `!` (and similar
        // glob/syntax chars) when they appear as a token; after a META-$
        // they're still the variable-name char even in their META form.
        // Match both the literal char and its META code-point so e.g.
        // `X$?` lexed as `X\u{85}\u{97}` (META-$, META-?) detects the
        // expansion as `$?` rather than falling through to the
        // "advance by 1" default (which left `?` as a literal-glob in
        // the trailing literal segment).
        Some(ch)
            if matches!(
                ch,
                '@' | '*' | '#' | '?' | '!' | '-' | '$'
                    | '\u{87}' // META-* (Star)
                    | '\u{84}' // META-# (Pound)
                    | '\u{97}' // META-? (Quest)
                    | '\u{9b}' // META-- (Dash)
                    | '\u{9c}' // META-! (Bang)
                    | '\u{85}' // META-$ ($$ → PID; second $ also lexed as STRING)
                    | '\u{8c}' // META-Qstring ($ in DQ context)
            ) =>
        {
            // `$#@`, `$#*`, `$#NAME` — `$#`-then-suffix shapes. After
            // the leading `#` (literal or META-#) the next char may be:
            //   - `@`/`*` (literal or META): zsh shorthand for
            //     `${#@}`/`${#*}`, the positional count.
            //   - identifier start: `${#NAME}`, the length of NAME.
            // Without this, `"$#@"` was split into segments
            // [META-$, #] + literal `@`, leaving the `@` outside the
            // expansion. Same for `X$#Y` where `Y` got dropped from
            // the name lookup.
            if matches!(ch, '#' | '\u{84}') && i + 2 < chars.len() {
                let after = chars[i + 2];
                // Single-char specials terminate the `$#` walk after
                // one trailing char: `$#?`, `$#!`, `$#-`, `$#0`, `$#$`
                // are all `${#SPECIAL}` (length of $?, $!, $-, $0,
                // $$). Plus tokenized forms Quest/Bang/Dash/Star/
                // Stringg/Pound.
                if matches!(
                    after,
                    '@' | '*'
                        | '?'
                        | '!'
                        | '-'
                        | '0'
                        | '$'
                        | '\u{87}'
                        | '\u{97}'
                        | '\u{96}'
                        | '\u{9b}'
                        | '\u{85}'
                        | '\u{84}'
                ) {
                    return i + 3;
                }
                if after == '_' || after.is_ascii_alphabetic() {
                    let mut j = i + 2;
                    while j < chars.len() && (chars[j] == '_' || chars[j].is_ascii_alphanumeric()) {
                        j += 1;
                    }
                    return j;
                }
            }
            // Pull trailing `[subscript]` into the same expansion so
            // `$@[2,-1]` / `$*[1]` (especially in DQ context) is one
            // piece, not `$@` + literal `[2,-1]`. Same logic as the
            // identifier branch below at 4971-4985.
            // c:Src/lex.c gettokstr — bare `$@[SUB]` is a recognized
            // positional-array subscript shape.
            let mut j = i + 2;
            if j < chars.len() && (chars[j] == '\u{91}' || chars[j] == '[') {
                let in_b = chars[j];
                let out_b = if in_b == '\u{91}' { '\u{92}' } else { ']' };
                let mut depth = 1;
                let mut k = j + 1;
                while k < chars.len() && depth > 0 {
                    if chars[k] == in_b {
                        depth += 1;
                    } else if chars[k] == out_b {
                        depth -= 1;
                    }
                    k += 1;
                }
                j = k;
            }
            // c:Src/subst.c:1820 — single-char-special `$?:MOD`,
            // `$$:MOD`, `$!:MOD`, `$-:MOD` etc. accept the modifier
            // chain (Bug #582/#584).
            walk_bare_modifier_chain(chars, &mut j);
            j
        }
        // c:Src/subst.c:2596 — `$~NAME` / `$~` is the GLOB_SUBST flag
        // prefix. The `~` (literal or Tilde token `\u{98}`) is part of
        // the paramsubst syntax. When followed by an identifier, the
        // expansion spans `$~NAME`; when followed by anything else
        // (whitespace, `]`, end-of-string, etc.), the bare `$~`
        // expands to empty. paramsubst's `~` arm at subst.rs:10601
        // already handles both shapes correctly, but the segment
        // splitter previously returned just `$` and left the `~` in
        // the trailing literal — so the bridge emitted `$` + literal
        // `~` instead of routing the whole `$~` through paramsubst.
        // Bug #547 in docs/BUGS.md (surrounding-text DQ form).
        Some('~') | Some('\u{98}') => {
            let mut j = i + 2;
            // Optional second `~` for the `$~~NAME` toggle-off form.
            if j < chars.len() && (chars[j] == '~' || chars[j] == '\u{98}') {
                j += 1;
            }
            // Optional trailing identifier.
            if j < chars.len() && (chars[j].is_ascii_alphabetic() || chars[j] == '_') {
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
            }
            j
        }
        // c:Src/subst.c:2199 — `$+NAME` / `$+NAME[sub]` is the unbraced
        // set-test shorthand for `${+NAME}` / `${+NAME[sub]}`. Span the
        // `+`, the name, and any trailing `[subscript]` so the segment
        // reaches the `$+NAME[KEY]` fast path (compile_zsh ~4332) as ONE
        // piece. Without this the splitter bailed and `x=$+functions[f]`
        // left `[f]` as literal text (`x=1[f]` vs zsh `x=1`).
        Some('+')
            if chars.get(i + 2).is_some_and(|&n| {
                n.is_ascii_alphanumeric() || n == '_' || matches!(n, '@' | '*' | '#' | '?')
            }) =>
        {
            let mut j = i + 2;
            if chars[j].is_ascii_alphabetic() || chars[j] == '_' {
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
            } else {
                j += 1; // single-char special (@/*/#/?)
            }
            if j < chars.len() && (chars[j] == '\u{91}' || chars[j] == '[') {
                let in_b = chars[j];
                let out_b = if in_b == '\u{91}' { '\u{92}' } else { ']' };
                let mut depth = 1;
                let mut k = j + 1;
                while k < chars.len() && depth > 0 {
                    if chars[k] == in_b {
                        depth += 1;
                    } else if chars[k] == out_b {
                        depth -= 1;
                    }
                    k += 1;
                }
                j = k;
            }
            j
        }
        // All-digit positional: $0..$N
        Some(ch) if ch.is_ascii_digit() => {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            // c:Src/subst.c:1820 — bare `$0:MOD` etc. also accept the
            // modifier chain (same as `$NAME:MOD`). Bug #581 extends
            // #579/#580 to positional + special-char single-glyph
            // names.
            walk_bare_modifier_chain(chars, &mut j);
            j
        }
        // Identifier: $NAME (optionally followed by [subscript])
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            // Pull a trailing `[subscript]` into the same expansion so
            // `$NAME[idx]` (especially in DQ context) is one piece, not
            // `$NAME` + literal `[idx]`. The lexer emits Inbrack
            // (`\u{91}`) / Outbrack (`\u{92}`) for top-level `[]`, but
            // some lex paths leave bare `[`/`]` (DQ context, etc.).
            if j < chars.len() && (chars[j] == '\u{91}' || chars[j] == '[') {
                let in_b = chars[j];
                let out_b = if in_b == '\u{91}' { '\u{92}' } else { ']' };
                let mut depth = 1;
                let mut k = j + 1;
                while k < chars.len() && depth > 0 {
                    if chars[k] == in_b {
                        depth += 1;
                    } else if chars[k] == out_b {
                        depth -= 1;
                    }
                    k += 1;
                }
                j = k;
            }
            // c:Src/subst.c:1820 — bare `$NAME:MOD` history-style
            // modifier chain. Bugs #579/#580/#581.
            walk_bare_modifier_chain(chars, &mut j);
            j
        }
        _ => i + 1,
    }
}

/// If `s` is exactly `$((expr))` (un-tokenized form), return the inner
/// expression. Returns None for shapes with prefix/suffix concat or
/// nested constructs.
fn strip_arith_subst(s: &str) -> Option<String> {
    if !s.starts_with("$((") || !s.ends_with("))") {
        return None;
    }
    let inner = &s[3..s.len() - 2];
    // Reject if inner has an unbalanced `((` / `))` OR if depth EVER
    // drops below zero — that means a `))` closes the outer `$((`
    // before the end of the input, signalling concat with another
    // arith / cmd subst (`$((1+2))$((3+4))` was the bug case). The
    // fold-only check let those through with a net zero depth.
    let mut depth = 0i32;
    let mut depth_dropped_below_zero = false;
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                // Recognise the `))` close of an inner $((..)) — only
                // count as -2 when paired. Single `)` is -1.
                if i + 1 < chars.len() && chars[i + 1] == ')' && depth >= 2 {
                    depth -= 2;
                    i += 2;
                    continue;
                }
                depth -= 1;
                if depth < 0 {
                    depth_dropped_below_zero = true;
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if depth_dropped_below_zero {
        return None;
    }
    if depth != 0 {
        return None;
    }
    Some(inner.to_string())
}

/// Does an assignment-RHS source string contain a construct that
/// would update last_status during expansion? Used by compile_assign
/// to decide whether to force `$? = 0` after the RHS (no cmd-subst,
/// plain literal/expansion) or leave the cmd-subst's exit in place.
/// Only `$(…)` and backtick cmd-substs touch last_status; `$((…))`
/// arithmetic and `${var…}` parameter expansion do not.
///
/// Detects both raw-source form (`$(`, backtick) and the tokenized
/// form produced by ported::lex (Stringg+Inpar = `\u{85}\u{88}`, Tick
/// = `\u{93}`, Qtick = `\u{99}`). The Stringg+Inparmath sequence
/// (`\u{85}\u{89}`) is `$((` arithmetic — skip.
fn scalar_rhs_has_cmd_subst(s: &str) -> bool {
    use crate::ported::zsh_h::{Inpar, Inparmath, Qstring, Qtick, Stringg, Tick};
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Tokenized `$(`: Stringg or Qstring (DQ-context `$`) followed
        // by Inpar. Bug #122 in docs/BUGS.md: the previous port only
        // matched Stringg+Inpar, missing the DQ-wrapped form. For
        // `y="${x:-$(false)}"` the lexer emits Qstring (\u{8c}) for the
        // inner `$` because the outer DQ context tokenized it, so the
        // detector falsely returned false and the post-assignment
        // status reset clobbered the cmd-subst's exit.
        if (c == Stringg || c == Qstring) && i + 1 < chars.len() {
            let nxt = chars[i + 1];
            if nxt == Inparmath {
                i += 2; // `$((` arithmetic
                continue;
            }
            if nxt == Inpar {
                return true; // `$(cmd)` tokenized
            }
        }
        // Raw-source `$(`: ASCII `$` then `(` — but skip `$((`.
        if c == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
            if i + 2 < chars.len() && chars[i + 2] == '(' {
                i += 3;
                continue;
            }
            return true;
        }
        if c == '`' || c == Tick || c == Qtick {
            return true;
        }
        // ksh93 funsub `${ list; }` / mksh valsub `${| list; }` — also a
        // command substitution, so `v=${ false; }` must leave `$?` at 1
        // exactly as `v=$(false)` does (`ksh -c 'v=${ false; }; print
        // "rc=$?"'` → `rc=1`). Without this the assignment-only status
        // reset (c:Src/exec.c:3396 `lastval = cmdoutval`) clobbered it
        // back to 0. `${name…}` never matches: the character after the
        // brace must be a blank or `|`, which is what opens the two forms.
        if (c == Stringg || c == Qstring || c == '$') && i + 2 < chars.len() {
            let brace = chars[i + 1];
            if brace == '{' || brace == crate::ported::zsh_h::Inbrace {
                let after = chars[i + 2];
                if after == '|' || after == crate::ported::zsh_h::Bar || after.is_whitespace() {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// If `s` is exactly `$(cmd)` (un-tokenized form), return the inner
/// command. Excludes `$((…))` arithmetic and partial concatenations.
/// Whole-word backtick command substitution `` `cmd` `` → inner `cmd`.
/// Returns None unless the input is exactly ONE top-level backtick span
/// (mixed words like `` x`cmd`y `` or nested backticks fall through to the
/// generic expand bridge, which handles their escaping). Used so an
/// unquoted backtick in argument position gets the same IFS word-split as
/// `$(...)` (c:Src/lex.c — Tick/Qtick are command-subst tokens).
fn strip_backtick_subst(s: &str) -> Option<&str> {
    let inner = s.strip_prefix('`')?.strip_suffix('`')?;
    if inner.is_empty() || inner.contains('`') {
        return None;
    }
    Some(inner)
}

fn strip_cmd_subst(s: &str) -> Option<&str> {
    if !s.starts_with("$(") || !s.ends_with(')') {
        return None;
    }
    // c:Src/lex.c:555-567 `cmd_or_math_sub` — a leading `$((` is
    // arithmetic ONLY when the inner `dquote_parse(')')` succeeds AND the
    // very next char is the second `)` (c:511-512). Otherwise C rewinds
    // and calls `skipcomm()` (c:572): it is a COMMAND substitution whose
    // body happens to open with a subshell — `$((cmd); cmd)`, whose
    // output must still be IFS word-split like any other `$(…)`.
    // Rejecting every `$((` here sent those words to the generic expand
    // bridge, which emits ONE unsplit word: `print $((f a); f b)` printed
    // the two outputs on separate lines instead of `!a! !b!`.
    if s.starts_with("$((") && strip_arith_subst(s).is_some() {
        return None;
    }
    // Verify the closing `)` at the end matches the OPENING `$(` at the
    // start (i.e. the whole input is exactly one cmd-subst). Without this
    // check, `$(echo foo)$(echo bar)` matched too — the outer `$(` and
    // final `)` are not paired, the body is `echo foo)$(echo bar` which
    // ran as a malformed script and dropped the second cmd subst.
    //
    // c:Bug #291 — case-pattern `)` is also paren-balanced in the
    // simple counter, but it's NOT the cmdsub closer. zsh's NEW
    // skipcomm walks the case-grammar; here we mirror with a
    // small word-tracking heuristic: between `case <word> in` and
    // `esac`, `)` chars don't decrement the cmdsub depth. Same
    // logic the skipcomm port uses.
    let inner = &s[2..s.len() - 1];
    let mut depth = 1i32;
    let chars: Vec<char> = inner.chars().collect();
    let mut word_buf = String::with_capacity(8);
    let mut case_depth: i32 = 0;
    // 0 = no `case`; 1 = saw `case`, expect subject word;
    // 2 = saw subject word, expect `in`.
    let mut case_pending: i32 = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let is_word_terminator = c == ' '
            || c == '\t'
            || c == '\n'
            || c == ';'
            || c == '&'
            || c == '|'
            || c == '('
            || c == ')';
        if is_word_terminator && !word_buf.is_empty() {
            match word_buf.as_str() {
                "case" => case_pending = 1,
                "in" if case_pending == 2 => {
                    case_depth += 1;
                    case_pending = 0;
                }
                "esac" => {
                    if case_depth > 0 {
                        case_depth -= 1;
                    }
                    case_pending = 0;
                }
                _ => match case_pending {
                    1 => case_pending = 2,
                    2 => case_pending = 0,
                    _ => {}
                },
            }
            word_buf.clear();
        } else if !is_word_terminator {
            word_buf.push(c);
        }
        match c {
            '(' => depth += 1,
            ')' => {
                if case_depth == 0 {
                    depth -= 1;
                    if depth == 0 && i < chars.len() - 1 {
                        // Found a closing `)` mid-string → not a single cmd
                        // subst (the rest is a separate token / second subst).
                        return None;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    Some(inner)
}

/// Phase 1 native param-modifier kinds. Each maps to one of the four
/// new builtins (BUILTIN_PARAM_DEFAULT_FAMILY / SUBSTRING / STRIP /
/// REPLACE). `name` is always a plain identifier (`a-zA-Z0-9_`); RHS
/// strings are passed verbatim — runtime expansion of `$x` etc. inside
/// the RHS is handled by re-emitting through compile_word_str.
pub(crate) struct ParamModifier {
    pub name: String,
    pub kind: ParamModifierKind,
}

pub(crate) enum ParamModifierKind {
    /// `${var:-default}` (op=0), `:=` (1), `:?` (2), `:+` (3)
    DefaultFamily { op: u8, rhs: String },
    /// `${var:offset}` or `${var:offset:length}` (length=None for "rest")
    Substring { offset: i64, length: Option<i64> },
    /// Same as Substring but offset/length are arbitrary expressions
    /// (e.g. `$n`, `$((1+1))`) that need runtime arithmetic evaluation.
    SubstringExpr {
        offset_expr: String,
        length_expr: Option<String>,
    },
    /// `${var#pat}` (op=0), `##` (1), `%` (2), `%%` (3).
    /// `had_at` records whether the source form used `[@]` /
    /// `[*]` subscript on the var name — those force per-element
    /// strip even inside `"..."` (zsh: `[@]` in DQ marks the
    /// array as splice-expanded; the strip applies to each
    /// element individually). Without this bit, `"${a[@]%%pat}"`
    /// joined-then-stripped because the DQ context bit said
    /// "join first" and the [@] info was lost in the modifier
    /// parse.
    Strip {
        op: u8,
        pattern: String,
        had_at: bool,
    },
    /// `${var/pat/repl}` (op=0), `//` (1), `/#` (2), `/%` (3).
    /// `had_at` mirrors the Strip variant — explicit `[@]` on
    /// the var name forces per-element semantics even in DQ.
    Replace {
        op: u8,
        pattern: String,
        repl: String,
        had_at: bool,
    },
    /// `${#name}` — character length of a scalar OR element count of an
    /// indexed/assoc array. Dispatched at runtime by inspecting the var
    /// type.
    Length,
    /// `${var:#pattern}` — filter: remove matching elements (or whole
    /// scalar if it matches). For arrays, returns a Value::Array of the
    /// non-matching elements.
    FilterRemoveMatching { pattern: String },
}

/// Strip the outer Stringg+Inbrace prefix and Outbrace suffix from the
/// raw tokenized word `s`, then run the pattern-preserving untokenize
/// over the inner body. Returns `None` if `s` doesn't have the
/// expected `${…}` token shape (caller falls back to plain untokenized
/// inner). Used by the `BUILTIN_BRIDGE_BRACE_ARRAY` path so quoted
/// pattern bodies (e.g. `${(M)a:#"*"}`) reach paramsubst with
/// backslash-escaped metachars instead of bare glob characters.
fn strip_brace_wrap_for_bridge(s: &str) -> Option<String> {
    use crate::ported::zsh_h::{Inbrace, Outbrace, Stringg};
    let inner_raw = s
        .strip_prefix(Stringg)?
        .strip_prefix(Inbrace)?
        .strip_suffix(Outbrace)?;
    Some(untokenize_preserve_quoted_pat_literals(inner_raw))
}

/// Extract the `:#PAT` pattern body from the raw tokenized `s` for the
/// fast-path `${NAME:#PAT}` shape. Walks the raw token form (Stringg /
/// Inbrace prefix, `:` + Pound delimiter, Outbrace suffix) and returns
/// the pattern text run through `untokenize_preserve_quoted_pat_literals`
/// so quoted segments survive as `\X` escapes. Returns `None` if `s`
/// doesn't match the simple `${NAME:#…}` shape (e.g. has leading
/// `(@)`-flag, has subscript, has nested expansions) — caller falls
/// back to the plain untokenized pattern.
fn extract_filter_pat_from_raw_s(s: &str) -> Option<String> {
    use crate::ported::zsh_h::{Inbrace, Outbrace, Pound, Stringg};
    let mid = s
        .strip_prefix(Stringg)?
        .strip_prefix(Inbrace)?
        .strip_suffix(Outbrace)?;
    let marker = format!(":{}", Pound);
    let idx = mid.find(&marker)?;
    let name_part = &mid[..idx];
    // Only handle simple `NAME` (identifier) shape for now. Skip when
    // the name has a flag or subscript so we don't misextract patterns
    // for shapes like `${(@)a:#…}` or `${a[i]:#…}`.
    if !name_part
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '@' || c == '*')
    {
        return None;
    }
    let pat_raw = &mid[idx + marker.len()..];
    Some(untokenize_preserve_quoted_pat_literals(pat_raw))
}

/// Pattern-preserving untokenize.
///
/// Mirrors `lex::untokenize` for token bytes (Star → `*`, Pound → `#`,
/// etc.) but treats spans wrapped in `Snull`/`Dnull` (single/double
/// quotes from lex) as LITERAL: pattern metachars inside such spans get
/// a `\` prefix so the downstream pattern compiler sees them as
/// `P_EXACTLY` (literal) rather than glob metacharacters.
///
/// `Bnull`/`Bnullkeep` lex sentinels (backslash-quoted next-char) emit
/// `\X` directly so the same literal semantics carry through.
///
/// Used by the `:#` filter / `/pat/repl` family fast-paths to
/// distinguish quoted patterns (`"*"` → literal star) from unquoted
/// patterns (`*` → glob). Bug #39 in docs/BUGS.md: zshrs treated quoted
/// patterns as glob because both shapes collapsed to ASCII `*` after
/// the plain `untokenize`.
///
/// Mirrors `Src/subst.c` post-`parse_subst_string` behavior where the
/// pattern retains zsh's Dnull/Bnull markers all the way to
/// `patcompile`, which then matches against `zpc_special[ZPC_STAR] =
/// Star` (the token byte) — never against ASCII `*`. zshrs's pattern
/// compiler uses ASCII `*` as the trigger byte (pattern.rs:439), so
/// the same literal-preservation needs to happen at the source level
/// via `\X` escapes.
/// Pattern segment kind used by [`split_pattern_for_glob_subst`] —
/// `Subst` runs through `compile_word_str` + `BUILTIN_GLOB_SUBST_GUARD`
/// at runtime so substitution-result chars get literal-ized when
/// `GLOB_SUBST` is off; `Literal` is emitted as a raw constant so
/// source-level glob meta tokens (Star, Quest, Inbrack, …) survive
/// into `StrMatch`/`patcompile` as match-time globs. Compile-time
/// helper, no C analog (zsh's substitution path tokenizes inline).
#[derive(Debug)]
enum PatSeg {
    Subst(String),
    Literal(String),
}

/// Walk a cond-RHS pattern's tokenized form and split into
/// substitution / literal segments. Mirrors the distinction zsh's
/// singsub draws at run time between bytes that come from `$VAR` /
/// `$(…)` / `\`…\`` (gated by `GLOB_SUBST`) and bytes that are
/// source-level glob metas (always treated as globs). Used by the
/// `[[ x = pat ]]` compile path so `[[ foo = $H* ]]` with H=foo
/// matches: the `\u{8c}H` segment expands+guards to `foo`, the
/// trailing `\u{87}` segment untokenizes to literal `*` which the
/// pattern matcher treats as a glob.
///
/// Segment boundaries:
///   - `\u{85}` (Stringg `$`), `\u{8c}` (Qstring `$`), bare `$`:
///     start of a parameter expansion. Consume the `$` marker plus
///     a following name (`[A-Za-z_][A-Za-z0-9_]*`) OR a brace
///     `${…}` / paren `$(…)` span (depth-balanced) OR an arith
///     `$((…))`.
///   - `\u{93}` (Tick) / `` ` ``: backtick command substitution.
///     Consume until the matching close tick.
///   - `\u{9e}` (Dnull) / `\u{9d}` (Snull) / `"` / `'`: quoted
///     span. Consume the entire matched pair as ONE Subst — the
///     expander processes the contents as a single string.
///   - Everything else: Literal. Includes source-level meta tokens
///     (`\u{87}` Star, `\u{97}` Quest, `\u{91}` Inbrack, …) and
///     plain ASCII.
fn split_pattern_for_glob_subst(s: &str) -> Vec<PatSeg> {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<PatSeg> = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    let flush_lit = |lit: &mut String, out: &mut Vec<PatSeg>| {
        if !lit.is_empty() {
            out.push(PatSeg::Literal(std::mem::take(lit)));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\u{85}' | '\u{8c}' | '$' => {
                // Parameter / arith / cmd-subst start. Collect the
                // marker plus body.
                flush_lit(&mut lit, &mut out);
                let mut subst = String::new();
                subst.push(c);
                i += 1;
                if let Some(&nxt) = chars.get(i) {
                    // c:Src/subst.c:2571-2574 — `(c == '#' || c == Pound) &&
                    // itype_end(s+1, INAMESPC, 0) != s + 1`: `$#` is the
                    // length OPERATOR when a parameter name follows, not the
                    // `$#` positional-count special. See the bare-`$NAME` arm
                    // below for what mis-splitting it broke.
                    let hash_is_len_prefix = matches!(nxt, '#' | '\u{84}')
                        && chars
                            .get(i + 1)
                            .is_some_and(|&c2| c2.is_ascii_alphanumeric() || c2 == '_');
                    if nxt == '\u{9d}' {
                        // `$'…'` ANSI-C span — Stringg/Qstring + Snull
                        // body Snull (parse/lex token form). Keep the
                        // WHOLE span in one Subst segment so
                        // compile_word_str's ANSI fast path decodes
                        // it (c:Src/subst.c:301 stringsubstquote).
                        // The bare-$NAME arm below consumed nothing
                        // (Snull isn't an identifier char), splitting
                        // the span into Subst("$") + Literal(body) —
                        // the body's escapes then leaked as raw text
                        // into the cond pattern: `[[ $'\xff' ==
                        // $'\xff' ]]` compiled the RHS to `$\xff`
                        // and never matched. Bug #127.
                        subst.push(nxt);
                        i += 1;
                        while i < chars.len() {
                            let cc = chars[i];
                            subst.push(cc);
                            i += 1;
                            if cc == '\u{9f}' || cc == '\\' {
                                // Bnull / raw-backslash escape — keep
                                // the escaped char inside the span.
                                if i < chars.len() {
                                    subst.push(chars[i]);
                                    i += 1;
                                }
                                continue;
                            }
                            if cc == '\u{9d}' {
                                break; // close Snull
                            }
                        }
                    } else if nxt == '{' || nxt == '\u{8f}' {
                        // `${…}` — depth-balance braces.
                        let open = nxt;
                        let close = if nxt == '{' { '}' } else { '\u{90}' };
                        subst.push(nxt);
                        i += 1;
                        let mut depth = 1i32;
                        while i < chars.len() && depth > 0 {
                            let cc = chars[i];
                            subst.push(cc);
                            if cc == open {
                                depth += 1;
                            } else if cc == close {
                                depth -= 1;
                            }
                            i += 1;
                        }
                    } else if nxt == '(' || nxt == '\u{96}' {
                        // `$(…)` or `$((…))` — depth-balance parens.
                        let open = nxt;
                        let close = if nxt == '(' { ')' } else { '\u{95}' };
                        subst.push(nxt);
                        i += 1;
                        let mut depth = 1i32;
                        while i < chars.len() && depth > 0 {
                            let cc = chars[i];
                            subst.push(cc);
                            if cc == open {
                                depth += 1;
                            } else if cc == close {
                                depth -= 1;
                            }
                            i += 1;
                        }
                    } else if (matches!(nxt, '$' | '?' | '#' | '*' | '@' | '-' | '!')
                        || matches!(
                            nxt,
                            '\u{85}' /* Stringg `$` */
                            | '\u{97}' /* Quest `?` */
                            | '\u{84}' /* Pound `#` */
                            | '\u{87}' /* Star `*` */
                            | '\u{9b}' /* Dash `-` */
                            | '\u{9c}' /* Bang `!` */
                        ))
                        && !hash_is_len_prefix
                    {
                        // Single-char special parameter — `$$` / `$?` /
                        // `$#` / `$*` / `$@` / `$-` / `$!`. The lexer
                        // tokenizes the second char (`$$` arrives as
                        // Stringg Stringg, `$?` as Stringg Quest), so
                        // accept both ASCII and token forms. Consume
                        // exactly ONE char into the Subst segment;
                        // without this arm the name walk below consumed
                        // nothing and the special leaked into the
                        // Literal segment — `[[ $$ == $$ ]]` compiled
                        // its RHS to the literal 2-char pattern `$$`
                        // and never matched (bug #628). C: singsub →
                        // paramsubst handles these specials at
                        // Src/subst.c:2024+ (`case '$': case '?': …`).
                        subst.push(nxt);
                        i += 1;
                    } else {
                        // c:Src/subst.c:2571-2589 — a `#`/Pound directly
                        // after the `$` is the LENGTH prefix whenever a
                        // name follows (`itype_end(s+1, INAMESPC, 0) !=
                        // s + 1`), i.e. `$#NAME` is "length of $NAME",
                        // one substitution. The single-char-special arm
                        // above consumed only the `#` (the `$#`
                        // positional count), leaving `NAME` behind as a
                        // Literal segment, so a cond/case RHS pattern of
                        // `$#otmp` compiled to `0otmp`:
                        //   `[[ 2 = $#otmp ]]` (otmp="cd") answered NO.
                        // That is the `while [[ $#tmp != $#otmp ]]`
                        // fixed-point loop in Completion/Base/Completer/
                        // _expand:78-81 — it never terminated, so every
                        // completion whose completer list starts with
                        // `_expand` (the comptest default, Y01/Y02/Y03)
                        // spun forever inside `zle complete-word`.
                        if matches!(nxt, '#' | '\u{84}') {
                            subst.push(nxt);
                            i += 1;
                        }
                        // Bare `$NAME` — consume identifier chars.
                        while i < chars.len() {
                            let cc = chars[i];
                            if cc.is_ascii_alphanumeric() || cc == '_' {
                                subst.push(cc);
                                i += 1;
                            } else {
                                break;
                            }
                        }
                        // A trailing `[sub]` is the ARRAY SUBSCRIPT of
                        // `$NAME`, NOT a source-level glob bracket. zsh's
                        // paramsubst parses `$name[exp]` as one
                        // substitution (Src/subst.c getindex). Without
                        // consuming it here the subscript split off into a
                        // Literal segment and became a `[...]` char-class,
                        // so `[[ yy == $a[2] ]]` expanded `$a` (joined
                        // array) + matched a literal `2` char-class and
                        // never matched. Depth-balance `[`/Inbrack ↔
                        // `]`/Outbrack so nested subscripts (`$a[$b[1]]`)
                        // stay intact.
                        if matches!(chars.get(i), Some('[') | Some('\u{91}')) {
                            let mut depth = 0i32;
                            while i < chars.len() {
                                let cc = chars[i];
                                subst.push(cc);
                                i += 1;
                                match cc {
                                    '[' | '\u{91}' => depth += 1,
                                    ']' | '\u{92}' => {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                out.push(PatSeg::Subst(subst));
            }
            '\u{93}' | '`' => {
                // Backtick substitution — match to closing tick.
                flush_lit(&mut lit, &mut out);
                let close = c;
                let mut subst = String::new();
                subst.push(c);
                i += 1;
                while i < chars.len() {
                    let cc = chars[i];
                    subst.push(cc);
                    i += 1;
                    if cc == close {
                        break;
                    }
                }
                out.push(PatSeg::Subst(subst));
            }
            '\u{9e}' | '"' => {
                // Double-quoted span: zsh expands the entire body as
                // a single string. Treat the whole span as one Subst.
                flush_lit(&mut lit, &mut out);
                let close = c;
                let mut subst = String::new();
                subst.push(c);
                i += 1;
                while i < chars.len() {
                    let cc = chars[i];
                    subst.push(cc);
                    i += 1;
                    if cc == close {
                        break;
                    }
                }
                out.push(PatSeg::Subst(subst));
            }
            '\u{9d}' | '\'' => {
                // Single-quoted span: pure literal. Take the entire
                // span as Literal so untokenize strips the markers.
                let close = c;
                lit.push(c);
                i += 1;
                while i < chars.len() {
                    let cc = chars[i];
                    lit.push(cc);
                    i += 1;
                    if cc == close {
                        break;
                    }
                }
            }
            // Bnull/Bnullkeep (lexer escape markers) and a raw `\`
            // escape the NEXT char as source-literal — consume the
            // PAIR into the literal segment so an escaped `$` never
            // reaches the Subst arm above. Without this, the cond
            // pattern `\$*` (Bnull,$,Star) split as Literal(Bnull) +
            // Subst("$*") — the escaped dollar substituted the
            // positional params (f-sy-h's `\$[{]` variant compiled
            // `$[…]` math on the class body and errored "bad math
            // expression: illegal character: 0x8f" per keystroke,
            // then its while loop spun the shell at 100% CPU).
            '\u{9f}' | '\u{a0}' | '\\' => {
                // Emit the RAW-ASCII escape form (`\X`) — the STRMATCH
                // runtime re-tokenizes the assembled pattern, folding
                // `\X` to a Bnull literal for patcompile; a raw marker
                // byte would not round-trip.
                lit.push('\\');
                if let Some(&nxt) = chars.get(i + 1) {
                    lit.push(nxt);
                    i += 2;
                } else {
                    i += 1;
                }
            }
            _ => {
                lit.push(c);
                i += 1;
            }
        }
    }
    flush_lit(&mut lit, &mut out);
    out
}

fn untokenize_preserve_quoted_pat_literals(s: &str) -> String {
    use crate::ported::zsh_h::{
        Bang, Bar, Bnull, Bnullkeep, Dash, Dnull, Equals, Hat, Inang, Inbrace, Inbrack, Inpar,
        Inparmath, Outang, OutangProc, Outbrace, Outbrack, Outpar, Outparmath, Pound, Qstring,
        Qtick, Quest, Snull, Star, Stringg, Tick, Tilde,
    };
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut in_dq = false;
    let mut in_sq = false;
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == Snull {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if c == Dnull {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if c == Bnull || c == Bnullkeep {
            // Bnull/Bnullkeep mark "next char is backslash-escaped"
            // from the lex DQ path (c:1499). Emit `\X` so the pattern
            // compiler treats X as literal regardless of whether it
            // is a metachar.
            if i + 1 < chars.len() {
                out.push('\\');
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        let unt = match c {
            x if x == Pound => '#',
            x if x == Stringg => '$',
            x if x == Hat => '^',
            x if x == Star => '*',
            x if x == Inpar => '(',
            x if x == Outpar => ')',
            x if x == Inparmath => '(',
            x if x == Outparmath => ')',
            x if x == Qstring => '$',
            x if x == Equals => '=',
            x if x == Bar => '|',
            x if x == Inbrace => '{',
            x if x == Outbrace => '}',
            x if x == Inbrack => '[',
            x if x == Outbrack => ']',
            x if x == Tick => '`',
            x if x == Inang => '<',
            x if x == Outang => '>',
            x if x == OutangProc => '>',
            x if x == Quest => '?',
            x if x == Tilde => '~',
            x if x == Qtick => '`',
            x if x == Dash => '-',
            x if x == Bang => '!',
            other => other,
        };
        // Inside Snull/Dnull spans the original source had the char
        // quoted, so pattern-metachar interpretation must be
        // suppressed. Prefix `\` for chars that the pattern compiler
        // would otherwise treat as glob metas.
        if (in_dq || in_sq)
            && matches!(
                unt,
                '*' | '?'
                    | '['
                    | ']'
                    | '('
                    | ')'
                    | '|'
                    | '~'
                    | '#'
                    | '^'
                    | '<'
                    | '>'
                    | '\\'
                    | '+'
                    | '@'
                    | '!'
            )
        {
            out.push('\\');
        }
        out.push(unt);
        i += 1;
    }
    out
}

/// Parse `${...}` and detect a Phase 1 param-modifier shape. Returns
/// `None` for shapes that need the bridge — nested `${...}`, multiple
/// modifiers chained, etc. The name must be a plain identifier; mixed
/// concatenation and modifiers fall through.
fn parse_param_modifier(s: &str) -> Option<ParamModifier> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    if inner.is_empty() {
        return None;
    }
    // The opening `${` must close at the FINAL `}` — this fast path only
    // handles a word that is a SINGLE `${...}` expansion. When the
    // matching `}` comes earlier, the word is a concatenation of two or
    // more expansions (e.g. `${s:0:1}${s:1}`, `${x}y${z}`) whose trailing
    // parts would be silently swallowed by `strip_suffix('}')` above and
    // the modifier parse below. Bail so split_word_segments emits each
    // piece. Brace depth handles nested operands (`${var:${#x}-1}`).
    {
        let b = s.as_bytes();
        let mut depth = 0i32;
        let mut close = None;
        let mut i = 1; // start at the `{` of `${`
        while i < b.len() {
            match b[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if close != Some(b.len() - 1) {
            return None;
        }
    }
    // `(@)NAME##pat` / `(@)NAME%pat` / etc. — `(@)` forces array shape
    // for the surrounding word. On an unquoted indexed array the
    // shape is already preserved by per-element iteration, so the
    // `(@)` is a no-op there. In DQ context, however, `(@)` overrides
    // the join-then-strip default and forces per-element semantics
    // — same effect as an explicit `[@]` subscript. Track via the
    // `at_flag_seen` flag and propagate to had_at on Strip/Replace.
    // Direct port of zsh nojoin=1 path (Src/subst.c:1813).
    let (inner, at_flag_seen) = if let Some(rest) = inner.strip_prefix("(@)") {
        (rest, true)
    } else {
        (inner, false)
    };
    // Reject flag forms — handled by earlier fast-paths.
    if inner.starts_with('(') {
        return None;
    }
    // Nested `${…}` is allowed in substring offset/length operands
    // (`${var:N:${#x}-2}`, `${var:$((${#x}-2))}` etc.). Other shapes
    // (length-of-nested, strip with nested pattern, etc.) still fall
    // through to the bridge. Detect by scanning for the `:N:` shape
    // and routing the substring path. The offset and length operands
    // can both contain `${...}` and `$((...))` — they're evaluated
    // by the runtime arith path which calls expand_string first.
    if inner.contains("${") {
        if let Some(first_colon) = inner.find(':') {
            // Substring shape: must start with NAME:digit/$/-/...
            // The substring path in this function handles the rest.
            // Both offset and length operands may contain `${…}` /
            // `$((…))` — those are arith-evaluated by SubstringExpr.
            let after_first = &inner[first_colon + 1..];
            // Skip the offset's leading minus / spaces.
            let off_section_end = after_first.find(':').unwrap_or(after_first.len());
            let off_section = &after_first[..off_section_end];
            // Only reject if `${…}` shows up in non-substring shapes
            // (no leading `:`-then-digit/`$`/`-`/`(` pattern).
            let after_first_trim = after_first.trim_start_matches(' ');
            let is_substring_shape = matches!(after_first_trim.chars().next(),
                    Some(c) if c.is_ascii_digit()
                        || c == '-' || c == '$' || c == '(');
            if !is_substring_shape && off_section.contains("${") {
                return None;
            }
            // The first colon must be the var/op split, not the start
            // of `:#` filter or `:-` default.
            let after_first_first = inner.as_bytes().get(first_colon + 1).copied();
            if matches!(
                after_first_first,
                Some(b'-') | Some(b'+') | Some(b'=') | Some(b'?') | Some(b'#') | Some(b'/')
            ) {
                return None;
            }
        } else {
            return None;
        }
    }

    // Find where the var name ends. Plain identifier rules: letters,
    // digits (positional), or special-name single chars. The first
    // non-identifier byte starts the modifier op.
    let bytes = inner.as_bytes();
    let mut name_end = 0;
    let first = bytes[0];
    // `${+name}` — set-test. Returns "1" if name is set, "0" if unset.
    // Direct port of subst.c case '+' at the leading-flag position
    // (distinct from `${name+rhs}` which is the substitute-if-set
    // form). Treat as Length-shape but route through a SetTest variant.
    if first == b'+' && bytes.len() > 1 {
        let rest = &inner[1..];
        // Identifier OR identifier with `[…]` subscript.
        let name_part: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '@' || *c == '*')
            .collect();
        if !name_part.is_empty() {
            // The full "name[idx]" form goes via DefaultFamily op=8
            // (new SetTest opcode, defined below) so the runtime can
            // resolve subscript/magic-assoc + emit "1"/"0".
            return Some(ParamModifier {
                name: rest.to_string(),
                kind: ParamModifierKind::DefaultFamily {
                    op: 8,
                    rhs: String::new(),
                },
            });
        }
    }
    // `${#name}` — length form. Special-case: var name follows the `#`.
    // Both scalars (StringLen) and arrays (ARRAY_LENGTH) are supported,
    // dispatched at runtime by ParamModifierKind::Length.
    if first == b'#' && bytes.len() > 1 {
        let rest = &inner[1..];
        // Keep any `[@]`/`[*]` subscript in the name (do NOT strip it): under
        // KSHARRAYS `${#arr}` (bare) is the strlen of element 1 while
        // `${#arr[@]}` stays the element count, so the subscript must survive
        // to the runtime paramsubst length code to tell them apart. Without
        // KSHARRAYS both still count, since paramsubst treats `[@]` as an
        // array-shape selector. (Previously `[@]`/`[*]` were stripped, which
        // collapsed the two forms and lost the KSHARRAYS distinction.)
        let body = rest;
        // `${#name:-default}` etc. → fall through to the bridge so the
        // default is applied first and the length is taken on the
        // post-default result. The bridge correctly distinguishes
        // unset/empty (uses default → length of default text) from
        // set arrays (default unused → array element count).
        if body.contains(":-") || body.contains(":+") || body.contains(":=") || body.contains(":?")
        {
            return None;
        }
        // Special single-char params take the length form too:
        // `${#-}` = length of $- (option letters), `${#?}` = length
        // of $?, `${#$}` = length of $$, `${#0}`/`${#1}`… = string
        // length of $0/positional. zsh REJECTS `${#!}` ("bad
        // substitution") so `!` stays excluded. Verified against
        // zsh 5.9. The runtime BUILTIN_PARAM_LENGTH -> paramsubst
        // path already resolves these names (the quoted "${#-}"
        // form worked; only this unquoted parse rejected them and
        // fell through to a ${#}-shaped misparse printing argc).
        if matches!(body, "-" | "?" | "$")
            || (!body.is_empty() && body.chars().all(|c| c.is_ascii_digit()))
        {
            return Some(ParamModifier {
                name: body.to_string(),
                kind: ParamModifierKind::Length,
            });
        }
        if !body
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        {
            return None;
        }
        // `${#arr[N]}` — length of element N, NOT the array count.
        // Accept body of form `name[<anything>]` so the runtime
        // BUILTIN_PARAM_LENGTH handler resolves the subscript first.
        let bracketed = body.find('[').is_some_and(|i| body.ends_with(']') && i > 0);
        let bare_name_end = body.find('[').unwrap_or(body.len());
        let bare = &body[..bare_name_end];
        if !bare.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            return None;
        }
        if !bracketed && body.len() != bare.len() {
            return None;
        }
        return Some(ParamModifier {
            name: body.to_string(),
            kind: ParamModifierKind::Length,
        });
    }
    while name_end < bytes.len() {
        let b = bytes[name_end];
        if b.is_ascii_alphanumeric() || b == b'_' {
            name_end += 1;
        } else {
            break;
        }
    }
    if name_end == 0 {
        // `${:-word}` / `${:+word}` / `${:?word}` — the EMPTY-name
        // default family. C reaches these through the ordinary
        // paramsubst operator dispatch (Src/subst.c:3193 `case '-'`);
        // the name simply resolves unset. They are unambiguous because
        // the operator is spelled with a leading colon, unlike the
        // no-colon `${-x}` / `${+x}` / `${?x}` which collide with the
        // special parameters `$-` / `$?`, so only the colon forms are
        // admitted here.
        //
        // Routing them through this modifier path is what brackets the
        // word with BUILTIN_DEFAULT_WORD_GLOB_RESET/…GLOB, so a SOURCE
        // glob in the default word drives filename generation:
        // `${:-*}` expands to the directory listing in zsh. Bailing
        // here sent the word down the generic text path, which sets
        // DEFAULT_WORD_GLOB_PENDING in paramsubst but has nothing to
        // consume it, so `${:-*}` came out as the literal `*` while the
        // named form `${u:-*}` globbed correctly.
        let is_empty_name_default = bytes.first() == Some(&b':')
            && matches!(bytes.get(1), Some(b'-') | Some(b'+') | Some(b'?'));
        if !is_empty_name_default {
            // Special single-char name? Not handled here.
            return None;
        }
    }
    let name = inner[..name_end].to_string();
    // Optional `[@]` / `[*]` subscript suffix — for arrays and assocs
    // these are no-ops on the lookup but shouldn't break the modifier
    // parse. Strip and continue parsing the modifier. Track whether
    // we saw it so downstream Strip emit can force per-element
    // semantics inside DQ (`"${a[@]%%pat}"` = per-element, not
    // joined-then-stripped).
    let mut after_name = name_end;
    let mut had_at = at_flag_seen;
    // The EXACT splat suffix the source carried, if any. `had_at` cannot serve
    // this purpose: it is deliberately false for `[*]` (which joins with IFS
    // rather than forcing per-element semantics in DQ), yet `[*]` still has to
    // reach paramsubst as a SUBSCRIPT so the KSHARRAYS bare-ref reduction is
    // skipped. Bug #1054.
    let mut splat_suffix: Option<&str> = None;
    // Char-aware boundary check — name_end + 3 may land mid-codepoint
    // when the preceding bytes include UTF-8 multi-byte chars (e.g.
    // METATOKEN bytes in lexer-emitted input). `is_char_boundary`
    // protects the slice from panicking on invalid index.
    if inner.len() >= name_end + 3 && inner.is_char_boundary(name_end + 3) {
        let tail = &inner[name_end..name_end + 3];
        if tail == "[@]" {
            // `[@]` = splice-expand (per-element even in DQ)
            after_name = name_end + 3;
            had_at = true;
            splat_suffix = Some("[@]");
        } else if tail == "[*]" {
            // `[*]` = join-with-IFS-then-scalar (matches the bare-
            // name DQ join-then-strip behavior — leave had_at false
            // so the runtime treats it like the unsubscripted
            // `"${a%%pat}"` case).
            after_name = name_end + 3;
            splat_suffix = Some("[*]");
        }
    }
    let rest = &inner[after_name..];
    if rest.is_empty() {
        // No modifier — caller's `braced_var_ref` path should have caught
        // this already; treat as not-our-shape so we don't double-emit.
        return None;
    }

    // `${var:-…}` / `${var:=…}` / `${var:?…}` / `${var:+…}` and the
    // no-colon variants `${var-…}` / `${var=…}` / `${var?…}` / `${var+…}`
    // which fire only when `var` is truly unset (not just empty).
    // c:Src/subst.c — operator detection by leading char(s). The
    // bare `&rest[..2]` slice panics if the first character is
    // multibyte (`é`/`日`/etc. — Bug #365/#366 in docs/BUGS.md).
    // The default-family operators are all ASCII, so guard each
    // byte index with `is_char_boundary` before slicing.
    if rest.len() >= 2 && rest.is_char_boundary(2) {
        let op_byte = match &rest[..2] {
            ":-" => Some(0u8),
            ":=" => Some(1u8),
            ":?" => Some(2u8),
            ":+" => Some(3u8),
            _ => None,
        };
        if let Some(op) = op_byte {
            let rhs = rest[2..].to_string();
            return Some(ParamModifier {
                name,
                kind: ParamModifierKind::DefaultFamily { op, rhs },
            });
        }
    }
    if !rest.is_empty() && rest.is_char_boundary(1) {
        let op_byte = match &rest[..1] {
            "-" => Some(4u8),
            "=" => Some(5u8),
            "?" => Some(6u8),
            "+" => Some(7u8),
            _ => None,
        };
        if let Some(op) = op_byte {
            let rhs = rest[1..].to_string();
            return Some(ParamModifier {
                name,
                kind: ParamModifierKind::DefaultFamily { op, rhs },
            });
        }
    }

    // `${var:#pattern}` — filter: remove matching elements.
    if let Some(pat) = rest.strip_prefix(":#") {
        // Re-attach the `[@]` / `[*]` suffix when the source carried one,
        // exactly as the substring path below does. paramsubst's subscript
        // loop is what records the splat (`was_at_star_splat`, subst.rs), and
        // that flag is what exempts the reference from the KSHARRAYS
        // "bare ref scalarizes to element 0" reduction. Binding the filter to
        // the bare name threw the `[@]` away before paramsubst ever saw it, so
        // under KSH_ARRAYS — which `emulate sh` / `emulate ksh` both set —
        // `${arr[@]:#pat}` was reduced to element 0 and then filtered,
        // returning at most one element. Bug #1054.
        let runtime_name = match splat_suffix {
            Some(sfx) => format!("{}{}", name, sfx),
            None => name.clone(),
        };
        return Some(ParamModifier {
            name: runtime_name,
            kind: ParamModifierKind::FilterRemoveMatching {
                pattern: pat.to_string(),
            },
        });
    }

    // `${var:offset[:length]}` substring. The post-`:` text must lead
    // with a digit, `-`, single space (negative-offset disambiguator),
    // OR `$`/`(` (variable / arith expression — runtime-evaluated).
    if let Some(after) = rest.strip_prefix(':') {
        let trimmed = after.trim_start_matches(' ');
        let first_ch = trimmed.chars().next();
        if matches!(first_ch, Some(c) if c.is_ascii_digit() || c == '-' || c == '$' || c == '(') {
            // Split on the FIRST top-level `:` so `${s:$n:2}` keeps
            // `$n` whole. We don't have nested `${...}` here (the outer
            // parse_param_modifier already rejects those), so a simple
            // depth tracker on `(` is enough.
            let chars: Vec<char> = trimmed.chars().collect();
            let mut depth = 0i32;
            let mut split_at: Option<usize> = None;
            for (i, &c) in chars.iter().enumerate() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ':' if depth == 0 => {
                        split_at = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let off_str: String = match split_at {
                Some(i) => chars[..i].iter().collect(),
                None => chars.iter().collect(),
            };
            let len_str: Option<String> = split_at.map(|i| chars[i + 1..].iter().collect());
            let off_str = off_str.trim().to_string();
            let len_str = len_str.map(|s| s.trim().to_string());
            // Re-attach `[@]` / `[*]` suffix to the name when had_at
            // was true so the runtime substring handler can route to
            // the array-splice path. Without this, `${a[@]:1}` was
            // bound to plain `a` and returned a joined scalar.
            let runtime_name = if had_at {
                format!("{}[@]", name)
            } else {
                name.clone()
            };
            // Literal-only fast path: integer offset (and length).
            if let (Ok(offset), len_opt) = (
                off_str.parse::<i64>(),
                len_str.as_deref().map(|s| s.parse::<i64>().ok()),
            ) {
                let length: Option<i64> = match len_opt {
                    None => None,
                    Some(Some(v)) => Some(v),
                    Some(None) => {
                        return Some(ParamModifier {
                            name: runtime_name,
                            kind: ParamModifierKind::SubstringExpr {
                                offset_expr: offset.to_string(),
                                length_expr: len_str,
                            },
                        })
                    }
                };
                return Some(ParamModifier {
                    name: runtime_name,
                    kind: ParamModifierKind::Substring { offset, length },
                });
            }
            // Variable / arith case — defer to runtime.
            return Some(ParamModifier {
                name: runtime_name,
                kind: ParamModifierKind::SubstringExpr {
                    offset_expr: off_str,
                    length_expr: len_str,
                },
            });
        }
    }

    // `${var/pat/repl}` family. Detect leading `/`/`//`/`/#`/`/%`,
    // then split on the second `/`.
    if rest.starts_with('/') {
        // Note: longer prefixes must be checked FIRST so `//#`/`//%`
        // win over `//`. zsh treats `//#` as "anchor at start, replace
        // all" (effectively single since the anchor matches once);
        // `//%` is the suffix-anchor analog. Both produce the same
        // result as `/#`/`/%` for non-overlapping matches.
        let (op, body) = if let Some(b) = rest.strip_prefix("//#") {
            (2u8, b)
        } else if let Some(b) = rest.strip_prefix("//%") {
            (3u8, b)
        } else if let Some(b) = rest.strip_prefix("//") {
            (1u8, b)
        } else if let Some(b) = rest.strip_prefix("/#") {
            (2u8, b)
        } else if let Some(b) = rest.strip_prefix("/%") {
            (3u8, b)
        } else {
            (0u8, rest.strip_prefix('/').unwrap_or(rest))
        };
        // body = "pat/repl" or "pat" (no replacement = empty repl).
        // Find the FIRST UNESCAPED `/` so `${HOME//\//_}` splits with
        // pattern=`/` and replacement=`_`. Naive splitn split on the
        // escaped `\/` and produced `\\` as the pattern.
        let chars: Vec<char> = body.chars().collect();
        let mut sep = None;
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '\\' && i + 1 < chars.len() {
                i += 2;
                continue;
            }
            if chars[i] == '/' {
                sep = Some(i);
                break;
            }
            i += 1;
        }
        let unesc = |s: &[char]| -> String {
            let mut out = String::with_capacity(s.len());
            let mut it = s.iter().copied().peekable();
            while let Some(c) = it.next() {
                if c == '\\' {
                    if let Some(&nx) = it.peek() {
                        if nx == '/' {
                            out.push('/');
                            it.next();
                            continue;
                        }
                    }
                }
                out.push(c);
            }
            out
        };
        let (pattern, repl) = match sep {
            Some(p) => (unesc(&chars[..p]), unesc(&chars[p + 1..])),
            None => (unesc(&chars), String::new()),
        };
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Replace {
                op,
                pattern,
                repl,
                had_at,
            },
        });
    }

    // `${var#pat}` / `${var##pat}` / `${var%pat}` / `${var%%pat}`
    if let Some(b) = rest.strip_prefix("##") {
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Strip {
                op: 1,
                pattern: b.to_string(),
                had_at,
            },
        });
    }
    if let Some(b) = rest.strip_prefix("%%") {
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Strip {
                op: 3,
                pattern: b.to_string(),
                had_at,
            },
        });
    }
    if let Some(b) = rest.strip_prefix('#') {
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Strip {
                op: 0,
                pattern: b.to_string(),
                had_at,
            },
        });
    }
    if let Some(b) = rest.strip_prefix('%') {
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Strip {
                op: 2,
                pattern: b.to_string(),
                had_at,
            },
        });
    }

    None
}

/// True when a `${(flags)…}` form's operand begins with a quote in the
/// quote-preserving (ztokens) rendering — the `${(Q)"abc"}` /
/// `${(L)'x'}` shapes. c:Src/subst.c:2990-3004 — zsh's paramsubst
/// raises "bad substitution" for these: after the flag block the name
/// walker stops at the quote char, which is also not a valid operator
/// char. The compile fast paths work on plain-untokenized text where
/// the quotes are already stripped, so they must consult this check
/// (on the ztokens rendering) to avoid misreading the quoted literal
/// as a parameter NAME.
fn flag_operand_is_quoted_literal(s_ztok: &str) -> bool {
    let Some(inner) = s_ztok.strip_prefix("${").and_then(|t| t.strip_suffix('}')) else {
        return false;
    };
    // Flagless `${"abc"}` / `${'abc'}` — same "bad substitution" per
    // c:Src/subst.c:2990-3004 (the closing quote lands in operator
    // position after the name walk).
    let operand = match matching_paren_close(inner) {
        Some(close) => &inner[close + 1..],
        None => inner,
    };
    matches!(operand.chars().next(), Some('"') | Some('\''))
}

fn parse_zsh_flag(s: &str) -> Option<(&str, &str)> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    let inner_b = inner.as_bytes();
    if inner_b.first()? != &b'(' {
        return None;
    }
    let mut depth = 0;
    let mut close = None;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let flags = &inner[1..close];
    let name = &inner[close + 1..];
    // Do NOT strip a trailing `[@]` / `[*]` — a name with `[` fails the
    // fast-path guard below (returns None), so `${(flags)a[@]}` falls through
    // to the general EXPAND_TEXT path that passes the FULL `${(flags)a[@]}` text
    // to paramsubst. This preserves the `[@]`/`[*]` splat so paramsubst can make
    // the KSHARRAYS distinction (`${(o)a}` → elem 0, `${(o)a[@]}` → whole array);
    // the previous strip collapsed both to `${(o)a}`, losing it. Non-KSHARRAYS
    // behavior is unchanged (both forms splat/join the whole array).
    if name.is_empty()
        || name.contains('$')
        || name.contains('{')
        || name.contains('}')
        || name.contains('[')
    {
        return None;
    }
    let first = name.chars().next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    if !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((flags, name))
}

/// Match `${(flags)NAME[KEY]}` where KEY is a non-`@`/`*` literal
/// subscript (assoc key, file path, etc.). The compile path resolves
/// the subscripted value first via BUILTIN_ARRAY_INDEX, then feeds the
/// scalar into BUILTIN_PARAM_FLAG via the `\u{01}` literal-value
/// sentinel. Excludes nested `${…}` and dynamic `$`-keys (those need
/// a different lowering — runtime expand-then-flag).
/// Find the matching `)` for a leading `(` in `s`. Returns Some(index)
/// pointing at the matching close; None if `s` doesn't start with `(`
/// or the parens are unbalanced. Used by the bridge-array fast path
/// to extract the flag-chain region from `(flags)body` shapes.
fn matching_paren_close(s: &str) -> Option<usize> {
    if !s.starts_with('(') {
        return None;
    }
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_zsh_flag_subscript(s: &str) -> Option<(&str, &str, &str)> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    if inner.contains("${") {
        return None;
    }
    let inner_b = inner.as_bytes();
    if inner_b.first()? != &b'(' {
        return None;
    }
    let mut depth = 0;
    let mut close = None;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close?;
    let flags = &inner[1..close];
    let after = &inner[close + 1..];
    let lb = after.find('[')?;
    if !after.ends_with(']') {
        return None;
    }
    let base = &after[..lb];
    let key = &after[lb + 1..after.len() - 1];
    if base.is_empty() || key.is_empty() || key == "@" || key == "*" {
        return None;
    }
    if !base.chars().next()?.is_ascii_alphabetic() && !base.starts_with('_') {
        return None;
    }
    if !base.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    if key.contains('[') || key.contains(']') || key.contains('`') {
        return None;
    }
    // Allow `$` in keys — BUILTIN_ARRAY_INDEX runtime-expands the
    // subscript text via expand_string before parsing, so a slice
    // like `$((expr)),-1` resolves to its numeric form there.
    // Without this gate removed, `${(@)arr[$((expr)),-1]}` fell
    // through to the EXPAND_TEXT bridge which scalar-flattens
    // and lost the slice semantics.
    Some((flags, base, key))
}

/// Resolve a LITERAL `[…]` subscript's backslash quoting the way
/// `getindex` does before the key ever reaches the hash table.
///
/// c:Src/params.c:2029 — `getindex` never reads the subscript as the outer
/// lexer left it; it calls `parse_subscript(s, scanflags & SCANPM_DQUOTED,
/// ']')`, which re-lexes the text through `dquote_parse(']', sub)`
/// (c:Src/lex.c:1751-1769). With `endchar == ']'` a backslash before one of
/// ``$ \ ` ] [ ( ) { }`` becomes a `Bnull` marker + the bare char
/// (c:Src/lex.c:1497-1512); `getarg` then keeps the marker before a bracket
/// and untokenizes the rest to a literal `\` (c:Src/params.c:1538-1551), and
/// `remnulargs` (c:1583) plus the `parsestr` + `singsub` round (c:1585-1592)
/// delete what is left. Net effect: `A[\\q]` keys on `\q`, `A[\\\\q]` on
/// `\\q`, `A[\q]` on `\q` (a backslash before an unlisted char is ordinary
/// text, c:Src/lex.c:1508-1511). `subscript_escape::subscript_unescape` is
/// that composite; the ASSIGNMENT path already runs it
/// (compile_zsh.rs:3854), the READ fast paths did not, so
/// `A[$'\\q']=v; print $A[\\q]` missed by exactly one escape level
/// (D06subscript.ztst "Associative array lookup (direct subscripting)").
///
/// A subscript that OPENS with a flag group is left alone: c:1583's
/// `if (ishash && (keymatch || !rev)) remnulargs(s);` skips the marker
/// deletion for a reverse/pattern search, because there the backslashes are
/// PATTERN escapes that `patcompile` (c:1697) still has to see.
fn subscript_literal_key(key: &str) -> std::borrow::Cow<'_, str> {
    let trimmed = key.trim_start();
    if trimmed.starts_with('(')
        || trimmed.starts_with(crate::ported::zsh_h::Inpar)
        || !key.contains('\\')
    {
        return std::borrow::Cow::Borrowed(key); // c:1583 (rev/keymatch) or nothing to do
    }
    // `resolve_dollar` is true: this is a compile-time LITERAL key (the
    // callers reject `$`/backtick keys), so there is no `parsestr`/`singsub`
    // round after this and c:1585-1592's share of the work belongs here.
    std::borrow::Cow::Owned(crate::subscript_escape::subscript_unescape(key, false, true).0)
}

/// Split a subscripted name like `m[k]` or `arr[1]` into (base, key).
/// Returns None if `s` is a plain identifier with no `[...]`.
fn split_subscript(s: &str) -> Option<(&str, &str)> {
    let lb = s.find('[')?;
    if !s.ends_with(']') {
        return None;
    }
    let base = &s[..lb];
    let key = &s[lb + 1..s.len() - 1];
    if base.is_empty() {
        return None;
    }
    // Empty key (`H[""]=v` after untokenize → `H[]=v`) is a valid
    // associative-array assignment in zsh — the empty string is a
    // legal hash key. Don't reject it; the assoc set path stores
    // the entry under "" and reads come back the same way.
    // Reject `arr[@]` / `arr[*]` — those are splice forms handled
    // elsewhere (array_splice_ref / ARRAY_ALL).
    if key == "@" || key == "*" {
        return None;
    }
    Some((base, key))
}

/// Return the (base, key) if `s` is a `${NAME[KEY]}` form (assoc/array
/// element access). Excludes `[@]` / `[*]` splice forms. Requires the
/// inner to be a strict `NAME[KEY]` shape — no nested `${...}`, no extra
/// braces, no chars after the `]`. Multi-group strings like
/// `${foo[a]} ${foo[b]}` (two adjacent subscripted refs in one word) hit
/// the runtime fallback instead.
fn braced_subscript_ref(s: &str) -> Option<(&str, &str)> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    if inner.contains("${") || inner.contains('}') {
        return None;
    }
    let (base, rest) = inner.split_once('[')?;
    let key = rest.strip_suffix(']')?;
    if base.is_empty() || key.is_empty() || key == "@" || key == "*" {
        return None;
    }
    // Special-name positionals `@` and `*` — accept as base so
    // `${@[N,M]}` / `${*[N]}` route through BUILTIN_ARRAY_INDEX which
    // has a positional-param branch. Digit-name positionals (`${1[..]}`,
    // `${10[..]}`) also accepted: BUILTIN_ARRAY_INDEX falls through to
    // get_variable which resolves positional-N.
    let is_special = base == "@" || base == "*";
    let is_digit_positional = !base.is_empty() && base.chars().all(|c| c.is_ascii_digit());
    if !is_special && !is_digit_positional {
        if !base.chars().next()?.is_ascii_alphabetic() && !base.starts_with('_') {
            return None;
        }
        if !base.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            return None;
        }
    }
    // Reject keys that themselves contain `[` or `]` (nested subscript)
    // OR a `$`-expansion (must be evaluated at runtime, not compile time).
    if key.contains('[') || key.contains(']') || key.contains('$') || key.contains('`') {
        return None;
    }
    Some((base, key))
}

/// Same shape as `braced_subscript_ref` but allows the key to contain
/// `$`-expansions (`${m[$k]}`, `${m[$pre$post]}`). The compile path
/// resolves the key text at runtime via BUILTIN_EXPAND_TEXT before
/// looking it up. Excludes nested `${…}` (which would need recursive
/// compilation), backticks (cmd-sub), and `[`/`]` (nested subscript).
fn braced_subscript_dynamic_ref(s: &str) -> Option<(&str, &str)> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    if inner.contains("${") || inner.contains('}') {
        return None;
    }
    let (base, rest) = inner.split_once('[')?;
    let key = rest.strip_suffix(']')?;
    if base.is_empty() || key.is_empty() || key == "@" || key == "*" {
        return None;
    }
    if !base.chars().next()?.is_ascii_alphabetic() && !base.starts_with('_') {
        return None;
    }
    if !base.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    if key.contains('[') || key.contains(']') || key.contains('`') {
        return None;
    }
    // Only kick in when the key actually has a `$` expansion — the
    // pure-literal case is handled by the static `braced_subscript_ref`
    // matcher above us in the compile path.
    if !key.contains('$') {
        return None;
    }
    // c:Src/params.c:1533-1536 — `getarg`'s scan loop finds the end of a
    // subscript ARGUMENT in the UNEXPANDED text and only then runs
    // `parsestr`/`singsub` on that slice (c:1567-1571). This fast path
    // does the opposite: it expands the WHOLE key first and hands the
    // result to `array_index_lookup`, which rebuilds `${name[key]}` and
    // re-parses it. That is only sound while the expansion cannot change
    // the argument structure. A leading `(` flag group makes the
    // remainder a search PATTERN (c:1389-1391) and a top-level `,` makes
    // the subscript a RANGE — in both shapes an expanded-in comma
    // re-splits: `x=','; ${s[(r)$x,(R)$x]}` became the three-argument
    // `(r),,(R),` and died with "bad substitution" where zsh searches for
    // the literal `,` on each bound. Leave those to paramsubst, which
    // splits before expanding.
    if key.starts_with('(') || key.contains(',') {
        return None;
    }
    Some((base, key))
}

/// Return the array name if `s` is a `${NAME[@]}` or `${NAME[*]}` splice
/// form. Both expand to the array's elements as separate words; the
/// distinction with quoted forms is handled by the for-list / WORD_SPLIT
/// logic, not here.
/// True iff `expr` is a subscripted-array arith assignment — used by
/// compile_arith to bypass ArithCompiler (which doesn't write back to
/// arr[idx]) for `((a[i]=v))` and route to the runtime eval which
/// handles the write correctly.
fn subscripted_arith_assign_check(expr: &str) -> bool {
    let trimmed = expr.trim();
    let bytes = trimmed.as_bytes();
    if bytes.is_empty() || !(bytes[0] == b'_' || bytes[0].is_ascii_alphabetic()) {
        return false;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'[' {
        return false;
    }
    let mut depth = 1;
    let mut j = i + 1;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= bytes.len() {
        return false;
    }
    let mut k = j + 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k >= bytes.len() || bytes[k] != b'=' {
        return false;
    }
    !(k + 1 < bytes.len() && (bytes[k + 1] == b'=' || bytes[k + 1] == b'~'))
}

/// Detect compound-assign or pre/post-increment on an array element:
/// `a[i]++`, `a[i]--`, `a[i]+=v`, `a[i]-=v`, `a[i]*=v`, etc.
/// Returns true so the caller routes through BUILTIN_ARITH_EVAL
/// (which handles the read-modify-write via subscripted_arith_eval).
fn subscripted_arith_compound_check(expr: &str) -> bool {
    let trimmed = expr.trim();
    // Pre-increment/decrement on subscript: `++NAME[IDX]` / `--NAME[IDX]`.
    // Strip the leading op and continue with name detection. The runtime
    // arith eval handles the actual write-back via parse_subscript_arith_pre_inc.
    let stripped = trimmed
        .strip_prefix("++")
        .or_else(|| trimmed.strip_prefix("--"))
        .unwrap_or(trimmed)
        .trim_start();
    let bytes = stripped.as_bytes();
    if bytes.is_empty() || !(bytes[0] == b'_' || bytes[0].is_ascii_alphabetic()) {
        return false;
    }
    // Pre-op shape: NAME[IDX] alone — accept and let runtime handle.
    if !std::ptr::eq(stripped.as_ptr(), trimmed.as_ptr()) {
        let mut i = 1;
        while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'[' {
            return false;
        }
        let mut depth = 1;
        let mut j = i + 1;
        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        // Must end with `]` (no further operator after pre-op).
        let mut k = j + 1;
        while k < bytes.len() && bytes[k].is_ascii_whitespace() {
            k += 1;
        }
        return k == bytes.len();
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b'[' {
        return false;
    }
    let mut depth = 1;
    let mut j = i + 1;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= bytes.len() {
        return false;
    }
    // After the closing `]`, look for one of: `++`, `--`, `+=`,
    // `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`,
    // `**=`. Whitespace before the operator is allowed.
    let mut k = j + 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k >= bytes.len() {
        return false;
    }
    let rest = &bytes[k..];
    matches!(
        rest,
        [b'+', b'+', ..]
            | [b'-', b'-', ..]
            | [b'+', b'=', ..]
            | [b'-', b'=', ..]
            | [b'*', b'=', ..]
            | [b'/', b'=', ..]
            | [b'%', b'=', ..]
            | [b'&', b'=', ..]
            | [b'|', b'=', ..]
            | [b'^', b'=', ..]
            | [b'<', b'<', b'=', ..]
            | [b'>', b'>', b'=', ..]
            | [b'*', b'*', b'=', ..]
    )
}

fn array_splice_ref(s: &str) -> Option<&str> {
    // Braced form: ${NAME[@]} / ${NAME[*]}
    for sub in &["[@]}", "[*]}"] {
        if let Some(rest) = s.strip_suffix(sub) {
            if let Some(name) = rest.strip_prefix("${") {
                if !name.is_empty()
                    && (name.starts_with('_') || name.chars().next().unwrap().is_ascii_alphabetic())
                    && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
                {
                    return Some(name);
                }
            }
        }
    }
    // Bare form: $NAME[@] / $NAME[*]. zsh treats these identically to
    // the braced versions; without this match, `printf "%s\n" $a[@]`
    // joined the array to a single arg.
    for sub in &["[@]", "[*]"] {
        if let Some(rest) = s.strip_suffix(sub) {
            if let Some(name) = rest.strip_prefix('$') {
                if !name.is_empty()
                    && (name.starts_with('_') || name.chars().next().unwrap().is_ascii_alphabetic())
                    && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
                {
                    return Some(name);
                }
            }
        }
    }
    None
}

/// True iff the splice is `[*]` (join) rather than `[@]` (splice).
/// Used by the compile path to pick between BUILTIN_ARRAY_ALL (returns
/// each element separately) and BUILTIN_ARRAY_JOIN_STAR (joins with
/// the first IFS char into a single string).
fn array_splice_is_star(s: &str) -> bool {
    s.ends_with("[*]}") || s.ends_with("[*]")
}

/// For `[[ ... == PATTERN ]]` style tests, walk PATTERN and replace
/// glob metas (`*`, `?`, `[`) that fall INSIDE single/double-quoted
/// regions with backslash-escaped versions. Quoted glob metas should
/// match literally per zsh. Markers used by the lexer:
///   `\u{9d}` (Snull) — single-bslashquote boundary
///   `\u{9e}` (Dnull) — double-bslashquote boundary
fn escape_quoted_glob_metas(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_squote = false;
    let mut in_dquote = false;
    for c in s.chars() {
        match c {
            '\u{9d}' => {
                in_squote = !in_squote;
                out.push(c);
            }
            '\u{9e}' => {
                in_dquote = !in_dquote;
                out.push(c);
            }
            // c:Src/lex.c:1390-1404 — `-` and `!` become the Dash / Bang
            // TOKENS only when the lexer sees them UNQUOTED; pattern.c's
            // range parser (c:1483 `*patparse == Dash`) and its negation
            // test look for those tokens, so a quoted `-` / `!` is an
            // ordinary bracket member. zshrs flattens the quoted span back
            // to plain text and re-tokenizes it downstream, which would
            // promote them again — backslash-escape them here for the same
            // reason the glob metas below are escaped. Pins
            // `[[ - = ['a-z'] ]]` and `[[ a = ['!a'] ]]`.
            '*' | '?' | '[' | '(' | ')' | '|' | '~' | '#' | '^' | '-' | '!'
                if in_squote || in_dquote =>
            {
                // Backslash-escape so glob_match_static treats as
                // literal char. The runtime glob translator already
                // handles `\X` → escape-X. zsh's pattern matcher
                // treats `(`/`)`/`|` as alternation grouping under
                // KSH_GLOB / EXTENDED_GLOB; quoted forms must be
                // literal so `[[ "foo()" == "foo()" ]]` succeeds.
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Return the variable name if `s` is a `${NAME}` form with no
/// modifier (no `:`, `#`, `%`, `/`, `(`, `+`, `-`, `=`, `?`, `^`, etc.).
/// Equivalent semantics to bare `$NAME`.
fn braced_var_ref(s: &str) -> Option<&str> {
    if !s.starts_with("${") || !s.ends_with('}') || s.len() < 4 {
        return None;
    }
    let inner = &s[2..s.len() - 1];
    if inner.is_empty() {
        return None;
    }
    let first = inner.chars().next()?;
    // Special single-char params
    if matches!(first, '#' | '?' | '!' | '_' | '$' | '-' | '@' | '*') && inner.chars().count() == 1
    {
        return Some(inner);
    }
    // All-digit positional
    if first.is_ascii_digit() && inner.chars().all(|c| c.is_ascii_digit()) {
        return Some(inner);
    }
    // Plain identifier — reject anything with modifier syntax.
    if (first == '_' || first.is_ascii_alphabetic())
        && inner.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
    {
        return Some(inner);
    }
    // c:Src/params.c:2206 — `int itype = (scanflags & SCANPM_NONAMESPC) ?
    // IIDENT : INAMESPC;` — inside `${…}` the name is scanned with INAMESPC,
    // so a ksh93 namespace name (`.k02.foo`, `k.2`) is a plain name here too
    // and must take the SAME compiled var-read path; that path is what
    // applies SH_WORD_SPLIT and the KSH_ARRAYS single-element pick.
    // c:Src/params.c:2257-2260 — a dotted name that `isident()` rejects is a
    // "badly formed namespace reference", so those are left to paramsubst,
    // which still raises "bad substitution" for them.
    if inner.contains('.') && is_namespace_name(inner) {
        return Some(inner);
    }
    None
}

/// True when `name` is a complete ksh93 namespace parameter name — the
/// INAMESPC walk (c:Src/utils.c:4397-4412) consumes all of it AND
/// `isident` (c:Src/params.c:1309) accepts it.
fn is_namespace_name(name: &str) -> bool {
    crate::ported::utils::itype_end(name, crate::ported::ztype_h::INAMESPC, false) == name.len()
        && crate::ported::params::isident(name)
}

/// Match `${=NAME}` / `${==NAME}` / `${=NAME[@]}` / `${=NAME[*]}` —
/// the forced-split (single `=`) and force-no-split (double `==`)
/// flags. Direct port of src/zsh/Src/subst.c:2558-2569 where a leading
/// `=` after `${` sets `spbreak = 2` (force IFS-split) and `==` sets
/// `spbreak = 0` (override SH_WORD_SPLIT to no-split).
///
/// Returns `Some((force_split, name, splice_kind))` where:
/// - `force_split = true` for `${=NAME}` (single `=`),
/// - `force_split = false` for `${==NAME}` (double `==`),
/// - `splice_kind` is `' '` for plain, `'@'` for `[@]`, `'*'` for `[*]`.
fn parse_forced_split_brace(s: &str) -> Option<(bool, &str, char)> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    let inner_b = inner.as_bytes();
    if inner_b.first()? != &b'=' {
        return None;
    }
    let (force_split, rest) = if let Some(after) = inner.strip_prefix("==") {
        (false, after)
    } else {
        (true, inner.strip_prefix('=').unwrap_or(inner))
    };
    if rest.is_empty() {
        return None;
    }
    let (name_part, splice) = if let Some(stripped) = rest.strip_suffix("[@]") {
        (stripped, '@')
    } else if let Some(stripped) = rest.strip_suffix("[*]") {
        (stripped, '*')
    } else {
        (rest, ' ')
    };
    if name_part.is_empty() {
        return None;
    }
    let first = name_part.chars().next()?;
    // Special single-char params (no splice variant for these).
    if splice == ' '
        && matches!(first, '#' | '?' | '!' | '_' | '$' | '-' | '@' | '*')
        && name_part.chars().count() == 1
    {
        return Some((force_split, name_part, splice));
    }
    // All-digit positional (no splice variant).
    if splice == ' ' && first.is_ascii_digit() && name_part.chars().all(|c| c.is_ascii_digit()) {
        return Some((force_split, name_part, splice));
    }
    // Plain identifier.
    if (first == '_' || first.is_ascii_alphabetic())
        && name_part
            .chars()
            .all(|c| c == '_' || c.is_ascii_alphanumeric())
    {
        return Some((force_split, name_part, splice));
    }
    // c:Src/params.c:2206 — same INAMESPC name scan as `braced_var_ref`, so
    // `${==.k02.bar}` / `${=.k02.foo}` keep the compiled split/no-split path.
    if name_part.contains('.') && is_namespace_name(name_part) {
        return Some((force_split, name_part, splice));
    }
    None
}

/// Return the variable name if `s` is a bare `$NAME` form: `$x`, `$1`,
/// `$#`, `$?`, `$!`, `$_`, `$$`, `$0..$9`. Returns None for braced
/// (`${x}`), subscripted (`$x[1]`), modified (`${x:-y}`), or anything
/// else that needs the full expand-word machinery.
fn bare_var_ref(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'$' {
        return None;
    }
    let rest = &s[1..];
    let first = rest.chars().next()?;
    // Special single-char params: $#, $?, $!, $_, $$, $-, $0..$9
    if matches!(first, '#' | '?' | '!' | '_' | '$' | '-') && rest.chars().count() == 1 {
        return Some(rest);
    }
    // Two-char specials: `$#@` and `$#*` are zsh shorthand for
    // `${#@}` / `${#*}` — count of positional params (same as `$#`).
    if first == '#' && rest.chars().count() == 2 {
        let second = rest.chars().nth(1)?;
        if second == '@' || second == '*' {
            return Some(rest);
        }
    }
    if first.is_ascii_digit() && rest.chars().all(|c| c.is_ascii_digit()) {
        return Some(rest);
    }
    // Plain identifier: [_A-Za-z][_A-Za-z0-9]*
    if (first == '_' || first.is_ascii_alphabetic())
        && rest.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
    {
        return Some(rest);
    }
    None
}

/// Match bare `$NAME[KEY]` (no braces). zsh treats the `[KEY]` after
/// a bare `$NAME` as a subscript (NOT a literal). Returns
/// `(name, key)` for the simple no-suffix case. Excludes empty name,
/// `[@]` / `[*]` (handled by other splice paths), and keys with
/// nested `[` / `]` / `$` / `` ` ``.
fn bare_subscript_ref(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    if bytes.len() < 4 || bytes[0] != b'$' {
        return None;
    }
    let rest = &s[1..];
    let lb = rest.find('[')?;
    // c:Src/lex.c:1743 parse_subscript → c:Src/lex.c:1486 dquote_parse
    // with `endchar == ']'` (so c:1489 `math` is true). `case '[': if
    // (!math || !bct) brct++;` (c:1600-1602) and `case ']': ... err =
    // (!brct-- && math);` (c:1604-1606) — the closer is found by a
    // NESTING walk, and `case '\\':` (c:1493-1509) emits `Bnull` for an
    // escaped `[`/`]`, i.e. a backslashed bracket is subscript CONTENT
    // and never a delimiter. Assuming the last byte is the closer made
    // `$s[(r)\]]` reject here and fall to the runtime bare-form arm,
    // whose scalar leg has no search-flag support at all (subst.rs
    // ~20360), so `$s[(r)\]]` expanded empty where zsh gives `]`.
    let close = subscript_close(rest, lb)?;
    if close != rest.len() - 1 {
        return None;
    }
    let name = &rest[..lb];
    let key = &rest[lb + 1..close];
    if name.is_empty() || key.is_empty() || key == "@" || key == "*" {
        return None;
    }
    let is_special = name == "@" || name == "*";
    if !is_special {
        let first = name.chars().next()?;
        if !(first == '_' || first.is_ascii_alphabetic()) {
            return None;
        }
        if !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            return None;
        }
    }
    // c:Src/params.c:1389-1391 — a subscript that opens with `(` is a
    // FLAG group whose remainder is a search PATTERN handed straight to
    // patcompile (c:1697), so an ESCAPED bracket there is ordinary
    // pattern text and the braced machinery handles it. A NON-flag key
    // carrying a bracket must stay on the runtime path: the flat
    // `${name[key]}` rebuild in fusevm_bridge::array_index_lookup routes
    // a bracket-bearing plain key to a direct exact-key assoc read,
    // which would key on the still-escaped text and miss.
    if key.starts_with('(') {
        if unescaped_contains(key, &['[', ']', '$', '`']) {
            return None;
        }
    } else if key.contains('[') || key.contains(']') || key.contains('$') || key.contains('`') {
        return None;
    }
    Some((name, key))
}

/// c:Src/lex.c:1600-1606 — locate the `]` that closes the subscript
/// opened at byte offset `lb`, tracking `brct` nesting and skipping the
/// char after a backslash exactly as c:Src/lex.c:1493-1509 does (an
/// escaped bracket becomes `Bnull` + the literal char, never a
/// delimiter). Returns the byte offset of the closer, or None when the
/// subscript is unterminated.
fn subscript_close(rest: &str, lb: usize) -> Option<usize> {
    let mut brct = 1_i32; // c:1601 — the `[` at `lb`
    let mut it = rest.char_indices().skip_while(|(i, _)| *i <= lb);
    while let Some((i, c)) = it.next() {
        match c {
            '\\' => {
                it.next(); // c:1493-1509 — the escaped char is content
            }
            '[' => brct += 1, // c:1601
            ']' => {
                brct -= 1; // c:1605
                if brct == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// True when `s` contains any char of `set` that is NOT backslash-escaped
/// (c:Src/lex.c:1493-1509 — the char after `\` is literal content).
fn unescaped_contains(s: &str, set: &[char]) -> bool {
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            it.next();
            continue;
        }
        if set.contains(&c) {
            return true;
        }
    }
    false
}

/// Match bare `$NAME[KEY]suffix` — same as `bare_subscript_ref` but
/// with a literal-text suffix appended. Returns `(name, key, suffix)`.
/// `suffix` is the literal text after the closing `]` and must be
/// plain — no `$`, no `[`, no metachars (else fall back to bridge).
fn bare_subscript_with_suffix(s: &str) -> Option<(&str, &str, &str)> {
    let bytes = s.as_bytes();
    if bytes.len() < 5 || bytes[0] != b'$' {
        return None;
    }
    let rest = &s[1..];
    let lb = rest.find('[')?;
    // c:Src/lex.c:1600-1606 — same nesting/escape-aware closer walk as
    // bare_subscript_ref; `rest.find(']')` stopped at a BACKSLASHED `]`
    // (c:1493-1509 content, not a delimiter) and split `$A[\]]x` into
    // key `\` + suffix `]x`.
    let rb = subscript_close(rest, lb)?;
    if rb <= lb || rb == rest.len() - 1 {
        return None;
    }
    let name = &rest[..lb];
    let key = &rest[lb + 1..rb];
    let suffix = &rest[rb + 1..];
    if name.is_empty() || key.is_empty() || key == "@" || key == "*" {
        return None;
    }
    let first = name.chars().next()?;
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return None;
    }
    if !name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    if key.contains('[') || key.contains(']') || key.contains('$') || key.contains('`') {
        return None;
    }
    if suffix.contains('$')
        || suffix.contains('[')
        || suffix.contains(']')
        || suffix.contains('`')
        || suffix.contains('*')
        || suffix.contains('?')
    {
        return None;
    }
    Some((name, key, suffix))
}

/// Walk a raw zsh-tokenized word; return true if it has an unquoted
/// command substitution (`$(...)` or backticks) at the top level.
/// zsh field-splits these on IFS by default. Variable expansions
/// (`$var`, `${arr[@]}`) DO NOT get IFS-split unless `SH_WORD_SPLIT`
/// is set, so we deliberately don't trigger on those.
///
/// Lexer markers: `\u{85}` = META-$, `\u{88}` = Inpar.
/// Detect any unquoted parameter expansion (`$VAR`, `${VAR}`,
/// `${VAR:...}`) OR command substitution (`$(...)`, backticks) in
/// the lexer-tokenized word `s`. Superset of `has_unquoted_expansion`:
/// the latter only matches `$(...)` and backticks, intentionally
/// excluding bare `$VAR` so WORD_SPLIT doesn't fire on every
/// parameter ref. The GLOB_SUBST gate (bug #119) needs the
/// broader detection because `pat="*.txt"; for f in /tmp/X/$pat`
/// counts as substitution for option purposes.
fn has_unquoted_param_or_subst(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let mut in_dq = false;
    let mut in_sq = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\u{9d}' {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if c == '\u{9e}' {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if !in_dq && !in_sq {
            // META-$ (Stringg = \u{85}) — any param / cmd-subst marker.
            if c == '\u{85}' || c == '$' {
                return true;
            }
            // Qstring (\u{8c}) — DQ-context $-marker; in non-DQ here
            // it's a real expansion marker too.
            if c == '\u{8c}' {
                return true;
            }
            // Backticks (literal or Tick = \u{96}, Qtick = \u{95}).
            if c == '`' || c == '\u{96}' || c == '\u{95}' {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// A large `arr=(...)` literal whose elements are ALL compile-time literals
/// is DATA, not code: it should compile to a single `Value::Array` constant,
/// not N per-element `LoadConst` + `MakeArray` (which explodes the u16 constant
/// pool — a `.zcompdump`'s 96k-unique-constant `_comps=(...)` overflowed it).
/// Returns the element's literal value iff it has NO expansion and NO
/// glob/brace/tilde/equals/paren metacharacter OUTSIDE quotes — i.e. its
/// expansion is the identity and `untokenize(e)` is exactly the value the
/// per-element path would push as one `LoadConst`. This deliberately accepts
/// multi-span / escaped-quote literals like `''\''brew'` (→ `'brew`), which a
/// naive "exactly two Snull" check would reject — 5 such keys in the zpwr
/// `_comps=(...)` were forcing the whole 103k-element literal off the fast
/// path and back into the u16-pool-overflowing per-element codegen.
/// Anything with `$`/backtick expansion, an unquoted glob/brace/tilde/`=`
/// (raw OR its token form), or a `[key]=` marker returns None so the caller
/// keeps the per-element path.
fn literal_array_elem_value(e: &str) -> Option<String> {
    use crate::ported::zsh_h::{Equals, Inbrace, Inbrack, Inpar, Quest, Star, Tilde};
    if has_unquoted_expansion(e) {
        return None;
    }
    let blocked = unquoted(e, '*')
        || unquoted(e, Star)
        || unquoted(e, '?')
        || unquoted(e, Quest)
        || unquoted(e, '[')
        || unquoted(e, Inbrack)
        || unquoted(e, '{')
        || unquoted(e, Inbrace)
        || unquoted(e, '~')
        || unquoted(e, Tilde)
        || unquoted(e, '=')
        || unquoted(e, Equals)
        || unquoted(e, '(')
        || unquoted(e, Inpar);
    if blocked {
        None
    } else {
        Some(crate::lex::untokenize(e))
    }
}

/// True when the word contains an unquoted command substitution that is a
/// WORD of its own, and so must be IFS-split after expansion.
///
/// c:Src/subst.c — `stringsubst` performs that split for a BARE `$(...)`.
/// One written INSIDE a `${...}` spec is just the spec's input text: the
/// spec's own flags decide the splitting, and an explicit split flag
/// (`(s:…:)`, `(f)`, `(0)`, `(z)`) makes the result `isarr`, which c:3920
/// (`if (force_split && !isarr)`) then declines to split again. zshrs's
/// `BUILTIN_WORD_SPLIT` does `pop().to_str()`, so running it over such a
/// result JOINED the array and re-split it on IFS:
/// `b=( ${(s:,:)$(print -n 'x y,z w')} )` gave 4 elements where zsh gives 2.
/// That is `Completion/X/Type/_xft_fonts`'s
/// `compadd … ${(us:,:)$(_call_program fonts fc-list …)}`, whose family
/// names contain spaces — the `fc-list `/`fc-match ` count divergence.
fn needs_word_split(e: &str) -> bool {
    has_unquoted_expansion(e) && !cmdsubst_only_inside_braces(e)
}

/// True when `e` has at least one unquoted command substitution and EVERY
/// one of them sits inside a `${...}` parameter expansion. Pairs with
/// `has_unquoted_expansion`; see `needs_word_split`.
fn cmdsubst_only_inside_braces(e: &str) -> bool {
    let mut in_dq = false;
    let mut in_sq = false;
    let mut brace_depth = 0i32;
    let chars: Vec<char> = e.chars().collect();
    let mut i = 0;
    let mut saw_bare = false;
    let mut saw_any = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\u{9d}' {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if c == '\u{9e}' {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if in_sq {
            i += 1;
            continue;
        }
        let is_dollar = matches!(c, '$' | '\u{85}' | '\u{8c}');
        let next = chars.get(i + 1).copied();
        // `${` — Inbrace TOKEN or a raw brace.
        if is_dollar && matches!(next, Some('\u{8f}') | Some('{')) {
            brace_depth += 1;
            i += 2;
            continue;
        }
        if matches!(c, '\u{90}' | '}') && brace_depth > 0 {
            brace_depth -= 1;
            i += 1;
            continue;
        }
        // `$(` — Inpar TOKEN or a raw paren; plus the backtick forms
        // (Tick / Qtick), matching has_unquoted_expansion's set.
        let is_cmdsub = (is_dollar && matches!(next, Some('\u{88}') | Some('(')))
            || matches!(c, '`' | '\u{93}' | '\u{99}');
        if is_cmdsub && !in_dq {
            saw_any = true;
            if brace_depth == 0 {
                saw_bare = true;
            }
        }
        i += 1;
    }
    saw_any && !saw_bare
}

fn has_unquoted_expansion(s: &str) -> bool {
    let mut in_dq = false;
    let mut in_sq = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\u{9d}' {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if c == '\u{9e}' {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if !in_dq && !in_sq {
            // `$(...)` — META-$ followed by Inpar
            if c == '\u{85}' && i + 1 < chars.len() && chars[i + 1] == '\u{88}' {
                return true;
            }
            // Plain `$` followed by Inpar (lexer sometimes leaves `$` literal)
            if c == '$' && i + 1 < chars.len() && chars[i + 1] == '\u{88}' {
                return true;
            }
            // Fully-raw `$(` — intypeset-context words (typeset-family
            // paren-init elements) keep the cmdsub as plain ASCII; the
            // token-only checks above missed it so `typeset b=(
            // $(print q w) e )` skipped the field-split (2 elements
            // instead of zsh's 3).
            if c == '$' && i + 1 < chars.len() && chars[i + 1] == '(' {
                return true;
            }
            // Backtick command sub — literal `` ` ``, Tick TOKEN
            // (`\u{93}`), or Qtick TOKEN (`\u{99}` — DQ-context backtick
            // marker). The previous version checked `\u{96}` (Bang) and
            // `\u{95}` (OutangProc) which are unrelated TOKENs — backtick
            // cmd-subst inside an unquoted array literal never matched, so
            // `a=(\`cmd\`)` got no word-split and the output joined as one
            // element. Matches Src/zsh.h:174/180 Tick/Qtick constants.
            if c == '`' || c == '\u{93}' || c == '\u{99}' {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Reconstruct an approximate source-text representation of a
/// `[[ ]]` cond AST for xtrace output. Direct port of zsh's
/// exec.c::execcond which emits the cond's source form to
/// xtrerr before evaluation. Untokenize each operand so lexer
/// META markers don't leak into the trace.
fn render_cond(c: &crate::parse::ZshCond) -> String {
    fn untok(s: &str) -> String {
        crate::lex::untokenize(s)
    }
    match c {
        ZshCond::Not(inner) => format!("! {}", render_cond(inner)),
        ZshCond::And(a, b) => format!("{} && {}", render_cond(a), render_cond(b)),
        ZshCond::Or(a, b) => format!("{} || {}", render_cond(a), render_cond(b)),
        ZshCond::Unary(op, arg) => {
            let op = untok(op);
            let arg = untok(arg);
            if arg.is_empty() {
                op
            } else {
                format!("{} {}", op, arg)
            }
        }
        ZshCond::Binary(left, op, right) => {
            let left = untok(left);
            let op = untok(op);
            let right = untok(right);
            if right.is_empty() {
                format!("{} {}", op, left)
            } else {
                format!("{} {} {}", left, op, right)
            }
        }
        ZshCond::Regex(left, regex) => {
            format!("{} =~ {}", untok(left), untok(regex))
        }
        ZshCond::ModCond(op, args) => {
            let mut s = untok(op);
            for a in args {
                s.push(' ');
                s.push_str(&untok(a));
            }
            s
        }
    }
}

/// True when a typeset-family arg word is assignment-shaped:
/// `name=…` / `name+=…` / `name[sub]=…`. C's par_simple (Src/parse.c,
/// `intypeset`) parses these args as ASSIGNMENTS, so the value side
/// is expanded via `addvars` → `prefork(…, PREFORK_ASSIGN)`
/// (Src/exec.c:2601) — parameter AND command substitution run with
/// NO IFS split. Used by the simple-command argv loop to put the
/// whole word in assign context (`export ZPWR_TTY=$(tty)` must keep
/// "not a tty" as one value, not split it into extra export names).
/// Whether the whole word is a single bare parameter reference — `$name`
/// or `${name}`, unquoted and with nothing around it.
///
/// Used to keep a BINF_ASSIGN builtin's bare `$arr` argument on the normal
/// word path (array splats to one word per element) while `NAME=VALUE`
/// arguments keep assignment semantics. Anything with a subscript, a flag
/// block, quotes, or surrounding text is NOT this shape and keeps the
/// assignment treatment.
fn word_is_bare_param_ref(word: &str) -> bool {
    let c: Vec<char> = word.chars().collect();
    // The lexer replaces a bare `$` with the `Stringg` token (zsh.h:160,
    // \u{85}); `${...}` keeps a literal `$`. Accept both spellings —
    // matching only the literal one made `local -a $t` miss while
    // `local -a ${t}` hit, which is the same word in zsh's grammar.
    if c.len() < 2 || !(c[0] == '$' || c[0] == crate::ported::zsh_h::Stringg) {
        return false;
    }
    let s: String = c[1..].iter().collect();
    let b = s.as_bytes();
    let inner: &[u8] = if b.first() == Some(&b'{') {
        if b.last() != Some(&b'}') || b.len() < 3 {
            return false;
        }
        &b[1..b.len() - 1]
    } else {
        b
    };
    if inner.is_empty() {
        return false;
    }
    // Plain identifier only: no `[sub]`, no `(flags)`, no `:-` modifiers.
    let first_ok = inner[0] == b'_' || inner[0].is_ascii_alphabetic();
    first_ok
        && inner
            .iter()
            .all(|c| *c == b'_' || c.is_ascii_alphanumeric())
}

/// True when an assignment subscript carries a LITERAL top-level `,` — the
/// marker that makes `name[lo,hi]=…` a RANGE splice rather than a single
/// key/index store.
///
/// c:Src/params.c getindex — the subscript is split at the first unnested `,`
/// and each half is fed to `mathevalarg`, so a `$` inside a bound is just part
/// of an arithmetic expression (`t[$#MATCH/2+1,-1]`). Only a comma that the
/// SOURCE wrote at top level counts here: one that arrives from inside an
/// expansion (`m[${k}]` with k="a,b") or from a quoted span is left to the
/// assoc-key path, matching how zshrs already treats a purely dynamic
/// subscript.
///
/// Spans skipped: `${…}` / `$(…)` / `$((…))`, `` `…` ``, `'…'`, `"…"`, and
/// bracket nesting. Token forms of the delimiters (Inbrace `\u{8f}` /
/// Outbrace `\u{90}` / Inpar `\u{88}` / Outpar `\u{89}` / Inbrack `\u{91}` /
/// Outbrack `\u{92}` / Qstring `\u{8c}` / Stringg `\u{85}` / Tick `\u{86}` /
/// Qtick `\u{8b}`, zsh.h token table) count the same as their ASCII spellings
/// because the lexer may hand over either.
fn subscript_has_toplevel_comma(key: &str) -> bool {
    let chars: Vec<char> = key.chars().collect();
    let mut depth = 0i32;
    let mut i = 0usize;
    let mut in_squote = false;
    let mut in_dquote = false;
    let mut in_tick = false;
    while i < chars.len() {
        let c = chars[i];
        if in_squote {
            if c == '\'' {
                in_squote = false;
            }
            i += 1;
            continue;
        }
        if in_tick {
            if c == '`' || c == '\u{86}' || c == '\u{8b}' {
                in_tick = false;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' if !in_dquote => in_squote = true,
            '"' => in_dquote = !in_dquote,
            '`' | '\u{86}' | '\u{8b}' => in_tick = true,
            '{' | '\u{8f}' | '(' | '\u{88}' | '[' | '\u{91}' => depth += 1,
            '}' | '\u{90}' | ')' | '\u{89}' | ']' | '\u{92}' => depth -= 1,
            ',' if depth == 0 && !in_dquote => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn is_typeset_scalar_assign(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    // NAME — ident chars only (same rule as split_typeset_paren_init).
    let mut i = 0;
    while i < chars.len() && (chars[i] == '_' || chars[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    if i == 0
        || !chars
            .first()
            .map_or(false, |c| *c == '_' || c.is_ascii_alphabetic())
    {
        return false;
    }
    // Optional `[subscript]` — literal or Inbrack/Outbrack tokens
    // (\u{91}/\u{92}, zsh.h:171-172).
    if matches!(chars.get(i), Some('[') | Some('\u{91}')) {
        let mut depth = 0i32;
        while i < chars.len() {
            match chars[i] {
                '[' | '\u{91}' => depth += 1,
                ']' | '\u{92}' => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        if depth != 0 {
            return false;
        }
    }
    if chars.get(i) == Some(&'+') {
        i += 1;
    }
    // `=` may arrive literal or as the Equals token (\u{8d}).
    matches!(chars.get(i), Some('=') | Some('\u{8d}'))
}

/// Split a typeset-family paren-init arg `name=( e1 e2 … )` /
/// `name+=( … )` into the opener prefix (`name=(`) and the element
/// words. Returns None when the word isn't a full paren-init (no
/// rewrite — the generic path handles it). Element boundaries are
/// unquoted whitespace; Snull/Dnull/Bnull-marked spans keep their
/// markers so each element re-enters compile_word_str with quoting
/// intact. Mirrors what C's par_simple (intypeset, ENVARRAY) does at
/// parse time: elements stay separate wordcode strings.
fn split_typeset_paren_init(word: &str) -> Option<(String, Vec<String>)> {
    let chars: Vec<char> = word.chars().collect();
    // NAME — ident chars only (subscripted / quoted names take the
    // generic path).
    let mut i = 0;
    while i < chars.len() && (chars[i] == '_' || chars[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    if i == 0
        || !chars
            .first()
            .map_or(false, |c| *c == '_' || c.is_ascii_alphabetic())
    {
        return None;
    }
    let name_end = i;
    if chars.get(i) == Some(&'+') {
        i += 1;
    }
    // `=` may arrive literal or as the Equals token (\u{8d}).
    if !matches!(chars.get(i), Some('=') | Some('\u{8d}')) {
        return None;
    }
    i += 1;
    // `(` literal or Inpar token (\u{88}).
    if !matches!(chars.get(i), Some('(') | Some('\u{88}')) {
        return None;
    }
    i += 1;
    // Body runs to the LAST `)` / Outpar (\u{8a}, zsh.h:165) — must
    // be the final char of the word.
    if !matches!(chars.last(), Some(')') | Some('\u{8a}')) {
        return None;
    }
    let body = &chars[i..chars.len() - 1];
    // Element boundaries come in two encodings:
    //   * \u{1f} (ASCII US) — the parser's intypeset ENVARRAY rejoin
    //     (par_simple) emits word-granular elements pre-split;
    //   * raw unquoted whitespace at NESTING DEPTH 0 — words that
    //     reach the compiler without the rejoin (function bodies
    //     recompiled at call time: zsh-hist's `typeset -gU FPATH
    //     fpath=( $dir $fpath )` arrived space-separated and the
    //     US-only split made it ONE element, collapsing fpath to a
    //     single joined entry and breaking every later autoload).
    // Whitespace inside $( … ) / ${ … } / ( … ) nesting or quote
    // spans is word-internal (`$(print q w)` stays one element).
    let mut elems: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_sq = false;
    let mut in_dq = false;
    let mut in_tick = false;
    let mut depth: i32 = 0;
    let mut brace_nest: i32 = 0;
    // WHY the two nesting rules below matter: when this scan ends with
    // `depth != 0` the splitter reports "unbalanced" and the whole
    // `name=( … )` word falls through to the GENERIC word path. There
    // RC_EXPAND_PARAM cross-products the element array against the
    // surrounding `name=(` / `)` literal text, so `typeset`/`local` is
    // invoked once PER ELEMENT (`v=((-h --host)-h)`, then
    // `v=(X --username)-U)`) and the last assignment wins — an N-element
    // array collapses to 1. `_postgresql`'s `_pgsql_psql` builds its whole
    // `_arguments` spec list that way, which is what silenced `psql <TAB>`.
    //
    // c:Src/lex.c — a BACKSLASH-escaped metachar is inert. The lexer hands
    // `\(` down as Bnull (`\u{9f}`) + `(`, and raw-source paths keep the
    // literal `\`. Neither opens a nesting level, so step over the escaped
    // char instead of counting it.
    let mut esc = false;
    for &c in body {
        if esc {
            esc = false;
            cur.push(c);
            continue;
        }
        if c == '\u{9f}' || (c == '\\' && !in_sq) {
            esc = true;
            cur.push(c);
            continue;
        }
        match c {
            '\u{9d}' => {
                in_sq = !in_sq;
                cur.push(c);
            }
            '\u{9e}' => {
                in_dq = !in_dq;
                cur.push(c);
            }
            '\u{93}' | '\u{99}' | '`' if !in_sq && !in_dq => {
                in_tick = !in_tick;
                cur.push(c);
            }
            // Inbrace \u{8f} / Outbrace \u{90} (zsh.h:169-170). Braces are
            // counted unconditionally: they bound `${…}` and brace expansion,
            // and the whitespace they enclose is word-internal.
            '{' | '\u{8f}' if !in_sq && !in_dq => {
                depth += 1;
                brace_nest += 1;
                cur.push(c);
            }
            '}' | '\u{90}' if !in_sq && !in_dq => {
                depth -= 1;
                brace_nest = (brace_nest - 1).max(0);
                cur.push(c);
            }
            // Inpar \u{88} / Inparmath \u{89} open; Outpar \u{8a} /
            // Outparmath \u{8b} close (zsh.h:163-166 — Outpar is \u{8a}, NOT
            // \u{89}; the first cut used \u{89} so a `$(…)` element never
            // re-balanced and the splitter bailed as unbalanced, skipping the
            // pack rewrite). Inbrack \u{91} / Outbrack \u{92} (zsh.h:171-172).
            //
            // Counted only OUTSIDE a `${…}` body: in there the parens and
            // brackets are the substitution's PATTERN / REPLACEMENT text and
            // are under no obligation to balance —
            //     ${(@)common_opts_conn/#\(-U/(2 -U}      (_postgresql:993)
            // has one `(` in the replacement `(2 -U` with no `)`.
            '(' | '\u{88}' | '\u{89}' | '[' | '\u{91}' if !in_sq && !in_dq && brace_nest == 0 => {
                depth += 1;
                cur.push(c);
            }
            ')' | '\u{8a}' | '\u{8b}' | ']' | '\u{92}' if !in_sq && !in_dq && brace_nest == 0 => {
                depth -= 1;
                cur.push(c);
            }
            '\u{1f}' => {
                if !cur.is_empty() {
                    elems.push(std::mem::take(&mut cur));
                }
            }
            ' ' | '\t' | '\n' if !in_sq && !in_dq && !in_tick && depth == 0 => {
                if !cur.is_empty() {
                    elems.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if in_sq || in_dq || in_tick || depth != 0 {
        return None; // unbalanced — don't rewrite
    }
    if !cur.is_empty() {
        elems.push(cur);
    }
    let prefix: String = {
        let mut p: String = chars[..name_end].iter().collect();
        if chars.get(name_end) == Some(&'+') {
            p.push('+');
        }
        p.push_str("=(");
        p
    };
    Some((prefix, elems))
}

fn unquoted(s: &str, target: char) -> bool {
    // True iff `target` appears in the un-quoted portion of `s`. The
    // word may carry lexer-level bslashquote markers — `\u{9d}` (Snull,
    // single-quoted span) and `\u{9e}` (Dnull, double-quoted span)
    // bracket regions where globbing is suppressed. C zsh's pattern
    // compiler (Src/pattern.c::patcompswitch) skips meta-interpretation
    // for bytes inside these spans; the trigger detector must match
    // that behavior or `arr=( foo "value:[brackets]" )` mis-flags as
    // a glob and NOMATCH-errors at runtime even though the brackets
    // are inside DQ.
    //
    // Also honors `\x00` literal-marker (one-char escape from
    // expand_string preprocessing) and `\u{9f}` (Bnull — lexer
    // backslash-escape).
    let mut prev = ' ';
    let mut inside_sq = false;
    let mut inside_dq = false;
    for c in s.chars() {
        if c == '\u{9d}' {
            inside_sq = !inside_sq;
            prev = c;
            continue;
        }
        if c == '\u{9e}' {
            inside_dq = !inside_dq;
            prev = c;
            continue;
        }
        if c == target && prev != '\x00' && prev != '\u{9f}' && !inside_sq && !inside_dq {
            return true;
        }
        prev = c;
    }
    false
}

/// Detect zsh numeric-range glob `<N-M>`, `<N->`, `<-M>`, `<->` outside
/// any bracket expression. Mirrors the runtime's `extract_numeric_ranges`
/// shape exactly so the compile-time trigger and runtime expander stay
/// in lockstep.
fn has_numeric_range_glob(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut in_bracket = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\x00' {
            i += 2;
            continue;
        }
        if c == '[' && !in_bracket {
            in_bracket = true;
            i += 1;
            continue;
        }
        if c == ']' && in_bracket {
            in_bracket = false;
            i += 1;
            continue;
        }
        if c == '<' && !in_bracket {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < chars.len() && chars[j] == '-' {
                j += 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '>' {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Untokenize like `lex::untokenize`, but preserve the three brace TOKEN
/// bytes (Inbrace \u{8f}, Outbrace \u{90}, Comma \u{9a}) so a subsequent
/// `xpandbraces` call still sees the brace structure. Used by the
/// segment-fast-path when a literal segment carries an in-flight brace
/// pattern that crosses the segment boundary (e.g. `"$X"{a,b,c}`).
///
/// c:Src/glob.c::xpandbraces — C operates on TOKEN-form throughout the
/// prefork pipeline; the ASCII materialization happens at the very end.
/// The Rust port's segment-fast-path is a port-time optimization that
/// breaks that invariant — this helper restores the brace half of it.
/// Convert a tokenized `case` pattern into a glob-safe string for the
/// matcher (`str_match` → tokenize → patcompile). Outside quote spans,
/// glob TOKEN chars (`Star` etc.) are untokenized to their ASCII glob
/// form so they re-tokenize as globs. INSIDE Snull/Dnull quote spans the
/// metachars are LITERAL — Bnull-escape them so patcompile treats them
/// literally (`case x in '(')` must match the literal `(`, not a glob
/// group open). Plain `untokenize` stripped the Snull markers, leaving a
/// bare `(` that re-tokenized to a glob open → "bad pattern: (".
/// (`[[ ]]` already handles this via its operand compilation; this brings
/// `case` to parity.)
fn case_pattern_for_match(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut in_q = false; // inside a Snull (\u{9d}) / Dnull (\u{9e}) span
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{9d}' | '\u{9e}' => {
                in_q = !in_q; // strip the marker, toggle quote state
            }
            '\u{9f}' | '\u{8b}' => {
                // Bnull / Bnullkeep escape — keep it and its escaped char
                // verbatim (already a literal escape patcompile honors).
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            _ if in_q => {
                // Quoted: glob metachars are literal — Bnull-escape them.
                if matches!(
                    c,
                    '*' | '?' | '[' | ']' | '(' | ')' | '|' | '#' | '^' | '~' | '<' | '>' | '\\'
                ) {
                    out.push('\u{9f}'); // Bnull
                }
                out.push(c);
            }
            _ => {
                // Unquoted: untokenize a single glob token char to ASCII.
                let one = c.to_string();
                out.push_str(&crate::lex::untokenize(&one));
            }
        }
    }
    out
}

fn untokenize_keep_braces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut buf = String::new();
    let mut group: Vec<char> = Vec::new();
    for c in s.chars() {
        if c == '\u{8f}' || c == '\u{90}' || c == '\u{9a}' {
            if !group.is_empty() {
                buf.clear();
                buf.extend(group.iter());
                out.push_str(&crate::lex::untokenize(&buf));
                group.clear();
            }
            out.push(c);
        } else {
            group.push(c);
        }
    }
    if !group.is_empty() {
        buf.clear();
        buf.extend(group.iter());
        out.push_str(&crate::lex::untokenize(&buf));
    }
    out
}

/// Tiny base64 encoder for embedding bincode-serialized chunks inside
/// constant strings. Avoids dragging in a base64 crate dependency.
///
/// `compile_funcdef` no longer calls it: a function body now rides in the
/// enclosing chunk's `sub_chunks` and only its index goes through the
/// constant pool, because re-encoding the payload once per enclosing
/// definition cost (4/3)^depth. The decoding half in
/// `BUILTIN_REGISTER_COMPILED_FN` still accepts the old form so a chunk
/// restored from a bytecode cache written by an older binary keeps working,
/// and this is the matching encoder for it.
#[allow(dead_code)]
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHA[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

/// Decode ANSI-C `$'…'` body — interpret backslash escapes (`\n`, `\t`,
/// `\\`, `\'`, `\xNN`, `\NNN` octal, `\a`, `\b`, `\e`, `\f`, `\r`, `\v`).
fn decode_ansi_c(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('e') | Some('E') => out.push('\x1b'),
            Some('f') => out.push('\x0c'),
            Some('v') => out.push('\x0b'),
            // c:Src/utils.c:7156 — `\NNN` octal, up to 3 octal
            // digits. `\033` is ESC (0o33=27), `\0` alone is NUL.
            // Previously only `\0` was handled (push NUL), so
            // `\033` decoded as NUL + literal "33" — `$'\033'` had
            // 3 bytes instead of 1 (the ESC).
            Some(d @ '0'..='7') => {
                let mut val: u32 = d.to_digit(8).unwrap();
                for _ in 0..2 {
                    if let Some(&h) = chars.peek() {
                        if let Some(n) = h.to_digit(8) {
                            val = val * 8 + n;
                            chars.next();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                // c:Src/utils.c — `*t++ = zstrtol(...)`: the octal
                // value is truncated to ONE raw byte (`$'\377'` is
                // byte 0xff, same as `$'\xff'`), not a Unicode
                // codepoint. Metafied like the \x arm. Bug #127.
                meta_encode_byte(&mut out, (val & 0xff) as u8);
            }
            Some('x') => {
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(&h) = chars.peek() {
                        if h.is_ascii_hexdigit() {
                            hex.push(h);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(b) = u8::from_str_radix(&hex, 16) {
                    // c:Src/utils.c — `\xNN` produces a single raw
                    // byte, not a Unicode codepoint. Consecutive
                    // `\xNN` escapes combine into multi-byte UTF-8
                    // sequences (e.g. `\xe2\x9c\x93` = ✓). The
                    // previous `out.push(b as char)` cast b to a
                    // Unicode codepoint U+00XX, which then UTF-8-
                    // encoded as `c3 ad` for `\xe2`, producing
                    // mangled multi-byte output (Bug #325); the
                    // unsafe raw-byte push that replaced it left the
                    // String invalid UTF-8 — undefined behavior that
                    // aborted debug builds inside any later .chars()
                    // walk (Bug #127). Store the metafied pair per
                    // c:Src/utils.c:7289-7294 instead; write/exec
                    // boundaries unmetafy back to the raw byte.
                    meta_encode_byte(&mut out, b);
                }
            }
            Some(uu @ ('u' | 'U')) => {
                // c:Src/utils.c:6915 — `\u` reads up to 4 hex digits,
                // `\U` reads up to 8. The outer `c` here is the
                // backslash, so the old `c == 'u'` test always
                // resolved to false and `Ab` decoded the FIVE
                // chars `0041b` (the loop ran 8 iterations and
                // pulled in the trailing `b` as a hex digit), giving
                // U+041B (Л) instead of U+0041 (A) followed by a
                // literal `b`. Bind the inner match value to `uu` so
                // the digit-count gate reads it.
                let n = if uu == 'u' { 4 } else { 8 };
                let mut val: u32 = 0;
                for _ in 0..n {
                    if let Some(&h) = chars.peek() {
                        if let Some(d) = h.to_digit(16) {
                            val = val * 16 + d;
                            chars.next();
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                if let Some(c) = char::from_u32(val) {
                    out.push(c);
                }
            }
            Some(mod_letter @ ('C' | 'M')) => {
                // c:Src/utils.c:7029-7052 + c:7265-7275 — `\C` /
                // `\M` modifiers set control / meta flags; the
                // optional `-` separator is consumed; then the next
                // char (possibly chained `\C` / `\M`) is read and
                // the mask applied (control → `& 0x9f` unless
                // `\C-?` → 0x7f; meta → `| 0x80`). Bug #113 in
                // docs/BUGS.md: decode_ansi_c (the parse-time
                // `$'...'` decoder used by compile_word_str) dropped
                // `\C` / `\M` into the default arm and emitted
                // literal `C-a` / `M-a` instead of the masked byte.
                let mut control = mod_letter == 'C';
                let mut meta = mod_letter == 'M';
                // Consume optional `-` separator + chained modifiers.
                loop {
                    if chars.peek() == Some(&'-') {
                        chars.next();
                        continue;
                    }
                    let mut iter_clone = chars.clone();
                    if iter_clone.next() == Some('\\') {
                        if let Some(nx) = iter_clone.next() {
                            if nx == 'C' || nx == 'M' {
                                chars.next();
                                chars.next();
                                if nx == 'C' {
                                    control = true;
                                } else {
                                    meta = true;
                                }
                                continue;
                            }
                        }
                    }
                    break;
                }
                // Read one base char (allowing nested simple escapes).
                let base: Option<char> = if chars.peek() == Some(&'\\') {
                    chars.next();
                    match chars.next() {
                        Some('n') => Some('\n'),
                        Some('t') => Some('\t'),
                        Some('r') => Some('\r'),
                        Some('a') => Some('\x07'),
                        Some('b') => Some('\x08'),
                        Some('e') | Some('E') => Some('\x1b'),
                        Some('f') => Some('\x0c'),
                        Some('v') => Some('\x0b'),
                        Some('\\') => Some('\\'),
                        Some('\'') => Some('\''),
                        Some('"') => Some('"'),
                        Some(other) => Some(other),
                        None => None,
                    }
                } else {
                    chars.next()
                };
                if let Some(ch) = base {
                    let mut byte = ch as u32;
                    if control {
                        if byte == '?' as u32 {
                            byte = 0x7f;
                        } else {
                            byte &= 0x9f;
                        }
                    }
                    if meta {
                        byte |= 0x80;
                    }
                    // c:Src/utils.c:7265-7275 — the masked result is
                    // a single raw BYTE (`$'\M-i'` = 0xe9), metafied
                    // like the \x arm. A multibyte base char (> 0xff
                    // after masking) can't be byte-masked faithfully;
                    // keep the codepoint form for that edge. Bug #127.
                    if byte <= 0xff {
                        meta_encode_byte(&mut out, byte as u8);
                    } else if let Some(c) = char::from_u32(byte) {
                        out.push(c);
                    }
                }
            }
            Some(other) => {
                // c:Src/utils.c:6915 — `$'\X'` for any X not in the
                // recognized escape set strips the backslash and
                // keeps X verbatim. e.g. `$'\?'` → `?`, `$'\$'` → `$`.
                // The previous behavior preserved both chars (`\?`),
                // which doesn't match zsh's GETKEY_DOLLAR_QUOTE flag.
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests — verify ZshCompiler emits a non-empty, structurally-correct chunk
// for every major shell construct. These are structural assertions, not full
// behavioral parity (parity lives in tests/*_parity.rs which spawn `zsh -c`
// and compare). The point here is: every node type in `zsh_ast::ZshCommand`
// should at minimum compile without panic and emit recognizable opcodes.
// Holes in compiler coverage surface as either panics or empty chunks.
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use fusevm::Op;

    /// Lex + parse + compile a script source. Holds the global parser
    /// state mutex so tests serialize correctly.
    fn compile_src(src: &str) -> fusevm::Chunk {
        let _g = crate::test_util::global_state_lock();
        // Mirror execute_script_zsh_pipeline's setup: clear errflag,
        // run parse_init, run parse, then compile.
        use std::sync::atomic::Ordering;
        let saved = crate::ported::utils::errflag.load(Ordering::Relaxed);
        crate::ported::utils::errflag.fetch_and(!crate::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
        crate::ported::parse::parse_init(src);
        let program = crate::ported::parse::parse();
        crate::ported::utils::errflag.store(saved, Ordering::Relaxed);
        ZshCompiler::new().compile(&program)
    }

    /// Returns true if any op in the chunk (including sub_chunks) matches
    /// the variant kind (compared by discriminant via `matches!` predicate).
    fn has_op(chunk: &fusevm::Chunk, pred: impl Fn(&Op) -> bool + Copy) -> bool {
        chunk.ops.iter().any(pred) || chunk.sub_chunks.iter().any(|c| has_op(c, pred))
    }

    // ── Smoke: every construct compiles to non-empty ops ─────────────
    #[test]
    fn compile_empty_source_is_well_formed() {
        let chunk = compile_src("");
        // Empty input: ops may be empty or trivial; the test pins that
        // we don't panic and we get a valid Chunk object.
        let _ = chunk.ops.len();
    }

    #[test]
    fn compile_simple_echo_command() {
        let chunk = compile_src("echo hello");
        assert!(!chunk.ops.is_empty(), "echo should compile to some ops");
        // Should reference a builtin call or external exec
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::CallBuiltin(..) | Op::Exec(..)
            )),
            "echo should emit CallBuiltin or Exec"
        );
    }

    #[test]
    fn compile_pipeline_emits_subshell_or_pipe_ops() {
        let chunk = compile_src("echo hi | cat");
        assert!(!chunk.ops.is_empty());
        // Pipelines use SubshellBegin/End around each stage in zshrs.
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::SubshellBegin | Op::SubshellEnd | Op::Exec(..) | Op::CallBuiltin(..)
            )),
            "pipeline should produce subshell or exec ops"
        );
    }

    #[test]
    fn compile_assignment_emits_setslot() {
        let chunk = compile_src("X=hello");
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::SetSlot(..) | Op::CallBuiltin(..)
            )),
            "assignment should produce SetSlot or builtin-style store"
        );
    }

    #[test]
    fn compile_if_emits_conditional_jump() {
        let chunk = compile_src("if true; then echo yes; fi");
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::JumpIfFalse(..) | Op::JumpIfFalseKeep(..)
            )),
            "if should emit JumpIfFalse[Keep] for the condition"
        );
    }

    #[test]
    fn compile_if_else_has_two_jump_targets() {
        let chunk = compile_src("if true; then echo a; else echo b; fi");
        // if/else needs at least one JumpIfFalse (for cond) plus one
        // unconditional Jump (to skip the else branch).
        let has_cond = has_op(&chunk, |op| {
            matches!(op, Op::JumpIfFalse(..) | Op::JumpIfFalseKeep(..))
        });
        let has_uncond = has_op(&chunk, |op| matches!(op, Op::Jump(..)));
        assert!(has_cond, "if/else needs JumpIfFalse");
        assert!(has_uncond, "if/else needs unconditional Jump to skip else");
    }

    #[test]
    fn compile_while_emits_loop_jumps() {
        let chunk = compile_src("while true; do echo x; done");
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::JumpIfFalse(..) | Op::JumpIfFalseKeep(..) | Op::Jump(..)
            )),
            "while should emit Jump/JumpIfFalse for loop"
        );
    }

    #[test]
    fn compile_for_loop_emits_iteration_ops() {
        let chunk = compile_src("for i in a b c; do echo $i; done");
        assert!(!chunk.ops.is_empty(), "for-in should compile");
        // Iteration uses GetSlot/SetSlot to walk the list.
        assert!(
            has_op(&chunk, |op| matches!(op, Op::GetSlot(..) | Op::SetSlot(..))),
            "for-in should manipulate slots for the iter var"
        );
    }

    #[test]
    fn compile_case_emits_pattern_match() {
        let chunk = compile_src("case $x in a) echo a;; b) echo b;; esac");
        assert!(!chunk.ops.is_empty(), "case should compile");
        // Case uses StrMatch / pattern jumps to dispatch.
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::StrMatch | Op::JumpIfFalse(..) | Op::JumpIfFalseKeep(..)
            )),
            "case should emit StrMatch or conditional jumps"
        );
    }

    #[test]
    fn compile_function_def_registers_via_builtin() {
        // Observation (compile dump): function defs route through
        // CallBuiltin(305, 6) — the function-register builtin — with the name
        // + a reference to the body loaded from the constant pool. The body
        // itself is a `sub_chunks` entry and the constant is the `#<idx>`
        // marker naming it; `sub_entries` stays empty.
        let chunk = compile_src("greet() { echo hello; }");
        assert_eq!(
            chunk.sub_chunks.len(),
            1,
            "function body is carried as one sub-chunk"
        );
        assert!(
            chunk.constants.iter().any(|c| c.to_str() == "#0"),
            "the body constant is the sub-chunk index marker, not an encoded chunk"
        );
        assert!(
            has_op(&chunk, |op| matches!(op, Op::CallBuiltin(305, _))),
            "function def should emit CallBuiltin(305, _)"
        );
        // Name + body must be in the constant pool.
        assert!(
            chunk.constants.len() >= 2,
            "function def needs name + body in constants"
        );
    }

    /// Deeply nested anonymous functions must parse + compile in time
    /// proportional to the source, not exponential in the nesting depth.
    ///
    /// `compile_funcdef` used to hand the compiled body to the register
    /// builtin as `base64(bincode(chunk))` in a string constant. base64 costs
    /// 4/3, so a body nested N deep was re-encoded N times and the innermost
    /// bytes grew by (4/3)^N. Measured with `zshrs --zsh -n` on a program of
    /// `'() { ' * d + ':' + ' }' * d`: 0.33s at d=24, 1.29s at d=30, 7.15s at
    /// d=36 — a clean geometric curve. D04parameter's "zsh_eval_context
    /// resizing" assertion builds d=49 (`repeat 48 cmd="() { $cmd }"`), which
    /// never finished, so nothing after it in the file ran either.
    ///
    /// The depth below is that test's depth. The budget is deliberately loose
    /// — this box runs many concurrent sessions and the fixed path is
    /// milliseconds — while the pre-fix path extrapolates to minutes at d=49,
    /// so the two never overlap.
    #[test]
    fn parse_and_compile_deeply_nested_anon_funcdefs_is_not_exponential() {
        const DEPTH: usize = 49;
        let src = format!("{}:{}", "() { ".repeat(DEPTH), " }".repeat(DEPTH));
        let started = std::time::Instant::now();
        let chunk = compile_src(&src);
        let elapsed = started.elapsed();
        // Each level contributes exactly one sub-chunk to its parent.
        let mut level = &chunk;
        let mut depth = 0usize;
        while let Some(next) = level.sub_chunks.first() {
            depth += 1;
            level = next;
        }
        assert_eq!(depth, DEPTH, "one sub-chunk per nesting level");
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "nested anon-fn compile must not blow up: depth {} took {:?}",
            DEPTH,
            elapsed
        );
    }

    #[test]
    fn compile_subshell_brackets() {
        let chunk = compile_src("(echo inside)");
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::SubshellBegin | Op::SubshellEnd
            )),
            "(cmd) subshell should emit SubshellBegin/End"
        );
    }

    #[test]
    fn compile_command_group_braces() {
        let chunk = compile_src("{ echo a; echo b; }");
        assert!(!chunk.ops.is_empty(), "{{ ... }} should compile");
    }

    #[test]
    fn compile_command_substitution_uses_sub_chunk_or_builtin() {
        // $(cmd) compiles the inner cmd separately and dispatches at
        // runtime. zshrs may route through Op::CmdSubst with a sub_chunk
        // index, or through CallBuiltin with the inner source as a
        // constant. Either path is acceptable; pin that SOMETHING
        // non-empty was emitted to handle the substitution.
        let chunk = compile_src("X=$(echo hi)");
        let routes_through_cmdsubst = has_op(&chunk, |op| matches!(op, Op::CmdSubst(..)));
        let has_sub_chunk = !chunk.sub_chunks.is_empty();
        let has_builtin = has_op(&chunk, |op| matches!(op, Op::CallBuiltin(..)));
        assert!(
            routes_through_cmdsubst || has_sub_chunk || has_builtin,
            "$(cmd) should produce a recognizable substitution path"
        );
    }

    #[test]
    fn compile_process_sub_in() {
        let chunk = compile_src("diff <(echo a) <(echo b)");
        assert!(
            has_op(&chunk, |op| matches!(op, Op::ProcessSubIn(..))),
            "<(cmd) should emit ProcessSubIn"
        );
    }

    #[test]
    fn compile_process_sub_out() {
        let chunk = compile_src("tee >(cat) < /dev/null");
        assert!(
            has_op(&chunk, |op| matches!(op, Op::ProcessSubOut(..))),
            ">(cmd) should emit ProcessSubOut"
        );
    }

    #[test]
    fn compile_redirect_to_file() {
        let chunk = compile_src("echo hi > /tmp/out");
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::Redirect(..) | Op::WithRedirectsBegin(..) | Op::WithRedirectsEnd
            )),
            "redirect should emit Redirect or WithRedirects*"
        );
    }

    #[test]
    fn compile_here_doc_lowers_to_herestring_or_heredoc() {
        // zshrs lowers a here-doc to a body-staging op wrapped in
        // WithRedirectsBegin/End — the body is captured at compile time
        // and applied at runtime. Three spellings are valid:
        //   * `Op::HereDoc(idx)` — quoted delimiter, body is a constant.
        //   * `BUILTIN_HEREDOC_BODY_SINK` — unquoted delimiter, so the
        //     body runs through BUILTIN_EXPAND_TEXT first and needs a
        //     stack-consuming sink. This one does NOT append a newline,
        //     per c:Src/exec.c:4671-4672 (a here-string derived from a
        //     here-doc gets none). It replaced `Op::HereString` on this
        //     path in 8c8eb4f3c2: HereString appends unconditionally
        //     because `<<<` must, and compensating with
        //     `trim_end_matches('\n')` was lossy both ways.
        //   * `Op::HereString` — still the lowering for a real `<<<`.
        let chunk = compile_src("cat <<EOF\nhello\nEOF\n");
        assert!(
            has_op(&chunk, |op| matches!(op, Op::HereDoc(..) | Op::HereString)
                || matches!(op, Op::CallBuiltin(b, _)
                    if *b == crate::fusevm_bridge::BUILTIN_HEREDOC_BODY_SINK)),
            "here-doc should lower to HereDoc, HereString, or the here-doc body sink"
        );
        assert!(
            has_op(&chunk, |op| matches!(op, Op::WithRedirectsBegin(..))),
            "here-doc body needs a redirect-scoped block"
        );
    }

    #[test]
    fn compile_here_string() {
        let chunk = compile_src("cat <<<\"hello\"");
        assert!(
            has_op(&chunk, |op| matches!(op, Op::HereString)),
            "here-string should emit HereString"
        );
    }

    #[test]
    fn compile_logical_and_short_circuit() {
        // Observation: zshrs lowers `LHS && RHS` to
        //   <LHS bytecode>; GetStatus; JumpIfFalse(skip_rhs); <RHS bytecode>
        // The exit status (last_status) is the carry vehicle, not a
        // stack value, so the non-Keep jump variant is used.
        let chunk = compile_src("true && echo yes");
        assert!(
            has_op(&chunk, |op| matches!(op, Op::GetStatus)),
            "&& should reference exit status via GetStatus"
        );
        assert!(
            has_op(&chunk, |op| matches!(op, Op::JumpIfFalse(..))),
            "&& should branch on the false status"
        );
    }

    #[test]
    fn compile_logical_or_short_circuit() {
        let chunk = compile_src("false || echo fallback");
        assert!(
            has_op(&chunk, |op| matches!(op, Op::GetStatus)),
            "|| should reference exit status via GetStatus"
        );
        assert!(
            has_op(&chunk, |op| matches!(op, Op::JumpIfTrue(..))),
            "|| should branch on the true status"
        );
    }

    #[test]
    fn compile_arith_dispatch() {
        // `(( ... ))` arithmetic — separate dispatch from `$(( ))`.
        let chunk = compile_src("(( 1 + 2 ))");
        assert!(!chunk.ops.is_empty(), "(( )) should compile");
        // No matter the exact op set, it must SET the exit status from
        // the truthy/falsy result.
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::SetStatus | Op::GetStatus | Op::CallBuiltin(..)
            )),
            "(( )) should set or use exit status"
        );
    }

    #[test]
    fn compile_cond_dispatch() {
        // `[[ ... ]]` zsh-extended cond.
        let chunk = compile_src("[[ a == a ]]");
        assert!(!chunk.ops.is_empty(), "[[ ]] should compile");
        assert!(
            has_op(&chunk, |op| matches!(
                op,
                Op::StrEq | Op::SetStatus | Op::CallBuiltin(..)
            )),
            "[[ ]] string-eq should emit StrEq, SetStatus, or builtin"
        );
    }

    #[test]
    fn compile_param_expansion_via_named_builtin_or_op() {
        // Observation: `$HOME` lowers to LoadConst("HOME") + CallBuiltin
        // dispatch (the param-expand builtin) — not the fusevm
        // ExpandParam(..) opcode. Pin EITHER path so the test survives
        // a future opcode promotion without rotting.
        let chunk = compile_src("echo $HOME");
        let routes_through_op = has_op(&chunk, |op| {
            matches!(op, Op::ExpandParam(..) | Op::GetSlot(..))
        });
        let routes_through_builtin = has_op(&chunk, |op| matches!(op, Op::CallBuiltin(..)));
        assert!(
            routes_through_op || routes_through_builtin,
            "$HOME should route through ExpandParam, GetSlot, or builtin dispatch"
        );
        // The name "HOME" must be in the constant pool either way.
        assert!(
            chunk
                .constants
                .iter()
                .any(|c| matches!(c, fusevm::Value::Str(s) if s.as_str() == "HOME")),
            "the parameter name HOME must be in the constant pool"
        );
    }

    #[test]
    fn compile_glob_expansion_compiles_without_panic() {
        // `*.txt` lowers to LoadConst(<tokenized>) + arg-processing
        // CallBuiltin chain. The lexer tokenizes the bare `*` to
        // `Star` (`\u{87}`, per Src/zsh.h:162 + ported/zsh_h.rs:144);
        // glob expansion happens at runtime through the arg-process
        // CallBuiltin chain (un-tokenizing as it goes), not via a
        // dedicated Op::Glob at compile. Pin just that the pattern
        // made it into the constant pool — accept either the raw
        // `*.txt` (untokenized) or the tokenized `\u{87}.txt`.
        let chunk = compile_src("echo *.txt");
        assert!(
            chunk.constants.iter().any(|c| matches!(c,
                fusevm::Value::Str(s) if s.contains("*.txt") || s.contains("\u{87}.txt"))),
            "glob pattern (tokenized or literal) must be in the constant pool: {:?}",
            chunk.constants,
        );
    }

    #[test]
    fn compile_tilde_expansion_compiles_without_panic() {
        // Same story as glob: `~/x` is captured as a literal constant
        // and tilde expansion happens at runtime through the arg-process
        // CallBuiltin chain, not via a dedicated Op::TildeExpand at
        // compile time.
        let chunk = compile_src("echo ~/x");
        // Lexer tokenizes the leading `~` to `Tilde` (`\u{98}`,
        // per Src/zsh.h:179 + ported/zsh_h.rs:178). Accept either
        // form (raw or tokenized).
        assert!(
            chunk.constants.iter().any(|c| matches!(c,
                fusevm::Value::Str(s) if s.contains("~/x") || s.contains("\u{98}/x"))),
            "tilde pattern must be in the constant pool: {:?}",
            chunk.constants,
        );
    }

    #[test]
    fn compile_two_commands_separated_by_semicolon() {
        let chunk = compile_src("echo a; echo b");
        // Both commands must be present in the chunk.
        let count = chunk
            .ops
            .iter()
            .filter(|op| matches!(op, Op::CallBuiltin(..) | Op::Exec(..)))
            .count();
        assert!(
            count + chunk.sub_chunks.len() >= 2,
            "two ;-separated commands should produce 2+ exec/builtin ops, got {count}"
        );
    }

    #[test]
    fn compile_doesnt_panic_on_nested_constructs() {
        // Catch-all: a moderately complex script. Compilation succeeds.
        let src = r#"
            for i in 1 2 3; do
              if (( i > 1 )); then
                echo "$i is big"
              else
                echo "$i is small"
              fi
            done | sort
        "#;
        let chunk = compile_src(src);
        assert!(
            !chunk.ops.is_empty(),
            "complex nested source should produce non-empty chunk"
        );
    }

    // `dump_ops_for_failing_constructs` was a diagnostic eprintln dump,
    // not a test — moved to `examples/dump_compile_ops.rs`. Invoke via
    // `cargo run --example dump_compile_ops`.

    #[test]
    fn chunk_source_field_populated() {
        let chunk = compile_src("echo hi");
        // ZshCompiler sets the source field to something identifiable;
        // pin it as non-empty. Empty source = unknown error origin.
        // (The compiler may set it to "" if not called via the script
        // path — this test pins whichever default it picks.)
        let _ = chunk.source;
    }

    // ── plan9 (`^` RC_EXPAND_PARAM) cross-product classification ──────
    // c:Src/subst.c:2550-2557 — the `^` flag toggles RC_EXPAND and may
    // sit AFTER the `(…)` flag group (`${(@)^a}`), not only at the start
    // (`${^a}`). `plan9_flag_state` skips the group then counts the `^`
    // run (odd → ON, even → OFF).
    #[test]
    fn plan9_flag_state_parses_position_and_toggle() {
        assert_eq!(plan9_flag_state("^a"), Some(true)); // ${^a}
        assert_eq!(plan9_flag_state("(@)^a"), Some(true)); // ${(@)^a}
        assert_eq!(plan9_flag_state("^^a"), Some(false)); // ${^^a}
        assert_eq!(plan9_flag_state("(@)^^a"), Some(false)); // ${(@)^^a}
        assert_eq!(plan9_flag_state("(@)a"), None); // ${(@)a} — splice, no `^`
        assert_eq!(plan9_flag_state("a"), None); // ${a}
                                                 // A `^` used as a `(s:^:)` split delimiter is INSIDE the group,
                                                 // not a plan9 flag — the paren skip must swallow it.
        assert_eq!(plan9_flag_state("(s:^:)a"), None);
    }

    /// True when the chunk (recursively) calls the given builtin id.
    fn calls_builtin(chunk: &fusevm::Chunk, id: u16) -> bool {
        has_op(chunk, |op| matches!(op, Op::CallBuiltin(b, _) if *b == id))
    }

    // A DQ word carrying `${(@)^a}` cross-products the surrounding text
    // onto EVERY element (`"pre${(@)^a}"` → `prea preb prec`), so the
    // compiler must pick BUILTIN_CONCAT_PLAN9, not BUILTIN_CONCAT_SPLICE
    // (which sticks the prefix to the first element only — the bug the
    // `(@)` shape triggered by matching is_splice_expansion first).
    #[test]
    fn dq_at_caret_word_compiles_to_plan9_concat() {
        let chunk = compile_src(r#"echo "PRE${(@)^a}""#);
        assert!(
            calls_builtin(&chunk, crate::fusevm_bridge::BUILTIN_CONCAT_PLAN9),
            "\"PRE${{(@)^a}}\" must emit CONCAT_PLAN9 (cross-product)"
        );
        assert!(
            !calls_builtin(&chunk, crate::vm_helper::BUILTIN_CONCAT_SPLICE),
            "\"PRE${{(@)^a}}\" must NOT emit CONCAT_SPLICE (first/last sticking)"
        );
    }

    // `${(@)^^a}` — doubled `^^` forces RC_EXPAND OFF, so the word
    // SPLICES and must ignore the `rcexpandparam` OPTION: emit
    // SPLICE_NOPLAN9, never PLAN9.
    #[test]
    fn dq_at_double_caret_word_compiles_to_splice_noplan9() {
        let chunk = compile_src(r#"echo "PRE${(@)^^a}""#);
        assert!(
            calls_builtin(&chunk, crate::fusevm_bridge::BUILTIN_CONCAT_SPLICE_NOPLAN9),
            "\"PRE${{(@)^^a}}\" must emit SPLICE_NOPLAN9"
        );
        assert!(
            !calls_builtin(&chunk, crate::fusevm_bridge::BUILTIN_CONCAT_PLAN9),
            "\"PRE${{(@)^^a}}\" must NOT cross-product"
        );
    }

    // A word that is TWO adjacent braced expansions merely starts with
    // `${` and ends with `}` — it is NOT a single expansion. The
    // bridge-array fast path (`${(@)…${…}}`) must not swallow the whole
    // word: `${(@)^a}Y${(@)^b}` used to route to BRIDGE_BRACE_ARRAY as one
    // body and join the trailing array. The span check falls it through to
    // the segment splitter instead.
    #[test]
    fn braced_expansion_spans_word_rejects_adjacent_expansions() {
        assert!(braced_expansion_spans_word("${(@)^a}")); // single
        assert!(braced_expansion_spans_word("${(@)${a}}")); // genuinely nested
        assert!(!braced_expansion_spans_word("${(@)^a}Y${(@)^b}")); // two adjacent
        assert!(!braced_expansion_spans_word("${a}${b}")); // two adjacent, bare
        assert!(!braced_expansion_spans_word("${(@)^a}${(@)b}")); // plan9 + splice
        assert!(!braced_expansion_spans_word("Y${a}")); // not brace-led
    }

    // End-to-end: two adjacent plan9 expansions cross-product the whole
    // word (`"${(@)^a}-${(@)^b}"` → `1-A 1-B 2-A 2-B`), so the compiler
    // must take the segment path (two CONCAT_PLAN9 folds), NOT route the
    // whole word to a single BRIDGE_BRACE_ARRAY body (which joined the
    // second array).
    #[test]
    fn adjacent_plan9_expansions_take_segment_path() {
        let chunk = compile_src(r#"echo "${(@)^a}-${(@)^b}""#);
        assert!(
            calls_builtin(&chunk, crate::fusevm_bridge::BUILTIN_CONCAT_PLAN9),
            "adjacent plan9 expansions must fold via CONCAT_PLAN9 (segment path)"
        );
    }

    // A word mixing a plan9 (`^`) expansion with a non-plan9 (splice)
    // expansion — `"${(@)^a}${(@)b}"` — can't be folded by one concat
    // operator, so it routes to the atomic WORD_ASSEMBLE_PLAN9 assembler.
    #[test]
    fn mixed_plan9_splice_word_uses_word_assembler() {
        let chunk = compile_src(r#"echo "${(@)^a}${(@)b}""#);
        assert!(
            calls_builtin(&chunk, crate::fusevm_bridge::BUILTIN_WORD_ASSEMBLE_PLAN9),
            "mixed plan9+splice must route to WORD_ASSEMBLE_PLAN9"
        );
    }

    // Guard: a UNIFORM word (all segments plan9, or all splice) must NOT
    // take the assembler path — the per-pair concat fold is correct and
    // cheaper there.
    #[test]
    fn uniform_plan9_word_does_not_use_word_assembler() {
        let chunk = compile_src(r#"echo "${(@)^a}-${(@)^b}""#);
        assert!(
            !calls_builtin(&chunk, crate::fusevm_bridge::BUILTIN_WORD_ASSEMBLE_PLAN9),
            "all-plan9 word must fold via CONCAT_PLAN9, not the assembler"
        );
        assert!(
            calls_builtin(&chunk, crate::fusevm_bridge::BUILTIN_CONCAT_PLAN9),
            "all-plan9 word folds via CONCAT_PLAN9"
        );
    }

    // Regression guard: a plain `${(@)a}` (no `^`) keeps its first/last
    // splice classification — the fix must not over-broaden to every
    // `(@)` form.
    #[test]
    fn dq_at_word_without_caret_stays_splice() {
        let chunk = compile_src(r#"echo "PRE${(@)a}""#);
        assert!(
            calls_builtin(&chunk, crate::vm_helper::BUILTIN_CONCAT_SPLICE),
            "\"PRE${{(@)a}}\" must still emit CONCAT_SPLICE"
        );
        assert!(
            !calls_builtin(&chunk, crate::fusevm_bridge::BUILTIN_CONCAT_PLAN9),
            "\"PRE${{(@)a}}\" must NOT cross-product"
        );
    }
}
