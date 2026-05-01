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
        "zsubscribe" => zsubscribe(args),
        "zunsubscribe" => zunsubscribe(args),
        "zjob" => super::zjob_builtin::zjob(args),
        "zsync" => super::zsync_builtin::zsync(args),
        "zask" => super::zask_builtin::zask(args),
        "zhistory" => super::zhistory_builtin::zhistory(args),
        "zsource" => super::zsource_builtin::zsource(args),
        "zlog" => zlog(args),
        _ => return None,
    };
    Some(status)
}

/// Whether `name` is a daemon-managed z* builtin. Lets the shell short-circuit to
/// `dispatch` without baking the list into the call site.
pub fn is_zshrs_builtin(name: &str) -> bool {
    ZSHRS_BUILTIN_NAMES.iter().any(|n| *n == name)
}

/// Combines the name check with `dispatch`: returns the exit status if `name` is
/// one of ours, `None` otherwise. The shell core routes through this so adding a
/// new z* builtin never requires changing exec.rs.
pub fn try_dispatch(name: &str, argv: &[String]) -> Option<i32> {
    if is_zshrs_builtin(name) {
        dispatch(name, argv)
    } else {
        None
    }
}

/// Names of every z* builtin handled here, for callers that want to expose a list
/// (e.g. `which`, `whence`, `type`, completion).
pub const ZSHRS_BUILTIN_NAMES: &[&str] = &[
    "zcache",
    "zls",
    "zid",
    "zping",
    "ztag",
    "zuntag",
    "zsend",
    "znotify",
    "zsubscribe",
    "zunsubscribe",
    "zjob",
    "zsync",
    "zask",
    "zhistory",
    "zsource",
    "zlog",
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
    let rest: &[String] = if args.len() > 2 { &args[2..] } else { &[] };
    match verb {
        "info" | "" => zcache_info(),
        "daemon" => zcache_daemon(args.get(2).map(|s| s.as_str()).unwrap_or("status")),
        "rebuild" => zcache_rebuild(rest),
        "clean" => zcache_clean(rest),
        "verify" => zcache_simple_op("verify", json!({})),
        "compact" => zcache_simple_op("compact", json!({})),
        "list" => zcache_list_targets(),
        "jobs" => zcache_simple_op("info", json!({})), // info doubles as jobs view for v1
        "view" => zcache_view(rest),
        "export" => zcache_export(rest),
        "import" => zcache_import(rest),
        "first-init" => zcache_first_init(rest),
        "hydrate-view" => zcache_hydrate_view(),
        "watch" => zcache_watch(rest),
        "log" => super::builtins::zlog(args), // alias for `zlog ...`
        other => err_exit("zcache", &format!("unknown verb `{}`", other)),
    }
}

// `zcache view <target> [--format text|json|yaml|sh] [--filter <pat>]`
// Default format = text. Calls server's `view` op which renders the target.
fn zcache_view(args: &[String]) -> i32 {
    let target = match args.first() {
        Some(t) => t.clone(),
        None => return err_exit("zcache view", "usage: zcache view <target> [--format <fmt>]"),
    };
    let mut format = "text".to_string();
    let mut filter: Option<String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--format" => match iter.next() {
                Some(f) => format = f.clone(),
                None => return err_exit("zcache view", "--format requires a value"),
            },
            "--filter" => match iter.next() {
                Some(p) => filter = Some(p.clone()),
                None => return err_exit("zcache view", "--filter requires a value"),
            },
            other if other.starts_with('-') => {
                return err_exit("zcache view", &format!("unknown flag `{}`", other));
            }
            _ => {}
        }
    }
    let mut payload = json!({ "target": target, "format": format });
    if let Some(f) = filter {
        payload["filter"] = json!(f);
    }
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("view", payload) {
        Ok(v) => {
            if let Some(body) = v.get("body").and_then(Value::as_str) {
                print!("{}", body);
                if !body.ends_with('\n') {
                    println!();
                }
            } else {
                print_pretty(&v);
            }
            0
        }
        Err(e) => err_exit("zcache view", &e.to_string()),
    }
}

