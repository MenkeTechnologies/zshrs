//! Enforces PORT.md's "no functions whose names aren't in zsh C source"
//! rule. Walks every free `fn` in `src/ported/**.rs` and verifies the
//! same name appears as a function definition in the upstream zsh C
//! source under `~/forkedRepos/zsh/Src/`.
//!
//! Methods inside `impl` / `trait` blocks are skipped — those map onto
//! C's struct-of-fn-pointers indirection which doesn't preserve the
//! name. Only top-level free functions count.
//!
//! Why this test exists: the substitution-bug audit on 2026-05-07
//! found two helper fns I added (`paramsubst_bridge`, `store_assign`)
//! plus seven pre-existing helpers that drifted from the freeze. The
//! port was claiming "100% port" while running 11 helpers with no C
//! counterpart. This test fails CI on any future drift so the next
//! contributor can't quietly add `helper_to_make_it_work` again.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Extract free-fn names from a Rust source file. Skips methods
/// (lines indented inside `impl` / `trait` blocks) by tracking brace
/// depth: depth 0 = module level. A `fn` at depth > 0 is a method.
/// Also skips test-only fns (#[test], #[cfg(test)] modules).
fn collect_free_fns(src: &str) -> Vec<(String, usize)> {
    let mut fns: Vec<(String, usize)> = Vec::new();
    let mut depth: i32 = 0;
    let mut in_test_mod = false;
    let mut test_mod_depth: i32 = 0;

    for (lineno, line) in src.lines().enumerate() {
        let lineno = lineno + 1;
        let trimmed = line.trim_start();

        // Detect entering a `#[cfg(test)] mod tests { ... }` so we can
        // ignore fns inside it. Approximate: any `mod X` after a `#[cfg(test)]`
        // attribute. Simpler: just recognize the literal `mod tests {`
        // shape that's our convention.
        if !in_test_mod && trimmed.starts_with("#[cfg(test)]") {
            // Next non-blank line might be `mod tests {`. Check via a peek
            // that looks at the same trimmed line continuation — for
            // simplicity treat any subsequent depth=0 `mod tests {` as test.
            // We mark and let the depth tracker handle entry.
        }
        // Recognize start of a test mod at module level.
        if depth == 0 && (trimmed.starts_with("mod tests {") || trimmed.starts_with("mod test {")) {
            in_test_mod = true;
            test_mod_depth = depth + 1;
        }

        // Update brace depth — count `{` and `}` outside string/char
        // literals. Crude but adequate for our consistent codebase
        // formatting. Skip lines that are line comments.
        let scan = if let Some(pos) = line.find("//") {
            &line[..pos]
        } else {
            line
        };
        for c in scan.chars() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if in_test_mod && depth < test_mod_depth {
                        in_test_mod = false;
                    }
                }
                _ => {}
            }
        }

        // Recognize free fn declarations at module level (depth 0
        // BEFORE the line's own `{` is consumed — the fn keyword
        // appears before its opening brace, so check pre-line depth).
        // For accuracy we re-scan: if the line contains `fn NAME(`
        // and the brace depth WAS 0 before this line, count it.
        // Compute pre-line depth by subtracting deltas from this line.
        let mut delta: i32 = 0;
        for c in scan.chars() {
            match c {
                '{' => delta += 1,
                '}' => delta -= 1,
                _ => {}
            }
        }
        let pre_depth = depth - delta;

        if in_test_mod {
            continue;
        }
        if pre_depth != 0 {
            continue;
        }

        // Look for `fn NAME(` patterns. Allow visibility modifiers and
        // optional `unsafe` / `async` / `extern`.
        let stripped = trimmed
            .strip_prefix("pub(crate) ")
            .or_else(|| trimmed.strip_prefix("pub(super) ").map(|s| s))
            .unwrap_or_else(|| trimmed.strip_prefix("pub ").unwrap_or(trimmed));
        let stripped = stripped.strip_prefix("unsafe ").unwrap_or(stripped);
        let stripped = stripped.strip_prefix("async ").unwrap_or(stripped);
        let stripped = stripped.strip_prefix(r#"extern "C" "#).unwrap_or(stripped);

        if let Some(rest) = stripped.strip_prefix("fn ") {
            // Extract NAME up to `(` or `<` (generics).
            let name_end = rest
                .find(|c: char| c == '(' || c == '<' || c.is_whitespace())
                .unwrap_or(0);
            if name_end > 0 {
                let name = rest[..name_end].to_string();
                if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    fns.push((name, lineno));
                }
            }
        }
    }
    fns
}

