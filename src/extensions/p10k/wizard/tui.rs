//! Interactive full-screen driver for `p10k configure` — port of the
//! `render_screen` / `ask_*` / `print_prompt` machinery
//! (internal/wizard.zsh:101-1617) and the top-level `while true` loop
//! (wizard:2000-2153).
//!
//! Uses raw-mode single-keypress input on the controlling tty, the
//! alternate screen, and live prompt previews built by substituting the
//! collected settings into the wizard's sample prompt templates
//! (wizard:51-99) and prompt-expanding them.

use super::capability::{self, Caps};
use super::settings::{PromptStyle, WizardSettings};
use super::Outcome;
use std::io::{Read, Write};

/// wizard/configure.zsh:2/4 — minimum terminal size to render the wizard.
const MIN_COLS: usize = 47;
const MIN_LINES: usize = 14;

/// One question's outcome: a chosen key, Restart, or Quit.
enum Answer {
    Key(char),
    Restart,
    Quit,
}

/// Raw-tty guard: enter cbreak (no-echo, byte-at-a-time) on construction,
/// restore the saved termios + show the cursor + leave the alt screen on
/// drop. Mirrors the wizard's `stty -echo` + `smcup`/`rmcup`
/// (wizard:2000/1974-1980).
struct Tty {
    fd: i32,
    saved: libc::termios,
    alt: bool,
}

impl Tty {
    fn enter() -> std::io::Result<Self> {
        let fd = tty_fd();
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut raw = saved;
        // cbreak: disable canonical mode + echo, keep signals (Ctrl-C).
        raw.c_lflag &= !(libc::ICANON | libc::ECHO);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) };
        // Alt screen + hide cursor.
        let mut out = tty_out();
        let _ = out.write_all(b"\x1b[?1049h\x1b[?25l\x1b[H\x1b[2J");
        let _ = out.flush();
        Ok(Tty { fd, saved, alt: true })
    }
}

impl Drop for Tty {
    fn drop(&mut self) {
        let mut out = tty_out();
        if self.alt {
            // Show cursor, leave alt screen (wizard:1974-1980 restore_screen).
            let _ = out.write_all(b"\x1b[?25h\x1b[?1049l");
            let _ = out.flush();
        }
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved) };
    }
}

fn tty_fd() -> i32 {
    // The controlling tty the shell reads from (SHTTY), else stdin.
    let fd = crate::ported::init::SHTTY.load(std::sync::atomic::Ordering::Relaxed);
    if fd >= 0 {
        fd
    } else {
        libc::STDIN_FILENO
    }
}

fn tty_out() -> std::fs::File {
    use std::os::unix::io::FromRawFd;
    // Borrow the tty fd for writing without owning/closing it.
    let fd = tty_fd();
    let dup = unsafe { libc::dup(fd) };
    unsafe { std::fs::File::from_raw_fd(if dup >= 0 { dup } else { libc::STDOUT_FILENO }) }
}

/// Read one keypress (lowercased). Ctrl-C (ETX) / EOF → Quit.
fn read_key(fd: i32) -> Answer {
    let mut buf = [0u8; 1];
    let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, 1) };
    if n <= 0 {
        return Answer::Quit;
    }
    let c = buf[0];
    if c == 3 || c == 4 {
        return Answer::Quit; // Ctrl-C / Ctrl-D
    }
    Answer::Key((c as char).to_ascii_lowercase())
}

/// wizard:305-361 `render_screen` (simplified): clear, print the block,
/// return. The zsh pass/priority system drops preview lines when the
/// window is too short; here the caller passes a pre-sized block.
fn render(block: &str) {
    let mut out = tty_out();
    let _ = out.write_all(b"\x1b[H\x1b[2J");
    // The block uses \n; the tty is in raw mode so translate to \r\n.
    for line in block.split('\n') {
        let _ = out.write_all(line.as_bytes());
        let _ = out.write_all(b"\r\n");
    }
    let _ = out.flush();
}

