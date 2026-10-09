//! powerlevel10k icon tables — port of internal/icons.zsh.
//!
//! Spec: internal/icons.zsh (mode tables, the flat/compatible overrides
//! and the ICON_PADDING post-processing at icons.zsh:1131-1142) and
//! internal/p10k.zsh:510-530 (_p9k_get_icon override order).
//!
//! Mode selection follows icons.zsh:3-4: POWERLEVEL9K_MODE when set,
//! else the default (`*`) table in a UTF-8 locale, else `ascii`.
//!   - nerdfont-complete / nerdfont-fontconfig (the common mode) is
//!     baked in below (`icon_raw`), with the NON-legacy `$s`/`$q`
//!     expansion (s=' ', q=''); under ICON_PADDING=none legacy and
//!     non-legacy spacing collapse to identical values.
//!   - every other mode, and legacy spacing with padding, is read from
//!     the theme's own `internal/icons.zsh` (under the p10k install
//!     root captured when the theme was sourced) by `theme_icons`, so
//!     the tables never drift from upstream. If that file cannot be
//!     read the baked nerdfont-complete table answers.

use crate::ported::params::getsparam;

// icons.zsh:834-840 — after the `%% #` trailing-space strip, exactly these
// keys get one space appended back. Every one of them carries exactly one
// trailing space in the raw table below, so strip-then-append-one-space is
// the identity for them: return the raw literal unchanged.
const PADDED_KEYS: [&str; 7] = [
    "LEFT_SEGMENT_END_SEPARATOR",   // icons.zsh:834
    "MULTILINE_LAST_PROMPT_PREFIX", // icons.zsh:835
    "VCS_TAG_ICON",                 // icons.zsh:836
    "VCS_BOOKMARK_ICON",            // icons.zsh:837
    "VCS_COMMIT_ICON",              // icons.zsh:838
    "VCS_BRANCH_ICON",              // icons.zsh:839
    "VCS_REMOTE_BRANCH_ICON",       // icons.zsh:840
];

