// Integration tests for the zshrs-daemon foundation (see docs/DAEMON.md, src/daemon/).
//
// Each test:
//   1. Creates an isolated $ZSHRS_HOME via tempdir.
//   2. Spawns `zshrs --daemon` with that cache home.
//   3. Drives the daemon via the sync IPC client (zsh::daemon::client::Client).
//   4. Tears the daemon down by invoking the `daemon stop` op (or kills it).
//
// Because each test owns its own cache root, parallel tests don't share state
// and the singleton bin_zsystem_flock isn't a hazard within a test. The binary used is the
// just-built target/debug/zshrs.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;

use zsh::daemon::client::Client;
use zsh::daemon::paths::CachePaths;
use zsh::daemon::pidlock;

const SPAWN_GRACE: Duration = Duration::from_secs(3);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Path to the built zshrs-daemon binary. The daemon is now its own
/// binary (the `--daemon` arg on the shell was removed); spawn it
/// directly.
///
/// `zshrs-daemon` lives in the `zshrs-daemon` package, not this one, so
/// cargo sets `CARGO_BIN_EXE_zshrs-daemon` only when the tests are run
/// from that package — running `cargo test --test daemon_integration`
/// here does NOT rebuild it, and the fallback path below happily picks
/// up whatever `target/debug/zshrs-daemon` was left by some earlier
/// build. That is how this suite ended up asserting against an 8-day-old
/// daemon: 27 of its 29 tests passed against a binary that predated the
/// code under test, and only the two `CARGO_PKG_VERSION` assertions
/// noticed (`0.12.36` vs `0.12.40`).
///
/// So build it. `cargo build -p zshrs-daemon` is a no-op when the binary
/// is current, and cargo's target-directory lock is not held while test
/// binaries run, so the nested invocation does not deadlock. Done once
/// per process via `OnceLock`.
fn zshrs_daemon_binary() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs-daemon") {
        return PathBuf::from(p);
    }
    static BUILT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BUILT
        .get_or_init(|| {
            let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
            let out = Command::new(cargo)
                .current_dir(&manifest)
                .args(["build", "-p", "zshrs-daemon", "--bin", "zshrs-daemon"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .expect("spawn cargo build -p zshrs-daemon");
            assert!(
                out.status.success(),
                "cargo build -p zshrs-daemon failed:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let target = std::env::var_os("CARGO_TARGET_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| manifest.join("target"));
            let bin = target.join("debug").join("zshrs-daemon");
            assert!(
                bin.exists(),
                "zshrs-daemon missing after build: {}",
                bin.display()
            );
            bin
        })
        .clone()
}

struct DaemonHandle {
    _tmpdir: tempfile::TempDir,
    paths: CachePaths,
    child: Option<Child>,
}

impl DaemonHandle {
    fn spawn() -> Self {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let zshrs_home = tmp.path().to_path_buf();

        let child = Command::new(zshrs_daemon_binary())
            .env("ZSHRS_HOME", &zshrs_home)
            .env("ZSHRS_QUIET_FIRST_RUN", "1")
            // Point every "where does the user keep shell state" env var
            // at the tempdir too. `ZSHRS_HOME` alone does NOT isolate the
            // daemon: `ops::legacy_scope_dirs` (daemon/ops.rs) adds
            // `dirs::home_dir()` + `$ZDOTDIR` / `$XDG_CACHE_HOME` /
            // `$ZPWR_LOCAL` / `$ZSH` to the scope that `verify` walks
            // (depth 4) hunting stale `.zwc` / `.zcompdump`. On a real
            // developer's account that walk is enormous, so `verify`
            // outran the client's 5s read timeout and the test failed
            // with EAGAIN — a pure property of whose $HOME it ran in.
            .env("HOME", &zshrs_home)
            .env("ZDOTDIR", &zshrs_home)
            .env("XDG_CACHE_HOME", &zshrs_home)
            .env("ZPWR_LOCAL", &zshrs_home)
            .env("ZSH", &zshrs_home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("daemon spawn");

        let paths = CachePaths::with_root(zshrs_home);

        // Wait until the daemon ACCEPTS a connection, not merely until
        // the socket file appears. `UnixListener::bind` creates the file
        // before `listen(2)` runs, and a connect landing in that window
        // gets ECONNREFUSED — invisible on an idle machine, reproducible
        // as soon as several test binaries run at once (6 concurrent runs
        // of this file produced 2-6 spurious "Connection refused" panics
        // each). `is_daemon_alive` is the daemon crate's own probe: it
        // connects and drops without doing the handshake.
        let start = Instant::now();
        while start.elapsed() < SPAWN_GRACE {
            if Client::is_daemon_alive(&paths) {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        assert!(
            Client::is_daemon_alive(&paths),
            "daemon failed to accept on socket at {}",
            paths.socket.display()
        );

        Self {
            _tmpdir: tmp,
            paths,
            child: Some(child),
        }
    }

    /// Connect a sync client to this daemon. Hits the existing daemon (no spawn).
    fn connect(&self) -> Client {
        Client::connect_existing(&self.paths).expect("connect to daemon")
    }
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Attempt graceful shutdown via `daemon stop`.
            if let Ok(mut c) = Client::connect_existing(&self.paths) {
                let _ = c.call("daemon", json!({ "verb": "stop" }));
            }
            let waited = Instant::now();
            while waited.elapsed() < Duration::from_millis(500) {
                if let Ok(Some(_)) = child.try_wait() {
                    return;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            // Hard kill if still alive.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

// -------- Tests --------

#[test]
fn handshake_and_info() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    assert_eq!(c.welcome.client_id, 1);
    assert!(c.welcome.daemon_pid > 0);

    let info = c.call("info", json!({})).expect("info");
    assert_eq!(info["version"].as_str().unwrap(), env!("CARGO_PKG_VERSION"));
    assert_eq!(info["protocol_version"].as_u64(), Some(1));
    assert_eq!(info["session_count"].as_u64(), Some(1));
    assert!(info["catalog"].is_object());
    // catalog/SCHEMA_VERSION lives in daemon/catalog.rs:42 — bump
    // here in lockstep with that constant.
    assert_eq!(info["catalog"]["schema_version"].as_u64(), Some(2));
}

#[test]
fn ping_roundtrip() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    let resp = c.call("ping", json!({ "echo": "hello" })).expect("ping");
    assert_eq!(resp["pong"], json!(true));
    assert_eq!(resp["echo"], json!("hello"));
    assert!(resp["daemon_uptime_ms"].as_u64().is_some());
}

#[test]
fn list_shells_returns_self() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let resp = c.call("list_shells", json!({})).expect("list_shells");
    let shells = resp["shells"].as_array().unwrap();
    assert_eq!(shells.len(), 1);
    assert_eq!(shells[0]["client_id"].as_u64(), Some(1));
}

#[test]
fn tag_then_untag_roundtrip() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    let r = c
        .call("tag", json!({ "tags": ["prod", "canary"] }))
        .expect("tag");
    let tags = r["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 2);
    assert!(tags.iter().any(|v| v == "prod"));
    assert!(tags.iter().any(|v| v == "canary"));

    let r = c.call("untag", json!({ "tags": ["prod"] })).expect("untag");
    let tags = r["tags"].as_array().unwrap();
    assert_eq!(tags, &vec![json!("canary")]);

    let r = c
        .call("untag", json!({ "all": true }))
        .expect("untag --all");
    assert!(r["tags"].as_array().unwrap().is_empty());
}

#[test]
fn list_shells_with_tag_filter() {
    let d = DaemonHandle::spawn();

    let mut c1 = d.connect();
    c1.call("tag", json!({ "tags": ["dev"] })).unwrap();

    let mut c2 = d.connect();
    c2.call("tag", json!({ "tags": ["prod"] })).unwrap();

    // Either client can drive the filter — they share the daemon registry.
    let mut probe = d.connect();
    let r = probe
        .call("list_shells", json!({ "tag": "dev" }))
        .expect("list_shells dev");
    let shells = r["shells"].as_array().unwrap();
    assert_eq!(shells.len(), 1);

    let r = probe
        .call("list_shells", json!({ "tag": "prod" }))
        .expect("list_shells prod");
    assert_eq!(r["shells"].as_array().unwrap().len(), 1);
}

#[test]
fn send_to_specific_shell() {
    let d = DaemonHandle::spawn();
    let mut sender = d.connect();
    let receiver = d.connect();
    let target_id = receiver.welcome.client_id;
    drop(receiver); // close so send actually fails (good — verifies error path)

    // Give the daemon a beat to register the disconnect before send.
    std::thread::sleep(Duration::from_millis(50));

    let res = sender.call(
        "send",
        json!({
            "target": { "shell_id": target_id },
            "command": "echo hi"
        }),
    );
    assert!(res.is_err(), "send to disconnected shell should fail");
}

#[test]
fn send_broadcast_excludes_self() {
    let d = DaemonHandle::spawn();
    let mut a = d.connect();
    let _b = d.connect();
    let _c = d.connect();

    let r = a
        .call(
            "send",
            json!({
                "target": { "all": true },
                "command": "rehash"
            }),
        )
        .expect("send --all");

    // Two other clients (b, c) — sender excluded.
    assert_eq!(r["delivered_count"].as_u64(), Some(2));
}

#[test]
fn singleton_pid_lock() {
    let d = DaemonHandle::spawn();
    let pid = pidlock::read_pid(&d.paths).expect("pidfile written");
    assert!(pid > 0);
    assert!(pidlock::pid_alive(pid));

    // Try to spawn a second daemon against the same root; it should detect the lock
    // and exit cleanly (status 0, per docs/DAEMON.md daemon lifecycle).
    let status = Command::new(zshrs_daemon_binary())
        .env("ZSHRS_HOME", &d.paths.root)
        .env("ZSHRS_QUIET_FIRST_RUN", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("second daemon spawn");
    assert!(status.success(), "second daemon should exit cleanly");

    // Original daemon should still be running.
    let r = d
        .connect()
        .call("ping", json!({}))
        .expect("original daemon still alive");
    assert_eq!(r["pong"], json!(true));
}

#[test]
fn unknown_op_returns_error() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let res = c.call("definitely_not_an_op", json!({}));
    assert!(res.is_err());
    let err_str = format!("{}", res.unwrap_err());
    assert!(err_str.contains("unknown_op") || err_str.contains("definitely_not_an_op"));
}

#[test]
fn daemon_status_op() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let r = c
        .call("daemon", json!({ "verb": "status" }))
        .expect("daemon status");
    assert!(r["pid"].as_i64().is_some());
    assert!(r["uptime_ms"].as_u64().is_some());
    assert_eq!(r["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
}

#[test]
fn unknown_op_returns_unknown_op_code() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    // Pin the unknown-op fall-through arm in daemon/ops.rs:171. The
    // op set has grown (recorder_ingest, definitions_*, metrics, …)
    // so picking a real-but-unimplemented op is unstable; use a
    // syntactically valid name that's guaranteed to never exist.
    let res = c.call("zzz_definitely_not_a_real_op_xyz", json!({}));
    assert!(res.is_err());
    let msg = format!("{}", res.unwrap_err());
    assert!(
        msg.contains("unknown_op") || msg.contains("unsupported op"),
        "unexpected error shape: {msg}"
    );
}

#[test]
fn cache_dir_is_0700() {
    let d = DaemonHandle::spawn();
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&d.paths.root)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn catalog_db_created() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let r = c.call("info", json!({})).expect("info");
    let catalog = &r["catalog"];
    // SCHEMA_VERSION lives in daemon/catalog.rs:42.
    assert_eq!(catalog["schema_version"].as_u64(), Some(2));
    assert!(catalog["size_bytes"].as_u64().unwrap() > 0);
    assert!(d.paths.catalog_db.exists());
}

// `rebuild_creates_shard` was removed: the `rebuild` op + the AST-walk
// pipeline it drove (daemon/walk.rs, daemon/plugin_walk.rs,
// daemon/ast_walker.rs) are gone. Per daemon/ops.rs:535 — state
// attribution is now sourced from `zshrs-recorder` via
// `recorder_ingest` (see docs/RECORDER.md). Shard creation is now
// covered by the recorder_harness corpus tests.

#[test]
fn verify_passes_on_fresh_daemon() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let r = c.call("verify", json!({})).expect("verify");
    assert_eq!(r["verified"], json!(true));
    assert_eq!(r["catalog_ok"], json!(true));
}

#[test]
fn clean_shards_op_works_on_empty_cache() {
    // Pin the `clean target=shards` op shape on a fresh daemon. The
    // pre-recorder version of this test seeded a shard via the now-
    // removed `rebuild` op then asserted clean removed it; today the
    // shard population path is `recorder_ingest`, which is exercised
    // separately by the recorder corpus harness. This test just
    // verifies the clean op surface stays stable.
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    let r = c
        .call("clean", json!({ "target": "shards" }))
        .expect("clean");
    assert_eq!(r["removed_count"].as_u64(), Some(0));
    assert!(r["removed"].as_array().unwrap().is_empty());

    let info = c.call("info", json!({})).expect("info");
    assert_eq!(info["shards"].as_array().unwrap().len(), 0);
}

#[test]
fn source_resolve_miss_then_hit() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    // Create a real file to source.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"alias hi='echo hello'\n").unwrap();
    let path = tmp.path().to_str().unwrap().to_string();

    let r = c
        .call(
            "source_resolve",
            json!({ "path": path, "mtime_ns": 0, "inode": 0 }),
        )
        .expect("source_resolve miss");
    assert_eq!(r["hit"], json!(false));
    assert!(r["bytes_in"].as_i64().unwrap() > 0);
    assert_eq!(r["sensitive"], json!(false));

    // Same path again — should hit.
    let r = c
        .call(
            "source_resolve",
            json!({ "path": path, "mtime_ns": 0, "inode": 0 }),
        )
        .expect("source_resolve hit");
    assert_eq!(r["hit"], json!(true));
}

#[test]
fn source_resolve_flags_sensitive_files() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    // Tempfile with .tokens-style name.
    let dir = tempfile::TempDir::new().unwrap();
    let secret = dir.path().join(".tokens.sh");
    std::fs::write(&secret, b"export AWS_SECRET_ACCESS_KEY=fake\n").unwrap();

    let r = c
        .call(
            "source_resolve",
            json!({ "path": secret.to_str().unwrap(), "mtime_ns": 0, "inode": 0 }),
        )
        .expect("source_resolve");
    assert_eq!(r["sensitive"], json!(true));
}

