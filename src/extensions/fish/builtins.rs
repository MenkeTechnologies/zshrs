//! fish builtins that are plain commands (they read arguments and stdin and write
//! stdout, with no access to the caller's variables): `string`, `math`, `count`,
//! `contains`. Registered as native commands when `--fish` is selected, so they
//! dispatch in-process like `cat` does.
//!
//! Behaviour is checked against fish 4.9: option spellings, output formats, exit
//! statuses. Regular expressions use `fancy-regex` (PCRE-style syntax, lookaround
//! and backreferences included), which is what fish's `-r` takes.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};

/// Register every fish command in the native-command table.
pub fn register_all() {
    for name in ["string", "math", "count", "contains", "path", "__fish_printf", "__fish_uvar_load", "__fish_uvar_write", "__fish_history_delete", "__fish_inherited", "__fish_rusage", "__fish_test"] {
        crate::native_cmds::register(name, move |argv| run(argv));
    }
}

/// Entry point: `argv[0]` names the command.
pub fn run(argv: &[String]) -> i32 {
    let args = &argv[1..];
    match argv[0].as_str() {
        "string" => string(args),
        "math" => math(args),
        "count" => count(args),
        "__fish_test" => test(args),
        "contains" => contains(args),
        "path" => path(args),
        "__fish_printf" => printf(args),
        "__fish_uvar_load" => uvar_load(),
        "__fish_history_delete" => history_delete(args),
        "__fish_rusage" => {
            // user and system microseconds of this shell, then of its waited-for children
            let t = |who| {
                let mut u: libc::rusage = unsafe { std::mem::zeroed() };
                unsafe { libc::getrusage(who, &mut u) };
                let us = |v: libc::timeval| v.tv_sec as i64 * 1_000_000 + v.tv_usec as i64;
                (us(u.ru_utime), us(u.ru_stime))
            };
            let (su, ss) = t(libc::RUSAGE_SELF);
            let (cu, cs) = t(libc::RUSAGE_CHILDREN);
            println!("{su} {ss} {cu} {cs}");
            0
        }
        "__fish_inherited" => match args.first().and_then(|n| super::inherited_value(n)) {
            Some(v) => {
                println!("{v}");
                0
            }
            None => 1,
        },
        "__fish_uvar_write" => uvar_write(args),
        _ => 127,
    }
}

fn err(cmd: &str, msg: &str) -> i32 {
    eprintln!("{cmd}: {msg}");
    2
}

/// Parsed options of one invocation.
#[derive(Default)]
struct Opts {
    flags: Vec<char>,
    vals: HashMap<char, String>,
    rest: Vec<String>,
    /// An explicit `--` ended the options: every operand after it is literal.
    ended: bool,
}

impl Opts {
    fn has(&self, c: char) -> bool {
        self.flags.contains(&c)
    }
    fn val(&self, c: char) -> Option<&str> {
        self.vals.get(&c).map(String::as_str)
    }
    fn num(&self, c: char) -> Option<i64> {
        self.val(c).and_then(|v| v.parse().ok())
    }
}

/// Options up to the first operand (or `--`). `valued` lists the short options that
/// take an argument; `long` maps long spellings onto short ones.
fn parse_opts(args: &[String], valued: &str, long: &[(&str, char)]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            i += 1;
            o.ended = true;
            break;
        }
        if let Some(l) = a.strip_prefix("--") {
            let (name, inline) = match l.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (l, None),
            };
            let Some(&(_, c)) = long.iter().find(|(n, _)| *n == name) else {
                return Err(format!("{a}: unknown option"));
            };
            if valued.contains(c) {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i).cloned().ok_or(format!("{a}: option requires an argument"))?
                    }
                };
                o.vals.insert(c, v);
            } else {
                o.flags.push(c);
            }
        } else if a.len() > 1 && a.starts_with('-') && !a[1..].starts_with(|c: char| c.is_ascii_digit()) {
            let cluster: Vec<char> = a[1..].chars().collect();
            let mut k = 0;
            while k < cluster.len() {
                let c = cluster[k];
                if valued.contains(c) {
                    let tail: String = cluster[k + 1..].iter().collect();
                    let v = if tail.is_empty() {
                        i += 1;
                        args.get(i).cloned().ok_or(format!("-{c}: option requires an argument"))?
                    } else {
                        tail
                    };
                    o.vals.insert(c, v);
                    break;
                }
                o.flags.push(c);
                k += 1;
            }
        } else {
            break;
        }
        i += 1;
    }
    o.rest = args[i..].to_vec();
    Ok(o)
}

/// The strings to work on: operands, or stdin lines when there are none.
fn inputs(operands: &[String], null_sep: bool) -> Vec<String> {
    if !operands.is_empty() {
        return operands.to_vec();
    }
    let mut out = Vec::new();
    let stdin = std::io::stdin();
    if null_sep {
        let mut buf = Vec::new();
        let _ = stdin.lock().read_to_end(&mut buf);
        for chunk in buf.split(|&b| b == 0).filter(|c| !c.is_empty()) {
            out.push(String::from_utf8_lossy(chunk).into_owned());
        }
        return out;
    }
    let mut lock = stdin.lock();
    let mut line = String::new();
    let mut unterminated = false;
    while lock.read_line(&mut line).unwrap_or(0) > 0 {
        unterminated = !line.ends_with('\n');
        if !unterminated {
            line.pop();
        }
        out.push(std::mem::take(&mut line));
    }
    STDIN_SHAPE.with(|s| s.set(Some((out.len(), unterminated))));
    out
}

/// `path` reads its operands from stdin when it has none, and takes NUL as the separator
/// when asked (`-z`) or when the input contains a NUL, newline otherwise.
fn inputs_detecting_nul(forced: bool) -> Vec<String> {
    let mut buf = Vec::new();
    let _ = std::io::stdin().lock().read_to_end(&mut buf);
    let sep = if forced || buf.contains(&0) { 0u8 } else { b'\n' };
    let mut items: Vec<&[u8]> = buf.split(|&b| b == sep).collect();
    if items.last().is_some_and(|l| l.is_empty()) {
        items.pop();
    }
    items.iter().map(|i| String::from_utf8_lossy(i).into_owned()).collect()
}

thread_local! {
    /// What the last read of stdin saw: the number of lines and whether the final one lacked
    /// its newline. `None` when stdin was not read.
    static STDIN_SHAPE: std::cell::Cell<Option<(usize, bool)>> = const { std::cell::Cell::new(None) };
}

/// fish's `string`, `count`, `math` and `path` take their input from stdin only when their
/// own stdin is redirected: not first in the pipeline, or a `<` of their own (exec.rs
/// `get_performer_for_builtin`, builtins/shared/misc.rs `Arguments::new`). The translator
/// marks such a command with `__fish_stdin=1`; otherwise stdin is not read at all, even
/// with no operands, so `(count)` with a pipe for stdin does not wait on it.
fn stdin_redirected() -> bool {
    crate::ported::params::getsparam("__fish_stdin").as_deref() == Some("1")
}

/// The strings a `string` subcommand works on. fish reads them from stdin when the
/// command's stdin is redirected and from the operands otherwise, never both and
/// regardless of whether there are operands (builtins/shared/misc.rs `Arguments::new`).
fn string_inputs(operands: &[String]) -> Vec<String> {
    if stdin_redirected() {
        inputs(&[], false)
    } else {
        operands.to_vec()
    }
}

struct Out {
    quiet: bool,
    buf: Vec<u8>,
    nul: bool,
}

impl Out {
    fn new(quiet: bool, nul: bool) -> Self {
        Out {
            quiet,
            buf: Vec::new(),
            nul,
        }
    }
    fn line(&mut self, s: &str) {
        self.line_bytes(s.as_bytes());
    }
    fn line_bytes(&mut self, s: &[u8]) {
        if !self.quiet {
            self.buf.extend_from_slice(s);
            self.buf.push(if self.nul { 0 } else { b'\n' });
        }
    }
    /// fish ends the output of an item that came from an unterminated stdin line the same way.
    fn drop_final_newline(&mut self) {
        if self.buf.last() == Some(&b'\n') {
            self.buf.pop();
        }
    }
    fn flush(self) {
        let mut so = std::io::stdout().lock();
        let _ = so.write_all(&self.buf);
        let _ = so.flush();
    }
}

// ---- string ----------------------------------------------------------------------

const STRING_LONG: &[(&str, char)] = &[
    ("quiet", 'q'),
    ("all", 'a'),
    ("regex", 'r'),
    ("entire", 'e'),
    ("invert", 'v'),
    ("ignore-case", 'i'),
    ("groups-only", 'g'),
    ("index", 'n'),
    ("filter", 'f'),
    ("start", 's'),
    ("end", 'e'),
    ("length", 'l'),
    ("width", 'w'),
    ("char", 'c'),
    ("count", 'n'),
    ("max", 'm'),
    ("right", 'r'),
    ("left", 'l'),
    ("no-empty", 'n'),
    ("fields", 'f'),
    ("style", 's'),
    ("visible", 'V'),
    ("null-in", 'z'),
    ("no-trim-newlines", 'N'),
    ("allow-empty", 'a'),
    ("ellipsis", 'c'),
];