// `zcache export <target> [--format sh|json|yaml|native] [--additive] [--out <path>]`
// Default format = sh (eval-compatible). The canonical reset pattern:
//   eval $(zcache export aliases)
fn zcache_export(args: &[String]) -> i32 {
    // Allow leading `--all-state` / `--all` flag form before the positional
    // target (per docs/DAEMON.md `zcache export --all-state [--out <path>]`).
    let (target, args_rest): (String, &[String]) = if let Some(first) = args.first() {
        if first == "--all-state" || first == "--all" {
            ("all-state".to_string(), &args[1..])
        } else {
            (first.clone(), &args[1..])
        }
    } else {
        return err_exit(
            "zcache export",
            "usage: zcache export <target>|--all-state [--format <fmt>] [--additive] [--out <path>]",
        );
    };
    let mut format = "sh".to_string();
    let mut additive = false;
    let mut out_path: Option<String> = None;
    let mut iter = args_rest.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--format" => match iter.next() {
                Some(f) => format = f.clone(),
                None => return err_exit("zcache export", "--format requires a value"),
            },
            "--additive" => additive = true,
            "--out" => match iter.next() {
                Some(p) => out_path = Some(p.clone()),
                None => return err_exit("zcache export", "--out requires a path"),
            },
            other if other.starts_with('-') => {
                return err_exit("zcache export", &format!("unknown flag `{}`", other));
            }
            _ => {}
        }
    }
    // Dedicated server ops for non-canonical targets that need bespoke rendering.
    let (op_name, op_payload) = match target.as_str() {
        "zcompdump" => {
            let mut p = json!({});
            if let Some(o) = out_path.as_ref() {
                p["path"] = Value::String(o.clone());
            }
            ("export_zcompdump", p)
        }
        "catalog" => {
            let mut p = json!({});
            if let Some(o) = out_path.as_ref() {
                p["path"] = Value::String(o.clone());
            }
            ("export_catalog", p)
        }
        "shard" => {
            // Need a name. Last positional is the shard name.
            let name = match args.iter().skip(1).find(|a| !a.starts_with("--") && a.as_str() != args.first().map(|s| s.as_str()).unwrap_or("")) {
                Some(n) => n.clone(),
                None => return err_exit("zcache export", "shard target requires a name (zcache export shard <name>)"),
            };
            let mut p = json!({ "name": name });
            if let Some(o) = out_path.as_ref() {
                p["path"] = Value::String(o.clone());
            }
            ("export_shard", p)
        }
        _ => ("export", json!({ "target": target, "format": format, "additive": additive })),
    };

    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call(op_name, op_payload) {
        Ok(v) => {
            // Bespoke ops return file-path metadata directly (e.g. "wrote N
            // bytes to <path>") rather than a body to print. Detect by
            // absence of "body" field.
            if v.get("body").is_none() {
                print_pretty(&v);
                return 0;
            }
            let body = v.get("body").and_then(Value::as_str).unwrap_or("");
            match out_path {
                Some(p) => match std::fs::write(&p, body) {
                    Ok(()) => {
                        eprintln!("wrote {} bytes to {}", body.len(), p);
                        0
                    }
                    Err(e) => err_exit("zcache export", &format!("write {}: {}", p, e)),
                },
                None => {
                    print!("{}", body);
                    if !body.ends_with('\n') {
                        println!();
                    }
                    0
                }
            }
        }
        Err(e) => err_exit("zcache export", &e.to_string()),
    }
}

// `zcache first-init [--zshrc <path>]` — single-pass walk lifecycle:
// .zshrc analyze (Pass 1+2) + $PATH/$FPATH walk (Pass 3) + system shard
// build + entries hydrate (Pass 4). Replaces the manual
// `zcache rebuild --zshrc PATH && zcache rebuild` dance.
fn zcache_first_init(args: &[String]) -> i32 {
    let mut payload = json!({});
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--zshrc" => match iter.next() {
                Some(p) => {
                    payload["zshrc"] = Value::String(p.clone());
                }
                None => return err_exit("zcache first-init", "--zshrc requires a path"),
            },
            other if other.starts_with('-') => {
                return err_exit("zcache first-init", &format!("unknown flag `{}`", other));
            }
            _ => {}
        }
    }
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("first_init", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit("zcache first-init", &e.to_string()),
    }
}

// `zcache watch <dir>...` — register one or more directories with the
// daemon's fsnotify watcher. Used for new fpath / source-root paths that the
// daemon hasn't already discovered. Maps to the `fpath_changed` IPC op.
fn zcache_watch(args: &[String]) -> i32 {
    if args.is_empty() {
        return err_exit("zcache watch", "usage: zcache watch <dir>...");
    }
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    let payload = json!({ "paths": args });
    match client.call("fpath_changed", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit("zcache watch", &e.to_string()),
    }
}

