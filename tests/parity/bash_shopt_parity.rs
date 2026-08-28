//! `shopt` / `$BASHOPTS` parity for `zshrs --bash` against the REAL bash.
//!
//! !!! BASH IS THE SPEC HERE — NOT zsh's C source !!!
//! Every expectation in this file is produced by running the local `bash`
//! binary, so there are no hard-coded golden strings to go stale when bash
//! changes its default-on shopt set between releases.
//!
//! Scope: the `shopt` builtin's four query shapes (`shopt`, `-p`, `-q`,
//! `-o`), the `$BASHOPTS` parameter, and the four shopt options that have
//! real semantics in zshrs (`nullglob`, `failglob`, `nocasematch`,
//! `xpg_echo`).

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

/// A bash new enough to have `shopt`/`$BASHOPTS` at all (bash 4+). macOS
/// ships bash 3.2 at /bin/bash, which predates `globstar` and several rows
/// in the table, so prefer the Homebrew build and fall back to `$PATH`.
fn bash_path() -> Option<&'static str> {
    for p in ["/opt/homebrew/bin/bash", "/usr/local/bin/bash", "/bin/bash"] {
        if !Path::new(p).exists() {
            continue;
        }
        let ok = Command::new(p)
            .args(["-c", "shopt -q globstar; echo ${BASHOPTS+y}"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "y")
            .unwrap_or(false);
        if ok {
            return Some(p);
        }
    }
    None
}

struct R {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run(bin: &str, args: &[&str], script: &str) -> R {
    let mut c = Command::new(bin);
    c.args(args).arg(script).env_remove("ZSHRS_CACHE");
    let o = c.output().unwrap_or_else(|e| panic!("spawn {bin}: {e}"));
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_bash(bash: &str, script: &str) -> R {
    run(bash, &["-c"], script)
}

fn run_zshrs_bash(script: &str) -> R {
    run(zshrs_bin().to_str().unwrap(), &["--bash", "-c"], script)
}

/// stdout + exit-status parity against the real bash. stderr is compared
/// only for emptiness (bash prefixes diagnostics with its own argv[0] and
/// line number, which zshrs deliberately does not reproduce).
fn assert_bash_parity(bash: &str, script: &str) {
    let b = run_bash(bash, script);
    let z = run_zshrs_bash(script);
    assert_eq!(
        b.stdout, z.stdout,
        "stdout divergence on:\n  {script}\n--- bash ---\n{:?}\n--- zshrs ---\n{:?}",
        b.stdout, z.stdout
    );
    assert_eq!(
        b.exit, z.exit,
        "exit divergence on:\n  {script}\n  bash={} zshrs={}\n  bash stderr={:?}\n  zshrs stderr={:?}",
        b.exit, z.exit, b.stderr, z.stderr
    );
    assert_eq!(
        b.stderr.is_empty(),
        z.stderr.is_empty(),
        "stderr-emptiness divergence on:\n  {script}\n--- bash ---\n{:?}\n--- zshrs ---\n{:?}",
        b.stderr, z.stderr
    );
}

/// The 59 rows bash 5.3 accepts. Read from the LIVE bash so this list can
/// never drift from the binary under test.
fn shopt_names(bash: &str) -> Vec<String> {
    run_bash(bash, "shopt")
        .stdout
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------
// $BASHOPTS
// ---------------------------------------------------------------------

/// Every name bash accepts must round-trip through `$BASHOPTS`: `shopt -s`
/// puts it in the colon-separated list, `shopt -u` takes it out. This is the
/// exact probe shape `bins/parity-fuzz --matrix` emits.
#[test]
fn bashopts_membership_round_trips_for_every_shopt_name() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    let names = shopt_names(bash);
    assert!(names.len() > 40, "bash listed only {} shopts", names.len());
    for n in &names {
        // `login_shell` / `restricted_shell` are read-only STATE in bash:
        // `shopt -s` on them exits 0 and changes nothing. Probing them the
        // same way as the rest is deliberate — it pins that zshrs also
        // refuses the write instead of letting `$BASHOPTS` claim a login
        // shell in a non-login one.
        for verb in ["-s", "-u"] {
            let script = format!(
                "shopt {verb} {n} 2>/dev/null; \
                 case \":$BASHOPTS:\" in *:{n}:*) printf 'in\\n';; *) printf 'out\\n';; esac"
            );
            assert_bash_parity(bash, &script);
        }
    }
}

/// The pristine `$BASHOPTS` — bash's default-on set, colon-joined in the
/// table's alphabetical order.
#[test]
fn bashopts_default_value_matches_bash() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    assert_bash_parity(bash, "printf '%s\\n' \"$BASHOPTS\"");
}

/// Ordering is the table's, not insertion order: setting `globstar` (g) then
/// `autocd` (a) must still list `autocd` first.
#[test]
fn bashopts_ordering_is_table_order_not_insertion_order() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    assert_bash_parity(
        bash,
        "shopt -s globstar; shopt -s autocd; printf '%s\\n' \"$BASHOPTS\"",
    );
    assert_bash_parity(
        bash,
        "shopt -u checkwinsize; shopt -u progcomp; printf '%s\\n' \"$BASHOPTS\"",
    );
}

