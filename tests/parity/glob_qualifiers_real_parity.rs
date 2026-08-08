//! Glob-qualifier parity tests in real file-globbing context.
//!
//! Each test creates a tempdir with specific file types, then globs
//! with a qualifier and compares the sorted result against zsh.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}
fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}
fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
#[allow(dead_code)]
struct R {
    stdout: String,
    exit: i32,
}
fn run_zsh_in(d: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(d)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs_in(d: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(d)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity_sorted(d: &Path, s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh_in(d, s);
    let r = run_zshrs_in(d, s);
    let z_sorted = {
        let mut v: Vec<&str> = z.stdout.lines().collect();
        v.sort();
        v.join("\n")
    };
    let r_sorted = {
        let mut v: Vec<&str> = r.stdout.lines().collect();
        v.sort();
        v.join("\n")
    };
    assert_eq!(
        z_sorted, r_sorted,
        "glob divergence on:\n{s}\n--- zsh ---\n{z_sorted}\n--- zshrs ---\n{r_sorted}"
    );
}

fn setup_mixed_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().expect("tempdir");
    let base = d.path();
    // 3 regular files
    std::fs::write(base.join("file1.txt"), b"").unwrap();
    std::fs::write(base.join("file2.txt"), b"").unwrap();
    std::fs::write(base.join("file3.md"), b"").unwrap();
    // 1 subdirectory
    std::fs::create_dir(base.join("subdir")).unwrap();
    // 1 hidden file
    std::fs::write(base.join(".hidden"), b"").unwrap();
    d
}

mod file_type_qualifiers {
    use super::*;

    /// `*(.)` — regular files only (no dirs, no symlinks).
    #[test]
    fn dot_qualifier_only_regular_files() {
        let d = setup_mixed_dir();
        assert_parity_sorted(d.path(), "print -l *(.)");
    }

    /// `*(/)` — directories only.
    #[test]
    fn slash_qualifier_only_directories() {
        let d = setup_mixed_dir();
        assert_parity_sorted(d.path(), "print -l *(/)");
    }

    /// `*(.x)` — regular file AND executable.
    /// Setup: write file and `chmod +x` one of them.
    #[test]
    fn dot_x_qualifier_regular_executable() {
        let d = setup_mixed_dir();
        // Make file1.txt executable
        let mut perm = std::fs::metadata(d.path().join("file1.txt"))
            .unwrap()
            .permissions();
        use std::os::unix::fs::PermissionsExt;
        perm.set_mode(0o755);
        std::fs::set_permissions(d.path().join("file1.txt"), perm).unwrap();
        assert_parity_sorted(d.path(), "print -l *(.x)");
    }

    /// `*(.,*)` — regular files OR (separated by ,) executable.
    #[test]
    fn comma_alternation_in_qualifiers() {
        let d = setup_mixed_dir();
        assert_parity_sorted(d.path(), "print -l *(.,*)");
    }
}

mod null_glob {
    use super::*;

    /// `*(N)` — null-glob: no error on no-match, just empty.
    #[test]
    #[allow(non_snake_case)]
    fn N_qualifier_no_error_on_no_match() {
        let d = setup_mixed_dir();
        assert_parity_sorted(d.path(), "print -l *.nonexistent_xyz(N); echo done");
    }

    /// `*(N)` with match still works.
    #[test]
    #[allow(non_snake_case)]
    fn N_qualifier_with_match_returns_files() {
        let d = setup_mixed_dir();
        assert_parity_sorted(d.path(), "print -l *.txt(N)");
    }
}

mod sort_qualifiers {
    use super::*;

    /// `*(on)` — sort by name ascending.
    #[test]
    fn on_qualifier_sorts_by_name_ascending() {
        let d = setup_mixed_dir();
        // Don't sort the output since the qualifier already sorts;
        // compare line-by-line ordering.
        if !zsh_available() {
            return;
        }
        let s = "print -l *.txt(on)";
        let z = run_zsh_in(d.path(), s);
        let r = run_zshrs_in(d.path(), s);
        // Order matters here — pin strict equality.
        assert_eq!(z.stdout, r.stdout);
    }

    /// `*(On)` — sort by name descending.
    #[test]
    #[allow(non_snake_case)]
    fn On_qualifier_sorts_by_name_descending() {
        let d = setup_mixed_dir();
        if !zsh_available() {
            return;
        }
        let s = "print -l *.txt(On)";
        let z = run_zsh_in(d.path(), s);
        let r = run_zshrs_in(d.path(), s);
        assert_eq!(z.stdout, r.stdout);
    }
}

mod size_qualifier {
    use super::*;

    /// `*(Lk+1)` — files larger than 1 kilobyte.
    #[test]
    #[allow(non_snake_case)]
    fn L_kilobyte_size_filter() {
        let d = tempfile::tempdir().unwrap();
        // small file
        std::fs::write(d.path().join("small"), b"x").unwrap();
        // large file (>1KB)
        std::fs::write(d.path().join("big"), vec![b'x'; 2000]).unwrap();
        assert_parity_sorted(d.path(), "print -l *(Lk+1)");
    }

    /// `*(L+100)` — files larger than 100 bytes.
    #[test]
    #[allow(non_snake_case)]
    fn L_byte_size_filter() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a"), b"short").unwrap();
        std::fs::write(d.path().join("b"), vec![b'x'; 150]).unwrap();
        assert_parity_sorted(d.path(), "print -l *(L+100)");
    }
}

