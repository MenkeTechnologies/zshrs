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
    // Top-level routing — `zask --target ...` (push form) routes to `ask`,
    // `zask <verb>` (pull form) routes to the verb. Spec per docs/DAEMON.md.
    if args.len() > 1 && args[1] == "--target" {
        return ask_with_target(&args[1..]);
    }
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let rest: &[String] = if args.len() > 2 { &args[2..] } else { &[] };
    match verb {
        "ask" => ask(rest), // legacy positional form: `zask ask <shell_id> <kind> <prompt>`
        "progress" => progress(rest),
        "pending" => pending(rest),
        "take" => take(rest),
        "dismiss" => dismiss(rest),
        "inbox-clear" => inbox_clear(rest),
        "response" => response(rest),
        "" | "-h" | "--help" => {
            println!("usage: zask --target <shell:N|tag:T|*> <picker|input|dialog|menu> [opts]");
            println!("       zask progress --target <T> --label <L> --percent N [--eta MS] [--request-id ID] [--done]");
            println!("       zask pending [--shell <id>]");
            println!("       zask take [--id <request_id>]");
            println!("       zask dismiss <request_id> [--reason <text>] | --all");
            println!("       zask inbox-clear");
            println!("       zask response <request_id> <data>");
            0
        }
        other => err_exit(&format!("unknown verb `{}`", other)),
    }
}

// `zask --target <scope> <kind> [--items "..."] [--prompt "..."] [--message "..."]`
//                                 [--urgency low|normal|high|critical] [--timeout SECS]
//                                 [--multi] [--secret] [--options yes,no]
fn ask_with_target(args: &[String]) -> i32 {
    // args starts with "--target"
    let target_raw = match args.get(1) {
        Some(s) => s.clone(),
        None => return err_exit("--target requires a value (shell:N|tag:T|*)"),
    };
    let target = parse_target(&target_raw)
        .unwrap_or_else(|e| {
            eprintln!("zshrs: zask: {}", e);
            json!({})
        });
    if target.as_object().map_or(true, |m| m.is_empty()) {
        return 1;
    }
    let kind = match args.get(2) {
        Some(s) => s.clone(),
        None => return err_exit("ask: missing kind (picker|input|dialog|menu)"),
    };

    let mut payload = serde_json::Map::new();
    let mut urgency = "normal".to_string();
    let mut timeout_ms: Option<u64> = None;

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--urgency" => {
                if let Some(u) = args.get(i + 1) {
                    urgency = u.clone();
                    i += 2;
                } else {
                    return err_exit("--urgency requires a value");
                }
            }
            "--timeout" => {
                match args.get(i + 1).and_then(|s| s.parse::<u64>().ok()) {
                    Some(secs) => {
                        timeout_ms = Some(secs * 1000);
                        i += 2;
                    }
                    None => return err_exit("--timeout requires integer seconds"),
                }
            }
            "--timeout-ms" => {
                match args.get(i + 1).and_then(|s| s.parse::<u64>().ok()) {
                    Some(ms) => {
                        timeout_ms = Some(ms);
                        i += 2;
                    }
                    None => return err_exit("--timeout-ms requires an integer"),
                }
            }
            // Generic --foo value passthrough into payload (e.g. --items, --prompt, --message,
            // --options, --title). --multi / --secret / --no-timeout become bool flags.
            other if other.starts_with("--") => {
                let key = other.trim_start_matches("--").to_string();
                let next = args.get(i + 1);
                let is_flag = next
                    .map(|n| n.starts_with("--"))
                    .unwrap_or(true);
                if matches!(key.as_str(), "multi" | "secret" | "done" | "no-timeout") || is_flag {
                    payload.insert(key, Value::Bool(true));
                    i += 1;
                } else {
                    payload.insert(key, Value::String(next.unwrap().clone()));
                    i += 2;
                }
            }
            _ => i += 1,
        }
    }

    let mut req = json!({
        "target": target,
        "kind": kind,
        "payload": Value::Object(payload),
        "urgency": urgency,
    });
    if let Some(t) = timeout_ms {
        req["timeout_ms"] = json!(t);
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_ask", req) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("ask: {}", e)),
    }
}

