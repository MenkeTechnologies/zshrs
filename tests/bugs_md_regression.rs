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
// Fix: src/ported/subst.rs::modify leaves the cursor on the offending
// text and the caller emits the unrecognized-modifier zerr per
// Src/subst.c:3797-3802. Two arms, both pinned below:
//   c:3799 `zerr("unrecognized modifier `%c\'", s[1])` when the
//          leftover starts with `:` and s[1] is not a token byte —
//          this covers `U`, `C` AND `W`, because an unusable pre-flag
//          rewinds to its own colon (c:4721, c:4727).
//   c:3801 `zerr("unrecognized modifier")` — bare, no letter — when
//          the leftover does not start with `:` (`${a:hX}`).
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
fn bug562_lone_W_preflag_without_modifier_emits_letter_diagnostic() {
    // `${a:W}` takes the LETTER form, not the bare one. Src/subst.c:4699
    // enters the `case 'W'` pre-flag arm, but every exit that fails to
    // settle on a modifier letter rewinds the cursor to the colon it
    // started from (c:4721 `*ptr = lptr; return;` in the switch default,
    // and c:4727-4728 for the `!c` exit). So back in modify()'s caller
    // the leftover text still begins with `:`, and c:3798-3799 takes the
    // `*s == ':' && !imeta(s[1])` branch:
    //     zerr("unrecognized modifier `%c'", s[1]);
    // Verified against the reference shell:
    //     $ zsh -fc 'a=hello; echo "${a:W}"'
    //     zsh:1: unrecognized modifier `W'
    //     rc=1
    let (ec, _stdout, stderr) = run_zshrs(r#"a=hello; echo "${a:W}""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "rc must be 1");
    assert_eq!(
        stderr, "zsh:1: unrecognized modifier `W\'\n",
        "must match zsh byte for byte, got stderr={:?}",
        stderr
    );
}

#[test]
fn bug562_trailing_junk_after_a_valid_modifier_emits_bare_error() {
    // The bare `unrecognized modifier` (no letter) arm at
    // Src/subst.c:3801 fires when the leftover text does NOT start with
    // a colon — i.e. a valid modifier ran and left junk glued to it.
    // `${a:hX}` is the canonical case: `h` is consumed, `X` remains, so
    // `*s != ':'` and C falls to the `else` arm. Verified:
    //     $ zsh -fc 'a=hello; echo "${a:hX}"'
    //     zsh:1: unrecognized modifier
    //     rc=1
    let (ec, _stdout, stderr) = run_zshrs(r#"a=hello; echo "${a:hX}""#);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(ec, 1, "rc must be 1");
    assert_eq!(
        stderr, "zsh:1: unrecognized modifier\n",
        "the bare arm carries no letter, got stderr={:?}",
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
    assert_eq!(
        out, "st=0\nv=6\n",
        "old value 5 (non-zero) → status 0, one increment"
    );
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
    assert_eq!(
        out, "p=1\nz=1\nn=0\n",
        "value 0 → status 1, value 42 → status 0"
    );
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
    assert_eq!(
        out, "1\n",
        "the ^Xa binding must survive the ${{keymaps}} read"
    );
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
    let ap = format!("{home}/.zinit/plugins/hlissner---zsh-autopair/autopair.plugin.zsh");
    if !std::path::Path::new(&ap).exists() {
        eprintln!("skip: autopair not installed");
        return;
    }
    let (_ec, out, _e) = run_zshrs(&format!("source {ap} 2>/dev/null; bindkey ']'"));
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(
        out, "\"]\" autopair-close\n",
        "] must bind to autopair-close"
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #637 — anonymous functions must not persist in $functions
// Fix: src/fusevm_bridge.rs::call_function (post-invoke anon removal)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug637_anonymous_function_does_not_persist() {
    let (_ec, out, _e) = run_zshrs(
        "before=${#functions}; () { : }; () { echo hi } arg >/dev/null; \
         print $before ${#functions}",
    );
    if zshrs_bin().is_none() {
        return;
    }
    // Both anon calls leave the function count exactly where it started.
    assert_eq!(out, "0 0\n");
}

#[test]
fn bug637_anonymous_function_preserves_exit_status() {
    // Removing the anon function must NOT clobber the body's $?.
    let (_ec, out, _e) = run_zshrs("() { return 3 }; print $?; () { false }; print $?");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(out, "3\n1\n");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #638 — `autoload -Uz /dir/name` names the function by basename
// Fix: src/ported/builtin.rs::add_autoload_function
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug638_autoload_full_path_names_by_basename() {
    use std::io::Write;
    // Create a function file in a temp dir and autoload it by full path.
    let dir = std::env::temp_dir().join(format!("zshrs_bug638_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let fpath = dir.join("myfunc");
    if let Ok(mut f) = std::fs::File::create(&fpath) {
        let _ = f.write_all(b"print loaded-body\n");
    }
    let (_ec, out, _e) = run_zshrs(&format!(
        "autoload -Uz {} 2>/dev/null; \
         print -rl -- ${{(M)${{(ok)functions}}:#*myfunc*}}; myfunc",
        fpath.display()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    if zshrs_bin().is_none() {
        return;
    }
    // Function named by BASENAME (not the full path), and it loads + runs.
    assert_eq!(out, "myfunc\nloaded-body\n");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #641 — adjacent expansions: empty ${(M)…:#}${x} kept literal
// Fix: src/ported/subst.rs::paramsubst (expand raw-expansion suffix)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug641_adjacent_expansion_after_empty_flag_strip() {
    let (_ec, out, _e) = run_zshrs(
        "x=HI; print -rn -- ${(M)0:#1}${x}; print; \
         typeset -A ICE=(); \
         [[ -n ${(M)${+ICE[wait]}:#1}${ICE[load]}${ICE[unload]} ]] && print T || print F",
    );
    if zshrs_bin().is_none() {
        return;
    }
    // ${x} expands to HI (not left literal), and the empty concat → F.
    assert_eq!(out, "HI\nF\n");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #642 — funcdef ending in `} always { … }` with its own `}` at
// end-of-input (no trailing newline) dropped the always-block close from
// body_source, so ${functions[f]} re-parsed as
// `par_subsh: 'always' block missing }`. Trigger: the .zwc source path
// (getpermtext re-emits the file with no trailing newline).
// Fix: src/ported/parse.rs par_funcdef / NAME() body-slice — strip the
// trailing `}` only when it is the excess (unmatched) funcdef brace.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug642_always_block_at_eof_keeps_close_brace() {
    use std::io::Write;
    // Write a funcdef whose body ends in a balanced `} always { … }` and
    // whose own closing `}` is the LAST byte of the file (no newline).
    let dir = std::env::temp_dir().join(format!("zshrs_bug642_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let fpath = dir.join("mini.zsh");
    if let Ok(mut f) = std::fs::File::create(&fpath) {
        // NOTE: no trailing newline after the final `}`.
        let _ = f.write_all(
            b"myfunc () {\n\tsetopt monitor || return\n\t{\n\t\tprint hello\n\t} always {\n\t\t(( $? )) && print cleanup\n\t}\n}",
        );
    }
    let (_ec, out, _e) = run_zshrs(&format!(
        "source {}; print -rn -- ${{functions[myfunc]}}",
        fpath.display()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    if zshrs_bin().is_none() {
        return;
    }
    // The rendered body must be brace-balanced (the always-block `}`
    // survives) and carry no parse-error text.
    assert!(
        !out.contains("missing"),
        "body_source lost the always-block close: {out:?}"
    );
    assert_eq!(
        out.matches('{').count(),
        out.matches('}').count(),
        "unbalanced braces in rendered body: {out:?}"
    );
    assert!(out.contains("always"), "always block dropped: {out:?}");
}

#[test]
fn bug642_funcdef_at_eof_with_brace_in_string() {
    use std::io::Write;
    // Guard the opposite failure mode: a body carrying an UNBALANCED
    // brace inside a string (`echo "{"`), with the funcdef `}` at EOF.
    // A naive "strip trailing `}` only when `}` > `{`" heuristic would
    // leave the funcdef `}` in place here (counts are equal) and the
    // re-parse would hit `parse error near }`. The fix strips the
    // funcdef `}` by position, not by brace balance.
    let dir = std::env::temp_dir().join(format!("zshrs_bug642b_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let fpath = dir.join("sb.zsh");
    if let Ok(mut f) = std::fs::File::create(&fpath) {
        let _ = f.write_all(b"sb () {\n\techo \"{\"\n}"); // no trailing newline
    }
    let (_ec, out, _e) = run_zshrs(&format!(
        "source {}; print -rn -- ${{functions[sb]}}",
        fpath.display()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    if zshrs_bin().is_none() {
        return;
    }
    assert!(
        !out.contains("near") && !out.contains("parse error"),
        "brace-in-string body left a stray funcdef `}}`: {out:?}"
    );
    // Rendered body is exactly the single statement, no funcdef braces.
    assert_eq!(out.trim(), "echo \"{\"", "unexpected render: {out:?}");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #643 — deep recursion must not crash; FUNCNEST guard matches
// zsh (default 500) instead of the old depth-80 clamp.
// Fix: bins/zshrs.rs (512MB stack thread) + vm_helper.rs (raise ceiling)
// + exec.rs::doshfunc (FUNCNEST guard on FS_FUNC depth).
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug643_infinite_recursion_errors_gracefully_no_crash() {
    // Runaway recursion must produce zsh's graceful error with rc 1,
    // NOT a stack-overflow SIGSEGV/SIGABRT (exit 134/139).
    let (ec, _out, err) = run_zshrs("f(){ f; }; f");
    if zshrs_bin().is_none() {
        return;
    }
    assert!(
        err.contains("maximum nested function level reached"),
        "expected FUNCNEST error, got stderr: {err:?}"
    );
    // rc must be a normal shell status, never a crash signal.
    assert!(
        ec != 134 && ec != 139,
        "infinite recursion crashed (exit {ec}) instead of erroring"
    );
}

#[test]
fn bug643_deep_finite_recursion_completes() {
    // Depth well past the old 80 clamp but under FUNCNEST=500 must run
    // to completion (zsh does), not falsely abort.
    let (_ec, out, _err) =
        run_zshrs("f(){ (( $1 >= 300 )) && { print DONE300; return }; f $(($1+1)); }; f 0");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(
        out.trim(),
        "DONE300",
        "deep recursion aborted early: {out:?}"
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #644 — ingetc was O(n²) per buffer (clone + chars().nth), so a
// large function body (p10k's 31KB `_p9k_init_icons` case) took tens of
// seconds to parse and froze the interactive shell.
// Fix: src/ported/input.rs::ingetc (byte-offset cache).
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug644_large_case_body_parses_in_linear_time() {
    // Build a big case with many wide branches — the exact shape that was
    // O(n²). Re-parse it via ${functions[f]} (getfunction → parse_string),
    // the read path that froze precmd. With the O(n²) regressed this takes
    // many seconds; linear it is well under a second even in debug.
    let filler: String = (0..150)
        .map(|i| format!("K{i} 'val{i}xxxxxxxxxxxxxxxxxxxx'"))
        .collect::<Vec<_>>()
        .join(" ");
    let branch = |name: &str| format!("({name}) icons=({filler}) ;;");
    let branches: String = (0..40)
        .map(|i| branch(&format!("mode{i}")))
        .collect::<Vec<_>>()
        .join("\n");
    let script = format!(
        "setopt extendedglob\nf() {{\ncase $1 in\n{branches}\n(*) icons=({filler}) ;;\nesac\n}}\nprint -rn -- ${{functions[f]}} | wc -c"
    );
    if zshrs_bin().is_none() {
        return;
    }
    let start = std::time::Instant::now();
    let (ec, out, err) = run_zshrs(&script);
    let elapsed = start.elapsed();
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    // The rendered body is non-trivial (thousands of bytes).
    let bytes: usize = out.trim().parse().unwrap_or(0);
    assert!(bytes > 1000, "rendered body too small: {out:?}");
    // Perf guard: the O(n²) version took tens of seconds for this size;
    // a generous ceiling still catches a regression without flaking.
    assert!(
        elapsed.as_secs() < 10,
        "large case body parse took {elapsed:?} — ingetc O(n²) regressed?"
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #649 — for-loop variable alias-expanded, breaking the parse
// Fix: src/ported/parse.rs::par_for (noaliases/nocorrect guard)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug649_for_loop_variable_not_alias_expanded() {
    // The user's zpwr config sets `alias i='if [[ … ]]; then … fi'`.
    // When fzf's completion.zsh (`for i in {0..${#args[@]}}; do …`) is
    // parsed with that alias active, the loop VARIABLE `i` must NOT be
    // alias-expanded. It regressed because par_for read the variable in
    // command position, so exalias expanded `i` → `if`, and par_for saw
    // IF instead of the STRING name → "expected variable name in for".
    // `eval` reproduces the sourced-file timing (alias set BEFORE parse).
    if zshrs_bin().is_none() {
        return;
    }
    let script = "alias i='if [[ x ]]; then y; fi'\n\
                  eval 'g() { for i in {0..2}; do print -n \"v$i\"; done }'\n\
                  g";
    let (ec, out, err) = run_zshrs(script);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert!(
        !err.contains("expected variable name in for"),
        "loop variable was alias-expanded: {err:?}"
    );
    assert_eq!(out.trim(), "v0v1v2", "for-loop body ran (stderr={err:?})");
}

#[test]
fn bug649_arith_for_and_for_paren_still_work() {
    // The fix must not regress the paren-form loops (an earlier
    // incmdpos=0 approach did): arith-for `for (( ))` and the short
    // for-paren `for x (list)` both rely on inherited command position.
    if zshrs_bin().is_none() {
        return;
    }
    let (ec1, out1, err1) = run_zshrs("for (( i=0; i<3; i++ )); do print -n n$i; done; print \"\"");
    assert_eq!(ec1, 0, "arith-for exit 0 (stderr={err1:?})");
    assert_eq!(out1.trim(), "n0n1n2", "arith-for (stderr={err1:?})");

    let (ec2, out2, err2) = run_zshrs("for x (p q r); do print -n $x; done; print \"\"");
    assert_eq!(ec2, 0, "for-paren exit 0 (stderr={err2:?})");
    assert_eq!(out2.trim(), "pqr", "for-paren (stderr={err2:?})");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #650 — compinit never installed a `compdef` function
// Fix: ext_builtins::builtin_compinit installs a compdef stub;
//      fusevm_bridge BUILTIN_COMPDEF routes it to native builtin_compdef
// ════════════════════════════════════════════════════════════════════

// The compdef fix lives on the NATIVE compinit path (default mode); the
// shared `run_zshrs` uses `--zsh`, which runs the slow fpath shell
// compinit. Run these in default mode with an empty fpath (no scan).
fn run_zshrs_native(script: &str) -> (i32, String, String) {
    let bin = match zshrs_bin() {
        Some(b) => b,
        None => return (0, String::new(), String::new()),
    };
    let out = Command::new(&bin)
        .args(["-c", script])
        .env_remove("ZDOTDIR")
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin:?}: {e}"));
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn bug650_compinit_defines_compdef_function() {
    // zsh's `compinit` defines `compdef` as a shell function; zshrs does
    // the scan natively and never installed it, so `${+functions[compdef]}`
    // stayed 0. zinit's `.zinit-compdef-replay` checks exactly that and
    // aborts with "compinit function hasn't been loaded, cannot do compdef
    // replay", and direct `compdef` calls hit "command not found: compdef".
    if zshrs_bin().is_none() {
        return;
    }
    let script = "fpath=()\n\
                  autoload -Uz compinit\n\
                  compinit -u -D 2>/dev/null\n\
                  print \"exists=${+functions[compdef]}\"\n\
                  compdef _pip pip\n\
                  print \"rc=$?\"";
    let (ec, out, err) = run_zshrs_native(script);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert!(
        out.contains("exists=1"),
        "compdef function must exist: {out:?}"
    );
    assert!(out.contains("rc=0"), "compdef call must succeed: {out:?}");
    assert!(
        !err.contains("command not found: compdef"),
        "compdef must not be command-not-found: {err:?}"
    );
}

#[test]
fn bug650_user_compdef_override_still_wins() {
    // The stub must not shadow a genuine user/compsys compdef function.
    if zshrs_bin().is_none() {
        return;
    }
    let script = "fpath=()\n\
                  autoload -Uz compinit\n\
                  compinit -u -D 2>/dev/null\n\
                  compdef() { print \"USER:$*\"; }\n\
                  compdef _x y";
    let (ec, out, _err) = run_zshrs_native(script);
    assert_eq!(ec, 0);
    assert_eq!(
        out.trim(),
        "USER:_x y",
        "user compdef override must win: {out:?}"
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #651 — ${(P)<empty>} in a concatenated word dropped the word
// Fix: fusevm_bridge CONCAT_DISTRIBUTE(_FORCED) — empty array in a
//      concat contributes nothing (keeps surrounding literals)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug651_p_flag_empty_keeps_surrounding_literals() {
    // `(P)` is compiled as a distribute expansion (CONCAT_DISTRIBUTE_FORCED);
    // when it indirects to an unset/empty SCALAR the value collapses to an
    // empty array, and the forced cartesian dropped the WHOLE word (incl.
    // literals): `x${(P)ZZ}y` → "" instead of zsh's "xy". p10k's
    // `typeset -g _$2=${(P)2}` then arrived as a bare `typeset`, dumping
    // every parameter ~217× (19 MB terminal flood → startup hang).
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, out, err) = run_zshrs_native(
        "unset ZZ\n\
         print -r -- \"A:x${(P)ZZ}y\"\n\
         E=; T=E; print -r -- \"B:x${(P)T}y\"\n\
         b=(one two); bn=b; print -r -- \"C:x${(P)bn}y\"\n\
         s=(a ${(P)ZZ} b); print -r -- \"D:$#s\"\n\
         a=(); print -r -- \"E:x${^a}y\"",
    );
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert!(out.contains("A:xy"), "empty (P) keeps literals: {out:?}");
    assert!(
        out.contains("B:xy"),
        "set-empty (P) keeps literals: {out:?}"
    );
    assert!(
        out.contains("C:xone twoy"),
        "quoted array (P) joins: {out:?}"
    );
    assert!(out.contains("D:2"), "standalone (P) still removed: {out:?}");
    assert!(
        out.contains("E:xy"),
        "empty array concat keeps literals: {out:?}"
    );
}

#[test]
fn bug651_p_flag_empty_typeset_does_not_dump_params() {
    // The concrete hang trigger: typeset -g _NAME=${(P)unset} must assign
    // (empty) NOT dump every parameter.
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, out, err) =
        run_zshrs_native("unset ZZ; typeset -g _f=${(P)ZZ}; print \"got=${+parameters[_f]}\"");
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert!(out.contains("got=1"), "_f assigned empty: {out:?}");
    // A full param dump has hundreds of lines; a clean run has one.
    assert!(
        out.lines().count() < 20,
        "no param dump: {} lines",
        out.lines().count()
    );
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #652 — `<(cmd)` process substitution leaked a pipe fd (+child)
// Fix: fusevm_bridge::process_sub_in registers read_end in
//      PSUB_PENDING_FDS and reaps the child (note_psub_child)
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug652_process_substitution_does_not_leak_fds() {
    // `process_sub_in` kept the pipe read_end open for the whole shell
    // lifetime (never registered for close-after-command) and never
    // reaped the forked child. p10k's async worker / realtime clock run
    // `exec {fd}< <(cmd)` on every prompt, so each keystroke/redraw
    // leaked a pipe fd until the ~256-fd limit → the interactive shell
    // locked up (107 leaked pipes + 107 zombies observed live).
    if zshrs_bin().is_none() {
        return;
    }
    // 100 proc-subs, each explicitly closed. fd count must stay FLAT
    // (zsh: constant). The pre-fix port grew ~1 fd per proc-sub.
    let script = "for i in {1..100}; do exec {fd}< <(print hi); read l <&$fd; exec {fd}<&-; done\n\
                  print \"fds=$(ls /dev/fd | wc -l)\"";
    let (ec, out, err) = run_zshrs_native(script);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    let fds: usize = out
        .split_once("fds=")
        .and_then(|(_, r)| r.split_whitespace().next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(9999);
    // A leak would put this in the hundreds; a healthy shell keeps a
    // small constant handful.
    assert!(
        fds < 30,
        "process-sub fd leak: {fds} fds open after 100 proc-subs ({out:?})"
    );
}

#[test]
fn bug652_process_substitution_still_works() {
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, out, err) = run_zshrs_native(
        "diff <(print -l a b c) <(print -l a b c) && print SAME\ncat <(print hello)",
    );
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert!(out.contains("SAME"), "diff of identical proc-subs: {out:?}");
    assert!(out.contains("hello"), "cat proc-sub: {out:?}");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #1072 — `${(l:N:)#var}` dropped the padding.
// C runs the getlen block (Src/subst.c:3584-3615) and FALLS THROUGH to
// the padding blocks (c:4061+), so the decimal length gets padded.
// Fix: src/ported/subst.rs, getlen early-return path.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug1072_left_pad_applies_to_length() {
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, stdout, err) = run_zshrs(r#"foo=ab; print -r -- "${(l:5:)#foo}""#);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert_eq!(stdout, "    2\n", "length padded to width 5");
}

#[test]
fn bug1072_right_pad_with_fill_applies_to_length() {
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, stdout, err) = run_zshrs(r#"foo=ab; print -r -- "${(l:5::y:)#foo}""#);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert_eq!(stdout, "yyyy2\n", "length padded with the fill string");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #1073 — `(Z)` forced LEXFLAGS_ACTIVE so `${(Z::)v}` split.
// C's Z arm (c:2206-2237) only ORs sub-flag bits; the split test is
// `if (shsplit)` at c:3906.
// Fix: src/ported/subst.rs, `Z` flag arm + the split gate.
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug1073_Z_empty_subflags_does_not_split() {
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, stdout, err) = run_zshrs(r#"v='a b'; print -rl -- ${(Z::)v}"#);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert_eq!(stdout, "a b\n", "empty (Z::) sub-flag list must not split");
}

#[test]
fn bug1073_Z_comment_subflag_still_splits() {
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, stdout, err) = run_zshrs(r#"v='a b'; print -rl -- ${(Z:c:)v}"#);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert_eq!(stdout, "a\nb\n", "a non-empty sub-flag list splits");
}

// ════════════════════════════════════════════════════════════════════
// BUGS.md #1074 — bracket-delimited flag/qualifier arguments failed
// once the word was tokenized. get_strarg's delimiter switch
// (Src/subst.c:1366-1391) maps BOTH the raw ASCII brackets and the
// lexer's Inpar/Inang/Inbrace/Inbrack tokens.
// Fix: src/ported/subst.rs (get_strarg + l|r, s|j, I, Z, g arms),
//      src/ported/glob.rs (parse_uid_gid + the `e` qualifier arm).
// ════════════════════════════════════════════════════════════════════

#[test]
fn bug1074_split_paren_delim_under_length_operator() {
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, stdout, err) = run_zshrs(r#"s=aXbXc; print -r -- ${(ws(X))#s}"#);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert_eq!(stdout, "3\n", "tokenized (X) must close on Outpar");
}

#[test]
fn bug1074_Z_flag_paren_delim() {
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, stdout, err) = run_zshrs(r#"v='a b'; print -rl -- ${(Z(c))v}"#);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert_eq!(stdout, "a\nb\n", "(Z(c)) closes on `)`");
}

#[test]
fn bug1074_g_flag_bracket_delim() {
    if zshrs_bin().is_none() {
        return;
    }
    let (ec, stdout, err) = run_zshrs(r#"v='a\tb'; print -r -- ${(g[o])v}"#);
    assert_eq!(ec, 0, "exit 0 (stderr={err:?})");
    assert_eq!(stdout, "a\tb\n", "(g[o]) closes on `]`");
}

#[test]
fn bug1074_flagerr_message_untokenizes_the_body() {
    if zshrs_bin().is_none() {
        return;
    }
    // c:2289 — flagerr untokenizes its copy before printing, so the
    // parens are visible rather than raw token bytes.
    let (_, _, err) = run_zshrs(r#"foo=ab; print -r -- ${(_(x))#foo}"#);
    assert!(
        err.contains("in '${(_(x))#foo}'"),
        "flagerr body must be untokenized, got {err:?}"
    );
}

// ════════════════════════════════════════════════════════════════════
// Function bodies defined from STDIN (`-s`) or an interactive prompt
// printed as `name () { }`.
//
// C zsh never stores function source: `par_funcdef` (Src/parse.c:1672)
// compiles the body into the `ecbuf` wordcode and `functions` rebuilds the
// text from it via `getpermtext`/`gettext2` (Src/text.c:189/296). zshrs's
// fusevm path builds no `Eprog`, so `printshfuncnode`
// (src/ported/hashtable.rs:1796) prints the raw source the parser kept —
// and that capture was a slice of the Rust-only `LEX_INPUT` window, which
// only exists for file / `-c` / `eval` input. Interactive and `-s` stdin
// input reaches the lexer through `hgetc` -> `ingetc` -> `inbuf`, so
// `LEX_POS` never moved and the body came out empty. C's `chline` is no
// substitute: `hbegin` (Src/hist.c:1119) leaves it NULL whenever the shell
// is not interactive-on-stdin.
//
// Fix: src/extensions/funcdef_capture.rs — echo every character `hgetc`
// returns into a capture buffer while a function body is being parsed.
// ════════════════════════════════════════════════════════════════════

/// Feed `script` on stdin with `--zsh -f -s` (the path with no `LEX_INPUT`).
fn run_zshrs_stdin(script: &str) -> String {
    use std::io::Write;
    let bin = match zshrs_bin() {
        Some(b) => b,
        None => return String::new(),
    };
    let mut child = Command::new(&bin)
        .args(["--zsh", "-f", "-s"])
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {bin:?}: {e}"));
    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn funcdef_body_survives_stdin_input() {
    let out = run_zshrs_stdin("tom(){ pwd; id }\nfunctions tom\n");
    if zshrs_bin().is_none() {
        return;
    }
    // Exactly what `zsh -f -s` prints for the same input.
    assert_eq!(out, "tom () {\n\tpwd\n\tid\n}\n", "stdin funcdef body lost");
}

#[test]
fn funcdef_body_survives_stdin_multiline_and_heredoc() {
    // A multi-line body with a here-document: `gethere` (Src/exec.c:4625)
    // reads the body with `hgetc` and then RE-LEXES it through `parsestr`
    // (c:4697), so a naive echo buffer duplicates every here-doc line. The
    // capture reuses hgetc's `counts_lineno` gate (c:Src/input.c:330) to
    // record only the first pass.
    let out = run_zshrs_stdin("hd() {\n  cat <<END\nhello\nEND\n  pwd\n}\nfunctions hd\n");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(
        out, "hd () {\n\tcat <<END\nhello\nEND\n\tpwd\n}\n",
        "stdin here-doc body wrong"
    );
}

#[test]
fn funcdef_body_survives_stdin_nested_definition() {
    // Nested funcdefs open two captures; the inner one must not swallow the
    // outer one's text, and closing the inner must not close the outer.
    let out = run_zshrs_stdin("outer(){ inner(){ echo deep }; inner }\nfunctions outer\n");
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(
        out, "outer () {\n\tinner () {\n\t\techo deep\n\t}\n\tinner\n}\n",
        "stdin nested funcdef body wrong"
    );
}

#[test]
fn funcdef_body_stdin_matches_file_and_dash_c() {
    // The three non-tty input paths must agree; before the fix only the
    // file and `-c` paths carried a body.
    let script = "q() { print \"a b\"; print 'c;d' }\nshort() print x\nfunction kw { print kw }\nfunctions q short kw\n";
    let from_stdin = run_zshrs_stdin(script);
    let (_ec, from_dash_c, _e) = run_zshrs(script);
    if zshrs_bin().is_none() {
        return;
    }
    assert_eq!(
        from_stdin, from_dash_c,
        "stdin and -c disagree on body text"
    );
    assert!(
        from_stdin.contains("print \"a b\"") && from_stdin.contains("print x"),
        "bodies missing: {from_stdin:?}"
    );
}
