//! C-parity pins for `print` builtin flag interactions.
//!
//! Pins coverage gaps from `tests/builtin_print_parity.rs` —
//! `-N` (NUL-separated), `-c` (single column), `-C N` (N columns),
//! and combined-flag forms (`-nl`, `-nlr`, `-nN`, `-N --`, `-rln`).
//! Source spec: `src/zsh/Src/builtin.c` `bin_print()`.

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
    stdout: Vec<u8>,
    exit: i32,
}
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: o.stdout,
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
        stdout: o.stdout,
        exit: o.status.code().unwrap_or(-1),
    }
}
/// Compare raw bytes — NUL separator from -N must round-trip exactly.
fn assert_parity_bytes(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout bytes divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit code divergence on:\n{s}");
}

mod print_N_null_separator {
    use super::*;

    /// `print -N a b c` emits `a\0b\0c\0` (NUL after every arg including last).
    #[test]
    fn print_N_three_args_nul_separated() {
        assert_parity_bytes("print -N a b c");
    }

    /// `print -N` single arg — `a\0`.
    #[test]
    fn print_N_single_arg_terminating_nul() {
        assert_parity_bytes("print -N hello");
    }

    /// `print -N --` followed by args that look like options.
    #[test]
    fn print_N_double_dash_terminator() {
        assert_parity_bytes("print -N -- -n -l");
    }

    /// `print -nN` — N forces NUL terminator AND -n suppresses extra newline.
    /// In zsh -N implies -n behavior (no trailing newline), so combined is idempotent.
    #[test]
    fn print_nN_combined_no_extra_newline() {
        assert_parity_bytes("print -nN a b");
    }

    /// `print -N` with empty arg list — produces nothing.
    #[test]
    fn print_N_no_args_empty_output() {
        assert_parity_bytes("print -N");
    }
}

mod print_C_columnar {
    use super::*;

    /// `print -c args` is shorthand for `-C 1` — one column, one per line.
    #[test]
    fn print_c_lowercase_one_column() {
        assert_parity_bytes("print -c one two three four five");
    }

    /// `print -C 2` lays args out in 2 columns, filling top-to-bottom first.
    #[test]
    fn print_C_two_columns_even_count() {
        assert_parity_bytes("print -C 2 a b c d e f");
    }

    /// `print -C 3` with 9 args — perfect 3x3 grid.
    #[test]
    fn print_C_three_columns_full_grid() {
        assert_parity_bytes("print -C 3 one two three four five six seven eight nine");
    }

    /// `print -C 2` with odd count — final column under-fills.
    #[test]
    fn print_C_two_columns_odd_count() {
        assert_parity_bytes("print -C 2 a b c");
    }

    /// `print -C 1` is equivalent to `print -l`.
    #[test]
    fn print_C_one_column_equals_l() {
        assert_parity_bytes("print -C 1 a b c");
    }
}

mod print_combined_flags {
    use super::*;

    /// `-ln` together — one-per-line but no trailing newline after last.
    #[test]
    fn print_ln_combined_no_trailing_newline() {
        assert_parity_bytes("print -ln a b c; print done");
    }

    /// `-nl` (reversed order) — same semantics as -ln.
    #[test]
    fn print_nl_order_independent() {
        assert_parity_bytes("print -nl a b c; print done");
    }

    /// `-rln` raw + one-per-line + no newline. Embedded \n stays literal.
    #[test]
    fn print_rln_raw_one_per_line_no_newline() {
        assert_parity_bytes(r#"print -rln 'a\n' 'b\t' 'c'"#);
    }

    /// `-ln --` then args resembling flags.
    #[test]
    fn print_ln_double_dash_terminator() {
        assert_parity_bytes("print -ln -- -r -E hello");
    }

    /// `-R` capital R = bsd echo-compat raw mode; combined with `-n`.
    /// Should NOT treat subsequent `-n` as a flag once -R is in play.
    #[test]
    fn print_R_then_n_arg_passes_through() {
        assert_parity_bytes("print -R hello -n world; echo done");
    }
}
