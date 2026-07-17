//! Example native zshrs plugin. Registers two commands, `rhello` and
//! `renv`, demonstrating the full host API: printing, reading/writing
//! shell parameters, and evaluating shell code.

use std::os::raw::c_int;
use zshrs_plugin::{declare_plugin, Args, Host};

/// `rhello [names...]` — greet, echoing argv and the shell's `$PWD`.
fn rhello(host: &Host, args: &Args) -> c_int {
    let who = if args.rest().is_empty() {
        "world".to_string()
    } else {
        args.rest().join(", ")
    };
    let pwd = host.getvar("PWD").unwrap_or_default();
    host.print(&format!("hello, {who} — from a native Rust plugin (pwd={pwd})\n"));
    0
}

/// `renv NAME [VALUE]` — get or set a shell scalar from Rust. With one
/// arg, prints `$NAME`; with two, sets it and echoes the assignment.
fn renv(host: &Host, args: &Args) -> c_int {
    match args.rest() {
        [name] => {
            match host.getvar(name) {
                Some(v) => host.print(&format!("{name}={v}\n")),
                None => host.print(&format!("{name} is unset\n")),
            }
            0
        }
        [name, value] => {
            if host.setvar(name, value) {
                host.print(&format!("set {name}={value}\n"));
                0
            } else {
                host.print(&format!("renv: failed to set {name}\n"));
                1
            }
        }
        _ => {
            host.print("usage: renv NAME [VALUE]\n");
            2
        }
    }
}

declare_plugin! {
    name: "hello",
    version: "0.1.0",
    builtins: {
        "rhello" => rhello,
        "renv"   => renv,
    },
}
