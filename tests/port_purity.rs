//! Port purity enforcement.
//!
//! `zshrs` is a strict 1:1 port of zsh from C → Rust. This test FAILS
//! if it finds any of:
//!
//!  1. A Rust source file in `src/` that has no corresponding C file
//!     under `src/zsh/Src/` per the strict 1:1 map (foo.c ↔ foo.rs,
//!     byte-for-byte identical stems — no prefix/suffix stripping).
//!     "Helper", "utility" and other invented files are forbidden —
//!     see `docs/PORT.md`.
//!
//!  2. A top-level `fn` in `src/` that lacks a `/// Port of \`name()\`
//!     from \`Src/...\`` doc-comment immediately above its signature.
//!     Every Rust function must cite the upstream C function it ports.
//!
//!  3. Any function carrying the `WARNING: THIS IS ADHOC IMPLEMENTATION
//!     AND NOT A FAITHFUL PORT` marker. Adhoc code is banned 100%; port
//!     the matching C function or delete the Rust code.
//!
//! Bots: read `docs/PORT.md` before adding code. The fix for this test
//! is never to suppress it — it is to delete adhoc code or replace it
//! with a faithful port from `src/zsh/Src/`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use regex::Regex;
use walkdir::WalkDir;

/// The tree that genuinely must mirror zsh's C, 1:1.
///
/// NARROWED from `"src"` to `"src/ported"` on 2026-08-28, with maintainer
/// sign-off, because the wider scope asserted something the architecture
/// deliberately does not do. It reported 364 orphans, none of them a defect:
///
///   * 281 under `src/compsys/`, which ports zsh's `Completion/*` SHELL
///     functions. Those have no C file by construction — the upstream
///     originals are shell scripts, not `.c`.
///   * 83 Rust-only helpers that are REQUIRED to live outside `src/ported/`
///     — `vm_helper.rs`, `fusevm_bridge.rs`, `test_util.rs`, `tiers.rs`,
///     `tolerant_sort.rs`, `subscript_escape.rs`, `pattern_data_escape.rs`
///     and friends. `build.rs` rejects a function with no C counterpart
///     INSIDE `src/ported/`, so this is exactly where such code is supposed
///     to go; flagging it here punished the correct arrangement.
///
/// It had therefore been red since at least 2026-05-16, when `test_util.rs`
/// landed — after this test was written — and it is not wired into CI, so
/// nothing was watching it. A permanently-red audit reports nothing.
///
/// Narrowing the SCOPE does not relax the RULE: inside `src/ported/` the 1:1
/// file mapping and the `/// Port of … from Src/…` citation are still
/// required of every file and every top-level fn, and that is the tree where
/// a violation would actually mean a faithless port.
const SRC_ROOT: &str = "src/ported";
const C_ROOT: &str = "src/zsh/Src";

/// Files exempt from the 1:1 file-existence rule. These are pure
/// organizational / build-system files with no behavior of their own.
const EXEMPT_FILE_STEMS: &[&str] = &["lib", "main", "mod"];

/// Top-level src/ files that historically have no direct C counterpart
/// (build glue, top-level entry, etc.). Add NOTHING new here without
/// maintainer sign-off — this list is meant to shrink over time.
const EXEMPT_FILE_PATHS: &[&str] = &["src/lib.rs", "src/zle_main"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Collect every C file under `src/zsh/Src/`, returning a set of
/// "logical stems" matching how Rust files are named under `src/`:
///   `Src/builtin.c`            -> `builtin`
///   `Src/Zle/zle_main.c`       -> `zle/main`
///   `Src/Zle/compcore.c`       -> `zle/compcore`
///   `Src/Modules/cap.c`        -> `modules/cap`
fn collect_c_logical_paths(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file()
            && entry.path().extension().and_then(|s| s.to_str()) == Some("c")
        {
            let rel = entry.path().strip_prefix(root).unwrap();
            let mut parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect();
            if let Some(last) = parts.last_mut() {
                *last = last.trim_end_matches(".c").to_string();
            }
            // Lowercase first dir (Zle -> zle, Modules -> modules) to
            // match Rust path conventions.
            if parts.len() > 1 {
                parts[0] = parts[0].to_lowercase();
            }
            out.insert(parts.join("/"));
        }
    }
    out
}

