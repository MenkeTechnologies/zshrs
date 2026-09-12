//! `cmdnamtab` — the command hash table — across the events that fill,
//! read and invalidate it.
//!
//! zsh hashes an external command when it RUNS it: `execcmd_exec` asks
//! `cmdnamtab` for the head word and, on a miss, calls `hashcmd()` to walk
//! `$path` and install the entry before the command executes
//! (`Src/exec.c:3661` + `c:3671-3675`). `execute()` then execs the pathname
//! that node names (`c:831-869`), so an explicit `hash foo=/bin/echo` is
//! what actually runs. Every reader of the table — `hash`, `unhash`,
//! `whence`, `type`, `$commands`, completion — depends on that fill.
//!
//! zshrs reached external commands by a second route that did none of it:
//! a statically-spelled head compiled to a fusevm exec op and arrived at
//! the spawn funnel, which handed the bare word to `Command::new`, i.e. to
//! libc `execvp`. libc did its own `$PATH` walk, so the table stayed empty
//! for the life of the shell and an explicit `hash` entry was ignored.
//! Only a run-time-resolved head (`c=awk; $c …`) went through the ported
//! `execcmd_exec` and hashed, which is why the gap did not show up from
//! the ported side.
//!
//! Every expected string below is the ORACLE's own answer (`zsh -f`,
//! 5.9.2 at /opt/homebrew/bin/zsh), pinned so the suite still runs where
//! no zsh is installed; where one is, each pin is re-checked against it in
//! the same case.
//!
//! The cases run against a private `$PATH` holding two throwaway commands
//! this file creates, never the system one, so no installed-package
//! difference can move an answer; `{BIN}` in an expectation is that
//! directory. The environment is cleared for every run (`env_clear`), so
//! an inherited variable cannot manufacture a divergence.

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

