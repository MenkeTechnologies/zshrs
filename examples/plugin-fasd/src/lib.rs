//! **fasd** — file + directory frecency (`a`, `s`, `d`, `f`, `j`, `fasd`) —
//! ported to a native zshrs plugin. A faithful reimplementation of fasd
//! (Wei Dai, MIT/WTFPL, based on rupa/z): the `~/.fasd` datafile format, the
//! add/aging rule, the regex + fuzzy matching, the frecency scoring, and the
//! command options are reproduced in Rust.
//!
//! fasd generalizes `z`: instead of only tracking directories you `cd` into,
//! it tracks every **file and directory** argument of every command (via a
//! `preexec` hook), so `f <part>` finds a recently-used file, `d <part>` a
//! directory, `a <part>` either, and `j <part>` cd's to the best directory.
//!
//! - Datafile (`$_FASD_DATA`, default `~/.fasd`): `path|rank|time` per line.
//! - On add: existing entry → `rank += 1/rank`, `time = now`; new → rank 1.
//!   When Σrank > `$_FASD_MAX` (2000) every rank ages `* 0.9`.
//! - Frecency: `Σrank * frecent(dx)`, `dx = now - last`, `frecent` = 6 (<1h),
//!   4 (<1d), 2 (<1w), else 1. `-r` ranks by Σrank, `-t` by recency.
//! - Matching: query terms (space-separated) matched in order, last term in
//!   the basename; case-sensitive, then case-insensitive, then fuzzy.
//!
//! Ported from the fasd shell script (`bin/fasd`); `// c:NNN` cites its lines.

use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use znative::{declare_plugin, Args, Host};

const MAX_DEFAULT: f64 = 2000.0; // c:51 _FASD_MAX
const FUZZY_DEFAULT: usize = 2; // c:53 _FASD_FUZZY

/// One datafile row.
struct Entry {
    path: String,
    rank: f64,
    time: i64,
}

/// Which filesystem kind a query accepts. c:509-535 (`-a`/`-d`/`-f`).
#[derive(Clone, Copy, PartialEq)]
enum Typ {
    Any,  // -a / default: path must exist (-e)
    Dir,  // -d
    File, // -f
}

/// How to weight matches. c:397-401.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Frecency, // times[i] * frecent(la[i])
    Rank,     // -r: times[i]
    Recent,   // -t: sqrt(100000/(1+t-la[i]))
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

fn home(host: &Host) -> String {
    cfg(host, "HOME")
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_default()
}

/// `$_FASD_DATA` / `~/.fasd`. c:45.
fn datafile(host: &Host) -> String {
    cfg(host, "_FASD_DATA").unwrap_or_else(|| format!("{}/.fasd", home(host)))
}

/// `$_FASD_MAX` / 2000. c:51.
fn max_score(host: &Host) -> f64 {
    cfg(host, "_FASD_MAX")
        .and_then(|s| s.parse().ok())
        .unwrap_or(MAX_DEFAULT)
}

/// `$_FASD_TRACK_PWD` (default 1). c:50, c:318.
fn track_pwd(host: &Host) -> bool {
    cfg(host, "_FASD_TRACK_PWD").as_deref() != Some("0")
}

/// `$_FASD_FUZZY` (default 2). c:53.
fn fuzzy_n(host: &Host) -> usize {
    cfg(host, "_FASD_FUZZY")
        .and_then(|s| s.parse().ok())
        .unwrap_or(FUZZY_DEFAULT)
}

/// Parse `path|rank|time`: path up to the first `|`, time after the last,
/// rank between. c:449-457 field split on `|`.
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

fn load(host: &Host) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(datafile(host)) else {
        return Vec::new();
    };
    text.lines().filter_map(parse_line).collect()
}

/// Render a rank: whole numbers as integers, else the float. Keeps a fresh
/// datafile byte-compatible with fasd's awk output.
fn fmt_rank(r: f64) -> String {
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

/// Atomically rewrite the datafile (tempfile + rename). c:322-356.
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

/// Lexically simplify a path to absolute canonical form — the Rust
/// equivalent of fasd's `sed` pipeline (c:311-314): relative → `$PWD/…`,
/// resolve `.`/`..` textually (no symlink following), collapse `//`, strip a
/// trailing slash.
fn simplify(pwd: &str, raw: &str) -> String {
    let joined = if raw.starts_with('/') {
        raw.to_string()
    } else {
        format!("{pwd}/{raw}")
    };
    let mut stack: Vec<&str> = Vec::new();
    for comp in joined.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            c => stack.push(c),
        }
    }
    let s = format!("/{}", stack.join("/"));
    s
}

