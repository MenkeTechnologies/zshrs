//! fish 4.9's terminal protocols, for the `--fish` drop-in.
//!
//! !!! RUST-ONLY: zsh has no counterpart. fish's own source is the spec
//! (vendor/fish): the startup and per-prompt queries (reader/reader.rs), the
//! keyboard protocols it turns on while a line is edited (tty_handoff.rs), the
//! line it abandons before a prompt (screen.rs) and the way it reads the key
//! reports those protocols produce (input_common.rs `parse_csi`).
//!
//! The protocols change what the terminal sends for a key: with the kitty
//! keyboard protocol Ctrl-C is `CSI 99;5u`, and with xterm's modifyOtherKeys a
//! key like Ctrl-Shift-letter is `CSI 27;6;…~`. ZLE's keymaps are keyed on the
//! legacy bytes, so a report is turned back into those bytes before the keymap
//! walk sees it, and a report with no legacy form is dropped, as fish drops a key
//! nothing is bound to.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};

/// The keyboard protocol fish picks (tty_handoff.rs `ProtocolKind`).
const KIND_UNSET: u8 = 0;
const KIND_KITTY: u8 = 1;
const KIND_OTHER: u8 = 2;
const KIND_WEZTERM: u8 = 3;
const KIND_NONE: u8 = 4;

static KIND: AtomicU8 = AtomicU8::new(KIND_UNSET);
/// tmux's version, when fish's quirks see tmux: (major, minor), 0 otherwise.
static TMUX: std::sync::Mutex<Option<(u32, u32)>> = std::sync::Mutex::new(None);
/// Whether the protocols are on (tty_handoff.rs `TTY_PROTOCOLS_ACTIVE`).
static ACTIVE: AtomicBool = AtomicBool::new(false);
static STARTED: std::sync::Once = std::sync::Once::new();
/// The terminal answered the startup queries' device attributes request.
static ANSWERS: AtomicBool = AtomicBool::new(false);
/// A prompt's queries are out and their device attributes reply has not
/// been read (fish `blocking_query`).
static PENDING: AtomicBool = AtomicBool::new(false);

/// fish `querying_allowed` (reader.rs:234-245).
fn querying_allowed() -> bool {
    crate::extensions::emulation_startup::fish_feature("query-term", true)
        && crate::ported::params::getsparam("TERM").as_deref() != Some("dumb")
        && crate::ported::params::getsparam("MC_SID").is_none()
        && unsafe { libc::isatty(libc::STDOUT_FILENO) } == 1
}

/// The startup queries (reader.rs:276-281): the kitty keyboard flags, the
/// terminal's name and version, its background colour, two XTGETTCAP
/// capabilities inside the alternate screen (reader.rs:3007-3021) and primary
/// device attributes, whose reply ends the wait.
fn startup_queries() -> String {
    let env = |name: &str| std::env::var(name).unwrap_or_default();
    let term = env("TERM");
    let mut q = String::new();
    // terminal.rs:168-178 — st answers the kitty query wrongly
    if term != "st-256color" {
        q.push_str("\x1b[?u");
    }
    q.push_str("\x1b[>0q");
    q.push_str("\x1b]11;?\x1b\\");
    let screen = !crate::ported::params::getsparam("STY").unwrap_or_default().is_empty()
        || matches!(
            crate::ported::params::getsparam("TERM").as_deref(),
            Some("screen" | "screen-256color")
        );
    if !screen {
        // `indn` (scroll content up) and fish's `query-os-name`, hex encoded
        q.push_str("\x1b[?1049h\x1bP+q696e646e\x1b\\\x1bP+q71756572792d6f732d6e616d65\x1b\\\x1b[?1049l");
    }
    q.push_str("\x1b[0c");
    q
}

