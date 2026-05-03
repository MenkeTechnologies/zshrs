#![cfg(feature = "zd")]

//! `zd` — thin HTTP client for `zshrs-daemon`. Standalone binary;
//! does not link against the shell. Shells that don't ship a recorder
//! / IPC client (bash, fish, dash, ksh, nu, elvish, pwsh, …) call
//! `zd` to drive the daemon's full op surface.
//!
//! See docs/DAEMON_AS_SERVICE.md for the canonical op contract. Every
//! `zd <SUBCMD>` maps 1:1 to a `POST /op/<NAME>` against the daemon's
//! HTTP listener (or a `GET` for `/health` / `/ops` / `/metrics`).
//!
//! Naming: per the same rule as `zshrs-recorder` (separate binary,
//! never `zshrs --recorder`), `zd` is a separate binary, never
//! `zshrs --client`. The shell stays the shell; the daemon client
//! stays the daemon client.
//!
//! Defaults:
//!   - URL:   `$DAEMON_URL`   or `http://127.0.0.1:7733`
//!   - Token: `$DAEMON_TOKEN` (empty = no auth header sent)
//!
//! Output: raw JSON to stdout (pipe through `jq` for pretty). Errors
//! to stderr; exit code 0 on success, 1 on transport / protocol error,
//! 2 on usage error.

use std::io::{Read, Write};
use std::process::{Command, ExitCode};

use serde_json::{json, Value};

const USAGE: &str = "\
zd — HTTP client for zshrs-daemon. Single binary, no shell required.

USAGE
    zd [GLOBAL OPTS] <COMMAND> [ARGS]

GLOBAL OPTS
    --url URL          override $DAEMON_URL (default http://127.0.0.1:7733)
    --token TOKEN      override $DAEMON_TOKEN
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
    events [PATTERN]               streams SSE via curl (default: *.*)
    watch DIR [--recursive]        streams SSE via curl

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

ENVIRONMENT
    DAEMON_URL    base URL of the daemon's HTTP listener
    DAEMON_TOKEN  bearer token sent in the Authorization header

EXAMPLES
    zd cache put build-config \"$(cat config.json)\"
    zd job submit -- find / -name '*.zsh'
    zd defs query --kind alias --shell-id bash
    zd defs diff bash zshrs alias
    zd snapshot save baseline
    DAEMON_URL=http://srv.lan:7733 DAEMON_TOKEN=xyz zd info
";

#[derive(Default)]
struct Globals {
    url: String,
    token: String,
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }

    // Strip global flags. Anything starting with `-` before the first
    // bare word is a global flag.
    let mut g = Globals {
        url: std::env::var("DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:7733".to_string()),
        token: std::env::var("DAEMON_TOKEN").unwrap_or_default(),
    };
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--url" => {
                if i + 1 >= argv.len() {
                    return usage_err("--url requires an argument");
                }
                g.url = argv[i + 1].clone();
                i += 2;
            }
            "--token" => {
                if i + 1 >= argv.len() {
                    return usage_err("--token requires an argument");
                }
                g.token = argv[i + 1].clone();
                i += 2;
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" => {
                println!("zd {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => break,
        }
    }
    if i >= argv.len() {
        return usage_err("missing command");
    }

    let rest: Vec<String> = argv[i + 1..].to_vec();
    let cmd = argv[i].as_str();

    let result = match cmd {
        // Health / introspection.
        "health" => http_get(&g, "/health"),
        "ops" => http_get(&g, "/ops"),
        "metrics" => post(&g, "metrics", json!({})),
        "info" => post(&g, "info", json!({})),
        "ping" => {
            let body = if rest.is_empty() {
                json!({})
            } else {
                json!({ "echo": rest.join(" ") })
            };
            post(&g, "ping", body)
        }
        "call" => cmd_call(&g, &rest),

        // Cache.
        "cache" => cmd_cache(&g, &rest),

        // Job.
        "job" => cmd_job(&g, &rest),

        // Lock.
        "lock" => cmd_lock(&g, &rest),

        // Event / watch / publish.
        "publish" => cmd_publish(&g, &rest),
        "events" => cmd_events(&g, &rest),
        "watch" => cmd_watch(&g, &rest),

        // Definitions.
        "defs" => cmd_defs(&g, &rest),

        // Snapshot.
        "snapshot" => cmd_snapshot(&g, &rest),

        // Artifact.
        "artifact" => cmd_artifact(&g, &rest),

        // Schedule.
        "schedule" => cmd_schedule(&g, &rest),

        // Export.
        "export" => cmd_export(&g, &rest),
        "view" => cmd_view(&g, &rest),

        other => return usage_err(&format!("unknown command: {other}")),
    };

    match result {
        Ok(v) => {
            println!("{}", v);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("zd: {e}");
            ExitCode::from(1)
        }
    }
}