/// A `$PATH` directory of our own: `zqhq` exits 0 silently, `zqhecho`
/// prints its arguments. Two commands are needed because several cases
/// have to hash one and then check that the other is unaffected, and one
/// of them has to be distinguishable by its OUTPUT so the "explicit hash
/// entry is what execs" case can tell which binary ran.
fn bin_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("zqhash_parity_bin");
    std::fs::create_dir_all(&dir).expect("create private PATH dir");
    for (name, body) in [
        ("zqhq", "#!/bin/sh\nexit 0\n"),
        ("zqhq2", "#!/bin/sh\nexit 0\n"),
        ("zqhecho", "#!/bin/sh\nprintf '%s\\n' \"$*\"\n"),
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

// ── the fill ────────────────────────────────────────────────────────────

/// The bug itself: running a bare-name external command must leave it in
/// the table. c:3674 `hn = (HashNode) hashcmd(cmdarg, checkpath)`.
#[test]
fn running_a_bare_name_hashes_it() {
    pin("bare name", "zqhq; hck zqhq", "HIT");
}

/// c:3674 again, one level down: the fill is a property of executing the
/// command, not of the top-level shell, so a function body hashes into
/// the caller's table.
#[test]
fn a_function_body_hashes_into_the_callers_table() {
    pin("function body", "zqf(){ zqhq }; zqf; hck zqhq", "HIT");
}

/// c:3674 behind the `command` precommand modifier — it suppresses the
/// function/alias lookup, not the hash.
#[test]
fn command_modifier_still_hashes() {
    pin("command modifier", "command zqhq; hck zqhq", "HIT");
}

/// Each external command hashes itself; the first one running does not
/// pull in the rest of `$path` (HASH_DIRS is off by default).
#[test]
fn a_second_command_hashes_separately() {
    pin("two commands", "zqhq; hck zqhq2", "MISS");
    pin("two commands", "zqhq; zqhq2; hck zqhq2", "HIT");
}

// ── what must NOT fill it ───────────────────────────────────────────────

/// c:1055 `if ((*arg0 == '/') || !strncmp(arg0, "./", 2) ||
/// !strncmp(arg0, "../", 3)) return NULL;` — a pathname the user spelled
/// out is exec'd directly and never enters the table.
#[test]
fn an_explicit_pathname_does_not_hash() {
    pin("absolute path", "{BIN}/zqhq; hck zqhq", "MISS");
    pin("dot-slash path", "cd {BIN} && ./zqhq; hck zqhq", "MISS");
}

/// c:3655 `if (!hn)` — the hash step is only reached when the head did
/// NOT resolve to a builtin or a shell function.
#[test]
fn builtins_and_functions_do_not_hash() {
    pin("builtin", "echo x >/dev/null; hck echo", "MISS");
    pin("function", "zqfn(){ : }; zqfn; hck zqfn", "MISS");
}

/// c:1069-1070 `if (!*pp) return NULL;` — a name no `$path` entry holds
/// installs nothing, so the table does not accumulate typos.
#[test]
fn a_command_not_found_does_not_hash() {
    pin("not found", "zqnosuch 2>/dev/null; print rc=$?; hck zqnosuch", "rc=127\nMISS");
}

/// c:3671 `strcmp(cmdarg, "..")` — `..` is excluded by name.
#[test]
fn dotdot_is_never_hashed() {
    pin("dotdot", ".. 2>/dev/null; hck ..", "MISS");
}

/// c:3659 `dohashcmd = isset(HASHCMDS)` — the option gates the fill.
#[test]
fn nohashcmds_suppresses_the_fill() {
    pin("nohashcmds", "setopt nohashcmds; zqhq; hck zqhq", "MISS");
}

// ── the table decides what execs ────────────────────────────────────────

/// c:831-835 — a HASHED entry's `cn->u.cmd` is what `execute()` runs, so
/// `hash` can redirect a name. The funnel used to ignore the table and
/// let libc re-derive the path from `$PATH`, running the real command.
#[test]
fn an_explicit_hash_entry_is_what_execs() {
    pin(
        "explicit entry execs",
        "hash zqhq={BIN}/zqhecho; zqhq marker",
        "marker",
    );
}

/// c:877-895 `execute_skip_exec:` — a table entry that will not exec does
/// not end the search; C falls through to a full `$path` walk, so a stale
/// entry still runs the real command.
#[test]
fn a_stale_hash_entry_falls_back_to_the_path_walk() {
    pin(
        "stale entry falls back",
        "hash zqhecho=/zqnonexistent/zqhecho; zqhecho marker; print rc=$?",
        "marker\nrc=0",
    );
}

// ── the readers ─────────────────────────────────────────────────────────

/// c:936-938 in `findcmd` — the lookup consults the table and, on a miss,
/// hashes. Both halves were missing: `whence -p` could not see a HASHED
/// entry (no `$path` directory holds that name), and no lookup ever
/// filled the table the way C's does.
#[test]
fn whence_p_sees_an_explicit_entry_and_hashes_on_its_own() {
    pin(
        "whence -p on explicit entry",
        "hash zqfoo={BIN}/zqhecho; whence -p zqfoo; print rc=$?",
        "{BIN}/zqhecho\nrc=0",
    );
    pin("whence -p hashes", "whence -p zqhq >/dev/null; hck zqhq", "HIT");
}

/// c:4116 `cmdnamtab->printnode(hn, printflags)` — `whence`/`type` print
/// a hashed entry through `printcmdnamnode`, which dispatches on the
/// PRINT_WHENCE_* bits. The port open-coded a bare path print, losing
/// every form but `-L`.
#[test]
fn type_and_whence_w_report_a_hashed_entry() {
    pin(
        "type on hashed entry",
        "hash zqfoo={BIN}/zqhecho; type zqfoo",
        "zqfoo is hashed to {BIN}/zqhecho",
    );
    pin(
        "whence -w on hashed entry",
        "hash zqfoo={BIN}/zqhecho; whence -w zqfoo",
        "zqfoo: hashed",
    );
}

/// c:4254 `scanhashtable(ht, 1, 0, 0, ht->printnode, printflags)` and
/// c:4267 `scanmatchtable(...)` — `hash` and `hash -L` list what running
/// the command installed, and `-m` matches it as a glob. The `-m` arm
/// only ever walked `nameddirtab`, so it matched nothing whatever the
/// command table held.
#[test]
fn hash_lists_and_pattern_matches_what_running_installed() {
    pin("hash listing", "zqhq; hash", "zqhq={BIN}/zqhq");
    pin("hash -L", "zqhq; hash -L", "hash zqhq={BIN}/zqhq");
    pin("hash -m glob", "zqhq; hash -m 'zqh*'", "zqhq={BIN}/zqhq");
}

/// `unhash` needs the entry to be there — pre-fix it reported
/// `no such hash table element` after the command had just run.
#[test]
fn unhash_removes_what_running_installed() {
    pin("unhash after run", "zqhq; unhash zqhq; print rc=$?; hck zqhq", "rc=0\nMISS");
}

/// `$commands` is the table as a parameter.
#[test]
fn commands_parameter_reflects_the_fill() {
    pin("commands[] after run", "zqhq; print -r -- \"[${commands[zqhq]}]\"", "[{BIN}/zqhq]");
}

// ── invalidation ────────────────────────────────────────────────────────

/// c:Src/params.c:5291 `if (t == path) cmdnamtab->emptytable(cmdnamtab)`
/// and c:4240 `ht->emptytable(ht)` for `hash -r` / `rehash`.
#[test]
fn path_assignment_and_hash_r_empty_the_table() {
    pin("hash -r", "zqhq; hash -r; hck zqhq", "MISS");
    pin("rehash", "zqhq; rehash; hck zqhq", "MISS");
    pin("PATH reassignment", "zqhq; PATH={BIN}; hck zqhq", "MISS");
}

// ── subshell isolation, now that there is something to isolate ──────────

/// C forks for `$( … )` and `( … )`, so a command the body hashes dies
/// with the child. zshrs runs both in process over a copy-on-write
/// snapshot of the table; with the table permanently empty this was
/// vacuously true, so these pin it for a table that actually fills.
#[test]
fn a_substitution_cannot_leak_its_fill_to_the_parent() {
    pin("cmdsubst does not leak", "zqx=$(zqhq); hck zqhq", "MISS");
    pin("subshell does not leak", "( zqhq ); hck zqhq", "MISS");
}

/// The other direction: the body starts from the parent's table, so an
/// entry the parent already has is visible inside and needs no second
/// `$path` walk.
#[test]
fn a_substitution_sees_the_parents_entries() {
    pin("cmdsubst sees parent", "zqhq; zqx=$(hck zqhq); print -r -- $zqx", "HIT");
    pin("subshell sees parent", "zqhq; ( hck zqhq )", "HIT");
}