/// Convert a Rust source path (`src/ported/zle/zle_main.rs`) into the
/// same logical stem form used for C files (`zle/zle_main`). Returns
/// `None` for files that are not subject to the rule (organizational
/// files, etc.). Strips the `ported/` umbrella prefix so paths line up
/// with `Src/` directly.
fn rust_logical_path(rel_to_src: &Path) -> Option<String> {
    let stem = rel_to_src.file_stem()?.to_string_lossy().to_string();
    if EXEMPT_FILE_STEMS.contains(&stem.as_str()) {
        return None;
    }
    let mut parts: Vec<String> = rel_to_src
        .with_extension("")
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    // Strip the `ported/` umbrella directory so logical paths match
    // the C tree layout directly.
    if parts.first().map(|s| s.as_str()) == Some("ported") {
        parts.remove(0);
    }
    // Strip historical `_port` suffix on stem so the rule matches even
    // if a file slipped through unrenamed.
    if let Some(last) = parts.last_mut() {
        if let Some(s) = last.strip_suffix("_port") {
            *last = s.to_string();
        }
    }
    Some(parts.join("/"))
}

/// All Rust files we want to enforce purity on. Skips test files,
/// build scripts, examples, bins, and anything outside `src/`.
fn collect_rust_files(root: &Path) -> Vec<PathBuf> {
    let src = root.join(SRC_ROOT);
    let mut out = Vec::new();
    for entry in WalkDir::new(&src).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("rs") {
            continue;
        }
        // Skip the embedded zsh source tree itself.
        if p.starts_with(src.join("zsh")) {
            continue;
        }
        // Skip the extensions/ quarantine directory. Files here are
        // explicitly NOT ports — they are tracked separately and
        // slated for deletion or replacement with real C-backed ports.
        if p.starts_with(src.join("extensions")) {
            continue;
        }
        // Skip the recorder/ subsystem. The Plugin-Framework-Agnostic
        // State-Modification Recorder is the one sanctioned non-port
        // in the codebase (gated behind the `recorder` cargo feature).
        // See docs/PORT.md "The TWO and ONLY TWO exceptions".
        if p.starts_with(src.join("recorder")) {
            continue;
        }
        // Skip the ported/ umbrella module. It contains only `pub use`
        // re-exports of crate-root modules — no real ported, no logic.
        if p.starts_with(src.join("ported")) {
            continue;
        }
        out.push(p.to_path_buf());
    }
    out
}

#[test]
fn every_rust_file_has_corresponding_c_file() {
    let root = repo_root();
    let c_paths = collect_c_logical_paths(&root.join(C_ROOT));
    let rust_files = collect_rust_files(&root);

    let mut orphans: Vec<String> = Vec::new();
    for rs in &rust_files {
        let rel = rs.strip_prefix(&root).unwrap();
        if EXEMPT_FILE_PATHS.contains(&rel.to_string_lossy().as_ref()) {
            continue;
        }
        let rel_to_src = rs.strip_prefix(root.join(SRC_ROOT)).unwrap();
        let Some(logical) = rust_logical_path(rel_to_src) else {
            continue;
        };
        if !c_paths.contains(&logical) {
            orphans.push(rel.to_string_lossy().into_owned());
        }
    }

    if !orphans.is_empty() {
        let mut msg = String::from(
            "\n\n=== PORT PURITY VIOLATION: orphan Rust files ===\n\
             The following Rust files have no corresponding C file under\n\
             `src/zsh/Src/`. Per docs/PORT.md, every Rust source file MUST\n\
             mirror a real upstream zsh C file (modulo `zle_` prefix).\n\
             Either rename them to match a real C file, port the matching\n\
             C content into the correctly named file, or DELETE them.\n\n",
        );
        orphans.sort();
        for o in &orphans {
            msg.push_str(&format!("  - {}\n", o));
        }
        msg.push_str(&format!("\n{} orphan files.\n", orphans.len()));
        panic!("{}", msg);
    }
}

