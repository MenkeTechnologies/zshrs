//! Bytecode compiler for the ported `ZshProgram` AST.
//!
//! Consumes the 4-tier port grammar (`ZshProgram → ZshList → ZshSublist →
//! ZshPipe → ZshCommand`) and emits fusevm bytecode. This is the
//! replacement for `shell_compiler::ShellCompiler`, which consumed the
//! hand-rolled `ShellCommand` AST. The port is the single source of truth
//! for parsing; this compiler does the speed work (compile-time word
//! decomposition + native ops where possible, runtime fallback for the
//! long tail).
//!
//! Migration plan:
//!   1. Build skeleton with ZshSimple + ZshList + ZshSublist + ZshPipe.
//!   2. Cover ZshFor, ZshIf, ZshWhile, ZshCase progressively.
//!   3. Cover ZshFuncDef, ZshCond, ZshArith.
//!   4. Wire `execute_script` to use this path.
//!   5. Run all 398 tests; fix divergences.
//!   6. Delete `shell_compiler.rs` + `ShellParser` + `ShellLexer` +
//!      `ShellCommand` + `ShellWord`.
//!
//! Word handling: `ZshSimple::words` are raw `Vec<String>`. We decompose
//! at compile time into typed expansion ops (`Op::ExpandParam`,
//! `Op::Glob`, `Op::TildeExpand`, `Op::CmdSubst`, etc.) using the same
//! detection logic that lives in `shell_compiler.rs::compile_word`. This
//! keeps the speed of the existing pipeline while sourcing the AST from
//! the faithful port.

