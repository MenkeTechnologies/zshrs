//! Builtin-output parity: the listings whose FORMAT differs per shell.
//!
//! `Src/builtin.c` prints one format, zsh's. A drop-in has to print what
//! ITS shell prints — these strings are parsed by scripts (`export -p` is
//! specified to be re-inputtable, `set -o` is grepped in the wild) and
//! read by people (`times`, `kill -l`, `hash`).
//!
//! Every expectation here is a live diff against the reference binary in
//! a cleared environment, so the suite tracks the installed shells rather
//! than a snapshot someone has to maintain by hand.
//!
//! Two families are deliberately not compared byte-for-byte:
//!
//!   * anything carrying a CLOCK (`times`) — the two shells are separate
//!     processes, so digits are masked and the FORMAT is what is asserted
//!     (field count, separators, zero-padding). That still catches a
//!     precision or padding change, which is the whole point.
//!   * `--sh` against a bash-backed `/bin/sh`. macOS ships bash 3.2 as
//!     `/bin/sh`, and bash-as-sh keeps bash's own listings; `--sh` targets
//!     POSIX sh (dash on Debian), the same split this repo already
//!     documents for startup files. Detected, reported and skipped.

use std::path::Path;
use std::process::{Command, Stdio};

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// A drop-in and the reference binary that defines its output.
struct Leg {
    name: &'static str,
    flag: &'static str,
    candidates: &'static [&'static str],
    /// Absence is a skip, never fatal, even under ZSHRS_REQUIRE_REF_SHELLS.
    optional: bool,
}

const LEGS: &[Leg] = &[
    Leg {
        name: "zsh",
        flag: "--zsh",
        candidates: &["zsh", "/bin/zsh", "/opt/homebrew/bin/zsh"],
        optional: false,
    },
    Leg {
        name: "bash",
        flag: "--bash",
        candidates: &["bash", "/bin/bash", "/opt/homebrew/bin/bash"],
        optional: false,
    },
    Leg {
        name: "ksh",
        flag: "--ksh",
        candidates: &["ksh", "/bin/ksh", "/usr/bin/ksh"],
        optional: false,
    },
    Leg {
        name: "mksh",
        flag: "--mksh",
        candidates: &["mksh", "/bin/mksh", "/opt/homebrew/bin/mksh"],
        optional: true,
    },
    Leg {
        name: "dash",
        flag: "--dash",
        candidates: &["/bin/dash", "/usr/bin/dash", "/opt/homebrew/bin/dash"],
        optional: false,
    },
    Leg {
        name: "sh",
        flag: "--sh",
        candidates: &["/bin/sh"],
        optional: false,
    },
];

