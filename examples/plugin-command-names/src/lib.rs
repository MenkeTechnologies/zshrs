//! Native (Rust ABI) override of zpwr's customized `_command_names`.
//!
//! zpwr replaces the stock `_command_names` with a version that
//! **unconditionally** shadows the command-name hash
//! (`local -a +h path; local -A +h commands; path=($_saved_path)`), so a
//! bare command completion (`l<TAB>`) offers only builtins / reserved words
//! / functions / aliases — never the raw external-command hash. zshrs's
//! built-in `_command_names` port mirrors *stock* zsh (the shadow is gated
//! behind the `command-path` style), so it diverges from zpwr's output.
//!
//! This plugin is the faithful port of zpwr's body. Registered via ABI v4
//! `register_compfn`, it supersedes the built-in port so `l<TAB>` is
//! byte-identical to the user's real (zpwr) zsh. Load after `compinit`:
//! `zmodload -R <path>/libzpwr_compsys.dylib`.
//!
//! Source (deparsed from the user's `.zcompdump`):
//! ```text
//! local args defs ffilt aliasesAry galiasesAry k v
//! local -a cmdpath _saved_path
//! zstyle -t ":completion:${curcontext}:commands" rehash && rehash
//! zstyle -t ":completion:${curcontext}:functions" prefix-needed \
//!   && [[ $PREFIX != [_.]* ]] && ffilt='[(I)[^_.]*]'
//! defs=('commands:external command:_path_commands')
//! [[ -n "$path[(r).]" || $PREFIX = */* ]] \
//!   && defs+=('executables:executable file:_files -g \*\(-\*\)')
//! if [[ "$1" = -e ]]; then shift
//! else
//!   [[ "$1" = - ]] && shift
//!   defs+=('global-aliases:global alias:__zpwr_galiases' \
//!          'aliases:alias:__zpwr_aliases' \
//!          "functions:shell function:compadd -k 'functions$ffilt'" \
//!          'builtins:builtin command:compadd -Qk builtins' \
//!          'suffix-aliases:suffix alias:_suffix_alias_files' \
//!          'reserved-words:reserved word:compadd -Qk reswords' \
//!          'jobs:: _jobs -t' 'parameters:: _parameters' 'files:files:_files')
//! fi
//! args=("$@")
//! _saved_path=($path); local -a +h path; local -A +h commands; path=($_saved_path)
//! if zstyle -a ":completion:${curcontext}" command-path cmdpath && (( $#cmdpath ))
//! then path=($cmdpath); fi
//! _alternative -O args "$defs[@]"
//! ```

use std::os::raw::c_int;
use znative::{declare_plugin, Args, Host};

/// Single-quote one word for safe reinjection into a shell script,
/// escaping embedded single quotes the POSIX way (`'\''`).
fn sq(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}

/// Port of zpwr's `_command_names`.
fn command_names(h: &Host, a: &Args) -> c_int {
    // `"$@"` — the completion-function arguments (argv[1..]).
    let mut argv: Vec<String> = a.rest().to_vec();

    // zstyle -t ":completion:${curcontext}:commands" rehash && rehash
    let _ = h.eval("zstyle -t \":completion:${curcontext}:commands\" rehash && rehash");

    // ffilt: prefix-needed style on functions, and $PREFIX not [_.]*.
    let prefix = h.getvar("PREFIX").unwrap_or_default();
    let mut ffilt = String::new();
    let prefix_needed =
        h.eval("zstyle -t \":completion:${curcontext}:functions\" prefix-needed") == 0;
    if prefix_needed && !(prefix.starts_with('_') || prefix.starts_with('.')) {
        ffilt = "[(I)[^_.]*]".to_string();
    }

    // defs=('commands:external command:_path_commands')
    let mut defs: Vec<String> = vec!["commands:external command:_path_commands".to_string()];

    // [[ -n "$path[(r).]" || $PREFIX = */* ]] → dot in $path OR slash in PREFIX
    let path_has_dot = h
        .getvar("PATH")
        .unwrap_or_default()
        .split(':')
        .any(|d| d == ".");
    if path_has_dot || prefix.contains('/') {
        defs.push("executables:executable file:_files -g \\*\\(-\\*\\)".to_string());
    }

    // if [[ "$1" = -e ]]; then shift; else [[ "$1" = - ]] && shift; defs+=(…)
    if argv.first().map(|s| s == "-e").unwrap_or(false) {
        argv.remove(0);
    } else {
        if argv.first().map(|s| s == "-").unwrap_or(false) {
            argv.remove(0);
        }
        defs.push("global-aliases:global alias:__zpwr_galiases".to_string());
        defs.push("aliases:alias:__zpwr_aliases".to_string());
        defs.push(format!("functions:shell function:compadd -k 'functions{ffilt}'"));
        defs.push("builtins:builtin command:compadd -Qk builtins".to_string());
        defs.push("suffix-aliases:suffix alias:_suffix_alias_files".to_string());
        defs.push("reserved-words:reserved word:compadd -Qk reswords".to_string());
        defs.push("jobs:: _jobs -t".to_string());
        defs.push("parameters:: _parameters".to_string());
        defs.push("files:files:_files".to_string());
    }

    // The remaining statements MUST run in one dynamic scope so the
    // `local -A +h commands` shadow is live while `_alternative` (and the
    // `_path_commands` it calls) run — a scoped EMPTY `commands` param is
    // what makes `compadd -k commands` add nothing (NOT emptying the global
    // command hash, which would just refill). Emit them as a single script:
    //
    //   args=("$@")
    //   local -a +h path
    //   local -A +h commands
    //   _alternative -O args "$defs[@]"
    //
    // (`_saved_path`/`path=($_saved_path)`/`command-path` reassignment leave
    // $path unchanged with the styles unset, so they are elided.)
    let mut script = String::from("local -a args=(");
    for a in &argv {
        script.push(' ');
        script.push_str(&sq(a));
    }
    script.push_str(" )\nlocal -a +h path\nlocal -A +h commands\n_alternative -O args");
    for d in &defs {
        script.push(' ');
        script.push_str(&sq(d));
    }
    h.eval(&script)
}

declare_plugin! {
    name: "zpwr-compsys",
    version: "0.1.0",
    compfns: { "_command_names" => command_names },
}
