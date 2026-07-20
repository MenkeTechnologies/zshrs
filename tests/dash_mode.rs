//! Tests for `zshrs --dash` strict-dash (Debian Almquist Shell) mode.
//!
//! dash is behaviourally `sh` for every emulation option, so `--dash`
//! sets the same `EMULATE_SH` presets as `--sh` (verified below). On top
//! of that it raises the Rust-only `DASH_STRICT` flag
//! (src/extensions/dash_mode.rs), which rejects the zsh syntactic
//! extensions dash has never had. Each rejection audited against real
//! `/bin/dash` is pinned here two ways:
//!   1. a self-contained assertion on `zshrs --dash` output (runs in any
//!      CI, no reference shell required), and
//!   2. a byte-parity differential against `/bin/dash` when that binary
//!      is present (the curated-corpus harness; skipped otherwise).

use std::path::Path;
use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// Run `zshrs --dash -f -c <script>` → (stdout, exit-code). `-f` skips
/// rc files so the result depends only on the mode, not the environment.
fn run_dash_mode(script: &str) -> (String, i32) {
    let out = Command::new(zshrs_bin())
        .args(["--dash", "-f", "-c", script])
        .output()
        .expect("zshrs --dash failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Query an emulation option via the `$options` associative array —
/// reliable across emulations, unlike bare `setopt`'s listing whose
/// format shifts under sh/ksh/posixbuiltins.
fn option_on(name: &str) -> bool {
    let (out, _) = run_dash_mode(&format!("print -r -- ${{options[{name}]}}"));
    out.trim() == "on"
}

// ── dash is sh for options: EMULATE_SH presets must be set ──────────────

#[test]
fn dash_mode_sets_shwordsplit() {
    assert!(option_on("shwordsplit"));
    // ... and it is behaviourally active.
    let (out, _) = run_dash_mode("v=\"a b c\"; set -- $v; echo $#");
    assert_eq!(out.trim(), "3");
}

#[test]
fn dash_mode_sets_posixbuiltins() {
    assert!(option_on("posixbuiltins"));
}

#[test]
fn dash_mode_sets_ksharrays() {
    // dash arrays don't exist, but the sh option preset still applies.
    assert!(option_on("ksharrays"));
}

#[test]
fn dash_mode_matches_sh_option_presets() {
    // dash IS sh for every emulation option — the two modes must agree on
    // the full EMULATE_SH delta set.
    for opt in [
        "shwordsplit",
        "posixbuiltins",
        "ksharrays",
        "shglob",
        "bsdecho",
    ] {
        let dash = run_dash_mode(&format!("print -r -- ${{options[{opt}]}}")).0;
        let sh = Command::new(zshrs_bin())
            .args([
                "--sh",
                "-f",
                "-c",
                &format!("print -r -- ${{options[{opt}]}}"),
            ])
            .output()
            .expect("spawn");
        assert_eq!(
            dash.trim(),
            String::from_utf8_lossy(&sh.stdout).trim(),
            "option `{opt}` differs between --dash and --sh"
        );
    }
}

// ── DASH_STRICT rejections: self-contained (no reference shell) ─────────

#[test]
fn dash_rejects_arith_power() {
    // `**` is not a dash arithmetic operator → error, no output.
    let (out, code) = run_dash_mode("echo $((2**10))");
    assert_eq!(out, "");
    assert_ne!(code, 0, "`**` should error under --dash");
}

#[test]
fn dash_rejects_arith_comma() {
    let (out, code) = run_dash_mode("echo $((1,2))");
    assert_eq!(out, "");
    assert_ne!(code, 0, "arith `,` should error under --dash");
}

#[test]
fn dash_rejects_arith_base_num() {
    // POSIX arithmetic (dash/ash) has NO `base#num` syntax — only decimal,
    // `0` octal, `0x` hex. `16#ff` etc. are zsh/bash/ksh extensions that real
    // dash rejects ("expecting EOF: 16#ff"). --dash must error, not compute.
    for script in [
        "echo $((16#ff))",
        "echo $((8#17))",
        "echo $((2#1010))",
        "echo $((0b101))", // 0b binary is also non-POSIX (bash 4+/zsh only)
    ] {
        let (out, code) = run_dash_mode(script);
        assert_eq!(out, "", "`{script}` should produce no output under --dash");
        assert_ne!(
            code, 0,
            "`{script}` non-POSIX arith base must error under --dash"
        );
    }
    // But plain POSIX bases still work.
    assert_eq!(run_dash_mode("echo $((0x1f))").0, "31\n");
    assert_eq!(run_dash_mode("echo $((010))").0, "8\n");
    assert_eq!(run_dash_mode("echo $((2 + 3 * 4))").0, "14\n");
}

#[test]
fn dash_old_dollar_bracket_arith_is_literal() {
    // `$[expr]` is a deprecated bash/zsh arithmetic form; POSIX uses `$(( ))`.
    // dash/ash have no `$[ ]` — the `$` is a literal dollar and `[expr]` plain
    // text, so `echo $[1+2]` prints `$[1+2]`, not `3`.
    assert_eq!(run_dash_mode("echo $[1+2]").0, "$[1+2]\n");
    assert_eq!(run_dash_mode("x=5; echo $[x*2]").0, "$[x*2]\n");
    // The POSIX `$(( ))` form still evaluates.
    assert_eq!(run_dash_mode("echo $((1+2))").0, "3\n");
}

#[test]
fn dash_double_bracket_is_not_reserved() {
    // dash has no `[[ ]]`; it is an ordinary command → "not found".
    let (out, code) = run_dash_mode("[[ 1 = 1 ]] && echo yes");
    assert_eq!(out, "");
    assert_ne!(code, 0, "`[[` must not be a reserved word under --dash");
}

#[test]
fn dash_rejects_array_literal() {
    let (out, code) = run_dash_mode("a=(1 2 3); echo done");
    assert_eq!(out, "");
    assert_ne!(code, 0, "`name=(...)` must be a syntax error under --dash");
}

#[test]
fn dash_rejects_here_string() {
    let (out, code) = run_dash_mode("cat <<< hi");
    assert_eq!(out, "");
    assert_ne!(code, 0, "`<<<` must be a syntax error under --dash");
}

#[test]
fn dash_no_ansi_c_quoting() {
    // `$'\t'` is a literal `$` followed by an ordinary single-quoted
    // `\t` (two chars), NOT a tab — exactly like /bin/dash.
    let (out, code) = run_dash_mode("printf '%s' $'\\t'");
    assert_eq!(out, "$\\t");
    assert_eq!(code, 0);
}

#[test]
fn dash_no_plus_equals() {
    // `x+=b` is a command word, not an append-assign; `x` keeps its
    // value and the stray command fails, but `echo $x` still runs.
    let (out, _) = run_dash_mode("x=a; x+=b 2>/dev/null; echo $x");
    assert_eq!(out.trim(), "a");
}

#[test]
fn dash_echo_is_xsi() {
    // dash's echo interprets backslash escapes by default (XSI), the
    // opposite of the BSDECHO that EMULATE_SH sets.
    let (out, _) = run_dash_mode("echo 'a\\tb'");
    assert_eq!(out, "a\tb\n");
}

#[test]
fn dash_printf_no_percent_q() {
    let (out, code) = run_dash_mode("printf '%q' 'a b'");
    assert_eq!(out, "");
    assert_ne!(
        code, 0,
        "printf %q must be an invalid directive under --dash"
    );
}

// ── posix-faithful field splitting: trailing empty field dropped ───────

#[test]
fn dash_field_split_drops_trailing_empty() {
    // dash/ksh/bash drop a trailing empty field on a non-whitespace IFS
    // separator; zsh keeps it. --dash must match the real shell.
    let cases: &[(&str, &str)] = &[
        ("a:b:", "2"),   // trailing separator → no trailing empty
        (":a:b", "3"),   // leading separator → empty first field kept
        ("a::b", "3"),   // middle empty kept
        (":", "1"),      // lone separator → one empty field
        (":a::b:", "4"), // combined
        ("a:b:c", "3"),  // no trailing separator
    ];
    for (val, want) in cases {
        let (out, _) = run_dash_mode(&format!("IFS=:; v={val}; set -- $v; printf '%s' \"$#\""));
        assert_eq!(out, *want, "IFS split of {val:?} under --dash");
    }
}

#[test]
fn sh_zsh_combo_keeps_zsh_split() {
    // `--sh --zsh` selects zsh-style emulation: the trailing empty field is
    // KEPT (zsh behavior), proving the --zsh opt-out works.
    let out = Command::new(zshrs_bin())
        .args([
            "--sh",
            "--zsh",
            "-f",
            "-c",
            "IFS=:; v=a:b:; set -- $v; printf '%s' \"$#\"",
        ])
        .output()
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "3");
}

// ── read: backslash-escaped IFS char is literal (fixed universal bug) ───

#[test]
fn read_backslash_escapes_ifs() {
    // `read x y` on `a\ b`: the escaped space is literal, so it is ONE
    // field — x="a b", y="". dash/ksh/bash/zsh all agree; this was wrong
    // in zshrs across every mode before the fix.
    let out = Command::new(zshrs_bin())
        .args([
            "--dash",
            "-f",
            "-c",
            "read x y; printf '[%s][%s]' \"$x\" \"$y\"",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut ch| {
            use std::io::Write;
            ch.stdin.take().unwrap().write_all(b"a\\ b\n")?;
            ch.wait_with_output()
        })
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[a b][]");
}

#[test]
fn read_backslash_escapes_custom_ifs() {
    // Escaped separator (`a\:b`) is literal; the unescaped `:` splits.
    let out = Command::new(zshrs_bin())
        .args([
            "--dash",
            "-f",
            "-c",
            "IFS=:; read x y; printf '[%s][%s]' \"$x\" \"$y\"",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut ch| {
            use std::io::Write;
            ch.stdin.take().unwrap().write_all(b"a\\:b:c\n")?;
            ch.wait_with_output()
        })
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[a:b][c]");
}

// ── printf %d: POSIX numeric parsing (not zsh math-eval) ────────────────

#[test]
fn dash_printf_d_numeric_contract() {
    // dash/POSIX-sh printf %d does strtoimax numeric parsing: leading value
    // used, exit non-zero on any junk; empty operand is a clean 0. --dash
    // must match /bin/dash exactly (output AND exit), unlike zsh's math-eval.
    let cases: &[(&str, &str, bool)] = &[
        // (arg, stdout, exit_ok)
        ("", "0", true),
        ("0x10", "16", true),
        ("010", "8", true),
        ("-5", "-5", true),
        ("A", "0", false),
        ("1+1", "1", false),
        ("12x", "12", false),
        ("0", "0", true),
    ];
    for (arg, want_out, want_ok) in cases {
        let out = Command::new(zshrs_bin())
            .args(["--dash", "-f", "-c", "printf '%d' \"$1\"", "_", arg])
            .output()
            .expect("spawn");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            *want_out,
            "printf %d {arg:?} output"
        );
        assert_eq!(out.status.success(), *want_ok, "printf %d {arg:?} exit");
    }
}

#[test]
fn printf_d_math_eval_kept_in_zsh_and_ksh() {
    // Under --zsh and --ksh, printf %d keeps zsh's math evaluation: `A` is a
    // math var (→ 0, exit 0) and `1+1` → 2. Proves the posix-numeric path is
    // gated to sh/dash only.
    for mode in ["--zsh", "--ksh"] {
        let out = Command::new(zshrs_bin())
            .args([mode, "-f", "-c", "printf '%d' A"])
            .output()
            .expect("spawn");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "0");
        assert!(
            out.status.success(),
            "{mode}: printf %d A should exit 0 (math var)"
        );
    }
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "printf '%d' 1+1"])
        .output()
        .expect("spawn");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "2");
}

