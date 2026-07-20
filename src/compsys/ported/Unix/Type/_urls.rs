//! Port of `_urls` from `Completion/Unix/Type/_urls`.
//!
//! Full upstream body (182 lines, abridged — the leading comment block
//! sh:1-39 documents the `urls` / `local` styles):
//! ```text
//! sh: 40  local ipre scheme host user uhosts ret=1 expl match glob suf localhttp
//! sh: 42  zstyle -a …:urls local localhttp   (servername/docroot/userdir)
//! sh: 47  zstyle -a …:urls urls urls
//! sh: 49-53  if the urls style holds >1 entry or a plain file → compadd them
//! sh: 55  urls="$urls[1]"   (the database directory)
//! sh: 57-58  glob=(-g '*(^/)'); zparseopts -D -K -E 'g:=glob'
//! sh: 62-74  no scheme yet → complete a scheme prefix (file:/ftp:///http://…)
//! sh: 75  scheme="$match[1]"
//! sh: 77-122  per-scheme: http/ftp/scp/gopher need `//`; file/unix → local
//!             path; bookmark → follow the bookmark file / _path_files under db
//! sh: 124-139  complete host component from $urls/$scheme/*(/:t) or _hosts
//! sh: 140  host="$match[1]"
//! sh: 142  a `:` after host → port number (_message)
//! sh: 146-181  path after host: localhttp docroot/userdir, urls db, or scp/sftp
//! ```
//!
//! `compstate[to_end]` (sh:66) and the `(#b)` match backrefs are read via
//! `$match`; sibling completers `_hosts`/`_users`/`_remote_files` dispatch
//! to their (possibly shell) implementations.

use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_path_files::_path_files;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}
fn compset(argv: &[&str]) -> i32 {
    bin_compset(
        "compset",
        &argv.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        &make_ops(),
        0,
    )
}
fn compadd(argv: &[String]) -> i32 {
    bin_compadd("compadd", argv, &make_ops(), 0)
}
fn get(name: &str) -> String {
    getsparam(name).unwrap_or_default()
}
fn dispatch(name: &str, args: &[String]) -> i32 {
    dispatch_function_call(name, args).unwrap_or(1)
}
fn match1() -> String {
    getaparam("match")
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default()
}