/// The first prompt of an interactive `--fish` shell: send fish's startup
/// queries and wait for the device attributes reply, then settle which
/// keyboard protocol to use (tty_handoff.rs `TtyQuirks::detect` and
/// `get_supported_protocol`). Once per process.
pub fn init() {
    STARTED.call_once(|| {
        if querying_allowed() {
            crate::ported::zle::termquery::probe_terminal(
                &startup_queries(),
                crate::ported::zle::termquery::QUERY_STATES,
                crate::ported::zle::termquery::handle_query,
                &mut None,
            );
            ANSWERS.store(crate::ported::zle::termquery::PROBE_ANSWERED.load(Relaxed), Relaxed);
        }
        // the reply to XTVERSION, split by the query grammar into name and version
        let id = crate::ported::params::getsparam(".term.id").unwrap_or_default();
        let version = crate::ported::params::getsparam(".term.version").unwrap_or_default();
        let numbers = |v: &str| -> Vec<u32> { v.split('.').map_while(|n| n.parse().ok()).collect() };
        let kitty = crate::ported::params::getaparam(".term.extensions")
            .unwrap_or_default()
            .iter()
            .any(|e| e == "modkeys-kitty");
        let midnight_commander = crate::ported::params::getsparam("MC_SID").is_some()
            && crate::ported::params::getsparam("__mc_kitty_keyboard").is_none();
        let old_iterm2 = id == "iTerm2" && numbers(&version).get(..3).is_some_and(|v| v < &[3, 5, 12][..]);
        if id == "tmux" {
            if let [major, minor, ..] = numbers(&version)[..] {
                *TMUX.lock().unwrap() = Some((major, minor));
            }
        }
        let kind = if midnight_commander {
            KIND_NONE
        } else if old_iterm2 {
            KIND_OTHER
        } else if kitty {
            KIND_KITTY
        } else if id == "WezTerm" {
            KIND_WEZTERM
        } else {
            KIND_OTHER
        };
        KIND.store(kind, Relaxed);
    });
}

/// The commands that turn the protocols on or off (tty_handoff.rs
/// `get_protocols`): focus reporting under tmux, bracketed paste, colour theme
/// reports (not under tmux before 3.7), then the keyboard protocol.
fn protocol_commands(enable: bool) -> String {
    let tmux = *TMUX.lock().unwrap();
    let mut s = String::new();
    if tmux.is_some() {
        s.push_str(if enable { "\x1b[?1004h" } else { "\x1b[?1004l" });
    }
    s.push_str(if enable { "\x1b[?2004h" } else { "\x1b[?2004l" });
    if !tmux.is_some_and(|v| v < (3, 7)) {
        s.push_str(if enable { "\x1b[?2031h" } else { "\x1b[?2031l" });
    }
    s.push_str(match (KIND.load(Relaxed), enable) {
        (KIND_KITTY, true) => "\x1b[=5u",
        (KIND_KITTY, false) => "\x1b[=0u",
        (KIND_OTHER, true) => "\x1b[>4;1m\x1b=",
        (KIND_OTHER, false) => "\x1b[>4;0m\x1b>",
        (KIND_WEZTERM, true) => "\x1b=",
        (KIND_WEZTERM, false) => "\x1b>",
        _ => "",
    });
    s
}

fn write_tty(s: &str) {
    let fd = crate::ported::init::SHTTY.load(Relaxed);
    let _ = crate::ported::utils::write_loop(if fd >= 0 { fd } else { 1 }, s.as_bytes());
}

/// Before a prompt is drawn: at a new command line, fish's per-prompt queries
/// (background colour, cursor position, device attributes; reader.rs:2622 and
/// 1716-1745), whose replies the key reader drops; then the protocols on
/// (reader.rs:2629-2631).
pub fn before_prompt(new_line: bool) {
    init();
    let mut s = String::new();
    if new_line && querying_allowed() {
        s.push_str("\x1b]11;?\x1b\\\x1b[6n\x1b[0c");
        PENDING.store(ANSWERS.load(Relaxed), Relaxed);
    }
    if !ACTIVE.swap(true, Relaxed) {
        s.push_str(&protocol_commands(true));
    }
    if !s.is_empty() {
        write_tty(&s);
    }
}

/// Editing is over: the protocols off (reader.rs:2638), so the command reads
/// keys as the terminal normally sends them.
pub fn after_edit() {
    // fish runs no command while a query is out: the replies still on their
    // way are read here (keys typed meanwhile go back on the input), so none
    // of them reaches the command as input
    if PENDING.swap(false, Relaxed) {
        crate::ported::zle::termquery::probe_terminal(
            "",
            crate::ported::zle::termquery::QUERY_STATES,
            crate::ported::zle::termquery::handle_query,
            &mut None,
        );
    }
    if ACTIVE.swap(false, Relaxed) {
        write_tty(&protocol_commands(false));
    }
}

