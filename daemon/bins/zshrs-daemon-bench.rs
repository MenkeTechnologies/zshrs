// zshrs-daemon-bench — RTT bench for the IPC fast path.
//
// Per docs/DAEMON.md "Acceptance criteria":
//   - Cold client launch (daemon already running): <5 ms
//   - Tab completion lookup:                       ~150-200 ns (mmap), <2 ms IPC
//   - Inline autosuggest:                          <2 ms IPC roundtrip
//   - Syntax highlight per keystroke:              <2 ms IPC roundtrip
//
// We measure end-to-end client→daemon→client wall-clock latency for the
// hot ops: ping, complete, suggest, highlight, history_query. Each op is
// run N times against a freshly-connected client; the connection-+-handshake
// cost is also measured separately.
//
// Output is a JSON object with min / p50 / p90 / p99 / max in microseconds.
// No criterion dep — keeps the daemon crate self-contained per CLAUDE.md
// endgame rules. Use:
//
//   cargo build --bin zshrs-daemon-bench -p zshrs-daemon
//   ./target/debug/zshrs-daemon-bench --runs 1000

use std::time::Instant;

use serde_json::{json, Value};
use zshrs_daemon::client::Client;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let runs: usize = args
        .iter()
        .position(|a| a == "--runs")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    let no_spawn = args.iter().any(|a| a == "--no-spawn");

    let paths = match zshrs_daemon::CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("zshrs-daemon-bench: cache paths: {}", e);
            std::process::exit(1);
        }
    };

    // Connect (spawning the daemon if needed). Measure handshake latency
    // separately because cold-launch is a single big budget item per spec.
    let connect_start = Instant::now();
    let connect_res = if no_spawn {
        Client::connect_existing(&paths)
    } else {
        Client::connect(&paths)
    };
    let mut client = match connect_res {
        Ok(c) => c,
        Err(e) => {
            eprintln!("zshrs-daemon-bench: connect: {}", e);
            std::process::exit(1);
        }
    };
    let connect_us = connect_start.elapsed().as_micros() as u64;

    let mut report = serde_json::Map::new();
    report.insert("runs".into(), json!(runs));
    report.insert("connect_us".into(), json!(connect_us));

    // Warm-up: 50 calls to take JIT path / TCP-style buffering effects out.
    for _ in 0..50 {
        let _ = client.call("ping", json!({}));
    }

    for (name, args_for_op) in [
        ("ping", json!({})),
        ("complete", json!({ "prefix": "g", "limit": 32 })),
        ("suggest", json!({ "prefix": "git " })),
        ("highlight", json!({ "line": "ls -la /tmp" })),
        ("history_query", json!({ "filter": "cargo", "mode": "fts", "limit": 32 })),
        ("info", json!({})),
        ("watcher_stats", json!({})),
    ] {
        let stats = measure(&mut client, name, args_for_op, runs);
        report.insert(name.into(), stats);
    }

    let json_out = Value::Object(report);
    println!("{}", serde_json::to_string_pretty(&json_out).unwrap());

    // Quick acceptance-criteria check vs DAEMON.md targets. Print a one-line
    // verdict to stderr (stdout is the JSON output for downstream tooling).
    let p99_ping = ms_p99(&json_out, "ping");
    let p99_suggest = ms_p99(&json_out, "suggest");
    let p99_highlight = ms_p99(&json_out, "highlight");
    let p99_hquery = ms_p99(&json_out, "history_query");
    eprintln!(
        "verdict: ping p99={:.2}ms (target <2ms) suggest p99={:.2}ms (target <2ms) highlight p99={:.2}ms (target <2ms) history_query p99={:.2}ms (target <2ms)",
        p99_ping, p99_suggest, p99_highlight, p99_hquery
    );
}

fn measure(client: &mut Client, op: &str, args: Value, runs: usize) -> Value {
    let mut us: Vec<u64> = Vec::with_capacity(runs);
    let mut errs: usize = 0;
    for _ in 0..runs {
        let t0 = Instant::now();
        match client.call(op, args.clone()) {
            Ok(_) => {
                us.push(t0.elapsed().as_micros() as u64);
            }
            Err(_) => errs += 1,
        }
    }
    if us.is_empty() {
        return json!({ "errors": errs });
    }
    us.sort_unstable();
    let len = us.len() as f64;
    let p = |q: f64| -> u64 {
        let idx = ((len - 1.0) * q).round() as usize;
        us[idx.min(us.len() - 1)]
    };
    let mean = us.iter().sum::<u64>() as f64 / len;
    json!({
        "samples": us.len(),
        "errors": errs,
        "min_us": us.first().copied().unwrap_or(0),
        "p50_us": p(0.50),
        "p90_us": p(0.90),
        "p99_us": p(0.99),
        "max_us": us.last().copied().unwrap_or(0),
        "mean_us": mean.round() as u64,
    })
}

fn ms_p99(report: &Value, op: &str) -> f64 {
    report
        .get(op)
        .and_then(|v| v.get("p99_us"))
        .and_then(|v| v.as_u64())
        .map(|us| us as f64 / 1000.0)
        .unwrap_or(f64::NAN)
}
