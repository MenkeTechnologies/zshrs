//! `zd` subcommand dispatch — transport-agnostic.
//!
//! Two callers:
//!   1. `bins/zd.rs` — standalone binary, uses an HTTP transport (ureq).
//!      For when the user has no shell or wants `zd` portable.
//!   2. `daemon/builtins.rs::zwhere` peer — `zd` shell builtin, uses a
//!      Unix-socket transport via the existing `Client`. **No fork**:
//!      the shell process directly calls `dispatch()` and gets the
//!      result back as a String. ~5-15× faster than fork+exec'ing the
//!      bin for short ops.
//!
//! Both call sites share the same arg parser + USAGE text + op→body
//! mapping. The only thing that differs is the transport — passed in
//! as `&mut dyn Transport`.

use serde_json::{json, Value};

pub const USAGE: &str = "\
zd — client for zshrs-daemon. Builtin (in-process, no fork) when
called from inside zshrs; standalone binary otherwise.

USAGE
    zd [GLOBAL OPTS] <COMMAND> [ARGS]

GLOBAL OPTS
    --url URL          override $DAEMON_URL (default http://127.0.0.1:7733)
                       — only honored by the standalone-binary form;
                       the builtin always uses the local Unix socket
    --token TOKEN      override $DAEMON_TOKEN (binary only)
    -h, --help         this message
    --version          version + exit

COMMANDS (top-level)
    health                         GET /health
    ops                            GET /ops
    info                           daemon snapshot
    ping [ECHO_ARGS...]            round-trip latency
    metrics                        Prometheus-shaped metrics (JSON)
    call OP [JSON_BODY]            generic op caller for anything not below

CACHE
    cache put NS KEY VALUE [--ttl SECS]
    cache get NS KEY
    cache del NS KEY
    cache list NS [PREFIX]
    cache stats [NS]

JOB
    job submit -- CMD [ARGS...]    submit cmd; prints job_id
    job status ID
    job output ID [--stderr]
    job list [--state S] [--tag T] [--limit N]
    job kill ID
    job wait ID                    blocks until terminal

LOCK
    lock acquire NAME [--timeout SECS]
    lock try NAME
    lock release NAME TOKEN
    lock list

EVENT / WATCH
    publish TOPIC JSON_DATA
    events [PATTERN]               streams SSE — binary only
    watch DIR [--recursive]        streams SSE — binary only

DEFINITIONS (federated catalog)
    defs query [--kind K] [--name N] [--prefix P] [--shell-id S] [--limit N]
    defs kinds
    defs emit --shell-id S --kind K --name N [--value V] [--file F] [--line L]
    defs diff SHELL_A SHELL_B [KIND]

SNAPSHOT
    snapshot save TAG [--notes N]
    snapshot list
    snapshot load TAG
    snapshot diff A B

ARTIFACT
    artifact put NAME VALUE | artifact put NAME --file PATH
    artifact get NAME [-o OUT]     writes value to OUT (default stdout)
    artifact list [PREFIX]
    artifact gc [--max-age SECS] [--max-bytes N]

SCHEDULE
    schedule add CRON_EXPR -- CMD [ARGS...]
    schedule add-once UNIX_SECS -- CMD [ARGS...]
    schedule list
    schedule remove ID

EXPORT
    export TARGET FORMAT           formats: sh|json|yaml|text|csv|sql|pdf|...
    view TARGET [FORMAT]
";

/// Transport that backs op/get/sse calls. Implemented twice:
/// `HttpTransport` in `bins/zd.rs` (ureq, talks to daemon's HTTP
/// listener) and `SocketTransport` in `daemon/builtins.rs` (uses the
/// existing `Client`, talks to the local Unix socket).
///
/// Returns the response body as a String — both transports JSON-encode
/// internally and the dispatch layer doesn't care about the wire.
pub trait Transport {
    fn post(&mut self, op: &str, body: Value) -> Result<String, String>;
    fn get(&mut self, path: &str) -> Result<String, String>;
    /// SSE streams. Implementations that can't stream (in-process
    /// builtin) return an error pointing the user at the binary form.
    fn sse(&mut self, path: &str) -> Result<String, String>;
}

/// Top-level dispatcher. Parses global flags, routes to the right
/// `cmd_*` handler, prints the result. Returns process-exit-style
/// status: 0 = success, 1 = transport/protocol error, 2 = usage error.
pub fn dispatch(args: &[String], t: &mut dyn Transport) -> i32 {
    if args.is_empty() {
        eprintln!("{USAGE}");
        return 2;
    }

    // Strip global flags. Anything starting with `-` before the first
    // bare word is a global flag. The transport itself owns --url /
    // --token at construction time; we just consume them here for
    // arg-position-correctness and ignore the values.
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" | "--token" => {
                if i + 1 >= args.len() {
                    return usage_err(&format!("{} requires an argument", &args[i]));
                }
                i += 2;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return 0;
            }
            "--version" => {
                println!("zd {}", env!("CARGO_PKG_VERSION"));
                return 0;
            }
            _ => break,
        }
    }
    if i >= args.len() {
        return usage_err("missing command");
    }

    let rest: Vec<String> = args[i + 1..].to_vec();
    let cmd = args[i].as_str();

    let result: Result<String, String> = match cmd {
        "health" => t.get("/health"),
        "ops" => t.get("/ops"),
        "metrics" => t.post("metrics", json!({})),
        "info" => t.post("info", json!({})),
        "ping" => {
            let body = if rest.is_empty() {
                json!({})
            } else {
                json!({ "echo": rest.join(" ") })
            };
            t.post("ping", body)
        }
        "call" => cmd_call(t, &rest),
        "cache" => cmd_cache(t, &rest),
        "job" => cmd_job(t, &rest),
        "lock" => cmd_lock(t, &rest),
        "publish" => cmd_publish(t, &rest),
        "events" => cmd_events(t, &rest),
        "watch" => cmd_watch(t, &rest),
        "defs" => cmd_defs(t, &rest),
        "snapshot" => cmd_snapshot(t, &rest),
        "artifact" => cmd_artifact(t, &rest),
        "schedule" => cmd_schedule(t, &rest),
        "export" => cmd_export(t, &rest),
        "view" => cmd_view(t, &rest),
        other => return usage_err(&format!("unknown command: {other}")),
    };

    match result {
        Ok(v) => {
            println!("{}", v);
            0
        }
        Err(e) => {
            eprintln!("zd: {e}");
            1
        }
    }
}

