//! **zsh-z** — the frecency directory jumper (`z <partial>`) — ported to a
//! native zshrs plugin. A faithful reimplementation of
//! MenkeTechnologies/zsh-z: the datafile format, the frecency formula, the
//! aging rule, the matching, and the `z` options are all reproduced in Rust.
//!
//! - Datafile (`$ZSHZ_DATA`, default `~/.z`): `path|rank|time` per line.
//! - Frecency: `rank * (3.75 / (0.0001*dx + 1) + 0.25)`, `dx = now - time`.
//! - On visit: `rank += 1`, `time = now`; when Σrank > `$ZSHZ_MAX_SCORE`
//!   (9000) every rank ages `* 0.99`; non-existent dirs and rank < 1 drop.
//! - `z QUERY` cd's to the best frecency match; `-c` restrict to subdirs of
//!   `$PWD`; `-e` echo instead of cd; `-l` list; `-r`/`-t` rank/time method;
//!   `-x` remove `$PWD`; `--add PATH` record; `--complete` completion output.
//!
//! Recording is driven by a `chpwd` hook (`z --add "$PWD"`) that the plugin
//! installs at its first `z` invocation (evaluating shell during plugin load
//! is unsafe), so tracking begins once you first use `z`.

use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zshrs_plugin::{declare_plugin, Args, Host};

const MAX_SCORE_DEFAULT: f64 = 9000.0;

