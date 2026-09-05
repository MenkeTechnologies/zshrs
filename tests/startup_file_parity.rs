//! Startup-file parity harness for the zshrs emulation drop-ins.
//!
//! `tests/emulation_parity.rs` proves that a SCRIPT runs identically in
//! `zshrs --X` and the real shell X. This harness proves the other half:
//! that the shell *gets into the same state before the script runs* —
//! that `zshrs --bash` reads `~/.bashrc`, `zshrs --ksh` reads `~/.kshrc`,
//! `zshrs --mksh` reads `~/.mkshrc`, that each reads them for the same
//! invocation shapes and in the same order, and that the effects of the
//! code inside them are byte-identical to the reference shell's.
//!
//! Two layers, both differential against the real binary:
//!
//!   1. **Placement** — a marker line in each candidate file, so a single
//!      run's stdout spells out exactly which files were read and in what
//!      order. Compared byte-for-byte with the reference shell given the
//!      identical `$HOME`, environment and argv.
//!   2. **Fuzz** — seeded, replayable rc bodies drawn from a POSIX-portable
//!      grammar (assignments, arithmetic, functions, exports, parameter
//!      expansion) paired with a probe that reads the state back out. The
//!      rc file is real shell code, not a marker, so a divergence in how
//!      the startup file is *executed* — not just whether it is opened —
//!      fails the run. `ZSHRS_STARTUP_FUZZ_SEED` / `ZSHRS_STARTUP_FUZZ_CASES`
//!      override the defaults; a failure prints the seed and the exact rc
//!      body so it replays.
//!
//! Both layers compare **stdout bytes exactly** plus the exact exit status.
//! stderr is dropped: interactive shells write prompts and job-control
//! chatter there, and its wording legitimately differs per shell.
//!
//! Missing reference shells are reported, never silently passed. Set
//! `ZSHRS_REQUIRE_REF_SHELLS=1` (as CI does) to turn a missing required
//! shell into a failure.
//!
//! The `--csh` leg is deliberately placement-only. `--csh` routes to the
//! zsh parser with `emulate csh` option deltas (zshrs has no separate csh
//! bucket), so it cannot execute real csh syntax such as `set path = (…)`.
//! Its corpus is restricted to bare `echo WORD`, which csh and zsh agree
//! on, and that is enough to assert file selection and ordering.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::path::Path;
use std::process::{Command, Stdio};

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

const ZSH: &[&str] = &["zsh", "/bin/zsh", "/usr/bin/zsh", "/opt/homebrew/bin/zsh"];

/// Which startup file a shell reads when its `$ENV`-equivalent is set, and
/// for which kind of shell.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EnvKind {
    /// `$ENV` — read by an INTERACTIVE shell (the Korn and Bourne line).
    Interactive,
    /// `$BASH_ENV` — read by a NON-interactive shell (bash only).
    NonInteractive,
}

/// What an rc file for this personality is allowed to contain.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Corpus {
    /// POSIX-portable shell code: every shell in the Bourne/Korn/zsh family
    /// executes it identically, so the fuzz layer runs.
    Posix,
    /// `echo WORD` only — see the module header's note on `--csh`.
    EchoOnly,
}

/// One personality's startup-file layout plus the reference binary that
/// defines the truth for it.
struct StartupCase {
    /// Human label / the leg's name.
    name: &'static str,
    /// zshrs flags selecting the drop-in.
    zshrs_flags: &'static [&'static str],
    /// Candidate paths / PATH names for the reference binary, first wins.
    candidates: &'static [&'static str],
    /// File an interactive NON-login shell reads, if the shell has one.
    interactive_rc: Option<&'static str>,
    /// File a login shell reads. The bash chain's first member for bash.
    login_rc: Option<&'static str>,
    /// True when an interactive LOGIN shell reads the interactive file too.
    /// The Korn/Bourne line does (`ksh -i -l` reads `.profile` then
    /// `.kshrc`); bash does not (`bash -i -l` reads `.bash_profile` alone).
    login_also_reads_rc: bool,
    /// The parameter naming an extra startup file, and which kind of shell
    /// reads it.
    env_param: Option<(&'static str, EnvKind)>,
    /// File read when a login shell exits, if any.
    logout_rc: Option<&'static str>,
    /// True when `-l` may NOT be combined with any other flag. tcsh(1):
    /// "-l  The shell is a login shell. Applicable only if -l is the only
    /// flag specified." So `-l -c CMD` is rejected outright (exit 1, no
    /// output) and the login shapes have to feed the probe on stdin with
    /// `-l` alone. `-i -l` is inexpressible for such a shell and is
    /// skipped rather than compared against a run that never happened.
    login_flag_is_exclusive: bool,
    corpus: Corpus,
    /// Best-effort leg: a missing reference is a skip even under
    /// `ZSHRS_REQUIRE_REF_SHELLS`.
    optional: bool,
}

/// The layouts, each row verified against the reference binary named in it.
/// See `src/extensions/emulation_startup.rs` for the manual citations and
/// the reference runs the table was derived from.
const STARTUP_CASES: &[StartupCase] = &[
    StartupCase {
        name: "zsh",
        zshrs_flags: &["--zsh"],
        candidates: ZSH,
        interactive_rc: Some(".zshrc"),
        login_rc: Some(".zprofile"),
        login_also_reads_rc: true,
        env_param: None,
        logout_rc: Some(".zlogout"),
        corpus: Corpus::Posix,
        login_flag_is_exclusive: false,
        optional: false,
    },
    StartupCase {
        name: "bash",
        zshrs_flags: &["--bash"],
        candidates: &[
            "bash",
            "/bin/bash",
            "/usr/bin/bash",
            "/opt/homebrew/bin/bash",
        ],
        interactive_rc: Some(".bashrc"),
        login_rc: Some(".bash_profile"),
        // bash(1): an interactive login shell reads the profile chain and
        // NOT ~/.bashrc. This is the one row that differs from every other
        // shell here, and the reason the flag exists.
        login_also_reads_rc: false,
        env_param: Some(("BASH_ENV", EnvKind::NonInteractive)),
        logout_rc: Some(".bash_logout"),
        corpus: Corpus::Posix,
        login_flag_is_exclusive: false,
        optional: false,
    },
    StartupCase {
        name: "ksh",
        zshrs_flags: &["--ksh"],
        candidates: &["ksh", "/bin/ksh", "/usr/bin/ksh"],
        // ksh(1): "$ENV … The default value is $HOME/.kshrc."
        interactive_rc: Some(".kshrc"),
        login_rc: Some(".profile"),
        login_also_reads_rc: true,
        env_param: Some(("ENV", EnvKind::Interactive)),
        logout_rc: None,
        corpus: Corpus::Posix,
        login_flag_is_exclusive: false,
        optional: false,
    },
    StartupCase {
        name: "mksh",
        zshrs_flags: &["--mksh"],
        candidates: &[
            "mksh",
            "/bin/mksh",
            "/usr/bin/mksh",
            "/opt/homebrew/bin/mksh",
        ],
        // mksh(1): "if unset or empty, the user mkshrc profile is processed".
        interactive_rc: Some(".mkshrc"),
        login_rc: Some(".profile"),
        login_also_reads_rc: true,
        env_param: Some(("ENV", EnvKind::Interactive)),
        logout_rc: None,
        corpus: Corpus::Posix,
        login_flag_is_exclusive: false,
        optional: true,
    },
    StartupCase {
        name: "pdksh",
        zshrs_flags: &["--pdksh"],
        candidates: &["pdksh", "/bin/pdksh", "/usr/bin/pdksh"],
        // OpenBSD ksh(1) documents no default for $ENV — it tells the user
        // to `export ENV=$HOME/.kshrc` by hand. That is the one behaviour
        // separating this leg from `--mksh`.
        interactive_rc: None,
        login_rc: Some(".profile"),
        login_also_reads_rc: true,
        env_param: Some(("ENV", EnvKind::Interactive)),
        logout_rc: None,
        corpus: Corpus::Posix,
        login_flag_is_exclusive: false,
        optional: true,
    },
    StartupCase {
        name: "sh",
        zshrs_flags: &["--sh"],
        candidates: &["/bin/sh"],
        interactive_rc: None,
        login_rc: Some(".profile"),
        login_also_reads_rc: true,
        env_param: Some(("ENV", EnvKind::Interactive)),
        logout_rc: None,
        corpus: Corpus::Posix,
        login_flag_is_exclusive: false,
        optional: false,
    },
    StartupCase {
        name: "dash",
        zshrs_flags: &["--dash"],
        candidates: &["/bin/dash", "/usr/bin/dash", "/opt/homebrew/bin/dash"],
        interactive_rc: None,
        login_rc: Some(".profile"),
        login_also_reads_rc: true,
        env_param: Some(("ENV", EnvKind::Interactive)),
        logout_rc: None,
        corpus: Corpus::Posix,
        login_flag_is_exclusive: false,
        optional: false,
    },
    StartupCase {
        name: "csh",
        zshrs_flags: &["--csh"],
        candidates: &["/bin/csh", "/usr/bin/csh"],
        // tcsh(1): "Non-login shells read only /etc/csh.cshrc and ~/.tcshrc
        // or ~/.cshrc on startup." Only ~/.cshrc is created, so the
        // ~/.tcshrc preference never fires.
        interactive_rc: Some(".cshrc"),
        login_rc: Some(".login"),
        login_also_reads_rc: true,
        env_param: None,
        logout_rc: Some(".logout"),
        corpus: Corpus::EchoOnly,
        login_flag_is_exclusive: true,
        optional: true,
    },
];