// ---------------------------------------------------------------------
// the `shopt` builtin's query shapes
// ---------------------------------------------------------------------

#[test]
fn shopt_query_shapes_match_bash() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    // Bare listing: NAME<TAB>on|off for all rows.
    assert_bash_parity(bash, "shopt");
    // -p: re-inputtable `shopt -s|-u NAME`.
    assert_bash_parity(bash, "shopt -p dotglob; shopt -s dotglob; shopt -p dotglob");
    // -q: status only, no output.
    assert_bash_parity(bash, "shopt -q dotglob; printf '%d\\n' $?");
    assert_bash_parity(
        bash,
        "shopt -s dotglob; shopt -q dotglob; printf '%d\\n' $?",
    );
    // -s / -u with no names list only the set / unset rows.
    assert_bash_parity(bash, "shopt -s");
    assert_bash_parity(bash, "shopt -u");
    // `--` ends flag parsing.
    assert_bash_parity(bash, "shopt -- dotglob; printf '%d\\n' $?");
    // Unknown name: diagnostic on stderr, status 1.
    assert_bash_parity(bash, "shopt zznope_xx; printf '%d\\n' $?");
    assert_bash_parity(bash, "shopt -s zznope_xx; printf '%d\\n' $?");
    assert_bash_parity(bash, "shopt -q zznope_xx; printf '%d\\n' $?");
    // Unknown FLAG: usage line, status 2 (not 1).
    assert_bash_parity(bash, "shopt -Z; printf '%d\\n' $?");
}

/// `shopt -o` is a second namespace with its own print shapes: `-p` emits
/// `set -o NAME` (the `set` builtin's spelling, not shopt's), and the
/// no-name listing pads to a different column width than a named query.
#[test]
fn shopt_dash_o_namespace_matches_bash() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    // Clustered short flags — `-so`, `-uo`, `-po`, `-pso` are all real.
    for script in [
        "shopt -o",
        "shopt -so",
        "shopt -uo",
        "shopt -po",
        "shopt -pso",
        "shopt -puo",
        "shopt -qo",
    ] {
        assert_bash_parity(bash, &format!("{script}; printf '%d\\n' $?"));
    }
    // Named queries: the listing and the query pad to different widths.
    assert_bash_parity(bash, "shopt -o errexit; printf '%d\\n' $?");
    assert_bash_parity(bash, "shopt -o braceexpand; printf '%d\\n' $?");
    assert_bash_parity(bash, "shopt -po errexit");
    assert_bash_parity(bash, "shopt -po braceexpand");
    // Shared state with `set -o`: one flag, two spellings.
    assert_bash_parity(bash, "shopt -so errexit; shopt -o errexit");
    assert_bash_parity(bash, "set -o errexit; shopt -o errexit");
    assert_bash_parity(bash, "shopt -so errexit; set +o | grep -c 'set -o errexit'");
    // Unknown `-o` name.
    assert_bash_parity(bash, "shopt -o zznope_xx; printf '%d\\n' $?");
}

// ---------------------------------------------------------------------
// options with REAL semantics
// ---------------------------------------------------------------------

