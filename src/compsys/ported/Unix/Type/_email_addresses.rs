//! Port of `_email_addresses` from
//! `Completion/Unix/Type/_email_addresses`.
//!
//! Full upstream body (187 lines, faithful):
//! ```text
//! sh: 17  _email-mail/-mutt/-mush  → parse `alias` lines from rcfiles, return 300
//! sh: 34  _email-MH   → `ali` output via _call_program, return 300
//! sh: 40  _email-pine → parse ~/.addressbook, return 300
//! sh: 46  _email-ldap → ldapsearch via `filter` style, own compadd
//! sh: 76  _email-local→ _hosts / _users (user@host)
//! sh: 90  _email_addresses() {
//! sh: 94-118  RFC-822 pattern language (__specialx … __addrspec, __addresses)
//! sh:119    zparseopts -D -E -A opts n: s: c
//! sh:120    set -- "$@" -M 'r:|[.@]=* r:|=* m:{a-zA-Z}={A-Za-z}'
//! sh:122-130 -s separator handling (compset -P / -S)
//! sh:133-140 build `files` assoc + `plugins` list
//! sh:142-184 _tags email-$plugins; while _tags; do per-plugin _requested/_next_label
//! ```
//!
//! Approximations (available-primitive limits, never faked):
//!  * `$~__addrspec` (sh:170) — the RFC-822 addr-spec is a zsh
//!    extended-glob pattern; `(SM)…##` returns the matched substring.
//!    We approximate with a `localpart@domain` token extractor
//!    (`extract_addrspec`); the full grammar strings are still built
//!    verbatim (see `__specialx`…`__addresses`) and passed through.
//!  * `$~__addresses$opts[-s]` count (sh:124) — the exact
//!    "chars before the last unquoted separator" computation via the
//!    glob backreference is approximated by a greedy
//!    `compset -P "*<sep>"` (strips through the LAST separator).
//!  * `_email-ldap` (sh:59) — depends on the external `ldapsearch`
//!    binary + the `filter` style; without the style it returns 1
//!    immediately (faithful early-out).
//!  * `_email-MH` (sh:36) — depends on the external `ali` binary.

use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::exec::dispatch_function_call;
use crate::ported::hashtable::shfunctab_lock;
use crate::ported::utils::getshfunc;
use crate::ported::modules::zutil::{bin_zformat, bin_zparseopts, lookupstyle, zstyletab};
use crate::ported::params::{getaparam, gethkparam, gethparam, getsparam, setaparam, unsetparam};
use crate::ported::zle::compcore::set_compstate_str;
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{options, MAX_OPS};
use std::fs;
use std::path::Path;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

// --- small helpers -------------------------------------------------

/// Tilde expansion for config-file paths (`~`, `~/…`).
fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with('/') {
            if let Ok(home) = std::env::var("HOME") {
                return format!("{}{}", home, rest);
            }
        }
    }
    s.to_string()
}

/// `${~word}` for a filesystem path: tilde expansion only.
fn expand_word(s: &str) -> String {
    expand_tilde(s)
}

/// `${s/from/to}` — replace first literal occurrence of `from`.
fn replace_first(s: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return s.to_string();
    }
    match s.find(from) {
        Some(i) => format!("{}{}{}", &s[..i], to, &s[i + from.len()..]),
        None => s.to_string(),
    }
}

/// `${line/[[:blank:]]##/:}` — replace the first run of blanks with a
/// single `:`.
fn repl_blank_colon(s: &str) -> String {
    if let Some(start) = s.find([' ', '\t']) {
        let b = s.as_bytes();
        let mut end = start;
        while end < s.len() && (b[end] == b' ' || b[end] == b'\t') {
            end += 1;
        }
        format!("{}:{}", &s[..start], &s[end..])
    } else {
        s.to_string()
    }
}

