//! p10k CORE segments — dir / status / context / ssh / root_indicator /
//! history / prompt_char / ip / dir_writable / background_jobs /
//! custom_* / vcs.
//!
//! `~/forkedRepos/powerlevel10k/internal/p10k.zsh` is THE SPEC; ported
//! lines are cited as `// p10k:NNN`. This is src/extensions (Rust-native
//! layer), so there are no C-port constraints — but segment SEMANTICS
//! (states, colors, content, hide rules) mirror the zsh theme exactly.
//!
//! Notes:
//! - dir: every SHORTEN_STRATEGY (p10k:1815-2018) is ported, including
//!   the filesystem-anchored truncate_to_unique (width-driven, resolved
//!   in render.rs), truncate_with_folder_marker and
//!   truncate_with_package_name (jq).
//! - vcs: renders p10k's own gitstatus format (p10k:3926-4005) or, with
//!   VCS_DISABLE_GITSTATUS_FORMATTING, publishes VCS_STATUS_* for the
//!   user's CONTENT_EXPANSION formatter.

use crate::extensions::p10k::config::{p9k_global, p9k_global_arr, p9k_param};
use crate::extensions::p10k::git;
use crate::extensions::p10k::icons;
use crate::extensions::p10k::render::Segment;
use crate::extensions::p10k::shared::{color1, color2, env_or_param, global_bool, global_int, esc_pct, decode_g, seg_icon, apply_visual_identifier, apply_content_expansion, make_segment};
use crate::ported::params::{getsparam, pipestatgetfn};
use crate::ported::utils::getkeystring;
use std::path::Path;
use std::sync::atomic::Ordering;

// p10k markers used by prompt_dir's part rewriting:
// $'\1' — "shortened here" placeholder replaced by the delimiter.
const MARK_ELIDE: char = '\u{1}';
// $'\2' — trailing anchor marker (kept-component highlighting).
const MARK_ANCHOR: char = '\u{2}';
// $'\3' — truncate_to_unique bracket marker (stripped at assembly).
const MARK_UNIQ: char = '\u{3}';

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

