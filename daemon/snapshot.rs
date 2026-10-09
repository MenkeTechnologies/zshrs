//! `daemon.snapshot.*` ops — portable canonical-state snapshots.
//!
//! Per docs/DAEMON_AS_SERVICE.md: "save: capture state via the
//! recorder; load: atomic swap; diff: structured per-record diff;
//! bisect: first diverging record".
//!
//! Implementation uses the existing rkyv `CanonicalShard` as the
//! on-disk format — same scheme as the recorder bundle, so snapshots
//! are byte-identical to what `recorder_ingest` produces. Tag-based
//! naming: `~/.zshrs/snapshots/<tag>.rkyv`. Tag is any
//! shell-safe string (matched against `[A-Za-z0-9._-]+` at op time).
//!
//! Signing: a per-user ed25519 key is generated on first use at
//! `<state root>/snapshot-signing.key` (mode 0600, hex seed). `snapshot_sign`
//! writes a detached signature `<tag>.rkyv.sig` over the exact snapshot
//! bytes; `snapshot_verify` checks it against a supplied or the local
//! public key; `snapshot_publish` verifies, then writes snapshot +
//! signature + `manifest.json` to a registry (a directory, `file://`
//! URL, or `http(s)://` URL via PUT). The registry comes from the
//! `registry` arg or `[snapshot] registry` in `zshrs-daemon.toml`
//! (optional `registry_token` is sent as a bearer token on HTTP).
//!
//! Op surface:
//!
//! | Op                | Args              | Returns                                    |
//! |-------------------|-------------------|--------------------------------------------|
//! | `snapshot_save`   | `{tag, notes?}`   | `{ok, tag, path, bytes, generation}`       |
//! | `snapshot_list`   | `{}`              | `{ok, snapshots: [...], count}`            |
//! | `snapshot_load`   | `{tag}`           | `{ok, tag, rows_restored}` (atomic swap)   |
//! | `snapshot_diff`   | `{a, b}`          | `{ok, added, removed, changed}`            |
//! | `snapshot_pubkey` | `{}`              | `{public_key, algorithm, key_path}`        |
//! | `snapshot_sign`   | `{tag}`           | `{tag, sha256, bytes, public_key, sig_path}` |
//! | `snapshot_verify` | `{tag, public_key?, registry?}` | `{ok, tag, sha256, public_key, trusted}` or error `snapshot_verify_failed` |
//! | `snapshot_publish`| `{tag, registry?}` | `{tag, registry, sha256, files}`          |

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;

/// All canonical subsystems the snapshot covers. Folded into the
/// rkyv `CanonicalShard` on save and replayed on load.
const SUBSYSTEMS: &[&str] = &[
    "alias",
    "galias",
    "salias",
    "function",
    "function_autoload",
    "env",
    "params",
    "params_typed",
    "bindkey",
    "compdef",
    "named_dir",
    "zstyle",
    "zmodload",
    "setopt",
    "trap",
    "sched",
    "source",
    "zle",
    "completion",
    "path",
    "fpath",
    "manpath",
];

fn tag_arg(args: &Value) -> std::result::Result<String, ErrPayload> {
    let tag = args
        .get("tag")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `tag`"))?;
    if tag.is_empty()
        || tag.starts_with('.')
        || !tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ErrPayload::new(
            "bad_args",
            "tag must match [A-Za-z0-9._-]+ and not start with `.`",
        ));
    }
    Ok(tag.to_string())
}

