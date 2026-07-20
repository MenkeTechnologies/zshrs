//! p10k EXTRA segments — user / host / date / public_ip / vpn_ip /
//! detect_virt / toolbox / xplr / docker_machine / dropbox / openfoam /
//! chruby / dotnet_version / php_version / swift_version /
//! laravel_version / terraform_version / symfony2_version /
//! rspec_stats / symfony2_tests.
//!
//! `~/forkedRepos/powerlevel10k/internal/p10k.zsh` is THE SPEC; ported
//! lines are cited as `// p10k:NNN`. Segment SEMANTICS (states, colors,
//! content, hide rules) mirror the zsh theme; the async worker layer
//! (`_p9k_worker_invoke` for public_ip / net_iface) collapses into
//! synchronous reads behind TTL caches, and `_p9k_cached_cmd` /
//! `_p9k_cache_stat_get` become a stat-keyed value cache — no external
//! tool ever runs uncached per prompt.
//!
//! Phase-1 scope notes (each also traced at the call site):
//! - public_ip: p10k fetches in a background worker; here the fetch
//!   runs synchronously on cache miss only, wrapped in the git.rs
//!   kill-on-budget subprocess pattern so a dead network can't wedge
//!   the prompt. Successes cache for PUBLIC_IP_TIMEOUT (p10k:1552),
//!   failures retry after 5s (p10k:1526). The single-pass method loop
//!   replaces p10k's doubled `$METHODS $METHODS` retry list
//!   (p10k:1527).
//! - vpn_ip: live getifaddrs scan per render (fork-free), replacing
//!   the async net_iface worker (p10k:5670-5692).
//! - rspec_stats / symfony2_tests: recursive globs run per prompt,
//!   exactly like the zsh originals (p10k:3212/3418).
//!
//! Shared-helper duplication: color1/color2/esc_pct/decode_g/seg_icon/
//! make_segment/cmd_on_path/up_ipv4_interfaces/is_ssh/is_root mirror
//! segments_core.rs / segments_sys.rs / segments_env.rs (private there;
//! this module may not edit other files). Hoisting them into a shared
//! submodule is a follow-up for the orchestrator.

use crate::extensions::p10k::config::{p9k_global, p9k_global_arr, p9k_param};
use crate::extensions::p10k::icons;
use crate::extensions::p10k::render::Segment;
use crate::ported::params::getsparam;
use crate::ported::utils::getkeystring;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

/// Segment dispatch per the p10k module contract. `None` = not handled
/// by this module; `Some(vec![])` = handled but hidden this prompt.
pub fn build_segment(name: &str) -> Option<Vec<Segment>> {
    match name {
        "user" => Some(user_segments()),                       // p10k:1636
        "host" => Some(host_segments()),                       // p10k:1662
        "date" => Some(date_segments()),                       // p10k:3500
        "public_ip" => Some(public_ip_segments()),             // p10k:1500
        "vpn_ip" => Some(vpn_ip_segments()),                   // p10k:2301
        "detect_virt" => Some(detect_virt_segments()),         // p10k:2274
        "toolbox" => Some(toolbox_segments()),                 // p10k:1676
        "xplr" => Some(xplr_segments()),                       // p10k:4897
        "docker_machine" => Some(docker_machine_segments()),   // p10k:2175
        "dropbox" => Some(dropbox_segments()),                 // p10k:4540
        "openfoam" => Some(openfoam_segments()),               // p10k:4411
        "chruby" => Some(chruby_segments()),                   // p10k:3121
        "dotnet_version" => Some(dotnet_version_segments()),   // p10k:2649
        "php_version" => Some(php_version_segments()),         // p10k:2673
        "swift_version" => Some(swift_version_segments()),     // p10k:4425
        "laravel_version" => Some(laravel_version_segments()), // p10k:2323
        "terraform_version" => Some(terraform_version_segments()), // p10k:4957
        "symfony2_version" => Some(symfony2_version_segments()), // p10k:3427
        "rspec_stats" => Some(rspec_stats_segments()),         // p10k:3210
        "symfony2_tests" => Some(symfony2_tests_segments()),   // p10k:3416
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Shared helpers (mirror segments_core.rs / segments_sys.rs — module doc)
// ---------------------------------------------------------------------

/// p10k:8390-8396 — `[[ $_POWERLEVEL9K_COLOR_SCHEME == light ]] &&
/// _p9k_color1=7 || _p9k_color1=0`.
fn color1() -> &'static str {
    if p9k_global("COLOR_SCHEME", "dark") == "light" {
        "7"
    } else {
        "0"
    }
}

/// p10k:8392/8395 — `_p9k_color2`: the inverse of color1.
fn color2() -> &'static str {
    if p9k_global("COLOR_SCHEME", "dark") == "light" {
        "0"
    } else {
        "7"
    }
}

/// Read a parameter, falling back to the process environment (covers
/// early-startup renders before exports land in the paramtab).
fn env_or_param(name: &str) -> String {
    if let Some(v) = getsparam(name) {
        return v;
    }
    std::env::var(name).unwrap_or_default()
}

/// `_p9k_declare -b` read semantics (p10k:141-151): ONLY the literal
/// string `true` is truthy; unset uses the declared default.
fn global_bool(name: &str, default: bool) -> bool {
    match getsparam(&format!("POWERLEVEL9K_{name}")) {
        Some(v) => v == "true",
        None => default,
    }
}

/// `_p9k_declare -F` read: unset/empty/unparseable → default.
fn global_float(name: &str, default: f64) -> f64 {
    getsparam(&format!("POWERLEVEL9K_{name}"))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(default)
}

/// Escape `%` for prompt-expansion contexts — p10k's ubiquitous
/// `${x//\%/%%}`.
fn esc_pct(s: &str) -> String {
    s.replace('%', "%%")
}

/// zsh `${(g::)x}` echo-style escape decoding, as p10k applies to
/// user-supplied icons/templates (p10k:524 and every `_p9k_declare -e`).
fn decode_g(s: &str) -> String {
    getkeystring(s).0
}

/// Port of `_p9k_get_icon $1 $2` (p10k:511-530) — identical to
/// segments_core::seg_icon: probe the user-param chain for `<KEY>`; on
/// a hit apply `(g::)` decoding (p10k:524) plus the backspace-wrap
/// quirk (p10k:525); on a whole-chain miss the mode icon table answers.
fn seg_icon(segment: &str, state: Option<&str>, key: &str) -> String {
    let probed = p9k_param(segment, state, key, "\u{1}");
    if probed == "\u{1}" {
        return icons::icon(key).to_string();
    }
    let decoded = decode_g(&probed);
    // p10k:525 — [[ $ret != $'\b'? ]] || ret="%{$ret%}"
    let mut ch = decoded.chars();
    if ch.next() == Some('\u{8}') && ch.next().is_some() && ch.next().is_none() {
        return format!("%{{{decoded}%}}");
    }
    decoded
}

