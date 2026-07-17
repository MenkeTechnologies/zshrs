//! p10k SYSTEM segments — time / ram / load / disk_usage / swap /
//! battery / wifi / command_execution_time / vi_mode / proxy / todo /
//! timewarrior / taskwarrior / nix_shell / vim_shell /
//! midnight_commander.
//!
//! `~/forkedRepos/powerlevel10k/internal/p10k.zsh` is THE SPEC; ported
//! lines are cited as `// p10k:NNN`. Segment SEMANTICS (states, colors,
//! content, hide rules) mirror the zsh theme; the async worker layer
//! (`_p9k_worker_invoke` + `_p9k_print_params` round-trips) collapses
//! into synchronous reads behind a small TTL cache for the segments
//! that fork (vm_stat, pmset, todo.sh, task).
//!
//! Phase-1 scope notes (each also traced at the call site):
//! - battery: macOS `pmset -g batt` only; the Linux
//!   /sys/class/power_supply reader (p10k:1409-1459) is unported —
//!   hidden on Linux.
//! - wifi: Apple removed the private airport CLI p10k shells out to
//!   (p10k:5211-5224); segment is hidden rather than faked. TODO:
//!   CoreWLAN-equivalent data source.
//! - vi_mode: zshrs has no `vivis`/`vivli` keymaps (zsh core doesn't
//!   either); VISUAL is detected as vicmd + region_active, exactly the
//!   `vicmd1` arm of p10k:4203-4204.
//! - todo: the todo-file discovery subshell (p10k:8656-8672) is
//!   unported; presence of the todo.sh/todo-txt binary gates the
//!   segment and a failed `-p ls` parse hides it.
//!
//! Shared-helper duplication: color1/esc_pct/decode_g/seg_icon/
//! make_segment mirror segments_core.rs (private there; this module
//! may not edit other files). Hoisting them into a shared submodule is
//! a follow-up for the orchestrator.

use crate::extensions::p10k::config::{p9k_global, p9k_param};
use crate::extensions::p10k::icons;
use crate::extensions::p10k::render::Segment;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::utils::getkeystring;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

