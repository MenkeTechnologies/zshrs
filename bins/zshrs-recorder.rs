//! `zshrs-recorder` — Plugin-Framework-Agnostic State-Modification
//! Recorder binary. See docs/RECORDER.md.
//!
//! Independent of `zshrs`. Built only with `--features recorder`
//! (Cargo.toml `required-features` enforces this). Self-contained main:
//! parses its own arg set, brings up a `zsh::exec::ShellExecutor`,
//! sources the requested file (or `${ZDOTDIR:-$HOME}/.zshrc` by
//! default), then exits. No interactive loop, no completion engine, no
//! delegation to `zshrs_main()`.
//!
//! Lifecycle:
//!   1. Parse `--file PATH` (and friends) from argv.
//!   2. `recorder::enable()` flips the global. Every state-mutating
//!      dispatcher in `src/vm_helper` checks this and emits a record.
//!   3. `recorder::install_atexit()` registers the libc atexit hook so
//!      the end-of-run summary + shard write still fire when the
//!      shell exits via `std::process::exit` (skipping Rust Drop).
//!   4. Build a fresh `ShellExecutor` and source the requested file.
//!   5. Process exits naturally; atexit hook prints summary and writes
//!      the canonical rkyv shard itself. No daemon involved.

#![cfg(feature = "recorder")]

use std::path::PathBuf;
use std::process::ExitCode;

use zsh::vm_helper::ShellExecutor;

