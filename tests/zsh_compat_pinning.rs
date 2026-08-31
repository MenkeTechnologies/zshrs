//! Pin `zsh -fc` against `zshrs --zsh -f -c`: default shell state,
//! CLI-linked flags (`$-`), and small language surfaces that must stay
//! aligned in parity mode.
//!
//! zshrs uses `-f` on our side of the harness so we match zsh's `-fc`
//! (no RC files) and avoid user `.zshenv` stdout leaking into captured
//! output — the same observable surface zsh's parity tests target.

use std::path::PathBuf;
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
    use std::path::Path;
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

struct ShellResult {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

/// Match [`run_zsh`]: `-f` suppresses RC noise; `--zsh` enables parity mode.
fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn run_zshrs_compat(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh-compat", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs --zsh-compat");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stdout, r.stdout
    );
    assert_eq!(
        z.stderr, r.stderr,
        "stderr divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stderr, r.stderr
    );
    assert_eq!(
        z.exit, r.exit,
        "exit divergence on script:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

fn assert_parity_with_trailing_args(script: &str, trailing: &[&str]) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = {
        let mut c = Command::new(zsh_path());
        c.arg("-fc").arg(script);
        for a in trailing {
            c.arg(a);
        }
        let out = c.output().expect("invoke zsh");
        ShellResult {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            exit: out.status.code().unwrap_or(-1),
        }
    };
    let r = {
        let mut c = Command::new(zshrs_bin());
        c.args(["--zsh", "-f", "-c", script])
            .env_remove("ZSHRS_CACHE");
        for a in trailing {
            c.arg(a);
        }
        let out = c.output().expect("invoke zshrs");
        ShellResult {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            exit: out.status.code().unwrap_or(-1),
        }
    };
    assert_eq!(
        z.stdout, r.stdout,
        "script={} trailing={trailing:?}",
        script
    );
    assert_eq!(z.stderr, r.stderr);
    assert_eq!(z.exit, r.exit);
}

mod shell_flags_dollar_minus {
    use super::*;

    #[test]
    fn default_plus_f_matches_zsh_fc() {
        assert_parity("echo $-");
    }

    #[test]
    fn after_set_errtrace_e() {
        assert_parity("set -e; echo $-");
    }

    #[test]
    fn after_set_xtrace_x_stderr_and_stdout() {
        // xtrace: PS4 + command line on stderr, `echo` result on stdout.
        assert_parity("set -x; echo $-");
    }
}

mod identity_and_defaults {
    use super::*;

    #[test]
    fn zsh_name_is_zsh() {
        assert_parity("echo $ZSH_NAME");
    }

    #[test]
    fn shlvl_integer_echo() {
        assert_parity("echo $SHLVL");
    }

    #[test]
    fn wordchars_default() {
        assert_parity("echo $WORDCHARS");
    }

    #[test]
    fn histchars_default() {
        assert_parity("echo $histchars");
    }

    #[test]
    fn optind_before_getopts() {
        assert_parity("echo $OPTIND");
    }
}

mod posix_c_positionals {
    use super::*;

