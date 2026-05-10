//! ShellExecutor method shims previously parked inside `src/ported/`.
//!
//! Per the no-ShellExecutor-in-src/ported rule, every
//! `impl crate::ported::exec::ShellExecutor { ... }` block that used
//! to live next to its companion port now lives here. The contents
//! are verbatim moves — same method bodies, same `pub(crate)` /
//! `pub` visibility — only the home file changed. Each block is
//! tagged with the source file it came from for auditability.
//!
//! New shim methods (e.g. argv → ops parsers for module builtins)
//! also belong here so `src/ported/` stays C-faithful.

#![allow(unused_imports, dead_code, unused_variables, unused_mut, non_snake_case)]

// Standard-library re-imports needed by the moved-in blocks — many
// of these used `HashMap` / `env` / `Path` / etc. via the imports at
// the top of their original `.rs` file.
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

// Crate-internal items the moved blocks reach for. Pulling them in
// at the top of this file is a one-time cost so each block doesn't
// need to be rewritten with fully-qualified paths.
use crate::ported::utils::{zwarn, zwarnnam, zerr, zerrnam};
use crate::ported::params::*;
use crate::ported::options::*;
use crate::ported::hist::*;
use crate::ported::pattern::*;
use crate::ported::prompt::*;
use crate::ported::subst::*;
use crate::ported::math::*;
use crate::ported::jobs::*;
use crate::ported::glob::*;
use crate::ported::module::*;
use crate::ported::signals::*;
// NOTE: do NOT `use crate::ported::modules::*;` — it shadows the
// `regex` crate (since modules has a `regex` submodule), breaking
// `regex::Regex::new(...)` call sites in the moved blocks.
use crate::ported::modules::cap::*;
use crate::ported::modules::tcp::bin_ztcp;
use crate::ported::modules::termcap::bin_echotc;
use crate::ported::modules::terminfo::*;
use crate::ported::zsh_h::{options, MAX_OPS};
use crate::options::ZSH_OPTIONS_SET;
use crate::exec::ShellExecutor;
use crate::fusevm_bridge::with_executor;
use crate::ported::utils::quotedzputs;
use crate::ported::text::FuncBodyFmt;
use ::regex::{Regex, RegexBuilder, Error as RegexError};

// =====================================================================
// MOVED FROM: src/ported/pattern.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Check if pattern contains extended glob syntax
    pub(crate) fn has_extglob_pattern(&self, pattern: &str) -> bool {
        let chars: Vec<char> = pattern.chars().collect();
        for i in 0..chars.len().saturating_sub(1) {
            if (chars[i] == '?'
                || chars[i] == '*'
                || chars[i] == '+'
                || chars[i] == '@'
                || chars[i] == '!')
                && chars[i + 1] == '('
            {
                return true;
            }
        }
        false
    }
    /// Convert extended glob pattern to regex
    pub(crate) fn extglob_to_regex(&self, pattern: &str) -> String {
        let mut regex = String::from("^");
        let chars: Vec<char> = pattern.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let c = chars[i];

            // Check for extglob patterns
            if i + 1 < chars.len() && chars[i + 1] == '(' {
                match c {
                    '?' => {
                        // ?(pattern) - zero or one occurrence
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})?", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '*' => {
                        // *(pattern) - zero or more occurrences
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})*", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '+' => {
                        // +(pattern) - one or more occurrences
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})+", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '@' => {
                        // @(pattern) - exactly one occurrence
                        let (inner, end) = self.extract_extglob_inner(&chars, i + 2);
                        let inner_regex = self.extglob_inner_to_regex(&inner);
                        regex.push_str(&format!("({})", inner_regex));
                        i = end + 1;
                        continue;
                    }
                    '!' => {
                        // !(pattern) - handled specially in expand_extglob
                        // Just skip this extglob for regex, will do manual filtering
                        let (_, end) = self.extract_extglob_inner(&chars, i + 2);
                        regex.push_str(".*"); // Match anything, we filter later
                        i = end + 1;
                        continue;
                    }
                    _ => {}
                }
            }

            // Handle regular glob characters
            match c {
                '*' => regex.push_str(".*"),
                '?' => regex.push('.'),
                '.' => regex.push_str("\\."),
                '[' => {
                    regex.push('[');
                    i += 1;
                    while i < chars.len() && chars[i] != ']' {
                        if chars[i] == '!' && regex.ends_with('[') {
                            regex.push('^');
                        } else {
                            regex.push(chars[i]);
                        }
                        i += 1;
                    }
                    regex.push(']');
                }
                '^' | '$' | '(' | ')' | '{' | '}' | '|' | '\\' => {
                    regex.push('\\');
                    regex.push(c);
                }
                _ => regex.push(c),
            }
            i += 1;
        }

        regex.push('$');
        regex
    }
    /// Extract the inner part of an extglob pattern (until closing paren)
    pub(crate) fn extract_extglob_inner(&self, chars: &[char], start: usize) -> (String, usize) {
        let mut inner = String::new();
        let mut depth = 1;
        let mut i = start;

        while i < chars.len() && depth > 0 {
            if chars[i] == '(' {
                depth += 1;
            } else if chars[i] == ')' {
                depth -= 1;
                if depth == 0 {
                    return (inner, i);
                }
            }
            inner.push(chars[i]);
            i += 1;
        }

        (inner, i)
    }
    /// Convert the inner part of extglob (handles | for alternation)
    pub(crate) fn extglob_inner_to_regex(&self, inner: &str) -> String {
        // Split by | and convert each alternative
        let alternatives: Vec<String> = inner
            .split('|')
            .map(|alt| {
                let mut result = String::new();
                for c in alt.chars() {
                    match c {
                        '*' => result.push_str(".*"),
                        '?' => result.push('.'),
                        '.' => result.push_str("\\."),
                        '^' | '$' | '(' | ')' | '{' | '}' | '\\' => {
                            result.push('\\');
                            result.push(c);
                        }
                        _ => result.push(c),
                    }
                }
                result
            })
            .collect();

        alternatives.join("|")
    }
    /// Extract !(pattern) info from file pattern, returns (inner_pattern, suffix)
    pub(crate) fn extract_neg_extglob(&self, pattern: &str) -> Option<(String, String)> {
        let chars: Vec<char> = pattern.chars().collect();
        if chars.len() >= 3 && chars[0] == '!' && chars[1] == '(' {
            let mut depth = 1;
            let mut i = 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '(' {
                    depth += 1;
                } else if chars[i] == ')' {
                    depth -= 1;
                }
                i += 1;
            }
            if depth == 0 {
                let inner: String = chars[2..i - 1].iter().collect();
                let suffix: String = chars[i..].iter().collect();
                return Some((inner, suffix));
            }
        }
        None
    }
}

// =====================================================================
// MOVED FROM: src/ported/options.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// zsh-compatible setopt builtin
    pub(crate) fn bin_setopt(&mut self, name: &str, args: &[String]) -> i32 {
        // C parity: zsh/Src/options.c bin_setopt handles both setopt
        // (BIN_SETOPT) and unsetopt (BIN_UNSETOPT) — same handler, two
        // BUILTIN() table entries (builtin.c:114, 130). The `func` arg
        // (here, the invoked name) flips the enable polarity for bare
        // names, -o, +o, and the pattern match branch.
        let is_unsetopt = name == "unsetopt";
        // PFA-SMR aspect: emit one `setopt`/`unsetopt` event per option
        // name. zsh accepts `-o NAME` / bare `NAME` interchangeably.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() && !args.is_empty() {
            let ctx = self.recorder_ctx();
            let mut iter = args.iter().peekable();
            while let Some(a) = iter.next() {
                match a.as_str() {
                    "-o" | "+o" => {
                        if let Some(opt) = iter.next() {
                            if is_unsetopt {
                                crate::recorder::emit_unsetopt(opt, ctx.clone());
                            } else {
                                crate::recorder::emit_setopt(opt, ctx.clone());
                            }
                        }
                    }
                    s if s.starts_with('-') || s.starts_with('+') => {
                        // single-letter -K / +K flags toggle named options
                        // by short name; skip in this proof — Phase 2.5
                        // material.
                    }
                    _ => {
                        if is_unsetopt {
                            crate::recorder::emit_unsetopt(a, ctx.clone());
                        } else {
                            crate::recorder::emit_setopt(a, ctx.clone());
                        }
                    }
                }
            }
        }
        if args.is_empty() {
            if is_unsetopt {
                // unsetopt with no args: list all options in the form
                // you'd pass to unsetopt to disable them. Default-ON ->
                // "noOPTION"; default-OFF -> "OPTION".
                let defaults_on = Self::default_on_options();
                let mut all_opts: Vec<String> = Vec::new();
                for &opt in Self::all_zsh_options() {
                    if defaults_on.contains(&opt) {
                        all_opts.push(format!("no{}", opt));
                    } else {
                        all_opts.push(opt.to_string());
                    }
                }
                all_opts.sort();
                for opt in all_opts {
                    println!("{}", opt);
                }
                return 0;
            }
            // List options that differ from compiled-in defaults (zsh behavior)
            // For default-ON options: show "noOPTION" if currently OFF
            // For default-OFF options: show "OPTION" if currently ON
            let defaults_on = Self::default_on_options();
            let mut diff_opts: Vec<String> = Vec::new();

            for &opt in Self::all_zsh_options() {
                let enabled = self.options.get(opt).copied().unwrap_or(false);
                let is_default_on = defaults_on.contains(&opt);

                if is_default_on && !enabled {
                    // Default ON but currently OFF -> show noOPTION
                    diff_opts.push(format!("no{}", opt));
                } else if !is_default_on && enabled {
                    // Default OFF but currently ON -> show OPTION
                    diff_opts.push(opt.to_string());
                }
            }
            diff_opts.sort();
            for opt in diff_opts {
                println!("{}", opt);
            }
            return 0;
        }

        // `setopt -p` / `setopt -L` — print currently-set options in
        // a form that can be sourced to restore the state. Bash uses -p,
        // zsh accepts both. Output: `setopt OPTION` per line for each
        // currently-set non-default option.
        if args.iter().any(|a| a == "-p" || a == "-L") {
            let defaults_on = Self::default_on_options();
            let mut diff_opts: Vec<String> = Vec::new();
            for &opt in Self::all_zsh_options() {
                let enabled = self.options.get(opt).copied().unwrap_or(false);
                let is_default_on = defaults_on.contains(&opt);
                if is_default_on && !enabled {
                    diff_opts.push(format!("setopt no{}", opt));
                } else if !is_default_on && enabled {
                    diff_opts.push(format!("setopt {}", opt));
                }
            }
            diff_opts.sort();
            for line in diff_opts {
                println!("{}", line);
            }
            return 0;
        }

        let mut use_pattern = false;
        let mut iter = args.iter().peekable();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-m" => use_pattern = true,
                "-o" => {
                    // -o option_name: set option (unsetopt: unset)
                    if let Some(opt) = iter.next() {
                        let (oname, enable) = Self::normalize_option_name(opt);
                        let v = if is_unsetopt { !enable } else { enable };
                        self.options.insert(oname, v);
                    }
                }
                "+o" => {
                    // +o option_name: unset option (unsetopt: set)
                    if let Some(opt) = iter.next() {
                        let (oname, enable) = Self::normalize_option_name(opt);
                        let v = if is_unsetopt { enable } else { !enable };
                        self.options.insert(oname, v);
                    }
                }
                _ => {
                    if use_pattern {
                        // Match pattern against all options
                        for opt in Self::all_zsh_options() {
                            if Self::option_matches_pattern(opt, arg) {
                                self.options.insert(opt.to_string(), !is_unsetopt);
                            }
                        }
                    } else {
                        // zsh: single-letter `-X` / `+X` flags on
                        // setopt are shortcuts for option names from
                        // the option-letter table (mirrors `set`).
                        // `setopt -h` is a no-op accepted silently
                        // (the `h` shortcut maps to `hashcmds`).
                        // zshrs's old default arm rejected ANY `-`
                        // prefixed arg as an unknown name.
                        if arg.len() == 2 && (arg.starts_with('-') || arg.starts_with('+')) {
                            // Single-letter form — accept silently
                            // (already covered for the few we wire
                            // up; the rest are no-ops in `-c` mode).
                            continue;
                        }
                        let (oname, enable) = Self::normalize_option_name(arg);
                        // zsh: `setopt nosuchoption` errors with
                        //   `setopt:1: no such option: nosuchoption`
                        // Reject unknown names against the canonical
                        // ZSH_OPTIONS_SET so user scripts get the same
                        // diagnostic. Strip leading `no` first because
                        // `nounset` ↔ `unset` style names are toggles.
                        if !ZSH_OPTIONS_SET.contains(oname.as_str()) {
                            zwarnnam(name, &format!("no such option: {}", arg));
                            return 1;
                        }
                        let v = if is_unsetopt { !enable } else { enable };
                        self.options.insert(oname, v);
                    }
                }
            }
        }
        0
    }
}