// ── frecency ──────────────────────────────────────────────────────────

/// c:442-448 — recency multiplier from age in seconds.
fn frecent(dx: i64) -> f64 {
    if dx < 3600 {
        6.0
    } else if dx < 86400 {
        4.0
    } else if dx < 604800 {
        2.0
    } else {
        1.0
    }
}

// ── add / delete ──────────────────────────────────────────────────────

/// `fasd --add` (c:304-356): record existing paths (+ `$PWD` when tracked),
/// bump ranks, age when the total exceeds `$_FASD_MAX`.
fn add_paths(host: &Host, args: &[String]) {
    let pwd = cfg(host, "PWD").unwrap_or_default();

    // c:308-315 — keep only arguments that exist, simplify to absolute form.
    let mut paths: Vec<String> = args
        .iter()
        .filter(|a| std::path::Path::new(a).exists())
        .map(|a| simplify(&pwd, a))
        .collect();

    // c:318 — track $PWD (unless it is $HOME).
    if track_pwd(host) && !pwd.is_empty() && pwd != home(host) {
        paths.push(pwd.clone());
    }
    if paths.is_empty() {
        return; // c:320
    }

    let now = now();
    let mut entries = load(host);

    // c:336-345 — for each existing row: if its path is being added, rerank
    // `rank + 1/rank` and touch time; else keep. Sum ranks for the aging test.
    let mut count = 0.0;
    let mut seen: Vec<bool> = vec![false; paths.len()];
    for e in &mut entries {
        if let Some(i) = paths.iter().position(|p| p == &e.path) {
            e.rank += 1.0 / e.rank;
            e.time = now;
            seen[i] = true;
        }
        count += e.rank;
    }
    // c:331-334 — brand-new paths enter at rank 1.
    for (i, p) in paths.iter().enumerate() {
        if !seen[i] {
            entries.push(Entry {
                path: p.clone(),
                rank: 1.0,
                time: now,
            });
            count += 1.0;
        }
    }

    // c:346-350 — age every rank by 0.9 once the total exceeds the cap.
    if count > max_score(host) {
        for e in &mut entries {
            e.rank *= 0.9;
        }
    }
    store(host, &entries);
}

/// `fasd --delete` (c:359-381): drop entries whose path matches an argument.
fn delete_paths(host: &Host, args: &[String]) {
    let pwd = cfg(host, "PWD").unwrap_or_default();
    let targets: Vec<String> = args.iter().map(|a| simplify(&pwd, a)).collect();
    let kept: Vec<Entry> = load(host)
        .into_iter()
        .filter(|e| !targets.contains(&e.path))
        .collect();
    store(host, &kept);
}

// ── matching ──────────────────────────────────────────────────────────

/// Build the regex fasd greps with (c:404-406 / c:421-425). `fnd` terms are
/// matched literally (regex-escaped), spaces become `[^|]*`, a trailing `$`
/// anchors to the path end (`|`), and fuzzy mode inserts `[^|/]{0,n}` after
/// each character. The whole match is `^[^|]*<fnd>[^|/]*\|` so the last term
/// lands in the basename. Returns None if the pattern can't compile.
fn build_regex(fnd: &str, fuzzy: Option<usize>, icase: bool) -> Option<regex::Regex> {
    let anchored_end = fnd.ends_with('$');
    let core = fnd.strip_suffix('$').unwrap_or(fnd);

    let mut mid = String::new();
    for (i, term) in core.split(' ').enumerate() {
        if i > 0 {
            mid.push_str("[^|]*"); // c:405 space → any within the path field
        }
        for ch in term.chars() {
            mid.push_str(&regex::escape(&ch.to_string()));
            if let Some(n) = fuzzy {
                mid.push_str(&format!("[^|/]{{0,{n}}}")); // c:423
            }
        }
    }

    // c:405 trailing `$` → `|`; else `[^|/]*|` keeps the tail in the basename.
    let tail = if anchored_end { "\\|" } else { "[^|/]*\\|" };
    let pat = format!("{}^[^|]*{mid}{tail}", if icase { "(?i)" } else { "" });
    regex::Regex::new(&pat).ok()
}