/// Raw nerdfont-complete table — icons.zsh:424-551, $s/$q expanded
/// non-legacy (s=' ', q=''). `None` = key not in this mode's table.
fn icon_raw(name: &str) -> Option<&'static str> {
    // icons.zsh:421-423 — nerd-font patched (complete) font required! See
    // https://github.com/ryanoasis/nerd-fonts
    // http://nerdfonts.com/#cheat-sheet
    let glyph = match name {
        "RULER_CHAR" => "\u{2500}",                 // icons.zsh:425 '─' ─
        "LEFT_SEGMENT_SEPARATOR" => "\u{E0B0}",     // icons.zsh:426 ''
        "RIGHT_SEGMENT_SEPARATOR" => "\u{E0B2}",    // icons.zsh:427 ''
        "LEFT_SEGMENT_END_SEPARATOR" => " ",        // icons.zsh:428 whitespace
        "LEFT_SUBSEGMENT_SEPARATOR" => "\u{E0B1}",  // icons.zsh:429 ''
        "RIGHT_SUBSEGMENT_SEPARATOR" => "\u{E0B3}", // icons.zsh:430 ''
        "CARRIAGE_RETURN_ICON" => "\u{21B5}",       // icons.zsh:431 '↵' ↵
        "ROOT_ICON" => "\u{E614}",                  // icons.zsh:432 ''$q
        "SUDO_ICON" => "\u{F09C} ",                 // icons.zsh:433 ''$s
        "RUBY_ICON" => "\u{F219} ",                 // icons.zsh:434 ' '
        "AWS_ICON" => "\u{F270} ",                  // icons.zsh:435 ''$s
        "AWS_EB_ICON" => "\u{F1BD}",                // icons.zsh:436 '\UF1BD'$q$q
        "BACKGROUND_JOBS_ICON" => "\u{F013} ",      // icons.zsh:437 ' '
        "TEST_ICON" => "\u{F188} ",                 // icons.zsh:438 ''$s
        "TODO_ICON" => "\u{2611}",                  // icons.zsh:439 '☑' ☑
        "BATTERY_ICON" => "\u{F240} ",              // icons.zsh:440 '\UF240 '
        "DISK_ICON" => "\u{F0A0} ",                 // icons.zsh:441 ''$s
        "OK_ICON" => "\u{F00C} ",                   // icons.zsh:442 ''$s
        "FAIL_ICON" => "\u{F00D}",                  // icons.zsh:443 ''
        "SYMFONY_ICON" => "\u{E757}",               // icons.zsh:444 ''
        "NODE_ICON" => "\u{E617} ",                 // icons.zsh:445 ' '
        "NODEJS_ICON" => "\u{E617} ",               // icons.zsh:446 ' '
        "MULTILINE_FIRST_PROMPT_PREFIX" => "\u{256D}\u{2500}", // icons.zsh:447 '╭\U2500' ╭─
        "MULTILINE_NEWLINE_PROMPT_PREFIX" => "\u{251C}\u{2500}", // icons.zsh:448 '├\U2500' ├─
        "MULTILINE_LAST_PROMPT_PREFIX" => "\u{2570}\u{2500} ", // icons.zsh:449 '╰\U2500 ' ╰─
        "APPLE_ICON" => "\u{F179}",                 // icons.zsh:450 ''
        "WINDOWS_ICON" => "\u{F17A} ",              // icons.zsh:451 ''$s
        "FREEBSD_ICON" => "\u{F30C} ",              // icons.zsh:452 '\UF30C '
        "ANDROID_ICON" => "\u{F17B}",               // icons.zsh:453 ''
        "LINUX_ARCH_ICON" => "\u{F303}",            // icons.zsh:454 ''
        "LINUX_CENTOS_ICON" => "\u{F304} ",         // icons.zsh:455 ''$s
        "LINUX_COREOS_ICON" => "\u{F305} ",         // icons.zsh:456 ''$s
        "LINUX_DEBIAN_ICON" => "\u{F306}",          // icons.zsh:457 ''
        "LINUX_RASPBIAN_ICON" => "\u{F315}",        // icons.zsh:458 ''
        "LINUX_ELEMENTARY_ICON" => "\u{F309} ",     // icons.zsh:459 ''$s
        "LINUX_FEDORA_ICON" => "\u{F30A} ",         // icons.zsh:460 ''$s
        "LINUX_GENTOO_ICON" => "\u{F30D} ",         // icons.zsh:461 ''$s
        "LINUX_MAGEIA_ICON" => "\u{F310}",          // icons.zsh:462 ''
        "LINUX_MINT_ICON" => "\u{F30E} ",           // icons.zsh:463 ''$s
        "LINUX_NIXOS_ICON" => "\u{F313} ",          // icons.zsh:464 ''$s
        "LINUX_MANJARO_ICON" => "\u{F312} ",        // icons.zsh:465 ''$s
        "LINUX_DEVUAN_ICON" => "\u{F307} ",         // icons.zsh:466 ''$s
        "LINUX_ALPINE_ICON" => "\u{F300} ",         // icons.zsh:467 ''$s
        "LINUX_AOSC_ICON" => "\u{F301} ",           // icons.zsh:468 ''$s
        "LINUX_OPENSUSE_ICON" => "\u{F314} ",       // icons.zsh:469 ''$s
        "LINUX_SABAYON_ICON" => "\u{F317} ",        // icons.zsh:470 ''$s
        "LINUX_SLACKWARE_ICON" => "\u{F319} ",      // icons.zsh:471 ''$s
        "LINUX_VOID_ICON" => "\u{F17C}",            // icons.zsh:472 ''
        "LINUX_ARTIX_ICON" => "\u{F17C}",           // icons.zsh:473 ''
        "LINUX_UBUNTU_ICON" => "\u{F31B} ",         // icons.zsh:474 ''$s
        "LINUX_RHEL_ICON" => "\u{F316} ",           // icons.zsh:475 ''$s
        "LINUX_AMZN_ICON" => "\u{F270} ",           // icons.zsh:476 ''$s
        "LINUX_ICON" => "\u{F17C}",                 // icons.zsh:477 ''
        "SUNOS_ICON" => "\u{F185} ",                // icons.zsh:478 ' '
        "HOME_ICON" => "\u{F015} ",                 // icons.zsh:479 ''$s
        "HOME_SUB_ICON" => "\u{F07C} ",             // icons.zsh:480 ''$s
        "FOLDER_ICON" => "\u{F115} ",               // icons.zsh:481 ''$s
        "ETC_ICON" => "\u{F013} ",                  // icons.zsh:482 ''$s
        "NETWORK_ICON" => "\u{F50D} ",              // icons.zsh:483 ''$s
        "LOAD_ICON" => "\u{F080} ",                 // icons.zsh:484 ' '
        "SWAP_ICON" => "\u{F464} ",                 // icons.zsh:485 ''$s
        "RAM_ICON" => "\u{F0E4} ",                  // icons.zsh:486 ''$s
        "SERVER_ICON" => "\u{F0AE} ",               // icons.zsh:487 ''$s
        "VCS_UNTRACKED_ICON" => "\u{F059} ",        // icons.zsh:488 ''$s
        "VCS_UNSTAGED_ICON" => "\u{F06A} ",         // icons.zsh:489 ''$s
        "VCS_STAGED_ICON" => "\u{F055} ",           // icons.zsh:490 ''$s
        "VCS_STASH_ICON" => "\u{F01C} ",            // icons.zsh:491 ' '
        "VCS_INCOMING_CHANGES_ICON" => "\u{F01A} ", // icons.zsh:492 ' '
        "VCS_OUTGOING_CHANGES_ICON" => "\u{F01B} ", // icons.zsh:493 ' '
        "VCS_TAG_ICON" => "\u{F02B} ",              // icons.zsh:494 ' '
        "VCS_BOOKMARK_ICON" => "\u{F461} ",         // icons.zsh:495 ' '
        "VCS_COMMIT_ICON" => "\u{E729} ",           // icons.zsh:496 ' '
        "VCS_BRANCH_ICON" => "\u{F126} ",           // icons.zsh:497 ' '
        "VCS_REMOTE_BRANCH_ICON" => "\u{E728} ",    // icons.zsh:498 ' '
        "VCS_LOADING_ICON" => "",                   // icons.zsh:499 ''
        "VCS_GIT_ICON" => "\u{F1D3} ",              // icons.zsh:500 ' '
        "VCS_GIT_GITHUB_ICON" => "\u{F113} ",       // icons.zsh:501 ' '
        "VCS_GIT_BITBUCKET_ICON" => "\u{E703} ",    // icons.zsh:502 ' '
        "VCS_GIT_GITLAB_ICON" => "\u{F296} ",       // icons.zsh:503 ' '
        "VCS_GIT_AZURE_ICON" => "\u{FD03} ",       // icons.zsh:740
        "VCS_GIT_ARCHLINUX_ICON" => "\u{F303} ",   // icons.zsh:741
        "VCS_GIT_CODEBERG_ICON" => "\u{F1D3} ",    // icons.zsh:742
        "VCS_GIT_DEBIAN_ICON" => "\u{F306} ",      // icons.zsh:743
        "VCS_GIT_FREEBSD_ICON" => "\u{F30C} ",     // icons.zsh:744
        "VCS_GIT_FREEDESKTOP_ICON" => "\u{F296} ", // icons.zsh:745
        "VCS_GIT_GNOME_ICON" => "\u{F296} ",       // icons.zsh:746
        "VCS_GIT_GNU_ICON" => "\u{E779} ",         // icons.zsh:747
        "VCS_GIT_KDE_ICON" => "\u{F296} ",         // icons.zsh:748
        "VCS_GIT_LINUX_ICON" => "\u{F17C} ",       // icons.zsh:749
        "VCS_GIT_GITEA_ICON" => "\u{F1D3} ",       // icons.zsh:750
        "VCS_GIT_SOURCEHUT_ICON" => "\u{F1DB} ",   // icons.zsh:751
        "VCS_HG_ICON" => "\u{F0C3} ",               // icons.zsh:504 ' '
        "VCS_SVN_ICON" => "\u{E72D}",               // icons.zsh:505 ''$q
        "RUST_ICON" => "\u{E7A8}",                  // icons.zsh:506 ''$q
        "PYTHON_ICON" => "\u{E73C} ",               // icons.zsh:507 '\UE73C '
        "SWIFT_ICON" => "\u{E755}",                 // icons.zsh:508 ''
        "GO_ICON" => "\u{E626}",                    // icons.zsh:509 ''
        "GOLANG_ICON" => "\u{E626}",                // icons.zsh:510 ''
        "PUBLIC_IP_ICON" => "\u{F0AC} ",            // icons.zsh:511 '\UF0AC'$s
        "LOCK_ICON" => "\u{F023}",                  // icons.zsh:512 '\UF023'
        "NORDVPN_ICON" => "\u{F023}",               // icons.zsh:513 '\UF023'
        "EXECUTION_TIME_ICON" => "\u{F252} ",       // icons.zsh:514 ''$s
        "SSH_ICON" => "\u{F489} ",                  // icons.zsh:515 ''$s
        "VPN_ICON" => "\u{F023}",                   // icons.zsh:516 '\UF023'
        "KUBERNETES_ICON" => "\u{2388}",            // icons.zsh:517 '\U2388' ⎈
        "DROPBOX_ICON" => "\u{F16B} ",              // icons.zsh:518 '\UF16B'$s
        "DATE_ICON" => "\u{F073} ",                 // icons.zsh:519 ' '
        "TIME_ICON" => "\u{F017} ",                 // icons.zsh:520 ' '
        "JAVA_ICON" => "\u{E738}",                  // icons.zsh:521 ''
        "LARAVEL_ICON" => "\u{E73F}",               // icons.zsh:522 ''$q
        "RANGER_ICON" => "\u{F00B} ",               // icons.zsh:523 ' '
        "MIDNIGHT_COMMANDER_ICON" => "mc",          // icons.zsh:524 'mc'
        "VIM_ICON" => "\u{E62B}",                   // icons.zsh:525 ''
        "TERRAFORM_ICON" => "\u{F1BB} ",            // icons.zsh:526 ' '
        "PROXY_ICON" => "\u{2194}",                 // icons.zsh:527 '↔' ↔
        "DOTNET_ICON" => "\u{E77F}",                // icons.zsh:528 ''
        "DOTNET_CORE_ICON" => "\u{E77F}",           // icons.zsh:529 ''
        "AZURE_ICON" => "\u{FD03}",                 // icons.zsh:530 'ﴃ' ﴃ
        "DIRENV_ICON" => "\u{25BC}",                // icons.zsh:531 '▼' ▼
        "FLUTTER_ICON" => "F",                      // icons.zsh:532 'F'
        "GCLOUD_ICON" => "\u{F7B7}",                // icons.zsh:533 ''
        "LUA_ICON" => "\u{E620}",                   // icons.zsh:534 ''
        "PERL_ICON" => "\u{E769}",                  // icons.zsh:535 ''
        "NNN_ICON" => "nnn",                        // icons.zsh:536 'nnn'
        "XPLR_ICON" => "xplr",                      // icons.zsh:537 'xplr'
        "TIMEWARRIOR_ICON" => "\u{F49B}",           // icons.zsh:538 ''
        "TASKWARRIOR_ICON" => "\u{F4A0} ",          // icons.zsh:539 ' '
        "NIX_SHELL_ICON" => "\u{F313} ",            // icons.zsh:540 ' '
        "WIFI_ICON" => "\u{F1EB} ",                 // icons.zsh:541 ' '
        "ERLANG_ICON" => "\u{E7B1} ",               // icons.zsh:542 ' '
        "ELIXIR_ICON" => "\u{E62D}",                // icons.zsh:543 ''
        "POSTGRES_ICON" => "\u{E76E}",              // icons.zsh:544 ''
        "PHP_ICON" => "\u{E608}",                   // icons.zsh:545 ''
        "HASKELL_ICON" => "\u{E61F}",               // icons.zsh:546 ''
        "PACKAGE_ICON" => "\u{F8D6}",               // icons.zsh:547 ''
        "JULIA_ICON" => "\u{E624}",                 // icons.zsh:548 ''
        "SCALA_ICON" => "\u{E737}",                 // icons.zsh:549 ''
        "TOOLBOX_ICON" => "\u{E20F} ",              // icons.zsh:550 ''$s
        _ => return None,
    };
    Some(glyph)
}

