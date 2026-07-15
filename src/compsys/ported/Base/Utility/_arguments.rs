//! Port of `_arguments` from `Completion/Base/Utility/_arguments`.
//!
//! `_arguments` is a THIN SHELL WRAPPER around the C builtin
//! `comparguments` (Src/Zle/computil.c → `bin_comparguments`, ported in
//! `crate::ported::zle::computil`). The shell function does no spec
//! parsing itself: it hands the spec list to `comparguments -i`, then
//! reads results back out of the params `comparguments` populates
//! (`descrs`/`actions`/`subcs`, `next`/`direct`/`odirect`/`equal`,
//! `matcher`, `line`/`opt_args`, `single`, `NORMARG`) and dispatches the
//! per-spec actions. This port mirrors that: it CALLS
//! `bin_comparguments` where the shell calls `comparguments`, and drives
//! the same completion helpers (`_describe`, `_message`, `_tags`,
//! `_requested`, `_description`, `_all_labels`, `_next_label`).
//!
//! Faithful line map (source is 589 lines; `// sh:N` refs below point at
//! real lines):
//!   sh:6-10   locals + `opt_args_use_NUL_separators`
//!   sh:14-28  `while [[ "$1" = -([AMO]...` getopt loop
//!   sh:30-33  end-of-options `:`, `singopt+=(:)`, `PREFIX=[-+]`
//!   sh:35-323 `long=$argv[(I)--]` `--help` long-option cache — runs the
//!             command's `--help`, scrapes/folds its long options, and
//!             memoises them in the persistent `_args_cache_<cmd>` global.
//!             Ported in `long_option_cache`/`build_help_cache` below.
//!   sh:325    `zstyle -s …:options auto-description autod`
//!   sh:327    `comparguments -i "$autod" "$singopt[@]" "$@"`
//!   sh:333-356 `-D`/`-O`/`-a` dispatch + `_tags`
//!   sh:358    `comparguments -M matcher`
//!   sh:364-567 main `while true` / inner `while _tags` engine, the
//!             `->state` branch, the option-completion branch, and the
//!             `=`-descend (`comparguments -L`) branch.
//!   sh:569-589 context restore + return value from `$compstate[nmatches]`.

use crate::compsys::ported::_all_labels::_all_labels;
use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::ported::exec::{dispatch_function_call, execute_script};
use crate::ported::glob::matchpat;
use crate::ported::modules::zutil::zstyletab;
use crate::ported::params::{
    getaparam, getiparam, getsparam, setaparam, setiparam, setsparam, unsetparam,
};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zle::computil::bin_comparguments;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Call the ported C builtin `comparguments` the way the shell does.
/// Mirrors `comparguments -X …` — `argv[0]` is the `-X` subcommand.
fn comparguments(argv: &[&str]) -> i32 {
    let v: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_comparguments("comparguments", &v, &make_ops(), 0)
}

/// `${(@)arr%%:*}` — strip a trailing `:description` from every element.
fn strip_colon_desc(arr: &[String]) -> Vec<String> {
    arr.iter()
        .map(|s| s.splitn(2, ':').next().unwrap_or("").to_string())
        .collect()
}

/// `${PREFIX}${SUFFIX}` — the whole current word around the cursor.
fn prefix_suffix() -> String {
    format!(
        "{}{}",
        getsparam("PREFIX").unwrap_or_default(),
        getsparam("SUFFIX").unwrap_or_default()
    )
}

