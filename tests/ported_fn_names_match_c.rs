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

use std::collections::{HashMap, HashSet};
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
    let mut in_block_comment: i32 = 0;

    for (lineno, line) in src.lines().enumerate() {
        let lineno = lineno + 1;
        let trimmed = line.trim_start();

        if depth == 0 && (trimmed.starts_with("mod tests {") || trimmed.starts_with("mod test {")) {
            in_test_mod = true;
            test_mod_depth = depth + 1;
        }

        // Mirror of build.rs::collect_free_fns — keep in sync.
        // Walks the line char-by-char tracking comment/string state
        // so `{`/`}` inside strings/chars/comments don't perturb depth.
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut delta: i32 = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if in_block_comment > 0 {
                if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    in_block_comment -= 1;
                    i += 2;
                    continue;
                }
                if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                    in_block_comment += 1;
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }
            match b {
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => break,
                b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                    in_block_comment += 1;
                    i += 2;
                }
                b'"' => {
                    i += 1;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c == b'\\' { i += 2; continue; }
                        if c == b'"' { i += 1; break; }
                        i += 1;
                    }
                }
                b'r' if i + 1 < bytes.len() && (bytes[i + 1] == b'"' || bytes[i + 1] == b'#') => {
                    let mut hashes = 0;
                    let mut j = i + 1;
                    while j < bytes.len() && bytes[j] == b'#' {
                        hashes += 1;
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'"' {
                        i = j + 1;
                        loop {
                            if i >= bytes.len() { break; }
                            if bytes[i] == b'"' {
                                let mut closed = 0;
                                let mut k = i + 1;
                                while k < bytes.len() && bytes[k] == b'#' && closed < hashes {
                                    closed += 1;
                                    k += 1;
                                }
                                if closed >= hashes {
                                    i = k;
                                    break;
                                }
                            }
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
                b'\'' => {
                    let mut j = i + 1;
                    let mut found_close = false;
                    let mut escape = false;
                    while j < bytes.len() && j - i < 12 {
                        if !escape && bytes[j] == b'\'' {
                            found_close = true;
                            break;
                        }
                        if bytes[j] == b'\\' && !escape {
                            escape = true;
                        } else {
                            escape = false;
                        }
                        j += 1;
                    }
                    if found_close {
                        i = j + 1;
                    } else {
                        i += 1;
                        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                            i += 1;
                        }
                    }
                }
                b'{' => { delta += 1; i += 1; }
                b'}' => { delta -= 1; i += 1; }
                _ => i += 1,
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

/// Load the C function-name index from
/// `tests/data/zsh_c_fn_names.txt`. The file is checked into git so
/// the test runs in any environment without depending on a local
/// checkout of zsh's C source.
///
/// Returns a map from function name to the set of C basenames
/// (e.g. "subst.c") that contain a definition for that name.
/// Regenerate after pulling new upstream commits via:
///   `tests/data/extract_c_fn_names.sh`
fn load_c_fn_index() -> HashMap<String, HashSet<String>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/zsh_c_fn_names.txt");
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing C-function index at {} ({}). \
             Regenerate via tests/data/extract_c_fn_names.sh.",
            path.display(),
            e
        )
    });
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();
    for line in src.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((file, name)) = line.split_once(':') {
            index.entry(name.to_string())
                .or_default()
                .insert(file.to_string());
        }
    }
    index
}

/// Map a Rust ported file path to the C basename(s) it should
/// port from. Rule: `src/ported/X.rs` → `X.c`, with explicit
/// per-area overrides for files that span multiple C sources.
///
/// Returns the set of acceptable C-source basenames for this Rust
/// file. Empty set = "no constraint" (file is exempt from
/// file-mapping check, only name presence is enforced). This
/// fallback is for files we haven't categorized yet.
fn rust_path_to_c_files(rust_path: &Path, root: &Path) -> HashSet<String> {
    let rel = rust_path.strip_prefix(root).unwrap_or(rust_path);
    let stem = rel.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let parent = rel.parent().and_then(|p| p.to_str()).unwrap_or("");

    // Explicit per-file overrides — Rust files that legitimately
    // pull from more than one C source (or use a different
    // basename). Each entry MUST cite the C areas it covers.
    let mut acceptable: HashSet<String> = HashSet::new();
    match (parent, stem) {
        // Rust pattern.rs covers Src/pattern.c (same name).
        ("ported", "pattern") => {
            acceptable.insert("pattern.c".to_string());
        }
        // glob.rs covers Src/glob.c plus the glob-helper bits in
        // pattern.c (matchcat/getmatch/etc).
        ("ported", "glob") => {
            acceptable.insert("glob.c".to_string());
            acceptable.insert("pattern.c".to_string());
        }
        // utils.rs is the catch-all — same as Src/utils.c plus
        // smaller helpers from string.c, mem.c, openssh_bsd_setres_id.c.
        ("ported", "utils") => {
            acceptable.insert("utils.c".to_string());
            acceptable.insert("string.c".to_string());
            acceptable.insert("mem.c".to_string());
        }
        // params.rs covers Src/params.c plus the special-param
        // helpers in Modules/parameter.c.
        ("ported", "params") => {
            acceptable.insert("params.c".to_string());
            acceptable.insert("parameter.c".to_string());
        }
        // builtin.rs covers Src/builtin.c — many builtins live there.
        ("ported", "builtin") => {
            acceptable.insert("builtin.c".to_string());
        }
        // Modules/* maps to Src/Modules/* by name.
        (p, name) if p.starts_with("ported/modules") => {
            acceptable.insert(format!("{}.c", name));
        }
        // Zle subdir maps to Src/Zle/* by exact name.
        (p, name) if p.starts_with("ported/zle") => {
            acceptable.insert(format!("{}.c", name));
        }
        // Default: same basename + .c (e.g., subst.rs → subst.c,
        // hist.rs → hist.c, cond.rs → cond.c).
        ("ported", name) => {
            acceptable.insert(format!("{}.c", name));
        }
        _ => {}
    }
    acceptable
}