fn string(args: &[String]) -> i32 {
    STDIN_SHAPE.with(|s| s.set(None));
    let Some(sub) = args.first() else {
        return err("string", "missing subcommand");
    };
    let rest = &args[1..];
    // option letters differ per subcommand (`-r` is regex for match, right for pad)
    let (valued, long): (&str, Vec<(&str, char)>) = match sub.as_str() {
        "sub" => ("sel", vec![("start", 's'), ("end", 'e'), ("length", 'l'), ("quiet", 'q')]),
        "pad" => ("wc", vec![("width", 'w'), ("char", 'c'), ("right", 'r'), ("quiet", 'q'), ("chars", 'c'), ("center", 'C')]),
        "trim" => ("c", vec![("chars", 'c'), ("left", 'l'), ("right", 'r'), ("quiet", 'q')]),
        "repeat" => ("nm", vec![("count", 'n'), ("max", 'm'), ("quiet", 'q'), ("no-newline", 'N')]),
        "split" => ("mf", vec![("max", 'm'), ("fields", 'f'), ("right", 'r'), ("no-empty", 'n'), ("quiet", 'q'), ("allow-empty", 'a')]),
        "shorten" => ("mc", vec![("max", 'm'), ("char", 'c'), ("chars", 'c'), ("left", 'l'), ("no-newline", 'N'), ("quiet", 'q')]),
        "escape" | "unescape" => ("s", vec![("style", 's'), ("no-quote", 'n')]),
        "join" | "join0" => ("", [STRING_LONG, &[("no-empty", 'n')][..]].concat()),
        "match" => ("", vec![("all", 'a'), ("entire", 'e'), ("regex", 'r'), ("invert", 'v'), ("ignore-case", 'i'), ("groups-only", 'g'), ("index", 'n'), ("quiet", 'q')]),
        "replace" => ("m", vec![("all", 'a'), ("regex", 'r'), ("ignore-case", 'i'), ("filter", 'f'), ("quiet", 'q'), ("max-matches", 'm')]),
        _ => ("", STRING_LONG.to_vec()),
    };
    let o = match parse_opts(rest, valued, &long) {
        Ok(o) => o,
        Err(e) => return err(&format!("string {sub}"), &e),
    };
    // `string join / -- a b`: fish takes `--` after the separator too
    let o = if matches!(sub.as_str(), "join" | "join0" | "pad" | "length" | "sub" | "trim" | "upper" | "lower" | "repeat" | "escape" | "unescape" | "shorten" | "collect") {
        without_dashdash(&o)
    } else {
        o
    };
    // with stdin redirected, operands beyond the subcommand's own leading ones (the
    // separator, pattern, count…) are an error (string.rs `run_impl`, TOO_MANY_ARGUMENTS)
    let strin = stdin_redirected();
    let leading = match sub.as_str() {
        "split" | "join" | "match" => 1,
        "replace" => 2,
        "repeat" if o.val('n').is_none() && o.val('m').is_none() => 1,
        _ => 0,
    };
    if strin && without_dashdash(&o).rest.len() > leading {
        return err(&format!("string {sub}"), "too many arguments");
    }
    let quiet = o.has('q');
    let mut out = Out::new(quiet, o.has('z') && matches!(sub.as_str(), "split0" | "join0"));
    let status = match sub.as_str() {
        "length" => {
            let mut any = false;
            for s in string_inputs(&o.rest) {
                if !o.has('V') {
                    let n = s.chars().count();
                    any |= n > 0;
                    out.line(&n.to_string());
                    continue;
                }
                // length.rs -V: a width per line, the widest stretch between carriage
                // returns, escape sequences taking none
                for line in s.split('\n') {
                    let n = line.split('\r').map(|r| width_without_escapes(&r.chars().collect::<Vec<_>>(), 0)).max().unwrap_or(0);
                    any |= n > 0;
                    out.line(&n.to_string());
                }
            }
            i32::from(!any)
        }
        "upper" | "lower" => {
            // (transform.rs: the status counts the arguments the case change altered)
            let up = sub == "upper";
            let mut changed = 0usize;
            for s in string_inputs(&o.rest) {
                let t = if up { s.to_uppercase() } else { s.to_lowercase() };
                changed += usize::from(t != s);
                out.line(&t);
                if quiet && changed > 0 {
                    break;
                }
            }
            i32::from(changed == 0)
        }
        "sub" => str_sub(&o, &mut out),
        "pad" => str_pad(&o, &mut out),
        "trim" => str_trim(&o, &mut out),
        "repeat" => str_repeat(&o, &mut out),
        "join" | "join0" => {
            let (sep, items): (String, Vec<String>) = if sub == "join" {
                match o.rest.split_first() {
                    Some((s, r)) => (s.clone(), string_inputs(r)),
                    None => return err("string join", "missing argument"),
                }
            } else {
                ("\0".into(), string_inputs(&o.rest))
            };
            let items: Vec<String> = if o.has('n') { items.into_iter().filter(|s| !s.is_empty()).collect() } else { items };
            let joined = items.join(&sep);
            if sub == "join0" {
                if !quiet && !items.is_empty() {
                    let mut so = std::io::stdout().lock();
                    let _ = so.write_all(joined.as_bytes());
                    let _ = so.write_all(b"\0");
                }
            } else if !items.is_empty() {
                // (join.rs: no arguments, no line)
                out.line(&joined);
            }
            i32::from(items.len() < 2)
        }
        "split" | "split0" => str_split(&o, &mut out, sub == "split0"),
        "collect" if strin => {
            // stdin is one value, written back without a newline; trailing newlines go
            // unless -N keeps them
            let mut buf = String::new();
            let _ = std::io::stdin().lock().read_to_string(&mut buf);
            let kept = if o.has('N') { buf.as_str() } else { buf.trim_end_matches('\n') };
            if !quiet {
                let mut so = std::io::stdout().lock();
                let _ = so.write_all(kept.as_bytes());
                // -a: nothing came in, so an empty line goes out (collect.rs)
                if o.has('a') && kept.is_empty() {
                    let _ = so.write_all(b"\n");
                }
            }
            i32::from(kept.is_empty())
        }
        "collect" => {
            // collect.rs: each operand loses its trailing newlines (unless -N) and ends
            // its own line; the status says whether any character was written
            let mut appended = 0usize;
            for s in &o.rest {
                let kept = if o.has('N') { s.as_str() } else { s.trim_end_matches('\n') };
                out.line(kept);
                appended += kept.len();
            }
            if o.has('a') && appended == 0 {
                out.line("");
            }
            i32::from(appended == 0)
        }
        "match" => str_match(&o, &mut out),
        "replace" => str_replace(&o, &mut out),
        "escape" => {
            // (escape.rs: status 1 when there was nothing to escape)
            let style = o.val('s').unwrap_or("script");
            let mut any = false;
            for s in string_inputs(&o.rest) {
                out.line(&escape(&s, style));
                any = true;
            }
            i32::from(!any)
        }
        "unescape" => {
            // (unescape.rs: an argument that does not decode is skipped; status 1 when
            // none did)
            let style = o.val('s').unwrap_or("script");
            let mut any = false;
            for s in string_inputs(&o.rest) {
                let u = if style == "script" { unescape_script(&s) } else { unescape(&s, style).map(String::into_bytes) };
                if let Some(u) = u {
                    out.line_bytes(&u);
                    any = true;
                }
            }
            i32::from(!any)
        }
        "shorten" => str_shorten(&o, &mut out),
        other => {
            return err("string", &format!("{other}: invalid subcommand"));
        }
    };
    if let Some((lines, unterminated)) = STDIN_SHAPE.with(|s| s.take()) {
        let mirrors = matches!(sub.as_str(), "upper" | "lower" | "trim" | "escape" | "unescape" | "collect" | "replace" | "sub" | "pad" | "join" | "repeat");
        if unterminated && mirrors {
            out.drop_final_newline();
        }
        // pad loses every newline when the last line lacked one
        if unterminated && sub == "pad" {
            out.buf.retain(|&b| b != b'\n');
        }
        // nothing came in, so nothing goes out
        if lines == 0 && matches!(sub.as_str(), "collect" | "join" | "join0") {
            return status;
        }
    }
    out.flush();
    status
}

/// fish accepts `--` between a subcommand's own operands (`string split , -- $x`), not
/// only before them.
fn without_dashdash(o: &Opts) -> Opts {
    Opts {
        flags: o.flags.clone(),
        vals: o.vals.clone(),
        // after an explicit terminator a `--` is just an operand
        rest: o.rest.iter().filter(|a| o.ended || a.as_str() != "--").cloned().collect(),
        ended: o.ended,
    }
}

fn str_sub(o: &Opts, out: &mut Out) -> i32 {
    let mut any = false;
    for s in string_inputs(&o.rest) {
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len() as i64;
        let start = match o.num('s') {
            Some(s) if s < 0 => (n + s + 1).max(1),
            Some(s) => s.max(1),
            None => 1,
        };
        let end = match (o.num('e'), o.num('l')) {
            (Some(e), _) if e < 0 => (n + e).max(0),
            (Some(e), _) => e.min(n),
            (None, Some(l)) => (start - 1 + l).min(n),
            (None, None) => n,
        };
        let piece: String = if start > end || start > n {
            String::new()
        } else {
            chars[(start - 1) as usize..end as usize].iter().collect()
        };
        out.line(&piece);
        any = true;
    }
    i32::from(!any)
}

fn str_pad(o: &Opts, out: &mut Out) -> i32 {
    let items = string_inputs(&o.rest);
    let fill = o.val('c').and_then(|c| c.chars().next()).unwrap_or(' ');
    let width = o
        .num('w')
        .map(|w| w.max(0) as usize)
        .unwrap_or(0)
        .max(items.iter().map(|s| s.chars().count()).max().unwrap_or(0));
    for s in &items {
        let total = width.saturating_sub(s.chars().count());
        // pad.rs: -C splits the padding, the odd column on the side -r does not name
        let (left, right) = match (o.has('r'), o.has('C')) {
            (false, false) => (total, 0),
            (true, false) => (0, total),
            (false, true) => (total - total / 2, total / 2),
            (true, true) => (total / 2, total - total / 2),
        };
        let p = |n| std::iter::repeat_n(fill, n).collect::<String>();
        out.line(&format!("{}{s}{}", p(left), p(right)));
    }
    0
}

fn str_trim(o: &Opts, out: &mut Out) -> i32 {
    let set: Vec<char> = o.val('c').map_or(vec![' ', '\t', '\n', '\r', '\x0c', '\x0b'], |c| c.chars().collect());
    let (left, right) = (o.has('l') || !o.has('r'), o.has('r') || !o.has('l'));
    let mut trimmed = false;
    for s in string_inputs(&o.rest) {
        let mut t: &str = &s;
        if left {
            t = t.trim_start_matches(|c| set.contains(&c));
        }
        if right {
            t = t.trim_end_matches(|c| set.contains(&c));
        }
        trimmed |= t.len() != s.len();
        out.line(t);
    }
    i32::from(!trimmed)
}

fn str_repeat(o: &Opts, out: &mut Out) -> i32 {
    // repeat.rs: the count is `-n`, or the first operand when neither -n nor -m is given;
    // `-m` caps the output at that many characters and with no count repeats up to it
    let parse = |v: &str, what: &str| v.trim_start().parse::<i64>().ok().filter(|n| *n >= 0).map(|n| n as usize).ok_or(format!("Invalid {what} value '{v}'"));
    let (mut count, mut max) = (0usize, 0usize);
    let mut operands: &[String] = &o.rest;
    if o.val('n').is_none() && o.val('m').is_none() {
        let Some((first, rest)) = o.rest.split_first() else {
            return err("string repeat", "missing argument");
        };
        match parse(first, "count") {
            Ok(n) => count = n,
            Err(e) => return err("string repeat", &e),
        }
        operands = rest;
    }
    if let Some(v) = o.val('n') {
        match parse(v, "count") {
            Ok(n) => count = n,
            Err(e) => return err("string repeat", &e),
        }
    }
    if let Some(v) = o.val('m') {
        match parse(v, "max") {
            Ok(n) => max = n,
            Err(e) => return err("string repeat", &e),
        }
    }
    if max == 0 && count == 0 {
        return 1;
    }
    let mut all_empty = true;
    let mut first = true;
    for s in string_inputs(operands) {
        if s.is_empty() {
            continue;
        }
        all_empty = false;
        if o.has('q') {
            return 0;
        }
        if !first {
            out.buf.push(b'\n');
        }
        first = false;
        let len = s.chars().count();
        let total = if max == 0 || (count > 0 && len.saturating_mul(count) < max) { len.saturating_mul(count) } else { max };
        let piece: String = s.chars().cycle().take(total).collect();
        out.buf.extend_from_slice(piece.as_bytes());
    }
    // no newline at all when every string was empty
    if !o.has('q') && !o.has('N') && !all_empty {
        out.buf.push(b'\n');
    }
    i32::from(all_empty)
}

/// `string split -f` fields: comma-separated one-based numbers and `a-b` spans, a span
/// running backwards when `a > b` (split.rs `Fields::try_from`). Zero-based indices.
fn split_fields(spec: &str) -> Result<Vec<usize>, String> {
    let mut indices = Vec::new();
    for f in spec.split(',') {
        let num = |s: &str| s.trim_start().parse::<i64>().map_err(|_| format!("{spec}: invalid integer"));
        match f.split('-').collect::<Vec<_>>()[..] {
            [a, b] => {
                let (a, b) = (num(a)?, num(b)?);
                if a <= 0 || b <= 0 {
                    return Err(format!("Invalid range value for field '{spec}'"));
                }
                let (a, b) = (a as usize, b as usize);
                if a <= b {
                    indices.extend(a - 1..b);
                } else {
                    indices.extend((b - 1..a).rev());
                }
            }
            _ => match num(f)? {
                n @ 1.. => indices.push(n as usize - 1),
                _ => return Err(format!("Invalid fields value '{spec}'")),
            },
        }
    }
    Ok(indices)
}

fn str_split(o: &Opts, out: &mut Out, nul: bool) -> i32 {
    let o = &without_dashdash(o);
    let (sep, items) = if nul {
        // fish reads split0's stdin whole (`SplitBehavior::Never`) and splits it on NUL
        let items = if stdin_redirected() {
            let mut buf = Vec::new();
            let _ = std::io::stdin().lock().read_to_end(&mut buf);
            if buf.is_empty() { Vec::new() } else { vec![String::from_utf8_lossy(&buf).into_owned()] }
        } else {
            o.rest.clone()
        };
        ("\0".to_string(), items)
    } else {
        match o.rest.split_first() {
            Some((s, r)) => (s.clone(), string_inputs(r)),
            None => return err("string split", "missing argument"),
        }
    };
    let fields = match o.val('f').map(split_fields).transpose() {
        Ok(f) => f.unwrap_or_default(),
        Err(e) => return err(&format!("string split{}", if nul { "0" } else { "" }), &e),
    };
    if fields.is_empty() && o.has('a') {
        return err("string split", "--allow-empty is only valid with --fields");
    }
    let max = o.num('m').map_or(usize::MAX, |m| m.max(0) as usize);
    let (mut split_count, arg_count) = (0usize, items.len());
    let mut all_splits: Vec<Vec<String>> = Vec::new();
    for s in items {
        let mut parts: Vec<String> = if sep.is_empty() {
            // an empty separator splits between characters; `-m N` stops after N of them
            let chars: Vec<char> = s.chars().collect();
            let cut = max.min(chars.len());
            let mut v: Vec<String> = if o.has('r') {
                let keep = chars.len() - cut;
                let mut v = vec![chars[..keep].iter().collect::<String>()];
                v.extend(chars[keep..].iter().map(|c| c.to_string()));
                v
            } else {
                let mut v: Vec<String> = chars[..cut].iter().map(|c| c.to_string()).collect();
                v.push(chars[cut..].iter().collect());
                v
            };
            v.retain(|p| !p.is_empty() || chars.is_empty());
            v
        } else if o.has('r') {
            let mut v: Vec<String> = s.rsplitn(max.saturating_add(1), sep.as_str()).map(str::to_string).collect();
            v.reverse();
            v
        } else {
            s.splitn(max.saturating_add(1), sep.as_str()).map(str::to_string).collect()
        };
        if o.has('n') {
            parts.retain(|p| !p.is_empty());
        }
        // -q is done as soon as one argument split
        if o.has('q') && parts.len() > 1 {
            return 0;
        }
        split_count += parts.len();
        all_splits.push(parts);
    }
    for mut parts in all_splits {
        // split0 ignores a trailing NUL: `a\0b\0` is two elements, `a\0\0` is `a` and ``
        if nul && parts.last().is_some_and(String::is_empty) {
            parts.pop();
        }
        if fields.is_empty() {
            for p in &parts {
                out.line(p);
            }
            continue;
        }
        // an argument that lacks one of the fields ends the command with status 1, the
        // arguments before it printed (split.rs: unless --allow-empty)
        if !o.has('a') && fields.iter().any(|&f| f >= parts.len()) {
            return 1;
        }
        for p in fields.iter().filter_map(|&f| parts.get(f)) {
            out.line(p);
        }
    }
    // something was split when there are more pieces than arguments
    i32::from(split_count <= arg_count)
}