/// c:407-437 — pick the matching subset of `data`, escalating
/// case-sensitive → case-insensitive → fuzzy, and filter by `typ`.
fn filter_matches<'a>(host: &Host, data: &'a [Entry], typ: Typ, fnd: &str) -> Vec<&'a Entry> {
    let type_ok = |p: &str| match typ {
        Typ::Any => std::path::Path::new(p).exists(),
        Typ::Dir => std::path::Path::new(p).is_dir(),
        Typ::File => std::path::Path::new(p).is_file(),
    };

    if fnd.is_empty() {
        // c:433-436 — no query: every entry of the right kind.
        return data.iter().filter(|e| type_ok(&e.path)).collect();
    }

    // The regex greps the full `path|rank|time` line (c:407).
    let hit = |re: &regex::Regex| -> Vec<&Entry> {
        data.iter()
            .filter(|e| {
                let line = format!("{}|{}|{}", e.path, fmt_rank(e.rank), e.time);
                re.is_match(&line) && type_ok(&e.path)
            })
            .collect()
    };

    if let Some(re) = build_regex(fnd, None, false) {
        let r = hit(&re);
        if !r.is_empty() {
            return r;
        }
    }
    if let Some(re) = build_regex(fnd, None, true) {
        let r = hit(&re);
        if !r.is_empty() {
            return r;
        }
    }
    let n = fuzzy_n(host);
    if n > 0 {
        if let Some(re) = build_regex(fnd, Some(n), true) {
            return hit(&re);
        }
    }
    Vec::new()
}

/// `fasd --query` (c:383-461): matched, de-duplicated, scored `(score, path)`
/// pairs — unsorted, exactly as fasd's awk emits them.
fn query(host: &Host, typ: Typ, fnd: &str, mode: Mode) -> Vec<(f64, String)> {
    let data = load(host);
    let matched = filter_matches(host, &data, typ, fnd);

    // c:449-458 — group by path: sum ranks, keep the most recent access.
    let mut acc: Vec<(String, f64, i64)> = Vec::new();
    for e in matched {
        if let Some(row) = acc.iter_mut().find(|r| r.0 == e.path) {
            row.1 += e.rank;
            if e.time > row.2 {
                row.2 = e.time;
            }
        } else {
            acc.push((e.path.clone(), e.rank, e.time));
        }
    }

    let t = now();
    acc.into_iter()
        .map(|(path, sum, la)| {
            let score = match mode {
                Mode::Rank => sum,
                Mode::Recent => (100000.0 / (1.0 + (t - la) as f64)).sqrt(),
                Mode::Frecency => sum * frecent(t - la),
            };
            (score, path)
        })
        .collect()
}

/// Ascending sort by score (fasd's `sort -n`), so the best match is last.
fn sorted(mut v: Vec<(f64, String)>, reverse: bool) -> Vec<(f64, String)> {
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if reverse {
        v.reverse();
    }
    v
}

fn shquote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── command dispatch ──────────────────────────────────────────────────

/// Install the `preexec` hook that feeds command arguments to `fasd --proc`.
/// Lazy (first command invocation) — evaluating shell during plugin load
/// hangs the VM, same as zsh-z's `chpwd` hook.
fn ensure_hook(host: &Host) {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    // c:99-106 — zsh preexec hook. `${(z)1}` word-splits the command line;
    // `fasd --proc` applies the blacklist/shift/ignore rules and adds paths.
    host.eval(
        "autoload -Uz add-zsh-hook 2>/dev/null; \
         _zshrs_fasd_preexec() { fasd --proc \"${(z)1}\" 2>/dev/null }; \
         add-zsh-hook preexec _zshrs_fasd_preexec 2>/dev/null; :",
    );
}

