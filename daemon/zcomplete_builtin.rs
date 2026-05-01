// zcomplete + zsuggest — client-facing builtins exposing the daemon's
// keystroke-rate data-plane ops (complete + suggest) as CLI verbs.
//
// The actual ZLE keystroke pipe (parsing the buffer, painting the menu /
// inline ghost text) lives in the interactive shell. These builtins let
// scripts and one-shot consumers query the same daemon endpoints over IPC,
// useful for testing, headless completion-debugging, and shells that don't
// have a full ZLE.
//
// CLI shape:
//   zcomplete <prefix> [--limit N]      # commands + handlers + history
//   zsuggest  <prefix> [--cwd DIR]      # single best-match history line

use serde_json::{json, Value};

use super::client::Client;
use super::paths::CachePaths;

fn err_exit(msg: &str) -> i32 {
    eprintln!("zshrs: {}", msg);
    1
}

fn print_pretty(v: &Value) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{}", s),
        Err(_) => println!("{}", v),
    }
}

fn connect(name: &str) -> Result<Client, ()> {
    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("zshrs: {}: daemon: {}", name, e);
            return Err(());
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("zshrs: {}: daemon: {}", name, e);
        return Err(());
    }
    Client::connect(&paths).map_err(|e| {
        eprintln!("zshrs: {}: daemon: {}", name, e);
    })
}

pub fn zcomplete(args: &[String]) -> i32 {
    let mut prefix: Option<String> = None;
    let mut limit: Option<u64> = None;
    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--limit" => match iter.next().and_then(|s| s.parse::<u64>().ok()) {
                Some(n) => limit = Some(n),
                None => return err_exit("zcomplete: --limit requires an integer"),
            },
            "-h" | "--help" => {
                println!("usage: zcomplete <prefix> [--limit N]");
                return 0;
            }
            other if other.starts_with('-') => {
                return err_exit(&format!("zcomplete: unknown flag `{}`", other));
            }
            other => {
                if prefix.is_some() {
                    return err_exit("zcomplete: expected exactly one prefix");
                }
                prefix = Some(other.to_string());
            }
        }
    }
    let prefix = prefix.unwrap_or_default();
    let mut payload = json!({ "prefix": prefix });
    if let Some(n) = limit {
        payload["limit"] = json!(n);
    }
    let mut client = match connect("zcomplete") {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("complete", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("zcomplete: {}", e)),
    }
}

pub fn zsuggest(args: &[String]) -> i32 {
    let mut prefix: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--cwd" => match iter.next() {
                Some(d) => cwd = Some(d.clone()),
                None => return err_exit("zsuggest: --cwd requires a directory"),
            },
            "-h" | "--help" => {
                println!("usage: zsuggest <prefix> [--cwd DIR]");
                return 0;
            }
            other if other.starts_with('-') => {
                return err_exit(&format!("zsuggest: unknown flag `{}`", other));
            }
            other => {
                if prefix.is_some() {
                    return err_exit("zsuggest: expected exactly one prefix");
                }
                prefix = Some(other.to_string());
            }
        }
    }
    let prefix = prefix.unwrap_or_default();
    let mut payload = json!({ "prefix": prefix });
    if let Some(c) = cwd {
        payload["cwd"] = Value::String(c);
    }
    let mut client = match connect("zsuggest") {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("suggest", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("zsuggest: {}", e)),
    }
}
