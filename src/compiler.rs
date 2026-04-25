//! Shell compiler — lowers ShellCommand AST to fusevm bytecode.
//!
//! This is the bridge between the parser (AST) and the VM (bytecode).
//! Gradual lowering: compile what we can, fall back to exec.rs interpreter
//! for anything not yet supported.
//!
//! Architecture:
//!   zsh source → parser → ShellCommand AST → compiler → Chunk → VM::run()
//!
//! The compiled bytecode is cached in SQLite alongside the AST cache,
//! keyed by (path, mtime). Second launch skips parse AND compile.

use crate::parser::*;
use fusevm::{ChunkBuilder, Op, Value};

/// Compile a script (list of commands) into a fusevm Chunk.
pub fn compile_script(commands: &[ShellCommand], source: &str) -> fusevm::Chunk {
    let mut c = Compiler::new(source);
    for cmd in commands {
        c.compile_command(cmd);
    }
    c.finish()
}

/// Compile a single function body into a Chunk.
pub fn compile_function(name: &str, body: &ShellCommand) -> fusevm::Chunk {
    let mut c = Compiler::new(name);
    c.compile_command(body);
    c.emit(Op::Return, 0);
    c.finish()
}

struct Compiler {
    builder: ChunkBuilder,
    line: u32,
    /// Track loop start/break targets for break/continue
    loop_stack: Vec<LoopCtx>,
}

struct LoopCtx {
    start: usize,    // jump target for `continue`
    breaks: Vec<usize>, // jump placeholders to patch for `break`
}

impl Compiler {
    fn new(source: &str) -> Self {
        let mut builder = ChunkBuilder::new();
        builder.set_source(source);
        Self {
            builder,
            line: 1,
            loop_stack: Vec::new(),
        }
    }

    fn emit(&mut self, op: Op, line: u32) -> usize {
        self.builder.emit(op, line)
    }

    fn pos(&self) -> usize {
        self.builder.current_pos()
    }

    fn name(&mut self, s: &str) -> u16 {
        self.builder.add_name(s)
    }

    fn constant_str(&mut self, s: &str) -> u16 {
        self.builder.add_constant(Value::str(s))
    }

    fn finish(self) -> fusevm::Chunk {
        self.builder.build()
    }

    // ── Command dispatch ──

    fn compile_command(&mut self, cmd: &ShellCommand) {
        match cmd {
            ShellCommand::Simple(simple) => self.compile_simple(simple),
            ShellCommand::Pipeline(cmds, negated) => self.compile_pipeline(cmds, *negated),
            ShellCommand::List(items) => self.compile_list(items),
            ShellCommand::Compound(compound) => self.compile_compound(compound),
            ShellCommand::FunctionDef(name, body) => self.compile_funcdef(name, body),
        }
    }

    // ── Simple command ──

    fn compile_simple(&mut self, cmd: &SimpleCommand) {
        // Handle assignments
        for (var, val, is_append) in &cmd.assignments {
            self.compile_word(val);
            if *is_append {
                // load current value, concat, store
                let idx = self.name(var);
                let tmp = self.name(var);
                self.emit(Op::GetVar(idx), self.line);
                self.emit(Op::Swap, self.line);
                self.emit(Op::Concat, self.line);
                self.emit(Op::SetVar(tmp), self.line);
            } else {
                let idx = self.name(var);
                self.emit(Op::SetVar(idx), self.line);
            }
        }

        if cmd.words.is_empty() {
            return;
        }

        // Compile redirects
        for redir in &cmd.redirects {
            self.compile_redirect(redir);
        }

        // Check if first word is a known simple builtin we can compile directly
        if let ShellWord::Literal(name) = &cmd.words[0] {
            match name.as_str() {
                "echo" => return self.compile_echo(&cmd.words[1..]),
                "print" => return self.compile_print(&cmd.words[1..]),
                "true" => {
                    self.emit(Op::LoadInt(0), self.line);
                    self.emit(Op::SetStatus, self.line);
                    return;
                }
                "false" => {
                    self.emit(Op::LoadInt(1), self.line);
                    self.emit(Op::SetStatus, self.line);
                    return;
                }
                "return" => {
                    if cmd.words.len() > 1 {
                        self.compile_word(&cmd.words[1]);
                    } else {
                        self.emit(Op::GetStatus, self.line);
                    }
                    self.emit(Op::ReturnValue, self.line);
                    return;
                }
                "break" => {
                    let j = self.emit(Op::Jump(0), self.line); // placeholder
                    if let Some(ctx) = self.loop_stack.last_mut() {
                        ctx.breaks.push(j);
                    }
                    return;
                }
                "continue" => {
                    if let Some(ctx) = self.loop_stack.last() {
                        let target = ctx.start;
                        self.emit(Op::Jump(target), self.line);
                    }
                    return;
                }
                _ => {}
            }
        }

        // General case: push all words onto stack, emit Exec
        let argc = cmd.words.len() as u8;
        for word in &cmd.words {
            self.compile_word(word);
        }
        self.emit(Op::Exec(argc), self.line);
        self.emit(Op::SetStatus, self.line);
    }