#[test]
fn source_resolve_rejects_relative_path() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let res = c.call("source_resolve", json!({ "path": "relative/path.sh" }));
    assert!(res.is_err());
    let s = format!("{}", res.unwrap_err());
    assert!(s.contains("absolute"));
}

#[test]
fn history_append_then_query() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    for cmd in [
        "git status",
        "cargo build --release",
        "git push origin main",
        "ls -la",
    ] {
        c.call(
            "history_append",
            json!({ "line": cmd, "ts_ns": 1000, "exit_code": 0 }),
        )
        .expect("append");
    }

    // FTS match for `git`.
    let r = c
        .call(
            "history_query",
            json!({ "filter": "git", "mode": "match", "limit": 100 }),
        )
        .expect("query");
    assert_eq!(r["count"].as_u64(), Some(2));
    let rows = r["rows"].as_array().unwrap();
    assert!(rows.iter().any(|r| r["line"] == "git status"));
    assert!(rows.iter().any(|r| r["line"] == "git push origin main"));
}

#[test]
fn history_query_recent_no_filter() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    for i in 1..=20 {
        c.call(
            "history_append",
            json!({ "line": format!("cmd_{}", i), "ts_ns": i * 100 }),
        )
        .unwrap();
    }
    let r = c
        .call("history_query", json!({ "limit": 5, "descending": true }))
        .expect("query");
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0]["line"], "cmd_20");
    assert_eq!(rows[4]["line"], "cmd_16");
}