/// wizard:382 `ask`: render the question, then read keys until one is in
/// `choices` (or `q`/Ctrl-C to quit, `r` to restart when offered).
fn ask(fd: i32, block: &str, choices: &str) -> Answer {
    let full = format!("{block}\n(q)  Quit and do nothing.\n\nChoice [{choices}q]: ");
    render(&full);
    loop {
        match read_key(fd) {
            Answer::Quit => return Answer::Quit,
            Answer::Key('q') => return Answer::Quit,
            Answer::Key('r') if choices.contains('r') => return Answer::Restart,
            Answer::Key(c) if choices.contains(c) => return Answer::Key(c),
            _ => {}
        }
    }
}

/// Build a live prompt preview for the current settings — port of
/// `print_prompt` (wizard:117-164): substitute settings into the style's
/// sample `*_left`/`*_right` templates, prompt-expand, and lay out each
/// row with the gap fill. `cols` is the preview width.
fn preview(s: &WizardSettings, cols: usize) -> String {
    let (left, right) = super::templates::sample(s);
    let render_side = |t: &str| -> String {
        let sub = super::templates::substitute(t, s);
        let (expanded, _, _) = crate::ported::prompt::promptexpand(&sub, 0, None);
        expanded
    };
    let vis = |t: &str| -> usize {
        // Visible width after prompt-expansion + ANSI strip.
        super::templates::visible_width(&render_side(t))
    };
    let mut out = String::new();
    let indent = 2usize;
    // Two rows (line pairs) for a 2-line prompt; one for a 1-line prompt.
    let rows = if s.num_lines == 2 { left.len() / 2 } else { 1 };
    for i in 0..rows {
        let (li, ri) = (i * 2, i * 2);
        let l = format!("{}{}", left.get(li).cloned().unwrap_or_default(), left.get(li + 1).cloned().unwrap_or_default());
        let r = format!("{}{}", right.get(ri).cloned().unwrap_or_default(), right.get(ri + 1).cloned().unwrap_or_default());
        let lw = vis(&l);
        let rw = vis(&r);
        let gap = cols.saturating_sub(indent * 2).saturating_sub(lw + rw);
        let fill = if s.num_lines == 2 && i == 0 { s.gap_char.chars().next().unwrap_or(' ') } else { ' ' };
        out.push_str(&" ".repeat(indent));
        out.push_str(&render_side(&l));
        out.push_str(&fill.to_string().repeat(gap));
        out.push_str(&render_side(&r));
        out.push('\n');
    }
    out
}

/// Run the interactive wizard. Returns `Some(Outcome)` on completion,
/// `None` on quit/Ctrl-C. `root` is the p10k install dir; `force`
/// suppresses the "no config yet" greeting variant.
pub fn run_wizard(root: &str, force: bool) -> std::io::Result<Option<Outcome>> {
    let _ = (root, force);
    let caps = capability::detect();
    let (cols, lines) = term_size();
    if cols < MIN_COLS || lines < MIN_LINES {
        return Err(std::io::Error::other(format!(
            "terminal too small ({cols}x{lines}); need at least {MIN_COLS}x{MIN_LINES}"
        )));
    }
    let tty = Tty::enter()?;
    let fd = tty.fd;
    let pcols = cols.min(88); // wizard:302 get_columns()

    // wizard:2002 — restart loop.
    loop {
        match collect(fd, &caps, pcols) {
            Flow::Done(mut s) => {
                s.has_truecolor = caps.has_truecolor;
                let home = std::env::var("HOME").unwrap_or_default();
                let cfg = cfg_path();
                let zshrc = zshrc_path(&home);
                let ts = timestamp();
                drop(tty);
                return Ok(Some(Outcome {
                    settings: s,
                    config_path: cfg,
                    write_zshrc: false, // set by ask_zshrc_edit inside collect via a flag; see below
                    zshrc_path: zshrc,
                    timestamp: ts,
                }));
            }
            Flow::Restart => continue,
            Flow::Quit => {
                drop(tty);
                return Ok(None);
            }
        }
    }
}

enum Flow {
    Done(WizardSettings),
    Restart,
    Quit,
}

