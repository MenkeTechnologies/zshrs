//! `${parameter@operator}` and the array case-modification operators for
//! `zshrs --bash`, checked against the REAL bash.
//!
//! !!! BASH IS THE SPEC HERE — NOT zsh's C source !!!
//! `${v@Q}`, `${v@A}`, `${v@K}`, `${v@k}`, `${v@a}`, `${v@E}`, `${v@U}` and
//! `${a[@]^^}` do not exist in zsh at all — `zsh -f` answers "bad
//! substitution" for every one of them — so there is no `Src/subst.c` line to
//! port and no zsh output to compare against. Expectations are therefore
//! produced by running the local bash binary, never hard-coded, so the file
//! cannot go stale against a future bash.
//!
//! The second half is the anti-regression half: the same scripts run under
//! `zsh -f` and `zshrs --zsh -f` must stay byte-identical (i.e. both must
//! still say "bad substitution"), because everything above is gated on
//! `crate::dash_mode::bash_mode()`.

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

/// A bash new enough to have `${var@Q}` at all — the transformation syntax
/// arrived in bash 4.4, so macOS's /bin/bash 3.2 is not usable. Probe for the
/// feature rather than parsing `$BASH_VERSION`.
fn bash_path() -> Option<&'static str> {
    for p in ["/opt/homebrew/bin/bash", "/usr/local/bin/bash", "/bin/bash"] {
        if !Path::new(p).exists() {
            continue;
        }
        let ok = Command::new(p)
            .args(["-c", r#"v=x; printf '%s' "${v@Q}""#])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout) == "'x'")
            .unwrap_or(false);
        if ok {
            return Some(p);
        }
    }
    None
}

fn zsh_path() -> Option<&'static str> {
    for p in ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh"] {
        if Path::new(p).exists() {
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

/// stdout + exit-status parity between the real bash and `zshrs --bash`.
fn assert_bash_parity(bash: &str, script: &str) {
    let b = run(bash, &["-c"], script);
    let z = run(zshrs_bin().to_str().unwrap(), &["--bash", "-c"], script);
    assert_eq!(
        b.stdout, z.stdout,
        "stdout divergence on:\n  {script}\n--- bash ---\n{:?}\n--- zshrs --bash ---\n{:?}",
        b.stdout, z.stdout
    );
    assert_eq!(
        b.exit, z.exit,
        "exit divergence on:\n  {script}\n  bash={} zshrs={}\n  zshrs stderr={:?}",
        b.exit, z.exit, z.stderr
    );
}

/// The negative control: the SAME script under `zsh -f` and `zshrs --zsh -f`.
/// Every operator this file exercises is bash-only, so both sides must agree
/// — in practice both reject it — and the bash-mode work must not move zsh
/// mode by a single byte.
fn assert_zsh_unchanged(zsh: &str, script: &str) {
    let z = run(zsh, &["-f", "-c"], script);
    let r = run(zshrs_bin().to_str().unwrap(), &["--zsh", "-f", "-c"], script);
    assert_eq!(
        z.stdout, r.stdout,
        "zsh-mode stdout REGRESSION on:\n  {script}\n--- zsh ---\n{:?}\n--- zshrs --zsh ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(
        z.exit, r.exit,
        "zsh-mode exit REGRESSION on:\n  {script}\n  zsh={} zshrs={}",
        z.exit, r.exit
    );
    assert_eq!(
        z.stderr.contains("bad substitution"),
        r.stderr.contains("bad substitution"),
        "zsh-mode diagnostic REGRESSION on:\n  {script}\n--- zsh ---\n{:?}\n--- zshrs --zsh ---\n{:?}",
        z.stderr, r.stderr
    );
}

/// Every script this file checks. `printf "<%s>"` makes the WORD COUNT
/// visible, which is load-bearing: `"${a[@]@A}"` is three words in bash while
/// `"${a[*]@A}"` is one, and `"${a[@]@K}"` is one however it is subscripted.
const SCRIPTS: &[&str] = &[
    // --- the exact shapes bins/parity-fuzz reported ---
    r#"a=(x 'y z'); printf "<%s>" "${a[*]@A}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[*]@K}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[*]@Q}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[@]@A}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[@]@K}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[@]@Q}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[@]@U}"; echo"#,
    r#"a=(a b c); printf "<%s>" "${a[@]^^}"; echo"#,
    r#"v="MixEd"; printf "<%s>" "${v@A}"; echo"#,
    r#"v="a b"; printf "<%s>" "${v@A}"; echo"#,
    r#"v="a b"; printf "<%s>" "${v@K}"; echo"#,
    r#"v="a\tb"; printf "<%s>" "${v@A}"; echo"#,
    r#"v="a\tb"; printf "<%s>" "${v@K}"; echo"#,
    r#"v="abc"; printf "<%s>" "${v@A}"; echo"#,
    r#"v="abc"; printf "<%s>" "${v@K}"; echo"#,
    r#"v="it's"; printf "<%s>" "${v@K}"; echo"#,
    // --- element-wise application across the rest of the operator set ---
    r#"a=(a b c); printf "<%s>" "${a[*]^^}"; echo"#,
    r#"a=(AB CD); printf "<%s>" "${a[@],,}" "${a[*],,}"; echo"#,
    r#"a=(ab cd); printf "<%s>" "${a[@]^}" "${a[@],}"; echo"#,
    r#"a=(ab cd); printf "<%s>" "${a[@]~~}" "${a[@]~}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[@]@L}" "${a[@]@u}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[*]@U}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[@]@E}" "${a[*]@E}"; echo"#,
    r#"a=('a\tb' c); printf "<%s>" "${a[@]@E}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[@]@k}" "${a[*]@k}"; echo"#,
    r#"a=(x 'y z'); printf "<%s>" "${a[@]@a}" "${a[*]@a}"; echo"#,
    // --- @A / @a carry the parameter's ATTRIBUTES ---
    r#"declare -i n=5; printf "<%s>" "${n@A}" "${n@a}"; echo"#,
    r#"declare -x e=hi; printf "<%s>" "${e@A}" "${e@a}"; echo"#,
    r#"declare -r r=hi; printf "<%s>" "${r@A}" "${r@a}"; echo"#,
    r#"v=plain; printf "[%s]" "${v@a}"; echo"#,
    // --- associative arrays ---
    r#"declare -A h=([k1]=v1); printf "<%s>" "${h[@]@A}"; echo"#,
    r#"declare -A h=([k1]=v1); printf "<%s>" "${h[*]@A}"; echo"#,
    r#"declare -A h=([k1]=v1); printf "<%s>" "${h[@]@K}" "${h[*]@K}"; echo"#,
    r#"declare -A h=([k1]=v1); printf "<%s>" "${h[@]@k}" "${h[@]@a}"; echo"#,
    // A key holding a shell metacharacter must be double-quoted; one holding
    // only `-` / `.` / `_` must stay bare (bash's sh_contains_shell_metas).
    r#"declare -A h=(["k 1"]=v); printf "<%s>" "${h[*]@K}" "${h[*]@A}"; echo"#,
    r#"declare -A h=([a-b.c_d]=v); printf "<%s>" "${h[*]@K}" "${h[*]@A}"; echo"#,
    // --- empty and sparse arrays ---
    r#"a=(); printf "<%s>" "${a[@]@A}" "${a[@]@K}" "${a[@]@k}" "${a[@]@Q}"; echo"#,
    r#"a=(); printf "<%s>" "${a[*]@A}" "${a[*]@K}"; echo"#,
    r#"a=(x); a[5]=z; printf "<%s>" "${a[*]@A}" "${a[*]@K}"; echo"#,
    // --- a bare array name is element 0, an [N] subscript reports under the
    //     PARAMETER's name and attributes ---
    r#"a=(x y); printf "<%s>" "${a@Q}" "${a@A}" "${a@K}" "${a@U}"; echo"#,
    r#"a=(x y); printf "<%s>" "${a[0]@A}" "${a[1]@Q}"; echo"#,
    // --- quoting: @Q switches to $'…' only for unprintable bytes ---
    r#"v=$'a\tb'; printf "<%s>" "${v@Q}" "${v@A}" "${v@K}"; echo"#,
    r#"v=$'a\nb'; printf "<%s>" "${v@Q}"; echo"#,
    r#"v=$'\a\v\b\f\n\r\t\e\001\177'; printf "<%s>" "${v@Q}"; echo"#,
    r#"v=""; printf "<%s>" "${v@Q}" "${v@A}" "${v@K}"; echo"#,
    r#"v="héllo"; printf "<%s>" "${v@Q}" "${v@A}"; echo"#,
    r#"v='a"b'; printf "<%s>" "${v@Q}" "${v@A}"; echo"#,
    // Array ELEMENTS use double quotes inside `name=(…)` / `@K`, not single.
    r#"a=('a"b' 'c\d' 'e$f' 'g`h'); printf "<%s>" "${a[*]@A}"; echo"#,
    r#"a=('a"b' 'c\d' 'e$f' 'g`h'); printf "<%s>" "${a[*]@K}"; echo"#,
    r#"a=($'p\tq' r); printf "<%s>" "${a[*]@A}" "${a[*]@K}"; echo"#,
    // --- `${a[@]@A}` word splitting is IFS-driven, with quoted AND
    //     parenthesised regions protected ---
    r#"a=(x 'y z'); IFS=":"; printf "<%s>" "${a[@]@A}"; echo"#,
    r#"a=(x 'y z'); IFS="a"; printf "<%s>" "${a[@]@A}"; echo"#,
    r#"a=(x y); IFS="0"; printf "<%s>" "${a[@]@A}"; echo"#,
    r#"a=(x y); IFS="["; printf "<%s>" "${a[@]@A}"; echo"#,
    r#"a=(x 'y z'); IFS=""; printf "<%s>" "${a[@]@A}"; echo"#,
    // `[*]` joins on $IFS[0] AFTER the per-element transform.
    r#"a=(x 'y z'); IFS=":"; printf "<%s>" "${a[*]@Q}" "${a[*]@U}"; echo"#,
    // --- the scalar operators that already worked, kept as a floor ---
    r#"v=abc; printf "<%s>" "${v@Q}" "${v@U}" "${v@L}" "${v@u}" "${v@E}"; echo"#,
];

// ---------------------------------------------------------------------
// bash parity
// ---------------------------------------------------------------------

/// Every script above, byte-for-byte against the real bash under `--bash`.
#[test]
fn bash_param_transformations_match_real_bash() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4.4 (needs ${{v@Q}}) found");
        return;
    };
    for s in SCRIPTS {
        assert_bash_parity(bash, s);
    }
}

