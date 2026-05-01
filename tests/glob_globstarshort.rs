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

fn opts() -> zsh::glob::GlobOptions {
    zsh::glob::GlobOptions {
        null_glob: true,
        mark_dirs: false,
        no_glob_dots: true,
        list_types: false,
        numeric_sort: false,
        follow_links: false,
        extended_glob: true,
        case_glob: true,
        glob_star_short: true,
        bare_glob_qual: true,
        brace_ccl: false,
    }
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
    let mut got = zsh::glob::glob_with_options(&pattern, opts());
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
    let got = zsh::glob::glob_with_options(&pattern, opts());

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

    // Spot check: known nested files exist.
    let has_glob_rs = got.iter().any(|p| p.ends_with("/src/glob.rs"));
    let has_exec_rs = got.iter().any(|p| p.ends_with("/src/exec.rs"));
    assert!(has_glob_rs, "src/glob.rs not found in `**.rs` results");
    assert!(has_exec_rs, "src/exec.rs not found in `**.rs` results");
}
