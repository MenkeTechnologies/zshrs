//! The fish builtins that name variables and so cannot be ordinary commands in
//! zsh: `set`, `read`, `alias` and `abbr`. Each is rewritten to the zsh
//! construct with the same effect on the caller's scope.

use super::emit::{fish_escape, is_auto_pathvar, path_kind, quote_lit, squote, zname, Emitter, PathKind};
use super::parse::{Part, Word};

/// Options of `set`, after the combined short flags (`-gx`) are split.
#[derive(Default, Clone)]
struct SetFlags {
    local: bool,
    /// `-f`: the function scope (`local` is set too).
    function: bool,
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
    /// `--unpath`: the variable is not a path variable, whatever its name.
    unpath: bool,
    /// `-g` itself (`-U` sets `global` too, for the paths that treat both alike).
    g_flag: bool,
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
                    'l' => f.local = true,
                    'f' => {
                        f.local = true;
                        f.function = true;
                    }
                    'g' => {
                        f.global = true;
                        f.g_flag = true;
                    }
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
                    'S' | 's' => {
                        f.list = true;
                        f.show = true;
                    }
                    'n' | 'L' => f.list = true,
                    'P' => f.path = true,
                    'N' => f.unpath = true,
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

    /// `set -U` also writes the variable to the universal store; `set -U -e` removes it.
    pub(super) fn set_cmd(&mut self, args: &[Word]) -> String {
        let (f, at) = self.set_flags(args);
        let text = self.set_stored(args, &f, at);
        // an assignment that works leaves `$status` as it was; one that fails has its own
        // (`preserve_failure_exit_status`, builtins/set.rs:1046)
        if f.erase || f.query || f.list || f.show || args.len() == at {
            return text;
        }
        let one = !text.contains([';', '\n']) && !text.contains("&&") && !text.contains("||");
        let text = if one { text } else { format!("{{ {text}; }}") };
        format!("{{ __fish_ss=$?; {text} && __fish_ret $__fish_ss; }}")
    }

