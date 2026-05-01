// zsource — client-facing builtin for the daemon's compiled-file registry.
//
// Wraps the `source_resolve` IPC op (daemon/source_resolver.rs) so callers can
// drive the source-bytecode-cache from the CLI without modifying the
// interactive shell's `source` / `.` builtins.
//
// Per docs/DAEMON.md "Source / dot interception and file registry":
//   client: source /path/to/file.sh
//        ↓ stat() on file → gets mtime + inode
//        ↓ IPC source_resolve { path, mtime_ns, inode }
//        ↓ daemon: hit / stale / miss against compiled_files table
//        ↑ returns { hit, stale, path, mtime_ns, inode }
//
// The compiled-file body lives in catalog.db's compiled_files table (a
// hydrated mirror of the rkyv shard). v1 stores raw source bytes; bytecode
// arrives once the parser/compiler is wired in.
//
// CLI shape:
//   zsource <path>                   # resolve (hit | stale | miss)
//   zsource --stat <path>            # stat first, send mtime/inode for sanity check
//   zsource --info <path>            # show compiled_files row for the path

use serde_json::{json, Value};

use super::client::Client;
use super::paths::CachePaths;

fn err_exit(msg: &str) -> i32 {
    eprintln!("zshrs: zsource: {}", msg);
    1
}

fn print_pretty(v: &Value) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{}", s),
        Err(_) => println!("{}", v),
    }
}

fn connect() -> Result<Client, ()> {
    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("zshrs: zsource: daemon: {}", e);
            return Err(());
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("zshrs: zsource: daemon: {}", e);
        return Err(());
    }
    Client::connect(&paths).map_err(|e| {
        eprintln!("zshrs: zsource: daemon: {}", e);
    })
}

pub fn zsource(args: &[String]) -> i32 {
    let mut path: Option<String> = None;
    let mut send_stat = false;
    let mut iter = args.iter().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--stat" => send_stat = true,
            "-h" | "--help" => {
                println!("usage: zsource <path>          # resolve compiled-file cache for <path>");
                println!("       zsource --stat <path>   # also send mtime/inode for sanity check");
                return 0;
            }
            other if other.starts_with('-') => {
                return err_exit(&format!("unknown flag `{}`", other));
            }
            other => {
                if path.is_some() {
                    return err_exit("expected exactly one path");
                }
                path = Some(other.to_string());
            }
        }
    }
    let path = match path {
        Some(p) => p,
        None => return err_exit("usage: zsource <path>"),
    };

    // Daemon requires absolute paths — canonicalize if relative.
    let abs = match std::fs::canonicalize(&path) {
        Ok(p) => p.display().to_string(),
        Err(e) => return err_exit(&format!("cannot resolve `{}`: {}", path, e)),
    };

    let mut payload = json!({ "path": abs });
    if send_stat {
        match std::fs::metadata(&path) {
            Ok(m) => {
                let mtime_ns = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0);
                use std::os::unix::fs::MetadataExt;
                let inode = m.ino() as i64;
                payload["mtime_ns"] = json!(mtime_ns);
                payload["inode"] = json!(inode);
            }
            Err(e) => return err_exit(&format!("stat `{}`: {}", path, e)),
        }
    }

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("source_resolve", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&e.to_string()),
    }
}