// ── controls: dash-legal POSIX must still work under --dash ─────────────

#[test]
fn dash_posix_still_works() {
    let cases: &[(&str, &str)] = &[
        ("x=hi; echo $x", "hi\n"),
        ("if [ 1 = 1 ]; then echo y; fi", "y\n"),
        ("v=\"a b c\"; set -- $v; echo $#", "3\n"),
        ("echo $((3*4))", "12\n"),
        ("case x in x) echo m;; esac", "m\n"),
        ("echo ${z:-def}", "def\n"),
        ("echo `echo bt`", "bt\n"),
        ("for i in a b c; do printf %s \"$i\"; done; echo", "abc\n"),
    ];
    for (script, want) in cases {
        let (out, code) = run_dash_mode(script);
        assert_eq!(out, *want, "script `{script}` diverged");
        assert_eq!(code, 0, "script `{script}` should succeed");
    }
}

// ── zsh mode must NOT regress: the extensions still work there ──────────

#[test]
fn zsh_mode_keeps_extensions() {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "printf '%s' $'\\t'"])
        .output()
        .expect("spawn");
    // Under zsh, `$'\t'` IS ANSI-C decoded to a real tab.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "\t");
}

// ── curated-corpus byte-parity against real /bin/dash (when present) ────

/// Scripts that MUST produce identical stdout + exit-sign in `/bin/dash`
/// and `zshrs --dash`. Kept to portable POSIX plus the audited-rejection
/// set so the comparison is meaningful (random fuzzing would just surface
/// intentional zsh≠dash language differences as noise).
const DASH_CORPUS: &[&str] = &[
    // portable POSIX — must run identically
    "x=hi; echo $x",
    "v=\"a b c\"; set -- $v; echo $#",
    "if [ -n x ]; then echo y; fi",
    "echo $((7%3)) $((2*3)) $((10/2))",
    "for i in 1 2 3; do printf %s $i; done; echo",
    "i=0; while [ $i -lt 3 ]; do i=$((i+1)); done; echo $i",
    "case abc in a*) echo hit;; esac",
    "echo ${undef:-fallback}",
    "f() { echo \"in $1\"; }; f arg",
    "echo a b c | wc -w",
    "printf '%s-%s\\n' one two",
    // audited rejections — dash errors, zshrs --dash must error too
    "echo $((2**10))",
    "a=(1 2 3)",
    "cat <<< x",
    "[[ 1 = 1 ]]",
    "printf '%q' x",
];

