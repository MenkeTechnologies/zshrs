//! !!! RUST-ORIGINAL — no counterpart in `zsh/Src` !!!
//!
//! The terminfo parameterized-string evaluator (`tparm`), the termcap cursor
//! addresser (`tgoto`) and the padding writer (`tputs`) — the last three
//! ncurses entry points zshrs imported, alongside the database reader in
//! [`crate::terminfo_db`].
//!
//! `tparm` runs the stack language specified in `terminfo(5)` under "Parameterized
//! Strings". A capability such as `cup` is stored as
//! `\E[%i%p1%d;%p2%dH`, and evaluating it with `(row, col)` pushes the two
//! parameters, increments them for one-based terminals (`%i`) and prints them.
//!
//! Directives implemented, all of them from that specification:
//!
//! ```text
//!   %%              literal %
//!   %d %o %x %X %s  pop and print (printf-style width/precision honoured)
//!   %c              pop and emit as one byte
//!   %pN             push parameter N (1-based)
//!   %'c'            push the character literal c
//!   %{n}            push the decimal constant n
//!   %l              pop a string, push its length
//!   %PA %gA         set / get static variable A-Z, dynamic a-z
//!   %+ %- %* %/ %m  pop two, push arithmetic result
//!   %& %| %^        pop two, push bitwise result
//!   %= %< %>        pop two, push comparison result
//!   %A %O           pop two, push logical result
//!   %! %~           pop one, push logical / bitwise negation
//!   %i              increment the first two parameters
//!   %? %t %e %;     conditional
//! ```
//!
//! Division and modulo by zero yield `0` rather than trapping, matching
//! ncurses — a terminfo entry is untrusted input read off disk, and a
//! malformed one must not take the shell down.
//!
//! Two ncurses behaviours are reproduced because real entries depend on them,
//! and both are visible in `ncurses/tinfo/lib_tparm.c`:
//!
//!   * **The termcap hack** (`tparm_tc_compat`). A capability that never says
//!     `%p` is a pre-terminfo string whose conversions consume parameters
//!     positionally. ncurses pushes the parameters onto the stack in REVERSE
//!     (`for (i = num_parsed - 1; i >= 0; i--) npush(param[i])`) so that
//!     popping yields p1, p2, … in order, and it pushes at most two.
//!     `%i` then writes the incremented values back into stack slots 0 and 1
//!     — the BOTTOM of the stack, which after the reverse push holds p2 and
//!     p1 — which is why `%i%d.%d` prints p2+1 before p1+1 while a plain
//!     `%d.%d` prints p1 before p2. `u6` (`\E[%i%d;%dR`) is the common
//!     consumer.
//!   * **Unknown directives are dropped.** `lib_tparm.c`'s switch ends in
//!     `default: break;`, so `%z` and the `%\E` that several vendor `is2`
//!     strings carry emit nothing at all rather than the literal character.

use std::sync::{Mutex, OnceLock};

/// A `tparm` parameter, and the value type on its evaluation stack.
///
/// terminfo is nominally integer-only, but `%s` and `%l` need strings: the
/// `pfkey`/`pfloc`/`pfx`/`pln`/`pfxl` capabilities take a string argument,
/// which is why `echoti pfkey 1 foo` has to reach the evaluator intact.
#[derive(Debug, Clone)]
pub enum V {
    /// A numeric parameter.
    Int(i64),
    /// A string parameter, consumed by `%s` and measured by `%l`.
    Str(String),
}

impl V {
    fn int(&self) -> i64 {
        match self {
            V::Int(i) => *i,
            V::Str(s) => s.parse().unwrap_or(0),
        }
    }
    fn string(&self) -> String {
        match self {
            V::Int(i) => i.to_string(),
            V::Str(s) => s.clone(),
        }
    }
}

/// `%PA`-`%PZ` static variables persist across `tparm` calls, per
/// `terminfo(5)`: "the static variables survive between calls".
fn statics() -> &'static Mutex<[i64; 26]> {
    static S: OnceLock<Mutex<[i64; 26]>> = OnceLock::new();
    S.get_or_init(|| Mutex::new([0; 26]))
}