    #[test]
    fn explicit_zero_and_rest() {
        assert_parity_with_trailing_args(r#"echo "$0-$1-$2-$3""#, &["nom", "aa", "bb", "cc"]);
    }

    #[test]
    fn star_joins_ifs_first_field() {
        assert_parity(r#"set -- p q; printf '[%s]\n' "$*""#);
    }

    #[test]
    fn at_preserves_separate_words() {
        assert_parity(r#"set -- p q; printf '[%s]\n' "$@""#);
    }

    #[test]
    fn argv_array_matches_positionals() {
        // `$argv` as a scalar joins with spaces; `"${argv[@]}"` pins the
        // array enumeration path (matches `print -l` on `$*`).
        assert_parity(
            r#"set -- x y; print -l -- "${argv[@]}"
echo "len=${#argv}""#,
        );
    }
}

mod conditionals_and_arith_cond {
    use super::*;

    #[test]
    fn double_bracket_pattern_glob() {
        assert_parity(r#"[[ xyz == *z ]] && echo glob"#);
    }

    #[test]
    fn double_bracket_string_equality() {
        assert_parity(r#"[[ a = a ]] && echo eq"#);
    }

    #[test]
    fn double_bracket_n_empty() {
        assert_parity(r#"[[ -n $PWD ]] && echo haspwd"#);
    }

    #[test]
    fn arith_cond_numeric() {
        assert_parity(r#"(( 7 > 3 )) && echo t"#);
    }
}

mod small_language_surface {
    use super::*;

    #[test]
    fn brace_expansion_commas() {
        assert_parity(r#"echo {a,b,c}"#);
    }

    #[test]
    fn arithmetic_power() {
        assert_parity(r#"echo $(( 2 ** 3 ))"#);
    }

    #[test]
    fn array_one_indexed_first_el() {
        assert_parity(r#"a=(u v w); echo $a[1]"#);
    }

    #[test]
    fn print_r_no_brand_mangling() {
        assert_parity(r#"print -r -- '-n--'"#);
    }
}

mod zsh_compat_cli_alias {
    use super::*;

    /// `--zsh-compat` must run the same parity surface as `--zsh`.
    #[test]
    fn matches_zsh_flag_for_script() {
        if !zsh_available() {
            return;
        }
        let script = r#"print -r -- "${${(q-)IFS}:0:3}""#;
        let z = run_zsh(script);
        let a = run_zshrs(script);
        let b = run_zshrs_compat(script);
        assert_eq!(z.stdout, a.stdout);
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(z.exit, a.exit);
        assert_eq!(a.exit, b.exit);
    }
}

// ════════════════════════════════════════════════════════════════════
// Regression pins for the 2026-08-30 parity fixes. Each script below
// was verified by hand against `zsh -fc` before being pinned here; the
// absolute assertions keep the pin meaningful on a CI box with no zsh
// installed (where `assert_parity` skips).
// ════════════════════════════════════════════════════════════════════

/// A subscripted assignment must report the exit status of a command
/// substitution on its RHS, exactly like a scalar assignment does.
///
/// Fix: src/extensions/compile_zsh.rs — `compile_assign`'s
/// BUILTIN_SET_SUBSCRIPT_RANGE and BUILTIN_SET_ASSOC arms returned
/// without recording `last_assign_had_cmd_subst`, so
/// BUILTIN_ASSIGN_ONLY_STATUS took its `else { 0 }` branch.
/// c:Src/exec.c:3396 `lastval = cmdoutval`.
#[test]
fn assign_subscript_reports_cmdsubst_exit_status() {
    let script = r#"typeset -gA h; typeset -ga a
h[k]="$(false)"; print "assoc_false=$?"
h[k]="$(true)";  print "assoc_true=$?"
a[2]="$(false)"; print "array_false=$?"
s="$(false)";    print "scalar_false=$?""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "assoc_false=1\nassoc_true=0\narray_false=1\nscalar_false=1\n",
        "subscripted assignment must propagate cmdoutval like a scalar one"
    );
    assert_parity(script);
}

/// The shape that made this user-visible: VCS_INFO_detect_git uses the
/// assignment AS the condition of an `&&` chain. Outside a repository
/// `rev-parse` fails, so zsh takes the false branch and never runs the
/// git backend. Reporting success left `gitdir` empty and
/// VCS_INFO_git_getbranch read `$gitdir/HEAD` as `/HEAD`.
#[test]
fn assign_subscript_as_condition_fails_when_cmdsubst_fails() {
    let script = r#"typeset -gA vcs_comm
if vcs_comm[gitdir]="$(false)"; then
  print "SUCCESS gitdir=[${vcs_comm[gitdir]}]"
else
  print "FAIL gitdir=[${vcs_comm[gitdir]}]"
fi"#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "FAIL gitdir=[]\n",
        "a failed $() in a subscripted assignment must fail the && chain"
    );
    assert_parity(script);
}

/// `${~...}` inside a `:=` / `::=` word must not leave GLOB_SUBST on
/// for the enclosing expansion. C keeps `globsubst` paramsubst-LOCAL
/// (c:Src/subst.c:1669) and clears it after the word unless the OUTER
/// spec forced it (c:3231-3232 `if (globsubst != 2) globsubst = 0;`).
///
/// Fix: src/ported/subst.rs — the assign arms now snapshot/restore
/// GLOB_SUBST + TILDE_GLOBSUBST_CARRIER around the word expansion.
/// Leaking it filename-globbed the result, so an unterminated bracket
/// became a fatal `bad pattern`. fast-syntax-highlighting runs
/// `: ${expanded_path::=${~_mybuf}}` on every keystroke.
#[test]
fn tilde_glob_inside_assign_word_does_not_leak() {
    for pat in ["[[", "foo[", "[a", "a(b", "*.txt", "["] {
        for op in ["::=", ":="] {
            let script = format!(
                "b={}\n: ${{e{}${{~b}}}}\nprint -r -- \"[$e]\"",
                shell_quote(pat),
                op
            );
            let r = run_zshrs(&script);
            assert_eq!(
                r.stdout,
                format!("[{}]\n", pat),
                "`{}` with b={:?} must assign the literal, not glob it",
                op,
                pat
            );
            assert!(
                !r.stderr.contains("bad pattern"),
                "no `bad pattern` for b={:?} op={} (stderr {:?})",
                pat,
                op,
                r.stderr
            );
            assert_parity(&script);
        }
    }
}

/// The other half of c:3231-3232: a `~` on the OUTER spec is "forced"
/// (globsubst == 2) and MUST still glob. Guards against fixing the
/// leak by disabling the flag outright.
#[test]
fn tilde_glob_on_outer_assign_spec_still_globs() {
    let script = r#"d=$(mktemp -d) || exit 1
cd "$d" || exit 1
: > f1.txt; : > f2.txt
b='*.txt'
print -r -- ${~e::=$b}
cd /; command rm -rf "$d""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "f1.txt f2.txt\n",
        "an outer `~` on the assign spec stays forced and still globs"
    );
    assert_parity(script);
}

/// `(V)` (render non-printing chars visible, c:Src/subst.c:2232) must
/// apply to a SUBSCRIPTED element, not just to a whole array or scalar.
///
/// Fix: src/extensions/compile_zsh.rs folded `(V)` into the same
/// "redundant flag" predicate as `(v)` and compiled `${(V)a[1]}` to a
/// bare BUILTIN_ARRAY_INDEX, dropping the flag; and src/ported/subst.rs
/// gates both `(V)` arms on `subscript.is_none()` so the arm that
/// re-fetches the array by name cannot discard the element selection.
#[test]
fn v_flag_applies_to_subscripted_element() {
    let script = r#"a=($'x\ny' $'p\tq')
typeset -A h; h[k]=$'x\ny'
s=$'x\ny'
print -r -- "elem1=${(V)a[1]}"
print -r -- "elem2=${(V)a[2]}"
print -r -- "neg=${(V)a[-1]}"
print -r -- "range=${(V)a[1,2]}"
print -r -- "assoc=${(V)h[k]}"
print -r -- "whole=${(V)a}"
print -r -- "scalar=${(V)s}""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout,
        "elem1=x\\ny\nelem2=p\\tq\nneg=p\\tq\nrange=x\\ny p\\tq\nassoc=x\\ny\nwhole=x\\ny p\\tq\nscalar=x\\ny\n",
        "(V) must escape non-printing chars through a subscript too"
    );
    assert_parity(script);
}

/// The folds that must KEEP working: `(v)` really is redundant with a
/// simple subscript (it asks for an assoc element's value), and `(k)`
/// yields the key. Pins that the `(V)` fix did not disable them.
#[test]
fn lowercase_v_and_k_subscript_folds_still_apply() {
    let script = r#"typeset -A h; h[k]=$'x\ny'; h[j]=plain
print -r -- "v=${(v)h[k]}"
print -r -- "k=${(k)h[j]}""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "v=x\ny\nk=j\n",
        "(v) stays a value fetch (raw newline) and (k) stays a key fetch"
    );
    assert_parity(script);
}

/// execcmd_exec's c:3315-3318 sweep isolates the head word, globs it,
/// then re-merges ahead of the tail. The merge moved from a per-element
/// `Vec::insert` (O(K*n) memmove) to a single `splice(0..0, ..)`; this
/// pins the ordering contract that made the swap safe — the globbed
/// head lands first, in match order, and the tail args keep their order.
///
/// Fix: src/ported/exec.rs.
#[test]
fn globbed_command_word_keeps_head_then_tail_order() {
    let script = r#"d=$(mktemp -d) || exit 1
cd "$d" || exit 1
cat > runme <<'SH'
#!/bin/sh
for a in "$@"; do printf '%s|' "$a"; done; echo
SH
chmod 755 runme
./run*e alpha beta gamma
cd /; command rm -rf "$d""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "alpha|beta|gamma|\n",
        "head glob resolves to the command and tail args keep order"
    );
    assert_parity(script);
}

/// Single-quote a string for embedding in a shell script.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// With `FPATH` absent, `fpath` leads with the bundled tree and carries no
/// host zsh *distribution* directory.
///
/// zsh falls back to paths fixed at compile time by `configure`
/// (c:Src/init.c:1132-1143): `SITEFPATH_DIR` and
/// `<prefix>/share/zsh/<version>/functions`. zshrs keeps the first and
/// drops the second, because it carries zsh's own function tree
/// (`vendor/zsh` -> `~/.zshrs/functions`). Keeping a host copy on top of
/// the bundle means two differently-versioned `compinit`s racing for the
/// same lookup, and the winner changes when Homebrew upgrades zsh.
///
/// The earlier bug this still guards: `env::var("FPATH")` +
/// `unwrap_or_default()` left fpath EMPTY, so nothing autoloaded --
///   zsh: is-at-least: function definition file not found
/// reachable from any `exec zshrs`, since zsh does not export FPATH.
#[test]
fn fpath_default_leads_with_bundle_and_has_no_distribution_tree() {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "print -l $fpath"])
        .env_remove("FPATH")
        .env_remove("fpath")
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let entries: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        entries.last().is_some_and(|e| e.ends_with("/.zshrs/functions")),
        "the bundled tree must be LAST on fpath, got {:?}",
        entries
    );
    assert!(
        !entries.iter().any(|e| e.ends_with("/share/zsh/functions")
            || (e.contains("/share/zsh/") && e.ends_with("/functions"))),
        "no host zsh distribution tree may be seeded, got {:?}",
        entries
    );
}

/// An INTERACTIVE shell has a populated `fpath`, not an empty one.
///
/// c:Src/params.c:893-988 — `createparamtable` imports `environ` and, for
/// an IPDEF8 PM_TIED colon-array, installs BOTH sides: the scalar and the
/// array split on ':'. zshrs re-seeds the specials from the static table
/// in `setupvals`, which leaves each tied array empty, and nothing split
/// the scalar back in. An interactive shell therefore reached its first
/// prompt with
///     typeset -aT FPATH fpath=(  )
/// while `$FPATH` still held the full value.
///
/// `path` masked it -- `PATH` is always exported, so a later env import
/// refilled it -- and `-c` never runs that path at all, so every
/// non-interactive probe passed. The damage was not a missing default: a
/// `.zshrc` doing the standard `fpath=( mydir $fpath )` appended to
/// NOTHING, leaving the shell with only what the rc file added.
///
/// Driven through a pty because the bug does not exist without one.
#[test]
fn interactive_shell_has_a_populated_fpath() {
    use std::io::{Read, Write};
    use std::os::unix::io::FromRawFd;

    let mut master: libc::c_int = 0;
    // `termp`/`winp` are `*mut` on macOS and `*const` on Linux; naming the
    // pointee keeps one source compiling for both (a `*mut T` coerces to
    // `*const T`, never the reverse).
    let termp = std::ptr::null_mut::<libc::termios>();
    let winp = std::ptr::null_mut::<libc::winsize>();
    let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), termp, winp) };
    assert!(pid >= 0, "forkpty failed");
    if pid == 0 {
        // Child: an interactive shell with no rc files and no FPATH, which
        // is exactly how a terminal launches one (zsh does not export it).
        unsafe {
            libc::setenv(c"TERM".as_ptr(), c"dumb".as_ptr(), 1);
            libc::unsetenv(c"FPATH".as_ptr());
        }
        let bin = std::ffi::CString::new(zshrs_bin().to_string_lossy().as_ref()).unwrap();
        let arg = std::ffi::CString::new("-f").unwrap();
        unsafe {
            libc::execl(bin.as_ptr(), bin.as_ptr(), arg.as_ptr(), std::ptr::null::<libc::c_char>());
            libc::_exit(127);
        }
    }

    let mut f = unsafe { std::fs::File::from_raw_fd(master) };
    // Let it reach a prompt, then ask.
    std::thread::sleep(std::time::Duration::from_millis(2500));
    let _ = f.write_all(b"print \"NFPATH=$#fpath\"\nexit\n");
    let _ = f.flush();

    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&out).contains("NFPATH=") && out.len() > 32 {
                    // keep reading briefly so the full number lands
                }
            }
            Err(_) => break,
        }
    }
    unsafe {
        let mut st = 0;
        libc::waitpid(pid, &mut st, 0);
    }
    let text = String::from_utf8_lossy(&out).replace('\r', "");
    // The pty echoes the typed line, so the FIRST "NFPATH=" is the literal
    // `NFPATH=$#fpath` being typed. Take the last one that is followed by
    // digits -- that is the shell's own answer.
    let n: usize = text
        .split("NFPATH=")
        .skip(1)
        .filter_map(|rest| {
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<usize>().ok()
        })
        .last()
        .unwrap_or_else(|| panic!("no NFPATH=<n> in interactive output: {text:?}"));
    assert!(
        n > 0,
        "an interactive shell must have a non-empty fpath, got {n}; \
         a .zshrc doing `fpath=( dir $fpath )` would keep only its own entry"
    );
}