// =====================================================================
// MOVED FROM: src/ported/options.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    pub(crate) fn all_zsh_options() -> &'static [&'static str] {
        &[
            "aliases",
            "aliasfuncdef",
            "allexport",
            "alwayslastprompt",
            "alwaystoend",
            "appendcreate",
            "appendhistory",
            "autocd",
            "autocontinue",
            "autolist",
            "automenu",
            "autonamedirs",
            "autoparamkeys",
            "autoparamslash",
            "autopushd",
            "autoremoveslash",
            "autoresume",
            "badpattern",
            "banghist",
            "bareglobqual",
            "bashautolist",
            "bashrematch",
            "beep",
            "bgnice",
            "braceccl",
            "braceexpand",
            "bsdecho",
            "caseglob",
            "casematch",
            "casepaths",
            "cbases",
            "cdablevars",
            "cdsilent",
            "chasedots",
            "chaselinks",
            "checkjobs",
            "checkrunningjobs",
            "clobber",
            "clobberempty",
            "combiningchars",
            "completealiases",
            "completeinword",
            "continueonerror",
            "correct",
            "correctall",
            "cprecedences",
            "cshjunkiehistory",
            "cshjunkieloops",
            "cshjunkiequotes",
            "cshnullcmd",
            "cshnullglob",
            "debugbeforecmd",
            "dotglob",
            "dvorak",
            "emacs",
            "equals",
            "errexit",
            "errreturn",
            "evallineno",
            "exec",
            "extendedglob",
            "extendedhistory",
            "flowcontrol",
            "forcefloat",
            "functionargzero",
            "glob",
            "globassign",
            "globcomplete",
            "globdots",
            "globstarshort",
            "globsubst",
            "globalexport",
            "globalrcs",
            "hashall",
            "hashcmds",
            "hashdirs",
            "hashexecutablesonly",
            "hashlistall",
            "histallowclobber",
            "histappend",
            "histbeep",
            "histexpand",
            "histexpiredupsfirst",
            "histfcntllock",
            "histfindnodups",
            "histignorealldups",
            "histignoredups",
            "histignorespace",
            "histlexwords",
            "histnofunctions",
            "histnostore",
            "histreduceblanks",
            "histsavebycopy",
            "histsavenodups",
            "histsubstpattern",
            "histverify",
            "hup",
            "ignorebraces",
            "ignoreclosebraces",
            "ignoreeof",
            "incappendhistory",
            "incappendhistorytime",
            "interactive",
            "interactivecomments",
            "ksharrays",
            "kshautoload",
            "kshglob",
            "kshoptionprint",
            "kshtypeset",
            "kshzerosubscript",
            "listambiguous",
            "listbeep",
            "listpacked",
            "listrowsfirst",
            "listtypes",
            "localloops",
            "localoptions",
            "localpatterns",
            "localtraps",
            "log",
            "login",
            "longlistjobs",
            "magicequalsubst",
            "mailwarn",
            "mailwarning",
            "markdirs",
            "menucomplete",
            "monitor",
            "multibyte",
            "multifuncdef",
            "multios",
            "nomatch",
            "notify",
            "nullglob",
            "numericglobsort",
            "octalzeroes",
            "onecmd",
            "overstrike",
            "pathdirs",
            "pathscript",
            "physical",
            "pipefail",
            "posixaliases",
            "posixargzero",
            "posixbuiltins",
            "posixcd",
            "posixidentifiers",
            "posixjobs",
            "posixstrings",
            "posixtraps",
            "printeightbit",
            "printexitvalue",
            "privileged",
            "promptbang",
            "promptcr",
            "promptpercent",
            "promptsp",
            "promptsubst",
            "promptvars",
            "pushdignoredups",
            "pushdminus",
            "pushdsilent",
            "pushdtohome",
            "rcexpandparam",
            "rcquotes",
            "rcs",
            "recexact",
            "rematchpcre",
            "restricted",
            "rmstarsilent",
            "rmstarwait",
            "sharehistory",
            "shfileexpansion",
            "shglob",
            "shinstdin",
            "shnullcmd",
            "shoptionletters",
            "shortloops",
            "shortrepeat",
            "shwordsplit",
            "singlecommand",
            "singlelinezle",
            "sourcetrace",
            "stdin",
            "sunkeyboardhack",
            "trackall",
            "transientrprompt",
            "trapsasync",
            "typesetsilent",
            "typesettounset",
            "unset",
            "verbose",
            "vi",
            "warncreateglobal",
            "warnnestedvar",
            "xtrace",
            "zle",
        ]
    }
    pub(crate) fn default_options() -> HashMap<String, bool> {
        let mut opts = HashMap::new();
        // Initialize all options to false first
        for opt in Self::all_zsh_options() {
            opts.insert(opt.to_string(), false);
        }
        // Set zsh defaults (options marked with <D> or <Z> in zshoptions man page)
        let defaults_on = [
            "aliases",
            "alwayslastprompt",
            "appendhistory",
            "autolist",
            "automenu",
            "autoparamkeys",
            "autoparamslash",
            "autoremoveslash",
            "badpattern",
            "banghist",
            "bareglobqual",
            "beep",
            "bgnice",
            "caseglob",
            "casematch",
            "checkjobs",
            "checkrunningjobs",
            "clobber",
            "debugbeforecmd",
            "equals",
            "evallineno",
            "exec",
            "flowcontrol",
            "functionargzero",
            "glob",
            "globalexport",
            "globalrcs",
            "hashcmds",
            "hashdirs",
            "hashlistall",
            "histbeep",
            "histsavebycopy",
            "hup",
            "interactive",
            "listambiguous",
            "listbeep",
            "listtypes",
            "monitor",
            "multibyte",
            "multifuncdef",
            "multios",
            "nomatch",
            "notify",
            "promptcr",
            "promptpercent",
            "promptsp",
            "rcs",
            "shinstdin",
            "shortloops",
            "unset",
            "zle",
        ];
        for opt in defaults_on {
            opts.insert(opt.to_string(), true);
        }
        opts
    }
    /// Normalize option name: lowercase, remove underscores/hyphens, handle "no" prefix
    pub(crate) fn normalize_option_name(name: &str) -> (String, bool) {
        let normalized = name.to_lowercase().replace(['-', '_'], "");
        if let Some(stripped) = normalized.strip_prefix("no") {
            // O(1) lookup in HashSet instead of linear scan
            if ZSH_OPTIONS_SET.contains(stripped) {
                return (stripped.to_string(), false);
            }
        }
        (normalized, true)
    }
    /// Check if option name matches a pattern for setopt -m. zsh
    /// normalizes both pattern and option name by lowercasing and
    /// stripping `-` / `_` (so `NO_GLOB`, `noGlob`, `no-glob` all
    /// map to the same key), then runs the pattern through the
    /// glob matcher. Direct port of options.c match_option pattern
    /// path with the same case-insensitive normalization.
    pub(crate) fn option_matches_pattern(opt: &str, pattern: &str) -> bool {
        let pat = pattern.to_lowercase().replace(['-', '_'], "");
        let opt_lower = opt.to_lowercase().replace(['-', '_'], "");
        // Use the canonical glob matcher so character classes,
        // extendedglob, etc. behave the same as everywhere else.
        Self::glob_match_static(&opt_lower, &pat)
    }
    pub(crate) fn default_on_options() -> &'static [&'static str] {
        &[
            "aliases",
            "alwayslastprompt",
            "appendhistory",
            "autolist",
            "automenu",
            "autoparamkeys",
            "autoparamslash",
            "autoremoveslash",
            "badpattern",
            "banghist",
            "bareglobqual",
            "beep",
            "bgnice",
            "caseglob",
            "casematch",
            "checkjobs",
            "checkrunningjobs",
            "clobber",
            "debugbeforecmd",
            "equals",
            "evallineno",
            "exec",
            "flowcontrol",
            "functionargzero",
            "glob",
            "globalexport",
            "globalrcs",
            "hashcmds",
            "hashdirs",
            "hashlistall",
            "histbeep",
            "histsavebycopy",
            "hup",
            "interactive",
            "listambiguous",
            "listbeep",
            "listtypes",
            "monitor",
            "multibyte",
            "multifuncdef",
            "multios",
            "nomatch",
            "notify",
            "promptcr",
            "promptpercent",
            "promptsp",
            "rcs",
            "shinstdin",
            "shortloops",
            "unset",
            "zle",
        ]
    }
    pub(crate) fn print_options_table(&self) {
        let mut opts: Vec<_> = Self::all_zsh_options().to_vec();
        opts.sort();
        let defaults_on = Self::default_on_options();
        for &opt in &opts {
            let enabled = self.options.get(opt).copied().unwrap_or(false);
            let is_default_on = defaults_on.contains(&opt);
            // zsh format: for default-ON options, show "noOPTION off" when on, "noOPTION on" when off
            // for default-OFF options, show "OPTION off" when off, "OPTION on" when on
            let (display_name, display_state) = if is_default_on {
                (format!("no{}", opt), if enabled { "off" } else { "on" })
            } else {
                (opt.to_string(), if enabled { "on" } else { "off" })
            };
            println!("{:<22}{}", display_name, display_state);
        }
    }
    pub(crate) fn print_options_reentrant(&self) {
        let mut opts: Vec<_> = Self::all_zsh_options().to_vec();
        opts.sort();
        let defaults_on = Self::default_on_options();
        for &opt in &opts {
            let enabled = self.options.get(opt).copied().unwrap_or(false);
            let is_default_on = defaults_on.contains(&opt);
            // zsh format: use noOPTION for default-on options
            let (display_name, use_minus) = if is_default_on {
                (format!("no{}", opt), !enabled)
            } else {
                (opt.to_string(), enabled)
            };
            if use_minus {
                println!("set -o {}", display_name);
            } else {
                println!("set +o {}", display_name);
            }
        }
    }
    /// Get options to set/unset for an emulation mode
    pub(crate) fn emulate_mode_options(mode: &str, reset: bool) -> (Vec<&'static str>, Vec<&'static str>) {
        match mode {
            "zsh" => {
                if reset {
                    // Full reset: return to zsh defaults
                    (
                        vec![
                            "aliases",
                            "alwayslastprompt",
                            "autolist",
                            "automenu",
                            "autoparamslash",
                            "autoremoveslash",
                            "banghist",
                            "bareglobqual",
                            "completeinword",
                            "extendedhistory",
                            "functionargzero",
                            "glob",
                            "hashcmds",
                            "hashdirs",
                            "histexpand",
                            "histignoredups",
                            "interactivecomments",
                            "listambiguous",
                            "listtypes",
                            "multios",
                            "nomatch",
                            "notify",
                            "promptpercent",
                            "promptsubst",
                        ],
                        vec![
                            "ksharrays",
                            "kshglob",
                            "shwordsplit",
                            "shglob",
                            "posixbuiltins",
                            "posixidentifiers",
                            "posixstrings",
                            "bsdecho",
                            "ignorebraces",
                        ],
                    )
                } else {
                    // Minimal changes for portability
                    (vec!["functionargzero"], vec!["ksharrays", "shwordsplit"])
                }
            }
            "sh" => {
                let set = vec![
                    "ksharrays",
                    "shwordsplit",
                    "posixbuiltins",
                    "shglob",
                    "shfileexpansion",
                    "globsubst",
                    "interactivecomments",
                    "rmstarsilent",
                    "bsdecho",
                    "ignorebraces",
                ];
                let unset = vec![
                    "badpattern",
                    "banghist",
                    "bgnice",
                    "equals",
                    "functionargzero",
                    "globalexport",
                    "multios",
                    "nomatch",
                    "notify",
                    "promptpercent",
                ];
                (set, unset)
            }
            "ksh" => {
                let set = vec![
                    "ksharrays",
                    "kshglob",
                    "shwordsplit",
                    "posixbuiltins",
                    "kshoptionprint",
                    "localoptions",
                    "promptbang",
                    "promptsubst",
                    "singlelinezle",
                    "interactivecomments",
                ];
                let unset = vec![
                    "badpattern",
                    "banghist",
                    "bgnice",
                    "equals",
                    "functionargzero",
                    "globalexport",
                    "multios",
                    "nomatch",
                    "notify",
                    "promptpercent",
                ];
                (set, unset)
            }
            "csh" => {
                // C shell emulation (limited)
                (vec!["cshnullglob", "cshjunkiequotes"], vec!["nomatch"])
            }
            "bash" => {
                let set = vec![
                    "ksharrays",
                    "shwordsplit",
                    "interactivecomments",
                    "shfileexpansion",
                    "globsubst",
                ];
                let unset = vec![
                    "badpattern",
                    "banghist",
                    "functionargzero",
                    "multios",
                    "nomatch",
                    "notify",
                    "promptpercent",
                ];
                (set, unset)
            }
            _ => (vec![], vec![]),
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/options.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Enter POSIX strict mode — drop all SQLite caches, shrink worker pool to minimum.
    /// No zsh extensions, no caching, no threads beyond the bare minimum. Dinosaur mode.
    pub fn enter_posix_mode(&mut self) {
        self.posix_mode = true;
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        // Worker pool stays at size 1 — we can't drop it entirely because
        // some code paths use it unconditionally, but with 1 thread it's
        // effectively serial.
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        tracing::info!("POSIX strict mode: SQLite caches dropped, worker pool shrunk to 1");
    }

    /// Enter ksh emulation mode — applies the same option presets that
    /// `emulate ksh` would (Src/options.c emulate_mode_options "ksh"):
    /// `ksharrays`, `kshglob`, `shwordsplit`, `posixbuiltins`,
    /// `kshoptionprint`, `localoptions`, `promptbang`, `promptsubst`,
    /// `singlelinezle`, `interactivecomments`; unsets `badpattern`,
    /// `banghist`, `bgnice`, `equals`, `functionargzero`,
    /// `globalexport`, `multios`, `nomatch`, `notify`, `promptpercent`.
    /// Also drops SQLite caches and shrinks worker pool — drop-in mode
    /// must not behave differently than /bin/ksh from observable I/O.
    pub fn enter_ksh_mode(&mut self) {
        let (set, unset) = Self::emulate_mode_options("ksh", false);
        for opt in set {
            self.options.insert(opt.to_string(), true);
        }
        for opt in unset {
            self.options.insert(opt.to_string(), false);
        }
        self.options.insert("kshemulation".to_string(), true);
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        tracing::info!("ksh emulation mode: option presets applied, caches dropped");
    }
}

// =====================================================================
// MOVED FROM: src/ported/params.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Parse subscript range like "1" or "1,5" or "-1" or "1,-1"
    pub(crate) fn parse_subscript_range(&self, s: &str, len: usize) -> Option<(usize, usize)> {
        if s.is_empty() || len == 0 {
            return None;
        }

        let parts: Vec<&str> = s.split(',').collect();

        let parse_idx = |idx_str: &str| -> Option<usize> {
            let idx: i64 = idx_str.trim().parse().ok()?;
            if idx < 0 {
                // Negative index from end
                let abs = (-idx) as usize;
                if abs > len {
                    None
                } else {
                    Some(len - abs)
                }
            } else if idx == 0 {
                Some(0)
            } else {
                // 1-indexed
                Some((idx as usize).saturating_sub(1).min(len))
            }
        };

        match parts.len() {
            1 => {
                // Single element [n]
                let idx = parse_idx(parts[0])?;
                Some((idx, idx + 1))
            }
            2 => {
                // Range [n,m]
                let start = parse_idx(parts[0])?;
                let end = parse_idx(parts[1])?.saturating_add(1);
                Some((start.min(end), start.max(end)))
            }
            _ => None,
        }
    }
    /// Split a string into words based on IFS
    pub(crate) fn split_words(&self, s: &str) -> Vec<String> {
        let ifs = self
            .variables
            .get("IFS")
            .cloned()
            .or_else(|| env::var("IFS").ok())
            .unwrap_or_else(|| " \t\n".to_string());

        if ifs.is_empty() {
            return vec![s.to_string()];
        }

        s.split(|c: char| ifs.contains(c))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
    /// Helper for `${arr[idx]:-default}` family — returns the element
    /// (or empty string if OOB / not present). Routes through assoc
    /// arrays first, then indexed arrays, then string subscripting.
    /// Uses the same numeric/range parsing as the main bracket handler
    /// but only the single-element case (sufficient for the modifiers
    /// that gate on emptiness).
    /// Companion to `lookup_array_element` — returns true iff the
    /// element at `index` is SET (key present for assoc, index in
    /// bounds for indexed array, char position in range for scalar
    /// substring). Used by `${arr[N]+set}` / `${arr[N]-default}` /
    /// `${arr[N]?msg}` — the no-colon variants test SET-ness, not
    /// empty-ness.
    pub(crate) fn array_element_is_set(&mut self, var_name: &str, index: &str) -> bool {
        if self.assoc_arrays.contains_key(var_name) {
            let key = self.singsub(index);
            return self
                .assoc_arrays
                .get(var_name)
                .map(|a| a.contains_key(&key))
                .unwrap_or(false);
        }
        let expanded_index = self.singsub(index);
        if let Ok(idx) = expanded_index.parse::<i64>() {
            if let Some(arr) = self.arrays.get(var_name) {
                let len = arr.len() as i64;
                let pos = if idx > 0 {
                    idx - 1
                } else if idx < 0 {
                    len + idx
                } else {
                    return false;
                };
                return pos >= 0 && pos < len;
            }
            // Scalar string — check if char index is in range.
            let val = self.get_variable(var_name);
            let n = val.chars().count() as i64;
            if n == 0 {
                return false;
            }
            let pos = if idx > 0 {
                idx - 1
            } else if idx < 0 {
                n + idx
            } else {
                return false;
            };
            return pos >= 0 && pos < n;
        }
        false
    }
    pub(crate) fn lookup_array_element(&mut self, var_name: &str, index: &str) -> String {
        if let Some(val) = self.get_special_array_value(var_name, index) {
            return val;
        }
        if self.assoc_arrays.contains_key(var_name) {
            let key = self.singsub(index);
            return self
                .assoc_arrays
                .get(var_name)
                .and_then(|a| a.get(&key).cloned())
                .unwrap_or_default();
        }
        let expanded_index = self.singsub(index);
        if let Ok(idx) = expanded_index.parse::<i64>() {
            if let Some(arr) = self.arrays.get(var_name) {
                let pos = if idx > 0 {
                    (idx - 1) as usize
                } else if idx < 0 {
                    let n = arr.len() as i64 + idx;
                    if n < 0 {
                        return String::new();
                    }
                    n as usize
                } else {
                    0
                };
                return arr.get(pos).cloned().unwrap_or_default();
            }
            // String subscript on scalar
            let val = self.get_variable(var_name);
            if val.is_empty() {
                return String::new();
            }
            let chars: Vec<char> = val.chars().collect();
            let pos = if idx > 0 {
                (idx - 1) as usize
            } else if idx < 0 {
                let n = chars.len() as i64 + idx;
                if n < 0 {
                    return String::new();
                }
                n as usize
            } else {
                0
            };
            return chars.get(pos).map(|c| c.to_string()).unwrap_or_default();
        }
        String::new()
    }
    /// Get value from zsh/parameter special arrays (options, commands, functions, etc.)
    /// Returns Some(value) if this is a special array access, None otherwise
    pub fn get_special_array_value(&self, array_name: &str, key: &str) -> Option<String> {
        match array_name {
            // === ZSH/MAPFILE module ===
            // `${mapfile[/path]}` reads the file's contents. Direct
            // port of `getpmmapfile()` (Src/Modules/mapfile.c:217)
            // which calls `get_contents()` (line 167) on the path.
            // Splice (`@`/`*`) returns the CWD entry list per
            // `scanpmmapfile()` (line 240).
            "mapfile" => {
                if key == "@" || key == "*" {
                    // Inline readdir loop — direct port of
                    // scanpmmapfile (Src/Modules/mapfile.c:241).
                    let mut files: Vec<String> = Vec::new();
                    if let Ok(rd) = std::fs::read_dir(".") {
                        for entry in rd.flatten() {
                            let path = entry.path();
                            if path.is_file() {
                                if let Some(name) =
                                    path.file_name().and_then(|n| n.to_str())
                                {
                                    files.push(name.to_string());
                                }
                            }
                        }
                    }
                    return Some(files.join(" "));
                }
                Some(crate::modules::mapfile::get_contents(key).unwrap_or_default())
            }
            // === ZSH/SYSTEM — errnos / sysparams ===
            "errnos" => {
                let table = crate::modules::system::ERRNO_NAMES;
                if key == "@" || key == "*" {
                    return Some(
                        table
                            .iter()
                            .map(|(n, _)| (*n).to_string())
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                }
                if let Ok(n) = key.parse::<i64>() {
                    let len = table.len() as i64;
                    let pos = if n > 0 {
                        (n - 1) as usize
                    } else if n < 0 {
                        let p = len + n;
                        if p < 0 {
                            return Some(String::new());
                        }
                        p as usize
                    } else {
                        return Some(String::new());
                    };
                    if let Some((name, _)) = table.get(pos) {
                        return Some((*name).to_string());
                    }
                }
                Some(String::new())
            }
            "sysparams" => {
                let pid = std::process::id().to_string();
                let ppid = unsafe { libc::getppid() }.to_string();
                if key == "@" || key == "*" {
                    return Some(format!("{} {}", pid, ppid));
                }
                Some(match key {
                    "pid" => pid,
                    "ppid" => ppid,
                    "procsubstpid" => "0".to_string(),
                    _ => String::new(),
                })
            }
            // === SHELL OPTIONS ===
            "options" => {
                if key == "@" || key == "*" {
                    // Return all options as "name=on/off" pairs
                    let opts: Vec<String> = self
                        .options
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, if *v { "on" } else { "off" }))
                        .collect();
                    return Some(opts.join(" "));
                }
                let opt_name = key.to_lowercase().replace('_', "");
                let is_on = self.options.get(&opt_name).copied().unwrap_or(false);
                Some(if is_on {
                    "on".to_string()
                } else {
                    "off".to_string()
                })
            }

            // === ALIASES ===
            // ${aliases[@]} returns values in sorted-name order.
            // Iterating HashMap::values() gave random order; tests
            // and prompt code that snapshot ${(v)aliases} flickered.
            "aliases" => {
                if key == "@" || key == "*" {
                    let mut keys: Vec<&String> = self.aliases.keys().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .filter_map(|k| self.aliases.get(*k).cloned())
                        .collect();
                    return Some(vals.join(" "));
                }
                Some(self.aliases.get(key).cloned().unwrap_or_default())
            }
            "galiases" => {
                if key == "@" || key == "*" {
                    let mut keys: Vec<&String> = self.global_aliases.keys().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .filter_map(|k| self.global_aliases.get(*k).cloned())
                        .collect();
                    return Some(vals.join(" "));
                }
                Some(self.global_aliases.get(key).cloned().unwrap_or_default())
            }
            "saliases" => {
                if key == "@" || key == "*" {
                    let mut keys: Vec<&String> = self.suffix_aliases.keys().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .filter_map(|k| self.suffix_aliases.get(*k).cloned())
                        .collect();
                    return Some(vals.join(" "));
                }
                Some(self.suffix_aliases.get(key).cloned().unwrap_or_default())
            }

            // === TERMINFO (zsh/terminfo module) ===
            // `${terminfo[capname]}` returns the escape sequence for
            // capability `capname`. Direct port of zsh/Src/Modules/
            // terminfo.c — the C version calls `tigetstr(name)` from
            // ncurses; we map the common-subset capability names to
            // standard xterm/VT escape sequences inline. Covers the
            // function-keys / cursor-motion / clear / color set that
            // user keymaps query (`key[F1]=$terminfo[kf1]` etc.).
            "terminfo" => {
                // Lazy lookup via ncurses tigetstr/tigetnum/tigetflag
                // — the pre-populated assoc init seeds the common
                // subset, but a script may query any cap by name
                // (`$terminfo[acsc]`, `$terminfo[colors]`). Mirror
                // zsh's terminfo.c::getterminfo lazy-resolve path.
                Some(crate::modules::terminfo::getterminfo(key).unwrap_or_default())
            }
            // `termcap` is dispatched in the `magic_assoc_lookup`
            // function (the primary special-array path) so that
            // ${termcap[cl]} resolves before this fallback runs.
            // Keeping a no-op arm here avoids a spurious "unknown
            // assoc" diagnostic if a caller bypasses
            // magic_assoc_lookup.
            "termcap" => Some(crate::modules::termcap::gettermcap(key).unwrap_or_default()),

            // === FUNCTIONS ===
            "functions" => {
                if key == "@" || key == "*" {
                    return Some(self.function_names().join(" "));
                }
                // Apply zsh's getfn_functions formatter — leading-tab
                // body, no trailing `;`. Direct port of Src/exec.c
                // shipped via compile_zsh's fast path; this branch
                // is the slow-path/subst_port entry that previously
                // returned the raw user-typed source. Keeps
                // `${functions[foo]:0:20}` (substring extraction)
                // consistent with the fast-path `\$functions[foo]`.
                let text = self.function_definition_text(key)?;
                let formatted = FuncBodyFmt::render(text.trim());
                Some(format!("\t{}", formatted))
            }
            "functions_source" => {
                // ${functions_source[name]} → file path where the
                // function was defined. zsh/Src/Modules/parameter.c
                // exposes this as an assoc keyed by function name.
                // For autoload functions we recover the source path
                // via the same fpath walk that loads them; for inline
                // functions we don't yet track the defining file, so
                // emit empty in that case.
                if key == "@" || key == "*" {
                    let mut all = String::new();
                    for fname in self.function_names() {
                        if let Some(p) = self.find_function_file(&fname) {
                            if !all.is_empty() {
                                all.push(' ');
                            }
                            all.push_str(&p.to_string_lossy());
                        }
                    }
                    return Some(all);
                }
                Some(
                    self.find_function_file(key)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_default(),
                )
            }

            // === COMMANDS (command hash table) ===
            // ${commands[name]} → full path (or empty), per
            // zsh/Modules/parameter.c. The @/* expansion enumerates
            // every command on PATH (deduplicated, first-wins).
            "commands" => {
                if key == "@" || key == "*" {
                    let path_var = env::var("PATH").unwrap_or_default();
                    let mut seen: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    let mut names: Vec<String> = Vec::new();
                    // Hashed entries first (rehash population).
                    for k in self.command_hash.keys() {
                        if seen.insert(k.clone()) {
                            names.push(k.clone());
                        }
                    }
                    for dir in path_var.split(':') {
                        if dir.is_empty() {
                            continue;
                        }
                        if let Ok(entries) = std::fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                if let Ok(name) = entry.file_name().into_string() {
                                    if seen.insert(name.clone()) {
                                        names.push(name);
                                    }
                                }
                            }
                        }
                    }
                    names.sort();
                    return Some(names.join(" "));
                }
                if let Some(path) = self.find_in_path(key) {
                    Some(path)
                } else {
                    Some(String::new())
                }
            }

            // === BUILTINS ===
            "builtins" => {
                let builtins = Self::get_builtin_names();
                if key == "@" || key == "*" {
                    return Some(builtins.join(" "));
                }
                if builtins.iter().any(|b| b == key) {
                    Some("defined".to_string())
                } else {
                    Some(String::new())
                }
            }

            // === PARAMETERS ===
            // ${parameters[name]} → full attribute string per
            // VarAttr::format_zsh (e.g. 'integer-readonly-export').
            // @/* enumerates every parameter name, sorted+deduped.
            "parameters" => {
                if key == "@" || key == "*" {
                    let mut names: std::collections::BTreeSet<String> =
                        self.variables.keys().cloned().collect();
                    names.extend(self.arrays.keys().cloned());
                    names.extend(self.assoc_arrays.keys().cloned());
                    let v: Vec<String> = names.into_iter().collect();
                    return Some(v.join(" "));
                }
                if let Some(attr) = self.var_attrs.get(key) {
                    return Some(attr.format_zsh());
                }
                if self.assoc_arrays.contains_key(key) {
                    Some("association".to_string())
                } else if self.arrays.contains_key(key) {
                    Some("array".to_string())
                } else if self.variables.contains_key(key) || std::env::var(key).is_ok() {
                    Some("scalar".to_string())
                } else {
                    Some(String::new())
                }
            }

            // === NAMED DIRECTORIES ===
            // ${nameddirs[@]} returns paths in sorted-name order (was
            // HashMap::values() with random iteration).
            "nameddirs" => {
                if key == "@" || key == "*" {
                    let mut keys: Vec<&String> = self.named_dirs.keys().collect();
                    keys.sort();
                    let vals: Vec<String> = keys
                        .iter()
                        .filter_map(|k| self.named_dirs.get(*k).map(|p| p.display().to_string()))
                        .collect();
                    return Some(vals.join(" "));
                }
                Some(
                    self.named_dirs
                        .get(key)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                )
            }

            // === USER DIRECTORIES ===
            // ${userdirs[name]} → home directory of user `name` per
            // zsh/Modules/parameter.c userdirs_*. With @/* expansion,
            // walk getpwent(3) to enumerate every passwd entry's
            // home directory.
            "userdirs" => {
                #[cfg(unix)]
                {
                    use std::ffi::{CStr, CString};
                    if key == "@" || key == "*" {
                        let mut homes: Vec<String> = Vec::new();
                        unsafe {
                            libc::setpwent();
                            loop {
                                let pwd = libc::getpwent();
                                if pwd.is_null() {
                                    break;
                                }
                                let dir = CStr::from_ptr((*pwd).pw_dir);
                                homes.push(dir.to_string_lossy().to_string());
                            }
                            libc::endpwent();
                        }
                        homes.sort();
                        homes.dedup();
                        return Some(homes.join(" "));
                    }
                    if let Ok(name) = CString::new(key) {
                        unsafe {
                            let pwd = libc::getpwnam(name.as_ptr());
                            if !pwd.is_null() {
                                let dir = CStr::from_ptr((*pwd).pw_dir);
                                return Some(dir.to_string_lossy().to_string());
                            }
                        }
                    }
                }
                Some(String::new())
            }

            // === USER GROUPS ===
            // ${usergroups[name]} → GID of group `name`. With @/*
            // expansion, walk getgrent(3) to enumerate every group's
            // gid.
            "usergroups" => {
                #[cfg(unix)]
                {
                    use std::ffi::{CStr, CString};
                    if key == "@" || key == "*" {
                        let mut gids: Vec<String> = Vec::new();
                        unsafe {
                            libc::setgrent();
                            loop {
                                let grp = libc::getgrent();
                                if grp.is_null() {
                                    break;
                                }
                                let name = CStr::from_ptr((*grp).gr_name);
                                gids.push(name.to_string_lossy().to_string());
                            }
                            libc::endgrent();
                        }
                        gids.sort();
                        gids.dedup();
                        return Some(gids.join(" "));
                    }
                    if let Ok(name) = CString::new(key) {
                        unsafe {
                            let grp = libc::getgrnam(name.as_ptr());
                            if !grp.is_null() {
                                return Some((*grp).gr_gid.to_string());
                            }
                        }
                    }
                }
                Some(String::new())
            }

            // === DIRECTORY STACK ===
            "dirstack" => {
                if key == "@" || key == "*" {
                    let dirs: Vec<String> = self
                        .dir_stack
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect();
                    return Some(dirs.join(" "));
                }
                if let Ok(idx) = key.parse::<usize>() {
                    Some(
                        self.dir_stack
                            .get(idx)
                            .map(|p| p.display().to_string())
                            .unwrap_or_default(),
                    )
                } else {
                    Some(String::new())
                }
            }

            // === JOBS ===
            "jobstates" => {
                if key == "@" || key == "*" {
                    let states: Vec<String> = self
                        .jobs
                        .iter()
                        .map(|(id, job)| format!("{}:{:?}", id, job.state))
                        .collect();
                    return Some(states.join(" "));
                }
                if let Ok(id) = key.parse::<usize>() {
                    if let Some(job) = self.jobs.get(id) {
                        return Some(format!("{:?}", job.state));
                    }
                }
                Some(String::new())
            }
            "jobtexts" => {
                if key == "@" || key == "*" {
                    let texts: Vec<String> = self
                        .jobs
                        .iter()
                        .map(|(_, job)| job.command.clone())
                        .collect();
                    return Some(texts.join(" "));
                }
                if let Ok(id) = key.parse::<usize>() {
                    if let Some(job) = self.jobs.get(id) {
                        return Some(job.command.clone());
                    }
                }
                Some(String::new())
            }
            "jobdirs" => {
                // ${jobdirs[N]}: cwd at the time job N was launched.
                // We don't yet capture per-job cwd at launch (would
                // need a JobInfo.cwd field plumbed through add_job),
                // so use the current PWD as a best-effort proxy. With
                // @/* expansion, return one entry per active job so
                // arr-length math (${#jobdirs}) matches ${#jobtexts}.
                let pwd = self
                    .variables
                    .get("PWD")
                    .cloned()
                    .or_else(|| env::var("PWD").ok())
                    .unwrap_or_default();
                if key == "@" || key == "*" {
                    let n = self.jobs.iter().count();
                    return Some(vec![pwd; n].join(" "));
                }
                if let Ok(id) = key.parse::<usize>() {
                    if self.jobs.get(id).is_some() {
                        return Some(pwd);
                    }
                }
                Some(String::new())
            }

            // === HISTORY ===
            "history" => {
                if key == "@" || key == "*" {
                    // Return recent history
                    if let Some(ref engine) = self.history {
                        if let Ok(entries) = engine.recent(100) {
                            let cmds: Vec<String> =
                                entries.iter().map(|e| e.command.clone()).collect();
                            return Some(cmds.join("\n"));
                        }
                    }
                    return Some(String::new());
                }
                if let Ok(num) = key.parse::<usize>() {
                    if let Some(ref engine) = self.history {
                        if let Ok(Some(entry)) = engine.get_by_offset(num.saturating_sub(1)) {
                            return Some(entry.command);
                        }
                    }
                }
                Some(String::new())
            }
            "historywords" => {
                // $historywords: flat list of words from recent history
                // entries (zsh/Modules/parameter.c historywords_*).
                // Each command is split on whitespace; the words are
                // collected newest-first across the recent window.
                if let Some(ref engine) = self.history {
                    if let Ok(entries) = engine.recent(100) {
                        let words: Vec<String> = entries
                            .iter()
                            .flat_map(|e| {
                                e.command
                                    .split_whitespace()
                                    .map(|s| s.to_string())
                                    .collect::<Vec<_>>()
                            })
                            .collect();
                        if key == "@" || key == "*" {
                            return Some(words.join(" "));
                        }
                        if let Ok(idx) = key.parse::<usize>() {
                            if idx >= 1 && idx <= words.len() {
                                return Some(words[idx - 1].clone());
                            }
                        }
                    }
                }
                Some(String::new())
            }

            // === MODULES ===
            // ${modules[name]} → "loaded" / "" per
            // zsh/Src/Modules/parameter.c modules_*. zshrs tracks
            // loaded modules via `_module_<name>` keys in
            // self.options (see bin_zmodload). Always-loaded
            // built-in modules are surfaced unconditionally so
            // compsys's `[[ ${+modules[zsh/zutil]} ]]` gating works.
            "modules" => {
                const ALWAYS_LOADED: &[&str] = &[
                    "zsh/parameter",
                    "zsh/zutil",
                    "zsh/complete",
                    "zsh/complist",
                    "zsh/zle",
                    "zsh/main",
                    "zsh/files",
                ];
                let user_loaded: Vec<String> = self
                    .options
                    .iter()
                    .filter_map(|(k, v)| {
                        if *v {
                            k.strip_prefix("_module_").map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                if key == "@" || key == "*" {
                    let mut all: Vec<String> = ALWAYS_LOADED
                        .iter()
                        .map(|s| s.to_string())
                        .chain(user_loaded.iter().cloned())
                        .collect();
                    all.sort();
                    all.dedup();
                    return Some(all.join(" "));
                }
                if ALWAYS_LOADED.contains(&key)
                    || self
                        .options
                        .get(&format!("_module_{}", key))
                        .copied()
                        .unwrap_or(false)
                {
                    Some("loaded".to_string())
                } else {
                    Some(String::new())
                }
            }

            // === RESERVED WORDS ===
            "reswords" => {
                let reswords = [
                    "do",
                    "done",
                    "esac",
                    "then",
                    "elif",
                    "else",
                    "fi",
                    "for",
                    "case",
                    "if",
                    "while",
                    "function",
                    "repeat",
                    "time",
                    "until",
                    "select",
                    "coproc",
                    "nocorrect",
                    "foreach",
                    "end",
                    "in",
                ];
                if key == "@" || key == "*" {
                    return Some(reswords.join(" "));
                }
                if let Ok(idx) = key.parse::<usize>() {
                    Some(reswords.get(idx).map(|s| s.to_string()).unwrap_or_default())
                } else {
                    Some(String::new())
                }
            }

            // === PATCHARS (characters with special meaning in patterns) ===
            "patchars" => {
                let patchars = ["?", "*", "[", "]", "^", "#", "~", "(", ")", "|"];
                if key == "@" || key == "*" {
                    return Some(patchars.join(" "));
                }
                if let Ok(idx) = key.parse::<usize>() {
                    Some(patchars.get(idx).map(|s| s.to_string()).unwrap_or_default())
                } else {
                    Some(String::new())
                }
            }

            // === FUNCTION CALL STACK ===
            // $funcstack: array of function names in the current call
            // chain (innermost first). Already maintained by the
            // function-call code at exec.rs:7828-7835. Surface it here
            // so `${funcstack[1]}` / `${funcstack[@]}` reads work.
            // funcfiletrace / funcsourcetrace need separate tables (file
            // and definition tracking) which we don't yet wire; emit
            // empty for those until they're populated.
            "funcstack" => {
                if let Some(stack) = self.arrays.get("funcstack") {
                    if key == "@" || key == "*" {
                        return Some(stack.join(" "));
                    }
                    if let Ok(idx) = key.parse::<usize>() {
                        // zsh subscripts are 1-based.
                        if idx >= 1 && idx <= stack.len() {
                            return Some(stack[idx - 1].clone());
                        }
                    }
                }
                Some(String::new())
            }
            "functrace" => {
                // $functrace: `caller_name:callsite_lineno` for each
                // frame. We don't yet track call-site line numbers, so
                // synthesize from funcstack with a `:0` placeholder
                // line. This still lets scripts that test
                // `[[ -n $functrace[1] ]]` work without false-empty.
                if let Some(stack) = self.arrays.get("funcstack") {
                    let synth: Vec<String> = stack.iter().map(|n| format!("{}:0", n)).collect();
                    if key == "@" || key == "*" {
                        return Some(synth.join(" "));
                    }
                    if let Ok(idx) = key.parse::<usize>() {
                        if idx >= 1 && idx <= synth.len() {
                            return Some(synth[idx - 1].clone());
                        }
                    }
                }
                Some(String::new())
            }
            "funcfiletrace" | "funcsourcetrace" => {
                // Would need file:line where each function was called
                // from / defined in. Per-frame file tracking is not yet
                // wired — return empty.
                Some(String::new())
            }

            // === DISABLED VARIANTS (dis_*) ===
            // ${dis_builtins[name]} → "defined" if the builtin was
            // disabled via `disable name`. Tracked through
            // self.options['_disabled_<name>']. The other dis_*
            // variants (aliases/functions/reswords/patchars) lose
            // their entries entirely on disable in zshrs's table
            // model (see do_enable_disable at exec.rs:31371) so the
            // disabled list isn't recoverable post-disable; emit
            // empty for those.
            "dis_builtins" => {
                let disabled: Vec<String> = self
                    .options
                    .iter()
                    .filter_map(|(k, v)| {
                        if *v {
                            k.strip_prefix("_disabled_").map(|s| s.to_string())
                        } else {
                            None
                        }
                    })
                    .collect();
                if key == "@" || key == "*" {
                    let mut sorted = disabled.clone();
                    sorted.sort();
                    return Some(sorted.join(" "));
                }
                if disabled.iter().any(|d| d == key) {
                    Some("defined".to_string())
                } else {
                    Some(String::new())
                }
            }
            "dis_aliases"
            | "dis_galiases"
            | "dis_saliases"
            | "dis_functions"
            | "dis_functions_source"
            | "dis_reswords"
            | "dis_patchars" => Some(String::new()),

            // === ZLE WIDGETS ===
            // ${widgets[name]} → widget-type prefix per
            // zsh/Src/Zle/zleparameter.c widgets_*: "builtin",
            // "user:<funcname>", or "completion:<funcname>".
            // Distinguishes builtin vs user-defined so
            // ${(t)widgets[name]} works.
            "widgets" => {
                use crate::zle::zle;
                let zle = zle();
                if key == "@" || key == "*" {
                    let mut names: Vec<&str> = zle.list_widgets();
                    names.sort();
                    return Some(
                        names
                            .into_iter()
                            .map(String::from)
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                }
                if let Some(target) = zle.get_widget(key) {
                    if target == key {
                        Some("builtin".to_string())
                    } else {
                        Some(format!("user:{}", target))
                    }
                } else {
                    Some(String::new())
                }
            }

            // === ZLE KEYMAPS ===
            // ${keymaps[N]} per zleparameter.c keymaps_*: list of
            // available keymap names. Single-key lookup returns 1
            // ("set") if the keymap exists, "" otherwise.
            "keymaps" => {
                const KEYMAPS: &[&str] = &[
                    "main",
                    "emacs",
                    "viins",
                    "vicmd",
                    "isearch",
                    "command",
                    "menuselect",
                ];
                if key == "@" || key == "*" {
                    return Some(KEYMAPS.join(" "));
                }
                if KEYMAPS.contains(&key) {
                    Some("1".to_string())
                } else {
                    Some(String::new())
                }
            }

            // === SIGNAL NAMES ===
            // $signals: array indexed by signal number (1-based) where
            // each slot holds the bare signal name. Direct port of
            // zsh/Modules/parameter.c signals_*. zshrs uses libc signal
            // constants so the mapping matches the host platform
            // (macOS USR1=30, Linux USR1=10).
            "signals" => {
                let map: &[(i32, &str)] = &[
                    (libc::SIGHUP, "HUP"),
                    (libc::SIGINT, "INT"),
                    (libc::SIGQUIT, "QUIT"),
                    (libc::SIGILL, "ILL"),
                    (libc::SIGTRAP, "TRAP"),
                    (libc::SIGABRT, "ABRT"),
                    #[cfg(target_os = "macos")]
                    (libc::SIGEMT, "EMT"),
                    (libc::SIGFPE, "FPE"),
                    (libc::SIGKILL, "KILL"),
                    (libc::SIGBUS, "BUS"),
                    (libc::SIGSEGV, "SEGV"),
                    (libc::SIGSYS, "SYS"),
                    (libc::SIGPIPE, "PIPE"),
                    (libc::SIGALRM, "ALRM"),
                    (libc::SIGTERM, "TERM"),
                    (libc::SIGURG, "URG"),
                    (libc::SIGSTOP, "STOP"),
                    (libc::SIGTSTP, "TSTP"),
                    (libc::SIGCONT, "CONT"),
                    (libc::SIGCHLD, "CHLD"),
                    (libc::SIGTTIN, "TTIN"),
                    (libc::SIGTTOU, "TTOU"),
                    (libc::SIGIO, "IO"),
                    (libc::SIGXCPU, "XCPU"),
                    (libc::SIGXFSZ, "XFSZ"),
                    (libc::SIGVTALRM, "VTALRM"),
                    (libc::SIGPROF, "PROF"),
                    (libc::SIGWINCH, "WINCH"),
                    #[cfg(target_os = "macos")]
                    (libc::SIGINFO, "INFO"),
                    (libc::SIGUSR1, "USR1"),
                    (libc::SIGUSR2, "USR2"),
                ];
                if key == "@" || key == "*" {
                    // Return one entry per signal in numeric order (1..N).
                    let max = map.iter().map(|(n, _)| *n).max().unwrap_or(0) as usize;
                    let mut slots: Vec<String> = vec![String::new(); max];
                    for (n, name) in map {
                        if (*n as usize) >= 1 && (*n as usize) <= max {
                            slots[*n as usize - 1] = (*name).to_string();
                        }
                    }
                    return Some(slots.join(" "));
                }
                // Numeric subscript -> name; name -> empty (zsh's
                // $signals is keyed by number).
                if let Ok(n) = key.parse::<i32>() {
                    for (sig_num, name) in map {
                        if *sig_num == n {
                            return Some((*name).to_string());
                        }
                    }
                }
                Some(String::new())
            }

            // Not a special array
            _ => None,
        }
    }
    pub(crate) fn get_variable(&self, name: &str) -> String {
        // Handle special parameters
        match name {
            "" => String::new(), // Empty name returns empty
            "$" => std::process::id().to_string(),
            "@" | "*" => {
                // $* joins by the first char of $IFS (POSIX). Default
                // IFS is " \t\n\0" so the join char is " "; with a
                // custom IFS like `:` the joined string uses `:`.
                // $@ technically does the same in scalar context but
                // is usually quoted-spliced — both fall through here.
                let sep = self
                    .variables
                    .get("IFS")
                    .and_then(|s| s.chars().next())
                    .unwrap_or(' ');
                self.positional_params.join(&sep.to_string())
            }
            "#" | "#@" | "#*" => self.positional_params.len().to_string(),
            // zsh alias: $ARGC also equals $#.
            "ARGC" => self.positional_params.len().to_string(),
            "?" | "status" => self.last_status.to_string(),
            "!" => self
                .variables
                .get("!")
                .cloned()
                .unwrap_or_else(|| "0".to_string()),
            // `$-` returns the concatenated single-letter flags of options
            // currently set. zsh always emits a baseline "569X" prefix
            // (internal-letter options that are on by default in -f mode)
            // followed by user-controllable flags. Match the prefix
            // verbatim so existing scripts that do `[[ $- == *e* ]]` /
            // `case $- in *x*) … esac` see consistent letters.
            "-" => {
                let mut letters = String::from("569X");
                let opt = |n: &str| self.options.get(n).copied().unwrap_or(false);
                // `e` comes BEFORE `f` in zsh's letter ordering: `set -e`
                // in -f mode produces "569Xef", not "569Xfe".
                if opt("errexit") {
                    letters.push('e');
                }
                if !opt("rcs") {
                    letters.push('f');
                }
                if opt("login") {
                    letters.push('l');
                }
                // i/m are present only when *truly* interactive; zsh's `-c`
                // path leaves them off, so we mirror that and don't surface
                // them just because `options.interactive` happens to be set
                // by the executor's default-options init.
                if opt("nounset") {
                    letters.push('u');
                }
                if opt("xtrace") {
                    letters.push('x');
                }
                if opt("verbose") {
                    letters.push('v');
                }
                if opt("noexec") {
                    letters.push('n');
                }
                if opt("hashall") {
                    letters.push('h');
                }
                letters
            }
            "EUID" => unsafe { libc::geteuid() }.to_string(),
            "UID" => unsafe { libc::getuid() }.to_string(),
            "EGID" => unsafe { libc::getegid() }.to_string(),
            "GID" => unsafe { libc::getgid() }.to_string(),
            "PPID" => unsafe { libc::getppid() }.to_string(),
            "ZSH_SUBSHELL" => self
                .variables
                .get("ZSH_SUBSHELL")
                .cloned()
                .unwrap_or_else(|| "0".to_string()),
            "HOST" => {
                // libc gethostname → up to 256 bytes.
                let mut buf = [0u8; 256];
                let r = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) };
                if r == 0 {
                    let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    String::from_utf8_lossy(&buf[..nul]).into_owned()
                } else {
                    String::new()
                }
            }
            // OS / machine identity vars. zsh hardcodes these from build-time
            // detection; we synthesize at runtime from libc uname(). Without
            // these arms `$OSTYPE` returned empty even though zle_params wrote
            // them into the params table — the executor's get_variable bypasses
            // that table for special names.
            "OSTYPE" => {
                let mut u: libc::utsname = unsafe { std::mem::zeroed() };
                if unsafe { libc::uname(&mut u) } == 0 {
                    let sysname = unsafe { std::ffi::CStr::from_ptr(u.sysname.as_ptr()) }
                        .to_string_lossy()
                        .to_lowercase();
                    let release = unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) }
                        .to_string_lossy()
                        .to_string();
                    format!("{}{}", sysname, release)
                } else {
                    std::env::consts::OS.to_string()
                }
            }
            "MACHTYPE" => {
                let mut u: libc::utsname = unsafe { std::mem::zeroed() };
                if unsafe { libc::uname(&mut u) } == 0 {
                    let m = unsafe { std::ffi::CStr::from_ptr(u.machine.as_ptr()) }
                        .to_string_lossy()
                        .to_string();
                    // zsh shortens common machines: aarch64 → arm, x86_64
                    // stays x86_64. Mirror that for the common cases.
                    if m == "aarch64" || m == "arm64" {
                        "arm".to_string()
                    } else {
                        m
                    }
                } else {
                    std::env::consts::ARCH.to_string()
                }
            }
            "CPUTYPE" => {
                let mut u: libc::utsname = unsafe { std::mem::zeroed() };
                if unsafe { libc::uname(&mut u) } == 0 {
                    unsafe { std::ffi::CStr::from_ptr(u.machine.as_ptr()) }
                        .to_string_lossy()
                        .to_string()
                } else {
                    std::env::consts::ARCH.to_string()
                }
            }
            "VENDOR" => {
                // No portable libc query for vendor; pick by OS family.
                if cfg!(target_os = "macos") {
                    "apple".to_string()
                } else if cfg!(target_os = "linux") {
                    "unknown".to_string()
                } else {
                    "pc".to_string()
                }
            }
            "HOSTNAME" => {
                let mut buf = [0u8; 256];
                let r = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) };
                if r == 0 {
                    let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    String::from_utf8_lossy(&buf[..nul]).into_owned()
                } else {
                    String::new()
                }
            }
            "RANDOM" => {
                // zsh/bash: pseudo-random unsigned 16-bit integer per
                // expansion. We use process+nano for a cheap, OS-portable
                // source — not cryptographically secure, but matches zsh's
                // "noise" semantics.
                use std::time::{SystemTime, UNIX_EPOCH};
                let nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() as u64)
                    .unwrap_or(0);
                let pid = std::process::id() as u64;
                let r = (nanos.wrapping_mul(2654435761).wrapping_add(pid)) as u32;
                ((r as u16) & 0x7fff).to_string()
            }
            "SECONDS" => {
                // Seconds since shell start. We approximate via the
                // tracked `shell_start_time` if present; otherwise 0.
                self.variables.get("SECONDS").cloned().unwrap_or_else(|| {
                    use std::time::{SystemTime, UNIX_EPOCH};
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let start = self
                        .variables
                        .get("__zshrs_start_secs")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(now);
                    now.saturating_sub(start).to_string()
                })
            }
            "EPOCHSECONDS" => {
                use std::time::{SystemTime, UNIX_EPOCH};
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_else(|_| "0".to_string())
            }
            "EPOCHREALTIME" => {
                // zsh/datetime: fractional seconds since the epoch with
                // microsecond resolution. Format: SECS.UUUUUU.
                use std::time::{SystemTime, UNIX_EPOCH};
                match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(d) => format!("{}.{:06}", d.as_secs(), d.subsec_micros()),
                    Err(_) => "0.000000".to_string(),
                }
            }
            "argv" => self.positional_params.join(" "),
            "HISTCMD" => {
                // zsh: HISTCMD = current history-event number. With -f
                // (no rc loading) and history-tracking off, zsh shows
                // 0. We mirror by returning the current session count
                // (or 0 when history isn't engaged).
                self.session_history_ids.len().to_string()
            }
            "TTY" => {
                // Path to the controlling terminal (`$TTY` in zsh).
                // ttyname(0) gives the device path. Returns "" if no tty.
                let ptr = unsafe { libc::ttyname(0) };
                if ptr.is_null() {
                    String::new()
                } else {
                    unsafe { std::ffi::CStr::from_ptr(ptr) }
                        .to_string_lossy()
                        .into_owned()
                }
            }
            "TTYIDLE" => {
                // Idle time of stdin TTY in seconds — stat the tty, return
                // (now - st_atime). Returns "-1" if not a tty per zsh docs.
                let ptr = unsafe { libc::ttyname(0) };
                if ptr.is_null() {
                    return "-1".to_string();
                }
                let path = unsafe { std::ffi::CStr::from_ptr(ptr) };
                let path_str = path.to_string_lossy().into_owned();
                match std::fs::metadata(&path_str) {
                    Ok(meta) => {
                        use std::time::SystemTime;
                        if let Ok(atime) = meta.accessed() {
                            let now = SystemTime::now();
                            let idle = now.duration_since(atime).unwrap_or_default();
                            return idle.as_secs().to_string();
                        }
                        "0".to_string()
                    }
                    Err(_) => "-1".to_string(),
                }
            }
            "TRY_BLOCK_ERROR" => {
                // Set by `{ … } always { … }` — last status of the try
                // block. Lives in self.variables under the same name when
                // the try arm assigns it; default 0.
                self.variables
                    .get("TRY_BLOCK_ERROR")
                    .cloned()
                    .unwrap_or_else(|| "0".to_string())
            }
            "patchars" => "*?[]<>(){}|^&;".to_string(),
            "RANDOM_FILE" => {
                // Path to entropy source. Mainline zsh leaves empty
                // unless `zmodload zsh/random` set it; we expose
                // /dev/urandom as a useful default — matches the
                // platform's actual entropy source.
                if std::path::Path::new("/dev/urandom").exists() {
                    "/dev/urandom".to_string()
                } else {
                    String::new()
                }
            }
            "LINENO" => {
                // Tracked elsewhere; default to 1 if not populated.
                self.variables
                    .get("LINENO")
                    .cloned()
                    .unwrap_or_else(|| "1".to_string())
            }
            "0" => self
                .variables
                .get("0")
                .cloned()
                .unwrap_or_else(|| env::args().next().unwrap_or_default()),
            n if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => {
                let idx: usize = n.parse().unwrap_or(0);
                if idx == 0 {
                    env::args().next().unwrap_or_default()
                } else {
                    self.positional_params
                        .get(idx - 1)
                        .cloned()
                        .unwrap_or_default()
                }
            }
            _ => {
                // Bare-assoc bypass: `declare -A h; h=(a 1 b 2); ${h}`
                // expects the joined values. The `declare -A` sets
                // variables["h"]="" as a side effect, which would
                // satisfy the variables lookup with empty. Skip the
                // variables lookup when an assoc with the same name
                // exists AND has entries.
                let assoc_has_entries = self
                    .assoc_arrays
                    .get(name)
                    .map(|h| !h.is_empty())
                    .unwrap_or(false);
                // GSU dispatch first — `$USERNAME` / `$IFS` / `$HOME`
                // / etc. route through their getfn callback. Mirrors
                // C zsh's `Param.gsu->getfn` lookup. Without this,
                // get_variable bypassed the GSU table entirely and
                // returned empty for usernamegetfn-backed reads.
                let resolved = lookup_special_var(name)
                    .or_else(|| {
                        if !assoc_has_entries {
                            self.variables.get(name).cloned()
                        } else {
                            None
                        }
                    })
                    .or_else(|| self.arrays.get(name).map(|a| a.join(" ")))
                    .or_else(|| {
                        self.assoc_arrays.get(name).map(|h| {
                            if h.is_empty() {
                                String::new()
                            } else {
                                h.values().cloned().collect::<Vec<_>>().join(" ")
                            }
                        })
                    })
                    .or_else(|| env::var(name).ok());
                match resolved {
                    Some(v) => v,
                    None => {
                        // zsh stores the option as "unset" (default ON =
                        // silently empty). `set -u` / `setopt nounset` /
                        // `set -o nounset` all turn it OFF. Different
                        // code paths in zshrs persist either key, so
                        // honor either signal.
                        let nounset_on = self.options.get("nounset").copied().unwrap_or(false)
                            || !self.options.get("unset").copied().unwrap_or(true);
                        if nounset_on {
                            zerr(&format!("{}: parameter not set", name));
                            std::process::exit(1);
                        }
                        String::new()
                    }
                }
            }
        }
    }
    /// Execute a command and capture its stdout (`$(cmd)` semantics).
    ///
    /// Bytecode-routed: compiles `cmd` to a chunk, runs on a fresh VM with
    /// stdout dup2'd to a pipe write end. Reads the pipe to a String. POSIX
    /// trims trailing newlines.
    /// Evaluate arithmetic expression using the full math module
    /// Pre-resolve `name[subscript]` references inside an arithmetic
    /// expression. MathEval only knows about scalar variables, so
    /// without this rewrite `m[k]` and `a[2]` evaluate to 0. We
    /// substitute the actual values inline before handing to the
    /// evaluator. Honors associative-array key lookups and 1-based
    /// numeric array indexing (with negative-from-end).
    /// First-pass resolver for `$NAME[…]` / `$@[…]` / `$*[…]`.
    /// Runs BEFORE expand_string so the array subscript stays bound
    /// to its variable name (otherwise `$@` joins to a scalar and
    /// the `[…]` becomes orphan text). Recognises both bare-numeric
    /// keys and zsh subscript-flag forms `(I)pat`, `(R)pat`, etc.
    /// Direct support for zinit's `(( $@[(I)-*] ))` pattern.
    pub(crate) fn pre_resolve_dollar_subscripts(&self, expr: &str) -> String {
        let bytes: Vec<char> = expr.chars().collect();
        let mut out = String::with_capacity(expr.len());
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c != '$' || i + 1 >= bytes.len() {
                out.push(c);
                i += 1;
                continue;
            }
            // `$#NAME` — length of array NAME (zsh shorthand for
            // `${#NAME}`). The C math lexer (Src/math.c::zzlex)
            // dispatches `#` after `$` via getstr() which calls
            // `getstrvalue()` with the param's flags including
            // PM_ARRAY length. Direct port: substitute the count
            // before arith eval reaches the lexer.
            let next = bytes[i + 1];
            if next == '#' {
                let name_start = i + 2;
                let mut name_end = name_start;
                while name_end < bytes.len()
                    && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == '_')
                {
                    name_end += 1;
                }
                if name_end > name_start {
                    let name: String = bytes[name_start..name_end].iter().collect();
                    let count = if let Some(arr) = self.arrays.get(&name) {
                        arr.len()
                    } else if let Some(assoc) = self.assoc_arrays.get(&name) {
                        assoc.len()
                    } else if name == "@" || name == "*" {
                        self.positional_params.len()
                    } else if let Some(s) = self.variables.get(&name) {
                        s.chars().count()
                    } else {
                        0
                    };
                    out.push_str(&count.to_string());
                    i = name_end;
                    continue;
                }
                // `$#` alone (no name) — single-char special; skip.
                out.push(c);
                i += 1;
                continue;
            }
            // Skip `$$`/`$?` — single-char specials, not arrays.
            let is_at_or_star = next == '@' || next == '*';
            let is_ident_start = next.is_ascii_alphabetic() || next == '_';
            if !is_at_or_star && !is_ident_start {
                out.push(c);
                i += 1;
                continue;
            }
            // Collect the name.
            let name_start = i + 1;
            let mut name_end = name_start + 1;
            if !is_at_or_star {
                while name_end < bytes.len()
                    && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == '_')
                {
                    name_end += 1;
                }
            }
            // Must be followed by `[` to qualify.
            if name_end >= bytes.len() || bytes[name_end] != '[' {
                out.push(c);
                i += 1;
                continue;
            }
            let name: String = bytes[name_start..name_end].iter().collect();
            // Collect balanced [...] for the key.
            let key_start = name_end + 1;
            let mut j = key_start;
            let mut depth = 1;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            let key_str: String = bytes[key_start..j].iter().collect();
            let trimmed_key = key_str.trim_start();
            let resolved = if trimmed_key.starts_with('(') {
                // getarg dispatches to the right pattern-search arm
                // based on which storage we pass it. Direct port of
                // C getarg's ishash branch (params.c:1581-1719).
                let scalar_val = self.variables.get(&name).cloned();
                let result = if let Some(assoc) = self.assoc_arrays.get(&name) {
                    getarg(trimmed_key, None, Some(assoc), None)
                } else if name == "@" || name == "*" {
                    let pos = self.positional_params.clone();
                    getarg(trimmed_key, Some(&pos), None, None)
                } else if let Some(arr) = self.arrays.get(&name).cloned() {
                    getarg(trimmed_key, Some(&arr), None, None)
                } else if let Some(ref s) = scalar_val {
                    getarg(trimmed_key, None, None, Some(s.as_str()))
                } else {
                    None
                };
                match result {
                    Some(GetargOut::Value(v)) => v.to_str(),
                    _ => "0".to_string(),
                }
            } else if let Some(assoc) = self.assoc_arrays.get(&name) {
                let key_clean = if (key_str.starts_with('"') && key_str.ends_with('"'))
                    || (key_str.starts_with('\'') && key_str.ends_with('\''))
                {
                    key_str[1..key_str.len() - 1].to_string()
                } else {
                    key_str.clone()
                };
                assoc
                    .get(&key_clean)
                    .cloned()
                    .unwrap_or_else(|| "0".to_string())
            } else if name == "@" || name == "*" {
                if let Ok(idx) = key_str.trim().parse::<i64>() {
                    let len = self.positional_params.len() as i64;
                    let pos = if idx < 0 { len + idx } else { idx - 1 };
                    if pos >= 0 && (pos as usize) < self.positional_params.len() {
                        self.positional_params[pos as usize].clone()
                    } else {
                        "0".to_string()
                    }
                } else {
                    "0".to_string()
                }
            } else if let Some(arr) = self.arrays.get(&name) {
                if let Ok(idx) = key_str.trim().parse::<i64>() {
                    let len = arr.len() as i64;
                    let pos = if idx < 0 { len + idx } else { idx - 1 };
                    if pos >= 0 && (pos as usize) < arr.len() {
                        arr[pos as usize].clone()
                    } else {
                        "0".to_string()
                    }
                } else {
                    "0".to_string()
                }
            } else {
                // Leave the original text — let downstream complain.
                let original: String = bytes[i..=j].iter().collect();
                original
            };
            out.push_str(&resolved);
            i = j + 1; // consume the closing `]`
        }
        out
    }
    pub(crate) fn pre_resolve_array_subscripts(&self, expr: &str) -> String {
        let bytes: Vec<char> = expr.chars().collect();
        let mut out = String::with_capacity(expr.len());
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            // `$@`, `$*`, `$NAME` followed by `[…]` — zinit's
            // `(( $@[(I)-*] ))` and similar arith uses this. Strip
            // the leading `$` and route through the same name+[key]
            // resolver as bare identifiers. Without this the `$@`
            // gets variable-expanded to its joined form before
            // arith eval, dropping the subscript flag entirely.
            if c == '$' && i + 1 < bytes.len() {
                let next = bytes[i + 1];
                let is_special_at = next == '@' || next == '*';
                let is_ident_start = next.is_ascii_alphabetic() || next == '_';
                if (is_special_at || is_ident_start) && i + 2 < bytes.len() {
                    // Look-ahead: must be followed by `[` to qualify
                    // as a subscript form. Bare `$@` without `[` is
                    // left alone (downstream substitution handles it).
                    let mut probe = i + 1;
                    if is_special_at {
                        probe += 1;
                    } else {
                        while probe < bytes.len()
                            && (bytes[probe].is_ascii_alphanumeric() || bytes[probe] == '_')
                        {
                            probe += 1;
                        }
                    }
                    if probe < bytes.len() && bytes[probe] == '[' {
                        // Drop the `$` and re-enter the bare-ident
                        // path on the next iteration.
                        i += 1;
                        continue;
                    }
                }
            }
            // Identifier start?
            if c.is_ascii_alphabetic() || c == '_' || c == '@' || c == '*' {
                let start = i;
                i += 1;
                if !(bytes[start] == '@' || bytes[start] == '*') {
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == '_') {
                        i += 1;
                    }
                }
                let name: String = bytes[start..i].iter().collect();
                if i < bytes.len() && bytes[i] == '[' {
                    // Collect balanced [...]
                    i += 1;
                    let key_start = i;
                    let mut depth = 1;
                    while i < bytes.len() && depth > 0 {
                        match bytes[i] {
                            '[' => depth += 1,
                            ']' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let key_str: String = bytes[key_start..i].iter().collect();
                    if i < bytes.len() {
                        i += 1; // skip closing ]
                    }
                    // Resolve sub-key (it may itself be an arith expr or
                    // string literal); strip surrounding quotes and
                    // expand $-refs.
                    let key_resolved: String = if key_str.starts_with('"') && key_str.ends_with('"')
                        || key_str.starts_with('\'') && key_str.ends_with('\'')
                    {
                        key_str[1..key_str.len() - 1].to_string()
                    } else {
                        key_str.clone()
                    };
                    // Subscript-flag form `(I)pat` / `(i)pat` etc. —
                    // route through array_subscript_flag so zinit's
                    // `(( $@[(I)-*] ))` and `(( OPTS[opt_-h,…] ))`
                    // patterns yield an index/key as zsh does.
                    let trimmed_key = key_resolved.trim_start();
                    let resolved = if trimmed_key.starts_with('(') {
                        // getarg with the right storage gives back the
                        // matched value or the all-matches join — see
                        // params.c:1581-1719 inside getarg.
                        let scalar_val = self.variables.get(&name).cloned();
                        let result = if let Some(assoc) = self.assoc_arrays.get(&name) {
                            getarg(trimmed_key, None, Some(assoc), None)
                        } else if name == "@" || name == "*" {
                            let pos = self.positional_params.clone();
                            getarg(trimmed_key, Some(&pos), None, None)
                        } else if let Some(arr) = self.arrays.get(&name).cloned() {
                            getarg(trimmed_key, Some(&arr), None, None)
                        } else if let Some(ref s) = scalar_val {
                            getarg(trimmed_key, None, None, Some(s.as_str()))
                        } else {
                            None
                        };
                        match result {
                            Some(GetargOut::Value(v)) => v.to_str(),
                            _ => "0".to_string(),
                        }
                    } else if let Some(assoc) = self.assoc_arrays.get(&name) {
                        assoc
                            .get(&key_resolved)
                            .cloned()
                            .unwrap_or_else(|| "0".to_string())
                    } else if let Some(arr) = self.arrays.get(&name) {
                        // Numeric subscript — can be a literal or an
                        // expression. For simple int literals only here;
                        // complex exprs are uncommon in real scripts.
                        if let Ok(idx) = key_resolved.trim().parse::<i64>() {
                            let len = arr.len() as i64;
                            let pos = if idx < 0 { len + idx } else { idx - 1 };
                            if pos >= 0 && (pos as usize) < arr.len() {
                                arr[pos as usize].clone()
                            } else {
                                "0".to_string()
                            }
                        } else {
                            "0".to_string()
                        }
                    } else {
                        // Unrecognised — emit the original text so the
                        // evaluator can complain naturally.
                        format!("{}[{}]", name, key_str)
                    };
                    out.push_str(&resolved);
                } else {
                    out.push_str(&name);
                }
                continue;
            }
            out.push(c);
            i += 1;
        }
        out
    }
    /// Apply `typeset -F N` / `-E N` precision when writing a float-
    /// typed variable. Direct port of zsh's params.c:
    /// `floatsetfn` formats the f64 through `convfloat()` which
    /// honors PM_FFLOAT/PM_EFLOAT + the declared precision before
    /// store. Without this, `typeset -F 3 x; (( x = 2.5 ))` stored
    /// the f64::to_string default instead of the expected `2.500`.
    pub(crate) fn format_for_var_attr(&self, name: &str, value: &str) -> String {
        let attr = match self.var_attrs.get(name) {
            Some(a) => a,
            None => return value.to_string(),
        };
        if !matches!(attr.kind, VarKind::Float) {
            return value.to_string();
        }
        let prec = match attr.float_precision {
            Some(p) => p,
            None => return value.to_string(),
        };
        let f: f64 = match value.parse() {
            Ok(f) => f,
            Err(_) => return value.to_string(),
        };
        if attr.float_exp {
            let frac_prec = prec.saturating_sub(1);
            let raw = format!("{:.prec$e}", f, prec = frac_prec);
            if let Some(epos) = raw.rfind('e') {
                let (mantissa, exp) = raw.split_at(epos);
                let exp_body = &exp[1..];
                let (sign, digits) = if let Some(d) = exp_body.strip_prefix('-') {
                    ("-", d)
                } else if let Some(d) = exp_body.strip_prefix('+') {
                    ("+", d)
                } else {
                    ("+", exp_body)
                };
                let padded = if digits.len() < 2 {
                    format!("0{}", digits)
                } else {
                    digits.to_string()
                };
                format!("{}e{}{}", mantissa, sign, padded)
            } else {
                raw
            }
        } else {
            format!("{:.prec$}", f, prec = prec)
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/hist.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Expand history references: !!, !n, !-n, !string, !?string?
    pub(crate) fn expand_history(&self, input: &str) -> String {
        let Some(ref engine) = self.history else {
            return input.to_string();
        };

        // Quick check: nothing to expand
        if !input.contains('!') && !input.starts_with('^') {
            return input.to_string();
        }

        // History expansion only fires in interactive mode (zsh's default).
        // For `-c` script mode, `!!` etc. are literal — pulling from the
        // persistent history db would inject random commands from the user's
        // saved sessions. We anchor on stdin-is-tty, which is the
        // unambiguous signal — the `interactive` option may be set on by
        // default in zshrs's options table for compat. atty::is checks the
        // OS-level fd state.
        if !atty::is(atty::Stream::Stdin) {
            return input.to_string();
        }

        let history_count = engine.count().unwrap_or(0) as usize;
        if history_count == 0 {
            return input.to_string();
        }

        let chars: Vec<char> = input.chars().collect();

        // ^foo^bar quick substitution (only at start of input)
        if chars.first() == Some(&'^') {
            if let Some(expanded) = self.history_quick_subst(&chars, engine) {
                return expanded;
            }
        }

        let mut result = String::new();
        let mut i = 0;
        let mut in_single_quote = false;
        let mut in_brace = 0; // Track ${...} nesting
        let mut last_subst: Option<(String, String)> = None; // for :& modifier

        while i < chars.len() {
            // Track single quotes — no history expansion inside them
            if chars[i] == '\'' && in_brace == 0 {
                in_single_quote = !in_single_quote;
                result.push(chars[i]);
                i += 1;
                continue;
            }
            if in_single_quote {
                result.push(chars[i]);
                i += 1;
                continue;
            }

            // Track ${...} nesting
            if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' {
                in_brace += 1;
                result.push(chars[i]);
                i += 1;
                result.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '}' && in_brace > 0 {
                in_brace -= 1;
                result.push(chars[i]);
                i += 1;
                continue;
            }

            // Backslash-escaped ! is literal
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '!' {
                result.push('!');
                i += 2;
                continue;
            }

            if chars[i] == '!' && in_brace == 0 {
                if i + 1 >= chars.len() {
                    // Trailing ! — literal
                    result.push('!');
                    i += 1;
                    continue;
                }

                let next = chars[i + 1];
                // ! followed by space, =, ( — literal (zsh rule)
                if next == ' ' || next == '\t' || next == '=' || next == '(' || next == '\n' {
                    result.push('!');
                    i += 1;
                    continue;
                }

                // Resolve the event string
                let (event_str, new_i) = self.history_resolve_event(&chars, i, engine, &result);
                if let Some(ev) = event_str {
                    // Check for word designators and modifiers
                    let (final_str, final_i) = self.history_apply_designators_and_modifiers(
                        &chars,
                        new_i,
                        &ev,
                        &mut last_subst,
                    );
                    result.push_str(&final_str);
                    i = final_i;
                } else {
                    // Could not resolve — keep the ! literal
                    result.push('!');
                    i += 1;
                }
                continue;
            }
            result.push(chars[i]);
            i += 1;
        }

        result
    }
    /// ^foo^bar quick substitution — replace first occurrence of foo with bar
    /// in the previous command.
    pub(crate) fn history_quick_subst(
        &self,
        chars: &[char],
        engine: &crate::history::HistoryEngine,
    ) -> Option<String> {
        let mut i = 1; // skip leading ^
        let mut old = String::new();
        while i < chars.len() && chars[i] != '^' {
            old.push(chars[i]);
            i += 1;
        }
        if i >= chars.len() {
            return None;
        }
        i += 1; // skip middle ^
        let mut new = String::new();
        while i < chars.len() && chars[i] != '^' && chars[i] != '\n' {
            new.push(chars[i]);
            i += 1;
        }
        let prev = engine.get_by_offset(0).ok()??;
        Some(prev.command.replacen(&old, &new, 1))
    }
    /// Resolve which history event ! refers to.  Returns (Some(full_command), index_after_event)
    /// or (None, original_index) if we can't resolve.
    pub(crate) fn history_resolve_event(
        &self,
        chars: &[char],
        bang_pos: usize,
        engine: &crate::history::HistoryEngine,
        current_line: &str,
    ) -> (Option<String>, usize) {
        let mut i = bang_pos + 1; // past the !

        // !{...} brace-wrapped event
        let in_brace = i < chars.len() && chars[i] == '{';
        if in_brace {
            i += 1;
        }

        let c = if i < chars.len() {
            chars[i]
        } else {
            return (None, bang_pos);
        };

        let (event, new_i) = match c {
            '!' => {
                // !! — previous command
                let entry = engine.get_by_offset(0).ok().flatten();
                (entry.map(|e| e.command), i + 1)
            }
            '#' => {
                // !# — current command line so far
                (Some(current_line.to_string()), i + 1)
            }
            '-' => {
                // !-n — nth previous command
                i += 1;
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                if i > start {
                    let n: usize = chars[start..i]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0);
                    if n > 0 {
                        let entry = engine.get_by_offset(n - 1).ok().flatten();
                        (entry.map(|e| e.command), i)
                    } else {
                        (None, bang_pos)
                    }
                } else {
                    (None, bang_pos)
                }
            }
            '?' => {
                // !?string? — contains search
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '?' && chars[i] != '\n' {
                    i += 1;
                }
                let search: String = chars[start..i].iter().collect();
                if i < chars.len() && chars[i] == '?' {
                    i += 1;
                }
                let entry = engine
                    .search(&search, 1)
                    .ok()
                    .and_then(|v| v.into_iter().next());
                (entry.map(|e| e.command), i)
            }
            c if c.is_ascii_digit() => {
                // !n — command by absolute number
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let n: i64 = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                if n > 0 {
                    let entry = engine.get_by_number(n).ok().flatten();
                    (entry.map(|e| e.command), i)
                } else {
                    (None, bang_pos)
                }
            }
            '$' => {
                // !$ — last word of previous command (shorthand for !!:$)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word =
                    entry.and_then(|e| Self::history_split_words(&e.command).last().cloned());
                // Return the word directly — skip designator parsing
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            '^' => {
                // !^ — first arg of previous command (shorthand for !!:1)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word = entry.and_then(|e| {
                    let words = Self::history_split_words(&e.command);
                    words.get(1).cloned()
                });
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            '*' => {
                // !* — all args of previous command (shorthand for !!:*)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word = entry.map(|e| {
                    let words = Self::history_split_words(&e.command);
                    if words.len() > 1 {
                        words[1..].join(" ")
                    } else {
                        String::new()
                    }
                });
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            c if c.is_alphabetic() || c == '_' || c == '/' || c == '.' => {
                // !string — prefix search
                let start = i;
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && chars[i] != ':'
                    && chars[i] != '!'
                    && chars[i] != '}'
                {
                    i += 1;
                }
                let prefix: String = chars[start..i].iter().collect();
                let entry = engine
                    .search_prefix(&prefix, 1)
                    .ok()
                    .and_then(|v| v.into_iter().next());
                (entry.map(|e| e.command), i)
            }
            _ => (None, bang_pos),
        };

        // Skip closing brace
        let final_i = if in_brace && new_i < chars.len() && chars[new_i] == '}' {
            new_i + 1
        } else {
            new_i
        };

        (event, final_i)
    }
    /// Split a command string into words for word designators, respecting quotes.
    pub(crate) fn history_split_words(cmd: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut in_sq = false;
        let mut in_dq = false;
        let mut escaped = false;

        for c in cmd.chars() {
            if escaped {
                current.push(c);
                escaped = false;
                continue;
            }
            if c == '\\' {
                current.push(c);
                escaped = true;
                continue;
            }
            if c == '\'' && !in_dq {
                in_sq = !in_sq;
                current.push(c);
                continue;
            }
            if c == '"' && !in_sq {
                in_dq = !in_dq;
                current.push(c);
                continue;
            }
            if c.is_whitespace() && !in_sq && !in_dq {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                continue;
            }
            current.push(c);
        }
        if !current.is_empty() {
            words.push(current);
        }
        words
    }
    /// Apply word designators (:0, :n, :^, :$, :*, :n-m) and modifiers
    /// (:h, :t, :r, :e, :s/old/new/, :gs/old/new/, :p, :l, :u, :q, :Q, :a, :A)
    /// to an already-resolved event string.
    pub(crate) fn history_apply_designators_and_modifiers(
        &self,
        chars: &[char],
        mut i: usize,
        event: &str,
        last_subst: &mut Option<(String, String)>,
    ) -> (String, usize) {
        let words = Self::history_split_words(event);
        let argc = words.len().saturating_sub(1); // last word index

        // Check for word designator — either :N or bare :^ :$ :*
        let mut sline = event.to_string();

        if i < chars.len() && chars[i] == ':' {
            i += 1;
            if i < chars.len() {
                // Parse word designator
                let (farg, larg, new_i) = self.history_parse_word_range(chars, i, argc);
                i = new_i;
                if farg.is_some() || larg.is_some() {
                    let f = farg.unwrap_or(0);
                    let l = larg.unwrap_or(argc);
                    let selected: Vec<&String> = words
                        .iter()
                        .enumerate()
                        .filter(|(idx, _)| *idx >= f && *idx <= l)
                        .map(|(_, w)| w)
                        .collect();
                    sline = selected
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                }
            }
        } else if i < chars.len() && chars[i] == '*' {
            // !!* shorthand for !!:1-$
            i += 1;
            if words.len() > 1 {
                sline = words[1..].join(" ");
            } else {
                sline = String::new();
            }
        }

        // Apply modifiers (:h :t :r :e :s :gs :p :l :u :q :Q :a :A)
        while i < chars.len() && chars[i] == ':' {
            i += 1;
            if i >= chars.len() {
                break;
            }
            let mut global = false;
            if chars[i] == 'g' && i + 1 < chars.len() {
                global = true;
                i += 1;
            }
            match chars[i] {
                'h' => {
                    // Head — remove trailing path component
                    i += 1;
                    if let Some(pos) = sline.rfind('/') {
                        if pos > 0 {
                            sline = sline[..pos].to_string();
                        } else {
                            sline = "/".to_string();
                        }
                    }
                }
                't' => {
                    // Tail — remove leading path components
                    i += 1;
                    if let Some(pos) = sline.rfind('/') {
                        sline = sline[pos + 1..].to_string();
                    }
                }
                'r' => {
                    // Remove extension
                    i += 1;
                    if let Some(pos) = sline.rfind('.') {
                        if pos > 0 && sline[..pos].rfind('/').is_none_or(|sp| sp < pos) {
                            sline = sline[..pos].to_string();
                        }
                    }
                }
                'e' => {
                    // Extension only
                    i += 1;
                    if let Some(pos) = sline.rfind('.') {
                        sline = sline[pos + 1..].to_string();
                    } else {
                        sline = String::new();
                    }
                }
                'l' => {
                    // Lowercase
                    i += 1;
                    sline = sline.to_lowercase();
                }
                'u' => {
                    // Uppercase
                    i += 1;
                    sline = sline.to_uppercase();
                }
                'p' => {
                    // Print only, don't execute (we just expand — caller handles this)
                    i += 1;
                    // For now, just expand — :p suppression would need upstream support
                }
                'q' => {
                    // Quote — single-bslashquote the result
                    i += 1;
                    sline = format!("'{}'", sline.replace('\'', "'\\''"));
                }
                'Q' => {
                    // Unquote — remove one level of shell quoting.
                    // zsh hist.c remquote: strips matching `'`/`"` pairs
                    // AND backslash escapes (`\X` → `X`). Without the
                    // backslash unescape, `a="a\\ b"; echo ${a:Q}` left
                    // the `\ ` sequence intact instead of giving `a b`.
                    i += 1;
                    let bytes: Vec<char> = sline.chars().collect();
                    let mut out = String::with_capacity(sline.len());
                    let mut j = 0;
                    let mut in_dq = false;
                    let mut in_sq = false;
                    while j < bytes.len() {
                        let c = bytes[j];
                        if in_sq {
                            if c == '\'' {
                                in_sq = false;
                            } else {
                                out.push(c);
                            }
                            j += 1;
                            continue;
                        }
                        if in_dq {
                            if c == '"' {
                                in_dq = false;
                            } else if c == '\\' && j + 1 < bytes.len() {
                                j += 1;
                                out.push(bytes[j]);
                            } else {
                                out.push(c);
                            }
                            j += 1;
                            continue;
                        }
                        match c {
                            '\'' => in_sq = true,
                            '"' => in_dq = true,
                            '\\' if j + 1 < bytes.len() => {
                                j += 1;
                                out.push(bytes[j]);
                            }
                            _ => out.push(c),
                        }
                        j += 1;
                    }
                    sline = out;
                }
                'a' => {
                    // Absolute path
                    i += 1;
                    if !sline.starts_with('/') {
                        if let Ok(cwd) = std::env::current_dir() {
                            sline = format!("{}/{}", cwd.display(), sline);
                        }
                    }
                }
                'A' => {
                    // Realpath
                    i += 1;
                    if let Ok(real) = std::fs::canonicalize(&sline) {
                        sline = real.to_string_lossy().to_string();
                    }
                }
                's' | 'S' => {
                    // :s/old/new/ or :gs/old/new/
                    i += 1;
                    if i < chars.len() {
                        let delim = chars[i];
                        i += 1;
                        let mut old_s = String::new();
                        while i < chars.len() && chars[i] != delim {
                            old_s.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1;
                        } // skip delimiter
                        let mut new_s = String::new();
                        while i < chars.len()
                            && chars[i] != delim
                            && chars[i] != ':'
                            && chars[i] != ' '
                        {
                            new_s.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() && chars[i] == delim {
                            i += 1;
                        } // skip trailing delimiter
                        *last_subst = Some((old_s.clone(), new_s.clone()));
                        if global {
                            sline = sline.replace(&old_s, &new_s);
                        } else {
                            sline = sline.replacen(&old_s, &new_s, 1);
                        }
                    }
                }
                '&' => {
                    // Repeat last substitution
                    i += 1;
                    if let Some((ref old_s, ref new_s)) = last_subst {
                        if global {
                            sline = sline.replace(old_s.as_str(), new_s.as_str());
                        } else {
                            sline = sline.replacen(old_s.as_str(), new_s.as_str(), 1);
                        }
                    }
                }
                _ => {
                    if global {
                        // 'g' was consumed but next char isn't s/S/& — put back
                        // by not advancing i further
                    }
                    break;
                }
            }
        }

        (sline, i)
    }
    /// Parse a word range like 0, 1, ^, $, *, n-m, n-
    pub(crate) fn history_parse_word_range(
        &self,
        chars: &[char],
        mut i: usize,
        argc: usize,
    ) -> (Option<usize>, Option<usize>, usize) {
        if i >= chars.len() {
            return (None, None, i);
        }

        // Check for modifiers that aren't word designators
        match chars[i] {
            'h' | 't' | 'r' | 'e' | 's' | 'S' | 'g' | 'p' | 'q' | 'Q' | 'l' | 'u' | 'a' | 'A'
            | '&' => {
                // This is a modifier, not a word designator — back up
                return (None, None, i - 1); // -1 to re-read the ':'
            }
            _ => {}
        }

        let farg = if chars[i] == '^' {
            i += 1;
            Some(1usize)
        } else if chars[i] == '$' {
            i += 1;
            return (Some(argc), Some(argc), i);
        } else if chars[i] == '*' {
            i += 1;
            return (Some(1), Some(argc), i);
        } else if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let n: usize = chars[start..i]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            Some(n)
        } else {
            None
        };

        // Check for range: n-m or n-
        if i < chars.len() && chars[i] == '-' {
            i += 1;
            if i < chars.len() && chars[i] == '$' {
                i += 1;
                return (farg, Some(argc), i);
            } else if i < chars.len() && chars[i].is_ascii_digit() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let m: usize = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                return (farg, Some(m), i);
            } else {
                // n- means n to argc-1
                return (farg, Some(argc.saturating_sub(1)), i);
            }
        }

        if farg.is_some() {
            (farg, farg, i)
        } else {
            (None, None, i)
        }
    }
    /// Check if a string starts with history modifier characters
    pub(crate) fn is_history_modifier(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let first = s.chars().next().unwrap();
        matches!(
            first,
            // `g` is the prefix for `:gs/.../.../` (global substitution).
            // `s` is `:s/old/new/`. `U`/`L`/`V`/`X` are bash-only forms
            // we accept here so they reach apply_history_modifiers and
            // emit zsh's "unrecognized modifier" error rather than
            // silently falling through to an empty substitution.
            'A' | 'a'
                | 'h'
                | 't'
                | 'r'
                | 'e'
                | 'l'
                | 'u'
                | 'q'
                | 'Q'
                | 'P'
                | 's'
                | 'g'
                | 'U'
                | 'L'
                | 'V'
                | 'X'
        )
    }
    /// Apply zsh history-style modifiers to a value
    /// Modifiers can be chained: :A:h:h
    pub(crate) fn apply_history_modifiers(&self, val: &str, modifiers: &str) -> String {
        let mut result = val.to_string();
        let mut chars = modifiers.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                ':' => continue,
                'A' => {
                    if let Ok(abs) = std::fs::canonicalize(&result) {
                        result = abs.to_string_lossy().to_string();
                    } else {
                        // canonicalize() requires the path to exist. For
                        // non-existent paths zsh still removes `./` and
                        // resolves `..` lexically — `./foo` → `<cwd>/foo`,
                        // not `<cwd>/./foo`. Without this normalization,
                        // `${a:A}` for `a=./foo` left the `./` segment in
                        // the output even after the cwd-prefix.
                        let joined = if result.starts_with('/') {
                            std::path::PathBuf::from(&result)
                        } else if let Ok(cwd) = std::env::current_dir() {
                            cwd.join(&result)
                        } else {
                            std::path::PathBuf::from(&result)
                        };
                        let mut parts: Vec<String> = Vec::new();
                        for comp in joined.components() {
                            use std::path::Component::*;
                            match comp {
                                CurDir => {}
                                ParentDir => {
                                    parts.pop();
                                }
                                Normal(s) => parts.push(s.to_string_lossy().to_string()),
                                RootDir => parts.insert(0, String::new()),
                                Prefix(p) => {
                                    parts.insert(0, p.as_os_str().to_string_lossy().to_string())
                                }
                            }
                        }
                        result = parts.join("/");
                        if result.is_empty() {
                            result = "/".to_string();
                        }
                    }
                }
                'a' => {
                    if !result.starts_with('/') {
                        if let Ok(cwd) = std::env::current_dir() {
                            result = cwd.join(&result).to_string_lossy().to_string();
                        }
                    }
                }
                'h' => {
                    // zsh strips trailing slashes BEFORE applying head:
                    // `/tmp/` :h is `/`, not `/tmp`. Repeatedly trim
                    // trailing `/` first, then drop the last segment.
                    let trimmed = result.trim_end_matches('/');
                    if trimmed.is_empty() {
                        // Pure-slash input (`/`, `//`, …) — head is `/`.
                        result = "/".to_string();
                    } else if let Some(pos) = trimmed.rfind('/') {
                        if pos == 0 {
                            result = "/".to_string();
                        } else {
                            result = trimmed[..pos].to_string();
                        }
                    } else {
                        result = ".".to_string();
                    }
                }
                't' => {
                    // Mirror zsh: strip trailing slashes before tail
                    // extraction so `foo/` :t is `foo`, not the empty
                    // segment after the slash.
                    let trimmed = result.trim_end_matches('/');
                    if let Some(pos) = trimmed.rfind('/') {
                        result = trimmed[pos + 1..].to_string();
                    } else {
                        result = trimmed.to_string();
                    }
                }
                'r' => {
                    if let Some(dot_pos) = result.rfind('.') {
                        let slash_pos = result.rfind('/').map(|p| p + 1).unwrap_or(0);
                        if dot_pos > slash_pos {
                            result = result[..dot_pos].to_string();
                        }
                    }
                }
                'e' => {
                    if let Some(dot_pos) = result.rfind('.') {
                        let slash_pos = result.rfind('/').map(|p| p + 1).unwrap_or(0);
                        if dot_pos > slash_pos {
                            result = result[dot_pos + 1..].to_string();
                        } else {
                            result = String::new();
                        }
                    } else {
                        result = String::new();
                    }
                }
                'l' => {
                    // `:l` lowercase. Direct port of
                    // src/zsh/Src/hist.c:931-933 — calls casemodify
                    // with CASMOD_LOWER. Use the faithful
                    // casemodify port instead of plain to_lowercase
                    // for Unicode-correct multibyte handling.
                    result = casemodify(&result, CaseMod::Lower);
                }
                'u' => {
                    // `:u` uppercase. Port of src/zsh/Src/hist.c:934-936.
                    result = casemodify(&result, CaseMod::Upper);
                }
                'C' => {
                    // `:C` capitalize. zsh-only modifier per
                    // hist.c (see CASMOD_CAPS dispatch via
                    // casemodify). The history-modifier loop's
                    // legacy path didn't recognize `:C` — only the
                    // `(C)` parameter flag did. Same semantics:
                    // word-aware capitalization with mid-word
                    // lowercase enforcement.
                    result = casemodify(&result, CaseMod::Caps);
                }
                'q' => {
                    // zsh `:q` uses backslash quoting, not single-bslashquote
                    // wrapping. Each shell-meta char gets a `\` prefix.
                    let mut out = String::with_capacity(result.len() + 8);
                    for ch in result.chars() {
                        if " \t\n'\"\\$`;|&<>()[]{}*?#~!".contains(ch) {
                            out.push('\\');
                        }
                        out.push(ch);
                    }
                    result = out;
                }
                'x' => {
                    // `:x` bslashquote with word breaks. Direct port of
                    // src/zsh/Src/hist.c:2527-2556 quotebreak —
                    // wraps the value in single quotes, escapes
                    // internal `'` as `'\''`, AND closes-then-reopens
                    // SQ around each whitespace char (so `hello world`
                    // becomes `'hello' 'world'`). Already ported as a
                    // standalone helper in zle_hist.
                    result = crate::hist::quotebreak(&result);
                }
                'Q' => {
                    // Same shell-bslashquote-remove as the other :Q path
                    // (hist.c remquote): strips matching `'`/`"` pairs
                    // AND backslash escapes inside or unquoted.
                    let bytes: Vec<char> = result.chars().collect();
                    let mut out = String::with_capacity(result.len());
                    let mut j = 0;
                    let mut in_dq = false;
                    let mut in_sq = false;
                    while j < bytes.len() {
                        let c = bytes[j];
                        if in_sq {
                            if c == '\'' {
                                in_sq = false;
                            } else {
                                out.push(c);
                            }
                            j += 1;
                            continue;
                        }
                        if in_dq {
                            if c == '"' {
                                in_dq = false;
                            } else if c == '\\' && j + 1 < bytes.len() {
                                j += 1;
                                out.push(bytes[j]);
                            } else {
                                out.push(c);
                            }
                            j += 1;
                            continue;
                        }
                        match c {
                            '\'' => in_sq = true,
                            '"' => in_dq = true,
                            '\\' if j + 1 < bytes.len() => {
                                j += 1;
                                out.push(bytes[j]);
                            }
                            _ => out.push(c),
                        }
                        j += 1;
                    }
                    result = out;
                }
                'P' => {
                    if let Ok(real) = std::fs::canonicalize(&result) {
                        result = real.to_string_lossy().to_string();
                    }
                }
                'g' => {
                    // `:g` is a prefix to `:s` (or `:&`) meaning "global
                    // substitution". Peek next char — if `s` or `&`,
                    // route through the substitution arm with global=true.
                    let global = true;
                    let next = chars.next();
                    match next {
                        Some('s') => {
                            /* :g substitute — stubbed pending faithful subst.c modify() port */ let _ = global;
                        }
                        _ => {
                            // Stray `:g` without `:s`/`:&` follow-up —
                            // unrecognized in zsh, exit modifier loop.
                            break;
                        }
                    }
                }
                's' => {
                    // `:s/old/new/` — single substitution. Delimiter is
                    // the char after `s` (typically `/`). Final delim
                    // optional.
                    /* :s/old/new/ — stubbed pending faithful subst.c modify() port */
                }
                // Bash-only modifiers — zsh rejects with "unrecognized
                // modifier". Match that error format. Without these arms,
                // unknown modifiers silently terminated the loop and the
                // caller saw the previous-stage value (often empty).
                'U' | 'L' | 'V' | 'X' => {
                    zerr(&format!("unrecognized modifier `{}'", c));
                    result = String::new();
                    break;
                }
                _ => break,
            }
        }
        result
    }
}

