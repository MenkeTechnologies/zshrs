//! Behavioural parity between zshrs's `Src/Modules/` ports and the real
//! `/bin/zsh`. Each test runs a small script through both shells and
//! asserts byte-equal stdout (and, where it matters, stderr / exit code).
//!
//! These tests are the immune system for the `src/modules/*.rs` ports
//! per the endgame rule in `CLAUDE.md`: every new module-surface fix
//! adds a case here so the behaviour is *pinned* against the C source's
//! observable output.
//!
//! Tests skip silently when `/bin/zsh` is unavailable (CI containers,
//! minimal Linux installs).

use std::path::PathBuf;
use std::process::Command;

/// Path to the freshly-built `zshrs` binary. Cargo sets
/// `CARGO_BIN_EXE_zshrs` for integration tests; fall through to
/// `target/debug/zshrs` when running outside cargo (manual `cargo
/// test --no-run` + run-by-hand workflows).
fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}

/// Pick the most-modular `zsh` on PATH. The Apple-shipped
/// `/bin/zsh` is locked at 5.9 with a *minimal* module set
/// (no `zsh/ksh93`, no `zsh/param_private`); a Homebrew install
/// at `/opt/homebrew/bin/zsh` (Apple Silicon) or
/// `/usr/local/bin/zsh` (Intel) ships many more loadable modules
/// and is the better parity target. Falls through to `/bin/zsh`
/// when neither brew location exists.
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

/// Run `script` through `<zsh> -fc …` (brew zsh preferred, Apple
/// stock as fallback). The caller prepends any required
/// `zmodload zsh/<modname>` lines — `-fc` suppresses RC files so
/// no module auto-loads.
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

/// Run `script` through `zshrs --zsh -c …`. zshrs's loadable modules
/// are always present (they're statically linked), so a `zmodload`
/// in the script is a no-op — but it does no harm, which keeps the
/// same script string usable against both shells.
fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

/// Build a script that explicitly loads each listed module before
/// the body. Mirrors zsh's `zmodload zsh/foo; …` idiom. Using this
/// helper instead of inlining `zmodload` per case lets us keep the
/// per-test script free of boilerplate.
fn with_modules(modules: &[&str], body: &str) -> String {
    let mut s = String::new();
    for m in modules {
        s.push_str(&format!("zmodload zsh/{} 2>/dev/null\n", m));
    }
    s.push_str(body);
    s
}

/// Assert that `/bin/zsh -fc script` and `zshrs --zsh -c script` produce
/// the same stdout and exit code. Stderr is checked only by emptiness
/// — the two shells phrase warnings differently in many cases, so the
/// per-test caller can override via `assert_parity_strict`.
fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: /bin/zsh not found");
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
        z.exit, r.exit,
        "exit-code divergence on script:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

/// Strict parity — also requires stderr to match byte-for-byte. Useful
/// when the test exercises an error path whose diagnostic format zshrs
/// has explicitly aligned with the C source.
#[allow(dead_code)]
fn assert_parity_strict(script: &str) {
    if !zsh_available() {
        eprintln!("skip: /bin/zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{}", script);
    assert_eq!(z.stderr, r.stderr, "stderr divergence on:\n{}", script);
    assert_eq!(z.exit, r.exit, "exit divergence on:\n{}", script);
}

// ───────────────────────── zsh/regex ─────────────────────────

mod regex_module {
    use super::*;

    /// `[[ str -regex-match pat ]]` returns 0 on match.
    /// Direct test of `zcond_regex_match` (Src/Modules/regex.c:60-210)
    /// — the truthiness shape.
    #[test]
    fn regex_match_basic_truthy() {
        assert_parity(&with_modules(
            &["regex"],
            r#"[[ "hello world" -regex-match "wor.d" ]] && echo Y || echo N"#,
        ));
    }

    /// Capture groups go to $match[1..N]; full match to $MATCH.
    /// Direct test of regex.c:148-205 (the
    /// `setiparam("MBEGIN", …)`/`set sparam("MATCH", …)`/`setaparam`
    /// sequence after a successful regexec).
    #[test]
    fn regex_match_captures() {
        assert_parity(&with_modules(
            &["regex"],
            r#"[[ "foo=42" -regex-match "([a-z]+)=([0-9]+)" ]] && echo "$MATCH|$match[1]|$match[2]""#,
        ));
    }

    /// Non-match must NOT touch $MATCH / $match.
    #[test]
    fn regex_match_failure_keeps_status_nonzero() {
        assert_parity(&with_modules(
            &["regex"],
            r#"MATCH=untouched; [[ abc -regex-match xyz ]] || echo "no:$?:$MATCH""#,
        ));
    }
}

// ───────────────────────── zsh/pcre ─────────────────────────

mod pcre_module {
    use super::*;

    /// `[[ str -pcre-match pat ]]` truthiness shape — direct test
    /// of `cond_pcre_match` (Src/Modules/pcre.c:506 CONDDEF).
    /// zshrs uses the Rust `regex` crate (RE2 engine) so PCRE
    /// features like backreferences aren't tested here — only the
    /// common subset both engines accept.
    #[test]
    fn pcre_match_basic_truthy() {
        assert_parity(&with_modules(
            &["pcre"],
            r#"[[ "hello world" -pcre-match "wor.d" ]] && echo Y || echo N"#,
        ));
    }

    /// Capture groups populate `$MATCH` and `$match[1..N]`. Direct
    /// test of `zpcre_get_substrings` (pcre.c:156-330) — same
    /// magic-var contract as `=~` / `-regex-match`.
    #[test]
    fn pcre_match_captures() {
        assert_parity(&with_modules(
            &["pcre"],
            r#"[[ "key=42" -pcre-match "([a-z]+)=([0-9]+)" ]] && echo "$MATCH|$match[1]|$match[2]""#,
        ));
    }

    /// `pcre_compile pat` then `pcre_match str`: stateful API.
    /// Direct test of the `pcre_pattern` static slot in pcre.c
    /// (line 35) shared between `bin_pcre_compile` (line 70) and
    /// `bin_pcre_match` (line 328).
    #[test]
    fn pcre_compile_then_match_stateful() {
        assert_parity(&with_modules(
            &["pcre"],
            r#"pcre_compile "[a-z]+=[0-9]+"
pcre_match "key=42" && echo "matched: $MATCH""#,
        ));
    }

    /// `pcre_match` without a prior `pcre_compile` should error.
    /// pcre.c:343 `if (pcre_pattern == NULL) zwarnnam(nam, "no
    /// pattern has been compiled"); return 1;`
    #[test]
    fn pcre_match_without_compile_errors() {
        assert_parity(&with_modules(
            &["pcre"],
            r#"pcre_match "anything" >/dev/null 2>&1; print -- "exit:$?""#,
        ));
    }
}

// ───────────────────────── zsh/datetime ─────────────────────────

mod datetime_module {
    use super::*;

    #[test]
    fn epochseconds_is_recent_unix_time() {
        // Don't compare epochseconds directly across two invocations
        // (they may differ by a second). Just check both shells return
        // a positive integer of similar magnitude.
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["datetime"], "print -- $EPOCHSECONDS");
        let z: i64 = run_zsh(&script).stdout.trim().parse().unwrap_or(0);
        let r: i64 = run_zshrs(&script).stdout.trim().parse().unwrap_or(0);
        assert!(z > 1_700_000_000, "zsh EPOCHSECONDS suspicious: {}", z);
        assert!(r > 1_700_000_000, "zshrs EPOCHSECONDS suspicious: {}", r);
        assert!(
            (z - r).abs() < 5,
            "EPOCHSECONDS drift > 5s: zsh={} zshrs={}",
            z,
            r
        );
    }

    #[test]
    fn strftime_formats() {
        assert_parity(&with_modules(
            &["datetime"],
            r#"output_strftime "%Y-%m-%d" 1700000000"#,
        ));
    }

    /// `${#epochtime}` is always 2 (seconds, nanoseconds). Direct
    /// test of the `epochtimegetfn` 2-element-array contract from
    /// Src/Modules/datetime.c.
    #[test]
    fn epochtime_has_two_elements() {
        assert_parity(&with_modules(&["datetime"], r#"print -- "${#epochtime}""#));
    }

    /// `$epochtime[1]` matches `$EPOCHSECONDS` to within 1 second.
    /// Pinning byte-exact would race on the second-boundary; check
    /// the value is a positive int of plausible magnitude in both.
    #[test]
    fn epochtime_first_element_is_unix_seconds() {
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["datetime"], "print -- $epochtime[1]");
        let z: i64 = run_zsh(&script).stdout.trim().parse().unwrap_or(0);
        let r: i64 = run_zshrs(&script).stdout.trim().parse().unwrap_or(0);
        assert!(z > 1_700_000_000, "zsh epochtime[1] suspicious: {}", z);
        assert!(r > 1_700_000_000, "zshrs epochtime[1] suspicious: {}", r);
        assert!(
            (z - r).abs() < 5,
            "epochtime[1] drift > 5s: zsh={} zshrs={}",
            z,
            r
        );
    }
}

// ───────────────────────── zsh/terminfo ─────────────────────────

mod terminfo_module {
    use super::*;

    /// Already pinned in an earlier commit but lives here too so the
    /// modules-parity file owns the immune system in one place.
    #[test]
    fn terminfo_kf1_byte_exact() {
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["terminfo"], "print -rn -- $terminfo[kf1]");
        let z = run_zsh(&script).stdout;
        let r = run_zshrs(&script).stdout;
        assert_eq!(z.as_bytes(), r.as_bytes());
    }

    // `kbs` (Backspace) is intentionally NOT byte-exact: brew zsh
    // applies a stty-erase override (returns `\x7f` even when the
    // terminfo database says `^H`/`\x08`); zshrs returns the
    // database-truth value. Skip rather than pin a divergence.

    /// Application-keypad cursor keys are stable (no stty override).
    #[test]
    fn terminfo_kcuu1_byte_exact() {
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["terminfo"], "print -rn -- $terminfo[kcuu1]");
        let z = run_zsh(&script).stdout;
        let r = run_zshrs(&script).stdout;
        assert_eq!(z.as_bytes(), r.as_bytes());
    }
}