fn snapshot_path(state: &DaemonState, tag: &str) -> PathBuf {
    state.paths.snapshots_dir.join(format!("{tag}.rkyv"))
}
/// `op_snapshot_save` — see implementation.
pub async fn op_snapshot_save(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let tag = tag_arg(&args)?;
    let _notes = args.get("notes").and_then(Value::as_str);
    let _ = std::fs::create_dir_all(&state.paths.snapshots_dir);

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let mut shard = state.canonical.snapshot_shard(now);
    // Override slug so the snapshot file lives under our snapshots/ dir
    // (write_canonical_shard composes the filename from slug+source_root,
    // not from a free-form name).
    shard.header.slug = format!("snapshot-{tag}");
    shard.header.source_root = format!("snapshot:{tag}");

    let bytes_before = if let Ok(meta) = std::fs::metadata(snapshot_path(state, &tag)) {
        meta.len()
    } else {
        0
    };

    // write_canonical_shard places the file under paths.images/, not
    // snapshots/. We want snapshots in their own dir for clarity, so
    // serialise + write directly.
    let bytes = rkyv::to_bytes::<_, 4096>(&shard)
        .map_err(|e| ErrPayload::new("snapshot_serialize", e.to_string()))?;
    let dest = snapshot_path(state, &tag);
    let tmp = dest.with_extension("rkyv.tmp");
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| ErrPayload::new("snapshot_write", e.to_string()))?;
    f.write_all(&bytes)
        .map_err(|e| ErrPayload::new("snapshot_write", e.to_string()))?;
    f.sync_all()
        .map_err(|e| ErrPayload::new("snapshot_write", e.to_string()))?;
    drop(f);
    std::fs::rename(&tmp, &dest).map_err(|e| ErrPayload::new("snapshot_rename", e.to_string()))?;

    let total_rows: i64 = SUBSYSTEMS
        .iter()
        .map(|s| state.canonical.rows_for_all_shells(s).len() as i64)
        .sum();

    Ok(json!({
        "tag": tag,
        "path": dest.display().to_string(),
        "bytes": bytes.len(),
        "bytes_prev": bytes_before,
        "generation": now,
        "total_rows": total_rows,
    }))
}
/// `op_snapshot_list` — see implementation.
pub async fn op_snapshot_list(state: &Arc<DaemonState>, _args: Value) -> OpResult {
    let mut entries: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&state.paths.snapshots_dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            let name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            let tag = match name.strip_suffix(".rkyv") {
                Some(t) => t,
                None => continue,
            };
            let meta = match ent.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            entries.push(json!({
                "tag": tag,
                "path": p.display().to_string(),
                "bytes": meta.len(),
                "modified_secs": meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
            }));
        }
    }
    entries.sort_by(|a, b| {
        a["tag"]
            .as_str()
            .unwrap_or("")
            .cmp(b["tag"].as_str().unwrap_or(""))
    });
    let count = entries.len();
    Ok(json!({ "snapshots": entries, "count": count }))
}
/// `op_snapshot_load` — see implementation.
pub async fn op_snapshot_load(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let tag = tag_arg(&args)?;
    let dest = snapshot_path(state, &tag);
    if !dest.exists() {
        return Err(ErrPayload::new(
            "no_such_file",
            format!("snapshot `{tag}` not found"),
        ));
    }
    let shard = super::shard::read_canonical_shard(&dest)
        .map_err(|e| ErrPayload::new("snapshot_read", e.to_string()))?;

    let mut rows_restored: usize = 0;
    let canon = &state.canonical;
    rows_restored += canon.replace_subsystem(
        "alias",
        shard.aliases.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "galias",
        shard
            .global_aliases
            .iter()
            .map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "salias",
        shard
            .suffix_aliases
            .iter()
            .map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "function",
        shard
            .functions
            .iter()
            .map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "env",
        shard
            .env_exports
            .iter()
            .map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "params",
        shard.params.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "bindkey",
        shard.bindkeys.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "compdef",
        shard.compdef.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "named_dir",
        shard
            .named_dirs
            .iter()
            .map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "zstyle",
        shard
            .zstyle
            .iter()
            .enumerate()
            .map(|(i, (p, r))| (format!("{}:{}", i, p), json_str(r))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "zmodload",
        shard.zmodload.iter().map(|m| (m.clone(), json_str(""))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "setopt",
        shard
            .setopts
            .iter()
            .map(|o| (o.clone(), "\"on\"".to_string()))
            .chain(
                shard
                    .unsetopts
                    .iter()
                    .map(|o| (o.clone(), "\"off\"".to_string())),
            ),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "source",
        shard
            .sourced_files
            .iter()
            .enumerate()
            .map(|(i, p)| (i.to_string(), json_str(p))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "path",
        shard
            .path
            .iter()
            .enumerate()
            .map(|(i, d)| (i.to_string(), json_str(d))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "fpath",
        shard
            .fpath
            .iter()
            .enumerate()
            .map(|(i, d)| (i.to_string(), json_str(d))),
        None,
    );
    // Extras (zle widgets, completions, params_typed) — fold each
    // sub-bucket into its named subsystem.
    for (subsystem, table) in &shard.extras {
        rows_restored += canon.replace_subsystem(
            subsystem,
            table.iter().map(|(k, v)| (k.clone(), json_str(v))),
            None,
        );
    }
    Ok(json!({
        "tag": tag,
        "rows_restored": rows_restored,
        "generation": shard.header.generation,
    }))
}
/// `op_snapshot_diff` — see implementation.
pub async fn op_snapshot_diff(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let a_tag = args
        .get("a")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `a`"))?
        .to_string();
    let b_tag = args
        .get("b")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `b`"))?
        .to_string();
    let a_path = snapshot_path(state, &a_tag);
    let b_path = snapshot_path(state, &b_tag);
    let a = super::shard::read_canonical_shard(&a_path)
        .map_err(|e| ErrPayload::new("snapshot_read", format!("read `{a_tag}`: {e}")))?;
    let b = super::shard::read_canonical_shard(&b_path)
        .map_err(|e| ErrPayload::new("snapshot_read", format!("read `{b_tag}`: {e}")))?;

    let mut added: Vec<Value> = Vec::new();
    let mut removed: Vec<Value> = Vec::new();
    let mut changed: Vec<Value> = Vec::new();
    let pairs: &[(&_, &_, &str)] = &[
        (&a.aliases, &b.aliases, "alias"),
        (&a.global_aliases, &b.global_aliases, "galias"),
        (&a.suffix_aliases, &b.suffix_aliases, "salias"),
        (&a.functions, &b.functions, "function"),
        (&a.env_exports, &b.env_exports, "env"),
        (&a.params, &b.params, "params"),
        (&a.bindkeys, &b.bindkeys, "bindkey"),
        (&a.compdef, &b.compdef, "compdef"),
        (&a.named_dirs, &b.named_dirs, "named_dir"),
    ];
    for (am, bm, kind) in pairs {
        added.extend(diff_added(am, bm, kind));
        removed.extend(diff_removed(am, bm, kind));
        changed.extend(diff_changed(am, bm, kind));
    }
    Ok(json!({
        "a": a_tag,
        "b": b_tag,
        "added": added,
        "removed": removed,
        "changed": changed,
    }))
}

fn diff_added(
    a: &std::collections::HashMap<String, String>,
    b: &std::collections::HashMap<String, String>,
    kind: &str,
) -> Vec<Value> {
    b.iter()
        .filter(|(k, _)| !a.contains_key(*k))
        .map(|(k, v)| json!({"kind": kind, "name": k, "value": v}))
        .collect()
}

fn diff_removed(
    a: &std::collections::HashMap<String, String>,
    b: &std::collections::HashMap<String, String>,
    kind: &str,
) -> Vec<Value> {
    a.iter()
        .filter(|(k, _)| !b.contains_key(*k))
        .map(|(k, v)| json!({"kind": kind, "name": k, "value": v}))
        .collect()
}

fn diff_changed(
    a: &std::collections::HashMap<String, String>,
    b: &std::collections::HashMap<String, String>,
    kind: &str,
) -> Vec<Value> {
    a.iter()
        .filter_map(|(k, av)| {
            b.get(k).and_then(|bv| {
                if av != bv {
                    Some(json!({"kind": kind, "name": k, "from": av, "to": bv}))
                } else {
                    None
                }
            })
        })
        .collect()
}

fn json_str(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

// ---------------------------------------------------------------------------
// Signing, verification, and registry publish.
// ---------------------------------------------------------------------------

use std::io::Read;
use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Signing-key file under the state root. Hex-encoded 32-byte ed25519 seed.
const KEY_FILE: &str = "snapshot-signing.key";
/// Upper bound on a snapshot fetched from an HTTP registry.
const MAX_REMOTE_BYTES: u64 = 1 << 30;
/// Registry file names, identical for directory and HTTP registries.
const REG_SNAPSHOT: &str = "snapshot.rkyv";
const REG_SIGNATURE: &str = "snapshot.rkyv.sig";
const REG_MANIFEST: &str = "manifest.json";

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn sha256_hex(data: &[u8]) -> String {
    hex_encode(&Sha256::digest(data))
}

/// Write `data` to `dest` via a sibling temp file + rename.
fn atomic_write(dest: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let name = dest.file_name().and_then(|n| n.to_str()).unwrap_or("out");
    let tmp = dest.with_file_name(format!(".{name}.tmp.{}", std::process::id()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, dest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn sig_path(state: &DaemonState, tag: &str) -> PathBuf {
    state.paths.snapshots_dir.join(format!("{tag}.rkyv.sig"))
}

fn key_path(state: &DaemonState) -> PathBuf {
    state.paths.root.join(KEY_FILE)
}

fn create_key_file(path: &Path) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(format!("{}\n", hex_encode(&seed)).as_bytes())?;
    f.sync_all()?;
    drop(f);
    // hard_link fails if a concurrent creator won; its key stays.
    let linked = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => Err(e),
        _ => Ok(()),
    }
}

/// Load the per-user signing key, generating it on first use. Refuses a
/// key file readable by group/other.
fn load_or_create_key(path: &Path) -> Result<SigningKey, ErrPayload> {
    use std::os::unix::fs::PermissionsExt;
    let key_err = |m: String| ErrPayload::new("snapshot_key", m);
    if std::fs::symlink_metadata(path).is_err() {
        create_key_file(path).map_err(|e| key_err(format!("create {}: {e}", path.display())))?;
    }
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| key_err(format!("stat {}: {e}", path.display())))?;
    let mode = meta.permissions().mode();
    if !meta.is_file() || mode & 0o077 != 0 {
        return Err(key_err(format!(
            "{} must be a regular file with mode 0600 (is {:o})",
            path.display(),
            mode & 0o7777
        )));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| key_err(format!("read {}: {e}", path.display())))?;
    let seed: [u8; 32] = hex_decode(&text)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| key_err(format!("{} is not a 32-byte hex seed", path.display())))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn parse_public_key(hex: &str) -> Result<VerifyingKey, String> {
    let bytes: [u8; 32] = hex_decode(hex)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| "public key must be 64 hex characters".to_string())?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| format!("invalid public key: {e}"))
}

/// Detached-signature document stored beside the snapshot.
fn make_sig_doc(key: &SigningKey, tag: &str, data: &[u8]) -> Value {
    let sig = key.sign(data);
    json!({
        "version": 1,
        "alg": "ed25519",
        "tag": tag,
        "sha256": sha256_hex(data),
        "bytes": data.len(),
        "public_key": hex_encode(key.verifying_key().as_bytes()),
        "signature": hex_encode(&sig.to_bytes()),
        "signed_at": chrono::Utc::now().to_rfc3339(),
    })
}

fn doc_str<'a>(doc: &'a Value, field: &str) -> Result<&'a str, String> {
    doc.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("signature document missing `{field}`"))
}

