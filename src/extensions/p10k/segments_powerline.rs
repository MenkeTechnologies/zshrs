//! p10k POWERLINE-catalog segments — weather / uptime / now_playing /
//! network_load / hg / svn / bzr / fossil.
//!
//! These are segments the powerline (python) status line has that
//! powerlevel10k does NOT. The installed `powerline-status 2.7`
//! package (`pip3 show powerline-status`; module tree under
//! `~/Library/Python/3.13/lib/python/site-packages/powerline/`) is THE
//! SPEC; ported lines are cited as `// powerline:<relpath>:NNN`
//! relative to that package root. Segment SHAPE (content layout, state
//! split, hide rules) mirrors the python segments; powerline's
//! threaded-segment refresh loop (`lib/threaded.py` KwThreadedSegment)
//! collapses into synchronous reads behind TTL caches — no external
//! tool ever runs uncached per prompt, and every subprocess is
//! budget-killed (the segments_extra run_tool_budget pattern).
//!
//! Scope notes (each also traced at the call site):
//! - weather: powerline fetches Yahoo! Weather YQL JSON in a thread
//!   (segments/common/wthr.py:104-201) — that API is dead; the SHAPE
//!   (condition icon + temperature, wthr.py:189-201) is kept but the
//!   data source is wttr.in `?format=%c%t` via curl, 15-min success
//!   TTL / 60s failure retry, 6s budget kill. Hidden on any failure.
//! - now_playing: powerline's darwin players drive osascript
//!   (segments/common/players.py:503-531 ITunesPlayerSegment);
//!   iTunes is renamed Music.app on modern macOS, so the same
//!   AppleScript targets "Music", gated on a cached `pgrep -x Music`
//!   so osascript can never launch the app. Linux uses playerctl
//!   (powerline's dbus players need the python dbus module — CLI
//!   equivalent per task spec).
//! - svn / fossil: NOT in powerline's VCS catalog (lib/vcs/
//!   __init__.py:216-220 lists git/mercurial/bzr only); implemented in
//!   the same branch+dirty shape per task spec. hg/bzr mirror
//!   powerline's branch + tree_status split (segments/common/
//!   vcs.py:18-40) via the tool CLI instead of python lib bindings.
//! - git is deliberately NOT handled here — git.rs owns it.
//!
//! Shared-helper duplication: color1/env_or_param/esc_pct/
//! decode_g/seg_icon/make_segment/cmd_on_path/cwd/upfind/cached_ttl/
//! run_tool_budget mirror segments_sys.rs / segments_extra.rs (private
//! there; this module may not edit other files). Hoisting them into a
//! shared submodule is a follow-up for the orchestrator.

use crate::extensions::p10k::config::{p9k_global, p9k_param};
use crate::extensions::p10k::icons;
use crate::extensions::p10k::render::Segment;
use crate::ported::params::getsparam;
use crate::ported::utils::getkeystring;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

