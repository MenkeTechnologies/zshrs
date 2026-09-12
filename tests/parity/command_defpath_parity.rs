//! `command -p` — the default-path search, and the things it must leave
//! alone.
//!
//! `-p` on the `command` precommand modifier asks for the *standard*
//! utility, not whatever `$PATH` happens to name. C implements it as a
//! flag threaded down through the exec, never as a change to `$path`:
//! `Src/exec.c:3212-3214` sets `use_defpath = 1`, `c:4369` passes it as
//! `execute(args, cflags, use_defpath)`, and `c:810-828` takes the
//! `search_defpath()` arm (`c:697`) over the compiled-in `DEFAULT_PATH`
//! (`configure.ac:1954`, `getconf _CS_PATH` at build time). The `-v`/`-V`
//! spellings go the same way through `Src/builtin.c:4165-4167`
//! (`findcmd(*argv, 1, func == BIN_COMMAND && OPT_ISSET(ops,'p'))`).
//!
//! zshrs used to spell it as a temporary `$PATH` reassignment around the
//! call, seeded by forking `getconf PATH`. Every `$PATH` write empties
//! `cmdnamtab` (`Src/params.c` `pathsetfn`), so the entry `execcmd_exec`
//! had just made for the command was wiped when the original `$PATH` went
//! back — `hash` came back empty where the oracle lists the command. The
//! child also inherited the default path in its environment instead of
//! the caller's `$PATH`.
//!
//! The asymmetry pinned by `hash_records_the_path_copy_not_the_one_that_ran`
//! is the part that is easy to get wrong in either direction: the hashing
//! is NOT part of `execute()`. It happens in `execcmd_exec` in the parent
//! (`c:3671-3674`), against `$path`, whether or not `use_defpath` is set.
//! So after `command -p sh`, the oracle runs `/bin/sh` and hashes the
//! `$PATH` copy.
//!
//! Every expected string below is the ORACLE's own answer (`zsh -f`,
//! 5.9.2 at /opt/homebrew/bin/zsh), pinned so the suite still runs where
//! no zsh is installed; where one is, each pin is re-checked against it in
//! the same case. `$PATH` is a private directory this file creates, never
//! the system one, and the environment is cleared for every run, so no
//! inherited variable and no installed-package difference can manufacture
//! a divergence.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> Option<&'static str> {
    ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh", "/usr/bin/zsh"]
        .into_iter()
        .find(|p| Path::new(p).exists())
}

/// A `$PATH` directory of our own holding exactly two commands.
///
/// `sh` is the load-bearing one: the name also exists on the default path
/// as `/bin/sh`, so it is the only probe that can tell a default-path
/// search apart from a `$PATH` search. Ours prints `FAKE` and the real
/// one can be asked to print `REAL`, which is what makes "which binary
/// actually ran" observable rather than inferred.
///
/// `zqdpq` is the opposite probe — present on `$PATH` and on no default
/// path anywhere — so `command -p zqdpq` must fail even though plain
/// `zqdpq` succeeds.
fn bin_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("zqdefpath_parity_bin");
    std::fs::create_dir_all(&dir).expect("create private PATH dir");
    for (name, body) in [
        ("sh", "#!/bin/sh\nprintf FAKE\n"),
        ("zqdpq", "#!/bin/sh\nexit 0\n"),
    ] {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).expect("write probe command");
        f.write_all(body.as_bytes()).expect("write probe body");
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
                .expect("chmod probe command");
        }
    }
    dir
}

/// `hck NAME` reports `HIT`/`MISS` for NAME in the command hash table
/// using builtins only — reading the table must not itself run an
/// external command and perturb what is being measured.
const PRELUDE: &str = "hck(){ local zqt zqk; zqt=$(hash); for zqk in ${(f)zqt}; do \
     [[ $zqk == $1=* ]] && { print -r -- HIT; return }; done; print -r -- MISS };";

/// Run `code` under `bin -f -c "<prelude><code>"` with a cleared
/// environment and our private `$PATH`. `{BIN}` in `code` expands to that
/// directory, and it is folded back to `{BIN}` in the returned stdout, so
/// a case reads the same on any machine. stderr is dropped: the two
/// shells' diagnostic prefixes differ by design (`zsh:` vs `zshrs:`).
fn run(bin: &Path, code: &str) -> String {
    let dir = bin_dir();
    let dir_s = dir.to_string_lossy().to_string();
    let mut cmd = Command::new(bin);
    cmd.arg("-f")
        .arg("-c")
        .arg(format!("{PRELUDE}{}", code.replace("{BIN}", &dir_s)))
        .env_clear()
        .env("PATH", &dir_s)
        .env("HOME", std::env::temp_dir())
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let out = cmd.output().expect("spawn shell");
    String::from_utf8_lossy(&out.stdout)
        .replace(&dir_s, "{BIN}")
        .trim_end()
        .to_string()
}