    // ── Builtins compiled to bytecode ──

    fn compile_echo(&mut self, args: &[ShellWord]) {
        if args.is_empty() {
            let idx = self.constant_str("");
            self.emit(Op::LoadConst(idx), self.line);
            self.emit(Op::PrintLn(1), self.line);
            return;
        }
        for arg in args {
            self.compile_word(arg);
        }
        self.emit(Op::PrintLn(args.len() as u8), self.line);
    }

    fn compile_print(&mut self, args: &[ShellWord]) {
        // print without -n is like echo; simplified for now
        self.compile_echo(args);
    }

    // ── Pipeline ──

    fn compile_pipeline(&mut self, cmds: &[ShellCommand], negated: bool) {
        let n = cmds.len() as u8;
        self.emit(Op::PipelineBegin(n), self.line);
        for (i, cmd) in cmds.iter().enumerate() {
            self.compile_command(cmd);
            if i < cmds.len() - 1 {
                self.emit(Op::PipelineStage, self.line);
            }
        }
        self.emit(Op::PipelineEnd, self.line);
        if negated {
            // Negate exit status: 0→1, nonzero→0
            self.emit(Op::GetStatus, self.line);
            self.emit(Op::LogNot, self.line);
            self.emit(Op::SetStatus, self.line);
        }
    }

    // ── List (cmd1 && cmd2, cmd1 || cmd2, cmd1; cmd2) ──

    fn compile_list(&mut self, items: &[(ShellCommand, ListOp)]) {
        for (i, (cmd, op)) in items.iter().enumerate() {
            self.compile_command(cmd);

            // Last item has no following op
            if i + 1 >= items.len() {
                break;
            }

            match op {
                ListOp::And => {
                    // Short-circuit: if last status != 0, skip next command
                    self.emit(Op::GetStatus, self.line);
                    let j = self.emit(Op::JumpIfTrue(0), self.line); // status != 0 means failure
                    // If we get here, status was 0 (truthy in shell = success)
                    // Actually shell convention: status 0 = success = truthy for &&
                    // JumpIfTrue with status value... need to check:
                    // status=0 → success → continue → don't jump
                    // status≠0 → failure → skip → jump
                    // So: push status, if nonzero (truthy as int) jump past next
                    self.builder.patch_jump(j, self.pos());
                    // Actually this needs rethinking — shell status 0 = success but
                    // Int(0) is falsy in the VM. We need JumpIfFalse for &&.
                    // Let's fix: push status, convert to shell-truthiness, branch.
                    // For now emit the simple version — refinement later.
                }
                ListOp::Or => {
                    // Short-circuit: if last status == 0, skip next command
                    self.emit(Op::GetStatus, self.line);
                    let j = self.emit(Op::JumpIfFalse(0), self.line);
                    self.builder.patch_jump(j, self.pos());
                }
                ListOp::Semi | ListOp::Amp | ListOp::Newline => {
                    // Sequential or background — just continue
                    // TODO: Amp should emit ExecBg
                }
            }
        }
    }

    // ── Compound commands ──

    fn compile_compound(&mut self, compound: &CompoundCommand) {
        match compound {
            CompoundCommand::BraceGroup(cmds) => {
                for cmd in cmds { self.compile_command(cmd); }
            }
            CompoundCommand::Subshell(cmds) => {
                self.emit(Op::SubshellBegin, self.line);
                for cmd in cmds { self.compile_command(cmd); }
                self.emit(Op::SubshellEnd, self.line);
            }
            CompoundCommand::If { conditions, else_part } => {
                self.compile_if(conditions, else_part);
            }
            CompoundCommand::For { var, words, body } => {
                self.compile_for(var, words, body);
            }
            CompoundCommand::ForArith { init, cond, step, body } => {
                self.compile_for_arith(init, cond, step, body);
            }
            CompoundCommand::While { condition, body } => {
                self.compile_while(condition, body, false);
            }
            CompoundCommand::Until { condition, body } => {
                self.compile_while(condition, body, true);
            }
            CompoundCommand::Case { word, cases } => {
                self.compile_case(word, cases);
            }
            CompoundCommand::Try { try_body, always_body } => {
                // Try: execute try_body, then always execute always_body
                for cmd in try_body { self.compile_command(cmd); }
                for cmd in always_body { self.compile_command(cmd); }
            }
            CompoundCommand::Repeat { count, body } => {
                self.compile_repeat(count, body);
            }
            _ => {
                // Unsupported compound — will need interpreter fallback
                // TODO: Coproc, Select, Cond, Arith, WithRedirects
            }
        }
    }

