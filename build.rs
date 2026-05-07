//! Build-time PORT.md drift enforcement.
//!
//! Runs the name-presence drift check (the spine of
//! `tests/ported_fn_names_match_c.rs`) on every cargo invocation
//! that touches the ported tree. Catches `pub fn name_not_in_zsh_c`
//! violations BEFORE any test binary compiles, so a bot doing
//! `cargo test --test foo` (which would otherwise skip the drift
//! tests) still gets stopped.
//!
//! This duplicates the core logic of `collect_free_fns` +
//! `load_c_fn_index` from the test. If you change either, mirror
//! the change here. Kept self-contained (std only) so the build
//! script has no extra dependencies.
//!
//! Re-run is gated by `cargo:rerun-if-changed` directives below —
//! unchanged builds skip the scan in the cargo cache.

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    // Only re-run when the inputs change. src/ported/** (the code
    // being checked), the C-name snapshot, and the allowlist files.
    println!("cargo:rerun-if-changed=src/ported");
    println!("cargo:rerun-if-changed=tests/data/zsh_c_fn_names.txt");
    println!("cargo:rerun-if-changed=tests/data/ported_fn_allowlist.txt");
    println!("cargo:rerun-if-changed=build.rs");

    let ported_root = manifest_dir.join("src/ported");
    let c_index_path = manifest_dir.join("tests/data/zsh_c_fn_names.txt");
    let allowlist_path = manifest_dir.join("tests/data/ported_fn_allowlist.txt");

    // If the ported root or the C-name snapshot is missing, this
    // isn't a configured port checkout — skip silently rather than
    // failing builds for unrelated workspaces.
    if !ported_root.exists() || !c_index_path.exists() {
        return;
    }

    let c_names = match load_c_fn_index(&c_index_path) {
        Ok(n) => n,
        Err(e) => {
            // Snapshot missing or unreadable — warn but don't break
            // the build. The full test will fail loudly if invoked.
            println!("cargo:warning=PORT.md drift: cannot read {} ({})", c_index_path.display(), e);
            return;
        }
    };

    let allowlist: HashSet<String> = fs::read_to_string(&allowlist_path)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();

    let mut rust_files: Vec<PathBuf> = Vec::new();
    collect_rust_files(&ported_root, &mut rust_files);

    let mut violations: Vec<String> = Vec::new();
    for path in &rust_files {
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let rel = path
            .strip_prefix(&manifest_dir)
            .unwrap_or(path)
            .display()
            .to_string();
        for (name, lineno) in collect_free_fns(&src) {
            if !allowlist.contains(&name) && !c_names.contains_key(&name) {
                violations.push(format!(
                    "  {}:{}  fn {} — no C counterpart in zsh source",
                    rel, lineno, name,
                ));
            }
        }
    }

    if !violations.is_empty() {
        violations.sort();
        // Emit cargo:warning for visibility in IDE / lint output,
        // then panic to fail the build. Cargo prints panic messages
        // before aborting.
        for v in &violations {
            println!("cargo:warning={}", v.trim());
        }
        panic!(
            "\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
             src/ported/ IS A PORT — NO NEW FUNCTIONS ALLOWED\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\n\
             Every `pub fn` / `fn` under src/ported/ MUST be a faithful\n\
             port of a function that exists in zsh's upstream C source\n\
             (~/forkedRepos/zsh/Src/, snapshotted at\n\
             tests/data/zsh_c_fn_names.txt). Rust-original helpers,\n\
             refactored extractions, convenience wrappers, and\n\
             ad-hoc abstractions DO NOT BELONG HERE — they create\n\
             drift between the port and the spec.\n\n\
             {} fn(s) violate this rule:\n\n\
             {}\n\n\
             To fix:\n\n\
               1. Preferred: inline the body at every call site\n\
                  (it isn't a real port, it shouldn't be a function).\n\
               2. Or: rename to match the actual C function it ports.\n\
                  Cite Src/<file>.c:<line> in the doc comment.\n\
               3. Last resort: add the name to\n\
                  tests/data/ported_fn_allowlist.txt with a comment\n\
                  explaining why no C analog exists (architectural\n\
                  Rust-only helpers like singleton accessors only).\n\n\
             Enforced by build.rs on every `cargo build` / `cargo test`\n\
             / `cargo check` whenever src/ported/** changes. Cannot be\n\
             bypassed by `cargo test --test X`.\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n",
            violations.len(),
            violations.join("\n")
        );
    }
}

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

/// Find free `fn NAME(` declarations at module level (depth 0).
/// Skips methods (anything inside `impl`/`trait`) and `mod tests`
/// blocks. Mirror of the test's `collect_free_fns` — keep in sync.
fn collect_free_fns(src: &str) -> Vec<(String, usize)> {
    let mut fns: Vec<(String, usize)> = Vec::new();
    let mut depth: i32 = 0;
    let mut in_test_mod = false;
    let mut test_mod_depth: i32 = 0;

    for (lineno, line) in src.lines().enumerate() {
        let lineno = lineno + 1;
        let trimmed = line.trim_start();

        if depth == 0
            && (trimmed.starts_with("mod tests {") || trimmed.starts_with("mod test {"))
        {
            in_test_mod = true;
            test_mod_depth = depth + 1;
        }

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

        if in_test_mod {
            continue;
        }
        if pre_depth != 0 {
            continue;
        }

        let stripped = trimmed
            .strip_prefix("pub(crate) ")
            .or_else(|| trimmed.strip_prefix("pub(super) "))
            .unwrap_or_else(|| trimmed.strip_prefix("pub ").unwrap_or(trimmed));
        let stripped = stripped.strip_prefix("unsafe ").unwrap_or(stripped);
        let stripped = stripped.strip_prefix("async ").unwrap_or(stripped);
        let stripped = stripped.strip_prefix(r#"extern "C" "#).unwrap_or(stripped);

        if let Some(rest) = stripped.strip_prefix("fn ") {
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

fn load_c_fn_index(path: &Path) -> Result<HashMap<String, HashSet<String>>, std::io::Error> {
    let src = fs::read_to_string(path)?;
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((file, name)) = line.split_once(':') {
            index
                .entry(name.to_string())
                .or_default()
                .insert(file.to_string());
        }
    }
    Ok(index)
}