fn compile(pattern: &str, icase: bool) -> Result<fancy_regex::Regex, String> {
    let p = if icase { format!("(?i){pattern}") } else { pattern.to_string() };
    fancy_regex::Regex::new(&p).map_err(|e| format!("Regular expression compile error: {e}"))
}

/// fish glob: `*` matches any run, `?` is literal.
fn glob_regex(pat: &str, icase: bool, anchored: bool) -> fancy_regex::Regex {
    let mut re = String::new();
    if anchored {
        re.push('^');
    }
    for c in pat.chars() {
        match c {
            '*' => re.push_str(".*"),
            c => re.push_str(&fancy_regex::escape(&c.to_string())),
        }
    }
    if anchored {
        re.push('$');
    }
    compile(&format!("(?s){re}"), icase).unwrap_or_else(|_| fancy_regex::Regex::new("^$").unwrap())
}

fn str_match(o: &Opts, out: &mut Out) -> i32 {
    let o = &without_dashdash(o);
    let Some((pat, items)) = o.rest.split_first() else {
        return err("string match", "missing argument");
    };
    let items = string_inputs(items);
    let icase = o.has('i');
    let regex = o.has('r');
    let re = if regex {
        match compile(pat, icase) {
            Ok(r) => r,
            Err(e) => return err("string match", &e),
        }
    } else {
        glob_regex(pat, icase, !o.has('e'))
    };
    // fish sets a variable for each named group (`(?<name>…)`) from the first
    // argument that matched, every match of it with --all (an empty element
    // where the group did not take part), and to no elements when nothing did
    let names: Vec<String> = if regex { re.capture_names().flatten().map(str::to_string).collect() } else { Vec::new() };
    let mut first_caps: Option<Vec<Vec<String>>> = None;
    let mut matched = 0usize;
    for s in &items {
        let mut hit = false;
        let caps_iter: Vec<fancy_regex::Captures> = if o.has('a') && regex {
            re.captures_iter(s).flatten().collect()
        } else {
            re.captures(s).ok().flatten().into_iter().collect()
        };
        if !names.is_empty() && first_caps.is_none() && !o.has('v') && !caps_iter.is_empty() {
            first_caps = Some(
                names
                    .iter()
                    .map(|n| {
                        caps_iter
                            .iter()
                            .filter_map(|c| match c.name(n) {
                                Some(m) => Some(m.as_str().to_string()),
                                None => o.has('a').then(String::new),
                            })
                            .collect()
                    })
                    .collect(),
            );
        }
        for caps in caps_iter {
            hit = true;
            let whole = caps.get(0).unwrap();
            if o.has('v') {
                continue;
            }
            if regex {
                // fish `report_match`: `-e` prints the argument, then the groups
                // from 1 (from 0 otherwise, minus 0 with -g); a group that took no
                // part prints nothing; `-n` gives each one's index and length
                if o.has('e') {
                    out.line(s);
                }
                let first = usize::from(o.has('e') || o.has('g'));
                for m in (first..caps.len()).filter_map(|k| caps.get(k)) {
                    if o.has('n') {
                        let start = s[..m.start()].chars().count() + 1;
                        out.line(&format!("{start} {}", m.as_str().chars().count()));
                    } else {
                        out.line(m.as_str());
                    }
                }
                continue;
            }
            if o.has('n') {
                let start = s[..whole.start()].chars().count() + 1;
                out.line(&format!("{} {}", start, whole.as_str().chars().count()));
                continue;
            }
            out.line(s);
        }
        if o.has('v') && !hit {
            if o.has('n') {
                out.line(&format!("1 {}", s.chars().count()));
            } else {
                out.line(s);
            }
            matched += 1;
        } else if hit && !o.has('v') {
            matched += 1;
        }
    }
    let vals = first_caps.unwrap_or_else(|| vec![Vec::new(); names.len()]);
    // a group whose name the translated code holds under another zsh name (a
    // `set -l` local) arrives in `__fish_capmap` as `name=zshname …`
    let map = crate::ported::params::getsparam("__fish_capmap").unwrap_or_default();
    let target = |name: &str| {
        map.split(' ')
            .find_map(|e| e.strip_prefix(name).and_then(|r| r.strip_prefix('=')))
            .map_or_else(|| super::emit::zname(name), str::to_string)
    };
    for (name, v) in names.iter().zip(vals) {
        crate::ported::params::setaparam(&target(name), v);
    }
    i32::from(matched == 0)
}