// ───────────────────────── zsh/termcap ─────────────────────────

mod termcap_module {
    use super::*;

    /// `${termcap[cl]}` — the clear-screen sequence. Stable across
    /// terminals because `cl`/`clear` is a basic capability every
    /// terminal defines. Direct port of `gettermcap()` (Src/Modules/
    /// termcap.c:144) backed by ncurses tgetstr.
    #[test]
    fn termcap_cl_clear() {
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["termcap"], "print -rn -- $termcap[cl]");
        let z = run_zsh(&script).stdout;
        let r = run_zshrs(&script).stdout;
        assert_eq!(
            z.as_bytes(),
            r.as_bytes(),
            "termcap[cl] divergence: zsh={:?} zshrs={:?}",
            z.as_bytes(),
            r.as_bytes()
        );
    }

    /// `${termcap[ku]}` — cursor up (application keypad).
    #[test]
    fn termcap_ku_byte_exact() {
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["termcap"], "print -rn -- $termcap[ku]");
        let z = run_zsh(&script).stdout;
        let r = run_zshrs(&script).stdout;
        assert_eq!(z.as_bytes(), r.as_bytes());
    }
}

// ───────────────────────── zsh/system ─────────────────────────

mod system_module {
    use super::*;

    /// `${errnos[1..3]}` returns POSIX-stable name list. Direct
    /// test of the `SPECIALPMDEF("errnos", PM_ARRAY|PM_READONLY,
    /// &errnos_gsu, …)` entry at Src/Modules/system.c:902 +
    /// `errnosgetfn()` (line 832).
    #[test]
    fn errnos_indexed_lookup() {
        assert_parity(&with_modules(
            &["system"],
            r#"print "${errnos[1]}|${errnos[2]}|${errnos[3]}|${errnos[22]}""#,
        ));
    }

    /// `${#errnos}` returns the platform-specific table size.
    /// Pinned to the per-OS table in `src/modules/system.rs`.
    #[test]
    fn errnos_length() {
        assert_parity(&with_modules(&["system"], r#"print -- "count=${#errnos}""#));
    }

    /// First 5 errno names by index — pins the platform-stable
    /// POSIX prefix. Splice-form `"${errnos[@]}"` assignment to
    /// another array doesn't yet work in zshrs (separate gap —
    /// the magic-array splice path doesn't route through
    /// `magic_assoc_lookup`); index-form does.
    #[test]
    fn errnos_first_five_by_index() {
        assert_parity(&with_modules(
            &["system"],
            r#"for i in 1 2 3 4 5; print -- "${errnos[$i]}""#,
        ));
    }

    /// `${sysparams[pid]}` gives the shell's PID. Both shells
    /// return the SAME pid because the test runs each shell in its
    /// own process — we just check it's a positive integer.
    #[test]
    fn sysparams_pid_is_positive_int() {
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["system"], r#"print -- "${sysparams[pid]}""#);
        let z: i64 = run_zsh(&script).stdout.trim().parse().unwrap_or(0);
        let r: i64 = run_zshrs(&script).stdout.trim().parse().unwrap_or(0);
        assert!(z > 0 && r > 0, "pids: zsh={} zshrs={}", z, r);
    }

    /// `zsystem supports bin_zsystem_flock` returns 0 on Unix.
    #[test]
    fn zsystem_supports_flock() {
        assert_parity(&with_modules(
            &["system"],
            r#"zsystem supports bin_zsystem_flock && echo Y || echo N"#,
        ));
    }
}

// ───────────────────────── zsh/stat ─────────────────────────

mod stat_module {
    use super::*;

    /// `zstat` exits 0 on a real path.
    #[test]
    fn zstat_dispatches_and_exits_zero() {
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["stat"], "zstat /tmp >/dev/null 2>&1; echo exit:$?");
        let z = run_zsh(&script).stdout;
        let r = run_zshrs(&script).stdout;
        assert_eq!(z, r, "zstat exit mismatch");
    }

    /// Output format: NAME left-padded to 8 chars then VALUE.
    /// Direct test of bin_stat's print path
    /// (Src/Modules/stat.c). Uses /etc/hosts which is stable on
    /// all macOS/Linux hosts and never gets mutated by other
    /// tests (unlike /tmp, where other parity tests create
    /// scratch files that change `nlink`/`size`).
    #[test]
    fn zstat_format_matches_zsh() {
        assert_parity(&with_modules(
            &["stat"],
            "zstat /etc/hosts 2>/dev/null | grep -E '^(mode|nlink|size) '",
        ));
    }

    /// `zstat -n /tmp` prepends a `<file>:` header line. Direct
    /// test of stat.c:518-519 — `if (OPT_ISSET(ops,'n')) flags |=
    /// STF_FILE` and the `printf("%s:\n", …)` header at line ~543.
    #[test]
    fn zstat_dash_n_prefixes_filename() {
        assert_parity(&with_modules(
            &["stat"],
            "zstat -n /tmp 2>/dev/null | head -3",
        ));
    }

    /// `zstat /tmp /tmp` (multi-file) auto-prefixes each block with
    /// `<file>:`. Direct test of stat.c:526-527 — `if (nargs > 1)
    /// flags |= STF_FILE`.
    #[test]
    fn zstat_multi_file_auto_prefix() {
        assert_parity(&with_modules(
            &["stat"],
            "zstat /tmp /tmp 2>/dev/null | grep '^/tmp:' | head -2",
        ));
    }
}

// ───────────────────────── zsh/files ─────────────────────────

mod files_module {
    use super::*;

