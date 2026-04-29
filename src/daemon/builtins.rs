// z* builtins — thin IPC wrappers per docs/DAEMON.md "z* builtin family".
//
// Every builtin connects to the daemon, sends one op, and prints the response.
// Spawn-on-demand happens automatically inside Client::connect.
//
// Naming check (per docs/DAEMON.md "z* builtin family (locked, no shadowing of zsh)"):
//   zsh-owned (do not shadow): zmv, zparseopts, zformat, zstat, zstyle, zprof,
//                              zcompile, zargs, zcurses, zsystem, ztie, zuntie,
//                              zselect, zsocket, zftp, zpty, zed, zcalc,
//                              zregexparse, zutil, zmodload, zle.
//   zshrs-owned: zcache, zls, zid, zping, ztag, zuntag, zsend, znotify,
//                zsubscribe, zunsubscribe, zjob (planned), zlog, zsync, zask.
//
// Foundation v1 implements: zcache (info / daemon status / daemon stop), zls, zid,
// zping, ztag, zuntag, zsend, znotify, zlog (path/level shortcut). Everything else
// returns "not yet implemented" via the daemon's stub responses.

use serde_json::{json, Value};

use super::client::Client;
use super::paths::CachePaths;
use super::DaemonError;

/// Dispatch by builtin name. Returns Some(exit_status) if the name is daemon-managed,
/// None otherwise (caller falls through to "not a shell builtin").
pub fn dispatch(cmd: &str, args: &[String]) -> Option<i32> {
    let status = match cmd {
        "zcache" => zcache(args),
        "zls" => zls(args),
        "zid" => zid(args),
        "zping" => zping(args),
        "ztag" => ztag(args),
        "zuntag" => zuntag(args),
        "zsend" => zsend(args),
        "znotify" => znotify(args),
        "zlog" => zlog(args),
        _ => return None,
    };
    Some(status)
}

/// Names of every z* builtin handled here, for callers that want to expose a list
/// (e.g. `which`, `whence`, `type`, completion).
pub const ZSHRS_BUILTIN_NAMES: &[&str] = &[
    "zcache", "zls", "zid", "zping", "ztag", "zuntag", "zsend", "znotify", "zlog",
];

/// Helper: open a client connection (spawn-on-demand) and return it. Reports the error
/// to stderr in zsh-style and returns Err(()) on failure so callers can exit with 1.
fn connect_or_err() -> Result<Client, ()> {
    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("zshrs: daemon: {}", e);
            return Err(());
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("zshrs: daemon: {}", e);
        return Err(());
    }
    Client::connect(&paths).map_err(|e| {
        eprintln!("zshrs: daemon: {}", e);
    })
}

fn print_pretty(v: &Value) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{}", s),
        Err(_) => println!("{}", v),
    }
}

fn err_exit(code: &str, msg: &str) -> i32 {
    eprintln!("zshrs: {}: {}", code, msg);
    1
}

// -------- zcache --------

fn zcache(args: &[String]) -> i32 {
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("info");
    match verb {
        "info" | "" => zcache_info(),
        "daemon" => zcache_daemon(args.get(2).map(|s| s.as_str()).unwrap_or("status")),
        "rebuild" => zcache_rebuild(&args[2..]),
        "clean" => zcache_clean(&args[2..]),
        "verify" => zcache_simple_op("verify", json!({})),
        "compact" => zcache_simple_op("compact", json!({})),
        "list" => zcache_list_targets(),
        "jobs" => zcache_simple_op("info", json!({})), // info doubles as jobs view for v1
        "view" | "export" | "import" | "log" => err_exit(
            "zcache",
            &format!("`{}` not yet implemented in v1 foundation", verb),
        ),
        other => err_exit("zcache", &format!("unknown verb `{}`", other)),
    }
}

fn zcache_info() -> i32 {
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("info", json!({})) {
        Ok(payload) => {
            print_pretty(&payload);
            0
        }
        Err(e) => err_exit("zcache info", &e.to_string()),
    }
}