fn usage_err(msg: &str) -> i32 {
    eprintln!("zd: {msg}");
    eprintln!();
    eprintln!("{USAGE}");
    2
}

// ---- Subcommand handlers --------------------------------------------------

fn cmd_call(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    if rest.is_empty() {
        return Err("usage: zd call OP [JSON_BODY]".into());
    }
    let op = &rest[0];
    let body: Value = if rest.len() < 2 {
        json!({})
    } else {
        serde_json::from_str(&rest[1]).map_err(|e| format!("invalid JSON body: {e}"))?
    };
    t.post(op, body)
}

fn cmd_cache(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd cache <put|get|del|list|stats> ...")?;
    match sub.as_str() {
        "put" => {
            if rest.len() < 4 {
                return Err("usage: zd cache put NS KEY VALUE [--ttl SECS]".into());
            }
            let mut body = json!({"ns": rest[1], "key": rest[2], "value": rest[3]});
            if let Some(pos) = rest.iter().position(|a| a == "--ttl") {
                let secs = rest
                    .get(pos + 1)
                    .ok_or("--ttl requires SECS")?
                    .parse::<u64>()
                    .map_err(|e| format!("--ttl: {e}"))?;
                body["ttl_secs"] = json!(secs);
            }
            t.post("cache_put", body)
        }
        "get" => {
            if rest.len() < 3 {
                return Err("usage: zd cache get NS KEY".into());
            }
            t.post("cache_get", json!({"ns": rest[1], "key": rest[2]}))
        }
        "del" => {
            if rest.len() < 3 {
                return Err("usage: zd cache del NS KEY".into());
            }
            t.post("cache_del", json!({"ns": rest[1], "key": rest[2]}))
        }
        "list" => {
            if rest.len() < 2 {
                return Err("usage: zd cache list NS [PREFIX]".into());
            }
            let mut body = json!({"ns": rest[1]});
            if let Some(p) = rest.get(2) {
                body["prefix"] = json!(p);
            }
            t.post("cache_list", body)
        }
        "stats" => {
            let body = if let Some(ns) = rest.get(1) {
                json!({"ns": ns})
            } else {
                json!({})
            };
            t.post("cache_stats", body)
        }
        other => Err(format!("unknown cache subcommand: {other}")),
    }
}

