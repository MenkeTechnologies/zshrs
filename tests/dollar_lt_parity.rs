//! `$(< file)` command-substitution-from-file shortcut parity.

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

mod single_line {
    use super::*;

    #[test]
    fn reads_single_line_file() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "hello\n").unwrap();
        assert_parity_in(d.path(), "echo [$(< in.txt)]");
    }

    /// Trailing newline stripped (same as `$(cat file)`).
    #[test]
    fn trailing_newline_stripped() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "value\n").unwrap();
        assert_parity_in(d.path(), "X=$(< in.txt); echo \"[$X]\"");
    }
}

mod multiline {
    use super::*;

    #[test]
    fn reads_multi_line_file() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "one\ntwo\nthree\n").unwrap();
        assert_parity_in(d.path(), "X=$(< in.txt); echo \"$X\"");
    }

    /// Internal newlines preserved.
    #[test]
    fn internal_newlines_preserved() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "line1\nline2\nline3\n").unwrap();
        assert_parity_in(d.path(), r#"X=$(< in.txt); echo "$X" | wc -l"#);
    }
}

mod empty {
    use super::*;

    /// Empty file → empty string.
    #[test]
    fn empty_file_yields_empty() {
        let d = tdir();
        std::fs::write(d.path().join("empty.txt"), "").unwrap();
        assert_parity_in(d.path(), "X=$(< empty.txt); echo \"[$X]\"");
    }

    /// Just a newline → empty (trailing newline stripped).
    #[test]
    fn newline_only_yields_empty() {
        let d = tdir();
        std::fs::write(d.path().join("nl.txt"), "\n").unwrap();
        assert_parity_in(d.path(), "X=$(< nl.txt); echo \"[$X]\"");
    }
}

mod missing_file {
    use super::*;

    /// Missing file → error (stderr) + empty result, exit nonzero.
    #[test]
    fn missing_file_errors() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            "X=$(< nonexistent_xyz.txt) 2>/dev/null; echo \"[$X]\"; echo $?",
        );
    }
}

mod with_var_path {
    use super::*;

    #[test]
    fn file_path_from_variable() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "content\n").unwrap();
        assert_parity_in(d.path(), "F=in.txt; X=$(< $F); echo $X");
    }
}

mod in_pipeline {
    use super::*;

    #[test]
    fn piped_through_grep() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "alpha\nbeta\ngamma\n").unwrap();
        assert_parity_in(d.path(), "echo \"$(< in.txt)\" | grep beta");
    }
}

mod cat_equivalence {
    use super::*;

    /// `$(< file)` should equal `$(cat file)` (and avoid forking).
    #[test]
    fn equivalent_to_cat_subst() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "alpha\nbeta\n").unwrap();
        assert_parity_in(
            d.path(),
            r#"
A=$(< in.txt)
B=$(cat in.txt)
[[ "$A" == "$B" ]]; echo $?
"#,
        );
    }
}

mod nested {
    use super::*;

    /// Nested inside `echo "...$(<file)...":
    #[test]
    fn nested_inside_double_quote_echo() {
        let d = tdir();
        std::fs::write(d.path().join("name.txt"), "world").unwrap();
        assert_parity_in(d.path(), r#"echo "hello, $(< name.txt)!""#);
    }

    /// `$(<$(< pathfile))` — nested.
    #[test]
    fn nested_via_inner_subst() {
        let d = tdir();
        std::fs::write(d.path().join("path.txt"), "data.txt").unwrap();
        std::fs::write(d.path().join("data.txt"), "actual content").unwrap();
        assert_parity_in(d.path(), r#"echo "$(< $(< path.txt))""#);
    }
}

mod special_chars {
    use super::*;

    /// File with embedded `$`, `*`, etc. — content stays literal.
    #[test]
    fn file_with_dollar_and_star() {
        let d = tdir();
        std::fs::write(d.path().join("in.txt"), "$X *.txt\n").unwrap();
        assert_parity_in(d.path(), r#"X=$(< in.txt); echo "[$X]""#);
    }

    /// Binary content (non-UTF8) — read as bytes.
    /// Just compare wc -c (byte count).
    #[test]
    fn binary_content_byte_count() {
        let d = tdir();
        // Write file with various byte values.
        let bytes: Vec<u8> = (0u8..=127).collect();
        std::fs::write(d.path().join("bytes.dat"), &bytes).unwrap();
        assert_parity_in(d.path(), "X=$(< bytes.dat); printf '%s' \"$X\" | wc -c");
    }
}
