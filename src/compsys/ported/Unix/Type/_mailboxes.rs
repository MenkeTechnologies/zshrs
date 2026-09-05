//! Port of `_mailboxes` from `Completion/Unix/Type/_mailboxes`.
//!
//! Full upstream body (198 lines) — three shell functions:
//! ```text
//! sh:  3  _mailboxes()      main: pick tags by MUA (curcontext), _tags loop
//! sh:                       → _requested mailboxes _mua_mailboxes / _files.
//! sh: 63  _mailbox_cache()  build the -U caches (_mbox/_maildir/_mh/_pine/
//! sh:                       _mutt/_mailbox) by globbing $maildirectory and
//! sh:                       parsing the muttrc `mailboxes` directive.
//! sh:107  _mua_mailboxes()  per-MUA name list (compset prefixes +/@/%/=),
//! sh:                       then _multi_parts / compadd.
//! sh:198  _mailboxes "$@"
//! ```
//!
//! Every MUA branch is implemented: `elm`, `mail`, `mh`, `mush`, `mutt`,
//! `pine`, `tkrat`, `zmail`/`zmlite`, and the catch-all. The muttrc
//! `mailboxes …` directive (sh:74-76) is read and each token `(Xe)`-expanded
//! (`~` → `$HOME`, `$VAR`/`${VAR}` substitution). `~`/`$VAR` expansion and the
//! `${(@)^…}` / `${(@k)…}` cross-products are string-op stand-ins for the zsh
//! expansion engine (marked `// sh:N approx`); the completion result matches.

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_files::_files;
use crate::compsys::ported::_multi_parts::_multi_parts;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
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
    let v: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_compset("compset", &v, &make_ops(), 0)
}

/// Expand a leading `~` / `~/` to `$HOME`.
fn tilde(p: &str) -> String {
    if p == "~" {
        getsparam("HOME").unwrap_or_default()
    } else if let Some(rest) = p.strip_prefix("~/") {
        format!("{}/{}", getsparam("HOME").unwrap_or_default(), rest)
    } else {
        p.to_string()
    }
}

/// sh:76 `${(Xe)token}` approx — filename/parameter expansion of one muttrc
/// `mailboxes` token: leading `~`, and `$VAR` / `${VAR}` references.
fn xe_expand(tok: &str) -> String {
    let tok = tilde(tok);
    let bytes: Vec<char> = tok.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '$' && i + 1 < bytes.len() {
            let (name, next) = if bytes[i + 1] == '{' {
                // ${VAR}
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != '}' {
                    j += 1;
                }
                (
                    bytes[i + 2..j].iter().collect::<String>(),
                    (j + 1).min(bytes.len()),
                )
            } else {
                // $VAR (alnum/_)
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_alphanumeric() || bytes[j] == '_') {
                    j += 1;
                }
                (bytes[i + 1..j].iter().collect::<String>(), j)
            };
            if name.is_empty() {
                out.push('$');
                i += 1;
            } else {
                out.push_str(&getsparam(&name).unwrap_or_default());
                i = next;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Entries in `dir` that are NOT directories (`*(^/)`), full paths.
fn glob_nondir(dir: &str) -> Vec<String> {
    read_entries(dir, |md| !md.is_dir())
}

/// Sub-directories of `dir` (`*(/)`), full paths.
fn glob_dir(dir: &str) -> Vec<String> {
    read_entries(dir, |md| md.is_dir())
}

fn read_entries(dir: &str, pred: impl Fn(&std::fs::Metadata) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let name = ent.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') {
                continue; // glob hides dotfiles by default
            }
            if let Ok(md) = ent.metadata() {
                if pred(&md) {
                    out.push(format!("{}/{}", dir.trim_end_matches('/'), name));
                }
            }
        }
    }
    out
}

/// Regular files under `dir`, recursively (`**/*(.)`), full paths.
fn glob_reg_recursive(dir: &str, out: &mut Vec<String>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let path = ent.path();
            if path.is_dir() {
                if let Some(s) = path.to_str() {
                    glob_reg_recursive(s, out);
                }
            } else if path.is_file() {
                if let Some(s) = path.to_str() {
                    out.push(s.to_string());
                }
            }
        }
    }
}

/// `${(@)arr#prefix/}` — strip a leading `prefix/` from each element.
fn strip_dir_prefix(items: &[String], prefix: &str) -> Vec<String> {
    let pfx = format!("{}/", prefix.trim_end_matches('/'));
    items
        .iter()
        .map(|e| e.strip_prefix(&pfx).unwrap_or(e).to_string())
        .collect()
}