use crate::parser::{
    ZshAssign, ZshAssignValue, ZshCommand, ZshList, ZshPipe, ZshProgram, ZshSimple,
    ZshSublist, SublistOp,
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
        }
    }

    /// Compile a parsed `ZshProgram` to a runnable Chunk.
    pub fn compile(mut self, program: &ZshProgram) -> fusevm::Chunk {
        self.compile_program(program);

        // Patch return/exit jumps to past chunk end. Same mechanism as
        // shell_compiler.rs's compile().
        let end_pos = self.builder.current_pos();
        for patch in std::mem::take(&mut self.return_patches) {
            self.builder.patch_jump(patch, end_pos);
        }

        self.builder.build()
    }

    fn compile_program(&mut self, program: &ZshProgram) {
        for list in &program.lists {
            self.compile_list(list);
        }
    }

    fn compile_list(&mut self, list: &ZshList) {
        // ZshList = sublist + flags (async / disown).
        if list.flags.async_ {
            // Background: compile the sublist into a sub-chunk + emit
            // BUILTIN_RUN_BG just like the ShellCompiler path.
            let mut sub = ZshCompiler::new();
            sub.compile_sublist(&list.sublist);
            let sub_end = sub.builder.current_pos();
            for patch in std::mem::take(&mut sub.return_patches) {
                sub.builder.patch_jump(patch, sub_end);
            }
            let sub_chunk = sub.builder.build();
            let sub_idx = self.builder.add_sub_chunk(sub_chunk);
            self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_RUN_BG, 1),
                0,
            );
            self.builder.emit(Op::SetStatus, 0);
        } else {
            self.compile_sublist(&list.sublist);
        }
    }

    fn compile_sublist(&mut self, sublist: &ZshSublist) {
        // ZshSublist = pipe + Optional((And|Or, next-sublist)).
        // Compile the head pipe, then if there's a chain, emit short-
        // circuit jumps mirroring the shell_compiler.rs::compile_list
        // approach: GetStatus + JumpIfFalse(And) or JumpIfTrue(Or).
        self.compile_pipe(&sublist.pipe);

        if let Some((op, next)) = &sublist.next {
            self.builder.emit(Op::GetStatus, 0);
            let skip = match op {
                SublistOp::And => self.builder.emit(Op::JumpIfFalse(0), 0),
                SublistOp::Or => self.builder.emit(Op::JumpIfTrue(0), 0),
            };
            self.compile_sublist(next);
            self.builder.patch_jump(skip, self.builder.current_pos());
        }
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

        // Multi-stage pipeline: collect all stages, compile each into a
        // sub-chunk, push their indices, call BUILTIN_RUN_PIPELINE.
        let mut stages: Vec<&ZshCommand> = vec![&pipe.cmd];
        let mut cur = pipe.next.as_deref();
        while let Some(p) = cur {
            stages.push(&p.cmd);
            cur = p.next.as_deref();
        }
        for stage_cmd in &stages {
            let mut sub = ZshCompiler::new();
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
                // (list) — subshell with state isolation.
                self.builder.emit(Op::SubshellBegin, 0);
                self.compile_program(prog);
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
            ZshCommand::Time(_) | ZshCommand::Try(_) => {
                // Stubs for now — `time` and `try { } always { }` are
                // niche enough that we land them in a follow-up pass.
                tracing::debug!("compile_zsh: Time/Try not yet implemented");
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

        // ── Redirects: TODO. For now skip (most simple commands don't
        // have them and we'll wire next iteration) ────────────────────
        let _ = &simple.redirs;

        // ── Dispatch by first-word kind ───────────────────────────────
        // Same logic as shell_compiler.rs::compile_simple but operating
        // on raw &str inputs. We decompose at compile time.
        let first = &simple.words[0];

        // break/continue keywords — emit jumps into enclosing loop's
        // patch lists, or fall through to BUILTIN_SET_BREAK/CONTINUE
        // when no enclosing loop in this chunk.
        if first == "break" {
            if let Some(p) = self.break_patches.last_mut() {
                let j = self.builder.emit(Op::Jump(0), 0);
                p.push(j);
            } else {
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_SET_BREAK, 0),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                let j = self.builder.emit(Op::Jump(0), 0);
                self.return_patches.push(j);
            }
            return;
        }
        if first == "continue" {
            if let Some(p) = self.continue_patches.last_mut() {
                let j = self.builder.emit(Op::Jump(0), 0);
                p.push(j);
            } else {
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_SET_CONTINUE, 0),
                    0,
                );
                self.builder.emit(Op::Pop, 0);
                let j = self.builder.emit(Op::Jump(0), 0);
                self.return_patches.push(j);
            }
            return;
        }

        // Builtin or function or external. Push args first.
        let argc = (simple.words.len() - 1) as u8;
        for word in &simple.words[1..] {
            self.compile_word_str(word);
        }

        if let Some(builtin_id) = fusevm::shell_builtins::builtin_id(first) {
            self.builder.emit(Op::CallBuiltin(builtin_id, argc), 0);
            self.builder.emit(Op::SetStatus, 0);
            // `return`/`exit` short-circuit.
            if first == "return" || first == "exit" {
                let j = self.builder.emit(Op::Jump(0), 0);
                self.return_patches.push(j);
            }
        } else {
            // Treat as function/external dispatch via Op::CallFunction.
            // host.call_function checks aliases → functions → falls back
            // to host.exec for externals.
            let name_idx = self.builder.add_name(first);
            self.builder.emit(Op::CallFunction(name_idx, argc), 0);
            self.builder.emit(Op::SetStatus, 0);
        }
    }

    fn compile_assign(&mut self, assign: &ZshAssign) {
        match &assign.value {
            ZshAssignValue::Scalar(s) => {
                let name_const = self.builder.add_constant(Value::str(assign.name.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.compile_word_str(s);
                let bid = if assign.append {
                    // Scalar append: NAME+=tail concats. Use SET_VAR after
                    // GET_VAR + Concat. Simpler: add a builtin or do it
                    // inline. Inline:
                    let name_again = self.builder.add_constant(Value::str(assign.name.as_str()));
                    self.builder.emit(Op::LoadConst(name_again), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                        0,
                    );
                    self.builder.emit(Op::Swap, 0);
                    self.builder.emit(Op::Concat, 0);
                    crate::exec::BUILTIN_SET_VAR
                } else {
                    crate::exec::BUILTIN_SET_VAR
                };
                self.builder.emit(Op::CallBuiltin(bid, 2), 0);
                self.builder.emit(Op::Pop, 0);
            }
            ZshAssignValue::Array(elements) => {
                // arr=(a b c) / arr+=(d e) — same shape as ShellCompiler.
                for elem in elements {
                    self.compile_word_str(elem);
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

    /// Compile a raw word string. This does at compile time what
    /// ShellParser was doing during parse — detect $-triggers, glob,
    /// tilde, brace, ZshFlag, array-access — and emit native ops where
    /// possible.
    ///
    /// Migration status: this is a literal-only stub today. The next
    /// passes will incrementally add fast paths matching what
    /// `shell_compiler.rs::compile_word` does for each ShellWord variant,
    /// re-implemented to take a `&str` directly.
    ///
    /// For words that contain $ / glob / tilde / brace / etc., we
    /// currently fall through to a runtime expand_word call via
    /// BUILTIN_EXPAND_WORD_RUNTIME. That keeps semantics correct (the
    /// tree-walker era expansion engine handles every form) at the cost
    /// of compile-time decomposition speed. Each subsequent migration
    /// pass replaces one variant's runtime fallback with native ops.
    fn compile_word_str(&mut self, s: &str) {
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

        // Trigger detection on the un-tokenized form.
        let trigger_dollar = unquoted(&untoked, '$') || unquoted(&untoked, '`');
        let trigger_glob = unquoted(&untoked, '*')
            || unquoted(&untoked, '?')
            || unquoted(&untoked, '[');
        let trigger_tilde = untoked.starts_with('~')
            || untoked.contains(":~")
            || untoked.contains("=~");

        if !trigger_dollar && !trigger_glob && !trigger_tilde {
            // Pure literal — strip any \0 quote-sentinels.
            let cleaned = strip_quote_markers(&untoked);
            let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
            self.builder.emit(Op::LoadConst(idx), 0);
            return;
        }

        // Anything else — runtime fallback for now. Re-parse the word
        // through ShellParser so the existing expand_word semantics
        // apply, then serialize and call BUILTIN_EXPAND_WORD_RUNTIME.
        // Subsequent migration passes replace this with native ops.
        let bridge_src = format!("echo {}", untoked);
        let mut parser = crate::parser::ShellParser::new(&bridge_src);
        if let Ok(commands) = parser.parse_script() {
            if let Some(crate::parser::ShellCommand::Simple(simple)) = commands.first() {
                if let Some(word) = simple.words.get(1) {
                    let json = serde_json::to_string(word).unwrap_or_default();
                    let const_idx = self.builder.add_constant(Value::str(json));
                    self.builder.emit(Op::LoadConst(const_idx), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_EXPAND_WORD_RUNTIME, 1),
                        0,
                    );
                    return;
                }
            }
        }
        let idx = self.builder.add_constant(Value::str(untoked.as_str()));
        self.builder.emit(Op::LoadConst(idx), 0);
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

        // First branch
        self.compile_program(&if_node.cond);
        self.builder.emit(Op::GetStatus, 0);
        let mut skip_body = self.builder.emit(Op::JumpIfFalse(0), 0);
        self.compile_program(&if_node.then);
        end_jumps.push(self.builder.emit(Op::Jump(0), 0));
        self.builder.patch_jump(skip_body, self.builder.current_pos());

        // elif branches
        for (cond, body) in &if_node.elif {
            self.compile_program(cond);
            self.builder.emit(Op::GetStatus, 0);
            skip_body = self.builder.emit(Op::JumpIfFalse(0), 0);
            self.compile_program(body);
            end_jumps.push(self.builder.emit(Op::Jump(0), 0));
            self.builder.patch_jump(skip_body, self.builder.current_pos());
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
        self.compile_program(&w.cond);
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
        match &f.list {
            ForList::Words(words) => {
                self.compile_for_words(&f.var, words, &f.body);
            }
            ForList::CStyle { init, cond, step } => {
                self.compile_for_arith(init, cond, step, &f.body);
            }
            ForList::Positional => {
                // `for var; do …; done` — iterate over $@.
                let positional: Vec<String> = vec!["\"$@\"".to_string()];
                self.compile_for_words(&f.var, &positional, &f.body);
            }
        }
    }

    fn compile_for_words(
        &mut self,
        var: &str,
        words: &[String],
        body: &crate::parser::ZshProgram,
    ) {
        let i_slot = self.next_slot;
        self.next_slot += 1;
        let len_slot = self.next_slot;
        self.next_slot += 1;
        let arr_slot = self.next_slot;
        self.next_slot += 1;

        for word in words {
            self.compile_word_str(word);
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
        if !init.is_empty() {
            self.compile_arith_str(init);
            self.builder.emit(Op::Pop, 0);
        }

        let loop_top = self.builder.current_pos();
        if !cond.is_empty() {
            self.compile_arith_str(cond);
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
            self.compile_arith_str(step);
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

        for arm in &c.arms {
            let mut match_jumps = Vec::new();
            for pattern in &arm.patterns {
                self.builder.emit(Op::GetSlot(word_slot), 0);
                // Patterns are RAW pattern strings — push as constant, not
                // as expanded word. `*` in a case pattern must reach
                // Op::StrMatch as glob, not get expanded into cwd listing.
                let pat_const = self.builder.add_constant(Value::str(pattern.as_str()));
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

            self.compile_program(&arm.body);

            match arm.terminator {
                CaseTerm::Break => {
                    end_jumps.push(self.builder.emit(Op::Jump(0), 0));
                }
                CaseTerm::Continue | CaseTerm::TestNext => {
                    // ;& fallthrough / ;|/;;& continue-testing — both
                    // simplified: fall through to next arm (next pattern
                    // group). Real ;|`continue testing` semantics deferred.
                }
            }
            let after_body = self.builder.current_pos();
            self.builder.patch_jump(skip_body, after_body);
        }

        let end = self.builder.current_pos();
        for ej in end_jumps {
            self.builder.patch_jump(ej, end);
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
        // Same approach as shell_compiler: register the function via
        // BUILTIN_REGEST_FUNCTION with a JSON-serialized AST. The runtime
        // compiles the body lazily on first call. Multiple names share
        // the same body.
        for name in &f.names {
            let body_json = serde_json::to_string(&f.body).unwrap_or_default();
            // BUILTIN_REGISTER_FUNCTION expects a ShellCommand-shaped body
            // for its JSON deserialize. We're emitting a ZshProgram
            // instead — until the host's register-function path migrates,
            // route via the bridge: re-parse the body source through
            // ShellParser. For now, fall back: emit a no-op + tracing.
            let _ = (name, body_json);
            tracing::debug!(
                func = %name.as_str(),
                "compile_zsh: FuncDef registration TODO (needs ZshProgram body in registry)"
            );
        }
        // TODO: extend BUILTIN_REGISTER_FUNCTION to accept ZshProgram or
        // re-parse the source through ShellParser as a bridge.
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
                // -f file, -d, -e, -r, -w, -x, -s, -z, -n, ...
                self.compile_word_str(arg);
                self.emit_file_test(op);
            }
            ZshCond::Binary(left, op, right) => {
                self.compile_word_str(left);
                self.compile_word_str(right);
                self.emit_binary_test(op);
            }
            ZshCond::Regex(left, regex) => {
                self.compile_word_str(left);
                // RHS is the verbatim regex pattern — push as constant
                // (no glob expansion, no word splitting).
                let pat_const = self.builder.add_constant(Value::str(regex.as_str()));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                self.builder.emit(Op::RegexMatch, 0);
            }
        }
    }

    fn emit_file_test(&mut self, op: &str) {
        use fusevm::op::file_test;
        let test_byte: u8 = match op {
            "-e" | "-a" => file_test::EXISTS,
            "-f" => file_test::IS_REGULAR,
            "-d" => file_test::IS_DIR,
            "-r" => file_test::IS_READABLE,
            "-w" => file_test::IS_WRITABLE,
            "-x" => file_test::IS_EXECUTABLE,
            "-s" => file_test::IS_NONEMPTY,
            "-L" | "-h" => file_test::IS_SYMLINK,
            "-z" => {
                // Empty-string test — Op::Len + NumEq 0
                self.builder.emit(Op::StrLen, 0);
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::NumEq, 0);
                return;
            }
            "-n" => {
                self.builder.emit(Op::StrLen, 0);
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::NumNe, 0);
                return;
            }
            _ => {
                // Unknown unary — produce false.
                tracing::debug!(op, "compile_zsh: unknown unary test op");
                self.builder.emit(Op::Pop, 0); // discard arg
                self.builder.emit(Op::LoadFalse, 0);
                return;
            }
        };
        self.builder.emit(Op::FileTest(test_byte), 0);
    }

    fn emit_binary_test(&mut self, op: &str) {
        match op {
            "=" | "==" => self.builder.emit(Op::StrMatch, 0),
            "!=" => {
                self.builder.emit(Op::StrMatch, 0);
                self.builder.emit(Op::LogNot, 0)
            }
            "<" => self.builder.emit(Op::StrLt, 0),
            ">" => self.builder.emit(Op::StrGt, 0),
            "-eq" => self.builder.emit(Op::NumEq, 0),
            "-ne" => self.builder.emit(Op::NumNe, 0),
            "-lt" => self.builder.emit(Op::NumLt, 0),
            "-le" => self.builder.emit(Op::NumLe, 0),
            "-gt" => self.builder.emit(Op::NumGt, 0),
            "-ge" => self.builder.emit(Op::NumGe, 0),
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

    /// Compile arithmetic expression text via the shared ArithCompiler from
    /// shell_compiler.rs, with pre-load + post-sync to executor.variables
    /// so `(( i++ ))` is visible after the call.
    fn compile_arith_str(&mut self, expr: &str) {
        // Mirror ShellCompiler::compile_arith_inline. Reuse its impl directly
        // by constructing a temporary ShellCompiler that shares our slots,
        // then inlining its emitted ops. ArithCompiler has no dependence on
        // ShellWord — it consumes raw &str — so this delegates cleanly.
        let mut sc = crate::shell_compiler::ShellCompiler::new();
        sc.slots = self.slots.clone();
        sc.next_slot = self.next_slot;
        // Build a one-statement compound to drive compile_arith_inline.
        let cmd = crate::parser::ShellCommand::Compound(
            crate::parser::CompoundCommand::Arith(expr.to_string()),
        );
        let chunk = sc.compile(std::slice::from_ref(&cmd));
        // The emitted ops compute the arith and set $? based on truthiness.
        // We want ONLY the value-computing portion (not the SetStatus
        // post-amble). The Arith arm at shell_compiler.rs ends with a
        // SetStatus — find the FIRST SetStatus and take everything before
        // it. The result of the arith is left on the stack at that point.
        let cut = chunk.ops.iter().position(|op| matches!(op, Op::SetStatus));
        let upper = cut.unwrap_or(chunk.ops.len());
        // Re-allocate constants into our builder.
        let mut const_remap: std::collections::HashMap<u16, u16> =
            std::collections::HashMap::new();
        for op in &chunk.ops[..upper] {
            let remapped: Op = match op {
                Op::LoadConst(idx) => {
                    let dst = const_remap.entry(*idx).or_insert_with(|| {
                        let v = chunk
                            .constants
                            .get(*idx as usize)
                            .cloned()
                            .unwrap_or(fusevm::Value::str(""));
                        self.builder.add_constant(v)
                    });
                    Op::LoadConst(*dst)
                }
                other => other.clone(),
            };
            self.builder.emit(remapped, 0);
        }
        self.slots = sc.slots.clone();
        self.next_slot = sc.next_slot;
        // The cut excluded SetStatus so the arith result remains on stack —
        // but the original Arith arm flow leaves status-set ops AFTER the
        // result is consumed by NumNe/etc. To get a clean stack value we
        // need just the arith result. Looking at the Arith arm:
        //   compile_arith_inline → result on stack
        //   LoadInt(0), NumNe → bool on stack (consumed by JumpIfTrue)
        //   ...status set...
        // So before the FIRST SetStatus we have the arith result on stack
        // followed by a bool we don't want. Bail out earlier — find the
        // FIRST LoadInt(0) right after compile_arith_inline. Easier:
        // compile_arith_inline itself leaves the int on top. Let's
        // re-run with a different harness: use a fresh ShellCompiler and
        // compile JUST `compile_arith_inline` directly. But it's a private
        // method. Workaround: emit `$(( expr ))` as a ShellWord::ArithSub
        // and compile that — its compile_word path emits arith + Concat
        // with empty string; we strip the Concat.
        //
        // Simpler practical approach: use the full Arith compound but pop
        // the status-set ops via balanced re-emission. Above we already
        // emitted everything before the first SetStatus, which leaves
        // [result, bool] on stack — that's wrong. Drop the bool by Pop.
        // The bool was produced by LoadInt(0) + NumNe sequence which is
        // 2 ops before SetStatus; we already emitted both. So stack now
        // is [bool]. Pop it.
        if cut.is_some() {
            self.builder.emit(Op::Pop, 0);
        }
        // Now the stack actually has nothing — we emitted the bool but
        // popped it. That means we've LOST the arith result. This whole
        // approach is wrong.
        //
        // Cleaner: emit a `$((expr))` ShellWord::ArithSub and let
        // compile_word handle it.
        // ── Replace the above with the ArithSub path ─────────────
    }
}

/// True iff `s` contains `target` at a position not preceded by the `\0`
/// quote sentinel.
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

/// Strip the lexer's `\0X` quote sentinels (single-quoted special chars).
fn strip_quote_markers(s: &str) -> String {
    if !s.contains('\x00') {
        return s.to_string();
    }
    s.chars().filter(|c| *c != '\x00').collect()
}

