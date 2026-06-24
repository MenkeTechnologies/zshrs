//! File-glob parity tests — each test builds its own tempdir, runs the
//! glob in both shells from inside, sorts output, and compares. Sort
//! is needed because directory enumeration order is filesystem-dependent.

#![allow(non_snake_case)]

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

struct R {
    stdout: String,
    exit: i32,
}

fn run_zsh_in(dir: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(dir)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs_in(dir: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(dir)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn assert_parity_in(dir: &Path, script: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh_in(dir, script);
    let r = run_zshrs_in(dir, script);
    let z_sorted: Vec<&str> = {
        let mut v: Vec<&str> = z.stdout.lines().collect();
        v.sort();
        v
    };
    let r_sorted: Vec<&str> = {
        let mut v: Vec<&str> = r.stdout.lines().collect();
        v.sort();
        v
    };
    assert_eq!(
        z_sorted, r_sorted,
        "glob output divergence on:\n{script}\n--- zsh sorted ---\n{:?}\n--- zshrs sorted ---\n{:?}",
        z_sorted, r_sorted
    );
}

/// Build a tempdir populated with the given files (subpaths). Directories
/// are auto-created from path separators.
fn mkdir_with_files(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for f in files {
        let p = dir.path().join(f);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir -p");
        }
        std::fs::write(&p, b"").expect("touch");
    }
    dir
}

mod basic_globs {
    use super::*;

    #[test]
    fn star_dot_txt_matches_txt_files_only() {
        let d = mkdir_with_files(&["a.txt", "b.txt", "c.rs", "d.md"]);
        assert_parity_in(d.path(), "print -l *.txt");
    }

    #[test]
    fn star_matches_all_non_hidden_files() {
        let d = mkdir_with_files(&["a", "b", "c"]);
        assert_parity_in(d.path(), "print -l *");
    }

    #[test]
    fn star_skips_hidden_by_default() {
        let d = mkdir_with_files(&[".hidden", "visible"]);
        assert_parity_in(d.path(), "print -l *");
    }

    #[test]
    fn question_matches_single_char() {
        let d = mkdir_with_files(&["a", "bb", "ccc"]);
        assert_parity_in(d.path(), "print -l ?");
    }

    #[test]
    fn three_questions_match_three_chars() {
        let d = mkdir_with_files(&["a", "bb", "ccc", "dddd"]);
        assert_parity_in(d.path(), "print -l ???");
    }
}

mod char_classes {
    use super::*;

    #[test]
    fn bracket_digit_range() {
        let d = mkdir_with_files(&["file1", "file2", "file3", "fileA"]);
        assert_parity_in(d.path(), "print -l file[0-9]");
    }

    #[test]
    fn bracket_specific_chars() {
        let d = mkdir_with_files(&["fa", "fb", "fc", "fd"]);
        assert_parity_in(d.path(), "print -l f[abc]");
    }

    #[test]
    fn bracket_negation() {
        let d = mkdir_with_files(&["fa", "fb", "fc", "fd"]);
        assert_parity_in(d.path(), "print -l f[^ab]");
    }

    #[test]
    fn bracket_multiple_ranges() {
        let d = mkdir_with_files(&["aX", "bY", "zZ", "1Q", "9P"]);
        assert_parity_in(d.path(), "print -l [a-z]?");
    }
}

mod nested_dirs {
    use super::*;

    #[test]
    fn glob_in_subdir() {
        let d = mkdir_with_files(&["sub/a.txt", "sub/b.txt", "sub/c.rs"]);
        assert_parity_in(d.path(), "print -l sub/*.txt");
    }

    /// Star at top level doesn't recurse into subdirs.
    #[test]
    fn star_does_not_recurse() {
        let d = mkdir_with_files(&["top1", "top2", "sub/inner1", "sub/inner2"]);
        assert_parity_in(d.path(), "print -l *");
    }
}

mod qualifiers {
    use super::*;

    /// `*(/)` — directories only.
    #[test]
    fn slash_qualifier_directories_only() {
        let d = mkdir_with_files(&["regular_file", "subdir/.placeholder"]);
        assert_parity_in(d.path(), "print -l *(/)");
    }

    /// `*(.)` — regular files only.
    #[test]
    fn dot_qualifier_regular_files_only() {
        let d = mkdir_with_files(&["regular_file", "subdir/.placeholder"]);
        assert_parity_in(d.path(), "print -l *(.)");
    }

    /// `*(N)` — null-glob (no error on no-match).
    #[test]
    fn N_qualifier_null_glob_no_match() {
        let d = mkdir_with_files(&["a.txt"]);
        // Without (N), `*.nonexistent` errors. With (N), produces no output, exits 0.
        assert_parity_in(d.path(), "print -l -- *.nonexistent_xyz(N)");
    }

    /// `*(.)` regular files combined with a glob — picks .txt files only.
    #[test]
    fn dot_qualifier_with_glob_pattern() {
        let d = mkdir_with_files(&["a.txt", "b.txt", "subdir/.placeholder"]);
        assert_parity_in(d.path(), "print -l *.txt(.)");
    }
}

mod no_match {
    use super::*;

    /// Without NULL_GLOB, no-match glob errors. zsh exits non-zero; zshrs
    /// may or may not — pin the contract and let the test flag divergence.
    #[test]
    fn unmatched_glob_errors_by_default() {
        let d = mkdir_with_files(&["only.txt"]);
        if !zsh_available() {
            return;
        }
        let z = run_zsh_in(d.path(), "echo *.nonexistent_xyz_42");
        let r = run_zshrs_in(d.path(), "echo *.nonexistent_xyz_42");
        assert_eq!(z.exit != 0, r.exit != 0, "exit-nonzero-ness must match");
    }
}

mod hidden_files {
    use super::*;

    /// `.*` matches hidden files explicitly.
    #[test]
    fn dot_star_matches_hidden_files() {
        let d = mkdir_with_files(&[".hidden_a", ".hidden_b", "visible"]);
        assert_parity_in(d.path(), "print -l .*");
    }
}

mod multibyte_text_is_not_glob {
    use super::*;

    /// UTF-8 continuation bytes that collide with token byte values
    /// (Hat = 0x86, Inang = 0x94, Star = 0x87 as u8) must not mark a
    /// word as a glob. Pre-fix, the dispatcher's pre-untokenize gate
    /// and pattern.rs::haswilds scanned BYTES, so `↔` (E2 86 94) and
    /// `⇇` (E2 87 87) fired "no matches found" from nested parameter
    /// substitutions — the zinit.zsh:251 `col-↔` load failure.
    #[test]
    fn nested_default_arm_with_u2194_arrow() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(d.path(), "echo ${${X}:-↔}");
    }

    #[test]
    fn nested_plus_arm_with_u2194_arrow() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(d.path(), "X=1; echo ${${X}:+↔}");
    }

    /// `⇇` carries 0x87 (Star as u8) twice — Star fires with no option
    /// gate, so this caught the bug even with extendedglob unset.
    #[test]
    fn nested_default_arm_with_u21c7_arrows() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(d.path(), "echo ${${X}:-⇇}");
    }

    /// Hat (0x86) is EXTENDEDGLOB-gated — pin the option-on path too.
    #[test]
    fn nested_default_arm_with_u2194_under_extendedglob() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(d.path(), "setopt extendedglob; echo ${${X}:-↔}");
    }

    /// The exact zinit.zsh:251 shape: nested `${(M)…:#…}` match flag
    /// feeding a `:+` arm whose value is a multibyte arrow.
    #[test]
    fn zinit_col_lr_shape() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(
            d.path(),
            "LANG=en_US.UTF-8; echo ${${${(M)LANG:#*UTF-8*}:+↔}:-fallback}",
        );
    }
}

