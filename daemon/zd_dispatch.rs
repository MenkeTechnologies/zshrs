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

use std::borrow::Cow;
use std::io::IsTerminal;

use serde_json::{json, Value};

// House-style "full spectrum" help — ANSI-Shadow banner + status box +
// `──`-ruled sections with `//` annotations, matching the temprs look.
// This const is the plain text; `render_usage` layers on the temprs
// color palette when the destination is a real terminal (piped/redirected
// output stays plain, and `$NO_COLOR` disables it). Version is injected at
// compile time via `env!` so it never drifts from Cargo.toml. Box and
// divider lines are pinned to 57 visual columns; the `usage_house_help_*`
// tests guard the alignment.
pub const USAGE: &str = concat!(
    " ███████╗██████╗\n",
    " ╚══███╔╝██╔══██╗\n",
    "   ███╔╝ ██║  ██║\n",
    "  ███╔╝  ██║  ██║\n",
    " ███████╗██████╔╝\n",
    " ╚══════╝╚═════╝\n",
    " ┌──────────────────────────────────────────────────────┐\n",
    " │ STATUS: ONLINE   //   LINK: ████████░░               │\n",
    " │ SUBSTRATE: zshrs-daemon   //   TRANSPORT: unix+http  │\n",
    " └──────────────────────────────────────────────────────┘\n",
    "  >> ZSHRS-DAEMON CLIENT // v",
    env!("CARGO_PKG_VERSION"),
    " // FULL SPECTRUM <<\n",
    "\n",
    "  Builtin (in-process, no fork) from inside zshrs; standalone HTTP\n",
    "  binary otherwise. Same arg surface either way — only transport differs.\n",
    "\n",
    "  USAGE: zd [GLOBAL OPTS] <COMMAND> [ARGS]\n",
    "\n",
    "  ── GLOBAL ─────────────────────────────────────────────\n",
    "      --url <URL>       // override $DAEMON_URL (default http://127.0.0.1:7733) — binary only\n",
    "      --token <TOKEN>   // override $DAEMON_TOKEN — binary only\n",
    "  -h, --help            // this message\n",
    "      --version         // version + exit\n",
    "\n",
    "  ── CORE ───────────────────────────────────────────────\n",
    "    health                        // GET /health\n",
    "    ops                           // GET /ops\n",
    "    info                          // daemon snapshot\n",
    "    ping [ECHO_ARGS...]           // round-trip latency\n",
    "    metrics                       // Prometheus-shaped metrics (JSON)\n",
    "    call OP [JSON_BODY]           // generic op caller for anything not below\n",
    "\n",
    "  ── CACHE ──────────────────────────────────────────────\n",
    "    cache put NS KEY VALUE [--ttl SECS]\n",
    "    cache get NS KEY\n",
    "    cache del NS KEY\n",
    "    cache list NS [PREFIX]\n",
    "    cache stats [NS]\n",
    "\n",
    "  ── JOB ────────────────────────────────────────────────\n",
    "    job submit -- CMD [ARGS...]   // submit cmd; prints job_id\n",
    "    job status ID\n",
    "    job output ID [--stderr]\n",
    "    job list [--state S] [--tag T] [--limit N]\n",
    "    job kill ID\n",
    "    job wait ID                   // blocks until terminal\n",
    "\n",
    "  ── LOCK ───────────────────────────────────────────────\n",
    "    lock acquire NAME [--timeout SECS]\n",
    "    lock try NAME\n",
    "    lock release NAME TOKEN\n",
    "    lock list\n",
    "\n",
    "  ── EVENT / WATCH ──────────────────────────────────────\n",
    "    publish TOPIC JSON_DATA\n",
    "    events [PATTERN]              // streams SSE — binary only\n",
    "    watch DIR [--recursive]      // streams SSE — binary only\n",
    "\n",
    "  ── DEFINITIONS ────────────────────────────────────────\n",
    "    defs query [--kind K] [--name N] [--prefix P] [--shell-id S] [--limit N]\n",
    "    defs kinds\n",
    "    defs emit --shell-id S --kind K --name N [--value V] [--file F] [--line L]\n",
    "    defs diff SHELL_A SHELL_B [KIND]\n",
    "\n",
    "  ── SNAPSHOT ───────────────────────────────────────────\n",
    "    snapshot save TAG [--notes N] // freeze canonical state under TAG\n",
    "    snapshot list                 // enumerate saved tags\n",
    "    snapshot load TAG             // restore canonical state from TAG\n",
    "    snapshot diff A B             // subsystem-by-subsystem diff\n",
    "\n",
    "  ── ARTIFACT ───────────────────────────────────────────\n",
    "    artifact put NAME VALUE | artifact put NAME --file PATH\n",
    "    artifact get NAME [-o OUT]    // writes value to OUT (default stdout)\n",
    "    artifact list [PREFIX]\n",
    "    artifact gc [--max-age SECS] [--max-bytes N]\n",
    "\n",
    "  ── SCHEDULE ───────────────────────────────────────────\n",
    "    schedule add CRON_EXPR -- CMD [ARGS...]\n",
    "    schedule add-once UNIX_SECS -- CMD [ARGS...]\n",
    "    schedule list\n",
    "    schedule remove ID\n",
    "\n",
    "  ── EXPORT / VIEW ──────────────────────────────────────\n",
    "    export TARGET FORMAT          // sh|json|yaml|text|csv|sql|pdf|...\n",
    "    view TARGET [FORMAT]\n",
    "\n",
    "  ── CONFIG ─────────────────────────────────────────────\n",
    "    config get KEY                // read a runtime knob\n",
    "    config set KEY VALUE          // write/override a runtime knob\n",
    "    config list                   // show every runtime override\n",
    "\n",
    "  ── CACHES ─────────────────────────────────────────────\n",
    "    prewarm [DIR ...] [--json]    // compile every _* completer on\n",
    "                                  // $fpath (or the given dirs) into\n",
    "                                  // autoloads.rkyv, so the first\n",
    "                                  // `ls -<TAB>` is an O(1) shard\n",
    "                                  // probe, not a parse + compile\n",
    "\n",
    "  ── DIAGNOSTICS ────────────────────────────────────────\n",
    "    doctor [--json]               // health-report sweep (perms, db\n",
    "                                  // integrity, shards, fsnotify,\n",
    "                                  // pidlock, jobs, legacy litter)\n",
    "\n",
    "  ── SYSTEM ─────────────────────────────────────────────\n",
    "  v",
    env!("CARGO_PKG_VERSION"),
    "  //  zshrs-daemon client  //  transport: unix socket + http\n",
    "  Every op maps 1:1 to  POST /op/<NAME>  against the daemon.\n",
    "  >>> ONE SOCKET. EVERY SHELL. OWN YOUR DAEMON. <<<\n",
    " ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░\n",
);