#[test]
fn ported_fns_match_c_source() {
    let mut ported_files: Vec<PathBuf> = Vec::new();
    let ported_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ported");
    collect_rust_files(&ported_root, &mut ported_files);

    let c_index = load_c_fn_index();
    let c_names: HashSet<&String> = c_index.keys().collect();
    eprintln!("Loaded {} C function names from snapshot", c_names.len());

    // Allowlist loaded from `tests/data/fake_fn_allowlist.txt`.
    // Snapshot of pre-existing violations — anything in this file is
    // exempt-for-now. Anything NOT in this file but free-fn-without-
    // C-counterpart fails the test, blocking new drift.
    //
    // To shrink: inline the body at every call site (or rename to a
    // real C function), then remove the line from the snapshot file.
    let allowlist_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/fake_fn_allowlist.txt");
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
        .map(|l| {
            // Strip inline `# justification` so `name # comment`
            // parses to just `name`.
            let l = match l.find('#') {
                Some(i) => &l[..i],
                None => l,
            };
            l.trim()
        })
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();

    // File-mapping allowlist: pre-existing fns whose Rust file
    // doesn't match the expected C basename. Same shape as the
    // name allowlist — exempt-for-now snapshot. Anything new
    // landing in the wrong file fails immediately.
    let file_mapping_allowlist_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/ported_fn_file_mapping_allowlist.txt");
    let file_mapping_src =
        fs::read_to_string(&file_mapping_allowlist_path).unwrap_or_default();
    let file_mapping_allowlist: HashSet<String> = file_mapping_src
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();

    let ported_root_canonical = ported_root
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    let mut name_violations: Vec<String> = Vec::new();
    let mut file_violations: Vec<String> = Vec::new();
    for path in &ported_files {
        let src = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let expected_c_files = rust_path_to_c_files(path, &ported_root_canonical);
        let rel_display = path
            .strip_prefix(&PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .unwrap_or(path)
            .display()
            .to_string();
        for (name, lineno) in collect_free_fns(&src) {
            // Phase 1: name presence check.
            if !allowlist.contains(&name) && !c_names.contains(&name) {
                name_violations.push(format!(
                    "  {}:{}  fn {} — no C counterpart in zsh source",
                    rel_display, lineno, name,
                ));
                continue;
            }
            // Phase 2: file-mapping check. Skip when the name is
            // in the name-allowlist (those are exempt globally) or
            // when the Rust file has no expected mapping.
            if allowlist.contains(&name) || expected_c_files.is_empty() {
                continue;
            }
            let mapping_key = format!("{}::{}", rel_display, name);
            if file_mapping_allowlist.contains(&mapping_key) {
                continue;
            }
            // Where does the C source actually define this name?
            if let Some(c_files) = c_index.get(&name) {
                if c_files.is_disjoint(&expected_c_files) {
                    let actual: Vec<&String> = c_files.iter().collect();
                    let mut acceptable: Vec<&String> = expected_c_files.iter().collect();
                    acceptable.sort();
                    file_violations.push(format!(
                        "  {}:{}  fn {} — defined in {:?}, but Rust file \
                         expects port from {:?}",
                        rel_display, lineno, name, actual, acceptable,
                    ));
                }
            }
        }
    }

    if !name_violations.is_empty() || !file_violations.is_empty() {
        name_violations.sort();
        file_violations.sort();
        let mut msg = String::new();
        if !name_violations.is_empty() {
            msg.push_str(&format!(
                "PORT.md freeze violation: {} NEW function(s) in src/ported/ \
                 have no matching definition in zsh's C source AND are not in \
                 the snapshot allowlist (tests/data/fake_fn_allowlist.txt). \
                 Either inline at call sites, rename to match a C function, \
                 or add to the snapshot with a justifying comment.\n\n{}\n",
                name_violations.len(),
                name_violations.join("\n")
            ));
        }
        if !file_violations.is_empty() {
            if !msg.is_empty() {
                msg.push_str("\n\n");
            }
            msg.push_str(&format!(
                "PORT.md file-mapping violation: {} function(s) in src/ported/ \
                 are defined in a Rust file whose C-counterpart basename \
                 doesn't match where the function lives in zsh's C source. \
                 Either move the fn to the right Rust file (e.g. paramsubst \
                 belongs in subst.rs because it's defined in subst.c), or \
                 add an explicit override in rust_path_to_c_files() in this \
                 test if the Rust file legitimately spans multiple C areas. \
                 Pre-existing mismatches are exempt via \
                 tests/data/ported_fn_file_mapping_allowlist.txt.\n\n{}\n",
                file_violations.len(),
                file_violations.join("\n")
            ));
        }
        panic!("{}", msg);
    }
}