#[test]
fn nullglob_and_failglob_apply() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    assert_bash_parity(
        bash,
        "shopt -s nullglob; printf '[%s]\\n' ./nonexistent_zz*; printf '%d\\n' $?",
    );
    // Neither set: the unmatched pattern survives literally.
    assert_bash_parity(
        bash,
        "printf '[%s]\\n' ./nonexistent_zz*; printf '%d\\n' $?",
    );
    // failglob: the command is not executed and the shell reports failure.
    assert_bash_parity(
        bash,
        "shopt -s failglob; printf '[%s]\\n' ./nonexistent_zz* 2>/dev/null; printf '%d\\n' $?",
    );
}

/// bash(1): "nocasematch — If set, bash matches patterns in a
/// case-insensitive fashion when performing matching while executing case or
/// [[ conditional commands, when performing pattern substitution word
/// expansions, or when filtering possible completions".
#[test]
fn nocasematch_applies_to_case_dbracket_and_regex() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    // `case` — the shape the fuzzer found.
    assert_bash_parity(
        bash,
        "shopt -s nocasematch; case ABC in abc) printf 'y\\n';; *) printf 'n\\n';; esac",
    );
    // …and OFF must stay case-SENSITIVE (guards against a blanket lowercase).
    assert_bash_parity(
        bash,
        "case ABC in abc) printf 'y\\n';; *) printf 'n\\n';; esac",
    );
    assert_bash_parity(
        bash,
        "shopt -s nocasematch; shopt -u nocasematch; \
         case ABC in abc) printf 'y\\n';; *) printf 'n\\n';; esac",
    );
    // Glob metacharacters must keep working through the case-fold.
    assert_bash_parity(
        bash,
        "shopt -s nocasematch; case ABCdef in ab*F) printf 'y\\n';; *) printf 'n\\n';; esac",
    );
    assert_bash_parity(
        bash,
        "shopt -s nocasematch; case ABC in [a-c][b-d][a-d]) printf 'y\\n';; *) printf 'n\\n';; esac",
    );
    // `[[ == ]]` and `[[ != ]]`.
    assert_bash_parity(
        bash,
        "shopt -s nocasematch; [[ ABC == abc ]] && printf 'y\\n' || printf 'n\\n'",
    );
    assert_bash_parity(
        bash,
        "shopt -s nocasematch; [[ ABC != abc ]] && printf 'y\\n' || printf 'n\\n'",
    );
    // `[[ =~ ]]` POSIX ERE.
    assert_bash_parity(
        bash,
        "shopt -s nocasematch; [[ ABC =~ ^abc$ ]] && printf 'y\\n' || printf 'n\\n'",
    );
    assert_bash_parity(
        bash,
        "[[ ABC =~ ^abc$ ]] && printf 'y\\n' || printf 'n\\n'",
    );
    assert_bash_parity(
        bash,
        "shopt -s nocasematch; shopt -u nocasematch; \
         [[ ABC =~ ^abc$ ]] && printf 'y\\n' || printf 'n\\n'",
    );
}

/// bash(1): "xpg_echo — If set, the echo builtin expands backslash-escape
/// sequences by default." Equivalent to zsh's NO_BSD_ECHO.
#[test]
fn xpg_echo_turns_on_echo_escape_expansion() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    for esc in ["a\\tb", "x\\ny", "q\\c", "p\\\\q", "u\\e[0m", "n\\0101"] {
        assert_bash_parity(
            bash,
            &format!("shopt -s xpg_echo; echo '{esc}'; printf 'END\\n'"),
        );
        // OFF (bash's default) keeps them literal.
        assert_bash_parity(bash, &format!("echo '{esc}'; printf 'END\\n'"));
        // `-e` forces expansion regardless; `-E` forces literal regardless.
        assert_bash_parity(
            bash,
            &format!("shopt -s xpg_echo; echo -e '{esc}'; printf 'END\\n'"),
        );
        assert_bash_parity(
            bash,
            &format!("shopt -s xpg_echo; echo -E '{esc}'; printf 'END\\n'"),
        );
    }
    // Toggling back off restores the literal default.
    assert_bash_parity(
        bash,
        "shopt -s xpg_echo; shopt -u xpg_echo; echo 'a\\tb'; printf 'END\\n'",
    );
    // `printf` is unaffected by xpg_echo either way.
    assert_bash_parity(bash, "shopt -s xpg_echo; printf '%s\\n' 'a\\tb'");
}