// SGR codes for the colorized banner — same palette temprs uses.
const C_CYAN: &str = "\x1b[36m";
const C_MAGENTA: &str = "\x1b[35m";
const C_RED: &str = "\x1b[31m";
const C_YELLOW: &str = "\x1b[33m";
const C_GREEN: &str = "\x1b[32m";
const C_BOLD: &str = "\x1b[1m";
const C_RESET: &str = "\x1b[0m";

/// Color is on only for a real terminal with `$NO_COLOR` unset. `stdout`
/// is the stream `--help` prints to; usage errors pass `stderr`.
fn color_enabled(stream_is_tty: bool) -> bool {
    stream_is_tty && std::env::var_os("NO_COLOR").is_none()
}

/// Render the house help. When `color`, apply the temprs palette:
/// cyan→magenta→red gradient banner, cyan status box + `──` dividers,
/// magenta `>>`/`<<` taglines, yellow `USAGE:`, bold leading token,
/// green ` // ` annotations. Otherwise return `USAGE` verbatim.
///
/// Whole-line-colored classes (banner, box, divider, tagline, shade)
/// skip token coloring so their embedded `//` stays the line color and
/// the `http://` URL is never mistaken for an annotation.
fn render_usage(color: bool) -> Cow<'static, str> {
    if !color {
        return Cow::Borrowed(USAGE);
    }
    let mut out = String::with_capacity(USAGE.len() * 2);
    for (idx, line) in USAGE.lines().enumerate() {
        let trimmed = line.trim_start();
        let whole = match idx {
            0 | 1 => Some(C_CYAN),
            2 | 3 => Some(C_MAGENTA),
            4 | 5 => Some(C_RED),
            _ if line.starts_with(" ┌") || line.starts_with(" │") || line.starts_with(" └") => {
                Some(C_CYAN)
            }
            _ if trimmed.starts_with("── ") => Some(C_CYAN),
            _ if line.starts_with(" ░") => Some(C_CYAN),
            _ if trimmed.starts_with(">>") => Some(C_MAGENTA),
            _ => None,
        };
        match whole {
            Some(c) => {
                out.push_str(c);
                out.push_str(line);
                out.push_str(C_RESET);
            }
            None if trimmed.starts_with("USAGE:") => {
                // `  USAGE: zd [GLOBAL OPTS] ...`
                let indent = &line[..line.len() - trimmed.len()];
                let after = trimmed.strip_prefix("USAGE:").unwrap_or(trimmed);
                out.push_str(indent);
                out.push_str(C_YELLOW);
                out.push_str("USAGE:");
                out.push_str(C_RESET);
                out.push_str(after);
            }
            None => out.push_str(&colorize_body_line(line)),
        }
        out.push('\n');
    }
    Cow::Owned(out)
}