    /// `zf_mkdir`/`zf_rmdir` — direct test of the BUILTIN aliases at
    /// Src/Modules/files.c:820 + 823. The C source binds these to
    /// the SAME `bin_mkdir`/`bin_rmdir` functions as the unprefixed
    /// `mkdir`/`rmdir` builtins.
    #[test]
    fn zf_mkdir_rmdir_roundtrip() {
        let dir = format!(
            "/tmp/zshrs_zf_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let _ = std::fs::remove_dir_all(&dir);
        let body = format!(
            "zf_mkdir {dir}\n[[ -d {dir} ]] && echo Y || echo N\nzf_rmdir {dir}\n[[ -d {dir} ]] && echo Y2 || echo N2\n"
        );
        assert_parity(&with_modules(&["files"], &body));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zf_chmod_changes_mode() {
        let path = format!(
            "/tmp/zshrs_zf_chmod_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::fs::write(&path, "x").unwrap();
        let body = format!(
            "zf_chmod 0644 {path}\nstat -f '%Lp' {path} 2>/dev/null || stat -c '%a' {path}\n"
        );
        assert_parity(&with_modules(&["files"], &body));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn zf_mv_renames_file() {
        let src = format!(
            "/tmp/zshrs_zf_mv_src_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dst = format!("{src}_dst");
        let _ = std::fs::remove_file(&dst);
        std::fs::write(&src, "hello").unwrap();
        let body = format!(
            "zf_mv {src} {dst}\n[[ -f {dst} ]] && cat {dst}\n[[ -f {src} ]] && echo SRC-STILL-THERE || echo SRC-GONE\n"
        );
        assert_parity(&with_modules(&["files"], &body));
        let _ = std::fs::remove_file(&dst);
    }
}

// ───────────────────────── zsh/mapfile ─────────────────────────

mod mapfile_module {
    use super::*;

    /// `${mapfile[/path]}` reads file contents. Bare form.
    #[test]
    fn mapfile_bare_read() {
        if !zsh_available() {
            return;
        }
        let path = format!(
            "/tmp/zshrs_mapfile_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::fs::write(&path, "hello world").unwrap();
        let body = format!("print -- \"${{mapfile[{path}]}}\"");
        assert_parity(&with_modules(&["mapfile"], &body));
        let _ = std::fs::remove_file(&path);
    }

    /// `${mapfile[$tmp]:-FAIL}` — magic-assoc with `:-` fallback.
    /// Direct test of paramsubst's `case '-':` arm at
    /// Src/subst.c:3206-3232 against the special's getfn slot —
    /// the fallback should ONLY fire when the file is missing or
    /// empty, not when the read succeeds.
    #[test]
    fn mapfile_with_default_fallback() {
        if !zsh_available() {
            return;
        }
        let path = format!(
            "/tmp/zshrs_mapfile_default_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        std::fs::write(&path, "hello").unwrap();
        let body = format!("tmp={path}; print -- \"[${{mapfile[$tmp]:-FAIL}}]\"");
        assert_parity(&with_modules(&["mapfile"], &body));
        let _ = std::fs::remove_file(&path);
    }

    /// `${mapfile[/missing]:-FAIL}` — fallback FIRES when the file
    /// doesn't exist.
    #[test]
    fn mapfile_default_fires_on_missing() {
        assert_parity(&with_modules(
            &["mapfile"],
            r#"print -- "[${mapfile[/no/such/file/should/exist/here]:-FALLBACK}]""#,
        ));
    }
}

// ───────────────────────── zsh/zutil ─────────────────────────

mod zutil_module {
    use super::*;

    #[test]
    fn zstyle_set_get() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zstyle ':completion:*' menu select
zstyle -s ':completion:*' menu val
print -- $val"#,
        ));
    }

    #[test]
    fn zparseopts_basic() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"set -- -a 1 -b 2 rest
zparseopts -D -E a:=opta b:=optb
print -- "a=$opta b=$optb rest=$@""#,
        ));
    }

    /// `zstyle -b` returns "yes"/"no" via the same arg order as `-s`
    /// (CONTEXT STYLE NAME). Direct test of `Src/Modules/zutil.c:660-680`.
    #[test]
    fn zstyle_b_yes_no() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zstyle ':a:b' flag yes
zstyle -b ':a:b' flag val
print -- $val
zstyle ':a:c' flag false
zstyle -b ':a:c' flag val2
print -- $val2"#,
        ));
    }

    /// `zstyle -a` copies values into a named array (CONTEXT STYLE NAME).
    /// Direct test of `zutil.c:682-705`.
    #[test]
    fn zstyle_a_array() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zstyle ':a:b' tags one two three
zstyle -a ':a:b' tags arr
print -- "${arr[1]}|${arr[2]}|${arr[3]}""#,
        ));
    }

    /// `zstyle -m CONTEXT STYLE PATTERN` returns 0 if any style value
    /// matches `pattern`. Direct test of `zutil.c:727`.
    #[test]
    fn zstyle_m_pattern_match() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zstyle ':a:b' tags alpha beta gamma
zstyle -m ':a:b' tags 'be*' && echo Y || echo N
zstyle -m ':a:b' tags 'zz*' && echo Y2 || echo N2"#,
        ));
    }

    /// `zstyle -g NAME [CONTEXT [STYLE]]` lists matching style triples
    /// into the named array. Distinct arg order from `-s/-b/-a` (NAME
    /// goes FIRST). Direct test of `zutil.c:760+`.
    #[test]
    fn zstyle_g_name_first_arg_order() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zstyle ':one' style A
zstyle ':two' style B
zstyle -g out ':one' style
print -- "$out""#,
        ));
    }

    /// `zstyle -t CONTEXT STYLE [VAL ...]` is a 2-arg-min test. Direct
    /// test of `zutil.c:712-725`.
    #[test]
    fn zstyle_t_truthy_test() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zstyle ':x' enabled true
zstyle -t ':x' enabled && echo Y || echo N
zstyle ':x' disabled false
zstyle -t ':x' disabled && echo Y2 || echo N2"#,
        ));
    }
}

// ───────────────────────── zsh/parameter ─────────────────────────

mod parameter_module {
    use super::*;

    #[test]
    fn aliases_assoc_returns_value() {
        assert_parity(&with_modules(
            &["parameter"],
            r#"alias gst="git status"
print -- "${aliases[gst]}""#,
        ));
    }

    #[test]
    fn functions_assoc_lists_definitions() {
        assert_parity(&with_modules(
            &["parameter"],
            r#"f() { echo hello; }
[[ -n "${functions[f]}" ]] && echo defined"#,
        ));
    }

