//! GLOBSTARSHORT regression — verifies that `**X` (no separating `/`)
//! recurses and matches files via the implied glue `*` per zsh's
//! `instr += ((shortglob ? 1 : 3) + follow)` rule at glob.c:727-730.
//!
//! Pre-fix bug: parser produced `[Recursive, Pattern(".stk")]` which
//! only matched files literally named `.stk`. Post-fix: parser produces
//! `[Recursive, Pattern("*.stk")]` so `**.stk` ≡ `**/*.stk`.

use std::fs;
use std::path::Path;
use tempfile::TempDir;

// GlobOptions struct was deleted (Rust-only bag). Options are now
// read from the canonical option store; tests set the relevant
// flags before each call.
fn set_opts() {
    use zsh::ported::options::opt_state_set;
    opt_state_set("nullglob", true);
    opt_state_set("markdirs", false);
    opt_state_set("dotglob", false);
    opt_state_set("globdots", false);
    opt_state_set("listtypes", false);
    opt_state_set("numericglobsort", false);
    opt_state_set("globlinks", false);
    opt_state_set("extendedglob", true);
    opt_state_set("caseglob", true);
    opt_state_set("nocaseglob", false);
    opt_state_set("globstarshort", true);
    opt_state_set("bareglobqual", true);
    opt_state_set("braceccl", false);
}

#[test]
fn globstarshort_double_star_dot_stk_matches_at_any_depth() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let depth0 = root.join("a.stk");
    let depth1 = root.join("sub/b.stk");
    let depth2 = root.join("sub/deep/c.stk");
    let non_stk_d0 = root.join("a.txt");
    let non_stk_d1 = root.join("sub/b.txt");
    let literal_dotstk = root.join(".stk");

    for f in [
        &depth0,
        &depth1,
        &depth2,
        &non_stk_d0,
        &non_stk_d1,
        &literal_dotstk,
    ] {
        if let Some(parent) = f.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(f, b"").unwrap();
    }

    let pattern = format!("{}/**.stk", root.display());
    let mut got = {
        set_opts();
        // glob_path's matcher recognizes only glob-TOKENIZED metacharacters
        // (`Star` etc.); a raw `*` reaches it as a literal. Tokenize first,
        // exactly as the production caller (stryke's stryke_glob) does.
        let mut tok = pattern.clone();
        zsh::glob::tokenize(&mut tok);
        zsh::glob::glob_path(&tok)
    };
    got.sort();

    let normalize = |p: &Path| p.canonicalize().unwrap().to_string_lossy().to_string();
    let mut want = vec![normalize(&depth0), normalize(&depth1), normalize(&depth2)];
    want.sort();
    let got_normalized: Vec<String> = got
        .iter()
        .map(|s| {
            Path::new(s)
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();

    assert_eq!(
        got_normalized, want,
        "expected only *.stk files at any depth; got {:?}",
        got
    );

    // Sanity: literal `.stk` (the pre-fix false-positive shape) must
    // NOT be returned by `**.stk` since GLOBSTARSHORT rewrites it to
    // `**/*.stk` and `*` does not match leading dots under
    // no_glob_dots=true.
    for g in &got_normalized {
        assert!(
            !g.ends_with("/.stk"),
            "literal /.stk leaked through, got {}",
            g
        );
    }
}

#[test]
fn globstarshort_double_star_dot_rs_finds_project_sources() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let pattern = format!("{}/src/**.rs", manifest_dir);
    let got = {
        set_opts();
        // glob_path's matcher recognizes only glob-TOKENIZED metacharacters
        // (`Star` etc.); a raw `*` reaches it as a literal. Tokenize first,
        // exactly as the production caller (stryke's stryke_glob) does.
        let mut tok = pattern.clone();
        zsh::glob::tokenize(&mut tok);
        zsh::glob::glob_path(&tok)
    };

    assert!(
        !got.is_empty(),
        "expected `**.rs` to find .rs files under src/, got empty"
    );
    for path in &got {
        assert!(path.ends_with(".rs"), "non-.rs result leaked: {}", path);
        assert!(
            Path::new(path).is_file(),
            "result is not a regular file: {}",
            path
        );
    }

    // Spot check: known nested files exist. glob.rs lives under
    // src/ported/ (nested) — proves the walk descends past depth 1;
    // vm_helper.rs sits directly under src/ (depth 0 relative to the
    // `**` anchor) — proves the zero-directory branch fires too.
    let has_glob_rs = got.iter().any(|p| p.ends_with("/src/ported/glob.rs"));
    let has_vm_helper_rs = got.iter().any(|p| p.ends_with("/src/vm_helper.rs"));
    assert!(
        has_glob_rs,
        "src/ported/glob.rs not found in `**.rs` results"
    );
    assert!(
        has_vm_helper_rs,
        "src/vm_helper.rs not found in `**.rs` results"
    );
}

/// Explicit-separator `**/*.md` must keep recursing at EVERY depth (0, 1, N)
/// even with GLOBSTARSHORT on. parsecomplist mirrors C's short-circuiting
/// `str[2]=='/' || … || (shortglob = isset(GLOBSTARSHORT))`: when `**` is
/// followed by an explicit `/`, the `shortglob` assignment never runs, so
/// the parser advances past `**/` (3 chars) and the next component stays
/// `*.md`. An eager evaluation of the `shortglob` branch set it to 1
/// regardless, advanced by 1, left a stray `*`, and collapsed `**/` to a
/// single directory level — so `**/*.md` returned only the depth-1 match.
#[test]
fn globstarshort_explicit_slash_double_star_recurses_all_depths() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let top = root.join("top.md");
    let mid = root.join("a/mid.md");
    let deep = root.join("a/b/c/deep.md");

    for f in [&top, &mid, &deep] {
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, b"").unwrap();
    }

    let pattern = format!("{}/**/*.md", root.display());
    let mut got = {
        set_opts();
        // glob_path's matcher recognizes only glob-TOKENIZED metacharacters
        // (`Star` etc.); a raw `*` reaches it as a literal. Tokenize first,
        // exactly as the production caller (stryke's stryke_glob) does.
        let mut tok = pattern.clone();
        zsh::glob::tokenize(&mut tok);
        zsh::glob::glob_path(&tok)
    };
    got.sort();

    let normalize = |p: &Path| p.canonicalize().unwrap().to_string_lossy().to_string();
    let mut want = vec![normalize(&top), normalize(&mid), normalize(&deep)];
    want.sort();
    let got_normalized: Vec<String> = got
        .iter()
        .map(|s| {
            Path::new(s)
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();

    assert_eq!(
        got_normalized, want,
        "`**/*.md` with globstarshort must hit depth 0/1/N; got {:?}",
        got
    );
}
