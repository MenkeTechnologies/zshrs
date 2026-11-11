//! The fish builtins that name variables and so cannot be ordinary commands in
//! zsh: `set`, `read`, `alias` and `abbr`. Each is rewritten to the zsh
//! construct with the same effect on the caller's scope.

use super::emit::{quote_lit, squote, Emitter};
use super::parse::Word;

/// Options of `set`, after the combined short flags (`-gx`) are split.
#[derive(Default)]
struct SetFlags {
    local: bool,
    global: bool,
    universal: bool,
    export: bool,
    unexport: bool,
    erase: bool,
    query: bool,
    append: bool,
    prepend: bool,
    list: bool,
}

impl Emitter {
    /// Flags first, then the operands. Returns the flags and the index of the first operand.
    fn set_flags(&self, args: &[Word]) -> (SetFlags, usize) {
        let mut f = SetFlags::default();
        let mut i = 0;
        while let Some(t) = args.get(i).and_then(Word::bare) {
            if t == "--" {
                i += 1;
                break;
            }
            let Some(opts) = t.strip_prefix('-').filter(|o| !o.is_empty()) else {
                break;
            };
            let long = |name: &str| match name {
                "--local" => Some('l'),
                "--global" => Some('g'),
                "--universal" => Some('U'),
                "--function" => Some('f'),
                "--export" => Some('x'),
                "--unexport" => Some('u'),
                "--erase" => Some('e'),
                "--query" => Some('q'),
                "--append" => Some('a'),
                "--prepend" => Some('p'),
                "--show" => Some('S'),
                "--names" => Some('n'),
                _ => None,
            };
            let letters: Vec<char> = if let Some(l) = opts.strip_prefix('-') {
                match long(&format!("--{l}")) {
                    Some(c) => vec![c],
                    None => break,
                }
            } else {
                opts.chars().collect()
            };
            for c in letters {
                match c {
                    'l' | 'f' => f.local = true,
                    'g' => f.global = true,
                    'U' => {
                        f.global = true;
                        f.universal = true;
                    }
                    'x' => f.export = true,
                    'u' => f.unexport = true,
                    'e' => f.erase = true,
                    'q' => f.query = true,
                    'a' => f.append = true,
                    'p' => f.prepend = true,
                    'S' | 'n' => f.list = true,
                    _ => {}
                }
            }
            i += 1;
        }
        (f, i)
    }

    /// `name` or `name[idx…]` from the first operand: the zsh parameter and the raw subscript.
    fn set_target(w: &Word) -> Option<(String, Option<String>)> {
        let t = w.text()?;
        match t.split_once('[') {
            Some((n, rest)) => {
                let idx = rest.strip_suffix(']')?;
                Some((n.to_string(), Some(idx.to_string())))
            }
            None => Some((t, None)),
        }
    }

    /// A fish index token as the zsh subscript it names (first token of the list).
    fn subscript(idx: &str) -> String {
        let first = idx.split_whitespace().next().unwrap_or("1");
        match first.split_once("..") {
            Some((a, b)) => format!("{a},{b}"),
            None => first.to_string(),
        }
    }

    /// `set -U` also writes the variable to the universal store; `set -U -e` removes it.
    pub(super) fn set_cmd(&mut self, args: &[Word]) -> String {
        let (f, at) = self.set_flags(args);
        let text = self.set_inner(args);
        if !f.universal || f.query || f.list {
            return text;
        }
        let names: Vec<String> = args[at..]
            .iter()
            .filter_map(Self::set_target)
            .take(if f.erase { usize::MAX } else { 1 })
            .map(|(n, _)| self.zn(&n))
            .collect();
        let call = if f.erase { "__fish_uvar_erase" } else { "__fish_uvar_save" };
        let saves: Vec<String> = names.iter().map(|n| format!("{call} {n}")).collect();
        format!("{text}; {}", saves.join("; "))
    }

    fn set_inner(&mut self, args: &[Word]) -> String {
        let (f, at) = self.set_flags(args);
        let operands = &args[at..];
        let Some((name, idx)) = operands.first().and_then(Self::set_target) else {
            if operands.is_empty() {
                return "__fish_set_list".into();
            }
            // `set $name …`: the variable is named at run time; hand it to the shim
            let all: Vec<String> = args.iter().map(|w| self.word_list(w)).collect();
            return format!("__fish_set_dynamic {}", all.join(" "));
        };
        let mut z = self.zn(&name);
        let values = &operands[1..];

        if f.list {
            return "__fish_set_list".into();
        }
        if f.erase {
            return operands
                .iter()
                .filter_map(Self::set_target)
                .map(|(n, i)| match i {
                    Some(i) => format!("{}[{}]=()", self.zn(&n), Self::subscript(&i)),
                    None => format!("unset {}", self.zn(&n)),
                })
                .collect::<Vec<_>>()
                .join("; ");
        }
        if f.query {
            let tests: Vec<String> = operands
                .iter()
                .filter_map(Self::set_target)
                .map(|(n, i)| {
                    let zn = self.zn(&n);
                    match (n.as_str(), i) {
                        ("argv", None) => "1".to_string(),
                        (_, None) => format!("${{+{zn}}}"),
                        (_, Some(i)) => format!("${{#{zn}}} >= {}", i.trim_start_matches('-')),
                    }
                })
                .collect();
            return format!("(( {} ))", tests.join(" && "));
        }

        // `set x[2] value`: one element
        if let Some(i) = idx {
            let v = values.first().map(|w| self.word_scalar(w)).unwrap_or_else(|| "''".into());
            return format!("{z}[{}]={v}", Self::subscript(&i));
        }

        let words: Vec<String> = values.iter().map(|w| self.word_list(w)).collect();
        let list = words.join(" ");

        if f.unexport {
            return format!("typeset -g +x {z}");
        }
        if f.export {
            return self.set_export(&name, &z, values, &list, f.local);
        }
        if f.append {
            return format!("{z}+=({list})");
        }
        if f.prepend {
            return format!("{z}=({list} \"${{{z}[@]}}\")");
        }
        // A fish local is invisible to the functions it calls, which zsh's dynamically
        // scoped `local` is not, so it lives under a name no other code refers to.
        if f.local {
            z = self.declare_local(&name);
            let decl = if self.fn_depth > 0 { "local -a" } else { "typeset -ga" };
            return format!("{decl} {z}=({list})");
        }
        if f.global && self.fn_depth > 0 {
            return format!("typeset -ga {z}=({list})");
        }
        format!("{z}=({list})")
    }

