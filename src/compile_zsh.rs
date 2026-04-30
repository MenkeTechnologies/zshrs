//! Bytecode compiler for the ported `ZshProgram` AST.
//!
//! Consumes the 4-tier port grammar (`ZshProgram → ZshList → ZshSublist →
//! ZshPipe → ZshCommand`) and emits fusevm bytecode. The ported parser
//! is the single source of truth for parsing; this compiler does the
//! speed work (compile-time word decomposition + native ops where
//! possible, runtime fallback for the long tail).
//!
//! Word handling: `ZshSimple::words` are raw `Vec<String>`. We decompose
//! at compile time into typed expansion ops (`Op::ExpandParam`,
//! `Op::Glob`, `Op::TildeExpand`, `Op::CmdSubst`, etc.).

use crate::parser::{
    SublistOp, ZshAssign, ZshAssignValue, ZshCommand, ZshList, ZshPipe, ZshProgram, ZshSimple,
    ZshSublist,
};
use fusevm::op::Op;
use fusevm::{ChunkBuilder, Value};
use std::collections::HashMap;

pub struct ZshCompiler {
    builder: ChunkBuilder,
    /// Variable name → slot index. Shared with arith sub-compilations.
    pub slots: HashMap<String, u16>,
    pub next_slot: u16,
    break_patches: Vec<Vec<usize>>,
    continue_patches: Vec<Vec<usize>>,
    return_patches: Vec<usize>,
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
    /// Subtract this from each pipe's `lineno` when emitting
    /// SET_LINENO calls. Top-level program: 0 (linenos passed
    /// verbatim). Function body: set to (first body line - 1) so
    /// `$LINENO` inside the function reads 1, 2, 3 relative to the
    /// body — matching zsh's `lineno = 1` reset on function entry
    /// (Src/init.c:1588).
    pub lineno_offset: u64,
}

impl ZshCompiler {
    pub fn new() -> Self {
        Self {
            builder: ChunkBuilder::new(),
            slots: HashMap::new(),
            next_slot: 0,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
            return_patches: Vec::new(),
            errexit_suppress_depth: 0,
            dq_context_depth: 0,
            assign_context_depth: 0,
            lineno_offset: 0,
        }
    }