fn parse_job_id(s: Option<&String>, sub: &str) -> Result<u64, String> {
    let s = s.ok_or_else(|| format!("usage: zd job {sub} JOB_ID"))?;
    s.parse::<u64>()
        .map_err(|e| format!("JOB_ID must be a number: {e}"))
}

fn cmd_job(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd job <submit|status|output|list|kill|wait> ...")?;
    match sub.as_str() {
        "submit" => {
            let cmd_start = if rest.get(1).map(String::as_str) == Some("--") {
                2
            } else {
                1
            };
            if rest.len() <= cmd_start {
                return Err("usage: zd job submit -- CMD [ARGS...]".into());
            }
            let command: Vec<&str> = rest[cmd_start..].iter().map(String::as_str).collect();
            t.post("job_submit", json!({"command": command}))
        }
        "status" => {
            let id = parse_job_id(rest.get(1), "status")?;
            t.post("job_status", json!({"id": id}))
        }
        "output" => {
            let id = parse_job_id(rest.get(1), "output")?;
            let stderr = rest.iter().any(|a| a == "--stderr");
            t.post("job_output", json!({"id": id, "stderr": stderr}))
        }
        "list" => {
            let mut body = json!({});
            if let Some(pos) = rest.iter().position(|a| a == "--state") {
                body["state"] = json!(rest.get(pos + 1).ok_or("--state requires VALUE")?);
            }
            if let Some(pos) = rest.iter().position(|a| a == "--tag") {
                body["tag"] = json!(rest.get(pos + 1).ok_or("--tag requires VALUE")?);
            }
            if let Some(pos) = rest.iter().position(|a| a == "--limit") {
                let n: u64 = rest
                    .get(pos + 1)
                    .ok_or("--limit requires N")?
                    .parse()
                    .map_err(|e| format!("--limit: {e}"))?;
                body["limit"] = json!(n);
            }
            t.post("job_list", body)
        }
        "kill" => {
            let id = parse_job_id(rest.get(1), "kill")?;
            t.post("job_kill", json!({"id": id}))
        }
        "wait" => {
            let id = parse_job_id(rest.get(1), "wait")?;
            t.post("job_wait", json!({"id": id}))
        }
        other => Err(format!("unknown job subcommand: {other}")),
    }
}

fn cmd_lock(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd lock <acquire|try|release|list> ...")?;
    let pid = std::process::id();
    match sub.as_str() {
        "acquire" => {
            let name = rest.get(1).ok_or("usage: zd lock acquire NAME [--timeout SECS]")?;
            let mut body = json!({"name": name, "pid": pid});
            if let Some(pos) = rest.iter().position(|a| a == "--timeout") {
                let secs: u64 = rest
                    .get(pos + 1)
                    .ok_or("--timeout requires SECS")?
                    .parse()
                    .map_err(|e| format!("--timeout: {e}"))?;
                body["timeout_secs"] = json!(secs);
            }
            t.post("lock_acquire", body)
        }
        "try" => {
            let name = rest.get(1).ok_or("usage: zd lock try NAME")?;
            t.post("lock_try_acquire", json!({"name": name, "pid": pid}))
        }
        "release" => {
            if rest.len() < 3 {
                return Err("usage: zd lock release NAME TOKEN".into());
            }
            t.post("lock_release", json!({"name": rest[1], "token": rest[2]}))
        }
        "list" => t.post("lock_list", json!({})),
        other => Err(format!("unknown lock subcommand: {other}")),
    }
}