    fn set_stored(&mut self, args: &[Word], f: &SetFlags, at: usize) -> String {
        let f = f.clone();
        let text = self.set_watched(args, &f, at);
        // `set -e NAME` with no scope flag erases the universal variable too, when that is
        // what the name was (the file is left alone if it does not hold it)
        let plain_erase = f.erase && !f.universal && !f.local && !f.global;
        // (`set -e -U` is a pass of its own in `erase_cmd`)
        if (!f.universal && !plain_erase) || f.query || f.list || (f.erase && f.universal) {
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
        // `--path` / `--unpath` make it a path variable or not from now on; erasing it
        // drops that, and the next variable of the name goes by its name again
        if f.path || f.unpath {
            let kind = if f.path { ":" } else { "' '" };
            text = format!("{text}; __fish_pathvars[{name}]={kind}");
        } else if f.erase && super::emit::path_flags_seen() {
            let z = self.zn(&name);
            text = format!("{{ {text}; __fish_st __fish_rc; (( ${{+{z}}} )) || unset '__fish_pathvars[{name}]'; (exit $__fish_rc); }}");
        }
        if !self.watched_vars.contains(&name) {
            return text;
        }
        let what = if f.erase { "ERASE" } else { "SET" };
        format!(
            "{{ {text}; __fish_st __fish_rc; (( ${{+__fish_events[var:{name}]}} )) && __fish_var_event {name} {what}; (exit $__fish_rc); }}"
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
        // fish variable names are letters, digits and underscores
        if !f.query && !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return format!(
                "{{ print -u2 -r -- {}; print -u2; (exit 1); }}",
                squote(&format!("set: {name}: invalid variable name. See `help language#shell-variable-and-function-names`"))
            );
        }
        let mut z = self.zn(&name);
        let values = &operands[1..];

        if f.list {
            // `set -S name`: how fish describes a variable
            if !operands.is_empty() && f.show {
                let calls: Vec<String> = operands
                    .iter()
                    .filter_map(|w| self.set_target(w))
                    .filter(|(n, _)| n != "_")
                    .map(|(n, _)| {
                        let global = Self::global_of(&n);
                        let show = |z: &str| format!("__fish_set_show {z} {}", squote(&n));
                        // a local and the global it hides are both described, local first
                        // (`show`, builtins/set.rs:683)
                        match self.local_of(&n).filter(|l| *l != global && !self.auto_locals.contains(l)) {
                            Some(l) => format!("{{ {}; __fish_rc=$?; {} && __fish_rc=0; (exit $__fish_rc); }}", show(&l), show(&global)),
                            None => show(&self.zn(&n)),
                        }
                    })
                    .collect();
                return calls.join("; ");
            }
            let flags: Vec<String> = args[..at].iter().map(|w| self.word_scalar(w)).collect();
            return self.set_list_call(flags);
        }
        if f.erase {
            return self.erase_cmd(&f, operands);
        }
        if f.query {
            // an operand whose index is built from a list is one operand per word
            let single = operands.len() == 1;
            let mut indexed = false;
            let mut tests: Vec<String> = Vec::new();
            for w in operands {
                let Some((n, i)) = self.set_target(w) else { continue };
                let mut zn = self.zn(&n);
                // `-g` asks for the global, not a local of the same name
                let global_name = zname(&n);
                if f.global && !f.local && zn != global_name {
                    zn = global_name.clone();
                }
                if i.is_some() {
                    // every index out of range counts, an invalid index ends the query with
                    // status 1 (`query`, builtins/set.rs:574-611)
                    indexed = true;
                    let arr = format!("${{+{zn}}} ${{{zn}+\"${{{zn}[@]}}\"}}");
                    tests.push(match self.runtime_target(w) {
                        Some(words) => {
                            let none = if single { "; (( __fish_en )) || __fish_nq=255" } else { "" };
                            format!(
                                "{{ __fish_en=0 __fish_qe=0; for __fish_w in {words}; do (( ++__fish_en )); __fish_set_idx query \"${{${{__fish_w#*\\[}}%\\]}}\" {arr} || {{ __fish_nq=1 __fish_qe=1; break; }}; done{none}; (( ! __fish_qe )); }}"
                            )
                        }
                        None => format!("{{ __fish_set_idx query {} {arr} || {{ __fish_nq=1; false; }}; }}", Self::index_spec(i.as_deref().unwrap_or_default())),
                    });
                    continue;
                }
                let t = match n.as_str() {
                    "argv" => "1".to_string(),
                    // fish's own variables, which no zsh parameter stands for (`_` is not set)
                    "_" => "0".to_string(),
                    "status" | "status_generation" | "fish_killring" | "fish_history" | "fish_bind_mode" | "fish_private_mode" | "history"
                    | "pipestatus" | "hostname" | "fish_pid" | "COLUMNS" | "LINES" | "EUID" | "IFS"
                        if zn == zname(&n) =>
                    {
                        "1".to_string()
                    }
                    // `-l` asks for a local: a name that resolves to the plain global is local only when a function declared it
                    _ if f.local && !f.global && zn == global_name => format!("${{+{zn}}} && ${{#${{(M)${{(t){zn}}}:#*local*}}}}"),
                    _ => format!("${{+{zn}}}"),
                };
                // (the status is how many of the names are not set)
                tests.push(format!("(( {t} )) || (( ++__fish_nq ))"));
            }
            // (an invalid index stops at the operand it is in)
            let sep = if indexed { " && " } else { "; " };
            return format!("{{ __fish_nq=0; {}; (exit $__fish_nq); }}", tests.join(sep));
        }

        // `set x[2 4..5] a b c`: one value per index, through `__fish_set_idx`
        if let Some(i) = idx {
            let list: Vec<String> = values.iter().map(|w| self.word_list(w)).collect();
            let mode = if f.append || f.prepend { "slice" } else { "set" };
            let target_words = self.runtime_target(&operands[0]);
            let (pre, z) = self.set_scope_target(&f, &name);
            // fish splits a path variable's new list on `:` like any other (set_in_node,
            // env/impl/environment.rs:940)
            let store = match self.path_split_cmd(&name, &f, "\"${__fish_io[@]}\"") {
                Some(split) => format!("{{ {split}; {z}=(\"${{__fish_csv[@]}}\"); }}"),
                None => format!("{z}=(\"${{__fish_io[@]}}\")"),
            };
            let Some(words) = target_words else {
                return format!(
                    "{{ {pre}__fish_iv=({}); __fish_set_idx {mode} {} ${{+{z}}} ${{{z}+\"${{{z}[@]}}\"}} && {store}; }}",
                    list.join(" "),
                    Self::index_spec(&i)
                );
            };
            // `set l[$i] …`: the operand is one argument per word it expands to; the first
            // is the target and the others are values. None at all leaves the values as
            // `set`'s operands.
            let flags: Vec<String> = args[..at].iter().map(|w| self.word_scalar(w)).collect();
            return format!(
                "{{ __fish_tw=({words}); if (( ${{#__fish_tw}} )); then {pre}__fish_iv=(\"${{(@)__fish_tw[2,-1]}}\" {}); __fish_set_idx {mode} \"${{${{__fish_tw[1]#*\\[}}%\\]}}\" ${{+{z}}} ${{{z}+\"${{{z}[@]}}\"}} && {store}; else __fish_set_dynamic {} {}; fi; }}",
                list.join(" "),
                flags.join(" "),
                list.join(" ")
            );
        }

        let words: Vec<String> = values.iter().map(|w| self.word_list(w)).collect();
        let mut list = words.join(" ");
        // a path variable's values are split on `:` (set_in_node, env/impl/environment.rs:940);
        // a literal without one and an unindexed path variable need no splitting
        let unsplit = |w: &Word| match w.parts.as_slice() {
            [Part::Var(v)] => v.index.is_none() && !v.deref && path_kind(&v.name, None) == PathKind::Always,
            _ => w.text().is_some_and(|t| !t.contains(':')),
        };
        if path_kind(&name, Self::path_flag(&f)) == PathKind::AtRunTime || !values.iter().all(unsplit) {
            if let Some(split) = self.path_split_cmd(&name, &f, &list) {
                // (as the last word list built, it leaves the status a substitution set)
                let reset = if self.hoists.is_empty() { ": ; " } else { "" };
                self.hoists.push(format!("{reset}{split}"));
                list = "\"${__fish_csv[@]}\"".into();
            }
        }

        if f.unexport {
            return format!("typeset -g +x {z}");
        }
        // `-g` (and `-U`) write the global even where a local of that name is in view,
        // which keeps hiding it (`EnvStackImpl::set`, env/impl/environment.rs:660)
        let global_only = f.global && !f.local;
        if global_only {
            z = Self::global_of(&name);
        }
        // (`--path` / `--unpath` on an exported list keeps it exported, joined anew)
        if f.export || ((f.path || f.unpath) && z.starts_with("__fx_")) {
            return self.set_export(&name, &z, values, &list, (f.local, f.append, f.prepend), Self::path_flag(&f));
        }
        // `-a` / `-p` start from the value `$x` shows, whatever scope it is in, and store
        // the result in the scope asked for (`new_var_values`, builtins/set.rs:852)
        let shown = self.zn(&name);
        let old = format!("${{{shown}+\"${{{shown}[@]}}\"}}");
        let items = match (f.append, f.prepend) {
            (true, _) => format!("{old} {list}"),
            (_, true) => format!("{list} {old}"),
            _ => list.clone(),
        };
        if global_only {
            let decl = if self.fn_depth > 0 { "typeset -ga " } else { "" };
            // (a function's unsettled name for `x` refers to the global from here on)
            return match self.auto_ref(&shown, &name) {
                Some(r) => format!("{{ {r}; {decl}{z}=({items}); {r}; }}"),
                None => format!("{decl}{z}=({items})"),
            };
        }
        // A fish local is invisible to the functions it calls, which zsh's dynamically
        // scoped `local` is not, so it lives under a name no other code refers to.
        if f.local {
            let outer = shown;
            z = if f.function { self.declare_function_local(&name) } else { self.declare_local(&name) };
            let decl = if self.in_local_scope() { "local -a" } else { "typeset -ga" };
            // `set -q x; or set -l x 1`: when the declaration is skipped the later reads of
            // `$x` must still find the variable that is visible, so the local starts as a
            // copy of it
            if self.guarded && z != outer {
                self.pre_chain.push(format!("(( ${{+{outer}}} )) && {decl} {z}=(${{{outer}+\"${{{outer}[@]}}\"}})"));
            }
            return self.declare_list(decl, &z, &items);
        }
        // no scope: the visible local, else the global, else (in a function) a new
        // function local
        if self.local_of(&name).is_none() {
            if let Some((m, settle)) = self.function_auto_local(&name) {
                // (the old value is read through the new name once it is settled)
                let old = format!("${{{m}+\"${{{m}[@]}}\"}}");
                let items = match (f.append, f.prepend) {
                    (true, _) => format!("{old} {list}"),
                    (_, true) => format!("{list} {old}"),
                    _ => list,
                };
                return format!("{{ {settle}; {m}=({items}); }}");
            }
        }
        let assign = if f.append {
            format!("{z}+=({list})")
        } else if f.prepend {
            format!("{z}=({list} \"${{{z}[@]}}\")")
        } else {
            format!("{z}=({list})")
        };
        if self.auto_locals.contains(&z) {
            return format!("{{ {}; {assign}; }}", Self::auto_settle(&z, &name));
        }
        assign
    }

    /// `--path` / `--unpath`, when given.
    fn path_flag(f: &SetFlags) -> Option<bool> {
        if f.path {
            Some(true)
        } else if f.unpath {
            Some(false)
        } else {
            None
        }
    }

    /// The command that leaves the words `words` in `__fish_csv` the way `set` stores them
    /// in `name`: split on `:` when it is a path variable, or `None` when it never is.
    fn path_split_cmd(&self, name: &str, f: &SetFlags, words: &str) -> Option<String> {
        match path_kind(name, Self::path_flag(f)) {
            PathKind::Never => None,
            PathKind::Always => Some(format!("__fish_colon_split {words}")),
            PathKind::AtRunTime => Some(format!("__fish_pvsplit {} {words}", squote(name))),
        }
    }

    /// `NAME[…]` whose index is built from variables (`l[$i]`): fish expands it like any
    /// argument, into one `NAME[…]` word per element (per combination, for several
    /// lists), and `set` takes each word as an operand of its own. The word list, or
    /// `None` for an operand with a literal index.
    fn runtime_target(&mut self, w: &Word) -> Option<String> {
        if w.text().is_some() || !matches!(self.set_target(w), Some((_, Some(_)))) {
            return None;
        }
        Some(self.word_list(w))
    }

    /// `set -e NAME…` (`erase`, builtins/set.rs:760-847): one pass per scope flag, inner
    /// to outer (`-l`, `-f`, `-g`, `-U`; the default lookup with none), each over every
    /// operand in turn. The status is that of the last operand that failed; an index
    /// error or a missing list under an index (status 1) ends the whole command. An
    /// operand like `l[$i]` is erased once per word it expands to, each against the
    /// list the previous one left.
    fn erase_cmd(&mut self, f: &SetFlags, operands: &[Word]) -> String {
        let scope = |local: bool, function: bool, global: bool| SetFlags {
            local,
            function,
            global,
            g_flag: global,
            ..SetFlags::default()
        };
        let mut passes: Vec<SetFlags> = Vec::new();
        if f.function {
            passes.push(scope(true, true, false));
        } else if f.local {
            passes.push(scope(true, false, false));
        }
        if f.g_flag {
            passes.push(scope(false, false, true));
        }
        if f.universal {
            passes.push(SetFlags { universal: true, ..SetFlags::default() });
        }
        if passes.is_empty() {
            passes.push(SetFlags::default());
        }
        let single = operands.len() == 1;
        let mut units: Vec<String> = Vec::new();
        for pass in &passes {
            for w in operands {
                let Some((n, i)) = self.set_target(w) else { continue };
                let Some(words) = self.runtime_target(w) else {
                    let op = self.erase_in(pass, &n, i);
                    units.push(op);
                    continue;
                };
                let op = self.erase_in(pass, &n, Some("${__fish_ix}".into()));
                let none = if single && passes.len() == 1 {
                    "; (( __fish_en )) || { print -u2 -r -- 'set: --erase: option requires an argument'; __fish_erc=2; }"
                } else {
                    ""
                };
                units.push(format!(
                    "__fish_en=0; for __fish_w in {words}; do (( ++__fish_en )); __fish_ix=${{${{__fish_w#*\\[}}%\\]}}; {op} || {{ __fish_st __fish_erc; (( __fish_erc == 1 )) && break; }}; done{none}"
                ));
            }
        }
        if units.len() == 1 && passes.len() == 1 && !passes[0].universal && operands.first().and_then(Word::text).is_some() {
            return units.pop().unwrap_or_default();
        }
        // each operand runs unless one before it ended the command
        let mut text = String::new();
        for (k, u) in units.iter().enumerate().rev() {
            let u = if u.starts_with("__fish_en=0;") { u.clone() } else { format!("{u} || __fish_st __fish_erc") };
            text = if k + 1 == units.len() { u } else { format!("{u}; (( __fish_erc == 1 )) || {{ {text}; }}") };
        }
        format!("{{ __fish_erc=0; {text}; (exit $__fish_erc); }}")
    }

    /// One `set -e` operand in one pass: [`erase_one`](Self::erase_one), or for `-U`
    /// the universal variable. That is a scope of its own in fish, erased only when the
    /// universal store has the name (`EnvStackImpl::remove`, env/impl/environment.rs:727:
    /// status 4 when not, and the global of that name stays), and an index needs the
    /// universal list (status 1 without it, builtins/set.rs:797).
    fn erase_in(&mut self, pass: &SetFlags, name: &str, idx: Option<String>) -> String {
        if !pass.universal {
            return self.erase_one(pass, name, idx);
        }
        let z = Self::global_of(name);
        let q = squote(name);
        match idx {
            None => format!("if __fish_uvar_has {q}; then __fish_uvar_erase {z}; unset {z}; else (exit 4); fi"),
            Some(i) => format!(
                "if __fish_uvar_has {q}; then {} && __fish_uvar_save {z}; else (exit 1); fi",
                Self::erase_index(&z, &i)
            ),
        }
    }

    /// `set -e NAME[…]` on the zsh parameter `z`.
    fn erase_index(z: &str, i: &str) -> String {
        format!(
            "{{ __fish_set_idx erase {} ${{+{z}}} ${{{z}+\"${{{z}[@]}}\"}} && {z}=(\"${{__fish_io[@]}}\"); }}",
            Self::index_spec(i)
        )
    }

    /// The index list of `name[…]` as one word for `__fish_set_idx`: literal text
    /// quoted, an index built from variables left for the shell to expand.
    fn index_spec(i: &str) -> String {
        if i.contains('$') {
            format!("\"{i}\"")
        } else {
            squote(i)
        }
    }

    /// The zsh parameter `set NAME[…] …` writes, and any declaration it needs first.
    /// fish reads the list being changed from the scope it writes (`new_var_values_by_index`,
    /// builtins/set.rs:886), so a new local starts empty.
    fn set_scope_target(&mut self, f: &SetFlags, name: &str) -> (String, String) {
        if f.global && !f.local {
            return (String::new(), Self::global_of(name));
        }
        if f.local {
            let before = self.local_of(name);
            let z = if f.function { self.declare_function_local(name) } else { self.declare_local(name) };
            if before.as_deref() == Some(z.as_str()) || z == zname(name) {
                return (String::new(), z);
            }
            let decl = if self.in_local_scope() { "local -a" } else { "typeset -ga" };
            return (format!("{decl} {z}; unset {z}; "), z);
        }
        if let Some(z) = self.local_of(name) {
            if self.auto_locals.contains(&z) {
                return (format!("{}; ", Self::auto_settle(&z, name)), z);
            }
            return (String::new(), z);
        }
        match self.function_auto_local(name) {
            Some((m, settle)) => (format!("{settle}; "), m),
            None => (String::new(), Self::global_of(name)),
        }
    }

    /// `set -e NAME` / `set -e NAME[…]` with the scope flags given. fish erases from
    /// the innermost local that has the name, else the global (`EnvStackImpl::remove`,
    /// env/impl/environment.rs:708); `-l` looks only at locals, `-g` only at the global.
    /// A missing variable is status 4, a missing list with indexes status 1
    /// (builtins/set.rs:783-799).
    fn erase_one(&mut self, f: &SetFlags, name: &str, idx: Option<String>) -> String {
        // (a function's unsettled name for `x` is the global once there is one)
        let pre = if f.global && !f.local { None } else { self.local_of(name).and_then(|l| self.auto_ref(&l, name)) };
        let text = self.erase_scoped(f, name, idx);
        match pre {
            Some(r) => format!("{{ {r}; {text}; }}"),
            None => text,
        }
    }

    fn erase_scoped(&mut self, f: &SetFlags, name: &str, idx: Option<String>) -> String {
        let local = self.local_of(name);
        let target = if f.global && !f.local {
            Some(Self::global_of(name))
        } else if f.local {
            local.clone()
        } else {
            Some(self.zn(name))
        };
        let Some(z) = target else {
            return if idx.is_some() { "(exit 1)" } else { "(exit 4)" }.into();
        };
        let Some(i) = idx else {
            if local.as_deref() == Some(z.as_str()) && !self.unmap_local(name) {
                // the erase runs in a nested block or behind `and`/`or`, so whether `$x`
                // is the next scope out afterwards is known only at run time: the local
                // becomes a reference to it, and a second erase goes through to it
                let outer = self.outer_of(name);
                let decl = if self.in_local_scope() { "local -n" } else { "typeset -gn" };
                self.ref_locals.insert(z.clone());
                return format!("if (( ${{+{z}}} )); then unset {z}; {decl} {z}={outer}; else (exit 4); fi");
            }
            return format!("if (( ${{+{z}}} )); then unset {z}; else (exit 4); fi");
        };
        Self::erase_index(&z, &i)
    }

    /// `local -a z=(items)` for a local that may be a reference left by an erase: the
    /// value is read first (it may come through the reference), then the reference is
    /// dropped.
    fn declare_list(&self, decl: &str, z: &str, items: &str) -> String {
        if !self.ref_locals.contains(z) {
            return format!("{decl} {z}=({items})");
        }
        format!("{{ __fish_rv=({items}); unset -n {z} 2>/dev/null; {decl} {z}=(\"${{__fish_rv[@]}}\"); }}")
    }

    /// `set -x NAME value…`: zsh exports scalars, so the list is joined the way fish
    /// writes it into the environment.
    fn set_export(&mut self, name: &str, z: &str, values: &[Word], list: &str, scope: (bool, bool, bool), path: Option<bool>) -> String {
        let (local, append, prepend) = scope;
        let decl = if local && self.in_local_scope() { "local -x" } else { "typeset -gx" };
        // a path variable is exported joined with `:` (`EnvVar::delimiter`, env/var.rs:235)
        let sep = match path_kind(name, path) {
            PathKind::Always => "':'".to_string(),
            PathKind::Never => "' '".to_string(),
            PathKind::AtRunTime => format!("\"${{__fish_pathvars[{name}]-{}}}\"", if is_auto_pathvar(name) { ":" } else { " " }),
        };
        // (`--path` / `--unpath` on an exported list ties it again with the new separator)
        let tie_when = if path.is_some() { "true".to_string() } else { format!("(( ! ${{+__fx_{name}}} ))") };
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
                format!("local -xT {name} {tied} {sep}; ")
            };
            self.map_local(name, &tied);
            return format!("{{ _fish_x=({items}); {decl}{tied}=(\"${{_fish_x[@]}}\"); }}");
        }
        super::emit::note_exported(name);
        format!(
            "{{ _fish_x=({items}); if {tie_when}; then unset {name} 2>/dev/null; typeset -gxT {name} {tied} {sep}; fi; {tied}=(\"${{_fish_x[@]}}\"); }}"
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