// ─────────────────────────── process plumbing ───────────────────────────

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

/// One shell invocation under a controlled `$HOME` and a cleared
/// environment. Returns raw stdout bytes plus the exact exit code.
///
/// The environment is cleared rather than inherited so the harness cannot
/// be perturbed by whatever `$ENV` / `$BASH_ENV` / `$ZDOTDIR` the developer
/// happens to export. stdin is `/dev/null`: an interactive shell given a
/// terminal would block at its prompt.
fn run(
    bin: &str,
    args: &[&str],
    home: &Path,
    env: &[(&str, String)],
    stdin: Option<&str>,
) -> (Vec<u8>, i32) {
    run_as(bin, None, args, home, env, stdin)
}

/// [`run`] with an explicit `argv[0]`. `None` keeps the binary's own path,
/// which is what a normal exec gives; `Some("-bash")` reproduces how
/// login(1) and sshd start a LOGIN shell — a leading dash on the name is
/// the only signal the shell gets.
fn run_as(
    bin: &str,
    arg0: Option<&str>,
    args: &[&str],
    home: &Path,
    env: &[(&str, String)],
    stdin: Option<&str>,
) -> (Vec<u8>, i32) {
    let mut cmd = Command::new(bin);
    if let Some(a0) = arg0 {
        use std::os::unix::process::CommandExt;
        cmd.arg0(a0);
    }
    cmd.args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "dumb")
        // A fixed locale so no shell formats a number or sorts differently.
        .env("LC_ALL", "C")
        .stderr(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let Some(script) = stdin else {
        // No script on stdin: `/dev/null`, so an interactive shell handed a
        // terminal cannot block at its prompt.
        cmd.stdin(Stdio::null());
        let out = cmd
            .output()
            .unwrap_or_else(|e| panic!("spawn {bin} {args:?}: {e}"));
        return (out.stdout, out.status.code().unwrap_or(-1));
    };
    use std::io::Write;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {bin} {args:?}: {e}"));
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(script.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    (out.stdout, out.status.code().unwrap_or(-1))
}

/// A scratch `$HOME` with the given `(filename, body)` pairs written into it.
fn home_with(files: &[(&str, String)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (name, body) in files {
        std::fs::write(dir.path().join(name), body).unwrap_or_else(|e| {
            panic!("write {name}: {e}");
        });
    }
    dir
}

/// Lines a reference shell prints on STDOUT purely because the harness
/// runs it without a controlling terminal.
///
/// tcsh emits these two when a login shell finds no tty, before any
/// startup file runs. They describe the test environment, not the shell's
/// startup-file behaviour, and giving every leg a pty to avoid two fixed
/// strings is not a trade worth making. This is the ONLY normalization in
/// the harness — everything else is compared as raw bytes.
const REFERENCE_TTY_NOISE: &[&str] = &[
    "Warning: no access to tty (Undefined error: 0).",
    "Thus no job control in this shell.",
];

/// Drop [`REFERENCE_TTY_NOISE`] lines. Applied to BOTH sides so it can
/// never turn a real difference into a match on one side only; the caller
/// additionally asserts it is a no-op on the zshrs side, since zshrs emits
/// no startup chatter of its own by design.
fn strip_tty_noise(bytes: &[u8]) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return bytes.to_vec();
    };
    if !REFERENCE_TTY_NOISE.iter().any(|n| text.contains(n)) {
        return bytes.to_vec();
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if REFERENCE_TTY_NOISE.contains(&line.trim_end_matches('\n')) {
            continue;
        }
        out.push_str(line);
    }
    out.into_bytes()
}

/// Render captured stdout for a failure message: UTF-8 when it is, escaped
/// bytes otherwise, so a divergence in invisible characters is still legible.
fn show(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => format!("{s:?}"),
        Err(_) => format!("{bytes:x?}"),
    }
}

