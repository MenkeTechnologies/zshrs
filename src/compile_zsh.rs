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
        // The parser synthesizes a FuncDef for the `name() { body }` shape
        // at parse time (ZshParser::parse_program_until detects the
        // Simple<INPAR><OUTPAR> + Inbrace pattern and emits a FuncDef with
        // body_source captured). No compile-side workaround is needed.
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
        self.compile_pipe(pipes[0]);
        if sublist.flags.not {
            self.emit_negate_status();
        }
        // For each subsequent pipe, emit the connector's skip jump that
        // lands right after that pipe.
        for (i, op) in ops.iter().enumerate() {
            self.builder.emit(Op::GetStatus, 0);
            let skip = match op {
                SublistOp::And => self.builder.emit(Op::JumpIfFalse(0), 0),
                SublistOp::Or => self.builder.emit(Op::JumpIfTrue(0), 0),
            };
            self.compile_pipe(pipes[i + 1]);
            self.builder.patch_jump(skip, self.builder.current_pos());
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
                self.builder.emit(Op::LoadInt(length.unwrap_or(-1)), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_PARAM_SUBSTRING, 3),
                    0,
                );
            }
            ParamModifierKind::Strip { op, pattern } => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                let pat_const = self.builder.add_constant(Value::str(pattern));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                self.builder.emit(Op::LoadInt(*op as i64), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_PARAM_STRIP, 3),
                    0,
                );
            }
            ParamModifierKind::Replace { op, pattern, repl } => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                let pat_const = self.builder.add_constant(Value::str(pattern));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                let repl_const = self.builder.add_constant(Value::str(repl));
                self.builder.emit(Op::LoadConst(repl_const), 0);
                self.builder.emit(Op::LoadInt(*op as i64), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_PARAM_REPLACE, 4),
                    0,
                );
            }
            ParamModifierKind::Length => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_PARAM_LENGTH, 1),
                    0,
                );
            }
            ParamModifierKind::FilterRemoveMatching { pattern } => {
                self.builder.emit(Op::LoadConst(name_const), 0);
                let pat_const = self.builder.add_constant(Value::str(pattern));
                self.builder.emit(Op::LoadConst(pat_const), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_PARAM_FILTER, 2),
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
                sub.builder.emit(
                    Op::Redirect(2, fusevm::op::redirect_op::DUP_WRITE),
                    0,
                );
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
                sub.builder.emit(
                    Op::Redirect(2, fusevm::op::redirect_op::DUP_WRITE),
                    0,
                );
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
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_TIME_SUBLIST, 1),
                        0,
                    );
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

        // ── Redirects on the simple command ─────────────────────────
        // Special case: `exec >file` (or `exec 2>err`, etc.) with NO
        // command body — apply redirects PERMANENTLY to the shell's
        // own fds, no scope-end restoration. zsh: `exec` with only
        // redirects rewires the running shell's fds.
        let bare_exec_redir = simple.words.len() == 1
            && simple.words[0] == "exec"
            && !simple.redirs.is_empty();
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
        // Same logic as shell_compiler.rs::compile_simple but operating
        // on raw &str inputs. We decompose at compile time.
        let first = &simple.words[0];

        // Dynamic command name: first word contains an unquoted expansion
        // (`$cmd`, `$(cmd)`, `*name`, `~/bin/foo`). Route through Op::Exec
        // so the host runtime expands and dispatches via host.exec →
        // host_exec_external → run_intercepts. Without this, `cmd=ls;
        // $cmd` would emit CallFunction(name="$cmd", ...) and fail with
        // `command not found: $cmd`.
        let first_untoked = crate::lexer::untokenize(first);
        let first_is_dynamic = unquoted(&first_untoked, '$')
            || unquoted(&first_untoked, '`')
            || unquoted(&first_untoked, '*')
            || unquoted(&first_untoked, '?')
            || unquoted(&first_untoked, '[')
            || first_untoked.starts_with('~');
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

        if has_redirects {
            self.builder.emit(Op::WithRedirectsEnd, 0);
        }
    }

    /// Translate a ZshRedir → fusevm Redirect/HereDoc/HereString op.
    fn compile_redir(&mut self, redir: &crate::parser::ZshRedir) {
        use crate::parser::RedirType;
        // Default fd: stdin for read-side redirects, stdout for write-side.
        // Matches shell_compiler's rules.
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
                if hd.quoted {
                    // Quoted-terminator form: pass body verbatim.
                    let idx = self.builder
                        .add_constant(Value::str(content_clean.as_str()));
                    self.builder.emit(Op::HereDoc(idx), 0);
                } else {
                    // Unquoted: expand `$var`/`$(cmd)`/`$((expr))` in the
                    // body. Strip the trailing newline before passing
                    // through HereString (which re-appends one) so the
                    // resulting stdin matches the heredoc body byte-for-
                    // byte. Phase 2: route through the text-based
                    // BUILTIN_EXPAND_TEXT (mode 0 = default) instead of
                    // the legacy ShellWord JSON path.
                    let trimmed = content_clean.trim_end_matches('\n').to_string();
                    let text_const = self.builder.add_constant(Value::str(trimmed));
                    self.builder.emit(Op::LoadConst(text_const), 0);
                    self.builder.emit(Op::LoadInt(0), 0); // mode = Default
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_EXPAND_TEXT, 2),
                        0,
                    );
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
            self.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_OPEN_NAMED_FD, 3),
                0,
            );
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
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2),
                        0,
                    );
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
        // would mis-route here (the `$` was escaped). Force the bridge so
        // ShellParser sees the original `"\$..."` form via
        // untokenize_preserve_quotes.
        let has_bnull = s.contains('\u{9f}');

        // Trigger detection on the un-tokenized form.
        let trigger_dollar = unquoted(&untoked, '$') || unquoted(&untoked, '`');
        let trigger_glob = unquoted(&untoked, '*')
            || unquoted(&untoked, '?')
            || unquoted(&untoked, '[');
        let trigger_tilde = untoked.starts_with('~')
            || untoked.contains(":~")
            || untoked.contains("=~");
        // Brace expansion: `{a,b,c}` and `{1..5}` need expansion. Detect
        // matched-brace forms with comma or `..` inside.
        let trigger_brace = looks_like_brace_expansion(&untoked);

        // Process substitution `<(cmd)` / `>(cmd)`. The lexer marks the
        // outer angle bracket with INANG (`\u{94}`) / OUTANG (`\u{95}`)
        // and the parens as INPAR/OUTPAR. After untokenize, the form
        // is `<(...)` / `>(...)`. Compile the inner program as a
        // sub-chunk and emit ProcessSubIn/Out which wires up the
        // FIFO/temp file at runtime.
        if (untoked.starts_with("<(") || untoked.starts_with(">("))
            && untoked.ends_with(')')
        {
            let is_in = untoked.starts_with("<(");
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
            self.builder.emit(Op::LoadConst(idx), 0);
            self.builder
                .emit(Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1), 0);
            return;
        }

        // Fast path: single bare `$NAME` (no braces, no concat, no idx,
        // no modifier). Covers `$x`, `$1`, `$#`, `$?`, `$!`, etc. — the
        // most common case in real scripts. Emits BUILTIN_GET_VAR
        // directly, bypassing the legacy ShellParser bridge.
        if !has_bnull {
            if let Some(name) = bare_var_ref(&untoked) {
                let idx = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1), 0);
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

        // Fast path: `${NAME[@]}` / `${NAME[*]}` — array splice. Emits
        // BUILTIN_ARRAY_ALL which always returns Value::Array, even for
        // empty arrays. (BUILTIN_GET_VAR joins arrays into a string for
        // scalar-context use, which collapses splice semantics.)
        if !has_bnull {
            if let Some(name) = array_splice_ref(&untoked) {
                let idx = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_ALL, 0), 0);
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
        // BUILTIN_PARAM_FLAG with [name, flags] on the stack. Mirrors
        // shell_compiler::try_lower_zsh_flag so behavior is identical
        // between pipelines.
        if !has_bnull {
            if let Some((flags, name)) = parse_zsh_flag(&untoked) {
                let name_const = self.builder.add_constant(Value::str(name));
                self.builder.emit(Op::LoadConst(name_const), 0);
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
                self.emit_param_modifier(&modifier);
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

        // Phase 1 step 3: `$(cmd)` command substitution. Push the
        // command text and call BUILTIN_CMD_SUBST_TEXT which routes
        // through `run_command_substitution` (uses ShellParser + an
        // in-process pipe capture). The ShellParser inner dependency
        // can be migrated to ZshParser as a follow-up — fixing the
        // current path's "$(printf "a\nb")" → "anb" quoting bug
        // independently is the harder problem.
        if !has_bnull {
            let preserved_for_cmdsub = crate::lexer::untokenize_preserve_quotes(s);
            if let Some(inner) = strip_cmd_subst(&preserved_for_cmdsub) {
                let idx = self.builder.add_constant(Value::str(inner));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_CMD_SUBST_TEXT, 1), 0);
                return;
            }
        }

        // Phase 1 step 4: concat. Walk the raw word, split into
        // (literal | expansion) segments, emit each, then fold via N-1
        // Concats. Each Expansion segment recurses through compile_word_str
        // (smaller input — terminates). Each Literal segment emits as a
        // pure-literal LoadConst (after untokenize so embedded META
        // chars resolve to their original ASCII).
        if !has_bnull {
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
        //
        // No more ShellParser → ShellWord → JSON round-trip.
        let preserved = crate::lexer::untokenize_preserve_quotes(s);
        let mode = expand_text_mode(s, &preserved);
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
        let body_compiler = ZshCompiler::new();
        let body_chunk = body_compiler.compile(&f.body);
        let body_bytes = bincode::serialize(&body_chunk).unwrap_or_default();
        let body_str = base64_encode(&body_bytes);
        let source_text = f.body_source.clone().unwrap_or_default();

        for name in &f.names {
            let name_const = self.builder.add_constant(Value::str(name.as_str()));
            self.builder.emit(Op::LoadConst(name_const), 0);
            let body_const = self.builder.add_constant(Value::str(body_str.as_str()));
            self.builder.emit(Op::LoadConst(body_const), 0);
            let source_const =
                self.builder.add_constant(Value::str(source_text.as_str()));
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
            if let Some(name) = f.names.first() {
                let name_idx = self.builder.add_name(name);
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
                self.compile_word_str(arg);
                self.emit_file_test(&op_clean);
            }
            ZshCond::Binary(left, op, right) => {
                let left_clean = crate::lexer::untokenize(left);
                let op_clean = crate::lexer::untokenize(op);
                // The port packs unary file tests as Binary too: `-d /tmp`
                // arrives as Binary("-d", "/tmp", ""). If left starts with
                // `-` and looks like a test flag, treat it as Unary with
                // the path as the argument.
                if left_clean.starts_with('-')
                    && left_clean.len() == 2
                    && right.is_empty()
                {
                    self.compile_word_str(op);
                    self.emit_file_test(&left_clean);
                    return;
                }
                self.compile_word_str(left);
                self.compile_word_str(right);
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
            _ => {
                tracing::debug!(op, "compile_zsh: unknown unary test op");
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
            "-ef" => {
                self.builder
                    .emit(Op::CallBuiltin(crate::exec::BUILTIN_SAME_FILE, 2), 0)
            }
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

    /// Compile arithmetic expression text. Leaves the result on stack as
    /// Value::Int. Inlines the same pre-load → arith ops → post-sync
    /// pattern that `ShellCompiler::compile_arith_inline` uses, but
    /// targets this ZshCompiler's builder + slot table directly so no
    /// `ShellCompiler` instance is constructed.
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
            ac.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                0,
            );
            ac.builder.emit(Op::SetSlot(slot), 0);
        }

        ac.expr();
        let new_slots = ac.slots.clone();
        let new_next = ac.next_slot;
        let chunk = ac.builder.build();

        // Inline ArithCompiler's emitted ops into ours, remapping const
        // indices into our local constant table.
        let mut const_remap: std::collections::HashMap<u16, u16> =
            std::collections::HashMap::new();
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
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_SET_VAR, 2),
                    0,
                );
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
            ..raw.char_indices().rev().nth(0).map(|(i, _)| i).unwrap_or(raw.len())];
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
            '\u{8f}' => brace_depth += 1, // INBRACE
            '\u{90}' => brace_depth = (brace_depth - 1).max(0), // OUTBRACE
            '\u{91}' => brack_depth += 1, // INBRACK
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
        // Special single-char params: $@ $* $# $? $! $- $_ $$
        Some(ch) if matches!(ch, '@' | '*' | '#' | '?' | '!' | '-' | '_' | '$') => {
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
        // Identifier: $NAME
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].is_ascii_alphanumeric() || chars[j] == '_')
            {
                j += 1;
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
    // Reject if inner contains an unbalanced `((` or `))` indicating
    // a more complex shape.
    let depth = inner.chars().fold(0i32, |d, c| match c {
        '(' => d + 1,
        ')' => d - 1,
        _ => d,
    });
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
    Some(&s[2..s.len() - 1])
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
    /// `${var#pat}` (op=0), `##` (1), `%` (2), `%%` (3)
    Strip { op: u8, pattern: String },
    /// `${var/pat/repl}` (op=0), `//` (1), `/#` (2), `/%` (3)
    Replace { op: u8, pattern: String, repl: String },
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
    // Reject nested expansions and flag forms — those are handled by
    // earlier fast-paths or the bridge.
    if inner.starts_with('(') || inner.contains("${") {
        return None;
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
        // The body must be a plain identifier — anything else is
        // ambiguous (e.g. `${#}` is `$#` itself, `${#*}` is positional
        // count). Route those through the bridge.
        if !rest.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false) {
            return None;
        }
        if !rest.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
            return None;
        }
        return Some(ParamModifier {
            name: rest.to_string(),
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
    let rest = &inner[name_end..];
    if rest.is_empty() {
        // No modifier — caller's `braced_var_ref` path should have caught
        // this already; treat as not-our-shape so we don't double-emit.
        return None;
    }

    // `${var:-…}` / `${var:=…}` / `${var:?…}` / `${var:+…}`
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
    // with a digit, `-`, or single space (zsh's negative-offset
    // disambiguator).
    if rest.starts_with(':') {
        let after = &rest[1..];
        let trimmed = after.trim_start_matches(' ');
        let first_ch = trimmed.chars().next();
        if matches!(first_ch, Some(c) if c.is_ascii_digit() || c == '-') {
            // Split on the next `:` (length separator)
            let mut iter = trimmed.splitn(2, ':');
            let off_str = iter.next()?.trim();
            let len_str = iter.next();
            let offset: i64 = off_str.parse().ok()?;
            let length: Option<i64> = len_str.and_then(|s| s.trim().parse().ok());
            return Some(ParamModifier {
                name,
                kind: ParamModifierKind::Substring { offset, length },
            });
        }
    }

    // `${var/pat/repl}` family. Detect leading `/`/`//`/`/#`/`/%`,
    // then split on the second `/`.
    if rest.starts_with('/') {
        let (op, body) = if let Some(b) = rest.strip_prefix("//") {
            (1u8, b)
        } else if let Some(b) = rest.strip_prefix("/#") {
            (2u8, b)
        } else if let Some(b) = rest.strip_prefix("/%") {
            (3u8, b)
        } else {
            (0u8, &rest[1..])
        };
        // body = "pat/repl" or "pat" (no replacement = empty repl)
        let mut iter = body.splitn(2, '/');
        let pattern = iter.next().unwrap_or("").to_string();
        let repl = iter.next().unwrap_or("").to_string();
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Replace { op, pattern, repl },
        });
    }

    // `${var#pat}` / `${var##pat}` / `${var%pat}` / `${var%%pat}`
    if let Some(b) = rest.strip_prefix("##") {
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Strip { op: 1, pattern: b.to_string() },
        });
    }
    if let Some(b) = rest.strip_prefix("%%") {
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Strip { op: 3, pattern: b.to_string() },
        });
    }
    if let Some(b) = rest.strip_prefix('#') {
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Strip { op: 0, pattern: b.to_string() },
        });
    }
    if let Some(b) = rest.strip_prefix('%') {
        return Some(ParamModifier {
            name,
            kind: ParamModifierKind::Strip { op: 2, pattern: b.to_string() },
        });
    }

    None
}

/// Parse `${(flags)NAME}` and return (flags, name). The name must be a
/// plain identifier; nested expansions or subscripted names disqualify
/// this fast-path and route to the bridge instead. Mirrors
/// shell_compiler::try_lower_zsh_flag.
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
    let name = &inner[close + 1..];
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
    if !base.chars().next()?.is_ascii_alphabetic() && !base.starts_with('_') {
        return None;
    }
    if !base.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    // Reject keys that themselves contain `[` or `]` (nested subscript)
    // OR a `$`-expansion (must be evaluated at runtime, not compile time).
    if key.contains('[') || key.contains(']') || key.contains('$') || key.contains('`') {
        return None;
    }
    Some((base, key))
}

/// Return the array name if `s` is a `${NAME[@]}` or `${NAME[*]}` splice
/// form. Both expand to the array's elements as separate words; the
/// distinction with quoted forms is handled by the for-list / WORD_SPLIT
/// logic, not here.
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
    if matches!(first, '#' | '?' | '!' | '_' | '$' | '-' | '@' | '*')
        && inner.chars().count() == 1
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
    if matches!(first, '#' | '?' | '!' | '_' | '$' | '-')
        && rest.chars().count() == 1
    {
        return Some(rest);
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
    const ALPHA: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
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