/// Body-row coloring: bold the leading token on command/flag rows and
/// paint every ` // ` marker green. The leading token is bolded only for
/// flag rows (trimmed starts with `-`) or command rows (indented ≥ 4) —
/// prose (2-space indent, e.g. the description and SYSTEM lines) keeps
/// its plain weight. Blank and `//`-continuation lines get no bold.
/// Greens ` // ` (space-slash-slash-space) only, so the `http://` in the
/// --url row is left untouched.
fn colorize_body_line(line: &str) -> String {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return line.to_string();
    }
    let indent_len = line.len() - trimmed.len();
    let mut s = String::with_capacity(line.len() + 16);
    s.push_str(&line[..indent_len]);

    let bold_leading = !trimmed.starts_with("//") && (trimmed.starts_with('-') || indent_len >= 4);
    let rest = match trimmed.find(' ') {
        Some(tok_end) if bold_leading => {
            let (tok, tail) = trimmed.split_at(tok_end);
            format!("{C_BOLD}{tok}{C_RESET}{tail}")
        }
        None if bold_leading => format!("{C_BOLD}{trimmed}{C_RESET}"),
        _ => trimmed.to_string(),
    };
    s.push_str(&rest.replace(" // ", &format!(" {C_GREEN}//{C_RESET} ")));
    s
}

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

/// Answer the arg shapes that need no daemon transport: empty args
/// (usage error → 2), `-h`/`--help` (print USAGE → 0), `--version`
/// (print version → 0), a `--url`/`--token` missing its value, and
/// global-flags-with-no-command. Returns `Some(exit_code)` when handled,
/// `None` when the args carry a real command that must go through a
/// transport.
///
/// Shared by the standalone binary (via `dispatch`) and the in-process
/// builtin, which calls it *before* its daemon-alive pre-flight so help
/// and version never require a running daemon.
pub fn handle_no_transport(args: &[String]) -> Option<i32> {
    if args.is_empty() {
        eprint!(
            "{}",
            render_usage(color_enabled(std::io::stderr().is_terminal()))
        );
        return Some(2);
    }
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" | "--token" => {
                if i + 1 >= args.len() {
                    return Some(usage_err(&format!("{} requires an argument", args[i])));
                }
                i += 2;
            }
            "-h" | "--help" => {
                print!(
                    "{}",
                    render_usage(color_enabled(std::io::stdout().is_terminal()))
                );
                return Some(0);
            }
            "--version" => {
                println!("zd {}", env!("CARGO_PKG_VERSION"));
                return Some(0);
            }
            // A real command — needs a transport, so hand back to dispatch.
            _ => return None,
        }
    }
    // Consumed only global flags, never reached a command.
    Some(usage_err("missing command"))
}

