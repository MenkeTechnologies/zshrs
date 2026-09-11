//! `zbanner` / `zshrs --banner` — the ZSHRS logo, a boxed summary of the
//! builtin totals, and one live line for the daemon and this shell.
//!
//! Ported from ztmux's `src/extensions/banner.rs`: the same ANSI Shadow logo
//! treatment, the same single-line box sized to its printed width, and the
//! same "live counts, or the reason they are missing" line under it. ztmux's
//! console prints its banner when it opens; zshrs prints nothing at startup,
//! so this only ever runs when asked for.
//!
//! The daemon is the counterpart of ztmux's server socket. It is read with
//! [`daemon_presence::check`], which connects without storing the result, so
//! a banner drawn mid-session cannot change how the shell treats the daemon.
//!
//! zshrs-original — C zsh has no counterpart, so this lives under
//! `src/extensions/` per `docs/PORT.md`.

use std::io::IsTerminal;
use std::path::Path;

use crate::daemon_presence::{self, Mode};
use crate::p10k::wizard::templates::visible_width;
use crate::ported::zsh_h::{DISABLED, PM_UNSET, STAT_NOPRINT};

/// `figlet -f "ANSI Shadow" ZSHRS` — the README banner — as (SGR colour,
/// line) pairs, graded cyan → magenta → red the way ztmux grades its own.
const LOGO: [(&str, &str); 6] = [
    ("36", " ███████╗███████╗██╗  ██╗██████╗ ███████╗"),
    ("36", " ╚══███╔╝██╔════╝██║  ██║██╔══██╗██╔════╝"),
    ("35", "   ███╔╝ ███████╗███████║██████╔╝███████╗"),
    ("35", "  ███╔╝  ╚════██║██╔══██║██╔══██╗╚════██║"),
    ("31", " ███████╗███████║██║  ██║██║  ██║███████║"),
    ("31", " ╚══════╝╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝"),
];

/// What this shell holds right now — the half of the live line ztmux fills
/// from its server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShellCounts {
    pub functions: usize,
    pub aliases: usize,
    pub parameters: usize,
    pub jobs: usize,
}

impl ShellCounts {
    /// Count the live tables with the filters zsh's own `parameter` module
    /// applies (`Src/Modules/parameter.c`): enabled functions (`$functions`,
    /// c:470), enabled aliases, set parameters (`$parameters`, c:138), and
    /// the jobs `$jobstates` lists (c:1429-1430).
    pub fn read() -> Self {
        let functions = crate::ported::hashtable::shfunctab_lock()
            .read()
            .map(|tab| {
                tab.iter()
                    .filter(|(_, f)| f.node.flags & DISABLED == 0)
                    .count()
            })
            .unwrap_or(0);
        let aliases = crate::ported::hashtable::aliastab_lock()
            .read()
            .map(|tab| {
                tab.iter()
                    .filter(|(_, a)| a.node.flags & DISABLED == 0)
                    .count()
            })
            .unwrap_or(0);
        let parameters = crate::ported::params::paramtab()
            .read()
            .map(|tab| {
                tab.iter()
                    .filter(|(_, p)| p.node.flags as u32 & PM_UNSET == 0)
                    .count()
            })
            .unwrap_or(0);
        let (jobtab, maxjob) = crate::ported::jobs::selectjobtab();
        let jobs = (1..=maxjob)
            .filter_map(|n| jobtab.get(n))
            .filter(|j| j.stat != 0 && !j.procs.is_empty() && j.stat & STAT_NOPRINT == 0)
            .count();
        Self {
            functions,
            aliases,
            parameters,
            jobs,
        }
    }
}

/// Print the banner. `shell` is `None` for `zshrs --banner`, where no shell
/// has started and the tables hold nothing of the user's, so the live line
/// reports the daemon alone.
pub fn print_banner(shell: Option<ShellCounts>) {
    let color = colored();
    for (code, line) in LOGO {
        println!("{}", paint(line, code, color));
    }

    let summary = format!(
        " ZSHRS // v{} // {} builtins // {} extensions ",
        env!("CARGO_PKG_VERSION"),
        crate::ext_builtins::compat_builtin_names().len(),
        crate::ext_builtins::extension_builtin_names().len(),
    );
    for line in box_lines(&summary, color) {
        println!("{line}");
    }

    let socket = daemon_presence::socket_path();
    println!(
        "{}",
        live_line(socket.as_deref(), daemon_presence::check(), shell, color)
    );
}