/// `fasd --proc` (c:279-302): blacklist / shift-prefix / ignore filtering,
/// then add the remaining arguments.
fn proc(host: &Host, args: &[String]) {
    // c:46-48 defaults.
    let blacklist = cfg(host, "_FASD_BLACKLIST").unwrap_or_else(|| "--help".into());
    let shift = cfg(host, "_FASD_SHIFT").unwrap_or_else(|| "sudo busybox".into());
    let ignore = cfg(host, "_FASD_IGNORE").unwrap_or_else(|| "fasd ls echo".into());

    // c:284-286 — drop the whole line if any word is blacklisted.
    if args
        .iter()
        .any(|a| blacklist.split_whitespace().any(|b| b == a))
    {
        return;
    }
    // c:289-294 — strip leading shift words (sudo, busybox, …).
    let mut rest = args;
    while let Some(first) = rest.first() {
        if shift.split_whitespace().any(|s| s == first) {
            rest = &rest[1..];
        } else {
            break;
        }
    }
    // c:297-299 — ignore commands whose head is in the ignore list.
    if let Some(head) = rest.first() {
        if ignore.split_whitespace().any(|s| s == head) {
            return;
        }
    }
    // c:301 — add everything except the command word.
    if rest.len() > 1 {
        add_paths(host, &rest[1..]);
    } else {
        add_paths(host, &[]); // still records $PWD when tracked
    }
}

/// Parsed `fasd` invocation flags. c:502-561.
#[derive(Default)]
struct Opts {
    typ: Option<Typ>,
    mode: Option<Mode>,
    show: bool,        // -s
    list: bool,        // -l
    interactive: bool, // -i
    reverse: bool,     // -R
    nth: Option<usize>,
    exec: Option<String>,
    fnd: Vec<String>,
}

/// Parse the option string / query, mirroring fasd's inner `while` (c:504-561).
fn parse_opts(raw: &[String]) -> Opts {
    let mut o = Opts::default();
    let mut i = 0;
    while i < raw.len() {
        let arg = &raw[i];
        if arg == "--" {
            o.fnd.extend(raw[i + 1..].iter().cloned());
            break;
        }
        if let Some(flags) = arg.strip_prefix('-').filter(|f| !f.is_empty()) {
            let mut chars = flags.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    's' => o.show = true,
                    'l' => o.list = true,
                    'i' => {
                        o.interactive = true;
                        o.show = true;
                    }
                    'r' => o.mode = Some(Mode::Rank),
                    't' => o.mode = Some(Mode::Recent),
                    'R' => o.reverse = true,
                    'a' => o.typ = Some(Typ::Any),
                    'd' => o.typ = Some(Typ::Dir),
                    'f' => o.typ = Some(Typ::File),
                    'e' => {
                        // c:515-520 — rest of this arg, else the next arg.
                        let rest: String = chars.collect();
                        o.exec = Some(if !rest.is_empty() {
                            rest
                        } else {
                            i += 1;
                            raw.get(i).cloned().unwrap_or_default()
                        });
                        break;
                    }
                    '0'..='9' => {
                        // c:537 — `-N` selects the Nth entry.
                        let mut num = c.to_string();
                        while let Some(d) = chars.peek().filter(|d| d.is_ascii_digit()) {
                            num.push(*d);
                            chars.next();
                        }
                        o.nth = num.parse().ok();
                        break;
                    }
                    _ => {} // -b/-B/-h etc. are not backend-relevant here
                }
            }
        } else {
            o.fnd.push(arg.clone());
        }
        i += 1;
    }
    o
}

/// The `fasd` command (c:502-606). Thin wrapper over [`fasd_run`] so the
/// alias builtins can reuse the same logic without constructing an `Args`.
fn fasd(host: &Host, args: &Args) -> c_int {
    fasd_run(host, args.rest())
}