/// Format an integer the way `printf` would for the given terminfo directive,
/// honouring an optional `%[[:]flags][width[.precision]]` run.
fn fmt_int(spec: &str, conv: char, v: i64) -> String {
    let spec = spec.strip_prefix(':').unwrap_or(spec);
    let mut left = false;
    let mut zero = false;
    let mut plus = false;
    let mut space = false;
    let mut alt = false;
    let mut rest = spec;
    while let Some(c) = rest.chars().next() {
        match c {
            '-' => left = true,
            '0' => zero = true,
            '+' => plus = true,
            ' ' => space = true,
            '#' => alt = true,
            _ => break,
        }
        rest = &rest[c.len_utf8()..];
    }
    let (w, p) = match rest.split_once('.') {
        Some((a, b)) => (a.parse::<usize>().ok(), b.parse::<usize>().ok()),
        None => (rest.parse::<usize>().ok(), None),
    };

    // ncurses formats through `int`, not a 64-bit type: `save_number` writes
    // with a plain `%d`/`%x` and an `int` argument (lib_tparm.c). `%x` of -1
    // is therefore `ffffffff`, not sixteen f's — `acsc` on the Data General
    // entries is the capability that shows the difference.
    let n32 = v as i32;
    let mut body = match conv {
        'o' => format!("{:o}", n32 as u32),
        'x' => format!("{:x}", n32 as u32),
        'X' => format!("{:X}", n32 as u32),
        _ => (n32 as i64).abs().to_string(),
    };
    // printf precision on an integer conversion is a MINIMUM DIGIT COUNT and
    // applies to every base, not just `%d`. `hpa=\036FP%p1%2.2XFF` needs the
    // `%2.2X` to render 0 as `00`.
    if let Some(p) = p {
        while body.len() < p {
            body.insert(0, '0');
        }
    }
    if conv == 'd' {
        let sign = if v < 0 {
            "-"
        } else if plus {
            "+"
        } else if space {
            " "
        } else {
            ""
        };
        body = format!("{sign}{body}");
    } else if alt && v != 0 {
        let prefix = match conv {
            'o' => "0",
            'x' => "0x",
            'X' => "0X",
            _ => "",
        };
        body = format!("{prefix}{body}");
    }

    let Some(w) = w else { return body };
    if body.len() >= w {
        return body;
    }
    let pad = w - body.len();
    if left {
        format!("{body}{}", " ".repeat(pad))
    } else if zero && conv == 'd' && (v < 0 || plus || space) {
        // Zero padding goes AFTER the sign, not before it.
        let (sign, digits) = body.split_at(1);
        format!("{sign}{}{digits}", "0".repeat(pad))
    } else if zero {
        format!("{}{body}", "0".repeat(pad))
    } else {
        format!("{}{body}", " ".repeat(pad))
    }
}

/// Evaluate a terminfo capability string with the given parameters.
///
/// Byte-oriented in and out: terminfo strings carry raw escape bytes that are
/// not necessarily valid UTF-8, and `%c` can emit any byte at all.
pub fn tparm(cap: &[u8], params: &[i64]) -> Vec<u8> {
    let p: Vec<V> = params.iter().map(|&i| V::Int(i)).collect();
    tparm_params(cap, &p)
}