/// Segment dispatch per the p10k module contract. `None` = not handled
/// by this module; `Some(vec![])` = handled but hidden this prompt.
pub fn build_segment(name: &str) -> Option<Vec<Segment>> {
    match name {
        "weather" => Some(weather_segments()), // powerline:segments/common/wthr.py:104
        "uptime" => Some(uptime_segments()),   // powerline:segments/common/sys.py:153
        "now_playing" => Some(now_playing_segments()), // powerline:segments/common/players.py:36
        "network_load" => Some(network_load_segments()), // powerline:segments/common/net.py:194
        "hg" => Some(hg_segments()),           // powerline:lib/vcs/__init__.py:218
        "svn" => Some(svn_segments()),         // task spec (not in powerline catalog)
        "bzr" => Some(bzr_segments()),         // powerline:lib/vcs/__init__.py:219
        "fossil" => Some(fossil_segments()),   // task spec (not in powerline catalog)
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Shared helpers (mirror segments_sys.rs / segments_extra.rs — module doc)
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

/// Read a parameter, falling back to the process environment (covers
/// early-startup renders before exports land in the paramtab).
fn env_or_param(name: &str) -> String {
    if let Some(v) = getsparam(name) {
        return v;
    }
    std::env::var(name).unwrap_or_default()
}

/// `_p9k_declare -i` read: unset/empty/unparseable → default.
fn global_int(name: &str, default: i64) -> i64 {
    getsparam(&format!("POWERLEVEL9K_{name}"))
        .and_then(|v| v.trim().parse::<i64>().ok())
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
/// caller sits behind a TTL cache or a cheap short-circuit). Mirrors
/// segments_env::have_cmd's scan.
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

/// Port of `_p9k_upglob` (p10k:265-292) for a literal name — mirrors
/// segments_extra::upfind: walk from $PWD to `/`, first ancestor
/// containing `name` (dir markers like `.hg` and file markers like
/// `.fslckout` both satisfy `exists()`). `None` == "not found → hidden".
/// Same walk powerline's `guess` does over vcs_props
/// (powerline:lib/vcs/__init__.py:229-242).
fn upfind(name: &str) -> Option<PathBuf> {
    let start = cwd();
    let mut dir: &Path = &start;
    loop {
        if dir.join(name).exists() {
            return Some(dir.to_path_buf());
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => return None,
        }
    }
}

// ---------------------------------------------------------------------
// Subprocess: TTL cache + budgeted runner
// ---------------------------------------------------------------------

/// segments_extra::cached_ttl mirror — String keys, producer-chosen TTL
/// (weather caches successes 15 min but failures 60s; vcs keys on the
/// repo root). Stored Instant is the EXPIRY. `None` results
/// (spawn/parse failure) are cached too, so a broken tool can't fork
/// on every prompt. This replaces powerline's background refresh
/// thread (powerline:lib/threaded.py:35 — interval-driven update loop).
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
/// pattern (mirrors segments_extra::run_tool_budget): stdout drained on
/// a helper thread, child killed and reaped on overrun so a hung
/// network tool can't wedge the prompt.
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

// ---------------------------------------------------------------------
// Pure formatters (ported; unit-tested below)
// ---------------------------------------------------------------------

/// Port of `humanize_bytes` (powerline:lib/humanize_bytes.py:10-25).
/// powerline:lib/humanize_bytes.py:7 —
/// `unit_list = tuple(zip(['', 'k', 'M', 'G', 'T', 'P'], [0, 0, 1, 2, 2, 2]))`.
fn humanize_bytes(num: f64, suffix: &str, si_prefix: bool) -> String {
    const UNIT_LIST: [(&str, usize); 6] = [("", 0), ("k", 0), ("M", 1), ("G", 2), ("T", 2), ("P", 2)];
    // humanize_bytes.py:15-16 — if num == 0: return '0 ' + suffix
    if num == 0.0 {
        return format!("0 {suffix}");
    }
    // humanize_bytes.py:17 — div = 1000 if si_prefix else 1024
    let div: f64 = if si_prefix { 1000.0 } else { 1024.0 };
    // humanize_bytes.py:18 — exponent = min(int(log(num, div)), len-1).
    // python int() truncates toward zero; sub-1 B/s rates give a small
    // negative log — clamp to 0 (python's int() already lands there for
    // any realistic rate).
    let exponent = ((num.ln() / div.ln()).trunc() as i64).clamp(0, 5) as usize;
    // humanize_bytes.py:19 — quotient = float(num) / div ** exponent
    let quotient = num / div.powi(exponent as i32);
    let (unit, decimals) = UNIT_LIST[exponent];
    // humanize_bytes.py:21-22 — if unit and not si_prefix: unit.upper() + 'i'
    let unit = if !unit.is_empty() && !si_prefix {
        format!("{}i", unit.to_uppercase())
    } else {
        unit.to_string()
    };
    // humanize_bytes.py:23-25 — '{quotient:.{decimals}f} {unit}{suffix}'
    format!("{quotient:.decimals$} {unit}{suffix}")
}

/// Port of the `uptime` body (powerline:segments/common/sys.py:174-183):
/// divmod cascade, zero components dropped entirely, first
/// `shorten_len` of the REMAINING components kept, joined, stripped.
fn format_uptime(total_seconds: i64, shorten_len: usize) -> String {
    // sys.py:174-176 — divmod cascade.
    let (minutes, seconds) = (total_seconds / 60, total_seconds % 60);
    let (hours, minutes) = (minutes / 60, minutes % 60);
    let (days, hours) = (hours / 24, hours % 24);
    // sys.py:177-182 — formats applied only when the component is
    // non-zero (`days_format.format(days=days) if days ... else None`,
    // then `filter(None, …)`), sliced `[0:shorten_len]`.
    let mut parts: Vec<String> = Vec::new();
    if days != 0 {
        parts.push(format!("{days}d")); // sys.py:153 days_format='{days:d}d'
    }
    if hours != 0 {
        parts.push(format!(" {hours}h")); // sys.py:153 hours_format=' {hours:d}h'
    }
    if minutes != 0 {
        parts.push(format!(" {minutes}m")); // sys.py:153 minutes_format
    }
    if seconds != 0 {
        parts.push(format!(" {seconds}s")); // sys.py:153 seconds_format
    }
    // sys.py:183 — ''.join(time_formatted).strip()
    parts
        .into_iter()
        .take(shorten_len)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Port of `_convert_seconds` (powerline:segments/common/players.py:31-33):
/// `'{0:.0f}:{1:02.0f}'.format(*divmod(float(seconds), 60))`. The
/// python quirk where 59.6s formats as "0:60" is kept.
fn convert_seconds(seconds: f64) -> String {
    let minutes = (seconds / 60.0).floor();
    let rem = seconds - minutes * 60.0;
    format!("{minutes:.0}:{rem:02.0}")
}

/// Port of `_convert_state` (powerline:segments/common/players.py:19-28).
fn convert_state(state: &str) -> &'static str {
    let state = state.to_lowercase();
    if state.contains("play") {
        "play"
    } else if state.contains("pause") {
        "pause"
    } else if state.contains("stop") {
        "stop"
    } else {
        "fallback"
    }
}

/// STATE_SYMBOLS (powerline:segments/common/players.py:11-16).
fn state_symbol(state: &str) -> &'static str {
    match state {
        "play" => ">",
        "pause" => "~",
        "stop" => "X",
        _ => "", // fallback
    }
}

// ---------------------------------------------------------------------
// weather (shape: powerline:segments/common/wthr.py:104-201)
// ---------------------------------------------------------------------

/// wttr.in `%c%t` sanity check: powerline validates the API response
/// before rendering (wthr.py:145-152 — malformed response → None →
/// hidden); the wttr equivalent rejects HTML error pages, rate-limit
/// prose, and anything that isn't a short "icon + signed temp" line.
fn valid_weather(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && !s.contains('<')
        && !s.contains('\n')
        && s.contains('°')
        && s.bytes().any(|b| b.is_ascii_digit())
}

/// Percent-encode a user-supplied location for the wttr.in URL path.
fn encode_location(loc: &str) -> String {
    let mut out = String::new();
    for b in loc.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' | b'+' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn weather_segments() -> Vec<Segment> {
    // wthr.py:104-136 — powerline resolves location via geoip when
    // unset; wttr.in does the same server-side, so an empty
    // POWERLEVEL9K_WEATHER_LOCATION just hits the bare endpoint.
    let location = encode_location(&p9k_global("WEATHER_LOCATION", ""));
    // wthr.py:166/97-101 — unit='C'|'F'|'K'; wttr.in only offers
    // metric (?m) and USCS (?u), so K falls back to the server default.
    let units = p9k_global("WEATHER_UNITS", "");
    let unit_q = match units.as_str() {
        "F" | "f" | "u" | "imperial" => "&u",
        "C" | "c" | "m" | "metric" => "&m",
        _ => "",
    };
    let Some(curl) = cmd_on_path("curl") else {
        return vec![]; // no fetch tool — hidden, never faked
    };
    let key = format!("weather {location} {unit_q}");
    let text = cached_ttl(&key, || {
        // wthr.py:105 — interval = 600; task spec raises the success
        // TTL to 15 min. Failures retry after 60s (the public_ip
        // fail-fast pattern, segments_extra module doc).
        let url = format!("https://wttr.in/{location}?format=%c%t{unit_q}");
        // -f: HTTP errors (404 unknown location, 503 rate limit) fail
        // the exit status; --max-time 5 inside a 6s kill budget, the
        // public_ip pattern (segments_extra::fetch_public_ip).
        let fetched = run_tool_budget(
            &curl,
            &["-fsS", "--max-time", "5", &url],
            None,
            Duration::from_secs(6),
            true,
        )
        .map(|s| s.trim().to_string())
        .filter(|s| valid_weather(s));
        match fetched {
            Some(v) => (Duration::from_secs(900), Some(v)),
            None => (Duration::from_secs(60), None),
        }
    });
    let Some(text) = text else {
        return vec![]; // fetch/validate failure — hidden
    };
    // wthr.py:189-201 — powerline emits icon + temp as two groups;
    // wttr's %c%t is already that shape in one string.
    vec![make_segment(
        "weather",
        None,
        "blue",
        "white",
        "WEATHER_ICON",
        esc_pct(&text),
    )]
}

// ---------------------------------------------------------------------
// uptime (powerline:segments/common/sys.py:134-183)
// ---------------------------------------------------------------------

/// darwin uptime source: sysctl kern.boottime → struct timeval, uptime
/// = now − boottime (powerline reads /proc/uptime or psutil,
/// sys.py:134-146; neither exists on macOS — kern.boottime is the
/// native equivalent).
#[cfg(target_os = "macos")]
fn uptime_seconds() -> Option<i64> {
    let name = std::ffi::CString::new("kern.boottime").ok()?;
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::timeval>();
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut tv as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || tv.tv_sec <= 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let up = now - tv.tv_sec;
    (up > 0).then_some(up)
}

/// Linux uptime source (powerline:segments/common/sys.py:134-137):
/// `int(float(f.readline().split()[0]))` over /proc/uptime.
#[cfg(not(target_os = "macos"))]
fn uptime_seconds() -> Option<i64> {
    let content = std::fs::read_to_string("/proc/uptime").ok()?;
    let first = content.split_whitespace().next()?;
    Some(first.parse::<f64>().ok()? as i64)
}

fn uptime_segments() -> Vec<Segment> {
    let Some(secs) = uptime_seconds() else {
        return vec![]; // sys.py:169-173 — NotImplementedError → None → hidden
    };
    // sys.py:153 — shorten_len=3.
    let shorten = global_int("UPTIME_SHORTEN_LEN", 3).clamp(1, 4) as usize;
    let text = format_uptime(secs, shorten);
    if text.is_empty() {
        return vec![]; // sub-second uptime — nothing to show
    }
    vec![make_segment("uptime", None, color1(), "cyan", "UPTIME_ICON", text)]
}

// ---------------------------------------------------------------------
// now_playing (powerline:segments/common/players.py)
// ---------------------------------------------------------------------

/// darwin: Music.app via osascript, the ITunesPlayerSegment script
/// (players.py:503-524) retargeted at "Music" (iTunes' successor).
/// Gated on a cached `pgrep -x Music` so the `tell application` can
/// never LAUNCH Music when it isn't running (the in-script System
/// Events process check, players.py:507-511, is kept as the second
/// gate). Returns (state, artist, title, total_seconds).
#[cfg(target_os = "macos")]
fn player_status() -> Option<(String, String, String, Option<f64>)> {
    // Process gate: 10s TTL, budget-killed. pgrep exits 1 when Music
    // is not running → None → hidden.
    cached_ttl("now_playing.pgrep", || {
        let out = cmd_on_path("pgrep").and_then(|bin| {
            run_tool_budget(&bin, &["-x", "Music"], None, Duration::from_secs(2), true)
        });
        (Duration::from_secs(10), out)
    })?;
    // players.py:505 — status_delimiter = '-~`/='.
    const DELIM: &str = "-~`/=";
    // players.py:506-524 — field order kept: title, artist, album,
    // elapsed, duration, state ("iTunes" → "Music"; `player state is
    // playing` only, exactly like the python segment).
    let script = r#"
        tell application "System Events"
            set process_list to (name of every process)
        end tell
        if process_list contains "Music" then
            tell application "Music"
                if player state is playing then
                    set t_title to name of current track
                    set t_artist to artist of current track
                    set t_album to album of current track
                    set t_duration to duration of current track
                    set t_elapsed to player position
                    set t_state to player state
                    return t_title & "-~`/=" & t_artist & "-~`/=" & t_album & "-~`/=" & t_elapsed & "-~`/=" & t_duration & "-~`/=" & t_state
                end if
            end tell
        end if
    "#;
    let out = cached_ttl("now_playing.osascript", || {
        let out = cmd_on_path("osascript").and_then(|bin| {
            run_tool_budget(&bin, &["-e", script], None, Duration::from_secs(5), true)
        });
        (Duration::from_secs(10), out)
    })?;
    let line = out.trim_end_matches('\n');
    if line.is_empty() {
        return None; // not playing — script returned nothing (players.py:526)
    }
    let fields: Vec<&str> = line.split(DELIM).collect();
    if fields.len() < 6 {
        tracing::debug!(target: "p10k", "now_playing: unexpected osascript field count");
        return None;
    }
    // players.py:527-537 — title/artist/total/state out of the split.
    let total = fields[4].trim().parse::<f64>().ok();
    Some((
        convert_state(fields[5]).to_string(),
        fields[1].to_string(),
        fields[0].to_string(),
        total,
    ))
}

/// Linux: playerctl when on PATH (task spec — powerline's MPRIS
/// players need the python dbus module, players.py:406-424; playerctl
/// is the CLI equivalent). `playerctl metadata --format` exits
/// non-zero when no player is available → hidden.
#[cfg(not(target_os = "macos"))]
fn player_status() -> Option<(String, String, String, Option<f64>)> {
    let out = cached_ttl("now_playing.playerctl", || {
        let out = cmd_on_path("playerctl").and_then(|bin| {
            run_tool_budget(
                &bin,
                &[
                    "metadata",
                    "--format",
                    "{{status}}\t{{artist}}\t{{title}}\t{{mpris:length}}",
                ],
                None,
                Duration::from_secs(2),
                true,
            )
        });
        (Duration::from_secs(10), out)
    })?;
    let line = out.lines().next()?;
    let mut f = line.split('\t');
    let state = convert_state(f.next()?);
    let artist = f.next().unwrap_or("").to_string();
    let title = f.next().unwrap_or("").to_string();
    // mpris:length is microseconds.
    let total = f
        .next()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|us| us / 1_000_000.0);
    if state == "stop" {
        return None; // players.py:374-375 — stop → None → hidden
    }
    Some((state.to_string(), artist, title, total))
}

fn now_playing_segments() -> Vec<Segment> {
    let Some((state, artist, title, total)) = player_status() else {
        return vec![]; // players.py:47-48 — no stats → None → hidden
    };
    // players.py:37 — default format
    // '{state_symbol} {artist} - {title} ({total})'.
    let sym = state_symbol(&state);
    let total_s = total.map(|t| convert_seconds(t)).unwrap_or_default();
    let mut content = format!("{sym} {artist} - {title}");
    if !total_s.is_empty() {
        content.push_str(&format!(" ({total_s})"));
    }
    vec![make_segment(
        "now_playing",
        None,
        color1(),
        "white",
        "NOW_PLAYING_ICON",
        esc_pct(content.trim()),
    )]
}

// ---------------------------------------------------------------------
// network_load (powerline:segments/common/net.py:194-282)
// ---------------------------------------------------------------------

/// darwin per-interface RX/TX byte counters: getifaddrs AF_LINK
/// entries carry `struct if_data` in ifa_data (man getifaddrs;
/// libc-0.2.186 unix/bsd/apple/b64/mod.rs:29-30 — ifi_ibytes /
/// ifi_obytes, u32). Fork-free — in-tree segments_core.rs:630 only
/// scans AF_INET addresses, so the counter walk is new here; the
/// netstat-subprocess fallback from the task spec is unnecessary.
/// Powerline gets the same numbers from psutil (net.py:161-169).
#[cfg(target_os = "macos")]
fn iface_counters() -> Vec<(String, u64, u64)> {
    let mut out: Vec<(String, u64, u64)> = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        tracing::warn!(target: "p10k", "getifaddrs failed for network_load scan");
        return out;
    }
    let mut cur = ifap;
    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_data.is_null() {
            continue;
        }
        let sa = unsafe { &*ifa.ifa_addr };
        if i32::from(sa.sa_family) != libc::AF_LINK {
            continue;
        }
        let data = unsafe { &*(ifa.ifa_data as *const libc::if_data) };
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        out.push((name, u64::from(data.ifi_ibytes), u64::from(data.ifi_obytes)));
    }
    unsafe { libc::freeifaddrs(ifap) };
    out
}

