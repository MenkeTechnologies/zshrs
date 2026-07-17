//! Sample prompt templates for the wizard's live previews
//! (internal/wizard.zsh:51-99, the `${style}_left`/`${style}_right`
//! arrays) plus the settings→escape-string substitution `print_prompt`
//! applies (wizard:117-164).
//!
//! Each style contributes a `left` and `right` array of 4 elements
//! (two line pairs: `[l0a l0b l1a l1b]`). The strings contain zsh prompt
//! escapes AND wizard locals (`$frame_color[$color]`, `$prompt_char`,
//! `$left_sep`, …) that `substitute` resolves from the settings before
//! `promptexpand`.

use super::config_gen::{BG_COLOR, PREFIX_COLOR, SEP_COLOR};
use super::settings::{PromptStyle, WizardSettings};

/// The (left, right) sample arrays for the settings' style (wizard:51-99).
pub fn sample(s: &WizardSettings) -> (Vec<String>, Vec<String>) {
    let v = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match s.style {
        // wizard:51-59
        PromptStyle::Lean => (
            v(&[
                "%$FF╭─ ",
                "%31F%B%39F~%b%31F/%B%39Fsrc%b%f $PFX1%76F$EI3master%f ",
                "%$FF╰─",
                "%76F$PC%f $CUR",
            ]),
            v(&[
                " $PFX2%101F$EI45s%f$TIMEBLK",
                " %$FF─╮%f",
                "",
                " %$FF─╯%f",
            ]),
        ),
        // wizard:61-69
        PromptStyle::Lean8 => (
            v(&[
                "%$FF╭─ ",
                "%4F~/src%f $PFX1%2F$EI3master%f ",
                "%$FF╰─",
                "%2F$PC%f $CUR",
            ]),
            v(&[
                " $PFX2%3F$EI45s%f$TIMEBLK8",
                " %$FF─╮%f",
                "",
                " %$FF─╯%f",
            ]),
        ),
        // wizard:71-79
        PromptStyle::Classic => (
            v(&[
                "%$FF╭─",
                "%F{$BG}$LTAIL%K{$BG} %31F%B%39F~%b%K{$BG}%31F/%B%39Fsrc%b%K{$BG} %$SEPF$LSUB%f %$PFXF$PFX1%76F$EI3master %k%$BGF$LHEAD%f",
                "%$FF╰─",
                "%f $CUR",
            ]),
            v(&[
                "%$BGF$RHEAD%K{$BG}%f %$PFXF$PFX2%101F5s $EI4$TIMEBLKC%k%F{$BG}$RTAIL%f",
                "%$FF─╮%f",
                "",
                "%$FF─╯%f",
            ]),
        ),
        // wizard:91-99
        PromptStyle::Rainbow => (
            v(&[
                "%$FF╭─",
                "%F{4}$LTAIL%K{4}%254F $EI2%B%255F~%b%K{4}%254F/%B%255Fsrc%b%K{4} %K{2}%4F$LSEP %0F$PFX1$EI3master %k%2F$LHEAD%f",
                "%$FF╰─",
                "%f $CUR",
            ]),
            v(&[
                "%3F$RHEAD%K{3} %0F$PFX25s $EI4%3F$TIMEBLKR%k$RTAIL%f",
                "%$FF─╮%f",
                "",
                "%$FF─╯%f",
            ]),
        ),
        // wizard:81-89
        PromptStyle::Pure => (
            v(&[
                "",
                "%F{$PBLUE}~/src%f %F{$PGREY}master%f $PURERP_L",
                "",
                "%F{$PMAG}$PC%f $CUR",
            ]),
            v(&["$PURERP_R", "", "", ""]),
        ),
    }
}