// ---- Subcommand handlers --------------------------------------------------

fn cmd_call(g: &Globals, rest: &[String]) -> Result<String, String> {
    if rest.is_empty() {
        return Err("usage: zd call OP [JSON_BODY]".into());
    }
    let op = &rest[0];
    let body: Value = if rest.len() < 2 {
        json!({})
    } else {
        serde_json::from_str(&rest[1]).map_err(|e| format!("invalid JSON body: {e}"))?
    };
    post(g, op, body)
}

fn cmd_cache(g: &Globals, rest: &[String]) -> Result<String, String> {
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
            post(g, "cache_put", body)
        }
        "get" => {
            if rest.len() < 3 {
                return Err("usage: zd cache get NS KEY".into());
            }
            post(g, "cache_get", json!({"ns": rest[1], "key": rest[2]}))
        }
        "del" => {
            if rest.len() < 3 {
                return Err("usage: zd cache del NS KEY".into());
            }
            post(g, "cache_del", json!({"ns": rest[1], "key": rest[2]}))
        }
        "list" => {
            if rest.len() < 2 {
                return Err("usage: zd cache list NS [PREFIX]".into());
            }
            let mut body = json!({"ns": rest[1]});
            if let Some(p) = rest.get(2) {
                body["prefix"] = json!(p);
            }
            post(g, "cache_list", body)
        }
        "stats" => {
            let body = if let Some(ns) = rest.get(1) {
                json!({"ns": ns})
            } else {
                json!({})
            };
            post(g, "cache_stats", body)
        }
        other => Err(format!("unknown cache subcommand: {other}")),
    }
}

fn cmd_job(g: &Globals, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd job <submit|status|output|list|kill|wait> ...")?;
    match sub.as_str() {
        "submit" => {
            // After `submit`, accept an optional `--` separator then
            // the command + args.
            let cmd_start = if rest.get(1).map(String::as_str) == Some("--") {
                2
            } else {
                1
            };
            if rest.len() <= cmd_start {
                return Err("usage: zd job submit -- CMD [ARGS...]".into());
            }
            let command: Vec<&str> = rest[cmd_start..].iter().map(String::as_str).collect();
            post(g, "job_submit", json!({"command": command}))
        }
        "status" => {
            let id = parse_job_id(rest.get(1), "status")?;
            post(g, "job_status", json!({"id": id}))
        }
        "output" => {
            let id = parse_job_id(rest.get(1), "output")?;
            let stderr = rest.iter().any(|a| a == "--stderr");
            post(g, "job_output", json!({"id": id, "stderr": stderr}))
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
            post(g, "job_list", body)
        }
        "kill" => {
            let id = parse_job_id(rest.get(1), "kill")?;
            post(g, "job_kill", json!({"id": id}))
        }
        "wait" => {
            let id = parse_job_id(rest.get(1), "wait")?;
            post(g, "job_wait", json!({"id": id}))
        }
        other => Err(format!("unknown job subcommand: {other}")),
    }
}

fn cmd_lock(g: &Globals, rest: &[String]) -> Result<String, String> {
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
            post(g, "lock_acquire", body)
        }
        "try" => {
            let name = rest.get(1).ok_or("usage: zd lock try NAME")?;
            post(g, "lock_try_acquire", json!({"name": name, "pid": pid}))
        }
        "release" => {
            if rest.len() < 3 {
                return Err("usage: zd lock release NAME TOKEN".into());
            }
            post(g, "lock_release", json!({"name": rest[1], "token": rest[2]}))
        }
        "list" => post(g, "lock_list", json!({})),
        other => Err(format!("unknown lock subcommand: {other}")),
    }
}