/// The replacement string of `string replace -r`, as PCRE2's extended substitution reads
/// it: `$1`, `${1}`, `${name}`, `$$`, `\n \t \r \e \a \f \xHH \x{H..}`, `\\` and the case
/// operators `\U \L \u \l \E`. A backslash before any other letter or digit, or at the end,
/// is an error.
fn expand_replacement(rep: &str, caps: &fancy_regex::Captures, names: &[&str]) -> Result<String, String> {
    #[derive(Clone, Copy, PartialEq)]
    enum Case {
        None,
        Upper,
        Lower,
    }
    let mut out = String::new();
    let mut mode = Case::None;
    let mut once: Option<Case> = None;
    let mut push = |out: &mut String, text: &str, mode: Case, once: &mut Option<Case>| {
        for c in text.chars() {
            let m = once.take().unwrap_or(mode);
            match m {
                Case::Upper => out.extend(c.to_uppercase()),
                Case::Lower => out.extend(c.to_lowercase()),
                Case::None => out.push(c),
            }
        }
    };
    let bad = || "Regular expression substitute error: bad escape sequence in replacement string".to_string();
    // (pcre2_substitute: PCRE2_ERROR_BADREPLACEMENT, PCRE2_ERROR_NOSUBSTRING)
    let invalid = || "Regular expression substitute error: invalid replacement string".to_string();
    let unknown = || "Regular expression substitute error: unknown substring".to_string();
    let chars: Vec<char> = rep.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '$' => {
                if chars.get(i + 1) == Some(&'$') {
                    push(&mut out, "$", mode, &mut once);
                    i += 2;
                    continue;
                }
                // a group number is digits only (`$2_` is group 2, then `_`), a name is
                // letters, digits and `_`; `${…}` holds either
                let braced = chars.get(i + 1) == Some(&'{');
                let start = i + 1 + usize::from(braced);
                let mut j = start;
                let digits = chars.get(j).is_some_and(char::is_ascii_digit);
                while j < chars.len() && (if digits { chars[j].is_ascii_digit() } else { chars[j].is_alphanumeric() || chars[j] == '_' }) {
                    j += 1;
                }
                let name: String = chars[start..j].iter().collect();
                if name.is_empty() || (braced && chars.get(j) != Some(&'}')) {
                    return Err(invalid());
                }
                let m = if digits {
                    match name.parse::<usize>() {
                        Ok(n) if n < caps.len() => caps.get(n),
                        _ => return Err(unknown()),
                    }
                } else if names.iter().any(|n| *n == name) {
                    caps.name(&name)
                } else {
                    return Err(unknown());
                };
                // (an existing group that took no part is empty: SUBSTITUTE_UNSET_EMPTY)
                push(&mut out, m.map_or("", |m| m.as_str()), mode, &mut once);
                i = j + usize::from(braced);
            }
            '\\' => {
                let Some(&e) = chars.get(i + 1) else { return Err(bad()) };
                i += 2;
                let lit = match e {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    'e' => '\x1b',
                    'a' => '\x07',
                    'f' => '\x0c',
                    'U' => {
                        mode = Case::Upper;
                        continue;
                    }
                    'L' => {
                        mode = Case::Lower;
                        continue;
                    }
                    'E' => {
                        mode = Case::None;
                        continue;
                    }
                    'u' => {
                        once = Some(Case::Upper);
                        continue;
                    }
                    'l' => {
                        once = Some(Case::Lower);
                        continue;
                    }
                    'x' => {
                        let (digits, next) = if chars.get(i) == Some(&'{') {
                            match chars[i + 1..].iter().position(|&c| c == '}') {
                                Some(p) => (chars[i + 1..i + 1 + p].iter().collect::<String>(), i + 2 + p),
                                None => return Err(bad()),
                            }
                        } else {
                            let mut j = i;
                            while j < chars.len() && j < i + 2 && chars[j].is_ascii_hexdigit() {
                                j += 1;
                            }
                            (chars[i..j].iter().collect(), j)
                        };
                        i = next;
                        match u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32) {
                            Some(c) => c,
                            None if digits.is_empty() => '\0',
                            None => return Err(bad()),
                        }
                    }
                    c if c.is_ascii_alphanumeric() => return Err(bad()),
                    c => c,
                };
                push(&mut out, &lit.to_string(), mode, &mut once);
            }
            c => {
                push(&mut out, &c.to_string(), mode, &mut once);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// `string replace` without `-r` (replace.rs `StringReplacer::Literal`). With `-i` fish
/// lowercases the pattern and searches a lowercased copy of the argument, and the text
/// before each match comes from that copy: `string replace -i o X Hello` is `hellX`.
/// Only the rest after the last match is the argument's own. Character offsets.
fn replace_literal(arg: &str, pattern: &str, rep: &str, icase: bool, all: bool) -> Option<String> {
    let (pattern, subject) = if icase { (pattern.to_lowercase(), arg.to_lowercase()) } else { (pattern.to_string(), arg.to_string()) };
    let pat: Vec<char> = pattern.chars().collect();
    let sub: Vec<char> = subject.chars().collect();
    let mut result = String::new();
    let mut offset = 0;
    while let Some(idx) = sub[offset..].windows(pat.len()).position(|w| w == pat.as_slice()) {
        result.extend(&sub[offset..offset + idx]);
        result.push_str(rep);
        offset += idx + pat.len();
        if !all {
            break;
        }
    }
    if offset == 0 {
        return None;
    }
    result.extend(arg.chars().skip(offset));
    Some(result)
}

fn str_replace(o: &Opts, out: &mut Out) -> i32 {
    let o = &without_dashdash(o);
    let (Some(pat), Some(rep)) = (o.rest.first(), o.rest.get(1)) else {
        return err("string replace", "missing argument");
    };
    let max_matches = match o.val('m') {
        None => None,
        Some(v) => match v.parse::<usize>() {
            Ok(n) if n > 0 => Some(n),
            _ => return err("string replace", &format!("Invalid max matches value '{v}'")),
        },
    };
    let items = string_inputs(o.rest.get(2..).unwrap_or_default());
    let icase = o.has('i');
    let re = if o.has('r') {
        match compile(pat, icase) {
            Ok(r) => Some(r),
            Err(e) => return err("string replace", &e),
        }
    } else {
        None
    };
    let names: Vec<&str> = re.as_ref().map_or(Vec::new(), |r| r.capture_names().flatten().collect());
    // the retry after an empty match: anchored where the search started, not empty there
    let retry = re.as_ref().and_then(|_| {
        let p = if icase { format!("(?i)\\G(?:{pat})") } else { format!("\\G(?:{pat})") };
        fancy_regex::RegexBuilder::new(&p).find_not_empty(true).build().ok()
    });
    let mut replace_count = 0usize;
    for s in &items {
        let replaced = match &re {
            // an empty literal pattern replaces nothing
            None if pat.is_empty() => None,
            None => replace_literal(s, pat, rep, icase, o.has('a')),
            Some(re) => match pcre2_substitute(re, retry.as_ref(), s, o.has('a'), |caps| expand_replacement(rep, caps, &names)) {
                Ok(r) => r,
                // the arguments before this one stay printed
                Err(e) => return err("string replace", &e),
            },
        };
        replace_count += usize::from(replaced.is_some());
        match replaced {
            Some(r) => out.line(&r),
            None if !o.has('f') => out.line(s),
            None => {}
        }
        if (o.has('q') && replace_count > 0) || max_matches == Some(replace_count) {
            return 0;
        }
    }
    i32::from(replace_count == 0)
}

/// `pcre2_substitute` (pcre2_substitute.c, the global loop), which fish's `replace -r`
/// calls through `Regex::replace{,_all}` (string/replace.rs `StringReplacer::replace`).
/// After an empty match that did not start past the search point, the next search is
/// anchored there and may not be empty (`PCRE2_NOTEMPTY_ATSTART | PCRE2_ANCHORED`, the
/// `retry` regex); when that fails one character is copied and the search goes on. An
/// empty match right after a non-empty one counts, and so does one at the end of the
/// subject: `string replace -ra '' L one` is `LoLnLeL`, `replace -ra 'x*' - xa` is `--a-`.
/// `None` when nothing matched.
fn pcre2_substitute(
    re: &fancy_regex::Regex,
    retry: Option<&fancy_regex::Regex>,
    s: &str,
    global: bool,
    mut expand: impl FnMut(&fancy_regex::Captures) -> Result<String, String>,
) -> Result<Option<String>, String> {
    let mut result = String::new();
    let mut subs = 0usize;
    let mut start = 0;
    let mut anchored = false;
    // the previous match and the offset its search started from (`ovecsave`)
    let mut save: Option<(usize, usize, usize)> = None;
    loop {
        let found = if anchored { retry.and_then(|r| r.captures_from_pos(s, start).ok().flatten()) } else { re.captures_from_pos(s, start).ok().flatten() };
        let Some(caps) = found else {
            if !anchored || start >= s.len() {
                break;
            }
            // advance one character, copying it
            let next = start + s[start..].chars().next().map_or(1, char::len_utf8);
            result.push_str(&s[start..next]);
            start = next;
            anchored = false;
            continue;
        };
        let m = caps.get(0).unwrap();
        if let Some((s0, e0, from)) = save.filter(|&(s0, e0, _)| (s0, e0) == (m.start(), m.end())) {
            // the same empty match found again from its own end: retry anchored there
            if m.start() == m.end() && from != start {
                save = Some((s0, e0, start));
                anchored = true;
                continue;
            }
            return Err("Regular expression substitute error: internal error - duplicate substitution match".into());
        }
        subs += 1;
        result.push_str(&s[start..m.start()]);
        result.push_str(&expand(&caps)?);
        save = Some((m.start(), m.end(), start));
        anchored = m.start() == m.end() && m.start() <= start;
        start = m.end();
        if !global {
            break;
        }
    }
    result.push_str(&s[start..]);
    Ok((subs > 0).then_some(result))
}

/// A character's terminal width as fish counts it (wcstringutil `fish_wcwidth_visible` over
/// fallback `fish_wcwidth`): backspace is -1, other non-printing characters 0, VS16 1,
/// VS15 and the Fitzpatrick modifiers 0.
fn visible_char_width(c: char) -> isize {
    match c {
        '\x08' => -1,
        '\u{FE0F}' => 1,
        '\u{FE0E}' | '\u{1F3FB}'..='\u{1F3FF}' => 0,
        c => unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as isize,
    }
}

/// wstr `char_at`: the character at `i`, NUL just past the end.
fn char_at(s: &[char], i: usize) -> char {
    s.get(i).copied().unwrap_or('\0')
}

/// The length of the terminal escape sequence that starts `code`, if it is one
/// (screen.rs `escape_code_length`): a screen-name `\ek…\e\\`, an OSC `\e]…` ended by BEL
/// or `\e\\`, a three-character `\e[` + `@`..`_`, a CSI `\e[…` up to its final character,
/// or a two-character `\e` + `@`..`_`.
fn escape_code_length(code: &[char]) -> Option<usize> {
    if char_at(code, 0) != '\x1b' {
        return None;
    }
    // is_screen_name_escape_seq (its tmux test compares the whole code, ESC included, with
    // `Ptmux;`, so only `\ek` gets past it)
    let screen_name = || {
        if char_at(code, 1) != 'k' {
            return None;
        }
        let end = code[2..].windows(2).position(|w| w == ['\x1b', '\\']);
        Some(end.map_or(2, |p| 2 + p + 2))
    };
    let osc = || {
        if char_at(code, 1) != ']' {
            return None;
        }
        (2..code.len()).find(|&i| code[i] == '\x07' || (code[i] == '\\' && code[i - 1] == '\x1b')).map(|i| i + 1)
    };
    let three_byte = || (char_at(code, 1) == '[' && ('@'..='_').contains(&char_at(code, 2))).then_some(3);
    let csi = || {
        if char_at(code, 1) != '[' {
            return None;
        }
        let mut cursor = 2;
        while cursor < code.len() {
            let c = code[cursor];
            if !c.is_ascii() {
                break;
            }
            cursor += 1;
            if ('@'..'~').contains(&c) {
                break;
            }
        }
        Some(cursor)
    };
    let two_byte = || ('@'..='_').contains(&char_at(code, 1)).then_some(2);
    screen_name().or_else(osc).or_else(three_byte).or_else(csi).or_else(two_byte)
}

/// string.rs `width_without_escapes`: the width of `s[start..]` on its own line (a
/// backspace cannot take it below 0), less the width of the escape sequences in it.
fn width_without_escapes(s: &[char], start: usize) -> usize {
    let mut width = s[start..].iter().fold(0isize, |w, &c| {
        let cw = visible_char_width(c);
        if cw > 0 || w > 0 { w + cw } else { w }
    });
    let mut pos = start;
    while let Some(off) = s[pos..].iter().position(|&c| c == '\x1b') {
        pos += off;
        match escape_code_length(&s[pos..]) {
            Some(len) => {
                width -= s[pos..pos + len].iter().map(|&c| visible_char_width(c)).sum::<isize>();
                // (the code may hold a second escape, `\e(B\e[m`)
                pos += len - 1;
            }
            None => pos += 1,
        }
    }
    width.max(0) as usize
}

/// shorten.rs: each line is cut to `-m` columns (default: the narrowest non-empty line),
/// the ellipsis counting toward the width and dropped when it alone is wider than that.
/// `-m 0` shortens nothing. Escape sequences take no width and are kept whole.
fn str_shorten(o: &Opts, out: &mut Out) -> i32 {
    let ell: Vec<char> = o.val('c').unwrap_or("…").chars().collect();
    let max = match o.val('m') {
        None => None,
        Some(v) => match v.parse::<usize>() {
            Ok(n) => Some(n),
            Err(_) => return err("string shorten", &format!("Invalid max value '{v}'")),
        },
    };
    let items = string_inputs(&o.rest);
    if max == Some(0) {
        for s in &items {
            out.line(s);
        }
        return 0;
    }
    let mut lines: Vec<Vec<char>> = Vec::new();
    let mut min_width = usize::MAX;
    for s in &items {
        let mut splits: Vec<&str> = s.split('\n').collect();
        if o.has('N') {
            // the first line (the last with -l), the ellipsis appended
            let pick = if o.has('l') { splits.pop() } else { Some(splits.remove(0)) };
            splits = pick.into_iter().collect();
        }
        for line in splits {
            let mut l: Vec<char> = line.chars().collect();
            if o.has('N') {
                l.extend(&ell);
            }
            let w = width_without_escapes(&l, 0);
            if w > 0 && w < min_width {
                min_width = w;
            }
            lines.push(l);
        }
    }
    let ourmax = max.unwrap_or(min_width);
    let ell_width = width_without_escapes(&ell, 0);
    let (ell, ell_width) = if ell_width > ourmax { (Vec::new(), 0) } else { (ell, ell_width) };
    // the escape sequences starting at `pos`, back to back
    let skip_escapes = |l: &[char], pos: usize| {
        let mut total = 0;
        while char_at(l, pos + total) == '\x1b' {
            let Some(len) = escape_code_length(&l[pos + total..]) else { break };
            total += len;
        }
        total
    };
    let mut nsub = 0usize;
    for line in lines {
        if o.has('l') {
            // keep the end: the first position from which the rest fits
            let mut pos = 0;
            while pos < line.len() {
                let w = width_without_escapes(&line, pos);
                if (w <= ourmax && pos == 0) || w + ell_width <= ourmax {
                    break;
                }
                pos += skip_escapes(&line, pos).max(1);
            }
            if o.has('q') {
                if pos != 0 {
                    return 0;
                }
                continue;
            }
            if pos == 0 {
                out.line(&line.iter().collect::<String>());
            } else {
                nsub += 1;
                out.line(&ell.iter().chain(line.get(pos..).unwrap_or_default()).collect::<String>());
            }
            continue;
        }
        // keep the start: as many characters as fit with the ellipsis, or all of them. An
        // escape sequence at the very end steps past the end, so the line counts as cut
        // and gets the ellipsis, as in fish.
        let (mut pos, mut width) = (0usize, 0usize);
        while width <= ourmax && pos < line.len() {
            pos += skip_escapes(&line, pos);
            let w = visible_char_width(char_at(&line, pos));
            if w <= 0 || width + w as usize + ell_width <= ourmax {
                width = width.saturating_add_signed(w);
                pos += 1;
            } else {
                // at the limit: does the whole line fit?
                let mut width2 = width + w as usize;
                let mut pos2 = pos + 1;
                while pos2 < line.len() {
                    pos2 += skip_escapes(&line, pos2);
                    width2 = width2.saturating_add_signed(visible_char_width(char_at(&line, pos2)));
                    pos2 += 1;
                }
                if width2 <= ourmax {
                    pos = pos2;
                }
                break;
            }
        }
        if o.has('q') {
            if pos != line.len() {
                return 0;
            }
            continue;
        }
        if pos == line.len() {
            out.line(&line.iter().collect::<String>());
            continue;
        }
        nsub += 1;
        out.line(&line[..pos.min(line.len())].iter().chain(&ell).collect::<String>());
    }
    i32::from(nsub == 0)
}

fn escape(s: &str, style: &str) -> String {
    match style {
        "url" => s
            .bytes()
            .map(|b| match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
                b => format!("%{b:02X}"),
            })
            .collect(),
        "var" => s
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_string() } else { format!("_{:02X}_", c as u32) })
            .collect(),
        "regex" => fancy_regex::escape(s).into_owned(),
        _ => {
            let plain = !s.is_empty()
                && s.chars().all(|c| c.is_alphanumeric() || "_./-+,:@%=".contains(c));
            if plain {
                s.to_string()
            } else if s.chars().any(|c| (c as u32) < 0x20 || c == '\x7f') {
                s.chars()
                    .map(|c| match c {
                        '\n' => "\\n".to_string(),
                        '\t' => "\\t".to_string(),
                        '\r' => "\\r".to_string(),
                        '\x08' => "\\b".to_string(),
                        '\x1b' => "\\e".to_string(),
                        c if (1..0x1a).contains(&(c as u32)) => format!("\\c{}", (b'a' + c as u8 - 1) as char),
                        '\x1a'..='\x1f' => c.to_string(),
                        c if (c as u32) < 0x20 || c == '\x7f' => format!("\\x{:02x}", c as u32),
                        c if c.is_alphanumeric() => c.to_string(),
                        c => format!("\\{c}"),
                    })
                    .collect()
            } else {
                format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
            }
        }
    }
}