#[test]
fn subscribe_unsubscribe_lifecycle() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    let r = c
        .call("subscribe", json!({ "pattern": "*.commands" }))
        .expect("subscribe");
    let sub_id = r["subscription_id"].as_u64().unwrap();
    assert_eq!(r["pattern"], "*.commands");

    let r = c
        .call("unsubscribe", json!({ "id": sub_id }))
        .expect("unsubscribe by id");
    assert_eq!(r["removed"], json!(1));

    // Second unsubscribe is a no-op (already gone).
    let r = c
        .call("unsubscribe", json!({ "id": sub_id }))
        .expect("unsubscribe absent");
    assert_eq!(r["removed"], json!(0));
}

#[test]
fn subscribe_rejects_bad_pattern() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let res = c.call("subscribe", json!({ "pattern": "no_separator" }));
    assert!(res.is_err());
    assert!(format!("{}", res.unwrap_err()).contains("bad_pattern"));
}

#[test]
fn publish_deliver_to_matching_subscribers() {
    let d = DaemonHandle::spawn();
    let publisher = d.connect();
    drop(publisher); // any client can publish; we don't actually need its handle for that

    // Two distinct subscribers.
    let mut sub_a = d.connect();
    let mut sub_b = d.connect();
    sub_a
        .call("subscribe", json!({ "pattern": "*.commands" }))
        .expect("sub a");
    sub_b
        .call("subscribe", json!({ "pattern": "*.chpwd" }))
        .expect("sub b");

    // A third client publishes a `commands` event. Sub A matches, B doesn't.
    let mut publisher = d.connect();
    let r = publisher
        .call(
            "publish",
            json!({ "topic": "commands", "data": { "line": "ls" } }),
        )
        .expect("publish");
    assert_eq!(r["delivered_to"].as_u64(), Some(1));

    // chpwd: only sub B matches.
    let r = publisher
        .call(
            "publish",
            json!({ "topic": "chpwd", "data": { "cwd": "/tmp" } }),
        )
        .expect("publish");
    assert_eq!(r["delivered_to"].as_u64(), Some(1));

    // unknown topic — zero subscribers.
    let r = publisher
        .call(
            "publish",
            json!({ "topic": "totally_unhandled", "data": {} }),
        )
        .expect("publish");
    assert_eq!(r["delivered_to"].as_u64(), Some(0));
}