fn zcache_daemon(verb: &str) -> i32 {
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    let args = json!({ "verb": verb });
    match client.call("daemon", args) {
        Ok(payload) => {
            print_pretty(&payload);
            0
        }
        Err(e) => err_exit("zcache daemon", &e.to_string()),
    }
}

fn zcache_rebuild(args: &[String]) -> i32 {
    let mut shard: Option<String> = None;
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "shard" => shard = iter.next().cloned(),
            "--wait" | "--parallel" => {
                let _ = iter.next();
            } // accepted but ignored in v1
            _ => {}
        }
    }
    let payload = match shard {
        Some(s) => json!({ "shard": s }),
        None => json!({}),
    };
    zcache_simple_op("rebuild", payload)
}

fn zcache_clean(args: &[String]) -> i32 {
    // zcache clean [shards|index|log|--all]
    let target = args
        .iter()
        .find(|a| matches!(a.as_str(), "shards" | "index" | "log" | "stats" | "shard" | "catalog"))
        .map(|s| s.as_str())
        .unwrap_or_else(|| {
            if args.iter().any(|a| a == "--all") {
                "all"
            } else {
                "all"
            }
        });
    zcache_simple_op("clean", json!({ "target": target }))
}

fn zcache_simple_op(op: &str, args: Value) -> i32 {
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call(op, args) {
        Ok(payload) => {
            print_pretty(&payload);
            0
        }
        Err(e) => err_exit(&format!("zcache {}", op), &e.to_string()),
    }
}

fn zcache_list_targets() -> i32 {
    // Static list — every named export target the daemon supports. Hand-maintained
    // until we wire `zcache view`/`zcache export` end-to-end. Matches docs/DAEMON.md
    // "Universal cache dump / view / export" Targets table.
    let targets = &[
        "path",
        "fpath",
        "manpath",
        "infopath",
        "cdpath",
        "ld_library_path",
        "named_dir",
        "command_hash",
        "autoload_table",
        "aliases",
        "galiases",
        "saliases",
        "functions",
        "_comps",
        "_services",
        "_patcomps",
        "_describe_handlers",
        "zstyle",
        "bindkey",
        "setopt",
        "zmodload",
        "env",
        "params",
        "theme",
        "history",
        "entry_stats",
        "subscriptions",
        "shells",
        "plugins",
        "shard",
        "index",
        "catalog",
        "script",
        "sourced",
        "compiled_files",
        "zcompdump",
        "daemon_state",
    ];
    for t in targets {
        println!("{}", t);
    }
    0
}

// -------- zls --------

fn zls(args: &[String]) -> i32 {
    let mut tag_filter: Option<String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--tag" => {
                if let Some(t) = iter.next() {
                    tag_filter = Some(t.clone());
                } else {
                    return err_exit("zls", "--tag requires a name");
                }
            }
            other if other.starts_with('-') => {
                return err_exit("zls", &format!("unknown flag `{}`", other));
            }
            _ => {}
        }
    }

    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };

    let args_payload = match tag_filter {
        Some(t) => json!({ "tag": t }),
        None => json!({}),
    };

    match client.call("list_shells", args_payload) {
        Ok(v) => {
            let shells = v.get("shells").cloned().unwrap_or(Value::Null);
            print_shells_table(&shells);
            0
        }
        Err(e) => err_exit("zls", &e.to_string()),
    }
}

fn print_shells_table(shells: &Value) {
    let arr = match shells.as_array() {
        Some(a) => a,
        None => {
            println!("(no shells)");
            return;
        }
    };
    if arr.is_empty() {
        println!("(no shells)");
        return;
    }
    println!("{:<6} {:<8} {:<14} {:<8} {:<10} {}", "ID", "PID", "TTY", "UPTIME", "TAGS", "CWD");
    for s in arr {
        let id = s.get("client_id").and_then(Value::as_u64).unwrap_or(0);
        let pid = s.get("pid").and_then(Value::as_i64).unwrap_or(0);
        let tty = s.get("tty").and_then(Value::as_str).unwrap_or("-");
        let uptime = s.get("uptime_secs").and_then(Value::as_u64).unwrap_or(0);
        let cwd = s.get("cwd").and_then(Value::as_str).unwrap_or("-");
        let tags = s
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let tags = if tags.is_empty() { "-".to_string() } else { tags };
        println!(
            "{:<6} {:<8} {:<14} {:<8} {:<10} {}",
            id, pid, tty, format!("{}s", uptime), tags, cwd
        );
    }
}