    /// Emit a runtime errexit check. The host examines `set -e` and the
    /// last command's status; if both fire and we're at the top level
    /// (no enclosing conditional/pipeline LHS/etc.), `exit($status)`.
    fn emit_errexit_check(&mut self) {
        if self.errexit_suppress_depth > 0 {
            return;
        }
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_ERREXIT_CHECK, 0), 0);
        self.builder.emit(Op::Pop, 0);
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
        // at parse time (ZshParser::parse_program_until detects the
        // Simple<INPAR><OUTPAR> + Inbrace pattern and emits a FuncDef with
        // body_source captured). No compile-side workaround is needed.
        for list in &program.lists {
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
        let raw_line = list.sublist.pipe.lineno;
        let rel_line = raw_line.saturating_sub(self.lineno_offset).max(1);
        self.builder.emit(Op::LoadInt(rel_line as i64), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_LINENO, 1), 0);
        self.builder.emit(Op::Pop, 0);

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
            self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_RUN_BG, 1), 0);
            self.builder.emit(Op::SetStatus, 0);
        } else {
            self.compile_sublist(&list.sublist);
        }
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
        let mut next_link = sublist.next.as_ref();
        while let Some((op, next_sublist)) = next_link {
            ops.push(op.clone());
            pipes.push(&next_sublist.pipe);
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
        let has_chain_or_negate = sublist.flags.not || !ops.is_empty();
        if has_chain_or_negate {
            self.errexit_suppress_depth += 1;
        }
        self.compile_pipe(pipes[0]);
        if sublist.flags.not {
            self.emit_negate_status();
        }
        for (i, op) in ops.iter().enumerate() {
            self.builder.emit(Op::GetStatus, 0);
            let skip = match op {
                SublistOp::And => self.builder.emit(Op::JumpIfFalse(0), 0),
                SublistOp::Or => self.builder.emit(Op::JumpIfTrue(0), 0),
            };
            self.compile_pipe(pipes[i + 1]);
            self.builder.patch_jump(skip, self.builder.current_pos());
        }
        if has_chain_or_negate {
            self.errexit_suppress_depth -= 1;
        }
    }

    fn compile_coproc_pipe(&mut self, pipe: &ZshPipe) {
        // Compile the pipe's command as a body sub-chunk, then push
        // [name="", sub_idx] and call BUILTIN_RUN_COPROC.
        let mut sub = ZshCompiler::new();
        sub.compile_command(&pipe.cmd);
        let sub_end = sub.builder.current_pos();
        for patch in std::mem::take(&mut sub.return_patches) {
            sub.builder.patch_jump(patch, sub_end);
        }
        let chunk = sub.builder.build();
        let sub_idx = self.builder.add_sub_chunk(chunk);

        let name_const = self.builder.add_constant(Value::str(""));
        self.builder.emit(Op::LoadConst(name_const), 0);
        self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_RUN_COPROC, 0), 0);
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
                    Op::CallBuiltin(crate::exec::BUILTIN_PARAM_DEFAULT_FAMILY, 3),
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
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_SUBSTRING, 3), 0);
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
                    Op::CallBuiltin(crate::exec::BUILTIN_PARAM_SUBSTRING_EXPR, 4),
                    0,
                );
            }
            ParamModifierKind::Strip { op, pattern, had_at } => {
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
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_STRIP, 4), 0);
            }
            ParamModifierKind::Replace { op, pattern, repl, had_at } => {
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
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_REPLACE, 5), 0);
            }
            ParamModifierKind::Length => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_LENGTH, 1), 0);
            }
            ParamModifierKind::FilterRemoveMatching { pattern } => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                let pat_const = self.builder.add_constant(Value::str(pattern));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_FILTER, 2), 0);
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

    fn compile_pipe(&mut self, pipe: &ZshPipe) {
        // ZshPipe = command + Optional(next ZshPipe). For a single-
        // command pipe (no next), just compile the command. Multi-stage
        // pipelines are TODO — they need fork-per-stage via
        // BUILTIN_RUN_PIPELINE which compiles each stage as a sub-chunk.
        if pipe.next.is_none() {
            self.compile_command(&pipe.cmd);
            return;
        }

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
        for (stage_cmd, merge) in &stages {
            let mut sub = ZshCompiler::new();
            if *merge {
                // `|&` producer: dup stderr→stdout for this stage so the
                // pipe's read end sees both streams.
                sub.builder
                    .emit(Op::Redirect(2, fusevm::op::redirect_op::DUP_WRITE), 0);
                let one_const = sub.builder.add_constant(Value::str("1"));
                // Op::Redirect pops the target from the stack — push it
                // first. Order: target then op call.
                // (Reorder: emit Push then Op::Redirect.)
            }
            // Re-emit cleanly with target before redirect op.
            let mut sub = ZshCompiler::new();
            if *merge {
                let one_const = sub.builder.add_constant(Value::str("1"));
                sub.builder.emit(Op::LoadConst(one_const), 0);
                sub.builder
                    .emit(Op::Redirect(2, fusevm::op::redirect_op::DUP_WRITE), 0);
            }
            sub.compile_command(stage_cmd);
            let sub_end = sub.builder.current_pos();
            for patch in std::mem::take(&mut sub.return_patches) {
                sub.builder.patch_jump(patch, sub_end);
            }
            let chunk = sub.builder.build();
            let idx = self.builder.add_sub_chunk(chunk);
            self.builder.emit(Op::LoadInt(idx as i64), 0);
        }
        self.builder.emit(
            Op::CallBuiltin(crate::exec::BUILTIN_RUN_PIPELINE, stages.len() as u8),
            0,
        );
        self.builder.emit(Op::SetStatus, 0);
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
                self.builder.emit(Op::SubshellBegin, 0);
                let saved = std::mem::take(&mut self.return_patches);
                self.compile_program(prog);
                let inner_patches = std::mem::take(&mut self.return_patches);
                self.return_patches = saved;
                let landing = self.builder.current_pos();
                for patch in inner_patches {
                    self.builder.patch_jump(patch, landing);
                }
                self.builder.emit(Op::SubshellEnd, 0);
            }
            ZshCommand::Cursh(prog) => {
                // {list} — brace group; no isolation.
                self.compile_program(prog);
            }
            ZshCommand::If(if_node) => self.compile_if(if_node),
            ZshCommand::While(w) => self.compile_while(w),
            ZshCommand::Until(w) => self.compile_while(w),
            ZshCommand::For(f) => self.compile_for(f),
            ZshCommand::Case(c) => self.compile_case(c),
            ZshCommand::Repeat(r) => self.compile_repeat(r),
            ZshCommand::FuncDef(f) => self.compile_funcdef(f),
            ZshCommand::Cond(c) => self.compile_cond(c),
            ZshCommand::Arith(expr) => self.compile_arith(expr),
            ZshCommand::Redirected(inner, redirs) => {
                // Compound command with trailing redirects (e.g.
                // `{ ... } 2>&1`). Bracket the body in a
                // WithRedirectsBegin/End scope so post-body fds are
                // restored. Status is whatever the inner cmd left.
                self.builder
                    .emit(Op::WithRedirectsBegin(redirs.len() as u8), 0);
                for r in redirs {
                    self.compile_redir(r);
                }
                self.compile_command(inner);
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
                    self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
                    self.builder
                        .emit(Op::CallBuiltin(crate::exec::BUILTIN_TIME_SUBLIST, 1), 0);
                    self.builder.emit(Op::SetStatus, 0);
                } else {
                    // Bare `time` — print zero stats and exit 0.
                    self.builder.emit(Op::LoadInt(0), 0);
                    self.builder.emit(Op::SetStatus, 0);
                }
            }
            ZshCommand::Try(t) => {
                // `{ try } always { finally }` — run both blocks, with the
                // finally block executing regardless of try's exit status.
                // The exit status of the whole construct is the LAST status
                // set (matches zsh: try's status is preserved unless the
                // finally block sets a different one).
                self.compile_program(&t.try_block);
                // Capture try-block's exit status into $TRY_BLOCK_ERROR so
                // the always arm can read it (zsh's documented semantics).
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_SET_TRY_BLOCK_ERROR, 0),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                self.compile_program(&t.always);
            }
        }
    }

    fn compile_simple(&mut self, simple: &ZshSimple) {
        // ── Assignments ───────────────────────────────────────────────
        // ZshAssign{ name, value: Scalar(String)|Array(Vec<String>), append }
        for assign in &simple.assigns {
            self.compile_assign(assign);
        }

        // ── If no words: bare assignment, done ────────────────────────
        if simple.words.is_empty() {
            return;
        }

        // `noglob CMD args...` is a precommand modifier — args must
        // NOT be glob-expanded. zsh handles this in the parser by
        // marking the command line "no-glob"; we strip the leading
        // `noglob` and recursively compile the rest with a runtime
        // option-toggle wrapper.
        let untoked_first0 = crate::lexer::untokenize(&simple.words[0]);
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
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_RAW_OPT, 2), 0);
            self.builder.emit(Op::Pop, 0);
            self.compile_simple(&inner);
            self.builder.emit(Op::LoadConst(opt_const), 0);
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_RAW_OPT, 2), 0);
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
            for redir in &simple.redirs {
                self.compile_redir(redir);
            }
            // No CallBuiltin / CallFunction / Exec — just the redirects.
            // Status is 0 (zsh: `exec` with only redirs returns 0).
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder.emit(Op::SetStatus, 0);
            return;
        }

        // Bracket each command's redirects in a WithRedirectsBegin/End
        // scope so subsequent commands see the original fds. Without the
        // scope, `cmd > out.txt` would leave fd 1 pointing at out.txt for
        // every following command in the script.
        let has_redirects = !simple.redirs.is_empty();
        if has_redirects {
            self.builder
                .emit(Op::WithRedirectsBegin(simple.redirs.len() as u8), 0);
            for redir in &simple.redirs {
                self.compile_redir(redir);
            }
        }

        // ── Dispatch by first-word kind ───────────────────────────────
        // Operates on raw &str inputs and decomposes at compile time.
        let first = &simple.words[0];

        // Dynamic command name: first word contains an unquoted expansion
        // (`$cmd`, `$(cmd)`, `*name`, `~/bin/foo`). Route through Op::Exec
        // so the host runtime expands and dispatches via host.exec →
        // host_exec_external → run_intercepts. Without this, `cmd=ls;
        // $cmd` would emit CallFunction(name="$cmd", ...) and fail with
        // `command not found: $cmd`.
        let first_untoked = crate::lexer::untokenize(first);
        // `[` and `[[` are the test/cond builtins, not glob-pattern
        // command names — exempt them from the "dynamic command name"
        // check that routes through Op::Exec.
        let first_is_test_builtin = first_untoked == "[" || first_untoked == "[[";
        let first_is_dynamic = !first_is_test_builtin
            && (unquoted(&first_untoked, '$')
                || unquoted(&first_untoked, '`')
                || unquoted(&first_untoked, '*')
                || unquoted(&first_untoked, '?')
                || unquoted(&first_untoked, '[')
                || first_untoked.starts_with('~'));
        if first_is_dynamic {
            let argc = simple.words.len() as u8;
            for w in &simple.words {
                self.compile_word_str(w);
            }
            self.builder.emit(Op::Exec(argc), 0);
            self.builder.emit(Op::SetStatus, 0);
            if has_redirects {
                self.builder.emit(Op::WithRedirectsEnd, 0);
            }
            return;
        }

        // break/continue keywords — emit jumps into enclosing loop's
        // patch lists, or fall through to BUILTIN_SET_BREAK/CONTINUE
        // when no enclosing loop in this chunk. `break N` / `continue
        // N` target the N-th enclosing loop (1 = innermost, 2 = next
        // out, etc.). zsh clamps N to the available depth.
        if first == "break" {
            let levels: usize = simple
                .words
                .get(1)
                .and_then(|s| crate::lexer::untokenize(s).parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            // Index from end: levels=1 → last (innermost); levels=2 →
            // second-to-last; etc. Clamped to depth.
            let depth = self.break_patches.len();
            if depth > 0 {
                let idx = depth.saturating_sub(levels);
                let j = self.builder.emit(Op::Jump(0), 0);
                self.break_patches[idx].push(j);
            } else {
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_BREAK, 0), 0);
                self.builder.emit(Op::Pop, 0);
                let j = self.builder.emit(Op::Jump(0), 0);
                self.return_patches.push(j);
            }
            return;
        }
        if first == "continue" {
            let levels: usize = simple
                .words
                .get(1)
                .and_then(|s| crate::lexer::untokenize(s).parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);
            let depth = self.continue_patches.len();
            if depth > 0 {
                // For `continue N`, jump to the N-th enclosing loop's
                // continue target. If N>1, that's actually a BREAK out
                // of inner loops and a continue at the outer — which
                // the existing patch-list mechanism handles by jumping
                // to the outer continue target (the loop will then
                // re-enter from the top of the body).
                let idx = depth.saturating_sub(levels);
                let j = self.builder.emit(Op::Jump(0), 0);
                self.continue_patches[idx].push(j);
            } else {
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_CONTINUE, 0), 0);
                self.builder.emit(Op::Pop, 0);
                let j = self.builder.emit(Op::Jump(0), 0);
                self.return_patches.push(j);
            }
            return;
        }

        // xtrace: emit a runtime print of the literal command text
        // BEFORE pushing args / dispatching. The runtime checks the
        // `xtrace` option and prints with `$PS4` prefix to stderr.
        // Compile-time literal — variable expansion in args isn't
        // resolved here (matches zsh's `+ echo "$x"` style).
        let trace_text = simple
            .words
            .iter()
            .map(|w| crate::lexer::untokenize(w))
            .collect::<Vec<_>>()
            .join(" ");
        let trace_const = self.builder.add_constant(Value::str(trace_text));
        self.builder.emit(Op::LoadConst(trace_const), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_XTRACE_LINE, 1), 0);
        self.builder.emit(Op::Pop, 0);

        // Builtin or function or external. Push args first.
        let argc = (simple.words.len() - 1) as u8;
        for word in &simple.words[1..] {
            self.compile_word_str(word);
        }

        // `shopt` is bash-only; zsh has no such builtin. Force external lookup
        // so it produces "command not found: shopt" matching /bin/zsh exactly.
        // `declare` and `typeset` both map to BUILTIN_TYPESET in fusevm — but
        // zsh prefixes "no such variable" errors with the builtin name the
        // user actually typed. Route `declare` to BUILTIN_DECLARE so the
        // distinct error-format path fires.
        let builtin_id = if first == "shopt" {
            None
        } else if first == "declare" {
            Some(fusevm::shell_builtins::BUILTIN_DECLARE)
        } else {
            fusevm::shell_builtins::builtin_id(first)
        };
        if let Some(builtin_id) = builtin_id {
            self.builder.emit(Op::CallBuiltin(builtin_id, argc), 0);
            self.builder.emit(Op::SetStatus, 0);
            // `return`/`exit` short-circuit.
            if first == "return" || first == "exit" {
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
            let cleaned_first = crate::lexer::untokenize(first);
            let name_idx = self.builder.add_name(&cleaned_first);
            self.builder.emit(Op::CallFunction(name_idx, argc), 0);
            self.builder.emit(Op::SetStatus, 0);
            self.emit_errexit_check();
        }

        if has_redirects {
            self.builder.emit(Op::WithRedirectsEnd, 0);
        }
    }

    /// Translate a ZshRedir → fusevm Redirect/HereDoc/HereString op.
    fn compile_redir(&mut self, redir: &crate::parser::ZshRedir) {
        use crate::parser::RedirType;
        // Default fd: stdin for read-side redirects, stdout for write-side.
        let fd_default: u8 = match redir.rtype {
            RedirType::Read
            | RedirType::Heredoc
            | RedirType::HeredocDash
            | RedirType::Herestr
            | RedirType::ReadWrite
            | RedirType::MergeIn
            | RedirType::InPipe => 0,
            _ => 1,
        };
        let fd = if redir.fd >= 0 {
            redir.fd as u8
        } else {
            fd_default
        };

        // Heredoc / herestring carry their content in `redir.heredoc`.
        if matches!(redir.rtype, RedirType::Heredoc | RedirType::HeredocDash) {
            if let Some(hd) = &redir.heredoc {
                let content_clean = crate::lexer::untokenize(&hd.content);
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
                    let trimmed = content_clean.trim_end_matches('\n').to_string();
                    let text_const = self.builder.add_constant(Value::str(trimmed));
                    self.builder.emit(Op::LoadConst(text_const), 0);
                    self.builder.emit(Op::LoadInt(4), 0); // mode = HeredocBody
                    self.builder
                        .emit(Op::CallBuiltin(crate::exec::BUILTIN_EXPAND_TEXT, 2), 0);
                    self.builder.emit(Op::HereString, 0);
                }
            }
            return;
        }
        if matches!(redir.rtype, RedirType::Herestr) {
            // <<< str — push the target string as the content.
            self.compile_word_str(&redir.name);
            self.builder.emit(Op::HereString, 0);
            return;
        }

        // For non-heredoc forms, the target file/path goes via compile_word_str
        // (handles var expansion etc.). DupRead/DupWrite take a numeric fd
        // string; the runtime parses it and dup2s.
        let op_byte = match redir.rtype {
            RedirType::Write => fusevm::op::redirect_op::WRITE,
            RedirType::Writenow => fusevm::op::redirect_op::CLOBBER,
            RedirType::Append => fusevm::op::redirect_op::APPEND,
            RedirType::Appendnow => fusevm::op::redirect_op::APPEND,
            RedirType::Read => fusevm::op::redirect_op::READ,
            RedirType::ReadWrite => fusevm::op::redirect_op::READ_WRITE,
            RedirType::MergeIn => fusevm::op::redirect_op::DUP_READ,
            RedirType::MergeOut => fusevm::op::redirect_op::DUP_WRITE,
            RedirType::ErrWrite => fusevm::op::redirect_op::WRITE_BOTH,
            RedirType::ErrWritenow => fusevm::op::redirect_op::WRITE_BOTH,
            RedirType::ErrAppend => fusevm::op::redirect_op::APPEND_BOTH,
            RedirType::ErrAppendnow => fusevm::op::redirect_op::APPEND_BOTH,
            RedirType::InPipe | RedirType::OutPipe => {
                // Process substitution attached to a redirect target —
                // unusual; the parser models `< <(cmd)` differently.
                // Defer.
                tracing::debug!(?redir.rtype, "compile_zsh: pipe-style redirect TODO");
                return;
            }
            // Already handled above.
            RedirType::Heredoc | RedirType::HeredocDash | RedirType::Herestr => return,
        };

        self.compile_word_str(&redir.name);
        // `{varid}>file` named-fd allocation: instead of dup2'ing onto
        // a fixed fd, BUILTIN_OPEN_NAMED_FD opens the file fresh, dup's
        // to fd >= 10, and stores the fd number in $varid.
        if let Some(ref vid) = redir.varid {
            let vid_const = self.builder.add_constant(Value::str(vid.as_str()));
            self.builder.emit(Op::LoadConst(vid_const), 0);
            self.builder.emit(Op::LoadInt(op_byte as i64), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_OPEN_NAMED_FD, 3), 0);
            self.builder.emit(Op::SetStatus, 0);
            return;
        }
        self.builder.emit(Op::Redirect(fd, op_byte), 0);
    }

    fn compile_assign(&mut self, assign: &ZshAssign) {
        // Subscripted scalar assignment: `name[key]=value` and
        // `name[key]+=tail`. Untokenize the raw name (which carries
        // INBRACK/OUTBRACK markers) and split on the subscript brackets.
        let untoked_name = crate::lexer::untokenize(&assign.name);
        if let Some((base, key)) = split_subscript(&untoked_name) {
            if let ZshAssignValue::Scalar(s) = &assign.value {
                let name_const = self.builder.add_constant(Value::str(base));
                let key_const = self.builder.add_constant(Value::str(key));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadConst(key_const), 0);
                if assign.append {
                    // Append: dup name+key, GET_VAR via assoc, Concat with new tail
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(Op::LoadConst(key_const), 0);
                    self.builder
                        .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2), 0);
                    self.compile_word_str(s);
                    self.builder.emit(Op::Concat, 0);
                } else {
                    self.compile_word_str(s);
                }
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_ASSOC, 3), 0);
                self.builder.emit(Op::Pop, 0);
                return;
            }
        }

        match &assign.value {
            ZshAssignValue::Scalar(s) => {
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
                let needs_dq_wrap = !s.starts_with('\u{9e}')
                    && !s.starts_with('\u{9d}')
                    && (s.contains('*') || s.contains('\u{87}')   // STAR
                        || s.contains('?') || s.contains('\u{86}') // QUEST
                        || s.contains('[') || s.contains('\u{91}') // INBRACK
                        || s.contains('{') || s.contains('\u{8f}')); // INBRACE
                self.assign_context_depth += 1;
                if needs_dq_wrap {
                    let wrapped = format!("\u{9e}{}\u{9e}", s);
                    self.compile_word_str(&wrapped);
                } else {
                    self.compile_word_str(s);
                }
                self.assign_context_depth -= 1;
                let bid = if assign.append {
                    // `name+=val` — runtime-dispatch via APPEND_SCALAR_OR_PUSH:
                    // if `name` is an indexed array, push the value as a new
                    // element; if assoc, refuse (zsh errors); else scalar concat.
                    crate::exec::BUILTIN_APPEND_SCALAR_OR_PUSH
                } else {
                    crate::exec::BUILTIN_SET_VAR
                };
                self.builder.emit(Op::CallBuiltin(bid, 2), 0);
                self.builder.emit(Op::Pop, 0);
            }
            ZshAssignValue::Array(elements) => {
                // Subscripted-array assign: `a[i]=(elements)`,
                // `a[i,j]=(elements)`, or `a[i]=()` (delete element).
                // The Scalar branch above only handles single-value
                // assigns; this branch handles the array-literal form
                // including the empty-list delete idiom.
                if let Some((base, key)) = split_subscript(&untoked_name) {
                    for elem in elements {
                        self.compile_word_str(elem);
                        if has_unquoted_expansion(elem) {
                            self.builder
                                .emit(Op::CallBuiltin(crate::exec::BUILTIN_WORD_SPLIT, 0), 0);
                        }
                    }
                    let name_const = self.builder.add_constant(Value::str(base));
                    let key_const = self.builder.add_constant(Value::str(key));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(Op::LoadConst(key_const), 0);
                    let argc = (elements.len() + 2) as u8;
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_SET_SUBSCRIPT_RANGE, argc),
                        0,
                    );
                    self.builder.emit(Op::Pop, 0);
                    return;
                }
                // arr=(a b c) / arr+=(d e).
                for elem in elements {
                    self.compile_word_str(elem);
                    // Same IFS-split rule as for-list words: unquoted
                    // `$(...)` / backtick inside an array literal
                    // (`a=($(...))`) should produce per-word elements.
                    if has_unquoted_expansion(elem) {
                        self.builder
                            .emit(Op::CallBuiltin(crate::exec::BUILTIN_WORD_SPLIT, 0), 0);
                    }
                }
                let name_const = self.builder.add_constant(Value::str(assign.name.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                let argc = (elements.len() + 1) as u8;
                let bid = if assign.append {
                    crate::exec::BUILTIN_APPEND_ARRAY
                } else {
                    crate::exec::BUILTIN_SET_ARRAY
                };
                self.builder.emit(Op::CallBuiltin(bid, argc), 0);
                self.builder.emit(Op::Pop, 0);
            }
        }
    }

    /// Compile a raw word string. Detects $-triggers, glob, tilde,
    /// brace, ZshFlag, array-access at compile time and emits native
    /// ops where possible. Words that hit no fast path fall through
    /// to a runtime expand call via BUILTIN_EXPAND_TEXT.
    fn compile_word_str(&mut self, s: &str) {
        // ANSI-C quoted form: `$'a\tb'` arrives from the lexer as
        // `<META-$><SNULL>a\tb<SNULL>` = `\u{85}\u{9d}a\tb\u{9d}`. Detect
        // this shape and decode the C-style escapes into bytes.
        if s.starts_with('\u{85}') && s.len() >= 3 {
            let inner = &s[s.char_indices().nth(1).map(|(i, _)| i).unwrap_or(s.len())..];
            if inner.starts_with('\u{9d}') && inner.ends_with('\u{9d}') && inner.len() >= 6 {
                // strip leading + trailing SNULL markers (3 bytes each in UTF-8)
                let body_start = inner.char_indices().nth(1).map(|(i, _)| i).unwrap_or(0);
                let body_end = inner.len() - '\u{9d}'.len_utf8();
                let body = &inner[body_start..body_end];
                let decoded = decode_ansi_c(body);
                let idx = self.builder.add_constant(Value::str(decoded.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
                return;
            }
        }
        // Single-quoted: word contains SNULL markers wrapping a literal
        // segment. Mixed forms like `g='echo greeted'` lex to
        // `g<EQUALS><SNULL>echo greeted<SNULL>` — META tokens outside the
        // SNULLs need de-tokenizing too. Run full untokenize (which strips
        // SNULL/DNULL/BNULL markers AND maps META → original char) and
        // emit the literal result. Note: `$` inside the SNULL block is
        // already a plain `$`, never a META-$, so this is safe.
        if s.contains('\u{9d}') {
            let cleaned = crate::lexer::untokenize(s);
            let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
            self.builder.emit(Op::LoadConst(idx), 0);
            return;
        }

        // ZshLexer marks shell-special chars with zsh's META-range tokens
        // (0x83-0x9f) so the parser can distinguish syntax from literal.
        // For runtime values we want the original char back. `untokenize`
        // does this mapping. We then check for unquoted triggers on the
        // de-tokenized form.
        let untoked = crate::lexer::untokenize(s);

        if untoked.is_empty() {
            let idx = self.builder.add_constant(Value::str(""));
            self.builder.emit(Op::LoadConst(idx), 0);
            return;
        }

        // BNULL marker (`\u{9f}`) means "the next char is literal" — used
        // by the lexer for backslash-escaped specials (`\$`, `\`, etc.).
        // Fast-paths that match `$NAME` shapes on the un-tokenized form
        // would mis-route here (the `$` was escaped). Skip the fast paths
        // and fall through to the runtime expand which honors the original
        // `"\$..."` form via untokenize_preserve_quotes.
        let has_bnull = s.contains('\u{9f}');

        // Trigger detection on the un-tokenized form.
        let trigger_dollar = unquoted(&untoked, '$') || unquoted(&untoked, '`');
        let trigger_glob = unquoted(&untoked, '*')
            || unquoted(&untoked, '?')
            || unquoted(&untoked, '[')
            // extendedglob `^pat` (negation) and `pat~excl` (exclusion).
            // `^` is a no-op without `setopt extendedglob`, but routing
            // through expand_glob lets the runtime decide. The unquoted
            // check ensures `"^b"` (literal) isn't treated as a glob.
            || (untoked.starts_with('^') && untoked.len() > 1)
            // zsh glob qualifiers: `*(.)` / `path(mh-1)` etc. The `(...)`
            // suffix triggers globbing even when the body has no other
            // glob metachar — needed for `/etc/hosts(mh-100)` style.
            // Conservative: require closing `)` at end and a bare `(`
            // somewhere before (no other meta chars in between).
            || (untoked.ends_with(')')
                && untoked.contains('(')
                && !untoked.contains('|'))
            // zsh numeric range glob `<N-M>`: any `<…-…>` shape with
            // optional digits on either side outside a bracket-class.
            || has_numeric_range_glob(&untoked);
        let trigger_tilde =
            untoked.starts_with('~') || untoked.contains(":~") || untoked.contains("=~");
        // Brace expansion: `{a,b,c}` and `{1..5}` need expansion. Detect
        // matched-brace forms with comma or `..` inside.
        let trigger_brace = looks_like_brace_expansion(&untoked);

        // Process substitution `<(cmd)` / `>(cmd)`. The lexer marks the
        // outer angle bracket with INANG (`\u{94}`) / OUTANG (`\u{95}`)
        // and the parens as INPAR/OUTPAR. After untokenize, the form
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
            let is_in = untoked.starts_with("<(") || is_eq_psub;
            let inner = &untoked[2..untoked.len() - 1];
            let mut sub_parser = crate::parser::ZshParser::new(inner);
            if let Ok(prog) = sub_parser.parse() {
                let mut sub = ZshCompiler::new();
                sub.compile_program(&prog);
                let sub_end = sub.builder.current_pos();
                for patch in std::mem::take(&mut sub.return_patches) {
                    sub.builder.patch_jump(patch, sub_end);
                }
                let chunk = sub.builder.build();
                let sub_idx = self.builder.add_sub_chunk(chunk);
                if is_in {
                    self.builder.emit(Op::ProcessSubIn(sub_idx), 0);
                } else {
                    self.builder.emit(Op::ProcessSubOut(sub_idx), 0);
                }
                return;
            }
        }

        if !trigger_dollar && !trigger_glob && !trigger_tilde && !trigger_brace {
            // Pure literal — strip any \0 quote-sentinels.
            let cleaned = strip_quote_markers(&untoked);
            let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
            self.builder.emit(Op::LoadConst(idx), 0);
            return;
        }

        // Skip native fast-paths if the raw word has a BNULL escape marker
        // — the bridge path is the only one that preserves backslash-quoted
        // specials. (Normal untokenize collapses BNULL away, hiding the
        // escape from the simple $NAME / ${NAME} matchers below.)
        if has_bnull {
            // Fall through to the bridge.
        }
        // Fast path: `$@` / `$*` (quoted or unquoted) — must emit a native
        // GET_VAR so the result is Value::Array of positionals. The bridge
        // path below routes through expand_word_glob which collapses
        // DoubleQuoted into one joined string, breaking spread semantics.
        if !has_bnull && (untoked == "$@" || untoked == "$*") {
            let name = &untoked[1..];
            let idx = self.builder.add_constant(Value::str(name));
            // Detect DQ context two ways: (a) the raw input `s` is
            // DQ-wrapped (`\u{9e}$*\u{9e}`), or (b) we're inside a
            // recursive compile_word_str whose parent set
            // dq_context_depth. zsh: `"$*"` joins by IFS first char,
            // `"$@"` keeps splice semantics (each positional its own
            // word). Only `*` gets the join — `@` continues to return
            // Array. Without the `*` fix, `v="$*"` captured only the
            // first positional because pop_args flattens Array.
            let in_dq =
                self.dq_context_depth > 0 || (s.starts_with('\u{9e}') && s.ends_with('\u{9e}'));
            self.builder.emit(Op::LoadConst(idx), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1), 0);
            if in_dq && untoked == "$*" {
                // Discard the GET_VAR result; JOIN_STAR re-fetches the
                // array and joins by IFS first char. (We can't easily
                // join an in-stack Array without a dedicated op.)
                self.builder.emit(Op::Pop, 0);
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_JOIN_STAR, 1), 0);
            }
            return;
        }

        // Fast path: single bare `$NAME` (no braces, no concat, no idx,
        // no modifier). Covers `$x`, `$1`, `$#`, `$?`, `$!`, etc. — the
        // most common case in real scripts. Emits BUILTIN_GET_VAR
        // directly without going through the runtime expand path.
        // Skip when the raw word has DNULL/SNULL quote markers — those
        // signal an internal quote boundary (e.g. `"$a"bar` becomes
        // DNULL+$+a+DNULL+bar; after untokenize it looks like `$abar`
        // and the fast-path reads the wrong name). The bridge below
        // handles those correctly by routing through expand_string.
        let has_quote_markers = s.contains('\u{9d}') || s.contains('\u{9e}');
        if !has_bnull && !has_quote_markers {
            if let Some(name) = bare_var_ref(&untoked) {
                let idx = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1), 0);
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
                        && bare
                            .chars()
                            .all(|c| c == '_' || c.is_ascii_alphanumeric());
                    let is_positional = !bare.is_empty()
                        && bare.chars().all(|c| c.is_ascii_digit());
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
                        self.builder.emit(
                            Op::CallBuiltin(
                                crate::exec::BUILTIN_EXPAND_TEXT,
                                2,
                            ),
                            0,
                        );
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
            if is_ident || is_positional {
                let idx = self.builder.add_constant(Value::str(bare_name));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_LENGTH, 1), 0);
                return;
            }
        }

        // Fast path: bare `$NAME[KEY]` — without braces, zsh lexes
        // `$NAME` as the variable name and `[KEY]` as a subscript that
        // applies to it (NOT a literal `[KEY]` suffix). Emit name+key
        // through BUILTIN_ARRAY_INDEX.
        if !has_bnull {
            if let Some((name, key)) = bare_subscript_ref(&untoked) {
                let name_const = self.builder.add_constant(Value::str(name));
                let key_const = self.builder.add_constant(Value::str(key));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadConst(key_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2), 0);
                return;
            }
        }

        // Fast path: bare `$NAME[KEY]suffix` — same as above but with a
        // literal suffix appended. Emit name+key, ARRAY_INDEX, then
        // concat the suffix.
        if !has_bnull {
            if let Some((name, key, suffix)) = bare_subscript_with_suffix(&untoked) {
                let name_const = self.builder.add_constant(Value::str(name));
                let key_const = self.builder.add_constant(Value::str(key));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadConst(key_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2), 0);
                let suffix_const = self.builder.add_constant(Value::str(suffix));
                self.builder.emit(Op::LoadConst(suffix_const), 0);
                self.builder.emit(Op::Concat, 0);
                return;
            }
        }

        // Fast path: `${NAME}` — braced bare ref, equivalent to `$NAME`.
        if !has_bnull {
            if let Some(name) = braced_var_ref(&untoked) {
                let idx = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1), 0);
                return;
            }
        }

        // Fast path: `${NAME[@]}` / `${NAME[*]}` — array splice/join.
        //   `[@]` → BUILTIN_ARRAY_ALL (returns Value::Array, splice).
        //   `[*]` → BUILTIN_ARRAY_JOIN_STAR (joins with first IFS
        //          char into a single Value::Str, matching zsh).
        if !has_bnull {
            if let Some(name) = array_splice_ref(&untoked) {
                let idx = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(idx), 0);
                let bid = if array_splice_is_star(&untoked) {
                    crate::exec::BUILTIN_ARRAY_JOIN_STAR
                } else {
                    crate::exec::BUILTIN_ARRAY_ALL
                };
                self.builder.emit(Op::CallBuiltin(bid, 0), 0);
                return;
            }
        }

        // Fast path: `${NAME[KEY]}` — assoc/indexed element access. Emits
        // BUILTIN_ARRAY_INDEX which routes through assoc_arrays first then
        // falls back to indexed arrays.
        if !has_bnull {
            if let Some((base, key)) = braced_subscript_ref(&untoked) {
                let name_const = self.builder.add_constant(Value::str(base));
                let key_const = self.builder.add_constant(Value::str(key));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadConst(key_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2), 0);
                return;
            }
        }

        // Fast path: `${NAME[KEY]}` where KEY contains `$` expansions
        // (e.g. `${m[$k]}`, `${m[$prefix$suffix]}`). Resolve the key
        // text at runtime via BUILTIN_EXPAND_TEXT (mode 1 = inner-string
        // expansion, no glob/brace), then index. Mirrors the static-key
        // fast path above except the key is computed instead of loaded
        // as a constant. Without this, the assoc-array case falls back
        // to a bridge path that doesn't perform the assoc lookup.
        if !has_bnull {
            if let Some((base, key)) = braced_subscript_dynamic_ref(&untoked) {
                let name_const = self.builder.add_constant(Value::str(base));
                let key_const = self.builder.add_constant(Value::str(key));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadConst(key_const), 0);
                // mode 1 → DoubleQuoted-style: expand $-refs only, no
                // glob/brace pollution of the key.
                self.builder.emit(Op::LoadInt(1), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_EXPAND_TEXT, 2), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2), 0);
                return;
            }
        }

        // Fast path: `${(flags)"literal"}` — zsh parameter flags applied
        // to a literal string operand. Detection runs on the original `s`
        // (with quote markers intact) so we can distinguish a quoted
        // literal from a bare name. The literal value is prefixed with
        // `\u{01}` so BUILTIN_PARAM_FLAG skips the variable lookup and
        // treats the rest as a scalar value.
        if !has_bnull {
            if let Some((flags, literal)) = parse_zsh_flag_literal(s) {
                let mut tagged = String::with_capacity(literal.len() + 1);
                tagged.push('\u{01}');
                tagged.push_str(&literal);
                let name_const = self.builder.add_constant(Value::str(tagged));
                self.builder.emit(Op::LoadConst(name_const), 0);
                let flags_const = self.builder.add_constant(Value::str(flags));
                self.builder.emit(Op::LoadConst(flags_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_FLAG, 2), 0);
                return;
            }
        }

        // Fast path: `${(flags)NAME}` — zsh parameter flags. Emit
        // BUILTIN_PARAM_FLAG with [name, flags] on the stack.
        // If the whole word is wrapped in raw DNULLs (`\u{9e}`), it's
        // double-quoted — prefix the flags with `\u{02}` so the
        // runtime knows to skip array-only flags ((o)/(O)/(n)/(i)/
        // (M)/(u)) per zsh's DQ semantics.
        if !has_bnull {
            if let Some((flags, name)) = parse_zsh_flag(&untoked) {
                // DQ context: either the raw word is itself DQ-wrapped,
                // OR we're recursing into an Expansion segment from a
                // DQ-wrapped parent (tracked via dq_context_depth).
                let dq_wrapped = (s.starts_with('\u{9e}') && s.ends_with('\u{9e}') && s.len() >= 2)
                    || self.dq_context_depth > 0;
                let flags_for_runtime = if dq_wrapped {
                    let mut f = String::from("\u{02}");
                    f.push_str(flags);
                    f
                } else {
                    flags.to_string()
                };
                let name_const = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(name_const), 0);
                let flags_const = self.builder.add_constant(Value::str(flags_for_runtime));
                self.builder.emit(Op::LoadConst(flags_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_FLAG, 2), 0);
                return;
            }
        }

        // Fast path: `${(flags)NAME[KEY]}` with a real (non-`@`/`*`)
        // subscript. Resolve the subscripted value first via
        // BUILTIN_ARRAY_INDEX, then prepend the `\u{01}` literal-value
        // sentinel so BUILTIN_PARAM_FLAG treats the operand as a
        // pre-resolved scalar instead of doing a name lookup. Closes
        // the `${(f)mapfile[/path]}` and `${(s:,:)assoc[k]}` shapes.
        if !has_bnull {
            if let Some((flags, base, key)) = parse_zsh_flag_subscript(&untoked) {
                let name_const = self.builder.add_constant(Value::str(base));
                let key_const = self.builder.add_constant(Value::str(key));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadConst(key_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2), 0);
                let sentinel = self.builder.add_constant(Value::str("\u{01}"));
                self.builder.emit(Op::LoadConst(sentinel), 0);
                self.builder.emit(Op::Swap, 0);
                self.builder.emit(Op::Concat, 0);
                let flags_const = self.builder.add_constant(Value::str(flags));
                self.builder.emit(Op::LoadConst(flags_const), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_PARAM_FLAG, 2), 0);
                return;
            }
        }

        // Phase 1 native param-modifier lowerings. Each replaces a
        // bridge case. The matcher is greedy from least-ambiguous to
        // most: `:-`, `:=`, `:?`, `:+` first (modifier ops), then
        // substring (`:` + digit/dash), strip (`#`/`##`/`%`/`%%`),
        // replace (`/`/`//`/`/#`/`/%`).
        if !has_bnull {
            if let Some(modifier) = parse_param_modifier(&untoked) {
                // The whole-word DNULL wrapping (`"${...}"`) gets
                // stripped from `untoked` before parse_param_modifier
                // sees it, but downstream emitters need to know the
                // DQ context (e.g. strip op: join-then-strip in DQ
                // vs per-element unquoted). Bump dq_context_depth
                // for the duration of emit_param_modifier when the
                // raw word is DNULL-wrapped, mirroring the
                // segments-loop above. Without this, the strip
                // fast path passed dq=0 to BUILTIN_PARAM_STRIP
                // even inside `"..."`.
                let raw_dq = s.starts_with('\u{9e}')
                    && s.ends_with('\u{9e}')
                    && s.len() >= 2;
                if raw_dq {
                    self.dq_context_depth += 1;
                }
                self.emit_param_modifier(&modifier);
                if raw_dq {
                    self.dq_context_depth -= 1;
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
            let preserved_for_arith = crate::lexer::untokenize_preserve_quotes(s);
            if let Some(expr) = strip_arith_subst(&preserved_for_arith) {
                let idx = self.builder.add_constant(Value::str(expr.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARITH_EVAL, 1), 0);
                return;
            }
        }

        // `$(cmd)` command substitution. Push the command text and
        // call BUILTIN_CMD_SUBST_TEXT which routes through
        // `run_command_substitution` (compile + sub-VM + in-process
        // pipe capture). Avoids the raw Op::CmdSubst path's
        // "$(printf "a\nb")" → "anb" quoting bug.
        if !has_bnull {
            let preserved_for_cmdsub = crate::lexer::untokenize_preserve_quotes(s);
            if let Some(inner) = strip_cmd_subst(&preserved_for_cmdsub) {
                let idx = self.builder.add_constant(Value::str(inner));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_CMD_SUBST_TEXT, 1), 0);
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
                        .emit(Op::CallBuiltin(crate::exec::BUILTIN_WORD_SPLIT, 0), 0);
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
                let concat_builtin = if has_splice_seg {
                    Some(crate::exec::BUILTIN_CONCAT_SPLICE)
                } else if has_distribute_seg {
                    Some(crate::exec::BUILTIN_CONCAT_DISTRIBUTE)
                } else {
                    // Pure scalars OR `${arr}` plain — runtime check via
                    // BUILTIN_CONCAT_DISTRIBUTE (handles scalar fast path
                    // AND RC_EXPAND_PARAM cartesian when GET_VAR returns
                    // Value::Array because the option is set).
                    Some(crate::exec::BUILTIN_CONCAT_DISTRIBUTE)
                };
                // If the parent word is DQ-wrapped (raw form starts and
                // ends with DNULL), each Expansion segment inherits the
                // DQ context. Track via the compiler's
                // `dq_context_depth` counter so child compile_word_str
                // calls can see they're being expanded inside DQ
                // without us having to re-wrap (which would recurse).
                let parent_is_dq = s.starts_with('\u{9e}') && s.ends_with('\u{9e}') && s.len() >= 2;
                if parent_is_dq {
                    self.dq_context_depth += 1;
                }
                for (i, seg) in segs.iter().enumerate() {
                    match seg {
                        WordSegment::Literal(lit) => {
                            let cleaned = crate::lexer::untokenize(lit);
                            let stripped = strip_quote_markers(&cleaned);
                            let idx = self.builder.add_constant(Value::str(stripped.as_str()));
                            self.builder.emit(Op::LoadConst(idx), 0);
                        }
                        WordSegment::Expansion(exp) => {
                            self.compile_word_str(exp);
                        }
                    }
                    if i > 0 {
                        if let Some(b) = concat_builtin {
                            self.builder.emit(Op::CallBuiltin(b, 2), 0);
                        } else {
                            self.builder.emit(Op::Concat, 0);
                        }
                    }
                }
                if parent_is_dq {
                    self.dq_context_depth -= 1;
                }
                return;
            }
        }

        // Phase 2 step 2: text-based bridge replacement. Determine the
        // word's quoting mode from its raw zsh-tokenized form, push the
        // preserved text + mode_byte, call BUILTIN_EXPAND_TEXT.
        //
        // Mode detection:
        // - Whole-word DNULL-wrapped (`"…"`) and no inner unescaped
        //   DNULL → DoubleQuoted. Suppresses brace + glob expansion;
        //   var / cmd-sub / arith inside still expand.
        // - Backquote-wrapped (`` `…` ``) → AltBackquote, runs as
        //   command substitution.
        // - Else → Default, full expand_string + braces + glob.
        let preserved = crate::lexer::untokenize_preserve_quotes(s);
        // If we're recursing inside a DQ-wrapped parent (tracked via
        // `dq_context_depth`), force mode 1 so child expansions
        // suppress array-only flags like the outer DQ does.
        let mode = if self.dq_context_depth > 0 {
            1
        } else {
            expand_text_mode(s, &preserved)
        };
        let idx = self.builder.add_constant(Value::str(preserved.as_str()));
        self.builder.emit(Op::LoadConst(idx), 0);
        self.builder.emit(Op::LoadInt(mode as i64), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_EXPAND_TEXT, 2), 0);
    }

    // ── Control flow ────────────────────────────────────────────────

    fn compile_if(&mut self, if_node: &crate::parser::ZshIf) {
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
        let mut end_jumps = Vec::new();

        // First branch — the test is errexit-suppressed.
        self.errexit_suppress_depth += 1;
        self.compile_program(&if_node.cond);
        self.errexit_suppress_depth -= 1;
        self.builder.emit(Op::GetStatus, 0);
        let mut skip_body = self.builder.emit(Op::JumpIfFalse(0), 0);
        self.compile_program(&if_node.then);
        end_jumps.push(self.builder.emit(Op::Jump(0), 0));
        self.builder
            .patch_jump(skip_body, self.builder.current_pos());

        // elif branches — same suppression for each cond.
        for (cond, body) in &if_node.elif {
            self.errexit_suppress_depth += 1;
            self.compile_program(cond);
            self.errexit_suppress_depth -= 1;
            self.builder.emit(Op::GetStatus, 0);
            skip_body = self.builder.emit(Op::JumpIfFalse(0), 0);
            self.compile_program(body);
            end_jumps.push(self.builder.emit(Op::Jump(0), 0));
            self.builder
                .patch_jump(skip_body, self.builder.current_pos());
        }

        // else
        if let Some(else_) = &if_node.else_ {
            self.compile_program(else_);
        }

        let end = self.builder.current_pos();
        for ej in end_jumps {
            self.builder.patch_jump(ej, end);
        }
    }

    fn compile_while(&mut self, w: &crate::parser::ZshWhile) {
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
        let status_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(status_slot), 0);

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

        self.break_patches.push(Vec::new());
        self.continue_patches.push(Vec::new());

        self.compile_program(&w.body);
        // Capture body's last status into status_slot so the loop's exit
        // status reflects the body, not the (failing) condition probe.
        self.builder.emit(Op::GetStatus, 0);
        self.builder.emit(Op::SetSlot(status_slot), 0);

        let cont = self.builder.current_pos();
        if let Some(continues) = self.continue_patches.pop() {
            for cp in continues {
                self.builder.patch_jump(cp, cont);
            }
        }

        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);

        if let Some(breaks) = self.break_patches.pop() {
            for bp in breaks {
                self.builder.patch_jump(bp, loop_exit);
            }
        }

        // Restore loop's exit status from the body's last-status slot.
        self.builder.emit(Op::GetSlot(status_slot), 0);
        self.builder.emit(Op::SetStatus, 0);
    }

    fn compile_for(&mut self, f: &crate::parser::ZshFor) {
        use crate::parser::ForList;
        if f.is_select {
            self.compile_select(f);
            return;
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
    }

    fn compile_select(&mut self, f: &crate::parser::ZshFor) {
        use crate::parser::ForList;
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
        let words: Vec<&str> = match &f.list {
            ForList::Words(ws) => ws.iter().map(|s| s.as_str()).collect(),
            ForList::Positional => vec!["\"$@\""],
            ForList::CStyle { .. } => {
                // C-style isn't valid for select; nothing to do.
                return;
            }
        };

        for w in &words {
            self.compile_word_str(w);
            if has_unquoted_expansion(w) {
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_WORD_SPLIT, 0), 0);
            }
        }
        let name_const = self.builder.add_constant(Value::str(f.var.as_str()));
        self.builder.emit(Op::LoadConst(name_const), 0);
        self.builder.emit(Op::LoadInt(body_idx as i64), 0);

        let argc = (words.len() + 2) as u8;
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_RUN_SELECT, argc), 0);
        self.builder.emit(Op::SetStatus, 0);
    }

    fn compile_for_positional(&mut self, var: &str, body: &crate::parser::ZshProgram) {
        // Push GET_VAR("@") which returns Value::Array of positionals.
        let at_const = self.builder.add_constant(Value::str("@"));
        self.builder.emit(Op::LoadConst(at_const), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1), 0);
        // Then flatten + iterate, same shape as compile_for_words' tail.
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_FLATTEN, 1), 0);
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

        let loop_top = self.builder.current_pos();
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::GetSlot(len_slot), 0);
        self.builder.emit(Op::NumLt, 0);
        let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

        let var_const = self.builder.add_constant(Value::str(var));
        self.builder.emit(Op::LoadConst(var_const), 0);
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::SlotArrayGet(arr_slot), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_VAR, 2), 0);
        self.builder.emit(Op::Pop, 0);

        self.break_patches.push(Vec::new());
        self.continue_patches.push(Vec::new());

        self.compile_program(body);

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

        if let Some(breaks) = self.break_patches.pop() {
            for bp in breaks {
                self.builder.patch_jump(bp, loop_exit);
            }
        }
    }

    fn compile_for_words(&mut self, var: &str, words: &[String], body: &crate::parser::ZshProgram) {
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
            let untoked = crate::lexer::untokenize(word);
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
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_ALL, 0), 0);
                continue;
            }
            self.compile_word_str(word);
            // Unquoted command/variable substitution in a for-list should
            // IFS-split. zsh's for-list naturally word-splits the result
            // of `$(...)` or unquoted `$var`. Quoted forms keep one word.
            if has_unquoted_expansion(word) {
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_WORD_SPLIT, 0), 0);
            }
        }
        self.builder.emit(
            Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_FLATTEN, words.len() as u8),
            0,
        );
        // ARRAY_FLATTEN pushes Array then Int(len) (its return). Top is len.
        self.builder.emit(Op::SetSlot(len_slot), 0);
        self.builder.emit(Op::SetSlot(arr_slot), 0);

        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(i_slot), 0);

        let loop_top = self.builder.current_pos();
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::GetSlot(len_slot), 0);
        self.builder.emit(Op::NumLt, 0);
        let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

        // var = array[i] via SET_VAR (visible to nested VMs)
        let var_const = self.builder.add_constant(Value::str(var));
        self.builder.emit(Op::LoadConst(var_const), 0);
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::SlotArrayGet(arr_slot), 0);
        self.builder
            .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_VAR, 2), 0);
        self.builder.emit(Op::Pop, 0);

        self.break_patches.push(Vec::new());
        self.continue_patches.push(Vec::new());

        self.compile_program(body);

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

        if let Some(breaks) = self.break_patches.pop() {
            for bp in breaks {
                self.builder.patch_jump(bp, loop_exit);
            }
        }
    }

    fn compile_for_arith(
        &mut self,
        init: &str,
        cond: &str,
        step: &str,
        body: &crate::parser::ZshProgram,
    ) {
        // For multi-statement comma init/step (`i=0,j=10`,
        // `i++,j--`), ArithCompiler only handles ONE op per call,
        // dropping the rest. Route through MathEval (via
        // BUILTIN_ARITH_EVAL) which evaluates the comma list in
        // order and writes back through extract_string_variables.
        // Same routing for any `$`-bearing expr — ArithCompiler's
        // lexer treats `$` as unknown so `for ((i=1; i<=$#a; i++))`
        // never iterated. The two arith engines use different
        // storage (ArithCompiler→slots, MathEval→variables); when ANY
        // section needs MathEval, route ALL sections so the value of
        // `i` survives across init/cond/step in the same backing store.
        let untoked_init = crate::lexer::untokenize(init);
        let untoked_cond = crate::lexer::untokenize(cond);
        let untoked_step = crate::lexer::untokenize(step);
        let needs_eval_global = untoked_init.contains(',')
            || untoked_init.contains('$')
            || untoked_cond.contains(',')
            || untoked_cond.contains('$')
            || untoked_step.contains(',')
            || untoked_step.contains('$');
        let route_through_eval =
            move |_s: &str| -> bool { needs_eval_global };
        let emit_arith = |this: &mut Self, s: &str| {
            let untoked = crate::lexer::untokenize(s);
            if route_through_eval(&untoked) {
                let idx = this.builder.add_constant(Value::str(untoked.as_str()));
                this.builder.emit(Op::LoadConst(idx), 0);
                this.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARITH_EVAL, 1), 0);
            } else {
                this.compile_arith_str(s);
            }
        };

        if !init.is_empty() {
            emit_arith(self, init);
            self.builder.emit(Op::Pop, 0);
        }

        let loop_top = self.builder.current_pos();
        if !cond.is_empty() {
            // Cond is evaluated for truthiness — keep simple
            // ArithCompiler path unless comma OR a `$`-bearing
            // expansion is present (ArithCompiler can't lex `$`).
            if needs_eval_global {
                let untoked = crate::lexer::untokenize(cond);
                let idx = self.builder.add_constant(Value::str(untoked.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARITH_EVAL, 1), 0);
                // ARITH_EVAL returns Value::Str ("0" / "1" / etc.).
                // Convert to bool: non-zero → true.
                let zero_const = self.builder.add_constant(Value::str("0"));
                self.builder.emit(Op::LoadConst(zero_const), 0);
                self.builder.emit(Op::StrEq, 0);
                self.builder.emit(Op::LogNot, 0);
            } else {
                self.compile_arith_str(cond);
            }
        } else {
            self.builder.emit(Op::LoadTrue, 0);
        }
        let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

        self.break_patches.push(Vec::new());
        self.continue_patches.push(Vec::new());

        self.compile_program(body);

        let cont = self.builder.current_pos();
        if let Some(continues) = self.continue_patches.pop() {
            for cp in continues {
                self.builder.patch_jump(cp, cont);
            }
        }

        if !step.is_empty() {
            emit_arith(self, step);
            self.builder.emit(Op::Pop, 0);
        }
        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);

        if let Some(breaks) = self.break_patches.pop() {
            for bp in breaks {
                self.builder.patch_jump(bp, loop_exit);
            }
        }
    }

    fn compile_case(&mut self, c: &crate::parser::ZshCase) {
        use crate::parser::CaseTerm;
        // Word goes onto a slot for repeated comparison.
        self.compile_word_str(&c.word);
        let word_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::SetSlot(word_slot), 0);

        let mut end_jumps = Vec::new();
        // Pending fall-through from the previous arm's `;&` terminator.
        // When Some, the patch needs to land at the CURRENT arm's body
        // start (skipping its own pattern check).
        let mut pending_fall: Option<usize> = None;

        for arm in &c.arms {
            let mut match_jumps = Vec::new();
            for pattern in &arm.patterns {
                self.builder.emit(Op::GetSlot(word_slot), 0);
                // Patterns are RAW glob strings. ZshLexer encodes glob
                // chars (`*`, `?`, `[`, `]`) in the META range so the
                // grammar can distinguish syntax from literal. For the
                // matcher we want the original glob char back —
                // un-tokenize before pushing.
                let pat_clean = crate::lexer::untokenize(pattern);
                let pat_const = self.builder.add_constant(Value::str(pat_clean.as_str()));
                self.builder.emit(Op::LoadConst(pat_const), 0);
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

        let end = self.builder.current_pos();
        for ej in end_jumps {
            self.builder.patch_jump(ej, end);
        }
        // A pending `;&` from the last arm has nowhere to fall through —
        // patch to `end` so it just exits cleanly.
        if let Some(prev) = pending_fall {
            self.builder.patch_jump(prev, end);
        }
    }

    fn compile_repeat(&mut self, r: &crate::parser::ZshRepeat) {
        let i_slot = self.next_slot;
        self.next_slot += 1;
        let count_slot = self.next_slot;
        self.next_slot += 1;

        self.compile_arith_str(&r.count);
        self.builder.emit(Op::SetSlot(count_slot), 0);

        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(i_slot), 0);

        let loop_top = self.builder.current_pos();
        self.builder.emit(Op::GetSlot(i_slot), 0);
        self.builder.emit(Op::GetSlot(count_slot), 0);
        self.builder.emit(Op::NumLt, 0);
        let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

        self.break_patches.push(Vec::new());
        self.continue_patches.push(Vec::new());

        self.compile_program(&r.body);

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
        if let Some(breaks) = self.break_patches.pop() {
            for bp in breaks {
                self.builder.patch_jump(bp, loop_exit);
            }
        }
    }

    fn compile_funcdef(&mut self, f: &crate::parser::ZshFuncDef) {
        // Compile the body to a fusevm sub-chunk and register via
        // BUILTIN_REGISTER_COMPILED_FN with three args:
        //   [name, base64(bincode(chunk)), body_source]
        // The handler stores the chunk in functions_compiled and the source
        // text in function_source so introspection (whence, which, typeset
        // -f, ${functions[name]}) returns canonical body text.
        //
        // Set lineno_offset = (first_body_line - 1) so $LINENO
        // inside the function reads 1, 2, 3 relative to the body
        // (matches zsh's `lineno = 1` reset on function entry at
        // Src/init.c:1588). Use the first list's pipe lineno as
        // the offset anchor.
        let mut body_compiler = ZshCompiler::new();
        let first_body_line = f
            .body
            .lists
            .first()
            .map(|l| l.sublist.pipe.lineno)
            .unwrap_or(1);
        body_compiler.lineno_offset = first_body_line.saturating_sub(1);
        let body_chunk = body_compiler.compile(&f.body);
        let body_bytes = bincode::serialize(&body_chunk).unwrap_or_default();
        let body_str = base64_encode(&body_bytes);
        let source_text = f.body_source.clone().unwrap_or_default();

        for raw_name in &f.names {
            // The lexer packs `name()` into a single String token by
            // appending INPAR+OUTPAR markers (`\u{88}\u{8a}`). Under
            // `function name() { body }` this lands in `names` with
            // the suffix attached. Strip it here so the registered
            // function name is the bare identifier — the parser stays
            // a pure port of zsh's grammar.
            let cleaned = if raw_name.ends_with('\u{8a}') && raw_name.contains('\u{88}') {
                let stripped = raw_name
                    .trim_end_matches('\u{8a}')
                    .trim_end_matches('\u{88}');
                crate::lexer::untokenize(stripped)
            } else {
                raw_name.clone()
            };
            let name_const = self.builder.add_constant(Value::str(cleaned.as_str()));
            self.builder.emit(Op::LoadConst(name_const), 0);
            let body_const = self.builder.add_constant(Value::str(body_str.as_str()));
            self.builder.emit(Op::LoadConst(body_const), 0);
            let source_const = self.builder.add_constant(Value::str(source_text.as_str()));
            self.builder.emit(Op::LoadConst(source_const), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_REGISTER_COMPILED_FN, 3),
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
                    crate::lexer::untokenize(stripped)
                } else {
                    raw_name.clone()
                };
                let name_idx = self.builder.add_name(&cleaned);
                self.builder.emit(Op::CallFunction(name_idx, argc), 0);
                self.builder.emit(Op::SetStatus, 0);
            }
        }
    }

    fn compile_cond(&mut self, c: &crate::parser::ZshCond) {
        use crate::parser::ZshCond;
        // Result on stack: bool. Status set after this returns.
        self.compile_cond_expr(c);
        // Convert bool → status (true=0, false=1)
        let true_jump = self.builder.emit(Op::JumpIfTrue(0), 0);
        self.builder.emit(Op::LoadInt(1), 0);
        self.builder.emit(Op::SetStatus, 0);
        let end_jump = self.builder.emit(Op::Jump(0), 0);
        let true_target = self.builder.current_pos();
        self.builder.patch_jump(true_jump, true_target);
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetStatus, 0);
        let end = self.builder.current_pos();
        self.builder.patch_jump(end_jump, end);
        let _ = ZshCond::Not;
    }

    fn compile_cond_expr(&mut self, c: &crate::parser::ZshCond) {
        use crate::parser::ZshCond;
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
                // ZshLexer encodes operator chars in the META range
                // (0x83-0x9f). Un-tokenize before matching.
                let op_clean = crate::lexer::untokenize(op);
                // `-v` takes a parameter NAME (with optional subscript)
                // — never glob-expand the operand. Without this,
                // `[[ -v a[1] ]]` errored "no matches found: a[1]"
                // because `a[1]` was treated as a `[1]` char-class
                // glob. Emit the literal text so the runtime's
                // BUILTIN_VAR_EXISTS handler sees `a[1]` intact and
                // can split on `[` to look up `arr[1]` element.
                if op_clean == "-v" {
                    let arg_clean = crate::lexer::untokenize(arg);
                    let idx = self.builder.add_constant(Value::str(arg_clean.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                } else {
                    self.compile_word_str(arg);
                }
                self.emit_file_test(&op_clean);
            }
            ZshCond::Binary(left, op, right) => {
                let left_clean = crate::lexer::untokenize(left);
                let op_clean = crate::lexer::untokenize(op);
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
                        let op_clean_arg = crate::lexer::untokenize(op);
                        let idx = self.builder.add_constant(Value::str(op_clean_arg.as_str()));
                        self.builder.emit(Op::LoadConst(idx), 0);
                    } else {
                        self.compile_word_str(op);
                    }
                    self.emit_file_test(&left_clean);
                    return;
                }
                self.compile_word_str(left);
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
                    // expansion + filesystem globbing during expansion.
                    let dq_wrapped = if right.starts_with('\u{9e}') {
                        right.clone()
                    } else {
                        format!("\u{9e}{}\u{9e}", right)
                    };
                    self.compile_word_str(&dq_wrapped);
                } else if is_pattern_op {
                    // RHS handling for `==` / `=` / `!=` patterns:
                    // - If it contains a variable / cmd-subst (`$`, `` ` ``)
                    //   route through compile_word_str so the value is
                    //   substituted in. Wrap in DQ so the expansion
                    //   doesn't trigger filesystem-glob — the result
                    //   is a PATTERN matched by the test, not a path.
                    // - Otherwise use the literal-pattern path with
                    //   pre-escaped quoted-glob metas.
                    let needs_expand = right.contains('\u{85}')   // META-$
                        || right.contains('\u{8c}')                  // QSTRING-$
                        || right.contains('\u{93}')                  // TICK
                        || right.contains('$')
                        || right.contains('`');
                    if needs_expand {
                        let dq_wrapped = if right.starts_with('\u{9e}') {
                            right.clone()
                        } else {
                            format!("\u{9e}{}\u{9e}", right)
                        };
                        self.compile_word_str(&dq_wrapped);
                    } else {
                        let escaped = escape_quoted_glob_metas(right);
                        let right_clean = crate::lexer::untokenize(&escaped);
                        let idx = self.builder.add_constant(Value::str(right_clean.as_str()));
                        self.builder.emit(Op::LoadConst(idx), 0);
                    }
                } else {
                    self.compile_word_str(right);
                }
                self.emit_binary_test(&op_clean);
            }
            ZshCond::Regex(left, regex) => {
                self.compile_word_str(left);
                let regex_clean = crate::lexer::untokenize(regex);
                let pat_const = self.builder.add_constant(Value::str(regex_clean.as_str()));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                self.builder.emit(Op::RegexMatch, 0);
            }
        }
    }

    fn emit_file_test(&mut self, op: &str) {
        use fusevm::op::file_test;
        let test_byte: u8 = match op {
            "-e" | "-a" => file_test::EXISTS,
            "-f" => file_test::IS_FILE,
            "-d" => file_test::IS_DIR,
            "-r" => file_test::IS_READABLE,
            "-w" => file_test::IS_WRITABLE,
            "-x" => file_test::IS_EXECUTABLE,
            "-s" => file_test::IS_NONEMPTY,
            "-L" | "-h" => file_test::IS_SYMLINK,
            "-c" => {
                // Character device. Not in fusevm's file_test set.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_IS_CHARDEV, 1), 0);
                return;
            }
            "-b" => {
                // Block device.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_IS_BLOCKDEV, 1), 0);
                return;
            }
            "-p" => {
                // FIFO (named pipe).
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_IS_FIFO, 1), 0);
                return;
            }
            "-S" => {
                // Socket.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_IS_SOCKET, 1), 0);
                return;
            }
            "-k" => {
                // Sticky bit (S_ISVTX). Not in fusevm's file_test set;
                // route through a thin host-side builtin.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_HAS_STICKY, 1), 0);
                return;
            }
            "-u" => {
                // Setuid bit (S_ISUID).
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_HAS_SETUID, 1), 0);
                return;
            }
            "-g" => {
                // Setgid bit (S_ISGID).
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_HAS_SETGID, 1), 0);
                return;
            }
            "-O" => {
                // Owned by effective UID.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_OWNED_BY_USER, 1), 0);
                return;
            }
            "-G" => {
                // Owned by effective GID.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_OWNED_BY_GROUP, 1), 0);
                return;
            }
            "-N" => {
                // File modified since last accessed (mtime > atime).
                // zsh: used to gate mailbox-style "fresh content" checks.
                self.builder.emit(
                    Op::CallBuiltin(
                        crate::exec::BUILTIN_FILE_MODIFIED_SINCE_ACCESS,
                        1,
                    ),
                    0,
                );
                return;
            }
            "-z" => {
                self.builder.emit(Op::StringLen, 0);
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::NumEq, 0);
                return;
            }
            "-n" => {
                self.builder.emit(Op::StringLen, 0);
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::NumNe, 0);
                return;
            }
            "-v" => {
                // `[[ -v name ]]` — variable existence check (bash; zsh
                // approximates via `(t)` flag). Stack-top is the name —
                // route through BUILTIN_VAR_EXISTS which checks scalar /
                // array / assoc / env tables.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_VAR_EXISTS, 1), 0);
                return;
            }
            "-o" => {
                // `[[ -o option ]]` — shell-option-set check. Routes
                // through BUILTIN_OPTION_SET which normalizes the name
                // (strip _, lowercase) and reads exec.options.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_OPTION_SET, 1), 0);
                return;
            }
            "-t" => {
                // `[[ -t fd ]]` — fd-is-a-tty check. Stack-top is the
                // fd-string (e.g. "0", "1", "2"). Route through a
                // host-side builtin that calls libc::isatty.
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_IS_TTY, 1), 0);
                return;
            }
            _ => {
                // zsh: `[[ -l file ]]` (and any other unknown unary
                // condition) errors with `unknown condition: -X`.
                // Emit the diagnostic at compile-time (stderr) and
                // produce false. Runtime BUILTIN dispatch failed (the
                // CallBuiltin op didn't reliably fire for this path),
                // so do the print here in the compile path — it runs
                // for every shell that tries the unknown condition.
                eprintln!("zshrs:1: unknown condition: {}", op);
                self.builder.emit(Op::Pop, 0);
                self.builder.emit(Op::LoadFalse, 0);
                return;
            }
        };
        self.builder.emit(Op::TestFile(test_byte), 0);
    }

    fn emit_binary_test(&mut self, op: &str) {
        match op {
            "=" | "==" => self.builder.emit(Op::StrMatch, 0),
            "!=" => {
                self.builder.emit(Op::StrMatch, 0);
                self.builder.emit(Op::LogNot, 0)
            }
            // `=~` arrives via ZshCond::Binary not ZshCond::Regex (the
            // port uses Binary for everything by default). Route to
            // RegexMatch here.
            "=~" => self.builder.emit(Op::RegexMatch, 0),
            "<" => self.builder.emit(Op::StrLt, 0),
            ">" => self.builder.emit(Op::StrGt, 0),
            "-eq" => self.builder.emit(Op::NumEq, 0),
            "-ne" => self.builder.emit(Op::NumNe, 0),
            "-lt" => self.builder.emit(Op::NumLt, 0),
            "-le" => self.builder.emit(Op::NumLe, 0),
            "-gt" => self.builder.emit(Op::NumGt, 0),
            "-ge" => self.builder.emit(Op::NumGe, 0),
            "-ef" => self
                .builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_SAME_FILE, 2), 0),
            "-nt" => self
                .builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_FILE_NEWER, 2), 0),
            "-ot" => self
                .builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_FILE_OLDER, 2), 0),
            _ => {
                tracing::debug!(op, "compile_zsh: unknown binary test op");
                self.builder.emit(Op::Pop, 0);
                self.builder.emit(Op::Pop, 0);
                self.builder.emit(Op::LoadFalse, 0);
                0usize
            }
        };
    }

    fn compile_arith(&mut self, expr: &str) {
        // Compound `(( expr ))` — set status based on whether expr is non-zero.
        // Subscripted-array assignment (`((a[i]=v))`) needs to bypass
        // ArithCompiler (which doesn't write back through arr[idx])
        // and use the runtime arith eval that we taught about
        // subscripted-array writes.
        let untoked = crate::lexer::untokenize(expr);
        // Strip leading/trailing `(` and `)` from the lexer's wrapper —
        // `(( a[i]=v ))` arrives here with parens still attached.
        let inner_arith_owned = untoked
            .trim_start_matches('(')
            .trim_end_matches(')')
            .trim()
            .to_string();
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
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARITH_EVAL, 1), 0);
            self.builder.emit(Op::Pop, 0);
            // Status is 0 (truthy assignment) per zsh — `((a[i]=42))` is
            // success unless rhs is 0.
            self.builder.emit(Op::LoadInt(0), 0);
            self.builder.emit(Op::SetStatus, 0);
            return;
        }
        // ArithCompiler emits float-only Op::Div, doesn't recognize
        // `|=` / `&=` / `^=` / `<<=` / `>>=` as compound assigns, and
        // doesn't write back the result. Route through MathEval (via
        // BUILTIN_ARITH_EVAL) when any of those appear OR the expr
        // contains `/`. MathEval has full operator support and writes
        // variable values back through extract_string_variables.
        let needs_eval = inner_arith.contains('/')
            || inner_arith.contains("|=")
            || inner_arith.contains("&=")
            || inner_arith.contains("^=")
            || inner_arith.contains("<<=")
            || inner_arith.contains(">>=")
            // Float literals and exponents — ArithCompiler's lexer
            // can't parse them. Route through MathEval which has
            // full float support including int→float promotion on
            // mixed-mode compound assigns (`((a *= 1.5))`).
            || inner_arith.contains('.')
            || inner_arith.contains('e')
            || inner_arith.contains('E')
            // Comma operator — ArithCompiler's compound-assign emit
            // path only handles a single `op=` and drops subsequent
            // expressions in `a+=5, b*=2`. MathEval evaluates the
            // entire comma-list in order.
            || inner_arith.contains(',')
            // Parameter expansion (`${…}`, `${+name}`, `${#x}`) —
            // ArithCompiler's lexer treats `$` as an unknown char and
            // either fails or computes the wrong thing. MathEval
            // routes through `evaluate_arithmetic` → `expand_string`
            // first, so the expansion produces a numeric string before
            // arith evaluation.
            || inner_arith.contains('$');
        if needs_eval {
            let idx_const = self.builder.add_constant(Value::str(inner_arith));
            self.builder.emit(Op::LoadConst(idx_const), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARITH_EVAL, 1), 0);
            // Result stays on stack as Value::Str (e.g. "3" / "0" / "1.5").
            // Compare against "0" to compute the truthiness. Don't
            // re-evaluate the expression — it's an assignment so the
            // second call would compound (e.g. `a/=3` runs twice).
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
            let end = self.builder.current_pos();
            self.builder.patch_jump(end_jump, end);
            return;
        }
        self.compile_arith_str(expr);
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::NumNe, 0);
        let true_jump = self.builder.emit(Op::JumpIfTrue(0), 0);
        self.builder.emit(Op::LoadInt(1), 0);
        self.builder.emit(Op::SetStatus, 0);
        let end_jump = self.builder.emit(Op::Jump(0), 0);
        let true_target = self.builder.current_pos();
        self.builder.patch_jump(true_jump, true_target);
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetStatus, 0);
        let end = self.builder.current_pos();
        self.builder.patch_jump(end_jump, end);
    }

    /// Compile arithmetic expression text. Leaves the result on stack
    /// as Value::Int. Pre-loads variable slots, emits arith ops via
    /// ArithCompiler against this compiler's builder + slot table,
    /// then post-syncs slots back to vars.
    fn compile_arith_str(&mut self, expr: &str) {
        // ZshLexer tokenizes operator chars (`<`, `>`, `=`, `&`, `|`,
        // `*`, `?`, etc.) into the META range. ArithCompiler can't parse
        // those — un-tokenize first to recover the original ASCII form.
        let expr_clean = crate::lexer::untokenize(expr);

        let mut ac = crate::arith_compiler::ArithCompiler::new(&expr_clean);
        ac.slots = self.slots.clone();
        ac.next_slot = self.next_slot;

        // Pre-load: any var the arith expression touches needs its current
        // value pulled from executor.variables into its slot. Without this
        // `i=5; (( i+1 ))` reads 0 from the uninitialized slot.
        let pre_load_names = ac.collect_identifiers(&expr_clean);
        for name in &pre_load_names {
            let slot = ac.slot_for(name);
            let name_const = ac.builder.add_constant(Value::str(name.as_str()));
            ac.builder.emit(Op::LoadConst(name_const), 0);
            ac.builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1), 0);
            ac.builder.emit(Op::SetSlot(slot), 0);
        }

        ac.expr();
        let new_slots = ac.slots.clone();
        let new_next = ac.next_slot;
        let chunk = ac.builder.build();

        // Inline ArithCompiler's emitted ops into ours, remapping const
        // indices into our local constant table.
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
                other => other.clone(),
            };
            self.builder.emit(remapped, 0);
        }

        self.slots = new_slots.clone();
        self.next_slot = new_next;

        // Post-sync: write each pre-loaded slot back to executor.variables
        // via BUILTIN_SET_VAR. This makes `(( i++ ))` visible to subsequent
        // `echo $i` and to the loop's own conditional check.
        // The arith result is on top of stack — capture into a temp slot,
        // sync, then restore.
        let result_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::SetSlot(result_slot), 0);

        for name in &pre_load_names {
            if let Some(&slot) = new_slots.get(name) {
                let name_const = self.builder.add_constant(Value::str(name.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::GetSlot(slot), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_SET_VAR, 2), 0);
                self.builder.emit(Op::Pop, 0); // discard Status(0)
            }
        }

        self.builder.emit(Op::GetSlot(result_slot), 0);
    }
}