/// icons.zsh:832 — the trailing-space strip runs only when
/// `POWERLEVEL9K_ICON_PADDING == none && POWERLEVEL9K_MODE != ascii`.
/// Mode is nerdfont-complete here (never ascii), so only the padding
/// param gates the transform. Exact string compare, matching zsh `==`.
fn icon_padding_none() -> bool {
    getsparam("POWERLEVEL9K_ICON_PADDING").as_deref() == Some("none")
}

/// icons.zsh:3-4 — `[[ -n ${POWERLEVEL9K_MODE-} || CODESET == utf8 ]] ||
/// MODE=ascii`: the effective mode name; "" selects the default table.
fn icon_mode() -> String {
    match getsparam("POWERLEVEL9K_MODE") {
        Some(m) if !m.is_empty() => m,
        _ if codeset_is_utf8() => String::new(),
        _ => "ascii".to_string(),
    }
}

/// `${langinfo[CODESET]} == (utf|UTF)(-|)8`.
fn codeset_is_utf8() -> bool {
    // SAFETY: nl_langinfo returns a pointer into static libc storage.
    let c = unsafe { std::ffi::CStr::from_ptr(libc::nl_langinfo(libc::CODESET)) };
    let name = c.to_string_lossy().to_ascii_lowercase();
    name == "utf-8" || name == "utf8"
}