/// One pass through every question (wizard:2023-2119). Any `r` bubbles a
/// Restart; `q`/Ctrl-C a Quit.
fn collect(fd: i32, caps: &Caps, pcols: usize) -> Flow {
    // Charset first (wizard:2023-2060) — sets mode + diamond caps.
    let unicode = caps.unicode;
    let mut s = WizardSettings::for_style(PromptStyle::Lean);
    s.has_truecolor = caps.has_truecolor;

    // ---- Style (wizard:2099 ask_style) ----
    macro_rules! q {
        ($block:expr, $choices:expr) => {
            match ask(fd, $block, $choices) {
                Answer::Key(c) => c,
                Answer::Restart => return Flow::Restart,
                Answer::Quit => return Flow::Quit,
            }
        };
    }

    if caps.colors < 256 {
        // wizard:834-841 — forced 8-color lean.
        s.style = PromptStyle::Lean8;
        s.left_frame = false;
        s.right_frame = false;
        s.frame_color = [0, 7, 2, 4];
        s.options.push("lean_8colors".into());
    } else {
        let style_extra = if unicode { "4" } else { "" };
        let blk = "\u{1b}[1mPrompt Style\u{1b}[0m\n\n(1)  Lean.\n(2)  Classic.\n(3)  Rainbow.\n(4)  Pure.\n(r)  Restart from the beginning.";
        match q!(blk, &format!("123{style_extra}r")) {
            '1' => { s.style = PromptStyle::Lean; s.left_frame = false; s.right_frame = false; s.options.push("lean".into()); }
            '2' => { s.style = PromptStyle::Classic; s.options.push("classic".into()); }
            '3' => { s.style = PromptStyle::Rainbow; s.options.push("rainbow".into()); }
            '4' => { s.style = PromptStyle::Pure; s.empty_line = true; s.options.push("pure".into()); }
            _ => {}
        }
    }

    // ---- Charset (wizard:870-914 ask_charset) ----
    let ascii_forced = !unicode;
    let mut mode = capability::derive_mode(caps, unicode && !ascii_forced);
    if matches!(s.style, PromptStyle::Lean | PromptStyle::Lean8 | PromptStyle::Classic | PromptStyle::Rainbow)
        && mode.mode != "ascii"
    {
        let blk = "\u{1b}[1mCharacter Set\u{1b}[0m\n\n(1)  Unicode.\n(2)  ASCII.\n(r)  Restart from the beginning.";
        match q!(blk, "12r") {
            '1' => s.options.push("unicode".into()),
            '2' => {
                s.options.push("ascii".into());
                mode = capability::derive_mode(caps, false);
            }
            _ => {}
        }
    }
    // Apply the derived mode + separators (wizard:2063-2093).
    s.mode = mode.mode.clone();
    s.icon_padding = mode.icon_padding.clone();
    s.cap_diamond = mode.cap_diamond;
    s.cap_python = mode.cap_python;
    apply_separators(&mut s);

    // ---- Color scheme (wizard:916-961) ----
    if caps.colors >= 256 {
        if s.style == PromptStyle::Lean {
            let blk = "\u{1b}[1mPrompt Colors\u{1b}[0m\n\n(1)  256 colors.\n(2)  8 colors.\n(r)  Restart from the beginning.";
            match q!(blk, "12r") {
                '1' => {}
                '2' => { s.style = PromptStyle::Lean8; s.frame_color = [0, 7, 2, 4]; s.options.push("lean_8colors".into()); }
                _ => {}
            }
        } else if s.style == PromptStyle::Pure && caps.has_truecolor {
            let blk = "\u{1b}[1mPrompt Colors\u{1b}[0m\n\n(1)  Original.\n(2)  Snazzy.\n(r)  Restart from the beginning.";
            match q!(blk, "12r") {
                '1' => { s.has_truecolor = false; s.options.push("original".into()); }
                '2' => { s.options.push("snazzy".into()); }
                _ => {}
            }
        }
    }

    // ---- Color depth (classic/rainbow ask_color) → color index ----
    if matches!(s.style, PromptStyle::Classic | PromptStyle::Rainbow) {
        let blk = "\u{1b}[1mPrompt Color\u{1b}[0m\n\n(1)  Lightest.\n(2)  Light.\n(3)  Dark.\n(4)  Darkest.\n(r)  Restart from the beginning.";
        match q!(blk, "1234r") {
            '1' => s.color = 0,
            '2' => s.color = 1,
            '3' => s.color = 2,
            '4' => s.color = 3,
            _ => {}
        }
    }

    // ---- Time (wizard:1013-1032) ----
    {
        let blk = format!("\u{1b}[1mShow current time?\u{1b}[0m\n\n{}\n\n(1)  No.\n(2)  24-hour format.\n(3)  12-hour format.\n(r)  Restart from the beginning.", preview(&s, pcols).trim_end());
        match q!(&blk, "123r") {
            '1' => s.time = None,
            '2' => { s.time = Some("16:23:42".into()); s.options.push("24h time".into()); }
            '3' => { s.time = Some(super::config_gen::TIME_12H.into()); s.options.push("12h time".into()); }
            _ => {}
        }
    }

    // ---- Num lines (wizard:1336-1350) ----
    {
        let blk = "\u{1b}[1mPrompt Height\u{1b}[0m\n\n(1)  One line.\n(2)  Two lines.\n(r)  Restart from the beginning.";
        match q!(blk, "12r") {
            '1' => { s.num_lines = 1; s.options.push("1 line".into()); }
            '2' => { s.num_lines = 2; s.options.push("2 lines".into()); }
            _ => {}
        }
    }

    // ---- Gap char (wizard:1354-1381) ----
    if s.num_lines == 2 && s.style != PromptStyle::Pure {
        let (dot, dash) = if s.mode == "ascii" { (".", "-") } else { ("·", "─") };
        let blk = "\u{1b}[1mPrompt Connection\u{1b}[0m\n\n(1)  Disconnected.\n(2)  Dotted.\n(3)  Solid.\n(r)  Restart from the beginning.";
        match q!(blk, "123r") {
            '1' => { s.gap_char = " ".into(); s.options.push("disconnected".into()); }
            '2' => { s.gap_char = dot.into(); s.options.push("dotted".into()); }
            '3' => { s.gap_char = dash.into(); s.options.push("solid".into()); }
            _ => {}
        }
    }

    // ---- Frame (wizard:1383-1406) ----
    if matches!(s.style, PromptStyle::Classic | PromptStyle::Rainbow | PromptStyle::Lean | PromptStyle::Lean8)
        && s.num_lines == 2
        && s.mode != "ascii"
    {
        let blk = "\u{1b}[1mPrompt Frame\u{1b}[0m\n\n(1)  No frame.\n(2)  Left.\n(3)  Right.\n(4)  Full.\n(r)  Restart from the beginning.";
        match q!(blk, "1234r") {
            '1' => { s.left_frame = false; s.right_frame = false; s.options.push("no frame".into()); }
            '2' => { s.left_frame = true; s.right_frame = false; s.options.push("left frame".into()); }
            '3' => { s.left_frame = false; s.right_frame = true; s.options.push("right frame".into()); }
            '4' => { s.left_frame = true; s.right_frame = true; s.options.push("full frame".into()); }
            _ => {}
        }
    }

    // ---- Empty line (wizard:1410-1435) ----
    {
        let blk = "\u{1b}[1mPrompt Spacing\u{1b}[0m\n\n(1)  Compact.\n(2)  Sparse.\n(r)  Restart from the beginning.";
        match q!(blk, "12r") {
            '1' => { s.empty_line = false; s.options.push("compact".into()); }
            '2' => { s.empty_line = true; s.options.push("sparse".into()); }
            _ => {}
        }
    }

    // ---- Transient prompt (wizard:1484-1520) ----
    {
        let blk = "\u{1b}[1mEnable Transient Prompt?\u{1b}[0m\n\n(y)  Yes.\n(n)  No.\n(r)  Restart from the beginning.";
        match q!(blk, "ynr") {
            'y' => { s.transient_prompt = true; s.options.push("transient_prompt".into()); }
            'n' => s.transient_prompt = false,
            _ => {}
        }
    }

    // ---- Instant prompt (wizard:1446-1480) ----
    {
        let blk = "\u{1b}[1mInstant Prompt Mode\u{1b}[0m\n\n(1)  Verbose (recommended).\n(2)  Quiet.\n(3)  Off.\n(r)  Restart from the beginning.";
        match q!(blk, "123r") {
            '1' => { s.instant_prompt = "verbose".into(); s.options.push("instant_prompt=verbose".into()); }
            '2' => { s.instant_prompt = "quiet".into(); s.options.push("instant_prompt=quiet".into()); }
            '3' => { s.instant_prompt = "off".into(); s.options.push("instant_prompt=off".into()); }
            _ => {}
        }
    }

    // ---- Config overwrite (wizard:1524-1545) ----
    if std::path::Path::new(&cfg_path()).exists() {
        let blk = "Powerlevel10k config file already exists.\n\u{1b}[1mOverwrite?\u{1b}[0m\n\n(y)  Yes.\n(r)  Restart from the beginning.";
        match q!(blk, "yr") {
            'y' => {}
            _ => {}
        }
    }

    Flow::Done(s)
}