/// Segment dispatch per the p10k module contract. `None` = not handled
/// by this module; `Some(vec![])` = handled but hidden this prompt.
pub fn build_segment(name: &str) -> Option<Vec<Segment>> {
    match name {
        "dir" => Some(dir_segments()),                         // p10k:1768
        "status" => Some(status_segments()),                   // p10k:3258
        "context" => Some(context_segments()),                 // p10k:1571
        "ssh" => Some(ssh_segments()),                         // p10k:3236
        "root_indicator" => Some(root_indicator_segments()),   // p10k:3134
        "history" => Some(history_segments()),                 // p10k:2211
        "prompt_char" => Some(prompt_char_segments()),         // p10k:3303
        "ip" => Some(ip_segments()),                           // p10k:2293
        "dir_writable" => Some(dir_writable_segments()),       // p10k:4437
        "background_jobs" => Some(background_jobs_segments()), // p10k:1234
        "vcs" => Some(vcs_segments()),                         // p10k:4131
        n if n.starts_with("custom_") => {
            // p10k:1691 _p9k_custom_prompt — invoked as
            // `prompt_custom_<name>` with $1 = <name>.
            Some(custom_segments(&n["custom_".len()..]))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------

/// p10k:585-587 `_p9k_background` — empty color means "default bg".
fn bgesc(c: &str) -> String {
    if c.is_empty() {
        "%k".to_string()
    } else {
        format!("%K{{{c}}}")
    }
}

/// p10k:589-594 `_p9k_foreground`.
fn fgesc(c: &str) -> String {
    if c.is_empty() {
        "%f".to_string()
    } else {
        format!("%F{{{c}}}")
    }
}

/// Logical cwd — p10k's `_p9k__cwd` is `$PWD` as the shell tracks it.
fn cwd() -> String {
    if let Some(p) = getsparam("PWD") {
        if !p.is_empty() {
            return p;
        }
    }
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn home_dir() -> String {
    env_or_param("HOME")
}

/// p10k:8498-8520 `_p9k_init_ssh` — `P9K_SSH=1` when SSH_CLIENT /
/// SSH_TTY / SSH_CONNECTION is set; otherwise (a user switched with su
/// on a remote host loses them) the login line of `who` is inspected
/// once for a remote address (computed once, like the init-time probe).
pub(crate) fn is_ssh() -> bool {
    if !env_or_param("SSH_CLIENT").is_empty()
        || !env_or_param("SSH_TTY").is_empty()
        || !env_or_param("SSH_CONNECTION").is_empty()
    {
        return true;
    }
    static VIA_WHO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VIA_WHO.get_or_init(ssh_via_who)
}

/// p10k:8508-8520 — remote address (IPv4, IPv6 or a hostname with two
/// non-consecutive dots) at the end of the `who -m` line, or of the
/// `who` line for this tty when `who -m` fails.
fn ssh_via_who() -> bool {
    use std::process::{Command, Stdio};
    if crate::extensions::p10k::segments_sys::cmd_on_path("who").is_none() {
        return false;
    }
    let who = |args: &[&str]| {
        Command::new("who")
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let w = match who(&["-m"]) {
        Some(out) => out.trim_end_matches('\n').to_string(),
        None => {
            let tty = env_or_param("TTY");
            let tty = tty.strip_prefix("/dev/").unwrap_or(&tty).to_string();
            who(&[])
                .unwrap_or_default()
                .lines()
                .filter(|l| {
                    // `*[[:space:]]$tty[[:space:]]*`
                    let f: Vec<&str> = l.split_whitespace().collect();
                    f.len() > 2 && f[1..f.len() - 1].contains(&tty.as_str())
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    };
    let ipv6 = "(([0-9a-fA-F]+:)|:){2,}[0-9a-fA-F]+";
    let ipv4 = r"([0-9]{1,3}\.){3}[0-9]+";
    let hostname = r"([.][^. ]+){2}";
    regex::Regex::new(&format!(r"\(?({ipv4}|{ipv6}|{hostname})\)?$"))
        .is_ok_and(|re| re.is_match(&w))
}

fn is_root() -> bool {
    // Prompt-escape %# root test — geteuid()==0 (Src/prompt.c '#').
    unsafe { libc::geteuid() == 0 }
}

/// `[[ -w $path ]]` — access(2) W_OK, matching the zsh -w condition.
fn path_writable(path: &str) -> bool {
    match std::ffi::CString::new(path) {
        Ok(c) => unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 },
        Err(_) => false,
    }
}

/// Active job count — the `%j` prompt escape body
/// (src/ported/prompt.rs:1537-1558, Src/prompt.c:563-570): jobs with a
/// nonzero stat, at least one proc, and no STAT_NOPRINT.
fn job_count() -> i32 {
    let mut numjobs = 0i32;
    if let Some(tab_lock) = crate::ported::jobs::JOBTAB.get() {
        if let Ok(tab) = tab_lock.lock() {
            let max = crate::ported::jobs::MAXJOB
                .get()
                .and_then(|m| m.lock().ok().map(|g| *g))
                .unwrap_or(0);
            let mut j = 1usize;
            while j <= max && j < tab.len() {
                let jb = &tab[j];
                if jb.stat != 0
                    && !jb.procs.is_empty()
                    && (jb.stat & crate::ported::zsh_h::STAT_NOPRINT) == 0
                {
                    numjobs += 1;
                }
                j += 1;
            }
        }
    }
    numjobs
}

// ---------------------------------------------------------------------
// context — user@host (p10k:1571-1632)
// ---------------------------------------------------------------------

fn context_segments() -> Vec<Segment> {
    let ssh = is_ssh();
    let default_user = env_or_param("DEFAULT_USER");
    // ${(%):-%n} — the current (effective) username special.
    let user = getsparam("USERNAME").unwrap_or_default();

    let always_show_context = global_bool("ALWAYS_SHOW_CONTEXT", false); // p10k:7302
    let always_show_user = global_bool("ALWAYS_SHOW_USER", false); // p10k:7303

    // p10k:1575-1580 — content collapses to just the username when it
    // matches DEFAULT_USER off-SSH…
    let mut content = String::new();
    if !always_show_context && !default_user.is_empty() && !ssh && user == default_user {
        // p10k:1624-1632 _p9k_prompt_context_init — …and the whole
        // segment is hidden unless ALWAYS_SHOW_USER.
        if !always_show_user {
            return vec![];
        }
        content = esc_pct(&user); // p10k:1578
    }

    // p10k:1582-1593 — state ladder; p10k:1596 splits ROOT off via the
    // `%#` prompt-cond pair, resolved here directly from euid.
    let sudo = !env_or_param("SUDO_COMMAND").is_empty();
    let state = if is_root() {
        "ROOT"
    } else if ssh && sudo {
        "REMOTE_SUDO"
    } else if ssh {
        "REMOTE"
    } else if sudo {
        "SUDO"
    } else {
        "DEFAULT"
    };

    // p10k:1598-1606 — per-state template, else the shared template.
    let text = if content.is_empty() {
        let per_state = getsparam(&format!("POWERLEVEL9K_CONTEXT_{state}_TEMPLATE"));
        match per_state {
            Some(t) => decode_g(&t), // p10k:1602 ${(g::)text}
            None => decode_g(&p9k_global("CONTEXT_TEMPLATE", "%n@%m")), // p10k:7304 default
        }
    } else {
        content
    };

    // p10k:1607 — `_p9k_prompt_segment "$0_$state" "$_p9k_color1" yellow '' …`
    vec![make_segment(
        "context",
        Some(state),
        color1(),
        "yellow",
        "",
        text,
    )]
}

// ---------------------------------------------------------------------
// ssh (p10k:3236-3253)
// ---------------------------------------------------------------------

fn ssh_segments() -> Vec<Segment> {
    // p10k:3242-3246 — segment cond is "never" when not over SSH.
    if !is_ssh() {
        return vec![];
    }
    // p10k:3238 — `_p9k_prompt_segment "$0" "$_p9k_color1" "yellow" 'SSH_ICON' 0 '' ''`
    vec![make_segment(
        "ssh",
        None,
        color1(),
        "yellow",
        "SSH_ICON",
        String::new(),
    )]
}

// ---------------------------------------------------------------------
// root_indicator (p10k:3134-3140)
// ---------------------------------------------------------------------

fn root_indicator_segments() -> Vec<Segment> {
    // p10k:3136 cond '${${(%):-%#}:#\%}' — shown only when %# is '#'
    // (euid == 0).
    if !is_root() {
        return vec![];
    }
    vec![make_segment(
        "root_indicator",
        None,
        color1(),
        "yellow",
        "ROOT_ICON",
        String::new(),
    )]
}

// ---------------------------------------------------------------------
// history (p10k:2211-2215)
// ---------------------------------------------------------------------

fn history_segments() -> Vec<Segment> {
    // p10k:2213 — `_p9k_prompt_segment "$0" "grey50" "$_p9k_color1" '' 0 '' '%h'`
    // %h stays a prompt escape; the prompt expander renders the live
    // history event number.
    vec![make_segment(
        "history",
        None,
        "grey50",
        color1(),
        "",
        "%h".to_string(),
    )]
}

// ---------------------------------------------------------------------
// status (p10k:3258-3301)
// ---------------------------------------------------------------------

/// p10k:8461-8470 — `_p9k_exitcode2str`: codes ≤128 render as numbers;
/// 128+N renders as the signal name, verbose form `SIG<NAME>(<N>)`
/// under STATUS_VERBOSE_SIGNAME (default on).
fn exit2str(code: i32) -> String {
    if code > 128 && !global_bool("STATUS_HIDE_SIGNAME", false) {
        let num = code - 128;
        if let Some(name) = crate::ported::signals_h::sigs_name(num) {
            if global_bool("STATUS_VERBOSE_SIGNAME", true) {
                return format!("SIG{name}({num})"); // p10k:8467
            }
            return name.to_string();
        }
    }
    code.to_string()
}

fn status_segments() -> Vec<Segment> {
    // _p9k__status is captured by p10k's precmd hook BEFORE other
    // precmd functions run (_p9k_save_status); the native mirror is
    // the preprompt snapshot in mod.rs — reading live LASTVAL here
    // showed precmd's own last command instead of the user's.
    let status = crate::p10k::last_status();
    let pipestatus: Vec<i32> = pipestatgetfn()
        .iter()
        .filter_map(|s| s.parse::<i32>().ok())
        .collect();

    // p10k:3260 — base state.
    let mut state = if status != 0 { "ERROR" } else { "OK" }.to_string();
    // p10k:3261-3271 — extended states.
    if global_bool("STATUS_EXTENDED_STATES", false) {
        if status != 0 {
            if pipestatus.len() > 1 {
                state.push_str("_PIPE"); // p10k:3264
            } else if status > 128 {
                state.push_str("_SIGNAL"); // p10k:3266
            }
        } else if pipestatus.iter().any(|&c| c != 0) {
            state.push_str("_PIPE"); // p10k:3269
        }
    }

    // p10k:3273 — `(( _POWERLEVEL9K_STATUS_$state ))` display gate.
    // Declared defaults (p10k:7465-7476): OK/OK_PIPE/ERROR/ERROR_PIPE/
    // ERROR_SIGNAL all 1.
    if !global_bool(&format!("STATUS_{state}"), true) {
        return vec![];
    }

    // p10k:3274-3278 — pipestatus text vs plain status text.
    let mut text = if global_bool("STATUS_SHOW_PIPESTATUS", true) && !pipestatus.is_empty() {
        pipestatus
            .iter()
            .map(|&c| exit2str(c))
            .collect::<Vec<_>>()
            .join("|")
    } else {
        exit2str(status as i32)
    };

    if status != 0 {
        // p10k:3280-3284.
        if !global_bool("STATUS_CROSS", false) && global_bool("STATUS_VERBOSE", true) {
            // p10k:3281 — `($0_$state red yellow1 CARRIAGE_RETURN_ICON 0 '' "$text")`
            vec![make_segment(
                "status",
                Some(&state),
                "red",
                "yellow1",
                "CARRIAGE_RETURN_ICON",
                text,
            )]
        } else {
            // p10k:3283 — `($0_$state $_p9k_color1 red FAIL_ICON 0 '' '')`
            vec![make_segment(
                "status",
                Some(&state),
                color1(),
                "red",
                "FAIL_ICON",
                String::new(),
            )]
        }
    } else if global_bool("STATUS_VERBOSE", true) || global_bool("STATUS_OK_IN_NON_VERBOSE", false)
    {
        // p10k:3285-3287 — plain OK shows the icon only.
        if state == "OK" {
            text.clear(); // p10k:3286
        }
        vec![make_segment(
            "status",
            Some(&state),
            color1(),
            "green",
            "OK_ICON",
            text,
        )]
    } else {
        vec![]
    }
}

// ---------------------------------------------------------------------
// prompt_char (p10k:3303-3356)
// ---------------------------------------------------------------------

fn prompt_char_segments() -> Vec<Segment> {
    // Snapshot from mod.rs (pre-precmd `$?`), same rationale as
    // status_segments.
    let status = crate::p10k::last_status();
    // p10k:3331/3341 — OK vs ERROR half of the state.
    let ok = status == 0;
    // `_p9k__keymap` (p10k's zle-keymap-select mirror of $KEYMAP)
    // picks the state half. Spec patterns (non-sh-glob branch):
    //   p10k:3336 VIINS — `${_p9k__keymap:#(vicmd|vivis|vivli)}` ❯
    //   p10k:3338 VICMD — `:#vicmd0` (vicmd + region INactive) ❮
    //   p10k:3339 VIVIS — `:#(vicmd1|vivis?|vivli?)` (visual) Ⅴ
    //   p10k:3333 VIOWR — `$_p9k__zle_state` lacks `insert` ▶
    //     (only under PROMPT_CHAR_OVERWRITE_STATE; VIINS then also
    //     requires the state to lack `overwrite`).
    // The native engine reads the live ZLE keymap, region and insert
    // mode directly (the engine re-renders from zle_keymap.rs on a
    // keymap switch).
    let keymap_name = crate::ported::zle::zle_keymap::curkeymapname().clone();
    // p10k:7886 `_p9k_check_visual_mode` — `${${REGION_ACTIVE:-0}/2/1}`.
    let region_active = crate::ported::zle::zle_main::REGION_ACTIVE.load(Ordering::Relaxed) != 0;
    let overwrite = crate::ported::zle::zle_main::INSMODE.load(Ordering::Relaxed) == 0;
    let keymap = match keymap_name.as_str() {
        "vicmd" if region_active => "VIVIS", // p10k:3339 vicmd1
        "vicmd" => "VICMD",                  // p10k:3338 vicmd0
        "vivis" | "vivli" => "VIVIS",        // p10k:3339
        _ if overwrite && global_bool("PROMPT_CHAR_OVERWRITE_STATE", false) => "VIOWR", // p10k:3333
        _ => "VIINS",                        // p10k:3336
    };
    let state = format!("{}_{}", if ok { "OK" } else { "ERROR" }, keymap);
    // p10k:3348 — `$0_OK_VIINS "$_p9k_color1" 76 '' … '❯'`
    // p10k:3336 — `$0_ERROR_VIINS "$_p9k_color1" 196 '' … '❯'`
    let default_fg = if ok { "76" } else { "196" };
    // Glyph per state: ❯ VIINS (p10k:3336), ❮ VICMD (p10k:3338),
    // Ⅴ VIVIS (p10k:3339), ▶ VIOWR (p10k:3333).
    let glyph = match keymap {
        "VICMD" => "\u{276E}", // ❮
        "VIVIS" => "\u{2164}", // Ⅴ
        "VIOWR" => "\u{25B6}", // ▶
        _ => "\u{276F}",       // ❯
    };
    vec![make_segment(
        "prompt_char",
        Some(&state),
        color1(),
        default_fg,
        "",
        glyph.to_string(),
    )]
}

// ---------------------------------------------------------------------
// ip (p10k:2293-2297 + net iface scan p10k:5654-5702)
// ---------------------------------------------------------------------

/// All (interface, IPv4) pairs for interfaces that are UP — the
/// getifaddrs equivalent of p10k's `ifconfig` parse (p10k:5674-5682:
/// flags word odd = IFF_UP, first `inet` line per iface). Fork-free.
fn up_ipv4_interfaces() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        tracing::warn!(target: "p10k", "getifaddrs failed for ip segment");
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

fn ip_segments() -> Vec<Segment> {
    // p10k:7398 — `_p9k_declare -s POWERLEVEL9K_IP_INTERFACE ""`.
    let pattern = p9k_global("IP_INTERFACE", "");
    if pattern.is_empty() {
        return vec![];
    }
    // p10k:5655 — `iface_regex="^($1)\$"` matched with zsh `=~` (ERE).
    let re = match regex::Regex::new(&format!("^({pattern})$")) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(target: "p10k", %pattern, %e, "bad POWERLEVEL9K_IP_INTERFACE regex");
            return vec![];
        }
    };
    // p10k:5658-5663 + 5702 — first matching interface's first IP.
    let ip = up_ipv4_interfaces()
        .into_iter()
        .find(|(name, _)| re.is_match(name))
        .map(|(_, ip)| ip);
    let Some(ip) = ip else {
        // p10k:2295 cond '$P9K_IP_IP' — hidden when empty.
        return vec![];
    };
    // Which interface produced the ip (needed for byte counters).
    let iface = up_ipv4_interfaces()
        .into_iter()
        .find(|(name, _)| re.is_match(name))
        .map(|(name, _)| name)
        .unwrap_or_default();
    // p10k:5702-5747 — publish P9K_IP_* so a user IP_CONTENT_EXPANSION
    // (`${P9K_IP_RX_RATE:+…}$P9K_IP_IP`) renders rates + address instead
    // of collapsing to just the NETWORK_ICON box.
    publish_ip_status(&iface, &ip);
    // p10k:2295 — `_p9k_prompt_segment "$0" "cyan" "$_p9k_color1" 'NETWORK_ICON' …`
    vec![make_segment(
        "ip",
        None,
        "cyan",
        color1(),
        "NETWORK_ICON",
        ip,
    )]
}

/// Per-interface byte-counter sample from the previous prompt, for the
/// rate delta (p10k stashes P9K_IP_{RX,TX}_BYTES + _p9__ip_timestamp).
static IP_LAST_SAMPLE: std::sync::Mutex<Option<(String, u64, u64, f64)>> =
    std::sync::Mutex::new(None);

/// p10k:5702-5747 — set the P9K_IP_* parameters a user IP_CONTENT_EXPANSION
/// reads: the address, interface, cumulative bytes, and the RX/TX rates
/// (bytes-delta since the last prompt over elapsed seconds).
fn publish_ip_status(iface: &str, ip: &str) {
    use crate::ported::params::setsparam;
    let _ = setsparam("P9K_IP_IP", ip); // p10k:5702
    let _ = setsparam("P9K_IP_INTERFACE", iface);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let (rx, tx) = iface_bytes(iface).unwrap_or((0, 0));

    // p10k:5728-5743 — rate only when the same iface+ip persisted across
    // prompts; else "0 B/s".
    let (rx_rate, tx_rate) = {
        let mut last = IP_LAST_SAMPLE.lock().unwrap();
        let rates = match last.as_ref() {
            // p10k:5729 — `$ip_ip == $P9K_IP_IP && iface == P9K_IP_INTERFACE`.
            Some((liface, lrx, ltx, lts)) if liface == iface => {
                let t = now - lts; // p10k:5730
                if t <= 0.0 {
                    ("0 B/s".to_string(), "0 B/s".to_string()) // p10k:5731-5733
                } else {
                    (
                        rate_str(rx.saturating_sub(*lrx) as f64 / t), // p10k:5737-5738
                        rate_str(tx.saturating_sub(*ltx) as f64 / t), // p10k:5735-5736
                    )
                }
            }
            _ => ("0 B/s".to_string(), "0 B/s".to_string()), // p10k:5741-5742
        };
        *last = Some((iface.to_string(), rx, tx, now));
        rates
    };
    let _ = setsparam("P9K_IP_RX_RATE", &rx_rate); // p10k:5646
    let _ = setsparam("P9K_IP_TX_RATE", &tx_rate); // p10k:5645
    let _ = setsparam("P9K_IP_RX_BYTES", &rx.to_string());
    let _ = setsparam("P9K_IP_TX_BYTES", &tx.to_string());
}

/// p10k:5736/5738 — format a byte/s rate: `_p9k_human_readable_bytes`
/// then `N B/s` (unscaled) or `N XiB/s` (scaled, X ∈ K M G …).
fn rate_str(bytes_per_sec: f64) -> String {
    let (val, suffix) = human_readable_bytes(bytes_per_sec);
    if suffix == 'B' {
        format!("{val} B/s")
    } else {
        format!("{val} {suffix}iB/s")
    }
}

/// Port of `_p9k_human_readable_bytes` (p10k:327-342): scale by 1024
/// through the B K M G … suffixes, format with 2/1/0 decimals by
/// magnitude, strip trailing zeros. Returns (value-string, suffix-char).
fn human_readable_bytes(n: f64) -> (String, char) {
    const SUF: [char; 9] = ['B', 'K', 'M', 'G', 'T', 'P', 'E', 'Z', 'Y']; // p10k:322
    let mut n = n;
    let mut suf = 'B';
    for (i, s) in SUF.iter().enumerate() {
        suf = *s;
        if n < 1024.0 || i == SUF.len() - 1 {
            break;
        }
        n /= 1024.0;
    }
    // p10k:334-340 — precision by magnitude.
    let raw = if n >= 100.0 {
        format!("{n:.0}.")
    } else if n >= 10.0 {
        format!("{n:.1}")
    } else {
        format!("{n:.2}")
    };
    // p10k:341 — `${${ret%%0#}%.}`: strip trailing zeros, then a trailing dot.
    let trimmed = raw.trim_end_matches('0').trim_end_matches('.').to_string();
    let trimmed = if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed
    };
    (trimmed, suf)
}

/// Interface (rx_bytes, tx_bytes) — Linux `/sys/class/net/<if>/statistics`,
/// macOS/BSD via `getifaddrs` AF_LINK `if_data` (fork-free, unlike p10k's
/// `netstat -inbI`). None when unavailable.
#[cfg(target_os = "linux")]
fn iface_bytes(iface: &str) -> Option<(u64, u64)> {
    let rd = |kind: &str| -> Option<u64> {
        std::fs::read_to_string(format!("/sys/class/net/{iface}/statistics/{kind}_bytes"))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some((rd("rx")?, rd("tx")?))
}

#[cfg(not(target_os = "linux"))]
fn iface_bytes(iface: &str) -> Option<(u64, u64)> {
    // getifaddrs → the AF_LINK entry for `iface` carries `if_data` with
    // ifi_ibytes / ifi_obytes (the same counters `netstat -inb` prints).
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return None;
    }
    let mut out = None;
    let mut cur = ifap;
    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() {
            continue;
        }
        let sa = unsafe { &*ifa.ifa_addr };
        if i32::from(sa.sa_family) != libc::AF_LINK {
            continue;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(ifa.ifa_name) }.to_string_lossy();
        if name != iface || ifa.ifa_data.is_null() {
            continue;
        }
        // ifa_data → struct if_data; read ifi_ibytes/ifi_obytes.
        let d = unsafe { &*(ifa.ifa_data as *const libc::if_data) };
        out = Some((d.ifi_ibytes as u64, d.ifi_obytes as u64));
        break;
    }
    unsafe { libc::freeifaddrs(ifap) };
    out
}

// ---------------------------------------------------------------------
// dir_writable (p10k:4437-4443)
// ---------------------------------------------------------------------

fn dir_writable_segments() -> Vec<Segment> {
    // p10k:4438 — `[[ ! -w "$_p9k__cwd_a" ]]`.
    if path_writable(&cwd()) {
        return vec![];
    }
    // p10k:4439 — `_p9k_prompt_segment "$0_FORBIDDEN" "red" "yellow1" 'LOCK_ICON' 0 '' ''`
    vec![make_segment(
        "dir_writable",
        Some("FORBIDDEN"),
        "red",
        "yellow1",
        "LOCK_ICON",
        String::new(),
    )]
}

// ---------------------------------------------------------------------
// background_jobs (p10k:1234-1246)
// ---------------------------------------------------------------------

fn background_jobs_segments() -> Vec<Segment> {
    let count = job_count();
    // p10k:1244 cond '${${(%):-%j}:#0}' — hidden at zero jobs.
    if count == 0 {
        return vec![];
    }
    // p10k:1237-1243 — verbose count; plain verbose hides the count
    // when it is exactly "1" (`${${(%):-%j}:#1}`).
    let msg = if global_bool("BACKGROUND_JOBS_VERBOSE", true) {
        if global_bool("BACKGROUND_JOBS_VERBOSE_ALWAYS", false) || count != 1 {
            count.to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    // p10k:1244 — `$0 "$_p9k_color1" cyan BACKGROUND_JOBS_ICON …`
    vec![make_segment(
        "background_jobs",
        None,
        color1(),
        "cyan",
        "BACKGROUND_JOBS_ICON",
        msg,
    )]
}

// ---------------------------------------------------------------------
// custom_* (p10k:1691-1701) — the ONLY segment that runs shell code
// ---------------------------------------------------------------------

fn custom_segments(name: &str) -> Vec<Segment> {
    // p10k:1692-1694 — command string from POWERLEVEL9K_CUSTOM_<NAME:u>.
    let upper = name.to_ascii_uppercase();
    let cmd = match getsparam(&format!("POWERLEVEL9K_CUSTOM_{upper}")) {
        Some(c) if !c.is_empty() => c,
        _ => return vec![],
    };
    // p10k:1703 — `(( $+functions[$cmd] || $+commands[$cmd] )) || return`,
    // where `$cmd` is the first word of the command, dequoted
    // (p10k:1701-1702 `parts=("${(@z)command}")`, `cmd="${(Q)parts[1]}"`).
    //
    // This gate USED to be omitted here under the claim that it "is
    // subsumed: a missing command yields empty output". That is true of the
    // RESULT and false of the COST, which is the whole point of the gate
    // upstream: zsh returns before `eval` ever runs, while this port paid a
    // full command substitution first. In zshrs a `$()` is not a cheap
    // fork — `run_command_substitution` deep-clones the entire mutable
    // shell state for subshell isolation (paramtab, the hashed-param
    // storage incl. `_comps`, shfunctab, …), measured at ~13ms per call on
    // a populated shell. So an absent tool cost ~13ms per prompt to
    // produce nothing at all.
    let first_word = cmd.split_whitespace().next().unwrap_or("");
    let first_word = first_word.trim_matches(|c| c == '\'' || c == '"');
    if !first_word.is_empty()
        && crate::ported::hashtable::shfunctab_lock()
            .read()
            .map(|t| t.get(first_word).is_none())
            .unwrap_or(true)
        && crate::extensions::p10k::segments_sys::cmd_on_path(first_word).is_none()
        && !crate::ported::builtin::createbuiltintable().contains_key(first_word)
    {
        return vec![];
    }
    let content = crate::ported::exec::run_command_substitution(&cmd);
    let content = content.trim().to_string();
    // p10k:1699 — `[[ -n $content ]] || return`.
    if content.is_empty() {
        return vec![];
    }
    // p10k:1700 — `_p9k_prompt_segment "prompt_custom_$1" $_p9k_color2
    // $_p9k_color1 "CUSTOM_${segment_name}_ICON" 0 '' "$content"`
    let seg_name = format!("custom_{name}");
    let icon_key = format!("CUSTOM_{upper}_ICON");
    vec![make_segment(
        &seg_name,
        None,
        color2(),
        color1(),
        &icon_key,
        content,
    )]
}

// ---------------------------------------------------------------------
// vcs (p10k:4131-4165 + _p9k_vcs_render p10k:3839-4012)
// ---------------------------------------------------------------------

/// p10k:3554-3560 — `__p9k_vcs_states` default backgrounds.
pub(super) fn vcs_state_default_bg(state: &str) -> &'static str {
    match state {
        "CLEAN" | "UNTRACKED" => "2",
        "MODIFIED" | "CONFLICTED" => "3",
        "LOADING" => "8",
        _ => "2",
    }
}

/// Per-part color override — the two `$+parameters` probes of
/// `_p9k_vcs_style` (p10k:568-575):
/// `POWERLEVEL9K_VCS_<STATE>_<PART>FORMAT_FOREGROUND`, then
/// `POWERLEVEL9K_VCS_<PART>FORMAT_FOREGROUND`.
fn vcs_part_fg(state: &str, part: &str) -> Option<String> {
    getsparam(&format!("POWERLEVEL9K_VCS_{state}_{part}FORMAT_FOREGROUND"))
        .or_else(|| getsparam(&format!("POWERLEVEL9K_VCS_{part}FORMAT_FOREGROUND")))
}

/// State selection per gitstatus counts — p10k:3910-3924 (identical to
/// the DISABLE_GITSTATUS_FORMATTING arm p10k:3856-3864).
fn vcs_state_for(gs: &git::GitStatus) -> &'static str {
    if gs.conflicted > 0 && global_bool("VCS_CONFLICTED_STATE", false) {
        "CONFLICTED" // p10k:3911
    } else if gs.staged != 0 || gs.unstaged != 0 {
        "MODIFIED" // p10k:3913
    } else if gs.untracked != 0 {
        "UNTRACKED" // p10k:3915
    } else {
        "CLEAN" // p10k:3924
    }
}

/// Branch-name shortening — p10k:3944-3955. Applies only when BOTH
/// VCS_SHORTEN_LENGTH and VCS_SHORTEN_MIN_LENGTH are set.
fn shorten_branch(branch: &str) -> String {
    let sl = getsparam("POWERLEVEL9K_VCS_SHORTEN_LENGTH").and_then(|v| v.parse::<usize>().ok());
    let minl =
        getsparam("POWERLEVEL9K_VCS_SHORTEN_MIN_LENGTH").and_then(|v| v.parse::<usize>().ok());
    let strategy = p9k_global("VCS_SHORTEN_STRATEGY", "");
    if let (Some(sl), Some(minl)) = (sl, minl) {
        let n = branch.chars().count();
        if n > minl
            && n > sl
            && (strategy == "truncate_middle" || strategy == "truncate_from_right")
        {
            // p10k:7246-7248 — delimiter default '…'.
            let delim = decode_g(&p9k_global("VCS_SHORTEN_DELIMITER", "\u{2026}"));
            let head: String = branch.chars().take(sl).collect();
            let mut out = format!("{}{delim}", esc_pct(&head)); // p10k:3948
            if strategy == "truncate_middle" {
                let tail: String = branch.chars().skip(n.saturating_sub(sl)).collect();
                out.push_str(&esc_pct(&tail)); // p10k:3951
            }
            return out;
        }
    }
    esc_pct(branch) // p10k:3954
}

/// Publish the `VCS_STATUS_*` parameters the gitstatus backend exposes
/// (Src gitstatus/gitstatus.plugin.zsh), read by user VCS_CONTENT_EXPANSION
/// formatters such as `my_git_formatter`. `P9K_CONTENT` is left EMPTY by
/// the caller on the formatting-disabled path so the formatter builds
/// from these rather than echoing a pre-formatted string.
fn publish_vcs_status(gs: &git::GitStatus) {
    let has = |n: i64| if n > 0 { 1 } else { 0 };
    // gitstatus.plugin.zsh:352-364 — an index over VCS_MAX_INDEX_SIZE_DIRTY
    // leaves the dirty facts unknown (-1).
    let has_dirty = |n: i64| if gs.dirty_unknown { -1 } else { has(n) };
    let (encoding, summary) = git::commit_message(Path::new(&gs.workdir), &gs.commit);
    for (k, v) in [
        ("VCS_STATUS_LOCAL_BRANCH", gs.branch.clone()),
        ("VCS_STATUS_REMOTE_BRANCH", gs.remote_branch.clone()),
        ("VCS_STATUS_TAG", gs.tag.clone()),
        ("VCS_STATUS_COMMIT", gs.commit.clone()),
        ("VCS_STATUS_REMOTE_URL", gs.remote_url.clone()),
        ("VCS_STATUS_WORKDIR", gs.workdir.clone()),
        ("VCS_STATUS_ACTION", gs.action.clone()),
        ("VCS_STATUS_COMMITS_AHEAD", gs.ahead.to_string()),
        ("VCS_STATUS_COMMITS_BEHIND", gs.behind.to_string()),
        ("VCS_STATUS_REMOTE_NAME", gs.remote_name.clone()),
        ("VCS_STATUS_PUSH_REMOTE_NAME", gs.push_remote_name.clone()),
        ("VCS_STATUS_PUSH_REMOTE_URL", gs.push_remote_url.clone()),
        ("VCS_STATUS_PUSH_COMMITS_AHEAD", gs.push_ahead.to_string()),
        ("VCS_STATUS_PUSH_COMMITS_BEHIND", gs.push_behind.to_string()),
        ("VCS_STATUS_INDEX_SIZE", gs.index_size.to_string()),
        ("VCS_STATUS_NUM_STAGED_NEW", gs.staged_new.to_string()),
        ("VCS_STATUS_NUM_STAGED_DELETED", gs.staged_deleted.to_string()),
        ("VCS_STATUS_NUM_UNSTAGED_DELETED", gs.unstaged_deleted.to_string()),
        ("VCS_STATUS_NUM_SKIP_WORKTREE", gs.skip_worktree.to_string()),
        ("VCS_STATUS_NUM_ASSUME_UNCHANGED", gs.assume_unchanged.to_string()),
        ("VCS_STATUS_COMMIT_ENCODING", encoding),
        ("VCS_STATUS_COMMIT_SUMMARY", summary),
        ("VCS_STATUS_STASHES", gs.stashes.to_string()),
        ("VCS_STATUS_NUM_STAGED", gs.staged.to_string()),
        ("VCS_STATUS_NUM_UNSTAGED", gs.unstaged.to_string()),
        ("VCS_STATUS_NUM_UNTRACKED", gs.untracked.to_string()),
        ("VCS_STATUS_NUM_CONFLICTED", gs.conflicted.to_string()),
        ("VCS_STATUS_HAS_STAGED", has(gs.staged).to_string()),
        ("VCS_STATUS_HAS_UNSTAGED", has_dirty(gs.unstaged).to_string()),
        ("VCS_STATUS_HAS_UNTRACKED", has_dirty(gs.untracked).to_string()),
        ("VCS_STATUS_HAS_CONFLICTED", has_dirty(gs.conflicted).to_string()),
    ] {
        let _ = crate::ported::params::setsparam(k, &v);
    }
}

/// Default `POWERLEVEL9K_VCS_GIT_REMOTE_ICONS` domain table (p10k:7481-7497).
const VCS_REMOTE_DOMAINS: [(&str, &str); 15] = [
    ("archlinux.org", "VCS_GIT_ARCHLINUX_ICON"),
    ("dev.azure.com|visualstudio.com", "VCS_GIT_AZURE_ICON"),
    ("bitbucket.org", "VCS_GIT_BITBUCKET_ICON"),
    ("codeberg.org", "VCS_GIT_CODEBERG_ICON"),
    ("debian.org", "VCS_GIT_DEBIAN_ICON"),
    ("freebsd.org", "VCS_GIT_FREEBSD_ICON"),
    ("freedesktop.org", "VCS_GIT_FREEDESKTOP_ICON"),
    ("gitea.com|gitea.io", "VCS_GIT_GITEA_ICON"),
    ("github.com", "VCS_GIT_GITHUB_ICON"),
    ("gitlab.com", "VCS_GIT_GITLAB_ICON"),
    ("gnome.org", "VCS_GIT_GNOME_ICON"),
    ("gnu.org", "VCS_GIT_GNU_ICON"),
    ("kde.org", "VCS_GIT_KDE_ICON"),
    ("kernel.org", "VCS_GIT_LINUX_ICON"),
    ("sr.ht", "VCS_GIT_SOURCEHUT_ICON"),
];

/// Native evaluation of the default per-domain pattern (p10k:7499)
/// `(|[A-Za-z0-9][A-Za-z0-9+.-]#://)(|[^:/?#]#[.@])((#i)DOMAIN)(|[/:?#]*)`
/// against `url`: optional scheme, optional host prefix ending in `.` or
/// `@` (no `:/?#` inside), case-insensitive domain alternative, then end
/// of string or one of `/:?#`.
fn remote_url_matches_domain(url: &str, domains: &str) -> bool {
    let url = url.to_lowercase();
    let mut bases: Vec<&str> = vec![url.as_str()];
    if let Some((scheme, rest)) = url.split_once("://") {
        let mut sc = scheme.chars();
        let ok = sc.next().is_some_and(|c| c.is_ascii_alphanumeric())
            && sc.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'));
        if ok {
            bases.push(rest);
        }
    }
    bases.iter().any(|base| {
        let mut starts = vec![0usize];
        for (i, c) in base.char_indices() {
            if matches!(c, ':' | '/' | '?' | '#') {
                break;
            }
            if c == '.' || c == '@' {
                starts.push(i + 1);
            }
        }
        starts.into_iter().any(|st| {
            domains.split('|').any(|d| {
                base[st..]
                    .strip_prefix(d)
                    .is_some_and(|tail| tail.is_empty() || tail.starts_with(['/', ':', '?', '#']))
            })
        })
    })
}

/// Port of `_p9k_vcs_icon` (p10k:3867-3876): walk
/// `POWERLEVEL9K_VCS_GIT_REMOTE_ICONS` (pattern/icon-key pairs; an odd
/// trailing pattern gets an empty icon, p10k:7479) and return the icon
/// key of the first match, "" when none. Unset array = the default
/// domain table followed by `* VCS_GIT_ICON` (p10k:7498-7500).
fn vcs_remote_icon_key(url: &str) -> String {
    let user = p9k_global_arr("VCS_GIT_REMOTE_ICONS");
    if user.is_empty() {
        for (domains, key) in VCS_REMOTE_DOMAINS {
            if remote_url_matches_domain(url, domains) {
                return key.to_string();
            }
        }
        return "VCS_GIT_ICON".to_string();
    }
    for pair in user.chunks(2) {
        let mut pat = pair[0].clone();
        crate::ported::glob::tokenize(&mut pat);
        if let Some(prog) = crate::ported::pattern::patcompile(&pat, 0, None) {
            if crate::ported::pattern::pattry(&prog, url) {
                return pair.get(1).cloned().unwrap_or_default();
            }
        }
    }
    String::new()
}

fn vcs_segments() -> Vec<Segment> {
    // p10k:4176 — configured backends. git renders through the gitstatus
    // port below; every other backend goes to the native vcs_info
    // equivalent (vcs_other.rs, p10k:4181-4207).
    let mut backends = p9k_global_arr("VCS_BACKENDS");
    if backends.is_empty() {
        backends.push("git".to_string()); // p10k:7480 default (git)
    }
    let others: Vec<String> = backends.iter().filter(|b| *b != "git").cloned().collect();
    if !backends.iter().any(|b| b == "git") {
        return super::vcs_other::vcs_info_segments(&others);
    }

    let cwd = cwd();

    let mut gs = match git::git_status_for(Path::new(&cwd)) {
        Some(g) => g,
        // p10k:4180-4181 — no git repo: `backends=(${backends:#git})`
        // falls through to vcs_info for the remaining backends.
        None => return super::vcs_other::vcs_info_segments(&others),
    };

    // p10k:4053-4057 _p9k_maybe_ignore_git_repo — repos whose
    // $VCS_STATUS_WORKDIR matches VCS_DISABLED_WORKDIR_PATTERN are
    // treated as no-repo.
    let disabled = getsparam("POWERLEVEL9K_VCS_DISABLED_WORKDIR_PATTERN").unwrap_or_default();
    if !disabled.is_empty() && glob_match(&disabled, &gs.workdir, &home_dir()) {
        return super::vcs_other::vcs_info_segments(&others);
    }

    // p10k:3871-3875 — VCS_GIT_HOOKS gate individual data sources.
    let hooks = {
        let h = p9k_global_arr("VCS_GIT_HOOKS");
        if h.is_empty() {
            // p10k:7481 declared default list.
            vec![
                "vcs-detect-changes".to_string(),
                "git-untracked".to_string(),
                "git-aheadbehind".to_string(),
                "git-stash".to_string(),
                "git-remotebranch".to_string(),
                "git-tagname".to_string(),
            ]
        } else {
            h
        }
    };
    let hook = |n: &str| hooks.iter().any(|h| h == n);
    if !hook("git-untracked") {
        gs.untracked = 0; // p10k:3871
    }
    if !hook("git-aheadbehind") {
        gs.ahead = 0; // p10k:3872
        gs.behind = 0;
    }
    if !hook("git-stash") {
        gs.stashes = 0; // p10k:3873
    }
    if !hook("git-remotebranch") {
        gs.remote_branch.clear(); // p10k:3874
    }
    if !hook("git-tagname") {
        gs.tag.clear(); // p10k:3875
    }

    // p10k:3877-3881 — MAX_NUM clamps for ahead/behind.
    let ahead_max = global_int("VCS_COMMITS_AHEAD_MAX_NUM", -1); // p10k:7496
    let behind_max = global_int("VCS_COMMITS_BEHIND_MAX_NUM", -1); // p10k:7497
    if ahead_max >= 0 && gs.ahead > ahead_max {
        gs.ahead = ahead_max;
    }
    if behind_max >= 0 && gs.behind > behind_max {
        gs.behind = behind_max;
    }

    let state = vcs_state_for(&gs);

    // p10k:3854-3868 — the gitstatus backend publishes VCS_STATUS_*
    // globals and, when VCS_DISABLE_GITSTATUS_FORMATTING is on, leaves
    // P9K_CONTENT EMPTY so the user's VCS_CONTENT_EXPANSION (e.g.
    // `my_git_formatter`) builds the content from those globals. Always
    // publish the globals (a CONTENT_EXPANSION may read them even in the
    // default path); the empty-content segment is only for the
    // formatting-disabled case.
    publish_vcs_status(&gs);
    // p10k:3867-3876 _p9k_vcs_icon — first matching remote-URL pattern
    // picks the icon key; computed only under the vcs-detect-changes
    // hook (p10k:3957-3960; the formatting-disabled arm p10k:3905 too).
    let icon = if hook("vcs-detect-changes") {
        let icon_key = vcs_remote_icon_key(&gs.remote_url);
        apply_visual_identifier("vcs", Some(state), seg_icon("vcs", Some(state), &icon_key))
    } else {
        None
    };
    if global_bool("VCS_DISABLE_GITSTATUS_FORMATTING", false) {
        // Empty content → P9K_CONTENT="" reaches the formatter, which
        // then formats from VCS_STATUS_*. Icon (VCS_GIT_ICON) and
        // bg/fg still apply; the formatter embeds its own branch glyph.
        let bg = p9k_param(
            "vcs",
            Some(state),
            "BACKGROUND",
            vcs_state_default_bg(state),
        );
        let fg = p9k_param("vcs", Some(state), "FOREGROUND", color1());
        return vec![Segment {
            name: "vcs".to_string(),
            state: Some(state.to_string()),
            content: String::new(),
            icon,
            fg,
            bg,
        }];
    }

    // ------- content assembly, p10k:3926-4005 -------
    // Each entry is (PART, text); PART keys the per-part color styles.
    let mut parts: Vec<(&'static str, String)> = Vec::new();
    let mut ws = ""; // p10k:3931,3935

    // p10k:3932-3936 — commit hash shown when SHOW_CHANGESET or no branch.
    if global_bool("SHOW_CHANGESET", false) || gs.branch.is_empty() {
        let hash_len = global_int("CHANGESET_HASH_LENGTH", 8).max(0) as usize; // p10k:7253
        let commit: String = gs.commit.chars().take(hash_len).collect();
        let commit = if commit.is_empty() {
            "HEAD".to_string() // p10k:3934 `:-HEAD`
        } else {
            commit
        };
        let icon = seg_icon("vcs", Some(state), "VCS_COMMIT_ICON"); // p10k:3933
        parts.push(("COMMIT", format!("{icon}{commit}")));
        ws = " ";
    }

    // p10k:3938-3957 — branch (icon unless HIDE_BRANCH_ICON).
    if !gs.branch.is_empty() {
        let mut b = ws.to_string();
        if !global_bool("HIDE_BRANCH_ICON", false) {
            b.push_str(&seg_icon("vcs", Some(state), "VCS_BRANCH_ICON")); // p10k:3941
        }
        b.push_str(&shorten_branch(&gs.branch));
        parts.push(("BRANCH", b)); // p10k:3956
    }

    // p10k:3959-3962 — tag.
    if !global_bool("VCS_HIDE_TAGS", false) && !gs.tag.is_empty() {
        let icon = seg_icon("vcs", Some(state), "VCS_TAG_ICON"); // p10k:3960
        parts.push(("TAG", format!(" {icon}{}", esc_pct(&gs.tag))));
    }

    if !gs.action.is_empty() {
        // p10k:3964-3965 — in-progress action (rebase/merge/…).
        parts.push(("ACTION", format!(" | {}", esc_pct(&gs.action))));
    } else {
        // p10k:3967-3971 — remote branch when it differs.
        if !gs.remote_branch.is_empty() && gs.remote_branch != gs.branch {
            let icon = seg_icon("vcs", Some(state), "VCS_REMOTE_BRANCH_ICON"); // p10k:3969
            parts.push((
                "REMOTE_BRANCH",
                format!(" {icon}{}", esc_pct(&gs.remote_branch)),
            ));
        }
        // p10k:3972-3990 — dirty details.
        if gs.staged > 0 || gs.unstaged > 0 || gs.untracked > 0 {
            let dirty = seg_icon("vcs", Some(state), "VCS_DIRTY_ICON"); // p10k:3973
            parts.push(("DIRTY", dirty));
            if gs.staged > 0 {
                let mut t = seg_icon("vcs", Some(state), "VCS_STAGED_ICON"); // p10k:3976
                if global_int("VCS_STAGED_MAX_NUM", 1) != 1 {
                    t.push_str(&gs.staged.to_string()); // p10k:3977
                }
                parts.push(("STAGED", format!(" {t}")));
            }
            if gs.unstaged > 0 {
                let mut t = seg_icon("vcs", Some(state), "VCS_UNSTAGED_ICON"); // p10k:3981
                if global_int("VCS_UNSTAGED_MAX_NUM", 1) != 1 {
                    t.push_str(&gs.unstaged.to_string()); // p10k:3982
                }
                parts.push(("UNSTAGED", format!(" {t}")));
            }
            if gs.untracked > 0 {
                let mut t = seg_icon("vcs", Some(state), "VCS_UNTRACKED_ICON"); // p10k:3986
                if global_int("VCS_UNTRACKED_MAX_NUM", 1) != 1 {
                    t.push_str(&gs.untracked.to_string()); // p10k:3987
                }
                parts.push(("UNTRACKED", format!(" {t}")));
            }
        }
        // p10k:3991-3995 — commits behind (incoming).
        if gs.behind > 0 {
            let mut t = seg_icon("vcs", Some(state), "VCS_INCOMING_CHANGES_ICON");
            if behind_max != 1 {
                t.push_str(&gs.behind.to_string()); // p10k:3993
            }
            parts.push(("INCOMING_CHANGES", format!(" {t}")));
        }
        // p10k:3996-4000 — commits ahead (outgoing).
        if gs.ahead > 0 {
            let mut t = seg_icon("vcs", Some(state), "VCS_OUTGOING_CHANGES_ICON");
            if ahead_max != 1 {
                t.push_str(&gs.ahead.to_string()); // p10k:3998
            }
            parts.push(("OUTGOING_CHANGES", format!(" {t}")));
        }
        // p10k:4001-4004 — stash count.
        if gs.stashes > 0 {
            let icon = seg_icon("vcs", Some(state), "VCS_STASH_ICON");
            parts.push(("STASH", format!(" {icon}{}", gs.stashes)));
        }
    }

    // p10k:4007 — colors: bg from __p9k_vcs_states, fg color1, both
    // overridable through the param chain.
    let bg = p9k_param(
        "vcs",
        Some(state),
        "BACKGROUND",
        vcs_state_default_bg(state),
    );
    let fg = p9k_param("vcs", Some(state), "FOREGROUND", color1());

    // _p9k_vcs_style (p10k:557-583): inject per-part %F colors only
    // when any *FORMAT_FOREGROUND override exists; otherwise the plain
    // segment fg applies and no escapes are emitted.
    let any_styled = parts.iter().any(|(p, _)| vcs_part_fg(state, p).is_some());
    let content = if any_styled {
        let base_style = format!("%b{}", bgesc(&bg)); // p10k:563-566
        parts
            .iter()
            .map(|(p, text)| {
                let part_fg = vcs_part_fg(state, p).unwrap_or_else(|| fg.clone());
                format!("{base_style}{}{text}", fgesc(&part_fg)) // p10k:578-581
            })
            .collect::<String>()
    } else {
        parts.iter().map(|(_, t)| t.as_str()).collect::<String>()
    };

    vec![Segment {
        name: "vcs".to_string(),
        state: Some(state.to_string()),
        content,
        icon,
        fg,
        bg,
    }]
}

// ---------------------------------------------------------------------
// dir (p10k:1768-2171)
// ---------------------------------------------------------------------

/// HOME-only contraction (`auto_name_dirs` branch, p10k:1771-1773:
/// `${cwd/#(#b)$HOME(|\/*)/'~'$match[1]}`); also the no-live-shell
/// fallback shape of `%~`.
fn contract_home(cwd: &str, home: &str) -> String {
    if home.is_empty() || home == "/" {
        return cwd.to_string();
    }
    if cwd == home {
        return "~".to_string();
    }
    if let Some(rest) = cwd.strip_prefix(home) {
        if rest.starts_with('/') {
            return format!("~{rest}");
        }
    }
    cwd.to_string()
}

/// `_p9k_shorten_delim_len` (p10k:1764-1766): the
/// SHORTEN_DELIMITER_LENGTH override, else the delimiter's length.
fn shorten_delim_len(delim: &str) -> usize {
    let d = global_int("SHORTEN_DELIMITER_LENGTH", -1); // p10k:7388
    if d >= 0 {
        d as usize
    } else {
        delim.chars().count()
    }
}

/// Default strategy (p10k:2009-2017): keep the last `shortenlen`
/// components, collapsing everything before them into one elision mark.
fn shorten_default(parts: &mut Vec<String>, shortenlen: i64) {
    if shortenlen <= 0 {
        return;
    }
    let sl = shortenlen as usize;
    // p10k:2011-2012 — a leading empty part ("/" root) doesn't count.
    let mut len = parts.len();
    if parts.first().is_some_and(|p| p.is_empty()) {
        len -= 1;
    }
    if len > sl {
        // p10k:2014 — parts[1,-shortenlen-1]=($'\1').
        let keep_from = parts.len() - sl;
        parts.splice(0..keep_from, [MARK_ELIDE.to_string()]);
    }
}

/// truncate_to_last (p10k:1879-1887). Returns fake_first.
fn shorten_to_last(parts: &mut Vec<String>, p: &str, mut shortenlen: i64) -> bool {
    if shortenlen <= 0 {
        shortenlen = 1; // p10k:1880-1881
    }
    let sl = shortenlen as usize;
    // p10k:1883 — `$#parts -gt i || $p[1] != / && $#parts -gt shortenlen`.
    if parts.len() > sl + 1 || (!p.starts_with('/') && parts.len() > sl) {
        // p10k:1885 — parts[1,-i]=() keeps the last `shortenlen` parts.
        let keep_from = parts.len().saturating_sub(sl);
        parts.drain(0..keep_from);
        return true; // fake_first=1 (p10k:1884)
    }
    false
}

/// truncate_to_first_and_last (p10k:1888-1896).
fn shorten_first_and_last(parts: &mut [String], p: &str, shortenlen: i64) {
    if shortenlen <= 0 {
        return;
    }
    let sl = shortenlen as usize;
    // p10k:1890-1891 — 1-based start i = shortenlen+1, +1 for absolute.
    let mut i = sl + 1;
    if p.starts_with('/') {
        i += 1;
    }
    // p10k:1892-1894 — components between the kept head and tail elide.
    while i <= parts.len().saturating_sub(sl) {
        parts[i - 1] = MARK_ELIDE.to_string();
        i += 1;
    }
}

/// truncate_middle / truncate_from_right per-component squeeze
/// (p10k:1866-1877). `middle` keeps a suffix as well as a prefix.
fn shorten_middle_or_right(parts: &mut [String], shortenlen: i64, delim: &str, middle: bool) {
    if shortenlen <= 0 {
        return;
    }
    let pref = shortenlen as usize;
    let suf = if middle { pref } else { 0 }; // p10k:1869
    let d = shorten_delim_len(delim);
    // p10k:1870 — for (( i=2; i < $#parts; ++i )): skip first and last.
    let n = parts.len();
    for part in parts.iter_mut().take(n.saturating_sub(1)).skip(1) {
        let chars: Vec<char> = part.chars().collect();
        if chars.len() > pref + suf + d {
            // p10k:1873 — dir[pref+1,-suf-1]=$'\1'.
            let mut out: String = chars[..pref].iter().collect();
            out.push(MARK_ELIDE);
            out.extend(&chars[chars.len() - suf..]);
            *part = out;
        }
    }
}

/// truncate_absolute / truncate_absolute_chars (p10k:1816-1835): keep
/// the last `shortenlen` characters of the whole path.
fn shorten_absolute(parts: &mut Vec<String>, p: &str, shortenlen: i64, delim: &str) {
    let plen = p.chars().count() as i64;
    if shortenlen <= 0 || plen <= shortenlen {
        return;
    }
    let dl = shorten_delim_len(delim) as i64;
    if plen <= shortenlen + dl {
        return; // p10k:1819
    }
    let mut n = shortenlen as usize; // p10k:1820
    let mut i = parts.len(); // p10k:1821 (1-based $#parts)
    loop {
        let dir: Vec<char> = parts[i - 1].chars().collect();
        // p10k:1824 — component length + its leading slash (i>1).
        let len = dir.len() + usize::from(i > 1);
        if len <= n {
            n -= len; // p10k:1826
            i -= 1; // p10k:1827
            if i == 0 {
                break;
            }
        } else {
            // p10k:1829-1830 — keep the last n chars of this component,
            // drop everything before it.
            let tail: String = if n == 0 {
                String::new()
            } else {
                dir[dir.len() - n..].iter().collect()
            };
            parts[i - 1] = format!("{MARK_ELIDE}{tail}");
            parts.drain(0..i - 1);
            break;
        }
    }
}

/// `[[ $cwd == ${~pat} ]]` for DIR_CLASSES / disabled-workdir patterns:
/// top-level `|` alternation, leading-`~` HOME expansion, then the
/// ported zsh pattern engine.
fn glob_match(pattern: &str, s: &str, home: &str) -> bool {
    for alt in pattern.split('|') {
        let expanded = if alt == "~" {
            home.to_string()
        } else if let Some(rest) = alt.strip_prefix("~/") {
            format!("{home}/{rest}")
        } else {
            alt.to_string()
        };
        if expanded == s {
            return true;
        }
        // p10k:2031 `[[ $_p9k__cwd == ${(e)~a} ]]` — `~a` makes the class a
        // PATTERN, i.e. its `*`/`?`/`[` are glob tokens. patcompile reads
        // tokens, not raw metacharacters (c:Src/pattern.c patcompile), so the
        // text is tokenized first as every other patcompile caller does.
        // Untokenized, `~/*` and `*` compiled as literal strings: only the
        // exact `~` and `/etc` arms could ever match, and a subdirectory of
        // $HOME got no class, no HOME_SUB_ICON and the DEFAULT colours.
        let mut expanded = expanded;
        crate::ported::glob::tokenize(&mut expanded);
        if let Some(prog) = crate::ported::pattern::patcompile(&expanded, 0, None) {
            if crate::ported::pattern::pattry(&prog, s) {
                return true;
            }
        }
    }
    false
}

/// Existence probe for the `$+parameters[…]` guards prompt_dir uses
/// around ANCHOR / SHORTENED / PATH_HIGHLIGHT / PATH_SEPARATOR
/// foregrounds (p10k:2074-2075, 2088-2089, 2106-2107, 2127-2128):
/// POWERLEVEL9K_DIR_<PARAM> or POWERLEVEL9K_DIR_<STATE>_<PARAM>.
fn dir_param_exists(state: Option<&str>, param: &str) -> bool {
    if getsparam(&format!("POWERLEVEL9K_DIR_{param}")).is_some() {
        return true;
    }
    if let Some(st) = state {
        if getsparam(&format!("POWERLEVEL9K_DIR_{st}_{param}")).is_some() {
            return true;
        }
    }
    false
}

/// DIR_CLASSES triples (pattern, CLASS, icon-glyph). User-configured
/// classes carry raw glyphs (p10k:8444-8447 applies (g::) to them);
/// the built-in defaults resolve through the icon table
/// (p10k:8449-8457).
fn dir_classes() -> Vec<(String, String, String)> {
    let user = p9k_global_arr("DIR_CLASSES");
    if !user.is_empty() {
        return user
            .chunks(3)
            .filter(|c| c.len() == 3)
            .map(|c| {
                (
                    c[0].clone(),
                    // p10k:2032 — class is uppercased on use.
                    c[1].to_ascii_uppercase(),
                    decode_g(&c[2]), // p10k:8446 ${(g::)...}
                )
            })
            .collect();
    }
    // p10k:8449-8457 — default classes.
    vec![
        (
            "/etc|/etc/*".to_string(),
            "ETC".to_string(),
            seg_icon("dir", Some("ETC"), "ETC_ICON"),
        ),
        (
            "~".to_string(),
            "HOME".to_string(),
            seg_icon("dir", Some("HOME"), "HOME_ICON"),
        ),
        (
            "~/*".to_string(),
            "HOME_SUBFOLDER".to_string(),
            seg_icon("dir", Some("HOME_SUBFOLDER"), "HOME_SUB_ICON"),
        ),
        (
            "*".to_string(),
            "DEFAULT".to_string(),
            seg_icon("dir", Some("DEFAULT"), "FOLDER_ICON"),
        ),
    ]
}

/// Candidate renderings of a `truncate_to_unique` dir segment. p10k
/// shortens the shortenable components left to right ONLY while the
/// prompt does not fit (`${_p9k__d:#-*}`, p10k:1948-1952, budget set at
/// p10k:6143); `variants[k]` is the content with the first `k` of them
/// shortened and `saved[k]` the columns that step reclaims. render.rs
/// picks the variant (`render::fit_unique_dirs`).
#[derive(Debug, Clone)]
pub struct DirUnique {
    pub variants: Vec<String>,
    pub saved: Vec<i64>,
}

thread_local! {
    static DIR_UNIQUE: std::cell::RefCell<Vec<DirUnique>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Drain the plans registered by this frame's dir segments.
pub fn take_dir_unique() -> Vec<DirUnique> {
    DIR_UNIQUE.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

/// p10k:8578 — `[[ $VTE_VERSION != (<1-4602>|4801) ]]`: terminals other
/// than old VTE support OSC 8 hyperlinks.
fn term_has_href() -> bool {
    match getsparam("VTE_VERSION").and_then(|v| v.parse::<i64>().ok()) {
        Some(v) => !((1..=4602).contains(&v) || v == 4801),
        None => true,
    }
}

/// p10k:1759-1763 `_p9k_url_escape`: every byte outside
/// `[a-zA-Z0-9/:_.-!'()~]` becomes `%%XX` (the percent is doubled
/// because the result is prompt-expanded).
fn url_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"/:_.-!'()~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%%{b:02X}"));
        }
    }
    out
}

/// p10k:1779-1794 — recover the `~[name]` head of a dynamic named dir:
/// the first of `zsh_directory_name` / `$zsh_directory_name_functions`
/// whose `d $cwd` reply reproduces the head of `p` supplies the first
/// component; the remainder is split on `/` dropping empties
/// (`${(s:/:)${p#$parts[1]}}`).
fn dynamic_named_parts(p: &str, cwd: &str) -> Option<Vec<String>> {
    let mut funcs = vec!["zsh_directory_name".to_string()];
    funcs.extend(crate::ported::params::getaparam("zsh_directory_name_functions").unwrap_or_default());
    for func in funcs {
        if crate::ported::utils::getshfunc(&func).is_none() {
            continue;
        }
        let Some(reply) = crate::ported::utils::subst_string_by_func(&func, Some("d"), cwd) else {
            continue;
        };
        let Some(name) = reply.first() else { continue };
        let head = format!("~[{name}]");
        if let Some(rest) = p.strip_prefix(&head) {
            let mut parts = vec![head.clone()];
            parts.extend(rest.split('/').filter(|c| !c.is_empty()).map(String::from));
            return Some(parts);
        }
    }
    None
}

/// `-n $dir/${~pat}(#qN)`: does any entry of `dir` match `pat`?
fn dir_has_entry_matching(dir: &str, pat: &str) -> bool {
    let mut t = pat.to_string();
    crate::ported::glob::tokenize(&mut t);
    let Some(prog) = crate::ported::pattern::patcompile(&t, 0, None) else {
        return false;
    };
    std::fs::read_dir(dir).is_ok_and(|rd| {
        rd.flatten()
            .any(|e| crate::ported::pattern::pattry(&prog, &e.file_name().to_string_lossy()))
    })
}

/// `${dir:h}` — parent directory ("/" stays "/", relative bottoms out at ".").
fn dir_head(dir: &str) -> String {
    match dir.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => dir[..i].to_string(),
        None => ".".to_string(),
    }
}