/// VISUAL_IDENTIFIER_EXPANSION hook (p10k:720/951) — identical to
/// segments_core::apply_visual_identifier.
fn apply_visual_identifier(segment: &str, state: Option<&str>, icon: String) -> Option<String> {
    let exp = p9k_param(
        segment,
        state,
        "VISUAL_IDENTIFIER_EXPANSION",
        "${P9K_VISUAL_IDENTIFIER}",
    );
    let resolved = if exp == "${P9K_VISUAL_IDENTIFIER}" {
        icon
    } else if !exp.contains('$') {
        exp
    } else {
        tracing::debug!(
            target: "p10k",
            segment,
            ?state,
            %exp,
            "dynamic VISUAL_IDENTIFIER_EXPANSION unported — using resolved icon"
        );
        icon
    };
    if resolved.is_empty() {
        None
    } else {
        Some(resolved)
    }
}

/// CONTENT_EXPANSION hook (p10k:724/955) — identical to
/// segments_core::apply_content_expansion.
fn apply_content_expansion(segment: &str, state: Option<&str>, content: String) -> String {
    let exp = p9k_param(segment, state, "CONTENT_EXPANSION", "${P9K_CONTENT}");
    if exp == "${P9K_CONTENT}" {
        return content;
    }
    if !exp.contains('$') {
        return exp;
    }
    if exp.contains("${P9K_CONTENT}") {
        let out = exp.replace("${P9K_CONTENT}", &content);
        if out.contains('$') {
            tracing::debug!(
                target: "p10k",
                segment, ?state, %exp,
                "CONTENT_EXPANSION has unevaluated expansions beyond ${{P9K_CONTENT}}"
            );
        }
        return out;
    }
    tracing::debug!(
        target: "p10k",
        segment, ?state, %exp,
        "dynamic CONTENT_EXPANSION unported — using segment content"
    );
    content
}

/// Common constructor mirroring `_p9k_prompt_segment name bg fg icon
/// expand cond content` — identical to segments_core::make_segment
/// (p10k:1101 + the color/icon/expansion hooks of
/// _p9k_left_prompt_segment).
fn make_segment(
    name: &str,
    state: Option<&str>,
    default_bg: &str,
    default_fg: &str,
    icon_key: &str,
    content: String,
) -> Segment {
    let bg = p9k_param(name, state, "BACKGROUND", default_bg);
    let fg = p9k_param(name, state, "FOREGROUND", default_fg);
    let icon_glyph = if icon_key.is_empty() {
        String::new()
    } else {
        seg_icon(name, state, icon_key)
    };
    let icon = apply_visual_identifier(name, state, icon_glyph);
    let content = apply_content_expansion(name, state, content);
    Segment {
        name: name.to_string(),
        state: state.map(|s| s.to_string()),
        content,
        icon,
        fg,
        bg,
    }
}

/// `$commands[name]` — locate an executable on $PATH (uncached; every
/// caller sits behind a TTL/stat cache or a cheap short-circuit).
/// Mirrors segments_env::have_cmd's scan.
fn cmd_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env_or_param("PATH");
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let cand = Path::new(dir).join(name);
        let is_exec = std::fs::metadata(&cand)
            .map(|md| {
                use std::os::unix::fs::PermissionsExt;
                md.is_file() && md.permissions().mode() & 0o111 != 0
            })
            .unwrap_or(false);
        if is_exec {
            return Some(cand);
        }
    }
    None
}

/// p10k:203-210 — `_p9k_fetch_cwd`: logical $PWD when absolute, else
/// the process cwd (mirrors segments_env::cwd).
fn cwd() -> PathBuf {
    let pwd = env_or_param("PWD");
    if !pwd.is_empty() {
        let p = PathBuf::from(&pwd);
        if p.is_absolute() {
            return p;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

/// p10k:8211-8214 — `[[ -n $SSH_CLIENT || -n $SSH_TTY || -n
/// $SSH_CONNECTION ]] && P9K_SSH=1` (mirrors segments_core::is_ssh;
/// the `who`-based su-after-ssh fallback is unported there too).
fn is_ssh() -> bool {
    !env_or_param("SSH_CLIENT").is_empty()
        || !env_or_param("SSH_TTY").is_empty()
        || !env_or_param("SSH_CONNECTION").is_empty()
}

/// Prompt-escape `%#` root test — geteuid()==0 (Src/prompt.c '#';
/// mirrors segments_core::is_root).
fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// `[[ -r $path ]]` — access(2) R_OK, matching the zsh -r condition
/// (the -w twin lives in segments_core::path_writable).
fn path_readable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    match std::ffi::CString::new(path.as_os_str().as_bytes()) {
        Ok(c) => unsafe { libc::access(c.as_ptr(), libc::R_OK) == 0 },
        Err(_) => false,
    }
}

/// `${(%):-%m}` — hostname up to the first dot (Src/prompt.c %m). zsh
/// keeps $HOST in sync with gethostname; probe the param first.
fn hostname_short() -> String {
    let host = env_or_param("HOST");
    if !host.is_empty() {
        return host.split('.').next().unwrap_or("").to_string();
    }
    let mut buf = [0u8; 256];
    if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) } != 0 {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let full = String::from_utf8_lossy(&buf[..end]).into_owned();
    full.split('.').next().unwrap_or("").to_string()
}

/// All (interface, IPv4) pairs for interfaces that are UP — mirror of
/// segments_core::up_ipv4_interfaces (private there): the getifaddrs
/// equivalent of p10k's `ifconfig` parse (p10k:5674-5682: flags word
/// odd = IFF_UP, first `inet` line per iface). Fork-free.
fn up_ipv4_interfaces() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        tracing::warn!(target: "p10k", "getifaddrs failed for vpn_ip/public_ip scan");
        return out;
    }
    let mut cur = ifap;
    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        // p10k:5677 — `[[ $match[2] == *[13579bdfBDF] ]]` = IFF_UP set.
        if ifa.ifa_flags & (libc::IFF_UP as u32) == 0 {
            continue;
        }
        if ifa.ifa_addr.is_null() {
            continue;
        }
        let sa = unsafe { &*ifa.ifa_addr };
        // p10k:5678 — only `inet` (IPv4) lines are collected.
        if i32::from(sa.sa_family) != libc::AF_INET {
            continue;
        }
        let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
        let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)).to_string();
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        // p10k keeps the FIRST inet address per interface (iface is
        // cleared after the first match, p10k:5680).
        if !out.iter().any(|(n, _)| *n == name) {
            out.push((name, ip));
        }
    }
    unsafe { libc::freeifaddrs(ifap) };
    out
}

/// p10k:5654-5663 — `_p9k_prompt_net_iface_match`: interfaces matched
/// with `^($pattern)$` as an ERE (`=~`); returns the matching pairs.
fn match_interfaces(pattern: &str) -> Vec<(String, String)> {
    if pattern.is_empty() {
        return Vec::new();
    }
    let re = match regex::Regex::new(&format!("^({pattern})$")) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(target: "p10k", %pattern, %e, "bad interface regex");
            return Vec::new();
        }
    };
    up_ipv4_interfaces()
        .into_iter()
        .filter(|(name, _)| re.is_match(name))
        .collect()
}

// ---------------------------------------------------------------------
// Subprocess: TTL cache + budgeted runner + stat-keyed command cache
// ---------------------------------------------------------------------

/// segments_sys::cached_ttl variant with String keys and a
/// producer-chosen TTL: public_ip caches successes for
/// PUBLIC_IP_TIMEOUT but failures for only 5s (p10k:1526/1552), and
/// dropbox keys on the cwd. Stored Instant is the EXPIRY. `None`
/// results (spawn/parse failure) are cached too, so a broken tool
/// can't fork on every prompt.
static TTL_CACHE: OnceLock<Mutex<HashMap<String, (Instant, Option<String>)>>> = OnceLock::new();

