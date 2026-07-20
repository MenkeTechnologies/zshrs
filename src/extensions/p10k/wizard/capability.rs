//! Terminal capability detection for `p10k configure`
//! (internal/wizard.zsh:1994-2060 + the `ask_font`/`ask_diamond`/… glyph
//! probes). The zsh wizard determines the exact `POWERLEVEL9K_MODE` by
//! interactively asking the user whether sample glyphs render; the
//! native version derives the same signals from locale + terminfo +
//! COLORTERM, defaulting to the modern nerdfont path for UTF-8
//! terminals (the interactive font-install flow is not reproduced —
//! the user installs MesloLGS NF out of band).

/// Detected terminal capabilities.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    /// wizard:1994-1998 — 24-bit color (COLORTERM=truecolor|24bit).
    pub has_truecolor: bool,
    /// terminfo `colors` — the palette size (8, 16, 256, …).
    pub colors: i32,
    /// wizard:2023 — UTF-8 locale + a non-dumb/linux terminal: the
    /// wizard's gate for offering unicode/nerdfont styles at all.
    pub unicode: bool,
}

/// Probe the terminal. `term`/`colorterm`/`lang`+`lc_all`+`lc_ctype`
/// come from the environment; `colors` from terminfo (via the ncurses
/// probe the shell already did, or a COLORTERM/TERM heuristic fallback).
pub fn detect() -> Caps {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let term = env("TERM");
    let colorterm = env("COLORTERM");

    // wizard:1994 — `is-at-least 5.7.1 && COLORTERM == (24bit|truecolor)`.
    let has_truecolor = colorterm == "truecolor" || colorterm == "24bit";

    // wizard:2023 — `$TERM != (dumb|linux) && CODESET == UTF-8`.
    let codeset_utf8 = {
        // $LC_ALL wins, then $LC_CTYPE, then $LANG (POSIX precedence).
        let loc = [env("LC_ALL"), env("LC_CTYPE"), env("LANG")]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_default()
            .to_ascii_lowercase();
        loc.contains("utf-8") || loc.contains("utf8")
    };
    let unicode = term != "dumb" && term != "linux" && codeset_utf8;

    // terminfo colors: prefer the shell's resolved value; else a
    // TERM/COLORTERM heuristic (truecolor or *-256color → 256).
    let colors = terminfo_colors().unwrap_or_else(|| {
        if has_truecolor || term.contains("256") || colorterm.contains("256") {
            256
        } else if term.is_empty() || term == "dumb" {
            0
        } else {
            8
        }
    });

    Caps {
        has_truecolor,
        colors,
        unicode,
    }
}

/// terminfo `colors` capability for the current $TERM. Reads `tccolours`
/// (Src/init.c:94 `Co` = max_colors), resolved by the shell's termcap
/// init. None (→ heuristic) when the shell hasn't initialized termcap.
fn terminfo_colors() -> Option<i32> {
    let n = crate::ported::init::tccolours.load(std::sync::atomic::Ordering::Relaxed);
    if n > 0 {
        Some(n)
    } else {
        None
    }
}

/// wizard:2023-2093 — derive `POWERLEVEL9K_MODE`, icon padding, and the
/// separator glyphs from the caps, WITHOUT the interactive glyph probes.
/// Modern default: UTF-8 → nerdfont-complete + diamond powerline; else
/// ascii. `unicode_selected` reflects the user's ask_charset answer
/// (false forces the ascii branch even on a UTF-8 terminal).
pub fn derive_mode(caps: &Caps, unicode_selected: bool) -> ModeChoice {
    if caps.unicode && unicode_selected {
        // Assume a Nerd Font is installed (the user has MesloLGS NF).
        ModeChoice {
            mode: "nerdfont-complete".to_string(), // wizard:2052
            icon_padding: "moderate".to_string(),  // wizard:2005
            cap_diamond: true,                     // powerline separators
            cap_python: true,
            cap_lock: true,
            cap_arrow: true,
            cap_debian: true,
        }
    } else {
        ModeChoice {
            mode: "ascii".to_string(), // wizard:2060
            icon_padding: "none".to_string(),
            cap_diamond: false,
            cap_python: false,
            cap_lock: false,
            cap_arrow: false,
            cap_debian: false,
        }
    }
}

/// The derived mode + capability flags feeding the wizard settings.
#[derive(Debug, Clone)]
pub struct ModeChoice {
    pub mode: String,
    pub icon_padding: String,
    pub cap_diamond: bool,
    pub cap_python: bool,
    pub cap_lock: bool,
    pub cap_arrow: bool,
    pub cap_debian: bool,
}