/// p10k:7585-7597 default `POWERLEVEL9K_SHORTEN_FOLDER_MARKER`.
const DEFAULT_FOLDER_MARKER: &str = "(.bzr|.citc|.git|.hg|.node-version|.python-version|.ruby-version|.shorten_folder_marker|.svn|.terraform|CVS|Cargo.toml|composer.json|go.mod|package.json)";

/// truncate_with_folder_marker (p10k:1993-2007): runs of directories
/// between two marker-bearing ancestors (more than one component apart)
/// collapse into one elision mark.
fn shorten_folder_marker(parts: &mut Vec<String>, cwd: &str) {
    let marker = p9k_global("SHORTEN_FOLDER_MARKER", DEFAULT_FOLDER_MARKER);
    if marker.is_empty() {
        return;
    }
    let mut dir = cwd.to_string();
    let mut m: Vec<usize> = Vec::new(); // 1-based indices, descending
    let mut i = parts.len().saturating_sub(1);
    while i > 1 {
        dir = dir_head(&dir);
        if dir_has_entry_matching(&dir, &marker) {
            m.push(i);
        }
        i -= 1;
    }
    m.push(1);
    for k in 0..m.len() - 1 {
        // p10k:2002 — (( m[i] - m[i+1] > 2 )) && parts[m[i+1]+1,m[i]-1]=($'\1')
        if m[k] - m[k + 1] > 2 {
            parts.splice(m[k + 1]..m[k] - 1, [MARK_ELIDE.to_string()]);
        }
    }
}

