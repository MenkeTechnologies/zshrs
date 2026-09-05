//! bash `$PS1` prompt-escape expansion for the `--bash` drop-in.
//!
//! **zshrs-only — no zsh C counterpart.** zsh prompts use `%` escapes;
//! bash prompts use BACKSLASH escapes (`\u@\h \w\$ `), and the two sets
//! are disjoint. Without this, every bash user's prompt rendered as
//! literal backslash text — `[\u@\h \W]\$` on screen — which makes the
//! `--bash` drop-in unusable as a real login shell, since `PS1=` is the
//! one line essentially every `.bashrc` sets.
//!
//! Rather than reimplement host / cwd / time / job-count lookups, this
//! TRANSLATES bash's escape set into the equivalent zsh prompt string and
//! hands it to the existing, tested `%`-expander in `ported::prompt`. The
//! mapping is bash(1) PROMPTING against zshall(1) EXPANSION OF PROMPT
//! SEQUENCES:
//!
//! | bash | zsh | meaning |
//! |------|-----|---------|
//! | `\u` | `%n` | user name |
//! | `\h` | `%m` | host up to the first `.` |
//! | `\H` | `%M` | full host name |
//! | `\w` | `%~` | `$PWD`, `$HOME` as `~` |
//! | `\W` | `%1~` | basename of `$PWD`, `$HOME` as `~` |
//! | `\$` | `%(!.#.$)` | `#` when euid 0, else `$` — NOT zsh's `%#`, which yields `%` |
//! | `\!` | `%!` | history number |
//! | `\j` | `%j` | jobs the shell is managing |
//! | `\l` | `%l` | tty basename |
//! | `\t` | `%D{%H:%M:%S}` | 24-hour time with seconds |
//! | `\T` | `%D{%I:%M:%S}` | 12-hour time with seconds |
//! | `\@` | `%D{%I:%M %p}` | 12-hour am/pm |
//! | `\A` | `%D{%H:%M}` | 24-hour, no seconds |
//! | `\d` | `%D{%a %b %d}` | "Tue May 26" |
//! | `\D{f}` | `%D{f}` | strftime, passed through |
//! | `\[` `\]` | `%{` `%}` | begin / end zero-width sequence |
//!
//! `\s`, `\v`, `\V`, `\#`, `\a`, `\e`, `\n`, `\r`, `\\` and `\nnn` are
//! substituted literally here — they need no prompt-engine support.
//!
//! Two details that are easy to get wrong and are handled explicitly:
//!
//!   * A literal `%` in a bash prompt must be doubled to `%%`, or zsh's
//!     expander eats it. `PS1='100%% done'` in bash prints `100% done`;
//!     an untranslated `%` would have been read as an escape introducer.
//!   * `\$` is NOT zsh's `%#`. zsh prints `%` for an unprivileged user
//!     where bash prints `$`, so the ternary `%(!.#.$)` is used instead.
//!
//! Not modelled: `PROMPT_DIRTRIM` (bash truncates `\w` to that many
//! trailing components), and the ordering nuance that bash expands
//! backslash escapes BEFORE the `promptvars` word expansion while this
//! runs the word expansion first — visible only when a command
//! substitution's OUTPUT itself contains `%` or a backslash escape.

