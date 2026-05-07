//! Enforces that `/// Port of \`<C_FN>()\`` doc comments match the
//! actual fn name. zsh's C source has well-known names; the Rust
//! port should preserve them (`paramsubst` stays `paramsubst`, not
//! `parameter_substitute`). When a Rust port deliberately renames
//! a C function, that's a drift signal — the next contributor reads
//! the doc, sees `Port of paramsubst()`, and assumes the names
//! match. They often don't, and bugs slip in.
//!
//! Rule: if a `/// Port of \`X()\`` cite appears immediately above a
//! `fn Y(...)`, and X != Y, the test fails.
//!
//! Snapshot allowlist at `tests/data/ported_fn_rename_allowlist.txt`
//! grandfathers pre-existing mismatches. The list shrinks one rename
//! at a time.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

#[derive(Debug)]
struct Mismatch {
    file: PathBuf,
    line: usize,
    rust_name: String,
    c_name: String,
}

fn collect_rust_files(root: &std::path::Path, out: &mut Vec<PathBuf>) {
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

/// Walk a Rust source line-by-line. Track the most recent
/// `Port of \`X()\`` cite. When we hit the next FREE fn NAME(...),
/// compare X to NAME. Methods inside `impl` / `trait` blocks are
/// skipped — those use Rust naming idioms (`new`, `add`, `len`,
/// `push_back`) where the C source uses prefixed names
/// (`znewlinklist`, `addlinknode`, `countlinknodes`,
/// `pushnode`). The doc comment still cites the C-equivalent for
/// readers, but the Rust name follows convention.
///
/// Brace-depth tracking: depth 0 = module level. A fn at depth > 0
/// is a method — those are skipped. Test mods (`#[cfg(test)] mod
/// tests`) are also skipped.
fn find_mismatches_in(file: &PathBuf, out: &mut Vec<Mismatch>) {
    let src = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut pending_c_name: Option<String> = None;
    let mut depth: i32 = 0;
    let mut in_test_mod = false;
    let mut test_mod_depth: i32 = 0;

    for (i, line) in src.lines().enumerate() {
        let lineno = i + 1;
        let trimmed = line.trim();

        // Mark entry into `mod tests { ... }` (any depth).
        if !in_test_mod
            && (trimmed.starts_with("mod tests {") || trimmed.starts_with("mod test {"))
        {
            in_test_mod = true;
            test_mod_depth = depth + 1;
        }

        // Update brace depth (skip line comments). Compute pre-line
        // depth so we know if a `fn` here is at module level.
        let scan = if let Some(pos) = line.find("//") {
            &line[..pos]
        } else {
            line
        };
        let mut delta: i32 = 0;
        for c in scan.chars() {
            match c {
                '{' => delta += 1,
                '}' => delta -= 1,
                _ => {}
            }
        }
        let pre_depth = depth;
        depth += delta;
        if in_test_mod && depth < test_mod_depth {
            in_test_mod = false;
        }

        // Look for the cite. Handle `Port of \`name()\`` form.
        if let Some(start) = line.find("Port of `") {
            let rest = &line[start + 9..];
            if let Some(end) = rest.find("()`") {
                let name = &rest[..end];
                if name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
                    pending_c_name = Some(name.to_string());
                    continue;
                }
            }
        }

        if in_test_mod {
            continue;
        }

        // Methods (depth > 0) still CONSUME the pending cite —
        // the cite was for the method directly below it. Without
        // this consumption, the cite leaks past the impl block
        // and falsely binds to the next free fn.
        if pre_depth != 0 {
            let s = trimmed
                .strip_prefix("pub(crate) ")
                .or_else(|| trimmed.strip_prefix("pub(super) "))
                .unwrap_or_else(|| trimmed.strip_prefix("pub ").unwrap_or(trimmed));
            let s = s.strip_prefix("async ").unwrap_or(s);
            let s = s.strip_prefix("unsafe ").unwrap_or(s);
            if s.starts_with("fn ") {
                pending_c_name = None;
            }
            continue;
        }

        // Reset pending cite when we hit a non-fn top-level
        // declaration (struct/enum/trait/impl/static/const/type/
        // mod/use). The cite was for that item, not for some later
        // fn. Without this, a cite above `pub struct BgStatus`
        // leaked forward to the next fn declaration and falsely
        // reported a mismatch.
        let raw_trimmed = trimmed
            .strip_prefix("pub(crate) ")
            .or_else(|| trimmed.strip_prefix("pub(super) "))
            .unwrap_or_else(|| trimmed.strip_prefix("pub ").unwrap_or(trimmed));
        if pending_c_name.is_some() {
            let starts_decl = raw_trimmed.starts_with("struct ")
                || raw_trimmed.starts_with("enum ")
                || raw_trimmed.starts_with("trait ")
                || raw_trimmed.starts_with("impl ")
                || raw_trimmed.starts_with("impl<")
                || raw_trimmed.starts_with("static ")
                || raw_trimmed.starts_with("const ")
                || raw_trimmed.starts_with("type ")
                || raw_trimmed.starts_with("mod ")
                || raw_trimmed.starts_with("use ")
                || raw_trimmed.starts_with("union ")
                || raw_trimmed.starts_with("macro_rules!")
                || raw_trimmed.starts_with("extern ");
            if starts_decl {
                pending_c_name = None;
                continue;
            }
        }

        // `fn NAME(` or `pub fn NAME(` etc — at module level only.
        let s = trimmed
            .strip_prefix("pub(crate) ")
            .or_else(|| trimmed.strip_prefix("pub(super) "))
            .unwrap_or_else(|| trimmed.strip_prefix("pub ").unwrap_or(trimmed));
        let s = s.strip_prefix("async ").unwrap_or(s);
        let s = s.strip_prefix("unsafe ").unwrap_or(s);
        if let Some(rest) = s.strip_prefix("fn ") {
            // Extract NAME up to `(` or `<` (generics).
            let end = rest
                .find(|c: char| c == '(' || c == '<' || c.is_whitespace())
                .unwrap_or(0);
            if end > 0 {
                let rust_name = &rest[..end];
                if rust_name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
                    if let Some(c_name) = pending_c_name.take() {
                        if rust_name != c_name {
                            out.push(Mismatch {
                                file: file.clone(),
                                line: lineno,
                                rust_name: rust_name.to_string(),
                                c_name,
                            });
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn ported_fn_renames_match_doc_cite() {
    let mut ported_files: Vec<PathBuf> = Vec::new();
    let ported_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ported");
    collect_rust_files(&ported_root, &mut ported_files);

    let mut mismatches: Vec<Mismatch> = Vec::new();
    for file in &ported_files {
        find_mismatches_in(file, &mut mismatches);
    }

    // Snapshot allowlist: pre-existing rename-mismatches we can't
    // fix in one pass. Format: `<rust_relpath>::<rust_name>->\<c_name>`.
    let allowlist_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/ported_fn_rename_allowlist.txt");
    let allowlist_src = fs::read_to_string(&allowlist_path).unwrap_or_default();
    let allowlist: HashSet<String> = allowlist_src
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut violations: Vec<String> = Vec::new();
    for m in &mismatches {
        let rel = m
            .file
            .strip_prefix(&manifest_dir)
            .unwrap_or(&m.file)
            .display()
            .to_string();
        let key = format!("{}::{}->{}", rel, m.rust_name, m.c_name);
        if allowlist.contains(&key) {
            continue;
        }
        violations.push(format!(
            "  {}:{}  fn {} — doc cites `{}()` but Rust name differs",
            rel, m.line, m.rust_name, m.c_name
        ));
    }

    if !violations.is_empty() {
        violations.sort();
        panic!(
            "PORT.md naming violation: {} fn(s) in src/ported/ have a \
             `/// Port of \\`X()\\`` cite that doesn't match the Rust \
             name. Either rename the fn to match the C name (preferred), \
             or — only when the rename is intentional — add to the \
             snapshot file with a justifying comment.\n\n{}\n",
            violations.len(),
            violations.join("\n")
        );
    }
}