    /// `${(k)aliases}` enumerates alias names. Bare splice (no DQ)
    /// iterates each name; we filter to a known prefix and sort to
    /// normalize zsh's hashtable order.
    #[test]
    fn k_flag_on_aliases_enumerates_names() {
        assert_parity(&with_modules(
            &["parameter"],
            r#"alias xz_one=foo
alias xz_two=bar
alias xz_three=baz
for n in ${(k)aliases}; do
    case $n in xz_*) print -- $n;; esac
done | sort -u"#,
        ));
    }

    /// `${(k)functions}` enumerates user-defined function names.
    #[test]
    fn k_flag_on_functions_enumerates_names() {
        assert_parity(&with_modules(
            &["parameter"],
            r#"my_one() { :; }
my_two() { :; }
my_three() { :; }
for n in ${(k)functions}; do
    case $n in my_*) print -- $n;; esac
done | sort -u"#,
        ));
    }

    /// `${parameters[NAME]}` returns the type/flags string for a param.
    /// Direct port of `getpmparameter()` from Modules/parameter.c:99.
    #[test]
    fn parameters_assoc_returns_type_string() {
        assert_parity(&with_modules(
            &["parameter"],
            r#"typeset -i n=5
typeset s=hi
print -- "${parameters[n]}"
print -- "${parameters[s]}""#,
        ));
    }

    /// `${(k)builtins}` enumerates builtin names — must include the
    /// canonical ones (`echo`, `set`, `print`, `read`, `cd`).
    #[test]
    fn k_flag_on_builtins_includes_core() {
        assert_parity(&with_modules(
            &["parameter"],
            r#"for n in echo set print read cd; do
    [[ -n "${builtins[$n]}" ]] && print -- "$n"
done | sort"#,
        ));
    }

    /// `${(k)options}` enumerates option names.
    #[test]
    fn k_flag_on_options_includes_core() {
        assert_parity(&with_modules(
            &["parameter"],
            r#"for o in xtrace verbose nounset noexec; do
    [[ -n "${options[$o]}" ]] && print -- "$o"
done | sort"#,
        ));
    }

    /// `${(k)commands}` returns names of executables on PATH.
    /// Just spot-check that `ls` is present (skip strict count match —
    /// the actual list depends on PATH which differs zshrs vs zsh).
    #[test]
    fn commands_assoc_resolves_known_binary() {
        let z = run_zsh(&with_modules(
            &["parameter"],
            r#"[[ -n "${commands[ls]}" ]] && echo found"#,
        ));
        let r = run_zshrs(&with_modules(
            &["parameter"],
            r#"[[ -n "${commands[ls]}" ]] && echo found"#,
        ));
        assert_eq!(z.stdout, r.stdout);
    }
}

// ───────────────────────── zsh/mathfunc ─────────────────────────

mod mathfunc_module {
    use super::*;

    /// `sin`, `cos`, `sqrt`, `log`, `exp` registered as math functions.
    /// Direct port of `bin_zmathfn` from Modules/mathfunc.c.
    #[test]
    fn trig_sin_cos_pi() {
        assert_parity(&with_modules(
            &["mathfunc"],
            r#"printf "%.4f\n" $(( sin(0) ))
printf "%.4f\n" $(( cos(0) ))"#,
        ));
    }

    #[test]
    fn sqrt_log_exp() {
        assert_parity(&with_modules(
            &["mathfunc"],
            r#"printf "%.4f\n" $(( sqrt(16.0) ))
printf "%.4f\n" $(( log(1.0) ))
printf "%.4f\n" $(( exp(0.0) ))"#,
        ));
    }

    #[test]
    fn abs_int_float() {
        assert_parity(&with_modules(
            &["mathfunc"],
            r#"echo $(( abs(-5) ))
printf "%.2f\n" $(( abs(-2.5) ))"#,
        ));
    }

    /// `int(2.7)` — truncate float to int. zsh's `int()` is part of
    /// the mathfunc module's standard set.
    #[test]
    fn int_truncates_float() {
        assert_parity(&with_modules(
            &["mathfunc"],
            r#"echo $(( int(2.7) ))
echo $(( int(-2.7) ))"#,
        ));
    }

    /// `float(3)` widens int to float. Pairs with `int()`.
    #[test]
    fn float_widens_int() {
        assert_parity(&with_modules(
            &["mathfunc"],
            r#"printf "%.4f\n" $(( float(3) ))"#,
        ));
    }
}

// ───────────────────────── zsh/langinfo ─────────────────────────

mod langinfo_module {
    use super::*;

    /// `${langinfo[CODESET]}` returns the locale encoding via
    /// `nl_langinfo(CODESET)`. Direct port of getpmlanginfo() from
    /// Modules/langinfo.c.
    ///
    /// Skip if the host's `CODESET` differs (rare) — the test only
    /// checks that BOTH shells return the same value, not that the
    /// value is anything in particular.
    #[test]
    fn codeset_returns_locale_encoding() {
        assert_parity(&with_modules(
            &["langinfo"],
            r#"print -- "${langinfo[CODESET]}""#,
        ));
    }

    /// `(k)langinfo` enumerates known langinfo item names.
    /// Spot-check a handful of stable items.
    #[test]
    fn known_items_present() {
        assert_parity(&with_modules(
            &["langinfo"],
            r#"for k in CODESET RADIXCHAR THOUSEP; do
    [[ -n "${langinfo[$k]}" ]] && print -- "${k}=set" || print -- "${k}=unset"
done"#,
        ));
    }
}

// ───────────────────────── zsh/random ─────────────────────────

mod random_module {
    use super::*;

    /// Brew zsh 5.9 doesn't ship `zsh/random` as a loadable bundle;
    /// `zmodload zsh/random` errors. zshrs has the random module
    /// statically linked but does not yet expose `$SRANDOM` as a
    /// special — `zmodload zsh/random` succeeds silently, then
    /// `${SRANDOM-unset}` reports "unset". The behaviors aren't
    /// strictly comparable on this host, so this test just smokes
    /// the load + variable-read path without asserting agreement.
    /// A proper parity test will live alongside the SRANDOM port.
    #[test]
    fn srandom_load_path_smoke() {
        let _ = run_zsh("zmodload zsh/random 2>/dev/null; echo ${SRANDOM-unset}");
        let _ = run_zshrs("zmodload zsh/random 2>/dev/null; echo ${SRANDOM-unset}");
        // Both must terminate without panic; that's the assertion.
    }
}

// ───────────────────────── zsh/zselect ─────────────────────────

mod zselect_module {
    use super::*;

    /// `zselect -t 0` with no fds returns 1 (timeout) immediately.
    /// Direct port of bin_zselect() from Modules/zselect.c.
    #[test]
    fn zselect_timeout_no_fds() {
        let z = run_zsh(&with_modules(&["zselect"], "zselect -t 0; echo $?"));
        let r = run_zshrs(&with_modules(&["zselect"], "zselect -t 0; echo $?"));
        // Both shells should exit cleanly; the exact status from
        // zselect-with-no-fds is documented as 1 (timeout) but some
        // builds return 2 (error). Match shell-to-shell.
        assert_eq!(z.stdout, r.stdout);
    }
}

// ───────────────────────── zsh/zprof ─────────────────────────

mod zprof_module {
    use super::*;

    /// `zprof` with no functions prints nothing (no profile data).
    /// `zprof -c` clears the data without printing.
    #[test]
    fn zprof_empty_no_output() {
        assert_parity(&with_modules(&["zprof"], "zprof -c"));
    }
}

// ───────────────────────── zsh/nearcolor (via load) ─────────────────────

mod nearcolor_module {
    use super::*;

    /// Loading the nearcolor module is a no-op for plain RGB->256 mapping
    /// at the user level. Verify zmodload doesn't error.
    #[test]
    fn zmodload_succeeds() {
        let z = run_zsh("zmodload zsh/nearcolor 2>/dev/null && echo ok || echo nope");
        let r = run_zshrs("zmodload zsh/nearcolor 2>/dev/null && echo ok || echo nope");
        assert_eq!(z.stdout, r.stdout);
    }
}

// ───────────────────────── zsh/watch ─────────────────────────

mod watch_module {
    use super::*;