/// Linux per-interface counters (powerline:segments/common/
/// net.py:180-191): /sys/class/net/<iface>/statistics/{rx,tx}_bytes.
#[cfg(not(target_os = "macos"))]
fn iface_counters() -> Vec<(String, u64, u64)> {
    let mut out: Vec<(String, u64, u64)> = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/class/net") else {
        return out;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stat = |leaf: &str| -> Option<u64> {
            std::fs::read_to_string(entry.path().join("statistics").join(leaf))
                .ok()?
                .trim()
                .parse()
                .ok()
        };
        if let (Some(rx), Some(tx)) = (stat("rx_bytes"), stat("tx_bytes")) {
            out.push((name, rx, tx));
        }
    }
    out
}

/// Port of the `interface == 'auto'` resolution (net.py:202-228): the
/// /proc/net/route default-route interface when readable (linux), else
/// the interface with the most total activity excluding well-known
/// virtual names by alphabetic prefix (net.py:222-223 — lo/vmnet/sit).
fn auto_interface(counters: &[(String, u64, u64)]) -> String {
    // net.py:207-216 — default route: destination field all zeros.
    if let Ok(routes) = std::fs::read_to_string("/proc/net/route") {
        for line in routes.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 1 && !parts[1].is_empty() && parts[1].bytes().all(|b| b == b'0') {
                return parts[0].to_string();
            }
        }
    }
    // net.py:217-228 — most rx+tx, `eth0` fallback.
    let mut best = ("eth0".to_string(), None::<u64>);
    for (name, rx, tx) in counters {
        // net.py:222 — replace_num_pat = [a-zA-Z]+ prefix; no match → skip.
        let base: String = name.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        if base.is_empty() || matches!(base.as_str(), "lo" | "vmnet" | "sit") {
            continue;
        }
        let activity = rx + tx;
        if best.1.is_none_or(|b| activity > b) {
            best = (name.clone(), Some(activity));
        }
    }
    best.0
}