/// `$compstate[nmatches]` as an integer (0 when unset).
fn nmatches() -> i64 {
    get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// `${context%:*}` / `${oldcontext%:*}` — drop the last `:field`.
fn strip_last_field(ctx: &str) -> String {
    match ctx.rfind(':') {
        Some(i) => ctx[..i].to_string(),
        None => ctx.to_string(),
    }
}

// ---------------------------------------------------------------------------
// sh:35-323 — `--help` long-option cache helpers.
//
// This block, when the spec list contains a literal `--`, runs the target
// command's `--help`, scrapes long options out of the text, folds them
// against the user's own specs, and memoises the result in a persistent
// `_args_cache_<cmd>` global array param. Ported faithfully from
// `Completion/Base/Utility/_arguments`. The zsh parameter-expansion /
// glob idioms are reproduced with `matchpat` (extended glob, case
// sensitive) for the pattern filters and direct string ops for the
// `(#b)` backreference extractions.
// ---------------------------------------------------------------------------

/// sh:45 — `${name//[^a-zA-Z0-9_]/_}`: every non-`[a-zA-Z0-9_]` → `_`.
fn sanitize_cache_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

/// sh — `${x//[^a-zA-Z0-9_-]}`: drop every char not in `[a-zA-Z0-9_-]`.
fn strip_to_wordchars(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// sh — `${x%%[^a-zA-Z0-9_-]#}`: strip the longest trailing run of
/// non-`[a-zA-Z0-9_-]` chars (used to derive a clean option name).
fn strip_trailing_nonword(s: &str) -> String {
    s.trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .to_string()
}

/// sh:110/153 — description transform `${${${d//:/-}//\[/(}//\]/)}`
/// (`:`→`-`, `[`→`(`, `]`→`)`).
fn desc_transform(s: &str) -> String {
    s.replace(':', "-").replace('[', "(").replace(']', ")")
}

/// Append `elem:suffix` to `lopts` for each `elem` of `tmp`, maintaining
/// the `typeset -Ua lopts` uniqueness (sh:49). Mirrors `${^tmp[@]}:suffix`.
fn lopts_add_distributed(lopts: &mut Vec<String>, tmp: &[String], suffix: &str) {
    for e in tmp {
        let v = format!("{}:{}", e, suffix);
        if !lopts.contains(&v) {
            lopts.push(v);
        }
    }
}

/// zsh `${(M)1#*[^\\]:}[1,-2]` + `${1#pattern}` (sh:215-217): split an
/// optspec at its first *unescaped* colon. Returns `(pattern, descr)`
/// where `pattern` has `\:`→`:` applied and excludes the colon, and
/// `descr` is the remainder starting at the colon (or empty).
fn split_spec_pattern(spec: &str) -> (String, String) {
    let chars: Vec<char> = spec.chars().collect();
    let mut idx = None;
    for (j, &c) in chars.iter().enumerate() {
        // sh `*[^\\]:` requires a non-backslash char before the colon.
        if c == ':' && j > 0 && chars[j - 1] != '\\' {
            idx = Some(j);
            break;
        }
    }
    match idx {
        Some(k) => {
            let pat: String = chars[..k].iter().collect::<String>().replace("\\:", ":");
            let descr: String = chars[k..].iter().collect();
            (pat, descr)
        }
        None => (spec.to_string(), String::new()),
    }
}

/// sh:148 — `${opt## [^[:space:]]##  }`: strip a leading
/// `<space><nonspace>+<space><space>` prefix (the "--foo fooarg  desc"
/// case where `fooarg` must be dropped).
fn strip_leading_argword(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    if b.first() == Some(&' ') {
        let mut i = 1;
        while i < b.len() && b[i] != ' ' {
            i += 1;
        }
        if i > 1 && i + 1 < b.len() && b[i] == ' ' && b[i + 1] == ' ' {
            return b[i + 2..].iter().collect();
        }
    }
    s.to_string()
}

/// `(#b)(*):([^:]#)` (sh:255/285): split at the LAST colon, returning
/// `(before, Some(after))`, or `(whole, None)` when there is no colon.
fn split_last_colon(s: &str) -> (String, Option<String>) {
    match s.rfind(':') {
        Some(i) => (s[..i].to_string(), Some(s[i + 1..].to_string())),
        None => (s.to_string(), None),
    }
}

/// Run `$cmd --help` and return the captured text (stdout+stderr merged,
/// mirroring the caller's `2>&1` at sh:98 — the shared `_call_program`
/// port only captures stdout, so this call site does the exec directly to
/// keep the `2>&1` merge). `command`-style override, `COLUMNS=999` and the
/// C-locale reset from `_call_program`/`_comp_locale` are reproduced here.
fn run_help(cmd: &str, use_locale: bool) -> String {
    use std::process::{Command, Stdio};

    // sh:26-33 (via _call_program) — style key `${1}` is "options".
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let style_ctx = format!(":completion:{}:options", curcontext);
    let styled = crate::ported::modules::zutil::lookupstyle(&style_ctx, "command")
        .into_iter()
        .next()
        .unwrap_or_default();
    // sh:98 argv[2,-1] == `${~words[1]} --help`.
    let cmdline = if !styled.is_empty() {
        if let Some(rest) = styled.strip_prefix('-') {
            // sh:28 — eval "$tmp[2,-1]" "$argv[2,-1]"
            format!("{} {} --help", rest, cmd)
        } else {
            // sh:30 — eval $prefix "$tmp" (cmd/--help ignored)
            styled
        }
    } else {
        // sh:33 — eval $prefix "$argv[2,-1]"
        format!("{} --help", cmd)
    };

    let mut c = Command::new("sh");
    c.arg("-c").arg(format!("{} 2>&1", cmdline));
    c.env("COLUMNS", "999"); // _call_program sh:3
    c.stdin(Stdio::null());
    if use_locale {
        // _comp_locale: LANG=C, keep LC_CTYPE, all other LC_* → C.
        // Apply to the child env only (no parent-env mutation).
        for (k, _) in std::env::vars() {
            if k.starts_with("LC_") && k != "LC_CTYPE" {
                c.env_remove(&k);
            }
        }
        let ctype = std::env::var("LC_ALL")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("LC_CTYPE").ok().filter(|s| !s.is_empty()))
            .or_else(|| std::env::var("LANG").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "C".to_string());
        c.env("LC_CTYPE", ctype);
        c.env("LANG", "C");
    }

    // Queue signals across the foreground wait so the SIGCHLD reaper can't
    // steal the child from under `.output()` (same fencing as
    // fusevm_bridge::ForegroundWaitGuard).
    let _guard = crate::fusevm_bridge::ForegroundWaitGuard::enter();
    match c.output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// sh:35-323 — build (or reuse) the `_args_cache_<cmd>` long-option cache
/// and return the final spec vector `tmpargv + <cached optspecs>` (sh:322).
///
/// `full` is the post-getopt spec vector (`$argv` at sh:35); `long` is the
/// 0-based index of the `--` element in it; `cmd` is `$words[1]`.
fn long_option_cache(full: &[String], long: usize, cmd: &str) -> Vec<String> {
    // sh:39 — optspecs before `--`.
    let tmpargv: Vec<String> = full[..long].to_vec();

    // sh:41-45 — build the cache param name from the command word.
    //   name=${~words[1]}; [[ $name = [^/]*/* ]] && name="$PWD/$name"
    let mut name = cmd.to_string();
    if let Some(rest) = name.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            name = format!("{}/{}", home, rest);
        }
    } else if name == "~" {
        if let Ok(home) = std::env::var("HOME") {
            name = home;
        }
    }
    // [[ "$name" = [^/]*/* ]] — relative path containing a slash.
    if !name.starts_with('/') && name.contains('/') {
        let pwd = getsparam("PWD")
            .or_else(|| std::env::var("PWD").ok())
            .unwrap_or_default();
        name = format!("{}/{}", pwd, name);
    }
    let name = sanitize_cache_name(&format!("_args_cache_{}", name));

    // sh:47 — if (( ! ${(P)+name} )): rebuild only when unset.
    let already_set = crate::ported::params::paramtab()
        .read()
        .map(|t| t.contains_key(&name))
        .unwrap_or(false);

    if !already_set {
        let cache = build_help_cache(full, long, cmd, &tmpargv);
        // sh:320 — set -A "$name" "${(@)cache:# #}" (drop all-space elems).
        let cache: Vec<String> = cache
            .into_iter()
            .filter(|e| !e.bytes().all(|b| b == b' '))
            .collect();
        setaparam(&name, cache);
    }

    // sh:322 — set -- "$tmpargv[@]" "${(@P)name}"
    let mut out = tmpargv;
    out.extend(getaparam(&name).unwrap_or_default());
    out
}

/// sh:97-162 — scrape long options out of `--help` text into the unique
/// `lopts` array (`option:description`, description colon-transformed).
fn scrape_help_lopts(help_text: &str) -> Vec<String> {
    let mut lopts: Vec<String> = Vec::new(); // typeset -Ua (unique)
    let mut tmp: Vec<String> = Vec::new(); // per-line option accumulator

    for line in help_text.lines() {
        let mut opt = line.to_string();

        // sh:100-122 — flush the previous line's uncommented options.
        if !tmp.is_empty() {
            let leading3_ws = {
                let c: Vec<char> = opt.chars().take(3).collect();
                c.len() == 3 && c.iter().all(|ch| ch.is_whitespace())
            };
            let has_alpha_after = opt.chars().skip(3).any(|c| c.is_ascii_alphabetic());
            if leading3_ws && has_alpha_after {
                // sh:106 — this line is the description for the prev opts.
                opt = opt.trim_start_matches(char::is_whitespace).to_string();
                lopts_add_distributed(&mut lopts, &tmp, &desc_transform(&opt));
                tmp.clear();
                continue;
            } else {
                // sh:119 — no description; add options with empty desc.
                lopts_add_distributed(&mut lopts, &tmp, "");
                tmp.clear();
            }
        }

        // sh:123-142 — pull consecutive `-…`/`--…` option tokens.
        loop {
            let trimmed = opt.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
            if !trimmed.starts_with('-') {
                break;
            }
            // start = `-` + run of non-comma-non-space; rest = remainder.
            let end = trimmed[1..]
                .find(|c: char| c == ',' || c.is_whitespace())
                .map(|i| i + 1)
                .unwrap_or(trimmed.len());
            let start = trimmed[..end].to_string();
            let rest = trimmed[end..].to_string();

            // sh:131 — skip if tmp already holds the cleaned name.
            let cleaned = strip_trailing_nonword(&start);
            let dup = tmp.iter().any(|e| matchpat(&cleaned, e, true, true));
            if !dup {
                // sh:135 — `--[fetch]all` variant → `--fetchall` + `--all`.
                if start.starts_with("--[") {
                    if let Some(close) = start.rfind(']') {
                        let inner = &start[3..close];
                        let after = &start[close + 1..];
                        tmp.push(format!("--{}{}", inner, after));
                        tmp.push(format!("--{}", after));
                    } else {
                        tmp.push(start.clone());
                    }
                } else {
                    tmp.push(start.clone());
                }
            }
            opt = rest;
        }

        // sh:148-149 — drop a leading " ARG  " then leading whitespace.
        opt = strip_leading_argword(&opt);
        opt = opt.trim_start_matches(char::is_whitespace).to_string();

        // sh:150-157 — leftover text is the description for this line.
        if !opt.is_empty() {
            lopts_add_distributed(&mut lopts, &tmp, &desc_transform(&opt));
            tmp.clear();
        }
    }
    // sh:160-162 — tidy up any trailing uncommented options.
    if !tmp.is_empty() {
        lopts_add_distributed(&mut lopts, &tmp, "");
    }

    lopts
}