fn cached_ttl(key: &str, run: impl FnOnce() -> (Duration, Option<String>)) -> Option<String> {
    let m = TTL_CACHE.get_or_init(Default::default);
    if let Ok(guard) = m.lock() {
        if let Some((expiry, val)) = guard.get(key) {
            if Instant::now() < *expiry {
                return val.clone();
            }
        }
    }
    let (ttl, val) = run();
    let expiry = Instant::now()
        .checked_add(ttl)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
    if let Ok(mut guard) = m.lock() {
        guard.insert(key.to_string(), (expiry, val.clone()));
    }
    val
}

/// Run a tool with a hard latency budget — the git.rs run_porcelain
/// pattern (git.rs:349-391): stdout drained on a helper thread, child
/// killed and reaped on overrun so a hung network tool can't wedge the
/// prompt. `require_success=false` mirrors plain `$(cmd)` capture
/// (systemd-detect-virt exits 1 with output "none", p10k:2275;
/// dropbox-cli prints its status regardless of exit, p10k:4542).
fn run_tool_budget(
    bin: &Path,
    args: &[&str],
    dir: Option<&Path>,
    budget: Duration,
    require_success: bool,
) -> Option<String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(target: "p10k", bin = %bin.display(), %e, "tool spawn failed");
            return None;
        }
    };
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        use std::io::Read;
        let _ = stdout.read_to_end(&mut out);
        let _ = tx.send(out);
    });
    match rx.recv_timeout(budget) {
        Ok(out) => {
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            if require_success && !ok {
                return None;
            }
            Some(String::from_utf8_lossy(&out).into_owned())
        }
        Err(_) => {
            tracing::debug!(
                target: "p10k",
                bin = %bin.display(), ?budget,
                "tool exceeded budget — killed"
            );
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// mtime in epoch seconds; -1 when the path can't be stat'd — mirrors
/// `zstat -A stat +mtime -- $1 2>/dev/null || stat=(-1)` (p10k:187-188;
/// identical to segments_env::mtime).
fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(-1)
}

type StatSig = Vec<(PathBuf, i64)>;

/// Stat-keyed value cache — port of `_p9k_cache_stat_get` /
/// `_p9k_cache_stat_set` (mirrors segments_env's STAT_CACHE): a cached
/// value stays valid while every witness file keeps its mtime (missing
/// file == mtime -1, also a valid, comparable state).
static STAT_CACHE: OnceLock<Mutex<HashMap<String, (StatSig, Option<String>)>>> = OnceLock::new();

fn stat_cached(
    key: &str,
    files: &[PathBuf],
    run: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let sig: StatSig = files.iter().map(|f| (f.clone(), mtime(f))).collect();
    let m = STAT_CACHE.get_or_init(Default::default);
    if let Ok(guard) = m.lock() {
        if let Some((cached_sig, val)) = guard.get(key) {
            if *cached_sig == sig {
                return val.clone();
            }
        }
    }
    let val = run();
    if let Ok(mut guard) = m.lock() {
        guard.insert(key.to_string(), (sig, val.clone()));
    }
    val
}

/// Port of `_p9k_cached_cmd 0 '' cmd args...` (p10k:2416-2431; mirrors
/// segments_env::cached_cmd): run the tool once, cache the output
/// invalidated by the binary's mtime; only a successful run with
/// output yields a value.
fn cached_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    // p10k:2417 — `local cmd=$commands[$3]; [[ -n $cmd ]] || return`
    let bin = cmd_on_path(cmd)?;
    let key = format!("cached_cmd {cmd} {}", args.join(" "));
    let out = stat_cached(&key, &[bin.clone()], || {
        // p10k:2419-2426 — run once; `$( )` strips trailing newlines.
        run_tool_budget(&bin, args, None, Duration::from_secs(5), true)
            .map(|s| s.trim_end_matches('\n').to_string())
    })?;
    // p10k:2429-2430 — empty output is a miss.
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

// ---------------------------------------------------------------------
// Upglob helpers (mirror segments_env::upfind / upfind_pred /
// dir_has_ext — private there)
// ---------------------------------------------------------------------

/// Port of `_p9k_upglob` (p10k:265-292) for a literal file name: walk
/// from $PWD to `/`, first ancestor directory containing `name`.
/// `None` == zsh `_p9k_upglob X && return` == "not found → hidden".
fn upfind(name: &str) -> Option<PathBuf> {
    upfind_pred(&|dir: &Path| dir.join(name).exists())
}

/// Predicate flavor for multi-pattern upglobs
/// (`project.json|*.csproj|...`).
fn upfind_pred(pred: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    let start = cwd();
    let mut dir: &Path = &start;
    loop {
        if pred(dir) {
            return Some(dir.to_path_buf());
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return None,
        }
    }
}

/// True when any entry of `dir` has one of `exts` as its extension —
/// the `*.(csproj|…)` half of glob-style upglobs. Without GLOB_DOTS a
/// leading dot never matches.
fn dir_has_ext(dir: &Path, exts: &[&str]) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        if let Some(ext) = name.rsplit('.').next() {
            if name.contains('.') && exts.contains(&ext) {
                return true;
            }
        }
    }
    false
}

/// Recursive `dir/**/*<suffix>(N)` — matched entries collected as
/// paths relative to `base`. Glob semantics without GLOB_DOTS: dot
/// entries are neither matched nor descended; `**/` does not follow
/// symlinked directories (that would be `***/`).
fn glob_recursive(base: &Path, dir: &Path, suffix: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name_s = name.to_string_lossy().into_owned();
        if name_s.starts_with('.') {
            continue;
        }
        let path = entry.path();
        // DirEntry::file_type does not follow symlinks — symlinked
        // dirs are matched but not descended, like zsh `**/`.
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            glob_recursive(base, &path, suffix, out);
        }
        if name_s.ends_with(suffix) {
            out.push(path.strip_prefix(base).unwrap_or(&path).to_path_buf());
        }
    }
}

// ---------------------------------------------------------------------
// Pure extractors (ported; unit-tested below)
// ---------------------------------------------------------------------

/// p10k:2678 — `[[ $_p9k__ret == (#b)(*$'\n')#'PHP '([[:digit:].]##)* ]]`:
/// zero or more complete lines, then a line starting `PHP ` followed
/// immediately by a digit/dot run (backtracks past `PHP `-lines with no
/// digits, like the zsh pattern).
fn php_version_extract(out: &str) -> Option<String> {
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("PHP ") {
            let run: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if !run.is_empty() {
                return Some(run);
            }
        }
    }
    None
}

/// p10k:4427 — `[[ $_p9k__ret == (#b)[^[:digit:]]#([[:digit:].]##)* ]]`:
/// skip leading non-digits (dots included in the skip class), capture
/// the digit/dot run starting at the first digit.
fn swift_version_extract(out: &str) -> Option<String> {
    let start = out.find(|c: char| c.is_ascii_digit())?;
    Some(
        out[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect(),
    )
}

/// p10k:4959-4962 — `v=${_p9k__ret#Terraform v}` (prefix must strip:
/// `(( $#v < $#_p9k__ret )) || return`), first line, non-empty.
fn terraform_version_extract(out: &str) -> Option<String> {
    let v = out.strip_prefix("Terraform v")?; // p10k:4959-4960
    let v = v.split('\n').next().unwrap_or(v); // p10k:4961 — ${v%%$'\n'*}
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    } // p10k:4962
}