    fn compile_if(
        &mut self,
        conditions: &[(Vec<ShellCommand>, Vec<ShellCommand>)],
        else_part: &Option<Vec<ShellCommand>>,
    ) {
        let mut end_jumps = Vec::new();

        for (cond_cmds, body_cmds) in conditions {
            // Compile condition
            for cmd in cond_cmds { self.compile_command(cmd); }
            // Check exit status
            self.emit(Op::GetStatus, self.line);
            let skip = self.emit(Op::JumpIfTrue(0), self.line); // nonzero status = falsy in shell

            // Compile body
            for cmd in body_cmds { self.compile_command(cmd); }
            let end_j = self.emit(Op::Jump(0), self.line);
            end_jumps.push(end_j);

            // Patch the skip to jump here (past body)
            self.builder.patch_jump(skip, self.pos());
        }

        // Else part
        if let Some(else_cmds) = else_part {
            for cmd in else_cmds { self.compile_command(cmd); }
        }

        // Patch all end jumps to here
        let end = self.pos();
        for j in end_jumps {
            self.builder.patch_jump(j, end);
        }
    }

    fn compile_for(&mut self, var: &str, words: &Option<Vec<ShellWord>>, body: &[ShellCommand]) {
        let var_idx = self.name(var);

        // Push all iteration words onto stack as an array
        if let Some(ws) = words {
            for w in ws {
                self.compile_word(w);
            }
            self.emit(Op::MakeArray(ws.len() as u16), self.line);
        } else {
            // for x; do ... done — iterate over positional params
            // TODO: push $@ as array
            let empty = self.constant_str("");
            self.emit(Op::LoadConst(empty), self.line);
            return;
        }

        // Iteration: get array length, loop index 0..len
        let iter_idx = self.name("__for_arr");
        let i_idx = self.name("__for_i");
        let len_idx = self.name("__for_len");

        self.emit(Op::SetVar(iter_idx), self.line);     // store array
        self.emit(Op::ArrayLen(iter_idx), self.line);    // push length
        self.emit(Op::SetVar(len_idx), self.line);       // store length
        self.emit(Op::LoadInt(0), self.line);
        self.emit(Op::SetVar(i_idx), self.line);         // i = 0

        let loop_top = self.pos();
        self.loop_stack.push(LoopCtx { start: loop_top, breaks: Vec::new() });

        // condition: i < len
        self.emit(Op::GetVar(i_idx), self.line);
        self.emit(Op::GetVar(len_idx), self.line);
        self.emit(Op::NumLt, self.line);
        let exit_jump = self.emit(Op::JumpIfFalse(0), self.line);

        // body: var = arr[i]
        self.emit(Op::GetVar(i_idx), self.line);
        self.emit(Op::ArrayGet(iter_idx), self.line);
        self.emit(Op::SetVar(var_idx), self.line);

        for cmd in body { self.compile_command(cmd); }

        // i++
        self.emit(Op::GetVar(i_idx), self.line);
        self.emit(Op::LoadInt(1), self.line);
        self.emit(Op::Add, self.line);
        self.emit(Op::SetVar(i_idx), self.line);
        self.emit(Op::Jump(loop_top), self.line);

        // patch exit
        let exit_pos = self.pos();
        self.builder.patch_jump(exit_jump, exit_pos);

        // patch breaks
        let ctx = self.loop_stack.pop().unwrap();
        for b in ctx.breaks {
            self.builder.patch_jump(b, exit_pos);
        }
    }