/// A host zsh distribution tree is dropped from an INHERITED `FPATH`,
/// while `site-functions` and user/plugin directories survive.
///
/// `exec zshrs` from a running zsh inherits that shell's fpath, which on
/// any Homebrew box names `/opt/homebrew/Cellar/zsh/<ver>/share/zsh/functions`
/// -- a second copy of every function the bundle already carries, able to
/// shadow it. `share/zsh/site-functions` is the opposite case: nothing in
/// it comes from zsh, it is where formulae install `_brew`, `_docker`,
/// `_kubectl`, and the bundle has no copy of those, so dropping it would
/// silently disable completion for package-manager-installed tools.
///
/// Both rejected shapes are covered: Homebrew's Cellar layout has no
/// version component under `share/zsh`, the FHS layout does.
#[test]
fn inherited_fpath_drops_distribution_tree_but_keeps_site_functions() {
    // Every layout zshrs runs on. macOS: Homebrew's Cellar (no version
    // component under share/zsh) and its opt prefix. Linux: the FHS
    // versioned tree, the unversioned one Debian ships, and Homebrew's
    // Linux prefix -- which is /home/linuxbrew/.linuxbrew, NOT
    // /opt/homebrew.
    let inherited = [
        "/opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions",
        "/usr/share/zsh/5.9/functions",
        "/usr/share/zsh/functions",
        "/home/linuxbrew/.linuxbrew/Cellar/zsh/5.9/share/zsh/functions",
        "/opt/homebrew/share/zsh/site-functions",
        "/usr/local/share/zsh/site-functions",
        "/usr/share/zsh/site-functions",
        "/home/linuxbrew/.linuxbrew/share/zsh/site-functions",
        "/tmp/zshrs-pin-plugin/src",
        "/tmp/zshrs-pin-zinit/completions",
    ];
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "print -l $fpath"])
        .env("FPATH", inherited.join(":"))
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let entries: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    for drop in [
        "/opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions",
        "/usr/share/zsh/5.9/functions",
        "/usr/share/zsh/functions",
        "/home/linuxbrew/.linuxbrew/Cellar/zsh/5.9/share/zsh/functions",
    ] {
        assert!(
            !entries.contains(&drop),
            "{drop} duplicates the bundled tree and must be dropped, got {entries:?}"
        );
    }
    for keep in [
        "/opt/homebrew/share/zsh/site-functions",
        "/usr/local/share/zsh/site-functions",
        "/usr/share/zsh/site-functions",
        "/home/linuxbrew/.linuxbrew/share/zsh/site-functions",
        "/tmp/zshrs-pin-plugin/src",
        "/tmp/zshrs-pin-zinit/completions",
    ] {
        assert!(
            entries.contains(&keep),
            "{keep} carries third-party completions and must survive, got {entries:?}"
        );
    }
    // The bundle goes LAST, so it supplies only what nothing else does.
    // Leading it shadowed every curated completion of the same name --
    // 242 of zsh-more-completions' files on the author's setup, `_ls`
    // among them. Nothing is lost by trailing: the only tree that could
    // out-rank it is a host zsh's own, which is filtered off fpath.
    assert!(
        entries.last().is_some_and(|e| e.ends_with("/.zshrs/functions")),
        "the bundled tree must trail, got {:?}",
        entries
    );
    let bundle_at = entries
        .iter()
        .position(|e| e.ends_with("/.zshrs/functions"))
        .expect("bundle on fpath");
    for user_dir in ["/tmp/zshrs-pin-plugin/src", "/tmp/zshrs-pin-zinit/completions"] {
        let at = entries.iter().position(|e| *e == user_dir).expect(user_dir);
        assert!(
            at < bundle_at,
            "{user_dir} must out-rank the bundle so a curated function wins, got {entries:?}"
        );
    }
}

