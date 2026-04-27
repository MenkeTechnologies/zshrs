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

use crate::arith_compiler::ArithCompiler;
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
    pub builder: ChunkBuilder,
    /// Variable name → slot index
    pub slots: HashMap<String, u16>,
    pub next_slot: u16,
    /// Break target stack — each loop pushes a fresh patch list. `break`
    /// inside the loop body emits an `Op::Jump(0)` and pushes the op index
    /// onto this list; the loop's exit code patches them all to loop_exit.
    break_patches: Vec<Vec<usize>>,
    /// Continue target stack — symmetric to `break_patches`. Was previously a
    /// `Vec<usize>` of pre-resolved targets, but `continue` inside a body
    /// captured the placeholder `0` (set before the body compiled), causing
    /// infinite loops. Now defers patching like break.
    continue_patches: Vec<Vec<usize>>,
    /// Patch sites for `return` / `exit` — `Op::Jump(0)` placeholders that
    /// `compile()` patches to the end of the chunk. This effectively halts
    /// the VM (ip past last op → run loop exits) without relying on
    /// `Op::Return`, which restores ip to the initial frame's return_ip (0)
    /// and would restart the body instead of halting.
    return_patches: Vec<usize>,
}

impl ShellCompiler {
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

    /// Compile a list of shell commands into a fusevm Chunk.
    pub fn compile(mut self, commands: &[ShellCommand]) -> fusevm::Chunk {
        // No PushFrame for top-level script — frames are for function calls only.
        // Top-level just executes and halts.
        for cmd in commands {
            self.compile_command(cmd);
        }
        // Return the last exit status
        self.builder.emit(Op::GetStatus, 0);
        // Patch all `return` / `exit` jumps to past the trailing GetStatus —
        // ip lands at ops.len(), the run loop's `while self.ip < ops.len()`
        // guard fails, VM exits naturally with last_status set by the builtin.
        let end = self.builder.current_pos();
        let returns = std::mem::take(&mut self.return_patches);
        for p in returns {
            self.builder.patch_jump(p, end);
        }
        self.builder.build()
    }