/// One datafile row.
struct Entry {
    path: String,
    rank: f64,
    time: i64,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn cfg(host: &Host, key: &str) -> Option<String> {
    host.getvar(key).filter(|s| !s.is_empty())
}

/// `$ZSHZ_DATA` / `$_Z_DATA` / `~/.z`.
fn datafile(host: &Host) -> String {
    cfg(host, "ZSHZ_DATA")
        .or_else(|| cfg(host, "_Z_DATA"))
        .unwrap_or_else(|| {
            let home = cfg(host, "HOME")
                .or_else(|| std::env::var("HOME").ok())
                .unwrap_or_default();
            format!("{home}/.z")
        })
}

fn max_score(host: &Host) -> f64 {
    cfg(host, "ZSHZ_MAX_SCORE")
        .or_else(|| cfg(host, "_Z_MAX_SCORE"))
        .and_then(|s| s.parse().ok())
        .unwrap_or(MAX_SCORE_DEFAULT)
}

/// Parse `path|rank|time`: path is up to the first `|`, time after the last
/// `|`, rank between — matching zsh-z's `%%\|*` / `##*\|` field splits.
fn parse_line(line: &str) -> Option<Entry> {
    let first = line.find('|')?;
    let last = line.rfind('|')?;
    if last <= first {
        return None;
    }
    let path = line[..first].to_string();
    let rank: f64 = line[first + 1..last].trim().parse().ok()?;
    let time: i64 = line[last + 1..].trim().parse().ok()?;
    Some(Entry { path, rank, time })
}

/// Load the datafile, dropping rows whose directory no longer exists (as
/// zsh-z does on every read/write).
fn load(host: &Host) -> Vec<Entry> {
    let df = datafile(host);
    let Ok(text) = std::fs::read_to_string(&df) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(parse_line)
        .filter(|e| std::path::Path::new(&e.path).is_dir())
        .collect()
}

/// Render a rank: whole numbers as integers, else full float (so a fresh
/// datafile stays byte-compatible with zsh-z's `$(( … ))` output).
fn fmt_rank(r: f64) -> String {
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// Atomically write the datafile (tempfile + rename), like `_zshz_add_path`.
fn store(host: &Host, entries: &[Entry]) {
    let df = datafile(host);
    let tmp = format!("{df}.{}.tmp", std::process::id());
    let mut out = String::new();
    for e in entries {
        out.push_str(&format!("{}|{}|{}\n", e.path, fmt_rank(e.rank), e.time));
    }
    if std::fs::write(&tmp, out).is_ok() {
        let _ = std::fs::rename(&tmp, &df);
    }
}

/// `_zshz_add_path` + `_zshz_update_datafile`: record `path`.
fn add_path(host: &Host, path: &str) {
    if path.is_empty() {
        return;
    }
    // $HOME isn't worth matching.
    let home = cfg(host, "HOME").or_else(|| std::env::var("HOME").ok());
    if home.as_deref() == Some(path) {
        return;
    }
    // ZSHZ_EXCLUDE_DIRS — prefix match (whitespace-separated when read as a
    // scalar; arrays flatten to space-joined via getvar).
    if let Some(ex) = cfg(host, "ZSHZ_EXCLUDE_DIRS").or_else(|| cfg(host, "_Z_EXCLUDE_DIRS")) {
        for e in ex.split_whitespace() {
            if !e.is_empty() && path.starts_with(e) {
                return;
            }
        }
    }

    let n = now();
    let mut entries = load(host);
    let mut count = 0.0;
    let mut found = false;
    for e in entries.iter_mut() {
        count += e.rank;
        if e.path == path {
            e.rank += 1.0;
            e.time = n;
            found = true;
        }
    }
    if !found {
        entries.push(Entry {
            path: path.to_string(),
            rank: 1.0,
            time: n,
        });
    }
    // Aging when the total rank grows too large.
    if count > max_score(host) {
        for e in entries.iter_mut() {
            e.rank *= 0.99;
        }
    }
    // Drop rank < 1 (kept the added row above so a brand-new dir survives).
    entries.retain(|e| e.rank >= 1.0 || e.path == path);
    store(host, &entries);
}

/// `_zshz_remove_path`: forget `path`.
fn remove_path(host: &Host, path: &str) {
    let mut entries = load(host);
    let before = entries.len();
    entries.retain(|e| e.path != path);
    if entries.len() != before {
        store(host, &entries);
    }
}

/// Score an entry under a method. `rank`, `time` (recency), or frecency.
fn score(e: &Entry, method: Method, n: i64) -> f64 {
    match method {
        Method::Rank => e.rank,
        Method::Time => (e.time - n) as f64, // most recent → highest (closest to 0)
        Method::Frecency => {
            let dx = (n - e.time) as f64;
            e.rank * (3.75 / (0.0001 * dx + 1.0) + 0.25)
        }
    }
}

#[derive(Clone, Copy)]
enum Method {
    Frecency,
    Rank,
    Time,
}

/// zsh `*`/`?` string glob (where `*` crosses `/`, matching `[[ $path ==
/// $pat ]]`). Iterative, backtracking on `*`.
fn glob_match(pat: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while t < text.len() {
        if p < pat.len() && (pat[p] == text[t] || pat[p] == b'?') {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == b'*' {
            star = p;
            mark = t;
            p += 1;
        } else if star != usize::MAX {
            p = star + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

/// Build the glob pattern from the query (runs of whitespace → `*`), wrapped
/// per zsh-z: `-c` restricts to `$PWD`-rooted (`fnd*`), else `*fnd*`.
fn make_pattern(query: &str, restrict_cwd: bool) -> String {
    let collapsed: String = {
        // spaces → '*' (a run of spaces becomes a single '*').
        let mut s = String::new();
        let mut prev_space = false;
        for c in query.chars() {
            if c.is_whitespace() {
                if !prev_space {
                    s.push('*');
                }
                prev_space = true;
            } else {
                s.push(c);
                prev_space = false;
            }
        }
        s
    };
    if restrict_cwd {
        format!("{collapsed}*")
    } else {
        format!("*{collapsed}*")
    }
}

/// Find matches. Returns (best_path, all_matches_sorted_desc). Case-sensitive
/// matches win; case-insensitive are the fallback (zsh-z's matches/imatches).
fn find_matches(
    host: &Host,
    query: &str,
    method: Method,
    restrict_cwd: bool,
) -> (Option<String>, Vec<(String, f64)>) {
    let n = now();
    let pat = make_pattern(query, restrict_cwd);
    let pat_l = pat.to_lowercase();

    let mut matches: Vec<(String, f64)> = Vec::new();
    let mut imatches: Vec<(String, f64)> = Vec::new();
    for e in load(host) {
        let sc = score(&e, method, n);
        if glob_match(pat.as_bytes(), e.path.as_bytes()) {
            matches.push((e.path, sc));
        } else if glob_match(pat_l.as_bytes(), e.path.to_lowercase().as_bytes()) {
            imatches.push((e.path, sc));
        }
    }
    let pick = if !matches.is_empty() { &mut matches } else { &mut imatches };
    pick.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let best = pick.first().map(|(p, _)| p.clone());
    (best, std::mem::take(pick))
}

fn shquote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Install the `chpwd` hook that records visited dirs. Done lazily at the
/// first `z` call (evaluating shell during plugin load hangs the VM).
fn ensure_hook(host: &Host) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    host.eval(
        "autoload -Uz add-zsh-hook 2>/dev/null; \
         _zshrs_z_chpwd() { z --add \"${PWD:A}\" 2>/dev/null }; \
         add-zsh-hook chpwd _zshrs_z_chpwd 2>/dev/null; :",
    );
}

/// The `z` command.
fn z(host: &Host, args: &Args) -> c_int {
    let raw: Vec<&str> = args.rest().iter().map(String::as_str).collect();

    // --add / --complete are internal (from the hook / completion); they must
    // not trigger hook install or cd.
    if raw.first() == Some(&"--add") {
        add_path(host, &raw[1..].join(" "));
        return 0;
    }
    if raw.first() == Some(&"--complete") {
        let q = raw.get(1).copied().unwrap_or("");
        let (_, all) = find_matches(host, q, Method::Frecency, false);
        for (p, _) in all {
            host.add_match(&p);
        }
        return 0;
    }

    ensure_hook(host);

    // Parse options (zparseopts-ish): flags then the query.
    let mut method = Method::Frecency;
    let mut restrict_cwd = false;
    let mut echo = false;
    let mut list = false;
    let mut remove = false;
    let mut words: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            "--" => {
                words.extend_from_slice(&raw[i + 1..]);
                break;
            }
            "-c" => restrict_cwd = true,
            "-e" => echo = true,
            "-l" => list = true,
            "-r" => method = Method::Rank,
            "-t" => method = Method::Time,
            "-x" => remove = true,
            "-h" | "--help" => {
                host.print("z [-cehlrtx] [--add PATH] [--complete] [DIR...]\n");
                return 0;
            }
            w if w.starts_with('-') && w.len() > 1 => {
                host.print("z: improper option(s) given\n");
                return 1;
            }
            w => words.push(w),
        }
        i += 1;
    }

    let query = words.join(" ");

    if remove {
        let pwd = cfg(host, "PWD").unwrap_or_default();
        remove_path(host, if query.is_empty() { &pwd } else { &query });
        return 0;
    }

    // A trailing existing absolute path → just cd there (no matching).
    if let Some(last) = words.last() {
        if last.starts_with('/') && !echo && !list && std::path::Path::new(last).is_dir() {
            return host.eval(&format!("cd -- {}", shquote(last)));
        }
    }

    // Empty query → list mode.
    let list = list || query.is_empty();

    let (best, all) = find_matches(host, &query, method, restrict_cwd);

    if list {
        for (p, sc) in all.iter().rev() {
            host.print(&format!("{:<10.2} {}\n", sc, p));
        }
        return if all.is_empty() { 1 } else { 0 };
    }

    match best {
        Some(path) if echo => {
            host.print(&format!("{path}\n"));
            0
        }
        Some(path) => host.eval(&format!("cd -- {}", shquote(&path))),
        None => 1,
    }
}

declare_plugin! {
    name: "zsh-z",
    version: "0.1.0",
    builtins: {
        "z" => z,
    },
    completions: {
        "z" => z_complete,
    },
}

/// Completion generator for `z`: frecency-sorted matching directories for
/// the current partial word.
fn z_complete(host: &Host, args: &Args) -> c_int {
    let a = args.rest();
    let Some(current) = a.first().and_then(|s| s.parse::<usize>().ok()) else {
        return 1;
    };
    let words = &a[1..]; // ["z", partial...]
    let partial = current
        .checked_sub(1)
        .and_then(|i| words.get(i))
        .map(String::as_str)
        .unwrap_or("");
    let (_, all) = find_matches(host, partial, Method::Frecency, false);
    for (p, _) in all {
        host.add_match(&p);
    }
    0
}