/// `${a[@]@A}` is THREE words and `${a[*]@A}` is ONE — the shape of the
/// expansion, not just its text, is part of the contract. Spelled out
/// separately from the table so a regression names itself.
#[test]
fn at_A_word_count_differs_between_at_and_star_subscripts() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4.4 found");
        return;
    };
    let count = |bin: &str, args: &[&str], sub: &str| -> usize {
        let script = format!(r#"a=(x 'y z'); set -- "${{a[{sub}]@A}}"; printf '%s\n' "$#""#);
        run(bin, args, &script).stdout.trim().parse().unwrap_or(0)
    };
    let zb = zshrs_bin();
    let z = zb.to_str().unwrap();
    assert_eq!(count(bash, &["-c"], "@"), 3, "bash: a[@] is three words");
    assert_eq!(count(bash, &["-c"], "*"), 1, "bash: a[*] is one word");
    assert_eq!(count(z, &["--bash", "-c"], "@"), 3);
    assert_eq!(count(z, &["--bash", "-c"], "*"), 1);
}

/// `@K` stays ONE word under both `[@]` and `[*]`, unlike `@A`. This is the
/// asymmetry that made a "just split it like @A" implementation wrong.
#[test]
fn at_K_is_one_word_under_both_subscripts() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4.4 found");
        return;
    };
    let count = |bin: &str, args: &[&str], sub: &str| -> usize {
        let script = format!(r#"a=(x 'y z'); set -- "${{a[{sub}]@K}}"; printf '%s\n' "$#""#);
        run(bin, args, &script).stdout.trim().parse().unwrap_or(0)
    };
    let zb = zshrs_bin();
    let z = zb.to_str().unwrap();
    for sub in ["@", "*"] {
        assert_eq!(count(bash, &["-c"], sub), 1, "bash: ${{a[{sub}]@K}}");
        assert_eq!(count(z, &["--bash", "-c"], sub), 1, "zshrs: ${{a[{sub}]@K}}");
    }
}