/// p10k:2330 — `${${(M)v:#Laravel Framework *}#Laravel Framework }`:
/// the whole output must match `Laravel Framework *`, prefix stripped.
fn laravel_version_extract(v: &str) -> Option<String> {
    v.strip_prefix("Laravel Framework ").map(str::to_string)
}

/// p10k:3429 — `${$(grep -F " VERSION " app/bootstrap.php.cache)//[![:digit:].]}`:
/// all lines containing ` VERSION `, everything but digits and dots
/// removed (newlines between matches vanish too).
fn symfony2_version_extract(content: &str) -> String {
    content
        .lines()
        .filter(|l| l.contains(" VERSION "))
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect()
}

/// Port of `_p9k_build_test_stats` (p10k:3436-3447): (state, bg,
/// content) for a code/tests ratio; None when there is no code.
fn test_stats(
    code: usize,
    tests: usize,
    headline: &str,
) -> Option<(&'static str, &'static str, String)> {
    if code == 0 {
        return None; // p10k:3441 — (( code_amount > 0 )) || return
    }
    // p10k:3442 — local -F 2 ratio: two fixed decimals on expansion.
    let ratio = 100.0 * tests as f64 / code as f64;
    let (state, bg) = if ratio >= 75.0 {
        ("GOOD", "cyan") // p10k:3444
    } else if ratio >= 50.0 {
        ("AVG", "yellow") // p10k:3445
    } else {
        ("BAD", "red") // p10k:3446
    };
    Some((state, bg, format!("{headline}: {ratio:.2}%%")))
}

/// p10k:1550 — `[[ $ip =~ '^[0-9a-f.:]+$' ]] || ip=''`.
fn valid_public_ip(ip: &str) -> bool {
    !ip.is_empty()
        && ip
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'.' | b':'))
}

/// p10k:4542 — `dropbox-cli filestatus . | cut -d\  -f2-`: first line,
/// first space-delimited field cut (a line with no delimiter passes
/// through whole, like cut without -s).
fn dropbox_cut_status(out: &str) -> String {
    let first = out.lines().next().unwrap_or("");
    match first.split_once(' ') {
        Some((_, rest)) => rest.to_string(),
        None => first.to_string(),
    }
}

/// p10k:8236-8238 — `name=(${(Q)${${(@M)${(f)"$(</run/.containerenv)"}:#name=*}#name=}})`:
/// lines starting `name=`, value unquoted; exactly ONE match required
/// and it must be non-empty.
fn containerenv_toolbox_name(content: &str) -> Option<String> {
    let mut names = content.lines().filter_map(|l| l.strip_prefix("name="));
    let name = names.next()?;
    if names.next().is_some() {
        return None; // p10k:8237 — [[ ${#name} -eq 1 ]]
    }
    // ${(Q)} — strip one level of shell quoting ("..."/'...').
    let unq = if name.len() >= 2
        && ((name.starts_with('"') && name.ends_with('"'))
            || (name.starts_with('\'') && name.ends_with('\'')))
    {
        &name[1..name.len() - 1]
    } else {
        name
    };
    if unq.is_empty() {
        None
    } else {
        Some(unq.to_string())
    }
}

// ---------------------------------------------------------------------
// user (p10k:1636-1658)
// ---------------------------------------------------------------------

fn user_segments() -> Vec<Segment> {
    // ${(%):-%n} — current (effective) username special.
    let user = getsparam("USERNAME").unwrap_or_default();
    // p10k:1654-1658 _p9k_prompt_user_init — hidden when the user
    // matches DEFAULT_USER and ALWAYS_SHOW_USER is off (p10k:7303
    // declares -b ALWAYS_SHOW_USER 0).
    if !global_bool("ALWAYS_SHOW_USER", false) && user == env_or_param("DEFAULT_USER") {
        return vec![];
    }
    // p10k:7305 — _p9k_declare -e POWERLEVEL9K_USER_TEMPLATE "%n".
    let template = decode_g(&p9k_global("USER_TEMPLATE", "%n"));
    if is_root() {
        // p10k:1638 — `${0}_ROOT color1 yellow ROOT_ICON` shown when
        // `%#` is '#' (cond '${${(%):-%#}:#\%}').
        vec![make_segment(
            "user",
            Some("ROOT"),
            color1(),
            "yellow",
            "ROOT_ICON",
            template,
        )]
    } else if !env_or_param("SUDO_COMMAND").is_empty() {
        // p10k:1639-1640 — `${0}_SUDO color1 yellow SUDO_ICON` with the
        // template, shown when non-root.
        vec![make_segment(
            "user",
            Some("SUDO"),
            color1(),
            "yellow",
            "SUDO_ICON",
            template,
        )]
    } else {
        // p10k:1642 — `${0}_DEFAULT color1 yellow USER_ICON … "%n"`
        // (literal %n, not the template).
        vec![make_segment(
            "user",
            Some("DEFAULT"),
            color1(),
            "yellow",
            "USER_ICON",
            "%n".to_string(),
        )]
    }
}

// ---------------------------------------------------------------------
// host (p10k:1662-1670)
// ---------------------------------------------------------------------

fn host_segments() -> Vec<Segment> {
    // p10k:7306 — _p9k_declare -e POWERLEVEL9K_HOST_TEMPLATE "%m".
    let template = decode_g(&p9k_global("HOST_TEMPLATE", "%m"));
    if is_ssh() {
        // p10k:1665 — `$0_REMOTE color1 yellow SSH_ICON`.
        vec![make_segment(
            "host",
            Some("REMOTE"),
            color1(),
            "yellow",
            "SSH_ICON",
            template,
        )]
    } else {
        // p10k:1667 — `$0_LOCAL color1 yellow HOST_ICON`.
        vec![make_segment(
            "host",
            Some("LOCAL"),
            color1(),
            "yellow",
            "HOST_ICON",
            template,
        )]
    }
}

// ---------------------------------------------------------------------
// date (p10k:3500-3508)
// ---------------------------------------------------------------------

fn date_segments() -> Vec<Segment> {
    // p10k:7477 — _p9k_declare -e POWERLEVEL9K_DATE_FORMAT "%D{%d.%m.%y}".
    let fmt = decode_g(&p9k_global("DATE_FORMAT", "%D{%d.%m.%y}"));
    // p10k:3501-3507 — `${${(%)_POWERLEVEL9K_DATE_FORMAT}//\%/%%}`
    // snapshots the expansion at precmd; this engine rebuilds PROMPT
    // every precmd, so passing the escape through IS the precmd
    // snapshot (same passthrough as segments_sys::time_segments).
    // p10k:3508 — `$0 "$_p9k_color2" "$_p9k_color1" "DATE_ICON" 0 '' …`
    vec![make_segment(
        "date",
        None,
        color2(),
        color1(),
        "DATE_ICON",
        fmt,
    )]
}

// ---------------------------------------------------------------------
// public_ip (p10k:1500-1567)
// ---------------------------------------------------------------------