// `zcache hydrate-view` — refresh the SQLite `canonical` view table from the
// rkyv-backed in-memory state. SQLite is the inspection mirror only; this op
// repopulates it on demand for `sqlite3 catalog.db` / `zcache view --format sql`
// consumers. Hot lookups never hit SQLite (per docs/DAEMON.md "Daemon = sole
// writer").
fn zcache_hydrate_view() -> i32 {
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("canonical_hydrate_view", json!({})) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit("zcache hydrate-view", &e.to_string()),
    }
}

// `zcache import <target> <path>`. Recognised targets: zwc, zcompdump.
// (Other targets per docs route to specific server ops; v1 supports the two
// migration-assist surfaces.)
fn zcache_import(args: &[String]) -> i32 {
    let target = match args.first() {
        Some(t) => t.as_str(),
        None => return err_exit("zcache import", "usage: zcache import <target> <path>"),
    };
    let op = match target {
        "zcompdump" => "import_zcompdump",
        "zwc" => return err_exit("zcache import", "zwc import not yet wired (v1 reads .zwc opportunistically only on user `source` of the .zwc path)"),
        other => return err_exit("zcache import", &format!("unknown target `{}` (try zcompdump|zwc)", other)),
    };
    let path = match args.get(1) {
        Some(p) => p.clone(),
        None => return err_exit("zcache import", "usage: zcache import <target> <path>"),
    };
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call(op, json!({ "path": path })) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit("zcache import", &e.to_string()),
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
    let mut zshrc: Option<String> = None;
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "shard" => shard = iter.next().cloned(),
            "--zshrc" => zshrc = iter.next().cloned(),
            "--wait" | "--parallel" => {
                let _ = iter.next();
            } // accepted but ignored in v1
            _ => {}
        }
    }

    // `zcache rebuild --zshrc <path>` runs the .zshrc analysis pass, seeds the
    // canonical table from deterministic state declarations, broadcasts
    // canonical_changed. Per docs/DAEMON.md "Walk lifecycle — first init".
    if let Some(path) = zshrc {
        return zcache_simple_op("zshrc_analyze", json!({ "path": path }));
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
        .find(|a| {
            matches!(
                a.as_str(),
                "shards" | "index" | "log" | "stats" | "shard" | "catalog"
            )
        })
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
    println!(
        "{:<6} {:<8} {:<14} {:<8} {:<10} {}",
        "ID", "PID", "TTY", "UPTIME", "TAGS", "CWD"
    );
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
        let tags = if tags.is_empty() {
            "-".to_string()
        } else {
            tags
        };
        println!(
            "{:<6} {:<8} {:<14} {:<8} {:<10} {}",
            id,
            pid,
            tty,
            format!("{}s", uptime),
            tags,
            cwd
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
    let mut all = false;
    let mut echo: Option<String> = None;
    for a in args.iter().skip(1) {
        match a.as_str() {
            "--all" => all = true,
            other if other.starts_with('-') => {
                return err_exit("zping", &format!("unknown flag `{}`", other));
            }
            other => echo = Some(other.to_string()),
        }
    }
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    let payload = match echo {
        Some(s) => json!({ "echo": s }),
        None => json!({}),
    };
    let start = std::time::Instant::now();
    let pong = match client.call("ping", payload) {
        Ok(v) => {
            let rtt = start.elapsed();
            let uptime = v
                .get("daemon_uptime_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            println!("pong from daemon (uptime {} ms, rtt {:?})", uptime, rtt);
            v
        }
        Err(e) => return err_exit("zping", &e.to_string()),
    };
    let _ = pong; // already printed

    if all {
        // --all: enumerate every connected shell and report it. The shells
        // aren't independently pingable in v1 (clients are pure consumers,
        // the daemon is the sole responder); reporting connectivity from
        // the daemon's session table is the meaningful equivalent.
        match client.call("list_shells", json!({})) {
            Ok(v) => {
                let arr = v.get("shells").and_then(Value::as_array);
                let count = arr.map(|a| a.len()).unwrap_or(0);
                println!("registered shells: {}", count);
                if let Some(arr) = arr {
                    for s in arr {
                        let id = s.get("client_id").and_then(Value::as_u64).unwrap_or(0);
                        let pid = s.get("pid").and_then(Value::as_i64).unwrap_or(0);
                        let tty = s.get("tty").and_then(Value::as_str).unwrap_or("-");
                        let uptime = s.get("uptime_secs").and_then(Value::as_u64).unwrap_or(0);
                        println!("  shell:{:<3} pid={:<6} tty={:<14} uptime={}s", id, pid, tty, uptime);
                    }
                }
            }
            Err(e) => return err_exit("zping --all", &e.to_string()),
        }
    }
    0
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
        args.iter()
            .skip(1)
            .filter(|a| !a.starts_with("--"))
            .cloned()
            .collect()
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
    let json_out = args.iter().any(|a| a == "--json");
    // Strip --json before parsing so parse_send_args doesn't see it as the
    // first positional.
    let stripped: Vec<String> = args.iter().filter(|a| *a != "--json").cloned().collect();
    let (target, command) = match parse_send_args(&stripped, "zsend") {
        Ok(v) => v,
        Err(code) => return code,
    };
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("send", json!({ "target": target, "command": command })) {
        Ok(v) => {
            if json_out {
                print_pretty(&v);
            } else {
                let count = v
                    .get("delivered_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                println!("delivered to {} shell(s)", count);
            }
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
            let count = v
                .get("delivered_count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
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
///   <name> --user <user> <text...>
fn parse_send_args(args: &[String], cmd: &str) -> Result<(Value, String), i32> {
    let mut iter = args.iter().skip(1);
    let first = match iter.next() {
        Some(s) => s.clone(),
        None => return Err(err_exit(cmd, "usage: --all|--tag <n>|--user <u>|<shell_id> <text...>")),
    };

    let (target, rest_first): (Value, Option<String>) = if first == "--all" {
        (json!({ "all": true }), None)
    } else if first == "--tag" {
        let name = iter
            .next()
            .ok_or_else(|| err_exit(cmd, "--tag requires a name"))?
            .clone();
        (json!({ "tag": name }), None)
    } else if first == "--user" {
        let name = iter
            .next()
            .ok_or_else(|| err_exit(cmd, "--user requires a username"))?
            .clone();
        (json!({ "user": name }), None)
    } else if let Ok(id) = first.parse::<u64>() {
        (json!({ "shell_id": id }), None)
    } else {
        return Err(err_exit(
            cmd,
            "first argument must be --all, --tag <n>, --user <u>, or <shell_id>",
        ));
    };

    let mut rest: Vec<String> = rest_first.into_iter().collect();
    rest.extend(iter.cloned());
    if rest.is_empty() {
        return Err(err_exit(cmd, "missing message/command text"));
    }

    Ok((target, rest.join(" ")))
}

// -------- zlog --------
//
// Most zlog verbs are pure client-side file operations against the daemon's
// log files in ~/.cache/zshrs/. Daemon-side ops would require dynamic
// EnvFilter reload (level) or appender fd handoff (rotate), neither of which
// is wired in v1; those two verbs surface a clear "restart-required" error.

fn zlog_level(args: &[String]) -> i32 {
    let directive = match args.first() {
        Some(d) => d.clone(),
        None => return err_exit(
            "zlog level",
            "usage: zlog level <directive>  (e.g. info | debug | info,fsnotify=trace)",
        ),
    };
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("log_level", json!({ "directive": directive })) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit("zlog level", &e.to_string()),
    }
}

fn zlog_rotate() -> i32 {
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("log_rotate", json!({})) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit("zlog rotate", &e.to_string()),
    }
}

fn zlog(args: &[String]) -> i32 {
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("path");
    let rest = if args.len() > 2 { &args[2..] } else { &[][..] };
    match verb {
        "path" => zlog_path(),
        "tail" => zlog_tail(rest),
        "grep" => zlog_grep(rest),
        "clear" => zlog_clear(),
        "stats" => zlog_stats(),
        "level" => zlog_level(rest),
        "rotate" => zlog_rotate(),
        _ => err_exit("zlog", &format!("unknown verb `{}`", verb)),
    }
}

fn zlog_path() -> i32 {
    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => return err_exit("zlog path", &e.to_string()),
    };
    // Walk the cache root for `zshrs.log*` files; print the newest one (the
    // bare prefix or the most recent rolled file). If none exist yet, print
    // the prefix path so callers can `ls` it.
    let files = log_files(&paths);
    match files.first() {
        Some(p) => println!("{}", p.display()),
        None => println!("{}", paths.log.display()),
    }
    0
}

fn zlog_tail(args: &[String]) -> i32 {
    let mut lines: usize = 100;
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "-n" | "--lines" => match iter.next().and_then(|s| s.parse::<usize>().ok()) {
                Some(n) => lines = n,
                None => return err_exit("zlog tail", "-n requires an integer"),
            },
            other if other.starts_with('-') => {
                return err_exit("zlog tail", &format!("unknown flag `{}`", other));
            }
            _ => {}
        }
    }

    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => return err_exit("zlog tail", &e.to_string()),
    };
    let files = log_files(&paths);
    let mut buf: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(lines);
    for f in files.iter().rev() {
        let content = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for line in content.lines() {
            if buf.len() == lines {
                buf.pop_front();
            }
            buf.push_back(line.to_string());
        }
    }
    for line in buf {
        println!("{}", line);
    }
    0
}

fn zlog_grep(args: &[String]) -> i32 {
    let pattern = match args.first() {
        Some(p) => p.clone(),
        None => return err_exit("zlog grep", "usage: zlog grep <pattern>"),
    };
    let re = match regex::Regex::new(&pattern) {
        Ok(r) => r,
        Err(e) => return err_exit("zlog grep", &format!("bad regex: {}", e)),
    };

    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => return err_exit("zlog grep", &e.to_string()),
    };
    let files = log_files(&paths);
    let mut hits = 0u64;
    for f in files.iter().rev() {
        let content = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let name = f.file_name().and_then(|s| s.to_str()).unwrap_or("");
        for line in content.lines() {
            if re.is_match(line) {
                println!("{}: {}", name, line);
                hits += 1;
            }
        }
    }
    if hits == 0 {
        1
    } else {
        0
    }
}

