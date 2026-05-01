// zjob — client-facing builtin for the daemon's session-persistent job supervisor.
//
// Per docs/DAEMON.md "z* builtin family" + memory cache_architecture_rkyv.md:
// `zjob` is one of three stacked world-firsts on the daemon:
//   (1) shell with dedicated daemon
//   (2) native session-persistent job supervisor   ← this
//   (3) native cross-shell pub/sub + dispatch
//
// Replaces nohup/disown/setsid/pueue/screen-as-job-runner. Jobs survive shell
// exit; output is captured to ~/.cache/zshrs/jobs/{id}.{out,err}; status is
// persisted in catalog.db so even daemon restarts don't lose history.
//
// CLI shape:
//   zjob submit <cmd> [<args>...] [--cwd DIR] [--tag T...] [--env K=V...]
//   zjob list   [--state running|exited|killed|failed] [--tag T] [--limit N]
//   zjob status <id>
//   zjob output <id> [--follow] [--stderr] [--lines N]
//   zjob kill   <id> [--signal NAME]
//   zjob wait   <id> [--timeout SECS]

use serde_json::{json, Value};

use super::client::Client;
use super::paths::CachePaths;
use super::DaemonError;

fn err_exit(msg: &str) -> i32 {
    eprintln!("zshrs: zjob: {}", msg);
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
            eprintln!("zshrs: zjob: daemon: {}", e);
            return Err(());
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("zshrs: zjob: daemon: {}", e);
        return Err(());
    }
    Client::connect(&paths).map_err(|e| {
        eprintln!("zshrs: zjob: daemon: {}", e);
    })
}

pub fn zjob(args: &[String]) -> i32 {
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("");
    match verb {
        "submit" => submit(&args[2..]),
        "list" | "ls" => list(&args[2..]),
        "status" => status(&args[2..]),
        "output" | "out" => output(&args[2..]),
        "kill" => kill(&args[2..]),
        "wait" => wait_for(&args[2..]),
        "" | "-h" | "--help" => {
            println!("usage: zjob submit <cmd> [<args>...] [--cwd DIR] [--tag T...] [--env K=V...]");
            println!("       zjob list   [--state running|exited|killed|failed] [--tag T] [--limit N]");
            println!("       zjob status <id>");
            println!("       zjob output <id> [--follow] [--stderr] [--lines N]");
            println!("       zjob kill   <id> [--signal NAME]");
            println!("       zjob wait   <id> [--timeout SECS]");
            0
        }
        other => err_exit(&format!("unknown verb `{}`", other)),
    }
}

fn submit(args: &[String]) -> i32 {
    let mut cwd: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut command: Vec<String> = Vec::new();

    let mut iter = args.iter();
    let mut in_command = false;
    while let Some(a) = iter.next() {
        if in_command {
            command.push(a.clone());
            continue;
        }
        match a.as_str() {
            "--cwd" => match iter.next() {
                Some(d) => cwd = Some(d.clone()),
                None => return err_exit("submit: --cwd requires a directory"),
            },
            "--tag" => match iter.next() {
                Some(t) => tags.push(t.clone()),
                None => return err_exit("submit: --tag requires a name"),
            },
            "--env" => match iter.next() {
                Some(kv) => match kv.split_once('=') {
                    Some((k, v)) => env.push((k.to_string(), v.to_string())),
                    None => return err_exit("submit: --env requires KEY=VALUE"),
                },
                None => return err_exit("submit: --env requires KEY=VALUE"),
            },
            "--" => {
                in_command = true;
            }
            other if other.starts_with('-') && command.is_empty() => {
                return err_exit(&format!("submit: unknown flag `{}`", other));
            }
            other => {
                in_command = true;
                command.push(other.to_string());
            }
        }
    }

    if command.is_empty() {
        return err_exit("submit: missing <cmd>");
    }

    let env_obj: serde_json::Map<String, Value> = env
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();

    let payload = json!({
        "command": command,
        "cwd": cwd,
        "tags": tags,
        "env": env_obj,
    });

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("job_submit", payload) {
        Ok(v) => {
            // Print just the id for easy capture: `id=$(zjob submit ...)`
            if let Some(id) = v.get("job_id").and_then(Value::as_u64) {
                println!("{}", id);
                0
            } else {
                print_pretty(&v);
                0
            }
        }
        Err(e) => err_exit(&format!("submit: {}", e)),
    }
}

fn list(args: &[String]) -> i32 {
    let mut payload = json!({});
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--state" => match iter.next() {
                Some(s) => payload["state"] = Value::String(s.clone()),
                None => return err_exit("list: --state requires a value"),
            },
            "--tag" => match iter.next() {
                Some(t) => payload["tag"] = Value::String(t.clone()),
                None => return err_exit("list: --tag requires a name"),
            },
            "--limit" => match iter.next() {
                Some(n) => match n.parse::<u64>() {
                    Ok(v) => payload["limit"] = json!(v),
                    Err(_) => return err_exit("list: --limit requires an integer"),
                },
                None => return err_exit("list: --limit requires a value"),
            },
            other if other.starts_with('-') => {
                return err_exit(&format!("list: unknown flag `{}`", other));
            }
            _ => {}
        }
    }

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("job_list", payload) {
        Ok(v) => {
            print_jobs_table(&v);
            0
        }
        Err(e) => err_exit(&format!("list: {}", e)),
    }
}