// =====================================================================
// MOVED FROM: src/ported/signals.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Execute trap handlers for a signal
    pub fn run_trap(&mut self, signal: &str) {
        if let Some(action) = self.traps.get(signal).cloned() {
            // Empty action = signal-ignore. Don't try to execute "".
            if !action.is_empty() {
                let _ = self.execute_script(&action);
            }
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/prompt.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Expand prompt escape sequences using the full prompt module
    pub(crate) fn expand_prompt_string(&self, s: &str) -> String {
        let ctx = self.build_prompt_context();
        expand_prompt(s, &ctx)
    }
    /// Same as `expand_prompt_string` but strips the readline cursor-
    /// width markers (`\x01` / `\x02`) and any spurious leading-reset
    /// `\e[0m` that the prompt expander emits before its first
    /// real-attr block. Used by `print -P` (zsh's `print -P` produces
    /// raw ANSI bytes for terminal display, not PS1-style markers).
    pub(crate) fn expand_prompt_string_for_print(&self, s: &str) -> String {
        let raw = self.expand_prompt_string(s);
        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();
        let mut emitted_anything = false;
        while let Some(c) = chars.next() {
            if c == '\x01' || c == '\x02' {
                continue;
            }
            // Strip a leading `\e[0m` that immediately precedes another
            // `\e[?` — it's the apply_attrs preamble, not a user-asked
            // reset. Conservative: only strip when nothing real has
            // been emitted yet.
            if !emitted_anything && c == '\x1b' && chars.peek() == Some(&'[') {
                let mut lookahead = String::new();
                let iter = chars.clone();
                for ch in iter {
                    lookahead.push(ch);
                    if ch.is_ascii_alphabetic() {
                        break;
                    }
                    if lookahead.len() > 8 {
                        break;
                    }
                }
                if lookahead == "[0m" {
                    // Skip the `[0m` (3 chars: `[`, `0`, `m`).
                    let mut peek2 = chars.clone();
                    peek2.next(); // [
                    peek2.next(); // 0
                    peek2.next(); // m
                    if peek2.peek() == Some(&'\x1b') {
                        // Confirm followed by another escape — the
                        // suppression is safe.
                        chars.next(); // [
                        chars.next(); // 0
                        chars.next(); // m
                        continue;
                    }
                }
            }
            out.push(c);
            if !c.is_whitespace() && c != '\x1b' {
                emitted_anything = true;
            }
        }
        out
    }
    /// Build a PromptContext from current executor state
    pub(crate) fn build_prompt_context(&self) -> PromptContext {
        // zsh's prompt expansion uses the *logical* pwd (`$PWD` env var
        // as set by `cd`), not the canonicalized `getcwd()` form. On
        // macOS, `cd /tmp` leaves `$PWD=/tmp` but `getcwd()` returns
        // `/private/tmp`, which would make `%2d` print `/private/tmp`
        // instead of `/tmp` to match zsh.
        let pwd = env::var("PWD")
            .ok()
            .filter(|p| !p.is_empty())
            .or_else(|| {
                env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "/".to_string());

        let home = env::var("HOME").unwrap_or_default();

        let user = env::var("USER")
            .or_else(|_| env::var("LOGNAME"))
            .unwrap_or_else(|_| "user".to_string());

        let host = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "localhost".to_string());

        let host_short = host.split('.').next().unwrap_or(&host).to_string();

        // Prefer the in-shell SHLVL (already incremented by 1 over
        // the parent's value at startup) over the env var which
        // still holds the parent's pre-increment count. Without
        // this, `print -P "%L"` was off by one (showed parent's
        // SHLVL, not zshrs's).
        let shlvl = self
            .variables
            .get("SHLVL")
            .and_then(|s| s.parse().ok())
            .or_else(|| env::var("SHLVL").ok().and_then(|s| s.parse().ok()))
            .unwrap_or(1);

        PromptContext {
            pwd,
            home,
            user,
            host,
            host_short,
            tty: String::new(),
            lastval: self.last_status,
            // zsh's `%h`/`%!` is the *current line* number — counted
            // from session start, not the persistent on-disk history
            // size. In `-c` (non-interactive) mode no command has been
            // recorded yet, so zsh emits 0. Use a session counter on
            // the executor instead of the disk count.
            histnum: self.session_histnum,
            shlvl,
            num_jobs: self.jobs.list().len() as i32,
            is_root: unsafe { libc::geteuid() } == 0,
            // `%_` in PS4 / prompt expansion renders the cumulative
            // control-flow context labels (`if`, `then`, `cmdand`,
            // `cmdor`, `cmdsubst`, …) — feed the executor's live
            // `cmd_stack` (pushed by BUILTIN_CMD_PUSH around each
            // compound command, popped by BUILTIN_CMD_POP) so the
            // prompt expander sees what zsh's `cmdstack` global
            // would show. Direct port of Src/prompt.c:855-887 `%_`
            // expansion which iterates the cmdstack and joins
            // names with spaces.
            cmd_stack: self.cmd_stack.clone(),
            psvar: self.get_psvar(),
            term_width: self.get_term_width(),
            // `$LINENO` is updated by `BUILTIN_SET_LINENO` before
            // every top-level pipe (compile_zsh.rs:142), carrying
            // the parser's `ZshPipe.lineno`. Reading it here lets
            // `%i` / `%I` / `%h` prompt expansion (and the xtrace
            // prefix that wraps each command) reflect the source
            // line currently executing — matching zsh's
            // `printprompt4()` reading the `lineno` C global before
            // it expands `prompt4`. Falls back to 1 only on the very
            // first dispatch before any SET_LINENO has fired.
            lineno: self
                .variables
                .get("LINENO")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(1),
            // `%N` / `%x` resolution per Src/prompt.c:541-556:
            // scriptname wins over argzero. C zsh keeps a separate
            // `scriptname` global (Src/init.c) — set to the binary
            // basename in `-c` mode (init.c:479), to the resolved
            // path when sourcing a file (init.c:1591), and to the
            // function name during a function call (exec.c:5903).
            // The dedicated `self.scriptname` field tracks that. Fall
            // back through $0 then $ZSH_ARGZERO if it's unset (e.g.
            // a script-file invocation that hasn't pushed a frame
            // yet).
            scriptname: self
                .scriptname
                .clone()
                .or_else(|| self.variables.get("0").cloned()),
            // %x reads scriptfilename — the file being read, NOT
            // the active function name. Falls back to scriptname
            // when scriptfilename is unset (the no-function case
            // where they coincide).
            scriptfilename: self
                .scriptfilename
                .clone()
                .or_else(|| self.scriptname.clone())
                .or_else(|| self.variables.get("0").cloned()),
            argzero: self
                .variables
                .get("ZSH_ARGZERO")
                .cloned()
                .unwrap_or_else(|| {
                    std::env::args().next().unwrap_or_else(|| "zsh".to_string())
                }),
        }
    }
    /// Interpret bindkey-style escapes per zsh/Src/utils.c:getkeystring
    /// when called with GETKEYS_BINDKEY. Superset of expand_printf_escapes:
    /// adds `\C-x` (ctrl-X), `\M-y` (meta-Y, high bit set), and `\^x`
    /// (alias for `\C-x`). Used by `print -b`.
    pub(crate) fn expand_bindkey_escapes(&self, s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('a') => out.push('\x07'),
                Some('b') => out.push('\x08'),
                Some('e') | Some('E') => out.push('\x1b'),
                Some('f') => out.push('\x0c'),
                Some('v') => out.push('\x0b'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('\'') => out.push('\''),
                // \C-x: ctrl+x (mask 0x1f). Accepts both `\C-X` and `\C-x`.
                Some('C') => {
                    if chars.peek() == Some(&'-') {
                        chars.next();
                    }
                    if let Some(target) = chars.next() {
                        let upper = target.to_ascii_uppercase();
                        let code = (upper as u32) & 0x1f;
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                        }
                    }
                }
                // \^x: alias for \C-x (sans the optional `-`).
                Some('^') => {
                    if let Some(target) = chars.next() {
                        let upper = target.to_ascii_uppercase();
                        let code = (upper as u32) & 0x1f;
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                        }
                    }
                }
                // \M-x: meta+x (set high bit on x). Most terminals emit
                // ESC + x for this, so emit `\x1b` followed by the
                // unmodified char to match the convention.
                Some('M') => {
                    if chars.peek() == Some(&'-') {
                        chars.next();
                    }
                    if let Some(target) = chars.next() {
                        out.push('\x1b');
                        out.push(target);
                    }
                }
                // \xHH hex byte
                Some('x') => {
                    let mut hex = String::new();
                    for _ in 0..2 {
                        if let Some(&p) = chars.peek() {
                            if p.is_ascii_hexdigit() {
                                hex.push(p);
                                chars.next();
                                continue;
                            }
                        }
                        break;
                    }
                    if let Ok(n) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(n) {
                            out.push(ch);
                        }
                    }
                }
                // \NNN octal (1-3 digits)
                Some(c) if c.is_digit(8) => {
                    let mut oct = String::from(c);
                    for _ in 0..2 {
                        if let Some(&p) = chars.peek() {
                            if p.is_digit(8) {
                                oct.push(p);
                                chars.next();
                                continue;
                            }
                        }
                        break;
                    }
                    if let Ok(n) = u32::from_str_radix(&oct, 8) {
                        if let Some(ch) = char::from_u32(n) {
                            out.push(ch);
                        }
                    }
                }
                Some(c) => {
                    out.push('\\');
                    out.push(c);
                }
                None => out.push('\\'),
            }
        }
        out
    }
    pub(crate) fn apply_prompt_theme(&mut self, theme: &str, preview: bool) {
        let (ps1, rps1) = match theme {
            "minimal" => ("%# ", ""),
            "off" => ("$ ", ""),
            "adam1" => (
                "%B%F{cyan}%n@%m %F{blue}%~%f%b %# ",
                "%F{yellow}%D{%H:%M}%f",
            ),
            "redhat" => ("[%n@%m %~]$ ", ""),
            _ => ("%n@%m %~ %# ", ""),
        };
        if preview {
            println!("PS1={:?}", ps1);
            println!("RPS1={:?}", rps1);
        } else {
            self.variables.insert("PS1".to_string(), ps1.to_string());
            self.variables.insert("RPS1".to_string(), rps1.to_string());
            self.variables
                .insert("prompt_theme".to_string(), theme.to_string());
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/prompt.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    pub(crate) fn get_psvar(&self) -> Vec<String> {
        if let Some(arr) = self.arrays.get("psvar") {
            arr.clone()
        } else {
            Vec::new()
        }
    }
    pub(crate) fn get_term_width(&self) -> usize {
        env::var("COLUMNS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(80)
    }
}

// =====================================================================
// MOVED FROM: src/ported/module.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// zmodload - load/unload zsh modules (stub)
    pub(crate) fn bin_zmodload(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `zmodload` event per module name (only
        // for load form — listing/query/unload are not state mutations
        // we want recorded as definitions).
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let mut listing_or_query = false;
            let mut flags: Vec<&str> = Vec::new();
            for a in args {
                match a.as_str() {
                    "-l" | "-L" | "-e" | "-u" => listing_or_query = true,
                    s if s.starts_with('-') => flags.push(s),
                    _ => {}
                }
            }
            if !listing_or_query {
                let ctx = self.recorder_ctx();
                let flag_blob = flags.join(" ");
                for a in args {
                    if a.starts_with('-') {
                        continue;
                    }
                    crate::recorder::emit_zmodload(a, &flag_blob, ctx.clone());
                }
            }
        }
        // Direct port of zsh/Src/module.c bin_zmodload control flow.
        // zshrs's modules are compiled-in, so load/unload simply toggle
        // a tracking flag in self.options['_module_<name>']. Listing
        // returns the union of always-loaded compiled-in modules plus
        // any user-zmodload'd entries.
        let mut list_loaded = false;
        let mut list_reusable = false;
        let mut unload = false;
        let mut existence_test = false; // -e: query whether loaded
        let mut modules: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-l" => list_loaded = true,
                "-L" => {
                    list_loaded = true;
                    list_reusable = true;
                }
                "-u" => unload = true,
                "-e" => existence_test = true,
                "-a" | "-b" | "-c" | "-d" | "-f" | "-i" | "-p" | "-s" | "-A" | "-R" | "-F"
                | "-I" | "-P" => {}
                _ if arg.starts_with('-') => {
                    // BUILTIN("zmodload", ..., "AFRILP:abcfdilmpsue")
                    // declares the valid letter set. Unknown flags
                    // silently dropped previously, masking typos.
                    let bad: String = arg[1..].chars().take(1).collect();
                    zwarnnam("zmodload", &format!("bad option: -{}", bad));
                    return 1;
                }
                _ => modules.push(arg),
            }
        }

        // Compiled-in modules zshrs supports. Same set the brew zsh
        // ships in /opt/homebrew/lib/zsh/*.bundle, plus a few zshrs-
        // specific entries (`zsh/profiler`, `zsh/main`, `zsh/random_real`,
        // `zsh/param_private`, etc.). `zmodload NAME` accepts any of
        // these; unknown names error like `failed to load module`.
        const ALWAYS_LOADED: &[&str] = &[
            "zsh/attr",
            "zsh/cap",
            "zsh/clone",
            "zsh/compctl",
            "zsh/complete",
            "zsh/complist",
            "zsh/computil",
            "zsh/curses",
            "zsh/datetime",
            "zsh/db/gdbm",
            "zsh/deltochar",
            "zsh/example",
            "zsh/files",
            "zsh/hlgroup",
            "zsh/ksh93",
            "zsh/langinfo",
            "zsh/main",
            "zsh/mapfile",
            "zsh/mathfunc",
            "zsh/nearcolor",
            "zsh/net/socket",
            "zsh/net/tcp",
            "zsh/newuser",
            "zsh/param/private",
            "zsh/parameter",
            "zsh/pcre",
            "zsh/private",
            "zsh/profiler",
            "zsh/random",
            "zsh/random_real",
            "zsh/regex",
            "zsh/rlimits",
            "zsh/sched",
            "zsh/stat",
            "zsh/system",
            "zsh/termcap",
            "zsh/terminfo",
            "zsh/watch",
            "zsh/zftp",
            "zsh/zle",
            "zsh/zleparameter",
            "zsh/zprof",
            "zsh/zpty",
            "zsh/zselect",
            "zsh/zutil",
        ];

        // `is_loaded` answers: has the user explicitly loaded this
        // module via `zmodload NAME` in this shell? Direct port of
        // zsh's `-e` semantics (Src/module.c bin_zmodload case 'e'):
        // "loaded" is observable state, NOT compile-time presence.
        // zshrs links every module statically, but `-e` must reflect
        // user-controlled load actions only; otherwise scripts that
        // do `zmodload -e zsh/datetime || zmodload zsh/datetime` get
        // the wrong answer.
        let is_loaded = |this: &Self, name: &str| -> bool {
            this
                .options
                .get(&format!("_module_{}", name))
                .copied()
                .unwrap_or(false)
        };

        // -e: existence test. Exit 0 if all named modules are loaded,
        // else 1. zsh module.c bin_zmodload case 'e'.
        if existence_test {
            for m in &modules {
                if !is_loaded(self, m) {
                    return 1;
                }
            }
            return 0;
        }

        if list_loaded || modules.is_empty() {
            let mut all: Vec<String> = ALWAYS_LOADED.iter().map(|s| s.to_string()).collect();
            for (k, v) in &self.options {
                if *v {
                    if let Some(name) = k.strip_prefix("_module_") {
                        all.push(name.to_string());
                    }
                }
            }
            all.sort();
            all.dedup();
            for m in &all {
                if list_reusable {
                    println!("zmodload {}", m);
                } else {
                    println!("{}", m);
                }
            }
            return 0;
        }

        // Reject unknown module names. zsh's bin_zmodload (Src/module.c)
        // attempts dlopen on the named bundle and reports
        // "failed to load module" when the lookup misses; zshrs has
        // no dlopen layer (modules are statically linked), so we
        // gate on the `ALWAYS_LOADED` whitelist instead. Without
        // this check, `zmodload zsh/no_such_module` silently
        // succeeded, masking typos in user scripts.
        for module in &modules {
            if !ALWAYS_LOADED.contains(module) {
                zwarnnam("zmodload", &format!("failed to load module: {}", module));
                return 1;
            }
        }
        for module in modules {
            if unload {
                self.options.remove(&format!("_module_{}", module));
            } else {
                self.options.insert(format!("_module_{}", module), true);
            }
        }
        0
    }
}