mod count_subscript {
    use super::*;

    /// `*([1])` — first match only.
    #[test]
    fn bracket_one_keeps_first() {
        let d = setup_mixed_dir();
        if !zsh_available() {
            return;
        }
        // Sort first to anchor "first" deterministically.
        let s = "print -l *(on[1])";
        let z = run_zsh_in(d.path(), s);
        let r = run_zshrs_in(d.path(), s);
        assert_eq!(z.stdout, r.stdout);
    }
}

mod combined {
    use super::*;

    /// `*(.on)` — regular files, sorted by name.
    #[test]
    fn dot_then_on_sorted_regular_files() {
        let d = setup_mixed_dir();
        if !zsh_available() {
            return;
        }
        let s = "print -l *(.on)";
        let z = run_zsh_in(d.path(), s);
        let r = run_zshrs_in(d.path(), s);
        assert_eq!(z.stdout, r.stdout);
    }

    /// `*(.N)` — regular files OR no-match-empty.
    #[test]
    #[allow(non_snake_case)]
    fn dot_then_N_regular_with_null_glob() {
        let d = setup_mixed_dir();
        assert_parity_sorted(d.path(), "print -l *(.N)");
    }
}

/// `l[+-]N` link-count qualifier. `qualnlink` once read a stale duplicate
/// `G_RANGE` static that the qual-eval never set, so every `l+N`/`l-N`
/// comparison was stuck on `==`. A dir with a subdir has nlink 3; an empty dir
/// has nlink 2.
mod link_count_qualifier {
    use super::*;

    fn setup_link_dirs() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(d.path().join("d")).unwrap(); // nlink 2…
        std::fs::create_dir(d.path().join("d/sub")).unwrap(); // …→ 3 with subdir
        std::fs::create_dir(d.path().join("e")).unwrap(); // nlink 2
        d
    }

    /// `l+2` → nlink > 2 → only `d`.
    #[test]
    fn links_greater_than_two() {
        let d = setup_link_dirs();
        assert_parity_sorted(d.path(), "print -l *(/l+2:t)");
    }

    /// `l-3` → nlink < 3 → only `e`.
    #[test]
    fn links_less_than_three() {
        let d = setup_link_dirs();
        assert_parity_sorted(d.path(), "print -l *(/l-3:t)");
    }

    /// `l2` → nlink == 2 → only `e`.
    #[test]
    fn links_equal_two() {
        let d = setup_link_dirs();
        assert_parity_sorted(d.path(), "print -l *(/l2:t)");
    }
}

/// `(n)` standalone numeric-sort qualifier — distinct from the `o`/`O`
/// sort KEY `n` (= GS_NAME, lexical). zsh/Src/glob.c:1575-1577
/// (`case 'n': gf_numsort = !(sense & 1)`): `(n)` makes the name
/// comparison numeric so `f2` sorts before `f10`; `(^n)` overrides the
/// global NUMERIC_GLOB_SORT option back to lexical. Order is significant,
/// so these compare stdout verbatim (no re-sort).
mod numeric_sort_qualifier {
    use super::*;