/// One fetch attempt per method (p10k:1528-1549). `$( )` strips
/// trailing newlines; validation is p10k:1550.
fn fetch_public_ip(method: &str, host: &str) -> Option<String> {
    let fetched = match method {
        "dig" => {
            // p10k:1530-1537 — -4 A first, then -6 AAAA; a ';'-prefixed
            // answer (dig comment/error) counts as empty.
            let bin = cmd_on_path("dig")?;
            let dig = |family: &str, rr: &str| {
                run_tool_budget(
                    &bin,
                    &[
                        "+tries=1",
                        "+short",
                        family,
                        rr,
                        "myip.opendns.com",
                        "@resolver1.opendns.com",
                    ],
                    None,
                    Duration::from_secs(6),
                    true,
                )
                .map(|s| s.trim_end_matches('\n').to_string())
                .filter(|s| !s.is_empty() && !s.starts_with(';')) // p10k:1532/1535
            };
            dig("-4", "A").or_else(|| dig("-6", "AAAA"))
        }
        // p10k:1539-1542 — `curl --max-time 5 -w '\n' $HOST`.
        "curl" => run_tool_budget(
            &cmd_on_path("curl")?,
            &["--max-time", "5", "-w", "\n", host],
            None,
            Duration::from_secs(6),
            true,
        )
        .map(|s| s.trim_end_matches('\n').to_string()),
        // p10k:1544-1547 — `wget -T 5 -qO- $HOST`.
        "wget" => run_tool_budget(
            &cmd_on_path("wget")?,
            &["-T", "5", "-qO-", host],
            None,
            Duration::from_secs(6),
            true,
        )
        .map(|s| s.trim_end_matches('\n').to_string()),
        _ => None,
    }?;
    // p10k:1550 — [[ $ip =~ '^[0-9a-f.:]+$' ]] || ip=''
    if valid_public_ip(&fetched) {
        Some(fetched)
    } else {
        None
    }
}

fn public_ip_segments() -> Vec<Segment> {
    // p10k:1519 + 1552/1526 — a fresh value is fetched only when
    // EPOCHREALTIME passes _p9k__public_ip_next_time: success extends
    // by PUBLIC_IP_TIMEOUT (p10k:7296 — declare -F, default 300),
    // failure retries after start+5.
    let timeout = match global_float("PUBLIC_IP_TIMEOUT", 300.0) {
        // NaN would panic Duration::from_secs_f64; clamp keeps the TTL sane.
        t if t.is_finite() => t.clamp(5.0, 31_536_000.0),
        _ => 300.0,
    };
    let ip = cached_ttl("public_ip", || {
        // p10k:7297 — declare -a PUBLIC_IP_METHODS (dig curl wget).
        let mut methods = p9k_global_arr("PUBLIC_IP_METHODS");
        if methods.is_empty() {
            methods = vec!["dig".into(), "curl".into(), "wget".into()];
        }
        // p10k:7299 — declare -s PUBLIC_IP_HOST "https://v4.ident.me/".
        let host = p9k_global("PUBLIC_IP_HOST", "https://v4.ident.me/");
        // p10k:1527 iterates `$METHODS $METHODS`; single pass here
        // (module doc) — the 5s failure TTL is the retry.
        for method in &methods {
            if let Some(ip) = fetch_public_ip(method, &host) {
                return (Duration::from_secs_f64(timeout), Some(ip)); // p10k:1552
            }
        }
        (Duration::from_secs(5), None) // p10k:1526 — next = start + 5
    })
    .unwrap_or_default();
    // p10k:1502 — '${_p9k__public_ip:-$_POWERLEVEL9K_PUBLIC_IP_NONE}';
    // p10k:7298 — declare -e PUBLIC_IP_NONE "".
    let text = if ip.is_empty() {
        decode_g(&p9k_global("PUBLIC_IP_NONE", ""))
    } else {
        ip
    };
    if text.is_empty() {
        return vec![]; // empty cond — hidden
    }
    // p10k:1503-1507 — with PUBLIC_IP_VPN_INTERFACE set, the icon
    // flips to VPN_ICON while an up interface matches the pattern.
    let vpn_pat = p9k_global("PUBLIC_IP_VPN_INTERFACE", ""); // p10k:7300
    let icon_key = if !vpn_pat.is_empty() && !match_interfaces(&vpn_pat).is_empty() {
        "VPN_ICON" // p10k:1505
    } else {
        "PUBLIC_IP_ICON" // p10k:1504/1507
    };
    // p10k:1504-1507 — `"$0" "$_p9k_color1" "$_p9k_color2" <icon> …`
    vec![make_segment(
        "public_ip",
        None,
        color1(),
        color2(),
        icon_key,
        esc_pct(&text),
    )]
}

// ---------------------------------------------------------------------
// vpn_ip (p10k:2301-2319 + net iface scan p10k:5748-5756)
// ---------------------------------------------------------------------

fn vpn_ip_segments() -> Vec<Segment> {
    // p10k:7401 — declare -s VPN_IP_INTERFACE "(gpd|wg|(.*tun)|tailscale)[0-9]*".
    let pattern = p9k_global("VPN_IP_INTERFACE", "(gpd|wg|(.*tun)|tailscale)[0-9]*");
    if pattern.is_empty() {
        return vec![];
    }
    let matched = match_interfaces(&pattern);
    // p10k:5748-5756 — SHOW_ALL (default 0, p10k:7404) keeps every
    // matching IP, else the first only.
    let ips: Vec<String> = if global_bool("VPN_IP_SHOW_ALL", false) {
        matched.into_iter().map(|(_, ip)| ip).collect()
    } else {
        matched.into_iter().take(1).map(|(_, ip)| ip).collect()
    };
    // p10k:2314-2316 — one segment per IP:
    // `prompt_vpn_ip "cyan" "$_p9k_color1" 'VPN_ICON' 0 '' $ip`.
    ips.into_iter()
        .map(|ip| make_segment("vpn_ip", None, "cyan", color1(), "VPN_ICON", ip))
        .collect()
}

// ---------------------------------------------------------------------
// detect_virt (p10k:2274-2289)
// ---------------------------------------------------------------------

fn detect_virt_segments() -> Vec<Segment> {
    // p10k:2288 — init cond '$commands[systemd-detect-virt]': absent
    // tool (macOS) hides the segment.
    let Some(bin) = cmd_on_path("systemd-detect-virt") else {
        return vec![];
    };
    // p10k:2275 — `virt="$(systemd-detect-virt 2>/dev/null)"`: output
    // captured regardless of exit status (the tool exits 1 with
    // output "none"). 30s TTL replaces the per-prompt fork.
    let mut virt = cached_ttl("detect_virt", || {
        (
            Duration::from_secs(30),
            run_tool_budget(&bin, &[], None, Duration::from_secs(2), false)
                .map(|s| s.trim_end_matches('\n').to_string()),
        )
    })
    .unwrap_or_default();
    if virt == "none" {
        // p10k:2277-2280 — root inode != 2 means chroot.
        use std::os::unix::fs::MetadataExt;
        let inode = std::fs::metadata("/").map(|m| m.ino()).unwrap_or(2);
        if inode != 2 {
            virt = "chroot".to_string();
        }
    }
    // p10k:2282-2284 — shown only when non-empty.
    if virt.is_empty() {
        return vec![];
    }
    // p10k:2283 — `"$0" "$_p9k_color1" "yellow" '' 0 '' "${virt//\%/%%}"`
    vec![make_segment(
        "detect_virt",
        None,
        color1(),
        "yellow",
        "",
        esc_pct(&virt),
    )]
}