/// Counter delta with darwin's 32-bit if_data wrap handled: ifi_ibytes
/// / ifi_obytes are u32 (libc apple b64/mod.rs:29-30) and wrap at
/// 4 GiB. Linux sysfs counters are u64-monotonic and never take the
/// wrap branch.
fn counter_delta(prev: u64, last: u64) -> u64 {
    if last >= prev {
        last - prev
    } else {
        last + (1u64 << 32) - prev
    }
}

/// (sampled_at, rx_bytes, tx_bytes).
type NetSample = (Instant, u64, u64);

/// Per-interface (last, prev) samples — the `idata['last']` /
/// `idata['prev']` pair of net.py:230-244, stored in a static instead
/// of the segment object. A new sample is taken at most once per
/// second (powerline:lib/threaded.py:35 — ThreadedSegment.interval=1).
static NET_SAMPLES: OnceLock<Mutex<HashMap<String, (NetSample, Option<NetSample>)>>> =
    OnceLock::new();

fn network_load_segments() -> Vec<Segment> {
    // net.py:199 — interface kwarg, default 'auto'.
    let mut iface = p9k_global("NETWORK_LOAD_INTERFACE", "auto");
    let counters = iface_counters();
    if iface == "auto" {
        iface = auto_interface(&counters);
    }
    let Some(&(_, rx, tx)) = counters.iter().find(|(n, _, _)| *n == iface) else {
        return vec![]; // net.py:166-168 — unknown interface → None → hidden
    };
    let now = Instant::now();
    let (last, prev) = {
        let m = NET_SAMPLES.get_or_init(Default::default);
        let Ok(mut guard) = m.lock() else {
            return vec![];
        };
        let entry = guard.entry(iface.clone()).or_insert(((now, rx, tx), None));
        // net.py:232-243 — shift last→prev, store the fresh sample;
        // gated at 1/s so per-keystroke renders don't produce 0-dt pairs.
        if now.duration_since(entry.0.0) >= Duration::from_secs(1) {
            entry.1 = Some(entry.0);
            entry.0 = (now, rx, tx);
        }
        *entry
    };
    // net.py:247-248 — 'prev' not in idata → None (hidden until two
    // samples exist).
    let Some(prev) = prev else {
        return vec![];
    };
    let dt = last.0.duration_since(prev.0).as_secs_f64();
    if dt <= 0.0 {
        return vec![];
    }
    // net.py:258-264 — value = (b2[i] - b1[i]) / measure_interval.
    let rx_rate = counter_delta(prev.1, last.1) as f64 / dt;
    let tx_rate = counter_delta(prev.2, last.2) as f64 / dt;
    // net.py:246 — suffix='B/s', si_prefix=False; DL/UL labels become
    // the ⇣/⇡ glyphs per task spec.
    let content = format!(
        "\u{21e3}{} \u{21e1}{}",
        humanize_bytes(rx_rate, "B/s", false),
        humanize_bytes(tx_rate, "B/s", false)
    );
    vec![make_segment(
        "network_load",
        None,
        "cyan",
        color1(),
        "NETWORK_ICON",
        content,
    )]
}

