//! Four-way emulation parity harness: `zshrs --{zsh,ksh,sh,dash}` vs the
//! matching reference shell (`zsh`, `ksh`, `/bin/sh`, `/bin/dash`).
//!
//! This is the curated-corpus differential the way `parity-fuzz.rs` is for
//! `--zsh` at scale: a hand-picked set of *portable* scripts that MUST be
//! byte-identical (stdout + exit-code sign) between zshrs-in-mode-X and the
//! real shell X. The corpus deliberately avoids constructs whose behavior
//! legitimately differs across these shells — unquoted word-splitting,
//! `echo` escape handling, arrays, `[[ ]]` — because a differential on
//! those would flag intentional language differences as noise. Mode-specific
//! rejections (e.g. dash's) are pinned in `tests/dash_mode.rs`.
//!
//! Missing reference shells are reported (never silently passed). Set
//! `ZSHRS_REQUIRE_REF_SHELLS=1` (CI does) to turn a missing shell into a
//! failure instead of a skip, so the parity contract is enforced rather
//! than aspirational.

use std::path::Path;
use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// A reference shell and the zshrs flag that emulates it.
struct RefShell {
    /// Human label / the emulation name.
    name: &'static str,
    /// zshrs emulation flag (e.g. `--dash`).
    zshrs_flag: &'static str,
    /// Candidate absolute paths / PATH names for the real shell, tried in
    /// order; first that exists wins.
    candidates: &'static [&'static str],
}

const REF_SHELLS: &[RefShell] = &[
    RefShell { name: "zsh",  zshrs_flag: "--zsh",  candidates: &["zsh", "/bin/zsh", "/usr/bin/zsh", "/opt/homebrew/bin/zsh"] },
    RefShell { name: "ksh",  zshrs_flag: "--ksh",  candidates: &["ksh", "/bin/ksh", "/usr/bin/ksh", "mksh"] },
    RefShell { name: "sh",   zshrs_flag: "--sh",   candidates: &["/bin/sh"] },
    RefShell { name: "dash", zshrs_flag: "--dash", candidates: &["/bin/dash", "/usr/bin/dash"] },
];

/// Portable scripts that every one of {zsh, ksh, sh, dash} executes
/// identically. Only `printf` is used for output (no `echo` escape
/// divergence); all expansions are quoted (no word-split divergence); no
/// arrays / `[[`. Each entry is `(script, why)` — the `why` documents the
/// POSIX feature exercised so a future edit knows what it protects.
const PORTABLE_CORPUS: &[&str] = &[
    "printf '%s\\n' hello",                                        // literal
    "x=5; printf '%s\\n' \"$x\"",                                  // scalar assign + expand
    "for i in 1 2 3; do printf '%s' \"$i\"; done; printf '\\n'",   // for loop
    "i=0; while [ \"$i\" -lt 3 ]; do i=$((i+1)); done; printf '%s\\n' \"$i\"", // while + arith
    "case foo in f*) printf match;; esac; printf '\\n'",           // case glob
    "printf '%s\\n' \"${undef:-def}\"",                            // default-value expansion
    "f() { printf '%s\\n' \"$1\"; }; f hi",                        // function + positional
    "printf '%d\\n' \"$((6/2+1))\"",                               // arithmetic
    "printf '%s\\n' \"$(printf sub)\"",                            // command substitution
    "set -- a b c; printf '%s\\n' \"$#\"",                         // positional count
    "v=abc; printf '%s\\n' \"${v#a}\"",                            // prefix strip
    "v=abc; printf '%s\\n' \"${v%c}\"",                            // suffix strip
    "v=aXbXc; printf '%s\\n' \"${v%X*}\"",                         // greedy-vs-lazy suffix
    "true && printf yes; printf '\\n'",                            // && short-circuit
    "false || printf recover; printf '\\n'",                       // || short-circuit
    "printf '%s\\n' \"${#abc}\" 2>/dev/null || printf '%s\\n' 0",  // length (abc undefined → 0)
    "x=1; y=2; printf '%s\\n' \"$((x<y))\"",                       // arith comparison
    "if [ a = a ]; then printf eq; fi; printf '\\n'",              // test builtin
    "n=5; printf '%s\\n' \"$((n*n))\"",                            // arith mult
    "printf '%s ' one two three; printf '\\n'",                    // printf reuse
    // Field splitting on a non-whitespace IFS — the trailing-empty-field
    // rule where zsh diverges from the POSIX shells. In a bare drop-in
    // mode these MUST match the real shell (posix-faithful): a trailing
    // separator drops the empty field, a leading/middle one keeps it.
    "IFS=:; v=a:b:; set -- $v; printf '%s\\n' \"$#\"",             // trailing → drop empty
    "IFS=:; v=:a:b; set -- $v; printf '%s\\n' \"$#\"",             // leading → keep empty
    "IFS=:; v=a::b; set -- $v; printf '%s\\n' \"$#\"",             // middle → keep empty
    "IFS=:; v=:; set -- $v; printf '%s\\n' \"$#\"",                // lone separator
    "IFS=:; v=:a::b:; set -- $v; printf '%s\\n' \"$#\"",           // combined
    "IFS=:; v=a:b:c; set -- $v; printf '%s\\n' \"$#\"",            // no trailing
];