fn find_shell(candidates: &[&str]) -> Option<String> {
    for c in candidates {
        if c.starts_with('/') {
            if Path::new(c).exists() {
                return Some((*c).to_string());
            }
        } else if let Ok(out) = Command::new("sh")
            .args(["-c", &format!("command -v {c}")])
            .output()
        {
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

/// Run one script with a cleared environment. `-f` is NOT used: in the
/// Bourne-family drop-ins it means `noglob`, which shows up in `set -o`.
fn run(bin: &str, pre: &[&str], script: &str) -> String {
    let mut cmd = Command::new(bin);
    cmd.args(pre)
        .args(["-c", script])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/tmp")
        .env("TERM", "dumb")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    let out = cmd.output().unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// No rc-suppressing flag on either side: `$HOME` is a directory with no
/// startup files, and passing `--no-rcs` to zshrs alone would show up as
/// `norcs on` in its own `set -o` listing while the reference reported
/// `off`. Symmetry beats suppression here.
fn zshrs(leg: &Leg, script: &str) -> String {
    run(&zshrs_bin(), &[leg.flag], script)
}

/// True when this reference is bash wearing another shell's name — macOS
/// ships bash 3.2 as `/bin/sh`, and it keeps bash's own listings.
fn reference_is_bash(refbin: &str) -> bool {
    !run(refbin, &[], r#"printf %s "${BASH_VERSION:-}""#).is_empty()
}

fn mask_digits(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_digit() { 'D' } else { c })
        .collect()
}

/// Drive one script across every leg and report the divergences.
fn compare(what: &str, script: &str, mask: bool, sort_lines: bool) {
    let mut mismatches = Vec::new();
    let mut tested = 0usize;
    let mut missing = Vec::new();

    for leg in LEGS {
        let Some(refbin) = find_shell(leg.candidates) else {
            if !leg.optional {
                missing.push(leg.name);
            }
            continue;
        };
        // `--sh` targets POSIX sh, which on Debian is dash. macOS ships
        // bash 3.2 as `/bin/sh`, and bash-as-sh keeps bash's own
        // listings — a different implementation, not a zshrs defect.
        if leg.name == "sh" && reference_is_bash(&refbin) {
            eprintln!("  note: {refbin} is bash wearing sh's name; skipping `{what}` for --sh");
            continue;
        }
        tested += 1;
        let (z, r) = (zshrs(leg, script), run(&refbin, &[], script));
        let (mut z, mut r) = if mask {
            (mask_digits(&z), mask_digits(&r))
        } else {
            (z, r)
        };
        // mksh's own default aliases store TWO backslashes
        // (`\\builtin typeset -fu`); zshrs stores one, because it
        // resolves a one-backslash command word and reports "command not
        // found" for two — the commands have to actually RUN. Collapse
        // the runs for this leg so the names and the rest of each body
        // are still compared exactly. See emulation_startup::MKSH_ALIASES.
        if matches!(leg.name, "mksh") {
            z = z.replace("\\\\", "\\");
            r = r.replace("\\\\", "\\");
        }
        // dash lists aliases in INSERTION order; zsh's table is sorted and
        // zshrs inherits that. The set and the rendering of each entry are
        // what this asserts, not the ordering.
        if sort_lines {
            let mut zl: Vec<&str> = z.lines().collect();
            let mut rl: Vec<&str> = r.lines().collect();
            zl.sort_unstable();
            rl.sort_unstable();
            z = zl.join("\n");
            r = rl.join("\n");
        }
        if z != r {
            mismatches.push(format!(
                "  [{}] {what}\n    ref({refbin}): {r:?}\n    zshrs:  {z:?}",
                leg.name
            ));
        }
    }

    assert!(tested > 0, "{what}: no reference shells available");
    assert!(
        mismatches.is_empty(),
        "{what} diverged on {} leg(s):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    if std::env::var("ZSHRS_REQUIRE_REF_SHELLS").is_ok() && !missing.is_empty() {
        panic!("ZSHRS_REQUIRE_REF_SHELLS set but these references were absent: {missing:?}");
    }
}

/// `kill -l`: bash numbers five to a row with a `SIG` prefix, ksh prints
/// bare names one per line (and calls signal 6 `IOT`), dash prepends
/// signal 0, mksh prints two columns with the strsignal text, zsh prints
/// one space-separated line. The signal SET comes from the running
/// system, so this also guards the macOS/Linux name differences.
#[test]
fn kill_l_matches_each_shell() {
    compare("kill -l", "kill -l", false, false);
}

/// `hash`: bash heads a `hits<TAB>command` table, dash prints the bare
/// resolved path, the Korn shells and zsh print `name=path`. mksh reaches
/// it through its own `hash` alias (`\builtin alias -t`), so this also
/// covers tracked-alias support.
#[test]
fn hash_listing_matches_each_shell() {
    compare("hash", "hash ls >/dev/null 2>&1; hash", false, false);
}

/// `set -o`: bash lists 27 names as `name<TAB>state`; dash, ksh93u+m and
/// mksh each head a "Current option settings" block with their own fixed
/// set — 17, 38 and 35 names — and mksh lays its out in four
/// column-major columns. ksh uses the POSITIVE sense (`clobber on` where
/// zsh has `noclobber` off).
#[test]
fn set_o_listing_matches_each_shell() {
    compare("set -o", "set -o", false, false);
}

/// `times`: the fraction width and seconds padding differ per shell —
/// bash milliseconds, dash microseconds, mksh centiseconds with a
/// zero-padded seconds field, ksh93u+m milliseconds zero-padded. Digits
/// are masked: two processes cannot share a clock reading.
#[test]
fn times_format_matches_each_shell() {
    compare("times", "times", true, false);
}

/// `alias` with no operands: bash emits the reusable `alias NAME='value'`
/// form, every other shell here the bare `NAME=value`.
#[test]
fn alias_listing_matches_each_shell() {
    compare(
        "alias listing",
        "alias zz='ls -l'; alias yy=1; alias",
        false,
        true,
    );
}

/// `export -p` / `readonly -p`: bash reports both through `declare`
/// (`declare -x NAME="v"`), the POSIX shells through the `export` /
/// `readonly` keyword.
#[test]
fn export_and_readonly_listings_match_each_shell() {
    compare(
        "export -p",
        "export FOO=bar; export -p | grep FOO",
        false,
        false,
    );
    compare(
        "readonly -p",
        "readonly RR=1; readonly -p | grep RR",
        false,
        false,
    );
}

/// `times_field` renders one clock value per shell. 100 ticks at 100Hz is
/// exactly one second, which pins the seconds field, its padding and the
/// fraction width in one shot.
///
/// This lives here rather than beside the code because it moves the
/// process-global personality, and the library test binary runs its tests
/// in parallel threads that share it.
#[test]
fn times_field_renders_each_shells_precision() {
    use zsh::emulation_output::times_field;
    use zsh::emulation_startup::{personality, set_personality, Personality};
    let saved = personality();
    for (p, want) in [
        (Personality::Zsh, "0m1.00s"),
        (Personality::Bash, "0m1.000s"),
        (Personality::Dash, "0m1.000000s"),
        (Personality::Mksh, "0m01.00s"),
        (Personality::Ksh93, "0m01.000s"),
    ] {
        set_personality(p);
        assert_eq!(times_field(100, 100), want, "{p:?}");
    }
    // A sub-second value exercises the fraction, and a multi-minute one
    // the minutes field. zsh's own arithmetic mods the seconds by the
    // CLOCK TICK rather than 60 (c:Src/builtin.c:7315-7318), so 125s
    // prints as 2m25s there and 2m5s everywhere else — upstream
    // behaviour the `--zsh` drop-in has to keep.
    set_personality(Personality::Bash);
    assert_eq!(times_field(1, 100), "0m0.010s");
    assert_eq!(times_field(12500, 100), "2m5.000s");
    set_personality(Personality::Zsh);
    assert_eq!(times_field(12500, 100), "2m25.00s");
    set_personality(saved);
}