fn cmd_publish(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    if rest.len() < 2 {
        return Err("usage: zd publish TOPIC JSON_DATA".into());
    }
    let data: Value =
        serde_json::from_str(&rest[1]).map_err(|e| format!("invalid JSON data: {e}"))?;
    t.post("publish", json!({"topic": rest[0], "data": data}))
}

fn cmd_events(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let pat = rest.first().cloned().unwrap_or_else(|| "*.*".to_string());
    t.sse(&format!("/stream/events?channel={pat}"))
}

fn cmd_watch(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let dir = rest.first().ok_or("usage: zd watch DIR [--recursive]")?;
    let recursive = rest.iter().any(|a| a == "--recursive");
    t.sse(&format!("/stream/watch?path={dir}&recursive={recursive}"))
}

fn cmd_defs(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd defs <query|kinds|emit|diff> ...")?;
    match sub.as_str() {
        "kinds" => t.post("definitions_kinds", json!({})),
        "query" => {
            let mut body = json!({});
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--kind" => {
                        body["kind"] = json!(rest.get(i + 1).ok_or("--kind requires VALUE")?);
                        i += 2;
                    }
                    "--name" => {
                        body["name"] = json!(rest.get(i + 1).ok_or("--name requires VALUE")?);
                        i += 2;
                    }
                    "--prefix" => {
                        body["prefix"] = json!(rest.get(i + 1).ok_or("--prefix requires VALUE")?);
                        i += 2;
                    }
                    "--shell-id" => {
                        body["shell_id"] =
                            json!(rest.get(i + 1).ok_or("--shell-id requires VALUE")?);
                        i += 2;
                    }
                    "--limit" => {
                        let n: u64 = rest
                            .get(i + 1)
                            .ok_or("--limit requires N")?
                            .parse()
                            .map_err(|e| format!("--limit: {e}"))?;
                        body["limit"] = json!(n);
                        i += 2;
                    }
                    other => return Err(format!("unknown defs query flag: {other}")),
                }
            }
            t.post("definitions_query", body)
        }
        "emit" => {
            let mut body = json!({});
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--shell-id" => {
                        body["shell_id"] =
                            json!(rest.get(i + 1).ok_or("--shell-id requires VALUE")?);
                        i += 2;
                    }
                    "--kind" => {
                        body["kind"] = json!(rest.get(i + 1).ok_or("--kind requires VALUE")?);
                        i += 2;
                    }
                    "--name" => {
                        body["name"] = json!(rest.get(i + 1).ok_or("--name requires VALUE")?);
                        i += 2;
                    }
                    "--value" => {
                        body["value"] = json!(rest.get(i + 1).ok_or("--value requires VALUE")?);
                        i += 2;
                    }
                    "--file" => {
                        body["file"] = json!(rest.get(i + 1).ok_or("--file requires VALUE")?);
                        i += 2;
                    }
                    "--line" => {
                        let n: u64 = rest
                            .get(i + 1)
                            .ok_or("--line requires N")?
                            .parse()
                            .map_err(|e| format!("--line: {e}"))?;
                        body["line"] = json!(n);
                        i += 2;
                    }
                    "--fn-chain" => {
                        body["fn_chain"] =
                            json!(rest.get(i + 1).ok_or("--fn-chain requires VALUE")?);
                        i += 2;
                    }
                    other => return Err(format!("unknown defs emit flag: {other}")),
                }
            }
            t.post("definitions_emit", body)
        }
        "diff" => {
            if rest.len() < 3 {
                return Err("usage: zd defs diff SHELL_A SHELL_B [KIND]".into());
            }
            let mut body = json!({"shell_a": rest[1], "shell_b": rest[2]});
            if let Some(k) = rest.get(3) {
                body["kind"] = json!(k);
            }
            t.post("definitions_diff", body)
        }
        other => Err(format!("unknown defs subcommand: {other}")),
    }
}