/// [`tparm`] with parameters that may be strings as well as integers.
pub fn tparm_params(cap: &[u8], params: &[V]) -> Vec<u8> {
    let mut p: Vec<V> = params.to_vec();
    let mut out: Vec<u8> = Vec::with_capacity(cap.len() + 8);
    let mut stack: Vec<V> = Vec::new();
    let mut dynamic = [0i64; 26];

    // `tparm_tc_compat`: a string with no `%p` is termcap-style, and its
    // parameters are pre-pushed in reverse so the first pop yields p1.
    // ncurses pushes `num_parsed` of them, which for this path is the
    // tgoto-like pair at most — a third conversion underflows to 0.
    let termcap_hack = !cap.windows(2).any(|w| w == b"%p");
    if termcap_hack {
        let n = count_conversions(cap).min(2);
        for i in (0..n).rev() {
            stack.push(p.get(i).cloned().unwrap_or(V::Int(0)));
        }
    }
    // `%i` applies once per evaluation (`incremented_two`).
    let mut incremented_two = false;

    let pop = |s: &mut Vec<V>| -> V { s.pop().unwrap_or(V::Int(0)) };
    let pop2 = |s: &mut Vec<V>| -> (i64, i64) {
        let b = s.pop().unwrap_or(V::Int(0)).int();
        let a = s.pop().unwrap_or(V::Int(0)).int();
        (a, b)
    };

    let mut i = 0usize;
    while i < cap.len() {
        if cap[i] != b'%' {
            out.push(cap[i]);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&c) = cap.get(i) else { break };

        // A printf-style flag/width run may precede the conversion char.
        if c == b':' || c.is_ascii_digit() || c == b'-' || c == b'+' || c == b' ' || c == b'#' || c == b'.' {
            let start = i;
            let mut j = i;
            if cap.get(j) == Some(&b':') {
                j += 1;
            }
            while let Some(&d) = cap.get(j) {
                if d.is_ascii_digit() || d == b'.' || d == b'-' || d == b'+' || d == b' ' || d == b'#' {
                    j += 1;
                } else {
                    break;
                }
            }
            if let Some(&conv) = cap.get(j) {
                if matches!(conv, b'd' | b'o' | b'x' | b'X' | b's') {
                    let spec = String::from_utf8_lossy(&cap[start..j]).into_owned();
                    let v = pop(&mut stack);
                    let s = if conv == b's' {
                        v.string()
                    } else {
                        fmt_int(&spec, conv as char, v.int())
                    };
                    out.extend_from_slice(s.as_bytes());
                    i = j + 1;
                    continue;
                }
            }
            // Not a formatted conversion after all. The run scanner treats
            // `-`, `+`, ` ` and `#` as printf flags, but those same bytes are
            // the arithmetic and logical operators when no conversion char
            // follows — `%-` in `setaf` is a subtraction, not a left-justify.
            // Fall through to the single-character directive match below.
        }

        i += 1;
        match c {
            b'%' => out.push(b'%'),
            b'd' | b'o' | b'x' | b'X' => {
                let v = pop(&mut stack).int();
                out.extend_from_slice(fmt_int("", c as char, v).as_bytes());
            }
            b's' => {
                let v = pop(&mut stack).string();
                out.extend_from_slice(v.as_bytes());
            }
            b'c' => {
                // ncurses `save_char` (lib_tparm.c): "if (c == 0) c = 0200".
                // The test is on the popped VALUE, not on its low byte, so a
                // value like 256 still writes a NUL — and since ncurses hands
                // back a C string, that NUL ends the result. `truncate_at_nul`
                // below reproduces the truncation.
                let v = pop(&mut stack).int();
                out.push(if v == 0 { 0o200 } else { (v & 0xff) as u8 });
            }
            b'p' => {
                if let Some(&n) = cap.get(i) {
                    i += 1;
                    let idx = (n as char).to_digit(10).unwrap_or(0) as usize;
                    if idx >= 1 {
                        stack.push(p.get(idx - 1).cloned().unwrap_or(V::Int(0)));
                    }
                }
            }
            b'\'' => {
                // `%'c'` — a character literal.
                if let Some(&ch) = cap.get(i) {
                    stack.push(V::Int(ch as i64));
                    i += 1;
                    if cap.get(i) == Some(&b'\'') {
                        i += 1;
                    }
                }
            }
            b'{' => {
                let start = i;
                while i < cap.len() && cap[i] != b'}' {
                    i += 1;
                }
                let n = String::from_utf8_lossy(&cap[start..i]);
                stack.push(V::Int(n.parse().unwrap_or(0)));
                if i < cap.len() {
                    i += 1; // consume `}`
                }
            }
            b'l' => {
                let v = pop(&mut stack).string();
                stack.push(V::Int(v.len() as i64));
            }
            b'P' => {
                if let Some(&n) = cap.get(i) {
                    i += 1;
                    let v = pop(&mut stack).int();
                    if n.is_ascii_uppercase() {
                        if let Ok(mut g) = statics().lock() {
                            g[(n - b'A') as usize] = v;
                        }
                    } else if n.is_ascii_lowercase() {
                        dynamic[(n - b'a') as usize] = v;
                    }
                }
            }
            b'g' => {
                if let Some(&n) = cap.get(i) {
                    i += 1;
                    let v = if n.is_ascii_uppercase() {
                        statics().lock().map(|g| g[(n - b'A') as usize]).unwrap_or(0)
                    } else if n.is_ascii_lowercase() {
                        dynamic[(n - b'a') as usize]
                    } else {
                        0
                    };
                    stack.push(V::Int(v));
                }
            }
            b'+' | b'-' | b'*' | b'/' | b'm' | b'&' | b'|' | b'^' | b'=' | b'<' | b'>' | b'A'
            | b'O' => {
                let (a, b) = pop2(&mut stack);
                let r = match c {
                    b'+' => a.wrapping_add(b),
                    b'-' => a.wrapping_sub(b),
                    b'*' => a.wrapping_mul(b),
                    // ncurses yields 0 rather than trapping; an entry read
                    // off disk must never abort the shell.
                    b'/' => {
                        if b == 0 {
                            0
                        } else {
                            a.wrapping_div(b)
                        }
                    }
                    b'm' => {
                        if b == 0 {
                            0
                        } else {
                            a.wrapping_rem(b)
                        }
                    }
                    b'&' => a & b,
                    b'|' => a | b,
                    b'^' => a ^ b,
                    b'=' => i64::from(a == b),
                    b'<' => i64::from(a < b),
                    b'>' => i64::from(a > b),
                    b'A' => i64::from(a != 0 && b != 0),
                    _ => i64::from(a != 0 || b != 0),
                };
                stack.push(V::Int(r));
            }
            b'!' => {
                let a = pop(&mut stack).int();
                stack.push(V::Int(i64::from(a == 0)));
            }
            b'~' => {
                let a = pop(&mut stack).int();
                stack.push(V::Int(!a));
            }
            b'i' => {
                // `%i` increments the first two parameters in place, and
                // under the termcap hack ALSO assigns them into stack slots
                // 0 and 1 — the bottom of the stack, per lib_tparm.c.
                if !incremented_two {
                    incremented_two = true;
                    for slot in p.iter_mut().take(2) {
                        *slot = V::Int(slot.int() + 1);
                    }
                    if termcap_hack {
                        for k in 0..2 {
                            if let (Some(v), Some(slot)) = (p.get(k), stack.get_mut(k)) {
                                *slot = V::Int(v.int());
                            }
                        }
                    }
                }
            }
            b'?' => { /* `if` — evaluation is driven by `%t` below. */ }
            b't' => {
                let cond = pop(&mut stack).int() != 0;
                if !cond {
                    i = skip_to_else_or_end(cap, i);
                }
            }
            b'e' => {
                // Reached only by falling out of a taken `%t` branch; skip
                // the else-part up to the matching `%;`.
                i = skip_to_end(cap, i);
            }
            b';' => { /* end of conditional */ }
            // lib_tparm.c `default: break;` — an unrecognized directive
            // emits nothing, and neither does the character after `%`.
            _ => {}
        }
    }
    // ncurses returns `char *`, so everything past an embedded NUL is
    // unreachable to every caller. `cup=\002%i%p1%c%p2%c` on a 256-column
    // terminal is the case that produces one.
    if let Some(z) = out.iter().position(|&b| b == 0) {
        out.truncate(z);
    }
    out
}