/// `${line/\t[^\t]#\t/:}` — replace the first `TAB … TAB` field with a
/// single `:` (pine `.addressbook` middle-field collapse).
fn replace_tab_field(s: &str) -> String {
    if let Some(a) = s.find('\t') {
        if let Some(rel) = s[a + 1..].find('\t') {
            let b = a + 1 + rel;
            return format!("{}:{}", &s[..a], &s[b + 1..]);
        }
    }
    s.to_string()
}

/// Approximation of `${(SM)value##$~__addrspec}` (sh:170) — extract
/// the `localpart@domain` token. `_pattern` is the verbatim zsh glob
/// (kept for provenance; the match itself is approximated).
fn extract_addrspec(s: &str, _pattern: &str) -> String {
    let inner = match (s.find('<'), s.find('>')) {
        (Some(a), Some(b)) if a < b => &s[a + 1..b],
        _ => s,
    };
    inner
        .split_whitespace()
        .find(|t| t.contains('@'))
        .map(|t| t.trim_matches('"').to_string())
        .unwrap_or_default()
}

/// `zstyle -t ctx style` — true only when the style is set truthy.
fn zstyle_test(ctx: &str, style: &str) -> bool {
    zstyletab.lock().ok().and_then(|t| t.test_bool(ctx, style)) == Some(true)
}

/// `$+opts[key]` — does the zparseopts assoc contain `key`?
fn assoc_has(name: &str, key: &str) -> bool {
    gethkparam(name)
        .unwrap_or_default()
        .iter()
        .any(|k| k == key)
}

/// `$opts[key]` — value stored for `key` (empty for boolean flags).
fn assoc_get(name: &str, key: &str) -> Option<String> {
    let keys = gethkparam(name).unwrap_or_default();
    let vals = gethparam(name).unwrap_or_default();
    keys.iter()
        .position(|k| k == key)
        .and_then(|i| vals.get(i).cloned())
}

// --- plugins -------------------------------------------------------

