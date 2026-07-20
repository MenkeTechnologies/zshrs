//! powerlevel10k icon table — port of internal/icons.zsh, nerdfont-complete mode.
//!
//! Spec: /Users/wizard/forkedRepos/powerlevel10k/internal/icons.zsh
//!       (nerdfont-complete branch, icons.zsh:420-551, plus the
//!       ICON_PADDING post-processing at icons.zsh:832-841) and
//!       internal/p10k.zsh:510-530 (_p9k_get_icon override order).
//!
//! icons.zsh:4 — mode defaults to ascii only when POWERLEVEL9K_MODE is unset
//! AND the locale is not UTF-8; the user's config pins MODE=nerdfont-complete,
//! which is the only mode table ported here. If POWERLEVEL9K_MODE names a
//! different mode we still serve the nerdfont-complete table (logged once per
//! miss at debug level) — other mode tables are out of scope.
//!
//! icons.zsh:8-14 — LEGACY_ICON_SPACING swaps the $s/$q suffixes:
//!   non-legacy (default): s=' '  q=''
//!   legacy:               s=''   q=' '
//! The table below is baked with the NON-legacy expansion; each entry that
//! used $s or $q in the zsh source carries a `$s`/`$q` note in its comment.
//! Under POWERLEVEL9K_ICON_PADDING=none (the user's setting, icons.zsh:832
//! strips ALL trailing spaces) legacy and non-legacy collapse to identical
//! values, so legacy spacing only diverges when padding != none — that
//! combination is logged and served with non-legacy values.

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

/// Table lookup by icon key with the ICON_PADDING=none post-processing
/// applied (icons.zsh:832-841). Returns "" for unknown keys.
pub fn icon(name: &str) -> &'static str {
    let raw = match icon_raw(name) {
        Some(r) => r,
        None => {
            tracing::debug!("p10k icons: unknown icon key {name}");
            return "";
        }
    };
    // icons.zsh:8-14 — legacy spacing only diverges from this table when
    // padding != none (see module doc); surface that unported combination.
    if !icon_padding_none() {
        if getsparam("POWERLEVEL9K_LEGACY_ICON_SPACING").as_deref() == Some("true") {
            tracing::debug!(
                "p10k icons: LEGACY_ICON_SPACING=true with ICON_PADDING!=none not ported; using default spacing for {name}"
            );
        }
        return raw; // padding != none — table value kept verbatim
    }
    if PADDED_KEYS.contains(&name) {
        // icons.zsh:833-840 — `%% #` strip then `+=' '`; each of these keys
        // has exactly one trailing space in the raw table, so the raw
        // literal IS the post-transform value.
        raw
    } else {
        // icons.zsh:833 — icons=("${(@kv)icons%% #}") strips every trailing
        // space run. A subslice of a &'static str stays 'static.
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