/// Default/alternate-word filename generation (#2 default-word globbing).
/// The unquoted default/alt word in `${x:-W}` / `${x-W}` / `${x:+W}` /
/// `${x+W}` is SOURCE text, so a glob metachar in it drives filename
/// generation on the ASSEMBLED word — a parameter VALUE never globs.
/// c:Src/subst.c → globlist. The paramsubst arm sets a pending flag
/// (only for a source-glob default, via pretokenize_src_pat which skips
/// nested `$..` spans), and the compile-emitted DEFAULT_WORD_GLOB op
/// globs the assembled word; gated off in DQ / scalar-assign / assign-
/// builtin-arg contexts.
mod default_word_globbing {
    use super::*;

    fn files() -> tempfile::TempDir {
        mkdir_with_files(&["afile", "bfile"])
    }

    /// Unquoted source-glob default globs (`*`, `?`, `[...]`).
    #[test]
    fn unquoted_default_globs() {
        let d = files();
        assert_parity_in(d.path(), "print -l ${x:-*file}");
        assert_parity_in(d.path(), "print -l ${x-*file}");
        assert_parity_in(d.path(), "print -l ${x:-*fil?}");
        assert_parity_in(d.path(), "print -l ${x:-[ab]file}");
        assert_parity_in(d.path(), "print -l ${x:-a*}");
    }