// =====================================================================
// MOVED FROM: src/ported/math.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    pub fn evaluate_arithmetic(&mut self, expr: &str) -> String {
        // First, resolve `$NAME[(flags)pat]` / `$@[(flags)pat]`
        // before expand_string — otherwise `$@` gets joined into
        // a scalar (`a b c`) and the trailing `[…]` becomes
        // ambiguous text. zinit relies on `(( $@[(I)-*] ))`.
        let expr_pre = if expr.contains('$') {
            self.pre_resolve_dollar_subscripts(expr)
        } else {
            expr.to_string()
        };
        // Only run expand_string when the expression has `$` (for
        // var/cmd-subst/nested-arith). Otherwise pass through —
        // expand_string would tilde-expand `~` (bitwise NOT in arith
        // context) into "no such user" errors.
        let expr = if expr_pre.contains('$') || expr_pre.contains('`') {
            self.singsub(&expr_pre)
        } else {
            expr_pre
        };
        // Subscripted-array compound-assign / increment / decrement:
        // `((a[i]++))`, `((a[i]+=v))`, `((a[i]-=v))`, etc. Read the
        // current value, apply the operation, write back. MathState
        // can't write through `a[i]` for compound forms (only the
        // bare `=` write was special-cased below), so handle here.
        // Subscript compound op: `((a[i]++))`, `((h[k]+=5))`, etc.
        // Combined post-op + pre-op detection. Direct port of zsh
        // math.c LVAL_NUM_SUBSC: the subscript receiver retains its
        // lvalue identity across the operator. Without this,
        // pre_resolve_array_subscripts substitutes the value first
        // and `5++` errors "lvalue required".
        let compound = parse_compound(&expr)
            .map(|(n, i, o, r)| (n, i, o, r, false))
            .or_else(|| {
                parse_pre_inc(&expr).map(|(n, i, o)| (n, i, o, String::new(), true))
            });
        if let Some((name, idx_expr, op, rhs, is_pre)) = compound {
            let is_assoc = self.assoc_arrays.contains_key(&name);
            let idx_val = if is_assoc {
                0
            } else {
                self.eval_arith_expr(&idx_expr)
            };
            let key_str = if is_assoc {
                let s = idx_expr.trim();
                if (s.starts_with('"') && s.ends_with('"'))
                    || (s.starts_with('\'') && s.ends_with('\''))
                {
                    s[1..s.len() - 1].to_string()
                } else {
                    s.to_string()
                }
            } else {
                String::new()
            };
            let rhs_val = if rhs.is_empty() {
                1
            } else {
                self.eval_arith_expr(&rhs)
            };
            let cur: i64 = if is_assoc {
                self.assoc_arrays
                    .get(&name)
                    .and_then(|m| m.get(&key_str))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            } else if let Some(arr) = self.arrays.get(&name) {
                let len = arr.len() as i64;
                let pos = if idx_val < 0 {
                    len + idx_val
                } else {
                    idx_val - 1
                };
                if pos >= 0 && (pos as usize) < arr.len() {
                    arr[pos as usize].parse().unwrap_or(0)
                } else {
                    0
                }
            } else {
                0
            };
            let new_val: i64 = match op.as_str() {
                "++" => cur + 1,
                "--" => cur - 1,
                "+=" => cur + rhs_val,
                "-=" => cur - rhs_val,
                "*=" => cur * rhs_val,
                "/=" => {
                    if rhs_val == 0 {
                        zerr("division by zero");
                        return cur.to_string();
                    }
                    cur / rhs_val
                }
                "%=" => {
                    if rhs_val == 0 {
                        zerr("division by zero");
                        return cur.to_string();
                    }
                    cur % rhs_val
                }
                "&=" => cur & rhs_val,
                "|=" => cur | rhs_val,
                "^=" => cur ^ rhs_val,
                "<<=" => cur << rhs_val,
                ">>=" => cur >> rhs_val,
                "**=" => (cur as f64).powi(rhs_val as i32) as i64,
                _ => cur,
            };
            // Write back.
            if is_assoc {
                if let Some(map) = self.assoc_arrays.get_mut(&name) {
                    map.insert(key_str, new_val.to_string());
                }
            } else if let Some(arr) = self.arrays.get_mut(&name) {
                let len = arr.len() as i64;
                let pos = if idx_val < 0 {
                    len + idx_val
                } else {
                    idx_val - 1
                };
                if pos >= 0 {
                    let p = pos as usize;
                    if p >= arr.len() {
                        arr.resize(p + 1, "0".to_string());
                    }
                    arr[p] = new_val.to_string();
                }
            } else {
                // Auto-create indexed array.
                let mut arr: Vec<String> = Vec::new();
                let pos = (idx_val - 1).max(0) as usize;
                arr.resize(pos + 1, "0".to_string());
                arr[pos] = new_val.to_string();
                self.arrays.insert(name, arr);
            }
            // Post `++`/`--` returns OLD value; pre-op + compound
            // assigns return NEW value.
            let result = if !is_pre && (op == "++" || op == "--") {
                cur
            } else {
                new_val
            };
            return result.to_string();
        }
        // Subscripted-array arith assignment: `((a[i]=expr))`. Without
        // this special case, pre_resolve_array_subscripts would
        // substitute a[i] with its current value (`0=42` → invalid).
        if let Some((name, idx_expr, rhs)) = parse_assign(&expr) {
            let idx_val = self.eval_arith_expr(&idx_expr);
            let rhs_val = self.eval_arith_expr(&rhs);
            if let Some(arr) = self.arrays.get_mut(&name) {
                let i_pos = if idx_val < 0 {
                    arr.len() as i64 + idx_val
                } else {
                    idx_val - 1
                };
                if i_pos >= 0 {
                    let pos = i_pos as usize;
                    if pos >= arr.len() {
                        arr.resize(pos + 1, "0".to_string());
                    }
                    arr[pos] = rhs_val.to_string();
                }
            } else if let Some(map) = self.assoc_arrays.get_mut(&name) {
                map.insert(idx_val.to_string(), rhs_val.to_string());
            } else {
                let mut arr: Vec<String> = Vec::new();
                let i_pos = if idx_val < 0 {
                    0
                } else {
                    (idx_val - 1).max(0) as usize
                };
                arr.resize(i_pos + 1, "0".to_string());
                arr[i_pos] = rhs_val.to_string();
                self.arrays.insert(name, arr);
            }
            return rhs_val.to_string();
        }
        let expr = self.pre_resolve_array_subscripts(&expr);
        // Output radix prefix `[#N]EXPR` (with `N#` prefix) and
        // `[##N]EXPR` (without). Direct port of zsh's math.c
        // (line 786 onward in patcompswitch's `[` case): `n=1`
        // for single-`#` (prefix kept), `n=-1` for double-`##`
        // (prefix dropped). The base must be 2..=36. Strip the
        // prefix from `expr`, store the radix for post-eval
        // formatting, then continue with the inner expression.
        let mut output_radix: Option<(u32, bool)> = None;
        let mut output_underscore: Option<u32> = None;
        let expr = {
            // Direct port of zsh src/zsh/Src/math.c:786-833. Handles:
            //   [N]NUM       (base-N literal, processed elsewhere)
            //   [#N]EXPR     (output radix N, prefixed `N#`)
            //   [##N]EXPR    (output radix N, no prefix)
            //   [#N_M]EXPR   (output radix N, group every M digits with `_`)
            //   [##N_]EXPR   (output radix, group default 3 digits)
            // Allow leading whitespace before `[#`; trim again after `]`.
            let mut e = expr.as_str().trim_start();
            if let Some(rest) = e.strip_prefix("[#") {
                let (no_prefix_form, body) = if let Some(r2) = rest.strip_prefix('#') {
                    (true, r2)
                } else {
                    (false, rest)
                };
                if let Some(close_idx) = body.find(']') {
                    // Split radix and optional `_GROUP` per math.c:810-815:
                    //   if (*ptr == '_') { ptr++; if (idigit(*ptr))
                    //     outputunderscore=zstrtol(ptr,...); else outputunderscore=3; }
                    let inside = &body[..close_idx];
                    let (n_str, under_part) = match inside.find('_') {
                        Some(p) => (&inside[..p], Some(&inside[p + 1..])),
                        None => (inside, None),
                    };
                    if let Ok(n) = n_str.parse::<u32>() {
                        if (2..=36).contains(&n) {
                            output_radix = Some((n, no_prefix_form));
                            // Underscore digit-group size. Empty
                            // suffix means default 3 (matches zsh's
                            // `else outputunderscore = 3`).
                            output_underscore = under_part.map(|s| {
                                if s.is_empty() {
                                    3
                                } else {
                                    s.parse::<u32>().unwrap_or(3)
                                }
                            });
                            e = body[close_idx + 1..].trim_start();
                        }
                    }
                }
            }
            e.to_string()
        };
        let force_float = self.options.get("forcefloat").copied().unwrap_or(false);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        // Pre-resolve dynamic special parameters that aren't in the
        // variables map: $RANDOM, $SECONDS, $EPOCHSECONDS,
        // $EPOCHREALTIME, $LINENO, $PPID, $UID, $EUID, $GID, $EGID.
        // MathState looks up names in a static HashMap, so without
        // substitution these would resolve to 0. Inject the current
        // value into a fresh extras HashMap.
        let mut extras = self.variables.clone();
        for special in [
            "RANDOM",
            "SECONDS",
            "EPOCHSECONDS",
            "EPOCHREALTIME",
            "LINENO",
            "PPID",
            "UID",
            "EUID",
            "GID",
            "EGID",
        ] {
            if !extras.contains_key(special) || special == "RANDOM" {
                let v = self.get_variable(special);
                extras.insert(special.to_string(), v);
            }
        }
        new(&expr);
        with_string_variables(&extras);
        with_force_float(force_float);
        with_c_precedences(c_prec);
        with_octal_zeroes(octal);

        match mathevall() {
            Ok(result) => {
                for (k, v) in extract_string_variables() {
                    let formatted = self.format_for_var_attr(&k, &v);
                    // Only mirror to env when the variable is
                    // explicitly exported (typeset -x or env::var
                    // already has it from a prior export). zshrs
                    // previously env::set_var-d every arith write-
                    // back, which leaked `local -i x=0; ((x=5))`
                    // values into the process env and survived the
                    // fn-exit local_save_stack unwind — variables
                    // got restored but env::var() lookup-fallback
                    // still saw the leaked value, so `${x:-unset}`
                    // post-fn returned the stale leaked value.
                    let is_exported = self
                        .var_attrs
                        .get(&k)
                        .map(|a| a.export)
                        .unwrap_or(false);
                    self.variables.insert(k.clone(), formatted.clone());
                    if is_exported {
                        env::set_var(&k, &formatted);
                    }
                }
                // If the expression had a `[#N]` / `[##N]` prefix,
                // format the integer result in base N. zsh's
                // single-`#` form prefixes `N#`; double-`##` drops
                // the prefix (math.c: `outputradix < 0` means
                // no-prefix). Floats fall back to the default %g
                // format (zsh: same thing — radix only affects
                // integer results).
                if let Some((base, no_prefix)) = output_radix {
                    let n = result.to_int();
                    // Direct port of convbase_underscore at
                    // Src/params.c:5645 — handles `[#N_M]` underscore
                    // grouping (no-op when group is None / 0).
                    let body = crate::ported::params::convbase_underscore(
                        n,
                        base,
                        output_underscore.map(|g| g as i32).unwrap_or(0),
                    );
                    // Direct port of convbase_ptr at
                    // src/zsh/Src/params.c:5596-5604:
                    //   isset(CBASES) && base == 16              → "0x"
                    //   isset(CBASES) && base == 8 && OCTALZEROES → "0"
                    //   base != 10                                → "N#"
                    //   else                                      → ""
                    // The double-`##` form (`[##N]`) drops the prefix
                    // entirely (math.c outputradix < 0 → params.c:5606
                    // takes the else branch with negated base, no prefix).
                    let cbases = self.options.get("cbases").copied().unwrap_or(false);
                    let octalzeroes = self.options.get("octalzeroes").copied().unwrap_or(false);
                    // body currently has "N#DIGITS" (or "-N#DIGITS").
                    // Strip the "N#" so we can prepend whichever prefix
                    // the option-set demands.
                    let (sign, raw_digits) = if let Some(stripped) = body.strip_prefix('-') {
                        ("-", stripped)
                    } else {
                        ("", body.as_str())
                    };
                    let digits = match raw_digits.find('#') {
                        Some(idx) => &raw_digits[idx + 1..],
                        None => raw_digits,
                    };
                    let prefix = if no_prefix {
                        ""
                    } else if cbases && base == 16 {
                        "0x"
                    } else if cbases && base == 8 && octalzeroes {
                        "0"
                    } else if base != 10 {
                        // Will format below with `N#` prefix.
                        return format!("{}{}#{}", sign, base, digits);
                    } else {
                        ""
                    };
                    return format!("{}{}{}", sign, prefix, digits);
                }
                // zsh splits formatting between the two contexts that
                // share this code path:
                //   - `$(())` arithmetic substitution → `%g`-ish: 4.0
                //     prints as "4." (zsh quirk — keeps the dot to
                //     mark "this is float", drops trailing zeros)
                //   - storage from `let`/`(( a=… ))` → `%.10f`
                // extract_string_variables (storage) already uses
                // %.10f via format_zsh; here for the substitution
                // return value emulate zsh's %g style.
                result.format_zsh_subst()
            }
            Err(msg) => {
                // zsh writes arith errors to stderr in `zsh:LINE: <msg>`
                // form. Status conventions differ by context but both
                // paths call this method — emit the diagnostic and
                // return "0"; the calling site decides whether to abort
                // (substitution: zsh aborts the whole command) or
                // continue (arith command: status 1-or-2 from the
                // StrEq-to-"0" check). Avoid touching `last_status`
                // here — the SetStatus op emitted by callers wins
                // anyway, AND a stray `last_status=2` clobbers the
                // status of unrelated paths that share evaluate_arith
                // (e.g. `a+=y` where the value parses as a non-arith
                // string then errors silently).
                zerr(&format!("{}", msg));
                // zsh aborts the surrounding command on arith
                // errors — `echo $((2#5))` emits the diagnostic
                // but does NOT print `0`. Match common error
                // shapes — "bad math expression" is the canonical
                // give-up signal; "invalid base" is a separate
                // diagnostic from numeric base parsing. Without
                // this, zshrs printed the diagnostic THEN the
                // bogus `0` value.
                if msg.starts_with("bad math expression") || msg.starts_with("invalid base") {
                    std::process::exit(1);
                }
                // NOTE: NOT aborting on "division by zero" — `((1/0))`
                // arith COMMAND continues with non-zero status (zsh
                // sets 2). Only `$((1/0))` substitution should abort,
                // but both share this evaluator and we lack a context
                // signal to distinguish. Keeping continue-with-"0"
                // for now; substitution callers see the diagnostic.
                "0".to_string()
            }
        }
    }
    pub(crate) fn eval_arith_expr(&mut self, expr: &str) -> i64 {
        let expr_expanded = if expr.contains('$') || expr.contains('`') {
            self.singsub(expr)
        } else {
            expr.to_string()
        };
        // Subscripted-array arith assignment: `((a[i]=expr))`. The
        // pre_resolve_array_subscripts pass below substitutes a[i]
        // with the current value (e.g. 0=42 → invalid). Detect the
        // assignment LHS first, evaluate the RHS, write to arrays.
        if let Some((name, idx_expr, rhs)) = parse_assign(&expr_expanded) {
            // Evaluate the index (could itself be an expression).
            let idx_val = self.eval_arith_expr(&idx_expr);
            // Evaluate the RHS.
            let rhs_val = self.eval_arith_expr(&rhs);
            // Write back: arrays for numeric idx, assoc otherwise.
            if let Some(arr) = self.arrays.get_mut(&name) {
                let i_pos = if idx_val < 0 {
                    arr.len() as i64 + idx_val
                } else {
                    idx_val - 1
                };
                if i_pos >= 0 {
                    let pos = i_pos as usize;
                    if pos >= arr.len() {
                        arr.resize(pos + 1, "0".to_string());
                    }
                    arr[pos] = rhs_val.to_string();
                }
            } else if let Some(map) = self.assoc_arrays.get_mut(&name) {
                map.insert(idx_val.to_string(), rhs_val.to_string());
            } else {
                // Auto-create indexed array.
                let mut arr: Vec<String> = Vec::new();
                let i_pos = if idx_val < 0 {
                    0
                } else {
                    (idx_val - 1).max(0) as usize
                };
                arr.resize(i_pos + 1, "0".to_string());
                arr[i_pos] = rhs_val.to_string();
                self.arrays.insert(name, arr);
            }
            return rhs_val;
        }
        let expr_expanded = self.pre_resolve_array_subscripts(&expr_expanded);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        new(&expr_expanded);
        with_string_variables(&self.variables);
        with_c_precedences(c_prec);
        with_octal_zeroes(octal);

        match mathevall() {
            Ok(result) => {
                for (k, v) in extract_string_variables() {
                    let formatted = self.format_for_var_attr(&k, &v);
                    // Only mirror to env when the variable is
                    // explicitly exported (typeset -x or env::var
                    // already has it from a prior export). zshrs
                    // previously env::set_var-d every arith write-
                    // back, which leaked `local -i x=0; ((x=5))`
                    // values into the process env and survived the
                    // fn-exit local_save_stack unwind — variables
                    // got restored but env::var() lookup-fallback
                    // still saw the leaked value, so `${x:-unset}`
                    // post-fn returned the stale leaked value.
                    let is_exported = self
                        .var_attrs
                        .get(&k)
                        .map(|a| a.export)
                        .unwrap_or(false);
                    self.variables.insert(k.clone(), formatted.clone());
                    if is_exported {
                        env::set_var(&k, &formatted);
                    }
                }
                result.to_int()
            }
            Err(msg) => {
                // zsh writes arith errors (div-by-zero, bad expr, etc.) to
                // stderr in the form `zshrs:LINE: <message>`. Without this
                // gate, `$((10/0))` returned 0 silently — masking real bugs
                // in user scripts.
                zerr(&format!("{}", msg));
                0
            }
        }
    }
    pub(crate) fn eval_arith_expr_float(&mut self, expr: &str) -> f64 {
        let expr_expanded = if expr.contains('$') || expr.contains('`') {
            self.singsub(expr)
        } else {
            expr.to_string()
        };
        let expr_expanded = self.pre_resolve_array_subscripts(&expr_expanded);
        let force_float = self.options.get("forcefloat").copied().unwrap_or(false);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        new(&expr_expanded);
        with_string_variables(&self.variables);
        with_force_float(force_float);
        with_c_precedences(c_prec);
        with_octal_zeroes(octal);

        match mathevall() {
            Ok(result) => {
                for (k, v) in extract_string_variables() {
                    let formatted = self.format_for_var_attr(&k, &v);
                    // Only mirror to env when the variable is
                    // explicitly exported (typeset -x or env::var
                    // already has it from a prior export). zshrs
                    // previously env::set_var-d every arith write-
                    // back, which leaked `local -i x=0; ((x=5))`
                    // values into the process env and survived the
                    // fn-exit local_save_stack unwind — variables
                    // got restored but env::var() lookup-fallback
                    // still saw the leaked value, so `${x:-unset}`
                    // post-fn returned the stale leaked value.
                    let is_exported = self
                        .var_attrs
                        .get(&k)
                        .map(|a| a.export)
                        .unwrap_or(false);
                    self.variables.insert(k.clone(), formatted.clone());
                    if is_exported {
                        env::set_var(&k, &formatted);
                    }
                }
                result.to_float()
            }
            Err(_) => 0.0,
        }
    }
    pub(crate) fn evaluate_arithmetic_expr(&mut self, expr: &str) -> i64 {
        self.eval_arith_expr(expr)
    }
    /// Execute arithmetic expression
    /// Port of execarith() from exec.c
    pub fn execarith(&mut self, expr: &str) -> i32 {
        let result = self.eval_arith_expr(expr);
        if result == 0 {
            1
        } else {
            0
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/subst.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
}

// =====================================================================
// MOVED FROM: src/ported/jobs.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    pub(crate) fn builtin_jobs(&mut self, args: &[String]) -> i32 {
        // jobs [ -dlprsZ ] [ job ... ]
        // -l: long format (show PID)
        // -p: print process group IDs only
        // -d: show directory from which job was started
        // -r: show running jobs only
        // -s: show stopped jobs only
        // -Z: set process name (not relevant here)

        let mut long_format = false;
        let mut pids_only = false;
        let mut show_dir = false;
        let mut running_only = false;
        let mut stopped_only = false;
        let mut job_ids: Vec<usize> = Vec::new();

        for arg in args {
            if let Some(after) = arg.strip_prefix('-') {
                for c in after.chars() {
                    match c {
                        'l' => long_format = true,
                        'p' => pids_only = true,
                        'd' => show_dir = true,
                        'r' => running_only = true,
                        's' => stopped_only = true,
                        // zsh: `jobs -Z` requires a process-name
                        // argument (it sets the shell's process name
                        // to that string). Without one, it errors
                        // `jobs:1: -Z requires one argument` exit 1.
                        // zshrs silently ignored `-Z` entirely.
                        'Z' => {
                            zwarnnam("jobs", "-Z requires one argument");
                            return 1;
                        }
                        // BUILTIN("jobs", ..., "dlpZrs") — only six
                        // letters are valid. zshrs's `_ => {}`
                        // accepted any letter silently so `jobs -X`
                        // would print all jobs as if -X were a no-op.
                        _ => {
                            zwarnnam("jobs", &format!("bad option: -{}", c));
                            return 1;
                        }
                    }
                }
            } else if let Some(after_pct) = arg.strip_prefix('%') {
                if let Ok(id) = after_pct.parse::<usize>() {
                    job_ids.push(id);
                }
            } else if let Ok(id) = arg.parse::<usize>() {
                job_ids.push(id);
            }
        }

        // Reap finished jobs first
        for job in self.jobs.reap_finished() {
            if !running_only && !stopped_only {
                if pids_only {
                    println!("{}", job.pid);
                } else {
                    println!("[{}]  Done                    {}", job.id, job.command);
                }
            }
        }

        // zsh: `jobs %N` for an N that doesn't exist errors
        // `jobs:1: %N: no such job` exit 1. zshrs's filter-by-id
        // loop silently produced no output. Validate the requested
        // ids against the current job list before listing.
        if !job_ids.is_empty() {
            for &requested in &job_ids {
                if !self.jobs.list().iter().any(|j| j.id == requested) {
                    zwarnnam("jobs", &format!("%{}: no such job", requested));
                    return 1;
                }
            }
        }

        // List jobs (optionally filtered)
        for job in self.jobs.list() {
            // Filter by specific job IDs if provided
            if !job_ids.is_empty() && !job_ids.contains(&job.id) {
                continue;
            }

            // Filter by state
            if running_only && job.state != JobState::Running {
                continue;
            }
            if stopped_only && job.state != JobState::Stopped {
                continue;
            }

            if pids_only {
                println!("{}", job.pid);
                continue;
            }

            let marker = if job.is_current { "+" } else { "-" };
            let state = match job.state {
                JobState::Running => "running",
                JobState::Stopped => "suspended",
                JobState::Done => "done",
            };

            if long_format {
                println!(
                    "[{}]{} {:6} {}  {}",
                    job.id, marker, job.pid, state, job.command
                );
            } else {
                println!("[{}]{} {}  {}", job.id, marker, state, job.command);
            }

            if show_dir {
                // jobs -d: print the directory the job was started in.
                // We don't yet capture per-job cwd at launch (would
                // need a JobInfo.cwd field plumbed through add_job),
                // so use logical $PWD as a best-effort proxy. Same
                // proxy that ${jobdirs[N]} uses, so the two views
                // agree. Direct port of zsh/Src/jobs.c printjob's
                // `pwd: %s` line when SHOWDIR is set.
                let pwd = self
                    .variables
                    .get("PWD")
                    .cloned()
                    .or_else(|| env::var("PWD").ok())
                    .unwrap_or_else(|| {
                        env::current_dir()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    });
                println!("    (pwd: {})", pwd);
            }
        }
        0
    }
    pub(crate) fn bin_fg(&mut self, args: &[String]) -> i32 {
        // zsh in `-c` mode has no real job-control regardless of the
        // `monitor` option. zsh `fg %N` always errors `fg:1: no job
        // control in this shell.` in this context. zshrs's options
        // table reports `interactive=true` and `monitor=true` even
        // in `-c` mode, so option-based checks don't work. Use the
        // stdin-tty status: a real interactive shell has a tty on
        // stdin; `-c` mode does not (stdin is piped or empty).
        if !atty::is(atty::Stream::Stdin) {
            zwarnnam("fg", "no job control in this shell.");
            return 1;
        }
        let job_id = if let Some(arg) = args.first() {
            // Parse %N or just N
            let s = arg.trim_start_matches('%');
            match s.parse::<usize>() {
                Ok(id) => Some(id),
                Err(_) => {
                    zwarnnam("fg", &format!("{}: no such job", arg));
                    return 1;
                }
            }
        } else {
            self.jobs.current().map(|j| j.id)
        };

        let Some(id) = job_id else {
            // Match zsh's diagnostic for non-interactive contexts.
            zwarnnam("fg", "no job control in this shell.");
            return 1;
        };

        let Some(job) = self.jobs.get(id) else {
            zwarnnam("fg", &format!("%{}: no such job", id));
            return 1;
        };

        let pid = job.pid;
        let cmd = job.command.clone();
        println!("{}", cmd);

        // Continue the job
        if let Err(e) = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGCONT).map_err(|e| e.to_string()) {
            zwarnnam("fg", &format!("{}", e));
            return 1;
        }

        // Wait for it
        match {
            // Inline wait_for_job — port of jobs.c::update_job's
            // waitpid loop (Src/jobs.c:460).
            use nix::sys::wait::{waitpid, WaitStatus};
            use nix::unistd::Pid;
            let result: Result<i32, String>;
            loop {
                result = match waitpid(Pid::from_raw(pid), None) {
                    Ok(WaitStatus::Exited(_, code)) => Ok(code),
                    Ok(WaitStatus::Signaled(_, sig, _)) => Ok(128 + sig as i32),
                    Ok(WaitStatus::Stopped(_, _)) => Ok(128),
                    Ok(_) => continue,
                    Err(nix::errno::Errno::ECHILD) => Ok(0),
                    Err(e) => Err(e.to_string()),
                };
                break;
            }
            result
        } {
            Ok(status) => {
                self.jobs.remove(id);
                status
            }
            Err(e) => {
                zwarnnam("fg", &format!("{}", e));
                1
            }
        }
    }
    pub(crate) fn builtin_bg(&mut self, args: &[String]) -> i32 {
        // Same no-job-control semantics as `fg` — see comment there.
        if !atty::is(atty::Stream::Stdin) {
            zwarnnam("bg", "no job control in this shell.");
            return 1;
        }
        let job_id = if let Some(arg) = args.first() {
            let s = arg.trim_start_matches('%');
            match s.parse::<usize>() {
                Ok(id) => Some(id),
                Err(_) => {
                    zwarnnam("bg", &format!("{}: no such job", arg));
                    return 1;
                }
            }
        } else {
            self.jobs.current().map(|j| j.id)
        };

        let Some(id) = job_id else {
            zwarnnam("bg", "no job control in this shell.");
            return 1;
        };

        let Some(job) = self.jobs.get_mut(id) else {
            zwarnnam("bg", &format!("%{}: no such job", id));
            return 1;
        };

        let pid = job.pid;
        let cmd = job.command.clone();

        if let Err(e) = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGCONT).map_err(|e| e.to_string()) {
            zwarnnam("bg", &format!("{}", e));
            return 1;
        }

        job.state = JobState::Running;
        println!("[{}] {} &", id, cmd);
        0
    }
    pub(crate) fn bin_kill(&mut self, args: &[String]) -> i32 {
        // kill [ -s signal_name | -n signal_number | -sig ] job ...
        // kill -l [ sig ... ]
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        if args.is_empty() {
            // zsh: bare `kill` -> `kill:1: not enough arguments` exit 1.
            // zshrs printed a multi-line bash-style usage banner that
            // didn't match zsh's terse format.
            zwarnnam("kill", "not enough arguments");
            return 1;
        }

        // Signal name/number mapping. Numbers are pulled from libc
        // so they're platform-correct: macOS USR1=30, Linux USR1=10.
        // Hardcoding caused `kill -l USR1` to print 10 on macOS.
        let signal_map: &[(&str, i32, Signal)] = &[
            ("HUP", libc::SIGHUP, Signal::SIGHUP),
            ("INT", libc::SIGINT, Signal::SIGINT),
            ("QUIT", libc::SIGQUIT, Signal::SIGQUIT),
            ("ILL", libc::SIGILL, Signal::SIGILL),
            ("TRAP", libc::SIGTRAP, Signal::SIGTRAP),
            ("ABRT", libc::SIGABRT, Signal::SIGABRT),
            #[cfg(target_os = "macos")]
            ("EMT", libc::SIGEMT, Signal::SIGEMT),
            ("BUS", libc::SIGBUS, Signal::SIGBUS),
            ("FPE", libc::SIGFPE, Signal::SIGFPE),
            ("KILL", libc::SIGKILL, Signal::SIGKILL),
            ("USR1", libc::SIGUSR1, Signal::SIGUSR1),
            ("SEGV", libc::SIGSEGV, Signal::SIGSEGV),
            ("USR2", libc::SIGUSR2, Signal::SIGUSR2),
            ("PIPE", libc::SIGPIPE, Signal::SIGPIPE),
            ("ALRM", libc::SIGALRM, Signal::SIGALRM),
            ("TERM", libc::SIGTERM, Signal::SIGTERM),
            ("CHLD", libc::SIGCHLD, Signal::SIGCHLD),
            ("CONT", libc::SIGCONT, Signal::SIGCONT),
            ("STOP", libc::SIGSTOP, Signal::SIGSTOP),
            ("TSTP", libc::SIGTSTP, Signal::SIGTSTP),
            ("TTIN", libc::SIGTTIN, Signal::SIGTTIN),
            ("TTOU", libc::SIGTTOU, Signal::SIGTTOU),
            ("URG", libc::SIGURG, Signal::SIGURG),
            ("XCPU", libc::SIGXCPU, Signal::SIGXCPU),
            ("XFSZ", libc::SIGXFSZ, Signal::SIGXFSZ),
            ("VTALRM", libc::SIGVTALRM, Signal::SIGVTALRM),
            ("PROF", libc::SIGPROF, Signal::SIGPROF),
            ("WINCH", libc::SIGWINCH, Signal::SIGWINCH),
            ("IO", libc::SIGIO, Signal::SIGIO),
            ("SYS", libc::SIGSYS, Signal::SIGSYS),
            // macOS-only SIGINFO (29). zsh's `kill -l` lists it
            // between WINCH and USR1; without this entry zshrs
            // skipped INFO and the listing didn't match.
            #[cfg(target_os = "macos")]
            ("INFO", libc::SIGINFO, Signal::SIGINFO),
        ];

        let mut sig = Signal::SIGTERM;
        let mut signal_zero = false;
        let mut pids: Vec<String> = Vec::new();
        let mut list_mode = false;
        let mut list_args: Vec<String> = Vec::new();

        let mut i = 0;
        let mut after_dashdash = false;
        while i < args.len() {
            let arg = &args[i];

            // `--` is end-of-options; subsequent args are PIDs.
            // zsh `kill -- PID` correctly sends SIGTERM. zshrs's
            // catch-all `arg.starts_with('-') && arg.len() > 1`
            // treated `--` as a signal name (`-` -> "L", missing).
            if arg == "--" && !after_dashdash {
                after_dashdash = true;
                i += 1;
                continue;
            }
            if after_dashdash {
                pids.push(arg.clone());
                i += 1;
                continue;
            }

            if arg == "-l" {
                list_mode = true;
                // Remaining args are signal numbers to translate
                list_args = args[i + 1..].to_vec();
                break;
            } else if arg == "-s" {
                // -s signal_name (or numeric signal-by-name)
                i += 1;
                if i >= args.len() {
                    zwarnnam("kill", "-s requires an argument");
                    return 1;
                }
                // zsh: empty signal name -> `kill:1: -: signal name
                // expected`. zshrs's name lookup of "" produced
                // "invalid signal:  " (with empty trailing).
                if args[i].is_empty() {
                    zwarnnam("kill", "-: signal name expected");
                    return 1;
                }
                // zsh accepts numeric values to `-s` too — `-s 0`
                // is the existence-check form. zshrs's name-only
                // lookup rejected `0` as an invalid signal.
                if args[i] == "0" {
                    signal_zero = true;
                } else if let Ok(num) = args[i].parse::<i32>() {
                    if let Some((_, _, s)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                        sig = *s;
                    } else {
                        zwarnnam("kill", &format!("invalid signal: {}", args[i]));
                        return 1;
                    }
                } else {
                    let sig_name = args[i].to_uppercase();
                    let sig_name = sig_name.strip_prefix("SIG").unwrap_or(&sig_name);
                    if let Some((_, _, s)) =
                        signal_map.iter().find(|(name, _, _)| *name == sig_name)
                    {
                        sig = *s;
                    } else {
                        zwarnnam("kill", &format!("invalid signal: {}", args[i]));
                        return 1;
                    }
                }
            } else if arg == "-n" {
                // -n signal_number
                i += 1;
                if i >= args.len() {
                    zwarnnam("kill", "-n requires an argument");
                    return 1;
                }
                let num: i32 = match args[i].parse() {
                    Ok(n) => n,
                    Err(_) => {
                        zwarnnam("kill", &format!("invalid signal number: {}", args[i]));
                        return 1;
                    }
                };
                if let Some((_, _, s)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                    sig = *s;
                } else {
                    zwarnnam("kill", &format!("invalid signal number: {}", num));
                    return 1;
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                // -SIGNAL or -NUM
                let sig_str = &arg[1..];
                let sig_upper = sig_str.to_uppercase();
                let sig_name = sig_upper.strip_prefix("SIG").unwrap_or(&sig_upper);

                // Try as number first
                if let Ok(num) = sig_str.parse::<i32>() {
                    // Signal 0: special "process existence check" — no
                    // signal sent, but kill(pid, 0) returns 0 if pid is
                    // alive, errno ESRCH if not. Mark with a sentinel
                    // (SIGUSR1 + override flag) handled below.
                    if num == 0 {
                        signal_zero = true;
                    } else if let Some((_, _, s)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                        sig = *s;
                    } else {
                        zwarnnam("kill", &format!("invalid signal: {}", arg));
                        return 1;
                    }
                } else if let Some((_, _, s)) =
                    signal_map.iter().find(|(name, _, _)| *name == sig_name)
                {
                    sig = *s;
                } else {
                    // zsh: `unknown signal: SIGFOO` followed by a hint
                    // line `type kill -l for a list of signals`. zshrs
                    // emitted the bash-style `kill: invalid signal:
                    // -FOO` (with the leading dash, no SIG prefix).
                    zwarnnam("kill", &format!("unknown signal: SIG{}", sig_name));
                    zwarnnam("kill", "type kill -l for a list of signals");
                    return 1;
                }
            } else {
                pids.push(arg.clone());
            }
            i += 1;
        }

        // Handle -l (list signals)
        if list_mode {
            if list_args.is_empty() {
                // zsh prints bare signal names separated by spaces on
                // a single line for `kill -l`, ordered by SIGNAL
                // NUMBER (not declaration order). Sort by num so
                // macOS shows HUP INT QUIT ILL TRAP ABRT EMT FPE
                // KILL BUS SEGV SYS PIPE ALRM TERM URG STOP TSTP …
                // matching `/bin/zsh -f -c 'kill -l'`.
                let mut by_num: Vec<&(&str, i32, _)> = signal_map.iter().collect();
                by_num.sort_by_key(|(_, n, _)| *n);
                let names: Vec<String> = by_num.iter().map(|(n, _, _)| (*n).to_string()).collect();
                println!("{}", names.join(" "));
            } else {
                // Translate signal numbers to names or vice versa
                for arg in &list_args {
                    if let Ok(num) = arg.parse::<i32>() {
                        // Number -> name. zsh passes through unknown
                        // numbers (`kill -l 100` → `100`) instead of
                        // erroring — matches POSIX-ish behavior.
                        if let Some((name, _, _)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                            println!("{}", name);
                        } else {
                            println!("{}", num);
                        }
                    } else {
                        // Name -> number
                        // Strip leading `-` in addition to SIG prefix
                        // — `kill -l -X` should report `unknown
                        // signal: SIGX`, not `SIG-X`.
                        let sig_upper = arg.trim_start_matches('-').to_uppercase();
                        let sig_name = sig_upper.strip_prefix("SIG").unwrap_or(&sig_upper);
                        if let Some((_, num, _)) =
                            signal_map.iter().find(|(name, _, _)| *name == sig_name)
                        {
                            println!("{}", num);
                        } else {
                            // zsh's diagnostic always uses the SIG prefix
                            // even when the user's input lacked it:
                            // `kill -l XYZ` → `unknown signal: SIGXYZ`.
                            zwarnnam("kill", &format!("unknown signal: SIG{}", sig_name));
                        }
                    }
                }
            }
            return 0;
        }

        if pids.is_empty() {
            // zsh: `kill -9` (signal but no pid) -> `kill:1: not enough
            // arguments` exit 1. Match the same terse format used for
            // bare `kill`.
            zwarnnam("kill", "not enough arguments");
            return 1;
        }

        let mut status = 0;
        for arg in &pids {
            // Handle %job syntax
            if let Some(spec) = arg.strip_prefix('%') {
                let id: usize = match spec.parse() {
                    Ok(id) => id,
                    Err(_) => {
                        // zsh format: `kill:1: job not found:
                        // <name-without-%>`. zshrs's `%abc: no such
                        // job` had the % AND wrong wording.
                        zwarnnam("kill", &format!("job not found: {}", spec));
                        status = 1;
                        continue;
                    }
                };
                if let Some(job) = self.jobs.get(id) {
                    if let Err(e) = kill(Pid::from_raw(job.pid), sig) {
                        zwarnnam("kill", &format!("{}", e));
                        status = 1;
                    }
                } else {
                    zwarnnam("kill", &format!("{}: no such job", arg));
                    status = 1;
                }
            } else {
                // Direct PID
                let pid: u32 = match arg.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        // zsh: `kill -0 abc` -> `kill:1: illegal pid:
                        // abc` exit 1. zshrs's bash-style `kill: abc:
                        // invalid pid` had no shell-name prefix.
                        zwarnnam("kill", &format!("illegal pid: {}", arg));
                        status = 1;
                        continue;
                    }
                };
                if signal_zero {
                    // `kill -0 PID` — process existence check. POSIX
                    // doesn't define a Signal::SIG0 enum variant; call
                    // libc::kill(pid, 0) directly.
                    let rc = unsafe { libc::kill(pid as i32, 0) };
                    if rc != 0 {
                        // zsh format: `kill:1: kill PID failed:
                        // <reason>` with the OS error message
                        // lowercased and the `(os error N)` suffix
                        // stripped. zshrs's `{}: {}` form was
                        // bash-style.
                        let err = std::io::Error::last_os_error();
                        let raw = err.to_string();
                        let cleaned = raw
                            .split(" (os error")
                            .next()
                            .unwrap_or(&raw)
                            .to_lowercase();
                        zwarnnam("kill", &format!("kill {} failed: {}", pid, cleaned));
                        status = 1;
                    }
                } else if let Err(e) = kill(Pid::from_raw(pid as i32), sig) {
                    // zsh format: `kill:1: kill PID failed: <reason>`
                    // with the OS error message lowercased and the
                    // `(os error N)` suffix stripped. zshrs's `kill:
                    // ESRCH: ...` printed the errno code verbatim.
                    let raw = e.to_string();
                    let cleaned = raw
                        .split(':')
                        .next_back()
                        .unwrap_or(&raw)
                        .trim()
                        .to_lowercase();
                    zwarnnam("kill", &format!("kill {} failed: {}", pid, cleaned));
                    status = 1;
                }
            }
        }
        status
    }
    pub(crate) fn builtin_disown(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // Disown current job — but if there isn't one, zsh emits
            // `no current job` exit 1. zshrs returned 0 silently,
            // hiding the no-current-job condition.
            if let Some(job) = self.jobs.current() {
                let id = job.id;
                self.jobs.remove(id);
                return 0;
            }
            zwarnnam("disown", "no current job");
            return 1;
        }

        let mut status = 0;
        for arg in args {
            // zsh: `-l`, `-h`, etc. are NOT recognized disown flags
            // — they're treated as job specs and error `job not
            // found: -l`. zshrs's flagless impl emitted `disown: -l:
            // no such job`. Use zsh's "<shell>:disown:1: job not
            // found:" form for non-`%`-prefixed unparseable input.
            // For `%N`-prefixed, the existing %-stripped path
            // applies; no-such-job uses `%N: no such job`.
            if arg.starts_with('%') {
                let s = arg.trim_start_matches('%');
                if let Ok(id) = s.parse::<usize>() {
                    if self.jobs.remove(id).is_none() {
                        zwarnnam("disown", &format!("{}: no such job", arg));
                        status = 1;
                    }
                } else {
                    zwarnnam("disown", &format!("{}: no such job", arg));
                    status = 1;
                }
            } else if let Ok(id) = arg.parse::<usize>() {
                if self.jobs.remove(id).is_none() {
                    zwarnnam("disown", &format!("%{}: no such job", id));
                    status = 1;
                }
            } else {
                zwarnnam("disown", &format!("job not found: {}", arg));
                status = 1;
            }
        }
        status
    }
    pub(crate) fn builtin_wait(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // Wait for all jobs. Two job-entry shapes coexist:
            //   - Spawned via `Command::spawn` → has Child, use child.wait()
            //   - Forked via raw libc::fork (BUILTIN_RUN_BG) → child=None,
            //     use waitpid(pid) per Src/jobs.c::update_job loop.
            let entries: Vec<(usize, i32, bool)> = self
                .jobs
                .list()
                .iter()
                .map(|j| (j.id, j.pid, j.child.is_some()))
                .collect();
            for (id, pid, has_child) in entries {
                if let Some(mut job) = self.jobs.remove(id) {
                    if has_child {
                        if let Some(ref mut child) = job.child {
                            let _ = child.wait();
                        }
                    } else if pid > 0 {
                        #[cfg(unix)]
                        {
                            use nix::sys::wait::{waitpid, WaitStatus};
                            use nix::unistd::Pid;
                            loop {
                                match waitpid(Pid::from_raw(pid), None) {
                                    Ok(WaitStatus::Exited(_, _))
                                    | Ok(WaitStatus::Signaled(_, _, _))
                                    | Ok(WaitStatus::Stopped(_, _)) => break,
                                    Ok(_) => continue,
                                    Err(_) => break,
                                }
                            }
                        }
                        let _ = pid;
                    }
                }
            }
            return 0;
        }

        let mut status = 0;
        for arg in args {
            if let Some(spec) = arg.strip_prefix('%') {
                let id: usize = match spec.parse() {
                    Ok(id) => id,
                    Err(_) => {
                        zwarnnam("wait", &format!("{}: no such job", arg));
                        status = 127;
                        continue;
                    }
                };
                if let Some(mut job) = self.jobs.remove(id) {
                    if let Some(ref mut child) = job.child {
                        match child.wait().map(|s| s.code().unwrap_or(0)).map_err(|e| e.to_string()) {
                            Ok(s) => status = s,
                            Err(e) => {
                                zwarnnam("wait", &format!("{}", e));
                                status = 127;
                            }
                        }
                    }
                } else {
                    // Distinguish "reaped job" (silent — bg `&` path
                    // doesn't currently flow through JobTable, so once
                    // the bg child completes the wait can't find the
                    // entry) from "never-existed id" (user error).
                    // Heuristic: if the session has EVER backgrounded
                    // a job (signalled by `$!` being set to a real
                    // pid), accept missing %1 silently — the bg/wait
                    // idiom relies on it. Otherwise error like zsh.
                    let bg_was_used = self
                        .variables
                        .get("!")
                        .and_then(|s| s.parse::<u32>().ok())
                        .map(|p| p > 0)
                        .unwrap_or(false);
                    if !bg_was_used {
                        zwarnnam("wait", &format!("{}: no such job", arg));
                        status = 127;
                    }
                    // else: silent success (a bg job was started; we
                    // can't tell if THIS specific id was the right one
                    // without job-table integration in BUILTIN_RUN_BG).
                }
            } else if arg.is_empty() {
                // zsh: `wait ""` (literal empty arg) -> `wait:1: job
                // not found: ` exit 127. zshrs silently continued,
                // masking the bad input. NOTE: `wait $!` with no bg
                // job started doesn't reach this arm because $!
                // defaults to "0" (the literal pid value), not "".
                zwarnnam("wait", "job not found: ");
                status = 127;
                continue;
            } else {
                let pid: u32 = match arg.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        // zsh: stops processing remaining args after
                        // the first non-PID. zshrs's `continue`
                        // emitted one error per bad arg, exceeding
                        // zsh's diagnostic count for `wait abc def`.
                        zwarnnam("wait", &format!("job not found: {}", arg));
                        return 127;
                    }
                };
                // Verify the PID is one of OUR children. If we never
                // forked it, zsh emits `pid N is not a child of this
                // shell` and exits 127.
                let known = self.variables.get("!").and_then(|s| s.parse::<u32>().ok())
                    == Some(pid)
                    || self.jobs.list().iter().any(|j| j.pid == pid as i32);
                if !known {
                    zwarnnam("wait", &format!("pid {} is not a child of this shell", pid));
                    status = 127;
                    continue;
                }
                // Inline wait_for_job — port of jobs.c::update_job's
                // waitpid loop (Src/jobs.c:460).
                use nix::sys::wait::{waitpid, WaitStatus};
                use nix::unistd::Pid;
                let result: Result<i32, String> = loop {
                    break match waitpid(Pid::from_raw(pid as i32), None) {
                        Ok(WaitStatus::Exited(_, code)) => Ok(code),
                        Ok(WaitStatus::Signaled(_, sig, _)) => Ok(128 + sig as i32),
                        Ok(WaitStatus::Stopped(_, _)) => Ok(128),
                        Ok(_) => continue,
                        Err(nix::errno::Errno::ECHILD) => Ok(0),
                        Err(e) => Err(e.to_string()),
                    };
                };
                match result {
                    Ok(s) => status = s,
                    Err(e) => {
                        zwarnnam("wait", &format!("{}", e));
                        status = 127;
                    }
                }
            }
        }
        status
    }
    pub(crate) fn bin_suspend(&self, args: &[String]) -> i32 {
        let mut force = false;
        for arg in args {
            if arg == "-f" {
                force = true;
            }
        }

        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::getppid;

            // Check if we're a login shell (parent is init/PID 1)
            let ppid = getppid();
            if !force && ppid == nix::unistd::Pid::from_raw(1) {
                zwarnnam("suspend", "cannot suspend a login shell");
                return 1;
            }

            // Send SIGTSTP to ourselves
            let pid = nix::unistd::getpid();
            if let Err(e) = kill(pid, Signal::SIGTSTP) {
                zwarnnam("suspend", &format!("{}", e));
                return 1;
            }
            0
        }

        #[cfg(not(unix))]
        {
            zwarnnam("suspend", "not supported on this platform");
            1
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/glob.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Match a string against a shell glob pattern
    pub(crate) fn glob_match(&self, s: &str, pattern: &str) -> bool {
        // Convert shell glob to regex
        let mut regex_pattern = String::from("^");
        let mut chars = pattern.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '*' => regex_pattern.push_str(".*"),
                '?' => regex_pattern.push('.'),
                '[' => {
                    regex_pattern.push('[');
                    // Handle character class
                    for cc in chars.by_ref() {
                        if cc == ']' {
                            regex_pattern.push(']');
                            break;
                        }
                        regex_pattern.push(cc);
                    }
                }
                '(' => {
                    // Handle alternation (a|b|c) -> (a|b|c)
                    regex_pattern.push('(');
                }
                ')' => regex_pattern.push(')'),
                '|' => regex_pattern.push('|'),
                '.' | '+' | '^' | '$' | '\\' | '{' | '}' => {
                    regex_pattern.push('\\');
                    regex_pattern.push(c);
                }
                _ => regex_pattern.push(c),
            }
        }
        regex_pattern.push('$');

        regex::Regex::new(&regex_pattern)
            .map(|re| re.is_match(s))
            .unwrap_or(false)
    }
    /// Static glob match — same logic as glob_match but callable without &self,
    /// needed for Rayon parallel iterators that can't capture &self.
    pub fn glob_match_static(s: &str, pattern: &str) -> bool {
        // Extendedglob `^pat` negation: when extendedglob is on AND
        // the pattern starts with a literal `^`, strip it and invert
        // the match of the remainder. Already done in
        // `extendedglob_match` for the param-filter path; do it here
        // too so `[[ str = ^pat ]]` works via the cond `=` matcher.
        let extendedglob_on =
            with_executor(|e| e.options.get("extendedglob").copied().unwrap_or(false));
        if extendedglob_on {
            if let Some(rest) = pattern.strip_prefix('^') {
                return !ShellExecutor::glob_match_static(s, rest);
            }
            // Extendedglob `~` exclusion: `pat1~pat2` matches strings
            // matching `pat1` AND NOT matching `pat2`. Direct port of
            // zsh's pattern.c P_EXCLUDE handling (line 155 onward) for
            // the top-level case — the canonical implementation also
            // handles nested exclusions (`(a~b)c`) but the top-level
            // form is what `*.txt~README*` and similar idioms produce.
            // Walk the pattern looking for a `~` that's NOT inside
            // `[...]` or `(...)` so nested specials stay literal.
            if let Some(idx) = find_top_level_tilde(pattern) {
                let lhs = &pattern[..idx];
                let rhs = &pattern[idx + 1..];
                return ShellExecutor::glob_match_static(s, lhs)
                    && !ShellExecutor::glob_match_static(s, rhs);
            }
        }

        // ksh-style negation `!(p)` (gated on `setopt kshglob`): when
        // the entire pattern is `!(<body>)`, match anything that does
        // NOT match `<body>`. This handles the standalone case (the
        // overwhelmingly common form); embedded `!()` inside a larger
        // pattern still falls through and is left literal — full
        // zsh-style negation needs lookahead which `regex` lacks.
        let kshglob_on = with_executor(|e| e.options.get("kshglob").copied().unwrap_or(false));
        if kshglob_on {
            if let Some(body) = pattern.strip_prefix("!(").and_then(|r| r.strip_suffix(')')) {
                // Don't recurse if body itself contains an unmatched
                // `(` that would change the meaning.
                let mut depth = 0;
                let mut balanced = true;
                for c in body.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            if depth == 0 {
                                balanced = false;
                                break;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                }
                if balanced && depth == 0 {
                    return !ShellExecutor::glob_match_static(s, body);
                }
            }
        }

        // Inline pattern flags `(#i)` / `(#I)` / `(#l)` / `(#a<n>)` per
        // zshexpn(1) "Globbing Flags". They prefix a pattern and modify
        // matching semantics for the rest.
        //   (#i) — case insensitive
        //   (#I) — case sensitive (turn (#i) back off)
        //   (#l) — lowercase pattern char matches both cases in input;
        //          uppercase pattern char is exact-match
        //   (#a<n>) — approximate match: up to <n> errors (Levenshtein
        //          distance, insert/delete/substitute)
        let (pattern, case_insensitive, l_flag, approx_n, _) = PatternFlags::parse(pattern);

        if let Some(n) = approx_n {
            // Inline (#aN) approximate-match — direct port of the
            // Levenshtein-distance check inside patmatch (Src/pattern.c)
            // when PAT_APPROX is set. m/k bound check skips early when
            // the strings differ in length by more than the budget;
            // otherwise standard 2-row DP table.
            let s_chars: Vec<char> = s.chars().collect();
            let p_chars: Vec<char> = pattern.chars().collect();
            let m = s_chars.len();
            let k = p_chars.len();
            if m.abs_diff(k) > n {
                return false;
            }
            let mut prev: Vec<usize> = (0..=k).collect();
            let mut curr: Vec<usize> = vec![0; k + 1];
            for i in 1..=m {
                curr[0] = i;
                for j in 1..=k {
                    let cost = if s_chars[i - 1] == p_chars[j - 1] { 0 } else { 1 };
                    curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
                }
                std::mem::swap(&mut prev, &mut curr);
            }
            return prev[k] <= n;
        }

        // Build the regex. For (#l) we need to inflate lowercase chars
        // to character classes that match either case. Also detect
        // zsh's numeric-range glob `<a-b>` (or `<->` for any number,
        // `<a->` / `<-b>` for one-sided ranges) — translate to a
        // capture group and remember the bounds for a post-match check.
        let mut regex_pattern = String::from("^");
        // Numeric ranges paired with the regex capture-group index they
        // correspond to. Required because user-written `(...)` groups
        // in the pattern (esp. alternation `(a|b)`) shift capture
        // indices, so we can't assume each `<N-M>` is at numeric_ranges
        // index + 1. Direct port of the bookkeeping zsh's pattern.c
        // does via `pat_captures` — each numeric atom remembers its
        // own group offset. Without this, `[[ 5.9 == (5.<1->*|<6->.*) ]]`
        // applied the lo/hi check against the OUTER alternation's
        // capture (the literal "5.9") and parse-as-int failed.
        let mut numeric_ranges: Vec<(usize, Option<i64>, Option<i64>)> = Vec::new();
        // Track the capture-group index. Increments on every `(` that
        // OPENS a new group in the emitted regex. Starts at 0 because
        // the outer `^...$` anchors don't add a group.
        let mut capture_group_count: usize = 0;
        let mut chars = pattern.chars().peekable();
        // Helper: after emitting any atom, check for zsh extendedglob
        // postfix `#` (zero-or-more) / `##` (one-or-more) and append
        // the equivalent regex quantifier. Direct port of zsh's
        // pattern.c (`POUND` / `POUND2` cases in `patcompswitch`).
        // Only fires when extendedglob is enabled.
        let consume_extglob_postfix =
            |chars: &mut std::iter::Peekable<std::str::Chars>| -> Option<&'static str> {
                if !extendedglob_on {
                    return None;
                }
                if chars.peek() != Some(&'#') {
                    return None;
                }
                chars.next();
                if chars.peek() == Some(&'#') {
                    chars.next();
                    Some("+")
                } else {
                    Some("*")
                }
            };
        while let Some(c) = chars.next() {
            match c {
                // ksh-style extglob: ?(p) *(p) +(p) @(p) — translate to
                // (?:p)? (?:p)* (?:p)+ (?:p) respectively. Gated on
                // the `kshglob` option (zsh's default is off). The
                // !(p) (negative) form needs lookahead which the
                // `regex` crate doesn't support; left literal.
                '?' | '*' | '+' | '@'
                    if chars.peek() == Some(&'(')
                        && with_executor(|e| {
                            e.options.get("kshglob").copied().unwrap_or(false)
                        }) =>
                {
                    let op = c;
                    chars.next(); // consume '('
                                  // Capture body until matching ')'. Track depth so
                                  // nested parens work.
                    let mut depth = 1;
                    let mut body = String::new();
                    while let Some(&pc) = chars.peek() {
                        chars.next();
                        if pc == '(' {
                            depth += 1;
                            body.push(pc);
                        } else if pc == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            body.push(pc);
                        } else {
                            body.push(pc);
                        }
                    }
                    // Inline ksh-extglob body -> regex translator.
                    // Direct port of the tiny per-char dispatch zsh's
                    // pattern.c does inside its extglob handler — no
                    // anchors, no (#flags), just glob -> regex chars.
                    let body_re = {
                        let mut out = String::new();
                        let mut chars = body.chars().peekable();
                        while let Some(c) = chars.next() {
                            match c {
                                '|' => out.push('|'),
                                '*' => out.push_str(".*"),
                                '?' => out.push('.'),
                                '[' => {
                                    out.push('[');
                                    for cc in chars.by_ref() {
                                        if cc == ']' {
                                            out.push(']');
                                            break;
                                        }
                                        out.push(cc);
                                    }
                                }
                                '.' | '+' | '^' | '$' | '\\' | '{' | '}' | '(' | ')' => {
                                    out.push('\\');
                                    out.push(c);
                                }
                                _ => out.push(c),
                            }
                        }
                        out
                    };
                    let suffix = match op {
                        '?' => "?",
                        '*' => "*",
                        '+' => "+",
                        '@' => "",
                        _ => "",
                    };
                    regex_pattern.push_str(&format!("(?:{}){}", body_re, suffix));
                }
                '*' => regex_pattern.push_str(".*"),
                '?' => {
                    regex_pattern.push('.');
                    if let Some(q) = consume_extglob_postfix(&mut chars) {
                        regex_pattern.push_str(q);
                    }
                }
                '<' => {
                    // Try to parse `<lo-hi>`. If the form doesn't
                    // match, fall back to literal `<`. Direct port of
                    // zsh's numeric-range glob handler — speculative
                    // scan for the closing `>`, split on `-`, parse
                    // optional bounds. Matches `<5-10>`, `<5->`,
                    // `<-10>`, `<->`.
                    let parsed: Option<(Option<i64>, Option<i64>, usize)> = (|| {
                        let mut buf = String::new();
                        let peek_iter = chars.clone();
                        for c in peek_iter {
                            buf.push(c);
                            if c == '>' { break; }
                            if buf.len() > 64 { return None; }
                        }
                        if !buf.ends_with('>') {
                            return None;
                        }
                        let inner = &buf[..buf.len() - 1];
                        let (lo_str, hi_str) = inner.split_once('-')?;
                        let lo: Option<i64> = if lo_str.is_empty() {
                            None
                        } else {
                            Some(lo_str.parse().ok()?)
                        };
                        let hi: Option<i64> = if hi_str.is_empty() {
                            None
                        } else {
                            Some(hi_str.parse().ok()?)
                        };
                        let n = buf.chars().count();
                        for _ in 0..n { chars.next(); }
                        Some((lo, hi, n))
                    })();
                    if let Some((lo, hi, consumed)) = parsed {
                        regex_pattern.push_str("(\\d+)");
                        capture_group_count += 1;
                        numeric_ranges.push((capture_group_count, lo, hi));
                        let _ = consumed;
                    } else {
                        regex_pattern.push('<');
                    }
                }
                '[' => {
                    // Direct port of zsh's character-class compile
                    // (pattern.c, see `patcompcls` and the `[`
                    // handling in `patcompswitch`):
                    //   - `[!...]` and `[^...]` both negate (POSIX +
                    //     zsh both accept; only `^` is canonical
                    //     regex). Translate `!` -> `^` so the regex
                    //     crate sees the right form. Was being
                    //     copied verbatim, so `[!a]` matched `!` or
                    //     `a` instead of "anything but a".
                    //   - POSIX character classes `[:alpha:]` /
                    //     `[:digit:]` etc. inside `[...]` already
                    //     pass through the regex crate, but the
                    //     trailing `]` of the class would be misread
                    //     as the closing of the outer bracket. Walk
                    //     past `[:NAME:]` as a unit so the next `]`
                    //     after the class isn't taken as the close.
                    //   - Backslash-escaped `]` (`[\\]]`) keeps the
                    //     `]` as a literal class member.
                    regex_pattern.push('[');
                    let mut first = true;
                    while let Some(cc) = chars.next() {
                        if first && cc == '!' {
                            regex_pattern.push('^');
                            first = false;
                            continue;
                        }
                        first = false;
                        if cc == ']' {
                            regex_pattern.push(']');
                            break;
                        }
                        if cc == '\\' {
                            // Pass escape + next char through.
                            regex_pattern.push('\\');
                            if let Some(nx) = chars.next() {
                                regex_pattern.push(nx);
                            }
                            continue;
                        }
                        if cc == '[' && chars.peek() == Some(&':') {
                            // POSIX class `[:NAME:]`. Read until
                            // `:]` then push the class verbatim.
                            regex_pattern.push('[');
                            let mut prev_colon = false;
                            for ic in chars.by_ref() {
                                regex_pattern.push(ic);
                                if prev_colon && ic == ']' {
                                    break;
                                }
                                prev_colon = ic == ':';
                            }
                            continue;
                        }
                        regex_pattern.push(cc);
                    }
                    // After a closed `[...]`, the bracket is a single
                    // regex atom — apply extendedglob `#`/`##`
                    // postfix as `*`/`+` directly.
                    if let Some(q) = consume_extglob_postfix(&mut chars) {
                        regex_pattern.push_str(q);
                    }
                }
                '(' => {
                    // `(#cN)` and `(#cN,M)` post-subpattern repetition
                    // qualifiers: the previous element gets a `{N}` or
                    // `{N,M}` regex quantifier. Detect by peeking for
                    // `#c` after the opening `(`.
                    let peek_iter = chars.clone();
                    let mut probe: Vec<char> = Vec::new();
                    let p = peek_iter;
                    for pc in p {
                        probe.push(pc);
                        if pc == ')' || probe.len() > 32 {
                            break;
                        }
                    }
                    let probe_str: String = probe.iter().collect();
                    if probe_str.starts_with("#c") && probe_str.ends_with(')') {
                        let body = &probe_str[2..probe_str.len() - 1];
                        let quant = if let Some((lo, hi)) = body.split_once(',') {
                            format!("{{{},{}}}", lo, hi)
                        } else {
                            format!("{{{}}}", body)
                        };
                        regex_pattern.push_str(&quant);
                        // Advance the real iterator past the consumed chars.
                        for _ in 0..probe.len() {
                            chars.next();
                        }
                    } else if probe_str == "#e)" {
                        // `(#e)` — match end-of-string anchor. Direct
                        // port of zsh's pattern.c P_EOL token (zsh's
                        // "globbing flag" `(#e)` per zshexpn(1)).
                        // Emits regex `$` to anchor the match at the
                        // end of the input. Used by zinit's
                        // `(#b)((*)\\(#e)|(*))` to detect a trailing
                        // `\` in each element.
                        regex_pattern.push('$');
                        for _ in 0..probe.len() {
                            chars.next();
                        }
                    } else if probe_str == "#s)" {
                        // `(#s)` — match start-of-string anchor.
                        // zshexpn(1): "matches at the start of the
                        // test string". Emits regex `^`.
                        regex_pattern.push('^');
                        for _ in 0..probe.len() {
                            chars.next();
                        }
                    } else {
                        regex_pattern.push('(');
                        capture_group_count += 1;
                    }
                }
                ')' => {
                    regex_pattern.push(')');
                    // Closed group is an atom — extendedglob `#`/`##`
                    // postfix applies to the whole group.
                    if let Some(q) = consume_extglob_postfix(&mut chars) {
                        regex_pattern.push_str(q);
                    }
                }
                '|' => regex_pattern.push('|'),
                '\\' => {
                    // Special-case: `\(#e)` / `\(#s)` — literal
                    // backslash followed by extendedglob end/start
                    // anchor. Emit `\\$` / `\\^` so the pattern matches
                    // a literal trailing/leading `\`. Without this the
                    // `(` of `(#e)` got consumed as the escaped char,
                    // dropping the anchor entirely. Direct port of
                    // pattern.c P_EOL/P_BOL recognition after a `\`.
                    // Only fires under extendedglob — without the
                    // option, `(#e)` is not a token at all.
                    if extendedglob_on {
                        let mut peek = chars.clone();
                        let p1 = peek.next();
                        let p2 = peek.next();
                        let p3 = peek.next();
                        let p4 = peek.next();
                        if p1 == Some('(')
                            && p2 == Some('#')
                            && (p3 == Some('e') || p3 == Some('s'))
                            && p4 == Some(')')
                        {
                            regex_pattern.push_str("\\\\");
                            regex_pattern.push(if p3 == Some('e') { '$' } else { '^' });
                            chars.next(); chars.next(); chars.next(); chars.next();
                            continue;
                        }
                    }
                    // Backslash escapes the next char — treat literally.
                    if let Some(next) = chars.next() {
                        if matches!(
                            next,
                            '.' | '+'
                                | '^'
                                | '$'
                                | '\\'
                                | '{'
                                | '}'
                                | '*'
                                | '?'
                                | '('
                                | ')'
                                | '|'
                                | '['
                                | ']'
                        ) {
                            regex_pattern.push('\\');
                        }
                        regex_pattern.push(next);
                    } else {
                        regex_pattern.push_str("\\\\");
                    }
                }
                '.' | '+' | '^' | '$' | '{' | '}' => {
                    regex_pattern.push('\\');
                    regex_pattern.push(c);
                }
                _ => {
                    if l_flag && c.is_ascii_lowercase() {
                        regex_pattern.push('[');
                        regex_pattern.push(c);
                        regex_pattern.push(c.to_ascii_uppercase());
                        regex_pattern.push(']');
                    } else {
                        regex_pattern.push(c);
                    }
                    // After a literal/(#l)-class atom, extendedglob
                    // `#`/`##` postfix maps to regex `*`/`+` and
                    // binds to that single atom. Same as zsh's
                    // pattern.c POUND/POUND2 handling on the atom
                    // just compiled.
                    if let Some(q) = consume_extglob_postfix(&mut chars) {
                        regex_pattern.push_str(q);
                    }
                }
            }
        }
        regex_pattern.push('$');
        let final_pattern = if case_insensitive {
            format!("(?i){}", regex_pattern)
        } else {
            regex_pattern
        };
        if !numeric_ranges.is_empty() {
            // Need captures + per-group numeric range checks.
            let re = match regex::Regex::new(&final_pattern) {
                Ok(re) => re,
                Err(_) => return false,
            };
            let caps = match re.captures(s) {
                Some(c) => c,
                None => return false,
            };
            for (group_idx, lo, hi) in numeric_ranges.iter() {
                // A numeric-range `<N-M>` inside an alternation branch
                // that didn't fire (e.g. branch B of `(A|B)` when A
                // matched) won't have a populated capture. Skip the
                // bounds check for those — the alternation's match
                // already commits to the branch that DID fire.
                let cap_str = match caps.get(*group_idx) {
                    Some(m) => m.as_str(),
                    None => continue,
                };
                let n: i64 = match cap_str.parse() {
                    Ok(n) => n,
                    Err(_) => return false,
                };
                if let Some(l) = lo {
                    if n < *l {
                        return false;
                    }
                }
                if let Some(h) = hi {
                    if n > *h {
                        return false;
                    }
                }
            }
            return true;
        }
        regex::Regex::new(&final_pattern)
            .map(|re| re.is_match(s))
            .unwrap_or(false)
    }
    /// True if the input has at least one `\{` and a matching `\}` such
    /// that treating them as literal would produce a balanced string.
    /// Conservative — we only short-circuit when escaping is clearly
    /// the user's intent. Mixed `{a,\{b,c\}}` cases keep going through
    /// the regular expansion path.
    pub(crate) fn has_balanced_escaped_braces(s: &str) -> bool {
        let mut esc_open = 0usize;
        let mut esc_close = 0usize;
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i + 1 < chars.len() {
            if chars[i] == '\\' && chars[i + 1] == '{' {
                esc_open += 1;
                i += 2;
                continue;
            }
            if chars[i] == '\\' && chars[i + 1] == '}' {
                esc_close += 1;
                i += 2;
                continue;
            }
            i += 1;
        }
        esc_open > 0 && esc_open == esc_close
    }
    /// Expand glob pattern to matching files
    pub fn expand_glob(&self, pattern: &str) -> Vec<String> {
        // Glob alternation `(a|b|c)` is a primary zsh feature
        // (no extendedglob needed, unlike `~` exclusion). Direct
        // port of zsh's pattern.c handling of P_BRANCH | inside
        // grouping parens — at the path level, `/etc/(passwd|
        // hostname)` matches multiple alternative paths. zshrs's
        // glob crate (and earlier hand-rolled code) didn't expand
        // the `(...|...)` form, so the literal parens reached the
        // OS glob and produced no matches.
        //
        // Pre-expand by splitting top-level `(...|...)` groups
        // into separate patterns and recursing — same shape as
        // brace expansion at this layer. Skip when extendedglob
        // is on AND the pattern is `(#flag)` (inline pattern flag,
        // handled by the regex compiler downstream).
        if let Some(alternatives) = expand_glob_alternation(pattern) {
            // For each alternative, treat as a GLOB pattern: if it
            // contains other glob chars, recurse through expand_glob
            // (which handles `*`/`?`/`[`/qualifier suffixes); if
            // it's a literal path, only include it if the path
            // EXISTS — zsh's pattern.c behavior is "alternation
            // produces matching paths, not literal alternatives".
            // Without the exists-check, `/etc/(passwd|nonexistent)`
            // would output both.
            let mut out: Vec<String> = Vec::new();
            for alt in alternatives {
                let has_meta = alt.chars().any(|c| matches!(c, '*' | '?' | '[' | '('));
                if has_meta {
                    out.extend(self.expand_glob(&alt));
                } else if std::path::Path::new(&alt).exists() {
                    out.push(alt);
                }
            }
            let mut seen = std::collections::HashSet::new();
            out.retain(|p| seen.insert(p.clone()));
            // zsh sorts glob results alphabetically by default.
            // Without sorting, the alternation order leaks
            // through (`/etc/(passwd|group)` would output
            // `passwd group` instead of zsh's `group passwd`).
            out.sort();
            if !out.is_empty() {
                return out;
            }
            // No matches — fall through to NOMATCH semantics
            // below (zsh: error if `nomatch` is on, else literal).
        }
        // extendedglob `~` exclusion: `*.txt~b.txt` matches `*.txt`
        // and excludes paths that also match `b.txt`. Detect a
        // top-level `~` (not inside brackets/parens) when extendedglob
        // is on and split. Recursively expand both halves and remove
        // the RHS matches from the LHS list.
        let extglob_on = self.options.get("extendedglob").copied().unwrap_or(false);
        if extglob_on {
            // extendedglob `^pat` (negation): match everything that
            // does NOT match `pat`. The lexer leaves `^` as a literal
            // char, so we detect a leading `^` here and convert to a
            // directory-walk-then-filter. Only applies at the start
            // of the LAST path component (zsh: `^pat` only negates
            // the basename portion).
            let last_seg_start = pattern.rfind('/').map(|i| i + 1).unwrap_or(0);
            let last_seg = &pattern[last_seg_start..];
            if last_seg.starts_with('^') && last_seg.len() > 1 {
                let prefix = &pattern[..last_seg_start];
                let neg = &last_seg[1..];
                let dir = if prefix.is_empty() {
                    ".".to_string()
                } else {
                    prefix.trim_end_matches('/').to_string()
                };
                let mut out = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with('.') {
                            continue;
                        }
                        if !ShellExecutor::glob_match_static(&name, neg) {
                            let path = if prefix.is_empty() {
                                name
                            } else {
                                format!("{}{}", prefix, name)
                            };
                            out.push(path);
                        }
                    }
                }
                out.sort();
                if !out.is_empty() {
                    return out;
                }
                let nullglob = self.options.get("nullglob").copied().unwrap_or(false);
                if nullglob {
                    return Vec::new();
                }
                let nomatch = self.options.get("nomatch").copied().unwrap_or(true);
                if nomatch {
                    zerr(&format!("no matches found: {}", pattern));
                    std::process::exit(1);
                }
                return vec![pattern.to_string()];
            }
            // Find a top-level `~` outside brackets.
            let chars: Vec<char> = pattern.chars().collect();
            let mut depth_b = 0i32;
            let mut depth_p = 0i32;
            let mut split_at: Option<usize> = None;
            for (i, &c) in chars.iter().enumerate() {
                match c {
                    '[' => depth_b += 1,
                    ']' => depth_b -= 1,
                    '(' => depth_p += 1,
                    ')' => depth_p -= 1,
                    '~' if depth_b == 0 && depth_p == 0 && i > 0 => {
                        // Skip `~` at start (tilde expansion) and `~` adjacent
                        // to space (zsh treats those as expansion).
                        split_at = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            if let Some(pos) = split_at {
                let lhs: String = chars[..pos].iter().collect();
                let rhs: String = chars[pos + 1..].iter().collect();
                let lhs_matches = self.expand_glob(&lhs);
                // zsh pattern.c: `~` is an exclusion operator that matches
                // RHS as a PATTERN against each LHS candidate, not a
                // separate glob expansion in CWD. Match RHS against each
                // result's basename and full path.
                let filtered: Vec<String> = lhs_matches
                    .into_iter()
                    .filter(|p| {
                        let basename = p.rsplit('/').next().unwrap_or(p);
                        !ShellExecutor::glob_match_static(basename, &rhs)
                            && !ShellExecutor::glob_match_static(p, &rhs)
                    })
                    .collect();
                if !filtered.is_empty() {
                    return filtered;
                }
                // Empty after exclusion — fall through so NOMATCH
                // semantics fire if no nullglob.
                let nullglob = self.options.get("nullglob").copied().unwrap_or(false);
                if nullglob {
                    return Vec::new();
                }
                let nomatch = self.options.get("nomatch").copied().unwrap_or(true);
                if nomatch && Self::looks_like_glob(pattern) {
                    zerr(&format!("no matches found: {}", pattern));
                    std::process::exit(1);
                }
                return vec![pattern.to_string()];
            }
        }
        // Check for zsh glob qualifiers at end: *(.) *(/) *(@) etc.
        let (glob_pattern, qualifiers) = self.parse_glob_qualifiers(pattern);
        // Pre-process `[^...]` → `[!...]` so the `glob` crate (which
        // only accepts `!` for class negation per fnmatch) works for
        // zsh's `^` form too. Walk the pattern and only translate
        // inside `[...]` regions (so a literal `^` outside brackets
        // stays literal — extendedglob handles those separately).
        let glob_pattern = if glob_pattern.contains("[^") {
            let mut out = String::with_capacity(glob_pattern.len());
            let mut chars = glob_pattern.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '[' {
                    out.push('[');
                    if chars.peek() == Some(&'^') {
                        chars.next();
                        out.push('!');
                    }
                    for cc in chars.by_ref() {
                        out.push(cc);
                        if cc == ']' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        } else {
            glob_pattern
        };

        // POSIX character classes: `[[:alpha:]]`, `[[:digit:]]` etc.
        // The `glob` crate doesn't recognise the `[:class:]` syntax —
        // convert each known class to its enumerated char range so
        // the underlying matcher sees a plain char-class. Done here
        // (not at the lexer) so the substitution survives all the
        // way to glob::glob_with(). Tracks: alnum, alpha, blank,
        // cntrl, digit, graph, lower, print, punct, space, upper,
        // xdigit. Each translates to ranges like `0-9`/`a-zA-Z`.
        let glob_pattern = if glob_pattern.contains("[:") {
            // Inline expansion of `[[:alpha:]]` → `[a-zA-Z]` etc.
            // Mirrors the inline `[:class:]` switch the C source does
            // in pattern.c::patmatchrange. Each known class translates
            // to its standard ASCII range; unknown classes pass through.
            let s = &glob_pattern;
            let mut out = String::with_capacity(s.len());
            let chars: Vec<char> = s.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '[' && i + 2 < chars.len() && chars[i + 1] == ':' {
                    let mut j = i + 2;
                    while j + 1 < chars.len() && !(chars[j] == ':' && chars[j + 1] == ']') {
                        j += 1;
                    }
                    if j + 1 < chars.len() && chars[j] == ':' && chars[j + 1] == ']' {
                        let name: String = chars[i + 2..j].iter().collect();
                        let range = match name.as_str() {
                            "alpha" => "a-zA-Z",
                            "alnum" => "a-zA-Z0-9",
                            "digit" => "0-9",
                            "xdigit" => "0-9a-fA-F",
                            "lower" => "a-z",
                            "upper" => "A-Z",
                            "space" => " \\t\\n\\r\\v\\f",
                            "blank" => " \\t",
                            "cntrl" => "\\x00-\\x1f\\x7f",
                            "print" => "\\x20-\\x7e",
                            "graph" => "\\x21-\\x7e",
                            "punct" => "!-/:-@\\[-`{-~",
                            _ => "",
                        };
                        if !range.is_empty() {
                            out.push_str(range);
                            i = j + 2;
                            continue;
                        }
                    }
                }
                out.push(chars[i]);
                i += 1;
            }
            out
        } else {
            glob_pattern
        };

        // zsh numeric range glob `<N-M>`, `<N->`, `<-M>`, `<->`.
        // The `glob` crate has no equivalent — match by replacing the
        // range with `*` and post-filtering by extracting the digit
        // sequence at that position and verifying it falls in [N, M].
        // Only fires when the pattern actually contains a `<…-…>` shape
        // — guard with a fast contains() before the regex.
        let numeric_ranges = if glob_pattern.contains('<') {
            NumericRange::extract_all(&glob_pattern)
        } else {
            Vec::new()
        };
        let glob_pattern = if !numeric_ranges.is_empty() {
            NumericRange::replace_all_with_star(&glob_pattern)
        } else {
            glob_pattern
        };

        // Check for extended glob patterns: ?(pat), *(pat), +(pat), @(pat), !(pat)
        if self.has_extglob_pattern(&glob_pattern) {
            let expanded = self.expand_glob(&glob_pattern);
            return self.filter_by_qualifiers(expanded, &qualifiers);
        }

        let nullglob = self.options.get("nullglob").copied().unwrap_or(false);
        // `(D)` glob qualifier — per-pattern dotglob. Same effect as
        // `setopt dotglob` but scoped to this expansion only.
        // Also: when the LAST path component starts with literal `.`,
        // treat as if dotglob was on (zsh: `.*` matches dotfiles even
        // without setopt dotglob, because the leading `.` is literal).
        let last_seg = glob_pattern.rsplit('/').next().unwrap_or(&glob_pattern);
        let pattern_starts_with_dot = last_seg.starts_with('.');
        // `globdots` is the zsh canonical name; `dotglob` is the bash
        // alias. Both end up stored under their own key by setopt — read
        // both so either spelling works.
        let dotglob = self.options.get("dotglob").copied().unwrap_or(false)
            || self.options.get("globdots").copied().unwrap_or(false)
            || qualifiers.contains('D')
            || pattern_starts_with_dot;
        // `setopt nocaseglob` normalizes to `caseglob=false` in the
        // options table (the `no` prefix is the negation marker).
        // Read both forms so user code that flips either key works:
        //   - `caseglob=false` → case-INSENSITIVE
        //   - `nocaseglob=true` → case-INSENSITIVE (legacy / direct)
        let nocaseglob = !self.options.get("caseglob").copied().unwrap_or(true)
            || self.options.get("nocaseglob").copied().unwrap_or(false);

        // Parallel recursive glob: when pattern contains **/ we split the
        // directory walk across worker pool threads — one thread per top-level
        // subdirectory.  zsh does this single-threaded via fork+exec which is
        // why `echo **/*.rs` is painfully slow on large trees.
        let mut expanded = if !numeric_ranges.is_empty() {
            // `<N-M>` numeric range glob — handle via direct directory
            // walk so the digit-count semantics survive (the glob crate
            // can't express "one or more digits" precisely).
            self.expand_glob_with_numeric_range(pattern, &numeric_ranges, dotglob, nocaseglob)
        } else if glob_pattern.contains("**/") {
            self.expand_glob_parallel(&glob_pattern, dotglob, nocaseglob)
        } else {
            let options = glob::MatchOptions {
                case_sensitive: !nocaseglob,
                require_literal_separator: false,
                require_literal_leading_dot: !dotglob,
            };
            match glob::glob_with(&glob_pattern, options) {
                Ok(paths) => paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                Err(_) => vec![],
            }
        };

        // zsh always excludes "." and ".." from glob results, even
        // with `dotglob` set or when the pattern is `.*`. The Rust
        // glob crate includes them. `Path::file_name` returns None
        // for these (treats them as cur/parent-dir components), so
        // check the trailing path segment textually.
        expanded.retain(|p| {
            let last = p.rsplit('/').next().unwrap_or(p);
            last != "." && last != ".."
        });

        let expanded = self.filter_by_qualifiers(expanded, &qualifiers);
        let mut expanded = expanded;
        // zsh: `echo */` outputs each directory with a trailing
        // slash. The Rust glob crate strips trailing slashes from
        // matches, so re-append when the pattern ended in `/`.
        if glob_pattern.ends_with('/') {
            for p in expanded.iter_mut() {
                if !p.ends_with('/') {
                    p.push('/');
                }
            }
        }
        // Locale-aware sort: under a Unicode locale, zsh folds case
        // (`Aaa bbb Ccc Ddd` not `Aaa Ccc Ddd bbb`). Fallback to byte
        // order under C/POSIX. Sort by basename so directory components
        // don't dominate the comparison and produce ASCII-style output.
        // Skip when the qualifier requested an explicit sort (`o*`/`O*`)
        // — those reorder by mtime/size/etc and the alpha sort would
        // clobber the result.
        let user_sort = qualifiers.contains('o') || qualifiers.contains('O');
        if !user_sort {
            // For `**/...` recursive globs, sort by the FULL path so
            // depth-first / breadth-first walk order is preserved
            // (zsh's natural recursive order: `dir/f sub sub/g`, not
            // basename-sorted `f g sub`). For plain (non-recursive)
            // globs, sort by BASENAME to match zsh's locale-aware
            // case-folded output.
            if glob_pattern.contains("**/") {
                expanded.sort_by(|a, b| crate::glob::gmatchcmp(a, b));
            } else {
                expanded.sort_by(|a, b| {
                    let an = a.rsplit('/').next().unwrap_or(a);
                    let bn = b.rsplit('/').next().unwrap_or(b);
                    crate::glob::gmatchcmp(an, bn)
                });
            }
        }

        if expanded.is_empty() {
            // The `(N)` per-pattern qualifier is the local equivalent of
            // `setopt nullglob` — when present on this glob, no-match
            // collapses to an empty list (silent) instead of the literal
            // pattern. Mirrors zsh's `*(N)` semantics.
            if nullglob || qualifiers.contains('N') {
                return vec![];
            }
            // zsh's default is `setopt nomatch`: an unmatched glob
            // emits "no matches found" on stderr and aborts the command
            // (the shell exits in -c mode). bash-style "pass literal
            // through" is the opt-out via `unsetopt nomatch`.
            let nomatch = self.options.get("nomatch").copied().unwrap_or(true);
            if nomatch && Self::looks_like_glob(pattern) {
                zerr(&format!("no matches found: {}", pattern));
                // zsh: command is aborted (skipped) with status 1,
                // script continues. Set the flag the simple-command
                // dispatcher checks; it returns early before exec.
                self.current_command_glob_failed.set(true);
                return Vec::new();
            }
            vec![pattern.to_string()]
        } else {
            expanded
        }
    }
    /// True iff the literal `pattern` actually contains a glob metachar
    /// in a position that would have triggered globbing. Used to avoid
    /// spurious "no matches" errors when expand_glob is called on a
    /// plain path that happened to route through this code (e.g. some
    /// fast paths bridge unconditionally).
    pub(crate) fn looks_like_glob(pattern: &str) -> bool {
        // A trailing `(qualifier)` is itself a glob trigger — e.g.
        // `path(L+10)` should be treated as a glob even when the
        // body has no `*`/`?`/`[...]`.
        let has_qual_suffix = if let Some(open) = pattern.rfind('(') {
            pattern.ends_with(')') && open + 1 < pattern.len() - 1
        } else {
            false
        };
        // Strip trailing `(...)` qualifier so we test the pattern body.
        let body = if let Some(open) = pattern.rfind('(') {
            if pattern.ends_with(')') {
                &pattern[..open]
            } else {
                pattern
            }
        } else {
            pattern
        };
        // Walk character-by-character so escaped metachars (`\*`, `\?`,
        // `\[`) are NOT counted as glob triggers. zsh: `echo \*` prints
        // a literal `*`; without the unescaped check, looks_like_glob
        // returned true on the bare `*` and the runtime glob expansion
        // aborted with NOMATCH.
        let chars: Vec<char> = body.chars().collect();
        let mut i = 0;
        let mut has_unescaped_star = false;
        let mut has_unescaped_question = false;
        let mut has_unescaped_bracket_open: Option<usize> = None;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' && i + 1 < chars.len() {
                // Escaped char — skip both.
                i += 2;
                continue;
            }
            match c {
                '*' => has_unescaped_star = true,
                '?' => has_unescaped_question = true,
                '[' if has_unescaped_bracket_open.is_none() => {
                    has_unescaped_bracket_open = Some(i);
                }
                _ => {}
            }
            i += 1;
        }
        // `[` only counts when there's a matching `]` after it.
        let has_bracket_class = has_unescaped_bracket_open
            .map(|i| body[i + 1..].contains(']'))
            .unwrap_or(false);
        // `<N-M>` numeric range glob is also a trigger — match shape
        // `<` + optional digits + `-` + optional digits + `>` outside
        // any bracket expression.
        let has_numeric_range =
            body.contains('<') && body.contains('>') && !NumericRange::extract_all(body).is_empty();
        has_unescaped_star
            || has_unescaped_question
            || has_bracket_class
            || has_qual_suffix
            || has_numeric_range
    }
    /// Direct directory walk for numeric-range glob `<N-M>`.
    ///
    /// Split the pattern at the last `/` so the dir component can stay
    /// concrete (or be globbed normally) and the basename gets a custom
    /// regex match. Numeric range groups capture `(\d+)` and each
    /// capture must fall inside its declared `[lo, hi]` range — open
    /// ends mean unbounded on that side.
    pub(crate) fn expand_glob_with_numeric_range(
        &self,
        pattern: &str,
        ranges: &[NumericRange],
        dotglob: bool,
        nocaseglob: bool,
    ) -> Vec<String> {
        let (dir_part, file_part) = match pattern.rfind('/') {
            Some(idx) => (&pattern[..idx], &pattern[idx + 1..]),
            None => ("", pattern),
        };
        // Build the basename regex: glob → regex, with each `<N-M>`
        // becoming a numbered capture group `(\d+)`.
        let mut rx = String::from("^");
        let chars: Vec<char> = file_part.chars().collect();
        let mut i = 0;
        let mut in_bracket = false;
        while i < chars.len() {
            let c = chars[i];
            if c == '[' && !in_bracket {
                in_bracket = true;
                rx.push('[');
                i += 1;
                continue;
            }
            if c == ']' && in_bracket {
                in_bracket = false;
                rx.push(']');
                i += 1;
                continue;
            }
            if in_bracket {
                rx.push(c);
                i += 1;
                continue;
            }
            if c == '<' {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '-' {
                    j += 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] == '>' {
                        rx.push_str("(\\d+)");
                        i = j + 1;
                        continue;
                    }
                }
            }
            match c {
                '*' => rx.push_str(".*"),
                '?' => rx.push('.'),
                '.' | '+' | '(' | ')' | '|' | '^' | '$' | '\\' | '{' | '}' => {
                    rx.push('\\');
                    rx.push(c);
                }
                _ => rx.push(c),
            }
            i += 1;
        }
        rx.push('$');
        let re = match if nocaseglob {
            regex::RegexBuilder::new(&rx).case_insensitive(true).build()
        } else {
            regex::Regex::new(&rx).map_err(|e| regex::Error::Syntax(e.to_string()))
        } {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        // Resolve dir_part: it may itself contain glob chars (e.g.
        // `**/file<2-4>`). For now require the dir part to be either
        // empty (cwd) or a literal path; defer recursive ranges.
        let mut dirs: Vec<String> = if dir_part.is_empty() {
            vec![".".to_string()]
        } else if dir_part.contains('*')
            || dir_part.contains('?')
            || dir_part.contains('[')
            || dir_part.contains('<')
        {
            // Glob the dir component first, keeping only directories.
            let opts = glob::MatchOptions {
                case_sensitive: !nocaseglob,
                require_literal_separator: false,
                require_literal_leading_dot: !dotglob,
            };
            match glob::glob_with(dir_part, opts) {
                Ok(paths) => paths
                    .filter_map(|p| p.ok())
                    .filter(|p| p.is_dir())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                Err(_) => return Vec::new(),
            }
        } else {
            vec![dir_part.to_string()]
        };
        if dirs.is_empty() {
            dirs.push(dir_part.to_string());
        }

        let mut out = Vec::new();
        for dir in &dirs {
            let read = match std::fs::read_dir(if dir.is_empty() { "." } else { dir }) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !dotglob && name.starts_with('.') && !file_part.starts_with('.') {
                    continue;
                }
                let caps = match re.captures(&name) {
                    Some(c) => c,
                    None => continue,
                };
                let mut ok = true;
                for (idx, range) in ranges.iter().enumerate() {
                    let cap = match caps.get(idx + 1) {
                        Some(m) => m.as_str(),
                        None => {
                            ok = false;
                            break;
                        }
                    };
                    let val: i64 = match cap.parse() {
                        Ok(v) => v,
                        Err(_) => {
                            ok = false;
                            break;
                        }
                    };
                    if let Some(lo) = range.lo {
                        if val < lo {
                            ok = false;
                            break;
                        }
                    }
                    if let Some(hi) = range.hi {
                        if val > hi {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                let full = if dir == "." || dir.is_empty() {
                    name
                } else if dir.ends_with('/') {
                    format!("{}{}", dir, name)
                } else {
                    format!("{}/{}", dir, name)
                };
                out.push(full);
            }
        }
        out.sort();
        out
    }
    /// Parallel recursive glob using the worker pool.
    ///
    /// Splits `base/**/file_pattern` into per-subdirectory walks, each
    /// running on a pool thread via walkdir.  Results merge via channel.
    /// This is why `echo **/*.rs` will be 5-10x faster than zsh.
    pub(crate) fn expand_glob_parallel(&self, pattern: &str, dotglob: bool, nocaseglob: bool) -> Vec<String> {
        use walkdir::WalkDir;

        // Split pattern at the first **/ into (base_dir, file_glob)
        // e.g. "src/**/*.rs" → ("src", "*.rs")
        //      "**/*.rs"     → (".", "*.rs")
        //      "**/"         → (".", "")  with dirs_only=true
        //      "**/*"        → (".", "*") with both files+dirs
        let (base, file_glob) = if let Some(pos) = pattern.find("**/") {
            let base = if pos == 0 {
                "."
            } else {
                &pattern[..pos.saturating_sub(1)]
            };
            let rest = &pattern[pos + 3..]; // skip "**/", get "*.rs" or "foo/**/*.rs"
            (base.to_string(), rest.to_string())
        } else {
            return vec![];
        };

        // Trailing-slash form `**/`: zsh enumerates matching directories
        // (with the trailing slash preserved). Empty file_glob means
        // "match every dir under base, no file mask".
        let dirs_only = file_glob.is_empty();

        // If file_glob itself contains **/, fall back to single-threaded glob
        // (nested recursive patterns are rare, not worth the complexity)
        if file_glob.contains("**/") {
            let options = glob::MatchOptions {
                case_sensitive: !nocaseglob,
                require_literal_separator: false,
                require_literal_leading_dot: !dotglob,
            };
            return match glob::glob_with(pattern, options) {
                Ok(paths) => paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                Err(_) => vec![],
            };
        }

        // Build the glob::Pattern for matching filenames. For
        // `dirs_only` (trailing-slash `**/`) we don't have a file mask
        // — every directory matches.
        let match_opts = glob::MatchOptions {
            case_sensitive: !nocaseglob,
            require_literal_separator: false,
            require_literal_leading_dot: !dotglob,
        };
        let file_pat = if dirs_only {
            None
        } else {
            match glob::Pattern::new(&file_glob) {
                Ok(p) => Some(p),
                Err(_) => return vec![],
            }
        };
        // For `**/*` (file_glob = "*"), zsh matches both files and
        // directories. For `**/foo` (specific file pattern), still
        // match either type — zsh doesn't restrict to file-type unless
        // a `(.)` qualifier is appended.
        let match_dirs_too = !dirs_only;

        // Enumerate top-level entries in base dir to fan out across workers
        let top_entries: Vec<std::path::PathBuf> = match std::fs::read_dir(&base) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
            Err(_) => return vec![],
        };

        // Also check files (and dirs in dirs_only / match_dirs_too mode)
        // directly in base (not in subdirs).
        let mut results: Vec<String> = Vec::new();
        for entry in &top_entries {
            let is_dir = entry.is_dir();
            let is_file = entry.is_file() || entry.is_symlink();
            let want = if dirs_only {
                is_dir
            } else {
                is_file || (match_dirs_too && is_dir)
            };
            if want {
                if let Some(name) = entry.file_name().and_then(|n| n.to_str()) {
                    let matches = match &file_pat {
                        None => true,
                        Some(p) => p.matches_with(name, match_opts),
                    };
                    if matches {
                        let mut s = entry.to_string_lossy().to_string();
                        if dirs_only {
                            s.push('/');
                        }
                        results.push(s);
                    }
                }
            }
        }

        // Fan out subdirectory walks to worker pool
        let subdirs: Vec<std::path::PathBuf> = top_entries
            .into_iter()
            .filter(|p| p.is_dir())
            .filter(|p| {
                dotglob
                    || !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with('.'))
                        .unwrap_or(false)
            })
            .collect();

        if subdirs.is_empty() {
            return results;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();

        for subdir in &subdirs {
            let tx = tx.clone();
            let subdir = subdir.clone();
            let file_pat = file_pat.clone();
            let skip_dot = !dotglob;
            let dirs_only_w = dirs_only;
            let match_dirs_too_w = match_dirs_too;
            self.worker_pool.submit(move || {
                let mut matches = Vec::new();
                let walker = WalkDir::new(&subdir)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(move |e| {
                        // Skip hidden dirs if !dotglob
                        if skip_dot {
                            if let Some(name) = e.file_name().to_str() {
                                if name.starts_with('.') && e.depth() > 0 {
                                    return false;
                                }
                            }
                        }
                        true
                    });
                for entry in walker.filter_map(|e| e.ok()) {
                    let is_file = entry.file_type().is_file() || entry.file_type().is_symlink();
                    let is_dir = entry.file_type().is_dir();
                    // Skip the subdir root itself — it was already added
                    // by the top-level loop.
                    if entry.depth() == 0 {
                        continue;
                    }
                    let want = if dirs_only_w {
                        is_dir
                    } else {
                        is_file || (match_dirs_too_w && is_dir)
                    };
                    if want {
                        if let Some(name) = entry.file_name().to_str() {
                            let matches_pat = match &file_pat {
                                None => true,
                                Some(p) => p.matches_with(name, match_opts),
                            };
                            if matches_pat {
                                let mut s = entry.path().to_string_lossy().to_string();
                                if dirs_only_w {
                                    s.push('/');
                                }
                                matches.push(s);
                            }
                        }
                    }
                }
                let _ = tx.send(matches);
            });
        }

        // Drop our sender so rx knows when all workers are done
        drop(tx);

        // Collect results from all workers
        for batch in rx {
            results.extend(batch);
        }

        // When base was the implicit "." (the user wrote `**/...`,
        // not `./**/...`), zsh emits relative paths without the `./`
        // prefix. Strip it here for parity.
        if base == "." {
            results = results
                .into_iter()
                .map(|s| s.strip_prefix("./").map(|t| t.to_string()).unwrap_or(s))
                .collect();
        }

        // zsh sorts the recursive-glob result lexicographically. Without
        // this, the parallel-walker order leaks through and `**/*`
        // returns paths in worker-completion order (`f sub/g sub`
        // instead of `f sub sub/g`).
        results.sort();

        results
    }
    /// Parse zsh glob qualifiers from the end of a pattern
    /// Returns (pattern_without_qualifiers, qualifiers_string)
    pub(crate) fn parse_glob_qualifiers(&self, pattern: &str) -> (String, String) {
        // Check if pattern ends with (...) that looks like qualifiers
        // Qualifiers are single chars like . / @ * % or combinations
        if !pattern.ends_with(')') {
            return (pattern.to_string(), String::new());
        }

        // Find matching opening paren
        let chars: Vec<char> = pattern.chars().collect();
        let mut depth = 0;
        let mut qual_start = None;

        for i in (0..chars.len()).rev() {
            match chars[i] {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        qual_start = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(start) = qual_start {
            let qual_content: String = chars[start + 1..chars.len() - 1].iter().collect();

            // Check if this looks like glob qualifiers (not extglob)
            // Qualifiers are things like: . / @ * % r w x ^ - etc.
            // Extglob would have | inside
            if !qual_content.contains('|') && self.looks_like_glob_qualifiers(&qual_content) {
                let base_pattern: String = chars[..start].iter().collect();
                return (base_pattern, qual_content);
            }
        }

        (pattern.to_string(), String::new())
    }
    /// Check if string looks like glob qualifiers
    pub(crate) fn looks_like_glob_qualifiers(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        // Valid qualifier chars (zsh glob qualifier set):
        //   type/perm: . / @ = p * % b r w x s A I E R W X
        //   sort:      o O n L l a m c d N
        //   time qual: a m c — followed by unit (s h m M d w) and op (+ -)
        //   user/grp:  u g
        //   nullglob:  N
        //   dotglob:   D
        //   T (path component)
        //   numeric ranges and digits for depth/uid/gid: 0-9 + - , [ ] :
        // Previously missing: `h` (hours unit), `g` (group qualifier),
        // `H` (non-empty-dir alt), `U` (owned-by-user) — adding them
        // unlocks `(mh-N)`, `(g+N)`, `(U)`, etc.
        // `O` (reverse-sort prefix, complementing `o`) was missing —
        // `*(Om)` was being treated as a literal pattern instead of a
        // qualifier set, leaving the trailing `)` unmatched. Added.
        let valid_chars = "./@=p*%bghilrwxAIERWXsStfHedDLNnMmcaouUYHTk^-+:0123456789,[]FO";
        s.chars()
            .all(|c| valid_chars.contains(c) || c.is_whitespace())
    }
    pub(crate) fn filter_by_qualifiers(&self, files: Vec<String>, qualifiers: &str) -> Vec<String> {
        if qualifiers.is_empty() {
            return files;
        }

        // Top-level `,` in the qualifier list is OR (zsh: `*(.,/)`
        // = files OR dirs). Direct port of zsh's pattern.c
        // qualifier parsing — comma splits at clause boundary,
        // each clause runs its own AND filter, the results are
        // UNIONed and de-duplicated. Single-clause (no comma)
        // path is unchanged.
        let has_or = {
            let mut depth_b = 0;
            let mut depth_p = 0;
            let mut found = false;
            for c in qualifiers.chars() {
                match c {
                    '[' => depth_b += 1,
                    ']' if depth_b > 0 => depth_b -= 1,
                    '(' if depth_b == 0 => depth_p += 1,
                    ')' if depth_b == 0 && depth_p > 0 => depth_p -= 1,
                    ',' if depth_b == 0 && depth_p == 0 => {
                        found = true;
                        break;
                    }
                    _ => {}
                }
            }
            found
        };
        if has_or {
            // Split at top-level commas, recurse for each clause,
            // union the results in original-file order. Each
            // clause re-runs the full filter so qualifier flags
            // (`L+0`, `om`, etc.) inside one clause stay scoped.
            let mut clauses: Vec<String> = Vec::new();
            let mut current = String::new();
            let mut depth_b = 0;
            let mut depth_p = 0;
            for c in qualifiers.chars() {
                match c {
                    '[' => {
                        depth_b += 1;
                        current.push(c);
                    }
                    ']' if depth_b > 0 => {
                        depth_b -= 1;
                        current.push(c);
                    }
                    '(' if depth_b == 0 => {
                        depth_p += 1;
                        current.push(c);
                    }
                    ')' if depth_b == 0 && depth_p > 0 => {
                        depth_p -= 1;
                        current.push(c);
                    }
                    ',' if depth_b == 0 && depth_p == 0 => {
                        clauses.push(std::mem::take(&mut current));
                    }
                    _ => current.push(c),
                }
            }
            clauses.push(current);
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut out: Vec<String> = Vec::new();
            for clause in &clauses {
                let matched = self.filter_by_qualifiers(files.clone(), clause);
                for m in matched {
                    if seen.insert(m.clone()) {
                        out.push(m);
                    }
                }
            }
            return out;
        }

        // Parallel metadata prefetch — all stat syscalls happen on pool threads,
        // then filter/sort uses cached metadata with zero syscalls.
        let meta_cache = self.prefetch_metadata(&files);

        let mut result = files;
        let mut negate = false;
        // (M) mark-dirs and (T) list-types qualifiers — direct port of
        // zsh/Src/glob.c:1557-1566. zsh appends a single char to each
        // output (or only to dirs for `M`). We collect the flags during
        // the filter loop and apply marking AFTER all filtering is done
        // so the suffix sticks on the final result, not midway. `^M`
        // disables (toggles negate to clear the flag) — same as zsh.
        let mut mark_dirs = false;
        let mut list_types = false;
        let mut chars = qualifiers.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                // Negation
                '^' => negate = !negate,
                // (M) mark dirs with `/`. negate=true (`^M`) clears.
                'M' => {
                    mark_dirs = !negate;
                    negate = false;
                }
                // (T) list types (ls -F style: /, *, @, |, =, #, %).
                'T' => {
                    list_types = !negate;
                    negate = false;
                }

                // History modifier `:r` / `:e` / `:t` / `:h` /
                // `:s/pat/repl/` etc. applied to each match. Direct
                // port of zsh's pattern.c qualifier modifier
                // handling — `:NAME` consumes through the next
                // qualifier-list-end (next `,` or `)`) and
                // dispatches each modifier to apply_history_modifiers
                // per element.
                ':' => {
                    // Collect the modifier chain — consume until
                    // we hit another qualifier-flag char or end.
                    // For simplicity, consume to end since the
                    // qualifier-end already strips the trailing
                    // `)`. The apply_history_modifiers helper
                    // tolerates a leading `:`.
                    let mut mods = String::from(":");
                    // Consume to end — qualifier-end already stripped
                    // the trailing `)`, so no internal delimiter check
                    // is needed (apply_history_modifiers tolerates the
                    // leading `:`).
                    while chars.peek().is_some() {
                        mods.push(chars.next().unwrap());
                    }
                    let modref = mods.as_str();
                    result = result
                        .into_iter()
                        .map(|p| self.apply_history_modifiers(&p, modref))
                        .collect();
                }

                // File types — all use prefetched metadata cache
                '.' => {
                    // zsh: `.` is "plain regular file" — excludes
                    // symlinks (use `@` for those). The `-`
                    // qualifier modifier (`(-.)`) inverts this:
                    // follow the symlink before testing, so a link
                    // to a regular file IS included. Direct port of
                    // zsh pattern.c QUAL_NULL → stat-not-lstat
                    // toggle.
                    let follow_links = qualifiers.contains('-');
                    result.retain(|f| {
                        let is_plain_file = meta_cache
                            .get(f)
                            .map(|(m, sm)| {
                                let is_link = sm
                                    .as_ref()
                                    .map(|m| m.file_type().is_symlink())
                                    .unwrap_or(false);
                                let is_reg = m.as_ref().map(|m| m.is_file()).unwrap_or(false);
                                if follow_links {
                                    is_reg
                                } else {
                                    is_reg && !is_link
                                }
                            })
                            .unwrap_or(false);
                        if negate {
                            !is_plain_file
                        } else {
                            is_plain_file
                        }
                    });
                    negate = false;
                }
                '/' => {
                    result.retain(|f| {
                        let is_dir = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.is_dir())
                            .unwrap_or(false);
                        if negate {
                            !is_dir
                        } else {
                            is_dir
                        }
                    });
                    negate = false;
                }
                '@' => {
                    result.retain(|f| {
                        let is_link = meta_cache
                            .get(f)
                            .and_then(|(_, sm)| sm.as_ref())
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false);
                        if negate {
                            !is_link
                        } else {
                            is_link
                        }
                    });
                    negate = false;
                }
                '=' => {
                    // Sockets
                    use std::os::unix::fs::FileTypeExt;
                    result.retain(|f| {
                        let is_socket = meta_cache
                            .get(f)
                            .and_then(|(_, sm)| sm.as_ref())
                            .map(|m| m.file_type().is_socket())
                            .unwrap_or(false);
                        if negate {
                            !is_socket
                        } else {
                            is_socket
                        }
                    });
                    negate = false;
                }
                'p' => {
                    // Named pipes (FIFOs)
                    use std::os::unix::fs::FileTypeExt;
                    result.retain(|f| {
                        let is_fifo = meta_cache
                            .get(f)
                            .and_then(|(_, sm)| sm.as_ref())
                            .map(|m| m.file_type().is_fifo())
                            .unwrap_or(false);
                        if negate {
                            !is_fifo
                        } else {
                            is_fifo
                        }
                    });
                    negate = false;
                }
                '*' => {
                    // Executable files
                    use std::os::unix::fs::PermissionsExt;
                    result.retain(|f| {
                        let is_exec = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.is_file() && (m.permissions().mode() & 0o111) != 0)
                            .unwrap_or(false);
                        if negate {
                            !is_exec
                        } else {
                            is_exec
                        }
                    });
                    negate = false;
                }
                '%' => {
                    // Device files
                    use std::os::unix::fs::FileTypeExt;
                    let next = chars.peek().copied();
                    result.retain(|f| {
                        let is_device = meta_cache
                            .get(f)
                            .and_then(|(_, sm)| sm.as_ref())
                            .map(|m| match next {
                                Some('b') => m.file_type().is_block_device(),
                                Some('c') => m.file_type().is_char_device(),
                                _ => {
                                    m.file_type().is_block_device()
                                        || m.file_type().is_char_device()
                                }
                            })
                            .unwrap_or(false);
                        if negate {
                            !is_device
                        } else {
                            is_device
                        }
                    });
                    if next == Some('b') || next == Some('c') {
                        chars.next();
                    }
                    negate = false;
                }

                // L[+-]N[k|m|g|p] — size qualifier. Default unit is 512-byte
                // blocks; suffix 'k'/'K' = kilobytes, 'm'/'M' = megabytes,
                // 'g'/'G' = gigabytes, 'p'/'P' = bytes (POSIX). +N matches
                // larger, -N smaller, N matches exactly. e.g. L0 = exactly
                // 0 bytes; L+10k = larger than 10 KB.
                'L' => {
                    let mut cmp = '=';
                    if let Some(&peek) = chars.peek() {
                        if peek == '+' || peek == '-' {
                            cmp = peek;
                            chars.next();
                        }
                    }
                    let mut num_str = String::new();
                    while let Some(&peek) = chars.peek() {
                        if peek.is_ascii_digit() {
                            num_str.push(peek);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let n: u64 = num_str.parse().unwrap_or(0);
                    let unit_mult: u64 = match chars.peek().copied() {
                        Some('k') | Some('K') => {
                            chars.next();
                            1024
                        }
                        Some('m') | Some('M') => {
                            chars.next();
                            1024 * 1024
                        }
                        Some('g') | Some('G') => {
                            chars.next();
                            1024 * 1024 * 1024
                        }
                        Some('p') | Some('P') => {
                            chars.next();
                            1
                        }
                        // zsh's default for L is BYTES (not 512-byte
                        // blocks). `(L+3)` means "more than 3 bytes".
                        _ => 1,
                    };
                    let target = n * unit_mult;
                    result.retain(|f| {
                        // zsh's L qualifier uses lstat size —
                        // for symlinks, that's the path-string
                        // length (NOT the target's size).
                        // Direct port: prefer the symlink
                        // metadata `sm` when present, fall
                        // back to the followed metadata.
                        let size = meta_cache
                            .get(f)
                            .map(|(m, sm)| {
                                sm.as_ref()
                                    .map(|m| m.len())
                                    .unwrap_or_else(|| m.as_ref().map(|m| m.len()).unwrap_or(0))
                            })
                            .unwrap_or(0);
                        let pass = match cmp {
                            '+' => size > target,
                            '-' => size < target,
                            _ => size == target,
                        };
                        if negate {
                            !pass
                        } else {
                            pass
                        }
                    });
                    negate = false;
                }

                // l[+-]N — link-count qualifier. zsh: `*(l2)` = files
                // with exactly 2 hard links (e.g. one regular + one
                // hardlink). `+N` matches more, `-N` matches fewer.
                'l' => {
                    let mut cmp = '=';
                    if let Some(&peek) = chars.peek() {
                        if peek == '+' || peek == '-' {
                            cmp = peek;
                            chars.next();
                        }
                    }
                    let mut num_str = String::new();
                    while let Some(&peek) = chars.peek() {
                        if peek.is_ascii_digit() {
                            num_str.push(peek);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let target: u64 = num_str.parse().unwrap_or(0);
                    use std::os::unix::fs::MetadataExt;
                    result.retain(|f| {
                        let nlink = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.nlink())
                            .unwrap_or(0);
                        let matches = match cmp {
                            '+' => nlink > target,
                            '-' => nlink < target,
                            _ => nlink == target,
                        };
                        if negate {
                            !matches
                        } else {
                            matches
                        }
                    });
                    negate = false;
                }

                // Permission qualifiers — all use prefetched metadata cache
                'r' => {
                    result = self.filter_by_permission(result, 0o400, negate, &meta_cache);
                    negate = false;
                }
                'w' => {
                    result = self.filter_by_permission(result, 0o200, negate, &meta_cache);
                    negate = false;
                }
                'x' => {
                    result = self.filter_by_permission(result, 0o100, negate, &meta_cache);
                    negate = false;
                }
                'A' => {
                    result = self.filter_by_permission(result, 0o040, negate, &meta_cache);
                    negate = false;
                }
                'I' => {
                    result = self.filter_by_permission(result, 0o020, negate, &meta_cache);
                    negate = false;
                }
                'E' => {
                    result = self.filter_by_permission(result, 0o010, negate, &meta_cache);
                    negate = false;
                }
                'R' => {
                    result = self.filter_by_permission(result, 0o004, negate, &meta_cache);
                    negate = false;
                }
                'W' => {
                    result = self.filter_by_permission(result, 0o002, negate, &meta_cache);
                    negate = false;
                }
                'X' => {
                    result = self.filter_by_permission(result, 0o001, negate, &meta_cache);
                    negate = false;
                }
                's' => {
                    result = self.filter_by_permission(result, 0o4000, negate, &meta_cache);
                    negate = false;
                }
                'S' => {
                    result = self.filter_by_permission(result, 0o2000, negate, &meta_cache);
                    negate = false;
                }
                't' => {
                    result = self.filter_by_permission(result, 0o1000, negate, &meta_cache);
                    negate = false;
                }

                // Full/empty directories
                'F' => {
                    // Non-empty directories
                    result.retain(|f| {
                        let path = std::path::Path::new(f);
                        let is_nonempty = path.is_dir()
                            && std::fs::read_dir(path)
                                .map(|mut d| d.next().is_some())
                                .unwrap_or(false);
                        if negate {
                            !is_nonempty
                        } else {
                            is_nonempty
                        }
                    });
                    negate = false;
                }

                // Ownership — uses prefetched metadata cache
                'U' => {
                    // Owned by effective UID
                    let euid = unsafe { libc::geteuid() };
                    result.retain(|f| {
                        use std::os::unix::fs::MetadataExt;
                        let is_owned = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.uid() == euid)
                            .unwrap_or(false);
                        if negate {
                            !is_owned
                        } else {
                            is_owned
                        }
                    });
                    negate = false;
                }
                'G' => {
                    // Owned by effective GID
                    let egid = unsafe { libc::getegid() };
                    result.retain(|f| {
                        use std::os::unix::fs::MetadataExt;
                        let is_owned = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.gid() == egid)
                            .unwrap_or(false);
                        if negate {
                            !is_owned
                        } else {
                            is_owned
                        }
                    });
                    negate = false;
                }

                // Sorting modifiers
                'o' => {
                    // Sort by name (ascending) - already default
                    if chars.peek() == Some(&'n') {
                        chars.next();
                        // Sort by name
                        result.sort();
                    } else if chars.peek() == Some(&'L') {
                        chars.next();
                        // Sort by size — uses prefetched metadata
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.len())
                                .unwrap_or(0)
                        });
                    } else if chars.peek() == Some(&'m') {
                        chars.next();
                        // zsh: `om` orders by modification time NEWEST
                        // FIRST (the time qualifiers default to
                        // descending; `Om` reverses to oldest-first).
                        // Was sorting ascending which inverted output.
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.modified().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                        result.reverse();
                    } else if chars.peek() == Some(&'a') {
                        chars.next();
                        // Same time-default-descending for atime.
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.accessed().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                        result.reverse();
                    } else if chars.peek() == Some(&'c') {
                        chars.next();
                        // ctime — same default-descending semantics.
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| {
                                    use std::os::unix::fs::MetadataExt;
                                    std::time::UNIX_EPOCH
                                        + std::time::Duration::from_secs(m.ctime() as u64)
                                })
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                        result.reverse();
                    }
                }
                'O' => {
                    // Reverse sort — uses prefetched metadata
                    if chars.peek() == Some(&'n') {
                        chars.next();
                        result.sort();
                        result.reverse();
                    } else if chars.peek() == Some(&'L') {
                        chars.next();
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.len())
                                .unwrap_or(0)
                        });
                        result.reverse();
                    } else if chars.peek() == Some(&'m') {
                        chars.next();
                        // `Om` flips the default time-descending — so
                        // `Om` is oldest-first. Just sort ascending.
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.modified().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                    } else {
                        // Just reverse current order
                        result.reverse();
                    }
                }

                // Subscript range [n] or [n,m]
                '[' => {
                    let mut range_str = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == ']' {
                            chars.next();
                            break;
                        }
                        range_str.push(chars.next().unwrap());
                    }

                    if let Some((start, end)) = self.parse_subscript_range(&range_str, result.len())
                    {
                        result = result.into_iter().skip(start).take(end - start).collect();
                    }
                }

                // Depth limit (for **/)
                'D' => {
                    // Include dotfiles (handled by dotglob)
                }
                'N' => {
                    // Nullglob for this pattern
                }

                // Time qualifiers `m` (mtime), `a` (atime), `c` (ctime).
                // Format: <qual><unit><op><N> e.g. `mh-100` =
                //   mtime within last 100 hours. Units: s (sec), m (min,
                //   default), h (hour), d (day, default for none),
                //   w (week), M (month, 30d). Ops: `+N` = older than,
                //   `-N` = newer than, no op = exactly N (within ±1 unit).
                'm' | 'a' | 'c' => {
                    let qual_kind = c;
                    // Unit (optional, default = days)
                    let unit_secs: i64 = match chars.peek().copied() {
                        Some('s') => {
                            chars.next();
                            1
                        }
                        Some('m') => {
                            chars.next();
                            60
                        }
                        Some('h') => {
                            chars.next();
                            3600
                        }
                        Some('d') => {
                            chars.next();
                            86400
                        }
                        Some('w') => {
                            chars.next();
                            7 * 86400
                        }
                        Some('M') => {
                            chars.next();
                            30 * 86400
                        }
                        _ => 86400,
                    };
                    // Op (optional, default = exact)
                    let op = match chars.peek().copied() {
                        Some('+') => {
                            chars.next();
                            '+'
                        }
                        Some('-') => {
                            chars.next();
                            '-'
                        }
                        _ => '=',
                    };
                    // Numeric value
                    let mut nstr = String::new();
                    while let Some(&nc) = chars.peek() {
                        if nc.is_ascii_digit() {
                            nstr.push(nc);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let n: i64 = nstr.parse().unwrap_or(0);
                    let cutoff = n * unit_secs;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    use std::os::unix::fs::MetadataExt;
                    result.retain(|f| {
                        let m = match meta_cache.get(f).and_then(|(m, _)| m.as_ref()) {
                            Some(m) => m,
                            None => return false,
                        };
                        let ts = match qual_kind {
                            'm' => m.mtime(),
                            'a' => m.atime(),
                            'c' => m.ctime(),
                            _ => 0,
                        };
                        let age = now - ts;
                        let pass = match op {
                            '+' => age > cutoff,
                            '-' => age < cutoff,
                            _ => age >= cutoff && age < cutoff + unit_secs,
                        };
                        if negate {
                            !pass
                        } else {
                            pass
                        }
                    });
                    negate = false;
                }

                // Unknown qualifier - ignore
                _ => {}
            }
        }

        // Apply (M) / (T) marking AFTER all filters have run. Direct
        // port of zsh/Src/glob.c:355,372 — output emit consults
        // gf_markdirs / gf_listtypes set by case 'M' / case 'T'.
        if mark_dirs || list_types {
            use std::os::unix::fs::PermissionsExt;
            result = result
                .into_iter()
                .map(|p| {
                    let meta = match std::fs::symlink_metadata(&p) {
                        Ok(m) => m,
                        Err(_) => return p,
                    };
                    let ch = crate::glob::file_type(meta.permissions().mode());
                    if list_types || (mark_dirs && ch == '/') {
                        format!("{}{}", p, ch)
                    } else {
                        p
                    }
                })
                .collect();
        }

        result
    }
    pub(crate) fn matches_pattern(&self, value: &str, pattern: &str) -> bool {
        // Simple glob matching
        if pattern == "*" {
            return true;
        }
        if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
            // Use glob matching for wildcards and character classes
            glob::Pattern::new(pattern)
                .map(|p| p.matches(value))
                .unwrap_or(false)
        } else {
            value == pattern
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/glob.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Filter file list by glob qualifiers
    /// Prefetch file metadata in parallel across the worker pool.
    /// Returns a map from path → (metadata, symlink_metadata).
    /// Each batch of files is stat'd on a pool thread.
    pub(crate) fn prefetch_metadata(
        &self,
        files: &[String],
    ) -> HashMap<String, (Option<std::fs::Metadata>, Option<std::fs::Metadata>)> {
        // After fork(), the worker pool's threads don't survive (POSIX:
        // only the calling thread persists). Pipeline children would
        // submit work that never gets picked up, blocking forever or
        // returning empty. Detect via pid mismatch with the original
        // main pid; use serial when forked.
        let in_forked_child = crate::signals::ProcId::is_forked_child();
        if files.len() < 32 || in_forked_child {
            // Small list OR forked child — serial stat is the only
            // safe path.
            return files
                .iter()
                .map(|f| {
                    let meta = std::fs::metadata(f).ok();
                    let symlink_meta = std::fs::symlink_metadata(f).ok();
                    (f.clone(), (meta, symlink_meta))
                })
                .collect();
        }

        let pool_size = self.worker_pool.size();
        let chunk_size = files.len().div_ceil(pool_size);
        let (tx, rx) = std::sync::mpsc::channel();

        for chunk in files.chunks(chunk_size) {
            let tx = tx.clone();
            let chunk: Vec<String> = chunk.to_vec();
            self.worker_pool.submit(move || {
                #[allow(clippy::type_complexity)]
                let batch: Vec<(
                    String,
                    (Option<std::fs::Metadata>, Option<std::fs::Metadata>),
                )> = chunk
                    .into_iter()
                    .map(|f| {
                        let meta = std::fs::metadata(&f).ok();
                        let symlink_meta = std::fs::symlink_metadata(&f).ok();
                        (f, (meta, symlink_meta))
                    })
                    .collect();
                let _ = tx.send(batch);
            });
        }
        drop(tx);

        let mut map = HashMap::with_capacity(files.len());
        for batch in rx {
            for (path, metas) in batch {
                map.insert(path, metas);
            }
        }
        map
    }
    /// Filter files by permission bits — uses prefetched metadata cache
    pub(crate) fn filter_by_permission(
        &self,
        files: Vec<String>,
        mode: u32,
        negate: bool,
        meta_cache: &HashMap<String, (Option<std::fs::Metadata>, Option<std::fs::Metadata>)>,
    ) -> Vec<String> {
        use std::os::unix::fs::PermissionsExt;
        files
            .into_iter()
            .filter(|f| {
                let has_perm = meta_cache
                    .get(f)
                    .and_then(|(m, _)| m.as_ref())
                    .map(|m| (m.permissions().mode() & mode) != 0)
                    .unwrap_or(false);
                if negate {
                    !has_perm
                } else {
                    has_perm
                }
            })
            .collect()
    }
}

