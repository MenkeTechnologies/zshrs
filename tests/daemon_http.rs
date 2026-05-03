//! Integration tests for the daemon's HTTP listener (see daemon/http.rs +
//! docs/DAEMON_AS_SERVICE.md).
//!
//! Each test:
//!   1. Allocates a free TCP port.
//!   2. Sets up an isolated `$XDG_CONFIG_HOME` with a `daemon.toml` that
//!      enables the HTTP listener on that port.
//!   3. Sets up an isolated `$XDG_CACHE_HOME` so the spawned daemon
//!      doesn't collide with any developer-machine daemon already
//!      running.
//!   4. Spawns `target/debug/zshrs --daemon`.
//!   5. Polls `GET /health` until the listener is up.
//!   6. Drives the listener via `ureq` and asserts response shapes.
//!   7. Kills the daemon on Drop.

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SPAWN_GRACE: Duration = Duration::from_secs(8);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

fn zshrs_binary() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}

/// Allocate a kernel-assigned free TCP port by binding 127.0.0.1:0
/// then dropping the listener; the port number stays free long enough
/// for the daemon to bind it next.
fn pick_free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

struct DaemonHttp {
    _cache: tempfile::TempDir,
    _config: tempfile::TempDir,
    port: u16,
    child: Option<Child>,
}