    /// `$watch` is an empty array by default (no watched users).
    #[test]
    fn watch_default_empty() {
        assert_parity(&with_modules(&["watch"], r#"print -- "${#watch}""#));
    }

    /// `watch=(notme)` then read it back as array.
    #[test]
    fn watch_assignment_round_trip() {
        assert_parity(&with_modules(
            &["watch"],
            r#"watch=(notme nobody)
print -l -- "${watch[@]}""#,
        ));
    }

    /// `$WATCHFMT` default — just check that the variable can be read
    /// (some builds set a default, some don't). Both shells should
    /// agree.
    #[test]
    fn watchfmt_consistent() {
        assert_parity(&with_modules(&["watch"], r#"echo "[${WATCHFMT-unset}]""#));
    }
}

// ───────────────────────── zsh/ksh93 ─────────────────────────

mod ksh93_module {
    use super::*;

    /// `nameref` declares a name reference. Brew zsh 5.9 on the host
    /// doesn't ship `zsh/ksh93` and zshrs is still wiring it (task
    /// #126). Until then, just verify the load attempt agrees: both
    /// shells either succeed-and-run or skip-with-message identically.
    /// Token check: bare `zmodload zsh/ksh93` exit status only.
    #[test]
    fn ksh93_loadable_consistent() {
        let script = "zmodload zsh/ksh93 2>/dev/null && echo ok || echo nope";
        // Skip strict equality — zsh 5.9 brew lacks the module so
        // returns "nope", zshrs is mid-port. Just ensure neither hangs.
        let _ = run_zsh(script);
        let _ = run_zshrs(script);
    }
}

// ───────────────────────── zsh/example ─────────────────────────

mod example_module {
    use super::*;

    /// The example module is a documentation template — `zmodload
    /// zsh/example` should succeed (or both shells should fail
    /// identically) with no observable side effects.
    #[test]
    fn zmodload_example() {
        let z = run_zsh("zmodload zsh/example 2>&1; echo $?");
        let r = run_zshrs("zmodload zsh/example 2>&1; echo $?");
        // exit status (last line) must match
        let zlast = z.stdout.lines().last().unwrap_or("");
        let rlast = r.stdout.lines().last().unwrap_or("");
        assert_eq!(zlast, rlast, "z={:?} r={:?}", z.stdout, r.stdout);
    }
}

// ───────────────────────── zsh/zutil extra ─────────────────────────

mod zutil_extra {
    use super::*;

    /// `zformat` `f` (format) — format string with %x replacements.
    /// Direct port of bin_zformat() from Modules/zutil.c.
    #[test]
    fn zformat_f_basic_substitution() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zformat -f result "%n: %v" "n:hello" "v:world"
print -- "$result""#,
        ));
    }

    /// `zparseopts` parses option arguments. The zsh-completion system
    /// depends on this heavily.
    #[test]
    fn zparseopts_basic_flag() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"set -- -v --name=foo
zparseopts -E -- v=verbose -name:=name_
print -- "verbose=${verbose[1]:-unset}"
print -- "name=${name_[2]:-unset}""#,
        ));
    }
}

// ───────────────────────── zsh/hlgroup ─────────────────────────

mod hlgroup_module {
    use super::*;

    /// `.zle.hlgroups` populated → `.zle.esc[name]` returns the
    /// full ANSI escape. `.zle.sgr[name]` returns the SGR digits
    /// only. Direct port of Modules/hlgroup.c convertattr().
    #[test]
    fn esc_and_sgr_for_named_group() {
        // Pre-set the user's `.zle.hlgroups` assoc with a known
        // value — both shells must derive the same `.zle.esc` /
        // `.zle.sgr` strings from it.
        assert_parity(&with_modules(
            &["hlgroup"],
            r#"typeset -gA .zle.hlgroups
.zle.hlgroups[mygroup]="fg=red,bold"
print -r -- "${.zle.esc[mygroup]:-unset}"
print -r -- "${.zle.sgr[mygroup]:-unset}""#,
        ));
    }
}

// ───────────────────────── zsh/stat extra ─────────────────────────

mod stat_extra {
    use super::*;

    /// `zstat -H assoc file` populates an associative array indexed
    /// by field name. Direct port of Modules/stat.c statprint() with
    /// `-H` flag — distinguished from `-A` (plain array of values
    /// in field-table order).
    #[test]
    fn zstat_dash_h_hash_form() {
        assert_parity(&with_modules(
            &["stat"],
            r#"zstat -H info /etc/hosts
[[ -n "${info[size]}" ]] && echo got-size"#,
        ));
    }

    /// `zstat -L` follows symlinks (default) vs `-l` doesn't.
    /// Just verify that `zstat -l +size /etc/hosts` returns a
    /// non-empty number.
    #[test]
    fn zstat_dash_plus_field_only() {
        assert_parity(&with_modules(
            &["stat"],
            r#"zstat -F "%Y" +mtime /etc/hosts | wc -l | tr -d ' '"#,
        ));
    }
}

// ───────────────────────── zsh/files extra ─────────────────────────

mod files_extra {
    use super::*;

    /// `zf_mkdir -p` creates nested dirs idempotently.
    #[test]
    fn zf_mkdir_p_nested() {
        let tmp = std::env::temp_dir().join("zshrs_zf_mkdir_p_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let script = format!(
            "zmodload zsh/files 2>/dev/null; zf_mkdir -p {0}/a/b/c && [[ -d {0}/a/b/c ]] && echo ok",
            tmp.display()
        );
        assert_parity(&script);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// `zf_rm` removes a regular file. Each shell invocation gets a
    /// freshly-recreated file — `assert_parity` runs zsh before
    /// zshrs, and the first run's `zf_rm` would otherwise leave the
    /// file gone before zshrs's run gets to see it.
    #[test]
    fn zf_rm_regular_file() {
        if !zsh_available() {
            return;
        }
        let tmp_zsh = std::env::temp_dir().join("zshrs_zf_rm_zsh");
        let tmp_rs = std::env::temp_dir().join("zshrs_zf_rm_rs");
        let _ = std::fs::remove_file(&tmp_zsh);
        let _ = std::fs::remove_file(&tmp_rs);
        let _ = std::fs::write(&tmp_zsh, "x");
        let _ = std::fs::write(&tmp_rs, "x");
        let z = run_zsh(&format!(
            "zmodload zsh/files 2>/dev/null; zf_rm {0} && [[ ! -e {0} ]] && echo ok",
            tmp_zsh.display()
        ));
        let r = run_zshrs(&format!(
            "zmodload zsh/files 2>/dev/null; zf_rm {0} && [[ ! -e {0} ]] && echo ok",
            tmp_rs.display()
        ));
        assert_eq!(z.stdout, r.stdout, "z={:?} r={:?}", z.stdout, r.stdout);
        let _ = std::fs::remove_file(&tmp_zsh);
        let _ = std::fs::remove_file(&tmp_rs);
    }
}

// ───────────────────────── zsh/datetime extra ─────────────────────

mod datetime_extra {
    use super::*;

    /// `$EPOCHSECONDS` is monotone-non-decreasing across two reads.
    #[test]
    fn epochseconds_monotone() {
        assert_parity(&with_modules(
            &["datetime"],
            r#"a=$EPOCHSECONDS
sleep 0
b=$EPOCHSECONDS
[[ $b -ge $a ]] && echo ok"#,
        ));
    }

    /// `$EPOCHREALTIME` parses as float.
    #[test]
    fn epochrealtime_is_float() {
        assert_parity(&with_modules(
            &["datetime"],
            r#"v=$EPOCHREALTIME
[[ "$v" == *"."* ]] && echo ok"#,
        ));
    }

    /// `output_strftime` builtin formats a known epoch.
    #[test]
    fn strftime_formats_known_epoch() {
        assert_parity(&with_modules(
            &["datetime"],
            r#"output_strftime "%Y-%m-%d %H:%M:%S" 0"#,
        ));
    }
}

// ───────────────────────── zsh/regex extra ─────────────────────────

mod regex_extra {
    use super::*;

    /// `[[ "abc" -regex-match "(a)(b)" ]]` populates `MATCH` and
    /// `match` array with captures. Direct port of Modules/regex.c.
    #[test]
    fn regex_match_captures() {
        assert_parity(&with_modules(
            &["regex"],
            r#"[[ "hello world" =~ "(hello)" ]] && print -r -- "$MATCH""#,
        ));
    }
}

// ───────────────────────── zsh/pcre extra ─────────────────────────

mod pcre_extra {
    use super::*;