/// Check `data` against a signature document and a trusted public key.
fn verify_sig_doc(data: &[u8], doc: &Value, trusted: &VerifyingKey) -> Result<(), String> {
    if doc_str(doc, "alg")? != "ed25519" {
        return Err("unsupported signature algorithm".into());
    }
    if sha256_hex(data) != doc_str(doc, "sha256")? {
        return Err("sha256 mismatch: snapshot bytes differ from the signed digest".into());
    }
    let signer = hex_decode(doc_str(doc, "public_key")?)
        .ok_or_else(|| "signature document public_key is not hex".to_string())?;
    if signer.as_slice() != trusted.as_bytes() {
        return Err("signed by a different key than the trusted key".into());
    }
    let sig_bytes: [u8; 64] = hex_decode(doc_str(doc, "signature")?)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| "signature must be 128 hex characters".to_string())?;
    trusted
        .verify_strict(data, &Signature::from_bytes(&sig_bytes))
        .map_err(|e| format!("signature invalid: {e}"))
}

/// Where `snapshot_publish` writes to / `snapshot_verify` reads from.
enum Registry {
    Dir(PathBuf),
    Http { base: String, token: Option<String> },
}

impl Registry {
    fn label(&self) -> String {
        match self {
            Registry::Dir(p) => p.display().to_string(),
            Registry::Http { base, .. } => base.clone(),
        }
    }
}