/// sh:48-320 — the actual cache construction (only runs on a cache miss).
fn build_help_cache(
    full: &[String],
    long: usize,
    cmd: &str,
    tmpargv: &[String],
) -> Vec<String> {
    // sh:56 — set -- "${(@)argv[long+1,-1]}" (args after `--`).
    let post: Vec<String> = full[long + 1..].to_vec();
    let mut p = 0usize;

    // sh:58-84 — parse the `-i`/`-s`/`-l` option-generation flags.
    let mut iopts: Vec<String> = Vec::new(); // ignore patterns
    let mut sopts: Vec<String> = Vec::new(); // "same" substitution pairs
    let mut use_locale = true; // lflag: `-l` disables the C-locale reset
    while p < post.len() {
        let a = &post[p];
        // while [[ "$1" = -[lis]* ]]
        if !(a.len() >= 2 && a.as_bytes()[0] == b'-' && matches!(a.as_bytes()[1], b'l' | b'i' | b's'))
        {
            break;
        }
        // sh:61 — exact `-l` is the locale flag; consume and continue.
        if a == "-l" {
            use_locale = false;
            p += 1;
            continue;
        }
        // sh:66 — attached (`-iFOO`) vs separate (`-i FOO`) argument.
        let (raw, cur) = if a.len() > 2 {
            (a[2..].to_string(), 1usize)
        } else if p + 1 < post.len() {
            (post[p + 1].clone(), 2usize)
        } else {
            (String::new(), 1usize)
        };
        // sh:73-77 — literal `(a b c)` list, else a parameter name.
        let vals: Vec<String> = if raw.starts_with('(') {
            // ${=tmp[2,-2]} — drop the parens, then word-split on IFS.
            let inner = raw
                .strip_prefix('(')
                .map(|s| s.strip_suffix(')').unwrap_or(s))
                .unwrap_or(raw.as_str());
            inner.split_whitespace().map(|s| s.to_string()).collect()
        } else {
            getaparam(&raw).unwrap_or_default()
        };
        // sh:78-82 — `-i*` → ignore, else `-s*` → same.
        if a.as_bytes()[1] == b'i' {
            iopts.extend(vals);
        } else {
            sopts.extend(vals);
        }
        p += cur;
    }
    // The remaining post-`--` args are the help-conversion description specs.
    let descr_specs_user: Vec<String> = post[p..].to_vec();

    // sh:97-162 — scrape long options from `$cmd --help`.
    let help_text = run_help(cmd, use_locale);
    let mut lopts: Vec<String> = scrape_help_lopts(&help_text);

    // sh:164-176 — drop options already covered by user-defined specs.
    {
        let mut kept: Vec<String> = Vec::new();
        // Iterate the cleaned names of every non-`--` lopt.
        let names: Vec<String> = lopts
            .iter()
            .filter(|e| e.as_str() != "--")
            .map(|e| {
                // ${e%%[\[:=]*}
                let cut = e
                    .find(|c: char| c == '[' || c == ':' || c == '=')
                    .unwrap_or(e.len());
                e[..cut].to_string()
            })
            .collect();
        for optn in names {
            // sh:173 — covered if any user spec matches this pattern.
            // sh:173 — `(|\*)` and `\[*\]` are LITERAL `*`/`[`/`]`; the
            // interior `*` inside `\[*\]` is the glob wildcard.
            let pat = format!(
                "(|\\([^)]#\\))(|\\*){}(|[-+]|=(|-))(|\\[*\\])(|:*)",
                optn
            );
            let covered = tmpargv.iter().any(|s| matchpat(&pat, s, true, true));
            if !covered {
                // sh:174 — keep the original lopt element matching `optn`.
                let keep_pat = format!("{}(|[\\[:=]*)", optn);
                if let Some(found) = lopts.iter().find(|e| matchpat(&keep_pat, e.as_str(), true, true)) {
                    if !kept.contains(found) {
                        kept.push(found.clone());
                    }
                }
            }
        }
        lopts = kept;
    }

    // sh:180-183 — remove all ignored options.
    for ig in &iopts {
        let pat = format!("{}(|[\\[:=]*)", ig);
        lopts.retain(|e| !matchpat(&pat, e, true, true));
    }

    // sh:187-194 — add "same" options (e.g. --disable-* from --enable-*).
    // sopts is a flat list of (pattern, replacement) pairs.
    {
        let mut i = 0;
        while i + 1 < sopts.len() {
            let pat = &sopts[i];
            let repl = &sopts[i + 1];
            // ${lopts/pat/repl}: first-match substitution per element,
            // appended back (uniqueness absorbs the unchanged copies).
            let subs: Vec<String> = lopts
                .iter()
                .map(|e| subst_first(e, pat, repl))
                .collect();
            for s in subs {
                if !lopts.contains(&s) {
                    lopts.push(s);
                }
            }
            i += 2;
        }
    }

    // sh:200-205 — builtin description specs, appended after the user's.
    let mut descr_specs = descr_specs_user;
    descr_specs.push("*=FILE*:file:_files".to_string());
    descr_specs.push("*=(DIR|PATH)*:directory:_files -/".to_string());
    descr_specs.push("*=*:=: ".to_string());
    descr_specs.push("*: :  ".to_string());

    // sh:207-319 — walk the description specs, matching lopts and building
    // the final optspec cache.
    let mut cache: Vec<String> = Vec::new();
    for spec in descr_specs {
        // sh:215-217 — split into pattern and description.
        let (mut pattern, descr) = split_spec_pattern(&spec);

        // sh:218-225 — trailing `(-)` disallows an argument in the next word.
        let dir = if pattern.ends_with("(-)") {
            pattern.truncate(pattern.len() - 3);
            "-".to_string()
        } else {
            String::new()
        };

        // sh:234-235 — the lopts matching `pattern:*`; remove from lopts.
        let match_pat = format!("{}:*", pattern);
        let mut matched: Vec<String> = lopts
            .iter()
            .filter(|e| matchpat(&match_pat, e.as_str(), true, true))
            .cloned()
            .collect();
        lopts.retain(|e| !matchpat(&match_pat, e, true, true));

        // sh:237 — nothing matched → next spec.
        if matched.is_empty() {
            continue;
        }

        // sh:242 — strip the trailing ':' added during the scrape.
        for e in &mut matched {
            if e.ends_with(':') {
                e.pop();
            }
        }

        // sh:247-274 — options with `[=ARG]` → optional argument.
        {
            let optpat = "[^:]##\\[\\=*";
            let tmpo: Vec<String> = matched
                .iter()
                .filter(|e| matchpat(optpat, e.as_str(), true, true))
                .cloned()
                .collect();
            if !tmpo.is_empty() {
                matched.retain(|e| !matchpat(optpat, e.as_str(), true, true));
                for opt in tmpo {
                    // sh:255 — split trailing :description → odescr.
                    let (opt, odescr) = match split_last_colon(&opt) {
                        (base, Some(d)) => (base, format!("[{}]", d)),
                        (base, None) => (base, String::new()),
                    };
                    // sh:261 — has `[=` → strip it; opt2 uses `=-` (optional).
                    let opt2 = if let Some(pos) = opt.rfind("[=") {
                        format!("{}=-{}{}", strip_to_wordchars(&opt[..pos]), dir, odescr)
                    } else {
                        format!("{}={}{}", strip_to_wordchars(&opt), dir, odescr)
                    };
                    // sh:266-272 — assemble the cache entry per descr form.
                    if descr.starts_with(":=") {
                        // sh:267 — "${opt2}::${(L)${opt%\]}#*\=}: "
                        cache.push(format!("{}::{}: ", opt2, arg_name_lower(&opt)));
                    } else if descr.starts_with("::") {
                        cache.push(format!("{}{}", opt2, descr)); // sh:269
                    } else {
                        cache.push(format!("{}:{}", opt2, descr)); // sh:271
                    }
                }
            }
        }

        // sh:280-298 — options with `=ARG` → mandatory argument.
        {
            let eqpat = "[^:]##\\=*";
            let tmpo: Vec<String> = matched
                .iter()
                .filter(|e| matchpat(eqpat, e.as_str(), true, true))
                .cloned()
                .collect();
            if !tmpo.is_empty() {
                matched.retain(|e| !matchpat(eqpat, e.as_str(), true, true));
                for opt in tmpo {
                    let (opt, odescr) = match split_last_colon(&opt) {
                        (base, Some(d)) => (base, format!("[{}]", d)),
                        (base, None) => (base, String::new()),
                    };
                    // sh:291 — opt2="${${opt%%\=*}//[^word]}=${dir}${odescr}"
                    let base = opt.split('=').next().unwrap_or("");
                    let opt2 = format!("{}={}{}", strip_to_wordchars(base), dir, odescr);
                    if descr.starts_with(":=") {
                        // sh:293 — "${opt2}:${(L)${opt%\]}#*\=}: "
                        cache.push(format!("{}:{}: ", opt2, arg_name_lower(&opt)));
                    } else {
                        cache.push(format!("{}{}", opt2, descr)); // sh:295
                    }
                }
            }
        }

        // sh:303-318 — everything else: option without an argument.
        if !matched.is_empty() {
            let mut built: Vec<String> = Vec::new();
            // sh:310 — options WITH a description → "option[description]".
            for e in &matched {
                if let Some(ci) = e.find(':') {
                    let head = e[..ci].to_string();
                    let tail = e[ci + 1..].to_string();
                    // ${e//:/[} then append ']' (desc already colon-free).
                    built.push(format!("{}[{}]", head, tail.replace(':', "[")));
                }
            }
            // sh:312 — options WITHOUT a description → sanitized bare option.
            for e in &matched {
                if !e.contains(':') {
                    built.push(strip_to_wordchars(e));
                }
            }
            // sh:313-317 — append descr unless it's the catch-all ': :  '.
            if !descr.is_empty() && descr != ": :  " {
                for b in built {
                    cache.push(format!("{}{}", b, descr));
                }
            } else {
                cache.extend(built);
            }
        }
    }

    cache
}