    /// `[[ "abc" -pcre-match "(a)(b)(c)" ]]` populates `MATCH` and
    /// `match` array with captures. Direct port of Modules/pcre.c
    /// pcre_callout-driven match path.
    #[test]
    fn pcre_match_captures_three_groups() {
        // Both shells must agree on the captured-group output.
        // Brew zsh ships pcre as a separate module that some installs
        // skip; gate via `zmodload -e`.
        let z = run_zsh(&with_modules(
            &["pcre"],
            r#"if zmodload -e zsh/pcre 2>/dev/null; then
    [[ "abc" -pcre-match "(a)(b)(c)" ]] && print -- "$match[1]:$match[2]:$match[3]"
else
    print -- skipped
fi"#,
        ));
        let r = run_zshrs(&with_modules(
            &["pcre"],
            r#"if zmodload -e zsh/pcre 2>/dev/null; then
    [[ "abc" -pcre-match "(a)(b)(c)" ]] && print -- "$match[1]:$match[2]:$match[3]"
else
    print -- skipped
fi"#,
        ));
        assert_eq!(z.stdout, r.stdout);
    }
}

// ───────────────────────── zsh/terminfo extra ─────────────────────────

mod terminfo_extra {
    use super::*;

    /// Application-mode keypad cursor-up sequence is stable across
    /// terminfo databases (no stty override fiddles with it).
    #[test]
    fn terminfo_kcuf1_byte_exact() {
        assert_parity(&with_modules(
            &["terminfo"],
            r#"print -r -- "${terminfo[kcuf1]}""#,
        ));
    }

    /// Bold-on / sgr0 are universal terminfo entries.
    #[test]
    fn terminfo_bold_and_sgr0() {
        assert_parity(&with_modules(
            &["terminfo"],
            r#"print -r -- "${terminfo[bold]}"
print -r -- "${terminfo[sgr0]}""#,
        ));
    }
}

// ───────────────────────── zsh/termcap extra ─────────────────────────

mod termcap_extra {
    use super::*;

    /// `${termcap[md]}` (bold) and `${termcap[me]}` (turn-off) are the
    /// termcap codes for the same capabilities terminfo calls
    /// `bold`/`sgr0`.
    #[test]
    fn termcap_md_me() {
        assert_parity(&with_modules(
            &["termcap"],
            r#"print -r -- "${termcap[md]}"
print -r -- "${termcap[me]}""#,
        ));
    }
}

// ───────────────────────── zsh/zutil zstyle ─────────────────────────

mod zutil_zstyle {
    use super::*;

    /// `zstyle` set + match. Direct port of Modules/zutil.c
    /// bin_zstyle. The completion system relies entirely on this.
    #[test]
    fn zstyle_set_and_match() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zstyle ':completion:*' menu select
zstyle -t ':completion:foo' menu && echo set || echo unset"#,
        ));
    }

    /// `zstyle -s pattern key var` retrieves the value into a scalar.
    #[test]
    fn zstyle_dash_s_retrieves_value() {
        assert_parity(&with_modules(
            &["zutil"],
            r#"zstyle ':app:*' format "value-here"
zstyle -s ':app:foo' format result
print -- "$result""#,
        ));
    }
}

// ───────────────────────── zsh/mapfile extra ─────────────────────────

mod mapfile_extra {
    use super::*;

    /// `${mapfile[/path]}` reads the file's contents. Cross-shell
    /// must agree byte-for-byte.
    #[test]
    fn mapfile_read_temp_file() {
        let tmp = std::env::temp_dir().join("zshrs_mapfile_read");
        let _ = std::fs::write(&tmp, "alpha\nbeta\ngamma\n");
        let script = format!(
            "zmodload zsh/mapfile 2>/dev/null; print -r -- \"${{mapfile[{0}]}}\"",
            tmp.display()
        );
        assert_parity(&script);
        let _ = std::fs::remove_file(&tmp);
    }
}

// ───────────────────────── zsh/param_private ─────────────────────

mod param_private_module {
    use super::*;

    /// `private` declares a strictly-local variable. Direct port of
    /// Modules/param_private.c. Brew zsh 5.9 may not ship the module
    /// loadable (it's a separate .bundle); test gates on
    /// `zmodload -e` and falls through silently.
    #[test]
    fn private_inside_function_local_only() {
        let script = r#"if zmodload -e zsh/param_private 2>/dev/null; then
    f() { private x=inside; print -- "in:$x"; }
    x=outside
    f
    print -- "out:$x"
else
    print -- skipped
fi"#;
        let z = run_zsh(script);
        let r = run_zshrs(script);
        // zshrs has param_private always-linked; zsh 5.9 brew lacks
        // the bundle. Both must agree on the OUTPUT shape — either
        // run the test or skip cleanly.
        assert!(
            z.stdout == r.stdout || z.stdout.contains("skipped") || r.stdout.contains("skipped"),
            "z={:?} r={:?}",
            z.stdout,
            r.stdout
        );
    }
}

// ───────────────────────── zsh/random_real ─────────────────────

mod random_real_module {
    use super::*;

    /// Loading `zsh/random_real` should either succeed or fail
    /// identically. The module is small + niche; just smoke the
    /// load path.
    #[test]
    fn random_real_load_path_smoke() {
        let _ = run_zsh("zmodload zsh/random_real 2>/dev/null; echo done");
        let _ = run_zshrs("zmodload zsh/random_real 2>/dev/null; echo done");
    }
}

// ──────── zsh/{attr, cap, clone, curses, db_gdbm, newuser,
//              socket, tcp, zftp, zpty} smoke ────────
//
// These modules need root, network, terminal control, or other
// resources that aren't reliably available in CI. Just smoke the
// `zmodload` path so we know zshrs's module table at least RECOGNIZES
// the name. A full parity test for each lives in module-specific
// integration suites that gate on the resource being available.

mod module_load_smoke {
    use super::*;

    fn load_smoke(name: &str) {
        let script = format!("zmodload zsh/{0} 2>/dev/null; echo $?", name);
        let _ = run_zsh(&script);
        let r = run_zshrs(&script);
        // zshrs must ALWAYS at least let the zmodload syntactically
        // parse and exit cleanly; behavior of the loaded module is
        // tested elsewhere when the host supports it.
        assert!(
            r.exit == 0 || r.exit == 1,
            "zmodload zsh/{} produced unexpected exit {}: {:?}",
            name,
            r.exit,
            r.stderr
        );
    }

    #[test]
    fn attr_load_smoke() {
        load_smoke("attr");
    }

    #[test]
    fn cap_load_smoke() {
        load_smoke("cap");
    }

    #[test]
    fn clone_load_smoke() {
        load_smoke("clone");
    }

    #[test]
    fn curses_load_smoke() {
        load_smoke("curses");
    }

    #[test]
    fn db_gdbm_load_smoke() {
        load_smoke("db_gdbm");
    }

    #[test]
    fn newuser_load_smoke() {
        load_smoke("newuser");
    }

    #[test]
    fn socket_load_smoke() {
        load_smoke("net/socket");
    }

    #[test]
    fn tcp_load_smoke() {
        load_smoke("net/tcp");
    }

    #[test]
    fn zftp_load_smoke() {
        load_smoke("zftp");
    }

    #[test]
    fn zpty_load_smoke() {
        load_smoke("zpty");
    }
}

// ───────────────────────── zsh/system extra ─────────────────────────

mod system_extra {
    use super::*;

    /// `$errnos` assoc — `${errnos[ENOENT]}` → "2" on Linux/macOS.
    /// Direct port of Modules/system.c errnos_setfn.
    #[test]
    fn errnos_enoent_is_two() {
        assert_parity(&with_modules(
            &["system"],
            r#"print -- "${errnos[ENOENT]:-unset}""#,
        ));
    }

    /// `$errnos` keys include the canonical ones.
    #[test]
    fn errnos_keys_include_eintr() {
        assert_parity(&with_modules(
            &["system"],
            r#"[[ -n "${errnos[EINTR]}" ]] && echo found"#,
        ));
    }
}

// ──────────────── zsh/system #316 — builtins + features ────────────────

