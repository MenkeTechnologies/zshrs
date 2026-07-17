//! revolver — a progress spinner — ported to a native zshrs plugin.
//!
//! Upstream is a zsh script (`bin/revolver`, MenkeTechnologies/revolver, a
//! fork of molovo/revolver). This is a faithful port of its **behavior**,
//! not its mechanism.
//!
//! ## What changed and why
//!
//! The zsh original coordinates through the filesystem: `revolver start`
//! writes a statefile at `$REVOLVER_DIR/$PPID`, forks a disowned background
//! process (`_revolver_process $PPID &!`) that animates the spinner, and
//! `update`/`stop` rewrite/kill via that statefile. Each `revolver` call is
//! a separate external process, so `$PPID` + a statefile are the only way
//! they can find each other.
//!
//! zshrs is **non-forking** — a plugin builtin runs in-process and "has
//! nothing to background" (docs/PLUGINS.md). But it also does not *need* to
//! fork: `start`, `update`, and `stop` are now three calls into the same
//! resident process, so they share memory directly. The port therefore:
//!
//!   * spawns an in-process animator **thread** instead of forking a shell
//!     (`_revolver_process`'s `while` loop → [`animate`]);
//!   * keeps the running spinner in a `static` [`active`] slot instead of a
//!     `$PPID` statefile — no filesystem, no PID handshake, no orphan-check;
//!   * `update` writes a shared `Mutex<String>`; `stop` flips an
//!     `AtomicBool` and joins the thread.
//!
//! Observable behavior — the CLI, the 55 styles, the frames, the intervals,
//! the dim-grey message, clearing the line on stop — matches upstream.
//!
//! ## Command mapping (zsh fn → Rust)
//!
//! | zsh (`bin/revolver`)   | here                     |
//! | ---------------------- | ------------------------ |
//! | `revolver()` dispatch  | [`revolver`]             |
//! | `_revolver_usage`      | [`usage`]                |
//! | `_revolver_start`      | [`cmd_start`]            |
//! | `_revolver_process`/`_revolver_spin` | [`animate`] |
//! | `_revolver_update`     | [`cmd_update`]           |
//! | `_revolver_stop`       | [`cmd_stop`]             |
//! | `_revolver_demo`       | [`cmd_demo`]             |

use std::io::Write;
use std::os::raw::c_int;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use zshrs_plugin::{declare_plugin, Args, Host};

