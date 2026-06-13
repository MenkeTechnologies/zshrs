// Integration test for zcompdump byte-identical round-trip.

use serde_json::json;
use std::fs;
use zsh::daemon::client::Client;
use zsh::daemon::paths::CachePaths;

/// Helper to get the daemon binary path.
fn zshrs_daemon_binary() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs-daemon") {
        return std::path::PathBuf::from(p);
    }
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs-daemon")
}

struct DaemonHandle {
    _tmpdir: tempfile::TempDir,
    paths: CachePaths,
    child: Option<std::process::Child>,
}

impl DaemonHandle {
    fn spawn() -> Self {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let zshrs_home = tmp.path().to_path_buf();

        let child = std::process::Command::new(zshrs_daemon_binary())
            .env("ZSHRS_HOME", &zshrs_home)
            .env("ZSHRS_QUIET_FIRST_RUN", "1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("daemon spawn");

        let paths = CachePaths::with_root(zshrs_home);
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(3) {
            if paths.socket.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Self {
            _tmpdir: tmp,
            paths,
            child: Some(child),
        }
    }

    fn connect(&self) -> Client {
        Client::connect_existing(&self.paths).expect("connect")
    }
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Ok(mut c) = Client::connect_existing(&self.paths) {
                let _ = c.call("daemon", json!({ "verb": "stop" }));
            }
            let _ = child.wait();
        }
    }
}

#[test]
fn zcompdump_byte_identical_roundtrip() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    let original_content = "#files: 2\tversion: 5.9\n\n_comps=(\n'git' '_git'\n'ls' '_ls'\n)\n\n_services=(\n'ls' 'ls'\n)\n\nautoload -Uz _git _ls\n\ntypeset -gUa _comp_assocs\n_comp_assocs=( '' )\n";
    fs::write(tmp.path(), original_content).unwrap();

    // 1. Import
    c.call(
        "import_zcompdump",
        json!({ "path": tmp.path().to_str().unwrap() }),
    )
    .expect("import");

    // 2. Export
    let out_tmp = tempfile::NamedTempFile::new().unwrap();
    let resp = c
        .call(
            "export_zcompdump",
            json!({ "path": out_tmp.path().to_str().unwrap() }),
        )
        .expect("export");
    assert_eq!(resp["round_tripped"], json!(true));

    // 3. Compare bytes
    let exported_content = fs::read_to_string(out_tmp.path()).unwrap();
    assert_eq!(
        original_content, exported_content,
        "exported zcompdump must be byte-identical to imported one"
    );
}

#[test]
fn plugin_delta_sqlite_roundtrip() {
    use std::collections::HashMap;
    use zsh::plugin_cache::{AliasKind, PluginCache, PluginDelta};

    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("plugins.db");
    let cache = PluginCache::open(&db_path).unwrap();

    let mut delta = PluginDelta::default();
    delta
        .aliases
        .push(("ll".to_string(), "ls -la".to_string(), AliasKind::Regular));
    delta.variables.push(("VAR".to_string(), "val".to_string()));
    delta
        .exports
        .push(("EXPORT".to_string(), "eval".to_string()));

    let mut assoc = HashMap::new();
    assoc.insert("key1".to_string(), "val1".to_string());
    assoc.insert("key2".to_string(), "val2".to_string());
    delta.assoc_arrays.push(("MYASSOC".to_string(), assoc));

    delta
        .arrays
        .push(("MYARR".to_string(), vec!["a".to_string(), "b".to_string()]));
    delta.fpath_additions.push("/tmp/fpath".to_string());
    delta
        .completions
        .push(("git".to_string(), "_git".to_string()));
    delta.options_changed.push(("autocd".to_string(), true));
    delta
        .functions
        .push(("_myfunc".to_string(), vec![0xDE, 0xAD, 0xBE, 0xEF]));

    let path = "/fake/plugin.zsh";
    cache.store(path, 123, 456, 10, &delta).expect("store");

    let loaded = cache.load(1).expect("load");

    assert_eq!(loaded.aliases, delta.aliases);
    assert_eq!(loaded.variables, delta.variables);
    assert_eq!(loaded.exports, delta.exports);
    assert_eq!(loaded.arrays, delta.arrays);
    assert_eq!(loaded.assoc_arrays, delta.assoc_arrays);
    assert_eq!(loaded.fpath_additions, delta.fpath_additions);
    assert_eq!(loaded.completions, delta.completions);
    assert_eq!(loaded.options_changed, delta.options_changed);
    assert_eq!(loaded.functions, delta.functions);
}

#[test]
fn zcompdump_synthesize_format() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    // Push some canonical completion state.
    c.call(
        "push_canonical",
        json!({
            "subsystem": "compdef",
            "value": { "mycmd": "_mycmd", "othercmd": "_othercmd" }
        }),
    )
    .unwrap();

    c.call(
        "push_canonical",
        json!({
            "subsystem": "autoload_completion",
            "value": { "_mycmd": "", "_othercmd": "" }
        }),
    )
    .unwrap();

    let out_tmp = tempfile::NamedTempFile::new().unwrap();
    let resp = c
        .call(
            "export_zcompdump",
            json!({ "path": out_tmp.path().to_str().unwrap() }),
        )
        .expect("export");
    assert_eq!(
        resp["round_tripped"],
        json!(false),
        "should be synthesized since no raw import"
    );

    let exported_content = fs::read_to_string(out_tmp.path()).unwrap();
    assert!(exported_content.contains("_comps=("));
    assert!(exported_content.contains("'mycmd' '_mycmd'"));
    assert!(exported_content.contains("'othercmd' '_othercmd'"));
    assert!(exported_content.contains("autoload -Uz _mycmd _othercmd"));
    assert!(exported_content.contains("_comp_assocs="));
}

#[test]
fn zstyle_canonical_roundtrip() {
    let d = DaemonHandle::spawn();
    let mut c = d.connect();

    // 1. Push
    c.call(
        "push_canonical",
        json!({
            "subsystem": "zstyle",
            "value": {
                ":completion:*:*:git:*" : "list-colors 'red'",
                ":completion:*:*:ls:*" : "menu yes"
            }
        }),
    )
    .unwrap();

    // 2. Pull
    let resp = c
        .call("pull_canonical", json!({ "subsystem": "zstyle" }))
        .expect("pull");
    let rows = resp["rows"].as_array().unwrap();
    assert!(rows.len() >= 2);

    // 3. Export (eval-compat sh format)
    let resp = c
        .call("export", json!({ "target": "zstyle" }))
        .expect("export");
    let body = resp["body"].as_str().unwrap();

    assert!(body.contains("zstyle ':completion:*:*:git:*' list-colors 'red'"));
    assert!(body.contains("zstyle ':completion:*:*:ls:*' menu yes"));
}