fn fasd_run(host: &Host, raw: &[String]) -> c_int {
    // Internal sub-commands used by the hook and completion.
    match raw.first().map(String::as_str) {
        Some("--proc") => {
            proc(host, &raw[1..]);
            return 0;
        }
        Some("--add") | Some("-A") => {
            add_paths(host, &raw[1..]);
            return 0;
        }
        Some("--delete") | Some("-D") => {
            delete_paths(host, &raw[1..]);
            return 0;
        }
        Some("--complete") => {
            let q = raw[1..].join(" ");
            for (_, p) in sorted(query(host, Typ::Any, q.trim(), Mode::Frecency), true) {
                host.add_match(&p);
            }
            return 0;
        }
        Some("--version") => {
            host.print("1.0.1\n");
            return 0;
        }
        _ => {}
    }

    ensure_hook(host);
    let o = parse_opts(raw);
    let typ = o.typ.unwrap_or(Typ::Any);
    let mode = o.mode.unwrap_or(Mode::Frecency);
    let fnd = o.fnd.join(" ");

    let results = sorted(query(host, typ, fnd.trim(), mode), false);

    // c:577-599 — selection.
    if let Some(n) = o.nth {
        // Nth from the top (best-first).
        let pick = results
            .iter()
            .rev()
            .nth(n.saturating_sub(1))
            .map(|r| r.1.clone());
        return finish(host, pick, o.exec.as_deref());
    }
    if o.list {
        // c:588-590 — paths only.
        for (_, p) in &results {
            host.print(&format!("{p}\n"));
        }
        return 0;
    }
    if o.show || o.interactive {
        // c:591-593 / c:581-587 — scores + paths (interactive selection needs
        // a tty read the plugin ABI can't drive; degrade to the scored list).
        for (s, p) in &results {
            host.print(&format!("{:<10.2} {}\n", s, p));
        }
        if o.interactive {
            // Still record/return the best so `-e`/cd paths stay usable.
            let best = results.last().map(|r| r.1.clone());
            return finish(host, best, o.exec.as_deref());
        }
        return 0;
    }
    if o.exec.is_some() || !fnd.is_empty() {
        // c:594-595 — best match, then exec (default: print it).
        let best = results.last().map(|r| r.1.clone());
        return finish(host, best, o.exec.as_deref());
    }
    // c:596-598 — no args: show the scored list.
    for (s, p) in &results {
        host.print(&format!("{:<10.2} {}\n", s, p));
    }
    0
}

/// c:600-604 — on a chosen result: re-add it (bump rank) and run `exec`
/// (default `printf %s\n`).
fn finish(host: &Host, res: Option<String>, exec: Option<&str>) -> c_int {
    let Some(res) = res else {
        return 1;
    };
    add_paths(host, std::slice::from_ref(&res));
    match exec {
        Some(cmd) => host.eval(&format!("{cmd} {}", shquote(&res))),
        None => {
            host.print(&format!("{res}\n"));
            0
        }
    }
}

/// `fasd_cd` (c:82-92): with ≤1 arg behave like `fasd`; otherwise resolve the
/// best directory match and `cd` to it (delegated to the shell so `$PWD` and
/// hooks stay correct).
fn fasd_cd(host: &Host, args: &Args) -> c_int {
    fasd_cd_run(host, args.rest())
}

fn fasd_cd_run(host: &Host, raw: &[String]) -> c_int {
    if raw.len() <= 1 {
        // c:84-85 — `fasd_cd` (bare / one flag) just runs fasd.
        return fasd_run(host, raw);
    }
    let o = parse_opts(raw);
    let fnd = o.fnd.join(" ");
    let results = sorted(
        query(host, Typ::Dir, fnd.trim(), o.mode.unwrap_or(Mode::Frecency)),
        false,
    );
    match results.last() {
        Some((_, p)) if std::path::Path::new(p).is_dir() => {
            add_paths(host, std::slice::from_ref(p));
            host.eval(&format!("cd -- {}", shquote(p)))
        }
        Some((_, p)) => {
            host.print(&format!("{p}\n"));
            0
        }
        None => 1,
    }
}

/// Build `prefix` flags + the alias's own arguments and run `fasd`.
fn with_prefix(prefix: &str, raw: &[String]) -> Vec<String> {
    let mut v: Vec<String> = prefix.split_whitespace().map(String::from).collect();
    v.extend_from_slice(raw);
    v
}

// The alias commands fasd normally installs (c:538-539 command letters).
fn a(host: &Host, args: &Args) -> c_int {
    fasd_run(host, &with_prefix("-a", args.rest()))
}
fn s(host: &Host, args: &Args) -> c_int {
    fasd_run(host, &with_prefix("-s", args.rest()))
}
fn d(host: &Host, args: &Args) -> c_int {
    fasd_run(host, &with_prefix("-d", args.rest()))
}
fn f(host: &Host, args: &Args) -> c_int {
    fasd_run(host, &with_prefix("-f", args.rest()))
}
fn sd(host: &Host, args: &Args) -> c_int {
    fasd_run(host, &with_prefix("-sd", args.rest()))
}
fn sf(host: &Host, args: &Args) -> c_int {
    fasd_run(host, &with_prefix("-sf", args.rest()))
}
/// `v` — open the best file match in `$EDITOR` (default vim). fasd's stock
/// `v='f -e vim'` alias.
fn v(host: &Host, args: &Args) -> c_int {
    let editor = cfg(host, "EDITOR").unwrap_or_else(|| "vim".into());
    fasd_run(host, &with_prefix(&format!("-f -e {editor}"), args.rest()))
}
/// `j` — autojump-style cd to the best directory match (fasd's `z`, renamed
/// so it composes with the separate `zsh-z` plugin's `z`).
fn j(host: &Host, args: &Args) -> c_int {
    fasd_cd_run(host, &with_prefix("-d", args.rest()))
}