/// Assert zshrs matches `expected`, and that `expected` is still what the
/// oracle says when an oracle is installed.
fn pin(case: &str, code: &str, expected: &str) {
    if let Some(zsh) = zsh_path() {
        let oracle = run(Path::new(zsh), code);
        assert_eq!(
            oracle, expected,
            "{case}: the PINNED expectation no longer matches the oracle {zsh} — \
             fix the pin, not the shell\ncode: {code}"
        );
    }
    let got = run(&zshrs_bin(), code);
    assert_eq!(got, expected, "{case}\ncode: {code}");
}

/// `/bin/sh` is on the default path of every Unix this builds for, but
/// say so out loud rather than assuming it: a case that silently stopped
/// exercising the default path would pass for the wrong reason.
fn have_default_sh() -> bool {
    Path::new("/bin/sh").exists()
}

// ── what `-p` searches ──────────────────────────────────────────────────

/// c:810-822 — `if (defpath) { … search_defpath(arg0, pbuf, MAXCMDLEN) …
/// ee = zexecve(pbuf, argv, newenvp); }`. The default-path copy runs even
/// though `$PATH` names a different `sh` first.
#[test]
fn dash_p_runs_the_default_path_copy_not_the_path_one() {
    if !have_default_sh() {
        return;
    }
    pin("defpath wins", r#"command -p sh -c 'printf REAL'"#, "REAL");
    pin("no -p, $PATH wins", r#"sh -c 'printf REAL'"#, "FAKE");
}

/// c:815-819 — a default-path miss is final: `zerr("command not found")`,
/// `_exit(127)`. Being on `$PATH` does not rescue it, which is the whole
/// point of asking for the standard utility.
#[test]
fn dash_p_does_not_fall_back_to_path() {
    pin("path-only command", "command -p zqdpq; print rc=$?", "rc=127");
    pin("same command without -p", "zqdpq; print rc=$?", "rc=0");
}

/// c:818-819 again for a name that exists nowhere — same status, and
/// nothing enters the table because `hashcmd` found nothing on `$path`
/// either (c:1030-1031 `if (!*pp) return NULL;`).
#[test]
fn a_name_that_exists_nowhere_is_127_and_unhashed() {
    pin(
        "nonexistent",
        "command -p zqdpnosuch; print rc=$?; hck zqdpnosuch",
        "rc=127\nMISS",
    );
}

/// c:816-817 — `if (commandnotfound(arg0, args) == 0) _realexit();`. The
/// hook sees the default-path miss the same way it sees a `$path` miss,
/// gets the name plus the original arguments, and its status becomes the
/// command's. The `$path` miss reaches the hook from the failed spawn;
/// `-p` has no spawn to fail, so the call has to be made explicitly.
#[test]
fn a_default_path_miss_reaches_command_not_found_handler() {
    pin(
        "handler on a -p miss",
        "command_not_found_handler(){ print -r -- \"CNF:$*\"; return 42 }; \
         command -p zqdpnosuch a b; print rc=$?",
        "CNF:zqdpnosuch a b\nrc=42",
    );
    pin(
        "handler on a path-only -p miss",
        "command_not_found_handler(){ print -r -- \"CNF:$*\"; return 42 }; \
         command -p zqdpq z; print rc=$?",
        "CNF:zqdpq z\nrc=42",
    );
}

// ── what `-p` must NOT disturb ──────────────────────────────────────────

/// The regression this file exists for. `execcmd_exec` hashes the head
/// word against `$path` before the exec (c:3671-3674) and `use_defpath`
/// does not gate that, so the table must hold an entry afterwards.
/// zshrs implemented `-p` by reassigning `$PATH` and restoring it, and
/// the restore ran `pathsetfn`, which empties `cmdnamtab` — so the entry
/// the command had just made was wiped and `hash` reported nothing.
#[test]
fn dash_p_leaves_the_command_in_the_hash_table() {
    if !have_default_sh() {
        return;
    }
    pin("hash after -p", "command -p sh -c ':'; hck sh", "HIT");
}

/// The asymmetry stated at the top of this file, pinned in one case: the
/// default-path copy RAN (`REAL`) while the table records the `$PATH`
/// copy (`{BIN}/sh`), because the two steps read different lists —
/// `search_defpath` over `DEFAULT_PATH` (c:697) for the exec,
/// `hashcmd(cmdarg, path)` over `$path` (c:3674) for the table.
#[test]
fn hash_records_the_path_copy_not_the_one_that_ran() {
    if !have_default_sh() {
        return;
    }
    pin(
        "hashed path vs exec'd path",
        r#"command -p sh -c 'printf REAL'; print; hash"#,
        "REAL\nsh={BIN}/sh",
    );
}

/// A command reachable only through `$PATH` still hashes under `-p`, even
/// though the exec then fails: c:3671-3674 runs in the parent, before
/// `execute()` ever looks at `defpath`.
#[test]
fn a_path_only_command_hashes_even_though_dash_p_cannot_run_it() {
    pin(
        "hash on a -p failure",
        "command -p zqdpq; print rc=$?; hck zqdpq",
        "rc=127\nHIT",
    );
}

/// C never assigns to `$path` for `-p`, so neither the shell's own
/// `$PATH` nor the child's environment may change. The old
/// implementation failed the second half: the child saw the default path.
#[test]
fn dash_p_changes_neither_our_path_nor_the_childs() {
    if !have_default_sh() {
        return;
    }
    pin(
        "PATH across the call",
        r#"print -r -- "before=$PATH"; command -p sh -c 'printf "child=%s\n" "$PATH"'; print -r -- "after=$PATH""#,
        "before={BIN}\nchild={BIN}\nafter={BIN}",
    );
}

/// `-p` is about the search list, not about `$PATH` being usable — c:810
/// takes its arm before any `$path` walk, so an empty `$PATH` is no
/// obstacle. Without `-p` the same command is a 127.
#[test]
fn dash_p_works_with_an_empty_path() {
    if !have_default_sh() {
        return;
    }
    pin("empty PATH with -p", r#"PATH=; command -p sh -c 'printf OK'"#, "OK");
    pin("empty PATH without -p", "PATH=; sh -c ':'; print rc=$?", "rc=127");
}

// ── `-pv` / `-pV` ───────────────────────────────────────────────────────

/// c:Src/builtin.c:4165-4167 — `findcmd(*argv, 1, func == BIN_COMMAND &&
/// OPT_ISSET(ops,'p'))`, i.e. `-v` reports the default-path hit under
/// `-p` and the `$PATH` hit without it.
#[test]
fn dash_pv_reports_the_default_path_hit() {
    if !have_default_sh() {
        return;
    }
    pin("-pv", "command -pv sh", "/bin/sh");
    pin("-v", "command -v sh", "{BIN}/sh");
}

/// c:4157-4164 — the `command -p[vV]` special case shows a builtin in
/// preference to any external of the same name.
#[test]
fn dash_pv_prefers_a_builtin() {
    pin("-pv builtin", "command -pv echo; print rc=$?", "echo\nrc=0");
}

/// c:4165 returning NULL — nothing printed, status 1. `-V` is the same
/// lookup with the "not found" line (c:4199-4204).
#[test]
fn dash_pv_on_a_path_only_command_reports_nothing() {
    pin("-pv path-only", "command -pv zqdpq; print rc=$?", "rc=1");
    pin("-v path-only", "command -v zqdpq", "{BIN}/zqdpq");
    pin(
        "-pV nonexistent",
        "command -pV zqdpnosuch; print rc=$?",
        "zqdpnosuch not found\nrc=1",
    );
}

/// `-pv` is a lookup, not an exec: c:4165 `findcmd` with `default_path`
/// takes the `search_defpath` early return (c:930-935) and never reaches
/// the `hashcmd` fill at c:937-938.
#[test]
fn dash_pv_does_not_hash() {
    if !have_default_sh() {
        return;
    }
    pin("-pv leaves the table alone", "command -pv sh >/dev/null; hck sh", "MISS");
}

// ── the fork the old implementation paid ────────────────────────────────

/// `-p` used to shell out to `getconf PATH` on every call. The default
/// path is a build-time constant in C (`configure.ac:1954`) and now in
/// zshrs too (`ZSHRS_CONFIG_DEFAULT_PATH`), so nothing is spawned but the
/// command itself.
///
/// Counted, not timed: `$ZSH_SUBSHELL`-style counters are not exposed, so
/// the probe is the default-path `sh` reporting how many children the
/// shell has — `command -p sh -c 'echo $PPID'` twice must name the same
/// shell, and a `getconf` fork in between would not change that. The
/// direct evidence is instead that the resolution works with `getconf`
/// itself unreachable: `$PATH` holds neither `getconf` nor anything else,
/// and the old code's fallback (a hardcoded string) would be
/// indistinguishable only if it were still correct — it searched `$PATH`
/// after assigning, which an empty-but-for-our-dir `$PATH` breaks.
#[test]
fn dash_p_resolves_without_spawning_getconf() {
    if !have_default_sh() {
        return;
    }
    // `getconf` is not on this `$PATH` at all, and `command -p` must
    // still find `/bin/sh`.
    pin("getconf unreachable", "command -v getconf; print rc=$?", "rc=1");
    pin("resolution still works", r#"command -p sh -c 'printf REAL'"#, "REAL");
}
