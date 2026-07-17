//! p10k CORE segments — dir / status / context / ssh / root_indicator /
//! history / prompt_char / ip / dir_writable / background_jobs /
//! custom_* / vcs.
//!
//! `~/forkedRepos/powerlevel10k/internal/p10k.zsh` is THE SPEC; ported
//! lines are cited as `// p10k:NNN`. This is src/extensions (Rust-native
//! layer), so there are no C-port constraints — but segment SEMANTICS
//! (states, colors, content, hide rules) mirror the zsh theme exactly.
//!
//! Phase-1 scope notes (each TODO is also traced at the call site):
//! - dir: SHORTEN_STRATEGY values truncate_absolute(_chars),
//!   truncate_middle, truncate_from_right, truncate_to_last,
//!   truncate_to_first_and_last and the default strategy are ported.
//!   truncate_to_unique / truncate_with_folder_marker /
//!   truncate_with_package_name (p10k:1837-1865,1897-2007) fall back to
//!   the default strategy with a debug log. The user's live config sets
//!   NO SHORTEN_STRATEGY (~/.zpwr/env/.p10k.zsh:242 commented out), so
//!   the default (no-truncation) path is what actually runs.
//! - prompt_char: keymap is VIINS always; live VICMD/VIVIS/VIOWR keymap
//!   tracking needs a ZLE hook (TODO).
//! - vcs: renders p10k's own gitstatus format (p10k:3926-4005). The
//!   user's `my_git_formatter` CONTENT_EXPANSION (a zsh function) is not
//!   evaluated; states/counts/icons come from the same underlying data.

use crate::extensions::p10k::config::{p9k_global, p9k_global_arr, p9k_param};
use crate::extensions::p10k::git;
use crate::extensions::p10k::icons;
use crate::extensions::p10k::render::Segment;
use crate::ported::params::{getsparam, pipestatgetfn};
use crate::ported::utils::getkeystring;
use std::path::Path;
use std::sync::atomic::Ordering;

// p10k markers used by prompt_dir's part rewriting:
// $'\1' — "shortened here" placeholder replaced by the delimiter.
const MARK_ELIDE: char = '\u{1}';
// $'\2' — trailing anchor marker (kept-component highlighting).
const MARK_ANCHOR: char = '\u{2}';
// $'\3' — truncate_to_unique bracket marker (strategy unported; stripped).
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

/// Read a parameter, falling back to the process environment. Exported
/// env (SSH_CONNECTION, SUDO_COMMAND, …) normally lives in the paramtab
/// already; the env fallback covers early-startup renders.
fn env_or_param(name: &str) -> String {
    if let Some(v) = getsparam(name) {
        return v;
    }
    std::env::var(name).unwrap_or_default()
}

/// `_p9k_declare -b POWERLEVEL9K_<name> <default>` read semantics
/// (p10k:141-151): ONLY the literal string `true` is truthy; unset uses
/// the declared default.
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

/// Escape `%` for prompt-expansion contexts — p10k's ubiquitous
/// `${x//\%/%%}`.
fn esc_pct(s: &str) -> String {
    s.replace('%', "%%")
}

/// zsh `${(g::)x}` echo-style escape decoding (\uNNNN, \e, \n, …) as
/// p10k applies to user-supplied icons/templates (p10k:524, 1602, 2135).
fn decode_g(s: &str) -> String {
    getkeystring(s).0
}

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