fn print_jobs_table(v: &Value) {
    let arr = match v.get("jobs").and_then(Value::as_array) {
        Some(a) => a,
        None => {
            print_pretty(v);
            return;
        }
    };
    if arr.is_empty() {
        println!("(no jobs)");
        return;
    }
    println!(
        "{:<6} {:<10} {:<8} {:<12} {:<10} {}",
        "ID", "STATE", "EXIT", "STARTED", "TAGS", "COMMAND"
    );
    for j in arr {
        let id = j.get("id").and_then(Value::as_u64).unwrap_or(0);
        let state = j.get("state").and_then(Value::as_str).unwrap_or("?");
        let exit = j
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string());
        let started = j
            .get("started_at")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let tags = j
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let tags = if tags.is_empty() {
            "-".to_string()
        } else {
            tags
        };
        let cmd = j
            .get("command")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        println!(
            "{:<6} {:<10} {:<8} {:<12} {:<10} {}",
            id, state, exit, started, tags, cmd
        );
    }
}

fn status(args: &[String]) -> i32 {
    let id = match args.first().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => return err_exit("status: missing or non-integer <id>"),
    };
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("job_status", json!({ "id": id })) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("status: {}", e)),
    }
}

fn output(args: &[String]) -> i32 {
    let mut id_opt: Option<u64> = None;
    let mut follow = false;
    let mut stderr = false;
    let mut lines: Option<u64> = None;

    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--follow" | "-f" => follow = true,
            "--stderr" => stderr = true,
            "--lines" | "-n" => match iter.next().and_then(|s| s.parse::<u64>().ok()) {
                Some(n) => lines = Some(n),
                None => return err_exit("output: --lines requires an integer"),
            },
            other if other.starts_with('-') => {
                return err_exit(&format!("output: unknown flag `{}`", other));
            }
            other => {
                if let Ok(n) = other.parse::<u64>() {
                    id_opt = Some(n);
                } else {
                    return err_exit("output: missing or non-integer <id>");
                }
            }
        }
    }
    let id = match id_opt {
        Some(n) => n,
        None => return err_exit("output: missing <id>"),
    };

    let mut payload = json!({
        "id": id,
        "stderr": stderr,
    });
    if let Some(n) = lines {
        payload["lines"] = json!(n);
    }
    if follow {
        payload["follow"] = json!(true);
    }

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };

    if !follow {
        return match client.call("job_output", payload) {
            Ok(v) => {
                if let Some(content) = v.get("content").and_then(Value::as_str) {
                    print!("{}", content);
                } else {
                    print_pretty(&v);
                }
                0
            }
            Err(e) => err_exit(&format!("output: {}", e)),
        };
    }

    // Follow mode: stream the existing content first, then subscribe to job
    // output events for new lines until the job completes.
    match client.call("job_output", payload.clone()) {
        Ok(v) => {
            if let Some(content) = v.get("content").and_then(Value::as_str) {
                print!("{}", content);
            }
        }
        Err(e) => return err_exit(&format!("output: {}", e)),
    }

    let pattern = format!("job:{}.{}", id, if stderr { "stderr" } else { "stdout" });
    let complete_pattern = format!("job:{}.complete", id);
    if let Err(e) = client.call("subscribe", json!({ "pattern": pattern })) {
        return err_exit(&format!("output --follow subscribe: {}", e));
    }
    if let Err(e) = client.call("subscribe", json!({ "pattern": complete_pattern })) {
        return err_exit(&format!("output --follow subscribe complete: {}", e));
    }
    if let Err(e) = client.set_read_timeout(None) {
        return err_exit(&format!("output --follow timeout: {}", e));
    }

    use super::ipc::Frame;
    loop {
        match client.next_frame() {
            Ok(Frame::Event { event, payload }) => {
                let topic = payload.get("topic").and_then(Value::as_str).unwrap_or("");
                if topic.ends_with(".complete") || event == "job_complete" {
                    return 0;
                }
                if let Some(line) = payload.get("data").and_then(Value::as_str) {
                    print!("{}", line);
                }
            }
            Ok(_) => continue,
            Err(DaemonError::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof) =>
            {
                return 0;
            }
            Err(e) => return err_exit(&format!("output --follow: {}", e)),
        }
    }
}

fn kill(args: &[String]) -> i32 {
    let id = match args.first().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => return err_exit("kill: missing or non-integer <id>"),
    };
    let mut signal: Option<String> = None;
    if let Some(idx) = args.iter().position(|a| a == "--signal") {
        signal = args.get(idx + 1).cloned();
    }
    let payload = match signal {
        Some(s) => json!({ "id": id, "signal": s }),
        None => json!({ "id": id }),
    };
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("job_kill", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("kill: {}", e)),
    }
}

fn wait_for(args: &[String]) -> i32 {
    let id = match args.first().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => return err_exit("wait: missing or non-integer <id>"),
    };
    let mut payload = json!({ "id": id });
    if let Some(idx) = args.iter().position(|a| a == "--timeout") {
        match args.get(idx + 1).and_then(|s| s.parse::<u64>().ok()) {
            Some(secs) => payload["timeout_ms"] = json!(secs * 1000),
            None => return err_exit("wait: --timeout requires integer seconds"),
        }
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    // wait can take arbitrarily long — set generous timeout, default 1h cap.
    if let Err(e) = client.set_read_timeout(Some(std::time::Duration::from_secs(3600))) {
        return err_exit(&format!("wait: {}", e));
    }
    match client.call("job_wait", payload) {
        Ok(v) => {
            print_pretty(&v);
            v.get("exit_code")
                .and_then(Value::as_i64)
                .map(|c| c as i32)
                .unwrap_or(0)
        }
        Err(e) => err_exit(&format!("wait: {}", e)),
    }
}