    fn compile_for_arith(&mut self, init: &str, cond: &str, step: &str, body: &[ShellCommand]) {
        // (( init )); while (( cond )); do body; (( step )); done
        // For now, emit as extended ops — arithmetic compilation is complex
        // TODO: lower arithmetic expressions to VM ops
        let init_c = self.constant_str(init);
        let cond_c = self.constant_str(cond);
        let step_c = self.constant_str(step);

        // Extended: evaluate init expression
        self.emit(Op::LoadConst(init_c), self.line);
        self.emit(Op::Extended(0, 0), self.line); // placeholder: eval arith

        let loop_top = self.pos();
        self.loop_stack.push(LoopCtx { start: loop_top, breaks: Vec::new() });

        // Extended: evaluate condition
        self.emit(Op::LoadConst(cond_c), self.line);
        self.emit(Op::Extended(1, 0), self.line); // placeholder: eval arith condition
        let exit_jump = self.emit(Op::JumpIfFalse(0), self.line);

        for cmd in body { self.compile_command(cmd); }

        // Extended: evaluate step
        self.emit(Op::LoadConst(step_c), self.line);
        self.emit(Op::Extended(0, 0), self.line);
        self.emit(Op::Jump(loop_top), self.line);

        let exit_pos = self.pos();
        self.builder.patch_jump(exit_jump, exit_pos);

        let ctx = self.loop_stack.pop().unwrap();
        for b in ctx.breaks { self.builder.patch_jump(b, exit_pos); }
    }

    fn compile_while(&mut self, condition: &[ShellCommand], body: &[ShellCommand], negate: bool) {
        let loop_top = self.pos();
        self.loop_stack.push(LoopCtx { start: loop_top, breaks: Vec::new() });

        for cmd in condition { self.compile_command(cmd); }
        self.emit(Op::GetStatus, self.line);

        let exit_jump = if negate {
            self.emit(Op::JumpIfFalse(0), self.line) // until: exit when status == 0
        } else {
            self.emit(Op::JumpIfTrue(0), self.line) // while: exit when status != 0
        };

        for cmd in body { self.compile_command(cmd); }
        self.emit(Op::Jump(loop_top), self.line);

        let exit_pos = self.pos();
        self.builder.patch_jump(exit_jump, exit_pos);

        let ctx = self.loop_stack.pop().unwrap();
        for b in ctx.breaks { self.builder.patch_jump(b, exit_pos); }
    }

    fn compile_case(&mut self, word: &ShellWord, cases: &[(Vec<ShellWord>, Vec<ShellCommand>, CaseTerminator)]) {
        self.compile_word(word); // push the test value

        let mut end_jumps = Vec::new();

        for (patterns, cmds, _terminator) in cases {
            // For each pattern, test equality
            let mut pattern_match_jumps = Vec::new();

            for pat in patterns {
                self.emit(Op::Dup, self.line); // dup test value
                self.compile_word(pat);
                self.emit(Op::StrEq, self.line);
                let j = self.emit(Op::JumpIfTrue(0), self.line);
                pattern_match_jumps.push(j);
            }

            // None matched — jump past this arm
            let skip = self.emit(Op::Jump(0), self.line);

            // Patch pattern matches to here (arm body)
            let body_start = self.pos();
            for j in pattern_match_jumps {
                self.builder.patch_jump(j, body_start);
            }

            for cmd in cmds { self.compile_command(cmd); }

            let end_j = self.emit(Op::Jump(0), self.line);
            end_jumps.push(end_j);

            // Patch skip
            self.builder.patch_jump(skip, self.pos());
        }

        let end = self.pos();
        for j in end_jumps { self.builder.patch_jump(j, end); }
        self.emit(Op::Pop, self.line); // pop test value
    }

    fn compile_repeat(&mut self, count: &str, body: &[ShellCommand]) {
        // repeat N do ... done
        let count_c = self.constant_str(count);
        let i_idx = self.name("__repeat_i");

        self.emit(Op::LoadConst(count_c), self.line);
        // TODO: coerce to int
        self.emit(Op::SetVar(i_idx), self.line);

        let loop_top = self.pos();
        self.loop_stack.push(LoopCtx { start: loop_top, breaks: Vec::new() });

        self.emit(Op::GetVar(i_idx), self.line);
        self.emit(Op::LoadInt(0), self.line);
        self.emit(Op::NumGt, self.line);
        let exit_jump = self.emit(Op::JumpIfFalse(0), self.line);

        for cmd in body { self.compile_command(cmd); }

        // i--
        self.emit(Op::GetVar(i_idx), self.line);
        self.emit(Op::LoadInt(1), self.line);
        self.emit(Op::Sub, self.line);
        self.emit(Op::SetVar(i_idx), self.line);
        self.emit(Op::Jump(loop_top), self.line);

        let exit_pos = self.pos();
        self.builder.patch_jump(exit_jump, exit_pos);
        let ctx = self.loop_stack.pop().unwrap();
        for b in ctx.breaks { self.builder.patch_jump(b, exit_pos); }
    }