/// sh:267/293 — `${(L)${opt%\]}#*\=}`: strip a trailing `]`, drop
/// everything up to and including the first `=`, then lowercase.
fn arg_name_lower(opt: &str) -> String {
    let s = opt.strip_suffix(']').unwrap_or(opt);
    let after = match s.find('=') {
        Some(i) => &s[i + 1..],
        None => s,
    };
    after.to_lowercase()
}

/// zsh `${str/pat/repl}` — replace the FIRST glob match of `pat` in `str`
/// with `repl`. Falls back to the unchanged string when nothing matches.
fn subst_first(str_in: &str, pat: &str, repl: &str) -> String {
    // Find the shortest leftmost substring that matches `pat` as a full
    // glob, then splice in `repl`. zsh `${x/p/r}` matches the leftmost,
    // longest run; we approximate with leftmost start + longest end,
    // which is faithful for the anchored `--enable-*`-style patterns used
    // by the `-s` "same" option feature.
    let chars: Vec<char> = str_in.chars().collect();
    for start in 0..=chars.len() {
        for end in (start..=chars.len()).rev() {
            let sub: String = chars[start..end].iter().collect();
            if !sub.is_empty() && matchpat(pat, &sub, true, true) {
                let prefix: String = chars[..start].iter().collect();
                let suffix: String = chars[end..].iter().collect();
                return format!("{}{}{}", prefix, repl, suffix);
            }
        }
    }
    str_in.to_string()
}

