//! Numeric-sort parity: `${(n)}` / `${(ni)}` / `${(On)}` and the glob-level
//! `(n)` qualifier + `NUMERIC_GLOB_SORT`, all driven by `zstrcmp`'s
//! `sortnumeric` block (`Src/sort.c:137-172`).
//!
//! Every probe below is a case where the two shells DID diverge before
//! `Src/sort.c:155`'s `if (idigit(*as) && idigit(*bs))` guard was ported and
//! before the `${(...)}` sort call site was collapsed onto C's single
//! `strmetasort(aval, sortit, NULL)` (`Src/subst.c:4045`).
//!
//! Scope note: only inputs whose ordering is decided by the digit-run
//! comparison, or by digit-vs-letter collation (stable in both the C and
//! en_US.UTF-8 locales), appear here. zsh's numeric comparator is not a
//! strict weak order — `zsh -fc 'a=(z1 z10Z2 z0019 Z0zZ_z); print ${(n)a}'`
//! answers differently depending on the INPUT order — so any probe whose
//! result depends on qsort's pivot choices is deliberately excluded.

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
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Byte-compare stdout + exit status against the real shell, and pin the
/// expected text so the test still states an answer when zsh is absent
/// (headless CI images without zsh installed still run the second half).
fn assert_parity_eq(s: &str, expected: &str) {
    if zsh_available() {
        let z = run_zsh(s);
        let r = run_zshrs(s);
        assert_eq!(
            z.stdout, r.stdout,
            "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
            z.stdout, r.stdout
        );
        assert_eq!(z.exit, r.exit, "exit-status divergence on:\n{s}");
        assert_eq!(
            z.stdout, expected,
            "the real shell no longer produces the pinned text for:\n{s}"
        );
    } else {
        let r = run_zshrs(s);
        assert_eq!(r.stdout, expected, "zshrs output for:\n{s}");
    }
}

/// A digit-initial element must collate among the letters, not after them.
/// `Src/sort.c:155` refuses the digit-run comparison unless BOTH sides have
/// a digit at the rewound position, so `9` vs `a` keeps the plain collation
/// answer. Comparing runs unconditionally made the 1-digit run beat the
/// empty run and parked every number at the end.
#[test]
fn numeric_sort_places_digit_initial_elements_among_letters() {
    assert_parity_eq("a=(1 a 2 b 10 c); print -r -- ${(n)a}", "1 2 10 a b c\n");
    assert_parity_eq("a=(9 a); print -r -- ${(n)a}", "9 a\n");
    assert_parity_eq("a=(a 9); print -r -- ${(n)a}", "9 a\n");
}

/// Same guard, but with the divergence in the middle of the string: `x1`
/// vs `xa` rewinds to index 1, where only one side is a digit.
#[test]
fn numeric_sort_one_sided_digit_run_mid_string_keeps_collation() {
    assert_parity_eq("a=(xa x1); print -r -- ${(n)a}", "x1 xa\n");
    assert_parity_eq("a=(a10 a9 ab); print -r -- ${(n)a}", "a9 a10 ab\n");
    assert_parity_eq("a=(z3 z30 z300 zz); print -r -- ${(n)a}", "z3 z30 z300 zz\n");
}

/// `(O)` reverses the same comparator rather than reversing a lexical sort.
#[test]
fn numeric_descending_sort_reverses_the_numeric_order() {
    assert_parity_eq("a=(1 a 2 b 10 c); print -r -- ${(On)a}", "c b a 10 2 1\n");
}

/// `(ni)` must apply BOTH flags. C makes one `strmetasort(aval, sortit,
/// NULL)` call (`Src/subst.c:4045`) whose pre-pass lowers the compare keys
/// before the numeric comparator runs; a call site that treats
/// numeric-vs-case-fold as exclusive branches compares `X` against `x`,
/// finds no digit run there, and falls through to collation.
#[test]
fn case_insensitive_numeric_sort_folds_case_before_comparing_digit_runs() {
    assert_parity_eq("a=(X2 x10); print -r -- ${(ni)a}", "X2 x10\n");
    assert_parity_eq("a=(B9 b10); print -r -- ${(ni)a}", "B9 b10\n");
    assert_parity_eq("a=(B9 b10 a); print -r -- ${(oni)a}", "a B9 b10\n");
    assert_parity_eq("a=(B9 b10 a); print -r -- ${(Oni)a}", "b10 B9 a\n");
}

/// Numbers must still beat each other numerically (the control case that
/// stays green whether or not the one-sided guard exists).
#[test]
fn numeric_sort_compares_digit_runs_by_value_not_lexically() {
    assert_parity_eq(
        "a=(f1 f10 f2 f100 f0 f01); print -r -- ${(n)a}",
        "f0 f01 f1 f2 f10 f100\n",
    );
    assert_parity_eq("a=(v1.2 v1.10 v1.9); print -r -- ${(n)a}", "v1.2 v1.9 v1.10\n");
}

/// The glob-qualifier `(n)` and the `NUMERIC_GLOB_SORT` option route the
/// same comparator through `gmatchcmp`'s `GS_NAME` arm (`Src/glob.c:945`),
/// so the digit-initial names must interleave there too.
#[test]
fn glob_numeric_sort_matches_zsh_for_digit_initial_names() {
    let dir = std::env::temp_dir().join(format!(
        "zshrs-numglobsort-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    for f in ["9", "10", "a", "z", "f1", "f2", "f10"] {
        std::fs::write(dir.join(f), b"").expect("touch");
    }
    let d = dir.to_str().expect("utf8 tmpdir");
    let expected = "9\n10\na\nf1\nf2\nf10\nz\n";
    assert_parity_eq(
        &format!("cd {d} && print -rl -- *(n)"),
        expected,
    );
    assert_parity_eq(
        &format!("setopt numericglobsort; cd {d} && print -rl -- *"),
        expected,
    );
    let _ = std::fs::remove_dir_all(&dir);
}