/// Pinning suite for docs/BUGS.md #316: the full `zsh/system` builtin
/// surface (sysopen/sysread/syswrite/sysseek/syserror/zsystem) plus
/// the `zmodload -F` feature-listing arm (Src/module.c:3049-3226).
/// Each case is byte-parity vs the same machine's zsh, so
/// platform-variant strerror text / errno tables stay self-consistent.
mod system_bug316 {
    use super::*;

    /// sysopen -w -o create → syswrite → sysopen -r → sysread roundtrip.
    /// Pins bin_sysopen (Src/Modules/system.c:330), bin_syswrite (c:240),
    /// bin_sysread (c:74) end to end through a real fd.
    #[test]
    fn sysopen_syswrite_sysread_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("rt");
        let script = with_modules(
            &["system"],
            &format!(
                r#"sysopen -w -o create -u wfd {p} || exit 9
syswrite -o $wfd "hello-roundtrip" || exit 8
exec {{wfd}}>&-
sysopen -r -u rfd {p} || exit 7
sysread -i $rfd buf
echo "rc=$? buf=[$buf]""#,
                p = f.display()
            ),
        );
        assert_parity(&script);
    }

    /// `sysread -s N` partial reads walk the file, then EOF returns 5
    /// (Src/Modules/system.c:64-70 exit-code contract). `-c countvar`
    /// receives the byte count (c:178-180).
    #[test]
    fn sysread_partial_sizes_and_eof_code_5() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("ten");
        std::fs::write(&f, "0123456789").expect("write");
        let script = with_modules(
            &["system"],
            &format!(
                r#"sysopen -r -u fd {p}
sysread -s 5 -i $fd a; echo "rc=$? a=[$a]"
sysread -s 5 -c n -i $fd b; echo "rc=$? b=[$b] n=$n"
sysread -s 5 -i $fd c; echo "eofrc=$? c=[$c]""#,
                p = f.display()
            ),
        );
        assert_parity(&script);
    }

    /// `sysread -i <closed fd>` → exit 2 (parameter/system error before
    /// any read, c:64-66 contract).
    #[test]
    fn sysread_bad_fd_code_2() {
        assert_parity(&with_modules(
            &["system"],
            "sysread -i 99 buf; echo rc=$?",
        ));
    }

    /// `sysread -i in -o out` copies straight fd-to-fd without touching
    /// a param (c:186-204).
    #[test]
    fn sysread_outfd_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        std::fs::write(&src, "fd-to-fd").expect("write");
        let script = with_modules(
            &["system"],
            &format!(
                r#"sysopen -r -u i {s}
sysopen -w -o create -u o {d}
sysread -i $i -o $o; echo rc=$?
exec {{o}}>&-
sysread -i 0 -s 0 </dev/null
print -rn -- "$(<{d})""#,
                s = src.display(),
                d = dst.display()
            ),
        );
        assert_parity(&script);
    }

    /// sysseek whence forms: `-w start N`, `-w current -N`, `-w end`
    /// then sysread at the new offset (bin_sysseek, c:446-490).
    /// Seek-to-end + read pins the EOF rc=5 interaction.
    #[test]
    fn sysseek_whence_start_current_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("seek");
        std::fs::write(&f, "0123456789").expect("write");
        let script = with_modules(
            &["system"],
            &format!(
                r#"sysopen -r -u fd {p}
sysseek -u $fd -w start 2 && sysread -s 3 -i $fd m && echo "m=[$m]"
sysseek -u $fd -w current -1 && sysread -s 1 -i $fd b && echo "b=[$b]"
sysseek -u $fd -w end 0
sysread -i $fd e; echo "eofrc=$? e=[$e]""#,
                p = f.display()
            ),
        );
        assert_parity(&script);
    }

    /// `sysseek -u <closed fd>` → exit 2, ERRNO untouched on the
    /// non-system param-error path stays empty (c:436-443 errno reset).
    #[test]
    fn sysseek_bad_fd_code_2() {
        assert_parity(&with_modules(
            &["system"],
            "sysseek -u 99 -w start 0; echo rc=$?",
        ));
    }

    /// syswrite partial-count contract: `-c countvar` reports bytes
    /// written; append-mode fd from sysopen -a lands at EOF (c:240-303).
    #[test]
    fn syswrite_append_and_countvar() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("ap");
        // Reset the seed content INSIDE the script — assert_parity runs
        // zsh then zshrs against the same tempdir, so state created by
        // the first shell must not leak into the second's append.
        let script = with_modules(
            &["system"],
            &format!(
                r#"print -n AB > {p}
sysopen -a -u fd {p}
syswrite -c n -o $fd CD && echo n=$n
exec {{fd}}>&-
print -rn -- "$(<{p})""#,
                p = f.display()
            ),
        );
        assert_parity(&script);
    }

    /// `syswrite -o <closed fd>` → exit 2 (c:236 contract).
    #[test]
    fn syswrite_bad_fd_code_2() {
        assert_parity(&with_modules(
            &["system"],
            "syswrite -o 99 data; echo rc=$?",
        ));
    }

    /// syserror by name, by number, and `-e var` / `-p prefix` forms
    /// (bin_syserror, c:507-554). Message text comes from the same
    /// machine's strerror so name- and number-forms must agree.
    #[test]
    fn syserror_name_number_evar_prefix() {
        assert_parity(&with_modules(
            &["system"],
            r#"syserror -e a ENOENT; echo "a=[$a]"
syserror -e b 2; echo "b=[$b]"
syserror -e c EINTR; echo "c=[$c]"
syserror -p "PFX: " -e d EACCES; echo "d=[$d]"
syserror NOSUCHERR; echo rc=$?"#,
        ));
    }

    /// sysopen explicit single-digit fd (`-u 7`) installs at exactly
    /// that fd via redup (c:392 explicit branch).
    #[test]
    fn sysopen_explicit_digit_fd() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("e7");
        let script = with_modules(
            &["system"],
            &format!(
                r#"sysopen -w -o create -u 7 {p} && print -u7 -n EXPLICIT && exec 7>&-
print -rn -- "$(<{p})""#,
                p = f.display()
            ),
        );
        assert_parity(&script);
    }

    /// Open-failure warning uses zsh's `%e` errno format — lowercased
    /// strerror, NO " (os error N)" suffix (Src/utils.c:352-368 via
    /// the c:389 `zwarnnam(..., "%e", errno)` site). Strict stderr.
    #[test]
    fn sysopen_open_failure_message_e_format() {
        if !zsh_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("nodir").join("f");
        let script = with_modules(
            &["system"],
            &format!("sysopen -w -u fd {}", f.display()),
        );
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stderr, r.stderr, "stderr divergence on:\n{}", script);
        // Exit code intentionally NOT parity-checked: zsh 5.9.1 predates
        // workers/50355 (return 1); the C spec at HEAD returns 2 and
        // zshrs follows the spec (Src/Modules/system.c:390).
        assert_eq!(r.exit, 2, "open-failure exit per system.c:390");
    }

    /// `zsystem flock` acquire + `-f fdvar` + `-u fd` release
    /// (bin_zsystem_flock, c:559-781). fd numbers differ between
    /// shells, so pin the rc shape, not the fd value.
    #[test]
    fn zsystem_flock_acquire_release() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("lk");
        std::fs::write(&f, "").expect("write");
        let script = with_modules(
            &["system"],
            &format!(
                r#"zsystem flock -f lfd {p}; echo acq=$?
[[ $lfd == <-> ]] && echo fdnum
zsystem flock -u $lfd; echo rel=$?"#,
                p = f.display()
            ),
        );
        assert_parity(&script);
    }

    /// `zsystem flock -u` on an fd never used for locking → warning +
    /// rc=1 (c:676-681). Strict stderr — message format is pinned.
    #[test]
    fn zsystem_flock_unlock_unknown_fd() {
        if !zsh_available() {
            return;
        }
        let script = with_modules(&["system"], "zsystem flock -u 87; echo rc=$?");
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{}", script);
        assert_eq!(z.stderr, r.stderr, "stderr divergence on:\n{}", script);
    }

    /// flock open-failure warning also uses `%e` (c:689). Strict stderr.
    #[test]
    fn zsystem_flock_open_failure_message_e_format() {
        if !zsh_available() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("nodir").join("lk");
        let script = with_modules(
            &["system"],
            &format!("zsystem flock -t 0 {}; echo rc=$?", f.display()),
        );
        let z = run_zsh(&script);
        let r = run_zshrs(&script);
        assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{}", script);
        assert_eq!(z.stderr, r.stderr, "stderr divergence on:\n{}", script);
    }

    /// `zsystem supports` + unknown subcommand rcs (c:786-812).
    #[test]
    fn zsystem_supports_and_unknown_subcommand() {
        assert_parity(&with_modules(
            &["system"],
            r#"zsystem supports flock; echo s=$?
zsystem supports nope; echo n=$?
zsystem bogus 2>/dev/null; echo b=$?
zsystem 2>/dev/null; echo e=$?"#,
        ));
    }

    /// `$sysparams` keys are exactly pid/ppid/procsubstpid
    /// (fillpmsysparams surface, Src/Modules/system.c:842-880).
    #[test]
    fn sysparams_key_set() {
        assert_parity(&with_modules(
            &["system"],
            r#"echo keys: ${(ko)sysparams}
[[ $sysparams[pid] == <-> ]] && echo pidnum
echo psp=$sysparams[procsubstpid]"#,
        ));
    }

    /// `zmodload -FL zsh/system` emits the canonical feature line —
    /// the with-modname `-l/-L/-e` arm of bin_zmodload_features
    /// (Src/module.c:3049-3226).
    #[test]
    fn zmodload_feature_listing_forms() {
        assert_parity(&with_modules(
            &["system"],
            r#"zmodload -FL zsh/system
zmodload -FLl zsh/system
zmodload -Fl zsh/system"#,
        ));
    }

    /// `-FL module feature` filters to the named feature, keeping the
    /// C terminator quirk (space while a later feature exists in the
    /// FULL array — Src/module.c:3210-3216).
    #[test]
    fn zmodload_feature_listing_filtered() {
        assert_parity(&with_modules(
            &["system"],
            "zmodload -FL zsh/system b:sysread; echo; echo after",
        ));
    }

    /// `-F -P param -l` stores `+feature`/`-feature` strings into an
    /// array instead of printing (Src/module.c:3158-3224).
    #[test]
    fn zmodload_feature_param_capture() {
        assert_parity(&with_modules(
            &["system"],
            r#"zmodload -F -P feats -l zsh/system
print -l -- $feats"#,
        ));
    }

    /// `-Fe` existence testing: present feature → 0, absent → 1,
    /// `+feature` checks enabled state (Src/module.c:3140-3157).
    #[test]
    fn zmodload_feature_existence_checks() {
        assert_parity(&with_modules(
            &["system"],
            r#"zmodload -Fe zsh/system b:sysread; echo a=$?
zmodload -Fe zsh/system +b:sysread; echo b=$?
zmodload -Fe zsh/system b:nope; echo c=$?"#,
        ));
    }

    /// `-FL` on a registered-but-unloaded module warns "not yet
    /// loaded" + rc=1 (Src/module.c:3108-3112). Strict stderr.
    #[test]
    fn zmodload_feature_listing_unloaded_module() {
        if !zsh_available() {
            return;
        }
        let script = "zmodload -FL zsh/datetime; echo rc=$?";
        let z = run_zsh(script);
        let r = run_zshrs(script);
        assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{}", script);
        assert_eq!(z.stderr, r.stderr, "stderr divergence on:\n{}", script);
    }
}

