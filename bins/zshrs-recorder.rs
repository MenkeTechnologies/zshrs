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
//!      dispatcher in `src/exec.rs` checks this and emits a record.
//!   3. `recorder::install_atexit()` registers the libc atexit hook so
//!      the end-of-run summary + daemon IPC bundle still fire when the
//!      shell exits via `std::process::exit` (skipping Rust Drop).
//!   4. Build a fresh `ShellExecutor` and source the requested file.
//!   5. Process exits naturally; atexit hook prints summary, ships the
//!      bundle to `zshrs-daemon` via `recorder_ingest`, returns.

#![cfg(feature = "recorder")]

use std::path::PathBuf;
use std::process::ExitCode;

use zsh::exec::ShellExecutor;

const USAGE: &str = "\
zshrs-recorder — capture every state-mutating dispatcher fire during
shell init and ship the bundle to zshrs-daemon. Single-shot.

USAGE
    zshrs-recorder [OPTIONS]

OPTIONS
    -f, --file PATH    Source PATH instead of the user's startup chain.
                       Use this to test recorder coverage on a small
                       script without dragging in the real .zshrc.
        --no-daemon    Skip the end-of-run IPC bundle. Captured events
                       still print to stderr + log; nothing reaches the
                       daemon (no rkyv shard, no SQLite hydration). Used
                       by `tests/recorder_harness.rs` for hermetic runs.
        --help         Print this message and exit.
        --version      Print version and exit.

DEFAULT BEHAVIOR (no --file)
    Sources ${ZDOTDIR:-$HOME}/.zshrc as if at login, capturing every
    alias, function, export, path/fpath edit, hash -d, zstyle, bindkey,
    compdef, zmodload, setopt, trap, sched, source, and assignment that
    fires through the runtime AOP layer.

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
}

fn parse_args() -> Result<Args, ExitCode> {
    let mut file: Option<PathBuf> = None;
    let mut no_daemon = false;
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
            "--no-daemon" => no_daemon = true,
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
    Ok(Args { file, no_daemon })
}

fn default_zshrc() -> PathBuf {
    let zdotdir = std::env::var_os("ZDOTDIR").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    zdotdir
        .or(home)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".zshrc")
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(code) => return code,
    };

    // Init logging FIRST so every recorder event reaches the zshrs log.
    zsh::log::init();

    zsh::recorder::enable();
    if args.no_daemon {
        zsh::recorder::set_daemon_disabled(true);
    }
    // libc atexit covers the `std::process::exit` paths inside builtins
    // (`exit`, fatal error sites). Without this, summary + IPC bundle
    // would only fire on natural fall-through from `main` — which is
    // not how shell scripts usually terminate.
    zsh::recorder::install_atexit();

    let target = args.file.unwrap_or_else(default_zshrc);
    let target_display = target.display().to_string();

    let content = match std::fs::read_to_string(&target) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "zshrs-recorder: cannot read {}: {}",
                target_display, e
            );
            return ExitCode::from(1);
        }
    };

    tracing::info!(file = %target_display, "zshrs-recorder: sourcing");
    eprintln!("zshrs-recorder: sourcing {}", target_display);

    let mut executor = ShellExecutor::new();
    // Set $0 to the sourced file so any script that introspects $0
    // (zinit / oh-my-zsh do) sees the right name.
    executor
        .variables
        .insert("0".to_string(), target_display.clone());

    let status = executor.execute_script(&content).unwrap_or_else(|e| {
        eprintln!("zshrs-recorder: {}: {}", target_display, e);
        1
    });

    // Process exits with the script's last status. atexit fires on the
    // way out; that's where summary + daemon IPC happen.
    ExitCode::from(status as u8)
}
