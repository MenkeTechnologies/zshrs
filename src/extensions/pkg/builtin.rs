//! The `znative` builtin — argv dispatcher over [`super::commands`]. Wired from
//! `fusevm_bridge` alongside the other zshrs-original `z*` builtins. Errors
//! print as `znative: <reason>` on stderr (terse zsh style) and return 1.

use super::commands;

const USAGE: &str = "\
usage: znative <command> [args]

  load [SOURCE...]   load plugin(s); a source not yet in the store is
                     installed first, then loaded (for .zshrc startup).
                     No args loads everything installed. Zero-network once
                     stored. SOURCE: owner/repo, github:o/r, git+URL, path:DIR
  add <SOURCE>       install + load a plugin (load self-installs, so add is
                     mainly for installing without a .zshrc line)
  remove <NAME>      unload + delete an installed plugin
  list               list installed plugins
  info <NAME>        show details for one plugin
  update [NAME]      re-resolve + reinstall from the recorded source
  gc [--dry-run]     remove orphan store entries + the git clone cache
  clean              clear scratch caches (git/, cache/, bin/)
  help               this message

aliases: add=install=i  remove=rm=uninstall  list=ls  info=show
         load=source  update=up=upgrade";

/// Entry point for the `znative` builtin. `args` are the arguments only (the
/// builtin name `znative` is NOT in `args`), so `args[0]` is the subcommand.
pub fn znative(args: &[String]) -> i32 {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[args.len().min(1)..];

    let result = match sub {
        "add" | "install" | "i" => match rest.first() {
            Some(spec) => {
                // Support `znative add a b c` → install several.
                let mut last = Ok(());
                for spec in rest {
                    if let Err(e) = commands::add(spec) {
                        last = Err(e);
                    }
                }
                let _ = spec;
                last
            }
            None => return usage_err("add requires a SOURCE"),
        },
        "remove" | "rm" | "uninstall" => match rest.first() {
            Some(_) => {
                let mut last = Ok(());
                for name in rest {
                    if let Err(e) = commands::remove(name) {
                        last = Err(e);
                    }
                }
                last
            }
            None => return usage_err("remove requires a NAME"),
        },
        "list" | "ls" => commands::list(),
        "info" | "show" => match rest.first() {
            Some(name) => commands::info(name),
            None => return usage_err("info requires a NAME"),
        },
        // `znative load` (no args) loads every installed plugin; `znative load SPEC…`
        // loads each — installing on first use when SPEC is a not-yet-stored
        // source (owner/repo, github:…, path:…). Lets `.zshrc` carry
        // `znative load owner/repo` lines that self-install on the first startup.
        "load" | "source" => {
            if rest.is_empty() {
                commands::load(None)
            } else {
                let mut last = Ok(());
                for spec in rest {
                    if let Err(e) = commands::load(Some(spec)) {
                        last = Err(e);
                    }
                }
                last
            }
        }
        "update" | "upgrade" | "up" => commands::update(rest.first().map(|s| s.as_str())),
        "gc" => commands::gc(rest.iter().any(|a| a == "--dry-run" || a == "-n")),
        "clean" => commands::clean(),
        "help" | "-h" | "--help" | "" => {
            println!("{}", USAGE);
            return 0;
        }
        other => return usage_err(&format!("unknown command '{}'", other)),
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("znative: {}", e);
            1
        }
    }
}

/// Print a usage error to stderr and return 1.
fn usage_err(msg: &str) -> i32 {
    eprintln!("znative: {}", msg);
    eprintln!("{}", USAGE);
    1
}

/// [`HandlerFunc`](crate::ported::zsh_h::HandlerFunc) adapter so `znative` is a
/// **first-class builtin**: registered in `createbuiltintable` via
/// [`bintab`], so `whence -w znative` reports `builtin`, `builtin znative …` works,
/// and the compiler emits a builtin opcode — not just the fusevm
/// command-dispatch arm. `argv` is the args after the command name (zsh
/// convention), exactly what [`znative`] expects.
pub fn bin_znative(
    _name: &str,
    argv: &[String],
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    znative(argv)
}

/// Builtin-table entry for `znative`, folded into
/// [`createbuiltintable`](crate::ported::builtin::createbuiltintable).
/// `BINF_HANDLES_OPTS` so `execbuiltin` passes args verbatim — `znative` parses
/// its own subcommands and flags.
#[allow(non_upper_case_globals)]
pub static bintab: std::sync::LazyLock<Vec<crate::ported::zsh_h::builtin>> =
    std::sync::LazyLock::new(|| {
        vec![crate::ported::builtin::BUILTIN(
            "znative",
            crate::ported::zsh_h::BINF_HANDLES_OPTS,
            Some(bin_znative as crate::ported::zsh_h::HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        )]
    });
