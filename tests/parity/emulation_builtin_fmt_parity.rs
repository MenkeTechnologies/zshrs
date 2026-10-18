//! Builtin OUTPUT-FORMAT parity for zshrs's emulation modes against the REAL
//! reference shells — `umask`, `getopts`'s `$OPTIND`/`$OPTARG` bookkeeping,
//! the `trap` listing, and `type` / `command -V` on a shell function.
//!
//! !!! THE REFERENCE BINARY IS THE SPEC HERE — NOT zsh's C source !!!
//! Every expectation below is produced by running the actual `bash` / `ksh` /
//! `mksh` / `dash` / `ash` / `/bin/sh` on this machine, so nothing goes stale
//! when a shell changes its formatting. zsh's C source stays the spec for
//! `--zsh`, which every family here re-checks as a negative control:
//! `zsh -f -c SCRIPT` and `zshrs --zsh -f -c SCRIPT` must stay byte-identical.
//!
//! A missing reference shell SKIPS its rows loudly (an eprintln naming the
//! shell) and the test still fails if NO reference for the family was found,
//! so an absent binary can never quietly turn a divergence into a pass.

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

/// First existing path for a reference shell, or `None` when it is not
/// installed here.
fn find_shell(candidates: &[&str]) -> Option<String> {
    for c in candidates {
        if c.starts_with('/') {
            if Path::new(c).exists() {
                return Some((*c).to_string());
            }
            continue;
        }
        if let Ok(out) = Command::new("/usr/bin/env")
            .args(["sh", "-c", &format!("command -v {c}")])
            .output()
        {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() && Path::new(&p).exists() {
                return Some(p);
            }
        }
    }
    None
}

fn bash() -> Option<String> {
    find_shell(&["/opt/homebrew/bin/bash", "bash", "/usr/local/bin/bash"])
}
fn ksh() -> Option<String> {
    find_shell(&["ksh", "/opt/homebrew/bin/ksh", "/bin/ksh", "/usr/bin/ksh"])
}
fn mksh() -> Option<String> {
    find_shell(&[
        "mksh",
        "/opt/homebrew/bin/mksh",
        "/bin/mksh",
        "/usr/bin/mksh",
    ])
}
fn dash() -> Option<String> {
    find_shell(&[
        "dash",
        "/opt/homebrew/bin/dash",
        "/bin/dash",
        "/usr/bin/dash",
    ])
}
fn ash() -> Option<String> {
    find_shell(&["ash", "/opt/homebrew/bin/ash", "/bin/ash", "/usr/bin/ash"])
}
fn zsh() -> Option<String> {
    find_shell(&[crate::oracle::zsh_path()])
}

/// One `(zshrs flag, reference-shell resolver, label)` row of the matrix.
/// `/bin/sh` is deliberately absent: it is bash on macOS and dash on Debian,
/// so pinning it would pin the host rather than a shell.
fn targets() -> Vec<(&'static str, Option<String>, &'static str)> {
    vec![
        ("--bash", bash(), "bash"),
        ("--ksh", ksh(), "ksh"),
        ("--mksh", mksh(), "mksh"),
        ("--dash", dash(), "dash"),
        ("--ash", ash(), "ash"),
    ]
}

struct Out {
    stdout: String,
    exit: i32,
}

