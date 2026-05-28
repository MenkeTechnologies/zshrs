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

#[cfg(test)]
mod tests {
    use super::*;
    use fusevm::Chunk;

    // === append_chunk: pure-compute disassembly formatting ===

    fn empty_chunk() -> Chunk {
        Chunk::default()
    }

    #[test]
    fn append_empty_chunk_writes_nothing() {
        // Default Chunk has empty source, names, ops, sub_entries,
        // sub_chunks — no lines should be emitted at all.
        let mut buf = String::new();
        append_chunk(&mut buf, &empty_chunk(), "");
        assert_eq!(buf, "", "empty chunk produces empty output");
    }

    #[test]
    fn append_chunk_emits_source_comment_when_present() {
        let mut c = empty_chunk();
        c.source = "myfile.zsh".to_string();
        let mut buf = String::new();
        append_chunk(&mut buf, &c, "");
        assert_eq!(buf, "; source: myfile.zsh\n");
    }

    #[test]
    fn append_chunk_skips_source_line_when_empty() {
        // Empty source string must NOT emit a "; source: " line —
        // saves a noise line in listings of synthetic chunks.
        let c = empty_chunk();
        assert!(c.source.is_empty());
        let mut buf = String::new();
        append_chunk(&mut buf, &c, "");
        assert!(!buf.contains("; source:"));
    }

    #[test]
    fn append_chunk_writes_each_name_with_index() {
        let mut c = empty_chunk();
        c.names = vec!["foo".to_string(), "bar".to_string(), "baz".to_string()];
        let mut buf = String::new();
        append_chunk(&mut buf, &c, "");
        assert!(buf.contains("; name[0] = foo"));
        assert!(buf.contains("; name[1] = bar"));
        assert!(buf.contains("; name[2] = baz"));
    }

    #[test]
    fn append_chunk_applies_indent_to_every_line() {
        let mut c = empty_chunk();
        c.source = "x.zsh".to_string();
        c.names = vec!["n".to_string()];
        let mut buf = String::new();
        append_chunk(&mut buf, &c, ">>>>");
        for line in buf.lines() {
            assert!(
                line.starts_with(">>>>"),
                "every line must carry the indent prefix, got {line:?}"
            );
        }
    }

    #[test]
    fn append_chunk_sub_entries_section_only_when_nonempty() {
        let mut c = empty_chunk();
        // Empty sub_entries → no header.
        let mut buf = String::new();
        append_chunk(&mut buf, &c, "");
        assert!(!buf.contains("; sub_entries:"));

        // With one entry → header AND the entry line.
        c.names = vec!["myfn".to_string()];
        c.sub_entries = vec![(0u16, 42usize)];
        let mut buf = String::new();
        append_chunk(&mut buf, &c, "");
        assert!(buf.contains("; sub_entries:"));
        assert!(buf.contains(";   myfn @ 42"));
    }

    #[test]
    fn append_chunk_sub_entry_unknown_name_index_shows_question_mark() {
        // sub_entries name-index out-of-bounds → format shows "?" so
        // a corrupt chunk doesn't panic the disassembler.
        let mut c = empty_chunk();
        c.sub_entries = vec![(99u16, 17usize)]; // names is empty
        let mut buf = String::new();
        append_chunk(&mut buf, &c, "");
        assert!(buf.contains(";   ? @ 17"));
    }

    #[test]
    fn append_chunk_ops_are_numbered_four_digit_width() {
        // Format: `{i:04} {line:>5}     {op:?}` — instruction index
        // pads to 4 digits so listings align cleanly.
        let mut c = empty_chunk();
        c.ops = vec![fusevm::Op::Nop, fusevm::Op::Nop, fusevm::Op::Nop];
        c.lines = vec![1, 2, 3];
        let mut buf = String::new();
        append_chunk(&mut buf, &c, "");
        assert!(buf.contains("0000     1     Nop"));
        assert!(buf.contains("0001     2     Nop"));
        assert!(buf.contains("0002     3     Nop"));
    }

    #[test]
    fn append_chunk_ops_default_line_zero_when_lines_short() {
        // If `chunk.lines` is shorter than `chunk.ops`, missing entries
        // default to 0 — fallback for old chunk formats.
        let mut c = empty_chunk();
        c.ops = vec![fusevm::Op::Nop, fusevm::Op::Nop];
        c.lines = vec![5]; // only one line entry for two ops
        let mut buf = String::new();
        append_chunk(&mut buf, &c, "");
        assert!(buf.contains("0000     5     Nop"));
        assert!(
            buf.contains("0001     0     Nop"),
            "missing line entry must default to 0"
        );
    }

    #[test]
    fn append_chunk_recurses_into_sub_chunks_with_increased_indent() {
        let mut outer = empty_chunk();
        let mut inner = empty_chunk();
        inner.source = "inner.zsh".to_string();
        outer.sub_chunks = vec![inner];
        let mut buf = String::new();
        append_chunk(&mut buf, &outer, "");
        // Header for the sub-chunk:
        assert!(buf.contains("; --- sub_chunk[0] ---"));
        // Inner chunk's source line is indented two spaces deeper:
        assert!(
            buf.contains("  ; source: inner.zsh"),
            "inner chunk should be indented 2 spaces deeper"
        );
    }

    #[test]
    fn append_chunk_nested_sub_chunks_compound_indent() {
        // Two-level nesting: indent should grow by 2 spaces per level.
        let mut deepest = empty_chunk();
        deepest.source = "deep.zsh".to_string();
        let mut middle = empty_chunk();
        middle.sub_chunks = vec![deepest];
        let mut outer = empty_chunk();
        outer.sub_chunks = vec![middle];

        let mut buf = String::new();
        append_chunk(&mut buf, &outer, "");
        // After two levels of recursion the prefix is "    " (4 spaces).
        assert!(
            buf.contains("    ; source: deep.zsh"),
            "two-level nested chunk should be indented 4 spaces:\n{buf}"
        );
    }

    // === set_enabled / ENABLED atomic flag ===

    #[test]
    fn set_enabled_toggles_atomic_flag() {
        // The flag controls whether `maybe_print_stdout` emits — verify
        // both store directions are observable. Restore the previous
        // value to keep test isolation (no global leak across tests).
        let prev = ENABLED.load(Ordering::Relaxed);
        set_enabled(true);
        assert!(ENABLED.load(Ordering::Relaxed));
        set_enabled(false);
        assert!(!ENABLED.load(Ordering::Relaxed));
        // Restore.
        ENABLED.store(prev, Ordering::Relaxed);
    }
}