    fn setup_numeric_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        for name in ["f1", "f2", "f3", "f10", "f20", "f100"] {
            std::fs::write(d.path().join(name), b"").unwrap();
        }
        d
    }

    fn assert_order(d: &Path, s: &str) {
        if !zsh_available() {
            return;
        }
        let z = run_zsh_in(d, s);
        let r = run_zshrs_in(d, s);
        assert_eq!(
            z.stdout, r.stdout,
            "order divergence on:\n{s}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
            z.stdout, r.stdout
        );
    }

    /// `*(n)` → numeric order: f1 f2 f3 f10 f20 f100.
    #[test]
    fn n_qualifier_numeric_order() {
        let d = setup_numeric_dir();
        assert_order(d.path(), "print -l *(n)");
    }

    /// `f<->(n)` — numeric sort with a numeric-range pattern.
    #[test]
    fn n_qualifier_with_numeric_range_pattern() {
        let d = setup_numeric_dir();
        assert_order(d.path(), "print -l f<->(n)");
    }

    /// `(on)` is the sort KEY n (lexical by name), NOT numeric —
    /// regression guard so the standalone-vs-key distinction stays.
    #[test]
    fn on_sort_key_is_lexical_not_numeric() {
        let d = setup_numeric_dir();
        assert_order(d.path(), "print -l *(on)");
    }

    /// Global NUMERIC_GLOB_SORT makes the default name sort numeric.
    #[test]
    fn global_numericglobsort_option() {
        let d = setup_numeric_dir();
        assert_order(d.path(), "setopt numericglobsort; print -l *");
    }

    /// `(^n)` overrides the global option back to lexical.
    #[test]
    #[allow(non_snake_case)]
    fn caret_n_overrides_global_to_lexical() {
        let d = setup_numeric_dir();
        assert_order(d.path(), "setopt numericglobsort; print -l *(^n)");
    }

    /// `(n)` forces numeric even when the global option is off.
    #[test]
    fn n_qualifier_forces_numeric_without_global() {
        let d = setup_numeric_dir();
        assert_order(d.path(), "unsetopt numericglobsort; print -l *(n)");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Qualifier ARGUMENTS delimited by bracket pairs.
//
// `(e...)` routes through `glob_exec_string` (Src/glob.c:1085) and `(u...)`
// / `(g...)` read the name directly (c:1474 / c:1516) — all three call
// `get_strarg` (Src/subst.c:1348), whose delimiter switch (c:1366-1391) maps
// `(`→`)`, `[`→`]`, `{`→`}`, `<`→`>` and leaves anything else closing
// itself. Scanning for a REPEAT of the opening char instead left the closing
// bracket in the stream, so `*(e[CODE])` aborted with "missing end of
// string" and `*(u{0})` resolved the username "0}".
// ═══════════════════════════════════════════════════════════════════════════

mod qualifier_arg_delimiters {
    use super::*;

    fn setup_txt_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        std::fs::write(d.path().join("a.txt"), b"").unwrap();
        std::fs::write(d.path().join("b.dat"), b"").unwrap();
        d
    }

    #[test]
    fn eval_qualifier_colon_delim() {
        let d = setup_txt_dir();
        assert_parity_sorted(d.path(), "print -l *(.e:'[[ $REPLY == *.txt ]]':)");
    }

    #[test]
    fn eval_qualifier_brace_delim() {
        let d = setup_txt_dir();
        assert_parity_sorted(d.path(), "print -l *(.e{'[[ $REPLY == *.txt ]]'})");
    }

    #[test]
    fn eval_qualifier_bracket_delim() {
        let d = setup_txt_dir();
        assert_parity_sorted(d.path(), "print -l *(.e[true])");
    }

    #[test]
    fn eval_qualifier_brace_delim_bare_word() {
        let d = setup_txt_dir();
        assert_parity_sorted(d.path(), "print -l *(.e{true})");
    }

    /// `u` with a non-numeric name and a bracket delimiter: the name must
    /// stop at the CLOSING bracket, so the diagnostic names `0`, not `0}`.
    /// Compared on stderr because both shells fail the lookup.
    #[test]
    fn uid_qualifier_brace_delim_name_excludes_closer() {
        if !zsh_available() {
            return;
        }
        let d = setup_txt_dir();
        let script = "print -l *(u{0})";
        let z = Command::new(zsh_path())
            .args(["-fc", script])
            .current_dir(d.path())
            .output()
            .expect("zsh");
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", script])
            .current_dir(d.path())
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("zshrs");
        let zs = String::from_utf8_lossy(&z.stderr).into_owned();
        let rs = String::from_utf8_lossy(&r.stderr).into_owned();
        assert!(
            zs.contains("unknown username '0'") && rs.contains("unknown username '0'"),
            "u-qualifier delimiter divergence:\n--- zsh ---\n{zs}\n--- zshrs ---\n{rs}"
        );
    }
}
