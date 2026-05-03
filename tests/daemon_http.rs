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
        Self::spawn_with_extra_toml(token, "")
    }

    /// Spawn variant that injects extra `[http.tokens]` lines beyond
    /// the optional default token. Lets scope tests configure scoped
    /// tokens without re-implementing the whole spawn dance.
    /// `extra_toml` is appended verbatim AFTER the default token line
    /// (or alone if `token` is None) and should already include the
    /// `[http.tokens.NAME]` headers it needs.
    fn spawn_with_extra_toml(token: Option<&str>, extra_toml: &str) -> Self {
        let cache = tempfile::TempDir::new().expect("cache tempdir");
        let config = tempfile::TempDir::new().expect("config tempdir");
        let port = pick_free_port();

        let cfg_dir = config.path().join("zshrs");
        std::fs::create_dir_all(&cfg_dir).expect("mk config dir");
        let mut f = std::fs::File::create(cfg_dir.join("daemon.toml")).expect("create toml");
        write!(f, "[http]\nlisten = \"127.0.0.1:{port}\"\n").unwrap();
        if let Some(tok) = token {
            write!(f, "\n[http.tokens]\ntest-tok = \"{tok}\"\n").unwrap();
        }
        if !extra_toml.is_empty() {
            write!(f, "\n{extra_toml}\n").unwrap();
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

#[test]
fn definitions_federation_keeps_per_shell_rows_distinct() {
    // Two shells emit `alias ll` with different bodies. Pre-federation
    // (composite-key) the second emit clobbered the first; with the
    // composite-key fix in canonical.rs both rows survive and the diff
    // op surfaces the conflict as `changed`.
    let d = DaemonHttp::spawn(None);

    // bash: ll = ls -al
    ureq::post(&d.url("/op/definitions_emit"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"shell_id":"bash","kind":"alias","name":"ll","value":"ls -al"}"#)
        .expect("emit bash ll");
    // zshrs: ll = ls -alh
    ureq::post(&d.url("/op/definitions_emit"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"shell_id":"zshrs","kind":"alias","name":"ll","value":"ls -alh"}"#)
        .expect("emit zshrs ll");
    // bash-only env to test diff `removed`
    ureq::post(&d.url("/op/definitions_emit"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"shell_id":"bash","kind":"env","name":"PAGER","value":"less"}"#)
        .expect("emit bash PAGER");

    // Query without filter: both alias rows present.
    let r = ureq::post(&d.url("/op/definitions_query"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"kind":"alias"}"#)
        .expect("query all");
    let body: serde_json::Value = r.into_json().unwrap();
    let recs = body["records"].as_array().unwrap();
    assert_eq!(recs.len(), 2, "expected both shells' ll rows: {body}");
    let shells: std::collections::HashSet<&str> = recs
        .iter()
        .filter_map(|r| r["shell_id"].as_str())
        .collect();
    assert!(shells.contains("bash"), "missing bash row: {body}");
    assert!(shells.contains("zshrs"), "missing zshrs row: {body}");

    // Filter to bash only.
    let r = ureq::post(&d.url("/op/definitions_query"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"kind":"alias","shell_id":"bash"}"#)
        .expect("query bash");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["count"], serde_json::json!(1));
    assert_eq!(body["records"][0]["shell_id"], serde_json::json!("bash"));
    assert_eq!(body["records"][0]["value"], serde_json::json!("ls -al"));

    // Diff bash vs zshrs: ll is `changed`, PAGER is `removed`.
    let r = ureq::post(&d.url("/op/definitions_diff"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"shell_a":"bash","shell_b":"zshrs"}"#)
        .expect("diff");
    let body: serde_json::Value = r.into_json().unwrap();
    let changed = body["changed"].as_array().unwrap();
    assert!(
        changed.iter().any(|c| c["name"] == "ll" && c["from"] == "ls -al" && c["to"] == "ls -alh"),
        "expected ll changed entry: {body}"
    );
    let removed = body["removed"].as_array().unwrap();
    assert!(
        removed.iter().any(|r| r["name"] == "PAGER"),
        "expected PAGER removed (only in bash): {body}"
    );
}

#[test]
fn definitions_emit_rejects_missing_shell_id() {
    let d = DaemonHttp::spawn(None);
    let resp = ureq::post(&d.url("/op/definitions_emit"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"kind":"alias","name":"ll","value":"ls"}"#);
    let err = resp.expect_err("expected 400 missing shell_id");
    let status = match err {
        ureq::Error::Status(s, _) => s,
        ureq::Error::Transport(t) => panic!("transport error: {t}"),
    };
    assert_eq!(status, 400);
}

#[test]
fn definitions_emit_rejects_unknown_kind() {
    let d = DaemonHttp::spawn(None);
    let resp = ureq::post(&d.url("/op/definitions_emit"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"shell_id":"bash","kind":"banana","name":"x"}"#);
    let err = resp.expect_err("expected 404 unknown kind");
    let status = match err {
        ureq::Error::Status(s, _) => s,
        ureq::Error::Transport(t) => panic!("transport error: {t}"),
    };
    assert_eq!(status, 404);
}

#[test]
fn definitions_subscribe_unsubscribe_round_trip() {
    // Pin the subscribe/unsubscribe op surface — flag flips on then
    // off, idempotent. Per-request HTTP sessions, so each call gets a
    // fresh client_id; we verify the op succeeds and returns the
    // expected shape, not cross-call state (cross-session state would
    // require a long-lived IPC client, out of scope for HTTP tests).
    let d = DaemonHttp::spawn(None);

    let r = ureq::post(&d.url("/op/definitions_subscribe"))
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("subscribe");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["subscribed"], serde_json::json!(true));
    assert_eq!(body["was_subscribed"], serde_json::json!(false));

    let r = ureq::post(&d.url("/op/definitions_unsubscribe"))
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("unsubscribe");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["subscribed"], serde_json::json!(false));
}

#[test]
fn watch_subscribe_returns_id_and_lists() {
    // Pin watch_subscribe → watch_id → watch_list visibility →
    // watch_unsubscribe removes. Refcounting verified by subscribing
    // the same path twice and asserting unsubscribe of one keeps the
    // other live.
    let d = DaemonHttp::spawn(None);
    let tmp = tempfile::TempDir::new().expect("tempdir for watch");
    let path = tmp.path().to_str().unwrap().to_string();

    // Two subscriptions on the same path.
    let r1 = ureq::post(&d.url("/op/watch_subscribe"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"path":"{path}"}}"#))
        .expect("watch_subscribe 1");
    let b1: serde_json::Value = r1.into_json().unwrap();
    let id1 = b1["watch_id"].as_u64().expect("watch_id u64");
    assert!(id1 > 0);

    let r2 = ureq::post(&d.url("/op/watch_subscribe"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"path":"{path}","recursive":true}}"#))
        .expect("watch_subscribe 2");
    let b2: serde_json::Value = r2.into_json().unwrap();
    let id2 = b2["watch_id"].as_u64().expect("watch_id u64");
    assert_ne!(id1, id2, "ids must be distinct");

    // List shows both with refcount=2 (same path).
    let r = ureq::post(&d.url("/op/watch_list"))
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("watch_list");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["count"], serde_json::json!(2));
    let subs = body["subscriptions"].as_array().unwrap();
    assert!(subs.iter().all(|s| s["ref_count"] == serde_json::json!(2)));

    // Unsubscribe one — other subscription survives, refcount drops to 1.
    let r = ureq::post(&d.url("/op/watch_unsubscribe"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"watch_id":{id1}}}"#))
        .expect("watch_unsubscribe 1");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["removed"], serde_json::json!(true));

    let r = ureq::post(&d.url("/op/watch_list"))
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("watch_list 2");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["count"], serde_json::json!(1));
    assert_eq!(body["subscriptions"][0]["ref_count"], serde_json::json!(1));

    // Final unsubscribe — list goes empty.
    let _ = ureq::post(&d.url("/op/watch_unsubscribe"))
        .set("Content-Type", "application/json")
        .send_string(&format!(r#"{{"watch_id":{id2}}}"#))
        .expect("watch_unsubscribe 2");
    let r = ureq::post(&d.url("/op/watch_list"))
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("watch_list 3");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["count"], serde_json::json!(0));
}

#[test]
fn watch_unsubscribe_unknown_id_is_idempotent() {
    let d = DaemonHttp::spawn(None);
    let r = ureq::post(&d.url("/op/watch_unsubscribe"))
        .set("Content-Type", "application/json")
        .send_string(r#"{"watch_id":999999}"#)
        .expect("watch_unsubscribe missing id");
    let body: serde_json::Value = r.into_json().unwrap();
    assert_eq!(body["removed"], serde_json::json!(false));
}

// ---- Per-token scope authorization (audit item #9) ------------------

/// Pull the HTTP status from a ureq error, panicking on transport
/// failures (which would mask scope-test bugs as test infrastructure
/// problems).
fn status_of(err: ureq::Error) -> u16 {
    match err {
        ureq::Error::Status(s, _) => s,
        ureq::Error::Transport(t) => panic!("transport error: {t}"),
    }
}

#[test]
fn legacy_unscoped_token_grants_full_access() {
    // `name = "secret"` flat string form. Pre-scope-feature configs
    // must keep working unchanged: any op the legacy token presents
    // is allowed.
    let d = DaemonHttp::spawn(Some("legacy-secret"));

    // Touch ops from multiple scope namespaces.
    for op in ["info", "cache_stats", "definitions_kinds", "snapshot_list"] {
        let r = ureq::post(&d.url(&format!("/op/{op}")))
            .set("Authorization", "Bearer legacy-secret")
            .set("Content-Type", "application/json")
            .send_string("{}")
            .unwrap_or_else(|e| panic!("{op}: {e}"));
        assert_eq!(r.status(), 200, "{op}");
    }
}

#[test]
fn scoped_token_allows_listed_scope_only() {
    // Scoped token may only `cache.*`. Other namespaces → 403
    // scope_denied. Verifies the table-form parser AND the dispatcher
    // scope check together.
    let extra = r#"
[http.tokens.cache-only]
token = "cache-secret"
scopes = ["cache.*"]
"#;
    let d = DaemonHttp::spawn_with_extra_toml(None, extra);

    // cache_stats is `cache.read` → allowed.
    let r = ureq::post(&d.url("/op/cache_stats"))
        .set("Authorization", "Bearer cache-secret")
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect("cache_stats");
    assert_eq!(r.status(), 200);

    // snapshot_save is `snapshot.write` → denied.
    let err = ureq::post(&d.url("/op/snapshot_save"))
        .set("Authorization", "Bearer cache-secret")
        .set("Content-Type", "application/json")
        .send_string(r#"{"tag":"x"}"#)
        .expect_err("snapshot_save must 403");
    assert_eq!(status_of(err), 403);
}

#[test]
fn scope_denied_response_carries_required_and_granted() {
    let extra = r#"
[http.tokens.read-only]
token = "ro-secret"
scopes = ["*.read"]
"#;
    let d = DaemonHttp::spawn_with_extra_toml(None, extra);

    // cache_put is `cache.write` → denied for *.read token.
    let err = ureq::post(&d.url("/op/cache_put"))
        .set("Authorization", "Bearer ro-secret")
        .set("Content-Type", "application/json")
        .send_string(r#"{"ns":"a","key":"b","value":"c"}"#)
        .expect_err("cache_put must 403");
    let (status, resp) = match err {
        ureq::Error::Status(s, r) => (s, r),
        ureq::Error::Transport(t) => panic!("transport error: {t}"),
    };
    assert_eq!(status, 403);
    let body: serde_json::Value = resp.into_json().unwrap();
    assert_eq!(body["code"], serde_json::json!("scope_denied"));
    assert_eq!(body["required_scope"], serde_json::json!("cache.write"));
    let granted = body["granted_scopes"].as_array().unwrap();
    assert!(granted.iter().any(|v| v == "*.read"));
}

#[test]
fn verb_wildcard_grants_read_across_areas() {
    // `*.read` should match every `<area>.read` op AND `defs.read`,
    // `cache.read`, `snapshot.read`, etc.
    let extra = r#"
[http.tokens.dashboard]
token = "dash-secret"
scopes = ["*.read"]
"#;
    let d = DaemonHttp::spawn_with_extra_toml(None, extra);

    for op in ["cache_stats", "definitions_kinds", "snapshot_list", "lock_list"] {
        let r = ureq::post(&d.url(&format!("/op/{op}")))
            .set("Authorization", "Bearer dash-secret")
            .set("Content-Type", "application/json")
            .send_string("{}")
            .unwrap_or_else(|e| panic!("{op}: {e}"));
        assert_eq!(r.status(), 200, "{op} (scope = {})", auth_scope(op));
    }
}

#[test]
fn unknown_op_falls_through_to_meta_admin_scope() {
    // Unmapped ops in auth.rs:op_scope return `meta.admin` so a
    // tightly-scoped token can't smuggle calls to ops the table
    // doesn't know about. The test op itself doesn't exist so we
    // expect 403 (scope_denied) FIRST, before the dispatcher's
    // unknown-op 404 has a chance to fire.
    let extra = r#"
[http.tokens.cache-only]
token = "co-secret"
scopes = ["cache.read"]
"#;
    let d = DaemonHttp::spawn_with_extra_toml(None, extra);

    let err = ureq::post(&d.url("/op/zzz_definitely_not_a_real_op"))
        .set("Authorization", "Bearer co-secret")
        .set("Content-Type", "application/json")
        .send_string("{}")
        .expect_err("must reject");
    let (status, resp) = match err {
        ureq::Error::Status(s, r) => (s, r),
        ureq::Error::Transport(t) => panic!("transport error: {t}"),
    };
    assert_eq!(status, 403, "scope check fires before unknown-op 404");
    let body: serde_json::Value = resp.into_json().unwrap();
    assert_eq!(body["required_scope"], serde_json::json!("meta.admin"));
}

// Helper used by the verb-wildcard test for clearer panic messages —
// surfaces what op→scope mapping was being asserted when an
// allowed-but-failed op was rejected.
fn auth_scope(op: &str) -> &'static str {
    zsh::daemon::auth::op_scope(op)
}
