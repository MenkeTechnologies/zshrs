//! The fish builtins that name variables and so cannot be ordinary commands in
//! zsh: `set`, `read`, `alias` and `abbr`. Each is rewritten to the zsh
//! construct with the same effect on the caller's scope.

use super::emit::{fish_escape, quote_lit, squote, zname, Emitter};
use super::parse::{Part, Word};

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
    show: bool,
    /// `--path`: the values are colon-separated lists to split.
    path: bool,
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
                "--long" => Some('L'),
                "--path" => Some('P'),
                "--unpath" => Some('N'),
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
                    'S' => {
                        f.list = true;
                        f.show = true;
                    }
                    'n' | 'L' => f.list = true,
                    'P' => f.path = true,
                    _ => {}
                }
            }
            i += 1;
        }
        (f, i)
    }

    /// `name` or `name[idx…]` from the first operand: the zsh parameter and the raw subscript.
    fn set_target(&self, w: &Word) -> Option<(String, Option<String>)> {
        if let Some(t) = w.text() {
            return match t.split_once('[') {
                Some((n, rest)) => {
                    let idx = rest.strip_suffix(']')?;
                    Some((n.to_string(), Some(idx.to_string())))
                }
                None => Some((t, None)),
            };
        }
        // `name[$i]`: the index is built from variables
        let (Some(Part::Lit(first)), Some(Part::Lit(last))) = (w.parts.first(), w.parts.last()) else {
            return None;
        };
        let (name, head) = first.split_once('[')?;
        let tail = last.strip_suffix(']')?;
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        let mut idx = head.to_string();
        let middle = &w.parts[1..w.parts.len() - 1];
        for p in middle {
            match p {
                Part::Lit(t) => idx.push_str(t),
                Part::Var(v) if v.index.is_none() && !v.deref => {
                    idx.push_str(&format!("${{(j.:.){}}}", self.zn(&v.name)))
                }
                _ => return None,
            }
        }
        if w.parts.len() > 1 {
            idx.push_str(tail);
        }
        Some((name.to_string(), Some(idx)))
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
        let text = self.set_watched(args, &f, at);
        // `set -e NAME` with no scope flag erases the universal variable too, when that is
        // what the name was (the file is left alone if it does not hold it)
        let plain_erase = f.erase && !f.universal && !f.local && !f.global;
        if (!f.universal && !plain_erase) || f.query || f.list {
            return text;
        }
        let names: Vec<String> = args[at..]
            .iter()
            .filter_map(|w| self.set_target(w))
            .take(if f.erase { usize::MAX } else { 1 })
            .filter(|(_, idx)| !plain_erase || idx.is_none())
            .map(|(n, _)| self.zn(&n))
            .collect();
        if names.is_empty() {
            return text; // `set -U` alone lists the universal variables
        }
        let call = if f.erase { "__fish_uvar_erase" } else { "__fish_uvar_save" };
        let saves: Vec<String> = names.iter().map(|n| format!("{call} {n}")).collect();
        if f.erase {
            // (the erase's own status, 4 for a variable that was not set, comes last)
            return format!("{}; {text}", saves.join("; "));
        }
        format!("{text}; {}", saves.join("; "))
    }

    /// `set NAME …` plus what must follow it: `--path` records that the variable is a
    /// path variable, and a variable some `--on-variable` handler watches (see
    /// `watched_vars`) gets its handlers called with fish's `VARIABLE SET|ERASE NAME`.
    fn set_watched(&mut self, args: &[Word], f: &SetFlags, at: usize) -> String {
        let mut text = self.set_inner(args);
        if f.query || f.list || f.show {
            return text;
        }
        let Some((name, None)) = args.get(at).and_then(|w| self.set_target(w)) else {
            return text;
        };
        if f.path {
            text = format!("{text}; __fish_pathvars[{name}]=1");
        }
        if !self.watched_vars.contains(&name) {
            return text;
        }
        let what = if f.erase { "ERASE" } else { "SET" };
        format!(
            "{{ {text}; __fish_rc=$?; (( ${{+__fish_events[var:{name}]}} )) && __fish_var_event {name} {what}; (exit $__fish_rc); }}"
        )
    }

    fn set_inner(&mut self, args: &[Word]) -> String {
        let (f, at) = self.set_flags(args);
        let operands = &args[at..];
        let Some((name, idx)) = operands.first().and_then(|w| self.set_target(w)) else {
            if operands.is_empty() {
                let flags: Vec<String> = args[..at].iter().map(|w| self.word_scalar(w)).collect();
                return self.set_list_call(flags);
            }
            // `set $name …`: the variable is named at run time; hand it to the shim
            let all: Vec<String> = args.iter().map(|w| self.word_list(w)).collect();
            return format!("__fish_set_dynamic {}", all.join(" "));
        };
        let presplit = |w: &Word| matches!(w.parts.as_slice(), [Part::Raw(r)] if r.starts_with("${(@s.:.)"));
        if f.path && idx.is_none() && operands.len() > 1 && !operands[1..].iter().all(presplit) {
            // path variables hold the pieces of their colon-separated value
            let mut split: Vec<Word> = args[..at].to_vec();
            split.push(operands[0].clone());
            for w in &operands[1..] {
                let raw = match w.text() {
                    Some(t) => format!("${{(@s.:.)${{:-{}}}}}", squote(&t)),
                    None => format!("${{(@s.:.)${{(j.:.){}}}}}", self.word_list(w)),
                };
                split.push(Word { parts: vec![Part::Raw(raw)] });
            }
            return self.set_inner(&split);
        }
        let mut z = self.zn(&name);
        let values = &operands[1..];

        if f.list {
            // `set -S name`: how fish describes a variable
            if !operands.is_empty() && f.show {
                let calls: Vec<String> = operands
                    .iter()
                    .filter_map(|w| self.set_target(w))
                    .map(|(n, _)| format!("__fish_set_show {} {}", self.zn(&n), crate::fish::emit::squote(&n)))
                    .collect();
                return calls.join("; ");
            }
            let flags: Vec<String> = args[..at].iter().map(|w| self.word_scalar(w)).collect();
            return self.set_list_call(flags);
        }
        if f.erase {
            return operands
                .iter()
                .filter_map(|w| self.set_target(w))
                .map(|(n, i)| match i {
                    Some(i) => format!("{}[{}]=()", self.zn(&n), Self::subscript(&i)),
                    // erasing a variable that is not set is status 4
                    None => {
                        let z = self.zn(&n);
                        format!("if (( ${{+{z}}} )); then unset {z}; else (exit 4); fi")
                    }
                })
                .collect::<Vec<_>>()
                .join("; ");
        }
        if f.query {
            let tests: Vec<String> = operands
                .iter()
                .filter_map(|w| self.set_target(w))
                .map(|(n, i)| {
                    let zn = self.zn(&n);
                    match (n.as_str(), i) {
                        ("argv", None) => "1".to_string(),
                        // fish's own variables, which no zsh parameter stands for (`_` is not set)
                        ("_", None) => "0".to_string(),
                        (
                            "status" | "status_generation" | "fish_killring" | "fish_bind_mode" | "fish_private_mode" | "history"
                            | "pipestatus" | "hostname" | "fish_pid" | "COLUMNS" | "LINES" | "EUID" | "IFS",
                            None,
                        ) if zn == zname(&n) => "1".to_string(),
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
            return self.set_export(&name, &z, values, &list, (f.local, f.append, f.prepend), f.path);
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
            let outer = self.zn(&name);
            z = self.declare_local(&name);
            let decl = if self.in_local_scope() { "local -a" } else { "typeset -ga" };
            // `set -q x; or set -l x 1`: when the declaration is skipped the later reads of
            // `$x` must still find the variable that is visible, so the local starts as a
            // copy of it
            if self.guarded && z != outer {
                self.pre_chain.push(format!("(( ${{+{outer}}} )) && {decl} {z}=(${{{outer}+\"${{{outer}[@]}}\"}})"));
            }
            return format!("{decl} {z}=({list})");
        }
        if f.global && self.fn_depth > 0 {
            return format!("typeset -ga {z}=({list})");
        }
        format!("{z}=({list})")
    }

    /// `set -x NAME value…`: zsh exports scalars, so the list is joined the way fish
    /// writes it into the environment.
    fn set_export(&mut self, name: &str, z: &str, values: &[Word], list: &str, scope: (bool, bool, bool), path: bool) -> String {
        let (local, append, prepend) = scope;
        let decl = if local && self.in_local_scope() { "local -x" } else { "typeset -gx" };
        let sep = if path || name.ends_with("PATH") { ':' } else { ' ' };
        // `path`, `cdpath`, `manpath` are tied to their exported scalars already
        if matches!(name, "PATH" | "CDPATH" | "MANPATH") {
            let old = format!("\"${{{z}[@]}}\"");
            let items = match (append, prepend) {
                (true, _) => format!("{old} {list}"),
                (_, true) => format!("{list} {old}"),
                _ => list.to_string(),
            };
            // in a function `-l` hides the global until the function returns; the list is
            // evaluated before the declaration, so it can still read the old value
            return if local && self.in_local_scope() {
                format!("local -a {z}=({items})")
            } else {
                format!("{z}=({items})")
            };
        }
        // The list stays a list: `NAME` (exported, joined with `sep` for the child) is tied
        // to the array `__fx_NAME`, which is what the translator reads and writes.
        let tied = format!("__fx_{name}");
        let old = format!("${{{z}+\"${{{z}[@]}}\"}}");
        let items = match (append, prepend) {
            (true, _) => format!("{old} {list}"),
            (_, true) => format!("{list} {old}"),
            _ if values.is_empty() => old,
            _ => list.to_string(),
        };
        if local && self.in_local_scope() {
            let already = self.is_mapped(name, &tied);
            let decl = if already {
                String::new()
            } else {
                format!("local -xT {name} {tied} '{sep}'; ")
            };
            self.map_local(name, &tied);
            return format!("{{ _fish_x=({items}); {decl}{tied}=(\"${{_fish_x[@]}}\"); }}");
        }
        super::emit::note_exported(name);
        format!(
            "{{ _fish_x=({items}); if (( ! ${{+{tied}}} )); then unset {name} 2>/dev/null; typeset -gxT {name} {tied} '{sep}'; fi; {tied}=(\"${{_fish_x[@]}}\"); }}"
        )
    }

    pub(super) fn read_cmd(&mut self, args: &[Word]) -> String {
        let mut local = false;
        let mut flags = String::from("-r");
        let mut prompt = None;
        let mut as_list = false;
        let mut by_line = false;
        let mut delim: Option<String> = None;
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
                "-L" | "--line" => by_line = true,
                "-z" | "--null" => flags.push_str(" -d ''"),
                "-P" | "--prompt-str" => prompt = arg(self),
                "-n" | "--nchars" => {
                    if let Some(n) = arg(self) {
                        flags.push_str(&format!(" -u 0 -k {n}"));
                    }
                }
                "-d" | "--delimiter" => {
                    delim = arg(self).filter(|d| d != "''");
                }
                "-p" | "--prompt" | "-c" | "--command" | "-R" | "--right-prompt" | "-S" | "--shell" => {
                    let _ = arg(self);
                }
                // short flags that take no value may be written together: `-la`
                s if s.len() > 2 && s.starts_with('-') && !s.starts_with("--") && s[1..].chars().all(|c| "lgxUsaLz".contains(c)) => {
                    for c in s[1..].chars() {
                        match c {
                            'l' => local = true,
                            's' => flags.push('s'),
                            'a' => as_list = true,
                            'L' => by_line = true,
                            'z' => flags.push_str(" -d ''"),
                            _ => {}
                        }
                    }
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
        if local && self.in_local_scope() {
            for n in &names {
                pre.push_str(&if as_list {
                    format!("local -a {n}=(); ")
                } else {
                    format!("local {n}=; ")
                });
            }
        }
        if by_line {
            // one line per variable
            let reads: Vec<String> = names.iter().map(|n| format!("IFS= builtin read {flags} {n}")).collect();
            return format!("{pre}{{ {}; }}", reads.join(" && "));
        }
        let first = match &prompt {
            Some(p) => format!("{}?{}", names[0], p.trim_matches('\'')),
            None => names[0].clone(),
        };
        let mut targets = vec![if prompt.is_some() { squote(&first) } else { first }];
        targets.extend(names[1..].iter().cloned());
        // (a group, so the declarations travel with the read when it is a pipeline's last stage)
        let group = |text: String| if pre.is_empty() { text } else { format!("{{ {text}; }}") };
        if as_list {
            let ifs = delim.map_or(String::new(), |d| format!("IFS={d} "));
            return group(format!("{pre}{ifs}builtin read {flags}A {}", targets.join(" ")));
        }
        // one variable takes the whole line, as fish does; several split on IFS
        let ifs = match (&delim, names.len()) {
            (_, 1) => "IFS= ".to_string(),
            (Some(d), _) => format!("IFS={d} "),
            _ => String::new(),
        };
        group(format!("{pre}{ifs}builtin read {flags} {}", targets.join(" ")))
    }

    /// `alias name 'command args'` / `alias name=command`: a function, as in fish.
    pub(super) fn alias_cmd(&mut self, args: &[Word]) -> String {
        let texts: Vec<String> = args
            .iter()
            .filter_map(Word::text)
            .filter(|t| !t.starts_with('-'))
            .collect();
        let (name, body, eq) = match texts.as_slice() {
            [one] => match one.split_once('=') {
                Some((n, b)) => (n.to_string(), b.to_string(), true),
                None => return "{ print -u2 -r -- 'alias: body cannot be empty'; (exit 1); }".into(),
            },
            [n, rest @ ..] => (n.clone(), rest.join(" "), false),
            [] => return "__fish_alias_list".into(),
        };
        let translated = match super::parse::parse(&body) {
            Ok(script) => self.script(&script),
            Err(_) => body.clone(),
        };
        // what `functions NAME` / `type NAME` print, and what `alias` lists
        let desc = if eq { format!("alias {name}={body}") } else { format!("alias {name} {body}") };
        let shown = format!(
            "function {} --wraps={} --description {}\n    {body} $argv\nend",
            fish_escape(&name),
            fish_escape(&body),
            fish_escape(&desc)
        );
        format!(
            "function {q} {{ {translated} \"$@\"; }}\n__fish_aliases[{q}]={}; __fish_fnsrc[{q}]={}; unset '__fish_fnline[{q}]' '__fish_fnfile[{q}]'",
            squote(&body),
            squote(&shown),
            q = quote_lit(&name)
        )
    }
}

impl Emitter {
    /// `set` with no names. In a function, `set -l` lists that function's locals only,
    /// which zsh's dynamic scope cannot tell apart, so the call names them.
    fn set_list_call(&self, flags: Vec<String>) -> String {
        let local = flags.iter().any(|f| f == "--local" || (f.starts_with('-') && !f.starts_with("--") && f.contains('l')));
        if !local || self.fn_depth == 0 {
            return format!("__fish_set_list {}", flags.join(" "));
        }
        let mine = self.scope_locals().join(" ");
        format!("{{ __fish_av=(\"${{argv[@]}}\"); __fish_set_list --in='{mine}' {}; }}", flags.join(" "))
    }
}