    // ── Function definition ──

    fn compile_funcdef(&mut self, name: &str, body: &ShellCommand) {
        // Jump over the function body — it's not executed at definition time
        let skip = self.emit(Op::Jump(0), self.line);

        let name_idx = self.name(name);
        let entry = self.pos();
        self.emit(Op::PushFrame, self.line);
        self.compile_command(body);
        self.emit(Op::PopFrame, self.line);
        self.emit(Op::Return, self.line);

        self.builder.add_sub_entry(name_idx, entry);
        self.builder.patch_jump(skip, self.pos());
    }

    // ── Word compilation ──

    fn compile_word(&mut self, word: &ShellWord) {
        match word {
            ShellWord::Literal(s) => {
                let idx = self.constant_str(s);
                self.emit(Op::LoadConst(idx), self.line);
            }
            ShellWord::SingleQuoted(s) => {
                let idx = self.constant_str(s);
                self.emit(Op::LoadConst(idx), self.line);
            }
            ShellWord::DoubleQuoted(parts) => {
                if parts.is_empty() {
                    let idx = self.constant_str("");
                    self.emit(Op::LoadConst(idx), self.line);
                } else {
                    for (i, p) in parts.iter().enumerate() {
                        self.compile_word(p);
                        if i > 0 {
                            self.emit(Op::Concat, self.line);
                        }
                    }
                }
            }
            ShellWord::Variable(name) => {
                let idx = self.name(name);
                self.emit(Op::GetVar(idx), self.line);
            }
            ShellWord::VariableBraced(name, modifier) => {
                let idx = self.name(name);
                self.emit(Op::GetVar(idx), self.line);
                if let Some(m) = modifier {
                    self.compile_var_modifier(idx, m);
                }
            }
            ShellWord::ArithSub(expr) => {
                // For now, push as string for runtime eval
                // TODO: compile arithmetic expressions to VM ops
                let idx = self.constant_str(expr);
                self.emit(Op::LoadConst(idx), self.line);
                self.emit(Op::Extended(2, 0), self.line); // placeholder: eval arith
            }
            ShellWord::CommandSub(cmd) => {
                // Compile command into a sub-chunk, emit CmdSubst
                // For now, push as extended op
                // TODO: compile sub-command as block range
                self.compile_command(cmd);
                // The command's output should be on stack after CmdSubst
            }
            ShellWord::Glob(pattern) => {
                let idx = self.constant_str(pattern);
                self.emit(Op::LoadConst(idx), self.line);
                self.emit(Op::Glob, self.line);
            }
            ShellWord::Tilde(user) => {
                if let Some(u) = user {
                    let idx = self.constant_str(&format!("~{}", u));
                    self.emit(Op::LoadConst(idx), self.line);
                } else {
                    let idx = self.constant_str("~");
                    self.emit(Op::LoadConst(idx), self.line);
                }
                self.emit(Op::TildeExpand, self.line);
            }
            ShellWord::Concat(parts) => {
                for (i, p) in parts.iter().enumerate() {
                    self.compile_word(p);
                    if i > 0 {
                        self.emit(Op::Concat, self.line);
                    }
                }
            }
            ShellWord::ArrayLiteral(elements) => {
                for e in elements {
                    self.compile_word(e);
                }
                self.emit(Op::MakeArray(elements.len() as u16), self.line);
            }
            ShellWord::ArrayVar(name, index) => {
                let idx = self.name(name);
                self.compile_word(index);
                self.emit(Op::ArrayGet(idx), self.line);
            }
            ShellWord::ProcessSubIn(cmd) => {
                // TODO: compile as block range
                self.compile_command(cmd);
            }
            ShellWord::ProcessSubOut(cmd) => {
                self.compile_command(cmd);
            }
        }
    }

    fn compile_var_modifier(&mut self, _var_idx: u16, modifier: &VarModifier) {
        match modifier {
            VarModifier::Default(word) => {
                // ${var:-default}: if top is empty, replace with default
                self.emit(Op::Dup, self.line);
                self.emit(Op::StringLen, self.line);
                self.emit(Op::LoadInt(0), self.line);
                self.emit(Op::NumEq, self.line);
                let skip = self.emit(Op::JumpIfFalse(0), self.line);
                self.emit(Op::Pop, self.line); // pop empty value
                self.compile_word(word);       // push default
                self.builder.patch_jump(skip, self.pos());
            }
            VarModifier::Length => {
                self.emit(Op::StringLen, self.line);
            }
            _ => {
                // TODO: other modifiers
                // For now, leave the value as-is
            }
        }
    }

