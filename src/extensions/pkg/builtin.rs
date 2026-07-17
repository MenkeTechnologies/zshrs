//! The `zpm` builtin — argv dispatcher over [`super::commands`]. Wired from
//! `fusevm_bridge` alongside the other zshrs-original `z*` builtins. Errors
//! print as `zpm: <reason>` on stderr (terse zsh style) and return 1.

use super::commands;

const USAGE: &str = "\
usage: zpm <command> [args]

  add <SOURCE>       install + load a plugin (owner/repo, github:o/r, git+URL, path:DIR)
  remove <NAME>      unload + delete an installed plugin
  list               list installed plugins
  info <NAME>        show details for one plugin
  load [NAME]        load installed plugin(s) without network (for .zshrc startup)
  update [NAME]      re-resolve + reinstall from the recorded source
  help               this message";

/// Entry point for the `zpm` builtin. `args` are the arguments only (the
/// builtin name `zpm` is NOT in `args`), so `args[0]` is the subcommand.
pub fn zpm(args: &[String]) -> i32 {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    let rest = &args[args.len().min(1)..];

    let result = match sub {
        "add" | "install" | "i" => match rest.first() {
            Some(spec) => {
                // Support `zpm add a b c` → install several.
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
        // `zpm load` (no args) loads every installed plugin; `zpm load SPEC…`
        // loads each — installing on first use when SPEC is a not-yet-stored
        // source (owner/repo, github:…, path:…). Lets `.zshrc` carry
        // `zpm load owner/repo` lines that self-install on the first startup.
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
        "help" | "-h" | "--help" | "" => {
            println!("{}", USAGE);
            return 0;
        }
        other => return usage_err(&format!("unknown command '{}'", other)),
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("zpm: {}", e);
            1
        }
    }
}

/// Print a usage error to stderr and return 1.
fn usage_err(msg: &str) -> i32 {
    eprintln!("zpm: {}", msg);
    eprintln!("{}", USAGE);
    1
}