fn zlog_clear() -> i32 {
    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => return err_exit("zlog clear", &e.to_string()),
    };
    let files = log_files(&paths);
    let mut cleared = 0;
    for f in &files {
        // Truncate rather than unlink — tracing-appender holds an fd on the
        // active file, and an unlink leaves it as a write-only "deleted"
        // inode that consumes disk until daemon restart.
        if std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(f)
            .is_ok()
        {
            cleared += 1;
        }
    }
    println!("cleared {} log file(s)", cleared);
    0
}

fn zlog_stats() -> i32 {
    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => return err_exit("zlog stats", &e.to_string()),
    };
    let files = log_files(&paths);
    let mut total_bytes: u64 = 0;
    let mut total_lines: u64 = 0;
    println!("{:<40} {:>12} {:>10}", "FILE", "BYTES", "LINES");
    for f in &files {
        let bytes = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
        let lines = std::fs::read_to_string(f)
            .map(|s| s.lines().count() as u64)
            .unwrap_or(0);
        total_bytes += bytes;
        total_lines += lines;
        let name = f.file_name().and_then(|s| s.to_str()).unwrap_or("");
        println!("{:<40} {:>12} {:>10}", name, bytes, lines);
    }
    println!(
        "{:<40} {:>12} {:>10}",
        format!("(total: {} files)", files.len()),
        total_bytes,
        total_lines
    );
    0
}