/// Resolve the wizard locals in a template string to zsh prompt escapes,
/// then leave it for `promptexpand`. Faithful to the substitutions
/// `print_prompt` performs (wizard:133-164 + the array-index expansions).
pub fn substitute(t: &str, s: &WizardSettings) -> String {
    let ci = s.color as usize;
    let fc = s.frame_color[ci.min(3)];
    let cursor = "%S %s"; // a reverse-video block stands in for the cursor cell
    let ei = |n: usize| -> String {
        let g = s.extra_icons.get(n).cloned().unwrap_or_default();
        if g.is_empty() { String::new() } else { format!("{g} ") }
    };
    let pfx = |n: usize| -> String {
        let p = s.prefixes.get(n).cloned().unwrap_or_default();
        if p.is_empty() { String::new() } else { p }
    };
    // wizard:57 `${time:+ $prefixes[3]%66F$extra_icons[5]$time%f}`.
    let time_blk = |fg: &str| -> String {
        match &s.time {
            Some(t) => format!(" %{fg}F{t}%f"),
            None => String::new(),
        }
    };
    let pure_rp_l = if s.pure_use_rprompt { String::new() } else { format!("%F{{{}}}5s%f ", s.pure_color("yellow")) };
    let pure_rp_r = if s.pure_use_rprompt {
        let mut r = format!("%F{{{}}}5s%f", s.pure_color("yellow"));
        if let Some(t) = &s.time {
            r.push_str(&format!(" %F{{{}}}{t}%f", s.pure_color("grey")));
        }
        r
    } else {
        match &s.time {
            Some(t) => format!("%F{{{}}}{t}%f", s.pure_color("grey")),
            None => String::new(),
        }
    };

    let mut out = t.to_string();
    let reps: &[(&str, String)] = &[
        // Frame foreground color. Like its siblings `$BGF`/`$SEPF`/`$PFXF`,
        // this must expand to a COMPLETE `%<n>F` escape — emitting just the
        // number (`%2`) makes promptexpand treat the next glyph's leading
        // byte as the escape's directive char, corrupting multibyte frame
        // glyphs (`╭`/`╰` → U+FFFD tofu). The trailing `F` was missing.
        ("$FF", format!("{}F", fc)),
        ("$BGF", format!("{}F", BG_COLOR[ci])),
        ("$BG", BG_COLOR[ci].to_string()),
        ("$SEPF", format!("{}F", SEP_COLOR[ci])),
        ("$PFXF", format!("{}F", PREFIX_COLOR[ci])),
        ("$LSEP", s.left_sep.clone()),
        ("$RSEP", s.right_sep.clone()),
        ("$LSUB", s.left_subsep.clone()),
        ("$RSUB", s.right_subsep.clone()),
        ("$LHEAD", s.left_head.clone()),
        ("$RHEAD", s.right_head.clone()),
        ("$LTAIL", s.left_tail.clone()),
        ("$RTAIL", s.right_tail.clone()),
        ("$PC", s.prompt_char.clone()),
        ("$CUR", cursor.to_string()),
        ("$PFX1", pfx(0)),
        ("$PFX2", pfx(1)),
        ("$EI2", ei(1)),
        ("$EI3", ei(2)),
        ("$EI45", ei(3)),
        ("$TIMEBLK8", time_blk("6")),
        ("$TIMEBLKC", time_blk(&SEP_COLOR[ci].to_string())),
        ("$TIMEBLKR", time_blk("7")),
        ("$TIMEBLK", time_blk("66")),
        ("$PBLUE", s.pure_color("blue").to_string()),
        ("$PGREY", s.pure_color("grey").to_string()),
        ("$PMAG", s.pure_color("magenta").to_string()),
        ("$PURERP_L", pure_rp_l),
        ("$PURERP_R", pure_rp_r),
    ];
    // Longest-token-first so `$BGF` isn't eaten by `$BG`.
    let mut ordered: Vec<&(&str, String)> = reps.iter().collect();
    ordered.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    for (k, val) in ordered {
        if out.contains(k) {
            out = out.replace(k, val);
        }
    }
    out
}

/// Visible display width of a prompt-EXPANDED string (ANSI stripped,
/// WCWIDTH summed) — the wizard's `prompt_length` (wizard:101-115).
pub fn visible_width(expanded: &str) -> usize {
    let mut out = String::with_capacity(expanded.len());
    let mut it = expanded.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            // Skip a CSI / two-byte escape.
            if it.peek() == Some(&'[') {
                it.next();
                for c in it.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            } else {
                it.next();
            }
            continue;
        }
        out.push(c);
    }
    out.chars()
        .map(|c| crate::ported::zsh_h::WCWIDTH(c).max(0) as usize)
        .sum()
}