/// Top-level dispatcher. Parses global flags, routes to the right
/// `cmd_*` handler, prints the result. Returns process-exit-style
/// status: 0 = success, 1 = transport/protocol error, 2 = usage error.
pub fn dispatch(args: &[String], t: &mut dyn Transport) -> i32 {
    // Everything that never touches the daemon (empty args, -h/--help,
    // --version, and the pure-usage errors) is answered here first. The
    // in-process builtin calls the same helper *before* its daemon
    // pre-flight so `zd --help` / `zd --version` behave identically to
    // the binary even when the daemon is down.
    if let Some(code) = handle_no_transport(args) {
        return code;
    }

    // Past the short-circuit there is a real command. Re-skip the global
    // flags (transport already owns --url / --token by value) to find it.
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" | "--token" => i += 2,
            _ => break,
        }
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
        "config" => cmd_config(t, &rest),
        "artifact" => cmd_artifact(t, &rest),
        "schedule" => cmd_schedule(t, &rest),
        "export" => cmd_export(t, &rest),
        "view" => cmd_view(t, &rest),
        "doctor" => cmd_doctor(t, &rest),
        "prewarm" => cmd_prewarm(t, &rest),
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
    eprint!(
        "{}",
        render_usage(color_enabled(std::io::stderr().is_terminal()))
    );
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
    let sub = rest
        .first()
        .ok_or("usage: zd cache <put|get|del|list|stats> ...")?;
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
    let sub = rest
        .first()
        .ok_or("usage: zd job <submit|status|output|list|kill|wait> ...")?;
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
    let sub = rest
        .first()
        .ok_or("usage: zd lock <acquire|try|release|list> ...")?;
    let pid = std::process::id();
    match sub.as_str() {
        "acquire" => {
            let name = rest
                .get(1)
                .ok_or("usage: zd lock acquire NAME [--timeout SECS]")?;
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
    let sub = rest
        .first()
        .ok_or("usage: zd defs <query|kinds|emit|diff> ...")?;
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
    let sub = rest
        .first()
        .ok_or("usage: zd snapshot <save|list|load|diff> ...")?;
    match sub.as_str() {
        "save" => {
            let tag = rest
                .get(1)
                .ok_or("usage: zd snapshot save TAG [--notes N]")?;
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

/// `zd config <get|set|list>` — runtime knob plumbing. Maps 1:1 to
/// the `config_get`, `config_set`, `config_list` daemon ops. Sets are
/// in-memory only — writing to `zshrs-daemon.toml` is a separate
/// (unwired) concern; today the toml seeds the initial values and
/// `config_set` overrides them for the daemon's lifetime.
fn cmd_config(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest.first().ok_or("usage: zd config <get|set|list> ...")?;
    match sub.as_str() {
        "get" => {
            let key = rest.get(1).ok_or("usage: zd config get KEY")?;
            t.post("config_get", json!({"key": key}))
        }
        "set" => {
            if rest.len() < 3 {
                return Err("usage: zd config set KEY VALUE".into());
            }
            t.post("config_set", json!({"key": rest[1], "value": rest[2]}))
        }
        "list" => t.post("config_list", json!({})),
        other => Err(format!("unknown config subcommand: {other}")),
    }
}

fn cmd_artifact(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let sub = rest
        .first()
        .ok_or("usage: zd artifact <put|get|list|gc> ...")?;
    match sub.as_str() {
        "put" => {
            let name = rest
                .get(1)
                .ok_or("usage: zd artifact put NAME (VALUE|--file PATH)")?;
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
    let sub = rest
        .first()
        .ok_or("usage: zd schedule <add|add-once|list|remove> ...")?;
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
            let when: i64 = rest[1].parse().map_err(|e| format!("UNIX_SECS: {e}"))?;
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
        return Err("usage: zd export TARGET FORMAT [--json]".into());
    }
    let target = &rest[0];
    let format = &rest[1];
    // `--json` opt-in restores the JSON envelope; default unwraps
    // straight to the requested payload so:
    //   zd export aliases sh   > aliases.sh
    //   zd export aliases csv  > aliases.csv
    //   zd export aliases pdf  > aliases.pdf
    // all produce the right file directly without `jq -r` gymnastics.
    let envelope = rest.iter().any(|a| a == "--json" || a == "--envelope");
    let response = t.post("export", json!({"target": target, "format": format}))?;

    if envelope {
        return Ok(response);
    }

    // Daemon's export response shape (see
    // `daemon/export.rs::op_view_or_export`):
    //   text formats  → { "body": "..." }
    //   binary (pdf)  → { "body_base64": "..." }
    // Extract the payload and stream it to stdout. For binary we
    // exit early to avoid the dispatcher's trailing newline; text
    // returns through the normal path which adds the newline (matches
    // every shell tool — `echo`, `cat`, etc.).
    let parsed: Value = serde_json::from_str(&response)
        .map_err(|e| format!("export response: {e}\nbody: {response}"))?;
    if let Some(b64) = parsed.get("body_base64").and_then(Value::as_str) {
        let bytes = base64_decode(b64).map_err(|e| format!("decode body_base64: {e}"))?;
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        out.write_all(&bytes)
            .map_err(|e| format!("write stdout: {e}"))?;
        out.flush().ok();
        std::process::exit(0);
    }
    if let Some(body) = parsed.get("body").and_then(Value::as_str) {
        // Strip a single trailing newline so we don't double up with
        // dispatch's `println!`. Most exporters end the body with
        // `\n` already; a second one creates a blank trailing line
        // in redirected files.
        let trimmed = body.strip_suffix('\n').unwrap_or(body);
        return Ok(trimmed.to_string());
    }
    // Daemon returned an envelope shape we don't know — surface the
    // raw response rather than swallowing it.
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

/// `zd doctor` — pretty-print the `doctor` op's check sweep as a
/// human-readable table. `--json` opt-in returns the raw envelope
/// (`{checks, total, passed, failed, ts_ns}`) for scripting.
///
/// Format: one line per check, `OK` or `FAIL` prefix, name padded
/// to the widest in the response, then the detail string. Trailing
/// summary line carries the pass/fail counts. Bare ASCII so
/// non-TTY pipes (CI, jq, grep) don't see escape sequences.
/// `zd prewarm [DIR ...] [--json]` — ask the daemon to compile the
/// completer corpus into `~/.zshrs/autoloads.rkyv`.
///
/// With no directories the spawned shell uses its own `$fpath`. Give
/// directories to prewarm just the plugin you installed rather than
/// re-walking 46k files.
fn cmd_prewarm(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let envelope = rest.iter().any(|a| a == "--json" || a == "--envelope");
    let dirs: Vec<&String> = rest.iter().filter(|a| !a.starts_with("--")).collect();
    let response = t.post("autoload_prewarm", json!({ "dirs": dirs }))?;
    if envelope {
        return Ok(response);
    }
    let parsed: Value = serde_json::from_str(&response)
        .map_err(|e| format!("prewarm response: {e}\nbody: {response}"))?;
    let s = parsed.get("summary").unwrap_or(&Value::Null);
    let n = |k: &str| s.get(k).and_then(Value::as_u64).unwrap_or(0);
    let bytes = n("bytes");
    Ok(format!(
        "autoload bytecode — {} compiled, {} already fresh, {} unparseable, {:.1} MB in {} ms\n",
        n("compiled"),
        n("fresh"),
        n("failed"),
        bytes as f64 / (1024.0 * 1024.0),
        n("ms"),
    ))
}

fn cmd_doctor(t: &mut dyn Transport, rest: &[String]) -> Result<String, String> {
    let envelope = rest.iter().any(|a| a == "--json" || a == "--envelope");
    let response = t.post("doctor", json!({}))?;
    if envelope {
        return Ok(response);
    }

    let parsed: Value = serde_json::from_str(&response)
        .map_err(|e| format!("doctor response: {e}\nbody: {response}"))?;
    let checks = parsed
        .get("checks")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("doctor response missing `checks` array: {response}"))?;

    // Column widths: status fixed at 4 chars (OK / FAIL), name padded
    // to max-of-checks for vertical alignment, detail flushed left.
    let max_name = checks
        .iter()
        .filter_map(|c| c.get("name").and_then(Value::as_str))
        .map(str::len)
        .max()
        .unwrap_or(0);

    let total = parsed.get("total").and_then(Value::as_u64).unwrap_or(0);
    let passed = parsed.get("passed").and_then(Value::as_u64).unwrap_or(0);
    let failed = parsed.get("failed").and_then(Value::as_u64).unwrap_or(0);

    let mut out = String::new();
    out.push_str(&format!(
        "zshrs-daemon doctor — {passed} passed, {failed} failed (of {total})\n"
    ));
    out.push_str(&format!("{}\n", "-".repeat(max_name + 16)));
    for c in checks {
        let name = c.get("name").and_then(Value::as_str).unwrap_or("?");
        let ok = c.get("ok").and_then(Value::as_bool).unwrap_or(false);
        let detail = c.get("detail").and_then(Value::as_str).unwrap_or("");
        let status = if ok { "OK  " } else { "FAIL" };
        out.push_str(&format!(
            "{status}  {name:<width$}  {detail}\n",
            width = max_name
        ));
    }
    if failed == 0 {
        out.push_str("\nall checks passed");
    } else {
        out.push_str(&format!(
            "\n{failed} check{} failed — see above",
            if failed == 1 { "" } else { "s" }
        ));
    }
    // Print + exit directly so the exit code reflects health (non-zero
    // on failures — CI / monitoring scripts depend on this). Bypasses
    // the dispatcher's Ok-prints-with-newline path.
    use std::io::Write;
    let mut w = std::io::stdout().lock();
    let _ = writeln!(w, "{}", out);
    let _ = w.flush();
    std::process::exit(if failed == 0 { 0 } else { 1 });
}

// ---- base64 helpers (avoid pulling the `base64` crate into the
// daemon-server build; it's needed only by artifact put/get and the
// stdlib gives us enough). 6-bit lookup table.

const BASE64_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
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
    let bytes: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
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

    /// The house-style banner box and every `── SECTION ──` divider must
    /// be pinned to the same visual width (57 columns) or the layout
    /// looks broken. Width is measured in `char`s, not bytes — box-drawing
    /// glyphs are multibyte. Guards against a future section name change
    /// that forgets to re-fill the trailing dashes.
    #[test]
    fn usage_house_help_lines_are_57_columns() {
        const W: usize = 57;
        for line in USAGE.lines() {
            let is_box = line.starts_with(" ┌") || line.starts_with(" └");
            let is_bar = line.starts_with(" │");
            let is_rule = line.trim_start().starts_with("── ");
            let is_shade = line.starts_with(" ░");
            if is_box || is_bar || is_rule || is_shade {
                assert_eq!(
                    line.chars().count(),
                    W,
                    "misaligned house-help line (expected {W} cols): {line:?}"
                );
            }
        }
    }

    /// Plain render is the const verbatim; colored render must be pure
    /// decoration — stripping every SGR sequence has to reproduce the
    /// plain help byte-for-byte, so color never drops or reflows content.
    #[test]
    fn render_usage_color_is_lossless() {
        assert_eq!(render_usage(false), USAGE);
        let colored = render_usage(true);
        assert!(colored.contains('\x1b'), "colored render has no SGR codes");
        let mut stripped = String::with_capacity(colored.len());
        let mut chars = colored.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // consume CSI `[ ... m`
                for e in chars.by_ref() {
                    if e == 'm' {
                        break;
                    }
                }
            } else {
                stripped.push(c);
            }
        }
        assert_eq!(stripped, USAGE);
    }

    /// The compile-time version must land in both the banner tagline and
    /// the SYSTEM footer — proves the `env!` injection wired through.
    #[test]
    fn usage_house_help_embeds_version() {
        let ver = env!("CARGO_PKG_VERSION");
        assert!(USAGE.contains(&format!("CLIENT // v{ver} // FULL SPECTRUM")));
        assert_eq!(USAGE.matches(&format!("v{ver}")).count(), 2);
    }
}