/// Enumerate `~/.cache/zshrs/zshrs.log*` files, newest first by mtime. Used by
/// the read-only zlog verbs (tail, grep, stats) and by `zlog clear`.
fn log_files(paths: &CachePaths) -> Vec<std::path::PathBuf> {
    let dir = match std::fs::read_dir(&paths.root) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s.starts_with("zshrs.log") {
            let mtime = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            out.push((mtime, entry.path()));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, p)| p).collect()
}

// -------- zsubscribe --------
//
// Streaming foreground consumer. Holds the daemon connection open for the
// lifetime of the process; on Ctrl-C / EOF the connection drops and the
// daemon's `unregister_session` automatically removes the subscription.
//
// Forms:
//   zsubscribe <pattern>             # default human format
//   zsubscribe --json <pattern>      # one raw JSON object per event
//   zsubscribe --count N <pattern>   # exit after N events
//   zsubscribe --list                # this client's existing subs (then exit)

fn zsubscribe(args: &[String]) -> i32 {
    let mut json_out = false;
    let mut list_only = false;
    let mut pause = false;
    let mut resume = false;
    let mut sub_id: Option<u64> = None;
    let mut all = false;
    let mut count: Option<u64> = None;
    let mut pattern: Option<String> = None;
    let mut filter: Option<FilterPredicate> = None;

    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--json" => json_out = true,
            "--list" => list_only = true,
            "--pause" => pause = true,
            "--resume" => resume = true,
            "--all" => all = true,
            "--id" => match iter.next().and_then(|s| s.parse::<u64>().ok()) {
                Some(n) => sub_id = Some(n),
                None => return err_exit("zsubscribe", "--id requires an integer"),
            },
            "--count" => match iter.next() {
                Some(n) => match n.parse::<u64>() {
                    Ok(v) => count = Some(v),
                    Err(_) => return err_exit("zsubscribe", "--count requires an integer"),
                },
                None => return err_exit("zsubscribe", "--count requires a value"),
            },
            "--filter" => match iter.next() {
                Some(expr) => match FilterPredicate::parse(expr) {
                    Ok(p) => filter = Some(p),
                    Err(e) => return err_exit("zsubscribe", &format!("--filter: {}", e)),
                },
                None => return err_exit("zsubscribe", "--filter requires <expr>"),
            },
            "-h" | "--help" => {
                println!("usage: zsubscribe [--json] [--count N] <pattern>");
                println!("       zsubscribe --list");
                println!("       zsubscribe --pause [--id N | --all]");
                println!("       zsubscribe --resume [--id N | --all]");
                println!("pattern: <scope>.<topic>  e.g. shell:42.commands  *.chpwd  tag:prod.long_cmd_complete");
                return 0;
            }
            other if other.starts_with('-') => {
                return err_exit("zsubscribe", &format!("unknown flag `{}`", other));
            }
            other => {
                if pattern.is_some() {
                    return err_exit("zsubscribe", "expected exactly one pattern");
                }
                pattern = Some(other.to_string());
            }
        }
    }

    if pause || resume {
        return zsubscribe_set_paused(pause, sub_id, all);
    }

    if list_only {
        return zsubscribe_list();
    }

    let pattern = match pattern {
        Some(p) => p,
        None => return err_exit("zsubscribe", "missing <pattern>"),
    };

    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };

    let sub_id = match client.call("subscribe", json!({ "pattern": pattern })) {
        Ok(v) => v.get("subscription_id").and_then(Value::as_u64).unwrap_or(0),
        Err(e) => return err_exit("zsubscribe", &e.to_string()),
    };

    if let Err(e) = client.set_read_timeout(None) {
        return err_exit("zsubscribe", &format!("set timeout: {}", e));
    }

    eprintln!(
        "zsubscribe: id={} pattern={} (Ctrl-C to exit)",
        sub_id, pattern
    );

    let mut delivered = 0u64;
    use super::ipc::Frame;
    loop {
        match client.next_frame() {
            Ok(Frame::Event { event, payload }) => {
                // Apply --filter (if any) before counting/printing.
                if let Some(ref pred) = filter {
                    if !pred.matches(&payload) {
                        continue;
                    }
                }
                if json_out {
                    let line = json!({ "event": event, "payload": payload });
                    println!("{}", line);
                } else {
                    print_event_human(&event, &payload);
                }
                delivered += 1;
                if let Some(limit) = count {
                    if delivered >= limit {
                        return 0;
                    }
                }
            }
            Ok(_) => continue,
            Err(DaemonError::Io(e))
                if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof) =>
            {
                eprintln!("zsubscribe: daemon closed connection");
                return 0;
            }
            Err(e) => return err_exit("zsubscribe", &e.to_string()),
        }
    }
}