// ---------------------------------------------------------------------
// VCS: hg / svn / bzr / fossil (shape: powerline:segments/common/
// vcs.py:18-40 — branch content, clean/dirty state split)
// ---------------------------------------------------------------------

/// Repo-root-keyed 5s TTL around a (branch, dirty) probe. The unit
/// separator joins the pair for the string cache; failures cache too.
fn vcs_cached(
    tool: &str,
    root: &Path,
    probe: impl FnOnce() -> Option<(String, bool)>,
) -> Option<(String, bool)> {
    let key = format!("vcs.{tool} {}", root.display());
    let joined = cached_ttl(&key, || {
        (
            Duration::from_secs(5),
            probe().map(|(b, d)| format!("{b}\u{1f}{}", u8::from(d))),
        )
    })?;
    let (b, d) = joined.split_once('\u{1f}')?;
    Some((b.to_string(), d == "1"))
}

/// Shared builder: powerline colors branch content by clean/dirty
/// state (vcs.py:35 — branch_dirty / branch_clean highlight groups);
/// here that is the CLEAN/DIRTY state split with green/yellow default
/// backgrounds, restylable via POWERLEVEL9K_<SEG>_<STATE>_* params.
fn vcs_segment(seg: &str, icon_key: &str, branch: &str, dirty: bool) -> Vec<Segment> {
    let (state, bg) = if dirty {
        ("DIRTY", "yellow")
    } else {
        ("CLEAN", "green")
    };
    vec![make_segment(
        seg,
        Some(state),
        bg,
        color1(),
        icon_key,
        esc_pct(branch),
    )]
}