/// True iff `s` contains `target` at a position not preceded by the `\0`
/// quote sentinel.
/// Cheap check: does `s` contain a top-level `{...}` group that's a brace
/// expansion (comma list or `..` range)? Used to trigger the runtime
/// expand-word path so `{a,b,c}` and `{1..5}` get expanded into multiple
/// arguments instead of being passed as a literal `{a,b,c}`.
fn looks_like_brace_expansion(s: &str) -> bool {
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
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    false
}

/// Determine the quote-mode for the bridge replacement based on the
/// raw zsh-tokenized word. Returns one of:
///   0 = Default (full expand_string + braces + glob)
///   1 = DoubleQuoted (expand vars, suppress brace + glob)
///   3 = AltBackquote (run as command substitution)
/// Mode 2 (SingleQuoted) is rare here because the SNULL early-return at
/// the top of compile_word_str already catches `'…'` shapes.
fn expand_text_mode(raw: &str, preserved: &str) -> u8 {
    // DoubleQuoted: starts AND ends with raw DNULL, no inner unescaped
    // DNULL pair (i.e. exactly one matching pair wrapping the whole
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
                .rev()
                .nth(0)
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
/// zsh-tokenized chars; may contain META markers like STAR/QUEST that
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
/// Walks the chars looking for META-$ (`\u{85}`), QSTRING-`$` inside
/// double-quotes (`\u{8c}`), or backtick (`` ` ``) markers. Each marker
/// plus its body becomes one Expansion segment; everything else is
/// Literal. NOTE: `\u{84}` is POUND (`#`), not a `$`-marker; including
/// it here would treat `${#arr[@]}` as a concat with `#arr` as the
/// expansion body.
/// True for expansions that splice with FIRST/LAST sticking semantics:
/// `${arr[@]}`, `${arr[*]}`, `$@`, `$*`. Surrounding text in the same
/// word sticks only to the first or last array element.
fn is_splice_expansion(s: &str) -> bool {
    let pq = crate::lexer::untokenize_preserve_quotes(s);
    if pq == "$@" || pq == "$*" || pq == "${@}" || pq == "${*}" {
        return true;
    }
    if let Some(inner) = pq.strip_prefix("${").and_then(|t| t.strip_suffix('}')) {
        if inner.contains("[@]") || inner.contains("[*]") {
            return true;
        }
        // `(@)NAME` flag form is the splice equivalent of `[@]` —
        // each element becomes its own arg; surrounding literals
        // should stick to first/last (so `[${(@)a}]` for empty `a`
        // still emits `[]` rather than dropping the brackets).
        if let Some(rest) = inner.strip_prefix('(') {
            if let Some(close) = rest.find(')') {
                let flags = &rest[..close];
                if flags.chars().any(|c| c == '@') {
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
fn is_distribute_expansion(s: &str) -> bool {
    let pq = crate::lexer::untokenize_preserve_quotes(s);
    if let Some(inner) = pq.strip_prefix("${").and_then(|t| t.strip_suffix('}')) {
        if inner.starts_with('^') {
            return true;
        }
        if let Some(rest) = inner.strip_prefix('(') {
            if let Some(close) = rest.find(')') {
                let flags = &rest[..close];
                for c in flags.chars() {
                    match c {
                        'f' | 'z' | 'w' | 'A' | 'a' | 'P' | '@' | 's' => return true,
                        _ => {}
                    }
                }
            }
        }
    }
    false
}

fn split_word_segments(s: &str) -> Option<Vec<WordSegment>> {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut segs: Vec<WordSegment> = Vec::new();
    let mut lit_start = 0;
    let mut i = 0;
    // Track nesting inside `{...}` (INBRACE/OUTBRACE) and `[...]`
    // (INBRACK/OUTBRACK) so an inner expansion marker like the `$i`
    // in `${a[$i]}` doesn't get pulled out as its own segment.
    // Top-level (depth 0) markers are real concat boundaries.
    let mut brace_depth = 0i32;
    let mut brack_depth = 0i32;
    while i < n {
        let c = chars[i];
        match c {
            '\u{8f}' => brace_depth += 1,                       // INBRACE
            '\u{90}' => brace_depth = (brace_depth - 1).max(0), // OUTBRACE
            '\u{91}' => brack_depth += 1,                       // INBRACK
            '\u{92}' => brack_depth = (brack_depth - 1).max(0), // OUTBRACK
            _ => {}
        }
        // Recognize segment boundaries:
        // - META-$ (\u{85}) and META-QSTRING (\u{8c}) — emitted by the
        //   lexer for `$` outside / inside double quotes
        // - Literal `$` (0x24) — emitted in some lexer paths where the
        //   `$` survives untokenized but the surrounding braces / brackets
        //   are META-marked. Followed by INBRACE/INPAR/alphanumeric to
        //   distinguish from a literal trailing `$`.
        let is_meta_dollar = c == '\u{85}' || c == '\u{8c}';
        let is_literal_dollar_with_expansion = c == '$' && {
            // peek next char — must be `{`-meta, `(`-meta, or ident-start
            chars
                .get(i + 1)
                .map(|&n| {
                    n == '\u{8f}'  // INBRACE
                        || n == '\u{88}'  // INPAR
                        || n == '_'
                        || n.is_ascii_alphanumeric()
                        || n == '@' || n == '*' || n == '#' || n == '?'
                        || n == '!' || n == '$'
                })
                .unwrap_or(false)
        };
        let is_dollar = is_meta_dollar || is_literal_dollar_with_expansion;
        let is_backtick = c == '`';
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

/// Given chars[i] is META-$ / QSTRING / backtick, return the index just
/// past the end of the expansion. Handles `${...}`, `$(...)`,
/// `$((...))`, `$NAME`, `$N`, `$@` etc., and `` `cmd` ``.
fn find_expansion_end(chars: &[char], i: usize) -> usize {
    let c = chars[i];
    if c == '`' {
        // Backtick: find matching `
        let mut j = i + 1;
        while j < chars.len() && chars[j] != '`' {
            j += 1;
        }
        return (j + 1).min(chars.len());
    }
    // META-$ or QSTRING — look at next char
    let next = chars.get(i + 1).copied();
    match next {
        // INBRACE: ${...}
        Some('\u{8f}') => {
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '\u{8f}' => depth += 1,
                    '\u{90}' => depth -= 1,
                    _ => {}
                }
                j += 1;
            }
            j
        }
        // INPAR: $(...) or $((...))
        // The lexer emits these shapes:
        //   `$(cmd)`    → META-$ INPAR <body chars> OUTPAR
        //   `$((expr))` → META-$ INPAR <body w/ literal `(`/`)`> OUTPARMATH
        // For `$((`, the inner `(` is kept literal and the closing `))`
        // is collapsed into a single OUTPARMATH (\u{8b}). We detect by
        // peeking after INPAR — if the next char is literal `(` (0x28)
        // or INPARMATH, we're in arith mode and end at OUTPARMATH.
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
        // Also catch META-$ + INPARMATH directly for arith forms.
        Some('\u{89}') => {
            let mut j = i + 2;
            while j < chars.len() && chars[j] != '\u{8b}' {
                j += 1;
            }
            (j + 1).min(chars.len())
        }
        // INBRACK: $[...]
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
        // Special single-char params: $@ $* $# $? $! $- $_ $$.
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
                '@' | '*' | '#' | '?' | '!' | '-' | '_' | '$'
                    | '\u{87}' // META-* (STAR)
                    | '\u{84}' // META-# (POUND)
                    | '\u{97}' // META-? (QUEST)
                    | '\u{9b}' // META-- (DASH)
                    | '\u{9c}' // META-! (BANG)
                    | '\u{85}' // META-$ ($$ → PID; second $ also lexed as STRING)
                    | '\u{8c}' // META-QSTRING ($ in DQ context)
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
                if matches!(after, '@' | '*' | '\u{87}') {
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
            i + 2
        }
        // All-digit positional: $0..$N
        Some(ch) if ch.is_ascii_digit() => {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
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
            // `$NAME` + literal `[idx]`. The lexer emits INBRACK
            // (`\u{91}`) / OUTBRACK (`\u{92}`) for top-level `[]`, but
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

/// If `s` is exactly `$(cmd)` (un-tokenized form), return the inner
/// command. Excludes `$((…))` arithmetic and partial concatenations.
fn strip_cmd_subst(s: &str) -> Option<&str> {
    if !s.starts_with("$(") || !s.ends_with(')') || s.starts_with("$((") {
        return None;
    }
    // Verify the closing `)` at the end matches the OPENING `$(` at the
    // start (i.e. the whole input is exactly one cmd-subst). Without this
    // check, `$(echo foo)$(echo bar)` matched too — the outer `$(` and
    // final `)` are not paired, the body is `echo foo)$(echo bar` which
    // ran as a malformed script and dropped the second cmd subst.
    let inner = &s[2..s.len() - 1];
    let mut depth = 1i32;
    let chars: Vec<char> = inner.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && i < chars.len() - 1 {
                    // Found a closing `)` mid-string → not a single cmd
                    // subst (the rest is a separate token / second subst).
                    return None;
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

/// Parse `${...}` and detect a Phase 1 param-modifier shape. Returns
/// `None` for shapes that need the bridge — nested `${...}`, multiple
/// modifiers chained, etc. The name must be a plain identifier; mixed
/// concatenation and modifiers fall through.
fn parse_param_modifier(s: &str) -> Option<ParamModifier> {
    let inner = s.strip_prefix("${")?.strip_suffix('}')?;
    if inner.is_empty() {
        return None;
    }
    // Reject flag forms — handled by earlier fast-paths.
    if inner.starts_with('(') {
        return None;
    }
    // Nested `${…}` is allowed ONLY in substring offset/length operands
    // (`${var:N:${#x}-2}` style). Other shapes (length-of-nested,
    // strip with nested pattern, etc.) still fall through to the
    // bridge. Detect by scanning for the `:N:` substring pattern and
    // ensuring the nested `${` only appears after the second `:`.
    if inner.contains("${") {
        if let Some(first_colon) = inner.find(':') {
            // Substring shape: must start with NAME:digit/$/-/...
            // The substring path in this function handles the rest;
            // we just need the offset operand to be a plain int (or
            // `$NAME` simple) and the length to be the nested form.
            // Check the offset segment: from after `:` up to the
            // second `:` should NOT contain `${`. Allow nested only
            // in the length operand.
            let after_first = &inner[first_colon + 1..];
            // Skip the offset's leading minus / spaces.
            let off_section_end = after_first.find(':').unwrap_or(after_first.len());
            let off_section = &after_first[..off_section_end];
            if off_section.contains("${") {
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
    // `${#name}` — length form. Special-case: var name follows the `#`.
    // Both scalars (StringLen) and arrays (ARRAY_LENGTH) are supported,
    // dispatched at runtime by ParamModifierKind::Length.
    if first == b'#' && bytes.len() > 1 {
        let rest = &inner[1..];
        // Identifier OR identifier with `[@]`/`[*]` suffix (zsh:
        // `${#arr[@]}` == `${#arr}` for arrays/assocs).
        let body = rest
            .strip_suffix("[@]")
            .or_else(|| rest.strip_suffix("[*]"))
            .unwrap_or(rest);
        // `${#name:-default}` etc. → fall through to the bridge so the
        // default is applied first and the length is taken on the
        // post-default result. The bridge correctly distinguishes
        // unset/empty (uses default → length of default text) from
        // set arrays (default unused → array element count).
        if body.contains(":-") || body.contains(":+") || body.contains(":=") || body.contains(":?")
        {
            return None;
        }
        if !body
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        {
            return None;
        }
        if !body.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
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
        // Special single-char name? Not handled here.
        return None;
    }
    let name = inner[..name_end].to_string();
    // Optional `[@]` / `[*]` subscript suffix — for arrays and assocs
    // these are no-ops on the lookup but shouldn't break the modifier
    // parse. Strip and continue parsing the modifier. Track whether
    // we saw it so downstream Strip emit can force per-element
    // semantics inside DQ (`"${a[@]%%pat}"` = per-element, not
    // joined-then-stripped).
    let mut after_name = name_end;
    let mut had_at = false;
    if inner.len() >= name_end + 3 {
        let tail = &inner[name_end..name_end + 3];
        if tail == "[@]" {
            // `[@]` = splice-expand (per-element even in DQ)
            after_name = name_end + 3;
            had_at = true;
        } else if tail == "[*]" {
            // `[*]` = join-with-IFS-then-scalar (matches the bare-
            // name DQ join-then-strip behavior — leave had_at false
            // so the runtime treats it like the unsubscripted
            // `"${a%%pat}"` case).
            after_name = name_end + 3;
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
    if rest.len() >= 2 {
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
    if !rest.is_empty() {
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
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::FilterRemoveMatching {
                pattern: pat.to_string(),
            },
        });
    }

    // `${var:offset[:length]}` substring. The post-`:` text must lead
    // with a digit, `-`, single space (negative-offset disambiguator),
    // OR `$`/`(` (variable / arith expression — runtime-evaluated).
    if rest.starts_with(':') {
        let after = &rest[1..];
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
                            name,
                            kind: ParamModifierKind::SubstringExpr {
                                offset_expr: offset.to_string(),
                                length_expr: len_str,
                            },
                        })
                    }
                };
                return Some(ParamModifier {
                    name,
                    kind: ParamModifierKind::Substring { offset, length },
                });
            }
            // Variable / arith case — defer to runtime.
            return Some(ParamModifier {
                name,
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
            (0u8, &rest[1..])
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

/// Parse `${(flags)NAME}` and return (flags, name). The name must be a
/// plain identifier; nested expansions or subscripted names disqualify
/// this fast-path and route through the runtime expand instead.
///
/// Detect `${(flags)"literal"}` or `${(flags)'literal'}` shape. Caller
/// passes the untokenize_preserve_quotes form so brace/paren markers are
/// already mapped back to ASCII and DNULL/SNULL are mapped to `"`/`'`.
/// Returns (flags, literal_value) on match.
fn parse_zsh_flag_literal(raw: &str) -> Option<(String, String)> {
    let pq = crate::lexer::untokenize_preserve_quotes(raw);
    let inner = pq.strip_prefix("${")?.strip_suffix('}')?;
    let inner_chars: Vec<char> = inner.chars().collect();
    if inner_chars.first()? != &'(' {
        return None;
    }
    let mut depth = 0;
    let mut close_idx = None;
    for (i, &c) in inner_chars.iter().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close_idx = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close_idx?;
    let flags: String = inner_chars[1..close].iter().collect();
    let operand: Vec<char> = inner_chars[close + 1..].to_vec();
    if operand.len() < 2 {
        return None;
    }
    let (open, closec) = (operand[0], *operand.last().unwrap());
    if !((open == '"' && closec == '"') || (open == '\'' && closec == '\'')) {
        return None;
    }
    let literal: String = operand[1..operand.len() - 1].iter().collect();
    Some((flags, literal))
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
    let mut name = &inner[close + 1..];
    // Strip `[@]` / `[*]` suffix — `${(kv)m[@]}` should apply the flag
    // to the whole array, same shape as `${(kv)m}` (which already
    // returns Value::Array for the (kv) interleave). Without this strip
    // the matcher rejected the name and the bridge path returned wrong
    // results (just values, ignoring the (k) flag).
    if let Some(stripped) = name
        .strip_suffix("[@]")
        .or_else(|| name.strip_suffix("[*]"))
    {
        name = stripped;
    }
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
    if key.contains('[') || key.contains(']') || key.contains('$') || key.contains('`') {
        return None;
    }
    Some((flags, base, key))
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
    if base.is_empty() || key.is_empty() {
        return None;
    }
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
    // has a positional-param branch.
    let is_special = base == "@" || base == "*";
    if !is_special {
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
    for sub in &["[@]}", "[*]}"] {
        if let Some(rest) = s.strip_suffix(sub) {
            if let Some(name) = rest.strip_prefix("${") {
                if !name.is_empty()
                    && (name.chars().next().unwrap() == '_'
                        || name.chars().next().unwrap().is_ascii_alphabetic())
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
    s.ends_with("[*]}")
}

/// For `[[ ... == PATTERN ]]` style tests, walk PATTERN and replace
/// glob metas (`*`, `?`, `[`) that fall INSIDE single/double-quoted
/// regions with backslash-escaped versions. Quoted glob metas should
/// match literally per zsh. Markers used by the lexer:
///   `\u{9d}` (SNULL) — single-quote boundary
///   `\u{9e}` (DNULL) — double-quote boundary
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
            '*' | '?' | '[' | '(' | ')' | '|' | '~' | '#' | '^' if in_squote || in_dquote => {
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
    if first == '_' || first.is_ascii_alphabetic() {
        if inner.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            return Some(inner);
        }
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
    if first == '_' || first.is_ascii_alphabetic() {
        if rest.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            return Some(rest);
        }
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
    if !s.ends_with(']') {
        return None;
    }
    let name = &rest[..lb];
    let key = &rest[lb + 1..rest.len() - 1];
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
    if key.contains('[') || key.contains(']') || key.contains('$') || key.contains('`') {
        return None;
    }
    Some((name, key))
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
    let rb = rest.find(']')?;
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
/// Lexer markers: `\u{85}` = META-$, `\u{88}` = INPAR.
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
            // `$(...)` — META-$ followed by INPAR
            if c == '\u{85}' && i + 1 < chars.len() && chars[i + 1] == '\u{88}' {
                return true;
            }
            // Plain `$` followed by INPAR (lexer sometimes leaves `$` literal)
            if c == '$' && i + 1 < chars.len() && chars[i + 1] == '\u{88}' {
                return true;
            }
            // Backtick command sub
            if c == '`' || c == '\u{96}' || c == '\u{95}' {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn unquoted(s: &str, target: char) -> bool {
    let mut prev = ' ';
    for c in s.chars() {
        if c == target && prev != '\x00' {
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

/// Strip the lexer's `\0X` quote sentinels (single-quoted special chars).
fn strip_quote_markers(s: &str) -> String {
    if !s.contains('\x00') {
        return s.to_string();
    }
    s.chars().filter(|c| *c != '\x00').collect()
}

/// Tiny base64 encoder for embedding bincode-serialized chunks inside
/// constant strings (the BUILTIN_REGISTER_COMPILED_FN handler decodes).
/// Avoids dragging in a base64 crate dependency just for this one call
/// site.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((bytes.len() + 2) / 3) * 4);
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
            Some('0') => out.push('\0'),
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
                    out.push(b as char);
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