/// Run one `(argv, env)` shape through both the reference shell and zshrs
/// against IDENTICAL scratch homes, and return a failure description when
/// stdout bytes or the exit code differ.
///
/// Each side gets its own copy of the home directory so a startup file that
/// writes into `$HOME` cannot leak from one side into the other.
fn diff_shape(
    case: &StartupCase,
    refbin: &str,
    label: &str,
    files: &[(&str, String)],
    args: &[&str],
    env: &[(&str, String)],
    stdin: Option<&str>,
) -> Option<String> {
    let ref_home = home_with(files);
    let (r_out, r_code) = run(refbin, args, ref_home.path(), env, stdin);

    let zsh_home = home_with(files);
    let mut zargs: Vec<&str> = case.zshrs_flags.to_vec();
    zargs.extend_from_slice(args);
    let (z_out, z_code) = run(&zshrs_bin(), &zargs, zsh_home.path(), env, stdin);

    // zshrs prints no startup banners or tty diagnostics — a hard project
    // rule — so the noise filter must be a no-op on its side. Asserting
    // that keeps the filter from ever masking a zshrs regression.
    assert_eq!(
        strip_tty_noise(&z_out),
        z_out,
        "[{}] {label}: zshrs emitted reference-shell tty chatter of its own",
        case.name
    );
    let r_out = strip_tty_noise(&r_out);

    if r_out == z_out && r_code == z_code {
        return None;
    }
    let env_desc = if env.is_empty() {
        String::new()
    } else {
        format!(
            "  env: {}\n",
            env.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    let files_desc = files
        .iter()
        .map(|(n, b)| format!("    --- {n} ---\n{}", indent(b)))
        .collect::<Vec<_>>()
        .join("");
    let stdin_desc = stdin
        .map(|s| format!("  stdin: {s:?}\n"))
        .unwrap_or_default();
    Some(format!(
        "  [{}] {label}\n  argv: {:?} (+{:?})\n{stdin_desc}{env_desc}{files_desc}\
         \n    ref({refbin}): code={r_code} out={}\n    zshrs:  code={z_code} out={}",
        case.name,
        args,
        case.zshrs_flags,
        show(&r_out),
        show(&z_out),
    ))
}

fn indent(body: &str) -> String {
    body.lines()
        .map(|l| format!("      {l}\n"))
        .collect::<String>()
}

// ───────────────────────────── corpora ──────────────────────────────────

/// The marker a placement case writes into each candidate file. Printed on
/// stdout so the ORDER of the reads is part of the compared bytes.
fn marker(file: &str, corpus: Corpus) -> String {
    match corpus {
        // `printf` avoids every `echo`-escape divergence across the family.
        Corpus::Posix => format!("printf 'READ {file}\\n'\n"),
        // csh has no printf builtin and its `echo` agrees with zsh's for a
        // bare word, which is all this corpus uses.
        Corpus::EchoOnly => format!("echo READ {file}\n"),
    }
}

/// A statement pair for the fuzz layer: `setup` goes into the startup file,
/// `probe` into the `-c` command that reads the state back out.
struct Stmt {
    setup: String,
    probe: String,
}

/// Draw one POSIX-portable startup-file statement plus the probe that
/// observes it.
///
/// Everything here is deliberately in the shared subset of
/// zsh/bash/ksh/mksh/dash/sh: no arrays, no `[[`, no `echo` escapes, no
/// unquoted expansions (word-splitting is where these shells legitimately
/// differ, and `emulation_parity.rs` already pins that separately). What is
/// under test is that the startup file RUNS, in the right order, with the
/// right effect — not the language.
fn gen_stmt(rng: &mut StdRng, n: usize) -> Stmt {
    let a = rng.gen_range(2i64..20);
    let b = rng.gen_range(2i64..20);
    match rng.gen_range(0..8) {
        0 => Stmt {
            setup: format!("V{n}=plain{n}"),
            probe: format!("printf '%s\\n' \"$V{n}\""),
        },
        1 => Stmt {
            // Embedded spaces: the value must survive as ONE word.
            setup: format!("V{n}='sp {n} ace'"),
            probe: format!("printf '[%s]\\n' \"$V{n}\""),
        },
        2 => Stmt {
            setup: format!("V{n}=$(( {a} * {b} + {n} ))"),
            probe: format!("printf '%d\\n' \"$V{n}\""),
        },
        3 => Stmt {
            setup: format!("f{n}() {{ printf 'f{n}:%s\\n' \"$1\"; }}"),
            probe: format!("f{n} arg{n}"),
        },
        4 => Stmt {
            // An export in the startup file must be visible to the command.
            setup: format!("export E{n}=exp{n}"),
            probe: format!("printf '%s\\n' \"$E{n}\""),
        },
        5 => Stmt {
            setup: format!("V{n}=pre{n}post"),
            probe: format!("printf '%s\\n' \"${{V{n}#pre}}\""),
        },
        6 => Stmt {
            // Unset-with-default: the startup file leaves it empty on
            // purpose, so the probe must take the `:-` branch.
            setup: format!("V{n}="),
            probe: format!("printf '%s\\n' \"${{V{n}:-fallback{n}}}\""),
        },
        _ => Stmt {
            setup: format!("V{n}={a}"),
            probe: format!("printf '%d\\n' \"$(( V{n} + {b} ))\""),
        },
    }
}

/// Build one fuzz body: `count` statements plus a leading marker so the
/// file's own execution order is observable alongside its effects.
fn gen_body(rng: &mut StdRng, tag: &str, count: usize) -> (String, String) {
    let mut setup = format!("printf 'ENTER {tag}\\n'\n");
    let mut probe = String::new();
    for n in 0..count {
        let s = gen_stmt(rng, n);
        setup.push_str(&s.setup);
        setup.push('\n');
        probe.push_str(&s.probe);
        probe.push_str("; ");
    }
    setup.push_str(&format!("printf 'LEAVE {tag}\\n'\n"));
    (setup, probe)
}

/// Divergences that are known, understood, and deliberately NOT fixed
/// here — `(leg, shape label, why)`.
///
/// A listed gap does not fail the run, but a listed gap that no longer
/// REPRODUCES does: the entry has to be deleted when the underlying
/// behaviour is fixed, so this list cannot quietly rot into a list of
/// things nobody remembers.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[(
    "zsh",
    "argv0-login, interactive",
    "zsh runs ~/.zlogout when an interactive login shell terminates \
     normally, not only via the `exit` builtin. zshrs's `-c` dispatch \
     leaves through `std::process::exit` without going through `zexit`, \
     and routing it there would double-fire the EXIT trap that path \
     already handles. bash does NOT do this (`bash -l -c true` reads no \
     ~/.bash_logout), so the bash drop-in is unaffected.",
)];

fn known_gap(leg: &str, label: &str) -> Option<&'static str> {
    KNOWN_GAPS
        .iter()
        .find(|(l, s, _)| *l == leg && *s == label)
        .map(|(_, _, why)| *why)
}

/// True when this reference binary is bash wearing another name.
///
/// macOS ships `/bin/sh` as bash 3.2, and bash-as-sh keeps bash's own
/// startup rule — the profile chain needs an interactive shell or an
/// explicit `--login`, so `-sh -c CMD` reads nothing. Every real POSIX sh
/// (dash, and ksh/mksh on the Korn side) reads `~/.profile` there, which
/// is what `--sh` implements and what `/bin/sh` on a Debian box does. The
/// leg therefore cannot be compared against a bash-backed `/bin/sh` for
/// the implicit-login shapes; detecting it beats hardcoding a platform.
fn reference_is_bash(refbin: &str) -> bool {
    let out = Command::new(refbin)
        .args(["-c", "printf %s \"${BASH_VERSION:-}\""])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    out.map(|o| !o.stdout.is_empty()).unwrap_or(false)
}

// ─────────────────────────── invocation shapes ──────────────────────────

/// The `(label, argv, sets-interactive, sets-login)` shapes every leg is
/// exercised through. `-c` is used for all of them: it is the only shape
/// that terminates deterministically for an interactive shell, and both
/// bash and zshrs source the interactive rc for `-i -c` (verified against
/// bash 5.3.15 and zsh 5.9).
const SHAPES: &[(&str, &[&str])] = &[
    ("interactive non-login", &["-i"]),
    ("login", &["-l"]),
    ("interactive login", &["-i", "-l"]),
    ("non-interactive non-login", &[]),
];

/// Resolve one SHAPES row into the concrete `(argv, stdin)` this leg needs.
///
/// Most shells take the probe as `-c CMD`. A shell whose `-l` is exclusive
/// (tcsh) rejects `-l -c`, so its login shape runs `-l` alone and feeds the
/// probe on stdin — a non-interactive login shell, which is exactly what
/// the shape is meant to exercise. `None` means the shape is inexpressible
/// for this leg and must be skipped rather than faked.
fn shape_invocation<'a>(
    case: &StartupCase,
    shape_args: &'a [&'a str],
    probe: &'a str,
) -> Option<(Vec<&'a str>, Option<&'a str>)> {
    let is_login = shape_args.contains(&"-l");
    let is_interactive = shape_args.contains(&"-i");
    if case.login_flag_is_exclusive && is_login {
        if is_interactive {
            // `-i -l` cannot be expressed at all for this shell.
            return None;
        }
        return Some((vec!["-l"], Some(probe)));
    }
    let mut argv: Vec<&str> = shape_args.to_vec();
    argv.push("-c");
    argv.push(probe);
    Some((argv, None))
}

