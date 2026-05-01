// zsync — client-facing builtin for canonical-state push/pull/diff.
//
// Thin IPC wrapper around the server-side ops in `zsync.rs`:
//   push_canonical / pull_canonical / diff_canonical
//
// Subsystems supported (validated daemon-side): path, fpath, manpath, named_dir,
// alias, galias, salias, function, compdef, env, params, zstyle, bindkey,
// setopt, zmodload.
//
// CLI shape:
//   zsync push <subsystem> <key> <value>          — promote one entry
//   zsync push <subsystem> --json <{k:v,…}>       — bulk push
//   zsync pull <subsystem>                        — list everything
//   zsync diff <subsystem> --overlay <{k:v,…}>    — overlay vs canonical
//
// All output is JSON (the canonical-state subsystem is intrinsically structured;
// pretty-printing it makes scripted consumers' life easier).

use serde_json::{json, Value};

use super::client::Client;
use super::paths::CachePaths;

fn err_exit(msg: &str) -> i32 {
    eprintln!("zshrs: zsync: {}", msg);
    1
}

fn print_pretty(v: &Value) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{}", s),
        Err(_) => println!("{}", v),
    }
}

fn connect() -> Result<Client, ()> {
    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("zshrs: zsync: daemon: {}", e);
            return Err(());
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("zshrs: zsync: daemon: {}", e);
        return Err(());
    }
    Client::connect(&paths).map_err(|e| {
        eprintln!("zshrs: zsync: daemon: {}", e);
    })
}

pub fn zsync(args: &[String]) -> i32 {
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("");
    match verb {
        "push" => push(&args[2..]),
        "pull" => pull(&args[2..]),
        "diff" => diff(&args[2..]),
        "" | "-h" | "--help" => {
            println!("usage: zsync push <subsystem> <key> <value>");
            println!("       zsync push <subsystem> --json '<{{\"k\":\"v\",...}}>'");
            println!("       zsync pull <subsystem>");
            println!("       zsync diff <subsystem> --overlay '<{{\"k\":\"v\",...}}>'");
            0
        }
        other => err_exit(&format!("unknown verb `{}`", other)),
    }
}

fn push(args: &[String]) -> i32 {
    let subsystem = match args.first() {
        Some(s) => s.clone(),
        None => return err_exit("push: missing <subsystem>"),
    };

    let value = if let Some(json_idx) = args.iter().position(|a| a == "--json") {
        let raw = match args.get(json_idx + 1) {
            Some(s) => s.clone(),
            None => return err_exit("push: --json requires a JSON value"),
        };
        match serde_json::from_str::<Value>(&raw) {
            Ok(v) => v,
            Err(e) => return err_exit(&format!("push: malformed --json: {}", e)),
        }
    } else if args.len() == 3 {
        json!({ &args[1]: &args[2] })
    } else if args.len() == 2 {
        // single string value, blank key
        Value::String(args[1].clone())
    } else {
        return err_exit("push: expected `<subsystem> <key> <value>` or `--json '<obj>'`");
    };

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call(
        "push_canonical",
        json!({ "subsystem": subsystem, "value": value }),
    ) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("push: {}", e)),
    }
}

fn pull(args: &[String]) -> i32 {
    let subsystem = match args.first() {
        Some(s) => s.clone(),
        None => return err_exit("pull: missing <subsystem>"),
    };
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("pull_canonical", json!({ "subsystem": subsystem })) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("pull: {}", e)),
    }
}

fn diff(args: &[String]) -> i32 {
    let subsystem = match args.first() {
        Some(s) => s.clone(),
        None => return err_exit("diff: missing <subsystem>"),
    };
    let overlay = if let Some(idx) = args.iter().position(|a| a == "--overlay") {
        let raw = match args.get(idx + 1) {
            Some(s) => s.clone(),
            None => return err_exit("diff: --overlay requires a JSON value"),
        };
        match serde_json::from_str::<Value>(&raw) {
            Ok(v) => v,
            Err(e) => return err_exit(&format!("diff: malformed --overlay: {}", e)),
        }
    } else {
        Value::Null
    };
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call(
        "diff_canonical",
        json!({ "subsystem": subsystem, "overlay": overlay }),
    ) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("diff: {}", e)),
    }
}