/// Segment dispatch per the p10k module contract. `None` = not handled
/// by this module; `Some(vec![])` = handled but hidden this prompt.
pub fn build_segment(name: &str) -> Option<Vec<Segment>> {
    match name {
        "time" => Some(time_segments()),                                     // p10k:3454
        "ram" => Some(ram_segments()),                                       // p10k:2689
        "load" => Some(load_segments()),                                     // p10k:2342
        "disk_usage" => Some(disk_usage_segments()),                         // p10k:1250
        "swap" => Some(swap_segments()),                                     // p10k:3360
        "battery" => Some(battery_segments()),                               // p10k:1340
        "wifi" => Some(wifi_segments()),                                     // p10k:5186
        "command_execution_time" => Some(command_execution_time_segments()), // p10k:1705
        "vi_mode" => Some(vi_mode_segments()),                               // p10k:4169
        "proxy" => Some(proxy_segments()),                                   // p10k:4970
        "todo" => Some(todo_segments()),                                     // p10k:3520
        "timewarrior" => Some(timewarrior_segments()),                       // p10k:5011
        "taskwarrior" => Some(taskwarrior_segments()),                       // p10k:5155
        "nix_shell" => Some(nix_shell_segments()),                           // p10k:4925
        "vim_shell" => Some(vim_shell_segments()),                           // p10k:4911
        "midnight_commander" => Some(midnight_commander_segments()),         // p10k:4871
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Shared helpers (mirror segments_core.rs — see module doc)
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

/// `_p9k_declare -i` read: unset/empty/unparseable → default.
fn global_int(name: &str, default: i64) -> i64 {
    getsparam(&format!("POWERLEVEL9K_{name}"))
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(default)
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
/// user-supplied icons/templates/mode strings (p10k:524 and every
/// `_p9k_declare -e`).
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
/// caller sits behind a TTL cache or a cheap-file short-circuit).
/// Mirrors segments_env::have_cmd's scan.
fn cmd_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env_or_param("PATH");
    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
        let cand = std::path::Path::new(dir).join(name);
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

// ---------------------------------------------------------------------
// Subprocess + TTL cache
// ---------------------------------------------------------------------

/// Replacement for p10k's `_p9k_worker_invoke` async recompute loop
/// (p10k:2704/1357/…): the zsh theme refreshes these values in a
/// background worker; here the subprocess runs synchronously at most
/// once per TTL. `None` results (spawn/parse failure) are cached too,
/// so a broken tool can't fork on every prompt. Bounded by the number
/// of `&'static str` keys (one per tool).
static TTL_CACHE: OnceLock<Mutex<HashMap<&'static str, (Instant, Option<String>)>>> =
    OnceLock::new();

fn cached_ttl(
    key: &'static str,
    ttl: Duration,
    run: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let m = TTL_CACHE.get_or_init(Default::default);
    if let Ok(guard) = m.lock() {
        if let Some((at, val)) = guard.get(key) {
            if at.elapsed() < ttl {
                return val.clone();
            }
        }
    }
    let val = run();
    if let Ok(mut guard) = m.lock() {
        guard.insert(key, (Instant::now(), val.clone()));
    }
    val
}

/// Run an external command, stdout as a string (stderr dropped — the
/// zsh originals all `2>/dev/null`). None on spawn failure or non-zero
/// exit.
fn run_tool(bin: &std::path::Path, args: &[&str]) -> Option<String> {
    match std::process::Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(out) if out.status.success() => Some(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!(target: "p10k", bin = %bin.display(), %e, "tool spawn failed");
            None
        }
    }
}

// ---------------------------------------------------------------------
// Pure formatters (ported; unit-tested below)
// ---------------------------------------------------------------------

/// Port of `_p9k_human_readable_bytes` (p10k:327-343).
fn human_readable_bytes(bytes: f64) -> String {
    // p10k:322 — __p9k_byte_suffix=('B' 'K' 'M' 'G' 'T' 'P' 'E' 'Z' 'Y')
    const SUFFIX: [&str; 9] = ["B", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let mut n = bytes;
    let mut suf = SUFFIX[0];
    for s in SUFFIX {
        suf = s;
        // p10k:331 — (( n < 1024 )) && break
        if n < 1024.0 {
            break;
        }
        n /= 1024.0; // p10k:332
    }
    let mut ret = if n >= 100.0 {
        format!("{n:.0}.") // p10k:335 — printf '%.0f.' (dot guards the zeros)
    } else if n >= 10.0 {
        format!("{n:.1}") // p10k:337
    } else {
        format!("{n:.2}") // p10k:339
    };
    // p10k:341 — ${${_p9k__ret%%0#}%.}: strip trailing zeros, then dot.
    while ret.ends_with('0') {
        ret.pop();
    }
    if ret.ends_with('.') {
        ret.pop();
    }
    format!("{ret}{suf}")
}

/// The duration formatter inlined in `prompt_command_execution_time`
/// (p10k:1709-1744). `precision` = COMMAND_EXECUTION_TIME_PRECISION,
/// `format` = COMMAND_EXECUTION_TIME_FORMAT ("H:M:S" or "d h m s").
fn human_readable_duration(seconds: f64, precision: usize, format: &str) -> String {
    if seconds < 60.0 {
        // p10k:1710-1715
        if precision == 0 {
            // p10k:1712 — local -i sec=$((P + 0.5)): add-then-truncate.
            let sec = (seconds + 0.5) as i64;
            format!("{sec}s")
        } else {
            // p10k:1714 — typeset -F $PRECISION sec prints fixed decimals.
            format!("{seconds:.precision$}s")
        }
    } else {
        // p10k:1718 — local -i d=$((P + 0.5))
        let d = (seconds + 0.5) as i64;
        if format == "H:M:S" {
            // p10k:1719-1729
            let mut text = format!("{:02}", d % 60); // p10k:1720 ${(l.2..0.)...}
            if d >= 60 {
                text = format!("{:02}:{text}", d / 60 % 60); // p10k:1722
                if d >= 36000 {
                    text = format!("{}:{text}", d / 3600); // p10k:1724
                } else if d >= 3600 {
                    text = format!("0{}:{text}", d / 3600); // p10k:1726
                }
            }
            text
        } else {
            // p10k:1730-1741 — "d h m s"
            let mut text = format!("{}s", d % 60); // p10k:1731
            if d >= 60 {
                text = format!("{}m {text}", d / 60 % 60); // p10k:1733
                if d >= 3600 {
                    text = format!("{}h {text}", d / 3600 % 24); // p10k:1735
                    if d >= 86400 {
                        text = format!("{}d {text}", d / 86400); // p10k:1737
                    }
                }
            }
            text
        }
    }
}

// ---------------------------------------------------------------------
// time (p10k:3451-3468)
// ---------------------------------------------------------------------

fn time_segments() -> Vec<Segment> {
    // p10k:7556 — _p9k_declare -e POWERLEVEL9K_TIME_FORMAT "%D{%H:%M:%S}".
    let fmt = decode_g(&p9k_global("TIME_FORMAT", "%D{%H:%M:%S}"));
    // p10k:3453 (REALTIME) and p10k:3456-3459 (precmd ${(%)...}) both
    // reduce to "let the prompt expander render the format": this
    // engine rebuilds PROMPT every precmd, so passing the escape
    // through IS the precmd-snapshot behavior. TIME_UPDATE_ON_COMMAND
    // (p10k:3461-3465, default 0 per p10k:7560) needs the
    // line-finished redraw hook and is unported.
    if global_bool("TIME_UPDATE_ON_COMMAND", false) {
        tracing::debug!(target: "p10k", "TIME_UPDATE_ON_COMMAND unported — static time per prompt");
    }
    // p10k:3453/3465 — `$0 "$_p9k_color2" "$_p9k_color1" "TIME_ICON" …`
    vec![make_segment("time", None, color2(), color1(), "TIME_ICON", fmt)]
}

// ---------------------------------------------------------------------
// vi_mode (p10k:4169-4210)
// ---------------------------------------------------------------------

fn vi_mode_segments() -> Vec<Segment> {
    // _p9k__keymap is captured from $KEYMAP in p10k's zle-line-init;
    // here the live ZLE keymap name answers directly.
    let keymap = crate::ported::zle::zle_keymap::curkeymapname().clone();
    // p10k:7500-7505 — mode strings; INSERT/COMMAND have declared
    // defaults, VISUAL/OVERWRITE exist only when the user sets them.
    let insert_str = decode_g(
        &getsparam("POWERLEVEL9K_VI_INSERT_MODE_STRING").unwrap_or_else(|| "INSERT".into()),
    );
    let command_str = decode_g(
        &getsparam("POWERLEVEL9K_VI_COMMAND_MODE_STRING").unwrap_or_else(|| "NORMAL".into()),
    );
    let visual_str = getsparam("POWERLEVEL9K_VI_VISUAL_MODE_STRING");
    let overwrite_str = getsparam("POWERLEVEL9K_VI_OVERWRITE_MODE_STRING");

    if keymap == "vicmd" {
        // p10k:4203-4204 — with VISUAL string set: vicmd + region==1 is
        // VISUAL, vicmd + region==0 is NORMAL (the `vicmd1`/`vicmd0`
        // patterns; no vivis/vivli keymaps in zshrs — module doc).
        if let Some(v) = &visual_str {
            if crate::ported::zle::zle_params::get_region_active() != 0 {
                return vec![make_segment(
                    "vi_mode",
                    Some("VISUAL"),
                    color1(),
                    "white",
                    "",
                    decode_g(v),
                )];
            }
        }
        // p10k:4203/4206 — `$0_NORMAL "$_p9k_color1" white '' … COMMAND_MODE_STRING`
        vec![make_segment(
            "vi_mode",
            Some("NORMAL"),
            color1(),
            "white",
            "",
            command_str,
        )]
    } else {
        // p10k:4192-4195 — with OVERWRITE string set, an insert-family
        // keymap in overwrite state shows OVERWRITE. _p9k__zle_state's
        // insert/overwrite bit is zle_main::INSMODE (Src/Zle/zle_main.c
        // insmode; 0 == overwrite).
        if let Some(o) = &overwrite_str {
            if crate::ported::zle::zle_main::INSMODE.load(Ordering::Relaxed) == 0 {
                return vec![make_segment(
                    "vi_mode",
                    Some("OVERWRITE"),
                    color1(),
                    "blue",
                    "",
                    decode_g(o),
                )];
            }
        }
        // p10k:4197 — INSERT only when the string is non-empty.
        if insert_str.is_empty() {
            return vec![];
        }
        // p10k:4198 — `$0_INSERT "$_p9k_color1" blue '' … INSERT_MODE_STRING`
        vec![make_segment(
            "vi_mode",
            Some("INSERT"),
            color1(),
            "blue",
            "",
            insert_str,
        )]
    }
}

// ---------------------------------------------------------------------
// command_execution_time (p10k:1705-1747)
// ---------------------------------------------------------------------

fn command_execution_time_segments() -> Vec<Segment> {
    // p10k:1706 — (( $+P9K_COMMAND_DURATION_SECONDS )) || return.
    let Some(dur) = crate::p10k::last_exec_duration() else {
        return vec![];
    };
    let secs = dur.as_secs_f64();
    // p10k:1707 + 7307 — threshold, float, default 3.
    if secs < global_float("COMMAND_EXECUTION_TIME_THRESHOLD", 3.0) {
        return vec![];
    }
    // p10k:7308 — precision default 2; p10k:7310 — format default H:M:S.
    let precision = global_int("COMMAND_EXECUTION_TIME_PRECISION", 2).max(0) as usize;
    let fmt = p9k_global("COMMAND_EXECUTION_TIME_FORMAT", "H:M:S");
    let text = human_readable_duration(secs, precision, &fmt);
    // p10k:1746 — `$0 "red" "yellow1" 'EXECUTION_TIME_ICON' 0 '' $text`
    vec![make_segment(
        "command_execution_time",
        None,
        "red",
        "yellow1",
        "EXECUTION_TIME_ICON",
        text,
    )]
}

// ---------------------------------------------------------------------
// load (p10k:2342-2364)
// ---------------------------------------------------------------------

/// `_p9k_num_cpus` (p10k:8436-8440): hw.logicalcpu on macOS, nproc on
/// Linux — both logical-CPU counts, which is exactly what
/// available_parallelism reports. Fork-free.
fn num_cpus() -> f64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as f64)
        .unwrap_or(1.0) // p10k:8440 — (( _p9k_num_cpus )) || _p9k_num_cpus=1
}

fn load_segments() -> Vec<Segment> {
    // getloadavg(3) replaces both data sources: `sysctl vm.loadavg`
    // (p10k:2383) on macOS and /proc/loadavg (p10k:2352-2354) on Linux.
    let mut avg = [0f64; 3];
    if unsafe { libc::getloadavg(avg.as_mut_ptr(), 3) } != 3 {
        tracing::warn!(target: "p10k", "getloadavg failed — load segment hidden");
        return vec![];
    }
    // p10k:7405-7410 — LOAD_WHICH (default 5): 1→1min, 15→15min,
    // anything else→5min (the 1-based field-index remap).
    let idx = match global_int("LOAD_WHICH", 5) {
        1 => 0,
        15 => 2,
        _ => 1,
    };
    let load = avg[idx];
    // p10k:2355/2390 — pct = 100. * load / _p9k_num_cpus.
    let pct = 100.0 * load / num_cpus();
    // p10k:2356-2361 + 7411-7412 — WARNING_PCT 50, CRITICAL_PCT 70.
    let (state, bg) = if pct > global_float("LOAD_CRITICAL_PCT", 70.0) {
        ("CRITICAL", "red") // p10k:2345/2357
    } else if pct > global_float("LOAD_WARNING_PCT", 50.0) {
        ("WARNING", "yellow") // p10k:2346/2359
    } else {
        ("NORMAL", "green") // p10k:2347/2361
    };
    // Two decimals matches both zsh data sources verbatim (sysctl
    // vm.loadavg and /proc/loadavg print %.2f).
    vec![make_segment(
        "load",
        Some(state),
        bg,
        color1(),
        "LOAD_ICON",
        format!("{load:.2}"),
    )]
}

// ---------------------------------------------------------------------
// ram (p10k:2689-2748)
// ---------------------------------------------------------------------

/// macOS available-RAM: `vm_stat` free+inactive pages × pagesize
/// (p10k:2715-2727), 10s TTL replacing the worker refresh loop.
#[cfg(target_os = "macos")]
fn ram_free_bytes() -> Option<f64> {
    let out = cached_ttl("ram.vm_stat", Duration::from_secs(10), || {
        run_tool(&cmd_on_path("vm_stat")?, &[])
    })?;
    fn pages(out: &str, key: &str) -> Option<f64> {
        // vm_stat lines: "Pages free:               123456."
        let rest = out.lines().find_map(|l| l.strip_prefix(key))?;
        rest.trim().trim_end_matches('.').parse::<f64>().ok()
    }
    // p10k:2717-2720 — Pages free + Pages inactive.
    let free = pages(&out, "Pages free:")? + pages(&out, "Pages inactive:")?;
    // p10k:2721-2726 — pagesize via the `pagesize` tool, default 4096;
    // sysconf(_SC_PAGESIZE) is the same value without the fork.
    let pagesize = match unsafe { libc::sysconf(libc::_SC_PAGESIZE) } {
        n if n > 0 => n as f64,
        _ => 4096.0,
    };
    Some(free * pagesize)
}

/// Linux available-RAM: /proc/meminfo MemAvailable (MemFree fallback),
/// kB → bytes (p10k:2734-2738).
#[cfg(not(target_os = "macos"))]
fn ram_free_bytes() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/meminfo").ok()?;
    let avail = meminfo_kb(&stat, "MemAvailable:").or_else(|| meminfo_kb(&stat, "MemFree:"))?;
    Some(avail * 1024.0) // p10k:2738 — match[2] * 1024
}