/// p10k:1838-1865 — nearest ancestor (cwd upward) holding one of
/// `POWERLEVEL9K_DIR_PACKAGE_FILES` whose `jq .name` is non-empty.
/// Returns (number of leading components it replaces, package name).
fn dir_package_name(cwd: &str, nparts: usize) -> Option<(usize, String)> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, (std::time::SystemTime, String)>>> =
        OnceLock::new();
    // p10k:1839 — `$+commands[jq] == 1 && $#_POWERLEVEL9K_DIR_PACKAGE_FILES > 0`.
    crate::extensions::p10k::segments_sys::cmd_on_path("jq")?;
    let pats = match crate::ported::params::getaparam("POWERLEVEL9K_DIR_PACKAGE_FILES") {
        Some(v) => v,
        None => match getsparam("POWERLEVEL9K_DIR_PACKAGE_FILES") {
            Some(s) => vec![s],
            None => vec!["package.json".to_string(), "composer.json".to_string()],
        },
    };
    if pats.is_empty() {
        return None;
    }
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut dir = cwd.to_string();
    for levels in (1..=nparts).rev() {
        for pat in &pats {
            let mut t = pat.clone();
            crate::ported::glob::tokenize(&mut t);
            let Some(prog) = crate::ported::pattern::patcompile(&t, 0, None) else {
                continue;
            };
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .filter(|n| crate::ported::pattern::pattry(&prog, n))
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            for n in names {
                let file = format!("{dir}/{n}");
                let Ok(mtime) = std::fs::metadata(&file).and_then(|m| m.modified()) else {
                    continue;
                };
                let hit = cache.lock().ok().and_then(|c| match c.get(&file) {
                    Some((t, name)) if *t == mtime => Some(name.clone()),
                    _ => None,
                });
                let name = match hit {
                    Some(name) => name,
                    None => {
                        // p10k:1853 — jq -j '.name | select(. != null)' <$pkg_file
                        let name = std::fs::File::open(&file)
                            .ok()
                            .and_then(|f| {
                                std::process::Command::new("jq")
                                    .args(["-j", ".name | select(. != null)"])
                                    .stdin(f)
                                    .stderr(std::process::Stdio::null())
                                    .output()
                                    .ok()
                            })
                            .filter(|o| o.status.success())
                            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                            .unwrap_or_default();
                        if let Ok(mut c) = cache.lock() {
                            c.insert(file.clone(), (mtime, name.clone()));
                        }
                        name
                    }
                };
                if !name.is_empty() {
                    return Some((levels, name));
                }
            }
        }
        dir = dir_head(&dir);
    }
    None
}