/// One spinner style: name, frame interval (seconds), animation frames.
/// Faithful transcription of `_revolver_spinners` in `bin/revolver`. The
/// `line`, `flip`, and `shark` frames are the *intended* ones — upstream's
/// double `${(@z)}` split mis-tokenises their backslashes/backticks; here
/// they are parsed with correct double-quote + `\\`→`\` handling.
type Style = (&'static str, f64, &'static [&'static str]);

#[rustfmt::skip]
static SPINNERS: &[Style] = &[
    ("dots", 0.08, &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    ("dots2", 0.08, &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"]),
    ("dots3", 0.08, &["⠋", "⠙", "⠚", "⠞", "⠖", "⠦", "⠴", "⠲", "⠳", "⠓"]),
    ("dots4", 0.08, &["⠄", "⠆", "⠇", "⠋", "⠙", "⠸", "⠰", "⠠", "⠰", "⠸", "⠙", "⠋", "⠇", "⠆"]),
    ("dots5", 0.08, &["⠋", "⠙", "⠚", "⠒", "⠂", "⠂", "⠒", "⠲", "⠴", "⠦", "⠖", "⠒", "⠐", "⠐", "⠒", "⠓", "⠋"]),
    ("dots6", 0.08, &["⠁", "⠉", "⠙", "⠚", "⠒", "⠂", "⠂", "⠒", "⠲", "⠴", "⠤", "⠄", "⠄", "⠤", "⠴", "⠲", "⠒", "⠂", "⠂", "⠒", "⠚", "⠙", "⠉", "⠁"]),
    ("dots7", 0.08, &["⠈", "⠉", "⠋", "⠓", "⠒", "⠐", "⠐", "⠒", "⠖", "⠦", "⠤", "⠠", "⠠", "⠤", "⠦", "⠖", "⠒", "⠐", "⠐", "⠒", "⠓", "⠋", "⠉", "⠈"]),
    ("dots8", 0.08, &["⠁", "⠁", "⠉", "⠙", "⠚", "⠒", "⠂", "⠂", "⠒", "⠲", "⠴", "⠤", "⠄", "⠄", "⠤", "⠠", "⠠", "⠤", "⠦", "⠖", "⠒", "⠐", "⠐", "⠒", "⠓", "⠋", "⠉", "⠈", "⠈"]),
    ("dots9", 0.08, &["⢹", "⢺", "⢼", "⣸", "⣇", "⡧", "⡗", "⡏"]),
    ("dots10", 0.08, &["⢄", "⢂", "⢁", "⡁", "⡈", "⡐", "⡠"]),
    ("dots11", 0.1, &["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"]),
    ("dots12", 0.08, &["⢀⠀", "⡀⠀", "⠄⠀", "⢂⠀", "⡂⠀", "⠅⠀", "⢃⠀", "⡃⠀", "⠍⠀", "⢋⠀", "⡋⠀", "⠍⠁", "⢋⠁", "⡋⠁", "⠍⠉", "⠋⠉", "⠋⠉", "⠉⠙", "⠉⠙", "⠉⠩", "⠈⢙", "⠈⡙", "⢈⠩", "⡀⢙", "⠄⡙", "⢂⠩", "⡂⢘", "⠅⡘", "⢃⠨", "⡃⢐", "⠍⡐", "⢋⠠", "⡋⢀", "⠍⡁", "⢋⠁", "⡋⠁", "⠍⠉", "⠋⠉", "⠋⠉", "⠉⠙", "⠉⠙", "⠉⠩", "⠈⢙", "⠈⡙", "⠈⠩", "⠀⢙", "⠀⡙", "⠀⠩", "⠀⢘", "⠀⡘", "⠀⠨", "⠀⢐", "⠀⡐", "⠀⠠", "⠀⢀", "⠀⡀"]),
    ("line", 0.13, &["-", "\\", "|", "/"]),
    ("line2", 0.1, &["⠂", "-", "–", "—", "–", "-"]),
    ("pipe", 0.1, &["┤", "┘", "┴", "└", "├", "┌", "┬", "┐"]),
    ("simpleDots", 0.4, &[".  ", ".. ", "...", "   "]),
    ("simpleDotsScrolling", 0.2, &[".  ", ".. ", "...", " ..", "  .", "   "]),
    ("star", 0.07, &["✶", "✸", "✹", "✺", "✹", "✷"]),
    ("star2", 0.08, &["+", "x", "*"]),
    ("flip", 0.07, &["_", "_", "_", "-", "`", "`", "'", "´", "-", "_", "_", "_"]),
    ("hamburger", 0.1, &["☱", "☲", "☴"]),
    ("growVertical", 0.12, &["▁", "▃", "▄", "▅", "▆", "▇", "▆", "▅", "▄", "▃"]),
    ("growHorizontal", 0.12, &["▏", "▎", "▍", "▌", "▋", "▊", "▉", "▊", "▋", "▌", "▍", "▎"]),
    ("balloon", 0.14, &[" ", ".", "o", "O", "@", "*", " "]),
    ("balloon2", 0.12, &[".", "o", "O", "°", "O", "o", "."]),
    ("noise", 0.14, &["▓", "▒", "░"]),
    ("bounce", 0.1, &["⠁", "⠂", "⠄", "⠂"]),
    ("boxBounce", 0.12, &["▖", "▘", "▝", "▗"]),
    ("boxBounce2", 0.1, &["▌", "▀", "▐", "▄"]),
    ("triangle", 0.05, &["◢", "◣", "◤", "◥"]),
    ("arc", 0.1, &["◜", "◠", "◝", "◞", "◡", "◟"]),
    ("circle", 0.12, &["◡", "⊙", "◠"]),
    ("squareCorners", 0.18, &["◰", "◳", "◲", "◱"]),
    ("circleQuarters", 0.12, &["◴", "◷", "◶", "◵"]),
    ("circleHalves", 0.05, &["◐", "◓", "◑", "◒"]),
    ("squish", 0.1, &["╫", "╪"]),
    ("toggle", 0.25, &["⊶", "⊷"]),
    ("toggle2", 0.08, &["▫", "▪"]),
    ("toggle3", 0.12, &["□", "■"]),
    ("toggle4", 0.1, &["■", "□", "▪", "▫"]),
    ("toggle5", 0.1, &["▮", "▯"]),
    ("toggle6", 0.3, &["ဝ", "၀"]),
    ("toggle7", 0.08, &["⦾", "⦿"]),
    ("toggle8", 0.1, &["◍", "◌"]),
    ("toggle9", 0.1, &["◉", "◎"]),
    ("toggle10", 0.1, &["㊂", "㊀", "㊁"]),
    ("toggle11", 0.05, &["⧇", "⧆"]),
    ("toggle12", 0.12, &["☗", "☖"]),
    ("toggle13", 0.08, &["=", "*", "-"]),
    ("arrow", 0.1, &["←", "↖", "↑", "↗", "→", "↘", "↓", "↙"]),
    ("arrow2", 0.12, &["▹▹▹▹▹", "▸▹▹▹▹", "▹▸▹▹▹", "▹▹▸▹▹", "▹▹▹▸▹", "▹▹▹▹▸"]),
    ("bouncingBar", 0.08, &["[    ]", "[   =]", "[  ==]", "[ ===]", "[====]", "[=== ]", "[==  ]", "[=   ]"]),
    ("bouncingBall", 0.08, &["( ●    )", "(  ●   )", "(   ●  )", "(    ● )", "(     ●)", "(    ● )", "(   ●  )", "(  ●   )", "( ●    )", "(●     )"]),
    ("pong", 0.08, &["▐⠂       ▌", "▐⠈       ▌", "▐ ⠂      ▌", "▐ ⠠      ▌", "▐  ⡀     ▌", "▐  ⠠     ▌", "▐   ⠂    ▌", "▐   ⠈    ▌", "▐    ⠂   ▌", "▐    ⠠   ▌", "▐     ⡀  ▌", "▐     ⠠  ▌", "▐      ⠂ ▌", "▐      ⠈ ▌", "▐       ⠂▌", "▐       ⠠▌", "▐       ⡀▌", "▐      ⠠ ▌", "▐      ⠂ ▌", "▐     ⠈  ▌", "▐     ⠂  ▌", "▐    ⠠   ▌", "▐    ⡀   ▌", "▐   ⠠    ▌", "▐   ⠂    ▌", "▐  ⠈     ▌", "▐  ⠂     ▌", "▐ ⠠      ▌", "▐ ⡀      ▌", "▐⠠       ▌"]),
    ("shark", 0.12, &["▐|\\____________▌", "▐_|\\___________▌", "▐__|\\__________▌", "▐___|\\_________▌", "▐____|\\________▌", "▐_____|\\_______▌", "▐______|\\______▌", "▐_______|\\_____▌", "▐________|\\____▌", "▐_________|\\___▌", "▐__________|\\__▌", "▐___________|\\_▌", "▐____________|\\▌", "▐____________/|▌", "▐___________/|_▌", "▐__________/|__▌", "▐_________/|___▌", "▐________/|____▌", "▐_______/|_____▌", "▐______/|______▌", "▐_____/|_______▌", "▐____/|________▌", "▐___/|_________▌", "▐__/|__________▌", "▐_/|___________▌", "▐/|____________▌"]),
];

/// Look up a style by name (`$_revolver_spinners[$style]` existence check).
fn find_style(name: &str) -> Option<&'static Style> {
    SPINNERS.iter().find(|s| s.0 == name)
}

// ─── Runtime state: the one running spinner (replaces the $PPID statefile) ──

/// A live spinner: the stop flag its thread polls, the shared message
/// `update` mutates, and the join handle `stop` waits on.
struct Spinner {
    stop: Arc<AtomicBool>,
    msg: Arc<Mutex<String>>,
    join: Option<JoinHandle<()>>,
}

/// The process-wide "currently animating" slot. The plugin cdylib stays
/// resident, so this persists across `start`/`update`/`stop` builtin calls —
/// the in-process analog of upstream's `$REVOLVER_DIR/$PPID` statefile.
fn active() -> &'static Mutex<Option<Spinner>> {
    static A: OnceLock<Mutex<Option<Spinner>>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(None))
}

/// Terminal width (`tput cols` in upstream), via `TIOCGWINSZ` on the tty.
fn term_cols() -> usize {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        for fd in [libc::STDERR_FILENO, libc::STDOUT_FILENO, libc::STDIN_FILENO] {
            if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return ws.ws_col as usize;
            }
        }
    }
    80
}