fn zsubscribe_set_paused(pause: bool, sub_id: Option<u64>, all: bool) -> i32 {
    let mut payload = json!({ "paused": pause });
    if all {
        payload["all"] = json!(true);
    } else if let Some(id) = sub_id {
        payload["id"] = json!(id);
    } else {
        return err_exit(
            "zsubscribe",
            "--pause/--resume requires --id <N> or --all",
        );
    }
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("subscription_set_paused", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit("zsubscribe", &e.to_string()),
    }
}

fn zsubscribe_list() -> i32 {
    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("subscribe", json!({ "pattern": "--list" })) {
        Ok(v) => {
            let subs = v.get("subscriptions").cloned().unwrap_or(Value::Null);
            print_pretty(&subs);
            0
        }
        Err(e) => err_exit("zsubscribe --list", &e.to_string()),
    }
}

/// `--filter` predicate parser. Accepts `<key> <op> <value>` where:
///   - key is a JSON path, dot-separated. `payload.duration_ns` walks down two
///     levels; `event` is also valid against the synthetic top-level event
///     name. Bare `duration_ns` is shorthand for `payload.duration_ns`.
///   - op is one of `>`, `>=`, `<`, `<=`, `=`, `==`, `!=`.
///   - value is parsed as: integer | float (with `e` notation) | quoted
///     string | bare string.
#[derive(Clone, Debug)]
struct FilterPredicate {
    path: Vec<String>,
    op: FilterOp,
    needle: FilterValue,
}