// =====================================================================
// MOVED FROM: src/ported/utils.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    pub(crate) fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
        if !dest.exists() {
            std::fs::create_dir_all(dest)?;
        }
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());

            if file_type.is_dir() {
                Self::copy_dir_recursive(&src_path, &dest_path)?;
            } else {
                std::fs::copy(&src_path, &dest_path)?;
            }
        }
        Ok(())
    }
}

// =====================================================================
// MOVED FROM: src/ported/zle/compcore.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// zsh compadd - add completion matches
    pub(crate) fn bin_compadd(&mut self, args: &[String]) -> i32 {
        // Basic stub for zsh completion system
        // In a full implementation, this would add completion candidates
        let _ = args;
        0
    }
    /// zsh compset - modify completion prefix/suffix
    pub(crate) fn bin_compset(&mut self, args: &[String]) -> i32 {
        // Basic stub for zsh completion system
        let _ = args;
        0
    }
}

// =====================================================================
// MOVED FROM: src/ported/zle/computil.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// compquote — shell-bslashquote the value of each named parameter.
    /// Direct port of zsh/Src/Zle/computil.c:3679 bin_compquote.
    /// Walks each arg as a parameter name, replaces scalar values
    /// with comp_quote(value); for arrays, quotes each element.
    /// `-p` flag enables param-substitution-context quoting (handled
    /// the same way by shell_quote_value, which is conservative).
    pub(crate) fn bin_compquote(&mut self, args: &[String]) -> i32 {
        // computil.c:3691-3692 — early-out when there's nothing to
        // bslashquote (no nested completion stack). zshrs has no compqstack
        // surfaced through the VM yet; mimic the no-op by still doing
        // the bslashquote so user code that calls compquote gets a value.
        let mut returnval = 0;
        for raw in args {
            let name = raw.trim_start_matches('-');
            if name.is_empty() {
                continue;
            }
            if let Some(arr) = self.arrays.get(name).cloned() {
                let quoted: Vec<String> = arr.iter().map(|v| quotedzputs(v)).collect();
                self.arrays.insert(name.to_string(), quoted);
            } else if let Some(val) = self.variables.get(name).cloned() {
                self.variables
                    .insert(name.to_string(), quotedzputs(&val));
            } else {
                zwarnnam("compquote", &format!("unknown parameter: {}", name));
                returnval = 1;
            }
        }
        returnval
    }
    /// comptags - manage completion tags
    pub(crate) fn bin_comptags(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            return 1;
        }
        match args[0].as_str() {
            "-i" => {
                // Initialize tags
                0
            }
            "-S" => {
                // Set tags
                0
            }
            _ => 1,
        }
    }
    /// comptry - try completion
    pub(crate) fn bin_comptry(&mut self, _args: &[String]) -> i32 {
        1 // No match
    }
    /// compvalues - complete values
    pub(crate) fn bin_compvalues(&mut self, _args: &[String]) -> i32 {
        0
    }
}