/// Port of `_p9k_get_icon $1 $2` (p10k:511-530): probe the user-param
/// chain for `<KEY>` (e.g. POWERLEVEL9K_VCS_CLEAN_VCS_BRANCH_ICON →
/// POWERLEVEL9K_VCS_VCS_BRANCH_ICON → POWERLEVEL9K_VCS_BRANCH_ICON); on
/// a hit the value gets `(g::)` decoding (p10k:524) plus the
/// backspace-wrap quirk (p10k:525 — "penance for past sins"); on a
/// whole-chain miss the mode icon table answers (p10k:521 —
/// `${icons[$2]-...}`).
fn seg_icon(segment: &str, state: Option<&str>, key: &str) -> String {
    // \x01 sentinel mirrors p10k's own $'\1' "raw glyph" marker trick.
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

/// VISUAL_IDENTIFIER_EXPANSION hook (p10k:720/951) applied to a
/// resolved icon. Default `'${P9K_VISUAL_IDENTIFIER}'` = the icon
/// itself. Literal user values ('✔', '⭐') replace it; expansions that
/// reference other shell state are unported (debug log, keep icon).
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
        exp // literal override, e.g. STATUS_OK_VISUAL_IDENTIFIER_EXPANSION='✔'
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

/// CONTENT_EXPANSION hook (p10k:724/955). Default `'${P9K_CONTENT}'` =
/// segment-computed content. Literal user values replace it; templates
/// embedding `${P9K_CONTENT}` are substituted; anything needing live
/// zsh evaluation (e.g. the user's `my_git_formatter`) is unported.
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

/// The style-name used for parameter probing. `prompt_char` is the one
/// segment whose zsh style name is `prompt_prompt_char` (p10k:3313 —
/// `$0_ERROR_VIINS` with $0=prompt_prompt_char); passing the bare name
/// would make config.rs treat "prompt_" as the style prefix and probe
/// POWERLEVEL9K_CHAR_* instead of POWERLEVEL9K_PROMPT_CHAR_*.
fn param_seg(name: &str) -> &str {
    if name == "prompt_char" {
        "prompt_prompt_char"
    } else {
        name
    }
}

/// Common constructor mirroring what `_p9k_prompt_segment
/// name bg fg icon expand cond content` resolves per-segment
/// (p10k:1101 + the color/icon/expansion hooks in
/// _p9k_left_prompt_segment): BACKGROUND/FOREGROUND via the param
/// chain, icon via _p9k_get_icon + VISUAL_IDENTIFIER_EXPANSION, content
/// via CONTENT_EXPANSION.
fn make_segment(
    name: &str,
    state: Option<&str>,
    default_bg: &str,
    default_fg: &str,
    icon_key: &str,
    content: String,
) -> Segment {
    let seg = param_seg(name);
    let bg = p9k_param(seg, state, "BACKGROUND", default_bg);
    let fg = p9k_param(seg, state, "FOREGROUND", default_fg);
    let icon_glyph = if icon_key.is_empty() {
        String::new()
    } else {
        seg_icon(seg, state, icon_key)
    };
    let icon = apply_visual_identifier(seg, state, icon_glyph);
    let content = apply_content_expansion(seg, state, content);
    Segment {
        name: name.to_string(),
        state: state.map(|s| s.to_string()),
        content,
        icon,
        fg,
        bg,
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

/// p10k:8211-8214 — `[[ -n $SSH_CLIENT || -n $SSH_TTY ||
/// -n $SSH_CONNECTION ]] && P9K_SSH=1`. The `who`-based fallback for
/// su-after-ssh (p10k:8216-8233) is unported (needs a fork).
fn is_ssh() -> bool {
    !env_or_param("SSH_CLIENT").is_empty()
        || !env_or_param("SSH_TTY").is_empty()
        || !env_or_param("SSH_CONNECTION").is_empty()
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
    //   p10k:3333 VIOWR — `$_p9k__zle_state` contains `overwrite` ▶
    //     (only under PROMPT_CHAR_OVERWRITE_STATE; TODO — needs the
    //     ZLE insert-mode flag surfaced; state defaults off).
    // The native engine reads the live ZLE keymap directly. At
    // preprompt time a fresh line starts in the insert keymap; the
    // mid-line keymap-select repaint is TODO (needs a native
    // zle-keymap-select hook to re-render). Region can't be active at
    // a fresh prompt, so vicmd → VICMD without the region check.
    let keymap_name = crate::ported::zle::zle_keymap::curkeymapname().clone();
    let keymap = match keymap_name.as_str() {
        "vicmd" => "VICMD",           // p10k:3338
        "vivis" | "vivli" => "VIVIS", // p10k:3339
        _ => "VIINS",                 // p10k:3336
    };
    let state = format!("{}_{}", if ok { "OK" } else { "ERROR" }, keymap);
    // p10k:3348 — `$0_OK_VIINS "$_p9k_color1" 76 '' … '❯'`
    // p10k:3336 — `$0_ERROR_VIINS "$_p9k_color1" 196 '' … '❯'`
    let default_fg = if ok { "76" } else { "196" };
    // Glyph per state: ❯ VIINS (p10k:3336), ❮ VICMD (p10k:3338),
    // Ⅴ VIVIS (p10k:3339).
    let glyph = match keymap {
        "VICMD" => "\u{276E}", // ❮
        "VIVIS" => "\u{2164}", // Ⅴ
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
    // p10k:2295 — `_p9k_prompt_segment "$0" "cyan" "$_p9k_color1" 'NETWORK_ICON' …`
    vec![make_segment("ip", None, "cyan", color1(), "NETWORK_ICON", ip)]
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
    // p10k:1698 — `content="$(eval $command)"`: run through the live
    // executor's command-substitution path (captures stdout, strips
    // trailing newlines). The $+functions/$+commands pre-check
    // (p10k:1697) is subsumed: a missing command yields empty output.
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
fn vcs_state_default_bg(state: &str) -> &'static str {
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
        if n > minl && n > sl && (strategy == "truncate_middle" || strategy == "truncate_from_right")
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

fn vcs_segments() -> Vec<Segment> {
    // p10k:4137 — configured backends; only git is ported.
    let mut backends = p9k_global_arr("VCS_BACKENDS");
    if backends.is_empty() {
        backends.push("git".to_string()); // p10k:7480 default (git)
    }
    if !backends.iter().any(|b| b == "git") {
        tracing::debug!(target: "p10k", ?backends, "non-git VCS backends unported");
        return vec![];
    }

    let cwd = cwd();

    // p10k:4014-4017 _p9k_maybe_ignore_git_repo — repos whose workdir
    // matches VCS_DISABLED_WORKDIR_PATTERN are treated as no-repo.
    // GitStatus does not expose the workdir, so the cwd is matched
    // instead: exact for `~` (the user's live value); repos rooted at a
    // matching dir but entered from a subdir diverge (TODO: needs
    // workdir on GitStatus).
    let disabled = getsparam("POWERLEVEL9K_VCS_DISABLED_WORKDIR_PATTERN").unwrap_or_default();
    if !disabled.is_empty() && glob_match(&disabled, &cwd, &home_dir()) {
        return vec![];
    }

    let mut gs = match git::git_status_for(Path::new(&cwd)) {
        Some(g) => g,
        None => return vec![], // p10k:3850-3851 — no repo, no segment
    };

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

    if global_bool("VCS_DISABLE_GITSTATUS_FORMATTING", false) {
        // p10k:3854-3868 — content comes from the user's
        // CONTENT_EXPANSION (a live zsh function in this user's config).
        // Unevaluable natively; fall through to p10k's own gitstatus
        // format, which renders the same counts.
        tracing::debug!(
            target: "p10k",
            "VCS_DISABLE_GITSTATUS_FORMATTING=true: custom CONTENT_EXPANSION not evaluated, \
             rendering p10k default vcs format"
        );
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
    let bg = p9k_param("vcs", Some(state), "BACKGROUND", vcs_state_default_bg(state));
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

    // p10k:3829-3837 _p9k_vcs_icon — remote-URL-specific icons
    // (github/gitlab/bitbucket) need GitStatus.remote_url, which the
    // phase-1 git backend does not expose. TODO(phase-2). Generic git
    // icon meanwhile.
    let icon = apply_visual_identifier(
        "vcs",
        Some(state),
        seg_icon("vcs", Some(state), "VCS_GIT_ICON"),
    );

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

/// `${(%):-%~}` HOME-only contraction. Named dirs / auto_name_dirs /
/// zsh_directory_name (p10k:1772-1800) are unported (TODO); the `~[…]`
/// dance there exists solely for dynamic named dirs.
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

fn dir_segments() -> Vec<Segment> {
    let cwd = cwd();
    let home = home_dir();

    // p10k:1769-1776 — path text. DIR_PATH_ABSOLUTE skips contraction.
    let p = if global_bool("DIR_PATH_ABSOLUTE", false) {
        cwd.clone()
    } else {
        contract_home(&cwd, &home)
    };
    // p10k:1799 — parts=("${(s:/:)p}") (quoted split keeps empties).
    let mut parts: Vec<String> = p.split('/').map(String::from).collect();

    let mut fake_first = false; // p10k:1803
    let shortenlen = global_int("SHORTEN_DIR_LENGTH", -1); // p10k:1803 `:--1`

    // p10k:1805-1813 — delimiter: SHORTEN_DELIMITER if SET (even
    // empty), else '…' (UTF-8 assumed; the '..' arm is non-UTF-8 only).
    let delim = match getsparam("POWERLEVEL9K_SHORTEN_DELIMITER") {
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
                // p10k:1838-1865 — needs jq + package.json stat cache.
                tracing::debug!(
                    target: "p10k",
                    "truncate_with_package_name package lookup unported; length squeeze only"
                );
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
        "truncate_to_unique" | "truncate_with_folder_marker" => {
            // p10k:1897-2007 — filesystem-anchored uniqueness scan;
            // TODO(phase-2). Falls back to full path (no truncation).
            tracing::debug!(target: "p10k", %strategy, "dir shorten strategy unported — showing full path");
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
        sep.push_str(&decode_g(&p9k_param("dir", state_ref, "PATH_SEPARATOR", "/"))); // p10k:2134-2137
        if sep.contains('%') {
            sep.push_str(&style); // p10k:2138
        }
        sep
    };

    // p10k:2141 — content = parts joined on the separator.
    let content = parts.join(&sep);
    // p10k:2142-2153 — DIR_HYPERLINK OSC-8 wrapping unported (the
    // user's config sets it false).
    if global_bool("DIR_HYPERLINK", false) {
        tracing::debug!(target: "p10k", "DIR_HYPERLINK unported — plain dir content");
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
        let mut parts: Vec<String> = vec![
            "".into(),
            "projects".into(),
            "deeply".into(),
            "last".into(),
        ];
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
