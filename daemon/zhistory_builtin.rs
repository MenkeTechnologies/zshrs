// zhistory — client-facing builtin for daemon-managed history.
//
// Wraps the existing history_append / history_query ops in zhistory.rs.
// Lets non-interactive contexts (scripts, tests) inject history rows and
// query them; an interactive shell uses the same ops on every command via
// preexec/precmd hooks (and triggers long_cmd_complete events when a
// command's duration exceeds the threshold).
//
// CLI shape:
//   zhistory append <line> [--exit-code N] [--cwd D] [--duration-ns N]
//                          [--ts-ns N] [--shell-id N]
//   zhistory query [--filter <pat>] [--mode match|fts|exact|prefix|cwd]
//                  [--cwd D] [--limit N] [--asc] [--after-ns N] [--before-ns N]
//   zhistory count

use serde_json::{json, Value};

use super::client::Client;
use super::paths::CachePaths;

fn err_exit(msg: &str) -> i32 {
    eprintln!("zshrs: zhistory: {}", msg);
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
            eprintln!("zshrs: zhistory: daemon: {}", e);
            return Err(());
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("zshrs: zhistory: daemon: {}", e);
        return Err(());
    }
    Client::connect(&paths).map_err(|e| {
        eprintln!("zshrs: zhistory: daemon: {}", e);
    })
}

pub fn zhistory(args: &[String]) -> i32 {
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let rest: &[String] = if args.len() > 2 { &args[2..] } else { &[] };
    match verb {
        "append" | "add" => append(rest),
        "query" | "search" => query(rest),
        "count" => count(),
        "" | "-h" | "--help" => {
            println!("usage: zhistory append <line> [--exit-code N] [--cwd DIR]");
            println!("                              [--duration-ns N] [--ts-ns N] [--shell-id N]");
            println!("       zhistory query [--filter <pat>] [--mode match|fts|exact|prefix|cwd]");
            println!("                      [--cwd DIR] [--limit N] [--asc] [--after-ns N] [--before-ns N]");
            println!("       zhistory count");
            0
        }
        other => err_exit(&format!("unknown verb `{}`", other)),
    }
}

fn append(args: &[String]) -> i32 {
    let line = match args.first() {
        Some(s) => s.clone(),
        None => return err_exit("append: missing <line>"),
    };
    let mut payload = json!({ "line": line });
    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--exit-code" => match iter.next().and_then(|s| s.parse::<i64>().ok()) {
                Some(n) => {
                    payload["exit_code"] = json!(n);
                }
                None => return err_exit("append: --exit-code requires an integer"),
            },
            "--cwd" => match iter.next() {
                Some(d) => {
                    payload["cwd"] = Value::String(d.clone());
                }
                None => return err_exit("append: --cwd requires a directory"),
            },
            "--duration-ns" => match iter.next().and_then(|s| s.parse::<i64>().ok()) {
                Some(n) => {
                    payload["duration_ns"] = json!(n);
                }
                None => return err_exit("append: --duration-ns requires an integer"),
            },
            "--ts-ns" => match iter.next().and_then(|s| s.parse::<i64>().ok()) {
                Some(n) => {
                    payload["ts_ns"] = json!(n);
                }
                None => return err_exit("append: --ts-ns requires an integer"),
            },
            "--shell-id" => match iter.next().and_then(|s| s.parse::<i64>().ok()) {
                Some(n) => {
                    payload["shell_id"] = json!(n);
                }
                None => return err_exit("append: --shell-id requires an integer"),
            },
            other if other.starts_with('-') => {
                return err_exit(&format!("append: unknown flag `{}`", other));
            }
            _ => {}
        }
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("history_append", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("append: {}", e)),
    }
}

fn query(args: &[String]) -> i32 {
    let mut payload = json!({});
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--filter" => match iter.next() {
                Some(p) => {
                    payload["filter"] = Value::String(p.clone());
                }
                None => return err_exit("query: --filter requires a value"),
            },
            "--mode" => match iter.next() {
                Some(m) => {
                    payload["mode"] = Value::String(m.clone());
                }
                None => return err_exit("query: --mode requires a value"),
            },
            "--cwd" => match iter.next() {
                Some(d) => {
                    payload["cwd"] = Value::String(d.clone());
                }
                None => return err_exit("query: --cwd requires a directory"),
            },
            "--limit" => match iter.next().and_then(|s| s.parse::<i64>().ok()) {
                Some(n) => {
                    payload["limit"] = json!(n);
                }
                None => return err_exit("query: --limit requires an integer"),
            },
            "--asc" => {
                payload["descending"] = json!(false);
            }
            "--after-ns" => match iter.next().and_then(|s| s.parse::<i64>().ok()) {
                Some(n) => {
                    payload["after_ns"] = json!(n);
                }
                None => return err_exit("query: --after-ns requires an integer"),
            },
            "--before-ns" => match iter.next().and_then(|s| s.parse::<i64>().ok()) {
                Some(n) => {
                    payload["before_ns"] = json!(n);
                }
                None => return err_exit("query: --before-ns requires an integer"),
            },
            other if other.starts_with('-') => {
                return err_exit(&format!("query: unknown flag `{}`", other));
            }
            _ => {}
        }
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("history_query", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("query: {}", e)),
    }
}

fn count() -> i32 {
    // history_query with limit=0 isn't ideal — server clamps to 1+. Use a
    // dedicated history_count op if/when added. For v1, approximate: query
    // with a high limit and report the row count.
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("history_query", json!({ "limit": 100000 })) {
        Ok(v) => {
            let n = v.get("count").and_then(Value::as_u64).unwrap_or(0);
            println!("{}", n);
            0
        }
        Err(e) => err_exit(&format!("count: {}", e)),
    }
}