/// Number of value-consuming conversions in a capability, used only to size
/// the termcap-hack pre-push. Counts the same directives ncurses' analysis
/// does: `%d %o %x %X %s %c`, with or without a printf-style flag/width run.
fn count_conversions(cap: &[u8]) -> usize {
    let mut n = 0usize;
    let mut i = 0usize;
    while i + 1 < cap.len() {
        if cap[i] != b'%' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if cap.get(j) == Some(&b':') {
            j += 1;
        }
        while let Some(&d) = cap.get(j) {
            if d.is_ascii_digit() || d == b'.' || d == b'-' || d == b'+' || d == b' ' || d == b'#' {
                j += 1;
            } else {
                break;
            }
        }
        match cap.get(j) {
            Some(&c) if matches!(c, b'd' | b'o' | b'x' | b'X' | b's' | b'c') => {
                n += 1;
                i = j + 1;
            }
            // No conversion: step over `%` and ONE character only. Skipping to
            // `j + 1` swallowed the `%` of whatever followed, so the second
            // `%c` of `u6=\037%c%'A'%-%c%'A'%-` went uncounted and the
            // termcap pre-push was one parameter short.
            _ => i += 2,
        }
    }
    n
}

/// Advance past a not-taken `%t` branch, stopping AFTER the matching `%e` or
/// `%;`. Nested `%?` blocks are counted so an inner conditional does not
/// terminate the outer one.
fn skip_to_else_or_end(cap: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < cap.len() {
        if cap[i] != b'%' {
            i += 1;
            continue;
        }
        let Some(&c) = cap.get(i + 1) else { break };
        match c {
            b'?' => {
                depth += 1;
                i += 2;
            }
            b';' => {
                i += 2;
                if depth == 0 {
                    return i;
                }
                depth -= 1;
            }
            b'e' if depth == 0 => return i + 2,
            _ => i += 2,
        }
    }
    i
}

