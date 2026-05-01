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
    let rest: &[String] = if args.len() > 2 { &args[2..] } else { &[] };
    match verb {
        // `up` is the canonical verb (per docs/DAEMON.md "zsync up <subsystem>");
        // `push` is the legacy alias kept for compat.
        "up" | "push" => push(rest),
        "pull" => pull(rest),
        "diff" => diff(rest),
        "watch" => watch(rest),
        "" | "-h" | "--help" => {
            println!("usage: zsync up <subsystem> <key> <value>");
            println!("       zsync up <subsystem> --json '<{{\"k\":\"v\",...}}>'");
            println!("       zsync up --all                        # promote every subsystem");
            println!("       zsync pull <subsystem>");
            println!("       zsync diff <subsystem> --overlay '<{{\"k\":\"v\",...}}>'");
            println!("       zsync watch <subsystem>...            # stream canonical_changed events");
            0
        }
        other => err_exit(&format!("unknown verb `{}`", other)),
    }
}

const ALL_SUBSYSTEMS: &[&str] = &[
    "path",
    "fpath",
    "manpath",
    "named_dir",
    "alias",
    "galias",
    "salias",
    "function",
    "compdef",
    "env",
    "params",
    "zstyle",
    "bindkey",
    "setopt",
    "zmodload",
];

// `zsync watch <subsystem>...` — streaming consumer that subscribes to
// canonical_changed events and prints them as they arrive. Exits on Ctrl-C.
fn watch(args: &[String]) -> i32 {
    if args.is_empty() {
        return err_exit("watch: usage: zsync watch <subsystem>...");
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    // canonical_changed events have no per-subsystem topic in v1; the daemon
    // broadcasts to all sessions on every push. We just consume them and
    // filter client-side by the user's subsystem list.
    if let Err(e) = client.set_read_timeout(None) {
        return err_exit(&format!("watch: {}", e));
    }
    eprintln!(
        "zsync: watching canonical_changed for subsystems {} (Ctrl-C to exit)",
        args.join(",")
    );
    use super::ipc::Frame;
    use super::DaemonError;
    loop {
        match client.next_frame() {
            Ok(Frame::Event { event, payload }) if event == "canonical_changed" => {
                let subsys = payload
                    .get("subsystem")
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                if args.iter().any(|s| s == subsys) {
                    let count = payload.get("row_count").and_then(Value::as_u64).unwrap_or(0);
                    let by = payload
                        .get("set_by_shell")
                        .and_then(Value::as_u64)
                        .unwrap_or(0);
                    let ts = chrono::Local::now().format("%H:%M:%S");
                    println!(
                        "[{}] {} updated by shell:{} ({} rows)",
                        ts, subsys, by, count
                    );
                }
            }
            Ok(_) => continue,
            Err(DaemonError::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof) =>
            {
                eprintln!("zsync watch: daemon closed connection");
                return 0;
            }
            Err(e) => return err_exit(&format!("watch: {}", e)),
        }
    }
}

fn push(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--all") {
        return err_exit(
            "up --all requires shell-side overlay enumeration (alias/path/setopt/etc tables); not yet wired in v1. Use `zsync up <subsystem> ...` per subsystem for now."
        );
    }
    let subsystem = match args.first() {
        Some(s) => s.clone(),
        None => return err_exit("up: missing <subsystem>"),
    };
    if !ALL_SUBSYSTEMS.contains(&subsystem.as_str()) {
        return err_exit(&format!(
            "up: subsystem `{}` not recognized; valid: {}",
            subsystem,
            ALL_SUBSYSTEMS.join(",")
        ));
    }

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