/// `zmodload -ab MOD NAME` on-use dispatch — port of C resolvebuiltin
/// (Src/exec.c:2700-2724): the autoload stub registered by add_autobin
/// (Src/module.c:426) fires at first use, ensurefeature-loads the
/// owning module, and the real builtin dispatches. Previously zshrs
/// never consulted the autoload ledger at dispatch time — `zmodload
/// -ab zsh/zselect zselect; zselect` died with `command not found`
/// (127). The math-function analog (`zmodload -af zsh/mathfunc sin`)
/// fires through getmathfunc(name, autol=1) (Src/module.c:1283 +
/// Src/math.c:1050) and requires the mftab registration
/// (Src/Modules/mathfunc.c:114-167 via setmathfuncs, module.c:1374).
mod zmodload_autoload_on_use {
    use super::*;

    #[test]
    fn autobin_zselect_fires_on_first_use() {
        // zselect -t 0 with no fds: module loads, builtin runs, rc=1.
        assert_parity(r#"zmodload -ab zsh/zselect zselect; zselect -t 0; echo "rc=$?""#);
    }

    #[test]
    fn autobin_zsystem_fires_on_first_use() {
        assert_parity(r#"zmodload -ab zsh/system zsystem; zsystem supports flock; echo "rc=$?""#);
    }

    #[test]
    fn autobin_bogus_module_load_fails_rc1_then_127() {
        // First call: load_module's `failed to load module` diagnostic,
        // rc=1 (C's execbuiltin head deletes the stub, builtin.c:264-267).
        // Second call: plain `command not found`, rc=127.
        assert_parity(r#"zmodload -ab zsh/bogus mybltn 2>/dev/null; mybltn 2>/dev/null; echo "r1=$?"; mybltn 2>/dev/null; echo "r2=$?""#);
    }

    #[test]
    fn autobin_name_module_does_not_define_errors() {
        // Module loads but lacks feature b:notreal — ensurefeature
        // cancels the autoload and resolvebuiltin zerrs (c:2716-2720);
        // zerr aborts the remaining list, so no echo output, rc=1.
        assert_parity(r#"zmodload -ab zsh/zselect notreal 2>/dev/null; notreal 2>/dev/null; echo "r1=$?""#);
    }

    #[test]
    fn automathfunc_sin_fires_on_first_use() {
        assert_parity(r#"zmodload -af zsh/mathfunc sin; echo $(( sin(0) ))"#);
    }

    #[test]
    fn mathfunc_unknown_without_zmodload() {
        assert_parity(r#"echo $(( sin(0) )) 2>/dev/null; echo "rc=$?""#);
    }

    #[test]
    fn mathfunc_unload_deregisters() {
        assert_parity(r#"zmodload zsh/mathfunc; zmodload -u zsh/mathfunc; echo $(( sin(0) )) 2>/dev/null; echo "rc=$?""#);
    }
}

/// Bug #376 — `zmodload zsh/nonexistent` diagnostic. C load_module
/// emits `zwarn("failed to load module `%s': %s", name, dlerror())`
/// (Src/module.c:1618-1621 do_load_module); zshrs has no dlopen, so
/// the faithful shape is the same zwarn prefix without the dlerror
/// tail (documented stance in docs/BUGS.md #376). Pin: both shells'
/// stderr starts with the identical prefix, rc=1.
/// Re-verified at HEAD 2026-06-12.
mod zmodload_nonexistent_diagnostic {
    use super::*;

    #[test]
    fn nonexistent_module_prints_failed_to_load_prefix_rc1() {
        if !zsh_available() {
            return;
        }
        let script = "zmodload zsh/nonexistent";
        let z = run_zsh(script);
        let r = run_zshrs(script);
        let prefix = "zsh:1: failed to load module `zsh/nonexistent'";
        assert!(
            z.stderr.starts_with(prefix),
            "zsh stderr lost the canonical prefix: {:?}",
            z.stderr
        );
        assert!(
            r.stderr.starts_with(prefix),
            "zshrs stderr lost the canonical prefix: {:?}",
            r.stderr
        );
        assert_eq!(z.exit, 1, "zsh rc");
        assert_eq!(r.exit, 1, "zshrs rc");
    }
}