/// Scan zsh's C source for any function whose definition name matches.
/// Crude check: search every `.c` file under the C source root for a
/// line whose first identifier (in a function-definition shape) is
/// the candidate. We accept either:
///   `NAME(`   — old K&R / no-modifier shape
///   `*NAME(`  — pointer-return shape
///   `<modifiers> NAME(...)` somewhere on the line
fn c_source_root() -> Option<PathBuf> {
    let env_path = std::env::var_os("ZSH_C_SOURCE")
        .map(PathBuf::from)
        .filter(|p| p.is_dir());
    if let Some(p) = env_path {
        return Some(p);
    }
    // Default: ~/forkedRepos/zsh/Src — the documented checkout location.
    if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push("forkedRepos/zsh/Src");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn collect_c_fn_names(c_root: &Path) -> HashSet<String> {
    let mut names: HashSet<String> = HashSet::new();
    let mut files: Vec<PathBuf> = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("c") {
                out.push(path);
            }
        }
    }
    walk(c_root, &mut files);

    for path in &files {
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for line in src.lines() {
            // Look for function-def shapes. Match a candidate name
            // followed by `(` then content then `)` ending the line
            // (typical K&R / one-line-decl). Also accept `*NAME(`.
            // Skip preprocessor lines and obvious control-flow.
            let l = line.trim_start();
            if l.starts_with('#') || l.starts_with("//") || l.starts_with("/*") {
                continue;
            }
            // Find a `(` and walk back for the identifier.
            if let Some(paren) = l.find('(') {
                let pre = &l[..paren];
                // Walk backward extracting an identifier.
                let id_end = pre.len();
                let mut id_start = id_end;
                for (i, c) in pre.char_indices().rev() {
                    if c.is_ascii_alphanumeric() || c == '_' {
                        id_start = i;
                    } else {
                        break;
                    }
                }
                if id_start < id_end {
                    let name = &pre[id_start..id_end];
                    // Skip C keywords that look like fn calls.
                    if !matches!(
                        name,
                        "if" | "while" | "for" | "switch" | "return"
                        | "sizeof" | "typedef" | "do" | "else"
                    ) {
                        names.insert(name.to_string());
                    }
                }
            }
        }
    }
    names
}

#[test]
fn ported_fns_match_c_source() {
    let mut ported_files: Vec<PathBuf> = Vec::new();
    let ported_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ported");
    collect_rust_files(&ported_root, &mut ported_files);

    let c_root = match c_source_root() {
        Some(p) => p,
        None => {
            // Soft-skip when the C source isn't present (CI without
            // the upstream checkout). Set ZSH_C_SOURCE to enforce.
            eprintln!(
                "ZSH_C_SOURCE not set and ~/forkedRepos/zsh/Src not found — \
                 skipping. Set ZSH_C_SOURCE to enforce the freeze."
            );
            return;
        }
    };
    let c_names = collect_c_fn_names(&c_root);
    eprintln!("Loaded {} C function names from {}", c_names.len(), c_root.display());

    // Allowlist loaded from `tests/data/ported_fn_allowlist.txt`.
    // Snapshot of pre-existing violations — anything in this file is
    // exempt-for-now. Anything NOT in this file but free-fn-without-
    // C-counterpart fails the test, blocking new drift.
    //
    // To shrink: inline the body at every call site (or rename to a
    // real C function), then remove the line from the snapshot file.
    let allowlist_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/ported_fn_allowlist.txt");
    let allowlist_src = fs::read_to_string(&allowlist_path).unwrap_or_else(|_| {
        panic!(
            "missing snapshot file {}. Generate via: \n  \
             cargo test --test ported_fn_names_match_c -- --nocapture 2>&1 | \
             grep 'no C counterpart' | sed -E 's/.*fn ([a-zA-Z_][a-zA-Z_0-9]*).*/\\1/' | sort -u",
            allowlist_path.display()
        )
    });
    let allowlist: HashSet<String> = allowlist_src
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();

    let mut violations: Vec<String> = Vec::new();
    for path in &ported_files {
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for (name, lineno) in collect_free_fns(&src) {
            if allowlist.contains(&name) {
                continue;
            }
            if !c_names.contains(&name) {
                violations.push(format!(
                    "  {}:{}  fn {} — no C counterpart in zsh source",
                    path.strip_prefix(&PathBuf::from(env!("CARGO_MANIFEST_DIR")))
                        .unwrap_or(path)
                        .display(),
                    lineno,
                    name,
                ));
            }
        }
    }

    if !violations.is_empty() {
        violations.sort();
        panic!(
            "PORT.md freeze violation: {} NEW function(s) in src/ported/ \
             have no matching definition in zsh's C source AND are not in \
             the snapshot allowlist (tests/data/ported_fn_allowlist.txt). \
             Either inline them at the call sites, rename to match a C \
             function name, or — for boundary adapters only — add to the \
             snapshot file with a comment justifying the exemption.\n\n{}\n",
            violations.len(),
            violations.join("\n")
        );
    }
}
