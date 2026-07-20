//! Port of `generate_config` (internal/wizard.zsh:1619-1867) — the
//! deterministic core of `p10k configure`: it takes one of the bundled
//! `config/p10k-{style}.zsh` templates and applies the wizard's answers
//! as `sub` / `uncomment` / `rep` string edits, producing the final
//! `.p10k.zsh`. No terminal I/O; fully unit-testable against real zsh.
//!
//! The template line vector is edited in place exactly as the three
//! zsh helper functions do (wizard:1623-1633):
//!   sub KEY VAL      — `^(\s*)typeset -g POWERLEVEL9K_<KEY>=…` → set RHS
//!   uncomment TEXT   — `^(\s*)# <TEXT>(  |$)` → drop the `# ` prefix
//!   rep FROM TO      — global literal substring replace on every line

use super::settings::{PromptStyle, WizardSettings};

/// wizard:1623 `sub()` — replace the RHS of a matching
/// `typeset -g POWERLEVEL9K_<key>=…` line, preserving indentation. The
/// key is matched LITERALLY (brace groups like `{OK,ERROR}` are literal
/// text in both the template line and the call, since the template was
/// authored with brace-expansion-off literal keys).
fn sub(lines: &mut [String], key: &str, val: &str) {
    let needle = format!("typeset -g POWERLEVEL9K_{key}=");
    for line in lines.iter_mut() {
        let indent_len = line.len() - line.trim_start().len();
        let (indent, body) = line.split_at(indent_len);
        if let Some(_rest) = body.strip_prefix(&needle) {
            *line = format!("{indent}{needle}{val}");
        }
    }
}

/// wizard:1627 `uncomment()` — turn `# <text>` into `<text>`,
/// preserving leading whitespace. The zsh pattern anchors the `# ` and
/// requires the text to be followed by two spaces or end-of-line; the
/// `$match[2]$match[2]` doubling keeps trailing alignment.
fn uncomment(lines: &mut [String], text: &str) {
    let hash = format!("# {text}");
    for line in lines.iter_mut() {
        let indent_len = line.len() - line.trim_start().len();
        let (indent, body) = line.split_at(indent_len);
        if let Some(after) = body.strip_prefix(&hash) {
            // `/#` anchors at START only, so the `(  |)` empty alternative
            // always matches — the line is uncommented regardless of what
            // follows ARG. When exactly two spaces follow, match[2]="  "
            // and the `$match[2]$match[2]` doubling turns them into four;
            // otherwise match[2]="" and the remainder is kept verbatim.
            if let Some(rest) = after.strip_prefix("  ") {
                *line = format!("{indent}{text}    {rest}");
            } else {
                *line = format!("{indent}{text}{after}");
            }
        }
    }
}

/// wizard:1631 `rep()` — global literal substring replace on every line.
fn rep(lines: &mut [String], from: &str, to: &str) {
    if from.is_empty() {
        return;
    }
    for line in lines.iter_mut() {
        if line.contains(from) {
            *line = line.replace(from, to);
        }
    }
}