fn cmd_snapshot(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd snapshot <save|list|load|diff> ...")?;
    match sub.as_str() {
        "save" => {
            let tag = rest.get(1).ok_or("usage: zd snapshot save TAG [--notes N]")?;
            let mut body = json!({"tag": tag});
            if let Some(pos) = rest.iter().position(|a| a == "--notes") {
                body["notes"] = json!(rest.get(pos + 1).ok_or("--notes requires VALUE")?);
            }
            t.post("snapshot_save", body)
        }
        "list" => t.post("snapshot_list", json!({})),
        "load" => {
            let tag = rest.get(1).ok_or("usage: zd snapshot load TAG")?;
            t.post("snapshot_load", json!({"tag": tag}))
        }
        "diff" => {
            if rest.len() < 3 {
                return Err("usage: zd snapshot diff A B".into());
            }
            t.post("snapshot_diff", json!({"a": rest[1], "b": rest[2]}))
        }
        other => Err(format!("unknown snapshot subcommand: {other}")),
    }
}

fn cmd_artifact(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd artifact <put|get|list|gc> ...")?;
    match sub.as_str() {
        "put" => {
            let name = rest.get(1).ok_or("usage: zd artifact put NAME (VALUE|--file PATH)")?;
            if let Some(pos) = rest.iter().position(|a| a == "--file") {
                let path = rest.get(pos + 1).ok_or("--file requires PATH")?;
                let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
                let b64 = base64_encode(&bytes);
                t.post("artifact_put", json!({"name": name, "value_base64": b64}))
            } else {
                let value = rest.get(2).ok_or("missing VALUE (or pass --file PATH)")?;
                t.post("artifact_put", json!({"name": name, "value": value}))
            }
        }
        "get" => {
            let name = rest.get(1).ok_or("usage: zd artifact get NAME [-o OUT]")?;
            let resp = t.post("artifact_get", json!({"name": name}))?;
            if let Some(pos) = rest.iter().position(|a| a == "-o") {
                let out = rest.get(pos + 1).ok_or("-o requires PATH")?;
                let v: Value =
                    serde_json::from_str(&resp).map_err(|e| format!("decode response: {e}"))?;
                let b64 = v["value_base64"]
                    .as_str()
                    .ok_or("response missing value_base64")?;
                let bytes = base64_decode(b64)?;
                std::fs::write(out, &bytes).map_err(|e| format!("write {out}: {e}"))?;
                Ok(format!("{{\"wrote\":\"{out}\",\"bytes\":{}}}", bytes.len()))
            } else {
                Ok(resp)
            }
        }
        "list" => {
            let mut body = json!({});
            if let Some(p) = rest.get(1) {
                body["prefix"] = json!(p);
            }
            t.post("artifact_list", body)
        }
        "gc" => {
            let mut body = json!({});
            if let Some(pos) = rest.iter().position(|a| a == "--max-age") {
                let secs: u64 = rest
                    .get(pos + 1)
                    .ok_or("--max-age requires SECS")?
                    .parse()
                    .map_err(|e| format!("--max-age: {e}"))?;
                body["max_age_secs"] = json!(secs);
            }
            if let Some(pos) = rest.iter().position(|a| a == "--max-bytes") {
                let n: u64 = rest
                    .get(pos + 1)
                    .ok_or("--max-bytes requires N")?
                    .parse()
                    .map_err(|e| format!("--max-bytes: {e}"))?;
                body["max_bytes"] = json!(n);
            }
            t.post("artifact_gc", body)
        }
        other => Err(format!("unknown artifact subcommand: {other}")),
    }
}