/// Colour when stdout is a terminal and `NO_COLOR` is unset.
fn colored() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

/// Wrap `s` in SGR `code` when colouring.
fn paint(s: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// The line under the box: the daemon socket and whether anything answers
/// on it, then this shell's counts when there is a shell to count.
fn live_line(socket: Option<&Path>, mode: Mode, shell: Option<ShellCounts>, color: bool) -> String {
    let socket = socket
        .map(tilde)
        .unwrap_or_else(|| "(no socket path)".to_string());
    let state = match mode {
        Mode::Present => paint("// up", "32", color),
        Mode::Disabled => paint("// daemon disabled", "33", color),
        Mode::Absent | Mode::Unknown => paint("// no daemon running", "31", color),
    };
    let mut line = format!(
        "{}  {state}",
        paint(&format!(" daemon {socket}"), "2", color)
    );
    if let Some(c) = shell {
        for part in [
            count(c.functions, "function", "functions"),
            count(c.aliases, "alias", "aliases"),
            count(c.parameters, "parameter", "parameters"),
            count(c.jobs, "job", "jobs"),
        ] {
            line.push_str("  ");
            line.push_str(&part);
        }
    }
    line
}

/// `1 alias` / `2 aliases` — every count in the banner is plural-correct.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `$HOME/…` as `~/…`, so the path reads the way the user types it.
fn tilde(path: &Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && shown.starts_with(&home) => {
            format!("~{}", &shown[home.len()..])
        }
        _ => shown,
    }
}

/// The three lines of a single-line box around `line`, sized to its printed
/// width (colour escapes excluded), so the borders stay aligned however long
/// the version or the builtin counts get.
fn box_lines(line: &str, color: bool) -> [String; 3] {
    let rule = "─".repeat(visible_width(line));
    let border = |s: &str| paint(s, "36", color);
    [
        border(&format!(" ┌{rule}┐")),
        format!("{}{line}{}", border(" │"), border("│")),
        border(&format!(" └{rule}┘")),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_box_borders_match_the_content_width() {
        // The box is measured on printed width, so neither a longer version
        // string nor the colour escapes can push the right border out of line.
        for content in [
            " ZSHRS // v0.12.61 // 157 builtins // 146 extensions ",
            " ZSHRS // v10.100.1000 // 1 builtin // 1 extension ",
            "",
        ] {
            for color in [false, true] {
                let widths: Vec<usize> = box_lines(content, color)
                    .iter()
                    .map(|l| visible_width(l))
                    .collect();
                assert_eq!(
                    widths[0], widths[1],
                    "top border and content disagree for {content:?} (color: {color})"
                );
                assert_eq!(
                    widths[1], widths[2],
                    "content and bottom border disagree for {content:?} (color: {color})"
                );
            }
        }
    }

    #[test]
    fn logo_lines_share_one_printed_width() {
        let widths: Vec<usize> = LOGO.iter().map(|(_, l)| visible_width(l)).collect();
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "logo lines are ragged: {widths:?}"
        );
    }

    #[test]
    fn live_line_says_why_the_daemon_is_missing_instead_of_claiming_it_is_up() {
        let sock = Path::new("/tmp/zshrs-test/daemon.sock");
        assert!(live_line(Some(sock), Mode::Absent, None, false).contains("// no daemon running"));
        assert!(live_line(Some(sock), Mode::Disabled, None, false).contains("// daemon disabled"));
        assert!(live_line(Some(sock), Mode::Present, None, false).contains("// up"));
        assert!(live_line(None, Mode::Absent, None, false).contains("(no socket path)"));
    }

    #[test]
    fn live_line_counts_are_plural_correct_and_absent_without_a_shell() {
        let one = ShellCounts {
            functions: 1,
            aliases: 1,
            parameters: 1,
            jobs: 1,
        };
        let line = live_line(None, Mode::Present, Some(one), false);
        assert!(
            line.ends_with("1 function  1 alias  1 parameter  1 job"),
            "{line:?}"
        );
        let line = live_line(None, Mode::Present, Some(ShellCounts::default()), false);
        assert!(
            line.ends_with("0 functions  0 aliases  0 parameters  0 jobs"),
            "{line:?}"
        );
        assert!(!live_line(None, Mode::Present, None, false).contains("functions"));
    }
}