// -------- zid --------

fn zid(_args: &[String]) -> i32 {
    let client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    println!("{}", client.welcome.client_id);
    drop(client); // hold for full RTT, then close cleanly
    0
}

// -------- zping --------

fn zping(args: &[String]) -> i32 {
    let echo = args.get(1).cloned();
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    let payload = if let Some(s) = echo {
        json!({ "echo": s })
    } else {
        json!({})
    };
    let start = std::time::Instant::now();
    match client.call("ping", payload) {
        Ok(v) => {
            let rtt = start.elapsed();
            let uptime = v.get("daemon_uptime_ms").and_then(Value::as_u64).unwrap_or(0);
            println!(
                "pong from daemon (uptime {} ms, rtt {:?})",
                uptime, rtt
            );
            0
        }
        Err(e) => err_exit("zping", &e.to_string()),
    }
}

// -------- ztag / zuntag --------

fn ztag(args: &[String]) -> i32 {
    if args.len() <= 1 {
        return err_exit("ztag", "usage: ztag <tag>...");
    }
    let tags: Vec<String> = args.iter().skip(1).cloned().collect();
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("tag", json!({ "tags": tags })) {
        Ok(v) => {
            let updated = v.get("tags").cloned().unwrap_or(Value::Null);
            print_pretty(&updated);
            0
        }
        Err(e) => err_exit("ztag", &e.to_string()),
    }
}

fn zuntag(args: &[String]) -> i32 {
    let all = args.iter().any(|a| a == "--all");
    let tags: Vec<String> = if all {
        Vec::new()
    } else {
        args.iter().skip(1).filter(|a| !a.starts_with("--")).cloned().collect()
    };
    if !all && tags.is_empty() {
        return err_exit("zuntag", "usage: zuntag <tag>... | --all");
    }
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("untag", json!({ "all": all, "tags": tags })) {
        Ok(v) => {
            let remaining = v.get("tags").cloned().unwrap_or(Value::Null);
            print_pretty(&remaining);
            0
        }
        Err(e) => err_exit("zuntag", &e.to_string()),
    }
}

// -------- zsend --------

fn zsend(args: &[String]) -> i32 {
    let (target, command) = match parse_send_args(args, "zsend") {
        Ok(v) => v,
        Err(code) => return code,
    };
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("send", json!({ "target": target, "command": command })) {
        Ok(v) => {
            let count = v.get("delivered_count").and_then(Value::as_u64).unwrap_or(0);
            println!("delivered to {} shell(s)", count);
            0
        }
        Err(e) => err_exit("zsend", &e.to_string()),
    }
}

fn znotify(args: &[String]) -> i32 {
    let (target, message) = match parse_send_args(args, "znotify") {
        Ok(v) => v,
        Err(code) => return code,
    };
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call(
        "notify",
        json!({ "target": target, "message": message, "urgency": "normal" }),
    ) {
        Ok(v) => {
            let count = v.get("delivered_count").and_then(Value::as_u64).unwrap_or(0);
            println!("notified {} shell(s)", count);
            0
        }
        Err(e) => err_exit("znotify", &e.to_string()),
    }
}