/// Is `prefix` the start of exactly one sub-directory of `parent`?
/// (`$parent/$prefix*/(N)` yields one match, p10k:1938-1941.)
fn unique_dir_prefix(parent: &str, prefix: &str) -> bool {
    let Ok(rd) = std::fs::read_dir(if parent.is_empty() { "/" } else { parent }) else {
        return false;
    };
    let mut n = 0;
    for e in rd.flatten() {
        if !e.file_name().to_string_lossy().starts_with(prefix) {
            continue;
        }
        // The trailing `/` in the glob follows symlinks to directories.
        if std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()) {
            n += 1;
            if n > 1 {
                return false;
            }
        }
    }
    n == 1
}

/// Shortest unique prefix length (1-based char count `j`) of `rsub`
/// among the sub-directories of `parent` (p10k:1936-1941), cached per
/// (parent, name) against the parent's mtime like p10k's
/// `_p9k__dir_stat_cache` (p10k:1930-1958).
fn unique_prefix_len(parent: &str, rsub: &str, d: usize) -> usize {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, (std::time::SystemTime, usize)>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let key = format!("{parent}\0{rsub}\0{d}");
    let mtime = std::fs::metadata(if parent.is_empty() { "/" } else { parent })
        .and_then(|m| m.modified())
        .ok();
    if let Some(mt) = mtime {
        if let Some((t, j)) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
            if t == mt {
                return j;
            }
        }
    }
    let chars: Vec<char> = rsub.chars().collect();
    // p10k:1936 — `local -i j=$rsub[(i)[^.]]`: first non-dot (len+1 if none).
    let mut j = chars.iter().position(|&c| c != '.').map_or(chars.len() + 1, |i| i + 1);
    // p10k:1937-1941 — `for (( ; j + d < $#rsub; ++j ))`.
    while j + d < chars.len() {
        let prefix: String = chars[..j].iter().collect();
        if unique_dir_prefix(parent, &prefix) {
            break;
        }
        j += 1;
    }
    if let (Some(mt), Ok(mut c)) = (mtime, cache.lock()) {
        c.insert(key, (mt, j));
    }
    j
}