/// The animation loop — `_revolver_process`'s `while` plus `_revolver_spin`.
/// Runs on its own thread; writes straight to the terminal (`/dev/tty`, with
/// a stderr fallback) so a redirected stdout is never corrupted. Exits when
/// `stop` is set, then clears the line.
fn animate(interval: f64, frames: Vec<String>, msg: Arc<Mutex<String>>, stop: Arc<AtomicBool>) {
    let mut out: Box<dyn Write + Send> =
        match std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            Ok(f) => Box::new(f),
            Err(_) => Box::new(std::io::stderr()),
        };
    let n = frames.len().max(1);
    let delay = Duration::from_secs_f64(if interval > 0.0 { interval } else { 0.1 });
    let mut idx = 0usize;

    while !stop.load(Ordering::Relaxed) {
        let cols = term_cols();
        let mut m = msg.lock().unwrap().clone();
        // Keep the drawn line within the terminal so it never wraps
        // (upstream truncates via the `%*.*b` precision field).
        let budget = cols.saturating_sub(4);
        if m.chars().count() > budget {
            m = m.chars().take(budget).collect();
        }
        let frame = &frames[idx % n];
        // Clear the line (cols spaces + CR), then draw "frame  dim(msg)" and
        // return to column 0 (bin/revolver:142-152). msg is dim grey 242.
        let _ = write!(
            out,
            "\r{}\r{} \x1b[0;38;5;242m{}\x1b[0;m\r",
            " ".repeat(cols),
            frame,
            m
        );
        let _ = out.flush();
        idx = (idx + 1) % n;
        sleep_interruptible(&stop, delay);
    }

    // Clear the line on the way out (bin/revolver:181-183).
    let cols = term_cols();
    let _ = write!(out, "\r{}\r", " ".repeat(cols));
    let _ = out.flush();
}

