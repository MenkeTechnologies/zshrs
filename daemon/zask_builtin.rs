// zask — client-facing builtin for daemon-queued UI primitives.
//
// Thin IPC wrapper around the server-side ops in `zask.rs`:
//   ask_ask / ask_pending / ask_take / ask_dismiss / ask_response
//
// Pull-mode UI: the daemon never auto-renders; it only queues requests, pushes
// `ask:pending` events to status-line indicators, and lets the user pull
// requests via `zask take` (or a Ctrl-X q keybinding hooked to the same op).
//
// CLI shape:
//   zask ask <shell_id> <kind> <prompt> [--urgency LEVEL] [--timeout-ms N]
//   zask pending [--all]
//   zask take [--id ID]            — pop the next/specified request
//   zask dismiss <id> [--reason R]
//   zask response <id> <data>      — return a result to the asker
//
// kind = picker | input | dialog | menu | progress

use serde_json::{json, Value};

use super::client::Client;
use super::paths::CachePaths;

fn err_exit(msg: &str) -> i32 {
    eprintln!("zshrs: zask: {}", msg);
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
            eprintln!("zshrs: zask: daemon: {}", e);
            return Err(());
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("zshrs: zask: daemon: {}", e);
        return Err(());
    }
    Client::connect(&paths).map_err(|e| {
        eprintln!("zshrs: zask: daemon: {}", e);
    })
}

pub fn zask(args: &[String]) -> i32 {
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("");
    match verb {
        "ask" => ask(&args[2..]),
        "pending" => pending(&args[2..]),
        "take" => take(&args[2..]),
        "dismiss" => dismiss(&args[2..]),
        "response" => response(&args[2..]),
        "" | "-h" | "--help" => {
            println!("usage: zask ask <shell_id> <picker|input|dialog|menu|progress> <prompt>");
            println!("              [--urgency low|normal|high|critical] [--timeout-ms N]");
            println!("       zask pending [--all]");
            println!("       zask take [--id <request_id>]");
            println!("       zask dismiss <request_id> [--reason <text>]");
            println!("       zask response <request_id> <data>");
            0
        }
        other => err_exit(&format!("unknown verb `{}`", other)),
    }
}

fn ask(args: &[String]) -> i32 {
    if args.len() < 3 {
        return err_exit("ask: usage <shell_id> <kind> <prompt>");
    }
    let shell_id = match args[0].parse::<u64>() {
        Ok(n) => n,
        Err(_) => return err_exit("ask: shell_id must be an integer"),
    };
    let kind = args[1].clone();
    let prompt = args[2].clone();

    let mut urgency = "normal".to_string();
    let mut timeout_ms: Option<u64> = None;

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--urgency" => {
                if let Some(u) = args.get(i + 1) {
                    urgency = u.clone();
                    i += 2;
                    continue;
                } else {
                    return err_exit("ask: --urgency requires a value");
                }
            }
            "--timeout-ms" => {
                if let Some(t) = args.get(i + 1) {
                    match t.parse::<u64>() {
                        Ok(n) => {
                            timeout_ms = Some(n);
                            i += 2;
                            continue;
                        }
                        Err(_) => return err_exit("ask: --timeout-ms requires an integer"),
                    }
                } else {
                    return err_exit("ask: --timeout-ms requires a value");
                }
            }
            _ => i += 1,
        }
    }

    let mut payload = json!({
        "target": { "shell_id": shell_id },
        "kind": kind,
        "payload": { "prompt": prompt },
        "urgency": urgency,
    });
    if let Some(t) = timeout_ms {
        payload["timeout_ms"] = json!(t);
    }

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_ask", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("ask: {}", e)),
    }
}

fn pending(args: &[String]) -> i32 {
    let all = args.iter().any(|a| a == "--all");
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_pending", json!({ "all": all })) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("pending: {}", e)),
    }
}

fn take(args: &[String]) -> i32 {
    let mut payload = json!({});
    if let Some(idx) = args.iter().position(|a| a == "--id") {
        if let Some(id) = args.get(idx + 1) {
            payload["request_id"] = Value::String(id.clone());
        } else {
            return err_exit("take: --id requires a value");
        }
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_take", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("take: {}", e)),
    }
}

fn dismiss(args: &[String]) -> i32 {
    let id = match args.first() {
        Some(s) => s.clone(),
        None => return err_exit("dismiss: missing <request_id>"),
    };
    let mut payload = json!({ "request_id": id });
    if let Some(idx) = args.iter().position(|a| a == "--reason") {
        if let Some(r) = args.get(idx + 1) {
            payload["reason"] = Value::String(r.clone());
        }
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_dismiss", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("dismiss: {}", e)),
    }
}

fn response(args: &[String]) -> i32 {
    if args.len() < 2 {
        return err_exit("response: usage <request_id> <data>");
    }
    let id = args[0].clone();
    let raw = args[1..].join(" ");
    // Try to parse the data as JSON; fall back to a plain string.
    let data: Value = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call(
        "ask_response",
        json!({ "request_id": id, "response": data }),
    ) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("response: {}", e)),
    }
}