    /// `set -x NAME value…`: zsh exports scalars, so the list is joined the way fish
    /// writes it into the environment.
    fn set_export(&mut self, name: &str, z: &str, values: &[Word], list: &str, local: bool) -> String {
        let decl = if local && self.fn_depth > 0 { "local -x" } else { "typeset -gx" };
        let sep = if name.ends_with("PATH") { ':' } else { ' ' };
        // `path`, `cdpath`, `manpath` are tied to their exported scalars already
        if matches!(name, "PATH" | "CDPATH" | "MANPATH") {
            return format!("{z}=({list})");
        }
        if values.is_empty() {
            // export the existing value
            return format!("{decl} {z}=\"${{(j.{sep}.){z}}}\"");
        }
        let plain_single = values.len() == 1 && values[0].text().is_some();
        if plain_single {
            return format!("{decl} {z}={}", self.word_scalar(&values[0]));
        }
        format!("_fish_x=({list}); {decl} {z}=\"${{(j.{sep}.)_fish_x}}\"")
    }

    pub(super) fn read_cmd(&mut self, args: &[Word]) -> String {
        let mut local = false;
        let mut flags = String::from("-r");
        let mut prompt = None;
        let mut as_list = false;
        let mut names: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let t = args[i].text().unwrap_or_default();
            i += 1;
            let mut arg = |this: &mut Self| -> Option<String> {
                let w = args.get(i)?;
                i += 1;
                Some(this.word_scalar(w))
            };
            match t.as_str() {
                "-l" | "--local" => local = true,
                "-g" | "--global" | "-x" | "--export" | "-U" | "--universal" => {}
                "-s" | "--silent" => flags.push('s'),
                "-a" | "--list" => as_list = true,
                "-z" | "--null" => flags.push_str(" -d ''"),
                "-P" | "--prompt-str" => prompt = arg(self),
                "-n" | "--nchars" => {
                    if let Some(n) = arg(self) {
                        flags.push_str(&format!(" -k {n}"));
                    }
                }
                "-d" | "--delimiter" => {
                    if let Some(d) = arg(self) {
                        flags.push_str(&format!(" -d {d}"));
                    }
                }
                "-p" | "--prompt" | "-c" | "--command" | "-R" | "--right-prompt" | "-S" | "--shell" => {
                    let _ = arg(self);
                }
                "--" => {}
                s if s.starts_with('-') && s.len() > 1 => {}
                _ if local => names.push(self.declare_local(&t)),
                _ => names.push(self.zn(&t)),
            }
        }
        if names.is_empty() {
            return format!("builtin read {flags} REPLY");
        }
        let mut pre = String::new();
        if local && self.fn_depth > 0 {
            for n in &names {
                pre.push_str(&if as_list {
                    format!("local -a {n}=(); ")
                } else {
                    format!("local {n}=; ")
                });
            }
        }
        let first = match &prompt {
            Some(p) => format!("{}?{}", names[0], p.trim_matches('\'')),
            None => names[0].clone(),
        };
        let mut targets = vec![if prompt.is_some() { squote(&first) } else { first }];
        targets.extend(names[1..].iter().cloned());
        if as_list {
            return format!("{pre}builtin read {flags}A {}", targets.join(" "));
        }
        // one variable takes the whole line, as fish does; several split on IFS
        let ifs = if names.len() == 1 { "IFS= " } else { "" };
        format!("{pre}{ifs}builtin read {flags} {}", targets.join(" "))
    }

    /// `alias name 'command args'` / `alias name=command`: a function, as in fish.
    pub(super) fn alias_cmd(&mut self, args: &[Word]) -> String {
        let texts: Vec<String> = args
            .iter()
            .filter_map(Word::text)
            .filter(|t| !t.starts_with('-'))
            .collect();
        let (name, body) = match texts.as_slice() {
            [one] => match one.split_once('=') {
                Some((n, b)) => (n.to_string(), b.to_string()),
                None => return "alias".into(),
            },
            [n, rest @ ..] => (n.clone(), rest.join(" ")),
            [] => return "alias".into(),
        };
        let translated = match super::parse::parse(&body) {
            Ok(script) => self.script(&script),
            Err(_) => body,
        };
        format!("function {} {{ {translated} \"$@\"; }}", quote_lit(&name))
    }

    /// `abbr -a name expansion…`: a zsh alias, which expands at the prompt the same way.
    pub(super) fn abbr_cmd(&mut self, args: &[Word]) -> String {
        let texts: Vec<String> = args.iter().filter_map(Word::text).collect();
        let operands: Vec<&String> = texts
            .iter()
            .filter(|t| !(t.starts_with('-') && t.len() > 1))
            .collect();
        match operands.as_slice() {
            [name, exp @ ..] if !exp.is_empty() => {
                let body = exp.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
                format!("alias -- {}={}", quote_lit(name), squote(&body))
            }
            _ => ":".into(),
        }
    }
}