impl DaemonHttp {
    fn spawn(token: Option<&str>) -> Self {
        let cache = tempfile::TempDir::new().expect("cache tempdir");
        let config = tempfile::TempDir::new().expect("config tempdir");
        let port = pick_free_port();

        // Write daemon.toml.
        let cfg_dir = config.path().join("zshrs");
        std::fs::create_dir_all(&cfg_dir).expect("mk config dir");
        let mut f = std::fs::File::create(cfg_dir.join("daemon.toml")).expect("create toml");
        write!(f, "[http]\nlisten = \"127.0.0.1:{port}\"\n").unwrap();
        if let Some(tok) = token {
            write!(f, "\n[http.tokens]\ntest-tok = \"{tok}\"\n").unwrap();
        }
        drop(f);

        let child = Command::new(zshrs_binary())
            .arg("--daemon")
            .env("XDG_CACHE_HOME", cache.path())
            .env("XDG_CONFIG_HOME", config.path())
            .env("ZSHRS_QUIET_FIRST_RUN", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("daemon spawn");

        let me = Self {
            _cache: cache,
            _config: config,
            port,
            child: Some(child),
        };
        me.wait_ready();
        me
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }

    fn wait_ready(&self) {
        let start = Instant::now();
        while start.elapsed() < SPAWN_GRACE {
            if let Ok(resp) = ureq::get(&self.url("/health")).timeout(Duration::from_millis(200)).call() {
                if resp.status() == 200 {
                    return;
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        panic!(
            "daemon http listener did not come up at 127.0.0.1:{} within {SPAWN_GRACE:?}",
            self.port
        );
    }
}

impl Drop for DaemonHttp {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[test]
fn health_endpoint_returns_version_and_uptime() {
    let d = DaemonHttp::spawn(None);
    let resp = ureq::get(&d.url("/health")).call().expect("GET /health");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.into_json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert!(
        body["version"].as_str().is_some(),
        "version field missing: {body}"
    );
    assert!(
        body["uptime_ms"].as_u64().is_some(),
        "uptime_ms field missing: {body}"
    );
}

#[test]
fn ops_endpoint_lists_known_ops() {
    let d = DaemonHttp::spawn(None);
    let resp = ureq::get(&d.url("/ops")).call().expect("GET /ops");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.into_json().expect("json");
    let ops = body["ops"]
        .as_array()
        .expect("ops array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();
    for must in ["ping", "info", "recorder_ingest", "config_get"] {
        assert!(ops.contains(&must), "missing op {must:?} in /ops list");
    }
}

#[test]
fn op_ping_returns_pong() {
    let d = DaemonHttp::spawn(None);
    let resp = ureq::post(&d.url("/op/ping"))
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("POST /op/ping");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.into_json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["pong"], serde_json::json!(true));
}

#[test]
fn unknown_op_returns_404() {
    let d = DaemonHttp::spawn(None);
    let resp = ureq::post(&d.url("/op/this_op_does_not_exist"))
        .set("Content-Type", "application/json")
        .send_string("{}");
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("unexpected ureq error: {e}"),
    };
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.into_json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["code"], serde_json::json!("unknown_op"));
}

#[test]
fn auth_required_when_tokens_configured() {
    let d = DaemonHttp::spawn(Some("test-secret-456"));

    // No token → 401.
    let r = ureq::post(&d.url("/op/ping"))
        .set("Content-Type", "application/json")
        .send_string("{}");
    let r = match r {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("unexpected ureq error: {e}"),
    };
    assert_eq!(r.status(), 401, "expected 401 without bearer token");

    // Wrong token → 401.
    let r = ureq::post(&d.url("/op/ping"))
        .set("Authorization", "Bearer wrong")
        .set("Content-Type", "application/json")
        .send_string("{}");
    let r = match r {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("unexpected ureq error: {e}"),
    };
    assert_eq!(r.status(), 401, "expected 401 with wrong bearer token");

    // Right token → 200.
    let r = ureq::post(&d.url("/op/ping"))
        .set("Authorization", "Bearer test-secret-456")
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("authorized POST should succeed");
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.into_json().expect("json");
    assert_eq!(body["pong"], serde_json::json!(true));
}

#[test]
fn cache_round_trip() {
    let d = DaemonHttp::spawn(None);
    // put
    let r = ureq::post(&d.url("/op/cache_put"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"ns":"t","key":"k1","value":"hello"}"#)
        .expect("cache_put");
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["bytes"], serde_json::json!(5));

    // get
    let r = ureq::post(&d.url("/op/cache_get"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"ns":"t","key":"k1"}"#)
        .expect("cache_get");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["value"], serde_json::json!("hello"));

    // list
    let r = ureq::post(&d.url("/op/cache_list"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"ns":"t"}"#)
        .expect("cache_list");
    let body: serde_json::Value = r.into_json().unwrap();
    let keys = body["keys"].as_array().unwrap();
    assert!(keys.iter().any(|v| v.as_str() == Some("k1")));

    // delete
    let r = ureq::post(&d.url("/op/cache_del"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"ns":"t","key":"k1"}"#)
        .expect("cache_del");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["deleted"], serde_json::json!(true));

    // get after delete → 404
    let r = ureq::post(&d.url("/op/cache_get"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"ns":"t","key":"k1"}"#);
    let r = match r {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("ureq error: {e}"),
    };
    assert_eq!(r.status(), 404);
}

#[test]
fn lock_acquire_release_roundtrip() {
    let d = DaemonHttp::spawn(None);
    let pid = std::process::id();

    // try_acquire returns a token
    let r = ureq::post(&d.url("/op/lock_try_acquire"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"name":"L","pid":{pid}}}"#))
        .expect("lock_try_acquire");
    let body: serde_json::Value = r.into_json().unwrap();
    let token = body["token"].as_str().expect("token").to_string();

    // second try → busy (409 in our HTTP mapping, status code = 500 here
    // since `busy` isn't in the http.rs whitelist; check ok=false instead)
    let r = ureq::post(&d.url("/op/lock_try_acquire"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"name":"L","pid":{pid}}}"#));
    let r = match r {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("ureq error: {e}"),
    };
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["ok"], serde_json::json!(false));
    assert_eq!(body["code"], serde_json::json!("busy"));

    // release
    let r = ureq::post(&d.url("/op/lock_release"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"name":"L","token":"{token}"}}"#))
        .expect("lock_release");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["released"], serde_json::json!(true));

    // re-acquire after release works
    let r = ureq::post(&d.url("/op/lock_try_acquire"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"name":"L","pid":{pid}}}"#))
        .expect("re-acquire");
    let body: serde_json::Value = r.into_json().unwrap();
    assert!(body["token"].as_str().is_some());
}

#[test]
fn artifact_round_trip() {
    let d = DaemonHttp::spawn(None);
    // put with literal value (UTF-8 string) — exercises the non-base64 path
    let r = ureq::post(&d.url("/op/artifact_put"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"name":"art-x","value":"some bytes"}"#)
        .expect("artifact_put");
    let body: serde_json::Value = r.into_json().unwrap();
    let digest = body["digest"].as_str().expect("digest").to_string();
    assert_eq!(digest.len(), 64); // sha256 hex

    // get by name → returns the bytes base64-encoded
    let r = ureq::post(&d.url("/op/artifact_get"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"name":"art-x"}"#)
        .expect("artifact_get");
    let body: serde_json::Value = r.into_json().unwrap();
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body["value_base64"].as_str().unwrap())
        .expect("base64 decode");
    assert_eq!(&bytes, b"some bytes");

    // get by digest works too
    let r = ureq::post(&d.url("/op/artifact_get_by_digest"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"digest":"{digest}"}}"#))
        .expect("artifact_get_by_digest");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["digest"], serde_json::json!(digest));
}

#[test]
fn snapshot_save_list_diff() {
    let d = DaemonHttp::spawn(None);
    // save baseline
    let r = ureq::post(&d.url("/op/snapshot_save"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"tag":"sn-base"}"#)
        .expect("snapshot_save");
    let body: serde_json::Value = r.into_json().unwrap();
    assert!(body["bytes"].as_u64().unwrap() > 0);

    // list contains the tag
    let r = ureq::post(&d.url("/op/snapshot_list"))
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("snapshot_list");
    let body: serde_json::Value = r.into_json().unwrap();
    let tags: Vec<&str> = body["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["tag"].as_str())
        .collect();
    assert!(tags.contains(&"sn-base"));

    // self-diff → all empty
    let r = ureq::post(&d.url("/op/snapshot_diff"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"a":"sn-base","b":"sn-base"}"#)
        .expect("snapshot_diff");
    let body: serde_json::Value = r.into_json().unwrap();
    assert!(body["added"].as_array().unwrap().is_empty());
    assert!(body["removed"].as_array().unwrap().is_empty());
    assert!(body["changed"].as_array().unwrap().is_empty());
}

#[test]
fn health_remains_open_when_tokens_configured() {
    // Health is intentionally always-open for monitoring, even with
    // tokens enabled. /op/<name> still requires auth (covered above).
    let d = DaemonHttp::spawn(Some("any-tok"));
    let resp = ureq::get(&d.url("/health")).call().expect("GET /health");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.into_json().expect("json");
    assert_eq!(body["ok"], serde_json::json!(true));
}