/// truncate_to_unique (p10k:1896-1992). Marks anchors (`\2`) and returns
/// the shortenable components as `(index into parts, shortened text,
/// columns saved)`, plus fake_first.
fn shorten_to_unique(parts: &mut Vec<String>, p: &str, cwd: &str) -> (Vec<(usize, String, i64)>, bool) {
    // p10k:1898-1901 — delimiter defaults to '*' here; length >= 0 else 1.
    let delim = match getsparam("POWERLEVEL9K_SHORTEN_DELIMITER") {
        Some(d) => decode_g(&d),
        None => "*".to_string(),
    };
    let mut shortenlen = match getsparam("POWERLEVEL9K_SHORTEN_DIR_LENGTH") {
        Some(v) if !v.is_empty() => v.trim().parse::<i64>().unwrap_or(0),
        _ => 1,
    };
    if shortenlen < 0 {
        shortenlen = 1;
    }
    let sl = shortenlen as usize;
    // p10k:7570-7575 — TRUNCATE_BEFORE_MARKER validation.
    let mut tbm = p9k_global("DIR_TRUNCATE_BEFORE_MARKER", "");
    if tbm == "first" || tbm == "last" {
        tbm.push_str(":0");
    }
    let tbm_ok = tbm.split_once(':').is_some_and(|(w, n)| {
        (w == "first" || w == "last")
            && n.strip_prefix('-').unwrap_or(n).chars().all(|c| c.is_ascii_digit())
            && !n.strip_prefix('-').unwrap_or(n).is_empty()
    });
    let folder_marker = p9k_global("SHORTEN_FOLDER_MARKER", DEFAULT_FOLDER_MARKER);
    if !tbm_ok || folder_marker.is_empty() {
        tbm.clear(); // p10k:7569/7576
    }

    let n = parts.len();
    let mut i = 2usize; // 1-based
    let mut e = n as i64 - shortenlen;
    let mut orig: Vec<String> = Vec::new();
    if !tbm.is_empty() {
        e += shortenlen;
        orig.push(parts.get(1).cloned().unwrap_or_default()); // p10k:1911
        let take = sl.min(n);
        orig.extend(parts[n - take..].iter().cloned());
    } else if p.starts_with('/') {
        i += 1; // p10k:1914
    }

    // p10k:1925-1932 — `$cwd[1,-2-$#rtail]`: the real path above component i.
    let rtail_len = if i <= n {
        parts[i - 1..].join("/").chars().count()
    } else {
        0
    };
    let cwd_chars: Vec<char> = cwd.chars().collect();
    let keep = cwd_chars.len().saturating_sub(rtail_len + 1);
    let mut parent: String = cwd_chars[..keep].iter().collect();

    // p10k:1937 — [[ -n $parts[i-1] ]] && parts[i-1]+=$'\2'
    if i >= 2 && i - 2 < n && !parts[i - 2].is_empty() {
        parts[i - 2].push(MARK_ANCHOR);
    }
    let d = shorten_delim_len(&delim);
    let mut steps: Vec<(usize, String, i64)> = Vec::new();
    while (i as i64) <= e && i <= n {
        let sub = parts[i - 1].clone();
        let dir = format!("{parent}/{sub}");
        if !folder_marker.is_empty() && dir_has_entry_matching(&dir, &folder_marker) {
            parts[i - 1].push(MARK_ANCHOR); // p10k:1944-1946
        } else {
            let j = unique_prefix_len(&parent, &sub, d);
            let chars: Vec<char> = sub.chars().collect();
            let tail: String = chars.iter().skip(j).collect();
            let saved = str_width(&tail) as i64 - d as i64; // p10k:1955
            if saved > 0 {
                let prefix: String = chars.iter().take(j).collect();
                steps.push((i - 1, format!("{MARK_UNIQ}{prefix}{MARK_ELIDE}{MARK_UNIQ}"), saved));
            }
        }
        parent.push('/');
        parent.push_str(&sub);
        i += 1;
    }

    let mut fake_first = false;
    if !tbm.is_empty() {
        // p10k:1970-1992 — truncate before the marker anchor.
        let (which, off) = tbm.split_once(':').unwrap_or(("first", "0"));
        let off: i64 = off.parse().unwrap_or(0);
        let is_anchor = |s: &String| s.ends_with(MARK_ANCHOR);
        let e2 = if which == "last" {
            parts.iter().rposition(is_anchor).map_or(0, |x| x as i64 + 1) + off
        } else {
            parts.iter().skip(1).position(is_anchor).map_or(parts.len() as i64 + 1, |x| x as i64 + 2) + off
        };
        if e2 > 1 && e2 as usize <= parts.len() {
            let cut = e2 as usize - 1;
            parts.drain(0..cut);
            steps.retain_mut(|(idx, _, _)| {
                if *idx < cut {
                    false
                } else {
                    *idx -= cut;
                    true
                }
            });
            fake_first = true;
        } else if p.starts_with('/') && p.len() > 1 && parts.len() > 1 {
            parts[1] = format!("{}{MARK_ANCHOR}", orig[0]);
            steps.retain(|(idx, _, _)| *idx != 1);
        }
        // p10k:1990-1998 — the kept tail reverts to its original names.
        let cnt = parts.len().min(sl);
        for back in (1..=cnt).rev() {
            if back > orig.len() {
                continue;
            }
            let at = parts.len() - back;
            parts[at] = format!("{}{MARK_ANCHOR}", orig[orig.len() - back]);
            steps.retain(|(idx, _, _)| *idx != at);
        }
    } else {
        // p10k:2000-2004 — the unshortened tail are anchors.
        for part in parts.iter_mut().skip(i - 1) {
            part.push(MARK_ANCHOR);
        }
    }
    (steps, fake_first)
}