    fn slot_for(&mut self, name: &str) -> u16 {
        if let Some(&slot) = self.slots.get(name) {
            return slot;
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        self.slots.insert(name.to_string(), slot);
        slot
    }

    /// Compile `cmd` so it runs in the background (`cmd &`).
    ///
    /// Strategy: compile the command into a fresh sub-chunk and emit a builtin
    /// call that pops the sub-chunk index, forks, runs the chunk in the child,
    /// and returns Status(0) in the parent. This works for *any* command
    /// shape — simple, pipeline, compound, list — because the sub-chunk is the
    /// full compiled form of `cmd` and the bg builtin replays it via a fresh
    /// VM. Op::ExecBg in fusevm doesn't suffice on its own: it bypasses the
    /// host (so builtins/functions never fire) and only handles the dynamic-
    /// name flat-argv case.
    ///
    /// Per the punch list note, job-table integration is deferred to Phase G6;
    /// `jobs`/`fg`/`wait` won't see these pids until then.
    fn compile_command_bg(&mut self, cmd: &ShellCommand) {
        let sub_compiler = ShellCompiler::new();
        let sub_chunk = sub_compiler.compile(std::slice::from_ref(cmd));
        let sub_idx = self.builder.add_sub_chunk(sub_chunk);
        self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
        self.builder.emit(
            Op::CallBuiltin(crate::exec::BUILTIN_RUN_BG, 1),
            0,
        );
        self.builder.emit(Op::SetStatus, 0);
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
                // Register the function via runtime builtin so it persists in
                // `executor.functions_compiled` and is callable across chunks
                // (sourced files, autoload, future calls). The body is JSON-
                // serialized; the BUILTIN_REGISTER_FUNCTION handler compiles
                // it to a Chunk and stores it.
                //
                // This replaces the prior inline approach that registered a
                // sub_entry and jumped past the body — that worked for in-
                // chunk calls only. With Phase C, all calls go through
                // Op::CallFunction, which dispatches via ShellHost into the
                // compiled-function table regardless of which chunk defined
                // the function.
                let name_const = self.builder.add_constant(Value::str(name.as_str()));
                let body_json = serde_json::to_string(body.as_ref()).unwrap_or_default();
                let body_const = self.builder.add_constant(Value::str(body_json));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadConst(body_const), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_REGISTER_FUNCTION, 2),
                    0,
                );
                self.builder.emit(Op::SetStatus, 0);
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
        // Assignments: VAR=value. Route through BUILTIN_SET_VAR so the value
        // lands in `executor.variables` and is visible across nested VMs
        // (function calls) and tree-walker code paths. The VM's `Op::SetVar`
        // would otherwise stash it in this VM's per-instance globals, hidden
        // from nested calls.
        for (var, val, is_append) in &simple.assignments {
            // `foo[key]=val` / `foo[key]+=tail` — assoc-array single-key
            // set or append. The parser emits the assignment with var =
            // "foo[key]" verbatim. Split out the name and key, push name +
            // key + value, emit SET_ASSOC (replace) or APPEND_ASSOC (concat).
            // Without this branch, BUILTIN_SET_VAR would store under the
            // literal string "foo[key]" in executor.variables — readable by
            // nothing.
            if let Some(lbrack) = var.find('[') {
                if var.ends_with(']') {
                    let name = &var[..lbrack];
                    let key = &var[lbrack + 1..var.len() - 1];
                    if !name.is_empty()
                        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
                    {
                        let name_const =
                            self.builder.add_constant(Value::str(name));
                        self.builder.emit(Op::LoadConst(name_const), 0);
                        let key_const = self.builder.add_constant(Value::str(key));
                        self.builder.emit(Op::LoadConst(key_const), 0);
                        self.compile_word(val);
                        let builtin_id = if *is_append {
                            crate::exec::BUILTIN_APPEND_ASSOC
                        } else {
                            crate::exec::BUILTIN_SET_ASSOC
                        };
                        self.builder.emit(Op::CallBuiltin(builtin_id, 3), 0);
                        self.builder.emit(Op::Pop, 0);
                        continue;
                    }
                }
            }

            // `arr=(a b c)` / `arr+=(d e)` — array assignment / append. Push
            // each element + name, then SET_ARRAY (replace) or APPEND_ARRAY
            // (extend). Without this branch, the runtime fallback would JSON-
            // serialize the literal, expand to a single space-joined scalar,
            // and ${arr[i]} / ${arr[@]} would all hit broken semantics.
            if let ShellWord::ArrayLiteral(elements) = val {
                // CommandSub elements need word-splitting on IFS so
                // `arr=($(echo a b c))` produces 3 elements not 1.
                for elem in elements {
                    self.compile_word(elem);
                    if matches!(elem, ShellWord::CommandSub(_)) {
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_WORD_SPLIT, 1),
                            0,
                        );
                    }
                }
                // Flatten any Value::Array elements into individual entries
                // before SET_ARRAY. ARRAY_FLATTEN with N elements pushes
                // an Array + Int(len); we need just the flat element list
                // followed by name + count for SET_ARRAY's argc.
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_FLATTEN, elements.len() as u8),
                    0,
                );
                // Stack now: [..., FlatArray, FlatLen]. We need [..., e1, e2, …, eN, name, count].
                // Easiest: pop both, push each element, then name + recompute count.
                // But we don't know N at compile time after flatten. Use a
                // dedicated builtin pattern: leave Array on stack, push name,
                // call a SET_ARRAY_FLAT variant. For now, the simpler path:
                // discard the count, push name, call SET_ARRAY which we now
                // teach to accept Value::Array as the only-non-name arg.
                self.builder.emit(Op::Pop, 0); // discard FlatLen
                let name_const = self.builder.add_constant(Value::str(var.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                let builtin_id = if *is_append {
                    crate::exec::BUILTIN_APPEND_ARRAY
                } else {
                    crate::exec::BUILTIN_SET_ARRAY
                };
                self.builder.emit(Op::CallBuiltin(builtin_id, 2), 0);
                self.builder.emit(Op::Pop, 0);
                continue;
            }

            let name_const = self.builder.add_constant(Value::str(var.as_str()));
            self.builder.emit(Op::LoadConst(name_const), 0);
            self.compile_word(val);
            self.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_SET_VAR, 2),
                0,
            );
            self.builder.emit(Op::Pop, 0); // discard the Status(0) result
        }

        if simple.words.is_empty() {
            return; // bare assignment, no command
        }

        // Bracket redirects in a WithRedirects scope so they're restored after
        // the command runs. Without this, `cmd > out.txt` would leave fd 1
        // pointing at out.txt for everything else in the script — breaking the
        // very next `cat out.txt` (which would write its output to its own
        // input file, then read from a now-empty fd).
        let has_redirects = !simple.redirects.is_empty();
        if has_redirects {
            self.builder
                .emit(Op::WithRedirectsBegin(simple.redirects.len() as u8), 0);
        }

        // Redirects before command
        for redir in &simple.redirects {
            // `<&fd` (DupRead) and `<&-` (close-stdin) default to fd 0;
            // `>&fd` and `>&-` default to fd 1. Without DupRead in the
            // "read group", `read line <&10` parses as `1<&10` and dup2's
            // fd 10 onto STDOUT instead of STDIN — read then blocks on the
            // original fd 0 (terminal). Same root cause for any var-
            // expanded fd target like `<&${COPROC[1]}`.
            let fd = redir.fd.unwrap_or(match redir.op {
                crate::parser::RedirectOp::Read
                | crate::parser::RedirectOp::HereDoc
                | crate::parser::RedirectOp::HereString
                | crate::parser::RedirectOp::ReadWrite
                | crate::parser::RedirectOp::DupRead => 0,
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

        // Dispatch by first-word kind:
        //   literal-builtin → CallBuiltin (fast: pre-registered handler table)
        //   literal-other   → CallFunction (host: function-then-external)
        //   anything else   → Exec (dynamic name; preserves in-chunk find_sub)
        //
        // A `Literal` ShellWord whose body contains an unquoted expansion
        // trigger (`$cmd ...`, `*foo ...`, `~/bin/foo ...`) is NOT a literal
        // command name — it must be expanded first. Skip the literal-name fast
        // path so it falls through to the dynamic Op::Exec branch, where
        // compile_word lowers $/glob/tilde to runtime expansion before the
        // exec dispatches via host.exec → host_exec_external → run_intercepts.
        // Without this guard, `cmd=ls; $cmd` would route through
        // CallFunction(name="$cmd", ...) and `command not found: $cmd`.
        let first_is_dynamic_literal = if let ShellWord::Literal(s) = &simple.words[0] {
            Self::contains_unquoted(s, '$')
                || Self::contains_unquoted(s, '`')
                || Self::contains_unquoted(s, '*')
                || Self::contains_unquoted(s, '?')
                || Self::contains_unquoted(s, '[')
                || s.starts_with('~')
        } else {
            false
        };

        let mut emitted = false;
        if !first_is_dynamic_literal {
        if let ShellWord::Literal(cmd_name) = &simple.words[0] {
            // `break` / `continue` are loop-control sentinels — they must emit
            // a Jump into the active loop's patch list, not call a builtin
            // that does nothing useful. Without this, `break` is a no-op and
            // loops run to completion regardless.
            match cmd_name.as_str() {
                "break" => {
                    if let Some(patches) = self.break_patches.last_mut() {
                        let j = self.builder.emit(Op::Jump(0), 0);
                        patches.push(j);
                    } else {
                        // No enclosing loop in this chunk — we're inside a
                        // body that runs on a sub-VM (e.g. select). Set the
                        // executor's loop_signal so the outer-loop builtin
                        // sees the break request after the body returns,
                        // then halt this chunk via the return-patch path.
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_SET_BREAK, 0),
                            0,
                        );
                        self.builder.emit(Op::Pop, 0);
                        let j = self.builder.emit(Op::Jump(0), 0);
                        self.return_patches.push(j);
                    }
                    if has_redirects {
                        self.builder.emit(Op::WithRedirectsEnd, 0);
                    }
                    return;
                }
                "continue" => {
                    if let Some(patches) = self.continue_patches.last_mut() {
                        let j = self.builder.emit(Op::Jump(0), 0);
                        patches.push(j);
                    } else {
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_SET_CONTINUE, 0),
                            0,
                        );
                        self.builder.emit(Op::Pop, 0);
                        let j = self.builder.emit(Op::Jump(0), 0);
                        self.return_patches.push(j);
                    }
                    if has_redirects {
                        self.builder.emit(Op::WithRedirectsEnd, 0);
                    }
                    return;
                }
                _ => {}
            }

            if let Some(builtin_id) = fusevm::shell_builtins::builtin_id(cmd_name) {
                let argc = (simple.words.len() - 1) as u8;
                for word in &simple.words[1..] {
                    self.compile_word(word);
                }
                self.builder.emit(Op::CallBuiltin(builtin_id, argc), 0);
                self.builder.emit(Op::SetStatus, 0);
                // `return` / `exit` short-circuit the function/script. Emit a
                // forward Jump that `compile()` patches to past the chunk end
                // (effectively halting the VM). Op::Return doesn't work here
                // because it restores ip to the initial frame's return_ip (0)
                // and would restart the body from the top.
                if cmd_name == "return" || cmd_name == "exit" {
                    let j = self.builder.emit(Op::Jump(0), 0);
                    self.return_patches.push(j);
                }
                emitted = true;
            } else {
                // Non-builtin literal name → Op::CallFunction. The host's
                // call_function checks `executor.functions_compiled`, running
                // it on a nested VM. If not a function, the host returns None
                // and fusevm falls back to host.exec which spawns externally.
                let name_idx = self.builder.add_name(cmd_name);
                let argc = (simple.words.len() - 1) as u8;
                for word in &simple.words[1..] {
                    self.compile_word(word);
                }
                self.builder.emit(Op::CallFunction(name_idx, argc), 0);
                self.builder.emit(Op::SetStatus, 0);
                emitted = true;
            }
        }
        } // end `if !first_is_dynamic_literal`

        if !emitted {
            // Dynamic command name (variable, command-sub, etc.) → Op::Exec.
            let argc = simple.words.len() as u8;
            for word in &simple.words {
                self.compile_word(word);
            }
            self.builder.emit(Op::Exec(argc), 0);
            self.builder.emit(Op::SetStatus, 0);
        }

        if has_redirects {
            self.builder.emit(Op::WithRedirectsEnd, 0);
        }
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
    fn compile_pipeline(&mut self, cmds: &[ShellCommand], negated: bool) {
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

        // Phase D pipeline lowering: compile each stage as its own sub-chunk
        // (using fusevm 0.10.0's `Chunk::sub_chunks`), push the indices on the
        // stack, then dispatch to BUILTIN_RUN_PIPELINE which forks per stage
        // with stdin/stdout wired through pipes. This is bytecode-native — no
        // tree-walker callback. The fork-per-stage approach keeps each stage
        // isolated (matching POSIX subshell semantics) and lets builtins,
        // functions, and compounds all participate in pipelines uniformly.
        let n = cmds.len() as u8;
        for cmd in cmds {
            let stage_compiler = ShellCompiler::new();
            let stage_chunk = stage_compiler.compile(std::slice::from_ref(cmd));
            let idx = self.builder.add_sub_chunk(stage_chunk);
            self.builder.emit(Op::LoadInt(idx as i64), 0);
        }
        self.builder.emit(
            Op::CallBuiltin(crate::exec::BUILTIN_RUN_PIPELINE, n),
            0,
        );
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

    /// Compile a list: `cmd1 && cmd2 || cmd3 ; cmd4 & cmd5`.
    ///
    /// Each item in `items` is `(cmd_i, op_i)` where `op_i` applies *between*
    /// `cmd_i` and `cmd_(i+1)`. So `&&`/`||` short-circuit the *next* command,
    /// not the current one. We walk `items` once: for each non-tail position,
    /// compile cmd_i, emit a conditional jump matching op_i, and continue —
    /// the patch lands at the position right after cmd_(i+1) is compiled by
    /// the next iteration. This avoids the double-compile that happened when
    /// `&&`/`||` arms tried to emit their successor inline while the outer
    /// loop also iterated to the successor.
    fn compile_list(&mut self, items: &[(ShellCommand, crate::parser::ListOp)]) {
        // Pending short-circuit jumps: list of patch sites that need to be
        // patched to the position right after the current command is compiled.
        let mut pending_skip: Option<usize> = None;

        for (i, (cmd, op)) in items.iter().enumerate() {
            // `cmd &` — the Amp op SUFFIXES `cmd`, so the BG dispatch belongs
            // with the current item, not the next one. Compile `cmd` into a
            // sub-chunk and emit BUILTIN_RUN_BG which forks. Background cmds
            // always return status 0 immediately, regardless of what the child
            // ends up doing.
            if matches!(op, crate::parser::ListOp::Amp) {
                self.compile_command_bg(cmd);
            } else {
                self.compile_command(cmd);
            }

            // Patch the prior `&&`/`||` skip target to land here, after this
            // command compiled. (The skip was emitted by the previous iteration
            // and was conditional on the previous command's status.)
            if let Some(skip_idx) = pending_skip.take() {
                self.builder.patch_jump(skip_idx, self.builder.current_pos());
            }

            // Now look at op_i — what to do *between* this command and the next.
            // Note: GetStatus pushes Value::Status(c). In fusevm's truthiness,
            // Status(0) (success) is_truthy=true (shell semantics). So:
            //   && — skip-next-if-current-failed → JumpIfFalse (status non-truthy)
            //   || — skip-next-if-current-succeeded → JumpIfTrue (status truthy)
            match op {
                crate::parser::ListOp::And if i + 1 < items.len() => {
                    self.builder.emit(Op::GetStatus, 0);
                    pending_skip = Some(self.builder.emit(Op::JumpIfFalse(0), 0));
                }
                crate::parser::ListOp::Or if i + 1 < items.len() => {
                    self.builder.emit(Op::GetStatus, 0);
                    pending_skip = Some(self.builder.emit(Op::JumpIfTrue(0), 0));
                }
                _ => {
                    // Semi / Newline / Amp / trailing And-Or — no jump. Amp
                    // already dispatched above as a BG sub-chunk.
                }
            }
        }

        // Trailing pending_skip with no following command shouldn't occur
        // (guarded by `i + 1 < items.len()` above), but patch defensively.
        if let Some(skip_idx) = pending_skip {
            self.builder.patch_jump(skip_idx, self.builder.current_pos());
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
                let var_idx = self.builder.add_name(var);

                // Build the word list — count items
                let item_count = if let Some(words) = words {
                    words.len()
                } else {
                    0
                };

                // Compile each word at runtime so `for i in ${arr[@]}` and
                // `for i in $var $other` actually expand. Words producing
                // Value::Array (notably `${arr[@]}`, `${arr[i]}` with `@`/`*`,
                // ArrayVar) get flattened one level via BUILTIN_ARRAY_FLATTEN
                // so the for-loop iterates over the array's elements, not
                // over a single-element array containing an Array. Pre-fix
                // this branch ran `word_to_string` at compile time which
                // produced literal strings like `${arr[@]}`.
                //
                // BUILTIN_ARRAY_FLATTEN's contract is unusual: it pushes a
                // Value::Array onto the stack AND returns Value::Int(len) as
                // its result. CallBuiltin pushes the result on top, so the
                // post-call stack is [..., Array, Int(len)] (top → Int(len)).
                // We consume them in that order: SetSlot(len_slot) first,
                // then SetSlot(arr_slot).
                let arr_slot = self.next_slot;
                self.next_slot += 1;
                if let Some(words) = words {
                    for word in words {
                        self.compile_word(word);
                    }
                    self.builder.emit(
                        Op::CallBuiltin(
                            crate::exec::BUILTIN_ARRAY_FLATTEN,
                            item_count as u8,
                        ),
                        0,
                    );
                    self.builder.emit(Op::SetSlot(len_slot), 0);
                    self.builder.emit(Op::SetSlot(arr_slot), 0);
                } else {
                    // No words = iterate $@ (positional params)
                    // TODO: load positional params
                    self.builder.emit(Op::MakeArray(0), 0);
                    self.builder.emit(Op::SetSlot(arr_slot), 0);
                    self.builder.emit(Op::LoadInt(0), 0);
                    self.builder.emit(Op::SetSlot(len_slot), 0);
                }

                // i = 0
                self.builder.emit(Op::LoadInt(0), 0);
                self.builder.emit(Op::SetSlot(i_slot), 0);

                // loop_top:
                let loop_top = self.builder.current_pos();
                self.builder.emit(Op::GetSlot(i_slot), 0);
                self.builder.emit(Op::GetSlot(len_slot), 0);
                self.builder.emit(Op::NumLt, 0);
                let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

                // var = array[i] — get element, write to executor.variables
                // via BUILTIN_SET_VAR so $var inside the body resolves to the
                // current iteration value across nested VMs.
                let var_name_const =
                    self.builder.add_constant(Value::str(var.as_str()));
                self.builder.emit(Op::LoadConst(var_name_const), 0);
                self.builder.emit(Op::GetSlot(i_slot), 0);
                self.builder.emit(Op::SlotArrayGet(arr_slot), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_SET_VAR, 2),
                    0,
                );
                self.builder.emit(Op::Pop, 0); // discard Status(0) result
                let _ = var_idx;

                // Push fresh break/continue patch lists for this loop.
                self.break_patches.push(Vec::new());
                self.continue_patches.push(Vec::new());

                // body
                for cmd in body {
                    self.compile_command(cmd);
                }

                // loop_continue: patch all `continue` jumps to here, then run
                // the increment + back-jump.
                let continue_target = self.builder.current_pos();
                if let Some(continues) = self.continue_patches.pop() {
                    for cp in continues {
                        self.builder.patch_jump(cp, continue_target);
                    }
                }

                self.builder.emit(Op::PreIncSlotVoid(i_slot), 0);
                self.builder.emit(Op::Jump(loop_top), 0);

                // loop_exit:
                let loop_exit = self.builder.current_pos();
                self.builder.patch_jump(exit_jump, loop_exit);

                // Patch all break jumps to loop_exit.
                if let Some(breaks) = self.break_patches.pop() {
                    for bp in breaks {
                        self.builder.patch_jump(bp, loop_exit);
                    }
                }
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

                self.break_patches.push(Vec::new());
                self.continue_patches.push(Vec::new());

                // body
                for cmd in body {
                    self.compile_command(cmd);
                }

                // continue target = step expression
                let continue_target = self.builder.current_pos();
                if let Some(continues) = self.continue_patches.pop() {
                    for cp in continues {
                        self.builder.patch_jump(cp, continue_target);
                    }
                }

                // step expression
                if !step.is_empty() {
                    self.compile_arith_inline(step);
                    self.builder.emit(Op::Pop, 0); // discard step result
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
                    // Shell: status 0 = success = truthy. Skip body when the
                    // condition FAILED — i.e. when GetStatus is non-truthy.
                    let skip_body = self.builder.emit(Op::JumpIfFalse(0), 0);

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
                self.builder.emit(Op::GetSlot(i_slot), 0);
                self.builder.emit(Op::GetSlot(count_slot), 0);
                self.builder.emit(Op::NumLt, 0);
                let exit_jump = self.builder.emit(Op::JumpIfFalse(0), 0);

                self.break_patches.push(Vec::new());
                self.continue_patches.push(Vec::new());

                for cmd in body {
                    self.compile_command(cmd);
                }

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
                        // Case patterns must reach Op::StrMatch as the literal
                        // pattern string — `compile_word` would otherwise glob-
                        // expand `*` into the cwd listing. Push the pattern's
                        // literal text via `compile_case_pattern` (no glob, no
                        // tilde, no command sub — just $-vars and concatenation).
                        self.compile_case_pattern(pattern);
                        // Op::StrMatch routes through `ShellHost::str_match`
                        // which uses zshrs's glob_match for `*`/`?`/`[...]`.
                        self.builder.emit(Op::StrMatch, 0);
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

            // ── select var in words; do body; done ──
            //
            // Push each word at runtime (so `select x in $arr; ...` expands),
            // then var name, then body sub-chunk index, then call
            // BUILTIN_RUN_SELECT. The builtin owns the menu/prompt/read
            // loop. Stack discipline at call:
            //   [..., word_1, word_2, ..., word_N, name, sub_idx]
            // argc = N + 2.
            CompoundCommand::Select { var, words, body } => {
                // Compile body as a sub-chunk so the builtin can re-run it
                // for each iteration without us having to manage a loop in
                // the parent chunk. body is a Vec<ShellCommand>.
                let sub_compiler = ShellCompiler::new();
                let sub_chunk = sub_compiler.compile(body);
                let sub_idx = self.builder.add_sub_chunk(sub_chunk);

                let word_count = words.as_ref().map(|w| w.len()).unwrap_or(0);
                if let Some(words) = words {
                    for w in words {
                        self.compile_word(w);
                    }
                }
                let name_const = self.builder.add_constant(Value::str(var.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadInt(sub_idx as i64), 0);

                let argc = (word_count + 2) as u8;
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_RUN_SELECT, argc),
                    0,
                );
                self.builder.emit(Op::SetStatus, 0);
            }

            // ── coproc ──
            //
            // `coproc [name] { body }` — body runs async with stdin/stdout
            // wired to two parent-side pipes. Compile body as a sub-chunk;
            // push name + sub-chunk index; CallBuiltin(BUILTIN_RUN_COPROC, 2).
            // The handler forks, sets up pipes, stores [read_fd, write_fd]
            // into executor.arrays[name] (default "COPROC"). Per Phase G6
            // note: pid not in JobTable yet, so `wait` won't see it.
            CompoundCommand::Coproc { name, body } => {
                let sub_compiler = ShellCompiler::new();
                let sub_chunk =
                    sub_compiler.compile(std::slice::from_ref(body.as_ref()));
                let sub_idx = self.builder.add_sub_chunk(sub_chunk);

                let name_str = name.as_deref().unwrap_or("");
                let name_const = self.builder.add_constant(Value::str(name_str));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(Op::LoadInt(sub_idx as i64), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_RUN_COPROC, 2),
                    0,
                );
                self.builder.emit(Op::SetStatus, 0);
            }

            // ── compound-cmd > out.txt | < in.txt etc. ──
            CompoundCommand::WithRedirects(cmd, redirects) => {
                // Push a redirect scope so the host saves dup'd fds; emit each
                // Redirect op (which pops target + calls host.redirect → host
                // tracks saved fd in the top scope); compile the inner command;
                // then pop the scope to restore. This mirrors how exec.rs
                // handled WithRedirects (saved_fds Vec) but in bytecode.
                self.builder
                    .emit(Op::WithRedirectsBegin(redirects.len() as u8), 0);

                for redir in redirects {
                    // Same default-fd rule as compile_simple: DupRead's `<&`
                    // form targets fd 0, not fd 1.
                    let fd = redir.fd.unwrap_or(match redir.op {
                        crate::parser::RedirectOp::Read
                        | crate::parser::RedirectOp::HereDoc
                        | crate::parser::RedirectOp::HereString
                        | crate::parser::RedirectOp::ReadWrite
                        | crate::parser::RedirectOp::DupRead => 0,
                        _ => 1,
                    }) as u8;

                    let op_byte = match redir.op {
                        crate::parser::RedirectOp::Write => fusevm::op::redirect_op::WRITE,
                        crate::parser::RedirectOp::Append => fusevm::op::redirect_op::APPEND,
                        crate::parser::RedirectOp::Read => fusevm::op::redirect_op::READ,
                        crate::parser::RedirectOp::ReadWrite => {
                            fusevm::op::redirect_op::READ_WRITE
                        }
                        crate::parser::RedirectOp::Clobber => fusevm::op::redirect_op::CLOBBER,
                        crate::parser::RedirectOp::DupRead => fusevm::op::redirect_op::DUP_READ,
                        crate::parser::RedirectOp::DupWrite => fusevm::op::redirect_op::DUP_WRITE,
                        crate::parser::RedirectOp::WriteBoth => {
                            fusevm::op::redirect_op::WRITE_BOTH
                        }
                        crate::parser::RedirectOp::AppendBoth => {
                            fusevm::op::redirect_op::APPEND_BOTH
                        }
                        crate::parser::RedirectOp::HereDoc => {
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

                self.compile_command(cmd);
                self.builder.emit(Op::WithRedirectsEnd, 0);
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
                self.builder
                    .emit(Op::TestFile(fusevm::op::file_test::EXISTS), 0);
            }
            CondExpr::FileRegular(w) => {
                self.compile_word(w);
                self.builder
                    .emit(Op::TestFile(fusevm::op::file_test::IS_FILE), 0);
            }
            CondExpr::FileDirectory(w) => {
                self.compile_word(w);
                self.builder
                    .emit(Op::TestFile(fusevm::op::file_test::IS_DIR), 0);
            }
            CondExpr::FileSymlink(w) => {
                self.compile_word(w);
                self.builder
                    .emit(Op::TestFile(fusevm::op::file_test::IS_SYMLINK), 0);
            }
            CondExpr::FileReadable(w) => {
                self.compile_word(w);
                self.builder
                    .emit(Op::TestFile(fusevm::op::file_test::IS_READABLE), 0);
            }
            CondExpr::FileWritable(w) => {
                self.compile_word(w);
                self.builder
                    .emit(Op::TestFile(fusevm::op::file_test::IS_WRITABLE), 0);
            }
            CondExpr::FileExecutable(w) => {
                self.compile_word(w);
                self.builder
                    .emit(Op::TestFile(fusevm::op::file_test::IS_EXECUTABLE), 0);
            }
            CondExpr::FileNonEmpty(w) => {
                self.compile_word(w);
                self.builder
                    .emit(Op::TestFile(fusevm::op::file_test::IS_NONEMPTY), 0);
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
                // zsh/bash: `[[ s = pat ]]` is glob-pattern match, not exact
                // string compare. Push LHS via compile_word (full expansion)
                // and RHS via compile_case_pattern (literal pattern, no glob/
                // tilde expansion of the pattern itself). Op::StrMatch routes
                // through ShellHost::str_match which uses zshrs's glob_match.
                self.compile_word(a);
                self.compile_case_pattern(b);
                self.builder.emit(Op::StrMatch, 0);
            }
            CondExpr::StringNotEqual(a, b) => {
                self.compile_word(a);
                self.compile_case_pattern(b);
                self.builder.emit(Op::StrMatch, 0);
                self.builder.emit(Op::LogNot, 0);
            }
            CondExpr::StringMatch(a, b) => {
                // `[[ s =~ regex ]]` — POSIX extended regex. Routed to host
                // via Op::RegexMatch which calls `ShellHost::regex_match`.
                // The RHS must NOT be glob-expanded — `^h.*o` is a regex
                // pattern, not a glob. compile_case_pattern preserves the
                // string verbatim (modulo $-expansion). Without this, `*`/
                // `?`/`[` in the regex trigger glob expansion and the regex
                // turns into the cwd listing.
                self.compile_word(a);
                self.compile_case_pattern(b);
                self.builder.emit(Op::RegexMatch, 0);
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

    /// If `s` matches one of the array-access shapes (`${arr[@]}`, `${arr[*]}`,
    /// `${arr[idx]}`, `${#arr[@]}`), emit native bytecode for it and return
    /// true. Otherwise return false so the caller continues with the normal
    /// trigger-based dispatch. This handles the gap where the parser produces
    /// `Literal("${arr[@]}")` instead of `VariableBraced(arr, ArrayAll)` for
    /// unquoted array references — a proper parser fix is Phase G1 follow-up.
    ///
    /// Tightness: the body between `${` and `}` must contain neither another
    /// `${` nor any `}` outside the closing one. Otherwise inputs like
    /// `${foo[x]}_${foo[y]}` would falsely match (treating `x]}_${foo[y` as
    /// the index) and emit a single ARRAY_INDEX instead of two separate
    /// lookups concatenated with `_`. Such mixed inputs go to the runtime
    /// fallback, which knows how to walk the string.
    fn try_lower_array_literal(&mut self, s: &str) -> bool {
        if !s.starts_with("${") || !s.ends_with('}') {
            return false;
        }
        let inner = &s[2..s.len() - 1];
        // Reject multi-group cases like `${foo[x]}_${foo[y]}` — those have
        // either a `}` or another `${` inside `inner` and need the runtime
        // string-walking expansion.
        if inner.contains("${") || inner.contains('}') {
            return false;
        }
        let (length_form, body) = if let Some(rest) = inner.strip_prefix('#') {
            (true, rest)
        } else {
            (false, inner)
        };
        let lbrack = match body.find('[') {
            Some(i) => i,
            None => return false,
        };
        if !body.ends_with(']') {
            return false;
        }
        let name = &body[..lbrack];
        let idx = &body[lbrack + 1..body.len() - 1];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return false;
        }
        // Name must start with letter or `_` (zsh identifier rules).
        if !name.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false) {
            return false;
        }

        let name_const = self.builder.add_constant(Value::str(name));
        self.builder.emit(Op::LoadConst(name_const), 0);

        if length_form {
            // ${#arr[@]} / ${#arr[*]} → array length. Other index forms with
            // `#` aren't standard; route through ARRAY_LENGTH which only needs
            // the name (it ignores idx — `${#arr[1]}` would be string length
            // of element 1 in zsh, but that's an edge case for follow-up).
            self.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_LENGTH, 1),
                0,
            );
            return true;
        }

        if idx == "@" || idx == "*" {
            self.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_ALL, 1),
                0,
            );
        } else {
            let idx_const = self.builder.add_constant(Value::str(idx));
            self.builder.emit(Op::LoadConst(idx_const), 0);
            self.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2),
                0,
            );
        }
        true
    }

    /// Compile a ShellWord to a value on the stack.
    fn compile_word(&mut self, word: &ShellWord) {
        match word {
            ShellWord::Literal(s) => {
                // ANSI-C quoting: `$'…'` is a single-quoted string with C-
                // style escape interpretation. The lexer preserves it as
                // `$'a\tb'` literal; here we recognize the shape and
                // un-escape into the actual bytes.
                if s.starts_with("$'") && s.ends_with('\'') && s.len() >= 3 {
                    let inner = &s[2..s.len() - 1];
                    let mut out = String::with_capacity(inner.len());
                    let mut chars = inner.chars().peekable();
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
                            Some('0') => out.push('\0'),
                            Some('a') => out.push('\x07'),
                            Some('b') => out.push('\x08'),
                            Some('e') | Some('E') => out.push('\x1b'),
                            Some('f') => out.push('\x0c'),
                            Some('v') => out.push('\x0b'),
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
                    let idx = self.builder.add_constant(Value::str(out.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    return;
                }
                // Process substitution: the lexer emits `<(...)` and `>(...)`
                // as a single literal token. Re-detect at compile time and
                // route through the dedicated sub-chunk + ProcessSub op so
                // host.process_sub_in/out can wire up the FIFO/temp file.
                if (s.starts_with("<(") || s.starts_with(">(")) && s.ends_with(')') {
                    let is_in = s.starts_with("<(");
                    let inner_src = &s[2..s.len() - 1];
                    let mut p = crate::parser::ShellParser::new(inner_src);
                    if let Ok(cmds) = p.parse_script() {
                        let sub_compiler = ShellCompiler::new();
                        let sub_chunk = sub_compiler.compile(&cmds);
                        let sub_idx = self.builder.add_sub_chunk(sub_chunk);
                        if is_in {
                            self.builder.emit(Op::ProcessSubIn(sub_idx), 0);
                        } else {
                            self.builder.emit(Op::ProcessSubOut(sub_idx), 0);
                        }
                        return;
                    }
                }
                // Native array-access lowering (compile-side parser hack —
                // see try_lower_array_literal for context).
                if Self::looks_like_array_literal(s) && self.try_lower_array_literal(s) {
                    return;
                }
                // `${(flags)name}` — zsh parameter-expansion flags.
                if Self::looks_like_zsh_flag(s) && self.try_lower_zsh_flag(s) {
                    return;
                }
                // Triggers that demand the runtime expander (expand_word / expand_string):
                //   $    — variables, ${...} modifiers, $((...)), $(...)
                //   `    — backtick command substitution
                //   ~    — tilde expansion (only at start of word, or after `:` for PATH-likes)
                //   * ? [ — glob characters
                //
                // The lexer flags single-quoted special chars with a leading
                // `\0` sentinel (parser.rs read_word: `'…'` arms). A `\0$` byte
                // pair means "literal $", not a parameter expansion. Trigger
                // detection must respect the sentinel, and emission must strip
                // it before the bytes reach the VM stack or runtime expander.
                let trigger_dollar =
                    Self::contains_unquoted(s, '$') || Self::contains_unquoted(s, '`');
                let trigger_glob = Self::contains_unquoted(s, '*')
                    || Self::contains_unquoted(s, '?')
                    || Self::contains_unquoted(s, '[');
                let trigger_tilde = s.starts_with('~') || s.contains(":~") || s.contains("=~");
                let trigger_brace = Self::has_brace_expansion(s);
                let trigger_glob_qual = Self::has_glob_qualifier(s);

                // Brace expansion takes precedence over scalar-literal storage:
                // `{1..5}`, `{a,b,c}`, `pre{a,b}post` all need runtime expansion.
                if trigger_brace && !trigger_dollar && !trigger_glob && !trigger_tilde {
                    let cleaned = Self::strip_quote_markers(s);
                    let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_BRACE_EXPAND, 1),
                        0,
                    );
                    return;
                }
                // Glob with qualifier suffix `pat(qual)` — e.g. `*(.x)`, `*(N)`.
                if trigger_glob_qual && !trigger_dollar && !trigger_tilde {
                    if let Some((pat, qual)) = Self::split_glob_qualifier(s) {
                        let pat_const =
                            self.builder.add_constant(Value::str(pat.as_str()));
                        self.builder.emit(Op::LoadConst(pat_const), 0);
                        let qual_const =
                            self.builder.add_constant(Value::str(qual.as_str()));
                        self.builder.emit(Op::LoadConst(qual_const), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_GLOB_QUALIFIED, 2),
                            0,
                        );
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_JOIN, 1),
                            0,
                        );
                        return;
                    }
                }
                if !trigger_dollar && !trigger_glob && !trigger_tilde {
                    // Pure literal — no expansion possible, store as constant.
                    let cleaned = Self::strip_quote_markers(s);
                    let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                } else if trigger_dollar
                    && !trigger_glob
                    && !trigger_tilde
                    && Self::literal_has_only_simple_vars(s)
                {
                    // Cheap inline path: only $VAR / ${VAR} and no glob/tilde.
                    self.compile_string_with_expansions(s);
                } else if !trigger_dollar && !trigger_glob && trigger_tilde {
                    // Pure tilde, e.g. `~/foo` or `~user/path` — push and
                    // dispatch to host.tilde_expand directly (no JSON
                    // roundtrip via expand_word).
                    let cleaned = Self::strip_quote_markers(s);
                    let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    self.builder.emit(Op::TildeExpand, 0);
                } else if !trigger_dollar && trigger_glob && !trigger_tilde {
                    // Pure glob, e.g. `*.rs` or `Cargo.*` — push pattern,
                    // expand via host.glob, join the resulting Array into the
                    // single argv-token form the word model expects.
                    let cleaned = Self::strip_quote_markers(s);
                    let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    self.builder.emit(Op::Glob, 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_JOIN, 1),
                        0,
                    );
                } else if !trigger_dollar && trigger_glob && trigger_tilde {
                    // Tilde + glob, e.g. `~/*.rs` — expand tilde first, then
                    // glob the result. Both operate on the same string so we
                    // can chain: TildeExpand pops/pushes string, Glob pops
                    // string and pushes Array, ArrayJoin reduces to string.
                    let cleaned = Self::strip_quote_markers(s);
                    let idx = self.builder.add_constant(Value::str(cleaned.as_str()));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    self.builder.emit(Op::TildeExpand, 0);
                    self.builder.emit(Op::Glob, 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_JOIN, 1),
                        0,
                    );
                } else {
                    // Mixed: $-expansion combined with glob/tilde, or
                    // `$(...)`, `$((...))`, backticks, `${VAR/x/y}`. The
                    // runtime expand_word handles every case correctly.
                    self.compile_word_runtime(word);
                }
            }
            ShellWord::SingleQuoted(s) => {
                let idx = self.builder.add_constant(Value::str(s.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
            }
            ShellWord::Variable(name) => {
                // Route through BUILTIN_GET_VAR (executor.variables + special
                // params + arrays + env) instead of Op::GetVar (per-VM
                // globals). Same storage as expand_word so nested VMs see it.
                let name_const = self.builder.add_constant(Value::str(name.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                    0,
                );
            }
            ShellWord::DoubleQuoted(parts) => {
                if parts.is_empty() {
                    let idx = self.builder.add_constant(Value::str(""));
                    self.builder.emit(Op::LoadConst(idx), 0);
                } else if parts.len() == 1 {
                    self.compile_word(&parts[0]);
                } else {
                    self.compile_word(&parts[0]);
                    for part in &parts[1..] {
                        self.compile_word(part);
                        self.builder.emit(Op::Concat, 0);
                    }
                }
            }
            ShellWord::Concat(parts) => {
                if parts.is_empty() {
                    let idx = self.builder.add_constant(Value::str(""));
                    self.builder.emit(Op::LoadConst(idx), 0);
                } else if parts.len() == 1 {
                    self.compile_word(&parts[0]);
                } else {
                    self.compile_word(&parts[0]);
                    for part in &parts[1..] {
                        self.compile_word(part);
                        self.builder.emit(Op::Concat, 0);
                    }
                }
            }
            // ── Native lowering for VariableBraced (${...}) ────────────────
            ShellWord::VariableBraced(name, modifier_opt) => {
                use crate::parser::VarModifier;
                use fusevm::op::param_mod;

                // Helper: push the variable name onto the stack.
                let push_name = |c: &mut Self| {
                    let idx = c.builder.add_constant(Value::str(name.as_str()));
                    c.builder.emit(Op::LoadConst(idx), 0);
                };

                match modifier_opt.as_deref() {
                    None => {
                        // Inside double quotes, the parser folds `${(flag)var}`
                        // and `${arr[idx]}` into a single VariableBraced node
                        // with the entire content as the `name` and no
                        // modifier — the flag/bracket isn't decomposed at
                        // parse time. Detect those shapes here and re-route
                        // to the same lowering used by the Literal arm.
                        if name.starts_with('(') {
                            let synthetic =
                                format!("${{{}}}", name.as_str());
                            if Self::looks_like_zsh_flag(synthetic.as_str())
                                && self.try_lower_zsh_flag(synthetic.as_str())
                            {
                                return;
                            }
                        }
                        if name.contains('[') && name.ends_with(']') {
                            let synthetic =
                                format!("${{{}}}", name.as_str());
                            if Self::looks_like_array_literal(synthetic.as_str())
                                && self.try_lower_array_literal(synthetic.as_str())
                            {
                                return;
                            }
                        }
                        // Plain ${var} — same as $var, route through GET_VAR.
                        let idx = self.builder.add_constant(Value::str(name.as_str()));
                        self.builder.emit(Op::LoadConst(idx), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                            0,
                        );
                    }
                    Some(VarModifier::Default(w)) => {
                        push_name(self);
                        self.compile_word(w);
                        self.builder.emit(Op::ExpandParam(param_mod::DEFAULT), 0);
                    }
                    Some(VarModifier::DefaultAssign(w)) => {
                        push_name(self);
                        self.compile_word(w);
                        self.builder.emit(Op::ExpandParam(param_mod::ASSIGN), 0);
                    }
                    Some(VarModifier::Error(w)) => {
                        push_name(self);
                        self.compile_word(w);
                        self.builder.emit(Op::ExpandParam(param_mod::ERROR), 0);
                    }
                    Some(VarModifier::Alternate(w)) => {
                        push_name(self);
                        self.compile_word(w);
                        self.builder.emit(Op::ExpandParam(param_mod::ALTERNATE), 0);
                    }
                    Some(VarModifier::Length) => {
                        push_name(self);
                        self.builder.emit(Op::ExpandParam(param_mod::LENGTH), 0);
                    }
                    Some(VarModifier::Substring(off, len_opt)) => {
                        push_name(self);
                        self.builder.emit(Op::LoadInt(*off), 0);
                        // -1 sentinel = no length specified (host inverts)
                        self.builder.emit(Op::LoadInt(len_opt.unwrap_or(-1)), 0);
                        self.builder.emit(Op::ExpandParam(param_mod::SLICE), 0);
                    }
                    Some(VarModifier::RemovePrefix(w)) => {
                        push_name(self);
                        self.compile_word(w);
                        self.builder.emit(Op::ExpandParam(param_mod::STRIP_SHORT), 0);
                    }
                    Some(VarModifier::RemovePrefixLong(w)) => {
                        push_name(self);
                        self.compile_word(w);
                        self.builder.emit(Op::ExpandParam(param_mod::STRIP_LONG), 0);
                    }
                    Some(VarModifier::RemoveSuffix(w)) => {
                        push_name(self);
                        self.compile_word(w);
                        self.builder.emit(Op::ExpandParam(param_mod::RSTRIP_SHORT), 0);
                    }
                    Some(VarModifier::RemoveSuffixLong(w)) => {
                        push_name(self);
                        self.compile_word(w);
                        self.builder.emit(Op::ExpandParam(param_mod::RSTRIP_LONG), 0);
                    }
                    Some(VarModifier::Replace(p, r)) => {
                        push_name(self);
                        self.compile_word(p);
                        self.compile_word(r);
                        self.builder.emit(Op::ExpandParam(param_mod::SUBST_FIRST), 0);
                    }
                    Some(VarModifier::ReplaceAll(p, r)) => {
                        push_name(self);
                        self.compile_word(p);
                        self.compile_word(r);
                        self.builder.emit(Op::ExpandParam(param_mod::SUBST_ALL), 0);
                    }
                    Some(VarModifier::Upper) => {
                        push_name(self);
                        self.builder.emit(Op::ExpandParam(param_mod::UPPER), 0);
                    }
                    Some(VarModifier::Lower) => {
                        push_name(self);
                        self.builder.emit(Op::ExpandParam(param_mod::LOWER), 0);
                    }
                    // ── Indexed-array lowering (Phase G1) ─────────────────
                    Some(VarModifier::ArrayLength) => {
                        // ${#arr[@]} → push name, BUILTIN_ARRAY_LENGTH.
                        push_name(self);
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_LENGTH, 1),
                            0,
                        );
                    }
                    Some(VarModifier::ArrayIndex(idx_str)) => {
                        // ${arr[idx]} — push name, idx, BUILTIN_ARRAY_INDEX.
                        // The idx is parsed as a string by the parser; the
                        // runtime resolves it (handles "@", "*", numeric, and
                        // anything else as 0).
                        push_name(self);
                        let idx_const = self.builder.add_constant(Value::str(idx_str.as_str()));
                        self.builder.emit(Op::LoadConst(idx_const), 0);
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2),
                            0,
                        );
                    }
                    Some(VarModifier::ArrayAll) => {
                        // ${arr[@]} — push name, BUILTIN_ARRAY_ALL. Returns a
                        // Value::Array that Op::Exec/ExecBg/CallFunction
                        // flatten into argv slots automatically.
                        push_name(self);
                        self.builder.emit(
                            Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_ALL, 1),
                            0,
                        );
                    }
                    // ZshFlags is zsh-specific (`(L)`, `(j: :)`, etc.) — many
                    // flags, large surface, deferred to Phase G4.
                    Some(_) => self.compile_word_runtime(word),
                }
            }

            // ── Native lowering for the simple variants ─────────────────────
            ShellWord::Tilde(user) => {
                // ~ → $HOME, ~user → user's home. Push the literal "~..."
                // and let Op::TildeExpand (host-routed to expand_tilde_named)
                // resolve it. No JSON roundtrip.
                let s = match user {
                    Some(u) => format!("~{}", u),
                    None => "~".to_string(),
                };
                let const_idx = self.builder.add_constant(Value::str(s));
                self.builder.emit(Op::LoadConst(const_idx), 0);
                self.builder.emit(Op::TildeExpand, 0);
            }
            ShellWord::CommandSub(inner) => {
                // $(cmd) — compile inner as a sub-chunk and emit Op::CmdSubst
                // which runs the sub-chunk on a nested VM and captures stdout.
                // The host's cmd_subst implementation handles fd plumbing.
                let sub_compiler = ShellCompiler::new();
                let sub_chunk = sub_compiler.compile(std::slice::from_ref(inner.as_ref()));
                let sub_idx = self.builder.add_sub_chunk(sub_chunk);
                self.builder.emit(Op::CmdSubst(sub_idx), 0);
            }
            ShellWord::ProcessSubIn(inner) => {
                let sub_compiler = ShellCompiler::new();
                let sub_chunk = sub_compiler.compile(std::slice::from_ref(inner.as_ref()));
                let sub_idx = self.builder.add_sub_chunk(sub_chunk);
                self.builder.emit(Op::ProcessSubIn(sub_idx), 0);
            }
            ShellWord::ProcessSubOut(inner) => {
                let sub_compiler = ShellCompiler::new();
                let sub_chunk = sub_compiler.compile(std::slice::from_ref(inner.as_ref()));
                let sub_idx = self.builder.add_sub_chunk(sub_chunk);
                self.builder.emit(Op::ProcessSubOut(sub_idx), 0);
            }
            ShellWord::ArithSub(expr) => {
                // $((expr)) — lower the arithmetic to bytecode via
                // ArithCompiler, then coerce the integer result to a string
                // (Op::Concat with an empty string forces to_str on both ops).
                let empty = self.builder.add_constant(Value::str(""));
                self.builder.emit(Op::LoadConst(empty), 0);
                self.compile_arith_inline(expr);
                self.builder.emit(Op::Concat, 0);
            }
            ShellWord::ArrayVar(name, idx_word) => {
                // ${arr[idx]} (un-braced form). Push name + expanded idx, then
                // BUILTIN_ARRAY_INDEX. Idx supports `@`/`*` (returns whole
                // array as Value::Array → flattens at exec) and integer.
                let name_const = self.builder.add_constant(Value::str(name.as_str()));
                self.builder.emit(Op::LoadConst(name_const), 0);
                self.compile_word(idx_word);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_ARRAY_INDEX, 2),
                    0,
                );
            }
            ShellWord::ArrayLiteral(_) => {
                // Bare ArrayLiteral as a word (not in assignment context) — zsh
                // permits this for some idioms but it's rare and zpwr doesn't
                // use it. Route to runtime fallback so any odd usage still
                // produces tree-walker-equivalent semantics.
                self.compile_word_runtime(word);
            }
            // ── Dynamic variants still routed through the runtime fallback ──
            // ZshFlags forms inside VariableBraced, Glob (needs Array→String
            // join). These get the JSON roundtrip until Phase G4/G5 lowers
            // them per-variant.
            _ => self.compile_word_runtime(word),
        }
    }

    /// Push a case-arm pattern as a literal string with $-variable expansion
    /// preserved but glob/tilde unexpanded. `case`/`[[ x = pat ]]` use the
    /// pattern AS the matcher — globbing the pattern itself would turn `*`
    /// into the cwd listing.
    fn compile_case_pattern(&mut self, pat: &ShellWord) {
        match pat {
            ShellWord::Literal(s) | ShellWord::SingleQuoted(s) => {
                let idx = self.builder.add_constant(Value::str(s.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
            }
            ShellWord::DoubleQuoted(parts) | ShellWord::Concat(parts) => {
                if parts.is_empty() {
                    let idx = self.builder.add_constant(Value::str(""));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    return;
                }
                self.compile_case_pattern(&parts[0]);
                for p in &parts[1..] {
                    self.compile_case_pattern(p);
                    self.builder.emit(Op::Concat, 0);
                }
            }
            ShellWord::Variable(name) => {
                let idx = self.builder.add_constant(Value::str(name.as_str()));
                self.builder.emit(Op::LoadConst(idx), 0);
                self.builder.emit(
                    Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                    0,
                );
            }
            // Less common in case arms — fall back to compile_word, accept
            // that something like `case x in $(generate_pattern)) ...` will
            // glob-expand the pattern. Real shells don't allow this anyway.
            _ => self.compile_word(pat),
        }
    }

    /// Emit a runtime expand_word call for a ShellWord variant that isn't yet
    /// lowered to native ops. Serialize the AST node to JSON, push as a
    /// constant, call `BUILTIN_EXPAND_WORD_RUNTIME` — the handler deserializes
    /// and delegates to `ShellExecutor::expand_word`, guaranteeing tree-walker
    /// parity. Phase B+ replaces individual call sites with dedicated ops.
    fn compile_word_runtime(&mut self, word: &ShellWord) {
        let json = serde_json::to_string(word).unwrap_or_default();
        let const_idx = self.builder.add_constant(Value::str(json));
        self.builder.emit(Op::LoadConst(const_idx), 0);
        self.builder.emit(
            Op::CallBuiltin(crate::exec::BUILTIN_EXPAND_WORD_RUNTIME, 1),
            0,
        );
    }

    /// True iff `s` contains only simple `$VAR` / `${VAR}` references that
    /// `compile_string_with_expansions` can handle inline. Returns false for
    /// `$(`, `$((`, `${VAR/x/y}`, `${#VAR}`, backticks — anything that needs
    /// the full `expand_string` runtime.
    fn literal_has_only_simple_vars(s: &str) -> bool {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'`' {
                return false;
            }
            if c == b'$' {
                let next = bytes.get(i + 1).copied();
                match next {
                    Some(b'(') => return false, // $( or $((
                    Some(b'{') => {
                        // Scan to matching `}`. The body chars are a mix of
                        // (1) plain identifier (allow), (2) flag form
                        // `(flags)name` (allow — compile_string_with_
                        // expansions detects and routes to PARAM_FLAG), (3)
                        // bracket form `name[idx]` (allow — same routing
                        // for ARRAY_INDEX), (4) modifier operators
                        // `:`/`#`/`%`/`/`/`!` etc. (disqualify — these need
                        // the full expansion engine via runtime fallback).
                        //
                        // If the body STARTS with `(`, skip past the matching
                        // `)` and don't disqualify on chars inside it (those
                        // are flag-modifier chars, not param-modifier ops).
                        // If the body contains `[`, allow it as the array-
                        // index path. Otherwise apply the strict disqualifier.
                        let mut j = i + 2;
                        let mut depth = 1;
                        let body_start = j;
                        let mut in_flag_paren = false;
                        let mut paren_depth = 0;
                        let mut in_brackets = false;
                        if j < bytes.len() && bytes[j] == b'(' {
                            in_flag_paren = true;
                            paren_depth = 1;
                            j += 1;
                        }
                        while j < bytes.len() && depth > 0 {
                            match bytes[j] {
                                b'{' => depth += 1,
                                b'}' => depth -= 1,
                                b'(' if in_flag_paren && paren_depth > 0 => paren_depth += 1,
                                b')' if in_flag_paren && paren_depth > 0 => {
                                    paren_depth -= 1;
                                    if paren_depth == 0 {
                                        in_flag_paren = false;
                                    }
                                }
                                b'[' if !in_flag_paren => in_brackets = true,
                                b']' if !in_flag_paren => in_brackets = false,
                                b':' | b'#' | b'%' | b'/' | b'!' | b'^' | b','
                                | b'?' | b'+' | b'=' | b'-'
                                    if !in_flag_paren && !in_brackets =>
                                {
                                    return false;
                                }
                                _ => {}
                            }
                            j += 1;
                        }
                        let _ = body_start;
                        i = j;
                        continue;
                    }
                    Some(b) if b.is_ascii_alphabetic() || b == b'_' => {
                        // Plain $VAR — fine
                        i += 2;
                        while i < bytes.len()
                            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                        {
                            i += 1;
                        }
                        continue;
                    }
                    Some(b) if b.is_ascii_digit() => {
                        // Positional `$1`, `$2`, ... — fine via GET_VAR.
                        i += 2;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                        continue;
                    }
                    Some(b'@') | Some(b'*') | Some(b'#') | Some(b'?') | Some(b'$')
                    | Some(b'!') | Some(b'-') | Some(b'_') => {
                        // Special params — handled via GET_VAR which now
                        // returns Array for `@`/`*` and the right scalar for
                        // `?`/`#`/`$`/`!`/etc.
                        i += 2;
                        continue;
                    }
                    None => {
                        // `$` at end-of-string — literal `$`. compile_
                        // string_with_expansions emits it as a constant.
                        i += 1;
                        continue;
                    }
                    _ => {
                        // Anything else after `$` — runtime fallback.
                        return false;
                    }
                }
            }
            i += 1;
        }
        true
    }

    /// Cheap pre-check for `try_lower_array_literal` so the more-careful
    /// matcher only runs when the shape is plausible.
    fn looks_like_array_literal(s: &str) -> bool {
        s.starts_with("${") && s.ends_with('}') && s.contains('[') && s.contains(']')
    }

    /// True iff the string contains a `{a,b,c}` or `{1..5}` brace-expansion
    /// pattern that should be expanded at runtime. Walks the string with a
    /// depth counter — only top-level `{…}` groups containing `..` or `,`
    /// trigger; bare `${…}` (parameter expansion) is ignored.
    fn has_brace_expansion(s: &str) -> bool {
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
                // Skip `${…}` parameter expansions — only their content
                // doesn't trigger brace expansion at the top level.
                let mut depth = 1;
                i += 2;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '{' {
                        depth += 1;
                    } else if chars[i] == '}' {
                        depth -= 1;
                    }
                    i += 1;
                }
                continue;
            }
            if chars[i] == '{' {
                // Find matching `}`; check if body has `..` or `,`.
                let mut depth = 1;
                let body_start = i + 1;
                let mut j = body_start;
                while j < chars.len() && depth > 0 {
                    if chars[j] == '{' {
                        depth += 1;
                    } else if chars[j] == '}' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    j += 1;
                }
                if j < chars.len() && depth == 0 {
                    let body: String = chars[body_start..j].iter().collect();
                    if body.contains("..") || body.contains(',') {
                        return true;
                    }
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
        false
    }

    /// Detect `pat(qualifier)` glob-qualifier shape — `*` or `?` somewhere in
    /// the string, ending with `(…)`. Tightness: the `(…)` must close at the
    /// END of the string (no trailing chars), and contain only qualifier-set
    /// chars (no `,` or `..` which would be brace-expansion).
    fn has_glob_qualifier(s: &str) -> bool {
        if !s.ends_with(')') {
            return false;
        }
        if !s.contains('*') && !s.contains('?') && !s.contains('[') {
            return false;
        }
        // Find the LAST `(` whose matching `)` is the final char.
        let bytes = s.as_bytes();
        let mut depth = 0;
        let mut start = None;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => {
                    if depth == 0 {
                        start = Some(i);
                    }
                    depth += 1;
                }
                b')' => {
                    depth -= 1;
                }
                _ => {}
            }
        }
        let start = match start {
            Some(s) => s,
            None => return false,
        };
        // The `(` must come AFTER a glob char to be a qualifier (otherwise
        // it's a literal paren or process substitution).
        let pre = &s[..start];
        let has_glob_before = pre.contains('*') || pre.contains('?') || pre.contains('[');
        if !has_glob_before {
            return false;
        }
        // The body must be all qualifier-valid chars: letters + a few syms.
        let body = &s[start + 1..s.len() - 1];
        body.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '.' | '/' | '@' | '-' | '+' | ',' | '[' | ']' | ' ')
        }) && !body.is_empty()
    }

    /// Split `pat(qualifier)` into (pattern, qualifier).
    fn split_glob_qualifier(s: &str) -> Option<(String, String)> {
        if !s.ends_with(')') {
            return None;
        }
        let bytes = s.as_bytes();
        let mut depth = 0;
        let mut start = None;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => {
                    if depth == 0 {
                        start = Some(i);
                    }
                    depth += 1;
                }
                b')' => depth -= 1,
                _ => {}
            }
        }
        let start = start?;
        Some((s[..start].to_string(), s[start + 1..s.len() - 1].to_string()))
    }

    /// Cheap pre-check for `try_lower_zsh_flag` — the `${(…)…}` shape.
    fn looks_like_zsh_flag(s: &str) -> bool {
        s.starts_with("${(") && s.ends_with('}')
    }

    /// If `s` matches `${(flags)name}`, emit BUILTIN_PARAM_FLAG with
    /// [name, flags] on stack and return true. Otherwise false. Tightness:
    /// the flags-paren must close before any other `${`/`}` appears (single-
    /// group only), and the name after the closing `)` must be a plain
    /// identifier — no nested expansions, no brackets. Composite forms like
    /// `${(j: :)$(cmd)}` (cmd-sub as the operand) hit the runtime fallback.
    fn try_lower_zsh_flag(&mut self, s: &str) -> bool {
        if !Self::looks_like_zsh_flag(s) {
            return false;
        }
        let inner = &s[2..s.len() - 1]; // strips outer `${` and `}`
        // inner must start with `(`, contain matching `)`, then a plain name.
        if !inner.starts_with('(') {
            return false;
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
        let close = match close {
            Some(i) => i,
            None => return false,
        };
        let flags = &inner[1..close];
        let name_part = &inner[close + 1..];
        // Name must be a plain identifier (no further $/braces/brackets — those
        // are runtime-fallback shapes).
        if name_part.is_empty()
            || name_part.contains('$')
            || name_part.contains('{')
            || name_part.contains('}')
            || name_part.contains('[')
        {
            return false;
        }
        if !name_part
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        {
            return false;
        }
        if !name_part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return false;
        }

        let name_const = self.builder.add_constant(Value::str(name_part));
        self.builder.emit(Op::LoadConst(name_const), 0);
        let flags_const = self.builder.add_constant(Value::str(flags));
        self.builder.emit(Op::LoadConst(flags_const), 0);
        self.builder.emit(
            Op::CallBuiltin(crate::exec::BUILTIN_PARAM_FLAG, 2),
            0,
        );
        true
    }

    /// True iff `s` contains `target` at a position NOT preceded by the `\0`
    /// quote sentinel that the lexer emits for single-quoted special chars.
    /// Used by trigger detection so `'echo $x'` (lexed as `echo \0$x`) routes
    /// to the pure-literal branch instead of the expansion branch.
    fn contains_unquoted(s: &str, target: char) -> bool {
        let mut prev = ' ';
        for c in s.chars() {
            if c == target && prev != '\x00' {
                return true;
            }
            prev = c;
        }
        false
    }

    /// Strip the lexer's `\0` quote sentinels from a string. The convention is
    /// `\0X` where X ∈ {$, `, (, )} means "literal X". After this function the
    /// `\0` markers are gone and the string is ready to ship as a constant.
    fn strip_quote_markers(s: &str) -> String {
        if !s.contains('\x00') {
            return s.to_string();
        }
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            if c != '\x00' {
                out.push(c);
            }
        }
        out
    }

    fn compile_string_with_expansions(&mut self, s: &str) {
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        let mut first = true;
        while i < chars.len() {
            // Honor the `\0X` quote sentinel: if the previous char is the
            // marker, the next char (including `$`) is literal — skip the
            // marker and append the next char verbatim. Without this guard,
            // `'echo $x'` (lexed as `echo \0$x`) would treat `$x` as a
            // parameter expansion at compile time.
            if chars[i] == '\x00' {
                if i + 1 < chars.len() {
                    let mut literal = String::new();
                    literal.push(chars[i + 1]);
                    i += 2;
                    while i < chars.len() && chars[i] != '$' && chars[i] != '\x00' {
                        literal.push(chars[i]);
                        i += 1;
                    }
                    let idx = self.builder.add_constant(Value::str(&literal));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    if !first {
                        self.builder.emit(Op::Concat, 0);
                    }
                    first = false;
                    continue;
                }
                i += 1;
                continue;
            }
            if chars[i] == '$' {
                i += 1;
                if i >= chars.len() {
                    let idx = self.builder.add_constant(Value::str("$"));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    if !first {
                        self.builder.emit(Op::Concat, 0);
                    }
                    first = false;
                    continue;
                }
                if chars[i] == '{' {
                    i += 1;
                    let mut var_name = String::new();
                    while i < chars.len() && chars[i] != '}' {
                        var_name.push(chars[i]);
                        i += 1;
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                    // `${(flag)name}` and `${name[idx]}` shapes need the
                    // dedicated PARAM_FLAG / ARRAY_INDEX lowerings — GET_VAR
                    // would look up a literal "(flag)name" key. Synthesize
                    // the full ${...} string and run the same matchers used
                    // by the Literal-word path.
                    if var_name.starts_with('(') {
                        let synthetic = format!("${{{}}}", var_name);
                        if Self::looks_like_zsh_flag(synthetic.as_str())
                            && self.try_lower_zsh_flag(synthetic.as_str())
                        {
                            if !first {
                                self.builder.emit(Op::Concat, 0);
                            }
                            first = false;
                            continue;
                        }
                    }
                    if var_name.contains('[') && var_name.ends_with(']') {
                        let synthetic = format!("${{{}}}", var_name);
                        if Self::looks_like_array_literal(synthetic.as_str())
                            && self.try_lower_array_literal(synthetic.as_str())
                        {
                            if !first {
                                self.builder.emit(Op::Concat, 0);
                            }
                            first = false;
                            continue;
                        }
                    }
                    // Route through BUILTIN_GET_VAR for consistent storage.
                    let name_const = self.builder.add_constant(Value::str(var_name.as_str()));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                        0,
                    );
                    if !first {
                        self.builder.emit(Op::Concat, 0);
                    }
                    first = false;
                } else if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
                    let mut var_name = String::new();
                    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                        var_name.push(chars[i]);
                        i += 1;
                    }
                    // Route through BUILTIN_GET_VAR for consistent storage.
                    let name_const = self.builder.add_constant(Value::str(var_name.as_str()));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                        0,
                    );
                    if !first {
                        self.builder.emit(Op::Concat, 0);
                    }
                    first = false;
                } else if chars[i].is_ascii_digit() {
                    // Positional `$1`, `$2`, ... — single digit is canonical;
                    // multi-digit needs `${10}` form. Push the number as the
                    // var name to GET_VAR which knows how to look up
                    // positional params.
                    let mut var_name = String::new();
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        var_name.push(chars[i]);
                        i += 1;
                    }
                    let name_const = self.builder.add_constant(Value::str(var_name.as_str()));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                        0,
                    );
                    if !first {
                        self.builder.emit(Op::Concat, 0);
                    }
                    first = false;
                } else if matches!(
                    chars[i],
                    '@' | '*' | '#' | '?' | '$' | '!' | '-' | '_'
                ) {
                    // Special param. Pass the single char as the name.
                    let var_name = chars[i].to_string();
                    i += 1;
                    let name_const = self.builder.add_constant(Value::str(var_name.as_str()));
                    self.builder.emit(Op::LoadConst(name_const), 0);
                    self.builder.emit(
                        Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                        0,
                    );
                    if !first {
                        self.builder.emit(Op::Concat, 0);
                    }
                    first = false;
                } else {
                    let idx = self.builder.add_constant(Value::str("$"));
                    self.builder.emit(Op::LoadConst(idx), 0);
                    if !first {
                        self.builder.emit(Op::Concat, 0);
                    }
                    first = false;
                }
            } else {
                let mut literal = String::new();
                while i < chars.len() && chars[i] != '$' && chars[i] != '\x00' {
                    literal.push(chars[i]);
                    i += 1;
                }
                let idx = self.builder.add_constant(Value::str(&literal));
                self.builder.emit(Op::LoadConst(idx), 0);
                if !first {
                    self.builder.emit(Op::Concat, 0);
                }
                first = false;
            }
        }
        if first {
            let idx = self.builder.add_constant(Value::str(""));
            self.builder.emit(Op::LoadConst(idx), 0);
        }
    }

    /// Shared implementation for while/until loops.
    fn compile_while_loop(
        &mut self,
        condition: &[ShellCommand],
        body: &[ShellCommand],
        is_until: bool,
    ) {
        // Track the body's last-command exit status in a slot. POSIX: the
        // loop's exit status is the body's last-command status, or 0 if the
        // body never ran. Without this, the status would be the failing
        // condition's status (1 for `while [[ i < N ]]` after exit).
        let status_slot = self.next_slot;
        self.next_slot += 1;
        self.builder.emit(Op::LoadInt(0), 0);
        self.builder.emit(Op::SetSlot(status_slot), 0);

        let loop_top = self.builder.current_pos();

        // Evaluate condition
        for cmd in condition {
            self.compile_command(cmd);
        }
        self.builder.emit(Op::GetStatus, 0);

        // Shell: status 0 = success = truthy.
        //   while → exit when condition FAILS → JumpIfFalse on Status truthiness
        //   until → exit when condition SUCCEEDS → JumpIfTrue
        let exit_jump = if is_until {
            self.builder.emit(Op::JumpIfTrue(0), 0)
        } else {
            self.builder.emit(Op::JumpIfFalse(0), 0)
        };

        self.break_patches.push(Vec::new());
        self.continue_patches.push(Vec::new());

        for cmd in body {
            self.compile_command(cmd);
        }

        // continue lands here — patch all in-body `continue` jumps to this
        // point, then save body status + back-jump.
        let continue_target = self.builder.current_pos();
        if let Some(continues) = self.continue_patches.pop() {
            for cp in continues {
                self.builder.patch_jump(cp, continue_target);
            }
        }

        // Save body's last-command status before the back-jump.
        self.builder.emit(Op::GetStatus, 0);
        self.builder.emit(Op::SetSlot(status_slot), 0);
        self.builder.emit(Op::Jump(loop_top), 0);

        let loop_exit = self.builder.current_pos();
        self.builder.patch_jump(exit_jump, loop_exit);

        // Restore the saved body status as the loop's exit status.
        self.builder.emit(Op::GetSlot(status_slot), 0);
        self.builder.emit(Op::SetStatus, 0);

        if let Some(breaks) = self.break_patches.pop() {
            for bp in breaks {
                self.builder.patch_jump(bp, loop_exit);
            }
        }
    }

    /// Extract a literal string from a ShellWord (for constant folding).
    /// Compile an arithmetic expression inline, emitting ops directly
    /// into this compiler's builder. Result is left on the stack.
    /// Variables are mapped into the parent's slot table so `i` in
    /// init/cond/step/body all resolve to the same slot.
    pub fn compile_arith_inline(&mut self, expr: &str) {
        let mut ac = ArithCompiler::new(expr);
        // Share the parent's slot table
        ac.slots = self.slots.clone();
        ac.next_slot = self.next_slot;

        // Pre-load: any var the arith expression touches needs its current
        // value pulled from executor.variables into the slot. Without this,
        // `i=5; (( i+1 ))` would read 0 because the slot starts uninitialized.
        // We do this BEFORE the arith ops by scanning the source for
        // identifiers and pre-allocating slots.
        let pre_load_names = ac.collect_identifiers(expr);
        for name in &pre_load_names {
            let slot = ac.slot_for(name);
            // Load var name via BUILTIN_GET_VAR, parse to int, store to slot.
            let name_const = ac.builder.add_constant(Value::str(name.as_str()));
            ac.builder.emit(Op::LoadConst(name_const), 0);
            ac.builder.emit(
                Op::CallBuiltin(crate::exec::BUILTIN_GET_VAR, 1),
                0,
            );
            // Coerce to int via Concat-with-empty + arith — fusevm's NumLt
            // and similar ops already coerce strings, but we want the slot
            // to hold an Int. Arith ops accept Str or Int so just store.
            ac.builder.emit(Op::SetSlot(slot), 0);
        }

        ac.expr();
        let new_slots = ac.slots.clone();
        let new_next = ac.next_slot;
        let chunk = ac.builder.build();

        // Merge any new slots back
        self.slots = new_slots.clone();
        self.next_slot = new_next;

        // Inline the computation ops
        for op in &chunk.ops {
            self.builder.emit(op.clone(), 0);
        }

        // Post-sync: for every var the arith expression mentioned, write the
        // slot's value back to executor.variables via BUILTIN_SET_VAR. This
        // makes `(( i++ ))` visible to subsequent `echo $i` and to the loop's
        // own conditional check.
        //
        // Result of the arith expression is on top of the stack — preserve
        // it by capturing into a temp slot, syncing, then restoring.
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

    fn word_to_string(&self, word: &ShellWord) -> String {
        match word {
            ShellWord::Literal(s) => s.clone(),
            ShellWord::SingleQuoted(s) => s.clone(),
            _ => String::new(), // dynamic words can't be const-folded
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use fusevm::{VMResult, VM};

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
            body: vec![ShellCommand::Compound(CompoundCommand::Arith(
                "sum = sum + i".to_string(),
            ))],
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
                vec![ShellCommand::Compound(CompoundCommand::Arith(
                    "1".to_string(),
                ))],
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
        // echo hello world → CallBuiltin(BUILTIN_ECHO, 2) since echo is a builtin
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
        // echo is a builtin, should emit CallBuiltin not Exec
        let has_builtin = chunk
            .ops
            .iter()
            .any(|op| matches!(op, Op::CallBuiltin(2, 2))); // BUILTIN_ECHO=2, 2 args
        assert!(
            has_builtin,
            "expected CallBuiltin(2, 2) for 'echo hello world', got: {:?}",
            chunk.ops
        );
    }

    #[test]
    fn test_external_command_compiles() {
        use crate::parser::SimpleCommand;
        // ls -la → Exec(2) since ls is NOT a builtin
        let cmd = ShellCommand::Simple(SimpleCommand {
            assignments: vec![],
            words: vec![
                ShellWord::Literal("ls".to_string()),
                ShellWord::Literal("-la".to_string()),
            ],
            redirects: vec![],
        });
        let compiler = ShellCompiler::new();
        let chunk = compiler.compile(&[cmd]);
        let has_exec = chunk.ops.iter().any(|op| matches!(op, Op::Exec(2)));
        assert!(
            has_exec,
            "expected Exec(2) for 'ls -la', got: {:?}",
            chunk.ops
        );
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
        let has_begin = chunk
            .ops
            .iter()
            .any(|op| matches!(op, Op::PipelineBegin(2)));
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
                    vec![ShellCommand::Compound(CompoundCommand::Arith(
                        "result = 1".to_string(),
                    ))],
                    CaseTerminator::Break,
                ),
                (
                    vec![ShellWord::Literal("world".to_string())],
                    vec![ShellCommand::Compound(CompoundCommand::Arith(
                        "result = 2".to_string(),
                    ))],
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
            Box::new(CondExpr::FileRegular(ShellWord::Literal(
                "/etc/passwd".to_string(),
            ))),
            Box::new(CondExpr::FileDirectory(ShellWord::Literal(
                "/tmp".to_string(),
            ))),
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
        assert!(
            !chunk.sub_entries.is_empty(),
            "expected sub entry for function"
        );
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
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::FileExists(
            ShellWord::Literal("/tmp".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0, "/tmp should exist");
    }

    #[test]
    fn test_exec_file_test_not_exists() {
        use crate::parser::CompoundCommand;
        // [[ -e /nonexistent_path_xyz ]] → status 1
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::FileExists(
            ShellWord::Literal("/nonexistent_path_xyz".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1, "/nonexistent should not exist");
    }

    #[test]
    fn test_exec_file_is_dir() {
        use crate::parser::CompoundCommand;
        // [[ -d /tmp ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::FileDirectory(
            ShellWord::Literal("/tmp".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0, "/tmp should be a directory");
    }

    #[test]
    fn test_exec_file_is_regular() {
        use crate::parser::CompoundCommand;
        // [[ -f /etc/hosts ]] → status 0 (exists on macOS/Linux)
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::FileRegular(
            ShellWord::Literal("/etc/hosts".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0, "/etc/hosts should be a regular file");
    }

    #[test]
    fn test_exec_string_equal() {
        use crate::parser::CompoundCommand;
        // [[ "abc" == "abc" ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::StringEqual(
            ShellWord::Literal("abc".to_string()),
            ShellWord::Literal("abc".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_exec_string_not_equal() {
        use crate::parser::CompoundCommand;
        // [[ "abc" == "xyz" ]] → status 1
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::StringEqual(
            ShellWord::Literal("abc".to_string()),
            ShellWord::Literal("xyz".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }

    #[test]
    fn test_exec_string_empty() {
        use crate::parser::CompoundCommand;
        // [[ -z "" ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::StringEmpty(
            ShellWord::Literal("".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);

        // [[ -z "notempty" ]] → status 1
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::StringEmpty(
            ShellWord::Literal("notempty".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }

    #[test]
    fn test_exec_cond_and() {
        use crate::parser::CompoundCommand;
        // [[ -d /tmp && -e /tmp ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::And(
            Box::new(CondExpr::FileDirectory(ShellWord::Literal(
                "/tmp".to_string(),
            ))),
            Box::new(CondExpr::FileExists(ShellWord::Literal("/tmp".to_string()))),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_exec_cond_and_short_circuit() {
        use crate::parser::CompoundCommand;
        // [[ -f /nonexistent && -d /tmp ]] → status 1 (short-circuits on first)
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::And(
            Box::new(CondExpr::FileRegular(ShellWord::Literal(
                "/nonexistent".to_string(),
            ))),
            Box::new(CondExpr::FileDirectory(ShellWord::Literal(
                "/tmp".to_string(),
            ))),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 1);
    }

    #[test]
    fn test_exec_cond_or() {
        use crate::parser::CompoundCommand;
        // [[ -f /nonexistent || -d /tmp ]] → status 0 (second is true)
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::Or(
            Box::new(CondExpr::FileRegular(ShellWord::Literal(
                "/nonexistent".to_string(),
            ))),
            Box::new(CondExpr::FileDirectory(ShellWord::Literal(
                "/tmp".to_string(),
            ))),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);
    }

    #[test]
    fn test_exec_cond_not() {
        use crate::parser::CompoundCommand;
        // [[ ! -f /nonexistent ]] → status 0
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::Not(Box::new(
            CondExpr::FileRegular(ShellWord::Literal("/nonexistent".to_string())),
        ))));
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
                vec![ShellCommand::Compound(CompoundCommand::Arith(
                    "1".to_string(),
                ))],
                vec![ShellCommand::Compound(CompoundCommand::Arith(
                    "result = 42".to_string(),
                ))],
            )],
            else_part: Some(vec![ShellCommand::Compound(CompoundCommand::Arith(
                "result = 99".to_string(),
            ))]),
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
                vec![ShellCommand::Compound(CompoundCommand::Arith(
                    "0".to_string(),
                ))],
                vec![ShellCommand::Compound(CompoundCommand::Arith(
                    "result = 42".to_string(),
                ))],
            )],
            else_part: Some(vec![ShellCommand::Compound(CompoundCommand::Arith(
                "result = 99".to_string(),
            ))]),
        });
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0); // (( 99 )) is truthy → status 0
    }

    #[test]
    fn test_exec_numeric_comparison() {
        use crate::parser::CompoundCommand;
        // [[ 5 -gt 3 ]] → true
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::NumGreater(
            ShellWord::Literal("5".to_string()),
            ShellWord::Literal("3".to_string()),
        )));
        let vm = compile_and_run(&[cmd]);
        assert_eq!(vm.last_status, 0);

        // [[ 2 -gt 3 ]] → false
        let cmd = ShellCommand::Compound(CompoundCommand::Cond(CondExpr::NumGreater(
            ShellWord::Literal("2".to_string()),
            ShellWord::Literal("3".to_string()),
        )));
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