fn find_shell(sh: &RefShell) -> Option<String> {
    for c in sh.candidates {
        if c.starts_with('/') {
            if Path::new(c).exists() {
                return Some((*c).to_string());
            }
        } else if let Ok(out) = Command::new("sh").args(["-c", &format!("command -v {c}")]).output() {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// (stdout, success). stderr is intentionally dropped — its text
/// legitimately differs across shells; only stdout + exit-sign are compared.
fn run(bin: &str, args: &[&str], script: &str) -> (String, bool) {
    let mut full: Vec<&str> = args.to_vec();
    full.push("-c");
    full.push(script);
    let out = Command::new(bin).args(&full).output().unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.success())
}

/// The enforcement decision, extracted so it is unit-testable without
/// depending on which reference shells happen to be installed: when
/// `ZSHRS_REQUIRE_REF_SHELLS` is set, any absent reference shell is fatal.
fn missing_is_fatal(require: bool, missing: &[&str]) -> bool {
    require && !missing.is_empty()
}

#[test]
fn enforcement_gate_logic() {
    // Not requiring → a miss is a skip, never fatal.
    assert!(!missing_is_fatal(false, &["ksh"]));
    assert!(!missing_is_fatal(false, &[]));
    // Requiring + all present → not fatal.
    assert!(!missing_is_fatal(true, &[]));
    // Requiring + a miss → fatal. This is the CI contract: a missing
    // reference shell fails the build instead of silently passing.
    assert!(missing_is_fatal(true, &["ksh"]));
    assert!(missing_is_fatal(true, &["ksh", "dash"]));
}

#[test]
fn emulation_parity_four_way() {
    let require = std::env::var("ZSHRS_REQUIRE_REF_SHELLS").is_ok();
    let mut tested = 0usize;
    let mut missing: Vec<&str> = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();

    for sh in REF_SHELLS {
        let Some(refbin) = find_shell(sh) else {
            missing.push(sh.name);
            eprintln!("skip: reference shell `{}` not found", sh.name);
            continue;
        };
        tested += 1;
        eprintln!("testing {} : zshrs {} vs {}", sh.name, sh.zshrs_flag, refbin);

        for script in PORTABLE_CORPUS {
            // zsh reference is run with -f (no rc); the POSIX shells ignore -f
            // but take no rc for -c anyway, so run them plain.
            let ref_args: &[&str] = if sh.name == "zsh" { &["-f"] } else { &[] };
            let (r_out, r_ok) = run(&refbin, ref_args, script);
            let (z_out, z_ok) = run(&zshrs_bin(), &[sh.zshrs_flag, "-f"], script);

            if r_out != z_out || r_ok != z_ok {
                mismatches.push(format!(
                    "  [{}] {script:?}\n    {}: ok={r_ok} out={r_out:?}\n    zrs: ok={z_ok} out={z_out:?}",
                    sh.name, sh.name
                ));
            }
        }
    }

    eprintln!("emulation parity: tested {tested} reference shell(s), {} missing", missing.len());

    assert!(
        mismatches.is_empty(),
        "emulation parity diverged on {} case(s):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );

    if missing_is_fatal(require, &missing) {
        panic!(
            "ZSHRS_REQUIRE_REF_SHELLS is set but these reference shells were absent: {missing:?}. \
             Install them so the parity contract is enforced, not skipped."
        );
    }
    assert!(tested > 0, "no reference shells available at all — cannot verify parity");
}