/// /proc/meminfo field in kB — "MemAvailable:    12345 kB".
#[cfg(not(target_os = "macos"))]
fn meminfo_kb(stat: &str, key: &str) -> Option<f64> {
    let rest = stat.lines().find_map(|l| l.strip_prefix(key))?;
    rest.trim()
        .trim_end_matches("kB")
        .trim()
        .parse::<f64>()
        .ok()
}

fn ram_segments() -> Vec<Segment> {
    let Some(free) = ram_free_bytes() else {
        return vec![]; // p10k:2691 cond '$_p9k__ram_free' — hidden on failure
    };
    // p10k:2691 — `$0 yellow "$_p9k_color1" RAM_ICON …`
    vec![make_segment(
        "ram",
        None,
        "yellow",
        color1(),
        "RAM_ICON",
        human_readable_bytes(free),
    )]
}

// ---------------------------------------------------------------------
// swap (p10k:3360-3407)
// ---------------------------------------------------------------------

/// macOS swap-used: sysctlbyname("vm.swapusage") xsu_used — the same
/// value p10k regex-parses out of `sysctl vm.swapusage` "used = …"
/// (p10k:3384-3392), read as the struct instead of a fork.
#[cfg(target_os = "macos")]
fn swap_used_bytes() -> Option<f64> {
    let name = std::ffi::CString::new("vm.swapusage").ok()?;
    let mut xsw: libc::xsw_usage = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::xsw_usage>();
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut xsw as *mut _ as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    Some(xsw.xsu_used as f64)
}