// ---------------------------------------------------------------------
// toolbox (p10k:1676-1686 + _p9k_init_toolbox p10k:8233-8245)
// ---------------------------------------------------------------------

/// Port of `_p9k_init_toolbox` (p10k:8233-8245), computed once per
/// process exactly like the zsh init (called once at p10k:9186).
fn toolbox_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        // p10k:8234 — an inherited P9K_TOOLBOX_NAME wins.
        let preset = env_or_param("P9K_TOOLBOX_NAME");
        if !preset.is_empty() {
            return preset;
        }
        // p10k:8235-8238 — /run/.containerenv `name=` line.
        if let Ok(content) = std::fs::read_to_string("/run/.containerenv") {
            if let Some(name) = containerenv_toolbox_name(&content) {
                return name;
            }
        }
        // p10k:8239-8243 — distrobox: hostname starts with $NAME.
        if !env_or_param("DISTROBOX_ENTER_PATH").is_empty() {
            let nm = env_or_param("NAME");
            if !nm.is_empty() {
                let host = hostname_short(); // ${(%):-%m}
                if host.starts_with(&nm) {
                    return host;
                }
            }
        }
        String::new()
    })
}

fn toolbox_segments() -> Vec<Segment> {
    // p10k:1681 — cond '$P9K_TOOLBOX_NAME'.
    let name = toolbox_name();
    if name.is_empty() {
        return vec![];
    }
    // p10k:1677 — `$0 $_p9k_color1 yellow TOOLBOX_ICON 0 '' $P9K_TOOLBOX_NAME`
    vec![make_segment(
        "toolbox",
        None,
        color1(),
        "yellow",
        "TOOLBOX_ICON",
        name.to_string(),
    )]
}

// ---------------------------------------------------------------------
// xplr (p10k:4897-4905)
// ---------------------------------------------------------------------

fn xplr_segments() -> Vec<Segment> {
    // p10k:4904 — cond '$XPLR_PID'.
    if env_or_param("XPLR_PID").is_empty() {
        return vec![];
    }
    // p10k:4899 — `$0 6 $_p9k_color1 XPLR_ICON 0 '' ''`
    vec![make_segment(
        "xplr",
        None,
        "6",
        color1(),
        "XPLR_ICON",
        String::new(),
    )]
}

// ---------------------------------------------------------------------
// docker_machine (p10k:2175-2181)
// ---------------------------------------------------------------------

fn docker_machine_segments() -> Vec<Segment> {
    // p10k:2180 — cond '$DOCKER_MACHINE_NAME'.
    let name = env_or_param("DOCKER_MACHINE_NAME");
    if name.is_empty() {
        return vec![];
    }
    // p10k:2176 — `"$0" "magenta" "$_p9k_color1" 'SERVER_ICON' 0 ''
    // "${DOCKER_MACHINE_NAME//\%/%%}"`
    vec![make_segment(
        "docker_machine",
        None,
        "magenta",
        color1(),
        "SERVER_ICON",
        esc_pct(&name),
    )]
}

// ---------------------------------------------------------------------
// dropbox (p10k:4540-4557)
// ---------------------------------------------------------------------

fn dropbox_segments() -> Vec<Segment> {
    // p10k:4556 — init cond '$commands[dropbox-cli]'.
    let Some(bin) = cmd_on_path("dropbox-cli") else {
        return vec![];
    };
    let dir = cwd();
    // p10k:4542 — `dropbox-cli filestatus .` runs in the shell's cwd
    // every prompt in zsh; 10s TTL keyed on the cwd here (hard rule:
    // no uncached per-prompt forks).
    let key = format!("dropbox {}", dir.display());
    let Some(status) = cached_ttl(&key, || {
        (
            Duration::from_secs(10),
            run_tool_budget(
                &bin,
                &["filestatus", "."],
                Some(&dir),
                Duration::from_secs(2),
                false,
            )
            .map(|out| dropbox_cut_status(&out)),
        )
    }) else {
        return vec![];
    };
    // p10k:4545 — only when the folder is tracked and dropbox runs.
    if status == "unwatched" || status == "isn't running!" {
        return vec![];
    }
    // p10k:4547-4549 — "up to date" collapses to the icon only
    // (`=~` substring match).
    let content = if status.contains("up to date") {
        String::new()
    } else {
        esc_pct(&status)
    };
    // p10k:4551 — `"$0" "white" "blue" "DROPBOX_ICON" 0 '' …`
    vec![make_segment(
        "dropbox",
        None,
        "white",
        "blue",
        "DROPBOX_ICON",
        content,
    )]
}

// ---------------------------------------------------------------------
// openfoam (p10k:4411-4421)
// ---------------------------------------------------------------------

fn openfoam_segments() -> Vec<Segment> {
    // p10k:4420 — cond '$WM_PROJECT_VERSION'.
    let version = env_or_param("WM_PROJECT_VERSION");
    if version.is_empty() {
        return vec![];
    }
    // `${WM_PROJECT_VERSION:t}` — path tail.
    let tail = version.rsplit('/').next().unwrap_or(&version);
    // p10k:4412-4416 — WM_FORK empty → "OF:", else "F-X:".
    let content = if env_or_param("WM_FORK").is_empty() {
        format!("OF: {}", esc_pct(tail)) // p10k:4413
    } else {
        format!("F-X: {}", esc_pct(tail)) // p10k:4415
    };
    // p10k:4413/4415 — `"$0" "yellow" "$_p9k_color1" '' 0 '' …`
    vec![make_segment(
        "openfoam",
        None,
        "yellow",
        color1(),
        "",
        content,
    )]
}

// ---------------------------------------------------------------------
// chruby (p10k:3121-3130)
// ---------------------------------------------------------------------

fn chruby_segments() -> Vec<Segment> {
    // p10k:3129 — cond '$RUBY_ENGINE'.
    let engine = env_or_param("RUBY_ENGINE");
    if engine.is_empty() {
        return vec![];
    }
    // p10k:3123 + 7464 — SHOW_ENGINE default 1.
    let mut v = if global_bool("CHRUBY_SHOW_ENGINE", true) {
        engine
    } else {
        String::new()
    };
    // p10k:3124 + 7463 — SHOW_VERSION default 1; `v+=${v:+ }$RUBY_VERSION`.
    let ruby_version = env_or_param("RUBY_VERSION");
    if global_bool("CHRUBY_SHOW_VERSION", true) && !ruby_version.is_empty() {
        if !v.is_empty() {
            v.push(' ');
        }
        v.push_str(&ruby_version);
    }
    // p10k:3125 — `"$0" "red" "$_p9k_color1" 'RUBY_ICON' 0 '' "${v//\%/%%}"`
    vec![make_segment(
        "chruby",
        None,
        "red",
        color1(),
        "RUBY_ICON",
        esc_pct(&v),
    )]
}

// ---------------------------------------------------------------------
// dotnet_version (p10k:2649-2659)
// ---------------------------------------------------------------------