fn cmd_schedule(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd schedule <add|add-once|list|remove> ...")?;
    match sub.as_str() {
        "add" => {
            if rest.len() < 4 {
                return Err("usage: zd schedule add CRON_EXPR -- CMD [ARGS...]".into());
            }
            let cron = &rest[1];
            let cmd_start = if rest.get(2).map(String::as_str) == Some("--") {
                3
            } else {
                2
            };
            let command: Vec<&str> = rest[cmd_start..].iter().map(String::as_str).collect();
            t.post(
                "schedule_add",
                json!({"cron_expr": cron, "command": command}),
            )
        }
        "add-once" => {
            if rest.len() < 4 {
                return Err("usage: zd schedule add-once UNIX_SECS -- CMD [ARGS...]".into());
            }
            let when: i64 = rest[1]
                .parse()
                .map_err(|e| format!("UNIX_SECS: {e}"))?;
            let cmd_start = if rest.get(2).map(String::as_str) == Some("--") {
                3
            } else {
                2
            };
            let command: Vec<&str> = rest[cmd_start..].iter().map(String::as_str).collect();
            t.post(
                "schedule_add_once",
                json!({"fire_at_unix_secs": when, "command": command}),
            )
        }
        "list" => t.post("schedule_list", json!({})),
        "remove" => {
            let id = rest.get(1).ok_or("usage: zd schedule remove ID")?;
            t.post("schedule_remove", json!({"id": id}))
        }
        other => Err(format!("unknown schedule subcommand: {other}")),
    }
}

fn cmd_export(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    if rest.len() < 2 {
        return Err("usage: zd export TARGET FORMAT".into());
    }
    let target = &rest[0];
    let format = &rest[1];
    let response = t.post("export", json!({"target": target, "format": format}))?;

    // For binary formats the daemon returns a JSON envelope with the
    // payload base64-encoded under `body_base64` (see
    // `daemon/export.rs::op_view_or_export`). The user almost always
    // wants the raw bytes — `zd export aliases pdf > out.pdf` should
    // produce a valid PDF file, not a JSON wrapper they have to
    // unpack with `jq -r .body_base64 | base64 -d`. Detect those
    // responses, decode, write raw to stdout, and exit immediately
    // so the dispatcher's trailing `println!` doesn't append a
    // newline that would corrupt the output.
    if let Ok(parsed) = serde_json::from_str::<Value>(&response) {
        if let Some(b64) = parsed.get("body_base64").and_then(Value::as_str) {
            let bytes = base64_decode(b64)
                .map_err(|e| format!("decode body_base64: {e}"))?;
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            out.write_all(&bytes)
                .map_err(|e| format!("write stdout: {e}"))?;
            out.flush().ok();
            std::process::exit(0);
        }
    }
    Ok(response)
}

fn cmd_view(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let target = rest.first().ok_or("usage: zd view TARGET [FORMAT]")?;
    let mut body = json!({"target": target});
    if let Some(fmt) = rest.get(1) {
        body["format"] = json!(fmt);
    }
    t.post("view", body)
}

// ---- base64 helpers (avoid pulling the `base64` crate into the
// daemon-server build; it's needed only by artifact put/get and the
// stdlib gives us enough). 6-bit lookup table.

const BASE64_ALPHABET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(BASE64_ALPHABET[(b0 >> 2) as usize] as char);
        out.push(BASE64_ALPHABET[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(BASE64_ALPHABET[(((b1 & 0b1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(BASE64_ALPHABET[(b2 & 0b111111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    let mut buf = [0u8; 4];
    let mut bi = 0;
    let mut padded = 0;
    for b in bytes {
        if b == b'=' {
            padded += 1;
            buf[bi] = 0;
            bi += 1;
        } else {
            let v = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => return Err(format!("invalid base64 byte: {b:#x}")),
            };
            buf[bi] = v;
            bi += 1;
        }
        if bi == 4 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            if padded < 2 {
                out.push((buf[1] << 4) | (buf[2] >> 2));
            }
            if padded < 1 {
                out.push((buf[2] << 6) | buf[3]);
            }
            bi = 0;
            padded = 0;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip_short() {
        for input in [&b""[..], b"a", b"ab", b"abc", b"abcd", b"hello world"] {
            let enc = base64_encode(input);
            let dec = base64_decode(&enc).unwrap();
            assert_eq!(dec.as_slice(), input);
        }
    }

    #[test]
    fn base64_round_trip_binary() {
        let input: Vec<u8> = (0u8..=255).collect();
        let enc = base64_encode(&input);
        let dec = base64_decode(&enc).unwrap();
        assert_eq!(dec, input);
    }
}