/// `_urls` — complete URLs from a filesystem URL database and styles.
pub fn _urls(args: &[String]) -> i32 {
    let curcontext = get("curcontext");
    let ctx = format!(":completion:{}:urls", curcontext);
    let mut ret = 1;

    // sh:42-45 — the `local` web-server style: servername/docroot/userdir.
    let localhttp = lookupstyle(&ctx, "local");
    let localhttp_servername = localhttp.first().cloned().unwrap_or_default();
    let localhttp_documentroot = localhttp.get(1).cloned().unwrap_or_default();
    let localhttp_userdir = localhttp.get(2).cloned().unwrap_or_default();

    // sh:47 — the `urls` style.
    let mut urls = lookupstyle(&ctx, "urls");

    // sh:49-53 — >1 entry, or a single plain-file/values entry: compadd them.
    let single_is_dir = urls.len() == 1 && std::path::Path::new(&urls[0]).is_dir();
    if urls.len() > 1 || (urls.len() == 1 && !single_is_dir) {
        // sh:50 — a single existing file: read the URLs from it.
        if urls.len() == 1 && std::path::Path::new(&urls[0]).is_file() {
            if let Ok(txt) = std::fs::read_to_string(&urls[0]) {
                urls = txt.split_whitespace().map(String::from).collect();
            }
        }
        // sh:51  _wanted urls expl 'URL' compadd "$@" -a urls
        let mut w: Vec<String> = vec![
            "urls".to_string(),
            "expl".to_string(),
            "URL".to_string(),
            "compadd".to_string(),
        ];
        w.extend(args.iter().cloned());
        w.push("-a".to_string());
        w.push("urls".to_string());
        crate::ported::params::setaparam("urls", urls.clone());
        if _wanted(&w) == 0 {
            return 0;
        }
        // sh:52  urls=()
        urls.clear();
    }

    // sh:55 — the database directory is the first (only) style entry.
    let db = urls.first().cloned().unwrap_or_default();
    let has_db = !db.is_empty();

    // sh:57-58 — default glob, overridable by `-g`.
    let mut glob: Vec<String> = vec!["-g".to_string(), "*(^/)".to_string()];
    let mut rest: Vec<String> = Vec::new();
    {
        let mut i = 0;
        while i < args.len() {
            if args[i] == "-g" && i + 1 < args.len() {
                glob = vec!["-g".to_string(), args[i + 1].clone()];
                i += 2;
            } else if let Some(g) = args[i].strip_prefix("-g") {
                glob = vec!["-g".to_string(), g.to_string()];
                i += 1;
            } else {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }

    let ipre = get("IPREFIX");

    // sh:62-74 — no `scheme:` yet → complete the scheme prefix.
    if compset(&["-P", "(#b)([-+.a-z0-9]#):"]) != 0 {
        let _ = _tags(&[
            "-C".to_string(),
            "argument".to_string(),
            "prefixes".to_string(),
        ]);
        loop {
            if _tags(&[]) != 0 {
                break;
            }
            loop {
                let mut nl: Vec<String> = vec![
                    "prefixes".to_string(),
                    "expl".to_string(),
                    "URL prefix".to_string(),
                    "-S".to_string(),
                    "".to_string(),
                ];
                nl.extend(rest.iter().cloned());
                if _next_label(&nl) != 0 {
                    break;
                }
                let _ = compset(&["-S", "[^:/]*"]); // sh:66 (to_end approx)
                let expl = getaparam("expl").unwrap_or_default();
                if has_db && std::path::Path::new(&format!("{}/bookmark", db)).is_dir() {
                    let mut c = expl.clone();
                    c.push("bookmark:".to_string());
                    if compadd(&c) == 0 {
                        ret = 0;
                    }
                }
                let mut c = expl.clone();
                c.extend(
                    ["file:", "ftp://", "gopher://", "http://", "https://"]
                        .iter()
                        .map(|s| s.to_string()),
                );
                if compadd(&c) == 0 {
                    ret = 0;
                }
            }
            if ret == 0 {
                return 0;
            }
        }
        return 1;
    }
    // sh:75
    let scheme = match1();

    // sh:77-122 — per-scheme handling.
    let is_web = matches!(
        scheme.as_str(),
        "http" | "https" | "ftp" | "sftp" | "scp" | "gopher"
    );
    if is_web {
        // sh:78-83 — need the `//` after `scheme:`.
        if compset(&["-P", "//"]) != 0 {
            let mut w: Vec<String> = vec![
                "-C".to_string(),
                scheme.clone(),
                "prefixes".to_string(),
                "expl".to_string(),
                "end of prefix".to_string(),
                "compadd".to_string(),
                "-S".to_string(),
                "".to_string(),
            ];
            w.extend(rest.iter().cloned());
            w.push("//".to_string());
            return _wanted(&w);
        }
    } else if scheme == "file" || scheme == "unix" {
        // sh:84-101 — local file path.
        let prefix = get("PREFIX");
        if prefix.starts_with("//127.0.0.1/") || prefix.starts_with("//localhost/") {
            let _ = compset(&["-P", "//(127.0.0.1|localhost)"]);
        }
        if prefix.starts_with("///") {
            let _ = compset(&["-P", "//"]);
        }
        if compset(&["-P", "//"]) != 0 {
            let _ = _tags(&["-C".to_string(), "file".to_string(), "files".to_string()]);
            loop {
                if _tags(&[]) != 0 {
                    break;
                }
                loop {
                    if _next_label(&[
                        "files".to_string(),
                        "expl".to_string(),
                        "local file".to_string(),
                    ]) != 0
                    {
                        break;
                    }
                    let expl = getaparam("expl").unwrap_or_default();
                    let prefix = get("PREFIX");
                    if prefix.starts_with('/') {
                        let mut pf = expl.clone();
                        pf.extend(["-S".to_string(), "".to_string()]);
                        pf.extend(glob.clone());
                        if _path_files(&pf) == 0 {
                            ret = 0;
                        }
                        let pf2 = build(&expl, &["-S", "/", "-r", "/", "-/"]);
                        if _path_files(&pf2) == 0 {
                            ret = 0;
                        }
                    } else if prefix.is_empty() {
                        let pwd = get("PWD");
                        let pwd = pwd.trim_end_matches('/').to_string();
                        let mut c = vec![
                            "-S".to_string(),
                            "/".to_string(),
                            "-r".to_string(),
                            "/".to_string(),
                        ];
                        c.extend(expl.clone());
                        c.extend(rest.iter().cloned());
                        c.push("-".to_string());
                        c.push(pwd);
                        if compadd(&c) == 0 {
                            ret = 0;
                        }
                    }
                }
                if ret == 0 {
                    return 0;
                }
            }
            return 1;
        }
    } else if scheme == "bookmark" {
        // sh:103-121 — follow a bookmark file, or list the bookmark tree.
        let pq = format!("{}{}", get("PREFIX"), get("SUFFIX"));
        let bmfile = format!("{}/{}/{}", db, scheme, pq);
        let meta = std::fs::metadata(&bmfile);
        if meta
            .as_ref()
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false)
        {
            let contents = std::fs::read_to_string(&bmfile).unwrap_or_default();
            let mut w: Vec<String> = vec![
                "-C".to_string(),
                "bookmark".to_string(),
                "bookmarks".to_string(),
                "expl".to_string(),
                "bookmark".to_string(),
                "compadd".to_string(),
            ];
            w.extend(rest.iter().cloned());
            w.push("-U".to_string());
            w.push("-".to_string());
            w.push(format!("{}{}", ipre, contents.trim_end_matches('\n')));
            return _wanted(&w);
        } else {
            let wdir = format!("{}/{}", db, scheme);
            let _ = _tags(&[
                "-C".to_string(),
                "bookmark".to_string(),
                "files".to_string(),
            ]);
            loop {
                if _tags(&[]) != 0 {
                    break;
                }
                loop {
                    if _next_label(&[
                        "files".to_string(),
                        "expl".to_string(),
                        "bookmark".to_string(),
                    ]) != 0
                    {
                        break;
                    }
                    let expl = getaparam("expl").unwrap_or_default();
                    let mut pf = vec!["-W".to_string(), wdir.clone()];
                    pf.extend(expl.clone());
                    pf.extend(["-S".to_string(), "".to_string()]);
                    pf.extend(glob.clone());
                    if _path_files(&pf) == 0 {
                        ret = 0;
                    }
                    let mut pf2 = vec![
                        "-W".to_string(),
                        wdir.clone(),
                        "-S".to_string(),
                        "/".to_string(),
                        "-r".to_string(),
                        "/".to_string(),
                    ];
                    pf2.extend(expl.clone());
                    pf2.push("-/".to_string());
                    if _path_files(&pf2) == 0 {
                        ret = 0;
                    }
                }
                if ret == 0 {
                    return 0;
                }
            }
            return ret;
        }
    }

    // sh:124-139 — complete the host component.
    if compset(&["-P", "(#b)([^:/]#)([:/])"]) != 0 {
        let prefix = get("PREFIX");
        let suffix = get("SUFFIX");
        let mut uhosts = glob_hosts(&db, &scheme, &prefix, &suffix);

        let _ = _tags(&["hosts".to_string()]);
        loop {
            if _tags(&[]) != 0 {
                break;
            }
            loop {
                if _next_label(&["hosts".to_string(), "expl".to_string(), "host".to_string()]) != 0
                {
                    break;
                }
                let suf = if compset(&["-S", "[:/]*"]) == 0 {
                    String::new()
                } else {
                    "/".to_string()
                };
                let expl = getaparam("expl").unwrap_or_default();
                if uhosts.is_empty() {
                    let mut h = vec![
                        "-S".to_string(),
                        suf.clone(),
                        "-r".to_string(),
                        "/:".to_string(),
                    ];
                    h.extend(expl.clone());
                    if dispatch("_hosts", &h) == 0 {
                        ret = 0;
                    }
                }
                if scheme == "http" && !localhttp_servername.is_empty() {
                    uhosts.push(localhttp_servername.clone());
                }
                let mut c = vec![
                    "-S".to_string(),
                    suf.clone(),
                    "-r".to_string(),
                    "/:".to_string(),
                ];
                c.extend(expl.clone());
                c.push("-a".to_string());
                c.push("uhosts".to_string());
                crate::ported::params::setaparam("uhosts", uhosts.clone());
                if compadd(&c) == 0 {
                    ret = 0;
                }
            }
            if ret == 0 {
                return 0;
            }
        }
        return 1;
    }
    // sh:140
    let host = match1();
    // sh:142 — a `:` after the host wants a port number.
    let match2 = getaparam("match")
        .unwrap_or_default()
        .get(1)
        .cloned()
        .unwrap_or_default();
    if match2 == ":" && compset(&["-P", "<->/"]) != 0 {
        let _ = crate::compsys::ported::_message::_message(&[
            "-e".to_string(),
            "ports".to_string(),
            "port number".to_string(),
        ]);
        return 0;
    }

    // sh:144-181 — path after the hostname.
    if _tags(&["remote-files".to_string(), "files".to_string()]) != 0 {
        return 1;
    }

    if localhttp_servername == host && !host.is_empty() {
        // sh:148-170 — the local web server: ~user area or document root.
        if compset(&["-P", "\\~"]) == 0 {
            if compset(&["-P", "(#b)([^/]#)/"]) != 0 {
                let mut u = vec!["-S".to_string(), "/".to_string()];
                u.extend(rest.iter().cloned());
                return dispatch("_users", &u);
            }
            let user = match1();
            path_after_host_loop(
                &mut ret,
                &rest,
                &glob,
                &format!("~{}/{}", user, localhttp_userdir),
            );
        } else {
            path_after_host_loop(&mut ret, &rest, &glob, &localhttp_documentroot);
        }
    } else {
        // sh:171-181 — the URL database, or scp/sftp remote listing.
        loop {
            if _tags(&[]) != 0 {
                break;
            }
            if has_db {
                let wdir = format!("{}/{}/{}", db, scheme, host);
                loop {
                    if _next_label(&[
                        "files".to_string(),
                        "expl".to_string(),
                        "local file".to_string(),
                    ]) != 0
                    {
                        break;
                    }
                    let expl = getaparam("expl").unwrap_or_default();
                    let mut pf = expl.clone();
                    pf.extend(rest.iter().cloned());
                    pf.extend(["-W".to_string(), wdir.clone()]);
                    pf.extend(glob.clone());
                    if _path_files(&pf) == 0 {
                        ret = 0;
                    }
                    let mut pf2 = vec![
                        "-S".to_string(),
                        "/".to_string(),
                        "-r".to_string(),
                        "/".to_string(),
                    ];
                    pf2.extend(expl.clone());
                    pf2.extend(["-W".to_string(), wdir.clone(), "-/".to_string()]);
                    if _path_files(&pf2) == 0 {
                        ret = 0;
                    }
                }
            }
            if (scheme == "scp" || scheme == "sftp")
                && _requested(&["remote-files".to_string()]) == 0
                && dispatch(
                    "_remote_files",
                    &[
                        "-h".to_string(),
                        host.clone(),
                        "--".to_string(),
                        "ssh".to_string(),
                    ],
                ) == 0
            {
                ret = 0;
            }
            if ret == 0 {
                return 0;
            }
        }
    }
    ret
}

/// sh:157-158 / 165-166 / 174-175 — the two `_path_files` calls under a
/// `-W dir` root inside the `_next_label files` loop.
fn path_after_host_loop(ret: &mut i32, rest: &[String], glob: &[String], wdir: &str) {
    loop {
        if _tags(&[]) != 0 {
            break;
        }
        loop {
            if _next_label(&[
                "files".to_string(),
                "expl".to_string(),
                "local file".to_string(),
            ]) != 0
            {
                break;
            }
            let expl = getaparam("expl").unwrap_or_default();
            let mut pf = expl.clone();
            pf.extend(rest.iter().cloned());
            pf.extend(["-W".to_string(), wdir.to_string()]);
            pf.extend(glob.iter().cloned());
            if _path_files(&pf) == 0 {
                *ret = 0;
            }
            let mut pf2 = vec![
                "-S".to_string(),
                "/".to_string(),
                "-r".to_string(),
                "/".to_string(),
            ];
            pf2.extend(expl.clone());
            pf2.extend(["-W".to_string(), wdir.to_string(), "-/".to_string()]);
            if _path_files(&pf2) == 0 {
                *ret = 0;
            }
        }
        if *ret == 0 {
            break;
        }
    }
}

/// sh:126 — `$urls/$scheme/$PREFIX*$SUFFIX(/:t)`: basenames of the
/// directories one level under `$db/$scheme` matching the prefix/suffix.
fn glob_hosts(db: &str, scheme: &str, prefix: &str, suffix: &str) -> Vec<String> {
    if db.is_empty() {
        return Vec::new();
    }
    let dir = format!("{}/{}", db, scheme);
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    if name.starts_with(prefix) && name.ends_with(suffix) {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Small helper: `expl` array followed by literal flag words.
fn build(expl: &[String], flags: &[&str]) -> Vec<String> {
    let mut v = expl.to_vec();
    v.extend(flags.iter().map(|s| s.to_string()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = crate::ported::params::setsparam("SUFFIX", "");
        let _ = crate::ported::params::setsparam("IPREFIX", "");
        assert_eq!(_urls(&[]), 1);
    }
}