#[derive(Clone, Copy, Debug)]
enum FilterOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Clone, Debug)]
enum FilterValue {
    Number(f64),
    Text(String),
}

impl FilterPredicate {
    fn parse(expr: &str) -> Result<Self, String> {
        // Split on the first op token. Order of checks matters: longer
        // operators first so `>=` doesn't get parsed as `>`.
        let s = expr.trim();
        let (key_raw, op, val_raw) = if let Some(i) = s.find(">=") {
            (&s[..i], FilterOp::Ge, &s[i + 2..])
        } else if let Some(i) = s.find("<=") {
            (&s[..i], FilterOp::Le, &s[i + 2..])
        } else if let Some(i) = s.find("==") {
            (&s[..i], FilterOp::Eq, &s[i + 2..])
        } else if let Some(i) = s.find("!=") {
            (&s[..i], FilterOp::Ne, &s[i + 2..])
        } else if let Some(i) = s.find('>') {
            (&s[..i], FilterOp::Gt, &s[i + 1..])
        } else if let Some(i) = s.find('<') {
            (&s[..i], FilterOp::Lt, &s[i + 1..])
        } else if let Some(i) = s.find('=') {
            (&s[..i], FilterOp::Eq, &s[i + 1..])
        } else {
            return Err(format!("no operator in `{}` (try `key=value` or `key>N`)", expr));
        };
        let key = key_raw.trim();
        let val = val_raw.trim();
        if key.is_empty() {
            return Err("empty key before operator".into());
        }
        if val.is_empty() {
            return Err("empty value after operator".into());
        }
        // Bare keys default to walking under `payload.`.
        let path: Vec<String> = if key.contains('.') {
            key.split('.').map(str::to_string).collect()
        } else {
            vec!["payload".to_string(), key.to_string()]
        };
        let needle = if let Ok(n) = val.parse::<f64>() {
            FilterValue::Number(n)
        } else {
            FilterValue::Text(val.trim_matches(|c: char| c == '"' || c == '\'').to_string())
        };
        Ok(Self {
            path,
            op,
            needle,
        })
    }

    fn matches(&self, payload: &Value) -> bool {
        // Synthetic top-level: tree starts at the WIRE FRAME (`{event, payload}`).
        // Our caller passes only `payload` here, so we wrap it for path lookup.
        let mut cursor = payload;
        for seg in &self.path {
            if seg == "payload" {
                continue; // already at payload
            }
            cursor = match cursor.get(seg) {
                Some(v) => v,
                None => return false,
            };
        }
        match (&self.needle, cursor) {
            (FilterValue::Number(n), Value::Number(v)) => {
                let lhs = v.as_f64().unwrap_or(f64::NAN);
                self.cmp_num(lhs, *n)
            }
            (FilterValue::Number(n), Value::String(s)) => match s.parse::<f64>() {
                Ok(lhs) => self.cmp_num(lhs, *n),
                Err(_) => false,
            },
            (FilterValue::Text(t), Value::String(s)) => self.cmp_str(s, t),
            (FilterValue::Text(t), other) => self.cmp_str(&other.to_string(), t),
            (_, Value::Null) => matches!(self.op, FilterOp::Ne),
            _ => false,
        }
    }

    fn cmp_num(&self, lhs: f64, rhs: f64) -> bool {
        match self.op {
            FilterOp::Eq => lhs == rhs,
            FilterOp::Ne => lhs != rhs,
            FilterOp::Gt => lhs > rhs,
            FilterOp::Ge => lhs >= rhs,
            FilterOp::Lt => lhs < rhs,
            FilterOp::Le => lhs <= rhs,
        }
    }