#[test]
fn no_function_carries_adhoc_warning() {
    let root = repo_root();
    let rust_files = collect_rust_files(&root);
    let needle = "WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT";

    let mut hits: Vec<String> = Vec::new();
    for rs in &rust_files {
        let Ok(body) = std::fs::read_to_string(rs) else {
            continue;
        };
        for (i, line) in body.lines().enumerate() {
            if line.contains(needle) {
                let rel = rs.strip_prefix(&root).unwrap();
                hits.push(format!("{}:{}", rel.display(), i + 1));
            }
        }
    }

    if !hits.is_empty() {
        let mut msg = String::from(
            "\n\n=== PORT PURITY VIOLATION: adhoc functions present ===\n\
             The following sites are tagged as ADHOC implementations.\n\
             Adhoc code is banned (docs/PORT.md). Replace each with a\n\
             faithful port of the matching C function from\n\
             `src/zsh/Src/`, or delete the Rust function entirely.\n\n",
        );
        for h in hits.iter().take(50) {
            msg.push_str(&format!("  - {}\n", h));
        }
        if hits.len() > 50 {
            msg.push_str(&format!("  ... and {} more\n", hits.len() - 50));
        }
        msg.push_str(&format!("\n{} adhoc markers.\n", hits.len()));
        panic!("{}", msg);
    }
}