const USAGE: &str = "\
zshrs-recorder — capture every state-mutating dispatcher fire during
shell init and write it to ~/.zshrs/images/*-recorder.rkyv, the shard
zshrs replays at startup instead of sourcing the rc files. Single-shot;
no daemon required.

USAGE
    zshrs-recorder [OPTIONS]

OPTIONS
    -f, --file PATH    Source PATH instead of the user's startup chain.
                       Use this to test recorder coverage on a small
                       script without dragging in the real .zshrc.
    -o, --output PATH  Write the captured bundle as JSON to PATH (in
                       addition to the shard, or as the sole output
                       under --dry-run). Useful for
                       post-mortem inspection / diffing two runs.
        --shell-id ID  Override the bundle's shell_id (default `zshrs`).
                       Used for federation testing — let a recorder
                       impersonate `bash` / `fish` etc. against the
                       same catalog. See docs/SHELL_IDS.md.
        --quiet        Suppress the per-event `Captured KIND ...` stderr
                       firehose. Summary footer + tracing log still fire.
        --json         Emit the end-of-run summary as one JSON line on
                       stdout instead of multi-line human text on stderr.
                       Lets scripts pipe straight to `jq`.
        --no-prewarm   Skip the end-of-run autoload bytecode pass. That
                       pass compiles every `_*` completer on the
                       recorded $fpath into ~/.zshrs/autoloads.rkyv so
                       the first `ls -<TAB>` of a later shell is an O(1)
                       shard probe instead of a parse + compile.
        --dry-run      Capture without writing the shard; the existing
                       recording is left untouched. Captured events still
                       print to stderr + log. Used by
                       `tests/recorder_harness.rs` for hermetic runs.
                       Combine with -o PATH to capture the bundle to a
                       file. `--no-daemon` is an alias.
        --help         Print this message and exit.
        --version      Print version and exit.

DEFAULT BEHAVIOR (no --file)
    Sources the full zsh login + interactive startup chain as a real
    `zsh -l -i` would (skipping any file that does not exist):

       1. /etc/zshenv
       2. ${ZDOTDIR:-$HOME}/.zshenv
       3. /etc/zprofile
       4. ${ZDOTDIR:-$HOME}/.zprofile
       5. /etc/zshrc
       6. ${ZDOTDIR:-$HOME}/.zshrc
       7. /etc/zlogin
       8. ${ZDOTDIR:-$HOME}/.zlogin

    Captures every alias, function, export, path/fpath edit, hash -d,
    zstyle, bindkey, compdef, zmodload, setopt, trap, sched, source,
    and assignment that fires through the runtime AOP layer across all
    eight files. $ZDOTDIR is re-resolved before each user-side file so
    a /etc-side script setting it propagates correctly.

OUTPUT
    Realtime stderr   `Captured KIND NAME[=value], file: PATH:LINE [(fn)]`
    End-of-run        Summary stats (counts per kind + elapsed_ms).
    Shard             ~/.zshrs/images/{hash8}-recorder.rkyv, replaced whole.
    Log               Same lines mirrored via tracing::info to the zshrs
                      log file.
";

struct Args {
    file: Option<PathBuf>,
    dry_run: bool,
    output: Option<PathBuf>,
    shell_id: Option<String>,
    quiet: bool,
    json: bool,
    no_prewarm: bool,
}

fn parse_args() -> Result<Args, ExitCode> {
    let mut file: Option<PathBuf> = None;
    let mut dry_run = false;
    let mut output: Option<PathBuf> = None;
    let mut shell_id: Option<String> = None;
    let mut quiet = false;
    let mut json = false;
    let mut no_prewarm = false;
    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "-f" | "--file" => match iter.next() {
                Some(p) => file = Some(PathBuf::from(p)),
                None => {
                    eprintln!("zshrs-recorder: --file requires a path");
                    eprintln!();
                    eprintln!("{USAGE}");
                    return Err(ExitCode::from(1));
                }
            },
            "-o" | "--output" => match iter.next() {
                Some(p) => output = Some(PathBuf::from(p)),
                None => {
                    eprintln!("zshrs-recorder: --output requires a path");
                    return Err(ExitCode::from(1));
                }
            },
            "--shell-id" => match iter.next() {
                Some(s) => shell_id = Some(s),
                None => {
                    eprintln!("zshrs-recorder: --shell-id requires an identifier");
                    return Err(ExitCode::from(1));
                }
            },
            "--quiet" => quiet = true,
            "--json" => json = true,
            "--dry-run" | "--no-daemon" => dry_run = true,
            "--no-prewarm" => no_prewarm = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Err(ExitCode::SUCCESS);
            }
            "--version" => {
                println!("zshrs-recorder {}", env!("CARGO_PKG_VERSION"));
                return Err(ExitCode::SUCCESS);
            }
            other => {
                eprintln!("zshrs-recorder: unknown argument: {other}");
                eprintln!();
                eprintln!("{USAGE}");
                return Err(ExitCode::from(2));
            }
        }
    }
    Ok(Args {
        file,
        dry_run,
        output,
        shell_id,
        quiet,
        json,
        no_prewarm,
    })
}

fn zdotdir() -> PathBuf {
    let zd = std::env::var_os("ZDOTDIR").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    zd.or(home).unwrap_or_else(|| PathBuf::from("."))
}

/// The eight-file zsh login + interactive startup chain. Mirrors the
/// `zsh(1)` STARTUP/SHUTDOWN FILES section (and `bins/zshrs.rs ::
/// source_startup_files`). Returned in source order; non-existent
/// entries stay in the list — the caller skips them silently. $ZDOTDIR
/// is resolved at the moment this function is called; in practice the
/// recorder runs it once after it has already entered the source loop
/// for the previous file, so /etc/zshenv gets a chance to set ZDOTDIR
/// before $ZDOTDIR-targeting files resolve.
fn login_chain() -> [PathBuf; 8] {
    let zd = zdotdir();
    [
        PathBuf::from(zsh::global_rc::global_rc_path("/etc/zshenv")),
        zd.join(".zshenv"),
        PathBuf::from(zsh::global_rc::global_rc_path("/etc/zprofile")),
        zd.join(".zprofile"),
        PathBuf::from(zsh::global_rc::global_rc_path("/etc/zshrc")),
        zd.join(".zshrc"),
        PathBuf::from(zsh::global_rc::global_rc_path("/etc/zlogin")),
        zd.join(".zlogin"),
    ]
}

/// Put a terminal of the recording's own on fd 0 and return its name.
///
/// The recorded files run in the interactive login shell's context, and
/// that shell has a terminal: `$TTY` is set and `tty` names it. Without
/// one, a config's `ZPWR_TTY=$(tty)` recorded `not a tty` and every
/// replayed shell inherited it. A fresh pty gives the recording a `$TTY`
/// path no other process has, so the recorder can mark every value
/// derived from it and the replay can substitute its own `$TTY`
/// (`recorder::set_recording_tty`). Must run before the executor is
/// built: `init_io` reads `ttyname(0)` into `$TTY` (Src/init.c:624-626).
///
/// The master stays open for the life of the process — closing it would
/// hang up the slave and it would stop being a tty. A thread drains
/// whatever the files write to the terminal so a write never blocks, and
/// a run of EOF characters is queued so a `read` from the terminal
/// returns at once, as it did on `/dev/null`.
fn attach_recording_tty() -> Option<String> {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: openpty fills two descriptors; the null termios/winsize take
    // the defaults.
    let rc = unsafe {
        libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
    };
    if rc != 0 {
        return None;
    }
    // SAFETY: `slave` is a valid descriptor from openpty.
    let name = unsafe {
        let p = libc::ttyname(slave);
        (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
    }?;
    // SAFETY: plain descriptor moves; children must not inherit the master.
    unsafe {
        libc::dup2(slave, 0);
        libc::close(slave);
        libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
    }
    // SAFETY: the master is owned by this File from here on.
    let mut writer = unsafe { std::fs::File::from_raw_fd(master) };
    let mut reader = writer.try_clone().ok()?;
    let _ = writer.write_all(&[0x04; 64]);
    std::mem::forget(writer);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
    });
    Some(name)
}

/// Variables a new terminal's login shell gets from its session rather
/// than from shell configuration; [`scrub_environment`] keeps only these.
const SESSION_ENV: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "COLORTERM",
    "LANG",
    "TMUX",
    "TMUX_PANE",
    "SSH_AUTH_SOCK",
    "SSH_CONNECTION",
    "SSH_CLIENT",
    "SSH_TTY",
    "DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
    "__CF_USER_TEXT_ENCODING",
    "ZDOTDIR",
];

/// Start the recording from the environment a new terminal's login shell
/// starts with, not from the shell that ran `zshrs-recorder`.
///
/// The recording stands in for the startup files in every later shell,
/// so it must hold what the files set given a fresh session. Run from a
/// zpwr shell, the recorder inherited that shell's ~180 exported `ZPWR_*`
/// variables; `.zpwr_re_env.sh`'s `[[ -z $ZPWR_EXA_COMMAND ]] && export
/// ZPWR_EXA_COMMAND=…` then assigned nothing, nothing was recorded, and a
/// shell replayed in a new terminal had no `$ZPWR_EXA_COMMAND` —
/// `zpwrClearList` listed nothing. Keeps [`SESSION_ENV`], `LC_*`, `XDG_*`
/// and `ZSHRS_*` (the recorder's own settings), and resets `PATH` to the
/// system default (`confstr(_CS_PATH)`) that `/etc/zprofile` builds on.
fn scrub_environment() {
    let keep = |name: &str| {
        SESSION_ENV.contains(&name) || ["LC_", "XDG_", "ZSHRS_"].iter().any(|p| name.starts_with(p))
    };
    for (name, _) in std::env::vars_os() {
        if let Some(n) = name.to_str() {
            if !keep(n) {
                std::env::remove_var(n);
            }
        }
    }
    let mut buf = vec![0u8; 1024];
    // SAFETY: confstr writes at most buf.len() bytes, NUL-terminated.
    let n = unsafe { libc::confstr(libc::_CS_PATH, buf.as_mut_ptr().cast(), buf.len()) };
    if n > 0 && n <= buf.len() {
        let path = String::from_utf8_lossy(&buf[..n - 1]).into_owned();
        std::env::set_var("PATH", path);
    }
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(code) => return code,
    };
    scrub_environment();

    // Make sure ~/.zshrs exists with the default config files BEFORE
    // log init so the just-seeded `[log] level` in zshrs-recorder.toml
    // can be picked up on first run. Idempotent — never overwrites a
    // user-edited file. Same call every binary makes, so whichever
    // runs first does the seeding for the rest.
    if let Ok(paths) = zshrs_daemon::paths::CachePaths::resolve() {
        let _ = paths.ensure_dirs();
        let _ = paths.ensure_default_configs();
    }

    // Init logging FIRST so every recorder event reaches the recorder
    // log file. Separate from `zshrs.log` (shell) and
    // `zshrs-daemon.log` (daemon) — three processes, three logs, no
    // interleaved tracing output.
    zsh::log::init_named("zshrs-recorder.log");

    zsh::recorder::enable();
    if args.dry_run {
        zsh::recorder::set_no_write(true);
    }
    if args.quiet {
        zsh::recorder::set_quiet(true);
    }
    if args.json {
        zsh::recorder::set_json_summary(true);
    }
    if let Some(sid) = args.shell_id {
        zsh::recorder::set_shell_id_override(Some(sid));
    }
    if let Some(out) = args.output {
        zsh::recorder::set_output_path(Some(out.display().to_string()));
    }
    // libc atexit covers the `std::process::exit` paths inside builtins
    // (`exit`, fatal error sites). Without this, summary + shard write
    // would only fire on natural fall-through from `main` — which is
    // not how shell scripts usually terminate.
    zsh::recorder::install_atexit();

    zsh::recorder::set_recording_tty(attach_recording_tty());
    let mut executor = ShellExecutor::new();
    // `$$`. `zsh_main`'s `setupvals` sets it (Src/init.c:1227); the
    // recorder never runs that, and every `$$`-derived name — zpwr's
    // `.temp$$` files, zconvey's PID — was recorded with 0. The real pid
    // is unique, so the recorder can mark it for the replay to substitute.
    zsh::ported::params::mypid.store(std::process::id() as i64, std::sync::atomic::Ordering::Relaxed);
    let mut last_status: i32 = 0;
    // The options and parameters the files change are diffed against this.
    zsh::recorder::mark_baseline();

    if let Some(path) = args.file {
        zsh::recorder::set_startup_files(vec![path.display().to_string()]);
        // Single-file mode (-f / --file): source ONLY that file, no
        // /etc/zshenv, no .zshenv chain. Used by tests + ad-hoc
        // recorder runs against a small script.
        let disp = path.display().to_string();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("zshrs-recorder: cannot read {}: {}", disp, e);
                return ExitCode::from(1);
            }
        };
        tracing::info!(file = %disp, "zshrs-recorder: sourcing");
        eprintln!("zshrs-recorder: sourcing {}", disp);
        executor.set_scalar("0".to_string(), disp.clone());
        // `source()` sets C's `scriptfilename` per file; this path runs the
        // text directly, so set it here or every event is attributed to no file.
        zsh::ported::utils::set_scriptfilename(Some(disp.clone()));
        last_status = executor.execute_script(&content).unwrap_or_else(|e| {
            eprintln!("zshrs-recorder: {}: {}", disp, e);
            1
        });
    } else {
        // Default mode: walk the full eight-file zsh login chain. Each
        // existing file is sourced in order; missing files are skipped
        // silently (matches how a real `zsh -l -i` boots when
        // /etc/zprofile etc. don't exist on the host). $0 is set to
        // each file as it's sourced so introspection in those scripts
        // sees the right name.
        //
        // The chain is the one an interactive login shell reads, and the
        // files test for it: a p10k config returns before defining a
        // single POWERLEVEL9K_* parameter under `[[ ! -o monitor ]]`.
        // Set the options `zsh -l -i` starts with. `zle` and `shinstdin`
        // stay off — stdin is not a terminal, as for `zsh -i </dev/null`.
        // The baseline was taken first, so none of these reach the shard;
        // they are also on the recorder's never-replayed option list.
        for opt in ["interactive", "loginshell", "monitor"] {
            zsh::ported::options::opt_state_set(opt, true);
        }
        zsh::recorder::set_startup_files(login_chain().iter().map(|p| p.display().to_string()).collect());
        for path in login_chain() {
            if !path.exists() {
                continue;
            }
            let disp = path.display().to_string();
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(file = %disp, error = %e, "skipping unreadable startup file");
                    continue;
                }
            };
            tracing::info!(file = %disp, "zshrs-recorder: sourcing");
            eprintln!("zshrs-recorder: sourcing {}", disp);
            executor.set_scalar("0".to_string(), disp.clone());
            // `source()` sets C's `scriptfilename` per file; this path runs the
            // text directly, so set it here or every event is attributed to no file.
            zsh::ported::utils::set_scriptfilename(Some(disp.clone()));
            last_status = executor.execute_script(&content).unwrap_or_else(|e| {
                eprintln!("zshrs-recorder: {}: {}", disp, e);
                1
            });
        }
    }

    // Deferred init. A real shell reaches its first prompt here and
    // keeps loading: `precmd` hooks run, then every due `sched` entry.
    // zinit turbo (`wait''`) and similar deferred loaders hang their
    // plugins off exactly that — zinit loads ONE plugin per scheduler
    // pass — so a recording that stops at the end of .zshrc misses most
    // of the environment (37 of ~2100 aliases on a zpwr config). Run
    // precmd once, as `preprompt` does, then fire every pending `sched`
    // entry pass by pass. Stop when nothing is pending, or after IDLE
    // consecutive passes that define nothing new (a loader that keeps
    // re-arming itself with an empty queue). IDLE is generous because a
    // run of plugins can legitimately add nothing — zpwr opens with ~15
    // completion-only snippets. MAX_PASSES is the backstop.
    {
        const MAX_PASSES: usize = 4096;
        const IDLE: usize = 32;
        zsh::ported::exec::install_session_executor(&mut executor);
        zsh::fusevm_bridge::with_session_context(|| {
            zsh::ported::utils::callhookfunc("precmd", None, 1, std::ptr::null_mut());
            let mut idle = 0;
            for _ in 0..MAX_PASSES {
                let before = zsh::recorder::definition_count();
                if zsh::recorder::run_pending_sched() == 0 {
                    break;
                }
                idle = if zsh::recorder::definition_count() == before { idle + 1 } else { 0 };
                if idle == IDLE {
                    break;
                }
            }
            // Read the end state inside the same context the drain ran in.
            zsh::recorder::capture_end_state();
        });
    }

    // The init chain has finished, so every fpath dir the user's
    // config registered is now on `$fpath` — and the shell is idle,
    // which is the whole reason this pass lives here rather than in
    // `compinit`: `parse()` walks process-global lexer state, and
    // compiling 46k completers beside a live ZLE corrupted the prompt
    // when that was tried. Nothing runs after this but the summary and
    // the shard write.
    //
    // Result: the first `ls -<TAB>` in any later shell is an O(1) probe
    // into `~/.zshrs/autoloads.rkyv` instead of a parse + compile of
    // the completer's file.
    if !args.no_prewarm {
        let dirs = zsh::autoload_prewarm::default_dirs();
        let stats = zsh::autoload_prewarm::prewarm_fpath(&dirs);
        if !args.quiet {
            eprintln!(
                "zshrs-recorder: autoload bytecode — {} compiled, {} already fresh, {} unparseable, {:.1} MB, {} ms",
                stats.compiled,
                stats.fresh,
                stats.failed,
                stats.bytes as f64 / (1024.0 * 1024.0),
                stats.elapsed_ms,
            );
        }
    }

    // Summary + shard write, here rather than in the atexit hook: after
    // `main` returns, thread-locals are gone and `tracing` panics.
    zsh::recorder::finalize();

    // Process exits with the last sourced script's status.
    ExitCode::from(last_status as u8)
}