/// Linux swap-used: /proc/meminfo SwapTotal − SwapFree, kB → bytes
/// (p10k:3394-3400).
#[cfg(not(target_os = "macos"))]
fn swap_used_bytes() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/meminfo").ok()?;
    let total = meminfo_kb(&stat, "SwapTotal:")?;
    let free = meminfo_kb(&stat, "SwapFree:")?;
    Some((total - free) * 1024.0)
}

fn swap_segments() -> Vec<Segment> {
    let Some(used) = swap_used_bytes() else {
        return vec![]; // p10k:3362 cond '$_p9k__swap_used'
    };
    // p10k:3362 — `$0 yellow "$_p9k_color1" SWAP_ICON …`
    vec![make_segment(
        "swap",
        None,
        "yellow",
        color1(),
        "SWAP_ICON",
        human_readable_bytes(used),
    )]
}

// ---------------------------------------------------------------------
// disk_usage (p10k:1250-1292)
// ---------------------------------------------------------------------

fn disk_usage_segments() -> Vec<Segment> {
    // p10k:1274 — `df -P $cwd`, capacity column = ceil(100*used/(used+avail));
    // statvfs(2) on the logical cwd is the fork-free equivalent.
    let cwd = env_or_param("PWD");
    let cwd = if cwd.is_empty() { ".".to_string() } else { cwd };
    let Ok(c) = std::ffi::CString::new(cwd) else {
        return vec![];
    };
    let mut vfs: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut vfs) } != 0 {
        return vec![]; // p10k:1275 — a non-<0-100> parse hides the segment
    }
    let used = (vfs.f_blocks as u64).saturating_sub(vfs.f_bfree as u64);
    let total = used + vfs.f_bavail as u64;
    if total == 0 {
        return vec![];
    }
    let pct = (used * 100).div_ceil(total) as i64; // df -P rounds up
    // p10k:1279-1285 + 7263-7265 — WARNING 90, CRITICAL 95, ONLY_WARNING.
    let (state, bg, fg): (&str, &str, &str) = if pct >= global_int("DISK_USAGE_CRITICAL_LEVEL", 95)
    {
        ("CRITICAL", "red", "white") // p10k:1252
    } else if pct >= global_int("DISK_USAGE_WARNING_LEVEL", 90) {
        ("WARNING", "yellow", color1()) // p10k:1253
    } else if !global_bool("DISK_USAGE_ONLY_WARNING", false) {
        ("NORMAL", color1(), "yellow") // p10k:1255
    } else {
        return vec![]; // p10k:1254 — NORMAL suppressed under ONLY_WARNING
    };
    vec![make_segment(
        "disk_usage",
        Some(state),
        bg,
        fg,
        "DISK_ICON",
        format!("{pct}%%"), // p10k:1252 — '$_p9k__disk_usage_pct%%'
    )]
}