    fn cmp_str(&self, lhs: &str, rhs: &str) -> bool {
        match self.op {
            FilterOp::Eq => lhs == rhs,
            FilterOp::Ne => lhs != rhs,
            FilterOp::Gt => lhs > rhs,
            FilterOp::Ge => lhs >= rhs,
            FilterOp::Lt => lhs < rhs,
            FilterOp::Le => lhs <= rhs,
        }
    }
}

fn print_event_human(event: &str, payload: &Value) {
    let scope = payload.get("scope").and_then(Value::as_str).unwrap_or("-");
    let topic = payload
        .get("topic")
        .and_then(Value::as_str)
        .unwrap_or(event);
    let data = payload
        .get("data")
        .map(|v| {
            if v.is_string() {
                v.as_str().unwrap_or("").to_string()
            } else {
                v.to_string()
            }
        })
        .unwrap_or_default();
    let ts = chrono::Local::now().format("%H:%M:%S");
    println!("[{} {} {}] {}", ts, scope, topic, data);
}

// -------- zunsubscribe --------
//
// Removes a subscription owned by THIS client. When invoked one-shot from the
// CLI, "this client" is a fresh ephemeral session that has no subscriptions, so
// the call is mostly useful from within a long-lived shell that maintains its
// own daemon connection. For one-shot CLI use, exit a foreground `zsubscribe`
// via Ctrl-C — the disconnect auto-clears the subscription.

fn zunsubscribe(args: &[String]) -> i32 {
    let mut by_id: Option<u64> = None;
    let mut pattern: Option<String> = None;

    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--id" => match iter.next() {
                Some(n) => match n.parse::<u64>() {
                    Ok(v) => by_id = Some(v),
                    Err(_) => return err_exit("zunsubscribe", "--id requires an integer"),
                },
                None => return err_exit("zunsubscribe", "--id requires a value"),
            },
            "-h" | "--help" => {
                println!("usage: zunsubscribe <pattern>");
                println!("       zunsubscribe --id <subscription_id>");
                return 0;
            }
            other if other.starts_with('-') => {
                return err_exit("zunsubscribe", &format!("unknown flag `{}`", other));
            }
            other => {
                if pattern.is_some() {
                    return err_exit("zunsubscribe", "expected exactly one pattern");
                }
                pattern = Some(other.to_string());
            }
        }
    }

    let payload = match (by_id, pattern) {
        (Some(id), _) => json!({ "id": id }),
        (None, Some(p)) => json!({ "pattern": p }),
        (None, None) => return err_exit("zunsubscribe", "missing pattern or --id"),
    };

    let mut client = match connect_or_err() {
        Ok(c) => c,
        Err(()) => return 1,
    };

    match client.call("unsubscribe", payload) {
        Ok(v) => {
            let removed = v.get("removed").and_then(Value::as_u64).unwrap_or(0);
            println!("removed {} subscription(s)", removed);
            if removed == 0 {
                1
            } else {
                0
            }
        }
        Err(e) => err_exit("zunsubscribe", &e.to_string()),
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
    fn try_dispatch_unknown_returns_none() {
        assert!(try_dispatch("ls", &["ls".into()]).is_none());
        assert!(try_dispatch("not_a_zthing", &["not_a_zthing".into()]).is_none());
    }

    #[test]
    fn is_zshrs_builtin_recognises_full_namespace() {
        for n in &[
            "zcache",
            "zls",
            "zid",
            "zping",
            "ztag",
            "zuntag",
            "zsend",
            "znotify",
            "zsubscribe",
            "zunsubscribe",
            "zjob",
            "zsync",
            "zask",
            "zlog",
        ] {
            assert!(is_zshrs_builtin(n), "expected {n} to be recognised");
        }
        assert!(!is_zshrs_builtin("ls"));
        assert!(!is_zshrs_builtin(""));
    }

    #[test]
    fn zshrs_builtin_names_no_zsh_clash() {
        // Per docs/DAEMON.md "z* builtin family (locked, no shadowing of zsh)".
        let zsh_owned: &[&str] = &[
            "zmv",
            "zparseopts",
            "zformat",
            "zstat",
            "zstyle",
            "zprof",
            "zcompile",
            "zargs",
            "zcurses",
            "zsystem",
            "ztie",
            "zuntie",
            "zselect",
            "zsocket",
            "zftp",
            "zpty",
            "zed",
            "zcalc",
            "zregexparse",
            "zutil",
            "zmodload",
            "zle",
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