/// sh:18-30 `_email-mail` (also `_email-mutt`, `_email-mush`). Parse
/// `alias NAME REST` lines from the rcfile and any `source`d files.
fn email_mail(config_path: &str, reply: &mut Vec<String>) -> i32 {
    // sh:21  rcfiles=( $files[$plugin] )
    let mut rcfiles: Vec<String> = vec![config_path.to_string()];
    // sh:22-24  follow `source …` lines (array grows during the loop)
    let mut i = 0usize;
    while i < rcfiles.len() {
        if let Ok(content) = fs::read_to_string(expand_word(&rcfiles[i])) {
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("source") {
                    let trimmed = rest.trim_start_matches([' ', '\t']);
                    // require ≥1 blank after `source`
                    if trimmed.len() < rest.len() && !trimmed.is_empty() {
                        // `(N)` — only include if the file exists
                        if Path::new(&expand_word(trimmed)).exists() {
                            rcfiles.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
        i += 1;
    }
    // sh:25  reply=()
    reply.clear();
    // sh:26-28
    for rc in &rcfiles {
        if let Ok(content) = fs::read_to_string(expand_word(rc)) {
            for line in content.lines() {
                if line.starts_with("alias") {
                    let rest = &line["alias".len()..];
                    let trimmed = rest.trim_start_matches([' ', '\t']);
                    // require ≥1 blank after `alias` (${x##alias[[:blank:]]##})
                    if trimmed.len() < rest.len() {
                        reply.push(repl_blank_colon(trimmed));
                    }
                }
            }
        }
    }
    // sh:29
    300
}

/// sh:34-38 `_email-MH` — parse `ali` output via `_call_program`.
fn email_mh(reply: &mut Vec<String>) -> i32 {
    // sh:36  reply=( ${${(f)"$(_call_program aliases ali)"}/: /:} )
    let _ = dispatch_function_call("_call_program", &["aliases".to_string(), "ali".to_string()]);
    let out = getsparam("REPLY").unwrap_or_default();
    reply.clear();
    for line in out.lines() {
        reply.push(replace_first(line, ": ", ":"));
    }
    // sh:37
    300
}

/// sh:40-44 `_email-pine` — parse `~/.addressbook`.
fn email_pine(reply: &mut Vec<String>) -> i32 {
    let home = std::env::var("HOME").unwrap_or_default();
    reply.clear();
    if let Ok(content) = fs::read_to_string(format!("{}/.addressbook", home)) {
        for line in content.lines() {
            // sh:42  :#*DELETED*  then  :#\ *
            if line.contains("DELETED") || line.starts_with(' ') {
                continue;
            }
            // /\t[^\t]#\t/:  then  %%\t*
            let transformed = replace_tab_field(line);
            let cut = match transformed.find('\t') {
                Some(i) => &transformed[..i],
                None => &transformed,
            };
            reply.push(cut.to_string());
        }
    }
    // sh:43
    300
}

/// sh:46-74 `_email-ldap` — does its own completion (returns non-300).
fn email_ldap(args: &[String], curcontext: &str, curtag: &str) -> i32 {
    // sh:52  zparseopts -D -E -A opts c
    let opts_c = args.iter().any(|a| a == "-c");
    let passthru: Vec<String> = args.iter().filter(|a| *a != "-c").cloned().collect();

    // sh:54  zstyle -a … filter filter
    let filter = lookupstyle(&format!(":completion:{}:{}", curcontext, curtag), "filter");
    // sh:55  (( $#filter )) || return
    if filter.is_empty() {
        return 1;
    }

    // sh:57  filter=( "("${filter}"=${PREFIX}*${SUFFIX})" )
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let filt: Vec<String> = filter
        .iter()
        .map(|f| format!("({}={}*{})", f, prefix, suffix))
        .collect();
    // sh:58  (( $#filter > 1 )) && filter="(|"${(j..)filter}")"
    let filterstr = if filt.len() > 1 {
        format!("(|{})", filt.join(""))
    } else {
        filt.join("")
    };

    // sh:59  res=( ${(f)"$(_call_program $curtag ldapsearch -LLL $filter cn mail)"} )
    let _ = dispatch_function_call(
        "_call_program",
        &[
            curtag.to_string(),
            "ldapsearch".to_string(),
            "-LLL".to_string(),
            filterstr,
            "cn".to_string(),
            "mail".to_string(),
        ],
    );
    let out = getsparam("REPLY").unwrap_or_default();
    let res: Vec<String> = out.lines().map(|l| l.to_string()).collect();
    // sh:60  (( $#res > 1 )) || return
    if res.len() <= 1 {
        return 1;
    }

    // sh:62-70  for dn cn mail in "${res[@]}"  (three fields per entry)
    let specialx = "][()<>@,;:\\\".";
    let mut ali: Vec<String> = Vec::new();
    for chunk in res.chunks(3) {
        if chunk.len() < 3 {
            break;
        }
        let (cn, mail) = (&chunk[1], &chunk[2]);
        if opts_c {
            // sh:64  ali+=( "${mail#*: }" )
            ali.push(mail.splitn(2, ": ").nth(1).unwrap_or(mail).to_string());
        } else {
            // sh:66-68
            let mut cn = cn.splitn(2, ": ").nth(1).unwrap_or(cn).to_string();
            if cn.chars().any(|c| specialx.contains(c)) {
                cn = format!("\"{}\"", cn);
            }
            let mailv = mail.splitn(2, ": ").nth(1).unwrap_or(mail);
            ali.push(format!("{} <{}>", cn, mailv));
        }
    }
    // sh:71  compstate[insert]=menu
    let _ = set_compstate_str("insert", "menu");
    // sh:72-73  _wanted email-ldap expl 'matching name' compadd -U -i "$IPREFIX" -I "$ISUFFIX" "$@" -a - ali
    setaparam("ali", ali);
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let mut w = vec![
        "email-ldap".to_string(),
        "expl".to_string(),
        "matching name".to_string(),
        "compadd".to_string(),
        "-U".to_string(),
        "-i".to_string(),
        iprefix,
        "-I".to_string(),
        isuffix,
    ];
    w.extend(passthru);
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("ali".to_string());
    let ret = _wanted(&w);
    unsetparam("ali");
    ret
}

/// sh:76-88 `_email-local` — complete `user@host` Unix addresses.
fn email_local(args: &[String]) -> i32 {
    // sh:79  zparseopts -D -E -A opts c S:=suf
    let mut suf: Vec<String> = Vec::new();
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        let a = &args[i];
        if a == "-c" {
            i += 1;
        } else if a == "-S" && i + 1 < args.len() {
            suf.push("-S".to_string());
            suf.push(args[i + 1].clone());
            i += 2;
        } else if let Some(v) = a.strip_prefix("-S") {
            if !v.is_empty() {
                suf.push("-S".to_string());
                suf.push(v.to_string());
                i += 1;
            } else {
                rest.push(a.clone());
                i += 1;
            }
        } else {
            rest.push(a.clone());
            i += 1;
        }
    }

    // sh:81  if compset -P '*@'; then
    if bin_compset(
        "compset",
        &["-P".to_string(), "*@".to_string()],
        &make_ops(),
        0,
    ) == 0
    {
        // sh:82  _hosts "$@" "$suf[@]"
        let mut a = rest.clone();
        a.extend(suf);
        dispatch_function_call("_hosts", &a).unwrap_or(1)
    } else {
        // sh:84-87
        let mut suf2: Vec<String> = Vec::new();
        if bin_compset(
            "compset",
            &["-S".to_string(), "@*".to_string()],
            &make_ops(),
            0,
        ) != 0
        {
            suf2 = vec!["-qS".to_string(), "@".to_string()];
        }
        let mut a = suf2;
        a.extend(rest);
        dispatch_function_call("_users", &a).unwrap_or(1)
    }
}

/// `_call_function fret _email-$plugin "$@" $args` (sh:156). Returns
/// `None` when the plugin name is unknown (mirrors call failure).
fn call_email_plugin(
    plugin: &str,
    call_args: &[String],
    files: &[(String, String)],
    curcontext: &str,
    curtag: &str,
    reply: &mut Vec<String>,
) -> Option<i32> {
    // sh:17/31/32/34/40/46/76  each built-in plugin is defined behind a
    // `(( $+functions[_email-<name>] )) ||` guard, so a shell function of
    // that name — whether a user override of a built-in plugin or an
    // entirely third-party one (sh:10-14) — always wins. `_call_function`
    // (sh:156) then invokes it by name.
    let fname = format!("_email-{}", plugin);
    if getshfunc(&fname).is_some() {
        let fret = dispatch_function_call(&fname, call_args).unwrap_or(1);
        // sh:11-13  a plugin that returns 300 has left its results in the
        // (function-local) `reply` array; read it back for sh:162-176.
        if fret == 300 {
            *reply = getaparam("reply").unwrap_or_default();
        }
        return Some(fret);
    }

    match plugin {
        "mail" | "mutt" | "mush" => {
            let cfg = files
                .iter()
                .find(|(k, _)| k == plugin)
                .map(|(_, v)| v.clone())
                .unwrap_or_default();
            Some(email_mail(&cfg, reply))
        }
        "MH" => Some(email_mh(reply)),
        "pine" => Some(email_pine(reply)),
        "ldap" => Some(email_ldap(call_args, curcontext, curtag)),
        "local" => Some(email_local(call_args)),
        _ => None,
    }
}

// --- main ----------------------------------------------------------

/// `_email_addresses` — complete e-mail addresses via pluggable
/// backends. `-c` = bare `user@host`; `-n plugin` = that plugin's
/// aliases; `-s sep` = a separator-delimited list.
pub fn _email_addresses(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_email_addresses");
    // sh:96-118 — RFC-822 pattern language, built verbatim.
    let __specialx = "][()<>@,;:\\\".";
    let __spacex = " \t"; // Space, tab
    let __specials = format!("[{}]", __specialx);
    let __atom = format!("[^{}{}]##", __specialx, __spacex);
    let __space = format!("[{}]#", __spacex); // Really, space or comment
    let __qtext = "[^\"\\\\]";
    let __qpair = "\\\\?";
    let __beginq = "\"";
    let __endq = "(|[^\\\\])\"";
    let __dot = format!("{}.{}", __space, __space);

    let __domainref = __atom.clone();
    let __domainlit = format!("\\[([^]]|{})#(|[^\\\\])\\]", __qpair);
    let __quotedstring = format!("{}({}|{})#{}", __beginq, __qtext, __qpair, __endq);
    let __word = format!("({}|{})", __atom, __quotedstring);
    let __phrase = format!("({}{}{})#", __space, __word, __space); // Strictly, should use `##'
    let __localpart = format!("{}({}{})#", __word, __dot, __word);

    let __subdomain = format!("({}|{})", __domainref, __domainlit);
    let __domain = format!("{}({}{})#", __subdomain, __dot, __subdomain);
    let __addrspec = format!("{}{}@{}{}", __localpart, __space, __space, __domain);

    let __addresses = format!("({}|{})##", __qtext, __quotedstring);
    // Built verbatim for provenance; matching is approximated (see notes).
    let _ = (&__specials, &__phrase, &__addresses);

    // sh:119  zparseopts -D -E -A opts n: s: c
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-E".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-A".to_string(),
            "opts".to_string(),
            "n:".to_string(),
            "s:".to_string(),
            "c".to_string(),
        ],
        &make_ops(),
        0,
    );
    let mut argv_at = getaparam(src).unwrap_or_default();
    unsetparam(src);

    // sh:120  set -- "$@" -M 'r:|[.@]=* r:|=* m:{a-zA-Z}={A-Za-z}'
    argv_at.push("-M".to_string());
    argv_at.push("r:|[.@]=* r:|=* m:{a-zA-Z}={A-Za-z}".to_string());

    // sh:122-130  -s separator handling
    if let Some(sep) = assoc_get("opts", "-s").filter(|s| !s.is_empty()) {
        // sh:124  remove up to the last unquoted separator (approx —
        //   see module note; the __addresses count is approximated by
        //   a greedy compset -P).
        let _ = &__addresses;
        let prefix = getsparam("PREFIX").unwrap_or_default();
        if prefix.contains(&sep) {
            let _ = bin_compset(
                "compset",
                &["-P".to_string(), format!("*{}", sep)],
                &make_ops(),
                0,
            );
        }
        // sh:129  compset -S "$opts[-s]*" || set -- -q -S "$opts[-s]" "$@"
        if bin_compset(
            "compset",
            &["-S".to_string(), format!("{}*", sep)],
            &make_ops(),
            0,
        ) != 0
        {
            let mut na = vec!["-q".to_string(), "-S".to_string(), sep.clone()];
            na.extend(argv_at);
            argv_at = na;
        }
    }

    // sh:133-135  muttrc location
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let muttrc = {
        let styled = lookupstyle(
            &format!(":completion:{}:email-addresses", curcontext),
            "muttrc",
        )
        .first()
        .cloned();
        match styled {
            Some(m) if !m.is_empty() => m,
            _ => {
                if Path::new(&expand_word("~/mutt/muttrc")).exists() {
                    "~/mutt/muttrc".to_string()
                } else {
                    "~/.muttrc".to_string()
                }
            }
        }
    };

    // sh:136  files=( MH … mutt … mush … mail … pine … )
    let files: Vec<(String, String)> = vec![
        (
            "MH".to_string(),
            std::env::var("MH")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "~/.mh_profile".to_string()),
        ),
        ("mutt".to_string(), muttrc),
        ("mush".to_string(), "~/.mushrc".to_string()),
        (
            "mail".to_string(),
            std::env::var("MAILRC")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "~/.mailrc".to_string()),
        ),
        ("pine".to_string(), "~/.addressbook".to_string()),
    ];

    // sh:137-140  plugins = config-less plugin fns + config-backed
    //   plugins whose file exists.
    //
    // sh:138  ${${(k)functions[(I)_email-*]#*-}:#(${(kj.|.)~files})}
    //   `(k)functions[(I)_email-*]` enumerates EVERY key of the live
    //   `$functions` table matching `_email-*` — the seven plugins the
    //   upstream file defines at sh:17-88 AND any third-party plugin
    //   function the user has loaded (sh:10-14 documents that "New
    //   plugins will be picked up and run automatically"). `#*-` strips
    //   through the first `-`, so `_email-sleuth` → `sleuth`.
    //
    //   In this port the seven upstream plugins are native Rust (they are
    //   not in `shfunctab`), so `known` stands in for them; the live table
    //   supplies everything else.
    let known = ["mail", "mutt", "mush", "MH", "pine", "ldap", "local"];
    let config_keys: Vec<&str> = files.iter().map(|(k, _)| k.as_str()).collect();
    let mut plugins: Vec<String> = Vec::new();
    for f in &known {
        if !config_keys.contains(f) {
            plugins.push(f.to_string());
        }
    }
    // The rest of `${(k)functions[(I)_email-*]}` — plugins that only
    // exist as shell functions.
    let mut fn_plugins: Vec<String> = Vec::new();
    if let Ok(tab) = shfunctab_lock().read() {
        for (k, _) in tab.iter() {
            // `(I)_email-*` then `#*-`
            if let Some(name) = k.strip_prefix("_email-") {
                if !name.is_empty() {
                    fn_plugins.push(name.to_string());
                }
            }
        }
    }
    fn_plugins.sort();
    fn_plugins.dedup();
    for f in fn_plugins {
        // `:#(${(kj.|.)~files})` — drop the config-backed names; they are
        // re-added below only when their config file exists.
        if !config_keys.contains(&f.as_str()) && !plugins.iter().any(|p| *p == f) {
            plugins.push(f);
        }
    }
    for (k, v) in &files {
        if Path::new(&expand_word(v)).exists() {
            plugins.push(k.clone());
        }
    }

    // sh:142  ret=1
    let mut ret: i64 = 1;
    // sh:143  _tags email-$plugins
    let tag_names: Vec<String> = plugins.iter().map(|p| format!("email-{}", p)).collect();
    let _ = _tags(&tag_names);

    // sh:144  while _tags; do
    loop {
        if _tags(&[]) != 0 {
            break;
        }
        // sh:145  for plugin in $plugins
        for plugin in &plugins {
            // sh:146  if _requested email-$plugin
            if _requested(&[format!("email-{}", plugin)]) == 0 {
                // sh:147  while _next_label email-$plugin expl 'email address'
                loop {
                    let nl = vec![
                        format!("email-{}", plugin),
                        "expl".to_string(),
                        "email address".to_string(),
                    ];
                    if _next_label(&nl) != 0 {
                        break;
                    }

                    // sh:149  args=()
                    let mut plugin_args: Vec<String> = Vec::new();
                    // sh:150-154
                    let curtag = getsparam("curtag").unwrap_or_default();
                    if assoc_has("opts", "-c")
                        || zstyle_test(
                            &format!(":completion:{}:{}", curcontext, curtag),
                            "strip-comments",
                        )
                    {
                        plugin_args.push("-c".to_string());
                    }

                    // sh:156  _call_function fret _email-$plugin "$@" $args
                    let mut call_args = argv_at.clone();
                    call_args.extend(plugin_args.iter().cloned());
                    let mut reply: Vec<String> = Vec::new();
                    let fret = match call_email_plugin(
                        plugin,
                        &call_args,
                        &files,
                        &curcontext,
                        &curtag,
                        &mut reply,
                    ) {
                        Some(f) => f,
                        None => {
                            // sh:157  _message "$plugin: plugin not found"; continue
                            let _ = _message(&[format!("{}: plugin not found", plugin)]);
                            continue;
                        }
                    };

                    // sh:160  ret=$(( ret && fret ))
                    ret = if ret != 0 && fret != 0 { 1 } else { 0 };

                    // sh:162  if (( fret == 300 ))
                    if fret == 300 {
                        // sh:163  (( ! $+opts[-c] )) && [[ $opts[-n] = $plugin ]]
                        if !assoc_has("opts", "-c")
                            && assoc_get("opts", "-n").as_deref() == Some(plugin.as_str())
                        {
                            // sh:164  list-separator
                            let sep = lookupstyle(
                                &format!(":completion:{}:{}", curcontext, curtag),
                                "list-separator",
                            )
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "--".to_string());
                            // sh:165  zformat -a list " $sep " "${reply[@]}"
                            let mut zf =
                                vec!["-a".to_string(), "list".to_string(), format!(" {} ", sep)];
                            zf.extend(reply.iter().cloned());
                            let _ = bin_zformat("zformat", &zf, &make_ops(), 0);
                            // sh:166-167  _wanted mail-aliases expl 'alias' compadd "$@" -d list - ${reply%%:*}
                            let mut w = vec![
                                "mail-aliases".to_string(),
                                "expl".to_string(),
                                "alias".to_string(),
                                "compadd".to_string(),
                            ];
                            w.extend(argv_at.iter().cloned());
                            w.push("-d".to_string());
                            w.push("list".to_string());
                            w.push("-".to_string());
                            for r in &reply {
                                w.push(r.splitn(2, ':').next().unwrap_or("").to_string());
                            }
                            if _wanted(&w) == 0 {
                                ret = 0;
                            }
                            unsetparam("list");
                        } else {
                            // sh:169-174  transform reply
                            let new_reply: Vec<String> = if !plugin_args.is_empty() {
                                // sh:170  ${(SM)${reply#*:}##$~__addrspec}
                                reply
                                    .iter()
                                    .map(|r| {
                                        let after = r.splitn(2, ':').nth(1).unwrap_or(r);
                                        extract_addrspec(after, &__addrspec)
                                    })
                                    .collect()
                            } else {
                                // sh:173  keep elems with `@`, strip up to first `:`
                                reply
                                    .iter()
                                    .filter(|r| r.contains('@'))
                                    .map(|r| r.splitn(2, ':').nth(1).unwrap_or(r).to_string())
                                    .collect()
                            };
                            setaparam("reply", new_reply);
                            // sh:175  compadd -a "$@" "$expl[@]" reply
                            let expl = getaparam("expl").unwrap_or_default();
                            let mut c = vec!["-a".to_string()];
                            c.extend(argv_at.iter().cloned());
                            c.extend(expl);
                            c.push("reply".to_string());
                            if bin_compadd("compadd", &c, &make_ops(), 0) == 0 {
                                ret = 0;
                            }
                            unsetparam("reply");
                        }
                    }
                }
            }
        }
        // sh:181  (( ret )) || return 0
        if ret == 0 {
            return 0;
        }
    }

    // sh:184
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_config() {
        // sh:144-184 — sandbox home (no mailrc/muttrc/addressbook):
        //   the config-backed plugins drop out; ldap/local produce no
        //   matches without styles/network → the tag loop returns 1.
        let _g = crate::test_util::global_state_lock();
        std::env::set_var("HOME", "/nonexistent/sandbox");
        std::env::remove_var("MH");
        std::env::remove_var("MAILRC");
        let r = _email_addresses(&[]);
        assert_eq!(r, 1);
    }

    #[test]
    fn extract_addrspec_pulls_bare_address() {
        // sh:170 — addr-spec extraction from an RFC-822 phrase.
        assert_eq!(extract_addrspec("Jane Doe <j@x.io>", ""), "j@x.io");
        assert_eq!(extract_addrspec("u@host", ""), "u@host");
        assert_eq!(extract_addrspec("no address here", ""), "");
    }

    #[test]
    fn repl_blank_colon_first_run_only() {
        // sh:27 — `alias NAME rest` → `NAME:rest`.
        assert_eq!(repl_blank_colon("bob  bob@x.io more"), "bob:bob@x.io more");
        assert_eq!(repl_blank_colon("noblank"), "noblank");
    }
}