fn run(bin: &str, pre: &[&str], script: &str) -> Out {
    let o = Command::new(bin)
        .args(pre)
        .arg(script)
        .env_remove("ZSHRS_CACHE")
        .env_remove("ENV")
        .output()
        .unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    Out {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Run every `script` in both `zshrs <flag>` and the real shell for each
/// installed target, asserting byte-identical stdout and exit status.
/// Panics if not one reference shell of the matrix is installed.
/// Drop `trap -- '' SIG` lines for a signal the SCRIPT never names.
///
/// Cargo's harness ignores SIGINT/SIGQUIT and the shell under test inherits
/// that `SIG_IGN`. POSIX requires a shell to KEEP an inherited ignore — `trap
/// - INT QUIT` cannot clear it — and bash lists such an entry while zshrs
/// `--bash` does not. That divergence is real and pinned by
/// `inherited_sig_ign_is_listed_by_bash`; here it is ambient noise that
/// depends on how the test binary was launched, so it is removed from BOTH
/// sides before comparing. Only the empty-body (`''`) form is dropped, and
/// only for a signal the script does not mention — a script that sets `trap ''
/// INT` still has its own entry compared.
fn strip_inherited_ignores(out: &str, script: &str) -> String {
    out.lines()
        .filter(|line| {
            let Some(rest) = line.strip_prefix("trap -- '' ") else {
                return true;
            };
            let sig = rest.trim();
            let bare = sig.strip_prefix("SIG").unwrap_or(sig);
            // Keep it when the script asked for this signal by name or number.
            !(!script.contains(bare) && !script.contains(sig))
        })
        .map(|l| format!("{l}\n"))
        .collect()
}

fn assert_matrix(scripts: &[&str]) {
    let zbin = zshrs_bin();
    let z = zbin.to_str().expect("zshrs path is UTF-8");
    let mut ran = 0usize;
    for (flag, refbin, label) in targets() {
        let Some(refbin) = refbin else {
            eprintln!("SKIP {label}: not installed on this host");
            continue;
        };
        ran += 1;
        for script in scripts {
            let r = run(&refbin, &["-c"], script);
            let g = run(z, &[flag, "-f", "-c"], script);
            let (r_out, g_out) = (
                strip_inherited_ignores(&r.stdout, script),
                strip_inherited_ignores(&g.stdout, script),
            );
            assert_eq!(
                r_out, g_out,
                "stdout divergence, {label} vs zshrs {flag}\n  script: {script}\n\
                 --- {label} ---\n{:?}\n--- zshrs ---\n{:?}",
                r_out, g_out
            );
            assert_eq!(
                r.exit, g.exit,
                "exit divergence, {label} vs zshrs {flag}\n  script: {script}"
            );
        }
    }
    assert!(
        ran > 0,
        "no reference shell of the emulation matrix is installed — \
         the comparison never ran, so this is a skip, not a pass"
    );
}

/// Negative control: the SAME scripts must keep producing zsh's own output in
/// `--zsh`. Every fix in this file is gated on `posix_faithful()`, which
/// `--zsh` never raises; this is what proves it.
fn assert_zsh_unchanged(scripts: &[&str]) {
    let Some(zshbin) = zsh() else {
        panic!("zsh is not installed — the negative control cannot run");
    };
    let zbin = zshrs_bin();
    let z = zbin.to_str().expect("zshrs path is UTF-8");
    for script in scripts {
        let r = run(&zshbin, &["-f", "-c"], script);
        let g = run(z, &["--zsh", "-f", "-c"], script);
        assert_eq!(
            r.stdout, g.stdout,
            "--zsh REGRESSED against real zsh\n  script: {script}\n\
             --- zsh ---\n{:?}\n--- zshrs --zsh ---\n{:?}",
            r.stdout, g.stdout
        );
    }
}

// ===========================================================================
// umask
// ===========================================================================

/// zsh emits the leading `0` only when the owner field is set
/// (`Src/builtin.c:7522-7524`); bash, ksh93, dash, ash and `/bin/sh` always
/// print four octal digits, and mksh keeps zsh's form. Covers the whole
/// range so the fix cannot be a blind `0` prefix.
const UMASK_SCRIPTS: &[&str] = &[
    "umask 022; umask",
    "umask 077; umask",
    "umask 000; umask",
    "umask 007; umask",
    "umask 0700; umask",
    "umask 0777; umask",
    "umask 0644; umask",
    // `-S` is byte-identical in every shell including zsh — pinned so the
    // four-digit change cannot leak into the symbolic form.
    "umask 022; umask -S",
];

#[test]
fn umask_display_matches_reference_shells() {
    assert_matrix(UMASK_SCRIPTS);
}

#[test]
fn umask_display_unchanged_under_zsh_mode() {
    assert_zsh_unchanged(UMASK_SCRIPTS);
}

// ===========================================================================
// getopts — $OPTIND / $OPTARG / the end-of-options name
// ===========================================================================

/// `$o`, `$OPTARG` and `$OPTIND` after a fixed number of `getopts` calls.
/// The three shapes named by the fuzz signatures (`-b`, `--`, `-b -a W`) plus
/// the neighbours that pin the state machine: clusters, an attached option
/// argument, a bad option, a missing argument, a non-option word, the
/// positional-parameter form, and an explicit `OPTIND` reset.
const GETOPTS_SCRIPTS: &[&str] = &[
    r#"OPTIND=1; getopts 'ab' o -b >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'ab' o -- >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'a:b' o -b -a W >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'a:b' o -b -a W >/dev/null 2>&1; getopts 'a:b' o -b -a W >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; for i in 1 2 3; do getopts 'a:b' o -b -a W >/dev/null 2>&1; done; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'ab' o -ab >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'ab' o -ab >/dev/null 2>&1; getopts 'ab' o -ab >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; for i in 1 2 3; do getopts 'ab' o -ab >/dev/null 2>&1; done; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'a:b' o -aW >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'ab' o -z >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'a:b' o -a >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'ab' o x >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'ab' o '' >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'ab' o -b x >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts 'ab' o -b x >/dev/null 2>&1; getopts 'ab' o -b x >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    // Explicit reset between two parses of the same word list.
    r#"OPTIND=1; getopts 'ab' o -b >/dev/null 2>&1; OPTIND=1; getopts 'ab' o -b >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    // The whole loop, reporting $OPTIND at every iteration.
    r#"OPTIND=1; while getopts 'a:bc' o -b -a W -c x 2>/dev/null; do printf 'o=%s arg=[%s] ind=%s\n' "$o" "$OPTARG" "$OPTIND"; done; printf 'end %s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    // The positional-parameter form, ending in the `shift $((OPTIND-1))` idiom
    // that makes an off-by-one in $OPTIND change which words survive.
    r#"set -- -b -a W rest1 rest2; OPTIND=1; while getopts 'a:b' o 2>/dev/null; do printf 'o=%s arg=[%s] ind=%s\n' "$o" "$OPTARG" "$OPTIND"; done; shift $((OPTIND-1)); printf 'rest=[%s]\n' "$*""#,
    // `set --` between two parses: dash and ash rewind the cursor with the
    // positional parameters, bash / ksh93 / mksh do not.
    r#"OPTIND=1; getopts 'ab' o -a >/dev/null 2>&1; set -- -b -a; getopts 'ab' o >/dev/null 2>&1; printf '%s %s\n' "$o" "$OPTIND""#,
    r#"set -- -a; OPTIND=1; getopts 'a' o >/dev/null 2>&1; printf '%s,' "$o"; set -- -a; OPTIND=1; getopts 'a' o >/dev/null 2>&1; printf '%s\n' "$o""#,
    // Silent (`:`-leading) optstring: the `:` name and $OPTARG on a missing
    // argument, and the `?` name on an unknown option.
    r#"OPTIND=1; getopts ':a:b' o -a >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    r#"OPTIND=1; getopts ':ab' o -z >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
];

#[test]
fn getopts_state_matches_reference_shells() {
    assert_matrix(GETOPTS_SCRIPTS);
}

#[test]
fn getopts_state_unchanged_under_zsh_mode() {
    assert_zsh_unchanged(GETOPTS_SCRIPTS);
}

/// bash, ksh93 and /bin/sh hook the ASSIGNMENT to `$OPTIND` rather than comparing
/// values, so they rewind the within-argument cursor even when the assigned value
/// equals the one last reported: mid-cluster they re-parse `a` where dash and
/// mksh continue to `b` (`dash_mode::note_optind_assignment`).
#[test]
fn getopts_same_value_reset_midcluster_is_a_known_gap() {
    assert_matrix(&[
        r#"OPTIND=1; getopts 'ab' o -ab >/dev/null 2>&1; OPTIND=1; getopts 'ab' o -ab >/dev/null 2>&1; printf '%s/%s\n' "$o" "$OPTIND""#,
    ]);
}

/// dash and ash key `getopts` off `shellparam.optind`, which a write to `$OPTIND`
/// never reaches, so an `OPTIND=1` between two parses of different word lists
/// carries their index forward. Every reference shell agrees with zshrs here.
#[test]
fn dash_ignores_optind_writes_is_a_known_gap() {
    assert_matrix(&[
        r#"OPTIND=1; getopts 'ab' o -- >/dev/null 2>&1; OPTIND=1; getopts ':a:b' o -a >/dev/null 2>&1; printf '%s/%s/%s\n' "$o" "$OPTARG" "$OPTIND""#,
    ]);
}

// ===========================================================================
// trap — the no-argument listing
// ===========================================================================

/// Bodies that exercise the raw-vs-deparsed text, the quoting rule and the
/// signal-name spelling. Bodies containing an UNBALANCED apostrophe are left
/// out on purpose: zsh parses a trap body at install time and rejects those
/// (`Src/builtin.c:7405-7409`), so they never reach the listing at all — a
/// separate divergence from this family.
///
/// Uses ONLY signals that no plausible launcher ignores — TERM, USR1, USR2 and
/// EXIT — and is compared through `strip_inherited_ignores`.
///
/// An inherited `SIG_IGN` is the hazard. POSIX forbids resetting one, so a
/// `trap - SIG` prefix is a no-op, and bash then both LISTS the inherited
/// entry and REFUSES to trap that signal, while zshrs does neither
/// (docs/BUGS.md #1109). Any script here naming an ignored signal therefore
/// measures THAT gap instead of the formatting it exists to pin.
///
/// Two launchers do this, and between them they rule out three signals:
/// cargo's own test harness ignores INT and QUIT, and `nohup` ignores HUP —
/// which is how this test broke a second time, having first been "fixed" by
/// moving off INT onto HUP. The inherited case is pinned separately by
/// `inherited_sig_ign_is_listed_by_bash`, never swept up here.
const TRAP_SCRIPTS: &[&str] = &[
    "trap ':' TERM; trap",
    "trap ':' EXIT; trap",
    "trap ':' 15; trap",
    "trap ':' EXIT TERM USR1 USR2; trap",
    "trap '' USR2; trap",
    "trap '' USR2 TERM USR1; trap",
    "trap 'printf x' TERM USR1; trap",
    "trap 'printf a; printf b' USR2; trap",
    "trap 'if true; then printf x; fi' USR1; trap",
    r#"trap "printf 'a b'" USR2; trap"#,
    r#"trap "a'b'c" USR2; trap"#,
    r"trap 'printf %s\n' TERM; trap",
    r#"trap 'printf "a b"' USR2; trap"#,
    "trap ':' USR2; trap - USR2; trap",
];

#[test]
fn trap_listing_matches_reference_shells() {
    assert_matrix(TRAP_SCRIPTS);
}

/// An inherited `SIG_IGN` disposition is LISTED by `trap` in bash, and a
/// non-interactive bash refuses to trap or reset it. docs/BUGS.md #1109.
///
/// Not a test artefact: it is exactly what cargo's harness produces, since it
/// ignores SIGINT/SIGQUIT and the shell under test inherits that. Reproduced
/// by hand outside any harness:
///
/// ```text
/// $ (trap '' INT QUIT; bash -c "trap ':' HUP; trap")
/// trap -- ':' SIGHUP
/// trap -- '' SIGINT
/// trap -- '' SIGQUIT
/// $ (trap '' INT QUIT; zshrs --bash -c "trap ':' HUP; trap")   -> identical
/// ```
///
/// zsh mode is NOT affected and needs no fix — zsh lists the inherited ignore
/// for QUIT (and not INT), and zshrs `--zsh` reproduces that exactly:
///
/// ```text
/// $ (trap '' INT QUIT; zsh -fc "trap ':' HUP; trap")      -> : HUP  /  '' QUIT
/// $ (trap '' INT QUIT; zshrs --zsh -f -c "trap ':' HUP; trap") -> identical
/// ```
///
/// dash lists neither, and `--dash` matches. So the gap is the bash leg alone.
#[test]
fn inherited_sig_ign_is_listed_by_bash() {
    // Only meaningful when the PARENT ignores these — under cargo's harness it
    // does. Standalone (no inherited ignore) both shells agree and this passes
    // vacuously, which is why the format matrix uses signals cargo leaves
    // alone (HUP/TERM/USR1/USR2/EXIT) instead of carrying these.
    assert_matrix(&[
        "trap ':' HUP; trap",
        // bash REFUSES to trap a signal inherited as SIG_IGN and prints
        // nothing; zshrs traps it and lists `trap -- ':' SIGINT`.
        "trap ':' 2; trap",
        "trap ':' INT; trap",
        "trap '' INT; trap",
        "trap ':' EXIT INT HUP USR1 TERM QUIT; trap",
    ]);
}

#[test]
fn trap_listing_unchanged_under_zsh_mode() {
    assert_zsh_unchanged(TRAP_SCRIPTS);
}

// ===========================================================================
// type / command -V on a shell function
// ===========================================================================

/// `f is a shell function from zsh` is zsh's wording and zsh's alone
/// (`Src/hashtable.c:927-941`). bash and `/bin/sh` say `f is a function` and
/// print the definition; ksh93 and mksh say `f is a function` with no body;
/// dash and ash say `f is a shell function`.
const TYPE_SCRIPTS: &[&str] = &[
    "f() { :; }; type f",
    "f() { :; }; command -V f",
    "f() { :; }; command -v f",
    "f() { echo a; echo b; }; type f",
    // Two functions in one invocation: the header (and, in bash, the body)
    // has to repeat per name, so a fix that emitted the emulation line once
    // and then fell back would show up here.
    "f() { :; }; g() { :; }; type f g",
];

#[test]
fn function_type_matches_reference_shells() {
    assert_matrix(TYPE_SCRIPTS);
}

#[test]
fn function_type_unchanged_under_zsh_mode() {
    assert_zsh_unchanged(TYPE_SCRIPTS);
}

/// bash re-prints a function body from its own AST (`make_command_string`),
/// keeping `if true; then` on one line where zsh's deparse splits it.
/// `dash_mode::bash_function_body` re-lays the compound commands in bash's
/// layout; this pins that for `type`.
#[test]
fn bash_compound_function_body_layout_matches_bash() {
    let Some(bashbin) = bash() else {
        panic!("bash is not installed — the comparison never ran");
    };
    let script = "f() { if true; then echo y; fi; }; type f";
    let zbin = zshrs_bin();
    let r = run(&bashbin, &["-c"], script);
    let g = run(zbin.to_str().unwrap(), &["--bash", "-f", "-c"], script);
    assert_eq!(r.stdout, g.stdout);
}

// ===========================================================================
// --emulate MODE
// ===========================================================================

/// c:Src/init.c:461-474 — `zsh --emulate MODE` stays zsh and installs a full
/// `emulate(MODE, 1, …)` before the remaining option words are read. The
/// zsh-compat option must not select the drop-in sh/ksh personality: its
/// `readonly -p` listed the zsh specials and its error prefix read `zshrs`.
/// The early `--emulate` errors are reported with argv[0] as the prefix and
/// no line number (`argzero` is set at c:282, before parseargs).
#[test]
fn emulate_option_is_zsh_running_emulate() {
    // 5.9.2 lists `restricted` in `setopt`; the development tree dropped it (54181).
    let Some(zshbin) = crate::oracle::release_zsh_path().map(String::from) else {
        eprintln!("skip: no zsh reporting zshrs's version is installed");
        return;
    };
    let zbin = zshrs_bin();
    let z = zbin.to_str().expect("zshrs path is UTF-8");
    let both = |pre: &[&str], script: Option<&str>| {
        let go = |bin: &str| {
            let mut c = Command::new(bin);
            c.args(pre).env_remove("ZSHRS_CACHE").env_remove("ENV");
            if let Some(s) = script {
                c.arg(s);
            }
            let o = c.output().unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
            let err = String::from_utf8_lossy(&o.stderr).replace(bin, "ZSH");
            (String::from_utf8_lossy(&o.stdout).into_owned(), err, o.status.code())
        };
        assert_eq!(go(&zshbin), go(z), "args {pre:?} script {script:?}");
    };
    both(&["--emulate", "sh", "-c"], Some("readonly x=1; readonly -p"));
    both(&["--emulate", "sh", "-f", "-c"], Some("setopt; nosuchcmd_zz"));
    both(&["--emulate", "ksh", "-f", "-c"], Some("emulate; setopt | wc -l"));
    both(&["-f", "--emulate", "sh"], None);
    both(&["--emulate"], None);
}

/// A signal ignored on entry to a non-interactive shell cannot be trapped or
/// reset in bash, ksh93 and `/bin/sh`, and each lists it differently: bash
/// always (`trap -- '' SIGUSR1`), ksh93 once a `trap` command has named it,
/// `/bin/sh` never. dash and mksh trap it normally. The inherited `SIG_IGN` is
/// set up explicitly by a wrapper, not left to how the harness was launched.
/// docs/BUGS.md #1109.
#[test]
fn inherited_sig_ign_follows_each_reference_shell() {
    let mut legs = targets();
    legs.push(("--sh", Path::new("/bin/sh").exists().then(|| "/bin/sh".to_string()), "sh"));
    let zbin = zshrs_bin();
    let z = zbin.to_str().expect("zshrs path is UTF-8");
    let ignoring = |argv: &[&str], script: &str| -> Out {
        let mut full = vec!["-c", "trap '' USR1; exec \"$@\"", "wrap"];
        full.extend_from_slice(argv);
        full.push(script);
        run("/bin/sh", &full, "")
    };
    for (flag, refbin, label) in legs {
        let Some(refbin) = refbin else {
            continue;
        };
        for script in [
            "trap",
            "trap ':' USR1; trap",
            "trap '' USR1; trap",
            "trap ':' USR1; trap - USR1; trap",
            "trap ':' USR1 TERM; trap",
            "trap ':' USR1; kill -USR1 $$; echo alive",
        ] {
            let r = ignoring(&[&refbin, "-c"], script);
            let g = ignoring(&[z, flag, "-f", "-c"], script);
            assert_eq!(
                (r.stdout.as_str(), r.exit),
                (g.stdout.as_str(), g.exit),
                "{label} vs zshrs {flag}: {script}"
            );
        }
    }
}

// ===========================================================================
// unset-name array splats (Korn shells)
// ===========================================================================

/// ksh93 and mksh splat an unset name as an EMPTY array: `"${nope[@]}"` and
/// an unused `"${nope[@]+w}"` are zero words, and an unbraced `$a[@]` is
/// `$a` followed by the literal `[@]` (KSH_ARRAYS). dash and ash have no
/// arrays, so only the Korn shells of the matrix are compared; bash is pinned
/// in `bash_shopt_parity`.
#[test]
fn unset_name_splats_match_korn_shells() {
    let zbin = zshrs_bin();
    let z = zbin.to_str().expect("zshrs path is UTF-8");
    let scripts = [
        r#"printf '<%s>' "${nope[@]}" "${nope}" "${nope[@]-e}" "${nope[@]+f}" ${nope[@]} "${#nope[@]}" "pre${nope[@]}post" "${nope[*]}" x; echo"#,
        r#"set -A a 1 2; printf '<%s>' "$a[@]" $a[@] "$a[1]" $a[*] "$nope[@]" x; echo"#,
    ];
    let mut ran = 0usize;
    for (flag, refbin, label) in [("--ksh", ksh(), "ksh"), ("--mksh", mksh(), "mksh")] {
        let Some(refbin) = refbin else {
            eprintln!("SKIP {label}: not installed on this host");
            continue;
        };
        ran += 1;
        for script in scripts {
            let r = run(&refbin, &["-c"], script);
            let g = run(z, &[flag, "-f", "-c"], script);
            assert_eq!(
                (r.stdout.as_str(), r.exit),
                (g.stdout.as_str(), g.exit),
                "{label} vs zshrs {flag}\n  script: {script}"
            );
        }
    }
    assert!(ran > 0, "neither ksh nor mksh is installed — the comparison never ran");
}

/// ksh93 keeps an associative array's keys sorted by bytes, and `typeset -p`
/// quotes a key exactly as it quotes a value (sh_fmtq). ASCII only: the local
/// reference may be AT&T 93u+, whose sh_fmtq treats every non-ASCII
/// character as unprintable where the ksh93u+m zshrs follows does not.
#[test]
fn associative_arrays_follow_ksh93_order_and_quoting() {
    let Some(refbin) = ksh() else {
        eprintln!("SKIP ksh: not installed on this host");
        return;
    };
    let zbin = zshrs_bin();
    let z = zbin.to_str().expect("zshrs path is UTF-8");
    for script in [
        r#"typeset -A h=([b]=1 [B]=2 [10]=3 [9]=4 ["a b"]=6 [_x]=7 [a]=8 [-]=9); echo "${!h[@]}"; echo "${h[@]}"; for k in "${!h[@]}"; do printf '<%s>' "$k"; done; echo"#,
        r##"typeset -A h=(["a b"]=1 ["*"]=2 [x=y]=3 ["]"]=4 [""]=5 ["\$v"]=6 ["it's"]=7 [-]=9 ["a\\b"]=10); typeset -p h"##,
        r##"for s in "" a "a b" "x=a b" "x==y" "x=" "a.b" "#a" "a#" "~a" "a~" "it's" "a]" "a:b" "\$HOME"; do printf '%q\n' "$s"; done"##,
    ] {
        let r = run(&refbin, &["-c"], script);
        let g = run(z, &["--ksh", "-f", "-c"], script);
        assert_eq!(
            (r.stdout.as_str(), r.exit),
            (g.stdout.as_str(), g.exit),
            "ksh vs zshrs --ksh\n  script: {script}"
        );
    }
}

/// ksh93 `read` warns about a read-only target, assigns the others and fails
/// with status 1; dash's `local` copies a read-only variable with its flag, so
/// `local r` keeps it and `local r=2` is the fatal "is read only" (status 2).
#[test]
fn read_only_targets_follow_ksh93_and_dash() {
    let zbin = zshrs_bin();
    let z = zbin.to_str().expect("zshrs path is UTF-8");
    let cases: [(&str, Option<String>, &[&str]); 2] = [
        (
            "--ksh",
            ksh(),
            &[
                r#"readonly r=1; read r <<< x; echo rc=$?"#,
                r#"readonly b=0; printf '1 2 3\n4 5 6\n' | { read a b c; echo "rc=$? a=$a b=$b c=$c"; read x; echo "x=$x"; }"#,
                r#"readonly r=0; read -A r <<< "1 2"; echo rc=$?"#,
            ],
        ),
        (
            "--dash",
            dash(),
            &[
                r#"readonly r=1; f(){ local r; echo "in=$r"; }; f; echo rc=$?"#,
                r#"readonly r=1; f(){ local r x; x=5; echo "$r $x"; }; f"#,
                r#"readonly r=1; f(){ local r=2; echo in=$r; }; f; echo rc=$?"#,
                r#"readonly r=1; f(){ local r; r=3; echo "in=$r"; }; f; echo rc=$?"#,
            ],
        ),
    ];
    let mut ran = 0usize;
    for (flag, refbin, scripts) in cases {
        let Some(refbin) = refbin else {
            eprintln!("SKIP {flag}: reference shell not installed on this host");
            continue;
        };
        ran += 1;
        for script in scripts {
            let r = run(&refbin, &["-c"], script);
            let g = run(z, &[flag, "-f", "-c"], script);
            assert_eq!(
                (r.stdout.as_str(), r.exit),
                (g.stdout.as_str(), g.exit),
                "{refbin} vs zshrs {flag}\n  script: {script}"
            );
        }
    }
    assert!(ran > 0, "neither ksh nor dash is installed — the comparison never ran");
}
