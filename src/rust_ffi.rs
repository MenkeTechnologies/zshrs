//! zsh wiring for inline Rust FFI (`rust { ... }` blocks).
//!
//! fusevm does the heavy lifting: [`fusevm::RustSugar`] rewrites the block at
//! the source level and [`fusevm::ffi`] compiles/loads/marshals it. This module
//! only supplies the zsh-flavored [`fusevm::RustSugar`] config and the desugar
//! entry the parser calls before lexing. The emitted `__rust_compile` command
//! and every exported bareword are resolved elsewhere:
//!
//! - `__rust_compile` is a zshrs builtin (`bin_rust_compile`, src/ported/
//!   builtin.rs) that calls [`fusevm::ffi::compile_and_register`].
//! - An exported name runs as a command via the FFI fallback in
//!   `ShellExecutor::try_registered_ffi_command` (src/vm_helper.rs), consulted
//!   only after builtins/functions/`$PATH`/`command_not_found_handler` all miss
//!   — real commands keep priority.

use fusevm::RustSugar;

/// Emit the zsh command a `rust { ... }` block desugars to: the `__rust_compile`
/// builtin carrying the base64-encoded block body and the block's source line.
/// The body is single-quoted, but the base64 alphabet (`A–Z a–z 0–9 + / =`)
/// contains no zsh-special character, so the quoting is belt-and-suspenders.
fn emit(b64: &str, line: usize) -> String {
    format!("__rust_compile '{b64}' {line}")
}

/// zsh desugar config: the `rust` keyword, `#` line comments, no block comment,
/// newline as a command boundary. zsh already uses `{ }` for grouping and
/// `;`/newline/`{`/`}` as command separators; the rewrite only fires on the
/// `rust` keyword immediately followed by `{` at a command boundary, so an
/// ordinary `{ ... }` group — or a bare `rust` command word — is never touched.
pub const SUGAR: RustSugar = RustSugar {
    keyword: "rust",
    line_comments: &["#"],
    block_comment: None,
    newline_boundary: true,
    emit,
};

/// Rewrite every `rust { ... }` block at a command boundary into a
/// `__rust_compile '<base64>' <line>` command, before lexing. No-op (single
/// substring scan) when the source has no `rust` token.
pub fn desugar(src: &str) -> String {
    SUGAR.desugar(src)
}

/// The `__rust_compile '<base64>' [line]` builtin handler — the target of the
/// desugar above (registered in the `BUILTINS` table, src/ported/builtin.rs).
/// It compiles the base64-encoded `rust { ... }` block body into a cached cdylib
/// and registers its exported `extern "C"` functions so each becomes callable
/// as a zshrs command (resolved in `ShellExecutor::try_registered_ffi_command`,
/// src/vm_helper.rs). `argv[0]` is the base64 body; `argv[1]` (the block's
/// source line) is carried for diagnostics but not yet consumed. Returns 0 on
/// success, or 1 with a `zshrs: <err>` diagnostic on failure.
///
/// Lives here rather than under `src/ported/` because it is a zshrs-native
/// builtin with no zsh C counterpart — the port-fidelity guard (build.rs)
/// requires every fn under `src/ported/` to map to an upstream C function.
pub fn bin_rust_compile(
    _name: &str,
    argv: &[String],
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    let Some(b64) = argv.first() else {
        eprintln!("zshrs: __rust_compile: missing block body");
        return 1;
    };
    match fusevm::ffi::compile_and_register(b64) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("zshrs: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn desugars_rust_block_at_boundary() {
        let src =
            "rust { pub extern \"C\" fn add(a: i64, b: i64) -> i64 { a + b } }\nadd 21 21\n";
        let out = super::desugar(src);
        assert!(out.contains("__rust_compile '"), "no builtin call: {out}");
        assert!(!out.contains("pub extern"), "Rust body leaked: {out}");
        assert!(out.contains("add 21 21"), "trailing command dropped: {out}");
    }

    #[test]
    fn leaves_ordinary_brace_group_untouched() {
        // A normal zsh `{ ... }` group (no `rust` keyword) must pass through
        // byte-for-byte — the desugar only triggers on `rust {`.
        let src = "{ echo hi; echo bye }\n";
        assert_eq!(super::desugar(src), src);
    }

    #[test]
    fn leaves_bare_rust_word_untouched() {
        // `rust` as an ordinary command word (not immediately followed by `{`)
        // is left alone, so an external `rust` binary still runs.
        let src = "rust --version\n";
        assert_eq!(super::desugar(src), src);
    }
}