#[test]
fn push_pull_canonical_roundtrip() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    let r = c
        .call(
            "push_canonical",
            json!({ "subsystem": "alias", "value": { "ll": "ls -la", "gst": "git status" } }),
        )
        .expect("push");
    assert_eq!(r["promoted"].as_u64(), Some(2));

    let r = c
        .call("pull_canonical", json!({ "subsystem": "alias" }))
        .expect("pull");
    let rows = r["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn export_aliases_sh_format() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    c.call(
        "push_canonical",
        json!({ "subsystem": "alias", "value": { "ll": "ls -la" } }),
    )
    .unwrap();
    let r = c
        .call("export", json!({ "target": "aliases" }))
        .expect("export");
    let body = r["body"].as_str().unwrap();
    assert!(body.contains("alias ll="));
    assert!(body.contains("unalias -m"));
}

#[test]
fn ask_take_pop_critical_first() {
    let d = DaemonHandle::spawn();
    let mut asker = d.connect();
    let target = d.connect();
    drop(target);

    // Need the target connected for ask_ask to accept; reconnect after.
    let mut target = d.connect();
    let target_id = target.welcome.client_id;

    asker
        .call(
            "ask_ask",
            json!({
                "kind": "picker",
                "target": { "shell_id": target_id },
                "urgency": "normal",
                "payload": { "items": ["a", "b"] }
            }),
        )
        .expect("normal ask");
    asker
        .call(
            "ask_ask",
            json!({
                "kind": "dialog",
                "target": { "shell_id": target_id },
                "urgency": "critical",
                "payload": { "message": "ok?" }
            }),
        )
        .expect("critical ask");

    // target takes — critical first.
    let r = target.call("ask_take", json!({})).expect("take");
    assert_eq!(r["kind"].as_str(), Some("dialog"));
    assert_eq!(r["urgency"].as_str(), Some("critical"));

    let r = target.call("ask_pending", json!({})).expect("pending");
    assert_eq!(r["pending_count"].as_u64(), Some(1));
}

