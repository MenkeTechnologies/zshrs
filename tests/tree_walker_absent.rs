//! Source-level proof that the tree-walker AST-dispatch functions are gone.
//!
//! Phase F deleted `execute_simple`, `execute_pipeline`, `execute_list`,
//! `execute_compound`, and `execute_command_bg` from `src/exec.rs`. These were
//! the recursive AST-walking functions that pattern-matched `ShellCommand`
//! variants and called themselves on sub-commands. Their existence in the
//! source would mean an interpreter is still present — even if currently
//! unreachable from `execute_command`. We assert literal absence here so a
//! future refactor can't silently bring them back.
//!
//! What we *don't* assert dead:
//!   - `execute_command` itself: kept as a public entry point that compiles to
//!     a fusevm `Chunk` and runs on the bytecode VM. The body must contain a
//!     `ShellCompiler` invocation; if it goes back to a `match cmd`-style
//!     dispatch, the `bytecode_dispatch_signature` test fails.
//!   - `execute_command_substitution` / `execute_command_capture`: kept, but
//!     bytecode-routed (compile + run + pipe-capture stdout). Asserted to use
//!     `ShellCompiler` and `os_pipe`, not a hand-rolled match dispatch.
//!   - `execute_external` / `execute_external_bg`: these are the actual
//!     fork-exec mechanism — *not* a tree walker. They're the leaf operation
//!     a bytecode `Op::Exec` should ultimately route to.

use std::fs;
use std::path::PathBuf;

fn read_exec_rs() -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("src/exec.rs");
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {}", p.display(), e))
}

fn assert_no_fn(src: &str, name: &str) {
    // Match `fn NAME(` and `pub fn NAME(` at any indent. The `(` rules out
    // accidental matches in comments/strings that mention the name.
    let needles = [format!("fn {}(", name), format!("pub fn {}(", name)];
    for needle in &needles {
        assert!(
            !src.contains(needle.as_str()),
            "src/exec.rs still defines `{}` — tree-walker dispatch must be gone",
            name
        );
    }
}

#[test]
fn execute_simple_is_deleted() {
    assert_no_fn(&read_exec_rs(), "execute_simple");
}

#[test]
fn execute_pipeline_is_deleted() {
    assert_no_fn(&read_exec_rs(), "execute_pipeline");
}

#[test]
fn execute_list_is_deleted() {
    assert_no_fn(&read_exec_rs(), "execute_list");
}

#[test]
fn execute_compound_is_deleted() {
    assert_no_fn(&read_exec_rs(), "execute_compound");
}

#[test]
fn execute_command_bg_is_deleted() {
    assert_no_fn(&read_exec_rs(), "execute_command_bg");
}

#[test]
fn execute_command_dispatches_via_compiler() {
    // The execute_command body must compile a chunk and run it on a VM —
    // primary path through `ZshParser` + `ZshCompiler` (the ported pipeline),
    // with `ShellCompiler` as the fallback when re-parse of the
    // round-tripped source fails. If a future refactor reintroduces a
    // `match cmd { Simple => ...` tree-walker dispatch, this test fails.
    let src = read_exec_rs();
    let entry = src
        .find("pub fn execute_command(&mut self, cmd: &ShellCommand)")
        .expect("execute_command entry point missing");
    let window = &src[entry..entry + 1800];
    assert!(
        window.contains("ZshParser::new") && window.contains("ZshCompiler::new()"),
        "execute_command must compile via ZshParser+ZshCompiler (primary):\n{}",
        window
    );
    assert!(
        window.contains("ShellCompiler::new()"),
        "execute_command must keep a ShellCompiler fallback so unparseable round-trip text still runs:\n{}",
        window
    );
    assert!(
        window.contains("fusevm::VM::new"),
        "execute_command must run the chunk on a fusevm VM:\n{}",
        window
    );
    assert!(
        !window.contains("ShellCommand::Simple(simple) => self.execute_simple"),
        "execute_command must not match-dispatch to a tree walker"
    );
}

#[test]
fn execute_command_substitution_uses_pipe_capture() {
    // The capture machinery: compile (via ZshCompiler primary, ShellCompiler
    // fallback) → dup2 stdout to a pipe → run VM → read pipe. The old version
    // had a giant `match cmd { Simple => ... }` with hand-rolled
    // `echo`/`printf`/`pwd` shortcuts.
    let src = read_exec_rs();
    let entry = src
        .find("pub fn execute_command_capture")
        .expect("execute_command_capture missing");
    let window = &src[entry..entry + 2200];
    assert!(
        window.contains("ZshParser::new") && window.contains("ZshCompiler::new()"),
        "capture must compile via ZshParser+ZshCompiler (primary)"
    );
    assert!(
        window.contains("ShellCompiler::new()"),
        "capture must keep a ShellCompiler fallback for unparseable round-trip text"
    );
    assert!(
        window.contains("os_pipe::pipe()"),
        "capture must use os_pipe for stdout redirection"
    );
    assert!(
        !window.contains("\"echo\" =>"),
        "capture must not have hand-rolled `echo` shortcut"
    );
    assert!(
        !window.contains("\"printf\" =>"),
        "capture must not have hand-rolled `printf` shortcut"
    );
    assert!(
        !window.contains("\"pwd\" =>"),
        "capture must not have hand-rolled `pwd` shortcut"
    );
}

#[test]
fn no_match_shellcommand_outside_compiler() {
    // The only place a `match cmd {` over `ShellCommand` variants should now
    // appear is in `src/shell_compiler.rs` (the lowering). `src/exec.rs`
    // should not contain such a match — that would be a tree walker.
    let src = read_exec_rs();
    // Strict check: the specific pattern that used to appear in execute_command
    // is `ShellCommand::Simple(simple) => self.execute_simple(simple)`. Even if
    // a future refactor doesn't use that exact name, dispatching by ShellCommand
    // variant from exec.rs is a regression.
    assert!(
        !src.contains("ShellCommand::Simple(simple) => self.execute_simple"),
        "tree-walker dispatch pattern resurfaced in exec.rs"
    );
    assert!(
        !src.contains("ShellCommand::Compound(compound) => self.execute_compound"),
        "tree-walker dispatch pattern resurfaced in exec.rs"
    );
    assert!(
        !src.contains("ShellCommand::List(items) => self.execute_list"),
        "tree-walker dispatch pattern resurfaced in exec.rs"
    );
}