/// An unknown operator letter is still a hard substitution error in bash, and
/// must stay one in `--bash` — the new `@A`/`@K`/`@k` arms must not have
/// turned the reject into a silent empty expansion.
#[test]
fn unknown_at_operator_is_still_rejected() {
    let Some(bash) = bash_path() else {
        eprintln!("SKIP: no bash >= 4.4 found");
        return;
    };
    for op in ["@Z", "@X", "@"] {
        let script = format!(r#"v=x; printf '<%s>' "${{v{op}}}"; printf 'after'"#);
        let b = run(bash, &["-c"], &script);
        let z = run(zshrs_bin().to_str().unwrap(), &["--bash", "-c"], &script);
        assert!(
            !b.status_ok(),
            "bash unexpectedly accepted ${{v{op}}}: {:?}",
            b.stdout
        );
        assert!(
            !z.status_ok(),
            "zshrs --bash accepted ${{v{op}}} but bash rejects it: {:?}",
            z.stdout
        );
        assert_eq!(
            b.stdout.contains("after"),
            z.stdout.contains("after"),
            "abort-vs-continue divergence on ${{v{op}}}"
        );
    }
}

impl R {
    fn status_ok(&self) -> bool {
        self.exit == 0
    }
}

// ---------------------------------------------------------------------
// zsh-mode negative controls — the whole point of the bash_mode() gate
// ---------------------------------------------------------------------

/// Not one of the bash operators may leak into `--zsh`: every script above
/// has to produce the same bytes and the same status under `zsh -f` and
/// `zshrs --zsh -f`.
#[test]
fn zsh_mode_is_unchanged_by_every_bash_transformation() {
    let Some(zsh) = zsh_path() else {
        eprintln!("SKIP: no zsh found");
        return;
    };
    for s in SCRIPTS {
        assert_zsh_unchanged(zsh, s);
    }
}

/// zsh's OWN spellings of the same ideas must keep working untouched:
/// `${(qq)a}` for quoting, `${(U)a}` / `${a:u}` for case, and `(j:X:)` for a
/// separator-controlled join.
#[test]
fn zsh_native_quote_and_case_flags_still_match_zsh() {
    let Some(zsh) = zsh_path() else {
        eprintln!("SKIP: no zsh found");
        return;
    };
    for s in [
        r#"a=(x 'y z'); printf "<%s>" "${(qq)a[@]}"; echo"#,
        r#"a=(x 'y z'); printf "<%s>" "${(q)a[@]}"; echo"#,
        r#"a=(x 'y z'); printf "<%s>" "${(U)a[@]}" "${(L)a[@]}"; echo"#,
        r#"v=hello; printf "<%s>" "${v:u}" "${(C)v}"; echo"#,
        r#"a=(a b); print -r -- "${(j:-:)a}"; print -rl -- "${(@j:-:)a}""#,
        r#"a=(x 'y z'); IFS=:; print -r -- "${a[*]}""#,
        r#"typeset -A m=(k v); printf "<%s>" "${(kv)m}"; echo"#,
    ] {
        assert_zsh_unchanged(zsh, s);
    }
}

// ---------------------------------------------------------------------
// documented gaps
// ---------------------------------------------------------------------

/// GAP: `${v@P}` (expand the value as a PROMPT string) is not implemented.
/// bash's prompt escapes (`\u`, `\h`, `\w`, `\!`) are a different language
/// from zsh's `%n` / `%m` / `%~` / `%!`, and zshrs has no bash prompt
/// expander, so `--bash` still rejects `@P` the way zsh does rather than
/// guess at a translation.
#[test]
#[ignore = "documented gap: bash @P needs a bash prompt-escape expander"]
fn at_P_expands_the_value_as_a_prompt_string() {
    let Some(bash) = bash_path() else {
        return;
    };
    assert_bash_parity(bash, r#"v='\u'; printf "<%s>" "${v@P}"; echo"#);
}

/// GAP: the POSITIONAL parameters take a different fetch path in `subst.rs`
/// than a named array, so `"${@@Q}"` and friends never reach the
/// transformation arm and expand unquoted.
///
/// ```text
/// $ bash            -c 'set -- "a b" c; printf "<%s>" "${@@Q}"'  → <'a b'><'c'>
/// $ zshrs --bash    -c 'set -- "a b" c; printf "<%s>" "${@@Q}"'  → <a b><c>
/// ```
#[test]
#[ignore = "documented gap: $@ / $* bypass the @-transformation arm"]
fn positional_parameters_take_at_transformations() {
    let Some(bash) = bash_path() else {
        return;
    };
    for s in [
        r#"set -- "a b" c; printf "<%s>" "${@@Q}"; echo"#,
        r#"set -- "a b" c; printf "<%s>" "${*@Q}"; echo"#,
        r#"set -- "a b" c; printf "<%s>" "${@@U}"; echo"#,
        r#"set -- "a b" c; printf "<%s>" "${@@A}"; echo"#,
    ] {
        assert_bash_parity(bash, s);
    }
}

/// GAP: `declare -a` combined with another attribute letter is rejected by
/// the `declare` builtin ("inconsistent type for assignment"), which is a
/// typeset-side limitation, not a transformation one — `${z[@]@a}` cannot be
/// checked until the declaration itself works.
///
/// ```text
/// $ bash         -c 'declare -air z=(1 2); echo hi'  → hi
/// $ zshrs --bash -c 'declare -air z=(1 2); echo hi'
///       zshrs:declare:1: z: inconsistent type for assignment
/// ```
#[test]
#[ignore = "documented gap: `declare -a` + another attribute letter is rejected"]
fn multi_attribute_array_declaration_reports_its_letters() {
    let Some(bash) = bash_path() else {
        return;
    };
    assert_bash_parity(bash, r#"declare -air z=(1 2); printf "<%s>" "${z[@]@a}"; echo"#);
}