#[test]
fn keys_op_returns_canonical_keys() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    c.call(
        "push_canonical",
        json!({
            "subsystem": "compdef",
            "value": { "git": "_git", "docker": "_docker" }
        }),
    )
    .unwrap();
    let r = c.call("keys", json!({ "param": "_comps" })).expect("keys");
    let keys = r["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert!(keys.iter().any(|v| v == "git"));
}

#[test]
fn stats_flush_merges_deltas() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let r = c
        .call(
            "stats_flush",
            json!({
                "deltas": [
                    { "fq_name": "_git", "calls": 3, "total_ns": 1500, "last_ns": 1000 },
                    { "fq_name": "_docker", "calls": 1, "total_ns": 500 }
                ]
            }),
        )
        .expect("flush");
    assert_eq!(r["merged"].as_u64(), Some(2));

    // Second flush merges via ON CONFLICT DO UPDATE.
    let r = c
        .call(
            "stats_flush",
            json!({
                "deltas": [{ "fq_name": "_git", "calls": 2, "total_ns": 800, "last_ns": 2000 }]
            }),
        )
        .expect("second flush");
    assert_eq!(r["merged"].as_u64(), Some(1));
}

#[test]
fn subscriptions_cleared_on_disconnect() {
    let d = DaemonHandle::spawn();
    let mut sub_client = d.connect();
    sub_client
        .call("subscribe", json!({ "pattern": "*.commands" }))
        .unwrap();
    drop(sub_client); // disconnect

    // Give the daemon a beat to register the disconnect cleanup.
    std::thread::sleep(std::time::Duration::from_millis(80));

    let mut probe = d.connect();
    let r = probe
        .call("publish", json!({ "topic": "commands", "data": {} }))
        .expect("publish");
    assert_eq!(r["delivered_to"].as_u64(), Some(0));
}