fn parse_registry(spec: &str, token: Option<String>) -> Result<Registry, ErrPayload> {
    if spec.starts_with("http://") || spec.starts_with("https://") {
        return Ok(Registry::Http {
            base: spec.trim_end_matches('/').to_string(),
            token,
        });
    }
    let dir = PathBuf::from(spec.strip_prefix("file://").unwrap_or(spec));
    if !dir.is_absolute() {
        return Err(ErrPayload::new(
            "bad_args",
            "registry must be an absolute directory, file:// URL, or http(s):// URL",
        ));
    }
    Ok(Registry::Dir(dir))
}

/// `registry` arg wins; otherwise `[snapshot] registry` (and
/// `registry_token`) from `zshrs-daemon.toml`.
fn resolve_registry(state: &DaemonState, args: &Value) -> Result<Option<Registry>, ErrPayload> {
    let section = std::fs::read_to_string(state.paths.daemon_config_path())
        .ok()
        .and_then(|body| body.parse::<toml::Table>().ok())
        .and_then(|t| t.get("snapshot").cloned());
    let cfg = |k: &str| {
        section
            .as_ref()
            .and_then(|s| s.get(k))
            .and_then(toml::Value::as_str)
            .map(str::to_string)
    };
    let spec = args
        .get("registry")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| cfg("registry"));
    spec.map(|s| parse_registry(&s, cfg("registry_token")))
        .transpose()
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(60))
        .build()
}