/// Display width of a string (`(m)` flag semantics: wide chars count 2).
fn str_width(s: &str) -> usize {
    s.chars()
        .map(|c| crate::ported::zsh_h::WCWIDTH(c).max(0) as usize)
        .sum()
}

fn dir_segments() -> Vec<Segment> {
    let cwd = cwd();
    let home = home_dir();

    // p10k:1768-1801 — path text. DIR_PATH_ABSOLUTE skips contraction;
    // auto_name_dirs contracts HOME only (p10k:1771-1773); otherwise
    // `local p=${(%):-%~}` — zsh's %~, which abbreviates against $HOME
    // AND `hash -d` named dirs (finddir picks the best diff, so ~ZPWR
    // beats ~/.zpwr), routed through the faithful promptpath port
    // (Src/prompt.c:134). A dynamic named dir `~[name]/…` (zsh_directory_name
    // hook) keeps its `~[name]` head as ONE component (p10k:1779-1794);
    // when no hook function reproduces that head the path is split from
    // the absolute cwd (p10k:1795-1796).
    let mut dynamic_parts: Option<Vec<String>> = None;
    let p = if global_bool("DIR_PATH_ABSOLUTE", false) {
        cwd.clone()
    } else if crate::ported::options::opt_state_get("autonamedirs").unwrap_or(false) {
        contract_home(&cwd, &home)
    } else {
        let abbrev = crate::ported::prompt::promptpath(&cwd, 0, true, &home);
        if abbrev.starts_with("~[") {
            dynamic_parts = dynamic_named_parts(&abbrev, &cwd);
            if dynamic_parts.is_some() {
                abbrev
            } else {
                cwd.clone() // p10k:1795-1796
            }
        } else {
            abbrev
        }
    };
    // p10k:1799 — parts=("${(s:/:)p}") (quoted split keeps empties).
    let mut parts: Vec<String> = dynamic_parts.unwrap_or_else(|| p.split('/').map(String::from).collect());

    let mut fake_first = false; // p10k:1803
    let mut unique_steps: Vec<(usize, String, i64)> = Vec::new();
    let shortenlen = global_int("SHORTEN_DIR_LENGTH", -1); // p10k:1803 `:--1`

    // p10k:1805-1813 — delimiter: SHORTEN_DELIMITER if SET (even
    // empty), else '…' (UTF-8 assumed; the '..' arm is non-UTF-8 only).
    let mut delim = match getsparam("POWERLEVEL9K_SHORTEN_DELIMITER") {
        Some(d) => decode_g(&d),
        None => "\u{2026}".to_string(),
    };

    // p10k:1815-2018 — shortening strategy.
    let strategy = p9k_global("SHORTEN_STRATEGY", "");
    match strategy.as_str() {
        "truncate_absolute" | "truncate_absolute_chars" => {
            shorten_absolute(&mut parts, &p, shortenlen, &delim); // p10k:1816
        }
        "truncate_with_package_name" | "truncate_middle" | "truncate_from_right" => {
            if strategy == "truncate_with_package_name" {
                // p10k:1836-1865 — nearest package file with a name wins.
                if let Some((levels, name)) = dir_package_name(&cwd, parts.len()) {
                    parts.splice(0..levels, [name]); // p10k:1858
                    fake_first = true; // p10k:1859
                }
            }
            shorten_middle_or_right(
                &mut parts,
                shortenlen,
                &delim,
                strategy == "truncate_middle", // p10k:1869
            );
        }
        "truncate_to_last" => {
            fake_first = shorten_to_last(&mut parts, &p, shortenlen); // p10k:1879
        }
        "truncate_to_first_and_last" => {
            shorten_first_and_last(&mut parts, &p, shortenlen); // p10k:1888
        }
        "truncate_to_unique" => {
            // p10k:1896-1992 — filesystem-anchored uniqueness scan; the
            // shortenable components are recorded as steps so the final
            // prompt assembly can apply them left to right only as far as
            // the available width demands (`_p9k__d`, p10k:6143).
            if getsparam("POWERLEVEL9K_SHORTEN_DELIMITER").is_none() {
                delim = "*".to_string(); // p10k:1898 `${..SHORTEN_DELIMITER-'*'}`
            }
            let (st, ff) = shorten_to_unique(&mut parts, &p, &cwd);
            unique_steps = st;
            fake_first = ff;
        }
        "truncate_with_folder_marker" => {
            shorten_folder_marker(&mut parts, &cwd); // p10k:1993-2007
        }
        _ => shorten_default(&mut parts, shortenlen), // p10k:2009-2017
    }

    // p10k:2020-2025 — writability probe.
    // w=0 writable, w=1 not writable, w=2 does not exist.
    let show_writable = match p9k_global("DIR_SHOW_WRITABLE", "").as_str() {
        "true" => 1, // p10k:7315
        "v2" => 2,   // p10k:7316
        "v3" => 3,   // p10k:7317
        _ => 0,      // p10k:7318
    };
    let mut w = i32::from(show_writable != 0 && !path_writable(&cwd)); // p10k:2023-2024
    if w != 0 && show_writable > 2 && !Path::new(&cwd).exists() {
        w = 2; // p10k:2025
    }

    // p10k:2027-2036 — DIR_CLASSES state + icon.
    let mut state: Option<String> = None;
    let mut icon = String::new();
    for (pat, class, glyph) in dir_classes() {
        if glob_match(&pat, &cwd, &home) {
            if !class.is_empty() {
                state = Some(class); // p10k:2032
            }
            icon = glyph; // p10k:2033
            break;
        }
    }
    // p10k:2037-2046 — writability overrides state + icon.
    if w != 0 {
        if show_writable == 1 {
            state = Some("NOT_WRITABLE".to_string()); // p10k:2039
        } else if w == 2 {
            state = Some(match state {
                Some(s) => format!("{s}_NON_EXISTENT"), // p10k:2041
                None => "NON_EXISTENT".to_string(),
            });
        } else {
            state = Some(match state {
                Some(s) => format!("{s}_NOT_WRITABLE"), // p10k:2043
                None => "NOT_WRITABLE".to_string(),
            });
        }
        icon = seg_icon("dir", state.as_deref(), "LOCK_ICON"); // p10k:2045
    }
    let state_ref = state.as_deref();

    // p10k:2050-2056 — segment style (defaults: bg blue, fg color1).
    let bg = p9k_param("dir", state_ref, "BACKGROUND", "blue"); // p10k:2051
    let fg = p9k_param("dir", state_ref, "FOREGROUND", color1()); // p10k:2054
    let style = format!("%b{}{}", bgesc(&bg), fgesc(&fg)); // p10k:2050-2056

    let format_parts = |mut parts: Vec<String>| -> String {
    // p10k:2062 — escape %.
    for part in parts.iter_mut() {
        *part = esc_pct(part);
    }

    // p10k:2063-2069 — HOME abbreviation / omit-first-character.
    let abbrev = decode_g(&p9k_global("HOME_FOLDER_ABBREVIATION", "~")); // p10k:7311
    if abbrev != "~" && !fake_first && (p == "~" || p.starts_with("~/")) {
        parts[0] = abbrev.clone(); // p10k:2065
        if parts[0].contains('%') {
            parts[0].push_str(&style); // p10k:2066
        }
    } else if global_bool("DIR_OMIT_FIRST_CHARACTER", false)
        && !fake_first
        && parts.len() > 1
        && parts[0].is_empty()
        && !parts[1].is_empty()
    {
        parts.remove(0); // p10k:2067-2068
    }

    // p10k:2071-2083 — last-component highlight.
    let mut last_style = String::new();
    if p9k_param("dir", state_ref, "PATH_HIGHLIGHT_BOLD", "") == "true" {
        last_style.push_str("%B"); // p10k:2072-2073
    }
    if dir_param_exists(state_ref, "PATH_HIGHLIGHT_FOREGROUND") {
        let c = p9k_param("dir", state_ref, "PATH_HIGHLIGHT_FOREGROUND", "");
        last_style.push_str(&fgesc(&c)); // p10k:2076-2078
    }
    if !last_style.is_empty() {
        if let Some(last) = parts.last_mut() {
            // p10k:2082 — restyle after each elision mark too.
            let restyled = last.replace(MARK_ELIDE, &format!("{MARK_ELIDE}{last_style}"));
            *last = format!("{last_style}{restyled}{style}");
        }
    }

    // p10k:2085-2104 — anchor (\2-marked component) highlight.
    let mut anchor_style = String::new();
    if p9k_param("dir", state_ref, "ANCHOR_BOLD", "") == "true" {
        anchor_style.push_str("%B"); // p10k:2086-2087
    }
    if dir_param_exists(state_ref, "ANCHOR_FOREGROUND") {
        let c = p9k_param("dir", state_ref, "ANCHOR_FOREGROUND", "");
        anchor_style.push_str(&fgesc(&c)); // p10k:2090-2092
    }
    if !anchor_style.is_empty() {
        // p10k:2096-2101 — anchors get style+reset; with a last_style
        // the final component just drops its marker.
        let n = parts.len();
        for (i, part) in parts.iter_mut().enumerate() {
            if let Some(stripped) = part.strip_suffix(MARK_ANCHOR) {
                if last_style.is_empty() || i + 1 < n {
                    *part = format!("{anchor_style}{stripped}{style}");
                } else {
                    *part = stripped.to_string(); // p10k:2100
                }
            }
        }
    } else {
        for part in parts.iter_mut() {
            if part.ends_with(MARK_ANCHOR) {
                part.pop(); // p10k:2103
            }
        }
    }

    // p10k:2106-2121 — elision-mark substitution, optionally with the
    // SHORTENED_FOREGROUND tint.
    if dir_param_exists(state_ref, "SHORTENED_FOREGROUND") {
        let sfg = fgesc(&p9k_param("dir", state_ref, "SHORTENED_FOREGROUND", "")); // p10k:2108-2111
        let mut dl = delim.clone();
        if dl.contains('%') {
            dl.push_str(&style);
            dl.push_str(&sfg); // p10k:2113
        }
        for part in parts.iter_mut() {
            if let Some(i) = part.find(MARK_ELIDE) {
                // p10k:2115 — $shortened_fg$match[1]$delim$match[2]$style.
                let before = part[..i].to_string();
                let after = part[i + MARK_ELIDE.len_utf8()..].to_string();
                *part = format!("{sfg}{before}{dl}{after}{style}");
            }
            *part = part.replace(MARK_UNIQ, ""); // p10k:2114 (\3 wrap unused)
        }
    } else {
        let mut dl = delim.clone();
        if dl.contains('%') {
            dl.push_str(&style); // p10k:2118
        }
        for part in parts.iter_mut() {
            *part = part.replace(MARK_ELIDE, &dl); // p10k:2119
            *part = part.replace(MARK_UNIQ, ""); // p10k:2120
        }
    }

    // p10k:2123-2139 — separator.
    let sep = if cwd == "/" && global_bool("DIR_OMIT_FIRST_CHARACTER", false) {
        "/".to_string() // p10k:2124
    } else {
        let mut sep = String::new();
        if dir_param_exists(state_ref, "PATH_SEPARATOR_FOREGROUND") {
            let c = p9k_param("dir", state_ref, "PATH_SEPARATOR_FOREGROUND", "");
            sep.push_str(&fgesc(&c)); // p10k:2129-2132
        }
        sep.push_str(&decode_g(&p9k_param(
            "dir",
            state_ref,
            "PATH_SEPARATOR",
            "/",
        ))); // p10k:2134-2137
        if sep.contains('%') {
            sep.push_str(&style); // p10k:2138
        }
        sep
    };

    // p10k:2141 — content = parts joined on the separator.
    let content = parts.join(&sep);
    // p10k:2141-2153 — DIR_HYPERLINK: OSC 8 file:// link around the path
    // when the terminal supports hyperlinks and cwd is absolute.
    if global_bool("DIR_HYPERLINK", false) && term_has_href() && cwd.starts_with('/') {
        return format!(
            "%{{\u{1b}]8;;file://{}\u{7}%}}{content}%{{\u{1b}]8;;\u{7}%}}",
            url_escape(&cwd)
        );
    }
    content
};

    let content = format_parts(parts.clone());
    if !unique_steps.is_empty() {
        // p10k:6143 — every prefix of the step list is a candidate
        // rendering; render.rs picks the shortest-needed one.
        let mut variants = vec![content.clone()];
        let mut saved = Vec::new();
        let mut cur = parts.clone();
        for (idx, short, sv) in &unique_steps {
            cur[*idx] = short.clone();
            variants.push(format_parts(cur.clone()));
            saved.push(*sv);
        }
        DIR_UNIQUE.with(|d| {
            let mut d = d.borrow_mut();
            if d.len() >= 8 {
                d.clear(); // never rendered (no render_prompt) — drop stale plans
            }
            d.push(DirUnique { variants, saved });
        });
    }
    // p10k:2168 — final segment; VISUAL_IDENTIFIER / CONTENT expansion
    // hooks apply as in _p9k_prompt_segment.
    let seg_state = state.clone();
    let icon = apply_visual_identifier("dir", state_ref, icon);
    let content = apply_content_expansion("dir", state_ref, content);
    vec![Segment {
        name: "dir".to_string(),
        state: seg_state,
        content,
        icon,
        fg,
        bg,
    }]
}