/// A complete CSI sequence the keymaps do not bind, as fish's `parse_csi`
/// reads it: `seq` is everything after `ESC [`, through the final byte.
/// Returns the legacy bytes the key stands for (empty when it has none or the
/// sequence is a reply to a query), to be read again in its place.
pub fn csi_input(seq: &[u8]) -> Vec<u8> {
    let Some((&last, body)) = seq.split_last() else { return Vec::new() };
    let (private, body) = match body.first() {
        Some(&p @ (b'?' | b'<' | b'=' | b'>')) => (Some(p), &body[1..]),
        _ => (None, body),
    };
    // parameters `;`-separated, each with up to three `:` sub-parameters
    let mut params = [[0u32; 4]; 16];
    let (mut count, mut sub) = (0usize, 0usize);
    for &c in body {
        match c {
            b'0'..=b'9' => {
                let p = &mut params[count][sub];
                *p = match p.checked_mul(10).and_then(|v| v.checked_add(u32::from(c - b'0'))) {
                    Some(v) => v,
                    None => return Vec::new(),
                };
            }
            b':' if sub < 3 => sub += 1,
            b';' if count < 15 => {
                count += 1;
                sub = 0;
            }
            _ => return Vec::new(),
        }
    }
    let mask = params[1][0].saturating_sub(1);
    if (last, private) == (b'c', Some(b'?')) {
        // the device attributes reply that ends a prompt's queries
        PENDING.store(false, Relaxed);
    }
    match (last, private) {
        // modifyOtherKeys: CSI 27 ; modifiers ; key ~
        (b'~', None) if params[0][0] == 27 => legacy(params[2][0], 0, mask),
        // the kitty keyboard protocol: CSI key[:shifted[:base]] ; modifiers u
        (b'u', None) => legacy(keypad(params[0][0]), params[0][1], mask),
        // everything else, query replies (`?…c`, `?…u`, `?997;…n`), cursor
        // position reports (`…R`), focus reports and keys with no binding
        _ => Vec::new(),
    }
}

/// fish reads the kitty keypad keys as their ordinary counterparts
/// (input_common.rs:1384-1414); the cursor and editing keys keep a code that
/// [`legacy`] writes in its xterm form.
fn keypad(code: u32) -> u32 {
    match code {
        57399..=57408 => u32::from(b'0') + (code - 57399),
        57409 => u32::from(b'.'),
        57410 => u32::from(b'/'),
        57411 => u32::from(b'*'),
        57412 => u32::from(b'-'),
        57413 => u32::from(b'+'),
        57414 => 13,
        57415 => u32::from(b'='),
        c => c,
    }
}

/// The bytes a terminal sends for `code` with `mask` (shift 1, alt 2, ctrl 4,
/// super 8, hyper 16, meta 32; caps and num lock above) without a keyboard
/// protocol, or nothing when there is no such form. A control character that
/// is the terminal's interrupt character raises SIGINT, as the terminal's line
/// discipline would.
fn legacy(code: u32, shifted: u32, mask: u32) -> Vec<u8> {
    let (shift, alt, ctrl) = (mask & 1 != 0, mask & 2 != 0, mask & 4 != 0);
    if mask & (8 | 16 | 32) != 0 {
        return Vec::new();
    }
    // the cursor and editing keys of the kitty keypad, in xterm's form
    let csi = |fin: &str| -> Vec<u8> {
        let m = mask & 7;
        let mut v = b"\x1b[".to_vec();
        match (fin.strip_suffix('~'), m) {
            (Some(n), 0) => v.extend_from_slice(format!("{n}~").as_bytes()),
            (Some(n), m) => v.extend_from_slice(format!("{n};{}~", m + 1).as_bytes()),
            (None, 0) => v.extend_from_slice(fin.as_bytes()),
            (None, m) => v.extend_from_slice(format!("1;{}{fin}", m + 1).as_bytes()),
        }
        v
    };
    match code {
        57417 => return csi("D"),
        57418 => return csi("C"),
        57419 => return csi("A"),
        57420 => return csi("B"),
        57421 => return csi("5~"),
        57422 => return csi("6~"),
        57423 => return csi("H"),
        57424 => return csi("F"),
        57425 => return csi("2~"),
        57426 => return csi("3~"),
        _ => {}
    }
    let mut c = code;
    if shift {
        if shifted != 0 {
            c = shifted;
        } else if let Some(u) = char::from_u32(c).filter(char::is_ascii_lowercase) {
            c = u32::from(u.to_ascii_uppercase());
        }
    }
    let mut out = Vec::new();
    match c {
        9 if shift => return b"\x1b[Z".to_vec(),
        13 | 9 | 27 => out.push(c as u8),
        127 => out.push(if ctrl { 0x08 } else { 0x7f }),
        _ if ctrl => {
            let Some(b) = ctrl_byte(c) else { return Vec::new() };
            out.push(b);
        }
        _ => match char::from_u32(c) {
            Some(ch) => out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes()),
            None => return Vec::new(),
        },
    }
    if alt {
        out.insert(0, 0x1b);
    } else if out.len() == 1 && interrupt_char() == Some(out[0]) {
        // SAFETY: kill(2) on our own pid, as the tty driver would signal us
        unsafe { libc::kill(libc::getpid(), libc::SIGINT) };
        return Vec::new();
    }
    out
}