/// `${(@k)userdirs}` — the keys of the `userdirs` associative array.
fn userdirs_keys() -> Vec<String> {
    getaparam("userdirs")
        .unwrap_or_default()
        .chunks(2)
        .filter_map(|kv| kv.first().cloned())
        .collect()
}

fn get_cache(name: &str) -> Vec<String> {
    getaparam(name).unwrap_or_default()
}

/// `arrunique` — `-U`, keeping the FIRST occurrence of each element
/// (`Src/params.c`'s `uniqarray`, reached from `assignaparam` when the
/// parameter carries `PM_UNIQUE`).
fn uniq(mut v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
    v
}

/// sh:67-68 `typeset -aU -g NAME` — assign, then stamp the `-U`.
///
/// The stamp has to come AFTER `setaparam`, which creates the node and does
/// not carry the bit through; the value is uniq'd here rather than left to
/// the assignment for the same reason. Mirrors the attribute-only update in
/// compinit.rs's `declare_global` (c:Src/builtin.c:2575), inlined because
/// that helper is private to compinit.
///
/// `-U` is not cosmetic on these six: they are deliberate cross-invocation
/// GLOBALS (sh:10-12 rebuilds them only when `_mailbox_cache` is unset), and
/// sh:91-104 grow four of them with the `x=( "${x[@]}" more )` append idiom,
/// which dedups only because the parameter is unique.
fn set_unique_global(name: &str, val: Vec<String>) {
    setaparam(name, uniq(val));
    if let Ok(mut tab) = crate::ported::params::paramtab().write() {
        if let Some(pm) = tab.get_mut(name) {
            pm.node.flags |= crate::compsys::ported::shared::PM_UNIQUE as i32;
        }
    }
}