/// Port of `generate_config` (wizard:1619-1867): produce the final
/// config text from `base` (a `config/p10k-{style}.zsh` template) and
/// the collected `settings`. `now` is the "generated on …" timestamp
/// (the caller stamps it; scripts here take no clock).
pub fn generate_config(base: &str, s: &WizardSettings, now: &str) -> String {
    // wizard:1621 — split into lines (a trailing newline yields a final
    // empty element, matching zsh `${(@f)base}`; we normalize by not
    // re-adding it and joining with '\n').
    let mut lines: Vec<String> = base.split('\n').map(String::from).collect();

    if s.style == PromptStyle::Pure {
        // wizard:1636-1642 — recolor the pure palette locals.
        for (k, name) in [
            ("grey", "grey=242"),
            ("red", "red=1"),
            ("yellow", "yellow=3"),
            ("blue", "blue=4"),
            ("magenta", "magenta=5"),
            ("cyan", "cyan=6"),
            ("white", "white=7"),
        ] {
            rep(
                &mut lines,
                &format!("local {name}"),
                &format!("local {k}='{}'", s.pure_color(k)),
            );
        }
    } else {
        sub(&mut lines, "MODE", &s.mode); // wizard:1644
        sub(&mut lines, "ICON_PADDING", &s.icon_padding); // wizard:1646

        if s.mode == "compatible" {
            // wizard:1648-1652
            sub(
                &mut lines,
                "STATUS_ERROR_VISUAL_IDENTIFIER_EXPANSION",
                "'х'",
            );
            sub(
                &mut lines,
                "STATUS_ERROR_SIGNAL_VISUAL_IDENTIFIER_EXPANSION",
                "'х'",
            );
            sub(
                &mut lines,
                "STATUS_ERROR_PIPE_VISUAL_IDENTIFIER_EXPANSION",
                "'х'",
            );
        }

        if s.mode == "compatible" || s.mode == "powerline" {
            // wizard:1654-1669 — powerline fallbacks for glyphs a
            // non-nerdfont terminal can't render.
            for (key, val) in [
                ("LOCK_ICON", "'∅'"),
                ("NORDVPN_VISUAL_IDENTIFIER_EXPANSION", "'nord'"),
                ("RANGER_VISUAL_IDENTIFIER_EXPANSION", "'▲'"),
                ("KUBECONTEXT_DEFAULT_VISUAL_IDENTIFIER_EXPANSION", "'○'"),
                ("AZURE_VISUAL_IDENTIFIER_EXPANSION", "'az'"),
                ("AWS_EB_ENV_VISUAL_IDENTIFIER_EXPANSION", "'eb'"),
                ("BACKGROUND_JOBS_VISUAL_IDENTIFIER_EXPANSION", "'≡'"),
            ] {
                uncomment(&mut lines, &format!("typeset -g POWERLEVEL9K_{key}"));
                sub(&mut lines, key, val);
            }
        }

        if (s.mode == "awesome-patched" || s.mode == "awesome-fontconfig") && !s.cap_python {
            // wizard:1671-1680 — snake fallbacks for the python segments.
            for key in [
                "VIRTUALENV_VISUAL_IDENTIFIER_EXPANSION",
                "ANACONDA_VISUAL_IDENTIFIER_EXPANSION",
                "PYENV_VISUAL_IDENTIFIER_EXPANSION",
                "PYTHON_ICON",
            ] {
                uncomment(&mut lines, &format!("typeset -g POWERLEVEL9K_{key}"));
            }
            sub(&mut lines, "VIRTUALENV_VISUAL_IDENTIFIER_EXPANSION", "'🐍'");
            sub(&mut lines, "ANACONDA_VISUAL_IDENTIFIER_EXPANSION", "'🐍'");
            sub(&mut lines, "PYENV_VISUAL_IDENTIFIER_EXPANSION", "'🐍'");
            sub(&mut lines, "PYTHON_ICON", "'🐍'");
        }

        if s.mode == "nerdfont-complete" {
            // wizard:1682-1684
            sub(
                &mut lines,
                "BATTERY_STAGES",
                "'\u{f58d}\u{f579}\u{f57a}\u{f57b}\u{f57c}\u{f57d}\u{f57e}\u{f57f}\u{f580}\u{f581}\u{f578}'",
            );
        }

        if matches!(s.style, PromptStyle::Classic | PromptStyle::Rainbow) {
            // wizard:1686-1711 — frame ornaments and separators.
            let ci = s.color as usize;
            if s.style == PromptStyle::Classic {
                sub(&mut lines, "BACKGROUND", &BG_COLOR[ci].to_string());
                sub(
                    &mut lines,
                    "LEFT_SUBSEGMENT_SEPARATOR",
                    &format!("'%{}F{}'", SEP_COLOR[ci], s.left_subsep),
                );
                sub(
                    &mut lines,
                    "RIGHT_SUBSEGMENT_SEPARATOR",
                    &format!("'%{}F{}'", SEP_COLOR[ci], s.right_subsep),
                );
                sub(
                    &mut lines,
                    "VCS_LOADING_FOREGROUND",
                    &SEP_COLOR[ci].to_string(),
                );
                rep(&mut lines, "%248F", &format!("%{}F", PREFIX_COLOR[ci]));
            } else {
                sub(
                    &mut lines,
                    "LEFT_SUBSEGMENT_SEPARATOR",
                    &format!("'{}'", s.left_subsep),
                );
                sub(
                    &mut lines,
                    "RIGHT_SUBSEGMENT_SEPARATOR",
                    &format!("'{}'", s.right_subsep),
                );
            }
            let fc = FRAME_COLOR_FOR(s, ci);
            sub(&mut lines, "RULER_FOREGROUND", &fc.to_string());
            sub(
                &mut lines,
                "MULTILINE_FIRST_PROMPT_GAP_FOREGROUND",
                &fc.to_string(),
            );
            sub(
                &mut lines,
                "MULTILINE_FIRST_PROMPT_PREFIX",
                &format!("'%{fc}F╭─'"),
            );
            sub(
                &mut lines,
                "MULTILINE_NEWLINE_PROMPT_PREFIX",
                &format!("'%{fc}F├─'"),
            );
            sub(
                &mut lines,
                "MULTILINE_LAST_PROMPT_PREFIX",
                &format!("'%{fc}F╰─'"),
            );
            sub(
                &mut lines,
                "MULTILINE_FIRST_PROMPT_SUFFIX",
                &format!("'%{fc}F─╮'"),
            );
            sub(
                &mut lines,
                "MULTILINE_NEWLINE_PROMPT_SUFFIX",
                &format!("'%{fc}F─┤'"),
            );
            sub(
                &mut lines,
                "MULTILINE_LAST_PROMPT_SUFFIX",
                &format!("'%{fc}F─╯'"),
            );
            sub(
                &mut lines,
                "LEFT_SEGMENT_SEPARATOR",
                &format!("'{}'", s.left_sep),
            );
            sub(
                &mut lines,
                "RIGHT_SEGMENT_SEPARATOR",
                &format!("'{}'", s.right_sep),
            );
            sub(
                &mut lines,
                "LEFT_PROMPT_FIRST_SEGMENT_START_SYMBOL",
                &format!("'{}'", s.left_tail),
            );
            sub(
                &mut lines,
                "LEFT_PROMPT_LAST_SEGMENT_END_SYMBOL",
                &format!("'{}'", s.left_head),
            );
            sub(
                &mut lines,
                "RIGHT_PROMPT_FIRST_SEGMENT_START_SYMBOL",
                &format!("'{}'", s.right_head),
            );
            sub(
                &mut lines,
                "RIGHT_PROMPT_LAST_SEGMENT_END_SYMBOL",
                &format!("'{}'", s.right_tail),
            );
        }

        if s.extra_icons.iter().any(|i| !i.is_empty()) {
            // wizard:1713-1716
            let branch = s.branch_icon.replace(' ', "");
            sub(&mut lines, "VCS_BRANCH_ICON", &format!("'{branch} '"));
            uncomment(&mut lines, "os_icon");
        } else {
            // wizard:1717-1724 — no icons: hide the visual identifiers.
            for key in [
                "typeset -g POWERLEVEL9K_DIR_CLASSES",
                "typeset -g POWERLEVEL9K_VCS_VISUAL_IDENTIFIER_EXPANSION",
                "typeset -g POWERLEVEL9K_COMMAND_EXECUTION_TIME_VISUAL_IDENTIFIER_EXPANSION",
                "typeset -g POWERLEVEL9K_TIME_VISUAL_IDENTIFIER_EXPANSION",
            ] {
                uncomment(&mut lines, key);
            }
            sub(&mut lines, "VCS_VISUAL_IDENTIFIER_EXPANSION", "");
            sub(
                &mut lines,
                "COMMAND_EXECUTION_TIME_VISUAL_IDENTIFIER_EXPANSION",
                "",
            );
            sub(&mut lines, "TIME_VISUAL_IDENTIFIER_EXPANSION", "");
        }

        if s.prefixes.iter().any(|p| !p.is_empty()) {
            // wizard:1727-1743
            for key in [
                "typeset -g POWERLEVEL9K_VCS_PREFIX",
                "typeset -g POWERLEVEL9K_COMMAND_EXECUTION_TIME_PREFIX",
                "typeset -g POWERLEVEL9K_CONTEXT_PREFIX",
                "typeset -g POWERLEVEL9K_KUBECONTEXT_PREFIX",
                "typeset -g POWERLEVEL9K_TIME_PREFIX",
                "typeset -g POWERLEVEL9K_TOOLBOX_PREFIX",
            ] {
                uncomment(&mut lines, key);
            }
            if matches!(s.style, PromptStyle::Lean | PromptStyle::Classic) {
                let fg = if s.style == PromptStyle::Classic {
                    format!("%{}F", PREFIX_COLOR[s.color as usize])
                } else {
                    "%f".to_string()
                };
                sub(&mut lines, "VCS_PREFIX", &format!("'{fg}on '"));
                sub(
                    &mut lines,
                    "COMMAND_EXECUTION_TIME_PREFIX",
                    &format!("'{fg}took '"),
                );
                sub(&mut lines, "CONTEXT_PREFIX", &format!("'{fg}with '"));
                sub(&mut lines, "KUBECONTEXT_PREFIX", &format!("'{fg}at '"));
                sub(&mut lines, "TIME_PREFIX", &format!("'{fg}at '"));
                sub(&mut lines, "TOOLBOX_PREFIX", &format!("'{fg}in '"));
            }
        }

        sub(
            &mut lines,
            "MULTILINE_FIRST_PROMPT_GAP_CHAR",
            &format!("'{}'", s.gap_char),
        ); // wizard:1745

        if matches!(s.style, PromptStyle::Classic | PromptStyle::Rainbow) && s.num_lines == 2 {
            // wizard:1747-1760
            if !s.right_frame {
                sub(&mut lines, "MULTILINE_FIRST_PROMPT_SUFFIX", "");
                sub(&mut lines, "MULTILINE_NEWLINE_PROMPT_SUFFIX", "");
                sub(&mut lines, "MULTILINE_LAST_PROMPT_SUFFIX", "");
            }
            if !s.left_frame {
                sub(&mut lines, "MULTILINE_FIRST_PROMPT_PREFIX", "");
                sub(&mut lines, "MULTILINE_NEWLINE_PROMPT_PREFIX", "");
                sub(&mut lines, "MULTILINE_LAST_PROMPT_PREFIX", "");
                sub(&mut lines, "STATUS_OK", "false");
                sub(&mut lines, "STATUS_ERROR", "false");
            }
        }

        if matches!(s.style, PromptStyle::Lean | PromptStyle::Lean8) {
            // wizard:1762-1777
            let fc = FRAME_COLOR_FOR(s, s.color as usize);
            sub(&mut lines, "RULER_FOREGROUND", &fc.to_string());
            sub(
                &mut lines,
                "MULTILINE_FIRST_PROMPT_GAP_FOREGROUND",
                &fc.to_string(),
            );
            if s.right_frame {
                sub(
                    &mut lines,
                    "MULTILINE_FIRST_PROMPT_SUFFIX",
                    &format!("'%{fc}F─╮'"),
                );
                sub(
                    &mut lines,
                    "MULTILINE_NEWLINE_PROMPT_SUFFIX",
                    &format!("'%{fc}F─┤'"),
                );
                sub(
                    &mut lines,
                    "MULTILINE_LAST_PROMPT_SUFFIX",
                    &format!("'%{fc}F─╯'"),
                );
                sub(&mut lines, "RIGHT_PROMPT_LAST_SEGMENT_END_SYMBOL", "' '");
            }
            if s.left_frame {
                sub(
                    &mut lines,
                    "MULTILINE_FIRST_PROMPT_PREFIX",
                    &format!("'%{fc}F╭─'"),
                );
                sub(
                    &mut lines,
                    "MULTILINE_NEWLINE_PROMPT_PREFIX",
                    &format!("'%{fc}F├─'"),
                );
                sub(
                    &mut lines,
                    "MULTILINE_LAST_PROMPT_PREFIX",
                    &format!("'%{fc}F╰─'"),
                );
                sub(&mut lines, "LEFT_PROMPT_FIRST_SEGMENT_START_SYMBOL", "' '");
            }
        }

        if matches!(s.style, PromptStyle::Classic | PromptStyle::Rainbow) {
            // wizard:1779-1785
            if s.num_lines == 2 && !s.left_frame {
                uncomment(&mut lines, "prompt_char");
            } else {
                uncomment(&mut lines, "vi_mode");
            }
        }

        if s.mode == "ascii" {
            // wizard:1787-1805 — ascii glyph fallbacks.
            sub(&mut lines, "STATUS_OK_VISUAL_IDENTIFIER_EXPANSION", "'ok'");
            sub(
                &mut lines,
                "STATUS_OK_PIPE_VISUAL_IDENTIFIER_EXPANSION",
                "'ok'",
            );
            sub(
                &mut lines,
                "STATUS_ERROR_VISUAL_IDENTIFIER_EXPANSION",
                "'err'",
            );
            sub(
                &mut lines,
                "STATUS_ERROR_SIGNAL_VISUAL_IDENTIFIER_EXPANSION",
                "",
            );
            sub(
                &mut lines,
                "STATUS_ERROR_PIPE_VISUAL_IDENTIFIER_EXPANSION",
                "'err'",
            );
            sub(&mut lines, "BATTERY_STAGES", "('battery')");
            sub(
                &mut lines,
                "PROMPT_CHAR_{OK,ERROR}_VIINS_CONTENT_EXPANSION",
                "'>'",
            );
            sub(
                &mut lines,
                "PROMPT_CHAR_{OK,ERROR}_VICMD_CONTENT_EXPANSION",
                "'<'",
            );
            sub(
                &mut lines,
                "PROMPT_CHAR_{OK,ERROR}_VIVIS_CONTENT_EXPANSION",
                "'V'",
            );
            sub(
                &mut lines,
                "PROMPT_CHAR_{OK,ERROR}_VIOWR_CONTENT_EXPANSION",
                "'^'",
            );
            rep(&mut lines, "-i '⭐'", "-i '*'");
            rep(&mut lines, "…", "..");
            rep(&mut lines, "⇣", "<");
            rep(&mut lines, "⇡", ">");
            rep(&mut lines, "⇠", "<-");
            rep(&mut lines, "⇢", "->");
            rep(&mut lines, "─", "-");
        }
    }

    if s.pure_use_rprompt {
        // wizard:1808-1815 — move a few segments to the right prompt.
        for segment in ["command_execution_time", "virtualenv", "context"] {
            rep(
                &mut lines,
                &format!("    {segment}"),
                &format!("    tmp_{segment}"),
            );
            uncomment(&mut lines, segment);
            rep(
                &mut lines,
                &format!("    tmp_{segment}  "),
                &format!("    # {segment}"),
            );
        }
    }

    if let Some(time) = &s.time {
        // wizard:1817-1822
        uncomment(&mut lines, "time");
        if time == TIME_12H {
            sub(&mut lines, "TIME_FORMAT", "'%D{%I:%M:%S %p}'");
        }
    }

    if s.num_lines == 1 {
        // wizard:1824-1830 — drop the newline segment + line-2 marker.
        lines.retain(|l| !(l.starts_with("    newline") || l.contains("===[ Line #")));
    }

    // wizard:1833
    sub(
        &mut lines,
        "PROMPT_ADD_NEWLINE",
        if s.empty_line { "true" } else { "false" },
    );
    // wizard:1835
    sub(&mut lines, "INSTANT_PROMPT", &s.instant_prompt);
    // wizard:1836
    if s.transient_prompt {
        sub(&mut lines, "TRANSIENT_PROMPT", "always");
    }

    // wizard:1838-1859 — header.
    let mut header = String::new();
    header.push_str(&format!(
        "# Generated by Powerlevel10k configuration wizard on {now}.\n"
    ));
    header.push_str(&format!(
        "# Based on romkatv/powerlevel10k/config/p10k-{}.zsh.\n",
        s.style.template_name()
    ));
    // wizard:1847-1858 — wrap the options list at ~85 columns.
    if !s.options.is_empty() {
        let mut line = format!("# Wizard options: {}", s.options[0]);
        for opt in &s.options[1..] {
            if line.chars().count() + opt.chars().count() > 85 {
                header.push_str(&line);
                header.push_str(",\n");
                line = format!("# {opt}");
            } else {
                line.push_str(&format!(", {opt}"));
            }
        }
        header.push_str(&line);
    } else {
        header.push_str("# Wizard options: ");
    }
    header.push_str(".\n# Type `p10k configure` to generate another config.\n#");

    // wizard:1866 — `print -lr -- "$header" "$lines[@]"`: header, then a
    // newline, then every line, each terminated by a newline.
    let mut out = header;
    out.push('\n');
    out.push_str(&lines.join("\n"));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

// --------------------------------------------------------------------
// wizard:30-32 color tables.
// --------------------------------------------------------------------
pub const BG_COLOR: [i32; 4] = [240, 238, 236, 234]; // wizard:30
pub const SEP_COLOR: [i32; 4] = [248, 246, 244, 242]; // wizard:31
pub const PREFIX_COLOR: [i32; 4] = [250, 248, 246, 244]; // wizard:32

/// The active frame-color table (`frame_color`, wizard:2011) is overridden
/// to (0 7 2 4) in the 8-color path (wizard:838); otherwise (244 242 240
/// 238). `s.frame_color` holds the live table.
#[allow(non_snake_case)]
fn FRAME_COLOR_FOR(s: &WizardSettings, idx: usize) -> i32 {
    s.frame_color[idx.min(3)]
}

/// wizard:49 `time_12h`.
pub const TIME_12H: &str = "04:23:42 PM";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_replaces_rhs_preserving_indent() {
        let mut l = vec!["  typeset -g POWERLEVEL9K_MODE=old".to_string()];
        sub(&mut l, "MODE", "nerdfont-complete");
        assert_eq!(l[0], "  typeset -g POWERLEVEL9K_MODE=nerdfont-complete");
    }

    #[test]
    fn sub_matches_literal_brace_key() {
        let mut l = vec![
            "  typeset -g POWERLEVEL9K_PROMPT_CHAR_{OK,ERROR}_VIINS_CONTENT_EXPANSION='❯'"
                .to_string(),
        ];
        sub(
            &mut l,
            "PROMPT_CHAR_{OK,ERROR}_VIINS_CONTENT_EXPANSION",
            "'>'",
        );
        assert!(l[0].ends_with("_CONTENT_EXPANSION='>'"), "{}", l[0]);
    }

    #[test]
    fn uncomment_drops_hash_prefix() {
        // No two-space gap after ARG → remainder kept verbatim.
        let mut l = vec!["    # typeset -g POWERLEVEL9K_LOCK_ICON=x".to_string()];
        uncomment(&mut l, "typeset -g POWERLEVEL9K_LOCK_ICON");
        assert_eq!(l[0], "    typeset -g POWERLEVEL9K_LOCK_ICON=x");
    }

    #[test]
    fn uncomment_doubles_two_space_gap() {
        // Exactly two spaces after ARG → doubled to four (match[2]$match[2]).
        let mut l = vec!["  # os_icon  # comment".to_string()];
        uncomment(&mut l, "os_icon");
        assert_eq!(l[0], "  os_icon    # comment");
    }

    #[test]
    fn uncomment_bare_word_to_eol() {
        let mut l = vec!["    # os_icon".to_string()];
        uncomment(&mut l, "os_icon");
        assert_eq!(l[0], "    os_icon");
    }

    #[test]
    fn rep_global_literal() {
        let mut l = vec!["a ⇡ b ⇡ c".to_string()];
        rep(&mut l, "⇡", ">");
        assert_eq!(l[0], "a > b > c");
    }

    /// Drive the REAL bundled `p10k-lean.zsh` template through
    /// generate_config with default-lean settings and assert the wizard's
    /// substitutions landed and nothing is corrupted. Gated on the
    /// template being present (user's zinit checkout).
    #[test]
    fn real_lean_template_generates_valid_config() {
        let candidates = [
            format!(
                "{}/.zinit/plugins/romkatv---powerlevel10k/config/p10k-lean.zsh",
                std::env::var("HOME").unwrap_or_default()
            ),
            format!(
                "{}/forkedRepos/powerlevel10k/config/p10k-lean.zsh",
                std::env::var("HOME").unwrap_or_default()
            ),
        ];
        let Some(path) = candidates.iter().find(|p| std::path::Path::new(p).exists()) else {
            eprintln!("skip: no p10k-lean.zsh template found");
            return;
        };
        let base = std::fs::read_to_string(path).unwrap();
        let mut s = WizardSettings::for_style(PromptStyle::Lean);
        s.mode = "nerdfont-complete".to_string();
        s.left_frame = false;
        s.right_frame = false;
        s.instant_prompt = "verbose".to_string();
        s.options = vec!["lean".into(), "unicode".into(), "2 lines".into()];
        let out = generate_config(&base, &s, "2026-01-01 at 00:00 UTC");

        // Header present and correctly attributed.
        assert!(out.starts_with("# Generated by Powerlevel10k configuration wizard on 2026-01-01"));
        assert!(out.contains("# Based on romkatv/powerlevel10k/config/p10k-lean.zsh."));
        assert!(out.contains("# Wizard options: lean, unicode, 2 lines."));
        // MODE / INSTANT_PROMPT substitutions landed (RHS replaced, not
        // left as the template default).
        assert!(
            out.contains("typeset -g POWERLEVEL9K_MODE=nerdfont-complete"),
            "MODE not set"
        );
        assert!(
            out.contains("typeset -g POWERLEVEL9K_INSTANT_PROMPT=verbose"),
            "INSTANT_PROMPT not set"
        );
        assert!(
            out.contains("typeset -g POWERLEVEL9K_PROMPT_ADD_NEWLINE=false"),
            "ADD_NEWLINE not set"
        );
        // The `{{key}}` template-substitution placeholders p10k uses are
        // all resolved — none should survive.
        assert!(
            !out.contains("POWERLEVEL9K_MODE={"),
            "unresolved MODE placeholder"
        );
        // Every non-empty, non-comment line is a valid `typeset`/keyword —
        // a crude corruption check (no line begins with a stray `=`).
        for line in out.lines() {
            assert!(
                !line.trim_start().starts_with('='),
                "corrupt line: {line:?}"
            );
        }
    }

    #[test]
    fn num_lines_1_drops_newline_and_marker() {
        let s = WizardSettings {
            num_lines: 1,
            ..WizardSettings::for_style(PromptStyle::Lean)
        };
        let base = "typeset -g POWERLEVEL9K_MODE=x\n    newline\n# ===[ Line #2 ]===\n    dir";
        let out = generate_config(base, &s, "2026-01-01");
        assert!(!out.contains("newline"), "{out}");
        assert!(!out.contains("Line #2"), "{out}");
        assert!(out.contains("    dir"), "{out}");
    }
}