    /// The alternate word (`:+`/`+`) globs when the var is set.
    #[test]
    fn alternate_word_globs() {
        let d = files();
        assert_parity_in(d.path(), "a=set; print -l ${a:+*file}");
        assert_parity_in(d.path(), "a=set; print -l ${a+*file}");
    }

    /// Glob runs on the ASSEMBLED word — prefix/suffix concatenate with
    /// the default before filename generation.
    #[test]
    fn assembled_word_globs() {
        let d = files();
        assert_parity_in(d.path(), "print -l ${x:-a*}bar; echo done");
        assert_parity_in(d.path(), "print -l pre${x:-*file}; echo done");
        assert_parity_in(d.path(), "print -l ${x:-*file}suf; echo done");
        assert_parity_in(d.path(), "p=pre; print -l ${p}${x:-*file}; echo done");
    }

    /// A parameter VALUE is never globbed (no GLOB_SUBST), even when the
    /// default word also carries a glob; quoted defaults stay literal.
    #[test]
    fn values_and_quoted_stay_literal() {
        let d = files();
        assert_parity_in(d.path(), "x='*file'; print -l $x");
        assert_parity_in(d.path(), "x='*file'; print -l ${x:-*other}");
        assert_parity_in(d.path(), "d='*file'; print -l ${x:-$d}");
        assert_parity_in(d.path(), "print -r -- \"${x:-*file}\"");
    }

    /// A `*` inside a NESTED strip/filter pattern within the default word
    /// is not a filename glob (`${x:-${p#a*}}`); pretokenize_src_pat skips
    /// the nested `${..}` span.
    #[test]
    fn nested_pattern_star_is_not_glob() {
        let d = files();
        assert_parity_in(d.path(), "p=abc; print -- ${x:-${p#a*}}");
        assert_parity_in(
            d.path(),
            "profile='user:custom'; print -- \"[${${${(M)profile:#*:*}:+${profile#*:}}:-default}]\"",
        );
    }

    /// Assignment contexts: scalar `v=` and assignment-builtin
    /// (typeset/export/...) NAME=value args do NOT glob; an array-literal
    /// element DOES glob; a regular command's `E=...` arg DOES glob.
    #[test]
    fn assignment_context_gating() {
        let d = files();
        assert_parity_in(d.path(), "v=${x:-*file}; print -l $v");
        assert_parity_in(d.path(), "typeset T=${x:-*file}; print $T");
        assert_parity_in(d.path(), "export E=${x:-*file}; print $E");
        assert_parity_in(d.path(), "a=(${x:-*file}); print -l $a");
        assert_parity_in(d.path(), "print -l ${x:=*file}"); // := assign-default never globs
    }

    /// No-match honours nomatch/nullglob; flag doesn't leak across words.
    #[test]
    fn no_match_and_no_leak() {
        let d = files();
        assert_parity_in(d.path(), "print -l ${x:-zzz*}; echo done");
        assert_parity_in(d.path(), "setopt nullglob; print -l ${x:-zzz*}; echo after");
        assert_parity_in(
            d.path(),
            "y=Y; print -l ${y:-*nomatch}; print -l ${x:-*file}",
        );
    }
}