/// Translate a bash prompt string into the equivalent zsh prompt string.
///
/// The result is fed to `ported::prompt::expand_prompt`, so every escape
/// that has a zsh counterpart keeps zsh's tested implementation.
pub fn translate(ps: &str) -> String {
    let mut out = String::with_capacity(ps.len() + 16);
    let mut chars = ps.chars().peekable();
    while let Some(c) = chars.next() {
        // A literal `%` must survive zsh's expander untouched.
        if c == '%' {
            out.push_str("%%");
            continue;
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(esc) = chars.next() else {
            // A trailing lone backslash is literal, as in bash.
            out.push('\\');
            break;
        };
        match esc {
            'u' => out.push_str("%n"),
            'h' => out.push_str("%m"),
            'H' => out.push_str("%M"),
            'w' => out.push_str("%~"),
            'W' => out.push_str("%1~"),
            'j' => out.push_str("%j"),
            'l' => out.push_str("%l"),
            '!' => out.push_str("%!"),
            // bash: `#` for euid 0, `$` otherwise. zsh's `%#` prints `%`
            // for the unprivileged case, so the ternary is required.
            '$' => out.push_str("%(!.#.$)"),
            // bash's command number. zsh has no counterpart; the history
            // number is the closest observable and matches for a session
            // that has not re-read its history file.
            '#' => out.push_str("%!"),
            't' => out.push_str("%D{%H:%M:%S}"),
            'T' => out.push_str("%D{%I:%M:%S}"),
            '@' => out.push_str("%D{%I:%M %p}"),
            'A' => out.push_str("%D{%H:%M}"),
            // bash zero-pads the day: `\d` on the 4th prints "Fri Sep 04",
            // not "Fri Sep  4". `%e` (space-padded) was wrong; `%d` is the
            // zero-padded field bash uses. Caught by the prompt fuzz.
            'd' => out.push_str("%D{%a %b %d}"),
            // `\D{format}` — pass the format straight through to `%D{…}`.
            // A `\D` with no brace is bash's default `%X` locale time.
            'D' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut fmt = String::new();
                    for f in chars.by_ref() {
                        if f == '}' {
                            break;
                        }
                        fmt.push(f);
                    }
                    out.push_str("%D{");
                    out.push_str(&fmt);
                    out.push('}');
                } else {
                    out.push_str("%D{%X}");
                }
            }
            // Zero-width regions: bash's `\[`/`\]` are zsh's `%{`/`%}`.
            '[' => out.push_str("%{"),
            ']' => out.push_str("%}"),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            'a' => out.push('\u{07}'),
            'e' => out.push('\u{1b}'),
            '\\' => out.push('\\'),
            's' => out.push_str(&shell_name()),
            'v' => out.push_str(&format!(
                "{}.{}",
                crate::dash_mode::BASH_VERSION_MAJOR,
                crate::dash_mode::BASH_VERSION_MINOR
            )),
            'V' => out.push_str(&format!(
                "{}.{}.{}",
                crate::dash_mode::BASH_VERSION_MAJOR,
                crate::dash_mode::BASH_VERSION_MINOR,
                crate::dash_mode::BASH_VERSION_PATCH
            )),
            // `\nnn` — up to three octal digits, as one character.
            '0'..='7' => {
                let mut oct = String::from(esc);
                while oct.len() < 3 {
                    match chars.peek() {
                        Some(d @ '0'..='7') => {
                            oct.push(*d);
                            chars.next();
                        }
                        _ => break,
                    }
                }
                match u32::from_str_radix(&oct, 8).ok().and_then(char::from_u32) {
                    Some(ch) if ch == '%' => out.push_str("%%"),
                    Some(ch) => out.push(ch),
                    None => out.push_str(&oct),
                }
            }
            // bash leaves an unknown escape as backslash + the character.
            other => {
                out.push('\\');
                if other == '%' {
                    out.push_str("%%");
                } else {
                    out.push(other);
                }
            }
        }
    }
    out
}

/// `\s` — bash(1): "the name of the shell, the basename of `$0`".
fn shell_name() -> String {
    crate::ported::params::getsparam("0")
        .map(|z| {
            let bare = z.trim_start_matches('-');
            std::path::Path::new(bare)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(bare)
                .to_string()
        })
        .unwrap_or_else(|| "bash".to_string())
}

