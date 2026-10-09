//! Port of `_jails` from `Completion/BSD/Type/_jails`.
//!
//! Head comment (sh:3-7) — options:
//!   -0        include jid 0 as a match for the host system
//!   -o param  jail parameter to complete instead of jid -
//!                e.g. name, path, ip4.addr, host.hostname
//!
//! Upstream is 70 lines; the transcript below is abridged. The head comment
//! additionally documents `-c` (complete configured jails that are not
//! running) and `-f file` (override the config file, default
//! `sysrc -n jail_conf`), implemented by [`configured_jails`] (sh:23-50).
//! ```text
//! sh: 1  #autoload
//! sh:11  local addhost host param desc=1 configured
//! sh:12  local -a jails args expl fopt match mbegin mend
//! sh:13  zparseopts -D -K -E 0=addhost c=configured f:=fopt o:=param
//! sh:14  param=${param[2]:-name}
//! sh:16  jails=( ${${(f)"$(_call_program jails jls $param name)"}/ /:} )
//! sh:18  if [[ -n $configured ]]; then  # ... sh:50 fi
//! sh:52  case $param in
//! sh:53    jid) host=0 ;;
//! sh:54    name)
//! sh:55      host=0
//! sh:56      desc=0
//! sh:57    ;;
//! sh:58    path)
//! sh:59      host=/
//! sh:60      args=( -M 'r:|/=* r:|=*' )
//! sh:61    ;;
//! sh:62    ip4.addr) args=( -M 'r:|.=* r:|=*' ) ;;
//! sh:63  esac
//! sh:64  [[ -n $addhost && -n $host ]] && jails+=( "$host:$HOST" )
//! sh:66  if (( desc )); then
//! sh:67    _describe -t jails jail jails "$@" "$args[@]"
//! sh:68  else
//! sh:69    _wanted jails expl jail compadd "$@" "$args[@]" - ${jails%:*}
//! sh:70  fi
//! ```

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_describe::_describe;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam, unsetparam};

/// sh:13 — bridge for `zparseopts -D -K -E 0=addhost c=configured f:=fopt o:=param`. `-E`
/// means the whole argv is scanned (not just a leading option run);
/// `-D` removes matched flags/values, leaving everything else as the
/// passthrough `rest` (the `"$@"` later handed to `_describe`/`_wanted`).
struct JailOpts {
    addhost: bool,
    configured: bool,
    fopt: Option<String>,
    param: Option<String>,
    rest: Vec<String>,
}

