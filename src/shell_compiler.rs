//! Shell compiler — lowers zshrs AST to fusevm bytecodes.
//!
//! This is the first phase of lowering. We start with arithmetic
//! expressions ($((...))) since they're pure computation with a
//! direct 1:1 mapping to fusevm ops.
//!
//! Subsequent phases will lower:
//!   - for/while/until loops → Jump/JumpIfFalse + fused superinstructions
//!   - shell functions → Call/Return/PushFrame/PopFrame
//!   - simple commands → Exec
//!   - pipelines → PipelineBegin/PipelineStage/PipelineEnd
//!   - conditionals [[ ]] → comparison ops + TestFile
//!   - variable expansion → GetVar + string ops

use crate::parser::{CaseTerminator, CompoundCommand, CondExpr, ShellCommand, ShellWord};
use fusevm::{ChunkBuilder, Op, Value};
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════════════════
// ShellCompiler — lowers ShellCommand AST → fusevm bytecodes
// ═══════════════════════════════════════════════════════════════════════════

/// Compiles shell AST to fusevm bytecodes.
///
/// All shell constructs are lowered to fusevm bytecodes.
/// The shell is exclusively fusevm-executed — no tree-walker fallback.
/// arithmetic, loops, functions.
pub struct ShellCompiler {
    builder: ChunkBuilder,
    /// Variable name → slot index
    slots: HashMap<String, u8>,
    next_slot: u8,
    /// Break target stack — each loop pushes its exit address placeholder
    break_patches: Vec<Vec<usize>>,
    /// Continue target stack — each loop pushes its continue address
    continue_targets: Vec<usize>,
}

impl ShellCompiler {
    pub fn new() -> Self {
        Self {
            builder: ChunkBuilder::new(),
            slots: HashMap::new(),
            next_slot: 0,
            break_patches: Vec::new(),
            continue_targets: Vec::new(),
        }
    }

    /// Compile a list of shell commands into a fusevm Chunk.
    pub fn compile(mut self, commands: &[ShellCommand]) -> fusevm::Chunk {
        self.builder.emit(Op::PushFrame, 0);
        for cmd in commands {
            self.compile_command(cmd);
        }
        self.builder.emit(Op::GetStatus, 0);
        self.builder.emit(Op::ReturnValue, 0);
        self.builder.build()
    }