/// The mapped-to-zsh rows that already carry real behavior. Pins them so a
/// future remap of the `BASH_SHOPTS` middle column cannot silently break
/// them.
#[test]
fn mapped_shopts_keep_their_semantics() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4 with $BASHOPTS found");
        return;
    };
    let dir = std::env::temp_dir().join("zshrs_shopt_parity");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join(".hidden"), "").unwrap();
    std::fs::write(dir.join("vis"), "").unwrap();
    std::fs::write(dir.join("sub").join("deep"), "").unwrap();
    let d = dir.display();

    // dotglob ~ GLOB_DOTS
    assert_bash_parity(bash, &format!("shopt -s dotglob; printf '%s\\n' {d}/*"));
    assert_bash_parity(bash, &format!("printf '%s\\n' {d}/*"));
    // globstar
    assert_bash_parity(
        bash,
        &format!("shopt -s globstar; printf '%s\\n' {d}/**/deep"),
    );
    // nocaseglob
    assert_bash_parity(
        bash,
        &format!("shopt -s nocaseglob; printf '%s\\n' {d}/VIS"),
    );
    // cdable_vars
    assert_bash_parity(
        bash,
        &format!("shopt -s cdable_vars; v={d}; cd v >/dev/null && pwd"),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------
// negative control: --zsh must not gain any of this
// ---------------------------------------------------------------------

fn zsh_path() -> Option<&'static str> {
    for p in ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh"] {
        if Path::new(p).exists() {
            return Some(p);
        }
    }
    None
}

/// `$BASHOPTS` is a bash personality parameter. Under `--zsh` it must stay
/// an ordinary unset name, exactly as in real zsh, no matter what the bash
/// side does with it.
#[test]
fn zsh_mode_has_no_bashopts() {
    let Some(zsh) = zsh_path() else {
        eprintln!("SKIP: no zsh found");
        return;
    };
    for script in [
        "printf '%s\\n' \"${BASHOPTS-UNSET}\"",
        "BASHOPTS=mine; printf '%s\\n' \"$BASHOPTS\"",
        "printf '%s\\n' \"${BASHOPTS:-plain}\"",
    ] {
        let z = run(zsh, &["-fc"], script);
        let r = run(
            zshrs_bin().to_str().unwrap(),
            &["--zsh", "-f", "-c"],
            script,
        );
        assert_eq!(
            z.stdout, r.stdout,
            "--zsh regression on:\n  {script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
            z.stdout, r.stdout
        );
        assert_eq!(z.exit, r.exit, "--zsh exit regression on:\n  {script}");
    }
}

/// zsh's own glob/echo options must be untouched by the bash mapping work:
/// `NULL_GLOB`, `NO_MATCH`, `CASE_MATCH` and `BSD_ECHO` still behave as zsh
/// does under `--zsh`.
#[test]
fn zsh_mode_glob_and_echo_options_unchanged() {
    let Some(zsh) = zsh_path() else {
        eprintln!("SKIP: no zsh found");
        return;
    };
    for script in [
        "setopt nullglob; printf '[%s]\\n' ./nonexistent_zz*; printf '%d\\n' $?",
        "setopt nonomatch; printf '[%s]\\n' ./nonexistent_zz*; printf '%d\\n' $?",
        "echo 'a\\tb'",
        "setopt bsdecho; echo 'a\\tb'",
        "echo -E 'a\\tb'",
        "setopt nocasematch; [[ ABC =~ ^abc$ ]] && printf 'y\\n' || printf 'n\\n'",
        "[[ ABC =~ ^abc$ ]] && printf 'y\\n' || printf 'n\\n'",
        "case ABC in abc) printf 'y\\n';; *) printf 'n\\n';; esac",
        "setopt nocasematch; case ABC in abc) printf 'y\\n';; *) printf 'n\\n';; esac",
    ] {
        let z = run(zsh, &["-fc"], script);
        let r = run(
            zshrs_bin().to_str().unwrap(),
            &["--zsh", "-f", "-c"],
            script,
        );
        assert_eq!(
            z.stdout, r.stdout,
            "--zsh regression on:\n  {script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
            z.stdout, r.stdout
        );
        assert_eq!(z.exit, r.exit, "--zsh exit regression on:\n  {script}");
    }
}