thread_local! {
    /// Set while a bash prompt is being translated and expanded, so the
    /// `%` sequences [`translate`] produces are not translated a second
    /// time by a nested `expand_prompt` (`%D{…}`, PROMPTSUBST, …).
    static IN_TRANSLATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Clears [`IN_TRANSLATION`] when the prompt expansion it guards returns.
pub struct TranslationGuard;

impl Drop for TranslationGuard {
    fn drop(&mut self) {
        IN_TRANSLATION.with(|c| c.set(false));
    }
}

/// The hook `ported::prompt::expand_prompt` calls on entry.
///
/// Returns the translated prompt plus a guard to hold for the rest of the
/// expansion, or `None` when this is not a bash-mode prompt (or is a
/// nested expansion of one already translated), in which case the caller
/// expands the string unchanged.
pub fn begin_translation(s: &str) -> Option<(String, TranslationGuard)> {
    if !crate::dash_mode::bash_mode() || IN_TRANSLATION.with(|c| c.get()) {
        return None;
    }
    let translated = translate(s);
    IN_TRANSLATION.with(|c| c.set(true));
    Some((translated, TranslationGuard))
}

/// Expand a bash prompt string the way bash's `${var@P}` does: translate
/// the escapes, then run the shared prompt expander.
pub fn expand(ps: &str) -> String {
    // `expand_prompt` performs the translation itself, through
    // `begin_translation`. Translating here as well would double it —
    // `\u` would become `%n` and then `%%n`, which expands to the literal
    // text `%n` instead of the user name.
    let expanded = crate::ported::prompt::expand_prompt(ps);
    // `\[` / `\]` become the readline "ignore this run when measuring
    // width" markers (`\x01` / `\x02`) on the way to the line editor, but
    // `${v@P}` hands the user a plain string: bash's own `${PS1@P}` emits
    // the escape sequence with no markers around it. Strip them here so
    // the reported string matches byte for byte; the display path keeps
    // its markers because it never goes through this function.
    if expanded.contains(['\u{01}', '\u{02}']) {
        expanded
            .chars()
            .filter(|c| *c != '\u{01}' && *c != '\u{02}')
            .collect()
    } else {
        expanded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mappings that are pure text, checked without a shell.
    #[test]
    fn escapes_translate_to_their_zsh_counterparts() {
        assert_eq!(translate(r"\u@\h"), "%n@%m");
        assert_eq!(translate(r"\u@\H"), "%n@%M");
        assert_eq!(translate(r"\w"), "%~");
        assert_eq!(translate(r"\W"), "%1~");
        assert_eq!(translate(r"\j \l \!"), "%j %l %!");
        assert_eq!(
            translate(r"\t|\T|\A"),
            "%D{%H:%M:%S}|%D{%I:%M:%S}|%D{%H:%M}"
        );
        assert_eq!(translate(r"\D{%F}"), "%D{%F}");
        // Zero-padded day, matching bash's own `\d` output.
        assert_eq!(translate(r"\d"), "%D{%a %b %d}");
        assert_eq!(translate(r"\D"), "%D{%X}");
        assert_eq!(translate(r"\[\e[0m\]"), "%{\u{1b}[0m%}");
    }

    /// `\$` must be the `#`/`$` ternary, NOT zsh's `%#` — zsh's prints a
    /// literal `%` for an unprivileged user, which is the wrong character
    /// and the most visible thing on a bash prompt.
    #[test]
    fn dollar_escape_is_not_zsh_percent_hash() {
        let t = translate(r"\$");
        assert_eq!(t, "%(!.#.$)");
        assert!(!t.contains("%#"), "must not map onto zsh's %# ");
    }

    /// A literal `%` has to be doubled or zsh's expander consumes it.
    /// `PS1='100% done'` is a real thing people write.
    #[test]
    fn literal_percent_is_doubled() {
        assert_eq!(translate("100% done"), "100%% done");
        assert_eq!(translate(r"\u 50%"), "%n 50%%");
        // …including one produced by an octal escape.
        assert_eq!(translate(r"\045"), "%%");
    }

    /// Octal escapes, including the short forms and the boundary where a
    /// following digit is NOT part of the escape.
    #[test]
    fn octal_escapes_decode() {
        assert_eq!(translate(r"\101"), "A");
        assert_eq!(translate(r"\1011"), "A1");
        assert_eq!(translate(r"\7"), "\u{07}");
    }

    /// Backslash handling at the edges: a doubled backslash is literal, a
    /// trailing lone one is literal, and an unknown escape keeps both
    /// characters exactly as bash leaves them.
    #[test]
    fn backslash_edge_cases_match_bash() {
        assert_eq!(translate(r"\\"), r"\");
        assert_eq!(translate(r"end\"), r"end\");
        assert_eq!(translate(r"\q"), r"\q");
        assert_eq!(translate(r"\z\u"), r"\z%n");
    }

    /// The C-style control escapes are emitted as the characters
    /// themselves, so a prompt can embed a newline or an ESC directly.
    #[test]
    fn control_escapes_become_characters() {
        assert_eq!(translate(r"a\nb"), "a\nb");
        assert_eq!(translate(r"a\rb"), "a\rb");
        assert_eq!(translate(r"\a"), "\u{07}");
        assert_eq!(translate(r"\e"), "\u{1b}");
    }
}