fn dotnet_version_segments() -> Vec<Segment> {
    // p10k:2650 + 7415 — PROJECT_ONLY default 1.
    if global_bool("DOTNET_VERSION_PROJECT_ONLY", true) {
        // p10k:2651 — upglob 'project.json|global.json|packet.dependencies|
        // *.csproj|*.fsproj|*.xproj|*.sln'.
        let found = upfind_pred(&|dir| {
            ["project.json", "global.json", "packet.dependencies"]
                .iter()
                .any(|f| dir.join(f).exists())
                || dir_has_ext(dir, &["csproj", "fsproj", "xproj", "sln"])
        });
        if found.is_none() {
            return vec![];
        }
    }
    // p10k:2653 — `_p9k_cached_cmd 0 '' dotnet --version || return`.
    let Some(v) = cached_cmd("dotnet", &["--version"]) else {
        return vec![];
    };
    // p10k:2654 — `"$0" "magenta" "white" 'DOTNET_ICON' 0 '' "$_p9k__ret"`
    vec![make_segment(
        "dotnet_version",
        None,
        "magenta",
        "white",
        "DOTNET_ICON",
        v,
    )]
}

// ---------------------------------------------------------------------
// php_version (p10k:2673-2685)
// ---------------------------------------------------------------------

fn php_version_segments() -> Vec<Segment> {
    // p10k:2674 + 7414 — PROJECT_ONLY default 0.
    if global_bool("PHP_VERSION_PROJECT_ONLY", false) {
        // p10k:2675 — upglob 'composer.json|*.php'.
        let found =
            upfind_pred(&|dir| dir.join("composer.json").exists() || dir_has_ext(dir, &["php"]));
        if found.is_none() {
            return vec![];
        }
    }
    // p10k:2677 — `_p9k_cached_cmd 0 '' php --version || return`.
    let Some(out) = cached_cmd("php", &["--version"]) else {
        return vec![];
    };
    // p10k:2678-2679 — 'PHP <digits.>' line.
    let Some(v) = php_version_extract(&out) else {
        return vec![];
    };
    // p10k:2680 — `"$0" "fuchsia" "grey93" 'PHP_ICON' 0 '' "${v//\%/%%}"`
    vec![make_segment(
        "php_version",
        None,
        "fuchsia",
        "grey93",
        "PHP_ICON",
        esc_pct(&v),
    )]
}

// ---------------------------------------------------------------------
// swift_version (p10k:4425-4433)
// ---------------------------------------------------------------------

fn swift_version_segments() -> Vec<Segment> {
    // p10k:4426 — `_p9k_cached_cmd 0 '' swift --version || return`.
    let Some(out) = cached_cmd("swift", &["--version"]) else {
        return vec![];
    };
    // p10k:4427 — first digit/dot run.
    let Some(v) = swift_version_extract(&out) else {
        return vec![];
    };
    // p10k:4428 — `"$0" "magenta" "white" 'SWIFT_ICON' 0 '' "${match[1]//\%/%%}"`
    vec![make_segment(
        "swift_version",
        None,
        "magenta",
        "white",
        "SWIFT_ICON",
        esc_pct(&v),
    )]
}

// ---------------------------------------------------------------------
// laravel_version (p10k:2323-2338)
// ---------------------------------------------------------------------

fn laravel_version_segments() -> Vec<Segment> {
    // p10k:2337 — init cond '$commands[php]'.
    let Some(php) = cmd_on_path("php") else {
        return vec![];
    };
    // p10k:2324-2325 — upglob artisan.
    let Some(dir) = upfind("artisan") else {
        return vec![];
    };
    let artisan = dir.join("artisan");
    let app = dir.join("vendor/laravel/framework/src/Illuminate/Foundation/Application.php");
    // p10k:2327 — [[ -r $app ]] || return.
    if !path_readable(&app) {
        return vec![];
    }
    // p10k:2328-2331 — `php $dir/artisan --version` cached on the
    // mtimes of artisan + Application.php (_p9k_cache_stat_get).
    let key = format!("laravel_version {}", dir.display());
    let artisan_arg = artisan.display().to_string();
    let ver = stat_cached(&key, &[artisan.clone(), app.clone()], || {
        let out = run_tool_budget(
            &php,
            &[&artisan_arg, "--version"],
            None,
            Duration::from_secs(5),
            false, // p10k:2329 — `$( … 2>/dev/null)` ignores exit status
        )?;
        // p10k:2330 — `${${(M)v:#Laravel Framework *}#Laravel Framework }`.
        laravel_version_extract(out.trim_end_matches('\n'))
    });
    // p10k:2332 — [[ -n $_p9k__cache_val[1] ]] || return.
    let Some(v) = ver.filter(|v| !v.is_empty()) else {
        return vec![];
    };
    // p10k:2333 — `"$0" "maroon" "white" 'LARAVEL_ICON' 0 '' "${…//\%/%%}"`
    vec![make_segment(
        "laravel_version",
        None,
        "maroon",
        "white",
        "LARAVEL_ICON",
        esc_pct(&v),
    )]
}

// ---------------------------------------------------------------------
// terraform_version (p10k:4957-4968)
// ---------------------------------------------------------------------

fn terraform_version_segments() -> Vec<Segment> {
    // p10k:4958 — `_p9k_cached_cmd 0 '' terraform --version || return`.
    let Some(out) = cached_cmd("terraform", &["--version"]) else {
        return vec![];
    };
    // p10k:4959-4962 — "Terraform v" prefix must strip; first line.
    let Some(v) = terraform_version_extract(&out) else {
        return vec![];
    };
    // p10k:4963 — `$0 $_p9k_color1 blue TERRAFORM_ICON 0 '' $v`
    vec![make_segment(
        "terraform_version",
        None,
        color1(),
        "blue",
        "TERRAFORM_ICON",
        v,
    )]
}

// ---------------------------------------------------------------------
// symfony2_version (p10k:3427-3432)
// ---------------------------------------------------------------------

fn symfony2_version_segments() -> Vec<Segment> {
    // p10k:3428 — [[ -r app/bootstrap.php.cache ]] (cwd-relative).
    let file = cwd().join("app/bootstrap.php.cache");
    if !path_readable(&file) {
        return vec![];
    }
    // p10k:3429 — grep -F " VERSION " + strip non-[0-9.]; the zsh
    // greps per prompt, here the read is stat-cached on the file's
    // mtime (bootstrap.php.cache can be large).
    let key = format!("symfony2_version {}", file.display());
    let v = stat_cached(&key, &[file.clone()], || {
        std::fs::read_to_string(&file)
            .ok()
            .map(|c| symfony2_version_extract(&c))
    })
    .unwrap_or_default();
    // p10k:3430 — segment shown (possibly empty content) once the
    // cache file is readable:
    // `"$0" "grey35" "$_p9k_color1" 'SYMFONY_ICON' 0 '' "${v//\%/%%}"`
    vec![make_segment(
        "symfony2_version",
        None,
        "grey35",
        color1(),
        "SYMFONY_ICON",
        esc_pct(&v),
    )]
}

// ---------------------------------------------------------------------
// rspec_stats (p10k:3210-3217) + symfony2_tests (p10k:3416-3423)
// via _p9k_build_test_stats (p10k:3436-3447)
// ---------------------------------------------------------------------

fn build_test_stats_segments(
    name: &str,
    code: usize,
    tests: usize,
    headline: &str,
) -> Vec<Segment> {
    let Some((state, bg, content)) = test_stats(code, tests, headline) else {
        return vec![];
    };
    // p10k:3444-3446 — `${1}_<STATE> <bg> "$_p9k_color1" TEST_ICON 0 '' …`
    vec![make_segment(
        name,
        Some(state),
        bg,
        color1(),
        "TEST_ICON",
        content,
    )]
}