fn cmd_publish(g: &Globals, rest: &[String]) -> Result<String, String> {
    if rest.len() < 2 {
        return Err("usage: zd publish TOPIC JSON_DATA".into());
    }
    let data: Value =
        serde_json::from_str(&rest[1]).map_err(|e| format!("invalid JSON data: {e}"))?;
    post(g, "publish", json!({"topic": rest[0], "data": data}))
}

fn cmd_events(g: &Globals, rest: &[String]) -> Result<String, String> {
    let pat = rest.first().cloned().unwrap_or_else(|| "*.*".to_string());
    sse_via_curl(g, &format!("/stream/events?channel={pat}"))
}

fn cmd_watch(g: &Globals, rest: &[String]) -> Result<String, String> {
    let dir = rest.first().ok_or("usage: zd watch DIR [--recursive]")?;
    let recursive = rest.iter().any(|a| a == "--recursive");
    sse_via_curl(g, &format!("/stream/watch?path={dir}&recursive={recursive}"))
}

fn cmd_defs(g: &Globals, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd defs <query|kinds|emit|diff> ...")?;
    match sub.as_str() {
        "kinds" => post(g, "definitions_kinds", json!({})),
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
            post(g, "definitions_query", body)
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
            post(g, "definitions_emit", body)
        }
        "diff" => {
            if rest.len() < 3 {
                return Err("usage: zd defs diff SHELL_A SHELL_B [KIND]".into());
            }
            let mut body = json!({"shell_a": rest[1], "shell_b": rest[2]});
            if let Some(k) = rest.get(3) {
                body["kind"] = json!(k);
            }
            post(g, "definitions_diff", body)
        }
        other => Err(format!("unknown defs subcommand: {other}")),
    }
}

fn cmd_snapshot(g: &Globals, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd snapshot <save|list|load|diff> ...")?;
    match sub.as_str() {
        "save" => {
            let tag = rest.get(1).ok_or("usage: zd snapshot save TAG [--notes N]")?;
            let mut body = json!({"tag": tag});
            if let Some(pos) = rest.iter().position(|a| a == "--notes") {
                body["notes"] = json!(rest.get(pos + 1).ok_or("--notes requires VALUE")?);
            }
            post(g, "snapshot_save", body)
        }
        "list" => post(g, "snapshot_list", json!({})),
        "load" => {
            let tag = rest.get(1).ok_or("usage: zd snapshot load TAG")?;
            post(g, "snapshot_load", json!({"tag": tag}))
        }
        "diff" => {
            if rest.len() < 3 {
                return Err("usage: zd snapshot diff A B".into());
            }
            post(g, "snapshot_diff", json!({"a": rest[1], "b": rest[2]}))
        }
        other => Err(format!("unknown snapshot subcommand: {other}")),
    }
}

fn cmd_artifact(g: &Globals, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd artifact <put|get|list|gc> ...")?;
    match sub.as_str() {
        "put" => {
            // Two forms:
            //   zd artifact put NAME VALUE
            //   zd artifact put NAME --file PATH
            let name = rest.get(1).ok_or("usage: zd artifact put NAME (VALUE|--file PATH)")?;
            if let Some(pos) = rest.iter().position(|a| a == "--file") {
                let path = rest.get(pos + 1).ok_or("--file requires PATH")?;
                let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
                use base64::Engine;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                post(g, "artifact_put", json!({"name": name, "value_base64": b64}))
            } else {
                let value = rest.get(2).ok_or("missing VALUE (or pass --file PATH)")?;
                post(g, "artifact_put", json!({"name": name, "value": value}))
            }
        }
        "get" => {
            let name = rest.get(1).ok_or("usage: zd artifact get NAME [-o OUT]")?;
            let resp = post(g, "artifact_get", json!({"name": name}))?;
            // If `-o OUT`, decode value_base64 into the file and print
            // a one-line summary. Otherwise dump the raw response.
            if let Some(pos) = rest.iter().position(|a| a == "-o") {
                let out = rest.get(pos + 1).ok_or("-o requires PATH")?;
                let v: Value =
                    serde_json::from_str(&resp).map_err(|e| format!("decode response: {e}"))?;
                let b64 = v["value_base64"]
                    .as_str()
                    .ok_or("response missing value_base64")?;
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| format!("base64 decode: {e}"))?;
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
            post(g, "artifact_list", body)
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
            post(g, "artifact_gc", body)
        }
        other => Err(format!("unknown artifact subcommand: {other}")),
    }
}