/// `string unescape`; `None` when the argument does not decode (fish_common
/// `unescape_string`): a `%` / `_` escape without its hex digits, a non-ASCII character in
/// the url and var styles, a malformed escape in the script style (`unescape_script`).
fn unescape(s: &str, style: &str) -> Option<String> {
    let c: Vec<char> = s.chars().collect();
    let hex = |i: usize| c.get(i).and_then(|d| d.to_digit(16));
    match style {
        "url" => {
            let mut out = Vec::new();
            let mut i = 0;
            while i < c.len() {
                if c[i] > '\x7f' {
                    return None;
                }
                if c[i] == '%' {
                    if c.get(i + 1) == Some(&'%') {
                        out.push(b'%');
                        i += 1;
                    } else {
                        out.push((hex(i + 1)? * 16 + hex(i + 2)?) as u8);
                        i += 2;
                    }
                } else {
                    out.push(c[i] as u8);
                }
                i += 1;
            }
            Some(String::from_utf8_lossy(&out).into_owned())
        }
        "var" => {
            let mut out = Vec::new();
            let mut prev_hex = false;
            let mut i = 0;
            while i < c.len() {
                if c[i] > '\x7f' {
                    return None;
                }
                if c[i] == '_' {
                    match c.get(i + 1) {
                        None if prev_hex => break,
                        None => return None,
                        Some('_') => {
                            out.push(b'_');
                            i += 1;
                        }
                        Some(d) if d.is_ascii_digit() || ('A'..='F').contains(d) => {
                            out.push((hex(i + 1)? * 16 + hex(i + 2)?) as u8);
                            i += 2;
                            prev_hex = true;
                        }
                        // a lone `_` only separates for readability
                        Some(_) => {}
                    }
                } else {
                    out.push(c[i] as u8);
                }
                i += 1;
            }
            Some(String::from_utf8_lossy(&out).into_owned())
        }
        _ => unescape_script(s).map(|b| String::from_utf8_lossy(&b).into_owned()),
    }
}

/// fish_common `unescape_string_internal` with no flags, the script style of
/// `string unescape`. Unquoted, a backslash starts an escape (`unescape_one`); quotes are
/// dropped and switch modes. In single quotes only `\\` and `\'` escape, in double quotes
/// `\\`, `\$`, `\"` and backslash-newline. A backslash at the end is an error, an unclosed
/// quote is not. Bytes, since `\xHH` writes a raw byte.
fn unescape_script(s: &str) -> Option<Vec<u8>> {
    #[derive(PartialEq)]
    enum Mode {
        Unquoted,
        Single,
        Double,
    }
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let push = |out: &mut Vec<u8>, ch: char| out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
    let mut mode = Mode::Unquoted;
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        let next = char_at(&c, i + 1);
        match mode {
            Mode::Unquoted => match ch {
                '\\' => i += unescape_one(&c[i..], &mut out)? - 1,
                '\'' => mode = Mode::Single,
                '"' => mode = Mode::Double,
                ch => push(&mut out, ch),
            },
            Mode::Single => match (ch, next) {
                ('\\', '\\' | '\'') => {
                    push(&mut out, next);
                    i += 1;
                }
                ('\\', '\0') => return None,
                ('\'', _) => mode = Mode::Unquoted,
                (ch, _) => push(&mut out, ch),
            },
            Mode::Double => match (ch, next) {
                ('"', _) => mode = Mode::Unquoted,
                ('\\', '\0') => return None,
                ('\\', '\\' | '$' | '"') => {
                    push(&mut out, next);
                    i += 1;
                }
                // a backslash-newline is swallowed
                ('\\', '\n') => i += 1,
                (ch, _) => push(&mut out, ch),
            },
        }
        i += 1;
    }
    Some(out)
}

/// fish_common `unescape_one`: the escape at the start of `code` (a backslash), appended
/// to `out`; the number of characters it took, `None` when it is malformed. `\0`–`\777`
/// octal up to 0x7f, `\xHH`/`\XHH` a raw byte, `\uHHHH`, `\UHHHHHHHH` up to U+10FFFF, `\cX`,
/// the letter escapes, backslash-newline nothing, anything else itself. A character fish
/// reserves for its own use is an error.
fn unescape_one(code: &[char], out: &mut Vec<u8>) -> Option<usize> {
    let mut pos = 1;
    loop {
        let c = char_at(code, pos);
        pos += 1;
        let ch = match c {
            // a backslash at the end
            '\0' => return None,
            '0'..='7' | 'u' | 'U' | 'x' | 'X' => {
                let (base, chars, max, byte) = match c {
                    'u' => (16, 4, 0xFFFF, false),
                    'U' => (16, 8, 0x10FFFF, false),
                    'x' | 'X' => (16, 2, 0xFF, true),
                    _ => {
                        pos -= 1;
                        (8, 3, 0x7F, false)
                    }
                };
                let mut res: u64 = 0;
                for n in 0..chars {
                    let Some(d) = char_at(code, pos).to_digit(base) else {
                        if n == 0 {
                            return None;
                        }
                        break;
                    };
                    res = res * u64::from(base) + u64::from(d);
                    pos += 1;
                }
                if res > max {
                    return None;
                }
                if byte {
                    out.push(res as u8);
                    // adjacent byte escapes make one multibyte character together
                    if char_at(code, pos) == '\\' && matches!(char_at(code, pos + 1), 'x' | 'X') {
                        pos += 1;
                        continue;
                    }
                    return Some(pos);
                }
                char::from_u32(res as u32).unwrap_or('\u{FFFD}')
            }
            'a' => '\x07',
            'b' => '\x08',
            'c' => {
                let x = char_at(code, pos) as u32;
                pos += 1;
                if !(u32::from('@')..0x7f).contains(&x) {
                    return None;
                }
                char::from_u32(x % 32).unwrap()
            }
            'e' => '\x1b',
            'f' => '\x0c',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\x0b',
            '\n' => return Some(pos),
            c => c,
        };
        // fish_reserved_codepoint
        if ('\u{FDD0}'..'\u{FDF0}').contains(&ch) || ('\u{F500}'..'\u{F700}').contains(&ch) {
            return None;
        }
        out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
        return Some(pos);
    }
}

// ---- printf ----------------------------------------------------------------------

/// `snprintf` with one argument, for the numeric conversions.
fn c_format<T>(spec: &str, v: T, width_hint: usize) -> String {
    let Ok(cspec) = std::ffi::CString::new(spec) else { return String::new() };
    let mut buf = vec![0u8; width_hint + 80];
    // SAFETY: `cspec` is a valid format with exactly one conversion taking a `T`, and the
    // buffer length is passed to snprintf.
    let n = unsafe { libc::snprintf(buf.as_mut_ptr().cast(), buf.len(), cspec.as_ptr(), v) };
    if n < 0 {
        return String::new();
    }
    buf.truncate((n as usize).min(buf.len() - 1));
    String::from_utf8_lossy(&buf).into_owned()
}

/// An integer argument as printf reads it: decimal, `0x` hex, leading-0 octal, or `'c` for
/// a character's code. Anything left over, or no number at all, is reported and counted
/// as a failure, while the value read so far is still used. An empty argument is 0.
fn printf_int(arg: &str, failed: &mut bool) -> i64 {
    // an empty argument is silently 0 (printf.rs `verify_numeric`)
    if arg.is_empty() {
        return 0;
    }
    if let Some(rest) = arg.strip_prefix(['\'', '"']) {
        return rest.chars().next().map_or(0, |c| c as i64);
    }
    let Ok(c) = std::ffi::CString::new(arg) else { return 0 };
    let mut end: *mut libc::c_char = std::ptr::null_mut();
    // SAFETY: `c` is NUL terminated and `end` receives a pointer into it.
    let v = unsafe { libc::strtoll(c.as_ptr(), &mut end, 0) };
    let used = (end as usize).saturating_sub(c.as_ptr() as usize);
    if used == 0 {
        eprintln!("{arg}: expected a numeric value");
        *failed = true;
        NO_NUMBER.with(|n| n.set(true));
    } else if used < arg.len() {
        eprintln!("{arg}: value not completely converted (can't convert '{}')", &arg[used..]);
        *failed = true;
    }
    v
}

thread_local! {
    /// Set when a numeric conversion got an argument with no number in it; fish stops
    /// printing there.
    static NO_NUMBER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn printf_float(arg: &str, failed: &mut bool) -> f64 {
    if arg.is_empty() {
        return 0.0;
    }
    if let Some(rest) = arg.strip_prefix(['\'', '"']) {
        return rest.chars().next().map_or(0.0, |c| c as u32 as f64);
    }
    let Ok(c) = std::ffi::CString::new(arg) else { return 0.0 };
    let mut end: *mut libc::c_char = std::ptr::null_mut();
    // SAFETY: as in `printf_int`.
    let v = unsafe { libc::strtod(c.as_ptr(), &mut end) };
    let used = (end as usize).saturating_sub(c.as_ptr() as usize);
    if used == 0 {
        eprintln!("{arg}: expected a numeric value");
        *failed = true;
        NO_NUMBER.with(|n| n.set(true));
    } else if used < arg.len() {
        eprintln!("{arg}: value not completely converted (can't convert '{}')", &arg[used..]);
        *failed = true;
    }
    v
}

/// Backslash escapes of a format or `%b` argument. Returns false after `\c`, which ends
/// all output.
fn printf_escapes(s: &str, out: &mut Vec<u8>, percent_b: bool) -> bool {
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] != '\\' || i + 1 >= b.len() {
            let mut buf = [0u8; 4];
            out.extend_from_slice(b[i].encode_utf8(&mut buf).as_bytes());
            i += 1;
            continue;
        }
        i += 1;
        let c = b[i];
        i += 1;
        match c {
            'a' => out.push(7),
            'b' => out.push(8),
            'f' => out.push(12),
            'n' => out.push(b'\n'),
            'r' => out.push(b'\r'),
            't' => out.push(b'\t'),
            'v' => out.push(11),
            'e' => out.push(27),
            '\\' => out.push(b'\\'),
            'c' => return false,
            '0'..='7' => {
                // up to three octal digits (`%b` also takes a leading 0)
                let mut v = c.to_digit(8).unwrap_or(0);
                let mut n = 1;
                let max = if percent_b && c == '0' { 4 } else { 3 };
                while n < max && i < b.len() && b[i].is_digit(8) {
                    v = v * 8 + b[i].to_digit(8).unwrap_or(0);
                    i += 1;
                    n += 1;
                }
                out.push(v as u8);
            }
            'x' | 'u' | 'U' => {
                let max = match c {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let mut v = 0u32;
                let mut n = 0;
                while n < max && i < b.len() && b[i].is_ascii_hexdigit() {
                    v = v * 16 + b[i].to_digit(16).unwrap_or(0);
                    i += 1;
                    n += 1;
                }
                if n == 0 {
                    out.extend_from_slice(format!("\\{c}").as_bytes());
                } else if c == 'x' {
                    out.push(v as u8);
                } else if let Some(ch) = char::from_u32(v) {
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
            }
            other => {
                out.push(b'\\');
                let mut buf = [0u8; 4];
                out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    true
}

fn pad(text: &str, width: usize, left: bool) -> String {
    let n = text.chars().count();
    if n >= width {
        return text.to_string();
    }
    let fill = " ".repeat(width - n);
    if left { format!("{text}{fill}") } else { format!("{fill}{text}") }
}

fn printf(args: &[String]) -> i32 {
    let Some(fmt) = args.first() else {
        eprintln!("printf: not enough arguments");
        return 2;
    };
    let mut next = 1usize;
    let mut failed = false;
    let mut out: Vec<u8> = Vec::new();
    'passes: loop {
        let start = next;
        let f: Vec<char> = fmt.chars().collect();
        let mut i = 0;
        while i < f.len() {
            if f[i] == '\\' {
                // collect the escape and let the shared decoder read it
                let mut j = i + 1;
                if j < f.len() {
                    j += 1;
                    if matches!(f[j - 1], '0'..='7') {
                        while j < f.len() && j < i + 4 && f[j].is_digit(8) {
                            j += 1;
                        }
                    } else if matches!(f[j - 1], 'x' | 'u' | 'U') {
                        let max = match f[j - 1] { 'x' => 2, 'u' => 4, _ => 8 };
                        let mut n = 0;
                        while j < f.len() && n < max && f[j].is_ascii_hexdigit() {
                            j += 1;
                            n += 1;
                        }
                    }
                }
                let piece: String = f[i..j.min(f.len())].iter().collect();
                if !printf_escapes(&piece, &mut out, false) {
                    break 'passes;
                }
                i = j;
                continue;
            }
            if f[i] != '%' {
                let mut buf = [0u8; 4];
                out.extend_from_slice(f[i].encode_utf8(&mut buf).as_bytes());
                i += 1;
                continue;
            }
            let spec_start = i;
            i += 1;
            if f.get(i) == Some(&'%') {
                out.push(b'%');
                i += 1;
                continue;
            }
            let mut flags = String::new();
            while i < f.len() && "-+ #0'".contains(f[i]) {
                flags.push(f[i]);
                i += 1;
            }
            let mut take = |next: &mut usize, failed: &mut bool| -> i64 {
                let v = args.get(*next).map_or(0, |a| printf_int(a, failed));
                *next += 1;
                v
            };
            let mut width = String::new();
            if f.get(i) == Some(&'*') {
                let w = take(&mut next, &mut failed);
                if w < 0 {
                    flags.push('-');
                }
                width = w.abs().to_string();
                i += 1;
            } else {
                while i < f.len() && f[i].is_ascii_digit() {
                    width.push(f[i]);
                    i += 1;
                }
            }
            let mut prec: Option<String> = None;
            if f.get(i) == Some(&'.') {
                i += 1;
                let mut p = String::new();
                if f.get(i) == Some(&'*') {
                    p = take(&mut next, &mut failed).max(0).to_string();
                    i += 1;
                } else {
                    while i < f.len() && f[i].is_ascii_digit() {
                        p.push(f[i]);
                        i += 1;
                    }
                }
                prec = Some(p);
            }
            while i < f.len() && "hlLqjzt".contains(f[i]) {
                i += 1;
            }
            let Some(&conv) = f.get(i) else {
                eprintln!("{}: invalid conversion specification", f[spec_start..].iter().collect::<String>());
                failed = true;
                break 'passes;
            };
            i += 1;
            let wnum: usize = width.parse().unwrap_or(0);
            let spec_tail = format!(
                "{flags}{width}{}",
                prec.as_ref().map_or(String::new(), |p| format!(".{p}"))
            );
            match conv {
                'd' | 'i' => {
                    let v = args.get(next).map_or(0, |a| printf_int(a, &mut failed));
                    next += 1;
                    if NO_NUMBER.with(|n| n.replace(false)) {
                        break 'passes;
                    }
                    out.extend_from_slice(c_format(&format!("%{spec_tail}lld"), v, wnum).as_bytes());
                }
                'u' | 'o' | 'x' | 'X' | 'c' if conv != 'c' => {
                    let v = args.get(next).map_or(0, |a| printf_int(a, &mut failed));
                    next += 1;
                    if NO_NUMBER.with(|n| n.replace(false)) {
                        break 'passes;
                    }
                    out.extend_from_slice(c_format(&format!("%{spec_tail}ll{conv}"), v as u64, wnum).as_bytes());
                }
                'f' | 'F' | 'e' | 'E' | 'g' | 'G' | 'a' | 'A' => {
                    let v = args.get(next).map_or(0.0, |a| printf_float(a, &mut failed));
                    next += 1;
                    if NO_NUMBER.with(|n| n.replace(false)) {
                        break 'passes;
                    }
                    out.extend_from_slice(c_format(&format!("%{spec_tail}{conv}"), v, wnum).as_bytes());
                }
                's' => {
                    let a = args.get(next).cloned().unwrap_or_default();
                    next += 1;
                    let a = match prec.as_deref().and_then(|p| p.parse::<usize>().ok()) {
                        Some(p) => a.chars().take(p).collect(),
                        None => a,
                    };
                    out.extend_from_slice(pad(&a, wnum, flags.contains('-')).as_bytes());
                }
                'c' => {
                    let a = args.get(next).cloned().unwrap_or_default();
                    next += 1;
                    // an empty or missing argument is a NUL (printf.rs: `argument.char_at(0)`)
                    let c = a.chars().next().unwrap_or('\0').to_string();
                    out.extend_from_slice(pad(&c, wnum, flags.contains('-')).as_bytes());
                }
                'b' => {
                    let a = args.get(next).cloned().unwrap_or_default();
                    next += 1;
                    let mut tmp = Vec::new();
                    let go_on = printf_escapes(&a, &mut tmp, true);
                    let text = String::from_utf8_lossy(&tmp).into_owned();
                    out.extend_from_slice(pad(&text, wnum, flags.contains('-')).as_bytes());
                    if !go_on {
                        break 'passes;
                    }
                }
                other => {
                    eprintln!("{}: invalid conversion specification", f[spec_start..i].iter().collect::<String>());
                    let _ = other;
                    failed = true;
                    break 'passes;
                }
            }
        }
        // the format is reused while arguments remain and the last pass used some
        if next >= args.len() || next == start {
            break;
        }
    }
    let mut so = std::io::stdout().lock();
    let _ = so.write_all(&out);
    let _ = so.flush();
    i32::from(failed)
}

// ---- universal variables ----------------------------------------------------------

/// fish's universal-variable file: `$XDG_CONFIG_HOME/fish/fish_variables`.
pub fn uvar_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    base.join("fish").join("fish_variables")
}

/// A value as `fish_variables` spells it: letters, digits, `_` and `/` as they are,
/// everything else as `\xHH`, `\uHHHH` or `\UHHHHHHHH`.
fn uvar_encode(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '/' {
            out.push(c);
        } else if (c as u32) < 0x80 {
            out.push_str(&format!("\\x{:02x}", c as u32));
        } else if (c as u32) < 0x10000 {
            out.push_str(&format!("\\u{:04x}", c as u32));
        } else {
            out.push_str(&format!("\\U{:08x}", c as u32));
        }
    }
    out
}

fn uvar_decode(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != '\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        let (digits, kind) = match b[i + 1] {
            'x' => (2, 'x'),
            'u' => (4, 'u'),
            'U' => (8, 'U'),
            'n' => {
                out.push('\n');
                i += 2;
                continue;
            }
            c => {
                out.push(c);
                i += 2;
                continue;
            }
        };
        let hex: String = b[i + 2..].iter().take(digits).collect();
        match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
            Some(c) => out.push(c),
            None => out.push_str(&format!("\\{kind}{hex}")),
        }
        i += 2 + hex.chars().count();
    }
    out
}