// ---------------------------------------------------------------------
// battery (p10k:1340-1497)
// ---------------------------------------------------------------------

/// Battery array param with the `<STATE>` → base fallback p10k builds
/// at declare time (p10k:7269-7285). `split_scalar` mirrors the STAGES
/// scalar form `("${(@s::)${(g::)...}}")` (p10k:7272) — a scalar
/// splits to one glyph per character.
fn battery_arr(state: &str, key: &str, split_scalar: bool) -> Vec<String> {
    for name in [
        format!("POWERLEVEL9K_BATTERY_{state}_{key}"),
        format!("POWERLEVEL9K_BATTERY_{key}"),
    ] {
        if let Some(v) = getaparam(&name) {
            return v.iter().map(|s| decode_g(s)).collect(); // p10k:7273 (@g::)
        }
        if let Some(s) = getsparam(&name) {
            if split_scalar {
                return decode_g(&s).chars().map(String::from).collect();
            }
            return vec![decode_g(&s)];
        }
    }
    Vec::new()
}

/// p10k:1472-1474/1480-1482/1488-1490 — pick from a level array:
/// idx = len when pct==100, else pct*len/100 + 1 (1-based).
fn battery_level_pick(arr: &[String], pct: i64) -> Option<String> {
    if arr.is_empty() {
        return None;
    }
    let mut idx = arr.len() as i64;
    if pct < 100 {
        idx = pct * arr.len() as i64 / 100 + 1;
    }
    arr.get((idx - 1).max(0) as usize).cloned()
}

/// macOS battery probe — `pmset -g batt` second line (p10k:1381-1406),
/// 30s TTL. Returns (state, percent, remain).
#[cfg(target_os = "macos")]
fn battery_status() -> Option<(&'static str, i64, String)> {
    let raw = cached_ttl("battery.pmset", Duration::from_secs(30), || {
        let out = run_tool(&cmd_on_path("pmset")?, &["-g", "batt"])?;
        // p10k:1383 — ${${(Af)"$(pmset -g batt)"}[2]}: the second line.
        out.lines().nth(1).map(str::to_string)
    })?;
    // p10k:1384 — [[ $raw_data == *InternalBattery* ]] || return
    if !raw.contains("InternalBattery") {
        return None; // no battery (desktop) — hidden
    }
    let fields: Vec<&str> = raw.split("; ").collect();
    // p10k:1385-1386 — remain = first word of field 3; *no* → "...".
    let mut remain = fields
        .get(2)
        .and_then(|f| f.split_whitespace().next())
        .unwrap_or("")
        .to_string();
    if remain.contains("no") {
        remain = "...".to_string();
    }
    // p10k:1387 — [[ $raw_data =~ '([0-9]+)%' ]]: the digit run
    // ending at the first '%'.
    let pos = raw.find('%')?;
    let digits: &str = &raw[..pos];
    let start = digits
        .rfind(|c: char| !c.is_ascii_digit())
        .map(|i| i + digits[i..].chars().next().map_or(1, |c| c.len_utf8()))
        .unwrap_or(0);
    let bat_percent: i64 = digits[start..].parse().ok()?;
    // p10k:1389-1405 — state ladder from field 2.
    let state = match fields.get(1).copied().unwrap_or("") {
        "charging" | "finishing charge" | "AC attached" => {
            if bat_percent == 100 {
                remain.clear(); // p10k:1392-1394
                "CHARGED"
            } else {
                "CHARGING" // p10k:1396
            }
        }
        "discharging" => {
            // p10k:1400 + 7266 — LOW_THRESHOLD default 10.
            if bat_percent < global_int("BATTERY_LOW_THRESHOLD", 10) {
                "LOW"
            } else {
                "DISCONNECTED"
            }
        }
        _ => {
            remain.clear(); // p10k:1402-1404
            "CHARGED"
        }
    };
    Some((state, bat_percent, remain))
}

/// Linux battery: the /sys/class/power_supply reader (p10k:1409-1459)
/// is unported — hidden. TODO(phase-2).
#[cfg(not(target_os = "macos"))]
fn battery_status() -> Option<(&'static str, i64, String)> {
    tracing::debug!(
        target: "p10k",
        "battery: Linux /sys/class/power_supply reader unported — hidden"
    );
    None
}