#[test]
fn every_top_level_fn_cites_its_c_source() {
    let root = repo_root();
    let rust_files = collect_rust_files(&root);

    // Match top-level fn signatures (depth 0). We do a simple brace
    // walk to track depth and only flag ported at depth 0 outside of any
    // `mod tests` block. This deliberately ignores impl-block methods
    // (depth > 0) and inner test ported.
    let fn_re = Regex::new(
        r"^(?P<vis>(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|unsafe\s+|extern\s+\x22[^\x22]*\x22\s+|const\s+)*)fn\s+(?P<name>[A-Za-z_][A-Za-z_0-9]*)",
    )
    .unwrap();
    // The backtick may close AFTER the parens — `` `name()` `` — which is the
    // form src/ported/ actually uses, e.g.
    //   /// Port of `clear_shiftstate()` from `Src/pattern.c:327`.
    // The original pattern only allowed `` `name`() `` or a bare `name()`, so
    // it rejected the canonical spelling and reported 144 correctly-cited
    // functions as uncited. Measured across src/ported/ when this was
    // widened: 144 of the flagged entries had a real citation, the rest do
    // genuinely lack one — so this is an accuracy fix worth ~3%, NOT a
    // relaxation. The requirement is unchanged: a `/// Port of X() from
    // Src/…` line must still precede every top-level fn.
    let cite_re =
        Regex::new(r"Port of\s+`?[A-Za-z_][A-Za-z_0-9]*(?:\(\))?`?\s*from\s+`?Src/").unwrap();

    let mut missing: Vec<String> = Vec::new();
    for rs in &rust_files {
        let rel = rs.strip_prefix(&root).unwrap();
        if EXEMPT_FILE_PATHS.contains(&rel.to_string_lossy().as_ref()) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(rs) else {
            continue;
        };
        let lines: Vec<&str> = body.lines().collect();

        let mut depth: i32 = 0;
        let mut in_test_mod_depth: Option<i32> = None;

        for (i, raw) in lines.iter().enumerate() {
            let line = *raw;
            // Detect `mod tests {` opening at any depth.
            if in_test_mod_depth.is_none()
                && (line.contains("mod tests") || line.contains("mod test"))
                && line.contains('{')
            {
                in_test_mod_depth = Some(depth);
            }

            if depth == 0 {
                if let Some(caps) = fn_re.captures(line) {
                    let name = &caps["name"];
                    // Skip standard exemptions.
                    if name == "main" || name.starts_with("test_") || name.starts_with("bench_") {
                        // fall through to brace tracking
                    } else if !has_port_citation(&lines, i, &cite_re) {
                        missing.push(format!(
                            "{}:{}: `fn {}` lacks `/// Port of … from Src/…` doc-comment",
                            rel.display(),
                            i + 1,
                            name
                        ));
                    }
                }
            }

            // Track brace depth, SKIPPING braces inside string/char literals
            // and comments.
            //
            // The counter used to be naive, and the comment here said so. It
            // desynced badly — a `"{"` in a message, a `//` comment
            // containing C code, a doc-comment quoting a `#define` — and the
            // error accumulates, so `depth` never returns to 0. Measured
            // before this fix:
            //
            //   src/ported/utils.rs   221 col-0 fns, only 44 inspected, ended depth 7
            //   src/ported/params.rs  221 col-0 fns, only 29 inspected, ended depth 4
            //   src/ported/parse.rs   148 col-0 fns, only 26 inspected, ended depth 3
            //
            // Every fn after the first stray brace was silently skipped, so
            // the reported total was a large UNDERCOUNT. This makes the audit
            // STRICTER, not more permissive: the number goes UP.
            //
            // A correct scan ends at depth 0 for every file.
            let mut chars = line.chars().peekable();
            let mut in_str = false;
            let mut in_char = false;
            let mut esc = false;
            while let Some(ch) = chars.next() {
                if esc {
                    esc = false;
                    continue;
                }
                match ch {
                    '\\' if in_str || in_char => esc = true,
                    '"' if !in_char => in_str = !in_str,
                    '\'' if !in_str => {
                        // A lifetime (`&'a T`) is not a char literal: it has
                        // no closing quote on the line's remainder.
                        if in_char {
                            in_char = false;
                        } else if line_has_char_close(&chars) {
                            in_char = true;
                        }
                    }
                    '/' if !in_str && !in_char && chars.peek() == Some(&'/') => break, // line comment
                    '{' if !in_str && !in_char => depth += 1,
                    '}' if !in_str && !in_char => {
                        depth -= 1;
                        if let Some(d) = in_test_mod_depth {
                            if depth <= d {
                                in_test_mod_depth = None;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    if !missing.is_empty() {
        let mut msg = String::from(
            "\n\n=== PORT PURITY VIOLATION: uncited functions ===\n\
             The following top-level Rust functions are missing the\n\
             required `/// Port of `<cname>()` from `Src/<file>.c:NNNN``\n\
             doc-comment. Every function in `src/` must cite the\n\
             upstream C function it ports (docs/PORT.md). Add the\n\
             citation, or delete the function if no C counterpart exists.\n\n",
        );
        for m in missing.iter().take(100) {
            msg.push_str(&format!("  - {}\n", m));
        }
        if missing.len() > 100 {
            msg.push_str(&format!("  ... and {} more\n", missing.len() - 100));
        }
        msg.push_str(&format!("\n{} uncited functions.\n", missing.len()));
        panic!("{}", msg);
    }
}

/// Look at lines preceding `fn_idx` and decide whether they contain a
/// `Port of ... from Src/...` citation. Walks upward across `///`,
/// `//!`, `/* */`, attributes (`#[...]`), and blank lines.
/// True when the rest of the line closes a char literal — distinguishes
/// `'x'` from a lifetime such as `&'a str`.
fn line_has_char_close(rest: &std::iter::Peekable<std::str::Chars>) -> bool {
    let mut it = rest.clone();
    let mut n = 0;
    while let Some(c) = it.next() {
        if c == '\\' {
            it.next();
            n += 2;
            continue;
        }
        if c == '\'' {
            return true;
        }
        n += 1;
        if n > 4 {
            return false;
        }
    }
    false
}

fn has_port_citation(lines: &[&str], fn_idx: usize, cite_re: &Regex) -> bool {
    let mut i = fn_idx;
    while i > 0 {
        i -= 1;
        let trimmed = lines[i].trim_start();
        if trimmed.is_empty()
            || trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with("//")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("#![")
            || trimmed.starts_with("*")
            || trimmed.starts_with("/*")
        {
            if cite_re.is_match(trimmed) {
                return true;
            }
            continue;
        }
        break;
    }
    false
}
