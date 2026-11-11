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
    for name in ["string", "math", "count", "contains", "path"] {
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
        "contains" => contains(args),
        "path" => path(args),
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
    while lock.read_line(&mut line).unwrap_or(0) > 0 {
        if line.ends_with('\n') {
            line.pop();
        }
        out.push(std::mem::take(&mut line));
    }
    out
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
        if !self.quiet {
            self.buf.extend_from_slice(s.as_bytes());
            self.buf.push(if self.nul { 0 } else { b'\n' });
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
    let Some(sub) = args.first() else {
        return err("string", "missing subcommand");
    };
    let rest = &args[1..];
    // option letters differ per subcommand (`-r` is regex for match, right for pad)
    let (valued, long): (&str, Vec<(&str, char)>) = match sub.as_str() {
        "sub" => ("sel", vec![("start", 's'), ("end", 'e'), ("length", 'l'), ("quiet", 'q')]),
        "pad" => ("wc", vec![("width", 'w'), ("char", 'c'), ("right", 'r'), ("quiet", 'q')]),
        "trim" => ("c", vec![("chars", 'c'), ("left", 'l'), ("right", 'r'), ("quiet", 'q')]),
        "repeat" => ("nm", vec![("count", 'n'), ("max", 'm'), ("quiet", 'q'), ("no-newline", 'N')]),
        "split" => ("mf", vec![("max", 'm'), ("fields", 'f'), ("right", 'r'), ("no-empty", 'n'), ("quiet", 'q')]),
        "shorten" => ("mc", vec![("max", 'm'), ("char", 'c'), ("left", 'l'), ("no-newline", 'N'), ("quiet", 'q')]),
        "escape" | "unescape" => ("s", vec![("style", 's'), ("no-quote", 'n')]),
        "match" => ("", vec![("all", 'a'), ("entire", 'e'), ("regex", 'r'), ("invert", 'v'), ("ignore-case", 'i'), ("groups-only", 'g'), ("index", 'n'), ("quiet", 'q')]),
        "replace" => ("", vec![("all", 'a'), ("regex", 'r'), ("ignore-case", 'i'), ("filter", 'f'), ("quiet", 'q')]),
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
    let quiet = o.has('q');
    let mut out = Out::new(quiet, o.has('z') && matches!(sub.as_str(), "split0" | "join0"));
    let status = match sub.as_str() {
        "length" => {
            let mut any = false;
            for s in inputs(&o.rest, false) {
                let n = s.chars().count();
                any |= n > 0;
                out.line(&n.to_string());
            }
            i32::from(!any)
        }
        "upper" | "lower" => {
            let up = sub == "upper";
            let mut any = false;
            for s in inputs(&o.rest, false) {
                out.line(&if up { s.to_uppercase() } else { s.to_lowercase() });
                any = true;
            }
            i32::from(!any)
        }
        "sub" => str_sub(&o, &mut out),
        "pad" => str_pad(&o, &mut out),
        "trim" => str_trim(&o, &mut out),
        "repeat" => str_repeat(&o, &mut out),
        "join" | "join0" => {
            let (sep, items): (String, Vec<String>) = if sub == "join" {
                match o.rest.split_first() {
                    Some((s, r)) => (s.clone(), if r.is_empty() { inputs(&[], false) } else { r.to_vec() }),
                    None => return err("string join", "missing argument"),
                }
            } else {
                ("\0".into(), inputs(&o.rest, false))
            };
            let joined = items.join(&sep);
            if sub == "join0" {
                if !quiet {
                    let mut so = std::io::stdout().lock();
                    let _ = so.write_all(joined.as_bytes());
                    let _ = so.write_all(b"\0");
                }
            } else {
                out.line(&joined);
            }
            i32::from(items.len() < 2)
        }
        "split" | "split0" => str_split(&o, &mut out, sub == "split0"),
        "collect" => {
            let all = inputs(&o.rest, false).join("\n");
            let all = if o.has('N') { all } else { all.trim_end_matches('\n').to_string() };
            out.line(&all);
            i32::from(all.is_empty())
        }
        "match" => str_match(&o, &mut out),
        "replace" => str_replace(&o, &mut out),
        "escape" => {
            let style = o.val('s').unwrap_or("script");
            for s in inputs(&o.rest, false) {
                out.line(&escape(&s, style));
            }
            0
        }
        "unescape" => {
            for s in inputs(&o.rest, false) {
                out.line(&unescape(&s, o.val('s').unwrap_or("script")));
            }
            0
        }
        "shorten" => str_shorten(&o, &mut out),
        other => {
            return err("string", &format!("{other}: invalid subcommand"));
        }
    };
    out.flush();
    status
}

/// fish accepts `--` between a subcommand's own operands (`string split , -- $x`), not
/// only before them.
fn without_dashdash(o: &Opts) -> Opts {
    Opts {
        flags: o.flags.clone(),
        vals: o.vals.clone(),
        rest: o.rest.iter().filter(|a| a.as_str() != "--").cloned().collect(),
    }
}

fn str_sub(o: &Opts, out: &mut Out) -> i32 {
    let mut any = false;
    for s in inputs(&o.rest, false) {
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
    let items = inputs(&o.rest, false);
    let fill = o.val('c').and_then(|c| c.chars().next()).unwrap_or(' ');
    let width = o
        .num('w')
        .map(|w| w.max(0) as usize)
        .unwrap_or_else(|| items.iter().map(|s| s.chars().count()).max().unwrap_or(0));
    for s in &items {
        let pad = width.saturating_sub(s.chars().count());
        let p: String = std::iter::repeat_n(fill, pad).collect();
        out.line(&if o.has('r') { format!("{s}{p}") } else { format!("{p}{s}") });
    }
    0
}

fn str_trim(o: &Opts, out: &mut Out) -> i32 {
    let set: Vec<char> = o.val('c').map_or(vec![' ', '\t', '\n', '\r', '\x0c', '\x0b'], |c| c.chars().collect());
    let (left, right) = (o.has('l') || !o.has('r'), o.has('r') || !o.has('l'));
    let mut trimmed = false;
    for s in inputs(&o.rest, false) {
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
    let n = o.num('n').unwrap_or(0).max(0) as usize;
    let max = o.num('m').map(|m| m.max(0) as usize);
    let mut any = false;
    for s in inputs(&o.rest, false) {
        let mut r = s.repeat(n);
        if let Some(m) = max {
            r = r.chars().take(m).collect();
        }
        any |= !r.is_empty();
        out.line(&r);
    }
    i32::from(!any)
}

fn str_split(o: &Opts, out: &mut Out, nul: bool) -> i32 {
    let o = &without_dashdash(o);
    let (sep, items) = if nul {
        ("\0".to_string(), inputs(&o.rest, true))
    } else {
        match o.rest.split_first() {
            Some((s, r)) => (s.clone(), if r.is_empty() { inputs(&[], false) } else { r.to_vec() }),
            None => return err("string split", "missing argument"),
        }
    };
    let max = o.num('m').map_or(usize::MAX, |m| m.max(0) as usize);
    let mut splits = false;
    let mut all: Vec<String> = Vec::new();
    for s in items {
        let mut parts: Vec<String> = if sep.is_empty() {
            s.chars().map(String::from).collect()
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
        splits |= parts.len() > 1;
        all.extend(match o.num('f') {
            Some(f) if f >= 1 => parts.get(f as usize - 1).cloned().into_iter().collect(),
            _ => parts,
        });
    }
    for p in &all {
        out.line(p);
    }
    i32::from(!splits)
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
    let items = if items.is_empty() { inputs(&[], false) } else { items.to_vec() };
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
    let mut matched = 0usize;
    for s in &items {
        let mut hit = false;
        let caps_iter: Vec<fancy_regex::Captures> = if o.has('a') && regex {
            re.captures_iter(s).flatten().collect()
        } else {
            re.captures(s).ok().flatten().into_iter().collect()
        };
        for caps in caps_iter {
            hit = true;
            let whole = caps.get(0).unwrap();
            if o.has('v') {
                continue;
            }
            if o.has('n') {
                let start = s[..whole.start()].chars().count() + 1;
                out.line(&format!("{} {}", start, whole.as_str().chars().count()));
                continue;
            }
            if regex {
                if !o.has('g') {
                    out.line(whole.as_str());
                }
                for k in 1..caps.len() {
                    out.line(caps.get(k).map_or("", |m| m.as_str()));
                }
            } else {
                out.line(if o.has('e') { whole.as_str() } else { s });
            }
        }
        if o.has('v') && !hit {
            out.line(s);
            matched += 1;
        } else if hit && !o.has('v') {
            matched += 1;
        }
    }
    i32::from(matched == 0)
}

/// `$1`/`${1}` in a replacement (regex mode) as the capture text.
fn expand_replacement(rep: &str, caps: &fancy_regex::Captures) -> String {
    let mut out = String::new();
    let chars: Vec<char> = rep.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            let (name, next) = if chars.get(i + 1) == Some(&'{') {
                let close = chars[i + 2..].iter().position(|&c| c == '}').map(|p| p + i + 2);
                match close {
                    Some(c) => (chars[i + 2..c].iter().collect::<String>(), c + 1),
                    None => (String::new(), i),
                }
            } else {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                (chars[i + 1..j].iter().collect(), j)
            };
            if !name.is_empty() {
                let m = name
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| caps.get(n))
                    .or_else(|| caps.name(&name));
                out.push_str(m.map_or("", |m| m.as_str()));
                i = next;
                continue;
            }
        }
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn str_replace(o: &Opts, out: &mut Out) -> i32 {
    let o = &without_dashdash(o);
    let (Some(pat), Some(rep)) = (o.rest.first(), o.rest.get(1)) else {
        return err("string replace", "missing argument");
    };
    let items = if o.rest.len() > 2 { o.rest[2..].to_vec() } else { inputs(&[], false) };
    let icase = o.has('i');
    let regex = o.has('r');
    if pat.is_empty() && !regex {
        for s in &items {
            if !o.has('f') {
                out.line(s);
            }
        }
        return 1;
    }
    let re = if regex {
        match compile(pat, icase) {
            Ok(r) => r,
            Err(e) => return err("string replace", &e),
        }
    } else {
        match compile(&fancy_regex::escape(pat), icase) {
            Ok(r) => r,
            Err(e) => return err("string replace", &e),
        }
    };
    let mut changed = false;
    for s in &items {
        let mut result = String::new();
        let mut last = 0;
        let mut any = false;
        for caps in re.captures_iter(s).flatten() {
            let m = caps.get(0).unwrap();
            if m.start() == m.end() && m.start() >= s.len() && any {
                break;
            }
            any = true;
            result.push_str(&s[last..m.start()]);
            if regex {
                result.push_str(&expand_replacement(rep, &caps));
            } else {
                result.push_str(rep);
            }
            last = m.end();
            if !o.has('a') {
                break;
            }
        }
        result.push_str(&s[last..]);
        changed |= any;
        if any || !o.has('f') {
            out.line(&result);
        }
    }
    i32::from(!changed)
}

fn str_shorten(o: &Opts, out: &mut Out) -> i32 {
    let ell = o.val('c').unwrap_or("…");
    let max = o.num('m').unwrap_or(0).max(0) as usize;
    let mut shortened = false;
    for s in inputs(&o.rest, false) {
        let n = s.chars().count();
        if n <= max {
            out.line(&s);
            continue;
        }
        shortened = true;
        let keep = max.saturating_sub(ell.chars().count());
        let piece: String = if o.has('l') {
            let tail: Vec<char> = s.chars().skip(n - keep).collect();
            format!("{ell}{}", tail.iter().collect::<String>())
        } else {
            format!("{}{ell}", s.chars().take(keep).collect::<String>())
        };
        out.line(&piece);
    }
    i32::from(!shortened)
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
            } else if s.chars().any(|c| (c as u32) < 0x20) {
                s.chars()
                    .map(|c| match c {
                        '\n' => "\\n".to_string(),
                        '\t' => "\\t".to_string(),
                        c if (c as u32) < 0x20 => format!("\\x{:02x}", c as u32),
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

fn unescape(s: &str, style: &str) -> String {
    if style == "url" {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 3 <= b.len() {
                if let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        return String::from_utf8_lossy(&out).into_owned();
    }
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            },
            '\'' | '"' => {}
            c => out.push(c),
        }
    }
    out
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
    let long = [("quiet", 'q'), ("invert", 'v'), ("reverse", 'r'), ("null-in", 'z'), ("null-out", 'Z'), ("type", 't'), ("perm", 'p'), ("key", 'k')];
    let o = match parse_opts(&args[1..], "tpk", &long) {
        Ok(o) => o,
        Err(e) => return err(&format!("path {sub}"), &e),
    };
    let mut out = Out::new(o.has('q'), o.has('Z'));
    let items = inputs(&o.rest, o.has('z'));
    let status = match sub.as_str() {
        "basename" | "dirname" | "extension" | "normalize" | "resolve" => {
            let mut any = false;
            for p in &items {
                let r = match sub.as_str() {
                    "basename" => base_name(p),
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
        "change-extension" => {
            let Some((ext, paths)) = o.rest.split_first() else {
                return err("path change-extension", "missing argument");
            };
            let paths = if paths.is_empty() { inputs(&[], false) } else { paths.to_vec() };
            for p in &paths {
                let e = extension(p);
                let stem = &p[..p.len() - e.len()];
                let dot = if ext.is_empty() || ext.starts_with('.') { "" } else { "." };
                out.line(&format!("{stem}{dot}{ext}"));
            }
            0
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

fn count(args: &[String]) -> i32 {
    let n = if args.is_empty() {
        let mut buf = String::new();
        if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            0
        } else {
            let _ = std::io::stdin().read_to_string(&mut buf);
            buf.lines().count()
        }
    } else {
        args.len()
    };
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
    let expr = o.rest.join(" ");
    let expr = if expr.trim().is_empty() {
        inputs(&[], false).join(" ")
    } else {
        expr
    };
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
        Err(e) => {
            eprintln!("math: Error: {e}\n'{expr}'");
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
            "-s" => {
                let v = take(&mut i)?;
                o.vals.insert('s', v);
            }
            "-b" => {
                let v = take(&mut i)?;
                o.vals.insert('b', v);
            }
            "-m" => {
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
    if v.abs() >= 9.2e18 {
        return None;
    }
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
}

fn eval_math(src: &str) -> Result<f64, String> {
    let mut p = MathParser {
        s: src.as_bytes(),
        i: 0,
    };
    let v = p.expr()?;
    p.ws();
    if p.i < p.s.len() {
        return Err(format!("Unexpected token '{}'", &src[p.i..]));
    }
    Ok(v)
}

impl MathParser<'_> {
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
            if self.s[self.i..].starts_with(b"**") {
                return Ok(v);
            }
            if self.eat("*") || self.eat("x") {
                v *= self.power()?;
            } else if self.eat("/") {
                let d = self.power()?;
                if d == 0.0 {
                    return Err("Division by zero".into());
                }
                v /= d;
            } else if self.eat("%") {
                let d = self.power()?;
                if d == 0.0 {
                    return Err("Division by zero".into());
                }
                v %= d;
            } else {
                return Ok(v);
            }
        }
    }
    fn power(&mut self) -> Result<f64, String> {
        let base = self.unary()?;
        if self.eat("**") || self.eat("^") {
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
                return Err("Expected ')'".into());
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
            return Err(match self.s.get(self.i) {
                Some(c) => format!("Unexpected token '{}'", *c as char),
                None => "Unexpected end of expression".to_string(),
            });
        }
        let name = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("").to_string();
        match name.as_str() {
            "pi" => return Ok(std::f64::consts::PI),
            "e" => return Ok(std::f64::consts::E),
            "tau" => return Ok(std::f64::consts::TAU),
            _ => {}
        }
        let args = if self.eat("(") {
            let mut v = vec![self.expr()?];
            while self.eat(",") {
                v.push(self.expr()?);
            }
            if !self.eat(")") {
                return Err("Expected ')'".into());
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
        math_fn(&name, &args)
    }
}

fn math_fn(name: &str, a: &[f64]) -> Result<f64, String> {
    let one = || a.first().copied().ok_or_else(|| format!("Too few arguments to '{name}'"));
    let two = || match a {
        [x, y, ..] => Ok((*x, *y)),
        _ => Err(format!("Too few arguments to '{name}'")),
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
        other => return Err(format!("Unknown function '{other}'")),
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
        assert_eq!(unescape("a%20b", "url"), "a b");
    }
}