/// Every file this leg could read, so a startup file the shell should NOT
/// have read still shows up in stdout when it wrongly does.
fn all_candidate_files(case: &StartupCase) -> Vec<&'static str> {
    let mut v = Vec::new();
    v.extend(case.interactive_rc);
    v.extend(case.login_rc);
    v
}

// ─────────────────────────────── tests ──────────────────────────────────

/// Layer 1: which startup files each invocation shape reads, and in what
/// order, byte-for-byte against the reference shell.
#[test]
fn startup_file_placement_matches_the_reference_shell() {
    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut tested = 0usize;

    for case in STARTUP_CASES {
        let Some(refbin) = find_shell(case.candidates) else {
            if !case.optional {
                missing.push(case.name);
            }
            eprintln!("skip: `{}` reference not found", case.name);
            continue;
        };
        tested += 1;
        eprintln!("placement: {} vs {refbin}", case.name);

        // Every candidate file present at once. A leg that reads one it
        // should not read shows up as an extra line in stdout.
        let files: Vec<(&str, String)> = all_candidate_files(case)
            .into_iter()
            .map(|f| (f, marker(f, case.corpus)))
            .collect();

        for (label, args) in SHAPES {
            let probe = match case.corpus {
                Corpus::Posix => "printf 'PROBE\\n'",
                Corpus::EchoOnly => "echo PROBE",
            };
            let Some((argv, stdin)) = shape_invocation(case, args, probe) else {
                continue;
            };
            if let Some(m) = diff_shape(case, &refbin, label, &files, &argv, &[], stdin) {
                mismatches.push(m);
            }
        }

        // Independently confirm the table's own claim against the REFERENCE
        // binary, not just zshrs-vs-reference. Byte parity alone cannot
        // catch a case row that mis-describes the shell — both sides would
        // simply agree on the wrong thing. This is what notices a platform
        // whose bash is patched, or a table row edited by hand.
        if let (Some(rc), Some(_), false) = (
            case.interactive_rc,
            case.login_rc,
            case.login_flag_is_exclusive,
        ) {
            let home = home_with(&files);
            let probe = match case.corpus {
                Corpus::Posix => "printf 'PROBE\\n'",
                Corpus::EchoOnly => "echo PROBE",
            };
            let (out, _) = run(&refbin, &["-i", "-l", "-c", probe], home.path(), &[], None);
            let saw_rc = String::from_utf8_lossy(&out).contains(&format!("READ {rc}"));
            assert_eq!(
                saw_rc,
                case.login_also_reads_rc,
                "{}: table says an interactive login shell {} read {rc}, but {refbin} {}.                  Fix the table (and src/extensions/emulation_startup.rs with it).",
                case.name,
                if case.login_also_reads_rc { "does" } else { "does not" },
                if saw_rc { "did" } else { "did not" },
            );
        }
    }

    report(tested, &missing, &mismatches, "startup-file placement");
}

/// Layer 1b: the `$ENV` / `$BASH_ENV` file, which kind of shell reads it,
/// and where it lands relative to the profile.
#[test]
fn env_parameter_file_matches_the_reference_shell() {
    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut tested = 0usize;

    for case in STARTUP_CASES {
        let Some((param, kind)) = case.env_param else {
            continue;
        };
        let Some(refbin) = find_shell(case.candidates) else {
            if !case.optional {
                missing.push(case.name);
            }
            continue;
        };
        tested += 1;
        eprintln!("env-param: {} ({param}) vs {refbin}", case.name);

        // The env file lives OUTSIDE $HOME so it cannot be confused with a
        // per-user rc; both sides get an identical copy at an identical path.
        let envdir = tempfile::tempdir().expect("tempdir");
        let envfile = envdir.path().join("envfile");
        std::fs::write(&envfile, "printf 'READ envfile\\n'\n").expect("write envfile");
        let env = vec![(param, envfile.to_string_lossy().into_owned())];

        let mut files: Vec<(&str, String)> = all_candidate_files(case)
            .into_iter()
            .map(|f| (f, marker(f, case.corpus)))
            .collect();
        files.sort_by_key(|(n, _)| *n);
        files.dedup_by_key(|(n, _)| *n);

        // Both kinds are exercised through every shape: the point is that
        // an INTERACTIVE shell reads $ENV and never $BASH_ENV, and a
        // NON-interactive one the reverse.
        let _ = kind;
        for (label, args) in SHAPES {
            let Some((argv, stdin)) = shape_invocation(case, args, "printf 'PROBE\\n'") else {
                continue;
            };
            if let Some(m) = diff_shape(
                case,
                &refbin,
                &format!("{label} with ${param} set"),
                &files,
                &argv,
                &env,
                stdin,
            ) {
                mismatches.push(m);
            }
        }
    }

    report(tested, &missing, &mismatches, "$ENV/$BASH_ENV placement");
}