// ---------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// p10k:2029-2035 — `[[ $_p9k__cwd == ${(e)~a} ]]` matches each
    /// DIR_CLASSES pattern as a GLOB. The patterns were handed to patcompile
    /// untokenized, so `*` was literal: a subdirectory of $HOME matched no
    /// class and lost HOME_SUB_ICON, and `*` never caught the default.
    #[test]
    fn dir_class_patterns_glob() {
        let _g = crate::test_util::global_state_lock();
        let home = "/Users/someone";
        assert!(glob_match("~", "/Users/someone", home)); // exact HOME
        assert!(glob_match("~/*", "/Users/someone/Library/Caches", home));
        assert!(!glob_match("~/*", "/Users/other/x", home));
        assert!(!glob_match("~/*", "/Users/someone", home)); // HOME itself is `~`
        assert!(glob_match("/etc|/etc/*", "/etc/ssh", home));
        assert!(glob_match("*", "/opt/homebrew", home)); // DEFAULT catch-all
    }

    #[test]
    fn exit2str_maps_signals_verbosely() {
        // Defaults: HIDE_SIGNAME off, VERBOSE_SIGNAME on (p10k:7472-7473).
        // 130 = 128 + SIGINT(2) → "SIGINT(2)" (p10k:8466-8468).
        assert_eq!(exit2str(130), "SIGINT(2)");
        assert_eq!(exit2str(129), "SIGHUP(1)");
        // ≤128 stays numeric (p10k:8462 {0..255} identity slots).
        assert_eq!(exit2str(0), "0");
        assert_eq!(exit2str(1), "1");
        assert_eq!(exit2str(128), "128");
    }

    /// publish_vcs_status exposes every VCS_STATUS_* param a user
    /// VCS_CONTENT_EXPANSION (my_git_formatter) reads, so the formatter
    /// builds the rich git format instead of echoing an empty
    /// P9K_CONTENT.
    /// p10k:327-342/5736 — human-readable byte rates.
    #[test]
    fn ip_rate_formatting_matches_p10k() {
        assert_eq!(rate_str(0.0), "0 B/s");
        assert_eq!(rate_str(512.0), "512 B/s");
        assert_eq!(rate_str(1024.0), "1 KiB/s");
        assert_eq!(rate_str(1536.0), "1.5 KiB/s");
        assert_eq!(rate_str(1024.0 * 1024.0), "1 MiB/s");
        // ≥100 in a unit → "N." integer form then dot-stripped.
        assert_eq!(rate_str(200.0 * 1024.0), "200 KiB/s");
    }

    #[test]
    fn publish_vcs_status_sets_params() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::params::getsparam;
        let gs = git::GitStatus {
            branch: "main".into(),
            commit: "abc123".into(),
            remote_branch: "origin/main".into(),
            ahead: 3,
            behind: 1,
            staged: 2,
            unstaged: 4,
            untracked: 5,
            conflicted: 0,
            stashes: 1,
            ..git::GitStatus::default()
        };
        publish_vcs_status(&gs);
        assert_eq!(
            getsparam("VCS_STATUS_LOCAL_BRANCH").as_deref(),
            Some("main")
        );
        assert_eq!(
            getsparam("VCS_STATUS_REMOTE_BRANCH").as_deref(),
            Some("origin/main")
        );
        assert_eq!(getsparam("VCS_STATUS_COMMITS_AHEAD").as_deref(), Some("3"));
        assert_eq!(getsparam("VCS_STATUS_COMMITS_BEHIND").as_deref(), Some("1"));
        assert_eq!(getsparam("VCS_STATUS_NUM_STAGED").as_deref(), Some("2"));
        assert_eq!(getsparam("VCS_STATUS_NUM_UNSTAGED").as_deref(), Some("4"));
        assert_eq!(getsparam("VCS_STATUS_NUM_UNTRACKED").as_deref(), Some("5"));
        assert_eq!(getsparam("VCS_STATUS_STASHES").as_deref(), Some("1"));
        // HAS_* are 1/0 from the counts.
        assert_eq!(getsparam("VCS_STATUS_HAS_UNSTAGED").as_deref(), Some("1"));
        assert_eq!(getsparam("VCS_STATUS_HAS_CONFLICTED").as_deref(), Some("0"));
    }

    #[test]
    fn vcs_state_precedence_matches_p10k() {
        // p10k:3910-3924 — MODIFIED beats UNTRACKED; CONFLICTED needs
        // the (default-off) VCS_CONFLICTED_STATE gate.
        let mut gs = git::GitStatus::default();
        assert_eq!(vcs_state_for(&gs), "CLEAN");
        gs.untracked = 3;
        assert_eq!(vcs_state_for(&gs), "UNTRACKED");
        gs.unstaged = 1;
        assert_eq!(vcs_state_for(&gs), "MODIFIED");
        gs.conflicted = 1; // gate off by default → still MODIFIED
        assert_eq!(vcs_state_for(&gs), "MODIFIED");
    }

    #[test]
    fn shorten_default_collapses_leading_components() {
        // p10k:2009-2017 with shortenlen=1 on /usr/local/share/zsh.
        let mut parts: Vec<String> = "/usr/local/share/zsh"
            .split('/')
            .map(String::from)
            .collect();
        shorten_default(&mut parts, 1);
        assert_eq!(parts, vec![MARK_ELIDE.to_string(), "zsh".to_string()]);
        // shortenlen -1 (the user's live config) → untouched.
        let mut parts2: Vec<String> = "/usr/local".split('/').map(String::from).collect();
        shorten_default(&mut parts2, -1);
        assert_eq!(parts2, vec!["", "usr", "local"]);
    }

    #[test]
    fn shorten_to_last_keeps_tail_and_sets_fake_first() {
        // p10k:1879-1887 with shortenlen=2.
        let p = "/usr/local/share/zsh";
        let mut parts: Vec<String> = p.split('/').map(String::from).collect();
        let fake = shorten_to_last(&mut parts, p, 2);
        assert!(fake);
        assert_eq!(parts, vec!["share", "zsh"]);
    }

    #[test]
    fn shorten_first_and_last_elides_middle() {
        // p10k:1889-1895 with shortenlen=1 on an absolute path:
        // i = shortenlen+1 = 2, absolute → ++i = 3; loop marks
        // parts[3..=$#parts-shortenlen] = parts[3..=5] (1-based), i.e.
        // b, c, d each become \1 — one mark PER elided component.
        let p = "/a/b/c/d/e";
        let mut parts: Vec<String> = p.split('/').map(String::from).collect();
        shorten_first_and_last(&mut parts, p, 1);
        let m = MARK_ELIDE.to_string();
        assert_eq!(
            parts,
            vec![
                "".to_string(),
                "a".to_string(),
                m.clone(),
                m.clone(),
                m,
                "e".to_string(),
            ]
        );
    }

    #[test]
    fn shorten_middle_and_right_squeeze_components() {
        // p10k:1866-1877 — pref=2, delim '…' (len 1), interior only.
        let mut parts: Vec<String> =
            vec!["".into(), "projects".into(), "deeply".into(), "last".into()];
        shorten_middle_or_right(&mut parts, 2, "\u{2026}", false);
        assert_eq!(parts[1], format!("pr{MARK_ELIDE}"));
        assert_eq!(parts[2], format!("de{MARK_ELIDE}"));
        assert_eq!(parts[3], "last"); // last component untouched

        let mut parts2: Vec<String> = vec!["".into(), "projects".into(), "last".into()];
        shorten_middle_or_right(&mut parts2, 2, "\u{2026}", true);
        assert_eq!(parts2[1], format!("pr{MARK_ELIDE}ts"));
    }

    #[test]
    fn contract_home_variants() {
        assert_eq!(contract_home("/Users/u/x", "/Users/u"), "~/x");
        assert_eq!(contract_home("/Users/u", "/Users/u"), "~");
        assert_eq!(contract_home("/Users/uv", "/Users/u"), "/Users/uv");
        assert_eq!(contract_home("/etc", "/Users/u"), "/etc");
        assert_eq!(contract_home("/etc", ""), "/etc");
    }

    #[test]
    fn shorten_absolute_keeps_tail_chars() {
        // p10k:1816-1835 — shortenlen=6 on /usr/local/bin (14 chars):
        // "bin" + its slash (4) fits in 6 leaving n=2; "local" doesn't
        // fit → keep its last 2 chars behind the elision mark.
        let p = "/usr/local/bin";
        let mut parts: Vec<String> = p.split('/').map(String::from).collect();
        shorten_absolute(&mut parts, p, 6, "\u{2026}");
        assert_eq!(parts, vec![format!("{MARK_ELIDE}al"), "bin".to_string()]);
    }
}