fn http_put(url: &str, token: Option<&str>, body: &[u8], ctype: &str) -> Result<(), String> {
    let mut req = http_agent().put(url).set("Content-Type", ctype);
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    req.send_bytes(body)
        .map(|_| ())
        .map_err(|e| format!("PUT {url}: {e}"))
}

fn http_get(url: &str, token: Option<&str>) -> Result<Vec<u8>, String> {
    let mut req = http_agent().get(url);
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let resp = req.call().map_err(|e| format!("GET {url}: {e}"))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(MAX_REMOTE_BYTES)
        .read_to_end(&mut buf)
        .map_err(|e| format!("GET {url}: {e}"))?;
    Ok(buf)
}

/// Write snapshot, signature, and manifest (last) to the registry.
fn registry_put(
    reg: &Registry,
    tag: &str,
    snapshot: &[u8],
    sig: &Value,
    manifest: &Value,
) -> Result<(), String> {
    let sig_bytes = serde_json::to_vec_pretty(sig).map_err(|e| e.to_string())?;
    let man_bytes = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    match reg {
        Registry::Dir(root) => {
            let dir = root.join(tag);
            std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
            for (name, bytes) in [
                (REG_SNAPSHOT, snapshot),
                (REG_SIGNATURE, sig_bytes.as_slice()),
                (REG_MANIFEST, man_bytes.as_slice()),
            ] {
                atomic_write(&dir.join(name), bytes, 0o644)
                    .map_err(|e| format!("write {}: {e}", dir.join(name).display()))?;
            }
            Ok(())
        }
        Registry::Http { base, token } => {
            let t = token.as_deref();
            http_put(&format!("{base}/{tag}/{REG_SNAPSHOT}"), t, snapshot, "application/octet-stream")?;
            http_put(&format!("{base}/{tag}/{REG_SIGNATURE}"), t, &sig_bytes, "application/json")?;
            http_put(&format!("{base}/{tag}/{REG_MANIFEST}"), t, &man_bytes, "application/json")
        }
    }
}

/// Fetch a published snapshot and its signature document.
fn registry_get(reg: &Registry, tag: &str) -> Result<(Vec<u8>, Value), String> {
    let (snap, sig) = match reg {
        Registry::Dir(root) => {
            let dir = root.join(tag);
            let rd = |n: &str| {
                std::fs::read(dir.join(n)).map_err(|e| format!("read {}: {e}", dir.join(n).display()))
            };
            (rd(REG_SNAPSHOT)?, rd(REG_SIGNATURE)?)
        }
        Registry::Http { base, token } => {
            let t = token.as_deref();
            (
                http_get(&format!("{base}/{tag}/{REG_SNAPSHOT}"), t)?,
                http_get(&format!("{base}/{tag}/{REG_SIGNATURE}"), t)?,
            )
        }
    };
    let doc = serde_json::from_slice(&sig).map_err(|e| format!("signature document: {e}"))?;
    Ok((snap, doc))
}