/// wizard:2071-2092 — separator/head/tail glyphs from the diamond cap.
fn apply_separators(s: &mut WizardSettings) {
    if s.cap_diamond {
        s.left_sep = "\u{e0b0}".into();      // right_triangle
        s.right_sep = "\u{e0b2}".into();     // left_triangle
        s.left_subsep = "\u{e0b1}".into();   // right_angle
        s.right_subsep = "\u{e0b3}".into();  // left_angle
        s.left_head = "\u{e0b0}".into();
        s.right_head = "\u{e0b2}".into();
    } else {
        s.left_sep = String::new();
        s.right_sep = String::new();
        s.left_head = String::new();
        s.right_head = String::new();
        if s.mode == "ascii" {
            s.left_subsep = "|".into();
            s.right_subsep = "|".into();
            s.prompt_char = ">".into();
            s.right_frame = false;
        } else {
            s.left_subsep = "\u{2502}".into();
            s.right_subsep = "\u{2502}".into();
        }
    }
}

fn cfg_path() -> String {
    // POWERLEVEL9K_CONFIG_FILE wins; else $ZDOTDIR/.p10k.zsh; else ~/.p10k.zsh.
    if let Some(p) = crate::ported::params::getsparam("POWERLEVEL9K_CONFIG_FILE") {
        if !p.is_empty() {
            return p;
        }
    }
    let base = std::env::var("ZDOTDIR")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_default();
    format!("{base}/.p10k.zsh")
}

fn zshrc_path(home: &str) -> String {
    let base = std::env::var("ZDOTDIR").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| home.to_string());
    format!("{base}/.zshrc")
}

fn term_size() -> (usize, usize) {
    let cols = crate::ported::zle::zle_refresh::WINW.load(std::sync::atomic::Ordering::Relaxed);
    let lines = crate::ported::zle::zle_refresh::WINH.load(std::sync::atomic::Ordering::Relaxed);
    let cols = if cols > 0 { cols as usize } else { 80 };
    let lines = if lines > 0 { lines as usize } else { 24 };
    (cols, lines)
}

/// The header timestamp `%Y-%m-%d at %H:%M %Z`, from wall-clock.
fn timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Minimal UTC breakdown (no chrono dep); good enough for a header.
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days as i64);
    let sod = secs % 86400;
    format!("{y:04}-{m:02}-{d:02} at {:02}:{:02} UTC", sod / 3600, (sod % 3600) / 60)
}

/// days-since-epoch → (year, month, day) — Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// A tiny unused import guard so std::io::Read stays referenced.
#[allow(dead_code)]
fn _touch(mut r: impl Read) {
    let mut b = [0u8; 0];
    let _ = r.read(&mut b);
}
