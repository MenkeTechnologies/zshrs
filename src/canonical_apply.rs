//! Apply daemon canonical state to a freshly-built ShellExecutor.
//!
//! This is the "skip configs" payoff: instead of sourcing
//! `/etc/zshenv` + `~/.zshenv` + `/etc/zprofile` + … + `~/.zlogin`
//! (every dotfile in the zsh init chain), the shell calls this once
//! at startup. The recorder previously captured end-state via runtime
//! AOP; the daemon stores that end-state in the canonical engine.
//! `apply_all` queries each subsystem and copies values straight into
//! the executor's pub fields. No re-evaluation; no `.zshrc` parse;
//! no plugin discovery.
//!
//! Failure mode: any IPC error means a partial apply. We log + return
//! a count so the caller can decide whether to fall back to vanilla
//! mode. Currently we apply best-effort and let the user see the
//! diagnostic in the log.
//!
//! Per `docs/DAEMON.md` "zshrs ↔ zshrs-daemon: independent processes"
//! + the "zshrs skips ALL zsh configs when daemon is running and has
//! config" rule from the user mandate.

#![cfg(feature = "daemon")]

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::daemon::client::call_once_no_spawn;
use crate::exec::ShellExecutor;

/// Pull canonical state for every subsystem the recorder populates and
/// apply to the executor. Returns the total number of rows applied
/// (sum across every kind), or 0 if the daemon is unreachable or
/// returned nothing.
pub fn apply_all(executor: &mut ShellExecutor) -> usize {
    let mut total = 0;
    total += apply_aliases(executor);
    total += apply_galiases(executor);
    total += apply_saliases(executor);
    total += apply_env(executor);
    total += apply_params(executor);
    total += apply_setopt(executor);
    total += apply_path(executor, "path");
    total += apply_path(executor, "fpath");
    total += apply_traps(executor);
    tracing::info!(rows = total, "canonical state applied to executor");
    total
}

fn query_kind(kind: &str) -> Vec<(String, String)> {
    let body = json!({
        "shell_id": "zshrs",
        "kind": kind,
        "limit": 100_000,
    });
    let resp = match call_once_no_spawn("definitions_query", body) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(kind, error = %e, "definitions_query failed");
            return Vec::new();
        }
    };
    let records = match resp.get("records").and_then(Value::as_array) {
        Some(r) => r,
        None => return Vec::new(),
    };
    records
        .iter()
        .filter_map(|r| {
            let name = r.get("name").and_then(Value::as_str)?.to_string();
            let value = r
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some((name, value))
        })
        .collect()
}

fn apply_aliases(executor: &mut ShellExecutor) -> usize {
    let rows = query_kind("alias");
    let n = rows.len();
    for (name, body) in rows {
        executor.aliases.insert(name, body);
    }
    n
}

fn apply_galiases(executor: &mut ShellExecutor) -> usize {
    let rows = query_kind("galias");
    let n = rows.len();
    for (name, body) in rows {
        executor.global_aliases.insert(name, body);
    }
    n
}

fn apply_saliases(executor: &mut ShellExecutor) -> usize {
    let rows = query_kind("salias");
    let n = rows.len();
    for (name, body) in rows {
        executor.suffix_aliases.insert(name, body);
    }
    n
}

fn apply_env(executor: &mut ShellExecutor) -> usize {
    let rows = query_kind("env");
    let n = rows.len();
    for (name, value) in rows {
        // `export FOO=bar` lands in process env (so child commands see
        // it) AND in the shell's variables map (so $FOO expands).
        std::env::set_var(&name, &value);
        executor.variables.insert(name, value);
    }
    n
}

fn apply_params(executor: &mut ShellExecutor) -> usize {
    let rows = query_kind("params");
    let n = rows.len();
    for (name, value) in rows {
        executor.variables.insert(name, value);
    }
    n
}

fn apply_setopt(executor: &mut ShellExecutor) -> usize {
    let rows = query_kind("setopt");
    let n = rows.len();
    for (name, on_off) in rows {
        let on = matches!(on_off.as_str(), "on" | "true" | "1");
        executor.options.insert(name, on);
    }
    n
}

fn apply_path(executor: &mut ShellExecutor, kind: &str) -> usize {
    // path / fpath are stored as ordered rows keyed by string-encoded
    // index (0, 1, 2, …) — see `daemon/ops.rs:op_recorder_ingest`.
    let rows = query_kind(kind);
    let n = rows.len();
    if n == 0 {
        return 0;
    }
    let mut indexed: Vec<(usize, String)> = rows
        .into_iter()
        .filter_map(|(idx, dir)| idx.parse::<usize>().ok().map(|i| (i, dir)))
        .collect();
    indexed.sort_by_key(|(i, _)| *i);
    let dirs: Vec<String> = indexed.into_iter().map(|(_, d)| d).collect();
    match kind {
        "path" => {
            // Push into both $PATH (process env, child commands see it)
            // and the shell's own indexed array if relevant.
            std::env::set_var("PATH", dirs.join(":"));
            executor.variables.insert("PATH".to_string(), dirs.join(":"));
            executor.arrays.insert("path".to_string(), dirs);
        }
        "fpath" => {
            executor.fpath = dirs.iter().map(PathBuf::from).collect();
            executor.arrays.insert(
                "fpath".to_string(),
                dirs.iter().cloned().collect(),
            );
            executor
                .variables
                .insert("FPATH".to_string(), dirs.join(":"));
            std::env::set_var("FPATH", dirs.join(":"));
        }
        _ => {}
    }
    n
}

fn apply_traps(executor: &mut ShellExecutor) -> usize {
    let rows = query_kind("trap");
    let n = rows.len();
    for (signal, handler) in rows {
        executor.traps.insert(signal, handler);
    }
    n
}