/// Read the file and define each variable in the running shell: a list is a zsh array, an
/// exported one is tied to its exported scalar like `set -x` does.
fn uvar_load() -> i32 {
    let Ok(text) = std::fs::read_to_string(uvar_path()) else { return 0 };
    let q = super::emit::squote;
    let mut code = String::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("SETUVAR ") else { continue };
        let mut export = false;
        let mut rest = rest;
        while let Some(flag) = rest.strip_prefix("--") {
            let (f, tail) = flag.split_once(' ').unwrap_or((flag, ""));
            export |= f == "export";
            rest = tail;
        }
        let Some((name, value)) = rest.split_once(':') else { continue };
        let items: Vec<String> = if value == "\\x1d" || value.is_empty() {
            Vec::new()
        } else {
            value.split("\\x1e").map(uvar_decode).collect()
        };
        let z = super::emit::zname(name);
        let list = items.iter().map(|v| q(v)).collect::<Vec<_>>().join(" ");
        if export {
            let tied = format!("__fx_{name}");
            let sep = if name.ends_with("PATH") { ':' } else { ' ' };
            code.push_str(&format!(
                "if (( ! ${{+{tied}}} )); then unset {name} 2>/dev/null; typeset -gxT {name} {tied} '{sep}'; fi; {tied}=({list})\n"
            ));
            super::emit::note_exported(name);
        } else {
            code.push_str(&format!("typeset -ga {z}=({list})\n"));
        }
    }
    if !code.is_empty() {
        let _ = crate::ported::exec::execute_script_zsh_pipeline(&code);
    }
    0
}

/// `__fish_uvar_write [--export] NAME VALUE…` sets a variable in the file, `-e NAME` removes it.
/// The file is rewritten whole, keeping every other line, and replaced atomically.
fn uvar_write(args: &[String]) -> i32 {
    let (erase, export, rest) = match args.first().map(String::as_str) {
        Some("-e") => (true, false, &args[1..]),
        Some("--export") => (false, true, &args[1..]),
        _ => (false, false, args),
    };
    let Some((name, values)) = rest.split_first() else { return 1 };
    // the shell's names for tied/special variables back to fish's
    let name = match name.as_str() {
        "path" => "PATH",
        "cdpath" => "CDPATH",
        "manpath" => "MANPATH",
        n => n.strip_prefix("_fish_v_").unwrap_or(n),
    };
    let file = uvar_path();
    let old = std::fs::read_to_string(&file).unwrap_or_default();
    let mut lines: Vec<String> = old.lines().map(str::to_string).collect();
    if lines.is_empty() {
        lines.push("# This file contains fish universal variable definitions.".into());
        lines.push("# VERSION: 3.0".into());
    }
    let is_ours = |l: &String| {
        l.strip_prefix("SETUVAR ").is_some_and(|r| {
            let r = r.trim_start_matches("--export ").trim_start_matches("--path ");
            r.split(':').next() == Some(name)
        })
    };
    let had = lines.iter().any(is_ours);
    if erase && !had {
        return 0; // nothing to remove: leave the file (which may not exist) alone
    }
    lines.retain(|l| !is_ours(l));
    if !erase {
        let value = if values.is_empty() {
            "\\x1d".to_string()
        } else {
            values.iter().map(|v| uvar_encode(v)).collect::<Vec<_>>().join("\\x1e")
        };
        lines.push(format!("SETUVAR {}{name}:{value}", if export { "--export " } else { "" }));
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = file.with_extension(format!("tmp{}", std::process::id()));
    let ok = std::fs::write(&tmp, lines.join("\n") + "\n").is_ok() && std::fs::rename(&tmp, &file).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
    i32::from(!ok)
}

// ---- history ---------------------------------------------------------------------

/// `__fish_history_delete --exact|--prefix|--contains TERM` / `--all`: remove matching
/// commands from the persistent history (sqlite index and text file).
fn history_delete(args: &[String]) -> i32 {
    let Some(mode) = args.first() else { return 1 };
    let term = args.get(1).cloned().unwrap_or_default();
    let Ok(engine) = crate::history::HistoryEngine::new() else { return 1 };
    let drop: Box<dyn Fn(&str) -> bool> = match mode.as_str() {
        "--all" => Box::new(|_| true),
        "--exact" => Box::new(move |c| c == term),
        "--prefix" => Box::new(move |c| c.starts_with(&term)),
        _ => Box::new(move |c| c.contains(&term)),
    };
    i32::from(engine.forget(&*drop).is_err())
}

// ---- path ------------------------------------------------------------------------

/// Lexical normalisation: `//`, `.` and `..` collapsed, no filesystem access.
fn normalize(p: &str) -> String {
    let abs = p.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.last().is_some_and(|l| *l != "..") {
                    stack.pop();
                } else if !abs {
                    stack.push("..");
                }
            }
            s => stack.push(s),
        }
    }
    let joined = stack.join("/");
    match (abs, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".into(),
        (false, false) => joined,
    }
}

fn base_name(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.is_empty() { String::new() } else { "/".into() };
    }
    t.rsplit('/').next().unwrap_or(t).to_string()
}

fn dir_name(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.is_empty() { ".".into() } else { "/".into() };
    }
    match t.rfind('/') {
        None => ".".into(),
        Some(0) => "/".into(),
        Some(i) => t[..i].trim_end_matches('/').to_string(),
    }
}

/// `.ext` of the final component; a leading dot alone is not an extension.
fn extension(p: &str) -> String {
    let b = base_name(p);
    match b.rfind('.') {
        Some(i) if i > 0 => b[i..].to_string(),
        _ => String::new(),
    }
}

