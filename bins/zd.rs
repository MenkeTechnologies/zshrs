#![cfg(feature = "zd")]

//! `zd` — standalone HTTP client for `zshrs-daemon`.
//!
//! When the user is INSIDE zshrs, `zd` is a builtin (in-process, no
//! fork) — see `daemon/builtins.rs::zd`. This binary is for everyone
//! else: bash/fish/dash/ksh/nu/elvish/pwsh users + scripts + CI
//! pipelines. Same arg surface either way; only the transport differs
//! (HTTP here, Unix socket from inside zshrs).
//!
//! See docs/DAEMON_AS_SERVICE.md for the canonical op contract. Every
//! `zd <SUBCMD>` maps 1:1 to a `POST /op/<NAME>` against the daemon's
//! HTTP listener (or a `GET` for `/health` / `/ops` / `/metrics`).
//!
//! Defaults:
//!   - URL:   `$DAEMON_URL`   or `http://127.0.0.1:7733`
//!   - Token: `$DAEMON_TOKEN` (empty = no auth header sent)
//!
//! Output: raw JSON to stdout (pipe through `jq` for pretty). Errors
//! to stderr; exit code 0 on success, 1 on transport / protocol error,
//! 2 on usage error.

use std::process::{Command, ExitCode, Stdio};

use serde_json::Value;

use zshrs_daemon::zd_dispatch::{dispatch, Transport};

/// HTTP transport: ureq-based, talks to the daemon's `[http].listen`
/// endpoint. Used by the standalone-binary form of `zd`.
struct HttpTransport {
    url: String,
    token: String,
}

impl HttpTransport {
    fn new() -> Self {
        Self {
            url: std::env::var("DAEMON_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:7733".to_string()),
            token: std::env::var("DAEMON_TOKEN").unwrap_or_default(),
        }
    }
}

impl Transport for HttpTransport {
    fn post(&mut self, op: &str, body: Value) -> Result<String, String> {
        let url = format!("{}/op/{}", self.url, op);
        let req = ureq::post(&url).set("Content-Type", "application/json");
        let req = if self.token.is_empty() {
            req
        } else {
            req.set("Authorization", &format!("Bearer {}", self.token))
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

    fn get(&mut self, path: &str) -> Result<String, String> {
        let url = format!("{}{}", self.url, path);
        let req = ureq::get(&url);
        let req = if self.token.is_empty() {
            req
        } else {
            req.set("Authorization", &format!("Bearer {}", self.token))
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
    /// that every shell user already has `curl` makes shelling out
    /// the cleanest path.
    fn sse(&mut self, path: &str) -> Result<String, String> {
        let url = format!("{}{}", self.url, path);
        let mut cmd = Command::new("curl");
        cmd.arg("-sN");
        if !self.token.is_empty() {
            cmd.arg("-H").arg(format!("Authorization: Bearer {}", self.token));
        }
        cmd.arg(&url);
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());
        let mut child = cmd.spawn().map_err(|e| format!("spawn curl: {e}"))?;
        let status = child.wait().map_err(|e| format!("wait curl: {e}"))?;
        if !status.success() {
            return Err(format!("curl exited {status}"));
        }
        Ok(String::new())
    }
}

fn main() -> ExitCode {
    // Make sure ~/.zshrs exists with the default config files. Same
    // idempotent call every binary makes — `zd` is often the first
    // contact a non-zsh user (bash / fish / CI script) has with the
    // zshrs stack, so seeding here means they get the dir + configs
    // even if they never run `zshrs` or `zshrs-daemon` directly.
    if let Ok(paths) = zshrs_daemon::paths::CachePaths::resolve() {
        let _ = paths.ensure_dirs();
        let _ = paths.ensure_default_configs();
    }

    let argv: Vec<String> = std::env::args().skip(1).collect();
    // Honor --url / --token on the bin (the builtin always uses the
    // local Unix socket). Strip them here, leave the rest for dispatch.
    let mut transport = HttpTransport::new();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--url" => {
                if i + 1 >= argv.len() {
                    eprintln!("zd: --url requires an argument");
                    return ExitCode::from(2);
                }
                transport.url = argv[i + 1].clone();
                i += 2;
            }
            "--token" => {
                if i + 1 >= argv.len() {
                    eprintln!("zd: --token requires an argument");
                    return ExitCode::from(2);
                }
                transport.token = argv[i + 1].clone();
                i += 2;
            }
            _ => break,
        }
    }
    let dispatched = &argv[i..];
    ExitCode::from(dispatch(dispatched, &mut transport) as u8)
}
