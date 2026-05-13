//! Optional fusevm bytecode listing to stdout when `zshrs` is invoked with `--disasm`.

use fusevm::Chunk;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Set from `bins/zshrs` after argv normalization (consumed from argv before `-c` / script dispatch).
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// If `--disasm` was passed on the zshrs CLI, print a listing to stdout before `VM::run`.
pub fn maybe_print_stdout(context: &str, chunk: &Chunk) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let mut buf = String::new();
    let _ = writeln!(buf, "; zshrs fusevm — {context}");
    append_chunk(&mut buf, chunk, "");
    print!("{buf}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

fn append_chunk(out: &mut String, chunk: &Chunk, indent: &str) {
    if !chunk.source.is_empty() {
        let _ = writeln!(out, "{indent}; source: {}", chunk.source);
    }
    for (i, n) in chunk.names.iter().enumerate() {
        let _ = writeln!(out, "{indent}; name[{i}] = {n}");
    }
    if !chunk.sub_entries.is_empty() {
        let _ = writeln!(out, "{indent}; sub_entries:");
        for (ni, ip) in &chunk.sub_entries {
            let name = chunk
                .names
                .get(*ni as usize)
                .map(String::as_str)
                .unwrap_or("?");
            let _ = writeln!(out, "{indent};   {name} @ {ip}");
        }
    }
    for (i, op) in chunk.ops.iter().enumerate() {
        let line = chunk.lines.get(i).copied().unwrap_or(0);
        let _ = writeln!(out, "{indent}{i:04} {line:>5}     {op:?}");
    }
    for (si, sub) in chunk.sub_chunks.iter().enumerate() {
        let _ = writeln!(out, "{indent}; --- sub_chunk[{si}] ---");
        let sub_indent = format!("{indent}  ");
        append_chunk(out, sub, &sub_indent);
    }
}