fn rspec_stats_segments() -> Vec<Segment> {
    let base = cwd();
    // p10k:3211 — [[ -d app && -d spec ]].
    if !base.join("app").is_dir() || !base.join("spec").is_dir() {
        return vec![];
    }
    // p10k:3212 — code=(app/**/*.rb(N)).
    let mut code: Vec<PathBuf> = Vec::new();
    glob_recursive(&base, &base.join("app"), ".rb", &mut code);
    if code.is_empty() {
        return vec![]; // p10k:3213 — (( $#code )) || return
    }
    // p10k:3214 — tests=(spec/**/*.rb(N)).
    let mut tests: Vec<PathBuf> = Vec::new();
    glob_recursive(&base, &base.join("spec"), ".rb", &mut tests);
    // p10k:3215 — _p9k_build_test_stats "$0" $#code $#tests "RSpec" TEST_ICON.
    build_test_stats_segments("rspec_stats", code.len(), tests.len(), "RSpec")
}

fn symfony2_tests_segments() -> Vec<Segment> {
    let base = cwd();
    // p10k:3417 — [[ -d src && -d app && -f app/AppKernel.php ]].
    if !base.join("src").is_dir()
        || !base.join("app").is_dir()
        || !base.join("app/AppKernel.php").is_file()
    {
        return vec![];
    }
    // p10k:3418 — all=(src/**/*.php(N)).
    let mut all: Vec<PathBuf> = Vec::new();
    glob_recursive(&base, &base.join("src"), ".php", &mut all);
    // p10k:3419 — code=(${(@)all##*Tests*}): unquoted expansion drops
    // the elements that matched (paths containing "Tests"); matched on
    // the cwd-relative glob string exactly like zsh.
    let code = all
        .iter()
        .filter(|p| !p.to_string_lossy().contains("Tests"))
        .count();
    if code == 0 {
        return vec![]; // p10k:3420 — (( $#code )) || return
    }
    // p10k:3421 — tests = $#all - $#code.
    build_test_stats_segments("symfony2_tests", code, all.len() - code, "SF2")
}

// ---------------------------------------------------------------------
// Tests — pure extractors only (literal inputs)
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// p10k:2678 — 'PHP <digits.>' line anywhere; digit-less "PHP "
    /// lines are backtracked past.
    #[test]
    fn php_version_extract_forms() {
        assert_eq!(
            php_version_extract("PHP 8.3.7 (cli) (built: May  8 2024)").as_deref(),
            Some("8.3.7")
        );
        assert_eq!(
            php_version_extract("Deprecation warning\nPHP 7.4.33 (cli)").as_deref(),
            Some("7.4.33")
        );
        // A "PHP " line without digits does not stop the scan.
        assert_eq!(
            php_version_extract("PHP warning: x\nPHP 8.0.1").as_deref(),
            Some("8.0.1")
        );
        assert_eq!(php_version_extract("no version here"), None);
    }

    /// p10k:4427 — capture starts at the FIRST digit; dots in the
    /// leading skip-run stay skipped.
    #[test]
    fn swift_version_extract_forms() {
        assert_eq!(
            swift_version_extract("swift-driver version: 1.90.11.1 Apple Swift version 5.10")
                .as_deref(),
            Some("1.90.11.1")
        );
        assert_eq!(
            swift_version_extract("Apple Swift version 5.9.2 (swiftlang…)").as_deref(),
            Some("5.9.2")
        );
        assert_eq!(swift_version_extract("no digits"), None);
    }

    /// p10k:4959-4962 — prefix must strip; first line only.
    #[test]
    fn terraform_version_extract_forms() {
        assert_eq!(
            terraform_version_extract("Terraform v1.7.5\non darwin_arm64").as_deref(),
            Some("1.7.5")
        );
        assert_eq!(terraform_version_extract("OpenTofu v1.6.0"), None);
        assert_eq!(terraform_version_extract("Terraform v"), None);
    }

    /// p10k:2330 — whole output must start with the exact headline.
    #[test]
    fn laravel_version_extract_forms() {
        assert_eq!(
            laravel_version_extract("Laravel Framework 11.9.2").as_deref(),
            Some("11.9.2")
        );
        assert_eq!(laravel_version_extract("Lumen (11.0.0)"), None);
    }

    /// p10k:3429 — ` VERSION ` lines, everything but [0-9.] removed.
    #[test]
    fn symfony2_version_extract_forms() {
        assert_eq!(
            symfony2_version_extract("    const VERSION = '2.8.52';\nother line"),
            "2.8.52"
        );
        assert_eq!(symfony2_version_extract("no version marker"), "");
    }

    /// p10k:3441-3446 — ratio thresholds, two fixed decimals, `%%`.
    #[test]
    fn test_stats_states_and_format() {
        assert_eq!(test_stats(0, 10, "RSpec"), None); // p10k:3441
        assert_eq!(
            test_stats(4, 3, "RSpec"),
            Some(("GOOD", "cyan", "RSpec: 75.00%%".to_string()))
        );
        assert_eq!(
            test_stats(2, 1, "SF2"),
            Some(("AVG", "yellow", "SF2: 50.00%%".to_string()))
        );
        assert_eq!(
            test_stats(3, 1, "RSpec"),
            Some(("BAD", "red", "RSpec: 33.33%%".to_string()))
        );
    }

    /// p10k:1550 — `^[0-9a-f.:]+$` over v4, v6, and junk.
    #[test]
    fn valid_public_ip_forms() {
        assert!(valid_public_ip("93.184.216.34"));
        assert!(valid_public_ip("2606:2800:220:1:248:1893:25c8:1946"));
        assert!(!valid_public_ip(""));
        assert!(!valid_public_ip(";; connection timed out"));
        assert!(!valid_public_ip("93.184.216.34\nextra"));
        assert!(!valid_public_ip("host.example.com")); // 'h','x','m' invalid
    }

    /// p10k:4542 — cut -d' ' -f2- on the first line; no-delimiter
    /// lines pass through whole.
    #[test]
    fn dropbox_cut_status_forms() {
        assert_eq!(
            dropbox_cut_status("/home/u/Dropbox: up to date"),
            "up to date"
        );
        assert_eq!(
            dropbox_cut_status("Dropbox isn't running!"),
            "isn't running!"
        );
        assert_eq!(dropbox_cut_status("unwatched"), "unwatched");
        assert_eq!(dropbox_cut_status(""), "");
    }

    /// p10k:8236-8238 — exactly one `name=` line, unquoted, non-empty.
    #[test]
    fn containerenv_toolbox_name_forms() {
        assert_eq!(
            containerenv_toolbox_name("engine=\"podman\"\nname=\"fedora-toolbox-39\"\nimage=\"x\"")
                .as_deref(),
            Some("fedora-toolbox-39")
        );
        assert_eq!(
            containerenv_toolbox_name("name=plain").as_deref(),
            Some("plain")
        );
        // Two name= lines → rejected (p10k:8237).
        assert_eq!(containerenv_toolbox_name("name=\"a\"\nname=\"b\""), None);
        assert_eq!(containerenv_toolbox_name("name=\"\""), None);
        assert_eq!(containerenv_toolbox_name("engine=\"podman\""), None);
    }
}