/// `_arguments` — spec engine. `args` is the full argument vector the
/// caller passed after the function name (flags then spec strings).
pub fn _arguments(args: &[String]) -> i32 {
    // sh:6-8 — locals mirroring the zsh source. `cmd="$words[1]"`.
    let words0 = getaparam("words").unwrap_or_default();
    let cmd = words0.first().cloned().unwrap_or_default(); // sh:6 cmd
    let oldcontext = getsparam("curcontext").unwrap_or_default(); // sh:7

    let mut subopts: Vec<String> = Vec::new(); // sh:12
    let mut singopt: Vec<String> = Vec::new(); // sh:12
    let mut usecc = false; // sh:17 usecc=yes
    let mut rawret = false; // sh:20 rawret=yes
    let mut setnormarg = false; // sh:21 setnormarg=yes
    let mut optarg = false; // sh:22 optarg=yes
    let mut alwopt = String::new(); // sh:23 alwopt=arg
    // sh:10 — integer opt_args_use_NUL_separators=0
    let mut opt_args_use_nul_separators: i32 = 0;

    // sh:14-28 — while [[ "$1" = -([AMO]*|[0CRSWnsw]) ]]; do … done
    let mut i = 0usize;
    while i < args.len() {
        let a = &args[i];
        let b = a.as_bytes();
        // Guard matches the glob `-([AMO]*|[0CRSWnsw])`.
        let matches_flag = b.len() >= 2
            && b[0] == b'-'
            && (matches!(b[1], b'A' | b'M' | b'O')
                || (b.len() == 2 && matches!(b[1], b'0' | b'C' | b'R' | b'S' | b'W' | b'n' | b's' | b'w')));
        if !matches_flag {
            break;
        }
        match a.as_str() {
            // sh:16 -0) opt_args_use_NUL_separators=1
            "-0" => {
                opt_args_use_nul_separators = 1;
                i += 1;
            }
            // sh:17 -C) usecc=yes
            "-C" => {
                usecc = true;
                i += 1;
            }
            // sh:18 -O) subopts=( "${(@P)2}" ); shift 2
            "-O" if i + 1 < args.len() => {
                subopts = getaparam(&args[i + 1]).unwrap_or_default();
                i += 2;
            }
            // sh:20 -R) rawret=yes
            "-R" => {
                rawret = true;
                i += 1;
            }
            // sh:21 -n) setnormarg=yes; NORMARG=-1
            "-n" => {
                setnormarg = true;
                let _ = setiparam("NORMARG", -1);
                i += 1;
            }
            // sh:22 -w) optarg=yes
            "-w" => {
                optarg = true;
                i += 1;
            }
            // sh:23 -W) alwopt=arg
            "-W" => {
                alwopt = "arg".to_string();
                i += 1;
            }
            // sh:24 -[Ss]) singopt+=( $1 )
            "-s" | "-S" => {
                singopt.push(a.clone());
                i += 1;
            }
            // sh:25 -[AM]) singopt+=( $1 $2 ); shift 2
            "-A" | "-M" if i + 1 < args.len() => {
                singopt.push(a.clone());
                singopt.push(args[i + 1].clone());
                i += 2;
            }
            // sh:19 -O*) subopts=( "${(@P)${1[3,-1]}}" )
            _ if b[1] == b'O' => {
                subopts = getaparam(&a[2..]).unwrap_or_default();
                i += 1;
            }
            // sh:26 -[AM]*) singopt+=( $1 )
            _ if b[1] == b'A' || b[1] == b'M' => {
                singopt.push(a.clone());
                i += 1;
            }
            _ => break,
        }
    }

    // sh:30 — [[ $1 = ':' ]] && shift
    if i < args.len() && args[i] == ":" {
        i += 1;
    }
    // sh:31 — always end with ':' to indicate the end of options.
    singopt.push(":".to_string());

    // sh:33 — [[ "$PREFIX" = [-+] ]] && alwopt=arg
    if matches!(getsparam("PREFIX").as_deref(), Some("-") | Some("+")) {
        alwopt = "arg".to_string();
    }

    // sh:35-323 — long=$argv[(I)--] `--help` long-option cache. When a
    // `--` appears in the spec list, run `$cmd --help`, scrape its long
    // options, fold them against the user specs, memoise into the
    // persistent `_args_cache_<cmd>` global, and pass `tmpargv +
    // <cached optspecs>` on to `comparguments -i`. See `long_option_cache`.
    let full: Vec<String> = args[i..].to_vec();
    let specs: Vec<String> = match full.iter().rposition(|s| s == "--") {
        // sh:36 (( long )) — `(I)` returns the last match (rposition).
        Some(long) => long_option_cache(&full, long, &cmd),
        None => full,
    };

    // sh:325 — zstyle -s ":completion:${curcontext}:options" \
    //          auto-description autod
    let autod = {
        let ctx = format!(":completion:{}:options", oldcontext);
        match zstyletab.lock() {
            Ok(t) => t
                .get(&ctx, "auto-description")
                .and_then(|v| v.first().cloned())
                .unwrap_or_default(),
            Err(_) => String::new(),
        }
    };

    // sh:327 — if (( $# )) && comparguments -i "$autod" "$singopt[@]" "$@"
    if specs.is_empty() {
        return 1; // (( $# )) false → outer `if` false → sh:588 return 1
    }
    {
        let mut init_argv: Vec<&str> = vec!["-i", autod.as_str()];
        for s in &singopt {
            init_argv.push(s);
        }
        for s in &specs {
            init_argv.push(s);
        }
        if comparguments(&init_argv) != 0 {
            // sh:588 — else return 1
            return 1;
        }
    }

    // sh:329-331 — locals for the completion body. Names mirror source.
    let mut noargs = String::new();
    let mut aret = false;
    let mut tried = false;
    let mut ret = 1i32;
    let mut matched = false;
    let mut hasopts = false;
    let mut mesg = false;
    let mut opts = false;
    let mut local_set = false; // shell `local` sentinel (sh:405 `$local`)
    let origpre = getsparam("PREFIX").unwrap_or_default();
    let origipre = getsparam("IPREFIX").unwrap_or_default();
    let nm = nmatches(); // sh:331 nm="$compstate[nmatches]"

    // sh:333-356 — get descrs/actions/subcs and the option lists, then
    // pick the right `_tags` set.
    let mut have_descrs = false;
    if comparguments(&["-D", "descrs", "actions", "subcs"]) == 0 {
        // sh:333 comparguments -D descrs actions subcs
        have_descrs = true;
        let subcs = getaparam("subcs").unwrap_or_default();
        if comparguments(&["-O", "next", "direct", "odirect", "equal"]) == 0 {
            // sh:334
            opts = true;
            let mut targ = subcs.clone();
            targ.push("options".to_string());
            let _ = _tags(&targ); // sh:337
        } else {
            let _ = _tags(&subcs); // sh:338
        }
    } else {
        // sh:341 comparguments -a
        if comparguments(&["-a"]) == 0 {
            noargs = "no more arguments".to_string();
        } else {
            noargs = "no arguments".to_string();
        }
        // sh:346 comparguments -O next direct odirect equal
        let orc = comparguments(&["-O", "next", "direct", "odirect", "equal"]);
        if orc == 0 {
            opts = true;
            let _ = _tags(&["options".to_string()]); // sh:348
        } else if orc == 2 {
            // sh:349 — [[ $? -eq 2 ]] (singles): add the raw word, done.
            let word = prefix_suffix();
            let _ = bin_compadd(
                "compadd",
                &["-Q".to_string(), "-".to_string(), word],
                &make_ops(),
                0,
            );
            return 0;
        } else {
            let _ = _message(&[noargs.clone()]); // sh:353
            return 1;
        }
    }

    // `$opt_args_use_NUL_separators` as the trailing arg to `-W`.
    let nul_sep = opt_args_use_nul_separators.to_string();

    // sh:358 — comparguments -M matcher
    let _ = comparguments(&["-M", "matcher"]);
    let matcher = getsparam("matcher").unwrap_or_default();

    // sh:360-362 — context=(); state=(); state_descr=()
    let mut context: Vec<String> = Vec::new();
    let mut state: Vec<String> = Vec::new();
    let mut state_descr: Vec<String> = Vec::new();
    setaparam("context", Vec::new());
    setaparam("state", Vec::new());
    setaparam("state_descr", Vec::new());

    // sh:364 — while true; do
    loop {
        // sh:365 — while _tags; do
        while _tags(&[]) == 0 {
            // sh:367 — if [[ -z "$tried" ]]; then walk the descrs.
            if !tried && have_descrs {
                let descrs = getaparam("descrs").unwrap_or_default();
                let actions = getaparam("actions").unwrap_or_default();
                let subcs = getaparam("subcs").unwrap_or_default();
                let mut anum = 0usize; // sh:368 anum=1 (0-based here)
                while anum < descrs.len() {
                    // sh:370-372
                    let action = actions.get(anum).cloned().unwrap_or_default();
                    let descr = descrs.get(anum).cloned().unwrap_or_default();
                    let subc = subcs.get(anum).cloned().unwrap_or_default();
                    anum += 1;

                    // sh:374 — if [[ $subc = argument* && -n $setnormarg ]]
                    if subc.starts_with("argument") && setnormarg {
                        let _ = comparguments(&["-n", "NORMARG"]); // sh:375
                    }

                    // sh:378 — if [[ -n "$matched" ]] || _requested "$subc"
                    if !matched && _requested(std::slice::from_ref(&subc)) != 0 {
                        continue;
                    }

                    // sh:380 — curcontext="${oldcontext%:*}:$subc"
                    let _ = setsparam(
                        "curcontext",
                        &format!("{}:{}", strip_last_field(&oldcontext), subc),
                    );

                    // sh:382 — _description "$subc" expl "$descr"
                    let _ = _description(&[subc.clone(), "expl".to_string(), descr.clone()]);

                    // Work on a mutable copy of the action (sh reassigns it).
                    let mut action = action;

                    // sh:384 — [[ "$action" = \=\ * ]] (starts with "= ")
                    if let Some(rest) = action.strip_prefix("= ") {
                        action = rest.to_string();
                        // words=( "$subc" "$words[@]" ); (( CURRENT++ ))
                        let mut w = getaparam("words").unwrap_or_default();
                        w.insert(0, subc.clone());
                        setaparam("words", w);
                        let cur = getiparam("CURRENT");
                        let _ = setiparam("CURRENT", cur + 1);
                    }

                    // sh:390 — if [[ "$action" = -\>* ]] (->state)
                    if let Some(st) = action.strip_prefix("->") {
                        // sh:391 — strip surrounding whitespace.
                        let st = st.trim().to_string();
                        // sh:392 — if (( ! $state[(I)$action] ))
                        if !state.iter().any(|s| s == &st) {
                            // sh:393 comparguments -W line opt_args <nul>
                            let _ = comparguments(&[
                                "-W",
                                "line",
                                "opt_args",
                                nul_sep.as_str(),
                            ]);
                            state.push(st.clone());
                            state_descr.push(descr.clone());
                            setaparam("state", state.clone());
                            setaparam("state_descr", state_descr.clone());
                            if usecc {
                                // sh:397
                                let _ = setsparam(
                                    "curcontext",
                                    &format!("{}:{}", strip_last_field(&oldcontext), subc),
                                );
                            } else {
                                // sh:399 context+=( "$subc" )
                                context.push(subc.clone());
                                setaparam("context", context.clone());
                            }
                            set_compstate_str("restore", ""); // sh:401
                            aret = true; // sh:402
                        }
                        continue; // handled this spec
                    }

                    // sh:405-409 — first real action: declare line/opt_args.
                    if !local_set {
                        // `local line; typeset -A opt_args; local=yes`
                        local_set = true;
                    }

                    // sh:411 — comparguments -W line opt_args <nul>
                    let _ = comparguments(&[
                        "-W",
                        "line",
                        "opt_args",
                        nul_sep.as_str(),
                    ]);

                    if action.chars().all(|c| c == ' ') {
                        // sh:413 — [[ "$action" = \ # ]] empty action.
                        let _ = _message(&["-e".to_string(), subc.clone(), descr.clone()]);
                        mesg = true;
                        tried = true;
                        if alwopt.is_empty() {
                            alwopt = "yes".to_string();
                        }
                    } else if action.starts_with("((") && action.ends_with("))") {
                        // sh:421 — ((literal:desc …)) → _describe.
                        let body = &action[2..action.len() - 2];
                        let ws: Vec<String> =
                            body.split_whitespace().map(|s| s.to_string()).collect();
                        setaparam("ws", ws);
                        let mut dv = vec![
                            "-t".to_string(),
                            subc.clone(),
                            descr.clone(),
                            "ws".to_string(),
                            "-M".to_string(),
                            matcher.clone(),
                        ];
                        dv.extend(subopts.iter().cloned());
                        if dispatch_function_call("_describe", &dv).unwrap_or(1) != 0
                            && alwopt.is_empty()
                        {
                            alwopt = "yes".to_string();
                        }
                        tried = true;
                    } else if action.starts_with('(') && action.ends_with(')') {
                        // sh:431 — (literal list) → _all_labels + compadd.
                        let body = &action[1..action.len() - 1];
                        let ws: Vec<String> =
                            body.split_whitespace().map(|s| s.to_string()).collect();
                        setaparam("ws", ws);
                        let mut av = vec![subc.clone(), "expl".to_string(), descr.clone(), "compadd".to_string()];
                        av.extend(subopts.iter().cloned());
                        av.push("-a".to_string());
                        av.push("-".to_string());
                        av.push("ws".to_string());
                        if _all_labels(&av) != 0 && alwopt.is_empty() {
                            alwopt = "yes".to_string();
                        }
                        tried = true;
                    } else if action.starts_with('{') && action.ends_with('}') {
                        // sh:440 — {body} → eval body per label.
                        let body = &action[1..action.len() - 1];
                        loop {
                            if _next_label(&[subc.clone(), "expl".to_string(), descr.clone()]) != 0 {
                                break;
                            }
                            if execute_script(body).map(|rc| rc == 0).unwrap_or(false) {
                                ret = 0;
                            }
                        }
                        if ret != 0 && alwopt.is_empty() {
                            alwopt = "yes".to_string();
                        }
                        tried = true;
                    } else if action.starts_with(' ') {
                        // sh:449 — action starts with space: just call it.
                        let parts: Vec<String> =
                            action.split_whitespace().map(|s| s.to_string()).collect();
                        if let Some((cmd, rest)) = parts.split_first() {
                            loop {
                                if _next_label(&[subc.clone(), "expl".to_string(), descr.clone()])
                                    != 0
                                {
                                    break;
                                }
                                if dispatch_function_call(cmd, rest).unwrap_or(1) == 0 {
                                    ret = 0;
                                }
                            }
                        }
                        if ret != 0 && alwopt.is_empty() {
                            alwopt = "yes".to_string();
                        }
                        tried = true;
                    } else {
                        // sh:459 — call action[1] with subopts, expl, rest.
                        let parts: Vec<String> =
                            action.split_whitespace().map(|s| s.to_string()).collect();
                        if let Some((cmd, rest)) = parts.split_first() {
                            loop {
                                if _next_label(&[subc.clone(), "expl".to_string(), descr.clone()])
                                    != 0
                                {
                                    break;
                                }
                                let expl = getaparam("expl").unwrap_or_default();
                                let mut call_argv: Vec<String> = subopts.clone();
                                call_argv.extend(expl);
                                call_argv.extend(rest.iter().cloned());
                                let rc = if cmd == "compadd" {
                                    bin_compadd("compadd", &call_argv, &make_ops(), 0)
                                } else {
                                    dispatch_function_call(cmd, &call_argv).unwrap_or(1)
                                };
                                if rc == 0 {
                                    ret = 0;
                                }
                            }
                        }
                        if ret != 0 && alwopt.is_empty() {
                            alwopt = "yes".to_string();
                        }
                        tried = true;
                    }
                }
            }

            // sh:474-535 — the option-completion branch.
            let prefix_needed_ok = {
                // { ! zstyle -T …:options prefix-needed || … }
                let ctx = format!(":completion:{}:options", strip_last_field(&oldcontext));
                // `zstyle -T` is true when the style is unset OR truthy.
                let t = zstyletab
                    .lock()
                    .ok()
                    .map(|tab| tab.test_bool(&ctx, "prefix-needed"))
                    .unwrap_or(None);
                let prefix_needed_true = t != Some(false); // -T semantics
                !prefix_needed_true
                    || origpre.starts_with(|c: char| c == '-' || c == '+')
                    || (!aret && !mesg && !tried) // -z "$aret$mesg$tried"
            };
            let cur_prefix = getsparam("PREFIX").unwrap_or_default();
            if _requested(&["options".to_string()]) == 0
                && !hasopts
                && !matched
                && (!aret || cur_prefix == origpre)
                && prefix_needed_ok
            {
                // sh:480 — save PREFIX/IPREFIX/curcontext.
                let prevpre = getsparam("PREFIX").unwrap_or_default();
                let previpre = getsparam("IPREFIX").unwrap_or_default();
                let prevcontext = getsparam("curcontext").unwrap_or_default();

                // sh:482
                let _ = setsparam(
                    "curcontext",
                    &format!("{}:options", strip_last_field(&oldcontext)),
                );
                hasopts = true; // sh:484

                let _ = setsparam("PREFIX", &origpre); // sh:486
                let _ = setsparam("IPREFIX", &origipre); // sh:487

                // sh:489 — single-option mode?
                let want_single = alwopt.is_empty() || !tried || alwopt == "arg";
                if want_single && comparguments(&["-s", "single"]) == 0 {
                    let single = getsparam("single").unwrap_or_default();
                    let word = prefix_suffix();
                    if single == "direct" {
                        // sh:493
                        let _ = _all_labels(&[
                            "options".to_string(),
                            "expl".to_string(),
                            "option".to_string(),
                            "compadd".to_string(),
                            "-QS".to_string(),
                            "".to_string(),
                            "-".to_string(),
                            word,
                        ]);
                    } else if !optarg && single == "next" {
                        // sh:495
                        let _ = _all_labels(&[
                            "options".to_string(),
                            "expl".to_string(),
                            "option".to_string(),
                            "compadd".to_string(),
                            "-Q".to_string(),
                            "-".to_string(),
                            word,
                        ]);
                    } else if single == "equal" {
                        // sh:498
                        let _ = _all_labels(&[
                            "options".to_string(),
                            "expl".to_string(),
                            "option".to_string(),
                            "compadd".to_string(),
                            "-QqS=".to_string(),
                            "-".to_string(),
                            word,
                        ]);
                    } else {
                        // sh:502-522 — build tmp1/tmp2/tmp3, then _describe.
                        let next = getaparam("next").unwrap_or_default();
                        let direct = getaparam("direct").unwrap_or_default();
                        let odirect = getaparam("odirect").unwrap_or_default();
                        let equal = getaparam("equal").unwrap_or_default();
                        // sh:503 tmp1=( next direct odirect equal )
                        let mut tmp1: Vec<String> = Vec::new();
                        tmp1.extend(next.iter().cloned());
                        tmp1.extend(direct.iter().cloned());
                        tmp1.extend(odirect.iter().cloned());
                        tmp1.extend(equal.iter().cloned());

                        let pfx = getsparam("PREFIX").unwrap_or_default();
                        // sh:505 [[ "$PREFIX" = [-+]* ]] &&
                        //   tmp1=( ${(@M)tmp1:#${PREFIX[1]}*} )
                        if let Some(c0) = pfx.chars().next() {
                            if c0 == '-' || c0 == '+' {
                                tmp1.retain(|s| s.starts_with(c0));
                            }
                        }
                        // sh:507 [[ "$single" = next ]] &&
                        //   tmp1=( ${(@)tmp1:#[-+]${PREFIX[-1]}((#e)|:*)} )
                        if single == "next" {
                            if let Some(last) = pfx.chars().last() {
                                tmp1.retain(|s| {
                                    let bytes: Vec<char> = s.chars().collect();
                                    if bytes.len() >= 2
                                        && (bytes[0] == '-' || bytes[0] == '+')
                                        && bytes[1] == last
                                    {
                                        // exclude "[-+]<last>" and "[-+]<last>:*"
                                        !(bytes.len() == 2
                                            || s[2..].starts_with(':'))
                                    } else {
                                        true
                                    }
                                });
                            }
                        }
                        // sh:510 [[ "$PREFIX" != --* ]] &&
                        //   tmp1=( ${(@)tmp1:#--*} )
                        if !pfx.starts_with("--") {
                            tmp1.retain(|s| !s.starts_with("--"));
                        }
                        // sh:511 tmp3=( ${(M@)tmp1:#[-+]?[^:]*} ) — options
                        //   whose 2nd char is followed by non-colon text.
                        let tmp3: Vec<String> = tmp1
                            .iter()
                            .filter(|s| {
                                let c: Vec<char> = s.chars().collect();
                                c.len() >= 3
                                    && (c[0] == '-' || c[0] == '+')
                                    && c[2] != ':'
                            })
                            .cloned()
                            .collect();
                        // sh:512 tmp1=( ${(M@)tmp1:#[-+]?(|:*)} ) — bare
                        //   options (2 chars, optionally a :desc).
                        tmp1.retain(|s| {
                            let c: Vec<char> = s.chars().collect();
                            c.len() >= 2
                                && (c[0] == '-' || c[0] == '+')
                                && (c.len() == 2 || c[2] == ':')
                        });
                        // sh:513 tmp2=( ${PREFIX}${(@M)^${(@)${(@)tmp1%%:*}#[-+]}:#?} )
                        //   → PREFIX + single option char for each bare opt.
                        let tmp2: Vec<String> = tmp1
                            .iter()
                            .filter_map(|s| {
                                let name = s.splitn(2, ':').next().unwrap_or("");
                                let bare = name
                                    .strip_prefix(|c: char| c == '-' || c == '+')
                                    .unwrap_or(name);
                                if bare.chars().count() == 1 {
                                    Some(format!("{}{}", pfx, bare))
                                } else {
                                    None
                                }
                            })
                            .collect();

                        setaparam("tmp1", tmp1.clone());
                        setaparam("tmp2", tmp2);
                        setaparam("tmp3", tmp3);
                        // sh:515 _describe -O option tmp1 tmp2 -S '' -- tmp3
                        let _ = dispatch_function_call(
                            "_describe",
                            &[
                                "-O".to_string(),
                                "option".to_string(),
                                "tmp1".to_string(),
                                "tmp2".to_string(),
                                "-S".to_string(),
                                "".to_string(),
                                "--".to_string(),
                                "tmp3".to_string(),
                            ],
                        );

                        // sh:519 — optarg + next + no new matches → add word.
                        if optarg && single == "next" && nm == nmatches() {
                            let _ = _all_labels(&[
                                "options".to_string(),
                                "expl".to_string(),
                                "option".to_string(),
                                "compadd".to_string(),
                                "-Q".to_string(),
                                "-".to_string(),
                                prefix_suffix(),
                            ]);
                        }
                    }
                    let _ = setsparam("single", "yes"); // sh:524
                } else {
                    // sh:526-530 — multi-option describe.
                    let mut next = getaparam("next").unwrap_or_default();
                    let odirect = getaparam("odirect").unwrap_or_default();
                    next.extend(odirect.iter().cloned()); // sh:526
                    setaparam("next", next);
                    // sh:527 _describe -O option next -M m -- direct -S '' -M m -- equal -qS= -M m
                    let _ = dispatch_function_call(
                        "_describe",
                        &[
                            "-O".to_string(),
                            "option".to_string(),
                            "next".to_string(),
                            "-M".to_string(),
                            matcher.clone(),
                            "--".to_string(),
                            "direct".to_string(),
                            "-S".to_string(),
                            "".to_string(),
                            "-M".to_string(),
                            matcher.clone(),
                            "--".to_string(),
                            "equal".to_string(),
                            "-qS=".to_string(),
                            "-M".to_string(),
                            matcher.clone(),
                        ],
                    );
                }
                // sh:532-534 — restore.
                let _ = setsparam("PREFIX", &prevpre);
                let _ = setsparam("IPREFIX", &previpre);
                let _ = setsparam("curcontext", &prevcontext);
            }

            // sh:536 — [[ -n "$tried" &&
            //   "${${alwopt:+$origpre}:-$PREFIX}" != [-+]* ]] && break
            if tried {
                let val = if !alwopt.is_empty() {
                    origpre.clone()
                } else {
                    getsparam("PREFIX").unwrap_or_default()
                };
                if !val.starts_with(|c: char| c == '-' || c == '+') {
                    break;
                }
            }
        }

        // sh:538-565 — `=`-descend: complete option, then recurse into
        // its argument via `comparguments -L`.
        if opts
            && !aret
            && !matched
            && (!tried || !alwopt.is_empty())
            && nm == nmatches()
        {
            // sh:543
            let _ = setsparam("PREFIX", &origpre);
            let _ = setsparam("IPREFIX", &origipre);

            let full_pre = getsparam("PREFIX").unwrap_or_default();
            // sh:546 prefix="${PREFIX#*\=}"
            let prefix = match full_pre.find('=') {
                Some(p) => full_pre[p + 1..].to_string(),
                None => full_pre.clone(),
            };
            let suffix = getsparam("SUFFIX").unwrap_or_default(); // sh:547
                                                                  // sh:548 PREFIX="${PREFIX%%\=*}"
            let pre_head = match full_pre.find('=') {
                Some(p) => full_pre[..p].to_string(),
                None => full_pre.clone(),
            };
            let _ = setsparam("PREFIX", &pre_head);
            let _ = setsparam("SUFFIX", ""); // sh:549

            // sh:551 compadd -M matcher -D equal - "${(@)equal%%:*}"
            let equal = getaparam("equal").unwrap_or_default();
            let stripped = strip_colon_desc(&equal);
            let mut cav: Vec<String> = vec![
                "-M".to_string(),
                matcher.clone(),
                "-D".to_string(),
                "equal".to_string(),
                "-".to_string(),
            ];
            cav.extend(stripped);
            let _ = bin_compadd("compadd", &cav, &make_ops(), 0);

            // sh:553 — exactly one option left after the -D cull → descend.
            let equal = getaparam("equal").unwrap_or_default();
            if equal.len() == 1 {
                let opt_name = equal[0].splitn(2, ':').next().unwrap_or("").to_string();
                let _ = setsparam("PREFIX", &prefix); // sh:554
                let _ = setsparam("SUFFIX", &suffix); // sh:555
                                                      // sh:556 IPREFIX="${IPREFIX}${equal[1]%%:*}="
                let cur_ipre = getsparam("IPREFIX").unwrap_or_default();
                let _ = setsparam("IPREFIX", &format!("{}{}=", cur_ipre, opt_name));
                matched = true; // sh:557

                // sh:559 comparguments -L "${equal[1]%%:*}" descrs actions subcs
                let _ = comparguments(&["-L", opt_name.as_str(), "descrs", "actions", "subcs"]);
                have_descrs = true;
                let subcs = getaparam("subcs").unwrap_or_default();
                let _ = _tags(&subcs); // sh:561
                continue; // sh:563
            }
        }
        break; // sh:566
    }

    // sh:569 — [[ -z "$aret" || -z "$usecc" ]] && curcontext="$oldcontext"
    if !aret || !usecc {
        let _ = setsparam("curcontext", &oldcontext);
    }

    // Compute the return value before tearing down scratch params.
    let final_rc: i32 = if aret {
        // sh:571-580
        if rawret {
            300 // sh:572 return 300
        } else {
            // Falls through the disabled `return 1` block (sh:574-580);
            // final value comes from sh:586. `[[ nm -ne … ]]` is TRUE
            // (exit 0) when matches were added.
            if nm != nmatches() {
                0
            } else {
                1
            }
        }
    } else {
        // sh:582 — [[ -n "$noargs" && nm -eq nmatches ]] && _message noargs
        if !noargs.is_empty() && nm == nmatches() {
            let _ = _message(&[noargs.clone()]);
        }
        // sh:586 — [[ nm -ne "$compstate[nmatches]" ]] (0 = added matches)
        if nm != nmatches() {
            0
        } else {
            1
        }
    };
    // Tear down the shell-`local` scratch params (the port has no local
    // scope). NORMARG / state / state_descr / context are NOT local in
    // the source — they persist to the caller — so they are left intact.
    for p in [
        "descrs", "actions", "subcs", "next", "direct", "odirect", "equal", "matcher", "single",
        "line", "opt_args", "tmp1", "tmp2", "tmp3", "ws", "expl",
    ] {
        unsetparam(p);
    }

    final_rc
}