// =====================================================================
// MOVED FROM: src/ported/zle/zle_main.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// bindkey - key binding management
    pub(crate) fn bin_bindkey(&mut self, args: &[String]) -> i32 {
        use crate::zle::{zle, KeymapName};

        // PFA-SMR aspect: emit one `bindkey` event per real binding. The
        // last two non-flag args are (sequence, widget); -M MAP / -A NEW
        // OLD / list flags don't bind a new key.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let mut keymap: Option<&str> = None;
            let mut listing_only = args.is_empty();
            let mut positional: Vec<&str> = Vec::new();
            let mut iter = args.iter().peekable();
            while let Some(a) = iter.next() {
                match a.as_str() {
                    "-M" | "-A" | "-N" | "-R" => {
                        keymap = iter.next().map(String::as_str);
                    }
                    "-l" | "-L" | "-d" | "-r" | "-e" | "-v" => listing_only = true,
                    s if s.starts_with('-') => {}
                    _ => positional.push(a.as_str()),
                }
            }
            if !listing_only && positional.len() >= 2 {
                let ctx = self.recorder_ctx();
                let seq = positional[positional.len() - 2];
                let widget = positional[positional.len() - 1];
                let value = match keymap {
                    Some(km) => format!("[{}] {}", km, widget),
                    None => widget.to_string(),
                };
                crate::recorder::emit_bindkey(seq, &value, ctx);
            }
        }

        if args.is_empty() {
            // List all bindings in main keymap
            let zle = zle();
            for (keys, widget) in zle
                .keymaps
                .get(&KeymapName::Main)
                .map(|km| km.list_bindings().collect::<Vec<_>>())
                .unwrap_or_default()
            {
                println!("\"{}\" {}", keys, widget);
            }
            return 0;
        }

        let mut iter = args.iter().peekable();
        let mut keymap = KeymapName::Main;
        let mut list_mode = false;
        let mut list_all = false;
        let mut remove = false;

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-l" => {
                    list_mode = true;
                }
                "-L" => {
                    list_mode = true;
                    list_all = true;
                }
                "-la" | "-lL" => {
                    list_mode = true;
                    list_all = true;
                }
                "-M" => {
                    if let Some(name) = iter.next() {
                        if let Some(km) = KeymapName::from_str(name) {
                            keymap = km;
                        }
                    }
                }
                "-r" => {
                    remove = true;
                }
                "-A" => {
                    // bindkey -A NEW EXISTING — link NEW to EXISTING
                    // so reads of NEW's bindings see EXISTING's table.
                    // Direct port of zle_keymap.c bin_bindkey case 'A':
                    // copy EXISTING's bindings into NEW's slot. With
                    // fewer than two args zsh errors 'not enough
                    // arguments for -A' exit 1.
                    let new = match iter.next() {
                        Some(s) => s.clone(),
                        None => {
                            zwarnnam("bindkey", "not enough arguments for -A");
                            return 1;
                        }
                    };
                    let existing = match iter.next() {
                        Some(s) => s.clone(),
                        None => {
                            zwarnnam("bindkey", "not enough arguments for -A");
                            return 1;
                        }
                    };
                    let map_name = |s: &str| match s {
                        "main" => Some(KeymapName::Main),
                        "emacs" => Some(KeymapName::Emacs),
                        "viins" => Some(KeymapName::ViInsert),
                        "vicmd" => Some(KeymapName::ViCommand),
                        "isearch" => Some(KeymapName::Isearch),
                        "command" => Some(KeymapName::Command),
                        "menuselect" => Some(KeymapName::MenuSelect),
                        _ => None,
                    };
                    let new_km = match map_name(&new) {
                        Some(k) => k,
                        None => {
                            zwarnnam("bindkey", &format!("no such keymap: {}", new));
                            return 1;
                        }
                    };
                    let src_km = match map_name(&existing) {
                        Some(k) => k,
                        None => {
                            zwarnnam("bindkey", &format!("no such keymap: {}", existing));
                            return 1;
                        }
                    };
                    let mut zle = zle();
                    let snapshot = zle.keymaps.get(&src_km).cloned();
                    if let Some(km) = snapshot {
                        zle.keymaps.insert(new_km, km);
                        return 0;
                    }
                    zwarnnam("bindkey", &format!("no such keymap: {}", existing));
                    return 1;
                }
                "-d" => {
                    // bindkey -d: reset all keymaps to defaults.
                    // Direct port of zle_keymap.c bin_bindkey case
                    // 'd': delete every existing keymap and recreate
                    // the canonical six (main/emacs/viins/vicmd/
                    // isearch/command/menuselect) with their factory
                    // bindings. ZleManager::new() reproduces exactly
                    // those six, so swap the manager state.
                    let mut zle = zle();
                    let preserved_widgets = std::mem::take(&mut zle.user_widgets);
                    let active = zle.active_keymap;
                    *zle = crate::zle::ZleManager::new();
                    zle.user_widgets = preserved_widgets;
                    zle.active_keymap = active;
                    return 0;
                }
                "-N" => {
                    // Create new keymap - stub
                    return 0;
                }
                "-e" => {
                    keymap = KeymapName::Emacs;
                }
                "-v" => {
                    keymap = KeymapName::ViInsert;
                }
                "-a" => {
                    keymap = KeymapName::ViCommand;
                }
                key if !key.starts_with('-') => {
                    // Key sequence - next arg is widget
                    if let Some(widget) = iter.next() {
                        let mut zle = zle();
                        if remove {
                            zle.unbind_key(keymap, key);
                        } else {
                            zle.bind_key(keymap, key, widget);
                        }
                    }
                    return 0;
                }
                // zsh: unknown bindkey flag errors `bindkey:1: bad
                // option: -X` exit 1. zshrs's silent fallback let
                // unknown flags drop into list-mode silently.
                other => {
                    let bad: String = other.chars().skip(1).take(1).collect();
                    zwarnnam("bindkey", &format!("bad option: -{}", bad));
                    return 1;
                }
            }
        }

        if list_mode {
            let zle = zle();
            if list_all {
                // Direct port of zsh/Src/Zle/zle_keymap.c bin_bindkey
                // case 'l': enumerate every keymap registered, sorted
                // by canonical name. Was a hardcoded 3-name subset
                // missing main/isearch/command/menuselect.
                for km_name in &[
                    KeymapName::Main,
                    KeymapName::Emacs,
                    KeymapName::ViInsert,
                    KeymapName::ViCommand,
                    KeymapName::Isearch,
                    KeymapName::Command,
                    KeymapName::MenuSelect,
                ] {
                    if zle.keymaps.contains_key(km_name) {
                        println!("{}", km_name.as_str());
                    }
                }
            } else {
                if let Some(km) = zle.keymaps.get(&keymap) {
                    for (keys, widget) in km.list_bindings() {
                        println!("bindkey \"{}\" {}", keys, widget);
                    }
                }
            }
        }

        0
    }
    /// zle - line editor control
    pub(crate) fn bin_zle(&mut self, args: &[String]) -> i32 {
        use crate::zle::zle;

        if args.is_empty() {
            return 0;
        }

        let mut iter = args.iter().peekable();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-l" => {
                    // zsh: in non-interactive mode (`-c`), the ZLE
                    // module is not loaded, so `zle -l` outputs
                    // nothing and returns 0. zshrs eagerly preloads
                    // its built-in widget table, so the listing fired
                    // even in scripts — diverging from zsh's silent
                    // empty output. Match zsh by returning 0 with no
                    // listing when stdin is not a tty.
                    if !atty::is(atty::Stream::Stdin) {
                        return 0;
                    }
                    let zle = zle();
                    let mut widgets: Vec<&str> = zle.list_widgets();
                    widgets.sort();
                    for w in widgets {
                        println!("{}", w);
                    }
                    return 0;
                }
                "-la" | "-lL" => {
                    // Same non-tty silence rule as bare `-l`.
                    if !atty::is(atty::Stream::Stdin) {
                        return 0;
                    }
                    let zle = zle();
                    let mut widgets: Vec<&str> = zle.list_widgets();
                    widgets.sort();
                    for w in widgets {
                        println!("{}", w);
                    }
                    return 0;
                }
                "-N" => {
                    // Define new widget: zle -N widget-name [function]
                    if let Some(widget_name) = iter.next() {
                        let func_name = iter
                            .next()
                            .map(|s| s.as_str())
                            .unwrap_or(widget_name.as_str());
                        let mut zle = zle();
                        zle.define_widget(widget_name, func_name);
                        // PFA-SMR aspect: ZLE widget definition.
                        // zinit-report lists these (along with bindkey
                        // and zstyle); the recorder gives them a
                        // dedicated `zle` kind so query-side can
                        // surface "every widget this plugin installed".
                        // Value field carries the underlying handler
                        // function name (defaults to the widget name
                        // for self-bound widgets).
                        #[cfg(feature = "recorder")]
                        if crate::recorder::is_enabled() {
                            let ctx = self.recorder_ctx();
                            crate::recorder::emit_zle(widget_name, Some(func_name), ctx);
                        }
                    }
                    return 0;
                }
                "-D" => {
                    // zle -D widget...: delete one or more user
                    // widgets. zsh: missing target -> exit 1 silently;
                    // unknown widget -> 'no such widget: NAME' exit 1.
                    let mut returnval = 0;
                    let mut had_arg = false;
                    let mut zle = zle();
                    for name in iter.by_ref() {
                        had_arg = true;
                        if !zle.delete_widget(name) {
                            zwarnnam("zle", &format!("no such widget: {}", name));
                            returnval = 1;
                        }
                    }
                    if !had_arg {
                        return 1;
                    }
                    return returnval;
                }
                "-A" => {
                    // zle -A old new: alias `new` to dispatch as `old`.
                    // zsh: needs exactly two args; unknown source ->
                    // 'no such widget: OLD' exit 1.
                    let old = match iter.next() {
                        Some(s) => s.clone(),
                        None => {
                            zwarnnam("zle", "-A requires source widget");
                            return 1;
                        }
                    };
                    let new = match iter.next() {
                        Some(s) => s.clone(),
                        None => {
                            zwarnnam("zle", "-A requires destination widget");
                            return 1;
                        }
                    };
                    let mut zle = zle();
                    if !zle.alias_widget(&new, &old) {
                        zwarnnam("zle", &format!("no such widget: {}", old));
                        return 1;
                    }
                    // PFA-SMR aspect: ZLE widget alias `zle -A old new`.
                    // Recorder treats it as a `zle` event for `new` with
                    // value `old` so the alias relationship is queryable.
                    #[cfg(feature = "recorder")]
                    if crate::recorder::is_enabled() {
                        let ctx = self.recorder_ctx();
                        crate::recorder::emit_zle(&new, Some(&old), ctx);
                    }
                    return 0;
                }
                "-R" => {
                    // Port of bin_zle_refresh from Src/Zle/zle_thingy.c:418.
                    // The C source: "zle -R [-c] [STATUS [LIST...]]"
                    //   - Without -c or args: just rerun zrefresh.
                    //   - With STATUS: set the status line, then refresh.
                    //   - With LIST...: display the list below the prompt.
                    //   - With -c: clear the prior list before refresh.
                    // C errors with `not bound` when zleactive is false; we
                    // approximate that by silently no-oping (the bin holds
                    // the live ZLE session, which we don't reach from
                    // here). For consistency: parse remaining args
                    // (status + list elems) and discard, then return 0.
                    let mut clear = false;
                    let mut status: Option<String> = None;
                    let mut list_items: Vec<String> = Vec::new();
                    for arg in iter.by_ref() {
                        match arg.as_str() {
                            "-c" => clear = true,
                            s if status.is_none() => status = Some(s.to_string()),
                            s => list_items.push(s.to_string()),
                        }
                    }
                    let _ = (clear, status, list_items);
                    return 0;
                }
                "-U" => {
                    // Port of bin_zle_unget from Src/Zle/zle_thingy.c:473.
                    // The C source ungets each byte of args[0] back into
                    // the input stream. zsh errors when zleactive==0 with
                    // "can only be called from widget function". We don't
                    // hold the live ZLE state here; emit the same
                    // diagnostic + exit 1 instead of silently dropping.
                    if iter.next().is_none() {
                        zwarnnam("zle", "-U requires a string argument");
                        return 1;
                    }
                    zwarnnam("zle", "can only be called from widget function");
                    return 1;
                }
                "-K" => {
                    // zle -K NAME: select active keymap. zsh:
                    // unknown name -> 'no such keymap: NAME' exit 1.
                    let name = match iter.next() {
                        Some(s) => s.clone(),
                        None => {
                            zwarnnam("zle", "-K requires keymap name");
                            return 1;
                        }
                    };
                    let mut zle = zle();
                    if !zle.select_keymap(&name) {
                        zwarnnam("zle", &format!("no such keymap: {}", name));
                        return 1;
                    }
                    return 0;
                }
                "-F" => {
                    // Port of bin_zle_fd from Src/Zle/zle_thingy.c:857.
                    //   zle -F [-L|-w] [FD [HANDLER]]
                    // The C source tracks watch_fds globally so zselect
                    // can dispatch to user handlers when fds become
                    // readable. Without a live ZLE event loop, we still
                    // need to parse args correctly so -L (list) returns
                    // empty cleanly and add/remove forms validate the
                    // fd argument.
                    let mut list = false;
                    let mut widget_mode = false;
                    let mut fd_arg: Option<String> = None;
                    let mut handler: Option<String> = None;
                    for arg in iter.by_ref() {
                        match arg.as_str() {
                            "-L" => list = true,
                            "-w" => widget_mode = true,
                            s if fd_arg.is_none() => fd_arg = Some(s.to_string()),
                            s if handler.is_none() => handler = Some(s.to_string()),
                            _ => {
                                zwarnnam("zle", "too many arguments for -F");
                                return 1;
                            }
                        }
                    }
                    let _ = widget_mode;
                    // Validate fd if supplied (mirrors zle_thingy.c:865).
                    if let Some(ref s) = fd_arg {
                        match s.parse::<i32>() {
                            Ok(n) if n >= 0 => {}
                            _ => {
                                zwarnnam("zle", &format!("bad file descriptor number for -F: {}", s));
                                return 1;
                            }
                        }
                    }
                    // Listing path: no watch_fds tracked here → exit 0
                    // for empty list, exit 1 if a specific fd was asked
                    // for and "not found" (matches zle_thingy.c:886
                    // `*args && !found`).
                    if list || (fd_arg.is_some() && handler.is_none()) {
                        return if fd_arg.is_some() { 1 } else { 0 };
                    }
                    // Add/remove path: silently no-op since the watch
                    // dispatch lives in the ZLE main loop we don't run
                    // from script context. Future port will wire watch
                    // registration through a host-side hook.
                    return 0;
                }
                "-M" => {
                    // zle -M message: display a message in the editor
                    // status area. Outside ZLE we have no status line,
                    // so emit to stderr (matches zsh's non-interactive
                    // fallback at zle_main.c:bin_zle 'M' branch).
                    if let Some(msg) = iter.next() {
                        eprintln!("{}", msg);
                    }
                    return 0;
                }
                "-I" => {
                    // Port of bin_zle_invalidate from Src/Zle/zle_thingy.c:830.
                    // The C source: if zleactive, calls trashzle() to move
                    // past the prompt and arms fetchttyinfo for a
                    // settyinfo restore on next zsetterm; if zleactive==0
                    // it returns 1. Without a live ZLE here we always
                    // take the inactive branch — return 1 to mirror
                    // zsh's exit status when no live editor is up.
                    return 1;
                }
                "-C" => {
                    // Port of bin_zle_complete from Src/Zle/zle_thingy.c:600.
                    //   zle -C completion-name builtin-widget shell-fn
                    // Defines a *completion* widget — a user-named widget
                    // that wraps a built-in completion widget but runs a
                    // shell function for the actual match generation.
                    // Used to define plugin completion widgets:
                    //   zle -C my-comp expand-or-complete _my_comp_fn
                    let name = match iter.next() {
                        Some(s) => s.clone(),
                        None => {
                            zwarnnam("zle", "-C requires a name");
                            return 1;
                        }
                    };
                    let target = match iter.next() {
                        Some(s) => s.clone(),
                        None => {
                            zwarnnam("zle", "-C requires a target completion widget");
                            return 1;
                        }
                    };
                    let func = match iter.next() {
                        Some(s) => s.clone(),
                        None => {
                            zwarnnam("zle", "-C requires a shell function");
                            return 1;
                        }
                    };
                    // The C source validates that `target` is a
                    // completion widget (ZLE_ISCOMP flag). Our widget
                    // table doesn't carry that flag yet — accept any
                    // target and register the user widget pointing to
                    // the func. zinit's _zinit and compsys's _* widgets
                    // depend on this for plugin loading.
                    let mut zle = zle();
                    zle.define_widget(&name, &func);
                    let _ = target; // referenced for future ZLE_ISCOMP check
                    return 0;
                }
                "-f" => {
                    // Check widget exists
                    if let Some(name) = iter.next() {
                        let zle = zle();
                        return if zle.get_widget(name).is_some() { 0 } else { 1 };
                    }
                    return 1;
                }
                widget_name if !widget_name.starts_with('-') => {
                    // Call widget
                    let mut zle = zle();
                    match zle.execute_widget(widget_name, None) {
                        crate::zle::WidgetResult::Ok => return 0,
                        crate::zle::WidgetResult::Error(e) => {
                            zwarnnam("zle", &format!("{}", e));
                            return 1;
                        }
                        crate::zle::WidgetResult::CallFunction(func) => {
                            // Call user widget through compiled-function dispatch.
                            drop(zle);
                            if let Some(status) = self.dispatch_function_call(&func, &[]) {
                                return status;
                            }
                            return 1;
                        }
                        _ => return 0,
                    }
                }
                _ => {}
            }
        }

        0
    }
    /// `vared` shim — parses the `"AaceghM:m:p:r:i:f:"` BUILTIN spec
    /// from zle_main.c:2186 into a real `options` struct, then invokes
    /// the canonical free-fn port at
    /// crate::ported::zle::zle_main::bin_vared which matches the C
    /// signature `bin_vared(name, args, ops, func)` exactly.
    pub(crate) fn bin_vared(&mut self, args: &[String]) -> i32 {
        use crate::ported::zsh_h::{options, MAX_OPS};
        let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                argscount: 0, argsalloc: 0 };
        let mut positional: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a == "--" { i += 1; positional.extend_from_slice(&args[i..]); break; }
            if let Some(rest) = a.strip_prefix('-') {
                if rest.is_empty() { positional.push(a.clone()); i += 1; continue; }
                let chars: Vec<char> = rest.chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    let c = chars[j] as u8;
                    // -M / -m / -p / -r / -i / -f all take an arg per the spec.
                    if matches!(c, b'M' | b'm' | b'p' | b'r' | b'i' | b'f') {
                        ops.ind[c as usize] = (ops.args.len() + 1) as u8;
                        let rest_after = &rest[j + 1..];
                        if !rest_after.is_empty() {
                            ops.args.push(rest_after.to_string());
                        } else {
                            i += 1;
                            ops.args.push(args.get(i).cloned().unwrap_or_default());
                        }
                        ops.argscount = ops.args.len() as i32;
                        break;
                    }
                    if c.is_ascii_alphabetic() { ops.ind[c as usize] = 1; }
                    j += 1;
                }
            } else {
                positional.push(a.clone());
            }
            i += 1;
        }
        crate::ported::zle::zle_main::bin_vared("vared", &positional, &ops, 0)
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/cap.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// `cap` builtin entry. Bridge to `bin_cap()` above.
    pub(crate) fn bin_cap(&self, args: &[String]) -> i32 {
        let ops = options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 };
        bin_cap("cap", args, &ops, 0)
    }

    /// `getcap` builtin entry. Bridge to `bin_getcap()` above.
    pub(crate) fn bin_getcap(&self, args: &[String]) -> i32 {
        let ops = options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 };
        bin_getcap("getcap", args, &ops, 0)
    }

    /// `setcap` builtin entry. Bridge to `bin_setcap()` above.
    pub(crate) fn bin_setcap(&self, args: &[String]) -> i32 {
        let ops = options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 };
        bin_setcap("setcap", args, &ops, 0)
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/zpty.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// `zpty` builtin — delegates to canonical port at
    /// `src/ported/modules/zpty.rs:367` (`bin_zpty()` from
    /// `Src/Modules/zpty.c`). The named-pty table lives on
    /// `ShellExecutor` so `zpty -w NAME ...` and `zpty -r NAME` can
    /// reach a session started by an earlier `zpty NAME ...` call.
    pub(crate) fn bin_zpty(&mut self, args: &[String]) -> i32 {
        use crate::zpty::ZptyOptions;
        let mut options = ZptyOptions::default();
        let mut positional: Vec<&str> = Vec::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-d" => options.delete = true,
                "-L" => options.list = true,
                "-w" => options.write = true,
                "-r" => {
                    if let Some(s) = iter.next() {
                        options.read_var = Some(s.clone());
                    }
                }
                "-e" => options.echo = true,
                "-t" => options.test = true,
                "-b" => options.block = true,
                "-m" => {
                    if let Some(s) = iter.next() {
                        options.pattern = Some(s.clone());
                    }
                }
                "-T" => {
                    if let Some(s) = iter.next() {
                        options.timeout = s.parse().ok();
                    }
                }
                _ => positional.push(arg.as_str()),
            }
        }
        let (status, output) = crate::zpty::bin_zpty(
            &positional, &options, &mut self.pty_cmds,
        );
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/terminfo.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// echoti - output terminfo value
    pub(crate) fn bin_echoti(&mut self, args: &[String]) -> i32 {
        // echoti uses TERMINFO names ('clear', 'home', 'el', etc.)
        // not termcap two-letter codes. Translate the common terminfo
        // names to their termcap equivalents and dispatch through
        // bin_echotc which already handles the ANSI emit. Direct
        // port of zsh/Src/Modules/terminfo.c bin_echoti's
        // tparm-style path with the canonical mapping below.
        if args.is_empty() {
            zwarnnam("echoti", "not enough arguments");
            return 1;
        }
        let cap = args[0].as_str();
        // terminfo → termcap two-letter mapping (most-used subset).
        let mapped = match cap {
            "clear" => "cl",
            "ed" => "cd",  // clear to end of display
            "el" => "ce",  // clear to end of line
            "cup" => "cm", // cursor position (with row, col)
            "cuu1" => "up",
            "cud1" => "do",
            "cub1" => "le",
            "cuf1" => "nd",
            "home" => "ho",
            "civis" => "vi",
            "cnorm" => "ve",
            "smso" => "so",
            "rmso" => "se",
            "smul" => "us",
            "rmul" => "ue",
            "bold" => "md",
            "sgr0" => "me",
            "rev" => "mr",
            "setaf" => "AF",
            "setab" => "AB",
            "colors" => "Co",
            "cols" => "co",
            "lines" => "li",
            other => other, // pass through unknown names; echotc rejects
        };
        let mut new_args = vec![mapped.to_string()];
        new_args.extend(args[1..].iter().cloned());
        self.bin_echotc(&new_args)
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/watch.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// `log` builtin — delegates to canonical port at
    /// `src/ported/modules/watch.rs` (`bin_log()` from
    /// `Src/Modules/watch.c`). The watch state lives in
    /// `thread_local!`s in the canonical port (mirroring C's
    /// `Src/Modules/watch.c:150-156` file-statics) so login/logout
    /// edge detection survives across calls without a struct on
    /// `ShellExecutor`.
    pub(crate) fn bin_log(&mut self, _args: &[String]) -> i32 {
        let user = std::env::var("USER").unwrap_or_default();
        let fmt = self.variables.get("WATCHFMT").cloned();
        let output = crate::watch::bin_log(&user, fmt.as_deref());
        if !output.is_empty() {
            print!("{}", output);
        }
        0
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/pcre.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// pcre_compile - compile a PCRE pattern
    /// `pcre_compile` builtin — delegates to canonical port at
    /// `src/ported/modules/pcre.rs:244` (`bin_pcre_compile()` from
    /// `Src/Modules/pcre.c:70`). All option parsing and pattern
    /// compilation now lives in the canonical port; this shim only
    /// builds the `&[&str]` view and threads `self.pcre_state`.
    pub(crate) fn bin_pcre_compile(&mut self, args: &[String]) -> i32 {
        use crate::pcre::PcreCompileOptions;
        let mut options = PcreCompileOptions::default();
        let mut positional: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-a" => options.anchored = true,
                "-i" => options.caseless = true,
                "-m" => options.multiline = true,
                "-s" => options.dotall = true,
                "-x" => options.extended = true,
                s if !s.starts_with('-') => positional.push(s),
                _ => {}
            }
        }
        let (status, output) = crate::pcre::bin_pcre_compile(&positional, &options);
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
    /// `pcre_match` builtin — delegates to canonical port at
    /// `src/ported/modules/pcre.rs:273` (`bin_pcre_match()` from
    /// `Src/Modules/pcre.c:328`). The shim parses `-v`/`-a` argv
    /// flags, calls the canonical matcher, then writes the resulting
    /// `MATCH`/`match` capture data back into the executor's
    /// variable/array tables — that side-effect cannot live in the
    /// canonical port because it doesn't own those tables.
    pub(crate) fn bin_pcre_match(&mut self, args: &[String]) -> i32 {
        use crate::pcre::PcreMatchOptions;

        let mut var_name = "MATCH".to_string();
        let mut array_name = "match".to_string();
        let mut positional: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-v" => { i += 1; if i < args.len() { var_name = args[i].clone(); } }
                "-a" => { i += 1; if i < args.len() { array_name = args[i].clone(); } }
                s if !s.starts_with('-') => positional.push(s),
                _ => {}
            }
            i += 1;
        }

        let options = PcreMatchOptions {
            match_var: Some(var_name.clone()),
            array_var: Some(array_name.clone()),
            ..Default::default()
        };

        let (status, result) = crate::pcre::bin_pcre_match(&positional, &options);
        if status == 0 {
            if let Some(m) = result.full_match {
                self.variables.insert(var_name, m);
            }
            let matches: Vec<String> = result.captures.into_iter().flatten().collect();
            self.arrays.insert(array_name, matches);
        }
        status
    }
    /// pcre_study - optimize compiled PCRE (no-op in Rust regex)
    pub(crate) fn bin_pcre_study(&mut self, _args: &[String]) -> i32 {
        let (status, msg) = crate::pcre::bin_pcre_study();
        if status != 0 {
            zwarnnam("pcre_study", msg.trim_start_matches("pcre_study: ").trim_end());
        }
        status
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/tcp.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    pub(crate) fn bin_ztcp(&mut self, args: &[String]) -> i32 {
        use crate::ported::zsh_h::{options, MAX_OPS};
        let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),
                                argscount: 0, argsalloc: 0 };
        let mut positional: Vec<String> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a == "--" { i += 1; positional.extend_from_slice(&args[i..]); break; }
            if let Some(rest) = a.strip_prefix('-') {
                if rest.is_empty() { positional.push(a.clone()); i += 1; continue; }
                let chars: Vec<char> = rest.chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    let c = chars[j] as u8;
                    if c == b'd' {
                        // -d takes an arg: rest of token, or next argv.
                        ops.ind[c as usize] = (ops.args.len() + 1) as u8;
                        let rest_after = &rest[j + 1..];
                        if !rest_after.is_empty() {
                            ops.args.push(rest_after.to_string());
                        } else {
                            i += 1;
                            ops.args.push(args.get(i).cloned().unwrap_or_default());
                        }
                        ops.argscount = ops.args.len() as i32;
                        break;
                    }
                    if c.is_ascii_alphabetic() { ops.ind[c as usize] = 1; }
                    j += 1;
                }
            } else {
                positional.push(a.clone());
            }
            i += 1;
        }
        bin_ztcp("ztcp", &positional, &ops, 0)
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/db_gdbm.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// Tie a parameter to a GDBM database
    /// Usage: ztie -d db/gdbm -f /path/to/db.gdbm [-r] PARAM_NAME
    pub(crate) fn bin_ztie(&mut self, args: &[String]) -> i32 {
        let mut db_type: Option<String> = None;
        let mut file_path: Option<String> = None;
        let mut readonly = false;
        let mut param_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-d" => {
                    if i + 1 < args.len() {
                        db_type = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        zwarnnam("ztie", "-d requires an argument");
                        return 1;
                    }
                }
                "-f" => {
                    if i + 1 < args.len() {
                        file_path = Some(args[i + 1].clone());
                        i += 2;
                    } else {
                        zwarnnam("ztie", "-f requires an argument");
                        return 1;
                    }
                }
                "-r" => {
                    readonly = true;
                    i += 1;
                }
                arg if arg.starts_with('-') => {
                    zwarnnam("ztie", &format!("bad option: {}", arg));
                    return 1;
                }
                _ => {
                    param_args.push(args[i].clone());
                    i += 1;
                }
            }
        }

        // Build the canonical `options` struct from the parsed flags
        // so `bin_ztie` (the C-faithful free fn) can read via OPT_ISSET
        // / OPT_ARG just like the C source.
        use crate::ported::zsh_h::{options, MAX_OPS};
        let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 };
        if readonly { ops.ind[b'r' as usize] = 1; }
        if let Some(d) = db_type {
            ops.ind[b'd' as usize] = (1 + ((ops.args.len() as u8 + 1) << 2)) | 1;
            ops.args.push(d);
        }
        if let Some(f) = file_path {
            ops.ind[b'f' as usize] = (1 + ((ops.args.len() as u8 + 1) << 2)) | 1;
            ops.args.push(f);
        }
        crate::ported::modules::db_gdbm::bin_ztie("ztie", &param_args, &ops, 0)
    }
    /// Untie a parameter from its GDBM database
    /// Usage: zuntie [-u] PARAM_NAME...
    pub(crate) fn bin_zuntie(&mut self, args: &[String]) -> i32 {
        use crate::ported::zsh_h::{options, MAX_OPS};

        let mut force_unset = false;
        let mut param_args: Vec<String> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-u" => force_unset = true,
                a if a.starts_with('-') => {
                    zwarnnam("zuntie", &format!("bad option: {}", a));
                    return 1;
                }
                _ => param_args.push(arg.clone()),
            }
        }

        if param_args.is_empty() {
            zwarnnam("zuntie", "not enough arguments");
            return 1;
        }

        // Build canonical `&options` from -u flag, then dispatch to the
        // C-faithful free fn.
        let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 };
        if force_unset { ops.ind[b'u' as usize] = 1; }
        crate::ported::modules::db_gdbm::bin_zuntie("zuntie", &param_args, &ops, 0)
    }
    /// Get the path of a tied GDBM database
    /// Usage: zgdbmpath PARAM_NAME
    /// Sets $REPLY to the path
    pub(crate) fn bin_zgdbmpath(&mut self, args: &[String]) -> i32 {
        use crate::ported::zsh_h::{options, MAX_OPS};

        // Build empty `&options` (zgdbmpath takes no flags), dispatch
        // to the C-faithful free fn. The free fn writes the path to
        // stdout (degraded $REPLY equivalent until params globalize);
        // bridge captures it into the executor's REPLY map.
        let ops = options { ind: [0u8; MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 };

        // Capture the path before dispatching so we can ALSO populate
        // the executor REPLY (the free fn's println is the diagnostic
        // form for non-bridged callers).
        if let Some(pmname) = args.first() {
            if let Ok(p) = crate::ported::modules::db_gdbm::TIED_PARAMS.lock() {
                if let Some(tied) = p.get(pmname) {
                    let path = tied.db.path().to_string_lossy().to_string();
                    self.variables.insert("REPLY".to_string(), path.clone());
                    std::env::set_var("REPLY", &path);
                }
            }
        }
        crate::ported::modules::db_gdbm::bin_zgdbmpath("zgdbmpath", args, &ops, 0)
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/termcap.rs
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// `echotc` builtin shim — adapts `&[String]` argv to
    /// `bin_echotc` over a `[bool; 256]` ops bitmask.
    pub(crate) fn bin_echotc(&mut self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let ops = [false; 256];
        bin_echotc("echotc", &argv, &ops)
    }
}

// =====================================================================
// MOVED FROM: src/ported/modules/parameter.rs
// =====================================================================
//
// !!! WARNING: FAKE IMPL — DOES NOT MATCH C SOURCE !!!
//
// `magic_assoc_keys` aggregates per-magic-table dispatch into one
// match-arm. The C source (Src/Modules/parameter.c) splits this
// across separate scanpm{aliases,functions,builtins,commands,
// reswords,options} helpers — each one walks the canonical
// hashtable (cmdnamtab, aliastab, shfunctab, builtintab, reswdtab,
// etc.) via the standard ScanFunc protocol.
//
// This Rust impl hard-codes static lists for "builtins" and
// "reswords" (lines below) and reaches `&self.<field>` for the
// others — both shortcuts. The honest fix is to replace each branch
// with a call to the real scanpm* port in
// src/ported/modules/parameter.rs once those land. Until then this
// stands as a placeholder so the magic-assoc lookup path doesn't
// hard-fail.
//
// !!! Replace with real scanpm* dispatch + remove this block. !!!
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    pub fn magic_assoc_keys(&self, name: &str) -> Option<Vec<String>> {
        let exec = self;
        match name {
            "aliases"  => Some(exec.aliases.keys().cloned().collect()),
            "galiases" => Some(exec.global_aliases.keys().cloned().collect()),
            "saliases" => Some(exec.suffix_aliases.keys().cloned().collect()),
            "dis_aliases" | "dis_galiases" | "dis_saliases" => Some(Vec::new()),
            "functions" | "dis_functions" =>
                Some(exec.function_names().into_iter().collect()),
            // FAKE: hard-coded builtin set instead of walking BUILTINS table.
            "builtins" | "dis_builtins" => {
                let names: &[&str] = &[
                    "echo", "print", "printf", "cd", "pwd", "exit", "return", "true", "false",
                    ":", "test", "[", "local", "private", "declare", "typeset", "export", "unset",
                    "set", "shift", "read", "source", "alias", "unalias", "function", "type",
                    "which", "whence", "command", "builtin", "jobs", "bg", "fg", "wait", "kill",
                    "trap", "eval", "exec", "ulimit", "umask", "getopts", "shopt", "history",
                    "fc", "hash", "rehash", "let", "select", "time", "times", "compdef",
                    "compadd", "complete", "compgen", "zmodload", "zparseopts", "zstyle",
                    "zle", "vared", "zcompile", "autoload",
                ];
                Some(names.iter().map(|s| (*s).to_string()).collect())
            }
            // FAKE: hard-coded reswords set instead of walking reswdtab.
            "reswords" | "dis_reswords" => {
                let names: &[&str] = &[
                    "do", "done", "esac", "then", "elif", "else", "fi", "for", "case", "if",
                    "while", "function", "repeat", "time", "until", "exec", "command", "select",
                    "coproc", "nocorrect", "foreach", "end", "!", "[[", "{", "}", "declare",
                    "export", "float", "integer", "local", "private", "readonly", "typeset",
                ];
                Some(names.iter().map(|s| (*s).to_string()).collect())
            }
            "options"  => Some(exec.options.keys().cloned().collect()),
            "commands" => Some(exec.command_hash.keys().cloned().collect()),
            "jobtexts" | "jobdirs" | "jobstates" =>
                Some(exec.jobs.iter().map(|(id, _)| id.to_string()).collect()),
            "dirstack" =>
                Some((0..exec.dir_stack.len()).map(|i| i.to_string()).collect()),
            "errnos" =>
                Some(crate::modules::system::ERRNO_NAMES
                    .iter().map(|(n, _)| (*n).to_string()).collect()),
            "sysparams" =>
                Some(vec!["pid".to_string(), "ppid".to_string(), "procsubstpid".to_string()]),
            "parameters" => {
                let mut keys: Vec<String> = exec.variables.keys().cloned().collect();
                keys.extend(exec.arrays.keys().cloned());
                keys.extend(exec.assoc_arrays.keys().cloned());
                Some(keys)
            }
            _ => None,
        }
    }
}
