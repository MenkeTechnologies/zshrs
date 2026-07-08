//! End-to-end regression tests for BUGS.md entries.
//!
//! Each fixed entry in `docs/BUGS.md` gets a subprocess parity test
//! here. If a fix regresses, the test fails before merge.
//!
//! Pattern mirrors `tests/recent_ports_parity.rs`: spawn the freshly
//! built `target/debug/zshrs` with `--zsh` and assert the stdout +
//! exit-code match the expected zsh-reference output.

#![allow(clippy::needless_raw_string_hashes)]
// Function names below preserve the literal zsh prompt-escape case
// (e.g. `%T` ≠ `%t`, `%D` ≠ `%d`) — non_snake_case is intentional.
#![allow(non_snake_case)]

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("target/debug/zshrs"),
        manifest.join("target/release/zshrs"),
    ]
    .into_iter()
    .find(|cand| cand.exists())
}

fn run_zshrs(script: &str) -> (i32, String, String) {
    let bin = match zshrs_bin() {
        Some(b) => b,
        None => {
            eprintln!("skip: zshrs binary not built");
            return (0, String::new(), String::new());
        }
    };
    let out = Command::new(&bin)
        .args(["--zsh", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin:?}: {e}"));
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #5 — print -P %j %T %D{…} escape dispatch
// Fix: src/ported/prompt.rs::putpromptchar
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug5_percent_j_prints_job_count_not_literal() {
    let (ec, stdout, _) = run_zshrs(r#"print -P "%j""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0");
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%j", "must NOT emit literal %j");
    assert!(
        trimmed.chars().all(|c| c.is_ascii_digit()),
        "%j must be a decimal job count, got {:?}",
        trimmed
    );
}

#[test]
fn bug5_percent_T_prints_hhmm_not_literal() {
    let (_, stdout, _) = run_zshrs(r#"print -P "%T""#);
    if zshrs_bin().is_none() {
        return;
    }
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%T", "must NOT emit literal %T");
    // zsh %T is %K:%M — hour has NO leading zero (zsh 5.9 at 00:50
    // prints "0:50"), so the shape is H:MM or HH:MM, 4-5 chars. The
    // old fixed len==5 assertion failed for ~10 hours of every day.
    let parts: Vec<&str> = trimmed.split(':').collect();
    assert_eq!(parts.len(), 2, "H:MM shape, got {:?}", trimmed);
    assert!(
        (1..=2).contains(&parts[0].len())
            && parts[1].len() == 2
            && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())),
        "H:MM digits, got {:?}",
        trimmed
    );
}

#[test]
fn bug5_percent_D_braces_year_emits_4_digit_year() {
    let (_, stdout, _) = run_zshrs(r#"print -P "%D{%Y}""#);
    if zshrs_bin().is_none() {
        return;
    }
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%D{%Y}", "%D{{...}} must NOT emit literally");
    let year: u32 = trimmed
        .parse()
        .unwrap_or_else(|_| panic!("expected 4-digit year, got {:?}", trimmed));
    assert!(year >= 2025, "year >= 2025, got {}", year);
}

#[test]
fn bug5_percent_bang_prints_history_number_not_literal() {
    let (_, stdout, _) = run_zshrs(r#"print -P "%!""#);
    if zshrs_bin().is_none() {
        return;
    }
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%!", "must NOT emit literal %!");
    // Allow leading minus for negative histct (defensive); body must
    // be decimal.
    let body = trimmed.strip_prefix('-').unwrap_or(trimmed);
    assert!(
        body.chars().all(|c| c.is_ascii_digit()),
        "%! must be a (signed) decimal, got {:?}",
        trimmed
    );
}

#[test]
fn bug5_percent_star_prints_hhmmss() {
    let (_, stdout, _) = run_zshrs(r#"print -P "%*""#);
    if zshrs_bin().is_none() {
        return;
    }
    let trimmed = stdout.trim();
    assert_ne!(trimmed, "%*", "must NOT emit literal %*");
    // zsh %* is %K:%M:%S — hour has no leading zero (see %T note
    // above), so 7-8 chars depending on the hour.
    let parts: Vec<&str> = trimmed.split(':').collect();
    assert_eq!(parts.len(), 3, "H:MM:SS shape, got {:?}", trimmed);
    assert!(
        (1..=2).contains(&parts[0].len())
            && parts[1].len() == 2
            && parts[2].len() == 2
            && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())),
        "H:MM:SS digits, got {:?}",
        trimmed
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #7 — local arr=( $=s ) word-split on IFS
// Fix: src/ported/builtin.rs::bin_typeset (=(…) paren-init handler)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug7_local_array_dollar_equals_splits_on_colon_ifs() {
    let (ec, stdout, stderr) = run_zshrs(
        r#"f() { local s=a:b:c IFS=:; local -a arr=( $=s ); echo "n=${#arr[@]} first=${arr[1]}"; }; f"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0 (stderr={:?})", stderr);
    assert!(
        stdout.contains("n=3"),
        "$= must split a:b:c on IFS=: into 3 fields, got {:?}",
        stdout
    );
    assert!(
        stdout.contains("first=a"),
        "first field must be 'a', got {:?}",
        stdout
    );
}

#[test]
fn bug7_typeset_a_array_dollar_equals_splits_on_default_ifs() {
    let (ec, stdout, stderr) = run_zshrs(
        r#"f() { local s="x y z"; typeset -a arr=( $=s ); echo "n=${#arr[@]} all=${arr[*]}"; }; f"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0 (stderr={:?})", stderr);
    assert!(
        stdout.contains("n=3"),
        "$= must split 'x y z' on default IFS into 3 fields, got {:?}",
        stdout
    );
    assert!(
        stdout.contains("all=x y z"),
        "joined output must be 'x y z', got {:?}",
        stdout
    );
}

#[test]
fn bug7_plain_array_element_no_dollar_equals_passes_through() {
    let (ec, stdout, stderr) = run_zshrs(
        r#"f() { local x=hello; local -a arr=( $x world "literal text" ); echo "n=${#arr[@]} last=${arr[-1]}"; }; f"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0 (stderr={:?})", stderr);
    // zsh truth (5.9, byte-verified): without $=, no splitting at
    // all — $x stays one word, the DQ "literal text" stays one word,
    // so n=3 and last is the intact quoted string. The old pin
    // asserted the broken split_whitespace pass (n=4) that has
    // since been fixed.
    assert!(
        stdout.contains("n=3 last=literal text"),
        "no splitting without $=, got {:?}",
        stdout
    );
}

#[test]
fn bug7_dollar_equals_on_unset_var_yields_empty_array() {
    let (ec, stdout, _) = run_zshrs(
        r#"f() { local -a arr=( $=zshrs_test_unset_var_xyz ); echo "n=${#arr[@]}"; }; f"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    assert!(
        stdout.contains("n=0"),
        "$= on unset var → empty array, got {:?}",
        stdout
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #563 — `zformat -F` (no args) error message format diverges
// Fix: src/ported/builtin.rs BUILTIN("zformat", ...) minargs 0→3,
// optstring "Faf"→None matching Src/Modules/zutil.c:2136.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug563_zformat_minus_F_no_args_emits_canonical_not_enough_arguments() {
    let (ec, _stdout, stderr) = run_zshrs(r#"zformat -F"#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "exit 1 expected");
    assert!(
        stderr.contains("not enough arguments"),
        "must emit canonical 'not enough arguments' (zsh dispatcher message), got stderr={:?}",
        stderr
    );
    assert!(
        !stderr.contains("missing arguments to"),
        "must NOT emit pre-fix Rust-only message, got stderr={:?}",
        stderr
    );
}

#[test]
fn bug563_zformat_minus_f_with_valid_args_still_works() {
    let (ec, stdout, stderr) =
        run_zshrs(r#"zformat -f r "[%n: %d]" n:Alice d:dept; print -- "$r""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0 expected (stderr={:?})", stderr);
    assert!(
        stdout.contains("[Alice: dept]"),
        "valid zformat -f still substitutes, got stdout={:?}",
        stdout
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #568 — `read -A` on empty input yields 0-elem array
// Fix: src/ported/builtin.rs bin_read array branch pushes empty
// element when no splits produced, mirroring Src/builtin.c:6929.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug568_read_minus_A_on_empty_input_yields_one_elem_empty_array() {
    let (_ec, stdout, _stderr) =
        run_zshrs(r#"read -A a </dev/null; echo "len=${#a} elem1=[${a[1]}]""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert!(
        stdout.contains("len=1"),
        "read -A on EOF must yield 1-element array (matching zsh), got stdout={:?}",
        stdout
    );
    assert!(
        stdout.contains("elem1=[]"),
        "the single element must be empty string, got stdout={:?}",
        stdout
    );
}

#[test]
fn bug568_read_minus_A_on_real_input_still_splits_correctly() {
    let (_ec, stdout, _stderr) = run_zshrs(
        r#"echo "a b c" | { read -A a; echo "len=${#a} e1=${a[1]} e2=${a[2]} e3=${a[3]}"; }"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert!(
        stdout.contains("len=3"),
        "non-empty input must still produce 3-element array, got stdout={:?}",
        stdout
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #572 — `print -S arg` emitted to stdout instead of silent
// Fix: src/ported/builtin.rs bin_print history branch now matches
// -s OR -S, mirroring Src/builtin.c:5047.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug572_print_minus_S_does_not_emit_to_stdout() {
    let (ec, stdout, _stderr) = run_zshrs(r#"print -S "a""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "exit 0 expected");
    assert_eq!(
        stdout, "",
        "print -S must save silently to history, not emit to stdout, got stdout={:?}",
        stdout
    );
}

#[test]
fn bug572_print_minus_S_with_multiple_args_rejects_per_c5058() {
    let (ec, _stdout, stderr) = run_zshrs(r#"print -S a b c"#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "exit 1 when -S given >1 arg per c:5058-5061");
    assert!(
        stderr.contains("option -S takes a single argument"),
        "must emit canonical -S single-arg error, got stderr={:?}",
        stderr
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #573 — `*(Lr)` glob qualifier emits "no matches found"
// instead of zsh's "number expected".
// Fix: src/ported/glob.rs parse_range_spec emits zerr("number
// expected") + sets errflag when no digit follows; zglob skips the
// "no matches found" path when errflag is set. Mirrors qgetnum at
// Src/glob.c:826-834.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug573_size_qualifier_no_digit_emits_canonical_number_expected() {
    // Use /tmp/_zg4 with at least one file so the glob would otherwise
    // produce matches.
    std::fs::create_dir_all("/tmp/_zg4_bug573_test").ok();
    std::fs::write("/tmp/_zg4_bug573_test/file_a", "").ok();
    let (ec, _stdout, stderr) = run_zshrs(r#"echo /tmp/_zg4_bug573_test/*(Lr)"#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "exit 1 expected");
    assert!(
        stderr.contains("number expected"),
        "must emit canonical 'number expected' for malformed size qualifier, got stderr={:?}",
        stderr
    );
    assert!(
        !stderr.contains("no matches found"),
        "must NOT emit 'no matches found' once 'number expected' fires, got stderr={:?}",
        stderr
    );
    std::fs::remove_dir_all("/tmp/_zg4_bug573_test").ok();
}

#[test]
fn bug573_size_qualifier_with_digit_still_works() {
    std::fs::create_dir_all("/tmp/_zg4_bug573_ok").ok();
    std::fs::write("/tmp/_zg4_bug573_ok/file_a", "").ok();
    let (ec, stdout, stderr) = run_zshrs(r#"echo /tmp/_zg4_bug573_ok/*(L0)"#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "valid (L0) still works (stderr={:?})", stderr);
    assert!(
        stdout.contains("/tmp/_zg4_bug573_ok/file_a"),
        "0-byte file matches (L0), got stdout={:?}",
        stdout
    );
    std::fs::remove_dir_all("/tmp/_zg4_bug573_ok").ok();
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #565 — `echo -e "\0NNN"` octal escape literal passthrough
// Fix: src/ported/utils.rs getkeystring_with added `Some('0')` arm
// for the no-OCTAL_ESC path mirroring Src/utils.c:7156-7178.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug565_echo_minus_e_zero_octal_decodes_byte() {
    let (ec, stdout, _stderr) = run_zshrs(r#"echo -e "\0101""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    assert!(
        stdout.starts_with("A"),
        "\\0101 must decode to 'A' (octal 101 = 65), got stdout={:?}",
        stdout
    );
}

#[test]
fn bug565_echo_minus_e_zero_octal_emits_NUL_byte() {
    let (_ec, stdout, _stderr) = run_zshrs(r#"echo -ne "x\0y" | od -An -c | head -1"#);
    if zshrs_bin().is_none() {
        return;
    }
    // od -c renders NUL as `\0` with 3 spaces of padding
    assert!(
        stdout.contains("\\0"),
        "embedded \\0 must be a real NUL byte in output, got od stdout={:?}",
        stdout
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #566 — `echo -e "\uNNNN"` / `echo -e "\UNNNNNNNN"` literal
// Fix: src/ported/utils.rs getkeystring_with added `Some('u')` (4-hex)
// and `Some('U')` (8-hex) arms mirroring Src/utils.c:7072-7138.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug566_echo_minus_e_uppercase_U_unicode_8hex_decodes() {
    let (ec, stdout, _stderr) = run_zshrs(r#"echo -e "\U00000041""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    assert!(
        stdout.starts_with("A"),
        "\\U00000041 must decode to 'A', got stdout={:?}",
        stdout
    );
}

#[test]
fn bug566_echo_minus_e_lowercase_u_unicode_4hex_decodes() {
    let (ec, stdout, _stderr) = run_zshrs(r#"echo -e "A""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    assert!(
        stdout.starts_with("A"),
        "\\u0041 must decode to 'A', got stdout={:?}",
        stdout
    );
}

#[test]
fn bug566_echo_minus_e_unicode_supplementary_plane() {
    // U+1F600 = grinning face emoji (4-byte UTF-8: F0 9F 98 80)
    let (ec, stdout, _stderr) = run_zshrs(r#"echo -e "\U0001F600""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    let bytes = stdout.as_bytes();
    assert!(
        bytes.starts_with(&[0xF0, 0x9F, 0x98, 0x80]),
        "U+1F600 must emit UTF-8 F0 9F 98 80, got first bytes={:02x?}",
        &bytes[..bytes.len().min(8)]
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #567 — `$'\UNNNNNNNN'` ANSI-C-quoting 8-hex Unicode literal
// Fix: getkeystring_with's new `Some('U')` arm covers the
// double-quoted-echo repro path; getkeystring_dollar_quote in lex.rs
// already handled the canonical $'...' path.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug567_ansi_c_quote_uppercase_U_decodes() {
    // The original repro: `zshrs --zsh -fc $'echo "\\U00000041"'`
    // which lands as `echo "\U00000041"` inside zshrs (double-quoted).
    let (ec, stdout, _stderr) = run_zshrs(r#"echo "\U00000041""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    assert!(
        stdout.starts_with("A"),
        "double-quoted \\U00000041 in echo must decode to 'A', got stdout={:?}",
        stdout
    );
}

#[test]
fn bug567_real_dollar_quote_uppercase_U_decodes() {
    // The actual `$'...'` ANSI-C path (lex.rs::getkeystring_dollar_quote).
    let (ec, stdout, _stderr) = run_zshrs(r#"echo $'\U00000041'"#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    assert!(
        stdout.starts_with("A"),
        "$'\\U00000041' must decode to 'A', got stdout={:?}",
        stdout
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #562 — `${a:U}` / `${a:C}` / `${a:W}` silently accepted.
// Fix: src/ported/subst.rs::modify emits unrecognized-modifier zerr
// per Src/subst.c:3786-3790 for non-modifier letters and for
// pre-flags consumed without a following modifier letter.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug562_uppercase_U_modifier_emits_canonical_diagnostic() {
    let (ec, _stdout, stderr) = run_zshrs(r#"a=hello; echo "${a:U}""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "rc must be 1");
    assert!(
        stderr.contains("unrecognized modifier `U'"),
        "must emit canonical error with letter, got stderr={:?}",
        stderr
    );
}

#[test]
fn bug562_uppercase_C_modifier_emits_canonical_diagnostic() {
    let (ec, _stdout, stderr) = run_zshrs(r#"a=hello; echo "${a:C}""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "rc must be 1");
    assert!(
        stderr.contains("unrecognized modifier `C'"),
        "must emit canonical error with letter, got stderr={:?}",
        stderr
    );
}

#[test]
fn bug562_lone_W_preflag_without_modifier_emits_bare_error() {
    // C source emits the bare form "unrecognized modifier" (no letter)
    // when the pre-flag W was consumed but no actual modifier letter
    // followed.
    let (ec, _stdout, stderr) = run_zshrs(r#"a=hello; echo "${a:W}""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "rc must be 1");
    assert!(
        stderr.contains("unrecognized modifier"),
        "must emit unrecognized-modifier diagnostic, got stderr={:?}",
        stderr
    );
    assert!(
        !stderr.contains("unrecognized modifier `W'"),
        "for bare-W case the zsh diagnostic has NO letter, got stderr={:?}",
        stderr
    );
}

#[test]
fn bug562_valid_lowercase_modifier_still_works() {
    let (ec, stdout, _stderr) = run_zshrs(r#"a=hello; echo "${a:u}""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "valid :u modifier must succeed");
    assert!(
        stdout.starts_with("HELLO"),
        "uppercase modifier output, got {:?}",
        stdout
    );
}

#[test]
fn bug562_substring_offset_still_works() {
    let (ec, stdout, _stderr) = run_zshrs(r#"a=hello; echo "${a:0:3}""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "substring offset must still work");
    assert!(
        stdout.starts_with("hel"),
        "substring [0:3] must yield 'hel', got {:?}",
        stdout
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #187 — `f() { :; } > /file` redirect on fn-def: the file is
// created at CALL time, not at definition time (C: par_funcdef stores
// the redir chain on the funcdef, Src/parse.c; doshfunc applies it on
// entry, Src/exec.c — Shfunc.redir). zsh 5.9 reference output for the
// probe below: "notyet\nx\n", rc=0. Re-verified at HEAD 2026-06-12.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug187_fn_def_redirect_applies_at_call_not_definition() {
    let (ec, stdout, stderr) = run_zshrs(
        r#"t=${TMPDIR:-/tmp}/zshrs_bug187_$$
command rm -f -- $t
f() { echo x; } > $t
[[ -e $t ]] || print notyet
f
print -r "$(< $t)"
command rm -f -- $t"#,
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "probe must exit 0 (stderr: {stderr:?})");
    assert_eq!(
        stdout, "notyet\nx\n",
        "file must NOT exist at definition time and must contain the \
         fn output after the call"
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #490 — `>&N` / `<&N` on an unopened fd errors with zsh's
// canonical diagnostic and rc=1 (C: the REDIR_MERGEOUT/MERGEIN dup
// path in Src/exec.c — dup fails EBADF → zwarn("%d: bad file
// descriptor") + execerr). zsh 5.9 reference: stderr
// "zsh:1: 5: bad file descriptor", rc=1, empty stdout.
// Re-verified at HEAD 2026-06-12.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug490_write_to_invalid_fd_errors_rc1() {
    let (ec, stdout, stderr) = run_zshrs("print x >&5");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "rc=1 on >&5 with fd 5 unopened");
    assert_eq!(stdout, "", "nothing written on the failed dup");
    assert!(
        stderr.contains("5: bad file descriptor"),
        "canonical diagnostic, got {:?}",
        stderr
    );
}

#[test]
fn bug490_read_from_invalid_fd_errors_rc1() {
    let (ec, stdout, stderr) = run_zshrs("read x <&5");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "rc=1 on <&5 with fd 5 unopened");
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("5: bad file descriptor"),
        "canonical diagnostic, got {:?}",
        stderr
    );
}

#[test]
fn bug490_valid_fd_dup_still_works() {
    let (ec, stdout, _stderr) = run_zshrs("print x >&1");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0, "valid fd 1 dup must succeed");
    assert_eq!(stdout, "x\n");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #631 — `(( a[i]++ ))` command exit status from element value
// Fix: src/extensions/compile_zsh.rs::compile_arith (subscripted branch)
// C ref: Src/exec.c:5267 `return (val.u.l == 0)`
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug631_assoc_postincr_unset_key_is_false() {
    // Post-increment value is the OLD value (0 for an unset key), so the
    // `(( ))` command exits 1 (false). This is the exact shape zinit's
    // `(( ZINIT[SOURCED]++ )) && return` reload-guard depends on.
    let (ec, out, _e) =
        run_zshrs("typeset -gA H; (( H[K]++ )) && echo RETURNED || echo CONTINUE; echo st=$?");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 0);
    assert_eq!(out, "CONTINUE\nst=0\n", "old value 0 → status 1 → && skips");
}

#[test]
fn bug631_assoc_postincr_nonzero_key_is_true() {
    let (_ec, out, _e) =
        run_zshrs("typeset -gA H; H[K]=5; (( H[K]++ )); echo st=$?; echo v=${H[K]}");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "st=0\nv=6\n", "old value 5 (non-zero) → status 0, one increment");
}

#[test]
fn bug631_array_postincr_and_assign_zero() {
    // `(( a[3]++ ))` on an unset slot → status 1; `(( a[3]=0 ))` → status 1.
    let (_ec, out, _e) = run_zshrs(
        "typeset -ga A; (( A[3]++ )); echo p=$?; (( A[3]=0 )); echo z=$?; (( A[3]=42 )); echo n=$?",
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "p=1\nz=1\nn=0\n", "value 0 → status 1, value 42 → status 0");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #632 — `export -T` / `readonly -T` attribute on tied scalar
// Fix: src/ported/builtin.rs tied-param creation
// C ref: Src/builtin.c:2986-2999
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug632_export_tied_scalar_is_exported() {
    let (_ec, out, _e) = run_zshrs("export -T FOO foo; echo ${(t)FOO}");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "scalar-tied-export\n");
}

#[test]
fn bug632_export_tied_typeset_p_prints_export() {
    let (_ec, out, _e) = run_zshrs("export -T FOO foo=(a b); typeset -p FOO");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "export -T FOO foo=( a b )\n");
}

#[test]
fn bug632_readonly_tied_scalar_is_readonly_not_exported() {
    // The array side must NOT carry PM_EXPORTED; readonly -T carries
    // readonly onto both, export onto neither.
    let (_ec, out, _e) = run_zshrs("readonly -T BAR bar; echo ${(t)BAR}");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "scalar-readonly-tied\n");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #633 — `__subexp_arr_*` scratch temp must not leak to paramtab
// Fix: src/extensions/subexp_cleanup.rs (RAII SubexpTempGuard)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug633_array_subexp_temp_does_not_leak() {
    // The value must still be correct, and no `__subexp_arr_N` may remain
    // visible in `typeset -p` afterward.
    let (_ec, out, _e) = run_zshrs(
        "x=${(s: :)$(echo a b c)}; print -r -- \"$x\"; \
         typeset -p 2>/dev/null | grep -c '^typeset -a __subexp_arr'",
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "a b c\n0\n", "value correct AND zero leaked temps");
}

#[test]
fn bug633_nested_array_subexp_temp_does_not_leak() {
    let (_ec, out, _e) = run_zshrs(
        "foo='a b c'; y=${${(s: :)foo}[2]}; print -r -- \"$y\"; \
         typeset -p 2>/dev/null | grep -c '^typeset -a __subexp_arr'",
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "b\n0\n");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #634 — `zle -l` vs `zle -lL` list forms (were swapped)
// Fix: src/ported/zle/zle_thingy.rs::scanlistwidgets
// C ref: Src/Zle/zle_thingy.c:533
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug634_zle_lL_emits_redefinable_form() {
    // -lL is the re-definable `zle -N name [fn]` form.
    let (_ec, out, _e) = run_zshrs("zle -N wa; zle -N wb mybody; zle -lL");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "zle -N wa\nzle -N wb mybody\n");
}

#[test]
fn bug634_zle_l_emits_abbreviated_form() {
    // plain -l is the abbreviated `name (fn)` form.
    let (_ec, out, _e) = run_zshrs("zle -N wa; zle -N wb mybody; zle -l");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "wa\nwb (mybody)\n");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #635 — reading `${keymaps}` must not wipe keybindings
// Fix: zle_keymap.rs + zleparameter.rs lazy-init gate on emptiness
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug635_reading_keymaps_preserves_bindings() {
    // A prior `bindkey` initialises the keymaps; reading ${keymaps}
    // afterward must NOT re-run the destructive default_bindings().
    let (_ec, out, _e) = run_zshrs(
        "bindkey '^Xa' beep; print -l ${(o)keymaps} >/dev/null; \
         bindkey -L | grep -c '\\^Xa'",
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "1\n", "the ^Xa binding must survive the ${{keymaps}} read");
}

#[test]
fn bug635_reading_keymaps_still_lists_full_default_set() {
    // No #383 regression: a bare ${keymaps} read (no prior bindkey) still
    // lazily populates the standard nine keymaps.
    let (_ec, out, _e) = run_zshrs("print -l ${(o)keymaps}");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(
        out, ".safe\ncommand\nemacs\nisearch\nmain\nvicmd\nviins\nviopp\nvisual\n",
        "bare ${{keymaps}} read still yields the full default keymap set"
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #636 — `[[ -n '[' ]]` conditional with a `[`/`]` operand
// Fix: src/ported/cond.rs::evalcond (dropped the bracket pre-filter)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug636_conditional_n_with_bracket_operand() {
    // A literal `[` or `]` is a non-empty string, so `-n` is true and
    // `-z` is false — in `[[ ]]`, `[ ]`, and `test` alike.
    let (_ec, out, _e) = run_zshrs(
        "[[ -n '[' ]] && print aT || print aF; \
         [[ -n ']' ]] && print bT || print bF; \
         [[ -z '[' ]] && print cT || print cF; \
         [ -n '[' ] && print dT || print dF; \
         test -n ']' && print eT || print eF",
    );
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "aT\nbT\ncF\ndT\neT\n");
}

#[test]
fn bug636_autopair_pair_lookup_and_binding() {
    // End-to-end: autopair's `_ap-get-pair` uses `[[ -n $1 ]]` + an assoc
    // keyed on `[`/`]`; the close-pair key `]` must bind to autopair-close.
    // (Skips cleanly if the plugin isn't installed.)
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return,
    };
    let ap = format!(
        "{home}/.zinit/plugins/hlissner---zsh-autopair/autopair.plugin.zsh"
    );
    if !std::path::Path::new(&ap).exists() {
        eprintln!("skip: autopair not installed");
        return;
    }
    let (_ec, out, _e) = run_zshrs(&format!("source {ap} 2>/dev/null; bindkey ']'"));
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "\"]\" autopair-close\n", "] must bind to autopair-close");
}