/// Layer 2: seeded fuzz. Real shell code in the startup file, a probe that
/// reads the resulting state back, byte parity on both.
#[test]
fn startup_file_contents_execute_identically() {
    let seed: u64 = std::env::var("ZSHRS_STARTUP_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5747_5f53_5441_5254);
    let cases: usize = std::env::var("ZSHRS_STARTUP_FUZZ_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);

    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut tested = 0usize;

    for case in STARTUP_CASES {
        // The echo-only legs cannot host arbitrary shell code; layer 1
        // already covers what they can prove.
        if case.corpus != Corpus::Posix {
            continue;
        }
        let Some(refbin) = find_shell(case.candidates) else {
            if !case.optional {
                missing.push(case.name);
            }
            continue;
        };
        tested += 1;
        eprintln!(
            "fuzz: {} vs {refbin} ({cases} cases, seed {seed:#x})",
            case.name
        );

        for i in 0..cases {
            // Per-case seed so any single divergence replays on its own:
            // ZSHRS_STARTUP_FUZZ_SEED=<seed> with the printed index.
            let mut rng = StdRng::seed_from_u64(seed ^ (i as u64).wrapping_mul(0x9E37_79B9));
            let stmts = rng.gen_range(1..6);

            // Drive the file the shape actually reads, so the fuzz body is
            // never written somewhere the shell would ignore.
            let (target, args): (Option<&str>, &[&str]) = if i % 2 == 0 {
                (case.interactive_rc, &["-i"])
            } else {
                (case.login_rc, &["-l"])
            };
            let Some(target) = target else { continue };

            let (body, probe) = gen_body(&mut rng, target, stmts);
            let files = vec![(target, body.clone())];
            let mut argv: Vec<&str> = args.to_vec();
            argv.push("-c");
            argv.push(&probe);

            if let Some(m) = diff_shape(
                case,
                &refbin,
                &format!("fuzz #{i} (seed {seed:#x}) into {target}"),
                &files,
                &argv,
                &[],
                None,
            ) {
                mismatches.push(m);
            }
        }
    }

    report(tested, &missing, &mismatches, "startup-file execution");
}

/// Script-file invocation: `SHELL script.sh`. bash reads `$BASH_ENV`
/// here, zsh reads `/etc/zshenv` + `~/.zshenv`, and the Korn/Bourne line
/// reads nothing unless `--login` was given. The script lives outside both
/// scratch homes so the two sides receive an identical path in argv.
#[test]
fn script_file_startup_matches_the_reference_shell() {
    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut tested = 0usize;

    let scriptdir = tempfile::tempdir().expect("tempdir");
    let posix_script = scriptdir.path().join("probe.sh");
    std::fs::write(&posix_script, "printf 'PROBE\n'\n").expect("write script");
    let echo_script = scriptdir.path().join("probe.csh");
    std::fs::write(&echo_script, "echo PROBE\n").expect("write script");

    for case in STARTUP_CASES {
        let Some(refbin) = find_shell(case.candidates) else {
            if !case.optional {
                missing.push(case.name);
            }
            continue;
        };
        tested += 1;
        eprintln!("script: {} vs {refbin}", case.name);

        let script = match case.corpus {
            Corpus::Posix => posix_script.to_string_lossy().into_owned(),
            Corpus::EchoOnly => echo_script.to_string_lossy().into_owned(),
        };

        // Every per-user file this shell could read, plus zsh's `.zshenv`,
        // which is the one file read on EVERY invocation including a
        // script — an extra line in stdout if a leg wrongly reads it.
        let mut files: Vec<(&str, String)> = all_candidate_files(case)
            .into_iter()
            .map(|f| (f, marker(f, case.corpus)))
            .collect();
        if case.name == "zsh" {
            files.push((".zshenv", marker(".zshenv", case.corpus)));
        }

        // Plain script, then the `--login` form, then with the shell's
        // `$ENV`-equivalent exported.
        let envdir = tempfile::tempdir().expect("tempdir");
        let envfile = envdir.path().join("envfile");
        std::fs::write(&envfile, marker("envfile", case.corpus)).expect("write envfile");

        for (label, args, env) in [
            ("script", vec![script.as_str()], Vec::new()),
            ("login script", vec!["-l", script.as_str()], Vec::new()),
            (
                "script with $ENV-equivalent set",
                vec![script.as_str()],
                case.env_param
                    .map(|(p, _)| vec![(p, envfile.to_string_lossy().into_owned())])
                    .unwrap_or_default(),
            ),
        ] {
            // tcsh's `-l` is exclusive, so it cannot take a script operand.
            if args.contains(&"-l") && case.login_flag_is_exclusive {
                continue;
            }
            if let Some(m) = diff_shape(case, &refbin, label, &files, &args, &env, None) {
                mismatches.push(m);
            }
        }
    }

    report(tested, &missing, &mismatches, "script-file startup");
}

/// LOGIN SHELL, started the way a login shell is actually started: the
/// binary is exec'd with a leading dash on `argv[0]` (`-bash`, `-zsh`, …).
/// This is the shape `chsh` + login(1) + sshd produce, and no flag is
/// involved — so a drop-in that only honours `--bash` / `-l` looks fine in
/// every other test here and still fails the moment someone sets it as
/// their shell.
///
/// zshrs is reached through a symlink named after the reference shell, the
/// same way a user installs it (`ln -s zshrs ~/bin/bash` + `chsh`), which
/// also exercises the `argv[0]` personality inference.
#[test]
fn login_shell_via_argv0_dash_matches_the_reference_shell() {
    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut tested = 0usize;

    let linkdir = tempfile::tempdir().expect("tempdir");

    for case in STARTUP_CASES {
        let Some(refbin) = find_shell(case.candidates) else {
            if !case.optional {
                missing.push(case.name);
            }
            continue;
        };
        // The symlink's NAME is what drives zshrs's argv[0] personality
        // inference, so it has to be the reference shell's own basename.
        let name = Path::new(&refbin)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(case.name);
        let link = linkdir.path().join(name);
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(zshrs_bin(), &link).expect("symlink");
        let link_s = link.to_string_lossy().into_owned();
        let arg0 = format!("-{name}");
        tested += 1;
        eprintln!("argv0-login: {} as {arg0} vs {refbin}", case.name);

        let probe = match case.corpus {
            Corpus::Posix => "printf 'PROBE\n'",
            Corpus::EchoOnly => "echo PROBE",
        };
        let mut files: Vec<(&str, String)> = all_candidate_files(case)
            .into_iter()
            .map(|f| (f, marker(f, case.corpus)))
            .collect();
        files.extend(case.logout_rc.map(|f| (f, marker(f, case.corpus))));

        // A bash-backed `/bin/sh` follows bash's profile rule, not the
        // POSIX one this leg implements — see `reference_is_bash`.
        let sh_is_bash = case.name == "sh" && reference_is_bash(&refbin);
        if sh_is_bash {
            eprintln!(
                "  note: {refbin} is bash wearing sh's name; skipping the \
                 non-interactive implicit-login shapes for this leg"
            );
        }

        for (label, args) in [
            ("argv0-login, non-interactive", vec!["-c", probe]),
            ("argv0-login, interactive", vec!["-i", "-c", probe]),
            ("argv0-login, exit", vec!["-c", "exit"]),
        ] {
            if sh_is_bash && !args.contains(&"-i") {
                continue;
            }
            let ref_home = home_with(&files);
            let (r_out, r_code) = run_as(&refbin, Some(&arg0), &args, ref_home.path(), &[], None);
            let zsh_home = home_with(&files);
            let (z_out, z_code) = run_as(&link_s, Some(&arg0), &args, zsh_home.path(), &[], None);

            let r_out = strip_tty_noise(&r_out);
            let diverged = r_out != z_out || r_code != z_code;
            if let Some(why) = known_gap(case.name, label) {
                assert!(
                    diverged,
                    "[{}] {label} is listed in KNOWN_GAPS but now MATCHES the \
                     reference. The gap is fixed — delete the entry.\n  was: {why}",
                    case.name
                );
                eprintln!("  known gap [{}] {label}: {why}", case.name);
                continue;
            }
            if diverged {
                mismatches.push(format!(
                    "  [{}] {label}\n  argv0: {arg0}  argv: {args:?}\n\
                     \n    ref({refbin}): code={r_code} out={}\n    zshrs:  code={z_code} out={}",
                    case.name,
                    show(&r_out),
                    show(&z_out),
                ));
            }
        }
    }

    report(tested, &missing, &mismatches, "argv[0]-dash login shell");
}

/// `argv[0]`'s dash sets `shopt login_shell` — bash's own report of how it
/// was started — independently of whether any startup file was read. It is
/// derived state, not a configurable default, so it must survive the
/// `shopt` defaults pass that runs before user code.
#[test]
fn argv0_dash_sets_bash_login_shell_shopt() {
    let case = &STARTUP_CASES[1];
    assert_eq!(case.name, "bash", "STARTUP_CASES[1] must stay the bash leg");
    let Some(refbin) = find_shell(case.candidates) else {
        eprintln!("skip: bash reference not found");
        return;
    };
    let linkdir = tempfile::tempdir().expect("tempdir");
    let link = linkdir.path().join("bash");
    std::os::unix::fs::symlink(zshrs_bin(), &link).expect("symlink");
    let link_s = link.to_string_lossy().into_owned();

    let home = home_with(&[]);
    let probe = "shopt -q login_shell && printf 'LOGIN\n' || printf 'NOLOGIN\n'";
    for (arg0, args, want) in [
        ("-bash", vec!["-c", probe], "LOGIN\n"),
        ("bash", vec!["-c", probe], "NOLOGIN\n"),
        ("bash", vec!["-l", "-c", probe], "LOGIN\n"),
        ("-bash", vec!["-i", "-c", probe], "LOGIN\n"),
    ] {
        let (r_out, _) = run_as(&refbin, Some(arg0), &args, home.path(), &[], None);
        let (z_out, _) = run_as(&link_s, Some(arg0), &args, home.path(), &[], None);
        assert_eq!(
            String::from_utf8_lossy(&strip_tty_noise(&r_out)),
            want,
            "reference bash changed: {arg0} {args:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&z_out),
            want,
            "zshrs as {arg0} {args:?}: shopt login_shell disagrees with bash"
        );
    }
}

/// The logout file: read when a login shell exits, not otherwise.
#[test]
fn logout_file_matches_the_reference_shell() {
    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut tested = 0usize;

    for case in STARTUP_CASES {
        let Some(logout) = case.logout_rc else {
            continue;
        };
        // zsh's own `.zlogout` is only read for an INTERACTIVE login shell,
        // and zshrs reaches the exit path only through the `exit` builtin;
        // the echo-only leg cannot host the marker's `printf`.
        if case.corpus != Corpus::Posix {
            continue;
        }
        let Some(refbin) = find_shell(case.candidates) else {
            if !case.optional {
                missing.push(case.name);
            }
            continue;
        };
        tested += 1;
        eprintln!("logout: {} ({logout}) vs {refbin}", case.name);

        let files = vec![(logout, marker(logout, case.corpus))];
        // `exit` explicitly, which is the shape bash documents for a
        // non-interactive login shell, and the only one zshrs routes
        // through `zexit`.
        for (label, args) in [
            ("login + exit", vec!["-l", "-c", "exit"]),
            ("non-login + exit", vec!["-c", "exit"]),
        ] {
            if let Some(m) = diff_shape(case, &refbin, label, &files, &args, &[], None) {
                mismatches.push(m);
            }
        }
    }

    report(tested, &missing, &mismatches, "logout-file placement");
}

/// bash's `~/.bash_profile` → `~/.bash_login` → `~/.profile` chain: the
/// FIRST readable one wins and the rest are skipped even when present.
#[test]
fn bash_profile_chain_matches_bash() {
    let case = &STARTUP_CASES[1];
    assert_eq!(case.name, "bash", "STARTUP_CASES[1] must stay the bash leg");
    let Some(refbin) = find_shell(case.candidates) else {
        eprintln!("skip: bash reference not found");
        return;
    };

    let chain = [".bash_profile", ".bash_login", ".profile"];
    let mut mismatches = Vec::new();
    // Drop one member at a time, so each run has a different winner.
    for start in 0..chain.len() {
        let files: Vec<(&str, String)> = chain[start..]
            .iter()
            .map(|f| (*f, marker(f, Corpus::Posix)))
            .collect();
        if let Some(m) = diff_shape(
            case,
            &refbin,
            &format!("profile chain from {}", chain[start]),
            &files,
            &["-l", "-c", "printf 'PROBE\\n'"],
            &[],
            None,
        ) {
            mismatches.push(m);
        }
    }
    assert!(
        mismatches.is_empty(),
        "bash profile chain diverged:\n{}",
        mismatches.join("\n")
    );
}

/// bash's startup-file flags: `--norc`, `--noprofile`, `--rcfile FILE` and
/// its `--init-file` alias, each compared against bash itself.
#[test]
fn bash_startup_flags_match_bash() {
    let case = &STARTUP_CASES[1];
    assert_eq!(case.name, "bash", "STARTUP_CASES[1] must stay the bash leg");
    let Some(refbin) = find_shell(case.candidates) else {
        eprintln!("skip: bash reference not found");
        return;
    };

    let files = vec![
        (".bashrc", marker(".bashrc", Corpus::Posix)),
        (".bash_profile", marker(".bash_profile", Corpus::Posix)),
        (".altrc", marker(".altrc", Corpus::Posix)),
    ];
    // `--rcfile` takes a path; both sides get their own $HOME, so the file
    // is named relative to each. bash resolves it against $PWD, so the two
    // runs must be given the SAME relative path — hence the `cd`-free
    // absolute form is avoided and the flag is exercised through a home
    // that both sides receive identically.
    let mut mismatches = Vec::new();
    for (label, args) in [
        ("--norc", vec!["--norc", "-i", "-c", "printf 'PROBE\\n'"]),
        (
            "--noprofile",
            vec!["--noprofile", "-l", "-c", "printf 'PROBE\\n'"],
        ),
        (
            "--norc + --noprofile (interactive login)",
            vec![
                "--norc",
                "--noprofile",
                "-i",
                "-l",
                "-c",
                "printf 'PROBE\\n'",
            ],
        ),
    ] {
        if let Some(m) = diff_shape(case, &refbin, label, &files, &args, &[], None) {
            mismatches.push(m);
        }
    }

    // `--rcfile` / `--init-file` need a path that exists identically for
    // both sides; write it outside $HOME and point both at it.
    let alt = tempfile::tempdir().expect("tempdir");
    let altrc = alt.path().join("altrc");
    std::fs::write(&altrc, "printf 'READ altrc\\n'\n").expect("write altrc");
    let altrc_s = altrc.to_string_lossy().into_owned();
    for flag in ["--rcfile", "--init-file"] {
        let args = vec![flag, altrc_s.as_str(), "-i", "-c", "printf 'PROBE\\n'"];
        if let Some(m) = diff_shape(case, &refbin, flag, &files, &args, &[], None) {
            mismatches.push(m);
        }
    }

    assert!(
        mismatches.is_empty(),
        "bash startup flags diverged:\n{}",
        mismatches.join("\n")
    );
}

/// zshrs's `--no-rcs` suppresses EVERY startup file in every drop-in.
///
/// Note it is `--no-rcs` and NOT `-f` for the Bourne family: those modes
/// install `SHOPTIONLETTERS`, where `-f` is bash's own "disable pathname
/// expansion" (verified: `bash -f -c 'echo /etc/pas*'` prints the pattern
/// unexpanded, and `zshrs --bash -f` matches). `-f` only means NO_RCS
/// where zsh's letter table is in force. Both halves are pinned here, so
/// a future change that makes `-f` swallow rc files in `--bash` — which
/// would silently break every bash script relying on `set -f` semantics —
/// fails this test.
#[test]
fn no_rcs_suppresses_every_startup_file() {
    for case in STARTUP_CASES {
        let files: Vec<(&str, String)> = all_candidate_files(case)
            .into_iter()
            .map(|f| (f, marker(f, case.corpus)))
            .collect();
        if files.is_empty() {
            continue;
        }
        let home = home_with(&files);
        let probe = match case.corpus {
            Corpus::Posix => "printf 'PROBE\\n'",
            Corpus::EchoOnly => "echo PROBE",
        };
        let mut shapes = vec![
            vec!["--no-rcs", "-i", "-c", probe],
            vec!["--no-rcs", "-l", "-c", probe],
        ];
        if !case.login_flag_is_exclusive {
            shapes.push(vec!["--no-rcs", "-i", "-l", "-c", probe]);
        }
        // `-f` is the NO_RCS spelling only under zsh's letter table.
        if !uses_bourne_option_letters(case) {
            shapes.push(vec!["-f", "-i", "-c", probe]);
        }
        for args in shapes {
            let mut argv: Vec<&str> = case.zshrs_flags.to_vec();
            argv.extend_from_slice(&args);
            let (out, code) = run(&zshrs_bin(), &argv, home.path(), &[], None);
            assert_eq!(
                String::from_utf8_lossy(&out),
                if case.corpus == Corpus::Posix {
                    "PROBE\n"
                } else {
                    "PROBE\n"
                },
                "{}: {argv:?} read a startup file despite --no-rcs",
                case.name
            );
            assert_eq!(code, 0, "{}: {argv:?} exited {code}", case.name);
        }
    }
}

/// True for the drop-ins whose `emulate` preset raises `SHOPTIONLETTERS`,
/// i.e. where the single-letter option table is the Bourne one and `-f`
/// therefore means `noglob` rather than `NO_RCS`.
fn uses_bourne_option_letters(case: &StartupCase) -> bool {
    !matches!(case.name, "zsh" | "csh")
}

/// The other half of the `-f` contract: in a Bourne-family drop-in it
/// disables globbing and leaves the startup files alone — the exact
/// behaviour of the reference shell, diffed against it.
#[test]
fn dash_f_is_noglob_in_the_bourne_drop_ins() {
    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut tested = 0usize;
    for case in STARTUP_CASES {
        if !uses_bourne_option_letters(case) || case.corpus != Corpus::Posix {
            continue;
        }
        let Some(refbin) = find_shell(case.candidates) else {
            if !case.optional {
                missing.push(case.name);
            }
            continue;
        };
        tested += 1;
        // No startup files at all: this leg is about the option letter, and
        // an unread rc must not colour the comparison.
        if let Some(m) = diff_shape(
            case,
            &refbin,
            "-f disables globbing",
            &[],
            &["-f", "-c", "printf '%s\\n' /etc/pas*"],
            &[],
            None,
        ) {
            mismatches.push(m);
        }
    }
    report(tested, &missing, &mismatches, "-f option-letter meaning");
}

/// bash `$PS1` backslash escapes, diffed against bash's own expansion.
///
/// `${PS1@P}` — bash(1) Parameter Expansion: "the expansion is a string
/// that is the result of expanding the value of parameter as if it were a
/// prompt string" — makes the prompt observable WITHOUT a terminal, so
/// this is a byte-for-byte differential rather than a screen-scrape.
///
/// `PS1=` is the one line essentially every `.bashrc` sets, and zsh's
/// prompt escapes are `%`-based, so an unhandled `\u` renders as literal
/// backslash text on a login shell's first line. Two escapes are excluded
/// on purpose: `\v` / `\V` report the bash version, which zshrs
/// deliberately pins to its own constant, and `\s` reports the basename
/// of `$0`, which differs because the two binaries have different names.
#[test]
fn bash_prompt_escapes_match_bash() {
    let case = &STARTUP_CASES[1];
    assert_eq!(case.name, "bash", "STARTUP_CASES[1] must stay the bash leg");
    let Some(refbin) = find_shell(case.candidates) else {
        eprintln!("skip: bash reference not found");
        return;
    };

    // Curated prompts: the common real-world shapes plus the edges that
    // are easy to get wrong (a literal `%`, `\$` vs zsh's `%#`, the
    // zero-width `\[`/`\]` regions, octal escapes, a trailing backslash).
    let mut prompts: Vec<String> = vec![
        r"\u@\h:\w\$ ".into(),
        r"[\u@\h \W]\$ ".into(),
        r"\w \$ ".into(),
        r"\W\$ ".into(),
        r"\H \W ".into(),
        r"100% done \$ ".into(),
        r"\[\e[32m\]\u\[\e[0m\]:\w\$ ".into(),
        r"\[\033[1;34m\]\w\[\033[0m\]\$ ".into(),
        r"\n\u \$ ".into(),
        r"\t \$ ".into(),
        r"\A \u ".into(),
        r"\D{%Y-%m-%d} \$ ".into(),
        r"\j jobs \$ ".into(),
        r"\\ \$ ".into(),
        r"\101\102 ".into(),
        r"plain text with no escapes ".into(),
        r"\q unknown escape ".into(),
        r"ends with a backslash \".into(),
    ];

    // Fuzz: shuffle the escape alphabet into prompts of random length, so
    // combinations nobody wrote by hand are covered too. Seeded, so a
    // failure replays from the printed seed.
    let seed: u64 = std::env::var("ZSHRS_STARTUP_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5052_4f4d_5054_5f31);
    let cases: usize = std::env::var("ZSHRS_STARTUP_FUZZ_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);
    const ALPHABET: &[&str] = &[
        r"\u", r"\h", r"\H", r"\w", r"\W", r"\$", r"\t", r"\T", r"\@", r"\A", r"\d", r"\j", r"\n",
        r"\\", r"\[", r"\]", "%", "-", ":", " ", "x", r"\D{%H}", r"\101",
    ];
    let mut rng = StdRng::seed_from_u64(seed);
    for _ in 0..cases {
        let n = rng.gen_range(1..8);
        let mut p = String::new();
        for _ in 0..n {
            p.push_str(ALPHABET[rng.gen_range(0..ALPHABET.len())]);
        }
        prompts.push(p);
    }

    let expand = |bin: &str, ps: &str| -> (Vec<u8>, i32) {
        let out = Command::new(bin)
            .args(["-c", r#"PS1=$1; printf %s "${PS1@P}""#, "_", ps])
            .env_clear()
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "dumb")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .expect("spawn");
        (out.stdout, out.status.code().unwrap_or(-1))
    };

    // zshrs needs its mode flag; otherwise the same shape.
    let expand_zshrs = |ps: &str| -> (Vec<u8>, i32) {
        let mut zargs: Vec<&str> = case.zshrs_flags.to_vec();
        zargs.push("-f");
        let out = Command::new(zshrs_bin())
            .args(&zargs)
            .args(["-c", r#"PS1=$1; printf %s "${PS1@P}""#, "_", ps])
            .env_clear()
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "dumb")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .expect("spawn");
        (out.stdout, out.status.code().unwrap_or(-1))
    };

    // A prompt carrying a clock escape cannot be compared instant-for-
    // instant: the two shells are separate processes, and under load the
    // spawns land seconds apart. For those prompts every ASCII DIGIT is
    // masked on both sides before comparing, so what is asserted is the
    // FORMAT — field count, separators, zero-padding, am/pm — rather than
    // the moment. That is still the property under test and it is not a
    // weaker one: masking `Sep 04` gives `Sep DD` while `Sep  4` gives
    // `Sep  D`, which is exactly the `\d` padding bug this caught. Every
    // other prompt is compared byte for byte.
    let clocks = ["\\t", "\\T", "\\@", "\\A", "\\d", "\\D"];
    let mask_digits = |b: &[u8]| -> Vec<u8> {
        b.iter()
            .map(|c| if c.is_ascii_digit() { b'D' } else { *c })
            .collect()
    };
    let mut mismatches = Vec::new();
    for ps in &prompts {
        let (r_out, r_code) = expand(&refbin, ps);
        let (z_out, z_code) = expand_zshrs(ps);
        let time_sensitive = clocks.iter().any(|e| ps.contains(e));
        let (r_cmp, z_cmp) = if time_sensitive {
            (mask_digits(&r_out), mask_digits(&z_out))
        } else {
            (r_out.clone(), z_out.clone())
        };
        if r_cmp != z_cmp || r_code != z_code {
            mismatches.push(format!(
                "  PS1={ps:?}{}\n    bash : code={r_code} {}\n    zshrs: code={z_code} {}",
                if time_sensitive {
                    " (digits masked — clock escape)"
                } else {
                    ""
                },
                show(&r_cmp),
                show(&z_cmp)
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "bash prompt expansion diverged on {} of {} prompt(s) (seed {seed:#x}):\n{}",
        mismatches.len(),
        prompts.len(),
        mismatches.join("\n")
    );
}

/// Serialises the two tests that mutate the process-global personality.
/// Every other test in this file drives zshrs as a SUBPROCESS, so only
/// these two can collide.
static PERSONALITY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Terminal sequences that belong to some shells and not others. Sending
/// one the emulated shell never sends is as visible as a wrong prompt —
/// it is what made a `--ksh` session announce bracketed paste that ksh93
/// has no concept of, and a `--bash` session emit an OSC 133 marker.
///
/// The table is the measured one: each shell was driven under a pty and
/// its output counted for `?2004h`. bash 5.3 → 1, zsh 5.9 → 1, ksh93 → 0,
/// mksh → 0, dash → 0, `/bin/sh` → 0, tcsh → 0.
///
/// These live here rather than beside the code because asserting them
/// means flipping the process-global personality, and the library test
/// binary runs its tests in parallel threads that share it.
#[test]
fn terminal_sequences_follow_the_emulated_shell() {
    let _g = PERSONALITY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use zsh::emulation_startup::{
        emits_bracketed_paste, emits_integration_prompt, personality, set_personality, Personality,
    };
    let saved = personality();
    for (p, paste, osc) in [
        (Personality::Zsh, true, true),
        (Personality::Bash, true, false),
        (Personality::Ksh93, false, false),
        (Personality::Mksh, false, false),
        (Personality::Pdksh, false, false),
        (Personality::Sh, false, false),
        (Personality::Dash, false, false),
        (Personality::Csh, false, false),
    ] {
        set_personality(p);
        assert_eq!(emits_bracketed_paste(), paste, "{p:?} bracketed paste");
        assert_eq!(emits_integration_prompt(), osc, "{p:?} OSC 133 marker");
    }
    set_personality(saved);
}

/// zsh's partial-line `%` marker (PROMPT_SP) is off for every drop-in and
/// untouched for native zsh. No Bourne-family shell prints one and it
/// lands in column 1 of the prompt line, so it is the first thing a user
/// of the drop-in would see.
#[test]
fn prompt_sp_is_off_for_every_drop_in() {
    let _g = PERSONALITY_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    use zsh::emulation_startup::{
        apply_personality_option_deltas, personality, set_personality, Personality,
    };
    use zsh::ported::options::{opt_state_get, opt_state_set};
    let saved = personality();
    for p in [
        Personality::Bash,
        Personality::Ksh93,
        Personality::Mksh,
        Personality::Pdksh,
        Personality::Sh,
        Personality::Dash,
        Personality::Csh,
    ] {
        opt_state_set("promptsp", true);
        set_personality(p);
        apply_personality_option_deltas();
        assert_eq!(
            opt_state_get("promptsp"),
            Some(false),
            "{p:?} must clear PROMPT_SP"
        );
    }
    // Native zsh keeps whatever it had.
    opt_state_set("promptsp", true);
    set_personality(Personality::Zsh);
    apply_personality_option_deltas();
    assert_eq!(
        opt_state_get("promptsp"),
        Some(true),
        "native zsh must keep PROMPT_SP"
    );
    set_personality(saved);
}

/// The default aliases a shell starts with, diffed against the reference.
///
/// These are startup STATE in the same sense the rc files are: they exist
/// before the first user command. zsh installs two (`run-help`,
/// `which-command`) and zshrs used to install them in every mode — so
/// `alias` under `--bash` listed two aliases bash does not have, and
/// under `--ksh` listed neither `r` nor `functions`, which ksh does.
/// Measured: bash 0, dash 0, `/bin/sh` 0, ksh93 19, mksh 11.
///
/// One documented exception, for the Korn drop-ins backed by mksh: mksh's
/// own defaults store TWO backslashes (`\\builtin typeset -fu`) and mksh
/// resolves that to its `builtin` builtin. zshrs resolves a ONE-backslash
/// command word and reports "command not found" for two, so its table
/// stores one — `type`, `functions`, `integer` and the rest actually run,
/// at the cost of one character per line in the listing. Working commands
/// beat a byte-perfect listing of broken ones. The comparison below
/// collapses backslash runs for those legs only, so the alias NAMES and
/// the rest of each body are still compared exactly.
#[test]
fn default_aliases_match_the_reference_shell() {
    let mut mismatches = Vec::new();
    let mut missing = Vec::new();
    let mut tested = 0usize;

    for case in STARTUP_CASES {
        // csh's `alias` output is a different format entirely and its
        // drop-in runs the zsh parser; zsh is the baseline, not a target.
        if case.corpus != Corpus::Posix || case.name == "zsh" {
            continue;
        }
        let Some(refbin) = find_shell(case.candidates) else {
            if !case.optional {
                missing.push(case.name);
            }
            continue;
        };
        tested += 1;

        // ksh93 is version-split: 93u+ 2012 (Apple's /bin/ksh) ships 19
        // default aliases, 93u+m 2024 (the maintained fork) ships none.
        // zshrs follows the maintained line, so a legacy reference is a
        // known version difference rather than a zshrs defect — say so
        // and skip, instead of asserting against a build we do not target.
        if case.name == "ksh" {
            let (v, _) = run(
                &refbin,
                &["-c", "echo ${KSH_VERSION:-}"],
                home_with(&[]).path(),
                &[],
                None,
            );
            let v = String::from_utf8_lossy(&v).into_owned();
            if v.contains("93u+ ") {
                eprintln!(
                    "  note: {refbin} is the legacy ksh93u+ 2012 line, which ships 19 \
                     default aliases the maintained 93u+m dropped; skipping this leg"
                );
                continue;
            }
        }

        let home = home_with(&[]);
        let mut zargs: Vec<&str> = case.zshrs_flags.to_vec();
        zargs.extend_from_slice(&["-f", "-c", "alias"]);
        let (z_out, _) = run(&zshrs_bin(), &zargs, home.path(), &[], None);
        let (r_out, _) = run(&refbin, &["-c", "alias"], home.path(), &[], None);

        // See the doc comment: the mksh-backed legs differ by backslash count.
        let collapse = matches!(case.name, "mksh" | "pdksh");
        let norm = |b: &[u8]| -> String {
            let t = String::from_utf8_lossy(b).into_owned();
            if collapse {
                t.replace("\\\\", "\\")
            } else {
                t
            }
        };
        let (z, r) = (norm(&z_out), norm(&r_out));
        if z != r {
            mismatches.push(format!(
                "  [{}] default aliases\n    ref({refbin}): {r:?}\n    zshrs:  {z:?}",
                case.name
            ));
        }
    }

    report(tested, &missing, &mismatches, "default alias set");
}

/// Shared reporting: a divergence fails, a missing REQUIRED reference fails
/// only under `ZSHRS_REQUIRE_REF_SHELLS`, and a run that tested nothing at
/// all always fails — a harness that silently covers zero legs is worse
/// than no harness.
fn report(tested: usize, missing: &[&str], mismatches: &[String], what: &str) {
    eprintln!("{what}: tested {tested} leg(s), {} missing", missing.len());
    assert!(
        mismatches.is_empty(),
        "{what} diverged on {} case(s):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    if std::env::var("ZSHRS_REQUIRE_REF_SHELLS").is_ok() && !missing.is_empty() {
        panic!(
            "ZSHRS_REQUIRE_REF_SHELLS is set but these references were absent: {missing:?}. \
             Install them so the {what} contract is enforced, not skipped."
        );
    }
    assert!(tested > 0, "{what}: no reference shells available at all");
}