fn zparse_0_o(args: &[String]) -> JailOpts {
    let mut o = JailOpts {
        addhost: false,
        configured: false,
        fopt: None,
        param: None,
        rest: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-0" => {
                o.addhost = true; // sh:13 0=addhost
                i += 1;
            }
            "-c" => {
                o.configured = true; // sh:13 c=configured
                i += 1;
            }
            "-f" | "-o" => {
                // sh:13 f:=fopt / o:=param — value-taking, value in the next word.
                if i + 1 < args.len() {
                    let v = Some(args[i + 1].clone());
                    if a == "-f" {
                        o.fopt = v;
                    } else {
                        o.param = v;
                    }
                    i += 2;
                } else {
                    i += 1;
                }
            }
            _ if a.starts_with("-f") || a.starts_with("-o") => {
                // `-fFILE` / `-oPARAM`: value attached to the option word.
                let v = Some(a[2..].to_string());
                if a.starts_with("-f") {
                    o.fopt = v;
                } else {
                    o.param = v;
                }
                i += 1;
            }
            _ => {
                o.rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    o
}

/// sh:21,32 — the `.include "path"` directive: `include_pat` is
/// `(#b)[[:space:]]#.include[[:space:]]##["']([^"']##)["']*`, matched
/// against the whole line. Returns `match[1]`.
fn include_path(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix(".include")?;
    let after_ws = rest.trim_start();
    if after_ws.len() == rest.len() {
        return None; // `[[:space:]]##` needs at least one space
    }
    let after_quote = after_ws.strip_prefix(['"', '\''])?;
    let end = after_quote.find(['"', '\''])?;
    (end > 0).then(|| &after_quote[..end])
}

/// sh:42 — `${content//$'\n'[[:space:]]#\{/' {'}`: a newline followed by
/// optional whitespace and `{` collapses to ` {`.
fn collapse_brace_lines(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(nl) = rest.find('\n') {
        out.push_str(&rest[..nl]);
        let tail = &rest[nl + 1..];
        match tail.trim_start().strip_prefix('{') {
            Some(after_brace) => {
                out.push_str(" {");
                rest = after_brace;
            }
            None => {
                out.push('\n');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// sh:43-44 — for one line, `[[ $line = [[:space:]]#[^#[:space:]]##[[:space:]]#\{* ]]`
/// then `${${line##[[:space:]]#}%%[[:space:]]#\{*}`: the jail name of a
/// `name {` block header, `None` when the line is not one.
fn block_name(line: &str) -> Option<&str> {
    let body = line.trim_start();
    let word_len = body
        .find(|c: char| c == '#' || c.is_whitespace())
        .unwrap_or(body.len());
    if word_len == 0 {
        return None;
    }
    // `[^#[:space:]]##` may itself contain `{`; otherwise `{` follows
    // optional whitespace after the word.
    let first_len = body.chars().next()?.len_utf8();
    if !body[first_len..word_len].contains('{') && !body[word_len..].trim_start().starts_with('{')
    {
        return None;
    }
    // `%%[[:space:]]#\{*`: cut from the whitespace run preceding the first `{`.
    let first_brace = body.find('{')?;
    Some(body[..first_brace].trim_end())
}

/// sh:18-50 — `if [[ -n $configured ]]`: names of the jails configured in
/// `jail.conf` (following `.include` directives) that are not running.
/// `running` is `running_names` (sh:48).
fn configured_jails(fopt: Option<&str>, running: &[String]) -> Vec<String> {
    // sh:23-27
    let jail_conf = match fopt {
        Some(f) => f.to_string(),
        None => {
            let (out, _) = call_program_capture(&[
                "paths".to_string(),
                "sysrc".to_string(),
                "-n".to_string(),
                "jail_conf".to_string(),
            ]);
            out.trim_end_matches('\n').to_string()
        }
    };

    // sh:29-35 — follow .include directives, then the main file.
    let mut conf_files: Vec<String> = Vec::new();
    if let Ok(text) = std::fs::read_to_string(&jail_conf) {
        // `while IFS= read -r line` drops an unterminated final line.
        let terminated = match text.rfind('\n') {
            Some(p) => &text[..p],
            None => "",
        };
        for line in terminated.split('\n') {
            if let Some(pat) = include_path(line) {
                // `${~match[1]}(N)` — pattern-expanded, nullglob.
                if let Ok(paths) = glob::glob(pat) {
                    conf_files.extend(paths.flatten().map(|p| p.to_string_lossy().into_owned()));
                }
            }
        }
        conf_files.push(jail_conf.clone());
    }

    // sh:37-45 — `for f in ${(u)conf_files}`
    let mut seen: Vec<&String> = Vec::new();
    let mut cjails: Vec<String> = Vec::new();
    for f in &conf_files {
        if seen.contains(&f) {
            continue;
        }
        seen.push(f);
        let Ok(raw) = std::fs::read_to_string(f) else {
            continue; // sh:40 `[[ -r $f ]] || continue`
        };
        let content = collapse_brace_lines(raw.trim_end_matches('\n')); // sh:41-42
        cjails.extend(content.split('\n').filter_map(block_name).map(String::from));
    }

    // sh:49 — `${(u)${cjails:#\*}:|running_names}`
    let mut jails: Vec<String> = Vec::new();
    for j in cjails {
        if j != "*" && !running.contains(&j) && !jails.contains(&j) {
            jails.push(j);
        }
    }
    jails
}

/// sh:14 — `${line/ /:}`: replace only the *first* space in a line
/// with a colon (zsh single-slash substitution replaces one match).
fn first_space_to_colon(line: &str) -> String {
    match line.find(' ') {
        Some(pos) => {
            let mut s = String::with_capacity(line.len());
            s.push_str(&line[..pos]);
            s.push(':');
            s.push_str(&line[pos + 1..]);
            s
        }
        None => line.to_string(),
    }
}

/// sh:69 — `${jails%:*}`: strip the shortest suffix starting at the
/// *last* colon (i.e. drop the trailing `:name` field).
fn strip_after_last_colon(s: &str) -> String {
    match s.rfind(':') {
        Some(pos) => s[..pos].to_string(),
        None => s.to_string(),
    }
}

/// `_jails` — complete FreeBSD jail identifiers via `jls`.
pub fn _jails(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_jails");
    // sh:10 — `local -a jails args expl`. `jails` is assigned with
    // `setaparam` at rs:156 and `expl` is filled by `_description`
    // through the name handed to `_wanted` at rs:175, so both were born
    // at level 0 (shared.rs:16-30). `args` stays Rust-side. Measured on
    // `lkjails <TAB>`: zsh leaves `expl` unset, zshrs left it populated
    // (`jails` itself needs a host with jails to observe, but it is the
    // same `setaparam` on the same declaration line).
    crate::compsys::ported::shared::declare_locals(
        &["jails", "expl"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    // sh:13
    let JailOpts {
        addhost,
        configured,
        fopt,
        param: param_opt,
        rest,
    } = zparse_0_o(args);
    // sh:14 — ${param[2]:-name}: default when unset OR empty.
    let param = param_opt
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "name".to_string());

    // sh:16
    let _ = call_program_capture(&[
        "jails".to_string(),
        "jls".to_string(),
        param.clone(),
        "name".to_string(),
    ]);
    let mut jails: Vec<String> = getsparam("REPLY")
        .unwrap_or_default()
        .lines()
        .map(first_space_to_colon)
        .collect();

    // sh:18-50
    if configured {
        let running: Vec<String> = jails.iter().map(|s| strip_after_last_colon(s)).collect(); // sh:48
        jails = configured_jails(fopt.as_deref(), &running);
    }

    // sh:52-63
    let mut host: Option<String> = None;
    let mut desc = true;
    let mut extra_args: Vec<String> = Vec::new();
    match param.as_str() {
        "jid" => host = Some("0".to_string()), // sh:53
        "name" => {
            host = Some("0".to_string()); // sh:55
            desc = false; // sh:56
        }
        "path" => {
            host = Some("/".to_string()); // sh:59
            extra_args = vec!["-M".to_string(), "r:|/=* r:|=*".to_string()]; // sh:60
        }
        "ip4.addr" => {
            extra_args = vec!["-M".to_string(), "r:|.=* r:|=*".to_string()]; // sh:62
        }
        _ => {}
    }

    // sh:64
    if addhost {
        if let Some(h) = &host {
            let hostname = getsparam("HOST").unwrap_or_default();
            jails.push(format!("{}:{}", h, hostname));
        }
    }

    // sh:66-70
    if desc {
        // sh:67  _describe -t jails jail jails "$@" "$args[@]"
        setaparam("jails", jails);
        let mut a: Vec<String> = vec![
            "-t".to_string(),
            "jails".to_string(),
            "jail".to_string(),
            "jails".to_string(),
        ];
        a.extend(rest);
        a.extend(extra_args);
        // sh:67 is a bare command word — reach it by name so `$fpath`/shfunc
        // arbitration runs and `_describe`'s locals land in its own scope.
        let r = _describe(&a);
        unsetparam("jails");
        r
    } else {
        // sh:69  _wanted jails expl jail compadd "$@" "$args[@]" - ${jails%:*}
        let stripped: Vec<String> = jails.iter().map(|s| strip_after_last_colon(s)).collect();
        let mut a: Vec<String> = vec![
            "jails".to_string(),
            "expl".to_string(),
            "jail".to_string(),
            "compadd".to_string(),
        ];
        a.extend(rest);
        a.extend(extra_args);
        a.push("-".to_string());
        a.extend(stripped);
        _wanted(&a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_pulls_0_and_o_leaving_rest() {
        let o = zparse_0_o(&[
            "-0".to_string(),
            "-J".to_string(),
            "grp".to_string(),
            "-o".to_string(),
            "path".to_string(),
            "-c".to_string(),
            "-f".to_string(),
            "/etc/j.conf".to_string(),
        ]);
        assert!(o.addhost && o.configured);
        assert_eq!(o.fopt.as_deref(), Some("/etc/j.conf"));
        assert_eq!(o.param.as_deref(), Some("path"));
        assert_eq!(o.rest, vec!["-J".to_string(), "grp".to_string()]);
    }

    #[test]
    fn zparse_defaults_when_absent() {
        let o = zparse_0_o(&["foo".to_string()]);
        assert!(!o.addhost && !o.configured);
        assert_eq!(o.fopt, None);
        assert_eq!(o.param, None);
        assert_eq!(o.rest, vec!["foo".to_string()]);
    }

    #[test]
    fn include_path_requires_space_and_quotes() {
        assert_eq!(
            include_path("  .include \"/etc/jail.d/*.conf\" ;"),
            Some("/etc/jail.d/*.conf")
        );
        assert_eq!(include_path(".include '/a/b'"), Some("/a/b"));
        assert_eq!(include_path(".include\"/a\""), None);
        assert_eq!(include_path("# .include \"/a\""), None);
    }

    #[test]
    fn block_names_cover_both_brace_styles() {
        let c = collapse_brace_lines("web {\n  x = 1;\n}\n  db\n  {\n}\n# c {\n* {\n");
        let names: Vec<&str> = c.split('\n').filter_map(block_name).collect();
        assert_eq!(names, vec!["web", "db", "*"]);
    }

    #[test]
    fn first_space_to_colon_replaces_only_first() {
        assert_eq!(first_space_to_colon("3 myjail extra"), "3:myjail extra");
        assert_eq!(first_space_to_colon("noSpaceHere"), "noSpaceHere");
    }

    #[test]
    fn strip_after_last_colon_drops_trailing_field() {
        assert_eq!(strip_after_last_colon("0:myhost.example"), "0");
        assert_eq!(strip_after_last_colon("myjail:myjail"), "myjail");
        assert_eq!(strip_after_last_colon("nocolon"), "nocolon");
    }

    #[test]
    fn returns_one_without_registered_tags() {
        // sh:19/32 — default param ("name") ⇒ desc=0 ⇒ the `_wanted`
        // branch, which returns 1 when no completion tagset is
        // registered (mirrors `_hosts.rs`'s analogous test).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _jails(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