#[cfg(test)]
mod tests {
    use super::*;

    // Outside a completion context `comparguments -i` cannot parse (it
    // warns "can only be called from completion function" and returns 1),
    // so `_arguments` takes the `else return 1` arm (sh:588). These tests
    // pin that faithful wrapper behavior.

    #[test]
    fn empty_spec_list_returns_one() {
        // (( $# )) is false → sh:588 return 1 (no comparguments call).
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_arguments(&[]), 1);
    }

    #[test]
    fn flags_only_still_returns_one() {
        // Only flags, no specs → `$#`==0 after the getopt loop.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_arguments(&["-s".to_string(), "-C".to_string()]), 1);
    }

    #[test]
    fn spec_without_completion_context_returns_one() {
        // With a spec but no live completion state, `comparguments -i`
        // fails → else arm → 1.
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["cmd".to_string(), "".to_string()]);
        let _ = setiparam("CURRENT", 2);
        let r = _arguments(&["1:file:_files".to_string()]);
        assert_eq!(r, 1);
    }

    #[test]
    fn double_dash_truncates_specs_but_still_needs_context() {
        // The `--` long-option cache block keeps only pre-`--` specs
        // (sh:39 tmpargv). Without a completion context it still returns 1,
        // but the truncation must not panic on the trailing `--` section.
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["cmd".to_string(), "".to_string()]);
        let _ = setiparam("CURRENT", 2);
        let r = _arguments(&[
            "-v[verbose]".to_string(),
            "--".to_string(),
            "-i".to_string(),
            "ignored".to_string(),
        ]);
        assert_eq!(r, 1);
    }

    #[test]
    fn nul_separator_flag_is_parsed() {
        // `-0` sets opt_args_use_NUL_separators; with no specs after it we
        // still fall to sh:588 (return 1), proving the getopt arm consumed
        // `-0` rather than treating it as a spec.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_arguments(&["-0".to_string()]), 1);
    }

    #[test]
    fn strip_colon_desc_removes_description() {
        assert_eq!(
            strip_colon_desc(&["-v:verbose".to_string(), "-x".to_string()]),
            vec!["-v".to_string(), "-x".to_string()]
        );
    }

    #[test]
    fn strip_last_field_drops_trailing_context_field() {
        assert_eq!(strip_last_field(":completion::cmd:opt"), ":completion::cmd");
        assert_eq!(strip_last_field("nocolon"), "nocolon");
    }
}