fn battery_segments() -> Vec<Segment> {
    let Some((state, bat_percent, remain)) = battery_status() else {
        return vec![];
    };
    // p10k:1462 + 7267/7278 — per-state HIDE_ABOVE_THRESHOLD, default
    // from the base param, default 999.
    let base_hide = global_int("BATTERY_HIDE_ABOVE_THRESHOLD", 999);
    if bat_percent >= global_int(&format!("BATTERY_{state}_HIDE_ABOVE_THRESHOLD"), base_hide) {
        return vec![];
    }
    // p10k:1464-1465 — msg; VERBOSE (default 1, p10k:7268) appends
    // " (remain)".
    let mut msg = format!("{bat_percent}%%");
    if global_bool("BATTERY_VERBOSE", true) && !remain.is_empty() {
        msg.push_str(&format!(" ({remain})"));
    }
    // p10k:1470-1476 — a STAGES glyph beats BATTERY_ICON.
    let stage = battery_level_pick(&battery_arr(state, "STAGES", true), bat_percent);
    let icon_glyph = match &stage {
        Some(g) => g.clone(),
        None => seg_icon("battery", Some(state), "BATTERY_ICON"),
    };
    // p10k:1478-1483 — LEVEL_BACKGROUND array, else color1.
    let default_bg =
        battery_level_pick(&battery_arr(state, "LEVEL_BACKGROUND", false), bat_percent)
            .unwrap_or_else(|| color1().to_string());
    // p10k:1486-1491 + 8403-8408 — LEVEL_FOREGROUND array, else the
    // _p9k_battery_states color.
    let state_fg = match state {
        "LOW" => "red",
        "CHARGING" => "yellow",
        "CHARGED" => "green",
        _ => color2(), // DISCONNECTED — p10k:8407 "$_p9k_color2"
    };
    let default_fg =
        battery_level_pick(&battery_arr(state, "LEVEL_FOREGROUND", false), bat_percent)
            .unwrap_or_else(|| state_fg.to_string());
    // p10k:1495 — `prompt_battery_$state "$bg" "$fg" $icon 0 '' $msg`.
    // Built directly (not make_segment) because the icon is a resolved
    // glyph, not a key.
    let bg = p9k_param("battery", Some(state), "BACKGROUND", &default_bg);
    let fg = p9k_param("battery", Some(state), "FOREGROUND", &default_fg);
    let icon = apply_visual_identifier("battery", Some(state), icon_glyph);
    let content = apply_content_expansion("battery", Some(state), msg);
    vec![Segment {
        name: "battery".to_string(),
        state: Some(state.to_string()),
        content,
        icon,
        fg,
        bg,
    }]
}

// ---------------------------------------------------------------------
// wifi (p10k:5186-5240)
// ---------------------------------------------------------------------

fn wifi_segments() -> Vec<Segment> {
    // p10k:5192-5195 shells out to the private Apple80211 `airport`
    // CLI, which Apple removed (macOS 14.4+); the Linux path needs
    // /proc/net/wireless + iw (p10k:5226-5240). TODO: CoreWLAN-
    // equivalent data source. Hidden rather than faked.
    tracing::debug!(target: "p10k", "wifi segment: airport CLI gone — hidden (TODO)");
    vec![]
}

// ---------------------------------------------------------------------
// proxy (p10k:4970-4980)
// ---------------------------------------------------------------------

/// p10k:4974 — `${(@)${(@)${(@)p#*://}##*@}%%/*}`: strip scheme,
/// credentials, path.
fn proxy_strip(v: &str) -> String {
    let v = v.split_once("://").map_or(v, |(_, r)| r); // ${p#*://}
    let v = v.rsplit_once('@').map_or(v, |(_, r)| r); // ${p##*@}
    v.split('/').next().unwrap_or(v).to_string() // ${p%%/*}
}