/// `$name` → env/param value (the font maps awesome-mapped-fontconfig
/// reads are plain exported parameters).
fn codepoint_var(name: &str) -> String {
    getsparam(name).unwrap_or_default()
}

/// One icons.zsh value: a run of `'…'`, `"…${VAR:+…}…"` and `$s`/`$q`
/// words ending at an optional `#` comment. Returns the DECODED glyph
/// string (`(g::)`-style escapes resolved).
fn parse_icon_value(rest: &str, s: &str, q: &str) -> Option<String> {
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    let mut raw = String::new();
    while i < chars.len() {
        match chars[i] {
            '\'' => {
                let j = chars[i + 1..].iter().position(|&c| c == '\'')? + i + 1;
                raw.extend(&chars[i + 1..j]);
                i = j + 1;
            }
            '"' => {
                let j = chars[i + 1..].iter().position(|&c| c == '"')? + i + 1;
                let body: String = chars[i + 1..j].iter().collect();
                raw.push_str(&expand_dq(&body, s, q));
                i = j + 1;
            }
            '$' => {
                match chars.get(i + 1) {
                    Some('s') => raw.push_str(s),
                    Some('q') => raw.push_str(q),
                    _ => return None,
                }
                i += 2;
            }
            '#' => break,
            c if c.is_whitespace() => i += 1,
            _ => return None,
        }
    }
    Some(unescape_g(&raw))
}