/// Parse `shell:N` / `tag:NAME` / `*` / `<bare integer>` into the target object
/// the daemon expects.
fn parse_target(s: &str) -> Result<Value, String> {
    if s == "*" {
        return Ok(json!({ "all": true }));
    }
    if let Some(rest) = s.strip_prefix("shell:") {
        match rest.parse::<u64>() {
            Ok(n) => return Ok(json!({ "shell_id": n })),
            Err(_) => return Err(format!("invalid shell:N: `{}`", s)),
        }
    }
    if let Some(rest) = s.strip_prefix("tag:") {
        return Ok(json!({ "tag": rest }));
    }
    if let Ok(n) = s.parse::<u64>() {
        return Ok(json!({ "shell_id": n }));
    }
    Err(format!("target must be shell:N | tag:T | * | <id> (got `{}`)", s))
}

// `zask progress` — passive status-line update. Uses ask_ask with kind=progress.
fn progress(args: &[String]) -> i32 {
    let mut target_str: Option<String> = None;
    let mut label: Option<String> = None;
    let mut percent: Option<i64> = None;
    let mut eta_ms: Option<u64> = None;
    let mut request_id: Option<String> = None;
    let mut done = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                target_str = args.get(i + 1).cloned();
                i += 2;
            }
            "--label" => {
                label = args.get(i + 1).cloned();
                i += 2;
            }
            "--percent" => {
                percent = args.get(i + 1).and_then(|s| s.parse::<i64>().ok());
                i += 2;
            }
            "--eta" => {
                eta_ms = args.get(i + 1).and_then(|s| s.parse::<u64>().ok());
                i += 2;
            }
            "--request-id" => {
                request_id = args.get(i + 1).cloned();
                i += 2;
            }
            "--done" => {
                done = true;
                i += 1;
            }
            _ => i += 1,
        }
    }

    let target_raw = match target_str {
        Some(t) => t,
        None => return err_exit("progress: --target required"),
    };
    let target = match parse_target(&target_raw) {
        Ok(t) => t,
        Err(e) => return err_exit(&e),
    };

    let mut payload = serde_json::Map::new();
    if let Some(l) = label {
        payload.insert("label".to_string(), Value::String(l));
    }
    if let Some(p) = percent {
        payload.insert("percent".to_string(), json!(p));
    }
    if let Some(e) = eta_ms {
        payload.insert("eta_ms".to_string(), json!(e));
    }
    if done {
        payload.insert("done".to_string(), Value::Bool(true));
    }
    if let Some(rid) = request_id {
        payload.insert("request_id".to_string(), Value::String(rid));
    }

    let req = json!({
        "target": target,
        "kind": "progress",
        "payload": Value::Object(payload),
        "urgency": "normal",
    });
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_ask", req) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("progress: {}", e)),
    }
}

// `zask inbox-clear` — drop every queued request for THIS shell.
fn inbox_clear(_args: &[String]) -> i32 {
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_dismiss", json!({ "all": true })) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("inbox-clear: {}", e)),
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
    let mut payload = json!({});
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--shell" => match iter.next().and_then(|s| s.parse::<u64>().ok()) {
                Some(id) => payload["shell_id"] = json!(id),
                None => return err_exit("pending: --shell requires an integer"),
            },
            other if other.starts_with('-') && other != "--all" => {
                return err_exit(&format!("pending: unknown flag `{}`", other));
            }
            _ => {}
        }
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_pending", payload) {
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
    let mut id: Option<String> = None;
    let mut value_words: Vec<String> = Vec::new();
    let mut cancelled = false;
    let mut from_shell: Option<u64> = None;
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--cancelled" => cancelled = true,
            "--from-shell" => match iter.next().and_then(|s| s.parse::<u64>().ok()) {
                Some(n) => from_shell = Some(n),
                None => return err_exit("response: --from-shell requires an integer"),
            },
            other => {
                if id.is_none() {
                    id = Some(other.to_string());
                } else {
                    value_words.push(other.to_string());
                }
            }
        }
    }
    let id = match id {
        Some(s) => s,
        None => return err_exit("response: usage <request_id> [<data>] [--cancelled] [--from-shell N]"),
    };
    let raw = value_words.join(" ");
    let data: Value = if raw.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&raw).unwrap_or(Value::String(raw))
    };
    let mut payload = json!({ "request_id": id, "value": data, "cancelled": cancelled });
    if let Some(s) = from_shell {
        payload["from_shell"] = json!(s);
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("ask_response", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("response: {}", e)),
    }
}
