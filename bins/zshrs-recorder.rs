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
//!      the end-of-run summary + daemon IPC bundle still fire when the
//!      shell exits via `std::process::exit` (skipping Rust Drop).
//!   4. Build a fresh `ShellExecutor` and source the requested file.
//!   5. Process exits naturally; atexit hook prints summary, ships the
//!      bundle to `zshrs-daemon` via `recorder_ingest`, returns.

#![cfg(feature = "recorder")]

use std::path::PathBuf;
use std::process::ExitCode;

use zsh::vm_helper::ShellExecutor;

const USAGE: &str = "\
zshrs-recorder — capture every state-mutating dispatcher fire during
shell init and ship the bundle to zshrs-daemon. Single-shot.

USAGE
    zshrs-recorder [OPTIONS]

OPTIONS
    -f, --file PATH    Source PATH instead of the user's startup chain.
                       Use this to test recorder coverage on a small
                       script without dragging in the real .zshrc.
    -o, --output PATH  Write the captured bundle as JSON to PATH (in
                       addition to shipping it to the daemon, or as the
                       sole output under --no-daemon). Useful for
                       post-mortem inspection / diffing two runs.
        --shell-id ID  Override the bundle's shell_id (default `zshrs`).
                       Used for federation testing — let a recorder
                       impersonate `bash` / `fish` etc. against the
                       same daemon. See docs/SHELL_IDS.md.
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
        --no-daemon    Skip the end-of-run IPC bundle. Captured events
                       still print to stderr + log; nothing reaches the
                       daemon (no rkyv shard, no SQLite hydration). Used
                       by `tests/recorder_harness.rs` for hermetic runs.
                       Combine with -o PATH to capture the bundle to a
                       file with no daemon at all.
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
    Daemon IPC        One `recorder_ingest` op shipping the full bundle.
    Log               Same lines mirrored via tracing::info to the zshrs
                      log file.
";

struct Args {
    file: Option<PathBuf>,
    no_daemon: bool,
    output: Option<PathBuf>,
    shell_id: Option<String>,
    quiet: bool,
    json: bool,
    no_prewarm: bool,
}

fn parse_args() -> Result<Args, ExitCode> {
    let mut file: Option<PathBuf> = None;
    let mut no_daemon = false;
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
            "--no-daemon" => no_daemon = true,
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
        no_daemon,
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

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(code) => return code,
    };

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
    if args.no_daemon {
        zsh::recorder::set_daemon_disabled(true);
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
    // (`exit`, fatal error sites). Without this, summary + IPC bundle
    // would only fire on natural fall-through from `main` — which is
    // not how shell scripts usually terminate.
    zsh::recorder::install_atexit();

    let mut executor = ShellExecutor::new();
    let mut last_status: i32 = 0;

    if let Some(path) = args.file {
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
            last_status = executor.execute_script(&content).unwrap_or_else(|e| {
                eprintln!("zshrs-recorder: {}: {}", disp, e);
                1
            });
        }
    }

    // The init chain has finished, so every fpath dir the user's
    // config registered is now on `$fpath` — and the shell is idle,
    // which is the whole reason this pass lives here rather than in
    // `compinit`: `parse()` walks process-global lexer state, and
    // compiling 46k completers beside a live ZLE corrupted the prompt
    // when that was tried. Nothing runs after this but the summary and
    // the daemon bundle.
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

    // Process exits with the last sourced script's status. atexit fires
    // on the way out; that's where summary + daemon IPC happen.
    ExitCode::from(last_status as u8)
}