fn read_local_snapshot(state: &DaemonState, tag: &str) -> Result<Vec<u8>, ErrPayload> {
    let path = snapshot_path(state, tag);
    std::fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ErrPayload::new("no_such_file", format!("snapshot `{tag}` not found"))
        } else {
            ErrPayload::new("snapshot_read", format!("{}: {e}", path.display()))
        }
    })
}

fn join_err(e: tokio::task::JoinError) -> ErrPayload {
    ErrPayload::new("internal", e.to_string())
}

/// `op_snapshot_pubkey` — export the local signing public key
/// (generating the key on first use).
pub async fn op_snapshot_pubkey(state: &Arc<DaemonState>, _args: Value) -> OpResult {
    let path = key_path(state);
    let key = load_or_create_key(&path)?;
    Ok(json!({
        "algorithm": "ed25519",
        "public_key": hex_encode(key.verifying_key().as_bytes()),
        "key_path": path.display().to_string(),
    }))
}

/// `op_snapshot_sign` — write `<tag>.rkyv.sig` beside the snapshot.
pub async fn op_snapshot_sign(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let tag = tag_arg(&args)?;
    let data = read_local_snapshot(state, &tag)?;
    let key = load_or_create_key(&key_path(state))?;
    let doc = make_sig_doc(&key, &tag, &data);
    let dest = sig_path(state, &tag);
    let body = serde_json::to_vec_pretty(&doc).map_err(|e| ErrPayload::new("snapshot_sign", e.to_string()))?;
    atomic_write(&dest, &body, 0o600).map_err(|e| ErrPayload::new("snapshot_write", e.to_string()))?;
    Ok(json!({
        "tag": tag,
        "sha256": doc["sha256"],
        "bytes": data.len(),
        "public_key": doc["public_key"],
        "sig_path": dest.display().to_string(),
    }))
}

/// `op_snapshot_verify` — verify the local snapshot (or, with
/// `registry`, its published copy) against `public_key` or the local key.
/// A mismatch or tamper is the error `snapshot_verify_failed`.
pub async fn op_snapshot_verify(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let tag = tag_arg(&args)?;
    let (trusted, trusted_src) = match args.get("public_key").and_then(Value::as_str) {
        Some(hex) => (
            parse_public_key(hex).map_err(|m| ErrPayload::new("bad_args", m))?,
            "supplied",
        ),
        None => {
            let path = key_path(state);
            if std::fs::symlink_metadata(&path).is_err() {
                return Err(ErrPayload::new(
                    "snapshot_key",
                    "no local signing key and no `public_key` supplied",
                ));
            }
            (load_or_create_key(&path)?.verifying_key(), "local")
        }
    };
    let registry = resolve_registry_if_requested(state, &args)?;
    let (data, doc) = match registry {
        Some(reg) => {
            let t = tag.clone();
            tokio::task::spawn_blocking(move || registry_get(&reg, &t))
                .await
                .map_err(join_err)?
                .map_err(|m| ErrPayload::new("snapshot_read", m))?
        }
        None => {
            let data = read_local_snapshot(state, &tag)?;
            let sp = sig_path(state, &tag);
            let raw = std::fs::read(&sp)
                .map_err(|e| ErrPayload::new("snapshot_read", format!("{}: {e}", sp.display())))?;
            let doc = serde_json::from_slice(&raw)
                .map_err(|e| ErrPayload::new("snapshot_read", format!("signature document: {e}")))?;
            (data, doc)
        }
    };
    verify_sig_doc(&data, &doc, &trusted).map_err(|m| {
        ErrPayload::new("snapshot_verify_failed", format!("snapshot `{tag}`: {m}"))
    })?;
    Ok(json!({
        "ok": true,
        "tag": tag,
        "sha256": sha256_hex(&data),
        "public_key": hex_encode(trusted.as_bytes()),
        "trusted": trusted_src,
    }))
}

/// Verify reads a registry only when the caller names one explicitly;
/// the configured default is a publish target, not an implicit source.
fn resolve_registry_if_requested(
    state: &DaemonState,
    args: &Value,
) -> Result<Option<Registry>, ErrPayload> {
    if args.get("registry").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
        resolve_registry(state, args)
    } else {
        Ok(None)
    }
}

