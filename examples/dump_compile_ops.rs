//! Diagnostic dump: lex + parse + compile a handful of zsh source
//! snippets and print the resulting fusevm `Chunk.ops` to stderr.
//!
//! Was `compile_zsh::tests::dump_ops_for_failing_constructs` but it
//! has no assertions — it's a developer-facing trace dump that gets
//! invoked when investigating why a specific zsh construct produces
//! the wrong bytecode. Moved here so `cargo test` doesn't surface it
//! as an "ignored" item every run.
//!
//! Usage: `cargo run --example dump_compile_ops`

use zsh::compile_zsh::ZshCompiler;
use zsh::ported::{parse, utils};
use zsh::zsh_h::ERRFLAG_ERROR;

fn compile_src(src: &str) -> fusevm::Chunk {
    // No global_state_lock here — this is a standalone example, not
    // a parallel test; the lock module is `#[cfg(test)]` and isn't
    // visible from the `examples/` build.
    use std::sync::atomic::Ordering;
    let saved = utils::errflag.load(Ordering::Relaxed);
    utils::errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
    parse::parse_init(src);
    let program = parse::parse();
    utils::errflag.store(saved, Ordering::Relaxed);
    ZshCompiler::new().compile(&program)
}

fn main() {
    for src in [
        "$(echo hi)",
        "greet() { echo hi; }",
        "echo *.txt",
        "cat <<EOF\nhi\nEOF\n",
        "true && echo a",
        "false || echo a",
        "echo $HOME",
        "echo ~/x",
    ] {
        let chunk = compile_src(src);
        eprintln!("=== src: {src:?} ===");
        for (i, op) in chunk.ops.iter().enumerate() {
            eprintln!("  [{i:3}] {op:?}");
        }
        for (i, sc) in chunk.sub_chunks.iter().enumerate() {
            eprintln!("  sub_chunk[{i}] ops={:?}", sc.ops);
        }
    }
}