/// Advance past a taken branch's else-part, stopping after the matching `%;`.
fn skip_to_end(cap: &[u8], mut i: usize) -> usize {
    let mut depth = 0usize;
    while i < cap.len() {
        if cap[i] != b'%' {
            i += 1;
            continue;
        }
        let Some(&c) = cap.get(i + 1) else { break };
        match c {
            b'?' => {
                depth += 1;
                i += 2;
            }
            b';' => {
                i += 2;
                if depth == 0 {
                    return i;
                }
                depth -= 1;
            }
            _ => i += 2,
        }
    }
    i
}

/// ncurses `tgoto(cap, col, row)` — the TERMCAP cursor addresser.
///
/// Termcap uses a different, older encoding than terminfo. When the string
/// contains terminfo `%p` directives (which is what a terminfo-backed
/// `tgetstr` hands back on a modern system) evaluation is delegated to
/// [`tparm`] with the arguments in terminfo order (row, col).
pub fn tgoto(cap: &[u8], col: i64, row: i64) -> Vec<u8> {
    if cap.windows(2).any(|w| w == b"%p") {
        return tparm(cap, &[row, col]);
    }
    // Classic termcap: `%d`/`%2`/`%3`/`%.` consume the next argument, which
    // starts as the row and switches to the column, and `%r` swaps them.
    let mut args = [row, col];
    let mut next = 0usize;
    let mut out = Vec::with_capacity(cap.len() + 8);
    let mut i = 0usize;
    while i < cap.len() {
        if cap[i] != b'%' {
            out.push(cap[i]);
            i += 1;
            continue;
        }
        i += 1;
        let Some(&c) = cap.get(i) else { break };
        i += 1;
        let mut take = || {
            let v = args.get(next).copied().unwrap_or(0);
            next += 1;
            v
        };
        match c {
            b'%' => out.push(b'%'),
            b'd' => out.extend_from_slice(take().to_string().as_bytes()),
            b'2' => out.extend_from_slice(format!("{:02}", take() % 100).as_bytes()),
            b'3' => out.extend_from_slice(format!("{:03}", take() % 1000).as_bytes()),
            b'.' => out.push((take() & 0xff) as u8),
            b'+' => {
                let add = cap.get(i).copied().unwrap_or(0) as i64;
                i += 1;
                out.push(((take() + add) & 0xff) as u8);
            }
            b'>' => {
                // `%>xy` — conditional add: if value > x, add y.
                let x = cap.get(i).copied().unwrap_or(0) as i64;
                let y = cap.get(i + 1).copied().unwrap_or(0) as i64;
                i += 2;
                if args.get(next).copied().unwrap_or(0) > x {
                    if let Some(slot) = args.get_mut(next) {
                        *slot += y;
                    }
                }
            }
            b'r' => args.swap(0, 1),
            b'i' => {
                args[0] += 1;
                args[1] += 1;
            }
            b'n' => {
                args[0] ^= 0o140;
                args[1] ^= 0o140;
            }
            b'B' => {
                if let Some(slot) = args.get_mut(next) {
                    *slot = (*slot / 10) * 16 + *slot % 10;
                }
            }
            b'D' => {
                if let Some(slot) = args.get_mut(next) {
                    *slot -= 2 * (*slot % 16);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The terminal properties `tputs` needs in order to decide whether a delay
/// spec turns into pad bytes. All of them come from the entry loaded by
/// [`crate::terminfo_db`], plus the tty's output speed.
#[derive(Debug, Clone, Copy)]
pub struct PadInfo {
    /// `ospeed` from `tcgetattr`, in bits per second. `0` when the output is
    /// not a tty, which is also ncurses' "emit nothing" case.
    pub baud: i32,
    /// `xon` (`xon_xoff`) — the terminal does its own flow control, so
    /// timing pads are pointless.
    pub xon: bool,
    /// `pb` (`padding_baud_rate`) — below this speed padding is unnecessary.
    pub padding_baud_rate: i32,
    /// `npc` (`no_pad_char`) — pad with real time rather than with bytes.
    pub no_pad_char: bool,
    /// `pad` (`pad_char`), the byte to send; NUL when the entry has none.
    pub pad_char: u8,
}

impl Default for PadInfo {
    /// The "not a tty" case: no speed, so no pad bytes, which is exactly what
    /// ncurses emits when `ospeed` is 0.
    fn default() -> Self {
        PadInfo {
            baud: 0,
            xon: false,
            padding_baud_rate: 0,
            no_pad_char: false,
            pad_char: 0,
        }
    }
}

/// ncurses' `BAUDBYTE`: 9 bits per character — 7 data, 1 parity, 1 stop.
const BAUDBYTE: i64 = 9;

/// Expand a capability's `$<…>` delay specs, the transformation half of
/// ncurses' `tputs(str, affcnt, putc)`.
///
/// The spec is `$<` digits [`.` digit] [`*`] [`/`] `>`, where `*` multiplies
/// the delay by `affcnt` (the number of lines affected) and `/` makes the
/// pad mandatory even when the terminal has `xon`. ncurses accumulates the
/// delay in TENTHS of a millisecond (`number *= 10` after the integer part,
/// then one decimal digit is added) and passes `number / 10` to
/// `delay_output`, which emits
///
/// ```text
///   nullcount = (ms * baudrate) / (BAUDBYTE * 1000)
/// ```
///
/// copies of the pad character. Padding is emitted only when the delay is
/// positive AND the pad is mandatory or `normal_delay` holds — ncurses'
/// `!xon_xoff && padding_baud_rate && ospeed >= padding_baud_rate`. So an
/// entry with no `pb`, or output that is not a tty, produces no bytes at
/// all and the spec is simply removed. `npc` asks for a real sleep instead
/// of pad bytes; zshrs drops the delay rather than stalling the shell, which
/// is the one deliberate divergence here.
pub fn tputs(cap: &[u8], affcnt: i32, info: &PadInfo) -> Vec<u8> {
    let mut out = Vec::with_capacity(cap.len());
    let mut i = 0usize;
    while i < cap.len() {
        if cap[i] != b'$' {
            out.push(cap[i]);
            i += 1;
            continue;
        }
        if cap.get(i + 1) != Some(&b'<') {
            out.push(b'$');
            i += 1;
            continue;
        }
        // A `$<` that is not followed by a digit or `.`, or that never
        // closes, is literal text — ncurses emits both characters.
        let rest = &cap[i + 2..];
        let starts_ok = matches!(rest.first(), Some(c) if c.is_ascii_digit() || *c == b'.');
        if !starts_ok || !rest.contains(&b'>') {
            out.push(b'$');
            out.push(b'<');
            i += 2;
            continue;
        }
        let mut j = i + 2;
        let mut number: i64 = 0;
        while let Some(&c) = cap.get(j) {
            if !c.is_ascii_digit() {
                break;
            }
            number = number * 10 + i64::from(c - b'0');
            j += 1;
        }
        number *= 10; // tenths of a millisecond
        if cap.get(j) == Some(&b'.') {
            j += 1;
            if let Some(&c) = cap.get(j) {
                if c.is_ascii_digit() {
                    number += i64::from(c - b'0');
                    j += 1;
                }
            }
            while matches!(cap.get(j), Some(c) if c.is_ascii_digit()) {
                j += 1;
            }
        }
        let mut mandatory = false;
        while let Some(&c) = cap.get(j) {
            match c {
                b'*' => {
                    number *= i64::from(affcnt);
                    j += 1;
                }
                b'/' => {
                    mandatory = true;
                    j += 1;
                }
                _ => break,
            }
        }
        if cap.get(j) == Some(&b'>') {
            j += 1;
        }
        let normal_delay =
            !info.xon && info.padding_baud_rate > 0 && info.baud >= info.padding_baud_rate;
        if number > 0 && (normal_delay || mandatory) && !info.no_pad_char {
            let ms = number / 10;
            let nullcount = (ms * i64::from(info.baud)) / (BAUDBYTE * 1000);
            for _ in 0..nullcount {
                out.push(info.pad_char);
            }
        }
        i = j;
    }
    out
}

/// [`tputs`] for a caller with no terminal information: the delay specs are
/// removed and no pad bytes are produced, which is what ncurses does
/// whenever `ospeed` is 0 (any non-tty destination).
pub fn tputs_strip_padding(s: &[u8]) -> Vec<u8> {
    tputs(s, 1, &PadInfo::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(cap: &str, params: &[i64]) -> String {
        String::from_utf8_lossy(&tparm(cap.as_bytes(), params)).into_owned()
    }

    /// `cup` as xterm stores it — the single most-evaluated capability in the
    /// shell, and the one that exercises `%i` plus two `%p%d` prints.
    #[test]
    fn cup_addresses_the_cursor_one_based() {
        assert_eq!(t("\x1b[%i%p1%d;%p2%dH", &[0, 0]), "\x1b[1;1H");
        assert_eq!(t("\x1b[%i%p1%d;%p2%dH", &[23, 79]), "\x1b[24;80H");
    }

    /// `setaf` on xterm-256color: a three-way conditional with arithmetic,
    /// which covers `%?`/`%t`/`%e`/`%;`, `%{n}`, `%<` and `%-`.
    #[test]
    fn setaf_takes_each_branch_of_the_nested_conditional() {
        let cap = "\x1b[%?%p1%{8}%<%t3%p1%d%e%p1%{16}%<%t9%p1%{8}%-%d%e38;5;%p1%d%;m";
        assert_eq!(t(cap, &[1]), "\x1b[31m", "first branch: colours 0-7");
        assert_eq!(t(cap, &[9]), "\x1b[91m", "second branch: bright 8-15");
        assert_eq!(t(cap, &[200]), "\x1b[38;5;200m", "else branch: 256-colour");
    }

    #[test]
    fn arithmetic_comparison_and_logic_directives() {
        assert_eq!(t("%{6}%{7}%*%d", &[]), "42");
        assert_eq!(t("%{10}%{3}%m%d", &[]), "1");
        assert_eq!(t("%{12}%{10}%&%d", &[]), "8");
        assert_eq!(t("%{1}%{0}%O%d", &[]), "1");
        assert_eq!(t("%{1}%{0}%A%d", &[]), "0");
        assert_eq!(t("%{0}%!%d", &[]), "1");
        assert_eq!(t("%p1%p2%=%d", &[5, 5]), "1");
    }

    /// A malformed entry must degrade, never panic — these strings are read
    /// off disk from a file zshrs does not control.
    #[test]
    fn division_by_zero_and_truncated_directives_do_not_panic() {
        assert_eq!(t("%{5}%{0}%/%d", &[]), "0");
        assert_eq!(t("%{5}%{0}%m%d", &[]), "0");
        assert_eq!(t("%p", &[1]), "");
        assert_eq!(t("%{123", &[]), "");
        assert_eq!(t("%", &[]), "");
        assert_eq!(t("%?%p1%t", &[1]), "");
    }

    /// Static variables survive between calls (terminfo(5)); dynamic ones
    /// must not, or two unrelated capabilities would see each other's state.
    #[test]
    fn static_variables_persist_and_dynamic_ones_do_not() {
        assert_eq!(t("%{42}%PZ", &[]), "");
        assert_eq!(t("%gZ%d", &[]), "42");
        assert_eq!(t("%{7}%Pz", &[]), "");
        assert_eq!(t("%gz%d", &[]), "0", "dynamic vars reset every call");
    }

    #[test]
    fn width_and_precision_are_honoured() {
        assert_eq!(t("%p1%5d", &[42]), "   42");
        assert_eq!(t("%p1%-5d|", &[42]), "42   |");
        assert_eq!(t("%p1%05d", &[42]), "00042");
        assert_eq!(t("%p1%x", &[255]), "ff");
        assert_eq!(t("%p1%X", &[255]), "FF");
        assert_eq!(t("%p1%o", &[8]), "10");
    }

    #[test]
    fn char_literals_and_c_conversion() {
        assert_eq!(t("%'A'%c", &[]), "A");
        assert_eq!(t("%{65}%c", &[]), "A");
        assert_eq!(t("%p1%c", &[66]), "B");
    }

    /// Classic termcap `cm`, which uses `%.`/`%+` rather than `%p`.
    #[test]
    fn tgoto_handles_the_termcap_encoding() {
        let g = |c: &str, col: i64, row: i64| String::from_utf8_lossy(&tgoto(c.as_bytes(), col, row)).into_owned();
        assert_eq!(g("\x1b[%i%d;%dH", 0, 0), "\x1b[1;1H");
        assert_eq!(g("\x1b=%+ %+ ", 0, 0), "\x1b= \x20");
        // A terminfo-style string routed through tgoto still evaluates, with
        // the arguments in terminfo (row, col) order.
        assert_eq!(g("\x1b[%i%p1%d;%p2%dH", 5, 9), "\x1b[10;6H");
    }

    /// Measured against ncurses `tputs` on a non-tty (`ospeed` 0): every
    /// well-formed delay spec disappears and nothing is emitted in its place.
    #[test]
    fn padding_specs_are_stripped_but_literal_text_is_kept() {
        assert_eq!(tputs_strip_padding(b"\x1b[H$<20>"), b"\x1b[H".to_vec());
        assert_eq!(tputs_strip_padding(b"a$<5*/>b"), b"ab".to_vec());
        assert_eq!(tputs_strip_padding(b"\x1b[1m$<2>"), b"\x1b[1m".to_vec());
        assert_eq!(tputs_strip_padding(b"x$<5>y"), b"xy".to_vec());
        // `$<` that cannot be a delay is literal text and must survive.
        assert_eq!(
            tputs_strip_padding(b"cost $<x> here"),
            b"cost $<x> here".to_vec()
        );
        assert_eq!(tputs_strip_padding(b"unterminated $<20"), b"unterminated $<20".to_vec());
        assert_eq!(tputs_strip_padding(b"no padding"), b"no padding".to_vec());
        assert_eq!(tputs_strip_padding(b"a$b"), b"a$b".to_vec());
    }

    /// The pad-byte arithmetic: `nullcount = ms * baud / (9 * 1000)`, gated
    /// on `normal_delay` (`!xon && pb && ospeed >= pb`) or a `/`-mandatory
    /// spec.
    #[test]
    fn pad_bytes_follow_the_baud_rate_formula() {
        let info = PadInfo {
            baud: 38400,
            xon: false,
            padding_baud_rate: 1200,
            no_pad_char: false,
            pad_char: 0,
        };
        // 2ms at 38400 => 2 * 38400 / 9000 = 8 pad characters.
        assert_eq!(tputs(b"\x1b[1m$<2>", 1, &info), b"\x1b[1m\0\0\0\0\0\0\0\0".to_vec());
        // `*` multiplies by affcnt.
        assert_eq!(tputs(b"$<1*>", 4, &info).len(), 4 * 38400 / 9000);
        // A non-NUL pad character is honoured.
        let dot = PadInfo { pad_char: b'.', ..info };
        assert_eq!(tputs(b"$<2>", 1, &dot), b"........".to_vec());
    }

    /// Every gate that suppresses padding, each of which leaves the spec
    /// removed but emits nothing.
    #[test]
    fn padding_is_suppressed_by_xon_missing_pb_zero_baud_and_npc() {
        let base = PadInfo {
            baud: 38400,
            xon: false,
            padding_baud_rate: 1200,
            no_pad_char: false,
            pad_char: 0,
        };
        let cases = [
            ("xon", PadInfo { xon: true, ..base }),
            ("no pb", PadInfo { padding_baud_rate: 0, ..base }),
            ("baud below pb", PadInfo { baud: 300, ..base }),
            ("not a tty", PadInfo { baud: 0, ..base }),
            ("npc", PadInfo { no_pad_char: true, ..base }),
        ];
        for (why, info) in cases {
            assert_eq!(tputs(b"A$<2>B", 1, &info), b"AB".to_vec(), "{why}");
        }
        // …except a `/`-mandatory pad, which beats xon.
        let xon = PadInfo { xon: true, ..base };
        assert_eq!(tputs(b"A$<2/>B", 1, &xon).len(), 2 + 2 * 38400 / 9000);
    }
}