fn proxy_segments() -> Vec<Segment> {
    // p10k:4971-4973 + 4981 — the exact 8-var list (the init cond is
    // their concatenation: any set+non-empty var shows the segment).
    const VARS: [&str; 8] = [
        "all_proxy",
        "http_proxy",
        "https_proxy",
        "ftp_proxy",
        "ALL_PROXY",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "FTP_PROXY",
    ];
    let mut hosts: Vec<String> = Vec::new(); // p10k:4971 `local -U p` — unique
    for var in VARS {
        let v = env_or_param(var);
        if v.is_empty() {
            continue;
        }
        let host = proxy_strip(&v);
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    if hosts.is_empty() {
        return vec![]; // init cond empty — segment hidden
    }
    // p10k:4975 — (( $#p == 1 )) || p=(""): ambiguous → icon only.
    let content = if hosts.len() == 1 {
        esc_pct(&hosts[0])
    } else {
        String::new()
    };
    // p10k:4976 — `$0 $_p9k_color1 blue PROXY_ICON 0 '' "$p[1]"`
    vec![make_segment(
        "proxy",
        None,
        color1(),
        "blue",
        "PROXY_ICON",
        content,
    )]
}

// ---------------------------------------------------------------------
// todo (p10k:3520-3543)
// ---------------------------------------------------------------------

fn todo_segments() -> Vec<Segment> {
    // p10k:8656-8659 — todo.sh, then todo-txt. The config-sourcing
    // subshell that locates $TODO_FILE (p10k:8662-8671) is unported;
    // a missing todo file makes the count parse fail → hidden.
    let Some(bin) = cmd_on_path("todo.sh").or_else(|| cmd_on_path("todo-txt")) else {
        return vec![]; // p10k:3546 cond '$_p9k__todo_file'
    };
    // p10k:3524 — `$_p9k__todo_command -p ls | command tail -1`; the
    // todo-file mtime cache (p10k:3523) becomes a 30s TTL.
    let Some(last) = cached_ttl("todo.count", Duration::from_secs(30), || {
        let out = run_tool(&bin, &["-p", "ls"])?;
        out.lines().last().map(str::to_string)
    }) else {
        return vec![];
    };
    // p10k:3525 — 'TODO: '<filtered>' of '<total>' '*.
    let Some(rest) = last.strip_prefix("TODO: ") else {
        return vec![];
    };
    let mut it = rest.split_whitespace();
    let (Some(filtered), Some("of"), Some(total)) = (it.next(), it.next(), it.next()) else {
        return vec![];
    };
    let (Ok(filtered), Ok(total)) = (filtered.parse::<i64>(), total.parse::<i64>()) else {
        return vec![];
    };
    // p10k:3533-3535 + 7225-7226 — zero-hiding gates, both default off.
    if (total == 0 && global_bool("TODO_HIDE_ZERO_TOTAL", false))
        || (filtered == 0 && global_bool("TODO_HIDE_ZERO_FILTERED", false))
    {
        return vec![];
    }
    // p10k:3536-3540 — "N" when the counts agree, else "F/T".
    let text = if total == filtered {
        total.to_string()
    } else {
        format!("{filtered}/{total}")
    };
    // p10k:3541 — `$0 "grey50" "$_p9k_color1" 'TODO_ICON' 0 '' "$text"`
    vec![make_segment(
        "todo",
        None,
        "grey50",
        color1(),
        "TODO_ICON",
        text,
    )]
}

// ---------------------------------------------------------------------
// timewarrior (p10k:5011-5059)
// ---------------------------------------------------------------------

fn timewarrior_segments() -> Vec<Segment> {
    timewarrior_inner().unwrap_or_default()
}

fn timewarrior_inner() -> Option<Vec<Segment>> {
    // p10k:5062 — init cond '$commands[timew]'.
    cmd_on_path("timew")?;
    // p10k:5013 — ${TIMEWARRIORDB:-~/.timewarrior}/data.
    let db = env_or_param("TIMEWARRIORDB");
    let dir = if db.is_empty() {
        PathBuf::from(env_or_param("HOME")).join(".timewarrior")
    } else {
        PathBuf::from(db)
    }
    .join("data");
    // p10k:5031-5044 — newest <->-<->.data file by name
    // (${(AO)files}[1] = descending sort). The mtime memo layer
    // (p10k:5015-5022) is dropped: one readdir + one small read per
    // prompt.
    let rd = std::fs::read_dir(&dir).ok()?; // p10k:5024-5027 — no dir, hidden
    let is_data_name = |n: &str| {
        n.strip_suffix(".data").is_some_and(|stem| {
            stem.split_once('-').is_some_and(|(a, b)| {
                !a.is_empty()
                    && !b.is_empty()
                    && a.bytes().all(|c| c.is_ascii_digit())
                    && b.bytes().all(|c| c.is_ascii_digit())
            })
        })
    };
    let newest = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            is_data_name(&name).then_some(name)
        })
        .max()?;
    let content = std::fs::read_to_string(dir.join(&newest)).ok()?;
    // p10k:5051 — last line; p10k:5052 — 'inc '[^\ ]##(|\ #\#(*)): an
    // OPEN interval ("inc <ts>", optionally " # tags"); closed
    // intervals ("inc <ts> - <ts> …") do not match → hidden.
    let last = content.lines().rev().find(|l| !l.is_empty())?;
    let rest = last.strip_prefix("inc ")?;
    let (first, tail) = match rest.split_once(' ') {
        Some((f, t)) => (f, Some(t)),
        None => (rest, None),
    };
    if first.is_empty() {
        return None;
    }
    let tags = match tail {
        None => String::new(),
        Some(t) => {
            // p10k:5053 — ${${match[2]## #}%% #}: trim spaces around
            // the '#'-prefixed tag list.
            let t = t.trim_start_matches(' ');
            t.strip_prefix('#')?.trim_matches(' ').to_string()
        }
    };
    // p10k:5054 — `$0 grey 255 TIMEWARRIOR_ICON 0 '' "${tags//\%/%%}"`
    Some(vec![make_segment(
        "timewarrior",
        None,
        "grey",
        "255",
        "TIMEWARRIOR_ICON",
        esc_pct(&tags),
    )])
}

// ---------------------------------------------------------------------
// taskwarrior (p10k:5155-5180)
// ---------------------------------------------------------------------

fn taskwarrior_segments() -> Vec<Segment> {
    // p10k:5183 — init cond gates on $commands[task].
    let Some(bin) = cmd_on_path("task") else {
        return vec![];
    };
    // p10k:5136-5141 — `task +PENDING count` / `task +OVERDUE count`
    // (rc.color=0 rc._forcecolor=0, stdin null); only values ≥ 1
    // count. The mtime-signature cache (p10k:5111-5121) becomes a 30s
    // TTL over the assembled text.
    let text = cached_ttl("taskwarrior.counts", Duration::from_secs(30), || {
        let count = |tag: &str| -> Option<i64> {
            let out = run_tool(
                &bin,
                &[
                    &format!("+{tag}"),
                    "count",
                    "rc.color=0",
                    "rc._forcecolor=0",
                ],
            )?;
            let n: i64 = out.trim().parse().ok()?;
            (n >= 1).then_some(n) // p10k:5139 — [[ $val == <1-> ]]
        };
        // p10k:5165-5177 — "!O" then "/" then "P".
        let mut text = String::new();
        if let Some(o) = count("OVERDUE") {
            text.push_str(&format!("!{o}"));
        }
        if let Some(p) = count("PENDING") {
            if !text.is_empty() {
                text.push('/');
            }
            text.push_str(&p.to_string());
        }
        Some(text)
    })
    .unwrap_or_default();
    // p10k:5178 — [[ -n $text ]] || return.
    if text.is_empty() {
        return vec![];
    }
    // p10k:5179 — `$0 6 $_p9k_color1 TASKWARRIOR_ICON 0 '' $text`
    vec![make_segment(
        "taskwarrior",
        None,
        "6",
        color1(),
        "TASKWARRIOR_ICON",
        text,
    )]
}