/// The double-quoted form icons.zsh uses for awesome-mapped-fontconfig:
/// `${CODEPOINT_OF_X:+\\u$CODEPOINT_OF_X$s}` — empty unless the font
/// map variable is set.
fn expand_dq(body: &str, s: &str, q: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else { break };
        let inner = &after[..end];
        if let Some((name, tail)) = inner.split_once(":+") {
            let val = codepoint_var(name);
            if !val.is_empty() {
                let tail = tail
                    .replace("\\\\", "\\")
                    .replace(&format!("${name}"), &val)
                    .replace("$s", s)
                    .replace("$q", q);
                out.push_str(&tail);
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `[start, end)` line range of the branch of the `case $POWERLEVEL9K_MODE
/// in` starting at line `case_at` that selects `mode`; the `*)` branch
/// when none names it.
fn mode_branch(lines: &[&str], case_at: usize, mode: &str) -> Option<(usize, usize)> {
    let end = case_at + lines[case_at..].iter().position(|l| l.trim() == "esac")?;
    let mut chosen = None;
    let mut default = None;
    let mut i = case_at + 1;
    while i < end {
        let l = lines[i];
        if l.starts_with("    ") && !l.starts_with("     ") && l.trim_end().ends_with(')') {
            let mut j = i + 1;
            while j < end && lines[j].trim() != ";;" {
                j += 1;
            }
            let pats = l.trim().trim_end_matches(')');
            if pats.trim() == "*" {
                default = Some((i + 1, j));
            } else if pats.split('|').any(|p| p.trim().trim_matches('\'') == mode) {
                chosen = Some((i + 1, j));
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    chosen.or(default)
}

/// Read the active mode's table out of the theme's icons.zsh, apply the
/// flat/compatible overrides and the ICON_PADDING=none post-processing
/// (icons.zsh:1115-1142).
fn load_theme_icons(mode: &str, legacy: bool, padding_none: bool) -> Option<std::collections::HashMap<String, String>> {
    let root = crate::p10k::p10k_root_dir()?;
    let text = std::fs::read_to_string(format!("{root}/internal/icons.zsh")).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let (s, q) = if legacy { ("", " ") } else { (" ", "") }; // icons.zsh:8-14
    let cases: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == "case $POWERLEVEL9K_MODE in")
        .map(|(i, _)| i)
        .collect();
    let (a, b) = mode_branch(&lines, *cases.first()?, mode)?;
    let mut map = std::collections::HashMap::new();
    let mut in_table = false;
    for l in &lines[a..b] {
        let t = l.trim();
        if t.starts_with("icons=(") {
            in_table = true;
        } else if in_table && t == ")" {
            break;
        } else if in_table && !t.is_empty() && !t.starts_with('#') {
            let Some((key, rest)) = t.split_once(char::is_whitespace) else {
                continue;
            };
            if let Some(v) = parse_icon_value(rest, s, q) {
                map.insert(key.to_string(), v);
            }
        }
    }
    if map.is_empty() {
        return None;
    }
    // icons.zsh:1115-1129 — second `case`: the flat / compatible overrides.
    if let Some(&second) = cases.get(1) {
        if let Some((a, b)) = mode_branch(&lines, second, mode) {
            for l in &lines[a..b] {
                let t = l.trim();
                if let Some(body) = t.strip_prefix("icons[") {
                    if let Some((key, val)) = body.split_once("]=") {
                        if let Some(v) = parse_icon_value(val, s, q) {
                            map.insert(key.to_string(), v);
                        }
                    }
                }
            }
        }
    }
    // icons.zsh:1131-1142 — ICON_PADDING=none (not ascii): strip trailing
    // spaces everywhere, then give seven keys one back.
    if padding_none && mode != "ascii" {
        for v in map.values_mut() {
            *v = v.trim_end_matches(' ').to_string();
        }
        for key in PADDED_KEYS {
            map.entry(key.to_string()).or_default().push(' ');
        }
    }
    Some(map)
}

/// The theme-derived table for the current mode/spacing/padding, parsed
/// once per combination (leaked: a handful of tables per process).
fn theme_icons() -> Option<&'static std::collections::HashMap<String, String>> {
    use std::sync::{Mutex, OnceLock};
    type Cache = Mutex<std::collections::HashMap<String, Option<&'static std::collections::HashMap<String, String>>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let mode = icon_mode();
    let legacy = getsparam("POWERLEVEL9K_LEGACY_ICON_SPACING").as_deref() == Some("true");
    let padding_none = icon_padding_none();
    let sig = format!("{mode}/{legacy}/{padding_none}");
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(&sig).copied()) {
        return hit;
    }
    let table = load_theme_icons(&mode, legacy, padding_none).map(|m| &*Box::leak(Box::new(m)));
    if table.is_none() {
        tracing::debug!("p10k icons: cannot read internal/icons.zsh for mode {mode:?}");
    }
    if let Ok(mut c) = cache.lock() {
        c.insert(sig, table);
    }
    table
}

/// Table lookup by icon key (icons.zsh tables incl. the ICON_PADDING=none
/// post-processing). Returns "" for unknown keys.
pub fn icon(name: &str) -> &'static str {
    let mode = icon_mode();
    let legacy = getsparam("POWERLEVEL9K_LEGACY_ICON_SPACING").as_deref() == Some("true");
    let baked = matches!(mode.as_str(), "nerdfont-complete" | "nerdfont-fontconfig")
        && (!legacy || icon_padding_none());
    if !baked {
        if let Some(table) = theme_icons() {
            return match table.get(name) {
                Some(v) => v.as_str(),
                None => {
                    tracing::debug!("p10k icons: unknown icon key {name}");
                    ""
                }
            };
        }
    }
    baked_icon(name)
}

/// The baked nerdfont-complete table with the ICON_PADDING=none
/// post-processing applied (icons.zsh:1131-1142).
fn baked_icon(name: &str) -> &'static str {
    let raw = match icon_raw(name) {
        Some(r) => r,
        None => {
            tracing::debug!("p10k icons: unknown icon key {name}");
            return "";
        }
    };
    if !icon_padding_none() {
        return raw; // padding != none — table value kept verbatim
    }
    if PADDED_KEYS.contains(&name) {
        // `%% #` strip then `+=' '`; each of these keys has exactly one
        // trailing space in the raw table, so the raw literal IS the
        // post-transform value.
        raw
    } else {
        // icons=("${(@kv)icons%% #}") strips every trailing space run.
        raw.trim_end_matches(' ')
    }
}

/// p10k.zsh:510-530 — _p9k_get_icon: the user parameter
/// POWERLEVEL9K_<NAME> overrides the mode table. Segment-scoped overrides
/// (POWERLEVEL9K_<SEGMENT>_<NAME>) are config.rs::p9k_param territory;
/// this resolves the global tier only, mirroring print_icon
/// (icons.zsh:845-854: `(( $+parameters[$var] ))` then `${(P)var}`).
pub fn icon_resolved(name: &str) -> String {
    if let Some(user) = getsparam(&format!("POWERLEVEL9K_{name}")) {
        // p10k.zsh:524 — _p9k__ret=${(g::)_p9k__ret}: user-supplied values
        // get echo-style escape processing (table defaults arrive as real
        // glyphs and skip this via the \1 marker path, p10k.zsh:521-522).
        let decoded = unescape_g(&user);
        // p10k.zsh:525 — [[ $_p9k__ret != $'\b'? ]] || _p9k__ret="%{$_p9k__ret%}"
        // "penance for past sins": a backspace followed by exactly one char
        // gets wrapped in zero-width %{...%} markers.
        let mut chars = decoded.chars();
        if chars.next() == Some('\u{8}') && chars.next().is_some() && chars.next().is_none() {
            return format!("%{{{decoded}%}}");
        }
        return decoded;
    }
    icon(name).to_string()
}

/// Rust-side equivalent of zsh `${(g::)...}` (p10k.zsh:524) — echo-style
/// backslash escapes (zsh getkeystring, Src/utils.c GETKEY semantics for
/// the plain `g::` flag set). Unrecognized/malformed escapes pass through
/// literally, matching zsh's lenient behavior.
fn unescape_g(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('a') => out.push('\u{7}'),
            Some('b') => out.push('\u{8}'),
            Some('e') | Some('E') => out.push('\u{1B}'),
            Some('f') => out.push('\u{C}'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('v') => out.push('\u{B}'),
            Some('\\') => out.push('\\'),
            Some('x') => push_hex(&mut out, &mut it, 2, "\\x"),
            Some('u') => push_hex(&mut out, &mut it, 4, "\\u"),
            Some('U') => push_hex(&mut out, &mut it, 8, "\\U"),
            Some('0') => {
                // echo-style \0NNN octal (up to 3 digits after the 0)
                let mut val: u32 = 0;
                let mut n = 0;
                while n < 3 {
                    match it.peek() {
                        Some(d @ '0'..='7') => {
                            val = val * 8 + (*d as u32 - '0' as u32);
                            it.next();
                            n += 1;
                        }
                        _ => break,
                    }
                }
                out.push(char::from_u32(val).unwrap_or('\u{0}'));
            }
            Some(other) => {
                // unknown escape — keep verbatim
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Consume up to `max` hex digits and push the decoded char; on zero digits
/// or an invalid codepoint, emit the escape introducer literally.
fn push_hex(
    out: &mut String,
    it: &mut std::iter::Peekable<std::str::Chars<'_>>,
    max: usize,
    intro: &str,
) {
    let mut val: u32 = 0;
    let mut n = 0;
    while n < max {
        match it.peek().and_then(|d| d.to_digit(16)) {
            Some(d) => {
                val = val * 16 + d;
                it.next();
                n += 1;
            }
            None => break,
        }
    }
    if n == 0 {
        out.push_str(intro);
    } else {
        match char::from_u32(val) {
            Some(ch) => out.push(ch),
            None => out.push_str(intro),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // icons.zsh:497 + :839 — one trailing space survives padding=none
    #[test]
    fn branch_icon_raw_has_single_trailing_space() {
        assert_eq!(icon_raw("VCS_BRANCH_ICON"), Some("\u{F126} "));
    }

    #[test]
    fn unknown_key_is_empty() {
        assert_eq!(icon("NO_SUCH_ICON_KEY"), "");
    }

    #[test]
    fn unescape_g_handles_unicode_and_octal() {
        assert_eq!(unescape_g("\\uE0B0"), "\u{E0B0}");
        assert_eq!(unescape_g("\\U0001F331"), "\u{1F331}");
        assert_eq!(unescape_g("\\x41"), "A");
        assert_eq!(unescape_g("\\0101"), "A");
        assert_eq!(unescape_g("plain"), "plain");
        assert_eq!(unescape_g("\\q"), "\\q"); // unknown escape kept verbatim
    }

    // every PADDED_KEYS entry must end in exactly one space in the raw
    // table, otherwise the strip+append identity in icon() is wrong
    #[test]
    fn padded_keys_end_in_exactly_one_space() {
        for key in PADDED_KEYS {
            let raw = icon_raw(key).unwrap_or_default();
            assert!(raw.ends_with(' '), "{key} missing trailing space");
            assert!(!raw.ends_with("  "), "{key} has multiple trailing spaces");
        }
    }
}