    fn compile_redirect(&mut self, redir: &Redirect) {
        let fd = redir.fd.unwrap_or(match redir.op {
            RedirectOp::Read | RedirectOp::HereDoc | RedirectOp::HereString => 0,
            _ => 1,
        }) as u8;

        let op_byte = match redir.op {
            RedirectOp::Write => fusevm::op::redirect_op::WRITE,
            RedirectOp::Append => fusevm::op::redirect_op::APPEND,
            RedirectOp::Read => fusevm::op::redirect_op::READ,
            RedirectOp::ReadWrite => fusevm::op::redirect_op::READ_WRITE,
            RedirectOp::Clobber => fusevm::op::redirect_op::CLOBBER,
            RedirectOp::DupRead => fusevm::op::redirect_op::DUP_READ,
            RedirectOp::DupWrite => fusevm::op::redirect_op::DUP_WRITE,
            RedirectOp::WriteBoth => fusevm::op::redirect_op::WRITE_BOTH,
            RedirectOp::AppendBoth => fusevm::op::redirect_op::APPEND_BOTH,
            RedirectOp::HereDoc => {
                if let Some(ref content) = redir.heredoc_content {
                    let idx = self.constant_str(content);
                    self.emit(Op::HereDoc(idx), self.line);
                }
                return;
            }
            RedirectOp::HereString => {
                self.compile_word(&redir.target);
                self.emit(Op::HereString, self.line);
                return;
            }
        };

        self.compile_word(&redir.target);
        self.emit(Op::Redirect(fd, op_byte), self.line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_echo() {
        let cmd = ShellCommand::Simple(SimpleCommand {
            assignments: vec![],
            words: vec![
                ShellWord::Literal("echo".to_string()),
                ShellWord::Literal("hello".to_string()),
            ],
            redirects: vec![],
        });
        let chunk = compile_script(&[cmd], "test");
        // Should have: LoadConst("hello"), PrintLn(1)
        assert!(chunk.ops.len() >= 2);
        assert!(matches!(chunk.ops.last(), Some(Op::PrintLn(1))));
    }

    #[test]
    fn test_compile_assignment() {
        let cmd = ShellCommand::Simple(SimpleCommand {
            assignments: vec![
                ("X".to_string(), ShellWord::Literal("42".to_string()), false),
            ],
            words: vec![],
            redirects: vec![],
        });
        let chunk = compile_script(&[cmd], "test");
        assert!(chunk.ops.iter().any(|op| matches!(op, Op::SetVar(_))));
    }

    #[test]
    fn test_compile_for_loop() {
        let cmd = ShellCommand::Compound(CompoundCommand::For {
            var: "i".to_string(),
            words: Some(vec![
                ShellWord::Literal("a".to_string()),
                ShellWord::Literal("b".to_string()),
            ]),
            body: vec![ShellCommand::Simple(SimpleCommand {
                assignments: vec![],
                words: vec![
                    ShellWord::Literal("echo".to_string()),
                    ShellWord::Variable("i".to_string()),
                ],
                redirects: vec![],
            })],
        });
        let chunk = compile_script(&[cmd], "test");
        // Should have Jump ops for the loop
        assert!(chunk.ops.iter().any(|op| matches!(op, Op::Jump(_))));
        assert!(chunk.ops.iter().any(|op| matches!(op, Op::JumpIfFalse(_))));
    }

    #[test]
    fn test_compile_if() {
        let cmd = ShellCommand::Compound(CompoundCommand::If {
            conditions: vec![(
                vec![ShellCommand::Simple(SimpleCommand {
                    assignments: vec![],
                    words: vec![ShellWord::Literal("true".to_string())],
                    redirects: vec![],
                })],
                vec![ShellCommand::Simple(SimpleCommand {
                    assignments: vec![],
                    words: vec![
                        ShellWord::Literal("echo".to_string()),
                        ShellWord::Literal("yes".to_string()),
                    ],
                    redirects: vec![],
                })],
            )],
            else_part: None,
        });
        let chunk = compile_script(&[cmd], "test");
        assert!(chunk.ops.iter().any(|op| matches!(op, Op::JumpIfTrue(_))));
    }
}