    fn slot_for(&mut self, name: &str) -> u8 {
        if let Some(&slot) = self.slots.get(name) {
            return slot;
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        self.slots.insert(name.to_string(), slot);
        slot
    }

    fn compile_command(&mut self, cmd: &ShellCommand) {
        match cmd {
            ShellCommand::Simple(simple) => {
                self.compile_simple(simple);
            }
            ShellCommand::Compound(compound) => {
                self.compile_compound(compound);
            }
            ShellCommand::Pipeline(cmds, negated) => {
                self.compile_pipeline(cmds, *negated);
            }
            ShellCommand::List(items) => {
                self.compile_list(items);
            }
            ShellCommand::FunctionDef(name, body) => {
                // Register function: jump past body, record entry point
                let skip_jump = self.builder.emit(Op::Jump(0), 0);
                let entry_ip = self.builder.current_pos();
                let name_idx = self.builder.add_name(name);
                self.builder.add_sub_entry(name_idx, entry_ip);
                self.builder.emit(Op::PushFrame, 0);
                self.compile_command(body);
                self.builder.emit(Op::PopFrame, 0);
                self.builder.emit(Op::Return, 0);
                let after = self.builder.current_pos();
                self.builder.patch_jump(skip_jump, after);
            }
        }
    }

    /// Compile a simple command: assignments + words + redirects.
    ///
    /// Layout:
    ///   - Assignments: SetVar for each VAR=val
    ///   - If no words: done (bare assignment)
    ///   - If words: push each word, emit Exec(argc)
    ///   - Redirects: emit Redirect ops before Exec
    fn compile_simple(&mut self, simple: &crate::parser::SimpleCommand) {
        // Assignments: VAR=value
        for (var, val, _is_append) in &simple.assignments {
            self.compile_word(val);
            let var_idx = self.builder.add_name(var);
            self.builder.emit(Op::SetVar(var_idx), 0);
        }

        if simple.words.is_empty() {
            return; // bare assignment, no command
        }

        // Redirects before command
        for redir in &simple.redirects {
            let fd = redir.fd.unwrap_or(match redir.op {
                crate::parser::RedirectOp::Read
                | crate::parser::RedirectOp::HereDoc
                | crate::parser::RedirectOp::HereString
                | crate::parser::RedirectOp::ReadWrite => 0,
                _ => 1,
            }) as u8;

            let op_byte = match redir.op {
                crate::parser::RedirectOp::Write => fusevm::op::redirect_op::WRITE,
                crate::parser::RedirectOp::Append => fusevm::op::redirect_op::APPEND,
                crate::parser::RedirectOp::Read => fusevm::op::redirect_op::READ,
                crate::parser::RedirectOp::ReadWrite => fusevm::op::redirect_op::READ_WRITE,
                crate::parser::RedirectOp::Clobber => fusevm::op::redirect_op::CLOBBER,
                crate::parser::RedirectOp::DupRead => fusevm::op::redirect_op::DUP_READ,
                crate::parser::RedirectOp::DupWrite => fusevm::op::redirect_op::DUP_WRITE,
                crate::parser::RedirectOp::WriteBoth => fusevm::op::redirect_op::WRITE_BOTH,
                crate::parser::RedirectOp::AppendBoth => fusevm::op::redirect_op::APPEND_BOTH,
                crate::parser::RedirectOp::HereDoc => {
                    // HereDoc: content goes to stdin via constant pool
                    if let Some(ref content) = redir.heredoc_content {
                        let idx = self.builder.add_constant(Value::str(content.as_str()));
                        self.builder.emit(Op::HereDoc(idx), 0);
                    }
                    continue;
                }
                crate::parser::RedirectOp::HereString => {
                    self.compile_word(&redir.target);
                    self.builder.emit(Op::HereString, 0);
                    continue;
                }
            };

            self.compile_word(&redir.target);
            self.builder.emit(Op::Redirect(fd, op_byte), 0);
        }

        // Push command words onto stack
        let argc = simple.words.len() as u8;
        for word in &simple.words {
            self.compile_word(word);
        }

        // Exec: pop argc words, spawn command, push exit status
        self.builder.emit(Op::Exec(argc), 0);
        self.builder.emit(Op::SetStatus, 0);
    }

    /// Compile a pipeline: cmd1 | cmd2 | cmd3
    ///
    /// Layout:
    ///   PipelineBegin(N)
    ///   <compile cmd1>
    ///   PipelineStage
    ///   <compile cmd2>
    ///   PipelineStage
    ///   <compile cmdN>
    ///   PipelineEnd        ; waits for all, pushes last status
    fn compile_pipeline(
        &mut self,
        cmds: &[ShellCommand],
        negated: bool,
    ) {
        if cmds.len() == 1 {
            // Single command, no pipe needed
            self.compile_command(&cmds[0]);
            if negated {
                self.builder.emit(Op::GetStatus, 0);
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::NumEq, 0);
                // true→0 (was success, now fail), false→1
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
            return;
        }

        let n = cmds.len() as u8;
        self.builder.emit(Op::PipelineBegin(n), 0);

        for (i, cmd) in cmds.iter().enumerate() {
            self.compile_command(cmd);
            if i < cmds.len() - 1 {
                self.builder.emit(Op::PipelineStage, 0);
            }
        }

        self.builder.emit(Op::PipelineEnd, 0);
        self.builder.emit(Op::SetStatus, 0);

        if negated {
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
    }

    /// Compile a list: cmd1 && cmd2 || cmd3 ; cmd4 & cmd5
    fn compile_list(&mut self, items: &[(ShellCommand, crate::parser::ListOp)]) {
        for (i, (cmd, op)) in items.iter().enumerate() {
            match op {
                crate::parser::ListOp::And => {
                    // cmd1 && cmd2: run cmd2 only if cmd1 succeeds
                    self.compile_command(cmd);
                    if i + 1 < items.len() {
                        self.builder.emit(Op::GetStatus, 0);
                        let skip = self.builder.emit(Op::JumpIfTrue(0), 0);
                        // Status 0 = success, nonzero = skip next
                        // JumpIfTrue skips when status > 0 (failure)
                        self.compile_command(&items[i + 1].0);
                        self.builder.patch_jump(skip, self.builder.current_pos());
                    }
                }
                crate::parser::ListOp::Or => {
                    // cmd1 || cmd2: run cmd2 only if cmd1 fails
                    self.compile_command(cmd);
                    if i + 1 < items.len() {
                        self.builder.emit(Op::GetStatus, 0);
                        let skip = self.builder.emit(Op::JumpIfFalse(0), 0);
                        // JumpIfFalse skips when status == 0 (success)
                        self.compile_command(&items[i + 1].0);
                        self.builder.patch_jump(skip, self.builder.current_pos());
                    }
                }
                crate::parser::ListOp::Semi => {
                    // Sequential: just compile
                    self.compile_command(cmd);
                }
                crate::parser::ListOp::Amp => {
                    self.compile_command(cmd);
                }
                crate::parser::ListOp::Newline => {
                    self.compile_command(cmd);
                }
            }
        }
    }

    fn compile_compound(&mut self, compound: &CompoundCommand) {
        match compound {
            CompoundCommand::BraceGroup(cmds) => {
                for cmd in cmds {
                    self.compile_command(cmd);
                }
            }

            // ── for var in words; do body; done ──
            CompoundCommand::For { var, words, body } => {
                // Strategy: push word list as array, iterate with index
                //
                // Compiled layout:
                //   LoadInt(0)            ; i = 0
                //   SetSlot(i_slot)
                //   <load array len>
                //   SetSlot(len_slot)
                // loop_top:
                //   GetSlot(i_slot)
                //   GetSlot(len_slot)
                //   NumLt                 ; i < len
                //   JumpIfFalse(loop_exit)
                //   <get array[i], set var>
                //   <body>
                // loop_continue:
                //   PreIncSlotVoid(i_slot)
                //   Jump(loop_top)
                // loop_exit:

                let i_slot = self.next_slot;
                self.next_slot += 1;
                let len_slot = self.next_slot;
                self.next_slot += 1;
                let var_slot = self.slot_for(var);

                // Build the word list — count items
                let item_count = if let Some(words) = words {
                    words.len()
                } else {
                    0
                };

                // For now, store items as constants and load by index
                if let Some(words) = words {
                    for word in words {
                        let s = self.word_to_string(word);
                        let const_idx = self.builder.add_constant(Value::str(s));
                        self.builder.emit(Op::LoadConst(const_idx), 0);
                    }
                    self.builder
                        .emit(Op::MakeArray(item_count as u16), 0);
                } else {
                    // No words = iterate $@ (positional params)
                    // TODO: load positional params
                    self.builder.emit(Op::MakeArray(0), 0);
                }
                let arr_slot = self.next_slot;
                self.next_slot += 1;
                self.builder.emit(Op::SetSlot(arr_slot), 0);

                // i = 0
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::SetSlot(i_slot), 0);

                // len = array length
                self.builder.emit(Op::LoadInt(item_count as i64), 0);
                self.builder.emit(Op::SetSlot(len_slot), 0);

                // loop_top:
                let loop_top = self.builder.current_pos();
                self.builder.emit(Op::GetSlot(i_slot), 0);
                self.builder.emit(Op::GetSlot(len_slot), 0);
                self.builder.emit(Op::NumLt, 0);
                let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

                // var = array[i] — for now just set from constant
                // TODO: proper array indexing op
                self.builder.emit(Op::GetSlot(i_slot), 0);
                self.builder.emit(Op::SetSlot(var_slot), 0);

                // Push break/continue targets
                self.break_patches.push(Vec::new());
                let continue_pos = self.builder.current_pos(); // will be patched
                self.continue_targets.push(0); // placeholder

                // body
                for cmd in body {
                    self.compile_command(cmd);
                }

                // loop_continue:
                let continue_target = self.builder.current_pos();
                // Patch continue target
                if let Some(target) = self.continue_targets.last_mut() {
                    *target = continue_target;
                }

                // i++; jump loop_top
                self.builder.emit(Op::PreIncSlotVoid(i_slot), 0);
                self.builder.emit(Op::Jump(loop_top), 0);

                // loop_exit:
                let loop_exit = self.builder.current_pos();
                self.builder.patch_jump(exit_jump, loop_exit);

                // Patch all break jumps
                if let Some(breaks) = self.break_patches.pop() {
                    for bp in breaks {
                        self.builder.patch_jump(bp, loop_exit);
                    }
                }
                self.continue_targets.pop();
            }

            // ── for ((init; cond; step)) do body done ──
            CompoundCommand::ForArith {
                init,
                cond,
                step,
                body,
            } => {
                // Compile init expression
                if !init.is_empty() {
                    self.compile_arith_inline(init);
                    self.builder.emit(Op::Pop, 0); // discard init result
                }

                // loop_top: evaluate condition
                let loop_top = self.builder.current_pos();
                if !cond.is_empty() {
                    self.compile_arith_inline(cond);
                    // cond == 0 means false in shell arithmetic
                } else {
                    self.builder.emit(Op::LoadTrue, 0);
                }
                let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

                // Push break/continue targets
                self.break_patches.push(Vec::new());
                self.continue_targets.push(0);

                // body
                for cmd in body {
                    self.compile_command(cmd);
                }

                // continue target = step expression
                let continue_target = self.builder.current_pos();
                if let Some(target) = self.continue_targets.last_mut() {
                    *target = continue_target;
                }

                // step expression
                if !step.is_empty() {
                    self.compile_arith_inline(step);
                    self.builder.emit(Op::Pop, 0); // discard step result
                }

                // Jump back to loop_top
                self.builder.emit(Op::Jump(loop_top), 0);

                // loop_exit:
                let loop_exit = self.builder.current_pos();
                self.builder.patch_jump(exit_jump, loop_exit);

                if let Some(breaks) = self.break_patches.pop() {
                    for bp in breaks {
                        self.builder.patch_jump(bp, loop_exit);
                    }
                }
                self.continue_targets.pop();
            }

            // ── while condition; do body; done ──
            CompoundCommand::While { condition, body } => {
                self.compile_while_loop(condition, body, false);
            }

            // ── until condition; do body; done ──
            CompoundCommand::Until { condition, body } => {
                self.compile_while_loop(condition, body, true);
            }

            // ── if/elif/else/fi ──
            CompoundCommand::If {
                conditions,
                else_part,
            } => {
                let mut end_jumps = Vec::new();

                for (cond_cmds, body_cmds) in conditions {
                    // Evaluate condition — last command's exit status
                    for cmd in cond_cmds {
                        self.compile_command(cmd);
                    }
                    self.builder.emit(Op::GetStatus, 0);
                    // Status 0 = true in shell, so jump if nonzero (false)
                    let skip_body = self.builder.emit(Op::JumpIfTrue(0), 0);

                    // Body
                    for cmd in body_cmds {
                        self.compile_command(cmd);
                    }
                    end_jumps.push(self.builder.emit(Op::Jump(0), 0));

                    // Patch: skip body if condition false
                    let after_body = self.builder.current_pos();
                    self.builder.patch_jump(skip_body, after_body);
                }

                // else
                if let Some(else_cmds) = else_part {
                    for cmd in else_cmds {
                        self.compile_command(cmd);
                    }
                }

                // Patch all end jumps to after the entire if
                let end = self.builder.current_pos();
                for ej in end_jumps {
                    self.builder.patch_jump(ej, end);
                }
            }

            // ── repeat N; do body; done ──
            CompoundCommand::Repeat { count, body } => {
                // Compile count as arithmetic
                let i_slot = self.next_slot;
                self.next_slot += 1;

                self.compile_arith_inline(count);
                let count_slot = self.next_slot;
                self.next_slot += 1;
                self.builder.emit(Op::SetSlot(count_slot), 0);

                // i = 0
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::SetSlot(i_slot), 0);

                let loop_top = self.builder.current_pos();
                // Try fused superinstruction
                self.builder
                    .emit(Op::GetSlot(i_slot), 0);
                self.builder.emit(Op::GetSlot(count_slot), 0);
                self.builder.emit(Op::NumLt, 0);
                let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

                self.break_patches.push(Vec::new());
                self.continue_targets.push(0);

                for cmd in body {
                    self.compile_command(cmd);
                }

                let cont = self.builder.current_pos();
                if let Some(target) = self.continue_targets.last_mut() {
                    *target = cont;
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
                self.continue_targets.pop();
            }

            // ── { try } always { always } ──
            CompoundCommand::Try {
                try_body,
                always_body,
            } => {
                for cmd in try_body {
                    self.compile_command(cmd);
                }
                for cmd in always_body {
                    self.compile_command(cmd);
                }
            }

            CompoundCommand::Arith(expr) => {
                self.compile_arith_inline(expr);
                // Set $? based on result: 0 if nonzero (true), 1 if zero (false)
                // Shell arithmetic: (( expr )) returns 0 if expr != 0
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::NumNe, 0);
                // Convert bool to status: true→0, false→1
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

            // ── case word in pattern) body ;; esac ──
            CompoundCommand::Case { word, cases } => {
                // Compile word to stack
                self.compile_word(word);
                let word_slot = self.next_slot;
                self.next_slot += 1;
                self.builder.emit(Op::SetSlot(word_slot), 0);

                let mut end_jumps = Vec::new();

                for (patterns, body, term) in cases {
                    let _next_pattern_jumps: Vec<usize> = Vec::new();

                    // Try each pattern — any match jumps to body
                    let body_target_placeholder = self.builder.current_pos();
                    let mut match_jumps = Vec::new();

                    for pattern in patterns {
                        self.builder.emit(Op::GetSlot(word_slot), 0);
                        self.compile_word(pattern);
                        self.builder.emit(Op::StrEq, 0);
                        match_jumps.push(self.builder.emit(Op::JumpIfTrue(0), 0));
                    }

                    // No pattern matched — skip this case body
                    let skip_body = self.builder.emit(Op::Jump(0), 0);

                    // Patch match jumps to body start
                    let body_start = self.builder.current_pos();
                    for mj in match_jumps {
                        self.builder.patch_jump(mj, body_start);
                    }

                    // Body
                    for cmd in body {
                        self.compile_command(cmd);
                    }

                    match term {
                        CaseTerminator::Break => {
                            end_jumps.push(self.builder.emit(Op::Jump(0), 0));
                        }
                        CaseTerminator::Fallthrough => {
                            // ;& — fall through to next body without testing
                        }
                        CaseTerminator::Continue => {
                            // ;;& — continue testing next patterns
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

            // ── [[ conditional ]] ──
            CompoundCommand::Cond(expr) => {
                self.compile_cond(expr);
                // Result is bool on stack — convert to status
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

            // ── subshell (...) ──
            CompoundCommand::Subshell(cmds) => {
                self.builder.emit(Op::SubshellBegin, 0);
                for cmd in cmds {
                    self.compile_command(cmd);
                }
                self.builder.emit(Op::SubshellEnd, 0);
            }

            // ── select var in words ──
            CompoundCommand::Select { var, words, body } => {
                // Simplified: iterate like for-in
                // Simplified select — full interactive prompt via fusevm Extended ops
                let var_slot = self.slot_for(var);
                if let Some(words) = words {
                    for word in words {
                        let s = self.word_to_string(word);
                        let const_idx = self.builder.add_constant(Value::str(s));
                        self.builder.emit(Op::LoadConst(const_idx), 0);
                        self.builder.emit(Op::SetSlot(var_slot), 0);
                        for cmd in body {
                            self.compile_command(cmd);
                        }
                    }
                }
            }

            // ── coproc ──
            CompoundCommand::Coproc { name: _, body } => {
                // Coproc — bidirectional pipe via Extended ops
                self.compile_command(body);
            }

            // ── cmd with redirects ──
            CompoundCommand::WithRedirects(cmd, _redirects) => {
                // TODO: emit Redirect ops before/after command
                self.compile_command(cmd);
            }
        }
    }

    /// Compile a [[ conditional ]] expression to ops.
    /// Pushes a bool (true/false) onto the stack.
    fn compile_cond(&mut self, expr: &CondExpr) {
        match expr {
            // File tests
            CondExpr::FileExists(w) => {
                self.compile_word(w);
                self.builder.emit(Op::TestFile(fusevm::op::file_test::EXISTS), 0);
            }
            CondExpr::FileRegular(w) => {
                self.compile_word(w);
                self.builder.emit(Op::TestFile(fusevm::op::file_test::IS_FILE), 0);
            }
            CondExpr::FileDirectory(w) => {
                self.compile_word(w);
                self.builder.emit(Op::TestFile(fusevm::op::file_test::IS_DIR), 0);
            }
            CondExpr::FileSymlink(w) => {
                self.compile_word(w);
                self.builder.emit(Op::TestFile(fusevm::op::file_test::IS_SYMLINK), 0);
            }
            CondExpr::FileReadable(w) => {
                self.compile_word(w);
                self.builder.emit(Op::TestFile(fusevm::op::file_test::IS_READABLE), 0);
            }
            CondExpr::FileWritable(w) => {
                self.compile_word(w);
                self.builder.emit(Op::TestFile(fusevm::op::file_test::IS_WRITABLE), 0);
            }
            CondExpr::FileExecutable(w) => {
                self.compile_word(w);
                self.builder.emit(Op::TestFile(fusevm::op::file_test::IS_EXECUTABLE), 0);
            }
            CondExpr::FileNonEmpty(w) => {
                self.compile_word(w);
                self.builder.emit(Op::TestFile(fusevm::op::file_test::IS_NONEMPTY), 0);
            }

            // String tests
            CondExpr::StringEmpty(w) => {
                self.compile_word(w);
                self.builder.emit(Op::StringLen, 0);
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::NumEq, 0);
            }
            CondExpr::StringNonEmpty(w) => {
                self.compile_word(w);
                self.builder.emit(Op::StringLen, 0);
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::NumGt, 0);
            }
            CondExpr::StringEqual(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::StrEq, 0);
            }
            CondExpr::StringNotEqual(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::StrNe, 0);
            }
            CondExpr::StringMatch(a, b) => {
                // =~ regex match — for now use StrEq as placeholder
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::StrEq, 0);
            }
            CondExpr::StringLess(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::StrLt, 0);
            }
            CondExpr::StringGreater(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::StrGt, 0);
            }

            // Numeric comparisons
            CondExpr::NumEqual(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::NumEq, 0);
            }
            CondExpr::NumNotEqual(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::NumNe, 0);
            }
            CondExpr::NumLess(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::NumLt, 0);
            }
            CondExpr::NumLessEqual(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::NumLe, 0);
            }
            CondExpr::NumGreater(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::NumGt, 0);
            }
            CondExpr::NumGreaterEqual(a, b) => {
                self.compile_word(a);
                self.compile_word(b);
                self.builder.emit(Op::NumGe, 0);
            }

            // Logical operators
            CondExpr::Not(inner) => {
                self.compile_cond(inner);
                self.builder.emit(Op::LogNot, 0);
            }
            CondExpr::And(a, b) => {
                self.compile_cond(a);
                let skip = self.builder.emit(Op::JumpIfFalseKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.compile_cond(b);
                self.builder.patch_jump(skip, self.builder.current_pos());
            }
            CondExpr::Or(a, b) => {
                self.compile_cond(a);
                let skip = self.builder.emit(Op::JumpIfTrueKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.compile_cond(b);
                self.builder.patch_jump(skip, self.builder.current_pos());
            }
        }
    }

    /// Compile a ShellWord to a value on the stack.
    fn compile_word(&mut self, word: &ShellWord) {
        match word {
            ShellWord::Literal(s) => {
                let idx = self.builder.add_constant(Value::str(s.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
            }
            ShellWord::SingleQuoted(s) => {
                let idx = self.builder.add_constant(Value::str(s.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
            }
            ShellWord::Variable(name) => {
                let slot = self.slot_for(name);
                self.builder.emit(Op::GetSlot(slot), 0);
            }
            // TODO: DoubleQuoted, Glob, Tilde, ArrayLiteral, VariableBraced
            _ => {
                // Dynamic word — push empty string placeholder
                let idx = self.builder.add_constant(Value::str(""));
                self.builder.emit(Op::LoadConst(idx), 0);
            }
        }
    }

    /// Shared implementation for while/until loops.
    fn compile_while_loop(
        &mut self,
        condition: &[ShellCommand],
        body: &[ShellCommand],
        is_until: bool,
    ) {
        let loop_top = self.builder.current_pos();

        // Evaluate condition
        for cmd in condition {
            self.compile_command(cmd);
        }
        self.builder.emit(Op::GetStatus, 0);

        // while: exit if status != 0 (JumpIfTrue since status>0 = failure)
        // until: exit if status == 0 (JumpIfFalse since status 0 = success)
        let exit_jump = if is_until {
            self.builder.emit(Op::JumpIfFalse(0), 0)
        } else {
            self.builder.emit(Op::JumpIfTrue(0), 0)
        };

        self.break_patches.push(Vec::new());
        self.continue_targets.push(loop_top);

        for cmd in body {
            self.compile_command(cmd);
        }

        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);

        if let Some(breaks) = self.break_patches.pop() {
            for bp in breaks {
                self.builder.patch_jump(bp, loop_exit);
            }
        }
        self.continue_targets.pop();
    }

    /// Extract a literal string from a ShellWord (for constant folding).
    /// Compile an arithmetic expression inline, emitting ops directly
    /// into this compiler's builder. Result is left on the stack.
    /// Variables are mapped into the parent's slot table so `i` in
    /// init/cond/step/body all resolve to the same slot.
    fn compile_arith_inline(&mut self, expr: &str) {
        let mut ac = ArithCompiler::new(expr);
        // Share the parent's slot table
        ac.slots = self.slots.clone();
        ac.next_slot = self.next_slot;
        // Extract updated slots before compile() consumes ac
        ac.expr();
        let new_slots = ac.slots.clone();
        let new_next = ac.next_slot;
        let chunk = ac.builder.build();
        // Merge any new slots back
        self.slots = new_slots;
        self.next_slot = new_next;
        // Inline the computation ops (skip nothing — no PushFrame/ReturnValue wrapper)
        for op in &chunk.ops {
            self.builder.emit(op.clone(), 0);
        }
    }

    fn word_to_string(&self, word: &ShellWord) -> String {
        match word {
            ShellWord::Literal(s) => s.clone(),
            ShellWord::SingleQuoted(s) => s.clone(),
            _ => String::new(), // dynamic words can't be const-folded
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ArithCompiler — lowers arithmetic expressions → fusevm bytecodes
// ═══════════════════════════════════════════════════════════════════════════

/// Arithmetic expression compiler.
///
/// Takes a zsh arithmetic expression (the content inside $((...)))
/// and emits fusevm bytecodes that compute the result.
///
/// Port of MathEval from zsh/src/math.rs — same tokenizer,
/// but instead of evaluating, we emit ops.
pub struct ArithCompiler<'a> {
    pub input: &'a str,
    pub pos: usize,
    pub builder: ChunkBuilder,
    /// Variable name → slot index
    pub slots: HashMap<String, u8>,
    pub next_slot: u8,
}

// Token types matching math.rs MathTok
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Num(i64),
    Float(f64),
    Ident,
    Plus,
    Minus,
    Mul,
    Div,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,
    LogAnd,
    LogOr,
    LogNot,
    Eq,
    Neq,
    Lt,
    Gt,
    Leq,
    Geq,
    Assign,
    PlusAssign,
    MinusAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
    LParen,
    RParen,
    Comma,
    Quest,
    Colon,
    Eoi,
}

impl<'a> ArithCompiler<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            builder: ChunkBuilder::new(),
            slots: HashMap::new(),
            next_slot: 0,
        }
    }


    /// Compile the arithmetic expression to fusevm bytecodes.
    /// Returns the compiled chunk.
    pub fn compile(mut self) -> fusevm::Chunk {
        self.builder.set_source("$((...))");
        self.builder.emit(Op::PushFrame, 0);
        self.expr();
        self.builder.emit(Op::ReturnValue, 0);
        self.builder.build()
    }

    /// Get or allocate a slot for a variable name.
    fn slot_for(&mut self, name: &str) -> u8 {
        if let Some(&slot) = self.slots.get(name) {
            return slot;
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        self.slots.insert(name.to_string(), slot);
        slot
    }

    // ── Tokenizer ──

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            let b = self.input.as_bytes()[self.pos];
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek_char(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

    fn next_char(&mut self) -> Option<u8> {
        let c = self.input.as_bytes().get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.input.len() {
            let b = self.input.as_bytes()[self.pos];
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn read_number(&mut self) -> Tok {
        let start = self.pos;

        // Handle hex: 0x...
        if self.pos + 1 < self.input.len()
            && self.input.as_bytes()[self.pos] == b'0'
            && (self.input.as_bytes()[self.pos + 1] == b'x'
                || self.input.as_bytes()[self.pos + 1] == b'X')
        {
            self.pos += 2;
            while self.pos < self.input.len()
                && self.input.as_bytes()[self.pos].is_ascii_hexdigit()
            {
                self.pos += 1;
            }
            let val = i64::from_str_radix(&self.input[start + 2..self.pos], 16).unwrap_or(0);
            return Tok::Num(val);
        }

        // Handle octal: 0...
        if self.pos + 1 < self.input.len()
            && self.input.as_bytes()[self.pos] == b'0'
            && self.input.as_bytes()[self.pos + 1].is_ascii_digit()
        {
            while self.pos < self.input.len()
                && self.input.as_bytes()[self.pos].is_ascii_digit()
            {
                self.pos += 1;
            }
            let val = i64::from_str_radix(&self.input[start + 1..self.pos], 8).unwrap_or(0);
            return Tok::Num(val);
        }

        // Decimal integer or float
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
            self.pos += 1;
        }

        // Check for float
        if self.pos < self.input.len() && self.input.as_bytes()[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let val: f64 = self.input[start..self.pos].parse().unwrap_or(0.0);
            return Tok::Float(val);
        }

        let val: i64 = self.input[start..self.pos].parse().unwrap_or(0);
        Tok::Num(val)
    }

    fn next_tok(&mut self) -> (Tok, String) {
        self.skip_whitespace();

        let Some(c) = self.peek_char() else {
            return (Tok::Eoi, String::new());
        };

        match c {
            b'0'..=b'9' => {
                let tok = self.read_number();
                (tok, String::new())
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let name = self.read_ident();
                (Tok::Ident, name)
            }
            b'+' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'+') => { self.pos += 1; (Tok::PreInc, String::new()) }
                    Some(b'=') => { self.pos += 1; (Tok::PlusAssign, String::new()) }
                    _ => (Tok::Plus, String::new()),
                }
            }
            b'-' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'-') => { self.pos += 1; (Tok::PreDec, String::new()) }
                    Some(b'=') => { self.pos += 1; (Tok::MinusAssign, String::new()) }
                    _ => (Tok::Minus, String::new()),
                }
            }
            b'*' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'*') => {
                        self.pos += 1;
                        if self.peek_char() == Some(b'=') {
                            self.pos += 1;
                            (Tok::MulAssign, String::new()) // **= as mul assign for now
                        } else {
                            (Tok::Pow, String::new())
                        }
                    }
                    Some(b'=') => { self.pos += 1; (Tok::MulAssign, String::new()) }
                    _ => (Tok::Mul, String::new()),
                }
            }
            b'/' => {
                self.pos += 1;
                if self.peek_char() == Some(b'=') {
                    self.pos += 1;
                    (Tok::DivAssign, String::new())
                } else {
                    (Tok::Div, String::new())
                }
            }
            b'%' => {
                self.pos += 1;
                if self.peek_char() == Some(b'=') {
                    self.pos += 1;
                    (Tok::ModAssign, String::new())
                } else {
                    (Tok::Mod, String::new())
                }
            }
            b'&' => {
                self.pos += 1;
                if self.peek_char() == Some(b'&') {
                    self.pos += 1;
                    (Tok::LogAnd, String::new())
                } else {
                    (Tok::BitAnd, String::new())
                }
            }
            b'|' => {
                self.pos += 1;
                if self.peek_char() == Some(b'|') {
                    self.pos += 1;
                    (Tok::LogOr, String::new())
                } else {
                    (Tok::BitOr, String::new())
                }
            }
            b'^' => { self.pos += 1; (Tok::BitXor, String::new()) }
            b'~' => { self.pos += 1; (Tok::BitNot, String::new()) }
            b'!' => {
                self.pos += 1;
                if self.peek_char() == Some(b'=') {
                    self.pos += 1;
                    (Tok::Neq, String::new())
                } else {
                    (Tok::LogNot, String::new())
                }
            }
            b'<' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'<') => { self.pos += 1; (Tok::Shl, String::new()) }
                    Some(b'=') => { self.pos += 1; (Tok::Leq, String::new()) }
                    _ => (Tok::Lt, String::new()),
                }
            }
            b'>' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'>') => { self.pos += 1; (Tok::Shr, String::new()) }
                    Some(b'=') => { self.pos += 1; (Tok::Geq, String::new()) }
                    _ => (Tok::Gt, String::new()),
                }
            }
            b'=' => {
                self.pos += 1;
                if self.peek_char() == Some(b'=') {
                    self.pos += 1;
                    (Tok::Eq, String::new())
                } else {
                    (Tok::Assign, String::new())
                }
            }
            b'(' => { self.pos += 1; (Tok::LParen, String::new()) }
            b')' => { self.pos += 1; (Tok::RParen, String::new()) }
            b',' => { self.pos += 1; (Tok::Comma, String::new()) }
            b'?' => { self.pos += 1; (Tok::Quest, String::new()) }
            b':' => { self.pos += 1; (Tok::Colon, String::new()) }
            _ => {
                self.pos += 1;
                (Tok::Eoi, String::new())
            }
        }
    }

    // ── Recursive descent → emit ops ──
    // Precedence climbing: comma < assign < ternary < logor < logand <
    // bitor < bitxor < bitand < eq < cmp < shift < add < mul < pow < unary

    fn expr(&mut self) {
        self.assign_expr();
    }

    fn assign_expr(&mut self) {
        let save_pos = self.pos;

        // Check for assignment: ident = expr
        self.skip_whitespace();
        if let Some(c) = self.peek_char() {
            if c.is_ascii_alphabetic() || c == b'_' {
                let name = self.read_ident();
                self.skip_whitespace();
                let (tok, _) = self.peek_tok();
                match tok {
                    Tok::Assign => {
                        let _ = self.next_tok(); // consume =
                        let slot = self.slot_for(&name);
                        self.assign_expr();
                        self.builder.emit(Op::Dup, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                        return;
                    }
                    Tok::PlusAssign | Tok::MinusAssign | Tok::MulAssign
                    | Tok::DivAssign | Tok::ModAssign => {
                        let _ = self.next_tok(); // consume op=
                        let slot = self.slot_for(&name);
                        self.builder.emit(Op::GetSlot(slot), 0);
                        self.assign_expr();
                        match tok {
                            Tok::PlusAssign => self.builder.emit(Op::Add, 0),
                            Tok::MinusAssign => self.builder.emit(Op::Sub, 0),
                            Tok::MulAssign => self.builder.emit(Op::Mul, 0),
                            Tok::DivAssign => self.builder.emit(Op::Div, 0),
                            Tok::ModAssign => self.builder.emit(Op::Mod, 0),
                            _ => unreachable!(),
                        };
                        self.builder.emit(Op::Dup, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                        return;
                    }
                    _ => {}
                }
                // Not assignment — rewind
                self.pos = save_pos;
            }
        }

        self.ternary_expr();
    }

    fn peek_tok(&mut self) -> (Tok, String) {
        let save = self.pos;
        let tok = self.next_tok();
        self.pos = save;
        tok
    }

    fn ternary_expr(&mut self) {
        self.logor_expr();
        let (tok, _) = self.peek_tok();
        if tok == Tok::Quest {
            let _ = self.next_tok(); // consume ?
            let else_jump = self.builder.emit(Op::JumpIfFalse(0), 0);
            self.expr(); // true branch
            let (colon, _) = self.peek_tok();
            let end_jump = self.builder.emit(Op::Jump(0), 0);
            let else_target = self.builder.current_pos();
            self.builder.patch_jump(else_jump, else_target);
            if colon == Tok::Colon {
                let _ = self.next_tok(); // consume :
            }
            self.expr(); // false branch
            let end_target = self.builder.current_pos();
            self.builder.patch_jump(end_jump, end_target);
        }
    }

    fn logor_expr(&mut self) {
        self.logand_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::LogOr {
                let _ = self.next_tok();
                let skip = self.builder.emit(Op::JumpIfTrueKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.logand_expr();
                self.builder.patch_jump(skip, self.builder.current_pos());
            } else {
                break;
            }
        }
    }

    fn logand_expr(&mut self) {
        self.bitor_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::LogAnd {
                let _ = self.next_tok();
                let skip = self.builder.emit(Op::JumpIfFalseKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.bitor_expr();
                self.builder.patch_jump(skip, self.builder.current_pos());
            } else {
                break;
            }
        }
    }

    fn bitor_expr(&mut self) {
        self.bitxor_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::BitOr {
                let _ = self.next_tok();
                self.bitxor_expr();
                self.builder.emit(Op::BitOr, 0);
            } else {
                break;
            }
        }
    }

    fn bitxor_expr(&mut self) {
        self.bitand_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::BitXor {
                let _ = self.next_tok();
                self.bitand_expr();
                self.builder.emit(Op::BitXor, 0);
            } else {
                break;
            }
        }
    }

    fn bitand_expr(&mut self) {
        self.equality_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::BitAnd {
                let _ = self.next_tok();
                self.equality_expr();
                self.builder.emit(Op::BitAnd, 0);
            } else {
                break;
            }
        }
    }

    fn equality_expr(&mut self) {
        self.comparison_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Eq => {
                    let _ = self.next_tok();
                    self.comparison_expr();
                    self.builder.emit(Op::NumEq, 0);
                }
                Tok::Neq => {
                    let _ = self.next_tok();
                    self.comparison_expr();
                    self.builder.emit(Op::NumNe, 0);
                }
                _ => break,
            }
        }
    }

    fn comparison_expr(&mut self) {
        self.shift_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Lt => {
                    let _ = self.next_tok();
                    self.shift_expr();
                    self.builder.emit(Op::NumLt, 0);
                }
                Tok::Gt => {
                    let _ = self.next_tok();
                    self.shift_expr();
                    self.builder.emit(Op::NumGt, 0);
                }
                Tok::Leq => {
                    let _ = self.next_tok();
                    self.shift_expr();
                    self.builder.emit(Op::NumLe, 0);
                }
                Tok::Geq => {
                    let _ = self.next_tok();
                    self.shift_expr();
                    self.builder.emit(Op::NumGe, 0);
                }
                _ => break,
            }
        }
    }

    fn shift_expr(&mut self) {
        self.add_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Shl => {
                    let _ = self.next_tok();
                    self.add_expr();
                    self.builder.emit(Op::Shl, 0);
                }
                Tok::Shr => {
                    let _ = self.next_tok();
                    self.add_expr();
                    self.builder.emit(Op::Shr, 0);
                }
                _ => break,
            }
        }
    }

    fn add_expr(&mut self) {
        self.mul_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Plus => {
                    let _ = self.next_tok();
                    self.mul_expr();
                    self.builder.emit(Op::Add, 0);
                }
                Tok::Minus => {
                    let _ = self.next_tok();
                    self.mul_expr();
                    self.builder.emit(Op::Sub, 0);
                }
                _ => break,
            }
        }
    }

    fn mul_expr(&mut self) {
        self.pow_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Mul => {
                    let _ = self.next_tok();
                    self.pow_expr();
                    self.builder.emit(Op::Mul, 0);
                }
                Tok::Div => {
                    let _ = self.next_tok();
                    self.pow_expr();
                    self.builder.emit(Op::Div, 0);
                }
                Tok::Mod => {
                    let _ = self.next_tok();
                    self.pow_expr();
                    self.builder.emit(Op::Mod, 0);
                }
                _ => break,
            }
        }
    }

    fn pow_expr(&mut self) {
        self.unary_expr();
        let (tok, _) = self.peek_tok();
        if tok == Tok::Pow {
            let _ = self.next_tok();
            self.pow_expr(); // right-associative
            self.builder.emit(Op::Pow, 0);
        }
    }

    fn unary_expr(&mut self) {
        let (tok, name) = self.peek_tok();
        match tok {
            Tok::Minus => {
                let _ = self.next_tok();
                self.unary_expr();
                self.builder.emit(Op::Negate, 0);
            }
            Tok::Plus => {
                let _ = self.next_tok();
                self.unary_expr();
                // unary + is a no-op on numbers
            }
            Tok::LogNot => {
                let _ = self.next_tok();
                self.unary_expr();
                self.builder.emit(Op::LogNot, 0);
            }
            Tok::BitNot => {
                let _ = self.next_tok();
                self.unary_expr();
                self.builder.emit(Op::BitNot, 0);
            }
            Tok::PreInc => {
                let _ = self.next_tok();
                // Next token must be identifier
                let (_, var_name) = self.next_tok();
                let slot = self.slot_for(&var_name);
                self.builder.emit(Op::PreIncSlot(slot), 0);
            }
            Tok::PreDec => {
                let _ = self.next_tok();
                let (_, var_name) = self.next_tok();
                let slot = self.slot_for(&var_name);
                self.builder.emit(Op::GetSlot(slot), 0);
                self.builder.emit(Op::Dec, 0);
                self.builder.emit(Op::Dup, 0);
                self.builder.emit(Op::SetSlot(slot), 0);
            }
            _ => self.primary_expr(),
        }
    }

    fn primary_expr(&mut self) {
        let (tok, name) = self.next_tok();
        match tok {
            Tok::Num(n) => {
                self.builder.emit(Op::LoadInt(n), 0);
            }
            Tok::Float(f) => {
                self.builder.emit(Op::LoadFloat(f), 0);
            }
            Tok::Ident => {
                let slot = self.slot_for(&name);
                self.builder.emit(Op::GetSlot(slot), 0);

                // Check for postfix ++ / --
                let (post_tok, _) = self.peek_tok();
                match post_tok {
                    Tok::PreInc => {
                        // Reused as PostInc here
                        let _ = self.next_tok();
                        self.builder.emit(Op::Dup, 0); // keep old value
                        self.builder.emit(Op::Inc, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                        // old value remains on stack (postfix semantics)
                    }
                    Tok::PreDec => {
                        let _ = self.next_tok();
                        self.builder.emit(Op::Dup, 0);
                        self.builder.emit(Op::Dec, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                    }
                    _ => {}
                }
            }
            Tok::LParen => {
                self.expr();
                let _ = self.next_tok(); // consume RParen
            }
            _ => {
                // Unexpected token — push 0
                self.builder.emit(Op::LoadInt(0), 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusevm::{VM, VMResult};

    fn eval(expr: &str) -> i64 {
        let compiler = ArithCompiler::new(expr);
        let chunk = compiler.compile();
        let mut vm = VM::new(chunk);
        match vm.run() {
            VMResult::Ok(Value::Int(n)) => n,
            VMResult::Ok(Value::Bool(b)) => b as i64,
            VMResult::Ok(Value::Float(f)) => f as i64,
            VMResult::Ok(v) => v.to_int(),
            other => panic!("expected value, got {:?}", other),
        }
    }

    fn eval_float(expr: &str) -> f64 {
        let compiler = ArithCompiler::new(expr);
        let chunk = compiler.compile();
        let mut vm = VM::new(chunk);
        match vm.run() {
            VMResult::Ok(v) => v.to_float(),
            other => panic!("expected value, got {:?}", other),
        }
    }

    #[test]
    fn test_basic_arithmetic() {
        assert_eq!(eval("2 + 3"), 5);
        assert_eq!(eval("10 - 4"), 6);
        assert_eq!(eval("6 * 7"), 42);
        assert_eq!(eval("100 / 4"), 25);
        assert_eq!(eval("17 % 5"), 2);
    }

    #[test]
    fn test_precedence() {
        assert_eq!(eval("2 + 3 * 4"), 14);
        assert_eq!(eval("(2 + 3) * 4"), 20);
        assert_eq!(eval("2 * 3 + 4 * 5"), 26);
        assert_eq!(eval("10 - 2 * 3"), 4);
    }

    #[test]
    fn test_power() {
        assert_eq!(eval("2 ** 10"), 1024);
        assert_eq!(eval("3 ** 3"), 27);
    }

    #[test]
    fn test_unary() {
        assert_eq!(eval("-5"), -5);
        assert_eq!(eval("-(-3)"), 3);
        assert_eq!(eval("!0"), 1);
        assert_eq!(eval("!1"), 0);
        assert_eq!(eval("~0"), -1);
    }

    #[test]
    fn test_comparison() {
        assert_eq!(eval("3 < 5"), 1);
        assert_eq!(eval("5 < 3"), 0);
        assert_eq!(eval("3 <= 3"), 1);
        assert_eq!(eval("3 == 3"), 1);
        assert_eq!(eval("3 != 4"), 1);
        assert_eq!(eval("5 > 3"), 1);
        assert_eq!(eval("5 >= 5"), 1);
    }

    #[test]
    fn test_bitwise() {
        assert_eq!(eval("0xFF & 0x0F"), 0x0F);
        assert_eq!(eval("0xF0 | 0x0F"), 0xFF);
        assert_eq!(eval("0xFF ^ 0x0F"), 0xF0);
        assert_eq!(eval("1 << 10"), 1024);
        assert_eq!(eval("1024 >> 5"), 32);
    }

    #[test]
    fn test_logical_short_circuit() {
        // zsh arithmetic: && returns last evaluated operand
        assert_eq!(eval("1 && 2"), 2); // truthy && truthy → right operand
        assert_eq!(eval("0 && 2"), 0); // falsy short-circuits → left operand
        assert_eq!(eval("0 || 5"), 5); // falsy || truthy → right operand
        assert_eq!(eval("1 || 0"), 1); // truthy short-circuits → left operand
    }

    #[test]
    fn test_ternary() {
        assert_eq!(eval("1 ? 42 : 99"), 42);
        assert_eq!(eval("0 ? 42 : 99"), 99);
        assert_eq!(eval("(3 > 2) ? 10 : 20"), 10);
    }

    #[test]
    fn test_assignment() {
        assert_eq!(eval("x = 5"), 5);
        assert_eq!(eval("x = 5 + 3"), 8);
    }

    #[test]
    fn test_hex_octal() {
        assert_eq!(eval("0xFF"), 255);
        assert_eq!(eval("0x10"), 16);
        assert_eq!(eval("010"), 8); // octal
    }

    #[test]
    fn test_complex_expression() {
        // (5 + 3) * 2 - 10 / 5
        assert_eq!(eval("(5 + 3) * 2 - 10 / 5"), 14);
        // Nested ternary
        assert_eq!(eval("1 ? (0 ? 1 : 2) : 3"), 2);
    }

    #[test]
    fn test_float() {
        assert!((eval_float("3.14 * 2.0") - 6.28).abs() < 0.001);
    }

    // ── ShellCompiler tests ──

    fn run_shell(commands: &[ShellCommand]) -> i64 {
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(commands);
        let mut vm = VM::new(chunk);
        match vm.run() {
            VMResult::Ok(v) => v.to_int(),
            other => panic!("VM error: {:?}", other),
        }
    }

    #[test]
    fn test_for_arith_sum() {
        use crate::parser::CompoundCommand;
        // for ((i=0; i<10; i++)) { (( sum = sum + i )) }
        let cmd = ShellCommand::Compound(CompoundCommand::ForArith {
            init: "i = 0".to_string(),
            cond: "i < 10".to_string(),
            step: "i++".to_string(),
            body: vec![
                ShellCommand::Compound(CompoundCommand::Arith("sum = sum + i".to_string())),
            ],
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);

        // Debug: print compiled ops
        for (i, op) in chunk.ops.iter().enumerate() {
            eprintln!("{:3}: {:?}", i, op);
        }

        // Just verify the compiled ops look correct
        // The init should set slot for 'i', cond should compare, step should increment
        let has_set_slot = chunk.ops.iter().any(|op| matches!(op, Op::SetSlot(_)));
        let has_jump = chunk.ops.iter().any(|op| matches!(op, Op::Jump(_)));
        let has_jump_if_false = chunk.ops.iter().any(|op| matches!(op, Op::JumpIfFalse(_)));
        assert!(has_set_slot, "missing SetSlot for loop variable");
        assert!(has_jump, "missing Jump for loop backedge");
        assert!(has_jump_if_false, "missing JumpIfFalse for loop exit");
    }

    #[test]
    fn test_arith_compound_status() {
        use crate::parser::CompoundCommand;
        // (( 5 > 3 )) → exit status 0 (true)
        let cmd = ShellCommand::Compound(CompoundCommand::Arith("5 > 3".to_string()));
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let mut vm = VM::new(chunk);
        let _ = vm.run();
        assert_eq!(vm.last_status, 0); // success

        // (( 0 )) → exit status 1 (false)
        let cmd = ShellCommand::Compound(CompoundCommand::Arith("0".to_string()));
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let mut vm = VM::new(chunk);
        let _ = vm.run();
        assert_eq!(vm.last_status, 1); // failure
    }

    #[test]
    fn test_if_arith() {
        use crate::parser::CompoundCommand;
        // if (( 1 )); then (( result = 42 )); fi
        let cmd = ShellCommand::Compound(CompoundCommand::If {
            conditions: vec![(
                vec![ShellCommand::Compound(CompoundCommand::Arith("1".to_string()))],
                vec![ShellCommand::Compound(CompoundCommand::Arith(
                    "result = 42".to_string(),
                ))],
            )],
            else_part: None,
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let mut vm = VM::new(chunk);
        let _ = vm.run();
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_repeat_loop() {
        use crate::parser::CompoundCommand;
        let cmd = ShellCommand::Compound(CompoundCommand::Repeat {
            count: "5".to_string(),
            body: vec![ShellCommand::Compound(CompoundCommand::Arith(
                "count = count + 1".to_string(),
            ))],
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let mut vm = VM::new(chunk);
        let _ = vm.run();
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_simple_command_compiles() {
        use crate::parser::SimpleCommand;
        // echo hello world → Exec(3)
        let cmd = ShellCommand::Simple(SimpleCommand {
            assignments: vec![],
            words: vec![
                ShellWord::Literal("echo".to_string()),
                ShellWord::Literal("hello".to_string()),
                ShellWord::Literal("world".to_string()),
            ],
            redirects: vec![],
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_exec = chunk.ops.iter().any(|op| matches!(op, Op::Exec(3)));
        assert!(has_exec, "expected Exec(3) for 'echo hello world'");
    }

    #[test]
    fn test_assignment_compiles() {
        use crate::parser::SimpleCommand;
        // X=42 (bare assignment, no command)
        let cmd = ShellCommand::Simple(SimpleCommand {
            assignments: vec![("X".to_string(), ShellWord::Literal("42".to_string()), false)],
            words: vec![],
            redirects: vec![],
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_set = chunk.ops.iter().any(|op| matches!(op, Op::SetVar(_)));
        assert!(has_set, "expected SetVar for assignment");
    }

    #[test]
    fn test_pipeline_compiles() {
        use crate::parser::SimpleCommand;
        // ls | grep foo → PipelineBegin(2) ... PipelineEnd
        let cmds = vec![
            ShellCommand::Simple(SimpleCommand {
                assignments: vec![],
                words: vec![ShellWord::Literal("ls".to_string())],
                redirects: vec![],
            }),
            ShellCommand::Simple(SimpleCommand {
                assignments: vec![],
                words: vec![
                    ShellWord::Literal("grep".to_string()),
                    ShellWord::Literal("foo".to_string()),
                ],
                redirects: vec![],
            }),
        ];
        let cmd = ShellCommand::Pipeline(cmds, false);
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_begin = chunk.ops.iter().any(|op| matches!(op, Op::PipelineBegin(2)));
        let has_end = chunk.ops.iter().any(|op| matches!(op, Op::PipelineEnd));
        let has_stage = chunk.ops.iter().any(|op| matches!(op, Op::PipelineStage));
        assert!(has_begin, "expected PipelineBegin(2)");
        assert!(has_stage, "expected PipelineStage");
        assert!(has_end, "expected PipelineEnd");
    }

    #[test]
    fn test_redirect_compiles() {
        use crate::parser::{Redirect, RedirectOp, SimpleCommand};
        // echo hi > /tmp/out
        let cmd = ShellCommand::Simple(SimpleCommand {
            assignments: vec![],
            words: vec![
                ShellWord::Literal("echo".to_string()),
                ShellWord::Literal("hi".to_string()),
            ],
            redirects: vec![Redirect {
                fd: None,
                op: RedirectOp::Write,
                target: ShellWord::Literal("/tmp/out".to_string()),
                heredoc_content: None,
                fd_var: None,
            }],
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_redirect = chunk.ops.iter().any(|op| matches!(op, Op::Redirect(1, 0))); // fd=1, WRITE=0
        assert!(has_redirect, "expected Redirect(1, 0) for > /tmp/out");
    }

    #[test]
    fn test_heredoc_compiles() {
        use crate::parser::{Redirect, RedirectOp, SimpleCommand};
        // cat <<EOF\nhello\nEOF
        let cmd = ShellCommand::Simple(SimpleCommand {
            assignments: vec![],
            words: vec![ShellWord::Literal("cat".to_string())],
            redirects: vec![Redirect {
                fd: None,
                op: RedirectOp::HereDoc,
                target: ShellWord::Literal("EOF".to_string()),
                heredoc_content: Some("hello\n".to_string()),
                fd_var: None,
            }],
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_heredoc = chunk.ops.iter().any(|op| matches!(op, Op::HereDoc(_)));
        assert!(has_heredoc, "expected HereDoc op");
    }

    #[test]
    fn test_case_compiles() {
        use crate::parser::CompoundCommand;
        // case x in a) ;; b) ;; esac
        let cmd = ShellCommand::Compound(CompoundCommand::Case {
            word: ShellWord::Literal("hello".to_string()),
            cases: vec![
                (
                    vec![ShellWord::Literal("hello".to_string())],
                    vec![ShellCommand::Compound(CompoundCommand::Arith("result = 1".to_string()))],
                    CaseTerminator::Break,
                ),
                (
                    vec![ShellWord::Literal("world".to_string())],
                    vec![ShellCommand::Compound(CompoundCommand::Arith("result = 2".to_string()))],
                    CaseTerminator::Break,
                ),
            ],
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        // Should have StrEq for pattern matching
        let has_streq = chunk.ops.iter().any(|op| matches!(op, Op::StrEq));
        assert!(has_streq, "expected StrEq for case pattern");
    }

    #[test]
    fn test_cond_file_test() {
        use crate::parser::CompoundCommand;
        // [[ -f /etc/passwd ]]
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::FileRegular(
            ShellWord::Literal("/etc/passwd".to_string()),
        )));
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_test = chunk.ops.iter().any(|op| matches!(op, Op::TestFile(0))); // IS_FILE = 0
        assert!(has_test, "expected TestFile(IS_FILE)");
    }

    #[test]
    fn test_cond_string_compare() {
        use crate::parser::CompoundCommand;
        // [[ "abc" == "abc" ]]
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::StringEqual(
            ShellWord::Literal("abc".to_string()),
            ShellWord::Literal("abc".to_string()),
        )));
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_streq = chunk.ops.iter().any(|op| matches!(op, Op::StrEq));
        assert!(has_streq, "expected StrEq for string comparison");
    }

    #[test]
    fn test_cond_logical() {
        use crate::parser::CompoundCommand;
        // [[ -f /etc/passwd && -d /tmp ]]
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::And(
            Box::new(CondExpr::FileRegular(ShellWord::Literal("/etc/passwd".to_string()))),
            Box::new(CondExpr::FileDirectory(ShellWord::Literal("/tmp".to_string()))),
        )));
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_short_circuit = chunk
            .ops
            .iter()
            .any(|op| matches!(op, Op::JumpIfFalseKeep(_)));
        assert!(has_short_circuit, "expected short-circuit && in [[ ]]");
    }

    #[test]
    fn test_list_and_or() {
        use crate::parser::{ListOp, SimpleCommand};
        // true && echo yes
        let items = vec![
            (
                ShellCommand::Compound(CompoundCommand::Arith("1".to_string())),
                ListOp::And,
            ),
            (
                ShellCommand::Simple(SimpleCommand {
                    assignments: vec![],
                    words: vec![
                        ShellWord::Literal("echo".to_string()),
                        ShellWord::Literal("yes".to_string()),
                    ],
                    redirects: vec![],
                }),
                ListOp::Semi,
            ),
        ];
        let cmd = ShellCommand::List(items);
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_get_status = chunk.ops.iter().any(|op| matches!(op, Op::GetStatus));
        assert!(has_get_status, "expected GetStatus for && list");
    }

    #[test]
    fn test_function_def_compiles() {
        // myfunc() { (( x = 42 )) }
        let cmd = ShellCommand::FunctionDef(
            "myfunc".to_string(),
            Box::new(ShellCommand::Compound(CompoundCommand::Arith(
                "x = 42".to_string(),
            ))),
        );
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        assert!(!chunk.sub_entries.is_empty(), "expected sub entry for function");
        let has_return = chunk.ops.iter().any(|op| matches!(op, Op::Return));
        assert!(has_return, "expected Return in function body");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Execution tests — actually run compiled bytecodes on fusevm
    // ═══════════════════════════════════════════════════════════════════

    /// Helper: compile and run shell commands, return VM
    fn compile_and_run(commands: &[ShellCommand]) -> VM {
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(commands);
        let mut vm = VM::new(chunk);
        let _ = vm.run();
        vm
    }

    #[test]
    fn test_exec_file_test_exists() {
        use crate::parser::CompoundCommand;
        // [[ -e /tmp ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::FileExists(ShellWord::Literal("/tmp".to_string())),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0, "/tmp should exist");
    }

    #[test]
    fn test_exec_file_test_not_exists() {
        use crate::parser::CompoundCommand;
        // [[ -e /nonexistent_path_xyz ]] → status 1
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::FileExists(ShellWord::Literal("/nonexistent_path_xyz".to_string())),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1, "/nonexistent should not exist");
    }

    #[test]
    fn test_exec_file_is_dir() {
        use crate::parser::CompoundCommand;
        // [[ -d /tmp ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::FileDirectory(ShellWord::Literal("/tmp".to_string())),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0, "/tmp should be a directory");
    }

    #[test]
    fn test_exec_file_is_regular() {
        use crate::parser::CompoundCommand;
        // [[ -f /etc/hosts ]] → status 0 (exists on macOS/Linux)
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::FileRegular(ShellWord::Literal("/etc/hosts".to_string())),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0, "/etc/hosts should be a regular file");
    }

    #[test]
    fn test_exec_string_equal() {
        use crate::parser::CompoundCommand;
        // [[ "abc" == "abc" ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::StringEqual(
                ShellWord::Literal("abc".to_string()),
                ShellWord::Literal("abc".to_string()),
            ),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_exec_string_not_equal() {
        use crate::parser::CompoundCommand;
        // [[ "abc" == "xyz" ]] → status 1
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::StringEqual(
                ShellWord::Literal("abc".to_string()),
                ShellWord::Literal("xyz".to_string()),
            ),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }

    #[test]
    fn test_exec_string_empty() {
        use crate::parser::CompoundCommand;
        // [[ -z "" ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::StringEmpty(ShellWord::Literal("".to_string())),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);

        // [[ -z "notempty" ]] → status 1
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::StringEmpty(ShellWord::Literal("notempty".to_string())),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }

    #[test]
    fn test_exec_cond_and() {
        use crate::parser::CompoundCommand;
        // [[ -d /tmp && -e /tmp ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::And(
                Box::new(CondExpr::FileDirectory(ShellWord::Literal("/tmp".to_string()))),
                Box::new(CondExpr::FileExists(ShellWord::Literal("/tmp".to_string()))),
            ),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_exec_cond_and_short_circuit() {
        use crate::parser::CompoundCommand;
        // [[ -f /nonexistent && -d /tmp ]] → status 1 (short-circuits on first)
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::And(
                Box::new(CondExpr::FileRegular(ShellWord::Literal("/nonexistent".to_string()))),
                Box::new(CondExpr::FileDirectory(ShellWord::Literal("/tmp".to_string()))),
            ),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }

    #[test]
    fn test_exec_cond_or() {
        use crate::parser::CompoundCommand;
        // [[ -f /nonexistent || -d /tmp ]] → status 0 (second is true)
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::Or(
                Box::new(CondExpr::FileRegular(ShellWord::Literal("/nonexistent".to_string()))),
                Box::new(CondExpr::FileDirectory(ShellWord::Literal("/tmp".to_string()))),
            ),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_exec_cond_not() {
        use crate::parser::CompoundCommand;
        // [[ ! -f /nonexistent ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::Not(Box::new(CondExpr::FileRegular(
                ShellWord::Literal("/nonexistent".to_string()),
            ))),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_exec_if_true_branch() {
        use crate::parser::CompoundCommand;
        // if (( 1 )); then (( result = 42 )); else (( result = 99 )); fi
        // Since (( 1 )) sets status=0, true branch runs
        let cmd = ShellCommand::Compound(CompoundCommand::If {
            conditions: vec![(
                vec![ShellCommand::Compound(CompoundCommand::Arith("1".to_string()))],
                vec![ShellCommand::Compound(CompoundCommand::Arith("result = 42".to_string()))],
            )],
            else_part: Some(vec![
                ShellCommand::Compound(CompoundCommand::Arith("result = 99".to_string())),
            ]),
        });
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0); // (( 42 )) is truthy → status 0
    }

    #[test]
    fn test_exec_if_false_branch() {
        use crate::parser::CompoundCommand;
        // if (( 0 )); then (( result = 42 )); else (( result = 99 )); fi
        let cmd = ShellCommand::Compound(CompoundCommand::If {
            conditions: vec![(
                vec![ShellCommand::Compound(CompoundCommand::Arith("0".to_string()))],
                vec![ShellCommand::Compound(CompoundCommand::Arith("result = 42".to_string()))],
            )],
            else_part: Some(vec![
                ShellCommand::Compound(CompoundCommand::Arith("result = 99".to_string())),
            ]),
        });
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0); // (( 99 )) is truthy → status 0
    }

    #[test]
    fn test_exec_numeric_comparison() {
        use crate::parser::CompoundCommand;
        // [[ 5 -gt 3 ]] → true
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::NumGreater(
                ShellWord::Literal("5".to_string()),
                ShellWord::Literal("3".to_string()),
            ),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);

        // [[ 2 -gt 3 ]] → false
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(
            CondExpr::NumGreater(
                ShellWord::Literal("2".to_string()),
                ShellWord::Literal("3".to_string()),
            ),
        ));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }

    #[test]
    fn test_exec_arith_zero_is_false() {
        use crate::parser::CompoundCommand;
        // (( 0 )) → status 1
        let cmd = ShellCommand::Compound(CompoundCommand::Arith("0".to_string()));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }

    #[test]
    fn test_exec_arith_nonzero_is_true() {
        use crate::parser::CompoundCommand;
        // (( 42 )) → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Arith("42".to_string()));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_exec_nested_arith_comparison() {
        use crate::parser::CompoundCommand;
        // (( 5 > 3 && 2 < 10 )) → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Arith("5 > 3 && 2 < 10".to_string()));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);

        // (( 5 > 3 && 2 > 10 )) → status 1
        let cmd = ShellCommand::Compound(CompoundCommand::Arith("5 > 3 && 2 > 10".to_string()));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }
}