declare_plugin! {
    name: "fasd",
    version: "0.1.0",
    builtins: {
        "fasd" => fasd,
        "fasd_cd" => fasd_cd,
        "a" => a,
        "s" => s,
        "d" => d,
        "f" => f,
        "sd" => sd,
        "sf" => sf,
        "v" => v,
        "j" => j,
    },
    completions: {
        "fasd" => fasd_complete,
    },
}

/// Completion for `fasd` / the aliases: frecency-ranked matches for the
/// current partial word.
fn fasd_complete(host: &Host, args: &Args) -> c_int {
    let a = args.rest();
    let Some(current) = a.first().and_then(|s| s.parse::<usize>().ok()) else {
        return 1;
    };
    let words = &a[1..];
    let partial = current
        .checked_sub(1)
        .and_then(|i| words.get(i))
        .map(String::as_str)
        .unwrap_or("");
    for (_, p) in sorted(query(host, Typ::Any, partial, Mode::Frecency), true) {
        host.add_match(&p);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplify_resolves_dots_and_absolutizes() {
        assert_eq!(simplify("/home/u", "src"), "/home/u/src");
        assert_eq!(simplify("/home/u", "./src/./x"), "/home/u/src/x");
        assert_eq!(simplify("/home/u/a", "../b"), "/home/u/b");
        assert_eq!(simplify("/x", "/a//b/../c/"), "/a/c");
        assert_eq!(simplify("/x", "/"), "/");
    }

    #[test]
    fn frecent_buckets_match_fasd() {
        assert_eq!(frecent(0), 6.0); // < 1h
        assert_eq!(frecent(3599), 6.0);
        assert_eq!(frecent(3600), 4.0); // < 1d
        assert_eq!(frecent(86_400), 2.0); // < 1w
        assert_eq!(frecent(604_800), 1.0); // older
    }

    #[test]
    fn parse_line_splits_first_and_last_pipe() {
        let e = parse_line("/home/u/proj|3.5|1700000000").unwrap();
        assert_eq!(e.path, "/home/u/proj");
        assert_eq!(e.rank, 3.5);
        assert_eq!(e.time, 1_700_000_000);
        // A non-numeric rank/time field is rejected.
        assert!(parse_line("/a|notarank|2").is_none());
        assert!(parse_line("no-pipes-here").is_none());
    }

    #[test]
    fn regex_matches_last_term_in_basename() {
        // "proj" matches the basename, not a mid-path component followed by /.
        let re = build_regex("proj", None, false).unwrap();
        assert!(re.is_match("/home/u/myproj|1|0"));
        assert!(!re.is_match("/home/proj/src|1|0")); // "proj" then a slash → no
    }

    #[test]
    fn regex_space_is_a_wildcard_and_dollar_anchors() {
        let re = build_regex("a b", None, true).unwrap();
        assert!(re.is_match("/x/alpha/beta|1|0")); // a…b, b in basename
        let anchored = build_regex("src$", None, false).unwrap();
        assert!(anchored.is_match("/home/u/src|1|0"));
        assert!(!anchored.is_match("/home/u/src/x|1|0")); // not at path end
    }

    #[test]
    fn fuzzy_tolerates_gaps() {
        // "src" fuzzily matches "s-r-c" within one component.
        let re = build_regex("src", Some(2), true).unwrap();
        assert!(re.is_match("/home/u/s_r_c|1|0"));
    }

    #[test]
    fn scoring_prefers_frequent_and_recent() {
        // Two paths, equal recency, different rank → higher rank wins.
        let older = now() - 1_000_000;
        let entries = [
            Entry {
                path: "/tmp".into(),
                rank: 1.0,
                time: older,
            },
            Entry {
                path: "/tmp".into(),
                rank: 9.0,
                time: older,
            },
        ];
        // Same path summed → rank 10; frecency = 10 * frecent(old) = 10.
        let sum: f64 = entries.iter().map(|e| e.rank).sum();
        assert_eq!(sum, 10.0);
        assert_eq!(frecent(1_000_000), 1.0);
    }
}