/// c:Src/lex.c:523-527 — `cmd_or_math`'s unget loop is
/// `while (lexbuf.len > oldlen && !(errflag & ERRFLAG_ERROR))`, and
/// C's `lexbuf.len--` decrements unconditionally.
///
/// The port dropped the errflag term and relied on `pop()` yielding a
/// char, so a buffer reporting length above `oldlen` that popped
/// nothing spun forever. `(( $+functions[a[b] ))` -- an unbalanced `[`
/// in a math subscript, which zsh rejects with "invalid subscript" --
/// hung the LEXER, so any file containing one could never be parsed.
/// That is real generated code: `_uu-coreutils` in
/// zsh-more-completions defines `_uu-coreutils__[_commands` for
/// coreutils' `[` utility, and it stalled `--prewarm-autoloads` (and
/// therefore `zshrs-recorder`) indefinitely.
///
/// Polls instead of blocking so a regression fails the suite rather
/// than hanging it.
#[test]
fn math_subscript_with_nested_bracket_terminates() {
    use std::time::{Duration, Instant};
    let mut child = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "(( $+functions[a[b] ))"])
        .env_remove("ZSHRS_CACHE")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn zshrs");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("`(( $+functions[a[b] ))` did not terminate in 30s -- cmd_or_math spun");
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// zshrs bundles zsh's function tree and materialises it into
/// `~/.zshrs/functions` on first run, then puts that directory first on
/// `fpath`. zsh gets the equivalent from a configure-baked
/// `<prefix>/share/zsh/<version>/functions`; zshrs is not installed under
/// a zsh prefix, so without this a shell started with no `FPATH` could
/// not autoload anything -- `exec zshrs` printed
/// "is-at-least: function definition file not found" and two more.
///
/// Runs against a throwaway HOME so it never touches the user's tree.
#[test]
fn bundled_functions_materialise_and_resolve() {
    let tmp = std::env::temp_dir().join(format!("zshrs-bundle-pin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("mkdir temp HOME");

    let run = |script: &str| -> String {
        let out = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", script])
            .env("HOME", &tmp)
            .env_remove("FPATH")
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("invoke zshrs");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    // First run materialises; the three functions below are exactly the
    // ones whose absence broke `exec zshrs`.
    let got = run("autoload -Uz is-at-least colors add-zsh-hook && print RESOLVED");
    assert!(
        got.contains("RESOLVED"),
        "bundled functions must autoload with FPATH unset, got {:?}",
        got
    );

    let dir = tmp.join(".zshrs").join("functions");
    let n = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
    assert!(
        n > 1000,
        "expected the full tree in {}, found {} entries",
        dir.display(),
        n
    );
    // FLAT, matching zsh's own install layout.
    let subdirs = std::fs::read_dir(&dir)
        .map(|d| d.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).count())
        .unwrap_or(0);
    assert_eq!(subdirs, 0, "layout must be flat like zsh's functions dir");
    // compsys leaves have to be there too, not just Misc helpers.
    for f in ["_git", "_describe", "_arguments", "compinit"] {
        assert!(dir.join(f).is_file(), "{f} missing from the bundled tree");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// zsh's man and info pages ship inside the binary and land in
/// `~/.zshrs/{man,info}`, with those directories published on `MANPATH`
/// and `INFOPATH`.
///
/// zsh's `make install` puts the pages under `<prefix>/share/{man,info}`,
/// which `man` and `info` already search. zshrs is not installed under a
/// zsh prefix, so `man zshall` worked only on a host that happened to have
/// zsh installed -- while the pages document the language zshrs itself
/// implements.
///
/// Two failure modes this pins, both of which shipped:
///   1. the pages were written to `~/.zshrs/man1` while `MANPATH` named
///      `~/.zshrs/man`, so nothing resolved;
///   2. an INHERITED `MANPATH`/`INFOPATH` kept its old value in the shell,
///      because paramtab is built from the process-entry `environ`
///      snapshot and a later `setenv` cannot reach it.
///
/// Runs against a throwaway HOME so it never touches the user's tree.
#[test]
fn bundled_docs_materialise_and_publish_search_paths() {
    let tmp = std::env::temp_dir().join(format!("zshrs-docs-pin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("mkdir temp HOME");

    let run = |script: &str, manpath: Option<&str>| -> String {
        let mut cmd = Command::new(zshrs_bin());
        cmd.args(["--zsh", "-f", "-c", script])
            .env("HOME", &tmp)
            .env_remove("ZSHRS_CACHE")
            .env_remove("INFOPATH");
        match manpath {
            Some(v) => cmd.env("MANPATH", v),
            None => cmd.env_remove("MANPATH"),
        };
        let out = cmd.output().expect("invoke zshrs");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };

    // First run materialises. The section directory must be BENEATH the
    // MANPATH entry -- `man` resolves a page as <manpath>/man1/<name>.1.
    let man1 = tmp.join(".zshrs").join("man").join("man1");
    let info = tmp.join(".zshrs").join("info");
    let manpath = run("print $MANPATH", None);
    for page in ["zsh.1", "zshall.1", "zshbuiltins.1", "zshexpn.1", "zshmisc.1"] {
        assert!(
            man1.join(page).is_file(),
            "{page} missing from {}",
            man1.display()
        );
    }
    for f in ["zsh.info", "zsh.info-1"] {
        assert!(
            info.join(f).is_file(),
            "{f} missing from {}",
            info.display()
        );
    }

    // With MANPATH unset, ours leads and the trailing empty entry keeps
    // the system default path spliced in -- a bare replacement would hide
    // every other man page on the machine.
    let want = tmp.join(".zshrs").join("man").display().to_string();
    assert_eq!(
        manpath,
        format!("{want}:"),
        "MANPATH must be our dir plus the default-path marker"
    );
    let infopath = run("print $INFOPATH", None);
    assert_eq!(
        infopath,
        format!("{}:", info.display()),
        "INFOPATH must be our dir plus the default-path marker"
    );

    // An inherited value is PREPENDED to, never replaced, and the shell
    // must report the new value -- not the frozen entry-environ one.
    let got = run("print $MANPATH", Some("/tmp/zshrs-pin-manpath"));
    assert_eq!(
        got,
        format!("{want}:/tmp/zshrs-pin-manpath"),
        "inherited MANPATH must survive behind ours"
    );

    // Re-entering must not stack a second copy.
    let twice = run("print $MANPATH", Some(&format!("{want}:/tmp/zshrs-pin-manpath")));
    assert_eq!(
        twice,
        format!("{want}:/tmp/zshrs-pin-manpath"),
        "an already-published MANPATH must not be prepended again"
    );

    // `run-help`'s database. The vendored `run-help` defaults HELPDIR to
    // the <prefix>/share/zsh/<ver>/help of whichever zsh built it -- a
    // path that does not exist on a host without that exact install --
    // so the shell has to point HELPDIR at the bundled tree itself.
    let help = tmp.join(".zshrs").join("help");
    for topic in ["zmodload", "bindkey", "autoload", "setopt"] {
        assert!(
            help.join(topic).is_file(),
            "{topic} missing from {}",
            help.display()
        );
    }
    assert_eq!(
        run("print $HELPDIR", None),
        help.display().to_string(),
        "HELPDIR must name the bundled help tree"
    );
    // `newuser`, which zsh's first-run path sources.
    assert!(
        tmp.join(".zshrs").join("scripts").join("newuser").is_file(),
        "the scripts tree must materialise too"
    );

    // A user-chosen HELPDIR always wins: the shell fills an empty slot,
    // it does not override.
    let mine = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "print $HELPDIR"])
        .env("HOME", &tmp)
        .env("HELPDIR", "/tmp/zshrs-pin-helpdir")
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    assert_eq!(
        String::from_utf8_lossy(&mine.stdout).trim(),
        "/tmp/zshrs-pin-helpdir",
        "an explicit HELPDIR must not be overwritten"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// `add-zsh-hook` accepts zshrs's own `async_precmd` hook.
///
/// `async_precmd` functions run on a POOL WORKER THREAD instead of
/// blocking the prompt (src/extensions/async_precmd.rs). zsh has no such
/// hook, so upstream's `add-zsh-hook` carries a fixed `hooktypes` list
/// that rejects the name:
///
///   add-zsh-hook async_precmd my_fn
///   Usage: add-zsh-hook hook function
///   Valid hooks are:
///     chpwd precmd preexec periodic zshaddhistory zshexit ...
///
/// -- which left the hook reachable only by assigning
/// `async_precmd_functions` by hand. zshrs ships an override in
/// `functions/`, walked ahead of `vendor/zsh/functions` by build.rs so a
/// re-sync of the vendored tree cannot revert it.
///
/// The rest of the function must keep working: the six upstream hooks
/// still register, an unknown name is still refused, and `-d` / `-L`
/// still operate on the new hook's array.
#[test]
fn add_zsh_hook_accepts_the_async_precmd_hook() {
    let run = |script: &str| -> (String, String) {
        let out = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", script])
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("invoke zshrs");
        (
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )
    };

    let (out, err) = run(
        "autoload -Uz add-zsh-hook; f(){ }; add-zsh-hook async_precmd f          && print \"list=$async_precmd_functions\"",
    );
    assert_eq!(
        out, "list=f",
        "async_precmd must register; stderr was {err:?}"
    );

    // zsh's own hooks are untouched.
    let (out, err) = run(
        "autoload -Uz add-zsh-hook; g(){ };          for h in chpwd precmd preexec periodic zshaddhistory zshexit; do            add-zsh-hook $h g || print BAD=$h; done; print OK",
    );
    assert_eq!(out, "OK", "upstream hooks must still register; {err:?}");

    // An unknown hook is still an error -- the list was extended, not
    // replaced by a blanket accept.
    let (out, _) = run("autoload -Uz add-zsh-hook; h(){ }; add-zsh-hook nonesuch h; print rc=$?");
    assert!(
        out.contains("rc=1"),
        "an unknown hook name must still be refused, got {out:?}"
    );

    // -L lists it and -d removes it.
    let (out, _) = run(
        "autoload -Uz add-zsh-hook; f(){ }; add-zsh-hook async_precmd f;          add-zsh-hook -L async_precmd",
    );
    assert!(
        out.contains("async_precmd_functions") && out.contains(" f "),
        "-L must show the registered hook, got {out:?}"
    );
    let (out, _) = run(
        "autoload -Uz add-zsh-hook; f(){ }; add-zsh-hook async_precmd f;          add-zsh-hook -d async_precmd f; print \"[$async_precmd_functions]\"",
    );
    assert_eq!(out, "[]", "-d must remove the hook again");
}

/// A host zsh installation's function tree is refused even when the
/// user's own config puts it back.
///
/// The startup filter only sees the INHERITED `FPATH`. A `.zshrc` that
/// re-adds `<prefix>/share/zsh/<ver>/functions`, or a plugin manager
/// restoring a saved fpath, then put a foreign zsh's `add-zsh-hook`,
/// `compinit` and `_git` back ahead of the bundled copies -- observed as
///     add-zsh-hook is a shell function from
///     /opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions/add-zsh-hook
/// which meant zshrs's own override, the one that knows `async_precmd`,
/// never ran. The rule has to hold on assignment.
///
/// `site-functions` is NOT a distribution tree and must survive: it is
/// where third-party formulae install completions.
#[test]
fn assigning_fpath_refuses_a_host_zsh_function_tree() {
    let out = Command::new(zshrs_bin())
        .args([
            "--zsh",
            "-f",
            "-c",
            "fpath=( /opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions \
                     /usr/share/zsh/5.9/functions /usr/share/zsh/functions \
                     /opt/homebrew/share/zsh/site-functions /tmp/zshrs-pin-assign )\nprint -l $fpath",
        ])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let got: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    for drop in [
        "/opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions",
        "/usr/share/zsh/5.9/functions",
        "/usr/share/zsh/functions",
    ] {
        assert!(
            !got.iter().any(|e| e == drop),
            "{drop} must not survive an fpath assignment, got {got:?}"
        );
    }
    for keep in ["/opt/homebrew/share/zsh/site-functions", "/tmp/zshrs-pin-assign"] {
        assert!(got.iter().any(|e| e == keep), "{keep} must survive, got {got:?}");
    }
}

/// The bundled tree is on `fpath` whether or not `FPATH` was inherited.
///
/// `setupvals` re-seeds the tied specials after the constructor ran. With
/// FPATH UNSET the array came back empty and a split refilled it; with
/// FPATH SET the env import refilled it non-empty, the split was skipped,
/// and the bundle -- appended by the constructor to the param that had
/// just been overwritten -- was gone. So `~/.zshrs/functions` was present
/// with FPATH unset and missing with FPATH set.
#[test]
fn inherited_fpath_still_carries_the_bundled_tree() {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", "print -l $fpath"])
        .env("FPATH", "/tmp/zshrs-pin-inherit-a:/tmp/zshrs-pin-inherit-b")
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let entries: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect();
    assert!(
        entries.iter().any(|e| e.ends_with("/.zshrs/functions")),
        "the bundled tree must survive an inherited FPATH, got {entries:?}"
    );
    for inherited in ["/tmp/zshrs-pin-inherit-a", "/tmp/zshrs-pin-inherit-b"] {
        assert!(
            entries.iter().any(|e| e == inherited),
            "{inherited} must be kept, got {entries:?}"
        );
    }
}