fn hg_segments() -> Vec<Segment> {
    // Gate: tool on PATH + `.hg` marker upfind (powerline:lib/vcs/
    // __init__.py:218 — ('mercurial', '.hg', os.path.isdir)).
    let Some(bin) = cmd_on_path("hg") else {
        return vec![];
    };
    let Some(root) = upfind(".hg") else {
        return vec![];
    };
    let Some((branch, dirty)) = vcs_cached("hg", &root, || {
        // `hg id -b -i` → "<node>[+] <branch>"; the '+' node suffix
        // means uncommitted changes (hg identify docs). CLI equivalent
        // of powerline reading .hg/branch (lib/vcs/mercurial.py:81-87)
        // + hglib status (lib/vcs/mercurial.py:41-79).
        let out = run_tool_budget(&bin, &["id", "-b", "-i"], Some(&root), Duration::from_secs(5), true)?;
        let line = out.lines().next()?;
        let mut it = line.split_whitespace();
        let node = it.next()?;
        let branch = it.next()?.to_string();
        Some((branch, node.ends_with('+')))
    }) else {
        return vec![];
    };
    vcs_segment("hg", "VCS_HG_ICON", &branch, dirty)
}

fn svn_segments() -> Vec<Segment> {
    // Not in powerline's catalog (lib/vcs/__init__.py:216-220); same
    // shape per task spec. Gate: tool + `.svn` marker upfind.
    let Some(bin) = cmd_on_path("svn") else {
        return vec![];
    };
    let Some(root) = upfind(".svn") else {
        return vec![];
    };
    let Some((rev, dirty)) = vcs_cached("svn", &root, || {
        // `svn info` "Revision: N" line (portable across svn versions,
        // unlike --show-item). Working copies have no branch name —
        // the revision is the content.
        let info = run_tool_budget(&bin, &["info"], Some(&root), Duration::from_secs(5), true)?;
        let rev = info
            .lines()
            .find_map(|l| l.strip_prefix("Revision: "))?
            .trim()
            .to_string();
        if rev.is_empty() {
            return None;
        }
        // `svn status -q` — quiet: tracked modifications only; any
        // output means dirty.
        let status =
            run_tool_budget(&bin, &["status", "-q"], Some(&root), Duration::from_secs(5), true)?;
        Some((format!("r{rev}"), status.lines().any(|l| !l.trim().is_empty())))
    }) else {
        return vec![];
    };
    vcs_segment("svn", "VCS_SVN_ICON", &rev, dirty)
}