/// Parse zsend/znotify-style args.
/// Forms:
///   <name> <shell_id> <text...>
///   <name> --all <text...>
///   <name> --tag <name> <text...>
fn parse_send_args(args: &[String], cmd: &str) -> Result<(Value, String), i32> {
    let mut iter = args.iter().skip(1);
    let first = match iter.next() {
        Some(s) => s.clone(),
        None => return Err(err_exit(cmd, "usage: --all|--tag <n>|<shell_id> <text...>")),
    };

    let (target, rest_first): (Value, Option<String>) = if first == "--all" {
        (json!({ "all": true }), None)
    } else if first == "--tag" {
        let name = iter
            .next()
            .ok_or_else(|| err_exit(cmd, "--tag requires a name"))?
            .clone();
        (json!({ "tag": name }), None)
    } else if let Ok(id) = first.parse::<u64>() {
        (json!({ "shell_id": id }), None)
    } else {
        // Treat as a literal text token; require at least one more arg as the target type.
        return Err(err_exit(cmd, "first argument must be --all, --tag <n>, or <shell_id>"));
    };

    let mut rest: Vec<String> = rest_first.into_iter().collect();
    rest.extend(iter.cloned());
    if rest.is_empty() {
        return Err(err_exit(cmd, "missing message/command text"));
    }

    Ok((target, rest.join(" ")))
}

// -------- zlog --------

fn zlog(args: &[String]) -> i32 {
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("path");
    match verb {
        "path" => zlog_path(),
        "tail" | "grep" | "level" | "clear" | "rotate" | "stats" => err_exit(
            "zlog",
            &format!("`{}` not yet implemented in v1 foundation; use `tail` on the path", verb),
        ),
        _ => err_exit("zlog", &format!("unknown verb `{}`", verb)),
    }
}

fn zlog_path() -> i32 {
    match CachePaths::resolve() {
        Ok(p) => {
            // Pick the most recent rolled file matching the prefix, or the bare prefix path.
            // tracing-appender writes daily-rotated files like `zshrs.log.2026-04-29`. Since
            // we don't know today's filename a priori, fall back to printing the directory
            // and prefix and let the user `ls` it for now.
            println!("{}", p.log.display());
            0
        }
        Err(e) => err_exit("zlog path", &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_unknown_returns_none() {
        assert!(dispatch("not_a_zthing", &["not_a_zthing".into()]).is_none());
    }

    #[test]
    fn parse_send_args_shell_id() {
        let args: Vec<String> = vec!["zsend".into(), "42".into(), "git".into(), "status".into()];
        let (target, msg) = parse_send_args(&args, "zsend").unwrap();
        assert_eq!(target, json!({ "shell_id": 42 }));
        assert_eq!(msg, "git status");
    }

    #[test]
    fn parse_send_args_all() {
        let args: Vec<String> = vec!["zsend".into(), "--all".into(), "echo".into(), "hi".into()];
        let (target, msg) = parse_send_args(&args, "zsend").unwrap();
        assert_eq!(target, json!({ "all": true }));
        assert_eq!(msg, "echo hi");
    }

    #[test]
    fn parse_send_args_tag() {
        let args: Vec<String> = vec![
            "zsend".into(),
            "--tag".into(),
            "prod".into(),
            "deploy".into(),
        ];
        let (target, msg) = parse_send_args(&args, "zsend").unwrap();
        assert_eq!(target, json!({ "tag": "prod" }));
        assert_eq!(msg, "deploy");
    }

    #[test]
    fn zshrs_builtin_names_no_zsh_clash() {
        // Per docs/DAEMON.md "z* builtin family (locked, no shadowing of zsh)".
        let zsh_owned: &[&str] = &[
            "zmv", "zparseopts", "zformat", "zstat", "zstyle", "zprof", "zcompile",
            "zargs", "zcurses", "zsystem", "ztie", "zuntie", "zselect", "zsocket",
            "zftp", "zpty", "zed", "zcalc", "zregexparse", "zutil", "zmodload", "zle",
        ];
        for name in ZSHRS_BUILTIN_NAMES {
            assert!(
                !zsh_owned.contains(name),
                "zshrs builtin `{}` collides with zsh-owned namespace",
                name
            );
        }
    }
}

// Used by callers that want a no-op suppress for unused-import warnings.
fn _unused(_: DaemonError) {}