fn path(args: &[String]) -> i32 {
    let Some(sub) = args.first() else {
        return err("path", "missing subcommand");
    };
    let long = [("quiet", 'q'), ("invert", 'v'), ("reverse", 'r'), ("null-in", 'z'), ("null-out", 'Z'), ("type", 't'), ("perm", 'p'), ("key", 'k'), ("relative", 'R')];
    let o = match parse_opts(&args[1..], "tpk", &long) {
        Ok(o) => o,
        Err(e) => return err(&format!("path {sub}"), &e),
    };
    let mut out = Out::new(o.has('q'), o.has('Z'));
    // path.rs `parse_opts`: with stdin redirected, operands past the subcommand's own one are
    // too many, and the paths come from stdin
    let leading = usize::from(sub == "change-extension");
    if stdin_redirected() && o.rest.len() > leading {
        return err(&format!("path {sub}"), "too many arguments");
    }
    let items = if stdin_redirected() { inputs_detecting_nul(o.has('z')) } else { o.rest.clone() };
    let status = match sub.as_str() {
        "basename" | "dirname" | "extension" | "normalize" | "resolve" => {
            let mut any = false;
            for p in &items {
                let r = match sub.as_str() {
                    "basename" => {
                        let b = base_name(p);
                        let ext = extension(p);
                        if o.has('E') { b[..b.len() - ext.len()].to_string() } else { b }
                    }
                    "dirname" => dir_name(p),
                    "extension" => extension(p),
                    "normalize" => normalize(p),
                    _ => std::fs::canonicalize(p)
                        .map(|c| c.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| normalize(&format!("{}/{p}", std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default()).replace("//", "/"))),
                };
                any |= sub != "extension" || !r.is_empty();
                out.line(&r);
            }
            i32::from(!any)
        }
        "mtime" => {
            use std::os::unix::fs::MetadataExt;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let mut any = false;
            for p in &items {
                if let Ok(m) = std::fs::metadata(p) {
                    let t = m.mtime();
                    out.line(&(if o.has('R') { now - t } else { t }).to_string());
                    any = true;
                }
            }
            i32::from(!any)
        }
        "change-extension" => {
            let Some((ext, paths)) = o.rest.split_first() else {
                return err("path change-extension", "missing argument");
            };
            // (path.rs `path_change_extension`: status 1 when there was no path)
            let paths = if stdin_redirected() { items.clone() } else { paths.to_vec() };
            for p in &paths {
                let e = extension(p);
                let stem = &p[..p.len() - e.len()];
                let dot = if ext.is_empty() || ext.starts_with('.') { "" } else { "." };
                out.line(&format!("{stem}{dot}{ext}"));
            }
            i32::from(paths.is_empty())
        }
        "filter" | "is" => {
            let mut passed = 0usize;
            for p in &items {
                let meta = std::fs::symlink_metadata(p);
                let full = std::fs::metadata(p);
                let mut ok = meta.is_ok();
                if let Some(types) = o.val('t') {
                    ok = types.split(',').any(|t| match t {
                        "file" => full.as_ref().is_ok_and(|m| m.is_file()),
                        "dir" => full.as_ref().is_ok_and(|m| m.is_dir()),
                        "link" => meta.as_ref().is_ok_and(|m| m.file_type().is_symlink()),
                        _ => false,
                    });
                }
                if ok {
                    if let Some(perms) = o.val('p') {
                        ok = perms.split(',').all(|perm| {
                            let bit = match perm {
                                "read" => libc::R_OK,
                                "write" => libc::W_OK,
                                "exec" => libc::X_OK,
                                _ => return false,
                            };
                            std::ffi::CString::new(p.as_str()).is_ok_and(|c| unsafe { libc::access(c.as_ptr(), bit) } == 0)
                        });
                    }
                }
                if ok != o.has('v') {
                    passed += 1;
                    if sub == "filter" {
                        out.line(p);
                    }
                }
            }
            if sub == "is" {
                i32::from(passed != items.len() || items.is_empty())
            } else {
                i32::from(passed == 0)
            }
        }
        "sort" => {
            let key = o.val('k').unwrap_or("path");
            let mut v = items;
            v.sort_by_key(|p| match key {
                "basename" => base_name(p),
                "dirname" => dir_name(p),
                _ => p.clone(),
            });
            if o.has('r') {
                v.reverse();
            }
            for p in &v {
                out.line(p);
            }
            i32::from(v.is_empty())
        }
        other => return err("path", &format!("{other}: invalid subcommand")),
    };
    out.flush();
    status
}

// ---- count / contains ------------------------------------------------------------

/// count.rs: the arguments, plus the lines of a redirected stdin that end in a newline
/// (`echo a | count x y` is 3).
fn count(args: &[String]) -> i32 {
    let mut n = args.len();
    if stdin_redirected() {
        let mut buf = Vec::new();
        let _ = std::io::stdin().lock().read_to_end(&mut buf);
        n += buf.iter().filter(|&&b| b == b'\n').count();
    }
    println!("{n}");
    i32::from(n == 0)
}

fn contains(args: &[String]) -> i32 {
    let mut index = false;
    let mut i = 0;
    while let Some(a) = args.get(i) {
        match a.as_str() {
            "-i" | "--index" => index = true,
            "--" => {
                i += 1;
                break;
            }
            _ => break,
        }
        i += 1;
    }
    let Some((needle, hay)) = args[i..].split_first() else {
        return err("contains", "Key not specified");
    };
    match hay.iter().position(|h| h == needle) {
        Some(p) => {
            if index {
                println!("{}", p + 1);
            }
            0
        }
        None => 1,
    }
}

// ---- math ------------------------------------------------------------------------

fn math(args: &[String]) -> i32 {
    let o = match parse_opts_math(args) {
        Ok(o) => o,
        Err(e) => return err("math", &e),
    };
    // math.rs: the operands, or the lines of a redirected stdin instead, joined by spaces
    let expr = if stdin_redirected() { inputs(&[], false).join(" ") } else { o.rest.join(" ") };
    if expr.is_empty() {
        eprintln!("math: expected >= 1 arguments; got 0");
        return 1;
    }
    match eval_math(&expr) {
        Ok(v) => {
            let scale = o.num('s').map_or(6, |s| s.clamp(0, 15) as usize);
            match format_math(v, scale, o.val('b')) {
                Some(s) => {
                    println!("{s}");
                    0
                }
                None => {
                    eprintln!("math: Error: Result magnitude is too large\n'{expr}'");
                    1
                }
            }
        }
        Err((e, col, len)) => {
            let carets = if len > 1 { format!("^{}^", "~".repeat(len - 2)) } else { "^".to_string() };
            eprintln!("math: Error: {e}\n'{expr}'\n{}{carets}", " ".repeat(col));
            1
        }
    }
}

/// `math` takes `-s N`, `-b BASE` and `-m MODE` ahead of the expression, but a leading
/// `-` in the expression itself (`math -1 \* 3`) must stay part of it.
fn parse_opts_math(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let take = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i).cloned().ok_or(format!("{a}: option requires an argument"))
        };
        match a {
            "-s" | "--scale" => {
                let v = take(&mut i)?;
                o.vals.insert('s', v);
            }
            "-b" | "--base" => {
                let v = take(&mut i)?;
                o.vals.insert('b', v);
            }
            "-m" | "--scale-mode" => {
                let _ = take(&mut i)?;
            }
            "--" => {
                i += 1;
                break;
            }
            s if s.starts_with("--scale=") => {
                o.vals.insert('s', s["--scale=".len()..].to_string());
            }
            s if s.starts_with("--base=") => {
                o.vals.insert('b', s["--base=".len()..].to_string());
            }
            s if s.starts_with("-s") && s[2..].chars().all(|c| c.is_ascii_digit()) && s.len() > 2 => {
                o.vals.insert('s', s[2..].to_string());
            }
            _ => break,
        }
        i += 1;
    }
    o.rest = args[i..].to_vec();
    Ok(o)
}

fn format_math(v: f64, scale: usize, base: Option<&str>) -> Option<String> {
    if v.is_nan() || v.is_infinite() {
        return Some(v.to_string().to_lowercase());
    }
    if v.abs() >= 9_007_199_254_740_992.0 {
        return None;
    }
    // beyond 2^52 a double has no fraction digits left
    let v = if v.abs() >= 4_503_599_627_370_496.0 { v.round() } else { v };
    if let Some(b) = base {
        let n = v.trunc() as i64;
        return Some(match b {
            "hex" | "16" => format!("{}0x{:x}", if n < 0 { "-" } else { "" }, n.unsigned_abs()),
            "octal" | "8" => format!("{}0{:o}", if n < 0 { "-" } else { "" }, n.unsigned_abs()),
            _ => n.to_string(),
        });
    }
    if scale == 0 {
        return Some((v.trunc() as i64).to_string());
    }
    if v.fract() == 0.0 {
        return Some((v as i64).to_string());
    }
    let s = format!("{v:.scale$}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    Some(if s == "-0" || s.is_empty() { "0".into() } else { s })
}

struct MathParser<'a> {
    s: &'a [u8],
    i: usize,
    /// Where fish's caret goes for the error returned: (column, token length).
    at: (usize, usize),
}

/// Evaluate a `math` expression; an error carries the caret position and token length
/// fish underlines in the echoed expression.
fn eval_math(src: &str) -> Result<f64, (String, usize, usize)> {
    let mut p = MathParser {
        s: src.as_bytes(),
        i: 0,
        at: (0, 1),
    };
    let v = match p.expr() {
        Ok(v) => v,
        Err(e) => return Err((e, p.at.0, p.at.1)),
    };
    let end = src[..p.i].trim_end().len();
    p.ws();
    if p.i < p.s.len() {
        // after a plain number the next operand lacks an operator, unless it opens a group
        let tail = src[..end].rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).next().unwrap_or("");
        let after_number = tail.starts_with(|c: char| c.is_ascii_digit() || c == '.');
        return Err(if !after_number || p.s[p.i] == b'(' || p.s[p.i] == b')' {
            ("Too many arguments".into(), p.i + next_token_len(&src[p.i..]) + 1, 1)
        } else {
            ("Missing operator".into(), end + 1, 1)
        });
    }
    Ok(v)
}

impl MathParser<'_> {
    /// An error whose caret is drawn `col` columns in, under `len` characters.
    fn fail<T>(&mut self, msg: &str, col: usize, len: usize) -> Result<T, String> {
        self.at = (col, len);
        Err(msg.into())
    }
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }
    fn eat(&mut self, t: &str) -> bool {
        self.ws();
        if self.s[self.i..].starts_with(t.as_bytes()) {
            self.i += t.len();
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<f64, String> {
        let mut v = self.term()?;
        loop {
            if self.eat("+") {
                v += self.term()?;
            } else if self.eat("-") {
                v -= self.term()?;
            } else {
                return Ok(v);
            }
        }
    }
    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.power()?;
        loop {
            self.ws();
            let star = self.eat("*");
            let ex = !star && self.eat("x");
            if ex && self.s[self.i..].iter().all(u8::is_ascii_whitespace) {
                // a trailing `x` has nothing to multiply by: fish reads it as a function name
                return self.fail("Unknown function", self.i, 1);
            }
            if star || ex {
                v *= self.power()?;
            } else if self.eat("/") {
                let op_end = self.i;
                let d = self.power()?;
                if d == 0.0 {
                    return self.fail("Division by zero", op_end, 1);
                }
                v /= d;
            } else if self.eat("%") {
                let op_end = self.i;
                let d = self.power()?;
                if d == 0.0 {
                    return self.fail("Division by zero", op_end, 1);
                }
                v %= d;
            } else {
                return Ok(v);
            }
        }
    }
    fn power(&mut self) -> Result<f64, String> {
        let base = self.unary()?;
        if self.eat("^") {
            let e = self.power()?;
            return Ok(base.powf(e));
        }
        Ok(base)
    }
    fn unary(&mut self) -> Result<f64, String> {
        if self.eat("-") {
            return Ok(-self.unary()?);
        }
        if self.eat("+") {
            return self.unary();
        }
        self.primary()
    }
    fn primary(&mut self) -> Result<f64, String> {
        self.ws();
        if self.eat("(") {
            let v = self.expr()?;
            if !self.eat(")") {
                return self.fail("Missing closing parenthesis", self.s.len() + 1, 1);
            }
            return Ok(v);
        }
        let start = self.i;
        if self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
            if self.s[self.i..].starts_with(b"0x") || self.s[self.i..].starts_with(b"0X") {
                self.i += 2;
                let h = self.i;
                while self.s.get(self.i).is_some_and(|c| c.is_ascii_hexdigit()) {
                    self.i += 1;
                }
                let txt = std::str::from_utf8(&self.s[h..self.i]).unwrap_or("0");
                return i64::from_str_radix(txt, 16).map(|n| n as f64).map_err(|e| e.to_string());
            }
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
                self.i += 1;
            }
            if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
                let save = self.i;
                self.i += 1;
                if matches!(self.s.get(self.i), Some(b'+' | b'-')) {
                    self.i += 1;
                }
                if self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                    while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                        self.i += 1;
                    }
                } else {
                    self.i = save;
                }
            }
            let txt = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("0");
            return txt.parse().map_err(|_| format!("Invalid number '{txt}'"));
        }
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
            self.i += 1;
        }
        if self.i == start {
            return match self.s.get(self.i) {
                Some(_) => self.fail("Unexpected token", self.i + 2, 1),
                None => self.fail("Too few arguments", self.s.len() + 1, 1),
            };
        }
        let name = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("").to_string();
        match name.as_str() {
            "pi" => return Ok(std::f64::consts::PI),
            "e" => return Ok(std::f64::consts::E),
            "tau" => return Ok(std::f64::consts::TAU),
            _ => {}
        }
        // an unknown name is reported before its arguments are looked at
        if matches!(math_fn(&name, &[1.0, 1.0]), Err(e) if e.starts_with("Unknown")) {
            return self.fail("Unknown function", start + 1, name.len());
        }
        let args = if self.eat("(") {
            let mut v = vec![self.expr()?];
            while self.eat(",") {
                v.push(self.expr()?);
            }
            if !self.eat(")") {
                return self.fail("Missing closing parenthesis", self.s.len() + 1, 1);
            }
            v
        } else {
            // `math sqrt 16`, `math max 1,5,3`: arguments follow without parentheses
            let mut v = vec![self.unary()?];
            while self.eat(",") {
                v.push(self.unary()?);
            }
            v
        };
        let len = self.s.len();
        math_fn(&name, &args).or_else(|e| self.fail(&e, len + 1, 1))
    }
}

