//! zsh-specific numeric-range glob `<N-M>` parity tests.
//! `<->` = any positive integer.
//! `<N-M>` = N to M inclusive.
//! `<N->`, `<-M>` = open ranges.

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
fn assert_parity_in(d: &Path, s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh_in(d, s);
    let r = run_zshrs_in(d, s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}
fn tdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}
fn make_files(d: &Path, names: &[&str]) {
    for n in names {
        std::fs::write(d.join(n), "").unwrap();
    }
}

mod any_positive_int {
    use super::*;

    /// `file<->` matches file1, file2, ... but not fileabc.
    #[test]
    fn dash_matches_any_integer_suffix() {
        let d = tdir();
        make_files(d.path(), &["file1", "file2", "file10", "fileabc", "file"]);
        assert_parity_in(d.path(), "print -l file<-> | sort");
    }

    /// `<->` standalone matches any integer-named file.
    #[test]
    fn standalone_dash_matches_integer_names() {
        let d = tdir();
        make_files(d.path(), &["1", "2", "10", "100", "abc"]);
        assert_parity_in(d.path(), "print -l <-> | sort -n");
    }

    /// Empty match → no file expansion error (with -f, nomatch active).
    #[test]
    fn no_integer_files_nomatch_error() {
        let d = tdir();
        make_files(d.path(), &["abc", "def"]);
        assert_parity_in(d.path(), "print -l <-> 2>/dev/null; echo done");
    }
}

mod closed_range {
    use super::*;

    /// `<5-10>` matches 5..10 only.
    #[test]
    fn closed_range_5_to_10() {
        let d = tdir();
        make_files(d.path(), &["3", "5", "7", "10", "11"]);
        assert_parity_in(d.path(), "print -l <5-10> | sort -n");
    }

    /// Range with prefix.
    #[test]
    fn closed_range_with_prefix() {
        let d = tdir();
        make_files(d.path(), &["a1", "a3", "a5", "a10", "a99"]);
        assert_parity_in(d.path(), "print -l a<3-10> | sort");
    }

    /// Range with suffix.
    #[test]
    fn closed_range_with_suffix() {
        let d = tdir();
        make_files(d.path(), &["1.txt", "5.txt", "10.txt", "100.txt"]);
        assert_parity_in(d.path(), "print -l <2-50>.txt | sort -n");
    }
}

mod open_range_upper {
    use super::*;

    /// `<5->` matches 5, 6, 7, ... any int >= 5.
    #[test]
    fn open_upper_5_and_above() {
        let d = tdir();
        make_files(d.path(), &["3", "5", "7", "100", "1000"]);
        assert_parity_in(d.path(), "print -l <5-> | sort -n");
    }
}

mod open_range_lower {
    use super::*;

    /// `<-10>` matches anything <= 10.
    #[test]
    fn open_lower_le_10() {
        let d = tdir();
        make_files(d.path(), &["1", "5", "10", "11", "50"]);
        assert_parity_in(d.path(), "print -l <-10> | sort -n");
    }
}

mod single_number {
    use super::*;

    /// `<5-5>` matches only "5".
    #[test]
    fn single_value_range() {
        let d = tdir();
        make_files(d.path(), &["4", "5", "6"]);
        assert_parity_in(d.path(), "print -l <5-5>");
    }
}

mod combined_with_other_globs {
    use super::*;

    /// `file*<1-5>.log` combines * and numeric.
    #[test]
    fn star_then_numeric_range() {
        let d = tdir();
        make_files(
            d.path(),
            &["fileA1.log", "fileB3.log", "fileC10.log", "fileD5.log"],
        );
        assert_parity_in(d.path(), "print -l file*<1-5>.log | sort");
    }
}

mod negation {
    use super::*;

    /// With EXTENDED_GLOB: `^<1-5>` files NOT in numeric range.
    #[test]
    fn extended_glob_negation_with_range() {
        let d = tdir();
        make_files(d.path(), &["1", "2", "3", "10", "20", "100"]);
        assert_parity_in(
            d.path(),
            "setopt extended_glob; print -l ^<1-5> 2>/dev/null | sort -n",
        );
    }
}

mod leading_zeros {
    use super::*;

    /// `<1-10>` matches "07" too? zsh interprets as numeric.
    #[test]
    fn leading_zero_in_filename() {
        let d = tdir();
        make_files(d.path(), &["07", "8", "10"]);
        assert_parity_in(d.path(), "print -l <1-10> | sort -n");
    }
}

mod negative_number_excluded {
    use super::*;

    /// `<->` doesn't match "-5".
    #[test]
    fn negative_number_not_matched() {
        let d = tdir();
        make_files(d.path(), &["1", "5", "-5", "abc"]);
        assert_parity_in(d.path(), "print -l <-> | sort -n");
    }
}

mod large_numbers {
    use super::*;

    /// `<1000-2000>` works for large ranges.
    #[test]
    fn large_range_thousand() {
        let d = tdir();
        make_files(d.path(), &["999", "1000", "1500", "2000", "2001"]);
        assert_parity_in(d.path(), "print -l <1000-2000> | sort -n");
    }
}