/// Sleep `total`, but wake within ~20ms of `stop` being set so `revolver
/// stop` joins promptly instead of blocking for a full frame interval.
fn sleep_interruptible(stop: &AtomicBool, total: Duration) {
    let step = Duration::from_millis(20);
    let mut slept = Duration::ZERO;
    while slept < total {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let chunk = step.min(total - slept);
        thread::sleep(chunk);
        slept += chunk;
    }
}

/// Stop and join the active spinner, if any (shared by `stop`, `demo`, and
/// a re-`start` while one is already running).
fn stop_active() {
    let prev = active().lock().unwrap().take();
    if let Some(mut s) = prev {
        s.stop.store(true, Ordering::Relaxed);
        if let Some(j) = s.join.take() {
            let _ = j.join();
        }
    }
}

// ───────────────────────────── commands ────────────────────────────────

/// `_revolver_start` — spawn the animator for `style` with `msg`. Any spinner
/// already running is stopped first (upstream overwrites the statefile).
fn cmd_start(style: &'static Style, msg: &str) -> c_int {
    stop_active();
    let (_, interval, frames) = *style;
    let frames: Vec<String> = frames.iter().map(|s| s.to_string()).collect();
    let stop = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Mutex::new(msg.to_string()));
    let (ts, tm) = (stop.clone(), shared.clone());
    let join = thread::spawn(move || animate(interval, frames, tm, ts));
    *active().lock().unwrap() = Some(Spinner {
        stop,
        msg: shared,
        join: Some(join),
    });
    0
}

/// `_revolver_update` — swap the message the animator draws.
fn cmd_update(msg: &str) -> c_int {
    match active().lock().unwrap().as_ref() {
        Some(s) => {
            *s.msg.lock().unwrap() = msg.to_string();
            0
        }
        None => {
            eprintln!("\x1b[0;31mRevolver process could not be found\x1b[0;m");
            1
        }
    }
}