/// `op_snapshot_publish` — verify, then write snapshot + signature +
/// manifest to the registry. Signs first when no signature exists; a
/// present-but-invalid signature aborts (re-run `snapshot_sign`).
pub async fn op_snapshot_publish(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let tag = tag_arg(&args)?;
    let reg = resolve_registry(state, &args)?.ok_or_else(|| {
        ErrPayload::new(
            "no_registry",
            "no `registry` arg and no [snapshot] registry in zshrs-daemon.toml",
        )
    })?;
    let data = read_local_snapshot(state, &tag)?;
    let key = load_or_create_key(&key_path(state))?;
    let sp = sig_path(state, &tag);
    let doc = match std::fs::read(&sp) {
        Ok(raw) => serde_json::from_slice::<Value>(&raw)
            .map_err(|e| ErrPayload::new("snapshot_read", format!("signature document: {e}")))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let doc = make_sig_doc(&key, &tag, &data);
            let body = serde_json::to_vec_pretty(&doc)
                .map_err(|e| ErrPayload::new("snapshot_sign", e.to_string()))?;
            atomic_write(&sp, &body, 0o600)
                .map_err(|e| ErrPayload::new("snapshot_write", e.to_string()))?;
            doc
        }
        Err(e) => return Err(ErrPayload::new("snapshot_read", format!("{}: {e}", sp.display()))),
    };
    verify_sig_doc(&data, &doc, &key.verifying_key()).map_err(|m| {
        ErrPayload::new(
            "snapshot_verify_failed",
            format!("refusing to publish `{tag}`: {m}"),
        )
    })?;
    let sha = sha256_hex(&data);
    let manifest = json!({
        "version": 1,
        "tag": tag,
        "sha256": sha,
        "bytes": data.len(),
        "public_key": doc["public_key"],
        "published_at": chrono::Utc::now().to_rfc3339(),
        "files": { "snapshot": REG_SNAPSHOT, "signature": REG_SIGNATURE },
    });
    let label = reg.label();
    let t = tag.clone();
    tokio::task::spawn_blocking(move || registry_put(&reg, &t, &data, &doc, &manifest))
        .await
        .map_err(join_err)?
        .map_err(|m| ErrPayload::new("snapshot_publish", m))?;
    Ok(json!({
        "tag": tag,
        "registry": label,
        "sha256": sha,
        "files": [REG_SNAPSHOT, REG_SIGNATURE, REG_MANIFEST],
    }))
}