/// The control character Ctrl plus `c` types (xterm's table).
fn ctrl_byte(c: u32) -> Option<u8> {
    let b = u8::try_from(c).ok()?;
    Some(match b {
        b'a'..=b'z' => b - 0x60,
        b'@'..=b'_' => b - 0x40,
        b' ' | b'2' => 0,
        b'3'..=b'7' => b - b'3' + 0x1b,
        b'8' | b'?' => 0x7f,
        b'/' => 0x1f,
        _ => return None,
    })
}

/// The terminal's interrupt character, while ISIG makes it one.
fn interrupt_char() -> Option<u8> {
    let t = crate::ported::utils::gettyinfo()?;
    (t.c_lflag & libc::ISIG != 0).then_some(t.c_cc[libc::VINTR] as u8)
}

/// `__fish_term_init`: [`init`], for the preamble's first prompt.
pub fn register_all() {
    crate::native_cmds::register("__fish_term_init", |_| {
        init();
        0
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_reports_read_as_their_legacy_bytes() {
        assert_eq!(csi_input(b"97;5u"), [0x01]); // ctrl-a
        assert_eq!(csi_input(b"97;3u"), b"\x1ba"); // alt-a
        assert_eq!(csi_input(b"97:65;2u"), b"A"); // shift-a, shifted key given
        assert_eq!(csi_input(b"27u"), [0x1b]); // escape
        assert_eq!(csi_input(b"13;3u"), b"\x1b\r"); // alt-enter
        assert_eq!(csi_input(b"9;2u"), b"\x1b[Z"); // shift-tab
        assert_eq!(csi_input(b"127;5u"), [0x08]); // ctrl-backspace
        assert_eq!(csi_input(b"57400u"), b"1"); // keypad 1
        assert_eq!(csi_input(b"57419;5u"), b"\x1b[1;5A"); // ctrl-keypad-up
        assert_eq!(csi_input(b"57426u"), b"\x1b[3~"); // keypad delete
        assert_eq!(csi_input(b"97;9u"), b""); // super-a: no legacy form
        assert_eq!(csi_input(b"233u"), "é".as_bytes());
    }

    #[test]
    fn modify_other_keys_reads_as_legacy_bytes() {
        assert_eq!(csi_input(b"27;5;105~"), [0x09]); // ctrl-i
        assert_eq!(csi_input(b"27;6;65~"), [0x01]); // ctrl-shift-a
        assert_eq!(csi_input(b"27;2;13~"), b"\r"); // shift-enter
    }

    #[test]
    fn replies_and_unknown_sequences_are_dropped() {
        assert_eq!(csi_input(b"?62;22c"), b""); // device attributes
        assert_eq!(csi_input(b"?0u"), b""); // kitty flags
        assert_eq!(csi_input(b"?997;1n"), b""); // colour theme
        assert_eq!(csi_input(b"12;40R"), b""); // cursor position
        assert_eq!(csi_input(b"I"), b""); // focus in
        assert_eq!(csi_input(b"1;5Q"), b""); // ctrl-F2, unbound
    }
}