fn bzr_segments() -> Vec<Segment> {
    // Gate: tool + `.bzr` marker upfind (powerline:lib/vcs/
    // __init__.py:219 — ('bzr', '.bzr', os.path.isdir)).
    let Some(bin) = cmd_on_path("bzr") else {
        return vec![];
    };
    let Some(root) = upfind(".bzr") else {
        return vec![];
    };
    let Some((nick, dirty)) = vcs_cached("bzr", &root, || {
        // `bzr nick` — the branch nickname, the same value powerline
        // reads out of .bzr/branch/branch.conf (lib/vcs/bzr.py:101-108).
        let out = run_tool_budget(&bin, &["nick"], Some(&root), Duration::from_secs(5), true)?;
        let nick = out.trim().to_string();
        if nick.is_empty() {
            return None;
        }
        // `bzr status -S` — short status; any output means dirty.
        let status =
            run_tool_budget(&bin, &["status", "-S"], Some(&root), Duration::from_secs(5), true)?;
        Some((nick, !status.trim().is_empty()))
    }) else {
        return vec![];
    };
    vcs_segment("bzr", "VCS_BRANCH_ICON", &nick, dirty)
}

fn fossil_segments() -> Vec<Segment> {
    // Not in powerline's catalog; same shape per task spec. Gate: tool
    // + `.fslckout` checkout-marker FILE upfind (fossil's per-checkout
    // database).
    let Some(bin) = cmd_on_path("fossil") else {
        return vec![];
    };
    let Some(root) = upfind(".fslckout") else {
        return vec![];
    };
    let Some((branch, dirty)) = vcs_cached("fossil", &root, || {
        // `fossil branch current` — the checked-out branch name.
        let out = run_tool_budget(
            &bin,
            &["branch", "current"],
            Some(&root),
            Duration::from_secs(5),
            true,
        )?;
        let branch = out.trim().to_string();
        if branch.is_empty() {
            return None;
        }
        // `fossil changes` — modified managed files; any output means dirty.
        let status =
            run_tool_budget(&bin, &["changes"], Some(&root), Duration::from_secs(5), true)?;
        Some((branch, !status.trim().is_empty()))
    }) else {
        return vec![];
    };
    vcs_segment("fossil", "VCS_BRANCH_ICON", &branch, dirty)
}