fn cmd_schedule(g: &Globals, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd schedule <add|add-once|list|remove> ...")?;
    match sub.as_str() {
        "add" => {
            // zd schedule add CRON_EXPR -- CMD [ARGS...]
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
            post(
                g,
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
            post(
                g,
                "schedule_add_once",
                json!({"fire_at_unix_secs": when, "command": command}),
            )
        }
        "list" => post(g, "schedule_list", json!({})),
        "remove" => {
            let id = rest.get(1).ok_or("usage: zd schedule remove ID")?;
            post(g, "schedule_remove", json!({"id": id}))
        }
        other => Err(format!("unknown schedule subcommand: {other}")),
    }
}

fn cmd_export(g: &Globals, rest: &[String]) -> Result<String, String> {
    if rest.len() < 2 {
        return Err("usage: zd export TARGET FORMAT".into());
    }
    post(g, "export", json!({"target": rest[0], "format": rest[1]}))
}

fn cmd_view(g: &Globals, rest: &[String]) -> Result<String, String> {
    let target = rest.first().ok_or("usage: zd view TARGET [FORMAT]")?;
    let mut body = json!({"target": target});
    if let Some(fmt) = rest.get(1) {
        body["format"] = json!(fmt);
    }
    post(g, "view", body)
}

// Job IDs are u64 server-side (daemon/ops.rs:3810). Convert here so
// `zd job status JOB_ID` accepts the user's CLI string and the wire
// payload is correctly typed.
fn parse_job_id(s: Option<&String>, sub: &str) -> Result<u64, String> {
    let s = s.ok_or_else(|| format!("usage: zd job {sub} JOB_ID"))?;
    s.parse::<u64>()
        .map_err(|e| format!("JOB_ID must be a number: {e}"))
}

// ---- HTTP helpers ---------------------------------------------------------

fn post(g: &Globals, op: &str, body: Value) -> Result<String, String> {
    let url = format!("{}/op/{}", g.url, op);
    let req = ureq::post(&url).set("Content-Type", "application/json");
    let req = if g.token.is_empty() {
        req
    } else {
        req.set("Authorization", &format!("Bearer {}", g.token))
    };
    match req.send_string(&body.to_string()) {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("read response body: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {body}"))
        }
        Err(ureq::Error::Transport(t)) => Err(format!("transport: {t}")),
    }
}

fn http_get(g: &Globals, path: &str) -> Result<String, String> {
    let url = format!("{}{}", g.url, path);
    let req = ureq::get(&url);
    let req = if g.token.is_empty() {
        req
    } else {
        req.set("Authorization", &format!("Bearer {}", g.token))
    };
    match req.call() {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("read response body: {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("HTTP {code}: {body}"))
        }
        Err(ureq::Error::Transport(t)) => Err(format!("transport: {t}")),
    }
}

/// SSE streams (`/stream/watch`, `/stream/events`) are long-lived
/// chunked reads. ureq's blocking client works for them but the
/// ergonomics of pumping bytes to stdout indefinitely + the fact
/// that every shell user already has `curl` makes shelling out the
/// cleanest path. Same approach as `daemon-shell.zsh` /
/// `daemon-shell.fish`. Returns Ok(empty) on Ctrl-C / TCP close.
fn sse_via_curl(g: &Globals, path: &str) -> Result<String, String> {
    let url = format!("{}{}", g.url, path);
    let mut cmd = Command::new("curl");
    cmd.arg("-sN");
    if !g.token.is_empty() {
        cmd.arg("-H").arg(format!("Authorization: Bearer {}", g.token));
    }
    cmd.arg(&url);
    let mut child = cmd.spawn().map_err(|e| format!("spawn curl: {e}"))?;
    let status = child.wait().map_err(|e| format!("wait curl: {e}"))?;
    if !status.success() && status.code() != Some(0) {
        return Err(format!("curl exited {status}"));
    }
    Ok(String::new())
}

// ---- Misc -----------------------------------------------------------------

fn usage_err(msg: &str) -> ExitCode {
    eprintln!("zd: {msg}");
    eprintln!();
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

// Suppress unused-import warning when no SSE features compile in.
#[allow(dead_code)]
fn _silence_unused_warnings(_r: &dyn Read, _w: &dyn Write) {}