fn math_fn(name: &str, a: &[f64]) -> Result<f64, String> {
    let one = || a.first().copied().ok_or_else(|| "Too few arguments".to_string());
    let two = || match a {
        [x, y, ..] => Ok((*x, *y)),
        _ => Err("Too few arguments".to_string()),
    };
    Ok(match name {
        "abs" => one()?.abs(),
        "acos" => one()?.acos(),
        "asin" => one()?.asin(),
        "atan" => one()?.atan(),
        "atan2" => {
            let (y, x) = two()?;
            y.atan2(x)
        }
        "ceil" => one()?.ceil(),
        "cos" => one()?.cos(),
        "cosh" => one()?.cosh(),
        "exp" => one()?.exp(),
        "fac" => (1..=(one()? as u64)).map(|n| n as f64).product(),
        "floor" => one()?.floor(),
        "ln" => one()?.ln(),
        "log" | "log10" => one()?.log10(),
        "log2" => one()?.log2(),
        "round" => one()?.round(),
        "sign" => {
            let x = one()?;
            if x == 0.0 { 0.0 } else { x.signum() }
        }
        "sin" => one()?.sin(),
        "sinh" => one()?.sinh(),
        "sqrt" => one()?.sqrt(),
        "tan" => one()?.tan(),
        "tanh" => one()?.tanh(),
        "pow" => {
            let (x, y) = two()?;
            x.powf(y)
        }
        "max" => a.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "min" => a.iter().copied().fold(f64::INFINITY, f64::min),
        "ncr" => {
            let (n, k) = two()?;
            let k = k.min(n - k);
            (0..k as u64).fold(1.0, |acc, i| acc * (n - i as f64) / (i as f64 + 1.0)).round()
        }
        "npr" => {
            let (n, k) = two()?;
            (0..k as u64).fold(1.0, |acc, i| acc * (n - i as f64))
        }
        "bitand" => {
            let (x, y) = two()?;
            ((x as i64) & (y as i64)) as f64
        }
        "bitor" => {
            let (x, y) = two()?;
            ((x as i64) | (y as i64)) as f64
        }
        "bitxor" => {
            let (x, y) = two()?;
            ((x as i64) ^ (y as i64)) as f64
        }
        _ => return Err("Unknown function".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(src: &str) -> String {
        format_math(eval_math(src).unwrap(), 6, None).unwrap()
    }

    #[test]
    fn math_matches_fish_formatting() {
        assert_eq!(m("1+2"), "3");
        assert_eq!(m("7/2"), "3.5");
        assert_eq!(m("2/3"), "0.666667");
        assert_eq!(m("2^10"), "1024");
        assert_eq!(m("0.1+0.2"), "0.3");
        assert_eq!(m("1e3"), "1000");
        assert_eq!(m("sqrt 16"), "4");
        assert_eq!(m("max 1,5,3"), "5");
        assert_eq!(m("1 + 2 * 3"), "7");
        assert_eq!(m("10 % 3"), "1");
        assert_eq!(format_math(3.5, 0, None).unwrap(), "3");
        assert_eq!(format_math(255.0, 6, Some("hex")).unwrap(), "0xff");
        assert!(eval_math("5 / 0").is_err());
    }

    #[test]
    fn sub_positions_follow_fish() {
        let sub = |s: &str, a: &[&str]| {
            let args: Vec<String> = a.iter().map(|x| x.to_string()).chain([s.to_string()]).collect();
            let o = parse_opts(&args, "sel", &[]).unwrap();
            let mut out = Out::new(false, false);
            str_sub(&o, &mut out);
            String::from_utf8(out.buf).unwrap()
        };
        assert_eq!(sub("abcdef", &["-s", "2", "-l", "3"]), "bcd\n");
        assert_eq!(sub("abcdef", &["-s", "2", "-e", "4"]), "bcd\n");
        assert_eq!(sub("abcdef", &["-s", "-3"]), "def\n");
        assert_eq!(sub("abcdef", &["-e", "-2"]), "abcd\n");
    }

    #[test]
    fn escape_round_trip_and_styles() {
        assert_eq!(escape("a b", "script"), "'a b'");
        assert_eq!(escape("a b/", "url"), "a%20b/");
        assert_eq!(unescape("a%20b", "url").as_deref(), Some("a b"));
        assert_eq!(unescape("%zz", "url"), None);
    }
}

/// Length of the token at the start of `s`: a run of word characters, else one character.
fn next_token_len(s: &str) -> usize {
    let word = s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).unwrap_or(s.len());
    if word > 0 { word } else { s.chars().next().map_or(0, char::len_utf8) }
}

// ---- test --------------------------------------------------------------------------

/// A node of fish's `test` expression.
enum TestExpr {
    Str(String),
    Unary(String, String),
    Binary(String, String, String),
    Not(Box<TestExpr>),
    And(Box<TestExpr>, Box<TestExpr>),
    Or(Box<TestExpr>, Box<TestExpr>),
}

struct TestParser<'a> {
    toks: &'a [String],
    i: usize,
}

const TEST_UNARY: &str = "-b -c -d -e -f -G -g -h -k -L -O -p -r -S -s -t -u -w -x -n -z";
const TEST_BINARY: &str = "= == != -eq -ne -gt -ge -lt -le -nt -ot -ef";

impl TestParser<'_> {
    fn peek(&self) -> Option<&str> {
        self.toks.get(self.i).map(String::as_str)
    }
    fn or(&mut self) -> Result<TestExpr, String> {
        let mut l = self.and()?;
        while self.peek() == Some("-o") {
            self.i += 1;
            l = TestExpr::Or(Box::new(l), Box::new(self.and()?));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<TestExpr, String> {
        let mut l = self.not()?;
        while self.peek() == Some("-a") {
            self.i += 1;
            l = TestExpr::And(Box::new(l), Box::new(self.not()?));
        }
        Ok(l)
    }
    fn not(&mut self) -> Result<TestExpr, String> {
        if self.peek() == Some("!") && self.toks.len() > self.i + 1 {
            self.i += 1;
            return Ok(TestExpr::Not(Box::new(self.not()?)));
        }
        self.primary()
    }
    fn primary(&mut self) -> Result<TestExpr, String> {
        let Some(t) = self.peek().map(str::to_string) else {
            return Err(format!("Missing argument at index {}", self.i + 1));
        };
        if t == "(" {
            self.i += 1;
            let e = self.or()?;
            if self.peek() != Some(")") {
                return Err(format!("Missing argument at index {}", self.i + 1));
            }
            self.i += 1;
            return Ok(e);
        }
        let left = self.toks.get(self.i + 1).map(String::as_str);
        let right = self.toks.get(self.i + 2).map(String::as_str);
        // a binary operator in the middle wins over everything else: `test = = =`
        if let Some(op) = left.filter(|o| TEST_BINARY.split(' ').any(|b| b == *o)) {
            let Some(r) = right else {
                return Err(format!("Missing argument at index {}", self.i + 3));
            };
            let e = TestExpr::Binary(op.to_string(), t, r.to_string());
            self.i += 3;
            return Ok(e);
        }
        if TEST_UNARY.split(' ').any(|u| u == t) {
            // `test -n` alone is the string "-n"
            if let Some(arg) = left {
                let e = TestExpr::Unary(t, arg.to_string());
                self.i += 2;
                return Ok(e);
            }
        }
        self.i += 1;
        Ok(TestExpr::Str(t))
    }
}

fn test_number(s: &str) -> Result<f64, String> {
    let t = s.trim();
    let hex = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X"));
    let v = match hex {
        Some(h) => i64::from_str_radix(h, 16).ok().map(|n| n as f64),
        None => t.parse::<f64>().ok().filter(|v| v.is_finite()),
    };
    v.ok_or_else(|| format!("Argument is not a number: '{s}'"))
}

fn test_file(op: &str, path: &str) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    let meta = std::fs::metadata(path);
    let lmeta = || std::fs::symlink_metadata(path);
    let access = |mode| {
        std::ffi::CString::new(path).is_ok_and(|c| unsafe { libc::access(c.as_ptr(), mode) } == 0)
    };
    match op {
        "-e" => meta.is_ok(),
        "-f" => meta.is_ok_and(|m| m.is_file()),
        "-d" => meta.is_ok_and(|m| m.is_dir()),
        "-L" | "-h" => lmeta().is_ok_and(|m| m.file_type().is_symlink()),
        "-p" => meta.is_ok_and(|m| m.file_type().is_fifo()),
        "-S" => meta.is_ok_and(|m| m.file_type().is_socket()),
        "-b" => meta.is_ok_and(|m| m.file_type().is_block_device()),
        "-c" => meta.is_ok_and(|m| m.file_type().is_char_device()),
        "-s" => meta.is_ok_and(|m| m.len() > 0),
        "-r" => access(libc::R_OK),
        "-w" => access(libc::W_OK),
        "-x" => access(libc::X_OK),
        "-u" => meta.is_ok_and(|m| m.permissions().mode() & 0o4000 != 0),
        "-g" => meta.is_ok_and(|m| m.permissions().mode() & 0o2000 != 0),
        "-k" => meta.is_ok_and(|m| m.permissions().mode() & 0o1000 != 0),
        "-O" => meta.is_ok_and(|m| m.uid() == unsafe { libc::geteuid() }),
        "-G" => meta.is_ok_and(|m| m.gid() == unsafe { libc::getegid() }),
        "-t" => path.parse::<i32>().is_ok_and(|fd| unsafe { libc::isatty(fd) } == 1),
        _ => false,
    }
}

fn test_eval(e: &TestExpr) -> Result<bool, String> {
    use std::os::unix::fs::MetadataExt;
    Ok(match e {
        TestExpr::Str(s) => !s.is_empty(),
        TestExpr::Not(x) => !test_eval(x)?,
        TestExpr::And(a, b) => test_eval(a)? && test_eval(b)?,
        TestExpr::Or(a, b) => test_eval(a)? || test_eval(b)?,
        TestExpr::Unary(op, arg) => match op.as_str() {
            "-n" => !arg.is_empty(),
            "-z" => arg.is_empty(),
            _ => test_file(op, arg),
        },
        TestExpr::Binary(op, a, b) => match op.as_str() {
            "=" | "==" => a == b,
            "!=" => a != b,
            "-nt" | "-ot" => {
                let (ma, mb) = (std::fs::metadata(a), std::fs::metadata(b));
                match (ma, mb) {
                    (Ok(x), Ok(y)) if op == "-nt" => x.mtime() > y.mtime() || (x.mtime() == y.mtime() && x.mtime_nsec() > y.mtime_nsec()),
                    (Ok(x), Ok(y)) => x.mtime() < y.mtime() || (x.mtime() == y.mtime() && x.mtime_nsec() < y.mtime_nsec()),
                    (Ok(_), Err(_)) => op == "-nt",
                    (Err(_), Ok(_)) => op == "-ot",
                    _ => false,
                }
            }
            "-ef" => match (std::fs::metadata(a), std::fs::metadata(b)) {
                (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
                _ => false,
            },
            _ => {
                let (x, y) = (test_number(a)?, test_number(b)?);
                match op.as_str() {
                    "-eq" => x == y,
                    "-ne" => x != y,
                    "-gt" => x > y,
                    "-ge" => x >= y,
                    "-lt" => x < y,
                    _ => x <= y,
                }
            }
        },
    })
}

/// fish's `test` / `[`: its own grammar (numbers may be floats or hex, `-n` alone is a
/// string, errors are status 2), not zsh's.
fn test(args: &[String]) -> i32 {
    let mut toks = args;
    if toks.first().is_some_and(|a| a == "--bracket") {
        toks = &toks[1..];
        if toks.last().map(String::as_str) != Some("]") {
            eprintln!("[: the last argument must be ']'");
            return 2;
        }
        toks = &toks[..toks.len() - 1];
    }
    if toks.is_empty() {
        return 2;
    }
    let mut p = TestParser { toks, i: 0 };
    // a malformed expression is status 1, a value that is no number status 2
    let expr = match p.or() {
        Ok(e) if p.i < toks.len() => Err(format!("unexpected argument at index {}: '{}'", p.i + 1, toks[p.i])),
        other => other,
    };
    let expr = match expr {
        Ok(e) => e,
        Err(e) => {
            eprintln!("test: {e}");
            return 1;
        }
    };
    match test_eval(&expr) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(e) => {
            eprintln!("test: {e}");
            2
        }
    }
}