// ---------------------------------------------------------------------
// Tests — pure formatters only (literal inputs)
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// powerline:lib/humanize_bytes.py:10-25 — zero literal, unit
    /// ladder, `Ki`/`Mi` binary suffixes, per-unit decimals, si_prefix
    /// keeps lowercase k without the 'i'.
    #[test]
    fn humanize_bytes_forms() {
        assert_eq!(humanize_bytes(0.0, "B/s", false), "0 B/s"); // py:15-16
        assert_eq!(humanize_bytes(512.0, "B/s", false), "512 B/s"); // exp 0, dec 0
        assert_eq!(humanize_bytes(2048.0, "B/s", false), "2 KiB/s"); // exp 1, dec 0
        assert_eq!(
            humanize_bytes(3.0 * 1024.0 * 1024.0, "B/s", false),
            "3.0 MiB/s" // exp 2, dec 1
        );
        assert_eq!(
            humanize_bytes(5.0 * 1024.0 * 1024.0 * 1024.0, "B/s", false),
            "5.00 GiB/s" // exp 3, dec 2
        );
        assert_eq!(humanize_bytes(2000.0, "B/s", true), "2 kB/s"); // si: div 1000, no 'i'
        assert_eq!(humanize_bytes(0.5, "B/s", false), "0.5 B/s"); // sub-1 rate clamps to exp 0
    }

    /// powerline:segments/common/sys.py:174-183 — divmod cascade, zero
    /// components dropped BEFORE the shorten slice, strip.
    #[test]
    fn format_uptime_forms() {
        assert_eq!(format_uptime(90061, 3), "1d 1h 1m"); // 1d1h1m1s, s cut by shorten
        assert_eq!(format_uptime(3661, 3), "1h 1m 1s");
        assert_eq!(format_uptime(61, 3), "1m 1s");
        assert_eq!(format_uptime(86_700, 3), "1d 5m"); // 0h dropped, not counted
        assert_eq!(format_uptime(59, 1), "59s");
        assert_eq!(format_uptime(0, 3), ""); // all-zero → empty → hidden
    }

    /// powerline:segments/common/players.py:31-33 — M:SS, python's
    /// "0:60" rounding quirk preserved.
    #[test]
    fn convert_seconds_forms() {
        assert_eq!(convert_seconds(203.417), "3:23");
        assert_eq!(convert_seconds(0.0), "0:00");
        assert_eq!(convert_seconds(59.6), "0:60"); // divmod + .0f quirk
    }

    /// powerline:segments/common/players.py:19-28 + 11-16.
    #[test]
    fn convert_state_and_symbols() {
        assert_eq!(convert_state("Playing"), "play");
        assert_eq!(convert_state("paused"), "pause");
        assert_eq!(convert_state("Stopped"), "stop");
        assert_eq!(convert_state("kNowhere"), "fallback");
        assert_eq!(state_symbol("play"), ">");
        assert_eq!(state_symbol("pause"), "~");
        assert_eq!(state_symbol("fallback"), "");
    }

    /// darwin if_data u32 wrap (libc apple b64/mod.rs:29-30).
    #[test]
    fn counter_delta_wrap() {
        assert_eq!(counter_delta(100, 350), 250);
        assert_eq!(counter_delta(u32::MAX as u64 - 10, 20), 31); // 4GiB wrap
    }

    /// net.py:202-228 — most-active fallback skips lo/vmnet/sit and
    /// non-alphabetic-prefix names; eth0 default when nothing matches.
    #[test]
    fn auto_interface_selection() {
        let counters = vec![
            ("lo0".to_string(), 900_000u64, 900_000u64),
            ("en0".to_string(), 5_000, 4_000),
            ("utun3".to_string(), 100, 100),
        ];
        assert_eq!(auto_interface(&counters), "en0");
        assert_eq!(auto_interface(&[]), "eth0"); // net.py:220 fallback
    }

    /// wttr.in response validation — real shapes pass, error pages don't.
    #[test]
    fn valid_weather_forms() {
        assert!(valid_weather("☀️ +33°C"));
        assert!(valid_weather("🌦 +12°F"));
        assert!(!valid_weather(""));
        assert!(!valid_weather("<html>rate limited</html>"));
        assert!(!valid_weather("Unknown location; please try again"));
        assert!(!valid_weather("line1\nline2 +3°C")); // multiline junk
    }

    /// Location URL encoding: alnum/‘+’ pass-through, spaces → '+',
    /// everything else percent-encoded.
    #[test]
    fn encode_location_forms() {
        assert_eq!(encode_location("Berlin"), "Berlin");
        assert_eq!(encode_location("New York"), "New+York");
        assert_eq!(encode_location("så"), "s%C3%A5");
    }
}
