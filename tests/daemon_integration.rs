// Integration tests for the zshrs-daemon foundation (see docs/DAEMON.md, src/daemon/).
//
// Each test:
//   1. Creates an isolated XDG_CACHE_HOME via tempdir.
//   2. Spawns `zshrs --daemon` with that cache home.
//   3. Drives the daemon via the sync IPC client (zsh::daemon::client::Client).
//   4. Tears the daemon down by invoking the `daemon stop` op (or kills it).
//
// Because each test owns its own cache root, parallel tests don't share state
// and the singleton flock isn't a hazard within a test. The binary used is the
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

/// Path to the built zshrs binary. Falls back to walking the workspace target
/// directory if the standard CARGO_BIN_EXE_zshrs env isn't set (older cargo).
fn zshrs_binary() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}

struct DaemonHandle {
    _tmpdir: tempfile::TempDir,
    paths: CachePaths,
    child: Option<Child>,
}

impl DaemonHandle {
    fn spawn() -> Self {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let cache_home = tmp.path().to_path_buf();

        let child = Command::new(zshrs_binary())
            .arg("--daemon")
            .env("XDG_CACHE_HOME", &cache_home)
            .env("ZSHRS_QUIET_FIRST_RUN", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("daemon spawn");

        let paths = CachePaths::with_root(cache_home.join("zshrs"));

        // Wait for the socket to appear.
        let start = Instant::now();
        while start.elapsed() < SPAWN_GRACE {
            if paths.socket.exists() {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        assert!(
            paths.socket.exists(),
            "daemon failed to bind socket at {}",
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
    assert_eq!(info["catalog"]["schema_version"].as_u64(), Some(1));
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

    let r = c.call("untag", json!({ "all": true })).expect("untag --all");
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

    // Try to spawn a second daemon against the same cache; it should detect the lock
    // and exit cleanly (status 0, per docs/DAEMON.md daemon lifecycle).
    let cache_home = d.paths.root.parent().unwrap().to_path_buf();
    let status = Command::new(zshrs_binary())
        .arg("--daemon")
        .env("XDG_CACHE_HOME", &cache_home)
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
fn unimplemented_op_returns_unimplemented_code() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    // `complete` (Tab completion enumeration) isn't implemented in v1 foundation
    // — pick any op known to be in the unimplemented bucket in src/daemon/ops.rs.
    let res = c.call("complete", json!({}));
    assert!(res.is_err());
    assert!(format!("{}", res.unwrap_err()).contains("unimplemented"));
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
    assert_eq!(catalog["schema_version"].as_u64(), Some(1));
    assert!(catalog["size_bytes"].as_u64().unwrap() > 0);
    assert!(d.paths.catalog_db.exists());
}

#[test]
fn rebuild_creates_shard() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    let r = c.call("rebuild", json!({})).expect("rebuild");
    assert_eq!(r["rebuilt"].as_array().unwrap().len(), 1);
    let path = r["path"].as_str().unwrap();
    assert!(std::path::Path::new(path).exists());

    let info = c.call("info", json!({})).expect("info");
    let shards = info["shards"].as_array().unwrap();
    assert_eq!(shards.len(), 1);
}

#[test]
fn verify_passes_on_fresh_daemon() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    let r = c.call("verify", json!({})).expect("verify");
    assert_eq!(r["verified"], json!(true));
    assert_eq!(r["catalog_ok"], json!(true));
}

#[test]
fn clean_shards_removes_them() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();
    c.call("rebuild", json!({})).unwrap();

    let r = c.call("clean", json!({ "target": "shards" })).expect("clean");
    assert_eq!(r["removed_count"].as_u64(), Some(1));

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
        .call("source_resolve", json!({ "path": path, "mtime_ns": 0, "inode": 0 }))
        .expect("source_resolve miss");
    assert_eq!(r["hit"], json!(false));
    assert!(r["bytes_in"].as_i64().unwrap() > 0);
    assert_eq!(r["sensitive"], json!(false));

    // Same path again — should hit.
    let r = c
        .call("source_resolve", json!({ "path": path, "mtime_ns": 0, "inode": 0 }))
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