#[cfg(test)]
mod signing_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn key_in(dir: &Path) -> SigningKey {
        load_or_create_key(&dir.join(KEY_FILE)).unwrap()
    }

    #[test]
    fn sign_verify_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_in(dir.path());
        let data = b"canonical snapshot bytes";
        let doc = make_sig_doc(&key, "t1", data);
        verify_sig_doc(data, &doc, &key.verifying_key()).unwrap();
        assert_eq!(doc["sha256"], sha256_hex(data));
    }

    #[test]
    fn tampered_bytes_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_in(dir.path());
        let doc = make_sig_doc(&key, "t1", b"original");
        let err = verify_sig_doc(b"0riginal", &doc, &key.verifying_key()).unwrap_err();
        assert!(err.contains("sha256 mismatch"), "{err}");
    }

    #[test]
    fn forged_digest_with_stale_signature_is_rejected() {
        // Attacker rewrites sha256 to match tampered bytes; the
        // signature over the original bytes must still fail.
        let dir = tempfile::tempdir().unwrap();
        let key = key_in(dir.path());
        let mut doc = make_sig_doc(&key, "t1", b"original");
        doc["sha256"] = json!(sha256_hex(b"tampered"));
        let err = verify_sig_doc(b"tampered", &doc, &key.verifying_key()).unwrap_err();
        assert!(err.contains("signature invalid"), "{err}");
    }

    #[test]
    fn wrong_key_is_rejected() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let (ka, kb) = (key_in(a.path()), key_in(b.path()));
        assert_ne!(ka.to_bytes(), kb.to_bytes());
        let doc = make_sig_doc(&ka, "t1", b"data");
        let err = verify_sig_doc(b"data", &doc, &kb.verifying_key()).unwrap_err();
        assert!(err.contains("different key"), "{err}");
    }

    #[test]
    fn key_is_0600_and_stable_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let first = key_in(dir.path());
        let mode = std::fs::metadata(dir.path().join(KEY_FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(first.to_bytes(), key_in(dir.path()).to_bytes());
    }

    #[test]
    fn loose_key_permissions_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        key_in(dir.path());
        let p = dir.path().join(KEY_FILE);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = load_or_create_key(&p).unwrap_err();
        assert_eq!(err.code, "snapshot_key");
    }

    #[test]
    fn public_key_hex_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_in(dir.path());
        let hex = hex_encode(key.verifying_key().as_bytes());
        assert_eq!(parse_public_key(&hex).unwrap(), key.verifying_key());
        assert!(parse_public_key("zz").is_err());
    }

    #[test]
    fn directory_registry_publish_and_reverify() {
        let dir = tempfile::tempdir().unwrap();
        let key = key_in(dir.path());
        let data = b"snapshot payload".to_vec();
        let doc = make_sig_doc(&key, "rel", &data);
        let reg_dir = dir.path().join("registry");
        let spec = format!("file://{}", reg_dir.display());
        let reg = parse_registry(&spec, None).unwrap();
        let manifest = json!({"tag": "rel"});
        registry_put(&reg, "rel", &data, &doc, &manifest).unwrap();

        assert!(reg_dir.join("rel").join(REG_MANIFEST).is_file());
        let (got, got_doc) = registry_get(&reg, "rel").unwrap();
        assert_eq!(got, data);
        verify_sig_doc(&got, &got_doc, &key.verifying_key()).unwrap();

        // Tamper with the published snapshot: re-verify must fail.
        std::fs::write(reg_dir.join("rel").join(REG_SNAPSHOT), b"evil").unwrap();
        let (bad, bad_doc) = registry_get(&reg, "rel").unwrap();
        assert!(verify_sig_doc(&bad, &bad_doc, &key.verifying_key()).is_err());
    }

    #[test]
    fn relative_registry_is_refused() {
        assert!(parse_registry("relative/dir", None).is_err());
        assert!(matches!(
            parse_registry("https://example.com/reg/", None),
            Ok(Registry::Http { .. })
        ));
    }

    #[tokio::test]
    async fn ops_sign_verify_publish_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let paths = super::super::paths::CachePaths::with_root(dir.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        let state = DaemonState::new(paths).unwrap();
        let reg_dir = dir.path().join("registry");

        op_snapshot_save(&state, json!({"tag": "s1"})).await.unwrap();
        let signed = op_snapshot_sign(&state, json!({"tag": "s1"})).await.unwrap();
        let pubkey = op_snapshot_pubkey(&state, json!({})).await.unwrap();
        assert_eq!(signed["public_key"], pubkey["public_key"]);
        op_snapshot_verify(&state, json!({"tag": "s1"})).await.unwrap();
        op_snapshot_verify(&state, json!({"tag": "s1", "public_key": pubkey["public_key"]}))
            .await
            .unwrap();

        let reg = reg_dir.display().to_string();
        op_snapshot_publish(&state, json!({"tag": "s1", "registry": reg})).await.unwrap();
        let ok = op_snapshot_verify(&state, json!({"tag": "s1", "registry": reg})).await.unwrap();
        assert_eq!(ok["ok"], true);

        // Tamper with the local snapshot: verify and publish both refuse.
        let snap = snapshot_path(&state, "s1");
        let mut bytes = std::fs::read(&snap).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&snap, bytes).unwrap();
        let e = op_snapshot_verify(&state, json!({"tag": "s1"})).await.unwrap_err();
        assert_eq!(e.code, "snapshot_verify_failed");
        let e = op_snapshot_publish(&state, json!({"tag": "s1", "registry": reg})).await.unwrap_err();
        assert_eq!(e.code, "snapshot_verify_failed");

        // Wrong trusted key is rejected.
        let other = tempfile::tempdir().unwrap();
        let other_pub = hex_encode(key_in(other.path()).verifying_key().as_bytes());
        let e = op_snapshot_verify(&state, json!({"tag": "s1", "public_key": other_pub}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "snapshot_verify_failed");
    }
}