// ---------------------------------------------------------------------
// nix_shell / vim_shell / midnight_commander (p10k:4925/4911/4871)
// ---------------------------------------------------------------------

fn nix_shell_segments() -> Vec<Segment> {
    // p10k:4930 — init cond '${IN_NIX_SHELL:#0}'.
    let v = env_or_param("IN_NIX_SHELL");
    if v.is_empty() || v == "0" {
        return vec![];
    }
    // p10k:4926 — content ${(M)IN_NIX_SHELL:#(pure|impure)}.
    let content = if v == "pure" || v == "impure" {
        v
    } else {
        String::new()
    };
    // p10k:4926 — `$0 4 $_p9k_color1 NIX_SHELL_ICON 0 '' …`
    vec![make_segment(
        "nix_shell",
        None,
        "4",
        color1(),
        "NIX_SHELL_ICON",
        content,
    )]
}

fn vim_shell_segments() -> Vec<Segment> {
    // p10k:4918 — init cond '$VIMRUNTIME'.
    if env_or_param("VIMRUNTIME").is_empty() {
        return vec![];
    }
    // p10k:4913 — `$0 green $_p9k_color1 VIM_ICON 0 '' ''`
    vec![make_segment(
        "vim_shell",
        None,
        "green",
        color1(),
        "VIM_ICON",
        String::new(),
    )]
}

fn midnight_commander_segments() -> Vec<Segment> {
    // p10k:4878 — init cond '$MC_TMPDIR'.
    if env_or_param("MC_TMPDIR").is_empty() {
        return vec![];
    }
    // p10k:4873 — `$0 $_p9k_color1 yellow MIDNIGHT_COMMANDER_ICON 0 '' ''`
    vec![make_segment(
        "midnight_commander",
        None,
        color1(),
        "yellow",
        "MIDNIGHT_COMMANDER_ICON",
        String::new(),
    )]
}

// ---------------------------------------------------------------------
// Tests — pure formatters only (literal inputs)
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// p10k:327-343 anchors — `_p9k_human_readable_bytes` output for
    /// exact byte counts (trailing-zero/dot stripping included).
    #[test]
    fn human_readable_bytes_literals() {
        assert_eq!(human_readable_bytes(0.0), "0B");
        assert_eq!(human_readable_bytes(512.0), "512B"); // %.0f. → "512." → "512B"
        assert_eq!(human_readable_bytes(1023.0), "1023B"); // no division below 1024
        assert_eq!(human_readable_bytes(1536.0), "1.5K"); // %.2f "1.50" → zeros stripped
        assert_eq!(human_readable_bytes(10.0 * 1024.0), "10K"); // "10.0" → "10." → "10"
        assert_eq!(human_readable_bytes(16000.0), "15.6K"); // 15.625 ≥ 10 → %.1f
        assert_eq!(human_readable_bytes(2.0 * 1024.0 * 1024.0), "2M"); // "2.00" → "2"
        assert_eq!(
            human_readable_bytes(100.0 * 1024.0 * 1024.0 * 1024.0),
            "100G" // "100." — the dot guards the zeros from stripping
        );
        assert_eq!(human_readable_bytes(1024f64.powi(4) * 1.25), "1.25T");
    }

    /// p10k:1709-1715 — sub-minute durations: precision 0 rounds via
    /// add-0.5-truncate; precision N prints fixed decimals.
    #[test]
    fn human_readable_duration_sub_minute() {
        assert_eq!(human_readable_duration(3.0, 0, "H:M:S"), "3s");
        assert_eq!(human_readable_duration(3.6, 0, "H:M:S"), "4s"); // 3.6+0.5=4.1 → 4
        assert_eq!(human_readable_duration(3.4, 0, "d h m s"), "3s"); // 3.4+0.5=3.9 → 3
        assert_eq!(human_readable_duration(3.5, 2, "H:M:S"), "3.50s"); // typeset -F 2
        assert_eq!(human_readable_duration(59.0, 1, "d h m s"), "59.0s");
    }

    /// p10k:1719-1729 — H:M:S: seconds/minutes zero-padded to 2,
    /// hours zero-padded only below 10h (`0$((d/3600))` vs bare).
    #[test]
    fn human_readable_duration_hms() {
        assert_eq!(human_readable_duration(95.4, 0, "H:M:S"), "01:35"); // d=95
        assert_eq!(human_readable_duration(3661.0, 0, "H:M:S"), "01:01:01");
        assert_eq!(human_readable_duration(36000.0, 0, "H:M:S"), "10:00:00"); // ≥36000 unpadded
        assert_eq!(human_readable_duration(90061.0, 0, "H:M:S"), "25:01:01"); // no day wrap
    }

    /// p10k:1730-1741 — "d h m s": unit words, hours wrap at 24.
    #[test]
    fn human_readable_duration_dhms() {
        assert_eq!(human_readable_duration(125.0, 0, "d h m s"), "2m 5s");
        assert_eq!(human_readable_duration(4225.0, 0, "d h m s"), "1h 10m 25s");
        assert_eq!(human_readable_duration(90061.0, 0, "d h m s"), "1d 1h 1m 1s");
    }

    /// p10k:4974 — proxy value stripping: scheme, credentials, path.
    #[test]
    fn proxy_strip_forms() {
        assert_eq!(
            proxy_strip("http://proxy.example.com:8080/x"),
            "proxy.example.com:8080"
        );
        assert_eq!(
            proxy_strip("socks5://user:pw@10.0.0.1:1080"),
            "10.0.0.1:1080"
        );
        assert_eq!(proxy_strip("proxy:3128"), "proxy:3128");
    }
}