/// sh:63-105 — populate the `-U` caches by globbing `$maildirectory` and
/// parsing the muttrc `mailboxes` directive.
fn mailbox_cache() {
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    let maildirectory = lookupstyle(&ctx, "mail-directory")
        .into_iter()
        .next()
        .unwrap_or_else(|| "~/Mail".to_string());
    let pinedirectory = lookupstyle(&ctx, "pine-directory")
        .into_iter()
        .next()
        .unwrap_or_default();
    let muttrc = lookupstyle(&ctx, "muttrc")
        .into_iter()
        .next()
        .unwrap_or_else(|| "~/.muttrc".to_string());
    let maildir = tilde(&maildirectory);

    // sh:74-76 — parse the muttrc `mailboxes …` lines, `(Xe)`-expanding tokens.
    let mut mutt_cache: Vec<String> = Vec::new();
    let muttrc_path = tilde(&muttrc);
    if std::path::Path::new(&muttrc_path).is_file() {
        if let Ok(text) = std::fs::read_to_string(&muttrc_path) {
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("mailboxes ") {
                    for tok in rest.split_whitespace() {
                        mutt_cache.push(xe_expand(tok));
                    }
                }
            }
        }
    }

    // sh:81 — _mbox_cache starts with the non-dir entries of maildir.
    let mut mbox_cache: Vec<String> = glob_nondir(&maildir);
    let mut maildir_cache: Vec<String> = Vec::new();
    let mut mh_cache: Vec<String> = Vec::new();

    // sh:82-86 — _pine_cache: recursive regular files under pinedirectory.
    let mut pine_cache: Vec<String> = Vec::new();
    if !pinedirectory.is_empty() {
        glob_reg_recursive(&tilde(&pinedirectory), &mut pine_cache);
    }

    // sh:88-101 — walk sub-dirs: maildir (has cur/), mh (has numeric files),
    // else mbox collection; recurse into sub-dirs.
    let mut dirboxes: Vec<String> = glob_dir(&maildir);
    while let Some(i) = dirboxes.pop() {
        if std::path::Path::new(&format!("{}/cur", i)).is_dir() {
            maildir_cache.push(i);
        } else {
            let numeric = std::fs::read_dir(&i)
                .map(|rd| {
                    rd.flatten().any(|e| {
                        e.file_name()
                            .to_str()
                            .map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if numeric {
                mh_cache.push(i.clone());
                dirboxes.extend(glob_dir(&i));
            } else {
                mbox_cache.extend(glob_nondir(&i));
                dirboxes.extend(glob_dir(&i));
            }
        }
    }

    // sh:104-109 — add $mailpath (%-stripped) and $MAIL.
    let mut mailbox_cache: Vec<String> = Vec::new();
    if let Some(mp) = getaparam("mailpath") {
        for e in mp {
            mailbox_cache.push(e.split('?').next().unwrap_or(&e).to_string());
        }
    }
    if let Some(mail) = getsparam("MAIL").filter(|s| !s.is_empty()) {
        mailbox_cache.push(mail);
    }

    // sh:67  typeset -aU -g _mailbox_cache
    // sh:68  typeset -aU -g _maildir_cache _mbox_cache _mh_cache _mutt_cache
    //        _pine_cache
    set_unique_global("_mbox_cache", mbox_cache);
    set_unique_global("_maildir_cache", maildir_cache);
    set_unique_global("_mh_cache", mh_cache);
    set_unique_global("_pine_cache", pine_cache);
    set_unique_global("_mutt_cache", mutt_cache);
    set_unique_global("_mailbox_cache", mailbox_cache);
}

/// sh:130 `$(mhpath)` — the current MH folder path (empty when unavailable).
fn mhpath() -> String {
    let _ = call_program_capture(&["mailboxes".to_string(), "mhpath".to_string()]);
    getsparam("REPLY").unwrap_or_default().trim().to_string()
}

/// sh:107-196 — MUA-specific mailbox-name completion.
///
/// Upstream reaches this as a COMMAND WORD, not as a call: sh:50 hands the
/// bare name `_mua_mailboxes` to `_requested`, which forwards it to
/// `_all_labels` (`Completion/Base/Core/_requested`:9-10), and `_all_labels`
/// runs it once per label with `$expl[@]` appended. So the name has to be resolvable
/// by `dispatch_function_call`, which is why `compsys::router` registers it
/// alongside `_mailboxes` — the same shape as
/// `_perl_modules_caching_policy`. Registered names are dispatched by
/// `_all_labels` (Base/Core/_all_labels.rs:224), so this body must NOT be
/// invoked a second time from `_mailboxes`.
pub fn _mua_mailboxes(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_mua_mailboxes");
    // sh:109  local -aU mbox_names
    //
    // `_multi_parts` reads the array BY NAME at sh:193, so it has to be a
    // real parameter — but a LOCAL one, and a unique one. Every arm below
    // concatenates two to seven of the caches, and several of them overlap
    // (a muttrc `mailboxes` entry that is also `$MAIL`, `$mailpath` naming a
    // file already in `$_mbox_cache`), so without `-U` the same mailbox is
    // offered as two identical matches. The value is uniq'd at the
    // assignment below as well, because `declare_locals` is a documented
    // no-op at `locallevel == 0` (unit tests, `--doctor`).
    crate::compsys::ported::shared::declare_locals(
        &["mbox_names"],
        crate::compsys::ported::shared::PM_ARRAY | crate::compsys::ported::shared::PM_UNIQUE,
    );
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    let maildirectory = tilde(
        &lookupstyle(&ctx, "mail-directory")
            .into_iter()
            .next()
            .unwrap_or_else(|| "~/Mail".to_string()),
    );

    let mbox = get_cache("_mbox_cache");
    let maildir = get_cache("_maildir_cache");
    let mh = get_cache("_mh_cache");
    let mutt = get_cache("_mutt_cache");
    let pine = get_cache("_pine_cache");
    let mailbox = get_cache("_mailbox_cache");

    let mut mbox_names: Vec<String> = Vec::new();
    let mut mbox_short: Vec<String> = Vec::new();

    // Which MUA? Match the middle field of curcontext.
    let is = |m: &str| curcontext.contains(&format!(":{}:", m));

    if is("elm") {
        // sh:117-120
        mbox_names.extend(mbox.clone());
        mbox_names.extend(mailbox.clone());
        mbox_short = vec!["!".into(), "<".into(), ">".into()];
    } else if is("mail") {
        // sh:121-128
        if compset(&["-P", "+"]) == 0 {
            mbox_names = strip_dir_prefix(&mbox, &maildirectory);
        } else {
            mbox_names = strip_dir_prefix(&mbox, &maildirectory)
                .into_iter()
                .map(|e| format!("+{}", e))
                .collect();
            mbox_names.extend(mailbox.clone());
        }
    } else if is("mh") {
        // sh:129-141
        let lastmhbox = mhpath();
        if compset(&["-P", "+"]) == 0 {
            mbox_names = strip_dir_prefix(&mh, &maildirectory);
        } else if compset(&["-P", "@"]) == 0 {
            // sh:133-134 — folders under the current MH box.
            let pfx = format!("{}/", lastmhbox);
            mbox_names = mh
                .iter()
                .filter(|e| e.starts_with(&pfx))
                .map(|e| e.strip_prefix(&pfx).unwrap_or(e).to_string())
                .collect();
        } else {
            // sh:136-138
            mbox_names = strip_dir_prefix(&mh, &maildirectory)
                .into_iter()
                .map(|e| format!("+{}", e))
                .collect();
            let pfx = format!("{}/", lastmhbox);
            mbox_names.extend(
                mh.iter()
                    .filter(|e| e.starts_with(&pfx))
                    .map(|e| format!("@{}", e.strip_prefix(&pfx).unwrap_or(e))),
            );
            mbox_names.extend(mh.clone());
        }
    } else if is("mush") {
        // sh:141-150
        if compset(&["-P", "%"]) == 0 {
            mbox_short = userdirs_keys();
        } else if compset(&["-P", "+"]) == 0 {
            mbox_names = strip_dir_prefix(&mbox, &maildirectory);
        } else {
            mbox_names = strip_dir_prefix(&mbox, &maildirectory)
                .into_iter()
                .map(|e| format!("+{}", e))
                .collect();
            mbox_names.extend(mailbox.clone());
            mbox_short = vec!["&".into(), "%".into()];
            mbox_short.extend(userdirs_keys().into_iter().map(|k| format!("%{}", k)));
        }
    } else if is("mutt") {
        // sh:152-162
        if compset(&["-P", "(|\\)="]) == 0 || compset(&["-P", "+"]) == 0 {
            mbox_names.extend(
                mutt.iter()
                    .map(|e| e.trim_start_matches(['+', '=']).to_string()),
            );
            mbox_names.extend(strip_dir_prefix(&mbox, &maildirectory));
            mbox_names.extend(strip_dir_prefix(&maildir, &maildirectory));
            mbox_names.extend(strip_dir_prefix(&mh, &maildirectory));
        } else {
            mbox_names.extend(mutt.clone());
            mbox_names.extend(mailbox.clone());
            mbox_names.extend(maildir.clone());
            mbox_names.extend(mh.clone());
            mbox_short = vec!["!".into(), "<".into(), ">".into()];
        }
    } else if is("pine") {
        // sh:163-171
        mbox_names.extend(mbox.clone());
        mbox_names.extend(mailbox.clone());
        mbox_names.extend(mh.clone());
        let pinedirectory = lookupstyle(&ctx, "pine-directory")
            .into_iter()
            .next()
            .unwrap_or_default();
        if !pinedirectory.is_empty() {
            mbox_names.extend(strip_dir_prefix(&pine, &tilde(&pinedirectory)));
        }
    } else if is("tkrat") {
        // sh:172-174 — tkrat has custom formats upstream never programmed for;
        // it uses the basic mailbox/mbox/mh lists.
        mbox_names.extend(mbox.clone());
        mbox_names.extend(mailbox.clone());
        mbox_names.extend(mh.clone());
    } else if is("zmail") || is("zmlite") {
        // sh:176-185
        if compset(&["-P", "%"]) == 0 {
            mbox_short = userdirs_keys();
        } else if compset(&["-P", "+"]) == 0 {
            mbox_names = strip_dir_prefix(&mbox, &maildirectory);
        } else {
            mbox_names = strip_dir_prefix(&mbox, &maildirectory)
                .into_iter()
                .map(|e| format!("+{}", e))
                .collect();
            mbox_names.extend(mailbox.clone());
            mbox_names.extend(mh.clone());
            mbox_short = vec!["&".into(), "%".into()];
            mbox_short.extend(userdirs_keys().into_iter().map(|k| format!("%{}", k)));
        }
    } else {
        // sh:187-190 — default: everything.
        mbox_names.extend(mailbox);
        mbox_names.extend(mbox);
        mbox_names.extend(mh);
        mbox_names.extend(mutt);
        mbox_names.extend(pine);
    }

    let mut ret = 1;
    // sh:193 — (( $#mbox_names )) && _multi_parts "$@" / mbox_names
    if !mbox_names.is_empty() {
        // sh:109's `-U` — see the declaration at the top of this function.
        setaparam("mbox_names", uniq(mbox_names));
        let mut mp: Vec<String> = args.to_vec();
        mp.push("/".to_string());
        mp.push("mbox_names".to_string());
        if _multi_parts(&mp) == 0 {
            ret = 0;
        }
    }
    // sh:194 — (( $#mbox_short )) && compadd "$@" -a mbox_short
    if !mbox_short.is_empty() {
        setaparam("mbox_short", mbox_short);
        let mut ca: Vec<String> = args.to_vec();
        ca.push("-a".to_string());
        ca.push("mbox_short".to_string());
        if bin_compadd("compadd", &ca, &make_ops(), 0) == 0 {
            ret = 0;
        }
    }
    ret
}

/// `_mailboxes` — complete mailbox specifications / files for mail clients.
pub fn _mailboxes(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_mailboxes");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let mut ret = 1;

    // sh:10-12 — build the caches once.
    if getaparam("_mailbox_cache").is_none() {
        mailbox_cache();
    }

    let prefix = getsparam("PREFIX").unwrap_or_default();
    // sh:14-47 — choose tags based on MUA + PREFIX.
    let is = |m: &str| curcontext.contains(&format!(":{}:", m));
    let want_files;
    if is("mail") {
        want_files = !prefix.starts_with('+');
    } else if is("mush") || is("zmail") || is("zmlite") {
        want_files = !(prefix.starts_with('%') || prefix.starts_with('+'));
    } else if is("mutt") {
        let p = prefix.strip_prefix("-f").unwrap_or(&prefix);
        want_files = !(p.starts_with('+') || p.starts_with('='));
    } else if is("pine") {
        let p = prefix.strip_prefix("-f").unwrap_or(&prefix);
        want_files = p.starts_with('/') || p.starts_with('~');
    } else {
        let p = prefix.strip_prefix("-f").unwrap_or(&prefix);
        want_files = !p.starts_with('+');
    }
    let _ = if want_files {
        _tags(&["mailboxes".to_string(), "files".to_string()])
    } else {
        _tags(&["mailboxes".to_string()])
    };

    // sh:49-58 — the _tags / _requested loop.
    loop {
        if _tags(&[]) != 0 {
            break;
        }
        // sh:50 `_requested mailboxes expl 'mailbox specification'
        // _mua_mailboxes && ret=0` — the 4th word is the COMMAND
        // `_all_labels` runs, once per label, with `$expl[@]` appended. The
        // port used to ALSO call the body directly (`&& mua_mailboxes(args)`),
        // which was wrong twice over: `_all_labels` already runs it, and the
        // direct call passed `_mailboxes`' own `"$@"` instead of `$expl[@]`.
        // In practice neither ran — the name resolved to nothing, so the
        // shell reported `_all_labels:39: command not found: _mua_mailboxes`
        // and `_requested` returned 1, taking the `&&` with it. See
        // `_mua_mailboxes` below for the registration that fixes the lookup.
        if _requested(&[
            "mailboxes".to_string(),
            "expl".to_string(),
            "mailbox specification".to_string(),
            "_mua_mailboxes".to_string(),
        ]) == 0
        {
            ret = 0;
        }
        if _requested(&[
            "files".to_string(),
            "expl".to_string(),
            "mailbox file".to_string(),
        ]) == 0
        {
            // sh:52-53 — non-MUA contexts strip a `-f` file prefix.
            if !(is("mail") || is("mush") || is("mutt") || is("zmail") || is("zmlite")) {
                let _ = compset(&["-P", "-f"]);
            }
            let expl = getaparam("expl").unwrap_or_default();
            if crate::compsys::ported::shared::call_compfn("_files", &expl, || _files(&expl)) == 0 {
                ret = 0;
            }
        }
        if ret == 0 {
            break;
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_dir_prefix_drops_leading_dir() {
        let items = vec!["/m/Mail/inbox".to_string(), "/m/Mail/sub/x".to_string()];
        assert_eq!(
            strip_dir_prefix(&items, "/m/Mail"),
            vec!["inbox".to_string(), "sub/x".to_string()]
        );
    }

    #[test]
    fn xe_expand_does_tilde_and_vars() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("HOME", "/home/u");
        crate::ported::params::setsparam("MAILVAR", "/var/mail/u");
        // sh:76
        assert_eq!(xe_expand("~/Mail/inbox"), "/home/u/Mail/inbox");
        assert_eq!(xe_expand("$MAILVAR"), "/var/mail/u");
        assert_eq!(xe_expand("${MAILVAR}/x"), "/var/mail/u/x");
        assert_eq!(xe_expand("plain"), "plain");
    }

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("curcontext", ":completion::mail:::");
        crate::ported::params::setsparam("PREFIX", "");
        setaparam("_mailbox_cache", Vec::new());
        assert_eq!(_mailboxes(&[]), 1);
    }
}