#[test]
fn dash_corpus_byte_parity_when_dash_present() {
    let dash = ["/bin/dash", "/usr/bin/dash"]
        .into_iter()
        .find(|p| Path::new(p).exists());
    let Some(dash) = dash else {
        eprintln!("skipping: no /bin/dash on this host");
        return;
    };

    let mut mismatches = Vec::new();
    for script in DASH_CORPUS {
        let d = Command::new(dash)
            .args(["-c", script])
            .output()
            .expect("dash spawn");
        let z = Command::new(zshrs_bin())
            .args(["--dash", "-f", "-c", script])
            .output()
            .expect("zshrs spawn");

        let d_out = String::from_utf8_lossy(&d.stdout);
        let z_out = String::from_utf8_lossy(&z.stdout);
        let d_ok = d.status.success();
        let z_ok = z.status.success();

        // Compare stdout exactly and exit-code SIGN (0 vs non-0). stderr
        // text legitimately differs across shells and is not compared.
        if d_out != z_out || d_ok != z_ok {
            mismatches.push(format!(
                "  script: {script:?}\n    dash: ok={d_ok} out={d_out:?}\n    zrs : ok={z_ok} out={z_out:?}"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "zshrs --dash diverged from /bin/dash on {} script(s):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
}

#[test]
fn dash_mode_help_lists_flag() {
    let out = Command::new(zshrs_bin())
        .arg("--help")
        .output()
        .expect("zshrs --help failed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--dash"),
        "--help missing --dash:\n{stdout}"
    );
}