/// `_revolver_stop` — end the animation and clear the line.
fn cmd_stop() -> c_int {
    if active().lock().unwrap().is_none() {
        eprintln!("\x1b[0;31mRevolver process could not be found\x1b[0;m");
        return 1;
    }
    stop_active();
    0
}

/// `_revolver_demo` — animate each style for two seconds.
fn cmd_demo() -> c_int {
    for st in SPINNERS {
        cmd_start(st, st.0);
        thread::sleep(Duration::from_secs(2));
        stop_active();
    }
    0
}

/// `_revolver_usage`.
fn usage(host: &Host) {
    host.print(
        "\x1b[0;33mUsage:\x1b[0;m\n  revolver [options] <command> <message>\n\n\
         \x1b[0;33mOptions:\x1b[0;m\n  -h, --help         Output help text and exit\n  \
         -v, --version      Output version information and exit\n  \
         -s, --style        Set the spinner style\n\n\
         \x1b[0;33mCommands:\x1b[0;m\n  start <message>    Start the spinner\n  \
         update <message>   Update the message\n  stop               Stop the spinner\n  \
         demo               Display an demo of each style\n",
    );
}

/// The `revolver` builtin — `revolver()` dispatch in `bin/revolver`.
fn revolver(host: &Host, args: &Args) -> c_int {
    let argv = args.rest();
    let mut style: Option<String> = None;
    let mut positionals: Vec<String> = Vec::new();

    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        match a.as_str() {
            "-h" | "--help" => {
                usage(host);
                return 0;
            }
            "-v" | "--version" => {
                host.print("0.2.0\n");
                return 0;
            }
            "-s" | "--style" => {
                i += 1;
                if let Some(v) = argv.get(i) {
                    style = Some(v.clone());
                }
            }
            s if s.starts_with("--style=") => style = Some(s["--style=".len()..].to_string()),
            s if s.starts_with("-s") && s.len() > 2 => style = Some(s[2..].to_string()),
            _ => positionals.push(a.clone()),
        }
        i += 1;
    }

    // Default style, then validate it exists.
    let style = style.unwrap_or_else(|| "dots".to_string());
    let st = match find_style(&style) {
        Some(s) => s,
        None => {
            eprintln!("\x1b[0;31mSpinner '{style}' is not recognised\x1b[0;m");
            return 1;
        }
    };

    let ctx = positionals.first().map(|s| s.as_str()).unwrap_or("");
    let msg = if positionals.len() > 1 {
        positionals[1..].join(" ")
    } else {
        String::new()
    };

    match ctx {
        "start" => cmd_start(st, &msg),
        "update" => cmd_update(&msg),
        "stop" => cmd_stop(),
        "demo" => cmd_demo(),
        "" => {
            usage(host);
            1
        }
        other => {
            eprintln!("Command {other} is not recognised");
            1
        }
    }
}

declare_plugin! {
    name: "revolver",
    version: "0.2.0",
    builtins: {
        "revolver" => revolver,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table must carry every upstream style (bin/revolver defines 55).
    #[test]
    fn all_styles_present() {
        assert_eq!(SPINNERS.len(), 55);
        for name in ["dots", "line", "flip", "shark", "bouncingBall", "pong"] {
            assert!(find_style(name).is_some(), "missing style {name}");
        }
        assert!(find_style("does-not-exist").is_none());
    }

    /// No style may have an empty frame list or a non-positive interval —
    /// either would divide-by-zero or busy-loop the animator.
    #[test]
    fn every_style_is_animatable() {
        for (name, interval, frames) in SPINNERS {
            assert!(*interval > 0.0, "{name} has non-positive interval");
            assert!(!frames.is_empty(), "{name} has no frames");
        }
    }

    /// The three styles upstream's double `${(@z)}` mangles must carry the
    /// repaired frame sequences, not the mis-split ones.
    #[test]
    fn repaired_styles_have_correct_frames() {
        assert_eq!(find_style("line").unwrap().2, &["-", "\\", "|", "/"]);
        assert_eq!(
            find_style("flip").unwrap().2,
            &["_", "_", "_", "-", "`", "`", "'", "´", "-", "_", "_", "_"]
        );
        let shark = find_style("shark").unwrap().2;
        assert_eq!(shark.len(), 26);
        assert_eq!(shark[0], "▐|\\____________▌");
    }
}
